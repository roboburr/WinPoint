"""Shared helpers for the Win11 Shell host.

Everything here is stdlib-only; PyGObject (Gio/GLib/GdkPixbuf) is imported lazily
and every helper degrades gracefully when it is missing.

Sections:
  * ApiError, logging
  * dry-run state + ``@dangerous`` guard
  * process helpers: run(), run_ok(), which(), spawn()
  * JSON + atomic file writes
  * dict helpers: deep_merge(), get_path(), set_path(), del_path(), diff_paths()
  * XDG dirs, user info, compositor detection
  * Hyprland IPC: hyprctl(), hypr_dispatch()
  * D-Bus helpers (Gio): session_bus(), system_bus(), dbus_call(), dbus_get_property()
  * instance file + call_instance() (HTTP client to a running host)
  * image sniffing: image_mime()
  * freedesktop icon-theme lookup that works without a display: find_icon() ...
"""

from __future__ import annotations

import collections
import contextlib
import contextvars
import copy
import functools
import hashlib
import http.client
import json
import logging
import math
import os
import pwd
import re
import shutil
import socket
import stat
import subprocess
import threading
import time
from pathlib import Path
from typing import Any, Callable, Iterable, NamedTuple

from . import APP_IDS, VERSION

log = logging.getLogger("win11shell.util")

__all__ = [
    "ApiError", "VERSION", "APP_IDS",
    "set_dry_run", "is_dry_run", "dry_for", "dry_scope", "current_dry_scope", "dangerous", "dry_run_result", "test_guard_blocks", "redact",
    "RunResult", "run", "run_ok", "which", "spawn", "GPU_ENV_MARK", "undo_gpu_env", "user_env",
    "USER_GPU_KEYS", "USER_GPU_MARK", "user_gpu_values", "remember_user_gpu_env", "apply_user_gpu_env",
    "json_dumps", "json_loads", "read_json", "atomic_write_bytes", "atomic_write_text", "atomic_write_json",
    "deep_merge", "split_path", "get_path", "set_path", "del_path", "diff_paths",
    "home", "xdg_config_home", "xdg_cache_home", "xdg_data_home", "xdg_runtime_dir", "xdg_user_dir",
    "runtime_dir", "user_info", "compositor", "is_wayland",
    "hypr_socket_path", "hyprctl", "hypr_dispatch",
    "glib_signal_add", "session_bus", "system_bus", "dbus_call", "dbus_get_property", "dbus_name_has_owner",
    "instance_path", "pid_alive", "proc_starttime", "read_instance", "write_instance", "remove_instance", "call_instance",
    "own_app", "image_mime", "BROWSER_IMAGE_MIMES",
    "find_icon", "find_icon_any", "find_mime_icon", "mime_icon_names", "icon_exists", "first_existing_icon",
    "current_icon_theme", "ICONS",
]


# ---------------------------------------------------------------------------
# Errors
# ---------------------------------------------------------------------------

class ApiError(Exception):
    """Raise from an API handler to answer ``{"ok": false, "error": {code, message}}``.

    ``message`` must be short, friendly Dutch (shown to the user as-is).
    """

    def __init__(self, code: str, message: str = "", data: Any = None):
        self.code = str(code or "error")
        self.message = message or "Er ging iets mis."
        self.data = data
        super().__init__(f"{self.code}: {self.message}")

    def to_dict(self) -> dict:
        d = {"code": self.code, "message": self.message}
        if self.data is not None:
            d["data"] = self.data
        return d


# ---------------------------------------------------------------------------
# Dry-run guard
# ---------------------------------------------------------------------------

# Dry-run is NOT one mutable process-wide switch (a later Ctx(dry_run=False) once turned another
# registry's guard off and dangerous test calls ran for real). It lives in three places:
#
#   * Api / Ctx own flag  – per registry. ``Api.call`` checks its OWN flag (see host/api.py).
#   * the call scope      – a ContextVar set by ``Api._call`` for the duration of a handler (and
#                           carried into ``ctx.thread`` / ``ctx.run_on_main``), so ``@dangerous``
#                           helpers and ``dry_for`` see the flag of the registry whose call they
#                           serve, per thread.
#   * the process default – only for code that runs outside any call (pollers, GLib callbacks).
#                           A one-way latch: any dry Ctx/Api turns it on and nothing but
#                           ``set_dry_run(False, force=True)`` turns it off again.

_dry_default = False
_dry_scope: contextvars.ContextVar = contextvars.ContextVar("win11shell_dry_run", default=None)


def set_dry_run(value: bool, *, force: bool = False) -> None:
    """Process *default* for code that runs outside any API call (no Api/Ctx in scope).

    One-way latch: ``True`` sticks. ``False`` is ignored (with a warning) once dry-run was on,
    unless ``force=True`` – so creating a non-dry context can never disarm a dry one."""
    global _dry_default
    if value:
        _dry_default = True
    elif force or not _dry_default:
        _dry_default = False
    else:
        log.warning("set_dry_run(False) genegeerd: dry-run blijft aan (ignored; use force=True)")


def current_dry_scope() -> bool | None:
    """Dry-run flag of the API call running in this thread/context, or None outside a call."""
    return _dry_scope.get()


@contextlib.contextmanager
def dry_scope(value: bool):
    """Run a block with the dry-run flag of a given registry/context (nested calls, helpers)."""
    token = _dry_scope.set(bool(value))
    try:
        yield
    finally:
        _dry_scope.reset(token)


def is_dry_run() -> bool:
    """Effective dry-run for the code running now: the flag of the API call in progress in this
    thread (set by ``Api._call``), else the process default."""
    v = _dry_scope.get()
    return _dry_default if v is None else bool(v)


def dry_for(ctx: Any = None) -> bool:
    """Effective dry-run for code that knows its ``ctx``: the context's own flag, OR a dry API
    call in progress (a non-dry context called from a dry call stays dry). Without a ctx:
    ``is_dry_run()``. Use this inside handlers instead of ``is_dry_run()``."""
    if ctx is not None and hasattr(ctx, "dry_run"):
        return bool(ctx.dry_run) or bool(_dry_scope.get())
    return is_dry_run()


# Methods that change the machine itself (not files in a sandbox). Inside a unit-test process
# they are always treated as dry-run unless WIN11SHELL_ALLOW_REAL=1: a forgotten dry-run flag in a
# test can then never reboot the PC, switch off the radios, change the volume or empty caches.
_MACHINE_METHODS = (
    "system.power.", "system.network.", "system.bluetooth.", "system.audio.set", "system.brightness.set",
    "system.nightLight.set", "system.openUpdater", "storage.cleanup.run", "sysmon.endTask",
    "sysmon.setEfficiency", "sysmon.setStartup", "sysmon.serviceAction", "sysmon.restartProcess", "shell.restart",
    "apps.setDefault", "settings.syncSystemTheme", "display.set",
    "disks.mount", "disks.unmount", "disks.setLabel", "disks.format", "disks.deletePartition", "disks.createPartition",
    "disks.initialize", "disks.eject", "disks.powerOff", "disks.unlock", "disks.lock",
)


