"""Settings parity batch C (notes/claude/settings-parity.md §2, §7; plan.json "testing", GNOME HARNESS): Geavanceerde
netwerkinstellingen, Geavanceerde instellingen voor delen, Extern bureaublad and Systeem › Geavanceerd in the private
GNOME, with the stand-ins of tests/gnome/lib/standins/c.py: NetworkManager (fakes/c/networkmanager.py --system-only:
wlan0 connected to Lab-5G, eth0 without a cable, a modem; NetworkManager can check connectivity) and the sharing
daemons (fakes/c/sharing.py --no-user-systemd: systemd's system manager, gsd Sharing with Lab-5G as the current
network and rygel shared on "Kantoor", gnome-remote-desktop's RDP server and its configuration daemon, PackageKit with
an installable openssh-server; systemd's user manager only while the Remote Desktop switch is tested, as a second
stand-in with --user-systemd-only, because WinPoint's launcher, which the harness uses to open each page, starts apps
through the user manager whenever one is there).

Run (from the worktree; with --strace because the pages reach systemd, PackageKit and grdctl):
    tools/heavy.sh --min-mb 2560 --tag c -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-sharing.py --out tests/gnome/scenarios/settings-sharing.out \\
        --strace tests/gnome/scenarios/settings-sharing.out/strace.txt
and the same with --lang en --out tests/gnome/scenarios/settings-sharing.en.out (--strace inside it); the strace logs
stay local (results.json has their straceConnect and straceExec verdicts).

Every page: opened with h.open('settings:<id>'), a screenshot '<id>.<lang>' (more of a page: '<id>-<what>.<lang>'),
the header checked against registry.rs (the breadcrumb of the parent chain: crumb-<id> buttons, then the current
page), Settings in the run's language, no placeholder and, in en, no Dutch word on the page. Writes reach their
daemon with GNOME's arguments and, wherever GNOME's daemons check polkit, with the flag that allows GNOME's password
dialog; the Remote Desktop keys are read from the run's own keyfile (h.gsettings_get) and written there from outside
(h.gsettings_set).

  1. No daemon: Geavanceerde netwerkinstellingen shows GNOME's "Netwerk niet beschikbaar"; Geavanceerde instellingen
     voor delen says that gnome-settings-daemon isn't running and its Mediastreaming switch is insensitive.
  2. network-advanced with NetworkManager: one expander per adapter, Hardware- en verbindingseigenschappen, GNOME's
     connectivity check (its switch sets NetworkManager's ConnectivityCheckEnabled false, then true) and the link to
     the sharing page; Uitschakelen of wlan0 sends Device.Disconnect.
  3. sharing-advanced with gsd Sharing: a real pointer click on Mediastreaming sends EnableService('rygel');
     Verwijderen of Kantoor DisableService('rygel', <Kantoor>); Verwijderen of the first folder writes the user's
     rygel.conf without it (GNOME's writer: upnp-enabled true).
  4. remotedesktop: the switch asks "Extern bureaublad inschakelen?", then the user unit gnome-remote-desktop.service
     is started and enabled, `enable` is true and tls-cert names GNOME's new certificate in the private data
     directory; grdctl is the run/stubs one and never gets a password on its command line; Externe besturing writes
     view-only false; outside writes of view-only and enable move both switches (gsettings.changed); Extern
     aanmelden asks GetCredentials, then SetCredentials, ImportCertificate and Enable on the configuration daemon.
  5. system-advanced: without openssh-server Windows' "Een optionele functie toevoegen"; Installeren runs PackageKit's
     Resolve and InstallPackages of openssh-server, and turns off what Ubuntu's postinst started (GNOME's
     remote-login-helper action, DisableUnitFiles, StopUnit: Windows' optional feature arrives stopped); then a real
     pointer click on the OpenSSH-server switch acquires that action and starts and enables ssh.socket (amendment 35)
     without a password dialog of systemd's own.

allow/network.rs has no Owned key, and the sandbox has no /etc/glib-2.0 locks (tests/gnome/README.md: lockdown is a
host unit test). Buttons inside a kit dialog get a real pointer click first and, when that does nothing, the dev
channel's click by name; one check (known C-dialog-click, batch B's note 9 for I) records whether the real clicks
reached them.
"""
from __future__ import annotations

