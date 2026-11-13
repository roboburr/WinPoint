// Instellingen › Persoonlijke instellingen › Taakbalk: items (search, task view, widgets),
// the notification area ("Andere systeemvakpictogrammen": which tray icons sit on the taskbar
// and which stay behind the ^ overflow, SPEC §6.15) and behaviour (alignment, auto-hide,
// seconds) with a live mini taskbar preview.
// Deep link: {page: 'taskbar', section: 'tray'} opens the tray expander and scrolls to it.

import { h, clear } from '../../../core/dom.js';
import { makeT, collator } from '../../../core/i18n.js';
import { bridge } from '../../../core/bridge.js';
import { getSetting, getSettings, setSetting, resolveWallpaper, themeMode } from '../../../core/theme.js';
import { time, date } from '../../../core/format.js';
import { icon, startLogo } from '../../../core/icons.js';
import { SettingsCard, SettingsGroup, SettingsExpander, ToggleSwitch, AppIcon } from '../../../core/components.js';
import { get, settingDropdown, toggleCard, showError, accentFor, notice } from '../common.js';

const t = makeT({
  nl: {
    items: 'Taakbalkitems',
    itemsDesc: 'Pictogrammen op de taakbalk weergeven of verbergen',
    search: 'Zoeken', searchHidden: 'Verbergen', searchIcon: 'Alleen pictogram zoeken', searchIconLabel: 'Pictogram en label zoeken', searchBox: 'Zoekvak',
    taskView: 'Taakweergave', widgets: 'Widgets',
    tray: 'Andere systeemvakpictogrammen',
    trayDesc: 'Extra systeemvakpictogrammen weergeven of verbergen',
    hiddenMenu: 'Menu met verborgen pictogrammen',
    hiddenMenuDesc: 'Pictogrammen die niet op de taakbalk staan, vind je achter de pijl ^',
    allShown: 'Het menu met verborgen pictogrammen staat uit, dus alle actieve pictogrammen staan op de taakbalk.',
    passiveDesc: 'Niet zichtbaar zolang de app niet actief is',
    trayNone: 'Er zijn nog geen apps met een pictogram in het systeemvak. Apps zoals Discord, Steam of Nextcloud verschijnen hier zodra ze er een tonen.',
    trayNa: 'Het systeemvak is in deze sessie niet beschikbaar. Je ziet hier de apps die eerder een pictogram hadden, maar je kunt ze nu niet wijzigen.',
    trayNaEmpty: 'Het systeemvak is in deze sessie niet beschikbaar.',
    behavior: 'Gedrag van de taakbalk',
    behaviorDesc: 'Uitlijning, automatisch verbergen, klok',
    alignment: 'Uitlijning van de taakbalk', left: 'Links', center: 'Gecentreerd',
    autoHide: 'De taakbalk automatisch verbergen',
    autoHideDesc: 'De taakbalk verschijnt als je de muis naar de onderkant van het scherm beweegt',
    seconds: 'Seconden weergeven in de klok van het systeemvak',
    secondsDesc: 'Gebruikt iets meer stroom',
    preview: 'Voorbeeld',
    searchPh: 'Zoeken',
  },
  en: {
    items: 'Taskbar items',
    itemsDesc: 'Show or hide icons on the taskbar',
    search: 'Search', searchHidden: 'Hide', searchIcon: 'Search icon only', searchIconLabel: 'Search icon and label', searchBox: 'Search box',
    taskView: 'Task view', widgets: 'Widgets',
    tray: 'Other system tray icons',
    trayDesc: 'Show or hide additional system tray icons',
    hiddenMenu: 'Hidden icon menu',
    hiddenMenuDesc: 'Icons that aren’t on the taskbar are kept behind the ^ arrow',
    allShown: 'The hidden icon menu is off, so all active icons are on the taskbar.',
    passiveDesc: 'Not shown while the app is inactive',
    trayNone: 'No apps have a system tray icon yet. Apps such as Discord, Steam or Nextcloud appear here once they show one.',
    trayNa: 'The system tray isn’t available in this session. You can see the apps that had an icon before, but you can’t change them right now.',
    trayNaEmpty: 'The system tray isn’t available in this session.',
    behavior: 'Taskbar behaviours',
    behaviorDesc: 'Alignment, auto-hide, clock',
    alignment: 'Taskbar alignment', left: 'Left', center: 'Centre',
    autoHide: 'Automatically hide the taskbar',
    autoHideDesc: 'The taskbar appears when you move the mouse to the bottom of the screen',
    seconds: 'Show seconds in system tray clock',
    secondsDesc: 'Uses a little more power',
    preview: 'Preview',
    searchPh: 'Search',
  },
});

