// Win11 Shell — Snelle instellingen (Quick Settings, Win+A). Owned by the flyouts agent.
//
//   setup(ctx) registers flyout 'quick' (right-aligned above the tray) and command
//   'quick-settings'. Layout like Windows 11 23H2:
//     [media card]            separate Acrylic card when a player exists (media.list/changed)
//     [tiles 3×2]             Wi-Fi ▸ · Bluetooth ▸ · Vliegtuigmodus · Nachtlampje · Donkere modus · Niet storen
//     [sliders]               helderheid (only when available) · volume (mute button, ▸ output devices)
//     [footer]                battery % (when present) · ⚙ all settings
//   Sub-pages (Wi-Fi networks, Bluetooth devices, sound output) slide in with a back arrow.
//   Live updates through system.*.changed, media.changed and settings.changed.

import { h, clear, loadCss, animate, throttle } from '../core/dom.js';
import { icon, wifiIcon, volumeIcon } from '../core/icons.js';
import { makeT, onLangChange, collator } from '../core/i18n.js';
import { ToggleSwitch, Slider, Button, IconButton, TextBox, Checkbox, ProgressBar, AppIcon } from '../core/components.js';
import { getSetting, setSetting, onSettings, getShellInfo, themeMode } from '../core/theme.js';
import { bridge as coreBridge } from '../core/bridge.js';
import { settleHost } from './flyouthost.js';

/**
 * animate() that always settles — on finish, or after its duration when the timeline is
 * throttled (hidden surface, headless) — then drops effects without fill:'forwards'.
 * Returns a Promise<Animation|null> with the Animation on `.anim`.
 */
function play(el, frames, opts = {}) {
  const a = animate(el, frames, opts);
  if (!a) { const p = Promise.resolve(null); p.anim = null; return p; }
  const ms = (Number(opts.duration) || 0) + (Number(opts.delay) || 0) + 60;
  const p = new Promise((resolve) => {
    let done = false;
    const end = () => {
      if (done) return;
      done = true;
      if (opts.fill !== 'forwards' && opts.fill !== 'both') { try { a.cancel(); } catch { /* ignore */ } }
      resolve(a);
    };
    a.finished.then(end, end);
    setTimeout(end, ms);
  });
  p.anim = a;
  return p;
}

let host = null;

const t = makeT({
  nl: {
    title: 'Snelle instellingen',
    wifi: 'Wi-Fi', wifiMore: 'Wi-Fi-netwerken weergeven', bluetooth: 'Bluetooth', btMore: 'Bluetooth-apparaten weergeven',
    airplane: 'Vliegtuigmodus', nightLight: 'Nachtlampje', darkMode: 'Donkere modus', dnd: 'Niet storen',
    ethernet: 'Ethernet', notConnected: 'Niet verbonden', network: 'Netwerk',
    brightness: 'Helderheid', volume: 'Volume', mute: 'Geluid uit', unmute: 'Geluid aan',
    output: 'Geluidsuitvoer', outputMore: 'Een geluidsuitvoer kiezen',
    allSettings: 'Alle instellingen', battery: 'Batterij: {p}', batteryCharging: 'Batterij: {p} (wordt opgeladen)',
    batteryFull: 'Batterij: {p} (aangesloten)',
    back: 'Terug',
    moreWifi: 'Meer Wi-Fi-instellingen', moreBt: 'Meer Bluetooth-instellingen', moreSound: 'Meer volume-instellingen',
    connected: 'Verbonden', connectedSecure: 'Verbonden, beveiligd', connectedOpen: 'Verbonden, open', secure: 'Beveiligd', open: 'Open',
    saved: 'Beveiligd, opgeslagen',
    connect: 'Verbinding maken', disconnect: 'Verbinding verbreken', connecting: 'Verbinding maken…', disconnecting: 'Verbinding verbreken…',
    autoConnect: 'Automatisch verbinding maken', properties: 'Eigenschappen',
    enterKey: 'Voer de netwerkbeveiligingssleutel in', password: 'Wachtwoord', next: 'Volgende', cancel: 'Annuleren',
    openWarning: 'Anderen kunnen mogelijk zien wat je via dit netwerk verstuurt.',
    scanning: 'Zoeken naar netwerken…', noNetworks: 'Geen netwerken gevonden', wifiOff: 'Wi-Fi is uitgeschakeld',
    wifiOffHint: 'Zet Wi-Fi hierboven aan om netwerken te zien.', airplaneOn: 'Vliegtuigmodus staat aan',
    airplaneHint: 'Zet vliegtuigmodus uit om weer verbinding te maken.',
    connectFailed: 'Kan geen verbinding maken met dit netwerk.',
    btOff: 'Bluetooth is uitgeschakeld', btOffHint: 'Zet Bluetooth hierboven aan om je apparaten te gebruiken.',
    btNone: 'Geen gekoppelde apparaten', btNoneHint: 'Kies hieronder “Apparaat toevoegen” om er een te koppelen.',
    paired: 'Gekoppeld', btBattery: '{p} batterij', pairNew: 'Apparaat toevoegen',
    btStuck: 'Bluetooth reageert niet.', btReset: 'Bluetooth opnieuw starten', btResetting: 'Bluetooth wordt opnieuw gestart…',
    btResetOk: 'Bluetooth werkt weer.',
    noSinks: 'Geen geluidsapparaten gevonden',
    playing: 'Speelt af', paused: 'Gepauzeerd', previous: 'Vorige', next2: 'Volgende', play: 'Afspelen', pause: 'Onderbreken',
    otherPlayer: 'Andere speler', unavailable: 'Niet beschikbaar',
    dryRun: 'Testmodus: er is niets veranderd.',
    failed: 'Dat is niet gelukt. Probeer het opnieuw.', on: 'Aan', off: 'Uit',
  },
  en: {
    title: 'Quick settings',
    wifi: 'Wi-Fi', wifiMore: 'Show Wi-Fi networks', bluetooth: 'Bluetooth', btMore: 'Show Bluetooth devices',
    airplane: 'Airplane mode', nightLight: 'Night light', darkMode: 'Dark mode', dnd: 'Do not disturb',
    ethernet: 'Ethernet', notConnected: 'Not connected', network: 'Network',
    brightness: 'Brightness', volume: 'Volume', mute: 'Mute', unmute: 'Unmute',
    output: 'Sound output', outputMore: 'Select a sound output',
    allSettings: 'All settings', battery: 'Battery: {p}', batteryCharging: 'Battery: {p} (charging)',
    batteryFull: 'Battery: {p} (plugged in)',
    back: 'Back',
    moreWifi: 'More Wi-Fi settings', moreBt: 'More Bluetooth settings', moreSound: 'More volume settings',
    connected: 'Connected', connectedSecure: 'Connected, secured', connectedOpen: 'Connected, open', secure: 'Secured', open: 'Open',
    saved: 'Secured, saved',
    connect: 'Connect', disconnect: 'Disconnect', connecting: 'Connecting…', disconnecting: 'Disconnecting…',
    autoConnect: 'Connect automatically', properties: 'Properties',
    enterKey: 'Enter the network security key', password: 'Password', next: 'Next', cancel: 'Cancel',
    openWarning: 'Other people might be able to see info you send over this network.',
    scanning: 'Looking for networks…', noNetworks: 'No networks found', wifiOff: 'Wi-Fi is turned off',
    wifiOffHint: 'Turn on Wi-Fi above to see networks.', airplaneOn: 'Airplane mode is on',
    airplaneHint: 'Turn off airplane mode to connect again.',
    connectFailed: 'Can’t connect to this network.',
    btOff: 'Bluetooth is turned off', btOffHint: 'Turn on Bluetooth above to use your devices.',
    btNone: 'No paired devices', btNoneHint: 'Choose “Add device” below to pair one.',
    paired: 'Paired', btBattery: '{p} battery', pairNew: 'Add device',
    btStuck: 'Bluetooth isn’t responding.', btReset: 'Restart Bluetooth', btResetting: 'Restarting Bluetooth…',
    btResetOk: 'Bluetooth works again.',
    noSinks: 'No sound devices found',
    playing: 'Playing', paused: 'Paused', previous: 'Previous', next2: 'Next', play: 'Play', pause: 'Pause',
    otherPlayer: 'Other player', unavailable: 'Not available',
    dryRun: 'Test mode: nothing was changed.',
    failed: 'That didn’t work. Please try again.', on: 'On', off: 'Off',
  },
});

