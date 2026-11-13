// Win11 Shell — desktop surface: wallpaper + desktop icons (desktop agent). SPEC §11.
//
//   export async function mountDesktop(root, {preview})
//
// Windows behaviour: column-major icon grid from the top left (small/medium/large), labels with
// a text shadow (2 lines, full name when selected), translucent hover/selection fills,
// rubber-band selection, Ctrl/Shift multi-select, drag to move (grid snapping, positions
// persisted in settings.desktop.positions keyed by file name), drop on a folder = move into it,
// drop on Prullenbak = trash. Double-click/Enter opens, F2 renames inline, Delete trashes with an
// "Ongedaan maken" toast, Ctrl+C/X/V/A/Z, arrows, type-to-select, Ctrl+Shift+2/3/4 and
// Ctrl+wheel change the icon size. Context menus in Win11 wording.
//
// Debug params (screenshots): wallpaper=<id> wpfit=<fit> wpcolor=%23hex iconsize=small|medium|large
//   select=<name>[,<name>] open=contextmenu-desktop|contextmenu-item|contextmenu-trash|properties|rename|toast|emptytrash

import { h, clear, loadCss, animate, debounce, clamp, uid } from '../core/dom.js';
import { bridge } from '../core/bridge.js';
import { getSettings, onSettings, setSetting } from '../core/theme.js';
import { showContextMenu } from '../core/contextmenu.js';
import { Dialog } from '../core/components.js';
import { icon } from '../core/icons.js';
import { bytes } from '../core/format.js';
import { onLangChange, collator } from '../core/i18n.js';
import { t } from './strings.js';
import { createWallpaper } from './wallpaper.js';
import { showToast } from './toast.js';
import { showProperties } from './properties.js';
import { backgroundMenu, itemMenu } from './menus.js';

const SIZES = {
  small: { cw: 76, ch: 74, icon: 32 },
  medium: { cw: 76, ch: 100, icon: 48 },
  large: { cw: 112, ch: 148, icon: 96 },
};
const SIZE_ORDER = ['small', 'medium', 'large'];
const ORIGIN = { x: 3, y: 3 };
const SPECIAL_ORDER = ['thispc', 'home', 'trash'];
const NAME_ORDER = { numeric: true, sensitivity: 'base' };

const extOf = (name) => { const m = /\.([^./]+)$/.exec(name); return m && !name.startsWith('.') ? m[1].toLowerCase() : ''; };
const stemOf = (name) => { const i = name.lastIndexOf('.'); return i > 0 ? name.slice(0, i) : name; };
const baseOf = (p) => p.slice(p.lastIndexOf('/') + 1);
const parentOf = (p) => (p.lastIndexOf('/') > 0 ? p.slice(0, p.lastIndexOf('/')) : '/');
const cellId = (c, r) => `${c},${r}`;

