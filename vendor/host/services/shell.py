"""shell.* (SPEC §6.2, §2): info, commands, input regions, opening our apps, file picker, restart/quit.

Opening apps (``shell.openApp`` / ``ctx.open_app``):
  * preview / serve → event ``shell.openApp {app, params}`` (preview WM opens an in-page window).
    If no page is connected yet, the event is held and delivered ~1.2 s after the first SSE client connects.
  * layer / app     → one ``python3 -m host.appwindow`` process per app (own Wayland app_id
    ``win11shell-<app>``). Re-opening focuses the running window and emits ``app.navigate {app, params}``
    (+ ``app.present {app}`` which the appwindow process uses to raise itself).
  * app mode        → the server process quits when the last app window closed.

Priority (layer/preview, SPEC §2.1): the host moves itself into its own systemd --user scope with
CPUWeight=1000 before any WebKit process starts (host/priority.py) and keeps that scope to itself
(``ScopeSweeper``; apps it launches get their own scope at once, apps.launch_context). An app-mode host started
for Taakbeheer (Ctrl+Shift+Esc without a desktop shell) gets such a scope too (``win11shell-apphost-<pid>.scope``).
A forced restart (win11shell-ctl restart --force, SPEC §2.2) leaves the app windows open: they reconnect to the
new host on the same port.

``shell.info`` also says ``mainLoopLagMs`` (since the main loop's last beat, host/lifeguard.py: the HTTP threads
answer while a hung GTK loop does not) and ``runId`` (new for every start of the host, also an in-place one).
"""

from __future__ import annotations

import json
import logging
import os
import re
import subprocess
import sys
import threading
import time
from pathlib import Path

from .. import APP_IDS, VERSION
from ..util import ApiError, hypr_dispatch, json_dumps, own_app, user_info

log = logging.getLogger("win11shell.shell")

PRIORITY = 1

KNOWN_COMMANDS = ("toggle-start", "toggle-search", "quick-settings", "notifications", "task-view", "alt-tab",
                  "winx", "show-desktop", "clipboard", "osd", "run", "power")
_COMMAND_RE = re.compile(r"^[a-z][a-z0-9-]{0,40}$")
_APP_GRACE_S = 1.5


def _clean_params(params) -> dict:
    if params is None:
        return {}
    if not isinstance(params, dict):
        raise ApiError("bad_args", "Ongeldige invoer.")
    try:
        if len(json_dumps(params)) > 64 * 1024:
            raise ApiError("bad_args", "Te veel gegevens.")
    except (TypeError, ValueError):
        raise ApiError("bad_args", "Ongeldige invoer.") from None
    return params


class _AppProc:
    __slots__ = ("app", "pid", "popen", "started")

    def __init__(self, app: str, pid: int, popen: subprocess.Popen | None):
        self.app = app
        self.pid = pid
        self.popen = popen
        self.started = time.monotonic()

    def alive(self) -> bool:
        if self.popen is not None:
            return self.popen.poll() is None
        try:  # adopted process: maybe our child (after a restart via exec), maybe not
            pid, _ = os.waitpid(self.pid, os.WNOHANG)
            if pid == self.pid:
                return False
        except ChildProcessError:
            pass
        except OSError:
            return False
        try:
            os.kill(self.pid, 0)
        except OSError:
            return False
        try:
            with open(f"/proc/{self.pid}/stat", "rb") as f:
                return b") Z " not in f.read(64)
        except OSError:
            return False