/** taskbar.search values (shared with the taskbar): 'hidden' | 'icon' | 'iconLabel' | 'box' */
export const SEARCH_MODES = ['hidden', 'icon', 'iconLabel', 'box'];

/* ------------------------------------------------------------ system tray */

const COLL_OPTS = { sensitivity: 'base', numeric: true };
/** params.section values that open "Andere systeemvakpictogrammen" */
const TRAY_SECTIONS = new Set(['tray', 'systray', 'notificationarea']);
const list = (v) => (Array.isArray(v) ? v : []);
const hiddenMenuOn = () => getSetting('taskbar.tray.hiddenMenu', true) !== false;
// ?section= from the window URL counts once (main.js passes the first page its params; this is the fallback)
let urlSectionUsed = false;

function wantsTraySection(ctx) {
  let s = ctx.params?.section;
  if (!s && !urlSectionUsed && ['taskbar', 'taskbar-settings'].includes(String(bridge.params.get('page') || '').toLowerCase())) {
    s = bridge.params.get('section');
  }
  urlSectionUsed = true;
  return TRAY_SECTIONS.has(String(s || '').toLowerCase());
}

/** tray.list → {supported, role, items}; a missing method or a failure counts as "not available". */
async function loadTray() {
  const v = await get('tray.list', {}, null);
  const ok = !!v && typeof v === 'object';
  return { supported: ok && v.supported !== false, role: (ok && v.role) || 'none', items: ok ? list(v.items) : [] };
}

/** Rows: live items ∪ taskbar.tray.known (apps not running now), one per key, sorted by name. */
function trayApps(items) {
  const byKey = new Map();
  const label = (key) => String(key).replace(/^(sni|exe):/, '');
  for (const it of items) {
    const key = it?.key || it?.id;
    if (!key || byKey.has(key)) continue;
    byKey.set(key, { key, name: it.appName || it.title || label(key), icon: it.appIcon || it.icon || null });
  }
  const known = getSetting('taskbar.tray.known', {});
  for (const [key, k] of Object.entries(known && typeof known === 'object' ? known : {})) {
    if (!byKey.has(key)) byKey.set(key, { key, name: k?.name || label(key), icon: k?.icon || null });
  }
  const coll = collator(COLL_OPTS);
  return [...byKey.values()].sort((a, b) => coll.compare(a.name, b.name) || coll.compare(a.key, b.key));
}

/** What the mini taskbar shows of the notification area: the ^ when icons are hidden, then ≤ 3 icons in the user's order. */
function trayPreview(items) {
  // passive items (the app asks not to be shown) are nowhere, like on the taskbar itself
  const live = items.filter((it) => it && (it.key || it.id) && it.status !== 'passive');
  const menu = hiddenMenuOn();
  const shown = list(getSetting('taskbar.tray.shown', []));
  const order = list(getSetting('taskbar.tray.order', []));
  const rank = (it) => { const i = order.indexOf(it.key); return i < 0 ? order.length : i; };
  const onBar = live.filter((it) => !menu || shown.includes(it.key));
  onBar.sort((a, b) => rank(a) - rank(b)); // stable: registration order for keys without a place
  return {
    chevron: menu && onBar.length < live.length,
    icons: onBar.slice(0, 3).map((it) => (it.status === 'attention' && it.attentionIcon) || it.icon || it.appIcon || null),
  };
}

/** Keep `nodes` as the children of `parent`, moving as little as possible (focused toggles keep focus). */
function syncChildren(parent, nodes) {
  nodes.forEach((n, i) => { if (parent.children[i] !== n) parent.insertBefore(n, parent.children[i] || null); });
  while (parent.children.length > nodes.length) parent.lastElementChild.remove();
}

/**
 * "Andere systeemvakpictogrammen" (Win11 24H2): the "Menu met verborgen pictogrammen" toggle, then one
 * row per app (On = on the taskbar, Off = in the ^ overflow). tray = {supported, items} (kept live by the page).
 */
