"""Settings parity batch C (notes/claude/settings-parity.md §2, §7; plan.json "testing", GNOME HARNESS): Netwerk en
internet and its pages in the private GNOME, with the stand-ins of tests/gnome/lib/standins/c.py: NetworkManager
(tests/dbus-fakes/fakes/c/networkmanager.py --system-only: Lab-5G connected on wlan0, eth0 without a cable, the
OpenVPN profile Kantoor and the WireGuard profile wg0, a gsm profile, a modem device), gsd's Rfkill (GSD_RFKILL, and
GSD_RFKILL_HARDWARE with the hardware switch on) and ModemManager (fakes/c/modemmanager.py: a modem whose SIM wants
its PIN, 1234).

Run (from the worktree; with --strace because the pages can reach nmcli, the flight mode fallback and the VPN import):
    tools/heavy.sh --min-mb 2560 --tag c -- python3 -B tests/gnome/harness.py launch run --lang nl --source worktree \\
        --scenario tests/gnome/scenarios/settings-network.py --out tests/gnome/scenarios/settings-network.out \\
        --strace tests/gnome/scenarios/settings-network.out/strace.txt
and the same with --lang en --out tests/gnome/scenarios/settings-network.en.out (--strace inside it); the strace logs
stay local (results.json has their straceConnect and straceExec verdicts).

Every page: opened with h.open('settings:<id>') (Wi-Fi-eigenschappen, a data page, by a real click on "Eigenschappen"
of the connected network: h.settings_data_page), a screenshot '<id>.<lang>' (more of a page: '<id>-<what>.<lang>'),
the header checked against registry.rs (the title of a top-level page, else the breadcrumb of the parent chain:
crumb-<id> buttons, then the current page or the data page's item name), Settings in the run's language, no
placeholder and, in en, no Dutch word on the page. The controls are flipped with real pointer clicks; a write must
reach its daemon with GNOME's exact value (NetworkManager's settings dictionary, gsd's property, ModemManager's
arguments) and, where the daemon checks polkit, with the flag that allows GNOME's password dialog; the proxy page's
keys are read back from the run's own keyfile (h.gsettings_get) and written there from outside (h.gsettings_set).

  1. No daemon: Netwerk en internet shows GNOME's "Netwerk niet beschikbaar", Mobiel netwerk is not offered (caps: no
     modem) and its deep link shows GNOME's "no adapter" page; flight mode without gsd's Rfkill falls back to the
     run/stubs nmcli (radio all off), nothing real runs, and the switch goes back.
  2. NetworkManager and gsd's Rfkill: Netwerk en internet lists Windows' rows (Wi-Fi, Ethernet, VPN, Mobiele hotspot
     with its switch, Vliegtuigmodus, Proxy, Geavanceerde netwerkinstellingen; still no Mobiel netwerk). Wi-Fi: the
     switch sets NetworkManager's WirelessEnabled (off, on); Verbinden on eduroam (WPA2-Enterprise, no profile) opens
     GNOME's dialog for the access point with the 802.1X fields. Wi-Fi-eigenschappen of Lab-5G: autoconnect, available to
     other users (permissions user:<login>:), metered (connection.metered 1) and Willekeurige hardwareadressen
     (assigned-mac-address random) each send one Update2 that keeps the profile's other keys. Bekende netwerken
     beheren lists Lab-5G. Ethernet: eth0 without a cable (its switch insensitive, as GNOME's), metered on its
     wired profile. VPN: both profiles; Verbinding maken sends ActivateConnection of Kantoor, "Beschikbaar maken voor
     andere gebruikers" an Update2 of its permissions. Vliegtuigmodus: the switch sets gsd's AirplaneMode true and
     false; with the hardware switch on, GNOME's notice and an insensitive switch. Proxy: "Instellingen automatisch
     detecteren" writes mode auto with an empty autoconfig-url and shows GNOME's WPAD warning, off again mode none;
     outside writes of mode move the switch; "Installatiescript gebruiken" writes the script address then mode
     auto; "Een proxyserver gebruiken" writes GNOME's HTTP, HTTPS and FTP servers, the ignored hosts (*.local for
     the intranet box) and then mode manual (amendment 51: one gsettings.set, mode last). Mobiele hotspot: GNOME's
     question (wlan0 carries Lab-5G), then AddAndActivateConnection2 of GNOME's hotspot profile (mode ap, IPv4
     shared).
  3. ModemManager as well (Settings opened again, so caps.get asks the bus again): Mobiel netwerk is offered on
     Netwerk en internet and on Vliegtuigmodus; its page shows the locked SIM, the unlock dialog sends SendPin('1234'),
     then Mobiele gegevens and Roaming write the gsm profile (home-only false for Roamen).

allow/network.rs has no Owned key (nothing on these pages points to a WinPoint page instead of a switch), and the
sandbox has no /etc/glib-2.0 locks (tests/gnome/README.md: lockdown is a host unit test). Buttons and switches inside a
kit dialog get a real pointer click first and, when that does nothing, the dev channel's click by name, so the action
itself is still checked; one check (known C-dialog-click: the kit's modal layer claims every press, batch B's note 9
for I) records whether the real clicks reached them.
"""
from __future__ import annotations

