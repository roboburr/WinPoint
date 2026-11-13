"""display.* (SPEC §6.8): monitors via Hyprland ``j/monitors``; changes via ``keyword monitor``.

* ``display.list {}`` → ``{supported, monitors:[{name, description, width, height, refresh, scale, x, y,
  focused, modes:["2560x1440@144"], …extras}]}``. Extras: ``mode`` (current, same notation),
  ``make``, ``model``, ``transform``, ``vrr``, ``disabled``, ``logicalWidth``, ``logicalHeight``,
  ``readOnly``.
  Under GNOME the list is read (read-only) from ``org.gnome.Mutter.DisplayConfig`` with
  ``supported:false`` so the Settings app can still show what is connected.
* ``display.set {name, mode?, scale?}`` (dangerous) → ``keyword monitor NAME,MODE,POS,SCALE``
  (POS = the monitor's current position so a multi-monitor layout is not reshuffled).
  Returns ``{ok, previous:{mode, scale}}`` so the UI can offer "Wijzigingen behouden?" and revert.
  The applied value is also written to ``<config>/hyprland-monitors.conf`` (``monitor = …`` lines)
  so it survives a restart when hyprland.conf sources that file.
* event ``display.changed {monitors}`` on Hyprland monitor events (debounced).
"""

from __future__ import annotations

import logging
import re
import threading

from ..util import ApiError, atomic_write_text, dbus_call, session_bus
from .apps import Debouncer
from .windows import Hypr, logical_monitor

log = logging.getLogger("win11shell.display")

PRIORITY = 40

_MODE_RE = re.compile(r"^\s*(\d{2,5})x(\d{2,5})(?:@(\d+(?:\.\d+)?)(?:Hz)?)?\s*$", re.I)
_NAME_RE = re.compile(r"^[A-Za-z0-9_.:-]{1,64}$")


def _fmt_rate(hz: float) -> str:
    """143.99 → '143.99', 60.0 → '60', 59.951 → '59.95'."""
    s = f"{float(hz):.2f}".rstrip("0").rstrip(".")
    return s or "0"


def norm_mode(text: str) -> str | None:
    m = _MODE_RE.match(str(text or ""))
    if not m:
        return None
    w, h, r = m.group(1), m.group(2), m.group(3)
    return f"{w}x{h}@{_fmt_rate(float(r))}" if r else f"{w}x{h}"


def _mode_key(mode: str) -> tuple:
    m = _MODE_RE.match(mode)
    if not m:
        return (0, 0, 0.0)
    return (int(m.group(1)) * int(m.group(2)), int(m.group(1)), float(m.group(3) or 0))


def monitor_dict(m: dict) -> dict:
    modes = []
    for s in m.get("availableModes") or []:
        n = norm_mode(s)
        if n and n not in modes:
            modes.append(n)
    modes.sort(key=_mode_key, reverse=True)
    w, h = int(m.get("width") or 0), int(m.get("height") or 0)
    rate = float(m.get("refreshRate") or 0)
    cur = f"{w}x{h}@{_fmt_rate(rate)}" if w and h else ""
    if cur and cur not in modes:
        modes.insert(0, cur)
    lm = logical_monitor(m)
    return {
        "name": m.get("name") or "",
        "description": (m.get("description") or "").strip(),
        "make": m.get("make") or "",
        "model": m.get("model") or "",
        "width": w, "height": h,
        "refresh": round(rate, 2),
        "scale": round(float(m.get("scale") or 1.0), 4),
        "x": int(m.get("x") or 0), "y": int(m.get("y") or 0),
        "logicalWidth": lm["w"], "logicalHeight": lm["h"],
        "focused": bool(m.get("focused")),
        "transform": int(m.get("transform") or 0),
        "vrr": bool(m.get("vrr")),
        "disabled": bool(m.get("disabled")),
        "mode": cur,
        "modes": modes,
        "readOnly": False,
    }


# -- GNOME (read-only) --------------------------------------------------------

