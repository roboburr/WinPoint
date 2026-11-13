// Win11 Shell — mock shell.* methods (owned by ui-core). SPEC §6.2, shaped like
// host/services/shell.py (see notes/host-core.md): shell.info has the extra fields
// layerShellInstalled, apps, services and dev; shell.command returns {delivered};
// shell.openApp accepts 'taskmgr', 'win11shell-taskmgr' and 'win11shell-taskmgr.desktop'.
// Test overrides: ?mockMode=layer|preview|serve|app  ?mockCompositor=hyprland|gnome|other
//
// Simulated host outage (Taakbeheer's "never stuck" checks). Affects every shell.* and sysmon.* call:
//   ?mockHostDown=1|hang       the host hangs: calls never answer (like a frozen host)
//   ?mockHostDown=offline      calls fail at once with code 'offline' (like a crashed host)
//   ?mockHostDownAfter=SEC     go down SEC seconds after the page loaded (mode from mockHostDown, default hang)
//   ?mockHostRestartMs=MS      how long a restart takes (default 2500)
//   ?mockLoopStuck=1           the host answers, but its main loop stands still (shell.info mainLoopLagMs 30000)
//   window.__win11MockHost = {down(mode), up(), restart(ms), stuck(on), state}  (restart = down 'offline', then up;
//                            a restart also ends a stuck main loop and gives shell.info a new runId)
// shell.restart and sysmon.restartProcess of the Win11 Shell row restart the mock host the same way.
// shell.restartAll {reason:'libraries'|'update'} (GX1, notes/claude/windows-update-gnome.md §5; GW3 D3): the host's
//   guard first (update.guard.upgrade while the release upgrader runs, update.guard.restart while an own update job
//   runs in any phase, update.guard.software while a software job is queued/running/authenticating; texts as
//   host/i18n/update.tsv), then the event shell.restartAll {nonce, origin, at, reason}; the mock host goes away
//   after the origin's 2 s and comes back on the new libraries: the update page's Win11 restart InfoBar is gone.
//   A second call while that restart is under way answers its nonce with pending:true and sends no event.
//   ?mockRestartAll=live       its answers are not a dry run (dryRun:false, as a host outside test mode answers),
//                              so the update page keeps its restart button disabled (R3-UI; surface
//                              settings-check.restartall-sent)

import { lang } from '../core/i18n.js';

const APP_IDS = ['settings', 'taskmgr', 'explorer', 'alles', 'disks'];
const KNOWN_COMMANDS = new Set(['toggle-start', 'toggle-search', 'quick-settings', 'notifications',
  'task-view', 'alt-tab', 'winx', 'show-desktop', 'clipboard', 'osd', 'run', 'power']);
const COMMAND_RE = /^[a-z][a-z0-9-]{0,40}$/;
// as host/services/__init__.py loads them (one per module in host/services)
const SERVICES = ['settings', 'shell', 'apps', 'clipboard', 'disks', 'display', 'filemanager1', 'files', 'index',
  'media', 'notifications', 'storage', 'sysmon', 'system', 'tray', 'windows'];

function err(code, message) { const e = new Error(message); e.code = code; return e; }
/** The host's text in the language of the settings (mock/settings.js), like gnome.js's L(). */
function L(m, nl, en) {
  let lang = 'nl';
  try { lang = m?.settings?.()?.language === 'en' ? 'en' : 'nl'; } catch { /* keep nl */ }
  return lang === 'en' ? en : nl;
}

/* ------------------------------------------------------------ host outage */

/** core/bridge.js's offline text, in the language of the call (a host that is down cannot render its own) */
const offlineText = () => (lang() === 'en'
  ? 'No connection to Win11 Shell. Try again in a moment.'
  : 'Geen verbinding met Win11 Shell. Probeer het zo opnieuw.');

