// Taakbeheer — host calls that can never hang the page, and the host's health.
//
// User requirement: "Taakbeheer mag nooit kunnen vastlopen, anders kan ik Win11 Shell niet eens opnieuw starten."
// Everything in Taakbeheer talks to the host through this module (a drop-in for core/bridge.js):
//
//   bridge.call(method, args, {timeout, source})   every call has a timeout (per-method defaults in TIMEOUTS);
//                                                  in real mode the HTTP request itself is aborted, so a stuck
//                                                  host can never use up the WebView's connections
//   health.state            'ok' | 'down' | 'restarting'
//   health.onChange(cb)     cb(state, info) — info = {lastOkAt, since, prev, restarted?, gaveUp?, cancelled?}
//   health.restartShell({pid})   restart Win11 Shell (see below)
//
// A call that fails at the transport level (timeout, no connection, garbled or refused answer) is a *strike*;
// an answer from the host — also an error answer — resets the strikes. Two strikes in a row (a failed poll plus
// the probe right after it), or three failures in a row of one recurring poll (`source`, e.g. 'processes': the
// host answers but sysmon hangs) → 'down'. While anything failed recently a light probe (shell.info) runs every
// few seconds, so Taakbeheer also notices when the host is back; while all is well the same probe is a heartbeat
// every HEARTBEAT_MS. Its answer says how long the host's main loop has not turned (mainLoopLagMs): the host's
// HTTP threads answer while a hung GTK loop (and with it the taskbar) stands still, so a lag above LOOP_STUCK_MS
// is 'down' as well. The app window (host/appwindow.py) checks the host too: its verdict ('win11:hoststate'
// {ok:false}) counts as 'down' here.
//
// Restarting Win11 Shell: in the real app window (WebKitGTK) we post {cmd:'restartShell'} to
// window.webkit.messageHandlers.win11 — the appwindow process restarts the shell itself, so this works even when
// the host does not answer at all. A healthy host is asked first (sysmon.restartProcess {pid} → how:'shell'); when
// it said yes but never went away (same runId in shell.info, no failed call) within RESTART_QUIET_MS, the app
// window is asked after all.
// In mock mode the request is only logged (and the mock host, if simulated down, comes back).

import { bridge as core } from '../../core/bridge.js';
import { makeT, lang } from '../../core/i18n.js';

const t = makeT({
  nl: {
    timeout: 'Win11 Shell reageert niet. Probeer het zo opnieuw.',
    offline: 'Geen verbinding met Win11 Shell. Probeer het zo opnieuw.',
    unauthorized: 'Geen toegang: deze pagina heeft geen geldige sleutel.',
    badResponse: 'Onverwacht antwoord van Win11 Shell.',
    generic: 'Er ging iets mis.',
  },
  en: {
    timeout: 'Win11 Shell is not responding. Try again in a moment.',
    offline: 'No connection to Win11 Shell. Try again in a moment.',
    unauthorized: 'No access: this page doesn’t have a valid key.',
    badResponse: 'Unexpected response from Win11 Shell.',
    generic: 'Something went wrong.',
  },
});

/* ---------------------------------------------------------------- timeouts */

/** Default timeout per method (ms); 0 = none (a native dialog that waits for the user). */
export const TIMEOUTS = {
  // the 1-second polls: a healthy host answers in 20–300 ms
  'sysmon.processes': 4000, 'sysmon.performance': 4000, 'sysmon.health': 10000, 'sysmon.startup': 8000,
  'sysmon.services': 8000, 'storage.volumes': 8000, 'shell.info': 2000,
  'sysmon.endTask': 10000, 'sysmon.setEfficiency': 8000, 'sysmon.setStartup': 8000,
  'sysmon.restartProcess': 45000, 'sysmon.serviceAction': 30000,
  'storage.scan': 30000, 'storage.scanResult': 30000, 'storage.scanCancel': 5000,
  'storage.largeFiles': 60000, 'storage.categories': 60000, 'storage.cleanup.list': 30000, 'storage.cleanup.run': 180000,
  'files.trash': 60000, 'shell.pickFile': 0,
};
const DEFAULT_TIMEOUT = 8000;
/** Calls slower than this say little about the host's health when they time out (scans, clean-up). */
const HEALTH_MAX_TIMEOUT = 10000;
/** Errors that mean "the host did not answer" (anything else is an answer). */
const TRANSPORT = new Set(['timeout', 'offline', 'bad_response', 'unauthorized']);
// The untrusted-launcher question lives in core/bridge.js: these always go through it.
const VIA_CORE = new Set(['files.open', 'apps.launch', 'files.openWith', 'files.trustLauncher']);

