// Win11 Shell — Verkenner: helpers (paths, friendly names, sizes, dates, icons, sorting).

import { h, clear } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { bytes, number, date, time } from '../../core/format.js';
import { lang, collator } from '../../core/i18n.js';
import { t } from './i18n.js';

/* ---------------------------------------------------------------- paths */

export function normPath(p) {
  if (!p) return '';
  const abs = p.startsWith('/');
  const out = [];
  for (const seg of p.split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') { out.pop(); continue; }
    out.push(seg);
  }
  return (abs ? '/' : '') + out.join('/') || '/';
}

export function basename(p) {
  if (!p) return '';
  const s = p.length > 1 ? p.replace(/\/+$/, '') : p;
  if (s === '/') return '/';
  return s.slice(s.lastIndexOf('/') + 1);
}

export function dirname(p) {
  if (!p || p === '/') return '/';
  const s = p.replace(/\/+$/, '');
  const i = s.lastIndexOf('/');
  return i <= 0 ? '/' : s.slice(0, i);
}

export const joinPath = (dir, name) => (dir.endsWith('/') ? dir + name : `${dir}/${name}`);
export const isUnder = (p, dir) => !!p && !!dir && (p === dir || p.startsWith(dir === '/' ? '/' : dir + '/'));

/** 'foto.tar.gz' → ['foto', '.tar.gz']; '.bashrc' → ['.bashrc', ''] */
export function splitExt(name = '') {
  if (!name || (name.startsWith('.') && name.indexOf('.', 1) < 0)) return [name, ''];
  const m = name.match(/^(.+?)(\.tar\.(?:gz|bz2|xz|zst)|\.[^.\s]{1,12})$/i);
  return m ? [m[1], m[2]] : [name, ''];
}

export const extOf = (name) => splitExt(name)[1].replace(/^\./, '').toLowerCase();

/** file:// URI for a path (for drag & drop to other apps). */
export function fileUri(p) {
  return 'file://' + p.split('/').map((s) => encodeURIComponent(s)).join('/');
}
export function uriToPath(u) {
  try {
    const url = new URL(u.trim());
    if (url.protocol !== 'file:') return null;
    return decodeURIComponent(url.pathname);
  } catch { return null; }
}

/* ------------------------------------------------------------- display */

export function displayName(item, showExt = true) {
  if (!item) return '';
  if ((item.isDesktopEntry || item.inTrash || item.trashName) && item.displayName) return item.displayName;
  const name = item.name || basename(item.path);
  if (showExt || item.isDir) return name;
  const [stem, ext] = splitExt(name);
  return ext && stem ? stem : name;
}

