"""org.freedesktop.FileManager1 on the session bus: "Weergeven in map" from other apps opens Verkenner (SPEC §12).

Browsers ("Show in folder" for a download), xdg-desktop-portal's OpenDirectory, Electron apps, IDEs and others call
``org.freedesktop.FileManager1`` (freedesktop "file manager DBus interface", object ``/org/freedesktop/FileManager1``):

* ``ShowFolders(as uris, s startupId)``        → those folders, one Verkenner tab each;
* ``ShowItems(as uris, s startupId)``          → the parent folder of each item with the items selected (and
                                                 scrolled into view), one tab per folder;
* ``ShowItemProperties(as uris, s startupId)`` → like ShowItems, then Verkenner's "Eigenschappen" dialog for them
                                                 (first folder only).

URIs and paths are handled by ``host/explorer_args.py`` (``file://`` decoding, mounted gvfs locations, places; other
schemes are skipped with a log line). Opening goes through ``ctx.open_app('explorer', params, activation_token=…)``,
i.e. ``shell.openApp``: in layer mode one ``host.appwindow`` process (``win11shell-explorer``; an open Verkenner gets
``app.navigate`` and adds tabs), in preview mode the in-page window. ``startupId`` (an XDG activation token, or an
X11 startup id) is passed on as the activation token, so the compositor may give Verkenner the focus.

The method returns at once (the reply never waits for a window); the work runs on ONE worker thread, in order, from
a bounded queue. Invalid input (not a list of strings, > 1000 URIs, a URI > 8 KiB, control characters) →
``InvalidArgs``. ``LimitsExceeded`` (logged, nothing opens) when :data:`MAX_QUEUE` requests are already waiting, when
one sender made more than :data:`RATE_N` calls in :data:`RATE_S` s (a looping client cannot pile up tabs), or while
:data:`MAX_STUCK` earlier requests still hang on a location that does not answer. Looking at the paths (``stat``,
gvfs) runs in a helper thread with a :data:`STAT_TIMEOUT` s limit: a stale NFS / sshfs / gvfs mount costs that one
request (skipped, logged), never the service.

**A window that is still starting.** When Verkenner is closed, the first request starts its window (1–2 s); a second
request in that time becomes ``app.navigate`` for a page that has not connected yet, and events are not kept for
late clients. :class:`NavBacklog` (all modes with app windows, also for ``win11shell explorer …`` from the desktop
entry) keeps every ``app.navigate {app:'explorer'}`` that the CURRENT explorer process has not confirmed yet; the page
fetches them with ``explorer.ready`` (SPEC §12.1) — a stopgap until ``shell.openApp`` keeps such a pending list itself.

**Bus name — never a fight.** The name is owned only when Win11 Shell *is* the desktop: ``layer`` mode (in Hyprland,
or on another layer-shell compositor that does not run a full desktop such as GNOME, KDE Plasma, Xfce, Cinnamon —
see :data:`FULL_DESKTOPS`; there that desktop's file manager answers), or ``preview`` inside a Hyprland session;
never in ``app``/``serve`` mode. ``WIN11SHELL_FILEMANAGER1=off`` disables it, ``=own`` forces it (tests / debugging).
Flags: ``ALLOW_REPLACEMENT`` and queueing (no ``REPLACE``, no ``DO_NOT_QUEUE``):
* name free → ours (the normal case: the shell starts with the session, before anyone asks);
* name already owned (Nautilus running as a D-Bus service, another file manager) → we wait in the bus's queue and
  get the name as soon as that owner quits — nobody is killed or replaced;
* a program that explicitly asks to *replace* the owner gets it; we go back into the queue (and get it back when
  it quits). Nautilus itself never asks for replacement, so with Win11 Shell running it queues behind us.
* ``preview`` mode adds ``DO_NOT_QUEUE``: a preview window takes the name only when it is free and never waits for
  it, so it cannot inherit it from the real desktop while that restarts.
* D-Bus activation (``org.freedesktop.FileManager1.service`` starts Nautilus when nobody owns the name) can only
  happen while the shell is not running (or in the ~2 s of a restart).
"""

from __future__ import annotations

import logging
import os
import threading
import time
from collections import deque
from typing import Any, Callable

from ..explorer_args import explorer_params, gio_local_path
from ..util import ApiError

log = logging.getLogger("win11shell.filemanager1")

PRIORITY = 60

