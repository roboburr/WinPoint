#!/usr/bin/python3 -I
"""GW7 layer test (design 6.4, 6.6, 9.2): win11-launch --post-upgrade-check and the release start's spawn path
against fake GNOME services, inside tests/update/run.sh's bubblewrap layer.

    tests/update/test_post_upgrade.py run --launch target/debug/win11-launch [--lib target/debug/deps/win11_host-HASH]
        [--only NAME ...]

`run` (outside) re-executes this file through `tests/update/run.sh exec --allow-layer-dgrams` (the allowlisted root,
a private /run and /tmp, --unshare-net, the strace check of every connect/sendto, the stub-log check: no stub of the
layer may run). `inner` (inside the layer) starts its own private session bus (tests/update/bus.conf: no service
dirs, a socket in /tmp/w11-update) and one Python + Gio process (`serve`) that owns, on that bus only:

* org.gnome.Shell (/org/gnome/Shell, property ShellVersion),
* org.gnome.Shell.Extensions (GetExtensionInfo → a{sv} with `state` as a double, EnableExtension),
* org.freedesktop.Notifications (Notify; per scenario it answers with ActivationToken + ActionInvoked +
  NotificationClosed, or only NotificationClosed, or nothing),
* org.freedesktop.systemd1 (the user manager: StartTransientUnit is recorded and nothing is started; the
  Environment property), so process_scope::spawn takes its Role::App path and the unit's Slice can be checked.

Every call is logged with its time (`<run>/gw7/calls.jsonl`). Each scenario gets a fresh marked fixture root (os-release,
procfs, the soname directory, org.gnome.shell's schema and a keyfile), a marked stub tool directory (loginctl,
dpkg-query, apt-cache, … : they log and answer from fixtures, builtins only), and its own XDG config/cache/data.
With --lib, the lib test binary's ignored test services::update::upgrade::tests::layer_start_goes_through_the_user_manager
runs against the same bus, and its StartTransientUnit is checked (app.slice, the exact argv and environment).
Exit 0 = every scenario passed.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import select
import shutil
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
W = HERE.parent.parent
RT = Path('/tmp/w11-update')
MARKER = '.win11-update-fixture'
UUID = 'win11-gnome@win11shell.local'
DOCK = 'ubuntu-dock@ubuntu.com'
FIXTURES = HERE / 'fixtures' / 'upgrade'
SONAMES = ('libwebkit2gtk-4.1.so.0', 'libjavascriptcoregtk-4.1.so.0', 'libgtk-3.so.0', 'libgdk-3.so.0', 'libsoup-3.0.so.0',
           'libglib-2.0.so.0', 'libgobject-2.0.so.0', 'libgio-2.0.so.0', 'libgdk_pixbuf-2.0.so.0', 'libcairo.so.2',
           'libz.so.1', 'libarchive.so.13')
RESOLUTE = 'NAME="Ubuntu"\nVERSION_ID="26.04"\nVERSION="26.04.1 LTS (Resolute Raccoon)"\nVERSION_CODENAME=resolute\n'
STONKING = 'NAME="Ubuntu"\nVERSION_ID="26.10"\nVERSION="26.10 (Stonking Stingray)"\nVERSION_CODENAME=stonking\n'


def wrapper_text() -> str:
    """The WRAPPER constant of host/src/services/update/upgrade.rs (a raw string)."""
    text = (W / 'host/src/services/update/upgrade.rs').read_text(encoding='utf-8')
    m = re.search(r'pub\(crate\) const WRAPPER: &str = r#"(.*?)"#;', text, re.S)
    if not m:
        raise SystemExit('test_post_upgrade: WRAPPER not found in upgrade.rs')
    return m.group(1)


# ---------------------------------------------------------------------------------------------------- serve

IFACES = {
    '/org/gnome/Shell': '<node><interface name="org.gnome.Shell"><property name="ShellVersion" type="s" access="read"/>'
                        '</interface></node>',
    '/org/gnome/Shell/Extensions': '<node><interface name="org.gnome.Shell.Extensions">'
                                   '<method name="GetExtensionInfo"><arg type="s" direction="in"/>'
                                   '<arg type="a{sv}" direction="out"/></method>'
                                   '<method name="EnableExtension"><arg type="s" direction="in"/>'
                                   '<arg type="b" direction="out"/></method></interface></node>',
    '/org/freedesktop/Notifications': '<node><interface name="org.freedesktop.Notifications">'
                                      '<method name="Notify"><arg type="s" direction="in"/><arg type="u" direction="in"/>'
                                      '<arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/>'
                                      '<arg type="as" direction="in"/><arg type="a{sv}" direction="in"/>'
                                      '<arg type="i" direction="in"/><arg type="u" direction="out"/></method>'
                                      '<method name="CloseNotification"><arg type="u" direction="in"/></method>'
                                      '<method name="GetCapabilities"><arg type="as" direction="out"/></method>'
                                      '<signal name="ActionInvoked"><arg type="u"/><arg type="s"/></signal>'
                                      '<signal name="ActivationToken"><arg type="u"/><arg type="s"/></signal>'
                                      '<signal name="NotificationClosed"><arg type="u"/><arg type="u"/></signal>'
                                      '</interface></node>',
    '/org/freedesktop/systemd1': '<node><interface name="org.freedesktop.systemd1.Manager">'
                                 '<method name="StartTransientUnit"><arg type="s" direction="in"/><arg type="s" direction="in"/>'
                                 '<arg type="a(sv)" direction="in"/><arg type="a(sa(sv))" direction="in"/>'
                                 '<arg type="o" direction="out"/></method>'
                                 '<method name="GetUnit"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>'
                                 '<property name="Environment" type="as" access="read"/></interface></node>',
}
NAMES = ('org.gnome.Shell', 'org.gnome.Shell.Extensions', 'org.freedesktop.Notifications', 'org.freedesktop.systemd1')


def cmd_serve(a) -> int:
    from gi.repository import Gio, GLib
    flags = Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION
    conn = Gio.DBusConnection.new_for_address_sync(a.bus, flags, None, None)
    log = open(a.log, 'a', encoding='utf-8')
    counter = {'id': 0, 'runs': {}}

    def scenario() -> dict:
        try:
            return json.loads(Path(a.scenario).read_text(encoding='utf-8'))
        except (OSError, ValueError):
            return {}

    def record(iface, method, args, extra=None):
        line = {'t': time.time(), 'iface': iface, 'method': method, 'args': args, **(extra or {})}
        log.write(json.dumps(line, default=str) + '\n')
        log.flush()

    def emit(member, sig, values):
        record('org.freedesktop.Notifications', 'emit:' + member, list(values))
        conn.emit_signal(None, '/org/freedesktop/Notifications', 'org.freedesktop.Notifications', member,
                         GLib.Variant(sig, values))
        return False

    def on_call(_conn, sender, path, iface, method, params, invocation):
        args = params.unpack()
        s = scenario()
        if iface == 'org.gnome.Shell.Extensions' and method == 'GetExtensionInfo':
            record(iface, method, list(args))
            info = {'uuid': GLib.Variant('s', args[0])}
            state = s.get('extensionState', 1.0)
            if isinstance(state, float):
                info['state'] = GLib.Variant('d', state)
            elif isinstance(state, str):
                info['state'] = GLib.Variant('s', state)
            if s.get('extensionEnabled') is not None:
                info['enabled'] = GLib.Variant('b', bool(s['extensionEnabled']))
            invocation.return_value(GLib.Variant('(a{sv})', (info,)))
        elif iface == 'org.gnome.Shell.Extensions' and method == 'EnableExtension':
            record(iface, method, list(args))
            invocation.return_value(GLib.Variant('(b)', (True,)))
        elif iface == 'org.freedesktop.Notifications' and method == 'Notify':
            counter['id'] += 1
            nid = counter['id']
            # the scenario's own count picks its answer and names its token (ids are global across scenarios)
            run = s.get('run', '')
            counter['runs'][run] = index = counter['runs'].get(run, 0) + 1
            app, _replaces, icon, summary, body, actions, hints, timeout = args
            record(iface, method, {'app': app, 'icon': icon, 'summary': summary, 'body': body, 'actions': actions,
                                   'hints': hints, 'timeout': timeout}, {'id': nid})
            invocation.return_value(GLib.Variant('(u)', (nid,)))
            answers = s.get('answers', [])
            answer = answers[index - 1] if index - 1 < len(answers) else s.get('answer', 'close')
            if answer.startswith('action:'):
                key = answer.split(':', 1)[1]
                GLib.timeout_add(150, emit, 'ActivationToken', '(us)', (nid, f'token-{index}'))
                GLib.timeout_add(200, emit, 'ActionInvoked', '(us)', (nid, key))
                GLib.timeout_add(250, emit, 'NotificationClosed', '(uu)', (nid, 2))
            elif answer == 'close':
                GLib.timeout_add(200, emit, 'NotificationClosed', '(uu)', (nid, 2))
        elif iface == 'org.freedesktop.Notifications' and method == 'GetCapabilities':
            invocation.return_value(GLib.Variant('(as)', (['actions', 'body'],)))
        elif iface == 'org.freedesktop.Notifications' and method == 'CloseNotification':
            record(iface, method, list(args))
            invocation.return_value(None)
        elif iface == 'org.freedesktop.systemd1.Manager' and method == 'StartTransientUnit':
            name, mode, props, aux = args
            record(iface, method, {'name': name, 'mode': mode, 'properties': {k: v for k, v in props}, 'aux': aux})
            invocation.return_value(GLib.Variant('(o)', ('/org/freedesktop/systemd1/job/1',)))
        elif iface == 'org.freedesktop.systemd1.Manager' and method == 'GetUnit':
            invocation.return_dbus_error('org.freedesktop.systemd1.NoSuchUnit', 'no such unit (GW7 fake)')
        else:
            invocation.return_dbus_error('org.freedesktop.DBus.Error.UnknownMethod', method)

    def on_get(_conn, sender, path, iface, prop):
        record(iface, 'Get:' + prop, [])
        if iface == 'org.gnome.Shell' and prop == 'ShellVersion':
            version = scenario().get('shellVersion', '51.0')
            return GLib.Variant('s', version) if version else None
        if iface == 'org.freedesktop.systemd1.Manager' and prop == 'Environment':
            return GLib.Variant('as', [])
        return None

    for path, xml in IFACES.items():
        info = Gio.DBusNodeInfo.new_for_xml(xml).interfaces[0]
        conn.register_object(path, info, on_call, on_get, None)
    for name in NAMES:
        reply = conn.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName',
                               GLib.Variant('(su)', (name, 4)), GLib.VariantType('(u)'), Gio.DBusCallFlags.NONE, 5000,
                               None)
        if reply.unpack()[0] != 1:
            print(f'serve: could not own {name}', file=sys.stderr)
            return 1
    Path(a.ready).write_text('ready\n')
    GLib.MainLoop().run()
    return 0


# ---------------------------------------------------------------------------------------------------- inner

def refuse(why: str) -> int:
    print(f'test_post_upgrade: REFUSED [{why}] (only inside tests/update/run.sh exec)', file=sys.stderr)
    return 2


def layer_problems() -> list[str]:
    problems = []
    if not os.environ.get('WIN11_FAKE_RUN_DIR'):
        problems.append('no WIN11_FAKE_RUN_DIR')
    if os.path.lexists('/run/dbus/system_bus_socket'):
        problems.append('the real system bus socket is visible')
    host = os.environ.get('WIN11_TEST_HOST_NETNS', '')
    if not host or os.readlink('/proc/self/ns/net') == host:
        problems.append('the host network namespace')
    try:
        if Path('/proc/1/comm').read_text().strip() != 'bwrap':
            problems.append('PID 1 is not bwrap')
    except OSError:
        problems.append('no /proc/1/comm')
    for key in ('DBUS_SESSION_BUS_ADDRESS', 'DBUS_SYSTEM_BUS_ADDRESS', 'XDG_RUNTIME_DIR'):
        if '/run/user/' in os.environ.get(key, ''):
            problems.append(f'{key} under /run/user')
    return problems


def start_bus(log: Path, procs: list) -> str:
    r, w = os.pipe()
    p = subprocess.Popen(['/usr/bin/dbus-daemon', f'--config-file={HERE / "bus.conf"}', '--nofork', f'--print-address={w}'],
                         pass_fds=(w,), stdin=subprocess.DEVNULL, stdout=open(log, 'w'), stderr=subprocess.STDOUT, cwd='/')
    os.close(w)
    procs.append(p)
    buf, deadline = b'', time.monotonic() + 10
    while b'\n' not in buf and time.monotonic() < deadline:
        if select.select([r], [], [], 0.2)[0]:
            chunk = os.read(r, 4096)
            if not chunk:
                break
            buf += chunk
    os.close(r)
    address = buf.decode().strip()
    if not address.startswith(f'unix:path={RT}/dbus-'):
        raise RuntimeError(f'the GW7 session bus did not start ({address!r})')
    return address


def private(path: Path, marker: bool = False) -> Path:
    path.mkdir(parents=True, exist_ok=True)
    os.chmod(path, 0o700)
    if marker:
        (path / MARKER).write_text('GW7 test seam marker\n')
    return path


def write(path: Path, text: str, mode: int = 0o644) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding='utf-8')
    os.chmod(path, mode)


def stub(tools: Path, name: str, body: str) -> None:
    """A stub of the scenario's tool directory: logs its argv (one JSON line via printf, builtins only), then body."""
    log = tools.parent / 'tools.log'
    text = ('#!/bin/sh\nprintf \'%s\' "' + name + '" >> \'' + str(log) + '\'\n'
            'for a in "$@"; do printf \'\\t%s\' "$a" >> \'' + str(log) + '\'; done\n'
            'printf \'\\n\' >> \'' + str(log) + '\'\n' + body)
    write(tools / name, text, 0o755)


