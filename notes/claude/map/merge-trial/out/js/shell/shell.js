// Win11 Shell — shell orchestrator (owned by the shell agent). SPEC §11.
//
//   export async function mountShell(root, {preview})
//
// Builds the taskbar and the `ctx` every shell module receives in `setup(ctx)`:
//   ctx = { bridge, root, registerFlyout, openFlyout, closeFlyout, toggleFlyout, isOpen,
//           onCommand(command, handler) → off, runCommand(command, args) → handled:boolean,
//           taskbarAnchor('start'|'search'|'quick'|'clock'|'center'|'taskview'|'tray') → DOMRect, preview }
// plus (additive, documented in notes/shell.md):
//   ctx.tray     notification area (tray.js): ready, items(), openMenu(key|id|index), openOverflow()
//   ctx.apps     shared app catalogue: ready, list(), get(id), resolve(desktopId, appId), launch(id, {uris}),
//                onChange(cb), onLaunch(cb), recentLaunches(n), topApps(n), recentlyInstalled(days)
//   ctx.windows  merged window list (host windows.list + preview WM): list(), onChange(cb), forApp(key),
//                focus(w, {fromRect}?), minimize(w), close(w), toggleMaximize(w), showDesktop(), supported()
//                (fromRect: the Task View / Alt+Tab card a minimized host window grows out of, SPEC §6.4.1),
//                animates() (the host says minimize / restore hints will animate: windows.anim, setIconRects),
//                noteAnim(bool) (the taskbar passes on setIconRects' answer)
//   ctx.pins     { taskbar: {list, has, add, remove, toggle, set}, start: {list, has, add, remove, toggle, moveToFront, move, set} }
//   ctx.openApp(app, params) · ctx.openSettings(pageIds) · ctx.openPath(path) · ctx.places() · ctx.settingsPages()
//   ctx.appIcon(iconName, size, {request}) → <span><img></span> with a fallback glyph
//   ctx.toastError(title, error) · ctx.taskbar (taskbar handle) · ctx.user() → {name, realName, avatar}
//
// Commands arrive as `shell.command {command, args}` events (win11shell-ctl / Hyprland binds) and from
// the UI itself. `?open=start|search|quick|notifications|calendar|taskview|winx|clipboard|power|tray|
// tray-menu|contextmenu-taskbar|allapps` runs the matching command after setup (debug / screenshots;
// `&q=` for search, `&item=<key|index>` for tray-menu).

import { bridge } from '../core/bridge.js';
import { h, loadCss, debounce } from '../core/dom.js';
import {
  registerFlyout, openFlyout, closeFlyout, toggleFlyout, isOpen, getFlyout, onFlyoutChange,
} from '../core/flyouts.js';
import { getSetting, setSetting, onSettings, getShellInfo, themeMode } from '../core/theme.js';
import { Dialog, Toast } from '../core/components.js';
import { icon } from '../core/icons.js';
import { makeT } from '../core/i18n.js';
import { refreshRegions } from '../core/regions.js';
import { createTaskbar } from './taskbar.js';

const t = makeT({
  nl: {
    launchFailed: '{name} kon niet worden gestart',
    saveFailed: 'Deze wijziging kon niet worden opgeslagen',
    openFailed: 'Dit kon niet worden geopend',
    powerTitle_shutdown: 'Computer afsluiten?', powerTitle_reboot: 'Computer opnieuw opstarten?',
    powerTitle_suspend: 'Slaapstand inschakelen?', powerTitle_logout: 'Afmelden?', powerTitle_lock: 'Vergrendelen?',
    powerMsg: 'Sla eerst je werk op. Apps die nog open zijn, worden gesloten.',
    powerMsgSuspend: 'Je computer gaat in slaapstand. Je werk blijft bewaard.',
    powerMsgLock: 'Je scherm wordt vergrendeld. Je hebt je wachtwoord nodig om verder te gaan.',
    power_shutdown: 'Afsluiten', power_reboot: 'Opnieuw opstarten', power_suspend: 'Slaapstand', power_logout: 'Afmelden', power_lock: 'Vergrendelen',
    powerFailed: 'Dat lukte niet',
  },
  en: {
    launchFailed: '{name} could not be started',
    saveFailed: 'This change could not be saved',
    openFailed: 'This could not be opened',
    powerTitle_shutdown: 'Shut down the computer?', powerTitle_reboot: 'Restart the computer?',
    powerTitle_suspend: 'Go to sleep?', powerTitle_logout: 'Sign out?', powerTitle_lock: 'Lock?',
    powerMsg: 'Save your work first. Apps that are still open will be closed.',
    powerMsgSuspend: 'Your computer will go to sleep. Your work is kept.',
    powerMsgLock: 'Your screen will be locked. You need your password to continue.',
    power_shutdown: 'Shut down', power_reboot: 'Restart', power_suspend: 'Sleep', power_logout: 'Sign out', power_lock: 'Lock',
    powerFailed: 'That did not work',
  },
});

