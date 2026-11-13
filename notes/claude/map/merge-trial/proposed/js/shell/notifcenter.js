// Win11 Shell — Meldingencentrum + agenda (Win+N / klik op de klok). Owned by the flyouts agent.
//
//   setup(ctx) registers flyout 'notifications' (right side, above the taskbar) and the
//   commands 'notifications' (toggle) and 'calendar' (open with the calendar focused).
//   Two stacked Acrylic cards like Windows 11 23H2:
//     • Meldingen — header (Niet storen bell + "Alles wissen"), notifications grouped per app
//       (app icon + name, toast-style cards with actions, × per item), empty state.
//     • Agenda — "zaterdag 25 oktober" header (collapsible), month grid with prev/next month,
//       first weekday from settings.time.firstDayOfWeek, today in an accent circle.

import { h, clear, loadCss, animate } from '../core/dom.js';
import { icon, startLogo } from '../core/icons.js';
import { makeT, locale, lang, onLangChange } from '../core/i18n.js';
import { Button, IconButton, AppIcon } from '../core/components.js';
import { getSetting, setSetting, onSettings } from '../core/theme.js';
import { time as fmtTime } from '../core/format.js';
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
    title: 'Meldingen', clearAll: 'Alles wissen', empty: 'Geen nieuwe meldingen',
    unsupported: 'Meldingen van apps verschijnen hier zodra je de Win11 Shell-sessie gebruikt.',
    dnd: 'Niet storen', dndOn: 'Niet storen staat aan', dndOff: 'Niet storen staat uit',
    dndNote: 'Niet storen staat aan. Je ziet geen pop-ups; nieuwe meldingen komen hier binnen.',
    dismiss: 'Melding sluiten', dismissGroup: 'Alle meldingen van {app} wissen', notification: 'Melding',
    calendar: 'Agenda', collapse: 'Agenda samenvouwen', expand: 'Agenda uitvouwen',
    prevMonth: 'Vorige maand', nextMonth: 'Volgende maand', today: 'vandaag',
    failed: 'Dat is niet gelukt. Probeer het opnieuw.', more: 'Meer', yesterday: 'gisteren',
  },
  en: {
    title: 'Notifications', clearAll: 'Clear all', empty: 'No new notifications',
    unsupported: 'Notifications from apps appear here once you use the Win11 Shell session.',
    dnd: 'Do not disturb', dndOn: 'Do not disturb is on', dndOff: 'Do not disturb is off',
    dndNote: 'Do not disturb is on. You won’t see pop-ups; new notifications arrive here.',
    dismiss: 'Dismiss notification', dismissGroup: 'Clear all notifications from {app}', notification: 'Notification',
    calendar: 'Calendar', collapse: 'Collapse calendar', expand: 'Expand calendar',
    prevMonth: 'Previous month', nextMonth: 'Next month', today: 'today',
    failed: 'That didn’t work. Please try again.', more: 'More', yesterday: 'yesterday',
  },
});

let ctx = null;
let bridge = coreBridge;
let items = [];          // notifications (newest last as received)
let supported = true;
let ui = null;
const COLLAPSE_KEY = 'win11shell.calendarCollapsed';

/* ==========================================================================
   Setup
   ========================================================================== */

