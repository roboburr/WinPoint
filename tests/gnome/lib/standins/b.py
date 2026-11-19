"""Batch B's D-Bus stand-ins for its settings-parity scenarios (notes/claude/settings-parity.md §7): `Spec`s for
`lib.standins.start(h, spec, tag)`, and `start_objects` for an ObjectManager daemon. Only batch B edits this file.

* ``bolt``: org.freedesktop.bolt on the private system bus with one new (pending) Thunderbolt dock, the Manager's
  AuthMode writable, EnrollDevice / ForgetDevice / DeviceByUid and the device's Authorize: what the USB page sends
  (devices.thunderbolt.*).
* ``upower``: org.freedesktop.UPower with the computer's own battery (never listed) and a wireless mouse with its own
  receiver (a card of its own on Apparaten: devices.batteries).
* ``cupspkhelper``: org.opensuse.CupsPkHelper.Mechanism with every method the printer pages call (each answers the
  empty error string, cph's success) and DevicesGet with a USB printer that needs a driver and a driverless IPP
  printer (tests/gnome/lib/fake_ipp.py's own /ipp/print): what printers.* writes go through.
* ``colord``: org.freedesktop.ColorManager with one display (an EDID profile as its default and a calibrated profile)
  and no sensor: devices.color.*; ``colord-sensor`` is the same with a colour sensor plugged in (Kalibreren…).
* ``bluez``: org.bluez as an ObjectManager daemon (``start_objects``, below: the builder of ``lib.standins`` answers
  only plain values, and BlueZ's GetManagedObjects is ``a{oa{sa{sv}}}``): adapter hci0 (powered, alias WinPoint-pc)
  with a paired and connected mouse (BlueZ battery 70 %), connected headphones and a paired phone. Properties.Set
  of Powered and Discoverable, Connect, Disconnect and RemoveDevice change the objects and send PropertiesChanged
  or InterfacesRemoved, as bluetoothd does: system.bluetooth.* and devices.bluetooth.*.
* ``gsd-rfkill``: gsd's org.gnome.SettingsDaemon.Rfkill on the session bus with a Bluetooth rfkill and flight mode
  off, BluetoothAirplaneMode writable (amendment 57's switch); ``gsd-rfkill-flight`` with flight mode on.
"""
from __future__ import annotations

import select
import subprocess
import time

from . import Spec, StandIn

DOCK = '/org/freedesktop/bolt/devices/c6030000_dock'
DOCK_UID = 'c6030000-0080-7f18-23e2-dock'

BOLT = Spec('system', 'org.freedesktop.bolt', {
    '/org/freedesktop/bolt': {'org.freedesktop.bolt1.Manager': {
        'props': {'Version': ['u', 1], 'Probing': ['b', False], 'DefaultPolicy': ['s', 'auto'],
                  'SecurityLevel': ['s', 'user'], 'AuthMode': ['s', 'enabled']},
        'writable': ['AuthMode'],
        'methods': {
            'ListDevices': {'out': 'ao', 'value': [[DOCK]]},
            'ListDomains': {'out': 'ao', 'value': [[]]},
            'DeviceByUid': {'in': 's', 'out': 'o', 'value': [DOCK]},
            'EnrollDevice': {'in': 'sss', 'out': 'o', 'value': [DOCK]},
            'ForgetDevice': {'in': 's'},
        },
    }},
    DOCK: {'org.freedesktop.bolt1.Device': {
        'props': {'Uid': ['s', DOCK_UID], 'Name': ['s', 'Thunderbolt Dock'], 'Vendor': ['s', 'Lenovo'],
                  'Type': ['s', 'peripheral'], 'Status': ['s', 'connected'], 'Stored': ['b', False],
                  'Policy': ['s', 'default'], 'Key': ['s', 'missing'], 'Parent': ['s', ''],
                  'Domain': ['s', 'domain0'], 'Generation': ['u', 3], 'Label': ['s', ''],
                  'ConnectTime': ['t', 1763280000], 'AuthorizeTime': ['t', 0], 'StoreTime': ['t', 0]},
        'methods': {'Authorize': {'in': 's'}},
    }},
})

