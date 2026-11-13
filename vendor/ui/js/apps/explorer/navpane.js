// Win11 Shell — Verkenner: navigation pane (Start, Galerij, pinned folders, Deze pc ▸ drives, Prullenbak).
// Tree with lazy sub-folders, chevrons on hover, accent pill on the selected item, drop targets.

import { h, clear } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bindContextMenu } from '../../core/contextmenu.js';
import { t } from './i18n.js';
import { isFileDrag, dropPaths, showHint, hideHint, endDrag } from './dnd.js';

export function NavPane(app) {
  const tree = h('div', { class: 'xp-tree', role: 'tree', 'aria-label': t('navPane') });
  const el = h('nav', { class: 'xp-nav' }, tree);
  const expanded = new Set(['thispc']);
  const kids = new Map();      // folder path → [{name, path}] | 'loading'
  let rows = [];               // visible rows in order: {node, el}

  function topNodes() {
    const list = [
      { id: 'home', loc: { kind: 'home' }, label: t('home'), glyph: 'home' },
      { id: 'gallery', loc: { kind: 'gallery' }, label: t('gallery'), glyph: 'image', glyphCls: 'is-gallery' },
      { sep: true },
    ];
    for (const p of app.pinnedList()) {
      list.push({ id: `pin:${p.path}`, loc: { kind: 'folder', path: p.path }, label: p.label, iconName: p.iconName, pinned: true, expandable: true });
    }
    list.push({ sep: true });
    list.push({
      id: 'thispc', loc: { kind: 'thispc' }, label: t('thisPC'), iconName: 'computer', glyph: 'pc', expandable: true,
      children: () => app.S.volumes.map((v) => ({
        id: `vol:${v.mount}`, loc: { kind: 'folder', path: v.mount }, label: app.volLabel(v), iconName: app.volIcon(v), glyph: 'drive', expandable: true,
      })),
    });
    if (app.S.places.trash) {
      list.push({ id: 'trash', loc: { kind: 'trash' }, label: t('trash'), iconName: app.S.trashFull ? 'user-trash-full' : 'user-trash', glyph: 'trash' });
    }
    return list;
  }

  function folderChildren(node) {
    const path = node.loc.path;
    const c = kids.get(path);
    if (!c || c === 'loading') return c === 'loading' ? 'loading' : null;
    return c.map((d) => ({ id: `${node.id}>${d.path}`, loc: { kind: 'folder', path: d.path }, label: d.name, iconName: d.icon || 'folder', glyph: 'folder', expandable: true }));
  }

  async function loadKids(node) {
    const path = node.loc.path;
    kids.set(path, 'loading');
    render();
    try { kids.set(path, await app.listDirs(path)); } catch { kids.set(path, []); }
    render();
  }

  function toggle(node, open = !expanded.has(node.id)) {
    if (!node.expandable) return;
    if (open) {
      expanded.add(node.id);
      if (node.loc.kind === 'folder') loadKids(node);
      else if (node.id === 'thispc') app.loadVolumes?.().then(render);
    } else expanded.delete(node.id);
    render();
  }

  function rowEl(node, level) {
    const isOpen = expanded.has(node.id);
    const childList = isOpen ? (node.children ? node.children() : folderChildren(node)) : null;
    const hasKids = node.expandable && !(isOpen && Array.isArray(childList) && childList.length === 0);
    const chev = h('span', { class: ['xp-node__chev', !hasKids && 'is-none'], 'aria-hidden': 'true' },
      hasKids ? icon(isOpen ? 'chevron-down' : 'chevron-right', { size: 12 }) : null);
    const ic = node.iconName ? app.namedIcon(node.iconName, 16, node.glyph || 'folder')
      : h('span', { class: ['xp-node__glyph', node.glyphCls] }, icon(node.glyph || 'folder', { size: 16 }));
    const row = h('div', {
      class: ['xp-node', node.pinned && 'is-pinned'], role: 'treeitem', tabindex: -1,
      'aria-level': level + 1, 'aria-expanded': node.expandable ? String(isOpen) : null, 'aria-selected': 'false',
      style: { '--lvl': level }, dataset: { id: node.id },
    }, chev, h('span', { class: 'xp-node__icon' }, ic), h('span', { class: 'xp-node__label' }, node.label),
    node.pinned ? h('span', { class: 'xp-node__pin', 'aria-label': t('pinned') }, icon('pin', { size: 12 })) : null);
    chev.addEventListener('pointerdown', (e) => { e.preventDefault(); e.stopPropagation(); toggle(node); });
    row.addEventListener('click', (e) => {
      if (e.target.closest('.xp-node__chev')) return;
      app.navigate(node.loc);
    });
    row.addEventListener('auxclick', (e) => { if (e.button === 1) { e.preventDefault(); app.openTab(node.loc, { activate: false }); } });
    row.addEventListener('mousedown', (e) => { if (e.button === 1) e.preventDefault(); });
    bindContextMenu(row, () => app.menus.navMenu(node));
    wireDrop(row, node);
    rows.push({ node, el: row });
    const frag = [row];
    if (isOpen && childList === 'loading') {
      frag.push(h('div', { class: 'xp-node is-loading', style: { '--lvl': level + 1 }, 'aria-hidden': 'true' },
        h('span', { class: 'xp-node__chev is-none' }), h('span', { class: 'xp-node__label' }, t('loading'))));
    } else if (isOpen && Array.isArray(childList) && childList.length) {
      const group = h('div', { role: 'group' });
      for (const c of childList) group.append(...rowEl(c, level + 1));
      frag.push(group);
    }
    return frag;
  }

  function wireDrop(row, node) {
    const dest = node.loc.kind === 'folder' ? node.loc.path : null;
    const isTrash = node.loc.kind === 'trash';
    if (!dest && !isTrash) return;
    let hover = 0;
    const info = (e) => (isTrash ? (app.dnd.current ? { label: t('trashTo'), copy: false, trash: true } : null) : app.dropInfo(dest, e));
    row.addEventListener('dragover', (e) => {
      if (!isFileDrag(e)) return;
      const i = info(e);
      if (!i) { e.dataTransfer.dropEffect = 'none'; return; }
      e.preventDefault();
      e.dataTransfer.dropEffect = i.copy ? 'copy' : 'move';
      row.classList.add('is-drop');
      showHint(e.clientX, e.clientY, i.label, i.trash ? 'trash' : (i.copy ? 'copy' : 'move'));
      if (!hover && node.expandable && !expanded.has(node.id)) hover = setTimeout(() => toggle(node, true), 800);
    });
    row.addEventListener('dragleave', (e) => {
      if (row.contains(e.relatedTarget)) return;
      row.classList.remove('is-drop'); clearTimeout(hover); hover = 0;
    });
    row.addEventListener('drop', (e) => {
      row.classList.remove('is-drop'); clearTimeout(hover); hover = 0;
      hideHint();
      const i = info(e);
      if (!i) return;
      e.preventDefault();
      const { paths, external } = dropPaths(e);
      endDrag();
      if (!paths.length) return;
      if (i.trash) app.act.trashPaths(paths);
      else app.dropTo(paths, dest, i.copy || external);
    });
  }

  function render() {
    const focusedId = document.activeElement?.closest?.('.xp-node')?.dataset.id;
    const scroll = el.scrollTop;
    rows = [];
    clear(tree);
    for (const n of topNodes()) {
      if (n.sep) { tree.append(h('div', { class: 'xp-tree__sep', role: 'separator' })); continue; }
      tree.append(...rowEl(n, 0));
    }
    markSelected();
    el.scrollTop = scroll;
    if (focusedId) rows.find((r) => r.node.id === focusedId)?.el.focus({ preventScroll: true });
  }

  function markSelected() {
    const loc = app.tab?.loc;
    const key = loc ? app.locKey(loc) : '';
    let found = null;
    for (const r of rows) {
      const on = !found && app.locKey(r.node.loc) === key;
      if (on) found = r;
      r.el.classList.toggle('is-selected', on);
      r.el.setAttribute('aria-selected', String(on));
    }
    for (const r of rows) r.el.tabIndex = -1;
    (found || rows[0])?.el && ((found || rows[0]).el.tabIndex = 0);
  }

  // keyboard: Win11 TreeView
  tree.addEventListener('keydown', (e) => {
    const cur = rows.findIndex((r) => r.el === document.activeElement);
    if (cur < 0) return;
    const r = rows[cur];
    const focusRow = (i) => { if (rows[i]) { rows.forEach((x) => { x.el.tabIndex = -1; }); rows[i].el.tabIndex = 0; rows[i].el.focus(); } };
    switch (e.key) {
      case 'ArrowDown': e.preventDefault(); focusRow(Math.min(rows.length - 1, cur + 1)); break;
      case 'ArrowUp': e.preventDefault(); focusRow(Math.max(0, cur - 1)); break;
      case 'Home': e.preventDefault(); focusRow(0); break;
      case 'End': e.preventDefault(); focusRow(rows.length - 1); break;
      case 'ArrowRight':
        e.preventDefault();
        if (r.node.expandable && !expanded.has(r.node.id)) toggle(r.node, true);
        else focusRow(cur + 1);
        break;
      case 'ArrowLeft': {
        e.preventDefault();
        if (expanded.has(r.node.id)) { toggle(r.node, false); break; }
        const lvl = Number(r.el.getAttribute('aria-level'));
        for (let i = cur - 1; i >= 0; i--) if (Number(rows[i].el.getAttribute('aria-level')) < lvl) { focusRow(i); break; }
        break;
      }
      case 'Enter': case ' ':
        e.preventDefault(); app.navigate(r.node.loc); break;
      default:
    }
  });

  return {
    el, render, markSelected,
    invalidate(path) { if (path) kids.delete(path); else kids.clear(); for (const id of expanded) { const n = rows.find((x) => x.node.id === id)?.node; if (n?.loc.kind === 'folder' && (!path || n.loc.path === path)) loadKids(n); } },
  };
}