export function setup(c) {
  ctx = c;
  bridge = c.bridge || coreBridge;
  loadCss('flyouts');

  ctx.registerFlyout('notifications', {
    anchor: 'right',
    slide: 24,
    create: (container) => {
      host = container;
      container.classList.add('fo-host', 'fo-host--nc');
      container.addEventListener('keydown', (e) => {
        if ((e.target === container || e.target === ui?.el) && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
          e.preventDefault();
          (ui.list.querySelector('.fo-notif') || ui.cal.headerBtn)?.focus();
        }
      });
      ui = buildUI();
      renderList();
      renderCalendar();
      return { el: ui.el, onOpen, onClose };
    },
  });
  ctx.onCommand('notifications', (args = {}) => {
    // {calendar:true} (e.g. ?open=calendar) opens with the calendar focused instead of toggling
    if (args.calendar || args.focus === 'calendar') return ctx.openFlyout('notifications', { anchor: 'right', source: 'command', ...args, focus: 'calendar' });
    return ctx.toggleFlyout('notifications', { anchor: 'right', source: 'command', ...args });
  });
  ctx.onCommand('calendar', (args = {}) => ctx.openFlyout('notifications', { anchor: 'right', source: 'command', ...args, focus: 'calendar' }));

  bridge.on('notifications.new', (d) => {
    const n = d?.notification || d;
    if (!n || n.id == null) return;
    items = items.filter((x) => x.id !== n.id).concat([n]);
    if (ui) renderList({ added: n.id });
  });
  bridge.on('notifications.closed', (d) => {
    const id = d?.id;
    if (id == null || !items.some((x) => x.id === id)) return;
    items = items.filter((x) => x.id !== id);
    if (ui) removeCard(id);
  });
  onLangChange(() => {
    if (!ui || !host) return;
    ui = buildUI();
    host.replaceChildren(ui.el);
    renderList();
  });
  onSettings((s, path) => {
    if (!ui) return;
    if (!path || path.startsWith('notifications')) renderHeader();
    if (!path || path.startsWith('time')) { renderCalendar(); }
  });
  refresh();
}

async function refresh() {
  try {
    const r = await bridge.call('notifications.list');
    supported = r?.supported !== false;
    items = Array.isArray(r?.items) ? r.items : [];
  } catch (e) {
    supported = false;
    items = [];
  }
  if (ui) renderList();
}

/** Number of notifications (for a taskbar badge, if the shell wants it). */
export function notificationCount() { return items.length; }

/* ==========================================================================
   UI
   ========================================================================== */

function buildUI() {
  const dndBtn = IconButton({ icon: 'notification-off', label: t('dnd'), size: 32, iconSize: 16, onClick: toggleDnd, cls: 'fo-nc__dnd' });
  const clearBtn = Button({ label: t('clearAll'), variant: 'subtle', size: 'small', onClick: clearAll, cls: 'fo-nc__clear' });
  const header = h('div', { class: 'fo-nc__header' },
    h('div', { class: 'fo-nc__title body-strong' }, t('title')), h('span', { class: 'grow' }), dndBtn, clearBtn);
  const note = h('div', { class: 'fo-nc__note caption', hidden: true });
  const list = h('div', { class: 'fo-nc__list', role: 'region', 'aria-label': t('title') });
  const card = h('section', { class: 'fo-nc fo-acrylic surface-flyout', 'aria-label': t('title') }, header, note, list);

  const cal = buildCalendar();
  const el = h('div', { class: 'fo-nc-wrap' }, card, cal.el);
  return { el, card, header, dndBtn, clearBtn, note, list, cal };
}

function renderHeader() {
  const dnd = !!getSetting('notifications.dnd', false);
  ui.dndBtn.classList.toggle('is-on', dnd);
  ui.dndBtn.setAttribute('aria-pressed', String(dnd));
  ui.dndBtn.setLabel(dnd ? t('dndOn') : t('dndOff'));
  ui.note.hidden = !dnd;
  ui.note.replaceChildren(icon('notification-off', { size: 12 }), h('span', null, t('dndNote')));
  ui.clearBtn.hidden = items.length === 0;
}

function toggleDnd() {
  const dnd = !getSetting('notifications.dnd', false);
  setSetting('notifications.dnd', dnd).catch(() => {});
  renderHeader();
}

function appKey(n) { return String(n.appName || n.appIcon || '?').toLowerCase(); }

