#!/usr/bin/python3 -IB
"""GW1: PackageKit probe for Robin's R1 (design 9.1, 11). Python 3 + Gio only; run as ``python3 -I``.

Without --live it refuses every bus except a private test bus inside a sandbox (the seam rule of the GW1 fix
round): one ``unix:path=`` socket directly in $XDG_RUNTIME_DIR, which is not under /run, is owned by the user with
mode 0700 and holds the ``.win11-update-fixture`` marker (tests/update/run.sh's layer writes it), and
/run/dbus/system_bus_socket must not be visible at all. It never falls back to the system bus or to
DBUS_SYSTEM_BUS_ADDRESS.

    python3 -I tools/update/probe.py --bus <private address> --get-updates [--simulate-all] [--phasing] [--out F]

With --live it is Robin's R1 tool on the real system bus (agents never run it; it also refuses inside a test
environment and without a terminal, and asks for the word "probe" before it connects):

    python3 -I tools/update/probe.py --live --get-updates [--simulate-all] [--phasing] --out r1-probe.json

What it does (and nothing else): NameHasOwner + ListActivatableNames (packagekitd is usually idle and D-Bus
activated: then the first call below starts it; if it is neither running nor activatable the probe stops with
errorCode "packagekitd-not-available"), the manager properties, CreateTransaction +
SetHints(interactive=false) + GetUpdates(filter none);
with --simulate-all UpdatePackages(SIMULATE|ONLY_TRUSTED = 6) on every listed id and GetDetails on them (sizes);
with --phasing the apt_pkg script the service ships (host/data/phasing.py, which phasing.rs embeds: the same bytes,
`name<TAB>arch` per package on stdin, under nice 10 and idle I/O as phasing.rs runs it) and its output, wall time and
peak RSS. Never RefreshCache, never a real install, never RepairSystem: the only
UpdatePackages call has the fixed flags 6 and asserts it. Exit 0 = done, 1 = a call failed, 2 = refused.
"""
from __future__ import annotations

import argparse
import json
import os
import resource
import subprocess
import sys
import time
import urllib.parse
from pathlib import Path

PK_NAME = PK_IFACE = 'org.freedesktop.PackageKit'
PK_PATH = '/org/freedesktop/PackageKit'
TX_IFACE = 'org.freedesktop.PackageKit.Transaction'
MARKER = '.win11-update-fixture'
REAL_SYSTEM_BUS = '/run/dbus/system_bus_socket'
FILTER_NONE = 1 << 1                       # pk_filter_bitfield_from_string("none")
SIMULATE_ONLY_TRUSTED = (1 << 2) | (1 << 1)  # pk_transaction_flag_bitfield_from_string("only-trusted;simulate")

# The phasing classifier of design 4.6 (WU-FIX, GW2b review n3): the file host/src/services/update/phasing.rs embeds
# with include_str!, read here byte for byte (phasing.rs's test probe_py_runs_the_script_this_module_ships compares
# the two), so R1 measures the script that ships. Read only for --phasing, so every refusal works without it.
PHASING_FILE = Path(__file__).resolve().parents[2] / 'host' / 'data' / 'phasing.py'
PHASING_END = '@@end'


def phasing_script() -> str:
    return PHASING_FILE.read_bytes().decode('utf-8')


def parse_phasing(stdout: str) -> tuple[list[list[str]], bool]:
    """phasing.rs's parse_script: lines of 7 fields (name, arch, candidate, phasing_applied, percentage, held,
    installed) up to the end marker; (rows, whether the marker came)."""
    rows = []
    for line in stdout.splitlines():
        if line == PHASING_END:
            return rows, True
        fields = line.split('\t')
        if len(fields) == 7:
            rows.append(fields)
    return rows, False


class _Stop(Exception):
    """A clear, expected end of the probe (the report says why)."""


def refuse(code: str, msg: str) -> int:
    print(f'probe.py: REFUSED [{code}] {msg}', file=sys.stderr, flush=True)
    return 2


def _under(path: str, prefix: str) -> bool:
    n = os.path.normpath(path)
    return n == prefix or n.startswith(prefix + '/')


