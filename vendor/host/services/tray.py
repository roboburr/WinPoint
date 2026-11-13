"""tray.* (SPEC §6.15): the notification area (system tray) — StatusNotifierItem + com.canonical.dbusmenu.

Apps that live in the background (Discord, Steam, Telegram, Signal, Nextcloud, blueman …) publish a
StatusNotifierItem (SNI) object on the session bus; the taskbar shows them left of the quick-settings
button and in the ``^`` overflow. XEmbed (the legacy X11 tray) is not supported.

Roles (``tray.list().role``):
* ``watcher`` ``org.kde.StatusNotifierWatcher`` was free: we own it (plus ``org.freedesktop.
              StatusNotifierWatcher`` when that is free too; one object ``/StatusNotifierWatcher`` that
              also answers under the freedesktop interface name) and apps register with us.
* ``host``    another process is the watcher (GNOME AppIndicator extension, KDE kded, a second Win11
              Shell): we own ``org.kde.StatusNotifierHost-<pid>``, call ``RegisterStatusNotifierHost``,
              read ``RegisteredStatusNotifierItems`` and follow its Registered/Unregistered signals.
              When that watcher goes away we take over (and follow a new one when it appears).
              ``app`` mode never takes the watcher: a short-lived app window must not collect the apps'
              icons. ``WIN11SHELL_TRAY=off|host|watcher`` overrides (tests / debugging).
* ``none``    no Gio, no session bus or ``WIN11SHELL_TRAY=off`` → ``supported:false``.

Items: id = ``<unique bus name><object path>``. Registration with a bus name → path ``/StatusNotifierItem``
(well-known names are resolved to their unique owner); with an object path (libayatana-appindicator) →
the bus name is the caller. At start and when becoming the watcher, bus names
``org.kde.StatusNotifierItem-*`` / ``org.freedesktop.StatusNotifierItem-*`` are adopted (apps that
registered with a previous watcher). An item disappears when its bus name loses its owner. Properties
come from ``GetAll`` on ``org.kde.StatusNotifierItem`` (fallback ``org.freedesktop.StatusNotifierItem``)
and are re-read, coalesced, on the ``New*`` signals.

Threads: D-Bus calls made in the GLib loop thread are all async; the ``tray.*`` handlers (HTTP threads)
use ``call_sync`` with ≤ 2 s timeouts. Disk and catalog work (pixmap → PNG, desktop entry lookup, the
``taskbar.tray.known`` list) runs on one worker thread, so the loop (= the GTK surfaces) never waits for
it. Shared state sits behind ``self.lock``. ``tray.changed {items}`` is coalesced (100 ms GLib timeout)
and only sent when the list really changed.

Buggy or hostile items: ≤ 64 items (≤ 16 per connection); pixmaps only with 1 ≤ w,h ≤ 1024 and
``len == w*h*4`` (only the one used is copied: the smallest ≥ 32 px, else the largest; stored ≤ 64 px);
menus ≤ 8 levels and ≤ 500 entries; labels/titles ≤ 200 and tooltips ≤ 1000 characters; malformed values
are skipped; an item whose ``GetAll`` fails is dropped after one retry, one whose ``GetAll`` times out (a
busy app) stays hidden and is asked again (5 / 15 / 60 s and on its ``New*`` signals). A refresh that gets
UnknownMethod / UnknownObject / UnknownInterface drops the item (its object is gone).
Icon files are named after a hash of their content (``<cache>/tray/<sha1>.png|svg``, ≤ 300 files, pruned at
start and after every 50 new files, never an icon a live item shows); an app's own icon file (absolute
``IconName`` or found in ``IconThemePath``, ≤ 512 KiB) is copied in as well, so a file rewritten in place
gets a new path. Nothing an app sends is ever used as an output path. ``IconThemePath`` is only read (real
path inside that folder, ``.png``/``.svg`` only).

``key`` (stable per app, used by the ``taskbar.tray`` settings): the desktop id of the owning process
(``_pid_hints`` + app catalog, like windows.py; interpreters such as ``python3.14`` are skipped, the script
name and a non-generic SNI ``Id`` are tried last), else ``sni:<Id>`` when the Id is not generic, else
``exe:<executable>`` (for interpreters: the script or module). ``taskbar.tray.known`` gets
``{key: {name, icon, seen}}`` (new key, or a changed name/icon at most once an hour per key; ``seen`` at most
once a day; ≤ 100 entries; only the first live item of a key; only written by the primary instance — the
layer/preview desktop, or a serve/app host that owns instance.json).

``tray.activate`` on an item with ``ItemIsMenu`` and a menu answers ``fallback:'menu'`` without calling
``Activate`` (the SNI spec asks hosts to prefer the menu for such items). ``tray.scroll`` takes KDE's sign
(+ = up / left, 120 per notch); libappindicator items (``/org/ayatana/NotificationItem/…``) read it the GTK
way and as a step count, so they get ∓1 per notch. Activate / SecondaryActivate / ContextMenu get screen
coordinates (layer mode: the shell monitor's origin is added to the page's x/y).
"""

from __future__ import annotations

import array
import hashlib
import logging
import os
import queue
import re
import stat
import struct
import threading
import time
import zlib
from typing import Any

from ..util import ApiError, atomic_write_bytes, icon_exists, json_dumps, read_instance
from .apps import Debouncer, GLib, Gio, ensure_glib_loop, get_catalog
from .notifications import clean_markup
from .windows import _INTERPRETER_RE, _pid_hints, _read_proc

log = logging.getLogger("win11shell.tray")

PRIORITY = 45

WATCHER = "org.kde.StatusNotifierWatcher"
WATCHER_FDO = "org.freedesktop.StatusNotifierWatcher"
WATCHER_PATH = "/StatusNotifierWatcher"
ITEM_IFACE = "org.kde.StatusNotifierItem"
ITEM_IFACE_FDO = "org.freedesktop.StatusNotifierItem"
ITEM_PATH = "/StatusNotifierItem"
AYATANA_PATH = "/org/ayatana/NotificationItem/"   # libappindicator / libayatana-appindicator items
ITEM_NAME_PREFIXES = ("org.kde.StatusNotifierItem-", "org.freedesktop.StatusNotifierItem-")
HOST_PREFIX = "org.kde.StatusNotifierHost-"
MENU_IFACE = "com.canonical.dbusmenu"
DBUS_NAME = "org.freedesktop.DBus"
DBUS_PATH = "/org/freedesktop/DBus"
PROPS_IFACE = "org.freedesktop.DBus.Properties"

MAX_ITEMS = 64
MAX_PER_SENDER = 16
MAX_PIXMAP = 256                 # menu icon-data PNGs
MAX_PIXMAP_IN = 1024             # SNI pixmaps (Electron sends the PNG the app passed to new Tray(), often 512 px)
PIXMAP_WANT = 32                 # the smallest pixmap of at least this size is used (16 css px at DPR 2) …
PIXMAP_STORE = 64                # … and one larger than this is scaled down before it is stored
MAX_ICON_FILE = 512 * 1024       # an app's own icon file is copied into the cache up to this size
MAX_MENU_DEPTH = 8
MAX_MENU_NODES = 500
MAX_LABEL = 200
MAX_TEXT = 1000
GETALL_TIMEOUT_MS = 5000
RETRY_DELAY_MS = 1000
TIMEOUT_RETRY_MS = (5000, 15_000, 60_000)   # an item whose GetAll timed out: stays hidden, asked again
CALL_TIMEOUT_MS = 2000
CHANGED_DELAY_MS = 100
REFRESH_DELAY_MS = 50
CACHE_MAX_FILES = 300
CACHE_PRUNE_EVERY = 50           # new cache files between two prunings during a session
KNOWN_MAX = 100
DAY_MS = 86_400_000
ICON_REWRITE_MS = 3_600_000      # name / icon changes of a known app: at most one settings write per hour

ITEM_SIGNALS = {"NewTitle", "NewIcon", "NewAttentionIcon", "NewOverlayIcon", "NewToolTip", "NewStatus",
                "NewMenu", "NewIconThemePath"}
MENU_SIGNALS = {"LayoutUpdated", "ItemsPropertiesUpdated"}
MENU_EVENTS = ("clicked", "hovered", "opened", "closed")
STATUS_MAP = {"active": "active", "passive": "passive", "needsattention": "attention"}

_WATCHER_XML = """
<node>
  <interface name="@IFACE@">
    <method name="RegisterStatusNotifierItem"><arg direction="in" type="s" name="service"/></method>
    <method name="RegisterStatusNotifierHost"><arg direction="in" type="s" name="service"/></method>
    <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
    <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
    <property name="ProtocolVersion" type="i" access="read"/>
    <signal name="StatusNotifierItemRegistered"><arg type="s" name="service"/></signal>
    <signal name="StatusNotifierItemUnregistered"><arg type="s" name="service"/></signal>
    <signal name="StatusNotifierHostRegistered"/>
    <signal name="StatusNotifierHostUnregistered"/>
  </interface>
</node>
"""

_UNIQUE_RE = re.compile(r"^:[A-Za-z0-9_-]+(\.[A-Za-z0-9_-]+)+$")
_WELLKNOWN_RE = re.compile(r"^[A-Za-z_-][A-Za-z0-9_-]*(\.[A-Za-z_-][A-Za-z0-9_-]*)+$")
_PATH_RE = re.compile(r"^/$|^(/[A-Za-z0-9_]+)+$")
_ICON_NAME_RE = re.compile(r"^[A-Za-z0-9_+@-][A-Za-z0-9_.+@ -]{0,199}$")
_GENERIC_ID_RE = re.compile(r"^(chrome_status_icon_\d+|tray_icon_\d+|\d+)$", re.I)
_CTRL_RE = re.compile(r"[\x00-\x08\x0b-\x1f\x7f]")
_IMAGE_EXTS = (".png", ".svg")
_NO_MENU = ("", "/", "/NO_DBUSMENU")

# NoReply = the app left the bus before answering (e.g. it quit after "Afsluiten" in its menu)
_GONE = {"org.freedesktop.DBus.Error.ServiceUnknown", "org.freedesktop.DBus.Error.NameHasNoOwner",
         "org.freedesktop.DBus.Error.Disconnected", "org.freedesktop.DBus.Error.NoReply", "closed"}
