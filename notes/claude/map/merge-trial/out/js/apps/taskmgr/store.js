// Taakbeheer — data store: polls sysmon/storage only while a page needs the data,
// at the chosen update speed, pauses while the window is hidden and keeps the
// performance history (for the graphs) in memory. Every poll has a timeout (hostcall.js); while Win11 Shell
// does not answer, the last data stays and polling slows down — except for a channel its worker still answers
// directly (hostcall.js "direct routes": the process list keeps its speed while the host is frozen); when it is
// back, every open channel refreshes.
//
//   const off = store.subscribe('performance', (data) => …)   // starts polling
//   store.get('processes')   store.refresh('processes')   store.history
//   store.onSpeed((ms) => …)

import { bridge, health, direct } from './hostcall.js';
import { getSetting, onSettings } from '../../core/theme.js';
import { lang } from '../../core/i18n.js';      // (last: hostcall.js has loaded it already, the module order stays)

export const SPEEDS = { high: 500, normal: 1000, low: 4000, paused: 0 };
const DOWN_INTERVAL = 2500; // realtime polls while the host does not answer
const MAX_HISTORY = 1200; // samples per series (10 min at 0.5 s)

const CHANNELS = {
  processes: { method: 'sysmon.processes', realtime: true },
  performance: { method: 'sysmon.performance', realtime: true },
  health: { method: 'sysmon.health', every: 30000 },
  volumes: { method: 'storage.volumes', every: 60000 },
  startup: { method: 'sysmon.startup', every: 0 },
  services: { method: 'sysmon.services', every: 10000 },
};

function push(arr, v) {
  arr.push(v);
  if (arr.length > MAX_HISTORY) arr.splice(0, arr.length - MAX_HISTORY);
}

