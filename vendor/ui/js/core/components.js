// Win11 Shell — Fluent (WinUI 3) controls (owned by ui-core). SPEC §4.
// Every factory returns a DOM element with a small API attached (el.setValue(), …).
// Styles: css/components.css (loaded by index.html). Keyboard accessible, light/dark.

import { h, clear, uid, clamp, animate, finished, focusables, fadeContent, cancelAll } from './dom.js';
import { icon } from './icons.js';
import { makeT, onLangChange } from './i18n.js';
import { bridge } from './bridge.js';
import { ACCENT_PRESETS } from './theme.js';

const t = makeT({
  nl: {
    on: 'Aan', off: 'Uit', ok: 'OK', cancel: 'Annuleren', close: 'Sluiten', search: 'Zoeken',
    clear: 'Wissen', back: 'Terug', reveal: 'Wachtwoord weergeven', expand: 'Uitvouwen',
    collapse: 'Samenvouwen', noResults: 'Geen resultaten', openNav: 'Navigatie openen',
    closeNav: 'Navigatie sluiten', select: 'Kies een optie', color: 'Kleur {c}', more: 'Meer',
    info: 'Informatie', success: 'Gelukt', warning: 'Waarschuwing', error: 'Fout',
    breadcrumb: 'Kruimelpad',
  },
  en: {
    on: 'On', off: 'Off', ok: 'OK', cancel: 'Cancel', close: 'Close', search: 'Search',
    clear: 'Clear', back: 'Back', reveal: 'Show password', expand: 'Expand',
    collapse: 'Collapse', noResults: 'No results', openNav: 'Open navigation',
    closeNav: 'Close navigation', select: 'Choose an option', color: 'Colour {c}', more: 'More',
    info: 'Information', success: 'Success', warning: 'Warning', error: 'Error',
    breadcrumb: 'Breadcrumb',
  },
});

/* ---- live language switch ------------------------------------------------------------------------------------
   The texts these controls take from the dictionary above (the clear, search and close buttons, the navigation
   toggle, default placeholders, On/Off, swatch names) follow a switch the way a freshly opened window would show
   them, also in apps that do not reload (F1 class, GE1a review M1). relabel(el, fn) holds el weakly; after a switch
   fn(el) runs for every such element that still exists, connected or not. Texts a caller passes in are the caller's
   to relabel. */
const RELABEL = Symbol('relabel');
const relabeled = new Set();
let sweepAt = 512;
const weakRef = (el) => (typeof WeakRef === 'function' ? new WeakRef(el) : { deref: () => el });
function relabel(el, fn) {
  if (!el) return el;
  if (!el[RELABEL]) { el[RELABEL] = []; relabeled.add(weakRef(el)); }
  el[RELABEL].push(fn);
  if (relabeled.size > sweepAt) {             // drop the entries of collected elements now and then
    for (const r of relabeled) if (!r.deref()) relabeled.delete(r);
    sweepAt = Math.max(512, relabeled.size * 2);
  }
  return el;
}
/** el's attribute = t(key, params), kept in the current language unless someone else has changed it since. */
function ownAttr(el, attr, key, params) {
  if (!el) return el;
  let last = t(key, params);
  el.setAttribute(attr, last);
  return relabel(el, (e) => {
    if (e.getAttribute(attr) !== last) return;
    last = t(key, params);
    e.setAttribute(attr, last);
  });
}
onLangChange(() => {
  for (const r of [...relabeled]) {
    const el = r.deref();
    if (!el) { relabeled.delete(r); continue; }
    for (const fn of el[RELABEL]) { try { fn(el); } catch (e) { console.error('[components] relabel failed', e); } }
  }
});

/* ==========================================================================
   Helpers
   ========================================================================== */

/** The data-theme in effect for an element (scoped containers win over <html>). */
export function themeScope(el) {
  return el?.closest?.('[data-theme]')?.getAttribute('data-theme') || document.documentElement.dataset.theme || 'dark';
}

/** Icon spec → element: 'name' | Element | {url, size} */
export function iconNode(spec, size = 16) {
  if (!spec) return null;
  if (spec instanceof Node) return spec;
  if (typeof spec === 'object' && spec.url) {
    return h('img', { class: 'fl-img-icon', src: spec.url, width: spec.size || size, height: spec.size || size, alt: '', draggable: false });
  }
  return icon(String(spec), { size });
}

function asNodes(v) {
  if (v == null || v === false) return [];
  if (Array.isArray(v)) return v.flatMap(asNodes);
  if (v instanceof Node) return [v];
  return [document.createTextNode(String(v))];
}

const popupStack = [];

/**
 * openPopup({anchor, content, placement:'bottom-start'|'bottom-end'|'top-start'|'right-start',
 *            offset, className, onClose, theme, position(pop, rect)}) → {el, close, reposition}
 * Light-dismiss popup in <body> (Acrylic, 8px radius). Used by Dropdown, SearchBox, ….
 */
export function openPopup({ anchor, content, placement = 'bottom-start', offset = 4, className = '',
  onClose, theme, position, animateIn = true, closeOnScroll = true } = {}) {
  const pop = h('div', {
    class: ['fl-popup', 'acrylic', 'surface-flyout', className],
    'data-popup': '', 'data-region': '', dataset: { theme: theme || themeScope(anchor) },
  }, content);
  document.body.appendChild(pop);
  let dir = 'down';

  function reposition() {
    const vw = innerWidth, vh = innerHeight, m = 8;
    const r = anchor ? anchor.getBoundingClientRect() : { left: vw / 2, right: vw / 2, top: vh / 2, bottom: vh / 2, width: 0, height: 0 };
    pop.style.maxHeight = '';
    if (position) { dir = position(pop, r) || 'down'; return; }
    const pw = pop.offsetWidth, ph = pop.offsetHeight;
    let left, top;
    if (placement.startsWith('right')) {
      left = r.right + offset; top = r.top;
      if (left + pw > vw - m) left = r.left - offset - pw;
    } else {
      left = placement.endsWith('end') ? r.right - pw : r.left;
      const below = r.bottom + offset, above = r.top - offset - ph;
      if (placement.startsWith('top')) { top = above >= m ? above : below; dir = above >= m ? 'up' : 'down'; }
      else { top = below + ph <= vh - m || above < m ? below : above; dir = top === below ? 'down' : 'up'; }
    }
    left = clamp(left, m, Math.max(m, vw - pw - m));
    if (top + ph > vh - m) { pop.style.maxHeight = `${Math.max(120, vh - m - top)}px`; }
    if (top < m) top = m;
    pop.style.left = `${Math.round(left)}px`;
    pop.style.top = `${Math.round(top)}px`;
  }
  reposition();

  // Frost-safe entrance: the Acrylic popup slides, its content fades (never the glass).
  let anims = [];
  if (animateIn) {
    anims = [
      animate(pop, [{ transform: `translateY(${dir === 'down' ? -6 : 6}px)` }, { transform: 'none' }],
        { duration: 167, easing: 'cubic-bezier(0,0,0,1)' }),
      ...fadeContent(pop, 'in', { duration: 133, easing: 'linear' }),
    ];
    finished(anims);
  }

  let closed = false;
  const onDown = (e) => {
    const target = e.target;
    if (pop.contains(target) || (anchor && anchor.contains(target))) return;
    // a popup opened from inside this one (e.g. nested dropdown) keeps us open
    const top = popupStack[popupStack.length - 1];
    if (top && top !== api && top.el.contains(target)) return;
    close('outside');
  };
  const onKey = (e) => {
    if (e.key === 'Escape' && popupStack[popupStack.length - 1] === api) {
      e.preventDefault(); e.stopPropagation();
      close('escape');
      anchor?.focus?.();
    }
  };
  const onBlur = () => close('blur');
  const vw0 = innerWidth, vh0 = innerHeight;
  const onResize = () => { if (innerWidth !== vw0 || innerHeight !== vh0) close('resize'); };
  const onScroll = (e) => { if (closeOnScroll && !pop.contains(e.target)) close('scroll'); };

  function close(reason = 'close') {
    if (closed) return;
    closed = true;
    document.removeEventListener('pointerdown', onDown, true);
    document.removeEventListener('keydown', onKey, true);
    removeEventListener('blur', onBlur);
    removeEventListener('resize', onResize);
    document.removeEventListener('scroll', onScroll, true);
    const i = popupStack.indexOf(api);
    if (i >= 0) popupStack.splice(i, 1);
    pop.removeAttribute('data-region');
    cancelAll(anims);
    finished(fadeContent(pop, 'out', { duration: 83, easing: 'linear', fill: 'forwards' })).then(() => pop.remove());
    onClose?.(reason);
  }

  const api = {
    el: pop, close, reposition, get closed() { return closed; },
    /** keep track of extra entrance animations (cancelled when the popup closes) */
    track(...a) { anims.push(...a.flat().filter(Boolean)); finished(a.flat().filter(Boolean)); },
  };
  popupStack.push(api);
  setTimeout(() => {
    if (closed) return;
    document.addEventListener('pointerdown', onDown, true);
    document.addEventListener('keydown', onKey, true);
    addEventListener('blur', onBlur);
    addEventListener('resize', onResize);
    document.addEventListener('scroll', onScroll, true);
  }, 0);
  return api;
}

/* ==========================================================================
   Buttons
   ========================================================================== */

/** Button({label, icon, variant:'standard'|'accent'|'subtle'|'danger', onClick, disabled, title, iconAfter}) */
export function Button({ label = '', icon: ic, variant = 'standard', onClick, disabled = false, title,
  iconAfter, type = 'button', cls = '', size } = {}) {
  const labelEl = h('span', { class: 'fl-btn__label' }, label);
  const btn = h('button', {
    type, class: ['fl-btn', `fl-btn--${variant}`, size && `fl-btn--${size}`, cls, !label && 'fl-btn--icon-only'],
    title, disabled: !!disabled,
  }, ic ? h('span', { class: 'fl-btn__icon' }, iconNode(ic, 16)) : null, label !== '' ? labelEl : null,
  iconAfter ? h('span', { class: 'fl-btn__icon fl-btn__icon--after' }, iconNode(iconAfter, 12)) : null);
  if (onClick) btn.addEventListener('click', (e) => { if (!btn.disabled) onClick(e, btn); });
  btn.setLabel = (s) => { labelEl.textContent = s; if (!labelEl.isConnected) btn.append(labelEl); };
  btn.setDisabled = (d) => { btn.disabled = !!d; };
  btn.setVariant = (v) => {
    btn.className = btn.className.replace(/fl-btn--(standard|accent|subtle|danger)/, `fl-btn--${v}`);
  };
  return btn;
}

