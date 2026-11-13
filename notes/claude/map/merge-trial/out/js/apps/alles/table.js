// Alles — virtualised results table (Win11 Explorer "Details" look).
//
// Only the visible rows exist in the DOM (a small recycled pool), pages of results
// are fetched on demand via onFetch(offset, limit), and very long lists (millions of
// rows) are scaled into a scrollable height browsers can handle.
//
//   const table = ResultsTable({ onFetch, onOpen, onContextMenu, onSort, onSelection, onFocusItem, onType, onKey });
//   table.reset(total, firstPageItems, { keepScroll });  table.setSort('size', true);  table.setHighlight(hl)

import { h, clamp } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { bytes, fileTime, number } from '../../core/format.js';
import { t } from './i18n.js';
import { highlightInto } from './highlight.js';

export const PAGE = 200;
const MAX_SCROLL = 8_000_000;       // px; browsers break down somewhere above ~16M
const MAX_PAGES = 80;

export const COLUMNS = [
  { id: 'name', sort: 'name', width: 340, min: 150 },
  { id: 'path', sort: 'path', width: 360, min: 120, flex: true },
  { id: 'size', sort: 'size', width: 100, min: 70, align: 'end', firstDesc: true },
  { id: 'modified', sort: 'modified', width: 148, min: 110, firstDesc: true },
];

const WIDTH_KEY = 'win11shell.alles.columns';

function loadWidths() {
  try {
    const v = JSON.parse(localStorage.getItem(WIDTH_KEY) || 'null');
    return v && typeof v === 'object' ? v : {};
  } catch { return {}; }
}
function saveWidths(w) {
  try { localStorage.setItem(WIDTH_KEY, JSON.stringify(w)); } catch { /* private mode */ }
}

/* ---------------------------------------------------------------- selection */
/** Selection as sorted, disjoint [a, b) ranges — "select all" of a million rows stays tiny. */
export class Selection {
  constructor() { this.r = []; }
  clear() { this.r = []; }
  count() { let n = 0; for (const [a, b] of this.r) n += b - a; return n; }
  has(i) {
    let lo = 0, hi = this.r.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      const [a, b] = this.r[mid];
      if (i < a) hi = mid - 1; else if (i >= b) lo = mid + 1; else return true;
    }
    return false;
  }
  set(i) { this.r = [[i, i + 1]]; }
  setRange(a, b) { this.r = [[Math.min(a, b), Math.max(a, b) + 1]]; }
  addRange(a, b) {
    let lo = Math.min(a, b), hi = Math.max(a, b) + 1;
    const out = [];
    for (const [x, y] of this.r) {
      if (y < lo || x > hi) out.push([x, y]);
      else { lo = Math.min(lo, x); hi = Math.max(hi, y); }
    }
    out.push([lo, hi]);
    out.sort((p, q) => p[0] - q[0]);
    this.r = out;
  }
  remove(i) {
    const out = [];
    for (const [a, b] of this.r) {
      if (i < a || i >= b) out.push([a, b]);
      else { if (a < i) out.push([a, i]); if (i + 1 < b) out.push([i + 1, b]); }
    }
    this.r = out;
  }
  toggle(i) { if (this.has(i)) this.remove(i); else this.addRange(i, i); }
  first() { return this.r.length ? this.r[0][0] : -1; }
  *indices(limit = Infinity) {
    let n = 0;
    for (const [a, b] of this.r) for (let i = a; i < b; i++) { if (n++ >= limit) return; yield i; }
  }
}

