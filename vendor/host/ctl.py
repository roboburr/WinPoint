"""win11shell-ctl: talk to the running Win11 Shell (SPEC §6.2, §9). Used by Hyprland key binds.

    win11shell-ctl toggle-start | toggle-search | quick-settings | notifications | task-view | alt-tab
                   winx | show-desktop | clipboard | run
    win11shell-ctl alt-tab reverse              (Alt+Shift+Tab: one step back)
    win11shell-ctl osd volume up|down|mute      (changes the volume, then shows the OSD)
    win11shell-ctl osd brightness up|down
    win11shell-ctl taskmgr | settings [page] | explorer [path] | alles [query] | disks [volume]
    win11shell-ctl taskmgr --direct             (Ctrl+Shift+Esc: works when the shell does not answer, see below)
    win11shell-ctl snap left|right|up|down
    win11shell-ctl lock
    win11shell-ctl power shutdown|reboot|suspend|logout|lock   (the shell asks for confirmation)
    win11shell-ctl info | status | quit | call <method> [json-args]
    win11shell-ctl restart [--force]            (Ctrl+Shift+Windows-toets+B: restart --force)

Exit codes: 0 ok · 1 failed (message on stderr) · 2 wrong usage.

Never stuck (SPEC §2.2): ``restart`` asks the shell (``shell.restart``, ~2 s) and checks that it really came
back; when it does not answer — or with ``--force`` — the host process itself is stopped (SIGTERM, SIGKILL
after 3 s; only the process that the instance file names *and* that is a ``python3 -m host --mode layer|preview``
of this user with the file's start time — or, when the file names no live desktop host, the child of the
supervisor that holds ``<runtime>/supervisor.lock``). Its WebKit processes and long-lived helpers (night light,
clipboard watcher …) that outlive it are killed too. ``bin/win11shell``'s supervisor starts it again on the
same port with the same token (so open app windows reconnect), told by ``<runtime>/restart-request`` that the
user asked for it (no crash count, no back-off); without a supervisor ctl starts ``bin/win11shell`` itself.
``taskmgr --direct`` focuses an open Taakbeheer through Hyprland, else asks the host (≤ 0.8 s), else starts
the Taakbeheer window process itself with the instance's port and token (its own page shows that the shell
does not answer and offers to restart it), else a standalone Taakbeheer (app mode).
"""

from __future__ import annotations

import contextlib
import fcntl
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

from . import APP_IDS, ROOT
from .util import ApiError, call_instance, read_instance, run, runtime_dir, which

SHELL_COMMANDS = ("toggle-start", "toggle-search", "quick-settings", "notifications", "task-view", "alt-tab",
                  "winx", "show-desktop", "clipboard", "run")
POWER_ACTIONS = ("shutdown", "reboot", "suspend", "logout", "lock")
# Win11 Super+arrows: up = maximize (or upper quadrant), down = restore / lower quadrant / minimize
# (windows.snap implements the 'up'/'down' semantics).
SNAP = {"left": "left", "right": "right", "up": "up", "down": "down"}
REVERSE_WORDS = ("reverse", "--reverse", "-r", "back", "terug")
DEFAULT_PARAM = {"settings": "page", "taskmgr": "page", "explorer": "path", "alles": "query", "disks": "volume"}
STEP = 5

def _is_english() -> bool:
    """Follow the shell's own language setting (default Dutch), like the UI."""
    from .util import read_json, xdg_config_home
    cfg = os.environ.get("WIN11SHELL_CONFIG_DIR") or str(xdg_config_home() / "win11shell")
    data = read_json(os.path.join(os.path.expanduser(cfg), "settings.json"), {})
    return isinstance(data, dict) and data.get("language") == "en"


_EN = _is_english()
MSG = {
    "not_running": ("Win11 Shell draait niet.", "Win11 Shell is not running."),
    "unknown": ("Onbekende opdracht: {x}", "Unknown command: {x}"),
    "usage": ("Gebruik: win11shell-ctl <opdracht>. Zie 'win11shell-ctl help'.",
              "Usage: win11shell-ctl <command>. See 'win11shell-ctl help'."),
    "failed": ("Mislukt: {x}", "Failed: {x}"),
    "starting": ("{x} wordt geopend…", "Opening {x}…"),
    "restarting": ("Win11 Shell wordt opnieuw gestart…", "Restarting Win11 Shell…"),
    "restarted": ("Win11 Shell is opnieuw gestart.", "Win11 Shell has been restarted."),
    "no_answer": ("Win11 Shell reageert niet; ik stop het en start het opnieuw.",
                  "Win11 Shell does not respond; stopping and starting it again."),
    "busy": ("Win11 Shell wordt al opnieuw gestart.", "Win11 Shell is already being restarted."),
    "starting_shell": ("Win11 Shell draaide niet; ik start het.", "Win11 Shell was not running; starting it."),
    "not_up": ("Win11 Shell is gestopt, maar nog niet terug. Wacht een paar seconden, of druk nog een keer op "
               "Ctrl+Shift+Windows-toets+B.",
               "Win11 Shell stopped but is not back yet. Wait a few seconds, or press Ctrl+Shift+Win+B again."),
    "cannot_stop": ("Win11 Shell liet zich niet stoppen.", "Win11 Shell could not be stopped."),
    "dry_run": ("Proefdraaimodus: Win11 Shell is niet opnieuw gestart (gebruik --force).",
                "Dry run: Win11 Shell was not restarted (use --force)."),
    "app_hung": ("Het venster van {app} reageerde niet meer; het wordt opnieuw geopend.",
                 "The {app} window stopped responding; opening it again."),
}