class Scenario:
    def __init__(self, base: Path, name: str, lang: str = 'nl'):
        self.name = name
        self.dir = base / name
        if self.dir.exists():
            shutil.rmtree(self.dir)
        self.root = private(self.dir / 'root', marker=True)
        self.tools = private(self.dir / 'tools', marker=True)
        self.config = private(self.dir / 'config')
        self.cache = private(self.dir / 'cache')
        self.data = private(self.dir / 'data')
        private(self.root / 'home')
        write(self.root / 'etc/os-release', STONKING)
        (self.root / 'proc').mkdir()
        for so in SONAMES:
            write(self.root / 'usr/lib/x86_64-linux-gnu' / so, '')
        schemas = self.root / 'usr/share/glib-2.0/schemas'
        schemas.mkdir(parents=True)
        for f in ('org.gnome.shell.gschema.xml', 'gschemas.compiled'):
            shutil.copy2(FIXTURES / 'schemas' / f, schemas / f)
        self.keyfile = self.root / 'home/.config/glib-2.0/settings/keyfile'
        self.shell_keys(False, [UUID])
        settings = json.loads((W / 'host/src/settings-defaults.json').read_text(encoding='utf-8'))
        settings['language'] = lang
        write(self.config / 'win11-gnome/settings.json', json.dumps(settings), 0o600)
        os.chmod(self.config / 'win11-gnome', 0o700)
        self.record({'versionId': '26.04', 'codename': 'resolute', 'gnomeMajor': '50', 'extensionVersion': 3})
        write(self.data / f'gnome-shell/extensions/{UUID}/metadata.json', json.dumps({'uuid': UUID, 'version': 3}))
        for name_ in ('compat-stonking.json', 'release.json', 'upgrade.json', 'snapshot.json'):
            write(self.cache / 'win11-gnome/update' / name_, '{}', 0o600)
        os.chmod(self.cache / 'win11-gnome/update', 0o700)
        # the wrapper's status directory as upgrade.rs makes it (0700, ours: install_parts refuses anything else)
        private(self.root / 'home/.run/win11-gnome')
        write(self.root / 'home/.run/win11-gnome/upgrade-status.json', '{"rc":0,"endedAt":1}\n')
        for tool in ('loginctl', 'dpkg-query', 'apt-cache', 'apt-get', 'pkexec', 'sh', 'xdg-terminal-exec'):
            stub(self.tools, tool, 'exit 97\n')
        stub(self.tools, 'loginctl', 'exit 0\n')
        apt = FIXTURES / 'apt'
        stub(self.tools, 'apt-cache', f'f=\'{apt}\'/"$1-$2.txt"\n[ -f "$f" ] || exit 0\n'
                                      'while IFS= read -r l; do printf \'%s\\n\' "$l"; done < "$f"\n')
        self.spec = {}

    def shell_keys(self, disable: bool, enabled: list[str]) -> None:
        values = ', '.join(f"'{u}'" for u in enabled)
        write(self.keyfile, f'[org/gnome/shell]\ndisable-user-extensions={"true" if disable else "false"}\n'
                            f'enabled-extensions=[{values}]\n')

    def record(self, value) -> None:
        path = self.config / 'win11-gnome/last-os-release.json'
        if value is None:
            path.unlink(missing_ok=True)
        else:
            write(path, json.dumps(value), 0o600)

    def stamp(self) -> Path:
        return self.config / 'win11-gnome/update-linger-by-us'

    def env(self, bus: str) -> dict:
        env = dict(os.environ)
        env.update({'DBUS_SESSION_BUS_ADDRESS': bus, 'WIN11_UPDATE_FIXTURE_ROOT': str(self.root),
                    'WIN11_UPDATE_TOOL_DIR': str(self.tools), 'XDG_CONFIG_HOME': str(self.config),
                    'XDG_CACHE_HOME': str(self.cache), 'XDG_DATA_HOME': str(self.data), 'HOME': str(self.root / 'home')})
        env.pop('WIN11_GNOME_CONFIG_DIR', None)
        return env

    def tool_log(self) -> list[list[str]]:
        path = self.dir / 'tools.log'
        return [line.split('\t') for line in path.read_text().splitlines()] if path.exists() else []


