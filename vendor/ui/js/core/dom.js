// Win11 Shell — tiny DOM helpers (owned by ui-core).
//
//   h('div', {class: 'a', style: {gap: '8px'}, onClick: fn, dataset: {id: 1},
//             'aria-label': 'x', attrs: {role: 'button'}, text: 'Hallo'}, child, [more])
//
// props may be omitted (h('span', 'text')). Children: Node | string | number |
// array | null/false (skipped).

const SVG_NS = 'http://www.w3.org/2000/svg';
const SVG_TAGS = new Set(['svg', 'path', 'circle', 'rect', 'g', 'line', 'polyline', 'polygon',
  'ellipse', 'defs', 'linearGradient', 'radialGradient', 'stop', 'text', 'tspan', 'use',
  'clipPath', 'mask', 'pattern', 'symbol', 'title', 'foreignObject', 'filter',
  'feGaussianBlur', 'feOffset', 'feMerge', 'feMergeNode', 'feColorMatrix']);

// Properties that must be set as DOM properties (not attributes)
const PROP_KEYS = new Set(['value', 'checked', 'selected', 'disabled', 'indeterminate',
  'tabIndex', 'hidden', 'id', 'title', 'type', 'src', 'href', 'placeholder', 'name',
  'readOnly', 'multiple', 'min', 'max', 'step', 'draggable', 'spellcheck', 'autocomplete',
  'htmlFor', 'alt', 'width', 'height', 'contentEditable', 'lang', 'dir', 'role',
  'textContent', 'innerHTML', 'scrollTop', 'scrollLeft', 'maxLength', 'rows', 'cols']);

function isPlainProps(v) {
  return v != null && typeof v === 'object' && !(v instanceof Node) && !Array.isArray(v);
}

export function classNames(v) {
  if (!v) return '';
  if (typeof v === 'string') return v;
  if (Array.isArray(v)) return v.map(classNames).filter(Boolean).join(' ');
  if (typeof v === 'object') return Object.keys(v).filter((k) => v[k]).join(' ');
  return String(v);
}

export function appendChildren(el, children) {
  for (const c of children) {
    if (c == null || c === false || c === true) continue;
    if (Array.isArray(c)) { appendChildren(el, c); continue; }
    if (c instanceof Node) { el.appendChild(c); continue; }
    el.appendChild(document.createTextNode(String(c)));
  }
  return el;
}

export function h(tag, props, ...children) {
  if (!isPlainProps(props)) {
    if (props !== undefined) children.unshift(props);
    props = null;
  }
  const svg = SVG_TAGS.has(tag) || (props && props.svg);
  const el = svg ? document.createElementNS(SVG_NS, tag) : document.createElement(tag);
  if (props) applyProps(el, props, svg);
  appendChildren(el, children);
  return el;
}

export function applyProps(el, props, svg = el instanceof SVGElement) {
  for (const key of Object.keys(props)) {
    const v = props[key];
    if (key === 'svg' || v === undefined) continue;
    if (key === 'class' || key === 'className') {
      const cls = classNames(v);
      if (cls) { if (svg) el.setAttribute('class', cls); else el.className = cls; }
    } else if (key === 'style') {
      if (typeof v === 'string') el.style.cssText = v;
      else if (v) for (const s of Object.keys(v)) {
        if (v[s] == null || v[s] === false) continue;
        if (s.startsWith('--') || s.includes('-')) el.style.setProperty(s, String(v[s]));
        else el.style[s] = typeof v[s] === 'number' && !UNITLESS.has(s) ? v[s] + 'px' : v[s];
      }
    } else if (key === 'dataset') {
      for (const d of Object.keys(v || {})) if (v[d] != null) el.dataset[d] = v[d];
    } else if (key === 'attrs') {
      for (const a of Object.keys(v || {})) setAttr(el, a, v[a]);
    } else if (key === 'text') {
      el.textContent = v == null ? '' : String(v);
    } else if (key === 'html') {
      el.innerHTML = v; // only for trusted, static markup
    } else if (key === 'ref') {
      if (typeof v === 'function') v(el);
    } else if (key.length > 2 && key[0] === 'o' && key[1] === 'n' && typeof v === 'function') {
      const ev = key[2] === key[2].toUpperCase() ? key.slice(2).toLowerCase() : key.slice(2);
      el.addEventListener(ev, v);
    } else if (!svg && PROP_KEYS.has(key) && key !== 'role') {
      if (v === null || v === false) {
        if (key === 'disabled' || key === 'checked' || key === 'hidden' || key === 'readOnly') el[key] = false;
      } else el[key] = v;
    } else {
      setAttr(el, key, v);
    }
  }
  return el;
}

