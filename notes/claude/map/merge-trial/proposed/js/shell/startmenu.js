// Win11 Shell — Start menu (owned by the shell agent). SPEC §11: setup(ctx), flyout 'start',
// command 'toggle-start' (args: {view:'all'|'home'}).
//
// 642×726 Acrylic panel: search box (typing turns Start into the search view, in place),
// "Vastgemaakt" grid (6 × 3, 18 per page, page dots, wheel paging, drag to reorder),
// "Alle apps" (alphabetical with letter headers + letter-jump grid), "Aanbevolen" (recent files +
// newly installed apps, "Meer"), footer with the user (account menu), folder shortcuts and the
// power menu (routed through ctx.runCommand('power', {action}) so power.js can confirm).

import { bridge } from '../core/bridge.js';
import { h, animate, clamp } from '../core/dom.js';
import { icon } from '../core/icons.js';
import { makeT, onLangChange, collator } from '../core/i18n.js';
import { getSetting, onSettings } from '../core/theme.js';
import { showContextMenu } from '../core/contextmenu.js';
import { populateSoftwareRemoval, softwareRemovalPlaceholder } from '../core/software.js';
import { Avatar, hideTooltip, Toast } from '../core/components.js';
import { relTime, time as fmtTime, date as fmtDate } from '../core/format.js';

const t = makeT({
  nl: {
    start: 'Start', searchPh: 'Zoek naar apps, instellingen en documenten', pinned: 'Vastgemaakt', allApps: 'Alle apps',
    back: 'Terug', recommended: 'Aanbevolen', more: 'Meer', page: 'Pagina {n}',
    unpinStart: 'Losmaken van Start', pinStart: 'Vastmaken aan Start', pinTaskbar: 'Vastmaken aan taakbalk',
    unpinTaskbar: 'Losmaken van taakbalk', moveFront: 'Naar voren verplaatsen', open: 'Openen',
    openLocation: 'Bestandslocatie openen', copyPath: 'Pad kopiëren', copied: 'Pad gekopieerd',
    accountSettings: 'Accountinstellingen wijzigen', lock: 'Vergrendelen', signout: 'Afmelden',
    power: 'Aan/uit', sleep: 'Slaapstand', shutdown: 'Afsluiten', restart: 'Opnieuw opstarten',
    newApp: 'Onlangs toegevoegd', justNow: 'Zojuist', yesterdayAt: 'Gisteren om {t}', todayAt: 'Vandaag om {t}',
    recEmpty: 'Hoe meer je je computer gebruikt, hoe meer recente bestanden en nieuwe apps je hier ziet.',
    recOff: 'Zet aanbevelingen aan om hier je recente bestanden en nieuwe apps te zien.',
    recOffLink: 'Instellingen voor Start', noPinned: 'Maak apps vast door in Alle apps met de rechtermuisknop te klikken.',
    letterHint: 'Kies een letter', user: 'Gebruiker', clear: 'Wissen',
    f_settings: 'Instellingen', f_explorer: 'Verkenner', f_documents: 'Documenten', f_downloads: 'Downloads',
    f_music: 'Muziek', f_pictures: 'Afbeeldingen', f_videos: "Video's", f_network: 'Netwerk', f_personal: 'Persoonlijke map', f_home: 'Persoonlijke map',
  },
  en: {
    start: 'Start', searchPh: 'Search for apps, settings and documents', pinned: 'Pinned', allApps: 'All apps',
    back: 'Back', recommended: 'Recommended', more: 'More', page: 'Page {n}',
    unpinStart: 'Unpin from Start', pinStart: 'Pin to Start', pinTaskbar: 'Pin to taskbar',
    unpinTaskbar: 'Unpin from taskbar', moveFront: 'Move to front', open: 'Open',
    openLocation: 'Open file location', copyPath: 'Copy path', copied: 'Path copied',
    accountSettings: 'Change account settings', lock: 'Lock', signout: 'Sign out',
    power: 'Power', sleep: 'Sleep', shutdown: 'Shut down', restart: 'Restart',
    newApp: 'Recently added', justNow: 'Just now', yesterdayAt: 'Yesterday at {t}', todayAt: 'Today at {t}',
    recEmpty: 'The more you use your computer, the more recent files and new apps you will see here.',
    recOff: 'Turn on recommendations to see your recent files and new apps here.',
    recOffLink: 'Start settings', noPinned: 'Pin apps by right-clicking them in All apps.',
    letterHint: 'Choose a letter', user: 'User', clear: 'Clear',
    f_settings: 'Settings', f_explorer: 'File Explorer', f_documents: 'Documents', f_downloads: 'Downloads',
    f_music: 'Music', f_pictures: 'Pictures', f_videos: 'Videos', f_network: 'Network', f_personal: 'Personal folder', f_home: 'Personal folder',
  },
});

const COLS = 6;
const LAYOUTS = { default: [3, 3], morePins: [4, 2], moreRecommendations: [2, 4], 'more-pins': [4, 2], 'more-recommendations': [2, 4] };
const FOLDER_GLYPHS = {
  settings: 'settings', explorer: 'folder', documents: 'document', downloads: 'download', music: 'music',
  pictures: 'image', videos: 'video', personal: 'user', home: 'home',
};
const COLL_OPTS = { sensitivity: 'base', numeric: true };