import os
import pwd
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from lib.standins import start  # noqa: E402
from lib.standins.c import GSD_RFKILL, GSD_RFKILL_HARDWARE, calls_after, start_fake  # noqa: E402

NM_P = '/org/freedesktop/NetworkManager'
WIFI_CONN, ETH_CONN, VPN_CONN = f'{NM_P}/Settings/1', f'{NM_P}/Settings/2', f'{NM_P}/Settings/3'
GSM_CONN, ICCID = f'{NM_P}/Settings/5', '8931440000000000017'
PROXY = 'org.gnome.system.proxy'
# registry.rs: each page's parent chain (the breadcrumb before the page's own title)
PARENTS = {'network': [], 'wifi': ['network'], 'wifi-network': ['network', 'wifi'], 'wifi-known': ['network', 'wifi'],
           'ethernet': ['network'], 'mobile': ['network'], 'vpn': ['network'], 'hotspot': ['network'],
           'flightmode': ['network'], 'proxy': ['network']}
TEXT = {
    'nl': {'unplugged': 'Netwerkkabel losgekoppeld', 'daily': 'Dagelijks wijzigen', 'roam': 'Roamen',
           'data': 'Mobiele gegevens'},
    'en': {'unplugged': 'Network cable unplugged', 'daily': 'Change daily', 'roam': 'Roam', 'data': 'Mobile data'},
}
DIALOG_CLICKS: list[dict] = []
# pages whose last crumb the kit's breadcrumb cuts to "…" (batch B's note 11 for I: app.rs/kit Breadcrumb)
KNOWN_CRUMB_ELLIPSIS = ()
LOGIN = pwd.getpwuid(os.getuid()).pw_name


# ---- helpers --------------------------------------------------------------------------------------------------

def wait(h, fn, label: str, timeout: float = 10):
    """h.until without the exception: None when it timed out (the check after it fails)."""
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
    """A real click on a Settings widget; a missing widget is a failed check, not the end of the scenario."""
    try:
        h.click('settings', name, **kw)
        return True
    except (LookupError, RuntimeError) as error:
        h.check(f'click {name}', False, error=str(error))
        return False


def goto(h, pid: str, wait_for: str, timeout: float = 25):
    """Show a page (h.open; when the harness's last page is this one already, Home first so it is built again) and
    wait for one of its widgets."""
    sf = h.surfaces.get('settings')
    if sf and sf.alive() and sf.page == pid:
        h.open('settings:home')
    h.open(f'settings:{pid}')
    node = wait(h, lambda: one(h, wait_for), f'{pid}: {wait_for}', timeout)
    h.check(f'{pid} shows {wait_for}', bool(node))
    return node


def header(h, pid: str, item: str | None = None, timeout: float = 10) -> bool:
    """Settings' header as a user reads it, against registry.rs: the title of a top-level page, else the breadcrumb of
    its parent chain (crumb-<id> buttons, then the current page or a data page's item name)."""
    titles = h.session.titles
    parents = PARENTS[pid]
    want = [titles[p][h.lang] for p in parents] + [item or titles[pid][h.lang]]
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


def check_page(h, pid: str, item: str | None = None, what: str = '') -> None:
    """Header, language, no placeholder, no Dutch word in en, a screenshot."""
    header(h, pid, item)
    h.sleep(1.0)
    text = h.page_text('settings')
    h.check(f'{pid}{" " + what if what else ""}: Settings is in {h.lang}', text['lang'] == h.lang, lang=text['lang'])
    h.check(f'{pid}{" " + what if what else ""}: no placeholder', not one(h, 'st-placeholder'))
    if h.lang == 'en':
        h.check(f'{pid}{" " + what if what else ""}: no Dutch words in en', not text['dutchWordHits'],
                hits=text['dutchWordHits'])
    h.screenshot(f'{pid}{"-" + what if what else ""}.{h.lang}', 'settings')


def press(h, name: str, done, label: str) -> bool:
    """A widget inside a kit dialog: a real pointer click, and when ``done()`` stays false the dev channel's click by
    name, so the action itself is checked either way; each attempt is listed in the C-dialog-click check."""
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
        ok = wait(h, done, f'{label} (dev click)', 15)
    DIALOG_CLICKS.append({'widget': name, 'action': label, 'path': path, 'done': bool(ok),
                          'pointer': {k: real.get(k) for k in ('x', 'y', 'rect', 'window', 'error') if k in real}})
    return bool(ok)


def option(h, label: str):
    """The open drop-down list's option with that label (a w11-list-item button of the popover)."""
    return next((n for n in h.find('settings', cls='w11-list-item')
                 if label in [k.get('label') for k in h.hm.tree_nodes([{'tree': n}])]), None)