function appIconEl(n, size) {
  const name = n.appIcon || '';
  if (name === 'win11shell' || /^win11 ?shell$/i.test(n.appName || '')) {
    return h('span', { class: 'fl-app-icon', style: { width: `${size}px`, height: `${size}px` } }, startLogo(size));
  }
  return AppIcon({ name: name || String(n.appName || '').toLowerCase(), size, fallback: 'notification' });
}

/** Notification image → {src, logo}: data:/blob: URLs, file:// or absolute paths, or a themed icon name (never remote). */
function imageSource(img) {
  if (!img) return null;
  const s = String(img);
  if (/^(data:|blob:)/.test(s)) return { src: s, logo: false };
  if (/^[a-z][\w+.-]*:\/\//i.test(s) && !s.startsWith('file://')) return null;   // never fetch remote images (privacy)
  if (s.startsWith('file://')) { try { return { src: bridge.fileUrl(decodeURIComponent(new URL(s).pathname)), logo: false }; } catch { return null; } }
  if (s.startsWith('/')) return { src: bridge.fileUrl(s), logo: false };
  return /^[\w.+-]+$/.test(s) ? { src: bridge.iconUrl(s, 96), logo: true } : null;
}

/**
 * Win11 shows chat avatars / small pictures as a 48 px "app logo" left of the text and wide
 * pictures (screenshots, photos) as a hero image below it. Decide once the image has loaded.
 */
function notifImage(img, { logoSlot, heroSlot, cls }) {
  const source = imageSource(img);
  if (!source || !source.src) return;
  const el = h('img', { class: `${cls}`, src: source.src, alt: '', draggable: false, decoding: 'async' });
  el.addEventListener('error', () => el.remove(), { once: true });
  const place = () => {
    const w = el.naturalWidth || 0, hgt = el.naturalHeight || 1;
    const hero = !source.logo && w >= 160 && w / hgt >= 1.3;
    el.classList.add(hero ? 'is-hero' : 'is-logo');
    if (!hero && Math.abs(w / hgt - 1) < 0.15) el.classList.add('is-round');
    (hero ? heroSlot : logoSlot).append(el);
    (hero ? heroSlot : logoSlot).hidden = false;
  };
  if (el.complete && el.naturalWidth) place(); else el.addEventListener('load', place, { once: true });
}

const isCritical = (u) => u === 2 || u === '2' || String(u).toLowerCase() === 'critical';
const hasDefaultAction = (n) => !!n.hasDefault || (n.actions || []).some((a) => a.key === 'default');

/** Group by app, newest group first, newest notification first. */
function groups() {
  const map = new Map();
  for (const n of items) {
    const k = appKey(n);
    // no app name (the host stores "" for it, §2.4.4): our own word, in the language of the moment
    if (!map.has(k)) map.set(k, { key: k, appName: n.appName || t('notification'), sample: n, items: [], latest: 0 });
    const g = map.get(k);
    g.items.push(n);
    g.latest = Math.max(g.latest, Number(n.time) || 0);
  }
  const out = [...map.values()];
  out.forEach((g) => g.items.sort((a, b) => (Number(b.time) || 0) - (Number(a.time) || 0)));
  out.sort((a, b) => b.latest - a.latest);
  return out;
}

function renderList({ added } = {}) {
  if (!ui) return;
  renderHeader();
  const list = ui.list;
  const focusId = list.contains(document.activeElement) ? document.activeElement.closest('[data-id]')?.dataset.id : null;
  clear(list);
  if (!items.length) {
    list.append(h('div', { class: 'fo-nc__empty' },
      h('div', { class: 'text-2' }, t('empty')),
      !supported ? h('div', { class: 'caption text-3 fo-nc__empty-hint' }, t('unsupported')) : null));
    return;
  }
  for (const g of groups()) {
    const gEl = h('div', { class: 'fo-nc__group', role: 'list', 'aria-label': g.appName, dataset: { group: g.key } },
      h('div', { class: 'fo-nc__ghead' },
        appIconEl(g.sample, 16),
        h('span', { class: 'fo-nc__gname caption ellipsis' }, g.appName),
        h('span', { class: 'grow' }),
        IconButton({ icon: 'close', label: t('dismissGroup', { app: g.appName }), size: 24, iconSize: 10, cls: 'fo-nc__gclose', onClick: () => dismissGroup(g) })));
    for (const n of g.items) gEl.append(notifCard(n));
    list.append(gEl);
  }
  if (added != null) {
    const el = list.querySelector(`.fo-notif[data-id="${CSS.escape(String(added))}"]`);
    if (el) play(el, [{ opacity: 0, transform: 'translateY(-8px)' }, { opacity: 1, transform: 'none' }], { duration: 250 });
  }
  if (focusId) list.querySelector(`.fo-notif[data-id="${CSS.escape(focusId)}"]`)?.focus({ preventScroll: true });
}

function notifCard(n) {
  const hasDefault = hasDefaultAction(n);
  const actions = (n.actions || []).filter((a) => a.key !== 'default' && a.label).slice(0, 3);
  const logoSlot = h('div', { class: 'fo-notif__logo', hidden: true });
  const heroSlot = h('div', { class: 'fo-notif__hero', hidden: true });
  if (n.image) notifImage(n.image, { logoSlot, heroSlot, cls: 'fo-notif__image' });
  const close = IconButton({ icon: 'close', label: t('dismiss'), size: 24, iconSize: 10, cls: 'fo-notif__close', onClick: (e) => { e.stopPropagation(); dismiss(n.id); } });
  const card = h('div', {
    class: ['fo-notif', hasDefault && 'is-clickable', isCritical(n.urgency) && 'is-critical'], role: 'listitem', tabindex: 0,
    dataset: { id: String(n.id) }, 'aria-label': [n.appName || t('notification'), n.summary, n.body].filter(Boolean).join(', '),
  },
  h('div', { class: 'fo-notif__main' },
    logoSlot,
    h('div', { class: 'fo-notif__text' },
      h('div', { class: 'fo-notif__top' },
        h('div', { class: 'fo-notif__title body-strong' }, plain(n.summary || n.appName || t('notification'))),
        h('span', { class: 'fo-notif__time caption text-3', dataset: n.time ? { tooltip: new Date(Number(n.time)).toLocaleString(locale(), { dateStyle: 'full', timeStyle: 'short' }) } : {} }, stamp(n.time))),
      n.body ? h('div', { class: 'fo-notif__body text-2' }, plain(n.body)) : null)),
  close,
  heroSlot,
  actions.length ? h('div', { class: 'fo-notif__actions' }, ...actions.map((a) => Button({
    label: a.label, onClick: (e) => { e.stopPropagation(); invoke(n, a.key); },
  }))) : null);
  card.addEventListener('click', () => { if (hasDefault) invoke(n, 'default'); });
  card.addEventListener('keydown', (e) => {
    if (e.target !== card) return;
    if (e.key === 'Delete') { e.preventDefault(); const nx = card.nextElementSibling || card.previousElementSibling; dismiss(n.id); nx?.focus?.(); }
    if ((e.key === 'Enter' || e.key === ' ') && hasDefault) { e.preventDefault(); invoke(n, 'default'); }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      const all = [...ui.list.querySelectorAll('.fo-notif')];
      const i = all.indexOf(card);
      all[Math.max(0, Math.min(all.length - 1, i + (e.key === 'ArrowDown' ? 1 : -1)))]?.focus();
    }
  });
  return card;
}