/** IconButton({icon, label, onClick, variant='subtle', size=32, iconSize=16, disabled, tooltip=true}) */
export function IconButton({ icon: ic, label = '', onClick, variant = 'subtle', size = 32, iconSize = 16,
  disabled = false, tooltip = true, cls = '' } = {}) {
  const btn = h('button', {
    type: 'button', class: ['fl-btn', 'fl-icon-btn', `fl-btn--${variant}`, cls],
    'aria-label': label || null, disabled: !!disabled,
    style: { width: `${size}px`, height: `${size}px`, minHeight: `${size}px` },
    dataset: tooltip && label ? { tooltip: label } : {},
  }, iconNode(ic, iconSize));
  if (onClick) btn.addEventListener('click', (e) => { if (!btn.disabled) onClick(e, btn); });
  btn.setIcon = (name) => { clear(btn).append(iconNode(name, iconSize)); };
  btn.setDisabled = (d) => { btn.disabled = !!d; };
  btn.setLabel = (l) => { btn.setAttribute('aria-label', l); if (tooltip) btn.dataset.tooltip = l; };
  return btn;
}

/** HyperlinkButton({label, onClick, href, icon}) — accent text, subtle hover fill. */
export function HyperlinkButton({ label, onClick, href, icon: ic, external = false } = {}) {
  const el = h(href ? 'a' : 'button', {
    class: 'fl-link', type: href ? null : 'button', href: href || null,
    target: href && external ? '_blank' : null, rel: href && external ? 'noopener' : null,
  }, ic ? iconNode(ic, 16) : null, h('span', null, label), external ? iconNode('external', 12) : null);
  if (onClick) el.addEventListener('click', (e) => { if (!href) e.preventDefault(); onClick(e, el); });
  return el;
}

/** ToggleButton({label, icon, checked, onChange}) — like a Button that stays pressed (accent when on). */
export function ToggleButton({ label = '', icon: ic, checked = false, onChange, disabled = false } = {}) {
  const btn = Button({ label, icon: ic, variant: checked ? 'accent' : 'standard', disabled });
  btn.setAttribute('aria-pressed', String(!!checked));
  let state = !!checked;
  btn.setChecked = (v, { silent = true } = {}) => {
    state = !!v;
    btn.setAttribute('aria-pressed', String(state));
    btn.setVariant(state ? 'accent' : 'standard');
    if (!silent) onChange?.(state, btn);
  };
  btn.addEventListener('click', () => btn.setChecked(!state, { silent: false }));
  Object.defineProperty(btn, 'checked', { get: () => state, set: (v) => btn.setChecked(v) });
  return btn;
}

/* ==========================================================================
   ToggleSwitch — 40×20, knob 12 → 14 on hover → 17×14 while pressed
   ========================================================================== */

export function ToggleSwitch({ checked = false, onChange, label, stateText, disabled = false,
  onLabel, offLabel, ariaLabel } = {}) {
  const showState = stateText ?? !label;
  const stateEl = showState ? h('span', { class: 'fl-toggle__state' }) : null;
  const knob = h('span', { class: 'fl-toggle__knob' });
  const track = h('button', {
    type: 'button', class: 'fl-toggle__track', role: 'switch',
    'aria-label': ariaLabel || (typeof label === 'string' ? label : null) || null,
  }, knob);
  const labelEl = label ? h('span', { class: 'fl-toggle__label' }, label) : null;
  const root = h('div', { class: 'fl-toggle' }, stateEl, track, labelEl);
  let value = !!checked;

  function render() {
    root.classList.toggle('is-on', value);
    track.setAttribute('aria-checked', String(value));
    if (stateEl) stateEl.textContent = value ? (onLabel ?? t('on')) : (offLabel ?? t('off'));
  }
  if (stateEl && (onLabel === undefined || offLabel === undefined)) relabel(root, render);

  root.setChecked = (v, { silent = true } = {}) => {
    const nv = !!v;
    if (nv === value) { render(); return; }
    value = nv;
    render();
    if (!silent) onChange?.(value, root);
  };
  root.toggle = () => root.setChecked(!value, { silent: false });
  root.setDisabled = (d) => { track.disabled = !!d; root.classList.toggle('is-disabled', !!d); };
  Object.defineProperty(root, 'checked', { get: () => value, set: (v) => root.setChecked(v) });

  // click anywhere on the control (label/state text included)
  let suppressClick = false;
  root.addEventListener('click', (e) => {
    if (track.disabled) return;
    if (suppressClick) { suppressClick = false; return; }
    if (e.target !== track && !track.contains(e.target)) track.focus({ preventScroll: true });
    root.toggle();
  });

  // drag the knob
  let drag = null;
  track.addEventListener('pointerdown', (e) => {
    if (track.disabled || e.button !== 0) return;
    drag = { x: e.clientX, moved: false, start: value };
    track.setPointerCapture(e.pointerId);
    root.classList.add('is-pressed');
  });
  track.addEventListener('pointermove', (e) => {
    if (!drag) return;
    const dx = e.clientX - drag.x;
    if (Math.abs(dx) > 3) drag.moved = true;
    if (drag.moved) {
      const base = drag.start ? 19 : 2; // knob left while pressed (17 px wide)
      knob.style.left = `${clamp(base + dx, 2, 19)}px`;
      root.classList.add('is-dragging');
    }
  });
  const endDrag = (e) => {
    if (!drag) return;
    root.classList.remove('is-pressed', 'is-dragging');
    if (drag.moved) {
      const left = parseFloat(knob.style.left) || 0;
      knob.style.left = '';
      suppressClick = true;
      root.setChecked(left > 10.5, { silent: false });
      setTimeout(() => { suppressClick = false; }, 0);
    }
    drag = null;
    if (e?.pointerId != null && track.hasPointerCapture?.(e.pointerId)) track.releasePointerCapture(e.pointerId);
  };
  track.addEventListener('pointerup', endDrag);
  track.addEventListener('pointercancel', endDrag);

  render();
  root.setDisabled(disabled);
  return root;
}

/* ==========================================================================
   Slider — 4px rail, 20px thumb with accent inner dot, value tooltip
   ========================================================================== */

export function Slider({ min = 0, max = 100, step = 1, value = 0, onInput, onChange, disabled = false,
  format = (v) => String(v), width, ariaLabel, showTip = true } = {}) {
  const fill = h('div', { class: 'fl-slider__fill' });
  const rail = h('div', { class: 'fl-slider__rail' }, fill);
  const thumb = h('div', { class: 'fl-slider__thumb' });
  const tip = h('div', { class: 'fl-slider__tip', 'aria-hidden': 'true' });
  const root = h('div', {
    class: 'fl-slider', role: 'slider', tabindex: 0, 'aria-label': ariaLabel || null,
    'aria-valuemin': min, 'aria-valuemax': max,
    style: width ? { width: typeof width === 'number' ? `${width}px` : width } : null,
  }, rail, thumb, tip);
  let v = value;

  const snap = (x) => {
    const s = step > 0 ? Math.round((x - min) / step) * step + min : x;
    return clamp(Number(s.toFixed(6)), min, max);
  };
  function render() {
    const p = max > min ? (v - min) / (max - min) : 0;
    root.style.setProperty('--p', String(p));
    root.setAttribute('aria-valuenow', String(v));
    root.setAttribute('aria-valuetext', format(v));
    tip.textContent = format(v);
  }
  function set(nv, fire) {
    nv = snap(nv);
    if (nv === v) return false;
    v = nv; render();
    if (fire) onInput?.(v, root);
    return true;
  }
  root.setValue = (nv) => { v = snap(nv); render(); };
  root.getValue = () => v;
  Object.defineProperty(root, 'value', { get: () => v, set: (nv) => root.setValue(nv) });
  root.setDisabled = (d) => { root.classList.toggle('is-disabled', !!d); root.tabIndex = d ? -1 : 0; root.setAttribute('aria-disabled', String(!!d)); };
  root.setRange = (nmin, nmax) => { min = nmin; max = nmax; root.setAttribute('aria-valuemin', min); root.setAttribute('aria-valuemax', max); v = snap(v); render(); };

  const fromX = (clientX) => {
    const r = root.getBoundingClientRect();
    const p = clamp((clientX - r.left - 10) / Math.max(1, r.width - 20), 0, 1);
    return min + p * (max - min);
  };
  let dragging = false, startV = v;
  root.addEventListener('pointerdown', (e) => {
    if (root.classList.contains('is-disabled') || e.button !== 0) return;
    e.preventDefault();
    root.focus({ preventScroll: true });
    dragging = true; startV = v;
    root.setPointerCapture(e.pointerId);
    root.classList.add('is-dragging');
    if (showTip) root.classList.add('show-tip');
    set(fromX(e.clientX), true);
  });
  root.addEventListener('pointermove', (e) => { if (dragging) set(fromX(e.clientX), true); });
  const end = () => {
    if (!dragging) return;
    dragging = false;
    root.classList.remove('is-dragging', 'show-tip');
    if (v !== startV) onChange?.(v, root);
  };
  root.addEventListener('pointerup', end);
  root.addEventListener('pointercancel', end);
  let tipTimer = 0;
  root.addEventListener('keydown', (e) => {
    if (root.classList.contains('is-disabled')) return;
    const big = Math.max(step, (max - min) / 10);
    let nv = null;
    switch (e.key) {
      case 'ArrowLeft': case 'ArrowDown': nv = v - step; break;
      case 'ArrowRight': case 'ArrowUp': nv = v + step; break;
      case 'PageDown': nv = v - big; break;
      case 'PageUp': nv = v + big; break;
      case 'Home': nv = min; break;
      case 'End': nv = max; break;
      default: return;
    }
    e.preventDefault();
    if (set(nv, true)) onChange?.(v, root);
    if (showTip) {
      root.classList.add('show-tip');
      clearTimeout(tipTimer);
      tipTimer = setTimeout(() => root.classList.remove('show-tip'), 900);
    }
  });
  render();
  root.setDisabled(disabled);
  return root;
}

/* ==========================================================================
   Checkbox & RadioGroup
   ========================================================================== */

export function Checkbox({ checked = false, indeterminate = false, label = '', onChange, disabled = false, description } = {}) {
  const input = h('input', { type: 'checkbox', class: 'fl-sr', checked: !!checked, disabled: !!disabled });
  input.indeterminate = !!indeterminate;
  const box = h('span', { class: 'fl-check__box', 'aria-hidden': 'true' },
    h('svg', { viewBox: '0 0 12 12', class: 'fl-check__glyph' },
      h('path', { class: 'fl-check__mark', d: 'M2.25 6.4 4.75 8.9 9.75 3.6' }),
      h('path', { class: 'fl-check__dash', d: 'M3 6h6' })));
  const text = label || description ? h('span', { class: 'fl-check__text' },
    label ? h('span', { class: 'fl-check__label' }, label) : null,
    description ? h('span', { class: 'fl-check__desc' }, description) : null) : null;
  const root = h('label', { class: 'fl-check' }, input, box, text);
  input.addEventListener('change', () => onChange?.(input.checked, root));
  root.input = input;
  root.setChecked = (v) => { input.checked = !!v; input.indeterminate = false; };
  root.setIndeterminate = (v) => { input.indeterminate = !!v; };
  root.setDisabled = (d) => { input.disabled = !!d; };
  Object.defineProperty(root, 'checked', { get: () => input.checked, set: (v) => root.setChecked(v) });
  return root;
}