const TYPES_NL = {
  txt: 'Tekstdocument', md: 'Markdown-document', log: 'Logbestand', rtf: 'RTF-document', pdf: 'PDF-document',
  doc: 'Word-document', docx: 'Word-document', odt: 'OpenDocument-tekst', xls: 'Excel-werkblad', xlsx: 'Excel-werkblad',
  ods: 'OpenDocument-werkblad', csv: 'CSV-bestand', ppt: 'PowerPoint-presentatie', pptx: 'PowerPoint-presentatie',
  odp: 'OpenDocument-presentatie', jpg: 'JPG-bestand', jpeg: 'JPG-bestand', png: 'PNG-bestand', gif: 'GIF-bestand',
  webp: 'WebP-afbeelding', svg: 'SVG-afbeelding', bmp: 'BMP-bestand', heic: 'HEIC-afbeelding', tif: 'TIFF-afbeelding',
  tiff: 'TIFF-afbeelding', ico: 'Pictogram', mp3: 'MP3-bestand', wav: 'WAV-geluid', flac: 'FLAC-audio', ogg: 'OGG-audio',
  m4a: 'MPEG-4-audio', opus: 'Opus-audio', mp4: 'MP4-video', mkv: 'MKV-video', avi: 'AVI-video', mov: 'QuickTime-film',
  webm: 'WebM-video', zip: 'Gecomprimeerde (gezipte) map', '7z': '7-Zip-archief', rar: 'RAR-archief', tar: 'TAR-archief',
  'tar.gz': 'TAR.GZ-archief', 'tar.xz': 'TAR.XZ-archief', 'tar.bz2': 'TAR.BZ2-archief', 'tar.zst': 'TAR.ZST-archief',
  gz: 'GZ-archief', xz: 'XZ-archief', deb: 'Softwarepakket', rpm: 'Softwarepakket', appimage: 'AppImage-programma',
  exe: 'Windows-programma', msi: 'Windows-installatiepakket', sh: 'Shellscript', py: 'Python-bestand',
  js: 'JavaScript-bestand', html: 'HTML-document', htm: 'HTML-document', css: 'CSS-bestand', json: 'JSON-bestand',
  xml: 'XML-document', iso: 'Schijfkopiebestand', img: 'Schijfkopiebestand', desktop: 'Snelkoppeling', ttf: 'TrueType-lettertype',
  otf: 'OpenType-lettertype', torrent: 'Torrent-bestand', epub: 'E-boek', ini: 'Configuratie-instellingen', conf: 'Configuratiebestand',
};
const TYPES_EN = {
  txt: 'Text Document', md: 'Markdown Document', log: 'Log File', rtf: 'RTF Document', pdf: 'PDF Document',
  doc: 'Word Document', docx: 'Word Document', odt: 'OpenDocument Text', xls: 'Excel Worksheet', xlsx: 'Excel Worksheet',
  ods: 'OpenDocument Spreadsheet', csv: 'CSV File', ppt: 'PowerPoint Presentation', pptx: 'PowerPoint Presentation',
  odp: 'OpenDocument Presentation', zip: 'Compressed (zipped) Folder', deb: 'Software Package', rpm: 'Software Package',
  appimage: 'AppImage Program', exe: 'Windows Program', sh: 'Shell Script', desktop: 'Shortcut', iso: 'Disc Image File',
  html: 'HTML Document', htm: 'HTML Document', json: 'JSON File', py: 'Python File', js: 'JavaScript File',
  jpg: 'JPG File', jpeg: 'JPG File', png: 'PNG File', gif: 'GIF File', webp: 'WebP Image', svg: 'SVG Image', bmp: 'BMP File',
  heic: 'HEIC Image', tif: 'TIFF Image', tiff: 'TIFF Image', ico: 'Icon', mp3: 'MP3 File', wav: 'WAV Audio', flac: 'FLAC Audio',
  ogg: 'OGG Audio', m4a: 'MPEG-4 Audio', opus: 'Opus Audio', mp4: 'MP4 Video', mkv: 'MKV Video', avi: 'AVI Video',
  mov: 'QuickTime Movie', webm: 'WebM Video', '7z': '7-Zip Archive', rar: 'RAR Archive', tar: 'TAR Archive',
  'tar.gz': 'TAR.GZ Archive', 'tar.xz': 'TAR.XZ Archive', 'tar.bz2': 'TAR.BZ2 Archive', 'tar.zst': 'TAR.ZST Archive',
  gz: 'GZ Archive', xz: 'XZ Archive', msi: 'Windows Installer Package', css: 'CSS File', xml: 'XML Document',
  img: 'Disc Image File', ttf: 'TrueType Font', otf: 'OpenType Font', torrent: 'Torrent File', epub: 'E-book',
  ini: 'Configuration Settings', conf: 'Configuration File',
};

export function typeName(item) {
  if (!item) return '';
  if (item.isDir) return t('folder');
  if (item.typeName) return item.typeName;
  if (item.isDesktopEntry) return t('shortcut');
  const ext = extOf(item.name || basename(item.path));
  const table = lang() === 'en' ? TYPES_EN : TYPES_NL;
  if (ext && table[ext]) return table[ext];
  if (ext) return t('extFile', { ext: ext.toUpperCase() });
  if (item.mime === 'text/plain') return table.txt || 'Text Document';
  return t('file');
}

/** Details column size: '' for folders, "0 kB", "12 kB", "4,2 MB" (Windows style, Dutch commas). */
export function sizeText(item) {
  if (!item || item.isDir || item.size == null || !isFinite(item.size)) return '';
  const n = item.size;
  if (n === 0) return '0 kB';
  const kb = Math.ceil(n / 1024);
  if (kb < 1000) return `${number(kb)} kB`;
  return bytes(n);
}

export function bytesFull(n) {
  if (n == null || !isFinite(n)) return '–';
  return t('bytesFull', { h: bytes(n), b: number(n) });
}

/** "25-10-2026 14:03" */
export function dateTime(ms) {
  if (!ms || !isFinite(ms)) return '';
  return `${date(ms, 'short')} ${time(ms)}`;
}

/* --------------------------------------------------------------- groups */

const DAY = 86400000;
function startOfDay(d) { const x = new Date(d); x.setHours(0, 0, 0, 0); return x.getTime(); }

