"""One app window per process (own Wayland app_id ``win11shell-<app>``).

    python3 -m host.appwindow --app taskmgr --port P [--token T] [--params JSON] [--dev]

The token may also come from $WIN11SHELL_TOKEN (preferred: not visible in ``ps``) or the
instance file. The window is undecorated + RGBA; the page draws the Win11 frame
(js/apps/appframe.js) and talks to us through ``window.webkit.messageHandlers.win11``:

    {cmd: 'minimize' | 'toggleMaximize' | 'maximize' | 'restore' | 'close' | 'dragStart'}
    {cmd: 'titlebar', height: 32, noDragRects: [{x, y, w, h}, ...]}   (CSS px, viewport-relative)
    {cmd: 'setTitle', title}
    {cmd: 'restartShell'}     restart Win11 Shell ourselves (``win11shell-ctl restart --force``, not through the
                              host: it works when the host does not answer at all)

We tell the page about window state with
    window.dispatchEvent(new CustomEvent('win11:windowstate', {detail: {maximized, focused, fullscreen, tiled}}))
plus ``<html data-window-maximized="1|0" data-window-focused="1|0">`` and ``window.__win11WindowState``,
and about the host with ``win11:hoststate`` ``{ok, mode, restarting}`` (+ ``window.__win11HostState``).
Closing the window (title-bar ×, Alt+F4, the compositor's close, ``{cmd:'close'}``) first dispatches ``win11:close``
in the page and destroys the window CLOSE_GRACE_MS later: the page can still send its last (keepalive) requests —
the Bluetooth dialog cancels a pairing question and the device search (SPEC §6.10).

Never stuck (SPEC §2.2) — every app window:
* **hang watchdog**: a JS probe every 2 s; no answer for 10 s → the web process is killed and the page
  reloads; hangs again within a minute → the window process restarts itself (≤ 2× per 5 min, then a notice
  offers it by hand). Not while the host is down (then the page cannot load, it is not hung). No probe while the
  view has no web process (that is no unanswered probe); the first answer after a load commit may take 30 s; memory
  or I/O thrashing postpones a verdict (≤ 30 s). With WIN11SHELL_GPU=safe the 2nd verdict within 10 min gives the GPU
  up and restarts the window without it (surfaces.note_page_hang).
* **host health**: ``shell.info`` every 4 s (2 s while failing). Two misses in a row = "Win11 Shell reageert
  niet" — an answer whose ``mainLoopLagMs`` is above 8 s is a miss too (the host's HTTP threads answer while its
  GTK main loop, and with it the taskbar, stands still; host/lifeguard.py): a native GTK notice over the page
  (it also works when the page never loaded) offers
  "Win11 Shell opnieuw starten"; Taakbeheer's own page shows that itself while it runs. The host back →
  the page is reloaded if it never loaded; a new host on another port/token (instance file) → the window
  process restarts itself onto it.
* Taakbeheer renders without the GPU, always (WebKit's DMA-BUF renderer off, hardware acceleration
  policy NEVER, Skia CPU raster), whatever ``no-gpu`` says.
* each window moves itself into its own systemd --user scope (Taakbeheer: CPUWeight=1000), see host/priority.py.
* one window per app: a second process for the same app asks the first to show itself (RAISE_SIGNAL = SIGRTMIN+2)
  and exits.
* never a SIGUSR1 handler in a process with WebKit (SIGUSR2 avoided as well): JavaScriptCore suspends threads with
  SIGUSR1 for its garbage collector (WTF, ``JSC_SIGNAL_FOR_GC``). Our GLib handler on SIGUSR1 (the earlier "raise"
  signal, installed after the WebView) replaced JSC's: the first GC of the UI process's JS heap then waited for
  ever — main thread in a futex, a WebKit thread spinning at 100 % — and the window froze 1–6 minutes after it
  opened (2026-10-27; A/B: tests/nested/glue/jsc_gc_signal.py usr1 hangs, rt/usr2 do not).
* main-loop heartbeat: every HEARTBEAT_S the main loop touches ``<runtime>/app-<app>.lock``. The file says
  ``<pid> start`` while the window starts up (GTK, WebKit, its scope: slow under memory pressure) and ``<pid> hb``
  from the first turn of the main loop on. A window whose main loop stopped turning cannot help itself (its timers
  no longer run — seen with a C call into WebKit/the NVIDIA driver that never returned); opening the app again
  (Ctrl+Shift+Esc, Start, ``shell.openApp``) then ends that process and starts a fresh one
  (``ctl.replace_hung_window``: no heartbeat for 12 s and none while it watched another 2.5 s; a window still
  starting only after 60 s) instead of focusing a frozen window.
* ``--dev`` (or WIN11SHELL_DEBUG_HANG=1): SIGRTMIN+6 puts the page into an endless JS loop (to test the watchdog),
  SIGRTMIN+4 does what the notice's restart button does, SIGRTMIN+5 runs ``<runtime>/devjs-<app>.js`` (ours, 0600)
  in the page and writes its answer to ``<cache>/devjs-<app>.json`` (works while the compositor sends no frames,
  e.g. a nested test Hyprland that is not on screen); SIGRTMIN+3 (always) dumps every thread's Python stack.

Title-bar dragging, double-click maximise, right-click window menu and 6 px edge resizing are
done natively (GTK button/motion handlers run before WebKit's). Alt+drag moves from anywhere.

Taakbeheer follows ``settings.taskmgr.alwaysOnTop`` (at start and on ``settings.changed``): under
Hyprland through ``windows.setPinned`` (Hyprland ``pin``: drawn above other windows, on every
workspace), elsewhere with GTK keep-above (works on X11; Wayland has no protocol for it).
"""

from __future__ import annotations

import argparse
import contextlib
import fcntl
import http.client
import json
import logging
import os
import signal
import subprocess
import sys
import threading
import time
from pathlib import Path
from urllib.parse import urlencode

log = logging.getLogger("win11shell.appwindow")

DEFAULT_SIZES = {"settings": (1100, 740), "taskmgr": (1040, 720), "explorer": (1120, 720), "alles": (1040, 680),
                 "disks": (1060, 760)}
MIN_SIZE = (640, 480)
EDGE = 6
CORNER = 14
TITLES = {
    "nl": {"settings": "Instellingen", "taskmgr": "Taakbeheer", "explorer": "Verkenner", "alles": "Alles",
           "disks": "Schijven"},
    "en": {"settings": "Settings", "taskmgr": "Task Manager", "explorer": "File Explorer", "alles": "Alles",
           "disks": "Disks"},
}
ICONS = {"settings": "preferences-system", "taskmgr": "utilities-system-monitor",
         "explorer": "system-file-manager", "alles": "system-search", "disks": "org.gnome.DiskUtility"}

WATCHDOG_TICK_S = 2
CLOSE_GRACE_MS = 200          # delete-event → 'win11:close' in the page → destroy this much later
SNAP_CHECK_MS = 400           # Hyprland: a size change settled this long → ask windows.list whether we are snapped
HANG_S = 10.0                 # the page did not answer the probe this long → hung
FIRST_ANSWER_S = 30.0         # … the first probe after a load commit (its web process may still be starting)
BUSY_MAX_S = 30.0             # memory/IO thrashing postpones a verdict, to at most this long after the probe
RECOVER_WINDOW_S = 60.0       # hung again this soon after a reload → restart the window process
HEALTH_OK_S = 4.0             # host health poll
HEALTH_FAIL_S = 2.0
HOST_DOWN_AFTER = 2           # misses in a row
MAX_SELF_RESTARTS = 2         # per SELF_RESTART_WINDOW_S
SELF_RESTART_WINDOW_S = 300
RESTARTS_ENV = "WIN11SHELL_APPWINDOW_RESTARTS"
HEARTBEAT_S = 1               # main loop → lock file mtime (ctl.replace_hung_window reads it)
RAISE_SIGNAL = signal.SIGRTMIN + 2   # "show yourself" from a second process (never SIGUSR1/2: JSC's GC uses them)
HANG_SIGNAL = signal.SIGRTMIN + 6    # --dev test aid: endless loop in the page
_ORIG_ENV: dict[str, str] = {}   # our environment as we got it (for ctl and for restarting ourselves)

NOTICE = {
    "nl": {
        "host_down_t": "Win11 Shell reageert niet",
        "host_down_m": "{app} krijgt geen antwoord van Win11 Shell. Start Win11 Shell opnieuw; je open "
                       "vensters en apps blijven open.",
        "restarting_t": "Win11 Shell wordt opnieuw gestart…",
        "restarting_m": "Een ogenblik geduld. Dit venster gaat daarna gewoon verder.",
        "hung_t": "{app} reageert niet",
        "hung_m": "De pagina wordt opnieuw geladen…",
        "reopen_t": "{app} reageert niet",
        "reopen_m": "{app} wordt opnieuw gestart…",
        "stuck_t": "{app} blijft vastlopen",
        "stuck_m": "Opnieuw laden hielp niet. Probeer het nog eens, of start Win11 Shell opnieuw.",
        "no_shell_t": "De taakbalk draait niet",
        "no_shell_m": "Win11 Shell (bureaublad, taakbalk en Start) is gestopt.",
        "restart": "Win11 Shell opnieuw starten", "start_shell": "Win11 Shell starten",
        "reload": "Opnieuw laden", "close": "Sluiten", "later": "Later",
    },
    "en": {
        "host_down_t": "Win11 Shell is not responding",
        "host_down_m": "{app} gets no answer from Win11 Shell. Restart Win11 Shell; your open windows and apps "
                       "stay open.",
        "restarting_t": "Restarting Win11 Shell…",
        "restarting_m": "One moment please. This window carries on afterwards.",
        "hung_t": "{app} is not responding",
        "hung_m": "Reloading the page…",
        "reopen_t": "{app} is not responding",
        "reopen_m": "Restarting {app}…",
        "stuck_t": "{app} keeps hanging",
        "stuck_m": "Reloading did not help. Try again, or restart Win11 Shell.",
        "no_shell_t": "The taskbar is not running",
        "no_shell_m": "Win11 Shell (desktop, taskbar and Start) has stopped.",
        "restart": "Restart Win11 Shell", "start_shell": "Start Win11 Shell",
        "reload": "Reload", "close": "Close", "later": "Later",
    },
}

