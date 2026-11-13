// Win11 Shell — Verkenner: the file list (details, list, icons, tiles, content views).
//
//   const v = FileView({ mode, columns, items, groups, selection, focusPath, cut, sort, compact, checkboxes,
//                        showExt, empty, collapsed, onOpen, onSelect, onContext, onRename, onSort,
//                        onColumnResize, onHeaderContext, dropInfo, onDrop, onMiddle });
//   v.el · v.focus() · v.select(paths, focus) · v.startRename(path) · v.scroll / v.restore(state) · v.setCut(set)
//
// Selection: click, Ctrl (toggle), Shift (range), rubber band on empty space, keyboard (arrows,
// Home/End, PgUp/PgDn, Shift/Ctrl variants, Space, Ctrl+A, type-to-select). Drag & drop with
// a Windows-like count badge. F2-style inline rename with validation balloon.

import { h, clear, clamp, appendChildren } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { t } from './i18n.js';
import { displayName, splitExt, fileIcon, fileUri, dirname } from './util.js';
import { DND_MIME, startDrag, endDrag, isFileDrag, dropPaths, showHint, hideHint, dragImage } from './dnd.js';

export const VIEW_MODES = ['xl', 'large', 'medium', 'small', 'list', 'details', 'tiles', 'content'];
export const ICON_SIZE = { xl: 256, large: 96, medium: 48, small: 16, list: 16, details: 16, tiles: 48, content: 32 };
const ORDER_NAV = new Set(['details', 'list', 'content']); // up/down follow the list order
const CHUNK = 400;

const checkGlyph = () => h('span', { class: 'xp-check', 'aria-hidden': 'true' },
  h('svg', { viewBox: '0 0 12 12', class: 'xp-check__glyph' }, h('path', { d: 'M2.25 6.4 4.75 8.9 9.75 3.6' })));

function sortGlyph(desc) {
  return h('svg', { class: 'xp-sortglyph', viewBox: '0 0 10 6', width: 10, height: 6, 'aria-hidden': 'true' },
    h('path', { d: desc ? 'M1 1l4 4 4-4' : 'M1 5l4-4 4 4' }));
}