import os
import pwd
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from lib.standins.c import calls_after, start_fake  # noqa: E402

OFFICE = '9b8a7c6d-5e4f-4321-8765-0fedcba98765'
RDP = 'org.gnome.desktop.remote-desktop.rdp'
PARENTS = {'network-advanced': ['network'], 'sharing-advanced': ['network', 'network-advanced'],
           'remotedesktop': ['system'], 'system-advanced': ['system']}
DIALOG_CLICKS: list[dict] = []
# pages whose last crumb the kit's breadcrumb cuts to "…" (batch B's note 11 for I: app.rs/kit Breadcrumb)
KNOWN_CRUMB_ELLIPSIS = ('sharing-advanced',)
LOGIN = pwd.getpwuid(os.getuid()).pw_name


# ---- helpers (as settings-network.py's) ----------------------------------------------------------------------

def wait(h, fn, label: str, timeout: float = 10):
    try:
        return h.until(fn, label, timeout, 0.3)
    except TimeoutError:
        return None


def one(h, name: str):
    found = h.find('settings', name)
    return found[0] if found else None


def active(h, name: str):
    node = one(h, name)
    return node.get('active') if node else None


def click(h, name: str, **kw) -> bool:
    try:
        h.click('settings', name, **kw)
        return True
    except (LookupError, RuntimeError) as error:
        h.check(f'click {name}', False, error=str(error))
        return False


def goto(h, pid: str, wait_for: str, timeout: float = 25):
    sf = h.surfaces.get('settings')
    if sf and sf.alive() and sf.page == pid:
        h.open('settings:home')
    h.open(f'settings:{pid}')
    node = wait(h, lambda: one(h, wait_for), f'{pid}: {wait_for}', timeout)
    h.check(f'{pid} shows {wait_for}', bool(node))
    return node


def header(h, pid: str, timeout: float = 10) -> bool:
    titles = h.session.titles
    parents = PARENTS[pid]
    want = [titles[p][h.lang] for p in parents] + [titles[pid][h.lang]]
    seen: dict = {}

    def shown():
        heads = h.find('settings', cls='st-title')
        if not heads:
            return None
        nodes = h.hm.tree_nodes([{'tree': heads[0]}])
        crumbs = [n for n in nodes if str(n.get('name') or '').startswith('crumb-')]
        current = [n.get('label') for n in nodes if 'w11-breadcrumb-current' in (n.get('classes') or [])]
        got = [n.get('label') for n in crumbs] + current if crumbs or current else [heads[0].get('label')]
        last = next((n for n in nodes if 'w11-breadcrumb-current' in (n.get('classes') or [])), None)
        seen.update(shown=got, ids=[n['name'][len('crumb-'):] for n in crumbs], currentRect=last and last.get('rect'))
        return got == want and seen['ids'] == parents
    ok = bool(wait(h, shown, f'{pid}: header', timeout))
    h.check(f'{pid}: title and breadcrumb {" › ".join(want)}', ok, want=want, **seen)
    if parents and seen.get('currentRect'):
        # the current crumb must be readable: one ellipsized to "…" is about 20 px wide; the title font (28 px) needs
        # about 14 px a character, so half of that is a safe floor (batch B's check; its note 11 for I)
        width, need = seen['currentRect'][2], 7 * len(want[-1])
        h.check(f'{pid}: the current crumb is shown in full, not as "…"', width >= need, width=width, need=need)
    return ok


