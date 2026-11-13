/* Determinism for the UI text dump (H: notes/english-option-design.md §3.3, step E0b; ported to Win11 voor GNOME in
 * GE0a, notes/claude/round0/ge0a.md, which added __i18nFreeze.extend() for the harness actions of dump.html).
 *
 * tests/i18n/serve.py puts this classic script first in /ui/index.html's <head>, so it runs before every other
 * script of the page (the inline theme script included). It makes the page's text a function of the page alone:
 *
 *  - A logical clock. Date and performance.now() show L, a time that only moves when a timer callback runs: a
 *    callback scheduled at L with delay d runs at logical time L + d, whatever the real (or Chrome's virtual) time
 *    is then. Date is Sunday 25 October 2026 14:35:00 Europe/Amsterdam (13:35 UTC) + L; run.sh starts Chrome with
 *    TZ=Europe/Amsterdam. Within one task the clock stands still.
 *  - Timers in logical order. setTimeout/setInterval/requestAnimationFrame/requestIdleCallback callbacks run one per
 *    task, in (logical due time, scheduling order) order, driven by real timers of the same delay (run.sh uses
 *    Chrome's --virtual-time-budget, so those cost no real time). rAF is a 16 ms timer: headless Chrome draws no
 *    frames under virtual time.
 *  - A stop time. ?i18nstop=MS (dump.html sets it per surface): no timer due after MS runs, so the page holds still
 *    at logical time MS and dump.html can take its text once window.__i18nFreeze.idle() says nothing due ≤ MS is
 *    left. A poll that runs every second therefore always ran the same number of times.
 *    __i18nFreeze.extend(MS) moves the stop time to L + MS (dump.html, after a harness action such as a click on a
 *    surface that has no ?open= hook): the timers due up to the new stop run in the same logical order, each after
 *    its own delay.
 *  - A seeded Math.random: one mulberry32 stream per call path (the function name and script path of every stack
 *    frame, never a line or column: an unrelated edit of a file on the path must not move the mock numbers), so a
 *    module's numbers do not depend on the order in which modules or calls happen to interleave. Within one path
 *    the calls come in a fixed order (one synchronous run, timers in logical order).
 *
 *  - A console log for the harness (G, GE0a): console.error / console.warn calls, uncaught errors and unhandled
 *    rejections are collected (message text only, no stacks: they hold the port) and handed to dump.html through
 *    __i18nFreeze.console(). They go to the run's notes, never into the dump. mock/index.js warns about an unknown
 *    method, so a surface that calls a method no mock serves shows up there.
 *
 * Only pages served by serve.py load this file (the real shell never does).
 */