const OWN_MODULES = ['startmenu', 'search', 'winx', 'tray'];
const FLYOUT_MODULES = ['quicksettings', 'notifcenter', 'toasts', 'osd', 'taskview', 'clipboard', 'power'];
const OWN_APP_RE = /^win11shell-(settings|taskmgr|explorer|alles|disks)(?:\.desktop)?$/;
const DAY = 86400e3;

/* ==========================================================================
   Small storage helpers (per-viewer conveniences only)
   ========================================================================== */
function readLS(key, fallback) {
  try { const v = JSON.parse(localStorage.getItem(key) || 'null'); return v ?? fallback; } catch { return fallback; }
}
function writeLS(key, value) {
  try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* ignore */ }
}

/* ==========================================================================
   App catalogue
   ========================================================================== */
function createAppStore(ctx) {
  let list = [];
  let byId = new Map();
  let alias = new Map();
  let installedAt = {};
  let browserId = null;   // apps.defaults().browser.id (the user's default web browser)
  const subs = new Set();
  const launchSubs = new Set();
  let launches = readLS('win11shell.shell.launches', []);
  let counts = readLS('win11shell.shell.launchCounts', {});
  if (!Array.isArray(launches)) launches = [];
  if (!counts || typeof counts !== 'object') counts = {};
  let resolveReady;
  const ready = new Promise((r) => { resolveReady = r; });

  const norm = (s) => String(s || '').toLowerCase().replace(/\.desktop$/, '');

  function index() {
    byId = new Map(list.map((a) => [a.id, a]));
    alias = new Map();
    const put = (k, a) => { if (k && !alias.has(k)) alias.set(k, a); };
    for (const a of list) put(norm(a.id), a);
    for (const a of list) {
      const n = norm(a.id);
      const snap = /^([^_]+)_(.+)$/.exec(n);
      if (snap) { put(snap[1], a); put(snap[2], a); }
      put(n.split('.').pop(), a);
      put(n.replace(/^(org|com|io|net|dev)\.[^.]+\./, ''), a);
      put(norm(a.icon).split('/').pop(), a);
      put(String(a.name || '').toLowerCase(), a);
    }
  }

  /**
   * "Onlangs toegevoegd": the first time this shell sees an app (apps present at the very first start
   * are not new). A snap/apt update that rewrites the .desktop file of an app we already knew never makes
   * it "new" again (the host's `added` is the file's mtime, so it changes with every update); `added` is
   * only used when it says an app that appears later is in fact old. The mock (screenshots) trusts its
   * curated `added` times from the start.
   */
  function trackInstalled() {
    const key = bridge.isMock ? 'win11shell.shell.knownApps.mock' : 'win11shell.shell.knownApps';
    let known = readLS(key, null);
    const first = !known || typeof known !== 'object';
    if (first) known = {};
    const now = Date.now();
    const recent = now - 14 * DAY;
    let dirty = false;
    for (const a of list) {
      if (a.id in known) continue;
      const hostT = Number(a.installedAt ?? a.added) || 0;
      known[a.id] = first ? (bridge.isMock && hostT > recent ? hostT : 0) : (hostT && hostT < recent ? hostT : now);
      dirty = true;
    }
    if (dirty) writeLS(key, known);
    installedAt = known;
  }

  async function load() {
    const defaults = bridge.call('apps.defaults').catch(() => null);
    try {
      const r = await bridge.call('apps.list');
      list = Array.isArray(r) ? r.filter((a) => a && a.id && !a.noDisplay) : [];
    } catch (e) {
      console.warn('[shell] apps.list failed', e);
    }
    const d = await defaults;
    if (d) browserId = d.browser?.id || null;
    index();
    trackInstalled();
    resolveReady(list);
    for (const cb of [...subs]) { try { cb(list); } catch (e) { console.error(e); } }
  }
  load();
  bridge.on('apps.changed', debounce(load, 400));

  function record(id) {
    launches = [{ id, t: Date.now() }, ...launches.filter((x) => x.id !== id)].slice(0, 40);
    counts[id] = (counts[id] || 0) + 1;
    writeLS('win11shell.shell.launches', launches);
    writeLS('win11shell.shell.launchCounts', counts);
  }

  const store = {
    ready,
    list: () => list,
    get: (id) => byId.get(id) || null,
    has: (id) => byId.has(id),
    /** Best app for a window: exact desktop id, then aliases (snap names, reverse-DNS tail, icon, …). */
    resolve(desktopId, appId) {
      if (desktopId && byId.has(desktopId)) return byId.get(desktopId);
      for (const k of [desktopId, appId]) {
        if (!k) continue;
        const n = norm(k);
        if (alias.has(n)) return alias.get(n);
        const tail = n.split('.').pop();
        if (alias.has(tail)) return alias.get(tail);
      }
      return null;
    },
    onChange(cb) { subs.add(cb); return () => subs.delete(cb); },
    /** The user's default web browser (installed, visible), or null. */
    defaultBrowser: () => (browserId && byId.has(browserId) ? browserId : null),
    onLaunch(cb) { launchSubs.add(cb); return () => launchSubs.delete(cb); },
    isOwn: (id) => OWN_APP_RE.test(String(id || '')),
    async launch(id, { uris } = {}) {
      const app = byId.get(id);
      record(id);
      for (const cb of [...launchSubs]) { try { cb(id); } catch (e) { console.error(e); } }
      const own = OWN_APP_RE.exec(String(id || ''));
      try {
        if (own) await bridge.call('shell.openApp', { app: own[1], params: {} });
        else await bridge.call('apps.launch', { id, uris });
        return true;
      } catch (e) {
        ctx.toastError(t('launchFailed', { name: app?.name || id }), e);
        return false;
      }
    },
    recentLaunches(n = 8) { return launches.filter((x) => byId.has(x.id)).slice(0, n).map((x) => ({ app: byId.get(x.id), time: x.t })); },
    /** Most launched apps, topped up with pinned ones. */
    topApps(n = 6) {
      const ids = Object.entries(counts).sort((a, b) => b[1] - a[1]).map(([id]) => id).filter((id) => byId.has(id));
      for (const id of [...ctx.pins.start.list(), ...ctx.pins.taskbar.list()]) if (!ids.includes(id) && byId.has(id)) ids.push(id);
      return ids.slice(0, n).map((id) => byId.get(id));
    },
    recentlyInstalled(days = 14) {
      const since = Date.now() - days * DAY;
      return list.filter((a) => (installedAt[a.id] || 0) > since)
        .sort((a, b) => installedAt[b.id] - installedAt[a.id])
        .map((a) => ({ app: a, time: installedAt[a.id] }));
    },
  };
  return store;
}