export const hostGate = (() => {
  const q = new URLSearchParams(typeof location !== 'undefined' ? location.search : '');
  const want = q.get('mockHostDown');
  const modeOf = (v) => (v === 'offline' ? 'offline' : 'hang');
  const restartMs = Math.max(200, Number(q.get('mockHostRestartMs')) || 2500);
  let mode = null; // null | 'hang' | 'offline'
  let restartTimer = 0;
  let loopStuck = q.get('mockLoopStuck') === '1';
  let runId = 1;
  const waiting = new Set();

  const gate = {
    get state() { return mode || 'up'; },
    get loopStuck() { return loopStuck; },
    get runId() { return `mock-${runId}`; },
    stuck(on = true) { loopStuck = !!on; console.info(`[mock] host main loop ${loopStuck ? 'stuck' : 'runs again'}`); },
    down(m = 'hang') {
      mode = modeOf(m);
      console.info(`[mock] host down (${mode})`);
    },
    up() {
      clearTimeout(restartTimer);
      if (mode) console.info('[mock] host up');
      mode = null;
      for (const r of waiting) r();
      waiting.clear();
    },
    restart(ms = restartMs) {
      console.info(`[mock] host restarts (${ms} ms)`);
      // the old host goes away at once; the new one answers after `ms`
      clearTimeout(restartTimer);
      mode = 'offline';
      for (const r of waiting) r();   // hanging calls end with the old host
      waiting.clear();
      restartTimer = setTimeout(() => { loopStuck = false; runId++; gate.up(); }, ms);
    },
    /** await gate.pass() in a method: hangs or throws while the host is down. */
    async pass() {
      if (!mode) return;
      if (mode === 'offline') throw err('offline', offlineText());
      await new Promise((r) => waiting.add(r));
      if (mode === 'offline') throw err('offline', offlineText());
    },
    /** Wrap a methods object so every method waits for the gate. */
    wrap(methods) {
      const out = {};
      for (const [name, fn] of Object.entries(methods)) out[name] = async (args, m) => { await gate.pass(); return fn(args, m); };
      return out;
    },
  };
  if (want && want !== '0') {
    const after = Number(q.get('mockHostDownAfter'));
    if (after > 0) setTimeout(() => gate.down(want === '1' ? 'hang' : want), after * 1000);
    else gate.down(want === '1' ? 'hang' : want);
  } else if (Number(q.get('mockHostDownAfter')) > 0) {
    setTimeout(() => gate.down('hang'), Number(q.get('mockHostDownAfter')) * 1000);
  }
  try { if (typeof window !== 'undefined') window.__win11MockHost = gate; } catch { /* ignore */ }
  return gate;
})();

/** 'taskmgr' / 'win11shell-taskmgr' / 'win11shell-taskmgr.desktop' → 'taskmgr' (host: util.own_app) */
function ownApp(id) {
  const s = String(id ?? '').trim().toLowerCase().replace(/\.desktop$/, '').replace(/^win11shell-/, '');
  return APP_IDS.includes(s) ? s : null;
}

/* ------------------------------------------------------------ shell.restartAll (GX1) */

const PRODUCT = 'Win11 Shell';                       // update.product (nl and en)
const SOFTWARE_ACTIVE = new Set(['queued', 'running', 'authenticating']);
const RESTART_ORIGIN_MS = 2000;                      // the origin restarts last (services/shell/restart.rs)
const RESTART_UP_MS = 1500;

/** update/guard.rs restart_all_blocked, on the mock's own update and software state */
async function restartGuard(m) {
  let s = null;
  try { s = await m.call('update.status', {}); } catch { /* a host without update.* (H28) */ }
  if (s?.phase === 'releaseRunning') {
    throw err('update_busy', L(m, 'Ubuntu wordt bijgewerkt naar een nieuwe versie. Wacht tot het upgradevenster om een herstart vraagt.',
      'Ubuntu is being upgraded to a new version. Wait until the upgrade window asks you to restart.'));
  }
  // an own job (job.json): install or repair, any phase incl. a confirm pause; not a check or a foreign job
  const j = s?.job;
  if (j && !j.foreign && j.role !== 'refresh' && j.finishedAt == null) {
    throw err('update_busy', L(m, `Er worden updates geïnstalleerd. ${PRODUCT} kan opnieuw starten zodra dat klaar is.`,
      `Updates are being installed. ${PRODUCT} can restart as soon as that’s done.`));
  }
  let jobs = [];
  try { jobs = await m.call('software.jobs', {}); } catch { /* no software mock */ }
  // a foreign transaction that is our own Software job (H26 foreignOwn) is an active Software job for the host
  if (j?.foreignOwn || (Array.isArray(jobs) && jobs.some((x) => SOFTWARE_ACTIVE.has(x?.state)))) {
    throw err('software_busy', L(m, 'Er wordt nog een app geïnstalleerd of verwijderd. Probeer het daarna opnieuw.',
      'An app is still being installed or removed. Try again afterwards.'));
  }
}

