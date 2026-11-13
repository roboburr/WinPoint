// Win11 Shell — Instellingen: shared helpers for the pages (owned by the settings agent).
//
// Pages are `async (ctx) => Node | Node[]`. ctx (built by main.js):
//   ctx.go(id, params?)  ctx.on(event, cb)  ctx.onSettings(cb)  ctx.onCleanup(fn)
//   ctx.interval(fn, ms) ctx.alive()  ctx.refresh()  ctx.params  ctx.frame  ctx.page
// Helpers here bind Fluent controls to settings paths, wrap API calls with friendly
// errors and draw the small previews (mini desktop, monitor) used on several pages.

import { h, clear } from '../../core/dom.js';
import { bridge, builtinWallpaper } from '../../core/bridge.js';
import { makeT, tr, lang } from '../../core/i18n.js';
import {
  getSettings, getSetting, setSetting, resolveWallpaper, accentPalette, themeMode, hexToOklch, oklchToHex, normalizeHex,
} from '../../core/theme.js';
import { SettingsCard, ToggleSwitch, Dropdown, Toast, InfoBar, iconNode } from '../../core/components.js';
import { settingsPage } from './registry.js';

export const ct = makeT({
  nl: {
    failed: 'Dat lukte niet',
    unavailable: 'Niet beschikbaar',
    unavailableHere: 'Dit is op deze computer niet beschikbaar.',
    loading: 'Even geduld…',
    retry: 'Opnieuw proberen',
    related: 'Gerelateerde instellingen',
    saved: 'Opgeslagen',
    dryRun: 'Testmodus: er is niets echt gewijzigd.',
    copied: 'Gekopieerd naar het klembord',
    copyFailed: 'Kopiëren lukte niet',
    on: 'Aan', off: 'Uit',
    mockNote: 'Testomgeving',
  },
  en: {
    failed: 'That didn’t work',
    unavailable: 'Not available',
    unavailableHere: 'This isn’t available on this computer.',
    loading: 'Just a moment…',
    retry: 'Try again',
    related: 'Related settings',
    saved: 'Saved',
    dryRun: 'Test mode: nothing was actually changed.',
    copied: 'Copied to the clipboard',
    copyFailed: 'Couldn’t copy',
    on: 'On', off: 'Off',
    mockNote: 'Test environment',
  },
});

/* ------------------------------------------------------------------ API */

/** call(method, args) → {ok, value, error} (never throws). */
export async function call(method, args = {}) {
  try {
    const value = await bridge.call(method, args);
    return { ok: true, value };
  } catch (error) {
    return { ok: false, error };
  }
}

/** get(method, args, fallback) → result or fallback (logs unexpected errors). */
export async function get(method, args = {}, fallback = null) {
  const r = await call(method, args);
  if (!r.ok && r.error?.code !== 'unknown_method' && r.error?.code !== 'unsupported') {
    console.warn(`[settings] ${method} failed`, r.error);
  }
  return r.ok ? r.value : fallback;
}

/** Friendly error toast. */
export function showError(error, title = ct('failed')) {
  const message = error?.code === 'unknown_method' ? ct('unavailableHere') : (error?.message || '');
  Toast.show({ severity: 'error', title, message, timeout: 6000 });
}

/** Toast after a dangerous call that ran in --dry-run. */
export function noteDryRun(result) {
  if (result && result.dryRun) Toast.show({ severity: 'info', title: ct('dryRun'), timeout: 3500 });
}

/** run(method, args, {errorTitle}) → result | null; shows an error toast when it fails. */
export async function run(method, args = {}, { errorTitle, quietDry = false } = {}) {
  const r = await call(method, args);
  if (!r.ok) { showError(r.error, errorTitle); return null; }
  if (!quietDry) noteDryRun(r.value);
  return r.value ?? {};
}

/* ------------------------------------------------------- setting bindings */