_CSS = b"""
.w11-scrim { background-color: rgba(0, 0, 0, 0.32); border-radius: 8px; }
.w11-scrim.w11-solid { background-color: #f3f3f3; }
.w11-dark .w11-scrim.w11-solid, .w11-scrim.w11-solid.w11-dark { background-color: #202020; }
.w11-card { background-color: #fbfbfb; color: #1a1a1a; border-radius: 8px; border: 1px solid rgba(0,0,0,0.12);
            padding: 22px 24px 20px 24px; box-shadow: 0 8px 24px rgba(0,0,0,0.28); }
.w11-card.w11-dark { background-color: #2b2b2b; color: #ffffff; border-color: rgba(255,255,255,0.08); }
.w11-card .w11-title { font-size: 15pt; font-weight: 600; }
.w11-card .w11-msg { font-size: 10.5pt; }
.w11-card button { border-radius: 4px; padding: 5px 14px; min-height: 22px; background-image: none;
                   background-color: #ffffff; color: #1a1a1a; border: 1px solid rgba(0,0,0,0.14); box-shadow: none; }
.w11-card.w11-dark button { background-color: #373737; color: #ffffff; border-color: rgba(255,255,255,0.09); }
.w11-card button.w11-accent { background-color: #005fb8; color: #ffffff; border-color: #005fb8; }
.w11-card.w11-dark button.w11-accent { background-color: #4cc2ff; color: #000000; border-color: #4cc2ff; }
.w11-bar { background-color: #fff4ce; color: #1a1a1a; border-radius: 0 0 8px 8px; padding: 8px 14px; }
.w11-bar.w11-dark { background-color: #433519; color: #ffffff; }
.w11-bar button { border-radius: 4px; padding: 3px 12px; background-image: none; box-shadow: none; }
"""


def _config_dir() -> Path:
    v = os.environ.get("WIN11SHELL_CONFIG_DIR")
    if v:
        return Path(os.path.expanduser(v))
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config")
    return Path(base) / "win11shell"


def _settings_file() -> dict:
    try:
        data = json.loads((_config_dir() / "settings.json").read_text(encoding="utf-8"))
        return data if isinstance(data, dict) else {}
    except (OSError, ValueError):
        return {}


def _language() -> str:
    return "en" if _settings_file().get("language") == "en" else "nl"


def _dark(settings: dict | None) -> bool:
    th = (settings or {}).get("theme")
    return not (isinstance(th, dict) and th.get("mode") == "light")


def _always_on_top(settings: dict | None) -> bool:
    tm = (settings or {}).get("taskmgr")
    return bool(tm.get("alwaysOnTop")) if isinstance(tm, dict) else False


class WindowState:
    """Remembered window sizes per app (<config>/window-state.json, shared by all apps). ``save`` is a read-modify-write
    under an exclusive flock on ``.window-state.lock``: when the host quits it SIGTERMs every app window in one loop,
    and without the lock each read the same old file and the last writer dropped the others' new sizes. A lock that
    is not free within LOCK_WAIT_S (a writer stuck on a full disk) is not waited for: then the old race, not a
    window that will not close."""
    LOCK_WAIT_S = 1.5

    def __init__(self, app: str):
        self.app = app
        self.path = _config_dir() / "window-state.json"

    def load(self) -> dict:
        try:
            data = json.loads(self.path.read_text(encoding="utf-8"))
            st = data.get(self.app) if isinstance(data, dict) else None
            return st if isinstance(st, dict) else {}
        except (OSError, ValueError):
            return {}

    @contextlib.contextmanager
    def _locked(self):
        fd = None
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            fd = os.open(self.path.with_name(".window-state.lock"), os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, 0o600)
            deadline = time.monotonic() + self.LOCK_WAIT_S
            while True:
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    break
                except BlockingIOError:
                    if time.monotonic() >= deadline:
                        log.debug("Vensterstatus-slot bezet; zonder slot (window state lock busy)")
                        break
                    time.sleep(0.02)
        except OSError as e:
            log.debug("Vensterstatus-slot niet beschikbaar (window state lock unavailable): %s", e)
        try:
            yield
        finally:
            if fd is not None:
                os.close(fd)             # releases the flock

    def save(self, state: dict) -> None:
        from .util import atomic_write_json, read_json
        try:
            with self._locked():
                data = read_json(self.path, {}) or {}
                if not isinstance(data, dict):
                    data = {}
                data[self.app] = state
                atomic_write_json(self.path, data, pretty=True)
        except OSError as e:
            log.debug("Vensterstatus niet bewaard (window state not saved): %s", e)


class HangWatchdog:
    """Is the page alive? ``tick()`` every ``tick_s`` (WATCHDOG_TICK_S) starts a probe (``probe(done)`` calls
    ``done()`` when the page answered). A probe unanswered for ``hang_s`` = hung: first ``reload()`` (which kills
    the web process); hung again within ``recover_s`` of that reload → ``restart()`` (the window process) while
    ``may_restart()`` allows it, else the stage becomes 'stuck'. ``host_ok()`` False: no verdict (a page that
    cannot reach its host is not hung). ``notify(stage)`` with 'hung' | 'reopen' | 'stuck' | 'ok'.

    A tick that comes more than 2× ``tick_s`` late means our own main loop did not run (swap storm, a blocking
    call): the page's answer may be queued right behind this tick (WebKit delivers it at a lower priority than our
    timer), so the probe is judged a full ``hang_s`` later instead — a busy system is not a hung page. The same when
    ``loop_stalled(since)`` (the host's own pages: lifeguard.loop_stalled_since, beat by beat) says the loop stood
    still after the probe was sent, even if the tick itself was on time.
    ``unanswered_since``: when the oldest probe still without an answer was sent (also while the host is down);
    cleared only by an answer — the window's notices use it, so they do not blink with every new probe.

    ``probe()`` returning False = nothing was asked (the view has no web process right now: crashed or stopped, not
    reloaded yet — WebKit would fail the call at once). That is no unanswered probe: the next tick asks again (a
    probe lost in the second between a crash and the reload's commit made the watchdog kill the freshly reloaded,
    healthy page 10 s later). Only a view that *stays* without a web process for ``hang_s`` (a crash loop's long
    back-off) is judged hung. ``page_committed()`` (load-changed COMMITTED): a probe sent to the page before can only
    time out — it is forgotten, and the new page's first answer may take ``first_s`` (its web process may still be
    starting: a cold start at login, paged out under memory pressure; a reload would only repeat the slow path).
    ``busy()`` True (the machine thrashes: lifeguard.system_thrashing) postpones a verdict to at most ``busy_max_s``
    after the probe — a slow page is not a hung one; CPU time is no progress signal (the 2026-10-26 hang spun)."""

    def __init__(self, probe, reload, restart, may_restart, notify, host_ok=lambda: True, clock=time.monotonic,
                 hang_s: float = HANG_S, recover_s: float = RECOVER_WINDOW_S, tick_s: float = WATCHDOG_TICK_S,
                 loop_stalled=None, first_s: float | None = None, busy=None, busy_max_s: float = BUSY_MAX_S):
        self.probe, self.reload, self.restart, self.may_restart = probe, reload, restart, may_restart
        self.notify, self.host_ok, self.clock = notify, host_ok, clock
        self.loop_stalled, self.busy = loop_stalled, busy
        self.hang_s, self.recover_s, self.tick_s = hang_s, recover_s, tick_s
        self.first_s = hang_s if first_s is None else max(hang_s, first_s)
        self.busy_max_s = busy_max_s
        self.pending: float | None = None
        self.unanswered_since: float | None = None
        self.dead_since: float | None = None   # probes could not be sent since (no web process)
        self._fresh = True                     # no answer yet since the start / the last load commit
        self._token: object | None = None
        self._last_tick: float | None = None
        self.last_reload = -1e9
        self.stage = "ok"

    def _stalled(self, since: float, late: bool) -> bool:
        return late or (self.loop_stalled is not None and self.loop_stalled(since))

    def _overdue(self, now: float, since: float) -> bool:
        limit = self.first_s if self._fresh else self.hang_s
        if now - since < limit:
            return False
        if self.busy is not None and now - since < self.busy_max_s:
            try:
                if self.busy():
                    return False               # thrashing: a slow page, not a hung one (bounded by busy_max_s)
            except Exception:  # noqa: BLE001
                pass
        return True

    def _verdict(self, now: float, since: float) -> None:
        if self.unanswered_since is None:
            self.unanswered_since = since
        if self.host_ok():
            self._hung(now)

    def tick(self) -> None:
        now = self.clock()
        last, self._last_tick = self._last_tick, now
        late = last is not None and now - last > 2 * self.tick_s
        if self.pending is not None:
            if self._stalled(self.pending, late):
                self.pending = now            # our loop stood still: give the (maybe queued) answer its chance
                return
            if not self._overdue(now, self.pending):
                return
            since, self.pending, self._token = self.pending, None, None
            self._verdict(now, since)
            return
        token = object()
        self.pending, self._token = now, token
        try:
            sent = self.probe(lambda: self._answered(token))
        except Exception:  # noqa: BLE001 - no page to ask: nothing hangs
            self.pending, self._token, self.dead_since = None, None, None
            return
        if sent is not False:
            self.dead_since = None
            return
        if self._token is token:              # nothing was asked: nothing can be unanswered — ask again next tick
            self.pending, self._token = None, None
        if self.dead_since is None or self._stalled(self.dead_since, late):
            self.dead_since = now
            return
        if now - self.dead_since < self.hang_s:
            return
        since, self.dead_since = self.dead_since, None
        self._verdict(now, since)              # still no web process after hang_s (a crash loop's back-off)

    def page_committed(self) -> None:
        """A new page committed in the view (the reload after a crash or after our own verdict)."""
        self.pending, self._token, self.dead_since = None, None, None
        self._fresh = True

    def _answered(self, token) -> None:
        if token is not self._token:
            return
        self.pending, self._token = None, None
        self.unanswered_since = None
        self._fresh = False
        if self.stage != "ok" and self.clock() - self.last_reload > 1.0:
            self.stage = "ok"
            self.notify("ok")

    def _hung(self, now: float) -> None:
        if now - self.last_reload > self.recover_s:
            self.last_reload = now
            self.stage = "hung"
            self.notify("hung")
            self.reload()
        elif self.may_restart():
            self.stage = "reopen"
            self.notify("reopen")
            self.restart()
        else:
            self.stage = "stuck"
            self.notify("stuck")