def _under_unittest() -> bool:
    import sys
    if "unittest" not in sys.modules:
        return False
    main = sys.modules.get("__main__")
    spec = getattr(main, "__spec__", None)
    name = getattr(spec, "name", "") or ""
    return not name.startswith("host")      # `python3 -m host` itself never counts as a test run


def test_guard_blocks(method: str) -> bool:
    """True when ``method`` must not run for real because this process is a unit-test run."""
    if not method.startswith(_MACHINE_METHODS):
        return False
    if os.environ.get("WIN11SHELL_ALLOW_REAL") == "1":
        return False
    return _under_unittest()


_SECRET_KEY = re.compile(r"pass|psk|secret|token|api_?key|private_?key|^key$|auth|cookie|credential", re.I)


def redact(obj: Any, depth: int = 0) -> Any:
    """Copy of ``obj`` with secret-looking values (passwords, tokens, keys) replaced by '***' (for logs)."""
    if depth > 6:
        return "…"
    if isinstance(obj, dict):
        return {k: ("***" if isinstance(k, str) and _SECRET_KEY.search(k) and v not in (None, "", False)
                    else redact(v, depth + 1)) for k, v in obj.items()}
    if isinstance(obj, (list, tuple)):
        return [redact(v, depth + 1) for v in obj]
    return obj


def dry_run_result(extra: Any = None) -> dict:
    """``{"ok": true, "dryRun": true}`` merged over an optional dict of extra fields."""
    result = dict(extra) if isinstance(extra, dict) else {}
    result["ok"] = True
    result["dryRun"] = True
    return result


def _short_repr(args: tuple, kwargs: dict, limit: int = 300) -> str:
    try:
        parts = [repr(redact(a)) for a in args] + [f"{k}={'***' if _SECRET_KEY.search(k) else repr(redact(v))}"
                                                   for k, v in kwargs.items()]
        text = "(" + ", ".join(parts) + ")"
    except Exception:  # pragma: no cover - repr failure
        text = "(...)"
    return text if len(text) <= limit else text[: limit - 4] + "...)"


def dangerous(fn: Callable | None = None, *, result: Any = None, name: str | None = None):
    """Guard a helper that changes the system.

    In dry-run the wrapped function is NOT called: the call is logged and
    ``{"ok": True, "dryRun": True}`` is returned (merged with ``result``, which may be
    a dict or a callable receiving the same arguments and returning a dict). "Dry-run" is
    ``is_dry_run()``: the flag of the API call (registry) this helper runs for, else the
    process default — never a flag another context overwrote. Unit-test processes also
    refuse machine-level helpers (``test_guard_blocks``).

        @dangerous
        def power_off(): ...

        @dangerous(result=lambda paths: {"failed": []})
        def trash(paths): ...
    """

    def deco(f: Callable) -> Callable:
        label = name or getattr(f, "__qualname__", repr(f))

        @functools.wraps(f)
        def wrapper(*args, **kwargs):
            if is_dry_run() or test_guard_blocks(label):
                log.info("[dry-run] %s%s overgeslagen (skipped)", label, _short_repr(args, kwargs))
                extra = result(*args, **kwargs) if callable(result) else copy.deepcopy(result)
                return dry_run_result(extra)
            return f(*args, **kwargs)

        wrapper.dangerous = True  # type: ignore[attr-defined]
        return wrapper

    if fn is not None and callable(fn):
        return deco(fn)
    return deco


# ---------------------------------------------------------------------------
# Processes
# ---------------------------------------------------------------------------

class RunResult(NamedTuple):
    rc: int
    out: str
    err: str

    @property
    def ok(self) -> bool:
        return self.rc == 0


def run(cmd: list[str] | tuple[str, ...], timeout: float = 5.0, input: str | None = None,
        env: dict | None = None, cwd: str | os.PathLike | None = None) -> RunResult:
    """Run a command, never raise. rc 127 = not found, 124 = timeout, 126 = OS error."""
    try:
        p = subprocess.run(list(cmd), capture_output=True, timeout=timeout, input=input, env=env,
                           cwd=cwd, text=True, errors="replace", stdin=None if input is not None else subprocess.DEVNULL)
        return RunResult(p.returncode, p.stdout, p.stderr)
    except FileNotFoundError:
        return RunResult(127, "", f"not found: {cmd[0] if cmd else ''}")
    except subprocess.TimeoutExpired:
        return RunResult(124, "", "timeout")
    except OSError as e:
        return RunResult(126, "", str(e))


def run_ok(cmd: list[str] | tuple[str, ...], timeout: float = 5.0, **kw) -> str | None:
    """stdout of a successful command, else None."""
    r = run(cmd, timeout=timeout, **kw)
    return r.out if r.rc == 0 else None


_which_cache: dict[str, tuple[float, str | None]] = {}


def which(name: str) -> str | None:
    """shutil.which with a 60 s cache."""
    now = time.monotonic()
    hit = _which_cache.get(name)
    if hit and now - hit[0] < 60:
        return hit[1]
    path = shutil.which(name)
    _which_cache[name] = (now, path)
    return path


# The WebKit GPU switches host/surfaces.py may set or remove in our own environment (SPEC §8.1). What it changed
# is written down in WIN11SHELL_GPU_ENV ({"set": [keys], "user": {key: original value}}; undo_gpu_env). Hosts of
# before 2026-10-27 relied on it alone; now the user's own values come from USER_GPU_MARK (below), which also
# covers switches an older image set without writing them down.
GPU_ENV_MARK = "WIN11SHELL_GPU_ENV"
GPU_ENV_KEYS = ("WEBKIT_FORCE_DMABUF_RENDERER", "WEBKIT_DMABUF_RENDERER_FORCE_SHM", "WEBKIT_SKIA_ENABLE_CPU_RENDERING",
                "__NV_DISABLE_EXPLICIT_SYNC")
# Programs started FOR the user get these renderer switches only as the user had them before the first Win11 Shell
# process of the session started — never a value our GPU code set (surfaces' WIN11SHELL_GPU=safe, Taakbeheer's
# software rendering) nor one an older host left in its own environment: hosts from before GPU_ENV_MARK carried
# WEBKIT_FORCE_DMABUF_RENDERER=1 / __NV_DISABLE_EXPLICIT_SYNC=1 through every in-place restart into everything they
# launched (Claude Desktop and terminals still had them on 2026-10-27, and so did the live host itself). The user's
# own values are worked out once (user_gpu_values) and travel in USER_GPU_MARK to in-place restarts and to every
# process of ours, which decide their own rendering afresh from them (surfaces._prefer_gpu_renderer, appwindow).
USER_GPU_KEYS = GPU_ENV_KEYS + ("WEBKIT_DISABLE_DMABUF_RENDERER", "WEBKIT_DISABLE_COMPOSITING_MODE", "GSK_RENDERER")
USER_GPU_MARK = "WIN11SHELL_USER_GPU"


