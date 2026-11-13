// Instellingen › Netwerk en internet (+ Wi-Fi and Ethernet sub-pages).

import { h, clear } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { icon, wifiIcon } from '../../../core/icons.js';
import {
  SettingsCard, SettingsGroup, SettingsExpander, ToggleSwitch, Button, IconButton, Dialog, TextBox, Toast, ProgressRing,
} from '../../../core/components.js';
import { call, get, run, notice, HeroTile, copyText, ct } from '../common.js';

const t = makeT({
  nl: {
    connected: 'Verbonden', connectedSecure: 'Verbonden, beveiligd', limited: 'Verbonden, geen internet',
    portal: 'Aanmelden vereist', notConnected: 'Niet verbonden', connecting: 'Verbinden…',
    properties: 'Eigenschappen', propertiesDesc: 'IP-adres en details',
    usage: 'Netwerkgebruik', usageDesc: 'Live in Taakbeheer',
    wifi: 'Wi-Fi', wifiDesc: 'Verbinden met draadloze netwerken',
    wifiOff: 'Uit', wifiNA: 'Er is geen Wi-Fi-adapter gevonden op deze computer.',
    ethernet: 'Ethernet', ethernetDesc: 'Verbinding via een netwerkkabel',
    airplane: 'Vliegtuigmodus', airplaneDesc: 'Alle draadloze verbindingen uitzetten, zoals Wi-Fi en Bluetooth',
    na: 'Netwerkinformatie is nu niet beschikbaar. Probeer het later opnieuw.',
    networks: 'Beschikbare netwerken weergeven', networksDesc: 'Kies een netwerk om verbinding te maken',
    refresh: 'Vernieuwen', scanning: 'Zoeken naar netwerken…', noNetworks: 'Geen netwerken gevonden',
    secured: 'Beveiligd', open: 'Open', saved: 'Opgeslagen',
    connect: 'Verbinden', disconnect: 'Verbinding verbreken',
    pwTitle: 'Verbinden met “{ssid}”', pwLabel: 'Netwerkbeveiligingssleutel',
    pwHint: 'Het wachtwoord van je wifi staat vaak op een sticker op je router.',
    pwEmpty: 'Typ het wachtwoord.',
    pwChanged: 'Het opgeslagen wachtwoord werkt niet meer. Typ het nieuwe wachtwoord.',
    openTitle: 'Verbinden met een open netwerk?',
    openBody: '“{ssid}” is niet beveiligd. Anderen op dit netwerk kunnen mogelijk zien wat je doet. Gebruik het liever niet voor bankzaken.',
    connectFailed: 'Verbinden lukte niet',
    connectedTo: 'Verbonden met {ssid}',
    disconnectQ: 'Verbinding met “{ssid}” verbreken?',
    disconnectBody: 'Je bent dan niet meer verbonden met internet via dit netwerk.',
    iface: 'Netwerkadapter', ipv4: 'IPv4-adres', ipv6: 'IPv6-adres', state: 'Status', copy: 'Kopiëren',
    noEthernet: 'Geen netwerkkabel gevonden',
    noEthernetBody: 'Sluit een netwerkkabel aan om via Ethernet te verbinden.',
    related: 'Gerelateerde instellingen',
  },
  en: {
    connected: 'Connected', connectedSecure: 'Connected, secured', limited: 'Connected, no internet',
    portal: 'Sign-in required', notConnected: 'Not connected', connecting: 'Connecting…',
    properties: 'Properties', propertiesDesc: 'IP address and details',
    usage: 'Network usage', usageDesc: 'Live in Task Manager',
    wifi: 'Wi-Fi', wifiDesc: 'Connect to wireless networks',
    wifiOff: 'Off', wifiNA: 'No Wi-Fi adapter was found on this computer.',
    ethernet: 'Ethernet', ethernetDesc: 'Connection through a network cable',
    airplane: 'Flight mode', airplaneDesc: 'Stop all wireless communication, such as Wi-Fi and Bluetooth',
    na: 'Network information isn’t available right now. Try again later.',
    networks: 'Show available networks', networksDesc: 'Choose a network to connect',
    refresh: 'Refresh', scanning: 'Looking for networks…', noNetworks: 'No networks found',
    secured: 'Secured', open: 'Open', saved: 'Saved',
    connect: 'Connect', disconnect: 'Disconnect',
    pwTitle: 'Connect to “{ssid}”', pwLabel: 'Network security key',
    pwHint: 'Your Wi-Fi password is often on a sticker on your router.',
    pwEmpty: 'Type the password.',
    pwChanged: 'The saved password no longer works. Type the new password.',
    openTitle: 'Connect to an open network?',
    openBody: '“{ssid}” isn’t secured. Other people on this network might see what you do. It’s best not to use it for banking.',
    connectFailed: 'Couldn’t connect',
    connectedTo: 'Connected to {ssid}',
    disconnectQ: 'Disconnect from “{ssid}”?',
    disconnectBody: 'You’ll no longer be connected to the internet through this network.',
    iface: 'Network adapter', ipv4: 'IPv4 address', ipv6: 'IPv6 address', state: 'Status', copy: 'Copy',
    noEthernet: 'No network cable found',
    noEthernetBody: 'Plug in a network cable to connect over Ethernet.',
    related: 'Related settings',
  },
});