export function FileView(o) {
  const mode = VIEW_MODES.includes(o.mode) ? o.mode : 'details';
  const items = o.items || [];
  const columns = (o.columns || []).filter((c) => !c.hidden);
  const selection = new Set(o.selection || []);
  const byPath = new Map(items.map((it) => [it.path, it]));
  for (const p of [...selection]) if (!byPath.has(p)) selection.delete(p);
  let focusPath = o.focusPath && byPath.has(o.focusPath) ? o.focusPath : null;
  let anchorPath = focusPath;
  let cut = o.cut || new Set();
  const collapsed = o.collapsed || new Set();
  const els = new Map();          // path → element
  let order = [];                 // rendered items in visual order (collapsed groups excluded)
  const iconSize = ICON_SIZE[mode];

  const scroller = h('div', {
    class: 'xp-scroll', tabindex: 0, role: mode === 'details' ? 'grid' : 'listbox',
    'aria-multiselectable': 'true', 'aria-label': o.label || '',
  });
  const list = h('div', { class: 'xp-items', role: 'presentation' });
  const band = h('div', { class: 'xp-band', hidden: true });
  const root = h('div', {
    class: ['xp-view', o.compact && 'is-compact', o.checkboxes && 'has-checks'],
    dataset: { mode },
  }, scroller);

  /* ------------------------------------------------------------ columns */
  let colWidths = columns.map((c) => Math.max(c.min || 60, c.width || 120));
  function applyCols() {
    if (mode !== 'details') return;
    root.style.setProperty('--cols', colWidths.map((w) => `${w}px`).join(' '));
    root.style.setProperty('--cols-w', `${colWidths.reduce((a, b) => a + b, 0) + (o.checkboxes ? 28 : 0) + 8}px`);
  }
  applyCols();

  let header = null;
  if (mode === 'details') {
    header = h('div', { class: 'xp-head', role: 'row' });
    if (o.checkboxes) {
      const all = checkGlyph();
      all.classList.add('xp-head__check');
      all.addEventListener('click', () => {
        if (selection.size === items.length) select([], focusPath); else select(items.map((x) => x.path), focusPath);
      });
      header.append(all);
    }
    columns.forEach((c, i) => {
      const sorted = o.sort?.key === c.id;
      const cell = h('div', {
        class: ['xp-head__cell', c.align === 'end' && 'is-end', sorted && 'is-sorted'],
        role: 'columnheader', tabindex: -1, 'aria-sort': sorted ? (o.sort.desc ? 'descending' : 'ascending') : 'none',
      },
      sorted ? sortGlyph(o.sort.desc) : null,
      h('span', { class: 'xp-head__label' }, c.label));
      if (c.sortable !== false) cell.addEventListener('click', (e) => { if (!e.target.closest('.xp-head__grip')) o.onSort?.(c.id); });
      const grip = h('div', { class: 'xp-head__grip', 'aria-hidden': 'true' });
      grip.addEventListener('pointerdown', (e) => {
        e.preventDefault(); e.stopPropagation();
        grip.setPointerCapture(e.pointerId);
        const x0 = e.clientX, w0 = colWidths[i];
        root.classList.add('is-resizing');
        const move = (ev) => { colWidths[i] = clamp(w0 + ev.clientX - x0, c.min || 60, 1200); applyCols(); };
        const up = () => {
          grip.removeEventListener('pointermove', move);
          root.classList.remove('is-resizing');
          o.onColumnResize?.(c.id, colWidths[i]);
        };
        grip.addEventListener('pointermove', move);
        grip.addEventListener('pointerup', up, { once: true });
        grip.addEventListener('pointercancel', up, { once: true });
      });
      grip.addEventListener('dblclick', (e) => { // auto-fit like Windows
        e.stopPropagation();
        let max = 60;
        for (const el of [...els.values()].slice(0, 400)) {
          const cellEl = el.children[i + (o.checkboxes ? 1 : 0)];
          const txt = cellEl?.querySelector('.xp-item__name, .xp-cell__text') || cellEl;
          if (txt) max = Math.max(max, txt.scrollWidth + (i === 0 ? 40 : 24));
        }
        colWidths[i] = clamp(max, c.min || 60, 900);
        applyCols();
        o.onColumnResize?.(c.id, colWidths[i]);
      });
      cell.append(grip);
      header.append(cell);
    });
    header.append(h('div', { class: 'xp-head__fill' }));
    header.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); o.onHeaderContext?.(e); });
    scroller.append(header);
  }
  scroller.append(list, band);

  /* ------------------------------------------------------------ items */
  function nameEl(item) {
    return h('span', { class: 'xp-item__name', title: mode === 'details' || mode === 'small' || mode === 'list' ? null : displayName(item, o.showExt) },
      displayName(item, o.showExt));
  }

  function makeItem(item) {
    const sel = selection.has(item.path);
    const el = h('div', {
      class: ['xp-item', sel && 'is-selected', cut.has(item.path) && 'is-cut', item.hidden && 'is-hidden', item.isDir && 'is-dir'],
      role: mode === 'details' ? 'row' : 'option', 'aria-selected': String(sel), tabindex: -1,
      draggable: o.draggable === false ? 'false' : 'true', dataset: { path: item.path },
    });
    const check = o.checkboxes ? checkGlyph() : null;
    const thumb = iconSize >= 32;
    if (mode === 'details') {
      if (check) el.append(check);
      columns.forEach((c, i) => {
        if (i === 0) {
          el.append(h('div', { class: 'xp-cell xp-cell--name', role: 'gridcell' }, fileIcon(item, 16), nameEl(item)));
        } else {
          const v = c.render ? c.render(item) : '';
          el.append(h('div', { class: ['xp-cell', c.align === 'end' && 'is-end'], role: 'gridcell' },
            v instanceof Node ? v : h('span', { class: 'xp-cell__text' }, v ?? '')));
        }
      });
    } else if (mode === 'tiles') {
      appendChildren(el, [check, fileIcon(item, 48, { thumb }),
        h('div', { class: 'xp-item__meta' }, nameEl(item),
          h('span', { class: 'xp-item__sub' }, o.typeOf?.(item) || ''),
          h('span', { class: 'xp-item__sub' }, item.isDir ? '' : (o.sizeOf?.(item) || '')))]);
    } else if (mode === 'content') {
      appendChildren(el, [check, fileIcon(item, 32, { thumb }),
        h('div', { class: 'xp-item__meta' }, nameEl(item), h('span', { class: 'xp-item__sub' }, o.typeOf?.(item) || '')),
        h('div', { class: 'xp-item__props' },
          h('span', { class: 'xp-item__sub' }, o.contentProps?.(item)?.[0] || ''),
          h('span', { class: 'xp-item__sub' }, o.contentProps?.(item)?.[1] || ''))]);
    } else {
      appendChildren(el, [check, fileIcon(item, iconSize, { thumb }), nameEl(item)]);
    }
    if (focusPath === item.path) { el.tabIndex = 0; el.classList.add('is-focus'); }
    els.set(item.path, el);
    order.push(item);
    return el;
  }

  function groupHeader(g) {
    const isCollapsed = collapsed.has(g.key);
    const btn = h('button', {
      type: 'button', class: 'xp-group', 'aria-expanded': String(!isCollapsed),
      'aria-label': `${g.label} (${g.items.length}) — ${isCollapsed ? t('expandGroup') : t('collapseGroup')}`,
    }, h('span', { class: 'xp-group__chev' }, icon(isCollapsed ? 'chevron-right' : 'chevron-down', { size: 12 })),
    h('span', { class: 'xp-group__label' }, g.label), h('span', { class: 'xp-group__count' }, `(${g.items.length})`),
    h('span', { class: 'xp-group__line' }));
    btn.addEventListener('click', () => {
      if (collapsed.has(g.key)) collapsed.delete(g.key); else collapsed.add(g.key);
      o.onCollapse?.(collapsed);
      rebuild();
    });
    btn.addEventListener('pointerdown', (e) => e.stopPropagation());
    return btn;
  }

  let chunkJob = 0;
  let revealWanted = null;        // {path, focus}: an item asked for (Weergeven in map) that is not rendered yet
  function revealNow(path, focus) {
    const el = els.get(path);
    if (!el) return false;
    if (focus && focusPath === path && document.activeElement === scroller) el.focus({ preventScroll: true });
    el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
    return true;
  }
  function rebuild() {
    cancelAnimationFrame(chunkJob);
    clear(list);
    els.clear();
    order = [];
    if (!items.length) {
      if (o.empty) list.append(h('div', { class: 'xp-empty' }, o.empty));
      return;
    }
    const sections = o.groups && o.groups.length ? o.groups : [{ key: '', label: '', items }];
    const queue = [];
    for (const g of sections) {
      if (g.label) queue.push({ group: g });
      if (g.label && collapsed.has(g.key)) continue;
      const box = mode === 'details' || mode === 'content' || !g.label ? null : h('div', { class: 'xp-groupbox' });
      if (box) queue.push({ box });
      for (const it of g.items) queue.push({ item: it, box });
    }
    const run = (start) => {
      const end = Math.min(queue.length, start + (start === 0 ? CHUNK : CHUNK * 2));
      const frag = document.createDocumentFragment();
      for (let i = start; i < end; i++) {
        const q = queue[i];
        if (q.group) frag.append(groupHeader(q.group));
        else if (q.box) { frag.append(q.box); }
        else {
          const el = makeItem(q.item);
          if (q.box) q.box.append(el); else frag.append(el);
        }
      }
      list.append(frag);
      if (revealWanted && revealNow(revealWanted.path, revealWanted.focus)) revealWanted = null;
      if (end < queue.length) chunkJob = requestAnimationFrame(() => run(end));
      else revealWanted = null;
    };
    run(0);
    if (!focusPath && order.length) setRoving(order[0].path, false);
  }

  /* -------------------------------------------------------- selection */
  function setRoving(path, focus = true) {
    if (focusPath && els.get(focusPath)) {
      const old = els.get(focusPath);
      old.tabIndex = -1;
      old.classList.remove('is-focus');
    }
    focusPath = path;
    const el = els.get(path);
    if (el) {
      el.tabIndex = 0;
      el.classList.add('is-focus');
      if (focus) {
        el.focus({ preventScroll: true });
        el.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      }
    }
  }

  function paint() {
    for (const [p, el] of els) {
      const s = selection.has(p);
      if (el.classList.contains('is-selected') !== s) {
        el.classList.toggle('is-selected', s);
        el.setAttribute('aria-selected', String(s));
      }
    }
    root.classList.toggle('has-selection', selection.size > 0);
    if (header && o.checkboxes) header.firstChild.classList.toggle('is-on', selection.size > 0 && selection.size === items.length);
  }

  function emit() { paint(); o.onSelect?.(new Set(selection), focusPath); }

  function select(paths, focus = focusPath, { scroll = false, silent = false } = {}) {
    selection.clear();
    for (const p of paths) if (byPath.has(p)) selection.add(p);
    if (focus && byPath.has(focus)) { setRoving(focus, scroll); anchorPath = focus; }
    if (silent) paint(); else emit();
  }

  function selectRange(fromPath, toPath, add) {
    const a = order.findIndex((x) => x.path === fromPath);
    const b = order.findIndex((x) => x.path === toPath);
    if (b < 0) return;
    const [lo, hi] = a < 0 ? [b, b] : [Math.min(a, b), Math.max(a, b)];
    if (!add) selection.clear();
    for (let i = lo; i <= hi; i++) selection.add(order[i].path);
  }

  const itemOf = (el) => (el ? byPath.get(el.dataset.path) : null);

  /* ---------------------------------------------------------- pointer */
  let pending = null; // {kind:'toggle'|'only', path}
  let bandState = null;

  scroller.addEventListener('pointerdown', (e) => {
    if (renameState && !renameState.box.contains(e.target)) commitRename();
    if (e.target.closest('.xp-head, .xp-group, .xp-rename, .xp-empty button')) return;
    const r = scroller.getBoundingClientRect();
    if (e.clientX - r.left >= scroller.clientWidth || e.clientY - r.top >= scroller.clientHeight) return; // scrollbar
    const itemEl = e.target.closest('.xp-item');
    const item = itemOf(itemEl);
    if (e.button === 1) {
      if (item) { e.preventDefault(); o.onMiddle?.(item, e); }
      return;
    }
    if (e.button === 2) {
      if (item && !selection.has(item.path)) { select([item.path], item.path); }
      else if (!item && !e.ctrlKey) { if (selection.size) select([], focusPath); }
      if (item) setRoving(item.path, true); else scroller.focus({ preventScroll: true });
      return;
    }
    if (e.button !== 0) return;
    pending = null;
    if (item) {
      if (e.target.closest('.xp-check')) {
        e.preventDefault();
        if (selection.has(item.path)) selection.delete(item.path); else selection.add(item.path);
        anchorPath = item.path;
        setRoving(item.path, true);
        emit();
        return;
      }
      if (e.shiftKey) {
        selectRange(anchorPath || item.path, item.path, e.ctrlKey || e.metaKey);
        setRoving(item.path, true);
        emit();
      } else if (e.ctrlKey || e.metaKey) {
        pending = { kind: 'toggle', path: item.path };
        setRoving(item.path, true);
      } else if (!selection.has(item.path)) {
        select([item.path], item.path);
        setRoving(item.path, true);
      } else {
        pending = { kind: 'only', path: item.path };
        setRoving(item.path, true);
      }
      return;
    }
    // empty space → rubber band
    e.preventDefault();
    scroller.focus({ preventScroll: true });
    const base = e.ctrlKey || e.metaKey || e.shiftKey ? new Set(selection) : new Set();
    if (!base.size && selection.size) { selection.clear(); paint(); }
    const p = toContent(e);
    bandState = { x0: p.x, y0: p.y, base, rects: null, id: e.pointerId, lastX: e.clientX, lastY: e.clientY, moved: false, raf: 0 };
    scroller.setPointerCapture(e.pointerId);
  });

  function toContent(e) {
    const r = scroller.getBoundingClientRect();
    return { x: e.clientX - r.left + scroller.scrollLeft, y: e.clientY - r.top + scroller.scrollTop };
  }

  function bandUpdate() {
    if (!bandState) return;
    const s = bandState;
    const r = scroller.getBoundingClientRect();
    const p = { x: s.lastX - r.left + scroller.scrollLeft, y: s.lastY - r.top + scroller.scrollTop };
    const x1 = Math.min(s.x0, p.x), y1 = Math.min(s.y0, p.y), x2 = Math.max(s.x0, p.x), y2 = Math.max(s.y0, p.y);
    if (!s.moved && (x2 - x1 > 3 || y2 - y1 > 3)) { s.moved = true; band.hidden = false; }
    if (!s.moved) return;
    Object.assign(band.style, { left: `${x1}px`, top: `${y1}px`, width: `${x2 - x1}px`, height: `${y2 - y1}px` });
    if (!s.rects) {
      s.rects = [];
      for (const [path, el] of els) {
        if (!el.offsetParent) continue;
        const left = offsetIn(el);
        s.rects.push({ path, l: left.x, t: left.y, r: left.x + el.offsetWidth, b: left.y + el.offsetHeight });
      }
    }
    selection.clear();
    for (const p0 of s.base) selection.add(p0);
    for (const q of s.rects) {
      if (q.r > x1 && q.l < x2 && q.b > y1 && q.t < y2) {
        if (s.base.has(q.path)) selection.delete(q.path); else selection.add(q.path);
      }
    }
    paint();
  }

  function offsetIn(el) {
    let x = 0, y = 0, n = el;
    while (n && n !== scroller) { x += n.offsetLeft; y += n.offsetTop; n = n.offsetParent; }
    return { x, y };
  }

  function autoScroll() {
    if (!bandState) return;
    const r = scroller.getBoundingClientRect();
    const s = bandState;
    let dy = 0, dx = 0;
    const edge = 28;
    if (s.lastY < r.top + edge + (header ? header.offsetHeight : 0)) dy = -Math.ceil((r.top + edge - s.lastY) / 3) - 4;
    else if (s.lastY > r.bottom - edge) dy = Math.ceil((s.lastY - r.bottom + edge) / 3) + 4;
    if (mode === 'list') {
      if (s.lastX < r.left + edge) dx = -12; else if (s.lastX > r.right - edge) dx = 12;
    }
    if (dy || dx) { scroller.scrollTop += dy; scroller.scrollLeft += dx; bandUpdate(); }
    s.raf = requestAnimationFrame(autoScroll);
  }

  scroller.addEventListener('pointermove', (e) => {
    if (!bandState || e.pointerId !== bandState.id) return;
    bandState.lastX = e.clientX; bandState.lastY = e.clientY;
    bandUpdate();
    if (bandState.moved && !bandState.raf) bandState.raf = requestAnimationFrame(autoScroll);
  });
  const endBand = (e) => {
    if (!bandState || (e && e.pointerId !== bandState.id)) return;
    cancelAnimationFrame(bandState.raf);
    band.hidden = true;
    const moved = bandState.moved;
    bandState = null;
    if (moved) {
      const first = order.find((x) => selection.has(x.path));
      if (first) { anchorPath = first.path; setRoving(first.path, false); }
    }
    emit();
  };
  scroller.addEventListener('pointerup', endBand);
  scroller.addEventListener('pointercancel', endBand);
  scroller.addEventListener('lostpointercapture', endBand);

  scroller.addEventListener('click', (e) => {
    if (!pending) return;
    const item = itemOf(e.target.closest('.xp-item'));
    const p = pending;
    pending = null;
    if (!item || item.path !== p.path) return;
    if (p.kind === 'toggle') {
      if (selection.has(item.path)) selection.delete(item.path); else selection.add(item.path);
      anchorPath = item.path;
      emit();
    } else if (p.kind === 'only' && (selection.size > 1)) {
      select([item.path], item.path);
    }
  });

  scroller.addEventListener('dblclick', (e) => {
    if (e.target.closest('.xp-head, .xp-group, .xp-rename, .xp-check')) return;
    const item = itemOf(e.target.closest('.xp-item'));
    if (item) { e.preventDefault(); o.onOpen?.(item, e); }
  });

  scroller.addEventListener('contextmenu', (e) => {
    if (e.target.closest('.xp-head, .xp-rename')) return;
    e.preventDefault();
    const item = itemOf(e.target.closest('.xp-item'));
    o.onContext?.(e, item || null);
  });

  /* --------------------------------------------------------- keyboard */
  let typeBuf = '', typeTimer = 0;
  function rectOf(path) {
    const el = els.get(path);
    if (!el) return null;
    const p = offsetIn(el);
    return { x: p.x, y: p.y, w: el.offsetWidth, h: el.offsetHeight, cx: p.x + el.offsetWidth / 2, cy: p.y + el.offsetHeight / 2 };
  }
  function neighbor(path, dir) {
    const idx = order.findIndex((x) => x.path === path);
    if (idx < 0) return order[0]?.path;
    if (ORDER_NAV.has(mode) && (dir === 'down' || dir === 'up')) {
      const j = idx + (dir === 'down' ? 1 : -1);
      return order[j]?.path || null;
    }
    if (mode !== 'list' && ORDER_NAV.has(mode)) return null; // left/right in rows: nothing
    const r0 = rectOf(path);
    if (!r0) return null;
    let best = null, bestA = Infinity, bestB = Infinity;
    for (const it of order) {
      if (it.path === path) continue;
      const r = rectOf(it.path);
      if (!r || (r.w === 0 && r.h === 0)) continue;
      let a, b;
      if (dir === 'down') { if (r.cy <= r0.cy + 2) continue; a = r.cy - r0.cy; b = Math.abs(r.cx - r0.cx); }
      else if (dir === 'up') { if (r.cy >= r0.cy - 2) continue; a = r0.cy - r.cy; b = Math.abs(r.cx - r0.cx); }
      else if (dir === 'right') { if (r.cx <= r0.cx + 2) continue; a = r.cx - r0.cx; b = Math.abs(r.cy - r0.cy); }
      else { if (r.cx >= r0.cx - 2) continue; a = r0.cx - r.cx; b = Math.abs(r.cy - r0.cy); }
      if (mode !== 'list' && (dir === 'left' || dir === 'right') && b > 2) continue; // stay on this row
      if (mode !== 'list' && (dir === 'up' || dir === 'down')) { [a, b] = [Math.round(a / 4), b]; }
      if (a < bestA - 0.5 || (Math.abs(a - bestA) <= 0.5 && b < bestB)) { best = it.path; bestA = a; bestB = b; }
    }
    return best;
  }

  function moveTo(path, e) {
    if (!path) return;
    if (e.shiftKey) {
      selectRange(anchorPath || focusPath || path, path, e.ctrlKey);
      setRoving(path, true);
      emit();
    } else if (e.ctrlKey || e.metaKey) {
      setRoving(path, true);
    } else {
      select([path], path, { scroll: true });
    }
  }

  scroller.addEventListener('keydown', (e) => {
    if (e.target.closest('.xp-rename')) return;
    if (!order.length) return;
    const cur = focusPath || order[0].path;
    const k = e.key;
    const dirs = { ArrowDown: 'down', ArrowUp: 'up', ArrowLeft: 'left', ArrowRight: 'right' };
    if (dirs[k] && !e.altKey) {
      e.preventDefault();
      if (!focusPath || (e.target === scroller && !selection.size)) { moveTo(focusPath || order[0].path, e); return; }
      moveTo(neighbor(cur, dirs[k]), e);
      return;
    }
    if ((k === 'Home' || k === 'End') && !e.altKey) {
      e.preventDefault();
      moveTo(k === 'Home' ? order[0].path : order[order.length - 1].path, e);
      return;
    }
    if (k === 'PageDown' || k === 'PageUp') {
      e.preventDefault();
      const r0 = rectOf(cur);
      const rowH = r0?.h || 32;
      const steps = Math.max(1, Math.floor(scroller.clientHeight / rowH) - 1);
      let p = cur;
      for (let i = 0; i < steps; i++) { const n = neighbor(p, k === 'PageDown' ? 'down' : 'up'); if (!n) break; p = n; }
      moveTo(p, e);
      return;
    }
    if (k === ' ' && !e.altKey) {
      e.preventDefault();
      if (e.ctrlKey) { if (selection.has(cur)) selection.delete(cur); else selection.add(cur); anchorPath = cur; emit(); }
      else if (!selection.has(cur)) select([cur], cur);
      return;
    }
    if ((e.ctrlKey || e.metaKey) && !e.altKey && (k === 'a' || k === 'A') && !e.shiftKey) {
      e.preventDefault();
      select(order.map((x) => x.path), focusPath);
      return;
    }
    if (k.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && k !== ' ') {
      clearTimeout(typeTimer);
      typeTimer = setTimeout(() => { typeBuf = ''; }, 1000);
      const same = typeBuf.length === 1 && typeBuf === k.toLowerCase();
      typeBuf = same ? typeBuf : typeBuf + k.toLowerCase();
      const startIdx = order.findIndex((x) => x.path === cur);
      const n = order.length;
      for (let i = same || typeBuf.length === 1 ? 1 : 0; i <= n; i++) {
        const it = order[(startIdx + i + n) % n];
        if (displayName(it, o.showExt).toLowerCase().startsWith(typeBuf)) {
          e.preventDefault();
          select([it.path], it.path, { scroll: true });
          break;
        }
      }
    }
  });


  /* ------------------------------------------------------ drag & drop */
  scroller.addEventListener('dragstart', (e) => {
    const el = e.target.closest?.('.xp-item');
    const item = itemOf(el);
    if (!item || renameState) { e.preventDefault(); return; }
    pending = null;
    if (!selection.has(item.path)) select([item.path], item.path);
    const paths = order.filter((x) => selection.has(x.path)).map((x) => x.path);
    try {
      e.dataTransfer.setData(DND_MIME, JSON.stringify(paths));
      e.dataTransfer.setData('text/uri-list', paths.map(fileUri).join('\r\n'));
      e.dataTransfer.setData('text/plain', paths.join('\n'));
      e.dataTransfer.effectAllowed = 'copyMove';
    } catch { /* ignore */ }
    dragImage(e, fileIcon(item, 64, { thumb: true }), paths.length);
    startDrag(paths, o.srcDir || dirname(item.path));
    for (const p of paths) els.get(p)?.classList.add('is-dragging');
  });
  scroller.addEventListener('dragend', () => {
    for (const el of els.values()) el.classList.remove('is-dragging');
    endDrag();
    clearDropMark();
  });

  let dropMark = null;
  function clearDropMark() { dropMark?.classList.remove('is-drop'); dropMark = null; root.classList.remove('is-drop'); }
  scroller.addEventListener('dragover', (e) => {
    if (!isFileDrag(e) || !o.dropInfo) return;
    const el = e.target.closest?.('.xp-item');
    const item = itemOf(el);
    const target = item && item.isDir ? item : null;
    const info = o.dropInfo(target, e);
    if (!info) { e.dataTransfer.dropEffect = 'none'; clearDropMark(); hideHint(); return; }
    e.preventDefault();
    e.dataTransfer.dropEffect = info.copy ? 'copy' : 'move';
    const mark = target ? el : null;
    if (mark !== dropMark) { clearDropMark(); dropMark = mark; mark?.classList.add('is-drop'); }
    root.classList.toggle('is-drop', !mark);
    showHint(e.clientX, e.clientY, info.label, info.copy ? 'copy' : 'move');
  });
  scroller.addEventListener('dragleave', (e) => {
    if (!scroller.contains(e.relatedTarget)) { clearDropMark(); hideHint(); }
  });
  scroller.addEventListener('drop', (e) => {
    if (!o.dropInfo) return;
    const el = e.target.closest?.('.xp-item');
    const item = itemOf(el);
    const target = item && item.isDir ? item : null;
    const info = o.dropInfo(target, e);
    clearDropMark();
    hideHint();
    if (!info) return;
    e.preventDefault();
    const { paths, external } = dropPaths(e);
    endDrag();
    if (paths.length) o.onDrop?.({ paths, dest: info.dest, copy: info.copy || external, label: info.label });
  });

  /* ------------------------------------------------------------ rename */
  let renameState = null;
  function startRename(path) {
    const item = byPath.get(path);
    const el = els.get(path);
    if (!item || !el) return false;
    if (renameState) cancelRename();
    select([path], path, { scroll: true });
    const nameSpan = el.querySelector('.xp-item__name');
    const full = item.name;
    const [stem, ext] = splitExt(full);
    const hideExt = !o.showExt && !item.isDir && ext && stem;
    const value = hideExt ? stem : full;
    const multi = !(mode === 'details' || mode === 'small' || mode === 'list' || mode === 'content' || mode === 'tiles');
    const input = h(multi ? 'textarea' : 'input', {
      class: 'xp-rename__input', spellcheck: false, autocomplete: 'off', 'aria-label': t('renameMenu'),
      rows: multi ? 1 : null,
    });
    input.value = value;
    const tip = h('div', { class: 'xp-rename__tip', role: 'alert', hidden: true });
    const box = h('div', { class: ['xp-rename', multi && 'is-multi'] }, input, tip);
    nameSpan.hidden = true;
    nameSpan.after(box);
    el.draggable = false;
    const fit = () => { if (multi) { input.style.height = 'auto'; input.style.height = `${input.scrollHeight}px`; } };
    renameState = { item, el, nameSpan, input, box, tip, hideExt, ext, busy: false };
    requestAnimationFrame(() => {
      fit();
      input.focus();
      const selEnd = item.isDir || hideExt || !ext || !stem ? value.length : stem.length;
      input.setSelectionRange(0, selEnd);
    });
    input.addEventListener('input', () => { fit(); showTip(validate(input.value)); });
    input.addEventListener('keydown', (e) => {
      e.stopPropagation();
      if (e.key === 'Enter') { e.preventDefault(); commitRename(); }
      else if (e.key === 'Escape') { e.preventDefault(); cancelRename(); }
      else if (e.key === 'Tab') { e.preventDefault(); commitRename(); }
    });
    input.addEventListener('blur', () => setTimeout(() => { if (renameState?.input === input) commitRename(); }, 0));
    input.addEventListener('pointerdown', (e) => e.stopPropagation());
    input.addEventListener('dblclick', (e) => e.stopPropagation());
    return true;
  }
  function validate(v) {
    const s = v.trim();
    if (!s) return t('nameEmpty');
    if (s.includes('/')) return t('nameSlash');
    if (s === '.' || s === '..') return t('nameBad');
    return '';
  }
  function showTip(msg) {
    if (!renameState) return;
    renameState.tip.textContent = msg || '';
    renameState.tip.hidden = !msg;
  }
  async function commitRename() {
    const s = renameState;
    if (!s || s.busy) return;
    const raw = s.input.value.trim();
    const err = validate(raw);
    const newName = s.hideExt ? raw + s.ext : raw;
    if (!raw || newName === s.item.name) { cancelRename(); return; }
    if (err) { showTip(err); s.input.focus(); return; }
    s.busy = true;
    const res = await o.onRename?.(s.item, newName);
    s.busy = false;
    if (typeof res === 'string' && res) { showTip(res); s.input.focus(); return; }
    if (res === false) { s.input.focus(); return; }
    finishRename(newName);
  }
  function finishRename(newName) {
    const s = renameState;
    if (!s) return;
    renameState = null;
    s.box.remove();
    s.nameSpan.hidden = false;
    if (newName) s.nameSpan.textContent = displayName({ ...s.item, name: newName, displayName: null }, o.showExt);
    s.el.draggable = o.draggable !== false;
    s.el.focus({ preventScroll: true });
    o.onRenameEnd?.(newName);
  }
  function cancelRename() { finishRename(null); }

  /* -------------------------------------------------------------- api */
  rebuild();
  paint();

  return {
    el: root,
    scroller,
    get selection() { return new Set(selection); },
    get focusPath() { return focusPath; },
    get order() { return order.slice(); },
    get renaming() { return !!renameState; },
    /** Focus the selected item (keyboard continues from there) or the list itself — no focus ring on a fresh folder. */
    focus() {
      const el = focusPath && selection.has(focusPath) && els.get(focusPath);
      if (el) el.focus({ preventScroll: true }); else scroller.focus({ preventScroll: true });
    },
    select(paths, focus, opts) { select(paths, focus, opts); },
    selectAll() { select(order.map((x) => x.path), focusPath); },
    invert() { select(order.filter((x) => !selection.has(x.path)).map((x) => x.path), focusPath); },
    startRename,
    /** Scroll an item into view; in a big folder that is still being drawn (chunks of CHUNK per frame) as soon as
     *  its row exists. focus: give it the keyboard focus too if the list has it. */
    reveal(path, { focus = false } = {}) {
      revealWanted = null;
      if (!revealNow(path, focus) && byPath.has(path)) revealWanted = { path, focus };
    },
    setCut(set) {
      cut = set || new Set();
      for (const [p, el] of els) el.classList.toggle('is-cut', cut.has(p));
    },
    get scroll() { return { top: scroller.scrollTop, left: scroller.scrollLeft }; },
    restore(s) { if (s) { scroller.scrollTop = s.top || 0; scroller.scrollLeft = s.left || 0; } },
    destroy() { cancelAnimationFrame(chunkJob); clearTimeout(typeTimer); },
  };
}
