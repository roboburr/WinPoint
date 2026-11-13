// Taakbeheer › Opslag — disk usage explorer: storage.scan with live progress (cancel),
// sunburst with hover/drill-down, breadcrumb, and a side list of the biggest items.

import { h, clear } from '../../../core/dom.js';
import { Button, IconButton, ProgressRing, Breadcrumb, EmptyState, InfoBar, AppIcon, Toast } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { bindContextMenu } from '../../../core/contextmenu.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, number } from '../../../core/format.js';
import { Sunburst, filesText } from './sunburst.js';
import { prettyPath, dirName } from '../util.js';

const t = makeT({
  nl: {
    scanning: 'Bezig met scannen…', scanned: '{size} · {files}', cancel: 'Annuleren', stopped: 'Scannen gestopt',
    stoppedBody: 'Je hebt het scannen gestopt. Start opnieuw wanneer je wilt.', again: 'Opnieuw scannen', scan: 'Scannen',
    pick: 'Map kiezen…', openExplorer: 'Openen in Verkenner', rescan: 'Opnieuw scannen', failed: 'Scannen is mislukt',
    contents: 'Grootste items in {name}', empty: 'Deze map is leeg.', other: { one: 'Overige ({n} kleiner item)', other: 'Overige ({n} kleinere items)' }, otherShort: 'Overige',
    openLocation: 'Bestandslocatie openen', largeHere: 'Grote bestanden in deze map', copyPath: 'Pad kopiëren', open: 'Openen',
    loadingFolder: 'Map wordt gescand…', hint: 'Wijs een stuk aan voor details. Klik om een map te openen; klik in het midden om terug te gaan.',
    pickTitle: 'Kies een map om te scannen', noPicker: 'Kiezen is hier niet beschikbaar', copied: 'Pad gekopieerd',
    up: 'Omhoog', hostDown: 'Win11 Shell reageert niet. Scan opnieuw zodra het weer werkt.',
  },
  en: {
    scanning: 'Scanning…', scanned: '{size} · {files}', cancel: 'Cancel', stopped: 'Scan stopped',
    stoppedBody: 'You stopped the scan. Start again whenever you like.', again: 'Scan again', scan: 'Scan',
    pick: 'Choose folder…', openExplorer: 'Open in File Explorer', rescan: 'Scan again', failed: 'The scan failed',
    contents: 'Largest items in {name}', empty: 'This folder is empty.', other: { one: 'Other ({n} smaller item)', other: 'Other ({n} smaller items)' }, otherShort: 'Other',
    openLocation: 'Open file location', largeHere: 'Large files in this folder', copyPath: 'Copy path', open: 'Open',
    loadingFolder: 'Scanning folder…', hint: 'Point at a segment for details. Click a folder to open it; click the centre to go back.',
    pickTitle: 'Choose a folder to scan', noPicker: 'Choosing is not available here', copied: 'Path copied',
    up: 'Up', hostDown: 'Win11 Shell is not responding. Scan again once it works again.',
  },
});

/* ------------------------------------------------------------- scanner */

/** One scan at a time; buffers scanDone events that arrive before the jobId is known. */
export function createScanner(bridge) {
  const trees = new Map();
  const early = new Map();
  let active = null;
  const listeners = new Set();

  bridge.on('storage.scanProgress', (d) => { if (active && d?.jobId === active.jobId) active.onProgress?.(d); });
  bridge.on('storage.scanDone', (d) => {
    if (!d) return;
    if (active && d.jobId === active.jobId) finish(active, d);
    else { early.set(d.jobId, d); setTimeout(() => early.delete(d.jobId), 30000); }
  });
  // the host stopped answering (or restarts): its scan job and the scanDone event will never come
  bridge.onHealth?.((st) => {
    if (st === 'ok' || !active) return;
    const job = active;
    active = null;
    const e = new Error(t('hostDown'));
    e.code = 'timeout';
    job.reject(e);
  });

  function finish(job, d) {
    if (active === job) active = null;
    if (d.error || d.errorCode) {
      const code = d.errorCode || (d.error === 'cancelled' ? 'cancelled' : 'scan_failed');
      const e = new Error(code === 'cancelled' ? 'cancelled' : (d.error || d.message || ''));
      e.code = code;
      job.reject(e);
      return;
    }
    if (!d.tree && !d.fetched) {
      // the event may come without the (large) tree: fetch it with storage.scanResult {jobId}
      const jobId = d.jobId || job.jobId;
      active = job;
      bridge.call('storage.scanResult', { jobId })
        .then((r) => {
          if (active !== job) return;                      // cancelled or replaced meanwhile
          if (r?.state === 'running') { setTimeout(() => { if (active === job) finish(job, { jobId }); }, 400); return; }
          if (r?.state === 'error' || !r?.tree) {
            finish(job, { jobId, fetched: true, error: r?.error || '', errorCode: r?.errorCode || 'failed' });
            return;
          }
          finish(job, { jobId, fetched: true, tree: r.tree });
        })
        .catch((e) => { if (active === job) finish(job, { jobId, fetched: true, error: e.message, errorCode: e.code || 'failed' }); });
      return;
    }
    if (d.tree) { trees.set(job.path, d.tree); for (const cb of listeners) cb(job.path, d.tree); }
    job.resolve(d.tree);
  }

  return {
    trees,
    onTree(cb) { listeners.add(cb); return () => listeners.delete(cb); },
    get busy() { return !!active; },
    async scan(path, { refresh = false, onProgress, maxDepth = 4, topN = 40 } = {}) {
      if (active) this.cancel();
      const job = { path, onProgress, jobId: null };
      const p = new Promise((resolve, reject) => { job.resolve = resolve; job.reject = reject; });
      active = job;
      try {
        const r = await bridge.call('storage.scan', { path, maxDepth, topN, refresh });
        job.jobId = r?.jobId;
        if (r?.tree) { finish(job, { jobId: job.jobId, tree: r.tree }); return p; }
        if (early.has(job.jobId)) { const d = early.get(job.jobId); early.delete(job.jobId); finish(job, d); }
      } catch (e) {
        if (active === job) active = null;
        job.reject(e);
      }
      return p;
    },
    cancel() {
      const job = active;
      if (!job) return;
      active = null;
      if (job.jobId) bridge.call('storage.scanCancel', { jobId: job.jobId }).catch(() => {});
      const e = new Error('cancelled'); e.code = 'cancelled';
      job.reject(e);
    },
  };
}