_TIMEOUTS = {"timeout", "org.freedesktop.DBus.Error.Timeout", "org.freedesktop.DBus.Error.TimedOut"}
_UNSUPPORTED_WORDS = ("not supported", "not implemented", "unsupported", "unknown method", "no such method",
                      "not available")

_instances = 0
_instances_lock = threading.Lock()


# ---------------------------------------------------------------------------
# Pure helpers (tested directly)
# ---------------------------------------------------------------------------

def is_bus_name(name: Any) -> bool:
    return isinstance(name, str) and 0 < len(name) <= 255 and bool(
        _UNIQUE_RE.match(name) if name.startswith(":") else _WELLKNOWN_RE.match(name))


def is_object_path(path: Any) -> bool:
    return isinstance(path, str) and 0 < len(path) <= 1024 and bool(_PATH_RE.match(path))


def parse_registration(service: str, sender: str | None) -> tuple[str, str] | None:
    """``RegisterStatusNotifierItem`` argument → (bus name, object path), or None when invalid.

    ``:1.42`` / ``org.kde.StatusNotifierItem-1234-1`` → that name + ``/StatusNotifierItem``;
    ``/org/ayatana/NotificationItem/x`` (libayatana-appindicator) → the caller + that path;
    ``name/some/path`` (KDE watchers) or ``name@/some/path`` (GNOME AppIndicator)
    → that name + ``/some/path``."""
    if not isinstance(service, str) or len(service) > 1280:     # the raw argument: padding counts too
        return None
    s = service.strip()
    if not s:
        return None
    if s.startswith("/"):
        if not sender or not is_bus_name(sender) or not is_object_path(s):
            return None
        return sender, s
    if "@" in s:
        # GNOME's AppIndicator watcher publishes uniqueId as busName@objectPath.
        # Keep exactly the same bus-name/path validation as standard SNI entries.
        name, _, path = s.partition("@")
    else:
        name, slash, rest = s.partition("/")
        path = "/" + rest if slash else ITEM_PATH
    if not is_bus_name(name) or not is_object_path(path):
        return None
    return name, path


def is_generic_id(sni_id: str | None) -> bool:
    """Ids that say nothing about the app (Electron/Chromium ``chrome_status_icon_1``, ``tray_icon_1``, numbers)."""
    return not sni_id or not sni_id.strip() or bool(_GENERIC_ID_RE.match(sni_id.strip()))


def item_key(desktop_id: str | None, sni_id: str | None, exe: str | None) -> str:
    """Stable per-app key for the ``taskbar.tray`` settings (SPEC §6.15)."""
    if desktop_id:
        return desktop_id[:200]
    if not is_generic_id(sni_id):
        return ("sni:" + sni_id.strip())[:200]
    if exe:
        return ("exe:" + exe)[:200]
    return ("sni:" + ((sni_id or "").strip() or "onbekend"))[:200]


def map_status(status: str | None) -> str:
    """SNI ``Status`` (Active | Passive | NeedsAttention) → 'active' | 'passive' | 'attention'."""
    return STATUS_MAP.get(str(status or "").strip().lower(), "active")


def strip_mnemonic(label: str) -> str:
    """dbusmenu access keys: ``_Open`` → ``Open``, ``__`` → ``_``."""
    out = []
    i, n = 0, len(label)
    while i < n:
        c = label[i]
        if c == "_":
            if i + 1 < n and label[i + 1] == "_":
                out.append("_")
                i += 2
                continue
            i += 1
            continue
        out.append(c)
        i += 1
    return "".join(out)


_KEY_NAMES = {"control": "Ctrl", "ctrl": "Ctrl", "alt": "Alt", "shift": "Shift", "super": "Super",
              "return": "Enter", "kp_enter": "Enter", "escape": "Esc", "delete": "Del", "backspace": "Backspace",
              "space": "Space", "page_up": "PgUp", "page_down": "PgDn", "plus": "+", "minus": "-"}


def shortcut_text(combos: Any) -> str | None:
    """dbusmenu ``shortcut`` (``aas``, e.g. ``[["Control", "q"]]``) → ``"Ctrl+Q"``; several → ``", "``."""
    if not isinstance(combos, (list, tuple)):
        return None
    parts = []
    for combo in list(combos)[:3]:
        if not isinstance(combo, (list, tuple)):
            continue
        keys = []
        for k in list(combo)[:6]:
            if not isinstance(k, str) or not k or len(k) > 40:
                continue
            name = _KEY_NAMES.get(k.lower())
            if name is None:
                name = k.upper() if len(k) == 1 else k
            keys.append(name)
        if keys:
            parts.append("+".join(keys))
    return ", ".join(parts) or None


def _pixmap_rank(w: int, h: int) -> tuple[int, int]:
    """Lower is better: the smallest pixmap of at least PIXMAP_WANT px, else the largest smaller one."""
    return (0, w * h) if min(w, h) >= PIXMAP_WANT else (1, -w * h)


def best_pixmap(pixmaps: Any) -> tuple[int, int, bytes] | None:
    """The pixmap to show from ``[(w, h, data), …]`` (consistent ARGB32, ≤ 1024 px): the smallest one of at
    least 32 px (Qt/KDE send 16…64, the browser would blur a 64 px one down to 16), else the largest."""
    best = None
    for p in list(pixmaps or [])[:32]:
        try:
            w, h, data = int(p[0]), int(p[1]), p[2]
        except (TypeError, ValueError, IndexError):
            continue
        if not (1 <= w <= MAX_PIXMAP_IN and 1 <= h <= MAX_PIXMAP_IN) or not isinstance(data, (bytes, bytearray)):
            continue
        if len(data) != w * h * 4:
            continue
        if best is None or _pixmap_rank(w, h) < _pixmap_rank(best[0], best[1]):
            best = (w, h, bytes(data))
    return best


def scale_argb(w: int, h: int, argb: bytes, side: int = PIXMAP_STORE) -> tuple[int, int, bytes]:
    """Scale an ARGB32 pixmap down so its longest side is ``side`` (GdkPixbuf when there, else nearest)."""
    f = side / max(w, h)
    nw, nh = max(1, round(w * f)), max(1, round(h * f))
    rgba = bytearray(len(argb))
    rgba[0::4], rgba[1::4], rgba[2::4], rgba[3::4] = argb[1::4], argb[2::4], argb[3::4], argb[0::4]
    out = None
    try:
        import gi
        gi.require_version("GdkPixbuf", "2.0")
        from gi.repository import GdkPixbuf
        pb = GdkPixbuf.Pixbuf.new_from_bytes(GLib.Bytes.new(bytes(rgba)), GdkPixbuf.Colorspace.RGB, True, 8, w, h,
                                             w * 4).scale_simple(nw, nh, GdkPixbuf.InterpType.HYPER)
        data, rs = pb.read_pixel_bytes().get_data(), pb.get_rowstride()
        if pb.get_n_channels() == 4 and len(data) >= rs * (nh - 1) + nw * 4:
            out = b"".join(data[y * rs:y * rs + nw * 4] for y in range(nh))
    except Exception as e:  # noqa: BLE001
        log.debug("GdkPixbuf niet bruikbaar, naaste pixel (nearest-neighbour scaling): %s", e)
    if out is None:
        px = memoryview(bytes(rgba)).cast("I")         # one item per pixel; the byte order is kept
        xs = [min(w - 1, int((x + 0.5) * w / nw)) for x in range(nw)]
        buf = array.array("I")
        for y in range(nh):
            row = min(h - 1, int((y + 0.5) * h / nh)) * w
            buf.extend(px[row + x] for x in xs)
        out = buf.tobytes()
    back = bytearray(len(out))
    back[0::4], back[1::4], back[2::4], back[3::4] = out[3::4], out[0::4], out[1::4], out[2::4]
    return nw, nh, bytes(back)


