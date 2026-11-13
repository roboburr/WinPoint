// Win11 Shell — taskbar (owned by the shell agent). SPEC §7/§8/§11.
//
//   createTaskbar(ctx) → { el, nodes, anchor(name) → DOMRect, openContextMenu(), openJumpList(id),
//                          openPreview(id), bounce(id), reveal(),
//                          notifyArea, setAnchor(name, () => Element|null), hold() → release() }
//
// 48 px Mica/Acrylic bar: centred (or left) group with Start, search box/icon, Task View and the
// pinned + running apps (merged by desktop id), and on the right the notification area (app tray
// icons + the "^" overflow, filled by tray.js into `notifyArea`), the system tray (network, volume,
// battery → quick settings), the clock (→ notifications, unread badge / do-not-disturb) and the
// show-desktop sliver. Hover previews with window titles, jump lists, drag to reorder pinned apps,
// auto-hide, Win11 micro-animations, arrow-key navigation.
// `setAnchor` lets a module answer anchor(name) for its own element (tray.js: 'tray' = the chevron);
// `hold()` keeps an auto-hiding taskbar up until the returned release() is called (tray drags).
// Window animations (plugin win11-csd, SPEC §6.4.1): in layer mode the taskbar tells the host where each
// window's button settles (windows.setIconRects, debounced, de-duplicated, transform-free), so minimize
// flies into it and restore grows out of it. ?iconRects=1 turns that on outside layer mode (tests).

import { bridge } from '../core/bridge.js';
import { regionsActive, keyboardSettling, flushRegions } from '../core/regions.js';
import { h, animate, debounce, clamp } from '../core/dom.js';
import { icon, startLogo, wifiIcon, volumeIcon } from '../core/icons.js';
import { showContextMenu, isContextMenuOpen, closeContextMenu } from '../core/contextmenu.js';
import { onFlyoutChange, currentFlyout, getFlyout } from '../core/flyouts.js';
import { getSettings, getSetting, onSettings } from '../core/theme.js';
import { time as fmtTime, date as fmtDate } from '../core/format.js';
import { makeT, onLangChange } from '../core/i18n.js';
import { hideTooltip } from '../core/components.js';

const t = makeT({
  nl: {
    taskbar: 'Taakbalk', start: 'Start', search: 'Zoeken', taskView: 'Taakweergave',
    quick: 'Snelle instellingen', showDesktop: 'Bureaublad weergeven', notifications: 'Meldingen',
    pin: 'Vastmaken aan taakbalk', unpin: 'Losmaken van taakbalk', closeWindow: 'Venster sluiten',
    closeAll: 'Alle vensters sluiten', close: 'Sluiten', taskmgr: 'Taakbeheer', taskbarSettings: 'Taakbalkinstellingen',
    dateTime: 'Datum en tijd aanpassen', notifSettings: 'Meldingsinstellingen',
    netSettings: 'Netwerk- en internetinstellingen', soundSettings: 'Geluidsinstellingen',
    windows: { one: '{n} venster', other: '{n} vensters' }, running: '{name} — {count} geopend',
    internet: 'Internettoegang', noInternet: 'Geen internettoegang', notConnected: 'Niet verbonden',
    wired: 'Ethernet', airplane: 'Vliegtuigmodus staat aan', netUnknown: 'Netwerk',
    speakers: 'Luidsprekers: {v}%', muted: 'Geluid staat uit', battery: 'Batterij: {p}%',
    charging: '{p}% beschikbaar (wordt opgeladen)', onAC: '{p}% beschikbaar (aangesloten)',
    unread: { one: '1 nieuwe melding', other: '{n} nieuwe meldingen' }, dnd: 'Niet storen staat aan',
    noNotifs: 'Geen nieuwe meldingen', moreApps: 'Meer apps',
    own_settings: 'Instellingen', own_taskmgr: 'Taakbeheer', own_explorer: 'Verkenner', own_alles: 'Alles', own_disks: 'Schijven',
  },
  en: {
    taskbar: 'Taskbar', start: 'Start', search: 'Search', taskView: 'Task view',
    quick: 'Quick settings', showDesktop: 'Show desktop', notifications: 'Notifications',
    pin: 'Pin to taskbar', unpin: 'Unpin from taskbar', closeWindow: 'Close window',
    closeAll: 'Close all windows', close: 'Close', taskmgr: 'Task Manager', taskbarSettings: 'Taskbar settings',
    dateTime: 'Adjust date and time', notifSettings: 'Notification settings',
    netSettings: 'Network and Internet settings', soundSettings: 'Sound settings',
    windows: { one: '{n} window', other: '{n} windows' }, running: '{name} — {count} open',
    internet: 'Internet access', noInternet: 'No internet access', notConnected: 'Not connected',
    wired: 'Ethernet', airplane: 'Airplane mode is on', netUnknown: 'Network',
    speakers: 'Speakers: {v}%', muted: 'Sound is off', battery: 'Battery: {p}%',
    charging: '{p}% available (charging)', onAC: '{p}% available (plugged in)',
    unread: { one: '1 new notification', other: '{n} new notifications' }, dnd: 'Do not disturb is on',
    noNotifs: 'No new notifications', moreApps: 'More apps',
    own_settings: 'Settings', own_taskmgr: 'Task Manager', own_explorer: 'File Explorer', own_alles: 'Alles', own_disks: 'Disks',
  },
});

const PITCH = 44;           // 40 px button + 4 px gap
const PREVIEW_DELAY = 450;
const OWN_RE = /^win11shell-(settings|taskmgr|explorer|alles|disks)\.desktop$/;

