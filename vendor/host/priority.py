"""CPU priority for Win11 Shell's own processes, without root (SPEC §2.1): transient systemd --user scopes.

Measured on this machine (tests/nested/priority/README.md; 8 cores / 16 threads, 32 busy processes): what keeps
the shell responsive under a CPU hog is *not sharing a cgroup with it* — a UI-like loop: 65 % late frames in the
hog's cgroup, 0–0.2 % in its own; Taakbeheer's ``sysmon.processes``: p50 73 → 36 ms, p95 143 → 45–58 ms. A higher
CPUWeight on top trims the tail a little (synthetic p99 12.2 → 10.0 ms, max 17 → 10.5 ms). So:

* the desktop host moves itself into ``win11shell-shell-<pid>.scope``   CPUWeight=1000 (its WebKit
  processes are started after that and stay with it)
* Taakbeheer moves itself into ``app-win11shell-taskmgr-<pid>.scope``  CPUWeight=1000
* the other app windows into ``app-win11shell-<app>-<pid>.scope``      default weight, like any app — a
  hung page there never competes inside the shell's cgroup
* an app-mode host serving Taakbeheer (Ctrl+Shift+Esc without a desktop shell) into
  ``win11shell-apphost-<pid>.scope``                                    CPUWeight=1000
* what the host starts *for the user* leaves the shell's scope: an app launched from Start, the taskbar or
  Verkenner (Gio, which neither calls setsid nor makes a scope) is moved into ``app-hyprland-<id>-<pid>.scope``
  the moment it is launched (``place_launched``, from GAppLaunchContext::launched); anything else the host
  started that is not one of its own helpers — also what an app forked before it was moved, a terminal, an
  updater — is swept into its own app scope within SWEEP_S, the indexer into ``background.slice`` (see
  ``ScopeSweeper``). Otherwise a browser, a game or a ``make -j`` in a terminal would share the shell's cgroup
  (measured: the worst case) and ride along on its weight.

``IOWeight=1000`` is set as well, but has no effect on this machine: systemd --user gets no io controller
(``user@.service`` delegates cpu memory pids) and the disks use the ``none``/``mq-deadline`` schedulers
(io.weight needs BFQ or io.cost). Nice values below 0 need privileges, so they are not used.

Everything goes through ``org.freedesktop.systemd1.Manager.StartTransientUnit`` on the session bus, the
way GLib puts launched apps into scopes. No systemd --user, no session bus, or ``WIN11SHELL_PRIORITY=0``:
nothing happens and nothing breaks.
"""

from __future__ import annotations

import logging
import os
import re
import threading
import time

log = logging.getLogger("win11shell.priority")

HIGH_CPU_WEIGHT = 1000
HIGH_IO_WEIGHT = 1000
SLICE = "app.slice"
BACKGROUND_SLICE = "background.slice"
SHELL_PREFIX = "win11shell-shell-"
APPHOST_PREFIX = "win11shell-apphost-"
OWN_HIGH_PREFIXES = (SHELL_PREFIX, APPHOST_PREFIX)
SWEEP_S = 3.0

# Children of the host that are part of the shell itself (WebKit and its sandbox, image loaders, monitors,
# the lock screen…) always stay in its scope.
_OWN_HELPERS = ("webkit", "xdg-dbus-proxy", "host.appwindow", "thumbnailer.py", "glycin", "bwrap", "wl-paste",
                "wl-copy", "pw-dump", "pactl", "hyprsunset", "hyprlock", "hypridle", "gdbus", "busctl",
                "nvidia-smi")
MIN_AGE_S = 1.0


def enabled() -> bool:
    """Off with WIN11SHELL_PRIORITY=0, and always in a unit-test process (never a scope in the real user manager)."""
    if os.environ.get("WIN11SHELL_PRIORITY", "1") == "0":
        return False
    from .util import _under_unittest
    return not _under_unittest() or os.environ.get("WIN11SHELL_ALLOW_REAL") == "1"