class Run:
    def __init__(self, args, bus: str, calls: Path, scenario_file: Path):
        self.args, self.bus, self.calls, self.scenario_file = args, bus, calls, scenario_file
        self.passed, self.failed = 0, 0

    def mark(self) -> int:
        return len(self.calls.read_text().splitlines()) if self.calls.exists() else 0

    def since(self, mark: int) -> list[dict]:
        return [json.loads(l) for l in self.calls.read_text().splitlines()[mark:]] if self.calls.exists() else []

    def launch(self, sc: Scenario, extra_env: dict | None = None) -> tuple[int, float, list[dict]]:
        self.scenario_file.write_text(json.dumps({**sc.spec, 'run': f'{sc.name}-{time.monotonic_ns()}'}))
        mark = self.mark()
        env = sc.env(self.bus)
        env.update(extra_env or {})
        t0 = time.monotonic()
        proc = subprocess.run([str(W / self.args.launch), '--post-upgrade-check'], env=env, stdin=subprocess.DEVNULL,
                              capture_output=True, text=True, timeout=120, cwd=str(W))
        took = time.monotonic() - t0
        (sc.dir / 'launch.out').write_text(proc.stdout + proc.stderr)
        time.sleep(0.4)       # signals the fake still emits after the program ended are logged too
        return proc.returncode, took, self.since(mark)

    def check(self, name: str, conditions: list[tuple[str, bool]]) -> None:
        bad = [what for what, ok in conditions if not ok]
        if bad:
            self.failed += 1
            print(f'FAIL {name}: ' + '; '.join(bad))
        else:
            self.passed += 1
            print(f'PASS {name} ({len(conditions)} checks)')


