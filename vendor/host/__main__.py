"""CLI entry (SPEC §2):

    python3 -m host --mode layer|preview|serve|app [--port N] [--token T] [--dry-run]
                    [--open APP[:PARAM|:JSON]] [--quit-after SEC] [--dev] [--no-gtk] [-v] [PAD…]

  PAD…     only with --open explorer:…: more folders / files / file:// URIs (the desktop entry's %U), see
           host/explorer_args.py — a folder opens as itself, a file in its folder with the file selected, tabs

  layer    HTTP server + layer-shell surfaces (Hyprland); falls back to preview — inside the Win11 Shell session
           a native notice instead (host/problem.py: what is missing, the apt command, Opnieuw proberen)
  preview  HTTP server + one normal window
  serve    HTTP server only (prints the URL to open in a browser)
  app      reuse a running instance (shell.openApp) or start a server + app window; exits with it
"""

from __future__ import annotations

import argparse
import json
import logging
import logging.handlers
import os
import secrets
import signal
import sys
import threading
import time
from pathlib import Path

from . import APP_IDS, ROOT, VERSION
from .util import ApiError, call_instance, glib_signal_add, own_app, proc_starttime, read_instance, \
    remove_instance, write_instance

log = logging.getLogger("win11shell")

DEFAULT_PARAM = {"settings": "page", "taskmgr": "page", "explorer": "path", "alles": "query", "disks": "volume"}


def parse_open(spec: str, extra: list[str] | None = None) -> tuple[str, dict]:
    """'taskmgr' | 'settings:display' | 'explorer:/home/x' | 'settings:{"page":"display"}' → (app, params).

    Verkenner (the desktop entry runs ``win11shell explorer %U``): the parameter and every ``extra`` argument is a
    folder, a file, a ``file://`` URI (percent-decoded) or a place; a folder opens as itself, a file in its folder
    with the file selected, more folders as tabs, other schemes are skipped with a log line (host/explorer_args.py).
    """
    app_s, sep, rest = spec.partition(":")
    app = own_app(app_s)
    if app is None:
        raise SystemExit(f"Onbekende app '{app_s}'. Kies uit: {', '.join(APP_IDS)}")
    extra = [a for a in (extra or []) if isinstance(a, str) and a]
    if extra and app != "explorer":
        raise SystemExit(f"Te veel argumenten voor {app} (unexpected arguments): {' '.join(extra)[:200]}")
    params: dict = {}
    targets: list[str] = []
    if sep and rest:
        parsed = None
        if rest.lstrip().startswith("{"):
            try:
                parsed = json.loads(rest)
            except ValueError:
                parsed = None
        if isinstance(parsed, dict):
            params = parsed
        elif app == "explorer":
            targets.append(rest)
        else:
            params = {DEFAULT_PARAM[app]: rest}
    if app != "explorer":
        return app, params
    if isinstance(params.get("path"), str):
        p = os.path.expanduser(params["path"])
        if not os.path.isabs(p) and os.path.exists(p):
            p = os.path.abspath(p)
        params["path"] = p
    targets += extra
    if targets:
        from .explorer_args import explorer_params
        got = explorer_params(targets, how="auto")
        if got is None:
            log.warning("Niets om te openen in Verkenner; ik open Start (nothing to show, opening Home): %s",
                        " ".join(targets)[:300])
        elif params:   # JSON spec + more arguments: those open as extra tabs
            more = [{k: v for k, v in got.items() if k in ("path", "select")}] + list(got.get("tabs", []))
            prev = params.get("tabs")
            params["tabs"] = (list(prev) if isinstance(prev, list) else []) + more
        else:
            params = got
    return app, params


class _PrivateLogFile(logging.handlers.RotatingFileHandler):
    """host.log (and its rotations) readable by the user only: it may end up in bug reports, but other
    local users have no business reading what the desktop did."""

    def _open(self):
        fd = os.open(self.baseFilename, os.O_WRONLY | os.O_CREAT | os.O_APPEND | os.O_CLOEXEC, 0o600)
        try:
            os.fchmod(fd, 0o600)
        except OSError:
            pass
        return open(fd, self.mode, encoding=self.encoding, errors=self.errors)


