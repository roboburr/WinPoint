// Win11 Shell — number / size / time formatting (owned by ui-core).
// Dutch by default ("1,2 GB", "3,4 MB/s", "2 u 5 min", "5 minuten geleden").

import { lang, locale } from './i18n.js';

// Intl formatters are cached per locale + options (building one costs far more than formatting: 2,000 rows of
// date + time took 208 ms uncached, 4 ms cached in JSC); the key holds the locale, so a language switch needs no reset.
const nfCache = new Map();
function nf(opts) {
  const key = locale() + JSON.stringify(opts);
  let f = nfCache.get(key);
  if (!f) { f = new Intl.NumberFormat(locale(), opts); nfCache.set(key, f); }
  return f;
}
const dtfCache = new Map();
function dtf(opts) {
  const key = locale() + JSON.stringify(opts);
  let f = dtfCache.get(key);
  if (!f) { f = new Intl.DateTimeFormat(locale(), opts); dtfCache.set(key, f); }
  return f;
}
const rtfCache = new Map();
function rtf(opts) {
  const key = locale() + JSON.stringify(opts);
  let f = rtfCache.get(key);
  if (!f) { f = new Intl.RelativeTimeFormat(locale(), opts); rtfCache.set(key, f); }
  return f;
}

/** number(1234.5) → "1.234,5"; number(n, 2) → max 2 decimals */
export function number(n, digits = 0, minDigits = 0) {
  if (n == null || !isFinite(n)) return '–';
  return nf({ maximumFractionDigits: digits, minimumFractionDigits: Math.min(minDigits, digits) }).format(n);
}

const UNITS = ['bytes', 'KB', 'MB', 'GB', 'TB', 'PB'];

/** bytes(1288490188) → "1,2 GB". Binary (1024) multiples like Windows, labelled KB/MB/GB.
 *  Like Windows, never four digits: from 1000 of a unit on, the next unit is used
 *  (1.053.818.880 bytes → "0,98 GB", not "1.005 MB"). */
export function bytes(n, { digits } = {}) {
  if (n == null || !isFinite(n)) return '–';
  const neg = n < 0;
  let v = Math.abs(n);
  if (v < 1000) {
    const whole = Math.round(v);
    const unit = whole === 1 ? 'byte' : 'bytes';
    return (neg ? '-' : '') + `${number(whole)} ${unit}`;
  }
  let i = 0;
  while (v >= 1000 && i < UNITS.length - 1) { v /= 1024; i++; }
  const d = digits ?? (v < 1 ? 2 : v >= 100 ? 0 : 1);
  return (neg ? '-' : '') + `${number(v, d)} ${UNITS[i]}`;
}

/** rate(3565158) → "3,4 MB/s"; rate(0) → "0 KB/s" */
export function rate(n, opts) {
  if (n == null || !isFinite(n)) return '–';
  if (n < 1000) return `${number(n / 1024, n > 0 ? 1 : 0)} KB/s`;
  return `${bytes(n, opts)}/s`;
}

/** bitrate(bytesPerSecond) → "8,0 Mbps" (network, Task Manager style) */
export function bitrate(bytesPerSec) {
  if (bytesPerSec == null || !isFinite(bytesPerSec)) return '–';
  let v = bytesPerSec * 8;
  const units = ['bps', 'Kbps', 'Mbps', 'Gbps'];
  let i = 0;
  while (v >= 1000 && i < units.length - 1) { v /= 1000; i++; }
  return `${number(v, i === 0 ? 0 : 1, i === 0 ? 0 : 1)} ${units[i]}`;
}

/** percent(45.2) → "45%"  (input 0–100). percent(0.452, {fraction:true}) → "45%" */
export function percent(v, opts = {}) {
  if (v == null || !isFinite(v)) return '–';
  const digits = typeof opts === 'number' ? opts : (opts.digits ?? 0);
  const val = (typeof opts === 'object' && opts.fraction) ? v * 100 : v;
  return `${number(val, digits, digits)}%`;
}