function TraySection(ctx, tray, { expanded }) {
  const hiddenSw = ToggleSwitch({
    checked: hiddenMenuOn(), ariaLabel: t('hiddenMenu'),
    onChange: async (on) => {
      try { await setSetting('taskbar.tray.hiddenMenu', on); } catch (e) { hiddenSw.setChecked(hiddenMenuOn()); showError(e); }
    },
  });
  const hiddenCard = SettingsCard({ title: t('hiddenMenu'), description: t('hiddenMenuDesc'), control: hiddenSw });
  const naBar = notice({ message: t('trayNa') });
  const naNote = h('div', { class: 'st-expander-note' }, naBar);
  const allNote = h('div', { class: 'st-sub-note' }, t('allShown'));
  const emptyNote = h('div', { class: 'st-sub-note' }, t('trayNone'));
  const exp = SettingsExpander({ icon: 'apps', title: t('tray'), description: t('trayDesc'), expanded, children: [hiddenCard] });
  exp.style.scrollMarginTop = '12px';

  async function setShown(key, on) {
    const shown = list(getSetting('taskbar.tray.shown', []));
    const next = on ? (shown.includes(key) ? shown : [...shown, key]) : shown.filter((k) => k !== key);
    try {
      await setSetting('taskbar.tray.shown', next);
      const order = list(getSetting('taskbar.tray.order', []));
      if (on && !order.includes(key)) {
        // keys missing from `order` sort after it on the taskbar (in registration order): write those out
        // first, or the switched-on icon would jump to the front instead of landing at the end
        const rest = list(tray.items).map((x) => x?.key || x?.id).filter((k) => typeof k === 'string' && k !== key && !order.includes(k));
        await setSetting('taskbar.tray.order', [...new Set([...order, ...rest]), key]);
      }
    } catch (e) {
      showError(e);
      paint();
    }
  }

  const rows = new Map(); // key → {card, sw, name, icon}
  function rowFor(app) {
    let r = rows.get(app.key);
    if (!r) {
      const sw = ToggleSwitch({ ariaLabel: app.name, onChange: (on) => setShown(app.key, on) });
      const card = SettingsCard({ icon: AppIcon({ name: app.icon, size: 20 }), title: app.name, control: sw });
      card.querySelector('.fl-sc__icon')?.style.setProperty('margin', '0 12px 0 0');
      card.dataset.trayKey = app.key;
      r = { card, sw, name: app.name, icon: app.icon };
      rows.set(app.key, r);
      return r;
    }
    if (r.name !== app.name) {
      r.card.setTitle(app.name);
      r.sw.querySelector('[role="switch"]')?.setAttribute('aria-label', app.name);
      r.name = app.name;
    }
    if (r.icon !== app.icon) {
      const slot = r.card.querySelector('.fl-sc__icon');
      if (slot) clear(slot).append(AppIcon({ name: app.icon, size: 20 }));
      r.icon = app.icon;
    }
    return r;
  }

  function paint() {
    const menu = hiddenMenuOn();
    const readOnly = !tray.supported;
    const shown = list(getSetting('taskbar.tray.shown', []));
    hiddenSw.setChecked(menu);
    hiddenSw.setDisabled(readOnly);
    const apps = trayApps(tray.items);
    // running apps whose every icon is Passive are not shown anywhere until they turn active (the
    // switch keeps the choice for then)
    const live = list(tray.items).filter((x) => x && (x.key || x.id));
    const passive = new Set(live.map((x) => x.key || x.id)
      .filter((k) => live.every((x) => (x.key || x.id) !== k || x.status === 'passive')));
    const nodes = [];
    if (readOnly) nodes.push(naNote);
    nodes.push(hiddenCard);
    if (!menu && apps.length) nodes.push(allNote);
    for (const app of apps) {
      const r = rowFor(app);
      const isPassive = passive.has(app.key);
      // hidden icon menu off: every active icon is on the taskbar, the per-app choice does not apply
      r.sw.setChecked(menu ? shown.includes(app.key) : !isPassive);
      r.card.setDescription(isPassive ? t('passiveDesc') : '');
      r.sw.setDisabled(readOnly || !menu);
      r.card.setDisabled(!menu);
      nodes.push(r.card);
    }
    naBar.setMessage(t(apps.length ? 'trayNa' : 'trayNaEmpty'));
    if (!apps.length && !readOnly) nodes.push(emptyNote);
    const keys = new Set(apps.map((a) => a.key));
    for (const k of [...rows.keys()]) if (!keys.has(k)) rows.delete(k);
    syncChildren(exp.content, nodes);
  }

  paint();
  ctx.onSettings((s, path) => { if (!path || String(path).startsWith('taskbar')) paint(); });
  exp.paint = paint;
  return exp;
}