NAME = "org.freedesktop.FileManager1"
PATH = "/org/freedesktop/FileManager1"
IFACE = "org.freedesktop.FileManager1"
MAX_URIS = 1000
MAX_URI_LEN = 8192
MAX_QUEUE = 16         # requests waiting for the worker
RATE_N, RATE_S = 20, 10.0   # calls per D-Bus sender per window
STAT_TIMEOUT = 3.0     # s for looking at the paths of one request
MAX_STUCK = 4          # helper threads still hanging on a dead mount before new requests are refused
NAV_KEEP_S = 30.0      # NavBacklog: how long an undelivered app.navigate is kept
NAV_MAX = 16

INTROSPECTION = """
<node>
  <interface name="org.freedesktop.FileManager1">
    <method name="ShowFolders">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
    <method name="ShowItems">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
    <method name="ShowItemProperties">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
  </interface>
</node>
"""

#: method → (explorer_args 'how', properties)
METHODS = {"ShowFolders": ("auto", False), "ShowItems": ("items", False), "ShowItemProperties": ("items", True)}

try:
    import gi
    gi.require_version("Gio", "2.0")
    from gi.repository import Gio, GLib
except Exception:  # noqa: BLE001
    Gio = GLib = None  # type: ignore[assignment]


def _env_mode() -> str:
    return os.environ.get("WIN11SHELL_FILEMANAGER1", "").strip().lower()


#: XDG_CURRENT_DESKTOP / XDG_SESSION_DESKTOP words of desktops that bring their own file manager (and answer
#: FileManager1 with it): Win11 Shell must not take the name there, even in layer mode.
FULL_DESKTOPS = ("gnome", "ubuntu", "kde", "plasma", "plasmawayland", "unity", "xfce", "x-cinnamon", "cinnamon", "mate",
                 "lxqt", "lxde", "budgie", "pantheon", "deepin", "dde", "cosmic", "enlightenment", "ukui")


def _full_desktop() -> str | None:
    """The full desktop session we run inside (``KDE``, ``GNOME``…), from the XDG desktop variables; else None."""
    for var in ("XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"):
        for word in os.environ.get(var, "").replace(";", ":").split(":"):
            w = word.strip().lower()
            if w and (w in FULL_DESKTOPS or w.split("-")[0] in FULL_DESKTOPS):
                return word.strip()
    return None


def _clean_token(tok: Any) -> str | None:
    """startupId → activation token (printable ASCII, ≤ 512 chars), else None."""
    if not isinstance(tok, str) or not tok or len(tok) > 512:
        return None
    if any(ord(c) < 33 or ord(c) > 126 for c in tok):
        return None
    return tok


class InvalidArgs(ValueError):
    pass


def check_uris(uris: Any) -> list[str]:
    if not isinstance(uris, (list, tuple)) or len(uris) > MAX_URIS:
        raise InvalidArgs(f"expected at most {MAX_URIS} URIs")
    out = []
    for u in uris:
        if not isinstance(u, str) or len(u) > MAX_URI_LEN or any(ord(c) < 32 for c in u):
            raise InvalidArgs("invalid URI")
        if u:
            out.append(u)
    return out


