// Win11 Shell — Taakbeheer (Task Manager). SPEC §11: export async function mountApp(root, params).
//
// Win11 22H2-style Task Manager with a left NavView (hamburger), a search box in the title
// bar, a per-page command bar, and pages that load on demand:
//   summary · processes · performance · storage · startup · services · details · settings
// Deep links: ?page=<id> (+ &pid=, &device=, &tab=, &volume=, &path=) and app.navigate {page, …}.
// Never stuck: every host call has a timeout (hostcall.js); while Win11 Shell does not answer, a banner above
// the page offers "Win11 Shell opnieuw starten" and the pages keep their last data (hostbanner.js).

import { createAppFrame } from '../appframe.js';
import { h, clear, loadCss, animate } from '../../core/dom.js';
import { SearchBox, Toast, EmptyState } from '../../core/components.js';
import { bridge } from './hostcall.js';
import { createHostBanner } from './hostbanner.js';
import { getSetting, getShellInfo } from '../../core/theme.js';
import { makeT, onLangChange } from '../../core/i18n.js';
import { createStore } from './store.js';
import { load, save } from './util.js';

const t = makeT({
  nl: {
    appTitle: 'Taakbeheer', search: 'Typ een naam of PID om te zoeken',
    summary: 'Samenvatting', processes: 'Processen', performance: 'Prestaties', storage: 'Opslag',
    startup: 'Opstart-apps', services: 'Services', details: 'Details', settings: 'Instellingen',
    loadFail: 'Deze pagina kon niet worden geladen', loadFailBody: 'Probeer het opnieuw. Helpt dat niet, sluit Taakbeheer dan en open het opnieuw.',
    newTaskFail: 'Zoeken kon niet worden geopend',
  },
  en: {
    appTitle: 'Task Manager', search: 'Type a name or PID to search',
    summary: 'Summary', processes: 'Processes', performance: 'Performance', storage: 'Storage',
    startup: 'Startup apps', services: 'Services', details: 'Details', settings: 'Settings',
    loadFail: 'This page couldn’t be loaded', loadFailBody: 'Try again. If that doesn’t help, close Task Manager and open it again.',
    newTaskFail: 'Couldn’t open search',
  },
});

const PAGE_IDS = ['summary', 'processes', 'performance', 'storage', 'startup', 'services', 'details', 'settings'];
const NAV_ICONS = {
  summary: 'summary', processes: 'apps', performance: 'chart', storage: 'storage',
  startup: 'startup', services: 'services', details: 'list', settings: 'settings',
};

/** Original Task Manager app icon (rounded tile with a pulse line). */
function appGlyph(size = 16) {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 16 16');
  s.setAttribute('width', size);
  s.setAttribute('height', size);
  s.setAttribute('aria-hidden', 'true');
  s.innerHTML = '<defs><linearGradient id="tmapp" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#38d39f"/><stop offset="1" stop-color="#0f8a6a"/></linearGradient></defs>'
    + '<rect x=".5" y="1.5" width="15" height="13" rx="3" fill="url(#tmapp)"/>'
    + '<path d="M2.6 9.4h2.3l1.4-3.3 2 5.4 1.6-3.6.8 1.5h2.7" fill="none" stroke="#fff" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"/>';
  return s;
}

/* Deep-link params from other modules (shell, Instellingen, health actions, Verkenner):
 *   page = summary|processes|performance|storage|startup|services|details|settings (+ a few friendly aliases)
 *   pid, sort = cpu|memory|disk|name (processes) · device (performance) · volume = id or mount, path,
 *   tab|section = explore|categories|large|cleanup (storage; 'cleanup' also as section/tab names like 'opruimen').
 * The page is inferred when only page-specific params are given. */
const PAGE_ALIASES = {
  apps: 'processes', process: 'processes', procs: 'processes', perf: 'performance', prestaties: 'performance',
  opslag: 'storage', disk: 'storage', disks: 'storage', cleanup: 'storage', opruimen: 'storage', 'free-space': 'storage',
  autostart: 'startup', 'startup-apps': 'startup', opstart: 'startup', service: 'services', processen: 'processes',
  samenvatting: 'summary', home: 'summary', instellingen: 'settings', options: 'settings',
};
const TAB_ALIASES = {
  explore: 'explore', verkennen: 'explore', overview: 'explore', categories: 'categories', categorieen: 'categories', 'categorieën': 'categories',
  large: 'large', largefiles: 'large', 'large-files': 'large', 'grote-bestanden': 'large', grotebestanden: 'large',
  cleanup: 'cleanup', clean: 'cleanup', opruimen: 'cleanup', 'free-space': 'cleanup', freeup: 'cleanup',
};
function normParams(raw = {}) {
  const p = { ...(raw || {}) };
  let page = typeof p.page === 'string' ? p.page.toLowerCase() : '';
  if (page && !PAGE_IDS.includes(page)) {
    if ((page === 'cleanup' || page === 'opruimen') && !p.tab && !p.section) p.tab = 'cleanup';
    page = PAGE_ALIASES[page] || '';
  }
  const tab = String(p.tab || p.section || '').toLowerCase();
  if (tab) p.tab = TAB_ALIASES[tab] || tab;
  if (!page) {
    if (p.tab && Object.values(TAB_ALIASES).includes(p.tab)) page = 'storage';
    else if (p.volume || (p.path && !p.pid)) page = 'storage';
    else if (p.device) page = 'performance';
    else if (p.sort) page = 'processes';
    else if (p.pid) page = 'details';
  }
  if (page) p.page = page; else delete p.page;
  return p;
}