def methods(calls: list[dict]) -> list[str]:
    return [c['method'] for c in calls if not c['method'].startswith('emit:')]


def notifies(calls: list[dict]) -> list[dict]:
    return [c for c in calls if c['method'] == 'Notify']


def scenarios(r: Run, base: Path, only: list[str]) -> None:
    uid = str(os.getuid())
    wrapper = wrapper_text()

    def want(name: str) -> bool:
        return not only or name in only

    if want('first-run'):
        sc = Scenario(base, 'first-run')
        sc.record(None)
        rc, took, calls = r.launch(sc)
        rec = json.loads((sc.config / 'win11-gnome/last-os-release.json').read_text())
        r.check('first-run', [('rc 0', rc == 0), ('record written', rec == {'versionId': '26.10', 'codename': 'stonking',
                                                                              'gnomeMajor': '51', 'extensionVersion': 3}),
                              ('only ShellVersion read', methods(calls) == ['Get:ShellVersion']),
                              ('caches kept', (sc.cache / 'win11-gnome/update/compat-stonking.json').exists()),
                              ('no tool', sc.tool_log() == [])])
    if want('unchanged'):
        sc = Scenario(base, 'unchanged')
        sc.record({'versionId': '26.10', 'codename': 'stonking', 'gnomeMajor': '51', 'extensionVersion': 3})
        rc, took, calls = r.launch(sc)
        r.check(f'unchanged [{took:.2f} s]', [('rc 0', rc == 0), ('only ShellVersion', methods(calls) == ['Get:ShellVersion']),
                              (f'quick ({took:.2f} s)', took < 1.5), ('no tool', sc.tool_log() == []),
                              ('caches kept', (sc.cache / 'win11-gnome/update/release.json').exists())])
    if want('unchanged-stamp'):
        sc = Scenario(base, 'unchanged-stamp')
        sc.record({'versionId': '26.10', 'codename': 'stonking', 'gnomeMajor': '51', 'extensionVersion': 3})
        write(sc.stamp(), '{}', 0o600)
        rc, took, calls = r.launch(sc)
        r.check('unchanged-stamp', [('rc 0', rc == 0), ('disable-linger', sc.tool_log() == [['loginctl', 'disable-linger', uid]]),
                                    ('stamp removed', not sc.stamp().exists()), ('no notification', not notifies(calls))])
    if want('upgraded-active'):
        sc = Scenario(base, 'upgraded-active')
        write(sc.stamp(), '{}', 0o600)
        sc.spec = {'extensionState': 1.0}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        rec = json.loads((sc.config / 'win11-gnome/last-os-release.json').read_text())
        upd = sc.cache / 'win11-gnome/update'
        r.check(f'upgraded-active [{took:.2f} s]', [
            ('rc 0', rc == 0), ('one notification', len(n) == 1),
            ('nl title', n and n[0]['args']['summary'] == 'Ubuntu is bijgewerkt naar 26.10'),
            ('no actions', n and n[0]['args']['actions'] == []),
            ('app name and desktop entry', n and n[0]['args']['app'] == 'Win11 Shell'
             and n[0]['args']['hints'].get('desktop-entry') == 'io.github.win11shell.Settings'),
            ('state read', 'GetExtensionInfo' in methods(calls)), ('no EnableExtension', 'EnableExtension' not in methods(calls)),
            ('record now 26.10', rec['versionId'] == '26.10' and rec['gnomeMajor'] == '51'),
            ('compat/release/upgrade.json removed', not any((upd / f).exists() for f in ('compat-stonking.json', 'release.json', 'upgrade.json'))),
            ('snapshot kept', (upd / 'snapshot.json').exists()),
            ('status file removed', not (sc.root / 'home/.run/win11-gnome/upgrade-status.json').exists()),
            ('linger off, stamp gone', sc.tool_log() == [['loginctl', 'disable-linger', uid]] and not sc.stamp().exists()),
            (f'no wait ({took:.2f} s)', took < 2)])
    if want('out-of-date-dock'):
        sc = Scenario(base, 'out-of-date-dock', 'en')
        sc.spec = {'extensionState': 4.0, 'answer': 'action:dockOn'}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        enables = [c for c in calls if c['method'] == 'EnableExtension']
        acted = [c for c in calls if c['method'] == 'emit:ActionInvoked']
        r.check(f'out-of-date-dock [{took:.2f} s]', [
            ('rc 0', rc == 0), ('one notification', len(n) == 1),
            ('en title', n and n[0]['args']['summary'] == 'GNOME 51 turned Win11 Shell off'),
            ('en body', n and n[0]['args']['body'] == 'Win11 Shell’s taskbar doesn’t work with this GNOME version yet. '
                                                       'Turn on the Ubuntu Dock until there’s an update.'),
            ('actions', n and n[0]['args']['actions'] == ['dockOn', 'Turn on Ubuntu Dock', 'later', 'Later']),
            ('EnableExtension(ubuntu-dock) once', [c['args'] for c in enables] == [[DOCK]]),
            ('only after the click', bool(enables and acted and enables[0]['t'] > acted[0]['t'])),
            (f'returns after the answer ({took:.2f} s)', took < 4)])
    if want('inactive-enabled-dismissed'):
        sc = Scenario(base, 'inactive-enabled-dismissed')
        sc.spec = {'extensionState': 2.0, 'extensionEnabled': False, 'answer': 'close'}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        r.check('inactive-enabled-dismissed', [
            ('rc 0', rc == 0), ('extOff', len(n) == 1 and n[0]['args']['summary'] == 'GNOME 51 heeft Win11 Shell uitgezet'),
            ('nl actions', n and n[0]['args']['actions'] == ['dockOn', 'Ubuntu Dock aanzetten', 'later', 'Later']),
            ('no EnableExtension without the click', 'EnableExtension' not in methods(calls))])
    if want('inactive-not-enabled'):
        sc = Scenario(base, 'inactive-not-enabled')
        sc.shell_keys(False, ['other@example.org'])
        sc.spec = {'extensionState': 2.0}
        rc, took, calls = r.launch(sc)
        r.check('inactive-not-enabled', [('rc 0', rc == 0), ('no notification', not notifies(calls)),
                                         ('record written', json.loads((sc.config / 'win11-gnome/last-os-release.json').read_text())['versionId'] == '26.10')])
    if want('error-later'):
        sc = Scenario(base, 'error-later')
        sc.spec = {'extensionState': 3.0, 'answer': 'action:later'}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        r.check('error-later', [('rc 0', rc == 0), ('extError', len(n) == 1 and n[0]['args']['summary'] == 'Win11 Shell kon niet starten'),
                                ('body', n and n[0]['args']['body'] == 'GNOME meldt een fout in Win11 Shell. Zet de Ubuntu Dock aan of installeer Win11 Shell opnieuw.'),
                                ('later does nothing', 'EnableExtension' not in methods(calls))])
    if want('state-missing'):
        sc = Scenario(base, 'state-missing')
        sc.spec = {'extensionState': None}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        r.check('state-missing', [('rc 0', rc == 0), ('missing state counts as ERROR', len(n) == 1 and n[0]['args']['summary'] == 'Win11 Shell kon niet starten')])
    if want('state-not-a-number'):
        sc = Scenario(base, 'state-not-a-number')
        sc.spec = {'extensionState': '1'}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        r.check('state-not-a-number', [('rc 0', rc == 0), ('a string state counts as ERROR', len(n) == 1 and n[0]['args']['summary'] == 'Win11 Shell kon niet starten')])
    if want('extensions-off'):
        sc = Scenario(base, 'extensions-off', 'en')
        sc.shell_keys(True, [UUID])
        sc.spec = {'extensionState': 1.0, 'answer': 'action:extsOn'}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        keyfile = sc.keyfile.read_text()
        r.check('extensions-off', [
            ('rc 0', rc == 0), ('extsOff', len(n) == 1 and n[0]['args']['summary'] == 'GNOME turned extensions off'),
            ('body', n and n[0]['args']['body'] == 'After an error, GNOME turned off all extensions, including Win11 Shell.'),
            ('actions', n and n[0]['args']['actions'] == ['extsOn', 'Turn extensions back on', 'dockOn', 'Turn on Ubuntu Dock']),
            ('disable-user-extensions now false', 'disable-user-extensions=false' in keyfile),
            ('no GetExtensionInfo, no EnableExtension', not ({'GetExtensionInfo', 'EnableExtension'} & set(methods(calls))))])
    if want('missing-parts'):
        sc = Scenario(base, 'missing-parts')
        for so in ('libsoup-3.0.so.0', 'libgdk-3.so.0'):
            (sc.root / 'usr/lib/x86_64-linux-gnu' / so).unlink()
        sc.spec = {'extensionState': 1.0, 'answers': ['close', 'action:installParts']}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        units = [c for c in calls if c['method'] == 'StartTransientUnit']
        unit = units[0]['args'] if units else {}
        props = unit.get('properties', {})
        exec_start = props.get('ExecStartEx') or [[None, [], []]]
        argv = list(exec_start[0][1])
        envs = props.get('Environment', [])
        tools, home = sc.tools, sc.root / 'home'
        want_argv = [str(tools / 'xdg-terminal-exec'), '--', str(tools / 'sh'), '-c', wrapper, 'win11-gnome-parts',
                     str(home / '.run/win11-gnome/parts-status.json'), 'always', str(tools / 'pkexec'), str(tools / 'apt-get'),
                     'install', 'libgtk-3-0t65', 'libsoup-3.0-0']
        apt = [line for line in sc.tool_log() if line[0] == 'apt-cache']
        r.check(f'missing-parts [{took:.2f} s]', [
            ('rc 0', rc == 0), ('two notifications', len(n) == 2),
            ('missing title', len(n) == 2 and n[1]['args']['summary'] == 'Win11 Shell mist onderdelen'),
            ('missing body', len(n) == 2 and n[1]['args']['body'] == 'Na de upgrade ontbreekt: GTK 3, libsoup.'),
            ('actions', len(n) == 2 and n[1]['args']['actions'] == ['installParts', 'Onderdelen installeren…', 'later', 'Later']),
            ('names resolved on the new release', apt == [['apt-cache', 'policy', 'libgtk-3-0t64'], ['apt-cache', 'showpkg', 'libgtk-3-0t64'],
                                                          ['apt-cache', 'policy', 'libsoup-3.0-0']]),
            ('one transient unit', len(units) == 1),
            ('Role::App: app.slice', props.get('Slice') == 'app.slice'),
            ('never the shell slice', not any(u['args'].get('name') == 'app-win11shell.slice' for u in units)),
            ('unit name', unit.get('name', '').startswith('app-win11-post-upgrade-parts-')),
            ('argv', argv == want_argv),
            ('program', exec_start[0][0] == str(tools / 'xdg-terminal-exec')),
            ('LANGUAGE', 'LANGUAGE=nl:en' in envs),
            ('WIN11_PRESS_ENTER', 'WIN11_PRESS_ENTER=Druk op Enter om dit venster te sluiten.' in envs),
            ('activation token', 'XDG_ACTIVATION_TOKEN=token-2' in envs and 'DESKTOP_STARTUP_ID=token-2' in envs),
            ('test PATH only', f'PATH={tools}' in envs),
            ('no seams in the terminal', not any(e.startswith('WIN11_UPDATE_') for e in envs)),
            ('cwd $HOME', props.get('WorkingDirectory') == str(home)),
            ('nothing ran directly', not any(line[0] in ('xdg-terminal-exec', 'pkexec', 'apt-get', 'sh') for line in sc.tool_log()))])
    if want('shell-absent-dpkg'):
        sc = Scenario(base, 'shell-absent-dpkg', 'en')
        stub(sc.tools, 'dpkg-query', "printf '%s' '1:51.0-0ubuntu2'\n")
        sc.spec = {'shellVersion': None, 'extensionState': 4.0, 'answer': 'close'}
        rc, took, calls = r.launch(sc)
        n = notifies(calls)
        r.check(f'shell-absent-dpkg [{took:.2f} s]', [('rc 0', rc == 0), (f'retried ShellVersion ({took:.1f} s)', 2 <= took < 8),
                                      ('dpkg-query fallback', ['dpkg-query', '-W', '-f=${Version}', 'gnome-shell'] in sc.tool_log()),
                                      ('major from dpkg-query', len(n) == 1 and n[0]['args']['summary'] == 'GNOME 51 turned Win11 Shell off')])
    if want('upgrader-running'):
        sc = Scenario(base, 'upgrader-running')
        write(sc.root / 'proc/77/cmdline', 'python3\0-s\0/tmp/ubuntu-release-upgrader-x1/stonking\0')
        write(sc.stamp(), '{}', 0o600)
        rc, took, calls = r.launch(sc)
        r.check('upgrader-running', [('rc 0', rc == 0), ('nothing on the bus', calls == []), ('no tool', sc.tool_log() == []),
                                     ('record kept', json.loads((sc.config / 'win11-gnome/last-os-release.json').read_text())['versionId'] == '26.04'),
                                     ('linger kept', sc.stamp().exists())])
    if want('seam-refused'):
        sc = Scenario(base, 'seam-refused')
        rc, took, calls = r.launch(sc, {'WIN11_UPDATE_TYPO': '1'})
        r.check('seam-refused', [('rc 0', rc == 0), ('nothing on the bus', calls == []), ('no tool', sc.tool_log() == []),
                                 ('record kept', json.loads((sc.config / 'win11-gnome/last-os-release.json').read_text())['versionId'] == '26.04')])
    if want('no-answer'):
        sc = Scenario(base, 'no-answer')
        sc.spec = {'extensionState': 4.0, 'answer': 'none'}
        rc, took, calls = r.launch(sc)
        r.check(f'no-answer [{took:.2f} s]', [('rc 0', rc == 0), (f'waits the (test) limit ({took:.1f} s)', 7.5 <= took < 12),
                              ('no EnableExtension', 'EnableExtension' not in methods(calls)),
                              ('record written after the wait', json.loads((sc.config / 'win11-gnome/last-os-release.json').read_text())['versionId'] == '26.10')])