class AppWindows:
    """Tracks one appwindow process per app (layer/app mode)."""

    def __init__(self, api, ctx):
        self.api = api
        self.ctx = ctx
        self.procs: dict[str, _AppProc] = {}
        self.lock = threading.RLock()
        self.ever_started = False
        self.last_change = time.monotonic()
        self._monitor: threading.Thread | None = None

    # -- discovery ------------------------------------------------------------
    def adopt_existing(self) -> None:
        """Find appwindow processes already talking to our port (e.g. after shell.restart)."""
        uid = os.getuid()
        port = str(self.ctx.port)
        try:
            pids = [int(p) for p in os.listdir("/proc") if p.isdigit()]
        except OSError:
            return
        for pid in pids:
            if pid == os.getpid():
                continue
            try:
                if os.stat(f"/proc/{pid}").st_uid != uid:
                    continue
                with open(f"/proc/{pid}/cmdline", "rb") as f:
                    argv = f.read().decode("utf-8", "replace").split("\0")
            except OSError:
                continue
            if "host.appwindow" not in argv:
                continue
            app = _argv_value(argv, "--app")
            if not app or own_app(app) is None or _argv_value(argv, "--port") != port:
                continue
            app = own_app(app)
            with self.lock:
                if app not in self.procs:
                    log.info("Bestaand venster overgenomen (adopted window) %s pid %d", app, pid)
                    self.procs[app] = _AppProc(app, pid, None)
                    self.ever_started = True
        self._ensure_monitor()

    def running(self, app: str) -> _AppProc | None:
        with self.lock:
            p = self.procs.get(app)
            if p and p.alive():
                return p
            if p:
                self.procs.pop(app, None)
            return None

    def count(self) -> int:
        with self.lock:
            return sum(1 for p in self.procs.values() if p.alive())

    # -- open -------------------------------------------------------------------
    def open(self, app: str, params: dict, activation_token: str | None = None) -> dict:
        try:   # its main loop stopped turning (heartbeat): a fresh window instead of focusing a frozen one
            from ..ctl import replace_hung_window
            if replace_hung_window(app):
                log.warning("App-venster %s reageerde niet; beëindigd en opnieuw geopend (hung window replaced)", app)
        except Exception:  # noqa: BLE001
            log.debug("hung-window check failed", exc_info=True)
        with self.lock:
            proc = self.running(app)
            if proc is None:
                self.adopt_existing()
                proc = self.running(app)
            if proc is not None:
                self._focus(app, activation_token)
                self.api.emit("app.navigate", {"app": app, "params": params})
                return {"focused": True, "pid": proc.pid}
            if self.ctx.shutting_down:
                raise ApiError("shutting_down", "Win11 Shell wordt afgesloten. Probeer het zo opnieuw.")
            proc = self._spawn(app, params, activation_token)
            self.procs[app] = proc
            self.ever_started = True
            self.last_change = time.monotonic()
        self._ensure_monitor()
        return {"started": True, "pid": proc.pid}

    def _spawn(self, app: str, params: dict, activation_token: str | None) -> _AppProc:
        cmd = [sys.executable, "-m", "host.appwindow", "--app", app, "--port", str(self.ctx.port)]
        if params:
            cmd += ["--params", json.dumps(params, ensure_ascii=False)]
        if self.ctx.dev:
            cmd.append("--dev")
        from ..util import user_env
        env = user_env()          # not the switches of our own WebViews: the window decides its rendering itself
        root = str(self.ctx.root)
        env["PYTHONPATH"] = root + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        env["WIN11SHELL_TOKEN"] = self.ctx.token  # never on the command line (visible in ps)
        env["WIN11SHELL_CONFIG_DIR"] = str(self.ctx.config_dir)
        env["WIN11SHELL_CACHE_DIR"] = str(self.ctx.cache_dir)
        if self.ctx.dry_run:
            env["WIN11SHELL_DRY_RUN"] = "1"
        for k in ("XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"):
            env.pop(k, None)
        if activation_token:
            env["XDG_ACTIVATION_TOKEN"] = activation_token
            env["DESKTOP_STARTUP_ID"] = activation_token
        try:
            popen = subprocess.Popen(cmd, cwd=root, env=env, stdin=subprocess.DEVNULL, close_fds=True)
        except OSError as e:
            log.error("Kan %s niet openen (cannot start app window): %s", app, e)
            raise ApiError("spawn_failed", "Deze app kon niet worden geopend.", str(e)) from None
        log.info("App-venster gestart (app window started) %s pid %d", app, popen.pid)
        return _AppProc(app, popen.pid, popen)

    def _focus(self, app: str, activation_token: str | None) -> None:
        app_id = f"win11shell-{app}"
        if self.ctx.compositor == "hyprland":
            done = False
            if self.api.has("windows.list") and self.api.has("windows.focus"):
                try:
                    wl = self.api.call("windows.list", {}) or {}
                    for w in wl.get("windows", []):
                        if w.get("appId") == app_id:
                            self.api.call("windows.focus", {"id": w.get("id")})
                            done = True
                            break
                except ApiError as e:
                    log.debug("windows.focus mislukt (failed): %s", e)
            if not done:
                hypr_dispatch("focuswindow", f"class:^({re.escape(app_id)})$")
        payload = {"app": app}
        if activation_token:
            payload["activationToken"] = activation_token
        self.api.emit("app.present", payload)

    # -- lifetime ---------------------------------------------------------------
    def _ensure_monitor(self) -> None:
        if self._monitor is None or not self._monitor.is_alive():
            self._monitor = threading.Thread(target=self._watch, name="appwindows", daemon=True)
            self._monitor.start()

    def _watch(self) -> None:
        while not self.ctx.shutting_down:
            time.sleep(0.5)
            closed = []
            with self.lock:
                for app, p in list(self.procs.items()):
                    if not p.alive():
                        self.procs.pop(app, None)
                        closed.append(app)
                        self.last_change = time.monotonic()
                remaining = len(self.procs)
            for app in closed:
                log.info("App-venster gesloten (app window closed) %s", app)
                self.api.emit("shell.appClosed", {"app": app})
            if (self.ctx.mode == "app" and self.ever_started and remaining == 0
                    and time.monotonic() - self.last_change > _APP_GRACE_S):
                log.info("Geen app-vensters meer; afsluiten (no app windows left; exiting)")
                self.ctx.shutting_down = True
                self.ctx.request_quit(0)
                return

    def terminate_all(self) -> int:
        with self.lock:
            procs = list(self.procs.values())
        n = 0
        for p in procs:
            try:
                os.kill(p.pid, 15)
                n += 1
            except OSError:
                pass
        return n


