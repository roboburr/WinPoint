"""system.* (SPEC §6.10): audio, brightness, battery, network, bluetooth, power, night light,
device info, updates and time.

Sources (no root, no shell, every external command gets an argument list):
  audio       PipeWire via ``wpctl`` (get-volume / set-volume -l 1.0 / set-mute / status / set-default)
  brightness  /sys/class/backlight + logind ``Session.SetBrightness`` (DDC monitors: out of scope)
  battery     UPower D-Bus (DisplayDevice + battery devices), cycles from /sys/class/power_supply
  network     NetworkManager D-Bus for state (cheap, signal driven) and Wi-Fi connect (the password goes to
              NM over D-Bus, never on a command line) + ``nmcli`` for scan / disconnect / radio
  bluetooth   BlueZ D-Bus (ObjectManager, Adapter1, Device1, Battery1); pairing with a temporary
              org.bluez.Agent1 of our own (only while we pair); reset via rfkill (/dev/rfkill ACL, no root)
  power       systemctl / loginctl / hyprctl; lock = hyprlock with our lock screen when hypridle is absent
  nightLight  hyprsunset (Hyprland only)
  info        /etc/os-release, uname, /sys/class/dmi/id, /proc/cpuinfo, /proc/meminfo, nvidia-smi/lspci,
              lsblk, /proc/uptime, pwd (GECOS), AccountsService icon / ~/.face
  updates     ``apt list --upgradable`` (background thread, cached 30 min)
  time        ``timedatectl show``

A background poller (2.5 s, woken early by D-Bus PropertiesChanged signals) emits
``system.<domain>.changed`` with the same payload as ``system.<domain>.get`` – only when something
changed and only while an SSE client is connected.

Additive (beyond SPEC, documented in notes/host-services-b.md): ``system.audio.set {delta, muted:'toggle'}``,
``system.brightness.set {delta}``, ``system.nightLight.get``, ``system.network.disconnect {iface?}``,
event ``system.updates.changed``, extra fields marked "extra" below.
"""

from __future__ import annotations

import codecs
import json
import logging
import os
import platform
import re
import shlex
import socket
import subprocess
import threading
import time
from pathlib import Path
from typing import Any, Callable

from ..util import ApiError, compositor, dbus_call, hypr_dispatch, hyprctl, dry_for, json_dumps, run, \
    spawn, system_bus, user_info, which

log = logging.getLogger("win11shell.system")

PRIORITY = 60
POLL_S = 2.5
AUDIO_MONITORED_EVERY = 8      # with the PipeWire monitor: audio safety poll every 8 ticks (20 s)
UPDATES_TTL_S = 30 * 60

_lang: Callable[[], str] = lambda: "nl"

_STR = {
    "nl": {
        "ethernet": "Ethernet",
        "wifi": "Wi-Fi",
        "hidden": "Verborgen netwerk",
        "desktop_hypr": "Hyprland",
        "desktop_gnome": "GNOME",
    },
    "en": {
        "ethernet": "Ethernet",
        "wifi": "Wi-Fi",
        "hidden": "Hidden network",
        "desktop_hypr": "Hyprland",
        "desktop_gnome": "GNOME",
    },
}


def _tr(key: str, **kw) -> str:
    table = _STR.get(_lang(), _STR["nl"])
    text = table.get(key) or _STR["nl"].get(key) or key
    return text.format(**kw) if kw else text


# ---------------------------------------------------------------------------
# small helpers
# ---------------------------------------------------------------------------

def _read(path: str | Path, default: str | None = None) -> str | None:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            return f.read().strip()
    except OSError:
        return default


def _int(v: Any, default: int | None = None) -> int | None:
    try:
        return int(str(v).strip())
    except (TypeError, ValueError):
        return default


def _num(v: Any) -> float:
    if isinstance(v, bool) or v is None:
        raise ApiError("bad_args", "Ongeldige waarde.")
    try:
        f = float(v)
    except (TypeError, ValueError):
        raise ApiError("bad_args", "Ongeldige waarde.") from None
    if f != f or f in (float("inf"), float("-inf")):
        raise ApiError("bad_args", "Ongeldige waarde.")
    return f


def _pct(v: Any) -> int:
    """0–100 integer percent (also accepts 0.0–1.0 fractions given as float < 1)."""
    f = _num(v)
    if isinstance(v, float) and 0 < f < 1:
        f *= 100
    return max(0, min(100, int(round(f))))


def _bool(v: Any) -> bool:
    if isinstance(v, bool):
        return v
    if isinstance(v, (int, float)):
        return bool(v)
    if isinstance(v, str) and v.lower() in ("1", "true", "yes", "on", "aan"):
        return True
    if isinstance(v, str) and v.lower() in ("0", "false", "no", "off", "uit", ""):
        return False
    raise ApiError("bad_args", "Ongeldige waarde.")


_gi_cache: tuple | None = None


def _gi():
    global _gi_cache
    if _gi_cache is None:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio, GLib
        _gi_cache = (Gio, GLib)
    return _gi_cache


_sysbus_obj = None
_sysbus_lock = threading.Lock()


def _sysbus():
    global _sysbus_obj
    with _sysbus_lock:
        if _sysbus_obj is None:
            _sysbus_obj = system_bus()
        return _sysbus_obj


def _getall(name: str, path: str, iface: str, timeout_ms: int = 1500) -> dict:
    r = dbus_call(_sysbus(), name, path, "org.freedesktop.DBus.Properties", "GetAll", (iface,),
                  signature="(s)", timeout_ms=timeout_ms)
    return r[0] if r else {}


def _dbus_error_name(e: Exception) -> str:
    try:
        Gio, _ = _gi()
        if hasattr(Gio, "DBusError") and Gio.DBusError.is_remote_error(e):
            return Gio.DBusError.get_remote_error(e) or ""
    except Exception:  # noqa: BLE001
        pass
    m = re.search(r"(org\.[A-Za-z0-9_.]+Error[A-Za-z0-9_.]*)", str(e))
    return m.group(1) if m else ""


def _proc_running(*names: str) -> bool:
    """True when a process of the current user with one of these comm names runs."""
    uid = os.getuid()
    wanted = {n[:15] for n in names}
    try:
        entries = os.listdir("/proc")
    except OSError:
        return False
    for e in entries:
        if not e.isdigit():
            continue
        try:
            if os.stat(f"/proc/{e}").st_uid != uid:
                continue
            with open(f"/proc/{e}/comm", "r") as f:
                if f.read().strip() in wanted:
                    return True
        except OSError:
            continue
    return False


# ---------------------------------------------------------------------------
# Audio (PipeWire / wpctl)
# ---------------------------------------------------------------------------

_VOL_RE = re.compile(r"Volume:\s*([0-9]+(?:\.[0-9]+)?)(\s*\[MUTED\])?")
_SECTION_RE = re.compile(r"^[\s│├└─]*(Devices|Sinks|Sources|Filters|Streams):\s*$")
_ENTRY_RE = re.compile(r"^(\*)?\s*(\d+)\.\s+(.*?)\s*(\[[^\]]*\])?\s*$")
_DEFAULT_SINK = "@DEFAULT_AUDIO_SINK@"


def parse_wpctl_status(text: str) -> tuple[list[dict], int | None]:
    """→ (sinks [{id, name, default}], id of the default *filter* sink or None). Pure function (tested)."""
    sinks: list[dict] = []
    filter_default: int | None = None
    in_audio = False
    section = None
    for raw in text.splitlines():
        stripped = raw.strip()
        if stripped in ("Audio", "Video", "Settings"):
            in_audio = stripped == "Audio"
            section = None
            continue
        if not in_audio:
            continue
        m = _SECTION_RE.match(raw)
        if m:
            section = m.group(1)
            continue
        if section not in ("Sinks", "Filters"):
            continue
        body = raw.lstrip(" \t│├└─")
        m = _ENTRY_RE.match(body)
        if not m:
            continue
        star, sid, name, tag = m.group(1), int(m.group(2)), m.group(3).strip(), (m.group(4) or "")
        if section == "Sinks":
            sinks.append({"id": sid, "name": name, "default": bool(star)})
        elif star and tag == "[Audio/Sink]":
            filter_default = sid
    return sinks, filter_default


def parse_wpctl_filter_sinks(text: str) -> list[int]:
    """Ids of the [Audio/Sink] filters in the Audio part of ``wpctl status`` (Bluetooth loopbacks …)."""
    out: list[int] = []
    in_audio = False
    section = None
    for raw in text.splitlines():
        stripped = raw.strip()
        if stripped in ("Audio", "Video", "Settings"):
            in_audio = stripped == "Audio"
            section = None
            continue
        m = _SECTION_RE.match(raw) if in_audio else None
        if m:
            section = m.group(1)
            continue
        if in_audio and section == "Filters":
            m = _ENTRY_RE.match(raw.lstrip(" \t│├└─"))
            if m and (m.group(4) or "") == "[Audio/Sink]":
                out.append(int(m.group(2)))
    return out


def _prop(text: str, key: str) -> str | None:
    m = re.search(r"^\s*\*?\s*" + re.escape(key) + r'\s*=\s*"([^"]*)"', text, re.M)
    return m.group(1) if m else None


class Audio:
    def __init__(self):
        self._lock = threading.Lock()
        self._sinks: list[dict] = []
        self._sig = ""

    @staticmethod
    def available() -> bool:
        return bool(which("wpctl"))

    @staticmethod
    def _unavailable() -> dict:
        return {"available": False, "volume": 0, "muted": False, "sinks": []}

    def _volume(self) -> tuple[int, bool] | None:
        r = run(["wpctl", "get-volume", _DEFAULT_SINK], timeout=2)
        m = _VOL_RE.search(r.out) if r.ok else None
        if not m:
            return None
        return int(round(float(m.group(1)) * 100)), bool(m.group(2))

    def sinks(self) -> list[dict]:
        r = run(["wpctl", "status"], timeout=3)
        if not r.ok:
            return []
        sinks, filter_default = parse_wpctl_status(r.out)
        sig = json.dumps([sinks, filter_default], sort_keys=True)
        with self._lock:
            if sig == self._sig and self._sinks:
                return [dict(s) for s in self._sinks]   # unchanged: skip the extra inspect call
        if sinks and not any(s["default"] for s in sinks):
            # The default node is a filter (e.g. a Bluetooth "smart filter" loopback in WirePlumber 0.5+):
            # mark the real sink it belongs to by its description.
            desc = ""
            ins = run(["wpctl", "inspect", _DEFAULT_SINK], timeout=2)
            if ins.ok:
                m = re.search(r'node\.description\s*=\s*"([^"]*)"', ins.out)
                desc = m.group(1).strip() if m else ""
                m2 = re.search(r"^id\s+(\d+)", ins.out, re.M)
                did = int(m2.group(1)) if m2 else -1
                for s in sinks:
                    if s["id"] == did:
                        s["default"] = True
                        break
                else:
                    if desc:
                        for s in sinks:
                            if s["name"] == desc or s["name"].startswith(desc):
                                s["default"] = True
                                break
        with self._lock:
            self._sinks = sinks
            self._sig = sig
        return [dict(s) for s in sinks]

    def get(self, fresh_sinks: bool = True) -> dict:
        if not self.available():
            return self._unavailable()
        vm = self._volume()
        if vm is None:
            return self._unavailable()
        if fresh_sinks:
            sinks = self.sinks()
        else:
            with self._lock:
                sinks = [dict(s) for s in self._sinks]
        return {"available": True, "volume": vm[0], "muted": vm[1], "sinks": sinks}

    def set(self, args: dict) -> dict:
        cur = self.get(fresh_sinks=False)
        if not cur["available"]:
            raise ApiError("unavailable", "Er is geen geluidsapparaat gevonden.")
        vol = args.get("volume")
        delta = args.get("delta")
        muted = args.get("muted")
        if delta is not None:
            vol = cur["volume"] + _num(delta)
        if isinstance(muted, str) and muted.lower() == "toggle":
            muted = not cur["muted"]
        elif muted is not None:
            muted = _bool(muted)
        if vol is not None:
            v = _pct(vol) if delta is None else max(0, min(100, int(round(_num(vol)))))
            r = run(["wpctl", "set-volume", "-l", "1.0", _DEFAULT_SINK, f"{v / 100:.2f}"], timeout=3)
            if not r.ok:
                raise ApiError("failed", "Het volume kon niet worden aangepast.")
            if muted is None and cur["muted"] and v > 0:
                muted = False  # moving the slider unmutes, like Windows
        if muted is not None:
            r = run(["wpctl", "set-mute", _DEFAULT_SINK, "1" if muted else "0"], timeout=3)
            if not r.ok:
                raise ApiError("failed", "Dempen lukte niet.")
        return self.get(fresh_sinks=False)

    @staticmethod
    def _default_target(sid: int) -> int:
        """The node to make the default for sink ``sid``. WirePlumber 0.5+ puts a "smart filter" loopback in
        front of Bluetooth output: the headset's own node is Audio/Sink/Internal, which ``wpctl set-default``
        refuses ("is not a device node"); the sink to choose is the filter (media.class Audio/Sink) driven by
        that node (node.driver-id) or belonging to the same device (device.id)."""
        ins = run(["wpctl", "inspect", str(sid)], timeout=2)
        if not ins.ok or _prop(ins.out, "media.class") != "Audio/Sink/Internal":
            return sid
        dev = _prop(ins.out, "device.id")
        st = run(["wpctl", "status"], timeout=3)
        for fid in parse_wpctl_filter_sinks(st.out if st.ok else ""):
            fi = run(["wpctl", "inspect", str(fid)], timeout=2)
            if not fi.ok or _prop(fi.out, "media.class") != "Audio/Sink":
                continue
            if _prop(fi.out, "node.driver-id") == str(sid) or (dev and _prop(fi.out, "device.id") == dev):
                return fid
        return sid

    def set_sink(self, sid: Any) -> dict:
        try:
            sid = int(sid)
        except (TypeError, ValueError):
            raise ApiError("bad_args", "Onbekend geluidsapparaat.") from None
        if not any(s["id"] == sid for s in self.sinks()):
            raise ApiError("not_found", "Dit geluidsapparaat is niet (meer) aangesloten.")
        r = run(["wpctl", "set-default", str(self._default_target(sid))], timeout=3)
        if not r.ok:
            raise ApiError("failed", "Overschakelen naar dit apparaat lukte niet.")
        time.sleep(0.15)
        return self.get()


# ---------------------------------------------------------------------------
# Brightness (sysfs + logind)
# ---------------------------------------------------------------------------

_LOGIN1 = "org.freedesktop.login1"
_BL_RANK = {"firmware": 0, "platform": 1, "raw": 2}


def _backlight() -> tuple[str, Path, int] | None:
    base = Path("/sys/class/backlight")
    try:
        entries = list(base.iterdir())
    except OSError:
        return None
    found = []
    for d in entries:
        mx = _int(_read(d / "max_brightness"), 0) or 0
        if mx <= 0:
            continue
        found.append((_BL_RANK.get(_read(d / "type", "raw") or "raw", 3), d.name, d, mx))
    if not found:
        return None
    found.sort()
    _, name, d, mx = found[0]
    return name, d, mx


def _session_path() -> str:
    """logind session object path of the current graphical session."""
    try:
        bus = _sysbus()
    except Exception:  # noqa: BLE001
        return "/org/freedesktop/login1/session/auto"
    sid = os.environ.get("XDG_SESSION_ID")
    if sid:
        try:
            return dbus_call(bus, _LOGIN1, "/org/freedesktop/login1", "org.freedesktop.login1.Manager",
                             "GetSession", (sid,), signature="(s)")[0]
        except Exception:  # noqa: BLE001
            pass
    try:
        return dbus_call(bus, _LOGIN1, "/org/freedesktop/login1", "org.freedesktop.login1.Manager",
                         "GetSessionByPID", (os.getpid(),), signature="(u)")[0]
    except Exception:  # noqa: BLE001
        pass
    try:
        sessions = dbus_call(bus, _LOGIN1, "/org/freedesktop/login1", "org.freedesktop.login1.Manager",
                             "ListSessions", None)[0]
        mine = [s for s in sessions if s[1] == os.getuid() and s[3]]  # (id, uid, user, seat, path)
        for s in mine:
            try:
                if _getall(_LOGIN1, s[4], "org.freedesktop.login1.Session").get("Active"):
                    return s[4]
            except Exception:  # noqa: BLE001
                continue
        if mine:
            return mine[0][4]
    except Exception:  # noqa: BLE001
        pass
    return "/org/freedesktop/login1/session/auto"


def _session_id() -> str | None:
    sid = os.environ.get("XDG_SESSION_ID")
    if sid:
        return sid
    try:
        props = _getall(_LOGIN1, _session_path(), "org.freedesktop.login1.Session")
        return str(props.get("Id")) if props.get("Id") else None
    except Exception:  # noqa: BLE001
        return None


def brightness_get() -> dict:
    bl = _backlight()
    if not bl:
        return {"available": False, "value": None}
    name, d, mx = bl
    cur = _int(_read(d / "brightness"), None)
    if cur is None:
        return {"available": False, "value": None}
    return {"available": True, "value": int(round(cur * 100 / mx)), "device": name}


def brightness_set(args: dict) -> dict:
    bl = _backlight()
    if not bl:
        raise ApiError("unsupported", "De helderheid van dit scherm kan hier niet worden aangepast.")
    name, d, mx = bl
    if args.get("delta") is not None:
        cur = brightness_get().get("value") or 0
        v = max(0, min(100, int(round(cur + _num(args["delta"])))))
    elif args.get("value") is not None:
        v = _pct(args["value"])
    else:
        raise ApiError("bad_args", "Geef een helderheid op.")
    raw = max(1, int(round(mx * v / 100)))  # 0 would switch some panels off completely
    try:
        dbus_call(_sysbus(), _LOGIN1, _session_path(), "org.freedesktop.login1.Session", "SetBrightness",
                  ("backlight", name, raw), signature="(ssu)", timeout_ms=3000)
    except Exception as e:  # noqa: BLE001
        log.info("SetBrightness mislukt (failed): %s", e)
        raise ApiError("failed", "De helderheid kon niet worden aangepast.") from None
    return {"available": True, "value": v, "device": name}


# ---------------------------------------------------------------------------
# Battery (UPower)
# ---------------------------------------------------------------------------

_UP = "org.freedesktop.UPower"
_UP_STATES = {0: "unknown", 1: "charging", 2: "discharging", 3: "empty", 4: "full", 5: "pending-charge",
              6: "pending-discharge"}