def check_page(h, pid: str, what: str = '') -> None:
    header(h, pid)
    h.sleep(1.0)
    text = h.page_text('settings')
    label = f'{pid}{" " + what if what else ""}'
    h.check(f'{label}: Settings is in {h.lang}', text['lang'] == h.lang, lang=text['lang'])
    h.check(f'{label}: no placeholder', not one(h, 'st-placeholder'))
    if h.lang == 'en':
        h.check(f'{label}: no Dutch words in en', not text['dutchWordHits'], hits=text['dutchWordHits'])
    h.screenshot(f'{pid}{"-" + what if what else ""}.{h.lang}', 'settings')


def press(h, name: str, done, label: str) -> bool:
    """A button inside a kit dialog: a real pointer click, else the dev channel's click (settings-network.py)."""
    try:
        real = h.click('settings', name)
    except (LookupError, RuntimeError) as error:
        real = {'error': str(error)}
    ok = wait(h, done, f'{label} (real click)', 3)
    path = 'real'
    if not ok:
        path = 'dev'
        try:
            h.click('settings', name, real=False)
        except (LookupError, RuntimeError) as error:
            h.check(f'dev click {name}', False, error=str(error))
        ok = wait(h, done, f'{label} (dev click)', 20)
    DIALOG_CLICKS.append({'widget': name, 'action': label, 'path': path, 'done': bool(ok),
                          'pointer': {k: real.get(k) for k in ('x', 'y', 'rect', 'window', 'error') if k in real}})
    return bool(ok)


def stub_calls(h, tool: str) -> list[str]:
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [(line.split('\t', 2) + [''])[2] for line in lines if line.split('\t')[1:2] == [tool]]


def private_stubs(h):
    """The common harness establishes private permissions; this scenario only checks them."""
    mode = (h.hm.RUN / 'stubs').stat().st_mode & 0o777
    h.check('the tool seam run/stubs is private (0700)', mode == 0o700, mode=oct(mode))


# ---- the steps ------------------------------------------------------------------------------------------------

def without_daemons(h) -> None:
    goto(h, 'network-advanced', 'network-unavailable')
    check_page(h, 'network-advanced', 'nonm')
    h.check('network-advanced without NetworkManager: GNOME\'s "Netwerk niet beschikbaar"',
            bool(one(h, 'network-unavailable')))
    goto(h, 'sharing-advanced', 'sharing-advanced-discovery')
    check_page(h, 'sharing-advanced', 'nogsd')
    rygel = one(h, 'sharing-advanced-no-rygel')
    h.record('sharing-advanced.rygelInstalled', not rygel)
    if not rygel:
        h.check('sharing-advanced without gsd: "gnome-settings-daemon wordt niet uitgevoerd"',
                bool(one(h, 'sharing-advanced-status')))
        sw = one(h, 'sharing-advanced-media')
        h.check('sharing-advanced without gsd: the Mediastreaming switch is insensitive',
                bool(sw) and sw.get('sensitive') is False)