def undo_gpu_env(env) -> Any:
    """Undo, in ``env`` (a dict or os.environ), the GPU switches a Win11 Shell host set or removed; drops the
    marker. Also understands the marker before 2026-10-27 (``WIN11SHELL_GPU_FORCED=1``: the host had set
    WEBKIT_FORCE_DMABUF_RENDERER and __NV_DISABLE_EXPLICIT_SYNC). Returns ``env``. Pure (tested)."""
    raw = env.pop(GPU_ENV_MARK, None)
    if raw:
        try:
            d = json.loads(raw)
        except ValueError:
            d = {}
        if not isinstance(d, dict):
            d = {}
        for k in d.get("set") or []:
            if k in GPU_ENV_KEYS:
                env.pop(k, None)
        user = d.get("user") or {}
        if isinstance(user, dict):
            for k, v in user.items():
                if k in GPU_ENV_KEYS and isinstance(v, str):
                    env[k] = v
    if env.pop("WIN11SHELL_GPU_FORCED", None) == "1":
        env.pop("WEBKIT_FORCE_DMABUF_RENDERER", None)
        env.pop("__NV_DISABLE_EXPLICIT_SYNC", None)
    return env


def _environ_of(pid: int) -> dict[str, str] | None:
    try:
        with open(f"/proc/{pid}/environ", "rb") as f:
            raw = f.read()
    except OSError:
        return None
    out = {}
    for item in raw.split(b"\0"):
        k, sep, v = item.partition(b"=")
        if sep and k:
            out[k.decode("utf-8", "surrogateescape")] = v.decode("utf-8", "surrogateescape")
    return out


def supervisor_env(ppid: int | None = None) -> dict[str, str] | None:
    """The environment of our parent when it is ``bin/win11shell`` (the supervisor, bash, this user), else None.
    The supervisor never touches the renderer switches, so for a host it runs this is what the session gave —
    also after in-place restarts (exec) of an older host that had changed its own environment."""
    ppid = os.getppid() if ppid is None else ppid
    if ppid <= 1:
        return None
    try:
        if os.stat(f"/proc/{ppid}").st_uid != os.getuid():
            return None
        with open(f"/proc/{ppid}/cmdline", "rb") as f:
            argv = [a.decode("utf-8", "replace") for a in f.read().split(b"\0") if a]
    except OSError:
        return None
    if not argv or os.path.basename(argv[0]) not in ("bash", "sh") or \
            not any(os.path.basename(a) == "win11shell" for a in argv[1:3]):
        return None
    return _environ_of(ppid)


def user_gpu_values(env=None, supervisor: Callable[[], dict | None] | None = None) -> dict[str, str]:
    """The renderer switches (USER_GPU_KEYS) as the user had them before the first Win11 Shell process of this
    session started: USER_GPU_MARK when a process of ours passed it on; else, for a host under the supervisor, the
    supervisor's environment; else our own environment as we got it (minus what GPU_ENV_MARK says a host
    changed). Pure apart from /proc (``supervisor`` injectable; tested)."""
    env = os.environ if env is None else env
    raw = env.get(USER_GPU_MARK)
    if raw:
        try:
            d = json.loads(raw)
        except ValueError:
            d = None
        if isinstance(d, dict):
            return {k: v for k, v in d.items() if k in USER_GPU_KEYS and isinstance(v, str)}
    ref = (supervisor or supervisor_env)()
    ref = undo_gpu_env(dict(env if ref is None else ref))
    return {k: ref[k] for k in USER_GPU_KEYS if isinstance(ref.get(k), str)}


def _user_gpu_mark(values: dict) -> str:
    return json.dumps(values, sort_keys=True, separators=(",", ":"))


def remember_user_gpu_env(env=None) -> dict[str, str]:
    """Once per process, before it changes any renderer switch: the user's own values (user_gpu_values), kept in
    USER_GPU_MARK so an in-place restart and every process we start still know them. → those values."""
    env = os.environ if env is None else env
    values = user_gpu_values(env)
    env[USER_GPU_MARK] = _user_gpu_mark(values)
    return values


def apply_user_gpu_env(env, user: dict | None = None) -> Any:
    """Set the renderer switches in ``env`` (a dict or os.environ) to the user's own values (``user``, default
    user_gpu_values()): whatever a host set, removed or leaked is gone; our record of changes (GPU_ENV_MARK, the
    legacy marker) too. Returns ``env``."""
    user = user_gpu_values() if user is None else user
    env.pop(GPU_ENV_MARK, None)
    env.pop("WIN11SHELL_GPU_FORCED", None)
    for k in USER_GPU_KEYS:
        if k in user:
            env[k] = user[k]
        else:
            env.pop(k, None)
    return env


def user_env(base: dict | None = None, drop_private: bool = False) -> dict:
    """A copy of our environment — or of ``base``, e.g. a process's own environment for sysmon.restartProcess —
    for a program started for the user: the renderer switches exactly as the user had them before Win11 Shell
    started (user_gpu_values of THIS process; never one our GPU code set or an older host leaked) and, with
    ``drop_private``, without our WIN11SHELL_* variables (token, test overrides). Without it, USER_GPU_MARK is
    set (our own processes started with it, e.g. by ctl, keep knowing the user's values)."""
    env = dict(os.environ if base is None else base)
    user = user_gpu_values()
    apply_user_gpu_env(env, user)
    if drop_private:
        env = {k: v for k, v in env.items() if not k.startswith("WIN11SHELL_")}
    else:
        env[USER_GPU_MARK] = _user_gpu_mark(user)
    return env


def spawn(cmd: list[str], env: dict | None = None, cwd: str | os.PathLike | None = None,
          detach: bool = True, quiet: bool = True) -> subprocess.Popen | None:
    """Start a process without waiting (NOT guarded: decorate the caller with @dangerous). Without ``env``: our
    environment with the user's own renderer switches (``user_env``)."""
    if env is None:
        env = user_env()
    try:
        return subprocess.Popen(
            list(cmd), env=env, cwd=cwd, stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL if quiet else None,
            stderr=subprocess.DEVNULL if quiet else None,
            start_new_session=detach, close_fds=True)
    except OSError as e:
        log.warning("Kan %s niet starten (cannot start): %s", cmd[0] if cmd else "?", e)
        return None


# ---------------------------------------------------------------------------
# JSON + files
# ---------------------------------------------------------------------------

def _json_default(o: Any):
    if isinstance(o, os.PathLike):
        return os.fspath(o)
    if isinstance(o, (set, frozenset, tuple)):
        return list(o)
    if isinstance(o, (bytes, bytearray)):
        return bytes(o).decode("utf-8", "replace")
    if hasattr(o, "to_dict"):
        return o.to_dict()
    raise TypeError(f"Object of type {type(o).__name__} is not JSON serializable")


def _sanitize_floats(o: Any):
    if isinstance(o, float):
        return o if math.isfinite(o) else None
    if isinstance(o, dict):
        return {k: _sanitize_floats(v) for k, v in o.items()}
    if isinstance(o, (list, tuple, set, frozenset)):
        return [_sanitize_floats(v) for v in o]
    return o


