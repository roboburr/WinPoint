// Win11 Shell — host bridge (owned by ui-core). See SPEC §3/§4.
//
//   import { bridge } from '../core/bridge.js';
//   const s = await bridge.call('settings.get');
//   const off = bridge.on('settings.changed', ({settings, path}) => …);
//
// Real mode: POST /api/call/<method> with X-Shell-Token and X-Shell-Lang (the UI language: the host answers its
// texts in it, notes/english-option-design.md D15), SSE on /api/events.
// Mock mode (?mock=1 or no token): js/mock/index.js serves every method in-browser.
// Local event 'bridge.status' {connected, state:'connected'|'reconnecting', message}
// is emitted when the event stream drops or comes back.
// Errors are Error{code, message, method?, data?, key?, params?}: key/params name the host's catalog text (E2), for
// tests and the rare caller that special-cases one message; display always uses message.

import { makeT, lang } from './i18n.js';

const t = makeT({
  nl: {
    connected: 'Verbonden', disconnected: 'Verbinding verbroken',
    failed: 'Er ging iets mis.',
    offline: 'Geen verbinding met Win11 Shell. Probeer het zo opnieuw.',
    unauthorized: 'Geen toegang: deze pagina heeft geen geldige sleutel.',
    badResponse: 'Onverwacht antwoord van Win11 Shell.',
    mockFailed: 'De testomgeving kon niet worden geladen.',
    badRequest: 'Ongeldige aanvraag.',
    launchTitle: 'Deze snelkoppeling starten?',
    launchBody: '‘{name}’ is een snelkoppeling die een programma start. Sta dit alleen toe als je weet waar hij vandaan komt.',
    launchRuns: 'Dit wordt uitgevoerd:',
    launchAllow: 'Starten toestaan',
    cancel: 'Annuleren',
    notStarted: 'Niet gestart.',
  },
  en: {
    connected: 'Connected', disconnected: 'Connection lost',
    failed: 'Something went wrong.',
    offline: 'No connection to Win11 Shell. Try again in a moment.',
    unauthorized: 'No access: this page doesn’t have a valid key.',
    badResponse: 'Unexpected response from Win11 Shell.',
    mockFailed: 'The test environment couldn’t be loaded.',
    badRequest: 'Invalid request.',
    launchTitle: 'Start this shortcut?',
    launchBody: '“{name}” is a shortcut that starts a program. Only allow it if you trust where it came from.',
    launchRuns: 'This will run:',
    launchAllow: 'Allow launching',
    cancel: 'Cancel',
    notStarted: 'Not started.',
  },
});

const TOKEN_KEY = 'win11shell.token';
const params = new URLSearchParams(location.search);

function readToken() {
  let tok = params.get('token');
  if (tok) {
    try { sessionStorage.setItem(TOKEN_KEY, tok); } catch { /* ignore */ }
    // strip the token from the address bar
    try {
      const u = new URL(location.href);
      u.searchParams.delete('token');
      history.replaceState(history.state, '', u.pathname + u.search + u.hash);
    } catch { /* ignore */ }
    params.delete('token');
    return tok;
  }
  try { tok = sessionStorage.getItem(TOKEN_KEY); } catch { tok = null; }
  return tok || '';
}

const token = readToken();
const isMock = params.get('mock') === '1' || !token;

// Screenshot/test helpers (?open=, ?select=, ?openapp= …) pre-select items and pre-open menus and
// dialogs. On a page that holds the real token they only work together with ?debug=1, so a link can
// never arrange e.g. "these photos selected + the Prullenbak confirm open" for a single stray click.
// (Verkenner's own `select` — "Bestandslocatie openen" selects the file — is a real parameter.)
const DEBUG_ONLY_PARAMS = ['open', 'select', 'scrollTo', 'openapp', 'openApp', 'wmdemo', 'demo'];
if (!isMock && params.get('debug') !== '1') {
  const explorer = params.get('surface') === 'app' && params.get('app') === 'explorer';
  for (const k of DEBUG_ONLY_PARAMS) if (!(explorer && k === 'select')) params.delete(k);
}

/* ---------------------------------------------------------------- events */

