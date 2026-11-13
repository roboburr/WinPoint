// Win11 Shell — cross-window tab transfer. Original tab UI is kept intact.
// The native host owns one-use tickets; a source tab closes only after the destination commits.
import { t } from './i18n.js';

export const TAB_MIME = 'application/x-win11-explorer-tab+json';
const TAB_TEXT_PREFIX = 'win11-explorer-tab:v1:';
const KINDS = new Set(['home', 'gallery', 'thispc', 'trash', 'folder', 'search']);
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// a tab state that fails validation: one localized message for the user, the English detail for developers
const invalid = (detail) => Object.assign(new Error(t('tabInvalid')), { code: 'tab_state', detail });

function location(value) {
  if (!value || typeof value !== 'object' || !KINDS.has(value.kind)) throw invalid('Invalid tab location');
  const result = { kind: value.kind };
  if (value.kind === 'folder' || value.kind === 'search') {
    if (typeof value.path !== 'string' || !value.path.startsWith('/') || value.path.length > 16384) throw invalid('Invalid tab path');
    result.path = value.path;
  }
  if (value.kind === 'search') {
    if (typeof value.query !== 'string' || value.query.length > 4096) throw invalid('Invalid tab search');
    result.query = value.query;
  }
  return result;
}

function paths(value) {
  if (!Array.isArray(value) || value.length > 10000 || value.some((path) => typeof path !== 'string' || !path.startsWith('/') || path.length > 16384)) throw invalid('Invalid tab selection');
  return value.slice();
}

/** Explicit serializable state: cached listings, requests and local tab IDs never cross processes. */
export function normalizeTabState(value) {
  if (!value || value.version !== 1) throw invalid('Unsupported tab state');
  const loc = location(value.loc);
  if (!Array.isArray(value.history) || !value.history.length || value.history.length > 60) throw invalid('Invalid tab history');
  const history = value.history.map(location);
  if (!Number.isInteger(value.hIndex) || value.hIndex < 0 || value.hIndex >= history.length) throw invalid('Invalid tab history index');
  if (typeof value.filter !== 'string' || value.filter.length > 4096) throw invalid('Invalid tab filter');
  if (!Array.isArray(value.scrollMemo) || value.scrollMemo.length > 120) throw invalid('Invalid tab scroll positions');
  const scrollMemo = value.scrollMemo.map((entry) => {
    if (!Array.isArray(entry) || entry.length !== 2 || typeof entry[0] !== 'string' || entry[0].length > 20000) throw invalid('Invalid tab scroll entry');
    const scroll = entry[1];
    if (!scroll || !Number.isFinite(scroll.top) || !Number.isFinite(scroll.left) || scroll.top < 0 || scroll.left < 0) throw invalid('Invalid tab scroll offset');
    return [entry[0], { top: scroll.top, left: scroll.left }];
  });
  const selection = paths(value.selection || []);
  const focus = value.focus == null ? null : paths([value.focus])[0];
  return { version: 1, loc, history, hIndex: value.hIndex, selection, focus,
    filter: value.filter, scrollMemo, pendingSelect: value.pendingSelect == null ? null : paths(Array.isArray(value.pendingSelect) ? value.pendingSelect : [value.pendingSelect]),
    pendingProps: value.pendingProps === true, pendingReveal: value.pendingReveal == null ? null : paths([value.pendingReveal])[0] };
}