class Battery:
    def __init__(self):
        self._health: tuple[float, Any, Any] = (0.0, None, None)

    @staticmethod
    def _none(on_ac: bool = True) -> dict:
        return {"present": False, "percent": None, "state": "none", "timeToEmpty": None, "timeToFull": None,
                "onAC": on_ac, "health": None, "cycles": None, "energyRate": None}

    def _health_cycles(self) -> tuple[int | None, int | None]:
        now = time.monotonic()
        if now - self._health[0] < 300:
            return self._health[1], self._health[2]
        health = cycles = None
        try:
            paths = dbus_call(_sysbus(), _UP, "/org/freedesktop/UPower", _UP, "EnumerateDevices", None)[0]
            for p in paths:
                d = _getall(_UP, p, _UP + ".Device")
                if d.get("Type") != 2 or not d.get("PowerSupply"):
                    continue
                full, design = d.get("EnergyFull") or 0, d.get("EnergyFullDesign") or 0
                if full > 0 and design > 0:
                    health = int(round(min(100.0, full * 100.0 / design)))
                if d.get("ChargeCycles", 0) and d["ChargeCycles"] > 0:
                    cycles = int(d["ChargeCycles"])
                break
        except Exception:  # noqa: BLE001
            pass
        try:
            for bat in sorted(Path("/sys/class/power_supply").glob("BAT*")):
                c = _int(_read(bat / "cycle_count"), None)
                if c is not None and c > 0:
                    cycles = c
                if health is None:
                    full = _int(_read(bat / "energy_full"), None) or _int(_read(bat / "charge_full"), None)
                    design = (_int(_read(bat / "energy_full_design"), None)
                              or _int(_read(bat / "charge_full_design"), None))
                    if full and design:
                        health = int(round(min(100.0, full * 100.0 / design)))
                break
        except OSError:
            pass
        self._health = (now, health, cycles)
        return health, cycles

    def get(self) -> dict:
        try:
            dd = _getall(_UP, "/org/freedesktop/UPower/devices/DisplayDevice", _UP + ".Device")
            root = _getall(_UP, "/org/freedesktop/UPower", _UP)
        except Exception:  # noqa: BLE001
            return self._none()
        on_ac = not bool(root.get("OnBattery"))
        if not dd.get("IsPresent") or dd.get("Type") not in (2, None):
            return self._none(on_ac)
        health, cycles = self._health_cycles()
        tte = int(dd.get("TimeToEmpty") or 0)
        ttf = int(dd.get("TimeToFull") or 0)
        rate = float(dd.get("EnergyRate") or 0.0)
        return {
            "present": True,
            "percent": int(round(float(dd.get("Percentage") or 0))),
            "state": _UP_STATES.get(int(dd.get("State") or 0), "unknown"),
            "timeToEmpty": tte or None,
            "timeToFull": ttf or None,
            "onAC": on_ac,
            "health": health,
            "cycles": cycles,
            "energyRate": round(rate, 2) if rate else None,
        }


# ---------------------------------------------------------------------------
# Network (NetworkManager)
# ---------------------------------------------------------------------------

_NM = "org.freedesktop.NetworkManager"
_NMP = "/org/freedesktop/NetworkManager"
_NM_CONNECTIVITY = {0: "unknown", 1: "none", 2: "portal", 3: "limited", 4: "full"}
_VIRTUAL_IF = ("lo", "veth", "docker", "br-", "virbr", "vnet", "tap", "tun", "tailscale", "wg", "zt", "vmnet",
               "vboxnet", "lxc", "lxd", "podman", "cni", "flannel")
_GENERIC_CONN = re.compile(r"^(netplan-.*|(wired|bekabelde|kabel)\s*(connection|verbinding)\s*\d*|"
                           r"(kabel)?verbinding\s*\d*|ethernet.*|auto.*)$", re.I)


def split_terse(line: str) -> list[str]:
    """Split an ``nmcli -t`` line on unescaped ':' (handles '\\:' and '\\\\'). Pure (tested)."""
    out, cur, i = [], [], 0
    while i < len(line):
        c = line[i]
        if c == "\\" and i + 1 < len(line):
            cur.append(line[i + 1])
            i += 2
            continue
        if c == ":":
            out.append("".join(cur))
            cur = []
        else:
            cur.append(c)
        i += 1
    out.append("".join(cur))
    return out


def _ssid_bytes(v: Any) -> str:
    if isinstance(v, (bytes, bytearray)):
        b = bytes(v)
    elif isinstance(v, (list, tuple)):
        b = bytes(int(x) & 0xFF for x in v)
    else:
        return str(v or "")
    return b.decode("utf-8", "replace")


class Network:
    def __init__(self):
        self._lock = threading.Lock()
        self._devices: tuple[float, list[dict]] = (0.0, [])
        self._saved: tuple[float, dict[str, list[str]]] = (0.0, {})

    def invalidate(self) -> None:
        with self._lock:
            self._devices = (0.0, [])

    def _device_list(self) -> list[dict]:
        now = time.monotonic()
        with self._lock:
            if now - self._devices[0] < 20 and self._devices[1]:
                return self._devices[1]
        devs = []
        for p in dbus_call(_sysbus(), _NM, _NMP, _NM, "GetDevices", None)[0]:
            try:
                d = _getall(_NM, p, _NM + ".Device")
                devs.append({"path": p, "type": int(d.get("DeviceType") or 0), "iface": d.get("Interface") or "",
                             "state": int(d.get("State") or 0), "active": d.get("ActiveConnection") or "/"})
            except Exception:  # noqa: BLE001
                continue
        with self._lock:
            self._devices = (now, devs)
        return devs

    def _active(self, path: str, devices: list[dict]) -> dict | None:
        if not path or path == "/":
            return None
        try:
            a = _getall(_NM, path, _NM + ".Connection.Active")
        except Exception:  # noqa: BLE001
            return None
        ctype = a.get("Type") or ""
        dev_paths = list(a.get("Devices") or [])
        iface = next((d["iface"] for d in devices if d["path"] in dev_paths), "")
        info = {"ctype": ctype, "id": a.get("Id") or "", "state": int(a.get("State") or 0), "iface": iface,
                "default": bool(a.get("Default") or a.get("Default6")), "ap": a.get("SpecificObject") or "/"}
        if ctype == "802-11-wireless":
            info["type"] = "wifi"
        elif ctype in ("802-3-ethernet", "pppoe"):
            info["type"] = "ethernet"
        else:
            info["type"] = "other"
        return info

    def _ap(self, path: str) -> tuple[str, int | None]:
        if not path or path == "/":
            return "", None
        try:
            ap = _getall(_NM, path, _NM + ".AccessPoint")
            return _ssid_bytes(ap.get("Ssid")), int(ap.get("Strength") or 0)
        except Exception:  # noqa: BLE001
            return "", None

    def get(self) -> dict:
        airplane = _airplane_state()
        try:
            p = _getall(_NM, _NMP, _NM)
            devices = self._device_list()
        except Exception:  # noqa: BLE001
            return {"connectivity": "unknown", "wifiAvailable": False, "wifiEnabled": False, "airplane": airplane,
                    "primary": {"type": "none", "name": "", "strength": None}, "available": False}
        wifi_devs = [d for d in devices if d["type"] == 2]
        prim = self._active(p.get("PrimaryConnection") or "/", devices)
        if not prim or prim["type"] not in ("wifi", "ethernet") or prim["state"] != 2:
            prim = None
            cands = []
            for ac in p.get("ActiveConnections") or []:
                info = self._active(ac, devices)
                if not info or info["type"] not in ("wifi", "ethernet") or info["state"] != 2:
                    continue
                if info["iface"].startswith(_VIRTUAL_IF):
                    continue
                cands.append(info)
            cands.sort(key=lambda i: (not i["default"], i["type"] != "wifi"))
            prim = cands[0] if cands else None
        primary: dict = {"type": "none", "name": "", "strength": None}
        if prim:
            if prim["type"] == "wifi":
                ssid, strength = self._ap(prim["ap"])
                primary = {"type": "wifi", "name": ssid or prim["id"], "strength": strength}
            else:
                cid = prim["id"]
                name = _tr("ethernet") if (not cid or cid == prim["iface"] or _GENERIC_CONN.match(cid)) else cid
                primary = {"type": "ethernet", "name": name, "strength": None}
            primary["device"] = prim["iface"]          # extra
            primary["connection"] = prim["id"]         # extra
        return {
            "connectivity": _NM_CONNECTIVITY.get(int(p.get("Connectivity") or 0), "unknown"),
            "wifiAvailable": bool(wifi_devs),
            "wifiEnabled": bool(p.get("WirelessEnabled")) and bool(wifi_devs),
            "airplane": airplane,
            "primary": primary,
            "available": True,                                     # extra
            "wifiHardwareEnabled": bool(p.get("WirelessHardwareEnabled")),  # extra
        }

    # -- Wi-Fi ---------------------------------------------------------------
    def saved_ssids(self, max_age: float = 30.0) -> dict[str, list[str]]:
        """{ssid: [connection object paths]} of saved Wi-Fi profiles."""
        now = time.monotonic()
        with self._lock:
            if now - self._saved[0] < max_age:
                return self._saved[1]
        out: dict[str, list[str]] = {}
        try:
            paths = dbus_call(_sysbus(), _NM, _NMP + "/Settings", _NM + ".Settings", "ListConnections", None)[0]
            for cp in paths:
                try:
                    s = dbus_call(_sysbus(), _NM, cp, _NM + ".Settings.Connection", "GetSettings", None)[0]
                except Exception:  # noqa: BLE001
                    continue
                w = s.get("802-11-wireless")
                if w and w.get("ssid") is not None:
                    out.setdefault(_ssid_bytes(w.get("ssid")), []).append(cp)
        except Exception:  # noqa: BLE001
            pass
        with self._lock:
            self._saved = (now, out)
        return out

    def wifi_list(self, rescan: bool) -> list[dict]:
        if not which("nmcli"):
            return []
        try:
            if not any(d["type"] == 2 for d in self._device_list()):
                return []
        except Exception:  # noqa: BLE001
            pass
        r = run(["nmcli", "-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "device", "wifi", "list", "--rescan",
                 "yes" if rescan else "no"], timeout=20 if rescan else 6, env=_c_env())
        if not r.ok:
            return []
        saved = self.saved_ssids()
        best: dict[str, dict] = {}
        for line in r.out.splitlines():
            if not line.strip():
                continue
            f = split_terse(line)
            if len(f) < 4:
                continue
            ssid = f[1]
            if not ssid or ssid == "--":
                continue
            sec = f[3].strip()
            item = {"ssid": ssid, "strength": _int(f[2], 0) or 0, "secure": bool(sec and sec != "--"),
                    "active": f[0].strip() == "*", "saved": ssid in saved, "security": "" if sec == "--" else sec}
            old = best.get(ssid)
            if old is None or (item["active"] and not old["active"]) \
                    or (not old["active"] and item["strength"] > old["strength"]):
                best[ssid] = item
        return sorted(best.values(), key=lambda i: (not i["active"], -i["strength"], i["ssid"].lower()))

    def connect(self, ssid: Any, password: Any) -> dict:
        """Connect to a Wi-Fi network through NetworkManager's D-Bus API.

        The password goes over the system bus to NetworkManager only (never on a command line, where
        every local user could read it in /proc/<pid>/cmdline). Result: ``{ok:true}`` or
        ``{ok:false, code:'needs_password'|'wrong_password'|'not_found'|'failed', error, reason?}``."""
        if not isinstance(ssid, str) or not ssid or len(ssid.encode("utf-8")) > 32:
            raise ApiError("bad_args", "Ongeldige netwerknaam.")
        if password is not None and (not isinstance(password, str) or len(password) > 64 or "\x00" in password):
            raise ApiError("bad_args", "Ongeldig wachtwoord.")
        password = password or None
        try:
            bus = _sysbus()
            dev = _nm_wifi_device(self._device_list())
        except Exception as e:  # noqa: BLE001
            log.info("NetworkManager niet bereikbaar (unreachable): %s", e)
            return wifi_fail("failed", "unavailable")
        if dev is None:
            return wifi_fail("failed", "no_adapter")
        ap = _nm_find_ap(bus, dev["path"], ssid)
        if ap is None:
            _nm_request_scan(bus, dev["path"])
            time.sleep(3.0)
            ap = _nm_find_ap(bus, dev["path"], ssid)
        if ap is None:
            return wifi_fail("not_found")
        sec = ap["security"]
        if sec in ("enterprise", "wep"):
            return wifi_fail("failed", sec)
        saved = self.saved_ssids(max_age=0).get(ssid, [])
        needs_key = sec in ("wpa-psk", "sae")
        if needs_key and not password and not saved:
            return wifi_fail("needs_password")
        if needs_key and password and not psk_valid(password):
            return wifi_fail("wrong_password", "psk_format")
        created = None
        try:
            with _DeviceWatch(bus, dev["path"]) as watch:
                if saved:
                    conn = saved[0]
                    if password and needs_key:
                        _nm_update_psk(bus, conn, sec, password)
                    active = dbus_call(bus, _NM, _NMP, _NM, "ActivateConnection", (conn, dev["path"], ap["path"]),
                                       signature="(ooo)", timeout_ms=15000)[0]
                else:
                    settings = wifi_settings_variant(ssid, sec, password)
                    _Gio, GLib = _gi()
                    params = GLib.Variant.new_tuple(settings, GLib.Variant("o", dev["path"]),
                                                    GLib.Variant("o", ap["path"]))
                    created, active = dbus_call(bus, _NM, _NMP, _NM, "AddAndActivateConnection", params,
                                                timeout_ms=15000)
                ok, reason = watch.wait(active, timeout=45.0)
        except Exception as e:  # noqa: BLE001 - D-Bus error before activation started
            name = _dbus_error_name(e) or str(e)
            log.info("Wi-Fi verbinden mislukt (connect failed): %s", name[:200])
            self.invalidate()
            if "InvalidProperty" in name or "psk" in name.lower():
                return wifi_fail("wrong_password", "invalid_property")
            if "PermissionDenied" in name or "NotAuthorized" in name or "AccessDenied" in name:
                return wifi_fail("failed", "denied")
            return wifi_fail("failed", "dbus")
        self.invalidate()
        with self._lock:
            self._saved = (0.0, {})
        if ok:
            log.info("Verbonden met Wi-Fi (connected)")
            return {"ok": True}
        code, why = wifi_reason_code(reason, bool(password), bool(saved))
        log.info("Wi-Fi verbinden mislukt (connect failed): device reason %s → %s", reason, code)
        if created:   # remove the half-made profile we just added (never the user's saved ones)
            try:
                dbus_call(bus, _NM, created, _NM + ".Settings.Connection", "Delete", None)
            except Exception:  # noqa: BLE001
                pass
        return wifi_fail(code, why)

    def disconnect(self, iface: Any = None) -> dict:
        devices = self._device_list()
        wifi = [d for d in devices if d["type"] == 2 and d["active"] not in ("", "/")]
        if iface:
            wifi = [d for d in devices if d["iface"] == iface]
            if not wifi:
                raise ApiError("not_found", "Deze netwerkadapter bestaat niet.")
        if not wifi:
            return {"ok": True}
        r = run(["nmcli", "device", "disconnect", wifi[0]["iface"]], timeout=10, env=_c_env())
        self.invalidate()
        if not r.ok:
            raise ApiError("failed", "Verbinding verbreken lukte niet.")
        return {"ok": True}

    def set_wifi(self, enabled: bool) -> dict:
        r = run(["nmcli", "radio", "wifi", "on" if enabled else "off"], timeout=8, env=_c_env())
        if not r.ok:
            raise ApiError("failed", "Wi-Fi aan- of uitzetten lukte niet.")
        return self.get()


# -- Wi-Fi connect over NetworkManager D-Bus (pure helpers are tested) ------------------------------

_NM_DEVICE_ACTIVATED, _NM_DEVICE_FAILED, _NM_DEVICE_DISCONNECTED = 100, 120, 30
_AP_PRIVACY = 0x1
_KM_PSK, _KM_8021X, _KM_SAE, _KM_OWE, _KM_OWE_TM, _KM_EAP_B192 = 0x100, 0x200, 0x400, 0x800, 0x1000, 0x2000

_WIFI_TEXT = {
    "needs_password": ("Voer het wachtwoord van dit netwerk in.", "Enter the password for this network."),
    "wrong_password": ("Het wachtwoord klopt niet. Probeer het opnieuw.", "The password is incorrect. Try again."),
    "psk_format": ("Een Wi-Fi-wachtwoord heeft 8 tot 63 tekens.", "A Wi-Fi password has 8 to 63 characters."),
    "not_found": ("Dit netwerk is niet (meer) in de buurt.", "This network is out of range."),
    "failed": ("Verbinden is niet gelukt.", "Couldn't connect."),
    "timeout": ("Verbinden duurde te lang. Probeer het opnieuw.", "Connecting took too long. Try again."),
    "denied": ("Je hebt geen toestemming om met dit netwerk te verbinden.",
               "You're not allowed to connect to this network."),
    "enterprise": ("Dit netwerk vraagt een aanmelding met gebruikersnaam (zakelijk netwerk). Dat kan hier nog niet.",
                   "This network needs a work or school sign-in, which isn't supported here yet."),
    "wep": ("Dit netwerk gebruikt een verouderde, onveilige beveiliging (WEP) en wordt niet ondersteund.",
            "This network uses outdated, unsafe security (WEP) and isn't supported."),
    "ip": ("Verbonden, maar het netwerk gaf geen adres. Probeer het later opnieuw.",
           "Connected, but the network didn't give an address. Try again later."),
    "no_adapter": ("Er is geen Wi-Fi-adapter gevonden.", "No Wi-Fi adapter was found."),
    "unavailable": ("Netwerkbeheer is niet beschikbaar.", "Network management isn't available."),
}


def _wifi_text(key: str) -> str:
    nl, en = _WIFI_TEXT.get(key, _WIFI_TEXT["failed"])
    return en if _lang() == "en" else nl


def wifi_fail(code: str, why: str | None = None) -> dict:
    """{ok:false, code, error (friendly text), reason?}. ``code`` is one of the four UI codes."""
    out = {"ok": False, "code": code, "error": _wifi_text(why if why in _WIFI_TEXT else code)}
    if why:
        out["reason"] = why
    return out


def wifi_security(flags: int, wpa: int, rsn: int) -> str:
    """Access point flags → 'open' | 'owe' | 'wpa-psk' | 'sae' | 'enterprise' | 'wep'. Pure (tested)."""
    km = int(wpa or 0) | int(rsn or 0)
    if km & (_KM_8021X | _KM_EAP_B192):
        return "enterprise"
    if km & _KM_PSK:
        return "wpa-psk"          # also WPA2/WPA3 transition networks
    if km & _KM_SAE:
        return "sae"              # WPA3-only
    if km & (_KM_OWE | _KM_OWE_TM):
        return "owe"              # "Enhanced Open": encrypted, no password
    if int(flags or 0) & _AP_PRIVACY:
        return "wep"
    return "open"


def psk_valid(password: str) -> bool:
    """WPA passphrase: 8–63 printable ASCII characters, or exactly 64 hex digits."""
    if len(password) == 64:
        return all(c in "0123456789abcdefABCDEF" for c in password)
    return 8 <= len(password) <= 63 and all(32 <= ord(c) < 127 for c in password)