/* ==========================================================================
   State
   ========================================================================== */

let ctx = null;
let bridge = coreBridge;
const st = {
  audio: null, brightness: null, battery: null, network: null, bluetooth: null,
  players: [], playerIndex: 0,
  nightLight: { available: false, enabled: false },
  wifiList: null, scanning: false,
};
let ui = null; // built on first open

const pct = (n) => `${Math.round(Number(n) || 0)}%`;

function safeCall(method, args) {
  return bridge.call(method, args).catch((e) => {
    if (e?.code !== 'unknown_method') console.warn(`[quick] ${method} failed`, e);
    throw e;
  });
}

async function load(method, args) {
  try { return await bridge.call(method, args); } catch { return null; }
}

/** Open Instellingen (optionally on a page: wifi, bluetooth, sound, power, network) and close the flyout. */
function openSettings(page) {
  if (typeof ctx?.openSettings === 'function') {
    Promise.resolve(ctx.openSettings(page ? [page] : [])).catch((e) => console.warn('[quick] openSettings failed', e));
  } else {
    bridge.call('shell.openApp', { app: 'settings', params: page ? { page } : {} }).catch((e) => console.warn('[quick] openApp failed', e));
  }
  ctx?.closeFlyout('quick');
}

/* ==========================================================================
   Setup
   ========================================================================== */

export function setup(c) {
  ctx = c;
  bridge = c.bridge || coreBridge;
  loadCss('flyouts');

  ctx.registerFlyout('quick', {
    anchor: 'right',
    slide: 24,
    create: (container) => {
      host = container;
      container.classList.add('fo-host', 'fo-host--quick');
      container.addEventListener('keydown', hostKeys);
      // focus can fall back to <body> when a list re-renders: keep the keys working then too
      document.addEventListener('keydown', (e) => { if (ctx.isOpen('quick') && (e.target === document.body || e.target === document.documentElement)) hostKeys(e, true); });
      ui = buildUI();
      renderAll();
      return { el: ui.el, onOpen, onClose };
    },
  });
  ctx.onCommand('quick-settings', (args = {}) => ctx.toggleFlyout('quick', { anchor: 'right', source: 'command', ...args }));

  bridge.on('system.audio.changed', (d) => { st.audio = d; ui && renderVolume(); ui?.sub?.kind === 'sound' && ui.sub.render(); });
  bridge.on('system.brightness.changed', (d) => { st.brightness = d; ui && renderBrightness(); });
  bridge.on('system.battery.changed', (d) => { st.battery = d; ui && renderFooter(); });
  bridge.on('system.network.changed', (d) => {
    st.network = d;
    if (!ui) return;
    renderTiles();
    if (ui.sub?.kind === 'wifi') refreshWifiList(false);
  });
  bridge.on('system.bluetooth.changed', (d) => { st.bluetooth = d; if (ui) { renderTiles(); if (ui.sub?.kind === 'bluetooth') ui.sub.render(); } });
  bridge.on('media.changed', (d) => { st.players = Array.isArray(d?.players) ? d.players : []; ui && renderMedia(); });
  onSettings((s, path) => {
    if (!ui) return;
    if (!path || /^(notifications|theme|nightLight)/.test(path)) renderTiles();
  });
  // language switched in Instellingen → rebuild the (static) texts
  // … and fetch the host's texts again (device and network names, states), now in the new language
  onLangChange(() => {
    if (!ui || !host) return;
    if (ui.sub) closeSub({ instant: true });
    ui = buildUI();
    host.replaceChildren(ui.el);
    renderAll();
    refreshAll();
  });

  refreshAll();
}

async function refreshAll() {
  const [audio, brightness, battery, network, bluetooth, players, night] = await Promise.all([
    load('system.audio.get'), load('system.brightness.get'), load('system.battery.get'),
    load('system.network.get'), load('system.bluetooth.get'), load('media.list'), loadNightLight(),
  ]);
  Object.assign(st, { audio, brightness, battery, network, bluetooth, players: Array.isArray(players) ? players : [] });
  st.nightLight = night;
  if (ui) renderAll();
}

async function loadNightLight() {
  const enabled = !!getSetting('nightLight.enabled', false);
  try {
    const r = await bridge.call('system.nightLight.get');
    // host-services-b: {supported, enabled, temperature, tool}
    const ok = r?.supported ?? r?.available;
    return { available: ok !== false, enabled: r?.enabled ?? enabled };
  } catch (e) {
    // no getter on this host: hyprsunset only exists under Hyprland
    const hypr = getShellInfo()?.compositor === 'hyprland';
    return { available: e?.code === 'unknown_method' ? (hypr || bridge.isMock) : false, enabled };
  }
}

/* ==========================================================================
   UI construction
   ========================================================================== */