def choose(h, name: str, label: str) -> bool:
    """A kit drop-down by real pointer clicks: its button, then the option with that label in its list."""
    if not click(h, name):
        return False
    opt = wait(h, lambda: option(h, label), f'{name}: option {label!r}', 5)
    if not opt:
        h.check(f'{name} lists {label!r}', False)
        h.key('settings', 'Escape')
        return False
    h.click('settings', node=opt)
    return True


def stub_calls(h, tool: str) -> list[str]:
    """The argv (after the name) of every call of a run/stubs tool (logs/guard-calls.log: time, name, args)."""
    try:
        lines = (h.hm.RUN / 'logs' / 'guard-calls.log').read_text().splitlines()
    except OSError:
        return []
    return [(line.split('\t', 2) + [''])[2] for line in lines if line.split('\t')[1:2] == [tool]]


def private_stubs(h):
    """The common harness establishes private permissions; this scenario only checks them."""
    mode = (h.hm.RUN / 'stubs').stat().st_mode & 0o777
    h.check('the tool seam run/stubs is private (0700)', mode == 0o700, mode=oct(mode))


def update2(nm, before: int, conn: str, timeout: float = 15) -> dict:
    """The first Update2 of the profile ``conn`` NetworkManager got after its first ``before`` calls (or {})."""
    found = [c for c in calls_after(nm, before, 'Update2', timeout=timeout) if c.get('path') == conn]
    if not found:
        found = [c for c in nm.calls()[before:] if c.get('method') == 'Update2' and c.get('path') == conn]
    return found[0] if found else {}


def switch_writes(h, nm, name: str, conn: str, want: str, label: str, keep: tuple[str, ...] = ()) -> None:
    """A real click on a profile's switch (or drop-down choice when ``name`` is a callable) sends one Update2 of the
    profile with ``want`` in its settings, the keys in ``keep`` kept, and the password dialog allowed."""
    before = len(nm.calls())
    if callable(name):
        if not name():
            return
    elif not click(h, name):
        return
    update = update2(nm, before, conn)
    body = update.get('args', '')
    h.check(f'{label}: Update2 with {want}', want in body, args=body[:600])
    if keep:
        h.check(f'{label}: the profile\'s other keys kept', all(k in body for k in keep), keep=list(keep))
    h.check(f'{label}: with the password dialog allowed', update.get('interactive') is True)
    h.sleep(1.5)     # the page reads the profile back (net.changed) before the next control is clicked


# ---- 1. no daemon ---------------------------------------------------------------------------------------------

def without_daemons(h) -> None:
    goto(h, 'network', 'network-unavailable')
    check_page(h, 'network', what='nonm')
    h.check('network without NetworkManager: GNOME\'s "Netwerk niet beschikbaar"', bool(one(h, 'network-unavailable')))
    caps = h.backend('settings', 'caps.get') or {}
    h.record('caps.nomodem', {k: caps.get(k) for k in ('session', 'gnome', 'modem')})
    h.check('caps: no modem without ModemManager', caps.get('modem') is False, modem=caps.get('modem'))
    h.check('network: Mobiel netwerk is not offered without a modem', not one(h, 'link-mobile'))
    goto(h, 'mobile', 'mobile-none')
    h.check('mobile by its deep link: GNOME\'s "no mobile broadband adapter" page', bool(one(h, 'mobile-none')))
    h.check('mobile without a modem: no switch', not one(h, 'mobile-switch'))
    h.screenshot(f'mobile-none.{h.lang}', 'settings')
    # flight mode without gsd's Rfkill: the old path (nmcli, then rfkill), only ever the run's stubs
    goto(h, 'flightmode', 'flightmode-airplane')
    before = len(stub_calls(h, 'nmcli'))
    if click(h, 'flightmode-airplane'):
        ran = wait(h, lambda: stub_calls(h, 'nmcli')[before:], 'the nmcli stub', 10) or []
        h.check('flightmode without gsd\'s Rfkill: the fallback runs the run/stubs nmcli "radio all off"',
                'radio all off' in ran, nmcli=ran)
        back = wait(h, lambda: active(h, 'flightmode-airplane') is False, 'the switch goes back', 10)
        h.check('flightmode without gsd\'s Rfkill: the switch goes back after the refusal', bool(back))


# ---- 2. NetworkManager and gsd's Rfkill ---------------------------------------------------------------------