class HostHealth:
    """Answers of the host's ``shell.info`` → down after ``down_after`` misses in a row."""

    def __init__(self, down_after: int = HOST_DOWN_AFTER):
        self.down_after = down_after
        self.fails = 0
        self.mode: str | None = None
        self.answered = False          # the host answered at least once

    @property
    def down(self) -> bool:
        return self.fails >= self.down_after

    def sample(self, ok: bool, mode: str | None = None) -> str | None:
        """→ 'down' / 'up' on a change, else None."""
        was = self.down
        if ok:
            self.fails, self.answered = 0, True
            if mode:
                self.mode = mode
            return "up" if was else None
        self.fails += 1
        return "down" if self.down and not was else None


def self_restart_allowed(env_value: str | None, now: float, limit: int = MAX_SELF_RESTARTS,
                         window: float = SELF_RESTART_WINDOW_S) -> tuple[bool, str]:
    """``WIN11SHELL_APPWINDOW_RESTARTS`` = space separated epoch times of earlier self-restarts →
    (allowed, new value). Pure (tested)."""
    times = []
    for x in (env_value or "").split():
        try:
            t = float(x)
        except ValueError:
            continue
        if now - t < window:
            times.append(t)
    if len(times) >= limit:
        return False, " ".join(f"{t:.0f}" for t in times)
    return True, " ".join(f"{t:.0f}" for t in times + [now])