UPOWER = Spec('system', 'org.freedesktop.UPower', {
    '/org/freedesktop/UPower': {'org.freedesktop.UPower': {
        'methods': {'EnumerateDevices': {'out': 'ao', 'value': [[
            '/org/freedesktop/UPower/devices/battery_BAT0', '/org/freedesktop/UPower/devices/mouse_hidpp_battery_0']]}},
    }},
    '/org/freedesktop/UPower/devices/battery_BAT0': {'org.freedesktop.UPower.Device': {
        'props': {'Type': ['u', 2], 'PowerSupply': ['b', True], 'Percentage': ['d', 80.0], 'IsPresent': ['b', True],
                  'State': ['u', 2], 'Model': ['s', 'BAT0'], 'NativePath': ['s', 'BAT0']},
    }},
    '/org/freedesktop/UPower/devices/mouse_hidpp_battery_0': {'org.freedesktop.UPower.Device': {
        'props': {'Type': ['u', 5], 'PowerSupply': ['b', False], 'Percentage': ['d', 55.0], 'IsPresent': ['b', True],
                  'State': ['u', 2], 'Model': ['s', 'MX Master 3'], 'Vendor': ['s', 'Logitech'],
                  'NativePath': ['s', 'hidpp_battery_0'], 'Serial': ['s', '4031-ab-cd'], 'BatteryLevel': ['u', 1]},
    }},
})


def _cph(*methods: tuple[str, str]) -> dict:
    return {name: {'in': sig, 'out': 's', 'value': ['']} for name, sig in methods}


CUPSPKHELPER = Spec('system', 'org.opensuse.CupsPkHelper.Mechanism', {
    '/': {'org.opensuse.CupsPkHelper.Mechanism': {'methods': {
        **_cph(('PrinterAdd', 'sssss'), ('PrinterAddWithPpdFile', 'sssss'), ('PrinterSetEnabled', 'sb'),
               ('PrinterSetAcceptJobs', 'sbs'), ('PrinterAddOptionDefault', 'ssas'), ('PrinterSetDefault', 's'),
               ('PrinterDelete', 's'), ('PrinterRename', 'ss'), ('PrinterSetLocation', 'ss'),
               ('JobCancelPurge', 'ib'), ('JobSetHoldUntil', 'is')),
        'DevicesGet': {'in': 'iiasas', 'out': 'sa{ss}', 'value': ['', {
            'device-class:0': 'direct', 'device-uri:0': 'usb://HP/DeskJet%202700%20series?serial=CN12345',
            'device-info:0': 'HP DeskJet 2700 series', 'device-make-and-model:0': 'HP DeskJet 2700 series',
            'device-id:0': 'MFG:HP;MDL:DeskJet 2700 series;', 'device-location:0': '',
            'device-class:1': 'network', 'device-uri:1': 'ipp://127.0.0.1:8631/ipp/print',
            'device-info:1': 'Fake LaserJet', 'device-make-and-model:1': 'HP Fake LaserJet',
            'device-id:1': 'MFG:HP;MDL:Fake LaserJet;', 'device-location:1': 'Lab',
        }]},
    }}},
})

CD = '/org/freedesktop/ColorManager'
CD_DISPLAY = f'{CD}/devices/xrandr_Dell_U2720Q_robin_1000'
CD_EDID = f'{CD}/profiles/icc_edid_dell'
CD_CALIB = f'{CD}/profiles/icc_calibrated_dell'
# profile titles are the system's data, not UI text: language-neutral ones, so the en run's Dutch-word check only
# sees WinPoint's own strings
CD_CALIB_TITLE = 'Dell U2720Q D65'
CD_SENSOR = f'{CD}/sensors/colormunki_0'


def _profile(pid: str, title: str, source: str) -> dict:
    return {'props': {
        'ProfileId': ['s', pid], 'Title': ['s', title], 'Kind': ['s', 'display-device'], 'Colorspace': ['s', 'rgb'],
        'Filename': ['s', f'/home/u/.local/share/icc/{pid}.icc'], 'Created': ['x', 1763280000],
        'IsSystemWide': ['b', False], 'Warnings': ['as', []], 'HasVcgt': ['b', source == 'calib'],
        'Scope': ['s', 'temp'], 'Format': ['s', 'ColorSpace..'], 'Qualifier': ['s', ''],
        'Metadata': ['a{ss}', {'DATA_source': source}]},
        'methods': {'InstallSystemWide': {}}}


