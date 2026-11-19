"""Batch C's ModemManager fake (tests/dbus-fakes/run.sh --batch c; template ../b0/echo.py), on the private system
bus: one modem (/org/freedesktop/ModemManager1/Modem/0, a Quectel with two SIM slots, the SIM in slot 1 locked with
its PIN, 3 tries left, KPN's 204 08) and its SIM.

As networkmanager.py: no object is registered, a message filter answers every call to org.freedesktop.ModemManager1
(ObjectManager GetManagedObjects, Properties Get/GetAll, the Modem, Modem3gpp and Sim methods) and appends it to the
file in argv[1] as {bus, path, iface, method, signature, args (GVariant text), interactive}; a call with another
in-signature than ModemManager's is refused with InvalidArgs. SendPin with the right PIN (1234) unlocks the SIM and
enables the modem; a wrong one costs a try (a third wrong one locks it with the PUK); EnablePin, ChangePin,
SetCurrentModes, Register and SetPrimarySimSlot change the state later reads see."""
import json
import sys
import threading
import warnings

warnings.simplefilter("ignore", DeprecationWarning)
from gi.repository import Gio, GLib  # noqa: E402

CALLS = open(sys.argv[1], "a", buffering=1)
LOCK = threading.Lock()
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
MM = "org.freedesktop.ModemManager1"
ROOT = "/org/freedesktop/ModemManager1"
MODEM = ROOT + "/Modem/0"
SIM = ROOT + "/SIM/0"
STATE = {"pin": "1234", "puk": "12345678", "tries": 3}


def v(t, x):
    return GLib.Variant(t, x)


OBJECTS = {
    MODEM: {
        MM + ".Modem": {
            "Sim": v("o", SIM), "SimSlots": v("ao", [SIM, "/"]), "PrimarySimSlot": v("u", 1),
            "Manufacturer": v("s", "Quectel"), "Model": v("s", "EM05-G"), "Revision": v("s", "EM05GFAR07A07M1G"),
            "HardwareRevision": v("s", "10000"), "DeviceIdentifier": v("s", "7f1c0b5e2a"),
            "Device": v("s", "/sys/devices/pci0000:00/0000:00:14.0/usb3/3-6"), "PrimaryPort": v("s", "cdc-wdm0"),
            "EquipmentIdentifier": v("s", "861536030196001"), "UnlockRequired": v("u", 2),
            "UnlockRetries": v("a{uu}", {2: 3, 4: 10}), "State": v("i", 2), "StateFailedReason": v("u", 0),
            "AccessTechnologies": v("u", 0), "SignalQuality": v("(ub)", (0, False)),
            "OwnNumbers": v("as", ["+31612345678"]), "PowerState": v("u", 3),
            "SupportedModes": v("a(uu)", [(0x2 | 0x4 | 0x8, 0x8), (0x2 | 0x4 | 0x8, 0x4), (0x4 | 0x8, 0x8),
                                          (0x8, 0)]),
            "CurrentModes": v("(uu)", (0x2 | 0x4 | 0x8, 0x8)),
        },
        MM + ".Modem.Modem3gpp": {
            "Imei": v("s", "861536030196001"), "RegistrationState": v("u", 0), "OperatorCode": v("s", ""),
            "OperatorName": v("s", ""), "EnabledFacilityLocks": v("u", 1),
        },
    },
    SIM: {
        MM + ".Sim": {
            "Active": v("b", True), "SimIdentifier": v("s", "8931440000000000017"), "Imsi": v("s", "204080000000001"),
            "Eid": v("s", ""), "OperatorIdentifier": v("s", "20408"), "OperatorName": v("s", "KPN"),
            "SimType": v("u", 1),
        },
    },
}

SIGNATURES = {
    "GetManagedObjects": "()", "SendPin": "(s)", "SendPuk": "(ss)", "EnablePin": "(sb)", "ChangePin": "(ss)",
    "SetCurrentModes": "((uu))", "Register": "(s)", "Scan": "()", "SetPrimarySimSlot": "(u)",
    "Get": "(ss)", "GetAll": "(s)",
}


class Fail(Exception):
    def __init__(self, name, text):
        super().__init__(text)
        self.name = name