def argb_to_png(w: int, h: int, argb: bytes) -> bytes:
    """ARGB32 in network byte order (SNI ``a(iiay)``) → PNG bytes (RGBA, 8 bit)."""
    rgba = bytearray(len(argb))
    rgba[0::4] = argb[1::4]
    rgba[1::4] = argb[2::4]
    rgba[2::4] = argb[3::4]
    rgba[3::4] = argb[0::4]
    stride = w * 4
    raw = b"".join(b"\x00" + bytes(rgba[y * stride:(y + 1) * stride]) for y in range(h))

    def chunk(tag: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def png_size(data: bytes) -> tuple[int, int] | None:
    """(width, height) of PNG bytes, or None when it is not a PNG."""
    if not isinstance(data, (bytes, bytearray)) or len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n" \
            or data[12:16] != b"IHDR":
        return None
    w, h = struct.unpack(">II", data[16:24])
    return w, h


def _size_order(name: str) -> tuple:
    if name == "scalable":
        return (0, 0)
    m = re.match(r"^(\d+)(?:x\d+)?(?:@\d+)?$", name)
    return (1, -int(m.group(1))) if m else (2, 0)


def theme_path_icon(theme_dir: str, name: str) -> str | None:
    """Look ``name`` up in an app's own ``IconThemePath`` (read only, never outside that folder)."""
    if not theme_dir or not isinstance(theme_dir, str) or not os.path.isabs(theme_dir) or "\x00" in theme_dir:
        return None
    if not name or not _ICON_NAME_RE.match(name) or name.startswith("."):
        return None
    try:
        base = os.path.realpath(theme_dir)
        if not os.path.isdir(base):
            return None
        cands = [os.path.join(base, name + ext) for ext in _IMAGE_EXTS]
        hic = os.path.join(base, "hicolor")
        if os.path.isdir(hic):
            sizes = sorted(os.listdir(hic)[:64], key=_size_order)
            for size in sizes:
                for sub in ("apps", "status"):
                    cands += [os.path.join(hic, size, sub, name + ext) for ext in _IMAGE_EXTS]
        for c in cands:
            rp = os.path.realpath(c)
            if rp.startswith(base + os.sep) and rp.lower().endswith(_IMAGE_EXTS) and os.path.isfile(rp):
                return rp
    except OSError:
        return None
    return None


def _clean_text(value: Any, limit: int) -> str:
    if not isinstance(value, str):
        return ""
    return _CTRL_RE.sub("", value[: limit * 4]).strip()[:limit]


def error_name(e: Exception) -> str:
    """D-Bus error name of a GLib.Error ('timeout' / 'cancelled' / 'closed' for local errors)."""
    try:
        name = Gio.DBusError.get_remote_error(e)
    except Exception:  # noqa: BLE001
        name = None
    if name:
        return name
    try:
        if e.matches(Gio.io_error_quark(), Gio.IOErrorEnum.TIMED_OUT):
            return "timeout"
        if e.matches(Gio.io_error_quark(), Gio.IOErrorEnum.CANCELLED):
            return "cancelled"
        if e.matches(Gio.io_error_quark(), Gio.IOErrorEnum.CLOSED):
            return "closed"
        if e.matches(Gio.dbus_error_quark(), Gio.DBusError.NO_REPLY) or \
                e.matches(Gio.dbus_error_quark(), Gio.DBusError.TIMEOUT):
            return "timeout"
        return f"{e.domain}:{e.code}"
    except Exception:  # noqa: BLE001
        return "error"


def error_kind(e: Exception) -> str:
    """'gone' | 'noobject' | 'timeout' | 'unsupported' | 'cancelled' | 'other'."""
    name = error_name(e)
    msg = (getattr(e, "message", "") or str(e)).lower()
    if name == "cancelled":
        return "cancelled"
    if name in _GONE:
        return "gone"
    if name in _TIMEOUTS:
        return "timeout"
    if name == "org.freedesktop.DBus.Error.UnknownObject" or \
            (name == "org.freedesktop.DBus.Error.UnknownMethod" and "object does not exist" in msg):
        return "noobject"
    if name.endswith((".UnknownMethod", ".NotSupported", ".UnknownInterface")):
        return "unsupported"
    if name == "org.freedesktop.DBus.Error.Failed" and any(w in msg for w in _UNSUPPORTED_WORDS):
        return "unsupported"
    return "other"


# -- variants -----------------------------------------------------------------

def _v_str(v) -> str | None:
    try:
        if v is not None and v.get_type_string() in ("s", "o", "g"):
            return v.get_string()
    except Exception:  # noqa: BLE001
        pass
    return None


def _v_bool(v, default: bool | None = None) -> bool | None:
    try:
        if v is not None and v.get_type_string() == "b":
            return v.get_boolean()
    except Exception:  # noqa: BLE001
        pass
    return default


def _v_int(v) -> int | None:
    try:
        t = v.get_type_string() if v is not None else ""
        if t == "i":
            return v.get_int32()
        if t == "u":
            return v.get_uint32()
        if t == "x":
            return v.get_int64()
        if t == "n":
            return v.get_int16()
        if t == "q":
            return v.get_uint16()
        if t == "y":
            return v.get_byte()
    except Exception:  # noqa: BLE001
        pass
    return None


def _v_pixmaps(v) -> list[tuple[int, int, bytes]]:
    """``a(iiay)`` → at most the one pixmap ``best_pixmap`` would pick: sizes are checked first and only the
    winner's data is copied (an app may send 32 × 4 MiB; that is not kept on the loop thread or the queue)."""
    try:
        if v is None or v.get_type_string() != "a(iiay)":
            return []
        best = None
        for i in range(min(v.n_children(), 32)):
            e = v.get_child_value(i)
            w = e.get_child_value(0).get_int32()
            h = e.get_child_value(1).get_int32()
            data_v = e.get_child_value(2)
            if not (1 <= w <= MAX_PIXMAP_IN and 1 <= h <= MAX_PIXMAP_IN) or data_v.get_size() != w * h * 4:
                continue
            if best is None or _pixmap_rank(w, h) < _pixmap_rank(best[0], best[1]):
                best = (w, h, data_v)
        if best is None:
            return []
        return [(best[0], best[1], bytes(best[2].get_data_as_bytes().get_data()))]
    except Exception as ex:  # noqa: BLE001
        log.debug("Onleesbare pixmap overgeslagen (unreadable pixmap skipped): %s", ex)
        return []


def _v_dict(v) -> dict:
    """``a{sv}`` → {name: GLib.Variant}; anything else → {}."""
    out = {}
    try:
        if v is None or v.get_type_string() != "a{sv}":
            return out
        for i in range(min(v.n_children(), 200)):
            e = v.get_child_value(i)
            try:
                out[e.get_child_value(0).get_string()] = e.get_child_value(1).get_variant()
            except Exception:  # noqa: BLE001
                continue
    except Exception:  # noqa: BLE001
        pass
    return out


def parse_item_props(v) -> dict:
    """``GetAll`` result (``a{sv}``) → plain values; every malformed property is skipped."""
    raw = _v_dict(v)
    p: dict[str, Any] = {}
    for k in ("Id", "Title", "Category", "Status", "IconName", "AttentionIconName", "OverlayIconName",
              "IconThemePath", "AttentionMovieName"):
        s = _v_str(raw.get(k))
        if s is not None:
            p[k] = _clean_text(s, MAX_LABEL if k not in ("IconThemePath",) else 1024)
    menu = _v_str(raw.get("Menu"))
    if menu is not None:
        p["Menu"] = menu
    for k in ("IconPixmap", "AttentionIconPixmap", "OverlayIconPixmap"):
        if k in raw:
            p[k] = _v_pixmaps(raw[k])
    b = _v_bool(raw.get("ItemIsMenu"))
    if b is not None:
        p["ItemIsMenu"] = b
    tt = raw.get("ToolTip")
    try:
        if tt is not None and tt.get_type_string() == "(sa(iiay)ss)":
            p["ToolTip"] = (clean_markup(tt.get_child_value(2).get_string(), MAX_LABEL),
                            clean_markup(tt.get_child_value(3).get_string(), MAX_TEXT))
    except Exception:  # noqa: BLE001
        pass
    return p


def parse_menu_layout(layout, write_icon=None, parents: list[int] | None = None) -> list[dict]:
    """dbusmenu ``(ia{sv}av)`` root → ``MenuItem[]`` (its children). ≤ 8 levels, ≤ 500 entries.

    ``parents`` (optional) collects the ids of submenu items (≤ 100) for ``AboutToShowGroup``, also the
    empty ``children-display=submenu`` ones that apps (Qt ``aboutToShow``, libdbusmenu ``about-to-show``)
    only fill when asked."""
    budget = [MAX_MENU_NODES]

    def node(v, depth: int) -> dict | None:
        try:
            if v is None or v.get_type_string() != "(ia{sv}av)" or budget[0] <= 0:
                return None
            budget[0] -= 1
            mid = v.get_child_value(0).get_int32()
            props = _v_dict(v.get_child_value(1))
            if _v_bool(props.get("visible"), True) is False:
                return None
            sep = _v_str(props.get("type")) == "separator"
            icon = None
            data = props.get("icon-data")
            if not sep and data is not None and data.get_type_string() == "ay" and write_icon is not None \
                    and data.get_size() <= 512 * 1024:
                icon = write_icon(bytes(data.get_data_as_bytes().get_data()))
            name = _v_str(props.get("icon-name"))
            if not sep and icon is None and name and (_ICON_NAME_RE.match(name) or
                                                      (name.startswith("/") and name.lower().endswith(_IMAGE_EXTS))):
                icon = name
            toggle = _v_str(props.get("toggle-type"))
            toggle = toggle if toggle in ("checkmark", "radio") else None
            state = _v_int(props.get("toggle-state"))
            children = []
            kids = v.get_child_value(2)
            submenu = _v_str(props.get("children-display")) == "submenu"
            if parents is not None and depth > 0 and not sep and (submenu or kids.n_children()) \
                    and len(parents) < 100:
                parents.append(mid)
            if not sep and depth < MAX_MENU_DEPTH and (submenu or kids.n_children()):
                for i in range(min(kids.n_children(), MAX_MENU_NODES)):
                    if budget[0] <= 0:
                        break
                    try:
                        c = node(kids.get_child_value(i).get_variant(), depth + 1)
                    except Exception:  # noqa: BLE001
                        c = None
                    if c is not None:
                        children.append(c)
                children = _tidy_separators(children)
            shortcut = None
            sc = props.get("shortcut")
            if sc is not None and sc.get_type_string() == "aas":
                shortcut = shortcut_text(sc.unpack())
            return {
                "id": mid,
                "type": "separator" if sep else "item",
                "label": "" if sep else strip_mnemonic(_clean_text(_v_str(props.get("label")) or "", MAX_LABEL * 2))[:MAX_LABEL],
                "enabled": bool(_v_bool(props.get("enabled"), True)),
                "icon": icon,
                "toggle": None if sep else toggle,
                "checked": None if sep or state not in (0, 1) else state == 1,
                "shortcut": None if sep else shortcut,
                "submenu": not sep and (submenu or bool(children)),   # also when still empty: never a leaf
                "children": children,
            }
        except Exception as e:  # noqa: BLE001 - one bad entry never breaks the menu
            log.debug("Onleesbaar menu-item overgeslagen (unreadable menu item skipped): %s", e)
            return None

    root = node(layout, 0)
    return root["children"] if root else []


def _tidy_separators(items: list[dict]) -> list[dict]:
    """No separators at the start/end or twice in a row (hidden items often leave them behind)."""
    out: list[dict] = []
    for it in items:
        if it["type"] == "separator" and (not out or out[-1]["type"] == "separator"):
            continue
        out.append(it)
    while out and out[-1]["type"] == "separator":
        out.pop()
    return out


def exe_name(pid: int | None) -> str | None:
    """Executable basename of a process; for interpreters the script / module (``python3 -m foo`` → foo)."""
    if not pid:
        return None
    try:
        exe = os.path.basename(os.readlink(f"/proc/{pid}/exe")).removesuffix(" (deleted)")
    except OSError:
        return None
    if exe and _INTERPRETER_RE.match(exe):
        argv = [a.decode("utf-8", "replace") for a in _read_proc(pid, "cmdline", 8192).split(b"\0") if a]
        args = iter(argv[1:])
        for a in args:
            if a in ("-m", "-jar"):
                mod = next(args, "")
                return (mod if a == "-m" else os.path.basename(mod).removesuffix(".jar"))[:100] or exe
            if a in ("-c", "-e"):
                break
            if a.startswith("-"):
                continue
            base = os.path.basename(a)
            for suf in (".py", ".js", ".pl", ".rb", ".jar", ".sh"):
                base = base.removesuffix(suf)
            return base[:100] or exe
    return exe[:100] or None


# ---------------------------------------------------------------------------
# Service
# ---------------------------------------------------------------------------

class _Item:
    __slots__ = ("id", "bus", "path", "seq", "names", "services", "iface", "pid", "ready", "fetching", "dirty",
                 "gen", "public", "menu", "sub_id", "menu_sub_id", "menu_sub_path", "refresh_src", "menu_src",
                 "resolved", "refreshes", "waiting")

    def __init__(self, iid: str, bus: str, path: str, seq: int):
        self.id = iid
        self.bus = bus                  # unique name
        self.path = path
        self.seq = seq
        self.names: set[str] = set()    # well-known names it was registered/adopted with
        self.services: set[str] = set()  # registration strings as a watcher listed them (host role)
        self.iface: str | None = None
        self.pid: int | None = None
        self.ready = False
        self.fetching = False
        self.dirty = False
        self.gen = 0
        self.public: dict | None = None
        self.menu: str | None = None
        self.sub_id = 0
        self.menu_sub_id = 0
        self.menu_sub_path: str | None = None
        self.refresh_src = 0
        self.menu_src = 0
        self.resolved: tuple | None = None   # (desktop_id, brief, exe)
        self.refreshes: list[float] = []
        self.waiting = False            # refresh_src is a back-off retry of an item that has not answered


class TrayService:
    def __init__(self, api, ctx, bus=None):
        global _instances
        self.api = api
        self.ctx = ctx
        self.bus = bus
        self.catalog = get_catalog(ctx)
        self.lock = threading.RLock()
        self.items: dict[str, _Item] = {}
        self.role = "none"
        self.seq = 0
        self.hosts: set[str] = set()          # other hosts registered with us (watcher role)
        self.watcher_owner: str | None = None  # unique name of the foreign watcher (host role)
        self.cache = os.path.join(str(ctx.cache_dir), "tray")
        # replaceable in tests
        self.pid_hints = _pid_hints
        self.exe_name = exe_name
        self.now_ms = lambda: int(time.time() * 1000)
        self.max_items = MAX_ITEMS
        self.max_per_sender = MAX_PER_SENDER
        self.getall_timeout_ms = GETALL_TIMEOUT_MS
        self.retry_delay_ms = RETRY_DELAY_MS
        self.timeout_retry_ms = TIMEOUT_RETRY_MS
        self.changed_delay_ms = CHANGED_DELAY_MS
        self._started = False
        self._stopped = False
        self._cancel = None
        self._reg_ids: list[int] = []
        self._subs: list[int] = []
        self._watcher_own = 0
        self._fdo_own = 0
        self._fdo_owned = False
        self._host_own = 0
        with _instances_lock:
            _instances += 1
            n = _instances
        self.host_name = f"{HOST_PREFIX}{os.getpid()}" + (f"-{n}" if n > 1 else "")
        self._changed_src = 0
        self._last_payload: str | None = None
        self._jobs: queue.Queue | None = None
        self._worker: threading.Thread | None = None
        self._known_lock = threading.Lock()
        self._known_cache: dict | None = None
        self._known_pending: dict[str, dict] = {}
        self._known_icon_at: dict[str, int] = {}
        self._known_flush = Debouncer(1.0, self._flush_known, name="tray-known")
        self._full_warned = False
        self._new_icons = 0

    def register(self) -> None:
        """Register the tray.* API methods on ``self.api``."""
        register_methods(self.api, self)

    # -- public state -----------------------------------------------------------------
    @property
    def supported(self) -> bool:
        return self.role != "none"

    def list_items(self) -> list[dict]:
        with self.lock:
            items = sorted((i for i in self.items.values() if i.ready and i.public), key=lambda i: i.seq)
            return [_copy_public(i.public) for i in items]

    def find(self, args: dict) -> _Item:
        iid = args.get("id")
        if not isinstance(iid, str) or not iid:
            raise ApiError("bad_args", "Ongeldig pictogram.")
        with self.lock:
            it = self.items.get(iid)
        if it is None or not it.ready:
            raise ApiError("not_found", "Dit pictogram bestaat niet meer.")
        return it

    # -- start / stop ------------------------------------------------------------------
    def _mode(self) -> str:
        return os.environ.get("WIN11SHELL_TRAY", "").strip().lower()

    def may_own_watcher(self) -> bool:
        force = self._mode()
        if force in ("watcher", "own", "on", "1"):
            return True
        if force == "host":
            return False
        return self.ctx.mode != "app"

    def start(self) -> None:
        if self._started:
            return
        self._started = True
        if Gio is None:
            log.info("Systeemvak niet beschikbaar: geen Gio (tray unavailable: no PyGObject)")
            return
        if self._mode() in ("off", "0", "no"):
            log.info("Systeemvak uitgezet met WIN11SHELL_TRAY=off (tray disabled)")
            return
        if self.bus is None:
            try:
                self.bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
            except Exception as e:  # noqa: BLE001
                log.info("Systeemvak: geen sessiebus (tray: no session bus): %s", e)
                self.bus = None
        if self.bus is None:
            return
        self._cancel = Gio.Cancellable()
        self._jobs = queue.Queue()
        self._worker = threading.Thread(target=self._work_loop, name="tray-worker", daemon=True)
        self._worker.start()
        self._work(self._prune_cache)
        bus = self.bus
        try:
            reg = getattr(bus, "register_object_with_closures2", None) or bus.register_object
            for iface_name in (WATCHER, WATCHER_FDO):
                node = Gio.DBusNodeInfo.new_for_xml(_WATCHER_XML.replace("@IFACE@", iface_name))
                self._reg_ids.append(reg(WATCHER_PATH, node.interfaces[0], self._on_watcher_call,
                                         self._on_watcher_get, None))
        except Exception:  # noqa: BLE001
            log.exception("Systeemvak: watcher-object registreren mislukt (cannot register watcher object)")
        self._subs.append(bus.signal_subscribe(DBUS_NAME, DBUS_NAME, "NameOwnerChanged", DBUS_PATH, None,
                                               Gio.DBusSignalFlags.NONE, self._on_name_owner_changed))
        self._subs.append(bus.signal_subscribe(None, WATCHER, None, WATCHER_PATH, None,
                                               Gio.DBusSignalFlags.NONE, self._on_watcher_signal))
        if self.may_own_watcher():
            self._own_watcher()      # acquired → watcher (adopts); lost → host (adopts)
        else:
            self._become_host()
        ensure_glib_loop(self.ctx)

    def stop(self) -> None:
        """Release names, objects, subscriptions and timers. Never raises."""
        if self._stopped:
            return
        self._stopped = True
        try:
            if self._cancel is not None:
                self._cancel.cancel()
        except Exception:  # noqa: BLE001
            pass
        with self.lock:
            items = list(self.items.values())
            self.items.clear()
            srcs = [self._changed_src] + [s for i in items for s in (i.refresh_src, i.menu_src)]
            self._changed_src = 0
            subs = list(self._subs) + [s for i in items for s in (i.sub_id, i.menu_sub_id)]
            self._subs.clear()
            regs = list(self._reg_ids)
            self._reg_ids.clear()
            owns = [self._watcher_own, self._fdo_own, self._host_own]
            self._watcher_own = self._fdo_own = self._host_own = 0
            self.role = "none"
        for src in srcs:
            if src:
                try:
                    GLib.source_remove(src)
                except Exception:  # noqa: BLE001
                    pass
        bus = self.bus
        if bus is not None:
            for sid in subs:
                if sid:
                    try:
                        bus.signal_unsubscribe(sid)
                    except Exception:  # noqa: BLE001
                        pass
            for rid in regs:
                try:
                    bus.unregister_object(rid)
                except Exception:  # noqa: BLE001
                    pass
        for oid in owns:
            if oid:
                try:
                    Gio.bus_unown_name(oid)
                except Exception:  # noqa: BLE001
                    pass
        if self._jobs is not None:
            self._jobs.put(None)
        try:
            self._known_flush.close()
            self._flush_known()
        except Exception:  # noqa: BLE001
            log.debug("tray known flush failed", exc_info=True)

    # -- worker ------------------------------------------------------------------------
    def _work(self, fn, *args) -> None:
        if self._jobs is not None and not self._stopped:
            self._jobs.put((fn, args))

    def _work_loop(self) -> None:
        jobs = self._jobs
        while jobs is not None:
            job = jobs.get()
            if job is None:
                return
            fn, args = job
            try:
                fn(*args)
            except Exception:  # noqa: BLE001
                log.exception("Systeemvak: achtergrondtaak faalde (tray job failed)")

    def _prune_cache(self) -> None:
        """≤ 300 files, oldest first; never an icon a live item shows."""
        with self.lock:
            live = {p for it in self.items.values() if it.public
                    for p in (it.public.get("icon"), it.public.get("attentionIcon"), it.public.get("overlayIcon"),
                              it.public.get("appIcon")) if p}
        try:
            entries = [e for e in os.scandir(self.cache)
                       if e.is_file() and e.name.endswith(_IMAGE_EXTS) and e.path not in live]
        except OSError:
            return
        if len(entries) <= CACHE_MAX_FILES:
            return

        def mtime(e):
            try:
                return e.stat().st_mtime
            except OSError:
                return 0

        entries.sort(key=mtime)
        for e in entries[: len(entries) - CACHE_MAX_FILES]:
            try:
                os.unlink(e.path)
            except OSError:
                pass

    # -- roles -------------------------------------------------------------------------
    def _set_role(self, role: str) -> None:
        with self.lock:
            if self.role == role or self._stopped:
                return
            self.role = role
        log.info("Systeemvak: rol %s (tray role)", role)
        self.api.emit("tray.status", {"supported": role != "none", "role": role})

    def _own_watcher(self) -> None:
        if self._watcher_own:
            try:
                Gio.bus_unown_name(self._watcher_own)
            except Exception:  # noqa: BLE001
                pass
        self._watcher_own = Gio.bus_own_name_on_connection(self.bus, WATCHER, Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
                                                           self._on_watcher_acquired, self._on_watcher_lost)

    def _on_watcher_acquired(self, _conn, _name) -> None:
        if self._stopped or self.role == "watcher":
            return
        # a host name from an earlier host role is kept (the watcher is a host too); Gio.bus_unown_name
        # would block the loop on a synchronous ReleaseName
        self.watcher_owner = None
        self._set_role("watcher")
        if not self._fdo_own:
            def fdo(owned):
                def cb(_conn, _name):
                    self._fdo_owned = owned and not self._stopped
                return cb

            self._fdo_own = Gio.bus_own_name_on_connection(self.bus, WATCHER_FDO, Gio.BusNameOwnerFlags.DO_NOT_QUEUE,
                                                           fdo(True), fdo(False))
        self._watcher_signal("StatusNotifierHostRegistered", None)
        with self.lock:
            known = [i.id for i in sorted(self.items.values(), key=lambda i: i.seq)]
        for iid in known:
            self._watcher_signal("StatusNotifierItemRegistered", GLib.Variant("(s)", (iid,)))
        self._adopt()

    def _on_watcher_lost(self, _conn, _name) -> None:
        if self._stopped:
            return
        if self.role == "watcher":
            # only happens when the connection closed: nothing works any more
            log.warning("Systeemvak: verbinding met de sessiebus verloren (lost the session bus)")
            self._set_role("none")
            return
        if self.role != "host":
            self._become_host()

    def _become_host(self) -> None:
        self._set_role("host")
        if not self._host_own:
            self._host_own = Gio.bus_own_name_on_connection(self.bus, self.host_name,
                                                            Gio.BusNameOwnerFlags.DO_NOT_QUEUE, None, None)
        self._async(DBUS_NAME, DBUS_PATH, DBUS_NAME, "GetNameOwner", GLib.Variant("(s)", (WATCHER,)), "(s)",
                    self._on_watcher_owner, timeout=CALL_TIMEOUT_MS)
        self._adopt()

    def _on_watcher_owner(self, reply, err) -> None:
        if err is not None or reply is None:
            # nobody is the watcher (it just vanished): take over when allowed, else wait for one
            if self.may_own_watcher() and self.role == "host":
                self._own_watcher()
            return
        owner = reply[0]
        if owner and owner != self.bus.get_unique_name():
            self.watcher_owner = owner
            self._host_register()

    def _host_register(self) -> None:
        """Host role: tell the foreign watcher about us and read its items."""
        if self.role != "host" or not self.watcher_owner:
            return
        self._async(WATCHER, WATCHER_PATH, WATCHER, "RegisterStatusNotifierHost",
                    GLib.Variant("(s)", (self.host_name,)), None, lambda r, e: None, timeout=CALL_TIMEOUT_MS)
        self._async(WATCHER, WATCHER_PATH, PROPS_IFACE, "Get",
                    GLib.Variant("(ss)", (WATCHER, "RegisteredStatusNotifierItems")), "(v)",
                    self._on_registered_items, timeout=CALL_TIMEOUT_MS)

    def _on_registered_items(self, reply, err) -> None:
        if err is not None or reply is None or self.role != "host":
            if err is not None:
                log.debug("RegisteredStatusNotifierItems onleesbaar (unreadable): %s", err)
            return
        v = reply.get_child_value(0).get_variant() if reply.n_children() else None
        if v is None or v.get_type_string() != "as":
            return
        for s in v.unpack()[: self.max_items]:
            self._register_service(s, None, listed=True)

    def _adopt(self) -> None:
        """Adopt items whose apps registered with an earlier watcher (they own SNI-style names)."""
        self._async(DBUS_NAME, DBUS_PATH, DBUS_NAME, "ListNames", None, "(as)", self._on_names,
                    timeout=CALL_TIMEOUT_MS)

    def _on_names(self, reply, err) -> None:
        if err is not None or reply is None:
            return
        names = [n for n in reply[0] if isinstance(n, str) and n.startswith(ITEM_NAME_PREFIXES)]
        for n in sorted(names)[: self.max_items]:
            self._add_service(n, ITEM_PATH, None)

    # -- bus signals -------------------------------------------------------------------
    def _on_name_owner_changed(self, _conn, _sender, _path, _iface, _signal, params) -> None:
        if self._stopped:
            return
        try:
            name, _old, new = params.unpack()
        except Exception:  # noqa: BLE001
            return
        if name == WATCHER:
            if self.role != "host":
                return
            if not new:
                self.watcher_owner = None
                if self.may_own_watcher():
                    self._own_watcher()          # the watcher went away: take over
            elif new != self.bus.get_unique_name() and new != self.watcher_owner:
                self.watcher_owner = new         # a new watcher: register with it
                self._host_register()
            return
        with self.lock:
            gone = [it.id for it in self.items.values()
                    if (it.bus == name and not new) or (name in it.names and new != it.bus)]
        for iid in gone:
            self._remove_item(iid, "verdwenen (owner gone)")
        if not new and name in self.hosts:
            self.hosts.discard(name)
            self._watcher_signal("StatusNotifierHostUnregistered", None)

    def _on_watcher_signal(self, _conn, sender, _path, _iface, signal, params) -> None:
        """Host role: the foreign watcher's Registered/Unregistered signals."""
        if self._stopped or self.role != "host" or not sender or sender != self.watcher_owner:
            return
        try:
            if signal == "StatusNotifierItemRegistered" and params.get_type_string() == "(s)":
                self._register_service(params.get_child_value(0).get_string(), None, listed=True)
            elif signal == "StatusNotifierItemUnregistered" and params.get_type_string() == "(s)":
                s = params.get_child_value(0).get_string().strip()
                with self.lock:
                    gone = [i.id for i in self.items.values() if s in i.services or s == i.id]
                for iid in gone:
                    self._remove_item(iid, "afgemeld (unregistered)")
        except Exception:  # noqa: BLE001
            log.debug("Watcher-signaal onleesbaar (bad watcher signal)", exc_info=True)

    def _watcher_signal(self, name: str, value) -> None:
        if self.role != "watcher" or self.bus is None:
            return
        ifaces = [WATCHER] + ([WATCHER_FDO] if self._fdo_owned else [])
        for iface in ifaces:
            try:
                self.bus.emit_signal(None, WATCHER_PATH, iface, name, value)
            except Exception as e:  # noqa: BLE001
                log.debug("Signaal %s mislukt (signal failed): %s", name, e)

    # -- watcher object ------------------------------------------------------------------
    def _on_watcher_call(self, _conn, sender, _path, _iface, method, params, invocation) -> None:
        try:
            if self.role != "watcher":
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", "Not the StatusNotifierWatcher")
            elif method == "RegisterStatusNotifierItem":
                service = params.get_child_value(0).get_string()
                parsed = parse_registration(service, sender)
                if parsed is None:
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs",
                                                 "Invalid StatusNotifierItem service")
                    return
                with self.lock:
                    full = len(self.items) >= self.max_items and not any(
                        i.id == parsed[0] + parsed[1] or (parsed[0] in i.names and i.path == parsed[1])
                        for i in self.items.values())
                if full:
                    self._warn_full()
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.LimitsExceeded", "Too many tray items")
                    return
                # no registration string kept: ``services`` only serves host-role Unregistered matching
                self._add_service(parsed[0], parsed[1], None)
                invocation.return_value(None)
            elif method == "RegisterStatusNotifierHost":
                name = params.get_child_value(0).get_string()
                if not is_bus_name(name):
                    invocation.return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", "Invalid host name")
                    return
                added = name not in self.hosts and len(self.hosts) < 32
                if added:
                    self.hosts.add(name)
                invocation.return_value(None)
                if added:       # a repeated or refused registration changes nothing: no broadcast
                    self._watcher_signal("StatusNotifierHostRegistered", None)
            else:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.UnknownMethod", "Unknown method")
        except Exception as e:  # noqa: BLE001
            log.exception("Systeemvak: aanroep %s mislukt (watcher call failed)", method)
            try:
                invocation.return_dbus_error("org.freedesktop.DBus.Error.Failed", str(e)[:200])
            except Exception:  # noqa: BLE001
                pass

    def _on_watcher_get(self, _conn, _sender, _path, _iface, prop):
        if prop == "RegisteredStatusNotifierItems":
            with self.lock:
                ids = [i.id for i in sorted(self.items.values(), key=lambda i: i.seq)]
            return GLib.Variant("as", ids)
        if prop == "IsStatusNotifierHostRegistered":
            return GLib.Variant("b", True)
        if prop == "ProtocolVersion":
            return GLib.Variant("i", 0)
        return None

    def _warn_full(self) -> None:
        if not self._full_warned:
            self._full_warned = True
            log.warning("Systeemvak vol: meer dan %d pictogrammen genegeerd (tray full, items ignored)",
                        self.max_items)

    # -- items -----------------------------------------------------------------------------
    def _register_service(self, service: str, sender: str | None, listed: bool = False) -> None:
        parsed = parse_registration(service, sender)
        if parsed is None:
            log.debug("Ongeldige registratie genegeerd (invalid registration ignored): %r", service[:200])
            return
        self._add_service(parsed[0], parsed[1], service if listed else None)

    def _add_service(self, bus_name: str, path: str, service: str | None) -> None:
        if bus_name.startswith(":"):
            self._add_item(bus_name, path, service, None)
            return
        with self.lock:
            self.seq += 1            # keep the registration order while the owner is looked up
            seq = self.seq

        def owner(reply, err):
            if err is None and reply is not None and str(reply[0]).startswith(":"):
                self._add_item(reply[0], path, service, bus_name, seq)
            else:
                log.debug("Systeemvak: %s heeft geen eigenaar (name has no owner)", bus_name)

        self._async(DBUS_NAME, DBUS_PATH, DBUS_NAME, "GetNameOwner", GLib.Variant("(s)", (bus_name,)), "(s)", owner,
                    timeout=CALL_TIMEOUT_MS)

    def _add_item(self, unique: str, path: str, service: str | None, wellknown: str | None,
                  seq: int | None = None) -> None:
        if self._stopped or self.bus is None or (unique == self.bus.get_unique_name() and path == WATCHER_PATH):
            return
        iid = unique + path
        with self.lock:
            it = self.items.get(iid)
            if it is not None:
                if wellknown:
                    it.names.add(wellknown)
                if service and len(it.services) < 8:
                    it.services.add(service.strip())
                new = False
            else:
                if len(self.items) >= self.max_items:
                    self._warn_full()
                    return
                if sum(1 for i in self.items.values() if i.bus == unique) >= self.max_per_sender:
                    log.info("Systeemvak: te veel pictogrammen van %s (too many items from one app)", unique)
                    return
                if seq is None:
                    self.seq += 1
                    seq = self.seq
                it = _Item(iid, unique, path, seq)
                if wellknown:
                    it.names.add(wellknown)
                if service:
                    it.services.add(service.strip())
                self.items[iid] = it
                new = True
        if not new:
            self._schedule_refresh(iid)   # duplicate registration: refresh, never a second icon
            return
        try:
            it.sub_id = self.bus.signal_subscribe(unique, None, None, path, None, Gio.DBusSignalFlags.NONE,
                                                  lambda c, s, p, i, m, v, iid=iid: self._on_item_signal(iid, m))
        except Exception:  # noqa: BLE001
            log.debug("signal_subscribe failed", exc_info=True)
        self._watcher_signal("StatusNotifierItemRegistered", GLib.Variant("(s)", (iid,)))

        def got_pid(reply, err):
            if err is None and reply is not None:
                pid = int(reply[0])
                with self.lock:
                    it.pid = pid if pid > 0 else None
            self._fetch_props(iid)

        self._async(DBUS_NAME, DBUS_PATH, DBUS_NAME, "GetConnectionUnixProcessID", GLib.Variant("(s)", (unique,)),
                    "(u)", got_pid, timeout=CALL_TIMEOUT_MS)

    def _remove_item(self, iid: str, why: str = "") -> None:
        with self.lock:
            it = self.items.pop(iid, None)
        if it is None:
            return
        log.info("Systeemvak: %s %s", iid, why)
        for src in (it.refresh_src, it.menu_src):
            if src:
                try:
                    GLib.source_remove(src)
                except Exception:  # noqa: BLE001
                    pass
        if self.bus is not None:
            for sid in (it.sub_id, it.menu_sub_id):
                if sid:
                    try:
                        self.bus.signal_unsubscribe(sid)
                    except Exception:  # noqa: BLE001
                        pass
        self._watcher_signal("StatusNotifierItemUnregistered", GLib.Variant("(s)", (iid,)))
        self._schedule_changed()

    def _on_item_signal(self, iid: str, member: str) -> None:
        if member in ITEM_SIGNALS or member == "PropertiesChanged":
            self._schedule_refresh(iid)

    def _schedule_refresh(self, iid: str) -> None:
        with self.lock:
            it = self.items.get(iid)
            if it is None or self._stopped:
                return
            if it.refresh_src and it.waiting and not it.ready:
                # a slow item that has not answered yet sends a signal: it is alive, ask now
                GLib.source_remove(it.refresh_src)
                it.refresh_src = 0
                it.waiting = False
            if it.refresh_src:
                return
            now = time.monotonic()
            it.refreshes = [t for t in it.refreshes if now - t < 10.0] + [now]
            delay = REFRESH_DELAY_MS if len(it.refreshes) <= 40 else 1000   # an app flooding signals is slowed down
            it.refresh_src = GLib.timeout_add(delay, self._refresh_due, iid)

    def _retry_later(self, iid: str, delay: int, attempt: int, waiting: bool = False) -> None:
        def retry():
            with self.lock:
                if iid in self.items:
                    self.items[iid].refresh_src = 0
                    self.items[iid].waiting = False
            self._fetch_props(iid, attempt=attempt)
            return False

        with self.lock:
            it = self.items.get(iid)
            if it is not None and not self._stopped and not it.refresh_src:
                it.refresh_src = GLib.timeout_add(delay, retry)
                it.waiting = waiting

    def _refresh_due(self, iid: str) -> bool:
        with self.lock:
            it = self.items.get(iid)
            if it is not None:
                it.refresh_src = 0
        if it is not None:
            self._fetch_props(iid)
        return False

    def _fetch_props(self, iid: str, attempt: int = 0, iface: str | None = None) -> None:
        with self.lock:
            it = self.items.get(iid)
            if it is None or self._stopped:
                return
            if it.fetching and iface is None:
                it.dirty = True
                return
            it.fetching = True
            iface = iface or it.iface or ITEM_IFACE
        self._async(it.bus, it.path, PROPS_IFACE, "GetAll", GLib.Variant("(s)", (iface,)), "(a{sv})",
                    lambda r, e: self._on_props(iid, iface, attempt, r, e), timeout=self.getall_timeout_ms)

    def _on_props(self, iid: str, iface: str, attempt: int, reply, err) -> None:
        with self.lock:
            it = self.items.get(iid)
            if it is None:
                return
            it.fetching = False
            dirty, it.dirty = it.dirty, False
            first = it.iface is None
        props = None
        if err is None and reply is not None:
            props = parse_item_props(reply.get_child_value(0))
        kind = error_kind(err) if err is not None else None
        if kind == "cancelled":
            return
        if kind == "gone":
            self._remove_item(iid, "verdwenen (gone)")
            return
        if first and iface == ITEM_IFACE and kind != "timeout" and not props:
            # older/other implementations use the freedesktop interface name
            with self.lock:
                it.fetching = True
            self._async(it.bus, it.path, PROPS_IFACE, "GetAll", GLib.Variant("(s)", (ITEM_IFACE_FDO,)), "(a{sv})",
                        lambda r, e: self._on_props(iid, ITEM_IFACE_FDO, attempt, r, e),
                        timeout=self.getall_timeout_ms)
            return
        if not props:
            if not first:
                if kind == "noobject" or (err is not None and error_name(err).endswith(
                        (".UnknownMethod", ".UnknownObject", ".UnknownInterface"))):
                    # GetAll on the interface that answered before: a live object always has Properties, so
                    # this means the object is gone (libdbus and translated GDBus messages say it otherwise)
                    self._remove_item(iid, "object verdwenen (object gone)")
                elif dirty:
                    self._schedule_refresh(iid)
                return
            if kind == "timeout":
                # slow (a busy main loop at login), not hostile: stay hidden but subscribed and try again
                # later; a New*/PropertiesChanged signal fetches at once (apps never register a second time)
                if dirty:
                    self._schedule_refresh(iid)
                elif attempt < len(self.timeout_retry_ms):
                    log.info("Systeemvak: %s antwoordt (nog) niet, later opnieuw (no answer yet, retrying later)",
                             iid)
                    self._retry_later(iid, self.timeout_retry_ms[attempt], attempt + 1, waiting=True)
                return
            if attempt == 0:
                log.info("Systeemvak: %s antwoordt niet, nog één poging (no answer, retrying): %s", iid,
                         error_name(err) if err is not None else "leeg (empty)")
                self._retry_later(iid, self.retry_delay_ms, 1)
                return
            self._remove_item(iid, "antwoordt niet, genegeerd (does not answer, dropped)")
            return
        with self.lock:
            it.iface = iface
            it.gen += 1
            gen = it.gen
        self._work(self._resolve, iid, gen, props)
        if dirty:
            self._schedule_refresh(iid)

    # -- resolving (worker thread) -----------------------------------------------------------
    def _resolve(self, iid: str, gen: int, props: dict) -> None:
        with self.lock:
            it = self.items.get(iid)
            if it is None or it.gen != gen:
                return              # gone, or a newer GetAll answer is queued behind this one
            pid = it.pid
            resolved = it.resolved
        sni_id = props.get("Id") or ""
        if resolved is None:
            exe = None
            try:
                exe = self.exe_name(pid)
            except Exception:  # noqa: BLE001
                pass
            desktop_id = self._desktop_for(pid, sni_id, exe)
            brief = None
            if desktop_id:
                try:
                    brief = self.catalog.brief(desktop_id)
                except Exception:  # noqa: BLE001
                    brief = None
            resolved = (desktop_id, brief, exe)
        desktop_id, brief, exe = resolved
        theme = props.get("IconThemePath") or ""
        icon = self._icon(props.get("IconName"), props.get("IconPixmap"), theme)
        title = props.get("Title") or ""
        tooltip = props.get("ToolTip") or ("", "")
        menu = props.get("Menu")
        menu = menu if menu and menu not in _NO_MENU and is_object_path(menu) else None
        pub = {
            "id": iid,
            "key": item_key(desktop_id, sni_id, exe),
            "sniId": sni_id,
            "title": title,
            "appName": ((brief or {}).get("name") or title or sni_id or exe or "App")[:MAX_LABEL],
            "appIcon": (brief or {}).get("icon") or icon,
            "desktopId": desktop_id,
            "pid": pid,
            "category": (props.get("Category") or "ApplicationStatus")[:40],
            "status": map_status(props.get("Status")),
            "icon": icon,
            "attentionIcon": self._icon(props.get("AttentionIconName"), props.get("AttentionIconPixmap"), theme),
            "overlayIcon": self._icon(props.get("OverlayIconName"), props.get("OverlayIconPixmap"), theme),
            "tooltip": {"title": tooltip[0], "text": tooltip[1]},
            "itemIsMenu": bool(props.get("ItemIsMenu", False)),
            "hasMenu": menu is not None,
        }
        with self.lock:
            it = self.items.get(iid)
            if it is None or it.gen != gen:
                return
            it.resolved = resolved
            it.public = pub
            it.ready = True
            menu_changed = it.menu != menu
            it.menu = menu
            # several live items with one key (two icons of one process, apps with the same generic Id):
            # only the first one (like the UI shows) keeps the known entry, or their names would alternate
            rep = min((i for i in self.items.values() if i.ready and i.public and i.public["key"] == pub["key"]),
                      key=lambda i: i.seq, default=None)
        if menu_changed:
            GLib.idle_add(self._subscribe_menu, iid)
        self._schedule_changed()
        if rep is None or rep.id == iid:
            self._note_known(pub)

    def _desktop_for(self, pid: int | None, sni_id: str, exe: str | None = None) -> str | None:
        """Desktop id of the app behind an item: the process hints (launcher desktop file, systemd scope,
        executable), then its script name and its SNI ``Id``. Interpreters never count: ``python3.14``
        would turn blueman & co. into "Python (v3.14)"."""
        hints = []
        if pid:
            try:
                hints = [h for h in self.pid_hints(pid) if h and not _INTERPRETER_RE.match(h)]
            except Exception:  # noqa: BLE001
                hints = []
        if exe and not _INTERPRETER_RE.match(exe):
            hints.append(exe)
        if not is_generic_id(sni_id):
            hints.append(sni_id)
        for hint in dict.fromkeys(hints):
            try:
                did, _icon = self.catalog.match_window(hint)
            except Exception:  # noqa: BLE001
                did = None
            if did:
                return did
        return None

    def _icon(self, name: str | None, pixmaps, theme: str) -> str | None:
        """Themed name / cached copy of a file / cached PNG for one of the three SNI icons, or None."""
        name = (name or "").strip()
        if name.startswith("/"):
            if name.lower().endswith(_IMAGE_EXTS) and "\x00" not in name and os.path.isfile(name):
                return self._file_icon(name)
            name = ""
        elif name and not _ICON_NAME_RE.match(name):
            name = ""
        if name and theme:
            p = theme_path_icon(theme, name)
            if p:
                return self._file_icon(p)
        best = best_pixmap(pixmaps)
        if name and (best is None or icon_exists(name)):
            return name
        if best is not None:
            return self.write_png(*best)
        return name or None

    def _file_icon(self, path: str) -> str:
        """An app's own icon file → a content-hashed copy in the cache: a file the app rewrites in place (and
        announces with NewIcon) then gets a new path, so the item and the /icon URL change. Files over
        512 KiB (or unreadable ones) are passed on as they are."""
        try:
            fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOCTTY)   # never blocks on a FIFO
        except OSError:
            return path
        try:
            st = os.fstat(fd)
            if not stat.S_ISREG(st.st_mode) or not 0 < st.st_size <= MAX_ICON_FILE:
                return path
            data = os.read(fd, MAX_ICON_FILE + 1)
        except OSError:
            return path
        finally:
            os.close(fd)
        if not data or len(data) > MAX_ICON_FILE:
            return path
        ext = ".svg" if path.lower().endswith(".svg") else ".png"
        return self._store_png(hashlib.sha1(data).hexdigest(), lambda: data, ext) or path

    def write_png(self, w: int, h: int, argb: bytes) -> str | None:
        digest = hashlib.sha1(b"%dx%d:" % (w, h) + argb).hexdigest()     # of the pixmap as sent

        def make():
            if max(w, h) > PIXMAP_STORE:
                return argb_to_png(*scale_argb(w, h, argb))
            return argb_to_png(w, h, argb)

        return self._store_png(digest, make)

    def write_png_bytes(self, data: bytes) -> str | None:
        """A menu ``icon-data`` (PNG bytes from the app): only real PNGs ≤ 256 px."""
        size = png_size(data)
        if size is None or not (1 <= size[0] <= MAX_PIXMAP and 1 <= size[1] <= MAX_PIXMAP):
            return None
        return self._store_png(hashlib.sha1(data).hexdigest(), lambda: data)

    def _store_png(self, digest: str, make, ext: str = ".png") -> str | None:
        path = os.path.join(self.cache, digest + ext)
        try:
            if os.path.isfile(path):
                try:
                    os.utime(path)       # recently used files survive the pruning
                except OSError:
                    pass
                return path
            atomic_write_bytes(path, make(), mode=0o600)
        except OSError as e:
            log.debug("Pictogram niet opgeslagen (icon not cached): %s", e)
            return None
        with self.lock:
            self._new_icons += 1
            due = self._new_icons >= CACHE_PRUNE_EVERY
            if due:
                self._new_icons = 0
        if due:                          # an app that draws a new icon each time must not fill the disk
            self._work(self._prune_cache)
        return path

    # -- tray.changed ---------------------------------------------------------------------------
    def _schedule_changed(self) -> None:
        with self.lock:
            if self._changed_src or self._stopped:
                return
            self._changed_src = GLib.timeout_add(self.changed_delay_ms, self._emit_changed)

    def _emit_changed(self) -> bool:
        with self.lock:
            self._changed_src = 0
            if self._stopped:
                return False
        items = self.list_items()
        payload = json_dumps(items)
        if payload == self._last_payload:
            return False
        self._last_payload = payload
        self.api.emit("tray.changed", {"items": items})
        return False

    # -- menus ------------------------------------------------------------------------------------
    def _subscribe_menu(self, iid: str) -> bool:
        with self.lock:
            it = self.items.get(iid)
            if it is None or self._stopped or it.menu == it.menu_sub_path:
                return False
            old, it.menu_sub_id = it.menu_sub_id, 0
            it.menu_sub_path = it.menu
            menu, bus_name = it.menu, it.bus
        if old:
            try:
                self.bus.signal_unsubscribe(old)
            except Exception:  # noqa: BLE001
                pass
        if menu:
            sid = self.bus.signal_subscribe(bus_name, MENU_IFACE, None, menu, None, Gio.DBusSignalFlags.NONE,
                                            lambda c, s, p, i, m, v, iid=iid: self._on_menu_signal(iid, m))
            with self.lock:
                if iid in self.items and not self._stopped:
                    self.items[iid].menu_sub_id = sid
                    sid = 0
            if sid:
                self.bus.signal_unsubscribe(sid)
        return False

    def _on_menu_signal(self, iid: str, member: str) -> None:
        if member not in MENU_SIGNALS:
            return
        with self.lock:
            it = self.items.get(iid)
            if it is None or it.menu_src or self._stopped:
                return

            def due():
                with self.lock:
                    if iid in self.items:
                        self.items[iid].menu_src = 0
                if not self._stopped:
                    self.api.emit("tray.menuChanged", {"id": iid})
                return False

            it.menu_src = GLib.timeout_add(100, due)

    def menu(self, it: _Item, refresh: bool = False) -> dict:
        if not it.menu:
            raise ApiError("no_menu", "Deze app heeft geen menu.")
        if refresh:
            # live refresh of an open menu (tray.menuChanged): GetLayout only. AboutToShow would make some
            # apps announce a new layout again (a refresh loop), and the open menu was prepared already.
            rev, items = self._layout(it)
            return {"revision": rev, "items": items}
        try:
            self._call(it, "AboutToShow", GLib.Variant("(i)", (0,)), "(b)", iface=MENU_IFACE, path=it.menu,
                       timeout=1000)
        except GLib.Error as e:
            if error_kind(e) in ("gone", "noobject"):
                raise self._menu_error(e, it) from None
        parents: list[int] = []
        rev, items = self._layout(it, parents)
        if parents:
            try:
                self._call(it, "AboutToShowGroup", GLib.Variant("(ai)", (parents,)), "(aiai)", iface=MENU_IFACE,
                           path=it.menu, timeout=1000)
            except GLib.Error:
                parents = []     # optional in dbusmenu (Qt and old libdbusmenu lack it)
            if parents:
                # Qt and libdbusmenu-glib fill (lazy) submenus in about-to-show before they reply, but always
                # answer updatesNeeded=[]: read the layout again whatever the reply says
                rev, items = self._layout(it)
        return {"revision": rev, "items": items}

    def _layout(self, it: _Item, parents: list[int] | None = None) -> tuple[int, list[dict]]:
        try:
            r = self._call(it, "GetLayout", GLib.Variant("(iias)", (0, -1, [])), "(u(ia{sv}av))",
                           iface=MENU_IFACE, path=it.menu)
        except GLib.Error as e:
            kind = error_kind(e)
            if kind in ("gone", "noobject", "timeout"):
                raise self._menu_error(e, it) from None
            if kind == "unsupported":
                # libdbus answers a call to a removed object with UnknownMethod: let GetAll find out
                GLib.idle_add(lambda: (self._schedule_refresh(it.id), False)[1])
            raise ApiError("menu_failed", "Het menu van deze app kon niet worden geopend.") from None
        return r.get_child_value(0).get_uint32(), parse_menu_layout(r.get_child_value(1), self.write_png_bytes,
                                                                    parents)

    def menu_event(self, it: _Item, item_id: int, event: str) -> dict:
        if not it.menu:
            raise ApiError("no_menu", "Deze app heeft geen menu.")
        params = GLib.Variant("(isvu)", (item_id, event, GLib.Variant("i", 0), int(time.time()) & 0xFFFFFFFF))
        if event != "clicked":
            # hover/open/close are informational: never wait for the app
            self.bus.call(it.bus, it.menu, MENU_IFACE, "Event", params, None, Gio.DBusCallFlags.NO_AUTO_START,
                          CALL_TIMEOUT_MS, None, None, None)
            return {"ok": True}
        try:
            self._call(it, "Event", params, None, iface=MENU_IFACE, path=it.menu)
        except GLib.Error as e:
            kind = error_kind(e)
            if kind in ("gone", "timeout"):
                return {"ok": True}      # delivered: the app quit ("Afsluiten") or is busy handling it
            if kind == "noobject":
                raise self._menu_error(e, it) from None
            log.info("Systeemvak: menu-klik geweigerd door %s (menu event refused): %s", it.id, error_name(e))
            return {"ok": False}
        return {"ok": True}

    # -- calls ---------------------------------------------------------------------------------------
    def _async(self, dest: str, path: str, iface: str, method: str, params, reply: str | None, cb,
               timeout: int = CALL_TIMEOUT_MS) -> None:
        """Async call; ``cb(reply_variant | None, GLib.Error | None)`` runs in the GLib loop thread."""
        bus = self.bus
        if bus is None or self._stopped:
            return

        def done(conn, res, _data=None):
            try:
                r = conn.call_finish(res)
                err = None
            except GLib.Error as e:
                r, err = None, e
            if self._stopped:
                return
            try:
                cb(r, err)
            except Exception:  # noqa: BLE001 - a hostile reply never breaks the loop
                log.exception("Systeemvak: antwoord op %s niet verwerkt (reply handling failed)", method)

        try:
            bus.call(dest, path, iface, method, params, GLib.VariantType.new(reply) if reply else None,
                     Gio.DBusCallFlags.NO_AUTO_START, timeout, self._cancel, done, None)
        except Exception:  # noqa: BLE001
            log.debug("call %s failed", method, exc_info=True)

    def _call(self, it: _Item, method: str, params=None, reply: str | None = None, iface: str | None = None,
              path: str | None = None, timeout: int = CALL_TIMEOUT_MS):
        """Synchronous call for API handlers (HTTP threads). Raises GLib.Error."""
        bus = self.bus
        if bus is None or self.role == "none":
            raise ApiError("not_supported", "Het systeemvak is hier niet beschikbaar.")
        return bus.call_sync(it.bus, path or it.path, iface or it.iface or ITEM_IFACE, method, params,
                             GLib.VariantType.new(reply) if reply else None, Gio.DBusCallFlags.NO_AUTO_START,
                             min(int(timeout), CALL_TIMEOUT_MS), None)

    def _api_error(self, e: Exception, it: _Item) -> ApiError:
        kind = error_kind(e)
        if kind in ("gone", "noobject"):
            GLib.idle_add(lambda: (self._remove_item(it.id, "verdwenen (gone)"), False)[1])
            return ApiError("not_found", "Dit pictogram bestaat niet meer.")
        if kind == "timeout":
            return ApiError("timeout", "Deze app reageert niet.")
        return ApiError("tray_failed", "Deze app kon de opdracht niet uitvoeren.", {"error": error_name(e)[:200]})

    def _menu_error(self, e: Exception, it: _Item) -> ApiError:
        """An error on the item's dbusmenu object: only the bus name being gone removes the icon (a Qt app
        that replaced its QMenu still advertises the old path for a moment)."""
        if error_kind(e) == "noobject":
            GLib.idle_add(lambda: (self._schedule_refresh(it.id), False)[1])   # re-read Menu (or drop a dead item)
            return ApiError("no_menu", "Deze app heeft geen menu.")
        return self._api_error(e, it)

    def item_call(self, it: _Item, method: str, params) -> bool:
        """Activate & co: True = done, False = the app does not support it."""
        try:
            self._call(it, method, params)
            return True
        except GLib.Error as e:
            if error_kind(e) == "unsupported":
                # libdbus answers every call to a removed object with UnknownMethod as well: a refresh
                # (GetAll) tells a live item without this method from a vanished one
                GLib.idle_add(lambda: (self._schedule_refresh(it.id), False)[1])
                return False
            raise self._api_error(e, it) from None

    # -- taskbar.tray.known ---------------------------------------------------------------------------
    def _read_known(self) -> dict:
        try:
            known = self.ctx.settings.get("taskbar.tray.known", {}) or {}
        except Exception:  # noqa: BLE001
            known = {}
        return {k: v for k, v in known.items() if isinstance(k, str) and isinstance(v, dict)} \
            if isinstance(known, dict) else {}

    def _note_known(self, pub: dict) -> None:
        """New key, changed name/icon (at most hourly per key) or a day since ``seen`` → one batched write."""
        key, name, icon = pub["key"], pub["appName"], pub["appIcon"]
        now = self.now_ms()
        with self._known_lock:
            if self._known_cache is None:
                self._known_cache = self._read_known()
            cur = self._known_cache.get(key)
            if pub["status"] == "attention" and cur:
                icon = cur.get("icon") or icon     # a blinking attention icon is not the app's icon
            if cur is None:
                self._known_icon_at[key] = now
            elif (cur.get("name") != name or cur.get("icon") != icon) and \
                    now - self._known_icon_at.get(key, 0) >= ICON_REWRITE_MS:
                # name (an unmatched app's Title: "Download 3%") and icon churn: one write per hour
                self._known_icon_at[key] = now
            elif now - _int(cur.get("seen")) >= DAY_MS:
                icon = cur.get("icon") or icon     # only ``seen`` is refreshed
            else:
                return
            entry = {"name": name, "icon": icon, "seen": now}
            self._known_cache[key] = entry
            self._known_pending[key] = entry
        self._known_flush.trigger()

    def maintains_known(self) -> bool:
        """Only the primary instance keeps taskbar.tray.known: a second process (``serve`` next to the
        desktop) loaded settings.json once and would write its whole, stale snapshot back."""
        if self.ctx.mode in ("layer", "preview"):
            return True                    # __main__ refuses a second layer / preview instance
        try:
            inst = read_instance(check_alive=False)
        except Exception:  # noqa: BLE001
            return False
        return bool(inst) and inst.get("pid") == os.getpid()   # serve / app: only while it owns instance.json

    def _flush_known(self) -> None:
        with self._known_lock:
            pending, self._known_pending = self._known_pending, {}
        if not pending or not self.maintains_known():
            return
        known = self._read_known()
        known.update(pending)
        if len(known) > KNOWN_MAX:
            for k in sorted(known, key=lambda k: _int(known[k].get("seen")))[: len(known) - KNOWN_MAX]:
                known.pop(k, None)
        try:
            self.ctx.settings.set("taskbar.tray.known", known)
        except Exception as e:  # noqa: BLE001
            log.info("Systeemvak: lijst met bekende apps niet bewaard (known list not saved): %s", e)
            return
        with self._known_lock:
            self._known_cache = {**known, **self._known_pending}
            self._known_icon_at = {k: v for k, v in self._known_icon_at.items()
                                   if k in known or k in self._known_pending}


