// Win11 Shell — mock tray.* (notification area / system tray). SPEC §6.15.
//
// Pretends to be host/services/tray.py in role 'watcher' with five background apps, in registration
// order: blueman (Bluetooth, Passive: the shell does not show it), Nextcloud, Steam, Discord and Signal. Items carry the full
// TrayItem shape; menus are dbusmenu-like MenuItem trees (submenu, checkmark, radio, separator,
// disabled, shortcut, icon-name) with the SPEC result shapes.
// * tray.activate on Steam answers fallback:'menu' (an app without Activate → the UI opens the menu);
//   Discord/Signal bring their (mock) window to the front.
// * tray.menuEvent 'clicked' flips checkmarks / radios (+ tray.menuChanged {id}); a "quit" entry
//   makes the app leave the tray (tray.changed).
// * Every ~30 s Signal or Discord toggles NeedsAttention (tray.changed) — not with ?mockStatic=1.
// * Like the host, it records every key in settings taskbar.tray.known ({name, icon, seen}).
// Icon values are what the host sends (themed names, or PNG paths under <cache>/tray/ for pixmaps);
// their artwork is provided through `icons` below (mock/index.js serves it from bridge.iconUrl).
// Original drawings on a 16 px grid, so they stay crisp at the tray's 16 px.
// Test switches: ?mockTray=0 (no items) · ?mockTray=unsupported (supported:false, role 'none') ·
//   ?mockTrayShown=<key>|<key> (puts those keys on the taskbar via settings.set — mock only) ·
//   ?mockTrayAttention=<key> (that item starts in NeedsAttention).

const clone = (v) => JSON.parse(JSON.stringify(v));
const err = (code, message) => Object.assign(new Error(message), { code });
const svg = (inner) => 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">${inner}</svg>`);

const CACHE = '/home/robin/.cache/win11shell/tray';
const ICON = {
  discord: `${CACHE}/3f9d2c41b7e05a6c8d1e4f72a9b0c3d5e6f71829.png`,
  signal: `${CACHE}/a1c47e09d2b3f5684c7e9a0b1d2f3e4c5a6b7d80.png`,
  signalUnread: `${CACHE}/b72e1d4c09a8f3b6e5d4c3b2a1f0e9d8c7b6a594.png`,
  nextcloud: `${CACHE}/0c4f8e2d6a1b9c3e7f5d2a8b4c6e0f1a3b5d7c92.png`,
};