/** ToggleSwitch bound to a settings path (invert: the switch shows !value). */
export function settingToggle(ctx, path, { invert = false, onAfter, ariaLabel, disabled = false } = {}) {
  const read = () => { const v = !!getSetting(path, false); return invert ? !v : v; };
  const sw = ToggleSwitch({
    checked: read(), ariaLabel, disabled,
    onChange: async (on) => {
      try {
        await setSetting(path, invert ? !on : on);
        onAfter?.(on);
      } catch (e) {
        sw.setChecked(read());
        showError(e);
      }
    },
  });
  ctx?.onSettings(() => sw.setChecked(read()));
  return sw;
}

/** Dropdown bound to a settings path. options: [{value, label}] */
export function settingDropdown(ctx, path, options, { width, onAfter, ariaLabel, fallback } = {}) {
  const read = () => getSetting(path, fallback ?? options[0]?.value);
  const dd = Dropdown({
    options, value: read(), width, ariaLabel,
    onChange: async (v) => {
      try {
        await setSetting(path, v);
        onAfter?.(v);
      } catch (e) {
        dd.setValue(read());
        showError(e);
      }
    },
  });
  ctx?.onSettings(() => dd.setValue(read()));
  return dd;
}

/** SettingsCard with a toggle bound to a settings path. */
export function toggleCard(ctx, { icon, title, description, path, invert, onAfter, disabled }) {
  const sw = settingToggle(ctx, path, { invert, onAfter, ariaLabel: typeof title === 'string' ? title : null, disabled });
  const card = SettingsCard({ icon, title, description, control: sw });
  card.toggle = sw;
  return card;
}

/** SettingsCard with a dropdown bound to a settings path. */
export function dropdownCard(ctx, { icon, title, description, path, options, width, onAfter, fallback }) {
  const dd = settingDropdown(ctx, path, options, { width, onAfter, ariaLabel: typeof title === 'string' ? title : null, fallback });
  const card = SettingsCard({ icon, title, description, control: dd });
  card.dropdown = dd;
  return card;
}

/** Clickable card that opens another Settings page (title/description/icon from the registry). */
export function linkCard(ctx, id, { title, description, icon, control } = {}) {
  const p = settingsPage(id);
  return SettingsCard({
    icon: icon ?? p?.icon, title: title ?? tr(p?.title), description: description ?? tr(p?.description), control,
    onClick: () => ctx.go(id),
  });
}

/** Non-closable InfoBar explaining why something is not available. */
export function notice({ severity = 'info', title = '', message = '', action } = {}) {
  const bar = InfoBar({ severity, title, message, action, closable: false });
  bar.classList.add('st-notice');
  return bar;
}

/** A disabled card that says "Niet beschikbaar" (keeps the page layout like Windows). */
export function unavailableCard({ icon, title, description }) {
  const c = SettingsCard({ icon, title, description: description || ct('unavailableHere'), control: h('span', { class: 'text-3' }, ct('unavailable')) });
  c.classList.add('is-unavailable');
  return c;
}

/* -------------------------------------------------------------- helpers */

const COLOR_NAMES = {
  nl: { gray: 'Grijs', red: 'Rood', orange: 'Oranje', yellow: 'Geel', green: 'Groen', teal: 'Groenblauw', blue: 'Blauw', purple: 'Paars', pink: 'Roze', brown: 'Bruin', black: 'Zwart', white: 'Wit', dark: 'Donker', light: 'Licht' },
  en: { gray: 'Grey', red: 'Red', orange: 'Orange', yellow: 'Yellow', green: 'Green', teal: 'Teal', blue: 'Blue', purple: 'Purple', pink: 'Pink', brown: 'Brown', black: 'Black', white: 'White', dark: 'Dark', light: 'Light' },
};