/** Scroll a freshly rendered element into view once main.js has put the page in the document.
 *  (Timers, not requestAnimationFrame: rAF does not run while the window is hidden or not painting.) */
function reveal(ctx, el) {
  let tries = 0;
  const step = () => {
    if (!ctx.alive()) return;
    if (!el.isConnected) { if (++tries < 120) setTimeout(step, 16); return; }
    el.scrollIntoView({ block: 'start' });
  };
  setTimeout(step, 0);
}

/* -------------------------------------------------------- taskbar preview */

function TaskbarPreview(ctx, trayInfo) {
  const bar = h('div', { class: 'st-tbprev__bar' });
  const el = h('div', { class: 'st-tbprev', 'aria-hidden': 'true' }, bar);
  // tray extras (^ and app icons) sit in grid spans so fit() can hide them, whatever they hold
  const slot = (child) => h('span', { style: { display: 'grid' } }, child);
  const trayImg = (name) => {
    if (!name) return slot(icon('app', { size: 14 }));
    const img = h('img', { src: bridge.iconUrl(name, 32), alt: '', width: 16, height: 16, draggable: false });
    img.addEventListener('error', () => img.replaceWith(icon('app', { size: 14 })), { once: true });
    return slot(img);
  };
  let extras = [];
  // The mini bar is narrow (the search box takes most of it): show only the tray extras that fit without
  // wrapping the clock, dropping the last icon first and the ^ last.
  const fit = () => {
    if (!el.isConnected || !extras.length) return;
    const wraps = () => [clockT, clockD].some((n) => n?.isConnected && n.offsetHeight > 20);
    for (const n of extras) n.style.display = 'grid';
    for (let i = extras.length - 1; i >= 0 && wraps(); i--) extras[i].style.display = 'none';
  };
  const paint = () => {
    const s = getSettings();
    const tb = s.taskbar || {};
    const acc = accentFor(s.theme?.accent || '#0078d4');
    el.style.setProperty('--tb-accent', acc.fill);
    const wp = resolveWallpaper(s);
    el.style.backgroundColor = s.wallpaper?.color || '#0b2a4a';
    el.style.backgroundImage = wp ? `url("${wp}")` : 'none';
    el.classList.toggle('is-left', tb.alignment === 'left');
    el.classList.toggle('is-autohide', !!tb.autoHide);
    // "Accentkleur weergeven op Start en taakbalk" (theme.accentOnTaskbar) only applies in dark mode, like Windows
    el.style.setProperty('--tb-accent-raw', s.theme?.accent || '#0078d4');
    el.classList.toggle('accent-taskbar', !!s.theme?.accentOnTaskbar && themeMode() === 'dark');
    clear(bar);
    const center = h('div', { class: 'st-tbprev__center' });
    center.append(h('span', { class: 'st-tbprev__btn is-start' }, startLogo(18)));
    if (tb.search === 'box') center.append(h('span', { class: 'st-tbprev__search' }, icon('search', { size: 14 }), h('span', null, t('searchPh'))));
    else if (tb.search === 'iconLabel') center.append(h('span', { class: 'st-tbprev__search is-pill' }, icon('search', { size: 14 }), h('span', null, t('searchPh'))));
    else if (tb.search === 'icon') center.append(h('span', { class: 'st-tbprev__btn' }, icon('search', { size: 18 })));
    if (tb.taskView) center.append(h('span', { class: 'st-tbprev__btn' }, icon('taskview', { size: 18 })));
    for (const n of ['system-file-manager', 'firefox', 'preferences-system']) {
      const img = h('img', { src: bridge.iconUrl(n, 48), alt: '', width: 22, height: 22, draggable: false });
      img.addEventListener('error', () => img.replaceWith(icon('app', { size: 18 })), { once: true });
      center.append(h('span', { class: ['st-tbprev__btn', 'is-app', n === 'preferences-system' && 'is-active'] }, img));
    }
    const left = h('div', { class: 'st-tbprev__left' }, tb.widgets ? h('span', { class: 'st-tbprev__widget' }, icon('widgets', { size: 16 }), h('span', null, '14°')) : null);
    clockT = h('span');
    clockD = h('span');
    // notification area like Win11: ^ · app icons · quick settings (wifi, volume) · clock
    const ti = trayInfo?.() || { chevron: false, icons: [] };
    extras = [ti.chevron ? slot(icon('chevron-up', { size: 12 })) : null, ...ti.icons.map(trayImg)].filter(Boolean);
    const tray = h('div', { class: 'st-tbprev__tray' }, ...extras,
      icon('wifi-3', { size: 14 }), icon('volume', { size: 14 }),
      h('span', { class: 'st-tbprev__clock' }, clockT, s.time?.showDate === false ? null : clockD));
    bar.append(left, center, tray);
    tick();
    fit();
  };
  let clockT = null, clockD = null;
  const tick = () => {
    const s = getSettings();
    const now = Date.now();
    if (clockT) clockT.textContent = time(now, { seconds: !!s.taskbar?.showSeconds, h24: s.time?.h24 !== false });
    if (clockD) clockD.textContent = date(now, 'short');
  };
  paint();
  ctx.onSettings(paint);
  ctx.interval(tick, 1000);
  // first layout, window resizes and late web fonts change what fits
  if (typeof ResizeObserver === 'function') {
    const ro = new ResizeObserver(() => fit());
    ro.observe(el);
    ctx.onCleanup(() => ro.disconnect());
  }
  document.fonts?.ready.then(() => { if (ctx.alive()) fit(); });
  el.paint = paint;
  return el;
}