function buildUI() {
  const media = h('div', { class: 'fo-media fo-acrylic surface-flyout', hidden: true, role: 'group', 'aria-label': 'Media' });
  const notice = h('div', { class: 'fo-qs__notice', role: 'status', hidden: true });
  const tiles = h('div', { class: 'fo-qs__tiles', role: 'group', 'aria-label': t('title') });
  const sliders = h('div', { class: 'fo-qs__sliders' });
  const main = h('div', { class: 'fo-qs__page fo-qs__main' }, notice, tiles, sliders);
  const pages = h('div', { class: 'fo-qs__pages' }, main);
  const footer = h('div', { class: 'fo-qs__footer' });
  const panel = h('div', { class: 'fo-qs fo-acrylic surface-flyout', role: 'group', 'aria-label': t('title') }, pages, footer);
  const el = h('div', { class: 'fo-qs-wrap' }, media, panel);

  const u = { el, media, panel, pages, main, tiles, sliders, footer, notice, sub: null, tileMap: {} };

  // tiles
  u.tileMap.wifi = Tile({
    id: 'wifi', icon: 'wifi-3', label: t('wifi'), split: true, moreLabel: t('wifiMore'),
    onToggle: toggleWifi, onMore: () => openSub('wifi'),
  });
  u.tileMap.bluetooth = Tile({
    id: 'bluetooth', icon: 'bluetooth', label: t('bluetooth'), split: true, moreLabel: t('btMore'),
    onToggle: toggleBluetooth, onMore: () => openSub('bluetooth'),
  });
  u.tileMap.airplane = Tile({ id: 'airplane', icon: 'airplane', label: t('airplane'), onToggle: toggleAirplane });
  u.tileMap.nightLight = Tile({ id: 'nightLight', icon: 'night-light', label: t('nightLight'), onToggle: toggleNightLight });
  u.tileMap.dark = Tile({ id: 'dark', icon: 'moon', label: t('darkMode'), onToggle: toggleDark });
  u.tileMap.dnd = Tile({ id: 'dnd', icon: 'notification-off', label: t('dnd'), onToggle: toggleDnd });
  tiles.append(...Object.values(u.tileMap).map((x) => x.el));
  tiles.addEventListener('keydown', gridKeys);

  // sliders
  const brightSlider = Slider({
    min: 0, max: 100, value: 50, ariaLabel: t('brightness'), format: (v) => String(v),
    onInput: throttle((v) => setBrightness(v), 90), onChange: (v) => setBrightness(v, true),
  });
  u.brightRow = h('div', { class: 'fo-qs__slider-row' },
    h('span', { class: 'fo-qs__slider-glyph', dataset: { tooltip: t('brightness') } }, icon('brightness', { size: 16 })),
    brightSlider, h('span', { class: 'fo-qs__slider-end' }));
  u.brightSlider = brightSlider;

  const muteBtn = IconButton({ icon: 'volume-2', label: t('mute'), size: 40, iconSize: 16, onClick: toggleMute, cls: 'fo-qs__mute' });
  const volSlider = Slider({
    min: 0, max: 100, value: 50, ariaLabel: t('volume'), format: (v) => String(v),
    onInput: throttle((v) => setVolume(v), 80), onChange: (v) => setVolume(v, true),
  });
  const outBtn = IconButton({ icon: 'chevron-right', label: t('outputMore'), size: 32, iconSize: 12, onClick: () => openSub('sound'), cls: 'fo-qs__out' });
  u.volRow = h('div', { class: 'fo-qs__slider-row' }, muteBtn, volSlider, outBtn);
  Object.assign(u, { muteBtn, volSlider, outBtn });
  sliders.append(u.brightRow, u.volRow);

  // footer
  u.batteryBtn = h('button', { type: 'button', class: 'fo-qs__battery', onClick: () => openSettings('power') });
  u.settingsBtn = IconButton({ icon: 'settings', label: t('allSettings'), size: 40, iconSize: 16, onClick: () => openSettings(null) });
  footer.append(u.batteryBtn, h('span', { class: 'grow' }), u.settingsBtn);

  return u;
}

/** Keys on the flyout itself: the first arrow press (nothing focused yet) enters the tiles; Backspace = back. */
function hostKeys(e, outside = false) {
  if (!ui || e.defaultPrevented) return;
  if ((outside || e.target === host || e.target === ui.el) && ['ArrowDown', 'ArrowRight', 'ArrowUp', 'ArrowLeft'].includes(e.key)) {
    e.preventDefault();
    (ui.sub ? ui.sub.el.querySelector('button:not(:disabled), [tabindex="0"]') : ui.tiles.querySelector('.fo-tile:not([hidden]) button:not(:disabled)'))?.focus();
    return;
  }
  if (e.key === 'Backspace' && ui.sub && !e.target.closest?.('input, textarea')) { e.preventDefault(); closeSub(); }
}

/** Win11 quick action tile: 96×48 button (optionally split with a ▸ part) + label below. */
function Tile({ id, icon: ic, label, split = false, moreLabel, onToggle, onMore }) {
  const glyph = h('span', { class: 'fo-tile__glyph' }, icon(ic, { size: 16 }));
  const main = h('button', { type: 'button', class: 'fo-tile__main', 'aria-pressed': 'false', 'aria-label': label }, glyph);
  const more = split ? h('button', { type: 'button', class: 'fo-tile__more', 'aria-label': moreLabel, dataset: { tooltip: moreLabel } }, icon('chevron-right', { size: 12 })) : null;
  const box = h('div', { class: ['fo-tile__box', split && 'is-split'] }, main, more);
  const lbl = h('div', { class: 'fo-tile__label' }, label);
  const el = h('div', { class: 'fo-tile', dataset: { tile: id } }, box, lbl);
  main.addEventListener('click', () => { if (!el.classList.contains('is-disabled')) onToggle?.(!el.classList.contains('is-on')); });
  more?.addEventListener('click', () => onMore?.());
  let curIcon = ic;
  const api = {
    el,
    set({ on, text, glyph: g, hidden, disabled, busy, toggleLabel, tooltip }) {
      if (hidden !== undefined) el.hidden = !!hidden;
      if (on !== undefined) { el.classList.toggle('is-on', !!on); main.setAttribute('aria-pressed', String(!!on)); }
      if (text !== undefined) { lbl.textContent = text; if (text.length > 14) lbl.dataset.tooltip = text; else delete lbl.dataset.tooltip; }
      if (g && g !== curIcon) { curIcon = g; glyph.replaceChildren(icon(g, { size: 16 })); }
      if (disabled !== undefined) { el.classList.toggle('is-disabled', !!disabled); main.disabled = !!disabled; if (more) more.disabled = false; }
      if (busy !== undefined) el.classList.toggle('is-busy', !!busy);
      if (toggleLabel) main.setAttribute('aria-label', toggleLabel);
      if (tooltip !== undefined) { if (tooltip) main.dataset.tooltip = tooltip; else delete main.dataset.tooltip; }
    },
    get on() { return el.classList.contains('is-on'); },
  };
  return api;
}

/** Arrow-key navigation between the tile buttons (geometry based, like Win11). */
function gridKeys(e) {
  const keys = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] };
  const k = keys[e.key];
  if (!k) return;
  const btns = [...ui.tiles.querySelectorAll('.fo-tile:not([hidden]) button:not(:disabled)')];
  const cur = btns.indexOf(document.activeElement);
  if (cur < 0) return;
  e.preventDefault();
  if (k[1] === 0) {
    const n = btns[cur + k[0]];
    n?.focus();
    return;
  }
  const r0 = btns[cur].getBoundingClientRect();
  const cx = r0.left + r0.width / 2;
  let best = null, bestScore = Infinity;
  for (const b of btns) {
    const r = b.getBoundingClientRect();
    const dy = (r.top - r0.top) * k[1];
    if (dy <= 4) continue;
    const score = dy * 4 + Math.abs(r.left + r.width / 2 - cx);
    if (score < bestScore) { bestScore = score; best = b; }
  }
  if (best) best.focus();
  else if (k[1] > 0) ui.volSlider?.isConnected && !ui.volRow.hidden && (ui.brightRow.hidden ? ui.volSlider : ui.brightSlider).focus();
}

/* ==========================================================================
   Rendering
   ========================================================================== */

function renderAll() {
  renderMedia();
  renderTiles();
  renderBrightness();
  renderVolume();
  renderFooter();
}