def _t(key: str, **kw) -> str:
    nl, en = MSG[key]
    return (en if _EN else nl).format(**kw)


class Usage(Exception):
    pass


def _call(method: str, args: dict | None = None, inst: dict | None = None):
    return call_instance(method, args or {}, inst=inst, timeout=4)


def _activation_args() -> dict:
    tok = os.environ.get("XDG_ACTIVATION_TOKEN") or os.environ.get("DESKTOP_STARTUP_ID")
    return {"activationToken": tok} if tok else {}


def _start_app_mode(app: str, param: str | None) -> None:
    """No instance running: start ``win11shell app`` detached."""
    from .util import user_env
    spec = app if not param else f"{app}:{param}"
    env = user_env()          # the user's own renderer switches (+ util.USER_GPU_MARK); the host decides its own
    env["PYTHONPATH"] = str(ROOT) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    python = "/usr/bin/python3" if os.path.exists("/usr/bin/python3") else sys.executable
    subprocess.Popen([python, "-m", "host", "--mode", "app", "--open", spec], cwd=str(ROOT), env=env,
                     stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                     start_new_session=True, close_fds=True)


# ---------------------------------------------------------------------------------------------------------
# /proc helpers: which process is our host?
# ---------------------------------------------------------------------------------------------------------

_CLK = os.sysconf("SC_CLK_TCK") if hasattr(os, "sysconf") else 100
HOST_MODES = ("layer", "preview")
HOST_FLAGS = ("--dry-run", "--dev", "-v", "--verbose", "--no-gtk")


def _argv(pid: int) -> list[str]:
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return [a.decode("utf-8", "replace") for a in f.read().split(b"\0") if a]
    except OSError:
        return []


def _stat(pid: int) -> tuple[str, str, int, int] | None:
    """(comm, state, ppid, starttime in clock ticks since boot)."""
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            data = f.read()
        rp = data.rfind(b")")
        f_ = data[rp + 2:].split()
        return data[data.find(b"(") + 1:rp].decode("utf-8", "replace"), f_[0].decode(), int(f_[1]), int(f_[19])
    except (OSError, ValueError, IndexError):
        return None


def _boot_time() -> float:
    try:
        with open("/proc/stat", encoding="ascii") as f:
            for line in f:
                if line.startswith("btime "):
                    return float(line.split()[1])
    except (OSError, ValueError):
        pass
    return 0.0


def _alive(pid: int, start: int | None = None) -> bool:
    """Still running (not a zombie) and — with ``start`` — still the same process (no pid reuse)."""
    st = _stat(pid)
    if st is None or st[1] in ("Z", "X"):
        return False
    return start is None or st[3] == start


def _argv_mode(argv: list[str]) -> str | None:
    for i, a in enumerate(argv):
        if a == "--mode" and i + 1 < len(argv):
            return argv[i + 1]
        if a.startswith("--mode="):
            return a.split("=", 1)[1]
    return None


def is_host_argv(argv: list[str], modes: tuple[str, ...] = HOST_MODES) -> bool:
    """``python3 -m host --mode layer|preview …`` (the desktop host, not ctl/appwindow/indexer). Pure (tested)."""
    if len(argv) < 3 or "python" not in os.path.basename(argv[0]):
        return False
    try:
        i = argv.index("-m")
    except ValueError:
        return False
    if i + 1 >= len(argv) or argv[i + 1] != "host":
        return False
    return (_argv_mode(argv) or "layer") in modes


def _listening_inodes(pid: int, port: int) -> set[str] | None:
    """Socket inodes LISTENing on ``port`` in ``pid``'s network namespace; None when that cannot be read."""
    found: set[str] = set()
    readable = False
    for name in ("tcp", "tcp6"):
        try:
            with open(f"/proc/{pid}/net/{name}", encoding="ascii", errors="replace") as f:
                lines = f.read().splitlines()[1:]
        except OSError:
            continue
        readable = True
        for ln in lines:
            parts = ln.split()
            try:
                if len(parts) > 9 and parts[3] == "0A" and int(parts[1].rsplit(":", 1)[1], 16) == port:
                    found.add(parts[9])
            except (ValueError, IndexError):
                continue
    return found if readable else None


