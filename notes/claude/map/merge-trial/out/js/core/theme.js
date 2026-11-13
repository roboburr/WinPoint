// Win11 Shell — theme & settings access (owned by ui-core). SPEC §4.
//
//   await initTheme('app');          // main.js does this before mounting any module
//   getSettings()                    // cached settings object (do not mutate)
//   await setSetting('theme.mode', 'light')
//   const off = onSettings((settings, path) => …)
//   resolveWallpaper(settings) → url ('' for a plain colour wallpaper)
//
// Sets on <html>: data-theme, data-transparency, data-blur, data-backdrop, data-surface,
// data-reduce-motion, and CSS vars --accent, --accent-light-1..3, --accent-dark-1..3,
// --accent-base, --accent-text, --accent-on-light, --accent-on-dark, --ts.
//
// data-backdrop="blur|none": does in-page backdrop-filter paint here? Our host's WebKit paints none at
// all without GPU compositing (CSS.supports() still says yes), so host/surfaces.py tells every frame
// before it runs (window.__win11shellHost.backdrop). With "none", data-blur "css" becomes "none" and
// app surfaces give their Acrylic overlays the solid fill (tokens.css). ?backdrop=none|blur forces it.

import { bridge } from './bridge.js';
import { setLang } from './i18n.js';
import { loadCss } from './dom.js';

const html = document.documentElement;
const CACHE_KEY = 'win11shell.themeCache';

/** Minimal fallback when settings cannot be loaded at all. */
const FALLBACK = {
  version: 1, language: 'nl',
  theme: { mode: 'dark', accent: '#0067c0', accentFromWallpaper: false, transparency: true, syncSystem: true,
           accentOnTaskbar: false, captionButtons: true, gtkFamily: 'fluent' },
  wallpaper: { type: 'image', path: 'builtin:bloom-dark', color: '#0b2a4a', fit: 'fill', followTheme: true },
  accessibility: { textScale: 100, reduceMotion: false },
  time: { h24: true, showSeconds: false, showDate: true, firstDayOfWeek: 1 },
};

/** The Windows accent colour grid (Instellingen › Persoonlijke instellingen › Kleuren). */
export const ACCENT_PRESETS = [
  '#FFB900', '#FF8C00', '#F7630C', '#CA5010', '#DA3B01', '#EF6950', '#D13438', '#FF4343',
  '#E74856', '#E81123', '#EA005E', '#C30052', '#E3008C', '#BF0077', '#C239B3', '#9A0089',
  '#0078D4', '#0063B1', '#8E8CD8', '#6B69D6', '#8764B8', '#744DA9', '#B146C2', '#881798',
  '#0099BC', '#2D7D9A', '#00B7C3', '#038387', '#00B294', '#018574', '#00CC6A', '#10893E',
  '#7A7574', '#5D5A58', '#68768A', '#515C6B', '#567C73', '#486860', '#498205', '#107C10',
  '#767676', '#4C4A48', '#69797E', '#4A5459', '#647C64', '#525E54', '#847545', '#7E735F',
];

const params = bridge.params;
const forcedTheme = ['light', 'dark'].includes(params.get('theme')) ? params.get('theme') : null;
const forcedBlur = ['css', 'compositor', 'none'].includes(params.get('blur')) ? params.get('blur') : null;
const forcedTransparency = ['on', 'off'].includes(params.get('transparency')) ? params.get('transparency') : null;
// ?lang=nl|en wins over settings.language (like ?theme=), for testing and screenshots
const forcedLang = ['nl', 'en'].includes(params.get('lang')) ? params.get('lang') : null;

/** 'blur' | 'none': can this page blur in the page (backdrop-filter)? Fixed for the page's lifetime. */
const backdrop = (() => {
  const ok = (v) => (v === 'blur' || v === 'none' ? v : null);
  const forced = ok(params.get('backdrop'));
  if (forced) return forced;
  let host = null;
  try { host = ok(window.__win11shellHost?.backdrop); } catch { host = null; }
  if (host) return host;
  if (bridge.embedded) {                     // preview iframes: same WebView, same answer as the page around it
    try { host = ok(window.parent.document.documentElement.dataset.backdrop); } catch { host = null; }
    if (host) return host;
  }
  return 'blur';                             // a browser (mock / serve mode)
})();

