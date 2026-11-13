// Win11 Shell — Alles: instant file name search (Everything-style), Win11 Fluent look.
// SPEC §6.13 / §11: export async function mountApp(root, params)
//
//   ?surface=app&app=alles&query=foto      deep link (also app.navigate {params:{query}})

import { createAppFrame } from '../appframe.js';
import { h, clear, loadCss, debounce, throttle } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { Dropdown, TextBox, ProgressBar, ProgressRing, HyperlinkButton, InfoBar, Toast } from '../../core/components.js';
import { showContextMenu } from '../../core/contextmenu.js';
import { getSettings, getSetting, setSetting, onSettings } from '../../core/theme.js';
import { number, relTime, bytes } from '../../core/format.js';
import { onLangChange } from '../../core/i18n.js';
import { t, FILTERS } from './i18n.js';
import { ResultsTable, PAGE } from './table.js';
import { PreviewPane, TipsPane } from './panes.js';
import { openItem, openContaining, openInExplorer, copyPaths, showProperties, trashItems } from './actions.js';
import { openOptions } from './options.js';

const VIEW_KEY = 'win11shell.alles.view';
const COPY_MAX = 10000;
// what a language switch keeps over the reload of this window (sessionStorage: this window only)
const RELOAD_KEY = 'win11shell.alles.langReload';
const RELOAD_MAX_AGE = 15000;

/** One sentence with a link inside: t(key, {link: LINK}) → [before, el, after], wherever the language puts it. */
const LINK = '\u0000';
function withLink(text, el) {
  const i = text.indexOf(LINK);
  if (i < 0) return [text, ' ', el];
  return [text.slice(0, i), el, text.slice(i + LINK.length)].filter((x) => x !== '');
}

/** The query, filter, sort, focused row and scroll position saved just before a language-switch reload (once). */
function takeReloadState() {
  try {
    const raw = sessionStorage.getItem(RELOAD_KEY);
    sessionStorage.removeItem(RELOAD_KEY);
    const s = raw ? JSON.parse(raw) : null;
    return s && typeof s.query === 'string' && Math.abs(Date.now() - (Number(s.at) || 0)) < RELOAD_MAX_AGE ? s : null;
  } catch { return null; }
}

function loadView() {
  try { return JSON.parse(localStorage.getItem(VIEW_KEY) || '{}') || {}; } catch { return {}; }
}
function saveView(v) {
  try { localStorage.setItem(VIEW_KEY, JSON.stringify(v)); } catch { /* ignore */ }
}

/** Original app logo: warm tile with a magnifier (16 px in the title bar). */
export function allesLogo(size = 16) {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 16 16');
  s.setAttribute('width', String(size));
  s.setAttribute('height', String(size));
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('class', 'alles-logo');
  s.innerHTML = '<defs><linearGradient id="allesLogoG" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffb347"/><stop offset="1" stop-color="#ec5b13"/></linearGradient></defs>'
    + '<rect x="1" y="1" width="14" height="14" rx="3.5" fill="url(#allesLogoG)"/>'
    + '<circle cx="7.1" cy="7.1" r="3.1" fill="none" stroke="#fff" stroke-width="1.6"/>'
    + '<path d="M9.4 9.4l2.9 2.9" stroke="#fff" stroke-width="1.8" stroke-linecap="round"/>';
  return s;
}

/* small inline glyphs for the toggle buttons */
function textGlyph(text, cls = '') { return h('span', { class: ['alles-tglyph', cls], 'aria-hidden': 'true' }, text); }
function zipGlyph() {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 20 20'); s.setAttribute('width', '16'); s.setAttribute('height', '16');
  s.setAttribute('class', 'fl-icon'); s.setAttribute('fill', 'none'); s.setAttribute('stroke', 'currentColor');
  s.setAttribute('stroke-linecap', 'round'); s.setAttribute('stroke-linejoin', 'round');
  s.innerHTML = '<rect x="3.5" y="2.75" width="13" height="14.5" rx="2"/><path d="M10 2.75v2M10 6.25v1.5M10 9.25v1.5"/><rect x="8.5" y="11.75" width="3" height="3" rx=".8"/>';
  return s;
}
const FILTER_ICONS = { all: 'grid', audio: 'music', compressed: zipGlyph, document: 'document', executable: 'app', folder: 'folder', picture: 'image', video: 'video' };

/** Deep-link params: {query} (also q / search from other callers), {path} → in:"path", {filter}. */
function withQuery(p) {
  const out = { ...(p || {}) };
  if (typeof out.query !== 'string') {
    const alt = [out.q, out.search, out.text].find((x) => typeof x === 'string');
    if (alt != null) out.query = alt;
  }
  return out;
}

