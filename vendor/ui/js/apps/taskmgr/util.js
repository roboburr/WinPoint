// Taakbeheer — shared helpers: formatting, colours, small glyphs, persistence.

import { h } from '../../core/dom.js';
import { number, bytes } from '../../core/format.js';
import { getSetting } from '../../core/theme.js';
import { CHART_COLORS } from '../../core/charts.js';
import { lang } from '../../core/i18n.js';

const NS = 'http://www.w3.org/2000/svg';

/* ------------------------------------------------------------ persistence */
const PREFIX = 'win11shell.taskmgr.';
export function load(key, fallback) {
  try {
    const v = localStorage.getItem(PREFIX + key);
    return v == null ? fallback : JSON.parse(v);
  } catch { return fallback; }
}
export function save(key, value) {
  try { localStorage.setItem(PREFIX + key, JSON.stringify(value)); } catch { /* ignore */ }
}

/* ------------------------------------------------------------- formatting */
/** cpu(3.456) → "3,5%" ; cpu(0.04) → "0%" (Task Manager style) */
export function cpuPct(v) {
  if (v == null || !isFinite(v)) return '–';
  if (v < 0.05) return '0%';
  return `${number(v, 1, 1)}%`;
}
/** Memory in MB with one decimal, like Task Manager ("245,3 MB"); ≥ 10 GB → GB. */
export function memMB(n) {
  if (n == null || !isFinite(n)) return '–';
  const mb = n / 1048576;
  if (mb >= 10240) return `${number(mb / 1024, 1, 1)} GB`;
  return `${number(mb, 1, 1)} MB`;
}
/** Disk rate in MB/s, Task Manager style ("0 MB/s", "0,1 MB/s", "12,4 MB/s"). */
export function diskMBs(n) {
  if (n == null || !isFinite(n)) return '–';
  const mb = n / 1048576;
  if (mb < 0.05) return '0 MB/s';
  return `${number(mb, 1, 1)} MB/s`;
}
export const gb = (n) => bytes(n);
/** How full a drive is (0–100): the host's df-style `percent` = used / (used + free) — the figure Verkenner,
 *  Instellingen and the health check use too (ext4's 5 % root reserve counts as taken). */
export function volPct(v) {
  if (!v) return 0;
  const p = Number.isFinite(v.percent) ? v.percent : ((v.used || 0) + (v.free || 0) > 0 ? (v.used / (v.used + v.free)) * 100 : 0);
  return Math.min(100, Math.max(0, p));
}
export function ghz(mhz) { return mhz ? `${number(mhz / 1000, 2, 2)} GHz` : '–'; }
export function pctInt(v) { return v == null || !isFinite(v) ? '–' : `${Math.round(v)}%`; }
export const isEn = () => lang() === 'en';

/* ---------------------------------------------------------------- colours */
export const KIND_COLOR = {
  cpu: CHART_COLORS.cpu, memory: CHART_COLORS.memory, disk: CHART_COLORS.disk,
  network: CHART_COLORS.network, gpu: CHART_COLORS.gpu,
};
/** Graph colour for a device kind (accent when colour-keyed graphs are off). */
export function kindColor(kind) {
  return getSetting('taskmgr.colorKeyedGraphs', true) ? (KIND_COLOR[kind] || 'var(--accent)') : 'var(--accent)';
}

/** 0..1 → heat bucket 0..10 (Win11 pale-yellow → orange cells). */
export function heatBucket(f) {
  if (f == null || !isFinite(f)) return 0;
  return Math.max(0, Math.min(10, Math.round(Math.pow(Math.max(0, Math.min(1, f)), 0.75) * 10)));
}

/** Category colours (storage). Work on light and dark. */
export const CATEGORY_COLORS = {
  documents: 'var(--chart-1)', pictures: 'var(--chart-3)', videos: 'var(--chart-2)', music: 'var(--chart-4)',
  downloads: 'var(--chart-5)', desktop: 'var(--chart-7)', trash: 'var(--chart-8)', browser: 'var(--tm-cat-browser)',
  chat: 'var(--tm-cat-chat)', dev: 'var(--chart-6)', appdata: 'var(--tm-cat-appdata)', snaps: 'var(--tm-cat-snaps)',
  flatpak: 'var(--tm-cat-flatpak)', cache: 'var(--tm-cat-cache)', other: 'var(--tm-cat-other)',
};
/** Sunburst / treemap palette for top-level children. */
export const RING_COLORS = ['var(--chart-1)', 'var(--chart-2)', 'var(--chart-3)', 'var(--chart-5)', 'var(--chart-4)',
  'var(--chart-6)', 'var(--chart-7)', 'var(--tm-cat-chat)', 'var(--tm-cat-browser)', 'var(--tm-cat-flatpak)'];