def network_advanced(h, nm) -> None:
    goto(h, 'network-advanced', 'network-advanced-adapter-0')
    check_page(h, 'network-advanced')
    for name in ('network-advanced-adapter-0', 'network-advanced-adapter-1', 'network-advanced-hardware',
                 'network-advanced-connectivity', 'link-sharing-advanced'):
        h.check(f'network-advanced: {name}', bool(one(h, name)))
    h.check('network-advanced: no "Netwerk niet beschikbaar" with NetworkManager', not one(h, 'network-unavailable'))
    if one(h, 'network-advanced-connectivity'):
        h.check('network-advanced: the connectivity check on (NetworkManager\'s)',
                active(h, 'network-advanced-connectivity') is True)
        for on in (False, True):
            before = len(nm.calls())
            if not click(h, 'network-advanced-connectivity'):
                break
            sets = [c for c in calls_after(nm, before, 'Set') if 'ConnectivityCheckEnabled' in c.get('args', '')]
            want = f"('org.freedesktop.NetworkManager', 'ConnectivityCheckEnabled', <{str(on).lower()}>)"
            h.check(f'network-advanced: the connectivity check {"off" if not on else "on"} sets ConnectivityCheckEnabled',
                    bool(sets) and sets[0].get('args') == want, calls=sets[:1])
            h.check(f'network-advanced: ConnectivityCheckEnabled {str(on).lower()} with the password dialog allowed',
                    bool(sets) and sets[0].get('interactive') is True)
            wait(h, lambda: active(h, 'network-advanced-connectivity') is on, 'the switch', 8)
    if one(h, 'network-advanced-toggle-0'):
        before = len(nm.calls())
        click(h, 'network-advanced-toggle-0')
        off = calls_after(nm, before, 'Disconnect')
        h.check('network-advanced: Uitschakelen sends Device.Disconnect of wlan0',
                bool(off) and off[0].get('path', '').endswith('/Devices/1'), calls=off[:1])
        h.check('network-advanced: with the password dialog allowed', bool(off) and off[0].get('interactive') is True)
    else:
        h.check('network-advanced: network-advanced-toggle-0', False)


def sharing_advanced(h, sharing) -> None:
    goto(h, 'sharing-advanced', 'sharing-advanced-media')
    check_page(h, 'sharing-advanced')
    for name in ('sharing-advanced-discovery', 'sharing-advanced-media', 'sharing-advanced-networks',
                 'sharing-advanced-folder-add', 'sharing-advanced-folder-remove-0'):
        h.check(f'sharing-advanced: {name}', bool(one(h, name)))
    h.check('sharing-advanced: no status notice while gsd Sharing says "available"', not one(h, 'sharing-advanced-status'))
    if one(h, 'sharing-advanced-media'):
        before = len(sharing.calls())
        click(h, 'sharing-advanced-media')
        on = calls_after(sharing, before, 'EnableService')
        h.check("sharing-advanced: Mediastreaming sends EnableService('rygel')",
                bool(on) and on[0].get('args') == "('rygel',)", calls=on[:1])
        h.sleep(1.5)
        h.screenshot(f'sharing-advanced-on.{h.lang}', 'settings')
    # The repaired expander exposes only visible children: open Networks before finding its rows.
    click(h, 'sharing-advanced-networks')
    if wait(h, lambda: one(h, 'sharing-advanced-network-remove-0'), 'the networks list', 8):
        h.sleep(1.0)
        h.screenshot(f'sharing-advanced-networks.{h.lang}', 'settings')
        before = len(sharing.calls())
        click(h, 'sharing-advanced-network-remove-0')
        off = calls_after(sharing, before, 'DisableService')
        h.check('sharing-advanced: Verwijderen of Kantoor sends DisableService(rygel, Kantoor)',
                bool(off) and off[0].get('args') == f"('rygel', '{OFFICE}')", calls=off[:1])
    else:
        h.check('sharing-advanced: sharing-advanced-network-remove-0', False)
    if wait(h, lambda: one(h, 'sharing-advanced-folder-remove-0'), 'the folders list', 8):
        conf = h.hm.RUN / 'config' / 'rygel.conf'
        click(h, 'sharing-advanced-folder-remove-0')
        wait(h, lambda: conf.exists(), 'rygel.conf written', 15)
        text = conf.read_text() if conf.exists() else ''
        uris = [line for line in text.splitlines() if line.startswith('uris=')]
        h.check('sharing-advanced: rygel.conf without the first folder (GNOME\'s writer)',
                uris == ['uris=@VIDEOS@;@PICTURES@;'] and 'upnp-enabled=true' in text, uris=uris)