const UNITLESS = new Set(['opacity', 'zIndex', 'flex', 'flexGrow', 'flexShrink', 'order',
  'fontWeight', 'lineHeight', 'zoom', 'gridRow', 'gridColumn']);

function setAttr(el, name, v) {
  if (v === null || v === false || v === undefined) el.removeAttribute(name);
  else el.setAttribute(name, v === true ? '' : String(v));
}

export const $ = (sel, root = document) => root.querySelector(sel);
export const $$ = (sel, root = document) => Array.from(root.querySelectorAll(sel));

export function clear(el) {
  if (!el) return el;
  while (el.firstChild) el.removeChild(el.firstChild);
  return el;
}

export function mount(el, parent = document.body) {
  parent.appendChild(el);
  return el;
}

/** Replace all children of parent with the given nodes. */
export function setChildren(parent, ...children) {
  clear(parent);
  appendChildren(parent, children);
  return parent;
}

const cssLoaded = new Map();
const CSS_BASE = new URL('../../css/', import.meta.url);

/** loadCss('taskbar') → adds <link href=".../css/taskbar.css"> once. Resolves when loaded. */
export function loadCss(name) {
  if (cssLoaded.has(name)) return cssLoaded.get(name);
  const href = new URL(name.endsWith('.css') ? name : name + '.css', CSS_BASE).href;
  const existing = Array.from(document.querySelectorAll('link[rel="stylesheet"]')).find((l) => l.href === href);
  const p = new Promise((resolve) => {
    if (existing) { resolve(existing); return; }
    const link = document.createElement('link');
    link.rel = 'stylesheet';
    link.href = href;
    link.onload = () => resolve(link);
    link.onerror = () => { console.warn('[dom] stylesheet failed to load:', href); resolve(link); };
    document.head.appendChild(link);
  });
  cssLoaded.set(name, p);
  return p;
}

/** el.addEventListener with an off() return value. */
export function on(target, type, fn, opts) {
  target.addEventListener(type, fn, opts);
  return () => target.removeEventListener(type, fn, opts);
}

export function debounce(fn, ms = 150) {
  let t = 0;
  const d = (...args) => { clearTimeout(t); t = setTimeout(() => fn(...args), ms); };
  d.cancel = () => clearTimeout(t);
  return d;
}

export function throttle(fn, ms = 100) {
  let last = 0, t = 0, lastArgs = null;
  return (...args) => {
    lastArgs = args;
    const now = performance.now();
    const run = () => { last = performance.now(); t = 0; fn(...lastArgs); };
    if (now - last >= ms) run();
    else if (!t) t = setTimeout(run, ms - (now - last));
  };
}

let uidCounter = 0;
export function uid(prefix = 'fl') { return `${prefix}-${(++uidCounter).toString(36)}`; }

export function clamp(v, lo, hi) { return Math.min(hi, Math.max(lo, v)); }

export function reducedMotion() {
  if (document.documentElement.dataset.reduceMotion === 'true') return true;
  try { return matchMedia('(prefers-reduced-motion: reduce)').matches; } catch { return false; }
}

/** Web Animations helper that honours reduce-motion. Returns the Animation (or null). */
export function animate(el, keyframes, opts = {}) {
  if (!el || !el.animate) return null;
  const o = typeof opts === 'number' ? { duration: opts } : { ...opts };
  if (reducedMotion()) o.duration = 1;
  if (!o.easing) o.easing = 'cubic-bezier(0,0,0,1)';
  return el.animate(keyframes, o);
}

/**
 * Resolves after the given animation(s) (an Animation, an array, or null).
 * Never hangs: when the document timeline is throttled (hidden window, headless
 * capture) the animation is finished after its end time + 100 ms, so close/cleanup
 * code always runs. Cancelled animations resolve too.
 */