def listens_on(pid: int, port) -> bool | None:
    """Does ``pid`` hold the listening socket of ``port``? None = cannot tell (no /proc/<pid>/net or fd access)."""
    try:
        port = int(port)
    except (TypeError, ValueError):
        return False
    inodes = _listening_inodes(pid, port)
    if inodes is None:
        return None
    if not inodes:
        return False
    try:
        fds = os.listdir(f"/proc/{pid}/fd")
    except OSError:
        return None
    for fd in fds:
        try:
            link = os.readlink(f"/proc/{pid}/fd/{fd}")
        except OSError:
            continue
        if link.startswith("socket:[") and link[8:-1] in inodes:
            return True
    return False


def host_process(inst: dict | None, modes: tuple[str, ...] = HOST_MODES) -> int | None:
    """The pid of the running desktop host the instance file names — only when that process is ours, alive, a
    ``python3 -m host --mode layer|preview`` and the very process that wrote the file: its start time (clock
    ticks since boot, kept across an in-place restart) equals the file's ``starttime`` — a reused pid is another
    process, and no wall-clock step can fool this. A file of an older host without ``starttime``: the process must
    hold the file's listening port (when that cannot be read: the old wall-clock rule). Anything else: None. This
    is the only process ``restart`` ever stops (besides ``supervised_host``)."""
    if not isinstance(inst, dict):
        return None
    try:
        pid = int(inst.get("pid"))
    except (TypeError, ValueError):
        return None
    if pid <= 1 or pid == os.getpid():
        return None
    try:
        if os.stat(f"/proc/{pid}").st_uid != os.getuid():
            return None
    except OSError:
        return None
    st = _stat(pid)
    if st is None or st[1] in ("Z", "X") or not is_host_argv(_argv(pid), modes):
        return None
    want = inst.get("starttime")
    if isinstance(want, int) and not isinstance(want, bool) and want > 0:
        return pid if st[3] == want else None
    listening = listens_on(pid, inst.get("port"))
    if listening is not None:
        return pid if listening else None
    started = inst.get("started")
    if isinstance(started, (int, float)) and started > 0:
        bt = _boot_time()
        if bt and bt + st[3] / _CLK > started + 5:
            return None
    return pid


def _children(pid: int) -> list[int]:
    out = []
    try:
        names = os.listdir("/proc")
    except OSError:
        return out
    for d in names:
        if d.isdigit():
            st = _stat(int(d))
            if st is not None and st[2] == pid:
                out.append(int(d))
    return out


def supervisor_pid() -> int | None:
    """The bin/win11shell supervisor of this runtime dir: a supervisor of this user with an open fd on
    ``<runtime>/supervisor.lock`` (the lock it holds). None when there is none."""
    try:
        lock = os.stat(runtime_dir() / "supervisor.lock")
    except OSError:
        return None
    try:
        names = os.listdir("/proc")
    except OSError:
        return None
    uid = os.getuid()
    for d in names:
        if not d.isdigit():
            continue
        pid = int(d)
        try:
            if os.stat(f"/proc/{pid}").st_uid != uid or not is_supervisor(pid):
                continue
            fds = os.listdir(f"/proc/{pid}/fd")
        except OSError:
            continue
        for fd in fds:
            try:
                st = os.stat(f"/proc/{pid}/fd/{fd}")
            except OSError:
                continue
            if (st.st_dev, st.st_ino) == (lock.st_dev, lock.st_ino):
                return pid
    return None


def supervised_host() -> int | None:
    """The desktop host the supervisor of this runtime dir runs (its child ``python3 -m host --mode
    layer|preview`` of this user) — the last resort when the instance file names no live desktop host."""
    sup = supervisor_pid()
    if sup is None:
        return None
    for pid in sorted(_children(sup)):
        try:
            if os.stat(f"/proc/{pid}").st_uid != os.getuid():
                continue
        except OSError:
            continue
        st = _stat(pid)
        if st is not None and st[1] not in ("Z", "X") and is_host_argv(_argv(pid)):
            return pid
    return None


def is_supervisor(pid: int) -> bool:
    """Is ``pid`` bin/win11shell's supervisor (``bash …/win11shell [layer|supervise] …``)?"""
    argv = _argv(pid)
    if not argv or os.path.basename(argv[0]) not in ("bash", "sh"):
        return False
    return any(os.path.basename(a) == "win11shell" for a in argv[1:3])


# ---------------------------------------------------------------------------------------------------------
# restart request (read by bin/win11shell's supervisor and by the stopping host, see host/services/shell.py)
# ---------------------------------------------------------------------------------------------------------

RESTART_REQUEST = "restart-request"
REQUEST_MAX_AGE = 120


def restart_request_path() -> Path:
    return runtime_dir() / RESTART_REQUEST