/* ---- artwork (original, 16 px grid) ---- */
const BT = 'M5.2 5.6 10.4 10.3 8 12.6V3.4l2.4 2.3L5.2 10.4';
const ART = {
  // a friendly chat face on blurple
  [ICON.discord]: svg('<rect x="1" y="2" width="14" height="12" rx="4" fill="#5865f2"/>'
    + '<circle cx="5.7" cy="7.6" r="1.25" fill="#fff"/><circle cx="10.3" cy="7.6" r="1.25" fill="#fff"/>'
    + '<path d="M5.2 10.3c1.6.9 4 .9 5.6 0" fill="none" stroke="#fff" stroke-width="1.1" stroke-linecap="round"/>'),
  // game pad on deep blue (matches the mock's Steam app tile)
  steam_tray_mono: svg('<rect x="1" y="1" width="14" height="14" rx="3.5" fill="#1b2a52"/>'
    + '<path d="M4.7 5.4h6.6a2.3 2.3 0 0 1 2.3 2.5l-.2 1.9a1.3 1.3 0 0 1-2.3.7l-.9-1.1H5.8l-.9 1.1a1.3 1.3 0 0 1-2.3-.7l-.2-1.9a2.3 2.3 0 0 1 2.3-2.5Z" fill="#fff"/>'
    + '<path d="M5.4 6.6v1.8M4.5 7.5h1.8" stroke="#1b2a52" stroke-width=".9" stroke-linecap="round"/>'
    + '<circle cx="10.5" cy="6.9" r=".6" fill="#1b2a52"/><circle cx="11.6" cy="8" r=".6" fill="#1b2a52"/>'),
  // speech bubble
  [ICON.signal]: svg('<path d="M8 1.5a6.5 6.5 0 0 0-5.5 9.95L1.7 14.3l3-.75A6.5 6.5 0 1 0 8 1.5Z" fill="#3a76f0"/>'
    + '<path d="M5 6.6h6M5 9.2h3.8" stroke="#fff" stroke-width="1.2" stroke-linecap="round"/>'),
  [ICON.signalUnread]: svg('<path d="M8 1.5a6.5 6.5 0 0 0-5.5 9.95L1.7 14.3l3-.75A6.5 6.5 0 1 0 8 1.5Z" fill="#3a76f0"/>'
    + '<path d="M5 6.6h6M5 9.2h3.8" stroke="#fff" stroke-width="1.2" stroke-linecap="round"/>'
    + '<circle cx="12.6" cy="3.4" r="3" fill="#e5484d" stroke="#fff" stroke-width="1"/>'),
  // white cloud on blue
  [ICON.nextcloud]: svg('<rect x="1" y="1" width="14" height="14" rx="3.5" fill="#0082c9"/>'
    + '<path d="M5 11.4a2.5 2.5 0 0 1-.25-5 3.5 3.5 0 0 1 6.65-.55 2.8 2.8 0 0 1 .2 5.55Z" fill="#fff"/>'),
  // small green "up to date" badge (OverlayIconName)
  'emblem-default': svg('<circle cx="8" cy="8" r="7" fill="#13a10e" stroke="#fff" stroke-width="1.2"/>'
    + '<path d="M4.7 8.2 7 10.4l4.4-4.6" fill="none" stroke="#fff" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"/>'),
  // Bluetooth rune on blue
  'blueman-tray': svg('<rect x="1" y="1" width="14" height="14" rx="3.5" fill="#1f6fd1"/>'
    + `<path d="${BT}" fill="none" stroke="#fff" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round"/>`),
  // menu icons (icon-name entries of blueman's menu)
  'bluetooth-disabled': svg(`<path d="${BT}" fill="none" stroke="#2b7de9" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/>`
    + '<path d="M3 3l10 10" stroke="#c42b1c" stroke-width="1.4" stroke-linecap="round"/>'),
  'blueman-device': svg('<rect x="4.5" y="1.5" width="7" height="13" rx="1.8" fill="none" stroke="#6b7785" stroke-width="1.2"/>'
    + '<path d="M7 12.4h2" stroke="#6b7785" stroke-width="1.2" stroke-linecap="round"/>'),
  'application-exit': svg('<path d="M8 2v5.5" stroke="#c42b1c" stroke-width="1.5" stroke-linecap="round"/>'
    + '<path d="M5 4a5 5 0 1 0 6 0" fill="none" stroke="#c42b1c" stroke-width="1.5" stroke-linecap="round"/>'),
};

/* ---- menus (labels as the apps show them in a Dutch session) ---- */
let mid = 0;
const it = (label, extra = {}) => ({ id: ++mid, type: 'item', label, enabled: true, icon: null, toggle: null,
  checked: null, shortcut: null, children: [], ...extra });
const sep = () => ({ id: ++mid, type: 'separator', label: '', enabled: true, icon: null, toggle: null,
  checked: null, shortcut: null, children: [] });
const check = (label, on = false, extra = {}) => it(label, { toggle: 'checkmark', checked: on, ...extra });
const radio = (label, on = false) => it(label, { toggle: 'radio', checked: on });
const quit = (label, extra = {}) => it(label, { effect: 'quit', ...extra });
const open = (label) => it(label, { effect: 'activate' });