export const isTransportError = (e) => !!e && TRANSPORT.has(e.code);

function makeError(code, message, method, data, from) {
  const e = new Error(message || t('generic'));
  e.code = code || 'internal';
  if (method) e.method = method;
  if (data !== undefined) e.data = data;
  if (from?.key) e.key = String(from.key);              // the host's catalog key (E2), as core/bridge.js
  if (from?.params && typeof from.params === 'object') e.params = from.params;
  return e;
}

/* ------------------------------------------------------------ transport */

function readToken() {
  try { return sessionStorage.getItem('win11shell.token') || ''; } catch { return ''; }
}

/** POST /api/call/<method> (SPEC §3) with an AbortController: the request really ends at the timeout. */
async function fetchCall(method, args, timeout, token) {
  const ctl = new AbortController();
  let timedOut = false;
  const timer = timeout ? setTimeout(() => { timedOut = true; ctl.abort(); }, timeout) : 0;
  try {
    let res;
    try {
      res = await fetch(`/api/call/${method}`, {
        method: 'POST', cache: 'no-store', signal: ctl.signal,
        // X-Shell-Lang: the worker (or the host) answers its texts in the UI language (D15; its CORS allows X-Shell-*)
        headers: { 'Content-Type': 'application/json', 'X-Shell-Token': token, 'X-Shell-Lang': lang() },
        body: JSON.stringify({ args: args ?? {} }),
      });
    } catch {
      throw timedOut ? makeError('timeout', t('timeout'), method) : makeError('offline', t('offline'), method);
    }
    if (res.status === 401 || res.status === 403) throw makeError('unauthorized', t('unauthorized'), method);
    let data;
    try { data = await res.json(); } catch {
      throw timedOut ? makeError('timeout', t('timeout'), method) : makeError('bad_response', t('badResponse'), method);
    }
    if (!data || data.ok !== true) {
      const err = (data && data.error) || {};
      throw makeError(err.code || 'internal', err.message || t('generic'), method, err.data, err);
    }
    return data.result;
  } finally {
    clearTimeout(timer);
  }
}

/** core.call with a timeout race (mock mode, launchers, or no token readable). */
function raceCall(method, args, timeout) {
  const p = core.call(method, args);
  if (!timeout) return p;
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(makeError('timeout', t('timeout'), method)), timeout);
    p.then((v) => { clearTimeout(timer); resolve(v); }, (e) => { clearTimeout(timer); reject(e); });
  });
}

/**
 * call(method, args, {timeout, source, health}) → Promise<result>; rejects with Error{code, message}.
 * `source` names a recurring poll (store channel); `health:false` keeps a call out of the health count.
 */
function call(method, args = {}, opts = {}) {
  const timeout = opts.timeout ?? TIMEOUTS[method] ?? DEFAULT_TIMEOUT;
  const tracked = opts.health ?? (timeout > 0 && timeout <= HEALTH_MAX_TIMEOUT);
  const token = core.isMock || VIA_CORE.has(method) ? '' : readToken();
  const p = token ? fetchCall(method, args, timeout, token) : raceCall(method, args, timeout);
  return p.then(
    (v) => { if (tracked) health.sample(true, opts.source); return v; },
    (e) => {
      if (isTransportError(e)) {
        if (tracked) health.sample(false, opts.source);
        else health.suspect();
      } else if (tracked) health.sample(true, opts.source);   // the host answered (with an error)
      throw e;
    },
  );
}

/* --------------------------------------------------------------- health */

const PROBE_MS = 2500;          // probe interval while something failed recently
const PROBE_FAST_MS = 1200;     // … while a restart is in progress
const HEARTBEAT_MS = 5000;      // probe when nothing asked the host for this long (window visible only)
const STREAK_TTL = 15000;       // a recurring poll's failures only count while it keeps failing
const RESTART_MIN_MS = 1500;    // show "wordt opnieuw gestart" at least this long
const RESTART_QUIET_MS = 12000; // host never went away: the restart was quick or did not happen
const RESTART_GIVEUP_MS = 30000;
const LOOP_STUCK_MS = 8000;     // shell.info mainLoopLagMs above this: the host answers, its main loop does not turn

function nativeHandler() {
  try { return window.webkit?.messageHandlers?.win11 || null; } catch { return null; }
}

