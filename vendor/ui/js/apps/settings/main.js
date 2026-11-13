// Win11 Shell — Instellingen (Settings app). SPEC §11: export async function mountApp(root, params).
//
// Layout like Windows 11 Settings: Mica window (createAppFrame) with back button and title,
// a 280 px navigation pane (user card, "Zoek een instelling", colourful categories) and the
// content: page title / breadcrumb (fixed) above a scrolling column of SettingsCards.
// Deep links: ?page=<id>, app.navigate {params:{page}}. Remembers the last page per session.

import { createAppFrame } from '../appframe.js';
import { h, clear, loadCss, animate } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { makeT, tr, lang, onLangChange } from '../../core/i18n.js';
import { getSettings, onSettings, getShellInfo, setSetting, resolveWallpaper } from '../../core/theme.js';
import { SearchBox, Avatar, ProgressRing, Breadcrumb, SettingsCard, SettingsGroup, EmptyState } from '../../core/components.js';
import { SETTINGS_TOP, SETTINGS_ALIASES, searchSettings, settingsPage } from './registry.js';
import { catIcon } from './navicons.js';
import { get, accentFromImage, ct, recordVisit } from './common.js';

const t = makeT({
  nl: {
    title: 'Instellingen',
    search: 'Zoek een instelling',
    localAccount: 'Lokaal account',
    results: 'Zoekresultaten',
    resultsFor: 'Resultaten voor “{q}”',
    noResults: 'Geen resultaten',
    noResultsBody: 'Probeer een ander woord, bijvoorbeeld “achtergrond”, “geluid” of “wifi”.',
    pageFailed: 'Deze pagina kon niet worden geladen',
    pageFailedBody: 'Er ging iets mis. Probeer het opnieuw.',
    you: 'Gebruiker',
  },
  en: {
    title: 'Settings',
    search: 'Find a setting',
    localAccount: 'Local account',
    results: 'Search results',
    resultsFor: 'Results for “{q}”',
    noResults: 'No results',
    noResultsBody: 'Try another word, for example “background”, “sound” or “wifi”.',
    pageFailed: 'This page couldn’t be loaded',
    pageFailedBody: 'Something went wrong. Try again.',
    you: 'User',
  },
});

/** page id → module file in ./pages/ (each module exports `pages = {id: render}`) */
const MODULES = {
  home: 'home',
  system: 'system', display: 'display', sound: 'sound', notifications: 'notifications', power: 'power',
  storage: 'storage', clipboard: 'clipboard', about: 'about',
  bluetooth: 'bluetooth',
  network: 'network', wifi: 'network', ethernet: 'network',
  personalization: 'personalization', background: 'background', colors: 'colors', themes: 'themes',
  start: 'start', taskbar: 'taskbar', desktopicons: 'desktopicons',
  apps: 'apps', installed: 'installed', defaultapps: 'defaultapps', startup: 'startup',
  accounts: 'accounts',
  time: 'time', datetime: 'time', language: 'time',
  accessibility: 'accessibility', textsize: 'accessibility', visualeffects: 'accessibility', contrast: 'accessibility',
  privacy: 'privacy',
  update: 'update',
  // Windows Update subpages (notes/claude/windows-update-gnome.md §3.6)
  'update-history': 'update-history', 'update-advanced': 'update-advanced',
};

const LAST_KEY = 'win11shell.settings.lastPage';

const store = {
  get(k) { try { return sessionStorage.getItem(k); } catch { return null; } },
  set(k, v) { try { sessionStorage.setItem(k, v); } catch { /* ignore */ } },
};

function resolveId(id) {
  if (!id) return null;
  const s = String(id).toLowerCase();
  if (s === 'search') return 'search';
  if (settingsPage(s)) return s;
  if (SETTINGS_ALIASES[s]) return SETTINGS_ALIASES[s];
  return null;
}