def wifi_settings(ssid: str, sec: str, password: str | None) -> dict:
    """NetworkManager connection settings for a new Wi-Fi profile (plain Python values). Pure (tested)."""
    s: dict = {
        "connection": {"id": ssid, "type": "802-11-wireless", "autoconnect": True},
        "802-11-wireless": {"ssid": ssid.encode("utf-8"), "mode": "infrastructure"},
        "ipv4": {"method": "auto"},
        "ipv6": {"method": "auto"},
    }
    if sec in ("wpa-psk", "sae"):
        s["802-11-wireless-security"] = {"key-mgmt": sec, "psk": password or ""}
    elif sec == "owe":
        s["802-11-wireless-security"] = {"key-mgmt": "owe"}
    return s


def wifi_settings_variant(ssid: str, sec: str, password: str | None):
    """``wifi_settings`` as a GLib.Variant of type a{sa{sv}}."""
    _Gio, GLib = _gi()

    def v(value):
        if isinstance(value, bool):
            return GLib.Variant("b", value)
        if isinstance(value, bytes):
            return GLib.Variant("ay", value)
        if isinstance(value, int):
            return GLib.Variant("u", value)
        return GLib.Variant("s", str(value))

    return GLib.Variant("a{sa{sv}}", {group: {k: v(val) for k, val in keys.items()}
                                      for group, keys in wifi_settings(ssid, sec, password).items()})


def wifi_reason_code(reason: int | None, had_password: bool, saved: bool) -> tuple[str, str | None]:
    """NMDeviceStateReason of a failed activation → (UI code, detail). Pure (tested)."""
    r = int(reason or 0)
    if r == 7:                                  # NO_SECRETS
        return ("wrong_password", None) if had_password else ("needs_password", None)
    if r in (8, 11):                            # SUPPLICANT_DISCONNECT / _TIMEOUT: 4-way handshake failed
        if had_password or saved:
            return "wrong_password", None
        return "failed", None
    if r == 53:                                 # SSID_NOT_FOUND
        return "not_found", None
    if r in (5, 17, 18):                        # IP_CONFIG_UNAVAILABLE, DHCP_ERROR, DHCP_FAILED
        return "failed", "ip"
    if r == -1:
        return "failed", "timeout"
    return "failed", None


def _nm_wifi_device(devices: list[dict]) -> dict | None:
    wifi = [d for d in devices if d["type"] == 2]
    if not wifi:
        return None
    return next((d for d in wifi if d["state"] >= 30), wifi[0])   # ≥ DISCONNECTED = usable


def _nm_find_ap(bus, dev_path: str, ssid: str) -> dict | None:
    """Strongest visible access point with this SSID → {path, strength, security} (read-only)."""
    try:
        aps = dbus_call(bus, _NM, dev_path, _NM + ".Device.Wireless", "GetAllAccessPoints", None)[0]
    except Exception:  # noqa: BLE001
        return None
    best = None
    for ap in aps:
        try:
            p = _getall(_NM, ap, _NM + ".AccessPoint")
        except Exception:  # noqa: BLE001
            continue
        if _ssid_bytes(p.get("Ssid")) != ssid:
            continue
        strength = int(p.get("Strength") or 0)
        if best is None or strength > best["strength"]:
            best = {"path": ap, "strength": strength,
                    "security": wifi_security(p.get("Flags") or 0, p.get("WpaFlags") or 0, p.get("RsnFlags") or 0)}
    return best


def _nm_request_scan(bus, dev_path: str) -> None:
    _Gio, GLib = _gi()
    try:
        dbus_call(bus, _NM, dev_path, _NM + ".Device.Wireless", "RequestScan",
                  GLib.Variant("(a{sv})", ({},)), timeout_ms=5000)
    except Exception as e:  # noqa: BLE001 - scanning too often is refused; the old list is fine
        log.debug("Wi-Fi-scan geweigerd (scan refused): %s", e)


def _nm_settings_variants(bus, conn_path: str) -> dict:
    """A saved profile's settings as {group: {key: GLib.Variant}} (types kept; read-only; no secrets)."""
    Gio, GLib = _gi()
    res = bus.call_sync(_NM, conn_path, _NM + ".Settings.Connection", "GetSettings", None,
                        GLib.VariantType.new("(a{sa{sv}})"), Gio.DBusCallFlags.NONE, 5000, None)
    settings = res.get_child_value(0)
    groups: dict = {}
    for i in range(settings.n_children()):
        entry = settings.get_child_value(i)
        inner = entry.get_child_value(1)
        keys = {}
        for j in range(inner.n_children()):
            kv = inner.get_child_value(j)
            keys[kv.get_child_value(0).get_string()] = kv.get_child_value(1).get_variant()
        groups[entry.get_child_value(0).get_string()] = keys
    return groups


def _nm_update_psk(bus, conn_path: str, sec: str, password: str) -> None:
    """Put a newly typed password into a saved profile (GetSettings never returns secrets)."""
    _Gio, GLib = _gi()
    groups = _nm_settings_variants(bus, conn_path)
    sec_group = groups.setdefault("802-11-wireless-security", {})
    sec_group.setdefault("key-mgmt", GLib.Variant("s", sec))
    sec_group["psk"] = GLib.Variant("s", password)
    dbus_call(bus, _NM, conn_path, _NM + ".Settings.Connection", "Update",
              GLib.Variant.new_tuple(GLib.Variant("a{sa{sv}}", groups)), timeout_ms=10000)


class _DeviceWatch:
    """Follow a device's StateChanged signals in a private GLib main context (works in any thread,
    independent of the host's main loop). Subscribed before activation starts, so nothing is missed."""

    def __init__(self, bus, dev_path: str):
        self.bus = bus
        self.dev_path = dev_path
        self.events: list[tuple[int, int, int]] = []
        self._sid = None
        self._mctx = None

    def __enter__(self):
        Gio, GLib = _gi()
        self._mctx = GLib.MainContext.new()
        self._mctx.push_thread_default()

        def on_state(_conn, _sender, _path, _iface, _signal, params, *_a):
            try:
                new, old, reason = params.unpack()
                self.events.append((int(new), int(old), int(reason)))
            except Exception:  # noqa: BLE001
                pass

        try:
            self._sid = self.bus.signal_subscribe(_NM, _NM + ".Device", "StateChanged", self.dev_path, None,
                                                  Gio.DBusSignalFlags.NONE, on_state)
        except BaseException:
            self._mctx.pop_thread_default()   # __exit__ does not run when __enter__ fails
            raise
        return self

    def __exit__(self, *exc):
        try:
            if self._sid is not None:
                self.bus.signal_unsubscribe(self._sid)
        finally:
            self._mctx.pop_thread_default()
        return False

    def wait(self, active_path: str, timeout: float = 45.0) -> tuple[bool, int | None]:
        """→ (connected, NMDeviceStateReason of the failure; -1 = timeout)."""
        deadline = time.monotonic() + timeout
        started = False
        last_poll = 0.0
        while time.monotonic() < deadline:
            while self._mctx.pending():
                self._mctx.iteration(False)
            while self.events:
                new, _old, reason = self.events.pop(0)
                if 40 <= new <= 90:          # PREPARE … SECONDARIES: our activation is running
                    started = True
                elif new == _NM_DEVICE_ACTIVATED and started:
                    return True, None
                elif new == _NM_DEVICE_FAILED:
                    return False, reason
                elif new == _NM_DEVICE_DISCONNECTED and started:
                    return False, reason
            now = time.monotonic()
            if now - last_poll > 1.0:        # safety net: the active connection's own state
                last_poll = now
                try:
                    st = int(_getall(_NM, active_path, _NM + ".Connection.Active").get("State") or 0)
                    if st == 2:
                        return True, None
                except Exception:  # noqa: BLE001 - the object disappears when activation failed
                    if started:
                        return False, None
            time.sleep(0.05)
        return False, -1


def _c_env() -> dict:
    env = dict(os.environ)
    env["LC_ALL"] = "C"
    env["LANG"] = "C"
    return env


def _rfkill_devices() -> list[dict]:
    out = []
    try:
        for d in Path("/sys/class/rfkill").iterdir():
            out.append({"type": _read(d / "type", ""), "soft": _read(d / "soft", "0") == "1",
                        "hard": _read(d / "hard", "0") == "1"})
    except OSError:
        pass
    return out


def _airplane_state() -> bool:
    devs = _rfkill_devices()
    return bool(devs) and all(d["soft"] or d["hard"] for d in devs)


def set_airplane(enabled: bool, bt: "Bluetooth") -> None:
    ok = False
    if which("nmcli"):
        ok = run(["nmcli", "radio", "all", "off" if enabled else "on"], timeout=8, env=_c_env()).ok or ok
    if which("rfkill") and _rfkill_devices():
        ok = run(["rfkill", "block" if enabled else "unblock", "all"], timeout=5).ok or ok
    elif enabled:
        try:
            bt.set_powered(False)
            ok = True
        except ApiError:
            pass
    if not ok:
        raise ApiError("failed", "Vliegtuigmodus kon niet worden aangepast.")


# ---------------------------------------------------------------------------
# Bluetooth (BlueZ D-Bus)
# ---------------------------------------------------------------------------

_BZ = "org.bluez"
_BZ_ADAPTER = "org.bluez.Adapter1"
_BZ_DEVICE = "org.bluez.Device1"
_BZ_AGENTS = "org.bluez.AgentManager1"
_DBUS_PROPS = "org.freedesktop.DBus.Properties"
_ADDR_RE = re.compile(r"^[0-9A-Fa-f]{2}(:[0-9A-Fa-f]{2}){5}$")
_MACLIKE_RE = re.compile(r"^[0-9A-Fa-f]{2}([:_-][0-9A-Fa-f]{2}){5}$")

BT_STUCK_S = 5.0            # a transitional PowerState for longer than this = stuck
BT_DISCOVERY_S = 60.0       # discover {on:true} stops by itself after this unless renewed
BT_NEARBY_KEEP_S = 60.0     # found devices stay listed this long after the search stopped
BT_PAIR_TOTAL_S = 60.0      # pair → trust → connect budget
BT_PAIR_CALL_S = 45.0       # Device1.Pair D-Bus timeout (then CancelPairing)
BT_CONNECT_MIN_S = 10.0     # Connect after a slow pairing still gets this long (45 + 10 + setup ≤ 60 s)
BT_AGENT_REPLY_S = 30.0     # a question to the user (confirm / passkey / PIN) is cancelled after this
BT_RESET_WAIT_S = 12.0      # reset waits this long for PowerState 'on'
BT_RFKILL_PAUSE_S = 1.0     # between rfkill block and unblock
BT_AUDIO_WAIT_S = 12.0      # after pair+connect: wait this long for the headset's PipeWire sink …
BT_AUDIO_GRACE_S = 1.5      # … and give WirePlumber this long to make it the default itself
BT_UNREGISTER_RETRY_S = (1.0, 3.0, 9.0)   # UnregisterAgent failed: try again after these pauses
BT_TAKEOVER_WAIT_S = 10.0   # pair of the address being paired (a new dialog): wait this long for the old one
BT_RESET_PAIR_WAIT_S = 8.0  # reset: wait this long for our cancelled pairing to end
_BT_TRANSITIONAL = ("on-disabling", "off-enabling")
_BT_AUDIO_KINDS = ("headphones", "headset", "speaker")
_AGENT_PATH = "/org/win11shell/bluetooth/agent"
_AGENT_CAPABILITY = "KeyboardDisplay"
_AGENT_XML = """<node><interface name="org.bluez.Agent1">
  <method name="Release"/>
  <method name="RequestPinCode"><arg type="o" direction="in"/><arg type="s" direction="out"/></method>
  <method name="DisplayPinCode"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
  <method name="RequestPasskey"><arg type="o" direction="in"/><arg type="u" direction="out"/></method>
  <method name="DisplayPasskey"><arg type="o" direction="in"/><arg type="u" direction="in"/>
    <arg type="q" direction="in"/></method>
  <method name="RequestConfirmation"><arg type="o" direction="in"/><arg type="u" direction="in"/></method>
  <method name="RequestAuthorization"><arg type="o" direction="in"/></method>
  <method name="AuthorizeService"><arg type="o" direction="in"/><arg type="s" direction="in"/></method>
  <method name="Cancel"/>
</interface></node>"""

_STR["nl"].update({
    "bt_unavailable": "Bluetooth is niet beschikbaar.",
    "bt_no_adapter": "Er is geen Bluetooth-adapter gevonden.",
    "bt_off": "Zet Bluetooth eerst aan.",
    "bt_not_responding": "Bluetooth reageert niet. Kies “Bluetooth opnieuw starten”.",
    "bt_busy": "Bluetooth is even bezig. Probeer het zo opnieuw.",
    "bt_power_failed": "Bluetooth aan- of uitzetten lukte niet.",
    "bt_hard_blocked": "Bluetooth is uitgezet met een schakelaar of toets op de computer.",
    "bt_reset_failed": "Bluetooth opnieuw starten lukte niet. Start de computer opnieuw op.",
    "bt_discover_failed": "Zoeken naar apparaten lukte niet.",
    "bt_auth_failed": "De koppelcode klopte niet. Probeer het opnieuw.",
    "bt_canceled": "Koppelen is geannuleerd.",
    "bt_rejected": "Het apparaat heeft het koppelen geweigerd.",
    "bt_timeout": "Het apparaat reageerde niet. Staat het nog in de koppelstand?",
    "bt_reply_timeout": "Er is niet op tijd bevestigd. Probeer het opnieuw.",
    "bt_gone": "Het apparaat is niet meer zichtbaar. Zet het opnieuw in de koppelstand.",
    "bt_busy_pairing": "Er wordt al een apparaat gekoppeld. Even geduld.",
    "bt_pair_failed": "Koppelen lukte niet. Probeer het opnieuw.",
    "bt_paired_not_connected": "Gekoppeld, maar verbinden lukte niet. Kies later “Verbinden”.",
    "bt_no_request": "Er is geen koppelverzoek (meer) voor dit apparaat.",
    "bt_bad_passkey": "Typ de zes cijfers die het apparaat laat zien.",
    "bt_bad_pin": "Typ een pincode van 1 tot 16 letters of cijfers.",
    "bt_in_progress": "Er wordt al verbinding gemaakt. Even geduld.",
    "bt_connect_failed": "Verbinden lukte niet. Staat het apparaat aan en in de buurt?",
    "bt_profile_unavailable": "Dit apparaat kan niet met deze computer verbinden.",
    "bt_disconnect_failed": "Verbinding verbreken lukte niet.",
    "bt_remove_failed": "Dit apparaat verwijderen lukte niet.",
    "bt_bad_device": "Ongeldig apparaat.",
    "bt_not_found": "Dit apparaat is niet gevonden.",
})
_STR["en"].update({
    "bt_unavailable": "Bluetooth is not available.",
    "bt_no_adapter": "No Bluetooth adapter was found.",
    "bt_off": "Turn on Bluetooth first.",
    "bt_not_responding": "Bluetooth is not responding. Choose “Restart Bluetooth”.",
    "bt_busy": "Bluetooth is busy. Try again in a moment.",
    "bt_power_failed": "Turning Bluetooth on or off failed.",
    "bt_hard_blocked": "Bluetooth was turned off with a switch or key on the computer.",
    "bt_reset_failed": "Restarting Bluetooth failed. Restart the computer.",
    "bt_discover_failed": "Searching for devices failed.",
    "bt_auth_failed": "The pairing code was wrong. Try again.",
    "bt_canceled": "Pairing was cancelled.",
    "bt_rejected": "The device refused to pair.",
    "bt_timeout": "The device did not respond. Is it still in pairing mode?",
    "bt_reply_timeout": "The pairing was not confirmed in time. Try again.",
    "bt_gone": "The device is no longer visible. Put it in pairing mode again.",
    "bt_busy_pairing": "Another device is being paired. Please wait.",
    "bt_pair_failed": "Pairing failed. Try again.",
    "bt_paired_not_connected": "Paired, but connecting failed. Choose “Connect” later.",
    "bt_no_request": "There is no (longer a) pairing request for this device.",
    "bt_bad_passkey": "Type the six digits the device shows.",
    "bt_bad_pin": "Type a PIN of 1 to 16 letters or digits.",
    "bt_in_progress": "Already connecting. Please wait.",
    "bt_connect_failed": "Connecting failed. Is the device on and nearby?",
    "bt_profile_unavailable": "This device cannot connect to this computer.",
    "bt_disconnect_failed": "Disconnecting failed.",
    "bt_remove_failed": "Removing this device failed.",
    "bt_bad_device": "Invalid device.",
    "bt_not_found": "This device was not found.",
})

# Device categories (Win11 shows an icon per kind; audio kinds become the default output after pairing).
_BT_ICON_KIND = {
    "audio-headphones": "headphones", "audio-headset": "headset", "audio-card": "speaker",
    "audio-speakers": "speaker", "phone": "phone", "computer": "computer", "input-keyboard": "keyboard",
    "input-mouse": "mouse", "input-gaming": "gamepad", "input-tablet": "pen", "video-display": "display",
    "camera-video": "camera", "camera-photo": "camera", "printer": "printer", "scanner": "printer",
    "multimedia-player": "media", "modem": "network", "network-wireless": "network",
}
_BT_AV_MINOR = {1: "headset", 2: "headset", 4: "headset", 5: "speaker", 6: "headphones", 7: "speaker",
                8: "speaker", 9: "media", 10: "speaker", 11: "media", 12: "camera", 13: "camera", 14: "display",
                15: "display", 16: "headset"}


def bt_kind(d: dict) -> str:
    """Win11 device category of a BlueZ Device1: Icon, else Class of Device (BR/EDR), Appearance (LE), UUIDs.
    One of headphones headset speaker phone computer keyboard mouse gamepad pen display camera printer
    watch network media other."""
    k = _BT_ICON_KIND.get(str(d.get("Icon") or ""))
    if k:
        return k
    cls = _int(d.get("Class"), 0) or 0
    major, minor = (cls >> 8) & 0x1F, (cls >> 2) & 0x3F
    if cls and major:
        if major == 1:
            return "computer"
        if major == 2:
            return "phone"
        if major == 3:
            return "network"
        if major == 4:
            return _BT_AV_MINOR.get(minor, "media")
        if major == 5:
            kb = (cls >> 6) & 0x3
            if kb == 2:
                return "mouse"
            if kb:
                return "keyboard"
            return {1: "gamepad", 2: "gamepad", 5: "pen"}.get(minor & 0xF, "other")
        if major == 6:
            return "display" if cls & 0x10 else "camera" if cls & 0x20 else "printer"
        if major == 7:
            return "watch"
    app = _int(d.get("Appearance"), 0) or 0
    if app:
        cat, sub = app >> 6, app & 0x3F
        if cat == 0x01:
            return "phone"
        if cat == 0x02:
            return "computer"
        if cat == 0x03:
            return "watch"
        if cat == 0x05:
            return "display"
        if cat == 0x0F:
            return {1: "keyboard", 2: "mouse", 3: "gamepad", 4: "gamepad", 5: "pen"}.get(sub, "other")
        if cat == 0x21:
            return "speaker"
        if cat == 0x25:
            return "headset" if sub == 2 else "headphones"
    uuids = [str(u).lower()[:8] for u in (d.get("UUIDs") or [])]
    if "00001108" in uuids or "0000111e" in uuids:
        return "headset"
    if "0000110b" in uuids:
        return "speaker"
    return "other"