class FileManager1:
    """The service. ``bus`` (a Gio.DBusConnection) and ``open_app`` are injectable for tests."""

    def __init__(self, api, ctx, bus=None, *, open_app: Callable | None = None,
                 remote: Callable[[str], str | None] | None = gio_local_path):
        self.api = api
        self.ctx = ctx
        self.bus = bus
        self.open_app = open_app or ctx.open_app
        self.remote = remote
        self.state = "off"           # off | waiting | owner
        self.reason = ""
        self.owner_id = 0
        self.reg_id = 0
        self.calls = 0
        self.refused = 0
        self._lock = threading.Lock()        # one request at a time, in order (tabs open in the asked order)
        self._cv = threading.Condition()     # queue, rate table, stuck counter
        self._queue: deque = deque()
        self._rate: dict[str, deque] = {}
        self._stuck = 0
        self._worker: threading.Thread | None = None
        self._started = False
        self._stopped = False

    # -- policy -----------------------------------------------------------------------
    def should_own(self) -> tuple[bool, str]:
        force = _env_mode()
        if force in ("off", "0", "no", "false"):
            return False, "WIN11SHELL_FILEMANAGER1=off"
        if force in ("own", "on", "1", "yes", "true"):
            return True, "WIN11SHELL_FILEMANAGER1=own"
        if self.ctx.mode == "layer":
            if self.ctx.compositor == "hyprland":
                return True, "layer mode in Hyprland"
            full = _full_desktop()
            if full:   # e.g. win11shell started inside Plasma (KWin has layer-shell): Dolphin stays the file manager
                return False, f"layer mode inside {full}"
            return True, f"layer mode on {self.ctx.compositor}"
        if self.ctx.mode == "preview" and self.ctx.compositor == "hyprland":
            return True, "preview mode in Hyprland"
        return False, f"{self.ctx.mode} mode on {self.ctx.compositor}"

    # -- start / stop -------------------------------------------------------------------
    def start(self) -> None:
        if self._started:
            return
        self._started = True
        if Gio is None:
            self.reason = "no Gio"
            return
        ok, why = self.should_own()
        self.reason = why
        if not ok:
            log.info("Verkenner neemt %s niet over: %s (FileManager1 not served)", NAME, why)
            return
        if self.bus is None:
            try:
                self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
            except Exception as e:  # noqa: BLE001
                log.info("FileManager1: geen sessiebus (no session bus): %s", e)
                self.bus = None
        if self.bus is None:
            self.reason = "no session bus"
            return
        try:
            node = Gio.DBusNodeInfo.new_for_xml(INTROSPECTION)
            reg = getattr(self.bus, "register_object_with_closures2", None) or self.bus.register_object
            self.reg_id = reg(PATH, node.interfaces[0], self._on_call, None, None)
        except Exception:  # noqa: BLE001
            log.exception("FileManager1-object registreren mislukt (cannot register object)")
            return
        self.state = "waiting"
        flags = Gio.BusNameOwnerFlags.ALLOW_REPLACEMENT   # queue when taken; never REPLACE anyone
        if self.ctx.mode != "layer":
            # a preview window only takes a free name and never waits for it: it must not inherit the name from
            # the real desktop while that restarts
            flags |= Gio.BusNameOwnerFlags.DO_NOT_QUEUE
        self.owner_id = Gio.bus_own_name_on_connection(self.bus, NAME, flags, self._acquired, self._lost)
        try:
            from .apps import ensure_glib_loop
            ensure_glib_loop(self.ctx)
        except Exception:  # noqa: BLE001
            pass

    def _acquired(self, _conn, _name) -> None:
        self.state = "owner"
        log.info("Verkenner is nu de bestandsbeheerder voor andere apps (owns %s)", NAME)

    def _lost(self, _conn, _name) -> None:
        was = self.state
        if self._stopped:
            return
        if self.ctx.mode != "layer":       # DO_NOT_QUEUE: not in the queue, nothing to wait for
            self.state = "off"
            self.reason = "name taken"
            log.info("%s is bezet; de voorbeeldweergave neemt het niet over (name taken, not served)", NAME)
            return
        self.state = "waiting"
        if was == "owner":
            log.warning("%s is overgenomen door een andere app; Verkenner wacht tot die stopt "
                        "(name replaced, queued again)", NAME)
        else:
            log.info("%s is bezet (bijv. door Bestanden); Verkenner wacht in de rij tot die stopt "
                     "(name taken, queued — nobody is replaced)", NAME)

    def stop(self) -> None:
        if self._stopped:
            return
        self._stopped = True
        with self._cv:
            self._queue.clear()
            self._cv.notify_all()
        if self.owner_id:
            try:
                Gio.bus_unown_name(self.owner_id)
            except Exception:  # noqa: BLE001
                pass
            self.owner_id = 0
        if self.bus is not None and self.reg_id:
            try:
                self.bus.unregister_object(self.reg_id)
            except Exception:  # noqa: BLE001
                pass
            self.reg_id = 0
        self.state = "off"

    # -- calls --------------------------------------------------------------------------
    def _on_call(self, conn, sender, path, iface, method, params, invocation):
        try:
            if method not in METHODS:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", f"No method {method}")
                return
            try:
                uris, startup_id = params.unpack()
                uris = check_uris(list(uris))
            except (InvalidArgs, TypeError, ValueError) as e:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", str(e) or "invalid arguments")
                return
            why = self.submit(sender or "?", method, uris, startup_id)
            if why:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.LimitsExceeded", why)
                return
            invocation.return_value(None)
            log.info("%s van %s: %d item(s) (from)", method, sender, len(uris))
        except Exception:  # noqa: BLE001
            log.exception("FileManager1.%s faalde (failed)", method)
            try:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "internal error")
            except Exception:  # noqa: BLE001
                pass

    def submit(self, sender: str, method: str, uris: list[str], startup_id: str = "") -> str | None:
        """Queue one call for the worker. → None, or why it was refused (logged; the caller gets LimitsExceeded)."""
        now = time.monotonic()
        with self._cv:
            why = None
            if self._stopped:
                why = "service stopped"
            elif len(self._queue) >= MAX_QUEUE:
                why = f"too many requests waiting ({MAX_QUEUE})"
            elif self._stuck >= MAX_STUCK:
                why = "file locations do not respond"
            else:
                hits = self._rate.get(sender)
                if hits is None:
                    if len(self._rate) >= 256:      # forget idle senders
                        for k in [k for k, v in self._rate.items() if not v or now - v[-1] > RATE_S]:
                            del self._rate[k]
                        while len(self._rate) >= 256:   # still full: the oldest sender goes
                            del self._rate[next(iter(self._rate))]
                    hits = self._rate[sender] = deque()
                while hits and now - hits[0] > RATE_S:
                    hits.popleft()
                if len(hits) >= RATE_N:
                    why = f"too many requests from this app ({RATE_N} in {RATE_S:g} s)"
                else:
                    hits.append(now)
            if why:
                self.refused += 1
            else:
                self.calls += 1
                self._queue.append((method, uris, startup_id))
                self._cv.notify()
                if self._worker is None or not self._worker.is_alive():
                    self._worker = self._spawn(self._run, "filemanager1")
        if why:
            log.warning("%s van %s geweigerd (refused): %s", method, sender, why)
        return why

    def _spawn(self, target, name: str) -> threading.Thread:
        if hasattr(self.ctx, "thread"):
            return self.ctx.thread(target, name=name)
        t = threading.Thread(target=target, name=name, daemon=True)  # pragma: no cover - a bare test context
        t.start()
        return t

    def _run(self) -> None:
        while True:
            with self._cv:
                while not self._queue and not self._stopped:
                    self._cv.wait()
                if self._stopped:
                    return
                item = self._queue.popleft()
            try:
                self.handle(*item)
            except Exception:  # noqa: BLE001
                log.exception("FileManager1.%s faalde (failed)", item[0])

    def _params(self, uris: list[str], how: str, props: bool) -> dict | None:
        """explorer_params in a helper thread, at most STAT_TIMEOUT s (a dead mount blocks stat() for good)."""
        box: dict[str, Any] = {}
        done = threading.Event()

        def work():
            try:
                box["params"] = explorer_params(uris, how=how, properties=props, remote=self.remote)
            except Exception as e:  # noqa: BLE001
                box["error"] = e
            finally:
                done.set()
                with self._cv:
                    if box.get("late"):
                        self._stuck -= 1
                        log.info("Een trage locatie antwoordt weer (a slow location answered after all)")

        threading.Thread(target=work, name="filemanager1-stat", daemon=True).start()
        if not done.wait(STAT_TIMEOUT):
            with self._cv:
                if not done.is_set():
                    box["late"] = True
                    self._stuck += 1
            if box.get("late"):
                log.warning("Locatie antwoordt niet binnen %g s; verzoek overgeslagen (location does not respond, "
                            "request skipped): %s", STAT_TIMEOUT, (uris[0] if uris else "")[:200])
                return None
        if "error" in box:
            raise box["error"]
        return box.get("params")

    def handle(self, method: str, uris: list[str], startup_id: str = "") -> dict | None:
        """Turn one call into Explorer params and open them (worker thread; also called by tests). → params|None"""
        how, props = METHODS[method]
        with self._lock:
            params = self._params(uris, how, props)
            if params is None:
                log.info("%s: niets om te tonen (nothing to show)", method)
                return None
            try:
                self.open_app("explorer", params, activation_token=_clean_token(startup_id))
            except Exception as e:  # noqa: BLE001
                log.warning("Verkenner kon niet openen voor %s (could not open): %s", method, getattr(e, "message", e))
                return None
            return params


