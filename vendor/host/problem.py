"""Native notice: the desktop (layer mode) cannot start inside a Win11 Shell session (SPEC §2, "Problem notice").

User requirement (Robin, 2026-10-27): "I don't want to see win11shell running in a window again" — so in the real
session a missing package never ends in the silent preview window. Instead this plain GTK 3 window (no WebKit, no
cairo foreign: exactly those may be what is missing) says what is wrong, shows the apt command (selectable, with
[Kopiëren]) and offers [Installeren in Terminal…], [Voorbeeldweergave] (only on request, greyed out when WebKit is
missing), [Afmelden] and [Opnieuw proberen]. Every POLL_S the preflight runs in a subprocess (a failed import is
never retried inside this process); once the part is there the host restarts in place into layer mode after a
AUTOSTART_S countdown — automatically at most once per missing package set (RETRY_ENV survives the exec), after
that only with the button. Design: notes/session-failure-ux.md §6.3.

<runtime>/problem.json (0600) carries the same problem for bin/win11shell, ``win11shell-ctl status`` and
shell.info; it is removed as soon as layer mode runs."""
from __future__ import annotations

import json
import logging
import os
import subprocess
import sys
import threading

from . import ROOT
from .util import hyprctl, runtime_dir, spawn, which

log = logging.getLogger("win11shell.problem")
POLL_S = 5                    # how often the preflight runs while the notice is open
AUTOSTART_S = 10              # countdown once the missing part is there
RETRY_ENV = "WIN11SHELL_LAYER_RETRY"   # package set an automatic restart was made for (survives the exec)
FIELDS = ("code", "packages", "command", "detail")

T = {
    "nl": {"title": "Het bureaublad van Win11 Shell kan niet starten",
           "missing": "Er ontbreekt een onderdeel: {parts}.",
           "fix": "Installeer het met deze opdracht; daarna start het bureaublad vanzelf:",
           "watch": "Ik kijk elke 5 seconden of het er is.",
           "found": "Gevonden. Het bureaublad start over {n} s…",
           "found_manual": "Gevonden. Klik op Nu starten.",
           "no_wayland": "Win11 Shell draait hier niet onder Wayland. Meld je af en kies bij het inloggen Win11 Shell.",
           "no_protocol": "Deze vensterbeheerder kan de taakbalk niet als vaste laag tonen (geen wlr-layer-shell).",
           "failed": "Het bureaublad kon niet worden opgebouwd ({detail}). Probeer het opnieuw; lukt het niet, "
                     "stuur dan ~/.cache/win11shell/host.log mee.",
           "keys": "Ctrl+Alt+T opent een terminal, Ctrl+Alt+Shift+Delete meldt je meteen af.",
           "retry": "Opnieuw proberen", "start_now": "Nu starten",
           "terminal": "Installeren in Terminal…", "copy": "Kopiëren", "preview": "Voorbeeldweergave",
           "logout": "Afmelden", "no_terminal": "Geen terminal gevonden. Open er zelf een en typ:"},
    "en": {"title": "The Win11 Shell desktop cannot start",
           "missing": "A component is missing: {parts}.",
           "fix": "Install it with this command; the desktop then starts by itself:",
           "watch": "Checking every 5 seconds.", "found": "Found. The desktop starts in {n} s…",
           "found_manual": "Found. Click Start now.",
           "no_wayland": "Win11 Shell is not running under Wayland here. Log out and choose Win11 Shell.",
           "no_protocol": "This compositor cannot show the taskbar as a layer (no wlr-layer-shell).",
           "failed": "The desktop could not be built ({detail}). Try again; if it keeps failing, "
                     "send ~/.cache/win11shell/host.log.",
           "keys": "Ctrl+Alt+T opens a terminal, Ctrl+Alt+Shift+Delete signs you out at once.",
           "retry": "Try again", "start_now": "Start now", "terminal": "Install in Terminal…",
           "copy": "Copy", "preview": "Preview", "logout": "Sign out",
           "no_terminal": "No terminal found. Open one yourself and type:"},
}


def problem_path():
    return runtime_dir() / "problem.json"


def write_problem(p: dict | None) -> None:
    """<runtime>/problem.json (0600, atomic like write_instance) or remove it (None)."""
    path = problem_path()
    try:
        if p is None:
            path.unlink(missing_ok=True)
            return
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        tmp = path.with_suffix(".tmp")
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_CLOEXEC, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(public(p), f, ensure_ascii=False)
        os.replace(tmp, path)
    except OSError:
        log.debug("problem.json niet geschreven", exc_info=True)


def read_problem() -> dict | None:
    try:
        with open(problem_path(), encoding="utf-8") as f:
            d = json.load(f)
        return d if isinstance(d, dict) else None
    except (OSError, ValueError):
        return None