def bt_power_state(aprops: dict, blocked: bool = False) -> str:
    """Adapter1.PowerState ('on' 'off' 'off-blocked' 'on-disabling' 'off-enabling'; BlueZ ≥ 5.68),
    derived from Powered for an older BlueZ."""
    ps = aprops.get("PowerState")
    if isinstance(ps, str) and ps:
        return ps
    if aprops.get("Powered"):
        return "on"
    return "off-blocked" if blocked else "off"


def _bt_err(e: Exception) -> str:
    """D-Bus error name + message (``org.bluez.Error.Failed br-connection-page-timeout``) for matching."""
    name = _dbus_error_name(e) or ""
    text = str(e)
    return f"{name} {text}".strip() if name not in text else text


# machine-readable ``code`` next to the Dutch ``error`` of pair results / pairing events (like network.connect)
_BT_CODES = {
    "bt_auth_failed": "auth_failed", "bt_canceled": "cancelled", "bt_rejected": "rejected", "bt_timeout": "timeout",
    "bt_reply_timeout": "timeout", "bt_not_responding": "not_ready", "bt_off": "off", "bt_busy": "busy",
    "bt_gone": "not_found", "bt_not_found": "not_found", "bt_busy_pairing": "busy", "bt_in_progress": "busy",
    "bt_profile_unavailable": "unsupported", "bt_paired_not_connected": "connect_failed",
    "bt_connect_failed": "connect_failed", "bt_unavailable": "unavailable", "bt_no_adapter": "unavailable",
}


def bt_error_key(err: str, op: str, powered: bool = True) -> str:
    """Message key for a BlueZ error (``err`` from ``_bt_err``); ``op`` is pair|connect|disconnect|remove|
    discover|power. NotReady while Powered=true means a stuck adapter (→ "Bluetooth opnieuw starten")."""
    e = err or ""
    low = e.lower()
    if "NotReady" in e:
        return "bt_not_responding" if powered else "bt_off"
    if "Error.Busy" in e:
        return "bt_busy"
    if "DoesNotExist" in e or "UnknownObject" in e:
        return "bt_gone" if op == "pair" else "bt_not_found"
    timeout = ("AuthenticationTimeout" in e or "Timeout" in e or "NoReply" in e or "timed out" in low
               or "ConnectionAttemptFailed" in e or "page-timeout" in low or "page timeout" in low
               or "host is down" in low or "temporarily unavailable" in low)
    if op == "pair":
        if "AuthenticationFailed" in e:
            return "bt_auth_failed"
        if "AuthenticationCanceled" in e or "Error.Canceled" in e:
            return "bt_canceled"
        if "AuthenticationRejected" in e or "Error.Rejected" in e:
            return "bt_rejected"
        if timeout:
            return "bt_timeout"
        if "InProgress" in e:
            return "bt_busy_pairing"
        return "bt_pair_failed"
    if op == "connect":
        if "InProgress" in e:
            return "bt_in_progress"
        if "profile-unavailable" in low or "NotAvailable" in e or "NotSupported" in e:
            return "bt_profile_unavailable"
        return "bt_connect_failed"
    return {"disconnect": "bt_disconnect_failed", "remove": "bt_remove_failed",
            "discover": "bt_discover_failed"}.get(op, "bt_power_failed")


def bt_error_text(err: str, op: str, powered: bool = True) -> str:
    """Friendly Dutch (or English) text for a BlueZ error; see ``bt_error_key``."""
    return _tr(bt_error_key(err, op, powered))


def bt_error_code(key: str) -> str:
    return _BT_CODES.get(key, "failed")


def _bt_name(d: dict) -> str | None:
    """A name a person can recognise (never a bare MAC address, which BlueZ uses as Alias without a Name)."""
    for key in ("Alias", "Name"):
        v = str(d.get(key) or "").strip()
        if v and not _MACLIKE_RE.match(v):
            return v
    return None


def _bt_call_timed_out(err: str) -> bool:
    """Our D-Bus call gave up (GLib timeout / NoReply): bluetoothd may still carry it out."""
    return "Timeout was reached" in err or "NoReply" in err or "timed out" in err.lower()


def _rfkill_write(block: bool) -> bool:
    """Soft-(un)block every Bluetooth radio through /dev/rfkill (fallback without the rfkill tool; the
    seat user has an ACL on it). struct rfkill_event {u32 idx; u8 type; u8 op; u8 soft; u8 hard}."""
    import struct
    try:
        fd = os.open("/dev/rfkill", os.O_WRONLY | os.O_CLOEXEC)
    except OSError as e:
        log.info("/dev/rfkill niet te openen (cannot open): %s", e)
        return False
    try:
        os.write(fd, struct.pack("=IBBBB", 0, 2, 3, 1 if block else 0, 0))   # TYPE_BLUETOOTH, OP_CHANGE_ALL
        return True
    except OSError as e:
        log.info("/dev/rfkill schrijven mislukt (write failed): %s", e)
        return False
    finally:
        os.close(fd)


def _rfkill_bt(block: bool) -> bool:
    if which("rfkill"):
        r = run(["rfkill", "block" if block else "unblock", "bluetooth"], timeout=5)
        if r.ok:
            return True
        log.info("rfkill %s bluetooth mislukt (failed): %s", "block" if block else "unblock", r.err.strip())
    return _rfkill_write(block)


def _bt_sink_for(audio: "Audio", address: str, cache: dict | None = None) -> tuple[int, bool] | None:
    """(sink id, is default) of the PipeWire sink of Bluetooth device ``address``, or None (not there yet).
    ``cache`` (sink id → is it this device) saves a ``wpctl inspect`` per sink and round."""
    want = address.upper()
    key = want.replace(":", "_")
    cache = {} if cache is None else cache
    for s in audio.sinks():
        hit = cache.get(s["id"])
        if hit is None:
            ins = run(["wpctl", "inspect", str(s["id"])], timeout=2)
            if not ins.ok:
                continue
            a = (_prop(ins.out, "api.bluez5.address") or "").upper()
            n = (_prop(ins.out, "node.name") or "").upper()
            hit = cache[s["id"]] = bool(a == want or key in n or want in n)
        if hit:
            return s["id"], bool(s.get("default"))
    return None


