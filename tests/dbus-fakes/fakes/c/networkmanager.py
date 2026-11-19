"""Batch C's D-Bus fake (tests/dbus-fakes/run.sh --batch c; template ../b0/echo.py): NetworkManager on the private
system bus (one Wi-Fi profile "Lab-5G" connected on wlan0, which can be an access point and also sees "eduroam", a
WPA2-Enterprise network without a profile; one wired profile for eth0
without a cable; an OpenVPN and a WireGuard profile; a gsm profile for the modem of modemmanager.py, whose NM device
is cdc-wdm0) and gnome-settings-daemon's Rfkill on the private session bus.

No object is registered: a message filter answers every method call to the two names itself, also
org.freedesktop.DBus.Properties (Get, GetAll, Set), so each call is seen with its flags. Every call is appended to
the file in argv[1] as {bus, path, iface, method, signature, args (GVariant text), interactive}; a call
with another in-signature than NetworkManager's is refused with InvalidArgs. With `--system-only` (argv[2];
tests/gnome/scenarios/settings-network.py, whose private session may have its own gsd) it owns only NetworkManager. Update2, AddConnection2,
AddAndActivateConnection2, Delete, the switches and (de)activation change the fake's state, so later reads see them:
an activated profile gets an ActiveConnection object, the secrets of a stored profile move to GetSecrets and are left
out of GetSettings, as NetworkManager does."""
import json
import sys
import threading
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
LOCK = threading.Lock()
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
NM = "org.freedesktop.NetworkManager"
P = "/org/freedesktop/NetworkManager"
RFKILL = "org.gnome.SettingsDaemon.Rfkill"
RFKILL_PATH = "/org/gnome/SettingsDaemon/Rfkill"
WIFI_UUID = "6a1f0c52-8d3e-4b7a-9c11-2f5e8d4a7b01"
ETH_UUID = "0b9e4d21-73aa-4c6f-8e2d-5a1c9f3e6d02"
VPN_UUID = "3c2b1a09-8f7e-4d6c-9b5a-4e3d2c1b0a93"
WG_UUID = "4d3c2b1a-9f8e-4e7d-8c6b-5f4e3d2c1b04"
GSM_UUID = "5e4d3c2b-a09f-4f8e-9d7c-6a5f4e3d2c15"
MODEM = "/org/freedesktop/ModemManager1/Modem/0"
ICCID = "8931440000000000017"


def v(t, x):
    return GLib.Variant(t, x)