/* ==========================================================================
   Windows: host (windows.list, Hyprland) + preview WM (our own apps in-page)
   ========================================================================== */
function createWindowStore() {
  let hostWins = [];
  let hostSupported = false;
  let prevWins = [];
  let prevFocused = null;
  let focusSrc = 'host';
  let merged = [];
  let previewHidden = null;
  let animOn = false;        // host: the win11-csd plugin will animate minimize / restore (SPEC §6.4.1)
  let animEvent = false;     // a windows.anim event arrived: newer than any setIconRects answer still under way
  const subs = new Set();

  function recompute() {
    const hostFocus = focusSrc === 'host';
    merged = [
      ...hostWins.map((w) => ({ ...w, source: 'host', key: `h:${w.id}`, focused: hostFocus && !!w.focused && !w.minimized })),
      ...prevWins.map((w) => ({ ...w, source: 'preview', key: `p:${w.id}`, focused: !hostFocus && !!w.focused && !w.minimized })),
    ];
    for (const cb of [...subs]) { try { cb(merged); } catch (e) { console.error(e); } }
  }
  const recomputeSoon = debounce(recompute, 16);

  function setPreview(list) {
    prevWins = Array.isArray(list) ? list : [];
    const f = prevWins.find((w) => w.focused && !w.minimized);
    if (f && f.id !== prevFocused) focusSrc = 'preview';
    if (!f && focusSrc === 'preview') focusSrc = 'host';
    prevFocused = f ? f.id : null;
    recomputeSoon();
  }

  bridge.call('windows.list').then((r) => {
    hostSupported = !!r?.supported;
    hostWins = Array.isArray(r?.windows) ? r.windows : [];
    recomputeSoon();
  }).catch(() => { /* unsupported / no backend */ });
  bridge.on('windows.changed', (d) => {
    hostWins = Array.isArray(d?.windows) ? d.windows : [];
    hostSupported = true;
    recomputeSoon();
  });
  bridge.on('windows.focused', (d) => {
    if (d?.id != null) focusSrc = 'host';
    hostWins = hostWins.map((w) => ({ ...w, focused: d?.id != null && w.id === d.id }));
    recomputeSoon();
  });
  bridge.on('preview.windows.changed', (d) => setPreview(d?.windows));
  bridge.on('windows.anim', (d) => { animOn = d?.on === true; animEvent = true; });
  const pollWM = () => { if (window.__previewWM?.list) setPreview(window.__previewWM.list()); };
  pollWM();
  setTimeout(pollWM, 600);

  const wm = () => window.__previewWM || null;
  const swallow = (p) => Promise.resolve(p).catch((e) => console.warn('[shell] window action failed', e));

  return {
    list: () => merged,
    supported: () => hostSupported || prevWins.length > 0 || !!wm(),
    animates: () => animOn,
    noteAnim(on) { if (typeof on === 'boolean' && !animEvent) animOn = on; },
    onChange(cb) { subs.add(cb); return () => subs.delete(cb); },
    focus(w, opts) {
      if (!w) return;
      if (w.source === 'preview') { focusSrc = 'preview'; wm()?.focus(w.id); return; }
      focusSrc = 'host';
      hostWins = hostWins.map((x) => ({ ...x, focused: x.id === w.id, minimized: x.id === w.id ? false : x.minimized }));
      recompute();
      previewHidden = null;
      const fromRect = opts?.fromRect;
      swallow(bridge.call('windows.focus', { id: w.id, ...(fromRect ? { fromRect } : {}) }));
    },
    minimize(w) {
      if (!w) return;
      if (w.source === 'preview') { wm()?.minimize(w.id); return; }
      hostWins = hostWins.map((x) => (x.id === w.id ? { ...x, minimized: true, focused: false } : x));
      recompute();
      swallow(bridge.call('windows.minimize', { id: w.id }));
    },
    close(w) {
      if (!w) return;
      if (w.source === 'preview') { wm()?.close(w.id); return; }
      swallow(bridge.call('windows.close', { id: w.id }));
    },
    toggleMaximize(w) {
      if (!w) return;
      if (w.source === 'preview') { wm()?.toggleMaximize(w.id); return; }
      swallow(bridge.call('windows.toggleMaximize', { id: w.id }));
    },
    /** Win+D / the taskbar sliver: toggles both the host and the in-page preview windows. */
    showDesktop() {
      swallow(bridge.call('windows.showDesktop'));
      const m = wm();
      if (!m?.list) return;
      if (previewHidden) {
        const ids = previewHidden;
        previewHidden = null;
        for (const id of ids) { try { m.focus(id); } catch { /* window gone */ } }
        return;
      }
      const vis = m.list().filter((w) => !w.minimized).sort((a, b) => (a.focused ? 1 : 0) - (b.focused ? 1 : 0));
      if (!vis.length) return;
      previewHidden = vis.map((w) => w.id);
      for (const w of vis) m.minimize(w.id);
    },
  };
}