function statusText(net) {
  if (!net) return t('notConnected');
  const p = net.primary || {};
  if (!p.type || p.type === 'none') return t('notConnected');
  if (net.connectivity === 'limited' || net.connectivity === 'none') return t('limited');
  if (net.connectivity === 'portal') return t('portal');
  return p.type === 'wifi' ? t('connectedSecure') : t('connected');
}

function netGlyph(net) {
  const p = net?.primary || {};
  if (net?.airplane) return 'airplane';
  if (p.type === 'wifi') return wifiIcon(p.strength ?? 70);
  if (p.type === 'ethernet') return 'ethernet';
  return 'globe';
}

/** "Verbonden met …", or the Testmodus notice when the host only pretended (--dry-run). */
function connectedToast(n, dryRun) {
  if (dryRun) Toast.show({ severity: 'info', title: ct('dryRun'), timeout: 3500 });
  else Toast.show({ severity: 'success', title: t('connectedTo', { ssid: n.ssid }), timeout: 2500 });
}

/** Password / open-network flow; resolves true when connected. `error` pre-fills the password box (e.g. the
 *  router's password changed, so the saved one no longer works). */
export async function connectWifi(n, { error = '' } = {}) {
  if (n.secure && (!n.saved || error)) {
    const box = TextBox({ type: 'password', header: t('pwLabel'), description: t('pwHint'), autofocus: true, width: '100%' });
    if (error) box.setError(error);
    let dryRun = false;
    const r = await Dialog.show({
      title: t('pwTitle', { ssid: n.ssid }), content: box, primary: t('connect'), width: 420,
      onPrimary: async () => {
        const pw = box.getValue();
        if (!pw) { box.setError(t('pwEmpty')); return false; }
        const res = await call('system.network.connect', { ssid: n.ssid, password: pw });
        if (res.ok && res.value?.ok !== false) { dryRun = !!res.value?.dryRun; return true; }
        box.setError(res.value?.error || res.error?.message || t('connectFailed'));
        return false;
      },
    });
    if (r === 'primary') { connectedToast(n, dryRun); return true; }
    return false;
  }
  if (!n.secure && !n.saved) {
    const ok = await Dialog.confirm({ title: t('openTitle'), message: t('openBody', { ssid: n.ssid }), confirmLabel: t('connect') });
    if (!ok) return false;
  }
  const res = await call('system.network.connect', { ssid: n.ssid });
  if (res.ok && res.value?.ok !== false) { connectedToast(n, res.value?.dryRun); return true; }
  const code = res.value?.code || res.error?.code;
  const msg = res.value?.error || res.error?.message || '';
  // a saved network whose password no longer works (changed on the router): ask for the new one, like Windows
  if (n.secure && (code === 'needs_password' || code === 'wrong_password')) return connectWifi({ ...n, saved: false }, { error: t('pwChanged') });
  Toast.show({ severity: 'error', title: t('connectFailed'), message: msg, timeout: 5000 });
  return false;
}

async function ethernetIfaces() {
  const perf = await get('sysmon.performance');
  return (perf?.network || []).filter((n) => n.kind === 'ethernet');
}