CONNS = {
    P + "/Settings/1": v("a{sa{sv}}", {
        "connection": {"id": v("s", "Lab-5G"), "uuid": v("s", WIFI_UUID), "type": v("s", "802-11-wireless"),
                       "autoconnect": v("b", True), "timestamp": v("t", 1763280000), "zone": v("s", "home"),
                       "permissions": v("as", [])},
        "802-11-wireless": {"ssid": v("ay", b"Lab-5G"), "mode": v("s", "infrastructure"),
                            "seen-bssids": v("as", ["AA:BB:CC:00:11:22"]), "powersave": v("u", 2)},
        "802-11-wireless-security": {"key-mgmt": v("s", "wpa-psk"), "psk-flags": v("u", 0)},
        "ipv4": {"method": v("s", "auto"), "addresses": v("aau", []), "address-data": v("aa{sv}", []),
                 "dns": v("au", []), "dns-data": v("as", [])},
        "ipv6": {"method": v("s", "auto"), "addr-gen-mode": v("i", 1)},
        "proxy": {},
        "x-vendor": {"pairs": v("a(ii)", [(1, 2)])},
    }),
    P + "/Settings/2": v("a{sa{sv}}", {
        "connection": {"id": v("s", "Wired connection 1"), "uuid": v("s", ETH_UUID),
                       "type": v("s", "802-3-ethernet"), "interface-name": v("s", "eth0"),
                       "autoconnect": v("b", True), "timestamp": v("t", 1763000000)},
        "802-3-ethernet": {"auto-negotiate": v("b", False)},
        "ipv4": {"method": v("s", "auto")},
        "ipv6": {"method": v("s", "auto")},
    }),
    P + "/Settings/3": v("a{sa{sv}}", {
        "connection": {"id": v("s", "Kantoor"), "uuid": v("s", VPN_UUID), "type": v("s", "vpn"),
                       "autoconnect": v("b", False), "timestamp": v("t", 1762000000)},
        "vpn": {"service-type": v("s", "org.freedesktop.NetworkManager.openvpn"),
                "data": v("a{ss}", {"remote": "vpn.example.org", "connection-type": "password",
                                    "username": "robin", "password-flags": "1", "ca": "/etc/openvpn/ca.crt",
                                    "x-extra": "1"})},
        "ipv4": {"method": v("s", "auto")},
        "ipv6": {"method": v("s", "auto")},
    }),
    P + "/Settings/4": v("a{sa{sv}}", {
        "connection": {"id": v("s", "wg0"), "uuid": v("s", WG_UUID), "type": v("s", "wireguard"),
                       "interface-name": v("s", "wg0"), "autoconnect": v("b", False)},
        "wireguard": {"listen-port": v("u", 51820), "private-key-flags": v("u", 0),
                      "peers": v("aa{sv}", [{"public-key": v("s", "cGVlcnB1YmxpY2tleWZvcnRoZWZha2VwZWVyMDA9"),
                                            "endpoint": v("s", "wg.example.org:51820"),
                                            "allowed-ips": v("as", ["0.0.0.0/0"]),
                                            "persistent-keepalive": v("u", 25)}])},
        "ipv4": {"method": v("s", "manual"),
                 "address-data": v("aa{sv}", [{"address": v("s", "10.8.0.2"), "prefix": v("u", 24)}])},
        "ipv6": {"method": v("s", "disabled")},
    }),
    P + "/Settings/5": v("a{sa{sv}}", {
        "connection": {"id": v("s", "KPN Internet"), "uuid": v("s", GSM_UUID), "type": v("s", "gsm"),
                       "autoconnect": v("b", True)},
        "gsm": {"apn": v("s", "portalmmm.nl"), "home-only": v("b", True), "sim-id": v("s", ICCID)},
        "ipv4": {"method": v("s", "auto")},
        "ipv6": {"method": v("s", "auto")},
    }),
}
SECRETS = {
    P + "/Settings/1": {"802-11-wireless-security": {"psk": "geheimwachtwoord"}},
    P + "/Settings/3": {"vpn": {"secrets": {"password": "vpnwachtwoord"}}},
    P + "/Settings/4": {"wireguard": {"private-key": "cHJpdmF0ZWtleWZvcnRoZWZha2Vjb25uZWN0aW9uMDA="}},
}
# the keys NetworkManager keeps out of GetSettings (and answers with GetSecrets)
SECRET_KEYS = {
    "802-11-wireless-security": {"psk", "wep-key0", "wep-key1", "wep-key2", "wep-key3", "leap-password"},
    "802-1x": {"password", "pin", "private-key-password", "ca-cert-password", "client-cert-password",
               "phase2-private-key-password"},
    "vpn": {"secrets"},
    "wireguard": {"private-key"},
    "gsm": {"password", "pin"},
}
OBJECTS = {
    P: {NM: {"WirelessEnabled": v("b", True), "WirelessHardwareEnabled": v("b", True), "WwanEnabled": v("b", True),
             "WwanHardwareEnabled": v("b", True), "NetworkingEnabled": v("b", True),
             "ActiveConnections": v("ao", [P + "/ActiveConnection/1"]),
             "PrimaryConnection": v("o", P + "/ActiveConnection/1"), "Connectivity": v("u", 4),
             "ConnectivityCheckEnabled": v("b", True), "ConnectivityCheckAvailable": v("b", True),
             "Version": v("s", "1.54.3"), "State": v("u", 70)}},
    P + "/Settings": {NM + ".Settings": {"Connections": v("ao", list(CONNS)), "CanModify": v("b", True)}},
    P + "/Devices/1": {
        NM + ".Device": {"Interface": v("s", "wlan0"), "DeviceType": v("u", 2), "State": v("u", 100),
                         "Driver": v("s", "iwlwifi"), "DriverVersion": v("s", "6.17.0"),
                         "FirmwareVersion": v("s", "89.202a2f7b.0"), "HwAddress": v("s", "AA:BB:CC:00:11:22"),
                         "Mtu": v("u", 1500), "Managed": v("b", True), "Autoconnect": v("b", True),
                         "ActiveConnection": v("o", P + "/ActiveConnection/1"), "Ip4Config": v("o", P + "/IP4Config/1"),
                         "Ip6Config": v("o", "/"), "AvailableConnections": v("ao", [P + "/Settings/1"])},
        NM + ".Device.Wireless": {"PermHwAddress": v("s", "AA:BB:CC:00:11:22"), "Bitrate": v("u", 866000),
                                  "ActiveAccessPoint": v("o", P + "/AccessPoint/1"),
                                  "AccessPoints": v("ao", [P + "/AccessPoint/1", P + "/AccessPoint/2"]),
                                  "WirelessCapabilities": v("u", 0x200 | 0x400 | 0x8 | 0x10 | 0x20 | 0x40),
                                  "Mode": v("u", 2)},
    },
    P + "/Devices/2": {
        NM + ".Device": {"Interface": v("s", "eth0"), "DeviceType": v("u", 1), "State": v("u", 20),
                         "Driver": v("s", "e1000e"), "DriverVersion": v("s", "6.17.0"), "FirmwareVersion": v("s", ""),
                         "HwAddress": v("s", "AA:BB:CC:00:00:01"), "Mtu": v("u", 1500), "Managed": v("b", True),
                         "Autoconnect": v("b", True), "ActiveConnection": v("o", "/"), "Ip4Config": v("o", "/"),
                         "Ip6Config": v("o", "/"), "AvailableConnections": v("ao", [P + "/Settings/2"])},
        NM + ".Device.Wired": {"Carrier": v("b", False), "Speed": v("u", 0), "PermHwAddress": v("s", "AA:BB:CC:00:00:01")},
    },
    P + "/Devices/3": {
        NM + ".Device": {"Interface": v("s", "cdc-wdm0"), "DeviceType": v("u", 8), "State": v("u", 30),
                         "Udi": v("s", MODEM), "Driver": v("s", "qmi_wwan"), "DriverVersion": v("s", ""),
                         "FirmwareVersion": v("s", ""), "HwAddress": v("s", ""), "Mtu": v("u", 1500),
                         "Managed": v("b", True), "Autoconnect": v("b", True), "ActiveConnection": v("o", "/"),
                         "Ip4Config": v("o", "/"), "Ip6Config": v("o", "/"),
                         "AvailableConnections": v("ao", [P + "/Settings/5"])},
    },
    P + "/AccessPoint/1": {NM + ".AccessPoint": {
        "Ssid": v("ay", b"Lab-5G"), "Frequency": v("u", 5180), "HwAddress": v("s", "AA:BB:CC:00:11:22"),
        "Strength": v("y", 80), "Flags": v("u", 1), "WpaFlags": v("u", 0), "RsnFlags": v("u", 0x188),
        "MaxBitrate": v("u", 866000), "Mode": v("u", 2), "Bandwidth": v("u", 80)}},
    # WPA2-Enterprise: RSN key management 802.1X (0x200) with CCMP
    P + "/AccessPoint/2": {NM + ".AccessPoint": {
        "Ssid": v("ay", b"eduroam"), "Frequency": v("u", 2437), "HwAddress": v("s", "AA:BB:CC:00:22:33"),
        "Strength": v("y", 55), "Flags": v("u", 1), "WpaFlags": v("u", 0), "RsnFlags": v("u", 0x288),
        "MaxBitrate": v("u", 270000), "Mode": v("u", 2), "Bandwidth": v("u", 20)}},
    P + "/IP4Config/1": {NM + ".IP4Config": {
        "AddressData": v("aa{sv}", [{"address": v("s", "192.168.1.20"), "prefix": v("u", 24)}]),
        "Gateway": v("s", "192.168.1.1"), "NameserverData": v("aa{sv}", [{"address": v("s", "192.168.1.1")}]),
        "Domains": v("as", ["lan"])}},
    P + "/ActiveConnection/1": {NM + ".Connection.Active": {
        "Connection": v("o", P + "/Settings/1"), "Uuid": v("s", WIFI_UUID), "Id": v("s", "Lab-5G"),
        "Type": v("s", "802-11-wireless"), "State": v("u", 2), "Devices": v("ao", [P + "/Devices/1"]),
        "Default": v("b", True), "Default6": v("b", False), "SpecificObject": v("o", P + "/AccessPoint/1")}},
    RFKILL_PATH: {RFKILL: {"AirplaneMode": v("b", False), "HardwareAirplaneMode": v("b", False),
                           "HasAirplaneMode": v("b", True), "ShouldShowAirplaneMode": v("b", True),
                           "BluetoothAirplaneMode": v("b", False), "BluetoothHasAirplaneMode": v("b", True)}},
}
for path in CONNS:
    OBJECTS[path] = {NM + ".Settings.Connection": {"Unsaved": v("b", False), "Flags": v("u", 0),
                                                   "Filename": v("s", "/etc/NetworkManager/system-connections/x")}}