def write_restart_request(pid: int, port: int | None, token: str | None) -> bool:
    from .util import _private_dir, atomic_write_text
    if not _private_dir(runtime_dir(), create=True):
        return False
    lines = [f"pid={int(pid)}", f"time={int(time.time())}"]
    if port:
        lines.append(f"port={int(port)}")
    if token and all(c.isalnum() or c in "-_" for c in token):
        lines.append(f"token={token}")
    try:
        atomic_write_text(restart_request_path(), "\n".join(lines) + "\n", mode=0o600)
        return True
    except OSError:
        return False


def read_restart_request() -> dict | None:
    p = restart_request_path()
    try:
        st = os.lstat(p)
        if st.st_uid != os.getuid() or st.st_mode & 0o077:
            return None
        text = p.read_text(encoding="utf-8")
    except OSError:
        return None
    out: dict = {}
    for line in text.splitlines():
        k, _, v = line.partition("=")
        out[k.strip()] = v.strip()
    try:
        out["pid"] = int(out.get("pid", 0))
        out["time"] = int(out.get("time", 0))
    except ValueError:
        return None
    return out


def forced_restart_pending(pid: int | None = None) -> bool:
    """A fresh restart request for ``pid`` (default: this process) exists — the host that receives SIGTERM
    keeps its app windows open then, they reconnect to the new host."""
    req = read_restart_request()
    return bool(req and req["pid"] == (pid or os.getpid()) and time.time() - req["time"] < REQUEST_MAX_AGE)


def _remove_restart_request(pid: int) -> None:
    req = read_restart_request()
    if req and req["pid"] == pid:
        with contextlib.suppress(OSError):
            restart_request_path().unlink()


@contextlib.contextmanager
def _lock(name: str, create: bool = True):
    """Non-blocking lock in the runtime dir → yields True (we have it, or there is no such lock file and
    ``create`` is False) / False (someone else holds it)."""
    from .util import _private_dir
    fd = None
    try:
        if _private_dir(runtime_dir(), create=create):
            try:
                fd = os.open(runtime_dir() / name, os.O_RDWR | os.O_CLOEXEC | (os.O_CREAT if create else 0), 0o600)
            except FileNotFoundError:
                fd = None
            if fd is not None:
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except OSError:
                    os.close(fd)
                    fd = None
                    yield False
                    return
        yield True
    finally:
        if fd is not None:
            os.close(fd)


def supervisor_running() -> bool:
    """bin/win11shell's supervisor holds <runtime>/supervisor.lock while it runs (read only: no file is made)."""
    with _lock("supervisor.lock", create=False) as free:
        return not free


# ---------------------------------------------------------------------------------------------------------
# stopping and starting the shell
# ---------------------------------------------------------------------------------------------------------

TERM_WAIT = 3.0
KILL_WAIT = 3.0
UP_WAIT = 25.0
API_RESTART_TIMEOUT = 2.0


def _wait_gone(pid: int, start: int, timeout: float) -> bool:
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        if not _alive(pid, start):
            return True
        time.sleep(0.05)
    return not _alive(pid, start)


def terminate(pid: int, term_wait: float = TERM_WAIT, kill_wait: float = KILL_WAIT, kill=os.kill) -> bool:
    """SIGCONT (a stopped host can then act on what follows) + SIGTERM, SIGKILL after ``term_wait``. True when it
    is gone. Every signal goes to the same process only (start time checked each time)."""
    st = _stat(pid)
    if st is None:
        return True
    start = st[3]
    for sig in (signal.SIGCONT, signal.SIGTERM):
        if not _alive(pid, start):
            return True
        with contextlib.suppress(ProcessLookupError):
            kill(pid, sig)
    if _wait_gone(pid, start, term_wait):
        return True
    if _alive(pid, start):
        with contextlib.suppress(ProcessLookupError):
            kill(pid, signal.SIGKILL)
    return _wait_gone(pid, start, kill_wait)


def _clean_env() -> dict:
    """Environment for what ctl starts (the shell's supervisor, app windows): no tokens, and the renderer switches
    as the user had them (util.user_env: never one a GPU-forcing host left in a terminal ctl was started from — the
    new shell's apps would inherit it), with util.USER_GPU_MARK so the new processes know the user's values."""
    from .util import user_env
    env = user_env()
    for k in ("XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID", "WIN11SHELL_TOKEN"):
        env.pop(k, None)
    env["PYTHONPATH"] = str(ROOT) + (os.pathsep + env["PYTHONPATH"] if env.get("PYTHONPATH") else "")
    return env


_DETACHED: list = []                 # Popen objects of detached children (kept: no "still running" warning)