def cgroup_of(pid: int | str = "self") -> str:
    """'/user.slice/…/app.slice/app-x-1.scope' (the cgroup v2 path), '' when unknown."""
    try:
        with open(f"/proc/{pid}/cgroup", encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("0::"):
                    return line[3:].strip()
    except OSError:
        pass
    return ""


def unit_of(pid: int | str = "self") -> str:
    """Last component of the cgroup path (the scope/service name)."""
    return cgroup_of(pid).rstrip("/").rsplit("/", 1)[-1]


def escape_unit_part(text: str) -> str:
    """systemd-escape for one name part: [A-Za-z0-9:_.] stay, everything else (also '-') → \\xNN.
    A leading '.' is escaped too. Pure (tested)."""
    out = []
    for i, ch in enumerate(text):
        if (ch.isascii() and (ch.isalnum() or ch in ":_.")) and not (i == 0 and ch == "."):
            out.append(ch)
        else:
            out.extend(f"\\x{b:02x}" for b in ch.encode("utf-8"))
    return "".join(out) or "x"


def app_window_unit(app: str, pid: int) -> str:
    # app-<id>-<pid>.scope with id "win11shell-<app>": Taakbeheer's sysmon groups the window's WebKit
    # processes under the window's own desktop entry (win11shell-<app>.desktop), as before.
    return f"app-win11shell-{app}-{pid}.scope"


def child_app_unit(exe: str, pid: int) -> str:
    # GLib's own pattern (app-<launcher>-<escaped id>-<pid>.scope), launcher "hyprland"
    return f"app-hyprland-{escape_unit_part(exe)[:120]}-{pid}.scope"


def _bus():
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio
    return Gio.bus_get_sync(Gio.BusType.SESSION, None)


def start_scope(pids: list[int], unit: str, *, cpu_weight: int | None = None, io_weight: int | None = None,
                slice_: str = SLICE, description: str = "", timeout_ms: int = 2000, wait: float = 1.0) -> bool:
    """Move ``pids`` into a new transient scope ``unit`` of the user manager. True once the first pid is in it
    (or, with ``wait=0``, once systemd accepted the job). Never raises."""
    if not enabled() or not pids:
        return False
    try:
        import gi
        gi.require_version("GLib", "2.0")
        from gi.repository import Gio, GLib
        props = [("PIDs", GLib.Variant("au", [int(p) for p in pids])),
                 ("Slice", GLib.Variant("s", slice_)),
                 ("CollectMode", GLib.Variant("s", "inactive-or-failed"))]
        if description:
            props.append(("Description", GLib.Variant("s", description[:200])))
        if cpu_weight:
            props.append(("CPUWeight", GLib.Variant("t", int(cpu_weight))))
        if io_weight:
            props.append(("IOWeight", GLib.Variant("t", int(io_weight))))
        _bus().call_sync("org.freedesktop.systemd1", "/org/freedesktop/systemd1",
                         "org.freedesktop.systemd1.Manager", "StartTransientUnit",
                         GLib.Variant("(ssa(sv)a(sa(sv)))", (unit, "fail", props, [])),
                         GLib.VariantType.new("(o)"), Gio.DBusCallFlags.NONE, timeout_ms, None)
    except Exception as e:  # noqa: BLE001 - no user manager / no bus / unit exists: stay where we are
        log.info("Geen eigen scope %s (no own scope): %s", unit, getattr(e, "message", e))
        return False
    end = time.monotonic() + max(0.0, wait)
    while True:
        if unit_of(pids[0]) == unit:
            return True
        if time.monotonic() >= end:
            return wait <= 0
        time.sleep(0.02)


def place_host() -> bool:
    """The desktop host (layer/preview mode): own high-priority scope, before any WebKit process exists.
    After an in-place restart (exec) it is already there."""
    pid = os.getpid()
    if unit_of(pid).startswith(SHELL_PREFIX):
        return True
    ok = start_scope([pid], f"{SHELL_PREFIX}{pid}.scope", cpu_weight=HIGH_CPU_WEIGHT, io_weight=HIGH_IO_WEIGHT,
                     description="Win11 Shell (bureaublad en taakbalk)")
    if ok:
        log.info("Hoge prioriteit: eigen scope %s (CPUWeight=%d)", unit_of(pid), HIGH_CPU_WEIGHT)
    return ok


def place_app_host(app: str | None) -> bool:
    """An app-mode host (no desktop shell runs) that serves Taakbeheer: the shell's priority for its backend too —
    under a CPU hog its sysmon polls would otherwise miss their timeouts and the fallback Taakbeheer would say
    "Win11 Shell reageert niet". Other apps' app-mode hosts stay where they are."""
    if app != "taskmgr":
        return False
    pid = os.getpid()
    if unit_of(pid).startswith(APPHOST_PREFIX):
        return True
    ok = start_scope([pid], f"{APPHOST_PREFIX}{pid}.scope", cpu_weight=HIGH_CPU_WEIGHT, io_weight=HIGH_IO_WEIGHT,
                     description="Win11 Shell: Taakbeheer (zonder bureaublad)")
    if ok:
        log.info("Hoge prioriteit voor Taakbeheer zonder bureaublad: %s (CPUWeight=%d)", unit_of(pid), HIGH_CPU_WEIGHT)
    return ok


def launched_unit(app_id: str, pid: int) -> str:
    """Scope for an app the host launched: GLib's pattern ``app-<launcher>-<escaped id>-<pid>.scope`` (launcher
    "hyprland"), so Taakbeheer groups it under its desktop entry. Pure (tested)."""
    base = os.path.basename(str(app_id or "")).removesuffix(".desktop") or "app"
    return f"app-hyprland-{escape_unit_part(base)[:120]}-{int(pid)}.scope"


def place_launched(pid: int, app_id: str) -> bool:
    """Right after Gio launched ``pid`` for the user (apps.launch_context's ``launched`` handler, in a thread): out
    of our high-priority scope into an app scope of its own. Only a process that is still in our own scope is moved
    (a D-Bus activated app is not ours to move)."""
    if not enabled() or pid <= 1:
        return False
    mine = unit_of(os.getpid())
    if not mine.startswith(OWN_HIGH_PREFIXES) or unit_of(pid) != mine:
        return False
    unit = launched_unit(app_id, pid)
    ok = start_scope([pid], unit, description=os.path.basename(str(app_id or "")) or unit, wait=0)
    if ok:
        log.info("Gestarte app in eigen scope (launched app moved to its own scope): %s", unit)
    return ok


def place_app_window(app: str) -> bool:
    """An app window process, before GTK/WebKit start: Taakbeheer gets the shell's priority, the others
    a scope of their own with the default weight."""
    pid = os.getpid()
    unit = app_window_unit(app, pid)
    current = unit_of(pid)
    if current == unit or current.startswith(f"app-win11shell-{app}-"):
        return True
    high = app == "taskmgr"
    names = {"taskmgr": "Taakbeheer", "settings": "Instellingen", "explorer": "Verkenner", "alles": "Alles",
             "disks": "Schijven"}
    return start_scope([pid], unit, cpu_weight=HIGH_CPU_WEIGHT if high else None,
                       io_weight=HIGH_IO_WEIGHT if high else None,
                       description=f"Win11 Shell: {names.get(app, app)}")


# ---------------------------------------------------------------------------------------------------------
# the host's own scope: keep it to the shell
# ---------------------------------------------------------------------------------------------------------

def _argv(pid: int) -> list[str]:
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return [a.decode("utf-8", "replace") for a in f.read().split(b"\0") if a]
    except OSError:
        return []


def _stat(pid: int) -> tuple[int, int, float] | None:
    """(ppid, session id, age in seconds) from /proc/<pid>/stat."""
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            data = f.read()
        f_ = data[data.rfind(b")") + 2:].split()
        with open("/proc/uptime", "rb") as f:
            up = float(f.read().split()[0])
        return int(f_[1]), int(f_[3]), up - int(f_[19]) / (os.sysconf("SC_CLK_TCK") or 100)
    except (OSError, ValueError, IndexError):
        return None


def classify_child(argv: list[str], session_leader: bool = False) -> str:
    """What to do with a process tree the host started inside its scope: 'keep' | 'background' | 'app'.
    Only the host's own helpers (WebKit, image loaders, monitors, the lock screen …) stay; the indexer goes to the
    background; everything else is an app — also when it is no session leader: Gio launches apps without setsid,
    and a short command of the host's (``util.run``) that runs longer than MIN_AGE_S does no harm in a scope of its
    own (``session_leader`` is kept for callers, it no longer decides). Pure (tested)."""
    if not argv:
        return "keep"
    if "host.indexer" in argv:
        return "background"
    joined = " ".join(argv).lower()
    if any(h in joined for h in _OWN_HELPERS):
        return "keep"
    return "app"


class ScopeSweeper:
    """Every few seconds: process trees the host started for the user leave the host's scope for a scope of
    their own, so a compile in a terminal opened from Verkenner never shares — or borrows — the shell's
    priority; the indexer goes to background.slice."""

    def __init__(self, host_pid: int | None = None, interval: float = SWEEP_S):
        self.host = host_pid or os.getpid()
        self.interval = interval
        self.stop = threading.Event()
        self.failed: dict[int, float] = {}
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        if self._thread is None:
            self._thread = threading.Thread(target=self._loop, name="scope-sweeper", daemon=True)
            self._thread.start()

    def _loop(self) -> None:
        while not self.stop.wait(self.interval):
            try:
                self.sweep()
            except Exception:  # noqa: BLE001
                log.debug("scope sweep failed", exc_info=True)

    def members(self) -> list[int]:
        cg = cgroup_of(self.host)
        if not cg or not unit_of(self.host).startswith(OWN_HIGH_PREFIXES):
            return []
        try:
            with open(f"/sys/fs/cgroup{cg}/cgroup.procs", encoding="ascii") as f:
                return [int(x) for x in f.read().split() if x.strip().isdigit()]
        except OSError:
            return []

    def plan(self, members: list[int]) -> list[tuple[str, str, list[int]]]:
        """[(what, unit, pids)] for the members of the host's scope (the host itself excluded): every member
        belongs to the highest ancestor that is still in the scope."""
        mset = set(members) - {self.host}
        info = {p: _stat(p) for p in mset}
        top_of: dict[int, int] = {}
        for pid in mset:
            p, n = pid, 0
            while n < 64 and info.get(p) and info[p][0] in mset:
                p, n = info[p][0], n + 1
            top_of[pid] = p
        groups: dict[int, list[int]] = {}
        for pid, top in top_of.items():
            groups.setdefault(top, []).append(pid)
        now = time.monotonic()
        out = []
        for top, pids in sorted(groups.items()):
            st = info.get(top)
            if st is None or st[2] < MIN_AGE_S or now - self.failed.get(top, -1e9) < 60:
                continue
            argv = _argv(top)
            what = classify_child(argv, st[1] == top)
            if what == "keep":
                continue
            pids = [top] + sorted(p for p in pids if p != top)
            if what == "background":
                out.append((what, f"win11shell-indexer-{top}.scope", pids))
            else:
                out.append((what, child_app_unit(os.path.basename(argv[0]), top), pids))
        return out

    def sweep(self) -> list[tuple[str, list[int]]]:
        moved = []
        for what, unit, pids in self.plan(self.members()):
            if what == "background":
                ok = start_scope(pids, unit, slice_=BACKGROUND_SLICE, description="Win11 Shell: Alles-index", wait=0)
            else:
                ok = start_scope(pids, unit, description=_argv(pids[0])[0] if _argv(pids[0]) else unit, wait=0)
            if ok:
                log.info("Uit de shell-scope gehaald (moved out of the shell scope): %s %s", unit, pids[:8])
                moved.append((unit, pids))
            else:
                self.failed[pids[0]] = time.monotonic()
        return moved


_UNIT_SAFE = re.compile(r"^[A-Za-z0-9:_.\\-]{1,250}\.scope$")


def systemd_run_scope(unit: str) -> list[str] | None:
    """Command prefix that runs a program in its own transient scope (``systemd-run --user --scope``), or None
    when that is not available here (ctl uses it to start the shell outside Taakbeheer's scope)."""
    from .util import which
    if not enabled() or not _UNIT_SAFE.match(unit) or not which("systemd-run"):
        return None
    rt = os.environ.get("XDG_RUNTIME_DIR")
    if not rt or not os.path.exists(os.path.join(rt, "systemd", "private")):
        return None
    return ["systemd-run", "--user", "--scope", "--quiet", "--collect", f"--unit={unit}", "--"]