export function createTaskbar(ctx) {
  /* ------------------------------------------------------------------ DOM */
  const startBtn = h('button', {
    type: 'button', class: 'sh-tb-btn sh-tb-start', 'aria-label': t('start'),
    dataset: { tooltip: t('start'), flyoutToggle: 'start winx', region: 'start' },
  }, h('span', { class: 'sh-tb-ico' }, startLogo(24)));

  const searchSlot = h('div', { class: 'sh-tb-search-slot' });
  const taskViewBtn = h('button', {
    type: 'button', class: 'sh-tb-btn sh-tb-taskview', 'aria-label': t('taskView'),
    dataset: { tooltip: t('taskView'), flyoutToggle: 'taskview' },
  }, h('span', { class: 'sh-tb-ico' }, taskViewGlyph()));
  const appsEl = h('div', { class: 'sh-tb-apps' });
  const center = h('div', { class: 'sh-tb-center' }, startBtn, searchSlot, taskViewBtn, appsEl);

  const netEl = h('span', { class: 'sh-tray-ico' });
  const volEl = h('span', { class: 'sh-tray-ico' });
  const batEl = h('span', { class: 'sh-tray-ico', hidden: true });
  const sysBtn = h('button', {
    type: 'button', class: 'sh-tb-sys', 'aria-label': t('quick'), dataset: { flyoutToggle: 'quick' },
  }, netEl, volEl, batEl);

  const timeEl = h('span', { class: 'sh-clock__time' });
  const dateEl = h('span', { class: 'sh-clock__date' });
  const bellEl = h('span', { class: 'sh-clock__bell' });
  const clockBtn = h('button', {
    type: 'button', class: 'sh-tb-clock', dataset: { flyoutToggle: 'notifications' },
  }, h('span', { class: 'sh-clock__text' }, timeEl, dateEl), bellEl);

  const desktopBtn = h('button', {
    type: 'button', class: 'sh-tb-desktop', 'aria-label': t('showDesktop'), dataset: { tooltip: t('showDesktop') },
  });
  // notification area: the app tray icons and the "^" overflow chevron (tray.js fills it)
  const notifyEl = h('div', { class: 'sh-ni-area' });
  const tray = h('div', { class: 'sh-tb-tray' }, notifyEl, sysBtn, clockBtn, desktopBtn);

  const bar = h('div', {
    class: 'sh-taskbar taskbar-material', role: 'toolbar', 'aria-label': t('taskbar'), 'data-region': '', 'data-region-margin': '0',
  }, center, tray);
  const sensor = h('div', { class: 'sh-tb-sensor', hidden: true, 'data-region': '', 'data-region-margin': '0' });

  /* -------------------------------------------------------------- state */
  let groups = new Map();          // key → group
  const btnByKey = new Map();      // key → button
  let runningOrder = [];           // keys of unpinned running groups, in first-seen order
  let overflowKeys = [];
  let maxApps = Infinity;
  let align = 'center';
  let searchMode = 'box';
  let searchEl = null;
  let drag = null;
  let suppressClickUntil = 0;
  const pendingBounce = new Set();
  const extraAnchors = new Map();  // name → () => Element|null (setAnchor)
  let holds = 0;                   // hold() count: keeps an auto-hiding taskbar visible

  /* ---------------------------------------------------------- search slot */
  function renderSearch() {
    const mode = getSetting('taskbar.search', 'box');
    if (mode === searchMode && searchEl) return;
    const previousInput = searchEl?.querySelector('input');
    const previousQuery = previousInput?.value || '';
    const keepFocus = previousInput === document.activeElement;
    searchMode = mode;
    searchSlot.replaceChildren();
    searchEl = null;
    if (mode === 'hidden' || mode === false) { searchSlot.hidden = true; return; }
    searchSlot.hidden = false;
    if (mode === 'icon') {
      searchEl = h('button', {
        type: 'button', class: 'sh-tb-btn sh-tb-searchicon', 'aria-label': t('search'),
        dataset: { tooltip: t('search'), flyoutToggle: 'search' },
      }, h('span', { class: 'sh-tb-ico' }, icon('search', { size: 20 })));
    } else {
      const input = h('input', {
        class: 'sh-tb-searchbox__input', type: 'search', placeholder: t('search'), value: previousQuery,
        'aria-label': t('search'), 'aria-controls': 'sh-search-results',
        'aria-expanded': 'false', autocomplete: 'off', spellcheck: false,
      });
      searchEl = h('label', {
        class: ['sh-tb-searchbox', (mode === 'label' || mode === 'iconLabel') && 'is-compact'],
        dataset: { flyoutToggle: 'search' },
      }, h('span', { class: 'sh-tb-searchbox__icon' }, icon('search', { size: 16 })),
      h('span', { class: 'sh-tb-searchbox__field' },
        h('span', { class: 'sh-tb-searchbox__label', 'aria-hidden': 'true' }, t('search')), input));
      const search = () => { hidePreview(); ctx.runCommand('taskbar-search', { input }); };
      input.addEventListener('focus', search);
      input.addEventListener('click', () => { if (currentFlyout() !== 'search') search(); });
      input.addEventListener('input', (event) => { if (!event.isComposing) search(); });
      input.addEventListener('compositionend', search);
      input.addEventListener('keydown', (event) => {
        if (event.isComposing) return;
        if (event.key === 'Escape') {
          event.preventDefault();
          ctx.closeFlyout('search');
          input.value = '';
          input.blur();
        } else ctx.runCommand('taskbar-search-key', { event });
      });
    }
    if (mode === 'icon') searchEl.addEventListener('click', () => { hidePreview(); ctx.runCommand('toggle-search'); });
    searchSlot.append(searchEl);
    if (keepFocus) requestAnimationFrame(() => searchEl?.querySelector('input')?.focus({ preventScroll: true }));
    syncActive();
  }

  /* ------------------------------------------------------------- groups */
  function computeGroups() {
    const apps = ctx.apps.list();
    const noCatalog = apps.length === 0;
    const out = new Map();
    for (const id of ctx.pins.taskbar.list()) {
      const app = ctx.apps.get(id);
      // pinned but not installed (any more): skip silently. Without a catalogue (apps.list failed)
      // only our own apps are known to exist.
      if (!app && (!noCatalog || !OWN_RE.test(String(id)))) continue;
      out.set(id, {
        key: id, appId: id, app, pinned: true, windows: [],
        name: app?.name || guessName(id), icon: app?.icon || guessIcon(id),
      });
    }
    for (const w of ctx.windows.list()) {
      const app = ctx.apps.resolve(w.desktopId, w.appId);
      const key = app ? app.id : `win:${w.appId || w.icon || w.title || w.key}`;
      let g = out.get(key);
      if (!g) {
        g = { key, appId: app?.id || null, app, pinned: false, windows: [], name: app?.name || nameFromWindow(w), icon: app?.icon || w.icon || w.appId };
        out.set(key, g);
      }
      g.windows.push(w);
    }
    // stable order for unpinned running apps
    const unpinned = [...out.values()].filter((g) => !g.pinned).map((g) => g.key);
    runningOrder = runningOrder.filter((k) => unpinned.includes(k));
    for (const k of unpinned) if (!runningOrder.includes(k)) runningOrder.push(k);
    const ordered = new Map();
    for (const g of out.values()) if (g.pinned) ordered.set(g.key, g);
    for (const k of runningOrder) ordered.set(k, out.get(k));
    return ordered;
  }

  let catalogReady = false;
  Promise.race([ctx.apps.ready, new Promise((r) => setTimeout(r, 1500))]).then(() => { catalogReady = true; render(); });

  function render() {
    if (drag?.active) return; // never reshuffle under the pointer
    if (!catalogReady) { layout(); return; } // wait for apps.list so windows group under the right app
    groups = computeGroups();
    const list = [...groups.values()];
    let visible = list;
    overflowKeys = [];
    if (list.length > maxApps) {
      visible = list.slice(0, Math.max(0, maxApps - 1));
      overflowKeys = list.slice(visible.length).map((g) => g.key);
    }
    const before = snapshotPositions();
    const wanted = visible.map((g) => g.key);
    // remove stale buttons
    for (const [key, btn] of btnByKey) {
      if (!wanted.includes(key)) {
        btnByKey.delete(key);
        leave(btn);
      }
    }
    let prev = null;
    for (const g of visible) {
      let btn = btnByKey.get(g.key);
      const isNew = !btn;
      if (isNew) { btn = makeAppButton(g.key); btnByKey.set(g.key, btn); }
      updateAppButton(btn, g);
      const ref = prev ? prev.nextSibling : appsEl.firstChild;
      if (btn !== ref) appsEl.insertBefore(btn, ref);
      prev = btn;
      if (isNew) enter(btn, g);
    }
    // overflow button
    let ovf = appsEl.querySelector('.sh-tb-overflow');
    if (overflowKeys.length) {
      if (!ovf) {
        ovf = h('button', { type: 'button', class: 'sh-tb-btn sh-tb-overflow', 'aria-label': t('moreApps'), dataset: { tooltip: t('moreApps') } },
          h('span', { class: 'sh-tb-ico' }, icon('more', { size: 20 })));
        ovf.addEventListener('click', () => openOverflow(ovf));
      }
      appsEl.append(ovf);
    } else ovf?.remove();
    playFlip(before);
    if (previewKey && !groups.get(previewKey)?.windows.length) hidePreview(true);
    else if (previewKey) refreshPreview();
    layout();
    publishIconRects();
  }

  function makeAppButton(key) {
    const btn = h('button', { type: 'button', class: 'sh-tb-btn sh-tb-app', dataset: { key } },
      h('span', { class: 'sh-tb-ico' }), h('span', { class: 'sh-tb-ind', 'aria-hidden': 'true' }));
    btn.addEventListener('click', (e) => {
      if (performance.now() < suppressClickUntil) return;
      activate(btn.dataset.key, e);
    });
    btn.addEventListener('auxclick', (e) => {
      if (e.button !== 1) return;
      e.preventDefault();
      const g = groups.get(btn.dataset.key);
      if (g?.appId) ctx.apps.launch(g.appId);
    });
    btn.addEventListener('mousedown', (e) => { if (e.button === 1) e.preventDefault(); });
    btn.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      openJumpList(btn.dataset.key);
    });
    btn.addEventListener('pointerenter', (e) => { if (e.pointerType !== 'touch') schedulePreview(btn.dataset.key); });
    btn.addEventListener('pointerleave', () => schedulePreviewHide());
    btn.addEventListener('pointerdown', (e) => startDrag(e, btn));
    return btn;
  }

  function updateAppButton(btn, g) {
    const ico = btn.querySelector('.sh-tb-ico');
    if (ico.dataset.icon !== String(g.icon)) {
      ico.dataset.icon = String(g.icon);
      ico.replaceChildren(ctx.appIcon(g.icon, 24, { request: 32 }));
    }
    const running = g.windows.length > 0;
    const focused = g.windows.some((w) => w.focused && !w.minimized);
    btn.classList.toggle('is-running', running);
    btn.classList.toggle('is-focused', focused);
    btn.classList.toggle('is-multi', g.windows.length > 1);
    btn.classList.toggle('is-pinned', g.pinned);
    const label = running ? t('running', { name: g.name, count: t('windows', { n: g.windows.length }) }) : g.name;
    btn.setAttribute('aria-label', label);
    btn.setAttribute('aria-pressed', focused ? 'true' : 'false');
    if (running) delete btn.dataset.tooltip; else btn.dataset.tooltip = g.name;
    if (pendingBounce.has(g.appId)) { pendingBounce.delete(g.appId); requestAnimationFrame(() => bounceBtn(btn)); }
  }

  function enter(btn) {
    animate(btn, [{ opacity: 0, transform: 'scale(.6)' }, { opacity: 1, transform: 'none' }], { duration: 250 });
  }
  function leave(btn) {
    if (!btn.isConnected) { btn.remove(); return; }
    btn.classList.add('is-leaving');
    btn.tabIndex = -1;
    const a = animate(btn, [{ opacity: 1, transform: 'none', width: '40px', marginRight: '0px' }, { opacity: 0, transform: 'scale(.6)', width: '0px', marginRight: '-4px' }], { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' });
    const done = () => { if (btn.isConnected) { btn.remove(); publishIconRects(); } };
    if (a) a.finished.then(done, done); else done();
    setTimeout(done, 400); // never leave a ghost button behind
  }
  function snapshotPositions() {
    const m = new Map();
    for (const [key, btn] of btnByKey) if (btn.isConnected) m.set(key, btn.getBoundingClientRect().left);
    m.set('__center', center.getBoundingClientRect().left);
    return m;
  }
  function playFlip(before) {
    for (const [key, btn] of btnByKey) {
      const old = before.get(key);
      if (old == null || !btn.isConnected) continue;
      const dx = old - btn.getBoundingClientRect().left;
      if (Math.abs(dx) > 0.5) animate(btn, [{ transform: `translateX(${dx}px)` }, { transform: 'none' }], { duration: 250 });
    }
  }

  /* ------------------------------------------------------------ actions */
  function activate(key, e) {
    const g = groups.get(key);
    if (!g) return;
    hideTooltip();
    if (!g.windows.length) {
      hidePreview(true);
      if (g.appId) { ctx.apps.launch(g.appId); }
      return;
    }
    if (e?.shiftKey && g.appId) { ctx.apps.launch(g.appId); return; }
    if (g.windows.length === 1) {
      hidePreview(true);
      const w = g.windows[0];
      // the host must know where this button is before the window flies into / out of it
      afterIconRects(() => { if (w.focused && !w.minimized) ctx.windows.minimize(w); else ctx.windows.focus(w); });
      return;
    }
    // several windows: show the window list (Win11 shows the thumbnails)
    if (previewKey === key && previewSticky) hidePreview(true);
    else showPreview(key, { sticky: true });
  }

  function bounceBtn(btn) {
    const ico = btn?.querySelector('.sh-tb-ico');
    if (!ico) return;
    animate(ico, [
      { transform: 'translateY(0)' }, { transform: 'translateY(-7px)', offset: 0.28 },
      { transform: 'translateY(0)', offset: 0.56 }, { transform: 'translateY(-2.5px)', offset: 0.76 },
      { transform: 'translateY(0)' },
    ], { duration: 720, easing: 'cubic-bezier(.3,.6,.4,1)' });
  }
  function bounce(id) {
    const btn = btnByKey.get(id);
    if (btn) bounceBtn(btn); else if (id) pendingBounce.add(id);
  }
  ctx.apps.onLaunch((id) => {
    bounce(id);
    setTimeout(() => pendingBounce.delete(id), 4000);
  });

  /* ------------------------------------------------------- hover preview */
  let previewEl = null;
  let previewKey = null;
  let previewSticky = false;
  let pvTimer = 0;
  let pvHideTimer = 0;

  function schedulePreview(key) {
    clearTimeout(pvHideTimer);
    if (drag?.active || currentFlyout() || isContextMenuOpen()) return;
    const g = groups.get(key);
    if (!g?.windows.length) { if (previewEl && !previewSticky) hidePreview(); return; }
    if (previewKey === key) return;
    clearTimeout(pvTimer);
    pvTimer = setTimeout(() => showPreview(key), previewEl ? 60 : PREVIEW_DELAY);
  }
  function schedulePreviewHide() {
    clearTimeout(pvTimer);
    clearTimeout(pvHideTimer);
    if (!previewEl) return;
    pvHideTimer = setTimeout(() => hidePreview(), previewSticky ? 900 : 380);
  }

  function showPreview(key, { sticky = false } = {}) {
    const g = groups.get(key);
    const btn = btnByKey.get(key);
    if (!g?.windows.length || !btn) return;
    hideTooltip();
    clearTimeout(pvTimer);
    clearTimeout(pvHideTimer);
    const fresh = !previewEl;
    if (fresh) {
      previewEl = h('div', { class: 'sh-tb-preview sh-acrylic', role: 'menu', 'data-region': '', 'data-popup': '' });
      previewEl.addEventListener('pointerenter', () => clearTimeout(pvHideTimer));
      previewEl.addEventListener('pointerleave', () => schedulePreviewHide());
      previewEl.addEventListener('keydown', onPreviewKey);
      ctx.root.append(previewEl);
    }
    previewKey = key;
    previewSticky = sticky || previewSticky;
    fillPreview(g);
    placePreview(btn);
    if (fresh) animate(previewEl, [{ transform: 'translateY(10px)' }, { transform: 'none' }], { duration: 200 });
    if (sticky) requestAnimationFrame(() => previewEl?.querySelector('.sh-pv-item')?.focus({ preventScroll: true }));
  }
  function refreshPreview() {
    const g = groups.get(previewKey);
    const btn = btnByKey.get(previewKey);
    if (!g || !btn || !previewEl) { hidePreview(true); return; }
    fillPreview(g);
    placePreview(btn);
  }
  function fillPreview(g) {
    previewEl.replaceChildren(h('div', { class: 'sh-pv-head' }, ctx.appIcon(g.icon, 16, { request: 32 }), h('span', { class: 'sh-pv-app' }, g.name)),
      ...g.windows.map((w) => {
        const closeB = h('button', { type: 'button', class: 'sh-pv-close', tabindex: -1, 'aria-label': t('close'), dataset: { tooltip: t('close') } }, icon('close', { size: 12 }));
        const item = h('div', {
          class: ['sh-pv-item', w.focused && !w.minimized && 'is-current', w.minimized && 'is-minimized'],
          role: 'menuitem', tabindex: -1, dataset: { key: w.key },
        }, h('span', { class: 'sh-pv-title', title: w.title || '' }, cleanTitle(w.title, g.name) || g.name), closeB);
        item.addEventListener('click', (e) => {
          if (e.target.closest('.sh-pv-close')) return;
          hidePreview(true);
          afterIconRects(() => ctx.windows.focus(w));   // a minimized window grows out of the group's button
        });
        closeB.addEventListener('click', (e) => {
          e.stopPropagation();
          animate(item, [{ opacity: 1 }, { opacity: 0.3 }], { duration: 120, fill: 'forwards' });
          ctx.windows.close(w);
        });
        item.addEventListener('auxclick', (e) => { if (e.button === 1) { e.preventDefault(); ctx.windows.close(w); } });
        return item;
      }));
  }
  function placePreview(btn) {
    const r = btn.getBoundingClientRect();
    const barR = bar.getBoundingClientRect();
    const w = previewEl.offsetWidth;
    previewEl.style.left = `${Math.round(clamp(r.left + r.width / 2 - w / 2, 8, innerWidth - w - 8))}px`;
    previewEl.style.bottom = `${Math.round(innerHeight - barR.top + 8)}px`;
  }
  function onPreviewKey(e) {
    const items = [...previewEl.querySelectorAll('.sh-pv-item')];
    const i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const j = i < 0 ? 0 : (i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length;
      items[j]?.focus();
    } else if ((e.key === 'Enter' || e.key === ' ') && i >= 0) {
      e.preventDefault();
      items[i].click();
    } else if (e.key === 'Delete' && i >= 0) {
      e.preventDefault();
      items[i].querySelector('.sh-pv-close')?.click();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      const key = previewKey;
      hidePreview(true);
      btnByKey.get(key)?.focus();
    }
  }
  function hidePreview(immediate = false) {
    clearTimeout(pvTimer);
    clearTimeout(pvHideTimer);
    const el = previewEl;
    previewEl = null;
    previewKey = null;
    previewSticky = false;
    if (!el) return;
    el.removeAttribute('data-region');
    if (immediate) { el.remove(); return; }
    const a = animate(el, [{ opacity: 1 }, { opacity: 0 }], { duration: 120, easing: 'linear', fill: 'forwards' });
    if (a) a.finished.catch(() => {}).then(() => el.remove()); else el.remove();
  }
  document.addEventListener('pointerdown', (e) => {
    if (previewEl && !previewEl.contains(e.target) && !e.target.closest?.('.sh-tb-app')) hidePreview(true);
  }, true);
  addEventListener('blur', () => hidePreview(true));

  /* ----------------------------------------------------------- jump list */
  function openJumpList(key) {
    const g = groups.get(key) || (key && [...groups.values()].find((x) => x.appId === key || x.key.includes(key)));
    const btn = g && btnByKey.get(g.key);
    if (!g || !btn) return;
    hidePreview(true);
    hideTooltip();
    const items = [];
    if (g.appId) items.push({ label: g.name, icon: ctx.appIcon(g.icon, 16, { request: 32 }), action: () => ctx.apps.launch(g.appId) });
    if (g.appId) {
      items.push(g.pinned
        ? { label: t('unpin'), icon: 'unpin', action: () => ctx.pins.taskbar.remove(g.appId) }
        : { label: t('pin'), icon: 'pin', action: () => ctx.pins.taskbar.add(g.appId) });
    }
    if (g.windows.length) {
      items.push({
        label: g.windows.length > 1 ? t('closeAll') : t('closeWindow'), icon: 'close',
        action: () => g.windows.forEach((w) => ctx.windows.close(w)),
      });
    }
    btn.classList.add('is-menu');
    showContextMenu({ anchor: btn, placement: 'top', align: 'center', items, onClose: () => btn.classList.remove('is-menu') });
  }

  function openOverflow(anchorBtn) {
    const items = overflowKeys.map((k) => groups.get(k)).filter(Boolean).map((g) => ({
      label: g.windows.length ? `${g.name} (${g.windows.length})` : g.name,
      icon: ctx.appIcon(g.icon, 16, { request: 32 }),
      action: () => activate(g.key),
    }));
    showContextMenu({ anchor: anchorBtn, placement: 'top', align: 'center', items });
  }

  /* ------------------------------------------------------- context menus */
  function taskbarMenuItems() {
    const tm = ctx.apps.get('win11shell-taskmgr.desktop');
    return [
      { label: t('taskmgr'), icon: ctx.appIcon(tm?.icon || 'win11shell-taskmgr', 16, { glyph: 'chart' }), action: () => ctx.openApp('taskmgr') },
      { label: t('taskbarSettings'), icon: 'settings', action: () => ctx.openSettings(['taskbar', 'personalization']) },
    ];
  }
  function openContextMenu(x, y) {
    hidePreview(true);
    if (x == null) {
      const r = anchor('center');
      x = Math.min(innerWidth - 240, r.right + 24);
      y = bar.getBoundingClientRect().top + 24;
    }
    showContextMenu({ x, y, items: taskbarMenuItems() });
  }
  bar.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    hidePreview(true);
    const target = e.target;
    if (target.closest('.sh-tb-start')) { ctx.runCommand('winx', { mouse: true }); return; }
    if (target.closest('.sh-tb-clock')) {
      showContextMenu({ x: e.clientX, y: e.clientY, items: [
        { label: t('dateTime'), icon: 'clock', action: () => ctx.openSettings(['datetime', 'time']) },
        { label: t('notifSettings'), icon: 'notification', action: () => ctx.openSettings(['notifications']) },
      ] });
      return;
    }
    if (target.closest('.sh-tb-sys')) {
      showContextMenu({ x: e.clientX, y: e.clientY, items: [
        { label: t('netSettings'), icon: 'globe', action: () => ctx.openSettings(['network']) },
        { label: t('soundSettings'), icon: 'sound', action: () => ctx.openSettings(['sound']) },
        { separator: true },
        ...taskbarMenuItems(),
      ] });
      return;
    }
    openContextMenu(e.clientX, e.clientY);
  });

  /* ---------------------------------------------------- drag to reorder */
  function startDrag(e, btn) {
    if (e.button !== 0 || e.pointerType === 'touch') return;
    drag = { btn, key: btn.dataset.key, x0: e.clientX, y0: e.clientY, id: e.pointerId, active: false, moved: false };
    const move = (ev) => onDragMove(ev);
    const up = (ev) => {
      removeEventListener('pointermove', move, true);
      removeEventListener('pointerup', up, true);
      removeEventListener('pointercancel', up, true);
      onDragEnd(ev);
    };
    addEventListener('pointermove', move, true);
    addEventListener('pointerup', up, true);
    addEventListener('pointercancel', up, true);
  }
  function onDragMove(e) {
    if (!drag || e.pointerId !== drag.id) return;
    const dx = e.clientX - drag.x0;
    if (!drag.active) {
      if (Math.abs(dx) < 6 && Math.abs(e.clientY - drag.y0) < 6) return;
      drag.active = true;
      hidePreview(true);
      hideTooltip();
      drag.btn.classList.add('is-dragging');
      try { drag.btn.setPointerCapture(drag.id); } catch { /* ignore */ }
    }
    const siblings = [...appsEl.querySelectorAll('.sh-tb-app:not(.is-leaving)')];
    const others = siblings.filter((b) => b !== drag.btn);
    // where would the dragged button go?
    let index = others.length;
    for (let i = 0; i < others.length; i++) {
      const r = others[i].getBoundingClientRect();
      if (e.clientX < r.left + r.width / 2) { index = i; break; }
    }
    const cur = siblings.indexOf(drag.btn);
    if (index !== cur) {
      const before = snapshotPositions();
      const ref = others[index] || appsEl.querySelector('.sh-tb-overflow');
      appsEl.insertBefore(drag.btn, ref || null);
      for (const b of others) {
        const old = before.get(b.dataset.key);
        const dxx = old - b.getBoundingClientRect().left;
        if (Math.abs(dxx) > 0.5) animate(b, [{ transform: `translateX(${dxx}px)` }, { transform: 'none' }], { duration: 200 });
      }
    }
    // follow the pointer
    drag.btn.style.transform = 'none';
    const slot = drag.btn.getBoundingClientRect();
    drag.btn.style.transform = `translateX(${Math.round(e.clientX - (slot.left + slot.width / 2))}px)`;
  }
  function onDragEnd() {
    const d = drag;
    drag = null;
    if (!d || !d.active) return;
    suppressClickUntil = performance.now() + 250;
    const tr = d.btn.style.transform;
    d.btn.style.transform = '';
    d.btn.classList.remove('is-dragging');
    animate(d.btn, [{ transform: tr || 'none' }, { transform: 'none' }], { duration: 200 });
    const keys = [...appsEl.querySelectorAll('.sh-tb-app:not(.is-leaving)')].map((b) => b.dataset.key);
    const pins = ctx.pins.taskbar.list();
    const visiblePinned = keys.filter((k) => groups.get(k)?.pinned);
    const hiddenPinned = pins.filter((k) => !visiblePinned.includes(k));
    const nextPins = [...visiblePinned, ...hiddenPinned];
    runningOrder = keys.filter((k) => !groups.get(k)?.pinned).concat(runningOrder.filter((k) => !keys.includes(k)));
    if (nextPins.join('|') !== pins.join('|')) ctx.pins.taskbar.set(nextPins);
    else render();
  }

  /* ---------------------------------------------------------- tray icons */
  const sys = { net: undefined, audio: undefined, battery: undefined };
  const loadSys = (method, key) => bridge.call(method)
    .then((v) => { sys[key] = v || null; renderTray(); })
    .catch(() => { sys[key] = null; renderTray(); });
  loadSys('system.network.get', 'net');
  loadSys('system.audio.get', 'audio');
  loadSys('system.battery.get', 'battery');
  bridge.on('system.network.changed', (d) => { sys.net = d; renderTray(); });
  bridge.on('system.audio.changed', (d) => { sys.audio = d; renderTray(); });
  bridge.on('system.battery.changed', (d) => { sys.battery = d; renderTray(); });

  function setGlyph(el, name, size = 16) {
    if (el.dataset.glyph === name) return;
    el.dataset.glyph = name;
    el.replaceChildren(icon(name, { size }));
  }
  function renderTray() {
    const n = sys.net;
    let netGlyph = 'globe';
    let netTip = t('netUnknown');
    if (n) {
      const p = n.primary || {};
      if (n.airplane) { netGlyph = 'airplane'; netTip = t('airplane'); }
      else if (p.type === 'ethernet') { netGlyph = 'ethernet'; netTip = `${p.name || t('wired')}\n${n.connectivity === 'full' || !n.connectivity ? t('internet') : t('noInternet')}`; }
      else if (p.type === 'wifi') { netGlyph = wifiIcon(p.strength); netTip = `${p.name || 'Wi-Fi'}\n${n.connectivity === 'full' || !n.connectivity ? t('internet') : t('noInternet')}`; }
      else { netGlyph = n.wifiAvailable && n.wifiEnabled !== false ? 'wifi-off' : 'globe'; netTip = t('notConnected'); }
    }
    setGlyph(netEl, netGlyph);
    netEl.dataset.tooltip = netTip;

    const a = sys.audio;
    volEl.hidden = a?.available === false;
    const vol = Math.round(Number(a?.volume ?? 50));
    setGlyph(volEl, a ? volumeIcon(vol, !!a.muted) : 'volume');
    volEl.dataset.tooltip = a ? (a.muted ? t('muted') : t('speakers', { v: vol })) : t('soundSettings');

    const b = sys.battery;
    batEl.hidden = !b?.present;
    if (b?.present) {
      const p = Math.round(Number(b.percent) || 0);
      const charging = b.state === 'charging' || (b.onAC && p < 100);
      batEl.replaceChildren(batteryGlyph(p, charging));
      batEl.dataset.glyph = '';
      batEl.dataset.tooltip = charging ? t('charging', { p }) : b.onAC ? t('onAC', { p }) : t('battery', { p });
    }
  }
  renderTray();

  // Win11 24H2: scrolling over the tray changes the volume
  volEl.addEventListener('wheel', (e) => {
    const a = sys.audio;
    if (!a || a.available === false) return;
    e.preventDefault();
    const step = e.deltaY < 0 ? 2 : -2;
    const v = clamp(Math.round(Number(a.volume) || 0) + step, 0, 100);
    sys.audio = { ...a, volume: v, muted: v === 0 ? a.muted : false };
    renderTray();
    bridge.call('system.audio.set', { volume: v, muted: false }).catch(() => {});
    ctx.runCommand('osd', { kind: 'volume', value: v });
  }, { passive: false });

  sysBtn.addEventListener('click', () => { hidePreview(true); ctx.runCommand('quick-settings'); });

  /* --------------------------------------------------------------- clock */
  let clockTimer = 0;
  function tick() {
    clearTimeout(clockTimer);
    const s = getSettings();
    const now = Date.now();
    const h24 = s?.time?.h24 !== false;
    const seconds = !!(s?.time?.showSeconds || s?.taskbar?.showSeconds);
    const showDate = s?.time?.showDate !== false;
    timeEl.textContent = fmtTime(now, { seconds, h24 });
    dateEl.textContent = showDate ? fmtDate(now, 'short') : '';
    dateEl.hidden = !showDate;
    clockBtn.classList.toggle('is-single', !showDate);
    clockBtn.dataset.tooltip = `${fmtDate(now, 'long')}\n${fmtTime(now, { seconds: true, h24 })}`;
    const step = seconds ? 1000 : 60000;
    clockTimer = setTimeout(tick, step - (now % step) + 20);
  }
  clockBtn.addEventListener('click', () => { hidePreview(true); ctx.runCommand('notifications'); });

  /* ---------------------------------------------------- notifications */
  let notifItems = [];
  const seen = new Set();
  function unreadCount() { return notifItems.filter((n) => !seen.has(n.id)).length; }
  function renderBell() {
    const dnd = !!getSetting('notifications.dnd', false);
    const n = unreadCount();
    bellEl.replaceChildren();
    bellEl.className = 'sh-clock__bell';
    if (dnd) {
      bellEl.classList.add('is-dnd');
      bellEl.append(icon('notification-off', { size: 16 }));
    } else if (n > 0) {
      bellEl.classList.add('has-count');
      bellEl.append(h('span', { class: 'sh-clock__count' }, n > 9 ? '9+' : String(n)));
    } else {
      bellEl.append(icon('notification', { size: 16 }));
    }
    clockBtn.setAttribute('aria-label', `${timeEl.textContent} ${dateEl.textContent}. ${dnd ? t('dnd') : n ? t('unread', { n }) : t('noNotifs')}`);
  }
  bridge.call('notifications.list').then((r) => {
    notifItems = Array.isArray(r?.items) ? r.items : [];
    renderBell();
  }).catch(() => renderBell());
  bridge.on('notifications.new', (d) => {
    const n = d?.notification;
    if (!n) return;
    notifItems = [n, ...notifItems.filter((x) => x.id !== n.id)];
    if (currentFlyout() === 'notifications') seen.add(n.id);
    renderBell();
    if (!getSetting('notifications.dnd', false)) {
      animate(bellEl, [{ transform: 'scale(1)' }, { transform: 'scale(1.25)' }, { transform: 'scale(1)' }], { duration: 400 });
    }
  });
  bridge.on('notifications.closed', (d) => {
    notifItems = notifItems.filter((x) => x.id !== d?.id);
    renderBell();
  });

  /* ------------------------------------------------------- show desktop */
  desktopBtn.addEventListener('click', () => { hidePreview(true); ctx.runCommand('show-desktop'); });

  /* ---------------------------------------------------- start & taskview */
  startBtn.addEventListener('click', () => { hidePreview(true); ctx.runCommand('toggle-start'); });
  taskViewBtn.addEventListener('click', () => { hidePreview(true); ctx.runCommand('task-view'); });
  for (const b of [startBtn, taskViewBtn]) b.addEventListener('pointerenter', () => { if (previewEl && !previewSticky) hidePreview(); });

  /* ------------------------------------------------- flyout active state */
  function syncActive() {
    const cur = currentFlyout();
    startBtn.classList.toggle('is-active', cur === 'start' || cur === 'winx');
    searchEl?.classList.toggle('is-active', cur === 'search');
    searchEl?.querySelector('input')?.setAttribute('aria-expanded', String(cur === 'search'));
    taskViewBtn.classList.toggle('is-active', cur === 'taskview');
    sysBtn.classList.toggle('is-active', cur === 'quick');
    clockBtn.classList.toggle('is-active', cur === 'notifications');
  }
  onFlyoutChange(({ name, open }) => {
    syncActive();
    if (name === 'search' && !open) {
      const input = searchEl?.querySelector('input');
      if (input) input.value = '';
    }
    if (open) { hidePreview(true); reveal(); } else scheduleAutoHide();
    if (name === 'notifications' && open) {
      for (const n of notifItems) seen.add(n.id);
      renderBell();
    }
  });

  /* ------------------------------------------------------------- layout */
  function layout() {
    const vw = bar.clientWidth || innerWidth;
    const trayLeft = tray.offsetLeft || vw;
    // Keep the same clearance on both sides so the group never shifts away from screen centre.
    const rightReserve = vw - trayLeft + 16;
    const avail = Math.max(PITCH, vw - 2 * rightReserve);
    const fixed = center.offsetWidth - appsEl.offsetWidth;
    const cap = Math.max(1, Math.floor((avail - fixed + 4) / PITCH));
    if (cap !== maxApps && (groups.size > cap || maxApps < groups.size)) {
      maxApps = cap;
      render();
      return;
    }
    maxApps = cap;
    const left = centerLeftFor(center.offsetWidth);
    centerLeft = left;
    center.style.left = `${left}px`;
    if (!layoutSettled) {
      layoutSettled = true;
      setTimeout(() => center.classList.add('is-animated'), 700);
    }
    publishIconRects();
  }
  /** Where the centre group's left edge goes for a group `w` px wide (left-aligned, or centred clear of the tray). */
  function centerLeftFor(w) {
    if (align === 'left') return 8;
    const vw = bar.clientWidth || innerWidth;
    return Math.round((vw - w) / 2);
  }
  let centerLeft = null;
  let layoutSettled = false;
  const relayout = debounce(layout, 16);
  try {
    const ro = new ResizeObserver(relayout);
    ro.observe(bar);
    ro.observe(center);
    ro.observe(tray);
  } catch { addEventListener('resize', relayout); }

  /* ------------------------------------------- icon rects (window animations) */
  // Where each host window's button comes to rest, in shell-surface px (= the layer's logical px):
  // offset* ignore CSS transforms (:active, enter(), playFlip, a drag, .is-hidden's translateY) and the
  // centre group's animated `left` is replaced by its target (centerLeft). Groups in the overflow use
  // the "…" button. The host makes them global and, while the bar is auto-hidden, moves them onto the
  // screen's bottom edge (SPEC §6.4 windows.setIconRects).
  // A button that leaves (leave()) still takes room while its width and margin animate in layout (to 0 and
  // -4 px, which with the 4 px gap is no room at all): what it still takes is left out, and the centre group's
  // left edge is the one for its final width, so the rects are where the buttons come to rest.
  const iconRectsOn = () => regionsActive() || bridge.params?.get?.('iconRects') === '1';
  const GAP = PITCH - 40;
  let iconTimer = 0;
  let lastIconKey = '';
  function leavingRoom(el) {
    const cs = getComputedStyle(el);
    return el.offsetWidth + (parseFloat(cs.marginLeft) || 0) + (parseFloat(cs.marginRight) || 0) + GAP;
  }
  function settledRect(el, settle) {
    let x = 0;
    let y = 0;
    for (let n = el; n; n = n.offsetParent) {
      x += (n === center && centerLeft != null) ? (settle.left ?? centerLeft) : n.offsetLeft;
      y += n.offsetTop;
      if (n.offsetParent) { x += n.offsetParent.clientLeft; y += n.offsetParent.clientTop; }
    }
    if (el.parentNode === appsEl) {
      for (const [l, room] of settle.leaving) if (l.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_FOLLOWING) x -= room;
    }
    return { x: Math.round(x), y: Math.round(y), w: el.offsetWidth, h: el.offsetHeight };
  }
  function settledLayout() {
    const leaving = [...appsEl.children].filter((b) => b.classList.contains('is-leaving')).map((b) => [b, leavingRoom(b)]);
    if (!leaving.length) return { leaving, left: null };
    const room = leaving.reduce((t, [, r]) => t + r, 0);
    const width = center.offsetWidth - appsEl.offsetWidth + Math.max(0, appsEl.offsetWidth - room);
    return { leaving, left: centerLeftFor(width) };
  }
  function iconRectsMessage() {
    const rects = [];
    let total = 0;
    const settle = settledLayout();
    const hostIds = (key) => (groups.get(key)?.windows || []).filter((w) => w.source === 'host').map((w) => w.id);
    const add = (el, ids) => {
      if (!ids.length || !el.isConnected || el.offsetParent === null || rects.length >= 64) return;
      const r = settledRect(el, settle);
      if (r.w <= 0 || r.h <= 0 || r.w > 512 || r.h > 512) return;
      const take = ids.slice(0, Math.min(64, 256 - total));
      if (!take.length) return;
      total += take.length;
      rects.push({ ids: take, ...r });
    };
    for (const [key, btn] of btnByKey) if (!btn.classList.contains('is-leaving')) add(btn, hostIds(key));
    const ovf = appsEl.querySelector('.sh-tb-overflow');
    if (ovf) add(ovf, overflowKeys.flatMap(hostIds));
    return { layer: 'shell', hidden: bar.classList.contains('is-hidden'), rects };
  }
  // One setIconRects call at a time (the host serves requests on several threads, so two in flight could be
  // applied in the wrong order): a message made meanwhile waits, and only the newest one waiting is sent.
  let iconBusy = false;
  let iconQueued = null;
  let iconDone = null;     // resolves once the host has the newest message sent or queued
  function deliver(msg) {
    if (iconBusy) { iconQueued = msg; return iconDone; }
    iconBusy = true;
    // (a call that hangs is not waited for longer than 2 s, so the taskbar never stops publishing)
    const run = (m) => Promise.race([bridge.call('windows.setIconRects', m), new Promise((r) => { setTimeout(r, 2000); })])
      .then((r) => { try { ctx.windows?.noteAnim?.(r?.anim); } catch { /* never stops the queue */ } }, () => { lastIconKey = ''; })
      .then(() => {
        const next = iconQueued;
        iconQueued = null;
        if (next) return run(next);
        iconBusy = false;
        return undefined;
      });
    iconDone = run(msg);
    return iconDone;
  }
  /** Send the rects now if they changed; resolves when the host has them (null: it has them already). */
  function sendIconRects() {
    clearTimeout(iconTimer);
    iconTimer = 0;
    if (!iconRectsOn()) return null;
    const msg = iconRectsMessage();
    const key = JSON.stringify(msg);
    if (key === lastIconKey) return iconBusy ? iconDone : null;
    lastIconKey = key;
    return deliver(msg);
  }
  /** Debounced (40 ms + a frame, like core/regions.js): call after anything that can move a button.
   *  A surface that gets no frames (throttled, headless) still sends after 120 ms. */
  function publishIconRects() {
    if (iconTimer || !iconRectsOn()) return;
    iconTimer = setTimeout(() => {
      iconTimer = 0;
      let sent = false;
      const go = () => { if (!sent) { sent = true; sendIconRects(); } };
      requestAnimationFrame(go);
      setTimeout(go, 120);
    }, 40);
  }
  /** Run fn once the host knows the current rects (≤ 150 ms) and, when a flyout / menu just gave the keyboard
   *  back, once the compositor has that (≤ 400 ms; Hyprland refuses focuswindow while the shell layer holds the
   *  keyboard exclusively: a click that closes Start and restores a window would leave it unfocused). At once
   *  when nothing is pending. */
  function afterIconRects(fn) {
    const capped = (p, ms) => Promise.race([p, new Promise((r) => { setTimeout(r, ms); })]);
    const waits = [];
    const rects = sendIconRects();
    if (rects) waits.push(capped(rects, 150));
    if (keyboardSettling()) waits.push(capped(flushRegions(), 400));
    if (!waits.length) { fn(); return; }
    Promise.all(waits).then(fn, fn);
  }
  center.addEventListener('transitionend', (e) => { if (e.target === center) publishIconRects(); });
  addEventListener('pagehide', () => {
    if (iconRectsOn()) bridge.call('windows.setIconRects', { layer: 'shell', hidden: false, rects: [] }).catch(() => {});
  });

  /* ------------------------------------------------------------ autohide */
  let autoHide = false;
  let hideTimer = 0;
  let hovering = false;
  bar.addEventListener('pointerenter', () => { hovering = true; reveal(); });
  bar.addEventListener('pointerleave', () => { hovering = false; scheduleAutoHide(); });
  // The host says the pointer left the shell surface (GTK leave-notify; layer mode), e.g. straight up off the
  // bar: the input region is only the bar, and WebKit keeps its last pointer position inside it, so no
  // pointerleave ever comes for the bar, a button or the preview.
  addEventListener('shell-pointerleft', () => {
    if (hovering) { hovering = false; scheduleAutoHide(); }
    schedulePreviewHide();
  });
  sensor.addEventListener('pointerenter', () => reveal());
  bar.addEventListener('focusin', (e) => { if (e.target.matches?.(':focus-visible')) reveal(); });
  bar.addEventListener('focusout', () => scheduleAutoHide());
  function reveal() {
    clearTimeout(hideTimer);
    if (!bar.classList.contains('is-hidden')) return;
    bar.classList.remove('is-hidden');
    sensor.hidden = true;
    publishIconRects();
  }
  function mustStay() {
    return hovering || holds > 0 || !!currentFlyout() || isContextMenuOpen() || !!previewEl || !!drag || !!bar.querySelector(':focus-visible');
  }
  /** Keep the taskbar up (e.g. while a tray icon is dragged); returns release(). */
  function hold() {
    holds++;
    reveal();
    let released = false;
    return () => {
      if (released) return;
      released = true;
      holds = Math.max(0, holds - 1);
      scheduleAutoHide();
    };
  }
  function scheduleAutoHide() {
    clearTimeout(hideTimer);
    if (!autoHide) return;
    hideTimer = setTimeout(() => {
      if (!autoHide) return;
      if (mustStay()) { scheduleAutoHide(); return; }
      bar.classList.add('is-hidden');
      sensor.hidden = false;
      publishIconRects();
    }, 800);
  }

  /* ------------------------------------------------------------ keyboard */
  bar.addEventListener('keydown', (e) => {
    if (e.target.closest('.sh-tb-searchbox__input')) return; // preserve caret and selection keys
    const items = [...bar.querySelectorAll('.sh-tb-btn, .sh-tb-searchbox__input, .sh-tb-sys, .sh-tb-clock, .sh-tb-desktop')]
      .filter((x) => !x.hidden && x.offsetParent !== null);
    const i = items.indexOf(document.activeElement);
    if (i < 0) return;
    let j = -1;
    if (e.key === 'ArrowRight') j = Math.min(items.length - 1, i + 1);
    else if (e.key === 'ArrowLeft') j = Math.max(0, i - 1);
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = items.length - 1;
    else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
      e.preventDefault();
      const el = items[i];
      if (el.classList.contains('sh-tb-app')) openJumpList(el.dataset.key);
      else if (el === startBtn) ctx.runCommand('winx');
      else { const r = el.getBoundingClientRect(); openContextMenu(r.left + 8, r.top); }
      return;
    } else if (e.key === 'ArrowUp' && document.activeElement.classList.contains('sh-tb-app')) {
      e.preventDefault();
      if (groups.get(document.activeElement.dataset.key)?.windows.length) showPreview(document.activeElement.dataset.key, { sticky: true });
      return;
    }
    if (j >= 0) { e.preventDefault(); items[j].focus(); }
  });

  /* ------------------------------------------------------------ settings */
  function applySettings() {
    const s = getSettings();
    const newAlign = s?.taskbar?.alignment === 'left' ? 'left' : 'center';
    const alignChanged = newAlign !== align;
    align = newAlign;
    bar.dataset.align = align;
    if (alignChanged) {
      center.classList.add('is-moving');
      setTimeout(() => center.classList.remove('is-moving'), 400);
    }
    taskViewBtn.hidden = s?.taskbar?.taskView === false;
    renderSearch();
    const ah = !!s?.taskbar?.autoHide;
    if (ah !== autoHide) {
      autoHide = ah;
      bar.classList.toggle('is-autohide', ah);
      if (ah) scheduleAutoHide(); else { reveal(); sensor.hidden = true; }
    }
    tick();
    renderBell();
    render();
    publishIconRects();
  }
  onSettings((s, path) => {
    // taskbar.tray.* (icon order, shown/hidden) is tray.js's business: no need to rebuild the apps
    if (!path || /^(taskbar(?!\.tray\b)|time|notifications|language|accessibility)/.test(path)) applySettings();
  });
  onLangChange(() => {
    startBtn.setAttribute('aria-label', t('start'));
    startBtn.dataset.tooltip = t('start');
    taskViewBtn.dataset.tooltip = t('taskView');
    sysBtn.setAttribute('aria-label', t('quick'));
    desktopBtn.dataset.tooltip = t('showDesktop');
    searchMode = null;
    renderSearch();
    renderTray();
    tick();
    renderBell();
  });

  ctx.apps.onChange(() => render());
  ctx.windows.onChange(() => render());

  applySettings();
  requestAnimationFrame(layout);

  /* ------------------------------------------------------------- anchors */
  function anchor(name) {
    let extra = null;
    try { extra = extraAnchors.get(name)?.() || null; } catch { extra = null; }
    const el = extra || {
      start: startBtn, search: searchEl && !searchSlot.hidden ? searchEl : startBtn, taskview: taskViewBtn,
      quick: sysBtn, clock: clockBtn, notifications: clockBtn, center,
      tray: notifyEl.offsetWidth ? notifyEl : sysBtn,
    }[name] || bar;
    const r = el.getBoundingClientRect();
    // report where the element ends up, not where a running slide animation currently has it
    let dx = 0;
    if (center.contains(el) && centerLeft != null) dx = centerLeft + bar.getBoundingClientRect().left - center.getBoundingClientRect().left;
    const dy = bar.classList.contains('is-hidden') ? -bar.offsetHeight : 0;
    return new DOMRect(r.left + dx, r.top + dy, r.width, r.height);
  }

  return {
    el: bar,
    nodes: [bar, sensor],
    anchor,
    openContextMenu: () => openContextMenu(),
    openJumpList: (id) => openJumpList(id || [...groups.keys()][0]),
    openPreview: (id) => {
      const key = id ? [...groups.keys()].find((k) => k === id || k.includes(id)) : [...groups.values()].find((g) => g.windows.length > 1)?.key;
      if (key) showPreview(key, { sticky: true });
    },
    bounce,
    reveal,
    hold,
    notifyArea: notifyEl,
    setAnchor(name, fn) {
      if (typeof fn === 'function') extraAnchors.set(name, fn); else extraAnchors.delete(name);
    },
    get groups() { return groups; },
  };
}

