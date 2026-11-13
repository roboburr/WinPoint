"""media.* (SPEC §6.7): MPRIS media players on the session bus.

* ``media.list {}`` → ``[{id, identity, status, title, artist, album, artUrl, canNext, canPrevious,
  canPlay, canPause}]`` + extras ``artPath`` (local file behind a ``file://`` artUrl, serve it with
  ``bridge.fileUrl``), ``desktopId``, ``icon``, ``length``/``position`` (seconds), ``rate``,
  ``canControl``, ``canSeek``, ``updated`` (ms, for interpolating the position).
  ``status`` = 'playing' | 'paused' | 'stopped'. Playing players come first.
* ``media.control {id, action}`` — action 'playPause' | 'next' | 'previous' (+ 'play', 'pause',
  'stop'). A user UI action (not dangerous) but skipped in --dry-run.
* event ``media.changed {players}`` (PropertiesChanged / NameOwnerChanged, debounced).
"""

from __future__ import annotations

import logging
import os
import threading
import time

from ..util import ApiError, dbus_call, dry_for, json_dumps, session_bus
from .apps import Debouncer, GLib, Gio, ensure_glib_loop, get_catalog, uri_to_path

log = logging.getLogger("win11shell.media")

PRIORITY = 45

PREFIX = "org.mpris.MediaPlayer2."
MPRIS_PATH = "/org/mpris/MediaPlayer2"
ROOT_IFACE = "org.mpris.MediaPlayer2"
PLAYER_IFACE = "org.mpris.MediaPlayer2.Player"
_SKIP = {"org.mpris.MediaPlayer2.playerctld"}   # proxy that mirrors another player

_ACTIONS = {"playPause": "PlayPause", "next": "Next", "previous": "Previous", "play": "Play", "pause": "Pause",
            "stop": "Stop"}


def _get_all(bus, name: str, iface: str) -> dict:
    try:
        r = dbus_call(bus, name, MPRIS_PATH, "org.freedesktop.DBus.Properties", "GetAll", (iface,),
                      signature="(s)", timeout_ms=1200)
    except Exception as e:  # noqa: BLE001
        log.debug("MPRIS %s %s onleesbaar (unreadable): %s", name, iface, e)
        return {}
    return r[0] if r and isinstance(r[0], dict) else {}


def _s(v) -> str:
    if isinstance(v, (list, tuple)):
        return ", ".join(str(x) for x in v if x)
    return str(v) if v is not None else ""


def player_dict(name: str, root: dict, player: dict, catalog=None) -> dict:
    md = player.get("Metadata") or {}
    if not isinstance(md, dict):
        md = {}
    status = str(player.get("PlaybackStatus") or "Stopped").lower()
    if status not in ("playing", "paused", "stopped"):
        status = "stopped"
    art = _s(md.get("mpris:artUrl"))
    art_path = None
    if art.startswith("file://"):
        p = uri_to_path(art)
        art_path = p if p and os.path.isfile(p) else None
    length = md.get("mpris:length")
    try:
        length = round(int(length) / 1_000_000, 3) if length else None
    except (TypeError, ValueError):
        length = None
    pos = player.get("Position")
    try:
        pos = round(int(pos) / 1_000_000, 3) if pos is not None else None
    except (TypeError, ValueError):
        pos = None
    identity = _s(root.get("Identity")) or name[len(PREFIX):].split(".")[0].capitalize()
    desktop = _s(root.get("DesktopEntry"))
    desktop_id = (desktop if desktop.endswith(".desktop") else desktop + ".desktop") if desktop else None
    icon = None
    if catalog is not None:
        b = catalog.brief(desktop_id) if desktop_id else None
        if b is None:
            did, ic = catalog.match_window(name[len(PREFIX):].split(".")[0], desktop or None)
            desktop_id = desktop_id or did
            icon = ic
        else:
            icon = b.get("icon")
    can_control = bool(player.get("CanControl", True))
    return {
        "id": name,
        "identity": identity,
        "status": status,
        "title": _s(md.get("xesam:title")),
        "artist": _s(md.get("xesam:artist")),
        "album": _s(md.get("xesam:album")),
        "artUrl": art or None,
        "artPath": art_path,
        "canNext": bool(player.get("CanGoNext")) and can_control,
        "canPrevious": bool(player.get("CanGoPrevious")) and can_control,
        "canPlay": bool(player.get("CanPlay")) and can_control,
        "canPause": bool(player.get("CanPause")) and can_control,
        "canSeek": bool(player.get("CanSeek")) and can_control,
        "canControl": can_control,
        "length": length,
        "position": pos,
        "rate": float(player.get("Rate") or 1.0) if isinstance(player.get("Rate"), (int, float)) else 1.0,
        "desktopId": desktop_id,
        "icon": icon or "multimedia-player",
        "updated": int(time.time() * 1000),
    }


