// Taakbeheer — Samenvatting: plain-language health hero, one-click fixes, five live
// mini graphs, the apps with the most impact and the disks with their free space.

import { h, clear } from '../../../core/dom.js';
import { Button, AppIcon, severityIcon, Toast } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { Sparkline } from '../../../core/charts.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, bitrate, time } from '../../../core/format.js';
import { kindColor, ghz, statusGlyph, cpuPct, memMB, volPct } from '../util.js';

const t = makeT({
  nl: {
    title: 'Samenvatting', newTask: 'Nieuwe taak', recheck: 'Opnieuw controleren',
    okTitle: 'Je pc werkt prima',
    okBody: 'Processor, geheugen, schijven en services zijn gecontroleerd. Alles ziet er goed uit.',
    warnTitle: { one: 'Er vraagt 1 ding je aandacht', other: 'Er vragen {n} dingen je aandacht' },
    warnBody: 'Hieronder staat wat er aan de hand is. Met één klik los je het op.',
    checked: 'Gecontroleerd om {time}', checking: 'Bezig met controleren…',
    now: 'Nu in gebruik', topApps: 'Apps die het meest gebruiken', allProcs: 'Alle processen', disks: 'Schijven', allStorage: 'Opslag bekijken',
    goodToKnow: 'Goed om te weten', allGood: 'Alles in orde',
    cpu: 'Processor', memory: 'Geheugen', disk: 'Schijf', gpu: 'Videokaart', network: 'Netwerk',
    ofGB: '{used} van {total}', free: '{free} vrij van {total}', almostFull: 'Bijna vol',
    impactHigh: 'Hoog', impactMid: 'Gemiddeld', impactLow: 'Laag', impact: 'Invloed',
    sysDisk: 'Systeemschijf', noGpu: 'Geen gegevens', down: '↓ {v}', up: '↑ {v}',
    noApps: 'Geen apps geopend.', loading: 'Laden…', openFailed: 'Kon dit niet openen',
    aProcesses: 'Processen bekijken', aProcess: 'Proces bekijken', aMemory: 'Geheugengebruik bekijken', aPerformance: 'Prestaties bekijken',
    aServices: 'Services bekijken', aStorage: 'Opslag bekijken', aFree: 'Ruimte vrijmaken', aStartup: 'Opstart-apps bekijken', aDetails: 'Details bekijken',
  },
  en: {
    title: 'Summary', newTask: 'Run new task', recheck: 'Check again',
    okTitle: 'Your PC is running fine',
    okBody: 'Processor, memory, disks and services were checked. Everything looks good.',
    warnTitle: { one: '1 thing needs your attention', other: '{n} things need your attention' },
    warnBody: 'Here’s what’s going on. Fix it with one click.',
    checked: 'Checked at {time}', checking: 'Checking…',
    now: 'In use now', topApps: 'Apps using the most', allProcs: 'All processes', disks: 'Disks', allStorage: 'View storage',
    goodToKnow: 'Good to know', allGood: 'All good',
    cpu: 'Processor', memory: 'Memory', disk: 'Disk', gpu: 'Graphics card', network: 'Network',
    ofGB: '{used} of {total}', free: '{free} free of {total}', almostFull: 'Almost full',
    impactHigh: 'High', impactMid: 'Medium', impactLow: 'Low', impact: 'Impact',
    sysDisk: 'System disk', noGpu: 'No data', down: '↓ {v}', up: '↑ {v}',
    noApps: 'No apps open.', loading: 'Loading…', openFailed: 'Couldn’t open this',
    aProcesses: 'View processes', aProcess: 'View process', aMemory: 'View memory use', aPerformance: 'View performance',
    aServices: 'View services', aStorage: 'View storage', aFree: 'Free up space', aStartup: 'View startup apps', aDetails: 'View details',
  },
});

const SEV_ORDER = { critical: 0, warning: 1, info: 2, ok: 3 };