/* ------------------------------------------------------------------------ */
export async function mountApp(root, params = {}) {
  params = withQuery(params);
  loadCss('alles');
  const view = loadView();
  const s0 = getSettings().alles || {};
  const kept = takeReloadState();   // a language switch reloaded this window: the user's search comes back
  if (kept) Object.assign(params, { query: kept.query, filter: kept.filter });
  const state = {
    query: typeof params.query === 'string' ? params.query : (typeof params.path === 'string' && params.path ? `in:"${params.path}" ` : ''),
    filter: FILTERS.includes(params.filter) ? params.filter : 'all',
    matchCase: !!s0.matchCase, matchPath: !!s0.matchPath, regex: !!s0.regex, wholeWord: !!s0.wholeWord,
    sort: ['name', 'path', 'size', 'modified', 'type'].includes(view.sort) ? view.sort : 'name',
    desc: !!view.desc,
    pane: view.pane === 'preview' || view.pane === 'tips' ? view.pane : null,
    status: null,
    result: null,
  };
  // the sort of the moment of the switch (a quick chip sets one without saving it as the view)
  if (kept && ['name', 'path', 'size', 'modified', 'type'].includes(kept.sort)) { state.sort = kept.sort; state.desc = !!kept.desc; }
  let seq = 0;
  let currentArgs = null;
  let overlay = null;
  let updateOverlay = null;
  let indexKind = null;
  let updateIndex = null;

  /* ---- title bar: big search box ------------------------------------------- */
  const search = TextBox({
    placeholder: t('searchPlaceholder'), value: state.query, leading: 'search', ariaLabel: t('searchLabel'),
    onInput: (v) => { state.query = v; queryPending = true; queryChanged(); },
  });
  search.classList.add('alles-search');
  search.input.setAttribute('spellcheck', 'false');
  search.input.setAttribute('autocapitalize', 'off');

  const frame = createAppFrame({ app: 'alles', title: t('appTitle'), icon: allesLogo(16), titlebarContent: search });
  frame.titlebar.classList.add('is-tall', 'alles-titlebar');
  frame.el.classList.add('alles-window');

  /* ---- command bar ------------------------------------------------------------ */
  const filterDd = Dropdown({
    options: FILTERS.map((f) => ({ value: f, label: t(`f_${f}`), icon: typeof FILTER_ICONS[f] === 'function' ? FILTER_ICONS[f]() : FILTER_ICONS[f] })),
    value: state.filter, ariaLabel: t('filterLabel'), width: 168,
    onChange: (v) => { state.filter = v; runQuery(); },
  });
  filterDd.classList?.add('alles-filter');

  function toggle(key, glyph, label, tip) {
    const b = h('button', {
      type: 'button', class: 'fl-btn fl-btn--subtle alles-toggle', 'aria-pressed': String(!!state[key]),
      dataset: { tooltip: tip }, 'aria-label': tip,
    }, glyph, h('span', { class: 'alles-toggle__label' }, label));
    b.addEventListener('click', () => setOption(key, !state[key]));
    b.sync = () => { b.setAttribute('aria-pressed', String(!!state[key])); b.classList.toggle('is-checked', !!state[key]); };
    b.sync();
    return b;
  }
  const toggles = {
    matchCase: toggle('matchCase', textGlyph('Aa'), t('matchCase'), t('matchCaseTip')),
    wholeWord: toggle('wholeWord', textGlyph('ab', 'is-word'), t('wholeWord'), t('wholeWordTip')),
    matchPath: toggle('matchPath', icon('folder-open', { size: 16 }), t('matchPath'), t('matchPathTip')),
    regex: toggle('regex', textGlyph('.*', 'is-mono'), t('regex'), t('regexTip')),
  };

  function cmdButton({ ic, label, onClick, chevron = false, pressed = null, tip }) {
    const b = h('button', {
      type: 'button', class: 'fl-btn fl-btn--subtle alles-cmdbtn', dataset: tip ? { tooltip: tip } : {},
      'aria-pressed': pressed == null ? null : String(pressed),
    }, typeof ic === 'string' ? icon(ic, { size: 16 }) : ic, h('span', { class: 'alles-cmdbtn__label' }, label),
    chevron ? h('span', { class: 'alles-cmdbtn__chev' }, icon('chevron-down', { size: 12 })) : null);
    b.addEventListener('click', (e) => onClick(e, b));
    return b;
  }
  const sortBtn = cmdButton({ ic: 'sort', label: t('sort'), chevron: true, onClick: (e, b) => openSortMenu(b) });
  const tipsBtn = cmdButton({ ic: 'help', label: t('tips'), pressed: state.pane === 'tips', onClick: () => setPane(state.pane === 'tips' ? null : 'tips') });
  const previewBtn = cmdButton({ ic: 'view', label: t('preview'), pressed: state.pane === 'preview', onClick: () => setPane(state.pane === 'preview' ? null : 'preview') });
  const optionsBtn = cmdButton({ ic: 'settings', label: t('options'), onClick: () => openOptions({ status: state.status }).then(() => { refreshStatus(); }) });

  const cmdbar = h('div', { class: 'alles-cmd', role: 'toolbar', 'aria-label': t('appTitle') },
    filterDd, h('span', { class: 'alles-cmd__sep', 'aria-hidden': 'true' }),
    h('div', { class: 'alles-cmd__group' }, Object.values(toggles)),
    h('span', { class: 'alles-cmd__spacer' }),
    sortBtn, h('span', { class: 'alles-cmd__sep', 'aria-hidden': 'true' }), tipsBtn, previewBtn, optionsBtn);

  /* ---- results ------------------------------------------------------------- */
  const table = ResultsTable({
    onFetch: async (offset, limit) => {
      const args = currentArgs;
      if (!args) return [];
      const r = await bridge.call('index.search', { ...args, offset, limit });
      return r.items;
    },
    onOpen: (item) => openItem(item),
    onContextMenu: (pt, index) => openRowMenu(pt, index),
    onSort: (by, col) => {
      if (by === state.sort) setSort(by, !state.desc);
      else setSort(by, !!col?.firstDesc);
    },
    onSelection: (n) => { renderCounts(); updatePreviewSelection(n); },
    onFocusItem: (item) => schedulePreview(item),
    onType: (ch, e) => {
      e.preventDefault();
      search.focus();
      const inp = search.input;
      const v = inp.value;
      inp.value = v + ch;
      inp.dispatchEvent(new Event('input', { bubbles: true }));
    },
    onKey: (e) => tableKeys(e),
  });

  const overlayHost = h('div', { class: 'alles-overlay', hidden: true });
  const results = h('div', { class: 'alles-results' }, table.el, overlayHost);
  const preview = PreviewPane({ onOpen: openItem, onOpenFolder: openContaining });
  const tips = TipsPane({ onExample: (q) => setQuery(q, { focus: true }), onClose: () => setPane(null) });
  const paneHost = h('div', { class: 'alles-panehost' });

  const countEl = h('span', { class: 'alles-status__count' });
  const selEl = h('span', { class: 'alles-status__sel' });
  const msEl = h('span', { class: 'alles-status__ms' });
  const idxEl = h('div', { class: 'alles-status__index' });
  const statusbar = h('footer', { class: 'alles-status' },
    h('div', { class: 'alles-status__left', 'aria-live': 'polite' }, countEl, selEl, msEl), idxEl);

  const layer = h('div', { class: 'alles-layer' }, h('div', { class: 'alles-main' }, results, paneHost), statusbar);
  const content = h('div', { class: 'alles' }, cmdbar, layer);
  frame.setContent(content);
  root.append(frame.el);

  /* ---- panes --------------------------------------------------------------- */
  function setPane(p) {
    state.pane = p;
    clear(paneHost);
    if (p === 'preview') { paneHost.append(preview.el); schedulePreview(table.focusedItem(), true); }
    if (p === 'tips') paneHost.append(tips.el);
    paneHost.hidden = !p;
    tipsBtn.setAttribute('aria-pressed', String(p === 'tips'));
    previewBtn.setAttribute('aria-pressed', String(p === 'preview'));
    tipsBtn.classList.toggle('is-checked', p === 'tips');
    previewBtn.classList.toggle('is-checked', p === 'preview');
    saveView({ sort: state.sort, desc: state.desc, pane: state.pane });
  }

  let pvTimer = 0;
  function schedulePreview(item, now = false) {
    if (state.pane !== 'preview') return;
    clearTimeout(pvTimer);
    const run = () => {
      const n = table.selectedCount();
      if (n > 1) updatePreviewSelection(n);
      else preview.show(item || table.focusedItem());
    };
    if (now) run(); else pvTimer = setTimeout(run, 70);
  }
  function updatePreviewSelection(n) {
    if (state.pane !== 'preview') return;
    if (n > 1) {
      const idx = table.selectedIndices(5000);
      let size = 0;
      let known = true;
      for (const i of idx) { const it = table.itemAt(i); if (it) size += it.size || 0; else known = false; }
      preview.showMany(n, known && idx.length === n ? size : null);
    } else schedulePreview(table.focusedItem());
  }

  /* ---- options / sorting ------------------------------------------------------ */
  function setOption(key, v) {
    state[key] = !!v;
    toggles[key].sync();
    setSetting(`alles.${key}`, !!v).catch(() => {});
    if (state.query.trim()) runQuery();
  }

  function setSort(by, desc) {
    state.sort = by;
    state.desc = !!desc;
    table.setSort(by, state.desc);
    saveView({ sort: state.sort, desc: state.desc, pane: state.pane });
    runQuery({ keepSelection: false });
  }

  function openSortMenu(anchor) {
    const items = ['name', 'path', 'size', 'modified', 'type'].map((k) => ({
      label: t(`col_${k}`), radio: true, checked: state.sort === k,
      action: () => setSort(k, k === state.sort ? state.desc : (k === 'size' || k === 'modified')),
    }));
    showContextMenu({
      anchor, align: 'start',
      items: [...items, { separator: true },
        { label: t('ascending'), radio: true, checked: !state.desc, action: () => setSort(state.sort, false) },
        { label: t('descending'), radio: true, checked: state.desc, action: () => setSort(state.sort, true) }],
    });
  }

  /* ---- searching ------------------------------------------------------------- */
  function baseArgs() {
    return {
      query: state.query, filter: state.filter, sort: state.sort, desc: state.desc,
      matchCase: state.matchCase, matchPath: state.matchPath, regex: state.regex, wholeWord: state.wholeWord,
      channel: 'alles',
    };
  }

  const queryChanged = debounce(() => runQuery(), 30);
  // Enter typed before the results for the latest letters arrived: apply it once they are in
  let queryPending = false;
  let pendingEnter = null;
  function enterInSearch(ctrl) {
    if (!table.total) return false;
    if (ctrl && table.itemAt(0)) { openContaining(table.focusedItem() || table.itemAt(0)); return true; }
    table.focus();
    if (table.focusIndex() < 0) table.selectFirst();
    return true;
  }

  async function runQuery({ keepScroll = false, keepSelection = false } = {}) {
    const my = ++seq;
    const q = state.query.trim();
    if (!q && state.filter === 'all') {
      queryPending = false;
      pendingEnter = null;
      currentArgs = null;
      state.result = null;
      table.setHighlight(null);
      table.reset(0, []);
      showOverlay(isFirstRun() ? 'first' : 'welcome');
      renderCounts();
      return;
    }
    if (isFirstRun()) { showOverlay('first'); }
    const args = baseArgs();
    try {
      const r = await bridge.call('index.search', { ...args, offset: 0, limit: PAGE });
      if (my !== seq) return;
      queryPending = false;
      currentArgs = args;
      state.result = r;
      table.setHighlight(r.highlight ? { ...r.highlight, path: state.matchPath || /(^|\s|!)(path|pad):/i.test(q) } : null);
      table.reset(r.total, r.items, { keepScroll, keepSelection });
      if (isFirstRun()) showOverlay('first');
      else if (!r.total) showOverlay('empty', { q });
      else showOverlay(null);
      renderCounts();
      if (pendingEnter) { const p = pendingEnter; pendingEnter = null; enterInSearch(p.ctrl); }
    } catch (e) {
      if (my !== seq) return;
      queryPending = false;
      pendingEnter = null;
      if (e?.code === 'cancelled') return;
      const typing = /[:<>=.;,|!"]$/.test(q) || /[:<>=](\.\.)?$/.test(q);
      if ((e?.code === 'bad_query' || e?.code === 'bad_regex') && typing && state.result) return;   // still typing a modifier
      currentArgs = null;
      state.result = null;
      table.reset(0, []);
      if (e?.code === 'bad_query' || e?.code === 'bad_regex') showOverlay('queryError', { message: e.message });
      else showOverlay('unavailable', { message: e?.code === 'index_unavailable' || e?.code === 'index_restarting' ? '' : e?.message });
      renderCounts();
    }
  }

  function setQuery(q, { focus = false } = {}) {
    state.query = q;
    search.setValue(q);
    runQuery();
    if (focus) { search.focus(); try { search.input.setSelectionRange(q.length, q.length); } catch { /* ignore */ } }
  }

  /* ---- overlays (welcome / first run / nothing found / errors) ----------------- */
  function isFirstRun() {
    const st = state.status;
    return !!st && st.state === 'building' && !(st.files || st.folders);
  }

  function chip(ic, label, onClick) {
    const b = h('button', { type: 'button', class: 'alles-chip' }, icon(ic, { size: 16 }), h('span', null, label));
    b.addEventListener('click', onClick);
    return b;
  }

  function welcomeArt() {
    const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    s.setAttribute('viewBox', '0 0 120 96'); s.setAttribute('width', '120'); s.setAttribute('height', '96');
    s.setAttribute('class', 'alles-art'); s.setAttribute('aria-hidden', 'true');
    s.innerHTML = '<defs><linearGradient id="aw1" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#ffd27a"/><stop offset="1" stop-color="#f4a62a"/></linearGradient>'
      + '<linearGradient id="aw2" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#ffb347"/><stop offset="1" stop-color="#ec5b13"/></linearGradient></defs>'
      + '<rect class="aw-card" x="14" y="26" width="58" height="46" rx="6"/>'
      + '<path d="M14 34a6 6 0 0 1 6-6h14l5 5h27a6 6 0 0 1 6 6v27a6 6 0 0 1-6 6H20a6 6 0 0 1-6-6Z" fill="url(#aw1)"/>'
      + '<rect class="aw-doc" x="44" y="14" width="34" height="44" rx="4"/><path class="aw-line" d="M51 26h20M51 33h20M51 40h13"/>'
      + '<circle cx="80" cy="56" r="17" fill="url(#aw2)"/><circle cx="80" cy="56" r="9.5" fill="none" stroke="#fff" stroke-width="3.6"/>'
      + '<path d="M87 63l9 9" stroke="#fff" stroke-width="4.4" stroke-linecap="round"/>'
      + '<path class="aw-spark" d="M101 22v8M97 26h8M28 12v6M25 15h6"/>';
    return s;
  }

  function showOverlay(kind, info = {}) {
    // Status events update the existing panel. Replacing it would replay its
    // entrance animation and reset the progress bar on every scanned directory.
    if (kind === overlay && updateOverlay) { updateOverlay(); return; }
    overlay = kind;
    updateOverlay = null;
    results.classList.toggle('has-overlay', !!kind && kind !== 'empty');
    results.classList.toggle('has-empty', kind === 'empty');
    if (!kind) { overlayHost.hidden = true; clear(overlayHost); return; }
    overlayHost.hidden = false;
    clear(overlayHost);
    const st = state.status || {};
    if (kind === 'welcome') {
      const body = h('p', { class: 'alles-state__body' });
      updateOverlay = () => {
        const st = state.status || {};
        const n = (st.files || 0) + (st.folders || 0);
        body.textContent = n ? t('welcomeBody', { n, c: number(n) }) : t('welcomeBodyNoCount');
      };
      const quick = h('div', { class: 'alles-quick' },
        chip('image', t('q_pictures'), () => { state.filter = 'picture'; filterDd.setValue('picture'); setSort('modified', true); }),
        chip('music', t('q_music'), () => { state.filter = 'audio'; filterDd.setValue('audio'); runQuery(); }),
        chip('video', t('q_videos'), () => { state.filter = 'video'; filterDd.setValue('video'); runQuery(); }),
        chip('document', t('q_documents'), () => { state.filter = 'document'; filterDd.setValue('document'); setSort('modified', true); }),
        chip('disk', t('q_big'), () => { state.sort = 'size'; state.desc = true; table.setSort('size', true); setQuery('file: size:>1gb'); }),
        chip('clock', t('q_today'), () => { state.sort = 'modified'; state.desc = true; table.setSort('modified', true); setQuery(t('q_todayQuery')); }),
        chip('download', t('q_downloads'), () => { state.sort = 'modified'; state.desc = true; table.setSort('modified', true); setQuery('parent:~/Downloads'); }));
      overlayHost.append(h('div', { class: 'alles-state alles-welcome' },
        welcomeArt(),
        h('h2', { class: 'alles-state__title' }, t('welcomeTitle')),
        body,
        h('div', { class: 'alles-state__label' }, t('quickTitle')),
        quick,
        HyperlinkButton({ label: t('showTips'), icon: 'help', onClick: () => setPane('tips') })));
    } else if (kind === 'first') {
      const bar = ProgressBar({ indeterminate: true, ariaLabel: t('firstTitle') });
      bar.classList.add('alles-first__bar');
      const found = h('span', { class: 'alles-first__found' });
      const percent = h('span', { class: 'alles-first__pct' });
      const current = h('span', { dir: 'ltr' });
      const path = h('p', { class: 'alles-first__path' }, current);
      updateOverlay = () => {
        const pr = state.status?.progress || {};
        if (pr.percent != null) bar.setValue(pr.percent); else bar.setIndeterminate(true);
        found.textContent = t('firstFound', { n: pr.done || 0, c: number(pr.done || 0) });
        percent.textContent = pr.percent != null ? `${pr.percent}%` : '';
        percent.hidden = pr.percent == null;
        // Retain one line while the worker moves between directories. The existing
        // nowrap/ellipsis rule keeps long paths from changing the panel's height.
        current.textContent = pr.current || '\u00a0';
        path.style.visibility = pr.current ? '' : 'hidden';
        if (pr.current) path.dataset.tooltip = pr.current; else delete path.dataset.tooltip;
      };
      overlayHost.append(h('div', { class: 'alles-state alles-first' },
        h('div', { class: 'alles-first__art' }, welcomeArt()),
        h('h2', { class: 'alles-state__title' }, t('firstTitle')),
        h('p', { class: 'alles-state__body' }, `${t('firstBody')} ${t('firstBody2')}`),
        bar,
        h('p', { class: 'alles-first__count' },
          found, percent),
        path,
        h('p', { class: 'alles-first__note' }, icon('info', { size: 14 }), h('span', null, window.__win11shellHost?.runtime === 'rust'
          ? t('firstNoteNative') : t('firstNote')))));
    } else if (kind === 'empty') {
      const hints = [h('li', null, t('hint1'))];
      if (!state.matchPath && !state.regex) {
        hints.push(h('li', null, t('hint2'), ' ', HyperlinkButton({ label: t('turnOnPath'), onClick: () => setOption('matchPath', true) })));
      }
      if (state.filter !== 'all') {
        hints.push(h('li', null, t('hint3'), ' ', HyperlinkButton({ label: t('f_all'), onClick: () => { state.filter = 'all'; filterDd.setValue('all'); runQuery(); } })));
      }
      if (!getSetting('alles.showHidden', false)) {
        hints.push(h('li', null, ...withLink(t('hintHidden', { link: LINK }), HyperlinkButton({ label: t('options'), onClick: () => optionsBtn.click() }))));
      }
      overlayHost.append(h('div', { class: 'alles-state alles-empty' },
        h('div', { class: 'alles-empty__icon' }, icon('search', { size: 40 })),
        h('h2', { class: 'alles-state__title is-small' }, t('noResults')),
        h('p', { class: 'alles-state__body' }, state.filter !== 'all'
          ? t('noResultsFilter', { q: info.q || state.query, f: t(`f_${state.filter}`) })
          : t('noResultsBody', { q: info.q || state.query })),
        h('ul', { class: 'alles-empty__hints' }, hints)));
    } else if (kind === 'queryError') {
      overlayHost.append(h('div', { class: 'alles-state alles-qerror' },
        InfoBar({ severity: 'warning', title: t('queryError'), message: info.message || '', closable: false,
          action: { label: t('tips'), onClick: () => setPane('tips') } })));
    } else if (kind === 'unavailable') {
      overlayHost.append(h('div', { class: 'alles-state alles-qerror' },
        InfoBar({ severity: 'error', title: t('unavailable'), message: info.message || t('unavailableBody'), closable: false,
          action: { label: t('retry'), onClick: () => { refreshStatus(); runQuery(); } } })));
    }
    updateOverlay?.();
  }

  /* ---- status bar --------------------------------------------------------------- */
  function renderCounts() {
    const r = state.result;
    const st = state.status || {};
    let n;
    if (r) n = r.total;
    else if (!state.query.trim() && state.filter === 'all') n = (st.files || 0) + (st.folders || 0);
    else n = 0;
    countEl.textContent = t('objects', { n, c: number(n) });
    const sel = table.selectedCount();
    selEl.textContent = sel ? t('selected', { n: number(sel), count: sel }) : '';
    selEl.hidden = !sel;
    msEl.textContent = r && r.tookMs != null ? t('tookMs', { ms: number(r.tookMs, r.tookMs < 10 ? 1 : 0) }) : '';
    msEl.hidden = !msEl.textContent;
    selEl.classList.toggle('is-sep', !selEl.hidden);
    msEl.classList.toggle('is-sep', !msEl.hidden);
  }

  function renderIndexState() {
    const st = state.status;
    const kind = !st ? 'none' : st.state === 'building'
      ? `building:${st.progress?.percent == null ? 'indeterminate' : 'determinate'}`
      : st.refreshing ? 'refreshing' : st.state === 'ready'
        ? `ready:${!!(st.liveUpdate && st.watching)}` : st.state === 'error' ? 'error' : 'idle';
    if (kind === indexKind) { updateIndex?.(); return; }
    indexKind = kind;
    updateIndex = null;
    clear(idxEl);
    if (!st) return;
    let node;
    if (st.state === 'building') {
      const p = st.progress?.percent;
      const ring = p != null ? ProgressRing({ size: 16, value: p }) : ProgressRing({ size: 16 });
      const label = h('span');
      node = h('span', { class: 'alles-idx is-building' }, ring, label);
      updateIndex = () => {
        const st = state.status;
        const p = st.progress?.percent;
        if (p != null) ring.setValue(p);
        label.textContent = p != null ? t('st_building', { p }) : t('st_buildingNoPct');
        if (st.progress?.current) node.dataset.tooltip = t('st_scanning', { path: st.progress.current });
        else delete node.dataset.tooltip;
      };
    } else if (st.refreshing) {
      node = h('span', { class: 'alles-idx is-building' }, ProgressRing({ size: 16 }), h('span', null, t('st_refreshing')));
    } else if (st.state === 'ready') {
      node = h('span', { class: 'alles-idx is-ready' },
        icon('check', { size: 14 }), h('span', null, t('st_ready')),
        st.liveUpdate && st.watching ? [h('span', { class: 'alles-idx__dot', 'aria-hidden': 'true' }, '·'), icon('lightning', { size: 14 }), h('span', null, t('st_live'))] : null);
      updateIndex = () => {
        const builtAt = state.status?.builtAt;
        if (builtAt) node.dataset.tooltip = t('st_builtAt', { when: relTime(builtAt) });
        else delete node.dataset.tooltip;
      };
    } else if (st.state === 'error') {
      const label = h('span');
      node = h('button', { type: 'button', class: 'alles-idx is-error', onClick: () => refreshStatus() },
        icon('warning', { size: 14 }), label);
      updateIndex = () => { label.textContent = state.status?.message || t('st_error'); };
    } else {
      node = h('span', { class: 'alles-idx' }, icon('info', { size: 14 }), h('span', null, t('st_idle')));
    }
    updateIndex?.();
    idxEl.append(node);
  }

  function applyStatus(st) {
    if (!st || typeof st !== 'object') return;
    const wasFirst = isFirstRun();
    state.status = st;
    renderIndexState();
    const first = isFirstRun();
    if (first) showOverlay('first');
    else if (wasFirst) runQuery();
    else if (overlay === 'welcome') showOverlay('welcome');
    renderCounts();
  }

  async function refreshStatus() {
    try { applyStatus(await bridge.call('index.status')); } catch (e) {
      applyStatus({ state: 'error', message: e?.message, files: 0, folders: 0 });
    }
  }

  const onIndexChanged = throttle(() => {
    if (state.query.trim() || state.filter !== 'all') runQuery({ keepScroll: true, keepSelection: true });
    else refreshStatus();
  }, 1200);

  /* ---- context menu + keyboard actions ------------------------------------------ */
  async function selectedItems(limit = COPY_MAX) {
    const idx = table.selectedIndices(limit);
    if (!idx.length && table.focusIndex() >= 0) idx.push(table.focusIndex());
    return table.getItems(idx);
  }

  async function copySelection(names = false) {
    const n = table.selectedCount();
    const items = await selectedItems(COPY_MAX);
    copyPaths(items, { names, capped: n > COPY_MAX });
  }

  async function trashSelection() {
    const items = await selectedItems(501);
    if (await trashItems(items)) runQuery({ keepScroll: true });
  }

  async function openRowMenu(pt, index) {
    const item = table.itemAt(index);
    if (!item) return;
    const n = Math.max(1, table.selectedCount());
    const multi = n > 1;
    showContextMenu({
      x: pt.x, y: pt.y,
      topActions: [
        { icon: 'copy', label: t('copyPath'), action: () => copySelection(false) },
        { icon: 'properties', label: t('properties'), action: () => showProperties(item), disabled: multi },
        { icon: 'delete', label: t('trash'), action: () => trashSelection(), danger: true },
      ],
      items: [
        { label: t('open'), icon: 'open', shortcut: 'Enter', action: () => openItem(item) },
        { label: t('openFolder'), icon: 'folder-open', shortcut: 'Ctrl+Enter', action: () => openContaining(item) },
        item.isDir ? { label: t('openExplorer'), icon: 'folder', action: () => openInExplorer(item) } : null,
        { separator: true },
        { label: t('copyPath'), icon: 'copy', shortcut: 'Ctrl+C', action: () => copySelection(false) },
        { label: t('copyName'), icon: 'rename', shortcut: 'Ctrl+Shift+C', action: () => copySelection(true) },
        { separator: true },
        { label: t('properties'), icon: 'properties', shortcut: 'Alt+Enter', disabled: multi, action: () => showProperties(item) },
        { separator: true },
        { label: t('trash'), icon: 'delete', shortcut: 'Delete', danger: true, action: () => trashSelection() },
      ].filter(Boolean),
    });
  }

  function tableKeys(e) {
    const ctrl = e.ctrlKey || e.metaKey;
    const item = table.focusedItem();
    if (e.key === 'Enter' && ctrl) { if (item) openContaining(item); return true; }
    if (e.key === 'Enter' && e.altKey) { if (item) showProperties(item); return true; }
    if ((e.key === 'c' || e.key === 'C') && ctrl) { copySelection(e.shiftKey); return true; }
    if (e.key === 'Delete') { trashSelection(); return true; }
    if (e.key === 'Escape') { search.focus(); search.select(); return true; }
    if (e.key === 'ArrowUp' && table.focusIndex() <= 0 && !e.shiftKey) { search.focus(); return true; }
    return false;
  }

  search.input.addEventListener('keydown', (e) => {
    if (e.key === 'ArrowDown' || e.key === 'PageDown') {
      if (table.total) { e.preventDefault(); table.focus(); if (table.focusIndex() < 0) table.selectFirst(); }
    } else if (e.key === 'Enter') {
      if (queryPending && search.getValue().trim()) { e.preventDefault(); pendingEnter = { ctrl: e.ctrlKey }; return; }
      if (table.total) { e.preventDefault(); enterInSearch(e.ctrlKey); }
    } else if (e.key === 'Escape') {
      if (search.getValue()) { e.preventDefault(); e.stopPropagation(); setQuery(''); }
    }
  });

  document.addEventListener('keydown', (e) => {
    if (document.querySelector('.fl-dialog-layer')) return;
    const ctrl = e.ctrlKey || e.metaKey;
    if ((ctrl && (e.key === 'f' || e.key === 'F' || e.key === 'e' || e.key === 'E' || e.key === 'l' || e.key === 'L')) || e.key === 'F3') {
      e.preventDefault(); search.focus(); search.select(); return;
    }
    if (e.key === 'F5') { e.preventDefault(); runQuery({ keepScroll: true }); refreshStatus(); return; }
    if (e.altKey && !ctrl && !e.shiftKey) {
      const map = { c: 'matchCase', w: 'wholeWord', p: 'matchPath', r: 'regex' };
      const k = map[e.key.toLowerCase()];
      if (k) { e.preventDefault(); setOption(k, !state[k]); }
    }
  });

  /* ---- live data ------------------------------------------------------------------ */
  bridge.on('index.status', (st) => applyStatus(st));
  bridge.on('index.changed', () => onIndexChanged());
  onSettings((settings, path) => {
    const a = settings?.alles || {};
    for (const k of ['matchCase', 'matchPath', 'regex', 'wholeWord']) {
      if (!!a[k] !== state[k]) { state[k] = !!a[k]; toggles[k].sync(); }
    }
    if (path == null || String(path).startsWith('alles.showHidden') || path === 'alles') {
      if (state.query.trim() || state.filter !== 'all') runQuery({ keepScroll: true });
    }
  });
  frame.onAppNavigate((raw) => {
    const p = withQuery(raw);
    if (typeof p.query === 'string') setQuery(p.query, { focus: true });
    else if (typeof p.path === 'string' && p.path) setQuery(`in:"${p.path}" `, { focus: true });
    if (FILTERS.includes(p.filter)) { state.filter = p.filter; filterDd.setValue(p.filter); runQuery(); }
  });
  // all labels are built once: a language switch reloads this window, keeping the search (D7, §2.7)
  onLangChange(() => {
    try {
      sessionStorage.setItem(RELOAD_KEY, JSON.stringify({ at: Date.now(), query: state.query, filter: state.filter,
        sort: state.sort, desc: state.desc, focus: table.focusIndex(), focusPath: table.focusedItem()?.path ?? null,
        top: table.topIndex() }));
    } catch { /* ignore */ }
    location.reload();
  });

  /* ---- start ----------------------------------------------------------------------- */
  table.setSort(state.sort, state.desc);
  const pp = bridge.params.get('pane');
  setPane(pp === 'preview' || pp === 'tips' ? pp : (pp === 'none' ? null : state.pane));
  await refreshStatus();
  await runQuery();
  if (kept && table.total) {
    if (kept.top > 0) table.scrollToIndex(kept.top);
    // the focused item itself: by its path when the order moved (type names sort per language), else by its row
    let fi = kept.focus;
    const at = fi >= 0 ? table.itemAt(fi) : undefined;
    if (kept.focusPath && at?.path !== kept.focusPath) {
      const j = (state.result?.items || []).findIndex((it) => it?.path === kept.focusPath);
      if (j >= 0) fi = j;
    }
    if (fi >= 0 && fi < table.total) table.setFocus(fi);
  }
  setTimeout(() => { search.focus(); try { search.input.setSelectionRange(state.query.length, state.query.length); } catch { /* ignore */ } }, 0);
  debugOpen();
  return frame;

  /** Screenshot/test helpers: ?select=3 (row index, or 1,4 for several) and ?open=options|tips|preview|menu|properties|trash */
  function debugOpen() {
    const sel = bridge.params.get('select');
    const open = bridge.params.get('open');
    if (!sel && !open) return;
    setTimeout(() => {
      if (sel) {
        const idx = sel.split(',').map((x) => parseInt(x, 10)).filter((x) => x >= 0);
        idx.forEach((i, k) => table.setFocus(i, k ? { toggle: true } : {}));
        table.focus();
      }
      const i = table.focusIndex() >= 0 ? table.focusIndex() : 0;
      const item = table.itemAt(i);
      if (open === 'options' || open === 'options-bottom') {
        openOptions({ status: state.status });
        if (open === 'options-bottom') setTimeout(() => { const b = document.querySelector('.alles-opt-dialog .fl-dialog__body'); if (b) b.scrollTop = b.scrollHeight; }, 400);
      }
      else if (open === 'tips' || open === 'preview') setPane(open);
      else if (open === 'properties' && item) showProperties(item);
      else if (open === 'trash') trashSelection();
      else if (open === 'menu' && item) {
        const r = table.el.querySelector('.alles-row.is-focused')?.getBoundingClientRect();
        openRowMenu({ x: (r?.left || 200) + 160, y: (r?.top || 200) + 16 }, i);
      } else if (open === 'sort') openSortMenu(sortBtn);
      const sc = parseFloat(bridge.params.get('scrollTo') || '');
      if (sc >= 0 && sc <= 1) table.scrollToIndex(Math.floor(sc * Math.max(0, table.total - 1)));
      else if (open === 'filter') filterDd.button?.click();
    }, 60);
  }
}