/* ==========================================================================
   Pins (taskbar.pinned / start.pinned in settings)
   ========================================================================== */
// Each entry: alternatives in order of preference; the first installed one is pinned.
const START_DEFAULTS = [
  [/^firefox/i, /^google-chrome/i, /^chromium/i, /^brave/i, /^microsoft-edge/i, /^org\.gnome\.epiphany/i],
  [/^win11shell-explorer/], [/^win11shell-settings/], [/^win11shell-taskmgr/], [/^win11shell-alles/],
  [/ptyxis/i, /gnome-terminal/i, /org\.gnome\.console/i, /konsole/i, /terminal/i],
  [/texteditor/i, /gedit/i, /kate\b/i, /mousepad/i, /xed/i],
  [/snap-store/i, /gnome\.software/i, /discover/i, /app-?center/i],
  [/calculator/i],
  [/thunderbird/i, /evolution/i, /geary/i],
  [/libreoffice-writer/i], [/libreoffice-calc/i],
  [/loupe/i, /eog/i, /shotwell/i, /gthumb/i],
  [/showtime/i, /totem/i, /celluloid/i, /vlc/i],
  [/rhythmbox/i, /spotify/i, /lollypop/i],
  [/calendar/i], [/gimp/i], [/steam/i], [/discord/i], [/telegram/i], [/whatsapp/i],
  [/code_code|vscode|codium/i], [/clocks/i],
];

