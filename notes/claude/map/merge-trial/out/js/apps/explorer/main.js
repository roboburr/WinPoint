// Win11 Shell — Verkenner (File Explorer). SPEC §11 entry: export async function mountApp(root, params).
//
// Layout (Win11 24H2): tab row in the title bar · address row (← → ↑ ⟳ · breadcrumb · search) ·
// command bar · navigation pane | content | details pane · status bar.
// Deep links: params.path (folder or file → selects it), params.place ('home'|'thispc'|'trash'|'gallery'|
// 'desktop'|'documents'|'downloads'|'music'|'pictures'|'videos'), params.select (one or more names / paths /
// file:// URIs: selected and scrolled into view), params.tabs ([{path, select?} | path]: more folders, one tab each),
// params.properties (true: "Eigenschappen" for the selection). Arrays arrive JSON-encoded in the URL of a new window.
// app.navigate opens a new tab. Other apps reach this through the desktop entry (%U) and org.freedesktop.FileManager1
// (host/explorer_args.py builds these params).
// Every file operation goes through files.* (trash, never permanent delete) with friendly Dutch errors.

import { createAppFrame, windowCommand } from '../appframe.js';
import { h, clear, loadCss, debounce, animate, clamp } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { getSetting, setSetting, onSettings, getShellInfo } from '../../core/theme.js';
import { InfoBar, Toast, Dialog, Button, ProgressRing, ProgressBar, HyperlinkButton } from '../../core/components.js';
import { isContextMenuOpen, showContextMenu } from '../../core/contextmenu.js';
import { bytes } from '../../core/format.js';
import { onLangChange } from '../../core/i18n.js';
import { t } from './i18n.js';
import * as U from './util.js';
import { FileView, VIEW_MODES } from './fileview.js';
import { TabStrip } from './tabs.js';
import { normalizeTabState } from './tabtransfer.js';
import { AddressRow, CommandBar } from './toolbar.js';
import { NavPane } from './navpane.js';
import { HomePage, ThisPCPage, GalleryPage } from './pages.js';
import { DetailsPane } from './detailspane.js';
import { showProperties, showDriveProperties, showTrashProperties, showShare, showOptions } from './dialogs.js';
import { dnd } from './dnd.js';

const PLACE_IDS = ['desktop', 'downloads', 'documents', 'pictures', 'music', 'videos'];
const PC_FOLDERS = ['desktop', 'documents', 'downloads', 'music', 'pictures', 'videos'];
const PLACE_ICON = {
  home: 'user-home', desktop: 'user-desktop', documents: 'folder-documents', downloads: 'folder-download',
  music: 'folder-music', pictures: 'folder-pictures', videos: 'folder-videos', trash: 'user-trash',
};
const ZOOM_ORDER = ['details', 'list', 'small', 'medium', 'large', 'xl'];
const IMG_RE = /\.(png|jpe?g|gif|webp|bmp|svg|avif|heic|tiff?)$/i;
// the tabs over a language-switch reload (saveForReload / restoreAfterReload)
const RELOAD_KEY = 'win11shell.explorer.langReload';
const RELOAD_MAX_AGE = 15000;

// clipboard-ordering: begin
/** One watermark covers HTTP responses and queued SSE updates in a standalone window. */
export function createClipboardGate() {
  let newestRevision = null;
  return (value) => {
    const raw = value?._revision;
    const versioned = Number.isSafeInteger(raw) && raw >= 0;
    if (raw !== undefined && !versioned) return null;
    if (versioned) {
      if (newestRevision !== null && raw < newestRevision) return null;
      newestRevision = raw;
    } else if (newestRevision !== null) {
      // A compatibility-server event must not replace Rust's session-wide state.
      return null;
    }
    return { paths: Array.isArray(value?.paths) ? value.paths.slice() : [], op: value?.op || null };
  };
}
// clipboard-ordering: end

export async function mountApp(root, params = {}) {
  await loadCss('explorer');
  let picker = params.picker || null;
  if (typeof picker === 'string') { try { picker = JSON.parse(picker); } catch { picker = null; } }
  const app = createExplorer(picker);
  root.append(app.frame.el);
  if (picker && typeof picker === 'object') { const {attachPicker} = await import('./picker.js'); await attachPicker(app, picker); }
  await app.start(params);
  return app.frame;
}