let settings = null;
let shellInfo = null;
let surfaceKind = params.get('surface') || 'preview';
let initPromise = null;
const subs = new Set();
// JSON of the settings subscribers last received: a change is announced whenever the cache differs from
// what they painted (not from the cache before it), so an optimistic value overwritten by an older event
// and then confirmed by the reply is painted again
// GNOME's real background effect is a separate capability; it never implies that
// WebKit can blur an in-page menu. No periodic per-window compositor polling.
let nativePid = null, nativePacket = null, nativeKey = '', nativePending = false;
let nativeAgain = false, nativeEpoch = 0, nativeStopped = false, nativeConnected = bridge.connected;
const hasNativeBackdrop = window.__win11shellHost?.nativeBackdrop === true;
const forcedColours = () => { try { return matchMedia('(forced-colors: active)').matches; } catch { return false; } };
function applyNativeBackdrop() {
  const state = nativeConnected && nativePacket?.available ? nativePacket.windows?.find((item) => item.pid === nativePid) : null;
  const enabled = html.dataset.transparency === 'on' && !forcedColours() && state?.blur;
  html.dataset.nativeBackdrop = enabled && ['window', 'taskbar'].includes(state.kind) ? state.kind : 'none';
}
function receiveNativeBackdrop(data) {
  if (!hasNativeBackdrop || nativeStopped || !nativeConnected || !data) return;
  if (data.reset) {
    ++nativeEpoch; nativePacket = null; nativeKey = ''; applyNativeBackdrop();
    if (data.available) requestNativeBackdrop();
    return;
  }
  if (nativePacket?.generation === data.generation && Number.isFinite(data.serial) && data.serial < nativePacket.serial) return;
  nativePacket = data; applyNativeBackdrop();
}
function requestNativeBackdrop() {
  if (!hasNativeBackdrop || nativeStopped || !nativeConnected || !settings) return;
  const key = JSON.stringify([settings.theme?.transparency !== false, settings.taskbar?.autoHide === true,
    settings.accessibility?.highContrast === true]);
  if (nativePending) { nativeAgain = true; return; }
  if (nativeKey === key) return;
  nativeKey = key; nativePending = true; const epoch = nativeEpoch;
  loadCss('native-backdrop');
  bridge.call('nativeBackdrop.configure').then((data) => {
    if (nativeStopped || epoch !== nativeEpoch) return;
    nativePid = data.pid; receiveNativeBackdrop(data);
  }).catch(() => { if (epoch === nativeEpoch) { nativePacket = null; applyNativeBackdrop(); } })
    .finally(() => { nativePending = false; if (nativeAgain) { nativeAgain = false; requestNativeBackdrop(); } });
}
if (hasNativeBackdrop) {
  bridge.on('nativeBackdrop.changed', receiveNativeBackdrop);
  bridge.on('bridge.status', (state) => {
    nativeConnected = state.connected === true;
    ++nativeEpoch; nativeKey = ''; nativePacket = null; applyNativeBackdrop();
    if (nativeConnected) requestNativeBackdrop();
  });
  addEventListener('pagehide', () => { nativeStopped = true; nativePacket = null; applyNativeBackdrop(); }, {once: true});
  try { matchMedia('(forced-colors: active)').addEventListener('change', applyNativeBackdrop); } catch { /* optional */ }
}

let notified = null;
let evSeq = 0;        // settings.changed events received (a reply older than one of them is not applied)

/* ======================================================================
   Colour maths (sRGB ⇄ OKLCH)
   ====================================================================== */