def private_bus_problem(address: str) -> tuple[str, str] | None:
    """None when the address is one private, marked test bus in a sandbox; else (code, message). The address
    checks come first (so each kind of bad address shows its own code); the sandbox checks last."""
    if ';' in address:
        return 'bus-not-private', 'an address list is not accepted'
    transport, _, rest = address.partition(':')
    pairs = {}
    for item in rest.split(','):
        if '=' in item:
            k, v = item.split('=', 1)
            pairs[k] = urllib.parse.unquote(v)
    path = pairs.get('path')
    for key in ('path', 'dir', 'tmpdir', 'abstract'):
        value = pairs.get(key, '')
        if _under(value, '/run/user') or _under(value, '/var/run/user'):
            return 'bus-live-session', f'{address} points under /run/user/'
        if _under(value, '/run/dbus') or _under(value, '/var/run/dbus') or _under(value, '/run') or _under(value, '/var/run'):
            return 'bus-real-system', f'{address} points into /run (the real system bus lives there)'
    if transport != 'unix' or not path or set(pairs) - {'path', 'guid'} or pairs.get('runtime'):
        return 'bus-not-private', f'{address} is not a plain unix:path= address'
    real = os.path.realpath(path)
    if _under(real, '/run') or _under(real, '/var/run'):
        return 'bus-real-system', f'{path} resolves into /run'
    try:
        if os.path.exists(REAL_SYSTEM_BUS) and os.path.samefile(real, REAL_SYSTEM_BUS):
            return 'bus-real-system', f'{path} is the real system-bus socket'
    except OSError:
        pass
    if not os.path.isfile(os.path.join(os.path.dirname(real), MARKER)):
        return 'bus-not-private', f'{os.path.dirname(real)} has no {MARKER} marker (not a test layer bus)'
    runtime = os.environ.get('XDG_RUNTIME_DIR', '')
    rt = os.path.realpath(runtime) if runtime else ''
    if not rt or _under(rt, '/run') or _under(rt, '/var/run') or _under(os.path.normpath(runtime), '/run'):
        return 'runtime-not-private', f'XDG_RUNTIME_DIR={runtime!r} is not a private runtime directory'
    if os.path.dirname(real) != rt or os.path.dirname(os.path.normpath(path)) != os.path.normpath(runtime):
        return 'bus-not-private', f'{path} is not directly in XDG_RUNTIME_DIR={runtime}'
    try:
        st = os.stat(rt)
    except OSError as e:
        return 'runtime-not-private', f'{rt}: {e}'
    if st.st_uid != os.geteuid() or (st.st_mode & 0o777) != 0o700:
        return 'runtime-not-private', f'{rt} is not owned by uid {os.geteuid()} with mode 0700'
    if os.path.lexists(REAL_SYSTEM_BUS) or os.path.lexists('/var/run/dbus/system_bus_socket'):
        return 'system-bus-visible', f'{REAL_SYSTEM_BUS} is visible: a private bus is only accepted in a sandbox'
    return None