export const pages = {
  async taskbar(ctx) {
    const openTray = wantsTraySection(ctx);
    const tray = await loadTray();

    const searchCard = SettingsCard({
      icon: 'search', title: t('search'),
      control: settingDropdown(ctx, 'taskbar.search', [
        { value: 'hidden', label: t('searchHidden') }, { value: 'icon', label: t('searchIcon') },
        { value: 'iconLabel', label: t('searchIconLabel') }, { value: 'box', label: t('searchBox') },
      ], { width: 240, fallback: 'box' }),
    });
    const items = SettingsExpander({
      icon: 'taskbar', title: t('items'), description: t('itemsDesc'), expanded: true,
      children: [
        searchCard,
        toggleCard(ctx, { icon: 'taskview', title: t('taskView'), path: 'taskbar.taskView' }),
        toggleCard(ctx, { icon: 'widgets', title: t('widgets'), path: 'taskbar.widgets' }),
      ],
    });

    const traySection = TraySection(ctx, tray, { expanded: openTray });

    const secondsOn = () => !!getSetting('taskbar.showSeconds', false);
    const secondsSw = ToggleSwitch({
      checked: secondsOn(), ariaLabel: t('seconds'),
      onChange: async (on) => {
        try { await bridge.call('settings.patch', { patch: { taskbar: { showSeconds: on }, time: { showSeconds: on } } }); }
        catch (e) { secondsSw.setChecked(!on); showError(e); }
      },
    });
    ctx.onSettings(() => secondsSw.setChecked(secondsOn()));

    const behavior = SettingsExpander({
      icon: 'settings', title: t('behavior'), description: t('behaviorDesc'), expanded: true,
      children: [
        SettingsCard({ title: t('alignment'), control: settingDropdown(ctx, 'taskbar.alignment', [{ value: 'left', label: t('left') }, { value: 'center', label: t('center') }], { width: 180, fallback: 'center' }) }),
        toggleCard(ctx, { title: t('autoHide'), description: t('autoHideDesc'), path: 'taskbar.autoHide' }),
        SettingsCard({ title: t('seconds'), description: t('secondsDesc'), control: secondsSw }),
      ],
    });

    const preview = TaskbarPreview(ctx, () => trayPreview(tray.supported ? tray.items : []));

    // live notification area: icons come and go, the host may switch role (watcher ↔ host ↔ none)
    const repaint = () => { traySection.paint(); preview.paint(); };
    ctx.on('tray.changed', (d) => { tray.items = list(d?.items); repaint(); });
    ctx.on('tray.status', async (d) => {
      const was = tray.supported;
      tray.supported = d?.supported !== false;
      tray.role = d?.role || tray.role;
      if (tray.supported && !was) Object.assign(tray, await loadTray());
      if (ctx.alive()) repaint();
    });

    if (openTray) reveal(ctx, traySection);

    return [
      h('div', { class: 'st-preview-top' }, preview),
      SettingsGroup({ children: [items, traySection, behavior] }),
    ];
  },
};