/** A friendly name for a colour ("Donker blauw") instead of a hex code. */
export function colorName(hex) {
  const n = normalizeHex(hex);
  const names = COLOR_NAMES[lang()] || COLOR_NAMES.nl;
  if (!n) return '';
  const v = parseInt(n.slice(1), 16);
  const r = ((v >> 16) & 255) / 255, g = ((v >> 8) & 255) / 255, b = (v & 255) / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  const l = (max + min) / 2, s = d ? d / (1 - Math.abs(2 * l - 1)) : 0;
  if (l < 0.1) return names.black;
  if (l > 0.94) return names.white;
  let hue = 0;
  if (d) { hue = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4; hue = (hue * 60 + 360) % 360; }
  let base;
  if (s < 0.15) base = names.gray;
  else if (hue < 15 || hue >= 345) base = names.red;
  else if (hue < 40) base = l < 0.42 ? names.brown : names.orange;
  else if (hue < 65) base = l < 0.4 ? names.brown : names.yellow;
  else if (hue < 160) base = names.green;
  else if (hue < 195) base = names.teal;
  else if (hue < 255) base = names.blue;
  else if (hue < 295) base = names.purple;
  else base = names.pink;
  const mod = l < 0.3 && base !== names.brown ? names.dark : l > 0.72 ? names.light : '';
  return mod ? `${mod} ${base.toLowerCase()}` : base;
}

export async function copyText(text) {
  try {
    if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(text); return true; }
  } catch { /* fall through */ }
  try {
    const ta = h('textarea', { style: { position: 'fixed', left: '-9999px', top: '0' } });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  } catch { return false; }
}

/** "Ubuntu 26.04.1 LTS" + "26.04.1 LTS (Resolute Raccoon)" → "Ubuntu 26.04.1 LTS (Resolute Raccoon)" */
export function osLabel(os, ver) {
  if (!os) return ver || '';
  if (!ver) return os;
  const v = String(ver).split(' ')[0];
  if (String(os).includes(v)) {
    const rest = String(ver).slice(v.length).trim().replace(/^LTS\s*/, '');
    return rest && !String(os).includes(rest) ? `${os} ${rest}` : os;
  }
  return `${os} ${ver}`;
}

/** Wallpaper path → URL usable for a thumbnail. */
export function wallpaperUrl(path, size = 512) {
  if (!path) return '';
  if (String(path).startsWith('builtin:')) return bridge.fileUrl(path);
  return bridge.thumbUrl(path, size) || bridge.fileUrl(path);
}

/** builtin lotus-/flow- twin for a mode (followTheme); an old builtin:bloom-… id means lotus. */
export function twinFor(path, mode, follow = true) {
  path = builtinWallpaper(path);
  if (!follow || !String(path).startsWith('builtin:')) return path;
  const m = String(path).slice(8).match(/^(lotus|flow)-(light|dark)$/);
  return m ? `builtin:${m[1]}-${mode}` : path;
}

export function accentFor(hex, mode = themeMode()) {
  const p = accentPalette(hex);
  return { fill: mode === 'dark' ? p.light2 : p.dark1, text: mode === 'dark' ? p.onDark : p.onLight, palette: p };
}

const FIT_CSS = {
  fill: { size: 'cover', repeat: 'no-repeat' },
  fit: { size: 'contain', repeat: 'no-repeat' },
  stretch: { size: '100% 100%', repeat: 'no-repeat' },
  tile: { size: '22%', repeat: 'repeat' },
  center: { size: '46%', repeat: 'no-repeat' },
  span: { size: 'cover', repeat: 'no-repeat' },
};

/**
 * DesktopPreview — a mini Windows 11 desktop: wallpaper, a small window and the taskbar,
 * rendered in the given mode and accent. update({settings, mode, accent, url, color, fit}).
 */