def json_dumps(obj: Any, pretty: bool = False) -> str:
    """Strict JSON (NaN/Infinity become null), UTF-8 friendly, Paths/sets supported."""
    kw = dict(ensure_ascii=False, default=_json_default, allow_nan=False)
    if pretty:
        kw["indent"] = 2
    else:
        kw["separators"] = (",", ":")
    try:
        return json.dumps(obj, **kw)
    except ValueError:
        return json.dumps(_sanitize_floats(obj), **kw)


def json_loads(text: str | bytes, default: Any = None) -> Any:
    try:
        return json.loads(text)
    except (ValueError, TypeError):
        return default


def read_json(path: str | os.PathLike, default: Any = None) -> Any:
    try:
        with open(path, "rb") as f:
            return json.loads(f.read())
    except (OSError, ValueError):
        return default


def atomic_write_bytes(path: str | os.PathLike, data: bytes, mode: int = 0o600, dir_mode: int = 0o700) -> Path:
    """Write via temp file + fsync + rename (never leaves a half-written file)."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True, mode=dir_mode)
    tmp = path.with_name(f".{path.name}.{os.getpid()}.{threading.get_ident()}.tmp")
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_CLOEXEC, mode)
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.chmod(tmp, mode)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    return path


def atomic_write_text(path: str | os.PathLike, text: str, mode: int = 0o600) -> Path:
    return atomic_write_bytes(path, text.encode("utf-8"), mode=mode)


def atomic_write_json(path: str | os.PathLike, obj: Any, pretty: bool = True, mode: int = 0o600) -> Path:
    return atomic_write_text(path, json_dumps(obj, pretty=pretty) + "\n", mode=mode)


# ---------------------------------------------------------------------------
# Dict helpers (settings-style dot paths)
# ---------------------------------------------------------------------------

def deep_merge(base: dict, override: dict) -> dict:
    """New dict: ``override`` merged into ``base`` recursively (lists are replaced)."""
    out = copy.deepcopy(base) if isinstance(base, dict) else {}
    if not isinstance(override, dict):
        return out
    for k, v in override.items():
        if isinstance(v, dict) and isinstance(out.get(k), dict):
            out[k] = deep_merge(out[k], v)
        else:
            out[k] = copy.deepcopy(v)
    return out


_SEG_RE = re.compile(r"^[^\x00-\x1f]{1,200}$")


def split_path(path: str | list | tuple) -> list[str]:
    """'a.b.c' or ['a', 'b', 'c.txt'] → segments. Raises ApiError('bad_path')."""
    if isinstance(path, (list, tuple)):
        segs = [str(p) for p in path]
    elif isinstance(path, str):
        segs = path.split(".")
    else:
        raise ApiError("bad_path", "Ongeldige instelling.")
    if not segs or any(not s or not _SEG_RE.match(s) for s in segs) or len(segs) > 16:
        raise ApiError("bad_path", "Ongeldige instelling.", {"path": path})
    return segs


def get_path(d: Any, path: str | list | tuple, default: Any = None) -> Any:
    try:
        segs = split_path(path)
    except ApiError:
        return default
    cur = d
    for s in segs:
        if isinstance(cur, dict) and s in cur:
            cur = cur[s]
        else:
            return default
    return cur


def set_path(d: dict, path: str | list | tuple, value: Any) -> dict:
    segs = split_path(path)
    cur = d
    for s in segs[:-1]:
        nxt = cur.get(s)
        if not isinstance(nxt, dict):
            nxt = {}
            cur[s] = nxt
        cur = nxt
    cur[segs[-1]] = value
    return d


def del_path(d: dict, path: str | list | tuple) -> bool:
    segs = split_path(path)
    cur = d
    for s in segs[:-1]:
        cur = cur.get(s) if isinstance(cur, dict) else None
        if not isinstance(cur, dict):
            return False
    if isinstance(cur, dict) and segs[-1] in cur:
        del cur[segs[-1]]
        return True
    return False


def diff_paths(a: Any, b: Any, prefix: str = "", limit: int = 200) -> list[str]:
    """Dot paths (leaf level) whose values differ between two nested dicts."""
    out: list[str] = []

    def walk(x, y, pre):
        if len(out) >= limit:
            return
        if isinstance(x, dict) and isinstance(y, dict):
            for k in list(x.keys()) + [k for k in y.keys() if k not in x]:
                walk(x.get(k, _MISSING), y.get(k, _MISSING), f"{pre}.{k}" if pre else str(k))
        elif x != y:
            out.append(pre)

    walk(a, b, prefix)
    return out


_MISSING = object()


# ---------------------------------------------------------------------------
# XDG, user, compositor
# ---------------------------------------------------------------------------

def home() -> Path:
    return Path(os.path.expanduser("~"))


def _xdg(env: str, default: str) -> Path:
    v = os.environ.get(env)
    return Path(v) if v and os.path.isabs(v) else home() / default


def xdg_config_home() -> Path:
    return _xdg("XDG_CONFIG_HOME", ".config")


def xdg_cache_home() -> Path:
    return _xdg("XDG_CACHE_HOME", ".cache")


def xdg_data_home() -> Path:
    return _xdg("XDG_DATA_HOME", ".local/share")


def xdg_runtime_dir() -> Path | None:
    v = os.environ.get("XDG_RUNTIME_DIR")
    return Path(v) if v and os.path.isdir(v) else None


_USER_DIR_DEFAULTS = {
    "DESKTOP": "Desktop", "DOWNLOAD": "Downloads", "DOCUMENTS": "Documents", "MUSIC": "Music",
    "PICTURES": "Pictures", "VIDEOS": "Videos", "TEMPLATES": "Templates", "PUBLICSHARE": "Public",
}


def xdg_user_dir(kind: str) -> Path:
    """XDG user dir (DESKTOP, DOWNLOAD, DOCUMENTS, MUSIC, PICTURES, VIDEOS, TEMPLATES, PUBLICSHARE)."""
    kind = kind.upper()
    key = f"XDG_{kind}_DIR"
    try:
        text = (xdg_config_home() / "user-dirs.dirs").read_text(encoding="utf-8", errors="replace")
        for line in text.splitlines():
            line = line.strip()
            if line.startswith(key + "="):
                val = line.split("=", 1)[1].strip().strip('"')
                val = val.replace("$HOME", str(home()))
                p = Path(os.path.expanduser(val))
                if p.is_absolute() and p != home():
                    return p
    except OSError:
        pass
    return home() / _USER_DIR_DEFAULTS.get(kind, kind.title())


def runtime_dir() -> Path:
    """Directory for the instance file: $WIN11SHELL_RUNTIME_DIR or $XDG_RUNTIME_DIR/win11shell.
    Without XDG_RUNTIME_DIR: ``/tmp/win11shell-<uid>`` (a predictable name, so ``_private_dir`` checks that
    it really is ours before anything is read from or written to it)."""
    v = os.environ.get("WIN11SHELL_RUNTIME_DIR")
    if v:
        return Path(v)
    xr = xdg_runtime_dir()
    if xr:
        return xr / "win11shell"
    return Path(f"/tmp/win11shell-{os.getuid()}")


def _private_dir(d: Path, create: bool) -> bool:
    """True when ``d`` is a real directory owned by us and closed to others (0700). With ``create`` a
    missing one is made; a wrong mode on our own directory is repaired. A directory another user
    created (``/tmp/win11shell-<uid>`` pre-created as a trap) or a symlink is refused."""
    if create:
        try:
            os.makedirs(d.parent, exist_ok=True)
            os.mkdir(d, 0o700)
        except FileExistsError:
            pass
        except OSError:
            return False
    try:
        st = os.lstat(d)
    except OSError:
        return False
    if not stat.S_ISDIR(st.st_mode) or st.st_uid != os.getuid():
        log.error("Map %s is niet van jou of geen echte map; niet gebruikt (not ours / not a directory)", d)
        return False
    if st.st_mode & 0o077:
        try:
            os.chmod(d, 0o700)
        except OSError:
            return False
    return True


def user_info() -> dict:
    try:
        pw = pwd.getpwuid(os.getuid())
        name = pw.pw_name
        real = (pw.pw_gecos or "").split(",")[0].strip()
    except KeyError:
        name = os.environ.get("USER", "gebruiker")
        real = ""
    return {"name": name, "realName": real or name}


def compositor() -> str:
    """'hyprland' | 'gnome' | 'other'."""
    if os.environ.get("HYPRLAND_INSTANCE_SIGNATURE"):
        return "hyprland"
    desk = (os.environ.get("XDG_CURRENT_DESKTOP", "") + ":" + os.environ.get("XDG_SESSION_DESKTOP", "")).lower()
    if "hyprland" in desk:
        return "hyprland"
    if "gnome" in desk or "ubuntu" in desk:
        return "gnome"
    return "other"


def is_wayland() -> bool:
    return bool(os.environ.get("WAYLAND_DISPLAY")) or os.environ.get("XDG_SESSION_TYPE") == "wayland"


# ---------------------------------------------------------------------------
# Hyprland IPC (socket, no subprocess)
# ---------------------------------------------------------------------------

def hypr_socket_path(kind: str = ".socket.sock") -> Path | None:
    """Path of Hyprland's request socket (``.socket.sock``) or event socket (``.socket2.sock``)."""
    his = os.environ.get("HYPRLAND_INSTANCE_SIGNATURE")
    if not his:
        return None
    bases = []
    xr = xdg_runtime_dir()
    if xr:
        bases.append(xr / "hypr")
    bases.append(Path("/tmp/hypr"))
    for b in bases:
        p = b / his / kind
        if p.exists():
            return p
    return None