function renderTiles() {
  if (!ui) return;
  const T = ui.tileMap;
  const n = st.network;
  // Wi-Fi (or a wired "network" tile when the pc has no Wi-Fi)
  if (!n) {
    T.wifi.set({ hidden: true });
    T.airplane.set({ hidden: true });
  } else if (n.wifiAvailable === false) {
    const wired = n.primary?.type === 'ethernet';
    T.wifi.set({
      hidden: false, on: wired, glyph: wired ? 'ethernet' : 'network', text: wired ? t('ethernet') : t('notConnected'),
      toggleLabel: t('network'),
    });
    T.wifi.el.classList.add('is-static');
    T.airplane.set({ hidden: false, on: !!n.airplane });
  } else {
    const on = !!n.wifiEnabled && !n.airplane;
    const connected = on && n.primary?.type === 'wifi';
    T.wifi.el.classList.remove('is-static');
    T.wifi.set({
      hidden: false, on,
      glyph: !on ? 'wifi-off' : connected ? wifiIcon(n.primary.strength) : 'wifi',
      text: connected ? n.primary.name || t('wifi') : on ? t('wifi') : t('wifi'),
      toggleLabel: `${t('wifi')}: ${on ? t('on') : t('off')}`,
    });
    T.airplane.set({ hidden: false, on: !!n.airplane });
  }
  // Bluetooth
  const b = st.bluetooth;
  if (!b || b.available === false) T.bluetooth.set({ hidden: true });
  else {
    const conn = (b.devices || []).filter((d) => d.connected);
    T.bluetooth.set({
      hidden: false, on: !!b.powered,
      text: b.powered && conn.length === 1 ? conn[0].name : t('bluetooth'),
      toggleLabel: `${t('bluetooth')}: ${b.powered ? t('on') : t('off')}`,
    });
  }
  // Night light
  T.nightLight.set({ hidden: !st.nightLight.available, on: !!st.nightLight.enabled });
  // Dark mode & DND (settings)
  T.dark.set({ on: themeMode() === 'dark' });
  T.dnd.set({ on: !!getSetting('notifications.dnd', false) });
}

function renderBrightness() {
  if (!ui) return;
  const b = st.brightness;
  const avail = !!(b && b.available && b.value != null);
  ui.brightRow.hidden = !avail;
  if (avail && !recentlyTouched(ui.brightSlider)) ui.brightSlider.setValue(Math.round(b.value));
}

function renderVolume() {
  if (!ui) return;
  const a = st.audio;
  const avail = !!(a && a.available !== false && a.volume != null);
  ui.volRow.hidden = !avail;
  if (!avail) return;
  if (!recentlyTouched(ui.volSlider)) ui.volSlider.setValue(Math.round(a.volume));
  const v = recentlyTouched(ui.volSlider) ? ui.volSlider.getValue() : a.volume;
  ui.muteBtn.setIcon(volumeIcon(v, a.muted));
  ui.muteBtn.setLabel(a.muted ? t('unmute') : t('mute'));
  ui.volRow.classList.toggle('is-muted', !!a.muted);
  ui.outBtn.hidden = !(a.sinks && a.sinks.length > 0);
}

function renderFooter() {
  if (!ui) return;
  const b = st.battery;
  const present = !!(b && b.present && b.percent != null);
  ui.batteryBtn.hidden = !present;
  if (!present) return;
  const charging = b.state === 'charging' || (b.onAC && b.state !== 'fully-charged' && b.state !== 'full');
  const full = b.onAC && !charging;
  const label = (charging ? t('batteryCharging', { p: pct(b.percent) }) : full ? t('batteryFull', { p: pct(b.percent) }) : t('battery', { p: pct(b.percent) }));
  ui.batteryBtn.replaceChildren(batteryGlyph(b.percent, charging || full), h('span', { class: 'tabular' }, pct(b.percent)));
  ui.batteryBtn.setAttribute('aria-label', label);
  ui.batteryBtn.dataset.tooltip = label;
}

/** Battery glyph with a fill level (and bolt while charging), drawn like the Win11 tray icon. */
function batteryGlyph(percentage, charging) {
  const p = Math.max(0, Math.min(100, Number(percentage) || 0));
  const w = Math.max(1, Math.round((p / 100) * 10.5));
  const low = p <= 10 && !charging;
  return h('svg', { class: ['fo-battery', low && 'is-low'], viewBox: '0 0 20 20', width: 16, height: 16, 'aria-hidden': 'true' },
    h('rect', { x: 2, y: 5.5, width: 14, height: 9, rx: 2, fill: 'none', stroke: 'currentColor', 'stroke-width': 1.35 }),
    h('path', { d: 'M18 8.5v3', stroke: 'currentColor', 'stroke-width': 1.35, 'stroke-linecap': 'round' }),
    h('rect', { class: 'fo-battery__fill', x: 3.75, y: 7.25, width: w, height: 5.5, rx: 0.9 }),
    charging ? h('path', { d: 'M10.4 6.6 7.9 10.3h2.4l-1 3.1 2.9-4.1H9.8Z', class: 'fo-battery__bolt' }) : null);
}

const touched = new WeakMap();
function markTouched(el) { touched.set(el, performance.now()); }
function recentlyTouched(el) {
  return el.classList.contains('is-dragging') || (touched.has(el) && performance.now() - touched.get(el) < 700);
}

/* ---------------------------------------------------------------- media */

function playerIcon(p) {
  if (p.icon) return p.icon;
  if (p.desktopId) return p.desktopId.replace(/\.desktop$/, '');
  return String(p.identity || 'audio-player').toLowerCase().replace(/\s+/g, '-');
}

/** MPRIS status: the host sends 'playing'|'paused'|'stopped' (MPRIS itself uses 'Playing'…). */
const status = (p) => String(p?.status || '').toLowerCase();
const isPlaying = (p) => status(p) === 'playing';

/**
 * Album art the shell may show: local only (data:/blob:, file:// or an absolute path through the
 * host's image-only /file). Remote http(s) art from a web page's media session or an MPRIS app is
 * never fetched by the shell (privacy, SPEC §4 "no external network"): the player icon shows instead.
 */
function artSrc(url) {
  if (!url) return '';
  const s = String(url);
  if (/^(data:|blob:)/.test(s)) return s;
  if (s.startsWith('file://')) { try { return bridge.fileUrl(decodeURIComponent(new URL(s).pathname)); } catch { return ''; } }
  if (s.startsWith('/')) return bridge.fileUrl(s);
  return '';
}

function renderMedia() {
  if (!ui) return;
  const players = st.players.filter((p) => p && (p.title || isPlaying(p) || status(p) === 'paused'));
  const box = ui.media;
  if (!players.length) { box.hidden = true; clear(box); return; }
  if (st.playerIndex >= players.length) st.playerIndex = 0;
  // prefer the playing one when the user has not picked
  if (!box.dataset.picked) {
    const playing = players.findIndex(isPlaying);
    if (playing >= 0) st.playerIndex = playing;
  }
  const p = players[st.playerIndex];
  const playingNow = isPlaying(p);
  const ctl = (action) => bridge.call('media.control', { id: p.id, action }).catch((e) => flash(e?.message || t('failed')));
  const src = p.artPath ? bridge.fileUrl(p.artPath) : artSrc(p.artUrl);
  const art = src
    ? h('img', { class: 'fo-media__art', src, alt: '', draggable: false, onError: (e) => e.target.replaceWith(artFallback()) })
    : artFallback();
  const pager = players.length > 1 ? h('div', { class: 'fo-media__pager' },
    IconButton({ icon: 'chevron-left', label: t('otherPlayer'), size: 24, iconSize: 10, onClick: () => { box.dataset.picked = '1'; st.playerIndex = (st.playerIndex + players.length - 1) % players.length; renderMedia(); } }),
    h('span', { class: 'caption text-2 tabular' }, `${st.playerIndex + 1}/${players.length}`),
    IconButton({ icon: 'chevron-right', label: t('otherPlayer'), size: 24, iconSize: 10, onClick: () => { box.dataset.picked = '1'; st.playerIndex = (st.playerIndex + 1) % players.length; renderMedia(); } })) : null;

  const keepFocus = box.contains(document.activeElement) ? document.activeElement.dataset.action : null;
  const btn = (action, ic, label, enabled, cls) => {
    const b = IconButton({ icon: ic, label, size: 40, iconSize: 16, disabled: !enabled, onClick: () => ctl(action), cls });
    b.dataset.action = action;
    return b;
  };
  box.replaceChildren(
    h('div', { class: 'fo-media__top' },
      AppIcon({ name: playerIcon(p), size: 16, fallback: 'music' }),
      h('span', { class: 'fo-media__app caption ellipsis' }, p.identity || ''),
      h('span', { class: 'grow' }), pager),
    h('div', { class: 'fo-media__mid' },
      h('div', { class: 'fo-media__text' },
        h('div', { class: 'fo-media__title body-strong ellipsis', dataset: (p.title || '').length > 32 ? { tooltip: p.title } : {} }, p.title || p.identity || ''),
        h('div', { class: 'fo-media__artist caption text-2 ellipsis' }, p.artist || (playingNow ? t('playing') : t('paused')))),
      art),
    h('div', { class: 'fo-media__controls' },
      btn('previous', 'previous', t('previous'), p.canPrevious !== false),
      btn('playPause', playingNow ? 'pause' : 'play', playingNow ? t('pause') : t('play'), playingNow ? p.canPause !== false : p.canPlay !== false, 'fo-media__play'),
      btn('next', 'next', t('next2'), p.canNext !== false)));
  box.hidden = false;
  if (keepFocus) box.querySelector(`[data-action="${keepFocus}"]`)?.focus();
}

