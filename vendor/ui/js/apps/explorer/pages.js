// Win11 Shell — Verkenner: the special pages Start (Snelle toegang · Favorieten · Recent), Deze pc and Galerij.

import { h, clear } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bindContextMenu } from '../../core/contextmenu.js';
import { ProgressRing, Button } from '../../core/components.js';
import { bridge } from '../../core/bridge.js';
import { bytes, date } from '../../core/format.js';
import { t } from './i18n.js';
import { fileIcon, dateTime, dirname, basename, fileUri, volumePercent } from './util.js';
import { isFileDrag, dropPaths, showHint, hideHint, endDrag, startDrag, DND_MIME, dragImage } from './dnd.js';

/* ------------------------------------------------------------ shared bits */

function section(app, id, title, count, body) {
  const key = `sec:${id}`;
  const collapsed = app.isCollapsed(key);
  const head = h('button', { type: 'button', class: 'xp-sec__head', 'aria-expanded': String(!collapsed) },
    h('span', { class: 'xp-sec__chev' }, icon(collapsed ? 'chevron-right' : 'chevron-down', { size: 12 })),
    h('span', { class: 'xp-sec__title' }, title),
    count != null ? h('span', { class: 'xp-sec__count' }, `(${count})`) : null);
  const content = h('div', { class: 'xp-sec__body', hidden: collapsed }, body);
  head.addEventListener('click', () => {
    const now = !content.hidden;
    content.hidden = now;
    head.setAttribute('aria-expanded', String(!now));
    head.querySelector('.xp-sec__chev').replaceChildren(icon(now ? 'chevron-right' : 'chevron-down', { size: 12 }));
    app.setCollapsed(key, now);
  });
  return h('section', { class: 'xp-sec', dataset: { id } }, head, content);
}

/** Make an element behave like a selectable Explorer item (click, Ctrl, double click, Enter, menu, drag & drop). */
function selectable(app, el, item, { drop = item.isDir, drag = true } = {}) {
  el.tabIndex = -1;
  el.dataset.path = item.path;
  el.classList.add('xp-pitem');
  app.registerPageItem(item);
  el.addEventListener('pointerdown', (e) => {
    if (e.button === 1 && item.isDir) { e.preventDefault(); app.openTab({ kind: 'folder', path: item.path }, { activate: false }); return; }
    if (e.button !== 0 && e.button !== 2) return;
    const sel = app.tab.selection;
    if (e.button === 0 && (e.ctrlKey || e.metaKey)) {
      const next = new Set(sel);
      if (next.has(item.path)) next.delete(item.path); else next.add(item.path);
      app.setSelection([...next], item.path);
    } else if (!sel.has(item.path)) app.setSelection([item.path], item.path);
    el.focus({ preventScroll: true });
  });
  el.addEventListener('dblclick', () => app.openItem(item));
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') { e.preventDefault(); app.openItem(item); return; }
    const dirs = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 };
    if (dirs[e.key]) {
      e.preventDefault();
      const all = [...el.closest('.xp-page').querySelectorAll('.xp-pitem')].filter((x) => x.offsetParent);
      const i = all.indexOf(el);
      let j = i + dirs[e.key];
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        // geometric: same column, next/previous row
        const r0 = el.getBoundingClientRect();
        const cands = all.filter((x) => { const r = x.getBoundingClientRect(); return e.key === 'ArrowDown' ? r.top > r0.top + 4 : r.top < r0.top - 4; });
        cands.sort((a, b) => {
          const ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect();
          const dya = Math.abs(ra.top - r0.top), dyb = Math.abs(rb.top - r0.top);
          return Math.round(dya - dyb) || Math.abs(ra.left - r0.left) - Math.abs(rb.left - r0.left);
        });
        j = cands[0] ? all.indexOf(cands[0]) : i;
      }
      const target = all[Math.max(0, Math.min(all.length - 1, j))];
      if (target) { app.setSelection([target.dataset.path], target.dataset.path); target.focus(); target.scrollIntoView({ block: 'nearest' }); }
    }
  });
  el.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); app.showMenuAt(e, app.menus.itemMenu(app.selectedItems())); });
  el.addEventListener('keydown', (e) => {
    if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) { e.preventDefault(); app.showMenuAt(null, app.menus.itemMenu(app.selectedItems())); }
  });
  if (drag) {
    el.draggable = true;
    el.addEventListener('dragstart', (e) => {
      if (!app.tab.selection.has(item.path)) app.setSelection([item.path], item.path);
      const paths = app.selectedItems().map((x) => x.path);
      try {
        e.dataTransfer.setData(DND_MIME, JSON.stringify(paths));
        e.dataTransfer.setData('text/uri-list', paths.map(fileUri).join('\r\n'));
        e.dataTransfer.setData('text/plain', paths.join('\n'));
        e.dataTransfer.effectAllowed = 'copyMove';
      } catch { /* ignore */ }
      dragImage(e, fileIcon(item, 64, { thumb: true }), paths.length);
      startDrag(paths, dirname(item.path));
    });
  }
  if (drop) {
    el.addEventListener('dragover', (e) => {
      if (!isFileDrag(e)) return;
      const info = app.dropInfo(item.path, e);
      if (!info) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = info.copy ? 'copy' : 'move';
      el.classList.add('is-drop');
      showHint(e.clientX, e.clientY, info.label, info.copy ? 'copy' : 'move');
    });
    el.addEventListener('dragleave', (e) => { if (!el.contains(e.relatedTarget)) el.classList.remove('is-drop'); });
    el.addEventListener('drop', (e) => {
      el.classList.remove('is-drop');
      hideHint();
      const info = app.dropInfo(item.path, e);
      if (!info) return;
      e.preventDefault();
      const { paths, external } = dropPaths(e);
      endDrag();
      if (paths.length) app.dropTo(paths, item.path, info.copy || external);
    });
  }
  return el;
}