function initialItems() {
  mid = 0;
  const blueman = {
    id: ':1.52/org/blueman/sni', key: 'blueman-manager.desktop', sniId: 'blueman', title: 'blueman',
    appName: 'Bluetooth-beheer', appIcon: 'blueman', desktopId: 'blueman-manager.desktop', pid: 1984,
    category: 'Hardware', status: 'passive', icon: 'blueman-tray', attentionIcon: null, overlayIcon: null,
    tooltip: { title: 'Bluetooth', text: 'Bluetooth staat aan\nGeen apparaten verbonden' }, itemIsMenu: false, hasMenu: true,
  };
  blueman.menu = [
    it('Bluetooth uitschakelen', { icon: 'bluetooth-disabled' }),
    it('Recente verbindingen', { children: [it('Geen recente verbindingen', { enabled: false })] }),
    sep(),
    it('Apparaten…', { icon: 'blueman-device' }),
    it('Adapters…', { icon: 'preferences-system' }),
    sep(),
    quit('Afsluiten', { icon: 'application-exit' }),
  ];
  mid = 0;
  const nextcloud = {
    id: ':1.64/StatusNotifierItem', key: 'com.nextcloud.desktopclient.nextcloud.desktop', sniId: 'Nextcloud',
    title: 'Nextcloud', appName: 'Nextcloud', appIcon: 'Nextcloud', desktopId: 'com.nextcloud.desktopclient.nextcloud.desktop',
    pid: 2841, category: 'ApplicationStatus', status: 'active', icon: ICON.nextcloud, attentionIcon: null,
    overlayIcon: 'emblem-default', tooltip: { title: 'Nextcloud', text: 'Alle bestanden zijn bijgewerkt' },
    itemIsMenu: false, hasMenu: true,
  };
  nextcloud.menu = [
    it('Alle bestanden zijn bijgewerkt', { enabled: false }),
    sep(),
    open('Nextcloud openen'),
    check('Synchronisatie pauzeren'),
    it('Instellingen…'),
    it('Help'),
    sep(),
    quit('Nextcloud afsluiten'),
  ];
  mid = 0;
  const steam = {
    id: ':1.71/org/ayatana/NotificationItem/steam', key: 'steam.desktop', sniId: 'steam', title: 'Steam',
    appName: 'Steam', appIcon: 'steam', desktopId: 'steam.desktop', pid: 3127, category: 'ApplicationStatus',
    status: 'active', icon: 'steam_tray_mono', attentionIcon: null, overlayIcon: null,
    tooltip: { title: 'Steam', text: '' }, itemIsMenu: false, hasMenu: true, noActivate: true,
  };
  steam.menu = [
    it('Winkel'), it('Bibliotheek'), it('Community'), it('Vrienden'),
    it('Recent gespeeld', { children: [it('Stardew Valley'), it('Portal 2'), it('Factorio')] }),
    it('Instellingen'),
    sep(),
    it('Big Picture'),
    quit('Steam afsluiten'),
  ];
  mid = 0;
  const discord = {
    id: ':1.88/StatusNotifierItem', key: 'discord_discord.desktop', sniId: 'chrome_status_icon_1', title: 'Discord',
    appName: 'Discord', appIcon: 'discord', desktopId: 'discord_discord.desktop', pid: 3388,
    category: 'ApplicationStatus', status: 'active', icon: ICON.discord, attentionIcon: null, overlayIcon: null,
    tooltip: { title: 'Discord', text: '' }, itemIsMenu: false, hasMenu: true, window: 'discord',
  };
  discord.menu = [
    open('Discord openen'),
    sep(),
    check('Microfoon dempen', false, { shortcut: 'Ctrl+Shift+M' }),
    check('Geluid uitschakelen', false, { shortcut: 'Ctrl+Shift+D' }),
    sep(),
    it('Controleren op updates…'),
    quit('Discord afsluiten'),
  ];
  mid = 0;
  const signal = {
    id: ':1.93/StatusNotifierItem', key: 'signal-desktop_signal-desktop.desktop', sniId: 'chrome_status_icon_1',
    title: 'Signal', appName: 'Signal', appIcon: 'signal-desktop', desktopId: 'signal-desktop_signal-desktop.desktop',
    pid: 3512, category: 'Communications', status: 'active', icon: ICON.signal, attentionIcon: ICON.signalUnread,
    overlayIcon: null, tooltip: { title: 'Signal', text: '' }, itemIsMenu: false, hasMenu: true, window: 'signal',
  };
  signal.menu = [
    open('Signal weergeven'),
    it('Meldingen', { children: [radio('Naam en inhoud', true), radio('Alleen naam'), radio('Geen naam of inhoud')] }),
    sep(),
    quit('Afsluiten'),
  ];
  return [blueman, nextcloud, steam, discord, signal];
}

