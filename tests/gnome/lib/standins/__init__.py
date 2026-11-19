"""D-Bus stand-ins for the settings-parity scenarios (notes/claude/settings-parity.md §7, amendment 61b).

The harness's private buses run no services, so a scenario starts the daemons its page talks to as stand-ins: a child
Python process of the harness (python3 -I, inside the sandbox) that owns a name on the private system or session bus,
answers from a JSON spec and records every call it gets (method, arguments as GVariant text, and whether the caller
allowed interactive authorization: host/src/services/system/common.rs call_interactive must set it). Each batch keeps its specs in ``tests/gnome/lib/standins/<batch>.py`` (b0 a b c d e f), so parallel
batches never edit the same file.

    from lib.standins import Spec, start
    spec = Spec('system', 'org.freedesktop.hostname1', {
        '/org/freedesktop/hostname1': {'org.freedesktop.hostname1': {
            'props': {'Hostname': ('s', 'winpoint-test'), 'Chassis': ('s', 'laptop')},
            'methods': {'SetStaticHostname': {'in': 'sb', 'out': ''},
                        'SetPrettyHostname': {'in': 'sb', 'error': ['org.freedesktop.DBus.Error.AccessDenied', 'no']}},
        }}})
    standin = start(h, spec, 'hostname1')          # None (and a failed check) when it could not own its name
    ...
    calls = standin.calls()                         # [{'path', 'iface', 'method', 'args', 'interactive'}]
    standin.stop()

A method answers ``out`` (its out signature) with ``value`` (a list matching it, default empty values), or an
``error`` [name, message]. Properties answer Get/GetAll and refuse Set unless ``writable`` lists the property.
"""
from __future__ import annotations

import json
import select
import subprocess
import time
from dataclasses import dataclass, field