/* -------------------------------------------------------------------- icons */
const iconUrls = new Map();
function iconUrl(item) {
  const key = item.isDir ? '\u0000dir' : (item.mime || 'application/octet-stream');
  let u = iconUrls.get(key);
  if (u === undefined) {
    const px = Math.round(16 * Math.min(2, Math.max(1, devicePixelRatio || 1)));
    u = item.isDir ? bridge.iconUrl('folder', px) : bridge.mimeIconUrl(key, px);
    iconUrls.set(key, u || '');
  }
  return u;
}
let fallbackIcons = null;
function fallbackIcon(isDir) {
  if (!fallbackIcons) {
    const svg = (inner) => 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16">${inner}</svg>`);
    fallbackIcons = {
      dir: svg('<path d="M1.5 4a1.5 1.5 0 0 1 1.5-1.5h3.2l1.6 1.5H13A1.5 1.5 0 0 1 14.5 5.5v6A1.5 1.5 0 0 1 13 13H3A1.5 1.5 0 0 1 1.5 11.5Z" fill="#f5c044" stroke="#d29a1b"/>'),
      file: svg('<path d="M4 1.5h5l3.5 3.5v9a.5.5 0 0 1-.5.5H4a.5.5 0 0 1-.5-.5V2a.5.5 0 0 1 .5-.5Z" fill="#fff" stroke="#8a95a3"/><path d="M9 1.5V5h3.5" fill="none" stroke="#8a95a3"/>'),
    };
  }
  return isDir ? fallbackIcons.dir : fallbackIcons.file;
}