def hyprctl(command: str, json_out: bool = False, timeout: float = 2.0) -> Any:
    """Send a request to Hyprland (e.g. ``hyprctl('clients', json_out=True)``,
    ``hyprctl('dispatch focuswindow class:^(x)$')``). Returns text / parsed JSON, None on failure."""
    path = hypr_socket_path()
    if not path:
        return None
    msg = ("j/" if json_out else "") + command
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(timeout)
            s.connect(str(path))
            s.sendall(msg.encode("utf-8"))
            chunks = []
            while True:
                b = s.recv(65536)
                if not b:
                    break
                chunks.append(b)
    except OSError as e:
        log.debug("hyprctl %r failed: %s", command, e)
        return None
    text = b"".join(chunks).decode("utf-8", "replace")
    if json_out:
        return json_loads(text, None)
    return text


def hypr_dispatch(*args: str) -> bool:
    r = hyprctl("dispatch " + " ".join(args))
    return r is not None and r.strip().lower() == "ok"


# ---------------------------------------------------------------------------
# D-Bus (Gio) helpers
# ---------------------------------------------------------------------------

_gio_mods: tuple | None = None


def _gio():
    global _gio_mods
    if _gio_mods is None:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio, GLib
        _gio_mods = (Gio, GLib)
    return _gio_mods


def session_bus():
    Gio, _ = _gio()
    return Gio.bus_get_sync(Gio.BusType.SESSION, None)


def system_bus():
    Gio, _ = _gio()
    return Gio.bus_get_sync(Gio.BusType.SYSTEM, None)


def dbus_call(bus, name: str, path: str, iface: str, method: str, params: Any = None,
              signature: str | None = None, reply: str | None = None, timeout_ms: int = 3000) -> Any:
    """Synchronous D-Bus call; ``params`` is a GLib.Variant or a tuple with ``signature``
    (e.g. ``signature='(su)', params=('x', 1)``). Returns the unpacked reply tuple."""
    Gio, GLib = _gio()
    if params is not None and not isinstance(params, GLib.Variant):
        params = GLib.Variant(signature or "()", tuple(params))
    res = bus.call_sync(name, path, iface, method, params,
                        GLib.VariantType.new(reply) if reply else None,
                        Gio.DBusCallFlags.NONE, timeout_ms, None)
    return res.unpack() if res is not None else None


def dbus_get_property(bus, name: str, path: str, iface: str, prop: str, timeout_ms: int = 3000) -> Any:
    r = dbus_call(bus, name, path, "org.freedesktop.DBus.Properties", "Get",
                  (iface, prop), signature="(ss)", timeout_ms=timeout_ms)
    return r[0] if r else None


def dbus_name_has_owner(bus, name: str) -> bool:
    try:
        r = dbus_call(bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                      "NameHasOwner", (name,), signature="(s)")
        return bool(r and r[0])
    except Exception:
        return False


def glib_signal_add(signum: int, callback: Callable[[], bool]) -> int | None:
    """Unix signal → callback on the GLib main loop (GLibUnix.signal_add, or the old GLib name)."""
    try:
        import gi
        try:
            gi.require_version("GLibUnix", "2.0")
            from gi.repository import GLib, GLibUnix
            return GLibUnix.signal_add(GLib.PRIORITY_HIGH, signum, callback)
        except (ImportError, ValueError, AttributeError):
            import warnings
            from gi.repository import GLib
            with warnings.catch_warnings():
                warnings.simplefilter("ignore")
                return GLib.unix_signal_add(GLib.PRIORITY_HIGH, signum, callback)
    except Exception as e:  # noqa: BLE001
        log.debug("signal_add failed: %s", e)
        return None


# ---------------------------------------------------------------------------
# Instance file + client
# ---------------------------------------------------------------------------

def instance_path() -> Path:
    return runtime_dir() / "instance.json"


def pid_alive(pid: Any) -> bool:
    """Is ``pid`` a running Win11 Shell host of THIS user?"""
    try:
        pid = int(pid)
        if pid <= 0:
            return False
        os.kill(pid, 0)
    except PermissionError:
        return False  # someone else's process: not ours
    except (ValueError, TypeError, OSError):
        return False
    try:
        if os.stat(f"/proc/{pid}").st_uid != os.getuid():
            return False
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            cmd = f.read()
        return b"host" in cmd or b"win11shell" in cmd
    except OSError:
        return True