// Taskbar pins as shipped (host/services/settings.py DEFAULTS, and the SPEC's first draft). While
// the user has never changed them, the shipped browser pin shows the user's own default browser
// (Zen, Chrome, the deb Firefox, …) instead; the first change saves the list as shown.
const SHIPPED_TASKBAR = [
  ['win11shell-explorer.desktop', 'firefox_firefox.desktop', 'snap-store_snap-store.desktop', 'win11shell-settings.desktop'],
  ['org.gnome.Nautilus.desktop', 'firefox_firefox.desktop', 'win11shell-settings.desktop'],
];
const SHIPPED_BROWSER = 'firefox_firefox.desktop';
const sameList = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);

function createPins(ctx) {
  function withDefaultBrowser(ids) {
    if (!SHIPPED_TASKBAR.some((d) => sameList(d, ids))) return ids;
    const b = ctx.apps.defaultBrowser?.();
    if (!b || b === SHIPPED_BROWSER) return ids;
    return ids.includes(b) ? ids.filter((x) => x !== SHIPPED_BROWSER) : ids.map((x) => (x === SHIPPED_BROWSER ? b : x));
  }
  const save = async (path, value) => {
    try { await setSetting(path, value); } catch (e) { ctx.toastError(t('saveFailed'), e); }
  };
  let startDefaults = null;
  ctx.apps.onChange(() => { startDefaults = null; });
  function defaultStart() {
    if (startDefaults) return startDefaults;
    const apps = ctx.apps.list();
    const out = [];
    // the first slot is the browser: the user's default browser when known
    const browser = ctx.apps.defaultBrowser?.();
    for (const [i, alts] of START_DEFAULTS.entries()) {
      if (i === 0 && browser) { out.push(browser); continue; }
      for (const re of alts) {
        const a = apps.find((x) => re.test(x.id) && !out.includes(x.id));
        if (a) { out.push(a.id); break; }
      }
      if (out.length >= 24) break;
    }
    if (apps.length) startDefaults = out;
    return out;
  }
  const taskbar = {
    list: () => { const v = getSetting('taskbar.pinned', []); return Array.isArray(v) ? withDefaultBrowser(v) : []; },
    has: (id) => taskbar.list().includes(id),
    set: (ids) => save('taskbar.pinned', [...new Set(ids)]),
    add: (id, index) => {
      const l = taskbar.list().filter((x) => x !== id);
      l.splice(index == null ? l.length : index, 0, id);
      return taskbar.set(l);
    },
    remove: (id) => taskbar.set(taskbar.list().filter((x) => x !== id)),
    toggle: (id) => (taskbar.has(id) ? taskbar.remove(id) : taskbar.add(id)),
  };
  const start = {
    list: () => {
      const v = getSetting('start.pinned', []);
      return Array.isArray(v) && v.length ? v : defaultStart();
    },
    isDefault: () => { const v = getSetting('start.pinned', []); return !Array.isArray(v) || !v.length; },
    has: (id) => start.list().includes(id),
    set: (ids) => save('start.pinned', [...new Set(ids)]),
    add: (id) => start.set([...start.list().filter((x) => x !== id), id]),
    remove: (id) => start.set(start.list().filter((x) => x !== id)),
    toggle: (id) => (start.has(id) ? start.remove(id) : start.add(id)),
    moveToFront: (id) => start.set([id, ...start.list().filter((x) => x !== id)]),
    move: (id, index) => {
      const l = start.list().filter((x) => x !== id);
      l.splice(Math.max(0, Math.min(l.length, index)), 0, id);
      return start.set(l);
    },
  };
  return { taskbar, start };
}

