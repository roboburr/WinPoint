"""Batch C's sharing fake (tests/dbus-fakes/run.sh --batch c; template ../b0/echo.py): the daemons behind
host/src/services/sharing.rs, on the private buses.

Session bus: systemd's user manager (org.freedesktop.systemd1, unit gnome-remote-desktop.service, stopped),
gnome-settings-daemon's Sharing plugin (org.gnome.SettingsDaemon.Sharing: current network Lab-5G, status available,
rygel shared on another network "Kantoor") and, once that unit runs, gnome-remote-desktop's own RDP server
(org.gnome.RemoteDesktop.User, /org/gnome/RemoteDesktop/Rdp/Server, a negotiated port 3390).
System bus: systemd's system manager (ssh.socket and ssh.service appear once openssh-server is installed, ssh.socket
enabled and running as Ubuntu's postinst leaves it), gnome-remote-desktop's configuration daemon
(org.gnome.RemoteDesktop.Configuration: no certificate, no credentials, disabled; Enable and Disable emit
PropertiesChanged), PackageKit (a transaction per CreateTransaction; Resolve of openssh-server answers one Package,
then Finished; InstallPackages reports Percentage 50 and Finished, then the ssh units exist) and polkit's Authority
(org.freedesktop.PolicyKit1: CheckAuthorization of GNOME Settings' org.gnome.controlcenter.remote-login-helper).

Polkit as Ubuntu has it for these actions (auth_admin_keep, no rule): a call that would show the password dialog is
logged with "dialog": true, and the authorization is then kept per caller and action id; remote-login-helper implies
systemd's manage-units (StartUnit, StopUnit) and manage-unit-files (EnableUnitFiles, DisableUnitFiles), so those then
pass without a dialog. Control files next to the calls file (argv[1] + "."): `forget` drops every kept authorization
(as after auth_admin_keep's few minutes), `dismiss` closes the next dialog (polkit.dismissed; systemd answers
AccessDenied), `no-action` makes the next CheckAuthorization answer that the action is not registered (no
gnome-control-center). Each is removed once used.

As networkmanager.py: no object is registered, a message filter answers every call to these names and appends it to
the file in argv[1] as {bus, path, iface, method, signature, args (GVariant text), interactive}; a call with another
in-signature than the daemon's is refused with InvalidArgs. The system writes (systemd's Start/Stop/Enable/Disable,
the configuration daemon's methods, InstallPackages) are refused with InteractiveAuthorizationRequired unless the
caller allows the password dialog, as polkit's auth_admin_keep does without an agent. ImportCertificate reads both
file descriptors and logs the first line of the certificate and the key's PEM header, never the key.

Options: --system-only (the system bus names only), --no-user-systemd (no user manager on the session bus: in the
harness, WinPoint's launcher starts apps through the user manager when there is one), --user-systemd-only (only the
user manager, for the moment a scenario needs it)."""
import base64
import hashlib
import json
import os
import sys
import threading
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
CONTROL = sys.argv[1] + "."
OPTS = set(sys.argv[2:])
LOCK = threading.Lock()
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
SD = "org.freedesktop.systemd1"
SD_PATH = "/org/freedesktop/systemd1"
SD_MANAGER = SD + ".Manager"
GSD = "org.gnome.SettingsDaemon.Sharing"
GSD_PATH = "/org/gnome/SettingsDaemon/Sharing"
GRD_USER = "org.gnome.RemoteDesktop.User"
GRD_RDP_PATH = "/org/gnome/RemoteDesktop/Rdp/Server"
GRD_RDP = "org.gnome.RemoteDesktop.Rdp.Server"
GRD_CONF = "org.gnome.RemoteDesktop.Configuration"
GRD_CONF_PATH = "/org/gnome/RemoteDesktop/Configuration"
GRD_CONF_IFACE = GRD_CONF + ".Rdp.Server"
PK = "org.freedesktop.PackageKit"
PK_PATH = "/org/freedesktop/PackageKit"
PK_TX = PK + ".Transaction"
PROPS = "org.freedesktop.DBus.Properties"
LAB = "0c7e3a52-1b2d-4f6e-8a90-5d4c3b2a1f00"
OFFICE = "9b8a7c6d-5e4f-4321-8765-0fedcba98765"
PACKAGE_ID = "openssh-server;1:10.2p1-2ubuntu3.8;amd64;ubuntu"
POLKIT = "org.freedesktop.PolicyKit1"
POLKIT_PATH = "/org/freedesktop/PolicyKit1/Authority"
POLKIT_IFACE = POLKIT + ".Authority"
REMOTE_LOGIN = "org.gnome.controlcenter.remote-login-helper"
SD_ACTIONS = {"StartUnit": "org.freedesktop.systemd1.manage-units", "StopUnit": "org.freedesktop.systemd1.manage-units",
              "EnableUnitFiles": "org.freedesktop.systemd1.manage-unit-files",
              "DisableUnitFiles": "org.freedesktop.systemd1.manage-unit-files"}
