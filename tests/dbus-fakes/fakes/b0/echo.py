"""B0's D-Bus fake and the template of every batch's (tests/dbus-fakes/run.sh): io.github.win11shell.Echo on the private
system and session buses. Echo(s) answers its argument; Deny fails as polkit's NotAuthorized. Every call is appended to
the file in argv[1] with whether the caller allowed interactive authorization (call_interactive sets it)."""
import json
import sys
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
XML = """<node><interface name="io.github.win11shell.Echo">
<method name="Echo"><arg type="s" direction="in"/><arg type="s" direction="out"/></method>
<method name="Deny"/></interface></node>"""
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION


def call(conn, sender, path, iface, method, params, invocation):
    interactive = bool(invocation.get_message().get_flags() & FLAG)
    CALLS.write(json.dumps({"method": method, "args": params.print_(True), "interactive": interactive}) + "\n")
    if method == "Echo":
        invocation.return_value(params)
    else:
        invocation.return_dbus_error("org.freedesktop.PolicyKit1.Error.NotAuthorized", "not authorized (fake)")


owned = []
loop = GLib.MainLoop()
for kind in (Gio.BusType.SYSTEM, Gio.BusType.SESSION):
    bus = Gio.bus_get_sync(kind, None)
    bus.register_object("/io/github/win11shell/Echo", Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0], call, None, None)

    def acquired(*_args):
        owned.append(1)
        if len(owned) == 2:
            print("ready", flush=True)

    Gio.bus_own_name_on_connection(bus, "io.github.win11shell.Echo", Gio.BusNameOwnerFlags.NONE, acquired,
                                   lambda *a: loop.quit())
loop.run()