export function dateGroup(ms, now = Date.now()) {
  if (!ms) return { key: 'unknown', label: t('gUnknown'), order: 99 };
  const today = startOfDay(now);
  if (ms >= today + DAY) return { key: 'future', label: t('gFuture'), order: 0 };
  if (ms >= today) return { key: 'today', label: t('gToday'), order: 1 };
  if (ms >= today - DAY) return { key: 'yesterday', label: t('gYesterday'), order: 2 };
  const d = new Date(today);
  const dow = (d.getDay() + 6) % 7; // monday = 0
  const weekStart = today - dow * DAY;
  if (ms >= weekStart) return { key: 'thisWeek', label: t('gThisWeek'), order: 3 };
  if (ms >= weekStart - 7 * DAY) return { key: 'lastWeek', label: t('gLastWeek'), order: 4 };
  const monthStart = new Date(d.getFullYear(), d.getMonth(), 1).getTime();
  if (ms >= monthStart) return { key: 'thisMonth', label: t('gThisMonth'), order: 5 };
  const lastMonth = new Date(d.getFullYear(), d.getMonth() - 1, 1).getTime();
  if (ms >= lastMonth) return { key: 'lastMonth', label: t('gLastMonth'), order: 6 };
  const yearStart = new Date(d.getFullYear(), 0, 1).getTime();
  if (ms >= yearStart) return { key: 'thisYear', label: t('gThisYear'), order: 7 };
  return { key: 'older', label: t('gOlder'), order: 8 };
}

export function sizeGroup(item) {
  if (item.isDir) return { key: 'folders', label: t('gFolders'), order: 0 };
  const n = item.size || 0;
  const K = 1024, M = K * K, G = M * K;
  if (n === 0) return { key: 'empty', label: t('gEmpty'), order: 1 };
  if (n < 16 * K) return { key: 'tiny', label: t('gTiny'), order: 2 };
  if (n < M) return { key: 'small', label: t('gSmall'), order: 3 };
  if (n < 128 * M) return { key: 'medium', label: t('gMedium'), order: 4 };
  if (n < G) return { key: 'large', label: t('gLarge'), order: 5 };
  if (n < 4 * G) return { key: 'huge', label: t('gHuge'), order: 6 };
  return { key: 'gigantic', label: t('gGigantic'), order: 7 };
}

/* -------------------------------------------------------------- sorting */

const NAME_ORDER = { numeric: true, sensitivity: 'base' };
const coll = () => collator(NAME_ORDER);
export const compareNames = (a, b) => coll().compare(a || '', b || '');

/** Windows order: folders first (ascending), numeric-aware names; desc reverses everything. */
export function sortItems(items, key = 'name', desc = false, getters = {}) {
  const c = coll();
  const nameOf = (x) => x.displayName || x.name || '';
  const val = getters[key] || ({
    name: (x) => nameOf(x),
    modified: (x) => x.modified || 0,
    type: (x) => typeName(x),
    size: (x) => (x.isDir ? -1 : (x.size || 0)),
  })[key] || ((x) => nameOf(x));
  const arr = items.slice();
  arr.sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    const va = val(a), vb = val(b);
    let r = typeof va === 'number' && typeof vb === 'number' ? va - vb : c.compare(String(va), String(vb));
    if (r === 0 && key !== 'name') r = c.compare(nameOf(a), nameOf(b));
    return r;
  });
  if (desc) arr.reverse();
  return arr;
}

/* ---------------------------------------------------------------- icons */

const DPR = () => Math.min(3, Math.max(1, window.devicePixelRatio || 1));
const THUMB_EXT = /\.(png|jpe?g|gif|webp|bmp|svg|avif|heic|tiff?|mp4|mkv|webm|mov|avi|pdf)$/i;

export function canThumb(item) {
  if (!item || item.isDir) return false;
  const m = item.mime || '';
  return m.startsWith('image/') || m.startsWith('video/') || m === 'application/pdf' || THUMB_EXT.test(item.name || item.path || '');
}

export function iconUrlFor(item, size) {
  const px = Math.round(size * DPR());
  if (item.icon) return bridge.iconUrl(item.icon, px);
  if (item.isDir) return bridge.iconUrl('folder', px);
  return bridge.mimeIconUrl(item.mime || 'application/octet-stream', px);
}

/**
 * File/folder icon element with a fallback chain: thumbnail → themed icon → MIME icon → own glyph.
 * fileIcon(item, size, {thumb:true})
 */