/* ==========================================================================
   mountShell
   ========================================================================== */
export async function mountShell(root, { preview = false } = {}) {
  await loadCss('shell');
  await applyMockSettings();
  const handlers = new Map();
  const standaloneStart = window.__win11shellHost?.standaloneStart === true;

  const ctx = {
    bridge,
    root,
    registerFlyout, openFlyout, closeFlyout, toggleFlyout, isOpen,
    preview: !!preview,
    onCommand(command, handler) {
      if (!handlers.has(command)) handlers.set(command, new Set());
      handlers.get(command).add(handler);
      return () => handlers.get(command)?.delete(handler);
    },
    runCommand(command, args = {}) {
      const set = handlers.get(command);
      if (set && set.size) {
        for (const fn of [...set]) {
          try { const r = fn(args || {}, command); if (r && r.catch) r.catch((e) => console.error(`[shell] ${command} failed`, e)); }
          catch (e) { console.error(`[shell] command ${command} failed`, e); }
        }
        return true;
      }
      return fallbackCommand(command, args || {});
    },
    taskbarAnchor(name) {
      return ctx.taskbar?.anchor(name) || new DOMRect(innerWidth / 2, innerHeight - 48, 0, 48);
    },
  };

  /* ---- shared services on ctx (additive) ---- */
  ctx.toastError = (title, error) => {
    console.warn('[shell]', title, error);
    try { Toast.show({ severity: 'error', title, message: error?.message || String(error || ''), timeout: 6000 }); } catch { /* ignore */ }
  };
  ctx.openApp = (app, params = {}) => bridge.call('shell.openApp', { app, params: params || {} })
    .catch((e) => ctx.toastError(t('openFailed'), e));
  let registry = null;
  ctx.settingsPages = () => (registry ||= import('../apps/settings/registry.js')
    .then((m) => (Array.isArray(m.SETTINGS_PAGES) ? m.SETTINGS_PAGES : []))
    .catch(() => []));
  ctx.openSettings = async (pages = []) => {
    const ids = [].concat(pages).filter(Boolean);
    const list = await ctx.settingsPages();
    const page = (list.length && ids.find((id) => list.some((p) => p.id === id))) || ids[0];
    return ctx.openApp('settings', page ? { page } : {});
  };
  ctx.openPath = (path) => ctx.openApp('explorer', path ? { path } : {});
  let placesP = null;
  ctx.places = () => (placesP ||= bridge.call('files.places').then((r) => (Array.isArray(r) ? r : [])).catch(() => { placesP = null; return []; }));
  let userInfo = null;
  ctx.user = () => {
    const info = getShellInfo();
    const base = { name: info?.user?.name || '', realName: info?.user?.realName || info?.user?.name || '', avatar: null };
    return userInfo ? { ...base, ...userInfo } : base;
  };
  bridge.call('system.info').then((r) => {
    if (!r?.user) return;
    userInfo = { avatar: r.user.avatar || null };
    if (r.user.realName || r.user.name) userInfo.realName = r.user.realName || r.user.name;
  }).catch(() => { /* optional */ });
  // App icons whose Icon= is missing from the theme fall back to the theme's generic program icon
  // on the host (no 404 per app), like Instellingen and Verkenner; the glyph is the last resort.
  ctx.appIcon = (name, size = 24, { request = 32, glyph = 'app', themeFallback = glyph === 'app' ? 'application-x-executable' : '' } = {}) => {
    const px = Math.max(request, Math.round(size * Math.min(3, devicePixelRatio || 1)));
    const wrap = h('span', { class: 'sh-appicon', style: { width: `${size}px`, height: `${size}px` }, 'aria-hidden': 'true' });
    const fallback = () => {
      wrap.replaceChildren(icon(glyph, { size: Math.round(size * 0.8) }));
      wrap.classList.add('is-fallback');
    };
    // (our own apps keep the bridge's themed fallback: preferences-system, system-file-manager, …)
    const fb = themeFallback && !/^win11shell-/i.test(String(name || '')) ? { fallback: themeFallback } : {};
    const url = name ? bridge.iconUrl(name, px, fb) : '';
    if (!url) fallback();
    else {
      const img = h('img', { src: url, alt: '', width: size, height: size, draggable: false, decoding: 'async' });
      img.addEventListener('error', fallback, { once: true });
      wrap.append(img);
    }
    return wrap;
  };

  ctx.apps = createAppStore(ctx);
  ctx.windows = createWindowStore();
  ctx.pins = createPins(ctx);

  /* ---- taskbar ---- */
  if (!standaloneStart) {
    ctx.taskbar = createTaskbar(ctx);
    root.append(...ctx.taskbar.nodes);
  }

  /* ---- built-in commands ---- */
  ctx.onCommand('show-desktop', () => {
    for (const n of ['start', 'search', 'winx', 'quick', 'notifications', 'clipboard', 'taskview', 'alttab', 'tray']) {
      if (isOpen(n)) closeFlyout(n);
    }
    ctx.windows.showDesktop();
  });

  /* ---- modules (own + flyouts agent), tolerant of missing files ---- */
  const modules = standaloneStart ? ['startmenu', 'search', 'power'] : [...OWN_MODULES, ...FLYOUT_MODULES];
  await Promise.allSettled(modules.map(async (name) => {
    let mod;
    try { mod = await import(`./${name}.js`); } catch (e) {
      console.warn(`[shell] module ${name}.js not available`, e?.message || e);
      return;
    }
    try { await mod.setup?.(ctx); } catch (e) { console.error(`[shell] ${name}.setup() failed`, e); }
  }));

  bridge.on('shell.command', (d) => { if (d?.command) ctx.runCommand(d.command, d.args || {}); });

  installKeyboard(ctx);
  // Settings › Kleuren › "Accentkleur weergeven op Start en taakbalk" (Win11: dark mode only).
  // shell.css / flyouts.css tint the taskbar, Start and the notification centre while this is set.
  const applyAccentSurfaces = () => {
    const on = getSetting('theme.accentOnTaskbar', false) === true && themeMode() === 'dark';
    document.documentElement.toggleAttribute('data-accent-surfaces', on);
  };
  applyAccentSurfaces();
  onSettings(() => { applyAccentSurfaces(); refreshRegions(); });
  onFlyoutChange(() => refreshRegions());

  /* ---- ?open= (debug / screenshots) ---- */
  const open = bridge.params.get('open');
  if (open) {
    await Promise.race([ctx.apps.ready, new Promise((r) => setTimeout(r, 1500))]);
    setTimeout(() => runOpenParam(ctx, open), 120);
  }
  return ctx;

  /* ---- fallbacks when the owning module is missing ---- */
  function fallbackCommand(command, args) {
    const flyout = { 'quick-settings': 'quick', notifications: 'notifications', 'task-view': 'taskview', clipboard: 'clipboard',
      'toggle-start': 'start', 'toggle-search': 'search', winx: 'winx' }[command];
    if (flyout && getFlyout(flyout)) { toggleFlyout(flyout); return true; }
    if (command === 'power' && args.action) { fallbackPower(args.action); return true; }
    console.info(`[shell] no handler for command "${command}"`);
    return false;
  }

  async function fallbackPower(action) {
    const msg = action === 'suspend' ? t('powerMsgSuspend') : action === 'lock' ? t('powerMsgLock') : t('powerMsg');
    const ok = action === 'lock' ? true : await Dialog.confirm({
      title: t(`powerTitle_${action}`), message: msg, confirmLabel: t(`power_${action}`),
      danger: action === 'shutdown' || action === 'reboot' || action === 'logout',
    });
    if (!ok) return;
    bridge.call('system.power.action', { action }).catch((e) => ctx.toastError(t('powerFailed'), e));
  }
}