def _spawn_detached(cmd: list[str], env: dict, unit: str | None = None, log_path: Path | None = None):
    """Start ``cmd`` in its own session — in its own systemd --user scope when ``unit`` is given and that works
    (so a shell started from Taakbeheer's restart button does not live in Taakbeheer's scope)."""
    from .priority import systemd_run_scope
    out = subprocess.DEVNULL
    if log_path is not None:
        with contextlib.suppress(OSError):
            log_path.parent.mkdir(parents=True, exist_ok=True)
            if log_path.exists() and log_path.stat().st_size > 1_000_000:
                log_path.unlink()
            out = open(log_path, "ab")
    kw = dict(cwd=str(ROOT), env=env, stdin=subprocess.DEVNULL, stdout=out, stderr=out,
              start_new_session=True, close_fds=True)
    try:
        prefix = systemd_run_scope(unit) if unit else None
        if prefix:
            p = subprocess.Popen(prefix + cmd, **kw)
            try:
                rc = p.wait(1.5)
            except subprocess.TimeoutExpired:
                _DETACHED.append(p)
                return p                    # running in its scope
            if rc in (0, 2, 3, 4):
                return p                    # it ran and ended by itself (e.g. already running)
        p = subprocess.Popen(cmd, **kw)
        _DETACHED.append(p)
        return p
    finally:
        if out is not subprocess.DEVNULL:
            out.close()


def start_shell(flags: list[str] | None = None, port: int | None = None, token: str | None = None,
                env: dict | None = None) -> None:
    """``bin/win11shell`` (the supervisor), detached, in a scope of its own when possible."""
    cmd = [str(ROOT / "bin" / "win11shell"), "layer", *[f for f in (flags or []) if f in HOST_FLAGS]]
    if port:
        cmd += ["--port", str(int(port))]
    env = dict(env) if env is not None else _clean_env()
    env.pop("WIN11SHELL_TOKEN", None)
    if token:
        env["WIN11SHELL_TOKEN"] = token
    _spawn_detached(cmd, env, unit=f"win11shell-supervisor-{os.getpid()}-{int(time.time())}.scope")


def _rewritten(inst: dict, old: dict | None) -> bool:
    """Is ``inst`` another host image than ``old`` (another pid, or the same pid after an in-place restart)?"""
    old = old or {}
    if inst.get("pid") != old.get("pid"):
        return True
    if inst.get("runId") and old.get("runId"):
        return inst.get("runId") != old.get("runId")
    return (inst.get("started") or 0) > (old.get("started") or 0)


def _wait_up(old: dict | None, timeout: float) -> dict | None:
    """A (new) desktop host that answers: pid differs from ``old`` or its instance file was written again."""
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        inst = read_instance()
        if inst and inst.get("mode") in HOST_MODES and _rewritten(inst, old):
            try:
                call_instance("shell.info", {}, inst=inst, timeout=1.5)
                return inst
            except ApiError:
                pass
        time.sleep(0.25)
    return None


def _say(key: str, err: bool = False) -> None:
    print(_t(key), file=sys.stderr if err else sys.stdout, flush=True)


def restart(force: bool = False, wait: bool = True) -> int:
    """See the module doc. 0 = the shell is back (or will be: ``wait=False``), 1 = not (yet)."""
    with _lock("restart.lock") as mine:
        if not mine:
            _say("busy")
            return 0
        inst = read_instance(check_alive=False)
        pid = host_process(inst)
        port, token = (inst or {}).get("port"), (inst or {}).get("token")
        if pid is None:
            # the file names no live desktop host (an app-mode host overwrote it, or it is gone) while the
            # supervisor still runs one — maybe a hung one: that is the process to stop
            alt = supervised_host()
            if alt is not None:
                pid, force = alt, True
                if not inst or inst.get("pid") != alt:
                    port = token = None           # unknown: the supervisor starts it on a new port + token
        if pid and not force:
            try:
                res = call_instance("shell.restart", {}, inst=inst, timeout=API_RESTART_TIMEOUT)
                if isinstance(res, dict) and res.get("dryRun"):
                    _say("dry_run")
                    return 0
                _say("restarting")
                # in-place restart (exec, same pid): the new process writes the instance file again
                if _wait_up(inst, 10.0):
                    _say("restarted")
                    return 0
            except ApiError as e:
                if e.code not in ("not_running", "shutting_down"):
                    raise
            pid = host_process(inst)            # still the same, stuck process?
            if pid:
                _say("no_answer")
        elif pid:
            _say("restarting")
        if pid is None:
            app_host = host_process(inst, modes=("app",))
            port = token = None
            if inst and not app_host and not _alive(int(inst.get("pid") or 0)) and \
                    inst.get("mode") in HOST_MODES:
                port, token = inst.get("port"), inst.get("token")     # crashed host: keep its port + token
            if supervisor_running():
                _say("restarting")
            else:
                _say("starting_shell")
                start_shell(["--dry-run"] if (inst or {}).get("dryRun") else [], port, token)
            if not wait:
                return 0
            if _wait_up(inst, UP_WAIT):
                _say("restarted")
                return 0
            _say("not_up", err=True)
            return 1
        argv = _argv(pid)
        parent = (_stat(pid) or ("", "", 0, 0))[2]
        supervised = parent > 1 and is_supervisor(parent)
        write_restart_request(pid, port, token)
        from .lifeguard import helper_children
        from .surfaces import webkit_snapshot   # imports no Gtk
        webkit = webkit_snapshot(pid)           # a hung web process outlives its host (it never notices)
        # long-lived helpers outlive a killed host too (SIGKILL skips stop_services): night light, clipboard
        # watcher, PipeWire monitor, indexer
        webkit.update(helper_children(pid))
        if not terminate(pid):
            _remove_restart_request(pid)
            _say("cannot_stop", err=True)
            return 1
        if supervised:
            # an older supervisor ends on exit code 0 instead of restarting: then start one ourselves
            end = time.monotonic() + 2.0
            while time.monotonic() < end and _alive(parent) and not _wait_up(inst, 0.3):
                pass
            if not _alive(parent):
                supervised = False
        if not supervised:
            _remove_restart_request(pid)
            start_shell([a for a in argv if a in HOST_FLAGS], port, token)
        from .surfaces import kill_webkit_leftovers
        kill_webkit_leftovers(webkit)
        if not wait:
            return 0
        if _wait_up(inst, UP_WAIT):
            _say("restarted")
            return 0
        _say("not_up", err=True)
        return 1