def network_page(h, nm) -> None:
    goto(h, 'network', 'network-wifi')
    check_page(h, 'network')
    h.check('network: NetworkManager runs (no "Netwerk niet beschikbaar")', not one(h, 'network-unavailable'))
    for name in ('network-wifi', 'network-ethernet', 'link-vpn', 'link-hotspot', 'network-flightmode', 'link-proxy',
                 'link-network-advanced'):
        h.check(f'network: {name}', bool(one(h, name)))
    h.check('network: still no Mobiel netwerk (no modem)', not one(h, 'link-mobile'))
    h.check('network: the hotspot switch once net.hotspot.get answers', bool(one(h, 'network-hotspot')))
    # the overview's hotspot switch asks the hotspot page's question first (wlan0 carries Lab-5G); cancelled, nothing
    # is started and the switch goes back
    if one(h, 'network-hotspot'):
        before = len(nm.calls())
        click(h, 'network-hotspot')
        asked = wait(h, lambda: one(h, 'w11-dialog-primary'), 'the hotspot question', 6)
        h.check('network: the hotspot switch asks GNOME\'s question first (Lab-5G would drop)', bool(asked))
        if asked:
            h.screenshot(f'network-hotspot-question.{h.lang}', 'settings')
            h.key('settings', 'Escape')
        h.check('network: cancelled, the hotspot switch goes back to Uit',
                bool(wait(h, lambda: not one(h, 'w11-dialog-primary') and active(h, 'network-hotspot') is False,
                          'the switch back', 6)))
        h.check('network: cancelled, no hotspot started',
                not any(c['method'] == 'AddAndActivateConnection2' for c in nm.calls()[before:]))


def wifi_page(h, nm) -> None:
    goto(h, 'wifi', 'wifi-active-properties')
    check_page(h, 'wifi')
    for name in ('wifi-active-properties', 'wifi-hidden-connect', 'wifi-toggle'):
        h.check(f'wifi: {name}', bool(one(h, name)))
    listed = wait(h, lambda: one(h, 'wifi-connect-0') or one(h, 'wifi-properties-0'), 'the network list', 10)
    h.check('wifi: the list shows NetworkManager\'s access point Lab-5G (D-Bus, as GNOME; no nmcli)',
            bool(listed) and 'Lab-5G' in h.text('settings')['text'])
    h.check('wifi: the switch shows NetworkManager\'s WirelessEnabled (on)', active(h, 'wifi-toggle') is True)
    for on in (False, True):
        before = len(nm.calls())
        if not click(h, 'wifi-toggle'):
            break
        sets = [c for c in calls_after(nm, before, 'Set') if 'WirelessEnabled' in c.get('args', '')]
        want = f"('org.freedesktop.NetworkManager', 'WirelessEnabled', <{str(on).lower()}>)"
        h.check(f'wifi: the switch sets WirelessEnabled {str(on).lower()} (GNOME\'s Wi-Fi switch, no nmcli)',
                bool(sets) and sets[0].get('args') == want, calls=sets[:1])
        h.check(f'wifi: WirelessEnabled {str(on).lower()} with the password dialog allowed',
                bool(sets) and sets[0].get('interactive') is True)
        settled = wait(h, lambda: active(h, 'wifi-toggle') is on, 'the switch stays', 8)
        h.check(f'wifi: the switch stays {"on" if on else "off"}', bool(settled))
        if not on:
            h.screenshot(f'wifi-off.{h.lang}', 'settings')
    # eduroam (WPA2-Enterprise, no profile; row 1 after the connected Lab-5G): Verbinden opens GNOME's dialog for the
    # access point with the 802.1X fields, instead of the key prompt and the "zakelijk netwerk" refusal
    listed = wait(h, lambda: one(h, 'wifi-connect-1') and 'eduroam' in h.text('settings')['text'], 'eduroam listed', 10)
    h.check('wifi: the enterprise network eduroam is listed', bool(listed))
    if listed and click(h, 'wifi-connect-1'):
        form = wait(h, lambda: one(h, 'net-eap-method') and one(h, 'net-new-ssid'), 'the 802.1X dialog', 8)
        h.check('wifi: Verbinden on eduroam asks for its 802.1X fields (GNOME\'s Wi-Fi dialog, no key prompt)',
                bool(form) and not one(h, 'wifi-password'))
        if form:
            h.screenshot(f'wifi-enterprise.{h.lang}', 'settings')
        h.key('settings', 'Escape')
        wait(h, lambda: not one(h, 'net-new-ssid'), 'the dialog closed', 5)