COUNTER = {"n": 10}


def new_path(kind):
    COUNTER["n"] += 1
    return "%s/%s/%d" % (P, kind, COUNTER["n"])


def uuid_of(settings):
    return settings.unpack()["connection"].get("uuid", "")


class Fail(Exception):
    def __init__(self, name, text):
        super().__init__(text)
        self.name = name


def secrets_of(settings):
    """The secrets of a settings variant: {group: {key: python value}}."""
    out = {}
    for group, keys in settings.unpack().items():
        for key, value in keys.items():
            if key in SECRET_KEYS.get(group, ()):
                out.setdefault(group, {})[key] = value
    return out


def public(settings):
    """The settings without their secrets (GetSettings)."""
    out = {}
    for i in range(settings.n_children()):
        entry = settings.get_child_value(i)
        group = entry.get_child_value(0).get_string()
        d = entry.get_child_value(1)
        keys = {}
        for j in range(d.n_children()):
            kv = d.get_child_value(j)
            key = kv.get_child_value(0).get_string()
            if key not in SECRET_KEYS.get(group, ()):
                keys[key] = kv.get_child_value(1).get_variant()
        out[group] = keys
    return GLib.Variant("a{sa{sv}}", out)


def store(conn, settings, replace):
    CONNS[conn] = settings
    found = secrets_of(settings)
    if replace or found:
        old = {} if replace else SECRETS.get(conn, {})
        for group, keys in found.items():
            old.setdefault(group, {}).update(keys)
        SECRETS[conn] = old


