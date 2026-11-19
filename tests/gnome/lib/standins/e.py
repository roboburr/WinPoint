"""Batch E's D-Bus stand-ins and scenario helpers for its settings-parity scenarios (notes/claude/settings-parity.md
§7): tests/gnome/scenarios/settings-accounts.py and settings-time.py. Only batch E edits this file.

The stand-ins run on a server of batch E's own (``start_live``), next to B0's frozen builder (``start`` in
``__init__.py``), because the daemons batch E's pages talk to keep state and send signals: AccountsService emits a
user's Changed after a Set*, timedated, hostnamed and localed send PropertiesChanged after a write (WinPoint's host
turns that into ``machine.changed``), fprintd reports an enrolment with EnrollStatus, and a PackageKit transaction
answers with Package and Finished. A spec is B0's ``Spec(bus, name, objects)`` with GVariant text for the values:

    {path: {iface: {
        'props':    {name: [signature, 'GVariant text']},     # exported (Get, GetAll, PropertiesChanged)
        'hidden':   {name: [signature, 'GVariant text']},     # state only, for replyProp
        'writable': [name, ...],                              # Properties.Set allowed (recorded as 'Set:<name>')
        'signals':  {name: signature},
        'methods':  {name: {
            'in': signature, 'out': signature,
            'reply': 'GVariant text of the out tuple',        # 'replies': {arguments text: reply text} first
            'replyProp': name,                                # the reply is (state[name],)
            'error': [D-Bus error name, message],
            'set': {name or '/path#iface#name': argument index or 'GVariant text'},   # then PropertiesChanged
            'emit': [{'after': ms, 'signal': name, 'sig': signature, 'args': text, 'path'?: p, 'iface'?: i},
                     {'after': ms, 'props': {name: text}}],
        }}}}}

Every call is recorded as B0's builder records it ({'path', 'iface', 'method', 'args', 'interactive'}; interactive is
the message's ALLOW_INTERACTIVE_AUTHORIZATION flag). A Properties.Set is recorded as 'Set:<name>' with interactive
None: GDBus hands the new value to the server without the message.
"""
from __future__ import annotations

import os
import select
import subprocess
import time
from pathlib import Path

from . import Spec, StandIn

SPECS: dict[str, "Spec"] = {}

LIVE_SERVER = r'''
import json, sys, warnings
warnings.simplefilter('ignore', DeprecationWarning)
from gi.repository import Gio, GLib
spec = json.loads(sys.argv[1])
log = open(sys.argv[2], 'a', buffering=1)
FLAG = Gio.DBusMessageFlags.ALLOW_INTERACTIVE_AUTHORIZATION
PROPS = 'org.freedesktop.DBus.Properties'
bus = Gio.bus_get_sync(Gio.BusType.SYSTEM if spec['bus'] == 'system' else Gio.BusType.SESSION, None)
def types(sig):
    out, i = [], 0
    while i < len(sig):
        j = i
        while sig[j] == 'a':
            j += 1
        if sig[j] in '({':
            depth = 0
            while True:
                depth += {'(': 1, '{': 1, ')': -1, '}': -1}.get(sig[j], 0)
                j += 1
                if depth == 0:
                    break
        else:
            j += 1
        out.append(sig[i:j])
        i = j
    return out
def parse(sig, text):
    return GLib.Variant.parse(GLib.VariantType.new(sig), text, None, None)
def note(path, iface, method, args, interactive):
    log.write(json.dumps({'path': path, 'iface': iface, 'method': method, 'args': args, 'interactive': interactive}) + '\n')
state, sigs = {}, {}
for path, ifaces in spec['objects'].items():
    for iface, desc in ifaces.items():
        for group in ('props', 'hidden'):
            for name, (sig, text) in desc.get(group, {}).items():
                state.setdefault(path, {}).setdefault(iface, {})[name] = parse(sig, text)
                sigs.setdefault(path, {}).setdefault(iface, {})[name] = sig
def xml_of(iface, desc):
    out = [f'<interface name="{iface}">']
    for name, m in desc.get('methods', {}).items():
        out.append(f'<method name="{name}">')
        out += [f'<arg type="{t}" direction="in"/>' for t in types(m.get('in', ''))]
        out += [f'<arg type="{t}" direction="out"/>' for t in types(m.get('out', ''))]
        out.append('</method>')
    for name, (sig, _) in desc.get('props', {}).items():
        access = 'readwrite' if name in desc.get('writable', []) else 'read'
        out.append(f'<property name="{name}" type="{sig}" access="{access}"/>')
    for name, sig in desc.get('signals', {}).items():
        out.append(f'<signal name="{name}">' + ''.join(f'<arg type="{t}"/>' for t in types(sig)) + '</signal>')
    out.append('</interface>')
    return ''.join(out)
def changed(path, iface, names):
    exported = spec['objects'][path][iface].get('props', {})
    values = {n: state[path][iface][n] for n in names if n in exported}
    if values:
        bus.emit_signal(None, path, PROPS, 'PropertiesChanged', GLib.Variant('(sa{sv}as)', (iface, values, [])))
def later(ms, fn):
    def run():
        try:
            fn()
        except Exception as e:  # noqa: BLE001 - the stand-in keeps serving; the error is in its call log
            note('', '', 'standin-error', repr(e), None)
        return False
    GLib.timeout_add(max(1, int(ms)), run)
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
        args = params.print_(True)
        note(obj, iname, method, args, bool(invocation.get_message().get_flags() & FLAG))
        m = desc.get('methods', {}).get(method, {})
        if 'error' in m:
            invocation.return_dbus_error(m['error'][0], m['error'][1])
            return
        touched = {}
        for target, src in m.get('set', {}).items():
            tpath, tiface, name = target.split('#') if '#' in target else (obj, iname, target)
            value = params.get_child_value(src) if isinstance(src, int) else parse(sigs[tpath][tiface][name], src)
            state[tpath][tiface][name] = value
            touched.setdefault((tpath, tiface), []).append(name)
        out = m.get('out', '')
        if 'replyProp' in m:
            reply = GLib.Variant.new_tuple(state[obj][iname][m['replyProp']])
        elif args in m.get('replies', {}):
            reply = parse('(' + out + ')', m['replies'][args])
        elif m.get('reply') is not None:
            reply = parse('(' + out + ')', m['reply'])
        elif out and ('value' in m or 'byArgs' in m):
            values = m.get('byArgs', {}).get(args, m.get('value', []))
            reply = GLib.Variant('(' + out + ')', tuple(conv(v) for v in values))
        else:
            reply = None
        invocation.return_value(reply)
        for (tpath, tiface), names in touched.items():
            later(1, lambda p=tpath, i=tiface, n=names: changed(p, i, n))
        for ev in m.get('emit', []):
            epath, eiface = ev.get('path') or obj, ev.get('iface') or iname
            if 'props' in ev:
                def props(p=epath, i=eiface, values=ev['props']):
                    for name, text in values.items():
                        state[p][i][name] = parse(sigs[p][i][name], text)
                    changed(p, i, list(values))
                later(ev.get('after', 1), props)
            else:
                body = parse('(' + ev['sig'] + ')', ev['args']) if ev.get('sig') else None
                later(ev.get('after', 1), lambda p=epath, i=eiface, n=ev['signal'], b=body: bus.emit_signal(None, p, i, n, b))
    def get(conn, sender, obj, iname, name):
        return state[obj][iname][name]
    def set_(conn, sender, obj, iname, name, value):
        note(obj, iname, 'Set:' + name, value.print_(True), None)
        if name in desc.get('writable', []):
            state[obj][iname][name] = value
            later(1, lambda: changed(obj, iname, [name]))
            return True
        return False
    return call, get, set_
for path, ifaces in spec['objects'].items():
    for iface, desc in ifaces.items():
        info = Gio.DBusNodeInfo.new_for_xml('<node>' + xml_of(iface, desc) + '</node>').interfaces[0]
        bus.register_object(path, info, *handler(path, iface, desc))
loop = GLib.MainLoop()
Gio.bus_own_name_on_connection(bus, spec['name'], Gio.BusNameOwnerFlags.NONE,
                               lambda *a: print('owned', flush=True), lambda *a: loop.quit())
loop.run()
'''