def public(p: dict | None) -> dict | None:
    """What shell.info / problem.json show: {code, packages, command, detail}."""
    if not isinstance(p, dict):
        return None
    return {k: p.get(k) for k in FIELDS}


def terminal_argv(cmd: list[str]) -> list[str] | None:
    exe = which("xdg-terminal-exec")
    if exe:
        return [exe, *cmd]
    for t, pre in (("ptyxis", ["--"]), ("gnome-terminal", ["--"]), ("kgx", ["--"]), ("kitty", []), ("foot", []),
                   ("alacritty", ["-e"]), ("konsole", ["-e"]), ("xterm", ["-e"])):
        exe = which(t)
        if exe:
            return [exe, *pre, *cmd]
    return None


def preflight_ok(timeout: float = 30.0) -> bool:
    """The preflight in a fresh interpreter (never in this process: a failed import stays failed here)."""
    env = {k: v for k, v in os.environ.items() if k not in ("WAYLAND_DISPLAY", "DISPLAY", "WAYLAND_SOCKET")}
    env["NO_AT_BRIDGE"] = "1"
    try:
        return subprocess.run([sys.executable, "-I", str(ROOT / "host" / "preflight.py"), "--scope", "layer",
                               "--quiet"], env=env, timeout=timeout, stdin=subprocess.DEVNULL,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode == 0
    except (OSError, subprocess.SubprocessError):
        return False


class ProblemNotice:
    """The window. ``on_preview`` opens the preview window on request (None: the button is greyed out);
    ``preview_open`` says whether it is open (then closing the notice only hides it)."""

    def __init__(self, ctx, problem: dict, *, can_preview: bool, on_preview, preview_open=lambda: False):
        self.ctx, self.p, self.can_preview, self.on_preview = ctx, problem, can_preview, on_preview
        self.preview_open = preview_open
        lang = "nl"
        try:
            lang = ctx.settings.get("language", "nl") if getattr(ctx, "settings", None) else "nl"
        except Exception:  # noqa: BLE001
            pass
        self.en = lang == "en"
        self.t = T["en" if self.en else "nl"]
        self.key = " ".join(problem.get("packages") or [])
        self.auto = bool(self.key) and os.environ.get(RETRY_ENV) != self.key   # one automatic restart per set
        self._busy = False
        self._found = False
        self._left = AUTOSTART_S
        self._restarting = False
        self.win = self.status = self.retry = None

    def text(self) -> str:
        code, t = self.p.get("code"), self.t
        if code == "missing":
            parts = ", ".join(f"{m['en' if self.en else 'nl']} ({m['package']})" for m in self.p.get("missing", []))
            return t["missing"].format(parts=parts or ", ".join(self.p.get("packages") or []))
        if code == "layer_failed":
            return t["failed"].format(detail=self.p.get("detail") or "?")
        return t.get(code) or t["failed"].format(detail=code)

    def status_text(self) -> str:
        return (self.t["watch"] + " " if self.key else "") + self.t["keys"]

    def show(self) -> None:
        from gi.repository import Gdk, GLib, Gtk
        t = self.t
        win = Gtk.Window(title=t["title"])
        win.set_resizable(False)                 # fixed size → Hyprland floats it (and our config floats all)
        win.set_position(Gtk.WindowPosition.CENTER)
        win.set_keep_above(True)
        win.set_icon_name("dialog-warning")
        box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=12, margin=24)
        head = Gtk.Box(spacing=12)
        head.pack_start(Gtk.Image.new_from_icon_name("dialog-warning", Gtk.IconSize.DIALOG), False, False, 0)
        title = Gtk.Label(xalign=0)
        title.set_markup("<span size='large' weight='bold'>%s</span>" % GLib.markup_escape_text(t["title"]))
        head.pack_start(title, True, True, 0)
        box.pack_start(head, False, False, 0)
        box.pack_start(Gtk.Label(label=self.text(), wrap=True, xalign=0, max_width_chars=64, selectable=True),
                       False, False, 0)
        if self.p.get("command"):
            box.pack_start(Gtk.Label(label=t["fix"], wrap=True, xalign=0), False, False, 0)
            row = Gtk.Box(spacing=6)
            cmd = Gtk.Entry(text=self.p["command"], editable=False, hexpand=True)
            cmd.get_style_context().add_class("monospace")
            copy = Gtk.Button(label=t["copy"])
            copy.connect("clicked", lambda *_: Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD).set_text(cmd.get_text(), -1))
            row.pack_start(cmd, True, True, 0)
            row.pack_start(copy, False, False, 0)
            box.pack_start(row, False, False, 0)
        self.status = Gtk.Label(xalign=0, wrap=True, max_width_chars=64)
        self.status.get_style_context().add_class("dim-label")
        self.status.set_text(self.status_text())
        box.pack_start(self.status, False, False, 0)
        bb = Gtk.ButtonBox(layout_style=Gtk.ButtonBoxStyle.END, spacing=6)
        if self.key:
            b = Gtk.Button(label=t["terminal"])
            b.connect("clicked", lambda *_: self._terminal())
            bb.add(b)
        b = Gtk.Button(label=t["preview"], sensitive=bool(self.can_preview and self.on_preview))
        b.connect("clicked", lambda *_: self._preview())
        bb.add(b)
        b = Gtk.Button(label=t["logout"])
        b.connect("clicked", lambda *_: self._logout())
        bb.add(b)
        self.retry = Gtk.Button(label=t["retry"], can_default=True)
        self.retry.get_style_context().add_class("suggested-action")
        self.retry.connect("clicked", lambda *_: self._restart())
        bb.add(self.retry)
        box.pack_end(bb, False, False, 0)
        win.add(box)
        win.connect("delete-event", lambda *_: self._on_close())
        win.show_all()
        self.retry.grab_default()
        self.win = win
        log.info("Uitleg getoond: %s (problem notice shown)", self.p.get("command") or self.p.get("code"))
        # belt and braces: Hyprland's own overlay works even if this window is behind something
        hyprctl(f"notify 0 15000 0 Win11 Shell: {t['title']} — {self.p.get('command') or self.text()}")
        if self.key:
            GLib.timeout_add_seconds(POLL_S, self._poll)

    def _on_close(self) -> bool:
        """With the preview open the notice may go (the problem stays in shell.info / ctl status); without it the
        notice is all there is on screen, so it stays (Afmelden / Opnieuw proberen are the ways out)."""
        if self.preview_open():
            self.win.hide()
        return True

    # -- the missing part appears -----------------------------------------------------------------
    def _poll(self) -> bool:
        if self._found or self._restarting:
            return False
        if not self._busy:
            self._busy = True
            threading.Thread(target=self._check, daemon=True, name="problem-poll").start()
        return True

    def _check(self) -> None:
        from gi.repository import GLib
        GLib.idle_add(self._result, preflight_ok())

    def _result(self, ok: bool) -> bool:
        self._busy = False
        if ok and not self._found:
            self._found = True
            log.info("Ontbrekend onderdeel gevonden (%s); bureaublad start opnieuw (missing part found)", self.key)
            if self.retry is not None:
                self.retry.set_label(self.t["start_now"])
            if self.win is not None and not self.win.get_visible():
                self.win.show_all()
            if self.auto:
                from gi.repository import GLib
                self.status.set_text(self.t["found"].format(n=self._left))
                GLib.timeout_add_seconds(1, self._tick)
            else:
                log.info("Al eens automatisch opnieuw gestart voor %s; nu alleen met de knop "
                         "(already retried automatically once for this set)", self.key)
                self.status.set_text(self.t["found_manual"])
        return False

    def _tick(self) -> bool:
        if self._restarting:
            return False
        self._left -= 1
        self.status.set_text(self.t["found"].format(n=max(self._left, 0)))
        if self._left <= 0:
            self._restart()
            return False
        return True

    # -- buttons ------------------------------------------------------------------------------------
    def _restart(self) -> None:
        if self._restarting:
            return
        self._restarting = True
        if self.key:
            os.environ[RETRY_ENV] = self.key      # exec keeps the environment: no second automatic restart
        log.info("Opnieuw proberen: Win11 Shell start opnieuw in bureaubladmodus (retry layer mode)")
        self.ctx.request_restart()               # exec in place: --mode layer (mode_requested), same port/token

    def _preview(self) -> None:
        if self.on_preview and self.can_preview:
            self.on_preview()

    def _logout(self) -> None:
        if getattr(self.ctx, "dry_run", False):
            log.info("[proef] afmelden: hyprctl dispatch exit (dry-run: sign-out not done)")
            return
        log.info("Afmelden vanuit de uitleg (sign out from the problem notice)")
        hyprctl("dispatch exit")

    def _terminal(self) -> None:
        tool = ROOT / "config" / "session" / "win11shell-session"
        argv = terminal_argv([str(tool), "fix-packages", *(self.p.get("packages") or [])])
        if argv is None:
            self.status.set_text(f"{self.t['no_terminal']} {self.p.get('command') or ''}")
            return
        if getattr(self.ctx, "dry_run", False):
            log.info("[proef] zou starten (dry-run, would start): %s", " ".join(argv))
            return
        if spawn(argv) is None:
            self.status.set_text(f"{self.t['no_terminal']} {self.p.get('command') or ''}")