def _colord(sensor: bool) -> Spec:
    objects = {
        CD: {'org.freedesktop.ColorManager': {'methods': {
            'GetDevices': {'out': 'ao', 'value': [[CD_DISPLAY]]},
            'GetProfiles': {'out': 'ao', 'value': [[CD_EDID, CD_CALIB]]},
            'GetSensors': {'out': 'ao', 'value': [[CD_SENSOR] if sensor else []]},
        }}},
        CD_DISPLAY: {'org.freedesktop.ColorManager.Device': {
            'props': {'DeviceId': ['s', 'xrandr-Dell-U2720Q'], 'Kind': ['s', 'display'], 'Model': ['s', 'U2720Q'],
                      'Vendor': ['s', 'Dell'], 'Serial': ['s', '1234'], 'Colorspace': ['s', 'rgb'],
                      'Mode': ['s', 'physical'], 'Scope': ['s', 'temp'], 'Enabled': ['b', True],
                      'Embedded': ['b', False], 'Created': ['t', 1763280000], 'Profiles': ['ao', [CD_EDID, CD_CALIB]],
                      'ProfilingInhibitors': ['as', []], 'Metadata': ['a{ss}', {}]},
            'methods': {'SetEnabled': {'in': 'b'}, 'MakeProfileDefault': {'in': 'o'}, 'AddProfile': {'in': 'so'},
                        'RemoveProfile': {'in': 'o'}},
        }},
        CD_EDID: {'org.freedesktop.ColorManager.Profile': _profile('icc-edid-dell', 'Dell U2720Q', 'edid')},
        CD_CALIB: {'org.freedesktop.ColorManager.Profile': _profile('icc-calib-dell', CD_CALIB_TITLE, 'calib')},
    }
    if sensor:
        objects[CD_SENSOR] = {'org.freedesktop.ColorManager.Sensor': {'props': {
            'SensorId': ['s', 'colormunki-0'], 'Kind': ['s', 'colormunki'], 'State': ['s', 'idle'],
            'Mode': ['s', 'unknown'], 'Serial': ['s', 'CM-0001'], 'Model': ['s', 'ColorMunki Display'],
            'Vendor': ['s', 'X-Rite'], 'Native': ['b', True], 'Embedded': ['b', False], 'Locked': ['b', False],
            'Capabilities': ['as', ['lcd', 'led', 'crt']]}}}
    return Spec('system', 'org.freedesktop.ColorManager', objects)


COLORD = _colord(False)
COLORD_SENSOR = _colord(True)

# ---- gsd's Rfkill (amendment 57: GNOME Settings switches Bluetooth through BluetoothAirplaneMode)

RFKILL_PATH = '/org/gnome/SettingsDaemon/Rfkill'


def _rfkill(flight: bool) -> Spec:
    return Spec('session', 'org.gnome.SettingsDaemon.Rfkill', {RFKILL_PATH: {'org.gnome.SettingsDaemon.Rfkill': {
        'props': {'AirplaneMode': ['b', flight], 'HardwareAirplaneMode': ['b', False], 'HasAirplaneMode': ['b', True],
                  'ShouldShowAirplaneMode': ['b', True], 'BluetoothAirplaneMode': ['b', flight],
                  'BluetoothHardwareAirplaneMode': ['b', False], 'BluetoothHasAirplaneMode': ['b', True]},
        'writable': ['AirplaneMode', 'BluetoothAirplaneMode'],
    }}})


GSD_RFKILL = _rfkill(False)
GSD_RFKILL_FLIGHT = _rfkill(True)

# ---- BlueZ (an ObjectManager daemon: start_objects below)

HCI = '/org/bluez/hci0'
MOUSE, HEADPHONES, PHONE = 'F1:22:33:44:55:01', 'AC:80:0A:12:34:56', 'C4:DD:57:AA:BB:02'
DEV = {addr: f'{HCI}/dev_{addr.replace(":", "_")}' for addr in (MOUSE, HEADPHONES, PHONE)}
NAMES = {MOUSE: 'MX Anywhere 3S', HEADPHONES: 'WH-1000XM5', PHONE: 'Pixel 8'}
ALIAS = 'WinPoint-pc'


def _device(addr: str, icon: str, klass: int, connected: bool, uuids: list[str]) -> dict:
    return {'props': {
        'Address': ['s', addr], 'AddressType': ['s', 'public'], 'Name': ['s', NAMES[addr]], 'Alias': ['s', NAMES[addr]],
        'Icon': ['s', icon], 'Class': ['u', klass], 'Paired': ['b', True], 'Bonded': ['b', True], 'Trusted': ['b', True],
        'Blocked': ['b', False], 'Connected': ['b', connected], 'LegacyPairing': ['b', False],
        'ServicesResolved': ['b', connected], 'Adapter': ['o', HCI], 'UUIDs': ['as', uuids], 'Modalias': ['s', '']},
        'methods': {'Connect': {'effects': [['set', 'Connected', True]]},
                    'Disconnect': {'effects': [['set', 'Connected', False]]},
                    'Pair': {}, 'CancelPairing': {}, 'ConnectProfile': {'in': ['s']}, 'DisconnectProfile': {'in': ['s']}}}