def _setup_logging(verbose: bool) -> None:
    root = logging.getLogger()
    root.setLevel(logging.DEBUG if verbose else logging.INFO)
    fmt = logging.Formatter("%(asctime)s %(levelname)s %(name)s: %(message)s", "%H:%M:%S")
    sh = logging.StreamHandler(sys.stderr)
    sh.setFormatter(fmt)
    root.addHandler(sh)
    cache = Path(os.environ.get("WIN11SHELL_CACHE_DIR") or
                 Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "win11shell")
    try:
        cache.mkdir(parents=True, exist_ok=True, mode=0o700)
        fh = _PrivateLogFile(cache / "host.log", maxBytes=1_000_000, backupCount=2, encoding="utf-8")
        fh.setFormatter(logging.Formatter("%(asctime)s %(process)d %(levelname)s %(name)s: %(message)s"))
        root.addHandler(fh)
    except OSError:
        pass


def _try_existing(app: str, params: dict) -> bool:
    """app mode: hand the request to a running instance. True when it took it."""
    inst = read_instance()
    if not inst or inst.get("mode") == "serve":
        return False  # a developer's browser preview is not where a native app window belongs
    args = {"app": app, "params": params}
    tok = os.environ.get("XDG_ACTIVATION_TOKEN") or os.environ.get("DESKTOP_STARTUP_ID")
    if tok:
        args["activationToken"] = tok
    for attempt in range(3):
        try:
            call_instance("shell.openApp", args, inst=inst, timeout=5)
            log.info("Doorgegeven aan draaiende Win11 Shell (handed to running instance) pid %s", inst.get("pid"))
            return True
        except ApiError as e:
            if e.code == "shutting_down" and attempt < 2:
                time.sleep(0.8)
                continue
            log.info("Draaiende instantie nam het niet over (%s); start eigen (starting own)", e.code)
            return False
    return False


def _names_live_desktop_host(inst: dict | None) -> bool:
    """Does the instance file name a running desktop host (layer/preview) — also a hung one?"""
    if not isinstance(inst, dict) or inst.get("mode") not in ("layer", "preview"):
        return False
    from .ctl import host_process
    return host_process(inst) is not None


def _instance_info(ctx, token: str, mode: str) -> dict:
    # starttime: boot-relative and kept across exec — ctl's exact identity check (no wall clock, no pid reuse);
    # runId: new for every image, also after an in-place restart (ctl._wait_up, Taakbeheer)
    return {"pid": os.getpid(), "port": ctx.port, "token": token, "mode": mode, "version": VERSION,
            "dryRun": ctx.dry_run, "started": int(ctx.started_at), "starttime": proc_starttime(),
            "runId": ctx.run_id}


def _restart_argv(args: argparse.Namespace, port: int) -> list[str]:
    argv = [sys.executable, "-m", "host", "--mode", args.mode_requested, "--port", str(port)]
    if args.dry_run:
        argv.append("--dry-run")
    if args.dev:
        argv.append("--dev")
    if args.verbose:
        argv.append("-v")
    if args.no_gtk:
        argv.append("--no-gtk")
    return argv


def _install_hang_aids(ctx, GLib) -> None:
    """Test aids (--dev or WIN11SHELL_DEBUG_HANG=1 only; tests/nested): SIGRTMIN+7 blocks the main loop for 10 min
    while the HTTP threads keep answering (a C call into WebKit/EGL that never returns: the taskbar freezes, SIGTERM
    is never handled); SIGRTMIN+8 does that and asks for a restart from a thread right before (Taakbeheer's
    "Win11 Shell opnieuw starten" while the loop hangs: only the exit deadline can help)."""
    def block():
        log.warning("[dev] hoofdlus geblokkeerd (main loop blocked on purpose)")
        time.sleep(600)
        return False

    def restart_then_block(*_a):
        threading.Timer(0.3, ctx.request_restart).start()
        GLib.idle_add(block)

    signal.signal(signal.SIGRTMIN + 7, lambda *_a: GLib.idle_add(block))
    signal.signal(signal.SIGRTMIN + 8, restart_then_block)