def lib_start(r: Run, base: Path) -> None:
    """The release start of the lib (upgrade.rs): its process_scope::spawn reaches the fake user manager."""
    mark = r.mark()
    env = dict(os.environ)
    env['DBUS_SESSION_BUS_ADDRESS'] = r.bus
    name = 'services::update::upgrade::tests::layer_start_goes_through_the_user_manager'
    proc = subprocess.run([str(W / r.args.lib), name, '--exact', '--ignored', '--test-threads=1', '--nocapture'], env=env,
                          stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=300, cwd=str(W))
    (base / 'lib-start.out').write_text(proc.stdout + proc.stderr)
    time.sleep(0.3)
    calls = r.since(mark)
    m = re.search(r'GW7-LAYER base=(\S+)', proc.stdout + proc.stderr)
    units = [c for c in calls if c['method'] == 'StartTransientUnit']
    props = units[0]['args'].get('properties', {}) if units else {}
    exec_start = props.get('ExecStartEx') or [[None, [], []]]
    envs = props.get('Environment', [])
    ok_run = proc.returncode == 0 and 'test result: ok. 1 passed' in proc.stdout
    checks = [('the lib test ran and passed', ok_run), ('fixture path printed', bool(m)), ('one transient unit', len(units) == 1)]
    if m and units:
        fb = Path(m.group(1))
        tools, home = fb / 'tools', fb / 'root/home'
        want_argv = [str(tools / 'xdg-terminal-exec'), '--title=Ubuntu-upgrade', '--', str(tools / 'sh'), '-c', wrapper_text(),
                     'win11-gnome-upgrade', str(home / '.run/win11-gnome/upgrade-status.json'), 'fail', str(tools / 'pkexec'),
                     str(tools / 'do-release-upgrade')]
        checks += [('Role::App: app.slice', props.get('Slice') == 'app.slice'),
                   ('unit name', units[0]['args'].get('name', '').startswith('app-win11-release-upgrade-')),
                   ('mode fail', units[0]['args'].get('mode') == 'fail'),
                   ('argv', list(exec_start[0][1]) == want_argv),
                   ('LANGUAGE', 'LANGUAGE=nl:en' in envs),
                   ('WIN11_PRESS_ENTER', 'WIN11_PRESS_ENTER=Druk op Enter om dit venster te sluiten.' in envs),
                   ('cwd $HOME', props.get('WorkingDirectory') == str(home)),
                   ('no seams in the terminal', not any(e.startswith('WIN11_UPDATE_') for e in envs))]
    r.check('lib-start-app-slice', checks)