class AppWindow:
    def __init__(self, mods: dict, app: str, port: int, token: str, params: dict, dev: bool, dry_run: bool):
        from .surfaces import _make_rgba_window, create_webview

        self.m = mods
        Gtk, Gdk, GLib = mods["Gtk"], mods["Gdk"], mods["GLib"]
        self.app = app
        self.port = port
        self.token = token
        self.dev = dev
        self.lang = _language()
        self.title = TITLES[self.lang][app]
        self.compositor = _compositor()
        self.titlebar_h = 0
        self.no_drag: list[tuple[int, int, int, int]] = []
        self.maximized = self.fullscreen = self.tiled = False
        self.focused = True
        self._press = None        # titlebar press we swallowed (waiting for drag threshold)
        self._last_press = None   # any recent press (for dragStart from JS)
        self._button_down = False
        self._hdrag = None        # Hyprland: title-bar drag we perform ourselves (see _start_move)
        self._cursor_name = None
        self._shown = False
        self._closing = False
        self._close_pending = False          # delete-event seen: 'win11:close' sent, destroy follows
        # Hyprland: is the window snapped by the shell (windows.py resizes a floating window: no GTK state says so)?
        # Asked after each size change; a snapped window keeps its last normal size as the remembered one (Win11).
        self._snapped = False
        self._normal_size: tuple[int, int] | None = None
        self._size_seen: tuple[int, int] | None = None
        self._snap_check = None
        self.state = WindowState(app)
        self.on_top = False                  # Taakbeheer: settings.taskmgr.alwaysOnTop
        self._pin_lock = threading.Lock()
        self.params = dict(params or {})
        self.dry_run = dry_run
        self.health = HostHealth()
        self.page_ok = False                 # our page finished loading (since the last (re)load)
        self._load_failed = False
        self.notice_kind: str | None = None
        self._restarting_since = 0.0         # we ran win11shell-ctl restart --force
        self._restart_by_notice = False
        self._host_pid: int | None = None    # from shell.info
        self._restart_old_pid: int | None = None
        self._hang_stage = "ok"
        self._no_shell_dismissed = False
        settings = _settings_file()
        self.dark = _dark(settings)
        self._win11_session = "win11shell" in (os.environ.get("XDG_CURRENT_DESKTOP", "") +
                                               os.environ.get("XDG_SESSION_DESKTOP", "")).lower()

        win = Gtk.Window(type=Gtk.WindowType.TOPLEVEL)
        self.win = win
        win.set_title(self.title)
        win.set_icon_name(ICONS[app])
        win.set_decorated(False)
        _make_rgba_window(Gtk, win)
        geom = Gdk.Geometry()
        geom.min_width, geom.min_height = MIN_SIZE
        win.set_geometry_hints(None, geom, Gdk.WindowHints.MIN_SIZE)
        w, h = self._initial_size()
        win.set_default_size(w, h)
        win.set_position(Gtk.WindowPosition.CENTER)
        if self.state.load().get("maximized"):
            win.maximize()

        origin = f"http://127.0.0.1:{port}"
        self.view = create_webview(origin=origin, transparent=True, dev=dev, dry_run=dry_run)
        if app == "taskmgr":
            _no_gpu_settings(mods, self.view)
        else:   # WIN11SHELL_GPU=safe: give the GPU up (no-gpu) and restart ourselves on crashes or the fd leak
            from .surfaces import set_gpu_fallback, start_gpu_leak_guard
            set_gpu_fallback(self._gpu_fallback)
            start_gpu_leak_guard()
        ucm = self.view.get_user_content_manager()
        ucm.register_script_message_handler("win11")
        ucm.connect("script-message-received::win11", self._on_message)
        self.view.connect("notify::title", self._on_title)
        self.view.connect("button-press-event", self._on_press)
        self.view.connect("button-release-event", self._on_release)
        self.view.connect("motion-notify-event", self._on_motion)
        self.view.connect("leave-notify-event", lambda *_a: (self._set_cursor(None), False)[1])
        self.view.connect("load-changed", self._on_load_changed)
        self.view.connect("load-failed", self._on_load_failed)
        self.view.connect("web-process-terminated", self._on_web_terminated)
        win.connect("window-state-event", self._on_state)
        win.connect("configure-event", self._on_configure)
        win.connect("key-press-event", self._on_key)
        win.connect("delete-event", self._on_delete)
        win.connect("destroy", self._on_destroy)
        self.overlay = Gtk.Overlay()
        self.overlay.add(self.view)
        self._build_notice()
        win.add(self.overlay)

        q = {"surface": "app", "app": app}
        for k, v in (params or {}).items():
            if k in ("surface", "app", "token"):
                continue
            q[str(k)] = v if isinstance(v, str) else json.dumps(v, ensure_ascii=False)
        q["token"] = token
        self.url = f"{origin}/ui/index.html?{urlencode(q)}"
        self.view.load_uri(self.url)
        self.view.show()
        self.overlay.show()
        # Show once the page has painted something (no empty transparent flash); fallback timer.
        GLib.timeout_add(1500, self._show)
        threading.Thread(target=self._events_loop, name="sse", daemon=True).start()
        from .lifeguard import system_thrashing
        self.watchdog = HangWatchdog(self._probe_page, self._reload_hung, self._restart_hung, self._may_restart,
                                     self._on_hang_stage, host_ok=lambda: not self.health.down,
                                     first_s=FIRST_ANSWER_S, busy=system_thrashing)
        GLib.timeout_add_seconds(WATCHDOG_TICK_S, self._watchdog_tick)
        threading.Thread(target=self._health_loop, name="host-health", daemon=True).start()

    # -- geometry ---------------------------------------------------------------
    def _initial_size(self) -> tuple[int, int]:
        Gdk = self.m["Gdk"]
        st = self.state.load()
        w, h = DEFAULT_SIZES[self.app]
        if isinstance(st.get("width"), int) and isinstance(st.get("height"), int):
            w, h = st["width"], st["height"]
        display = Gdk.Display.get_default()
        mon = display.get_primary_monitor() if display else None
        if mon is None and display is not None and display.get_n_monitors():
            mon = display.get_monitor(0)
        wa = mon.get_workarea() if mon is not None else None
        if wa is not None and wa.width > 200 and wa.height > 200:
            w = min(w, int(wa.width * 0.94))
            h = min(h, int(wa.height * 0.92))
        return max(MIN_SIZE[0], w), max(MIN_SIZE[1], h)

    def _show(self):
        if not self._shown and not self._closing:
            self._shown = True
            self.win.show_all()
            self.win.present()
            self._push_state()
            if self.app == "taskmgr":
                threading.Thread(target=self._init_on_top, name="on-top", daemon=True).start()
        return False

    # -- always on top (Taakbeheer) --------------------------------------------------
    def _init_on_top(self):
        try:
            settings = self._api("settings.get", {})
        except Exception:  # noqa: BLE001 - host busy: the settings file is good enough
            settings = _settings_file()
        self.set_on_top(_always_on_top(settings), force=True)

    def set_on_top(self, value: bool, force: bool = False):
        """Thread-safe. Hyprland: pin via the host (retries until the window is mapped)."""
        value = bool(value)
        if value == self.on_top and not force:
            return
        self.on_top = value
        log.info("Altijd op voorgrond (always on top): %s", value)
        self.m["GLib"].idle_add(lambda: (self.win.set_keep_above(value), False)[1])
        if self.compositor == "hyprland":
            threading.Thread(target=self._pin_worker, name="pin", daemon=True).start()

    def _pin_worker(self):
        from .util import ApiError
        with self._pin_lock:
            for _attempt in range(10):
                if self._closing:
                    return
                want = self.on_top
                try:
                    self._api("windows.setPinned", {"pid": os.getpid(), "pinned": want})
                except ApiError as e:
                    if e.code not in ("not_found", "not_running"):
                        log.info("Vastzetten mislukt (pin failed): %s", e.message)
                        return
                    time.sleep(0.3)     # window not mapped yet / host busy
                    continue
                if want == self.on_top:
                    return

    def _on_load_changed(self, _view, event):
        WebKit2 = self.m["WebKit2"]
        if event == WebKit2.LoadEvent.STARTED:
            self.page_ok = self._load_failed = False
        if event == WebKit2.LoadEvent.COMMITTED:
            wd = getattr(self, "watchdog", None)
            if wd is not None:
                wd.page_committed()          # a probe sent to the old page can only time out
        if event == WebKit2.LoadEvent.FINISHED:
            uri = self.view.get_uri() or ""
            self.page_ok = not self._load_failed and uri.startswith(f"http://127.0.0.1:{self.port}/ui/")
            self.m["GLib"].timeout_add(60, self._show)
            self.m["GLib"].timeout_add(80, lambda: (self._push_state(), self._push_host_state(), False)[2])
            if self.page_ok and self._hang_stage in ("hung", "reopen") and self.watchdog.stage == "ok":
                self._hang_stage = "ok"
            self._update_notice()

    def _on_load_failed(self, _view, _event, _uri, _error):
        self.page_ok, self._load_failed = False, True
        self._update_notice()
        return False

    def _on_web_terminated(self, _view, _reason):
        self.page_ok = False
        return False

    # -- never stuck: hang watchdog ----------------------------------------------------
    def _watchdog_tick(self):
        if self._closing:
            return False
        self.watchdog.tick()
        return True

    def _probe_page(self, done) -> bool:
        """→ False when nothing was asked (HangWatchdog asks again on its next tick)."""
        from .surfaces import view_dead
        if view_dead(self.view):
            return False            # no web process (WebKit would fail the call at once): no answer from a page
        def finish(view, result, *_a):
            try:
                (view.evaluate_javascript_finish if hasattr(view, "evaluate_javascript_finish")
                 else view.run_javascript_finish)(result)
            except Exception:  # noqa: BLE001 - an error answer is an answer
                pass
            done()
        if hasattr(self.view, "evaluate_javascript"):
            self.view.evaluate_javascript("1", -1, None, None, None, finish, None)
        else:
            self.view.run_javascript("1", None, finish, None)
        return True

    def _reload_hung(self):
        """Watchdog verdict 'hung'. While we force the GPU (WIN11SHELL_GPU=safe) the second verdict within 10 min
        gives it up (surfaces.note_page_hang: a stopped web process is no crash, so _note_crash never sees it) and
        the window restarts without it — a reload would keep the GPU path this process chose at its start."""
        from .surfaces import note_page_hang
        if note_page_hang() and self._gpu_fallback():
            return
        self._reload_page()

    def _restart_hung(self):
        """Watchdog verdict 'reopen' (hung again within a minute of the reload)."""
        from .surfaces import note_page_hang
        gpu_off = note_page_hang()
        self._restart_self(why="vastgelopen, zonder GPU" if gpu_off else "vastgelopen", gpu_off=gpu_off)

    def _reload_page(self):
        log.warning("Pagina reageert niet; webproces gestopt en opnieuw laden (page hung: killing the web "
                    "process, reloading)")
        self.page_ok = False
        from .surfaces import view_dead
        try:
            if view_dead(self.view):
                self.view.reload()                   # no web process left (a crash loop's back-off): load now
            else:
                self.view.terminate_web_process()    # → web-process-terminated → reload (create_webview)
        except Exception:  # noqa: BLE001
            self.view.reload()

        def ensure():
            if not self._closing and not self.view.is_loading() and not self.page_ok:
                self.view.load_uri(self.url)
            return False
        self.m["GLib"].timeout_add(1500, ensure)

    def _may_restart(self) -> bool:
        ok, value = self_restart_allowed(os.environ.get(RESTARTS_ENV), time.time())
        if ok:
            os.environ[RESTARTS_ENV] = value
        return ok

    def _gpu_fallback(self) -> bool:
        """The GPU renderer was given up (surfaces._give_up_gpu): restart without it — within the self-restart
        budget, so a GPU that keeps failing can never become a restart loop. → False when no restart is left (else
        the process is exec'd and this never returns)."""
        if not self._may_restart():
            log.error("GPU opgegeven, maar geen herstart meer over; dit venster blijft zoals het is "
                      "(GPU given up, no restart budget left)")
            return False
        self._restart_self(why="zonder GPU", gpu_off=True)
        return True

    def _restart_self(self, port: int | None = None, token: str | None = None, why: str = "vastgelopen",
                      gpu_off: bool = False):
        """exec ourselves again (same pid, new GTK/WebKit), optionally onto another host; ``gpu_off``: render
        without the GPU from now on (also when no-gpu could not be written)."""
        port, token = port or self.port, token or self.token
        log.warning("App-venster start zichzelf opnieuw (%s) (app window restarts itself)", why)
        argv = [sys.executable, "-m", "host.appwindow", "--app", self.app, "--port", str(port)]
        if self.params:
            argv += ["--params", json.dumps(self.params, ensure_ascii=False)]
        if self.dev:
            argv.append("--dev")
        if self.dry_run:
            argv.append("--dry-run")
        env = dict(_ORIG_ENV or os.environ)
        env["WIN11SHELL_TOKEN"] = token
        if gpu_off or os.environ.get("WIN11SHELL_GPU") == "0":
            env["WIN11SHELL_GPU"] = "0"
        if os.environ.get(RESTARTS_ENV):
            env[RESTARTS_ENV] = os.environ[RESTARTS_ENV]
        from . import ROOT
        env["PYTHONPATH"] = str(ROOT) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        self._remember()
        try:
            self.view.terminate_web_process()        # a busy web process may not notice our exec
        except Exception:  # noqa: BLE001
            pass
        logging.shutdown()
        os.chdir(str(ROOT))
        os.execve(sys.executable, argv, env)

    def dev_probe(self):
        """Test aid (--dev, SIGRTMIN+5; see the module doc): a private script's answer, posted back by the page as
        ``{cmd:'devAnswer'}`` (the same path as every page message)."""
        from .util import runtime_dir
        src = runtime_dir() / f"devjs-{self.app}.js"
        script = "JSON.stringify({title: document.title, ready: document.readyState})"
        try:
            st = os.stat(src)
            if st.st_uid == os.getuid() and not st.st_mode & 0o077:
                script = src.read_text(encoding="utf-8")
        except OSError:
            pass
        from .surfaces import run_js
        run_js(self.view, "(function(){var a;try{a=String(eval(%s));}catch(e){a=JSON.stringify({error:String(e)});}"
                          "window.webkit.messageHandlers.win11.postMessage({cmd:'devAnswer',text:a});})();"
               % json.dumps(script))

    def _dev_answer(self, text) -> None:
        out = _cache_dir()
        with contextlib.suppress(OSError):
            out.mkdir(parents=True, exist_ok=True)
            (out / f"devjs-{self.app}.json").write_text(str(text)[:1_000_000], encoding="utf-8")

    def _on_hang_stage(self, stage: str):
        self._hang_stage = stage
        if stage == "stuck":                 # a verdict too (no restart budget: the notice offers a reload)
            from .surfaces import note_page_hang
            note_page_hang()                 # gives the GPU up for the next start of this window
        self._update_notice()

    # -- never stuck: the host's health ---------------------------------------------
    def _health_loop(self):
        from .lifeguard import LOOP_STUCK_MS
        from .util import ApiError, call_instance, read_instance
        GLib = self.m["GLib"]
        stuck_logged = False
        while not self._closing:
            ok, mode, pid = False, None, None
            try:
                info = call_instance("shell.info", {}, inst={"port": self.port, "token": self.token}, timeout=2.5)
                if isinstance(info, dict):
                    ok, mode, pid = True, info.get("mode"), info.get("pid")
                    lag = info.get("mainLoopLagMs")
                    if isinstance(lag, (int, float)) and not isinstance(lag, bool) and lag > LOOP_STUCK_MS:
                        ok = False          # it answers, but its main loop (the taskbar) stands still
                        if not stuck_logged:
                            log.warning("Win11 Shell antwoordt, maar zijn hoofdlus staat %.0f s stil (host main "
                                        "loop stuck)", lag / 1000)
                        stuck_logged = True
                    else:
                        stuck_logged = False
                else:
                    ok = True
            except ApiError:
                ok = False
            except Exception:  # noqa: BLE001
                ok = False
            GLib.idle_add(self._on_health, ok, mode, pid)
            if not ok and self.health.fails + 1 >= HOST_DOWN_AFTER:
                # a new host elsewhere (it was restarted without our port/token): follow it
                inst = read_instance()
                if inst and inst.get("mode") in ("layer", "preview", "app") and \
                        (int(inst.get("port") or 0), inst.get("token")) != (self.port, self.token):
                    try:
                        call_instance("shell.info", {}, inst=inst, timeout=2.0)
                        GLib.idle_add(self._restart_self, int(inst["port"]), str(inst["token"]), "nieuwe host")
                        return
                    except ApiError:
                        pass
            time.sleep(HEALTH_OK_S if ok else HEALTH_FAIL_S)

    def _on_health(self, ok: bool, mode: str | None, pid: int | None = None):
        if ok and self._restarting_since and pid is not None and pid == self._restart_old_pid and \
                time.monotonic() - self._restarting_since < 30:
            ok = False          # the old (stuck) host answering while it is being stopped: not back yet
        if ok and isinstance(pid, int):
            self._host_pid = pid
        change = self.health.sample(ok, mode)
        if change == "down":
            log.warning("Win11 Shell reageert niet (host not answering) op poort %d", self.port)
        if ok and (change == "up" or self._restarting_since):
            if change == "up":
                log.info("Win11 Shell antwoordt weer (host answers again)")
            self._restarting_since = 0.0
            if not self.page_ok and not self.view.is_loading():
                self.view.load_uri(self.url)          # it never loaded while the host was away
        if change or self._restarting_since:
            self._push_host_state()
        self._update_notice()
        return False

    def _push_host_state(self):
        d = json.dumps({"ok": not self.health.down, "mode": self.health.mode,
                        "restarting": bool(self._restarting_since)})
        from .surfaces import run_js
        run_js(self.view, "(function(){var d=%s;window.__win11HostState=d;"
                          "window.dispatchEvent(new CustomEvent('win11:hoststate',{detail:d}));})();" % d)

    def restart_shell(self, by_notice: bool = False):
        """Restart Win11 Shell without the host: ``win11shell-ctl restart --force`` as our own child."""
        now = time.monotonic()
        if self._restarting_since and now - self._restarting_since < 8:
            return
        self._restarting_since, self._restart_by_notice = now, by_notice
        self._restart_old_pid = self._host_pid
        from . import ROOT
        env = dict(_ORIG_ENV or os.environ)
        env.pop("WIN11SHELL_TOKEN", None)
        env.pop(RESTARTS_ENV, None)
        env["PYTHONPATH"] = str(ROOT) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
        log_path = _cache_dir() / "restart.log"
        out = subprocess.DEVNULL
        with contextlib.suppress(OSError):
            log_path.parent.mkdir(parents=True, exist_ok=True)
            if log_path.exists() and log_path.stat().st_size > 512_000:
                log_path.unlink()
            out = open(log_path, "ab")
        try:
            p = subprocess.Popen([sys.executable, "-m", "host.ctl", "restart", "--force"], cwd=str(ROOT), env=env,
                                 stdin=subprocess.DEVNULL, stdout=out, stderr=out, start_new_session=True,
                                 close_fds=True)
            log.warning("Win11 Shell wordt opnieuw gestart door %s (restarting the shell), ctl pid %d",
                        self.app, p.pid)
            threading.Thread(target=p.wait, name="ctl-wait", daemon=True).start()
        except OSError as e:
            log.error("win11shell-ctl niet te starten (cannot run ctl): %s", e)
            self._restarting_since = 0.0
        finally:
            if out is not subprocess.DEVNULL:
                out.close()
        self._push_host_state()
        self._update_notice()

    # -- native notice (GTK, over the page: works when the page is hung or never loaded) -----------
    def _build_notice(self):
        Gtk, Gdk = self.m["Gtk"], self.m["Gdk"]
        provider = Gtk.CssProvider()
        try:
            provider.load_from_data(_CSS)
            Gtk.StyleContext.add_provider_for_screen(Gdk.Screen.get_default(), provider,
                                                     Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
        except Exception:  # noqa: BLE001
            pass
        self._n_events = Gtk.EventBox()           # takes the clicks: never into a hung page
        self._n_events.set_visible_window(False)
        self._n_scrim = Gtk.Box()
        self._n_scrim.get_style_context().add_class("w11-scrim")
        card = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=10)
        card.get_style_context().add_class("w11-card")
        card.set_halign(Gtk.Align.CENTER)
        card.set_valign(Gtk.Align.CENTER)
        card.set_size_request(420, -1)
        self._n_title = Gtk.Label(xalign=0)
        self._n_title.get_style_context().add_class("w11-title")
        self._n_msg = Gtk.Label(xalign=0)
        self._n_msg.set_line_wrap(True)
        self._n_msg.set_max_width_chars(52)
        self._n_msg.get_style_context().add_class("w11-msg")
        row = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=8)
        row.set_halign(Gtk.Align.END)
        row.set_margin_top(10)
        self._n_primary = Gtk.Button()
        self._n_primary.get_style_context().add_class("w11-accent")
        self._n_secondary = Gtk.Button()
        self._n_primary.connect("clicked", lambda *_a: self._notice_action(True))
        self._n_secondary.connect("clicked", lambda *_a: self._notice_action(False))
        row.pack_start(self._n_secondary, False, False, 0)
        row.pack_start(self._n_primary, False, False, 0)
        for w in (self._n_title, self._n_msg, row):
            card.pack_start(w, False, False, 0)
        self._n_card = card
        self._n_scrim.set_center_widget(card) if hasattr(self._n_scrim, "set_center_widget") else \
            self._n_scrim.pack_start(card, True, False, 0)
        self._n_events.add(self._n_scrim)
        self._n_events.set_no_show_all(True)
        self.overlay.add_overlay(self._n_events)
        # slim bar (Taakbeheer without a running desktop shell): does not cover the page
        self._n_bar = Gtk.Box(orientation=Gtk.Orientation.HORIZONTAL, spacing=10)
        self._n_bar.get_style_context().add_class("w11-bar")
        self._n_bar.set_valign(Gtk.Align.END)
        self._n_bar_label = Gtk.Label(xalign=0)
        self._n_bar_label.set_line_wrap(True)
        self._n_bar_btn = Gtk.Button()
        self._n_bar_btn.connect("clicked", lambda *_a: self.restart_shell(by_notice=True))
        bar_close = Gtk.Button(label="✕")
        bar_close.set_relief(Gtk.ReliefStyle.NONE)
        bar_close.connect("clicked", lambda *_a: self._dismiss_bar())
        self._n_bar.pack_start(self._n_bar_label, True, True, 0)
        self._n_bar.pack_start(self._n_bar_btn, False, False, 0)
        self._n_bar.pack_start(bar_close, False, False, 0)
        self._n_bar.set_no_show_all(True)
        self.overlay.add_overlay(self._n_bar)
        for w in (self._n_card, self._n_bar):
            if self.dark:
                w.get_style_context().add_class("w11-dark")

    def _dismiss_bar(self):
        self._no_shell_dismissed = True
        self._update_notice()

    def _wanted_notice(self) -> str | None:
        if self._hang_stage in ("hung", "reopen", "stuck"):
            return self._hang_stage
        if self._restarting_since and time.monotonic() - self._restarting_since < 45 and \
                (self._restart_by_notice or not self.page_ok):
            return "restarting"
        if self.health.down:
            # Taakbeheer's own page shows this itself while it is loaded and answers (unanswered_since is sticky:
            # the watchdog starts a new probe every hang_s while the host is down, the card must not blink)
            wd = self.watchdog
            if self.app != "taskmgr" or not self.page_ok or wd.unanswered_since is not None or \
                    wd.pending is not None and time.monotonic() - wd.pending > 4:
                return "host_down"
            return None
        if self.app == "taskmgr" and self.health.mode == "app" and self._win11_session and \
                not self._no_shell_dismissed and not self._restarting_since:
            return "no_shell"
        return None

    def _update_notice(self):
        kind = self._wanted_notice()
        if kind == self.notice_kind:
            return
        log.info("Melding in het venster (notice): %s → %s", self.notice_kind, kind)
        self.notice_kind = kind
        t = NOTICE[self.lang]
        name = self.title
        if kind is None:
            self._n_events.hide()
            self._n_bar.hide()
            return
        if kind == "no_shell":
            self._n_events.hide()
            self._n_bar_label.set_text(f"{t['no_shell_t']}. {t['no_shell_m']}")
            self._n_bar_btn.set_label(t["start_shell"])
            self._n_bar.show_all()
            return
        self._n_bar.hide()
        self._n_title.set_text(t[f"{kind}_t"].format(app=name))
        self._n_msg.set_text(t[f"{kind}_m"].format(app=name))
        buttons = {"host_down": ("restart", "close"), "stuck": ("reload", "close"),
                   "restarting": (None, "close"), "hung": (None, None), "reopen": (None, None)}[kind]
        for btn, key in ((self._n_primary, buttons[0]), (self._n_secondary, buttons[1])):
            if key:
                btn.set_label(t[key])
                btn.show()
            else:
                btn.hide()
        self._notice_buttons = buttons
        ctx = self._n_scrim.get_style_context()
        (ctx.remove_class if self.page_ok else ctx.add_class)("w11-solid")
        if self.dark:
            ctx.add_class("w11-dark")
        self._n_events.show()
        self._n_scrim.show()
        self._n_card.show_all()
        for btn, key in ((self._n_primary, buttons[0]), (self._n_secondary, buttons[1])):
            btn.set_visible(bool(key))
        if not self._shown:
            self._show()

    def _notice_action(self, primary: bool):
        key = (getattr(self, "_notice_buttons", (None, None)))[0 if primary else 1]
        if key == "restart":
            self.restart_shell(by_notice=True)
        elif key == "reload":
            self._hang_stage = "ok"
            self.watchdog.stage = "ok"
            self.watchdog.last_reload = time.monotonic()
            self._reload_page()
            self._update_notice()
        elif key == "close":
            self.close()

    # -- focus requests from a second process of this app (RAISE_SIGNAL) -------------------------
    def focus_self(self):
        def work():
            done = False
            if self.compositor == "hyprland":
                from .ctl import focus_hypr_window
                from .util import hyprctl
                for c in hyprctl("clients", json_out=True, timeout=1.0) or []:
                    if isinstance(c, dict) and c.get("pid") == os.getpid():
                        done = focus_hypr_window(c)
                        break
            if not done:
                self.m["GLib"].idle_add(self.present)
        threading.Thread(target=work, name="focus-self", daemon=True).start()
        return False

    # -- page messages ------------------------------------------------------------
    def _on_message(self, _ucm, js_result):
        data = None
        try:
            value = js_result.get_js_value() if hasattr(js_result, "get_js_value") else js_result
            data = json.loads(value.to_json(0))
        except Exception:  # noqa: BLE001
            try:
                data = {"cmd": value.to_string()}
            except Exception:  # noqa: BLE001
                return
        if isinstance(data, str):
            data = {"cmd": data}
        if not isinstance(data, dict):
            return
        cmd = data.get("cmd")
        log.debug("Bericht van pagina (page message): %s", data)
        if cmd == "minimize":
            self.minimize()
        elif cmd == "toggleMaximize":
            self.toggle_maximize()
        elif cmd == "maximize":
            if not self.maximized:
                self.toggle_maximize()
        elif cmd == "restore":
            if self.maximized:
                self.toggle_maximize()
        elif cmd == "close":
            self.close()
        elif cmd == "dragStart":
            self._drag_from_last_press()
        elif cmd == "titlebar":
            try:
                self.titlebar_h = max(0, min(200, int(float(data.get("height") or 0))))
                rects = []
                for r in (data.get("noDragRects") or [])[:64]:
                    rects.append(tuple(int(float(r.get(k, 0))) for k in ("x", "y", "w", "h")))
                self.no_drag = rects
            except (TypeError, ValueError, AttributeError):
                pass
        elif cmd == "setTitle":
            t = str(data.get("title") or "").strip()
            if t:
                self.win.set_title(t[:200])
        elif cmd == "restartShell":
            self.restart_shell()
        elif cmd == "devAnswer" and (self.dev or os.environ.get("WIN11SHELL_DEBUG_HANG") == "1"):
            self._dev_answer(data.get("text"))

    def _on_title(self, view, _pspec):
        t = (view.get_title() or "").strip()
        self.win.set_title(t[:200] if t else self.title)

    # -- window actions -----------------------------------------------------------
    def minimize(self):
        if self.compositor == "hyprland":
            def via_api():
                try:
                    self._api("windows.minimize", {"pid": os.getpid()})
                except Exception:  # noqa: BLE001
                    self.m["GLib"].idle_add(lambda: (self.win.iconify(), False)[1])
            threading.Thread(target=via_api, daemon=True).start()
        else:
            self.win.iconify()

    def toggle_maximize(self):
        if self.compositor == "hyprland":
            def via_api():
                try:
                    self._api("windows.toggleMaximize", {"pid": os.getpid()})
                except Exception:  # noqa: BLE001
                    self.m["GLib"].idle_add(lambda: (self._gtk_toggle_maximize(), False)[1])
            threading.Thread(target=via_api, daemon=True).start()
        else:
            self._gtk_toggle_maximize()

    def _gtk_toggle_maximize(self):
        if self.maximized:
            self.win.unmaximize()
        else:
            self.win.maximize()

    def close(self):
        self.win.close()  # → delete-event → destroy

    def present(self, token: str | None = None):
        if token:
            try:
                self.win.set_startup_id(token)
            except Exception:  # noqa: BLE001
                pass
        self.win.deiconify()
        self.win.present()
        return False

    def _api(self, method: str, args: dict):
        from .util import call_instance
        return call_instance(method, args, inst={"port": self.port, "token": self.token}, timeout=3)

    # -- state ----------------------------------------------------------------
    def _on_state(self, _win, event):
        Gdk = self.m["Gdk"]
        st = event.new_window_state
        was_max = self.maximized or self.fullscreen
        self.maximized = bool(st & Gdk.WindowState.MAXIMIZED)
        self.fullscreen = bool(st & Gdk.WindowState.FULLSCREEN)
        self.focused = bool(st & Gdk.WindowState.FOCUSED)
        tiled = getattr(Gdk.WindowState, "TILED", 0)
        # Hyprland (0.53) puts every xdg toplevel in TILED_LEFT/RIGHT/TOP/BOTTOM from its first configure on
        # (XDGShell.cpp:160-165) and never takes it away: there it says nothing about snapping, so it is ignored
        # (plugin win11-csd 1.6.2, notes/plugin-1.6.2-design.md H1). Maximized / fullscreen still square the corners.
        self.tiled = bool(st & tiled) if tiled and self.compositor != "hyprland" else False
        self._push_state()
        if was_max and not (self.maximized or self.fullscreen) and self.on_top and self.compositor == "hyprland":
            # Hyprland cannot pin a maximized window: pin again once it is restored (no-op if already pinned)
            threading.Thread(target=self._pin_worker, name="pin", daemon=True).start()
        return False

    def _push_state(self):
        d = json.dumps({"maximized": self.maximized, "focused": self.focused, "fullscreen": self.fullscreen,
                        "tiled": self.tiled})
        js = ("(function(){var d=%s;var h=document.documentElement;if(h){h.dataset.windowMaximized=d.maximized?'1':'0';"
              "h.dataset.windowFocused=d.focused?'1':'0';}window.__win11WindowState=d;"
              "window.dispatchEvent(new CustomEvent('win11:windowstate',{detail:d}));})();") % d
        from .surfaces import run_js
        run_js(self.view, js)

    def _on_configure(self, _win, _event):
        """Hyprland: after a size change (debounced) ask the host whether the shell snapped us (windows.list
        ``snapped``), off the main thread; see _remember."""
        if self.compositor != "hyprland" or self._closing or self._close_pending:
            return False
        try:
            size = tuple(int(x) for x in self.win.get_size())
        except Exception:  # noqa: BLE001
            return False
        if size == self._size_seen:
            return False
        self._size_seen = size
        GLib = self.m["GLib"]
        if self._snap_check is not None:
            GLib.source_remove(self._snap_check)
        self._snap_check = GLib.timeout_add(SNAP_CHECK_MS, self._check_snapped)
        return False

    def _check_snapped(self):
        self._snap_check = None
        if not self._closing:
            size = tuple(int(x) for x in self.win.get_size())
            threading.Thread(target=self._snapped_worker, args=(size,), name="snapped", daemon=True).start()
        return False

    def _snapped_worker(self, size: tuple[int, int]):
        try:
            res = self._api("windows.list", {})      # {supported, windows: [...]}
        except Exception:  # noqa: BLE001 - host busy: keep what we knew
            return
        wins = res.get("windows") if isinstance(res, dict) else None
        if not isinstance(wins, list):
            return
        me = os.getpid()
        snapped = any(isinstance(w, dict) and w.get("pid") == me and w.get("snapped") for w in wins)
        self.m["GLib"].idle_add(self._set_snapped, snapped, size)

    def _set_snapped(self, snapped: bool, size: tuple[int, int]):
        self._snapped = bool(snapped)
        if not snapped and not (self.maximized or self.fullscreen or self.tiled) and \
                tuple(int(x) for x in self.win.get_size()) == tuple(size):
            self._normal_size = tuple(size)
        return False

    def _remember(self):
        """Save size + maximized. Maximized / fullscreen / tiled (not on Hyprland) / snapped by the shell: the last
        normal size stays the remembered one — Windows 11 also reopens a window that was snapped at its pre-snap size
        (``_normal_size``: the last size windows.list called unsnapped, Hyprland only; else the stored one)."""
        try:
            w, h = self.win.get_size()
            prev = self.state.load()
            st = {"maximized": self.maximized}
            normal = getattr(self, "_normal_size", None)
            if self.maximized or self.fullscreen or self.tiled or getattr(self, "_snapped", False):
                if normal:
                    st["width"], st["height"] = int(normal[0]), int(normal[1])
                elif "width" in prev:
                    st["width"], st["height"] = prev.get("width"), prev.get("height")
            else:
                st["width"], st["height"] = int(w), int(h)
            self.state.save(st)
        except Exception:  # noqa: BLE001
            pass

    def _on_delete(self, *_a):
        """The window is being closed: remember its size, tell the page (``win11:close``: the Bluetooth dialog
        sends its keepalive clean-up requests, SPEC §6.10) and destroy it CLOSE_GRACE_MS later. A second close
        in that time changes nothing; a hung page does not hold it up (fire-and-forget)."""
        if self._close_pending or self._closing:
            return True
        self._close_pending = True
        self._remember()
        from .surfaces import run_js
        run_js(self.view, "window.dispatchEvent(new CustomEvent('win11:close'));")
        self.m["GLib"].timeout_add(CLOSE_GRACE_MS, self._close_now)
        return True                          # keep the window for now

    def _close_now(self):
        if not self._closing:
            self.win.destroy()               # → destroy → main_quit
        return False

    def close_from_signal(self):
        """SIGTERM/SIGINT/SIGHUP (ctl, the session ending): the app lock is given back at once (exit_soon: the
        process ends within EXIT_GRACE_S whatever happens), then the same ``win11:close`` + CLOSE_GRACE_MS as a
        close by hand — without it the Bluetooth dialog's search kept running (nested check 2026-10-27)."""
        self._closing = True
        exit_soon()
        self._remember()
        from .surfaces import run_js
        run_js(self.view, "window.dispatchEvent(new CustomEvent('win11:close'));")

        def destroy():
            try:
                self.win.destroy()           # inside the loop → main_quit via the destroy handler
            except Exception:  # noqa: BLE001
                self.m["Gtk"].main_quit()
            return False
        self.m["GLib"].timeout_add(CLOSE_GRACE_MS, destroy)

    def _on_destroy(self, *_a):
        log.info("Venster gesloten (window destroyed)")
        self._closing = True
        exit_soon()
        self.m["Gtk"].main_quit()

    def _on_key(self, _win, event):
        Gdk = self.m["Gdk"]
        if self.dev and event.keyval == Gdk.KEY_F12:
            insp = self.view.get_inspector()
            if insp is not None:
                insp.show()
            return True
        return False

    # -- pointer: drag, resize ----------------------------------------------------
    def _edge_at(self, x: float, y: float):
        # Hyprland resizes from its own border grab area (resize_on_border); in-window edge
        # presses would only become a corner resize there, so leave them to the page
        if self.maximized or self.fullscreen or self.compositor == "hyprland":
            return None
        Gdk = self.m["Gdk"]
        alloc = self.view.get_allocation()
        w, h = alloc.width, alloc.height
        left, right = x < EDGE, x >= w - EDGE
        top, bottom = y < EDGE, y >= h - EDGE
        cl, cr = x < CORNER, x >= w - CORNER
        ct, cb = y < CORNER, y >= h - CORNER
        E = Gdk.WindowEdge
        if (top and cl) or (left and ct):
            return E.NORTH_WEST
        if (top and cr) or (right and ct):
            return E.NORTH_EAST
        if (bottom and cl) or (left and cb):
            return E.SOUTH_WEST
        if (bottom and cr) or (right and cb):
            return E.SOUTH_EAST
        if top:
            return E.NORTH
        if bottom:
            return E.SOUTH
        if left:
            return E.WEST
        if right:
            return E.EAST
        return None

    _CURSORS = {"NORTH_WEST": "nw-resize", "NORTH_EAST": "ne-resize", "SOUTH_WEST": "sw-resize",
                "SOUTH_EAST": "se-resize", "NORTH": "n-resize", "SOUTH": "s-resize", "WEST": "w-resize",
                "EAST": "e-resize"}

    def _set_cursor(self, edge):
        Gdk = self.m["Gdk"]
        name = self._CURSORS.get(edge.value_nick.upper().replace("-", "_")) if edge is not None else None
        if name == self._cursor_name:
            return
        gw = self.view.get_window()
        if gw is None:
            return
        if name is None:
            if self._cursor_name is not None:
                gw.set_cursor(Gdk.Cursor.new_from_name(gw.get_display(), "default"))
            self._cursor_name = None
            return
        gw.set_cursor(Gdk.Cursor.new_from_name(gw.get_display(), name))
        self._cursor_name = name

    def _in_titlebar(self, x: float, y: float) -> bool:
        if y >= self.titlebar_h:
            return False
        for (rx, ry, rw, rh) in self.no_drag:
            if rx <= x < rx + rw and ry <= y < ry + rh:
                return False
        return True

    def _on_press(self, _view, event):
        Gdk = self.m["Gdk"]
        double = getattr(Gdk.EventType, "DOUBLE_BUTTON_PRESS", None) or getattr(Gdk.EventType, "_2BUTTON_PRESS")
        self._button_down = True
        self._last_press = (event.button, event.x_root, event.y_root, event.time, time.monotonic())
        edge = self._edge_at(event.x, event.y)
        if edge is not None and event.button == 1 and event.type == Gdk.EventType.BUTTON_PRESS:
            self.win.begin_resize_drag(edge, event.button, int(event.x_root), int(event.y_root), event.time)
            return True
        alt = bool(event.state & Gdk.ModifierType.MOD1_MASK)
        if event.button == 1 and alt and event.type == Gdk.EventType.BUTTON_PRESS:
            self._start_move(event.button, event.x_root, event.y_root, event.time, swallowed=True)
            return True
        if not self._in_titlebar(event.x, event.y):
            return False
        if event.button == 1 and event.type == double:
            self._press = None
            self.toggle_maximize()
            return True
        if event.button == 1 and event.type == Gdk.EventType.BUTTON_PRESS:
            self._press = (event.button, event.x_root, event.y_root, event.time)
            return True
        if event.button == 3 and event.type == Gdk.EventType.BUTTON_PRESS:
            gw = self.win.get_window()
            if gw is not None:
                try:
                    gw.show_window_menu(event)
                except Exception:  # noqa: BLE001
                    pass
            return True
        return event.type != Gdk.EventType.BUTTON_PRESS  # swallow triple clicks in the title bar

    def _on_release(self, _view, _event):
        self._button_down = False
        if self._hdrag is not None:
            swallowed = self._hdrag["swallowed"]
            self._hypr_drag_end()
            return swallowed
        if self._press is not None:
            self._press = None
            return True
        return False

    def _on_motion(self, _view, event):
        if self._hdrag is not None:
            if not (event.state & self.m["Gdk"].ModifierType.BUTTON1_MASK):
                self._hypr_drag_end()           # release got lost: never leave a window stuck to the pointer
                return True
            if not self._hdrag["native"]:
                self._hypr_drag_step()
            return True
        if self._press is not None:
            button, x0, y0, t0 = self._press
            if abs(event.x_root - x0) > 3 or abs(event.y_root - y0) > 3:
                self._press = None
                self._start_move(button, x0, y0, t0, swallowed=True)
            return True
        if self._button_down:
            return False
        edge = self._edge_at(event.x, event.y)
        self._set_cursor(edge)
        return edge is not None

    def _drag_from_last_press(self):
        lp = self._last_press
        if lp and self._button_down and time.monotonic() - lp[4] < 1.0:
            button, xr, yr, t, _ = lp
            self._start_move(button, xr, yr, t, swallowed=False)

    # -- moving the window --------------------------------------------------------
    def _start_move(self, button, x_root, y_root, t, swallowed: bool):
        """Start an interactive move. Plain Hyprland ignores xdg_toplevel.move (begin_move_drag).
        With the win11-csd plugin loaded it is a native, smooth drag (the plugin restores a
        maximized window under the pointer and the shell snaps at the screen edges on release);
        without it we follow the pointer ourselves through Hyprland's IPC socket."""
        if self.compositor == "hyprland" and not self._csd_plugin() and self._hypr_drag_begin(swallowed):
            return
        self._button_down = False
        self.win.begin_move_drag(button, int(x_root), int(y_root), t)

    def _csd_plugin(self) -> bool:
        """Is the win11-csd Hyprland plugin loaded right now? (one IPC round trip per drag)"""
        from .util import hyprctl
        return "win11-csd" in (hyprctl("plugin list", timeout=0.5) or "")

    def _hypr_client(self):
        from .util import hyprctl
        pid = os.getpid()
        for c in hyprctl("clients", json_out=True) or []:
            if c.get("pid") == pid and c.get("mapped", True):
                return c
        return None

    def _hypr_drag_begin(self, swallowed: bool) -> bool:
        from .util import hyprctl
        me, cur = self._hypr_client(), hyprctl("cursorpos", json_out=True)
        if not me or not isinstance(cur, dict):
            return False
        (wx, wy), (ww, wh) = me.get("at") or (0, 0), me.get("size") or (0, 0)
        cx, cy = float(cur.get("x", 0)), float(cur.get("y", 0))
        off_x, off_y = cx - wx, cy - wy
        if self.maximized or self.fullscreen or me.get("fullscreen"):
            # like Windows: a maximized window is restored under the pointer, keeping the
            # grab point at the same relative spot of the title bar
            try:
                self._api("windows.toggleMaximize", {"pid": os.getpid(), "want": False})
            except Exception:  # noqa: BLE001
                return False
            me = self._hypr_client() or me
            nw = (me.get("size") or (ww, wh))[0]
            if ww:
                off_x = off_x * nw / ww
        addr = me.get("address")
        # Hyprland's "mouse 1movewindow" answers ok but drops the drag on the next pointer event
        # unless a bindm key is held, so we follow the pointer ourselves, without move animation.
        native = False
        hyprctl(f"dispatch setprop address:{addr} no_anim 1")
        self._hdrag = {"addr": addr, "off": (off_x, off_y), "swallowed": swallowed, "native": native,
                       "last": 0.0, "cursor": (cx, cy)}
        if not native:
            self._hypr_drag_step(force=True)
        return True

    def _hypr_drag_step(self, force: bool = False):
        from .util import hyprctl
        d = self._hdrag
        now = time.monotonic()
        if not force and now - d["last"] < 0.008:      # ≈ one move per frame is plenty
            return
        cur = hyprctl("cursorpos", json_out=True)
        if not isinstance(cur, dict):
            return
        cx, cy = float(cur.get("x", 0)), float(cur.get("y", 0))
        d["cursor"], d["last"] = (cx, cy), now
        x, y = int(round(cx - d["off"][0])), int(round(cy - d["off"][1]))
        hyprctl(f"dispatch movewindowpixel exact {x} {y},address:{d['addr']}")

    def _hypr_drag_end(self):
        from .util import hyprctl
        d = self._hdrag
        if d["native"]:
            hyprctl("dispatch mouse 0movewindow")
            cur = hyprctl("cursorpos", json_out=True)
            if isinstance(cur, dict):
                d["cursor"] = (float(cur.get("x", 0)), float(cur.get("y", 0)))
        else:
            self._hypr_drag_step(force=True)
            hyprctl(f"dispatch setprop address:{d['addr']} no_anim 0")
        cx, cy = d["cursor"]
        self._hdrag = None
        # Windows snap: let go at the top edge = maximize, side = half, corner = quarter
        mon = None
        for m in hyprctl("monitors", json_out=True) or []:
            scale = float(m.get("scale") or 1) or 1
            mx, my = m.get("x", 0), m.get("y", 0)
            mw, mh = m.get("width", 0) / scale, m.get("height", 0) / scale
            if mx <= cx < mx + mw and my <= cy < my + mh:
                mon = (mx, my, mw, mh)
                break
        if not mon:
            return
        mx, my, mw, mh = mon
        left, right = cx <= mx + 2, cx >= mx + mw - 3
        top, bottom = cy <= my + 2, cy >= my + mh - 3
        edge = ("top-left" if left and top else "top-right" if right and top else
                "bottom-left" if left and bottom else "bottom-right" if right and bottom else
                "left" if left else "right" if right else "maximize" if top else None)
        if edge:
            threading.Thread(target=lambda: self._api("windows.snap", {"pid": os.getpid(), "edge": edge}),
                             name="snap", daemon=True).start()

    # -- events from the host (focus requests) ------------------------------------
    def _events_loop(self):
        GLib = self.m["GLib"]
        backoff = 1.0
        while not self._closing:
            conn = None
            try:
                conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=40)
                conn.request("GET", "/api/events", headers={"X-Shell-Token": self.token,
                                                            "Accept": "text/event-stream",
                                                            "Host": f"127.0.0.1:{self.port}"})
                resp = conn.getresponse()
                if resp.status != 200:
                    raise OSError(f"HTTP {resp.status}")
                backoff = 1.0
                event, data = None, []
                while not self._closing:
                    raw = resp.fp.readline()
                    if not raw:
                        break
                    line = raw.decode("utf-8", "replace").rstrip("\r\n")
                    if not line:
                        if event in ("app.present", "app.navigate") and data:
                            try:
                                payload = json.loads("\n".join(data))
                            except ValueError:
                                payload = {}
                            if isinstance(payload, dict) and payload.get("app") == self.app:
                                tok = payload.get("activationToken") if event == "app.present" else None
                                GLib.idle_add(self.present, tok)
                        elif event == "settings.changed" and data and self.app == "taskmgr" and self._shown:
                            try:
                                payload = json.loads("\n".join(data))
                            except ValueError:
                                payload = {}
                            if isinstance(payload, dict) and isinstance(payload.get("settings"), dict):
                                self.set_on_top(_always_on_top(payload["settings"]))
                        event, data = None, []
                    elif line.startswith("event:"):
                        event = line[6:].strip()
                    elif line.startswith("data:"):
                        data.append(line[5:].lstrip())
            except (OSError, http.client.HTTPException):
                pass
            finally:
                if conn is not None:
                    conn.close()
            if self._closing:
                return
            time.sleep(backoff)
            backoff = min(4.0, backoff * 2)        # back quickly after a shell restart