function artFallback() {
  return h('div', { class: 'fo-media__art is-empty' }, icon('music', { size: 24 }));
}

/* ==========================================================================
   Actions (optimistic, revert on error)
   ========================================================================== */

let noticeTimer = 0;
/** Short inline note on the visible page (main page or the open sub-page); kind 'warning' | 'info'. */
function flash(message, kind = 'warning') {
  if (!ui) return;
  const content = () => [icon(kind === 'info' ? 'info' : 'warning', { size: 16 }), h('span', null, message)];
  const page = ui.sub?.el;
  if (page) {
    page.querySelector('.fo-sub__notice')?.remove();
    const n = h('div', { class: ['fo-qs__notice', 'fo-sub__notice', kind === 'info' && 'is-info'], role: 'status' }, ...content());
    const head = page.querySelector('.fo-sub__header');
    if (head) head.after(n); else page.prepend(n);
    setTimeout(() => n.remove(), 5000);
    return;
  }
  ui.notice.classList.toggle('is-info', kind === 'info');
  ui.notice.replaceChildren(...content());
  ui.notice.hidden = false;
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => { ui.notice.hidden = true; }, 5000);
}

/** The host runs with --dry-run (preview / testing): it answered {ok, dryRun} and changed nothing. */
const isDryRun = (r) => !!(r && typeof r === 'object' && r.dryRun);

/**
 * Run a change optimistically. On an error, or in test mode (nothing really changed and the host
 * will not report a change), undo the optimistic state and say so instead of pretending.
 */
async function guarded(fn, revert) {
  try {
    const r = await fn();
    if (isDryRun(r)) { revert?.(); flash(t('dryRun'), 'info'); }
    return r;
  } catch (e) {
    revert?.();
    flash(e?.code === 'unknown_method' ? t('unavailable') : (e?.message || t('failed')));
    return null;
  }
}

/** After a slider was released in test mode: show the real value again. */
async function restoreAfterDryRun(method, key, render, slider) {
  flash(t('dryRun'), 'info');
  const fresh = await load(method);
  if (slider) touched.delete(slider);
  if (fresh) { st[key] = fresh; render(); }
}

/** After undoing an optimistic toggle: the tiles and an open Wi-Fi/Bluetooth sub-page show it again. */
function rerender() {
  renderTiles();
  if (ui?.sub?.kind === 'wifi' || ui?.sub?.kind === 'bluetooth') ui.sub.render?.();
}

function toggleWifi(on) {
  if (st.network?.wifiAvailable === false) { openSettings('network'); return; }
  const prev = st.network;
  st.network = { ...prev, wifiEnabled: on, primary: on ? prev.primary : { type: 'none', name: '', strength: null } };
  renderTiles();
  guarded(() => safeCall('system.network.setWifi', { enabled: on }), () => { st.network = prev; rerender(); });
}

function toggleBluetooth(on) {
  const prev = st.bluetooth;
  st.bluetooth = { ...prev, powered: on };
  renderTiles();
  guarded(() => safeCall('system.bluetooth.setPowered', { powered: on }), () => { st.bluetooth = prev; rerender(); });
}

function toggleAirplane(on) {
  const prev = st.network;
  st.network = { ...prev, airplane: on };
  renderTiles();
  guarded(() => safeCall('system.network.setAirplane', { enabled: on }), () => { st.network = prev; rerender(); });
}

async function toggleNightLight(on) {
  const prev = { ...st.nightLight };
  st.nightLight.enabled = on;
  renderTiles();
  const res = await guarded(() => safeCall('system.nightLight.set', { enabled: on, temperature: getSetting('nightLight.temperature', 4500) }), () => {
    st.nightLight = prev;
    renderTiles();
  });
  if (isDryRun(res)) return;   // test mode: reverted by guarded()
  if (res) { setSetting('nightLight.enabled', on).catch(() => {}); return; }
  // not supported here after all → hide the tile gracefully
  if (prev.available) {
    const night = await loadNightLight();
    if (!night.available) { st.nightLight = night; renderTiles(); }
  }
}

function toggleDark(on) {
  setSetting('theme.mode', on ? 'dark' : 'light').catch((e) => flash(e?.message || t('failed')));
}

function toggleDnd(on) {
  setSetting('notifications.dnd', on).catch((e) => flash(e?.message || t('failed')));
  ui?.tileMap.dnd.set({ on });
}

function setBrightness(v, final = false) {
  markTouched(ui.brightSlider);
  st.brightness = { ...st.brightness, value: v };
  bridge.call('system.brightness.set', { value: Math.round(v) }).then((r) => {
    if (final && isDryRun(r)) restoreAfterDryRun('system.brightness.get', 'brightness', renderBrightness, ui?.brightSlider);
  }, (e) => { if (final) flash(e?.message || t('failed')); });
}

function setVolume(v, final = false) {
  markTouched(ui.volSlider);
  // like Windows: moving the slider while muted turns the sound back on
  const wasMuted = !!st.audio?.muted;
  const muted = wasMuted && v > 0 ? false : wasMuted;
  st.audio = { ...st.audio, volume: v, muted };
  ui.muteBtn.setIcon(volumeIcon(v, muted));
  ui.muteBtn.setLabel(muted ? t('unmute') : t('mute'));
  ui.volRow.classList.toggle('is-muted', muted);
  const args = { volume: Math.round(v) };
  if (wasMuted && !muted) args.muted = false;
  bridge.call('system.audio.set', args).then((r) => {
    if (final && isDryRun(r)) restoreAfterDryRun('system.audio.get', 'audio', renderVolume, ui?.volSlider);
  }, (e) => { if (final) flash(e?.message || t('failed')); });
}

function toggleMute() {
  const a = st.audio;
  if (!a) return;
  const muted = !a.muted;
  st.audio = { ...a, muted };
  renderVolume();
  guarded(() => safeCall('system.audio.set', { muted }), () => { st.audio = a; renderVolume(); });
}

/* ==========================================================================
   Sub-pages (slide in from the right, back arrow)
   ========================================================================== */

function subHeader(title, toggle) {
  return h('div', { class: 'fo-sub__header' },
    IconButton({ icon: 'back', label: t('back'), size: 32, iconSize: 16, onClick: () => closeSub(), cls: 'fo-sub__back' }),
    h('div', { class: 'fo-sub__title body-strong' }, title),
    toggle || null);
}