function createExplorer(picker = null) {
  const S = { places: {}, home: '', user: {}, volumes: [], clipboard: { paths: [], op: null }, trashFull: false };
  const acceptClipboard = createClipboardGate();
  const tabs = [];
  let activeId = null;
  let tabSeq = 0;
  let view = null;             // current FileView (folder / trash / search)
  const pageItems = new Map(); // items shown on Start / Deze pc / Galerij
  const collapsedSecs = new Set();
  const groupCollapse = new Map();
  const watched = new Set();
  const undoStack = [];
  const dirCache = new Map();

  const call = (method, args = {}) => bridge.call(method, args);
  const cfg = (key, def) => getSetting(`explorer.${key}`, def);
  const setCfg = (key, val) => setSetting(`explorer.${key}`, val).catch((e) => console.warn('[explorer] setting failed', key, e));

  /* =============================================================== frame */
  const app = {};
  const tabStrip = TabStrip(app);
  const frame = createAppFrame({ app: 'explorer', title: t('appName'), icon: null, titlebarContent: tabStrip.el });
  frame.el.classList.add('xp-window');
  const addressRow = AddressRow(app);
  const cmdBar = CommandBar(app);
  const navPane = NavPane(app);
  const details = DetailsPane(app);
  const infobars = h('div', { class: 'xp-infobars', 'aria-live': 'polite' });
  const contentHost = h('div', { class: 'xp-main' });
  const splitter = h('div', { class: 'xp-splitter', role: 'separator', tabindex: 0, 'aria-orientation': 'vertical', 'aria-label': t('splitter') });
  const center = h('div', { class: 'xp-center' }, infobars, contentHost);
  const body = h('div', { class: 'xp-body' }, navPane.el, splitter, center, details.el);
  const statusLeft = h('div', { class: 'xp-status__text', 'aria-live': 'polite' });
  const stDetails = h('button', { type: 'button', class: 'xp-status__btn', 'aria-label': t('statusDetails'), dataset: { tooltip: t('statusDetails') } }, icon('list', { size: 14 }));
  const stLarge = h('button', { type: 'button', class: 'xp-status__btn', 'aria-label': t('statusLarge'), dataset: { tooltip: t('statusLarge') } }, icon('grid', { size: 14 }));
  stDetails.addEventListener('click', () => app.act.setView('details'));
  stLarge.addEventListener('click', () => app.act.setView('large'));
  const status = h('footer', { class: 'xp-status' }, statusLeft, h('div', { class: 'xp-status__btns' }, stDetails, stLarge));
  const rootEl = h('div', { class: 'xp' }, h('div', { class: 'xp-toolbar' }, addressRow.el, cmdBar.el), body, status);
  frame.setContent(rootEl);
  details.el.hidden = true; // until the settings are applied in start()

  /* ============================================================ helpers */
  const tabOf = (id) => tabs.find((x) => x.id === id);
  const active = () => tabOf(activeId);
  const placeById = (id) => S.places[id];
  const placeForPath = (p) => Object.values(S.places).find((pl) => pl.path && pl.path === p && pl.id !== 'root');
  const volFor = (p) => U.volumeFor(p, S.volumes);
  const userLabel = () => S.user.realName || S.user.name || U.basename(S.home) || t('homeFolder');

  function locKey(loc) {
    if (!loc) return '';
    if (loc.kind === 'folder') return `folder:${loc.path}`;
    if (loc.kind === 'search') return `search:${loc.path}:${loc.query}`;
    return loc.kind;
  }

  function folderLabel(p) {
    const pl = placeForPath(p);
    if (pl) return pl.id === 'home' ? userLabel() : (PLACE_IDS.includes(pl.id) ? t(pl.id) : (pl.name || U.basename(p)));
    const v = S.volumes.find((x) => x.mount === p);
    if (v) return U.volumeLabel(v);
    if (p === '/') return `${t('root')} (/)`;
    return U.basename(p);
  }

  function locLabel(loc, { full = false } = {}) {
    if (!loc) return '';
    switch (loc.kind) {
      case 'home': return t('home');
      case 'gallery': return t('gallery');
      case 'thispc': return t('thisPC');
      case 'trash': return t('trash');
      case 'search': return t('searchResults', { name: folderLabel(loc.path) });
      default: return full ? loc.path : folderLabel(loc.path);
    }
  }

  function iconNameFor(p) {
    const pl = placeForPath(p);
    if (pl) return pl.icon && !pl.icon.endsWith('-symbolic') ? pl.icon : (PLACE_ICON[pl.id] || 'folder');
    const v = S.volumes.find((x) => x.mount === p);
    if (v) return U.volumeIcon(v);
    if (p === '/') return 'drive-harddisk-system';
    return 'folder';
  }

  function glyphBox(name, size, cls) {
    return h('span', { class: ['xp-glyphicon', cls], style: { width: `${size}px`, height: `${size}px` } }, icon(name, { size: Math.round(size * (size > 32 ? 0.7 : 1)) }));
  }

  function locIcon(loc, size = 16) {
    switch (loc?.kind) {
      case 'home': return glyphBox('home', size, 'is-home');
      case 'gallery': return glyphBox('image', size, 'is-gallery');
      case 'thispc': return U.namedIcon('computer', size, 'pc');
      case 'trash': return U.namedIcon(S.trashFull ? 'user-trash-full' : 'user-trash', size, 'trash');
      case 'search': return glyphBox('search', size, 'is-search');
      case 'folder': return U.namedIcon(iconNameFor(loc.path), size, 'folder');
      default: return glyphBox('folder', size);
    }
  }

  function locIconSpec(loc) {
    const px = Math.round(16 * Math.min(3, Math.max(1, devicePixelRatio || 1)));
    switch (loc?.kind) {
      case 'home': return 'home';
      case 'gallery': return 'image';
      case 'search': return 'search';
      case 'thispc': return { url: bridge.iconUrl('computer', px), size: 16 };
      case 'trash': return { url: bridge.iconUrl(S.trashFull ? 'user-trash-full' : 'user-trash', px), size: 16 };
      case 'folder': return { url: bridge.iconUrl(iconNameFor(loc.path), px), size: 16 };
      default: return 'folder';
    }
  }

  /** Breadcrumb segments for a location. */
  function crumbsFor(loc) {
    if (loc.kind !== 'folder' && loc.kind !== 'search') return [{ label: locLabel(loc), loc }];
    const p = loc.path;
    const out = [];
    // root of the chain: the nearest known place, the home folder, or Deze pc › drive
    const places = Object.values(S.places).filter((pl) => pl.path && pl.id !== 'trash' && pl.id !== 'root' && pl.id !== 'home' && U.isUnder(p, pl.path));
    places.sort((a, b) => b.path.length - a.path.length);
    let base = places[0]?.path;
    if (!base && S.home && U.isUnder(p, S.home)) base = S.home;
    if (!base) {
      out.push({ label: t('thisPC'), loc: { kind: 'thispc' } });
      const v = volFor(p);
      base = v ? v.mount : '/';
    }
    out.push({ label: folderLabel(base), loc: { kind: 'folder', path: base } });
    if (p !== base) {
      const rest = p.slice(base === '/' ? 1 : base.length + 1).split('/').filter(Boolean);
      let cur = base;
      for (const seg of rest) {
        cur = U.joinPath(cur, seg);
        out.push({ label: seg, loc: { kind: 'folder', path: cur } });
      }
    }
    if (loc.kind === 'search') out.push({ label: t('searchResults', { name: folderLabel(p) }), loc });
    return out;
  }

  function parentLoc(loc) {
    if (!loc) return null;
    if (loc.kind === 'search') return { kind: 'folder', path: loc.path };
    if (loc.kind === 'trash' || loc.kind === 'gallery' || loc.kind === 'thispc') return { kind: 'home' };
    if (loc.kind !== 'folder') return null;
    if (loc.path === '/') return { kind: 'thispc' };
    const v = S.volumes.find((x) => x.mount === loc.path);
    if (v && v.mount !== '/') return { kind: 'thispc' };
    return { kind: 'folder', path: U.dirname(loc.path) };
  }

  function storedOn(p) {
    const v = volFor(p);
    if (!v || v.isSystem || v.isHome || v.mount === '/') return t('savedOnPC');
    return t('savedOn', { name: U.volumeLabel(v) });
  }

  function friendlyDir(dir) {
    if (!dir) return '';
    const pl = placeForPath(dir);
    if (pl) return pl.id === 'home' ? userLabel() : folderLabel(dir);
    if (S.home && U.isUnder(dir, S.home)) {
      const rel = dir.slice(S.home.length + 1);
      const first = rel.split('/')[0];
      const fp = placeForPath(U.joinPath(S.home, first));
      return fp ? [folderLabel(fp.path), ...rel.split('/').slice(1)].join(' › ') : `${userLabel()} › ${rel.split('/').join(' › ')}`;
    }
    return dir;
  }

  const origDir = (it) => it.originalDir || (it.originalPath ? U.dirname(it.originalPath) : '');
  const itemName = (it) => (it.volume ? U.volumeLabel(it.volume) : U.displayName(it, cfg('showExtensions', true)));

  function pinnedList() {
    let list = cfg('pinned', null);
    if (!Array.isArray(list)) list = PLACE_IDS.map((id) => placeById(id)?.path).filter(Boolean);
    return list.filter(Boolean).map((p) => ({ path: p, label: folderLabel(p), iconName: iconNameFor(p) }));
  }

  function defaultFolders() {
    return PC_FOLDERS.map((id) => placeById(id)).filter((pl) => pl?.path)
      .map((pl) => ({ path: pl.path, label: folderLabel(pl.path), iconName: iconNameFor(pl.path) }));
  }

  /* ============================================================ data loading */
  async function loadPlaces() {
    try {
      const list = await call('files.places');
      for (const pl of list || []) if (pl?.id) S.places[pl.id] = pl;
    } catch (e) { console.warn('[explorer] files.places failed', e); }
    S.home = S.places.home?.path || (S.user.name ? `/home/${S.user.name}` : '/');
  }

  let volPromise = null;
  function setVolumes(vols) {
    S.volumes = Array.isArray(vols) ? vols.filter((v) => v && v.mount && !v.hidden) : [];
    S.volumes.sort((a, b) => (b.isSystem - a.isSystem) || (a.removable - b.removable) || U.compareNames(U.volumeLabel(a), U.volumeLabel(b)));
  }
  function loadVolumes() {
    if (volPromise) return volPromise;
    volPromise = call('storage.volumes').then((vols) => {
      setVolumes(vols);
    }).catch((e) => { console.warn('[explorer] storage.volumes failed', e); })
      .finally(() => { setTimeout(() => { volPromise = null; }, 4000); navPane.render(); });
    return volPromise;
  }

  function applyClipboard(value) {
    const next = acceptClipboard(value);
    if (!next) return;
    S.clipboard = next;
    view?.setCut(S.clipboard.op === 'cut' ? new Set(S.clipboard.paths) : new Set());
    cmdBar.update();
  }

  async function loadClipboard() {
    try { applyClipboard(await call('files.clipboardGet')); }
    catch { /* A delayed failed request must not erase a newer successful update. */ }
  }

  async function loadTrashInfo() {
    try {
      const info = await call('files.trashInfo');
      const full = (info?.count || 0) > 0;
      if (full !== S.trashFull) { S.trashFull = full; navPane.render(); tabStrip.render(); }
    } catch { /* ignore */ }
  }

  async function listDirs(path) {
    const hit = dirCache.get(path);
    if (hit && Date.now() - hit.at < 4000) return hit.list;
    const res = await call('files.list', { path, showHidden: false });
    const list = U.sortItems((res?.items || []).filter((x) => x.isDir && !x.hidden && !x.name.startsWith('.')), 'name')
      .map((x) => ({ name: x.displayName || x.name, path: x.path, icon: x.icon }));
    dirCache.set(path, { at: Date.now(), list });
    return list;
  }

  async function collectImages(rootPath, max) {
    const out = [];
    const queue = [{ p: rootPath, d: 0 }];
    let dirs = 0;
    while (queue.length && dirs < 40 && out.length < 4000) {
      const { p, d } = queue.shift();
      dirs++;
      let res;
      try { res = await call('files.list', { path: p, showHidden: false }); } catch { continue; }
      for (const it of res?.items || []) {
        if (it.hidden || it.name.startsWith('.')) continue;
        if (it.isDir) { if (d < 3) queue.push({ p: it.path, d: d + 1 }); continue; }
        if ((it.mime || '').startsWith('image/') || IMG_RE.test(it.name)) out.push(it);
      }
    }
    out.sort((a, b) => (b.modified || 0) - (a.modified || 0));
    return out.slice(0, max);
  }

  /* ================================================================ tabs */
  function newTab(loc) {
    return { id: `t${++tabSeq}`, loc, history: [loc], hIndex: 0, items: null, status: 'idle', error: null,
      selection: new Set(), focus: null, filter: '', gen: 0, scrollMemo: new Map(), pendingSelect: null, pendingProps: false,
      pendingReveal: null, pendingPropsItems: null, pendingRename: null, listPath: null };
  }

  function openTab(loc, { activate = true } = {}) {
    const tab = newTab(loc);
    const i = tabs.findIndex((x) => x.id === activeId);
    tabs.splice(i + 1, 0, tab);
    if (activate) activeId = tab.id;
    load(tab);
    tabStrip.render();
    if (activate) { renderContent({ animateIn: true }); updateChrome(); }
    syncWatches();
    return tab;
  }

  function closeTab(id) {
    const i = tabs.findIndex((x) => x.id === id);
    if (i < 0) return;
    if (tabs.length === 1) { frame.close(); return; }
    tabs.splice(i, 1);
    if (activeId === id) { activeId = tabs[Math.min(i, tabs.length - 1)].id; renderContent(); updateChrome(); }
    tabStrip.render();
    syncWatches();
  }

  /** Transfer only semantic tab state; the destination refreshes its own file listing. */
  function exportTab(id) {
    const tab = tabOf(id);
    if (!tab) throw new Error('The source tab no longer exists');
    if (renamingIn(tab)) throw new Error(document.documentElement.lang === 'nl'
      ? 'Rond het hernoemen eerst af voordat je dit tabblad verplaatst.'
      : 'Finish renaming before moving this tab.');
    saveScroll(tab);
    return normalizeTabState({ version: 1, loc: tab.loc, history: tab.history, hIndex: tab.hIndex,
      selection: [...tab.selection], focus: tab.focus, filter: tab.filter || '', scrollMemo: [...tab.scrollMemo],
      pendingSelect: tab.pendingSelect, pendingProps: tab.pendingProps, pendingReveal: tab.pendingReveal });
  }

  // Native sessions contain semantic tabs only, never directory listings or in-flight requests.
  function saveWindowSession() {
    if (picker || window.__win11shellHost?.runtime !== 'rust' || !started || !tabs.length) return;
    saveScroll(active());
    const selected = tabs.slice(0, 20);
    if (active() && !selected.includes(active())) selected[selected.length - 1] = active();
    const states = selected.map((tab) => {
      try { return exportTab(tab.id); }
      catch { return normalizeTabState({ version: 1, loc: tab.loc, history: tab.history, hIndex: tab.hIndex,
        selection: [...tab.selection], focus: tab.focus, filter: tab.filter || '', scrollMemo: [...tab.scrollMemo],
        pendingSelect: tab.pendingSelect, pendingProps: false, pendingReveal: tab.pendingReveal }); }
    });
    const state = { version: 1, tabs: states, active: Math.max(0, selected.indexOf(active())) };
    if (new TextEncoder().encode(JSON.stringify(state)).length > 1024 * 1024) {
      for (const tab of states) Object.assign(tab, { history: [tab.loc], hIndex: 0, selection: [], focus: null,
        scrollMemo: [], pendingSelect: null, pendingProps: false, pendingReveal: null });
    }
    if (new TextEncoder().encode(JSON.stringify(state)).length <= 1024 * 1024)
      windowCommand('window-session', { session: state }, 'explorer');
  }
  addEventListener('win11:close', saveWindowSession);
  addEventListener('pagehide', saveWindowSession);

  function hydrateTab(state) {
    const saved = normalizeTabState(state);
    const tab = newTab(saved.loc);
    Object.assign(tab, { history: saved.history, hIndex: saved.hIndex,
      selection: new Set(saved.selection), focus: saved.focus, filter: saved.filter,
      scrollMemo: new Map(saved.scrollMemo), pendingSelect: saved.pendingSelect,
      pendingProps: saved.pendingProps, pendingReveal: saved.pendingReveal });
    return tab;
  }

  function moveTabBefore(id, beforeId) {
    const from = tabs.findIndex((tab) => tab.id === id);
    if (from < 0 || beforeId === id) return;
    const [tab] = tabs.splice(from, 1);
    const before = beforeId == null ? tabs.length : tabs.findIndex((item) => item.id === beforeId);
    tabs.splice(before < 0 ? tabs.length : before, 0, tab);
    tabStrip.render();
  }

  function rollbackImportedTab(id, previousActive) {
    const index = tabs.findIndex((tab) => tab.id === id);
    if (index < 0) return;
    const [tab] = tabs.splice(index, 1);
    tab.gen++; // Ignore any file response still in flight for the removed tab.
    if (!tabs.length) tabs.push(newTab({ kind: 'home' }));
    activeId = tabOf(previousActive) ? previousActive : tabs[Math.min(index, tabs.length - 1)].id;
    renderContent(); updateChrome(); tabStrip.render(); syncWatches();
  }

  async function importTab(state, { beforeId = null } = {}) {
    const previousActive = activeId;
    saveScroll(active());
    const tab = hydrateTab(state);
    const index = beforeId == null ? tabs.length : tabs.findIndex((item) => item.id === beforeId);
    tabs.splice(index < 0 ? tabs.length : index, 0, tab);
    activeId = tab.id;
    try {
      renderContent(); updateChrome(); tabStrip.render(); syncWatches();
      await load(tab);
      if (tab.status === 'error') throw tab.error || new Error('The destination could not load this tab');
      renderContent(); updateChrome(); tabStrip.render(); syncWatches();
      return tab;
    } catch (error) {
      rollbackImportedTab(tab.id, previousActive);
      throw error;
    }
  }

  function activateTab(id) {
    if (id === activeId || !tabOf(id)) return;
    saveScroll(active());
    activeId = id;
    if (active().status === 'idle') load(active());
    renderContent();
    updateChrome();
    tabStrip.render();
  }

  /* ========================================================== navigation */
  function saveScroll(tab) {
    if (tab && view && tab === active()) tab.scrollMemo.set(locKey(tab.loc), view.scroll);
  }

  /** select: one path or several (all selected, the first one focused and scrolled into view); props: then open
   *  "Eigenschappen" for them (org.freedesktop.FileManager1.ShowItemProperties). */
  function navigate(loc, { push = true, focus = false, tabId = activeId, select = null, props = false } = {}) {
    const tab = tabOf(tabId);
    if (!tab || !loc) return;
    if (loc.kind === 'folder' && loc.path) loc = { kind: 'folder', path: U.normPath(loc.path) };
    const prev = tab.loc;
    saveScroll(tab);
    if (push) {
      if (locKey(tab.history[tab.hIndex]) !== locKey(loc)) {
        tab.history = tab.history.slice(0, tab.hIndex + 1);
        tab.history.push(loc);
        if (tab.history.length > 60) tab.history.shift();
        tab.hIndex = tab.history.length - 1;
      }
    }
    tab.loc = loc;
    tab.filter = '';
    tab.selection = new Set();
    tab.focus = null;
    tab.items = null;
    tab.error = null;
    tab.wantFocus = focus;
    // going up/back: select the folder we came from (Windows does this)
    tab.pendingSelect = select || (prev?.kind === 'folder' && loc.kind === 'folder' && U.dirname(prev.path) === loc.path ? prev.path : null);
    tab.pendingProps = !!(props && select);
    tab.pendingReveal = null;
    tab.pendingPropsItems = null;
    load(tab);
    if (tab.id === activeId) { renderContent({ animateIn: true }); updateChrome(); }
    tabStrip.render();
    syncWatches();
  }

  const canBack = () => (active()?.hIndex || 0) > 0;
  const canForward = () => { const tb = active(); return !!tb && tb.hIndex < tb.history.length - 1; };
  function go(delta) {
    const tab = active();
    const idx = tab.hIndex + delta;
    if (idx < 0 || idx >= tab.history.length) return;
    tab.hIndex = idx;
    const prev = tab.loc;
    navigate(tab.history[idx], { push: false, focus: true, select: prev.kind === 'folder' && tab.history[idx].kind === 'folder' && U.dirname(prev.path) === tab.history[idx].path ? prev.path : null });
  }

  function historyItems(dir) {
    const tab = active();
    const out = [];
    if (dir < 0) for (let i = tab.hIndex - 1; i >= 0 && out.length < 12; i--) out.push(i);
    else for (let i = tab.hIndex + 1; i < tab.history.length && out.length < 12; i++) out.push(i);
    return out.map((i) => ({ label: locLabel(tab.history[i]), icon: locIconSpec(tab.history[i]), action: () => go(i - tab.hIndex) }));
  }

  function recentLocations() {
    const seen = new Set();
    const out = [];
    for (const tab of [active(), ...tabs]) {
      for (let i = tab.history.length - 1; i >= 0; i--) {
        const k = locKey(tab.history[i]);
        if (seen.has(k)) continue;
        seen.add(k);
        out.push(tab.history[i]);
      }
    }
    return out.slice(0, 12);
  }

  /* ================================================================ load */
  async function load(tab, { keep = false } = {}) {
    const gen = ++tab.gen;
    const loc = tab.loc;
    tab.reloadAfterRename = false; // this load is the refresh a rename may have postponed
    if (loc.kind === 'folder' || loc.kind === 'trash' || loc.kind === 'search') {
      const path = loc.kind === 'trash' ? S.places.trash?.path : loc.path;
      if (!path) {
        tab.status = 'error';
        tab.error = { message: t('noTrash') };
        if (tab.id === activeId) { renderContent(); updateChrome(); }
        return;
      }
      tab.status = 'loading';
      if (!keep && tab.id === activeId) setTimeout(() => { if (tab.gen === gen && tab.status === 'loading' && tab.id === activeId) renderContent(); }, 220);
      try {
        let items, extra = {};
        if (loc.kind === 'search') {
          const res = await call('files.find', { path, query: loc.query, limit: 500 });
          items = res?.items || [];
          extra = { truncated: !!res?.truncated };
        } else {
          const res = await call('files.list', { path, showHidden: !!cfg('showHidden', false) });
          items = res?.items || [];
          tab.listPath = res?.path || path;
        }
        if (gen !== tab.gen) return;
        tab.items = items;
        tab.byPath = new Map(items.map((x) => [x.path, x]));
        tab.truncated = extra.truncated || false;
        tab.status = 'ready';
        tab.error = null;
      } catch (e) {
        if (gen !== tab.gen) return;
        // a file instead of a folder → open its folder and select it
        if (loc.kind === 'folder' && !keep) {
          const props = await call('files.properties', { path }).catch(() => null);
          if (props && props.type !== 'folder' && props.contains == null && gen === tab.gen) {
            tab.loc = { kind: 'folder', path: U.dirname(path) };
            tab.history[tab.hIndex] = tab.loc;
            tab.pendingSelect = path;
            load(tab);
            if (tab.id === activeId) updateChrome();
            return;
          }
        }
        tab.status = 'error';
        tab.error = e;
        tab.items = null;
      }
      // a refresh that was already under way when the user pressed F2: keep the rename box, redraw afterwards
      if (tab.id === activeId && keep && renamingIn(tab)) { renderAfterRename = true; updateStatus(); }
      else if (tab.id === activeId) { renderContent({ keep }); updateChrome(); }
      tabStrip.render();
    } else {
      tab.items = null;
      tab.byPath = new Map();
      tab.status = 'ready';
    }
  }

  const reloadSoon = new Map();
  /** True while the visible list shows an inline rename box: re-rendering would throw away what the user typed. */
  const renamingIn = (tab) => !!(tab && tab.id === activeId && view?.renaming);
  function reload(tab = active(), delay = 0) {
    if (!tab) return;
    clearTimeout(reloadSoon.get(tab.id));
    reloadSoon.set(tab.id, setTimeout(() => {
      reloadSoon.delete(tab.id);
      // a folder change (files.dirChanged: a download finishing, the new folder itself …) while the user is
      // typing a name: refresh as soon as the rename is committed or cancelled (see renameEnded)
      if (renamingIn(tab)) { tab.reloadAfterRename = true; return; }
      if (tab.loc.kind === 'folder' || tab.loc.kind === 'trash' || tab.loc.kind === 'search') load(tab, { keep: true });
      else if (tab.id === activeId) renderContent();
    }, delay));
  }

  /** The inline rename box closed (committed or cancelled): run what was postponed while it was open. */
  let renderAfterRename = false;
  function renameEnded(tab) {
    const postponed = renderAfterRename;
    renderAfterRename = false;
    if (tab.reloadAfterRename) { tab.reloadAfterRename = false; reload(tab); }
    // (a committed rename has already scheduled its own reload, which redraws with the new name)
    else if (postponed && tab.id === activeId && !reloadSoon.has(tab.id)) renderContent({ keep: true });
  }

  function syncWatches() {
    const want = new Set();
    for (const tab of tabs) if (tab.loc.kind === 'folder' && tab.loc.path.startsWith('/')) want.add(tab.loc.path);
    for (const p of want) if (!watched.has(p)) { watched.add(p); call('files.watch', { path: p }).catch(() => {}); }
    for (const p of [...watched]) if (!want.has(p)) { watched.delete(p); call('files.unwatch', { path: p }).catch(() => {}); }
  }

  /* ============================================================== render */
  function columnsFor(kind) {
    const widths = cfg('columns', {}) || {};
    const hidden = new Set(cfg('hiddenColumns', []) || []);
    const W = (id, def) => (typeof widths[id] === 'number' ? widths[id] : def);
    const date = (it) => U.dateTime(it.modified);
    if (kind === 'trash') {
      return [
        { id: 'name', label: t('colName'), width: W('name', 280), min: 120 },
        { id: 'orig', label: t('colOrig'), width: W('orig', 220), render: (it) => friendlyDir(origDir(it)) },
        { id: 'deleted', label: t('colDeleted'), width: W('deleted', 150), render: (it) => U.dateTime(it.deletedAt || it.deleted) },
        { id: 'size', label: t('colSize'), width: W('size', 100), align: 'end', render: U.sizeText },
      ];
    }
    if (kind === 'search') {
      return [
        { id: 'name', label: t('colName'), width: W('name', 280), min: 120 },
        { id: 'location', label: t('colLocation'), width: W('location', 240), render: (it) => friendlyDir(U.dirname(it.path)) },
        { id: 'modified', label: t('colModified'), width: W('modified', 150), render: date },
        { id: 'size', label: t('colSize'), width: W('size', 100), align: 'end', render: U.sizeText },
      ];
    }
    return [
      { id: 'name', label: t('colName'), width: W('name', 300), min: 120 },
      { id: 'modified', label: t('colModified'), width: W('modified', 150), render: date, hidden: hidden.has('modified') },
      { id: 'type', label: t('colType'), width: W('type', 150), render: U.typeName, hidden: hidden.has('type') },
      { id: 'size', label: t('colSize'), width: W('size', 100), align: 'end', render: U.sizeText, hidden: hidden.has('size') },
    ];
  }

  const sortState = (kind) => {
    const cols = columnsFor(kind).map((c) => c.id);
    let key = cfg('sortBy', 'name');
    if (!cols.includes(key) && key !== 'type') key = 'name';
    return { key, desc: !!cfg('sortDesc', false) };
  };

  function visibleItems(tab) {
    let items = tab.items || [];
    if (!cfg('showHidden', false)) items = items.filter((x) => !x.hidden && !(x.name || '').startsWith('.'));
    if (tab.filter) {
      const q = tab.filter.toLowerCase();
      items = items.filter((x) => (x.displayName || x.name || '').toLowerCase().includes(q));
    }
    if (app.pickerFilter) items = items.filter(app.pickerFilter);
    const { key, desc } = sortState(tab.loc.kind);
    const getters = {
      orig: (x) => x.originalPath || '', deleted: (x) => x.deletedAt || x.deleted || 0, location: (x) => U.dirname(x.path),
    };
    return U.sortItems(items, key, desc, getters);
  }

  function groupsFor(tab, items) {
    const by = cfg('groupBy', 'none');
    if (!by || by === 'none' || !items.length) return null;
    const map = new Map();
    for (const it of items) {
      let g;
      if (by === 'modified') g = U.dateGroup(it.modified);
      else if (by === 'deleted') g = U.dateGroup(it.deletedAt || it.deleted);
      else if (by === 'size') g = U.sizeGroup(it);
      else if (by === 'type') { const n = U.typeName(it); g = { key: n, label: n, order: it.isDir ? -1 : 0 }; }
      else if (by === 'name') { const c = (it.displayName || it.name || '?').charAt(0).toUpperCase(); const k = /[A-Z]/.test(c) ? c : (/[0-9]/.test(c) ? '0 - 9' : '#'); g = { key: k, label: k, order: 0 }; }
      else g = { key: '', label: '', order: 0 };
      if (!map.has(g.key)) map.set(g.key, { ...g, items: [] });
      map.get(g.key).items.push(it);
    }
    const list = [...map.values()];
    list.sort((a, b) => (a.order - b.order) || U.compareNames(a.label, b.label));
    if (cfg('sortDesc', false) && (by === 'name' || by === 'type')) list.reverse();
    return list;
  }

  /** fn once, on the next frame — or after 120 ms when no frame comes (a window that is not drawn right now: hidden
   *  behind another workspace, still being mapped, a headless test), so a selection asked for from another app is
   *  still scrolled into view and its Eigenschappen still open. */
  function afterFirstFrame(fn) {
    let done = false;
    const run = () => { if (!done) { done = true; fn(); } };
    requestAnimationFrame(run);
    setTimeout(run, 120);
  }

  let loadingTimer = 0;
  function renderContent({ animateIn = false, keep = false } = {}) {
    clearTimeout(loadingTimer);
    renderAfterRename = false;
    const tab = active();
    if (!tab) return;
    const hadFocus = contentHost.contains(document.activeElement);
    const prevScroll = keep && view ? view.scroll : null;
    view?.destroy();
    view = null;
    pageItems.clear();
    const kind = tab.loc.kind;
    let node;
    rootEl.dataset.kind = kind;
    if (kind === 'home') node = HomePage(app);
    else if (kind === 'thispc') node = ThisPCPage(app);
    else if (kind === 'gallery') node = GalleryPage(app);
    else if (tab.status === 'error') node = errorView(tab);
    else if (tab.status === 'loading' && !tab.items) {
      node = h('div', { class: 'xp-loading' }, ProgressRing({ size: 32, ariaLabel: t('loading') }));
    } else if (!tab.items) {
      node = h('div', { class: 'xp-loading' });
    } else {
      node = fileView(tab, prevScroll, hadFocus || tab.wantFocus);
    }
    clear(contentHost).append(node);
    if (animateIn && !keep) animate(node, [{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }], { duration: 167 });
    if ((kind === 'home' || kind === 'thispc' || kind === 'gallery') && tab.wantFocus) node.focus?.({ preventScroll: true });
    repaintSelection();
    updateSelectionUI();
  }

  function fileView(tab, prevScroll, focus) {
    const kind = tab.loc.kind;
    const items = visibleItems(tab);
    const mode = kind === 'search' && !cfg('view') ? 'details' : cfg('view', 'details');
    const gkey = locKey(tab.loc);
    if (!groupCollapse.has(gkey)) groupCollapse.set(gkey, new Set());
    let empty = null;
    if (!items.length) {
      if (tab.filter || kind === 'search') empty = h('div', { class: 'xp-empty__text' }, t('noMatch'));
      else if (kind === 'trash') {
        empty = h('div', { class: 'xp-bigempty' }, h('span', { class: 'xp-bigempty__icon' }, U.namedIcon('user-trash', 64, 'trash')),
          h('div', { class: 'subtitle' }, t('trashEmpty')), h('div', { class: 'text-2' }, t('trashEmptyMsg')));
      } else empty = h('div', { class: 'xp-empty__text' }, t('folderEmpty'));
    }
    // one path or several (Weergeven in map / ShowItems): select them all, focus + scroll to the first one. The
    // scroll and the Eigenschappen dialog wait on the tab for the first frame of a view that is still current
    // (a refresh right after loading — settings, a watch event — replaces the view before that frame).
    const wanted = (Array.isArray(tab.pendingSelect) ? tab.pendingSelect : [tab.pendingSelect]).filter((p) => p && tab.byPath?.has(p));
    if (wanted.length) {
      tab.selection = new Set(wanted);
      tab.focus = wanted[0];
      tab.pendingReveal = wanted[0];
      if (tab.pendingProps) tab.pendingPropsItems = wanted.slice();
    }
    tab.pendingSelect = null;
    tab.pendingProps = false;
    const v = FileView({
      mode, items, columns: columnsFor(kind), groups: groupsFor(tab, items), selection: tab.selection, focusPath: tab.focus,
      cut: S.clipboard.op === 'cut' ? new Set(S.clipboard.paths) : new Set(), sort: sortState(kind),
      compact: !!cfg('compact', false), checkboxes: !!cfg('checkboxes', false), showExt: !!cfg('showExtensions', true),
      empty, collapsed: groupCollapse.get(gkey), label: locLabel(tab.loc), srcDir: kind === 'folder' ? tab.loc.path : null,
      draggable: kind !== 'trash',
      typeOf: U.typeName, sizeOf: U.sizeText,
      contentProps: (it) => [it.modified ? `${t('colModified')}: ${U.dateTime(it.modified)}` : '', it.isDir ? '' : `${t('colSize')}: ${U.sizeText(it)}`],
      onOpen: (item) => openItem(item, { focus: true }),
      onMiddle: (item) => { if (item.isDir) openTab({ kind: 'folder', path: item.path }, { activate: false }); },
      onSelect: (sel, focusPath) => { tab.selection = sel; tab.focus = focusPath; updateSelectionUI(); },
      onContext: (e, item) => {
        const items2 = item ? selectedItems() : null;
        const m = item ? menus.itemMenu(items2) : menus.backgroundMenu();
        if (m) showMenuAt(e, m);
      },
      onRename: (item, newName) => renameItem(item, newName),
      onRenameEnd: () => { if (view === v) renameEnded(tab); },
      onSort: (key) => act.setSort(key),
      onColumnResize: (id, w) => { const c = { ...(cfg('columns', {}) || {}) }; c[id] = Math.round(w); setCfg('columns', c); },
      onHeaderContext: (e) => showMenuAt(e, { items: menus.columnsMenu() }),
      onCollapse: () => {},
      dropInfo: kind === 'folder' ? (target, e) => dropInfo(target ? target.path : tab.loc.path, e) : null,
      onDrop: ({ paths, dest, copy }) => dropTo(paths, dest, copy),
    });
    view = v;
    const scroll = prevScroll || tab.scrollMemo.get(locKey(tab.loc));
    afterFirstFrame(() => {
      if (view !== v) return;
      if (scroll) v.restore(scroll);
      if (tab.pendingReveal) { v.reveal(tab.pendingReveal, { focus }); tab.pendingReveal = null; }
      if (tab.pendingRename && tab.byPath?.has(tab.pendingRename)) { const p = tab.pendingRename; tab.pendingRename = null; v.startRename(p); }
      else if (focus) v.focus();
      const props = (tab.pendingPropsItems || []).map((p) => tab.byPath?.get(p)).filter(Boolean);   // after a refresh: the new items
      tab.pendingPropsItems = null;
      if (props.length) act.properties(props);
    });
    tab.wantFocus = false;
    // header bar for filtered folders and search results
    const parts = [];
    if (kind === 'search') {
      parts.push(h('div', { class: 'xp-searchbar' }, icon('search', { size: 14 }),
        h('span', { class: 'xp-searchbar__text' }, `${t('searchResults', { name: folderLabel(tab.loc.path) })} · “${tab.loc.query}”`),
        tab.truncated ? h('span', { class: 'xp-searchbar__note' }, t('truncated')) : null,
        HyperlinkButton({ label: t('searchEverywhere'), onClick: () => openAlles(tab.loc.query) })));
    } else if (tab.filter && kind === 'folder') {
      parts.push(h('div', { class: 'xp-searchbar' }, icon('filter', { size: 14 }),
        h('span', { class: 'xp-searchbar__text' }, t('filtered', { name: folderLabel(tab.loc.path) })),
        HyperlinkButton({ label: t('searchSub'), onClick: () => navigate({ kind: 'search', path: tab.loc.path, query: tab.filter }, { focus: true }) }),
        HyperlinkButton({ label: t('searchEverywhere'), onClick: () => openAlles(tab.filter) })));
    }
    if (!parts.length) return v.el;
    return h('div', { class: 'xp-viewwrap' }, ...parts, v.el);
  }

  function errorView(tab) {
    const e = tab.error || {};
    return h('div', { class: 'xp-bigempty' },
      h('span', { class: 'xp-bigempty__icon is-warning' }, icon('warning', { size: 40 })),
      h('div', { class: 'subtitle' }, t('errOpenFolder')),
      h('div', { class: 'text-2' }, e.message || ''),
      h('div', { class: 'row' },
        Button({ label: t('tryAgain'), variant: 'accent', onClick: () => load(tab) }),
        Button({ label: t('goHome'), onClick: () => navigate({ kind: 'home' }) })));
  }

  function updateChrome() {
    const tab = active();
    if (!tab) return;
    addressRow.update();
    cmdBar.update();
    navPane.markSelected();
    frame.setTitle(picker ? app.pickerTitle?.() || t('appName') : `${locLabel(tab.loc)} - ${t('appName')}`);
    updateStatus();
  }

  function selectedItems() {
    const tab = active();
    if (!tab) return [];
    const src = view ? tab.byPath : pageItems;
    const out = [];
    const order = view ? view.order : null;
    if (order) { for (const it of order) if (tab.selection.has(it.path)) out.push(it); }
    else for (const p of tab.selection) { const it = src?.get(p); if (it) out.push(it); }
    return out;
  }

  function updateStatus() {
    const tab = active();
    const sel = selectedItems();
    clear(statusLeft);
    const count = view ? view.order.length : (tab.loc.kind === 'folder' || tab.loc.kind === 'trash' ? visibleItems(tab).length : pageItems.size);
    if (tab.status !== 'error') statusLeft.append(h('span', null, t('items', { n: count })));
    if (sel.length) {
      statusLeft.append(h('span', { class: 'xp-status__sep' }), h('span', null, t('selected', { n: sel.length })));
      const files = sel.filter((x) => !x.isDir);
      if (files.length) statusLeft.append(h('span', { class: 'xp-status__size' }, U.sizeText({ size: files.reduce((a, x) => a + (x.size || 0), 0) })));
    }
    const m = cfg('view', 'details');
    stDetails.classList.toggle('is-on', m === 'details');
    stLarge.classList.toggle('is-on', m === 'large');
    const showBtns = !!view;
    stDetails.hidden = !showBtns; stLarge.hidden = !showBtns;
  }

  function updateSelectionUI() {
    cmdBar.update();
    app.pickerSelection?.(selectedItems());
    updateStatus();
    if (cfg('detailsPane', false) || cfg('previewPane', false)) details.update();
  }

  function repaintSelection() {
    const tab = active();
    if (!tab) return;
    for (const el of contentHost.querySelectorAll('.xp-pitem')) el.classList.toggle('is-selected', tab.selection.has(el.dataset.path));
    updateStatus();
  }

  function setSelection(paths, focus) {
    const tab = active();
    if (view) { view.select(paths, focus); return; }
    tab.selection = new Set(paths);
    tab.focus = focus;
    repaintSelection();
    updateSelectionUI();
  }

  function applyLayout() {
    const navOn = cfg('navPane', true);
    const detOn = cfg('detailsPane', false);
    const prevOn = !detOn && cfg('previewPane', false);   // one right-hand pane at a time, like Windows
    body.style.setProperty('--nav-w', `${clamp(Number(cfg('navWidth', 248)) || 248, 160, 480)}px`);
    navPane.el.hidden = !navOn;
    splitter.hidden = !navOn;
    details.setMode(prevOn ? 'preview' : 'details');
    details.el.hidden = !(detOn || prevOn);
    if (detOn || prevOn) details.render();
  }

  /* ============================================================= messages */
  function toast(title, { message = '', severity, action, timeout = 5000 } = {}) {
    return Toast.show({ title, message, severity, action, timeout });
  }

  function showError(e, title = t('errTitle')) {
    const msg = typeof e === 'string' ? e : (e?.message || '');
    while (infobars.children.length >= 3) infobars.firstChild.remove();
    const bar = InfoBar({ severity: 'error', title, message: msg, onClose: () => bar.remove() });
    infobars.append(bar);
    animate(bar, [{ opacity: 0, transform: 'translateY(-4px)' }, { opacity: 1, transform: 'none' }], { duration: 167 });
    setTimeout(() => { if (bar.isConnected) bar.remove(); }, 15000);
    return bar;
  }

  let dryToastShown = false;
  function checkDry(res) {
    if (res && res.dryRun && !dryToastShown) { dryToastShown = true; toast(t('dryRun'), { severity: 'info' }); }
    return res;
  }

  /* Long file operations (copy/move/paste/zip/unzip) carry a jobId; when one takes longer than a moment a
     Windows-style progress card ("3 items kopiëren naar Documenten") appears above the file list, fed by
     the host's files.progress events. */
  const JOB_OPS = { 'files.copy': 'copy', 'files.move': 'move', 'files.paste': 'paste', 'files.compress': 'compress', 'files.extract': 'extract' };
  const jobs = new Map();
  let jobSeq = 0;
  function showJob(job) {
    const title = h('div', { class: 'xp-job__title' }, t(`job_${job.kind}`, { n: job.count, name: job.dest ? folderLabel(job.dest) : '' }));
    const bar = ProgressBar({ indeterminate: true, ariaLabel: title.textContent });
    const sub = h('div', { class: 'xp-job__sub' });
    const pct = h('div', { class: 'xp-job__pct' });
    job.card = h('div', { class: 'xp-job', role: 'status' }, h('span', { class: 'xp-job__icon' }, icon(job.kind === 'move' ? 'cut' : job.kind === 'compress' || job.kind === 'extract' ? 'folder' : 'copy', { size: 16 })),
      h('div', { class: 'xp-job__text' }, h('div', { class: 'xp-job__row' }, title, pct), bar, sub));
    job.update = (d) => {
      if (d.total) { const v = Math.max(0, Math.min(100, (d.done / d.total) * 100)); bar.setValue(v); pct.textContent = `${Math.floor(v)}%`; }
      if (d.current) sub.textContent = U.basename(d.current);
    };
    infobars.append(job.card);
    if (job.last) job.update(job.last);
  }
  bridge.on('files.progress', (d) => {
    const job = d?.jobId ? jobs.get(d.jobId) : null;
    if (!job) return;
    job.last = d;
    job.update?.(d);
  });

  async function op(method, args) {
    const kind = JOB_OPS[method];
    if (!kind) return checkDry(await call(method, args));
    const jobId = `xp-${Date.now().toString(36)}-${++jobSeq}`;
    const job = { kind, count: (args.paths || [args.path]).filter(Boolean).length || S.clipboard.paths.length || 1, dest: args.dest || null, card: null, last: null };
    jobs.set(jobId, job);
    const timer = setTimeout(() => { if (jobs.has(jobId)) showJob(job); }, 700);
    try {
      return checkDry(await call(method, { ...args, jobId }));
    } finally {
      clearTimeout(timer);
      jobs.delete(jobId);
      job.card?.remove();
    }
  }

  function failedNames(res) {
    const f = res?.failed || [];
    if (!f.length) return '';
    return f.slice(0, 5).map((x) => (typeof x === 'string' ? U.basename(x) : U.basename(x.path || x.name || ''))).join(', ') + (f.length > 5 ? ' …' : '');
  }

  /* ============================================================ operations */
  const curDir = () => { const tb = active(); return tb?.loc.kind === 'folder' ? tb.loc.path : null; };

  function selectAfterReload(paths, { rename = null } = {}) {
    const tab = active();
    tab.pendingSelect = paths?.[0] || null;
    tab.selection = new Set(paths || []);
    tab.focus = paths?.[0] || null;
    tab.pendingRename = rename;
    reload(tab);
  }

  async function openItem(item, { focus = false } = {}) {
    if (!item) return;
    const tab = active();
    if (item.volume) { navigate({ kind: 'folder', path: item.volume.mount }, { focus }); return; }
    if (tab?.loc.kind === 'trash') { showProperties(app, [item]); return; }
    if (item.isDir) { navigate({ kind: 'folder', path: item.path }, { focus }); return; }
    if (picker) { app.pickerSelect?.(item); return; }
    try { await op('files.open', { path: item.path }); }
    catch (e) { showError(e); }
  }

  async function renameItem(item, newName) {
    try {
      const res = await op('files.rename', { path: item.path, newName });
      const newPath = res?.path || U.joinPath(U.dirname(item.path), newName);
      undoStack.push({ label: 'rename', run: () => op('files.rename', { path: newPath, newName: item.name }) });
      if (newName.startsWith('.') && !item.name.startsWith('.') && !cfg('showHidden', false)) toast(t('nameHidden'), { severity: 'info' });
      selectAfterReload([newPath]);
      return true;
    } catch (e) {
      return e?.message || t('errTitle');
    }
  }

  /** files.setAttributes (host) with fallbacks: files.setReadOnly (mock) / 'rename-hidden' when unsupported. */
  async function setAttributes(args, { hiddenFallback = false } = {}) {
    try {
      checkDry(await call('files.setAttributes', args));
      reload(active(), 150);
      return true;
    } catch (e) {
      if (e?.code !== 'unknown_method') return e?.message || t('errTitle');
      if (args.readOnly != null) {
        try { await call('files.setReadOnly', { path: args.path, readOnly: args.readOnly }); }
        catch (e2) { if (e2?.code !== 'unknown_method') return e2?.message || t('errTitle'); }
      }
      reload(active(), 150);
      return hiddenFallback && args.hidden != null ? 'rename-hidden' : true;
    }
  }

  async function trashPaths(paths) {
    if (!paths.length) return;
    try {
      const res = await op('files.trash', { paths });
      const failed = res?.failed || [];
      const done = paths.filter((p) => !failed.some((f) => (typeof f === 'string' ? f : f.path) === p));
      if (failed.length) showError(t('errSome', { names: failedNames(res) }), t('errTitle'));
      if (done.length && !res?.dryRun) {
        // the host may tell us the names inside the trash; otherwise look them up when undoing
        const known = Array.isArray(res?.trashed) && res.trashed.length
          ? res.trashed.map((x) => x.name || x.trashName).filter(Boolean) : null;
        const undo = async () => {
          const names = known || await trashNamesFor(done);
          if (names.length) restoreNames(names);
        };
        undoStack.push({ label: 'trash', run: undo });
        toast(t('trashed', { n: done.length, name: U.basename(done[0]) }), { action: { label: t('undo'), onClick: undo } });
      }
      loadTrashInfo();
      reload(active());
    } catch (e) { showError(e); }
  }

  /** Trash names of the most recently trashed items that came from these paths. */
  async function trashNamesFor(paths) {
    const tp = S.places.trash?.path;
    if (!tp) return [];
    const res = await call('files.list', { path: tp }).catch(() => null);
    const items = res?.items || [];
    return paths.map((p) => {
      const c = items.filter((x) => (x.originalPath || x.origPath) === p)
        .sort((a, b) => (b.deletedAt || 0) - (a.deletedAt || 0))[0];
      return c ? (c.trashName || c.name) : null;
    }).filter(Boolean);
  }

  async function restoreNames(names) {
    try {
      const res = await op('files.restoreTrash', { names });
      if (res?.failed?.length) showError(t('errSome', { names: failedNames(res) }));
      else toast(t('restored', { n: names.length, name: names[0] }), { severity: 'success', timeout: 3000 });
      loadTrashInfo();
      reload(active());
    } catch (e) { showError(e); }
  }

  function dropInfo(dest, e) {
    if (!dest) return null;
    const cur = dnd.current;
    if (cur) {
      if (cur.paths.includes(dest)) return null;
      if (cur.paths.some((p) => U.isUnder(dest, p))) return null;
      if (cur.srcDir === dest && !e.ctrlKey) return null;
    }
    const srcVol = cur ? volFor(cur.paths[0]) : null;
    const dstVol = volFor(dest);
    const sameDisk = !!cur && (srcVol?.mount || '/') === (dstVol?.mount || '/');
    const copy = e.ctrlKey || (!e.shiftKey && !sameDisk);
    return { dest, copy, label: t(copy ? 'copyTo' : 'moveTo', { name: folderLabel(dest) }) };
  }

  async function dropTo(paths, dest, copy) {
    paths = paths.filter((p) => p && p !== dest && !U.isUnder(dest, p) && (copy || U.dirname(p) !== dest));
    if (!paths.length) return;
    try {
      const res = await op(copy ? 'files.copy' : 'files.move', { paths, dest });
      if (res?.failed?.length) showError(t('errSome', { names: failedNames(res) }));
      if (!res?.dryRun) {
        const n = paths.length - (res?.failed?.length || 0);
        if (n > 0) {
          let undo = null;
          if (!copy) {
            const byDir = new Map();
            for (const p of paths) { const d = U.dirname(p); if (!byDir.has(d)) byDir.set(d, []); byDir.get(d).push(U.joinPath(dest, U.basename(p))); }
            undo = async () => { for (const [d, ps] of byDir) await op('files.move', { paths: ps, dest: d }).catch(showError); reload(active()); };
            undoStack.push({ label: 'move', run: undo });
          }
          toast(t(copy ? 'copied' : 'moved', { n, name: U.basename(paths[0]), dest: folderLabel(dest) }), undo ? { action: { label: t('undo'), onClick: undo } } : {});
        }
      }
      dirCache.delete(dest);
      reload(active());
    } catch (e) { showError(e); }
  }

  function openAlles(query) {
    call('shell.openApp', { app: 'alles', params: { query: query || '' } }).catch((e) => showError(e));
  }

  const act = {
    open() {
      const sel = selectedItems();
      if (!sel.length) return;
      if (sel.length === 1) { openItem(sel[0], { focus: true }); return; }
      for (const it of sel.slice(0, 10)) {
        if (it.isDir) openTab({ kind: 'folder', path: it.volume?.mount || it.path }, { activate: false });
        else op('files.open', { path: it.path }).catch(showError);
      }
    },
    async newFolder() {
      const dir = curDir();
      if (!dir) return;
      try {
        const res = await op('files.createFolder', { dir });
        if (res?.path) { undoStack.push({ label: 'new', run: () => op('files.trash', { paths: [res.path] }) }); selectAfterReload([res.path], { rename: res.path }); }
        else reload();
      } catch (e) { showError(e); }
    },
    async newText() {
      const dir = curDir();
      if (!dir) return;
      try {
        const res = await op('files.createFile', { dir, kind: 'text' });
        if (res?.path) { undoStack.push({ label: 'new', run: () => op('files.trash', { paths: [res.path] }) }); selectAfterReload([res.path], { rename: res.path }); }
        else reload();
      } catch (e) { showError(e); }
    },
    async cut() { await clip('cut'); },
    async copy() { await clip('copy'); },
    async paste(dest = curDir()) {
      if (!dest) return;
      try {
        const res = await op('files.paste', { dest });
        if (res?.failed?.length) showError(t('errSome', { names: failedNames(res) }));
        await loadClipboard();
        if (Array.isArray(res?.paths) && res.paths.length) selectAfterReload(res.paths); else reload();
      } catch (e) { showError(e); }
    },
    rename() {
      const tab = active();
      const sel = selectedItems();
      if (!view || sel.length !== 1 || tab.loc.kind === 'trash') return;
      view.startRename(sel[0].path);
    },
    trash() {
      const tab = active();
      if (tab.loc.kind === 'trash') { toast(t('trashDelete'), { severity: 'info', timeout: 7000 }); return; }
      if (tab.loc.kind !== 'folder' && tab.loc.kind !== 'search' && tab.loc.kind !== 'gallery') return;
      trashPaths(selectedItems().filter((x) => !x.volume).map((x) => x.path));
    },
    trashPaths,
    async copyPath(items = selectedItems()) {
      const paths = items.length ? items.map((x) => x.volume?.mount || x.path) : [curDir()].filter(Boolean);
      if (!paths.length) return;
      if (await U.copyText(paths.map((p) => `"${p}"`).join('\n'))) toast(t('pathCopied', { n: paths.length }), { timeout: 2500 });
    },
    properties(items = selectedItems()) {
      const tab = active();
      if (items.length) { showProperties(app, items); return; }
      if (tab.loc.kind === 'folder') {
        showProperties(app, [{ path: tab.loc.path, name: U.basename(tab.loc.path) || '/', isDir: true, icon: iconNameFor(tab.loc.path) }]);
      } else if (tab.loc.kind === 'trash') showTrashProperties(app);
    },
    share(items = selectedItems()) { if (items.length) showShare(app, items); },
    async compress(items = selectedItems()) {
      if (!items.length) return;
      try {
        const res = await op('files.compress', { paths: items.map((x) => x.path) });
        if (res?.path) { toast(t('compressed', { name: U.basename(res.path) }), { timeout: 3000 }); selectAfterReload([res.path], { rename: curDir() === U.dirname(res.path) ? res.path : null }); }
      } catch (e) { showError(e); }
    },
    async extract(item) {
      try {
        const res = await op('files.extract', { path: item.path });
        if (res?.path) {
          toast(t('extracted', { name: U.basename(res.path) }), { action: { label: t('open'), onClick: () => navigate({ kind: 'folder', path: res.path }) } });
          selectAfterReload([res.path]);
        }
      } catch (e) { showError(e); }
    },
    terminal(dir = curDir()) { if (dir) op('files.openTerminal', { dir }).catch(showError); },
    pin(path) {
      const list = pinnedList().map((x) => x.path);
      if (!list.includes(path)) { list.push(path); setCfg('pinned', list); }
    },
    unpin(path) { setCfg('pinned', pinnedList().map((x) => x.path).filter((p) => p !== path)); },
    isPinned: (path) => pinnedList().some((x) => x.path === path),
    fav(path) { const f = cfg('favorites', []) || []; if (!f.includes(path)) setCfg('favorites', [...f, path]); },
    unfav(path) { setCfg('favorites', (cfg('favorites', []) || []).filter((p) => p !== path)); },
    isFav: (path) => (cfg('favorites', []) || []).includes(path),
    async emptyTrash() {
      const info = await call('files.trashInfo').catch(() => null);
      const n = info?.count ?? active().items?.length ?? 0;
      if (!n) return;
      const ok = await Dialog.confirm({
        title: t('emptyTitle'), message: t('emptyMsg', { n, size: bytes(info?.size || 0) }), confirmLabel: t('emptyConfirm'),
        cancelLabel: t('cancel'), danger: true, icon: 'warning',
      });
      if (!ok) return;
      try {
        await op('files.emptyTrash');
        toast(t('emptied'), { severity: 'success', timeout: 3000 });
        loadTrashInfo();
        reload(active());
      } catch (e) { showError(e); }
    },
    restore(items) {
      const tab = active();
      if (tab.loc.kind !== 'trash') return; // only items in the Prullenbak can be restored
      const list = items || (tab.selection.size ? selectedItems() : (tab.items || []));
      if (list.length) restoreNames(list.map((x) => x.trashName || x.name));
    },
    selectAll() { if (view) view.selectAll(); else setSelection([...pageItems.keys()], null); },
    selectNone() { setSelection([], null); },
    invert() { if (view) view.invert(); },
    setView(mode) { if (VIEW_MODES.includes(mode)) setCfg('view', mode); },
    setSort(key) {
      if (cfg('sortBy', 'name') === key) setCfg('sortDesc', !cfg('sortDesc', false));
      else { setCfg('sortBy', key); setCfg('sortDesc', key === 'modified' || key === 'deleted' || key === 'size'); }
    },
    setSortDir(desc) { setCfg('sortDesc', !!desc); },
    setGroup(key) { setCfg('groupBy', key); },
    toggleCfg(key) {
      const defs = { navPane: true, detailsPane: false, previewPane: false, showExtensions: true };
      const on = !cfg(key, defs[key] ?? false);
      // Detailvenster and Voorbeeldvenster share the right-hand side: turning one on turns the other off
      if (on && key === 'detailsPane' && cfg('previewPane', false)) setCfg('previewPane', false);
      if (on && key === 'previewPane' && cfg('detailsPane', false)) setCfg('detailsPane', false);
      setCfg(key, on);
    },
    async undo() {
      const u = undoStack.pop();
      if (!u) return;
      try { await u.run(); toast(t('undone'), { timeout: 2000 }); } catch (e) { showError(e); }
      loadTrashInfo();
      reload(active(), 100);
    },
    openNewTab(items = selectedItems()) {
      for (const it of items.filter((x) => x.isDir).slice(0, 10)) openTab({ kind: 'folder', path: it.volume?.mount || it.path }, { activate: false });
    },
    openLocation(item) { navigate({ kind: 'folder', path: U.dirname(item.path) }, { select: item.path, focus: true }); },
  };

  async function clip(opName) {
    const sel = selectedItems().filter((x) => !x.volume);
    if (!sel.length) return;
    const paths = sel.map((x) => x.path);
    try {
      applyClipboard(await call('files.clipboardSet', { paths, op: opName }));
    } catch (e) { showError(e); }
  }

  /* =============================================================== menus */
  /** Show a menu (object, array or a promise of one) at the pointer, or at the focused item for keyboard. */
  async function showMenuAt(e, m) {
    const x = e?.clientX, y = e?.clientY;
    const opts0 = await m;
    if (!opts0) return;
    const opts = Array.isArray(opts0) ? { items: opts0 } : opts0;
    if (x || y) showContextMenu({ x, y, ...opts });
    else {
      const target = document.activeElement?.closest?.('.xp-item, .xp-pitem') || contentHost;
      const r = target.getBoundingClientRect();
      showContextMenu({ x: r.left + 24, y: r.top + Math.min(r.height, 28), keyboard: true, ...opts });
    }
  }

  /** Small line glyphs for the view modes (Windows shows one per entry in the Weergeven menu). */
  const VIEW_GLYPH = {
    xl: '<rect x="2.5" y="2.5" width="11" height="11" rx="1.5"/>',
    large: '<rect x="2.5" y="2.5" width="4.5" height="4.5" rx="1"/><rect x="9" y="2.5" width="4.5" height="4.5" rx="1"/><rect x="2.5" y="9" width="4.5" height="4.5" rx="1"/><rect x="9" y="9" width="4.5" height="4.5" rx="1"/>',
    medium: '<rect x="2" y="2" width="3" height="3" rx=".6"/><rect x="6.5" y="2" width="3" height="3" rx=".6"/><rect x="11" y="2" width="3" height="3" rx=".6"/><rect x="2" y="6.5" width="3" height="3" rx=".6"/><rect x="6.5" y="6.5" width="3" height="3" rx=".6"/><rect x="11" y="6.5" width="3" height="3" rx=".6"/><rect x="2" y="11" width="3" height="3" rx=".6"/><rect x="6.5" y="11" width="3" height="3" rx=".6"/><rect x="11" y="11" width="3" height="3" rx=".6"/>',
    small: '<rect x="2" y="3" width="2.5" height="2.5" rx=".5"/><path d="M6 4.25h3"/><rect x="2" y="10.5" width="2.5" height="2.5" rx=".5"/><path d="M6 11.75h3"/><rect x="10" y="3" width="2.5" height="2.5" rx=".5"/><path d="M13.5 4.25h1"/><rect x="10" y="10.5" width="2.5" height="2.5" rx=".5"/><path d="M13.5 11.75h1"/>',
    list: '<path d="M2.5 3.5h1.5M6 3.5h7.5M2.5 8h1.5M6 8h7.5M2.5 12.5h1.5M6 12.5h7.5"/>',
    details: '<rect x="2" y="2.5" width="12" height="11" rx="1.5"/><path d="M2 6h12M2 9.5h12M6.5 2.5v11"/>',
    tiles: '<rect x="2" y="2.5" width="5" height="4.5" rx="1"/><path d="M8.5 3.5h5M8.5 6h3.5"/><rect x="2" y="9" width="5" height="4.5" rx="1"/><path d="M8.5 10h5M8.5 12.5h3.5"/>',
    content: '<rect x="2" y="2.5" width="3.5" height="3.5" rx=".7"/><path d="M7 3.5h7M7 5.5h4.5"/><rect x="2" y="10" width="3.5" height="3.5" rx=".7"/><path d="M7 11h7M7 13h4.5"/>',
  };
  function viewGlyph(m) {
    const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    s.setAttribute('viewBox', '0 0 16 16');
    s.setAttribute('width', '16');
    s.setAttribute('height', '16');
    s.setAttribute('fill', 'none');
    s.setAttribute('stroke', 'currentColor');
    s.setAttribute('stroke-width', '1.1');
    s.setAttribute('stroke-linecap', 'round');
    s.setAttribute('aria-hidden', 'true');
    s.setAttribute('class', 'fl-icon xp-viewglyph');
    s.innerHTML = VIEW_GLYPH[m] || '';
    return s;
  }

  function viewItems() {
    const cur = cfg('view', 'details');
    // the selected mode shows the radio dot (core menu: one glyph cell), the others their view glyph
    return VIEW_MODES.map((m) => ({
      label: t(m === 'details' ? 'detailsView' : m), radio: true, checked: cur === m, icon: viewGlyph(m),
      shortcut: `Ctrl+Shift+${VIEW_MODES.indexOf(m) + 1}`, action: () => act.setView(m),
    }));
  }

  function sortKeys(kind) {
    return columnsFor(kind).filter((c) => !c.hidden).map((c) => ({ id: c.id, label: c.label }))
      .concat(kind === 'folder' && !columnsFor(kind).some((c) => c.id === 'type' && !c.hidden) ? [{ id: 'type', label: t('colType') }] : []);
  }

  function groupItems(kind) {
    const cur = cfg('groupBy', 'none');
    const keys = kind === 'trash' ? ['name', 'deleted', 'size'] : ['name', 'modified', 'type', 'size'];
    const label = { name: t('colName'), modified: t('colModified'), type: t('colType'), size: t('colSize'), deleted: t('colDeleted') };
    return [...keys.map((k) => ({ label: label[k], radio: true, checked: cur === k, action: () => act.setGroup(k) })),
      { separator: true }, { label: t('none'), radio: true, checked: !cur || cur === 'none', action: () => act.setGroup('none') }];
  }

  const menus = {
    newMenu: () => [
      { label: t('newFolder'), icon: 'folder', shortcut: 'Ctrl+Shift+N', action: act.newFolder },
      { separator: true },
      { label: t('newText'), icon: 'document', action: act.newText },
    ],
    sortMenu() {
      const kind = active().loc.kind;
      const cur = sortState(kind);
      return [
        ...sortKeys(kind).map((c) => ({ label: c.label, radio: true, checked: cur.key === c.id, action: () => { setCfg('sortBy', c.id); } })),
        { separator: true },
        { label: t('asc'), radio: true, checked: !cur.desc, action: () => act.setSortDir(false) },
        { label: t('desc'), radio: true, checked: cur.desc, action: () => act.setSortDir(true) },
        { separator: true },
        { label: t('groupBy'), icon: 'list', submenu: groupItems(kind) },
      ];
    },
    viewMenu: () => [
      ...viewItems(),
      { separator: true },
      { label: t('compact'), checked: !!cfg('compact', false), action: () => act.toggleCfg('compact') },
      { separator: true },
      { label: t('show'), icon: 'eye', submenu: menus.showItems() },
    ],
    showItems: () => [
      { label: t('navPane'), checked: !!cfg('navPane', true), action: () => act.toggleCfg('navPane') },
      { label: t('detailsPane'), checked: !!cfg('detailsPane', false), shortcut: 'Alt+Shift+P', action: () => act.toggleCfg('detailsPane') },
      { label: t('previewPane'), checked: !cfg('detailsPane', false) && !!cfg('previewPane', false), shortcut: 'Alt+P', action: () => act.toggleCfg('previewPane') },
      { separator: true },
      { label: t('checkboxes'), checked: !!cfg('checkboxes', false), action: () => act.toggleCfg('checkboxes') },
      { label: t('extensions'), checked: !!cfg('showExtensions', true), action: () => act.toggleCfg('showExtensions') },
      { label: t('hidden'), checked: !!cfg('showHidden', false), action: () => act.toggleCfg('showHidden') },
    ],
    moreMenu() {
      const fileish = !!view;
      return [
        { label: t('selectAll'), icon: 'check', shortcut: 'Ctrl+A', disabled: !fileish && !pageItems.size, action: act.selectAll },
        { label: t('selectNone'), icon: 'close', disabled: !active().selection.size, action: act.selectNone },
        { label: t('invert'), icon: 'refresh', disabled: !fileish, action: act.invert },
        { separator: true },
        { label: t('properties'), icon: 'properties', shortcut: 'Alt+Enter', action: () => act.properties() },
        { label: t('options'), icon: 'settings', action: () => showOptions(app) },
      ];
    },
    columnsMenu() {
      const hidden = new Set(cfg('hiddenColumns', []) || []);
      const toggle = (id) => { if (hidden.has(id)) hidden.delete(id); else hidden.add(id); setCfg('hiddenColumns', [...hidden]); };
      return [
        { label: t('colName'), checked: true, disabled: true },
        { label: t('colModified'), checked: !hidden.has('modified'), action: () => toggle('modified') },
        { label: t('colType'), checked: !hidden.has('type'), action: () => toggle('type') },
        { label: t('colSize'), checked: !hidden.has('size'), action: () => toggle('size') },
      ];
    },
    itemMenu(items) {
      const tab = active();
      if (!items?.length) return null;
      const one = items.length === 1 ? items[0] : null;
      const kind = tab.loc.kind;
      if (kind === 'trash') {
        return {
          items: [
            { label: t('restore'), icon: 'undo', action: () => act.restore(items) },
            { separator: true },
            { label: t('properties'), icon: 'properties', shortcut: 'Alt+Enter', action: () => act.properties(items) },
          ],
        };
      }
      const fileish = kind === 'folder' || kind === 'search';
      const isVol = !!one?.volume;
      const top = [
        { icon: 'cut', label: t('cutMenu'), disabled: !fileish, action: act.cut },
        { icon: 'copy', label: t('copyMenu'), disabled: isVol, action: act.copy },
        { icon: 'rename', label: t('renameMenu'), disabled: !(fileish && one), action: act.rename },
        { icon: 'share', label: t('share'), disabled: isVol, action: () => act.share(items) },
        { icon: 'delete', label: t('deleteMenu'), disabled: !(fileish || kind === 'gallery') || isVol, action: act.trash },
      ];
      const list = [];
      list.push({ label: t('open'), icon: 'open', shortcut: 'Enter', action: act.open });
      if (items.every((x) => x.isDir)) list.push({ label: t('openNewTab'), icon: 'window', action: () => act.openNewTab(items) });
      if (one && !one.isDir) {
        list.push({ label: t('openWith'), icon: 'apps', submenu: [{ label: t('loading'), disabled: true }], lazy: one });
      }
      if (kind === 'home' && one && !one.isDir) list.push({ label: t('openLocation'), icon: 'folder-open', action: () => act.openLocation(one) });
      if (kind === 'search' && one) list.push({ label: t('openLocation'), icon: 'folder-open', action: () => act.openLocation(one) });
      list.push({ separator: true });
      if (one && one.isDir && !isVol) {
        const pinned = act.isPinned(one.path);
        list.push({ label: pinned ? t('unpin') : t('pin'), icon: pinned ? 'unpin' : 'pin', action: () => (pinned ? act.unpin(one.path) : act.pin(one.path)) });
      }
      if (one && !one.isDir) {
        const fav = act.isFav(one.path);
        list.push({ label: fav ? t('unfav') : t('fav'), icon: 'star', action: () => (fav ? act.unfav(one.path) : act.fav(one.path)) });
      }
      if (fileish && !isVol) list.push({ label: t('compress'), icon: 'clipboard', action: () => act.compress(items) });
      if (one && !one.isDir && U.ARCHIVE_RE.test(one.name || '')) list.push({ label: t('extract'), icon: 'folder-open', action: () => act.extract(one) });
      list.push({ label: t('copyPath'), icon: 'link', shortcut: 'Ctrl+Shift+C', action: () => act.copyPath(items) });
      list.push({ separator: true });
      list.push({ label: t('properties'), icon: 'properties', shortcut: 'Alt+Enter', action: () => act.properties(items) });
      if (one && one.isDir) list.push({ separator: true }, { label: t('terminal'), icon: 'terminal', action: () => act.terminal(one.volume?.mount || one.path) });
      // "Openen met" is filled asynchronously
      const owItem = list.find((x) => x.lazy);
      if (owItem) {
        owItem.submenu = [{ label: t('loading'), disabled: true }];
        return loadOpenWith(owItem, one).then(() => ({ items: list, topActions: top.every((x) => x.disabled) ? [] : top }));
      }
      return { items: list, topActions: top.every((x) => x.disabled) ? [] : top };
    },
    backgroundMenu() {
      const tab = active();
      const kind = tab.loc.kind;
      const fileish = kind === 'folder' || kind === 'search';
      if (kind === 'trash') {
        return {
          items: [
            { label: t('ctxView'), icon: 'view', submenu: viewItems() },
            { label: t('sortBy'), icon: 'sort', submenu: menus.sortMenu() },
            { label: t('refreshMenu'), icon: 'refresh', shortcut: 'F5', action: () => app.refresh() },
            { separator: true },
            { label: t('emptyTrash'), icon: 'trash', disabled: !tab.items?.length, action: act.emptyTrash },
            { label: t('restoreAll'), icon: 'undo', disabled: !tab.items?.length, action: () => act.restore(tab.items || []) },
            { separator: true },
            { label: t('properties'), icon: 'properties', action: () => showTrashProperties(app) },
          ],
        };
      }
      if (!fileish) return { items: [{ label: t('refreshMenu'), icon: 'refresh', shortcut: 'F5', action: () => app.refresh() }] };
      return {
        items: [
          { label: t('ctxView'), icon: 'view', submenu: viewItems() },
          { label: t('sortBy'), icon: 'sort', submenu: menus.sortMenu().slice(0, -2) },
          { label: t('groupBy'), icon: 'list', submenu: groupItems(kind) },
          { label: t('refreshMenu'), icon: 'refresh', shortcut: 'F5', action: () => app.refresh() },
          { separator: true },
          { label: t('pasteMenu'), icon: 'paste', shortcut: 'Ctrl+V', disabled: !(kind === 'folder' && S.clipboard.paths.length), action: () => act.paste() },
          { label: t('undo'), icon: 'undo', shortcut: 'Ctrl+Z', disabled: !undoStack.length, action: act.undo },
          { separator: true },
          { label: t('newMenu'), icon: 'plus', disabled: kind !== 'folder', submenu: menus.newMenu() },
          { separator: true },
          { label: t('properties'), icon: 'properties', action: () => act.properties([]) },
          { label: t('terminal'), icon: 'terminal', disabled: kind !== 'folder', action: () => act.terminal() },
        ],
      };
    },
    navMenu(node) {
      const loc = node.loc;
      if (loc.kind === 'trash') {
        return {
          items: [
            { label: t('openNewTab'), icon: 'window', action: () => openTab(loc, { activate: false }) },
            { separator: true },
            { label: t('emptyTrash'), icon: 'trash', disabled: !S.trashFull, action: act.emptyTrash },
            { label: t('properties'), icon: 'properties', action: () => showTrashProperties(app) },
          ],
        };
      }
      if (loc.kind !== 'folder') return { items: [{ label: t('openNewTab'), icon: 'window', action: () => openTab(loc, { activate: false }) }] };
      const vol = S.volumes.find((v) => v.mount === loc.path && node.id.startsWith('vol:'));
      const pinned = act.isPinned(loc.path);
      return {
        items: [
          { label: t('openNewTab'), icon: 'window', action: () => openTab(loc, { activate: false }) },
          { separator: true },
          vol ? null : { label: pinned ? t('unpin') : t('pin'), icon: pinned ? 'unpin' : 'pin', action: () => (pinned ? act.unpin(loc.path) : act.pin(loc.path)) },
          { label: t('copyPath'), icon: 'link', action: () => act.copyPath([{ path: loc.path }]) },
          { label: t('terminal'), icon: 'terminal', action: () => act.terminal(loc.path) },
          { separator: true },
          { label: t('properties'), icon: 'properties', action: () => (vol ? showDriveProperties(app, vol) : showProperties(app, [{ path: loc.path, name: U.basename(loc.path), isDir: true, icon: iconNameFor(loc.path) }])) },
        ].filter(Boolean),
      };
    },
  };

  async function loadOpenWith(owItem, item) {
    delete owItem.lazy;
    try {
      // never keep the menu waiting long: after 450 ms it opens with "Bezig met laden…"
      const apps = await Promise.race([call('apps.forFile', { path: item.path }), new Promise((r) => setTimeout(() => r(null), 450))]);
      if (apps === null) return;
      owItem.submenu = (apps || []).length
        ? apps.map((a) => ({
          label: a.name, icon: { url: bridge.iconUrl(a.icon, 32), size: 16 },
          action: () => op('files.openWith', { path: item.path, appId: a.id }).catch(showError),
        }))
        : [{ label: t('noApps'), disabled: true }];
    } catch { owItem.submenu = [{ label: t('noApps'), disabled: true }]; }
  }

  /* ========================================================== address bar */
  function addressText(loc) {
    if (loc.kind === 'folder' || loc.kind === 'search') return loc.path;
    return locLabel(loc);
  }

  const NAMED = {
    home: ['start', 'home', 'snelle toegang', 'quick access'], thispc: ['deze pc', 'this pc', 'computer', 'mijn computer'],
    trash: ['prullenbak', 'recycle bin', 'trash'], gallery: ['galerij', 'gallery'],
  };

  async function resolveAddress(text) {
    let s = (text || '').trim().replace(/^"(.*)"$/, '$1');
    if (!s) return null;
    const low = s.toLowerCase();
    for (const [kind, names] of Object.entries(NAMED)) if (names.includes(low)) return { kind };
    for (const id of PLACE_IDS) {
      const pl = placeById(id);
      if (pl?.path && (low === t(id).toLowerCase() || low === (pl.name || '').toLowerCase())) return { kind: 'folder', path: pl.path };
    }
    if (/^https?:\/\//i.test(s)) { op('apps.openUri', { uri: s }).catch(showError); return null; }
    if (/^trash:/i.test(s)) return { kind: 'trash' };
    if (/^computer:/i.test(s)) return { kind: 'thispc' };
    if (s.startsWith('file://')) s = U.uriToPath(s) || s;
    if (s === '~' || s.startsWith('~/')) s = S.home + s.slice(1);
    if (!s.startsWith('/')) { const base = curDir() || S.home; s = U.joinPath(base, s); }
    s = U.normPath(s);
    try {
      const res = await call('files.list', { path: s, showHidden: false });
      return { kind: 'folder', path: res?.path || s };
    } catch {
      const props = await call('files.properties', { path: s }).catch(() => null);
      if (props) { navigate({ kind: 'folder', path: U.dirname(s) }, { select: s }); op('files.open', { path: s }).catch(showError); return { kind: 'folder', path: U.dirname(s), handled: true }; }
      return null;
    }
  }

  async function addressSuggestions(text) {
    let s = text || '';
    if (!s) return [];
    if (s === '~' || s.startsWith('~/')) s = S.home + s.slice(1);
    if (!s.startsWith('/')) {
      const low = s.toLowerCase();
      const named = [
        { label: t('home'), value: t('home'), icon: 'home' }, { label: t('thisPC'), value: t('thisPC'), icon: locIconSpec({ kind: 'thispc' }) },
        { label: t('trash'), value: t('trash'), icon: locIconSpec({ kind: 'trash' }) }, { label: t('gallery'), value: t('gallery'), icon: 'image' },
        ...PLACE_IDS.filter((id) => placeById(id)).map((id) => ({ label: t(id), value: placeById(id).path, icon: locIconSpec({ kind: 'folder', path: placeById(id).path }) })),
      ];
      return named.filter((x) => x.label.toLowerCase().startsWith(low)).slice(0, 8);
    }
    const dir = s.endsWith('/') ? U.normPath(s) : U.dirname(s);
    const prefix = s.endsWith('/') ? '' : U.basename(s).toLowerCase();
    let list = [];
    try { list = await listDirs(dir); } catch { return []; }
    return list.filter((d) => d.name.toLowerCase().startsWith(prefix)).slice(0, 12)
      .map((d) => ({ label: d.path, value: d.path, icon: { url: bridge.iconUrl(d.icon || 'folder', 32), size: 16 } }));
  }

  async function subfolderMenu(loc) {
    const nav = (l) => () => navigate(l);
    if (loc.kind === 'root') {
      return [
        { label: t('home'), icon: 'home', action: nav({ kind: 'home' }) },
        { label: t('gallery'), icon: 'image', action: nav({ kind: 'gallery' }) },
        { separator: true },
        ...pinnedList().map((p) => ({ label: p.label, icon: locIconSpec({ kind: 'folder', path: p.path }), action: nav({ kind: 'folder', path: p.path }) })),
        { separator: true },
        { label: t('thisPC'), icon: locIconSpec({ kind: 'thispc' }), action: nav({ kind: 'thispc' }) },
        S.places.trash ? { label: t('trash'), icon: locIconSpec({ kind: 'trash' }), action: nav({ kind: 'trash' }) } : null,
      ].filter(Boolean);
    }
    if (loc.kind === 'thispc') {
      await loadVolumes();
      return [...defaultFolders().map((f) => ({ label: f.label, icon: locIconSpec({ kind: 'folder', path: f.path }), action: nav({ kind: 'folder', path: f.path }) })),
        { separator: true },
        ...S.volumes.map((v) => ({ label: U.volumeLabel(v), icon: locIconSpec({ kind: 'folder', path: v.mount }), action: nav({ kind: 'folder', path: v.mount }) }))];
    }
    if (loc.kind !== 'folder') return [];
    try {
      const dirs = await listDirs(loc.path);
      return dirs.slice(0, 60).map((d) => ({
        label: d.name, icon: { url: bridge.iconUrl(d.icon || 'folder', 32), size: 16 }, action: nav({ kind: 'folder', path: d.path }),
      }));
    } catch { return []; }
  }

  /* ============================================================= keyboard */
  function onKey(e) {
    if (document.querySelector('.fl-dialog-layer') || isContextMenuOpen()) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const k = e.key;
    const low = k.length === 1 ? k.toLowerCase() : k;
    const inInput = U.isTextInput(e.target);
    const done = () => { e.preventDefault(); e.stopPropagation(); };
    if (picker && ctrl && ['t', 'n'].includes(low)) { done(); return; }
    if (ctrl && !e.altKey && !e.shiftKey && low === 't') { done(); openTab({ kind: 'home' }); return; }
    if (ctrl && !e.altKey && low === 'n' && !e.shiftKey) { done(); openTab(active().loc); return; }
    if (ctrl && !e.altKey && low === 'w') { done(); closeTab(activeId); return; }
    if (ctrl && k === 'Tab') {
      done();
      const i = tabs.findIndex((x) => x.id === activeId);
      activateTab(tabs[(i + (e.shiftKey ? -1 : 1) + tabs.length) % tabs.length].id);
      return;
    }
    if ((ctrl && low === 'l') || (e.altKey && low === 'd') || (k === 'F4' && !e.altKey && !ctrl)) { done(); addressRow.focusAddress(); return; }
    if ((ctrl && (low === 'f' || low === 'e')) || k === 'F3') { done(); addressRow.focusSearch(); return; }
    if (e.altKey && !ctrl && k === 'ArrowLeft') { done(); go(-1); return; }
    if (e.altKey && !ctrl && k === 'ArrowRight') { done(); go(1); return; }
    if (e.altKey && !ctrl && k === 'ArrowUp') { done(); app.up(); return; }
    if (k === 'F5' || (ctrl && low === 'r')) { done(); app.refresh(); return; }
    if (e.altKey && low === 'p') { done(); act.toggleCfg(e.shiftKey ? 'detailsPane' : 'previewPane'); return; }
    if (ctrl && e.shiftKey && /^Digit[1-8]$/.test(e.code)) { done(); act.setView(VIEW_MODES[Number(e.code.slice(5)) - 1]); return; }
    if (inInput) return;
    const inContent = contentHost.contains(e.target) || e.target === document.body || e.target === frame.body;
    const onButton = e.target.closest?.('button, [role="tab"], [role="treeitem"], .fl-menu');
    if (ctrl && e.shiftKey && low === 'n') { done(); act.newFolder(); return; }
    if (ctrl && e.shiftKey && low === 'c') { done(); act.copyPath(); return; }
    if (e.altKey && k === 'Enter') { done(); act.properties(); return; }
    if (k === 'Backspace' && !onButton) { done(); app.up(); return; }
    if (onButton && (k === 'Enter' || k === ' ')) return; // let buttons (group headers, tabs, …) handle these
    if (!inContent && onButton) return;
    if (k === 'Enter' && !e.altKey && inContent && view) { if (view.renaming) return; done(); act.open(); return; }
    if (k === 'F2') { done(); act.rename(); return; }
    if (k === 'Delete') { done(); act.trash(); return; }
    if (ctrl && !e.shiftKey && low === 'c') { done(); act.copy(); return; }
    if (ctrl && !e.shiftKey && low === 'x') { done(); act.cut(); return; }
    if (ctrl && !e.shiftKey && low === 'v') { done(); act.paste(); return; }
    if (ctrl && !e.shiftKey && low === 'z') { done(); act.undo(); return; }
    if (ctrl && !e.shiftKey && low === 'a' && !contentHost.contains(e.target)) { done(); act.selectAll(); return; }
    if (k === 'Escape' && active().filter) { done(); addressRow.clearSearch(); app.setFilter(''); }
  }
  document.addEventListener('keydown', onKey);

  // Ctrl + mouse wheel: change icon size (Windows)
  contentHost.addEventListener('wheel', (e) => {
    if (!e.ctrlKey || !view) return;
    e.preventDefault();
    const cur = cfg('view', 'details');
    let i = ZOOM_ORDER.indexOf(cur);
    if (i < 0) i = 0;
    const j = clamp(i + (e.deltaY < 0 ? 1 : -1), 0, ZOOM_ORDER.length - 1);
    if (j !== i) act.setView(ZOOM_ORDER[j]);
  }, { passive: false });

  /* ============================================================= splitter */
  splitter.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    splitter.setPointerCapture(e.pointerId);
    const x0 = e.clientX;
    const w0 = navPane.el.getBoundingClientRect().width;
    body.classList.add('is-resizing');
    let w = w0;
    const move = (ev) => { w = clamp(w0 + ev.clientX - x0, 160, 480); body.style.setProperty('--nav-w', `${w}px`); };
    const up = () => {
      splitter.removeEventListener('pointermove', move);
      body.classList.remove('is-resizing');
      setCfg('navWidth', Math.round(w));
    };
    splitter.addEventListener('pointermove', move);
    splitter.addEventListener('pointerup', up, { once: true });
    splitter.addEventListener('pointercancel', up, { once: true });
  });
  splitter.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const w = clamp(navPane.el.getBoundingClientRect().width + (e.key === 'ArrowRight' ? 16 : -16), 160, 480);
    body.style.setProperty('--nav-w', `${w}px`);
    setCfg('navWidth', Math.round(w));
  });

  /* ======================================================== native title bar */
  // the tabs change the no-drag areas without resizing the title bar, so ask appframe to re-send the drag region
  const reportTitlebar = () => frame.reportTitlebar();
  const reportSoon = debounce(reportTitlebar, 60);

  /* =============================================================== settings */
  const RELOAD_KEYS = ['explorer.showHidden'];
  const CHROME_ONLY = ['explorer.columns', 'explorer.navWidth'];
  onSettings((s, path) => {
    if (path && CHROME_ONLY.includes(path)) return;
    if (path && !path.startsWith('explorer') && !path.startsWith('privacy') && !path.startsWith('theme') && !path.startsWith('language')) return;
    applyLayout();
    if (path === 'explorer.pinned' || path == null) navPane.render();
    if (path && RELOAD_KEYS.includes(path)) { for (const tb of tabs) if (tb.loc.kind === 'folder') dirCache.clear(); reload(active()); return; }
    if (path === 'explorer.detailsPane' || path === 'explorer.previewPane' || path === 'explorer.navPane') { updateChrome(); return; }
    // a setting changed elsewhere (another window, Instellingen) while a name is being typed: redraw afterwards
    if (view?.renaming) { renderAfterRename = true; updateChrome(); return; }
    renderContent({ keep: true });
    updateChrome();
  });
  // all labels are built once; a language switch (from Instellingen) simply reloads this window, a moment later so
  // the settings write that caused it has finished. The tabs come back as they were (D7, §2.7): each tab's place,
  // its history, selection and scroll position, and the active tab (sessionStorage: this window only, read once).
  let langTimer = 0;
  onLangChange(() => {
    clearTimeout(langTimer);
    langTimer = setTimeout(() => { saveForReload(); location.reload(); }, 400);
  });
  function saveForReload() {
    try {
      saveScroll(active());
      const keep = (loc) => loc && loc.kind !== 'search';          // a search result list is not kept, its folder is
      const state = {
        at: Date.now(),
        active: Math.max(0, tabs.findIndex((x) => x.id === activeId)),
        tabs: tabs.map((tb) => {
          const loc = keep(tb.loc) ? tb.loc : (tb.loc.path ? { kind: 'folder', path: tb.loc.path } : { kind: 'home' });
          return { loc, history: tb.history.filter(keep), select: [...tb.selection].slice(0, 1000),
            scroll: tb.scrollMemo.get(locKey(tb.loc)) || null };
        }),
      };
      sessionStorage.setItem(RELOAD_KEY, JSON.stringify(state));
    } catch (e) { console.warn('[explorer] could not keep the tabs over the reload', e); }
  }

  /* ================================================================ events */
  bridge.on('files.dirChanged', (d) => {
    const p = d?.path;
    if (!p) return;
    dirCache.delete(p);
    for (const tb of tabs) {
      if ((tb.loc.kind === 'folder' && tb.loc.path === p) || (tb.loc.kind === 'trash' && S.places.trash?.path === p)) reload(tb, 250);
    }
    if (S.places.trash?.path && (p === S.places.trash.path || U.isUnder(p, U.dirname(S.places.trash.path)))) loadTrashInfo();
    navPane.invalidate(p);
  });
  bridge.on('files.desktopChanged', () => {
    const dp = S.places.desktop?.path;
    if (dp) for (const tb of tabs) if (tb.loc.kind === 'folder' && tb.loc.path === dp) reload(tb, 250);
  });
  bridge.on('files.clipboardChanged', applyClipboard);
  // USB sticks and other drives that come and go (host: storage.volumesChanged {volumes})
  bridge.on('storage.volumesChanged', (d) => {
    if (!Array.isArray(d?.volumes)) return;
    setVolumes(d.volumes);
    navPane.render();
    if (active()?.loc.kind === 'thispc') renderContent({ keep: true });
  });
  bridge.on('files.trashChanged', () => {
    loadTrashInfo();
    for (const tb of tabs) if (tb.loc.kind === 'trash') reload(tb, 250);
  });
  addEventListener('focus', () => { loadClipboard(); });

  // Windows draws the 1 px focus rectangle on items only after keyboard navigation, never after a mouse click.
  // Chrome/WebKit's :focus-visible cannot tell (the list is focused by script), so track the input modality.
  const NAV_KEYS = new Set(['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown', 'Tab', 'F6']);
  document.addEventListener('keydown', (e) => {
    const typeSelect = e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.target?.closest?.('.xp-scroll, .xp-page');
    if (NAV_KEYS.has(e.key) || typeSelect) frame.el.classList.add('has-kbd');
  }, true);
  document.addEventListener('pointerdown', () => frame.el.classList.remove('has-kbd'), true);

  /** A place word (params.place, or a non-path params.path such as the desktop's 'thispc'/'trash') → location. */
  function placeToLoc(word) {
    const w = String(word || '').trim().toLowerCase();
    if (!w) return null;
    if (/^trash:/.test(w) || ['trash', 'prullenbak', 'recyclebin', 'recycle-bin'].includes(w)) return { kind: 'trash' };
    if (/^computer:/.test(w) || ['thispc', 'this-pc', 'computer', 'mypc', 'deze-pc', 'drives'].includes(w)) return { kind: 'thispc' };
    if (/^recent:/.test(w) || ['home', 'start', 'quickaccess', 'quick-access', 'recent'].includes(w)) return { kind: 'home' };
    if (['gallery', 'galerij'].includes(w)) return { kind: 'gallery' };
    const alias = { pictures: 'pictures', afbeeldingen: 'pictures', documents: 'documents', documenten: 'documents', downloads: 'downloads',
      music: 'music', muziek: 'music', videos: 'videos', "video's": 'videos', desktop: 'desktop', bureaublad: 'desktop',
      homefolder: 'home', 'home-folder': 'home', user: 'home', personal: 'home' }[w];
    if (alias && placeById(alias)?.path) return { kind: 'folder', path: placeById(alias).path };
    if (placeById(w)?.path && w !== 'trash' && w !== 'root') return { kind: 'folder', path: placeById(w).path };
    return null;
  }

  function paramsToLoc(params = {}) {
    if (params.path) {
      const raw = String(params.path);
      let p = raw.startsWith('/') ? raw : raw.trim();   // '/x/Map ' is a real folder name; ' thispc ' is a place word
      if (/^(trash|computer|recent):/i.test(p) || !/^(\/|~|file:)/.test(p)) {
        const named = placeToLoc(p);
        if (named) return named;
      }
      if (p.startsWith('file://')) p = U.uriToPath(p) || p;
      if (p === '~' || p.startsWith('~/')) p = S.home + p.slice(1);
      if (!p.startsWith('/')) return placeToLoc(params.place) || null;
      return { kind: 'folder', path: U.normPath(p) };
    }
    return placeToLoc(params.place);
  }

  /** A params value as the app got it: the real thing (app.navigate) or JSON text (the URL of a new window). */
  function jsonish(v) {
    if (typeof v !== 'string' || !/^\s*[[{]/.test(v)) return v;
    try { return JSON.parse(v); } catch { return v; }   // a name such as "[concept] brief.txt" stays a name
  }
  const truthy = (v) => v === true || v === 1 || /^(1|true|yes|ja)$/i.test(String(v ?? ''));

  /** params.select: one or more full paths, file:// URIs or names inside the target folder → absolute paths|null. */
  function selectFor(params, loc) {
    let sel = jsonish(params?.select);
    if (!sel) return null;
    if (!Array.isArray(sel)) sel = [sel];
    const out = new Set();
    for (let s of sel.slice(0, 1000)) {
      if (typeof s !== 'string' || !s) continue;
      if (s.startsWith('file://')) s = U.uriToPath(s) || s;
      if (!s.startsWith('/') && loc?.kind === 'folder') s = U.joinPath(loc.path, s);
      if (s.startsWith('/')) out.add(U.normPath(s));
    }
    return out.size ? [...out] : null;
  }

  /** params.tabs: more folders, each in its own tab right after the current one (in the given order). */
  const MAX_EXTRA_TABS = 20;
  async function extraTabs(params) {
    const list = jsonish(params?.tabs);
    if (!Array.isArray(list) || !list.length) return 0;
    if (list.length > MAX_EXTRA_TABS) console.info('[explorer] %d more tabs asked, %d opened', list.length, MAX_EXTRA_TABS);
    let at = Math.max(0, tabs.findIndex((x) => x.id === activeId)) + 1;
    let n = 0;
    for (const e of list.slice(0, MAX_EXTRA_TABS)) {
      const p = typeof e === 'string' ? { path: e } : (e && typeof e === 'object' ? e : null);
      let loc = p && paramsToLoc(p);
      if (!loc) continue;
      let select = selectFor(p, loc);
      ({ loc, select } = await resolveFileParam(loc, select));
      const nt = newTab(loc);
      nt.pendingSelect = select;
      at = Math.min(at, tabs.length);
      tabs.splice(at++, 0, nt);
      load(nt);
      n++;
    }
    if (n) { tabStrip.render(); syncWatches(); }
    return n;
  }

  /** "Bestandslocatie openen" (Taakbeheer, Alles, Start) passes the file itself as params.path: ask the host what it
   *  is first, so the file's folder opens with the file selected, instead of listing (and watching) a file. */
  async function resolveFileParam(loc, select) {
    if (loc?.kind !== 'folder' || select || placeForPath(loc.path)) return { loc, select };
    try {
      const st = await call('files.stat', { path: loc.path });
      if (st && st.isDir === false) return { loc: { kind: 'folder', path: U.dirname(loc.path) }, select: loc.path };
    } catch { /* missing or unreadable: load() shows the friendly error */ }
    return { loc, select };
  }

  async function handleParams(params = {}, { newTabIfBusy = false } = {}) {
    let loc = paramsToLoc(params);
    if (!loc) return (await extraTabs(params)) > 0;
    let select = selectFor(params, loc);
    ({ loc, select } = await resolveFileParam(loc, select));
    const props = truthy(params.properties);
    const tab = active();
    if (newTabIfBusy && tab && !(tab.loc.kind === 'home' && tab.history.length === 1)) {
      const nt = openTab(loc);
      if (select) nt.pendingSelect = select;
      nt.pendingProps = props && !!select;
    } else navigate(loc, { focus: true, select, props });
    await extraTabs(params);
    return true;
  }

  /* Requests that reach a window that is still starting (SPEC §12.1 explorer.ready): app.navigate before start()
   * finished waits in `early`; the host keeps what this page cannot have received yet and sends it as
   * explorer.backlog once our event stream is up — we skip what did arrive as app.navigate, and confirm. */
  let started = false;
  const early = [];
  const seenNav = [];                    // {k: JSON of params, t} of app.navigate received in the last minute
  const navId = Math.random().toString(36).slice(2, 12) + Date.now().toString(36);
  let backlogGot = false, readyTries = 0, readyTimer = 0, lastBoot = '', lastSeq = 0;
  const openNav = (params) => (started ? handleParams(params, { newTabIfBusy: true }) : early.push(params));
  frame.onAppNavigate((params) => {
    const now = Date.now();
    while (seenNav.length && (seenNav.length > 32 || now - seenNav[0].t > 60000)) seenNav.shift();
    seenNav.push({ k: JSON.stringify(params), t: now });
    openNav(params);
  });
  bridge.on('explorer.backlog', async (d) => {
    if (!d || d.id !== navId) return;
    backlogGot = true;
    clearTimeout(readyTimer);
    if (d.boot !== lastBoot) { lastBoot = d.boot; lastSeq = 0; }   // another host: its own numbers
    for (const it of Array.isArray(d.navigate) ? d.navigate : []) {
      const p = it?.params, n = Number(it?.seq) || 0;
      if (!p || typeof p !== 'object' || n <= lastSeq) continue;   // already opened from an earlier backlog
      lastSeq = n;
      const i = seenNav.findIndex((x) => x.k === JSON.stringify(p));
      if (i >= 0) { seenNav.splice(i, 1); continue; }              // it did arrive as app.navigate
      try { await openNav(p); } catch (e) { console.warn('[explorer] late request failed', e); }
    }
    call('explorer.ready', { id: navId, ack: Number(d.seq) || 0 }).catch(() => {});
  });
  function askBacklog(restart = false) {
    if (restart) { backlogGot = false; readyTries = 0; }
    clearTimeout(readyTimer);
    if (backlogGot || readyTries++ >= 20) return;
    call('explorer.ready', { id: navId }).catch((e) => {
      if (e?.code === 'unknown_method') { readyTries = Infinity; clearTimeout(readyTimer); }   // an older host
    });
    readyTimer = setTimeout(askBacklog, 1000);              // until explorer.backlog proves the stream is up
  }
  bridge.on('hello', () => { if (started) askBacklog(true); });

  /** Screenshot/test helpers, as the desktop's (mock, or ?debug=1: core/bridge.js drops `open` otherwise):
   *  ?open=menu (the context menu of ?select=…) | menu-background | options | share (of ?select=…) | drive (the first
   *  drive's Eigenschappen) | trash (the Prullenbak's Eigenschappen). */
  function debugOpen(what) {
    const items = selectedItems();
    if (what === 'menu' && items.length) showMenuAt(null, menus.itemMenu(items));
    else if (what === 'menu-background') showMenuAt(null, menus.backgroundMenu());
    else if (what === 'options') showOptions(app);
    else if (what === 'share' && items.length) showShare(app, items);
    else if (what === 'drive' && S.volumes.length) showDriveProperties(app, S.volumes[0]);
    else if (what === 'trash') showTrashProperties(app);
  }

  /** After a language-switch reload (saveForReload): the tabs as they were, instead of the URL's place. */
  function restoreAfterReload() {
    let saved = null;
    try {
      const raw = sessionStorage.getItem(RELOAD_KEY);
      sessionStorage.removeItem(RELOAD_KEY);
      saved = raw ? JSON.parse(raw) : null;
    } catch { saved = null; }
    if (!saved || !Array.isArray(saved.tabs) || !saved.tabs.length || Math.abs(Date.now() - (Number(saved.at) || 0)) > RELOAD_MAX_AGE) return false;
    const valid = (loc) => loc && typeof loc === 'object' && typeof loc.kind === 'string' && (loc.kind !== 'folder' || typeof loc.path === 'string');
    for (const s of saved.tabs.slice(0, 50)) {
      if (!valid(s?.loc)) continue;
      const tb = newTab(s.loc);
      const hist = Array.isArray(s.history) ? s.history.filter(valid).slice(-60) : [];
      const at = hist.findIndex((l) => locKey(l) === locKey(s.loc));
      if (at >= 0) { tb.history = hist; tb.hIndex = at; }
      tb.pendingSelect = Array.isArray(s.select) && s.select.length ? s.select.filter((p) => typeof p === 'string') : null;
      if (s.scroll && typeof s.scroll === 'object') tb.scrollMemo.set(locKey(s.loc), s.scroll);
      tabs.push(tb);
    }
    if (!tabs.length) return false;
    const first = tabs[Math.min(Math.max(0, Number(saved.active) || 0), tabs.length - 1)];
    first.wantFocus = true;
    activeId = first.id;
    applyLayout();
    navPane.render();
    tabStrip.render();
    for (const tb of tabs) load(tb);
    renderContent();
    updateChrome();
    syncWatches();
    started = true;
    for (const p of early.splice(0)) handleParams(p, { newTabIfBusy: true }).catch((e) => console.warn('[explorer] request failed', e));
    askBacklog(true);
    if ('ResizeObserver' in window) new ResizeObserver(reportSoon).observe(frame.titlebar);
    setTimeout(reportTitlebar, 50);
    return true;
  }

  /* ============================================================ public API */
  // defineProperties (not Object.assign) so the getters stay live
  Object.defineProperties(app, Object.getOwnPropertyDescriptors({
    S, frame, t, call, cfg, setCfg, dnd,
    get tabs() { return tabs; }, get activeId() { return activeId; }, get tab() { return active(); },
    get view() { return view; },
    openTab, closeTab, activateTab, navigate,
    exportTab, importTab, moveTabBefore, rollbackImportedTab,
    back: () => go(-1), forward: () => go(1), up: () => { const p = parentLoc(active().loc); if (p) navigate(p, { focus: true }); },
    refresh: () => { dirCache.clear(); loadVolumes(); loadTrashInfo(); reload(active()); },
    canBack, canForward, parentLoc, historyItems, recentLocations,
    locKey, locLabel, locIcon, locIconSpec, crumbsFor, addressText, resolveAddress, addressSuggestions, subfolderMenu,
    addressNotFound: (text) => Dialog.alert({ title: t('appName'), message: t('cantFind', { path: text }), icon: 'error' }),
    canFilter: () => ['folder', 'trash', 'search'].includes(active()?.loc.kind),
    setFilter(v) {
      const tab = active();
      if (!tab || !app.canFilter()) return;
      if ((tab.filter || '') === (v || '')) return;
      tab.filter = v || '';
      renderContent({ keep: true });
      updateStatus();
    },
    searchSubmit(v) { if ((v || '').trim()) openAlles(v.trim()); },
    selectedItems, setSelection, repaintSelection, registerPageItem: (it) => pageItems.set(it.path, it),
    visibleCount: () => (view ? view.order.length : null),
    focusView: () => { if (view) view.focus(); else contentHost.querySelector('.xp-pitem, .xp-page')?.focus(); },
    isCollapsed: (k) => collapsedSecs.has(k), setCollapsed: (k, v) => { if (v) collapsedSecs.add(k); else collapsedSecs.delete(k); },
    pinnedList, defaultFolders, storedOn, friendlyDir, itemName, origDir, listDirs, collectImages, loadVolumes,
    volLabel: U.volumeLabel, volIcon: U.volumeIcon, namedIcon: U.namedIcon,
    openItem, renameItem, setAttributes, dropInfo, dropTo, act, menus, toast, showError,
    privacyRecent: () => getSetting('privacy.recentFiles', true) !== false,
    setSetting: (path, v) => setSetting(path, v).catch((e) => showError(e)),
    reportTitlebar: reportSoon, showMenuAt,

    async start(params) {
      S.user = getShellInfo()?.user || {};
      await Promise.all([loadPlaces(), loadVolumes(), loadClipboard()]);
      loadTrashInfo();
      if (restoreAfterReload()) return;
      const loc0 = paramsToLoc(params) || { kind: 'home' };
      const { loc, select } = await resolveFileParam(loc0, selectFor(params, loc0));
      let ticket = typeof params?.tabTicket === 'string' ? params.tabTicket : null;
      const explicit = ['path', 'place', 'tabs', 'select', 'properties', 'tabTicket'].some((key) =>
        params[key] != null && params[key] !== '');
      let restored = null;
      if (!picker && !explicit && window.__win11shellHost?.runtime === 'rust') {
        const state = window.__win11shellHost.explorerSession;
        try {
          if (state?.version === 1 && Array.isArray(state.tabs) && state.tabs.length > 0 && state.tabs.length <= 20
              && Number.isInteger(state.active) && state.active >= 0 && state.active < state.tabs.length
              && new TextEncoder().encode(JSON.stringify(state)).length <= 1024 * 1024)
            restored = { tabs: state.tabs.map(hydrateTab), active: state.active };
        } catch (error) { console.warn('[explorer] saved tabs ignored', error); }
      }
      let first;
      if (ticket) {
        try {
          const claim = await call('explorer.tabClaim', { ticket });
          first = hydrateTab(claim.state);
        } catch (error) {
          await call('explorer.tabRelease', { ticket }).catch(() => {});
          ticket = null;
          showError(error);
        }
      }
      if (!first && restored) first = restored.tabs[restored.active];
      if (!first) {
        first = newTab(loc);
        first.pendingSelect = select;
        first.pendingProps = truthy(params?.properties) && !!select;
      }
      first.wantFocus = true;
      if (restored && !ticket) tabs.push(...restored.tabs);
      else tabs.push(first);
      activeId = first.id;
      applyLayout();
      navPane.render();
      tabStrip.render();
      const firstLoad = load(first);
      renderContent();
      updateChrome();
      syncWatches();
      if (ticket) {
        try {
          await firstLoad;
          if (first.status === 'error') throw first.error || new Error('The detached tab could not be loaded');
          await call('explorer.tabCommit', { ticket });
        } catch (error) {
          await call('explorer.tabRelease', { ticket }).catch(() => {});
          rollbackImportedTab(first.id, null);
          showError(error);
        }
      }
      extraTabs(params).catch((e) => console.warn('[explorer] tabs failed', e))
        .then(async () => {
          started = true;
          for (const p of early.splice(0)) await handleParams(p, { newTabIfBusy: true }).catch((e) => console.warn('[explorer] request failed', e));
          askBacklog(true);
        });
      if ('ResizeObserver' in window) new ResizeObserver(reportSoon).observe(frame.titlebar);
      setTimeout(reportTitlebar, 50);
      if (params?.open) setTimeout(() => debugOpen(String(params.open)), 900);
    },
  }));
  return app;
}