def _compositor() -> str:
    from .util import compositor
    return compositor()


def _cache_dir() -> Path:
    v = os.environ.get("WIN11SHELL_CACHE_DIR")
    if v:
        return Path(os.path.expanduser(v))
    return Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "win11shell"


def force_software_rendering(env: dict | None = None) -> None:
    """Taakbeheer must never get stuck: no GPU path, whatever ``no-gpu`` or the environment say (GPU WebViews
    leaked NVIDIA sync-file fds and froze, 2026-10-26). Before WebKit initialises."""
    from .util import apply_user_gpu_env, remember_user_gpu_env
    env = os.environ if env is None else env
    # the user's own switches first: what the host changed for its own WebViews, or leaked, is gone; they stay in
    # util.USER_GPU_MARK, so what Taakbeheer starts (ctl, a restarted shell) gets the user's values, not these
    apply_user_gpu_env(env, remember_user_gpu_env(env))
    env.pop("WEBKIT_FORCE_DMABUF_RENDERER", None)
    env.pop("WEBKIT_DMABUF_RENDERER_FORCE_SHM", None)
    env["WEBKIT_DISABLE_DMABUF_RENDERER"] = "1"
    env["WEBKIT_SKIA_ENABLE_CPU_RENDERING"] = "1"
    env["WIN11SHELL_GPU"] = "0"
    if env is os.environ:
        from .surfaces import software_only
        software_only()                                  # surfaces' own GPU decision keeps its hands off these