def gnome_monitors() -> list[dict]:
    """Read the current layout from Mutter (no changes ever)."""
    try:
        bus = session_bus()
        r = dbus_call(bus, "org.gnome.Mutter.DisplayConfig", "/org/gnome/Mutter/DisplayConfig",
                      "org.gnome.Mutter.DisplayConfig", "GetCurrentState", timeout_ms=2000)
    except Exception as e:  # noqa: BLE001
        log.debug("Mutter DisplayConfig niet beschikbaar (unavailable): %s", e)
        return []
    if not r or len(r) < 3:
        return []
    _serial, monitors, logical = r[0], r[1], r[2]
    placement: dict[tuple, dict] = {}
    for lm in logical or []:
        try:
            x, y, scale, _transform, primary, mons = lm[0], lm[1], lm[2], lm[3], lm[4], lm[5]
        except (IndexError, TypeError):
            continue
        for spec in mons or []:
            placement[tuple(spec)] = {"x": x, "y": y, "scale": scale, "primary": primary}
    out = []
    for mon in monitors or []:
        try:
            spec, modes, props = mon[0], mon[1], mon[2]
        except (IndexError, TypeError):
            continue
        connector = spec[0] if spec else ""
        cur = None
        mode_list = []
        for md in modes or []:
            try:
                _id, w, h, rate, _pref, _scales, mprops = md[0], md[1], md[2], md[3], md[4], md[5], md[6]
            except (IndexError, TypeError):
                continue
            s = f"{w}x{h}@{_fmt_rate(rate)}"
            if s not in mode_list:
                mode_list.append(s)
            if (mprops or {}).get("is-current"):
                cur = (w, h, rate)
        mode_list.sort(key=_mode_key, reverse=True)
        pl = placement.get(tuple(spec), {})
        w, h, rate = cur if cur else (0, 0, 0.0)
        name = (props or {}).get("display-name") or connector
        out.append({
            "name": connector, "description": name, "make": spec[1] if len(spec) > 1 else "",
            "model": spec[2] if len(spec) > 2 else "", "width": w, "height": h, "refresh": round(rate, 2),
            "scale": round(float(pl.get("scale", 1.0)), 4), "x": pl.get("x", 0), "y": pl.get("y", 0),
            "logicalWidth": int(round(w / (pl.get("scale") or 1.0))) if w else 0,
            "logicalHeight": int(round(h / (pl.get("scale") or 1.0))) if h else 0,
            "focused": bool(pl.get("primary")), "transform": 0, "vrr": False, "disabled": not cur,
            "mode": f"{w}x{h}@{_fmt_rate(rate)}" if cur else "", "modes": mode_list, "readOnly": True,
        })
    return out


# -- service ------------------------------------------------------------------

