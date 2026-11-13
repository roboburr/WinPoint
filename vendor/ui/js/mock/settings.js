// Win11 Shell — mock settings store (owned by ui-core). Mirrors host/services/settings.py:
// the same DEFAULTS (SPEC §5 + the keys agreed at integration), the same type checks and
// error codes (bad_value, bad_path, bad_args, read_only, invalid_settings), and the same
// event: settings.changed {settings, path, changed:[dot paths]} — path is null for
// settings.patch, settings.import and settings.reset without a path; no event when nothing changed.
// Persisted in localStorage ('win11shell.mock.settings'); every access is try/catch'ed.

// Must stay equal to DEFAULTS in host/services/settings.py. explorer.pinned is deliberately
// absent (absent = Windows default pins; becomes a list once the user pins or unpins).
export const DEFAULTS = {
  version: 1,
  language: 'nl',
  theme: { mode: 'dark', accent: '#0067c0', accentFromWallpaper: false,
    transparency: true, syncSystem: true, accentOnTaskbar: false, captionButtons: true, gtkFamily: 'fluent' },
  wallpaper: { type: 'image', path: 'builtin:lotus-dark', color: '#0b2a4a',
    fit: 'fill', followTheme: true },
  taskbar: { alignment: 'center', search: 'box', taskView: true,
    widgets: false, autoHide: false, showSeconds: false,
    pinned: ['win11shell-explorer.desktop', 'firefox_firefox.desktop', 'snap-store_snap-store.desktop',
      'win11shell-settings.desktop'],
    // notification area (SPEC §6.15): known = {key: {name, icon, seen}}, maintained by the host
    tray: { hiddenMenu: true, shown: [], order: [], known: {} } },
  start: { pinned: [], showRecommended: true, showRecentApps: true,
    layout: 'default', folders: ['settings', 'explorer', 'documents', 'downloads'] },
  desktop: { showIcons: true, iconSize: 'medium', autoArrange: false,
    alignToGrid: true, sortBy: 'name', positions: {},
    showThisPC: true, showTrash: true, showHome: false },
  notifications: { dnd: false, sounds: true, showToasts: true },
  time: { h24: true, showSeconds: false, showDate: true, firstDayOfWeek: 1 },
  accessibility: { textScale: 100, reduceMotion: false },
  nightLight: { enabled: false, temperature: 4500 },
  taskmgr: { updateSpeed: 'normal', defaultPage: 'summary', alwaysOnTop: false,
    confirmEndTask: true, colorKeyedGraphs: true, graphSeconds: 60 },
  alles: { roots: 'auto', excluded: [], matchCase: false, matchPath: false,
    regex: false, wholeWord: false, showHidden: false, liveUpdate: true },
  explorer: { view: 'details', showHidden: false, showExtensions: true, sortBy: 'name',
    sortDesc: false, groupBy: 'none', navPane: true, detailsPane: false, previewPane: false,
    checkboxes: false, compact: false, navWidth: 248, columns: {},
    hiddenColumns: [], favorites: [], previewPane: false },
  privacy: { recentFiles: true },
  power: { confirm: true },
  search: { webEngine: 'https://duckduckgo.com/?q=' },
  // Windows Update (notes/claude/windows-update-gnome.md §4.12): Geavanceerde opties' switches and the values the
  // host restores (autoOpenPrevious, ubuntuHoldPrevious: null until the host writes them)
  update: { notifyRestart: true, metered: false, autoOpenPrevious: null, ubuntuHoldPrevious: null },
};

const KEY = 'win11shell.mock.settings';

const clone = (v) => (v === undefined ? undefined : JSON.parse(JSON.stringify(v)));
const isObj = (v) => v != null && typeof v === 'object' && !Array.isArray(v);
const MISSING = Symbol('missing');
const BAD_KEYS = new Set(['__proto__', 'constructor', 'prototype']);
// settings whose value may be a string or a list (host: _STR_OR_LIST)
const STR_OR_LIST = new Set(['alles.roots']);
// lists that may only hold strings (host: _STR_LISTS; tray app keys, SPEC §6.15)
const STR_LISTS = new Set(['taskbar.tray.shown', 'taskbar.tray.order']);
// settings with a fixed set of values (host: _ENUMS); a value of the right type outside the set is invalid_value
// (D-G31, as host/src/settings.rs set/patch since D-G24), a wrong type stays bad_value; both are dropped when loaded
const ENUMS = new Map([['language', ['nl', 'en']]]);
/** settings.badValue of host/i18n/settings.tsv (the same Dutch text as the mock's bad_value) */
const BAD_VALUE = 'Deze waarde past niet bij deze instelling.';