def _no_gpu_settings(mods: dict, view) -> None:
    WebKit2 = mods["WebKit2"]
    try:
        view.get_settings().set_hardware_acceleration_policy(WebKit2.HardwareAccelerationPolicy.NEVER)
    except Exception as e:  # noqa: BLE001
        log.info("Hardwareversnelling niet uit te zetten (cannot turn hardware acceleration off): %s", e)


_LOCK = {"fd": None, "exiting": False, "beating": False}
_RAISE: dict = {"window": None}


def _on_raise_signal(*_a) -> None:
    """RAISE_SIGNAL (Python-level handler: PyGObject's main loop wakes up for it). Before the window exists: later."""
    w = _RAISE.get("window")
    if w is None:
        _RAISE["pending"] = True
        return
    from gi.repository import GLib
    GLib.idle_add(lambda: (w.focus_self(), False)[1])
EXIT_GRACE_S = 3.0


def exit_soon(grace: float = EXIT_GRACE_S) -> None:
    """The window is gone: give the app id back at once (another Taakbeheer may start now) and make sure this
    process ends. WebKit's teardown can block the main thread for good (seen when the compositor sends no
    frames, e.g. the screen is off) — a lingering process must never keep "Taakbeheer is al open"."""
    if _LOCK["exiting"]:
        return
    _LOCK["exiting"] = True
    fd = _LOCK["fd"]
    if isinstance(fd, int) and fd >= 0:
        with contextlib.suppress(OSError):
            os.close(fd)
        _LOCK["fd"] = None

    def bail():
        time.sleep(grace)
        log.warning("Afsluiten bleef hangen; proces beëindigd (exit hung; leaving now)")
        with contextlib.suppress(Exception):
            logging.shutdown()
        os._exit(0)
    threading.Thread(target=bail, name="exit-guard", daemon=True).start()


