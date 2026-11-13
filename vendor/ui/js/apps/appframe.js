// Win11 Shell — shared app window frame (owned by ui-core). SPEC §4.
//
//   const frame = createAppFrame({
//     app: 'settings', title: 'Instellingen', icon: 'settings',
//     back: true,                                   // show the title-bar back button
//     nav: { items, selected, header, footerItems, paneToggle },   // optional NavView
//     content: element, titlebarContent: element (e.g. a SearchBox), layer: false,
//   });
//   root.append(frame.el);
//   frame.onNavigate((id, {back}) => …)   frame.navigate(id)   frame.setContent(el)
//   frame.onAppNavigate((params) => …)    // app.navigate events for this app
//   frame.reportTitlebar()                // re-send the drag region (e.g. after tabs changed)
//
// Window actions: native (window.webkit.messageHandlers.win11.postMessage({cmd})) →
// embedded iframe (parent.postMessage({win11: cmd, app})) → plain browser (no-op / close()).
// Window state comes from the preview WM (message {win11Frame: {focused, maximized}}) or from
// the native host (event 'win11:windowstate' {maximized, focused, fullscreen, tiled}, also kept
// in window.__win11WindowState). The WM re-opens an app with {win11Navigate: {app, params}},
// which is delivered locally as the 'app.navigate' event.

import { h, clear, loadCss } from '../core/dom.js';
import { hasIcon } from '../core/icons.js';
import { NavView, IconButton, AppIcon, iconNode } from '../core/components.js';
import { bridge } from '../core/bridge.js';
import { makeT, onLangChange } from '../core/i18n.js';

loadCss('appframe');

const t = makeT({
  nl: { minimize: 'Minimaliseren', maximize: 'Maximaliseren', restore: 'Vorige grootte', close: 'Sluiten', back: 'Terug' },
  en: { minimize: 'Minimise', maximize: 'Maximise', restore: 'Restore down', close: 'Close', back: 'Back' },
});

const SVG = (inner) => {
  const s = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  s.setAttribute('viewBox', '0 0 10 10');
  s.setAttribute('width', '10');
  s.setAttribute('height', '10');
  s.setAttribute('fill', 'none');
  s.setAttribute('stroke', 'currentColor');
  s.setAttribute('stroke-width', '1');
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('class', 'af-glyph');
  s.innerHTML = inner;
  return s;
};
const GLYPH = {
  minimize: () => SVG('<path d="M0 5.5h10"/>'),
  maximize: () => SVG('<rect x=".5" y=".5" width="9" height="9" rx="1.5"/>'),
  restore: () => SVG('<rect x=".5" y="2.5" width="7" height="7" rx="1.5"/><path d="M2.6.5h5.4c.83 0 1.5.67 1.5 1.5v5.4"/>'),
  close: () => SVG('<path d="M.5.5l9 9M9.5.5l-9 9"/>'),
};

function nativeHandler() {
  try { return window.webkit?.messageHandlers?.win11 || null; } catch { return null; }
}
const isEmbedded = () => bridge.embedded || window.parent !== window;

/** Route a window command: native → embedded parent → browser. Returns the route used. */
export function windowCommand(cmd, extra = {}, app = bridge.params.get('app')) {
  const n = nativeHandler();
  if (n) {
    try { n.postMessage({ cmd, ...extra }); return 'native'; } catch (e) { console.warn('[appframe] native postMessage failed', e); }
  }
  if (window.parent !== window) {
    try { parent.postMessage({ win11: cmd, app, ...extra }, location.origin === 'null' ? '*' : location.origin); } catch { /* ignore */ }
    return 'embedded';
  }
  if (cmd === 'close') { try { window.close(); } catch { /* ignore */ } }
  return 'browser';
}