/** Deep merge src into target (objects merged, arrays/values replaced). Returns target. */
export function deepMerge(target, src) {
  if (!isObj(src)) return target;
  for (const k of Object.keys(src)) {
    if (BAD_KEYS.has(k)) continue;
    if (isObj(src[k]) && isObj(target[k])) deepMerge(target[k], src[k]);
    else target[k] = clone(src[k]);
  }
  return target;
}

function err(code, message, data, key) {
  const e = new Error(message);
  e.code = code;
  if (data) e.data = data;
  if (key) { e.key = key; e.params = {}; }
  return e;
}
/** D-G31: the host's refusal of a language outside nl/en (settings.rs: invalid_value + settings.badValue) */
const invalidValue = (data) => err('invalid_value', BAD_VALUE, data, 'settings.badValue');
/** a value of the right type outside an enum's set (a wrong type is typeOk's bad_value) */
const outsideEnum = (path, value) => ENUMS.has(path) && typeof value === typeof ENUMS.get(path)[0] && !ENUMS.get(path).includes(value);

/** 'a.b.c' or ['a', 'b', 'c.txt'] → segments (host: split_path). */
function splitPath(path) {
  let segs;
  if (Array.isArray(path)) segs = path.map(String);
  else if (typeof path === 'string') segs = path.split('.');
  else throw err('bad_path', 'Ongeldige instelling.');
  // eslint-disable-next-line no-control-regex
  if (!segs.length || segs.length > 16 || segs.some((x) => !x || x.length > 200 || /[\x00-\x1f]/.test(x) || BAD_KEYS.has(x))) {
    throw err('bad_path', 'Ongeldige instelling.', { path });
  }
  return segs;
}

export function getPath(obj, path, fallback) {
  let segs;
  try { segs = splitPath(path); } catch { return fallback; }
  let cur = obj;
  for (const k of segs) {
    if (isObj(cur) && Object.prototype.hasOwnProperty.call(cur, k)) cur = cur[k];
    else return fallback;
  }
  return cur;
}

function setPath(obj, segs, value) {
  let o = obj;
  for (let i = 0; i < segs.length - 1; i++) {
    if (!isObj(o[segs[i]])) o[segs[i]] = {};
    o = o[segs[i]];
  }
  o[segs[segs.length - 1]] = value;
  return obj;
}

function delPath(obj, segs) {
  let o = obj;
  for (const k of segs.slice(0, -1)) { o = isObj(o) ? o[k] : undefined; if (!isObj(o)) return; }
  if (isObj(o)) delete o[segs[segs.length - 1]];
}

/** Leaf dot paths that differ (host: diff_paths). */
function diffPaths(a, b, pre = '', out = []) {
  if (out.length >= 200) return out;
  if (isObj(a) && isObj(b)) {
    for (const k of [...Object.keys(a), ...Object.keys(b).filter((k2) => !(k2 in a))]) {
      diffPaths(k in a ? a[k] : MISSING, k in b ? b[k] : MISSING, pre ? `${pre}.${k}` : k, out);
    }
  } else if (JSON.stringify(a === MISSING ? '\u0000missing' : a) !== JSON.stringify(b === MISSING ? '\u0000missing' : b)) {
    out.push(pre);
  }
  return out;
}

/** Is `value` acceptable where the default is `def`? (host: _type_ok) */
function typeOk(def, value, path) {
  if (ENUMS.has(path)) return ENUMS.get(path).includes(value);
  if (def === undefined || def === null || def === MISSING) return true;
  if (typeof def === 'boolean') return typeof value === 'boolean';
  if (typeof def === 'number') return typeof value === 'number' && Number.isFinite(value);
  if (typeof def === 'string') return typeof value === 'string' || (STR_OR_LIST.has(path) && Array.isArray(value));
  if (Array.isArray(def)) return Array.isArray(value) && (!STR_LISTS.has(path) || value.every((v) => typeof v === 'string'));
  if (isObj(def)) return isObj(value);
  return true;
}

/** Drop leaves whose type does not match the defaults (unknown keys are kept). */
function sanitize(value, def, prefix, dropped) {
  if (isObj(def) && isObj(value)) {
    const out = {};
    for (const [k, v] of Object.entries(value)) {
      if (BAD_KEYS.has(k)) continue;
      const p = prefix ? `${prefix}.${k}` : k;
      const dv = Object.prototype.hasOwnProperty.call(def, k) ? def[k] : MISSING;
      if (dv === MISSING) out[k] = v;
      else if (!typeOk(dv, v, p)) dropped.push(p);
      else out[k] = sanitize(v, dv, p, dropped);
    }
    return out;
  }
  return value;
}

function load() {
  try {
    const raw = localStorage.getItem(KEY);
    if (raw) {
      const parsed = JSON.parse(raw);
      if (isObj(parsed)) return deepMerge(clone(DEFAULTS), sanitize(parsed, DEFAULTS, '', []));
    }
  } catch { /* ignore */ }
  return clone(DEFAULTS);
}