def remotedesktop(h, sharing) -> None:
    goto(h, 'remotedesktop', 'st-remotedesktop-enable')
    check_page(h, 'remotedesktop')
    for name in ('st-remotedesktop-enable', 'remotedesktop-credentials', 'remotedesktop-copy-name',
                 'remotedesktop-login-switch'):
        h.check(f'remotedesktop: {name}', bool(one(h, name)))
    get = lambda key: h.gsettings_get(RDP, key)  # noqa: E731
    h.check('remotedesktop: off at first (enable is GNOME\'s default)', get('enable') in (None, 'false'),
            enable=get('enable'))
    if one(h, 'st-remotedesktop-enable'):
        # the user manager only now: WinPoint's launcher (the page relay) uses it when it is there
        user = start_fake(h, 'sharing', '--user-systemd-only', tag='sharing-user')
        grd_before = len(stub_calls(h, 'grdctl'))
        try:
            click(h, 'st-remotedesktop-enable')
            asked = wait(h, lambda: one(h, 'w11-dialog-primary'), 'the question', 6)
            h.check('remotedesktop: "Extern bureaublad inschakelen?" first', bool(asked))
            if asked and user:
                h.screenshot(f'remotedesktop-question.{h.lang}', 'settings')
                press(h, 'w11-dialog-primary', lambda: bool(calls_after(user, 0, 'EnableUnitFiles', timeout=0.5)),
                      'remotedesktop: Bevestigen')
                enabled = calls_after(user, 0, 'EnableUnitFiles')
                started = [c for c in user.calls() if c['method'] == 'StartUnit']
                h.check('remotedesktop: the user unit started and enabled, no password dialog (a user unit)',
                        bool(started) and started[0].get('args') == "('gnome-remote-desktop.service', 'replace')"
                        and bool(enabled) and enabled[0].get('interactive') is False, start=started[:1],
                        enable=enabled[:1])
                wait(h, lambda: get('enable') == 'true', 'enable true', 10)
                h.check('remotedesktop: enable true in the keyfile', get('enable') == 'true', enable=get('enable'))
                cert = get('tls-cert') or ''
                path = h.hm.RUN / 'data' / 'gnome-remote-desktop' / 'certificates' / 'rdp-tls.crt'
                h.check('remotedesktop: tls-cert names GNOME\'s new certificate', cert == f"'{path}'" and path.exists(),
                        cert=cert)
                key = h.hm.RUN / 'data' / 'gnome-remote-desktop' / 'certificates' / 'rdp-tls.key'
                h.check('remotedesktop: the key file is private (0600)',
                        key.exists() and key.stat().st_mode & 0o777 == 0o600,
                        mode=oct(key.stat().st_mode & 0o777) if key.exists() else None)
                grd = stub_calls(h, 'grdctl')[grd_before:]
                h.record('remotedesktop.grdctl', grd)
                h.check('remotedesktop: grdctl is the run/stubs one (status --show-credentials)',
                        any(g.startswith('status --show-credentials') for g in grd), grdctl=grd)
                h.check('remotedesktop: no password on grdctl\'s command line',
                        all(not g.startswith('rdp set-credentials') or g.split() == ['rdp', 'set-credentials', LOGIN]
                            for g in grd), grdctl=grd)
                h.sleep(2.0)
                nla = one(h, 'remotedesktop-nla')
                h.check('remotedesktop: the enabled section shows mandatory Network Level Authentication',
                        bool(nla) and nla.get('active') is True and nla.get('sensitive') is False, node=nla)
                h.screenshot(f'remotedesktop-on.{h.lang}', 'settings')
        finally:
            if user:
                user.stop()
    view = wait(h, lambda: (lambda n: n if n and n.get('sensitive') is not False else None)(one(h, 'st-remotedesktop-view-only')),
                'Externe besturing', 10)
    if view:
        h.check('remotedesktop: Externe besturing off (view-only is GNOME\'s default true)',
                active(h, 'st-remotedesktop-view-only') is False)
        click(h, 'st-remotedesktop-view-only')
        wait(h, lambda: get('view-only') == 'false', 'view-only false', 10)
        h.check('remotedesktop: Externe besturing writes view-only false', get('view-only') == 'false',
                view=get('view-only'))
        # another program writes the keys: both switches follow (gsettings.changed)
        h.gsettings_set(RDP, 'view-only', 'true')
        h.check('remotedesktop: an outside write of view-only true turns Externe besturing off',
                bool(wait(h, lambda: active(h, 'st-remotedesktop-view-only') is False, 'Externe besturing off', 10)))
        h.gsettings_set(RDP, 'enable', 'false')
        h.check('remotedesktop: an outside write of enable false turns the switch off',
                bool(wait(h, lambda: active(h, 'st-remotedesktop-enable') is False, 'the switch off', 10)))
        node = one(h, 'st-remotedesktop-view-only')
        h.check('remotedesktop: Externe besturing insensitive while Desktop Sharing is off (GNOME\'s)',
                bool(node) and node.get('sensitive') is False)
    else:
        h.check('remotedesktop: Externe besturing (st-remotedesktop-view-only) once Desktop Sharing is on', False)
    if one(h, 'remotedesktop-login-switch'):
        before = len(sharing.calls())
        click(h, 'remotedesktop-login-switch')
        calls_after(sharing, before, 'GetCredentials')
        asked = wait(h, lambda: one(h, 'w11-dialog-primary'), 'the details dialog', 8)
        h.check('remotedesktop: Extern aanmelden asks for the sign-in details first', bool(asked))
        if asked:
            h.screenshot(f'remotedesktop-login-question.{h.lang}', 'settings')
            press(h, 'w11-dialog-primary', lambda: bool(calls_after(sharing, before, 'Enable', timeout=0.5)),
                  'remotedesktop: Extern aanmelden on')
            got = [(c['method'], c.get('interactive')) for c in sharing.calls()[before:]
                   if c['method'] in ('GetCredentials', 'SetCredentials', 'ImportCertificate', 'Enable')]
            h.check('remotedesktop: remote login through the configuration daemon, each call interactive',
                    got == [('GetCredentials', True), ('SetCredentials', True), ('ImportCertificate', True),
                            ('Enable', True)], calls=got)