def activate(conn, device):
    """A new ActiveConnection object for a stored profile on a device ("/" for either: NetworkManager picks)."""
    active = new_path("ActiveConnection")
    if conn in CONNS:
        c = CONNS[conn].unpack()["connection"]
        devices = [device] if device != "/" else []
        OBJECTS[active] = {NM + ".Connection.Active": {
            "Connection": v("o", conn), "Uuid": v("s", c.get("uuid", "")), "Id": v("s", c.get("id", "")),
            "Type": v("s", c.get("type", "")), "State": v("u", 2), "Devices": v("ao", devices),
            "Default": v("b", False), "Default6": v("b", False), "SpecificObject": v("o", "/")}}
        root = OBJECTS[P][NM]
        root["ActiveConnections"] = v("ao", root["ActiveConnections"].unpack() + [active])
        if device in OBJECTS:
            OBJECTS[device][NM + ".Device"]["ActiveConnection"] = v("o", active)
    return active


def deactivate(active):
    OBJECTS.pop(active, None)
    root = OBJECTS[P][NM]
    root["ActiveConnections"] = v("ao", [a for a in root["ActiveConnections"].unpack() if a != active])
    for obj in OBJECTS.values():
        dev = obj.get(NM + ".Device")
        if dev and dev["ActiveConnection"].unpack() == active:
            dev["ActiveConnection"] = v("o", "/")


def connection_props(conn):
    return OBJECTS.setdefault(conn, {NM + ".Settings.Connection": {"Unsaved": v("b", False), "Flags": v("u", 0),
                                                                    "Filename": v("s", "")}})


# NetworkManager's in-signatures of the methods WinPoint calls (introspection of NM 1.54); another one is refused, as
# NetworkManager refuses it
SIGNATURES = {
    "Update2": "(a{sa{sv}}ua{sv})", "AddConnection2": "(a{sa{sv}}ua{sv})",
    "AddAndActivateConnection2": "(a{sa{sv}}ooa{sv})", "ActivateConnection": "(ooo)", "DeactivateConnection": "(o)",
    "GetSecrets": "(s)", "GetConnectionByUuid": "(s)", "Reapply": "(a{sa{sv}}tu)", "RequestScan": "(a{sv})",
    "Set": "(ssv)", "Get": "(ss)", "GetAll": "(s)",
}