function subFooter(label, page) {
  return h('div', { class: 'fo-sub__footer' },
    h('button', { type: 'button', class: 'fo-sub__more', onClick: () => openSettings(page) }, label));
}

function emptyMsg(title, hint, ic) {
  return h('div', { class: 'fo-sub__empty' },
    ic ? h('span', { class: 'fo-sub__empty-icon' }, icon(ic, { size: 24 })) : null,
    h('div', { class: 'body-strong' }, title),
    hint ? h('div', { class: 'caption text-2' }, hint) : null);
}

function openSub(kind) {
  if (!ui) return;
  const page = kind === 'wifi' ? WifiPage() : kind === 'bluetooth' ? BluetoothPage() : SoundPage();
  page.kind = kind;
  const prev = ui.sub;
  ui.sub = page;
  transition(prev ? prev.el : ui.main, page.el, 1);
  setTimeout(() => page.el.querySelector('.fo-sub__back')?.focus({ preventScroll: true }), 0);
}

function closeSub({ instant = false } = {}) {
  if (!ui || !ui.sub) return;
  const sub = ui.sub;
  ui.sub = null;
  sub.destroy?.();
  transition(sub.el, ui.main, -1, instant);
  if (!instant) {
    const focusTile = { wifi: 'wifi', bluetooth: 'bluetooth' }[sub.kind];
    setTimeout(() => {
      if (focusTile) ui.tileMap[focusTile].el.querySelector('.fo-tile__more')?.focus({ preventScroll: true });
      else if (sub.kind === 'sound') ui.outBtn.focus({ preventScroll: true });
    }, 0);
  }
}

let transitionAnim = null;
function transition(from, to, dir, instant = false) {
  const pages = ui.pages;
  transitionAnim?.forEach((a) => { try { a?.cancel(); } catch { /* ignore */ } });
  transitionAnim = null;
  for (const c of [...pages.children]) if (c !== from && c !== to) c.remove();
  const h0 = pages.offsetHeight;
  if (!to.isConnected) pages.append(to);
  to.classList.remove('is-leaving');
  if (instant) { from.remove(); from.classList.remove('is-leaving'); return; }
  from.classList.add('is-leaving');
  const h1 = to.offsetHeight;
  const ease = 'cubic-bezier(0,0,0,1)';
  const p1 = play(pages, [{ height: `${h0}px` }, { height: `${h1}px` }], { duration: 250, easing: ease });
  const p2 = play(from, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: `translateX(${-dir * 40}px)` }], { duration: 120, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' });
  const p3 = play(to, [{ opacity: 0, transform: `translateX(${dir * 48}px)` }, { opacity: 1, transform: 'none' }], { duration: 250, easing: ease });
  const set = [p1.anim, p2.anim, p3.anim];
  transitionAnim = set;
  p2.then((a) => {
    from.classList.remove('is-leaving');
    if (from !== ui.main || ui.sub) from.remove();
    try { a?.cancel(); } catch { /* ignore */ }
  });
  Promise.all([p1, p3]).then(() => { if (transitionAnim === set) transitionAnim = null; });
}

/* ---------------------------------------------------------------- Wi-Fi */

function netStatus(n) {
  if (n.active) return n.secure ? t('connectedSecure') : t('connectedOpen');
  if (!n.secure) return t('open');
  return n.saved ? t('saved') : t('secure');
}

function wifiGlyph(n) {
  return h('span', { class: 'fo-net__glyph' }, icon(wifiIcon(n.strength), { size: 20 }),
    n.secure ? h('span', { class: 'fo-net__lock' }, icon('lock', { size: 10 })) : null);
}

let wifiPageRef = null;
async function refreshWifiList(rescan) {
  const page = wifiPageRef;
  if (!page) return;
  if (rescan) { st.scanning = true; page.render(); }
  try {
    const list = await bridge.call('system.network.wifiList', { rescan: !!rescan });
    st.wifiList = Array.isArray(list) ? list : [];
  } catch (e) {
    if (rescan) flash(e?.message || t('failed'));
  }
  st.scanning = false;
  if (wifiPageRef === page) page.render();
}

