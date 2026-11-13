// Win11 Shell — i18n (owned by ui-core).
//
//   const t = makeT({ nl: { hello: 'Hallo {name}', files: { one: '1 bestand', other: '{n} bestanden' } },
//                     en: { hello: 'Hello {name}', files: { one: '1 file', other: '{n} files' } } });
//   t('hello', {name: 'Robin'})   t('files', {n: 3})
//
// Fallback order: current language → nl → key. Values may be strings, functions
// (params) => string, or plural objects {zero?, one, other} chosen by countOf(params): the first
// *number* of params.n / params.count, so t('items', {n: number(c), count: c}) selects by c.
//
//   list(['A', 'B', 'C'])  → "A, B en C" / "A, B and C"      collator(opts) → cached Intl.Collator(locale(), opts)
//
// ?i18ncheck=1 (or ?debug=1): missing keys, unfilled placeholders and plurals that got a formatted n
// without a numeric count are collected in i18nReport (window.__i18n; JSON.stringify works), never thrown.

const SUPPORTED = ['nl', 'en'];
let current = 'nl';
const listeners = new Set();

try {
  const q = new URLSearchParams(location.search).get('lang');
  if (q && SUPPORTED.includes(q)) current = q;
} catch { /* ignore */ }

export function lang() { return current; }

/** Intl locale for the current language. */
export function locale() { return current === 'en' ? 'en-GB' : 'nl-NL'; }

export function setLang(l) {
  if (!l || !SUPPORTED.includes(l) || l === current) return;
  current = l;
  try { document.documentElement.lang = l; } catch { /* ignore */ }
  for (const cb of [...listeners]) { try { cb(l); } catch (e) { console.error(e); } }
}

export function onLangChange(cb) {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/* dev/test report (only with ?i18ncheck=1 or ?debug=1) */
const CHECK = (() => { try { return /[?&](i18ncheck|debug)=1\b/.test(location.search); } catch { return false; } })();
export const i18nReport = {
  missing: new Set(), unfilled: new Set(), plural: new Set(),
  toJSON() { return { missing: [...this.missing], unfilled: [...this.unfilled], plural: [...this.plural] }; },
};
if (CHECK) { try { globalThis.window.__i18n = i18nReport; } catch { /* no window */ } }

function note(set, entry) {
  if (set.has(entry)) return;
  set.add(entry);
  try { console.warn('[i18n]', entry); } catch { /* ignore */ }
}

/** The module that called makeT, from a stack trace: "apps/explorer/i18n.js" (V8 "at …" and JSC "fn@url:l:c"). */
function moduleOf(stack) {
  for (const line of String(stack || '').split('\n')) {
    const m = line.match(/(?:\(|@|at |^)((?:[a-z]+:\/\/|\/)[^\s()]*?):\d+:\d+\)?\s*$/);
    if (!m || /\/core\/i18n\.js(?:[?#].*)?$/.test(m[1])) continue;
    const u = m[1].replace(/[?#].*$/, '');
    const i = u.lastIndexOf('/js/');
    return i >= 0 ? u.slice(i + 4) : u.slice(u.lastIndexOf('/') + 1);
  }
  return '?';
}

/**
 * The count that selects a plural form: the first *number* of params.n and params.count (n first, as before),
 * else an integer string ("12"), else NaN (→ other); no n and no count at all is 0, as before. Equal to the old
 * Number(params.n ?? params.count ?? 0) for every numeric count; it differs only where callers pass a formatted
 * n: nl "1.000" was Number() 1 and selected `one` ("1.000 item"), now `other` (en "1,000" was NaN already).
 */
export function countOf(p) {
  const n = p?.n, c = p?.count;
  if (n == null && c == null) return 0;
  if (typeof n === 'number') return n;
  if (typeof c === 'number') return c;
  for (const v of [n, c]) if (typeof v === 'string' && /^-?\d+$/.test(v)) return Number(v);
  return NaN;
}

const listCache = new Map();
/** list(['A', 'B', 'C']) → "A, B en C" / "A, B and C" (en-GB: no serial comma); type 'disjunction' → of / or. */
export function list(items, type = 'conjunction') {
  const xs = [...(items || [])].map(String);
  const key = locale() + '|' + type;
  let f = listCache.get(key);
  if (f === undefined) {
    try { f = new Intl.ListFormat(locale(), { type }); } catch { f = null; }
    listCache.set(key, f);
  }
  if (f) return f.format(xs);
  if (xs.length < 2) return xs[0] ?? '';
  const word = type === 'disjunction' ? (current === 'en' ? ' or ' : ' of ') : (current === 'en' ? ' and ' : ' en ');
  return xs.slice(0, -1).join(', ') + word + xs[xs.length - 1];
}

const collCache = new Map();
/** collator({numeric: true, sensitivity: 'base'}) → a cached Intl.Collator for the current locale and options. */
export function collator(opts = {}) {
  const key = locale() + JSON.stringify(opts);
  let c = collCache.get(key);
  if (!c) { c = new Intl.Collator(locale(), opts); collCache.set(key, c); }
  return c;
}

function pick(dict, l, key) {
  const d = dict && dict[l];
  return d && Object.prototype.hasOwnProperty.call(d, key) ? d[key] : undefined;
}

function fill(s, params, where) {
  if (!params) {
    if (where !== undefined) for (const m of s.matchAll(/\{(\w+)\}/g)) note(i18nReport.unfilled, `${where}:{${m[1]}}`);
    return s;
  }
  return s.replace(/\{(\w+)\}/g, (m, k) => {
    if (params[k] === undefined || params[k] === null) {
      if (where !== undefined) note(i18nReport.unfilled, `${where}:{${k}}`);
      return m;
    }
    return String(params[k]);
  });
}

export function makeT(dict) {
  const mod = CHECK ? moduleOf(new Error().stack) : '';
  return function t(key, params) {
    let v = pick(dict, current, key);
    if (v === undefined) {
      if (CHECK) note(i18nReport.missing, `${mod}:${current}:${key}`);
      v = pick(dict, 'nl', key);
    }
    if (v === undefined) return fill(String(key), params);
    if (typeof v === 'function') v = v(params || {});
    if (v && typeof v === 'object') {
      if (CHECK && typeof params?.n === 'string' && typeof params?.count !== 'number') {
        note(i18nReport.plural, `${mod}:${key}`);
      }
      const n = countOf(params);
      v = (n === 0 && v.zero !== undefined) ? v.zero : (n === 1 ? (v.one ?? v.other) : (v.other ?? v.one));
    }
    return fill(String(v ?? ''), params, CHECK ? `${mod}:${key}` : undefined);
  };
}

/** Pick a localized value from an object like {nl: '..', en: '..'} (e.g. SETTINGS_PAGES titles). */
export function tr(obj) {
  if (obj == null) return '';
  if (typeof obj === 'string') return obj;
  return obj[current] ?? obj.nl ?? obj.en ?? '';
}
