// Win11 Shell — input regions for the layer-shell `shell` surface (owned by ui-core).
//
// Every visible element marked `data-region` (taskbar, flyouts, menus, popups, toasts…)
// becomes part of the surface's input region: its bounding rect + a 16 px shadow margin
// (override per element with data-region-margin="0"). Changes are sent, throttled and
// de-duplicated, with bridge.call('shell.setRegions', {rects, keyboard}).
// keyboard = 'exclusive' while something holds the keyboard (open flyout / menu, or
// setKeyboardMode('exclusive')), otherwise 'on-demand'. A switch to 'exclusive' waits while a pointer
// button is held on the surface: Hyprland (LayerSurface.cpp, none → exclusive) answers it with
// releaseAllMouseButtons(), a fake button release, and then drops the real one — a drag that opens a
// flyout (tray icon dragged up) would end on the spot.
// flushRegions() sends the current state at once and waits until the host committed a keyboard change
// (the tray gives the keyboard back before it activates an app, so the window that app shows gets focus).
// keyboardSettling(): a keyboard change is still waiting to be sent, or was sent < 300 ms ago (perhaps not
// committed yet): whoever is about to focus a window then awaits flushRegions() first.
// Everything is a no-op unless shell.info().mode === 'layer' on the `shell` surface.

import { bridge } from './bridge.js';
import { getShellInfo } from './theme.js';

const DEFAULT_MARGIN = 16;
let active = false;
let started = false;
let kbMode = 'on-demand';
let holds = 0;
let lastKey = '';
let lastKb = '';      // keyboard mode last sent
let lastKbAt = -1e9;  // performance.now() when a different keyboard mode was last sent
let buttons = 0;      // pointer buttons held on this surface
let timer = 0;
let ro = null;
const observed = new WeakSet();

/** Start tracking (main.js calls this for the shell surface). Resolves to true when active. */
export async function initRegions({ force = false } = {}) {
  if (started) return active;
  started = true;
  let info = getShellInfo();
  if (!info) { try { info = await bridge.call('shell.info'); } catch { info = null; } }
  const surface = document.documentElement.dataset.surface || bridge.surface;
  active = force || (info?.mode === 'layer' && surface === 'shell');
  if (!active) return false;
  const mo = new MutationObserver(schedule);
  mo.observe(document.documentElement, {
    subtree: true, childList: true, attributes: true,
    attributeFilter: ['data-region', 'data-region-margin', 'class', 'style', 'hidden', 'open'],
  });
  ro = new ResizeObserver(schedule);
  addEventListener('resize', schedule);
  // a missed pointerup is harmless: the next pointer event (or a blur) corrects `buttons`
  const track = (e) => {
    const was = buttons;
    buttons = e.type === 'pointercancel' ? 0 : e.buttons;
    if (was && !buttons) schedule();
  };
  for (const t of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) addEventListener(t, track, true);
  addEventListener('blur', () => { buttons = 0; schedule(); });
  document.addEventListener('transitionend', schedule, true);
  document.addEventListener('animationend', schedule, true);
  schedule();
  return true;
}

export function regionsActive() { return active; }

/** setKeyboardMode('exclusive' | 'on-demand') */
export function setKeyboardMode(mode) {
  kbMode = mode === 'exclusive' ? 'exclusive' : 'on-demand';
  schedule();
}

/** Request exclusive keyboard while something is open; returns release(). */
export function holdKeyboard() {
  holds++;
  schedule();
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holds = Math.max(0, holds - 1);
    schedule();
  };
}

export function refreshRegions() { schedule(); }

/** A keyboard-mode change not sent yet, or sent so recently that the compositor may not have it yet. */
export function keyboardSettling() {
  return active && (keyboardToSend() !== lastKb || performance.now() - lastKbAt < 300);
}

function noteKb(keyboard) {
  if (keyboard !== lastKb) lastKbAt = performance.now();
  lastKb = keyboard;
}

/** Current region rects (viewport px) — exported for debugging/tests. */
export function collectRegions() {
  const vw = innerWidth, vh = innerHeight;
  const rects = [];
  for (const el of document.querySelectorAll('[data-region]')) {
    if (!el.isConnected || !el.getClientRects().length) continue;
    if (ro && !observed.has(el)) { ro.observe(el); observed.add(el); }
    const cs = getComputedStyle(el);
    if (cs.visibility === 'hidden' || cs.display === 'none') continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) continue;
    const m = el.hasAttribute('data-region-margin') ? Number(el.getAttribute('data-region-margin')) || 0 : DEFAULT_MARGIN;
    const x1 = Math.max(0, Math.floor(r.left - m)), y1 = Math.max(0, Math.floor(r.top - m));
    const x2 = Math.min(vw, Math.ceil(r.right + m)), y2 = Math.min(vh, Math.ceil(r.bottom + m));
    if (x2 > x1 && y2 > y1) rects.push({ x: x1, y: y1, w: x2 - x1, h: y2 - y1 });
  }
  return rects;
}

function keyboardMode() {
  return holds > 0 || kbMode === 'exclusive' ? 'exclusive' : 'on-demand';
}

/** The mode to send now: 'exclusive' only once no button is held (see the header). */
function keyboardToSend() {
  const keyboard = keyboardMode();
  if (keyboard === 'exclusive' && lastKb !== 'exclusive' && buttons) return lastKb || 'on-demand';
  return keyboard;
}

function schedule() {
  if (!active || timer) return;
  timer = setTimeout(() => {
    timer = 0;
    requestAnimationFrame(send);
  }, 40);
}

function send() {
  const rects = collectRegions();
  const keyboard = keyboardToSend();
  const key = JSON.stringify([rects, keyboard]);
  if (key === lastKey) return;
  lastKey = key;
  noteKb(keyboard);
  bridge.call('shell.setRegions', { rects, keyboard }).catch((e) => {
    console.warn('[regions] shell.setRegions failed', e);
    lastKey = '';
    lastKb = '';
  });
}

/**
 * Send the regions + keyboard mode now (no throttle) with sync:true: resolves once the host has applied
 * them and committed a keyboard change to the compositor. Resolves at once when regions are off.
 */
export function flushRegions() {
  if (!active) return Promise.resolve();
  clearTimeout(timer);
  timer = 0;
  const rects = collectRegions();
  const keyboard = keyboardToSend();
  lastKey = JSON.stringify([rects, keyboard]);
  noteKb(keyboard);
  return bridge.call('shell.setRegions', { rects, keyboard, sync: true }).then(() => {}, () => { lastKey = ''; lastKb = ''; });
}