def handle(path, iface, method, body):
    """The reply body of one call (a GLib.Variant tuple or None), or Fail."""
    want = SIGNATURES.get(method)
    got = body.get_type_string() if body is not None else "()"
    if want and got != want:
        raise Fail("org.freedesktop.DBus.Error.InvalidArgs", "%s wants %s, got %s" % (method, want, got))
    args = body.unpack() if body is not None else ()
    if iface == "org.freedesktop.DBus.Properties":
        props = OBJECTS.get(path, {}).get(args[0])
        if props is None:
            raise Fail("org.freedesktop.DBus.Error.UnknownInterface", "no interface %s at %s" % (args[0], path))
        if method == "GetAll":
            return v("(a{sv})", (props,))
        if method == "Get":
            if args[1] not in props:
                raise Fail("org.freedesktop.DBus.Error.UnknownProperty", args[1])
            return GLib.Variant.new_tuple(GLib.Variant("v", props[args[1]]))
        if method == "Set":
            props[args[1]] = body.get_child_value(2).get_variant()
            return None
    if path == P and iface == NM:
        if method in ("GetDevices", "GetAllDevices"):
            return v("(ao)", ([p for p in OBJECTS if p.startswith(P + "/Devices/")],))
        if method == "ActivateConnection":
            return v("(o)", (activate(args[0], args[1]),))
        if method == "DeactivateConnection":
            if args[0] not in OBJECTS:
                raise Fail(NM + ".Manager.ConnectionNotActive", "not active")
            deactivate(args[0])
            return None
        if method == "AddAndActivateConnection2":
            conn = new_path("Settings")
            store(conn, body.get_child_value(0), True)
            connection_props(conn)
            return GLib.Variant("(ooa{sv})", (conn, activate(conn, args[1]), {}))
    if path == P + "/Settings" and iface == NM + ".Settings":
        if method == "ListConnections":
            return v("(ao)", (list(CONNS),))
        if method == "GetConnectionByUuid":
            for conn, settings in CONNS.items():
                if uuid_of(settings) == args[0]:
                    return v("(o)", (conn,))
            raise Fail(NM + ".Settings.InvalidConnection", "No connection with the UUID was found")
        if method == "AddConnection2":
            conn = new_path("Settings")
            store(conn, body.get_child_value(0), True)
            connection_props(conn)
            return GLib.Variant("(oa{sv})", (conn, {}))
    if path in CONNS and iface == NM + ".Settings.Connection":
        if method == "GetSettings":
            return GLib.Variant.new_tuple(public(CONNS[path]))
        if method == "GetSecrets":
            group = SECRETS.get(path, {}).get(args[0])
            if group is None:
                raise Fail(NM + ".AgentManager.NoSecrets", "no secrets")
            typed = {k: v("a{ss}", x) if isinstance(x, dict) else v("s", x) for k, x in group.items()}
            return v("(a{sa{sv}})", ({args[0]: typed},))
        if method == "Update2":
            # an update with secrets replaces the stored ones; one without keeps them (NetworkManager)
            store(path, body.get_child_value(0), bool(secrets_of(body.get_child_value(0))))
            return v("(a{sv})", ({},))
        if method == "Delete":
            del CONNS[path]
            return None
    if path.startswith(P + "/Devices/"):
        if method in ("Disconnect", "Reapply", "RequestScan"):
            return None
        if method in ("GetAccessPoints", "GetAllAccessPoints") and NM + ".Device.Wireless" in OBJECTS.get(path, {}):
            return v("(ao)", (OBJECTS[path][NM + ".Device.Wireless"]["AccessPoints"].unpack(),))
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s at %s" % (iface, method, path))


def make_filter(kind, names):
    def filt(conn, message, incoming):
        if not incoming or message.get_message_type() != Gio.DBusMessageType.METHOD_CALL:
            return message
        if message.get_destination() not in names and message.get_destination() != conn.get_unique_name():
            return message
        path, iface, method, body = message.get_path(), message.get_interface(), message.get_member(), message.get_body()
        with LOCK:
            CALLS.write(json.dumps({"bus": kind, "path": path, "iface": iface, "method": method,
                                    "signature": body.get_type_string() if body is not None else "()",
                                    "args": body.print_(True) if body is not None else "",
                                    "interactive": bool(message.get_flags() & FLAG)}) + "\n")
            try:
                result = handle(path, iface, method, body)
                reply = Gio.DBusMessage.new_method_reply(message)
                if result is not None:
                    reply.set_body(result)
            except Fail as error:
                reply = Gio.DBusMessage.new_method_error_literal(message, error.name, str(error))
            except Exception as error:  # a bug in the fake: answer, never hang the caller
                reply = Gio.DBusMessage.new_method_error_literal(message, "org.freedesktop.DBus.Error.Failed", repr(error))
        conn.send_message(reply, Gio.DBusSendMessageFlags.NONE)
        return None
    return filt


owned = []
loop = GLib.MainLoop()
NAMES = [("system", Gio.BusType.SYSTEM, NM), ("session", Gio.BusType.SESSION, RFKILL)]
if sys.argv[2:3] == ["--system-only"]:
    NAMES = NAMES[:1]
for kind, bus_type, name in NAMES:
    bus = Gio.bus_get_sync(bus_type, None)
    bus.add_filter(make_filter(kind, {name}))

    def acquired(*_args):
        owned.append(1)
        if len(owned) == len(NAMES):
            print("ready", flush=True)

    Gio.bus_own_name_on_connection(bus, name, Gio.BusNameOwnerFlags.NONE, acquired, lambda *a: loop.quit())
loop.run()
