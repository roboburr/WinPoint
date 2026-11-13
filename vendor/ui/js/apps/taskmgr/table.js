// Taakbeheer — DataTable: the Win11 Task Manager grid.
//
// Sortable + resizable columns (widths remembered), optional groups ("Apps (8)"),
// expandable rows (chevron, child rows), heat-map cells, single/multi selection,
// full keyboard navigation, type-ahead, context menu, and keyed live updates that keep
// scroll position, focus, selection and expansion while the data refreshes. Rendering is time-sliced
// (≈8 ms per frame), so hundreds of new rows never block the page; API calls that need every row in the
// DOM (select, rowEl, keyboard focus) finish a pending render first.
//
//   const t = DataTable({ columns, rowKey, childrenOf, storageKey, defaultSort, onSelectionChange,
//                         onActivate, onDelete, contextMenu, rowClass, multiSelect, emptyText, ariaLabel });
//   t.setGroups([{ id, label, rows }])  or  t.setRows(rows)
//   t.setFilter((row) => bool)  t.setHeaderValues({ cpu: '23%' })  t.getSelection()  t.select(key)

import { h, clear, clamp } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bindContextMenu } from '../../core/contextmenu.js';
import { makeT, collator } from '../../core/i18n.js';
import { load, save, heatBucket } from './util.js';

const t = makeT({
  nl: { columns: 'Kolommen', expand: 'Uitvouwen', collapse: 'Samenvouwen', sortAsc: 'Oplopend gesorteerd', sortDesc: 'Aflopend gesorteerd', reset: 'Standaardkolommen herstellen' },
  en: { columns: 'Columns', expand: 'Expand', collapse: 'Collapse', sortAsc: 'Sorted ascending', sortDesc: 'Sorted descending', reset: 'Reset columns' },
});

const COLL_OPTS = { sensitivity: 'base', numeric: true };