/** RadioGroup({options:[{value,label,description?,disabled?}], value, onChange, header, name, horizontal}) */
export function RadioGroup({ options = [], value, onChange, header, name = uid('radio'), disabled = false, horizontal = false } = {}) {
  const list = h('div', { class: ['fl-radio-group__list', horizontal && 'is-horizontal'] });
  const root = h('div', { class: 'fl-radio-group', role: 'radiogroup', 'aria-label': header || null },
    header ? h('div', { class: 'fl-radio-group__header' }, header) : null, list);
  let current = value;
  function build() {
    clear(list);
    for (const o of options) {
      const input = h('input', { type: 'radio', class: 'fl-sr', name, value: String(o.value), checked: o.value === current, disabled: !!(disabled || o.disabled) });
      input.addEventListener('change', () => { if (input.checked) { current = o.value; onChange?.(o.value, root); } });
      list.append(h('label', { class: 'fl-radio' }, input,
        h('span', { class: 'fl-radio__circle', 'aria-hidden': 'true' }),
        h('span', { class: 'fl-check__text' },
          h('span', { class: 'fl-check__label' }, o.label),
          o.description ? h('span', { class: 'fl-check__desc' }, o.description) : null)));
    }
  }
  build();
  root.setValue = (v) => {
    current = v;
    for (const i of list.querySelectorAll('input')) i.checked = i.value === String(v);
  };
  root.getValue = () => current;
  root.setOptions = (opts) => { options = opts; build(); };
  Object.defineProperty(root, 'value', { get: () => current, set: (v) => root.setValue(v) });
  return root;
}

/* ==========================================================================
   Dropdown (ComboBox) — custom Fluent popup, selected item overlays the box
   ========================================================================== */

export function Dropdown({ options = [], value, onChange, placeholder, disabled = false,
  width, header, ariaLabel } = {}) {
  const textEl = h('span', { class: 'fl-dropdown__text' });
  const chev = h('span', { class: 'fl-dropdown__chevron', 'aria-hidden': 'true' }, icon('chevron-down', { size: 12 }));
  const btn = h('button', {
    type: 'button', class: 'fl-dropdown', 'aria-haspopup': 'listbox', 'aria-expanded': 'false',
    'aria-label': ariaLabel || header || null, disabled: !!disabled,
    style: width ? { width: typeof width === 'number' ? `${width}px` : width } : null,
  }, textEl, chev);
  const root = header ? h('div', { class: 'fl-field' }, h('div', { class: 'fl-field__header' }, header), btn) : btn;
  let current = value;
  let popup = null;

  const optLabel = (o) => o?.label ?? String(o?.value ?? '');
  function render() {
    const o = options.find((x) => x.value === current);
    textEl.textContent = o ? optLabel(o) : (placeholder ?? t('select'));
    btn.classList.toggle('is-placeholder', !o);
  }
  function choose(v, fire = true) {
    if (v === current) return;
    current = v; render();
    if (fire) onChange?.(v, root);
  }

  function open() {
    if (popup || btn.disabled || !options.length) return;
    const ids = options.map(() => uid('opt'));
    const items = options.map((o, i) => h('div', {
      class: ['fl-list-item', 'fl-dropdown__item', o.value === current && 'is-selected', o.disabled && 'is-disabled'],
      role: 'option', id: ids[i], 'aria-selected': String(o.value === current), tabindex: -1,
      dataset: { index: i },
    }, o.icon ? iconNode(o.icon, 16) : null, h('span', { class: 'fl-list-item__text' }, optLabel(o),
      o.description ? h('span', { class: 'fl-list-item__desc' }, o.description) : null)));
    const list = h('div', { class: 'fl-dropdown__list', role: 'listbox' }, items);
    const selIdx = Math.max(0, options.findIndex((o) => o.value === current));
    btn.setAttribute('aria-expanded', 'true');
    btn.classList.add('is-open');
    popup = openPopup({
      anchor: btn, content: list, className: 'fl-dropdown__popup', animateIn: false,
      position(pop, r) {
        const vh = innerHeight, vw = innerWidth, m = 8;
        pop.style.minWidth = `${Math.round(r.width + 8)}px`;
        const ph = pop.offsetHeight, pw = pop.offsetWidth;
        const item = items[selIdx];
        const itemCenter = item ? item.offsetTop + item.offsetHeight / 2 : ph / 2;
        let top = r.top + r.height / 2 - itemCenter;
        top = clamp(top, m, Math.max(m, vh - ph - m));
        if (ph > vh - 2 * m) { top = m; pop.style.maxHeight = `${vh - 2 * m}px`; }
        pop.style.top = `${Math.round(top)}px`;
        pop.style.left = `${Math.round(clamp(r.left - 4, m, vw - pw - m))}px`;
        return 'down';
      },
      onClose() {
        popup = null;
        btn.setAttribute('aria-expanded', 'false');
        btn.classList.remove('is-open');
      },
    });
    const pop = popup.el;
    // reveal from the selected item (Win11 ComboBox open animation)
    const item = items[selIdx];
    if (item) {
      item.scrollIntoView({ block: 'nearest' });
      const top = item.offsetTop - pop.scrollTop, bottom = pop.offsetHeight - top - item.offsetHeight;
      // clip-path on the glass itself keeps the blur; the list fades (never the popup)
      popup.track(
        animate(pop, [{ clipPath: `inset(${Math.max(0, top)}px 0 ${Math.max(0, bottom)}px 0 round 8px)` },
          { clipPath: 'inset(0 0 0 0 round 8px)' }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' }),
        fadeContent(pop, 'in', { duration: 167, easing: 'linear', from: 0.4 }));
    }
    let active = selIdx;
    const focusItem = (i) => {
      active = clamp(i, 0, items.length - 1);
      let guard = 0;
      while (options[active]?.disabled && guard++ < items.length) active = (active + 1) % items.length;
      items[active]?.focus({ preventScroll: false });
    };
    focusItem(selIdx);
    list.addEventListener('click', (e) => {
      const it = e.target.closest('.fl-dropdown__item');
      if (!it || it.classList.contains('is-disabled')) return;
      choose(options[+it.dataset.index].value);
      popup?.close('select');
      btn.focus({ preventScroll: true });
    });
    list.addEventListener('pointermove', (e) => {
      const it = e.target.closest('.fl-dropdown__item');
      if (it && document.activeElement !== it) { active = +it.dataset.index; it.focus({ preventScroll: true }); }
    });
    let typed = '', typedT = 0;
    list.addEventListener('keydown', (e) => {
      if (e.key === 'ArrowDown') { e.preventDefault(); focusItem(active + 1); }
      else if (e.key === 'ArrowUp') { e.preventDefault(); focusItem(active - 1); }
      else if (e.key === 'Home') { e.preventDefault(); focusItem(0); }
      else if (e.key === 'End') { e.preventDefault(); focusItem(items.length - 1); }
      else if (e.key === 'Enter' || e.key === ' ') {
        e.preventDefault();
        if (!options[active]?.disabled) choose(options[active].value);
        popup?.close('select'); btn.focus({ preventScroll: true });
      } else if (e.key === 'Tab') { popup?.close('tab'); }
      else if (e.key.length === 1) {
        clearTimeout(typedT); typed += e.key.toLowerCase(); typedT = setTimeout(() => { typed = ''; }, 700);
        const i = options.findIndex((o) => optLabel(o).toLowerCase().startsWith(typed));
        if (i >= 0) focusItem(i);
      }
    });
  }

  btn.addEventListener('click', () => (popup ? popup.close('toggle') : open()));
  btn.addEventListener('keydown', (e) => {
    if (popup) return;
    const i = options.findIndex((o) => o.value === current);
    if (e.key === 'ArrowDown' && e.altKey) { e.preventDefault(); open(); }
    else if (e.key === 'F4') { e.preventDefault(); open(); }
    else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault();
      let j = i + (e.key === 'ArrowDown' ? 1 : -1);
      while (options[j]?.disabled) j += e.key === 'ArrowDown' ? 1 : -1;
      if (options[j]) choose(options[j].value);
    }
  });

  root.setValue = (v) => { current = v; render(); };
  root.getValue = () => current;
  root.setOptions = (opts, v = current) => { options = opts || []; current = v; render(); };
  root.setDisabled = (d) => { btn.disabled = !!d; };
  root.button = btn;
  Object.defineProperty(root, 'value', { get: () => current, set: (v) => root.setValue(v) });
  render();
  if (placeholder === undefined) relabel(btn, render);
  return root;
}

/* ==========================================================================
   TextBox & SearchBox
   ========================================================================== */

export function TextBox({ value = '', placeholder = '', header, description, type = 'text', onInput, onChange,
  onEnter, clearable = true, multiline = false, rows = 3, disabled = false, readOnly = false, maxLength,
  autofocus = false, width, name, autocomplete = 'off', spellcheck = false, ariaLabel, leading, trailing } = {}) {
  const input = h(multiline ? 'textarea' : 'input', {
    class: 'fl-textbox__input', type: multiline ? null : (type === 'password' ? 'password' : type),
    placeholder, value, disabled: !!disabled, readOnly: !!readOnly, name: name || null,
    maxLength: maxLength || null, rows: multiline ? rows : null, autocomplete, spellcheck,
    'aria-label': ariaLabel || header || placeholder || null,
  });
  if (multiline) input.value = value;
  const btns = h('div', { class: 'fl-textbox__buttons' });
  const field = h('div', { class: ['fl-textbox__field', multiline && 'is-multiline'] },
    leading ? h('span', { class: 'fl-textbox__leading' }, iconNode(leading, 16)) : null, input, btns);
  const descEl = h('div', { class: 'fl-textbox__desc' }, description || '');
  if (!description) descEl.hidden = true;
  const root = h('div', {
    class: ['fl-textbox', disabled && 'is-disabled'],
    style: width ? { width: typeof width === 'number' ? `${width}px` : width } : null,
  }, header ? h('label', { class: 'fl-field__header' }, header) : null, field, descEl);
  if (header) root.querySelector('label').addEventListener('click', () => input.focus());

  let clearBtn = null;
  if (clearable && !multiline && type !== 'password') {
    clearBtn = h('button', { type: 'button', class: 'fl-textbox__btn fl-textbox__clear', tabindex: -1, 'aria-label': t('clear') }, icon('close', { size: 12 }));
    ownAttr(clearBtn, 'aria-label', 'clear');
    clearBtn.addEventListener('pointerdown', (e) => e.preventDefault());
    clearBtn.addEventListener('click', () => { input.value = ''; sync(); input.focus(); onInput?.('', root); onChange?.('', root); });
    btns.append(clearBtn);
  }
  if (type === 'password') {
    const eye = h('button', { type: 'button', class: 'fl-textbox__btn', tabindex: -1, 'aria-label': t('reveal') }, icon('eye', { size: 14 }));
    ownAttr(eye, 'aria-label', 'reveal');
    const show = (s) => { input.type = s ? 'text' : 'password'; };
    eye.addEventListener('pointerdown', (e) => { e.preventDefault(); show(true); });
    for (const ev of ['pointerup', 'pointerleave', 'pointercancel']) eye.addEventListener(ev, () => show(false));
    btns.append(eye);
  }
  if (trailing) btns.append(trailing);

  function sync() { root.classList.toggle('has-text', !!input.value); }
  input.addEventListener('input', () => { sync(); onInput?.(input.value, root); });
  input.addEventListener('change', () => onChange?.(input.value, root));
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !multiline) onEnter?.(input.value, root, e);
  });
  input.addEventListener('focus', () => root.classList.add('is-focused'));
  input.addEventListener('blur', () => root.classList.remove('is-focused'));
  sync();
  if (autofocus) setTimeout(() => input.focus(), 0);

  root.input = input;
  root.field = field;
  root.focus = (o) => input.focus(o);
  root.select = () => input.select();
  root.setValue = (v) => { input.value = v ?? ''; sync(); };
  root.getValue = () => input.value;
  root.setDisabled = (d) => { input.disabled = !!d; root.classList.toggle('is-disabled', !!d); };
  root.setError = (msg) => {
    root.classList.toggle('has-error', !!msg);
    descEl.textContent = msg || description || '';
    descEl.hidden = !msg && !description;
  };
  Object.defineProperty(root, 'value', { get: () => input.value, set: (v) => root.setValue(v) });
  return root;
}

