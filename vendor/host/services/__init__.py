"""Service auto-discovery.

Every module in this package is imported and its ``register(api, ctx)`` called.
Optional module hooks:
  ``PRIORITY = 50``          lower registers earlier (settings/shell always first)
  ``start(api, ctx)``        after ALL services registered and the HTTP server listens
  ``stop(api, ctx)``         on shutdown (reverse order)

One failing service logs a clear error and never breaks the others.
Set ``WIN11SHELL_DISABLE_SERVICES=media,index`` to skip modules (debugging).
"""

from __future__ import annotations

import importlib
import logging
import os
import pkgutil
import time
import traceback
from types import ModuleType

log = logging.getLogger("win11shell.services")

_FIRST = ("settings", "shell")

__all__ = ["discover", "load_services", "start_services", "stop_services"]


def discover(package: str = __name__) -> list[str]:
    pkg = importlib.import_module(package)
    names = [m.name for m in pkgutil.iter_modules(pkg.__path__) if not m.name.startswith("_")]
    return sorted(names)


def _order_key(name: str, mod: ModuleType | None):
    first = _FIRST.index(name) if name in _FIRST else len(_FIRST)
    prio = getattr(mod, "PRIORITY", 50) if mod is not None else 50
    try:
        prio = int(prio)
    except (TypeError, ValueError):
        prio = 50
    return (first, prio, name)


def load_services(api, ctx, package: str = __name__) -> dict[str, ModuleType]:
    """Import + register every service. Returns {name: module} of the ones that loaded."""
    disabled = {s.strip() for s in os.environ.get("WIN11SHELL_DISABLE_SERVICES", "").split(",") if s.strip()}
    imported: dict[str, ModuleType] = {}
    for name in discover(package):
        if name in disabled:
            ctx.services[name] = {"ok": False, "error": "disabled"}
            log.info("Dienst %s uitgeschakeld (service disabled)", name)
            continue
        try:
            imported[name] = importlib.import_module(f"{package}.{name}")
        except Exception as e:  # noqa: BLE001
            ctx.services[name] = {"ok": False, "error": f"import: {type(e).__name__}: {e}"}
            log.error("Dienst '%s' kon niet laden (service failed to import): %s\n%s",
                      name, e, traceback.format_exc())
    loaded: dict[str, ModuleType] = {}
    for name in sorted(imported, key=lambda n: _order_key(n, imported[n])):
        mod = imported[name]
        reg = getattr(mod, "register", None)
        if not callable(reg):
            ctx.services[name] = {"ok": False, "error": "no register(api, ctx)"}
            log.error("Dienst '%s' heeft geen register(api, ctx) (service has no register function)", name)
            continue
        t0 = time.monotonic()
        try:
            reg(api, ctx)
        except Exception as e:  # noqa: BLE001
            ctx.services[name] = {"ok": False, "error": f"{type(e).__name__}: {e}"}
            log.error("Dienst '%s' kon niet starten (service failed to register): %s\n%s",
                      name, e, traceback.format_exc())
            continue
        dt = time.monotonic() - t0
        if dt > 1.0:
            log.warning("Dienst '%s' deed %.1f s over register() (slow register)", name, dt)
        ctx.services[name] = {"ok": True}
        loaded[name] = mod
    ctx._service_modules = loaded
    log.info("Diensten geladen (services loaded): %s", ", ".join(loaded) or "-")
    return loaded


def start_services(api, ctx) -> None:
    for name, mod in getattr(ctx, "_service_modules", {}).items():
        fn = getattr(mod, "start", None)
        if not callable(fn):
            continue
        try:
            fn(api, ctx)
        except Exception as e:  # noqa: BLE001
            ctx.services[name] = {"ok": False, "error": f"start: {type(e).__name__}: {e}"}
            log.error("Dienst '%s' faalde bij start (service start failed): %s\n%s",
                      name, e, traceback.format_exc())


def stop_services(api, ctx) -> None:
    for name, mod in reversed(list(getattr(ctx, "_service_modules", {}).items())):
        fn = getattr(mod, "stop", None)
        if not callable(fn):
            continue
        try:
            fn(api, ctx)
        except Exception:  # noqa: BLE001
            log.exception("Dienst '%s' faalde bij stoppen (service stop failed)", name)