function WifiPage() {
  const toggle = ToggleSwitch({ checked: !!st.network?.wifiEnabled, stateText: false, ariaLabel: t('wifi'), onChange: (on) => { toggleWifi(on); page.render(); } });
  const progress = ProgressBar({ indeterminate: true, ariaLabel: t('scanning') });
  progress.classList.add('fo-sub__progress');
  const list = h('div', { class: 'fo-sub__list', role: 'list' });
  const el = h('div', { class: 'fo-qs__page fo-sub', dataset: { sub: 'wifi' } },
    subHeader(t('wifi'), toggle), progress, list, subFooter(t('moreWifi'), 'wifi'));
  const rowState = new Map(); // ssid → {expanded, stage:'idle'|'password'|'connecting'|'error', error, auto}
  let expanded = null;

  function rowFor(n) {
    const rs = rowState.get(n.ssid) || { stage: 'idle', auto: true };
    rowState.set(n.ssid, rs);
    const isOpen = expanded === n.ssid;
    const head = h('button', {
      type: 'button', class: 'fo-net__head', 'aria-expanded': String(isOpen),
      onClick: () => { expanded = isOpen ? null : n.ssid; if (!isOpen) rs.stage = rs.stage === 'connecting' ? 'connecting' : 'idle'; render(); focusRow(n.ssid); },
    },
    wifiGlyph(n),
    h('span', { class: 'fo-net__text' },
      h('span', { class: 'fo-net__name ellipsis' }, n.ssid),
      h('span', { class: 'fo-net__status caption text-2' }, rs.stage === 'connecting' ? t('connecting') : netStatus(n))));
    const row = h('div', { class: ['fo-net', isOpen && 'is-expanded', n.active && 'is-active'], role: 'listitem', dataset: { ssid: n.ssid } }, head);
    if (!isOpen) return row;

    const body = h('div', { class: 'fo-net__body' });
    if (n.active) {
      body.append(h('div', { class: 'fo-net__actions' },
        h('button', { type: 'button', class: 'fo-sub__link', onClick: () => openSettings('wifi') }, t('properties')),
        h('span', { class: 'grow' }),
        Button({ label: t('disconnect'), onClick: async (ev, b) => {
          b.setDisabled(true); b.setLabel(t('disconnecting'));
          await guarded(() => safeCall('system.network.disconnect'));
          await refreshWifiList(false);
        } })));
    } else if (rs.stage === 'password') {
      const pw = TextBox({ type: 'password', placeholder: t('password'), ariaLabel: t('enterKey'), autofocus: true, onEnter: () => go() });
      if (rs.error) pw.setError(rs.error);
      const go = () => { const v = pw.getValue(); if (!v) { pw.focus(); return; } connect(n, rs, v); };
      body.append(
        h('div', { class: 'caption text-2 fo-net__prompt' }, t('enterKey')), pw,
        h('div', { class: 'fo-net__actions fo-net__actions--split' },
          Button({ label: t('next'), variant: 'accent', onClick: go }),
          Button({ label: t('cancel'), onClick: () => { rs.stage = 'idle'; rs.error = null; render(); focusRow(n.ssid); } })));
    } else if (rs.stage === 'connecting') {
      const pb = ProgressBar({ indeterminate: true, ariaLabel: t('connecting') });
      body.append(h('div', { class: 'caption text-2' }, t('connecting')), pb);
    } else {
      if (!n.secure) body.append(h('div', { class: 'fo-net__warn caption' }, icon('info', { size: 12 }), h('span', null, t('openWarning'))));
      if (rs.error) body.append(h('div', { class: 'fo-net__error caption' }, rs.error));
      const auto = Checkbox({ label: t('autoConnect'), checked: rs.auto !== false, onChange: (v) => { rs.auto = v; } });
      body.append(h('div', { class: 'fo-net__actions' }, auto, h('span', { class: 'grow' }),
        Button({ label: t('connect'), variant: 'accent', onClick: () => {
          if (n.secure && !n.saved) { rs.stage = 'password'; rs.error = null; render(); } else connect(n, rs, null);
        } })));
    }
    row.append(body);
    return row;
  }

  async function connect(n, rs, password) {
    rs.stage = 'connecting'; rs.error = null;
    render();
    let res = null;
    try {
      res = await bridge.call('system.network.connect', password != null ? { ssid: n.ssid, password } : { ssid: n.ssid });
    } catch (e) {
      res = { ok: false, error: e?.message, code: e?.code };
    }
    if (isDryRun(res)) {   // test mode: nothing was connected
      rs.stage = 'idle';
      flash(t('dryRun'), 'info');
      render();
      return;
    }
    if (res && res.ok !== false) {
      rs.stage = 'idle';
      expanded = null;
      await refreshWifiList(false);
      return;
    }
    const msg = res?.error || t('connectFailed');
    // the host's codes say it (in every language): bad_args to a call with a password is the password itself
    // (longer than 64 characters, or a NUL in it); the text test is only for NetworkManager's own (English) error
    const needsKey = n.secure && (res?.code === 'needs_password' || res?.code === 'wrong_password'
      || (password != null && res?.code === 'bad_args') || /password|key|secret/i.test(msg));
    rs.stage = needsKey ? 'password' : 'idle';
    rs.error = msg;
    render();
  }

  function focusRow(ssid) {
    setTimeout(() => {
      const r = [...list.children].find((x) => x.dataset.ssid === ssid);
      const target = r?.querySelector('.fo-net__body input') || r?.querySelector('.fo-net__head');
      if (target && document.activeElement !== target) target.focus({ preventScroll: true });
      r?.scrollIntoView({ block: 'nearest' });
    }, 0);
  }

  function render() {
    const n = st.network || {};
    toggle.setChecked(!!n.wifiEnabled && !n.airplane);
    toggle.setDisabled(!!n.airplane);
    progress.hidden = !st.scanning;
    const keepFocus = list.contains(document.activeElement) && document.activeElement.matches('.fo-net__head')
      ? document.activeElement.closest('.fo-net')?.dataset.ssid : null;
    clear(list);
    if (n.airplane) { list.append(emptyMsg(t('airplaneOn'), t('airplaneHint'), 'airplane')); return; }
    if (!n.wifiEnabled) { list.append(emptyMsg(t('wifiOff'), t('wifiOffHint'), 'wifi-off')); return; }
    const nets = st.wifiList || [];
    if (!nets.length) {
      list.append(st.scanning || st.wifiList === null ? h('div', { class: 'fo-sub__scanning caption text-2' }, t('scanning')) : emptyMsg(t('noNetworks'), null, 'wifi'));
      return;
    }
    for (const net of nets) list.append(rowFor(net));
    if (keepFocus) focusRow(keepFocus);
  }

  const page = { el, render, destroy() { if (wifiPageRef === page) wifiPageRef = null; } };
  wifiPageRef = page;
  render();
  refreshWifiList(true);
  return page;
}

/* ------------------------------------------------------------ Bluetooth */

// host bt_kind() → glyph; older hosts only give BlueZ's icon name
const BT_KIND_ICON = {
  headphones: 'headphones', headset: 'headphones', speaker: 'sound', phone: 'phone', computer: 'pc', keyboard: 'keyboard',
  mouse: 'mouse', gamepad: 'gamepad', pen: 'edit', display: 'display', camera: 'camera', printer: 'print', watch: 'clock',
  network: 'network', media: 'sound',
};
function btIcon(d = {}) {
  if (BT_KIND_ICON[d.kind]) return BT_KIND_ICON[d.kind];
  const n = String(d.icon || '');
  if (/headset|headphone|audio-head/.test(n)) return 'headphones';
  if (/mouse/.test(n)) return 'mouse';
  if (/keyboard/.test(n)) return 'keyboard';
  if (/gaming|joystick|gamepad/.test(n)) return 'gamepad';
  if (/phone/.test(n)) return 'phone';
  if (/audio|speaker|card/.test(n)) return 'sound';
  if (/computer/.test(n)) return 'pc';
  return 'bluetooth';
}
// the host's NotReady text: "Bluetooth reageert niet. Kies “Bluetooth opnieuw starten”."
// "Bluetooth reageert niet": the host's code (not_ready; every BT answer has one) or its catalog key (E2), never the
// text, which is in the user's language (§2.10)
const isBtNotReady = (e) => !!e && typeof e === 'object' && (e.code === 'not_ready' || e.key === 'system.bt_not_responding');

/** Win11 "Apparaat toevoegen": Instellingen › Bluetooth en apparaten with "Een apparaat toevoegen" open. */
function addBtDevice() {
  const params = { page: 'bluetooth', add: 1 };
  if (typeof ctx?.openApp === 'function') Promise.resolve(ctx.openApp('settings', params)).catch((e) => console.warn('[quick] openApp failed', e));
  else bridge.call('shell.openApp', { app: 'settings', params }).catch((e) => console.warn('[quick] openApp failed', e));
  ctx?.closeFlyout('quick');
}