class Probe:
    def __init__(self, conn, timeout: float):
        from gi.repository import Gio, GLib
        self.Gio, self.GLib, self.conn, self.timeout = Gio, GLib, conn, timeout

    def presence(self) -> dict:
        """F3: packagekitd normally runs only on demand (D-Bus activation, ShutdownTimeout 300 s), so 'not
        running' is the usual state; only 'not running and not activatable' means it is not installed."""
        GLib, Gio = self.GLib, self.Gio

        def driver(method, args, sig, reply):
            return self.conn.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus',
                                       method, GLib.Variant(sig, args) if sig else None, GLib.VariantType(reply),
                                       Gio.DBusCallFlags.NONE, 10000, None).unpack()[0]
        return {'running': bool(driver('NameHasOwner', (PK_NAME,), '(s)', '(b)')),
                'activatable': PK_NAME in driver('ListActivatableNames', None, None, '(as)')}

    def manager_props(self) -> dict:
        """Without NO_AUTO_START: when packagekitd is idle this call starts it (read-only GetAll)."""
        GLib = self.GLib
        r = self.conn.call_sync(PK_NAME, PK_PATH, 'org.freedesktop.DBus.Properties', 'GetAll',
                                GLib.Variant('(s)', (PK_IFACE,)), GLib.VariantType('(a{sv})'),
                                self.Gio.DBusCallFlags.NONE, 30000, None).unpack()[0]
        keep = ('VersionMajor', 'VersionMinor', 'VersionMicro', 'BackendName', 'BackendDescription', 'Locked',
                'NetworkState', 'DistroId')
        return {k: r[k] for k in keep if k in r}

    def transaction(self, method: str, sig: str, args: tuple) -> dict:
        """One transaction; collects its signals until Finished (and Destroy) or the timeout."""
        Gio, GLib = self.Gio, self.GLib
        if method == 'UpdatePackages' and args[0] != SIMULATE_ONLY_TRUSTED:
            raise SystemExit('probe.py: internal error, UpdatePackages without SIMULATE|ONLY_TRUSTED')
        if method not in ('GetUpdates', 'UpdatePackages', 'GetDetails'):
            raise SystemExit(f'probe.py: internal error, {method} is not a probe method')
        t0 = time.monotonic()
        tid = self.conn.call_sync(PK_NAME, PK_PATH, PK_IFACE, 'CreateTransaction', None, GLib.VariantType('(o)'),
                                  Gio.DBusCallFlags.NONE, 30000, None).unpack()[0]
        out = {'method': method, 'transaction': tid, 'packages': [], 'details': [], 'errors': [], 'exit': None,
               'statuses': []}
        done = {'finished': False, 'destroyed': False}

        def on_signal(conn, sender, path, iface, member, params):
            a = params.unpack() if params is not None else ()
            if member == 'Package':
                out['packages'].append({'info': a[0], 'id': a[1], 'summary': a[2]})
            elif member == 'Packages':
                out['packages'] += [{'info': i, 'id': p, 'summary': s} for i, p, s in a[0]]
            elif member == 'Details':
                out['details'].append(a[0])
            elif member == 'ErrorCode':
                out['errors'].append({'code': a[0], 'details': a[1]})
            elif member == 'Finished':
                out['exit'], out['runtimeMs'] = a[0], a[1]
                done['finished'] = True
            elif member == 'Destroy':
                done['destroyed'] = True
            elif member == 'PropertiesChanged' and 'Status' in a[1]:
                out['statuses'].append(a[1]['Status'])
        sub = self.conn.signal_subscribe(PK_NAME, None, None, tid, None, Gio.DBusSignalFlags.NONE, on_signal)
        self.conn.call_sync(PK_NAME, tid, TX_IFACE, 'SetHints',
                            GLib.Variant('(as)', (['interactive=false', 'background=false'],)), None,
                            Gio.DBusCallFlags.NONE, 10000, None)
        self.conn.call_sync(PK_NAME, tid, TX_IFACE, method, GLib.Variant(sig, args), None, Gio.DBusCallFlags.NONE,
                            30000, None)
        loop = GLib.MainLoop()
        deadline = time.monotonic() + self.timeout

        def check():
            if done['finished'] or time.monotonic() > deadline:
                loop.quit()
                return False
            return True
        GLib.timeout_add(20, check)
        loop.run()
        self.conn.signal_unsubscribe(sub)
        out['seconds'] = round(time.monotonic() - t0, 2)
        out['timedOut'] = not done['finished']
        return out


def run_phasing(packages: list[tuple[str, str]], live: bool) -> dict:
    """The shipped script on (name, arch) pairs, as phasing.rs runs it (nice 10, idle I/O, 30 s)."""
    try:
        script = phasing_script()
    except (OSError, UnicodeDecodeError) as e:
        return {'refused': f'{PHASING_FILE}: {e}'}
    if live:
        argv = ['/usr/bin/nice', '-n', '10', '/usr/bin/ionice', '-c', '3', '/usr/bin/python3', '-I', '-c', script]
    else:
        tool_dir = os.environ.get('WIN11_UPDATE_TOOL_DIR', '')
        if not tool_dir or not os.path.isfile(os.path.join(tool_dir, MARKER)):
            return {'refused': 'WIN11_UPDATE_TOOL_DIR is not a marked test tool directory'}
        argv = [os.path.join(tool_dir, 'python3'), '-I', '-c', script]
    before = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
    t0 = time.monotonic()
    p = subprocess.run(argv, input=''.join(f'{name}\t{arch}\n' for name, arch in packages),
                       capture_output=True, text=True, timeout=30)
    rows, ended = parse_phasing(p.stdout)
    return {'rc': p.returncode, 'seconds': round(time.monotonic() - t0, 3),
            'peakRssKiB': resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss, 'peakRssBeforeKiB': before,
            'stdout': p.stdout, 'stderr': p.stderr[-2000:], 'packages': len(packages), 'ended': ended,
            'noAptPkg': '@@no-apt-pkg' in p.stdout.splitlines(), 'script': str(PHASING_FILE.relative_to(PHASING_FILE.parents[2])),
            'phased': sorted(r[0] for r in rows if r[3] == '1'), 'held': sorted(r[0] for r in rows if r[5] == '1'),
            'pup': {r[0]: r[4] for r in rows if r[4] != '-'}, 'tool': 'stub' if not live else '/usr/bin/python3'}