def register(api, ctx) -> None:
    hypr = Hypr()
    lock = threading.Lock()
    state = {"last": None}

    def hypr_monitors() -> list[dict] | None:
        mons = hypr.json("monitors all") or hypr.json("monitors")
        if not isinstance(mons, list):
            return None
        return [monitor_dict(m) for m in mons if isinstance(m, dict)]

    def emit_changed():
        mons = hypr_monitors()
        if mons is None:
            return
        with lock:
            if mons == state["last"]:
                return
            state["last"] = mons
        api.emit("display.changed", {"monitors": mons})

    deb = Debouncer(0.5, emit_changed, name="display-changed")
    ctx._display_debounce = deb

    # windows.py owns the Hyprland event socket and forwards raw events to listeners.
    def on_hypr_event(name: str, _data: str):
        if name.startswith(("monitor", "configreloaded")):
            deb.trigger()

    svc = getattr(ctx, "_windows_service", None)
    if svc is not None:
        svc.event_listeners.append(on_hypr_event)

    @api.method("display.list")
    def _list(args):
        """Monitors: Hyprland (changeable) or GNOME (read-only)."""
        if hypr.available():
            mons = hypr_monitors()
            if mons is not None:
                with lock:
                    state["last"] = mons
                return {"supported": True, "monitors": mons}
        if ctx.compositor == "gnome":
            return {"supported": False, "monitors": gnome_monitors()}
        return {"supported": False, "monitors": []}

    def _dry(args):
        return {"previous": None}

    @api.method("display.set", dangerous=True, dry_result=_dry)
    def _set(args):
        """Change resolution/refresh (mode 'WxH@Hz') and/or scale of one monitor (Hyprland)."""
        name = args.get("name")
        if not isinstance(name, str) or not _NAME_RE.match(name):
            raise ApiError("bad_args", "Onbekend beeldscherm.")
        if not hypr.available():
            raise ApiError("unsupported", "Beeldscherminstellingen wijzigen kan alleen in de Win11-sessie.")
        mons = hypr.json("monitors all") or hypr.json("monitors") or []
        raw = next((m for m in mons if isinstance(m, dict) and m.get("name") == name), None)
        if raw is None:
            raise ApiError("not_found", "Dit beeldscherm is niet (meer) aangesloten.")
        cur = monitor_dict(raw)
        mode = args.get("mode")
        if mode in (None, ""):
            mode_s = cur["mode"] or "preferred"
        elif mode in ("preferred", "highrr", "highres"):
            mode_s = mode
        else:
            mode_s = norm_mode(mode)
            if not mode_s:
                raise ApiError("bad_args", "Ongeldige resolutie.")
            if cur["modes"] and mode_s not in cur["modes"]:
                # accept "2560x1440@144" for "2560x1440@143.99"
                want = _mode_key(mode_s)
                close = [m for m in cur["modes"] if _mode_key(m)[:2] == want[:2]
                         and (not want[2] or abs(_mode_key(m)[2] - want[2]) < 0.6)]
                if not close:
                    raise ApiError("bad_args", "Deze resolutie wordt niet ondersteund door dit beeldscherm.")
                mode_s = close[0]
        scale = args.get("scale")
        if scale in (None, ""):
            scale_v = cur["scale"]
        else:
            try:
                scale_v = float(scale)
            except (TypeError, ValueError):
                raise ApiError("bad_args", "Ongeldige schaal.") from None
            if not 0.5 <= scale_v <= 4.0:
                raise ApiError("bad_args", "Kies een schaal tussen 50 en 400 procent.")
        scale_s = f"{scale_v:.4f}".rstrip("0").rstrip(".")
        pos = f"{cur['x']}x{cur['y']}"
        value = f"{name},{mode_s},{pos},{scale_s}"
        if not hypr.batch([f"keyword monitor {value}"]):
            raise ApiError("failed", "Het beeldscherm kon niet worden aangepast.")
        log.info("Beeldscherm ingesteld (monitor set): %s", value)
        _persist(ctx, name, value)
        deb.trigger()
        return {"ok": True, "previous": {"mode": cur["mode"], "scale": cur["scale"]}, "applied": value}


def _persist(ctx, name: str, value: str) -> None:
    path = ctx.config_dir / "hyprland-monitors.conf"
    lines = []
    try:
        for line in path.read_text(encoding="utf-8").splitlines():
            s = line.strip()
            if s.startswith("monitor") and "=" in s and s.split("=", 1)[1].strip().split(",", 1)[0] == name:
                continue
            if s:
                lines.append(line)
    except OSError:
        pass
    if not lines or not lines[0].startswith("#"):
        lines.insert(0, "# Win11 Shell: beeldscherminstellingen (automatisch bijgewerkt door Instellingen)")
    lines.append(f"monitor = {value}")
    try:
        atomic_write_text(path, "\n".join(lines) + "\n", mode=0o644)
    except OSError as e:
        log.warning("Kan %s niet opslaan (cannot save): %s", path, e)


def stop(api, ctx) -> None:
    deb = getattr(ctx, "_display_debounce", None)
    if deb is not None:
        deb.close()