# ---------------------------------------------------------------------------------------------------------
# opening our apps without the host (Ctrl+Shift+Esc must always work)
# ---------------------------------------------------------------------------------------------------------

def _hypr_window(app: str) -> dict | None:
    from .util import compositor, hyprctl
    if compositor() != "hyprland":
        return None
    for c in hyprctl("clients", json_out=True, timeout=1.0) or []:
        if isinstance(c, dict) and c.get("class") == f"win11shell-{app}" and c.get("mapped", True):
            return c
    return None


def focus_hypr_window(c: dict) -> bool:
    """Show and focus a window through Hyprland only (also from the minimized special workspace)."""
    from .util import hyprctl
    addr = c.get("address")
    if not addr:
        return False
    ws = c.get("workspace") or {}
    if str(ws.get("name", "")).startswith("special:"):
        aw = hyprctl("activeworkspace", json_out=True, timeout=1.0) or {}
        if isinstance(aw, dict) and aw.get("id") is not None:
            hyprctl(f"dispatch movetoworkspacesilent {aw['id']},address:{addr}", timeout=1.0)
    ok = (hyprctl(f"dispatch focuswindow address:{addr}", timeout=1.0) or "").strip() == "ok"
    hyprctl(f"dispatch alterzorder top,address:{addr}", timeout=1.0)
    return ok


def _cache_dir() -> Path:
    v = os.environ.get("WIN11SHELL_CACHE_DIR")
    if v:
        return Path(os.path.expanduser(v))
    return Path(os.environ.get("XDG_CACHE_HOME") or Path.home() / ".cache") / "win11shell"


def spawn_appwindow(app: str, params: dict | None, inst: dict) -> None:
    """The app window process itself, on the instance's port/token (the token travels in the environment)."""
    python = "/usr/bin/python3" if os.path.exists("/usr/bin/python3") else sys.executable
    cmd = [python, "-m", "host.appwindow", "--app", app, "--port", str(int(inst["port"]))]
    if params:
        cmd += ["--params", json.dumps(params, ensure_ascii=False)]
    if inst.get("dryRun"):
        cmd.append("--dry-run")
    env = _clean_env()
    env["WIN11SHELL_TOKEN"] = str(inst["token"])
    for k in ("XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"):
        if os.environ.get(k):
            env[k] = os.environ[k]
    _spawn_detached(cmd, env, log_path=_cache_dir() / f"appwindow-{app}.log")


def _percent(value) -> tuple[float, bool]:
    """→ (percent 0..100, was_fraction). Accepts 0..1 floats and 0..100 numbers."""
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return 0.0, False
    if isinstance(value, float) and value <= 1.0:
        return value * 100.0, True
    return float(value), False


def _osd(kind: str, direction: str, inst: dict | None) -> None:
    if kind not in ("volume", "brightness") or direction not in ("up", "down", "mute") or \
            (kind == "brightness" and direction == "mute"):
        raise Usage()
    if inst is None:
        _osd_fallback(kind, direction)
        return
    getter, setter = ("system.audio.get", "system.audio.set") if kind == "volume" else \
        ("system.brightness.get", "system.brightness.set")
    try:
        cur = _call(getter, {}, inst) or {}
    except ApiError as e:
        if e.code != "unknown_method":
            raise
        cur = {"available": False}
    osd: dict = {"kind": kind}
    if cur.get("available", True) is not False:
        raw = cur.get("volume") if kind == "volume" else cur.get("value")
        pct, frac = _percent(raw)
        muted = bool(cur.get("muted"))
        args: dict = {}
        if direction == "mute":
            muted = not muted
            args["muted"] = muted
        else:
            pct = max(0.0, min(100.0, round(pct / STEP) * STEP + (STEP if direction == "up" else -STEP)))
            value = pct / 100.0 if frac else int(pct)
            args["volume" if kind == "volume" else "value"] = value
            if kind == "volume" and direction == "up" and muted:
                muted = False
                args["muted"] = False
        _call(setter, args, inst)
        osd["value"] = int(round(pct))
        if kind == "volume":
            osd["muted"] = muted
    else:
        osd["available"] = False
    _call("shell.command", {"command": "osd", "args": osd}, inst)