/** duration(7500) → "2 u 5 min"; duration(45) → "45 s"; {long:true} → "2 uur en 5 minuten" */
export function duration(sec, { long = false } = {}) {
  if (sec == null || !isFinite(sec)) return '–';
  sec = Math.max(0, Math.round(sec));
  const d = Math.floor(sec / 86400), hh = Math.floor((sec % 86400) / 3600),
    mm = Math.floor((sec % 3600) / 60), ss = sec % 60;
  const nl = lang() !== 'en';
  if (long) {
    const parts = [];
    const w = (n, one, many) => `${n} ${n === 1 ? one : many}`;
    if (d) parts.push(nl ? w(d, 'dag', 'dagen') : w(d, 'day', 'days'));
    if (hh) parts.push(nl ? w(hh, 'uur', 'uur') : w(hh, 'hour', 'hours'));
    if (mm && !d) parts.push(nl ? w(mm, 'minuut', 'minuten') : w(mm, 'minute', 'minutes'));
    if (!parts.length) parts.push(nl ? w(ss, 'seconde', 'seconden') : w(ss, 'second', 'seconds'));
    const and = nl ? ' en ' : ' and ';
    return parts.length > 1 ? parts.slice(0, -1).join(', ') + and + parts[parts.length - 1] : parts[0];
  }
  const H = nl ? 'u' : 'h';
  if (d) return hh ? `${d} d ${hh} ${H}` : `${d} d`;
  if (hh) return mm ? `${hh} ${H} ${mm} min` : `${hh} ${H}`;
  if (mm) return `${mm} min`;
  return `${ss} s`;
}

/** uptime(93784) → "1:02:03:04" (Task Manager style d:hh:mm:ss) */
export function uptime(sec) {
  sec = Math.max(0, Math.floor(sec || 0));
  const d = Math.floor(sec / 86400), hh = Math.floor((sec % 86400) / 3600),
    mm = Math.floor((sec % 3600) / 60), ss = sec % 60;
  const p = (n) => String(n).padStart(2, '0');
  return `${d}:${p(hh)}:${p(mm)}:${p(ss)}`;
}

/** relTime(Date.now() - 300000) → "5 minuten geleden"; < 45 s → "zojuist" */
export function relTime(ms, now = Date.now()) {
  if (ms == null || !isFinite(ms)) return '–';
  const diff = (ms - now) / 1000;
  const abs = Math.abs(diff);
  const nl = lang() !== 'en';
  if (abs < 45) return nl ? 'zojuist' : 'just now';
  const f = rtf({ numeric: 'auto' });
  const steps = [[60, 'second', 1], [3600, 'minute', 60], [86400, 'hour', 3600],
    [604800, 'day', 86400], [2629800, 'week', 604800], [31557600, 'month', 2629800], [Infinity, 'year', 31557600]];
  for (const [limit, unit, div] of steps) {
    if (abs < limit) return f.format(Math.round(diff / div), unit);
  }
  return '';
}

const DATE_FORMATS = {
  short: { day: '2-digit', month: '2-digit', year: 'numeric' },            // 25-10-2026
  medium: { day: 'numeric', month: 'short', year: 'numeric' },             // 25 okt 2026
  long: { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' }, // zaterdag 25 oktober 2026
  day: { weekday: 'long', day: 'numeric', month: 'long' },                 // zaterdag 25 oktober
  dayShort: { weekday: 'short', day: 'numeric', month: 'short' },          // za 25 okt
  month: { month: 'long', year: 'numeric' },                               // oktober 2026
  monthShort: { month: 'short' },
  weekday: { weekday: 'long' },
  weekdayShort: { weekday: 'short' },
};

/** date(ms, 'short'|'medium'|'long'|'day'|'month'|'datetime'|Intl options) */
export function date(ms, fmt = 'short') {
  if (ms == null || !isFinite(ms)) return '–';
  const d = new Date(ms);
  if (fmt === 'datetime') return `${date(ms, 'short')} ${time(ms)}`;
  const opts = typeof fmt === 'object' ? fmt : (DATE_FORMATS[fmt] || DATE_FORMATS.short);
  let s = dtf(opts).format(d);
  if (fmt === 'short' && lang() !== 'en') s = s.replace(/\//g, '-');
  return s;
}

/** time(ms, {seconds:false, h24:true}) → "14:05" */
export function time(ms, { seconds = false, h24 = true } = {}) {
  if (ms == null || !isFinite(ms)) return '–';
  const opts = { hour: h24 ? '2-digit' : 'numeric', minute: '2-digit', hour12: !h24 };
  if (seconds) opts.second = '2-digit';
  return dtf(opts).format(new Date(ms));
}

/** fileTime(ms) → "25-10-2026 14:05" (Explorer details column) */
export function fileTime(ms) { return date(ms, 'datetime'); }