export async function mountDesktop(root, { preview = false } = {}) {
  await loadCss('desktop');
  const P = bridge.params;

  /* ------------------------------------------------------------------ DOM */
  const wall = createWallpaper();
  const grid = h('div', { class: 'dt-grid' });
  const band = h('div', { class: 'dt-band', hidden: true });
  const ghosts = h('div', { class: 'dt-ghosts', 'aria-hidden': 'true' });
  const live = h('div', { class: 'sr-only', 'aria-live': 'polite' });
  const desk = h('div', {
    class: 'dt', tabindex: 0, role: 'listbox', 'aria-multiselectable': 'true', 'aria-label': t('desktopLabel'),
    dataset: { preview: preview ? 'true' : 'false' },
  }, wall.el, grid, band, ghosts);
  root.append(desk, live);

  /* ---------------------------------------------------------------- state */
  let dir = '';
  let homePath = '';
  let files = [];
  let trashInfo = { count: 0, size: 0 };
  let clip = { paths: [], op: 'copy' };
  let model = [];                 // ordered desktop items
  const byKey = new Map();
  const placed = new Map();       // key → {x, y} (grid units)
  const els = new Map();          // key → tile element
  let selected = new Set();
  let anchorKey = null;
  let focusKey = null;
  let undoStack = [];
  let metrics = null;
  let renaming = null;            // {key, ta, done}
  let loaded = false;

  const dset = () => ({ showIcons: true, iconSize: 'medium', autoArrange: false, alignToGrid: true, sortBy: 'name', positions: {}, showThisPC: true, showTrash: true, showHome: false, ...(getSettings().desktop || {}) });
  const iconSize = () => (SIZES[P.get('iconsize')] ? P.get('iconsize') : (SIZES[dset().iconSize] ? dset().iconSize : 'medium'));

  /* ------------------------------------------------------------- data load */
  async function loadFiles() {
    try {
      const r = await bridge.call('files.desktop');
      dir = r?.dir || dir;
      files = Array.isArray(r?.items) ? r.items.filter((f) => !f.hidden && !f.name.startsWith('.')) : [];
    } catch (e) {
      console.warn('[desktop] files.desktop failed', e);
      files = [];
    }
  }
  async function loadTrash() {
    try { trashInfo = (await bridge.call('files.trashInfo')) || { count: 0, size: 0 }; } catch { trashInfo = { count: 0, size: 0 }; }
  }
  async function loadClip() {
    try { clip = (await bridge.call('files.clipboardGet')) || { paths: [], op: 'copy' }; } catch { clip = { paths: [], op: 'copy' }; }
  }
  async function loadHome() {
    try {
      const places = await bridge.call('files.places');
      homePath = places.find((p) => p.id === 'home')?.path || '';
    } catch { /* ignore */ }
    if (!homePath && dir) homePath = parentOf(dir);
  }

  /* ----------------------------------------------------------------- model */
  function labelFor(f) {
    if (f.shellShortcut === 'thispc' && !f.untrusted) return t('thispc');
    if (f.isDesktopEntry) return f.displayName || stemOf(f.name);
    const showExt = getSettings().explorer?.showExtensions !== false;
    return !showExt && !f.isDir ? stemOf(f.displayName || f.name) : (f.displayName || f.name);
  }

  function sortFiles(list) {
    const by = dset().sortBy || 'name';
    const coll = collator(NAME_ORDER);
    const name = (a, b) => coll.compare(a.label, b.label);
    const cmp = {
      name,
      size: (a, b) => (a.file.size || 0) - (b.file.size || 0) || name(a, b),
      type: (a, b) => coll.compare(a.isDir ? '' : extOf(a.file.name) || a.file.mime || '', b.isDir ? '' : extOf(b.file.name) || b.file.mime || '') || name(a, b),
      modified: (a, b) => (b.file.modified || 0) - (a.file.modified || 0) || name(a, b),
    }[by] || name;
    return list.sort((a, b) => (b.isDir - a.isDir) || cmp(a, b));
  }

  function buildModel() {
    const d = dset();
    const specials = [];
    if (d.showThisPC !== false) specials.push({ key: '::thispc', special: 'thispc', label: t('thispc'), icon: 'computer', isDir: true });
    if (d.showHome) specials.push({ key: '::home', special: 'home', label: t('home'), icon: 'user-home', isDir: true, path: homePath });
    if (d.showTrash !== false) specials.push({ key: '::trash', special: 'trash', label: t('trash'), icon: trashInfo.count ? 'user-trash-full' : 'user-trash', isDir: true });
    specials.sort((a, b) => SPECIAL_ORDER.indexOf(a.special) - SPECIAL_ORDER.indexOf(b.special));
    // Keep the explicit legacy "Deze PC.desktop" file on disk. The built-in
    // special item already opens the same native Explorer computer location.
    const items = files.filter((f) => !(d.showThisPC !== false && f.shellShortcut === 'thispc')).map((f) => ({
      key: f.name, file: f, path: f.path, label: labelFor(f), icon: f.icon, mime: f.mime, isDir: !!f.isDir,
      isDesktopEntry: !!f.isDesktopEntry,
    }));
    model = [...specials, ...sortFiles(items)];
    byKey.clear();
    for (const it of model) byKey.set(it.key, it);
    // drop selection of vanished items
    selected = new Set([...selected].filter((k) => byKey.has(k)));
    if (focusKey && !byKey.has(focusKey)) focusKey = null;
    if (anchorKey && !byKey.has(anchorKey)) anchorKey = null;
  }

  /* ---------------------------------------------------------------- layout */
  function computeMetrics() {
    const sz = SIZES[iconSize()];
    const ts = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--ts')) || 1;
    const cw = Math.round(sz.cw + (ts - 1) * 24);
    const ch = Math.round(sz.ch + (ts - 1) * 32);
    const bottom = getSettings().taskbar?.autoHide ? 0 : (parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--taskbar-height')) || 48);
    const W = desk.clientWidth || innerWidth;
    const H = (desk.clientHeight || innerHeight) - bottom;
    const cols = Math.max(1, Math.floor((W - ORIGIN.x * 2) / cw));
    const rows = Math.max(1, Math.floor((H - ORIGIN.y * 2) / ch));
    return { ...sz, cw, ch, cols, rows, tw: cw - 2, ts };
  }

  function nearestFree(c, r, occ, cols, rows) {
    const maxD = Math.max(cols, rows) + 1;
    for (let d = 0; d <= maxD; d++) {
      const ring = [];
      for (let dc = -d; dc <= d; dc++) {
        for (let dr = -d; dr <= d; dr++) {
          if (Math.max(Math.abs(dc), Math.abs(dr)) !== d) continue;
          const cc = c + dc, rr = r + dr;
          if (cc < 0 || rr < 0 || cc >= cols || rr >= rows) continue;
          ring.push([cc, rr, dc * dc * 1.2 + dr * dr]);
        }
      }
      ring.sort((a, b) => a[2] - b[2]);
      for (const [cc, rr] of ring) if (!occ.has(cellId(cc, rr))) return [cc, rr];
    }
    return firstFree(occ, cols, rows);
  }
  function firstFree(occ, cols, rows) {
    for (let c = 0; c < cols + 200; c++) for (let r = 0; r < rows; r++) if (!occ.has(cellId(c, r))) return [c, r];
    return [0, 0];
  }

  let pendingNew = {};
  const savePositionsSoon = debounce(() => {
    const add = pendingNew;
    pendingNew = {};
    if (!Object.keys(add).length) return;
    const pos = { ...(dset().positions || {}), ...add };
    setSetting('desktop.positions', pos).catch((e) => console.warn('[desktop] saving positions failed', e));
  }, 600);

  function layout() {
    metrics = computeMetrics();
    const { cols, rows } = metrics;
    const d = dset();
    placed.clear();
    const occ = new Map();
    const put = (key, x, y) => { placed.set(key, { x, y }); occ.set(cellId(Math.round(x), Math.round(y)), key); };
    if (d.autoArrange) {
      model.forEach((it, i) => put(it.key, Math.floor(i / rows), i % rows));
      return;
    }
    const pos = d.positions || {};
    const hasPos = Object.keys(pos).length > 0;
    const rest = [];
    for (const it of model) {
      const p = pos[it.key];
      if (!p || !Number.isFinite(p.x) || !Number.isFinite(p.y)) { rest.push(it); continue; }
      let { x, y } = p;
      if (d.alignToGrid !== false) { x = Math.round(x); y = Math.round(y); }
      if (x < -0.01 || y < -0.01 || x > cols - 0.99 || y > rows - 0.99) { rest.push(it); continue; }
      if (occ.has(cellId(Math.round(x), Math.round(y)))) { rest.push(it); continue; }
      put(it.key, x, y);
    }
    for (const it of rest) {
      const [c, r] = firstFree(occ, cols, rows);
      put(it.key, c, r);
      // once the user arranged icons, new items keep their spot (like Windows)
      if (hasPos && !pos[it.key]) pendingNew[it.key] = { x: c, y: r };
    }
    if (Object.keys(pendingNew).length) savePositionsSoon();
  }

  /* ---------------------------------------------------------------- render */
  function iconUrlFor(it, px) {
    if (it.file && it.mime?.startsWith('image/')) {
      const u = bridge.thumbUrl(it.path, px);
      if (u) return { url: u, thumb: true };
    }
    return { url: bridge.iconUrl(it.icon, px) || bridge.mimeIconUrl(it.mime || 'application/octet-stream', px), thumb: false };
  }

  function makeImage(it) {
    const size = metrics.icon;
    const px = Math.round(size * Math.min(3, Math.max(1, devicePixelRatio || 1)));
    const wrap = h('div', { class: 'dt-item__icon', style: { width: `${size}px`, height: `${size}px` } });
    const { url, thumb } = iconUrlFor(it, px);
    const img = h('img', { src: url, alt: '', draggable: false, decoding: 'async', class: thumb ? 'is-thumb' : '' });
    let step = 0;
    img.addEventListener('error', () => {
      step++;
      if (step === 1 && thumb) { img.classList.remove('is-thumb'); img.src = bridge.iconUrl(it.icon, px) || bridge.mimeIconUrl(it.mime, px); return; }
      if (step <= 2 && it.mime) { const u = bridge.mimeIconUrl(it.mime, px); if (u && u !== img.src) { img.src = u; return; } }
      img.replaceWith(icon(it.isDir ? 'folder' : 'file', { size: Math.round(size * 0.8) }));
    });
    wrap.append(img);
    if (it.isDesktopEntry) wrap.append(shortcutBadge(size));
    return wrap;
  }

  function shortcutBadge(size) {
    const s = Math.max(12, Math.round(size / 3));
    const el = h('span', { class: 'dt-item__shortcut', style: { width: `${s}px`, height: `${s}px` } });
    el.innerHTML = '<svg viewBox="0 0 12 12" width="100%" height="100%" aria-hidden="true"><rect x=".5" y=".5" width="11" height="11" rx="2" fill="#fff" stroke="#9aa4ae" stroke-width=".8"/>'
      + '<path d="M3.4 8.8c.2-2.4 1.6-3.7 4.1-3.7" fill="none" stroke="#1f6fd0" stroke-width="1.3" stroke-linecap="round"/><path d="M6.2 3.6 8.2 5.1 6.4 6.9" fill="none" stroke="#1f6fd0" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round"/></svg>';
    return el;
  }

  function createTile(it) {
    const el = h('div', {
      class: 'dt-item', role: 'option', id: uid('dt-item'), 'aria-selected': 'false', dataset: { key: it.key },
    }, makeImage(it), h('div', { class: 'dt-item__label' }, it.label));
    el._sig = sig(it);
    return el;
  }
  const sig = (it) => `${it.label}|${it.icon}|${it.mime}|${it.path || ''}|${metrics.icon}|${it.file?.modified || ''}`;

  function updateTile(el, it) {
    const s = sig(it);
    if (el._sig === s) return;
    el._sig = s;
    el.querySelector('.dt-item__icon')?.replaceWith(makeImage(it));
    const lab = el.querySelector('.dt-item__label');
    if (lab) lab.textContent = it.label;
  }

  function isCut(it) {
    return clip.op === 'cut' && it.path && clip.paths.includes(it.path);
  }

  function render({ animateMoves = false } = {}) {
    const d = dset();
    desk.dataset.size = iconSize();
    grid.hidden = d.showIcons === false;
    desk.style.setProperty('--dt-cw', `${metrics.cw}px`);
    desk.style.setProperty('--dt-ch', `${metrics.ch}px`);
    desk.style.setProperty('--dt-tw', `${metrics.tw}px`);
    desk.style.setProperty('--dt-icon', `${metrics.icon}px`);
    grid.classList.toggle('is-animating', !!animateMoves);
    const seen = new Set();
    for (const it of model) {
      seen.add(it.key);
      let el = els.get(it.key);
      if (!el) { el = createTile(it); els.set(it.key, el); grid.append(el); } else updateTile(el, it);
      const p = placed.get(it.key) || { x: 0, y: 0 };
      el.style.left = `${Math.round(ORIGIN.x + p.x * metrics.cw + (metrics.cw - metrics.tw) / 2)}px`;
      el.style.top = `${Math.round(ORIGIN.y + p.y * metrics.ch)}px`;
      el.style.width = `${metrics.tw}px`;
      const sel = selected.has(it.key);
      el.classList.toggle('is-selected', sel);
      el.setAttribute('aria-selected', String(sel));
      el.setAttribute('aria-label', it.label);
      el.classList.toggle('is-focused', focusKey === it.key);
      el.classList.toggle('is-cut', isCut(it));
      el.style.zIndex = sel ? '2' : '';
    }
    for (const [k, el] of els) if (!seen.has(k)) { el.remove(); els.delete(k); }
    if (focusKey && els.get(focusKey)) desk.setAttribute('aria-activedescendant', els.get(focusKey).id);
    else desk.removeAttribute('aria-activedescendant');
    if (animateMoves) setTimeout(() => grid.classList.remove('is-animating'), 260);
  }

  function refreshView(opts) {
    buildModel();
    layout();
    render(opts);
  }

  async function reload({ flash = false } = {}) {
    await Promise.all([loadFiles(), loadTrash()]);
    refreshView();
    if (flash) animate(grid, [{ opacity: 0.25 }, { opacity: 1 }], { duration: 250, easing: 'linear' });
  }
  const reloadSoon = debounce(() => reload(), 120);

  /* ------------------------------------------------------------- selection */
  function setSelection(keys, { focus } = {}) {
    selected = new Set(keys);
    if (focus !== undefined) focusKey = focus;
    render();
  }
  const selectOnly = (key) => { anchorKey = key; setSelection(key ? [key] : [], { focus: key }); };
  const selectedItems = () => model.filter((it) => selected.has(it.key));

  /** Column-major order of the current layout (for Shift-ranges and type-to-select). */
  function visualOrder() {
    return [...model].sort((a, b) => {
      const pa = placed.get(a.key), pb = placed.get(b.key);
      return (Math.round(pa.x) - Math.round(pb.x)) || (pa.y - pb.y);
    });
  }

  function rangeSelect(toKey) {
    const order = visualOrder().map((i) => i.key);
    const a = order.indexOf(anchorKey ?? toKey), b = order.indexOf(toKey);
    if (a < 0 || b < 0) { selectOnly(toKey); return; }
    const [lo, hi] = a < b ? [a, b] : [b, a];
    setSelection(order.slice(lo, hi + 1), { focus: toKey });
  }

  /* ------------------------------------------------------------- actions */
  function errorAlert(e, fallback) {
    console.warn('[desktop]', e);
    Dialog.alert({ title: t('errorTitle'), message: e?.message || fallback || t('genericError') });
  }

  /** --dry-run (test mode): the host changed nothing — tell the user instead of pretending. */
  function dryRun(r, msgKey = 'dryRunChange') {
    if (!r?.dryRun) return false;
    showToast({ icon: 'info', title: t('dryRunTitle'), message: t(msgKey), timeout: 5000 });
    return true;
  }

  function busyCursor() {
    desk.classList.add('is-busy');
    clearTimeout(busyCursor.timer);
    busyCursor.timer = setTimeout(() => desk.classList.remove('is-busy'), 1400);
  }

  async function openItems(items) {
    for (const it of items) {
      try {
        if (it.special === 'thispc') await bridge.call('shell.openApp', { app: 'explorer', params: { place: 'thispc' } });
        else if (it.special === 'trash') await bridge.call('shell.openApp', { app: 'explorer', params: { place: 'trash' } });
        else if (it.special === 'home') await bridge.call('shell.openApp', { app: 'explorer', params: homePath ? { path: homePath } : { place: 'home' } });
        else if (it.isDir) await bridge.call('shell.openApp', { app: 'explorer', params: { path: it.path } });
        else { busyCursor(); await bridge.call('files.open', { path: it.path }); }
      } catch (e) { errorAlert(e, t('openError')); }
    }
  }

  async function openWith(it, appId) {
    try { busyCursor(); await bridge.call('files.openWith', { path: it.path, appId }); } catch (e) { errorAlert(e, t('openError')); }
  }

  function pushUndo(entry) {
    undoStack.push(entry);
    if (undoStack.length > 20) undoStack.shift();
  }
  async function runUndo() {
    const u = undoStack.pop();
    if (!u) return;
    u.toast?.close();
    try { await u.run(); } catch (e) { errorAlert(e); }
    await reload();
  }

  async function trashItems(items) {
    const list = items.filter((i) => !i.special && i.path);
    if (!list.length) return;
    const paths = list.map((i) => i.path);
    // keep keyboard focus near where it was
    const order = visualOrder().map((i) => i.key);
    const idx = Math.max(0, Math.min(...list.map((i) => order.indexOf(i.key))));
    const oldPos = {};   // so "Ongedaan maken" puts the icons back on their old spot
    for (const i of list) { const p = placed.get(i.key); if (p) oldPos[i.key] = { x: p.x, y: p.y }; }
    let r;
    try { r = await bridge.call('files.trash', { paths }); } catch (e) { errorAlert(e, t('trashError')); return; }
    if (dryRun(r, 'dryRunMsg')) return;
    const failed = r?.failed || [];
    const failedPaths = new Set(failed.map((f) => f.path ?? f));
    const okItems = list.filter((i) => !failedPaths.has(i.path));
    // names inside the Prullenbak (what files.restoreTrash wants): from files.trash().trashed, else looked up
    // in the trash listing at undo time (newest entry per original path), else the plain file names
    const trashedNames = Array.isArray(r?.trashed) && r.trashed.length ? r.trashed.map((x) => x.name).filter(Boolean) : null;
    const okPaths = okItems.map((i) => i.path);
    selected = new Set();
    await reload();
    const rest = visualOrder();
    const next = rest[Math.min(idx, rest.length - 1)];
    focusKey = next?.key || null;
    render();
    if (failed.length) errorAlert(null, `${t('trashError')} ${failed.map((f) => f.error || '').filter(Boolean)[0] || ''}`.trim());
    if (!okItems.length) return;
    const restore = async () => {
      let names = trashedNames;
      if (!names) {
        names = [];
        try {
          const tl = await bridge.call('files.list', { path: 'trash:///' });
          for (const p of okPaths) {
            const hit = (tl?.items || []).filter((x) => x.originalPath === p)
              .sort((a, b) => (b.deletedAt || 0) - (a.deletedAt || 0))[0];
            names.push(hit?.trashName || baseOf(p));
          }
        } catch { names = okPaths.map(baseOf); }
      }
      const rr = await bridge.call('files.restoreTrash', { names });
      const d = dset();
      if (!d.autoArrange && Object.keys(d.positions || {}).length) {
        const pos = { ...d.positions };
        for (const p of rr?.restored || []) { const k = baseOf(p); if (oldPos[k]) pos[k] = oldPos[k]; }
        try { await setSetting('desktop.positions', pos); } catch { /* ignore */ }
      }
      await reload();
      const back = (rr?.restored || []).map(baseOf).filter((k) => byKey.has(k));
      if (back.length) setSelection(back, { focus: back[0] });
    };
    const entry = { label: t('undoTrash'), run: restore };
    pushUndo(entry);
    live.textContent = t('trashed');
    entry.toast = showToast({
      icon: 'trash',
      title: t('trashed'),
      message: okItems.length === 1 ? t('trashedOne', { name: okItems[0].label }) : t('trashedMany', { n: okItems.length }),
      action: {
        label: t('undoAction'),
        onClick: async () => {
          const i = undoStack.indexOf(entry);
          if (i >= 0) undoStack.splice(i, 1);
          try { await restore(); } catch (e) { errorAlert(e); }
        },
      },
    });
  }

  async function emptyTrash() {
    await loadTrash();
    if (!trashInfo.count) return;
    const ok = await Dialog.confirm({
      title: t('emptyTitle'),
      message: t('emptyMsg', { n: trashInfo.count, size: bytes(trashInfo.size) }),
      confirmLabel: t('emptyConfirm'),
      cancelLabel: t('cancel'),
      danger: true,
      icon: 'warning',
    });
    if (!ok) return;
    try {
      const r = await bridge.call('files.emptyTrash');
      if (!dryRun(r)) showToast({ icon: 'trash', title: t('trashEmptied'), message: t('trashEmptiedMsg'), timeout: 4000 });
    } catch (e) { errorAlert(e); }
    await loadTrash();
    refreshView();
  }

  async function setClipboard(items, op) {
    const paths = items.filter((i) => !i.special && i.path).map((i) => i.path);
    if (!paths.length) return;
    try {
      await bridge.call('files.clipboardSet', { paths, op });
      clip = { paths, op };
      render();
    } catch (e) { errorAlert(e); }
  }

  function cellAt(point) {
    if (!point || !metrics) return null;
    const r = desk.getBoundingClientRect();
    const c = Math.floor((point.x - r.left - ORIGIN.x) / metrics.cw);
    const rr = Math.floor((point.y - r.top - ORIGIN.y) / metrics.ch);
    return [clamp(c, 0, metrics.cols - 1), clamp(rr, 0, metrics.rows - 1)];
  }

  /** Pin positions: all current spots + `extra` names at/near a cell (new or pasted items). */
  async function pinNewAt(names, point) {
    const d = dset();
    if (d.autoArrange || !names.length) return;
    const pos = {};
    const occMap = new Map();
    for (const [k, p] of placed) {
      pos[k] = { x: p.x, y: p.y };
      occMap.set(cellId(Math.round(p.x), Math.round(p.y)), k);
    }
    let start = cellAt(point);
    for (const n of names) {
      const [c, r] = start ? nearestFree(start[0], start[1], occMap, metrics.cols, metrics.rows) : firstFree(occMap, metrics.cols, metrics.rows);
      pos[n] = { x: c, y: r };
      occMap.set(cellId(c, r), 1);
      start = start || [c, r];
    }
    try { await setSetting('desktop.positions', pos); } catch (e) { console.warn('[desktop] saving positions failed', e); }
  }

  async function paste(point) {
    await loadClip();
    if (!clip.paths?.length) return;
    let r;
    try { r = await bridge.call('files.paste', { dest: dir }); } catch (e) { errorAlert(e, t('pasteError')); return; }
    if (dryRun(r)) return;
    const names = (r?.paths || []).filter((p) => parentOf(p) === dir).map(baseOf);
    await loadFiles();
    const fresh = names.filter((n) => !placed.has(n));
    if (fresh.length) await pinNewAt(fresh, point);
    await loadClip();
    await reload();
    const present = names.filter((n) => byKey.has(n));
    if (present.length) { anchorKey = present[0]; setSelection(present, { focus: present[0] }); }
  }

  async function createNew(kind, point) {
    let r;
    try {
      r = kind === 'folder'
        ? await bridge.call('files.createFolder', { dir })
        : await bridge.call('files.createFile', { dir, kind: 'text' });
    } catch (e) { errorAlert(e); return; }
    if (dryRun(r) || !r?.path) return;
    const name = baseOf(r.path);
    await loadFiles();
    await pinNewAt([name], point);
    await reload();
    pushUndo({ label: t('undoNew'), run: () => bridge.call('files.trash', { paths: [r.path] }) });
    selectOnly(name);
    startRename(name);
  }

  async function compress(items) {
    try {
      const r = await bridge.call('files.compress', { paths: items.map((i) => i.path) });
      if (dryRun(r)) return;
      await reload();
      const n = r?.path && baseOf(r.path);
      if (n && byKey.has(n)) selectOnly(n);
    } catch (e) { errorAlert(e); }
  }
  async function extract(it) {
    try {
      const r = await bridge.call('files.extract', { path: it.path });
      if (dryRun(r)) return;
      await reload();
      const n = r?.path && baseOf(r.path);
      if (n && byKey.has(n)) selectOnly(n);
    } catch (e) { errorAlert(e); }
  }

  async function copyText(text) {
    try { await navigator.clipboard.writeText(text); return true; } catch { /* fall back */ }
    const ta = h('textarea', { style: { position: 'fixed', left: '-9999px', top: '0' } }, text);
    document.body.append(ta);
    ta.select();
    let ok = false;
    try { ok = document.execCommand('copy'); } catch { ok = false; }
    ta.remove();
    desk.focus({ preventScroll: true });
    return ok;
  }
  const copyPath = (items) => copyText(items.map((i) => `"${i.path}"`).join('\n'));

  async function properties(items) {
    const list = items.filter((i) => i.path);
    if (!list.length) return;
    await showProperties(list.map((i) => ({ path: i.path, label: i.label, isDir: i.isDir, isDesktopEntry: i.isDesktopEntry, iconUrl: iconUrlFor({ ...i, mime: i.isDir ? 'inode/directory' : i.mime }, 64).url })), {
      onRenamed: (oldPath, newPath) => renamedKey(baseOf(oldPath), baseOf(newPath)),
    });
    await reload();
  }

  function openSettings(page) {
    bridge.call('shell.openApp', { app: 'settings', params: { page } }).catch((e) => errorAlert(e));
  }
  function openTerminal(d) {
    busyCursor();
    bridge.call('files.openTerminal', { dir: d || dir }).catch((e) => errorAlert(e));
  }

  async function setIconSize(size) {
    if (!SIZES[size]) return;
    try { await setSetting('desktop.iconSize', size); } catch (e) { errorAlert(e); }
  }
  async function toggle(name) {
    const d = dset();
    const cur = name === 'alignToGrid' || name === 'showIcons' ? d[name] !== false : !!d[name];
    try {
      if (name === 'alignToGrid' && !cur) {
        // snap everything to the grid right away (like Windows)
        const pos = {};
        for (const [k, p] of placed) pos[k] = { x: Math.round(p.x), y: Math.round(p.y) };
        await setSetting('desktop.positions', pos);
      }
      await setSetting(`desktop.${name}`, !cur);
    } catch (e) { errorAlert(e); }
  }
  async function sortBy(k) {
    try {
      await setSetting('desktop.sortBy', k);
      await setSetting('desktop.positions', {});
    } catch (e) { errorAlert(e); }
    refreshView({ animateMoves: true });
  }

  /** Keep position + selection when an item was renamed (key = file name). */
  function renamedKey(oldKey, newKey) {
    if (oldKey === newKey) return;
    const pos = { ...(dset().positions || {}) };
    if (pos[oldKey]) { pos[newKey] = pos[oldKey]; delete pos[oldKey]; setSetting('desktop.positions', pos).catch(() => {}); }
    const el = els.get(oldKey);
    if (el) { els.delete(oldKey); el.dataset.key = newKey; els.set(newKey, el); }
    if (selected.has(oldKey)) { selected.delete(oldKey); selected.add(newKey); }
    if (focusKey === oldKey) focusKey = newKey;
    if (anchorKey === oldKey) anchorKey = newKey;
  }

  /* ---------------------------------------------------------------- rename */
  function startRename(key) {
    const it = byKey.get(key);
    const el = els.get(key);
    if (!it || !el || it.special || it.isDesktopEntry || !it.path) return;
    finishRename(false);
    const name = it.file.name;
    const ta = h('textarea', {
      class: 'dt-rename', spellcheck: false, rows: 1, 'aria-label': t('rename'), autocomplete: 'off',
    });
    ta.value = name;
    el.classList.add('is-renaming');
    el.append(ta);
    const tip = h('div', { class: 'dt-rename-tip', role: 'alert', hidden: true });
    el.append(tip);
    const fit = () => { ta.style.height = 'auto'; ta.style.height = `${ta.scrollHeight}px`; };
    renaming = { key, ta, tip, done: false, old: name };
    requestAnimationFrame(() => {
      fit();
      ta.focus({ preventScroll: true });
      const dot = it.isDir ? -1 : name.lastIndexOf('.');
      ta.setSelectionRange(0, dot > 0 ? dot : name.length);
    });
    ta.addEventListener('input', () => { fit(); tip.hidden = true; });
    ta.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') { e.preventDefault(); commitRename(); }
      else if (e.key === 'Escape') { e.preventDefault(); finishRename(false); desk.focus({ preventScroll: true }); }
      else if (e.key === 'Tab') { e.preventDefault(); commitRename(); }
    });
    ta.addEventListener('pointerdown', (e) => e.stopPropagation());
    ta.addEventListener('dblclick', (e) => e.stopPropagation());
    ta.addEventListener('contextmenu', (e) => e.stopPropagation());
    ta.addEventListener('blur', () => { if (renaming?.ta === ta && !renaming.busy) commitRename({ fromBlur: true }); });
  }

  function finishRename(keepFocus = true) {
    if (!renaming) return;
    const { key, ta, tip } = renaming;
    renaming = null;
    ta.remove();
    tip.remove();
    els.get(key)?.classList.remove('is-renaming');
    if (keepFocus) desk.focus({ preventScroll: true });
  }

  function renameTip(msg) {
    if (!renaming) return;
    const tip = renaming.tip;
    tip.textContent = msg;
    tip.style.transform = '';
    tip.hidden = false;
    // keep the balloon on screen (icons in the first column sit right at the edge)
    const r = tip.getBoundingClientRect();
    const shift = r.left < 6 ? 6 - r.left : (r.right > innerWidth - 6 ? innerWidth - 6 - r.right : 0);
    if (shift) tip.style.transform = `translateX(calc(-50% + ${Math.round(shift)}px))`;
    renaming.ta.focus({ preventScroll: true });
  }

  async function commitRename({ fromBlur = false } = {}) {
    if (!renaming || renaming.busy) return;
    const r = renaming;
    const it = byKey.get(r.key);
    const value = r.ta.value.replace(/[\r\n]+/g, ' ').trim();
    if (!it || !value || value === r.old) { finishRename(!fromBlur); return; }
    if (value.includes('/')) {
      if (fromBlur) { finishRename(false); return; }
      renameTip(t('badName'));
      return;
    }
    r.busy = true;
    if (!it.isDir && extOf(value) !== extOf(r.old)) {
      const yes = await Dialog.confirm({ title: t('extTitle'), message: t('extMsg'), confirmLabel: t('extYes'), cancelLabel: t('extNo'), icon: 'warning' });
      if (!yes) { r.busy = false; finishRename(true); return; }
    }
    try {
      const res = await bridge.call('files.rename', { path: it.path, newName: value });
      if (res?.dryRun) { finishRename(!fromBlur); dryRun(res); return; }
      const newPath = res?.path || `${dir}/${value}`;
      const newKey = baseOf(newPath);
      finishRename(!fromBlur);
      renamedKey(r.old, newKey);
      pushUndo({ label: t('undoRename'), run: () => bridge.call('files.rename', { path: newPath, newName: r.old }).then(() => renamedKey(newKey, r.old)) });
      await reload();
      if (byKey.has(newKey)) { anchorKey = newKey; setSelection([newKey], { focus: newKey }); }
    } catch (e) {
      r.busy = false;
      if (fromBlur || !renaming) { finishRename(false); errorAlert(e, t('renameError')); return; }
      renameTip(e?.message || t('renameError'));
    }
  }

  /* ---------------------------------------------------------------- menus */
  const ctl = {
    dir: () => dir,
    desktopSettings: dset,
    trashInfo: () => trashInfo,
    canPaste: () => !!clip.paths?.length,
    undoInfo: () => (undoStack.length ? { label: undoStack[undoStack.length - 1].label } : null),
    runUndo,
    setIconSize, toggle, sortBy,
    refresh: (flash) => reload({ flash }),
    paste, createNew, openSettings, openTerminal,
    open: openItems, openWith,
    cut: (items) => setClipboard(items, 'cut'),
    copy: (items) => setClipboard(items, 'copy'),
    rename: (it) => it && startRename(it.key),
    trash: trashItems,
    compress, extract, copyPath, properties, emptyTrash,
  };

  const withTimeout = (p, ms, fallback) => Promise.race([p, new Promise((r) => setTimeout(() => r(fallback), ms))]);

  async function openMenuAt(x, y, { keyboard = false, onItem = null } = {}) {
    finishRename(false);
    if (onItem) {
      const items = selectedItems();
      if (!items.length) return;
      let apps = null;
      const one = items.length === 1 ? items[0] : null;
      if (one && !one.special && !one.isDir && !one.isDesktopEntry) {
        apps = await withTimeout(bridge.call('apps.forFile', { path: one.path }).catch(() => []), 350, []);
      }
      showContextMenu({ x, y, keyboard, ...itemMenu(ctl, items, apps) });
    } else {
      await withTimeout(loadClip(), 250, null);
      showContextMenu({ x, y, keyboard, ...backgroundMenu(ctl, { x, y }) });
    }
  }

  desk.addEventListener('contextmenu', (e) => {
    if (e.target.closest('.dt-rename')) return;
    e.preventDefault();
    const tile = e.target.closest('.dt-item');
    if (tile) {
      const key = tile.dataset.key;
      if (!selected.has(key)) selectOnly(key);
      else { focusKey = key; render(); }
      openMenuAt(e.clientX, e.clientY, { onItem: true });
    } else {
      if (!e.ctrlKey) setSelection([], { focus: focusKey });
      openMenuAt(e.clientX, e.clientY);
    }
  });

  /* -------------------------------------------------------------- pointer */
  let gesture = null;   // {type:'band'|'drag', ...}

  desk.addEventListener('pointerdown', (e) => {
    if (e.target.closest('.dt-rename')) return;
    desk.classList.remove('is-kbd');
    if (renaming) commitRename({ fromBlur: true });
    if (e.button !== 0) return;
    if (document.activeElement !== desk) desk.focus({ preventScroll: true });
    const tile = e.target.closest('.dt-item');
    const additive = e.ctrlKey || e.metaKey;
    if (tile) {
      const key = tile.dataset.key;
      if (e.shiftKey) { rangeSelect(key); }
      else if (!selected.has(key) && !additive) { selectOnly(key); }
      else { focusKey = key; render(); }
      gesture = { type: 'press', key, x: e.clientX, y: e.clientY, id: e.pointerId, toggle: additive, wasSelected: selected.has(key) };
      if (additive && !selected.has(key)) { selected.add(key); anchorKey = key; render(); gesture.added = true; }
    } else {
      const r = desk.getBoundingClientRect();
      gesture = { type: 'band', x: e.clientX - r.left, y: e.clientY - r.top, id: e.pointerId, base: additive ? new Set(selected) : new Set(), moved: false };
      if (!additive && !e.shiftKey) setSelection([], { focus: focusKey });
    }
    try { desk.setPointerCapture(e.pointerId); } catch { /* ignore */ }
  });

  desk.addEventListener('pointermove', (e) => {
    if (!gesture || e.pointerId !== gesture.id) return;
    if (gesture.type === 'band') return moveBand(e);
    if (gesture.type === 'press') {
      if (Math.hypot(e.clientX - gesture.x, e.clientY - gesture.y) < 5) return;
      startDrag(e);
    }
    if (gesture.type === 'drag') moveDrag(e);
  });

  const endGesture = (e, cancelled = false) => {
    if (!gesture || e.pointerId !== gesture.id) return;
    const g = gesture;
    gesture = null;
    try { desk.releasePointerCapture(e.pointerId); } catch { /* ignore */ }
    if (g.type === 'band') { band.hidden = true; return; }
    if (g.type === 'drag') { if (cancelled) cancelDrag(g); else dropDrag(g, e); return; }
    if (g.type === 'press' && !cancelled) {
      // a plain click on an already selected item narrows the selection to it
      if (g.toggle) {
        if (!g.added && g.wasSelected) { selected.delete(g.key); render(); }
      } else if (!e.shiftKey && selected.size > 1) selectOnly(g.key);
    }
  };
  desk.addEventListener('pointerup', (e) => endGesture(e));
  desk.addEventListener('pointercancel', (e) => endGesture(e, true));
  desk.addEventListener('lostpointercapture', (e) => { if (gesture && gesture.id === e.pointerId) endGesture(e, gesture.type === 'drag'); });

  desk.addEventListener('dblclick', (e) => {
    if (e.target.closest('.dt-rename')) return;
    // pointer capture retargets click/dblclick to the desktop itself → look at what is under the pointer
    const tile = document.elementFromPoint(e.clientX, e.clientY)?.closest?.('.dt-item');
    if (!tile || !desk.contains(tile) || tile.classList.contains('dt-ghost')) return;
    const it = byKey.get(tile.dataset.key);
    if (it) openItems([it]);
  });

  function tileRects() {
    const r0 = desk.getBoundingClientRect();
    return model.map((it) => {
      const el = els.get(it.key);
      const r = el.getBoundingClientRect();
      return { key: it.key, l: r.left - r0.left, t: r.top - r0.top, r: r.right - r0.left, b: r.bottom - r0.top };
    });
  }

  function moveBand(e) {
    const g = gesture;
    const r0 = desk.getBoundingClientRect();
    const x = e.clientX - r0.left, y = e.clientY - r0.top;
    if (!g.moved && Math.hypot(x - g.x, y - g.y) < 3) return;
    g.moved = true;
    const l = Math.min(g.x, x), tp = Math.min(g.y, y), w = Math.abs(x - g.x), hh = Math.abs(y - g.y);
    Object.assign(band.style, { left: `${l}px`, top: `${tp}px`, width: `${w}px`, height: `${hh}px` });
    band.hidden = false;
    if (dset().showIcons === false) return;
    const hit = new Set(g.base);
    for (const tr of (g.rects ||= tileRects())) {
      const inside = tr.l < l + w && tr.r > l && tr.t < tp + hh && tr.b > tp;
      if (inside) { if (g.base.has(tr.key)) hit.delete(tr.key); else hit.add(tr.key); }
    }
    selected = hit;
    render();
  }

  /* ------------------------------------------------------------ drag & drop */
  function startDrag(e) {
    const g = gesture;
    if (!selected.has(g.key)) selectOnly(g.key);
    const keys = [g.key, ...[...selected].filter((k) => k !== g.key)];
    const r0 = desk.getBoundingClientRect();
    clear(ghosts);
    const list = keys.map((k) => {
      const el = els.get(k);
      const r = el.getBoundingClientRect();
      const ghost = el.cloneNode(true);
      ghost.removeAttribute('id');
      ghost.classList.add('dt-ghost');
      ghost.classList.remove('is-focused');
      ghost.style.left = `${r.left - r0.left}px`;
      ghost.style.top = `${r.top - r0.top}px`;
      ghost.style.zIndex = '';
      ghosts.append(ghost);
      el.classList.add('is-drag-source');
      return { key: k, ghost };
    });
    if (keys.length > 1) {
      const lead = list[0].ghost;
      lead.append(h('span', { class: 'dt-ghost__count' }, String(keys.length)));
    }
    gesture = { ...g, type: 'drag', keys, list, target: null };
    desk.classList.add('is-dragging');
  }

  function dropTargetAt(x, y, keys) {
    const el = document.elementFromPoint(x, y)?.closest?.('.dt-item');
    if (!el || !desk.contains(el) || el.classList.contains('dt-ghost')) return null;
    const it = byKey.get(el.dataset.key);
    if (!it || keys.includes(it.key)) return null;
    if (it.special === 'trash' || it.special === 'home' || (it.isDir && !it.special)) return it;
    return null;
  }

  function moveDrag(e) {
    const g = gesture;
    const dx = e.clientX - g.x, dy = e.clientY - g.y;
    for (const { ghost } of g.list) ghost.style.transform = `translate(${dx}px, ${dy}px)`;
    const target = dropTargetAt(e.clientX, e.clientY, g.keys);
    if (target?.key !== g.target?.key) {
      if (g.target) els.get(g.target.key)?.classList.remove('is-drop-target');
      if (target) els.get(target.key)?.classList.add('is-drop-target');
      g.target = target;
    }
  }

  function clearDrag(g) {
    clear(ghosts);
    desk.classList.remove('is-dragging');
    for (const k of g.keys) els.get(k)?.classList.remove('is-drag-source');
    if (g.target) els.get(g.target.key)?.classList.remove('is-drop-target');
  }

  function cancelDrag(g) {
    // fly the ghosts back
    for (const { ghost } of g.list) animate(ghost, [{ transform: ghost.style.transform }, { transform: 'none' }], { duration: 167 });
    setTimeout(() => clearDrag(g), 170);
  }

  async function dropDrag(g, e) {
    const target = g.target;
    const items = g.keys.map((k) => byKey.get(k)).filter(Boolean);
    if (target) {
      clearDrag(g);
      const fileItems = items.filter((i) => !i.special && i.path);
      if (!fileItems.length) return;
      if (target.special === 'trash') { await trashItems(fileItems); return; }
      const dest = target.special === 'home' ? homePath : target.path;
      try {
        const r = await bridge.call('files.move', { paths: fileItems.map((i) => i.path), dest });
        if (dryRun(r)) return;
        const moved = r?.paths || [];
        pushUndo({ label: t('undoMove'), run: () => bridge.call('files.move', { paths: moved, dest: dir }) });
        if (r?.failed?.length) errorAlert(null, r.failed[0]?.error);
      } catch (err) { errorAlert(err); }
      await reload();
      return;
    }
    const d = dset();
    if (d.autoArrange) { cancelDrag(g); return; }
    const dxc = (e.clientX - g.x) / metrics.cw, dyc = (e.clientY - g.y) / metrics.ch;
    const align = d.alignToGrid !== false;
    const occ = new Map();
    for (const [k, p] of placed) if (!g.keys.includes(k)) occ.set(cellId(Math.round(p.x), Math.round(p.y)), k);
    const next = new Map();
    for (const k of g.keys) {
      const o = placed.get(k);
      let x = clamp(o.x + dxc, 0, metrics.cols - 1), y = clamp(o.y + dyc, 0, metrics.rows - 1);
      if (align) {
        let c = Math.round(x), r = Math.round(y);
        if (occ.has(cellId(c, r))) [c, r] = nearestFree(c, r, occ, metrics.cols, metrics.rows);
        x = c; y = r;
      }
      occ.set(cellId(Math.round(x), Math.round(y)), k);
      next.set(k, { x, y });
    }
    // move the real tiles to where the ghosts are, then let them glide into the cell
    const r0 = desk.getBoundingClientRect();
    for (const { key, ghost } of g.list) {
      const el = els.get(key);
      const gr = ghost.getBoundingClientRect();
      el.style.left = `${gr.left - r0.left}px`;
      el.style.top = `${gr.top - r0.top}px`;
    }
    clearDrag(g);
    const pos = {};
    for (const [k, p] of placed) pos[k] = { x: p.x, y: p.y };
    for (const [k, p] of next) { pos[k] = p; placed.set(k, p); }
    void grid.offsetWidth;
    render({ animateMoves: true });
    try { await setSetting('desktop.positions', pos); } catch (err) { errorAlert(err); }
  }

  /* -------------------------------------------------------------- keyboard */
  let typeBuf = '', typeTimer = 0;

  function moveFocus(dirKey, e) {
    const cur = focusKey && byKey.has(focusKey) ? focusKey : null;
    let nextKey = null;
    if (!cur) nextKey = visualOrder()[0]?.key;
    else if (dirKey === 'Home') nextKey = visualOrder()[0]?.key;
    else if (dirKey === 'End') nextKey = visualOrder().at(-1)?.key;
    else {
      const p = placed.get(cur);
      const v = { ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0] }[dirKey];
      let best = null, bestScore = Infinity;
      for (const it of model) {
        if (it.key === cur) continue;
        const q = placed.get(it.key);
        const dx = q.x - p.x, dy = q.y - p.y;
        const along = dx * v[0] + dy * v[1];
        if (along <= 0.01) continue;
        const across = Math.abs(dx * v[1]) + Math.abs(dy * v[0]);
        const score = along + across * 3;
        if (score < bestScore) { bestScore = score; best = it.key; }
      }
      nextKey = best;
    }
    if (!nextKey) return;
    desk.classList.add('is-kbd');
    if (e.shiftKey) rangeSelect(nextKey);
    else if (e.ctrlKey) { focusKey = nextKey; render(); }
    else selectOnly(nextKey);
    els.get(nextKey)?.scrollIntoView?.({ block: 'nearest' });
  }

  desk.addEventListener('keydown', (e) => {
    if (e.target !== desk) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const sel = selectedItems();
    const k = e.key;
    const handled = () => { e.preventDefault(); e.stopPropagation(); };
    if (gesture?.type === 'drag' && k === 'Escape') { handled(); const g = gesture; gesture = null; cancelDrag(g); return; }
    if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(k)) { handled(); moveFocus(k, e); return; }
    if (ctrl && e.shiftKey && /^Digit[234]$/.test(e.code)) {
      handled();
      setIconSize({ Digit2: 'large', Digit3: 'medium', Digit4: 'small' }[e.code]);
      return;
    }
    if (ctrl && e.shiftKey && (k === 'N' || k === 'n')) { handled(); createNew('folder', null); return; }
    if (ctrl && e.shiftKey && (k === 'C' || k === 'c')) { handled(); if (sel.some((i) => i.path)) copyPath(sel.filter((i) => i.path)); return; }
    if (ctrl && !e.shiftKey) {
      const lk = k.toLowerCase();
      if (lk === 'a') { handled(); anchorKey = model[0]?.key; setSelection(model.map((i) => i.key), { focus: focusKey || model[0]?.key }); return; }
      if (lk === 'c') { handled(); setClipboard(sel, 'copy'); return; }
      if (lk === 'x') { handled(); setClipboard(sel, 'cut'); return; }
      if (lk === 'v') { handled(); paste(null); return; }
      if (lk === 'z') { handled(); runUndo(); return; }
      if (k === ' ' && focusKey) { handled(); if (selected.has(focusKey)) selected.delete(focusKey); else selected.add(focusKey); render(); return; }
    }
    if (k === 'Enter' && e.altKey) { handled(); properties(sel.filter((i) => !i.special || i.special === 'home')); return; }
    if (k === 'Enter') { handled(); if (sel.length) openItems(sel); return; }
    if (k === 'F2') { handled(); const f = sel.length === 1 ? sel[0] : byKey.get(focusKey); if (f) startRename(f.key); return; }
    if (k === 'F5') { handled(); reload({ flash: true }); return; }
    if (k === 'Delete') { handled(); trashItems(sel); return; }
    if (k === 'Escape') { if (selected.size) { handled(); setSelection([], { focus: focusKey }); } return; }
    if (k === 'ContextMenu' || (e.shiftKey && k === 'F10')) {
      handled();
      const el = sel.length ? els.get(focusKey && selected.has(focusKey) ? focusKey : sel[0].key) : null;
      const r = el ? el.getBoundingClientRect() : { left: 24, top: 24, width: 0, height: 0 };
      openMenuAt(r.left + Math.min(32, r.width / 2), r.top + Math.min(40, r.height / 2), { keyboard: true, onItem: !!el });
      return;
    }
    // type to select
    if (k.length === 1 && !ctrl && !e.altKey && k !== ' ') {
      handled();
      clearTimeout(typeTimer);
      typeBuf += k.toLowerCase();
      typeTimer = setTimeout(() => { typeBuf = ''; }, 900);
      const order = visualOrder();
      const start = Math.max(0, order.findIndex((i) => i.key === focusKey));
      const n = order.length;
      const single = typeBuf.length > 1 && [...typeBuf].every((c) => c === typeBuf[0]);
      const pref = single ? typeBuf[0] : typeBuf;
      for (let i = 0; i < n; i++) {
        const it = order[(start + (typeBuf.length === 1 || single ? i + 1 : i)) % n];
        if (it.label.toLowerCase().startsWith(pref)) { desk.classList.add('is-kbd'); selectOnly(it.key); break; }
      }
    }
  });

  // Ctrl + mouse wheel changes the icon size (like Windows)
  let ctlWheelLock = false;
  desk.addEventListener('wheel', (e) => {
    if (!e.ctrlKey) return;
    e.preventDefault();
    const i = SIZE_ORDER.indexOf(iconSize());
    const j = clamp(i + (e.deltaY < 0 ? 1 : -1), 0, SIZE_ORDER.length - 1);
    if (j !== i && !ctlWheelLock) { ctlWheelLock = true; setIconSize(SIZE_ORDER[j]); setTimeout(() => { ctlWheelLock = false; }, 250); }
  }, { passive: false });

  desk.addEventListener('focus', () => desk.classList.add('is-active'));
  desk.addEventListener('blur', () => desk.classList.remove('is-active'));

  /* ----------------------------------------------------------------- events */
  bridge.on('files.desktopChanged', () => reloadSoon());
  bridge.on('files.dirChanged', (d) => {
    if (!d) return;
    if (d.path === dir) reloadSoon();
    else if (String(d.path).startsWith('trash:')) loadTrash().then(() => refreshView());
  });
  bridge.on('files.trashChanged', () => loadTrash().then(() => refreshView()));
  bridge.on('files.clipboardChanged', (d) => { if (d && Array.isArray(d.paths)) { clip = d; render(); } });
  onSettings((s, path) => {
    if (!path || path.startsWith('wallpaper') || path.startsWith('theme')) wall.apply(s);
    if (!loaded) return;
    if (!path || path.startsWith('desktop') || path.startsWith('explorer') || path.startsWith('taskbar') || path.startsWith('accessibility') || path === 'language') {
      refreshView({ animateMoves: path === 'desktop.sortBy' || path === 'desktop.autoArrange' || path === 'desktop.positions' });
    }
  });
  // a language switch: our labels at once, then the native files service's launcher names (Name[nl]/Name[en]) and
  // typeName/displayName in the new language
  onLangChange(() => { if (loaded) { refreshView(); reloadSoon(); } });
  if ('ResizeObserver' in window) {
    let raf = 0;
    new ResizeObserver(() => { cancelAnimationFrame(raf); raf = requestAnimationFrame(() => { if (loaded) { layout(); render(); } }); }).observe(desk);
  }
  addEventListener('focus', () => { if (loaded) loadClip().then(render); });

  /* ------------------------------------------------------------------ start */
  // the wallpaper decodes in parallel; icons never wait for a large picture
  wall.apply(getSettings(), { instant: true }).catch((e) => console.warn('[desktop] wallpaper failed', e));
  await loadFiles();
  await Promise.all([loadTrash(), loadClip(), loadHome()]);
  loaded = true;
  refreshView();

  // debug helpers for screenshots
  const pre = (P.get('select') || '').split(',').filter((k) => byKey.has(k));
  if (pre.length) { anchorKey = pre[0]; setSelection(pre, { focus: pre[0] }); desk.focus({ preventScroll: true }); }
  const open = P.get('open');
  if (open) setTimeout(() => debugOpen(open), 900);

  function firstFileKey() { return model.find((i) => !i.special && !i.isDesktopEntry)?.key; }
  async function debugOpen(what) {
    const fk = selected.size ? [...selected][0] : firstFileKey();
    switch (what) {
      case 'contextmenu-desktop': {
        const r = desk.getBoundingClientRect();
        openMenuAt(Math.round(r.width * 0.42), Math.round(r.height * 0.22));
        break;
      }
      case 'contextmenu-item':
      case 'contextmenu-trash': {
        const key = what === 'contextmenu-trash' ? '::trash' : fk;
        if (!byKey.has(key)) return;
        if (!selected.has(key)) selectOnly(key);
        const r = els.get(key).getBoundingClientRect();
        openMenuAt(r.left + r.width * 0.6, r.top + r.height * 0.55, { onItem: true });
        break;
      }
      case 'properties': if (fk) { selectOnly(fk); properties([byKey.get(fk)]); } break;
      case 'rename': if (fk) { selectOnly(fk); startRename(fk); } break;
      case 'toast': showToast({ icon: 'trash', title: t('trashed'), message: t('trashedOne', { name: 'Notities.txt' }), action: { label: t('undoAction'), onClick() {} }, timeout: 0 }); break;
      case 'emptytrash': emptyTrash(); break;
      case 'launcher-trust':   // the question core/bridge.js asks before an untrusted .desktop file runs (demo data)
        bridge.demoLauncherQuestion({ path: `${dir}/minecraft-launcher.desktop`, name: 'Minecraft Launcher',
          exec: 'env GDK_BACKEND=x11 minecraft-launcher' });
        break;
      default:
    }
  }

  window.__desktop = { reload, select: (keys) => setSelection(keys), state: () => ({ dir, model: model.map((i) => i.key), selected: [...selected], placed: Object.fromEntries(placed) }) };
  return { reload };
}
