// Taakbeheer — Prestaties: device list with live mini graphs (left) and the big graph +
// Win11 stats grid for the selected device (right). Colour-keyed per device type.

import { h, clear } from '../../../core/dom.js';
import { Button, Toast } from '../../../core/components.js';
import { showContextMenu } from '../../../core/contextmenu.js';
import { LineChart, resolveColor } from '../../../core/charts.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, rate, bitrate, number, uptime as fmtUptime } from '../../../core/format.js';
import { getSetting, setSetting, onSettings } from '../../../core/theme.js';
import { kindColor, ghz, load, save } from '../util.js';

const t = makeT({
  nl: {
    title: 'Prestaties', newTask: 'Nieuwe taak', view: 'Weergave', copy: 'Kopiëren', copied: 'Gegevens gekopieerd',
    graph: 'Grafiek', overall: 'Totaal gebruik', perCore: 'Per logische processor', duration: 'Grafiekduur',
    s30: '30 seconden', s60: '60 seconden', s120: '2 minuten', s300: '5 minuten',
    cpu: 'CPU', memory: 'Geheugen', disk: 'Schijf {n}', gpu: 'GPU {n}',
    usageOver: '% gebruik over {d}', memUsage: 'Geheugengebruik', memComp: 'Geheugensamenstelling',
    activeTime: 'Actieve tijd', transfer: 'Overdrachtssnelheid schijf', throughput: 'Doorvoer', gpuMem: 'Toegewezen GPU-geheugen',
    usage: 'Gebruik', speed: 'Snelheid', processes: 'Processen', threads: 'Threads', upTime: 'Actief', temp: 'Temperatuur',
    cores: 'Kernen', logical: 'Logische processors', model: 'Model',
    inUse: 'In gebruik', available: 'Beschikbaar', cached: 'In cache', swap: 'Wisselbestand', total: 'Totaal', buffers: 'Buffers', free: 'Vrij',
    read: 'Lezen', write: 'Schrijven', readSpeed: 'Leessnelheid', writeSpeed: 'Schrijfsnelheid', capacity: 'Capaciteit', type: 'Type',
    systemDisk: 'Systeemschijf', yes: 'Ja', no: 'Nee', mounts: 'Koppelpunten',
    send: 'Verzenden', receive: 'Ontvangen', adapter: 'Adapternaam', connType: 'Verbindingstype', ipv4: 'IPv4-adres', ipv6: 'IPv6-adres',
    gpuUsage: 'Gebruik', gpuMemShort: 'GPU-geheugen', gpuTemp: 'GPU-temperatuur', vendor: 'Leverancier', name: 'Naam',
    ssd: 'SSD', hdd: 'Harde schijf', nvme: 'SSD (NVMe)', usb: 'USB-schijf', ethernet: 'Ethernet', wifi: 'Wifi', other: 'Overig',
    loading: 'Gegevens verzamelen…', unknown: 'Onbekend', notAvailable: 'Niet beschikbaar', paused: 'Bijwerken is gepauzeerd (zie Instellingen)',
    send_s: 'Verz.:', recv_s: 'Ontv.:', zero: '0',
    baseSpeed: 'Basissnelheid', maxSpeed: 'Maximale snelheid', committed: 'Toegewezen', response: 'Gem. reactietijd',
    linkSpeed: 'Verbindingssnelheid', iface: 'Interface', driver: 'Stuurprogramma', power: 'Vermogen', clock: 'Kloksnelheid',
    e3d: '3D', encode: 'Video coderen', decode: 'Video decoderen', disconnected: 'Niet verbonden', device: 'Apparaat', modelKv: 'Model',
  },
  en: {
    title: 'Performance', newTask: 'Run new task', view: 'View', copy: 'Copy', copied: 'Details copied',
    graph: 'Graph', overall: 'Overall utilisation', perCore: 'Logical processors', duration: 'Graph length',
    s30: '30 seconds', s60: '60 seconds', s120: '2 minutes', s300: '5 minutes',
    cpu: 'CPU', memory: 'Memory', disk: 'Disk {n}', gpu: 'GPU {n}',
    usageOver: '% Utilisation over {d}', memUsage: 'Memory usage', memComp: 'Memory composition',
    activeTime: 'Active time', transfer: 'Disk transfer rate', throughput: 'Throughput', gpuMem: 'Dedicated GPU memory',
    usage: 'Utilisation', speed: 'Speed', processes: 'Processes', threads: 'Threads', upTime: 'Up time', temp: 'Temperature',
    cores: 'Cores', logical: 'Logical processors', model: 'Model',
    inUse: 'In use', available: 'Available', cached: 'Cached', swap: 'Swap', total: 'Total', buffers: 'Buffers', free: 'Free',
    read: 'Read', write: 'Write', readSpeed: 'Read speed', writeSpeed: 'Write speed', capacity: 'Capacity', type: 'Type',
    systemDisk: 'System disk', yes: 'Yes', no: 'No', mounts: 'Mount points',
    send: 'Send', receive: 'Receive', adapter: 'Adapter name', connType: 'Connection type', ipv4: 'IPv4 address', ipv6: 'IPv6 address',
    gpuUsage: 'Utilisation', gpuMemShort: 'GPU memory', gpuTemp: 'GPU temperature', vendor: 'Vendor', name: 'Name',
    ssd: 'SSD', hdd: 'HDD', nvme: 'SSD (NVMe)', usb: 'USB drive', ethernet: 'Ethernet', wifi: 'Wi-Fi', other: 'Other',
    loading: 'Collecting data…', unknown: 'Unknown', notAvailable: 'Not available', paused: 'Updating is paused (see Settings)',
    send_s: 'S:', recv_s: 'R:', zero: '0',
    baseSpeed: 'Base speed', maxSpeed: 'Maximum speed', committed: 'Committed', response: 'Average response time',
    linkSpeed: 'Link speed', iface: 'Interface', driver: 'Driver', power: 'Power', clock: 'Clock speed',
    e3d: '3D', encode: 'Video encode', decode: 'Video decode', disconnected: 'Not connected', device: 'Device', modelKv: 'Model',
  },
});