def _argv_value(argv: list[str], flag: str) -> str | None:
    for i, a in enumerate(argv):
        if a == flag and i + 1 < len(argv):
            return argv[i + 1]
        if a.startswith(flag + "="):
            return a.split("=", 1)[1]
    return None


def register(api, ctx) -> None:
    windows = AppWindows(api, ctx)
    pending: list[dict] = []
    pending_lock = threading.Lock()

    # -- opening apps -------------------------------------------------------------
    def open_app(app, params=None, activation_token: str | None = None, **_kw):
        name = own_app(app)
        if name is None:
            raise ApiError("unknown_app", "Deze app bestaat niet.", {"app": app})
        params = _clean_params(params)
        if name == "explorer" and isinstance(params.get("path"), str) and params["path"].startswith("~"):
            params["path"] = os.path.expanduser(params["path"])
        if ctx.mode in ("preview", "serve"):
            payload = {"app": name, "params": params}
            if api.subscriber_count() == 0:
                with pending_lock:
                    pending.append(payload)
                return {"queued": True}
            api.emit("shell.openApp", payload)
            return {}
        return windows.open(name, params, activation_token)

    ctx.set_open_app(open_app)

    def flush_pending(sub):
        with pending_lock:
            if not pending:
                return
            items = list(pending)
            pending.clear()

        def later():
            if sub.closed:
                with pending_lock:
                    pending[:0] = items
                return
            for it in items:
                sub.push_obj("shell.openApp", it)

        t = threading.Timer(1.2, later)
        t.daemon = True
        t.start()

    api.on_connect(flush_pending)

    def on_shutdown():
        if ctx.restart_requested:
            return
        try:   # win11shell-ctl restart --force stopped us: the windows stay, the new host adopts them
            from ..ctl import forced_restart_pending
            if forced_restart_pending():
                log.info("Herstart op verzoek: app-vensters blijven open (forced restart: app windows stay)")
                return
        except Exception:  # noqa: BLE001
            pass
        n = windows.terminate_all()
        log.info("App-vensters gesloten bij afsluiten (app windows closed on exit): %d", n)

    if ctx.mode in ("layer", "app"):
        ctx.on_shutdown(on_shutdown)

    if ctx.mode in ("layer", "preview"):
        from .. import priority
        if priority.place_host():
            ctx._scope_sweeper = priority.ScopeSweeper()
    elif ctx.mode == "app" and getattr(ctx, "open_app_id", None) == "taskmgr":
        from .. import priority     # the fallback Taakbeheer's backend: the shell's priority (sysmon polls)
        if priority.place_app_host("taskmgr"):
            ctx._scope_sweeper = priority.ScopeSweeper()

    # -- methods ------------------------------------------------------------------
    def _layer_shell_installed(surfaces) -> bool:
        if surfaces is not None and getattr(surfaces, "gtk_ready", False):
            return bool(getattr(surfaces, "layer_installed", False))
        try:   # serve mode / no GTK: look the typelib up without importing Gtk
            from ..surfaces import layer_shell_installed
            return layer_shell_installed()
        except Exception:  # noqa: BLE001
            return False

    def _gpu_info() -> dict | None:
        try:   # importing host.surfaces imports no Gtk/WebKit
            from ..surfaces import gpu_info
            return gpu_info()
        except Exception:  # noqa: BLE001
            return None

    @api.method("shell.info")
    def _info(args):
        surfaces = ctx.surfaces
        return {
            "mode": ctx.mode,
            "compositor": ctx.compositor,
            "dryRun": ctx.dry_run,
            "version": VERSION,
            "layerShell": bool(surfaces is not None and getattr(surfaces, "layer_active", False)),
            "layerShellInstalled": _layer_shell_installed(surfaces),
            "user": user_info(),
            "apps": list(APP_IDS),
            "services": ctx.services,
            "dev": ctx.dev,
            "pid": os.getpid(),
            "gpu": _gpu_info(),
            "mainLoopLagMs": _loop_lag_ms(),
            "runId": getattr(ctx, "run_id", None),
            # layer mode could not start (host/problem.py): {code, packages, command, detail} or None
            "problem": _problem(surfaces),
        }

    def _problem(surfaces) -> dict | None:
        p = getattr(surfaces, "problem", None) if surfaces is not None else None
        if not isinstance(p, dict):
            return None
        return {k: p.get(k) for k in ("code", "packages", "command", "detail")}

    def _loop_lag_ms() -> int | None:
        beat = getattr(ctx, "loop_beat", None)
        if not isinstance(beat, (int, float)):
            return None
        return int(max(0.0, time.monotonic() - beat) * 1000)

    @api.method("shell.command")
    def _command(args):
        command = args.get("command")
        if not isinstance(command, str) or not _COMMAND_RE.match(command):
            raise ApiError("unknown_command", "Onbekende opdracht.", {"command": command})
        cargs = args.get("args") or {}
        if not isinstance(cargs, dict):
            raise ApiError("bad_args", "Ongeldige invoer.")
        if command not in KNOWN_COMMANDS:
            log.info("Onbekende shell-opdracht doorgegeven (unknown command passed on): %s", command)
        n = api.emit("shell.command", {"command": command, "args": cargs})
        return {"delivered": n}

    @api.method("shell.setRegions")
    def _regions(args):
        rects = args.get("rects") or []
        keyboard = args.get("keyboard")
        if not isinstance(rects, list):
            raise ApiError("bad_args", "Ongeldige invoer.")
        if keyboard not in (None, "none", "on-demand", "exclusive"):
            raise ApiError("bad_args", "Ongeldige toetsenbordmodus.")
        clean = []
        for r in rects[:64]:
            if not isinstance(r, dict):
                continue
            try:
                x, y, w, h = (int(round(float(r.get(k, 0)))) for k in ("x", "y", "w", "h"))
            except (TypeError, ValueError):
                continue
            if w > 0 and h > 0:
                clean.append({"x": x, "y": y, "w": w, "h": h})
        surfaces = ctx.surfaces
        if surfaces is not None and getattr(surfaces, "layer_active", False):
            # sync: answer once a keyboard change reached the compositor (tray: release before Activate)
            surfaces.set_regions(clean, keyboard, sync=args.get("sync") is True)
        return {}

    @api.method("shell.openApp")
    def _open(args):
        tok = args.get("activationToken")
        return open_app(args.get("app"), args.get("params"),
                        activation_token=tok if isinstance(tok, str) and len(tok) < 512 else None)

    @api.method("shell.pickFile")
    def _pick(args):
        surfaces = ctx.surfaces
        if ctx.mode == "serve" or surfaces is None or not getattr(surfaces, "gtk_ready", False):
            return {"path": None}
        filters = args.get("filters") or []
        if not isinstance(filters, list):
            filters = []
        path = surfaces.pick_file(title=args.get("title"), filters=filters, folder=bool(args.get("folder")))
        return {"path": path}

    @api.method("shell.restart", dangerous=True)
    def _restart(args):
        log.info("Herstart aangevraagd (restart requested)")
        t = threading.Timer(0.3, ctx.request_restart)
        t.daemon = True
        t.start()
        return {}

    @api.method("shell.quit")
    def _quit(args):
        log.info("Afsluiten aangevraagd (quit requested)")
        t = threading.Timer(0.3, ctx.request_quit)
        t.daemon = True
        t.start()
        return {}

    ctx._app_windows = windows


def start(api, ctx) -> None:
    windows = getattr(ctx, "_app_windows", None)
    if windows is not None and ctx.mode in ("layer", "app"):
        windows.adopt_existing()
    sweeper = getattr(ctx, "_scope_sweeper", None)
    if sweeper is not None:
        sweeper.start()


def stop(api, ctx) -> None:
    sweeper = getattr(ctx, "_scope_sweeper", None)
    if sweeper is not None:
        sweeper.stop.set()