let store = load();

function save() {
  try { localStorage.setItem(KEY, JSON.stringify(store)); } catch { /* ignore (private mode etc.) */ }
}

/** Replace the store when something changed; emits settings.changed like the host. */
function apply(next, path, m) {
  const changed = diffPaths(store, next);
  if (!changed.length) return store;
  store = next;
  save();
  m?.emit('settings.changed', { settings: store, path, changed });
  return store;
}

const invalidSettings = () => err('invalid_settings', 'Dit bestand bevat geen geldige instellingen.');

export default {
  methods: {
    'settings.get': async () => store,

    'settings.set': async (args = {}, m) => {
      if (!('path' in args)) throw err('bad_args', 'Geef aan welke instelling je wilt wijzigen.');
      const segs = splitPath(args.path);
      if (segs.length === 1 && segs[0] === 'version') throw err('read_only', 'Deze instelling kan niet worden gewijzigd.');
      let value = args.value === undefined ? null : args.value;
      const label = segs.join('.');
      // D-G31 (settings.rs set): a language other than "nl"/"en", also an object path below it
      if (segs[0] === 'language' && (segs.length > 1 || outsideEnum('language', value))) throw invalidValue({ path: label });
      const def = getPath(DEFAULTS, segs, MISSING);
      if (def !== MISSING) {
        if (!typeOk(def, value, label)) throw err('bad_value', BAD_VALUE, { path: label });
        if (isObj(def)) {
          const dropped = [];
          value = sanitize(value, def, label, dropped);
          if (dropped.length) throw err('bad_value', BAD_VALUE, { paths: dropped });
        }
      }
      return apply(setPath(clone(store), segs, clone(value)), label, m);
    },

    'settings.patch': async ({ patch } = {}, m) => {
      if (!isObj(patch || {})) throw err('bad_value', 'Ongeldige wijziging.');
      const dropped = [];
      const clean = sanitize(patch || {}, DEFAULTS, '', dropped);
      // D-G31 (settings.rs patch): wrong types first (bad_value), then a language string other than "nl"/"en"
      const badLanguage = outsideEnum('language', patch?.language);
      const wrong = dropped.filter((d) => !(badLanguage && d === 'language'));
      if (wrong.length) throw err('bad_value', BAD_VALUE, { paths: wrong });
      if (badLanguage) throw invalidValue({ paths: ['language'] });
      delete clean.version;
      return apply(deepMerge(clone(store), clean), null, m);
    },

    'settings.reset': async ({ path } = {}, m) => {
      if (path == null || path === '') return apply(clone(DEFAULTS), null, m);
      const segs = splitPath(path);
      const def = getPath(DEFAULTS, segs, MISSING);
      const next = clone(store);
      if (def === MISSING) delPath(next, segs); else setPath(next, segs, clone(def));
      return apply(next, segs.join('.'), m);
    },

    'settings.export': async () => {
      const d = new Date();
      const p = (n) => String(n).padStart(2, '0');
      return { json: JSON.stringify(store, null, 2), name: `win11shell-instellingen-${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}.json` };
    },

    'settings.import': async ({ json } = {}, m) => {
      let obj = json;
      if (!isObj(json)) {
        if (typeof json !== 'string' || json.length > 4 * 1024 * 1024) throw invalidSettings();
        try { obj = JSON.parse(json); } catch { throw invalidSettings(); }
      }
      if (!isObj(obj) || !Object.keys(obj).some((k) => k !== 'version' && k in DEFAULTS)) throw invalidSettings();
      const dropped = [];
      const clean = sanitize(obj, DEFAULTS, '', dropped);
      clean.version = DEFAULTS.version;
      // D-G31: an invalid language in the file is dropped and the current one stays (everything else is the file's);
      // D-G49 (WU-FIX): a file without a language keeps the current one too, as host/src/settings.rs import()
      if (dropped.includes('language') || !('language' in obj)) clean.language = store.language;
      return apply(deepMerge(clone(DEFAULTS), clean), null, m);
    },

    'settings.backupFile': async () => {
      const d = new Date();
      const p = (n) => String(n).padStart(2, '0');
      const stamp = `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}${p(d.getMinutes())}${p(d.getSeconds())}`;
      const dir = '/home/robin/.config/win11shell';
      const name = `settings-backup-${stamp}.json`;
      return { path: `${dir}/${name}`, name, dir };
    },
  },

  start() { /* nothing periodic */ },

  /** current settings object (live; do not mutate) */
  current() { return store; },
  /** replace from another frame's settings.changed */
  replace(s) { if (isObj(s)) store = deepMerge(clone(DEFAULTS), s); },
  resetStorage() {
    try { localStorage.removeItem(KEY); } catch { /* ignore */ }
    store = clone(DEFAULTS);
  },
};