/** Win11 style time stamp: "14:05" today, "gisteren", otherwise "za 24 okt". */
function stamp(ms) {
  const n = Number(ms);
  if (!n) return '';
  const d = new Date(n), now = new Date();
  const day = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(now) - day(d)) / 86400000);
  if (diff <= 0) return fmtTime(n, { h24: getSetting('time.h24', true) !== false });
  if (diff === 1) return t('yesterday');
  return new Intl.DateTimeFormat(locale(), { weekday: 'short', day: 'numeric', month: 'short' }).format(d).replace(/\./g, '');
}

/** Notification bodies may contain simple markup (<b>, <i>, <a>) — show plain text only. */
function plain(s) {
  const str = String(s ?? '');
  if (!/[<&]/.test(str)) return str;
  const tmp = document.createElement('template');
  tmp.innerHTML = str.replace(/<br\s*\/?>/gi, '\n');
  return tmp.content.textContent || '';
}

function removeCard(id) {
  const el = ui.list.querySelector(`.fo-notif[data-id="${CSS.escape(String(id))}"]:not([data-removing])`);
  if (!el) { if (!ui.list.querySelector('.fo-notif')) renderList(); return; }
  const group = el.closest('.fo-nc__group');
  const last = group && group.querySelectorAll('.fo-notif:not([data-removing])').length === 1;
  const target = last ? group : el;
  el.dataset.removing = '1';
  collapseOut(target, 48).then(() => {
    target.remove();
    if (group && group.isConnected && !group.querySelector('.fo-notif')) group.remove();
    if (!items.length) renderList();
    else renderHeader();
  });
}

