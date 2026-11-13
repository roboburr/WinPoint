"""notifications.* (SPEC §6.6): our own org.freedesktop.Notifications server.

* The name is owned on the session bus **only if it is free** (checked with
  ``org.freedesktop.DBus.NameHasOwner`` first, then ``Gio.bus_own_name_on_connection`` with
  DO_NOT_QUEUE) **and** only when Win11 Shell is the desktop: ``layer`` mode, or ``preview``/``serve``
  inside a Hyprland session. Never in ``app`` mode and never on GNOME (there the name can be free
  for a while; taking it would hide the user's notifications) → ``supported:false``.
  ``WIN11SHELL_NOTIFICATIONS=own|off`` overrides this (tests / debugging).
* Implements Notify / CloseNotification / GetCapabilities / GetServerInformation and emits
  NotificationClosed (1 expired, 2 dismissed, 3 closed by app) and ActionInvoked.
* History: newest first, max 100. Do-not-disturb (``settings.notifications.dnd``): still stored,
  marked ``silent:true`` (the UI then shows no toast/sound).
* Item = {id, appName, appIcon, summary, body, actions:[{key,label}], time, urgency, image}
  + extras ``hasDefault`` (a "default" action exists: clicking the toast invokes it),
  ``silent``, ``transient``, ``resident``, ``category``, ``value`` (progress 0–100), ``desktopId``,
  ``source`` ('dbus'|'test'). ``urgency`` = 'low'|'normal'|'critical'. ``image`` = PNG data URL,
  absolute image path or themed icon name (or null). The body is plain text (markup stripped).
* ``notifications.test`` works even when unsupported.
"""

from __future__ import annotations

import base64
import html
import logging
import os
import re
import threading
import time
from collections import OrderedDict
from typing import Any

from .. import VERSION
from ..util import ApiError, dbus_name_has_owner, session_bus
from .apps import GLib, Gio, ensure_glib_loop, get_catalog, uri_to_path

log = logging.getLogger("win11shell.notifications")

PRIORITY = 40

NAME = "org.freedesktop.Notifications"
PATH = "/org/freedesktop/Notifications"
IFACE = "org.freedesktop.Notifications"
MAX_ITEMS = 100

INTROSPECTION = """
<node>
  <interface name="org.freedesktop.Notifications">
    <method name="GetCapabilities">
      <arg direction="out" type="as" name="capabilities"/>
    </method>
    <method name="Notify">
      <arg direction="in" type="s" name="app_name"/>
      <arg direction="in" type="u" name="replaces_id"/>
      <arg direction="in" type="s" name="app_icon"/>
      <arg direction="in" type="s" name="summary"/>
      <arg direction="in" type="s" name="body"/>
      <arg direction="in" type="as" name="actions"/>
      <arg direction="in" type="a{sv}" name="hints"/>
      <arg direction="in" type="i" name="expire_timeout"/>
      <arg direction="out" type="u" name="id"/>
    </method>
    <method name="CloseNotification">
      <arg direction="in" type="u" name="id"/>
    </method>
    <method name="GetServerInformation">
      <arg direction="out" type="s" name="name"/>
      <arg direction="out" type="s" name="vendor"/>
      <arg direction="out" type="s" name="version"/>
      <arg direction="out" type="s" name="spec_version"/>
    </method>
    <signal name="NotificationClosed">
      <arg type="u" name="id"/>
      <arg type="u" name="reason"/>
    </signal>
    <signal name="ActionInvoked">
      <arg type="u" name="id"/>
      <arg type="s" name="action_key"/>
    </signal>
  </interface>
</node>
"""

CAPABILITIES = ["actions", "body", "body-markup", "icon-static", "persistence"]

_URGENCY = {0: "low", 1: "normal", 2: "critical"}
_TAG_RE = re.compile(r"<[^>]{0,500}>")
_BR_RE = re.compile(r"<\s*br\s*/?\s*>", re.I)


def clean_markup(text: str, limit: int) -> str:
    """Notification body markup (b, i, u, a, img, br) → plain text."""
    t = str(text or "")[: limit * 4]
    t = _BR_RE.sub("\n", t)
    t = _TAG_RE.sub("", t)
    t = html.unescape(t)
    t = re.sub(r"[ \t]+\n", "\n", t).strip()
    return t[:limit]