function pageRoot(app, ...children) {
  const page = h('div', { class: 'xp-page', tabindex: -1 }, ...children);
  page.addEventListener('pointerdown', (e) => {
    if (e.button === 0 && !e.target.closest('.xp-pitem, button, a, input') && app.tab.selection.size) app.setSelection([], null);
  });
  bindContextMenu(page, (e) => (e.target.closest?.('.xp-pitem') ? null : app.menus.backgroundMenu()));
  return page;
}

/* ------------------------------------------------------------ folder tile */

function folderTile(app, f, sub) {
  const tile = h('div', { class: 'xp-ftile', role: 'listitem', title: f.path },
    h('span', { class: 'xp-ftile__icon' }, app.namedIcon(f.iconName || 'folder', 48, 'folder')),
    h('span', { class: 'xp-ftile__text' },
      h('span', { class: 'xp-ftile__name' }, f.label),
      h('span', { class: 'xp-ftile__sub' }, sub,
        f.pinned ? h('span', { class: 'xp-ftile__pin', 'aria-label': t('pinned') }, icon('pin', { size: 12 })) : null)));
  return selectable(app, tile, { path: f.path, name: f.label, displayName: f.label, isDir: true, icon: f.iconName, pinned: f.pinned });
}

/* ================================================================== Start */

export function HomePage(app) {
  const pins = app.pinnedList();
  const qaGrid = h('div', { class: 'xp-ftiles', role: 'list' }, pins.map((p) => folderTile(app, { ...p, pinned: true }, app.storedOn(p.path))));

  const favBody = h('div', { class: 'xp-reclist' });
  const recBody = h('div', { class: 'xp-reclist' }, h('div', { class: 'xp-page__loading' }, ProgressRing({ size: 24 })));

  const page = pageRoot(app,
    section(app, 'qa', t('quickAccess'), null, qaGrid),
    section(app, 'fav', t('favorites'), null, favBody),
    section(app, 'recent', t('recent'), null, recBody));

  function recTable(items, emptyText, opened) {
    if (!items.length) return h('div', { class: 'xp-page__empty' }, emptyText);
    const head = h('div', { class: 'xp-rec xp-rec--head', role: 'row' },
      h('span', { class: 'xp-rec__cell' }, t('colName')),
      h('span', { class: 'xp-rec__cell' }, opened ? t('colOpened') : t('colModified')),
      h('span', { class: 'xp-rec__cell' }, t('colLocation')));
    const rows = items.map((it) => {
      const item = { path: it.path, name: it.name || basename(it.path), isDir: !!it.isDir, mime: it.mime, icon: it.icon, modified: it.modified, size: it.size };
      const row = h('div', { class: 'xp-rec', role: 'row', title: it.path },
        h('span', { class: 'xp-rec__cell xp-rec__name' }, fileIcon(item, 16), h('span', { class: 'xp-rec__text' }, app.itemName(item))),
        h('span', { class: 'xp-rec__cell xp-rec__dim' }, dateTime(it.modified)),
        h('span', { class: 'xp-rec__cell xp-rec__dim' }, app.friendlyDir(dirname(it.path))));
      return selectable(app, row, item, { drop: false });
    });
    return h('div', { class: 'xp-rectable', role: 'grid' }, head, rows);
  }

  // favourites (stored in settings)
  const favs = app.cfg('favorites', []);
  favBody.append(recTable(favs.map((p) => ({ path: p, name: basename(p) })), t('favEmpty'), false));
  if (favs.length) {
    Promise.all(favs.map((p) => app.call('files.properties', { path: p }).then((pr) => ({ path: p, name: pr.name || basename(p), mime: pr.mime, modified: pr.modified, isDir: pr.type === 'folder' || pr.contains != null })).catch(() => null)))
      .then((list) => { if (!page.isConnected) return; clear(favBody).append(recTable(list.filter(Boolean), t('favEmpty'), false)); app.repaintSelection(); });
  }

  // recent files
  (async () => {
    const enabled = app.privacyRecent();
    let list = [];
    if (enabled) {
      try { list = await app.call('apps.recent', { limit: 20 }); } catch { list = []; }
    }
    if (!page.isConnected) return;
    clear(recBody).append(recTable(Array.isArray(list) ? list : [], enabled ? t('recentEmpty') : t('recentOff'), true));
    app.repaintSelection();
  })();

  return page;
}