def cmd_inner(a) -> int:
    problems = layer_problems()
    if problems:
        return refuse(', '.join(problems))
    run = Path(os.environ['WIN11_FAKE_RUN_DIR'])
    base = private(run / 'gw7')
    procs: list = []
    try:
        bus = start_bus(base / 'bus.log', procs)
        calls = base / 'calls.jsonl'
        calls.write_text('')
        scenario_file = base / 'scenario.json'
        scenario_file.write_text('{}')
        ready = base / 'serve.ready'
        env = dict(os.environ)
        env['DBUS_SESSION_BUS_ADDRESS'] = bus
        procs.append(subprocess.Popen([sys.executable, '-I', '-B', str(Path(__file__).resolve()), 'serve', '--bus', bus,
                                       '--log', str(calls), '--scenario', str(scenario_file), '--ready', str(ready)],
                                      env=env, stdin=subprocess.DEVNULL, stdout=open(base / 'serve.log', 'w'),
                                      stderr=subprocess.STDOUT))
        deadline = time.monotonic() + 15
        while not ready.exists():
            if time.monotonic() > deadline or procs[-1].poll() is not None:
                raise RuntimeError(f'the fake GNOME services did not start ({base / "serve.log"})')
            time.sleep(0.05)
        r = Run(a, bus, calls, scenario_file)
        scenarios(r, base, a.only or [])
        if a.lib and (not a.only or 'lib-start-app-slice' in a.only):
            lib_start(r, base)
        print(f'test_post_upgrade: {r.passed} passed, {r.failed} failed (logs in {base})')
        return 0 if r.failed == 0 and r.passed > 0 else 1
    finally:
        for p in reversed(procs):
            p.terminate()
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                p.kill()