def system_advanced(h, sharing) -> None:
    goto(h, 'system-advanced', 'system-advanced-ssh-install')
    check_page(h, 'system-advanced')
    h.check('system-advanced: Windows\' "Een optionele functie toevoegen" without openssh-server',
            bool(one(h, 'system-advanced-ssh-install')))
    h.check('system-advanced: no switch before the install', not one(h, 'system-advanced-ssh-switch'))
    if one(h, 'system-advanced-ssh-install'):
        before = len(sharing.calls())
        click(h, 'system-advanced-ssh-install')
        installed = calls_after(sharing, before, 'InstallPackages', timeout=30)
        resolve = [c for c in sharing.calls()[before:] if c['method'] == 'Resolve']
        h.check('system-advanced: PackageKit resolves openssh-server',
                bool(resolve) and "['openssh-server']" in resolve[0].get('args', ''), calls=resolve[:1])
        h.check('system-advanced: InstallPackages with the password dialog allowed',
                bool(installed) and installed[0].get('interactive') is True
                and 'openssh-server;' in installed[0].get('args', ''), calls=installed[:1])
        wait(h, lambda: one(h, 'system-advanced-ssh-switch'), 'the switch after the install', 25)
        h.sleep(1.0)
        h.screenshot(f'system-advanced-installed.{h.lang}', 'settings')
        # Ubuntu's postinst enabled and started ssh.socket (the fake does it too): off again, as Windows' optional
        # feature arrives stopped, under GNOME's remote-login action, unit files disabled before the units stop
        after = [(c['method'], c.get('args')) for c in sharing.calls()[before:]
                 if c['method'] in ('CheckAuthorization', 'DisableUnitFiles', 'StopUnit', 'StartUnit')]
        h.check('system-advanced: the install leaves the server off (remote-login-helper, DisableUnitFiles, StopUnit)',
                [m for m, _ in after] == ['CheckAuthorization', 'DisableUnitFiles', 'StopUnit', 'StopUnit']
                and 'remote-login-helper' in (after[0][1] or '')
                and after[1][1] == "(['ssh.socket', 'ssh.service'], false)", calls=after)
        h.check('system-advanced: the switch shows Uit after the install',
                bool(wait(h, lambda: active(h, 'system-advanced-ssh-switch') is False, 'the switch off', 8)))
    if one(h, 'system-advanced-ssh-switch'):
        before = len(sharing.calls())
        click(h, 'system-advanced-ssh-switch')
        enabled = calls_after(sharing, before, 'EnableUnitFiles')
        got = sharing.calls()[before:]
        started = [c for c in got if c['method'] == 'StartUnit']
        h.check('system-advanced: ssh.socket started and enabled (amendment 35), with the password dialog allowed',
                bool(started) and started[0].get('args') == "('ssh.socket', 'replace')"
                and started[0].get('interactive') is True and bool(enabled)
                and enabled[0].get('args') == "(['ssh.socket'], false, false)" and enabled[0].get('interactive') is True,
                start=started[:1], enable=enabled[:1])
        order = [c['method'] for c in got if c['method'] in ('CheckAuthorization', 'StartUnit', 'EnableUnitFiles')]
        h.check('system-advanced: GNOME\'s remote-login action first, so systemd asks no password of its own',
                order == ['CheckAuthorization', 'StartUnit', 'EnableUnitFiles']
                and not any(c.get('dialog') for c in got if c['method'] in ('StartUnit', 'EnableUnitFiles')),
                order=order, dialogs=[c['method'] for c in got if c.get('dialog')])
        h.check('system-advanced: the switch stays on',
                bool(wait(h, lambda: active(h, 'system-advanced-ssh-switch') is True, 'the switch', 8)))
        h.check('system-advanced: the login command', bool(one(h, 'system-advanced-ssh-copy')))
    else:
        h.check('system-advanced: the OpenSSH-server switch', False)