BLUEZ = Spec('system', 'org.bluez', {
    '/org/bluez': {'org.bluez.AgentManager1': {'methods': {
        'RegisterAgent': {'in': ['o', 's']}, 'UnregisterAgent': {'in': ['o']}, 'RequestDefaultAgent': {'in': ['o']}}}},
    HCI: {'org.bluez.Adapter1': {
        'props': {'Address': ['s', 'C0:FF:EE:00:00:01'], 'AddressType': ['s', 'public'], 'Name': ['s', 'winpoint-test'],
                  'Alias': ['s', ALIAS], 'Class': ['u', 0x7C010C], 'Powered': ['b', True],
                  'PowerState': ['s', 'on'], 'Discoverable': ['b', False], 'DiscoverableTimeout': ['u', 180],
                  'Pairable': ['b', True], 'PairableTimeout': ['u', 0], 'Discovering': ['b', False],
                  'UUIDs': ['as', []], 'Modalias': ['s', 'usb:v1D6Bp0246d0548'],
                  'Roles': ['as', ['central', 'peripheral']]},
        'writable': ['Powered', 'Discoverable', 'Alias', 'Pairable', 'DiscoverableTimeout'],
        'follow': {'Powered': {'PowerState': {'true': 'on', 'false': 'off'}}},
        'methods': {'StartDiscovery': {'effects': [['set', 'Discovering', True]]},
                    'StopDiscovery': {'effects': [['set', 'Discovering', False]]},
                    'SetDiscoveryFilter': {'in': ['a{sv}']}, 'GetDiscoveryFilters': {'out': ['as'], 'value': [[]]},
                    'RemoveDevice': {'in': ['o'], 'effects': [['remove', 0]]}},
    }},
    DEV[MOUSE]: {'org.bluez.Device1': _device(MOUSE, 'input-mouse', 0x002580, True,
                                              ['00001124-0000-1000-8000-00805f9b34fb']),
                 'org.bluez.Battery1': {'props': {'Percentage': ['y', 70], 'Source': ['s', 'HID']}}},
    DEV[HEADPHONES]: {'org.bluez.Device1': _device(HEADPHONES, 'audio-headphones', 0x240418, True,
                                                   ['0000110b-0000-1000-8000-00805f9b34fb'])},
    DEV[PHONE]: {'org.bluez.Device1': _device(PHONE, 'phone', 0x5A020C, False,
                                              ['00001105-0000-1000-8000-00805f9b34fb'])},
})