/**
 * SearchBox({placeholder, value, onInput, onSubmit, onSelect, suggest:(q)=>Promise<[{label, description, icon, value}]>, debounce})
 * AutoSuggestBox look: search glyph button on the right, suggestion list below.
 */
export function SearchBox({ placeholder, value = '', onInput, onSubmit, onSelect, suggest,
  debounce: wait = 150, width, ariaLabel } = {}) {
  const ownPlaceholder = placeholder === undefined;
  if (ownPlaceholder) placeholder = t('search');
  const searchBtn = h('button', { type: 'button', class: 'fl-textbox__btn fl-search__btn', tabindex: -1, 'aria-label': t('search') }, icon('search', { size: 14 }));
  ownAttr(searchBtn, 'aria-label', 'search');
  const box = TextBox({ value, placeholder, width, ariaLabel: ariaLabel || placeholder, trailing: searchBtn });
  if (ownPlaceholder) {
    ownAttr(box.input, 'placeholder', 'search');
    if (!ariaLabel) ownAttr(box.input, 'aria-label', 'search');
  }
  box.classList.add('fl-search');
  const input = box.input;
  input.setAttribute('role', 'combobox');
  input.setAttribute('aria-autocomplete', 'list');
  input.setAttribute('aria-expanded', 'false');
  let popup = null, items = [], results = [], active = -1, timer = 0, seq = 0;

  function closeList() { popup?.close('close'); popup = null; input.setAttribute('aria-expanded', 'false'); }
  function setActive(i) {
    active = i;
    items.forEach((it, j) => it.classList.toggle('is-active', j === i));
    if (items[i]) { input.setAttribute('aria-activedescendant', items[i].id); items[i].scrollIntoView({ block: 'nearest' }); }
    else input.removeAttribute('aria-activedescendant');
  }
  function pick(i) {
    const r = results[i];
    if (!r) return;
    closeList();
    onSelect?.(r, box);
  }
  function showList(list) {
    results = list || [];
    if (!results.length && !input.value) { closeList(); return; }
    const content = results.length
      ? results.map((r, i) => h('div', {
        class: 'fl-list-item fl-search__item', role: 'option', id: uid('sug'),
        onPointerdown: (e) => e.preventDefault(), onClick: () => pick(i),
      }, r.icon ? iconNode(r.icon, 16) : null, h('span', { class: 'fl-list-item__text' }, r.label,
        r.description ? h('span', { class: 'fl-list-item__desc' }, r.description) : null)))
      : [h('div', { class: 'fl-search__empty' }, t('noResults'))];
    items = results.length ? content : [];
    if (popup) { clear(popup.el.firstChild).append(...content); popup.reposition(); }
    else {
      const list = h('div', { class: 'fl-search__list', role: 'listbox' }, content);
      popup = openPopup({
        anchor: box.field, content: list, className: 'fl-search__popup', closeOnScroll: false,
        position(pop, r) {
          pop.style.width = `${Math.round(r.width)}px`;
          const ph = pop.offsetHeight;
          const below = r.bottom + 2;
          const top = below + ph > innerHeight - 8 ? Math.max(8, r.top - 2 - ph) : below;
          pop.style.left = `${Math.round(r.left)}px`;
          pop.style.top = `${Math.round(top)}px`;
          return top === below ? 'down' : 'up';
        },
        onClose: () => { popup = null; input.setAttribute('aria-expanded', 'false'); },
      });
      input.setAttribute('aria-expanded', 'true');
    }
    setActive(-1);
  }
  async function runSuggest(q) {
    if (!suggest) return;
    const my = ++seq;
    let list = [];
    try { list = await suggest(q); } catch (e) { console.warn('[SearchBox] suggest failed', e); }
    if (my === seq && document.activeElement === input) showList(list);
  }
  input.addEventListener('input', () => {
    onInput?.(input.value, box);
    clearTimeout(timer);
    if (!input.value) { closeList(); return; }
    timer = setTimeout(() => runSuggest(input.value), wait);
  });
  input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' && items.length) { e.preventDefault(); setActive(Math.min(items.length - 1, active + 1)); }
    else if (e.key === 'ArrowUp' && items.length) { e.preventDefault(); setActive(Math.max(-1, active - 1)); }
    else if (e.key === 'Enter') {
      e.preventDefault();
      if (active >= 0) pick(active);
      else { closeList(); onSubmit?.(input.value, box); }
    } else if (e.key === 'Escape' && popup) { e.preventDefault(); e.stopPropagation(); closeList(); }
  });
  input.addEventListener('blur', () => setTimeout(() => { if (document.activeElement !== input) closeList(); }, 120));
  searchBtn.addEventListener('pointerdown', (e) => e.preventDefault());
  searchBtn.addEventListener('click', () => { closeList(); onSubmit?.(input.value, box); input.focus(); });
  box.closeSuggestions = closeList;
  return box;
}

/* ==========================================================================
   Progress
   ========================================================================== */

export function ProgressBar({ value = 0, max = 100, indeterminate = false, paused = false, error = false, width, ariaLabel } = {}) {
  const bar = h('div', { class: 'fl-progress__bar' });
  const bar2 = h('div', { class: 'fl-progress__bar fl-progress__bar--2' });
  const root = h('div', {
    class: 'fl-progress', role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': max, 'aria-label': ariaLabel || null,
    style: width ? { width: typeof width === 'number' ? `${width}px` : width } : null,
  }, h('div', { class: 'fl-progress__track' }), bar, bar2);
  let v = value, ind = indeterminate;
  function render() {
    root.classList.toggle('fl-progress--indeterminate', ind);
    root.classList.toggle('fl-keep-motion', ind);
    root.style.setProperty('--p', String(clamp(v / (max || 1), 0, 1)));
    if (ind) root.removeAttribute('aria-valuenow'); else root.setAttribute('aria-valuenow', String(v));
  }
  root.setValue = (nv) => { v = nv; ind = false; render(); };
  root.setIndeterminate = (b) => { ind = !!b; render(); };
  root.setState = (s) => {
    root.classList.toggle('is-paused', s === 'paused');
    root.classList.toggle('is-error', s === 'error');
  };
  Object.defineProperty(root, 'value', { get: () => v, set: (nv) => root.setValue(nv) });
  root.setState(error ? 'error' : paused ? 'paused' : 'normal');
  render();
  return root;
}

export function ProgressRing({ size = 32, value, indeterminate, thickness, ariaLabel } = {}) {
  const ind = indeterminate ?? value == null;
  const sw = thickness ?? Math.max(2, Math.round(size / 10.5));
  const r = (size - sw) / 2;
  const circ = 2 * Math.PI * r;
  const arc = h('circle', { class: 'fl-ring__arc', cx: size / 2, cy: size / 2, r, 'stroke-width': sw, fill: 'none' });
  const track = h('circle', { class: 'fl-ring__track', cx: size / 2, cy: size / 2, r, 'stroke-width': sw, fill: 'none' });
  const svg = h('svg', { viewBox: `0 0 ${size} ${size}`, width: size, height: size }, track, arc);
  const root = h('span', {
    class: ['fl-ring', ind && 'fl-ring--indeterminate', ind && 'fl-keep-motion'], role: 'progressbar',
    'aria-label': ariaLabel || null, style: { width: `${size}px`, height: `${size}px`, '--circ': circ.toFixed(2) },
  }, svg);
  root.setValue = (nv) => {
    root.classList.remove('fl-ring--indeterminate', 'fl-keep-motion');
    const p = clamp(nv / 100, 0, 1);
    arc.style.strokeDasharray = `${(circ * p).toFixed(2)} ${circ.toFixed(2)}`;
    root.setAttribute('aria-valuenow', String(nv));
  };
  if (!ind) root.setValue(value);
  return root;
}

/* ==========================================================================
   InfoBar
   ========================================================================== */

const SEVERITY_GLYPH = {
  info: 'M8 7v4.25M8 4.85v.1',
  success: 'M5.1 8.2 7 10.1l3.9-4.2',
  warning: 'M8 4.5v4.25M8 11.1v.1',
  error: 'M5.75 5.75l4.5 4.5M10.25 5.75l-4.5 4.5',
};

export function severityIcon(severity = 'info', size = 16) {
  const sev = SEVERITY_GLYPH[severity] ? severity : 'info';
  return h('svg', { class: `fl-sev-icon fl-sev-icon--${sev}`, viewBox: '0 0 16 16', width: size, height: size, 'aria-hidden': 'true' },
    h('circle', { cx: 8, cy: 8, r: 8, class: 'fl-sev-icon__disc' }),
    h('path', { d: SEVERITY_GLYPH[sev], class: 'fl-sev-icon__glyph' }));
}