/* ---- state ---- */
const INTERNAL = ['menu', 'noActivate', 'window', 'revision'];
function S(m) {
  if (!m.state.tray) {
    const mode = m.params.get('mockTray');
    const items = mode === '0' || mode === 'unsupported' ? [] : initialItems();
    const att = m.params.get('mockTrayAttention');
    for (const x of items) {
      x.revision = 1;
      if (att && (x.key === att || x.key.startsWith(att))) x.status = 'attention';
    }
    m.state.tray = { supported: mode !== 'unsupported', items };
  }
  return m.state.tray;
}
function pub(x) {
  const o = {};
  for (const [k, v] of Object.entries(x)) if (!INTERNAL.includes(k)) o[k] = clone(v);
  return o;
}
function find(m, id) {
  const x = S(m).items.find((i) => i.id === id);
  if (!x) throw err('not_found', 'Dit pictogram bestaat niet meer.');
  return x;
}
function pubMenu(list) {
  return list.map(({ effect, ...rest }) => ({
    ...rest,
    submenu: rest.type !== 'separator' && !!(rest.submenu || (rest.children || []).length),
    children: pubMenu(rest.children || []),
  }));
}
function findNode(list, id, parent = null) {
  for (const n of list) {
    if (n.id === id) return { node: n, siblings: list, parent };
    const r = findNode(n.children || [], id, n);
    if (r) return r;
  }
  return null;
}
let changeTimer = 0;
function changed(m) {
  // coalesced like the host (≥ 100 ms)
  clearTimeout(changeTimer);
  changeTimer = setTimeout(() => m.emit('tray.changed', { items: S(m).items.map(pub) }), 100);
}

/** Discord / Signal: clicking the tray icon brings the app's window up (mock windows). */
async function showWindow(m, x) {
  if (!x.window) return;
  try {
    const r = await m.call('windows.list', {});
    const w = (r?.windows || []).find((win) => String(win.appId).toLowerCase().includes(x.window));
    if (w) await m.call('windows.focus', { id: w.id });
    else m.state.mockWindows?.spawn({ id: x.desktopId, icon: x.appIcon, name: x.appName });
  } catch (e) { console.info('[mock] tray: no window for', x.title, e?.message); }
}

function remember(m) {
  // the host keeps settings.taskbar.tray.known up to date (name, icon, seen; refreshed once a day)
  const s = S(m);
  const known = clone(m.settings()?.taskbar?.tray?.known || {});
  const now = Date.now();
  let dirty = false;
  for (const x of s.items) {
    const k = known[x.key];
    if (!k || k.name !== x.appName || k.icon !== x.appIcon || now - (Number(k.seen) || 0) > 86400e3) {
      known[x.key] = { name: x.appName, icon: x.appIcon, seen: now };
      dirty = true;
    }
  }
  if (dirty) m.call('settings.set', { path: 'taskbar.tray.known', value: known }).catch(() => {});
}