/** Slide an element out to the right, then fold its height away. */
function collapseOut(target, dx) {
  const hgt = target.offsetHeight;
  return play(target, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: `translateX(${dx}px)` }], { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' }).then(() => {
    target.style.overflow = 'hidden';
    return play(target, [{ height: `${hgt}px` }, { height: '0px' }], { duration: 167, easing: 'cubic-bezier(0,0,0,1)', fill: 'forwards' });
  });
}

async function dismiss(id) {
  items = items.filter((x) => x.id !== id);
  removeCard(id);
  try { await bridge.call('notifications.dismiss', { id }); } catch (e) { console.warn('[notifcenter] dismiss failed', e); }
}

/** × on a group header: the whole group slides out at once. */
function dismissGroup(g) {
  const ids = new Set(g.items.map((n) => n.id));
  items = items.filter((x) => !ids.has(x.id));
  const el = ui.list.querySelector(`.fo-nc__group[data-group="${CSS.escape(g.key)}"]`);
  el?.querySelectorAll('.fo-notif').forEach((c) => { c.dataset.removing = '1'; });
  (el ? collapseOut(el, 64) : Promise.resolve()).then(() => {
    el?.remove();
    if (!items.length) renderList(); else renderHeader();
  });
  for (const id of ids) bridge.call('notifications.dismiss', { id }).catch((e) => console.warn('[notifcenter] dismiss failed', e));
}

