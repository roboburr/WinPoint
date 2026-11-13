"""Ctx: the object every service receives in ``register(api, ctx)``.

Attributes (read-only unless noted):
  root            project root (Path); ``ui_dir`` = root/'ui' (or $WIN11SHELL_UI_DIR)
  mode            'layer' | 'preview' | 'serve' | 'app'
  dry_run, dev    flags
  version         host version string
  config_dir      $WIN11SHELL_CONFIG_DIR or ~/.config/win11shell
  cache_dir       $WIN11SHELL_CACHE_DIR or ~/.cache/win11shell
  data_dir        $WIN11SHELL_DATA_DIR or ~/.local/share/win11shell
  desktop_dir     $WIN11SHELL_DESKTOP_DIR or the XDG desktop dir
  runtime_dir     where instance.json lives
  index_roots     list[str] from $WIN11SHELL_INDEX_ROOTS, or None
  home            Path.home()
  compositor      'hyprland' | 'gnome' | 'other'
  api             the Api registry
  settings        settings store: get(path=None, default=None), set(path, value), patch(dict),
                  on_change(cb(settings, path, changed)) → unsubscribe
  surfaces        SurfaceManager or None (serve mode / no GTK)
  port, token     HTTP server port + token (port is set once the server listens)
  services        {name: {"ok": bool, "error"?: str}}
  run_id          random per process image (also new after an in-place restart): ``shell.info.runId``
  loop_beat       time.monotonic() of the main loop's last beat (host/lifeguard.py), None without one
  open_app_id     the app this process was started for (``--open``), or None

Helpers:
  ctx.logger('files')              → logging.Logger 'win11shell.files'
  ctx.run_on_main(fn, *a, wait=True, timeout=15) → fn's result (GLib main thread)
  ctx.thread(target, *a, name=)    → started daemon thread
  ctx.open_app(app, params)        → open one of our apps (settings|taskmgr|explorer|alles|disks)
  ctx.on_shutdown(cb)              → cb() runs once on exit
  ctx.request_quit() / ctx.request_restart()
  ctx.ui_url(surface='app', app='taskmgr', ...) → full URL incl. token
  ctx.base_url                     → 'http://127.0.0.1:PORT'
"""

from __future__ import annotations

import logging
import os
import secrets
import threading
from pathlib import Path
from typing import Any, Callable
from urllib.parse import urlencode

from . import ROOT, VERSION
from .util import ApiError, compositor, current_dry_scope, dry_scope, home, runtime_dir, set_dry_run, \
    xdg_cache_home, xdg_config_home, xdg_data_home, xdg_user_dir

__all__ = ["Ctx"]


class _NoSettings:
    """Placeholder until services/settings.py installs the real store."""

    def __init__(self, log: logging.Logger):
        self._log = log
        self._warned = False

    def _warn(self):
        if not self._warned:
            self._warned = True
            self._log.warning("ctx.settings gebruikt vóór de instellingen geladen zijn (settings used before load)")

    def get(self, path=None, default=None):
        self._warn()
        try:
            from .services.settings import DEFAULTS
            from .util import get_path
            import copy
            return copy.deepcopy(DEFAULTS) if path is None else copy.deepcopy(get_path(DEFAULTS, path, default))
        except Exception:
            return default

    def set(self, path, value):
        raise ApiError("not_ready", "De instellingen zijn nog niet geladen.")

    def patch(self, patch):
        raise ApiError("not_ready", "De instellingen zijn nog niet geladen.")

    def on_change(self, cb):
        self._warn()
        return lambda: None


def _env_path(name: str, default: Path) -> Path:
    v = os.environ.get(name)
    return Path(os.path.expanduser(v)) if v else default