export function createTabTransfer(app) {
  let session = null;
  const windowId = () => window.__win11shellHost?.windowId || '';
  const release = (ticket) => app.call('explorer.tabRelease', { ticket }).catch(() => {});
  const fail = (error) => app.showError(error, t('tabMoveFailed'));
  const isTabDrag = (event) => {
    const types = Array.from(event.dataTransfer?.types || []);
    // WebKit only exposes custom MIME data to the same origin. Separate native hosts have
    // different localhost ports, so use a marked plain-text envelope across their origins.
    // Protected dragover data cannot be read yet; validate its envelope again on drop.
    return types.includes(TAB_MIME) || (types.includes('text/plain')
      && !types.includes('Files') && !types.includes('text/uri-list'));
  };
  function payload(event) {
    if (!isTabDrag(event)) return null;
    try {
      let encoded = event.dataTransfer.getData(TAB_MIME);
      if (!encoded) {
        const plain = event.dataTransfer.getData('text/plain');
        if (!plain.startsWith(TAB_TEXT_PREFIX)) return null;
        encoded = plain.slice(TAB_TEXT_PREFIX.length);
      }
      const data = JSON.parse(encoded);
      if (!data || typeof data.ticket !== 'string' || !/^[a-f0-9-]{32,36}$/i.test(data.ticket)
          || typeof data.sourceWindow !== 'string' || !data.sourceWindow) return null;
      return data;
    } catch { return null; }
  }
  function cancel() { if (session) session.cancelled = true; }
  // Native drag-failed(USER_CANCELLED) may consume Escape before a DOM key event is delivered.
  addEventListener('win11:tab-drag-cancelled', cancel);
  document.addEventListener('keydown', (event) => { if (event.key === 'Escape') cancel(); }, true);
  document.addEventListener('dragleave', (event) => {
    if (session && !event.relatedTarget && (event.clientX < 0 || event.clientY < 0 || event.clientX >= innerWidth || event.clientY >= innerHeight)) session.outside = true;
  }, true);
  document.addEventListener('dragover', () => { if (session) session.outside = false; }, true);
  // A drop within the client area that is not the tab strip is cancelled, never detached.
  document.addEventListener('drop', (event) => {
    if (session && isTabDrag(event) && !event.target.closest('.xp-tabs')) session.handled = true;
  }, true);

  function start(event, tab) {
    if (!windowId() || !event.dataTransfer || session) { event.preventDefault(); return false; }
    let state;
    try { state = app.exportTab(tab.id); } catch (error) { event.preventDefault(); fail(error); return false; }
    const ticket = crypto.randomUUID();
    const current = { ticket, tabId: tab.id, sourceWindow: windowId(), cancelled: false, handled: false, outside: false, signature: JSON.stringify(state) };
    session = current;
    current.beginPosition = app.call('explorer.tabPosition', { phase: 'begin' }).catch(() => null);
    event.dataTransfer.effectAllowed = 'move';
    const envelope = JSON.stringify({ ticket, sourceWindow: current.sourceWindow });
    event.dataTransfer.setData(TAB_MIME, envelope);
    event.dataTransfer.setData('text/plain', TAB_TEXT_PREFIX + envelope);
    try { event.dataTransfer.setDragImage(event.currentTarget, 40, 14); } catch { /* native default is fine */ }
    current.offer = app.call('explorer.tabOffer', { ticket, state }).catch((error) => { current.failed = true; fail(error); });
    return true;
  }

  async function end(event) {
    const current = session;
    if (!current) return;
    const domOutside = current.outside || event.clientX < 0 || event.clientY < 0 || event.clientX >= innerWidth || event.clientY >= innerHeight;
    const dropEffect = event.dataTransfer?.dropEffect;
    // WebKitGTK can retain its last in-window drag coordinates after a desktop drop.
    // Sample the compositor immediately, before the cancellation grace or a new window moves focus.
    const nativePosition = dropEffect === 'none'
      ? current.beginPosition.then(() => app.call('explorer.tabPosition', { phase: 'end' })).catch(() => null)
      : Promise.resolve(null);
    await current.offer;
    // GTK may emit drag-failed(USER_CANCELLED) just after the DOM dragend event.
    await pause(220);
    if (current.failed) { if (session === current) session = null; return; }
    const position = await nativePosition;
    if (current.cancelled || current.handled) {
      await release(current.ticket); if (session === current) session = null; return;
    }
    const outside = typeof position?.outside === 'boolean' ? position.outside : domOutside;
    // A dropEffect of "none" alone is ambiguous (Escape, unsupported target, desktop); keep the source.
    const detach = outside && dropEffect === 'none';
    if (detach) {
      try { await app.call('explorer.tabDetach', { ticket: current.ticket }); }
      catch (error) { await release(current.ticket); fail(error); if (session === current) session = null; return; }
    } else if (!current.handled && dropEffect !== 'move') {
      await release(current.ticket); if (session === current) session = null; return;
    }
    // A new Rust process plus its compatibility backend may take several seconds to start.
    for (let attempt = 0; attempt < 300; attempt++) {
      try {
        const status = await app.call('explorer.tabStatus', { ticket: current.ticket });
        if (status?.committed === true) {
          let unchanged = false;
          try { unchanged = JSON.stringify(app.exportTab(current.tabId)) === current.signature; } catch { /* changed or already closed */ }
          if (unchanged) app.closeTab(current.tabId);
          else if (app.tabs.some((tab) => tab.id === current.tabId)) fail(new Error(t('tabKeptChanged')));
          break;
        }
      } catch { /* temporary backend loss must never close the source tab */ }
      await pause(150);
    }
    if (session === current) session = null;
  }

  async function accept(data, beforeId = null) {
    if (!data || !windowId()) return false;
    if (data.sourceWindow === windowId()) {
      if (!session || session.ticket !== data.ticket) return false;
      session.handled = true;
      app.moveTabBefore(session.tabId, beforeId);
      await session.offer;
      if (!session.failed) await release(data.ticket);
      return true;
    }
    let imported = null;
    const previousActive = app.activeId;
    try {
      const claim = await app.call('explorer.tabClaim', { ticket: data.ticket });
      if (!claim || claim.sourceWindow !== data.sourceWindow) throw invalid('Tab transfer source does not match');
      imported = await app.importTab(normalizeTabState(claim.state), { beforeId });
      await app.call('explorer.tabCommit', { ticket: data.ticket });
      return true;
    } catch (error) {
      if (imported) app.rollbackImportedTab(imported.id, previousActive);
      await release(data.ticket); fail(error); return false;
    }
  }

  function dragover(event) {
    if (!isTabDrag(event)) return false;
    event.preventDefault(); event.stopPropagation(); event.dataTransfer.dropEffect = 'move'; return true;
  }
  function drop(event) {
    if (!isTabDrag(event)) return false;
    const data = payload(event);
    // Unrelated text and file drops retain their original handlers and default behaviour.
    if (!data) return false;
    event.preventDefault(); event.stopPropagation();
    const target = event.target.closest('.xp-tab');
    let beforeId = target?.dataset.id || null;
    if (target) {
      const rect = target.getBoundingClientRect();
      if (event.clientX >= rect.left + rect.width / 2) {
        const index = app.tabs.findIndex((tab) => tab.id === beforeId);
        beforeId = app.tabs[index + 1]?.id || null;
      }
    }
    void accept(data, beforeId);
    return true;
  }
  return { enabled: () => !!windowId(), start, end, dragover, drop, accept, isTabDrag };
}