async function clearAll() {
  const cards = [...ui.list.querySelectorAll('.fo-nc__group')];
  items = [];
  ui.clearBtn.hidden = true;
  await Promise.all(cards.map((c, i) => play(c, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateX(64px)' }],
    { duration: 200, delay: i * 40, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' })));
  renderList();
  try { await bridge.call('notifications.clear'); } catch (e) { console.warn('[notifcenter] clear failed', e); refresh(); }
}

async function invoke(n, key) {
  try {
    await bridge.call('notifications.invokeAction', { id: n.id, key });
  } catch (e) {
    console.warn('[notifcenter] action failed', e);
  }
  ctx.closeFlyout('notifications');
}

/* ==========================================================================
   Calendar
   ========================================================================== */

function readCollapsed() { try { return localStorage.getItem(COLLAPSE_KEY) === '1'; } catch { return false; } }
function writeCollapsed(v) { try { localStorage.setItem(COLLAPSE_KEY, v ? '1' : '0'); } catch { /* ignore */ } }

function firstDay() {
  const v = Number(getSetting('time.firstDayOfWeek', 1));
  return Number.isFinite(v) ? ((v % 7) + 7) % 7 : 1; // 0 = zondag, 1 = maandag, 6 = zaterdag (7 → 0)
}

function sameDay(a, b) { return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate(); }

function buildCalendar() {
  const state = { view: startOfMonth(new Date()), focus: new Date(), selected: null, collapsed: readCollapsed() };
  const headerText = h('span', { class: 'fo-cal__date body-strong' });
  const chevron = h('span', { class: 'fo-cal__chev' });
  const headerBtn = h('button', { type: 'button', class: 'fo-cal__header', onClick: () => setCollapsed(!state.collapsed) }, headerText, chevron);
  const monthLabel = h('div', { class: 'fo-cal__month body-strong', 'aria-live': 'polite' });
  const prev = IconButton({ icon: 'chevron-up', label: t('prevMonth'), size: 32, iconSize: 12, onClick: () => shiftMonth(-1) });
  const next = IconButton({ icon: 'chevron-down', label: t('nextMonth'), size: 32, iconSize: 12, onClick: () => shiftMonth(1) });
  const weekdays = h('div', { class: 'fo-cal__weekdays', 'aria-hidden': 'true' });
  const grid = h('div', { class: 'fo-cal__grid', role: 'grid' });
  const body = h('div', { class: 'fo-cal__body' },
    h('div', { class: 'fo-cal__monthrow' }, monthLabel, h('span', { class: 'grow' }), prev, next),
    weekdays, h('div', { class: 'fo-cal__gridwrap' }, grid));
  const el = h('section', { class: 'fo-cal fo-acrylic surface-flyout', 'aria-label': t('calendar') }, headerBtn, body);

  function startOfMonth(d) { return new Date(d.getFullYear(), d.getMonth(), 1); }

  function setCollapsed(v, animated = true) {
    state.collapsed = v;
    writeCollapsed(v);
    const from = el.offsetHeight;
    el.classList.toggle('is-collapsed', v);
    headerBtn.setAttribute('aria-expanded', String(!v));
    headerBtn.setAttribute('aria-label', `${headerText.textContent}. ${v ? t('expand') : t('collapse')}`);
    chevron.replaceChildren(icon(v ? 'chevron-up' : 'chevron-down', { size: 12 }));
    body.hidden = v;
    if (animated) {
      const to = el.offsetHeight;
      play(el, [{ height: `${from}px` }, { height: `${to}px` }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' });
      if (!v) play(body, [{ opacity: 0 }, { opacity: 1 }], { duration: 250, delay: 60, easing: 'linear', fill: 'backwards' });
    }
  }

  function shiftMonth(d, focusDay = false) {
    state.view = new Date(state.view.getFullYear(), state.view.getMonth() + d, 1);
    if (!focusDay) state.focus = new Date(state.view);
    render();
    play(grid, [{ opacity: 0, transform: `translateY(${d * 24}px)` }, { opacity: 1, transform: 'none' }], { duration: 250 });
  }

  function render() {
    const today = new Date();
    const loc = locale();
    headerText.textContent = new Intl.DateTimeFormat(loc, { weekday: 'long', day: 'numeric', month: 'long' }).format(today);
    headerBtn.setAttribute('aria-label', `${headerText.textContent}. ${state.collapsed ? t('expand') : t('collapse')}`);
    monthLabel.textContent = new Intl.DateTimeFormat(loc, { month: 'long', year: 'numeric' }).format(state.view);
    // weekday header (ma di wo do vr za zo / Mo Tu We …)
    const fd = firstDay();
    const wdFmt = new Intl.DateTimeFormat(loc, { weekday: 'short' });
    clear(weekdays);
    for (let i = 0; i < 7; i++) {
      const d = new Date(2026, 0, 4 + ((fd + i) % 7)); // 4 Jan 2026 = Sunday
      let s = wdFmt.format(d).replace('.', '');
      s = lang() === 'en' ? s.slice(0, 2) : s.slice(0, 2).toLowerCase();
      weekdays.append(h('span', null, s));
    }
    // 6×7 days
    clear(grid);
    const first = state.view;
    const offset = (first.getDay() - fd + 7) % 7;
    const start = new Date(first.getFullYear(), first.getMonth(), 1 - offset);
    const longFmt = new Intl.DateTimeFormat(loc, { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
    for (let r = 0; r < 6; r++) {
      const row = h('div', { class: 'fo-cal__row', role: 'row' });
      for (let c = 0; c < 7; c++) {
        const d = new Date(start.getFullYear(), start.getMonth(), start.getDate() + r * 7 + c);
        const inMonth = d.getMonth() === first.getMonth();
        const isToday = sameDay(d, today);
        const isFocus = sameDay(d, state.focus);
        const isSel = state.selected && sameDay(d, state.selected);
        const cell = h('button', {
          type: 'button', role: 'gridcell', tabindex: isFocus ? 0 : -1,
          class: ['fo-cal__day', !inMonth && 'is-other', isToday && 'is-today', isSel && 'is-selected'],
          'aria-label': longFmt.format(d) + (isToday ? `, ${t('today')}` : ''),
          'aria-selected': isSel ? 'true' : null, 'aria-current': isToday ? 'date' : null,
          dataset: { date: `${d.getFullYear()}-${d.getMonth()}-${d.getDate()}` },
          onClick: () => {
            state.selected = d; state.focus = d;
            if (!inMonth) { state.view = startOfMonth(d); }
            render();
            grid.querySelector('[tabindex="0"]')?.focus();
          },
        }, h('span', null, String(d.getDate())));
        row.append(cell);
      }
      grid.append(row);
    }
  }

  grid.addEventListener('keydown', (e) => {
    const moves = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -7, ArrowDown: 7 };
    let d = null;
    if (moves[e.key] != null) d = new Date(state.focus.getFullYear(), state.focus.getMonth(), state.focus.getDate() + moves[e.key]);
    else if (e.key === 'PageUp') d = new Date(state.focus.getFullYear(), state.focus.getMonth() - 1, Math.min(state.focus.getDate(), 28));
    else if (e.key === 'PageDown') d = new Date(state.focus.getFullYear(), state.focus.getMonth() + 1, Math.min(state.focus.getDate(), 28));
    else if (e.key === 'Home') d = new Date();
    if (!d) return;
    e.preventDefault();
    const monthDelta = (d.getFullYear() - state.view.getFullYear()) * 12 + d.getMonth() - state.view.getMonth();
    state.focus = d;
    if (monthDelta) shiftMonth(monthDelta, true); else render();
    grid.querySelector('[tabindex="0"]')?.focus();
  });

  setCollapsed(state.collapsed, false);
  render();

  return {
    el, render, headerBtn,
    reset() { state.view = startOfMonth(new Date()); state.focus = new Date(); state.selected = null; render(); },
    focus() {
      if (state.collapsed) setCollapsed(false);
      (grid.querySelector('[tabindex="0"]') || headerBtn).focus({ preventScroll: true });
    },
  };
}

function renderCalendar() { ui?.cal.render(); }

/* ==========================================================================
   Lifecycle
   ========================================================================== */

let tick = 0;
function onOpen(opts = {}) {
  settleHost(host);
  ui.cal.reset();
  renderList();
  refresh();
  clearInterval(tick);
  // keep relative times ("5 minuten geleden") and today's date fresh while open
  tick = setInterval(() => {
    for (const el of ui.list.querySelectorAll('.fo-notif')) {
      const n = items.find((x) => String(x.id) === el.dataset.id);
      const tEl = el.querySelector('.fo-notif__time');
      if (n && tEl && n.time) tEl.textContent = stamp(n.time);
    }
    if (!ui.cal.el.contains(document.activeElement)) ui.cal.render(); // date may have changed
  }, 60000);
  if (opts.focus === 'calendar') setTimeout(() => ui.cal.focus(), 0);
}

function onClose() {
  clearInterval(tick);
}

export default { setup };