export default {
  icons: ART,

  methods: {
    'tray.list': async (a, m) => {
      const s = S(m);
      return { supported: s.supported, role: s.supported ? 'watcher' : 'none', items: s.items.map(pub) };
    },
    'tray.activate': async ({ id } = {}, m) => {
      const x = find(m, id);
      // like the host: ItemIsMenu items get their menu without Activate; Steam has no Activate at all
      if ((x.itemIsMenu && x.hasMenu) || x.noActivate) return { ok: true, fallback: 'menu' };
      console.info('[mock] tray.activate', x.title);
      showWindow(m, x);
      return { ok: true, fallback: null };
    },
    'tray.secondaryActivate': async ({ id } = {}, m) => {
      console.info('[mock] tray.secondaryActivate', find(m, id).title);
      return { ok: true };
    },
    'tray.scroll': async ({ id, delta, orientation = 'vertical' } = {}, m) => {
      const x = find(m, id);
      if (!['vertical', 'horizontal'].includes(orientation)) throw err('bad_args', 'Ongeldige scrollrichting.');
      if (!Number.isFinite(Number(delta ?? 0))) throw err('bad_args', 'Ongeldige scrollwaarde.');
      console.info('[mock] tray.scroll', x.title, delta, orientation);
      return { ok: true };
    },
    'tray.contextMenu': async ({ id } = {}, m) => {
      console.info('[mock] tray.contextMenu (the app draws its own menu)', find(m, id).title);
      return { ok: true };
    },
    'tray.menu': async ({ id } = {}, m) => { // {refresh:true} (GetLayout only on the host) reads the same here
      const x = find(m, id);
      if (!x.hasMenu) throw err('no_menu', 'Deze app heeft geen menu.');
      return { revision: x.revision, items: pubMenu(x.menu) };
    },
    'tray.menuEvent': async ({ id, itemId, event = 'clicked' } = {}, m) => {
      const x = find(m, id);
      if (!['clicked', 'hovered', 'opened', 'closed'].includes(event)) throw err('bad_args', 'Onbekende menu-actie.');
      if (!Number.isInteger(Number(itemId)) || Number(itemId) < 0) throw err('bad_args', 'Ongeldig menu-item.');
      if (event !== 'clicked') return { ok: true };
      const hit = findNode(x.menu, Number(itemId));
      if (!hit || hit.node.type === 'separator' || !hit.node.enabled) return { ok: true };
      const n = hit.node;
      console.info('[mock] tray menu clicked', x.title, '›', n.label);
      if (n.toggle === 'checkmark') n.checked = !n.checked;
      else if (n.toggle === 'radio') for (const sib of hit.siblings) if (sib.toggle === 'radio') sib.checked = sib === n;
      if (n.toggle) {
        x.revision++;
        setTimeout(() => m.emit('tray.menuChanged', { id: x.id }), 30);
      }
      if (n.effect === 'activate') showWindow(m, x);
      if (n.effect === 'quit') {
        // the app quits: its bus name loses its owner a moment later
        setTimeout(() => {
          const s = S(m);
          s.items = s.items.filter((i) => i !== x);
          changed(m);
        }, 300);
      }
      return { ok: true };
    },
  },

  start(m) {
    const s = S(m);
    if (!s.supported) { m.emit('tray.status', { supported: false, role: 'none' }); return; }
    remember(m);
    const shown = m.params.get('mockTrayShown');
    if (shown) {
      const keys = shown.split('|').map((k) => k.trim()).filter(Boolean);
      m.call('settings.set', { path: 'taskbar.tray.shown', value: keys }).catch(() => {});
    }
    if (m.params.get('mockStatic') === '1') return;
    // now and then an app asks for attention (unread message), and stops again
    let n = 0;
    setInterval(() => {
      const pool = S(m).items.filter((x) => x.key.startsWith('signal') || x.key.startsWith('discord'));
      const x = pool[n++ % Math.max(1, pool.length)];
      if (!x || document.hidden) return;
      const on = x.status !== 'attention';
      x.status = on ? 'attention' : 'active';
      if (x.key.startsWith('signal')) x.tooltip = { title: 'Signal', text: on ? '1 ongelezen bericht' : '' };
      changed(m);
    }, 30000);
  },
};