export async function mountApp(root, params = {}) {
  await loadCss('settings');

  /* ---- navigation pane header: user card + search ------------------------ */
  const info = getShellInfo() || {};
  const user = { name: info.user?.realName || info.user?.name || '', avatar: null };
  const avatarSlot = h('span', { class: 'st-user__avatar' });
  const userName = h('span', { class: 'st-user__name' });
  const userKind = h('span', { class: 'st-user__kind' });
  const userCard = h('button', { type: 'button', class: 'st-user', onClick: () => show('accounts') },
    avatarSlot, h('span', { class: 'st-user__text' }, userName, userKind));
  function renderUser() {
    clear(avatarSlot).append(Avatar({ name: user.name || t('you'), image: user.avatar ? bridge.fileUrl(user.avatar) : null, size: 60 }));
    userName.textContent = user.name || t('you');
    userKind.textContent = t('localAccount');
  }
  renderUser();
  get('system.info').then((si) => {
    if (!si?.user) return;
    user.name = si.user.realName || si.user.name || user.name;
    user.avatar = si.user.avatar || null;
    renderUser();
  });

  const search = SearchBox({
    placeholder: t('search'),
    suggest: async (q) => searchSettings(q, lang(), 8).map(({ page }) => ({
      label: tr(page.title),
      description: page.parent ? tr(settingsPage(page.parent).title) : '',
      icon: page.icon,
      value: page.id,
    })),
    onSelect: (r) => { search.setValue(''); show(r.value); },
    onSubmit: (q) => { if (q.trim()) show('search', { params: { q: q.trim() } }); },
  });
  search.classList.add('st-search');
  const navHeader = h('div', { class: 'st-navhead' }, userCard, search);

  const navItems = () => SETTINGS_TOP.map((id) => ({ id, label: tr(settingsPage(id).title), icon: catIcon(id, 16) }));

  /* ---- frame ---------------------------------------------------------------- */
  const { page: firstPage, ...firstParams } = params || {};
  const start = resolveId(firstPage) || resolveId(store.get(LAST_KEY)) || 'home';
  const startTop = start === 'search' ? 'home' : (settingsPage(start)?.parent || start);
  const frame = createAppFrame({
    app: 'settings', title: t('title'), icon: 'settings', back: true,
    nav: { items: navItems(), selected: startTop, header: navHeader },
  });
  frame.el.classList.add('st-app');
  root.append(frame.el);

  /* ---- router ---------------------------------------------------------------- */
  const hist = [];
  let current = null;
  let seq = 0;
  let cleanups = [];
  const modCache = new Map();

  function runCleanups() {
    const list = cleanups;
    cleanups = [];
    for (const fn of list) { try { fn(); } catch (e) { console.error('[settings] cleanup failed', e); } }
  }

  function topOf(id) { const p = settingsPage(id); return p?.parent || p?.id || 'home'; }

  function header(id, params) {
    if (id === 'search') return h('h1', { class: 'title st-title' }, t('results'));
    const p = settingsPage(id);
    if (!p.parent) return h('h1', { class: 'title st-title' }, tr(p.title));
    const parent = settingsPage(p.parent);
    const bc = Breadcrumb({ items: [{ id: parent.id, label: tr(parent.title) }, { id: p.id, label: tr(p.title) }], onSelect: (pid) => show(pid) });
    bc.classList.add('st-title');
    return h('h1', { class: 'st-title-wrap' }, bc);
  }

  function makeCtx(my, id, params) {
    const ctx = {
      id, params, frame, page: settingsPage(id),
      alive: () => my === seq,
      go: (pid, p) => show(pid, { params: p || {} }),
      onCleanup: (fn) => { if (my === seq) cleanups.push(fn); else fn(); },
      on: (event, cb) => { const off = bridge.on(event, (d, n) => { if (my === seq) cb(d, n); }); ctx.onCleanup(off); return off; },
      onSettings: (cb) => { const off = onSettings((s, p) => { if (my === seq) cb(s, p); }); ctx.onCleanup(off); return off; },
      interval: (fn, ms) => { const tm = setInterval(() => { if (my === seq) fn(); }, ms); ctx.onCleanup(() => clearInterval(tm)); return tm; },
      timeout: (fn, ms) => { const tm = setTimeout(() => { if (my === seq) fn(); }, ms); ctx.onCleanup(() => clearTimeout(tm)); return tm; },
      refresh: () => show(id, { push: false, params, animated: false, keepScroll: true }),
      setTitle: (s) => { const el = frame.body.querySelector('.st-title'); if (el && el.tagName === 'H1') el.textContent = s; },
    };
    return ctx;
  }

  async function renderBody(id, ctx) {
    if (id === 'search') return renderSearch(ctx);
    const file = MODULES[id];
    let mod = modCache.get(file);
    if (!mod) { mod = await import(`./pages/${file}.js`); modCache.set(file, mod); }
    const fn = mod.pages?.[id];
    if (typeof fn !== 'function') throw new Error(`pages/${file}.js has no page "${id}"`);
    return fn(ctx);
  }

  function renderSearch(ctx) {
    const q = ctx.params.q || '';
    const hits = searchSettings(q, lang(), 40);
    const sub = h('p', { class: 'st-lead' }, t('resultsFor', { q }));
    if (!hits.length) {
      return [sub, EmptyState({ icon: 'search', title: t('noResults'), message: t('noResultsBody') })];
    }
    return [sub, SettingsGroup({
      children: hits.map(({ page }) => SettingsCard({
        icon: page.icon,
        title: tr(page.title),
        description: (page.parent ? `${tr(settingsPage(page.parent).title)} › ` : '') + tr(page.description),
        onClick: () => ctx.go(page.id),
      })),
    })];
  }

  // One-shot params (a deep link's add:1 = "open the add dialog", consumed by the page) and the window's own
  // URL params (the first page gets them all: surface, app, token, theme, mock…) do not make the same page a
  // new history entry: Back would only show that page again.
  const ONE_SHOT = new Set(['add', 'surface', 'app', 'token', 'theme', 'lang', 'debug']);
  const histKey = (q) => JSON.stringify(Object.entries(q || {})
    .filter(([k]) => !ONE_SHOT.has(k) && !/^mock/.test(k)).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)));

  async function show(rawId, { push = true, params: p = {}, animated = true, keepScroll = false } = {}) {
    const id = resolveId(rawId) || 'home';
    const params = { ...p };
    const same = current && current.id === id && histKey(current.params) === histKey(params);
    if (push && current && !same) hist.push(current);
    if (hist.length > 50) hist.shift();
    current = { id, params };
    frame.setBackEnabled(hist.length > 0);
    if (id !== 'search') {
      frame.navigate(topOf(id), { silent: true, push: false });
      store.set(LAST_KEY, id);
      recordVisit(id);
    } else {
      // no category is selected while showing search results
      frame.navigate('__search__', { silent: true, push: false });
    }
    const pageTitle = id === 'search' ? t('results') : tr(settingsPage(id).title);
    document.title = `${pageTitle} – ${t('title')}`;

    const prevScroll = keepScroll ? frame.body.querySelector('.st-scroll')?.scrollTop || 0 : 0;
    runCleanups();
    const my = ++seq;
    const ctx = makeCtx(my, id, params);

    const head = h('div', { class: 'st-head' }, header(id, params));
    const body = h('div', { class: 'st-body' });
    const scroll = h('div', { class: 'st-scroll' }, h('div', { class: 'st-page' }, body));
    const view = h('div', { class: 'st-view', dataset: { page: id } }, head, scroll);

    // Navigation shows the new title at once (like Windows); the body follows when its data is in.
    // A refresh (language switch, live update) swaps the whole view only when ready, keeping the scroll.
    const immediate = animated && !keepScroll;
    const headIn = () => animate(head, [{ opacity: 0, transform: 'translateY(12px)' }, { opacity: 1, transform: 'none' }], { duration: 167, easing: 'cubic-bezier(0,0,0,1)' });
    if (immediate) { frame.setContent(view); headIn(); }
    const spinnerTimer = setTimeout(() => {
      if (my !== seq) return;
      clear(body).append(h('div', { class: 'st-loading' }, ProgressRing({ size: 32, ariaLabel: ct('loading') })));
      if (!view.isConnected) frame.setContent(view);
    }, 180);

    let content;
    try {
      content = await renderBody(id, ctx);
    } catch (e) {
      console.error(`[settings] page ${id} failed`, e);
      content = EmptyState({ icon: 'warning', title: t('pageFailed'), message: t('pageFailedBody'), action: { label: ct('retry'), onClick: () => ctx.refresh() } });
    }
    clearTimeout(spinnerTimer);
    if (my !== seq) return;
    clear(body).append(...[].concat(content).filter(Boolean));
    if (!view.isConnected) frame.setContent(view);
    if (keepScroll) scroll.scrollTop = prevScroll;
    if (animated) {
      if (!immediate) headIn();
      animate(body, [{ opacity: 0, transform: 'translateY(20px)' }, { opacity: 1, transform: 'none' }], { duration: 250, easing: 'cubic-bezier(0,0,0,1)' });
    }
  }

  // navigation pane → top-level pages
  frame.onNavigate((id) => { if (id && id !== current?.id) show(id); });
  // clicking the selected category while on one of its sub-pages returns to the category page
  frame.nav?.addEventListener('click', (e) => {
    const item = e.target.closest('.fl-nav__item');
    if (!item || e.target.closest('.fl-nav__chev')) return;
    const id = item.dataset.id;
    if (current && current.id !== id && topOf(current.id) === id) show(id);
  });
  frame.nav?.addEventListener('keydown', (e) => {
    if (e.key !== 'Enter' && e.key !== ' ') return;
    const item = e.target.closest?.('.fl-nav__item');
    if (item && current && current.id !== item.dataset.id && topOf(current.id) === item.dataset.id) show(item.dataset.id);
  });
  frame.onBack(() => {
    const prev = hist.pop();
    if (prev) show(prev.id, { push: false, params: prev.params });
    frame.setBackEnabled(hist.length > 0);
  });
  frame.onAppNavigate((p) => {
    const { page, ...rest } = p || {};
    if (page) show(page, { params: rest });
  });
  // Ctrl+F / F3 → search box (like Windows Settings)
  document.addEventListener('keydown', (e) => {
    if ((e.ctrlKey && e.key.toLowerCase() === 'f') || e.key === 'F3') { e.preventDefault(); search.input.focus(); search.input.select(); }
  });

  /* ---- live language switch --------------------------------------------- */
  onLangChange(() => {
    frame.setTitle(t('title'));
    frame.nav?.setItems(navItems());
    if (current && current.id !== 'search') frame.nav?.select(topOf(current.id), { animated: false });
    search.input.placeholder = t('search');
    search.input.setAttribute('aria-label', t('search'));
    renderUser();
    if (current) show(current.id, { push: false, params: current.params, animated: false, keepScroll: true });
  });

  /* ---- automatic accent colour from the wallpaper ------------------------ */
  let accentTimer = 0;
  async function autoAccent() {
    const s = getSettings();
    if (!s.theme?.accentFromWallpaper) return;
    const url = resolveWallpaper(s);
    const hex = url ? await accentFromImage(url) : (s.wallpaper?.color || null);
    if (hex && hex.toLowerCase() !== String(s.theme.accent).toLowerCase()) {
      try { await setSetting('theme.accent', hex); } catch (e) { console.warn('[settings] auto accent failed', e); }
    }
  }
  onSettings((s, path) => {
    if (!s?.theme?.accentFromWallpaper) return;
    if (path == null || path.startsWith('wallpaper') || path === 'theme.mode' || path === 'theme.accentFromWallpaper') {
      clearTimeout(accentTimer);
      accentTimer = setTimeout(autoAccent, 120);
    }
  });

  /* ---- first page ----------------------------------------------------------- */
  await show(start, { push: false, animated: false, params: start === 'search' ? { q: firstParams.q || '' } : firstParams });

  return frame;
}