/** Mock-only test helper: ?mockSet=taskbar.alignment:left,taskbar.autoHide:true (values are JSON when possible). */
async function applyMockSettings() {
  const spec = bridge.isMock ? bridge.params.get('mockSet') : null;
  if (!spec) return;
  for (const pair of spec.split(',')) {
    const i = pair.indexOf(':');
    if (i < 1) continue;
    const path = pair.slice(0, i);
    let value = pair.slice(i + 1);
    try { value = JSON.parse(value); } catch { /* plain string */ }
    try { await setSetting(path, value); } catch (e) { console.warn('[shell] mockSet failed', path, e); }
  }
}

function runOpenParam(ctx, open) {
  const p = bridge.params;
  const map = {
    start: ['toggle-start', { view: p.get('view') || undefined, query: p.get('q') ?? undefined }],
    allapps: ['toggle-start', { view: 'all' }],
    search: ['toggle-search', { query: p.get('q') ?? undefined, tab: p.get('tab') || undefined }],
    quick: ['quick-settings', {}],
    notifications: ['notifications', {}],
    calendar: ['notifications', { calendar: true }],
    taskview: ['task-view', {}],
    'alt-tab': ['alt-tab', {}],
    winx: ['winx', {}],
    clipboard: ['clipboard', {}],
    power: ['power', { action: p.get('action') || 'shutdown' }],
    run: ['run', {}],
    tray: ['tray-overflow', {}],
    'tray-menu': ['tray-menu', { item: p.get('item') || undefined }],
  };
  if (open === 'contextmenu-taskbar') { ctx.taskbar.openContextMenu(); return; }
  if (open === 'jumplist') { ctx.taskbar.openJumpList(p.get('app')); return; }
  if (open === 'preview') { ctx.taskbar.openPreview(p.get('app')); return; }
  const m = map[open];
  if (m) ctx.runCommand(m[0], m[1]);
}