export function DataTable({
  columns, rowKey, childrenOf = null, storageKey = null, defaultSort = null, onSelectionChange, onActivate,
  onDelete, contextMenu, rowClass, multiSelect = false, emptyText = '', ariaLabel = '', groupCounts = true,
  rowHeight = 30,
} = {}) {
  const saved = storageKey ? load(`table.${storageKey}`, {}) : {};
  const state = {
    widths: { ...(saved.widths || {}) },
    hidden: new Set(saved.hidden ?? columns.filter((c) => c.hidden).map((c) => c.id)),
    sort: saved.sort || defaultSort || null, // {id, desc}
  };
  const persist = () => {
    if (!storageKey) return;
    save(`table.${storageKey}`, { widths: state.widths, hidden: [...state.hidden], sort: state.sort });
  };

  let groups = [];
  let filter = null;
  let items = [];                 // visible items (groups + rows)
  const expanded = new Set();     // row keys
  const autoExpanded = new Set(); // expanded because of the filter
  const selected = new Set();
  let anchorKey = null;
  let focusKey = null;
  const rowCache = new Map();     // key → {el, cells, row, cls}
  const groupCache = new Map();

  const head = h('div', { class: 'tm-head', role: 'row' });
  const body = h('div', { class: 'tm-body', role: 'rowgroup' });
  const empty = h('div', { class: 'tm-empty', role: 'row' }, h('div', { role: 'gridcell' }, emptyText));
  const root = h('div', {
    class: 'tm-table', role: childrenOf ? 'treegrid' : 'grid', 'aria-label': ariaLabel || null,
    'aria-multiselectable': multiSelect ? 'true' : null, tabindex: 0, style: { '--tm-row-h': `${rowHeight}px` },
  }, head, body);

  const visibleCols = () => columns.filter((c) => !state.hidden.has(c.id));
  const widthOf = (c) => state.widths[c.id] ?? c.width ?? 100;

  function applyTemplate() {
    const tpl = visibleCols().map((c) => (c.flex ? `minmax(${widthOf(c)}px, 1fr)` : `${widthOf(c)}px`)).join(' ');
    root.style.setProperty('--tm-cols', tpl);
    const min = visibleCols().reduce((a, c) => a + widthOf(c), 0);
    root.style.setProperty('--tm-min-w', `${min}px`);
  }

  /* ---------------------------------------------------------------- header */
  const headerCells = new Map();
  function buildHeader() {
    clear(head);
    headerCells.clear();
    for (const c of visibleCols()) {
      const sortEl = h('span', { class: 'tm-th__sort', 'aria-hidden': 'true' });
      const valEl = h('span', { class: 'tm-th__val' });
      const labEl = h('span', { class: 'tm-th__label' }, c.label);
      const grip = h('span', { class: 'tm-th__grip', 'aria-hidden': 'true' });
      const th = h('div', {
        class: ['tm-th', `is-${c.align || 'left'}`, c.sortable === false && 'is-static'],
        role: 'columnheader', tabindex: c.sortable === false ? -1 : 0, dataset: { col: c.id },
        title: null,
      }, sortEl, h('span', { class: 'tm-th__text' }, valEl, labEl), grip);
      if (c.headerTip) th.dataset.tooltip = c.headerTip;
      headerCells.set(c.id, { th, valEl, sortEl });
      let resized = false;
      th.addEventListener('click', () => {
        if (resized) { resized = false; return; }
        if (c.sortable === false) return;
        sortBy(c.id);
      });
      th.addEventListener('keydown', (e) => {
        if ((e.key === 'Enter' || e.key === ' ') && c.sortable !== false) { e.preventDefault(); sortBy(c.id); }
      });
      grip.addEventListener('pointerdown', (e) => {
        if (e.button !== 0) return;
        e.preventDefault(); e.stopPropagation();
        grip.setPointerCapture(e.pointerId);
        const startX = e.clientX, startW = widthOf(c);
        root.classList.add('is-resizing');
        const move = (ev) => {
          state.widths[c.id] = Math.round(clamp(startW + ev.clientX - startX, c.minWidth || 48, 900));
          applyTemplate();
          resized = true;
        };
        const up = () => {
          grip.removeEventListener('pointermove', move);
          grip.removeEventListener('pointerup', up);
          grip.removeEventListener('pointercancel', up);
          root.classList.remove('is-resizing');
          persist();
          setTimeout(() => { resized = false; }, 0);
        };
        grip.addEventListener('pointermove', move);
        grip.addEventListener('pointerup', up);
        grip.addEventListener('pointercancel', up);
      });
      grip.addEventListener('dblclick', (e) => {
        e.stopPropagation();
        delete state.widths[c.id];
        applyTemplate(); persist();
      });
      head.append(th);
    }
    renderSortMarks();
    updateHeader();
  }

  let headerValues = {};
  function updateHeader() {
    for (const [id, { valEl }] of headerCells) {
      const v = headerValues[id] ?? '';
      if (valEl.textContent !== v) valEl.textContent = v;
      valEl.hidden = !v;
    }
    head.classList.toggle('has-values', Object.values(headerValues).some(Boolean));
  }

  function renderSortMarks() {
    for (const [id, { th, sortEl }] of headerCells) {
      const on = state.sort?.id === id;
      th.classList.toggle('is-sorted', on);
      th.setAttribute('aria-sort', on ? (state.sort.desc ? 'descending' : 'ascending') : 'none');
      clear(sortEl);
      if (on) sortEl.append(icon(state.sort.desc ? 'chevron-down' : 'chevron-up', { size: 10 }));
    }
  }

  function sortBy(id) {
    const c = columns.find((x) => x.id === id);
    if (!c) return;
    if (state.sort?.id === id) state.sort = { id, desc: !state.sort.desc };
    else state.sort = { id, desc: !!c.defaultDesc };
    persist();
    renderSortMarks();
    render();
  }

  bindContextMenu(head, () => ({
    items: [
      { header: t('columns') },
      ...columns.filter((c) => !c.required).map((c) => ({
        label: c.menuLabel || c.label, checked: !state.hidden.has(c.id),
        action: () => setHidden(c.id, !state.hidden.has(c.id)),
      })),
      { separator: true },
      { label: t('reset'), icon: 'refresh', action: () => { state.widths = {}; state.hidden = new Set(columns.filter((c) => c.hidden).map((c) => c.id)); persist(); rebuild(); } },
    ],
  }));

  function setHidden(id, hidden) {
    if (hidden) state.hidden.add(id); else state.hidden.delete(id);
    persist();
    rebuild();
  }

  /* ------------------------------------------------------------ sorting */
  function compare(a, b) {
    const s = state.sort;
    if (!s) return 0;
    const c = columns.find((x) => x.id === s.id);
    if (!c) return 0;
    const get = c.value || c.text || (() => '');
    const va = get(a), vb = get(b);
    const coll = collator(COLL_OPTS);
    let r;
    if (typeof va === 'number' || typeof vb === 'number') r = (Number(va) || 0) - (Number(vb) || 0);
    else r = coll.compare(String(va ?? ''), String(vb ?? ''));
    if (r === 0) {
      const first = columns[0];
      const fa = (first.value || first.text)(a), fb = (first.value || first.text)(b);
      return coll.compare(String(fa ?? ''), String(fb ?? ''));
    }
    return s.desc ? -r : r;
  }
  const sorted = (rows) => (state.sort ? [...rows].sort(compare) : rows);

  /* -------------------------------------------------------------- items */
  function buildItems() {
    const out = [];
    autoExpanded.clear();
    for (const g of groups) {
      const rowsOut = [];
      for (const row of sorted(g.rows || [])) {
        const key = rowKey(row);
        const kids = childrenOf ? (childrenOf(row) || []) : [];
        let showKids = kids;
        let match = !filter || filter(row);
        if (filter) {
          const km = kids.filter((k) => filter(k));
          if (!match && !km.length) continue;
          if (!match) { showKids = km; autoExpanded.add(key); }
        }
        const hasKids = kids.length > 0;
        rowsOut.push({ type: 'row', key, row, depth: 0, hasKids, count: kids.length });
        if (hasKids && (expanded.has(key) || autoExpanded.has(key))) {
          for (const kid of sorted(showKids)) rowsOut.push({ type: 'row', key: rowKey(kid), row: kid, depth: 1, parentKey: key, hasKids: false });
        }
      }
      const topCount = rowsOut.filter((i) => i.depth === 0).length;
      if (g.label && topCount) out.push({ type: 'group', key: `g:${g.id}`, label: g.label, count: topCount, group: g });
      out.push(...rowsOut);
    }
    return out;
  }

  /* ------------------------------------------------------------- render */
  function makeRowEl(item) {
    const el = h('div', { class: 'tm-row', role: 'row', tabindex: -1, dataset: { key: item.key } });
    const cells = visibleCols().map((c, i) => {
      const content = h('span', { class: 'tm-cell__content' });
      const cell = h('div', { class: ['tm-cell', `is-${c.align || 'left'}`, c.heat && 'has-heat', c.cellClass], role: 'gridcell' });
      if (i === 0 && childrenOf) {
        const chev = h('button', { type: 'button', class: 'tm-chev', tabindex: -1, 'aria-hidden': 'true' }, icon('chevron-right', { size: 12 }));
        chev.addEventListener('click', (e) => { e.stopPropagation(); toggleExpand(el.dataset.key); });
        cell.append(h('span', { class: 'tm-tree' }, chev));
      }
      cell.append(content);
      el.append(cell);
      return { cell, content, heat: -1, text: null, tip: null };
    });
    return { el, cells, cls: '' };
  }

  function updateRowEl(rec, item, isNew) {
    const { row } = item;
    rec.row = row;
    rec.item = item;
    const el = rec.el;
    if (el.dataset.depth !== String(item.depth)) {
      el.dataset.depth = String(item.depth);
      el.setAttribute('aria-level', String(item.depth + 1));
    }
    el.classList.toggle('is-child', item.depth > 0);
    el.classList.toggle('has-kids', !!item.hasKids);
    const isOpen = item.hasKids && (expanded.has(item.key) || autoExpanded.has(item.key));
    if (item.hasKids) el.setAttribute('aria-expanded', String(isOpen)); else el.removeAttribute('aria-expanded');
    el.classList.toggle('is-expanded', !!isOpen);
    const sel = selected.has(item.key);
    el.classList.toggle('is-selected', sel);
    el.setAttribute('aria-selected', String(sel));
    const extra = rowClass ? [].concat(rowClass(row) || []).filter(Boolean).join(' ') : '';
    if (extra !== rec.cls) {
      if (rec.cls) el.classList.remove(...rec.cls.split(' '));
      if (extra) el.classList.add(...extra.split(' '));
      rec.cls = extra;
    }
    visibleCols().forEach((c, i) => {
      const cc = rec.cells[i];
      if (!cc) return;
      if (c.render) c.render(cc.content, row, { isNew, depth: item.depth, item, cell: cc.cell });
      else {
        const txt = c.text ? String(c.text(row) ?? '') : '';
        if (cc.text !== txt) { cc.content.textContent = txt; cc.text = txt; }
      }
      if (c.heat) {
        const b = heatBucket(c.heat(row));
        if (b !== cc.heat) {
          if (cc.heat >= 0) cc.cell.classList.remove(`tm-heat-${cc.heat}`);
          cc.cell.classList.add(`tm-heat-${b}`);
          cc.heat = b;
        }
      }
      if (c.tip) {
        const tip = c.tip(row) || '';
        if (tip !== cc.tip) { if (tip) cc.cell.dataset.tooltip = tip; else delete cc.cell.dataset.tooltip; cc.tip = tip; }
      }
    });
  }

  function makeGroupEl(item) {
    const label = h('span', { class: 'tm-group__label' });
    const el = h('div', { class: 'tm-row tm-row--group', role: 'row', dataset: { key: item.key } },
      h('div', { class: 'tm-group', role: 'gridcell' }, label));
    return { el, label, text: '' };
  }

  // Time-sliced render: items are placed in order (nodes before `cur` are done); a newer render() replaces a
  // pending one. flush() finishes it synchronously for API calls that need every row element.
  const SLICE_MS = 8;
  let job = null;

  function placeItem(item) {
    if (item.type === 'group') {
      let g = groupCache.get(item.key);
      if (!g) { g = makeGroupEl(item); groupCache.set(item.key, g); }
      const txt = groupCounts ? `${item.label} (${item.count})` : item.label;
      if (g.text !== txt) { g.label.textContent = txt; g.text = txt; }
      return g.el;
    }
    let rec = rowCache.get(item.key);
    const isNew = !rec;
    if (!rec) { rec = makeRowEl(item); rowCache.set(item.key, rec); }
    updateRowEl(rec, item, isNew);
    return rec.el;
  }

  function render() {
    cancelJob();
    const active = document.activeElement;
    const hadFocus = root.contains(active);
    items = buildItems();
    const seen = new Set();
    for (const item of items) seen.add(item.key);
    // drop stale rows
    for (const [k, rec] of rowCache) if (!seen.has(k)) { rec.el.remove(); rowCache.delete(k); }
    for (const [k, g] of groupCache) if (!seen.has(k)) { g.el.remove(); groupCache.delete(k); }
    // prune selection of rows that disappeared
    let selChanged = false;
    for (const k of [...selected]) if (!seen.has(k)) { selected.delete(k); selChanged = true; }
    if (items.length) empty.remove();
    job = { items, i: 0, cur: body.firstChild, hadFocus, active, selChanged, handle: 0 };
    runSlice(job, false);
  }

  function runSlice(s, all) {
    const t0 = performance.now();
    let n = 0;
    while (s.i < s.items.length) {
      const el = placeItem(s.items[s.i++]);
      // reorder with minimal moves
      if (el === s.cur) s.cur = s.cur.nextSibling;
      else body.insertBefore(el, s.cur);
      if (!all && (++n & 15) === 0 && performance.now() - t0 > SLICE_MS) break;
    }
    if (s.i < s.items.length) { s.handle = requestAnimationFrame(() => { s.handle = 0; runSlice(s, false); }); return; }
    finishJob(s);
  }

  function finishJob(s) {
    if (job === s) job = null;
    let cur = s.cur;
    while (cur) { const next = cur.nextSibling; cur.remove(); cur = next; }
    if (!s.items.length) body.append(empty); else empty.remove();
    // roving tabindex + restore focus
    if (focusKey && !rowCache.has(focusKey)) focusKey = null;
    for (const [k, rec] of rowCache) rec.el.tabIndex = k === focusKey ? 0 : -1;
    const { active } = s;
    if (s.hadFocus && active && !root.contains(document.activeElement)) {
      const back = active.isConnected ? active : (focusKey && rowCache.get(focusKey)?.el);
      (back || root).focus({ preventScroll: true });
    }
    if (s.selChanged) fireSelection();
  }

  function cancelJob() {
    if (!job) return;
    if (job.handle) cancelAnimationFrame(job.handle);
    job = null;
  }

  /** Finish a pending time-sliced render now. */
  function flush() {
    const s = job;
    if (!s) return;
    if (s.handle) { cancelAnimationFrame(s.handle); s.handle = 0; }
    runSlice(s, true);
  }

  function rebuild() {
    cancelJob();
    for (const rec of rowCache.values()) rec.el.remove();
    rowCache.clear();
    applyTemplate();
    buildHeader();
    render();
  }

  /* ---------------------------------------------------------- selection */
  function rowItems() { return items.filter((i) => i.type === 'row'); }
  function fireSelection() { onSelectionChange?.(api.getSelection()); }

  function focusRow(key, { scroll = true } = {}) {
    if (!rowCache.has(key)) flush();
    const rec = rowCache.get(key);
    if (!rec) return;
    if (focusKey && rowCache.get(focusKey)) rowCache.get(focusKey).el.tabIndex = -1;
    focusKey = key;
    rec.el.tabIndex = 0;
    rec.el.focus({ preventScroll: true });
    if (scroll) scrollIntoView(rec.el);
  }

  function scrollIntoView(el) {
    const headH = head.offsetHeight;
    const top = el.offsetTop, bottom = top + el.offsetHeight;
    if (top - headH < root.scrollTop) root.scrollTop = top - headH;
    else if (bottom > root.scrollTop + root.clientHeight) root.scrollTop = bottom - root.clientHeight;
  }

  function setSelection(keys, { anchor, fire = true } = {}) {
    selected.clear();
    for (const k of keys) selected.add(k);
    if (anchor !== undefined) anchorKey = anchor;
    for (const [k, rec] of rowCache) {
      const on = selected.has(k);
      rec.el.classList.toggle('is-selected', on);
      rec.el.setAttribute('aria-selected', String(on));
    }
    if (fire) fireSelection();
  }

  function selectRange(toKey) {
    const list = rowItems().map((i) => i.key);
    const a = list.indexOf(anchorKey ?? toKey), b = list.indexOf(toKey);
    if (a < 0 || b < 0) { setSelection([toKey], { anchor: toKey }); return; }
    const [lo, hi] = a < b ? [a, b] : [b, a];
    setSelection(list.slice(lo, hi + 1));
  }

  function toggleExpand(key, open) {
    const want = open ?? !(expanded.has(key) || autoExpanded.has(key));
    if (want) expanded.add(key); else { expanded.delete(key); autoExpanded.delete(key); }
    render();
  }

  body.addEventListener('click', (e) => {
    const el = e.target.closest('.tm-row');
    if (!el || el.classList.contains('tm-row--group') || !body.contains(el)) return;
    const key = el.dataset.key;
    if (multiSelect && (e.ctrlKey || e.metaKey)) {
      const next = new Set(selected);
      if (next.has(key)) next.delete(key); else next.add(key);
      setSelection(next, { anchor: key });
    } else if (multiSelect && e.shiftKey) selectRange(key);
    else setSelection([key], { anchor: key });
    focusRow(key, { scroll: false });
  });
  body.addEventListener('dblclick', (e) => {
    const el = e.target.closest('.tm-row');
    if (!el || el.classList.contains('tm-row--group') || e.target.closest('.tm-chev, button, .fl-toggle')) return;
    const rec = rowCache.get(el.dataset.key);
    if (!rec) return;
    if (onActivate) onActivate(rec.row, rec.item);
    else if (rec.item.hasKids) toggleExpand(el.dataset.key);
  });

  root.addEventListener('focus', (e) => {
    if (e.target !== root) return;
    const list = rowItems();
    const key = focusKey && rowCache.has(focusKey) ? focusKey : ([...selected][0] || list[0]?.key);
    if (key) focusRow(key);
  });

  let typed = '', typedTimer = 0;
  root.addEventListener('keydown', (e) => {
    // keys typed inside a cell control (toggle, button) belong to that control
    if (e.target !== root && !e.target.classList?.contains('tm-row')) return;
    flush();
    const list = rowItems();
    if (!list.length) return;
    let i = list.findIndex((x) => x.key === focusKey);
    const cur = list[i];
    const go = (j) => {
      j = clamp(j, 0, list.length - 1);
      const key = list[j].key;
      if (multiSelect && e.shiftKey) { if (!anchorKey) anchorKey = focusKey || key; selectRange(key); }
      else if (!(multiSelect && e.ctrlKey)) setSelection([key], { anchor: key });
      focusRow(key);
    };
    const page = Math.max(1, Math.floor(root.clientHeight / rowHeight) - 2);
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); go(i < 0 ? 0 : i + 1); return;
      case 'ArrowUp': e.preventDefault(); go(i < 0 ? 0 : i - 1); return;
      case 'Home': e.preventDefault(); go(0); return;
      case 'End': e.preventDefault(); go(list.length - 1); return;
      case 'PageDown': e.preventDefault(); go((i < 0 ? 0 : i) + page); return;
      case 'PageUp': e.preventDefault(); go((i < 0 ? 0 : i) - page); return;
      case 'ArrowRight':
        if (!cur) return;
        e.preventDefault();
        if (cur.hasKids && !(expanded.has(cur.key) || autoExpanded.has(cur.key))) toggleExpand(cur.key, true);
        else if (cur.hasKids) go(i + 1);
        return;
      case 'ArrowLeft':
        if (!cur) return;
        e.preventDefault();
        if (cur.hasKids && (expanded.has(cur.key) || autoExpanded.has(cur.key))) toggleExpand(cur.key, false);
        else if (cur.parentKey) { const pi = list.findIndex((x) => x.key === cur.parentKey); if (pi >= 0) go(pi); }
        return;
      case 'Enter':
        if (!cur) return;
        e.preventDefault();
        if (onActivate) onActivate(cur.row, cur);
        else if (cur.hasKids) toggleExpand(cur.key);
        return;
      case 'Delete':
        if (!selected.size || !onDelete) return;
        e.preventDefault();
        onDelete(api.getSelection());
        return;
      case ' ':
        if (!cur) return;
        e.preventDefault();
        if (multiSelect && e.ctrlKey) {
          const next = new Set(selected);
          if (next.has(cur.key)) next.delete(cur.key); else next.add(cur.key);
          setSelection(next, { anchor: cur.key });
        } else setSelection([cur.key], { anchor: cur.key });
        return;
      case 'a':
        if (multiSelect && (e.ctrlKey || e.metaKey)) { e.preventDefault(); setSelection(list.map((x) => x.key)); return; }
        break;
      default:
    }
    // type-ahead on the first column
    if (e.key.length === 1 && !e.ctrlKey && !e.altKey && !e.metaKey && /\S/.test(e.key)) {
      clearTimeout(typedTimer);
      typed += e.key.toLowerCase();
      typedTimer = setTimeout(() => { typed = ''; }, 800);
      const first = columns[0];
      const textOf = (r) => String((first.searchText || first.text || first.value)(r) ?? '').toLowerCase();
      const start = typed.length === 1 ? i + 1 : Math.max(0, i);
      const order = [...list.slice(start), ...list.slice(0, start)];
      const hit = order.find((x) => x.depth === 0 && textOf(x.row).startsWith(typed));
      if (hit) { e.preventDefault(); go(list.indexOf(hit)); }
    }
  });

  if (contextMenu) {
    bindContextMenu(body, (e) => {
      let el = e.target.closest?.('.tm-row');
      if (e.type === 'keydown') el = document.activeElement?.closest?.('.tm-row') || (focusKey && rowCache.get(focusKey)?.el);
      if (!el || el.classList.contains('tm-row--group')) return null;
      const key = el.dataset.key;
      if (!selected.has(key)) setSelection([key], { anchor: key });
      focusRow(key, { scroll: false });
      return contextMenu(api.getSelection(), e);
    });
  }

  /* ----------------------------------------------------------------- api */
  const api = {
    el: root,
    columns,
    setGroups(g) { groups = g || []; render(); },
    setRows(rows) { groups = [{ id: 'all', label: '', rows: rows || [] }]; render(); },
    setFilter(fn) { filter = fn || null; render(); },
    setEmptyText(s) { empty.firstChild.textContent = s || ''; },
    refresh: render,
    rebuild,
    setHeaderValues(v) { headerValues = v || {}; updateHeader(); },
    getSelection() {
      const out = [];
      for (const k of selected) { const rec = rowCache.get(k); if (rec) out.push(rec.row); }
      return out;
    },
    getSelectedItems() {
      const out = [];
      for (const k of selected) { const rec = rowCache.get(k); if (rec) out.push(rec.item); }
      return out;
    },
    select(key, { focus = false, scroll = true } = {}) {
      if (!rowCache.has(key)) flush();
      if (!rowCache.has(key)) return false;
      setSelection([key], { anchor: key });
      if (focus) focusRow(key, { scroll });
      else if (scroll) scrollIntoView(rowCache.get(key).el);
      return true;
    },
    clearSelection() { setSelection([]); },
    expand(key, open = true) { toggleExpand(key, open); },
    expandAll(open = true) {
      if (open) for (const it of items) { if (it.hasKids) expanded.add(it.key); }
      else expanded.clear();
      render();
    },
    isHidden: (id) => state.hidden.has(id),
    setColumnHidden: setHidden,
    get sort() { return state.sort; },
    /** sortBy(id, desc) — programmatic sort (e.g. from a health action). */
    sortBy(id, desc) {
      if (!columns.some((c) => c.id === id)) return;
      state.sort = { id, desc: !!desc };
      persist(); renderSortMarks(); render();
    },
    focus() { root.focus({ preventScroll: true }); },
    rowEl: (key) => { if (!rowCache.has(key)) flush(); return rowCache.get(key)?.el || null; },
    /** Finish a pending (time-sliced) render now. */
    flush,
    get pending() { return !!job; },
    get items() { return items; },
  };

  applyTemplate();
  buildHeader();
  render();
  return api;
}