def no_owned_keys(h) -> None:
    allow = Path(__file__).resolve().parents[3] / 'host' / 'src' / 'services' / 'gsettings' / 'allow' / 'network.rs'
    text = allow.read_text() if allow.exists() else ''
    h.check('allow/network.rs has no Owned key (nothing here points to a WinPoint page)',
            bool(text) and 'Owned' not in text)


def run(h):
    private_stubs(h)
    no_owned_keys(h)
    without_daemons(h)
    nm = start_fake(h, 'networkmanager', '--system-only')
    if nm is None:
        return
    sharing = None
    try:
        network_advanced(h, nm)
        sharing = start_fake(h, 'sharing', '--no-user-systemd')
        if sharing is None:
            return
        sharing_advanced(h, sharing)
        remotedesktop(h, sharing)
        system_advanced(h, sharing)
    finally:
        reached = [d for d in DIALOG_CLICKS if d['path'] == 'real']
        h.record('dialogClicks', DIALOG_CLICKS)
        h.check('the buttons inside kit dialogs take real pointer clicks', len(reached) == len(DIALOG_CLICKS), real=len(reached), all=len(DIALOG_CLICKS))
        h.check('every dialog action was done (real or dev click)', all(d['done'] for d in DIALOG_CLICKS),
                failed=[d['action'] for d in DIALOG_CLICKS if not d['done']])
        if sharing:
            h.record('sharing.calls', [{k: c.get(k) for k in ('bus', 'method', 'args', 'interactive')}
                                       for c in sharing.calls()][-120:])
            sharing.stop()
        h.record('networkmanager.calls', [{k: c.get(k) for k in ('method', 'path', 'interactive')}
                                          for c in nm.calls()][-60:])
        nm.stop()
        h.record('stubs.grdctl', stub_calls(h, 'grdctl'))