IMPLIES = {REMOTE_LOGIN: set(SD_ACTIONS.values())}
KEPT = set()

UNITS = {
    "session": {"gnome-remote-desktop.service": {"file": "disabled", "active": "inactive"}},
    "system": {},
}
# after InstallPackages, as Ubuntu's openssh-server postinst leaves them: the socket enabled and listening
SSH_UNITS = {"ssh.socket": {"file": "enabled", "active": "active"},
             "ssh.service": {"file": "disabled", "active": "inactive"}}
SHARED = {"rygel": [OFFICE]}
CONF = {"Enabled": False, "Port": 3389, "TlsCert": "", "TlsFingerprint": "", "TlsKey": "", "credentials": {}}
TX = {"n": 0}
CONNS = {}

SIGNATURES = {
    "GetUnitFileState": "(s)", "LoadUnit": "(s)", "GetUnit": "(s)", "StartUnit": "(ss)", "StopUnit": "(ss)",
    "EnableUnitFiles": "(asbb)", "DisableUnitFiles": "(asb)", "Get": "(ss)", "GetAll": "(s)",
    "ListNetworks": "(s)", "EnableService": "(s)", "DisableService": "(ss)",
    "Enable": "()", "Disable": "()", "GetCredentials": "()", "SetCredentials": "(a{sv})",
    "ImportCertificate": "((sh)(sh))", "CreateTransaction": "()", "SetHints": "(as)", "Resolve": "(tas)",
    "InstallPackages": "(tas)", "CheckAuthorization": "((sa{sv})sa{ss}us)",
}
SYSTEM_WRITES = {"StartUnit", "StopUnit", "EnableUnitFiles", "DisableUnitFiles", "Enable", "Disable",
                 "GetCredentials", "SetCredentials", "ImportCertificate", "InstallPackages"}


class Fail(Exception):
    def __init__(self, name, text):
        super().__init__(text)
        self.name = name


def v(t, x):
    return GLib.Variant(t, x)


def control(what):
    """Whether the test left the control file `what` (removed: each is used once)."""
    try:
        os.remove(CONTROL + what)
        return True
    except FileNotFoundError:
        return False


def kept(sender, action):
    if control("forget"):
        KEPT.clear()
    return (sender, action) in KEPT or any((sender, a) in KEPT and action in IMPLIES.get(a, ()) for a in IMPLIES)


def polkit(path, iface, method, args, message, note):
    if path != POLKIT_PATH or iface != POLKIT_IFACE or method != "CheckAuthorization":
        raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s" % (iface, method))
    (kind, subject), action, _details, flags, _cancel = args
    sender = message.get_sender()
    if kind != "system-bus-name" or subject.get("name") != sender:
        raise Fail("org.freedesktop.PolicyKit1.Error.NotAuthorized", "only the caller's own bus name (fake)")
    if control("no-action") or action != REMOTE_LOGIN:
        raise Fail("org.freedesktop.PolicyKit1.Error.Failed", "Action %s is not registered" % action)
    if kept(sender, action):
        return v("((bba{ss}))", ((True, False, {}),))
    if not flags & 1:
        return v("((bba{ss}))", ((False, True, {}),))
    note["dialog"] = True
    if control("dismiss"):
        return v("((bba{ss}))", ((False, False, {"polkit.dismissed": "true"}),))
    KEPT.add((sender, action))
    return v("((bba{ss}))", ((True, False, {}),))