(function () {
  'use strict';
  if (window.__i18nFreeze) return;
  var BASE = Date.UTC(2026, 9, 25, 13, 35, 0);         // Sun 25 Oct 2026 14:35 CET (DST ended at 03:00 that day)
  var STOP = Number(new URLSearchParams(location.search).get('i18nstop')) || Infinity;
  var RealDate = Date;
  var realST = window.setTimeout.bind(window);
  var L = 0;                                            // logical ms since the page started

  /* ---- console log (harness notes, never part of the dump) ---- */
  var LOG = [];
  var fmt = function (v) {
    try {
      if (v && typeof v === 'object' && 'message' in v) return (v.code ? '[' + v.code + '] ' : '') + v.message;
      return typeof v === 'string' ? v : JSON.stringify(v);
    } catch (e) { return String(v); }
  };
  ['error', 'warn'].forEach(function (k) {
    var orig = console[k];
    console[k] = function () {
      try { LOG.push(k + ': ' + Array.prototype.map.call(arguments, fmt).join(' ').replace(/https?:\/\/127\.0\.0\.1:\d+/g, '')); } catch (e) { /* ignore */ }
      return orig.apply(console, arguments);
    };
  });
  window.addEventListener('error', function (e) { LOG.push('uncaught: ' + fmt(e.error || e.message)); });
  window.addEventListener('unhandledrejection', function (e) { LOG.push('unhandled rejection: ' + fmt(e.reason)); });

  /* ---- clock ---- */
  var nowMs = function () { return BASE + L; };
  function FDate(a, b, c, d, e, f, g) {
    var n = arguments.length;
    if (!new.target) return new RealDate(nowMs()).toString();
    if (n === 0) return new RealDate(nowMs());
    if (n === 1) return new RealDate(a);
    return new RealDate(a, b, n > 2 ? c : 1, n > 3 ? d : 0, n > 4 ? e : 0, n > 5 ? f : 0, n > 6 ? g : 0);
  }
  FDate.prototype = RealDate.prototype;
  FDate.now = nowMs;
  FDate.UTC = RealDate.UTC;
  FDate.parse = RealDate.parse;
  try { Object.defineProperty(FDate, 'name', { value: 'Date' }); } catch (e) { /* ignore */ }
  window.Date = FDate;
  try { window.performance.now = function () { return L; }; } catch (e) { /* ignore */ }

  /* ---- timers ---- */
  var Q = new Map();          // id → {due, seq, fn, args, period}
  var ids = 0, seqs = 0;
  function earliest(limit) {
    var best = null;
    Q.forEach(function (e) {
      if (e.due <= limit && e.due <= STOP && (!best || e.due < best.due || (e.due === best.due && e.seq < best.seq))) best = e;
    });
    return best;
  }
  function pump(limit) {      // one callback per task, the earliest due ≤ limit; then continue in a new task
    var e = earliest(limit);
    if (!e) return;
    if (e.due > L) L = e.due;
    if (e.period) { e.due += e.period; e.seq = ++seqs; arm(e.due, e.period); } else Q.delete(e.id);
    if (earliest(limit)) realST(function () { pump(limit); }, 0);
    try { e.fn.apply(window, e.args); } catch (err) { realST(function () { throw err; }, 0); }
  }
  function arm(due, delay) { realST(function () { pump(due); }, delay); }
  function add(fn, delay, args, repeat) {
    if (typeof fn !== 'function') { var src = String(fn); fn = function () { (0, eval)(src); }; }
    var d = Math.max(0, Number(delay) || 0);
    var id = ++ids;
    var e = { id: id, due: L + (repeat ? Math.max(4, d) : d), seq: ++seqs, fn: fn, args: args, period: repeat ? Math.max(4, d) : 0 };
    Q.set(id, e);
    arm(e.due, e.due - L);
    return id;
  }
  var clear = function (id) { Q.delete(id); };
  window.setTimeout = function (fn, delay) { return add(fn, delay, Array.prototype.slice.call(arguments, 2), false); };
  window.setInterval = function (fn, delay) { return add(fn, delay, Array.prototype.slice.call(arguments, 2), true); };
  window.clearTimeout = clear;
  window.clearInterval = clear;
  window.requestAnimationFrame = function (cb) { return add(function () { cb(L); }, 16, [], false); };
  window.cancelAnimationFrame = clear;
  window.requestIdleCallback = function (cb) {
    return add(function () { cb({ didTimeout: false, timeRemaining: function () { return 50; } }); }, 1, [], false);
  };
  window.cancelIdleCallback = clear;

  /* ---- Math.random: a stream per call path ---- */
  function mulberry32(s) {
    return function () {
      s = (s + 0x6D2B79F5) | 0;
      var t = Math.imul(s ^ (s >>> 15), 1 | s);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  function hash(str) {          // FNV-1a
    var h = 0x811c9dc5;
    for (var i = 0; i < str.length; i++) { h ^= str.charCodeAt(i); h = Math.imul(h, 0x01000193); }
    return h >>> 0;
  }
  var streams = new Map();
  // One frame of the key: "function@/path" — never the line or the column, so an edit elsewhere in a file (a comment
  // line in bridge.js, a new import in theme.js) keeps every stream and the mock numbers stay the same.
  var V8 = /^\s*at\s+(?:async\s+)?(?:(.*?)\s+\()?(\/[^\s()]*?):\d+:\d+\)?\s*$/;   // "at fn (/x.js:1:2)", "at /x.js:1:2"
  var JSC = /^(?:(.*?)@)?(\/[^\s()@]*?):\d+:\d+\s*$/;                             // "fn@/x.js:1:2", "/x.js:1:2"
  function path() {
    var lim = Error.stackTraceLimit;
    Error.stackTraceLimit = 16;
    var stack = String(new Error().stack || '');
    Error.stackTraceLimit = lim;
    var out = [];
    stack.split('\n').forEach(function (l) {
      if (l.indexOf('/tests/i18n/freeze.js') >= 0) return;
      l = l.replace(/[a-z]+:\/\/[^/\s]+/gi, '');                 // scheme://127.0.0.1:PORT (a new port per run)
      var m = V8.exec(l) || JSC.exec(l);
      if (m) out.push((m[1] || '') + '@' + m[2]);
    });
    return out.join('<') || '?';
  }
  Math.random = function () {
    var k = path();
    var st = streams.get(k);
    if (!st) { st = mulberry32(hash(k)); streams.set(k, st); }
    return st();
  };

  window.__i18nFreeze = {
    base: BASE, stop: STOP,
    now: function () { return L; },
    /** nothing left to run up to the stop time (or up to `t`) */
    idle: function (t) { return !earliest(t === undefined ? STOP : t); },
    pending: function () { return Q.size; },
    /** console.error/warn calls, uncaught errors and unhandled rejections so far (harness notes) */
    console: function () { return LOG.slice(); },
    /** move the stop time to L + ms (a harness action, dump.html). The timers held back by the old stop time are
     *  armed again with their own delay, so logical time keeps moving with Chrome's virtual time (which waits for
     *  pending fetches) instead of jumping ahead of a module or style sheet that is still loading. */
    extend: function (ms) {
      STOP = L + Math.max(0, Number(ms) || 0);
      window.__i18nFreeze.stop = STOP;
      Q.forEach(function (e) { if (e.due <= STOP) arm(e.due, Math.max(0, e.due - L)); });
      return STOP;
    },
  };
})();