export const pages = {
  async network(ctx) {
    const [nr, eths] = await Promise.all([call('system.network.get'), ethernetIfaces()]);
    const net = nr.ok ? nr.value : null;
    if (!net) return [notice({ message: t('na') })];
    let state = net;

    const heroIcon = h('span', { class: 'st-net-hero__icon' });
    const heroName = h('div', { class: 'st-net-hero__name' });
    const heroStatus = h('div', { class: 'st-net-hero__status' });
    const type = () => state.primary?.type;
    const hero = h('section', { class: 'st-net-hero' },
      heroIcon, h('div', { class: 'st-net-hero__text' }, heroName, heroStatus),
      h('div', { class: 'st-hero__tiles' },
        HeroTile({ icon: 'properties', title: t('properties'), status: t('propertiesDesc'), onClick: () => ctx.go(type() === 'ethernet' ? 'ethernet' : 'wifi') }),
        HeroTile({ icon: 'chart', title: t('usage'), status: t('usageDesc'), onClick: () => run('shell.openApp', { app: 'taskmgr', params: { page: 'performance' } }, { quietDry: true }) })));

    const wifiToggle = ToggleSwitch({
      checked: !!state.wifiEnabled, ariaLabel: t('wifi'), disabled: !state.wifiAvailable || state.airplane,
      onChange: async (on) => { if (!(await run('system.network.setWifi', { enabled: on }, { quietDry: true }))) wifiToggle.setChecked(!on); },
    });
    const wifiCard = SettingsCard({ icon: 'wifi', title: t('wifi'), description: '', control: wifiToggle, onClick: () => ctx.go('wifi') });
    const airToggle = ToggleSwitch({
      checked: !!state.airplane, ariaLabel: t('airplane'),
      onChange: async (on) => { if (!(await run('system.network.setAirplane', { enabled: on }, { quietDry: true }))) airToggle.setChecked(!on); },
    });

    function paint() {
      clear(heroIcon).append(icon(netGlyph(state), { size: 36 }));
      const p = state.primary || {};
      heroName.textContent = p.type && p.type !== 'none' ? (p.name || (p.type === 'wifi' ? t('wifi') : t('ethernet'))) : t('notConnected');
      heroStatus.textContent = state.airplane ? t('airplane') : statusText(state);
      hero.classList.toggle('is-offline', !p.type || p.type === 'none');
      wifiToggle.setChecked(!!state.wifiEnabled);
      wifiToggle.setDisabled(!state.wifiAvailable || state.airplane);
      wifiCard.setDescription(!state.wifiAvailable ? t('wifiNA')
        : state.wifiEnabled ? (p.type === 'wifi' ? `${t('connected')}: ${p.name}` : t('wifiDesc')) : t('wifiOff'));
      airToggle.setChecked(!!state.airplane);
    }
    paint();
    ctx.on('system.network.changed', (d) => { if (d) { state = d; paint(); } });

    const cards = state.wifiAvailable ? [wifiCard] : [];
    if (eths.length || state.primary?.type === 'ethernet') {
      cards.push(SettingsCard({
        icon: 'ethernet', title: t('ethernet'),
        description: state.primary?.type === 'ethernet' ? t('connected') : t('ethernetDesc'),
        onClick: () => ctx.go('ethernet'),
      }));
    }
    cards.push(SettingsCard({ icon: 'airplane', title: t('airplane'), description: t('airplaneDesc'), control: airToggle }));
    return [hero, SettingsGroup({ children: cards })];
  },

  async wifi(ctx) {
    const nr = await call('system.network.get');
    const net = nr.ok ? nr.value : null;
    if (!net) return [notice({ message: t('na') })];
    if (!net.wifiAvailable) return [notice({ message: t('wifiNA') })];
    let state = net;

    const list = h('div', { class: 'st-wifi-list', role: 'list' });
    let networks = [];
    let busy = false;
    const refreshBtn = IconButton({ icon: 'refresh', label: t('refresh'), size: 32 });

    async function load(rescan) {
      if (busy) return;
      busy = true;
      refreshBtn.disabled = true;
      if (rescan || !networks.length) clear(list).append(h('div', { class: 'st-wifi-scan' }, ProgressRing({ size: 16 }), t('scanning')));
      const r = await get('system.network.wifiList', { rescan: !!rescan }, []);
      busy = false;
      refreshBtn.disabled = false;
      if (!ctx.alive()) return;
      networks = Array.isArray(r) ? r : [];
      paintList();
    }

    function row(n) {
      const status = n.active ? (n.secure ? t('connectedSecure') : t('connected')) : [n.secure ? t('secured') : t('open'), n.saved ? t('saved') : ''].filter(Boolean).join(' · ');
      const btn = Button({
        label: n.active ? t('disconnect') : t('connect'), variant: 'standard',
        onClick: async () => {
          if (n.active) {
            const ok = await Dialog.confirm({ title: t('disconnectQ', { ssid: n.ssid }), message: t('disconnectBody'), confirmLabel: t('disconnect') });
            if (ok && await run('system.network.disconnect', {}, { quietDry: true })) load(false);
            return;
          }
          btn.disabled = true;
          btn.setLabel(t('connecting'));
          const ok = await connectWifi(n);
          btn.disabled = false;
          btn.setLabel(t('connect'));
          if (ok) load(false);
        },
      });
      const glyph = h('span', { class: 'st-wifi__icon' }, icon(wifiIcon(n.strength ?? 50), { size: 20 }),
        n.secure ? h('span', { class: 'st-wifi__lock' }, icon('lock', { size: 10 })) : null);
      return h('div', { class: ['st-wifi', n.active && 'is-active'], role: 'listitem' },
        glyph,
        h('div', { class: 'st-wifi__text' }, h('div', { class: 'st-wifi__name' }, n.ssid || '–'), h('div', { class: 'st-wifi__status' }, status)),
        btn);
    }

    function paintList() {
      clear(list);
      if (!state.wifiEnabled) return;
      const sorted = [...networks].sort((a, b) => (b.active - a.active) || (b.saved - a.saved) || (b.strength - a.strength));
      if (!sorted.length) { list.append(h('div', { class: 'st-wifi-scan' }, t('noNetworks'))); return; }
      list.append(...sorted.map(row));
    }

    const toggle = ToggleSwitch({
      checked: !!state.wifiEnabled, ariaLabel: t('wifi'), disabled: state.airplane,
      onChange: async (on) => {
        if (!(await run('system.network.setWifi', { enabled: on }, { quietDry: true }))) { toggle.setChecked(!on); return; }
        state = { ...state, wifiEnabled: on };
        exp.hidden = !on;
        if (on) load(true); else paintList();
      },
    });
    refreshBtn.addEventListener('click', (e) => { e.stopPropagation(); load(true); });
    const exp = SettingsExpander({
      icon: 'wifi-3', title: t('networks'), description: t('networksDesc'), expanded: true,
      control: refreshBtn, children: [list],
    });
    exp.classList.add('st-wifi-exp');
    exp.hidden = !state.wifiEnabled;
    ctx.on('system.network.changed', (d) => {
      if (!d) return;
      const was = state.wifiEnabled;
      state = d;
      toggle.setChecked(!!d.wifiEnabled);
      toggle.setDisabled(!!d.airplane);
      exp.hidden = !d.wifiEnabled;
      if (d.wifiEnabled && !was) load(true);
    });
    if (state.wifiEnabled) load(false);

    return [
      SettingsGroup({ children: [SettingsCard({ icon: 'wifi', title: t('wifi'), description: state.airplane ? t('airplane') : t('wifiDesc'), control: toggle }), exp] }),
    ];
  },

  async ethernet(ctx) {
    const [nr, eths] = await Promise.all([call('system.network.get'), ethernetIfaces()]);
    const net = nr.ok ? nr.value : null;
    const active = net?.primary?.type === 'ethernet';
    if (!eths.length && !active) {
      return [SettingsGroup({ children: [SettingsCard({ icon: 'ethernet', title: t('noEthernet'), description: t('noEthernetBody') })] })];
    }
    const out = [];
    const list = eths.length ? eths : [{ iface: net?.primary?.name || 'eth0', name: t('ethernet'), ipv4: '', ipv6: '' }];
    for (const e of list) {
      const connected = !!(e.ipv4 || e.ipv6) || active;
      const rows = [[t('state'), connected ? t('connected') : t('notConnected')], [t('iface'), e.iface], [t('ipv4'), e.ipv4], [t('ipv6'), e.ipv6]];
      const copyB = Button({
        label: t('copy'),
        onClick: async (ev) => {
          ev.stopPropagation();
          const ok = await copyText(rows.filter((r) => r[1]).map(([k, v]) => `${k}\t${v}`).join('\n'));
          Toast.show({ severity: ok ? 'success' : 'error', title: ok ? ct('copied') : ct('copyFailed'), timeout: 2000 });
        },
      });
      out.push(SettingsExpander({
        icon: 'ethernet', title: e.name && e.name !== e.iface ? `${e.name} (${e.iface})` : e.iface,
        description: connected ? t('connected') : t('notConnected'), expanded: true, control: copyB,
        children: [h('dl', { class: 'st-specs selectable' }, rows.filter((r) => r[1]).map(([k, v]) => [h('dt', null, k), h('dd', null, v)]))],
      }));
    }
    return [SettingsGroup({ children: out })];
  },
};