/** InfoBar({severity:'info'|'success'|'warning'|'error', title, message, action, closable, onClose}) */
export function InfoBar({ severity = 'info', title = '', message = '', action, closable = true, onClose, open = true } = {}) {
  const iconWrap = h('span', { class: 'fl-infobar__icon' }, severityIcon(severity));
  const titleEl = h('span', { class: 'fl-infobar__title' }, title);
  const msgEl = h('span', { class: 'fl-infobar__message' }, message);
  const actionWrap = h('span', { class: 'fl-infobar__action' });
  const setAction = (a) => {
    clear(actionWrap);
    if (!a) { actionWrap.hidden = true; return; }
    actionWrap.hidden = false;
    actionWrap.append(a instanceof Node ? a : Button({ label: a.label, onClick: a.onClick, variant: a.variant || 'standard' }));
  };
  const body = h('div', { class: 'fl-infobar__body' }, titleEl, msgEl, actionWrap);
  const closeBtn = closable ? IconButton({ icon: 'close', label: t('close'), size: 38, iconSize: 14, cls: 'fl-infobar__close', tooltip: false }) : null;
  ownAttr(closeBtn, 'aria-label', 'close');
  const root = h('div', {
    class: ['fl-infobar', `fl-infobar--${severity}`],
    role: severity === 'error' || severity === 'warning' ? 'alert' : 'status',
  }, iconWrap, body, closeBtn);
  closeBtn?.addEventListener('click', () => { root.close(); onClose?.(); });
  setAction(action);
  if (!title) titleEl.hidden = true;
  root.setTitle = (s) => { titleEl.textContent = s; titleEl.hidden = !s; };
  root.setMessage = (s) => { msgEl.textContent = s; };
  root.setSeverity = (s) => {
    root.className = root.className.replace(/fl-infobar--\w+/, `fl-infobar--${s}`);
    clear(iconWrap).append(severityIcon(s));
  };
  root.setAction = setAction;
  root.close = () => { root.hidden = true; };
  root.show = () => { root.hidden = false; };
  if (!open) root.hidden = true;
  return root;
}

/* ==========================================================================
   Settings cards (Windows 11 Instellingen rows)
   ========================================================================== */

function headerIcon(ic) {
  if (!ic) return null;
  return h('span', { class: 'fl-sc__icon', 'aria-hidden': 'true' }, iconNode(ic, 20));
}

/** SettingsCard({icon, title, description, control, onClick, chevron:true|'external'|false, disabled}) */
export function SettingsCard({ icon: ic, title = '', description = '', control, onClick, chevron, disabled = false, cls = '' } = {}) {
  const clickable = !!onClick;
  const titleEl = h('div', { class: 'fl-sc__title' }, title);
  const descEl = h('div', { class: 'fl-sc__desc' }, ...asNodes(description));
  if (!description) descEl.hidden = true;
  const ctl = h('div', { class: 'fl-sc__control' }, ...asNodes(control));
  if (!control) ctl.hidden = true;
  const chev = (chevron ?? clickable)
    ? h('span', { class: 'fl-sc__chevron', 'aria-hidden': 'true' }, icon(chevron === 'external' ? 'external' : 'chevron-right', { size: chevron === 'external' ? 14 : 12 }))
    : null;
  const root = h('div', {
    class: ['fl-settings-card', clickable && 'is-clickable', disabled && 'is-disabled', cls],
    role: clickable ? 'button' : null, tabindex: clickable ? 0 : null,
    'aria-disabled': disabled ? 'true' : null,
  }, h('div', { class: 'fl-sc__inner' },
    headerIcon(ic),
    h('div', { class: 'fl-sc__text' }, titleEl, descEl),
    h('div', { class: 'fl-sc__end' }, ctl, chev)));
  if (clickable) {
    root.addEventListener('click', (e) => {
      if (root.classList.contains('is-disabled')) return;
      if (e.target.closest('.fl-sc__control button, .fl-sc__control input, .fl-sc__control .fl-toggle, .fl-sc__control a')) return;
      onClick(e, root);
    });
    root.addEventListener('keydown', (e) => {
      if ((e.key === 'Enter' || e.key === ' ') && e.target === root) { e.preventDefault(); onClick(e, root); }
    });
  }
  root.setTitle = (s) => { titleEl.textContent = s; };
  root.setDescription = (s) => { clear(descEl).append(...asNodes(s)); descEl.hidden = !s; };
  root.setControl = (c) => { clear(ctl).append(...asNodes(c)); ctl.hidden = !c; };
  root.setDisabled = (d) => { root.classList.toggle('is-disabled', !!d); root.setAttribute('aria-disabled', String(!!d)); };
  return root;
}