/* ------------------------------------------------------------------ glyphs */
function svg(size, inner, cls = '') {
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 20 20');
  s.setAttribute('width', size);
  s.setAttribute('height', size);
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('class', `fl-icon ${cls}`.trim());
  s.innerHTML = inner;
  return s;
}
/** Efficiency mode leaf (green, filled). */
export function leafIcon(size = 16) {
  return svg(size, '<path d="M16.5 3.5c-6.6-.4-11.5 2.3-12.3 7.4-.3 1.8.3 3.4 1.3 4.5 1.1 1 2.7 1.5 4.4 1.2 5-.9 7-5.8 6.6-13.1Z" fill="var(--tm-leaf)" stroke="none"/><path d="M4.2 16.6c2.2-3.6 4.8-6.2 8.4-8.1" fill="none" stroke="var(--tm-leaf-vein)" stroke-width="1.3" stroke-linecap="round"/>', 'tm-leaf');
}
/** Outline leaf for buttons (currentColor). */
export function leafOutline(size = 16) {
  return svg(size, '<path d="M16.5 3.5c-6.6-.4-11.5 2.3-12.3 7.4-.3 1.8.3 3.4 1.3 4.5 1.1 1 2.7 1.5 4.4 1.2 5-.9 7-5.8 6.6-13.1Z" fill="none" stroke="currentColor" stroke-linejoin="round"/><path d="M3.6 17.2c2.2-3.8 5-6.6 8.9-8.7" fill="none" stroke="currentColor" stroke-linecap="round"/>');
}
/** End task (circle with a slash). */
export function endTaskIcon(size = 16) {
  return svg(size, '<circle cx="10" cy="10" r="7.25" fill="none" stroke="currentColor"/><path d="M5 15 15 5" fill="none" stroke="currentColor" stroke-linecap="round"/>');
}
export function stopIcon(size = 16) {
  return svg(size, '<rect x="5" y="5" width="10" height="10" rx="1.6" fill="none" stroke="currentColor"/>');
}
export function pauseIcon(size = 16) {
  return svg(size, '<path d="M7.25 5v10M12.75 5v10" fill="none" stroke="currentColor" stroke-linecap="round" stroke-width="1.6"/>');
}
/** Health status glyph: big round icon with a check / exclamation. */
export function statusGlyph(severity = 'ok', size = 48) {
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', '0 0 48 48');
  s.setAttribute('width', size);
  s.setAttribute('height', size);
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('class', `tm-status-glyph tm-status-glyph--${severity}`);
  const shield = 'M24 4.5 39 9.8c.9.3 1.5 1.2 1.5 2.2v9.7c0 9.6-6.4 18.2-15.8 21.6-.5.2-1 .2-1.4 0C14 39.9 7.5 31.3 7.5 21.7V12c0-1 .6-1.9 1.5-2.2Z';
  const mark = severity === 'ok'
    ? '<path d="M16.5 24.2 21.6 29.3 31.8 18.6" fill="none" stroke="var(--tm-glyph-fg)" stroke-width="3.2" stroke-linecap="round" stroke-linejoin="round"/>'
    : '<path d="M24 15.5v10.5" stroke="var(--tm-glyph-fg)" stroke-width="3.4" stroke-linecap="round"/><circle cx="24" cy="32.2" r="2.1" fill="var(--tm-glyph-fg)"/>';
  s.innerHTML = `<defs><linearGradient id="tmg-${severity}" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--tm-glyph-a)"/><stop offset="1" stop-color="var(--tm-glyph-b)"/></linearGradient></defs>`
    + `<path d="${shield}" fill="url(#tmg-${severity})"/><path d="${shield}" fill="none" stroke="rgba(255,255,255,.25)"/>${mark}`;
  return s;
}

/* ------------------------------------------------------------------ misc */
/** The executable path of a command line ('' when not absolute). */
export function exePath(cmd) {
  const s = String(cmd || '').trim();
  if (!s.startsWith('/')) return '';
  // paths may contain spaces when quoted; take up to the first " -" or space
  const m = s.match(/^(\/[^ ]+)/);
  return m ? m[1] : '';
}
export function dirName(path) {
  const p = String(path || '');
  const i = p.lastIndexOf('/');
  return i > 0 ? p.slice(0, i) : '/';
}
export function baseName(path) {
  const p = String(path || '').replace(/\/+$/, '');
  return p.slice(p.lastIndexOf('/') + 1) || '/';
}
/** Short, friendly display of a path: /home/robin/… → ~/… */
export function prettyPath(path, home = '/home/') {
  const p = String(path || '');
  const m = p.match(/^\/home\/[^/]+/);
  return m ? '~' + p.slice(m[0].length) : p;
}

/** Wrap an element so a tooltip also shows while it is disabled. */
export function tipWrap(el, text) {
  const w = h('span', { class: 'tm-tipwrap' }, el);
  w.setTip = (s) => { if (s) w.dataset.tooltip = s; else delete w.dataset.tooltip; };
  w.setTip(text);
  return w;
}

/** Animate a number change smoothly (only text). */
export function setText(el, text) {
  if (el.textContent !== text) el.textContent = text;
}

export const sum = (arr, f = (x) => x) => arr.reduce((a, x) => a + (f(x) || 0), 0);