def _osd_fallback(kind: str, direction: str) -> None:
    """Shell not running: still let the keys work (no OSD)."""
    if kind == "volume" and which("wpctl"):
        cmd = ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"] if direction == "mute" else \
            ["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", f"{STEP}%{'+' if direction == 'up' else '-'}"]
    elif kind == "brightness" and which("brightnessctl"):
        cmd = ["brightnessctl", "-q", "set", f"{STEP}%{'+' if direction == 'up' else '-'}"]
    else:
        raise ApiError("not_running", _t("not_running"))
    r = run(cmd, timeout=3)
    if not r.ok:
        raise ApiError("failed", _t("failed", x=(r.err or r.out).strip()[:200] or cmd[0]))


def _app_params(app: str, param: str | None) -> dict:
    params: dict = {}
    if param:
        if param.lstrip().startswith("{"):
            try:
                parsed = json.loads(param)
                if isinstance(parsed, dict):
                    params = parsed
            except ValueError:
                params = {}
        if not params:
            if app == "explorer":
                param = os.path.abspath(os.path.expanduser(param)) if not param.startswith("/") else param
            params = {DEFAULT_PARAM[app]: param}
    return params


APP_HUNG_S = 12.0          # an app window's main loop did not touch its lock file this long → hung
APP_RECHECK_S = 2.5        # … and still did not while we watched (> 2 heartbeats): not a clock step or a resume
APP_START_HUNG_S = 60.0    # a window still starting up (marker "start") after this long (process age) → hung


def _lock_state(path: Path) -> tuple[int, str, float] | None:
    """(pid, marker, mtime) of an app lock file of ours, None when unreadable."""
    try:
        st = path.stat()
        if st.st_uid != os.getuid():
            return None
        words = path.read_text(encoding="ascii", errors="replace").split()
        return (int(words[0]) if words else 0), (words[1] if len(words) >= 2 else ""), st.st_mtime
    except (OSError, ValueError):
        return None


def _age(pid: int) -> float | None:
    """Seconds since ``pid`` started (boot-relative: no wall clock)."""
    st = _stat(pid)
    try:
        with open("/proc/uptime", encoding="ascii") as f:
            up = float(f.read().split()[0])
    except (OSError, ValueError, IndexError):
        return None
    return None if st is None else up - st[3] / _CLK


def hung_window(app: str, now: float | None = None, recheck: float = APP_RECHECK_S,
                sleep=time.sleep) -> int | None:
    """pid of our window process for ``app`` whose main loop stopped turning: it holds ``<runtime>/app-<app>.lock``,
    is this user's ``python3 -m host.appwindow --app <app>``, and
    * content ``<pid> hb`` (its main loop runs and touches the file every HEARTBEAT_S): the file was not touched for
      APP_HUNG_S, and still not during ``recheck`` s — a forward clock step or a resume from suspend makes a healthy
      window's file look old for a moment (the mtime is wall-clock time, its timer is monotonic);
    * content ``<pid> start`` (still starting: GTK, WebKit, its scope — slow under memory pressure, exactly when
      Taakbeheer is needed): only when the process is older than APP_START_HUNG_S.
    A file without a marker (an older version) is never judged. None otherwise."""
    path = runtime_dir() / f"app-{app}.lock"
    state = _lock_state(path)
    if state is None:
        return None
    pid, marker, mtime = state
    if pid <= 1 or marker not in ("hb", "start"):
        return None
    if marker == "hb" and ((now or time.time()) - mtime) < APP_HUNG_S:
        return None
    if marker == "start":
        age = _age(pid)
        if age is None or age < APP_START_HUNG_S:
            return None
    argv = _argv(pid)
    if "host.appwindow" not in argv or "--app" not in argv or argv[argv.index("--app") + 1:][:1] != [app]:
        return None
    try:
        if os.stat(f"/proc/{pid}").st_uid != os.getuid():
            return None
    except OSError:
        return None
    try:   # the lock is really held (a dead holder's file stays behind)
        fd = os.open(path, os.O_RDONLY | os.O_CLOEXEC)
    except OSError:
        return None
    try:
        fcntl.flock(fd, fcntl.LOCK_SH | fcntl.LOCK_NB)
        fcntl.flock(fd, fcntl.LOCK_UN)
        return None                              # nobody holds it
    except OSError:
        pass
    finally:
        os.close(fd)
    if recheck > 0:
        # a healthy main loop touches the file within HEARTBEAT_S: watch it a little before judging
        end = time.monotonic() + recheck
        while time.monotonic() < end:
            sleep(0.1)
            if _lock_state(path) != state:
                return None
    return pid