# Exit codes that tell bin/win11shell's supervisor NOT to start the shell again (5: not until it is installed).
EXIT_USAGE = 2
EXIT_ALREADY_RUNNING = 3
EXIT_NO_DISPLAY = 4
EXIT_MISSING = 5     # an essential package is missing (<runtime>/problem.json); bin/win11shell waits for it


def _desktop_session() -> bool:
    """Are we the desktop (the Win11 Shell session's Hyprland), not a try-out in GNOME? There a missing package
    gives the problem notice, never the silent preview window (notes/session-failure-ux.md §6)."""
    return bool(os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")) or \
        "Win11Shell" in os.environ.get("XDG_CURRENT_DESKTOP", "")


def _missing_packages() -> dict | None:
    """host/preflight.py in this process: the problem dict when something essential is missing, else None."""
    try:
        from . import preflight
        r = preflight.check("layer", in_process=True)
    except Exception:  # noqa: BLE001 - the check itself must never be what stops the shell
        log.debug("preflight faalde (failed)", exc_info=True)
        return None
    return None if r["ok"] else {"code": "missing", **r}


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="win11shell", description="Win11 Shell for Ubuntu")
    p.add_argument("--mode", choices=("layer", "preview", "serve", "app"), default="layer")
    p.add_argument("--port", type=int, default=0, help="HTTP port (default: random)")
    p.add_argument("--token", default=None, help="API token (default: random; also $WIN11SHELL_TOKEN)")
    p.add_argument("--dry-run", action="store_true", help="never change the system, only log")
    p.add_argument("--open", default=None, metavar="APP[:PARAM]", help="open an app at start")
    p.add_argument("--quit-after", type=float, default=None, metavar="SEC")
    p.add_argument("--dev", action="store_true", help="developer extras (inspector), verbose logs")
    p.add_argument("--no-gtk", action="store_true", help="never import GTK (serve mode)")
    p.add_argument("-v", "--verbose", action="store_true")
    # more folders / files / URIs for --open explorer:… (desktop entry: Exec=win11shell explorer %U)
    p.add_argument("targets", nargs="*", metavar="PAD", help="meer mappen, bestanden of URI's voor --open explorer")
    args = p.parse_intermixed_args(argv)
    if args.targets and own_app((args.open or "").partition(":")[0]) != "explorer":
        p.error(f"onverwachte argumenten (unexpected arguments): {' '.join(args.targets)[:200]}")
    args.mode_requested = args.mode
    _setup_logging(args.verbose or args.dev)
    from .surfaces import reap_inherited_children   # imports no Gtk
    reap_inherited_children()     # after an in-place restart: the old image's WebKit/helper children (zombies, hung)

    token = args.token or os.environ.pop("WIN11SHELL_TOKEN", None) or secrets.token_urlsafe(24)
    open_spec = parse_open(args.open, args.targets) if args.open else None
    mode = args.mode

    if mode == "app":
        if not open_spec:
            print("Geef een app op, bijvoorbeeld: --open taskmgr (give an app)", file=sys.stderr)
            return 2
        if _try_existing(*open_spec):
            return 0

    existing = read_instance()
    if existing and existing.get("pid") != os.getpid() and existing.get("mode") in ("layer", "preview") \
            and mode in ("layer", "preview"):
        log.error("Win11 Shell draait al (pid %s, %s). / Already running.", existing.get("pid"), existing.get("mode"))
        print("Win11 Shell draait al.", file=sys.stderr)
        return EXIT_ALREADY_RUNNING

    from .api import Api
    from .context import Ctx

    api = Api(dry_run=bool(args.dry_run))      # the registry's own flag (never a process global)
    ctx = Ctx(mode=mode, api=api, dry_run=args.dry_run, dev=args.dev, token=token, root=ROOT, logger=log)
    ctx.open_app_id = open_spec[0] if open_spec else None
    if args.dry_run:
        log.info("Proefdraaimodus: niets aan het systeem wordt gewijzigd (dry-run: no system changes)")

    surfaces = None
    use_gtk = False
    if mode in ("layer", "preview", "app") and not args.no_gtk:
        from .surfaces import SurfaceManager
        surfaces = SurfaceManager(ctx)
        if surfaces.init_gtk():
            use_gtk = True
        elif mode in ("layer", "preview"):
            missing = _missing_packages()          # gi or Gtk itself may be what is missing, not the display
            if missing:
                from .problem import write_problem
                log.error("Win11 Shell kan niet starten: %s ontbreekt. Installeer: %s "
                          "/ missing packages, exiting with %d", ", ".join(missing["packages"]), missing["command"],
                          EXIT_MISSING)
                write_problem(missing)
                return EXIT_MISSING
            log.error("Geen grafische omgeving gevonden; gebruik 'win11shell serve' voor de browser. "
                      "/ No display found; use 'serve' mode for a browser.")
            return EXIT_NO_DISPLAY
        else:
            surfaces = None
        if mode == "layer":
            ok, reason = surfaces.layer_support()
            if not ok:
                if surfaces.problem and _desktop_session():
                    log.error("Bureaubladmodus kan niet starten: %s. Ik toon uitleg, geen voorbeeldvenster. "
                              "/ Layer mode cannot start (%s); showing the problem notice.", reason, reason)
                    surfaces.show_problem = True
                    from .problem import write_problem
                    write_problem(surfaces.problem)
                else:
                    log.warning("Bureaubladmodus niet beschikbaar: %s. Ik open de voorbeeldweergave. "
                                "/ Layer mode unavailable (%s); falling back to preview.", reason, reason)
                mode = "preview"
    ctx.mode = mode
    ctx.surfaces = surfaces

    from .server import ShellServer
    from .services import load_services, start_services, stop_services

    load_services(api, ctx)
    try:
        server = ShellServer(ctx, args.port)
    except OSError as e:
        log.error("Poort %s niet beschikbaar (port unavailable): %s", args.port, e)
        return 1
    ctx.port = server.port
    server.start()
    ctx.started_at = time.time()

    wrote_instance = False
    current = read_instance()
    if current is None or current.get("pid") == os.getpid():
        write = True
    elif mode == "serve":
        write = False
    elif mode == "app":
        # never over the file of a running desktop host, also not a hung one: ctl restart (Ctrl+Shift+Win+B,
        # Taakbeheer) finds the host to stop through it, and the app windows of that host follow it
        write = not _names_live_desktop_host(current)
    else:
        write = True
    if write:
        write_instance(_instance_info(ctx, token, mode))
        wrote_instance = True
    else:
        log.info("Andere instantie actief (pid %s, %s); instance-bestand niet overschreven (instance file kept)",
                 current.get("pid"), current.get("mode"))

    start_services(api, ctx)

    if mode == "serve":
        url = f"http://127.0.0.1:{ctx.port}/ui/index.html?surface=preview&token={token}"
        print(f"Win11 Shell draait op (running at): {url}", flush=True)

    # -- main loop -------------------------------------------------------------
    GLib = None
    loop = None
    try:
        import gi
        gi.require_version("GLib", "2.0")
        from gi.repository import GLib as _GLib
        GLib = _GLib
    except Exception:  # noqa: BLE001
        GLib = None

    exit_code = 0
    try:
        if use_gtk and surfaces is not None:
            from gi.repository import Gtk
            try:
                surfaces.start(mode)
            except Exception as e:  # noqa: BLE001
                if mode != "layer":
                    raise
                desk = _desktop_session()
                log.exception("Bureaubladmodus mislukt; ik %s. / Layer surfaces failed; %s.",
                              "toon uitleg" if desk else "open de voorbeeldweergave",
                              "showing the problem notice" if desk else "falling back to preview")
                surfaces.destroy()
                surfaces.problem = _missing_packages() or \
                    {"code": "layer_failed", "detail": f"{type(e).__name__}: {e}"[:300]}
                surfaces.show_problem = desk
                if desk:
                    from .problem import write_problem
                    write_problem(surfaces.problem)
                mode = ctx.mode = "preview"
                surfaces.start(mode)
                if wrote_instance:
                    write_instance(_instance_info(ctx, token, mode))
            def quit_impl():
                # Destroy the WebViews while the loop still runs (WebKit shuts its processes down
                # cleanly); destroying them after Gtk.main() returned crashes at interpreter exit.
                try:
                    surfaces.destroy()
                except Exception:  # noqa: BLE001
                    log.debug("destroy failed", exc_info=True)
                Gtk.main_quit()
        elif GLib is not None:
            loop = GLib.MainLoop()
            quit_impl = loop.quit
        else:
            quit_impl = None

        # never stuck (host/lifeguard.py): the main-loop beat for shell.info, the stuck watch (desktop host) and the
        # deadline after request_quit / request_restart
        if quit_impl is not None and GLib is not None:
            from .lifeguard import BEAT_MS, LoopGuard
            guard = LoopGuard(ctx)
            guard.beat()
            GLib.timeout_add(BEAT_MS, guard.beat)
            ctx.exit_guard = guard.arm
            if use_gtk and mode in ("layer", "preview"):
                guard.watch()
            if args.dev or os.environ.get("WIN11SHELL_DEBUG_HANG") == "1":
                _install_hang_aids(ctx, GLib)

        if open_spec:
            try:
                ctx.open_app(open_spec[0], open_spec[1],
                             activation_token=os.environ.get("XDG_ACTIVATION_TOKEN") or None)
            except ApiError as e:
                log.error("App niet geopend (app not opened): %s", e.message)
                if mode == "app":
                    raise SystemExit(1)

        if args.quit_after:
            t = threading.Timer(args.quit_after, ctx.request_quit)
            t.daemon = True
            t.start()

        if quit_impl is not None and GLib is not None:
            def on_signal():
                log.info("Signaal ontvangen, afsluiten (signal received, quitting)")
                ctx.request_quit(0)
                return True

            for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
                glib_signal_add(sig, on_signal)
            ctx.set_main_loop(quit_impl, True)
            if ctx.shutting_down:
                pass  # quit requested before the loop started
            elif use_gtk:
                Gtk.main()
            else:
                loop.run()
        else:
            def handler(_s, _f):
                ctx.request_quit(0)

            for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
                signal.signal(sig, handler)
            while not ctx.wait_quit(0.5):
                pass
        exit_code = ctx.exit_code
    except SystemExit as e:
        exit_code = int(e.code or 0)
    except KeyboardInterrupt:
        exit_code = 0
    finally:
        ctx.set_main_loop(None, False)
        ctx.shutting_down = True
        log.info("Afsluiten (shutting down)…")
        stop_services(api, ctx)
        ctx.run_shutdown_callbacks()
        server.stop()
        if wrote_instance and not ctx.restart_requested:
            remove_instance(os.getpid())

    if ctx.restart_requested:
        argv2 = _restart_argv(args, ctx.port)
        env = dict(os.environ)
        env["WIN11SHELL_TOKEN"] = token
        env["PYTHONPATH"] = str(ROOT) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        log.info("Herstarten (restarting)…")
        logging.shutdown()
        os.chdir(str(ROOT))
        os.execve(sys.executable, argv2, env)
    logging.shutdown()
    if use_gtk:
        # Everything is cleaned up explicitly above. Skip interpreter finalization: native
        # GTK/WebKit destructors at exit can crash (→ an Ubuntu "app crashed" popup).
        try:
            sys.stdout.flush()
            sys.stderr.flush()
        finally:
            os._exit(exit_code)
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