def proc_starttime(pid: int | str = "self") -> int | None:
    """When ``pid`` started, in clock ticks since boot (/proc/<pid>/stat field 22): immune to wall-clock steps and
    unchanged by an in-place restart (exec). The instance file carries the host's own (``starttime``), so ctl can
    tell the very same process from a reused pid exactly (host/ctl.py host_process)."""
    try:
        with open(f"/proc/{pid}/stat", "rb") as f:
            data = f.read()
        return int(data[data.rfind(b")") + 2:].split()[19])
    except (OSError, ValueError, IndexError):
        return None


def read_instance(check_alive: bool = True) -> dict | None:
    """The running instance from instance.json — only when the file and its folder are ours and private
    (another local user must not be able to point ctl/app windows at their own server)."""
    p = instance_path()
    if not _private_dir(p.parent, create=False):
        return None
    try:
        st = os.lstat(p)
    except OSError:
        return None
    if not stat.S_ISREG(st.st_mode) or st.st_uid != os.getuid() or st.st_mode & 0o077:
        log.warning("instance.json genegeerd: niet privé (ignored: not owned by you / not mode 0600)")
        return None
    data = read_json(p)
    if not isinstance(data, dict) or "port" not in data or "token" not in data:
        return None
    if check_alive and not pid_alive(data.get("pid")):
        return None
    return data


def write_instance(info: dict) -> Path | None:
    d = runtime_dir()
    if not _private_dir(d, create=True):
        log.error("Instance-bestand niet geschreven: %s is niet veilig (instance file not written: unsafe dir)", d)
        return None
    return atomic_write_json(instance_path(), info, pretty=False, mode=0o600)


def remove_instance(pid: int | None = None) -> None:
    p = instance_path()
    data = read_json(p)
    if pid is not None and isinstance(data, dict) and data.get("pid") != pid:
        return
    try:
        p.unlink()
    except OSError:
        pass


def call_instance(method: str, args: dict | None = None, inst: dict | None = None, timeout: float = 3.0) -> Any:
    """Call an API method on a running host. Raises ApiError (code 'not_running' when unreachable)."""
    if inst is None:
        inst = read_instance()
    if not inst:
        raise ApiError("not_running", "Win11 Shell draait niet.")
    body = json_dumps({"args": args or {}}).encode("utf-8")
    conn = http.client.HTTPConnection("127.0.0.1", int(inst["port"]), timeout=timeout)
    try:
        conn.request("POST", f"/api/call/{method}", body, {
            "Content-Type": "application/json", "X-Shell-Token": str(inst["token"]),
            "Host": f"127.0.0.1:{int(inst['port'])}"})
        resp = conn.getresponse()
        raw = resp.read()
        status = resp.status
    except (OSError, http.client.HTTPException) as e:
        raise ApiError("not_running", "Win11 Shell reageert niet.", str(e)) from None
    finally:
        conn.close()
    if status == 401:
        raise ApiError("unauthorized", "Geen toegang tot Win11 Shell.")
    data = json_loads(raw, None)
    if not isinstance(data, dict):
        raise ApiError("bad_response", "Onverwacht antwoord van Win11 Shell.")
    if not data.get("ok"):
        err = data.get("error") or {}
        raise ApiError(err.get("code", "error"), err.get("message", ""), err.get("data"))
    return data.get("result")


def own_app(app_or_desktop_id: Any) -> str | None:
    """'taskmgr' / 'win11shell-taskmgr' / 'win11shell-taskmgr.desktop' → 'taskmgr' (else None)."""
    s = str(app_or_desktop_id or "").strip().lower()
    s = s.removesuffix(".desktop").removeprefix("win11shell-")
    return s if s in APP_IDS else None


# ---------------------------------------------------------------------------
# Image sniffing
# ---------------------------------------------------------------------------

_IMAGE_EXT = {
    ".png": "image/png", ".apng": "image/png", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".jpe": "image/jpeg",
    ".jfif": "image/jpeg", ".gif": "image/gif", ".webp": "image/webp", ".svg": "image/svg+xml",
    ".svgz": "image/svg+xml", ".bmp": "image/bmp", ".ico": "image/vnd.microsoft.icon",
    ".avif": "image/avif", ".heic": "image/heic", ".heif": "image/heif", ".tif": "image/tiff",
    ".tiff": "image/tiff", ".xpm": "image/x-xpixmap", ".jxl": "image/jxl", ".tga": "image/x-tga",
}

#: Formats every browser engine we target renders natively.
BROWSER_IMAGE_MIMES = frozenset({"image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml",
                                 "image/bmp", "image/vnd.microsoft.icon", "image/avif"})


def _sniff(head: bytes, ext: str) -> str | None:
    if head.startswith(b"\x89PNG\r\n\x1a\n"):
        return "image/png"
    if head.startswith(b"\xff\xd8\xff"):
        return "image/jpeg"
    if head.startswith((b"GIF87a", b"GIF89a")):
        return "image/gif"
    if head[:4] == b"RIFF" and head[8:12] == b"WEBP":
        return "image/webp"
    if head.startswith(b"BM") and ext == ".bmp":
        return "image/bmp"
    if head.startswith(b"\x00\x00\x01\x00") and ext in (".ico", ".cur", ""):
        return "image/vnd.microsoft.icon"
    if head[4:8] == b"ftyp":
        brand = head[8:12]
        if brand in (b"avif", b"avis"):
            return "image/avif"
        if brand in (b"heic", b"heix", b"mif1", b"msf1", b"hevc"):
            return "image/heic"
    if head.startswith((b"II*\x00", b"MM\x00*")):
        return "image/tiff"
    if head.startswith(b"\xff\x0a") or head.startswith(b"\x00\x00\x00\x0cJXL "):
        return "image/jxl"
    if head.startswith(b"\x1f\x8b") and ext == ".svgz":
        return "image/svg+xml"
    if b"/* XPM */" in head[:64]:
        return "image/x-xpixmap"
    text = head.lstrip(b"\xef\xbb\xbf \t\r\n").lower()
    if text.startswith(b"<") and b"<svg" in text:
        return "image/svg+xml"
    if ext == ".tga" and len(head) > 18:
        return "image/x-tga"
    return None


def image_mime(path: str | os.PathLike) -> str | None:
    """MIME type of a regular file if (and only if) its content really is an image."""
    try:
        with open(path, "rb") as f:
            head = f.read(4096)
    except OSError:
        return None
    ext = os.path.splitext(str(path))[1].lower()
    return _sniff(head, ext)


# ---------------------------------------------------------------------------
# Icon theme lookup (freedesktop icon theme spec, no display needed)
# ---------------------------------------------------------------------------

_ICON_EXTS = (".svg", ".png", ".xpm")
_NAME_RE = re.compile(r"^[A-Za-z0-9_.+@ -][A-Za-z0-9_.+@ -]{0,254}$")


def _parse_ini(path: Path) -> dict[str, dict[str, str]]:
    sections: dict[str, dict[str, str]] = {}
    cur: dict[str, str] | None = None
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return sections
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            cur = sections.setdefault(line[1:-1].strip(), {})
            continue
        if cur is not None and "=" in line:
            k, v = line.split("=", 1)
            cur.setdefault(k.strip(), v.strip())
    return sections


