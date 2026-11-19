"""Batch E's system-bus fake for machine.* (tests/dbus-fakes/run.sh --batch e): hostname1, timedate1 (NTP on and
synchronised; there is no timesync1, amendment 30), locale1 and PackageKit, with the methods and properties
host/src/services/machine/ uses. SetNTP changes the NTP property; PackageKit's CreateTransaction hands out
/org/freedesktop/PackageKit/tx/<n>, Resolve answers a Package signal per name (id "<name>;1:26.04;all;fake") and
Finished, InstallPackages answers Finished. Every method call is appended to argv[1] as JSON: method, args, path and
whether the caller allowed interactive authorization (call_interactive sets it). Template: ../b0/echo.py."""
import json
import sys
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
TX = "org.freedesktop.PackageKit.Transaction"
XML = """<node>
<interface name="org.freedesktop.hostname1">
  <method name="SetPrettyHostname"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
  <method name="SetStaticHostname"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
  <property name="Hostname" type="s" access="read"/><property name="StaticHostname" type="s" access="read"/>
  <property name="PrettyHostname" type="s" access="read"/><property name="Chassis" type="s" access="read"/>
  <property name="HardwareVendor" type="s" access="read"/><property name="HardwareModel" type="s" access="read"/>
</interface>
<interface name="org.freedesktop.timedate1">
  <method name="SetTime"><arg type="x" direction="in"/><arg type="b" direction="in"/><arg type="b" direction="in"/></method>
  <method name="SetTimezone"><arg type="s" direction="in"/><arg type="b" direction="in"/></method>
  <method name="SetNTP"><arg type="b" direction="in"/><arg type="b" direction="in"/></method>
  <method name="ListTimezones"><arg type="as" direction="out"/></method>
  <property name="Timezone" type="s" access="read"/><property name="NTP" type="b" access="read"/>
  <property name="CanNTP" type="b" access="read"/><property name="NTPSynchronized" type="b" access="read"/>
  <property name="LocalRTC" type="b" access="read"/>
</interface>
<interface name="org.freedesktop.locale1">
  <method name="SetLocale"><arg type="as" direction="in"/><arg type="b" direction="in"/></method>
  <property name="Locale" type="as" access="read"/>
</interface>
<interface name="org.freedesktop.PackageKit">
  <method name="CreateTransaction"><arg type="o" direction="out"/></method>
</interface>
<interface name="org.freedesktop.PackageKit.Transaction">
  <method name="SetHints"><arg type="as" direction="in"/></method>
  <method name="Resolve"><arg type="t" direction="in"/><arg type="as" direction="in"/></method>
  <method name="InstallPackages"><arg type="t" direction="in"/><arg type="as" direction="in"/></method>
  <signal name="Package"><arg type="u"/><arg type="s"/><arg type="s"/></signal>
  <signal name="ErrorCode"><arg type="u"/><arg type="s"/></signal>
  <signal name="Finished"><arg type="u"/><arg type="u"/></signal>
</interface>
</node>"""
NODE = Gio.DBusNodeInfo.new_for_xml(XML)
STATE = {"ntp": True, "tx": 0}
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)


def iface(name):
    return next(i for i in NODE.interfaces if i.name == name)


def record(method, params, path, interactive):
    CALLS.write(json.dumps({"method": method, "args": params.print_(True), "path": path,
                            "interactive": interactive}) + "\n")


def later(path, member, sig, values):
    def emit():
        bus.emit_signal(None, path, TX, member, GLib.Variant(sig, values))
        return False
    GLib.timeout_add(50, emit)


def call(conn, sender, path, iface_name, method, params, invocation):
    record(method, params, path, bool(invocation.get_message().get_flags() & FLAG))
    if method == "SetNTP":
        STATE["ntp"] = params.unpack()[0]
        invocation.return_value(None)
    elif method == "ListTimezones":
        invocation.return_value(GLib.Variant("(as)", (["Europe/Amsterdam", "UTC"],)))
    elif method == "CreateTransaction":
        STATE["tx"] += 1
        tx = "/org/freedesktop/PackageKit/tx/%d" % STATE["tx"]
        bus.register_object(tx, iface(TX), call, None, None)
        invocation.return_value(GLib.Variant("(o)", (tx,)))
    elif method == "Resolve":
        invocation.return_value(None)
        for name in params.unpack()[1]:
            later(path, "Package", "(uss)", (2, "%s;1:26.04;all;fake" % name, ""))
        later(path, "Finished", "(uu)", (1, 5))
    elif method == "InstallPackages":
        invocation.return_value(None)
        later(path, "Finished", "(uu)", (1, 50))
    else:
        invocation.return_value(None)


def get_property(conn, sender, path, iface_name, prop):
    values = {
        "Hostname": GLib.Variant("s", "testpc"), "StaticHostname": GLib.Variant("s", "testpc"),
        "PrettyHostname": GLib.Variant("s", "Testpc"), "Chassis": GLib.Variant("s", "desktop"),
        "HardwareVendor": GLib.Variant("s", "Fake"), "HardwareModel": GLib.Variant("s", "Machine"),
        "Timezone": GLib.Variant("s", "Europe/Amsterdam"), "NTP": GLib.Variant("b", STATE["ntp"]),
        "CanNTP": GLib.Variant("b", True), "NTPSynchronized": GLib.Variant("b", True),
        "LocalRTC": GLib.Variant("b", False),
        "Locale": GLib.Variant("as", ["LANG=en_US.UTF-8"]),
    }
    return values[prop]


for path, name in [("/org/freedesktop/hostname1", "org.freedesktop.hostname1"),
                   ("/org/freedesktop/timedate1", "org.freedesktop.timedate1"),
                   ("/org/freedesktop/locale1", "org.freedesktop.locale1"),
                   ("/org/freedesktop/PackageKit", "org.freedesktop.PackageKit")]:
    bus.register_object(path, iface(name), call, get_property, None)

NAMES = ("org.freedesktop.hostname1", "org.freedesktop.timedate1", "org.freedesktop.locale1",
         "org.freedesktop.PackageKit")
owned = []
loop = GLib.MainLoop()


def acquired(*_args):
    owned.append(1)
    if len(owned) == len(NAMES):
        print("ready", flush=True)


for well_known in NAMES:
    Gio.bus_own_name_on_connection(bus, well_known, Gio.BusNameOwnerFlags.NONE, acquired, lambda *a: loop.quit())
loop.run()