class _PairAgent:
    """A temporary ``org.bluez.Agent1`` for ONE pairing that we start ourselves.

    BlueZ asks the agent of the connection that called ``Device1.Pair`` (and the default agent) for codes and
    confirmations. We register just before our Pair (``RegisterAgent`` + ``RequestDefaultAgent``) and
    unregister right after, so another agent (bluetoothctl, blueman …) is the default again afterwards.
    Requests about any other device, or from anyone but bluetoothd, are rejected. A question for the user
    (RequestConfirmation / RequestAuthorization / RequestPasskey / RequestPinCode) is not answered on the
    spot: the D-Bus reply is held until ``reply`` (system.bluetooth.pairReply) or ``cancel`` or the
    ``timeout``, so nothing ever blocks. Every held question has an id (``qid`` in its pairing event); an answer
    for another question (one that was replaced meanwhile) is refused, so a "yes" never confirms a code the
    user did not see. The object lives on its own thread + GLib main context, independent of the shell's main
    loop. When UnregisterAgent fails, the object stays exported refusing everything and the unregister is
    retried, so a dead path never stays BlueZ's default agent."""

    _seq = 0
    _live_path: str | None = None                 # our registration BlueZ may still hold (UnregisterAgent failed)
    _lingering: "_PairAgent | None" = None        # stopped, but BlueZ did not take the unregister yet

    def __init__(self, bus, address: str, dev_path: str, emit: Callable[..., None], timeout: float | None = None):
        self.bus = bus
        self.address = address.upper()
        self.dev_path = dev_path
        self._emit = emit                         # emit(stage, **extra)
        self.timeout = BT_AGENT_REPLY_S if timeout is None else timeout
        _PairAgent._seq += 1
        self.path = f"{_AGENT_PATH}{_PairAgent._seq}"
        self.owner: str | None = None             # unique bus name of bluetoothd
        self.declined = False                     # the user said no / cancelled
        self.timed_out = False                    # a question was not answered in time
        self.released = False
        self._lock = threading.Lock()
        self._pending: dict | None = None         # {kind, inv, timer, token, passkey}
        self._token = 0
        self._reg_id = None
        self._registered = False
        self._thread: threading.Thread | None = None
        self._ctx = None
        self._stopping = False                    # the GLib loop ends
        self._closing = False                     # stop() ran: every request is refused
        self._retry: threading.Timer | None = None

    # -- requests (agent thread) ---------------------------------------------------------------------
    def handle(self, sender: str, method: str, params: Any, inv) -> None:
        args = params.unpack() if hasattr(params, "unpack") else tuple(params or ())
        if not self.owner or sender != self.owner:
            log.info("Bluetooth-agent: verzoek %s van %s geweigerd (not bluetoothd)", method, sender)
            return self._reject(inv)
        if method == "Release":
            self.released = True
            self._drop_pending()
            inv.return_value(None)
            if self._closing:                     # a lingering agent: BlueZ let go of it, now it may go
                threading.Thread(target=self._finish, name="bt-agent-end", daemon=True).start()
            return None
        if method == "Cancel":
            if self._drop_pending() and not self._closing:
                self._emit("pairing")
            return inv.return_value(None)
        if self.declined or self._closing:        # the user cancelled / our pairing is over: no new questions
            return self._reject(inv)
        dev = str(args[0]) if args else ""
        if dev != self.dev_path:
            log.info("Bluetooth-agent: %s voor ander apparaat %s geweigerd (other device)", method, dev)
            return self._reject(inv)
        if method == "RequestConfirmation":
            return self._hold("confirm", inv, passkey=f"{int(args[1]) % 1000000:06d}")
        if method == "RequestAuthorization":
            return self._hold("authorize", inv)
        if method == "RequestPasskey":
            return self._hold("passkey", inv)
        if method == "RequestPinCode":
            return self._hold("pin", inv)
        if method == "DisplayPasskey":
            inv.return_value(None)
            return self._emit("display", passkey=f"{int(args[1]) % 1000000:06d}", entered=int(args[2]))
        if method == "DisplayPinCode":
            inv.return_value(None)
            return self._emit("display", pin=str(args[1]))
        if method == "AuthorizeService":
            return inv.return_value(None)          # the device the user is adding may use its services
        return self._reject(inv)

    @staticmethod
    def _reject(inv) -> None:
        inv.return_dbus_error("org.bluez.Error.Rejected", "Rejected")

    @staticmethod
    def _canceled(inv) -> None:
        try:
            inv.return_dbus_error("org.bluez.Error.Canceled", "Canceled")
        except Exception:  # noqa: BLE001
            pass

    def _hold(self, kind: str, inv, **extra) -> None:
        with self._lock:
            old = self._pending
            self._token += 1
            tok = self._token
            timer = threading.Timer(self.timeout, self._expire, args=(tok,))
            timer.daemon = True
            self._pending = {"kind": kind, "inv": inv, "timer": timer, "token": tok, "passkey": extra.get("passkey")}
        if old:
            old["timer"].cancel()
            self._canceled(old["inv"])
        timer.start()
        self._emit("confirm" if kind == "authorize" else kind, qid=tok, **extra)

    def _expire(self, tok: int) -> None:
        with self._lock:
            p = self._pending
            if not p or p["token"] != tok:
                return
            self._pending = None
            self.timed_out = True
        log.info("Bluetooth-agent: geen antwoord op %s binnen %.0f s (no reply in time)", p["kind"], self.timeout)
        self._canceled(p["inv"])

    def _drop_pending(self) -> bool:
        with self._lock:
            p, self._pending = self._pending, None
        if p:
            p["timer"].cancel()
            self._canceled(p["inv"])
        return p is not None

    # -- answers (API threads) -----------------------------------------------------------------------
    def pending_kind(self) -> str | None:
        with self._lock:
            return self._pending["kind"] if self._pending else None

    def reply(self, accept: bool, passkey: Any = None, pin: Any = None, qid: Any = None) -> dict:
        """Answer the held question. ``qid`` (from its pairing event) must be the question that is open now;
        a yes that does not fit it (another question, a passkey for a code to compare, a code that is not the
        one shown) is refused with code ``stale`` and the open question stays open. A no ends any question."""
        _, GLib = _gi()
        given = lambda v: v is not None and str(v).strip() != ""   # noqa: E731
        stale = {"ok": False, "error": _tr("bt_no_request"), "code": "stale"}
        with self._lock:
            p = self._pending
            if p is None:
                return {"ok": False, "error": _tr("bt_no_request")}
            kind = p["kind"]
            value = None
            if accept:
                if qid is not None and str(qid).strip() != str(p["token"]):
                    return stale
                if kind == "passkey":
                    if not given(passkey) and given(pin):
                        return stale
                    s = str(passkey if passkey is not None else "").strip()
                    if not s.isdigit() or len(s) > 6:
                        raise ApiError("bad_args", _tr("bt_bad_passkey"))
                    value = GLib.Variant("(u)", (int(s),))
                elif kind == "pin":
                    if not given(pin) and given(passkey):
                        return stale
                    s = str(pin if pin is not None else "").strip()
                    if not (1 <= len(s) <= 16) or not s.isascii() or not s.isalnum():
                        raise ApiError("bad_args", _tr("bt_bad_pin"))
                    value = GLib.Variant("(s)", (s,))
                else:                             # confirm / authorize: only a yes; a code must be the one shown
                    if given(pin):
                        return stale
                    if given(passkey):
                        s = str(passkey).strip()
                        if not (kind == "confirm" and p.get("passkey") and s.isdigit() and len(s) <= 6
                                and f"{int(s):06d}" == p["passkey"]):
                            return stale
            self._pending = None
        p["timer"].cancel()
        if not accept:
            self.declined = True
            self._reject(p["inv"])
            return {"ok": True}
        p["inv"].return_value(value)
        self._emit("pairing")
        return {"ok": True}

    def cancel(self) -> None:
        self.declined = True
        self._drop_pending()

    # -- D-Bus plumbing --------------------------------------------------------------------------------
    def start(self) -> None:
        """Export the object, then RegisterAgent + RequestDefaultAgent. Raises on failure (nothing left)."""
        Gio, GLib = _gi()
        try:
            r = dbus_call(self.bus, "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                          "GetNameOwner", GLib.Variant("(s)", (_BZ,)), timeout_ms=2000)
            self.owner = str(r[0]) if r else None
        except Exception as e:  # noqa: BLE001
            log.info("Bluetooth-agent: eigenaar van org.bluez onbekend (owner unknown): %s", e)
        info = Gio.DBusNodeInfo.new_for_xml(_AGENT_XML).interfaces[0]
        ready = threading.Event()
        box: dict[str, Any] = {}

        def on_call(_conn, sender, _path, _iface, method, params, inv):
            try:
                self.handle(sender, method, params, inv)
            except Exception:  # noqa: BLE001
                log.exception("Bluetooth-agent: %s faalde (failed)", method)
                self._reject(inv)

        def loop():
            ctx = GLib.MainContext.new()
            ctx.push_thread_default()
            try:
                reg = getattr(self.bus, "register_object_with_closures2", None) or self.bus.register_object
                box["id"] = reg(self.path, info, on_call, None, None)
                self._ctx = ctx
            except Exception as e:  # noqa: BLE001
                box["error"] = e
            finally:
                ready.set()
            try:
                while "id" in box and not self._stopping:
                    ctx.iteration(True)
            finally:
                ctx.pop_thread_default()

        self._thread = threading.Thread(target=loop, name="bt-agent", daemon=True)
        self._thread.start()
        ready.wait(5)
        if "id" not in box:
            raise RuntimeError(f"agent object: {box.get('error')}")
        self._reg_id = box["id"]
        try:
            self._register()
        except Exception:
            self.stop()
            raise
        try:
            dbus_call(self.bus, _BZ, "/org/bluez", _BZ_AGENTS, "RequestDefaultAgent",
                      GLib.Variant("(o)", (self.path,)), timeout_ms=3000)
        except Exception as e:  # noqa: BLE001
            log.info("Bluetooth-agent: RequestDefaultAgent mislukt (failed): %s", _bt_err(e))

    def _register(self) -> None:
        _, GLib = _gi()
        params = GLib.Variant("(os)", (self.path, _AGENT_CAPABILITY))
        try:
            self._register_call(params)
        except Exception as e:  # noqa: BLE001
            if "AlreadyExists" not in _bt_err(e):
                raise
            # BlueZ keeps one agent per D-Bus connection: an older one of ours was not unregistered. It can only
            # be removed by its own path.
            self._clear_stale()
            self._register_call(params)
        self._registered = True
        _PairAgent._live_path = self.path

    def _register_call(self, params) -> None:
        try:
            dbus_call(self.bus, _BZ, "/org/bluez", _BZ_AGENTS, "RegisterAgent", params, timeout_ms=3000)
        except Exception as e:  # noqa: BLE001
            if _bt_call_timed_out(_bt_err(e)):
                # our call gave up, but a slow bluetoothd may still register it: take it back (best effort);
                # if that fails too, remember the path so the next RegisterAgent (AlreadyExists) can clear it
                if self._unregister(self.path) == "failed":
                    _PairAgent._live_path = self.path
            raise

    def _unregister(self, path: str) -> str:
        """UnregisterAgent(path) → 'ok' (removed), 'absent' (BlueZ does not hold it) or 'failed'."""
        _, GLib = _gi()
        try:
            dbus_call(self.bus, _BZ, "/org/bluez", _BZ_AGENTS, "UnregisterAgent", GLib.Variant("(o)", (path,)),
                      timeout_ms=3000)
            return "ok"
        except Exception as e:  # noqa: BLE001
            err = _bt_err(e)
            if "DoesNotExist" in err or "ServiceUnknown" in err or "NameHasNoOwner" in err:
                return "absent"                   # not ours (any more), or bluetoothd is gone and with it all
            log.info("Bluetooth-agent: UnregisterAgent %s mislukt (failed): %s", path, err)
            return "failed"

    def _clear_stale(self) -> None:
        """RegisterAgent said AlreadyExists: unregister our old registration. Its path is known when an
        UnregisterAgent failed; after a RegisterAgent that timed out but went through it may not be, then the
        earlier paths of this process are tried (BlueZ answers DoesNotExist for the others)."""
        cands = [_PairAgent._live_path] if _PairAgent._live_path else []
        cands += [f"{_AGENT_PATH}{i}" for i in range(_PairAgent._seq - 1, max(0, _PairAgent._seq - 33), -1)]
        gone = None
        for path in dict.fromkeys(c for c in cands if c and c != self.path):
            if self._unregister(path) == "ok":
                gone = path
                break
        log.info("Bluetooth-agent: oude registratie %s vervangen (stale agent replaced)", gone)
        if gone is None:
            return
        if _PairAgent._live_path == gone:
            _PairAgent._live_path = None
        old = _PairAgent._lingering
        if old is not None and old.path == gone:
            old._registered = False
            old._finish()

    def stop(self) -> None:
        """Answer what is still open (Canceled), UnregisterAgent, unexport, end the thread. Idempotent.
        When UnregisterAgent fails the object stays exported (it refuses every request) and the unregister is
        retried in the background (BT_UNREGISTER_RETRY_S); it goes once BlueZ took it or sent Release."""
        self._closing = True
        self._drop_pending()
        if self._registered and not self.released:
            if self._retry is not None:
                return                            # already lingering: the retries run
            if self._unregister(self.path) == "failed":
                return self._linger(0)
        self._finish()

    def _linger(self, attempt: int) -> None:
        _PairAgent._live_path = self.path
        _PairAgent._lingering = self
        if attempt >= len(BT_UNREGISTER_RETRY_S):
            log.warning("Bluetooth-agent %s: UnregisterAgent blijft mislukken (keeps failing); de agent weigert "
                        "alles tot de volgende koppeling hem opruimt", self.path)
            return
        t = threading.Timer(BT_UNREGISTER_RETRY_S[attempt], self._retry_unregister, args=(attempt,))
        t.daemon = True
        self._retry = t
        t.start()

    def _retry_unregister(self, attempt: int) -> None:
        if not self._registered or self.released or self._reg_id is None:
            return self._finish()
        if self._unregister(self.path) == "failed":
            return self._linger(attempt + 1)
        log.info("Bluetooth-agent %s: alsnog afgemeld (unregistered on retry %d)", self.path, attempt + 1)
        self._finish()

    def _finish(self) -> None:
        """BlueZ no longer holds our registration (or never did): unexport and end the thread."""
        if self._retry is not None:
            self._retry.cancel()
        if (self._registered or self.released) and _PairAgent._live_path == self.path:
            _PairAgent._live_path = None
        if _PairAgent._lingering is self:
            _PairAgent._lingering = None
        self._registered = False
        if self._reg_id is not None:
            try:
                self.bus.unregister_object(self._reg_id)
            except Exception:  # noqa: BLE001
                log.debug("unregister_object failed", exc_info=True)
            self._reg_id = None
        self._stopping = True
        if self._ctx is not None:
            self._ctx.wakeup()
        if self._thread is not None and self._thread is not threading.current_thread():
            self._thread.join(2)


class Bluetooth:
    """BlueZ over D-Bus: state (+ nearby devices while searching), power, search, pair (with our own
    temporary agent), connect, remove and reset. ``emit``/``notify``/``on_audio`` are set by ``register``."""

    agent_cls = _PairAgent

    def __init__(self, audio: Audio | None = None):
        self.audio = audio
        self.emit: Callable[[str, Any], Any] = lambda *_a: None        # api.emit
        self.notify: Callable[[], None] = lambda: None                  # state may have changed: poll now
        self.on_audio: Callable[[dict], None] = lambda _res: None       # audio default changed
        self._lock = threading.RLock()
        self._pair_lock = threading.Lock()
        self._pairing: dict | None = None         # {address, path, agent, cancelled, stage, new_bond}
        self._reset_job: dict | None = None       # the running reset {done: Event, error} (one at a time)
        self._trans: tuple[str, float] | None = None   # (transitional PowerState, first seen)
        self._last_state: str | None = None
        self._notready_at: float | None = None    # a call answered NotReady while Powered
        self._disc_wanted = False                 # the UI searches (until _disc_deadline)
        self._disc_active = False                 # our StartDiscovery session runs
        self._disc_adapter: str | None = None
        self._disc_deadline = 0.0
        self._disc_timer: threading.Timer | None = None
        self._nearby_until = 0.0
        self._seen: dict[str, float] = {}         # address → last seen with an RSSI while searching
        self._audio_thread: threading.Thread | None = None

    # -- reading ---------------------------------------------------------------------------------------
    @staticmethod
    def _objects() -> dict:
        return dbus_call(_sysbus(), _BZ, "/", "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", None,
                         timeout_ms=2000)[0]

    def _adapter(self, objs: dict) -> tuple[str, dict] | None:
        ads = sorted((p, ifs[_BZ_ADAPTER]) for p, ifs in objs.items() if _BZ_ADAPTER in ifs)
        return ads[0] if ads else None

    @staticmethod
    def _rfkill_state() -> tuple[bool, bool]:
        """(soft or hard blocked, hard blocked) of the Bluetooth radios."""
        rf = [r for r in _rfkill_devices() if r["type"] == "bluetooth"]
        return any(r["soft"] or r["hard"] for r in rf), any(r["hard"] for r in rf)

    @staticmethod
    def _unavailable(blocked: bool = False) -> dict:
        return {"available": False, "powered": False, "powerState": "off", "stuck": False, "discovering": False,
                "name": "", "blocked": blocked, "devices": [], "nearby": []}

    def _track(self, state: str, powered: bool, now: float) -> bool:
        """Update the stuck bookkeeping with an observed PowerState; → stuck."""
        with self._lock:
            if state != self._last_state:
                if self._last_state is not None:
                    self._notready_at = None      # the adapter moved: an old NotReady says nothing now
                self._last_state = state
            if state in _BT_TRANSITIONAL:
                if self._trans is None or self._trans[0] != state:
                    self._trans = (state, now)
                trans_stuck = now - self._trans[1] > BT_STUCK_S
            else:
                self._trans = None
                trans_stuck = False
            return trans_stuck or (powered and self._notready_at is not None)

    def _note_error(self, err: str, powered: bool) -> None:
        """A call answered NotReady while the adapter says Powered=true: it is stuck (→ ``stuck``)."""
        if powered and "NotReady" in err:
            with self._lock:
                self._notready_at = time.monotonic()
            self.notify()

    def _note_ok(self) -> None:
        with self._lock:
            had = self._notready_at is not None
            self._notready_at = None
        if had:
            self.notify()

    def _stuck_now(self) -> bool:
        """Stuck as last observed (no D-Bus call)."""
        with self._lock:
            trans = self._trans is not None and time.monotonic() - self._trans[1] > BT_STUCK_S
            return trans or self._notready_at is not None

    def get(self) -> dict:
        blocked, _hard = self._rfkill_state()
        try:
            objs = self._objects()
        except Exception:  # noqa: BLE001
            return self._unavailable(blocked)
        ad = self._adapter(objs)
        if not ad:
            return self._unavailable(blocked)
        apath, aprops = ad
        now = time.monotonic()
        powered = bool(aprops.get("Powered"))
        state = bt_power_state(aprops, blocked)
        stuck = self._track(state, powered, now)
        discovering = bool(aprops.get("Discovering"))
        with self._lock:
            if discovering or self._disc_wanted:
                self._nearby_until = max(self._nearby_until, now + BT_NEARBY_KEEP_S)
            show_nearby = powered and (discovering or self._disc_wanted or now < self._nearby_until)
            if not show_nearby:
                self._seen.clear()
        devices, nearby = [], []
        for p, ifs in objs.items():
            d = ifs.get(_BZ_DEVICE)
            if not d or not p.startswith(apath + "/"):
                continue
            addr = str(d.get("Address") or "")
            kind = bt_kind(d)
            if d.get("Paired") or d.get("Connected") or d.get("Bonded"):
                item = {"address": addr, "name": _bt_name(d) or addr, "icon": d.get("Icon") or "bluetooth",
                        "paired": bool(d.get("Paired") or d.get("Bonded")), "connected": bool(d.get("Connected")),
                        "kind": kind}
                bat = ifs.get("org.bluez.Battery1")
                if bat and bat.get("Percentage") is not None:
                    item["battery"] = int(bat["Percentage"])
                devices.append(item)
                continue
            if not show_nearby:
                continue
            name = _bt_name(d)
            if not name:
                continue                          # nameless MACs (LE beacons …) mean nothing to a person
            rssi = d.get("RSSI")
            rssi = int(rssi) if isinstance(rssi, (int, float)) and not isinstance(rssi, bool) else None
            with self._lock:
                if rssi is not None:
                    self._seen[addr.upper()] = now
                elif addr.upper() not in self._seen:
                    continue                      # cached by BlueZ, not seen during this search
            nearby.append({"address": addr, "name": name, "icon": d.get("Icon") or "bluetooth", "kind": kind,
                           "rssi": rssi})
        devices.sort(key=lambda x: (not x["connected"], str(x["name"]).lower()))
        nearby.sort(key=lambda x: (x["rssi"] is None, -(x["rssi"] or 0), x["name"].lower()))
        return {"available": True, "powered": powered, "powerState": state, "stuck": stuck,
                "discovering": discovering, "name": aprops.get("Alias") or aprops.get("Name") or "",
                "blocked": blocked, "devices": devices, "nearby": nearby}

    def _adapter_or_raise(self) -> tuple[str, dict]:
        try:
            objs = self._objects()
        except Exception:  # noqa: BLE001
            raise ApiError("unavailable", _tr("bt_unavailable")) from None
        ad = self._adapter(objs)
        if not ad:
            raise ApiError("unavailable", _tr("bt_no_adapter"))
        return ad

    def _powered_now(self) -> bool:
        try:
            ad = self._adapter(self._objects())
            return bool(ad and ad[1].get("Powered"))
        except Exception:  # noqa: BLE001
            return True

    @staticmethod
    def _check_address(address: Any) -> str:
        if not isinstance(address, str) or not _ADDR_RE.match(address):
            raise ApiError("bad_args", _tr("bt_bad_device"))
        return address.upper()

    @staticmethod
    def _find_device(objs: dict, address: str) -> str | None:
        for p, ifs in sorted(objs.items()):
            d = ifs.get(_BZ_DEVICE)
            if d and str(d.get("Address", "")).upper() == address.upper():
                return p
        return None

    def _device_path(self, address: Any) -> str:
        address = self._check_address(address)
        try:
            objs = self._objects()
        except Exception:  # noqa: BLE001
            raise ApiError("unavailable", _tr("bt_unavailable")) from None
        p = self._find_device(objs, address)
        if p is None:
            raise ApiError("not_found", _tr("bt_not_found"))
        return p

    def _device_props(self, path: str) -> dict:
        try:
            return dict(self._objects().get(path, {}).get(_BZ_DEVICE) or {})
        except Exception:  # noqa: BLE001
            return {}

    def _set_prop(self, path: str, iface: str, prop: str, value, timeout_ms: int = 5000) -> None:
        _, GLib = _gi()
        dbus_call(_sysbus(), _BZ, path, _DBUS_PROPS, "Set", GLib.Variant("(ssv)", (iface, prop, value)),
                  timeout_ms=timeout_ms)

    # -- power -----------------------------------------------------------------------------------------
    def set_powered(self, powered: bool) -> dict:
        _, GLib = _gi()
        apath, aprops = self._adapter_or_raise()
        if not powered:
            self._stop_discovery()
        for attempt in range(2):
            try:
                self._set_prop(apath, _BZ_ADAPTER, "Powered", GLib.Variant("b", bool(powered)))
                break
            except Exception as e:  # noqa: BLE001
                err = _bt_err(e)
                if attempt == 0 and powered and "Blocked" in err and not self._rfkill_state()[1]:
                    _rfkill_bt(False)             # soft-blocked (rfkill / airplane mode): unblock, try again
                    time.sleep(0.4)
                    continue
                log.info("Bluetooth Powered=%s mislukt (failed): %s", powered, err)
                was = bool(aprops.get("Powered"))
                self._note_error(err, was)
                if was and ("NotReady" in err or self._stuck_now()):
                    raise ApiError("not_ready", _tr("bt_not_responding")) from None
                if "Blocked" in err:
                    raise ApiError("blocked", _tr("bt_hard_blocked")) from None
                if "Error.Busy" in err or "InProgress" in err:
                    raise ApiError("busy", _tr("bt_busy")) from None
                raise ApiError("failed", _tr("bt_power_failed")) from None
        self._note_ok()
        time.sleep(0.2)
        return self.get()

    def _wait_power(self, done: Callable[[str, dict], bool], timeout: float, power_on: bool = False) -> bool:
        """Poll the adapter until ``done(powerState, props)``; with ``power_on`` (re)ask Powered=true while it
        is plainly 'off' (after an rfkill unblock BlueZ may stay off or answer NotReady for a moment)."""
        _, GLib = _gi()
        deadline = time.monotonic() + timeout
        asked = None
        while True:
            try:
                ad = self._adapter(self._objects())
            except Exception:  # noqa: BLE001
                ad = None
            if ad:
                blocked, _hard = self._rfkill_state()
                state = bt_power_state(ad[1], blocked)
                if done(state, ad[1]):
                    return True
                now = time.monotonic()
                if power_on and state == "off" and (asked is None or now - asked > 2.5):
                    asked = now
                    try:
                        self._set_prop(ad[0], _BZ_ADAPTER, "Powered", GLib.Variant("b", True))
                    except Exception as e:  # noqa: BLE001
                        log.info("Bluetooth aanzetten na herstart (power on after reset): %s", _bt_err(e))
            if time.monotonic() >= deadline:
                return False
            time.sleep(0.25)

    def reset(self) -> dict:
        """"Bluetooth opnieuw starten": stuck / NotReady / no adapter → rfkill block + unblock (no root: the seat
        user has an ACL on /dev/rfkill), else Powered off → on; then wait until PowerState is 'on'.
        One at a time: a second caller (quick settings and Instellingen are separate processes) waits for the
        running reset and gets its outcome. Our own pairing is cancelled first (it could not survive anyway)."""
        with self._lock:
            job = self._reset_job
            mine = job is None
            if mine:
                job = self._reset_job = {"done": threading.Event(), "error": None}
        if not mine:
            log.info("Bluetooth opnieuw starten loopt al (reset already running): wachten (waiting)")
            if not job["done"].wait(BT_RESET_WAIT_S + BT_RESET_PAIR_WAIT_S + 15):
                raise ApiError("busy", _tr("bt_busy"))
            e = job["error"]
            if e is not None:
                if isinstance(e, ApiError):
                    raise ApiError(e.code, e.message)
                raise ApiError("failed", _tr("bt_reset_failed"))
            return self.get()
        try:
            return self._reset()
        except Exception as e:
            job["error"] = e
            raise
        finally:
            with self._lock:
                self._reset_job = None
            job["done"].set()

    def _reset(self) -> dict:
        radios = [r for r in _rfkill_devices() if r["type"] == "bluetooth"]
        if any(r["hard"] for r in radios):
            raise ApiError("blocked", _tr("bt_hard_blocked"))
        with self._lock:
            p = self._pairing
        if p:
            log.info("Bluetooth opnieuw starten: koppelen van %s eerst gestopt (pairing cancelled first)", p["address"])
            self.cancel_pair(p["address"])
        held = self._pair_lock.acquire(timeout=BT_RESET_PAIR_WAIT_S if p else 2.0)   # no new pairing meanwhile
        try:
            return self._reset_adapter(radios)
        finally:
            if held:
                self._pair_lock.release()

    def _reset_adapter(self, radios: list) -> dict:
        _, GLib = _gi()
        st = self.get()
        if not st["available"]:
            try:
                self._objects()
            except Exception:  # noqa: BLE001
                raise ApiError("unavailable", _tr("bt_unavailable")) from None   # bluetoothd itself is gone
            if not radios:
                raise ApiError("unavailable", _tr("bt_no_adapter"))
        self._stop_discovery()
        state = st["powerState"] if st["available"] else "missing"
        rfkill = not st["available"] or st["stuck"] or state in _BT_TRANSITIONAL or state == "off-blocked"
        log.info("Bluetooth opnieuw starten (reset): powerState=%s stuck=%s → %s", state, st["stuck"],
                 "rfkill" if rfkill else "Powered off/on")
        if not rfkill and st["powered"]:
            apath, _ = self._adapter_or_raise()
            try:
                self._set_prop(apath, _BZ_ADAPTER, "Powered", GLib.Variant("b", False))
            except Exception as e:  # noqa: BLE001
                log.info("Bluetooth uitzetten mislukt, dan rfkill (power off failed, using rfkill): %s", _bt_err(e))
                rfkill = True
            else:
                if not self._wait_power(lambda s, _p: s in ("off", "off-blocked"), 5.0):
                    rfkill = True
        if rfkill:
            if not _rfkill_bt(True):
                raise ApiError("failed", _tr("bt_reset_failed"))
            time.sleep(BT_RFKILL_PAUSE_S)
            if not _rfkill_bt(False):
                time.sleep(0.5)
                if not _rfkill_bt(False):
                    raise ApiError("failed", _tr("bt_reset_failed"))
        with self._lock:
            self._notready_at = None
            self._trans = None
        if not self._wait_power(lambda s, p: s == "on" and bool(p.get("Powered")), BT_RESET_WAIT_S, power_on=True):
            log.info("Bluetooth opnieuw starten: niet 'on' binnen %.0f s (not on in time)", BT_RESET_WAIT_S)
            self.notify()
            raise ApiError("failed", _tr("bt_reset_failed"))
        time.sleep(0.2)
        return self.get()

    # -- searching -------------------------------------------------------------------------------------
    def discover(self, on: bool) -> dict:
        """on=True: start (or renew) our search; it stops by itself BT_DISCOVERY_S after the last renewal.
        While we pair, the search is paused (inquiry disturbs paging) and resumes afterwards."""
        if not on:
            self._stop_discovery()
            self.notify()
            return {"ok": True}
        apath, aprops = self._adapter_or_raise()
        if not aprops.get("Powered"):
            return {"ok": False, "error": _tr("bt_off"), "code": "off"}
        with self._lock:
            self._disc_wanted = True
            self._disc_deadline = time.monotonic() + BT_DISCOVERY_S
            self._arm_discovery_timer()
            if self._disc_active and (self._disc_adapter != apath or not aprops.get("Discovering")):
                self._disc_active = False         # BlueZ ended our session (power cycle, reset, adapter gone)
            start = not self._disc_active and self._pairing is None
        if start:
            err = self._start_session(apath, aprops)
            if err:
                with self._lock:
                    self._disc_wanted = False
                    if self._disc_timer:
                        self._disc_timer.cancel()
                        self._disc_timer = None
                powered = bool(aprops.get("Powered"))
                self._note_error(err, powered)
                key = bt_error_key(err, "discover", powered)
                return {"ok": False, "error": _tr(key), "code": bt_error_code(key)}
            self._note_ok()
        self.notify()
        return {"ok": True}

    def _start_session(self, apath: str, aprops: dict) -> str | None:
        """SetDiscoveryFilter + StartDiscovery on ``apath``; → error text for matching, None when running."""
        _, GLib = _gi()
        roles = aprops.get("Roles")
        # Transport auto = BR/EDR inquiry interleaved with an LE scan (headsets and LE mice/keyboards alike);
        # an adapter without LE roles gets plain BR/EDR.
        transport = "bredr" if isinstance(roles, (list, tuple)) and not roles else "auto"
        filt = {"Transport": GLib.Variant("s", transport), "DuplicateData": GLib.Variant("b", False)}
        try:
            dbus_call(_sysbus(), _BZ, apath, _BZ_ADAPTER, "SetDiscoveryFilter", GLib.Variant("(a{sv})", (filt,)),
                      timeout_ms=3000)
        except Exception as e:  # noqa: BLE001
            log.info("Bluetooth SetDiscoveryFilter mislukt (failed): %s", _bt_err(e))
        try:
            dbus_call(_sysbus(), _BZ, apath, _BZ_ADAPTER, "StartDiscovery", None, timeout_ms=5000)
        except Exception as e:  # noqa: BLE001
            err = _bt_err(e)
            # InProgress / Busy: this connection already has a discovery session (BlueZ: one per client)
            if "InProgress" not in err and "Error.Busy" not in err:
                log.info("Bluetooth StartDiscovery mislukt (failed): %s", err)
                return err
        with self._lock:
            self._disc_active = True
            self._disc_adapter = apath
            still = self._disc_wanted
        if not still:                             # stopped while we were starting (discover off, shutdown)
            self._stop_session()
        return None

    def _stop_session(self) -> None:
        with self._lock:
            active, apath = self._disc_active, self._disc_adapter
            self._disc_active = False
        if active and apath:
            try:
                dbus_call(_sysbus(), _BZ, apath, _BZ_ADAPTER, "StopDiscovery", None, timeout_ms=3000)
            except Exception as e:  # noqa: BLE001
                log.debug("StopDiscovery: %s", _bt_err(e))

    def _arm_discovery_timer(self) -> None:
        # with self._lock held
        if self._disc_timer is not None:
            self._disc_timer.cancel()
        t = threading.Timer(max(0.05, self._disc_deadline - time.monotonic()), self._discovery_expired)
        t.daemon = True
        self._disc_timer = t
        t.start()

    def _discovery_expired(self) -> None:
        with self._lock:
            if not self._disc_wanted:
                return
            left = self._disc_deadline - time.monotonic()
            if left > 0.05:                       # renewed meanwhile
                self._arm_discovery_timer()
                return
        log.info("Bluetooth: zoeken na %.0f s gestopt (search stopped by itself)", BT_DISCOVERY_S)
        self._stop_discovery()
        self.notify()

    def _stop_discovery(self) -> None:
        with self._lock:
            was = self._disc_wanted or self._disc_active
            self._disc_wanted = False
            if self._disc_timer is not None:
                self._disc_timer.cancel()
                self._disc_timer = None
            if was:
                self._nearby_until = time.monotonic() + BT_NEARBY_KEEP_S
        self._stop_session()

    # -- pairing ---------------------------------------------------------------------------------------
    def _pair_emit(self, address: str, stage: str, **extra) -> None:
        data = {"address": address, "stage": stage}
        data.update({k: v for k, v in extra.items() if v is not None})
        try:
            self.emit("system.bluetooth.pairing", data)
        except Exception:  # noqa: BLE001
            log.debug("pairing event failed", exc_info=True)

    def _pair_end(self, address: str, stage: str, key: str) -> dict:
        """A pairing that did not succeed: event ``failed``/``cancelled`` + the result {ok:false, error, code}."""
        msg, code = _tr(key), bt_error_code(key)
        self._pair_emit(address, stage, error=msg, code=code)
        return {"ok": False, "error": msg, "code": code, "cancelled": stage == "cancelled"}

    def _current_agent(self, address: Any) -> "_PairAgent | None":
        with self._lock:
            p = self._pairing
            if p and isinstance(address, str) and p["address"] == address.upper():
                return p["agent"]
        return None

    def _ensure_pairable(self, apath: str) -> bool:
        """BlueZ is pairable while an agent is registered (main.conf AlwaysPairable=false); without it a pairing
        is not stored (no bond). When BlueZ does not switch it on for our agent, do it ourselves → True."""
        _, GLib = _gi()
        for _i in range(4):
            try:
                r = dbus_call(_sysbus(), _BZ, apath, _DBUS_PROPS, "Get",
                              GLib.Variant("(ss)", (_BZ_ADAPTER, "Pairable")), timeout_ms=2000)
                if r and r[0]:
                    return False
            except Exception:  # noqa: BLE001
                return False
            time.sleep(0.15)
        try:
            self._set_prop(apath, _BZ_ADAPTER, "Pairable", GLib.Variant("b", True), 3000)
            return True
        except Exception as e:  # noqa: BLE001
            log.info("Bluetooth Pairable=true mislukt (failed): %s", _bt_err(e))
            return False

    def _restore_pairable(self, apath: str) -> None:
        _, GLib = _gi()
        try:
            self._set_prop(apath, _BZ_ADAPTER, "Pairable", GLib.Variant("b", False), 3000)
        except Exception as e:  # noqa: BLE001
            log.info("Bluetooth Pairable=false mislukt (failed): %s", _bt_err(e))

    def _pair_slot(self, addr: str) -> bool:
        """Take the one pairing slot. The same address being paired already means a new dialog asks again (the
        old Instellingen window was closed mid-pairing, its question is still open): that pairing is cancelled
        and this one takes over; one that is connecting already is waited for (it ends paired)."""
        if self._pair_lock.acquire(blocking=False):
            return True
        with self._lock:
            p = self._pairing
            same = bool(p and p["address"] == addr and self._reset_job is None)
            stage = p.get("stage") if p else None
        if not same:
            return False
        if stage not in ("connecting", "done"):
            log.info("Bluetooth %s: opnieuw gevraagd, oude koppeling gestopt (asked again, old one cancelled)", addr)
            self.cancel_pair(addr)
        return self._pair_lock.acquire(timeout=BT_TAKEOVER_WAIT_S if stage not in ("connecting", "done")
                                       else BT_PAIR_TOTAL_S)

    def _forget_bond(self, path: str, why: str) -> None:
        """Our pairing was cancelled after BlueZ had made a new bond: remove it, so cancelling leaves nothing."""
        _, GLib = _gi()
        log.info("Bluetooth %s: nieuwe koppeling weer verwijderd (%s)", path.rsplit("/", 1)[-1], why)
        try:
            dbus_call(_sysbus(), _BZ, path.rsplit("/", 1)[0], _BZ_ADAPTER, "RemoveDevice",
                      GLib.Variant("(o)", (path,)), timeout_ms=10000)
        except Exception as e:  # noqa: BLE001
            log.info("Bluetooth RemoveDevice na annuleren mislukt (failed): %s", _bt_err(e))

    def _cancelled_now(self, stage: str | None = None) -> bool:
        """Was our pairing cancelled? Otherwise move it to ``stage`` (atomically: cancel_pair sees it)."""
        with self._lock:
            if self._pairing["cancelled"]:
                return True
            if stage:
                self._pairing["stage"] = stage
            return False

    def pair(self, address: Any) -> dict:
        """Pair → Trusted=true → Connect, with our temporary agent; progress as ``system.bluetooth.pairing``
        events. Blocks until done (≤ ~60 s). A failed connect after a good pairing keeps the pairing; a cancel
        at any point before 'done' leaves nothing behind (a bond BlueZ made meanwhile is removed again)."""
        _, GLib = _gi()
        addr = self._check_address(address)
        if not self._pair_slot(addr):
            with self._lock:
                key = "bt_busy" if self._reset_job is not None else "bt_busy_pairing"
            return {"ok": False, "error": _tr(key), "code": "busy", "cancelled": False}   # no event: not ours
        t0 = time.monotonic()
        agent = None
        apath = None
        pairable_set = False
        success = False
        with self._lock:                          # registered first, so an early cancelPair is never lost
            self._pairing = {"address": addr, "path": None, "agent": None, "cancelled": False, "stage": "setup",
                             "new_bond": False}
        try:
            self._pair_emit(addr, "pairing")
            try:
                objs = self._objects()
            except Exception:  # noqa: BLE001
                return self._pair_end(addr, "failed", "bt_unavailable")
            path = self._find_device(objs, addr)
            if path is None:
                return self._pair_end(addr, "failed", "bt_gone")
            apath = path.rsplit("/", 1)[0]
            if not ((objs.get(apath) or {}).get(_BZ_ADAPTER) or {}).get("Powered"):
                return self._pair_end(addr, "failed", "bt_off")
            with self._lock:
                self._pairing["path"] = path
            self._stop_session()                  # pause our search: inquiry disturbs the pairing
            bus = _sysbus()
            agent = self.agent_cls(bus, addr, path, lambda stage, **kw: self._pair_emit(addr, stage, **kw))
            try:
                agent.start()
            except Exception as e:  # noqa: BLE001
                log.info("Bluetooth-agent registreren mislukt (agent failed): %s", e)
                agent = None
                return self._pair_end(addr, "failed", "bt_pair_failed")
            with self._lock:
                self._pairing["agent"] = agent
                cancelled = self._pairing["cancelled"]
            if cancelled:
                return self._pair_end(addr, "cancelled", "bt_canceled")
            pairable_set = self._ensure_pairable(apath)
            dprops = (objs.get(path) or {}).get(_BZ_DEVICE) or {}
            had_bond = bool(dprops.get("Paired") or dprops.get("Bonded"))
            if self._cancelled_now("pair"):       # cancelled while the agent / Pairable was set up: no Pair
                return self._pair_end(addr, "cancelled", "bt_canceled")
            try:
                dbus_call(bus, _BZ, path, _BZ_DEVICE, "Pair", None, timeout_ms=int(BT_PAIR_CALL_S * 1000))
                new_bond = not had_bond
            except Exception as e:  # noqa: BLE001
                err = _bt_err(e)
                if "AlreadyExists" not in err:     # AlreadyExists: paired before, go on with trust + connect
                    return self._pair_failed(addr, path, err, agent)
                new_bond = False
            self._note_ok()
            with self._lock:
                self._pairing["new_bond"] = new_bond
            if self._cancelled_now("connecting"):  # cancelled just as BlueZ finished: no trust / connect
                return self._pair_undo(addr, path, new_bond, False, "paired, then cancelled")
            try:
                self._set_prop(path, _BZ_DEVICE, "Trusted", GLib.Variant("b", True), 3000)
            except Exception as e:  # noqa: BLE001
                log.info("Bluetooth Trusted=true mislukt (failed): %s", _bt_err(e))
            self._pair_emit(addr, "connecting")
            if self._cancelled_now():
                return self._pair_undo(addr, path, new_bond, False, "cancelled before connecting")
            left = max(BT_CONNECT_MIN_S, BT_PAIR_TOTAL_S - (time.monotonic() - t0))
            connected, error, code = True, None, None
            try:
                dbus_call(bus, _BZ, path, _BZ_DEVICE, "Connect", None, timeout_ms=int(left * 1000))
            except Exception as e:  # noqa: BLE001
                err = _bt_err(e)
                if "AlreadyConnected" not in err:
                    connected = False
                    log.info("Bluetooth verbinden na koppelen mislukt (connect after pair failed): %s", err)
                    error, code = _tr("bt_paired_not_connected"), "connect_failed"
            if self._cancelled_now("done"):       # cancelled while connecting (dialog closed): undo, no audio
                return self._pair_undo(addr, path, new_bond, connected, "cancelled while connecting")
            res = {"ok": True, "paired": True, "connected": connected}
            if error:
                res.update(error=error, code=code)
            success = True
            self._pair_emit(addr, "done", connected=connected, error=error, code=code)
            log.info("Bluetooth %s gekoppeld (paired), verbonden (connected)=%s", addr, connected)
            if connected:
                self._follow_audio(addr, bt_kind(self._device_props(path)))
            return res
        finally:
            if agent is not None:
                agent.stop()
            if pairable_set and apath:
                self._restore_pairable(apath)
            with self._lock:
                self._pairing = None
                # the search goes on after a failed or cancelled pairing (the user picks again), not after a
                # good one: inquiry next to a fresh A2DP stream makes it stutter, and Windows stops too
                resume = not success and self._disc_wanted and time.monotonic() < self._disc_deadline
            self._pair_lock.release()
            if resume:
                try:
                    _ap, aprops = self._adapter_or_raise()
                    if aprops.get("Powered"):
                        self._start_session(_ap, aprops)
                except Exception:  # noqa: BLE001
                    log.debug("resume discovery failed", exc_info=True)
            self.notify()

    def _pair_undo(self, addr: str, path: str, new_bond: bool, connected: bool, why: str) -> dict:
        """Our pairing was cancelled after Pair went through: a bond it made is removed again (that also drops
        the connection), else a connection it made is closed. No trust/connect/audio switch follows."""
        log.info("Bluetooth %s geannuleerd na het koppelen (%s)", addr, why)
        if new_bond:
            self._forget_bond(path, why)
        elif connected:
            self._disconnect_quietly(path)
        return self._pair_end(addr, "cancelled", "bt_canceled")

    def _pair_failed(self, addr: str, path: str, err: str, agent: "_PairAgent") -> dict:
        with self._lock:
            user_cancel = bool(self._pairing and self._pairing.get("cancelled"))
        powered = self._powered_now()
        self._note_error(err, powered)
        if user_cancel or agent.declined:
            stage, key = "cancelled", "bt_canceled"
        elif agent.timed_out:
            stage, key = "failed", "bt_reply_timeout"
        else:
            key = bt_error_key(err, "pair", powered)
            stage = "cancelled" if key == "bt_canceled" else "failed"
        if "Timeout was reached" in err or "NoReply" in err:     # our call gave up: stop BlueZ too
            self._cancel_pairing_call(path)
        log.info("Bluetooth koppelen mislukt (pair failed): %s → %s", err, stage)
        return self._pair_end(addr, stage, key)

    @staticmethod
    def _cancel_pairing_call(path: str) -> None:
        try:
            dbus_call(_sysbus(), _BZ, path, _BZ_DEVICE, "CancelPairing", None, timeout_ms=3000)
        except Exception as e:  # noqa: BLE001
            log.debug("CancelPairing: %s", _bt_err(e))

    @staticmethod
    def _disconnect_quietly(path: str) -> None:
        try:
            dbus_call(_sysbus(), _BZ, path, _BZ_DEVICE, "Disconnect", None, timeout_ms=5000)
        except Exception as e:  # noqa: BLE001
            log.debug("Disconnect: %s", _bt_err(e))

    def pair_reply(self, address: Any, accept: Any, passkey: Any = None, pin: Any = None, qid: Any = None) -> dict:
        """Answer the agent's open question for the device being paired (confirm / passkey / PIN / allow).
        ``qid`` = the question's id from its pairing event: a yes for a question that was replaced meanwhile is
        refused ({ok:false, code:'stale'}), the new one stays open."""
        self._check_address(address)
        agent = self._current_agent(address)
        if agent is None:
            return {"ok": False, "error": _tr("bt_no_request")}
        return agent.reply(_bool(accept), passkey, pin, qid)

    def cancel_pair(self, address: Any) -> dict:
        """Stop OUR pairing of ``address``: an open question is answered Canceled, then Device1.CancelPairing
        (while connecting: Device1.Disconnect ends the Connect; pair() then removes a bond it made). Someone
        else's pairing (bluetoothctl …) is never touched; once our pairing is done it is too late."""
        addr = self._check_address(address)
        with self._lock:
            p = self._pairing
            if not p or p["address"] != addr or p.get("stage") == "done":
                return {"ok": True}
            p["cancelled"] = True
            agent, path, stage = p["agent"], p["path"], p.get("stage")
        log.info("Bluetooth koppelen %s geannuleerd (pairing cancelled by the user) in stage %s", addr, stage)
        if agent is not None:
            agent.cancel()
        if path and stage == "connecting":
            self._disconnect_quietly(path)
        elif path and agent is not None:
            self._cancel_pairing_call(path)
        return {"ok": True}

    # -- audio follows a new headset (like Windows) ----------------------------------------------------
    def _follow_audio(self, address: str, kind: str) -> None:
        if kind not in _BT_AUDIO_KINDS or self.audio is None or not Audio.available():
            return
        t = threading.Thread(target=self._audio_switch, args=(address,), name="bt-audio", daemon=True)
        self._audio_thread = t
        t.start()

    def _audio_switch(self, address: str) -> None:
        """Wait for the headset's sink; when WirePlumber did not make it the default within a moment, do it
        (Audio.set_sink → _default_target picks the Bluetooth loopback filter on WirePlumber 0.5+)."""
        deadline = time.monotonic() + BT_AUDIO_WAIT_S
        first = None
        cache: dict = {}
        try:
            while time.monotonic() < deadline:
                hit = _bt_sink_for(self.audio, address, cache)
                if hit:
                    sid, is_default = hit
                    if is_default:
                        log.info("Bluetooth-audio %s is al de standaarduitvoer (already default)", address)
                        return
                    if first is None:
                        first = time.monotonic()
                    elif time.monotonic() - first >= BT_AUDIO_GRACE_S:
                        log.info("Bluetooth-audio %s wordt de standaarduitvoer (sink %s → default)", address, sid)
                        self.on_audio(self.audio.set_sink(sid))
                        return
                time.sleep(0.4)
            log.info("Bluetooth-audio %s: geen geluidsapparaat verschenen (no sink appeared)", address)
        except Exception:  # noqa: BLE001
            log.info("Bluetooth-audio als standaard instellen mislukt (failed)", exc_info=True)

    # -- connect / remove ------------------------------------------------------------------------------
    def remove(self, address: Any) -> dict:
        """Forget a device (Win11 "Apparaat verwijderen"): Adapter1.RemoveDevice also drops the pairing keys,
        so the device can be paired again from scratch. A device that is already gone counts as removed."""
        addr = self._check_address(address)
        try:
            path = self._device_path(addr)
        except ApiError as e:
            if e.code == "not_found":
                return {"ok": True}
            return {"ok": False, "error": e.message, "code": e.code}
        if self._current_agent(addr) is not None:
            self.cancel_pair(addr)                # removing the device we are pairing: stop that first
        _, GLib = _gi()
        try:
            dbus_call(_sysbus(), _BZ, path.rsplit("/", 1)[0], _BZ_ADAPTER, "RemoveDevice",
                      GLib.Variant("(o)", (path,)), timeout_ms=10000)
        except Exception as e:  # noqa: BLE001
            err = _bt_err(e)
            log.info("Bluetooth-apparaat verwijderen mislukt (remove failed): %s", err)
            if "DoesNotExist" in err or "UnknownObject" in err:
                return {"ok": True}
            powered = self._powered_now()
            self._note_error(err, powered)
            key = bt_error_key(err, "remove", powered)
            return {"ok": False, "error": _tr(key), "code": bt_error_code(key)}
        self._note_ok()
        log.info("Bluetooth-apparaat %s verwijderd (removed)", addr)
        return {"ok": True}

    def connect(self, address: Any, connect: bool) -> dict:
        """Connect / disconnect a paired device. A headset the user connects becomes the sound output, like
        Windows (only when WirePlumber does not do that by itself)."""
        path = self._device_path(address)
        op = "connect" if connect else "disconnect"
        try:
            dbus_call(_sysbus(), _BZ, path, _BZ_DEVICE, "Connect" if connect else "Disconnect", None,
                      timeout_ms=30000)
        except Exception as e:  # noqa: BLE001
            err = _bt_err(e)
            log.info("Bluetooth %s mislukt (failed): %s", op, err)
            if "AlreadyConnected" in err or (not connect and "NotConnected" in err):
                return {"ok": True}
            powered = self._powered_now()
            self._note_error(err, powered)
            key = bt_error_key(err, op, powered)
            return {"ok": False, "error": _tr(key), "code": bt_error_code(key)}
        self._note_ok()
        if connect:
            self._follow_audio(str(address).upper(), bt_kind(self._device_props(path)))
        return {"ok": True}

    def shutdown(self) -> None:
        """Host stops: end our search and an open pairing (BlueZ would also drop both with our connection).
        The search goes first, so the pairing's end does not resume it."""
        try:
            self._stop_discovery()
        except Exception:  # noqa: BLE001
            pass
        with self._lock:
            p = self._pairing
            if p:
                p["cancelled"] = True
        if p and p.get("agent") is not None:
            p["agent"].cancel()
            if p.get("path"):
                self._cancel_pairing_call(p["path"])


# ---------------------------------------------------------------------------
# Power
# ---------------------------------------------------------------------------

_POWER_ACTIONS = ("shutdown", "reboot", "suspend", "logout", "lock")


def hyprlock_cmd() -> list[str]:
    """hyprlock with our lock screen: $WIN11SHELL_HYPRLOCK_CONFIG, else <config>/hypr/hyprlock.conf, when present."""
    conf = os.environ.get("WIN11SHELL_HYPRLOCK_CONFIG") or ""
    if not conf:
        cfg = os.environ.get("WIN11SHELL_CONFIG_DIR") or os.path.join(
            os.environ.get("XDG_CONFIG_HOME") or os.path.expanduser("~/.config"), "win11shell")
        conf = os.path.join(os.path.expanduser(cfg), "hypr", "hyprlock.conf")
    conf = os.path.expanduser(conf)
    return ["hyprlock", "-c", conf] if os.path.isfile(conf) else ["hyprlock"]


def power_action(action: str, comp: str) -> dict:
    if action == "shutdown":
        r = run(["systemctl", "poweroff"], timeout=15)
    elif action == "reboot":
        r = run(["systemctl", "reboot"], timeout=15)
    elif action == "suspend":
        r = run(["systemctl", "suspend"], timeout=15)
    elif action == "logout":
        if comp == "hyprland":
            if hypr_dispatch("exit"):
                return {"ok": True}
        if comp == "gnome" and which("gnome-session-quit"):
            r = run(["gnome-session-quit", "--logout", "--no-prompt"], timeout=15)
        else:
            sid = _session_id()
            if not sid:
                raise ApiError("failed", "Afmelden lukte niet: geen sessie gevonden.")
            r = run(["loginctl", "terminate-session", sid], timeout=15)
    elif action == "lock":
        if comp == "hyprland" and which("hyprlock") and not _proc_running("hypridle"):
            # Without hypridle nobody listens to logind's Lock signal: start the locker ourselves,
            # with our Windows-style lock screen (win11shell-session exports WIN11SHELL_HYPRLOCK_CONFIG).
            if spawn(hyprlock_cmd()) is not None:
                return {"ok": True}
        sid = _session_id()
        r = run(["loginctl", "lock-session"] + ([sid] if sid else []), timeout=10)
    else:
        raise ApiError("bad_args", "Onbekende actie.")
    if not r.ok:
        low = (r.err or "").lower()
        log.warning("Energieactie %s mislukt (power action failed): %s", action, r.err.strip()[:300])
        if "inhibit" in low:
            raise ApiError("inhibited", "Een app houdt dit op dit moment tegen. Sluit die app en probeer het opnieuw.")
        if "interactive authentication required" in low or "access denied" in low:
            raise ApiError("denied", "Dit mag nu niet: er zijn misschien andere gebruikers aangemeld.")
        raise ApiError("failed", "Dat lukte niet. Probeer het opnieuw.")
    return {"ok": True}


# ---------------------------------------------------------------------------
# Night light (hyprsunset)
# ---------------------------------------------------------------------------

class NightLight:
    def __init__(self):
        self._proc: subprocess.Popen | None = None
        self._lock = threading.Lock()
        self.enabled = False
        self.temperature = 4500

    @staticmethod
    def supported() -> bool:
        return bool(which("hyprsunset")) and compositor() == "hyprland"

    def get(self) -> dict:
        return {"supported": self.supported(), "enabled": self.enabled, "temperature": self.temperature,
                "tool": "hyprsunset" if which("hyprsunset") else None}

    def _kill_own(self) -> None:
        p, self._proc = self._proc, None
        if p is not None and p.poll() is None:
            try:
                p.terminate()
                p.wait(timeout=2)
            except Exception:  # noqa: BLE001
                try:
                    p.kill()
                except Exception:  # noqa: BLE001
                    pass

    def set(self, enabled: bool, temperature: Any) -> dict:
        if not self.supported():
            raise ApiError("unsupported", "Nachtlampje werkt hier niet. Daarvoor zijn Hyprland en hyprsunset nodig.")
        temp = self.temperature if temperature is None else max(1000, min(10000, int(_num(temperature))))
        with self._lock:
            external = self._proc is None and _proc_running("hyprsunset")
            if external:
                # A hyprsunset started by someone else: use its IPC (hyprsunset ≥ 0.2) instead of a 2nd copy.
                cmd = f"hyprsunset temperature {temp}" if enabled else "hyprsunset identity"
                res = hyprctl(cmd)
                if res is not None and "ok" in res.lower():
                    self.enabled, self.temperature = bool(enabled), temp
                    return self.get()
            self._kill_own()
            if enabled:
                p = spawn(["hyprsunset", "-t", str(temp)], detach=False)
                if p is None:
                    raise ApiError("failed", "Nachtlampje kon niet worden gestart.")
                self._proc = p
            self.enabled, self.temperature = bool(enabled), temp
        return self.get()

    def stop(self) -> None:
        with self._lock:
            self._kill_own()


# ---------------------------------------------------------------------------
# Device info
# ---------------------------------------------------------------------------

def _os_release() -> dict:
    out = {}
    for path in ("/etc/os-release", "/usr/lib/os-release"):
        text = _read(path)
        if not text:
            continue
        for line in text.splitlines():
            if "=" in line and not line.startswith("#"):
                k, v = line.split("=", 1)
                try:
                    v = shlex.split(v)[0] if v.strip() else ""
                except ValueError:
                    v = v.strip().strip('"')
                out[k.strip()] = v
        break
    return out


_BAD_DMI = re.compile(r"^(to be filled.*|system product name|system manufacturer|default string|o\.e\.m\.?|"
                      r"not applicable|none|n/a|\s*)$", re.I)


def _device_model() -> str:
    d = Path("/sys/class/dmi/id")
    vendor = _read(d / "sys_vendor", "") or ""
    product = _read(d / "product_name", "") or ""
    version = _read(d / "product_version", "") or ""
    if _BAD_DMI.match(product):
        vendor = _read(d / "board_vendor", "") or vendor
        product = _read(d / "board_name", "") or ""
    if _BAD_DMI.match(vendor):
        vendor = ""
    if product and version and not _BAD_DMI.match(version) and version.lower() not in product.lower() \
            and "lenovo" in vendor.lower():
        product = version  # Lenovo keeps the marketing name in product_version
    vendor = re.sub(r"\s+(Co\.,?\s*Ltd\.?|Inc\.?|Corporation|Corp\.?|GmbH|Technology)\b.*$", "", vendor).strip()
    return " ".join(x for x in (vendor, product) if x).strip() or platform.node()


def cpu_info() -> dict:
    model = ""
    cores: set = set()
    threads = 0
    phys = core = None
    try:
        with open("/proc/cpuinfo", "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                if ":" not in line:
                    if phys is not None or core is not None:
                        cores.add((phys, core))
                    phys = core = None
                    continue
                k, v = (x.strip() for x in line.split(":", 1))
                if k == "processor":
                    threads += 1
                elif k == "model name" and not model:
                    model = v
                elif k == "physical id":
                    phys = v
                elif k == "core id":
                    core = v
        if phys is not None or core is not None:
            cores.add((phys, core))
    except OSError:
        pass
    model = re.sub(r"\s+", " ", re.sub(r"\((R|TM)\)", "", model)).strip()
    threads = threads or (os.cpu_count() or 1)
    return {"model": model or platform.processor() or "Processor", "cores": len(cores) or threads,
            "threads": threads}


def _meminfo() -> dict:
    out = {}
    try:
        with open("/proc/meminfo", "r") as f:
            for line in f:
                k, _, rest = line.partition(":")
                parts = rest.split()
                if parts:
                    out[k] = int(parts[0]) * (1024 if len(parts) > 1 else 1)
    except OSError:
        pass
    return out


def _installed_memory() -> int | None:
    """Physical RAM from memory-hotplug blocks (≈ what is installed), None if unknown."""
    base = Path("/sys/devices/system/memory")
    bs = _read(base / "block_size_bytes")
    if not bs:
        return None
    try:
        block = int(bs, 16)
        n = 0
        for d in base.glob("memory*"):
            st = _read(d / "online", "1")
            if st == "1":
                n += 1
        return block * n if n else None
    except (ValueError, OSError):
        return None


def _avatar(name: str) -> str | None:
    for p in (Path("/var/lib/AccountsService/icons") / name, Path.home() / ".face", Path.home() / ".face.icon"):
        try:
            if p.is_file() and os.access(p, os.R_OK) and p.stat().st_size > 0:
                return str(p)
        except OSError:
            continue
    return None


def _disks() -> list[dict]:
    r = run(["lsblk", "-J", "-b", "-d", "-o", "NAME,MODEL,SIZE,TYPE,TRAN,ROTA,RM"], timeout=4)
    out = []
    if not r.ok:
        return out
    try:
        data = json.loads(r.out)
    except ValueError:
        return out
    for d in data.get("blockdevices", []):
        name = d.get("name") or ""
        if d.get("type") != "disk" or name.startswith(("zram", "loop", "ram", "md", "sr")):
            continue
        tran = (d.get("tran") or "").lower()
        rota = d.get("rota") in (True, "1", 1)
        kind = "nvme" if tran == "nvme" or name.startswith("nvme") else "usb" if tran == "usb" else \
            "hdd" if rota else "ssd"
        out.append({"model": (d.get("model") or name).strip(), "size": int(d.get("size") or 0),
                    "kind": kind, "device": name})
    return out


def _desktop_name(comp: str) -> str:
    if comp == "hyprland":
        return _tr("desktop_hypr")
    if comp == "gnome":
        return _tr("desktop_gnome")
    cur = os.environ.get("XDG_CURRENT_DESKTOP", "")
    return cur.split(":")[0] if cur else "Win11 Shell"


def _installed_at() -> int | None:
    for p in ("/var/log/installer", "/etc/machine-id"):
        try:
            return int(os.stat(p).st_mtime * 1000)
        except OSError:
            continue
    return None


class Info:
    def __init__(self):
        self._cache: tuple[float, dict] | None = None
        self._lock = threading.Lock()

    def get(self) -> dict:
        with self._lock:
            if self._cache and time.monotonic() - self._cache[0] < 120:
                data = dict(self._cache[1])
            else:
                data = self._collect()
                self._cache = (time.monotonic(), data)
                data = dict(data)
        try:
            data["uptime"] = int(float(_read("/proc/uptime", "0").split()[0]))
        except (ValueError, IndexError, AttributeError):
            data["uptime"] = None
        return data

    def _collect(self) -> dict:
        osr = _os_release()
        u = user_info()
        mem = _meminfo()
        comp = compositor()
        try:
            from .sysmon import gpu_names
            gpus = gpu_names()
        except Exception:  # noqa: BLE001
            log.debug("gpu names failed", exc_info=True)
            gpus = []
        host = socket.gethostname()
        pretty_host = None
        mi = _read("/etc/machine-info")
        if mi:
            m = re.search(r'^PRETTY_HOSTNAME=(.*)$', mi, re.M)
            if m:
                try:
                    pretty_host = shlex.split(m.group(1))[0]
                except (ValueError, IndexError):
                    pretty_host = None
        un = platform.uname()
        return {
            "hostname": host,
            "prettyHostname": pretty_host or host,                                  # extra
            "os": osr.get("PRETTY_NAME") or f"{un.system}",
            "osVersion": osr.get("VERSION") or osr.get("VERSION_ID") or "",
            "osId": osr.get("ID") or "",                                            # extra
            "kernel": un.release,
            "architecture": un.machine,                                             # extra
            "deviceModel": _device_model(),
            "cpu": cpu_info(),
            "memory": {"total": mem.get("MemTotal", 0), "installed": _installed_memory()},
            "gpus": gpus,
            "disks": _disks(),
            "uptime": None,
            "user": {"name": u["name"], "realName": u["realName"], "avatar": _avatar(u["name"])},
            "desktop": _desktop_name(comp),
            "sessionType": os.environ.get("XDG_SESSION_TYPE") or ("wayland" if os.environ.get("WAYLAND_DISPLAY")
                                                                  else "x11"),   # extra
            "installedAt": _installed_at(),                                         # extra
        }


# ---------------------------------------------------------------------------
# Updates (apt, read-only)
# ---------------------------------------------------------------------------

class Updates:
    def __init__(self, emit: Callable[[str, Any], Any]):
        self._emit = emit
        self._lock = threading.Lock()
        self._result: dict | None = None
        self._checked_mono = 0.0
        self._thread: threading.Thread | None = None

    @staticmethod
    def tool() -> str | None:
        return "apt" if which("apt") else None

    def _check(self) -> None:
        r = run(["apt", "list", "--upgradable"], timeout=60, env=_c_env())
        available = security = None
        if r.ok:
            lines = [ln for ln in r.out.splitlines() if "[upgradable from" in ln]
            available = len(lines)
            security = sum(1 for ln in lines if "-security" in ln.split(" ", 1)[0])
        result = {"available": available, "checkedAt": int(time.time() * 1000), "tool": "apt",
                  "security": security, "checking": False}
        with self._lock:
            changed = self._result is None or self._result.get("available") != available
            self._result = result
            self._checked_mono = time.monotonic()
            self._thread = None
        if changed:
            self._emit("system.updates.changed", dict(result))

    def get(self, refresh: bool = False) -> dict:
        tool = self.tool()
        if not tool:
            return {"available": None, "checkedAt": None, "tool": None, "checking": False}
        with self._lock:
            stale = self._result is None or refresh or time.monotonic() - self._checked_mono > UPDATES_TTL_S
            if stale and self._thread is None:
                self._thread = threading.Thread(target=self._safe_check, name="system-updates", daemon=True)
                self._thread.start()
            res = dict(self._result) if self._result else {"available": None, "checkedAt": None, "tool": tool,
                                                           "security": None}
            res["checking"] = self._thread is not None
        return res

    def _safe_check(self) -> None:
        try:
            self._check()
        except Exception:  # noqa: BLE001
            log.exception("Updatecontrole faalde (update check failed)")
            with self._lock:
                self._thread = None

    def wait(self, timeout: float) -> None:
        t = self._thread
        if t is not None:
            t.join(timeout)


def open_updater() -> dict:
    for cmd in (["update-manager"], ["gnome-software", "--mode=updates"], ["snap-store", "--mode=updates"]):
        if which(cmd[0]) and spawn(cmd) is not None:
            return {"ok": True, "app": cmd[0]}
    raise ApiError("unsupported", "Er is geen programma voor updates gevonden.")


# ---------------------------------------------------------------------------
# Time
# ---------------------------------------------------------------------------

_locale_cache: list = []


def _locale_h24() -> bool | None:
    if _locale_cache:
        return _locale_cache[0]
    r = run(["locale", "t_fmt"], timeout=2)
    fmt = r.out.strip() if r.ok else ""
    val = None if not r.ok else not ("%I" in fmt or "%r" in fmt or "%l" in fmt or "%p" in fmt)
    _locale_cache.append(val)
    return val


def time_info(settings_h24: bool | None) -> dict:
    props = {}
    r = run(["timedatectl", "show"], timeout=4, env=_c_env())
    if r.ok:
        for line in r.out.splitlines():
            if "=" in line:
                k, v = line.split("=", 1)
                props[k] = v
    tz = props.get("Timezone")
    if not tz:
        try:
            tz = os.path.realpath("/etc/localtime").split("/zoneinfo/", 1)[1]
        except (IndexError, OSError):
            tz = time.tzname[0] if time.tzname else "UTC"
    sys_h24 = _locale_h24()
    return {
        "timezone": tz,
        "ntp": props.get("NTP", "no") == "yes",
        "h24": settings_h24 if settings_h24 is not None else (sys_h24 if sys_h24 is not None else True),
        "ntpSynced": props.get("NTPSynchronized", "no") == "yes",   # extra
        "canNTP": props.get("CanNTP", "no") == "yes",               # extra
        "localRTC": props.get("LocalRTC", "no") == "yes",           # extra
        "systemH24": sys_h24,                                       # extra: from LC_TIME
        "now": int(time.time() * 1000),                             # extra
    }


# ---------------------------------------------------------------------------
# Poller
# ---------------------------------------------------------------------------

def _die_with_parent() -> Callable[[], None] | None:
    """preexec_fn: the child gets SIGTERM when the thread that started it dies (host killed), so an idle
    reader such as ``pw-dump --monitor`` never lingers. libc is looked up before fork (async-signal-safe)."""
    try:
        import ctypes
        prctl = ctypes.CDLL(None, use_errno=True).prctl
    except (OSError, AttributeError):
        return None
    import signal as _signal
    sig = int(_signal.SIGTERM)

    def pre() -> None:
        prctl(1, sig, 0, 0, 0)            # PR_SET_PDEATHSIG

    return pre


class AudioMonitor:
    """Follows PipeWire with ONE long-lived ``pw-dump --monitor`` reader instead of spawning ``wpctl`` every
    few seconds. pw-dump writes the whole graph once, then one JSON array per change; an array that touches
    a node, device or metadata object (volume, mute, outputs, default output) or removes an object calls
    ``on_change()``. Read-only. If pw-dump is missing or keeps exiting, ``alive`` stays False and the
    poller keeps polling as before."""

    _RELEVANT = ("PipeWire:Interface:Node", "PipeWire:Interface:Device", "PipeWire:Interface:Metadata")
    MAX_BUFFER = 8 * 1024 * 1024

    def __init__(self, on_change: Callable[[], None], on_alive: Callable[[bool], None] | None = None,
                 argv: list[str] | None = None, max_failures: int = 5):
        self.on_change = on_change
        self.on_alive = on_alive
        self.argv = argv or ["pw-dump", "--monitor", "--no-colors"]
        self.max_failures = max_failures      # quick exits in a row before giving up (then: polling)
        self.alive = False
        self._stop = threading.Event()
        self._proc: subprocess.Popen | None = None
        self._thread: threading.Thread | None = None

    def start(self) -> bool:
        if self._thread is not None or not which(self.argv[0]):
            return False
        self._thread = threading.Thread(target=self._run, name="audio-monitor", daemon=True)
        self._thread.start()
        return True

    def stop(self) -> None:
        self._stop.set()
        p = self._proc
        if p is not None and p.poll() is None:
            try:
                p.terminate()
                p.wait(timeout=2)
            except (OSError, subprocess.TimeoutExpired):
                try:
                    p.kill()
                except OSError:
                    pass

    def _set_alive(self, v: bool) -> None:
        if v != self.alive:
            self.alive = v
            if self.on_alive:
                try:
                    self.on_alive(v)
                except Exception:  # noqa: BLE001
                    log.debug("audio monitor on_alive failed", exc_info=True)

    @classmethod
    def relevant(cls, chunk: Any) -> bool:
        if not isinstance(chunk, list):
            return False
        for o in chunk:
            if isinstance(o, dict) and (o.get("type") in cls._RELEVANT or o.get("info", 0) is None):
                return True
        return False

    def feed(self, buf: str, first: bool) -> tuple[str, bool]:
        """Parse complete top-level arrays from ``buf``; → (rest, still_first). The first array is the
        initial dump (no change)."""
        dec = json.JSONDecoder()
        i, n = 0, len(buf)
        while True:
            while i < n and buf[i] in " \t\r\n,":
                i += 1
            if i >= n:
                return "", first
            try:
                obj, j = dec.raw_decode(buf, i)
            except ValueError:
                return buf[i:], first          # incomplete: wait for more output
            i = j
            if first:
                first = False
                self._set_alive(True)
            elif self.relevant(obj):
                try:
                    self.on_change()
                except Exception:  # noqa: BLE001
                    log.debug("audio monitor on_change failed", exc_info=True)

    def _run(self) -> None:
        failures = 0
        while not self._stop.is_set():
            started = time.monotonic()
            try:
                p = subprocess.Popen(self.argv, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                     stdin=subprocess.DEVNULL, close_fds=True, preexec_fn=_die_with_parent())
            except OSError as e:
                log.info("pw-dump niet te starten (cannot start): %s", e)
                break
            self._proc = p
            buf, first = "", True
            dec = codecs.getincrementaldecoder("utf-8")("replace")
            try:
                assert p.stdout is not None
                fd = p.stdout.fileno()
                while not self._stop.is_set():
                    data = os.read(fd, 65536)
                    if not data:
                        break
                    buf, first = self.feed(buf + dec.decode(data), first)
                    if len(buf) > self.MAX_BUFFER:
                        log.warning("pw-dump: onverwachte uitvoer, opnieuw starten (unexpected output, restarting)")
                        break
            except OSError:
                pass
            finally:
                self._set_alive(False)
                if p.poll() is None:
                    try:
                        p.kill()
                    except OSError:
                        pass
                try:
                    p.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    pass
                self._proc = None
            if self._stop.is_set():
                break
            failures = 0 if time.monotonic() - started > 60 else failures + 1
            if failures > self.max_failures:
                log.info("pw-dump stopt steeds; geluid wordt weer gepeild (keeps exiting; polling instead)")
                break
            self._stop.wait(min(30, 2 ** failures))
        self._set_alive(False)


class Poller:
    """Polls every domain (cheap reads), emits ``system.<domain>.changed`` only on change."""

    def __init__(self, api, sources: dict[str, tuple[Callable[[], dict], int]]):
        self.api = api
        self.sources = sources            # name -> (fn, every_n_ticks)
        self.last: dict[str, str] = {}
        self.wake = threading.Event()
        self.stopped = threading.Event()
        self.dirty: set[str] = set()
        self._lock = threading.Lock()
        self._thread: threading.Thread | None = None
        self._last_run: dict[str, float] = {}

    def start(self) -> None:
        if self._thread is None:
            self._thread = threading.Thread(target=self._loop, name="system-poller", daemon=True)
            self._thread.start()

    def stop(self) -> None:
        self.stopped.set()
        self.wake.set()

    def poke(self, name: str) -> None:
        with self._lock:
            self.dirty.add(name)
        self.wake.set()

    def publish(self, name: str, data: Any) -> None:
        """Called after an API change: update the baseline and emit right away."""
        if not isinstance(data, dict) or data.get("dryRun"):
            return
        s = json_dumps(data)
        with self._lock:
            changed = self.last.get(name) != s
            self.last[name] = s
        if changed:
            self.api.emit(f"system.{name}.changed", data)

    def refresh(self, name: str) -> None:
        fn = self.sources.get(name, (None, 0))[0]
        if fn is None:
            return
        try:
            data = fn()
        except Exception:  # noqa: BLE001
            log.debug("poll %s failed", name, exc_info=True)
            return
        s = json_dumps(data)
        with self._lock:
            first = name not in self.last
            changed = self.last.get(name) != s
            self.last[name] = s
        self._last_run[name] = time.monotonic()
        if changed and not first:
            self.api.emit(f"system.{name}.changed", data)

    def _loop(self) -> None:
        tick = 0
        for name in self.sources:          # baseline
            if self.stopped.is_set():
                return
            self.refresh(name)
        next_tick = time.monotonic() + POLL_S
        while not self.stopped.is_set():
            woke = self.wake.wait(max(0.0, next_tick - time.monotonic()))
            if self.stopped.is_set():
                return
            if woke:
                self.wake.clear()
                time.sleep(0.2)            # coalesce bursts of D-Bus signals
                with self._lock:
                    dirty, self.dirty = self.dirty, set()
                now = time.monotonic()
                for name in dirty:
                    if now - self._last_run.get(name, 0) >= 0.5:
                        self.refresh(name)
                    else:
                        self.poke_later(name)
            if time.monotonic() < next_tick:
                continue
            next_tick = time.monotonic() + POLL_S
            tick += 1
            if self.api.subscriber_count() == 0:
                continue                    # nobody listens: stay idle
            for name, (_fn, every) in self.sources.items():
                if tick % max(1, every) == 0:
                    self.refresh(name)

    def poke_later(self, name: str) -> None:
        t = threading.Timer(0.6, self.poke, args=(name,))
        t.daemon = True
        t.start()


# ---------------------------------------------------------------------------
# registration
# ---------------------------------------------------------------------------

_state: dict[str, Any] = {}


def register(api, ctx):
    global _lang

    def lang() -> str:
        try:
            return "en" if ctx.settings.get("language", "nl") == "en" else "nl"
        except Exception:  # noqa: BLE001
            return "nl"

    _lang = lang
    audio, net, bat = Audio(), Network(), Battery()
    bt = Bluetooth(audio)
    night, info, updates = NightLight(), Info(), Updates(api.emit)
    comp = ctx.compositor

    def audio_poll() -> dict:
        # Without the PipeWire monitor: volume every tick (one `wpctl get-volume`, ~8 ms), the device list
        # (`wpctl status`, sometimes `inspect`) every 4th tick. With it (AudioMonitor alive) this only runs
        # when PipeWire reports a change, plus a slow safety poll; then the device list is always fresh.
        audio_poll.n = getattr(audio_poll, "n", 0) + 1
        monitored = bool(getattr(audio_poll, "monitored", False))
        return audio.get(fresh_sinks=monitored or audio_poll.n % 4 == 1)

    has_backlight = _backlight() is not None
    poller = Poller(api, {
        "audio": (audio_poll, 1),
        "brightness": (brightness_get, 1 if has_backlight else 1_000_000),
        "battery": (bat.get, 2),
        "network": (net.get, 2),
        "bluetooth": (bt.get, 2),
    })

    def _monitor_alive(alive: bool) -> None:
        audio_poll.monitored = alive
        # PipeWire tells us about changes: poll only as a safety net (every 8 ticks = 20 s), else every tick
        poller.sources["audio"] = (audio_poll, AUDIO_MONITORED_EVERY if alive else 1)

    audio_monitor = AudioMonitor(lambda: poller.poke("audio"), _monitor_alive)
    bt.emit = api.emit
    bt.notify = lambda: poller.poke("bluetooth")
    bt.on_audio = lambda res: poller.publish("audio", res)
    _state.update(poller=poller, night=night, updates=updates, audio=audio, net=net, bt=bt,
                  audio_monitor=audio_monitor)

    def changed(name: str, fn: Callable[[], dict]):
        try:
            poller.publish(name, fn())
        except Exception:  # noqa: BLE001
            log.debug("publish %s failed", name, exc_info=True)

    # -- audio --------------------------------------------------------------
    @api.method("system.audio.get")
    def _audio_get(args):
        """{available, volume (0-100), muted, sinks:[{id, name, default}]}"""
        return audio.get()

    def _audio_dry(args):
        try:
            return audio.get(fresh_sinks=False)
        except Exception:  # noqa: BLE001
            return {}

    @api.method("system.audio.set", dangerous=True, dry_result=_audio_dry)
    def _audio_set(args):
        """{volume?: 0-100, delta?: ±n, muted?: bool|'toggle'} → new audio state"""
        res = audio.set(args)
        poller.publish("audio", res)
        return res

    @api.method("system.audio.setSink", dangerous=True)
    def _audio_sink(args):
        res = audio.set_sink(args.get("id"))
        poller.publish("audio", res)
        return res

    # -- brightness ---------------------------------------------------------
    @api.method("system.brightness.get")
    def _br_get(args):
        return brightness_get()

    @api.method("system.brightness.set", dangerous=True, dry_result=lambda a: brightness_get())
    def _br_set(args):
        res = brightness_set(args)
        poller.publish("brightness", res)
        return res

    # -- battery ------------------------------------------------------------
    @api.method("system.battery.get")
    def _bat_get(args):
        return bat.get()

    # -- network ------------------------------------------------------------
    @api.method("system.network.get")
    def _net_get(args):
        return net.get()

    @api.method("system.network.wifiList")
    def _net_list(args):
        """{rescan} → [{ssid, strength, secure, active, saved}]"""
        return net.wifi_list(bool(args.get("rescan")))

    @api.method("system.network.setWifi", dangerous=True)
    def _net_wifi(args):
        res = net.set_wifi(_bool(args.get("enabled")))
        poller.publish("network", res)
        return res

    @api.method("system.network.setAirplane", dangerous=True)
    def _net_airplane(args):
        set_airplane(_bool(args.get("enabled")), bt)
        time.sleep(0.3)
        net.invalidate()
        res = net.get()
        poller.publish("network", res)
        changed("bluetooth", bt.get)
        return res

    @api.method("system.network.connect", dangerous=True)
    def _net_connect(args):
        """{ssid, password?} → {ok, error?}"""
        res = net.connect(args.get("ssid"), args.get("password"))
        changed("network", net.get)
        return res

    @api.method("system.network.disconnect", dangerous=True)
    def _net_disconnect(args):
        res = net.disconnect(args.get("iface"))
        changed("network", net.get)
        return res

    # -- bluetooth ----------------------------------------------------------
    @api.method("system.bluetooth.get")
    def _bt_get(args):
        """{available, powered, powerState, stuck, discovering, name, blocked, devices:[…], nearby:[…]}"""
        return bt.get()

    @api.method("system.bluetooth.setPowered", dangerous=True)
    def _bt_power(args):
        res = bt.set_powered(_bool(args.get("powered")))
        poller.publish("bluetooth", res)
        return res

    @api.method("system.bluetooth.connect", dangerous=True)
    def _bt_connect(args):
        res = bt.connect(args.get("address"), True)
        changed("bluetooth", bt.get)
        return res

    @api.method("system.bluetooth.disconnect", dangerous=True)
    def _bt_disconnect(args):
        res = bt.connect(args.get("address"), False)
        changed("bluetooth", bt.get)
        return res

    @api.method("system.bluetooth.remove", dangerous=True)
    def _bt_remove(args):
        """{address} → {ok, error?, code?}: forget the device (Adapter1.RemoveDevice); already gone = ok"""
        res = bt.remove(args.get("address"))
        changed("bluetooth", bt.get)
        return res

    @api.method("system.bluetooth.discover", dangerous=True, dry_result={})
    def _bt_discover(args):
        """{on} → {ok, error?, code?}: search for devices (stops by itself after 60 s unless renewed)"""
        res = bt.discover(_bool(args.get("on", True)))
        changed("bluetooth", bt.get)
        return res

    @api.method("system.bluetooth.pair", dangerous=True)
    def _bt_pair(args):
        """{address} → {ok, paired?, connected?, error?, code?, cancelled?} (≤ 60 s; events
        system.bluetooth.pairing {address, stage, qid?, passkey?, pin?, entered?, connected?, error?, code?})"""
        res = bt.pair(args.get("address"))
        changed("bluetooth", bt.get)
        return res

    @api.method("system.bluetooth.pairReply", dangerous=True)
    def _bt_pair_reply(args):
        """{address, accept, qid?, passkey?, pin?} → {ok, error?, code?}: answer the open pairing question (qid from
        its pairing event; a yes for a question that was replaced meanwhile → {ok:false, code:'stale'})"""
        return bt.pair_reply(args.get("address"), args.get("accept", False), args.get("passkey"), args.get("pin"),
                             args.get("qid"))

    @api.method("system.bluetooth.cancelPair", dangerous=True)
    def _bt_cancel_pair(args):
        """{address} → {ok}: stop our pairing of this device (Device1.CancelPairing)"""
        return bt.cancel_pair(args.get("address"))

    def _bt_dry_state(args):
        try:
            return bt.get()
        except Exception:  # noqa: BLE001
            return {}

    @api.method("system.bluetooth.reset", dangerous=True, dry_result=_bt_dry_state)
    def _bt_reset(args):
        """{} → state like get: "Bluetooth opnieuw starten" (rfkill cycle when stuck, else Powered off/on)"""
        res = bt.reset()
        poller.publish("bluetooth", res)
        return res

    # -- power --------------------------------------------------------------
    @api.method("system.power.action", dangerous=True)
    def _power(args):
        """{action: shutdown|reboot|suspend|logout|lock} (the UI confirms first)"""
        action = args.get("action")
        if action not in _POWER_ACTIONS:
            raise ApiError("bad_args", "Onbekende actie.")
        log.info("Energieactie (power action): %s", action)
        return power_action(action, comp)

    # -- night light --------------------------------------------------------
    @api.method("system.nightLight.get")
    def _nl_get(args):
        return night.get()

    @api.method("system.nightLight.set", dangerous=True)
    def _nl_set(args):
        """{enabled, temperature} – hyprsunset (Hyprland); else error 'unsupported'"""
        return night.set(_bool(args.get("enabled")), args.get("temperature"))

    # -- info / updates / time ----------------------------------------------
    @api.method("system.info")
    def _info(args):
        return info.get()

    @api.method("system.updates")
    def _updates(args):
        """{available:int|null, checkedAt, tool:'apt'|null, checking, security} (cached 30 min)"""
        return updates.get(bool(args.get("refresh")))

    @api.method("system.openUpdater", dangerous=True)
    def _open_updater(args):
        return open_updater()

    @api.method("system.time")
    def _time(args):
        try:
            h24 = ctx.settings.get("time.h24", None)
        except Exception:  # noqa: BLE001
            h24 = None
        return time_info(h24 if isinstance(h24, bool) else None)

    # settings → night light follows settings.nightLight when supported (Hyprland)
    def on_settings(settings, path=None, changed_paths=None):
        try:
            paths = changed_paths or ([path] if path else [])
            if paths and not any(str(p).startswith("nightLight") for p in paths):
                return
            nl = (settings or {}).get("nightLight") or {}
            if not night.supported() or dry_for(ctx):
                return
            want = bool(nl.get("enabled"))
            temp = nl.get("temperature", night.temperature)
            if want != night.enabled or (want and temp != night.temperature):
                night.set(want, temp)
        except Exception:  # noqa: BLE001
            log.debug("night light settings sync failed", exc_info=True)

    try:
        ctx.settings.on_change(on_settings)
    except Exception:  # noqa: BLE001
        pass


def start(api, ctx):
    poller: Poller = _state["poller"]
    poller.start()
    mon: AudioMonitor | None = _state.get("audio_monitor")
    if mon is not None and Audio.available():
        mon.start()
    # Wake the poller on D-Bus property changes (instant Wi-Fi/Bluetooth/battery updates).
    try:
        Gio, _ = _gi()
        bus = _sysbus()

        def cb(_conn, _sender, _path, _iface, _signal, _params, domain):
            poller.poke(domain)

        for name, domain in ((_NM, "network"), (_BZ, "bluetooth"), (_UP, "battery")):
            bus.signal_subscribe(name, "org.freedesktop.DBus.Properties", "PropertiesChanged", None, None,
                                 Gio.DBusSignalFlags.NONE, cb, domain)
        bus.signal_subscribe(_BZ, "org.freedesktop.DBus.ObjectManager", None, "/", None,
                             Gio.DBusSignalFlags.NONE, cb, "bluetooth")
        bus.signal_subscribe(_NM, _NM, "DeviceAdded", None, None, Gio.DBusSignalFlags.NONE,
                             lambda *a: (_state["net"].invalidate(), poller.poke("network")), None)
        bus.signal_subscribe(_NM, _NM, "DeviceRemoved", None, None, Gio.DBusSignalFlags.NONE,
                             lambda *a: (_state["net"].invalidate(), poller.poke("network")), None)
    except Exception as e:  # noqa: BLE001
        log.info("D-Bus-signalen niet beschikbaar (signals unavailable): %s", e)
    # Apply the saved night light (Hyprland + hyprsunset only, never in dry-run).
    try:
        nl = ctx.settings.get("nightLight", {}) or {}
        night: NightLight = _state["night"]
        if nl.get("enabled") and night.supported() and not dry_for(ctx):
            night.set(True, nl.get("temperature", 4500))
    except Exception:  # noqa: BLE001
        log.debug("night light start failed", exc_info=True)


def stop(api, ctx):
    p = _state.get("poller")
    if p:
        p.stop()
    b = _state.get("bt")
    if b:
        b.shutdown()
    m = _state.get("audio_monitor")
    if m:
        m.stop()
    n = _state.get("night")
    if n:
        n.stop()
