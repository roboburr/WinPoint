// Win11 Shell — toast pop-ups (bottom right, above the taskbar). Owned by the flyouts agent.
//
//   setup(ctx) shows a toast for every notifications.new — unless Niet storen is on
//   (critical notifications still show), settings.notifications.showToasts is false, or the
//   notification centre is open. Toasts: app icon + name, title, body, optional image, up
//   to 3 actions, × close; slide in from the right; auto-dismiss after 6 s unless hovered;
//   at most 3 on screen (older ones stay in the notification centre).
//
//   showToast({title, body, icon, appName, image, actions:[{label, onClick}], timeout})
//   → {close} — local toasts for shell modules (not stored in the notification centre).

import { h, loadCss, animate, fadeContent } from '../core/dom.js';
import { icon, startLogo, hasIcon } from '../core/icons.js';
import { makeT, onLangChange } from '../core/i18n.js';
import { Button, IconButton, AppIcon } from '../core/components.js';
import { getSetting } from '../core/theme.js';
import { bridge as coreBridge } from '../core/bridge.js';

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

const t = makeT({
  nl: { close: 'Sluiten', region: 'Meldingen', shell: 'Win11 Shell', notification: 'Melding' },
  en: { close: 'Close', region: 'Notifications', shell: 'Win11 Shell', notification: 'Notification' },
});

const MAX = 3;
const TIMEOUT = 6000;

let ctx = null;
let bridge = coreBridge;
let stack = null;
const live = []; // [{el, id, close}]

function ensureStack() {
  if (stack && stack.isConnected) return stack;
  const host = ctx?.root || document.getElementById('shell-layer') || document.body;
  stack = h('div', { class: 'fo-toasts', role: 'region', 'aria-label': t('region'), 'aria-live': 'polite' });
  host.append(stack);
  return stack;
}