export function createStore() {
  const ch = new Map();
  for (const [name, def] of Object.entries(CHANNELS)) {
    ch.set(name, { name, ...def, subs: new Set(), errSubs: new Set(), timer: 0, data: null, error: null, inflight: null, inflightLang: null, gen: 0, lastAt: 0, lastTry: 0 });
  }

  const history = {
    t: [],
    cpu: [], cpuCores: [], freq: [],
    mem: [], memUsed: [],
    disks: new Map(),  // name → {active, read, write}
    nets: new Map(),   // iface → {rx, tx}
    gpus: new Map(),   // index → {usage, mem}
    count: 0,
  };

  const speedSubs = new Set();
  let speedKey = getSetting('taskmgr.updateSpeed', 'normal');
  if (!(speedKey in SPEEDS)) speedKey = 'normal';

  function speedMs() { return SPEEDS[speedKey] ?? 1000; }

  function intervalOf(c) {
    if (c.realtime) {
      const ms = speedMs();
      return ms && health.state !== 'ok' && !direct.live(c.method) ? Math.max(ms, DOWN_INTERVAL) : ms;
    }
    return c.every;
  }

  function recordPerformance(p) {
    if (!p || !p.cpu) return;
    push(history.t, p.timestamp || Date.now());
    push(history.cpu, p.cpu.usage ?? 0);
    push(history.freq, p.cpu.freqMHz ?? 0);
    (p.cpu.perCore || []).forEach((v, i) => {
      if (!history.cpuCores[i]) history.cpuCores[i] = [];
      push(history.cpuCores[i], v ?? 0);
    });
    const m = p.memory || {};
    push(history.memUsed, m.used || 0);
    push(history.mem, m.total ? ((m.used || 0) / m.total) * 100 : 0);
    for (const d of p.disks || []) {
      if (!history.disks.has(d.name)) history.disks.set(d.name, { active: [], read: [], write: [] });
      const hd = history.disks.get(d.name);
      push(hd.active, d.activePct ?? 0); push(hd.read, d.readBps ?? 0); push(hd.write, d.writeBps ?? 0);
    }
    for (const n of p.network || []) {
      if (!history.nets.has(n.iface)) history.nets.set(n.iface, { rx: [], tx: [] });
      const hn = history.nets.get(n.iface);
      push(hn.rx, n.rxBps ?? 0); push(hn.tx, n.txBps ?? 0);
    }
    (p.gpus || []).forEach((g, i) => {
      if (!history.gpus.has(i)) history.gpus.set(i, { usage: [], mem: [] });
      const hg = history.gpus.get(i);
      push(hg.usage, g.usage ?? 0);
      push(hg.mem, g.memTotal ? ((g.memUsed || 0) / g.memTotal) * 100 : 0);
    });
    history.count++;
  }

  // A poll is sent in the language of that moment (hostcall.js's X-Shell-Lang): one still running after a language
  // switch is not reused, and its late answer is dropped (a newer poll in the new language owns the channel).
  async function poll(c) {
    const l = lang();
    if (c.inflight && c.inflightLang === l) return c.inflight;
    const gen = ++c.gen;
    c.inflightLang = l;
    c.inflight = (async () => {
      try {
        const data = await bridge.call(c.method, {}, { source: c.name });
        if (gen !== c.gen) return c.data;
        c.data = data;
        c.error = null;
        c.lastAt = Date.now();
        if (c.name === 'performance') recordPerformance(data);
        for (const cb of [...c.subs]) { try { cb(data); } catch (e) { console.error(`[taskmgr] ${c.name} subscriber failed`, e); } }
      } catch (e) {
        if (gen !== c.gen) return c.data;
        c.error = e;
        console.warn(`[taskmgr] ${c.method} failed`, e);
        for (const cb of [...c.errSubs]) { try { cb(e); } catch { /* ignore */ } }
      } finally {
        if (gen === c.gen) {
          c.inflight = null;
          c.lastTry = Date.now();
        }
      }
      return c.data;
    })();
    return c.inflight;
  }

  function schedule(c) {
    clearTimeout(c.timer);
    c.timer = 0;
    if (!c.subs.size || document.hidden) return;
    const iv = intervalOf(c);
    if (!iv) return; // paused / one-shot
    // after a failure wait a full interval (not "overdue since the last success"): no back-to-back retries
    const due = Math.max(30, (c.error ? c.lastTry : c.lastAt) + iv - Date.now());
    c.timer = setTimeout(async () => {
      c.timer = 0;
      await poll(c);
      schedule(c);
    }, due);
  }

  function start(c) {
    const fresh = c.data && Date.now() - c.lastAt < Math.max(400, intervalOf(c) || 0);
    if (!fresh) poll(c).then(() => schedule(c));
    else schedule(c);
  }

  // back from "reageert niet": fetch everything that is on screen now
  health.onChange((state, info) => {
    if (state !== 'ok' || info.prev === 'ok') return;
    for (const c of ch.values()) if (c.subs.size) { clearTimeout(c.timer); c.timer = 0; poll(c).then(() => schedule(c)); }
  });

  document.addEventListener('visibilitychange', () => {
    for (const c of ch.values()) {
      if (document.hidden) { clearTimeout(c.timer); c.timer = 0; } else if (c.subs.size) start(c);
    }
  });

  onSettings(() => {
    const k = getSetting('taskmgr.updateSpeed', 'normal');
    if (k === speedKey || !(k in SPEEDS)) return;
    speedKey = k;
    for (const c of ch.values()) if (c.realtime && c.subs.size) { clearTimeout(c.timer); c.timer = 0; if (speedMs()) schedule(c); }
    for (const cb of [...speedSubs]) cb(speedMs(), speedKey);
  });

  return {
    history,
    /** subscribe(name, cb, onError?) → unsubscribe. Starts polling on the first subscriber. */
    subscribe(name, cb, onError) {
      const c = ch.get(name);
      if (!c) throw new Error(`unknown channel ${name}`);
      c.subs.add(cb);
      if (onError) c.errSubs.add(onError);
      if (c.data) { try { cb(c.data); } catch (e) { console.error(e); } }
      if (c.subs.size === 1) start(c);
      return () => {
        c.subs.delete(cb);
        if (onError) c.errSubs.delete(onError);
        if (!c.subs.size) { clearTimeout(c.timer); c.timer = 0; }
      };
    },
    get(name) { return ch.get(name)?.data ?? null; },
    /** Push data that arrived as an event (e.g. storage.volumesChanged). */
    inject(name, data) {
      const c = ch.get(name);
      if (!c || data == null) return;
      c.data = data; c.lastAt = Date.now();
      for (const cb of [...c.subs]) { try { cb(data); } catch (e) { console.error(e); } }
    },
    error(name) { return ch.get(name)?.error ?? null; },
    /** Poll now (F5, after an action). Resolves with the new data. */
    async refresh(name) {
      if (!name) return Promise.all([...ch.values()].filter((c) => c.subs.size).map((c) => poll(c).then(() => schedule(c))));
      const c = ch.get(name);
      if (!c) return null;
      const d = await poll(c);
      schedule(c);
      return d;
    },
    /** The language changed: cached answers hold the host's texts in the old one. Poll what is on screen now;
     *  a channel subscribed later polls on its first subscriber (nothing counts as fresh any more). */
    langChanged() {
      for (const c of ch.values()) {
        c.lastAt = 0;
        if (c.subs.size) { clearTimeout(c.timer); c.timer = 0; poll(c).then(() => schedule(c)); }
      }
    },
    /** Fetch once without subscribing (uses the cache when it is recent). */
    async fetch(name, maxAge = 2000) {
      const c = ch.get(name);
      if (c.data && Date.now() - c.lastAt < maxAge) return c.data;
      return poll(c);
    },
    speedMs,
    get speedKey() { return speedKey; },
    get paused() { return speedMs() === 0; },
    onSpeed(cb) { speedSubs.add(cb); return () => speedSubs.delete(cb); },
  };
}