class Ctx:
    def __init__(self, *, mode: str, api=None, dry_run: bool = False, dev: bool = False,
                 port: int = 0, token: str = "", root: Path | None = None, logger: logging.Logger | None = None):
        self.root = Path(root or ROOT)
        self.ui_dir = _env_path("WIN11SHELL_UI_DIR", self.root / "ui")
        self.mode = mode
        self._dry_run = bool(dry_run)
        if self._dry_run:
            set_dry_run(True)      # one-way latch of the process default; never switched off here
        self.dev = bool(dev)
        self.version = VERSION
        self.home = home()
        self.config_dir = _env_path("WIN11SHELL_CONFIG_DIR", xdg_config_home() / "win11shell")
        self.cache_dir = _env_path("WIN11SHELL_CACHE_DIR", xdg_cache_home() / "win11shell")
        self.data_dir = _env_path("WIN11SHELL_DATA_DIR", xdg_data_home() / "win11shell")
        self.desktop_dir = _env_path("WIN11SHELL_DESKTOP_DIR", xdg_user_dir("DESKTOP"))
        self.runtime_dir = runtime_dir()
        roots = os.environ.get("WIN11SHELL_INDEX_ROOTS")
        self.index_roots = [r for r in roots.split(":") if r] if roots else None
        self.compositor = compositor()
        self.log = logger or logging.getLogger("win11shell")
        self.api = api
        if api is not None:
            api.ctx = self
        self.settings: Any = _NoSettings(self.log)
        self.surfaces = None
        self.port = int(port or 0)
        self.token = token
        self.services: dict[str, dict] = {}
        self.started_at = None
        self.shutting_down = False
        self.restart_requested = False
        self.exit_code = 0
        self.run_id = secrets.token_hex(6)
        self.loop_beat: float | None = None
        self.open_app_id: str | None = None
        # set by __main__ (host/lifeguard.py LoopGuard.arm): after request_quit/request_restart the process must be
        # gone in time, whatever the main loop does — never set in a unit test's own Ctx
        self.exit_guard: Callable[[str], None] | None = None
        self._open_app_impl: Callable | None = None
        self._shutdown_cbs: list[Callable[[], None]] = []
        self._quit_impl: Callable[[], None] | None = None
        self._loop_running = False
        self._quit_event = threading.Event()
        for d in (self.config_dir, self.cache_dir):
            try:
                d.mkdir(parents=True, exist_ok=True, mode=0o700)
            except OSError as e:
                self.log.warning("Kan map %s niet maken (cannot create dir): %s", d, e)

    # -- dry-run ----------------------------------------------------------------
    @property
    def dry_run(self) -> bool:
        """This context's own dry-run flag. Its Api registry follows it (``Api.dry_run``), so a
        dry context stays dry even when another context in the same process is not."""
        return self._dry_run

    @dry_run.setter
    def dry_run(self, value: bool) -> None:
        self._dry_run = bool(value)
        if self._dry_run:
            set_dry_run(True)

    def _scope_flag(self) -> bool:
        """Dry-run flag that work started from here (threads, main-loop calls) must carry."""
        return self._dry_run or bool(current_dry_scope())

    # -- URLs -----------------------------------------------------------------
    @property
    def base_url(self) -> str:
        return f"http://127.0.0.1:{self.port}"

    def ui_url(self, **params: Any) -> str:
        q = {k: (v if isinstance(v, str) else str(v)) for k, v in params.items() if v is not None}
        q["token"] = self.token
        return f"{self.base_url}/ui/index.html?{urlencode(q)}"

    # -- logging / threads ------------------------------------------------------
    def logger(self, name: str) -> logging.Logger:
        return logging.getLogger(f"win11shell.{name}")

    def thread(self, target: Callable, *args, name: str | None = None, **kwargs) -> threading.Thread:
        dry = self._scope_flag()

        def runner():
            try:
                with dry_scope(dry):           # @dangerous helpers in the thread follow THIS context
                    target(*args, **kwargs)
            except Exception:
                self.log.exception("Achtergrondtaak %s faalde (background thread failed)", name or target)

        t = threading.Thread(target=runner, name=name or getattr(target, "__name__", "worker"), daemon=True)
        t.start()
        return t

    # -- main loop --------------------------------------------------------------
    def set_main_loop(self, quit_impl: Callable[[], None] | None, running: bool) -> None:
        """Called by __main__ when the GLib/Gtk main loop starts (running=True) or stops."""
        self._quit_impl = quit_impl
        self._loop_running = running

    @property
    def has_main_loop(self) -> bool:
        return self._loop_running

    def run_on_main(self, fn: Callable, *args, wait: bool = True, timeout: float = 15.0, **kwargs) -> Any:
        """Run ``fn`` on the GLib main thread (GTK work). Without a main loop: direct call.

        wait=True blocks until done and returns the result (re-raises exceptions);
        wait=False schedules and returns None.
        """
        if not self._loop_running or threading.current_thread() is threading.main_thread():
            return fn(*args, **kwargs)
        from gi.repository import GLib

        done = threading.Event()
        box: dict[str, Any] = {}
        dry = self._scope_flag()

        def idle():
            try:
                with dry_scope(dry):
                    box["result"] = fn(*args, **kwargs)
            except BaseException as e:  # noqa: BLE001
                box["error"] = e
                if not wait:
                    self.log.exception("Fout op hoofdthread (error on main thread)")
            finally:
                done.set()
            return False

        GLib.idle_add(idle)
        if not wait:
            return None
        if not done.wait(timeout):
            raise ApiError("timeout", "Dit duurt te lang. Probeer het opnieuw.")
        if "error" in box:
            raise box["error"]
        return box.get("result")

    # -- apps -------------------------------------------------------------------
    def set_open_app(self, impl: Callable[[str, dict | None], Any]) -> None:
        self._open_app_impl = impl

    def open_app(self, app: str, params: dict | None = None, **kw) -> Any:
        """Open one of our apps (settings|taskmgr|explorer|alles|disks; also accepts
        'win11shell-<app>.desktop'). Same behaviour as API ``shell.openApp``."""
        if self._open_app_impl is None:
            raise ApiError("not_ready", "De shell is nog niet klaar.")
        return self._open_app_impl(app, params, **kw)

    # -- lifecycle --------------------------------------------------------------
    def on_shutdown(self, cb: Callable[[], None]) -> None:
        self._shutdown_cbs.append(cb)

    def run_shutdown_callbacks(self) -> None:
        cbs, self._shutdown_cbs = list(reversed(self._shutdown_cbs)), []
        for cb in cbs:
            try:
                cb()
            except Exception:
                self.log.exception("Afsluit-callback faalde (shutdown callback failed)")

    def request_quit(self, code: int = 0) -> None:
        """Thread-safe: stop the main loop (process exits cleanly). With ``exit_guard`` (the real host): if the
        process has not exited / exec'd in time (a hung main loop never runs the idle callback), the guard ends it."""
        self.exit_code = code
        self.shutting_down = True
        self._quit_event.set()
        guard = self.exit_guard
        if guard is not None:
            try:
                guard("restart" if self.restart_requested else "quit")
            except Exception:  # noqa: BLE001
                self.log.debug("exit guard failed", exc_info=True)
        if self._loop_running and self._quit_impl is not None:
            try:
                from gi.repository import GLib
                GLib.idle_add(lambda: (self._quit_impl(), False)[1])
            except Exception:
                self._quit_impl()

    def request_restart(self) -> None:
        self.restart_requested = True
        self.request_quit(0)

    def wait_quit(self, timeout: float | None = None) -> bool:
        return self._quit_event.wait(timeout)