def single_instance(app: str) -> int | None:
    """One window process per app (per runtime dir): an fd holding ``<runtime>/app-<app>.lock``, or None when
    another one holds it — that one was asked to show itself (RAISE_SIGNAL)."""
    from .util import _private_dir, runtime_dir
    d = runtime_dir()
    if not _private_dir(d, create=True):
        return -1                                   # no safe place for a lock: just run
    path = d / f"app-{app}.lock"
    try:
        fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, 0o600)
    except OSError:
        return -1
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        try:
            other = int((os.pread(fd, 32, 0).decode().split() or ["0"])[0])
        except (OSError, ValueError):
            other = 0
        os.close(fd)
        if other > 1:
            try:
                with open(f"/proc/{other}/cmdline", "rb") as f:
                    argv = f.read().split(b"\0")
                if b"host.appwindow" in argv and app.encode() in argv:
                    os.kill(other, RAISE_SIGNAL)
            except OSError:
                pass
        return None
    os.ftruncate(fd, 0)
    os.pwrite(fd, f"{os.getpid()} start\n".encode(), 0)    # start: not judged until the main loop runs (heartbeat)
    return fd


def heartbeat() -> bool:
    """Main-loop timer (HEARTBEAT_S) and first idle: touch our lock file — proof that the main loop still turns.
    The first call switches the marker from ``start`` to ``hb`` (from now on a missing heartbeat means hung)."""
    fd = _LOCK["fd"]
    if isinstance(fd, int) and fd >= 0 and not _LOCK["exiting"]:
        with contextlib.suppress(OSError):
            if not _LOCK.get("beating"):
                os.ftruncate(fd, 0)
                os.pwrite(fd, f"{os.getpid()} hb\n".encode(), 0)
                _LOCK["beating"] = True
            os.utime(fd)
    return True


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="python3 -m host.appwindow", description="Win11 Shell app window")
    p.add_argument("--app", required=True)
    p.add_argument("--port", type=int, default=0)
    p.add_argument("--token", default=None)
    p.add_argument("--params", default=None, help="JSON object, e.g. '{\"page\": \"display\"}'")
    p.add_argument("--dev", action="store_true")
    p.add_argument("--dry-run", action="store_true")
    a = p.parse_args(argv)
    logging.basicConfig(level=logging.DEBUG if a.dev else logging.INFO,
                        format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    from .surfaces import reap_inherited_children
    reap_inherited_children()     # after _restart_self (exec): the old image's WebKit processes

    from .util import own_app, read_instance
    app = own_app(a.app)
    if app is None:
        print(f"Onbekende app (unknown app): {a.app}", file=sys.stderr)
        return 2
    _ORIG_ENV.clear()
    _ORIG_ENV.update(os.environ)
    import faulthandler                     # SIGRTMIN+3 → every thread's Python stack on stderr (a hung window)
    with contextlib.suppress(Exception):
        faulthandler.register(signal.SIGRTMIN + 3, all_threads=True)
    # before anything can send it (a second process right after we took the lock): the default action would end us
    signal.signal(RAISE_SIGNAL, _on_raise_signal)
    lock = single_instance(app)
    if lock is None:
        log.info("%s is al open; dat venster komt naar voren (already open: raising it)", app)
        return 0
    _LOCK["fd"] = lock
    from .priority import place_app_window
    place_app_window(app)                   # own systemd --user scope (Taakbeheer: high CPU weight)
    if app == "taskmgr":
        force_software_rendering()
    token = a.token or os.environ.pop("WIN11SHELL_TOKEN", None)
    port = a.port
    if not token or not port:
        inst = read_instance()
        if inst:
            token = token or inst.get("token")
            port = port or int(inst.get("port") or 0)
    if not token or not port:
        print("Win11 Shell draait niet (no running host: give --port/--token)", file=sys.stderr)
        return 1
    params = {}
    if a.params:
        try:
            params = json.loads(a.params)
            if not isinstance(params, dict):
                raise ValueError
        except ValueError:
            print("--params moet een JSON-object zijn (must be a JSON object)", file=sys.stderr)
            return 2
    dry_run = a.dry_run or os.environ.get("WIN11SHELL_DRY_RUN") == "1"

    # prgname / WM class BEFORE GTK initialises → Wayland app_id = win11shell-<app>
    from .surfaces import gi_modules, init_gtk
    if not init_gtk(f"win11shell-{app}", TITLES[_language()][app]):
        print("Geen grafische omgeving gevonden (no display)", file=sys.stderr)
        return 1
    mods = gi_modules(webkit=True)
    Gtk, GLib = mods["Gtk"], mods["GLib"]
    window = AppWindow(mods, app, port, token, params, a.dev, dry_run)

    def on_signal():
        log.info("Signaal ontvangen; venster sluiten (signal received, closing)")
        window.close_from_signal()
        return False

    from .util import glib_signal_add
    for sig in (signal.SIGTERM, signal.SIGINT, signal.SIGHUP):
        glib_signal_add(sig, on_signal)
    GLib.idle_add(lambda: (heartbeat(), False)[1])     # from the running main loop: marker start → hb
    GLib.timeout_add_seconds(HEARTBEAT_S, heartbeat)
    _RAISE["window"] = window
    if _RAISE.pop("pending", False):
        GLib.idle_add(lambda: (window.focus_self(), False)[1])
    if a.dev or os.environ.get("WIN11SHELL_DEBUG_HANG") == "1":
        from .surfaces import run_js

        def hang():                         # test aid: an endless loop in the page (the watchdog must recover)
            log.warning("[dev] pagina in een eindeloze lus gezet (page put into an endless loop)")
            run_js(window.view, "setTimeout(function(){for(;;){}},0);")
            return True
        signal.signal(HANG_SIGNAL, lambda *_a: GLib.idle_add(lambda: (hang(), False)[1]))
        # test aid: what the notice's "Win11 Shell opnieuw starten" button does (GLib takes no RT signals)
        signal.signal(signal.SIGRTMIN + 4,
                      lambda *_a: GLib.idle_add(lambda: (window.restart_shell(by_notice=True), False)[1]))
        signal.signal(signal.SIGRTMIN + 5, lambda *_a: GLib.idle_add(lambda: (window.dev_probe(), False)[1]))
    Gtk.main()
    # Skip interpreter finalization (native WebKit destructors at exit can crash).
    logging.shutdown()
    sys.stdout.flush()
    sys.stderr.flush()
    os._exit(0)


if __name__ == "__main__":
    sys.exit(main())