/* ================================================================ Deze pc */

export function ThisPCPage(app) {
  const folders = app.defaultFolders();
  const fGrid = h('div', { class: 'xp-ftiles', role: 'list' }, folders.map((f) => folderTile(app, f, app.storedOn(f.path))));
  const dGrid = h('div', { class: 'xp-dtiles', role: 'list' });
  const devSec = section(app, 'devices', t('devices'), null, dGrid);
  const page = pageRoot(app, section(app, 'folders', t('folders'), folders.length, fGrid), devSec);

  function renderDrives() {
    clear(dGrid);
    const vols = app.S.volumes;
    devSec.querySelector('.xp-sec__count')?.remove();
    devSec.querySelector('.xp-sec__title').after(h('span', { class: 'xp-sec__count' }, `(${vols.length})`));
    if (!vols.length) { dGrid.append(h('div', { class: 'xp-page__empty' }, t('noDrives'))); return; }
    for (const v of vols) {
      const pct = volumePercent(v);
      const full = pct >= 90;
      const tile = h('div', { class: ['xp-dtile', full && 'is-full'], role: 'listitem', title: v.mount },
        h('span', { class: 'xp-dtile__icon' }, app.namedIcon(app.volIcon(v), 48, 'drive')),
        h('span', { class: 'xp-dtile__text' },
          h('span', { class: 'xp-dtile__name' }, app.volLabel(v)),
          h('span', { class: 'xp-dtile__bar', role: 'meter', 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': Math.round(pct), 'aria-label': full ? t('almostFull') : null },
            h('span', { class: 'xp-dtile__fill', style: { width: `${pct.toFixed(1)}%` } })),
          h('span', { class: 'xp-dtile__sub' }, t('freeOf', { free: bytes(v.free), total: bytes(v.total) }))));
      selectable(app, tile, { path: v.mount, name: app.volLabel(v), displayName: app.volLabel(v), isDir: true, volume: v, icon: app.volIcon(v) }, { drag: false });
      dGrid.append(tile);
    }
    app.repaintSelection();
  }
  renderDrives();
  app.loadVolumes().then(() => { if (page.isConnected) renderDrives(); });
  return page;
}

/* ================================================================ Galerij */

const MAX_PHOTOS = 600;

export function GalleryPage(app) {
  const body = h('div', { class: 'xp-gallery' }, h('div', { class: 'xp-page__loading' }, ProgressRing({ size: 32 })));
  const page = pageRoot(app, body);
  page.classList.add('xp-page--gallery');
  const root = app.S.places.pictures?.path;

  (async () => {
    const photos = root ? await app.collectImages(root, MAX_PHOTOS + 1) : [];
    if (!page.isConnected) return;
    clear(body);
    if (!photos.length) {
      body.append(h('div', { class: 'xp-bigempty' },
        h('span', { class: 'xp-bigempty__icon' }, icon('image', { size: 40 })),
        h('div', { class: 'subtitle' }, t('galleryEmpty')),
        h('div', { class: 'text-2' }, t('galleryEmptyMsg')),
        root ? Button({ label: t('openFolder'), onClick: () => app.navigate({ kind: 'folder', path: root }) }) : null));
      return;
    }
    const shown = photos.slice(0, MAX_PHOTOS);
    let month = '';
    let row = null;
    for (const p of shown) {
      const m = date(p.modified, 'month');
      if (m !== month) {
        month = m;
        body.append(h('h3', { class: 'xp-gallery__month' }, m.charAt(0).toUpperCase() + m.slice(1)));
        row = h('div', { class: 'xp-gallery__grid', role: 'list' });
        body.append(row);
      }
      const img = h('img', { src: bridge.thumbUrl(p.path, 360), alt: '', loading: 'lazy', decoding: 'async', draggable: false });
      const tile = h('div', { class: 'xp-photo', role: 'listitem', title: p.name, style: { '--ar': '1.5' } }, img);
      img.addEventListener('load', () => { if (img.naturalWidth && img.naturalHeight) tile.style.setProperty('--ar', (img.naturalWidth / img.naturalHeight).toFixed(3)); });
      img.addEventListener('error', () => { tile.classList.add('is-broken'); img.replaceWith(fileIcon(p, 48)); });
      selectable(app, tile, p, { drop: false });
      row.append(tile);
    }
    if (photos.length > MAX_PHOTOS) body.append(h('div', { class: 'xp-page__empty' }, t('galleryMore', { n: MAX_PHOTOS })));
    app.repaintSelection();
  })();
  return page;
}

