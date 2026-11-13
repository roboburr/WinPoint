"""API method registry + event bus.

    api = Api()

    @api.method("settings.get")
    def _get(args):            # args: dict  →  JSON-serialisable result
        return store.get()

    @api.method("files.trash", dangerous=True, dry_result=lambda a: {"failed": []})
    def _trash(args): ...      # not called in --dry-run: returns {"ok": True, "dryRun": True, "failed": []}

    api.emit("settings.changed", {...})     # thread-safe broadcast to every SSE client
    api.call("settings.get", {})            # in-process call (raises ApiError)

Errors: raise ``ApiError(code, "Nederlandse uitleg")`` → ``{"ok": false, "error": {code, message}}``.
Any other exception → code ``internal`` (logged with traceback).
"""

from __future__ import annotations

import copy
import itertools
import logging
import queue
import threading
import time
from dataclasses import dataclass, field
from typing import Any, Callable

from .util import ApiError, current_dry_scope, dry_run_result, dry_scope, is_dry_run, json_dumps, redact, \
    set_dry_run, test_guard_blocks

__all__ = ["Api", "ApiError", "Subscriber", "MethodInfo"]

log = logging.getLogger("win11shell.api")

_SLOW_CALL_S = 2.0


@dataclass
class MethodInfo:
    name: str
    fn: Callable[[dict], Any]
    dangerous: bool = False
    dry_result: Any = None
    doc: str = ""
    calls: int = field(default=0)


class Subscriber:
    """One SSE client. The server thread reads ``next(timeout)``; ``push`` never blocks."""

    _ids = itertools.count(1)

    def __init__(self, maxsize: int = 512, remote: str | None = None):
        self.id = next(self._ids)
        self.q: queue.Queue = queue.Queue(maxsize=maxsize)
        self.closed = False
        self.dropped = False
        self.remote = remote
        self.created = time.time()

    def push(self, event: str, data_json: str) -> bool:
        if self.closed:
            return False
        try:
            self.q.put_nowait((event, data_json))
            return True
        except queue.Full:
            # Too slow: drop this client (EventSource reconnects and gets a fresh 'hello').
            self.dropped = True
            self.close()
            return False

    def push_obj(self, event: str, data: Any) -> bool:
        return self.push(event, json_dumps(data))

    def close(self) -> None:
        if self.closed:
            return
        self.closed = True
        try:
            while True:
                self.q.get_nowait()
        except queue.Empty:
            pass
        try:
            self.q.put_nowait(None)  # wake the reader
        except queue.Full:  # pragma: no cover
            pass

    def next(self, timeout: float):
        """(event, data_json) | None (closed). Raises queue.Empty on timeout."""
        return self.q.get(timeout=timeout)