def main() -> int:
    ap = argparse.ArgumentParser(description='PackageKit probe for R1 (GW1).')
    ap.add_argument('--bus', help='a private test bus address (without --live)')
    ap.add_argument('--live', action='store_true', help='Robin only: the real system bus')
    ap.add_argument('--get-updates', action='store_true')
    ap.add_argument('--simulate-all', action='store_true')
    ap.add_argument('--phasing', action='store_true')
    ap.add_argument('--out')
    ap.add_argument('--timeout', type=float, default=300)
    a = ap.parse_args()
    if os.geteuid() == 0:
        return refuse('root', 'never run the probe as root')
    if a.live:
        test_vars = sorted(k for k in os.environ if k.startswith(('WIN11_UPDATE_', 'WIN11_FAKE_')))
        if test_vars:
            return refuse('live-in-test', f'--live inside a test environment ({", ".join(test_vars)})')
        if a.bus:
            return refuse('live-with-bus', '--live and --bus exclude each other')
        if not sys.stdin.isatty():
            return refuse('live-needs-tty', '--live asks for a confirmation on a terminal')
        if not a.get_updates:
            return refuse('nothing-to-do', 'give --get-updates')
        print('This reads the update list from the real PackageKit (it starts packagekitd when it is idle; no '
              'refresh, no install; with --simulate-all a simulation only). Type "probe" to continue: ', end='',
              flush=True)
        if sys.stdin.readline().strip() != 'probe':
            return refuse('not-confirmed', 'not confirmed')
    else:
        if not a.bus:
            return refuse('no-bus', 'without --live a private --bus is required (the system bus is never used)')
        problem = private_bus_problem(a.bus)
        if problem:
            return refuse(*problem)
        if not a.get_updates:
            return refuse('nothing-to-do', 'give --get-updates')
    if (a.simulate_all or a.phasing) and not a.get_updates:
        return refuse('nothing-to-do', '--simulate-all and --phasing need --get-updates')

    import gi
    gi.require_version('Gio', '2.0')
    from gi.repository import Gio
    if a.live:
        conn = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
    else:
        flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
        conn = Gio.DBusConnection.new_for_address_sync(a.bus, flags, None, None)
    probe = Probe(conn, a.timeout if a.live else min(a.timeout, 60))
    report = {'tool': 'tools/update/probe.py', 'when': time.strftime('%Y-%m-%d %H:%M:%S %z'),
              'bus': 'live' if a.live else 'private', 'address': 'system' if a.live else a.bus}
    rc = 0
    try:
        report['daemon'] = daemon = probe.presence()
        if not daemon['running'] and not daemon['activatable']:
            report['errorCode'] = 'packagekitd-not-available'
            report['error'] = ('packagekitd is not running and not D-Bus activatable '
                               '(is the packagekit package installed?)')
            raise _Stop()
        report['packagekit'] = probe.manager_props()
        report['getUpdates'] = gu = probe.transaction('GetUpdates', '(t)', (FILTER_NONE,))
        ids = [p['id'] for p in gu['packages']]
        if a.simulate_all and ids:
            report['simulate'] = probe.transaction('UpdatePackages', '(tas)', (SIMULATE_ONLY_TRUSTED, ids))
            report['details'] = det = probe.transaction('GetDetails', '(as)', (ids,))
            det['downloadSizePresent'] = sum(1 for d in det['details'] if 'download-size' in d)
        if a.phasing:
            report['phasing'] = run_phasing(sorted({(i.split(';')[0], i.split(';')[2]) for i in ids if i.count(';') == 3}),
                                            a.live)
        if any(t.get('timedOut') or t.get('exit') != 1 for t in
               (report.get('getUpdates', {}), report.get('simulate', {'exit': 1}), report.get('details', {'exit': 1}))):
            rc = 1
    except _Stop:
        rc = 1
    except Exception as e:      # noqa: BLE001
        report['error'] = repr(e)
        rc = 1
    finally:
        conn.close_sync(None)
    text = json.dumps(report, indent=1, ensure_ascii=False, default=str) + '\n'
    if a.out:
        Path(a.out).write_text(text, encoding='utf-8')
        print(f'probe.py: wrote {a.out} ({len(report.get("getUpdates", {}).get("packages", []))} updates)')
    else:
        sys.stdout.write(text)
    return rc


if __name__ == '__main__':
    sys.exit(main())