export const health = (() => {
  let state = 'ok';
  let since = Date.now();
  let lastOkAt = 0;
  let lastSampleAt = Date.now();
  let strikes = 0;
  const streaks = new Map();    // source → {n, at}
  const subs = new Set();
  let probeTimer = 0;
  let probing = false;
  let lastProbeAt = 0;
  let loopStuck = false;        // the last shell.info said the host's main loop stands still
  let nativeDown = false;       // the app window says the host does not answer (win11:hoststate ok:false)
  let runId = null;             // shell.info runId: new for every start of the host, also an in-place one
  let restart = null;           // {at, sawFail, via?, runId?, nativeTried?}

  const info = (extra) => ({ lastOkAt, since, restarting: !!restart, ...extra });
  function setState(s, extra = {}) {
    if (s === state) return;
    const prev = state;
    state = s;
    since = Date.now();
    if (s === 'down') console.warn('[taskmgr] Win11 Shell does not answer');
    else if (s === 'ok' && prev !== 'ok') console.info('[taskmgr] Win11 Shell answers again');
    const i = info({ prev, ...extra });
    for (const cb of [...subs]) { try { cb(state, i); } catch (e) { console.error('[taskmgr] health listener failed', e); } }
  }

  function streakDown() {
    const now = Date.now();
    let down = false;
    for (const [k, v] of streaks) {
      if (now - v.at > STREAK_TTL) streaks.delete(k);
      else if (v.n >= 3) down = true;
    }
    return down;
  }
  const failing = () => strikes > 0 || streaks.size > 0;

  function evaluate() {
    const down = strikes >= 2 || streakDown() || loopStuck || nativeDown;
    if (restart) {
      const age = Date.now() - restart.at;
      if (restart.via === 'api' && !restart.sawFail && !restart.nativeTried && age >= RESTART_QUIET_MS) {
        // the host said it restarts itself, but it never went away (its main loop may hang): the app window does it
        const native = nativeHandler();
        if (native) {
          restart.nativeTried = true;
          restart.at = Date.now();
          console.warn('[taskmgr] Win11 Shell did not restart itself; asking the app window');
          try { native.postMessage({ cmd: 'restartShell' }); } catch (e) { console.warn('[taskmgr] restartShell post failed', e); }
          schedule();
          return;
        }
      }
      if (!down && !failing() && age >= RESTART_MIN_MS && (restart.sawFail || age >= RESTART_QUIET_MS)) {
        const came = restart.sawFail;
        restart = null;
        setState('ok', { restarted: came });
      } else if (down && age >= RESTART_GIVEUP_MS) {
        restart = null;
        setState('down', { gaveUp: true });
      }
    } else setState(down ? 'down' : 'ok');
    schedule();
  }

  function schedule(ms) {
    clearTimeout(probeTimer);
    probeTimer = 0;
    if (probing || document.hidden) return;
    if (ms == null) {
      // the heartbeat probe runs even while other polls answer: only shell.info carries the main-loop lag
      if (!failing() && state === 'ok' && !loopStuck) ms = Math.max(1000, lastProbeAt + HEARTBEAT_MS - Date.now());
      else ms = restart ? PROBE_FAST_MS : PROBE_MS;
    }
    probeTimer = setTimeout(probe, ms);
  }

  async function probe() {
    probeTimer = 0;
    if (probing) return;
    probing = true;
    lastProbeAt = Date.now();
    try {
      const info = await call('shell.info', {}, { timeout: TIMEOUTS['shell.info'], health: true });
      const lag = info && typeof info.mainLoopLagMs === 'number' ? info.mainLoopLagMs : 0;
      if ((lag > LOOP_STUCK_MS) !== loopStuck) {
        loopStuck = lag > LOOP_STUCK_MS;
        if (loopStuck) console.warn(`[taskmgr] Win11 Shell answers, but its main loop has not run for ${Math.round(lag / 1000)} s`);
      }
      if (loopStuck && restart) restart.sawFail = true;
      const id = info && typeof info.runId === 'string' ? info.runId : null;
      if (id && runId && id !== runId && restart) restart.sawFail = true;   // it restarted (in place: same pid)
      if (id) runId = id;
    } catch { /* counted */ }
    probing = false;
    evaluate();
    if (strikes === 1) schedule(300);   // a first failure of the probe itself: confirm it right away
  }

  document.addEventListener('visibilitychange', () => { if (!document.hidden) schedule(); });
  // the app window (host/appwindow.py) checks the host too and tells the page: {ok, mode, restarting}
  addEventListener('win11:hoststate', (ev) => {
    const d = ev.detail || {};
    nativeDown = d.ok === false;
    if (d.restarting && !restart) { restart = { at: Date.now(), sawFail: false }; setState('restarting'); }
    else if (d.ok === false && restart) restart.sawFail = true;
    evaluate();
    schedule(d.ok === false || d.restarting ? 0 : 50);
  });
  // real mode: the event stream drops / comes back when the host goes away / is back
  core.on('bridge.status', (s) => {
    if (s && !s.connected && restart) restart.sawFail = true;
    schedule(s?.connected ? 50 : 300);
  });

  const api = {
    get state() { return state; },
    get info() { return info(); },
    get lastOkAt() { return lastOkAt; },
    onChange(cb) { subs.add(cb); return () => subs.delete(cb); },
    /** One call's outcome (see the header). */
    sample(ok, source) {
      lastSampleAt = Date.now();
      if (ok) {
        lastOkAt = Date.now();
        strikes = 0;
        if (source) streaks.delete(source);
      } else {
        strikes++;
        if (restart) restart.sawFail = true;
        if (source) {
          const s = streaks.get(source) || { n: 0, at: 0 };
          streaks.set(source, { n: s.n + 1, at: Date.now() });
        }
      }
      evaluate();
      // a failed poll plus a failed probe right after it is enough to say "reageert niet"
      if (!ok && strikes === 1) schedule(0);
    },
    /** Something slow failed: check the host without counting it. */
    suspect() { schedule(0); },
    /** Check now (e.g. after the window came back). */
    check() { schedule(0); },
    /** The host said it restarts itself (sysmon.restartProcess → how:'shell'): via 'api' arms the fallback. */
    expectRestart(via) { restart = { at: Date.now(), sawFail: false, via: via || null }; setState('restarting'); schedule(); },

    /**
     * Restart Win11 Shell. {pid} = the host's pid (Win11 Shell row), when known.
     * → {how:'native'|'api'|'mock'|'dryRun'|'none', result?}; throws an ApiError when the host refused.
     */
    async restartShell({ pid } = {}) {
      const native = nativeHandler();
      const begin = () => api.expectRestart();
      const cancel = () => { restart = null; setState(strikes >= 2 || streakDown() ? 'down' : 'ok', { cancelled: true }); schedule(); };
      // A healthy host restarts itself (and says how); a stuck one cannot: then the app window does it.
      if (state === 'ok' && pid) {
        begin();   // the banner says "wordt opnieuw gestart…" right away
        try {
          const res = await call('sysmon.restartProcess', { pid }, { timeout: 6000, health: false });
          if (res?.dryRun && !core.isMock) { cancel(); return { how: 'dryRun', result: res }; }
          if (res && res.ok === false) throw makeError('failed', res.error || t('generic'), 'sysmon.restartProcess');
          if (restart) restart.via = 'api';   // never went away within RESTART_QUIET_MS → the app window does it
          return { how: 'api', result: res };
        } catch (e) {
          if (!native || !(isTransportError(e) || e.code === 'unknown_method')) { cancel(); throw e; }
          // no answer (or an older host): the app window restarts it
        }
      }
      if (native) {
        begin();
        try { native.postMessage({ cmd: 'restartShell' }); } catch (e) { console.warn('[taskmgr] restartShell post failed', e); }
        return { how: 'native' };
      }
      if (core.isMock) {
        console.info('[taskmgr] restartShell: would post {cmd:"restartShell"} to the app window (mock: the mock host restarts)');
        begin();
        try { window.__win11MockHost?.restart?.(); } catch { /* ignore */ }
        return { how: 'mock' };
      }
      // embedded preview / plain browser: only the host itself can do it
      begin();
      try {
        const res = pid ? await call('sysmon.restartProcess', { pid }, { timeout: 6000, health: false })
          : await call('shell.restart', {}, { timeout: 6000, health: false });
        if (res?.dryRun) { cancel(); return { how: 'dryRun', result: res }; }
        return { how: 'api', result: res };
      } catch (e) {
        if (isTransportError(e)) return { how: 'none' };
        cancel();
        throw e;
      }
    },
  };
  schedule();   // heartbeat
  return api;
})();

/* ------------------------------------------------------------------ API */

/** Drop-in for core/bridge.js inside Taakbeheer (calls with timeouts; everything else is core's). */
export const bridge = {
  call,
  on: (event, cb) => core.on(event, cb),
  once: (event) => core.once(event),
  emitLocal: (event, data) => core.emitLocal(event, data),
  iconUrl: (...a) => core.iconUrl(...a),
  mimeIconUrl: (...a) => core.mimeIconUrl(...a),
  thumbUrl: (...a) => core.thumbUrl(...a),
  fileUrl: (...a) => core.fileUrl(...a),
  pageUrl: (...a) => core.pageUrl(...a),
  get isMock() { return core.isMock; },
  get params() { return core.params; },
  get surface() { return core.surface; },
  get embedded() { return core.embedded; },
  get status() { return core.status; },
  get connected() { return core.connected; },
  health,
  onHealth: (cb) => health.onChange(cb),
};

export default bridge;