export function createPage(ctx) {
  const { store } = ctx;
  const H = store.history;
  const offs = [];
  let health = null, perf = null, procs = null, vols = null;

  /* ---------------------------------------------------------------- hero */
  const glyphWrap = h('div', { class: 'tm-hero__glyph' });
  const heroTitle = h('h2', { class: 'tm-hero__title' }, t('loading'));
  const heroBody = h('p', { class: 'tm-hero__body' });
  const heroMeta = h('div', { class: 'tm-hero__meta caption' });
  const recheckBtn = Button({ label: t('recheck'), icon: 'refresh', onClick: () => recheck() });
  const hero = h('section', { class: 'tm-hero', 'aria-live': 'polite' }, glyphWrap,
    h('div', { class: 'tm-hero__text' }, heroTitle, heroBody),
    h('div', { class: 'tm-hero__side' }, heroMeta, recheckBtn));
  const attention = h('div', { class: 'tm-attn' });

  /* -------------------------------------------------------- live tiles */
  const tileDefs = [
    { id: 'cpu', kind: 'cpu', label: t('cpu'), device: 'cpu' },
    { id: 'memory', kind: 'memory', label: t('memory'), device: 'memory' },
    { id: 'disk', kind: 'disk', label: t('disk') },
    { id: 'gpu', kind: 'gpu', label: t('gpu'), device: 'gpu:0' },
    { id: 'network', kind: 'network', label: t('network') },
  ];
  const tiles = tileDefs.map((d) => {
    const value = h('div', { class: 'tm-mini__value' }, '–');
    const sub = h('div', { class: 'tm-mini__sub' });
    const canvas = h('canvas', { class: 'tm-mini__spark', 'aria-hidden': 'true' });
    const el = h('button', { type: 'button', class: 'tm-mini', dataset: { kind: d.kind } },
      h('div', { class: 'tm-mini__head' }, h('span', { class: 'tm-mini__dot' }), h('span', null, d.label), icon('chevron-right', { size: 12 })),
      value, sub, h('div', { class: 'tm-mini__graph' }, canvas));
    const spark = Sparkline(canvas, { values: [], color: kindColor(d.kind), max: d.kind === 'network' ? null : 100, points: 60, lineWidth: 1.5 });
    const rec = { ...d, el, value, sub, spark };
    el.addEventListener('click', () => ctx.navigate('performance', { device: rec.device || 'cpu' }));
    return rec;
  });
  const tileRow = h('div', { class: 'tm-minis' }, tiles.map((x) => x.el));

  /* --------------------------------------------------- apps + volumes */
  const appsList = h('div', { class: 'tm-list' }, h('div', { class: 'tm-list__empty' }, t('loading')));
  const volsList = h('div', { class: 'tm-list' }, h('div', { class: 'tm-list__empty' }, t('loading')));
  const goodList = h('div', { class: 'tm-good' });
  const linkBtn = (label, onClick) => h('button', { type: 'button', class: 'fl-link tm-section__link', onClick }, label, icon('chevron-right', { size: 12 }));

  const el = h('div', { class: 'tm-scroll tm-summary' },
    hero, attention,
    h('section', { class: 'tm-section' }, h('h3', { class: 'tm-section__title' }, t('now')), tileRow),
    h('div', { class: 'tm-two' },
      h('section', { class: 'tm-section' },
        h('div', { class: 'tm-section__head' }, h('h3', { class: 'tm-section__title' }, t('topApps')), linkBtn(t('allProcs'), () => ctx.navigate('processes'))),
        h('div', { class: 'tm-card' }, appsList)),
      h('section', { class: 'tm-section' },
        h('div', { class: 'tm-section__head' }, h('h3', { class: 'tm-section__title' }, t('disks')), linkBtn(t('allStorage'), () => ctx.navigate('storage'))),
        h('div', { class: 'tm-card' }, volsList))),
    h('section', { class: 'tm-section' }, h('h3', { class: 'tm-section__title' }, t('goodToKnow')), goodList));

  /* --------------------------------------------------------------- render */
  // methods a health action may ask us to call directly (the host suggests them)
  const ACTION_METHODS = new Set(['system.openUpdater']);
  const isOwn = (a) => !a.app || a.app === 'taskmgr' || a.app === 'win11shell-taskmgr.desktop';
  /** Params for an action that stays inside Taakbeheer (never open a second Taakbeheer through shell.openApp). */
  function ownParams(it) {
    const p = { ...(it.action?.params || {}) };
    if (!p.page) p.page = p.pid ? 'processes' : 'summary';
    if (p.page === 'storage' && !p.tab && !p.section) {
      // "Ruimte vrijmaken" for the system/home disk and for a big Prullenbak lands on Opruimen (like Windows' Opschonen)
      const v = p.volume ? (vols || []).find((x) => x.id === p.volume || x.mount === p.volume) : null;
      if (String(it.id || '').startsWith('trash') || !p.volume || v?.isSystem || v?.isHome) p.tab = 'cleanup';
    }
    return p;
  }
  /** Button label: the host's text, except "Taakbeheer openen"-style labels, which make no sense in here. */
  function actionLabel(it) {
    const a = it.action;
    if (!isOwn(a)) return a.label;
    const p = ownParams(it);
    const generic = !a.label || /taakbeheer|task manager/i.test(a.label);
    if (!generic) return a.label;
    switch (p.page) {
      case 'processes': return p.pid ? t('aProcess') : p.sort === 'memory' ? t('aMemory') : t('aProcesses');
      case 'performance': return t('aPerformance');
      case 'services': return t('aServices');
      case 'storage': return p.tab === 'cleanup' ? t('aFree') : t('aStorage');
      case 'startup': return t('aStartup');
      case 'details': return t('aDetails');
      default: return a.label || t('aProcesses');
    }
  }
  function runAction(it) {
    const a = it?.action;
    if (!a) return;
    if (a.method && ACTION_METHODS.has(a.method)) {
      ctx.bridge.call(a.method, {}).catch((e) => Toast.show({ title: t('openFailed'), message: e.message, severity: 'error' }));
      return;
    }
    if (isOwn(a)) { const p = ownParams(it); ctx.navigate(p.page, p); return; }
    ctx.bridge.call('shell.openApp', { app: a.app, params: a.params || {} })
      .catch((e) => Toast.show({ title: t('openFailed'), message: e.message, severity: 'error' }));
  }

  function renderHealth() {
    if (!health) return;
    const items = [...health].sort((a, b) => (SEV_ORDER[a.severity] ?? 9) - (SEV_ORDER[b.severity] ?? 9));
    const attn = items.filter((i) => i.severity === 'warning' || i.severity === 'critical');
    const infos = items.filter((i) => i.severity === 'info');
    const oks = items.filter((i) => i.severity === 'ok');
    const sev = attn.some((i) => i.severity === 'critical') ? 'critical' : attn.length ? 'warning' : 'ok';
    hero.dataset.severity = sev;
    clear(glyphWrap).append(statusGlyph(sev === 'ok' ? 'ok' : sev, 56));
    heroTitle.textContent = attn.length ? t('warnTitle', { n: attn.length }) : t('okTitle');
    heroBody.textContent = attn.length ? t('warnBody') : t('okBody');
    heroMeta.textContent = t('checked', { time: time(Date.now()) });

    clear(attention);
    for (const it of [...attn, ...infos]) {
      const btn = it.action ? Button({ label: actionLabel(it), variant: it.severity === 'info' ? 'standard' : 'accent', onClick: () => runAction(it) }) : null;
      attention.append(h('article', { class: ['tm-attn__card', `is-${it.severity}`] },
        h('span', { class: 'tm-attn__icon' }, severityIcon(it.severity === 'critical' ? 'error' : it.severity === 'warning' ? 'warning' : 'info', 20)),
        h('div', { class: 'tm-attn__text' }, h('div', { class: 'tm-attn__title' }, it.title), h('div', { class: 'tm-attn__msg' }, it.message)),
        btn ? h('div', { class: 'tm-attn__action' }, btn) : null));
    }
    attention.hidden = !attention.childElementCount;

    clear(goodList);
    if (!oks.length) goodList.append(h('div', { class: 'tm-list__empty' }, t('allGood')));
    for (const it of oks) {
      goodList.append(h('div', { class: 'tm-good__item' },
        h('span', { class: 'tm-good__icon' }, severityIcon('success', 16)),
        h('div', null, h('div', { class: 'tm-good__title' }, it.title), h('div', { class: 'tm-good__msg' }, it.message))));
    }
  }

  async function recheck() {
    heroMeta.textContent = t('checking');
    recheckBtn.setDisabled(true);
    try { await store.refresh('health'); await store.refresh('volumes'); } finally { recheckBtn.setDisabled(false); }
  }

  function sysDisk(p) {
    return (p.disks || []).find((d) => (d.mounts || []).includes('/')) || p.disks?.[0];
  }
  function primaryNet(p) {
    const list = p.network || [];
    return list.find((n) => n.kind === 'ethernet' || n.kind === 'wifi') || list[0];
  }

  function renderPerf() {
    if (!perf) return;
    const p = perf;
    for (const tl of tiles) {
      tl.spark.setColor(kindColor(tl.kind));
      let v = '–', sub = '', series = [];
      if (tl.id === 'cpu') {
        v = `${Math.round(p.cpu.usage)}%`;
        sub = [ghz(p.cpu.freqMHz), p.cpu.temperature != null ? `${Math.round(p.cpu.temperature)} °C` : ''].filter(Boolean).join(' · ');
        series = H.cpu;
      } else if (tl.id === 'memory') {
        v = `${Math.round((p.memory.used / p.memory.total) * 100)}%`;
        sub = t('ofGB', { used: bytes(p.memory.used, { digits: 1 }), total: bytes(p.memory.total, { digits: 1 }) });
        series = H.mem;
      } else if (tl.id === 'disk') {
        const d = sysDisk(p);
        if (d) {
          v = `${Math.round(d.activePct)}%`;
          sub = `${t('sysDisk')} · ${bytes((d.readBps || 0) + (d.writeBps || 0))}/s`;
          series = H.disks.get(d.name)?.active || [];
          tl.device = `disk:${d.name}`;
        }
      } else if (tl.id === 'gpu') {
        const g = p.gpus?.[0];
        if (g && g.usage != null) {
          v = `${Math.round(g.usage)}%`;
          sub = [String(g.name || '').replace(/^(NVIDIA|AMD|Intel)\s+(GeForce\s+|Radeon\s+)?/i, ''), g.temperature != null ? `${Math.round(g.temperature)} °C` : ''].filter(Boolean).join(' · ');
          series = H.gpus.get(0)?.usage || [];
        } else sub = t('noGpu');
        tl.el.hidden = !g;
      } else if (tl.id === 'network') {
        const n = primaryNet(p);
        if (n) {
          v = bitrate(n.rxBps);
          sub = `${t('up', { v: bitrate(n.txBps) })} · ${n.name || n.iface}`;
          const hn = H.nets.get(n.iface);
          series = hn?.rx || [];
          tl.device = `net:${n.iface}`;
        }
      }
      if (tl.value.textContent !== v) tl.value.textContent = v;
      if (tl.sub.textContent !== sub) tl.sub.textContent = sub;
      tl.spark.update(series.slice(-60));
    }
  }

  function impactOf(a, total) {
    return (a.cpu || 0) / 8 + (a.memory || 0) / (total * 0.06);
  }

  function renderApps() {
    if (!procs) return;
    const total = procs.totals?.memoryTotal || 1;
    const apps = [...(procs.apps || [])].sort((a, b) => impactOf(b, total) - impactOf(a, total)).slice(0, 5);
    clear(appsList);
    if (!apps.length) { appsList.append(h('div', { class: 'tm-list__empty' }, t('noApps'))); return; }
    const maxI = Math.max(0.5, ...apps.map((a) => impactOf(a, total)));
    for (const a of apps) {
      const imp = impactOf(a, total);
      const level = imp > 1.2 ? 'high' : imp > 0.45 ? 'mid' : 'low';
      const row = h('button', { type: 'button', class: 'tm-app-row' },
        AppIcon({ name: a.icon, size: 24, fallback: 'app' }),
        h('div', { class: 'tm-app-row__text' },
          h('div', { class: 'tm-app-row__name' }, a.name),
          h('div', { class: 'tm-app-row__meta' }, `CPU ${cpuPct(a.cpu)} · ${memMB(a.memory)}`)),
        h('div', { class: 'tm-impact' },
          h('div', { class: ['tm-impact__label', `is-${level}`] }, t(level === 'high' ? 'impactHigh' : level === 'mid' ? 'impactMid' : 'impactLow')),
          h('div', { class: 'tm-impact__bar' }, h('span', { class: `is-${level}`, style: { width: `${Math.max(4, (imp / maxI) * 100)}%` } }))));
      row.addEventListener('click', () => ctx.navigate('processes', { pid: a.pids?.[0] }));
      appsList.append(row);
    }
  }

  function renderVols() {
    if (!vols) return;
    clear(volsList);
    const list = vols.filter((v) => !v.hidden).sort((a, b) => (b.isSystem - a.isSystem) || (volPct(b) - volPct(a)));
    // the friendly name only (like Verkenner); the mount folder goes in the tooltip, unless two drives share a name
    const seen = new Map();
    for (const v of list) seen.set(v.name, (seen.get(v.name) || 0) + 1);
    for (const v of list) {
      const pct = volPct(v);
      const level = pct >= 97 ? 'critical' : pct >= 90 ? 'warning' : 'ok';
      const row = h('button', { type: 'button', class: ['tm-vol-row', `is-${level}`], dataset: { tooltip: v.mount } },
        AppIcon({ name: v.removable ? 'drive-removable-media' : 'drive-harddisk', size: 24, fallback: 'drive' }),
        h('div', { class: 'tm-vol-row__text' },
          h('div', { class: 'tm-vol-row__top' },
            h('span', { class: 'tm-vol-row__name' }, v.name, seen.get(v.name) > 1 ? h('span', { class: 'tm-vol-row__mount' }, ` (${v.mount})`) : null),
            level !== 'ok' ? h('span', { class: 'tm-pill is-warning' }, t('almostFull')) : null),
          h('div', { class: 'tm-bar' }, h('span', { style: { width: `${Math.min(100, pct)}%` } })),
          h('div', { class: 'tm-vol-row__meta' }, t('free', { free: bytes(v.free), total: bytes(v.total) }))),
        h('div', { class: 'tm-vol-row__pct' }, `${Math.round(pct)}%`));
      row.addEventListener('click', () => ctx.navigate('storage', { volume: v.id || v.mount }));
      volsList.append(row);
    }
  }

  return {
    id: 'summary',
    title: t('title'),
    el,
    commands: [Button({ label: t('newTask'), icon: 'new', variant: 'subtle', onClick: () => ctx.newTask() })],
    mount() {
      offs.push(store.subscribe('health', (d) => { health = d; renderHealth(); }));
      offs.push(store.subscribe('performance', (d) => { perf = d; renderPerf(); }));
      offs.push(store.subscribe('processes', (d) => { procs = d; renderApps(); }));
      offs.push(store.subscribe('volumes', (d) => { vols = d; renderVols(); }));
    },
    unmount() { while (offs.length) offs.pop()(); },
    refresh() { return Promise.all([store.refresh('health'), store.refresh('performance'), store.refresh('processes'), store.refresh('volumes')]); },
  };
}
