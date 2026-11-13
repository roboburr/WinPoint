"""Never stuck, inside the host process (SPEC §2.2): the main-loop beat, the stuck watch and the exit deadline.

The HTTP server runs in its own threads, so a host whose GTK main loop hangs (a C call into WebKit/EGL that never
returns, a WebKit teardown that blocks, an atk-bridge call) still answers ``shell.info`` while its taskbar is frozen
and its own page watchdogs (GLib timers on that loop) never fire. So:

* **beat** — a GLib timer (BEAT_MS) on the main loop writes ``time.monotonic()`` into ``ctx.loop_beat``;
  ``shell.info`` answers ``mainLoopLagMs``. App windows (host/appwindow.py) and Taakbeheer's page
  (ui/js/apps/taskmgr/hostcall.js) count a lag above LOOP_STUCK_MS as "Win11 Shell reageert niet", so their restart
  goes through ``win11shell-ctl restart --force`` (it works whatever the loop does). A beat more than STALL_S late
  means the loop stood still in between: its end goes into ``ctx.loop_stall_end``, and the shell's own page
  watchdogs (host/surfaces.py) ask ``loop_stalled_since()`` before they call a page hung — a page whose answer was
  queued behind our own stalled loop is busy, not hung (a long stall is this module's job: the watch below).
  The same watchdogs (and the app windows') ask ``system_thrashing()`` too: with memory or I/O pressure this high
  (PSI ``full avg10``) a web process that is paged out or waits for the disk answers late; the verdict waits (≤ 30 s
  after the probe), since killing and reloading it only adds to the pressure.
* **watch** (layer/preview) — a thread: no beat for STUCK_S → emergency restart, without anyone's help. A watch
  thread that did not run itself for WAKE_GAP_S (the process was stopped, the machine busy) first gives the loop a
  fresh STUCK_S.
* **exit deadline** — ``ctx.request_restart()`` / ``request_quit()`` only ask the main loop to stop
  (``GLib.idle_add``): on a hung loop, or a teardown that blocks, nothing would ever happen. The process must have
  exec'd (in-place restart) within RESTART_DEADLINE_S, or exited within QUIT_DEADLINE_S; else: emergency restart /
  exit.
* **emergency restart** — our own WebKit processes and helpers (night light, clipboard watcher, PipeWire monitor,
  indexer, thumbnailer: children of this process, never apps it launched) get SIGKILL; under bin/win11shell's
  supervisor: ``<runtime>/restart-request`` (pid, port, token) + exit code 75 → the supervisor starts the shell again
  at once on the same port with the same token (open app windows reconnect, no crash count); without a supervisor:
  ``win11shell-ctl restart --force --no-wait`` (detached) does that, then exit 75.
"""

from __future__ import annotations

import contextlib
import logging
import os
import signal
import subprocess
import sys
import threading
import time
from typing import Callable

log = logging.getLogger("win11shell.lifeguard")

BEAT_MS = 1000
LOOP_STUCK_MS = 8000          # shell.info's mainLoopLagMs above this = the host answers, but its main loop stands still
STALL_S = 2.5                 # a beat this late: the main loop stood still in between (loop_stalled_since)
STUCK_S = 30.0                # layer/preview: no beat this long → restart by itself
WAKE_GAP_S = 5.0              # the watch thread did not run this long itself (SIGSTOP/SIGCONT, swap storm)
RESTART_DEADLINE_S = 8.0
QUIT_DEADLINE_S = 10.0
EXIT_EMERGENCY = 75
PRESSURE_FULL_PCT = 10.0      # /proc/pressure/{memory,io} "full avg10" at or above this: the machine is thrashing

# long-lived helpers the host starts for itself (direct children): they outlive a host that is killed (SIGKILL skips
# stop_services) — hyprsunset would keep the night light on, wl-paste watchers would pile up
HELPER_MARKS = ("hyprsunset", "wl-paste", "pw-dump", "host.indexer", "thumbnailer.py", "glycin")


def _argv(pid: int) -> list[str]:
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return [a.decode("utf-8", "replace") for a in f.read().split(b"\0") if a]
    except OSError:
        return []