def wifi_network_page(h, nm) -> None:
    h.settings_data_page('wifi', 'wifi-active-properties', title='Lab-5G')
    wait(h, lambda: one(h, 'st-wifi-network-autoconnect'), 'wifi-network: its switches', 15)
    check_page(h, 'wifi-network', item='Lab-5G')
    for name in ('st-wifi-network-autoconnect', 'st-wifi-network-permissions', 'st-wifi-network-assigned-mac-address',
                 'st-wifi-network-metered', 'wifi-network-ip-edit', 'wifi-network-dns-edit',
                 'wifi-network-security-edit', 'wifi-network-key-view', 'wifi-network-forget'):
        h.check(f'wifi-network: {name}', bool(one(h, name)))
    keep = ("'zone': <'home'>", "'x-vendor'", "'seen-bssids'")
    switch_writes(h, nm, 'st-wifi-network-autoconnect', WIFI_CONN, "'autoconnect': <false>",
                  'wifi-network autoconnect', keep)
    wait(h, lambda: one(h, 'st-wifi-network-permissions'), 'the page again', 10)
    switch_writes(h, nm, 'st-wifi-network-permissions', WIFI_CONN, f"'permissions': <['user:{LOGIN}:']>",
                  'wifi-network available to other users (off)', keep)
    wait(h, lambda: one(h, 'st-wifi-network-metered'), 'the page again', 10)
    switch_writes(h, nm, 'st-wifi-network-metered', WIFI_CONN, "'metered': <1>", 'wifi-network metered (on)', keep)
    wait(h, lambda: one(h, 'st-wifi-network-assigned-mac-address'), 'the page again', 10)
    switch_writes(h, nm, lambda: choose(h, 'st-wifi-network-assigned-mac-address', TEXT[h.lang]['daily']), WIFI_CONN,
                  "'assigned-mac-address': <'random'>", 'wifi-network random hardware addresses (change daily)', keep)
    h.sleep(1.0)
    h.screenshot(f'wifi-network-written.{h.lang}', 'settings')


def known_and_ethernet(h, nm) -> None:
    goto(h, 'wifi-known', 'wifi-known-0')
    check_page(h, 'wifi-known')
    for name in ('wifi-known-0', 'wifi-known-add', 'wifi-known-search', 'wifi-known-forget-0'):
        h.check(f'wifi-known: {name}', bool(one(h, name)))
    goto(h, 'ethernet', 'ethernet-adapter-0')
    h.wait_text('settings', TEXT[h.lang]['unplugged'], timeout=10)
    check_page(h, 'ethernet')
    sw = one(h, 'st-ethernet-eth0-enabled')
    h.check('ethernet: eth0\'s switch insensitive while the cable is out (GNOME\'s)',
            bool(sw) and sw.get('sensitive') is False, node={k: sw.get(k) for k in ('active', 'sensitive')} if sw else None)
    if one(h, 'st-ethernet-metered'):
        switch_writes(h, nm, 'st-ethernet-metered', ETH_CONN, "'metered': <1>", 'ethernet metered (on)',
                      ("'interface-name': <'eth0'>",))
    else:
        h.check('ethernet: st-ethernet-metered', False)


def vpn_page(h, nm) -> None:
    goto(h, 'vpn', 'vpn-0')
    check_page(h, 'vpn')
    for name in ('vpn-0', 'vpn-1', 'vpn-add', 'vpn-import', 'vpn-0-connect'):
        h.check(f'vpn: {name}', bool(one(h, name)))
    before = len(nm.calls())
    if click(h, 'vpn-0-connect'):
        act = calls_after(nm, before, 'ActivateConnection')
        h.check('vpn: Verbinding maken sends ActivateConnection of Kantoor',
                bool(act) and VPN_CONN in act[0].get('args', ''), calls=act[:1])
        h.check('vpn: ActivateConnection with the password dialog allowed', bool(act) and act[0].get('interactive') is True)
    # the page reads the profiles again after Verbinding maken (Kantoor now "Verbonden"), its expanders closed: a real
    # click on Kantoor's header opens its rows (a closed expander's rows are in the dev state too, but take no click)
    wait(h, lambda: (lambda n: n and n.get('label') != 'Verbinding maken' and n.get('label') != 'Connect')(one(h, 'vpn-0-connect')),
         'vpn-0 connected', 10)
    h.sleep(1.0)
    if one(h, 'vpn-0'):
        click(h, 'vpn-0')
        h.sleep(1.0)
    if one(h, 'st-vpn-0-permissions'):
        switch_writes(h, nm, 'st-vpn-0-permissions', VPN_CONN, f"'permissions': <['user:{LOGIN}:']>",
                      'vpn available to other users (off)', ("'service-type': <'org.freedesktop.NetworkManager.openvpn'>",))
        h.screenshot(f'vpn-open.{h.lang}', 'settings')
    else:
        h.check('vpn: st-vpn-0-permissions', False)

    # Import through the running production host, with no external daemon or VPN traffic.
    # This complements the real-pointer import-button presence check and the offline importer unit/bus tests.
    fixture = Path(os.environ['XDG_CONFIG_HOME']) / 'wgtest.conf'
    fixture.write_text('[Interface]\nPrivateKey = AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=\nAddress = 10.70.0.2/24\n[Peer]\nPublicKey = ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=\nAllowedIPs = 10.70.0.0/24\nEndpoint = vpn.example.invalid:51820\n')
    before = len(nm.calls())
    result = h.backend('settings', 'net.vpn.import', {'path': str(fixture)}) or {}
    added = calls_after(nm, before, 'AddConnection2')
    h.check('vpn: offline WireGuard import returns the new profile UUID', bool(result.get('uuid')) and result.get('type') == 'wireguard')
    h.check('vpn: imported typed settings use interactive AddConnection2', bool(added)
            and added[0].get('interactive') is True and "'wireguard'" in added[0].get('args', ''))
    fixture.unlink()