def _int(v: Any, default: int) -> int:
    try:
        return int(str(v).strip())
    except (TypeError, ValueError):
        return default


# Theme directories whose icons are small monochrome UI glyphs (toolbar actions, emblems…). When a name
# exists both there and in a full-colour context (e.g. Win11-dark has ``actions/32/system-search.svg``, a
# thin #dedede magnifier, next to ``apps/scalable/system-search.svg``), the colour icon wins: /icon is used
# for app, place, device and file-type icons; our own UI glyphs come from icons.js.
_GLYPH_CONTEXTS = frozenset({"actions", "animations", "emblems", "emotes", "intl", "international"})


class _ThemeDir:
    __slots__ = ("rel", "size", "scale", "type", "minsize", "maxsize", "threshold", "glyph")

    def __init__(self, rel: str, props: dict[str, str]):
        self.rel = rel
        ctx = (props.get("Context") or "").strip().lower()
        if not ctx:                                    # some themes omit Context: guess from the path
            ctx = next((c for c in _GLYPH_CONTEXTS if c in rel.lower().split("/")), "")
        self.glyph = ctx in _GLYPH_CONTEXTS
        self.size = max(1, _int(props.get("Size"), 48))
        self.scale = max(1, _int(props.get("Scale"), 1))
        self.type = (props.get("Type") or "Threshold").strip().lower()
        self.minsize = _int(props.get("MinSize"), self.size)
        self.maxsize = _int(props.get("MaxSize"), self.size)
        self.threshold = _int(props.get("Threshold"), 2)

    def score(self, ext: str, size: int) -> tuple:
        """Lower is better: scalable SVG > SVG > raster (smallest >= size, else largest) > XPM."""
        hidpi = 0 if self.scale == 1 else 1
        eff = self.size * self.scale
        if ext == ".svg":
            if self.type == "scalable":
                in_range = self.minsize * self.scale <= size <= self.maxsize * self.scale
                return (0, 0 if in_range else 1, hidpi, abs(eff - size))
            return (1, abs(eff - size), hidpi, 0)
        tier = 2 if ext == ".png" else 4
        if eff >= size:
            return (tier, eff - size, hidpi, 0)
        return (tier + 1, size - eff, hidpi, 0)


class IconTheme:
    """One icon theme spread over several base dirs, indexed lazily (one scandir per directory)."""

    def __init__(self, name: str, base_dirs: list[Path]):
        self.name = name
        self.bases = [b for b in base_dirs if (b / name).is_dir()]
        index_file = next((b / name / "index.theme" for b in self.bases if (b / name / "index.theme").is_file()), None)
        self.valid = index_file is not None
        self.dirs: list[_ThemeDir] = []
        self.inherits: list[str] = []
        if index_file:
            ini = _parse_ini(index_file)
            head = ini.get("Icon Theme", {})
            self.inherits = [s.strip() for s in head.get("Inherits", "").split(",") if s.strip()]
            seen = set()
            for key in ("Directories", "ScaledDirectories"):
                for rel in head.get(key, "").split(","):
                    rel = rel.strip()
                    if rel and rel not in seen and rel in ini:
                        seen.add(rel)
                        self.dirs.append(_ThemeDir(rel, ini[rel]))
        self._index: dict[str, list[int]] | None = None
        self._mtimes: dict[str, float] = {}
        self._checked = 0.0
        self._lock = threading.Lock()

    def _build(self) -> None:
        index: dict[str, list[int]] = {}
        mtimes: dict[str, float] = {}
        for di, d in enumerate(self.dirs):
            for bi, base in enumerate(self.bases):
                p = os.path.join(base, self.name, d.rel)
                try:
                    mtimes[p] = os.stat(p).st_mtime
                    with os.scandir(p) as it:
                        for e in it:
                            n = e.name
                            dot = n.rfind(".")
                            if dot <= 0:
                                continue
                            ext = n[dot:].lower()
                            if ext not in _ICON_EXTS:
                                continue
                            index.setdefault(n[:dot], []).append((di << 8) | (bi << 2) | _ICON_EXTS.index(ext))
                except OSError:
                    continue
        self._index = index
        self._mtimes = mtimes
        self._checked = time.monotonic()

    def _ensure(self, refresh: bool = False) -> dict[str, list[int]]:
        with self._lock:
            if self._index is None:
                self._build()
            elif refresh and time.monotonic() - self._checked > 30:
                self._checked = time.monotonic()
                for p, mt in self._mtimes.items():
                    try:
                        if os.stat(p).st_mtime != mt:
                            self._build()
                            break
                    except OSError:
                        self._build()
                        break
            return self._index or {}

    def lookup(self, name: str, size: int, refresh: bool = False, glyphs: bool | None = None) -> Path | None:
        """Best file for ``name``. ``glyphs=False``: only full-colour contexts (apps, places, devices,
        mimetypes…); ``True``: only glyph contexts (actions, emblems…); ``None``: both, colour first."""
        if not self.valid:
            return None
        cands = self._ensure(refresh).get(name)
        if not cands:
            return None
        ranked = []
        for code in cands:
            di, bi, ei = code >> 8, (code >> 2) & 63, code & 3
            d = self.dirs[di]
            if glyphs is not None and d.glyph != glyphs:
                continue
            ext = _ICON_EXTS[ei]
            ranked.append(((d.glyph,) + d.score(ext, size), os.path.join(self.bases[bi], self.name, d.rel, name + ext)))
        ranked.sort(key=lambda t: t[0])
        for _, p in ranked:
            if os.path.isfile(p):  # follows symlinks; skips broken ones
                return Path(p)
        return None

    def has(self, name: str) -> bool:
        return self.valid and name in self._ensure()