class NavBacklog:
    """``app.navigate`` for an app window that may not have received it yet (SPEC §12.1, ``explorer.ready``).

    ``shell.openApp`` sends a request for a RUNNING app window as the event ``app.navigate``; a window that is still
    starting (its page has not opened the event stream yet) never sees it. This keeps every such event (≤ NAV_MAX,
    ≤ NAV_KEEP_S s old) until the page of the current app process confirmed it:

    1. the page (after its ``app.navigate`` handler is in place, again after each reconnect ``hello``) calls
       ``explorer.ready {id}``; the answer is ``{}`` and the backlog follows as the event
       ``explorer.backlog {id, boot, seq, navigate:[{seq, params}…]}`` — on the same event stream, so receiving it
       proves that the page's stream is connected (the page retries every second until it has it);
    2. the page opens those it has not seen yet (not as ``app.navigate``, not in an earlier backlog of this ``boot``)
       and calls ``explorer.ready {id, ack: seq}``; the host drops them and, when nothing newer is left, marks the
       current process as ready (nothing is kept for it any more); a newer one → another ``explorer.backlog``.

    "Current process": the pid ``shell.py``'s app-window table has for the app (read only; without it a counter of
    ``shell.appClosed`` events stands in). Only in ``layer``/``app`` mode (preview windows get ``shell.openApp``)."""

    def __init__(self, api, ctx, app: str = "explorer"):
        self.api = api
        self.ctx = ctx
        self.app = app
        self._lock = threading.Lock()
        self._items: deque = deque(maxlen=NAV_MAX)     # (seq, monotonic, params)
        self._seq = 0
        self._ready_key: tuple | None = None
        self._closed = 0
        self.boot = os.urandom(6).hex()      # seq numbers restart with the host (also after an in-place exec)
        api.on("app.navigate", self._on_navigate)
        api.on("shell.appClosed", self._on_closed)

    def _key(self) -> tuple:
        procs = getattr(getattr(self.ctx, "_app_windows", None), "procs", None)
        proc = procs.get(self.app) if isinstance(procs, dict) else None
        pid = getattr(proc, "pid", None)
        return ("pid", pid) if isinstance(pid, int) else ("closed", self._closed)

    def _on_navigate(self, _event, data) -> None:
        if not isinstance(data, dict) or data.get("app") != self.app or self.ctx.mode not in ("layer", "app"):
            return
        params = data.get("params") if isinstance(data.get("params"), dict) else {}
        with self._lock:
            if self._ready_key == self._key():
                return
            self._seq += 1
            self._items.append((self._seq, time.monotonic(), params))

    def _on_closed(self, _event, data) -> None:
        if isinstance(data, dict) and data.get("app") == self.app:
            with self._lock:
                self._closed += 1
                self._items.clear()

    def ready(self, args: dict) -> dict:
        page = args.get("id")
        if not isinstance(page, str) or not page or len(page) > 64:
            raise ApiError("bad_args", "Ongeldige invoer.")
        ack = args.get("ack")
        now = time.monotonic()
        with self._lock:
            while self._items and now - self._items[0][1] > NAV_KEEP_S:
                self._items.popleft()
            if isinstance(ack, int) and not isinstance(ack, bool):
                while self._items and self._items[0][0] <= ack:
                    self._items.popleft()
                if not self._items:
                    self._ready_key = self._key()
                    return {}
            items = list(self._items)
            seq = items[-1][0] if items else self._seq
        if items:
            log.info("%d verzoek(en) voor een net gestart venster nagestuurd (requests for a starting window "
                     "delivered late): %s", len(items), self.app)
        self.api.emit(f"{self.app}.backlog", {"id": page, "boot": self.boot, "seq": seq,
                                              "navigate": [{"seq": n, "params": p} for n, _t, p in items]})
        return {}


def register(api, ctx) -> None:
    ctx._filemanager1 = FileManager1(api, ctx)
    backlog = ctx._explorer_backlog = NavBacklog(api, ctx, "explorer")

    @api.method("explorer.ready")
    def _ready(args):
        """Verkenner's page: ``{id}`` → event ``explorer.backlog {id, seq, navigate}``; ``{id, ack}`` confirms."""
        return backlog.ready(args)


def start(api, ctx) -> None:
    svc = getattr(ctx, "_filemanager1", None)
    if svc is not None:
        svc.start()


def stop(api, ctx) -> None:
    svc = getattr(ctx, "_filemanager1", None)
    if svc is not None:
        svc.stop()