class MediaService:
    def __init__(self, api, ctx):
        self.api = api
        self.ctx = ctx
        self.bus = None
        self.subs: list[int] = []
        self.lock = threading.Lock()
        self.last = ""
        self.deb = Debouncer(0.25, self.refresh, name="media-changed")
        self.catalog = get_catalog(ctx)

    def _bus(self):
        if self.bus is None and Gio is not None:
            try:
                self.bus = session_bus()
            except Exception as e:  # noqa: BLE001
                log.debug("Geen sessiebus (no session bus): %s", e)
                self.bus = None
        return self.bus

    def names(self) -> list[str]:
        bus = self._bus()
        if bus is None:
            return []
        try:
            r = dbus_call(bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                          "ListNames", timeout_ms=1500)
        except Exception as e:  # noqa: BLE001
            log.debug("ListNames mislukt (failed): %s", e)
            return []
        return sorted(n for n in (r[0] if r else []) if n.startswith(PREFIX) and n not in _SKIP)

    def players(self) -> list[dict]:
        bus = self._bus()
        if bus is None:
            return []
        out = []
        for name in self.names():
            player = _get_all(bus, name, PLAYER_IFACE)
            if not player:
                continue
            root = _get_all(bus, name, ROOT_IFACE)
            out.append(player_dict(name, root, player, self.catalog))
        order = {"playing": 0, "paused": 1, "stopped": 2}
        out.sort(key=lambda p: (order.get(p["status"], 3), p["identity"].lower()))
        return out

    def refresh(self) -> None:
        players = self.players()
        key = json_dumps([{k: v for k, v in p.items() if k not in ("updated", "position")} for p in players])
        with self.lock:
            if key == self.last:
                return
            self.last = key
        self.api.emit("media.changed", {"players": players})

    def subscribe(self) -> None:
        bus = self._bus()
        if bus is None:
            return

        def on_signal(*_a):
            self.deb.trigger()

        try:
            self.subs.append(bus.signal_subscribe(None, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                                                  MPRIS_PATH, None, Gio.DBusSignalFlags.NONE, on_signal))
            self.subs.append(bus.signal_subscribe("org.freedesktop.DBus", "org.freedesktop.DBus", "NameOwnerChanged",
                                                  "/org/freedesktop/DBus", "org.mpris.MediaPlayer2",
                                                  Gio.DBusSignalFlags.MATCH_ARG0_NAMESPACE, on_signal))
            self.subs.append(bus.signal_subscribe(None, PLAYER_IFACE, "Seeked", MPRIS_PATH, None,
                                                  Gio.DBusSignalFlags.NONE, on_signal))
        except Exception:  # noqa: BLE001
            log.exception("MPRIS-signalen niet beschikbaar (cannot subscribe)")

    def unsubscribe(self) -> None:
        bus = self.bus
        for sid in self.subs:
            try:
                bus.signal_unsubscribe(sid)
            except Exception:  # noqa: BLE001
                pass
        self.subs = []
        self.deb.close()

    def control(self, pid: str, action: str) -> dict:
        method = _ACTIONS.get(action)
        if method is None:
            raise ApiError("bad_args", "Onbekende bediening.")
        if not isinstance(pid, str) or not pid.startswith(PREFIX):
            raise ApiError("bad_args", "Onbekende speler.")
        if pid not in self.names():
            raise ApiError("not_found", "Deze speler is gesloten.")
        if dry_for(self.ctx):
            log.info("[dry-run] media.control %s %s overgeslagen (skipped)", pid, action)
            return {"ok": True, "dryRun": True}
        try:
            dbus_call(self._bus(), pid, MPRIS_PATH, PLAYER_IFACE, method, timeout_ms=2000)
        except Exception as e:  # noqa: BLE001
            log.info("MPRIS %s %s mislukt (failed): %s", pid, method, e)
            raise ApiError("failed", "De speler reageert niet.") from None
        self.deb.trigger()
        return {"ok": True}


def register(api, ctx) -> None:
    svc = MediaService(api, ctx)
    ctx._media_service = svc

    @api.method("media.list")
    def _list(args):
        """MPRIS players (playing first)."""
        return svc.players()

    @api.method("media.control")
    def _control(args):
        """Play/pause, next, previous (skipped in --dry-run)."""
        return svc.control(args.get("id"), args.get("action"))


def start(api, ctx) -> None:
    svc = getattr(ctx, "_media_service", None)
    if svc is not None:
        svc.subscribe()
        ensure_glib_loop(ctx)


def stop(api, ctx) -> None:
    svc = getattr(ctx, "_media_service", None)
    if svc is not None:
        svc.unsubscribe()