/** after the restart every process runs the new libraries: the update snapshot's Win11 restart InfoBar goes */
function restarted(m) {
  const s = m.state.update?.snap;                    // mock/update.js state, if the update page was used
  if (!s?.restart?.shell) return;
  s.restart = { ...s.restart, shell: false, shellParts: [] };
  s.rev += 1;
  m.emit('update.changed', { rev: s.rev, phase: s.phase, totals: s.totals, restart: s.restart, job: s.job,
    error: null, release: { state: s.release?.state, version: s.release?.version } });
}

let restartSerial = 0;
let restartUnderWay = null;                          // the nonce of the mock restart until the host is back

function cleanParams(params) {
  if (params == null) return {};
  if (typeof params !== 'object' || Array.isArray(params)) throw err('bad_args', 'Ongeldige invoer.');
  if (JSON.stringify(params).length > 64 * 1024) throw err('bad_args', 'Te veel gegevens.');
  return params;
}

export default {
  methods: hostGate.wrap({
    'shell.info': async (args, m) => {
      const mode = m.params.get('mockMode') || 'serve';
      return {
        mode,
        compositor: m.params.get('mockCompositor') || 'gnome',
        dryRun: true,
        version: '0.1.0',
        layerShell: mode === 'layer',
        layerShellInstalled: mode === 'layer',
        user: { name: 'robin', realName: 'Robin' },
        apps: [...APP_IDS],
        services: Object.fromEntries(SERVICES.map((s) => [s, { ok: true }])),
        dev: false,
        pid: 4242,
        gpu: { mode: 'off', compositing: false, forced: false },
        mainLoopLagMs: hostGate.loopStuck ? 30000 : 20,
        runId: hostGate.runId,
        problem: null,
      };
    },

    'shell.command': async ({ command, args } = {}, m) => {
      if (typeof command !== 'string' || !COMMAND_RE.test(command)) throw err('unknown_command', 'Onbekende opdracht.');
      if (args != null && (typeof args !== 'object' || Array.isArray(args))) throw err('bad_args', 'Ongeldige invoer.');
      if (!KNOWN_COMMANDS.has(command)) console.info('[mock] shell.command (passed on):', command);
      m.emit('shell.command', { command, args: args || {} });
      return { delivered: 1 };
    },

    'shell.setRegions': async ({ rects, keyboard } = {}) => {
      if (rects != null && !Array.isArray(rects)) throw err('bad_args', 'Ongeldige invoer.');
      if (![undefined, null, 'none', 'on-demand', 'exclusive'].includes(keyboard)) throw err('bad_args', 'Ongeldige toetsenbordmodus.');
      return {};
    },

    'shell.openApp': async ({ app, params } = {}, m) => {
      const name = ownApp(app);
      if (!name) throw err('unknown_app', 'Deze app bestaat niet.');
      m.emit('shell.openApp', { app: name, params: cleanParams(params) });
      return {};
    },

    'shell.pickFile': async () => ({ path: null }),

    'shell.restart': async () => {
      setTimeout(() => hostGate.restart(), 50);
      return { ok: true, dryRun: true };
    },
    'shell.quit': async () => ({}),

    'shell.restartAll': async (args = {}, m) => {
      const keys = Object.keys(args || {});
      if (!['libraries', 'update'].includes(args?.reason) || keys.some((k) => k !== 'reason')) {
        throw err('bad_args', L(m, 'Ongeldige invoer.', 'Invalid input.'));
      }
      // a second click while the restart is under way: that restart, no second event (services/shell/restart.rs)
      const dryRun = m.params.get('mockRestartAll') !== 'live';
      if (restartUnderWay) return { ok: true, nonce: restartUnderWay, pending: true, dryRun };
      await restartGuard(m);
      const nonce = `mock-${Date.now()}-${++restartSerial}`;
      restartUnderWay = nonce;
      m.emit('shell.restartAll', { nonce, origin: 4242, at: Date.now(), reason: args.reason });
      setTimeout(() => hostGate.restart(RESTART_UP_MS), RESTART_ORIGIN_MS);
      setTimeout(() => { restartUnderWay = null; restarted(m); }, RESTART_ORIGIN_MS + RESTART_UP_MS + 100);
      return { ok: true, nonce, dryRun };
    },
  }),
  start() {},
};