export function setup(c) {
  ctx = c;
  bridge = c.bridge || coreBridge;
  loadCss('flyouts');
  bridge.on('notifications.new', (d) => {
    const n = d?.notification || d;
    if (!n || n.id == null) return;
    const critical = isCritical(n.urgency);
    if (n.source === 'gnome' && n.silent) return;
    if (getSetting('notifications.showToasts', true) === false) return;
    if (getSetting('notifications.dnd', false) && !critical) return;
    if (ctx.isOpen?.('notifications')) return;
    showNotification(n);
  });
  bridge.on('notifications.closed', (d) => {
    const entry = live.find((x) => x.id != null && x.id === d?.id);
    entry?.close('closed');
  });
  // the region is built once: its name follows a language switch
  onLangChange(() => { if (stack) stack.setAttribute('aria-label', t('region')); });
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

function appIconEl(name, appName, size = 16, glyph = null) {
  if (glyph) return h('span', { class: 'fl-app-icon fo-toast__glyph', style: { width: `${size}px`, height: `${size}px` } }, icon(glyph, { size }));
  const shell = /^win11 ?shell$/i.test(appName || '');
  if (name === 'win11shell' || (!name && shell)) {
    return h('span', { class: 'fl-app-icon', style: { width: `${size}px`, height: `${size}px` } }, startLogo(size));
  }
  return AppIcon({ name: name || String(appName || '').toLowerCase(), size, fallback: 'notification' });
}

function showNotification(n) {
  const hasDefault = hasDefaultAction(n);
  const invoke = (key) => bridge.call('notifications.invokeAction', { id: n.id, key }).catch((e) => console.warn('[toasts] action failed', e));
  return present({
    id: n.id,
    appName: n.appName,
    appIcon: n.appIcon,
    title: n.summary,
    body: n.body,
    image: n.image || null,
    critical: isCritical(n.urgency),
    actions: (n.actions || []).filter((a) => a.key !== 'default' && a.label).slice(0, 3)
      .map((a) => ({ label: a.label, onClick: () => invoke(a.key) })),
    onActivate: hasDefault ? () => invoke('default') : null,
  });
}

/** Local toast (not stored in the notification centre). */
export function showToast({ title = '', body = '', icon: ic, appName, image, actions = [], timeout = TIMEOUT, onActivate } = {}) {
  if (!ctx) {
    // not set up (e.g. used from another surface) — fall back to a console note
    console.info('[toasts] showToast before setup:', title, body);
  }
  const glyph = ic && hasIcon(ic) ? ic : null;
  return present({
    id: null, appName: appName || t('shell'), appIcon: glyph ? null : (ic || 'win11shell'), glyph, title, body,
    image: image || null, actions: actions.slice(0, 3), timeout, onActivate,
  });
}

function plain(s) {
  const str = String(s ?? '');
  if (!/[<&]/.test(str)) return str;
  const tmp = document.createElement('template');
  tmp.innerHTML = str.replace(/<br\s*\/?>/gi, '\n');
  return tmp.content.textContent || '';
}

/** FLIP: animate the other toasts from their old to their new position. */
function flip(before) {
  for (const [el, top] of before) {
    if (!el.isConnected) continue;
    const dy = top - el.getBoundingClientRect().top;
    if (Math.abs(dy) > 0.5) play(el, [{ transform: `translateY(${dy}px)` }, { transform: 'none' }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' });
  }
}
function positions() { return new Map(live.map((x) => [x.el, x.el.getBoundingClientRect().top])); }

function present({ id, appName: app, appIcon, glyph = null, title, body, image, actions = [], critical = false, timeout = TIMEOUT, onActivate }) {
  const appName = app || t('notification');   // no app name (the host stores "" for it, §2.4.4): our own word
  const s = ensureStack();
  // A server replacement keeps its notification ID. Keep exactly one visible card.
  if (id != null) {
    live.find((entry) => entry.id === id)?.close('replace');
    // A toast already fading out is no longer in live, but still has a DOM node.
    for (const node of s.querySelectorAll('[data-notification-id]'))
      if (node.dataset.notificationId === String(id)) node.remove();
  }
  const logoSlot = h('div', { class: 'fo-toast__logo', hidden: true });
  const heroSlot = h('div', { class: 'fo-toast__hero', hidden: true });
  if (image) notifImage(image, { logoSlot, heroSlot, cls: 'fo-toast__image' });
  const closeBtn = IconButton({ icon: 'close', label: t('close'), size: 28, iconSize: 10, tooltip: false, cls: 'fo-toast__close' });
  const el = h('div', {
    class: ['fo-toast', 'fo-acrylic', 'surface-flyout', onActivate && 'is-clickable', critical && 'is-critical'],
    role: 'status', 'data-region': '', tabindex: -1,
    'aria-label': [appName, title, body].filter(Boolean).join(', '),
  },
  h('div', { class: 'fo-toast__head' },
    appIconEl(appIcon, app, 16, glyph),
    h('span', { class: 'fo-toast__app caption ellipsis' }, appName),
    h('span', { class: 'grow' }),
    closeBtn),
  h('div', { class: 'fo-toast__content' },
    logoSlot,
    h('div', { class: 'fo-toast__text' },
      title ? h('div', { class: 'fo-toast__title body-strong' }, plain(title)) : null,
      body ? h('div', { class: 'fo-toast__body text-2' }, plain(body)) : null)),
  heroSlot,
  actions.length ? h('div', { class: 'fo-toast__actions' }, ...actions.map((a) => Button({
    label: a.label, onClick: (e) => { e.stopPropagation(); try { a.onClick?.(); } finally { close('action'); } },
  }))) : null);
  if (id != null) el.dataset.notificationId = String(id);

  let timer = 0, closed = false, hover = false;
  const arm = (ms) => { clearTimeout(timer); if (!critical && timeout) timer = setTimeout(() => { if (!hover) close('timeout'); }, ms); };

  function close(reason = 'close') {
    if (closed) return;
    closed = true;
    clearTimeout(timer);
    el.removeAttribute('data-region');
    const i = live.indexOf(entry);
    if (i >= 0) live.splice(i, 1);
    if (reason === 'replace') { el.remove(); return; }
    // exit: the glass may fade itself (its own opacity keeps the blur, it just blends out)
    play(el, [{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateX(48px)' }],
      { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' }).then(() => {
      const before = positions();
      el.remove();
      flip(before);
    });
    void reason;
  }
  const entry = { el, id, close };

  closeBtn.addEventListener('click', (e) => { e.stopPropagation(); close('user'); });
  el.addEventListener('click', () => { if (onActivate) { try { onActivate(); } finally { close('activate'); } } });
  el.addEventListener('pointerenter', () => { hover = true; clearTimeout(timer); });
  el.addEventListener('pointerleave', () => { hover = false; arm(3000); });
  el.addEventListener('focusin', () => { hover = true; clearTimeout(timer); });
  el.addEventListener('focusout', (e) => { if (!el.contains(e.relatedTarget)) { hover = false; arm(3000); } });
  el.addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.preventDefault(); close('user'); } });

  // too many on screen → the oldest leaves (it is still in the notification centre)
  while (live.length >= MAX) live[0].close('overflow');

  const before = positions();
  s.append(el);
  live.push(entry);
  flip(before);
  // frosted glass: slide only (opacity on the glass would switch its blur off); the content fades in
  play(el, [{ transform: 'translateX(calc(100% + 24px))' }, { transform: 'none' }],
    { duration: 333, easing: 'cubic-bezier(0,0,0,1)' });
  fadeContent(el, 'in', { duration: 250, easing: 'linear' });
  arm(timeout);
  return { close: () => close('api'), el };
}

export default { setup, showToast };