function animateExpand(content, open) {
  if (open) {
    content.hidden = false;
    const hgt = content.scrollHeight;
    const a = animate(content, [{ height: '0px', opacity: 0 }, { height: `${hgt}px`, opacity: 1 }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' });
    const inner = content.firstElementChild;
    if (inner) animate(inner, [{ transform: 'translateY(-12px)' }, { transform: 'none' }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' });
    return finished(a);
  }
  const hgt = content.scrollHeight;
  const a = animate(content, [{ height: `${hgt}px`, opacity: 1 }, { height: '0px', opacity: 0 }], { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)' });
  return finished(a).then(() => { content.hidden = true; });
}

/** Expander({header, description, icon, content, expanded, onToggle, control}) — generic WinUI Expander. */
export function Expander({ header = '', description = '', icon: ic, content, expanded = false, onToggle, control, cls = '' } = {}) {
  const chevBtn = h('span', { class: 'fl-expander__chevron', 'aria-hidden': 'true' }, icon('chevron-down', { size: 12 }));
  const headId = uid('exp');
  const contentId = uid('expc');
  const head = h('div', {
    class: 'fl-expander__header', role: 'button', tabindex: 0, id: headId,
    'aria-expanded': String(!!expanded), 'aria-controls': contentId,
  }, h('div', { class: 'fl-sc__inner' },
    headerIcon(ic),
    h('div', { class: 'fl-sc__text' },
      h('div', { class: 'fl-sc__title' }, ...asNodes(header)),
      description ? h('div', { class: 'fl-sc__desc' }, ...asNodes(description)) : null),
    h('div', { class: 'fl-sc__end' }, control ? h('div', { class: 'fl-sc__control' }, ...asNodes(control)) : null, chevBtn)));
  const inner = h('div', { class: 'fl-expander__inner' }, ...asNodes(content));
  const body = h('div', { class: 'fl-expander__content', id: contentId, role: 'region', 'aria-labelledby': headId }, inner);
  const root = h('div', { class: ['fl-expander', expanded && 'is-expanded', cls] }, head, body);
  body.hidden = !expanded;
  let open = !!expanded;
  root.setExpanded = (v, { silent = true, animated = true } = {}) => {
    v = !!v;
    if (v === open) return;
    open = v;
    root.classList.toggle('is-expanded', open);
    head.setAttribute('aria-expanded', String(open));
    if (animated) animateExpand(body, open); else body.hidden = !open;
    if (!silent) onToggle?.(open, root);
  };
  root.toggle = () => root.setExpanded(!open, { silent: false });
  Object.defineProperty(root, 'expanded', { get: () => open, set: (v) => root.setExpanded(v) });
  head.addEventListener('click', (e) => {
    if (e.target.closest('.fl-sc__control button, .fl-sc__control input, .fl-sc__control .fl-toggle, .fl-sc__control a, .fl-sc__control .fl-dropdown')) return;
    root.toggle();
  });
  head.addEventListener('keydown', (e) => {
    if ((e.key === 'Enter' || e.key === ' ') && e.target === head) { e.preventDefault(); root.toggle(); }
  });
  root.content = inner;
  root.header = head;
  root.add = (...nodes) => inner.append(...nodes.flatMap(asNodes));
  return root;
}

/** SettingsExpander({icon, title, description, control, children, expanded, onToggle}) */
export function SettingsExpander({ icon: ic, title = '', description = '', control, children = [], expanded = false, onToggle } = {}) {
  return Expander({ header: title, description, icon: ic, control, content: children, expanded, onToggle, cls: 'fl-settings-expander' });
}

/** SettingsGroup({title, description, children}) — header + 4px-spaced cards */
export function SettingsGroup({ title = '', description = '', children = [] } = {}) {
  const items = h('div', { class: 'fl-settings-group__items' }, ...asNodes(children));
  const root = h('section', { class: 'fl-settings-group' },
    title ? h('h3', { class: 'fl-settings-group__title' }, title) : null,
    description ? h('p', { class: 'fl-settings-group__desc' }, description) : null,
    items);
  root.items = items;
  root.add = (...nodes) => items.append(...nodes.flatMap(asNodes));
  return root;
}

/* ==========================================================================
   Dialog (ContentDialog)
   ========================================================================== */

let dialogCount = 0;

function buildDialog({ title, content, primary, secondary, close, danger, icon: ic, width }) {
  const titleId = uid('dlg');
  const contentNode = typeof content === 'string'
    ? h('div', { class: 'fl-dialog__text' }, content)
    : (content instanceof Node ? content : h('div', null, ...asNodes(content)));
  const btns = [];
  const mk = (kind, label, variant) => {
    const b = Button({ label, variant });
    b.dataset.result = kind;
    btns.push(b);
    return b;
  };
  const closeLabel = close === false ? null : (close ?? (primary || secondary ? t('cancel') : t('close')));
  const footer = h('div', { class: 'fl-dialog__footer' },
    primary ? mk('primary', primary, danger ? 'danger' : 'accent') : null,
    secondary ? mk('secondary', secondary, 'standard') : null,
    closeLabel ? mk('close', closeLabel, primary || secondary ? 'standard' : 'accent') : null);
  const box = h('div', {
    class: 'fl-dialog acrylic', role: danger ? 'alertdialog' : 'dialog', 'aria-modal': 'true', 'aria-labelledby': title ? titleId : null,
    style: width ? { width: typeof width === 'number' ? `${width}px` : width } : null,
  },
  h('div', { class: 'fl-dialog__body' },
    title ? h('div', { class: 'fl-dialog__title', id: titleId },
      ic ? (SEVERITY_GLYPH[ic] ? severityIcon(ic, 20) : iconNode(ic, 20)) : null, h('span', null, title)) : null,
    h('div', { class: 'fl-dialog__content' }, contentNode)),
  footer);
  return { box, btns, footer };
}

/** Static dialog element (for previews/galleries; no behaviour). */
function dialogElement(opts = {}) {
  return buildDialog(opts).box;
}

/**
 * Dialog.show({title, content, primary, secondary, close, danger, icon, defaultButton, onPrimary, width, theme})
 * → Promise<'primary'|'secondary'|'close'>. Esc → 'close'. The promise has .dialog = {el, close(result), setBusy(b)}.
 * While onPrimary runs (or setBusy(true)) Esc, Enter and the buttons do nothing: a running format can't be
 * "cancelled" by closing its dialog. close() (programmatic) still works.
 */
function showDialog(opts = {}) {
  const { primary, secondary, danger, defaultButton, onPrimary, theme } = opts;
  const { box, btns } = buildDialog(opts);
  // No data-theme of its own unless asked for: the layer inherits <html>'s tokens, so a dialog that stays open
  // (e.g. "Een apparaat toevoegen") follows a light/dark switch like the page under it.
  const layer = h('div', {
    class: 'fl-dialog-layer', 'data-region': '', 'data-popup': '', dataset: { theme: theme || null },
  }, box);
  const previouslyFocused = document.activeElement;
  document.body.appendChild(layer);
  dialogCount++;
  document.documentElement.classList.add('has-dialog');

  let resolveFn;
  const promise = new Promise((r) => { resolveFn = r; });
  let done = false;
  let busy = false;

  async function finish(result, force = false) {
    if (done) return;
    if (busy && !force) return;
    if (!force && result === 'primary' && onPrimary) {
      api.setBusy(true);
      let ok = true;
      try { ok = (await onPrimary()) !== false; } catch (e) { console.error(e); ok = false; }
      if (done) return;                 // closed programmatically meanwhile
      api.setBusy(false);
      if (!ok) return;
    }
    done = true;
    document.removeEventListener('keydown', onKey, true);
    cancelAll(entrance);
    // The smoke fades by its colour (opacity on it would switch the dialog's blur off);
    // the glass scales and fades itself.
    const a1 = [
      animate(box, [{ transform: 'scale(1)' }, { transform: 'scale(1.05)' }], { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' }),
      ...fadeContent(box, 'out', { duration: 167, easing: 'linear', fill: 'forwards' }),
      animate(layer, [{ backgroundColor: smoke }, { backgroundColor: 'rgba(0, 0, 0, 0)' }], { duration: 167, easing: 'linear', fill: 'forwards' }),
    ];
    finished(a1).then(() => {
      layer.remove();
      dialogCount--;
      if (!dialogCount) document.documentElement.classList.remove('has-dialog');
      try { previouslyFocused?.focus?.({ preventScroll: true }); } catch { /* ignore */ }
    });
    resolveFn(result);
  }

  btns.forEach((b) => b.addEventListener('click', () => finish(b.dataset.result)));

  const def = defaultButton || (danger ? 'close' : primary ? 'primary' : secondary ? 'secondary' : 'close');
  function onKey(e) {
    const layers = document.querySelectorAll('.fl-dialog-layer');
    if (layers[layers.length - 1] !== layer) return;
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); if (!busy) finish('close'); return; }
    if (busy && e.key === 'Enter') { e.preventDefault(); e.stopPropagation(); return; }
    if (e.key === 'Enter' && !e.target.closest('textarea, .fl-dropdown, button')) {
      e.preventDefault();
      finish(def);
      return;
    }
    if (e.key === 'Tab') {
      const f = focusables(box);
      if (!f.length) return;
      const i = f.indexOf(document.activeElement);
      if (e.shiftKey && (i <= 0)) { e.preventDefault(); f[f.length - 1].focus(); }
      else if (!e.shiftKey && i === f.length - 1) { e.preventDefault(); f[0].focus(); }
    }
  }
  document.addEventListener('keydown', onKey, true);

  const smoke = getComputedStyle(layer).backgroundColor || 'rgba(0, 0, 0, 0.3)';
  const entrance = [
    animate(layer, [{ backgroundColor: 'rgba(0, 0, 0, 0)' }, { backgroundColor: smoke }], { duration: 167, easing: 'linear' }),
    animate(box, [{ transform: 'scale(1.05)' }, { transform: 'scale(1)' }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' }),
    ...fadeContent(box, 'in', { duration: 167, easing: 'linear' }),
  ];
  finished(entrance);

  setTimeout(() => {
    const input = box.querySelector('.fl-dialog__content input, .fl-dialog__content textarea, .fl-dialog__content [autofocus]');
    const target = input || btns.find((b) => b.dataset.result === def) || btns[0];
    target?.focus({ preventScroll: true });
  }, 0);

  const api = {
    el: box,
    close: (result = 'close') => finish(result, true),
    setBusy(b) {
      busy = !!b;
      box.classList.toggle('is-busy', busy);
      btns.forEach((x) => { x.disabled = busy; });
    },
  };
  promise.dialog = api;
  return promise;
}

export const Dialog = {
  show: showDialog,
  /** Dialog.confirm({title, message|content, confirmLabel, cancelLabel, danger, icon}) → Promise<boolean> */
  async confirm({ title, message, content, confirmLabel = t('ok'), cancelLabel = t('cancel'), danger = false, icon: ic } = {}) {
    const r = await showDialog({ title, content: content ?? message, primary: confirmLabel, close: cancelLabel, danger, icon: ic });
    return r === 'primary';
  },
  /** Dialog.alert({title, message, okLabel}) → Promise<void> */
  async alert({ title, message, content, okLabel = t('ok'), icon: ic } = {}) {
    await showDialog({ title, content: content ?? message, close: okLabel, icon: ic });
  },
  element: dialogElement,
};

/* ==========================================================================
   NavView — Win11 left navigation with the animated 3×16 accent pill
   ========================================================================== */

/**
 * NavView({items:[{id, label, icon, badge?, children?, disabled?, separator?, heading?}], selected,
 *          onSelect(id, item), header, footer (Element), footerItems, compact, paneToggle})
 */
export function NavView({ items = [], selected = null, onSelect, header, footer, footerItems = [], compact = false,
  paneToggle = false, ariaLabel } = {}) {
  const list = h('div', { class: 'fl-nav__list', role: 'tree', 'aria-label': ariaLabel || null });
  const footList = h('div', { class: 'fl-nav__list fl-nav__list--footer', role: 'tree' });
  const scroller = h('div', { class: 'fl-nav__scroll' }, list);
  const toggleBtn = paneToggle ? IconButton({ icon: 'menu', label: t('openNav'), size: 36, cls: 'fl-nav__toggle', tooltip: false }) : null;
  relabel(toggleBtn, (b) => b.setLabel(root.classList.contains('is-overlay-open') ? t('closeNav') : t('openNav')));
  const root = h('nav', { class: ['fl-nav', compact && 'is-compact'] },
    toggleBtn ? h('div', { class: 'fl-nav__toggle-row' }, toggleBtn) : null,
    header ? h('div', { class: 'fl-nav__header' }, header) : null,
    scroller,
    footer || footerItems.length ? h('div', { class: 'fl-nav__footer' }, footer || null, footList) : null);
  let sel = selected;
  const byId = new Map();
  const expanded = new Set();

  function itemNode(it, depth, parentId) {
    if (it.separator) return h('div', { class: 'fl-nav__sep', role: 'separator' });
    if (it.heading) return h('div', { class: 'fl-nav__heading' }, it.heading);
    const hasKids = Array.isArray(it.children) && it.children.length > 0;
    const chev = hasKids ? h('span', { class: 'fl-nav__chev', 'aria-hidden': 'true' }, icon('chevron-down', { size: 12 })) : null;
    const badge = h('span', { class: 'fl-nav__badge' });
    const el = h('div', {
      class: ['fl-nav__item', it.disabled && 'is-disabled', depth > 0 && 'is-child'],
      role: 'treeitem', tabindex: -1, dataset: { id: it.id, tooltip: compact ? it.label : null },
      'aria-selected': 'false', 'aria-level': depth + 1, 'aria-disabled': it.disabled ? 'true' : null,
      style: { '--depth': depth },
    },
    h('span', { class: 'fl-nav__icon' }, iconNode(it.icon, 16)),
    h('span', { class: 'fl-nav__label' }, it.label),
    badge, chev);
    byId.set(it.id, { it, el, badge, parentId, hasKids });
    if (it.badge != null) setBadgeEl(badge, it.badge);
    if (!hasKids) return el;
    const group = h('div', { class: 'fl-nav__group', role: 'group' }, it.children.map((c) => itemNode(c, depth + 1, it.id)));
    if (it.expanded) expanded.add(it.id);
    group.hidden = !expanded.has(it.id);
    el.setAttribute('aria-expanded', String(expanded.has(it.id)));
    byId.get(it.id).group = group;
    return [el, group];
  }

  function setBadgeEl(b, v) {
    clear(b);
    if (v == null || v === false || v === 0) { b.hidden = true; return; }
    b.hidden = false;
    b.append(Badge({ value: v === true ? '' : v }));
  }

  function build() {
    byId.clear();
    clear(list).append(...items.flatMap((it) => itemNode(it, 0, null)));
    clear(footList).append(...footerItems.flatMap((it) => itemNode(it, 0, null)));
    markSelected(false);
    rovingReset();
  }

  function visibleItems() {
    return [...root.querySelectorAll('.fl-nav__item')].filter((e) => e.offsetParent !== null && !e.classList.contains('is-disabled'));
  }
  function rovingReset() {
    const all = [...root.querySelectorAll('.fl-nav__item')];
    all.forEach((e) => { e.tabIndex = -1; });
    const target = byId.get(sel)?.el || all.find((e) => !e.classList.contains('is-disabled'));
    if (target) target.tabIndex = 0;
  }

  function pillRect(el) {
    const r = el.getBoundingClientRect();
    const rr = root.getBoundingClientRect();
    return { top: r.top - rr.top + r.height / 2 - 8, left: r.left - rr.left };
  }

  function markSelected(animated = true, prev = null) {
    let visible = byId.get(sel);
    // a collapsed parent shows the selection for its hidden child
    if (visible?.parentId && !expanded.has(visible.parentId)) visible = byId.get(visible.parentId);
    const oldEl = prev && root.querySelector('.fl-nav__item.is-selected');
    for (const { el } of byId.values()) {
      el.classList.remove('is-selected');
      el.setAttribute('aria-selected', 'false');
    }
    if (!visible) return;
    visible.el.classList.add('is-selected');
    byId.get(sel)?.el.setAttribute('aria-selected', 'true');
    if (animated && oldEl && oldEl !== visible.el && oldEl.offsetParent && visible.el.offsetParent) {
      animatePill(oldEl, visible.el);
    }
  }

  let pillAnim = null;
  function animatePill(fromEl, toEl) {
    const a = pillRect(fromEl), b = pillRect(toEl);
    const pill = h('span', { class: 'fl-nav__pill' });
    root.appendChild(pill);
    root.classList.add('is-pill-moving');
    pillAnim?.cancel();
    const down = b.top > a.top;
    const dist = Math.abs(b.top - a.top);
    const frames = [
      { transform: `translate(${a.left}px, ${a.top}px)`, height: '16px' },
      { transform: `translate(${down ? a.left : b.left}px, ${down ? a.top : b.top}px)`, height: `${dist + 16}px`, offset: 0.5 },
      { transform: `translate(${b.left}px, ${b.top}px)`, height: '16px' },
    ];
    pillAnim = animate(pill, frames, { duration: 400, easing: 'cubic-bezier(0.55,0.55,0,1)' });
    finished(pillAnim).then(() => {
      pill.remove();
      if (!root.querySelector('.fl-nav__pill')) root.classList.remove('is-pill-moving');
    });
  }

  function setExpanded(id, open) {
    const n = byId.get(id);
    if (!n?.group) return;
    if (open) expanded.add(id); else expanded.delete(id);
    n.el.setAttribute('aria-expanded', String(open));
    n.group.hidden = !open;
    markSelected(false);
  }

  function userSelect(id) {
    const n = byId.get(id);
    if (!n || n.it.disabled) return;
    if (n.hasKids && (n.it.selectable === false)) { setExpanded(id, !expanded.has(id)); return; }
    if (n.hasKids && !expanded.has(id)) setExpanded(id, true);
    if (id === sel) return;
    const prev = sel;
    sel = id;
    markSelected(true, prev);
    rovingReset();
    if (root.classList.contains('is-overlay-open')) setOverlay(false);
    onSelect?.(id, n.it);
  }

  root.addEventListener('click', (e) => {
    const chev = e.target.closest('.fl-nav__chev');
    const item = e.target.closest('.fl-nav__item');
    if (!item || !root.contains(item)) return;
    if (chev) { const id = item.dataset.id; setExpanded(id, !expanded.has(id)); return; }
    userSelect(item.dataset.id);
  });
  root.addEventListener('keydown', (e) => {
    const item = e.target.closest?.('.fl-nav__item');
    if (!item) return;
    const vis = visibleItems();
    const i = vis.indexOf(item);
    const id = item.dataset.id;
    const n = byId.get(id);
    const focus = (el) => { if (!el) return; vis.forEach((x) => { x.tabIndex = -1; }); el.tabIndex = 0; el.focus(); };
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focus(vis[i + 1]); break;
      case 'ArrowUp': e.preventDefault(); focus(vis[i - 1]); break;
      case 'Home': e.preventDefault(); focus(vis[0]); break;
      case 'End': e.preventDefault(); focus(vis[vis.length - 1]); break;
      case 'ArrowRight': if (n?.hasKids) { e.preventDefault(); if (!expanded.has(id)) setExpanded(id, true); else focus(n.group.querySelector('.fl-nav__item')); } break;
      case 'ArrowLeft':
        if (n?.hasKids && expanded.has(id)) { e.preventDefault(); setExpanded(id, false); }
        else if (n?.parentId) { e.preventDefault(); focus(byId.get(n.parentId).el); }
        break;
      case 'Enter': case ' ': e.preventDefault(); userSelect(id); break;
      default:
    }
  });

  function setOverlay(open) {
    root.classList.toggle('is-overlay-open', open);
    root.classList.toggle('acrylic', open);
    toggleBtn?.setLabel(open ? t('closeNav') : t('openNav'));
  }
  toggleBtn?.addEventListener('click', () => {
    if (root.classList.contains('is-compact')) setOverlay(!root.classList.contains('is-overlay-open'));
    else root.setCompact(true);
  });

  root.select = (id, { notify = false, animated = true } = {}) => {
    if (!byId.has(id)) { sel = id; markSelected(false); return; }
    const n = byId.get(id);
    if (n.parentId && !expanded.has(n.parentId)) setExpanded(n.parentId, true);
    const prev = sel;
    sel = id;
    markSelected(animated && prev != null, prev);
    rovingReset();
    if (notify) onSelect?.(id, n.it);
  };
  root.getSelected = () => sel;
  Object.defineProperty(root, 'selected', { get: () => sel });
  root.setItems = (newItems, newFooter = footerItems) => { items = newItems; footerItems = newFooter; build(); };
  /** The tree's accessible name (createAppFrame passes the app title, and setTitle keeps them equal). */
  root.setAriaLabel = (s) => { if (s) list.setAttribute('aria-label', s); else list.removeAttribute('aria-label'); };
  root.setBadge = (id, v) => { const n = byId.get(id); if (n) setBadgeEl(n.badge, v); };
  root.expand = (id, open = true) => setExpanded(id, open);
  root.setCompact = (c) => {
    root.classList.toggle('is-compact', !!c);
    if (!c) setOverlay(false);
    for (const { el, it } of byId.values()) {
      if (c) el.dataset.tooltip = it.label; else delete el.dataset.tooltip;
    }
  };
  root.onSelect = (cb) => { onSelect = cb; };
  build();
  return root;
}

/* ==========================================================================
   Tabs / Pivot (SelectorBar look)
   ========================================================================== */

/** Tabs({items:[{id, label, icon?}], selected, onSelect}) */
export function Tabs({ items = [], selected, onSelect, ariaLabel } = {}) {
  const bar = h('div', { class: 'fl-tabs', role: 'tablist', 'aria-label': ariaLabel || null });
  const pill = h('span', { class: 'fl-tabs__pill', 'aria-hidden': 'true' });
  let sel = selected ?? items[0]?.id;
  const tabs = new Map();
  function build() {
    clear(bar);
    tabs.clear();
    for (const it of items) {
      const b = h('button', {
        type: 'button', class: 'fl-tabs__item', role: 'tab', 'aria-selected': 'false', tabindex: -1, dataset: { id: it.id },
      }, it.icon ? iconNode(it.icon, 16) : null, h('span', { class: 'fl-tabs__label' }, it.label));
      b.addEventListener('click', () => choose(it.id, true));
      tabs.set(it.id, b);
      bar.append(b);
    }
    bar.append(pill);
    render(false);
  }
  function pillPos(b) {
    const w = 16;
    return { left: b.offsetLeft + b.offsetWidth / 2 - w / 2, w };
  }
  function render(animated, prev) {
    for (const [id, b] of tabs) {
      const on = id === sel;
      b.classList.toggle('is-selected', on);
      b.setAttribute('aria-selected', String(on));
      b.tabIndex = on ? 0 : -1;
    }
    const b = tabs.get(sel);
    if (!b) { pill.hidden = true; return; }
    pill.hidden = false;
    // Measure the tab that is selected WHEN placing: a deferred place() from an earlier render (strip not
    // yet in the DOM) must not move the pill back under a tab that was selected before.
    const place = () => {
      const cur = tabs.get(sel);
      if (!cur || !bar.isConnected || !cur.offsetWidth) return;
      const p = pillPos(cur);
      if (animated && prev && prev !== sel && tabs.get(prev) && bar.isConnected) {
        const a = pillPos(tabs.get(prev));
        const right = p.left > a.left;
        const dist = Math.abs(p.left - a.left);
        animate(pill, [
          { transform: `translateX(${a.left}px)`, width: '16px' },
          { transform: `translateX(${right ? a.left : p.left}px)`, width: `${dist + 16}px`, offset: 0.5 },
          { transform: `translateX(${p.left}px)`, width: '16px' },
        ], { duration: 333, easing: 'cubic-bezier(0.55,0.55,0,1)' });
      }
      pill.style.transform = `translateX(${p.left}px)`;
    };
    if (bar.isConnected && b.offsetWidth) place(); else requestAnimationFrame(() => requestAnimationFrame(place));
  }
  function choose(id, fire) {
    if (id === sel) return;
    const prev = sel;
    sel = id;
    render(true, prev);
    if (fire) onSelect?.(id);
  }
  bar.addEventListener('keydown', (e) => {
    const ids = items.map((i) => i.id);
    const i = ids.indexOf(sel);
    let j = null;
    if (e.key === 'ArrowRight') j = (i + 1) % ids.length;
    else if (e.key === 'ArrowLeft') j = (i - 1 + ids.length) % ids.length;
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = ids.length - 1;
    if (j == null) return;
    e.preventDefault();
    choose(ids[j], true);
    tabs.get(ids[j])?.focus();
  });
  if ('ResizeObserver' in window) new ResizeObserver(() => render(false)).observe(bar);
  bar.select = (id) => choose(id, false);
  bar.getSelected = () => sel;
  bar.setItems = (n, s = sel) => { items = n; sel = s; build(); };
  build();
  return bar;
}
export const Pivot = Tabs;

/* ==========================================================================
   Breadcrumb
   ========================================================================== */

/** Breadcrumb({items:[{id, label}], onSelect, size:'title'|'body'}) */
export function Breadcrumb({ items = [], onSelect, size = 'title' } = {}) {
  const root = h('nav', { class: ['fl-breadcrumb', `fl-breadcrumb--${size}`], 'aria-label': t('breadcrumb') });
  ownAttr(root, 'aria-label', 'breadcrumb');
  function build() {
    clear(root);
    root.setAttribute('aria-label', t('breadcrumb'));
    items.forEach((it, i) => {
      const last = i === items.length - 1;
      if (i > 0) root.append(h('span', { class: 'fl-breadcrumb__sep', 'aria-hidden': 'true' }, icon('chevron-right', { size: size === 'title' ? 16 : 12 })));
      if (last) root.append(h('span', { class: 'fl-breadcrumb__item is-current', 'aria-current': 'page' }, it.label));
      else {
        const b = h('button', { type: 'button', class: 'fl-breadcrumb__item' }, it.label);
        b.addEventListener('click', () => onSelect?.(it.id, it, i));
        root.append(b);
      }
    });
  }
  build();
  root.setItems = (n) => { items = n; build(); };
  return root;
}

/* ==========================================================================
   Tooltip (attach with data-tooltip="text"; shows after 600 ms)
   ========================================================================== */

let tipEl = null, tipTimer = 0, tipTarget = null, tipsInstalled = false;

function showTip(target, text) {
  if (!text) return;
  if (!tipEl) tipEl = h('div', { class: 'fl-tooltip acrylic', role: 'tooltip', 'data-region': '', 'data-popup': '' });
  tipEl.classList.remove('is-visible', 'is-below');
  clear(tipEl).append(h('span', { class: 'fl-tooltip__text' }, text));
  tipEl.dataset.theme = themeScope(target);
  document.body.appendChild(tipEl);
  const r = target.getBoundingClientRect();
  const tw = tipEl.offsetWidth, th = tipEl.offsetHeight, m = 6;
  const placement = target.dataset.tooltipPlacement;
  let top = placement === 'bottom' ? r.bottom + 8 : r.top - th - 8;
  if (top < m) top = r.bottom + 8;
  if (top + th > innerHeight - m) top = r.top - th - 8;
  const left = clamp(r.left + r.width / 2 - tw / 2, m, innerWidth - tw - m);
  tipEl.style.left = `${Math.round(left)}px`;
  tipEl.style.top = `${Math.round(top)}px`;
  tipEl.classList.toggle('is-below', top > r.top);   // slides down from the anchor instead of up
  void tipEl.offsetWidth;                            // restart the entrance animation
  tipEl.classList.add('is-visible');
}

export function hideTooltip() {
  clearTimeout(tipTimer);
  tipTarget = null;
  if (tipEl) { tipEl.classList.remove('is-visible'); tipEl.remove(); }
}

/** Installs the global tooltip behaviour (main.js calls this once). */
export function installTooltips(root = document) {
  if (tipsInstalled) return;
  tipsInstalled = true;
  const arm = (target, delay = 600) => {
    if (target === tipTarget) return;
    hideTooltip();
    tipTarget = target;
    tipTimer = setTimeout(() => { if (tipTarget === target && target.isConnected) showTip(target, target.dataset.tooltip); }, delay);
  };
  root.addEventListener('pointerover', (e) => {
    if (e.pointerType === 'touch') return;
    const target = e.target.closest?.('[data-tooltip]');
    if (!target || !target.dataset.tooltip) { if (tipTarget && !tipTarget.contains(e.target)) hideTooltip(); return; }
    arm(target);
  });
  root.addEventListener('pointerout', (e) => {
    if (tipTarget && !tipTarget.contains(e.relatedTarget)) hideTooltip();
  });
  root.addEventListener('focusin', (e) => {
    const target = e.target.closest?.('[data-tooltip]');
    if (target && target.matches(':focus-visible')) arm(target, 600);
  });
  root.addEventListener('focusout', hideTooltip);
  root.addEventListener('pointerdown', hideTooltip, true);
  root.addEventListener('keydown', (e) => { if (e.key === 'Escape') hideTooltip(); }, true);
  addEventListener('scroll', hideTooltip, true);
  addEventListener('blur', hideTooltip);
}

export const Tooltip = {
  install: installTooltips,
  attach(el, text, placement) { el.dataset.tooltip = text; if (placement) el.dataset.tooltipPlacement = placement; return el; },
  hide: hideTooltip,
};

/* ==========================================================================
   Toast (in-app notification, bottom centre)
   ========================================================================== */

let toastStack = null;
export const Toast = {
  /** Toast.show({title, message, severity, icon, action:{label,onClick}, timeout=5000}) → {close} */
  show({ title = '', message = '', severity, icon: ic, action, timeout = 5000, theme } = {}) {
    if (!toastStack || !toastStack.isConnected) {
      toastStack = h('div', { class: 'fl-toast-stack', 'aria-live': 'polite' });
      document.body.appendChild(toastStack);
    }
    const lead = severity ? severityIcon(severity, 16) : (ic ? iconNode(ic, 20) : null);
    const closeBtn = IconButton({ icon: 'close', label: t('close'), size: 28, iconSize: 12, tooltip: false });
    ownAttr(closeBtn, 'aria-label', 'close');
    const el = h('div', {
      class: 'fl-toast acrylic surface-flyout', role: 'status', 'data-region': '', 'data-popup': '',
      dataset: { theme: theme || document.documentElement.dataset.theme },
    },
    lead ? h('span', { class: 'fl-toast__icon' }, lead) : null,
    h('div', { class: 'fl-toast__body' },
      title ? h('div', { class: 'fl-toast__title' }, title) : null,
      message ? h('div', { class: 'fl-toast__message' }, message) : null,
      action ? h('div', { class: 'fl-toast__action' }, Button({ label: action.label, onClick: () => { action.onClick?.(); close(); } })) : null),
    closeBtn);
    toastStack.append(el);
    const entrance = [
      animate(el, [{ transform: 'translateY(16px)' }, { transform: 'none' }], { duration: 250 }),
      ...fadeContent(el, 'in', { duration: 167, easing: 'linear' }),
    ];
    finished(entrance);
    let timer = timeout ? setTimeout(close, timeout) : 0;
    el.addEventListener('pointerenter', () => clearTimeout(timer));
    el.addEventListener('pointerleave', () => { if (timeout) timer = setTimeout(close, timeout / 2); });
    let closed = false;
    function close() {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      el.removeAttribute('data-region');
      cancelAll(entrance);
      finished([
        animate(el, [{ transform: 'none' }, { transform: 'translateY(8px)' }], { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' }),
        ...fadeContent(el, 'out', { duration: 167, easing: 'linear', fill: 'forwards' }),
      ]).then(() => el.remove());
    }
    closeBtn.addEventListener('click', close);
    return { el, close };
  },
};

/* ==========================================================================
   Badge, Avatar, AppIcon
   ========================================================================== */

/** Badge({value, severity:'attention'|'informational'|'success'|'caution'|'critical', icon}) — InfoBadge */
export function Badge({ value = '', severity = 'attention', icon: ic } = {}) {
  const dot = value === '' && !ic;
  const el = h('span', { class: ['fl-badge', `fl-badge--${severity}`, dot && 'fl-badge--dot'] },
    ic ? iconNode(ic, 10) : (dot ? null : String(value)));
  el.setValue = (v) => {
    el.classList.toggle('fl-badge--dot', v === '' || v == null);
    el.textContent = v === '' || v == null ? '' : String(v);
  };
  return el;
}

/** Avatar({name, image, size=32}) — PersonPicture: photo, initials or a person glyph */
export function Avatar({ name = '', image, size = 32 } = {}) {
  const el = h('span', { class: 'fl-avatar', style: { width: `${size}px`, height: `${size}px`, fontSize: `${Math.round(size * 0.4)}px` }, 'aria-label': name || null, role: name ? 'img' : null });
  const initials = String(name).trim().split(/\s+/).filter(Boolean).map((w) => w[0]).filter((c) => /\p{L}|\d/u.test(c));
  const fallback = () => {
    clear(el);
    if (initials.length) el.append(h('span', { class: 'fl-avatar__initials' }, (initials[0] + (initials.length > 1 ? initials[initials.length - 1] : '')).toUpperCase()));
    else el.append(icon('user', { size: Math.round(size * 0.5) }));
  };
  if (image) {
    const img = h('img', { src: image, alt: '', draggable: false });
    img.addEventListener('error', fallback);
    el.append(img);
  } else fallback();
  return el;
}

/** AppIcon({name, size=32, fallback='app'}) — themed app/file icon via bridge.iconUrl, crisp on HiDPI */
export function AppIcon({ name, mime, size = 32, fallback = 'app' } = {}) {
  const px = Math.round(size * Math.min(3, Math.max(1, devicePixelRatio || 1)));
  const url = mime ? bridge.mimeIconUrl(mime, px) : bridge.iconUrl(name, px);
  const wrap = h('span', { class: 'fl-app-icon', style: { width: `${size}px`, height: `${size}px` } });
  const showFallback = () => { clear(wrap).append(icon(fallback, { size: Math.round(size * 0.75) })); wrap.classList.add('is-fallback'); };
  if (!url) showFallback();
  else {
    const img = h('img', { src: url, width: size, height: size, alt: '', draggable: false, loading: 'lazy', decoding: 'async' });
    img.addEventListener('error', showFallback, { once: true });
    wrap.append(img);
  }
  return wrap;
}

/* ==========================================================================
   ColorSwatchGrid & EmptyState
   ========================================================================== */

/** ColorSwatchGrid({colors=ACCENT_PRESETS, value, onChange, columns=8, size=40}) */
export function ColorSwatchGrid({ colors = ACCENT_PRESETS, value, onChange, columns = 8, size = 40, ariaLabel } = {}) {
  const norm = (c) => String(c || '').toLowerCase();
  let current = norm(value);
  // the schema default #0067c0 is Windows' default blue (#0078d4) rendered in light mode
  const eq = (a, b) => norm(a) === norm(b) || (new Set([norm(a), norm(b)]).size === 2 && [norm(a), norm(b)].every((x) => x === '#0067c0' || x === '#0078d4'));
  const grid = h('div', {
    class: 'fl-swatches', role: 'radiogroup', 'aria-label': ariaLabel || null,
    style: { gridTemplateColumns: `repeat(${columns}, ${size}px)`, '--sw': `${size}px` },
  });
  const btns = colors.map((c, i) => {
    const b = h('button', {
      type: 'button', class: 'fl-swatch', role: 'radio', 'aria-label': t('color', { c }), tabindex: -1,
      style: { '--c': c }, dataset: { color: c, index: i, tooltip: c.toUpperCase() },
    }, h('span', { class: 'fl-swatch__check' }, icon('check', { size: 12 })));
    ownAttr(b, 'aria-label', 'color', { c });
    b.addEventListener('click', () => choose(c, true));
    grid.append(b);
    return b;
  });
  function render() {
    let any = false;
    btns.forEach((b) => {
      const on = eq(b.dataset.color, current);
      b.classList.toggle('is-selected', on);
      b.setAttribute('aria-checked', String(on));
      b.tabIndex = on ? 0 : -1;
      if (on) any = true;
    });
    if (!any && btns[0]) btns[0].tabIndex = 0;
  }
  function choose(c, fire) {
    current = norm(c); render();
    if (fire) onChange?.(c, grid);
  }
  grid.addEventListener('keydown', (e) => {
    const i = btns.indexOf(document.activeElement);
    if (i < 0) return;
    const moves = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: columns, ArrowUp: -columns };
    if (moves[e.key] != null) {
      e.preventDefault();
      const j = clamp(i + moves[e.key], 0, btns.length - 1);
      btns[j].focus();
    } else if (e.key === ' ' || e.key === 'Enter') { e.preventDefault(); choose(btns[i].dataset.color, true); }
  });
  render();
  grid.setValue = (c) => choose(c, false);
  grid.getValue = () => current;
  return grid;
}

/** EmptyState({icon, title, message, action:{label,onClick}|Element}) */
export function EmptyState({ icon: ic = 'info', title = '', message = '', action } = {}) {
  return h('div', { class: 'fl-empty' },
    ic ? h('div', { class: 'fl-empty__icon' }, iconNode(ic, 32)) : null,
    title ? h('div', { class: 'fl-empty__title subtitle' }, title) : null,
    message ? h('div', { class: 'fl-empty__message' }, message) : null,
    action ? h('div', { class: 'fl-empty__action' }, action instanceof Node ? action : Button({ label: action.label, onClick: action.onClick, variant: action.variant || 'accent' })) : null);
}

/* ==========================================================================
   Small layout helpers
   ========================================================================== */

/** Page header used inside apps: title (or breadcrumb) + optional description. */
export function PageHeader({ title, description, breadcrumb } = {}) {
  return h('header', { class: 'fl-page-header' },
    breadcrumb || h('h1', { class: 'title' }, title),
    description ? h('p', { class: 'fl-page-header__desc' }, description) : null);
}

/** ListItem({icon, title, description, trailing, onClick, selected}) — simple 40px+ list row */
export function ListItem({ icon: ic, title = '', description = '', trailing, onClick, selected = false } = {}) {
  const el = h('div', {
    class: ['fl-list-item', 'fl-list-row', selected && 'is-selected', onClick && 'is-clickable'],
    role: onClick ? 'button' : null, tabindex: onClick ? 0 : null,
  }, ic ? iconNode(ic, 20) : null,
  h('span', { class: 'fl-list-item__text' }, title, description ? h('span', { class: 'fl-list-item__desc' }, description) : null),
  trailing ? h('span', { class: 'fl-list-row__trailing' }, ...asNodes(trailing)) : null);
  if (onClick) {
    el.addEventListener('click', (e) => onClick(e, el));
    el.addEventListener('keydown', (e) => { if ((e.key === 'Enter' || e.key === ' ') && e.target === el) { e.preventDefault(); onClick(e, el); } });
  }
  return el;
}