/* ------------------------------------------------------------------ helpers */
/** "Recept — Mozilla Firefox" → "Recept" when the group already says Firefox. */
function cleanTitle(title, appName) {
  const tt = String(title || '');
  const name = String(appName || '').trim();
  if (!name) return tt;
  const esc = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const out = tt.replace(new RegExp(`\\s+[—–-]\\s+(?:mozilla\\s+)?${esc}$`, 'i'), '').trim();
  return out || tt;
}
function guessName(id) {
  const own = OWN_RE.exec(String(id));
  if (own) return t(`own_${own[1]}`);
  const base = String(id).replace(/\.desktop$/, '').replace(/^(.+)_\1$/, '$1').split('.').pop();
  return base.charAt(0).toUpperCase() + base.slice(1);
}
function guessIcon(id) { return String(id).replace(/\.desktop$/, '').replace(/^(.+)_\1$/, '$1'); }
function nameFromWindow(w) {
  const title = String(w.title || '');
  const tail = title.split(/\s[—–-]\s/).pop();
  if (tail && tail !== title && tail.length < 32) return tail.trim();
  return guessName(w.appId || title || 'App');
}

/** The Win11 Task View glyph: a dark front pane over a lighter back pane. */
function taskViewGlyph() {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 24 24');
  s.setAttribute('width', '24');
  s.setAttribute('height', '24');
  s.setAttribute('class', 'sh-taskview-glyph');
  s.setAttribute('aria-hidden', 'true');
  s.innerHTML = '<rect class="back" x="8.5" y="3.5" width="12" height="11" rx="2"/>'
    + '<rect class="front" x="3.5" y="8.5" width="12" height="11" rx="2"/>';
  return s;
}

/** Battery glyph with the charge level drawn in (Win11 style). */
function batteryGlyph(percent, charging) {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 20 20');
  s.setAttribute('width', '16');
  s.setAttribute('height', '16');
  s.setAttribute('class', 'fl-icon sh-battery');
  s.setAttribute('aria-hidden', 'true');
  const w = Math.max(1, Math.round(10.5 * Math.min(100, Math.max(0, percent)) / 100 * 10) / 10);
  const low = percent <= 10 && !charging;
  s.innerHTML = '<rect x="2" y="5.5" width="14" height="9" rx="2" fill="none" stroke="currentColor"/>'
    + '<path d="M18 8.5v3" stroke="currentColor" stroke-linecap="round"/>'
    + `<rect x="3.75" y="7.25" width="${w}" height="5.5" rx="1" fill="${low ? '#e81123' : 'currentColor'}"/>`
    + (charging ? '<path d="M9.8 6.2 7.6 10.3h2.6l-1 3.5 2.9-4.4H9.5l1-3.2Z" fill="var(--sh-tb-tint-solid, #202020)" stroke="currentColor" stroke-width=".8" stroke-linejoin="round"/>' : '');
  return s;
}