def start_live(h, spec: Spec, tag: str, timeout: float = 10) -> StandIn | None:
    """B0's ``start`` with batch E's server: a child of the harness on its private bus, waited for until it owns its
    name; recorded as ``standin.<tag>`` in the results, or a failed check and None."""
    logs = h.hm.RUN / 'logs'
    calls = logs / f'standin-{tag}.calls.jsonl'
    calls.write_text('')
    err = open(logs / f'standin-{tag}.log', 'w')
    proc = subprocess.Popen(['python3', '-I', '-c', LIVE_SERVER, spec.json(), str(calls)], stdout=subprocess.PIPE,
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


def _q(text: str) -> str:
    """A GVariant string literal."""
    return "'" + text.replace('\\', '\\\\').replace("'", "\\'") + "'"


# ------------------------------------------------------------------------------------------------ AccountsService

ACCOUNTS = 'org.freedesktop.Accounts'
USER = 'org.freedesktop.Accounts.User'
CHANGED = [{'after': 1, 'signal': 'Changed', 'sig': '', 'args': ''}]


def user_path(uid: int) -> str:
    return f'/org/freedesktop/Accounts/User{uid}'


# the account's language: the live session's (the harness, as Robin's GNOME, runs LANG=en_US.UTF-8, plan D-G3; the
# sandbox's locale archive is the host's, which has only English locales), so no page sees a language change.
# LANGUAGE is the generated locale the pages show and send; STORED_LANGUAGE what Ubuntu's accounts-daemon keeps of it
# (language-validate drops the codeset: "en_US"), which the host resolves to LANGUAGE (review of batch E, finding 5)
LANGUAGE = 'en_US.UTF-8'
STORED_LANGUAGE = 'en_US'


def _user(uid: int, name: str, real: str, kind: int, language: str = STORED_LANGUAGE) -> dict:
    def setter(sig, prop):
        return {'in': sig, 'out': '', 'set': {prop: 0}, 'emit': CHANGED}
    return {USER: {
        'props': {'Uid': ['t', str(uid)], 'UserName': ['s', _q(name)], 'RealName': ['s', _q(real)],
                  'AccountType': ['i', str(kind)], 'IconFile': ['s', "''"], 'AutomaticLogin': ['b', 'false'],
                  'PasswordMode': ['i', '0'], 'Language': ['s', _q(language)], 'FormatsLocale': ['s', "''"],
                  'Locked': ['b', 'false'], 'LocalAccount': ['b', 'true'], 'SystemAccount': ['b', 'false'],
                  'HomeDirectory': ['s', _q(f'/home/{name}')], 'Shell': ['s', "'/bin/bash'"],
                  'LoginTime': ['x', '0'], 'PasswordHint': ['s', "''"]},
        'methods': {
            'SetRealName': setter('s', 'RealName'), 'SetIconFile': setter('s', 'IconFile'),
            'SetLanguage': setter('s', 'Language'), 'SetFormatsLocale': setter('s', 'FormatsLocale'),
            'SetAccountType': setter('i', 'AccountType'), 'SetAutomaticLogin': setter('b', 'AutomaticLogin'),
            'SetPasswordMode': setter('i', 'PasswordMode'),
            'SetPassword': {'in': 'ss', 'out': '', 'set': {'PasswordMode': '0'}, 'emit': CHANGED},
        },
        'signals': {'Changed': ''},
    }}


def accounts(uid: int) -> Spec:
    """AccountsService for users.*: the own account (uid, "alex", an administrator), "sam" (uid + 1, a standard user)
    and "kim" (uid + 2), whom CreateUser makes (ListCachedUsers lists kim after it, and no longer after DeleteUser).
    The names have no Dutch word, so the en run's Dutch-word check sees only WinPoint's texts."""
    root = '/org/freedesktop/Accounts'
    me, sam, kim = user_path(uid), user_path(uid + 1), user_path(uid + 2)
    return Spec('system', ACCOUNTS, {
        root: {ACCOUNTS: {
            'hidden': {'cached': ['ao', f'[objectpath {_q(sam)}]']},
            'methods': {
                'FindUserById': {'in': 'x', 'out': 'o', 'reply': f'(objectpath {_q(me)},)', 'replies': {
                    f'(int64 {u},)': f'(objectpath {_q(user_path(u))},)' for u in (uid, uid + 1, uid + 2)}},
                'ListCachedUsers': {'in': '', 'out': 'ao', 'replyProp': 'cached'},
                'CreateUser': {'in': 'ssi', 'out': 'o', 'reply': f'(objectpath {_q(kim)},)',
                               'set': {'cached': f'[objectpath {_q(sam)}, {_q(kim)}]'},
                               'emit': [{'after': 1, 'signal': 'UserAdded', 'sig': 'o', 'args': f'(objectpath {_q(kim)},)'}]},
                'DeleteUser': {'in': 'xb', 'out': '', 'set': {'cached': f'[objectpath {_q(sam)}]'},
                               'emit': [{'after': 1, 'signal': 'UserDeleted', 'sig': 'o', 'args': f'(objectpath {_q(kim)},)'}]},
            },
            'signals': {'UserAdded': 'o', 'UserDeleted': 'o'},
        }},
        me: _user(uid, 'alex', 'Alex Morgan', 1),
        sam: _user(uid + 1, 'sam', 'Sam Taylor', 0),
        kim: _user(uid + 2, 'kim', 'Kim Lee', 0),
    })


# ---------------------------------------------------------------------------------------------------------- logind

LOGIN = 'org.freedesktop.login1'


def logind(sessions: list[tuple[int, str]]) -> Spec:
    """systemd-logind for users.* (who is logged in, as libaccountsservice asks): ListSessions lists one session per
    (uid, user name), each of class "user" and state "online" (a user behind Gebruiker wisselen)."""
    rows, objects = [], {}
    for i, (uid, name) in enumerate(sessions):
        path = f'/org/freedesktop/login1/session/_3{i}'
        rows.append(f"('{30 + i}', uint32 {uid}, {_q(name)}, 'seat0', objectpath {_q(path)})")
        objects[path] = {'org.freedesktop.login1.Session': {
            'props': {'Id': ['s', _q(str(30 + i))], 'Class': ['s', "'user'"], 'State': ['s', "'online'"],
                      'Active': ['b', 'false']}}}
    objects['/org/freedesktop/login1'] = {'org.freedesktop.login1.Manager': {'methods': {
        'ListSessions': {'in': '', 'out': 'a(susso)', 'reply': f"([{', '.join(rows)}],)" if rows else '(@a(susso) [],)'}}}}
    return Spec('system', LOGIN, objects)


# -------------------------------------------------------------------------------------------------------- fprintd

FPRINT = 'net.reactivated.Fprint'
FP_DEVICE = '/net/reactivated/Fprint/Device/0'
FP_IFACE = 'net.reactivated.Fprint.Device'


def fprintd(stages: int = 3) -> Spec:
    """fprintd with one press reader that has the right index finger enrolled. EnrollStart passes ``stages`` - 1
    stages and then completes (EnrollStatus, as a reader does) and adds the left index finger;
    DeleteEnrolledFingers2 empties the list."""
    statuses = [{'after': 400 * (i + 1), 'signal': 'EnrollStatus', 'sig': 'sb', 'args': "('enroll-stage-passed', false)"}
                for i in range(stages - 1)]
    statuses.append({'after': 400 * stages, 'signal': 'EnrollStatus', 'sig': 'sb', 'args': "('enroll-completed', true)"})
    return Spec('system', FPRINT, {
        '/net/reactivated/Fprint/Manager': {'net.reactivated.Fprint.Manager': {'methods': {
            'GetDevices': {'in': '', 'out': 'ao', 'reply': f'([objectpath {_q(FP_DEVICE)}],)'},
            'GetDefaultDevice': {'in': '', 'out': 'o', 'reply': f'(objectpath {_q(FP_DEVICE)},)'}}}},
        FP_DEVICE: {FP_IFACE: {
            'props': {'name': ['s', "'WinPoint Test Reader'"], 'num-enroll-stages': ['i', str(stages)],
                      'scan-type': ['s', "'press'"], 'finger-present': ['b', 'false'], 'finger-needed': ['b', 'false']},
            'hidden': {'enrolled': ['as', "['right-index-finger']"]},
            'methods': {
                'ListEnrolledFingers': {'in': 's', 'out': 'as', 'replyProp': 'enrolled'},
                'Claim': {'in': 's', 'out': ''}, 'Release': {'in': '', 'out': ''},
                'EnrollStart': {'in': 's', 'out': '', 'emit': statuses,
                                'set': {'enrolled': "['right-index-finger', 'left-index-finger']"}},
                'EnrollStop': {'in': '', 'out': ''},
                'DeleteEnrolledFingers2': {'in': '', 'out': '', 'set': {'enrolled': '@as []'}},
            },
            'signals': {'EnrollStatus': 'sb'},
        }}})


# ------------------------------------------------------------------------------------------------------------- GOA

GOA = 'org.gnome.OnlineAccounts'
GOA_ROOT = '/org/gnome/OnlineAccounts'
GOA_ACCOUNT = '/org/gnome/OnlineAccounts/Accounts/account_1_0'
GOA_IFACE = 'org.gnome.OnlineAccounts.Account'
OBJECT_MANAGER = 'org.freedesktop.DBus.ObjectManager'


def goa() -> Spec:
    """goa-daemon with one Google account (alex@example.org) used for mail and contacts, its calendar switched off;
    Remove takes it out of GetManagedObjects and sends InterfacesRemoved."""
    props = {'Id': "'account_1_0'", 'ProviderType': "'google'", 'ProviderName': "'Google'",
             'ProviderIcon': "''", 'Identity': "'alex@example.org'", 'PresentationIdentity': "'alex@example.org'",
             'MailDisabled': 'false', 'CalendarDisabled': 'true', 'ContactsDisabled': 'false',
             'AttentionNeeded': 'false', 'IsLocked': 'false', 'IsTemporary': 'false'}
    sig = {k: ('b' if v in ('true', 'false') else 's') for k, v in props.items()}
    account = '{' + ', '.join(f'{_q(k)}: <{v}>' for k, v in props.items()) + '}'
    objects = (f'{{objectpath {_q(GOA_ACCOUNT)}: {{{_q(GOA_IFACE)}: {account}, '
               "'org.gnome.OnlineAccounts.Mail': @a{sv} {}, 'org.gnome.OnlineAccounts.Contacts': @a{sv} {}}}")
    return Spec('session', GOA, {
        GOA_ROOT: {OBJECT_MANAGER: {
            'hidden': {'objects': ['a{oa{sa{sv}}}', objects]},
            'methods': {'GetManagedObjects': {'in': '', 'out': 'a{oa{sa{sv}}}', 'replyProp': 'objects'}},
            'signals': {'InterfacesAdded': 'oa{sa{sv}}', 'InterfacesRemoved': 'oas'},
        }},
        GOA_ACCOUNT: {GOA_IFACE: {
            'props': {k: [sig[k], v] for k, v in props.items()},
            'writable': ['MailDisabled', 'CalendarDisabled', 'ContactsDisabled'],
            'methods': {
                'EnsureCredentials': {'in': '', 'out': 'i', 'reply': '(3600,)'},
                'Remove': {'in': '', 'out': '', 'set': {f'{GOA_ROOT}#{OBJECT_MANAGER}#objects': '@a{oa{sa{sv}}} {}'},
                           'emit': [{'after': 1, 'path': GOA_ROOT, 'iface': OBJECT_MANAGER, 'signal': 'InterfacesRemoved',
                                     'sig': 'oas', 'args': f'(objectpath {_q(GOA_ACCOUNT)}, [{_q(GOA_IFACE)}])'}]},
            },
        }}})


# --------------------------------------------------------------------------------- timedated, hostnamed, localed

def timedate1(ntp: bool = True) -> Spec:
    """systemd-timedated for machine.time.*: Europe/Amsterdam, NTP on and synchronised. SetNTP and SetTimezone change
    the properties and send PropertiesChanged, as timedated does. There is no timesync1 here (amendment 30), so
    "Nu synchroniseren" may only send SetNTP(false) then SetNTP(true) (amendment 68)."""
    on = 'true' if ntp else 'false'
    return Spec('system', 'org.freedesktop.timedate1', {
        '/org/freedesktop/timedate1': {'org.freedesktop.timedate1': {
            'props': {'Timezone': ['s', "'Europe/Amsterdam'"], 'LocalRTC': ['b', 'false'], 'CanNTP': ['b', 'true'],
                      'NTP': ['b', on], 'NTPSynchronized': ['b', on], 'TimeUSec': ['t', '0'], 'RTCTimeUSec': ['t', '0']},
            'methods': {'SetTime': {'in': 'xbb', 'out': ''},
                        'SetTimezone': {'in': 'sb', 'out': '', 'set': {'Timezone': 0}},
                        'SetLocalRTC': {'in': 'bbb', 'out': ''},
                        'SetNTP': {'in': 'bb', 'out': '', 'set': {'NTP': 0, 'NTPSynchronized': 0}},
                        'ListTimezones': {'in': '', 'out': 'as', 'reply': "(['Europe/Amsterdam', 'UTC'],)"}},
        }}})


def hostname1() -> Spec:
    """systemd-hostnamed for machine.info and machine.setName (names without a Dutch word, for the en run); the
    Set*Hostname methods change the names and send PropertiesChanged, as hostnamed does."""
    return Spec('system', 'org.freedesktop.hostname1', {
        '/org/freedesktop/hostname1': {'org.freedesktop.hostname1': {
            'props': {'Hostname': ['s', "'winpoint-test'"], 'StaticHostname': ['s', "'winpoint-test'"],
                      'PrettyHostname': ['s', "'WinPoint Test'"], 'Chassis': ['s', "'vm'"],
                      'HardwareVendor': ['s', "'QEMU'"], 'HardwareModel': ['s', "'Standard PC'"],
                      'FirmwareVersion': ['s', "'1.16.3'"]},
            'methods': {'SetPrettyHostname': {'in': 'sb', 'out': '', 'set': {'PrettyHostname': 0}},
                        'SetStaticHostname': {'in': 'sb', 'out': '', 'set': {'StaticHostname': 0, 'Hostname': 0}},
                        'SetHostname': {'in': 'sb', 'out': '', 'set': {'Hostname': 0}}},
        }}})


def locale1() -> Spec:
    """systemd-localed for machine.login.*: the sign-in screen in en_US.UTF-8; SetLocale changes Locale."""
    return Spec('system', 'org.freedesktop.locale1', {
        '/org/freedesktop/locale1': {'org.freedesktop.locale1': {
            'props': {'Locale': ['as', "['LANG=en_US.UTF-8']"], 'X11Layout': ['s', "'us'"],
                      'VConsoleKeymap': ['s', "'us'"]},
            'methods': {'SetLocale': {'in': 'asb', 'out': '', 'set': {'Locale': 0}}},
        }}})


# ------------------------------------------------------------------------------------------------------- PackageKit

PK = 'org.freedesktop.PackageKit'
PK_TX = '/1_winpoint'
PK_TX_IFACE = 'org.freedesktop.PackageKit.Transaction'
PK_PACKAGE = 'language-pack-de;1:26.04+20260401;all;ubuntu'
# a multiarch system's foreign build that Resolve also reports: never installed (dpkg's architecture is amd64 here)
PK_FOREIGN = 'libenchant-2-voikko;2.8.2+dfsg1-3build1;i386;ubuntu'


def packagekit(package: str = PK_PACKAGE) -> Spec:
    """packagekitd for machine.languages.install: CreateTransaction always gives the same transaction object; its
    Resolve reports ``package`` and an i386 build (PK_FOREIGN) as available (PK_INFO_ENUM_AVAILABLE 2) and finishes,
    InstallPackages reports 40 %,
    then 100 %, and finishes with success (PK_EXIT_ENUM_SUCCESS 1)."""
    finished = {'signal': 'Finished', 'sig': 'uu', 'args': '(uint32 1, uint32 120)'}
    return Spec('system', PK, {
        '/org/freedesktop/PackageKit': {PK: {
            'props': {'VersionMajor': ['u', '1'], 'VersionMinor': ['u', '3'], 'Locked': ['b', 'false']},
            'methods': {'CreateTransaction': {'in': '', 'out': 'o', 'reply': f'(objectpath {_q(PK_TX)},)'}},
        }},
        PK_TX: {PK_TX_IFACE: {
            'props': {'Percentage': ['u', '101'], 'Status': ['u', '1']},
            'methods': {
                'SetHints': {'in': 'as', 'out': ''},
                'Resolve': {'in': 'tas', 'out': '', 'emit': [
                    {'after': 50, 'signal': 'Package', 'sig': 'uss', 'args': f"(uint32 2, {_q(package)}, 'Translations')"},
                    {'after': 60, 'signal': 'Package', 'sig': 'uss', 'args': f"(uint32 2, {_q(PK_FOREIGN)}, 'Spelling')"},
                    {'after': 100, **finished}]},
                'InstallPackages': {'in': 'tas', 'out': '', 'emit': [
                    {'after': 200, 'props': {'Percentage': 'uint32 40'}},
                    {'after': 2500, 'props': {'Percentage': 'uint32 100'}},
                    {'after': 2700, **finished}]},
            },
            'signals': {'Package': 'uss', 'Finished': 'uu', 'ErrorCode': 'us', 'ItemProgress': 'suu'},
        }}})


# ----------------------------------------------------------------------------------------------- scenario helpers

# registry.rs: (parent, nl, en) of batch E's pages; the breadcrumb is the parent chain
PAGES = {
    'system': (None, 'Systeem', 'System'),
    'about': ('system', 'Info', 'About'),
    'accounts': (None, 'Accounts', 'Accounts'),
    'yourinfo': ('accounts', 'Je info', 'Your info'),
    'email-accounts': ('accounts', 'E-mail en accounts', 'Email & accounts'),
    'signin': ('accounts', 'Aanmeldingsopties', 'Sign-in options'),
    'otherusers': ('accounts', 'Andere gebruikers', 'Other users'),
    'time': (None, 'Tijd en taal', 'Time & language'),
    'datetime': ('time', 'Datum en tijd', 'Date & time'),
    'language': ('time', 'Taal en regio', 'Language & region'),
    'formats': ('language', 'Regionale notatie', 'Regional format'),
    'language-admin': ('language', 'Beheerinstellingen voor taal', 'Administrative language settings'),
}

# the wheel over a window: a Clutter virtual pointer (the one scenario.py's clicks use) moved there, then notches
SCROLL_JS = (
    "(async()=>{{const Clutter=(await import('gi://Clutter')).default;const GLib=(await import('gi://GLib')).default;"
    "const wait=ms=>new Promise(r=>GLib.timeout_add(GLib.PRIORITY_DEFAULT,ms,()=>{{r();return GLib.SOURCE_REMOVE;}}));"
    "const seat=Clutter.get_default_backend().get_default_seat();"
    "globalThis.__ge1hPointer??=seat.create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);"
    "const d=globalThis.__ge1hPointer,us=()=>GLib.get_monotonic_time();"
    "d.notify_absolute_motion(us(),{x},{y});await wait(60);"
    "for(let i=0;i<{n};i++){{d.notify_discrete_scroll(us(),Clutter.ScrollDirection.{dir},Clutter.ScrollSource.WHEEL);"
    "await wait(40);}}return {n};}})()")
FRAME_JS = ("(()=>{const w=global.get_window_actors().map(a=>a.meta_window).find(w=>w.get_pid()===%d&&"
            "w.get_window_type()===Meta.WindowType.NORMAL&&!w.minimized);if(!w)return null;const r=w.get_frame_rect();"
            "return {x:r.x,y:r.y,w:r.width,h:r.height};})()")


def title(page_id: str, lang: str) -> str:
    return PAGES[page_id][2 if lang == 'en' else 1]


def chain(page_id: str) -> list[str]:
    out = [page_id]
    while PAGES.get(out[-1], (None,))[0]:
        out.append(PAGES[out[-1]][0])
    return out[::-1]


def tr(h, nl: str, en: str) -> str:
    return en if h.lang == 'en' else nl


def labels(h, n: dict) -> list[str]:
    return [str(k.get('label')) for k in h.hm.tree_nodes([{'tree': n}]) if k.get('label')]


def page(h, pid: str, shot: str | None = None) -> dict:
    """Open a page as Start's search does (h.open waits for its title), screenshot it, and check the title or the
    breadcrumb (the parent chain's crumb-<id> buttons and the current label), that the page is built (no B0
    placeholder) and, in en, that no Dutch word shows."""
    h.open(f'settings:{pid}')
    h.sleep(0.6)
    h.screenshot(shot or f'{pid}.{h.lang}', 'settings')
    text = h.page_text('settings')
    parents = chain(pid)[:-1]
    if parents:
        current = [n.get('label') for n in h.find('settings', cls='w11-breadcrumb-current')]
        crumbs = [(p, (h.find('settings', f'crumb-{p}') or [{}])[0].get('label')) for p in parents]
        h.check(f'{pid}: the breadcrumb is {" > ".join(title(p, h.lang) for p in chain(pid))}',
                current == [title(pid, h.lang)] and all(label == title(p, h.lang) for p, label in crumbs),
                current=current, crumbs=crumbs)
    else:
        heads = [n.get('label') for n in h.find('settings', cls='st-title')]
        h.check(f'{pid}: the title is {title(pid, h.lang)}', heads == [title(pid, h.lang)], heads=heads)
    h.check(f'{pid}: built, no placeholder', not h.find('settings', 'st-placeholder'))
    if h.lang == 'en':
        h.check(f'{pid}: no Dutch words in en', not text['dutchWordHits'], hits=text['dutchWordHits'])
    return text


def node(h, name: str, surface: str = 'settings') -> dict | None:
    found = [n for n in h.find(surface, name) if n.get('rect')] or h.find(surface, name)
    return found[0] if found else None


# Settings' fixed header (the title or breadcrumb) covers the page's top 140 px: a control under it is not reachable
TOP, BOTTOM = 170, 40


def wheel(h, x: int, y: int, down: bool, steps: int, surface: str = 'settings') -> None:
    """``steps`` notches of the real wheel at (x, y) of the surface's window."""
    frame = h.eval(FRAME_JS % h.surface(surface).pid)
    if frame:
        h.eval(SCROLL_JS.format(x=frame['x'] + x, y=frame['y'] + y, n=max(1, steps), dir='DOWN' if down else 'UP'))


def settled(h, name: str, surface: str = 'settings', timeout: float = 2.0) -> dict | None:
    """The widget's node once its place stops moving (a scroll or a page rebuild has ended)."""
    last, end = None, time.monotonic() + timeout
    while time.monotonic() < end:
        n = node(h, name, surface)
        rect = (n or {}).get('rect')
        if rect and rect == last:
            return n
        last = rect
        time.sleep(0.15)
    return node(h, name, surface)


def reveal(h, name: str, surface: str = 'settings', tries: int = 14, room: int = 0) -> dict | None:
    """Scroll the page with the real wheel (the virtual pointer over the content column) until the widget's centre is
    in the window below the fixed header, with ``room`` px below it (a drop-down's list opens there), as a user
    scrolls to a control; the node, or None when the page has no such widget."""
    for _ in range(tries):
        tops = h.state(surface)
        found = [n for n in h.find(surface, name, state=tops) if n.get('rect')]
        if not found:
            return node(h, name, surface)
        n = found[0]
        width = tops[n.get('top', 0)].get('width') or 0
        height = tops[n.get('top', 0)].get('height') or 0
        _, y, _, hh = n['rect']
        cy = y + hh // 2
        bottom = max(TOP + 40, height - BOTTOM - room)
        if not height or TOP <= cy <= bottom:
            return settled(h, name, surface) or n
        target = (TOP + bottom) // 2
        wheel(h, int(width * 0.66), height // 2, cy > bottom, max(1, min(8, abs(cy - target) // 80)), surface)
        h.sleep(0.3)
        settled(h, name, surface)
    return node(h, name, surface)


def press(h, name: str, surface: str = 'settings', room: int = 0) -> dict:
    """A real pointer click on the widget ``name`` after scrolling it into view (``h.click`` takes the dev channel only
    for a widget it cannot place); the click's record (``input`` 'clutter' for a pointer click)."""
    n = reveal(h, name, surface, room=room)
    if n is None:
        raise LookupError(f'{surface}: no visible widget {name!r}')
    return h.click(surface, node=n)


def expander_open(h, header: str) -> bool:
    """Whether the kit expander whose header is the widget ``header`` is open (its root carries w11-expanded; the
    children of a closed one stay in the dev state, as a GtkRevealer keeps them)."""
    holder = None
    for n in h.find('settings', cls='w11-expander'):
        if any(k.get('name') == header for k in h.hm.tree_nodes([{'tree': n}])):
            holder = n          # depth first: the innermost expander that holds it comes last
    return bool(holder) and 'w11-expanded' in (holder.get('classes') or [])


def expand(h, header: str, part: str, tries: int = 3) -> dict | None:
    """Open the expander ``header`` (a real click) until the widget ``part`` inside it can be clicked; a page that is
    built again (300 ms after users.changed, goa.changed or machine.changed) closes it, so this waits for a rebuild
    to pass and tries again. The settled node of ``part``, or None."""
    for _ in range(tries):
        h.sleep(0.8)
        if expander_open(h, header) and node(h, part):
            return reveal(h, part)
        if node(h, header) is None:
            return None
        press(h, header)
        if until(h, lambda: expander_open(h, header) or None, f'{header} opens', 5):
            h.sleep(0.5)    # the revealer's slide
            if expander_open(h, header) and node(h, part):
                return reveal(h, part)
    return None


def until(h, fn, label: str, timeout: float = 10):
    try:
        return h.until(fn, label, timeout)
    except TimeoutError:
        return None


def call_of(standin, method: str, after: int = 0, timeout: float = 10) -> dict | None:
    """The first call of ``method`` the stand-in got after its first ``after`` calls, waited for; None."""
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        call = next((c for c in standin.calls()[after:] if c['method'] == method), None)
        if call:
            return call
        time.sleep(0.2)
    return None


def writes(standin, after: int = 0) -> list[tuple]:
    """(method, args, interactive) of the stand-in's calls after its first ``after``, without the reads."""
    return [(c['method'], c['args'], c['interactive']) for c in standin.calls()[after:]
            if not c['method'].startswith(('Get', 'List', 'Find', 'org.', 'Introspect'))]


def written(h, schema: str, key: str, before, timeout: float = 10):
    """The keyfile value of ``key`` once it differs from ``before``; None."""
    return until(h, lambda: (lambda v: v if v not in (None, before) else None)(h.gsettings_get(schema, key)),
                 f'{key} written', timeout)


def active(h, name: str):
    n = node(h, name)
    return None if n is None else n.get('active')


def flip(h, page_id: str, name: str, schema: str, key: str, default: str) -> None:
    """A bound switch or checkbox by a real click: the keyfile gets GNOME's other boolean; then a write from outside
    (the keyfile, as another program writes it) moves the control back (gsettings.changed)."""
    before = h.gsettings_get(schema, key) or default
    want = 'false' if before == 'true' else 'true'
    if node(h, name) is None:
        h.check(f'{page_id}: {name} is on the page', False)
        return
    click = press(h, name)
    got = until(h, lambda: h.gsettings_get(schema, key) == want or None, f'{key} = {want}')
    h.check(f'{page_id}: a real click on {name} writes {key} {want}', bool(got) and click.get('input') == 'clutter',
            before=before, after=h.gsettings_get(schema, key), input=click.get('input'))
    h.gsettings_set(schema, key, before)
    follows = until(h, lambda: active(h, name) is (before == 'true') or None, f'{name} follows {key}')
    h.check(f'{page_id}: {name} follows {key} = {before} written from outside', bool(follows), active=active(h, name))


def popover_list(h) -> dict | None:
    """The scrolled list of an open drop-down's popover (its viewport rect in the window)."""
    for n in h.find('settings', where=lambda n: n.get('type') == 'GtkScrolledWindow' and n.get('rect')):
        if any('w11-list-item' in (k.get('classes') or []) for k in h.hm.tree_nodes([{'tree': n}])):
            return n
    return None


def _visible_options(h) -> list[dict]:
    """The open popover's options a pointer can reach: inside the popover's list and inside the window (the popover is
    a surface of its own and may reach past the window, and the harness clicks inside the window only), each
    settled (the popover's entrance has ended)."""
    height = next((t.get('height') for t in h.state('settings') if t.get('visible', True)), 0) or 10_000
    last, items, end = None, [], time.monotonic() + 2.0
    while time.monotonic() < end:
        items = h.find('settings', cls='w11-list-item')
        rects = [n.get('rect') for n in items]
        if rects == last:
            break
        last = rects
        time.sleep(0.15)
    view = popover_list(h)
    low, high = (view['rect'][1], view['rect'][1] + view['rect'][3]) if view else (0, height)
    high = min(high, height - 8)
    return [n for n in items if n.get('rect') and low <= n['rect'][1] and n['rect'][1] + n['rect'][3] <= high]


def choose(h, dropdown: str, starts: str | None = None, exact: bool = False, accept=None) -> dict | None:
    """Open the kit drop-down ``dropdown`` with a real click and click, also with the pointer, the first option the
    pointer can reach whose label starts with (or is) ``starts``, or for which ``accept(label)`` holds. The click's
    record with the option's ``text``, or None (with a failed check) when no such option can be reached."""
    def fits(n):
        text = ' '.join(labels(h, n))
        if accept is not None:
            return accept(text)
        return text == starts if exact else text.startswith(starts)
    # room below it for the list: a popover that does not fit under the window opens past it (or flipped)
    press(h, dropdown, room=330)
    if not until(h, lambda: h.find('settings', cls='w11-list-item'), f'{dropdown} open', 5):
        h.click('settings', dropdown, real=False)
        until(h, lambda: h.find('settings', cls='w11-list-item'), f'{dropdown} open (dev click)', 5)
    offered = [' '.join(labels(h, n)) for n in h.find('settings', cls='w11-list-item')]
    found = next((n for n in _visible_options(h) if fits(n)), None)
    if not found:
        h.check(f'{dropdown}: an option {starts or "accepted"!r} the pointer can reach', False, offered=offered[:30])
        close_popover(h, dropdown)
        return None
    text = ' '.join(labels(h, found))
    try:
        return {**h.click('settings', node=found), 'text': text}
    except LookupError as err:
        h.check(f'{dropdown}: the option {text!r} can be clicked', False, error=str(err), rect=found.get('rect'))
        close_popover(h, dropdown)
        return None


def close_popover(h, dropdown: str) -> None:
    """Close a drop-down's popover: Escape, else a real click on the drop-down's button (a click outside an open
    popover closes it)."""
    h.key('settings', 'Escape')
    if until(h, lambda: not h.find('settings', cls='w11-list-item') or None, 'the popover closes', 2) is None:
        h.click('settings', dropdown)
        until(h, lambda: not h.find('settings', cls='w11-list-item') or None, 'the popover closes', 3)


def options(h, dropdown: str) -> list[str]:
    """The labels a kit drop-down offers (opened with a real click, then closed)."""
    press(h, dropdown, room=330)
    found = until(h, lambda: h.find('settings', cls='w11-list-item'), f'{dropdown} open', 5) or []
    out = [' '.join(labels(h, n)) for n in found]
    close_popover(h, dropdown)
    return out


def dropdown_label(h, name: str) -> str:
    n = node(h, name)
    return ' '.join(labels(h, n)) if n else ''


def dialog_open(h, timeout: float = 10) -> bool:
    return bool(until(h, lambda: h.find('settings', 'w11-dialog'), 'a dialog opens', timeout))


def dialog_closed(h, timeout: float = 10) -> bool:
    """Whether the open dialog closed by itself; Escape closes one that stays (it would take every later click)."""
    closed = bool(until(h, lambda: not h.find('settings', 'w11-dialog') or None, 'the dialog closes', timeout))
    if not closed:
        h.key('settings', 'Escape')
        until(h, lambda: not h.find('settings', 'w11-dialog') or None, 'the dialog closes after Escape', 5)
    return closed


def dialog_button(h, name: str = 'w11-dialog-primary', wait: float = 8) -> dict:
    """Click a sensitive kit dialog button with the real pointer; report any dev fallback as a failure."""
    until(h, lambda: (node(h, name) or {}).get('sensitive') is True or None, f'{name} enabled', wait)
    click = h.click('settings', name)
    h.check(f'{name}: the dialog takes a real pointer click', click.get('input') == 'clutter', click=click)
    return click


def proc_cmdlines(match: str) -> list[tuple[int, list[str]]]:
    """The processes of the sandbox (its own PID namespace) one of whose arguments contains ``match``."""
    out = []
    for entry in Path('/proc').iterdir():
        if not entry.name.isdigit() or int(entry.name) == os.getpid():
            continue
        try:
            argv = (entry / 'cmdline').read_bytes().split(b'\0')
        except OSError:
            continue
        args = [a.decode(errors='replace') for a in argv if a]
        if any(match in a for a in args):
            out.append((int(entry.name), args))
    return out


def guard_calls(h, name: str) -> list[str]:
    """The lines of logs/guard-calls.log for the stub ``name`` (time, name, argv)."""
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [line for line in lines if line.split('\t')[1:2] == [name]]


def record_calls(h, tag: str, standin) -> None:
    """The stand-in's calls in the results (``<tag>.calls``), without the reads."""
    if standin:
        h.record(f'{tag}.calls', [{k: c[k] for k in ('path', 'method', 'args', 'interactive')} for c in standin.calls()
                                  if not c['method'].startswith(('Get', 'org.'))])