export function createAppFrame({ app = bridge.params.get('app') || 'app', title = '', icon: ic, nav = null, content = null,
  back = false, titlebarContent = null, layer = false, compactBelow = 760 } = {}) {
  const html = document.documentElement;
  if (nativeHandler()) html.dataset.nativeWindow = 'true';
  const tall = !!(nav || back);

  /* ---- title bar -------------------------------------------------------- */
  const backBtn = back ? IconButton({ icon: 'arrow-left', label: t('back'), size: 36, iconSize: 16, cls: 'af-back', tooltip: true }) : null;
  if (backBtn) { backBtn.style.width = '40px'; backBtn.disabled = true; }
  const appIcon = h('span', { class: 'af-appicon', 'aria-hidden': 'true' },
    ic instanceof Node ? ic : (ic && hasIcon(ic) ? iconNode(ic, 16) : (ic ? AppIcon({ name: ic, size: 16 }) : null)));
  if (!ic) appIcon.hidden = true;
  const titleEl = h('span', { class: 'af-title' }, title);
  const center = h('div', { class: 'af-titlebar__center' });
  if (titlebarContent) center.append(titlebarContent);
  const minBtn = h('button', { type: 'button', class: 'af-cap af-cap--min', 'aria-label': t('minimize'), dataset: { tooltip: t('minimize'), tooltipPlacement: 'bottom' } }, GLYPH.minimize());
  const maxBtn = h('button', { type: 'button', class: 'af-cap af-cap--max', 'aria-label': t('maximize'), dataset: { tooltip: t('maximize'), tooltipPlacement: 'bottom' } }, GLYPH.maximize());
  const closeBtn = h('button', { type: 'button', class: 'af-cap af-cap--close', 'aria-label': t('close'), dataset: { tooltip: t('close'), tooltipPlacement: 'bottom' } }, GLYPH.close());
  const captions = h('div', { class: 'af-captions' }, minBtn, maxBtn, closeBtn);
  const titlebar = h('header', { class: ['af-titlebar', tall && 'is-tall', backBtn && 'has-back'] },
    h('div', { class: 'af-titlebar__left' }, backBtn, appIcon, titleEl), center, captions);

  /* ---- body ---------------------------------------------------------------- */
  const main = h('main', { class: ['af-content', layer && 'is-layer'], tabindex: -1 });
  if (content) main.append(content);
  const body = h('div', { class: 'af-body' });
  const win = h('div', { class: 'af-window mica', dataset: { app, active: 'true', maximized: 'false' } }, titlebar, body);

  const navListeners = new Set();
  const backListeners = new Set();
  const history = [];
  let navView = null;
  let current = nav?.selected ?? null;

  function emitNavigate(id, info) {
    for (const cb of [...navListeners]) { try { cb(id, info); } catch (e) { console.error('[appframe] onNavigate failed', e); } }
  }
  function updateBack() {
    if (!backBtn) return;
    backBtn.disabled = !(backListeners.size ? frame.backEnabled : history.length > 0);
  }

  if (nav) {
    navView = NavView({
      items: nav.items || [], selected: current, header: nav.header, footer: nav.footer,
      footerItems: nav.footerItems || [], paneToggle: nav.paneToggle ?? true, ariaLabel: title,
      onSelect: (id) => {
        if (id === current) return;
        if (current != null) history.push(current);
        current = id;
        updateBack();
        emitNavigate(id, { back: false });
      },
    });
    body.append(navView, main);
  } else {
    body.append(main);
  }

  /* ---- window state ------------------------------------------------------- */
  let wmState = null; // set when the WM / host tells us
  let docFocused = isEmbedded() ? true : document.hasFocus();
  let lastMax = null;
  function applyState() {
    const focused = wmState?.focused ?? docFocused;
    const maximized = !!(wmState?.maximized || wmState?.fullscreen);
    win.dataset.active = focused ? 'true' : 'false';
    win.dataset.maximized = maximized ? 'true' : 'false';
    // square corners whenever the window touches the screen edges (maximized, fullscreen, tiled)
    win.dataset.square = maximized || wmState?.tiled ? 'true' : 'false';
    if (lastMax !== maximized) {
      lastMax = maximized;
      clear(maxBtn).append(maximized ? GLYPH.restore() : GLYPH.maximize());
      const label = maximized ? t('restore') : t('maximize');
      maxBtn.setAttribute('aria-label', label);
      maxBtn.dataset.tooltip = label;
    }
  }
  function mergeState(s) {
    if (!s || typeof s !== 'object') return;
    const next = { ...(wmState || {}) };
    for (const k of ['focused', 'maximized', 'fullscreen', 'tiled']) if (typeof s[k] === 'boolean') next[k] = s[k];
    wmState = next;
    applyState();
  }
  addEventListener('focus', () => { docFocused = true; if (!wmState || wmState.focused === undefined) applyState(); });
  addEventListener('blur', () => { docFocused = false; if (!wmState || wmState.focused === undefined) applyState(); });
  addEventListener('message', (e) => {
    if (e.source !== window.parent || window.parent === window || !e.data || typeof e.data !== 'object') return;
    if (location.origin !== 'null' && e.origin !== location.origin) return;   // only our own preview page
    if (e.data.win11Navigate && typeof e.data.win11Navigate === 'object') {
      // the preview WM re-opens this app with new params (real host mode; the mock relays itself)
      const nav = e.data.win11Navigate;
      bridge.emitLocal('app.navigate', { app: nav.app || app, params: nav.params && typeof nav.params === 'object' ? nav.params : {} });
      return;
    }
    if (e.data.win11Frame) mergeState(e.data.win11Frame);
  });
  // native app window (host/appwindow.py): 'win11:windowstate' events + the last state
  addEventListener('win11:windowstate', (e) => mergeState(e.detail));
  // a live language switch: the chrome this frame labels itself (the caption buttons' aria-label and hover tooltip,
  // Back), as a freshly opened window shows it; the app relabels its own parts (F1 class, GE1a review M1)
  onLangChange(() => {
    backBtn?.setLabel(t('back'));
    for (const [btn, key] of [[minBtn, 'minimize'], [closeBtn, 'close']]) {
      btn.setAttribute('aria-label', t(key));
      btn.dataset.tooltip = t(key);
    }
    const label = lastMax ? t('restore') : t('maximize');
    maxBtn.setAttribute('aria-label', label);
    maxBtn.dataset.tooltip = label;
  });
  if (window.__win11WindowState) mergeState(window.__win11WindowState);
  // older entry point, kept: window.__win11frame.setState({focused, maximized})
  window.__win11frame = { setState: (s) => mergeState(s) };

  /* ---- caption buttons & dragging ----------------------------------------- */
  minBtn.addEventListener('click', () => windowCommand('minimize', {}, app));
  maxBtn.addEventListener('click', () => windowCommand('toggleMaximize', {}, app));
  closeBtn.addEventListener('click', () => frame.close());
  backBtn?.addEventListener('click', () => frame.back());

  const interactive = (el) => el.closest('button, input, textarea, select, a, [role="button"], [contenteditable="true"], .fl-textbox, .fl-dropdown, .no-drag');
  let drag = null;
  titlebar.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || interactive(e.target)) return;
    if (nativeHandler()) {
      windowCommand('dragStart', { x: e.screenX, y: e.screenY, button: 1 }, app);
      return;
    }
    if (window.parent === window) return;
    drag = { id: e.pointerId };
    titlebar.setPointerCapture(e.pointerId);
    windowCommand('dragStart', { screenX: e.screenX, screenY: e.screenY, clientX: e.clientX, clientY: e.clientY }, app);
  });
  titlebar.addEventListener('pointermove', (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    windowCommand('dragMove', { screenX: e.screenX, screenY: e.screenY }, app);
  });
  const endDrag = (e) => {
    if (!drag || e.pointerId !== drag.id) return;
    drag = null;
    windowCommand('dragEnd', { screenX: e.screenX, screenY: e.screenY }, app);
  };
  titlebar.addEventListener('pointerup', endDrag);
  titlebar.addEventListener('pointercancel', endDrag);
  titlebar.addEventListener('dblclick', (e) => { if (!interactive(e.target)) windowCommand('toggleMaximize', {}, app); });

  // clicking anywhere inside an embedded app activates its window in the preview WM
  if (window.parent !== window && !nativeHandler()) {
    document.addEventListener('pointerdown', () => windowCommand('activate', {}, app), true);
  }

  // report the drag region to a native host: once, and whenever the title bar changes
  function reportTitlebar() {
    if (!nativeHandler()) return;
    const tr = titlebar.getBoundingClientRect();
    const noDragRects = [...titlebar.querySelectorAll('button, input, .fl-textbox, .fl-dropdown, a, [role="tab"], .no-drag')]
      .filter((el) => el.getClientRects().length)
      .map((el) => { const r = el.getBoundingClientRect(); return { x: Math.round(r.left), y: Math.round(r.top), w: Math.round(r.width), h: Math.round(r.height) }; });
    windowCommand('titlebar', { height: Math.round(tr.height), noDragRects }, app);
  }
  let reportRaf = 0;
  const scheduleReport = () => { cancelAnimationFrame(reportRaf); reportRaf = requestAnimationFrame(reportTitlebar); };
  if (nativeHandler()) {
    // until the host knows the drag region, the native title bar does not drag
    if ('ResizeObserver' in window) new ResizeObserver(scheduleReport).observe(titlebar);
    addEventListener('resize', scheduleReport);
    document.fonts?.ready?.then(scheduleReport, () => {});
  }

  // keyboard: Alt+Left = back
  document.addEventListener('keydown', (e) => {
    if (e.altKey && e.key === 'ArrowLeft' && backBtn && !backBtn.disabled) { e.preventDefault(); frame.back(); }
  });

  // compact navigation pane on narrow windows
  if (navView && 'ResizeObserver' in window) {
    new ResizeObserver(([entry]) => {
      const w = entry.contentRect.width;
      navView.setCompact(w < compactBelow);
    }).observe(win);
  }

  /* ---- API ------------------------------------------------------------------ */
  const frame = {
    el: win,
    titlebar,
    body: main,
    nav: navView,
    backEnabled: false,
    setTitle(s) {
      titleEl.textContent = s;
      document.title = s;
      navView?.setAriaLabel(s);                  // the navigation tree is named after the app (NavView ariaLabel)
      // preview WM / taskbar only: a native window's GTK title belongs to the host (contracts C4), never sent from here
      if (window.parent !== window && !nativeHandler()) windowCommand('setTitle', { title: s }, app);
    },
    setIcon(spec) { clear(appIcon).append(spec instanceof Node ? spec : (hasIcon(spec) ? iconNode(spec, 16) : AppIcon({ name: spec, size: 16 }))); appIcon.hidden = !spec; },
    setContent(el) { clear(main); if (el) main.append(el); main.scrollTop = 0; },
    setTitlebarContent(el) { clear(center); if (el) center.append(el); scheduleReport(); },
    /** Re-send the title-bar drag region to a native window (call after changing title-bar content). */
    reportTitlebar() { scheduleReport(); },
    onNavigate(cb) { navListeners.add(cb); return () => navListeners.delete(cb); },
    /** navigate(id, {push=true, silent=false}) — select a nav item programmatically */
    navigate(id, { push = true, silent = false } = {}) {
      if (id === current && !silent) { emitNavigate(id, { back: false, same: true }); return; }
      if (push && current != null && id !== current) history.push(current);
      current = id;
      navView?.select(id);
      updateBack();
      if (!silent) emitNavigate(id, { back: false });
    },
    back() {
      if (backListeners.size) { for (const cb of [...backListeners]) cb(); return; }
      const prev = history.pop();
      if (prev == null) { updateBack(); return; }
      current = prev;
      navView?.select(prev);
      updateBack();
      emitNavigate(prev, { back: true });
    },
    get current() { return current; },
    /** Take over the back button (e.g. Explorer's own history). */
    onBack(cb) { backListeners.add(cb); updateBack(); return () => { backListeners.delete(cb); updateBack(); }; },
    setBackEnabled(b) { frame.backEnabled = !!b; if (backBtn) backBtn.disabled = !b; },
    /** app.navigate events for this app → cb(params) */
    onAppNavigate(cb) {
      return bridge.on('app.navigate', (d) => { if (!d || !d.app || d.app === app) cb(d?.params || {}); });
    },
    setState(s) { wmState = { ...(wmState || {}), ...s }; applyState(); },
    minimize: () => windowCommand('minimize', {}, app),
    toggleMaximize: () => windowCommand('toggleMaximize', {}, app),
    close: () => windowCommand('close', {}, app),
  };

  document.title = title || document.title;
  applyState();
  setTimeout(scheduleReport, 0);
  return frame;
}