def modem():
    return OBJECTS[MODEM][MM + ".Modem"]


def unlocked():
    modem()["UnlockRequired"] = v("u", 1)
    modem()["State"] = v("i", 8)
    modem()["AccessTechnologies"] = v("u", 1 << 14)
    modem()["SignalQuality"] = v("(ub)", (72, True))
    g = OBJECTS[MODEM][MM + ".Modem.Modem3gpp"]
    g["RegistrationState"] = v("u", 1)
    g["OperatorCode"] = v("s", "20408")
    g["OperatorName"] = v("s", "KPN")


def wrong_try():
    STATE["tries"] -= 1
    if STATE["tries"] <= 0:
        modem()["UnlockRequired"] = v("u", 4)
    modem()["UnlockRetries"] = v("a{uu}", {2: max(STATE["tries"], 0), 4: 10})
    raise Fail(MM + ".Error.MobileEquipment.IncorrectPassword", "Incorrect password")


def handle(path, iface, method, body):
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
            return GLib.Variant.new_tuple(GLib.Variant("v", props[args[1]]))
    if path == ROOT and iface == "org.freedesktop.DBus.ObjectManager" and method == "GetManagedObjects":
        return v("(a{oa{sa{sv}}})", (OBJECTS,))
    if path == SIM and iface == MM + ".Sim":
        if method == "SendPin":
            if args[0] != STATE["pin"]:
                wrong_try()
            unlocked()
            return None
        if method == "SendPuk":
            if args[0] != STATE["puk"]:
                raise Fail(MM + ".Error.MobileEquipment.IncorrectPassword", "Incorrect PUK")
            STATE["pin"] = args[1]
            unlocked()
            return None
        if method == "EnablePin":
            if args[0] != STATE["pin"]:
                wrong_try()
            OBJECTS[MODEM][MM + ".Modem.Modem3gpp"]["EnabledFacilityLocks"] = v("u", 1 if args[1] else 0)
            return None
        if method == "ChangePin":
            if args[0] != STATE["pin"]:
                wrong_try()
            STATE["pin"] = args[1]
            return None
    if path == MODEM and iface == MM + ".Modem":
        if method == "SetCurrentModes":
            modem()["CurrentModes"] = v("(uu)", args[0])
            return None
        if method == "SetPrimarySimSlot":
            modem()["PrimarySimSlot"] = v("u", args[0])
            return None
    if path == MODEM and iface == MM + ".Modem.Modem3gpp":
        if method == "Scan":
            return v("(aa{sv})", ([
                {"status": v("u", 1), "operator-long": v("s", "Vodafone NL"), "operator-short": v("s", "voda NL"),
                 "operator-code": v("s", "20404"), "access-technology": v("u", 1 << 14)},
                {"status": v("u", 2), "operator-long": v("s", "KPN"), "operator-short": v("s", "KPN"),
                 "operator-code": v("s", "20408"), "access-technology": v("u", 1 << 15)},
                {"status": v("u", 3), "operator-long": v("s", "Odido"), "operator-short": v("s", "Odido"),
                 "operator-code": v("s", "20416"), "access-technology": v("u", 1 << 5)},
            ],))
        if method == "Register":
            g = OBJECTS[MODEM][MM + ".Modem.Modem3gpp"]
            g["OperatorCode"] = v("s", args[0] or "20408")
            return None
    raise Fail("org.freedesktop.DBus.Error.UnknownMethod", "%s.%s at %s" % (iface, method, path))


def filt(conn, message, incoming):
    if not incoming or message.get_message_type() != Gio.DBusMessageType.METHOD_CALL:
        return message
    if message.get_destination() not in (MM, conn.get_unique_name()):
        return message
    path, iface, method, body = message.get_path(), message.get_interface(), message.get_member(), message.get_body()
    with LOCK:
        CALLS.write(json.dumps({"bus": "system", "path": path, "iface": iface, "method": method,
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


loop = GLib.MainLoop()
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
bus.add_filter(filt)
Gio.bus_own_name_on_connection(bus, MM, Gio.BusNameOwnerFlags.NONE, lambda *a: print("ready", flush=True),
                               lambda *a: loop.quit())
loop.run()