/** Smart "when" label for recommended items (Win11 style). */
export function whenLabel(ms) {
  if (!ms) return '';
  const now = Date.now();
  const diff = now - ms;
  if (diff < 60e3) return t('justNow');
  if (diff < 3600e3) return relTime(ms);
  const d = new Date(ms), today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  if (sameDay) return diff < 6 * 3600e3 ? relTime(ms) : t('todayAt', { t: fmtTime(ms) });
  const y = new Date(today); y.setDate(today.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return t('yesterdayAt', { t: fmtTime(ms) });
  return fmtDate(ms, d.getFullYear() === today.getFullYear() ? { day: 'numeric', month: 'long' } : 'medium');
}

function letterOf(name) {
  const c = String(name || '').trim().normalize('NFD').replace(/[̀-ͯ]/g, '').charAt(0).toUpperCase();
  if (/[A-Z]/.test(c)) return c;
  if (/[0-9]/.test(c)) return '#';
  return '&';
}

export async function copyText(text) {
  try { await navigator.clipboard.writeText(text); return true; } catch { /* fall back */ }
  try {
    const ta = h('textarea', { style: { position: 'fixed', left: '-9999px', top: '0' } }, text);
    document.body.append(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  } catch { return false; }
}

export function setup(ctx) {
  let inst = null;
  ctx.registerFlyout('start', {
    slide: 40,
    create(container) {
      inst = createStart(ctx, container);
      return inst;
    },
  });
  const anchor = () => (getSetting('taskbar.alignment', 'center') === 'left' ? 'left' : 'center');
  ctx.onCommand('toggle-start', (args = {}) => {
    if (ctx.isOpen('start') && !args.view && args.query == null) { ctx.closeFlyout('start'); return; }
    ctx.openFlyout('start', { anchor: anchor(), view: args.view || 'home', query: args.query });
  });
  onSettings((s, path) => {
    if (ctx.isOpen('start') && (!path || path.startsWith('taskbar.alignment'))) ctx.openFlyout('start', { anchor: anchor() });
  });
}

function createStart(ctx, container) {
  let view = 'home';
  let page = 0;
  let searchPanel = null;
  let searchLoading = null;
  let recItems = [];
  let lastWheel = 0;
  let shown = false;
  let exitAnims = [];   // Start's own exit slide (playExit)
  let entranceClip = null;

  /* ------------------------------------------------------------ search box */
  const input = h('input', {
    class: 'sh-sbox__input', type: 'text', placeholder: t('searchPh'), spellcheck: false, autocomplete: 'off',
    'aria-label': t('searchPh'),
  });
  const clearBtn = h('button', { type: 'button', class: 'sh-sbox__clear', tabindex: -1, 'aria-label': t('clear'), hidden: true }, icon('close', { size: 12 }));
  const sbox = h('label', { class: 'sh-sbox' }, h('span', { class: 'sh-sbox__icon' }, icon('search', { size: 16 })), input, clearBtn);
  clearBtn.addEventListener('click', (e) => { e.preventDefault(); input.value = ''; onQuery(); input.focus(); });

  /* ---------------------------------------------------------------- home */
  const allBtn = pillButton(t('allApps'), 'chevron-right', true, () => showView('all'));
  const pinnedHead = h('div', { class: 'sh-start__head' }, h('h2', { class: 'sh-start__title' }, t('pinned')), allBtn);
  const pinPage = h('div', { class: 'sh-pinned__page', role: 'grid', 'aria-label': t('pinned') });
  const dots = h('div', { class: 'sh-pinned__dots', role: 'tablist' });
  const pinnedArea = h('div', { class: 'sh-pinned' }, pinPage, dots);
  const moreBtn = pillButton(t('more'), 'chevron-right', true, () => showView('rec'));
  const recHead = h('div', { class: 'sh-start__head' }, h('h2', { class: 'sh-start__title' }, t('recommended')), moreBtn);
  const recGrid = h('div', { class: 'sh-rec-grid' });
  const recSection = h('section', { class: 'sh-start__rec' }, recHead, recGrid);
  const home = h('div', { class: 'sh-start__view sh-start__home' },
    h('section', { class: 'sh-start__pinned' }, pinnedHead, pinnedArea), recSection);

  /* ------------------------------------------------------------- all apps */
  const allList = h('div', { class: 'sh-allapps__list', role: 'list' });
  const letters = h('div', { class: 'sh-letters', hidden: true, role: 'grid', 'aria-label': t('letterHint') });
  const allScroll = h('div', { class: 'sh-allapps' }, allList);
  const all = h('div', { class: 'sh-start__view sh-start__all', hidden: true },
    h('div', { class: 'sh-start__head' }, h('h2', { class: 'sh-start__title' }, t('allApps')),
      pillButton(t('back'), 'chevron-left', false, () => showView('home'))),
    h('div', { class: 'sh-allapps-wrap' }, allScroll, letters));

  /* ------------------------------------------------------ recommended "Meer" */
  const recAll = h('div', { class: 'sh-rec-grid sh-rec-grid--all' });
  const recMore = h('div', { class: 'sh-start__view sh-start__recmore', hidden: true },
    h('div', { class: 'sh-start__head' }, h('h2', { class: 'sh-start__title' }, t('recommended')),
      pillButton(t('back'), 'chevron-left', false, () => showView('home'))),
    h('div', { class: 'sh-recmore-scroll' }, recAll));

  /* --------------------------------------------------------------- search */
  const searchView = h('div', { class: 'sh-start__view sh-start__search', hidden: true });

  /* --------------------------------------------------------------- footer */
  const userBtn = h('button', { type: 'button', class: 'sh-start-user' });
  const footRight = h('div', { class: 'sh-start__foot-right' });
  const powerBtn = h('button', { type: 'button', class: 'sh-start-footbtn', 'aria-label': t('power'), dataset: { tooltip: t('power') } }, icon('power', { size: 16 }));
  const footer = h('div', { class: 'sh-start__footer' }, userBtn, footRight);

  const main = h('div', { class: 'sh-start__main' }, home, all, recMore, searchView);
  const panel = h('div', { class: 'sh-start sh-acrylic', role: 'dialog', 'aria-label': t('start') },
    h('div', { class: 'sh-start__top' }, sbox), main, footer);

  /* ============================================================ pinned grid */
  function rowsFor() {
    const lay = LAYOUTS[getSetting('start.layout', 'default')] || LAYOUTS.default;
    const showRec = getSetting('start.showRecommended', true) !== false;
    return { pin: lay[0], rec: lay[1], showRec };
  }
  function pinnedApps() {
    return ctx.pins.start.list().map((id) => ctx.apps.get(id)).filter(Boolean);
  }
  function renderPinned({ dir = 0 } = {}) {
    const { pin } = rowsFor();
    const per = COLS * pin;
    panel.style.setProperty('--pin-rows', pin);
    const apps = pinnedApps();
    const pages = Math.max(1, Math.ceil(apps.length / per));
    page = clamp(page, 0, pages - 1);
    const slice = apps.slice(page * per, page * per + per);
    pinPage.replaceChildren(...slice.map((app, i) => tile(app, page * per + i)));
    if (!apps.length && ctx.apps.list().length) pinPage.append(h('div', { class: 'sh-pinned__empty' }, t('noPinned')));
    dots.replaceChildren();
    dots.hidden = pages < 2;
    for (let i = 0; i < pages; i++) {
      const d = h('button', {
        type: 'button', class: ['sh-dot', i === page && 'is-active'], role: 'tab', tabindex: -1,
        'aria-label': t('page', { n: i + 1 }), 'aria-selected': String(i === page),
      });
      d.addEventListener('click', () => goPage(i));
      dots.append(d);
    }
    if (dir) animate(pinPage, [{ transform: `translateY(${dir * 32}px)`, opacity: 0 }, { transform: 'none', opacity: 1 }], { duration: 250 });
  }
  function goPage(i, focusFirst = false) {
    const { pin } = rowsFor();
    const pages = Math.max(1, Math.ceil(pinnedApps().length / (COLS * pin)));
    const next = clamp(i, 0, pages - 1);
    if (next === page) return false;
    const dir = next > page ? 1 : -1;
    page = next;
    renderPinned({ dir });
    if (focusFirst) pinPage.querySelector('.sh-tile')?.focus();
    return true;
  }
  pinnedArea.addEventListener('wheel', (e) => {
    if (Math.abs(e.deltaY) < 4) return;
    e.preventDefault();
    const now = performance.now();
    if (now - lastWheel < 350) return;
    lastWheel = now;
    goPage(page + (e.deltaY > 0 ? 1 : -1));
  }, { passive: false });

  function tile(app, index) {
    const b = h('button', {
      type: 'button', class: 'sh-tile', role: 'gridcell', tabindex: -1, dataset: { id: app.id, index },
      'aria-label': app.name, title: '',
    }, ctx.appIcon(app.icon, 32, { request: 48 }), h('span', { class: 'sh-tile__label' }, app.name));
    b.addEventListener('click', () => { if (!tileDrag?.moved) launch(app.id); });
    b.addEventListener('pointerenter', () => {
      const label = b.lastElementChild;
      if (label.scrollWidth > label.clientWidth + 1) b.dataset.tooltip = app.name; else delete b.dataset.tooltip;
    });
    b.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); tileMenu(app, b, e); });
    b.addEventListener('pointerdown', (e) => startTileDrag(e, b, app));
    return b;
  }
  function tileMenu(app, el, e) {
    const list = ctx.pins.start.list();
    const idx = list.indexOf(app.id);
    const onTb = ctx.pins.taskbar.has(app.id);
    const items = [];
    if (idx > 0) items.push({ label: t('moveFront'), icon: 'arrow-up', action: () => { page = 0; ctx.pins.start.moveToFront(app.id); } });
    items.push({ label: t('unpinStart'), icon: 'unpin', action: () => ctx.pins.start.remove(app.id) });
    items.push(onTb
      ? { label: t('unpinTaskbar'), icon: 'unpin', action: () => ctx.pins.taskbar.remove(app.id) }
      : { label: t('pinTaskbar'), icon: 'pin', action: () => ctx.pins.taskbar.add(app.id) });
    openAppMenu(items, app, el, e);
  }

  /* drag a tile onto another position */
  let tileDrag = null;
  function startTileDrag(e, el, app) {
    if (e.button !== 0 || e.pointerType === 'touch') return;
    tileDrag = { el, app, x0: e.clientX, y0: e.clientY, moved: false, ghost: null, target: -1, id: e.pointerId };
    const move = (ev) => {
      const d = tileDrag;
      if (!d || ev.pointerId !== d.id) return;
      const dx = ev.clientX - d.x0, dy = ev.clientY - d.y0;
      if (!d.moved) {
        if (Math.hypot(dx, dy) < 8) return;
        d.moved = true;
        hideTooltip();
        el.classList.add('is-dragging');
        try { el.setPointerCapture(d.id); } catch { /* ignore */ }
      }
      el.style.transform = `translate(${dx}px, ${dy}px) scale(1.04)`;
      const tiles = [...pinPage.querySelectorAll('.sh-tile')];
      let target = -1;
      for (const tl of tiles) {
        if (tl === el) continue;
        const r = tl.getBoundingClientRect();
        if (ev.clientX >= r.left && ev.clientX <= r.right && ev.clientY >= r.top && ev.clientY <= r.bottom) target = Number(tl.dataset.index);
      }
      tiles.forEach((tl) => tl.classList.toggle('is-drop-target', Number(tl.dataset.index) === target));
      d.target = target;
    };
    const up = () => {
      removeEventListener('pointermove', move, true);
      removeEventListener('pointerup', up, true);
      removeEventListener('pointercancel', up, true);
      const d = tileDrag;
      setTimeout(() => { tileDrag = null; }, 0);
      if (!d?.moved) return;
      el.classList.remove('is-dragging');
      el.style.transform = '';
      if (d.target >= 0) ctx.pins.start.move(d.app.id, d.target);
      else renderPinned();
    };
    addEventListener('pointermove', move, true);
    addEventListener('pointerup', up, true);
    addEventListener('pointercancel', up, true);
  }

  /* ============================================================ recommended */
  async function loadRecommended() {
    let files = [];
    try {
      const r = await bridge.call('apps.recent', { limit: 24 });
      files = Array.isArray(r) ? r : [];
    } catch { files = []; }
    const fresh = ctx.apps.recentlyInstalled(14).slice(0, 4);
    recItems = [
      ...fresh.map(({ app, time }) => ({ kind: 'app', key: `app:${app.id}`, title: app.name, sub: t('newApp'), time, app })),
      ...files.map((f) => ({ kind: 'file', key: `file:${f.path}`, title: f.name, sub: whenLabel(f.modified), time: f.modified, file: f })),
    ].sort((a, b) => (b.time || 0) - (a.time || 0));
    renderRecommended();
  }
  function recEl(item) {
    const ic = item.kind === 'app'
      ? ctx.appIcon(item.app.icon, 32, { request: 48 })
      : fileIcon(item.file, 32);
    const b = h('button', { type: 'button', class: 'sh-rec', tabindex: -1, dataset: { key: item.key } },
      ic, h('span', { class: 'sh-rec__text' }, h('span', { class: 'sh-rec__title' }, item.title), h('span', { class: 'sh-rec__sub' }, item.sub)));
    b.addEventListener('click', () => openRec(item));
    b.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); recMenu(item, b, e); });
    return b;
  }
  function fileIcon(f, size) {
    const wrap = h('span', { class: 'sh-appicon', style: { width: `${size}px`, height: `${size}px` } });
    const px = Math.round(size * Math.min(3, Math.max(1.5, devicePixelRatio || 1)));
    const url = f.icon ? bridge.iconUrl(f.icon, px) : bridge.mimeIconUrl(f.mime, px);
    const img = h('img', { src: url || bridge.mimeIconUrl(f.mime || 'text/plain', px), alt: '', width: size, height: size, draggable: false });
    img.addEventListener('error', () => {
      const alt = bridge.mimeIconUrl(f.mime || 'text/plain', px);
      if (img.src !== alt && alt) img.src = alt; else wrap.replaceChildren(icon('document', { size: Math.round(size * 0.8) }));
    }, { once: false });
    wrap.append(img);
    return wrap;
  }
  function openRec(item) {
    if (item.kind === 'app') launch(item.app.id);
    else {
      ctx.closeFlyout('start');
      bridge.call('files.open', { path: item.file.path }).catch((e) => ctx.toastError(item.title, e));
    }
  }
  function recMenu(item, el, e) {
    const items = item.kind === 'app'
      ? [{ label: t('open'), icon: 'open', action: () => openRec(item) },
        ctx.pins.start.has(item.app.id)
          ? { label: t('unpinStart'), icon: 'unpin', action: () => ctx.pins.start.remove(item.app.id) }
          : { label: t('pinStart'), icon: 'pin', action: () => ctx.pins.start.add(item.app.id) }]
      : [{ label: t('open'), icon: 'open', action: () => openRec(item) },
        { label: t('openLocation'), icon: 'folder-open', action: () => { ctx.closeFlyout('start'); ctx.openApp('explorer', { path: item.file.path.replace(/\/[^/]*$/, '') || '/', select: item.file.path }); } },
        { label: t('copyPath'), icon: 'copy', action: async () => { if (await copyText(item.file.path)) Toast.show({ title: t('copied'), severity: 'success', timeout: 2500 }); } }];
    if (item.kind === 'app') openAppMenu(items, item.app, el, e);
    else openMenu(items, el, e);
  }
  function renderRecommended() {
    const { rec, showRec } = rowsFor();
    panel.style.setProperty('--rec-rows', rec);
    recSection.classList.toggle('is-off', !showRec);
    moreBtn.hidden = !showRec || recItems.length <= rec * 2;
    if (!showRec) {
      recAll.replaceChildren();
      const link = h('button', { type: 'button', class: 'sh-linkbtn' }, t('recOffLink'));
      link.addEventListener('click', () => { ctx.closeFlyout('start'); ctx.openSettings(['start', 'personalization']); });
      recGrid.replaceChildren(h('div', { class: 'sh-rec-empty' }, h('span', null, t('recOff')), link));
      return;
    }
    if (!recItems.length) {
      recAll.replaceChildren();
      recGrid.replaceChildren(h('div', { class: 'sh-rec-empty' }, t('recEmpty')));
      return;
    }
    recGrid.replaceChildren(...recItems.slice(0, rec * 2).map(recEl));
    recAll.replaceChildren(...recItems.map(recEl));
  }

  /* ============================================================== all apps */
  function renderAll() {
    const coll = collator(COLL_OPTS);
    const apps = [...ctx.apps.list()].sort((a, b) => coll.compare(a.name, b.name));
    const groups = new Map();
    for (const a of apps) {
      const L = letterOf(a.name);
      if (!groups.has(L)) groups.set(L, []);
      groups.get(L).push(a);
    }
    const order = ['&', '#', ...'ABCDEFGHIJKLMNOPQRSTUVWXYZ'];
    const nodes = [];
    for (const L of order) {
      const list = groups.get(L);
      if (!list) continue;
      const head = h('button', { type: 'button', class: 'sh-letter', tabindex: -1, dataset: { letter: L }, 'aria-label': `${L} — ${t('letterHint')}` }, L);
      head.addEventListener('click', () => showLetters());
      nodes.push(head);
      for (const app of list) {
        const row = h('button', { type: 'button', class: 'sh-allapp', tabindex: -1, dataset: { id: app.id }, role: 'listitem' },
          ctx.appIcon(app.icon, 24, { request: 32 }), h('span', { class: 'sh-allapp__name' }, app.name));
        row.addEventListener('click', () => launch(app.id));
        row.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); allMenu(app, row, e); });
        nodes.push(row);
      }
    }
    allList.replaceChildren(...nodes);
    letters.replaceChildren(...order.map((L) => {
      const has = groups.has(L);
      const b = h('button', { type: 'button', class: 'sh-letters__btn', disabled: !has, tabindex: -1 }, L);
      b.addEventListener('click', () => jumpTo(L));
      return b;
    }));
  }
  function allMenu(app, el, e) {
    const onStart = ctx.pins.start.has(app.id);
    const onTb = ctx.pins.taskbar.has(app.id);
    openAppMenu([
      onStart ? { label: t('unpinStart'), icon: 'unpin', action: () => ctx.pins.start.remove(app.id) }
        : { label: t('pinStart'), icon: 'pin', action: () => ctx.pins.start.add(app.id) },
      onTb ? { label: t('unpinTaskbar'), icon: 'unpin', action: () => ctx.pins.taskbar.remove(app.id) }
        : { label: t('pinTaskbar'), icon: 'pin', action: () => ctx.pins.taskbar.add(app.id) },
    ], app, el, e);
  }
  function showLetters() {
    letters.hidden = false;
    allScroll.classList.add('is-dimmed');
    animate(letters, [{ opacity: 0, transform: 'scale(1.08)' }, { opacity: 1, transform: 'none' }], { duration: 200 });
    (letters.querySelector('button:not(:disabled)'))?.focus();
  }
  function hideLetters() {
    if (letters.hidden) return false;
    letters.hidden = true;
    allScroll.classList.remove('is-dimmed');
    return true;
  }
  function jumpTo(L) {
    hideLetters();
    const head = allList.querySelector(`.sh-letter[data-letter="${CSS.escape(L)}"]`);
    if (head) {
      allScroll.scrollTo({ top: head.offsetTop - 4, behavior: 'smooth' });
      head.focus({ preventScroll: true });
    }
  }

  /* ================================================================ views */
  function showView(next, { animateIt = true, focus = true } = {}) {
    if (next === view) return;
    const els = { home, all, rec: recMore, search: searchView };
    const from = els[view], to = els[next];
    const prevView = view;
    view = next;
    hideLetters();
    if (next === 'all') { renderAll(); allScroll.scrollTop = 0; }
    footer.hidden = next === 'search';
    panel.classList.toggle('is-searching', next === 'search');
    from.hidden = true;
    to.hidden = false;
    if (animateIt && next !== 'search' && prevView !== 'search') {
      const dir = next === 'home' ? -1 : 1;
      animate(to, [{ transform: `translateX(${dir * 48}px)`, opacity: 0 }, { transform: 'none', opacity: 1 }], { duration: 250 });
    } else if (animateIt) {
      animate(to, [{ opacity: 0 }, { opacity: 1 }], { duration: 167 });
    }
    if (focus && next !== 'search') {
      requestAnimationFrame(() => {
        const first = next === 'all' ? allList.querySelector('.sh-allapp') : next === 'rec' ? recAll.querySelector('.sh-rec') : null;
        if (first && document.activeElement !== input) first.focus({ preventScroll: true });
      });
    }
  }

  async function ensureSearch() {
    if (searchPanel) return searchPanel;
    if (!searchLoading) {
      searchLoading = import('./search.js').then((m) => {
        searchPanel = m.createSearchPanel(ctx, { input, embedded: true, onDone: () => ctx.closeFlyout('start') });
        searchView.replaceChildren(searchPanel.el);
        return searchPanel;
      }).catch((e) => { console.error('[start] search panel failed', e); searchLoading = null; return null; });
    }
    return searchLoading;
  }

  // an Enter typed while the search part is still loading (first search after login) or
  // while this query has not reached it yet is remembered and run once it has
  let queryPending = false;
  let pendingEnter = false;
  async function onQuery() {
    const q = input.value;
    clearBtn.hidden = !q;
    if (q.trim()) {
      queryPending = true;
      const sp = await ensureSearch();
      if (!sp) { queryPending = pendingEnter = false; return; }
      if (input.value !== q) return; // a newer keystroke runs its own onQuery
      if (view !== 'search') showView('search');
      sp.setQuery(q);
      queryPending = false;
      if (pendingEnter) { pendingEnter = false; sp.activate(); }
    } else {
      queryPending = pendingEnter = false;
      if (view === 'search') {
        searchPanel?.setQuery('');
        showView('home');
      }
    }
  }
  input.addEventListener('input', onQuery);

  /* ============================================================== actions */
  function launch(id) {
    ctx.closeFlyout('start');
    ctx.apps.launch(id);
  }
  function openMenu(items, el, e) {
    if (e && e.clientX) return showContextMenu({ x: e.clientX, y: e.clientY, items });
    return showContextMenu({ anchor: el, items, keyboard: true });
  }
  function openAppMenu(items, app, el, e) {
    const menu = openMenu([...items, {separator: true}, softwareRemovalPlaceholder()], el, e);
    populateSoftwareRemoval(menu, items, app, () => {
      ctx.closeFlyout('start');
      ctx.openApp('installer', {uninstall: app.id});
    });
  }

  /* ---- footer ---- */
  function renderFooter() {
    const u = ctx.user();
    const name = u.realName || u.name || t('user');
    userBtn.replaceChildren(Avatar({ name, image: u.avatar ? bridge.fileUrl(u.avatar) : undefined, size: 32 }), h('span', { class: 'sh-start-user__name' }, name));
    userBtn.setAttribute('aria-label', name);
    const folders = getSetting('start.folders', ['settings', 'explorer', 'documents', 'downloads']) || [];
    footRight.replaceChildren(...folders.filter((f) => FOLDER_GLYPHS[f]).map((f) => {
      const b = h('button', { type: 'button', class: 'sh-start-footbtn', 'aria-label': t(`f_${f}`), dataset: { tooltip: t(`f_${f}`) } }, icon(FOLDER_GLYPHS[f], { size: 16 }));
      b.addEventListener('click', () => openFolder(f));
      return b;
    }), powerBtn);
  }
  function openFolder(f) {
    ctx.closeFlyout('start');
    if (f === 'settings') { ctx.openApp('settings'); return; }
    if (f === 'explorer') { ctx.openApp('explorer'); return; }
    ctx.openApp('explorer', { place: f === 'personal' ? 'home' : f });
  }
  userBtn.addEventListener('click', () => {
    const u = ctx.user();
    showContextMenu({
      anchor: userBtn, placement: 'top', items: [
        { header: u.realName || u.name || t('user') },
        { label: t('accountSettings'), icon: 'user', action: () => { ctx.closeFlyout('start'); ctx.openSettings(['accounts']); } },
        { separator: true },
        { label: t('lock'), icon: 'lock', action: () => { ctx.closeFlyout('start'); ctx.runCommand('power', { action: 'lock' }); } },
        { label: t('signout'), icon: 'signout', action: () => { ctx.closeFlyout('start'); ctx.runCommand('power', { action: 'logout' }); } },
      ],
    });
  });
  powerBtn.addEventListener('click', () => {
    showContextMenu({
      anchor: powerBtn, placement: 'top', align: 'center', items: [
        { label: t('sleep'), icon: 'sleep', action: () => power('suspend') },
        { label: t('shutdown'), icon: 'power', action: () => power('shutdown') },
        { label: t('restart'), icon: 'restart', action: () => power('reboot') },
      ],
    });
  });
  function power(action) {
    ctx.closeFlyout('start');
    ctx.runCommand('power', { action });
  }

  /* ============================================================ keyboard */
  function gridMove(el, dx, dy) {
    const tiles = [...pinPage.querySelectorAll('.sh-tile')];
    const i = tiles.indexOf(el);
    if (i < 0) return false;
    let j = i + dx + dy * COLS;
    if (j >= tiles.length) {
      if (dy > 0 || dx > 0) {
        if (goPage(page + 1, true)) return true;
        if (dy > 0) { recGrid.querySelector('.sh-rec')?.focus(); return true; }
      }
      return true;
    }
    if (j < 0) {
      if (dy < 0 || dx < 0) {
        if (page > 0) { goPage(page - 1); const tl = [...pinPage.querySelectorAll('.sh-tile')]; tl[tl.length - 1]?.focus(); return true; }
        if (dy < 0) { input.focus(); return true; }
      }
      return true;
    }
    tiles[j]?.focus();
    return true;
  }
  function listMove(list, el, d) {
    const items = [...list].filter((x) => !x.hidden && !x.disabled);
    const i = items.indexOf(el);
    if (i < 0) return false;
    const j = i + d;
    if (j < 0) { input.focus(); return true; }
    items[clamp(j, 0, items.length - 1)]?.focus();
    return true;
  }
  panel.addEventListener('keydown', (e) => {
    const a = document.activeElement;
    if (e.defaultPrevented) return;
    if (e.key === 'Enter' && a === input && input.value.trim() && (queryPending || view !== 'search' || !searchPanel)) {
      e.preventDefault();
      pendingEnter = true;
      if (!queryPending) onQuery();
      return;
    }
    // search mode: the panel owns the arrows/enter
    if (view === 'search' && searchPanel && (a === input || searchView.contains(a))) {
      if (searchPanel.handleKey(e)) return;
    }
    if (a === input) {
      if (e.key === 'ArrowDown' && !input.value) {
        e.preventDefault();
        (view === 'home' ? pinPage.querySelector('.sh-tile') : view === 'all' ? allList.querySelector('.sh-allapp') : recAll.querySelector('.sh-rec'))?.focus();
      } else if (e.key === 'Enter' && !input.value && view === 'home') {
        e.preventDefault();
      }
      return;
    }
    if (e.key === 'Escape') {
      if (hideLetters()) { e.preventDefault(); e.stopPropagation(); allList.querySelector('.sh-letter')?.focus(); }
      return;
    }
    const k = e.key;
    if (a?.classList.contains('sh-tile')) {
      const map = { ArrowRight: [1, 0], ArrowLeft: [-1, 0], ArrowDown: [0, 1], ArrowUp: [0, -1] };
      if (map[k]) { e.preventDefault(); gridMove(a, ...map[k]); return; }
      if (k === 'PageDown') { e.preventDefault(); goPage(page + 1, true); return; }
      if (k === 'PageUp') { e.preventDefault(); goPage(page - 1, true); return; }
      if (k === 'ContextMenu' || (e.shiftKey && k === 'F10')) { e.preventDefault(); const app = ctx.apps.get(a.dataset.id); if (app) tileMenu(app, a); return; }
    }
    if (a?.classList.contains('sh-rec')) {
      const items = [...a.parentElement.querySelectorAll('.sh-rec')];
      const i = items.indexOf(a);
      const map = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 2, ArrowUp: -2 };
      if (map[k] != null) {
        e.preventDefault();
        const j = i + map[k];
        if (j < 0 && a.parentElement === recGrid) { const tl = pinPage.querySelectorAll('.sh-tile'); tl[tl.length - 1]?.focus(); return; }
        items[clamp(j, 0, items.length - 1)]?.focus();
        return;
      }
      if (k === 'ContextMenu' || (e.shiftKey && k === 'F10')) { e.preventDefault(); const it = recItems.find((x) => x.key === a.dataset.key); if (it) recMenu(it, a); return; }
    }
    if (a?.classList.contains('sh-allapp') || a?.classList.contains('sh-letter')) {
      if (k === 'ArrowDown' || k === 'ArrowUp') { e.preventDefault(); listMove(allList.querySelectorAll('.sh-allapp, .sh-letter'), a, k === 'ArrowDown' ? 1 : -1); return; }
      if ((k === 'ContextMenu' || (e.shiftKey && k === 'F10')) && a.classList.contains('sh-allapp')) { e.preventDefault(); const app = ctx.apps.get(a.dataset.id); if (app) allMenu(app, a); return; }
      if (k === 'Backspace') { e.preventDefault(); showView('home'); return; }
    }
    if (a?.classList.contains('sh-letters__btn')) {
      const btns = [...letters.querySelectorAll('.sh-letters__btn')];
      const i = btns.indexOf(a);
      const map = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 4, ArrowUp: -4 };
      if (map[k] != null) {
        e.preventDefault();
        let j = i + map[k];
        while (j >= 0 && j < btns.length && btns[j].disabled) j += Math.sign(map[k]);
        if (j >= 0 && j < btns.length) btns[j].focus();
        return;
      }
    }
    // typing anywhere starts a search (Win11)
    if (k.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && k !== ' ') {
      e.preventDefault();
      input.focus();
      input.value += k;
      input.dispatchEvent(new Event('input'));
    } else if (k === 'Backspace' && view === 'search') {
      input.focus();
    }
  });

  /* ============================================================ lifecycle */
  /* roving tabindex: one Tab stop per zone, arrows inside it (Win11) */
  function roving(zone, selector) {
    zone.addEventListener('focusin', (e) => {
      const el = e.target.closest?.(selector);
      if (!el || !zone.contains(el)) return;
      zone.querySelectorAll(selector).forEach((x) => { x.tabIndex = x === el ? 0 : -1; });
    });
    const ensure = () => {
      const items = [...zone.querySelectorAll(selector)];
      if (items.length && !items.some((x) => x.tabIndex === 0)) items[0].tabIndex = 0;
    };
    new MutationObserver(ensure).observe(zone, { childList: true, subtree: true });
    ensure();
  }
  roving(pinPage, '.sh-tile');
  roving(recGrid, '.sh-rec');
  roving(recAll, '.sh-rec');
  roving(allList, '.sh-allapp, .sh-letter');
  roving(letters, '.sh-letters__btn:not(:disabled)');

  function refreshAll() {
    renderPinned();
    renderRecommended();
    if (view === 'all') renderAll();
  }
  ctx.apps.onChange(() => { refreshAll(); if (ctx.isOpen('start')) loadRecommended(); });
  onSettings((s, path) => {
    if (!path || /^(start|taskbar\.pinned|privacy|language)/.test(path)) {
      renderPinned();
      renderRecommended();
      renderFooter();
      if (view === 'all') renderAll();
    }
  });
  onLangChange(() => {
    input.placeholder = t('searchPh');
    input.setAttribute('aria-label', t('searchPh'));
    clearBtn.setAttribute('aria-label', t('clear'));
    renderFooter();
    refreshAll();
  });

  refreshAll();
  renderFooter();
  container.append(panel);

  return {
    el: panel,
    onOpen(opts = {}) {
      const already = shown;
      shown = true;
      if (opts.view === 'all') showView('all', { animateIt: already, focus: false });
      else if (opts.view === 'rec') showView('rec', { animateIt: already, focus: false });
      if (opts.query) { input.value = opts.query; onQuery(); }
      if (already) return; // re-positioned only (e.g. alignment changed)
      cancelExit();
      playEntranceClip();
      renderFooter();
      loadRecommended();
      renderPinned();
      animateContent();
      requestAnimationFrame(() => input.focus({ preventScroll: true }));
      ensureSearch(); // load the search part now, so the first letters and Enter never wait for it
    },
    onClose() {
      shown = false;
      pendingEnter = false;
      searchPanel?.cancelPending?.();
      playExit();
      hideLetters();
      setTimeout(() => {
        if (ctx.isOpen('start')) return;
        input.value = '';
        clearBtn.hidden = true;
        searchPanel?.setQuery('');
        page = 0;
        if (view !== 'home') showView('home', { animateIt: false, focus: false });
      }, 200);
    },
  };

  /*
   * Windows 11 drops Start down behind the taskbar when it closes (a quick accelerating slide)
   * instead of fading it in place. The flyout host slides the container 20 px (Fluent accelerate)
   * and fades the glass linearly; closeFlyout() starts those right after onClose(), so the
   * animations made here a microtask later replace the host's fade of the glass: the glass slides
   * another 60 px, is clipped at the taskbar's top edge (as if it went behind it) and only fades in
   * the second half, so the slide is seen. Only the glass itself animates (transform, clip-path and
   * its own opacity keep the backdrop blur on); fill 'forwards' holds the end state until the host
   * hides the container, and the next open cancels it.
   */
  function cancelExit() {
    for (const a of exitAnims) { try { a?.cancel(); } catch { /* ignore */ } }
    exitAnims = [];
  }
  function playExit() {
    cancelEntranceClip();
    cancelExit();
    queueMicrotask(() => {
      if (ctx.isOpen('start')) return;
      // the host's linear fade of the glass (one fade only: Start's own, below)
      for (const a of panel.getAnimations?.() || []) {
        let fades = false;
        try { fades = (a.effect?.getKeyframes?.() || []).some((k) => 'opacity' in k); } catch { fades = false; }
        if (fades) { try { a.cancel(); } catch { /* ignore */ } }
      }
      const tb = document.querySelector('.sh-taskbar:not(.is-hidden)');
      const gap = (tb ? tb.getBoundingClientRect().top : innerHeight) - panel.getBoundingClientRect().bottom;
      const host = cubicBezier(0.7, 0, 1, 0.5);   // core/flyouts.js exit curve (container, 20 px)
      const own = cubicBezier(0.4, 0, 1, 1);      // accelerate
      const standalone = window.__win11shellHost?.standaloneStart === true;
      const frames = [];
      for (let i = 0; i <= 10; i++) {
        const o = i / 10;
        const d = own(o) * 60;
        const below = Math.round(host(o) * 20 + d - gap);   // px of the glass past the taskbar edge
        // The native Start surface ends at the menu edge. Round the glass's own
        // visible cut, so its animated clip cannot square off the lower corners.
        // The full desktop surface retains the original shadow/occlusion bounds.
        const clipPath = standalone
          ? `inset(0px 0px ${Math.max(0, below)}px 0px round var(--radius-overlay))`
          : `inset(-64px -64px ${below}px -64px)`;
        frames.push({ offset: o, transform: `translateY(${d.toFixed(1)}px)`, clipPath });
      }
      exitAnims = [
        animate(panel, frames, { duration: 167, easing: 'linear', fill: 'forwards' }),
        animate(panel, [{ opacity: 1 }, { opacity: 1, offset: 0.5 }, { opacity: 0 }], { duration: 167, easing: 'linear', fill: 'forwards' }),
      ].filter(Boolean);
    });
  }

  function cancelEntranceClip() {
    try { entranceClip?.cancel(); } catch { /* already removed */ }
    entranceClip = null;
  }

  function playEntranceClip() {
    cancelEntranceClip();
    if (window.__win11shellHost?.standaloneStart !== true) return;
    // The separate native viewport also cuts the rising glass during opening.
    // Match the host's 40px/250ms deceleration and round the visible glass edge.
    queueMicrotask(() => {
      if (!ctx.isOpen('start')) return;
      const animation = animate(panel, [
        { clipPath: 'inset(0px 0px 40px 0px round var(--radius-overlay))' },
        { clipPath: 'inset(0px 0px 0px 0px round var(--radius-overlay))' },
      ], { duration: 250, easing: 'cubic-bezier(0,0,0,1)', fill: 'forwards' });
      entranceClip = animation;
      animation?.finished.then(() => {
        if (entranceClip === animation) cancelEntranceClip();
      }, () => {});
    });
  }

  function animateContent() {
    for (const el of [panel.querySelector('.sh-start__top'), main, footer]) {
      animate(el, [{ opacity: 0, transform: 'translateY(10px)' }, { opacity: 1, transform: 'none' }], { duration: 300, delay: 30 });
    }
  }
}

/** CSS cubic-bezier(x1, y1, x2, y2) as a function of progress 0..1 (for sampled keyframes). */
function cubicBezier(x1, y1, x2, y2) {
  const bx = (s) => 3 * x1 * s * (1 - s) ** 2 + 3 * x2 * s * s * (1 - s) + s ** 3;
  const by = (s) => 3 * y1 * s * (1 - s) ** 2 + 3 * y2 * s * s * (1 - s) + s ** 3;
  return (t) => {
    let lo = 0, hi = 1, s = t;
    for (let i = 0; i < 24; i++) { s = (lo + hi) / 2; if (bx(s) < t) lo = s; else hi = s; }
    return by(s);
  };
}

function pillButton(label, chevron, after, onClick) {
  const b = h('button', { type: 'button', class: 'sh-pillbtn' },
    !after ? icon(chevron, { size: 12 }) : null, h('span', null, label), after ? icon(chevron, { size: 12 }) : null);
  b.addEventListener('click', onClick);
  return b;
}