def is_helper_argv(argv: list[str]) -> bool:
    """One of the host's own long-lived helpers (HELPER_MARKS): hyprsunset / wl-paste / pw-dump / a glycin loader
    itself, ``python3 -m host.indexer``, or the thumbnailer / a glycin loader behind bwrap. Exact names only: an
    app the host launched with such a word in a file name is never one. Pure (tested)."""
    if not argv:
        return False
    base = os.path.basename(argv[0])
    if base in ("hyprsunset", "wl-paste", "pw-dump") or base.startswith("glycin"):
        return True
    if "python" in base and "host.indexer" in argv[1:4]:
        return True
    if base == "bwrap" or "python" in base:
        return any(os.path.basename(a) == "thumbnailer.py" or os.path.basename(a).startswith("glycin-")
                   for a in argv[1:])
    return False


def helper_children(root: int, table: dict | None = None) -> dict[int, bytes]:
    """{pid: start time} of ``root``'s direct children that are its own helpers (is_helper_argv) — taken while
    ``root`` lives, killed with surfaces.kill_webkit_leftovers() (same start time only) once it is gone."""
    from .surfaces import _proc_table, _start_time
    table = _proc_table() if table is None else table
    out = {}
    for pid, (ppid, _comm) in table.items():
        if ppid == root and is_helper_argv(_argv(pid)):
            st = _start_time(pid)
            if st is not None:
                out[pid] = st
    return out


def kill_own_children(kill: Callable = os.kill) -> list[int]:
    """This process's WebKit processes and helpers: SIGKILL (our own unreaped children: their pids cannot have been
    reused). Apps the host launched for the user are left alone. → the pids signalled."""
    from .surfaces import _WEBKIT_GLUE, _proc_table, webkit_children
    me = os.getpid()
    table = _proc_table()
    victims = set(webkit_children(me, table))
    for pid, (ppid, comm) in table.items():
        if ppid == me and (comm.startswith(b"WebKit") or comm in _WEBKIT_GLUE or is_helper_argv(_argv(pid))):
            victims.add(pid)
    done = []
    for pid in sorted(victims):
        with contextlib.suppress(OSError):
            kill(pid, signal.SIGKILL)
            done.append(pid)
    return done


def _supervised() -> bool:
    try:
        from .ctl import is_supervisor
        return is_supervisor(os.getppid())
    except Exception:  # noqa: BLE001
        return False


def _spawn_ctl_restart() -> None:
    from . import ROOT
    from .util import user_env
    env = {k: v for k, v in user_env().items() if k not in ("WIN11SHELL_TOKEN", "XDG_ACTIVATION_TOKEN",
                                                            "DESKTOP_STARTUP_ID")}   # user's renderer switches
    env["PYTHONPATH"] = str(ROOT) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    python = "/usr/bin/python3" if os.path.exists("/usr/bin/python3") else sys.executable
    subprocess.Popen([python, "-m", "host.ctl", "restart", "--force", "--no-wait"], cwd=str(ROOT), env=env,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True, close_fds=True)


def _flush() -> None:
    with contextlib.suppress(Exception):
        logging.shutdown()
    for s in (sys.stdout, sys.stderr):
        with contextlib.suppress(Exception):
            s.flush()


def emergency_restart(ctx, why: str, *, exit_: Callable[[int], None] = os._exit,
                      flush_: Callable[[], None] = _flush) -> None:
    """See the module doc. Never returns (with the real ``exit_``)."""
    log.error("Noodherstart van Win11 Shell: %s (emergency restart)", why)
    with contextlib.suppress(Exception):
        kill_own_children()
    if ctx.mode in ("layer", "preview"):
        try:
            if _supervised():
                from .ctl import write_restart_request
                write_restart_request(os.getpid(), ctx.port, ctx.token)
            else:
                _spawn_ctl_restart()
        except Exception:  # noqa: BLE001
            log.exception("Noodherstart niet voorbereid (emergency restart not prepared)")
    flush_()
    exit_(EXIT_EMERGENCY)


def emergency_exit(code: int, *, exit_: Callable[[int], None] = os._exit) -> None:
    log.error("Afsluiten bleef hangen; proces beëindigd (exit hung; leaving now)")
    with contextlib.suppress(Exception):
        kill_own_children()
    _flush()
    exit_(int(code or 0))