class Api:
    def __init__(self, dry_run: bool | None = None):
        # Per-registry dry-run: an explicit flag, else the flag of the Ctx that owns this registry.
        # Never the process default, so another Ctx/Api in the same process cannot disarm this one.
        self._dry_run = None if dry_run is None else bool(dry_run)
        if self._dry_run:
            set_dry_run(True)          # latch the default for code outside calls (pollers…)
        self._methods: dict[str, MethodInfo] = {}
        self._subs: set[Subscriber] = set()
        self._listeners: dict[str, list[Callable]] = {}
        self._connect_hooks: list[Callable[[Subscriber], None]] = []
        self._lock = threading.RLock()
        self._closed = False
        self.ctx = None  # set by Ctx

    # -- registration -----------------------------------------------------------
    def method(self, name: str, dangerous: bool = False, dry_result: Any = None, doc: str = ""):
        """Decorator registering ``fn(args: dict) -> result`` under ``name``.

        ``dangerous=True``: in --dry-run the handler is skipped, the call is logged and
        ``{"ok": True, "dryRun": True}`` (merged with ``dry_result`` – a dict or
        ``callable(args) -> dict``) is returned instead.
        """

        def deco(fn: Callable[[dict], Any]):
            self.register(name, fn, dangerous=dangerous, dry_result=dry_result, doc=doc or (fn.__doc__ or ""))
            return fn

        return deco

    def register(self, name: str, fn: Callable[[dict], Any], dangerous: bool = False,
                 dry_result: Any = None, doc: str = "") -> None:
        if not isinstance(name, str) or not name or len(name) > 100:
            raise ValueError(f"bad method name: {name!r}")
        with self._lock:
            if name in self._methods:
                log.warning("Methode %s wordt opnieuw geregistreerd (re-registered; last one wins)", name)
            self._methods[name] = MethodInfo(name, fn, bool(dangerous), dry_result, doc.strip())

    def unregister(self, name: str) -> None:
        with self._lock:
            self._methods.pop(name, None)

    def has(self, name: str) -> bool:
        return name in self._methods

    def names(self) -> list[str]:
        with self._lock:
            return sorted(self._methods)

    def describe(self) -> list[dict]:
        with self._lock:
            return [{"name": m.name, "dangerous": m.dangerous, "doc": m.doc.splitlines()[0] if m.doc else ""}
                    for m in sorted(self._methods.values(), key=lambda m: m.name)]

    @property
    def dry_run(self) -> bool:
        """This registry's own dry-run flag: the explicit one (``Api(dry_run=…)``), else its Ctx's.
        A registry without either follows ``is_dry_run()``. A call made from inside a dry call
        (``current_dry_scope()``) is always dry."""
        own = self._dry_run
        if own is None:
            ctx = self.ctx
            own = bool(getattr(ctx, "dry_run", False)) if ctx is not None else is_dry_run()
        return bool(own) or bool(current_dry_scope())

    @dry_run.setter
    def dry_run(self, value: bool | None) -> None:
        self._dry_run = None if value is None else bool(value)
        if self._dry_run:
            set_dry_run(True)

    # -- calling ----------------------------------------------------------------
    def call(self, name: str, args: dict | None = None) -> Any:
        """In-process call. Returns the result or raises ApiError."""
        return self._call(name, args)[0]

    def _call(self, name: str, args: dict | None) -> tuple[Any, bool]:
        """→ (result, skipped_because_dry_run)."""
        info = self._methods.get(name)
        if info is None:
            raise ApiError("unknown_method", "Deze functie bestaat niet.", {"method": name})
        if args is None:
            args = {}
        if not isinstance(args, dict):
            raise ApiError("bad_args", "Ongeldige invoer.")
        info.calls += 1
        dry = self.dry_run
        if info.dangerous and (dry or test_guard_blocks(name)):
            if dry:
                log.info("[dry-run] %s %s overgeslagen (skipped)", name, _short(args))
            else:
                log.warning("[test] %s %s overgeslagen: testrun, niets aan het systeem veranderd "
                            "(skipped in a unit-test process; WIN11SHELL_ALLOW_REAL=1 to allow)", name, _short(args))
            extra = info.dry_result(args) if callable(info.dry_result) else copy.deepcopy(info.dry_result)
            return dry_run_result(extra), True
        t0 = time.monotonic()
        try:
            # Helpers (@dangerous, dry_for) and nested api.call()s inside this handler see THIS
            # registry's flag – per thread, whatever another context in the process says.
            with dry_scope(dry):
                result = info.fn(args)
            return result, bool(getattr(info.fn, "dangerous", False) and isinstance(result, dict)
                                and result.get("dryRun") is True)
        finally:
            dt = time.monotonic() - t0
            if dt > _SLOW_CALL_S:
                log.warning("Trage aanroep (slow call) %s: %.1f s", name, dt)

    def dispatch(self, name: str, args: Any) -> dict:
        """Call and wrap in the HTTP envelope ``{"ok": ..., "result"|"error": ...}``."""
        try:
            result, skipped = self._call(name, args if args is not None else {})
            env = {"ok": True, "result": result}
            if skipped:
                env["dryRun"] = True
            return env
        except ApiError as e:
            if e.code not in ("unknown_method",):
                log.info("%s → %s: %s", name, e.code, e.message)
            return {"ok": False, "error": e.to_dict()}
        except Exception as e:  # noqa: BLE001 - every failure becomes a friendly error
            log.exception("Fout in %s (error in handler)", name)
            return {"ok": False, "error": {"code": "internal",
                                           "message": "Er ging iets mis. Probeer het opnieuw.",
                                           "detail": f"{type(e).__name__}: {e}"[:300]}}

    # -- events -----------------------------------------------------------------
    def emit(self, event: str, data: Any = None) -> int:
        """Broadcast to every SSE client and in-process listener. Thread-safe. Returns #clients."""
        if data is None:
            data = {}
        try:
            payload = json_dumps(data)
        except (TypeError, ValueError) as e:
            log.error("Gebeurtenis %s niet te verzenden (event not serialisable): %s", event, e)
            return 0
        with self._lock:
            subs = list(self._subs)
            listeners = list(self._listeners.get(event, ())) + list(self._listeners.get("*", ()))
        sent = 0
        for s in subs:
            if s.push(event, payload):
                sent += 1
            elif s.dropped:
                log.warning("Trage SSE-client %s losgekoppeld (slow client dropped)", s.id)
                self.unsubscribe(s)
        for cb in listeners:
            try:
                cb(event, data)
            except Exception:
                log.exception("Listener voor %s faalde (listener failed)", event)
        return sent

    def on(self, event: str, cb: Callable[[str, Any], None]) -> Callable[[], None]:
        """In-process listener ``cb(event, data)`` (``'*'`` = all events). Returns unsubscribe()."""
        with self._lock:
            self._listeners.setdefault(event, []).append(cb)

        def off():
            with self._lock:
                lst = self._listeners.get(event, [])
                if cb in lst:
                    lst.remove(cb)

        return off

    def on_connect(self, cb: Callable[[Subscriber], None]) -> None:
        """``cb(subscriber)`` runs right after a new SSE client got its 'hello'."""
        with self._lock:
            self._connect_hooks.append(cb)

    def subscribe(self, maxsize: int = 512, remote: str | None = None) -> Subscriber:
        sub = Subscriber(maxsize=maxsize, remote=remote)
        with self._lock:
            if self._closed:
                sub.close()
            else:
                self._subs.add(sub)
        return sub

    def connected(self, sub: Subscriber) -> None:
        """Called by the server after the hello event was written."""
        with self._lock:
            hooks = list(self._connect_hooks)
        for cb in hooks:
            try:
                cb(sub)
            except Exception:
                log.exception("on_connect hook faalde (failed)")

    def unsubscribe(self, sub: Subscriber) -> None:
        with self._lock:
            self._subs.discard(sub)
        sub.close()

    def subscriber_count(self) -> int:
        with self._lock:
            return len(self._subs)

    def close(self) -> None:
        """Disconnect every SSE client (shutdown)."""
        with self._lock:
            self._closed = True
            subs = list(self._subs)
            self._subs.clear()
        for s in subs:
            s.close()


def _short(args: dict, limit: int = 300) -> str:
    """Log form of call args: secrets (passwords, tokens…) are replaced by '***'."""
    try:
        text = json_dumps(redact(args))
    except Exception:
        text = repr(args)
    return text if len(text) <= limit else text[: limit - 3] + "..."