def flightmode_page(h, rfkill) -> None:
    goto(h, 'flightmode', 'flightmode-airplane')
    check_page(h, 'flightmode')
    h.check('flightmode: the Wi-Fi switch (Draadloze apparaten)', bool(one(h, 'flightmode-wifi')))
    h.check('flightmode: no Mobiel netwerk switch without a modem', not one(h, 'flightmode-mobile'))
    h.check('flightmode: no hardware notice', not one(h, 'flightmode-hardware'))
    for on in (True, False):
        before = len(rfkill.calls())
        if not click(h, 'flightmode-airplane'):
            break
        sets = calls_after(rfkill, before, 'Set:AirplaneMode', timeout=10)
        h.check(f'flightmode: the switch sets gsd\'s AirplaneMode {str(on).lower()} (amendment 57)',
                bool(sets) and sets[0].get('args') == str(on).lower(), calls=sets[:1])
        settled = wait(h, lambda: active(h, 'flightmode-airplane') is on, 'the switch stays', 8)
        h.check(f'flightmode: the switch stays {"on" if on else "off"}', bool(settled))
        if on:
            h.screenshot(f'flightmode-on.{h.lang}', 'settings')


def flightmode_hardware(h, rfkill) -> None:
    rfkill.stop()
    hw = start(h, GSD_RFKILL_HARDWARE, 'gsd-rfkill-hardware')
    if not hw:
        return
    try:
        goto(h, 'flightmode', 'flightmode-hardware')
        h.check('flightmode with the hardware switch on: GNOME\'s notice', bool(one(h, 'flightmode-hardware')))
        sw = one(h, 'flightmode-airplane')
        h.check('flightmode with the hardware switch on: the switch is on and cannot change it',
                bool(sw) and sw.get('active') is True and sw.get('sensitive') is False,
                node={k: sw.get(k) for k in ('active', 'sensitive')} if sw else None)
        h.screenshot(f'flightmode-hardware.{h.lang}', 'settings')
    finally:
        hw.stop()