/* -------------------------------------------------------------- explorer */

export function createExplorer(ctx, { scanner, onLargeFiles }) {
  const { bridge } = ctx;
  let scope = null;       // {path, label}
  let rootTree = null;
  let trail = [];         // nodes root → current
  let seq = 0;

  const crumbs = Breadcrumb({ items: [], size: 'body', onSelect: (id, it, i) => goTo(i) });
  const upBtn = IconButton({ icon: 'arrow-up', label: t('up'), onClick: () => goTo(trail.length - 2) });
  const pickBtn = Button({ label: t('pick'), icon: 'folder-open', variant: 'subtle', onClick: () => pick() });
  const rescanBtn = Button({ label: t('rescan'), icon: 'refresh', variant: 'subtle', onClick: () => api.refresh() });
  const openBtn = Button({ label: t('openExplorer'), icon: 'external', variant: 'subtle', onClick: () => openInExplorer(current()?.path) });
  const bar = h('div', { class: 'tm-explore__bar' }, upBtn, h('div', { class: 'tm-explore__crumbs' }, crumbs),
    h('div', { class: 'tm-explore__actions' }, pickBtn, rescanBtn, openBtn));

  const sunburst = Sunburst({
    rings: 3,
    onDrill: (node) => drill(node),
    onUp: () => goTo(trail.length - 2),
    onHover: (node) => markListHover(node),
    onPick: (node) => selectFile(node),
  });
  const chartCard = h('div', { class: 'tm-card tm-explore__chart' }, sunburst, h('p', { class: 'tm-explore__hint caption' }, t('hint')));
  const listTitle = h('div', { class: 'tm-explore__listtitle' });
  const list = h('div', { class: 'tm-explore__list', role: 'list' });
  const listCard = h('div', { class: 'tm-card tm-explore__side' }, listTitle, list);
  const main = h('div', { class: 'tm-explore__main' }, chartCard, listCard);
  const status = h('div', { class: 'tm-explore__status' });
  const el = h('div', { class: 'tm-explore' }, bar, status, main);

  const current = () => trail[trail.length - 1] || null;

  /* ------------------------------------------------------------ progress */
  function showProgress(path) {
    main.hidden = true;
    if (!trail.length) { crumbs.setItems([{ id: '0', label: scope?.label || prettyPath(path) }]); upBtn.setDisabled(true); }
    const ring = ProgressRing({ size: 56 });
    const amount = h('div', { class: 'tm-scan__amount' }, ' ');
    const cur = h('div', { class: 'tm-scan__path' }, prettyPath(path));
    const cancel = Button({ label: t('cancel'), onClick: () => scanner.cancel() });
    clear(status).append(h('div', { class: 'tm-card tm-scan' }, ring,
      h('div', { class: 'tm-scan__title subtitle' }, t('scanning')), amount, cur, cancel));
    status.hidden = false;
    return (d) => {
      amount.textContent = t('scanned', { size: bytes(d.bytes || 0), files: filesText(d.files || 0) });
      if (d.current) cur.textContent = prettyPath(d.current);
    };
  }

  function showMessage(node) {
    main.hidden = true;
    clear(status).append(node);
    status.hidden = false;
  }

  async function runScan(path, { refresh = false } = {}) {
    const my = ++seq;
    const cached = !refresh && scanner.trees.get(path);
    if (cached) { setTree(cached); return; }
    const onProgress = showProgress(path);
    try {
      const tree = await scanner.scan(path, { refresh, onProgress });
      if (my !== seq) return;
      setTree(tree);
    } catch (e) {
      if (my !== seq) return;
      if (e.code === 'cancelled') {
        showMessage(h('div', { class: 'tm-card tm-scan' }, EmptyState({ icon: 'info', title: t('stopped'), message: t('stoppedBody'), action: { label: t('again'), onClick: () => runScan(path, { refresh: true }) } })));
      } else {
        showMessage(InfoBar({ severity: 'error', title: t('failed'), message: e.message || '', closable: false, action: { label: t('again'), onClick: () => runScan(path, { refresh: true }) } }));
      }
    }
  }

  function setTree(tree) {
    rootTree = tree;
    trail = [tree];
    status.hidden = true;
    clear(status);
    main.hidden = false;
    render(null);
  }

  /* --------------------------------------------------------------- render */
  function render(direction) {
    const node = current();
    if (!node) return;
    const items = trail.map((n, i) => ({ id: String(i), label: i === 0 ? (scope?.label || n.name) : n.name }));
    crumbs.setItems(items);
    upBtn.setDisabled(trail.length < 2);
    sunburst.setRoot(node, { direction, up: trail.length > 1, label: trail.length === 1 ? scope?.label : '' });
    listTitle.textContent = t('contents', { name: trail.length === 1 ? (scope?.label || node.name) : node.name });
    renderList(node);
  }

  function renderList(node) {
    clear(list);
    const kids = (node.children || []).filter((c) => c.size > 0);
    if (!kids.length) { list.append(h('div', { class: 'tm-list__empty' }, t('empty'))); return; }
    const max = Math.max(...kids.map((k) => k.size));
    for (const c of kids) {
      const pct = node.size ? (c.size / node.size) * 100 : 0;
      const color = sunburst.colorOf(c);
      const ic = c.other ? h('span', { class: 'tm-glyph' }, icon('more', { size: 16 }))
        : c.isDir ? AppIcon({ name: 'folder', size: 20, fallback: 'folder' })
          : AppIcon({ mime: guessMime(c.name), size: 20, fallback: 'file' });
      const row = h('div', {
        class: ['tm-xrow', c.isDir && !c.other && 'is-dir', c.other && 'is-other'], role: 'listitem',
        tabindex: c.other ? -1 : 0, dataset: { tooltip: c.other ? '' : prettyPath(c.path) },
      },
      h('span', { class: 'tm-xrow__swatch', style: { background: color } }),
      h('span', { class: 'tm-xrow__icon' }, ic),
      h('span', { class: 'tm-xrow__main' },
        h('span', { class: 'tm-xrow__top' },
          h('span', { class: 'tm-xrow__name' }, c.other ? (c.count ? t('other', { n: number(c.count), count: c.count }) : t('otherShort')) : c.name),
          h('span', { class: 'tm-xrow__size' }, bytes(c.size))),
        h('span', { class: 'tm-xrow__barline' },
          h('span', { class: 'tm-xrow__bar' }, h('span', { style: { width: `${Math.max(1.5, (c.size / max) * 100)}%`, background: color } })),
          h('span', { class: 'tm-xrow__pct' }, `${number(pct, pct < 10 ? 1 : 0)}%`))),
      c.isDir && !c.other ? h('span', { class: 'tm-xrow__chev' }, icon('chevron-right', { size: 12 })) : null);
      row._node = c;
      row.addEventListener('pointerenter', () => sunburst.highlight(c));
      row.addEventListener('pointerleave', () => sunburst.highlight(null));
      row.addEventListener('focus', () => sunburst.highlight(c));
      row.addEventListener('blur', () => sunburst.highlight(null));
      row.addEventListener('click', () => { if (c.other) return; if (c.isDir) drill(c); else selectFile(c); });
      row.addEventListener('keydown', (e) => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); row.click(); }
        else if (e.key === 'ArrowDown') { e.preventDefault(); row.nextElementSibling?.focus(); }
        else if (e.key === 'ArrowUp') { e.preventDefault(); row.previousElementSibling?.focus(); }
        else if (e.key === 'Backspace' || (e.altKey && e.key === 'ArrowUp')) { e.preventDefault(); goTo(trail.length - 2); }
      });
      list.append(row);
    }
  }

  function markListHover(node) {
    for (const row of list.children) row.classList.toggle('is-hot', !!node && row._node === node);
  }

  bindContextMenu(list, (e) => {
    const row = (e.type === 'keydown' ? document.activeElement : e.target).closest?.('.tm-xrow');
    const n = row?._node;
    if (!n || n.other) return null;
    return {
      items: [
        n.isDir ? { label: t('open'), icon: 'open', action: () => drill(n) } : null,
        n.isDir ? { label: t('openExplorer'), icon: 'external', action: () => openInExplorer(n.path) } : { label: t('openLocation'), icon: 'folder-open', action: () => openInExplorer(dirName(n.path)) },
        n.isDir ? { label: t('largeHere'), icon: 'filter', action: () => onLargeFiles?.(n.path, n.name) } : null,
        { separator: true },
        { label: t('copyPath'), icon: 'copy', action: () => copyPath(n.path) },
      ].filter(Boolean),
    };
  });

  /** Nodes from (excluding) `from` down to `target`, or null. */
  function chain(from, target) {
    for (const c of from.children || []) {
      if (c === target) return [c];
      const sub = chain(c, target);
      if (sub) return [c, ...sub];
    }
    return null;
  }

  async function drill(node) {
    if (!node?.isDir || node.other) return;
    const path = chain(current(), node) || [node];
    const hasKids = node.children && node.children.length;
    if ((!hasKids || node.truncated) && (node.files || node.size || node.truncated)) {
      // deeper than the first scan: scan this folder and graft the result
      const my = ++seq;
      listTitle.textContent = t('loadingFolder');
      clear(list).append(h('div', { class: 'tm-list__empty' }, ProgressRing({ size: 28 })));
      try {
        const sub = scanner.trees.get(node.path) || await scanner.scan(node.path, { onProgress: () => {} });
        if (my !== seq) return;
        node.children = sub?.children || [];
        node.truncated = false;
        if (sub?.files != null) node.files = sub.files;
      } catch (e) {
        if (my !== seq) return;
        if (e.code !== 'cancelled') Toast.show({ title: t('failed'), message: e.message, severity: 'error' });
        render(null);
        return;
      }
    }
    trail.push(...path);
    render('in');
  }

  function goTo(i) {
    if (i < 0 || i >= trail.length - 1) return;
    trail = trail.slice(0, i + 1);
    render('out');
  }

  function selectFile(node) {
    for (const row of list.children) row.classList.toggle('is-picked', row._node === node);
    const row = [...list.children].find((r) => r._node === node);
    row?.focus();
  }

  function openInExplorer(path) {
    if (!path) return;
    bridge.call('shell.openApp', { app: 'explorer', params: { path } })
      .catch((e) => Toast.show({ title: t('openExplorer'), message: e.message, severity: 'error' }));
  }

  async function copyPath(p) {
    try { await navigator.clipboard.writeText(p); Toast.show({ title: t('copied'), severity: 'success', timeout: 2000 }); } catch { /* ignore */ }
  }

  async function pick() {
    try {
      const r = await bridge.call('shell.pickFile', { title: t('pickTitle'), folder: true });
      if (r?.path) api.setScope({ path: r.path, label: prettyPath(r.path) });
      else if (bridge.isMock) Toast.show({ title: t('noPicker'), severity: 'info', timeout: 2500 });
    } catch (e) {
      Toast.show({ title: t('noPicker'), message: e.message, severity: 'warning' });
    }
  }

  document.addEventListener('keydown', (e) => {
    if (!el.isConnected || !el.offsetParent) return;
    if (e.altKey && e.key === 'ArrowUp') { e.preventDefault(); goTo(trail.length - 2); }
  });

  const api = {
    el,
    get scope() { return scope; },
    get path() { return current()?.path || scope?.path; },
    setScope(s, { refresh = false } = {}) {
      const same = scope && scope.path === s.path;
      scope = s;
      if (same && rootTree && !refresh) { render(null); return; }
      rootTree = null; trail = [];
      runScan(s.path, { refresh });
    },
    refresh() { if (scope) runScan(scope.path, { refresh: true }); },
    cancel() { scanner.cancel(); },
  };
  return api;
}

const EXT = {
  iso: 'application/x-cd-image', mkv: 'video/x-matroska', mp4: 'video/mp4', mov: 'video/quicktime', jpg: 'image/jpeg', jpeg: 'image/jpeg',
  png: 'image/png', cr3: 'image/x-canon-cr3', zip: 'application/zip', pdf: 'application/pdf', deb: 'application/vnd.debian.binary-package',
  flac: 'audio/flac', mp3: 'audio/mpeg', qcow2: 'application/x-qemu-disk', txt: 'text/plain', odt: 'application/vnd.oasis.opendocument.text',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', js: 'text/javascript', csv: 'text/csv',
};
export function guessMime(name) {
  const ext = String(name).toLowerCase().match(/\.([a-z0-9]+)$/)?.[1];
  return EXT[ext] || 'application/octet-stream';
}