# The server of start_objects: lib.standins' SERVER with the ObjectManager on "/", every property a variant,
# PropertiesChanged on each change and method effects ('set' a property of the called object, 'remove' the object
# an in-argument names, with InterfacesRemoved). In and out signatures are lists of complete types.
OBJECTS_SERVER = r'''
import json, sys, warnings
warnings.simplefilter('ignore', DeprecationWarning)
from gi.repository import Gio, GLib
spec = json.loads(sys.argv[1])
log = open(sys.argv[2], 'a', buffering=1)
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
OM, PROPS = 'org.freedesktop.DBus.ObjectManager', 'org.freedesktop.DBus.Properties'
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM if spec['bus'] == 'system' else Gio.BusType.SESSION, None)
objects, regs = spec['objects'], {}
def record(obj, iname, method, args, interactive):
    log.write(json.dumps({'path': obj, 'iface': iname, 'method': method, 'args': args,
                          'interactive': interactive}) + '\n')
def xml_of(iface, desc):
    out = [f'<interface name="{iface}">']
    for name, m in desc.get('methods', {}).items():
        out.append(f'<method name="{name}">')
        out += [f'<arg type="{t}" direction="in"/>' for t in m.get('in', [])]
        out += [f'<arg type="{t}" direction="out"/>' for t in m.get('out', [])]
        out.append('</method>')
    for name, (sig, _) in desc.get('props', {}).items():
        access = 'readwrite' if name in desc.get('writable', []) else 'read'
        out.append(f'<property name="{name}" type="{sig}" access="{access}"/>')
    out.append('</interface>')
    return ''.join(out)
def variants(desc):
    return {name: GLib.Variant(sig, value) for name, (sig, value) in desc.get('props', {}).items()}
def put(path, iface, name, value):
    desc = objects[path][iface]
    desc['props'][name][1] = value
    names = [name]
    for other, by in desc.get('follow', {}).get(name, {}).items():
        desc['props'][other][1] = by[json.dumps(value)]
        names.append(other)
    changed = {n: GLib.Variant(*desc['props'][n]) for n in names}
    bus.emit_signal(None, path, PROPS, 'PropertiesChanged', GLib.Variant('(sa{sv}as)', (iface, changed, [])))
def remove(path):
    ifaces = list(objects.pop(path, {}))
    for rid in regs.pop(path, []):
        bus.unregister_object(rid)
    if ifaces:
        bus.emit_signal(None, '/', OM, 'InterfacesRemoved', GLib.Variant('(oas)', (path, ifaces)))
def call(conn, sender, obj, iname, method, params, invocation):
    record(obj, iname, method, params.print_(True), bool(invocation.get_message().get_flags() & FLAG))
    m = objects.get(obj, {}).get(iname, {}).get('methods', {}).get(method, {})
    if 'error' in m:
        invocation.return_dbus_error(m['error'][0], m['error'][1])
        return
    args = params.unpack()
    for effect in m.get('effects', []):
        if effect[0] == 'set':
            put(obj, iname, effect[1], effect[2])
        elif effect[0] == 'remove':
            remove(args[effect[1]])
    if m.get('out'):
        invocation.return_value(GLib.Variant('(' + ''.join(m['out']) + ')', tuple(m.get('value', []))))
    else:
        invocation.return_value(None)
def get(conn, sender, obj, iname, name):
    sig, value = objects[obj][iname]['props'][name]
    return GLib.Variant(sig, value)
def set_(conn, sender, obj, iname, name, value):
    record(obj, iname, 'Set:' + name, value.print_(True), False)
    if name not in objects.get(obj, {}).get(iname, {}).get('writable', []):
        return False
    put(obj, iname, name, value.unpack())
    return True
def managed(conn, sender, obj, iname, method, params, invocation):
    record(obj, iname, method, params.print_(True), bool(invocation.get_message().get_flags() & FLAG))
    tree = {path: {iface: variants(desc) for iface, desc in ifaces.items()} for path, ifaces in objects.items()}
    invocation.return_value(GLib.Variant('(a{oa{sa{sv}}})', (tree,)))
for path, ifaces in objects.items():
    for iface, desc in ifaces.items():
        info = Gio.DBusNodeInfo.new_for_xml('<node>' + xml_of(iface, desc) + '</node>').interfaces[0]
        regs.setdefault(path, []).append(bus.register_object(path, info, call, get, set_))
om = Gio.DBusNodeInfo.new_for_xml(
    f'<node><interface name="{OM}"><method name="GetManagedObjects">'
    '<arg type="a{oa{sa{sv}}}" direction="out"/></method>'
    '<signal name="InterfacesAdded"><arg type="o"/><arg type="a{sa{sv}}"/></signal>'
    '<signal name="InterfacesRemoved"><arg type="o"/><arg type="as"/></signal></interface></node>').interfaces[0]
bus.register_object('/', om, managed, None, None)
loop = GLib.MainLoop()
Gio.bus_own_name_on_connection(bus, spec['name'], Gio.BusNameOwnerFlags.NONE,
                               lambda *a: print('owned', flush=True), lambda *a: loop.quit())
loop.run()
'''


def start_objects(h, spec: Spec, tag: str, timeout: float = 10) -> StandIn | None:
    """``lib.standins.start`` for a daemon with an ObjectManager (BlueZ): the same child of the harness inside the
    sandbox, the same calls log (``standin.calls()``) and record, with OBJECTS_SERVER; None and a failed check when it
    could not own its name."""
    logs = h.hm.RUN / 'logs'
    calls = logs / f'standin-{tag}.calls.jsonl'
    calls.write_text('')
    err = open(logs / f'standin-{tag}.log', 'w')
    proc = subprocess.Popen(['python3', '-I', '-c', OBJECTS_SERVER, spec.json(), str(calls)], stdout=subprocess.PIPE,
                            stderr=err, text=True)
    deadline = time.monotonic() + timeout
    line = ''
    while time.monotonic() < deadline and proc.poll() is None:
        if not select.select([proc.stdout], [], [], 0.5)[0]:
            continue
        line = proc.stdout.readline().strip()
        if line == 'owned':
            h.record(f'standin.{tag}', {'pid': proc.pid, 'bus': spec.bus, 'name': spec.name,
                                        'objects': sorted(spec.objects)})
            return StandIn(proc, calls)
    h.check(f'stand-in {tag} owns {spec.name}', False, exit=proc.poll(), line=line)
    proc.kill()
    return None


SPECS: dict[str, "Spec"] = {'bolt': BOLT, 'upower': UPOWER, 'cupspkhelper': CUPSPKHELPER, 'colord': COLORD,
                            'colord-sensor': COLORD_SENSOR, 'gsd-rfkill': GSD_RFKILL,
                            'gsd-rfkill-flight': GSD_RFKILL_FLIGHT, 'bluez': BLUEZ}