def proxy_page(h) -> None:
    get = lambda key, schema=PROXY: h.gsettings_get(schema, key)  # noqa: E731
    goto(h, 'proxy', 'st-proxy-mode')
    check_page(h, 'proxy')
    h.check('proxy: mode none at first', get('mode') in (None, "'none'"), mode=get('mode'))
    for name in ('st-proxy-mode', 'proxy-script-edit', 'proxy-manual-edit', 'link-network-advanced'):
        h.check(f'proxy: {name}', bool(one(h, name)))
    h.check('proxy: no WPAD warning while the proxy is off', not one(h, 'proxy-wpad'))
    # Instellingen automatisch detecteren = GNOME's mode auto without a configuration URL (WPAD)
    if click(h, 'st-proxy-mode'):
        wait(h, lambda: get('mode') == "'auto'", 'mode auto', 10)
        h.check('proxy: the switch writes mode auto', get('mode') == "'auto'", mode=get('mode'))
        h.check('proxy: with an empty autoconfig-url (WPAD)', get('autoconfig-url') in (None, "''"),
                url=get('autoconfig-url'))
        h.check('proxy: GNOME\'s WPAD warning shows', bool(wait(h, lambda: one(h, 'proxy-wpad'), 'the warning', 5)))
        h.screenshot(f'proxy-auto.{h.lang}', 'settings')
        click(h, 'st-proxy-mode')
        wait(h, lambda: get('mode') == "'none'", 'mode none', 10)
        h.check('proxy: off again writes mode none', get('mode') == "'none'", mode=get('mode'))
    # another program writes the mode: the switch follows (gsettings.changed)
    h.gsettings_set(PROXY, 'mode', "'auto'")
    h.check('proxy: an outside write of mode auto turns the switch on',
            bool(wait(h, lambda: active(h, 'st-proxy-mode') is True, 'the switch on', 10)))
    h.gsettings_set(PROXY, 'mode', "'none'")
    h.check('proxy: an outside write of mode none turns it off',
            bool(wait(h, lambda: active(h, 'st-proxy-mode') is False, 'the switch off', 10)))
    # Installatiescript gebruiken = mode auto with GNOME's Configuration URL
    url = 'http://wpad.example.org/proxy.pac'
    if click(h, 'proxy-script-edit') and wait(h, lambda: one(h, 'st-proxy-autoconfig-url'), 'the script dialog', 8):
        press(h, 'proxy-script-on', lambda: active(h, 'proxy-script-on') is True, 'proxy: Installatiescript gebruiken on')
        h.type_text('settings', 'st-proxy-autoconfig-url', url)
        h.screenshot(f'proxy-script.{h.lang}', 'settings')
        press(h, 'w11-dialog-primary', lambda: get('mode') == "'auto'" and not one(h, 'w11-dialog-primary'),
              'proxy: the script dialog saves')
        h.check('proxy: the script address written', get('autoconfig-url') == f"'{url}'", url=get('autoconfig-url'))
        h.check('proxy: then mode auto', get('mode') == "'auto'", mode=get('mode'))
        h.check('proxy: a script is not "automatically detect"', active(h, 'st-proxy-mode') is False)
    else:
        h.check('proxy: the script dialog', False)
    # Een proxyserver gebruiken = mode manual with GNOME's servers
    if click(h, 'proxy-manual-edit') and wait(h, lambda: one(h, 'st-proxy-http-host'), 'the server dialog', 8):
        press(h, 'proxy-manual-on', lambda: active(h, 'proxy-manual-on') is True, 'proxy: Een proxyserver gebruiken on')
        h.type_text('settings', 'st-proxy-http-host', 'proxy.example.org')
        h.type_text('settings', 'st-proxy-http-port', '8080')
        h.type_text('settings', 'st-proxy-ignore-hosts', 'localhost;127.0.0.1')
        press(h, 'proxy-local', lambda: active(h, 'proxy-local') is True, 'proxy: the intranet box on')
        h.screenshot(f'proxy-manual.{h.lang}', 'settings')
        press(h, 'w11-dialog-primary', lambda: get('mode') == "'manual'" and not one(h, 'w11-dialog-primary'),
              'proxy: the server dialog saves')
        got = {schema.rsplit('.', 1)[-1]: (get('host', schema), get('port', schema))
               for schema in (f'{PROXY}.http', f'{PROXY}.https', f'{PROXY}.ftp')}
        h.record('proxy.manual', {**got, 'ignore-hosts': get('ignore-hosts'), 'mode': get('mode')})
        h.check('proxy: the HTTP server (Windows\' one server)', got['http'] == ("'proxy.example.org'", '8080'), http=got['http'])
        h.check('proxy: HTTPS and FTP the same ("Dezelfde proxyserver voor alle protocollen")',
                got['https'] == got['ftp'] == ("'proxy.example.org'", '8080'), https=got['https'], ftp=got['ftp'])
        h.check('proxy: the exceptions and *.local for the intranet box',
                get('ignore-hosts') == "['localhost', '127.0.0.1', '*.local']", ignore=get('ignore-hosts'))
        h.check('proxy: then mode manual', get('mode') == "'manual'", mode=get('mode'))
        h.check('proxy: authentication-password never written',
                get('authentication-password', f'{PROXY}.http') is None)
    else:
        h.check('proxy: the server dialog', False)
    h.gsettings_set(PROXY, 'mode', "'none'")
    wait(h, lambda: active(h, 'st-proxy-mode') is False, 'proxy off', 5)


def hotspot_page(h, nm) -> None:
    goto(h, 'hotspot', 'hotspot-switch')
    check_page(h, 'hotspot')
    for name in ('hotspot-switch', 'hotspot-from', 'hotspot-edit'):
        h.check(f'hotspot: {name}', bool(one(h, name)))
    h.check('hotspot: a new hotspot shows its proposed password (no "Wachtwoord weergeven" to read a stored one)',
            not one(h, 'hotspot-show'))
    h.check('hotspot: no QR code while the hotspot is off (GNOME shows it in the active view only)',
            not one(h, 'hotspot-qr'))
    if not one(h, 'hotspot-switch'):
        return
    before = len(nm.calls())
    click(h, 'hotspot-switch')
    shown = wait(h, lambda: one(h, 'w11-dialog-primary'), 'the question', 6)
    h.check('hotspot: GNOME\'s question first (wlan0 carries Lab-5G)', bool(shown))
    if shown:
        h.screenshot(f'hotspot-question.{h.lang}', 'settings')
        press(h, 'w11-dialog-primary', lambda: bool(calls_after(nm, before, 'AddAndActivateConnection2', timeout=0.5)),
              'hotspot: the question\'s Inschakelen')
    add = calls_after(nm, before, 'AddAndActivateConnection2')
    body = add[0].get('args', '') if add else ''
    h.check('hotspot: AddAndActivateConnection2 of GNOME\'s hotspot profile',
            "'mode': <'ap'>" in body and "'method': <'shared'>" in body and "'id': <'Hotspot'>" in body, args=body[:500])
    h.check('hotspot: with the password dialog allowed', bool(add) and add[0].get('interactive') is True)
    h.check('hotspot: on, the QR code of the network it was turned on with (GNOME\'s active view)',
            bool(wait(h, lambda: one(h, 'hotspot-qr'), 'the QR code', 8)))
    h.screenshot(f'hotspot-on.{h.lang}', 'settings')


# ---- 3. ModemManager ------------------------------------------------------------------------------------------