def cmd_run(a) -> int:
    inner = ['/usr/bin/python3', '-I', '-B', str(Path(__file__).resolve()), 'inner', '--launch', a.launch]
    if a.lib:
        inner += ['--lib', a.lib]
    if a.only:
        inner += ['--only', *a.only]
    return subprocess.run([str(HERE / 'run.sh'), 'exec', '--allow-layer-dgrams', '--timeout', '900', '--', *inner],
                          cwd=str(W)).returncode


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    sub = p.add_subparsers(dest='mode', required=True)
    for name in ('run', 'inner'):
        s = sub.add_parser(name)
        s.add_argument('--launch', required=True, help='win11-launch, relative to the worktree')
        s.add_argument('--lib', help='the lib test binary, relative to the worktree')
        s.add_argument('--only', nargs='*')
    s = sub.add_parser('serve')
    for opt in ('--bus', '--log', '--scenario', '--ready'):
        s.add_argument(opt, required=True)
    a = p.parse_args()
    if a.mode == 'serve':
        if layer_problems():
            return refuse(', '.join(layer_problems()))
        if not a.bus.startswith(f'unix:path={RT}/dbus-'):
            return refuse('not the layer bus')
        return cmd_serve(a)
    return cmd_inner(a) if a.mode == 'inner' else cmd_run(a)


if __name__ == '__main__':
    sys.exit(main())