export function fileIcon(item, size = 16, { thumb = false, cls = '' } = {}) {
  const wrap = h('span', { class: ['xp-ficon', cls], style: { width: `${size}px`, height: `${size}px` }, 'aria-hidden': 'true' });
  const chain = [];
  if (thumb && canThumb(item)) chain.push({ url: bridge.thumbUrl(item.path, Math.round(Math.max(size, 64) * DPR())), thumb: true });
  chain.push({ url: iconUrlFor(item, size) });
  if (item.icon && !item.isDir && item.mime) chain.push({ url: bridge.mimeIconUrl(item.mime, Math.round(size * DPR())) });
  if (item.isDir && item.icon && item.icon !== 'folder') chain.push({ url: bridge.iconUrl('folder', Math.round(size * DPR())) });
  let i = 0;
  const next = () => {
    while (i < chain.length && !chain[i].url) i++;
    if (i >= chain.length) {
      clear(wrap).append(icon(item.isDir ? 'folder' : 'file', { size: Math.max(12, Math.round(size * 0.8)) }));
      wrap.classList.add('is-glyph');
      return;
    }
    const c = chain[i++];
    const img = h('img', { src: c.url, alt: '', draggable: false, decoding: 'async', loading: size >= 48 ? 'lazy' : 'eager' });
    if (c.thumb) { wrap.classList.add('is-thumb'); img.classList.add('xp-thumb'); } else wrap.classList.remove('is-thumb');
    img.addEventListener('error', next, { once: true });
    clear(wrap).append(img);
  };
  next();
  return wrap;
}

/** Themed named icon (places, drives) with a glyph fallback. */
export function namedIcon(name, size = 16, glyph = 'folder') {
  const wrap = h('span', { class: 'xp-ficon', style: { width: `${size}px`, height: `${size}px` }, 'aria-hidden': 'true' });
  const url = name ? bridge.iconUrl(name, Math.round(size * DPR())) : '';
  const fallback = () => { clear(wrap).append(icon(glyph, { size: Math.max(12, Math.round(size * 0.8)) })); wrap.classList.add('is-glyph'); };
  if (!url) fallback();
  else {
    const img = h('img', { src: url, alt: '', draggable: false, decoding: 'async' });
    img.addEventListener('error', fallback, { once: true });
    wrap.append(img);
  }
  return wrap;
}

/** Friendly drive name: "Systeemschijf (/)", "BACKUP", "Virtuele machines (/srv/vm)". */
export function volumeLabel(v) {
  if (!v) return '';
  const name = v.name || v.label || basename(v.mount) || v.mount;
  if (v.mount === '/') return `${name} (/)`;
  if (!/^\/(media|run\/media|mnt)\//.test(v.mount || '') && !name.includes(v.mount)) return `${name} (${v.mount})`;
  return name;
}

/** How full a drive is, in % (0–100): the host's df-style `percent` = used / (used + free), so space that only
 *  the system may use (ext4 keeps 5 % for root) counts as taken. Every app shows this same number. */
export function volumePercent(v) {
  if (!v) return 0;
  const p = Number.isFinite(v.percent) ? v.percent : ((v.used || 0) + (v.free || 0) > 0 ? (v.used / (v.used + v.free)) * 100 : 0);
  return Math.min(100, Math.max(0, p));
}

export function volumeIcon(v) {
  if (v?.removable || v?.kind === 'usb') return 'drive-removable-media';
  if (v?.isSystem || v?.mount === '/') return 'drive-harddisk-system';
  return 'drive-harddisk';
}

/** Volume a path lives on (longest mount prefix). */
export function volumeFor(path, volumes = []) {
  let best = null;
  for (const v of volumes) {
    if (v.mount && isUnder(path, v.mount) && (!best || v.mount.length > best.mount.length)) best = v;
  }
  return best;
}

/** Write text to the system clipboard (async API with a textarea fallback). */
export async function copyText(text) {
  try {
    if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(text); return true; }
  } catch { /* fall through */ }
  const ta = h('textarea', { style: { position: 'fixed', left: '-9999px', top: '0' } });
  ta.value = text;
  document.body.append(ta);
  ta.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch { ok = false; }
  ta.remove();
  return ok;
}

export const isTextInput = (el) => !!el && (el.matches?.('input, textarea, [contenteditable="true"]') ?? false);

export const ARCHIVE_RE = /\.(zip|tar|tar\.(gz|bz2|xz|zst)|tgz|tbz2|txz|gz|bz2|xz)$/i;