def with_modem(h, nm, mm) -> None:
    h.close('settings')      # a new Settings (and its backend): caps.get asks the bus again
    goto(h, 'network', 'network-wifi')
    caps = h.backend('settings', 'caps.get') or {}
    h.check('caps: a modem with ModemManager', caps.get('modem') is True, modem=caps.get('modem'))
    h.check('network: Mobiel netwerk is offered with a modem', bool(wait(h, lambda: one(h, 'link-mobile'), 'link', 8)))
    h.screenshot(f'network-modem.{h.lang}', 'settings')
    goto(h, 'flightmode', 'flightmode-airplane')
    h.check('flightmode: the Mobiel netwerk switch with a modem', bool(wait(h, lambda: one(h, 'flightmode-mobile'), 'row', 8)))
    goto(h, 'mobile', 'mobile-switch')
    check_page(h, 'mobile')
    for name in ('mobile-switch', 'mobile-sim', 'mobile-unlock', 'mobile-properties'):
        h.check(f'mobile: {name}', bool(one(h, name)))
    if not one(h, 'mobile-unlock'):
        return
    before = len(mm.calls())
    click(h, 'mobile-unlock')     # the info bar's button: on the page itself, a real click
    if not wait(h, lambda: one(h, 'mobile-unlock-pin'), 'the unlock dialog', 8):
        h.check('mobile: the unlock dialog', False)
        return
    h.screenshot(f'mobile-unlock.{h.lang}', 'settings')
    h.type_text('settings', 'mobile-unlock-pin', '1234')
    press(h, 'w11-dialog-primary', lambda: bool(calls_after(mm, before, 'SendPin', timeout=0.5)), 'mobile: the PIN sent')
    send = calls_after(mm, before, 'SendPin')
    h.check('mobile: SendPin with the typed PIN', bool(send) and send[0].get('args') == "('1234',)", calls=send[:1])
    h.check('mobile: SendPin with the password dialog allowed', bool(send) and send[0].get('interactive') is True)
    data = wait(h, lambda: one(h, 'st-mobile-autoconnect'), 'the data rows', 15)
    h.check('mobile: Mobiele gegevens after the unlock', bool(data))
    if not data:
        return
    h.sleep(1.0)
    h.screenshot(f'mobile-unlocked.{h.lang}', 'settings')
    # the SIM's gsm profile KPN Internet (autoconnect true, home-only true): data off = autoconnect false (it is not
    # active, so nothing to deactivate), Roamen = home-only false
    h.check('mobile: Mobiele gegevens on (the profile connects automatically)', active(h, 'st-mobile-autoconnect') is True)
    switch_writes(h, nm, 'st-mobile-autoconnect', GSM_CONN, "'autoconnect': <false>", 'mobile data (off)',
                  ("'apn': <'portalmmm.nl'>", f"'sim-id': <'{ICCID}'>"))
    roam = wait(h, lambda: (lambda n: n if n and n.get('sensitive') is not False else None)(one(h, 'st-mobile-home-only')),
                'the roaming row', 10)
    if roam:
        switch_writes(h, nm, lambda: choose(h, 'st-mobile-home-only', TEXT[h.lang]['roam']), GSM_CONN,
                      "'home-only': <false>", 'mobile roaming (Roamen)', ("'apn': <'portalmmm.nl'>",))
    else:
        h.check('mobile: the Roaming drop-down', False)


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
    rfkill = start(h, GSD_RFKILL, 'gsd-rfkill')
    mm = None
    try:
        network_page(h, nm)
        wifi_page(h, nm)
        wifi_network_page(h, nm)
        known_and_ethernet(h, nm)
        vpn_page(h, nm)
        if rfkill:
            flightmode_page(h, rfkill)
            flightmode_hardware(h, rfkill)
        proxy_page(h)
        hotspot_page(h, nm)
        mm = start_fake(h, 'modemmanager')
        if mm:
            with_modem(h, nm, mm)
    finally:
        reached = [d for d in DIALOG_CLICKS if d['path'] == 'real']
        h.record('dialogClicks', DIALOG_CLICKS)
        h.check('the widgets inside kit dialogs take real pointer clicks', len(reached) == len(DIALOG_CLICKS), real=len(reached), all=len(DIALOG_CLICKS))
        h.check('every dialog action was done (real or dev click)', all(d['done'] for d in DIALOG_CLICKS),
                failed=[d['action'] for d in DIALOG_CLICKS if not d['done']])
        h.record('networkmanager.calls', [{k: c.get(k) for k in ('method', 'path', 'interactive')}
                                          for c in nm.calls()][-120:])
        if mm:
            h.record('modemmanager.calls', [{k: c.get(k) for k in ('method', 'path', 'interactive')}
                                            for c in mm.calls()][-40:])
            mm.stop()
        if rfkill:
            h.record('gsd-rfkill.calls', [{k: c.get(k) for k in ('method', 'args')} for c in rfkill.calls()][-20:])
            rfkill.stop()
        nm.stop()
        h.record('stubs.nmcli', stub_calls(h, 'nmcli'))