export function finished(anim) {
  if (!anim) return Promise.resolve();
  if (Array.isArray(anim)) return Promise.all(anim.map(finished)).then(() => {});
  let end = NaN;
  try { end = anim.effect?.getComputedTiming?.().endTime; } catch { end = NaN; }
  if (end === Infinity) return anim.finished.then(() => {}, () => {});
  if (!Number.isFinite(end)) end = 400;
  return new Promise((resolve) => {
    let done = false;
    const settle = () => { if (!done) { done = true; clearTimeout(timer); resolve(); } };
    const timer = setTimeout(() => {
      if (!done && (anim.playState === 'running' || anim.playState === 'pending')) {
        try { anim.finish(); } catch { /* playback rate 0 etc. */ }
        // still stuck (frozen timeline): drop the effect so nothing stays half-faded
        if (anim.playState !== 'finished') { try { anim.cancel(); } catch { /* ignore */ } }
      }
      settle();
    }, Math.max(0, end) + 100);
    anim.finished.then(settle, settle);
  });
}

/* ---- Frost-safe animation ------------------------------------------------
   Chrome and WebKit switch backdrop-filter OFF while an element or any of its
   ancestors has opacity < 1 (also filter / mask / clip-path on an ancestor). So a
   frosted surface (Acrylic / Mica) must never sit inside something that fades.
   Recipe: animate only transform (and clip-path on the frosted element itself),
   and fade the CONTENT with fadeContent(). On exit, fading the glass itself is
   fine (its own opacity keeps the blur, it just blends out). */

/** Frosted materials (classes from base.css, flyouts.css, shell.css; opt in with data-frosted). */
export const FROSTED = '.acrylic, .acrylic-base, .taskbar-material, .mica, .mica-alt, .fo-acrylic, .sh-acrylic, [data-frosted]';

/** Does `el` paint a backdrop blur (known material class or a computed backdrop-filter)? */
export function isFrosted(el) {
  if (!(el instanceof Element)) return false;
  if (el.matches(FROSTED)) return true;
  try {
    const cs = getComputedStyle(el);
    const bf = cs.backdropFilter || cs.webkitBackdropFilter;
    return !!bf && bf !== 'none';
  } catch { return false; }
}

/**
 * The elements whose opacity can be animated to fade `root` without ever giving a
 * frosted element an ancestor with opacity < 1. `glass: true` lets the frosted
 * elements themselves fade (exit animations); otherwise their children fade.
 */
export function fadeTargets(root, { glass = false } = {}) {
  const out = [];
  const walk = (el) => {
    const frosted = isFrosted(el);
    if (frosted && glass) { out.push(el); return; }
    if (!frosted && !el.querySelector(FROSTED)) { out.push(el); return; }
    for (const c of el.children) walk(c);
  };
  if (root instanceof Element) walk(root);
  return out;
}

/**
 * fadeContent(root, 'in'|'out', {duration, easing, delay, fill, from, glass}) → Animation[]
 * Fades `root` the frost-safe way (see fadeTargets). Exit ('out') fades the glass
 * itself unless glass:false.
 */
export function fadeContent(root, dir = 'in', opts = {}) {
  const { glass = dir === 'out', from, ...o } = opts;
  const lo = Number.isFinite(from) ? from : 0;
  const frames = dir === 'in' ? [{ opacity: lo }, { opacity: 1 }] : [{ opacity: 1 }, { opacity: lo }];
  return fadeTargets(root, { glass })
    .map((el) => animate(el, frames, { duration: 167, easing: 'linear', ...o }))
    .filter(Boolean);
}

/** Cancel a list of animations (ignores nulls). */
export function cancelAll(anims) {
  for (const a of anims || []) { try { a?.cancel(); } catch { /* ignore */ } }
}

/** Is the element (or something inside it) focused? */
export function hasFocusWithin(el) {
  return !!el && el.contains(document.activeElement);
}

/** Visible, focusable descendants (for focus traps / roving focus). */
export function focusables(root) {
  return $$('a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])', root)
    .filter((e) => e.offsetParent !== null || e.getClientRects().length);
}

/** Escape text for use in innerHTML (prefer h()/textContent). */
export function esc(s) {
  return String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
}
