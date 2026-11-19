"""Batch E's system-bus fake (tests/dbus-fakes/run.sh --batch e): AccountsService (org.freedesktop.Accounts) and
fprintd (net.reactivated.Fprint), with the methods and properties host/src/services/users.rs uses. The own uid is
"ik" (an administrator), uid + 1 is "anna". EnrollStart answers with two EnrollStatus signals (a stage, then done).
Every method call and property write is appended to argv[1] as JSON: method, args, path and whether the caller
allowed interactive authorization (call_interactive sets it). Template: ../b0/echo.py."""
import json
import os
import sys
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
ME = os.getuid()
ANNA = ME + 1
XML = """<node>
<interface name="org.freedesktop.Accounts">
  <method name="FindUserById"><arg type="x" direction="in"/><arg type="o" direction="out"/></method>
  <method name="ListCachedUsers"><arg type="ao" direction="out"/></method>
  <method name="CreateUser"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="i" direction="in"/>
    <arg type="o" direction="out"/></method>
  <method name="DeleteUser"><arg type="x" direction="in"/><arg type="b" direction="in"/></method>
</interface>
<interface name="org.freedesktop.Accounts.User">
  <method name="SetRealName"><arg type="s" direction="in"/></method>
  <method name="SetIconFile"><arg type="s" direction="in"/></method>
  <method name="SetLanguage"><arg type="s" direction="in"/></method>
  <method name="SetFormatsLocale"><arg type="s" direction="in"/></method>
  <method name="SetAccountType"><arg type="i" direction="in"/></method>
  <method name="SetAutomaticLogin"><arg type="b" direction="in"/></method>
  <method name="SetPassword"><arg type="s" direction="in"/><arg type="s" direction="in"/></method>
  <method name="SetPasswordMode"><arg type="i" direction="in"/></method>
  <signal name="Changed"/>
  <property name="Uid" type="t" access="read"/><property name="UserName" type="s" access="read"/>
  <property name="RealName" type="s" access="read"/><property name="AccountType" type="i" access="read"/>
  <property name="IconFile" type="s" access="read"/><property name="AutomaticLogin" type="b" access="read"/>
  <property name="PasswordMode" type="i" access="read"/><property name="Language" type="s" access="read"/>
  <property name="FormatsLocale" type="s" access="read"/>
  <property name="Locked" type="b" access="read"/><property name="LocalAccount" type="b" access="read"/>
  <property name="SystemAccount" type="b" access="read"/>
</interface>
<interface name="net.reactivated.Fprint.Manager">
  <method name="GetDefaultDevice"><arg type="o" direction="out"/></method>
</interface>
<interface name="net.reactivated.Fprint.Device">
  <method name="Claim"><arg type="s" direction="in"/></method>
  <method name="Release"/>
  <method name="ListEnrolledFingers"><arg type="s" direction="in"/><arg type="as" direction="out"/></method>
  <method name="EnrollStart"><arg type="s" direction="in"/></method>
  <method name="EnrollStop"/>
  <method name="DeleteEnrolledFingers2"/>
  <signal name="EnrollStatus"><arg type="s"/><arg type="b"/></signal>
  <property name="name" type="s" access="read"/><property name="num-enroll-stages" type="i" access="read"/>
  <property name="scan-type" type="s" access="read"/>
</interface>
</node>"""
NODE = Gio.DBusNodeInfo.new_for_xml(XML)
DEVICE = "/net/reactivated/Fprint/Device/0"
REAL = {ME: "Ik Zelf", ANNA: "Anna de Vries"}


def user_path(uid):
    return "/org/freedesktop/Accounts/User%d" % uid


def record(method, params, path, interactive):
    CALLS.write(json.dumps({"method": method, "args": params.print_(True), "path": path,
                            "interactive": interactive}) + "\n")


def call(conn, sender, path, iface, method, params, invocation):
    record(method, params, path, bool(invocation.get_message().get_flags() & FLAG))
    if method == "FindUserById":
        uid = params.unpack()[0]
        if uid not in (ME, ANNA):
            invocation.return_dbus_error("org.freedesktop.Accounts.Error.Failed", "Failed to look up user with uid %d." % uid)
            return
        invocation.return_value(GLib.Variant("(o)", (user_path(uid),)))
    elif method == "ListCachedUsers":
        invocation.return_value(GLib.Variant("(ao)", ([user_path(ANNA)],)))
    elif method == "CreateUser":
        invocation.return_value(GLib.Variant("(o)", (user_path(ANNA),)))
    elif method == "GetDefaultDevice":
        invocation.return_value(GLib.Variant("(o)", (DEVICE,)))
    elif method == "ListEnrolledFingers":
        invocation.return_value(GLib.Variant("(as)", ([],)))
    elif method == "SetRealName":
        REAL[int(path.rsplit("User", 1)[1])] = params.unpack()[0]
        invocation.return_value(None)
        conn.emit_signal(None, path, "org.freedesktop.Accounts.User", "Changed", None)
    elif method == "EnrollStart":
        invocation.return_value(None)

        def status(result, done):
            conn.emit_signal(None, DEVICE, "net.reactivated.Fprint.Device", "EnrollStatus",
                             GLib.Variant("(sb)", (result, done)))
            return False
        GLib.timeout_add(100, status, "enroll-stage-passed", False)
        GLib.timeout_add(200, status, "enroll-completed", True)
    else:
        invocation.return_value(None)


def get_property(conn, sender, path, iface, prop):
    if iface == "net.reactivated.Fprint.Device":
        return {"name": GLib.Variant("s", "Fake Reader"), "num-enroll-stages": GLib.Variant("i", 2),
                "scan-type": GLib.Variant("s", "press")}[prop]
    uid = int(path.rsplit("User", 1)[1])
    values = {"Uid": GLib.Variant("t", uid), "UserName": GLib.Variant("s", "ik" if uid == ME else "anna"),
              "RealName": GLib.Variant("s", REAL[uid]), "AccountType": GLib.Variant("i", 1 if uid == ME else 0),
              "IconFile": GLib.Variant("s", ""), "AutomaticLogin": GLib.Variant("b", False),
              "PasswordMode": GLib.Variant("i", 0), "Language": GLib.Variant("s", "nl_NL.UTF-8"),
              "FormatsLocale": GLib.Variant("s", ""),
              "Locked": GLib.Variant("b", False), "LocalAccount": GLib.Variant("b", True),
              "SystemAccount": GLib.Variant("b", False)}
    return values[prop]


def iface(name):
    return next(i for i in NODE.interfaces if i.name == name)


bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
for path, name in [("/org/freedesktop/Accounts", "org.freedesktop.Accounts"),
                   (user_path(ME), "org.freedesktop.Accounts.User"),
                   (user_path(ANNA), "org.freedesktop.Accounts.User"),
                   ("/net/reactivated/Fprint/Manager", "net.reactivated.Fprint.Manager"),
                   (DEVICE, "net.reactivated.Fprint.Device")]:
    bus.register_object(path, iface(name), call, get_property, None)

owned = []
loop = GLib.MainLoop()


def acquired(*_args):
    owned.append(1)
    if len(owned) == 2:
        print("ready", flush=True)


for well_known in ("org.freedesktop.Accounts", "net.reactivated.Fprint"):
    Gio.bus_own_name_on_connection(bus, well_known, Gio.BusNameOwnerFlags.NONE, acquired, lambda *a: loop.quit())
loop.run()
