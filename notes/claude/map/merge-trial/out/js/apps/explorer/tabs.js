// Win11 Shell — Verkenner: tab strip in the title bar (Win11 22H2+ tabs).
// Click = activate · middle click / × = close · "+" = new tab · drag files over a tab = switch to it.

import { h, animate, finished } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { t } from './i18n.js';
import { createTabTransfer } from './tabtransfer.js';
import { isFileDrag, dropPaths, showHint, hideHint, endDrag } from './dnd.js';

export function TabStrip(app) {
  const list = h('div', { class: 'xp-tabs__list', role: 'tablist', 'aria-label': t('tabs') });
  const addBtn = h('button', {
    type: 'button', class: 'xp-tabs__add no-drag', 'aria-label': t('newTab'),
    dataset: { tooltip: t('newTab'), tooltipPlacement: 'bottom' },
  }, icon('plus', { size: 14 }));
  addBtn.addEventListener('click', () => app.openTab({ kind: 'home' }, { activate: true }));
  const el = h('div', { class: 'xp-tabs' }, list, addBtn);
  const els = new Map(); // tab id → element
  const transfer = createTabTransfer(app);
  el.addEventListener('dragover', transfer.dragover, true);
  el.addEventListener('drop', transfer.drop, true);

  function makeTab(tab) {
    const iconBox = h('span', { class: 'xp-tab__icon' });
    const label = h('span', { class: 'xp-tab__label' });
    const close = h('button', {
      type: 'button', class: 'xp-tab__close', tabindex: -1, 'aria-label': t('closeTab'),
      dataset: { tooltip: t('closeTab'), tooltipPlacement: 'bottom' },
    }, icon('close', { size: 10 }));
    const tabEl = h('div', { class: 'xp-tab no-drag', role: 'tab', tabindex: -1, dataset: { id: tab.id } },
      h('span', { class: 'xp-tab__flare xp-tab__flare--l', 'aria-hidden': 'true' }),
      iconBox, label, close,
      h('span', { class: 'xp-tab__flare xp-tab__flare--r', 'aria-hidden': 'true' }));
    tabEl.draggable = transfer.enabled();
    tabEl.addEventListener('dragstart', (event) => transfer.start(event, tab));
    tabEl.addEventListener('dragend', (event) => { void transfer.end(event); });
    tabEl._icon = iconBox;
    tabEl._label = label;
    close.addEventListener('click', (e) => { e.stopPropagation(); app.closeTab(tab.id); });
    close.addEventListener('pointerdown', (e) => e.stopPropagation());
    tabEl.addEventListener('pointerdown', (e) => {
      if (e.button === 0) app.activateTab(tab.id);
      if (e.button === 1) e.preventDefault();
    });
    tabEl.addEventListener('auxclick', (e) => { if (e.button === 1) { e.preventDefault(); app.closeTab(tab.id); } });
    tabEl.addEventListener('keydown', (e) => {
      const ids = app.tabs.map((x) => x.id);
      const i = ids.indexOf(tab.id);
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        e.preventDefault();
        const j = (i + (e.key === 'ArrowRight' ? 1 : -1) + ids.length) % ids.length;
        app.activateTab(ids[j]);
        els.get(ids[j])?.focus();
      } else if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); app.activateTab(tab.id); app.focusView(); }
      else if (e.key === 'Delete') { e.preventDefault(); app.closeTab(tab.id); }
    });
    // drag files over a tab → switch to it after a moment (Windows behaviour); drop → into its folder
    let hoverTimer = 0;
    tabEl.addEventListener('dragenter', (e) => {
      if (!isFileDrag(e)) return;
      clearTimeout(hoverTimer);
      hoverTimer = setTimeout(() => app.activateTab(tab.id), 600);
    });
    tabEl.addEventListener('dragleave', (e) => { if (!tabEl.contains(e.relatedTarget)) { clearTimeout(hoverTimer); tabEl.classList.remove('is-drop'); } });
    tabEl.addEventListener('dragover', (e) => {
      const tb = app.tabs.find((x) => x.id === tab.id);
      const dest = tb && tb.loc.kind === 'folder' ? tb.loc.path : null;
      const info = dest && isFileDrag(e) ? app.dropInfo(dest, e) : null;
      if (!info) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = info.copy ? 'copy' : 'move';
      tabEl.classList.add('is-drop');
      showHint(e.clientX, e.clientY, info.label, info.copy ? 'copy' : 'move');
    });
    tabEl.addEventListener('drop', (e) => {
      clearTimeout(hoverTimer);
      tabEl.classList.remove('is-drop');
      hideHint();
      const tb = app.tabs.find((x) => x.id === tab.id);
      const dest = tb && tb.loc.kind === 'folder' ? tb.loc.path : null;
      const info = dest ? app.dropInfo(dest, e) : null;
      if (!info) return;
      e.preventDefault();
      const { paths, external } = dropPaths(e);
      endDrag();
      if (paths.length) app.dropTo(paths, dest, info.copy || external);
    });
    return tabEl;
  }

  function render() {
    const alive = new Set(app.tabs.map((x) => x.id));
    for (const [id, tabEl] of els) {
      if (alive.has(id)) continue;
      els.delete(id);
      tabEl.classList.add('is-closing');
      const w = tabEl.offsetWidth;
      const a = animate(tabEl, [{ maxWidth: `${w}px`, minWidth: '0px', opacity: 1 }, { maxWidth: '0px', minWidth: '0px', opacity: 0 }],
        { duration: 167, easing: 'cubic-bezier(0.7,0,1,0.5)', fill: 'forwards' });
      finished(a).then(() => { tabEl.remove(); app.reportTitlebar?.(); });
    }
    let prev = null;
    app.tabs.forEach((tab) => {
      let tabEl = els.get(tab.id);
      const isNew = !tabEl;
      if (isNew) { tabEl = makeTab(tab); els.set(tab.id, tabEl); }
      const active = tab.id === app.activeId;
      tabEl.classList.toggle('is-active', active);
      tabEl.setAttribute('aria-selected', String(active));
      tabEl.tabIndex = active ? 0 : -1;
      const label = app.locLabel(tab.loc);
      if (tabEl._label.textContent !== label) tabEl._label.textContent = label;
      tabEl.title = label;
      const key = app.locKey(tab.loc);
      if (tabEl._icon.dataset.key !== key) { tabEl._icon.replaceChildren(app.locIcon(tab.loc, 16)); tabEl._icon.dataset.key = key; }
      const want = prev ? prev.nextSibling : list.firstChild;
      if (want !== tabEl) list.insertBefore(tabEl, want);
      prev = tabEl;
      if (isNew && app.tabs.length > 1) {
        animate(tabEl, [{ maxWidth: '0px', minWidth: '0px', opacity: 0 }, { maxWidth: '240px', minWidth: '0px', opacity: 1 }],
          { duration: 250, easing: 'cubic-bezier(0,0,0,1)' });
      }
    });
    el.classList.toggle('is-single', app.tabs.length === 1);
    app.reportTitlebar?.();
  }

  return { el, render, focusActive() { els.get(app.activeId)?.focus(); } };
}