/* -------------------------------------------------------------------- table */
export function ResultsTable({ onFetch, onOpen, onContextMenu, onSort, onSelection, onFocusItem, onType, onKey } = {}) {
  const widths = { ...Object.fromEntries(COLUMNS.map((c) => [c.id, c.width])), ...loadWidths() };
  const sel = new Selection();
  let total = 0;
  let gen = 0;
  let pages = new Map();
  const pending = new Set();
  let focusIdx = -1;
  let anchor = -1;
  let hl = null;
  let sortBy = 'name';
  let sortDesc = false;
  let rowH = 32;
  let headH = 32;
  let viewH = 0;
  let scale = 1;
  let raf = 0;
  let loadTimer = 0;
  let lastScrollTs = 0;

  /* ---- header ---- */
  const headCells = {};
  const head = h('div', { class: 'alles-head', role: 'row' },
    COLUMNS.map((c) => {
      const glyph = h('span', { class: 'alles-sortglyph', 'aria-hidden': 'true' });
      const label = h('span', { class: 'alles-head__label' }, t(`col_${c.id}`));
      const cell = h('div', {
        class: ['alles-hcell', `c-${c.id}`, c.align === 'end' && 'is-end'], role: 'columnheader', tabindex: -1,
        'aria-sort': 'none', dataset: { col: c.id },
      }, glyph, label,
      h('span', { class: 'alles-resizer', 'aria-hidden': 'true', dataset: { col: c.id } }));
      cell.addEventListener('click', (e) => {
        if (e.target.closest('.alles-resizer')) return;
        onSort?.(c.sort, c);
      });
      headCells[c.id] = { cell, glyph };
      return cell;
    }));

  /* ---- body ---- */
  const rowsLayer = h('div', { class: 'alles-rows', role: 'rowgroup' });
  const spacer = h('div', { class: 'alles-spacer', 'aria-hidden': 'true' });
  const scroller = h('div', { class: 'alles-scroller' }, head, rowsLayer, spacer);
  const live = h('div', { class: 'fl-sr', 'aria-live': 'polite' });
  const root = h('div', {
    class: 'alles-table', role: 'grid', tabindex: 0, 'aria-label': t('resultsLabel'), 'aria-multiselectable': 'true',
    'aria-rowcount': 1,
  }, scroller, live);

  function applyWidths() {
    let sum = 0;
    for (const c of COLUMNS) {
      const w = Math.max(c.min, Math.round(widths[c.id] || c.width));
      widths[c.id] = w;
      root.style.setProperty(`--w-${c.id}`, `${w}px`);
      sum += w;
    }
    root.style.setProperty('--w-total', `${sum + 16}px`);
  }
  applyWidths();

  /* ---- row pool ---- */
  const slots = new Map();   // index → slot
  const free = [];
  let rid = 0;
  const idPrefix = `alles-r${Math.random().toString(36).slice(2, 7)}-`;

  function makeSlot() {
    const ic = h('img', { class: 'alles-ic', width: 16, height: 16, alt: '', draggable: false, decoding: 'async' });
    ic.addEventListener('error', () => {
      const s = ic.__slot;
      if (s?.item && !ic.dataset.fallback) { ic.dataset.fallback = '1'; ic.src = fallbackIcon(s.item.isDir); }
    });
    const nm = h('span', { class: 'alles-nm' });
    const cName = h('div', { class: 'alles-cell c-name', role: 'gridcell' }, h('span', { class: 'alles-icwrap' }, ic), nm);
    const pathText = h('span', { class: 'alles-pathtext', dir: 'ltr' });
    const cPath = h('div', { class: 'alles-cell c-path', role: 'gridcell' }, pathText);
    const cSize = h('div', { class: 'alles-cell c-size', role: 'gridcell' });
    const cMod = h('div', { class: 'alles-cell c-modified', role: 'gridcell' });
    const el = h('div', { class: 'alles-row', role: 'row', id: idPrefix + (rid++) }, cName, cPath, cSize, cMod);
    const slot = { el, ic, nm, cPath, pathText, cSize, cMod, index: -1, item: null, hlKey: '' };
    ic.__slot = slot;
    el.__slot = slot;
    return slot;
  }

  function fill(slot, i) {
    const item = itemAt(i);
    slot.index = i;
    slot.el.setAttribute('aria-rowindex', String(i + 2));
    if (!item) {
      if (slot.item !== null || !slot.el.classList.contains('is-placeholder')) {
        slot.item = null;
        slot.el.classList.add('is-placeholder');
        slot.ic.removeAttribute('src');
        slot.nm.textContent = '';
        slot.pathText.textContent = '';
        slot.cSize.textContent = '';
        slot.cMod.textContent = '';
      }
      return;
    }
    slot.el.classList.remove('is-placeholder');
    const hk = hl ? `${(hl.terms || []).join('\u0001')}\u0002${hl.regex || ''}\u0002${hl.case ? 1 : 0}${hl.path ? 1 : 0}` : '';
    if (slot.item === item && slot.hlKey === hk) return;
    slot.item = item;
    slot.hlKey = hk;
    const u = iconUrl(item);
    delete slot.ic.dataset.fallback;
    if (u) { if (slot.ic.getAttribute('src') !== u) slot.ic.src = u; } else slot.ic.src = fallbackIcon(item.isDir);
    highlightInto(slot.nm, item.name, hl);
    if (hl?.path) highlightInto(slot.pathText, item.dir || '/', hl);
    else slot.pathText.textContent = item.dir || '/';
    slot.cSize.textContent = item.isDir && !item.size ? '' : bytes(item.size);
    slot.cMod.textContent = item.modified ? fileTime(item.modified) : '';
    slot.el.classList.toggle('is-hidden-item', !!item.hidden);
    slot.el.classList.toggle('is-dir', !!item.isDir);
  }

  function decorate(slot) {
    const i = slot.index;
    const s = sel.has(i);
    slot.el.classList.toggle('is-selected', s);
    slot.el.classList.toggle('is-focused', i === focusIdx);
    slot.el.setAttribute('aria-selected', s ? 'true' : 'false');
  }

  /* ---- geometry ---- */
  function measure() {
    viewH = Math.max(0, scroller.clientHeight - headH);
    const contentH = total * rowH;
    const virtualH = Math.min(contentH, MAX_SCROLL);
    if (contentH > MAX_SCROLL && virtualH > viewH) scale = (contentH - viewH) / (virtualH - viewH);
    else scale = 1;
    spacer.style.height = `${Math.max(0, virtualH - viewH)}px`;
    rowsLayer.style.height = `${viewH}px`;
  }

  function offset() { return scroller.scrollTop * scale; }

  function render() {
    raf = 0;
    if (!viewH) measure();
    const s = offset();
    const first = Math.max(0, Math.floor(s / rowH) - 2);
    const last = Math.min(total - 1, Math.ceil((s + viewH) / rowH) + 2);
    for (const [i, slot] of slots) {
      if (i < first || i > last) { slots.delete(i); slot.el.remove(); slot.index = -1; free.push(slot); }
    }
    for (let i = first; i <= last; i++) {
      let slot = slots.get(i);
      if (!slot) {
        slot = free.pop() || makeSlot();
        slots.set(i, slot);
        slot.item = undefined;
        rowsLayer.append(slot.el);
      }
      fill(slot, i);
      decorate(slot);
      slot.el.style.transform = `translateY(${Math.round(i * rowH - s)}px)`;
    }
    const fs = slots.get(focusIdx);
    if (fs && root === document.activeElement) root.setAttribute('aria-activedescendant', fs.el.id);
    else root.removeAttribute('aria-activedescendant');
    scheduleLoads(first, last);
  }

  function queueRender() {
    if (!raf) raf = requestAnimationFrame(render);
  }

  /* ---- data ---- */
  function itemAt(i) {
    const p = pages.get(Math.floor(i / PAGE));
    return p ? p[i % PAGE] : undefined;
  }

  function scheduleLoads(first, last) {
    if (total <= 0) return;
    const fast = performance.now() - lastScrollTs < 60;
    clearTimeout(loadTimer);
    const run = () => {
      const p0 = Math.floor(first / PAGE), p1 = Math.floor(last / PAGE);
      for (let p = p0; p <= p1; p++) loadPage(p);
    };
    if (fast) loadTimer = setTimeout(run, 70); else run();
  }

  async function loadPage(p) {
    if (pages.has(p) || pending.has(p) || p * PAGE >= total) return;
    const g = gen;
    pending.add(p);
    try {
      const items = await onFetch?.(p * PAGE, PAGE);
      if (g !== gen || !Array.isArray(items)) return;
      pages.set(p, items);
      if (pages.size > MAX_PAGES) {
        const centre = Math.floor(offset() / rowH / PAGE);
        const far = [...pages.keys()].sort((a, b) => Math.abs(b - centre) - Math.abs(a - centre));
        for (const k of far.slice(0, pages.size - MAX_PAGES)) pages.delete(k);
      }
      queueRender();
      if (focusIdx >= p * PAGE && focusIdx < (p + 1) * PAGE) onFocusItem?.(itemAt(focusIdx), focusIdx);
    } catch (e) {
      if (e?.code !== 'cancelled') console.warn('[alles] page failed', e);
    } finally {
      pending.delete(p);
    }
  }

  /** Items for indices (fetching missing pages), in index order. */
  async function getItems(indices) {
    const need = new Set(indices.map((i) => Math.floor(i / PAGE)).filter((p) => !pages.has(p)));
    const g = gen;
    for (const p of need) {
      try {
        const items = await onFetch?.(p * PAGE, PAGE);
        if (g !== gen) return [];
        if (Array.isArray(items)) pages.set(p, items);
      } catch { /* skip */ }
    }
    return indices.map(itemAt).filter(Boolean);
  }

  /* ---- selection / focus ---- */
  function redecorate() { for (const slot of slots.values()) decorate(slot); }

  function announce() {
    const n = sel.count();
    onSelection?.(n);
  }

  function setFocus(i, { select = true, extend = false, toggle = false, scroll = true } = {}) {
    if (!total) return;
    i = clamp(i, 0, total - 1);
    focusIdx = i;
    if (extend && anchor >= 0) sel.setRange(anchor, i);
    else if (toggle) { sel.toggle(i); anchor = i; } else if (select) { sel.set(i); anchor = i; }
    if (scroll) ensureVisible(i);
    redecorate();
    queueRender();
    announce();
    onFocusItem?.(itemAt(i), i);
  }

  function ensureVisible(i) {
    const s = offset();
    const top = i * rowH;
    let target = null;
    if (top < s) target = top;
    else if (top + rowH > s + viewH) target = top + rowH - viewH;
    if (target != null) scroller.scrollTop = Math.round(target / scale);
  }

  /* ---- events ---- */
  scroller.addEventListener('scroll', () => { lastScrollTs = performance.now(); queueRender(); }, { passive: true });
  if ('ResizeObserver' in window) {
    new ResizeObserver(() => { measure(); queueRender(); }).observe(scroller);
  }

  // Windows (and Everything) draw the focus rectangle on a row only after keyboard use, never after a click.
  // :focus-visible cannot tell (the table is focused by script after a click), so track the input modality.
  const KBD_NAV = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown', 'Tab', 'F6']);
  document.addEventListener('keydown', (e) => { if (KBD_NAV.has(e.key)) root.classList.add('has-kbd'); }, true);
  document.addEventListener('pointerdown', () => root.classList.remove('has-kbd'), true);

  rowsLayer.addEventListener('pointerdown', (e) => {
    const row = e.target.closest('.alles-row');
    if (!row) return;
    if (document.activeElement !== root) root.focus({ preventScroll: true });
    const i = row.__slot.index;
    if (e.button === 2) {
      if (!sel.has(i)) setFocus(i, { scroll: false });
      return;
    }
    if (e.button !== 0) return;
    if (e.shiftKey) setFocus(i, { extend: true, scroll: false });
    else if (e.ctrlKey || e.metaKey) setFocus(i, { toggle: true, scroll: false });
    else setFocus(i, { scroll: false });
  });
  rowsLayer.addEventListener('dblclick', (e) => {
    const row = e.target.closest('.alles-row');
    if (!row || !row.__slot.item) return;
    onOpen?.(row.__slot.item, row.__slot.index, e);
  });
  rowsLayer.addEventListener('contextmenu', (e) => {
    const row = e.target.closest('.alles-row');
    if (!row) return;
    e.preventDefault();
    onContextMenu?.({ x: e.clientX, y: e.clientY }, row.__slot.index);
  });
  rowsLayer.addEventListener('pointerover', (e) => {
    const cell = e.target.closest('.c-path, .c-name');
    if (!cell) return;
    const isPath = cell.classList.contains('c-path');
    const target = isPath ? cell : cell.querySelector('.alles-nm');
    const text = target?.textContent;
    if (target && target.scrollWidth > target.clientWidth + 1) cell.dataset.tooltip = text;
    else delete cell.dataset.tooltip;
  });

  root.addEventListener('focus', () => {
    if (focusIdx < 0 && total) { focusIdx = clamp(sel.first(), 0, total - 1); redecorate(); }
    queueRender();
  });

  root.addEventListener('keydown', (e) => {
    if (onKey?.(e) === true) { e.preventDefault(); return; }
    const page = Math.max(1, Math.floor(viewH / rowH) - 1);
    const ctrl = e.ctrlKey || e.metaKey;
    const mv = (to) => {
      e.preventDefault();
      if (ctrl && !e.shiftKey) { focusIdx = clamp(to, 0, total - 1); ensureVisible(focusIdx); redecorate(); queueRender(); onFocusItem?.(itemAt(focusIdx), focusIdx); }
      else setFocus(to, { extend: e.shiftKey });
    };
    const cur = focusIdx < 0 ? -1 : focusIdx;
    switch (e.key) {
      case 'ArrowDown': mv(cur + 1); return;
      case 'ArrowUp': mv(cur < 0 ? 0 : cur - 1); return;
      case 'PageDown': mv(cur + page); return;
      case 'PageUp': mv(cur - page); return;
      case 'Home': mv(0); return;
      case 'End': mv(total - 1); return;
      case ' ':
        if (ctrl && focusIdx >= 0) { e.preventDefault(); sel.toggle(focusIdx); anchor = focusIdx; redecorate(); announce(); }
        return;
      case 'Enter': {
        const it = itemAt(focusIdx);
        if (it && !ctrl && !e.altKey) { e.preventDefault(); onOpen?.(it, focusIdx, e); }
        return;
      }
      case 'ContextMenu': case 'F10':
        if (e.key === 'F10' && !e.shiftKey) return;
        if (focusIdx >= 0) {
          e.preventDefault();
          const slot = slots.get(focusIdx);
          const r = (slot?.el || root).getBoundingClientRect();
          onContextMenu?.({ x: r.left + 48, y: r.top + rowH / 2, keyboard: true }, focusIdx);
        }
        return;
      default:
        if (ctrl && (e.key === 'a' || e.key === 'A')) {
          e.preventDefault();
          if (total) { sel.setRange(0, total - 1); anchor = 0; redecorate(); announce(); }
          return;
        }
        if (!ctrl && !e.altKey && e.key.length === 1 && e.key !== ' ') onType?.(e.key, e);
    }
  });

  /* column resizing (pointer drag on the header edge) */
  head.addEventListener('pointerdown', (e) => {
    const rz = e.target.closest('.alles-resizer');
    if (!rz || e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();
    const id = rz.dataset.col;
    const col = COLUMNS.find((c) => c.id === id);
    const startX = e.clientX;
    const startW = widths[id];
    rz.setPointerCapture(e.pointerId);
    root.classList.add('is-resizing');
    const move = (ev) => { widths[id] = Math.max(col.min, startW + ev.clientX - startX); applyWidths(); };
    const up = () => {
      rz.removeEventListener('pointermove', move);
      rz.removeEventListener('pointerup', up);
      rz.removeEventListener('pointercancel', up);
      root.classList.remove('is-resizing');
      saveWidths(widths);
    };
    rz.addEventListener('pointermove', move);
    rz.addEventListener('pointerup', up);
    rz.addEventListener('pointercancel', up);
  });
  head.addEventListener('dblclick', (e) => {
    const rz = e.target.closest('.alles-resizer');
    if (!rz) return;
    const col = COLUMNS.find((c) => c.id === rz.dataset.col);
    widths[col.id] = col.width;
    applyWidths();
    saveWidths(widths);
  });

  function renderSort() {
    for (const c of COLUMNS) {
      const { cell, glyph } = headCells[c.id];
      const on = c.sort === sortBy;
      cell.classList.toggle('is-sorted', on);
      cell.setAttribute('aria-sort', on ? (sortDesc ? 'descending' : 'ascending') : 'none');
      glyph.replaceChildren(on ? icon(sortDesc ? 'chevron-down' : 'chevron-up', { size: 8 }) : '');
    }
  }
  renderSort();

  /* ---- API ---- */
  const api = {
    el: root,
    scroller,
    /** New result set. firstItems = items of page 0 (already fetched). */
    reset(n, firstItems, { keepScroll = false, keepSelection = false } = {}) {
      gen++;
      total = Math.max(0, n | 0);
      pages = new Map();
      pending.clear();
      if (Array.isArray(firstItems)) pages.set(0, firstItems);
      root.setAttribute('aria-rowcount', String(total + 1));
      if (!keepSelection || focusIdx >= total) { sel.clear(); anchor = -1; focusIdx = -1; }
      measure();
      if (!keepScroll) scroller.scrollTop = 0;
      for (const slot of slots.values()) slot.item = undefined;
      render();
      announce();
    },
    get total() { return total; },
    setSort(by, desc) { sortBy = by; sortDesc = !!desc; renderSort(); },
    setHighlight(h2) { hl = h2 || null; },
    itemAt,
    getItems,
    focusIndex: () => focusIdx,
    focusedItem: () => itemAt(focusIdx),
    selection: sel,
    selectedCount: () => sel.count(),
    selectedIndices: (limit) => [...sel.indices(limit)],
    setFocus,
    selectFirst() { if (total) setFocus(0); },
    scrollToIndex(i) { if (total) { scroller.scrollTop = Math.round(clamp(i, 0, total - 1) * rowH / scale); queueRender(); } },
    /** The first visible row (for scrollToIndex after a reload). */
    topIndex: () => Math.max(0, Math.floor(offset() / rowH)),
    focus() { root.focus({ preventScroll: true }); },
    clearSelection() { sel.clear(); redecorate(); announce(); },
    rowHeight: () => rowH,
    setDensity(compact) {
      rowH = compact ? 24 : 32;
      root.classList.toggle('is-compact', !!compact);
      measure();
      queueRender();
    },
    refresh() { for (const slot of slots.values()) slot.item = undefined; queueRender(); },
    announce(text) { live.textContent = text; },
    countLabel: () => number(total),
  };
  return api;
}