const listeners = new Map(); // event → Set<cb>
const LOCAL_ONLY = /^(bridge\.|preview\.|flyout\.|local\.)/;

function dispatch(event, data) {
  const set = listeners.get(event);
  if (set) for (const cb of [...set]) {
    try { cb(data, event); } catch (e) { console.error(`[bridge] listener for "${event}" failed`, e); }
  }
  const any = listeners.get('*');
  if (any) for (const cb of [...any]) {
    try { cb(data, event); } catch (e) { console.error('[bridge] * listener failed', e); }
  }
}

let status = { connected: true, state: 'connected', changed: false };
/** The status as listeners get it; its message in the current language. */
const statusOut = () => ({ connected: status.connected, state: status.state,
  message: status.changed ? t(status.connected ? 'connected' : 'disconnected') : '' });
function setStatus(connected) {
  if (status.connected === connected) return;
  status = { connected, state: connected ? 'connected' : 'reconnecting', changed: true };
  dispatch('bridge.status', statusOut());
}

/* ------------------------------------------------------------ SSE (real) */

let es = null;
let esNames = new Set(['hello']);
let retryMs = 1000;
let retryTimer = 0;

function attachName(name) {
  if (!es) return;
  es.addEventListener(name, (ev) => {
    let data = null;
    try { data = ev.data ? JSON.parse(ev.data) : null; } catch { data = ev.data; }
    dispatch(name, data);
  });
}

function connectEvents() {
  if (isMock || es || typeof EventSource === 'undefined') return;
  clearTimeout(retryTimer);
  try {
    es = new EventSource(`/api/events?token=${encodeURIComponent(token)}`);
  } catch (e) {
    console.error('[bridge] EventSource failed', e);
    scheduleReconnect();
    return;
  }
  es.onopen = () => { retryMs = 1000; setStatus(true); };
  es.onerror = () => {
    setStatus(false);
    if (es && es.readyState === 2 /* CLOSED */) {
      es.close();
      es = null;
      scheduleReconnect();
    }
  };
  for (const n of esNames) attachName(n);
}

function scheduleReconnect() {
  clearTimeout(retryTimer);
  retryTimer = setTimeout(connectEvents, retryMs);
  retryMs = Math.min(retryMs * 2, 15000);
}

addEventListener('pagehide', () => { try { es?.close(); } catch { /* ignore */ } es = null; });

/* ------------------------------------------------------------ mock setup */

let mock = null;
let mockIcons = null;
if (isMock) {
  try {
    const mod = await import('../mock/index.js');
    mock = await mod.createMock({ dispatch });
    mockIcons = mod;
  } catch (e) {
    console.error('[bridge] mock backend failed to load', e);
  }
}

/* ------------------------------------------------------------------ calls */

function makeError(code, message, method, data, from) {
  const e = new Error(message || t('failed'));
  e.code = code || 'internal';
  if (method) e.method = method;
  if (data !== undefined) e.data = data;
  if (from?.key) e.key = String(from.key);              // the host's catalog key (E2), additive
  if (from?.params && typeof from.params === 'object') e.params = from.params;
  return e;
}

async function realCall(method, args, opts = {}) {
  let res;
  try {
    res = await fetch(`/api/call/${method}`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Shell-Token': token, 'X-Shell-Lang': lang() },
      body: JSON.stringify({ args: args ?? {} }),
      // keepalive: the request survives the page going away (pagehide / the window closing)
      ...(opts.keepalive ? { keepalive: true } : {}),
    });
  } catch {
    setStatus(false);
    throw makeError('offline', t('offline'), method);
  }
  if (res.status === 401 || res.status === 403) {
    throw makeError('unauthorized', t('unauthorized'), method);
  }
  let data;
  try { data = await res.json(); } catch {
    throw makeError('bad_response', t('badResponse'), method);
  }
  if (!data || data.ok !== true) {
    const err = (data && data.error) || {};
    throw makeError(err.code || 'internal', err.message || t('failed'), method, err.data, err);
  }
  return data.result;
}