class IconLookup:
    """Themed icon lookup with caching. Thread-safe."""

    CACHE_SIZE = 4096

    def __init__(self):
        self._themes: dict[str, IconTheme] = {}
        self._cache: collections.OrderedDict = collections.OrderedDict()
        self._lock = threading.Lock()
        self._theme_name: tuple[float, str] | None = None
        self._bases: tuple[float, list[Path]] | None = None

    # -- base dirs -----------------------------------------------------------
    def base_dirs(self) -> list[Path]:
        now = time.monotonic()
        if self._bases and now - self._bases[0] < 60:
            return self._bases[1]
        cands = [xdg_data_home() / "icons", home() / ".icons"]
        data_dirs = os.environ.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share"
        cands += [Path(d) / "icons" for d in data_dirs.split(":") if d and os.path.isabs(d)]
        cands += [Path("/usr/share/icons"), Path("/var/lib/flatpak/exports/share/icons"),
                  xdg_data_home() / "flatpak/exports/share/icons", Path("/var/lib/snapd/desktop/icons")]
        out, seen = [], set()
        for c in cands:
            try:
                rc = c.resolve()
            except OSError:
                continue
            if rc in seen or not rc.is_dir():
                continue
            seen.add(rc)
            out.append(c)
        self._bases = (now, out)
        return out

    def flat_dirs(self) -> list[Path]:
        return self.base_dirs() + [p for p in (Path("/usr/share/pixmaps"),) if p.is_dir()]

    # -- theme ----------------------------------------------------------------
    def current_theme(self) -> str:
        override = os.environ.get("WIN11SHELL_ICON_THEME")
        if override:
            return override
        now = time.monotonic()
        if self._theme_name and now - self._theme_name[0] < 5:
            return self._theme_name[1]
        name = None
        out = run_ok(["gsettings", "get", "org.gnome.desktop.interface", "icon-theme"], timeout=2)
        if out:
            name = out.strip().strip("'\"") or None
        if not name:
            ini = _parse_ini(xdg_config_home() / "gtk-3.0" / "settings.ini")
            name = ini.get("Settings", {}).get("gtk-icon-theme-name")
        name = name or "hicolor"
        self._theme_name = (now, name)
        return name

    def theme(self, name: str) -> IconTheme:
        with self._lock:
            t = self._themes.get(name)
            if t is None:
                t = IconTheme(name, self.base_dirs())
                self._themes[name] = t
            return t

    def chain(self, name: str | None = None) -> list[IconTheme]:
        name = name or self.current_theme()
        out: list[IconTheme] = []
        seen: set[str] = set()

        def add(n: str, depth: int = 0):
            if n in seen or depth > 10:
                return
            seen.add(n)
            t = self.theme(n)
            if not t.valid:
                return
            out.append(t)
            for parent in t.inherits:
                if parent != "hicolor":
                    add(parent, depth + 1)

        add(name)
        add("Adwaita")
        add("hicolor")
        return out

    # -- lookup ---------------------------------------------------------------
    def _flat_lookup(self, name: str) -> Path | None:
        for d in self.flat_dirs():
            for ext in (".svg", ".png", ".xpm"):
                p = d / (name + ext)
                if p.is_file():
                    return p
        return None

    def _lookup_one(self, name: str, size: int, themes: list[IconTheme], refresh: bool) -> Path | None:
        # A full-colour icon anywhere in the chain (or in pixmaps) beats a monochrome UI glyph in an
        # earlier theme: e.g. Win11-dark's thin grey actions/system-search must never stand in for the
        # Alles app icon when an apps/ icon of that name exists in Win11 or hicolor.
        for t in themes:
            p = t.lookup(name, size, refresh=refresh, glyphs=False)
            if p:
                return p
        p = self._flat_lookup(name)
        if p:
            return p
        for t in themes:
            p = t.lookup(name, size, refresh=refresh, glyphs=True)
            if p:
                return p
        return None

    def lookup(self, names: Iterable[str] | str, size: int = 32, generic: bool = True) -> Path | None:
        if isinstance(names, str):
            names = [names]
        clean: list[str] = []
        for n in names:
            if not n:
                continue
            n = str(n).strip()
            low = n.lower()
            for ext in _ICON_EXTS:
                if low.endswith(ext):
                    n = n[: -len(ext)]
                    break
            if _NAME_RE.match(n) and ".." not in n:
                clean.append(n)
        if not clean:
            return None
        size = max(8, min(1024, int(size)))
        theme_name = self.current_theme()
        key = (theme_name, tuple(clean), size, generic)
        now = time.monotonic()
        with self._lock:
            hit = self._cache.get(key)
            if hit is not None:
                path, ts = hit
                if path is not None or now - ts < 60:
                    self._cache.move_to_end(key)
                    if path is None or os.path.isfile(path):
                        return path
        themes = self.chain(theme_name)
        result = None
        for n in clean:
            result = self._lookup_one(n, size, themes, refresh=False)
            if result:
                break
        if result is None:
            for n in clean:  # retry once with a freshness check (newly installed apps)
                result = self._lookup_one(n, size, themes, refresh=True)
                if result:
                    break
        if result is None and generic:
            for n in clean:
                parts = n.split("-")
                while result is None and len(parts) > 1:
                    parts.pop()
                    result = self._lookup_one("-".join(parts), size, themes, refresh=False)
                if result:
                    break
        with self._lock:
            self._cache[key] = (result, now)
            while len(self._cache) > self.CACHE_SIZE:
                self._cache.popitem(last=False)
        return result

    def exists(self, name: str) -> bool:
        return any(t.has(name) for t in self.chain()) or self._flat_lookup(name) is not None


ICONS = IconLookup()


def current_icon_theme() -> str:
    return ICONS.current_theme()


def _abs_icon(name: str) -> Path | None:
    if name.startswith("file://"):
        from urllib.parse import unquote, urlsplit
        name = unquote(urlsplit(name).path)
    if not os.path.isabs(name):
        return None
    p = Path(name)
    try:
        if p.is_file() and image_mime(p):
            return p
    except OSError:
        return None
    return None


def find_icon(name: str, size: int = 32, generic: bool = True) -> Path | None:
    """Themed icon name (or absolute image path / file:// URI) → file path, or None."""
    if not name:
        return None
    name = str(name).strip()
    if name.startswith("/") or name.startswith("file://"):
        return _abs_icon(name)
    return ICONS.lookup([name], size, generic=generic)


def find_icon_any(names: Iterable[str], size: int = 32, generic: bool = True) -> Path | None:
    names = [n for n in names if n]
    for n in names:
        if n.startswith("/") or n.startswith("file://"):
            p = _abs_icon(n)
            if p:
                return p
    return ICONS.lookup([n for n in names if not n.startswith(("/", "file://"))], size, generic=generic)


def mime_icon_names(mime: str) -> list[str]:
    """Themed icon names for a MIME type, best first (non-symbolic before symbolic)."""
    mime = (mime or "").strip().lower()
    if not mime or "/" not in mime or len(mime) > 200:
        return []
    names: list[str] = []
    try:
        Gio, _ = _gio()
        icon = Gio.content_type_get_icon(mime)
        if icon is not None and hasattr(icon, "get_names"):
            names.extend(icon.get_names())
        gen = Gio.content_type_get_generic_icon_name(mime)
        if gen:
            names.append(gen)
    except Exception:
        pass
    major = mime.split("/", 1)[0]
    names += [mime.replace("/", "-"), f"{major}-x-generic"]
    if mime == "inode/directory":
        names.insert(0, "folder")
    names.append("text-x-generic" if major == "text" else "application-x-generic")
    names.append("unknown")
    seen, out = set(), []
    for n in sorted(names, key=lambda n: n.endswith("-symbolic")):
        if n not in seen:
            seen.add(n)
            out.append(n)
    return out


def find_mime_icon(mime: str, size: int = 32) -> Path | None:
    return ICONS.lookup(mime_icon_names(mime), size, generic=False)


def icon_exists(name: str) -> bool:
    if not name:
        return False
    if name.startswith(("/", "file://")):
        return _abs_icon(name) is not None
    return ICONS.exists(name)


def first_existing_icon(names: Iterable[str], fallback: str | None = None) -> str | None:
    """First icon *name* from ``names`` that the current theme chain can resolve."""
    for n in names:
        if n and icon_exists(n):
            return n
    return fallback


def sha1(text: str) -> str:
    return hashlib.sha1(text.encode("utf-8", "surrogateescape")).hexdigest()