def _int(v) -> int:
    try:
        return int(v or 0)
    except (TypeError, ValueError):
        return 0


def _copy_public(p: dict) -> dict:
    d = dict(p)
    d["tooltip"] = dict(p["tooltip"])
    return d


def _coord(args: dict, name: str) -> int:
    try:
        return max(-100_000, min(100_000, int(args.get(name) or 0)))
    except (TypeError, ValueError):
        raise ApiError("bad_args", "Ongeldige positie.") from None


def _screen_point(ctx, args: dict) -> tuple[int, int]:
    """The UI sends shell-surface (monitor-local) x/y; SNI wants screen coordinates: in layer mode add the
    shell monitor's logical origin. Preview mode is a normal window whose position Wayland does not tell."""
    x, y = _coord(args, "x"), _coord(args, "y")
    s = getattr(ctx, "surfaces", None)
    if s is not None and getattr(s, "layer_active", False):
        try:
            ox, oy = getattr(s, "_monitor_origin", None) or (0, 0)
            x, y = max(-100_000, min(100_000, x + int(ox))), max(-100_000, min(100_000, y + int(oy)))
        except (TypeError, ValueError):
            pass
    return x, y


# ---------------------------------------------------------------------------
# register
# ---------------------------------------------------------------------------

def register_methods(api, svc: TrayService) -> None:
    @api.method("tray.list")
    def _list(args):
        """{supported, role, items: TrayItem[]} (registration order)."""
        return {"supported": svc.supported, "role": svc.role, "items": svc.list_items()}

    @api.method("tray.activate")
    def _activate(args):
        """Left click → Activate(x, y); {ok, fallback:'menu'} when the app only has a menu."""
        it = svc.find(args)
        x, y = _screen_point(svc.ctx, args)
        if it.public and it.public["itemIsMenu"] and it.public["hasMenu"]:
            return {"ok": True, "fallback": "menu"}
        if svc.item_call(it, "Activate", GLib.Variant("(ii)", (x, y))):
            return {"ok": True, "fallback": None}
        return {"ok": True, "fallback": "menu"}

    @api.method("tray.secondaryActivate")
    def _secondary(args):
        """Middle click → SecondaryActivate(x, y)."""
        it = svc.find(args)
        return {"ok": svc.item_call(it, "SecondaryActivate", GLib.Variant("(ii)", _screen_point(svc.ctx, args)))}

    @api.method("tray.scroll")
    def _scroll(args):
        """Mouse wheel → Scroll(delta, orientation); 120 per notch, up = + (libappindicator: ±1 per notch, GTK sign)."""
        it = svc.find(args)
        orientation = args.get("orientation") or "vertical"
        if orientation not in ("vertical", "horizontal"):
            raise ApiError("bad_args", "Ongeldige scrollrichting.")
        try:
            delta = max(-12_000, min(12_000, int(args.get("delta") or 0)))
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Ongeldige scrollwaarde.") from None
        if it.path.startswith(AYATANA_PATH):
            # libappindicator: the GTK sign (+ = down / right; KDE's, which tray.scroll takes, is the other way
            # round) and a step count — its scroll-event passes ABS(delta) on as "steps", so 1 per notch like
            # Unity, the GNOME AppIndicator extension and waybar send (120 would be 120 steps)
            steps = max(1, round(abs(delta) / 120)) if delta else 0
            delta = -steps if delta > 0 else steps
        return {"ok": svc.item_call(it, "Scroll", GLib.Variant("(is)", (delta, orientation)))}

    @api.method("tray.contextMenu")
    def _context_menu(args):
        """The app draws its own menu (ContextMenu(x, y)); only for items without a dbusmenu."""
        it = svc.find(args)
        return {"ok": svc.item_call(it, "ContextMenu", GLib.Variant("(ii)", _screen_point(svc.ctx, args)))}

    @api.method("tray.menu")
    def _menu(args):
        """{revision, items: MenuItem[]} (AboutToShow + AboutToShowGroup first, then GetLayout; with
        ``refresh: true`` only GetLayout — the live refresh of an open menu)."""
        return svc.menu(svc.find(args), refresh=args.get("refresh") is True)

    @api.method("tray.menuEvent")
    def _menu_event(args):
        """A menu entry was clicked / hovered, or the menu opened / closed."""
        it = svc.find(args)
        event = args.get("event") or "clicked"
        if event not in MENU_EVENTS:
            raise ApiError("bad_args", "Onbekende menu-actie.")
        try:
            item_id = int(args.get("itemId"))
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Ongeldig menu-item.") from None
        if not (0 <= item_id <= 0x7FFFFFFF):
            raise ApiError("bad_args", "Ongeldig menu-item.")
        return svc.menu_event(it, item_id, event)


def register(api, ctx) -> None:
    svc = TrayService(api, ctx)
    ctx._tray = svc
    register_methods(api, svc)


def start(api, ctx) -> None:
    svc = getattr(ctx, "_tray", None)
    if svc is not None:
        svc.start()


def stop(api, ctx) -> None:
    svc = getattr(ctx, "_tray", None)
    if svc is not None:
        svc.stop()