export function DesktopPreview(opts = {}) {
  const win = h('div', { class: 'st-dp__win' },
    h('div', { class: 'st-dp__bar' }, h('i'), h('i'), h('i')),
    h('div', { class: 'st-dp__body' },
      h('div', { class: 'st-dp__aa' }, 'Aa'),
      h('div', { class: 'st-dp__lines' }, h('i'), h('i'), h('i')),
      h('div', { class: 'st-dp__btn' })));
  const taskbar = h('div', { class: 'st-dp__taskbar' },
    h('span', { class: 'st-dp__start' }, h('i'), h('i'), h('i'), h('i')), h('span'), h('span'), h('span'), h('span'));
  const el = h('div', { class: ['st-dp', opts.cls], 'aria-hidden': 'true' }, win, taskbar);
  el.update = (o = {}) => {
    const s = o.settings || getSettings();
    const mode = o.mode || themeMode();
    const accent = accentFor(o.accent || s.theme?.accent || '#0078d4', mode);
    const url = o.url !== undefined ? o.url : resolveWallpaper(s);
    const fit = FIT_CSS[o.fit || s.wallpaper?.fit] || FIT_CSS.fill;
    el.dataset.theme = mode;
    el.style.setProperty('--dp-accent', accent.fill);
    el.style.setProperty('--dp-accent-text', accent.text);
    el.style.backgroundColor = o.color || s.wallpaper?.color || '#0b2a4a';
    el.style.backgroundImage = url ? `url("${url}")` : 'none';
    el.style.backgroundSize = fit.size;
    el.style.backgroundRepeat = fit.repeat;
    el.classList.toggle('no-window', o.window === false);
    el.classList.toggle('accent-taskbar', !!(o.accentTaskbar ?? (s.theme?.accentOnTaskbar && mode === 'dark')));
  };
  el.update(opts);
  return el;
}

/** DeviceArt — a monitor showing the wallpaper (System / Home hero). */
export function DeviceArt({ settings, small = false } = {}) {
  const screen = h('div', { class: 'st-device__screen' });
  const el = h('div', { class: ['st-device', small && 'is-small'], 'aria-hidden': 'true' },
    h('div', { class: 'st-device__bezel' }, screen), h('div', { class: 'st-device__neck' }), h('div', { class: 'st-device__foot' }));
  el.update = (s = getSettings()) => {
    const url = resolveWallpaper(s);
    screen.style.backgroundColor = s.wallpaper?.color || '#0b2a4a';
    screen.style.backgroundImage = url ? `url("${url}")` : 'none';
  };
  el.update(settings);
  return el;
}

/** Hero row (image left, text, optional right side). */
export function Hero({ art, title, lines = [], actions, aside, cls } = {}) {
  return h('section', { class: ['st-hero', cls] },
    art ? h('div', { class: 'st-hero__art' }, art) : null,
    h('div', { class: 'st-hero__text' },
      title ? h('div', { class: 'st-hero__title' }, title) : null,
      ...lines.filter(Boolean).map((l) => h('div', { class: 'st-hero__line' }, l)),
      actions ? h('div', { class: 'st-hero__actions' }, actions) : null),
    aside ? h('div', { class: 'st-hero__aside' }, aside) : null);
}

/** Small status tile (used on the right of heroes): icon, title, status, onClick. */
export function HeroTile({ icon, title, status, onClick }) {
  const el = h(onClick ? 'button' : 'div', { type: onClick ? 'button' : null, class: ['st-tile', onClick && 'is-clickable'] },
    h('span', { class: 'st-tile__icon' }, iconNode(icon, 24)),
    h('span', { class: 'st-tile__text' },
      h('span', { class: 'st-tile__title' }, title),
      ...[].concat(status || '').map((s) => h('span', { class: 'st-tile__status' }, s))));
  if (onClick) el.addEventListener('click', onClick);
  return el;
}

/** A section header like Win11 ("Gerelateerde instellingen"). */
export function sectionTitle(text) {
  return h('h3', { class: 'st-section-title' }, text);
}

/** How full a drive is (0–100), the same figure Taakbeheer and Verkenner show: the host's df-style `percent`
 *  = used / (used + free), so space reserved for the system (ext4 keeps 5 %) counts as taken. */