def _image_data_url(v) -> str | None:
    """(iiibiiay) image-data variant → PNG data URL (scaled to ≤ 128 px)."""
    try:
        import gi
        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf
        w = v.get_child_value(0).get_int32()
        h = v.get_child_value(1).get_int32()
        rowstride = v.get_child_value(2).get_int32()
        has_alpha = v.get_child_value(3).get_boolean()
        bps = v.get_child_value(4).get_int32()
        data = v.get_child_value(6).get_data_as_bytes()
        if w <= 0 or h <= 0 or w > 4096 or h > 4096 or data.get_size() < rowstride * (h - 1) + w:
            return None
        pb = GdkPixbuf.Pixbuf.new_from_bytes(data, GdkPixbuf.Colorspace.RGB, has_alpha, bps, w, h, rowstride)
        m = max(w, h)
        if m > 128:
            pb = pb.scale_simple(max(1, w * 128 // m), max(1, h * 128 // m), GdkPixbuf.InterpType.BILINEAR)
        ok, buf = pb.save_to_bufferv("png", [], [])
        if not ok:
            return None
        return "data:image/png;base64," + base64.b64encode(bytes(buf)).decode("ascii")
    except Exception as e:  # noqa: BLE001
        log.debug("Afbeelding in melding onleesbaar (image-data unreadable): %s", e)
        return None


def _icon_ref(value: str) -> str | None:
    """app_icon / image-path → absolute path (file:// converted) or themed icon name."""
    v = (value or "").strip()
    if not v:
        return None
    if v.startswith("file://"):
        v = uri_to_path(v) or ""
    if v.startswith("/"):
        return v if os.path.isfile(v) else None
    if re.match(r"^[A-Za-z0-9_.+-]{1,200}$", v):
        return v
    return None


class NotificationStore:
    def __init__(self, api, ctx):
        self.api = api
        self.ctx = ctx
        self.lock = threading.RLock()
        self.items: OrderedDict[int, dict] = OrderedDict()
        self.next_id = 1
        self.supported = False
        self.bus = None
        self.reg_id = 0
        self.owner_id = 0
        self.timers: dict[int, threading.Timer] = {}

    # -- store ------------------------------------------------------------------
    def _new_id(self) -> int:
        with self.lock:
            nid = self.next_id
            self.next_id = self.next_id + 1 if self.next_id < 0xFFFFFFF0 else 1
            return nid

    def add(self, n: dict, replaces: int = 0) -> dict:
        with self.lock:
            if replaces and replaces in self.items:
                n["id"] = replaces
                self.items.pop(replaces)
                n["replaced"] = True
            elif not n.get("id"):
                n["id"] = self._new_id()
            self.items[n["id"]] = n
            while len(self.items) > MAX_ITEMS:
                old_id, _old = self.items.popitem(last=False)
                self._cancel_timer(old_id)
        self.api.emit("notifications.new", {"notification": dict(n)})
        if n.get("transient") or n.get("expireAfter"):
            delay = n.get("expireAfter") or 6.0
            t = threading.Timer(delay, self.close, args=(n["id"], 1))
            t.daemon = True
            with self.lock:
                self._cancel_timer(n["id"])
                self.timers[n["id"]] = t
            t.start()
        return n

    def _cancel_timer(self, nid: int) -> None:
        t = self.timers.pop(nid, None)
        if t is not None:
            t.cancel()

    def close(self, nid: int, reason: int) -> bool:
        with self.lock:
            n = self.items.pop(nid, None)
            self._cancel_timer(nid)
        if n is None:
            return False
        if n.get("source") == "dbus":
            self.signal("NotificationClosed", "(uu)", (nid, reason))
        self.api.emit("notifications.closed", {"id": nid, "reason": reason})
        return True

    def list(self) -> list[dict]:
        with self.lock:
            return [dict(n) for n in reversed(self.items.values())]

    def dnd(self) -> bool:
        try:
            return bool(self.ctx.settings.get("notifications.dnd", False))
        except Exception:  # noqa: BLE001
            return False

    # -- D-Bus --------------------------------------------------------------------
    def signal(self, name: str, sig: str, values: tuple) -> None:
        bus = self.bus
        if bus is None or not self.supported:
            return
        try:
            bus.emit_signal(None, PATH, IFACE, name, GLib.Variant(sig, values))
        except Exception as e:  # noqa: BLE001
            log.debug("Signaal %s mislukt (signal failed): %s", name, e)

    def should_own(self) -> bool:
        force = os.environ.get("WIN11SHELL_NOTIFICATIONS", "").strip().lower()
        if force in ("own", "on", "1"):
            return True
        if force in ("off", "0", "no"):
            return False
        if self.ctx.mode == "layer":
            return True
        return self.ctx.mode in ("preview", "serve") and self.ctx.compositor == "hyprland"

    def start_dbus(self) -> None:
        if Gio is None:
            return
        if not self.should_own():
            log.info("Meldingendienst niet gestart: Win11 Shell is hier niet het bureaublad "
                     "(notification server not started in %s mode on %s)", self.ctx.mode, self.ctx.compositor)
            return
        try:
            bus = session_bus()
        except Exception as e:  # noqa: BLE001
            log.info("Geen sessiebus (no session bus): %s", e)
            return
        if bus is None:
            return
        if dbus_name_has_owner(bus, NAME):
            log.info("Een andere meldingendienst is actief; Win11 Shell toont alleen eigen meldingen "
                     "(another notification server owns %s)", NAME)
            return
        try:
            node = Gio.DBusNodeInfo.new_for_xml(INTROSPECTION)
            iface = node.interfaces[0]
            reg = getattr(bus, "register_object_with_closures2", None) or bus.register_object
            self.reg_id = reg(PATH, iface, self._on_call, None, None)
        except Exception:  # noqa: BLE001
            log.exception("Meldingendienst registreren mislukt (cannot register object)")
            return
        self.bus = bus

        def acquired(_conn, _name):
            self.supported = True
            log.info("Meldingendienst actief (notification server owns %s)", NAME)
            self.api.emit("notifications.status", {"supported": True})

        def lost(_conn, _name):
            was = self.supported
            self.supported = False
            if was:
                log.warning("Meldingendienst overgenomen door een andere app (lost %s)", NAME)
            else:
                log.info("%s is bezet (name taken)", NAME)
            self.api.emit("notifications.status", {"supported": False})

        self.owner_id = Gio.bus_own_name_on_connection(bus, NAME, Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
                                                       acquired, lost)

    def stop_dbus(self) -> None:
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
        with self.lock:
            for t in self.timers.values():
                t.cancel()
            self.timers.clear()

    def _on_call(self, conn, sender, path, iface, method, params, invocation):
        try:
            if method == "GetCapabilities":
                invocation.return_value(GLib.Variant("(as)", (CAPABILITIES,)))
            elif method == "GetServerInformation":
                invocation.return_value(GLib.Variant("(ssss)", ("Win11 Shell", "Win11 Shell", VERSION, "1.2")))
            elif method == "Notify":
                nid = self._notify(params, sender)
                invocation.return_value(GLib.Variant("(u)", (nid,)))
            elif method == "CloseNotification":
                nid = params.get_child_value(0).get_uint32()
                self.close(nid, 3)
                invocation.return_value(None)
            else:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "Unknown method")
        except Exception as e:  # noqa: BLE001
            log.exception("Melding verwerken mislukt (notification call failed)")
            try:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", str(e)[:200])
            except Exception:  # noqa: BLE001
                pass

    def _notify(self, params, sender: str | None) -> int:
        app_name = params.get_child_value(0).get_string()
        replaces = params.get_child_value(1).get_uint32()
        app_icon = params.get_child_value(2).get_string()
        summary = params.get_child_value(3).get_string()
        body = params.get_child_value(4).get_string()
        actions_v = params.get_child_value(5).unpack() or []
        hints_v = params.get_child_value(6)
        expire = params.get_child_value(7).get_int32()

        def hint(*names):
            for nm in names:
                v = hints_v.lookup_value(nm, None)
                if v is not None:
                    return v
            return None

        def hint_py(*names, default=None):
            v = hint(*names)
            if v is None:
                return default
            try:
                return v.unpack()
            except Exception:  # noqa: BLE001
                return default

        urgency = hint_py("urgency", default=1)
        try:
            urgency = int(urgency) if not isinstance(urgency, (bytes, str)) else int(ord(urgency[:1] or b"\x01"))
        except (TypeError, ValueError):
            urgency = 1
        image = None
        img_v = hint("image-data", "image_data", "icon_data")
        if img_v is not None and img_v.get_type_string() == "(iiibiiay)":
            image = _image_data_url(img_v)
        if image is None:
            ip = hint_py("image-path", "image_path")
            if isinstance(ip, str):
                image = _icon_ref(ip)
        desktop = hint_py("desktop-entry")
        desktop_id = None
        app_info = None
        if isinstance(desktop, str) and desktop:
            desktop_id = desktop if desktop.endswith(".desktop") else desktop + ".desktop"
            app_info = get_catalog(self.ctx).brief(desktop_id)
        icon = _icon_ref(app_icon)
        if icon and icon.startswith("/") and image is None:
            image = icon               # many apps pass a photo/avatar as app_icon
        if not icon or icon.startswith("/"):
            icon = (app_info or {}).get("icon") or icon or "preferences-system-notifications"
        actions = []
        has_default = False
        for i in range(0, len(actions_v) - 1, 2):
            key, label = str(actions_v[i]), str(actions_v[i + 1])
            if key == "default":
                has_default = True
                continue
            if len(actions) < 3:
                actions.append({"key": key[:100], "label": label[:60] or key[:60]})
        value = hint_py("value")
        n = {
            "id": 0,
            "appName": (app_name or (app_info or {}).get("name") or "").strip()[:100] or "Melding",
            "appIcon": icon,
            "summary": clean_markup(summary, 300),
            "body": clean_markup(body, 3000),
            "actions": actions,
            "hasDefault": has_default,
            "time": int(time.time() * 1000),
            "urgency": _URGENCY.get(urgency, "normal"),
            "image": image,
            "desktopId": desktop_id,
            "category": hint_py("category") if isinstance(hint_py("category"), str) else None,
            "transient": bool(hint_py("transient", default=False)),
            "resident": bool(hint_py("resident", default=False)),
            "value": int(value) if isinstance(value, int) and 0 <= value <= 100 else None,
            "expireTimeout": expire,
            "silent": self.dnd(),
            "source": "dbus",
            "sender": sender,
        }
        if n["transient"]:
            n["expireAfter"] = (expire / 1000.0) if expire and expire > 0 else 6.0
        return self.add(n, replaces)["id"]

    # -- API helpers ----------------------------------------------------------------
    def invoke(self, nid: int, key: str) -> bool:
        with self.lock:
            n = self.items.get(nid)
        if n is None:
            return False
        if n.get("source") == "dbus":
            self.signal("ActionInvoked", "(us)", (nid, key))
        self.api.emit("notifications.action", {"id": nid, "key": key})
        if not n.get("resident"):
            self.close(nid, 2)
        return True


def register(api, ctx) -> None:
    store = NotificationStore(api, ctx)
    ctx._notifications = store

    def nid_of(args) -> int:
        try:
            nid = int(args.get("id"))
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Ongeldige melding.") from None
        return nid

    @api.method("notifications.list")
    def _list(args):
        """{supported, dnd, items} (newest first)."""
        return {"supported": store.supported, "dnd": store.dnd(), "items": store.list()}

    @api.method("notifications.dismiss")
    def _dismiss(args):
        """Remove one notification (apps are told it was dismissed)."""
        return {"ok": store.close(nid_of(args), 2)}

    @api.method("notifications.clear")
    def _clear(args):
        """Remove every notification."""
        ids = [n["id"] for n in store.list()]
        for nid in ids:
            store.close(nid, 2)
        return {"cleared": len(ids)}

    @api.method("notifications.invokeAction")
    def _invoke(args):
        """Run a notification button (or ``default`` = the toast itself was clicked)."""
        key = args.get("key") or "default"
        if not isinstance(key, str) or len(key) > 200:
            raise ApiError("bad_args", "Ongeldige actie.")
        if not store.invoke(nid_of(args), key):
            raise ApiError("not_found", "Deze melding bestaat niet meer.")
        return {}

    @api.method("notifications.test")
    def _test(args):
        """Show a sample notification (also when another notification server is active)."""
        nl = (ctx.settings.get("language", "nl") or "nl") == "nl"
        n = {
            "id": 0,
            "appName": "Win11 Shell",
            "appIcon": "preferences-system-notifications",
            "summary": "Zo ziet een melding eruit" if nl else "This is what a notification looks like",
            "body": ("Meldingen van je apps verschijnen hier en blijven bewaard in het meldingencentrum."
                     if nl else "Notifications from your apps appear here and stay in the notification centre."),
            "actions": [{"key": "ok", "label": "Prima" if nl else "Got it"}],
            "hasDefault": True,
            "time": int(time.time() * 1000),
            "urgency": "normal",
            "image": None,
            "desktopId": "win11shell-settings.desktop",
            "category": None,
            "transient": False,
            "resident": False,
            "value": None,
            "expireTimeout": -1,
            "silent": store.dnd(),
            "source": "test",
        }
        return {"notification": store.add(n)}


def start(api, ctx) -> None:
    store = getattr(ctx, "_notifications", None)
    if store is None:
        return
    store.start_dbus()
    ensure_glib_loop(ctx)


def stop(api, ctx) -> None:
    store = getattr(ctx, "_notifications", None)
    if store is not None:
        store.stop_dbus()