SERVER = r'''
import json, sys, warnings
warnings.simplefilter('ignore', DeprecationWarning)
from gi.repository import Gio, GLib
spec = json.loads(sys.argv[1])
log = open(sys.argv[2], 'a', buffering=1)
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
def xml_of(iface, desc):
    out = [f'<interface name="{iface}">']
    for name, m in desc.get('methods', {}).items():
        out.append(f'<method name="{name}">')
        out += [f'<arg type="{t}" direction="in"/>' for t in split(m.get('in', ''))]
        out += [f'<arg type="{t}" direction="out"/>' for t in split(m.get('out', ''))]
        out.append('</method>')
    for name, (sig, _) in desc.get('props', {}).items():
        access = 'readwrite' if name in desc.get('writable', []) else 'read'
        out.append(f'<property name="{name}" type="{sig}" access="{access}"/>')
    for name, sig in desc.get('signals', {}).items():
        out.append(f'<signal name="{name}">' + ''.join(f'<arg type="{t}"/>' for t in split(sig)) + '</signal>')
    out.append('</interface>')
    return ''.join(out)
def split(sig):
    out, i = [], 0
    while i < len(sig):
        j = i + 1
        if sig[i] in 'a':
            while sig[j - 1] == 'a':
                j += 1
            j -= 1
            k = j
            if sig[k] in '({':
                depth = 0
                while True:
                    depth += {'(': 1, '{': 1, ')': -1, '}': -1}.get(sig[k], 0)
                    k += 1
                    if depth == 0:
                        break
                j = k
            else:
                j = k + 1
        elif sig[i] in '({':
            depth, k = 0, i
            while True:
                depth += {'(': 1, '{': 1, ')': -1, '}': -1}.get(sig[k], 0)
                k += 1
                if depth == 0:
                    break
            j = k
        out.append(sig[i:j])
        i = j
    return out
def conv(x):
    if isinstance(x, dict) and set(x) == {'$v'}:
        return GLib.Variant(x['$v'][0], conv(x['$v'][1]))
    if isinstance(x, dict):
        return {k: conv(v) for k, v in x.items()}
    if isinstance(x, list):
        return [conv(v) for v in x]
    return x
def handler(path, iface, desc):
    def call(conn, sender, obj, iname, method, params, invocation):
        interactive = bool(invocation.get_message().get_flags() & FLAG)
        log.write(json.dumps({'path': obj, 'iface': iname, 'method': method, 'args': params.print_(True),
                              'interactive': interactive}) + '\n')
        m = desc.get('methods', {}).get(method, {})
        if 'error' in m:
            invocation.return_dbus_error(m['error'][0], m['error'][1])
        elif m.get('permissionStore'):
            table, _create, eid, app, values = params.unpack()
            lookup = desc['methods']['Lookup'].setdefault('byArgs', {})
            key = GLib.Variant('(ss)', (table, eid)).print_(True)
            entry = dict((lookup.get(key) or [{}])[0])
            entry[app] = list(values)
            lookup[key] = [entry]
            invocation.return_value(None)
            conn.emit_signal(None, obj, iname, 'Changed',
                             GLib.Variant('(ssbva{sas})', (table, eid, False, GLib.Variant('s', ''), entry)))
        elif m.get('out'):
            value = m.get('byArgs', {}).get(params.print_(True), m.get('value', []))
            invocation.return_value(GLib.Variant(f"({m['out']})", tuple(conv(v) for v in value)))
        else:
            invocation.return_value(None)
    def get(conn, sender, obj, iname, name):
        sig, value = desc['props'][name]
        return GLib.Variant(sig, conv(value))
    def set_(conn, sender, obj, iname, name, value):
        log.write(json.dumps({'path': obj, 'iface': iname, 'method': 'Set:' + name, 'args': value.print_(True),
                              'interactive': False}) + '\n')
        if name in desc.get('writable', []):
            desc['props'][name][1] = value.unpack()
            return True
        return False
    return call, get, set_
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM if spec['bus'] == 'system' else Gio.BusType.SESSION, None)
for path, ifaces in spec['objects'].items():
    for iface, desc in ifaces.items():
        info = Gio.DBusNodeInfo.new_for_xml('<node>' + xml_of(iface, desc) + '</node>').interfaces[0]
        bus.register_object(path, info, *handler(path, iface, desc))
loop = GLib.MainLoop()
Gio.bus_own_name_on_connection(bus, spec['name'], Gio.BusNameOwnerFlags.NONE,
                               lambda *a: print('owned', flush=True), lambda *a: loop.quit())
loop.run()
'''


@dataclass
class Spec:
    """A stand-in: the bus ('system' or 'session'), the name it owns, its objects {path: {iface: description}}."""
    bus: str
    name: str
    objects: dict = field(default_factory=dict)

    def json(self) -> str:
        if self.bus not in ('system', 'session'):
            raise ValueError(f'bus {self.bus!r}: system or session')
        return json.dumps({'bus': self.bus, 'name': self.name, 'objects': self.objects})


class StandIn:
    def __init__(self, proc: subprocess.Popen, log_path):
        self.proc, self.log_path = proc, log_path

    def calls(self) -> list[dict]:
        """Every call the stand-in got so far, in order."""
        try:
            return [json.loads(line) for line in self.log_path.read_text().splitlines() if line.strip()]
        except OSError:
            return []

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                self.proc.kill()


def start(h, spec: Spec, tag: str, timeout: float = 10) -> StandIn | None:
    """Start a stand-in as a child of the harness (inside the sandbox, on its private bus) and wait until it owns its
    name; records it in the results (``standin.<tag>``), or a failed check and None."""
    logs = h.hm.RUN / 'logs'
    calls = logs / f'standin-{tag}.calls.jsonl'
    calls.write_text('')
    err = open(logs / f'standin-{tag}.log', 'w')
    proc = subprocess.Popen(['python3', '-I', '-c', SERVER, spec.json(), str(calls)], stdout=subprocess.PIPE,
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