def replace_hung_window(app: str) -> bool:
    """Opening ``app`` while its window process hangs: end that process (and its WebKit processes) so a fresh window
    can start — instead of focusing a frozen one. True when one was ended."""
    pid = hung_window(app)
    if pid is None:
        return False
    from .surfaces import kill_webkit_leftovers, webkit_snapshot
    webkit = webkit_snapshot(pid)
    print(_t("app_hung", app=app), file=sys.stderr, flush=True)
    ok = terminate(pid, term_wait=1.0)
    kill_webkit_leftovers(webkit, grace=0.5)
    return ok


def _app(app: str, param: str | None, inst: dict | None, direct: bool = False) -> None:
    params = _app_params(app, param)
    replace_hung_window(app)
    if direct:
        c = _hypr_window(app)
        if c is not None and focus_hypr_window(c):
            return
    if inst is not None:
        try:
            call_instance("shell.openApp", {"app": app, "params": params, **_activation_args()}, inst=inst,
                          timeout=0.8 if direct else 4)
            return
        except ApiError as e:
            if e.code not in ("not_running", "shutting_down"):
                raise
        if inst.get("mode") != "serve" and host_process(inst, modes=("layer", "preview", "app")):
            # the host runs but does not answer: the window process works without it (and says so)
            spawn_appwindow(app, params, inst)
            return
    _start_app_mode(app, json.dumps(params, ensure_ascii=False) if params else None)


def _print_problem() -> None:
    """The host stopped because something essential is missing (exit 5, <runtime>/problem.json): say what."""
    try:
        from .problem import read_problem
        p = read_problem()
    except Exception:  # noqa: BLE001
        p = None
    if p and p.get("command"):
        print(f"Er ontbreekt iets / missing: {' '.join(p.get('packages') or [])}. Installeer: {p['command']}")


def dispatch(argv: list[str]) -> int:
    if not argv or argv[0] in ("-h", "--help", "help"):
        print(__doc__.strip())
        return 0 if argv else 2
    cmd, rest = argv[0], argv[1:]
    inst = read_instance()

    if cmd in APP_IDS:
        direct = "--direct" in rest
        rest = [r for r in rest if r != "--direct"]
        if len(rest) > 1:
            rest = [" ".join(rest)]
        _app(cmd, rest[0] if rest else None, inst, direct=direct)
        return 0
    if cmd == "restart":
        if any(r not in ("--force", "-f", "--no-wait") for r in rest):
            raise Usage()
        return restart(force=bool({"--force", "-f"} & set(rest)), wait="--no-wait" not in rest)
    if cmd == "osd":
        if len(rest) != 2:
            raise Usage()
        _osd(rest[0], rest[1], inst)
        return 0
    if cmd == "lock":
        if inst is None:
            r = run(["loginctl", "lock-session"], timeout=3)
            if not r.ok:
                raise ApiError("failed", _t("failed", x=r.err.strip()[:200]))
            return 0
        _call("system.power.action", {"action": "lock"}, inst)
        return 0

    if inst is None:
        if cmd == "status":
            print(_t("not_running"))
            _print_problem()
            return 1
        raise ApiError("not_running", _t("not_running"))

    if cmd in SHELL_COMMANDS:
        args: dict = {}
        if cmd == "alt-tab" and len(rest) == 1 and rest[0] in REVERSE_WORDS:
            args = {"reverse": True}
        elif rest:
            raise Usage()
        _call("shell.command", {"command": cmd, "args": args}, inst)
    elif cmd == "snap":
        if len(rest) != 1 or rest[0] not in SNAP:
            raise Usage()
        _call("windows.snap", {"edge": SNAP[rest[0]]}, inst)
    elif cmd == "power":
        if len(rest) != 1 or rest[0] not in POWER_ACTIONS:
            raise Usage()
        _call("shell.command", {"command": "power", "args": {"action": rest[0]}}, inst)
    elif cmd in ("info", "status"):
        print(json.dumps(_call("shell.info", {}, inst), ensure_ascii=False, indent=2))
    elif cmd == "quit":
        _call("shell.quit", {}, inst)
    elif cmd == "call":
        if not rest:
            raise Usage()
        args = {}
        if len(rest) > 1:
            try:
                args = json.loads(rest[1])
            except ValueError:
                raise Usage() from None
            if not isinstance(args, dict):
                raise Usage()
        print(json.dumps(_call(rest[0], args, inst), ensure_ascii=False, indent=2))
    else:
        print(_t("unknown", x=cmd), file=sys.stderr)
        return 2
    return 0


def main(argv: list[str] | None = None) -> int:
    argv = sys.argv[1:] if argv is None else argv
    try:
        return dispatch(argv)
    except Usage:
        print(_t("usage"), file=sys.stderr)
        return 2
    except ApiError as e:
        print(e.message if e.code in ("not_running", "failed") else _t("failed", x=e.message), file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