export async function mountApp(root, params = {}) {
  await loadCss('taskmgr');
  const store = createStore();
  const info = getShellInfo() || {};

  const navItems = PAGE_IDS.filter((id) => id !== 'settings').map((id) => ({ id, label: t(id), icon: NAV_ICONS[id] }));
  const footerItems = [{ id: 'settings', label: t('settings'), icon: NAV_ICONS.settings }];
  params = normParams(params);
  let initial = PAGE_IDS.includes(params.page) ? params.page : getSetting('taskmgr.defaultPage', 'summary');
  if (!PAGE_IDS.includes(initial)) initial = 'summary';

  const search = SearchBox({ placeholder: t('search'), onInput: (q) => onSearch(q), onSubmit: (q) => onSearch(q, true), debounce: 0 });
  search.classList.add('tm-search');

  const frame = createAppFrame({
    app: 'taskmgr', title: t('appTitle'), icon: appGlyph(16),
    nav: { items: navItems, selected: initial, footerItems, paneToggle: true },
    titlebarContent: search, layer: true, compactBelow: 0,
  });
  frame.el.classList.add('tm-app');
  root.append(frame.el);

  /* ---------------------------------------------------- nav pane (hamburger) */
  let userCompact = load('navCompact', false);
  const nav = frame.nav;
  const applyCompact = () => nav.setCompact(userCompact || frame.el.clientWidth < 1000);
  nav.addEventListener('click', (e) => {
    if (!e.target.closest('.fl-nav__toggle')) return;
    const wide = frame.el.clientWidth >= 1000;
    if (nav.classList.contains('is-compact') && wide) {
      e.stopImmediatePropagation();
      userCompact = false; save('navCompact', false); applyCompact();
    } else if (!nav.classList.contains('is-compact')) {
      userCompact = true; save('navCompact', true);
    }
  }, true);
  if ('ResizeObserver' in window) new ResizeObserver(applyCompact).observe(frame.el);
  applyCompact();

  /* ----------------------------------------------------------------- pages */
  const instances = new Map();
  let current = null;
  let currentId = null;
  let query = '';

  const ctx = {
    store, frame, bridge,
    user: info.user?.name || '',
    get query() { return query; },
    navigate: (id, p = {}) => go(id, p),
    newTask() {
      bridge.call('shell.command', { command: 'toggle-search', args: {} })
        .catch((e) => Toast.show({ title: t('newTaskFail'), message: e.message, severity: 'error' }));
    },
    setSearch(q) { search.setValue(q); onSearch(q); },
  };
  const banner = createHostBanner(ctx);

  async function getPage(id) {
    if (instances.has(id)) return instances.get(id);
    const mod = await import(`./pages/${id}.js`);
    const page = mod.createPage(ctx);
    const header = h('header', { class: 'tm-page__header' },
      h('h1', { class: 'tm-page__title' }, page.title),
      h('div', { class: 'tm-cmdbar', role: 'toolbar', 'aria-label': page.title }, page.commands || []));
    page.headerEl = header;
    page.titleEl = header.firstChild;
    page.cmdEl = header.lastChild;
    page.wrap = h('section', { class: ['tm-page', `tm-page--${id}`], dataset: { page: id } },
      header, h('div', { class: ['tm-page__body', page.fill !== false && page.el.classList.contains('tm-page__fill') && 'is-fill'] }, page.el));
    instances.set(id, page);
    return page;
  }

  let navSeq = 0;
  async function show(id, p = {}) {
    const seq = ++navSeq;
    let page;
    try {
      page = await getPage(id);
    } catch (e) {
      console.error('[taskmgr] page failed to load', id, e);
      current?.unmount?.();
      current = null; currentId = id;
      frame.setContent(h('div', { class: 'tm-page' }, EmptyState({ icon: 'warning', title: t('loadFail'), message: t('loadFailBody'), action: { label: 'OK', onClick: () => show(id) } })));
      banner.place(frame);
      return;
    }
    if (seq !== navSeq) return;
    if (current && current !== page) current.unmount?.();
    const same = current === page;
    current = page; currentId = id;
    if (!same) {
      frame.setContent(page.wrap);
      banner.place(frame);
      page.mount?.(p);
      if (page.onSearch) page.onSearch(query);
      animate(page.wrap, [{ opacity: 0, transform: 'translateY(28px)' }, { opacity: 1, transform: 'none' }], { duration: 300, easing: 'cubic-bezier(0,0,0,1)' });
    } else page.navigate?.(p);
    document.title = `${t('appTitle')} – ${page.title}`;
  }

  function go(id, p = {}) {
    if (!PAGE_IDS.includes(id)) return;
    if (id === currentId) { show(id, p); return; }
    pendingParams = p;
    frame.navigate(id);
  }
  let pendingParams = null;
  frame.onNavigate((id) => { const p = pendingParams || {}; pendingParams = null; show(id, p); });
  frame.onAppNavigate((raw = {}) => {
    const p = normParams(raw);
    if (p.page && PAGE_IDS.includes(p.page)) go(p.page, p);
  });

  function onSearch(q, submit = false) {
    query = q || '';
    if (!current?.onSearch) {
      if (!query && !submit) return;
      go('processes');
      return;
    }
    current.onSearch(query);
  }

  // live updates pushed by the host
  bridge.on('storage.volumesChanged', (d) => { if (Array.isArray(d?.volumes)) store.inject('volumes', d.volumes); });

  /* ------------------------------------------------------------- keyboard */
  document.addEventListener('keydown', (e) => {
    if (document.documentElement.classList.contains('has-dialog')) return;
    const k = e.key;
    if ((e.ctrlKey && (k === 'f' || k === 'F')) || (e.ctrlKey && k === 'e')) { e.preventDefault(); search.input.focus(); search.input.select(); return; }
    if (k === 'Escape' && document.activeElement === search.input && search.input.value) { e.preventDefault(); search.setValue(''); onSearch(''); return; }
    if (k === 'F5') { e.preventDefault(); (current?.refresh ? current.refresh() : store.refresh()); return; }
    if (e.ctrlKey && k === 'Tab') {
      e.preventDefault();
      const i = PAGE_IDS.indexOf(currentId);
      go(PAGE_IDS[(i + (e.shiftKey ? PAGE_IDS.length - 1 : 1)) % PAGE_IDS.length]);
    }
  });

  // language switch at runtime: rebuild labels and pages
  onLangChange(() => {
    frame.setTitle(t('appTitle'));
    search.input.placeholder = t('search');
    search.input.setAttribute('aria-label', t('search'));
    nav.setItems(PAGE_IDS.filter((id) => id !== 'settings').map((id) => ({ id, label: t(id), icon: NAV_ICONS[id] })),
      [{ id: 'settings', label: t('settings'), icon: NAV_ICONS.settings }]);
    nav.select(currentId, { animated: false });
    applyCompact();
    const id = currentId;
    current?.unmount?.();
    current = null;
    instances.clear();
    store.langChanged();   // Opstart-apps, Services, Opslag: the host's names and descriptions in the new language
    if (id) show(id);
  });

  await show(initial, params);
  if (params.open) debugOpen(String(params.open), params);
  return frame;

  /** Screenshot/test helpers (mock, or ?debug=1: core/bridge.js drops `open` otherwise): the dialogs of an app row,
   *  ?open=end|restart|efficiency|properties|menu|restart-shell [&target=<part of its id or name>, default the first].
   *  Each opens a menu or a question that still needs a click; restart-shell has none (it restarts at once), so it
   *  runs in mock mode only (the mock host only logs it), never against a real Win11 Shell. */
  async function debugOpen(what, p) {
    const acts = await import('./actions.js');
    if (what === 'restart-shell') { if (bridge.isMock) acts.restartShell(ctx); return; }
    const { buildProcessGroups } = await import('./pages/processes.js');
    const data = await store.fetch('processes', 60000);
    const rows = buildProcessGroups(data).flatMap((g) => g.rows);
    const want = String(p.target || '').toLowerCase();
    const row = (want && rows.find((r) => `${r.key || ''} ${r.name || ''}`.toLowerCase().includes(want))) || rows[0];
    if (!row) return;
    setTimeout(() => {
      if (what === 'end') acts.endTask(ctx, row);
      else if (what === 'restart') acts.restartTask(ctx, row);
      else if (what === 'efficiency') acts.setEfficiency(ctx, row, true);
      else if (what === 'properties') acts.showProperties(row);
      else if (what === 'menu') {
        const m = acts.processMenu(ctx, [row]);
        if (m) import('../../core/contextmenu.js').then(({ showContextMenu }) => showContextMenu({ x: 420, y: 260, ...m }));
      }
    }, 400);
  }
}