def loop_stalled_since(ctx, since: float, now: float | None = None) -> bool:
    """Did the host's main loop stand still after ``since`` (a beat more than STALL_S late), or is its beat overdue
    right now (a timer of the same loop ran before the beat after a stall)? False without a beat (no LoopGuard:
    unit tests, serve mode). Monotonic clock, as LoopGuard and the page watchdogs use."""
    beat = getattr(ctx, "loop_beat", None)
    if beat is None:
        return False
    now = time.monotonic() if now is None else now
    if now - beat > STALL_S:
        return True
    end = getattr(ctx, "loop_stall_end", None)
    return end is not None and end > since


def pressure_full_avg10(resource: str, root: str = "/proc/pressure") -> float | None:
    """PSI: the share of the last 10 s (percent) in which every non-idle task waited for ``resource`` ('memory' |
    'io'); None when unreadable (no PSI in this kernel, a sandbox)."""
    try:
        with open(os.path.join(root, resource), encoding="ascii") as f:
            for line in f:
                if line.startswith("full "):
                    for part in line.split()[1:]:
                        k, _, v = part.partition("=")
                        if k == "avg10":
                            return float(v)
    except (OSError, ValueError):
        return None
    return None


def system_thrashing(threshold: float = PRESSURE_FULL_PCT, root: str = "/proc/pressure") -> bool:
    """Memory or I/O pressure this high (a swap storm, an I/O storm at login): a page that answers late is slow, not
    hung. Two small reads, only when a page watchdog is about to give a verdict."""
    return any((pressure_full_avg10(r, root) or 0.0) >= threshold for r in ("memory", "io"))


class LoopGuard:
    """``beat()`` is the main loop's timer; ``stuck()`` one look of the watch thread (pure with ``clock``: tested);
    ``watch()`` starts that thread; ``arm(kind)`` the exit deadline ('restart' | 'quit')."""

    def __init__(self, ctx, *, stuck_s: float = STUCK_S, clock: Callable[[], float] = time.monotonic,
                 on_stuck: Callable[[str], None] | None = None, on_quit_hung: Callable[[int], None] | None = None):
        self.ctx = ctx
        self.stuck_s = stuck_s
        self.clock = clock
        self.on_stuck = on_stuck or (lambda why: emergency_restart(ctx, why))
        self.on_quit_hung = on_quit_hung or emergency_exit
        self._last_wake: float | None = None
        self._fresh_from: float | None = None
        self.fired = False
        self._armed: set[str] = set()
        ctx.loop_beat = None
        ctx.loop_stall_end = None

    def beat(self) -> bool:
        now = self.clock()
        prev = getattr(self.ctx, "loop_beat", None)
        if prev is not None and now - prev > STALL_S:
            self.ctx.loop_stall_end = now
        self.ctx.loop_beat = now
        return True

    def stuck(self, now: float | None = None) -> bool:
        now = self.clock() if now is None else now
        last, self._last_wake = self._last_wake, now
        if last is not None and now - last > WAKE_GAP_S:
            self._fresh_from = now
        beat = getattr(self.ctx, "loop_beat", None)
        if beat is None or self.ctx.shutting_down:
            return False                  # not started yet / stopping: the exit deadline covers that
        return now - max(beat, self._fresh_from or beat) >= self.stuck_s

    def watch(self, interval: float = 1.0) -> threading.Thread:
        def loop():
            while not self.fired:
                time.sleep(interval)
                if self.stuck():
                    self.fired = True
                    lag = self.clock() - (self.ctx.loop_beat or 0)
                    log.error("De hoofdlus van Win11 Shell staat al %.0f s stil (main loop stuck)", lag)
                    self.on_stuck(f"de hoofdlus stond {lag:.0f} s stil")
                    return
        t = threading.Thread(target=loop, name="loop-guard", daemon=True)
        t.start()
        return t

    def arm(self, kind: str) -> None:
        """After request_restart ('restart') / request_quit ('quit'): the process must be gone in time."""
        if kind in self._armed:
            return
        self._armed.add(kind)
        delay = RESTART_DEADLINE_S if kind == "restart" else QUIT_DEADLINE_S

        def later():
            time.sleep(delay)
            if self.fired:
                return
            self.fired = True
            if kind == "restart" or self.ctx.restart_requested:
                log.error("Herstart na %.0f s nog niet gelukt (restart hangs in teardown)", delay)
                self.on_stuck("de herstart bleef hangen")
            else:
                self.on_quit_hung(self.ctx.exit_code)
        threading.Thread(target=later, name=f"exit-deadline-{kind}", daemon=True).start()