export function volumePercent(v) {
  if (!v) return 0;
  const p = Number.isFinite(v.percent) ? v.percent : ((v.used || 0) + (v.free || 0) > 0 ? (v.used / (v.used + v.free)) * 100 : 0);
  return Math.min(100, Math.max(0, p));
}

/** Thin usage bar (Win11 storage style). pct 0–100; red when ≥ 90. */
export function UsageBar({ pct = 0, color, warnAt = 90 } = {}) {
  const fill = h('span', { class: 'st-usage__fill' });
  const el = h('span', { class: 'st-usage', role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': 100 }, fill);
  el.set = (p, c = color) => {
    const v = Math.max(0, Math.min(100, Number(p) || 0));
    fill.style.width = `${v}%`;
    fill.style.background = c || (v >= warnAt ? 'var(--sys-critical)' : 'var(--accent)');
    el.setAttribute('aria-valuenow', String(Math.round(v)));
  };
  el.set(pct, color);
  return el;
}

/* --------------------------------------------- accent from the wallpaper */

/**
 * Average dominant colour of an image URL → a Windows-like accent hex (or null).
 * Saturated, mid-bright pixels count most; hue buckets pick the dominant tone.
 */
export async function accentFromImage(url) {
  if (!url) return null;
  const img = new Image();
  img.decoding = 'async';
  const loaded = new Promise((res, rej) => { img.onload = res; img.onerror = rej; });
  img.src = url;
  try { await loaded; } catch { return null; }
  try {
    const c = document.createElement('canvas');
    c.width = 64; c.height = 40;
    const g = c.getContext('2d', { willReadFrequently: true });
    g.drawImage(img, 0, 0, c.width, c.height);
    const { data } = g.getImageData(0, 0, c.width, c.height);
    const buckets = Array.from({ length: 18 }, () => ({ w: 0, r: 0, g: 0, b: 0 }));
    for (let i = 0; i < data.length; i += 4) {
      const r = data[i], gg = data[i + 1], b = data[i + 2];
      const max = Math.max(r, gg, b), min = Math.min(r, gg, b);
      const v = max / 255, s = max ? (max - min) / max : 0;
      if (s < 0.18 || v < 0.15) continue;
      let hue = 0;
      const d = max - min;
      if (d) {
        if (max === r) hue = ((gg - b) / d) % 6;
        else if (max === gg) hue = (b - r) / d + 2;
        else hue = (r - gg) / d + 4;
        hue = (hue * 60 + 360) % 360;
      }
      const w = s * s * (1 - Math.abs(v - 0.65));
      const k = buckets[Math.floor(hue / 20) % 18];
      k.w += w; k.r += r * w; k.g += gg * w; k.b += b * w;
    }
    const best = buckets.reduce((a, b) => (b.w > a.w ? b : a));
    if (!best.w) return null;
    const hex = '#' + [best.r, best.g, best.b].map((x) => Math.round(x / best.w).toString(16).padStart(2, '0')).join('');
    const [L, C, H] = hexToOklch(hex);
    return normalizeHex(oklchToHex(Math.min(0.62, Math.max(0.48, L)), Math.max(0.1, Math.min(0.2, C)), H));
  } catch (e) {
    console.warn('[settings] accentFromImage failed', e);
    return null;
  }
}

/* ------------------------------------------------ recently opened pages */

const RECENT_KEY = 'win11shell.settings.recent';

/** Recently opened sub-pages (per viewer; feeds "Aanbevolen instellingen" on the home page). */
export function recentPages() {
  let list = null;
  try { list = JSON.parse(localStorage.getItem(RECENT_KEY) || 'null'); } catch { list = null; }
  return Array.isArray(list) ? list.filter((id) => settingsPage(id)) : [];
}
export function recordVisit(id) {
  const p = settingsPage(id);
  if (!p || !p.parent) return;
  const list = [id, ...recentPages().filter((x) => x !== id)].slice(0, 6);
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(list)); } catch { /* ignore */ }
}

export { clear };