def systemd_polkit(method, message, note):
    """polkit's check of a system manager write (the caller already allowed the dialog)."""
    action = SD_ACTIONS.get(method)
    if action is None or kept(message.get_sender(), action):
        return
    note["dialog"] = True
    if control("dismiss"):
        raise Fail("org.freedesktop.DBus.Error.AccessDenied", "Access denied")
    KEPT.add((message.get_sender(), action))


def unit_path(unit):
    return SD_PATH + "/unit/" + "".join(c if c.isalnum() else "_%02x" % ord(c) for c in unit)


def units(bus):
    return UNITS[bus]


def unit_of(bus, name):
    u = units(bus).get(name)
    if u is None:
        raise Fail(SD + ".NoSuchUnit", "Unit %s not found." % name)
    return u


def systemd(bus, path, iface, method, args):
    if iface == PROPS and method == "Get" and args[0] == SD + ".Unit":
        for name, u in units(bus).items():
            if unit_path(name) == path:
                return GLib.Variant.new_tuple(v("v", v("s", u["active"] if args[1] == "ActiveState" else u["file"])))
        raise Fail("org.freedesktop.DBus.Error.UnknownObject", path)
    if path != SD_PATH or iface != SD_MANAGER:
        raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s" % (iface, method))
    if method == "GetUnitFileState":
        return v("(s)", (unit_of(bus, args[0])["file"],))
    if method in ("LoadUnit", "GetUnit"):
        unit_of(bus, args[0])
        return v("(o)", (unit_path(args[0]),))
    if method == "StartUnit":
        unit_of(bus, args[0])["active"] = "active"
        return v("(o)", (SD_PATH + "/job/1",))
    if method == "StopUnit":
        unit_of(bus, args[0])["active"] = "inactive"
        return v("(o)", (SD_PATH + "/job/2",))
    if method == "EnableUnitFiles":
        for name in args[0]:
            unit_of(bus, name)["file"] = "enabled"
        return v("(ba(sss))", (False, [("symlink", "/etc/systemd/" + args[0][0], "/usr/lib/systemd/" + args[0][0])]))
    if method == "DisableUnitFiles":
        for name in args[0]:
            unit_of(bus, name)["file"] = "disabled"
        return v("(a(sss))", ([("unlink", "/etc/systemd/" + args[0][0], "")],))
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", method)


def gsd(path, iface, method, args):
    if iface == PROPS and method == "GetAll" and args[0] == GSD:
        return v("(a{sv})", ({"CurrentNetwork": v("s", LAB), "CurrentNetworkName": v("s", "Lab-5G"),
                              "CarrierType": v("s", "802-11-wireless"), "SharingStatus": v("u", 3)},))
    if iface == GSD and method == "ListNetworks":
        names = {LAB: ("Lab-5G", "802-11-wireless"), OFFICE: ("Kantoor", "802-3-ethernet")}
        return v("(a(sss))", ([(u,) + names[u] for u in SHARED.get(args[0], [])],))
    if iface == GSD and method == "EnableService":
        if LAB not in SHARED.setdefault(args[0], []):
            SHARED[args[0]].append(LAB)
        return None
    if iface == GSD and method == "DisableService":
        SHARED[args[0]] = [u for u in SHARED.get(args[0], []) if u != args[1]]
        return None
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s" % (iface, method))


def grd_user(path, iface, method, args):
    if UNITS["session"]["gnome-remote-desktop.service"]["active"] != "active":
        raise Fail("org.freedesktop.DBus.Error.ServiceUnknown", "not running")
    if path == GRD_RDP_PATH and iface == PROPS and method == "GetAll" and args[0] == GRD_RDP:
        return v("(a{sv})", ({"Enabled": v("b", True), "Port": v("i", 3390), "NegotiatePort": v("b", True),
                              "ViewOnly": v("b", True)},))
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s" % (iface, method))