const DURATIONS = [30, 60, 120, 300];

/** "227 Kbps", "1,2 Mbps" — compact for the device list. */
function shortRate(bps) {
  let v = (bps || 0) * 8;
  const units = ['bps', 'Kbps', 'Mbps', 'Gbps'];
  let i = 0;
  while (v >= 1000 && i < units.length - 1) { v /= 1000; i++; }
  return `${number(v, v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}
const shortGpu = (name) => String(name || '').replace(/^(NVIDIA|AMD|Intel\(R\)|Intel)\s+(GeForce\s+|Radeon\s+)?/i, '') || name;
const durLabel = (s) => t(`s${s}`) || `${s} s`;

/** A rounded-up "nice" maximum for auto-scaled graphs (bytes/s). */
function niceMax(v, minimum) {
  const x = Math.max(minimum, v * 1.15);
  const p = 10 ** Math.floor(Math.log10(x));
  const m = x / p;
  const step = m <= 1 ? 1 : m <= 2 ? 2 : m <= 5 ? 5 : 10;
  return step * p;
}

function deviceList(p) {
  if (!p) return [];
  const list = [{ id: 'cpu', kind: 'cpu' }, { id: 'memory', kind: 'memory' }];
  (p.disks || []).forEach((d, i) => list.push({ id: `disk:${d.name}`, kind: 'disk', index: d.index ?? i, name: d.name }));
  (p.network || []).forEach((n) => list.push({ id: `net:${n.iface}`, kind: 'network', iface: n.iface }));
  (p.gpus || []).forEach((g, i) => list.push({ id: `gpu:${i}`, kind: 'gpu', index: i }));
  return list;
}

export function createPage(ctx) {
  const { store } = ctx;
  const H = store.history;
  let perf = null;
  let selected = load('perf.device', 'cpu');
  let cpuMode = load('perf.cpuMode', 'overall');
  let tiles = new Map(); // id → {el, chart, title, sub1, sub2}
  let detail = null;     // {id, update(p), charts:[]}
  const offs = [];
  // friendly drive names (storage.volumes: "Systeemschijf", "Virtuele machines", "563 GB-schijf"), the same names
  // Verkenner and Instellingen show, instead of raw mount folders or UUIDs
  let volByMount = null; // null = not loaded yet
  let volsFailed = false;
  function diskTitle(dev, d) {
    const mounts = d?.mounts || [];
    let names = [];
    if (volByMount) names = mounts.map((m) => volByMount.get(m)).filter((v) => v && !v.hidden).map((v) => v.name || v.label).filter(Boolean);
    else if (volsFailed) names = mounts.filter((m) => !/^[0-9A-Fa-f-]{8,}$/.test(m.split('/').pop())).map((m) => (m === '/' ? '/' : m.split('/').pop()));
    names = [...new Set(names)];
    return `${t('disk', { n: dev.index })}${names.length ? ` (${names.join(', ')})` : ''}`;
  }
  function onVolumes(vols) {
    volByMount = new Map((Array.isArray(vols) ? vols : []).map((v) => [v.mount, v]));
    if (!perf) return;
    updateList(perf);
    detail?.update(perf);
  }

  const graphSeconds = () => getSetting('taskmgr.graphSeconds', 60);
  const points = () => Math.max(8, Math.round((graphSeconds() * 1000) / (store.speedMs() || 1000)) + 1);
  const tail = (arr) => (arr || []).slice(-points());
  /** Feed a chart from history arrays: push the newest sample (grid scrolls) or redraw all. */
  function feed(chart, sources, opts = []) {
    const n = H.count, pts = points();
    if (chart._fed === n && chart._pts === pts) return;
    if (chart._fed === n - 1 && chart._pts === pts && chart._len === sources.length) {
      chart.push(sources.map((a) => (a && a.length ? a[a.length - 1] : 0)));
    } else {
      chart.setPoints(pts);
      chart.update(sources.map((a, i) => ({ values: tail(a), ...(opts[i] || {}) })));
    }
    chart._fed = n; chart._pts = pts; chart._len = sources.length;
  }

  const list = h('div', { class: 'tm-perf__list', role: 'listbox', 'aria-label': t('title'), tabindex: 0 });
  const panel = h('div', { class: 'tm-perf__panel' }, h('div', { class: 'tm-perf__loading' }, t('loading')));
  const el = h('div', { class: 'tm-perf' }, list, panel);

  /* ---------------------------------------------------------- left list */
  function seriesFor(dev) {
    switch (dev.kind) {
      case 'cpu': return { src: [H.cpu], max: 100 };
      case 'memory': return { src: [H.mem], max: 100 };
      case 'disk': return { src: [H.disks.get(dev.name)?.active || []], max: 100 };
      case 'network': {
        const hn = H.nets.get(dev.iface) || { rx: [], tx: [] };
        const max = niceMax(Math.max(1, ...tail(hn.rx), ...tail(hn.tx)), 12500);
        return { src: [hn.rx, hn.tx], max, dashed: true };
      }
      case 'gpu': return { src: [H.gpus.get(dev.index)?.usage || []], max: 100 };
      default: return { src: [[]], max: 100 };
    }
  }

  function tileTexts(dev, p) {
    switch (dev.kind) {
      case 'cpu': return [t('cpu'), `${Math.round(p.cpu.usage)}%  ${ghz(p.cpu.freqMHz)}`, p.cpu.temperature != null ? `${Math.round(p.cpu.temperature)} °C` : ''];
      case 'memory': {
        const m = p.memory;
        return [t('memory'), `${bytes(m.used, { digits: 1 })}/${bytes(m.total, { digits: 1 })} (${Math.round((m.used / m.total) * 100)}%)`, ''];
      }
      case 'disk': {
        const d = p.disks.find((x) => x.name === dev.name);
        if (!d) return [t('disk', { n: dev.index }), '', ''];
        return [diskTitle(dev, d), t(d.kind) || d.kind, `${Math.round(d.activePct)}%`];
      }
      case 'network': {
        const n = p.network.find((x) => x.iface === dev.iface);
        if (!n) return ['', '', ''];
        if (n.connected === false) return [n.name || n.iface, n.iface, t('disconnected')];
        return [n.name || n.iface, `${t('send_s')} ${shortRate(n.txBps)}`, `${t('recv_s')} ${shortRate(n.rxBps)}`];
      }
      case 'gpu': {
        const g = p.gpus[dev.index];
        if (!g) return ['', '', ''];
        const temp = g.temperature != null ? ` (${Math.round(g.temperature)} °C)` : '';
        return [t('gpu', { n: dev.index }), shortGpu(g.name), g.usage != null ? `${Math.round(g.usage)}%${temp}` : t('notAvailable')];
      }
      default: return ['', '', ''];
    }
  }

  function buildList(p) {
    const devs = deviceList(p);
    const ids = devs.map((d) => d.id).join('|');
    if (list.dataset.ids === ids) return;
    list.dataset.ids = ids;
    for (const tl of tiles.values()) tl.chart.destroy();
    tiles = new Map();
    clear(list);
    for (const dev of devs) {
      const canvas = h('canvas', { class: 'tm-tile__graph', 'aria-hidden': 'true' });
      const title = h('div', { class: 'tm-tile__title' });
      const sub1 = h('div', { class: 'tm-tile__sub' });
      const sub2 = h('div', { class: 'tm-tile__sub' });
      const el2 = h('div', {
        class: 'tm-tile', role: 'option', tabindex: -1, dataset: { id: dev.id, kind: dev.kind }, 'aria-selected': 'false',
      }, h('div', { class: 'tm-tile__graphwrap' }, canvas), h('div', { class: 'tm-tile__text' }, title, sub1, sub2));
      el2.addEventListener('click', () => select(dev.id, true));
      const color = kindColor(dev.kind);
      const chart = LineChart(canvas, { series: [], max: 100, color, gridlines: false, points: points(), fillAlpha: 0.18 });
      tiles.set(dev.id, { el: el2, chart, title, sub1, sub2, dev });
      list.append(el2);
    }
    if (!tiles.has(selected)) selected = 'cpu';
    markSelected();
  }

  function updateList(p) {
    for (const tl of tiles.values()) {
      const [a, b, c] = tileTexts(tl.dev, p);
      if (tl.title.textContent !== a) {
        tl.title.textContent = a;
        // long drive names are cut off in the narrow list: the full name on hover
        if (tl.dev.kind === 'disk') tl.title.dataset.tooltip = a;
      }
      if (tl.sub1.textContent !== b) tl.sub1.textContent = b;
      if (tl.sub2.textContent !== c) tl.sub2.textContent = c;
      tl.sub2.hidden = !c;
      const s = seriesFor(tl.dev);
      if (tl.max !== s.max) { tl.max = s.max; tl.chart.setMax(s.max); }
      feed(tl.chart, s.src, [{}, { dashed: true }]);
    }
  }

  function markSelected() {
    for (const [id, tl] of tiles) {
      const on = id === selected;
      tl.el.classList.toggle('is-selected', on);
      tl.el.setAttribute('aria-selected', String(on));
      tl.el.tabIndex = on ? 0 : -1;
    }
  }

  function select(id, user = false) {
    if (!tiles.has(id)) return;
    selected = id;
    save('perf.device', id);
    markSelected();
    buildDetail();
    if (user) tiles.get(id).el.focus({ preventScroll: true });
  }

  list.addEventListener('keydown', (e) => {
    const ids = [...tiles.keys()];
    const i = ids.indexOf(selected);
    let j = null;
    if (e.key === 'ArrowDown') j = Math.min(ids.length - 1, i + 1);
    else if (e.key === 'ArrowUp') j = Math.max(0, i - 1);
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = ids.length - 1;
    if (j == null) return;
    e.preventDefault();
    select(ids[j], true);
    tiles.get(ids[j]).el.scrollIntoView({ block: 'nearest' });
  });
  list.addEventListener('focus', (e) => { if (e.target === list) tiles.get(selected)?.el.focus(); });

  /* -------------------------------------------------------- detail panel */
  function graphBlock({ labelLeft, labelRight, kind, height = null, cls = '' }) {
    const canvas = h('canvas', { class: 'tm-graph__canvas' });
    const rightEl = h('span', { class: 'tm-graph__lab' }, labelRight);
    const bottomLeft = h('span', null, durLabel(graphSeconds()));
    const block = h('div', { class: ['tm-graph', cls], style: height ? { '--tm-graph-h': `${height}px` } : null, dataset: { kind } },
      h('div', { class: 'tm-graph__top' }, h('span', { class: 'tm-graph__lab' }, labelLeft), rightEl),
      h('div', { class: 'tm-graph__box' }, canvas),
      h('div', { class: 'tm-graph__bottom' }, bottomLeft, h('span', null, t('zero'))));
    block.canvas = canvas;
    block.setRight = (s) => { if (rightEl.textContent !== s) rightEl.textContent = s; };
    return block;
  }

  function stat(label, cls = '') {
    const v = h('div', { class: 'tm-stat__value' }, '–');
    const el2 = h('div', { class: ['tm-stat', cls] }, h('div', { class: 'tm-stat__label' }, label), v);
    el2.set = (s) => { if (v.textContent !== s) v.textContent = s; };
    return el2;
  }
  function kvRow(label) {
    const v = h('dd', null, '–');
    const row = [h('dt', null, label), v];
    row.set = (s) => { if (v.textContent !== s) v.textContent = s; };
    return row;
  }

  function header(title, right) {
    const r = h('div', { class: 'tm-detail__model' }, right || '');
    const el2 = h('div', { class: 'tm-detail__head' }, h('h2', { class: 'tm-detail__title' }, title), r);
    el2.setRight = (s) => { if (r.textContent !== s) r.textContent = s; };
    return el2;
  }

  function buildDetail() {
    if (detail) detail.charts.forEach((c) => c.destroy());
    detail = null;
    clear(panel);
    if (!perf) { panel.append(h('div', { class: 'tm-perf__loading' }, t('loading'))); return; }
    const dev = tiles.get(selected)?.dev || { id: 'cpu', kind: 'cpu' };
    const color = kindColor(dev.kind);
    const builders = { cpu: cpuDetail, memory: memDetail, disk: diskDetail, network: netDetail, gpu: gpuDetail };
    detail = builders[dev.kind](dev, color);
    detail.root.style.setProperty('--tm-kind', resolveColor(panel, color));
    panel.append(detail.root);
    detail.update(perf);
    if (store.paused) panel.append(h('div', { class: 'tm-perf__paused caption' }, t('paused')));
  }

  function cpuDetail(dev, color) {
    const head = header(t('cpu'), perf.cpu.model || '');
    let graphArea;
    const charts = [];
    if (cpuMode === 'perCore' && (perf.cpu.perCore || []).length) {
      const n = perf.cpu.perCore.length;
      const cols = n <= 4 ? 2 : n <= 8 ? 4 : n <= 16 ? 4 : n <= 32 ? 8 : 8;
      const grid = h('div', { class: 'tm-cores', style: { '--cols': cols } });
      for (let i = 0; i < n; i++) {
        const c = h('canvas', { class: 'tm-graph__canvas' });
        grid.append(h('div', { class: 'tm-cores__cell' }, c));
        charts.push(LineChart(c, { series: [], max: 100, color, points: points(), gridRows: 4 }));
      }
      graphArea = h('div', { class: 'tm-graph tm-graph--grow' },
        h('div', { class: 'tm-graph__top' }, h('span', { class: 'tm-graph__lab' }, t('usageOver', { d: durLabel(graphSeconds()) })), h('span', { class: 'tm-graph__lab' }, '100%')),
        grid,
        h('div', { class: 'tm-graph__bottom' }, h('span', null, durLabel(graphSeconds())), h('span', null, t('zero'))));
    } else {
      graphArea = graphBlock({ labelLeft: t('usageOver', { d: durLabel(graphSeconds()) }), labelRight: '100%', kind: 'cpu', cls: 'tm-graph--grow' });
      charts.push(LineChart(graphArea.canvas, { series: [], max: 100, color, points: points() }));
    }
    const sUse = stat(t('usage'), 'is-big'), sSpeed = stat(t('speed'), 'is-big');
    const sProc = stat(t('processes')), sThr = stat(t('threads')), sUp = stat(t('upTime'));
    const kBase = kvRow(t('baseSpeed')), kMax = kvRow(t('maxSpeed'));
    const kCores = kvRow(t('cores')), kLog = kvRow(t('logical')), kTemp = kvRow(t('temp'));
    const root = h('div', { class: 'tm-detail' }, head, graphArea,
      h('div', { class: 'tm-stats' },
        h('div', { class: 'tm-stats__main' }, h('div', { class: 'tm-stats__row' }, sUse, sSpeed), h('div', { class: 'tm-stats__row' }, sProc, sThr), h('div', { class: 'tm-stats__row' }, sUp)),
        h('dl', { class: 'tm-kv' }, kBase, kMax, kCores, kLog, kTemp)));
    const showKv = (row, on) => row.forEach((n) => { n.hidden = !on; });
    return {
      root, charts,
      update(p) {
        const c = p.cpu;
        head.setRight(c.model || '');
        if (charts.length > 1) charts.forEach((ch, i) => feed(ch, [H.cpuCores[i] || []]));
        else feed(charts[0], [H.cpu]);
        sUse.set(`${Math.round(c.usage)}%`);
        sSpeed.set(ghz(c.freqMHz));
        sProc.set(number(c.processes));
        sThr.set(number(c.threadsTotal));
        sUp.set(fmtUptime(c.uptime));
        kCores.set(String(c.cores ?? '–'));
        kLog.set(String(c.threads ?? '–'));
        kTemp.set(c.temperature != null ? `${Math.round(c.temperature)} °C` : t('notAvailable'));
        showKv(kBase, !!c.baseMHz); if (c.baseMHz) kBase.set(ghz(c.baseMHz));
        showKv(kMax, !!c.maxMHz); if (c.maxMHz) kMax.set(ghz(c.maxMHz));
      },
    };
  }

  function memDetail(dev, color) {
    const head = header(t('memory'), '');
    const g = graphBlock({ labelLeft: t('memUsage'), labelRight: '', kind: 'memory', cls: 'tm-graph--grow' });
    const chart = LineChart(g.canvas, { series: [], max: 100, color, points: points() });
    const segUsed = h('div', { class: 'tm-memcomp__seg is-used' });
    const segCache = h('div', { class: 'tm-memcomp__seg is-cache' });
    const segFree = h('div', { class: 'tm-memcomp__seg is-free' });
    const comp = h('div', { class: 'tm-graph tm-memcomp' },
      h('div', { class: 'tm-graph__top' }, h('span', { class: 'tm-graph__lab' }, t('memComp'))),
      h('div', { class: 'tm-memcomp__bar' }, segUsed, segCache, segFree));
    const sUse = stat(t('inUse'), 'is-big'), sAvail = stat(t('available'), 'is-big');
    const sCache = stat(t('cached')), sSwap = stat(t('swap')), sCommit = stat(t('committed'));
    const kTotal = kvRow(t('total')), kBuf = kvRow(t('buffers'));
    const root = h('div', { class: 'tm-detail' }, head, g, comp,
      h('div', { class: 'tm-stats' },
        h('div', { class: 'tm-stats__main' }, h('div', { class: 'tm-stats__row' }, sUse, sAvail), h('div', { class: 'tm-stats__row' }, sCommit, sCache, sSwap)),
        h('dl', { class: 'tm-kv' }, kTotal, kBuf)));
    return {
      root, charts: [chart],
      update(p) {
        const m = p.memory;
        head.setRight(bytes(m.total, { digits: 1 }));
        g.setRight(bytes(m.total, { digits: 1 }));
        feed(chart, [H.mem]);
        const cache = (m.cached || 0) + (m.buffers || 0);
        const used = m.used || 0;
        const free = Math.max(0, m.total - used - cache);
        segUsed.style.width = `${(used / m.total) * 100}%`;
        segCache.style.width = `${(Math.min(cache, m.total - used) / m.total) * 100}%`;
        segFree.style.width = `${(free / m.total) * 100}%`;
        segUsed.dataset.tooltip = `${t('inUse')}: ${bytes(used)}`;
        segCache.dataset.tooltip = `${t('cached')}: ${bytes(cache)}`;
        segFree.dataset.tooltip = `${t('free')}: ${bytes(free)}`;
        sUse.set(bytes(used, { digits: 1 }));
        sAvail.set(bytes(m.available, { digits: 1 }));
        sCache.set(bytes(cache, { digits: 1 }));
        sSwap.set(m.swapTotal ? `${bytes(m.swapUsed, { digits: 1 })}/${bytes(m.swapTotal, { digits: 1 })}` : t('notAvailable'));
        kTotal.set(bytes(m.total, { digits: 1 }));
        kBuf.set(bytes(m.buffers || 0));
        sCommit.hidden = !(m.committed && m.commitLimit);
        if (m.committed && m.commitLimit) sCommit.set(`${bytes(m.committed, { digits: 1 })}/${bytes(m.commitLimit, { digits: 1 })}`);
      },
    };
  }

  function diskDetail(dev, color) {
    const head = header(t('disk', { n: dev.index }), '');
    const g1 = graphBlock({ labelLeft: t('activeTime'), labelRight: '100%', kind: 'disk', cls: 'tm-graph--grow' });
    const g2 = graphBlock({ labelLeft: t('transfer'), labelRight: '', kind: 'disk', cls: 'tm-graph--half' });
    const c1 = LineChart(g1.canvas, { series: [], max: 100, color, points: points() });
    const c2 = LineChart(g2.canvas, { series: [], max: 1048576, color, points: points(), gridRows: 4 });
    const legend = h('div', { class: 'tm-legend' },
      h('span', { class: 'tm-legend__key' }, h('span', { class: 'tm-legend__line' }), t('read')),
      h('span', { class: 'tm-legend__key' }, h('span', { class: 'tm-legend__line is-dashed' }), t('write')));
    const sAct = stat(t('activeTime'), 'is-big'), sResp = stat(t('response'), 'is-big'), sRead = stat(t('readSpeed'), 'is-big'), sWrite = stat(t('writeSpeed'), 'is-big');
    const kCap = kvRow(t('capacity')), kType = kvRow(t('type')), kSys = kvRow(t('systemDisk')), kMnt = kvRow(t('mounts')), kDev = kvRow(t('device'));
    const root = h('div', { class: 'tm-detail' }, head, g1, g2, legend,
      h('div', { class: 'tm-stats' },
        h('div', { class: 'tm-stats__main' }, h('div', { class: 'tm-stats__row' }, sAct, sResp), h('div', { class: 'tm-stats__row' }, sRead, sWrite)),
        h('dl', { class: 'tm-kv' }, kCap, kType, kSys, kMnt, kDev)));
    return {
      root, charts: [c1, c2],
      update(p) {
        const d = p.disks.find((x) => x.name === dev.name);
        if (!d) return;
        head.querySelector('.tm-detail__title').textContent = diskTitle(dev, d);
        head.setRight(d.model || '');
        const hd = H.disks.get(d.name) || { active: [], read: [], write: [] };
        const rd = tail(hd.read), wr = tail(hd.write);
        const max = niceMax(Math.max(1, ...rd, ...wr), 1048576);
        feed(c1, [hd.active]);
        if (c2._max !== max) { c2._max = max; c2.setMax(max); }
        feed(c2, [hd.read, hd.write], [{}, { dashed: true }]);
        g2.setRight(rate(max).replace(/,0 /, ' '));
        sAct.set(`${Math.round(d.activePct)}%`);
        sRead.set(rate(d.readBps));
        sWrite.set(rate(d.writeBps));
        kCap.set(bytes(d.capacity));
        kType.set(t(d.kind) || d.kind);
        kSys.set((d.mounts || []).includes('/') ? t('yes') : t('no'));
        kMnt.set((d.mounts || []).join(', ') || '–');
        sResp.hidden = d.responseMs == null;
        if (d.responseMs != null) sResp.set(`${number(d.responseMs, 1, 1)} ms`);
        kDev.forEach((n) => { n.hidden = !d.device; });
        if (d.device) kDev.set(`/dev/${d.device}`);
      },
    };
  }

  function netDetail(dev, color) {
    const head = header('', '');
    const g = graphBlock({ labelLeft: t('throughput'), labelRight: '', kind: 'network', cls: 'tm-graph--grow' });
    const chart = LineChart(g.canvas, { series: [], max: 125000, color, points: points() });
    const legend = h('div', { class: 'tm-legend' },
      h('span', { class: 'tm-legend__key' }, h('span', { class: 'tm-legend__line' }), t('receive')),
      h('span', { class: 'tm-legend__key' }, h('span', { class: 'tm-legend__line is-dashed' }), t('send')));
    const sSend = stat(t('send'), 'is-big'), sRecv = stat(t('receive'), 'is-big');
    const kAd = kvRow(t('adapter')), kIf = kvRow(t('iface')), kType = kvRow(t('connType')), kLink = kvRow(t('linkSpeed')), k4 = kvRow(t('ipv4')), k6 = kvRow(t('ipv6'));
    const root = h('div', { class: 'tm-detail' }, head, g, legend,
      h('div', { class: 'tm-stats' },
        h('div', { class: 'tm-stats__main' }, h('div', { class: 'tm-stats__row' }, sSend, sRecv)),
        h('dl', { class: 'tm-kv' }, kAd, kIf, kType, kLink, k4, k6)));
    return {
      root, charts: [chart],
      update(p) {
        const n = p.network.find((x) => x.iface === dev.iface);
        if (!n) return;
        head.querySelector('.tm-detail__title').textContent = n.name || n.iface;
        head.setRight(n.iface);
        const hn = H.nets.get(n.iface) || { rx: [], tx: [] };
        const rx = tail(hn.rx), tx = tail(hn.tx);
        const max = niceMax(Math.max(1, ...rx, ...tx), 12500);
        if (chart._max !== max) { chart._max = max; chart.setMax(max); }
        feed(chart, [hn.rx, hn.tx], [{}, { dashed: true }]);
        g.setRight(bitrate(max).replace(/,0 /, ' '));
        sSend.set(bitrate(n.txBps));
        sRecv.set(bitrate(n.rxBps));
        kAd.set(n.adapter || n.iface);
        kIf.forEach((x) => { x.hidden = !n.adapter; });
        kIf.set(n.iface);
        kType.set(t(n.kind) || n.kind);
        kLink.forEach((x) => { x.hidden = !n.linkSpeedMbps; });
        if (n.linkSpeedMbps) kLink.set(n.linkSpeedMbps >= 1000 ? `${number(n.linkSpeedMbps / 1000, 1)} Gbps` : `${number(n.linkSpeedMbps)} Mbps`);
        k4.set(n.ipv4 || '–');
        k6.set(n.ipv6 || '–');
      },
    };
  }

  function gpuDetail(dev, color) {
    const head = header(t('gpu', { n: dev.index }), '');
    const g1 = graphBlock({ labelLeft: t('usageOver', { d: durLabel(graphSeconds()) }), labelRight: '100%', kind: 'gpu', cls: 'tm-graph--grow' });
    const g2 = graphBlock({ labelLeft: t('gpuMem'), labelRight: '', kind: 'gpu', cls: 'tm-graph--half' });
    const c1 = LineChart(g1.canvas, { series: [], max: 100, color, points: points() });
    const c2 = LineChart(g2.canvas, { series: [], max: 100, color, points: points(), gridRows: 4 });
    const sUse = stat(t('gpuUsage'), 'is-big'), sMem = stat(t('gpuMemShort'), 'is-big'), sTemp = stat(t('gpuTemp'), 'is-big');
    const s3d = stat(t('e3d')), sEnc = stat(t('encode')), sDec = stat(t('decode'));
    const kName = kvRow(t('name')), kVendor = kvRow(t('vendor')), kDriver = kvRow(t('driver')), kPower = kvRow(t('power')), kClock = kvRow(t('clock'));
    const engines = h('div', { class: 'tm-stats__row' }, s3d, sEnc, sDec);
    const root = h('div', { class: 'tm-detail' }, head, g1, g2,
      h('div', { class: 'tm-stats' },
        h('div', { class: 'tm-stats__main' }, h('div', { class: 'tm-stats__row' }, sUse, sMem, sTemp), engines),
        h('dl', { class: 'tm-kv' }, kName, kVendor, kDriver, kPower, kClock)));
    return {
      root, charts: [c1, c2],
      update(p) {
        const g = p.gpus[dev.index];
        if (!g) return;
        head.setRight(g.name || '');
        const hg = H.gpus.get(dev.index) || { usage: [], mem: [] };
        feed(c1, [hg.usage]);
        feed(c2, [hg.mem]);
        g2.setRight(g.memTotal ? bytes(g.memTotal, { digits: 1 }) : '');
        sUse.set(g.usage != null ? `${Math.round(g.usage)}%` : t('notAvailable'));
        sMem.set(g.memTotal ? `${bytes(g.memUsed, { digits: 1 })}/${bytes(g.memTotal, { digits: 1 })}` : t('notAvailable'));
        sTemp.set(g.temperature != null ? `${Math.round(g.temperature)} °C` : t('notAvailable'));
        kName.set(g.name || t('unknown'));
        kVendor.set(g.vendor ? g.vendor.toUpperCase() === 'NVIDIA' || g.vendor === 'nvidia' ? 'NVIDIA' : g.vendor : t('unknown'));
        const e = g.engines || null;
        engines.hidden = !e;
        if (e) { s3d.set(e['3d'] != null ? `${Math.round(e['3d'])}%` : '–'); sEnc.set(e.encode != null ? `${Math.round(e.encode)}%` : '–'); sDec.set(e.decode != null ? `${Math.round(e.decode)}%` : '–'); }
        kDriver.forEach((x) => { x.hidden = !g.driver; }); if (g.driver) kDriver.set(String(g.driver));
        kPower.forEach((x) => { x.hidden = g.power == null; }); if (g.power != null) kPower.set(`${number(g.power, 0)} W`);
        kClock.forEach((x) => { x.hidden = !g.clockMHz; }); if (g.clockMHz) kClock.set(`${number(g.clockMHz)} MHz`);
      },
    };
  }

  /* ------------------------------------------------------------ commands */
  const newBtn = Button({ label: t('newTask'), icon: 'new', variant: 'subtle', onClick: () => ctx.newTask() });
  const copyBtn = Button({ label: t('copy'), icon: 'copy', variant: 'subtle', onClick: () => copySummary() });
  const viewBtn = Button({ label: t('view'), icon: 'view', iconAfter: 'chevron-down', variant: 'subtle' });
  viewBtn.addEventListener('click', () => {
    const items = [];
    if (selected === 'cpu') {
      items.push({ header: t('graph') },
        { label: t('overall'), radio: true, checked: cpuMode === 'overall', action: () => setCpuMode('overall') },
        { label: t('perCore'), radio: true, checked: cpuMode === 'perCore', action: () => setCpuMode('perCore') },
        { separator: true });
    }
    items.push({
      label: t('duration'), icon: 'clock', submenu: DURATIONS.map((s) => ({
        label: durLabel(s), radio: true, checked: graphSeconds() === s,
        action: () => setSetting('taskmgr.graphSeconds', s).catch(() => {}),
      })),
    });
    showContextMenu({ anchor: viewBtn, align: 'end', items });
  });

  function setCpuMode(m) { cpuMode = m; save('perf.cpuMode', m); buildDetail(); }

  async function copySummary() {
    const lines = [...panel.querySelectorAll('.tm-detail__title, .tm-detail__model, .tm-stat, .tm-kv dt')].map((n) => {
      if (n.matches('dt')) return `${n.textContent}: ${n.nextElementSibling?.textContent || ''}`;
      if (n.matches('.tm-stat')) return `${n.firstChild.textContent}: ${n.lastChild.textContent}`;
      return n.textContent;
    }).filter(Boolean);
    try { await navigator.clipboard.writeText(lines.join('\n')); Toast.show({ title: t('copied'), severity: 'success', timeout: 2500 }); } catch { /* ignore */ }
  }

  // graph colours / durations follow the settings
  offs.push(onSettings((s, path) => {
    if (path && !String(path).startsWith('taskmgr')) return;
    if (!perf) return;
    list.dataset.ids = '';
    buildList(perf);
    updateList(perf);
    buildDetail();
  }));

  function onData(p) {
    const first = !perf;
    perf = p;
    buildList(p);
    updateList(p);
    if (first || !detail) buildDetail();
    else detail.update(p);
  }

  let mounted = [];
  return {
    id: 'performance',
    title: t('title'),
    el,
    commands: [newBtn, h('span', { class: 'tm-cmd-sep' }), copyBtn, viewBtn],
    mount(params = {}) {
      if (params.device) { selected = params.device; save('perf.device', selected); }
      mounted.push(store.subscribe('performance', onData));
      mounted.push(store.subscribe('volumes', onVolumes, () => { volsFailed = true; if (perf) updateList(perf); }));
      if (perf && params.device) select(params.device);
    },
    navigate(params = {}) { if (params.device) select(params.device); },
    unmount() { while (mounted.length) mounted.pop()(); },
    refresh() { return store.refresh('performance'); },
  };
}
