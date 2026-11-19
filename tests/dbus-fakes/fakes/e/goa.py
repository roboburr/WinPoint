"""Batch E's session-bus fake (tests/dbus-fakes/run.sh --batch e): goa-daemon (org.gnome.OnlineAccounts) with one
Google account whose mail is on and calendar off, as host/src/services/goa.rs reads and writes it (the ObjectManager,
the Account's Remove and EnsureCredentials, its *Disabled properties). Every call and property write is appended to
argv[1] as JSON (method, args, path, interactive). Template: ../b0/echo.py."""
import json
import sys
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
ACCOUNT = "/org/gnome/OnlineAccounts/Accounts/account_1_0"
XML = """<node>
<interface name="org.freedesktop.DBus.ObjectManager">
  <method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>
</interface>
<interface name="org.gnome.OnlineAccounts.Account">
  <method name="Remove"/>
  <method name="EnsureCredentials"><arg type="i" direction="out"/></method>
  <property name="MailDisabled" type="b" access="readwrite"/>
  <property name="CalendarDisabled" type="b" access="readwrite"/>
</interface>
</node>"""
NODE = Gio.DBusNodeInfo.new_for_xml(XML)
STATE = {"MailDisabled": False, "CalendarDisabled": True}


def objects():
    account = {"Id": GLib.Variant("s", "account_1_0"), "ProviderType": GLib.Variant("s", "google"),
               "ProviderName": GLib.Variant("s", "Google"), "Identity": GLib.Variant("s", "anna@example.org"),
               "PresentationIdentity": GLib.Variant("s", "anna@example.org"),
               "AttentionNeeded": GLib.Variant("b", False)}
    for prop, value in STATE.items():
        account[prop] = GLib.Variant("b", value)
    ifaces = {"org.gnome.OnlineAccounts.Account": account}
    if not STATE["MailDisabled"]:
        ifaces["org.gnome.OnlineAccounts.Mail"] = {}
    if not STATE["CalendarDisabled"]:
        ifaces["org.gnome.OnlineAccounts.Calendar"] = {}
    return GLib.Variant("(a{oa{sa{sv}}})", ({ACCOUNT: ifaces},))


def call(conn, sender, path, iface, method, params, invocation):
    CALLS.write(json.dumps({"method": method, "args": params.print_(True), "path": path,
                            "interactive": bool(invocation.get_message().get_flags() & FLAG)}) + "\n")
    if method == "GetManagedObjects":
        invocation.return_value(objects())
    elif method == "EnsureCredentials":
        invocation.return_value(GLib.Variant("(i)", (3600,)))
    else:
        invocation.return_value(None)


def get_property(conn, sender, path, iface, prop):
    return GLib.Variant("b", STATE[prop])


def set_property(conn, sender, path, iface, prop, value):
    CALLS.write(json.dumps({"method": "Set", "args": "%s = %s" % (prop, value.print_(True)), "path": path,
                            "interactive": False}) + "\n")
    STATE[prop] = value.unpack()
    return True


def iface(name):
    return next(i for i in NODE.interfaces if i.name == name)


bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
bus.register_object("/org/gnome/OnlineAccounts", iface("org.freedesktop.DBus.ObjectManager"), call, None, None)
bus.register_object(ACCOUNT, iface("org.gnome.OnlineAccounts.Account"), call, get_property, set_property)
loop = GLib.MainLoop()
Gio.bus_own_name_on_connection(bus, "org.gnome.OnlineAccounts", Gio.BusNameOwnerFlags.NONE,
                               lambda *a: print("ready", flush=True), lambda *a: loop.quit())
loop.run()