def read_fd(fd):
    data = b""
    while True:
        chunk = os.read(fd, 65536)
        if not chunk:
            break
        data += chunk
    os.close(fd)
    return data.decode("utf-8", "replace")


def grd_conf(conn, path, iface, method, args, message, note):
    if iface == PROPS and method == "GetAll" and args[0] == GRD_CONF_IFACE:
        return v("(a{sv})", ({"Enabled": v("b", CONF["Enabled"]), "Port": v("i", CONF["Port"]),
                              "TlsCert": v("s", CONF["TlsCert"]), "TlsFingerprint": v("s", CONF["TlsFingerprint"]),
                              "TlsKey": v("s", CONF["TlsKey"])},))
    if path != GRD_CONF_PATH or iface != GRD_CONF_IFACE:
        raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s" % (iface, method))
    if method in ("Enable", "Disable"):
        CONF["Enabled"] = method == "Enable"
        emit_later(conn, GRD_CONF_PATH, PROPS, "PropertiesChanged",
                   v("(sa{sv}as)", (GRD_CONF_IFACE, {"Enabled": v("b", CONF["Enabled"])}, [])))
        return None
    if method == "GetCredentials":
        return v("(a{sv})", ({k: v("s", x) for k, x in CONF["credentials"].items()},))
    if method == "SetCredentials":
        CONF["credentials"] = {k: x for k, x in args[0].items()}
        return None
    if method == "ImportCertificate":
        fds = message.get_unix_fd_list()
        if fds is None or fds.get_length() != 2:
            raise Fail("org.freedesktop.DBus.Error.InvalidArgs", "two file descriptors expected")
        cert, key = read_fd(fds.get(args[0][1])), read_fd(fds.get(args[1][1]))
        note["files"] = [args[0][0], cert.splitlines()[0] if cert else "", args[1][0],
                         key.splitlines()[0] if key else ""]
        if "BEGIN CERTIFICATE" not in cert or "PRIVATE KEY" not in key:
            raise Fail("org.gnome.RemoteDesktop.Error.InvalidCertificate", "Invalid certificate")
        body = cert.split("-----BEGIN CERTIFICATE-----")[1].split("-----END CERTIFICATE-----")[0]
        digest = hashlib.sha256(base64.b64decode("".join(body.split()))).hexdigest()
        CONF["TlsCert"] = "/var/lib/gnome-remote-desktop/.local/share/gnome-remote-desktop/certificates/" + args[0][0]
        CONF["TlsKey"] = CONF["TlsCert"][:-len(args[0][0])] + args[1][0]
        CONF["TlsFingerprint"] = ":".join(digest[i:i + 2] for i in range(0, 64, 2))
        return None
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", method)


def emit_later(conn, path, iface, member, params, ms=30):
    GLib.timeout_add(ms, lambda: (conn.emit_signal(None, path, iface, member, params), False)[1])


def packagekit(conn, path, iface, method, args):
    if path == PK_PATH and iface == PK and method == "CreateTransaction":
        TX["n"] += 1
        return v("(o)", ("/%d_fake" % TX["n"],))
    if iface != PK_TX:
        raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s" % (iface, method))
    if method == "SetHints":
        return None
    if method == "Resolve":
        if args[1] == ["openssh-server"] and "ssh.service" not in UNITS["system"]:
            emit_later(conn, path, PK_TX, "Package", v("(uss)", (2, PACKAGE_ID, "secure shell (SSH) server")))
        emit_later(conn, path, PK_TX, "Finished", v("(uu)", (1, 12)), 120)
        return None
    if method == "InstallPackages":
        emit_later(conn, path, PROPS, "PropertiesChanged",
                   v("(sa{sv}as)", (PK_TX, {"Percentage": v("u", 50)}, [])))

        def done():
            UNITS["system"].update({k: dict(x) for k, x in SSH_UNITS.items()})
            conn.emit_signal(None, path, PK_TX, "Finished", v("(uu)", (1, 2400)))
            return False
        GLib.timeout_add(300, done)
        return None
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", method)