/* In-page shortcuts (preview window / browser). Under Hyprland the binds call win11shell-ctl. */
function installKeyboard(ctx) {
  let metaAlone = false;
  const cmd = (c, a) => { ctx.runCommand(c, a); };
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Meta' || e.key === 'OS' || e.key === 'Super') { metaAlone = !e.repeat; return; }
    metaAlone = false;
    if (e.defaultPrevented) return;
    const k = e.key.length === 1 ? e.key.toLowerCase() : e.key;
    if (e.ctrlKey && e.shiftKey && k === 'Escape') { e.preventDefault(); ctx.openApp('taskmgr'); return; }
    if (e.ctrlKey && !e.shiftKey && !e.altKey && k === 'Escape') { e.preventDefault(); cmd('toggle-start'); return; }
    if (!e.metaKey || e.ctrlKey) return;
    const map = {
      s: ['toggle-search'], x: ['winx'], a: ['quick-settings'], n: ['notifications'], d: ['show-desktop'],
      Tab: ['task-view'], v: ['clipboard'], r: ['run'],
    };
    if (map[k]) { e.preventDefault(); cmd(...map[k]); return; }
    if (k === 'e') { e.preventDefault(); ctx.openApp('explorer'); }
    else if (k === 'i') { e.preventDefault(); ctx.openApp('settings'); }
    else if (k === 'f') { e.preventDefault(); ctx.openApp('alles'); }
  }, true);
  document.addEventListener('keyup', (e) => {
    if ((e.key === 'Meta' || e.key === 'OS' || e.key === 'Super') && metaAlone) {
      metaAlone = false;
      cmd('toggle-start');
    }
  }, true);
  addEventListener('blur', () => { metaAlone = false; });
}