async function mockCall(method, args) {
  if (!mock) throw makeError('mock_failed', t('mockFailed'), method);
  try {
    return await mock.call(method, args ?? {});
  } catch (e) {
    throw makeError(e?.code || 'internal', e?.message, method, e?.data, e);
  }
}

/* ----------------------------------------------- untrusted launchers (.desktop) */

// A .desktop file can run any command. Like GNOME's "Starten toestaan" (and Windows' "Open File – Security
// Warning"), the host refuses one that the user has not allowed yet (code 'untrusted', data {path, name, exec,
// canTrust}). Every caller (desktop, Verkenner, Start, Alles) gets the same question here: show what it runs,
// and only after "Starten toestaan" mark it trusted (files.trustLauncher) and try once more.
const LAUNCH_METHODS = new Set(['files.open', 'apps.launch', 'files.openWith']);

async function confirmLauncher(err, retry, { trust = true } = {}) {
  const d = err.data || {};
  let Dialog, h;
  try {
    ({ Dialog } = await import('./components.js'));
    ({ h } = await import('./dom.js'));
  } catch { throw err; }
  const name = d.name || (d.path || '').split('/').pop();
  const content = h('div', { class: 'fl-launcher-warn' },
    h('p', null, t('launchBody', { name })),
    h('div', { class: 'fl-launcher-warn__label caption text-2' }, t('launchRuns')),
    h('code', { class: 'fl-launcher-warn__cmd' }, d.exec || '?'),
    h('div', { class: 'fl-launcher-warn__path caption text-3' }, d.path || ''));
  const r = await Dialog.show({
    title: t('launchTitle'),
    content, icon: 'shield',
    primary: t('launchAllow'),
    close: t('cancel'),
    defaultButton: 'close',
  });
  if (r !== 'primary') throw makeError('cancelled', t('notStarted'), err.method);
  if (!trust) return null;
  await (isMock ? mockCall : realCall)('files.trustLauncher', { path: d.path });
  return retry();
}

/* ------------------------------------------------------------------ URLs */

const enc = encodeURIComponent;
const tokq = () => (token ? `&token=${enc(token)}` : '');

// Our apps' own icon names exist only when the installer added them; the host's /icon
// then serves this themed fallback instead of a 404 (same fallbacks as host apps.py).
const OWN_ICON_FALLBACK = {
  'win11shell-settings': 'preferences-system',
  'win11shell-taskmgr': 'utilities-system-monitor',
  'win11shell-explorer': 'system-file-manager',
  'win11shell-alles': 'system-search',
};

function isDirectUrl(s) {
  return /^(data:|blob:|https?:)/.test(s) || s.startsWith('/ui/');
}

// Built-in wallpapers that were renamed: settings saved before the rename still name the old id.
const WALLPAPER_ALIASES = { 'bloom-dark': 'lotus-dark', 'bloom-light': 'lotus-light' };

/** 'builtin:bloom-dark' → 'builtin:lotus-dark' (an old built-in id); every other path unchanged. */
export function builtinWallpaper(path) {
  const p = String(path ?? '');
  if (!p.startsWith('builtin:')) return path;
  const id = p.slice(8);
  return Object.hasOwn(WALLPAPER_ALIASES, id) ? `builtin:${WALLPAPER_ALIASES[id]}` : p;
}

/* ------------------------------------------------------------------- API */