def handle(bus, conn, dest, message):
    path, iface, method, body = message.get_path(), message.get_interface(), message.get_member(), message.get_body()
    want = SIGNATURES.get(method)
    got = body.get_type_string() if body is not None else "()"
    if want and got != want:
        raise Fail("org.freedesktop.DBus.Error.InvalidArgs", "%s wants %s, got %s" % (method, want, got))
    if bus == "system" and method in SYSTEM_WRITES and not message.get_flags() & FLAG:
        raise Fail("org.freedesktop.DBus.Error.InteractiveAuthorizationRequired", "Interactive authentication required.")
    args = body.unpack() if body is not None else ()
    note = {}
    if dest == POLKIT:
        return polkit(path, iface, method, args, message, note), note
    if dest == SD:
        if bus == "system":
            systemd_polkit(method, message, note)
        return systemd(bus, path, iface, method, args), note
    if dest == GSD:
        return gsd(path, iface, method, args), note
    if dest == GRD_USER:
        return grd_user(path, iface, method, args), note
    if dest == GRD_CONF:
        return grd_conf(conn, path, iface, method, args, message, note), note
    if dest == PK:
        return packagekit(conn, path, iface, method, args), note
    raise Fail("org.freedesktop.DBus.Error.ServiceUnknown", dest)


def make_filter(bus):
    def filt(conn, message, incoming):
        if not incoming or message.get_message_type() != Gio.DBusMessageType.METHOD_CALL:
            return message
        dest = message.get_destination()
        if dest not in OWNED[bus]:
            return message
        body = message.get_body()
        with LOCK:
            entry = {"bus": bus, "dest": dest, "path": message.get_path(), "iface": message.get_interface(),
                     "method": message.get_member(), "signature": body.get_type_string() if body is not None else "()",
                     "args": body.print_(True) if body is not None else "",
                     "interactive": bool(message.get_flags() & FLAG)}
            try:
                result, note = handle(bus, conn, dest, message)
                entry.update(note)
                reply = Gio.DBusMessage.new_method_reply(message)
                if result is not None:
                    reply.set_body(result)
            except Fail as error:
                entry["error"] = error.name
                reply = Gio.DBusMessage.new_method_error_literal(message, error.name, str(error))
            except Exception as error:  # a bug in the fake: answer, never hang the caller
                entry["error"] = repr(error)
                reply = Gio.DBusMessage.new_method_error_literal(message, "org.freedesktop.DBus.Error.Failed",
                                                                 repr(error))
            if dest == GRD_CONF and entry["method"] == "SetCredentials":
                entry["args"] = "(redacted)"
            CALLS.write(json.dumps(entry) + "\n")
        conn.send_message(reply, Gio.DBusSendMessageFlags.NONE)
        return None
    return filt


OWNED = {"session": [], "system": [SD, GRD_CONF, PK, POLKIT]}
if "--user-systemd-only" in OPTS:
    OWNED = {"session": [SD], "system": []}
elif "--system-only" not in OPTS:
    OWNED["session"] = [GSD, GRD_USER] + ([] if "--no-user-systemd" in OPTS else [SD])
loop = GLib.MainLoop()
waiting = {"n": sum(len(x) for x in OWNED.values())}


def owned(*_):
    waiting["n"] -= 1
    if waiting["n"] == 0:
        print("ready", flush=True)


for bus_name, kind in (("session", Gio.BusType.SESSION), ("system", Gio.BusType.SYSTEM)):
    if not OWNED[bus_name]:
        continue
    conn = Gio.bus_get_sync(kind, None)
    CONNS[bus_name] = conn
    conn.add_filter(make_filter(bus_name))
    for name in OWNED[bus_name]:
        Gio.bus_own_name_on_connection(conn, name, Gio.BusNameOwnerFlags.NONE, owned, lambda *a: loop.quit())
loop.run()