export function normalizeHex(hex) {
  if (typeof hex !== 'string') return null;
  let h = hex.trim().toLowerCase();
  if (!h.startsWith('#')) h = '#' + h;
  if (/^#[0-9a-f]{3}$/.test(h)) h = '#' + h.slice(1).split('').map((c) => c + c).join('');
  if (/^#[0-9a-f]{8}$/.test(h)) h = h.slice(0, 7);
  return /^#[0-9a-f]{6}$/.test(h) ? h : null;
}

export function hexToRgb(hex) {
  const n = parseInt(normalizeHex(hex)?.slice(1) || '000000', 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export function rgbToHex(r, g, b) {
  const c = (v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, '0');
  return '#' + c(r) + c(g) + c(b);
}

const toLin = (c) => { c /= 255; return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4; };
const fromLin = (c) => 255 * (c <= 0.0031308 ? 12.92 * c : 1.055 * c ** (1 / 2.4) - 0.055);

export function hexToOklch(hex) {
  const [R, G, B] = hexToRgb(hex).map(toLin);
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const b = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [L, Math.hypot(a, b), ((Math.atan2(b, a) * 180) / Math.PI + 360) % 360];
}

function oklchToLinRgb(L, C, H) {
  const hr = (H * Math.PI) / 180;
  const a = C * Math.cos(hr), b = C * Math.sin(hr);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

/** OKLCH → hex, reducing chroma until the colour fits in sRGB. */
export function oklchToHex(L, C, H) {
  L = Math.min(1, Math.max(0, L));
  let lo = 0, hi = C, rgb = oklchToLinRgb(L, C, H);
  const inGamut = (c) => c.every((v) => v >= -0.0005 && v <= 1.0005);
  if (!inGamut(rgb)) {
    for (let i = 0; i < 22; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(oklchToLinRgb(L, mid, H))) lo = mid; else hi = mid;
    }
    rgb = oklchToLinRgb(L, lo, H);
  }
  return rgbToHex(...rgb.map((v) => fromLin(Math.min(1, Math.max(0, v)))));
}

export function luminance(hex) {
  const [r, g, b] = hexToRgb(hex).map(toLin);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a, b) {
  const la = luminance(a), lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

/** Text colour (#fff or #000) for a fill; `prefer` wins when it is readable enough. */
export function textOn(fill, prefer = '#ffffff') {
  const other = prefer === '#ffffff' ? '#000000' : '#ffffff';
  if (contrast(prefer, fill) >= 3.5) return prefer;
  return contrast(other, fill) > contrast(prefer, fill) ? other : prefer;
}

/* Windows' own palette for the default blue (registry AccentPalette). */
const WINDOWS_BLUE = {
  base: '#0078d4', light1: '#0091f8', light2: '#4cc2ff', light3: '#99ebff',
  dark1: '#0067c0', dark2: '#003e92', dark3: '#001a68',
};

/**
 * Win11-like accent palette from a base colour (the swatch the user picked).
 * Light mode fills use dark1, dark mode fills use light2 (like Windows).
 */
export function accentPalette(input) {
  const hex = normalizeHex(input) || '#0078d4';
  let p;
  if (hex === '#0078d4' || hex === '#0067c0') {
    p = { ...WINDOWS_BLUE };
  } else {
    const [L, C, H] = hexToOklch(hex);
    const mk = (l, cMul) => oklchToHex(Math.min(0.97, Math.max(0.12, l)), C * cMul, H);
    // Lightness steps calibrated on Windows' own blue palette (OKLCH), clamped so that
    // dark-mode fills stay light enough and light-mode fills dark enough.
    const up = (k) => L + (0.95 - L) * k;
    const l1 = Math.max(up(0.2), 0.6);
    const l2 = Math.max(up(0.55), 0.72);
    const l3 = Math.max(up(0.85), 0.86);
    const d1 = Math.min(L * 0.91, 0.6);
    const d2 = Math.min(L * 0.68, 0.47);
    const d3 = Math.min(L * 0.475, 0.34);
    p = {
      base: hex,
      light1: mk(l1, 1.02), light2: mk(l2, 0.86), light3: mk(l3, 0.55),
      dark1: mk(d1, 0.97), dark2: mk(d2, 0.9), dark3: mk(d3, 0.83),
    };
  }
  p.onLight = textOn(p.dark1, '#ffffff');
  p.onDark = textOn(p.light2, '#000000');
  return p;
}

/* ======================================================================
   Apply
   ====================================================================== */

function currentMode(s = settings) {
  if (forcedTheme) return forcedTheme;
  let mode = s?.theme?.mode;
  if (mode !== 'light' && mode !== 'dark') {
    try { mode = matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark'; } catch { mode = 'dark'; }
  }
  return mode;
}

function computeBlur(transparency) {
  if (forcedBlur) return forcedBlur;
  if (!transparency) return 'none';
  const blur = blurSource();
  // everything such a surface frosts lies in the page: without in-page blur it would be see-through
  // colour over sharp content, so it shows the "transparency effects off" fills instead
  return blur === 'css' && backdrop === 'none' ? 'none' : blur;
}

/** Where the frosting comes from on this surface, given transparency on. */
function blurSource() {
  if (surfaceKind === 'preview' || surfaceKind === 'gallery') return 'css';
  const hypr = shellInfo?.compositor === 'hyprland';
  if (surfaceKind === 'app') {
    // Embedded in the preview WM: the WM window behind the iframe blurs (like a compositor).
    if (bridge.embedded) return 'compositor';
    return hypr ? 'compositor' : 'none';
  }
  if (surfaceKind === 'desktop') return 'css'; // nothing is behind the desktop layer; blur the wallpaper in-page
  return hypr ? 'compositor' : 'css';          // shell (and anything else)
}

function setVar(name, value) { html.style.setProperty(name, value); }

function applyPalette(pal, mode) {
  setVar('--accent-base', pal.base);
  setVar('--accent-light-1', pal.light1);
  setVar('--accent-light-2', pal.light2);
  setVar('--accent-light-3', pal.light3);
  setVar('--accent-dark-1', pal.dark1);
  setVar('--accent-dark-2', pal.dark2);
  setVar('--accent-dark-3', pal.dark3);
  setVar('--accent-on-light', pal.onLight);
  setVar('--accent-on-dark', pal.onDark);
  setVar('--accent', mode === 'dark' ? pal.light2 : pal.dark1);
  setVar('--accent-text', mode === 'dark' ? pal.onDark : pal.onLight);
}

let lastPaletteKey = '';
let lastPalette = null;

function apply(s) {
  const t = s?.theme || {};
  const mode = currentMode(s);
  const transparency = forcedTransparency ? forcedTransparency === 'on' : t.transparency !== false;
  html.dataset.surface = surfaceKind;
  html.dataset.theme = mode;
  html.dataset.transparency = transparency ? 'on' : 'off';
  html.dataset.blur = computeBlur(transparency);
  html.dataset.backdrop = backdrop;
  applyNativeBackdrop(); requestNativeBackdrop();
  if (bridge.embedded) html.dataset.embedded = 'true';

  const key = String(t.accent || '#0078d4');
  if (key !== lastPaletteKey || !lastPalette) { lastPalette = accentPalette(key); lastPaletteKey = key; }
  applyPalette(lastPalette, mode);

  const ts = Math.min(225, Math.max(100, Number(s?.accessibility?.textScale) || 100)) / 100;
  setVar('--ts', String(ts));
  const rm = !!s?.accessibility?.reduceMotion;
  html.dataset.reduceMotion = rm ? 'true' : 'false';

  const language = forcedLang || s?.language || 'nl';
  setLang(language);
  try { html.lang = language; } catch { /* ignore */ }

  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify({ mode, transparency, accent: key, ts, rm, palette: lastPalette,
      lang: s?.language === 'en' ? 'en' : 'nl' }));      // the settings.get-failed path keeps this language
  } catch { /* ignore */ }
}

/** Apply the last known theme instantly (before settings arrive) to avoid flashes. */
function applyCached() {
  let c = null;
  try { c = JSON.parse(localStorage.getItem(CACHE_KEY) || 'null'); } catch { c = null; }
  const mode = forcedTheme || c?.mode || (matchMedia?.('(prefers-color-scheme: light)').matches ? 'light' : 'dark');
  const transparency = forcedTransparency ? forcedTransparency === 'on' : c?.transparency !== false;
  html.dataset.surface = surfaceKind;
  html.dataset.theme = mode;
  html.dataset.transparency = transparency ? 'on' : 'off';
  html.dataset.blur = computeBlur(transparency);
  html.dataset.backdrop = backdrop;
  const pal = c?.palette?.dark1 ? c.palette : accentPalette(c?.accent || '#0067c0');
  applyPalette(pal, mode);
  if (c?.ts) setVar('--ts', String(c.ts));
  html.dataset.reduceMotion = c?.rm ? 'true' : 'false';
}

function notify(path) {
  notified = JSON.stringify(settings);
  for (const cb of [...subs]) {
    try { cb(settings, path ?? null); } catch (e) { console.error('[theme] onSettings callback failed', e); }
  }
}

/* ======================================================================
   Public API
   ====================================================================== */

export function initTheme(kind) {
  if (initPromise) return initPromise;
  if (kind) surfaceKind = kind;
  applyCached();
  initPromise = (async () => {
    const [s, info] = await Promise.allSettled([bridge.call('settings.get'), bridge.call('shell.info')]);
    shellInfo = info.status === 'fulfilled' ? info.value : { mode: 'serve', compositor: 'other', user: {} };
    if (s.status === 'fulfilled' && s.value && typeof s.value === 'object') settings = s.value;
    else {
      console.warn('[theme] settings.get failed; using defaults', s.reason);
      settings = JSON.parse(JSON.stringify(FALLBACK));
      try { if (JSON.parse(localStorage.getItem(CACHE_KEY) || 'null')?.lang === 'en') settings.language = 'en'; } catch { /* nl */ }
    }
    apply(settings);
    notified = JSON.stringify(settings);

    bridge.on('settings.changed', (data) => {
      evSeq++;
      if (data && data.settings) {
        settings = data.settings;
        apply(settings);
        if (JSON.stringify(settings) !== notified) notify(data.path);
      } else {
        reloadSettings();
      }
    });
    try {
      matchMedia('(prefers-color-scheme: light)').addEventListener('change', () => {
        const m = settings?.theme?.mode;
        if (m !== 'light' && m !== 'dark') { apply(settings); notify('theme.mode'); }
      });
    } catch { /* ignore */ }
    return settings;
  })();
  return initPromise;
}

export async function reloadSettings() {
  try {
    settings = await bridge.call('settings.get');
    apply(settings);
    notify(null);
  } catch (e) { console.warn('[theme] reload failed', e); }
  return settings;
}

/** Cached settings (populated by initTheme). Safe to `await` too. */
export function getSettings() {
  return settings || JSON.parse(JSON.stringify(FALLBACK));
}

/** Read one value by dot path: getSetting('taskbar.alignment', 'center') */
export function getSetting(path, fallback) {
  const v = String(path).split('.').reduce((o, k) => (o == null ? undefined : o[k]), getSettings());
  return v === undefined ? fallback : v;
}

/** setSetting('theme.mode', 'light') → Promise<settings>. Applies optimistically. */
export async function setSetting(path, value) {
  const prev = settings;
  if (settings) {
    const next = JSON.parse(JSON.stringify(settings));
    const keys = String(path).split('.');
    let o = next;
    for (let i = 0; i < keys.length - 1; i++) {
      if (o[keys[i]] == null || typeof o[keys[i]] !== 'object') o[keys[i]] = {};
      o = o[keys[i]];
    }
    o[keys[keys.length - 1]] = value;
    settings = next;
    apply(settings);
    notify(path);
  }
  const seq = evSeq;
  try {
    const result = await bridge.call('settings.set', { path, value });
    // the event stream is ordered and ends with the latest state: when an event came in meanwhile, it (or
    // one still on its way) is at least as new as this reply. Without events (no SSE) the reply counts.
    if (result && typeof result === 'object' && evSeq === seq) {
      settings = result;
      apply(settings);
      if (JSON.stringify(settings) !== notified) notify(path);
    }
    return settings;
  } catch (e) {
    settings = prev;
    if (prev) { apply(prev); notify(path); }
    throw e;
  }
}

/** onSettings((settings, path) => …) → unsubscribe. path may be null (= many keys). */
export function onSettings(cb) {
  subs.add(cb);
  return () => subs.delete(cb);
}

/** shell.info() result, cached by initTheme (null before). */
export function getShellInfo() { return shellInfo; }

export function getSurface() { return surfaceKind; }

/** 'blur' | 'none': does in-page backdrop-filter paint on this page (see data-backdrop above)? */
export function getBackdrop() { return backdrop; }

/** 'light' | 'dark' currently in effect. */
export function themeMode() { return html.dataset.theme === 'light' ? 'light' : 'dark'; }

/**
 * The wallpaper path in effect: with followTheme, built-in bloom-… and flow-… use the twin
 * that matches the current light/dark mode. '' for a plain colour wallpaper.
 */
export function resolveWallpaperPath(s = settings) {
  const w = (s || FALLBACK).wallpaper || {};
  if (w.type === 'color') return '';
  let p = w.path || 'builtin:bloom-dark';
  if (w.followTheme !== false && p.startsWith('builtin:')) {
    const m = p.slice(8).match(/^(bloom|flow)-(light|dark)$/);
    if (m) p = `builtin:${m[1]}-${currentMode(s || settings)}`;
  }
  return p;
}

/** resolveWallpaper(settings) → url for CSS/img ('' when wallpaper.type === 'color'). */
export function resolveWallpaper(s = settings) {
  const p = resolveWallpaperPath(s);
  return p ? bridge.fileUrl(p) : '';
}