export const bridge = {
  /** call(method, args, {keepalive?}) → Promise<result>; rejects with Error{code, message}.
   *  keepalive: also delivered when the page is going away (a last "stop" from pagehide). */
  call(method, args = {}, opts = {}) {
    if (typeof method !== 'string' || !method) return Promise.reject(makeError('bad_request', t('badRequest')));
    const raw = () => (isMock ? mockCall(method, args) : realCall(method, args, opts || {}));
    const p = raw();
    if (!LAUNCH_METHODS.has(method)) return p;
    return p.catch((e) => (e?.code === 'untrusted' && e.data?.canTrust && e.data?.path
      ? confirmLauncher(e, raw) : Promise.reject(e)));
  },

  /** on(event, cb) → unsubscribe(). cb(data, eventName). '*' receives every dispatched event. */
  on(event, cb) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(cb);
    if (!isMock && event !== '*' && !LOCAL_ONLY.test(event) && !esNames.has(event)) {
      esNames.add(event);
      attachName(event);
    }
    if (!isMock && !es) connectEvents();
    return () => { listeners.get(event)?.delete(cb); };
  },

  /** once(event) → Promise<data> */
  once(event) {
    return new Promise((resolve) => {
      const off = bridge.on(event, (d) => { off(); resolve(d); });
    });
  },

  /** Dispatch to local listeners only (no host round trip). */
  emitLocal(event, data) { dispatch(event, data); },

  /** iconUrl(name|absPath, size, {fallback}) — our own apps get a themed fallback icon automatically */
  iconUrl(nameOrPath, size = 32, { fallback } = {}) {
    if (!nameOrPath) return '';
    const n = String(nameOrPath);
    if (isDirectUrl(n)) return n;
    if (['win11shell-settings', 'io.github.win11shell.settings', 'preferences-system', 'org.gnome.settings', 'gnome-control-center', 'systemsettings'].includes(n.toLowerCase().replace(/\.desktop$/, ''))) return '/ui/assets/icons/settings.svg';
    if (isMock) return mockIcons ? mockIcons.mockIconUrl(n, size) : '';
    const fb = fallback || OWN_ICON_FALLBACK[n.toLowerCase().replace(/\.desktop$/, '')];
    return `/icon?name=${enc(n)}&size=${Math.round(size)}${fb ? `&fallback=${enc(fb)}` : ''}${tokq()}`;
  },

  mimeIconUrl(mime, size = 32) {
    if (!mime) return '';
    if (isMock) return mockIcons ? mockIcons.mockMimeIconUrl(mime, size) : '';
    return `/icon?mime=${enc(mime)}&size=${Math.round(size)}${tokq()}`;
  },

  thumbUrl(path, size = 256) {
    if (!path) return '';
    if (isMock) return mockIcons ? mockIcons.mockThumbUrl(path, size) : '';
    return `/thumb?path=${enc(path)}&size=${Math.round(size)}${tokq()}`;
  },

  /** fileUrl('builtin:lotus-dark') → '/ui/assets/wallpapers/lotus-dark.svg' (old builtin:bloom-… ids too) */
  fileUrl(path) {
    if (!path) return '';
    const p = String(builtinWallpaper(path));
    if (p.startsWith('builtin:')) return `/ui/assets/wallpapers/${enc(p.slice(8))}.svg`;
    if (isDirectUrl(p)) return p;
    if (isMock) return mockIcons ? mockIcons.mockThumbUrl(p, 1024) : '';
    return `/file?path=${enc(p)}${tokq()}`;
  },

  /** URL of another surface of this UI, keeping mock/theme/lang flags. */
  pageUrl(extra = {}) {
    const u = new URL('index.html', new URL('../../', import.meta.url));
    // mock test switches too, so an embedded app simulates the same tray as the page (not mockTrayShown:
    // the top frame applies that once, embedded frames get it through the shared settings)
    for (const k of ['mock', 'theme', 'lang', 'blur', 'transparency', 'mockCompositor', 'mockMode', 'mockTray',
      'mockStatic', 'mockTrayAttention']) {
      const v = params.get(k);
      if (v != null) u.searchParams.set(k, v);
    }
    for (const [k, v] of Object.entries(extra)) if (v != null && v !== '') u.searchParams.set(k, v);
    return u.pathname + u.search;
  },

  get status() { return statusOut(); },

  /** Screenshot/test helper (mock, or ?debug=1): the untrusted-launcher question for {path, name, exec}, as
   *  ?open=launcher-trust shows it (desktop). Trusts and launches nothing, whatever is answered. */
  demoLauncherQuestion(data = {}) {
    if (!isMock && params.get('debug') !== '1') return Promise.resolve(null);
    return confirmLauncher(makeError('untrusted', '', 'files.open', { ...data, canTrust: true }), () => null, { trust: false })
      .catch(() => null);
  },
  get connected() { return status.connected; },

  isMock,
  params,
  surface: params.get('surface') || 'preview',
  embedded: params.get('embedded') === '1',
};

export default bridge;