function BluetoothPage() {
  const toggle = ToggleSwitch({
    checked: !!st.bluetooth?.powered, stateText: false, ariaLabel: t('bluetooth'),
    onChange: (on) => { toggleBluetooth(on); render(); },
  });
  const list = h('div', { class: 'fo-sub__list', role: 'list' });
  const el = h('div', { class: 'fo-qs__page fo-sub', dataset: { sub: 'bluetooth' } },
    subHeader(t('bluetooth'), toggle), list, subFooter(t('moreBt'), 'bluetooth'));
  let expanded = null;
  const busy = new Map(); // address → 'connect'|'disconnect'
  const errors = new Map();
  let notReady = false;   // a call answered "Bluetooth reageert niet" (until the host's next state)
  let resetting = false;

  async function act(d, what) {
    busy.set(d.address, what);
    errors.delete(d.address);
    render();
    try {
      // the host answers {ok, error?, code?} (its text in our language) instead of throwing for device problems
      const r = await bridge.call(what === 'connect' ? 'system.bluetooth.connect' : 'system.bluetooth.disconnect', { address: d.address });
      if (r && r.ok === false) {
        if (isBtNotReady(r)) notReady = true;
        errors.set(d.address, r.error || t('failed'));
      } else if (isDryRun(r)) flash(t('dryRun'), 'info');   // test mode: nothing changed
      else expanded = null;
    } catch (e) {
      if (isBtNotReady(e)) notReady = true;
      errors.set(d.address, e?.message || t('failed'));
    }
    busy.delete(d.address);
    const fresh = await load('system.bluetooth.get');
    if (fresh) st.bluetooth = fresh;
    renderTiles();
    const hadFocus = el.contains(document.activeElement) || document.activeElement === document.body;
    render();
    if (hadFocus) list.querySelector(`[data-address="${CSS.escape(d.address)}"] .fo-net__head`)?.focus({ preventScroll: true });
  }

  // "Bluetooth opnieuw starten" right here (rfkill cycle on the host when the adapter is stuck)
  async function reset() {
    if (resetting) return;
    resetting = true;
    render();
    try {
      const r = await bridge.call('system.bluetooth.reset');
      if (r && r.available != null) st.bluetooth = r;
      notReady = false;
      errors.clear();
      resetting = false;
      render();
      flash(t('btResetOk'), 'info');
    } catch (e) {
      resetting = false;
      render();
      flash(e?.message || t('failed'));
    }
    renderTiles();
  }

  function stuckNotice() {
    return h('div', { class: ['fo-qs__notice', 'fo-bt-stuck', resetting && 'is-info'], role: 'status' },
      icon(resetting ? 'info' : 'warning', { size: 16 }),
      h('div', { class: 'fo-bt-stuck__body' },
        h('span', null, resetting ? t('btResetting') : t('btStuck')),
        resetting ? ProgressBar({ indeterminate: true, ariaLabel: t('btResetting') })
          : h('button', { type: 'button', class: 'fo-sub__link fo-bt-stuck__reset', onClick: reset }, t('btReset'))));
  }

  function addRow() {
    return h('div', { class: 'fo-net fo-bt-add', role: 'listitem' },
      h('button', { type: 'button', class: 'fo-net__head', onClick: addBtDevice },
        h('span', { class: 'fo-net__glyph fo-bt-add__glyph' }, icon('plus', { size: 16 })),
        h('span', { class: 'fo-net__text' }, h('span', { class: 'fo-net__name ellipsis' }, t('pairNew')))));
  }

  function render() {
    const b = st.bluetooth || {};
    const stuck = !!(b.stuck || notReady);
    toggle.setChecked(!!b.powered);
    toggle.setDisabled(resetting);
    const keep = list.contains(document.activeElement) && document.activeElement.matches('.fo-net__head')
      ? document.activeElement.closest('.fo-net')?.dataset.address || (document.activeElement.closest('.fo-bt-add') ? '+' : null) : null;
    clear(list);
    if (stuck || resetting) list.append(stuckNotice());
    if (!b.powered) {
      if (!stuck && !resetting) list.append(emptyMsg(t('btOff'), t('btOffHint'), 'bluetooth'));
      return;
    }
    const devs = (b.devices || []).filter((d) => d.paired !== false);
    if (!devs.length) list.append(emptyMsg(t('btNone'), t('btNoneHint'), 'bluetooth'));
    const coll = collator();
    devs.sort((a, c) => (c.connected - a.connected) || coll.compare(String(a.name), String(c.name)));
    for (const d of devs) {
      const isOpen = expanded === d.address;
      const state = busy.get(d.address);
      const connected = d.connected && !stuck;
      const status = state === 'connect' ? t('connecting') : state === 'disconnect' ? t('disconnecting')
        : connected ? [t('connected'), d.battery != null ? t('btBattery', { p: pct(d.battery) }) : null].filter(Boolean).join(' · ') : t('notConnected');
      const row = h('div', { class: ['fo-net', isOpen && 'is-expanded', connected && 'is-active'], role: 'listitem', dataset: { address: d.address } },
        h('button', {
          type: 'button', class: 'fo-net__head', 'aria-expanded': String(isOpen),
          onClick: () => { expanded = isOpen ? null : d.address; render(); setTimeout(() => list.querySelector(`[data-address="${CSS.escape(d.address)}"] .fo-net__head`)?.focus(), 0); },
        },
        h('span', { class: 'fo-net__glyph' }, icon(btIcon(d), { size: 20 })),
        h('span', { class: 'fo-net__text' },
          h('span', { class: 'fo-net__name ellipsis' }, d.name || d.address),
          h('span', { class: 'fo-net__status caption text-2' }, status))));
      if (isOpen) {
        const body = h('div', { class: 'fo-net__body' });
        if (errors.get(d.address)) body.append(h('div', { class: 'fo-net__error caption' }, errors.get(d.address)));
        if (state) body.append(ProgressBar({ indeterminate: true, ariaLabel: status }));
        else {
          body.append(h('div', { class: 'fo-net__actions' }, h('span', { class: 'grow' }),
            Button({ label: connected ? t('disconnect') : t('connect'), variant: connected ? 'standard' : 'accent', disabled: resetting || stuck, onClick: () => act(d, connected ? 'disconnect' : 'connect') })));
        }
        row.append(body);
      }
      list.append(row);
    }
    list.append(addRow());
    if (keep === '+') list.querySelector('.fo-bt-add .fo-net__head')?.focus({ preventScroll: true });
    else if (keep) list.querySelector(`[data-address="${CSS.escape(keep)}"] .fo-net__head`)?.focus({ preventScroll: true });
  }
  // the host's own verdict (it counts NotReady answers in `stuck` too) replaces ours
  const page = { el, render() { if (st.bluetooth?.stuck != null && !busy.size) notReady = false; render(); } };
  render();
  load('system.bluetooth.get').then((b) => { if (b) { st.bluetooth = b; page.render(); renderTiles(); } });
  return page;
}

/* ---------------------------------------------------------------- Sound */

function SoundPage() {
  const list = h('div', { class: 'fo-sub__list', role: 'listbox', 'aria-label': t('output') });
  const el = h('div', { class: 'fo-qs__page fo-sub', dataset: { sub: 'sound' } },
    subHeader(t('output')), list, subFooter(t('moreSound'), 'sound'));
  function render() {
    const sinks = st.audio?.sinks || [];
    const keep = list.contains(document.activeElement) ? document.activeElement.dataset.id : null;
    clear(list);
    if (!sinks.length) { list.append(emptyMsg(t('noSinks'), null, 'sound')); return; }
    for (const s of sinks) {
      const isSel = !!s.default;
      const item = h('button', {
        type: 'button', class: ['fo-sink', isSel && 'is-selected'], role: 'option', 'aria-selected': String(isSel), dataset: { id: s.id },
        onClick: async () => {
          if (isSel) return;
          const prev = st.audio;
          st.audio = { ...prev, sinks: sinks.map((x) => ({ ...x, default: x.id === s.id })) };
          render();
          await guarded(() => safeCall('system.audio.setSink', { id: s.id }), () => { st.audio = prev; render(); });
        },
      },
      h('span', { class: 'fo-sink__glyph' }, icon(/headset|headphone|hoofdtelefoon/i.test(s.name) ? 'headphones' : /hdmi|display|monitor|lg|dell|samsung/i.test(s.name + s.id) ? 'display' : 'volume-2', { size: 16 })),
      h('span', { class: 'fo-sink__name ellipsis' }, s.name || s.id));
      list.append(item);
    }
    if (keep) list.querySelector(`[data-id="${CSS.escape(keep)}"]`)?.focus();
  }
  list.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const items = [...list.querySelectorAll('.fo-sink')];
    const i = items.indexOf(document.activeElement);
    if (i < 0) return;
    e.preventDefault();
    items[Math.max(0, Math.min(items.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))].focus();
  });
  render();
  return { el, render };
}

/* ==========================================================================
   Flyout lifecycle
   ========================================================================== */

function onOpen() {
  settleHost(host);
  if (ui?.sub) closeSub({ instant: true });
  ui.notice.hidden = true;
  renderAll();
  // refresh in the background (events keep it live while open)
  refreshAll();
}

function onClose() {
  wifiPageRef = null;
  const sub = ui?.sub;
  if (sub) setTimeout(() => { if (ui.sub === sub && !ctx.isOpen('quick')) closeSub({ instant: true }); }, 200);
}

export default { setup };
