// Win11 Shell — Verkenner: address row (← → ↑ ⟳ · breadcrumb address bar · search) and the command bar.

import { h, clear, debounce } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { SearchBox, iconNode, openPopup } from '../../core/components.js';
import { showContextMenu } from '../../core/contextmenu.js';
import { t } from './i18n.js';
import { isTextInput } from './util.js';
import { isFileDrag, dropPaths, showHint, hideHint, endDrag } from './dnd.js';

function navBtn(glyph, label, onClick) {
  const b = h('button', { type: 'button', class: 'xp-navbtn', 'aria-label': label, dataset: { tooltip: label, tooltipPlacement: 'bottom' } },
    icon(glyph, { size: 16 }));
  b.addEventListener('click', () => { if (!b.disabled) onClick(); });
  b.setLabel = (s) => { b.setAttribute('aria-label', s); b.dataset.tooltip = s; };
  return b;
}

/* ===================================================================== address row */

export function AddressRow(app) {
  const backBtn = navBtn('arrow-left', t('back'), () => app.back());
  const fwdBtn = navBtn('arrow-right', t('forward'), () => app.forward());
  const upBtn = navBtn('arrow-up', t('up'), () => app.up());
  const refreshBtn = navBtn('refresh', t('refresh'), () => app.refresh());
  // right click on back/forward → history list (Windows behaviour)
  for (const [b, dir] of [[backBtn, -1], [fwdBtn, 1]]) {
    b.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      const items = app.historyItems(dir);
      if (items.length) showContextMenu({ anchor: b, items });
    });
  }

  const crumbs = h('div', { class: 'xp-crumbs', role: 'navigation', 'aria-label': t('address') });
  const histBtn = h('button', {
    type: 'button', class: 'xp-addr__hist', tabindex: -1, 'aria-label': t('recentLocations'),
    dataset: { tooltip: t('recentLocations'), tooltipPlacement: 'bottom' },
  }, icon('chevron-down', { size: 12 }));
  const input = h('input', { class: 'xp-addr__input', type: 'text', spellcheck: false, autocomplete: 'off', 'aria-label': t('address'), hidden: true });
  const addr = h('div', { class: 'xp-addr' }, crumbs, input, histBtn);

  histBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    const items = app.recentLocations().map((loc) => ({
      label: app.locLabel(loc, { full: true }), icon: app.locIconSpec(loc), action: () => app.navigate(loc),
    }));
    if (items.length) showContextMenu({ anchor: addr, items, align: 'end' });
  });

  addr.addEventListener('click', (e) => {
    if (e.target.closest('.xp-crumb, .xp-crumb__chev, .xp-addr__hist, .xp-crumbs__more, input')) return;
    editAddress();
  });

  /* ---- breadcrumb ---- */
  function chevButton(parentLoc) {
    const b = h('button', { type: 'button', class: 'xp-crumb__chev', tabindex: -1, 'aria-label': t('subfolders', { name: app.locLabel(parentLoc) }) },
      icon('chevron-right', { size: 12 }));
    b.addEventListener('click', async (e) => {
      e.stopPropagation();
      b.classList.add('is-open');
      const items = await app.subfolderMenu(parentLoc);
      showContextMenu({ anchor: b, items: items.length ? items : [{ label: t('noSubfolders'), disabled: true }], onClose: () => b.classList.remove('is-open') });
    });
    return b;
  }

  function dropTarget(elm, loc) {
    if (loc.kind !== 'folder') return;
    elm.addEventListener('dragover', (e) => {
      const info = isFileDrag(e) ? app.dropInfo(loc.path, e) : null;
      if (!info) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = info.copy ? 'copy' : 'move';
      elm.classList.add('is-drop');
      showHint(e.clientX, e.clientY, info.label, info.copy ? 'copy' : 'move');
    });
    elm.addEventListener('dragleave', () => elm.classList.remove('is-drop'));
    elm.addEventListener('drop', (e) => {
      elm.classList.remove('is-drop');
      hideHint();
      const info = app.dropInfo(loc.path, e);
      if (!info) return;
      e.preventDefault();
      const { paths, external } = dropPaths(e);
      endDrag();
      if (paths.length) app.dropTo(paths, loc.path, info.copy || external);
    });
  }

  function renderCrumbs() {
    clear(crumbs);
    const loc = app.tab?.loc;
    if (!loc) return;
    const parts = app.crumbsFor(loc); // [{label, loc}]
    const rootIcon = h('span', { class: 'xp-crumbs__icon' }, app.locIcon(loc, 16));
    const more = h('button', { type: 'button', class: 'xp-crumbs__more', tabindex: -1, hidden: true, 'aria-label': t('hiddenParts') },
      icon('chevron-left', { size: 12 }), icon('chevron-left', { size: 12 }));
    crumbs.append(rootIcon, more, chevButton({ kind: 'root' }));
    const segEls = [];
    parts.forEach((p, i) => {
      const seg = h('button', { type: 'button', class: ['xp-crumb', i === parts.length - 1 && 'is-current'], tabindex: -1 }, p.label);
      seg.addEventListener('click', (e) => { e.stopPropagation(); app.navigate(p.loc); });
      seg.addEventListener('auxclick', (e) => { if (e.button === 1) { e.preventDefault(); app.openTab(p.loc, { activate: false }); } });
      dropTarget(seg, p.loc);
      const group = h('span', { class: 'xp-crumbs__seg' }, seg, i < parts.length - 1 ? chevButton(p.loc) : null);
      segEls.push({ group, part: p });
      crumbs.append(group);
    });
    // collapse leading segments when they don't fit (Windows shows « with a menu)
    requestAnimationFrame(() => {
      let hidden = 0;
      while (crumbs.scrollWidth > crumbs.clientWidth + 1 && hidden < segEls.length - 1) {
        segEls[hidden].group.hidden = true;
        hidden++;
      }
      more.hidden = hidden === 0;
      more.onclick = (e) => {
        e.stopPropagation();
        showContextMenu({
          anchor: more,
          items: segEls.slice(0, hidden).reverse().map((s) => ({ label: s.part.label, icon: app.locIconSpec(s.part.loc), action: () => app.navigate(s.part.loc) })),
        });
      };
    });
  }

  /* ---- edit mode ---- */
  let sugPopup = null, sugItems = [], sugActive = -1, sugSeq = 0;
  function closeSug() { sugPopup?.close(); sugPopup = null; sugItems = []; sugActive = -1; }
  function editAddress() {
    if (!input.hidden) { input.select(); return; }
    addr.classList.add('is-editing');
    crumbs.hidden = true;
    input.hidden = false;
    input.value = app.addressText(app.tab.loc);
    input.focus();
    input.select();
  }
  function endEdit() {
    closeSug();
    addr.classList.remove('is-editing');
    input.hidden = true;
    crumbs.hidden = false;
  }
  const suggest = debounce(async () => {
    const my = ++sugSeq;
    const v = input.value;
    const list = await app.addressSuggestions(v);
    if (my !== sugSeq || input.hidden) return;
    showSug(list);
  }, 120);
  function showSug(list) {
    sugItems = list;
    sugActive = -1;
    if (!list.length) { closeSug(); return; }
    const content = h('div', { class: 'xp-sug', role: 'listbox' }, list.map((s, i) => {
      const row = h('div', { class: 'fl-list-item xp-sug__item', role: 'option', dataset: { i } },
        iconNode(s.icon, 16), h('span', { class: 'xp-sug__text' }, s.label));
      row.addEventListener('pointerdown', (e) => e.preventDefault());
      row.addEventListener('click', () => { input.value = s.value; closeSug(); go(); });
      return row;
    }));
    if (sugPopup) { sugPopup.el.replaceChildren(content); sugPopup.reposition(); return; }
    sugPopup = openPopup({
      anchor: addr, content, className: 'xp-sug__popup', closeOnScroll: false,
      position(pop, r) {
        pop.style.width = `${Math.round(r.width)}px`;
        pop.style.left = `${Math.round(r.left)}px`;
        pop.style.top = `${Math.round(r.bottom + 2)}px`;
        return 'down';
      },
      onClose: () => { sugPopup = null; },
    });
  }
  function markSug() {
    sugPopup?.el.querySelectorAll('.xp-sug__item').forEach((r, i) => r.classList.toggle('is-active', i === sugActive));
  }
  async function go() {
    const text = input.value;
    const loc = await app.resolveAddress(text);
    if (loc) { endEdit(); app.navigate(loc); app.focusView(); }
    else app.addressNotFound(text);
  }
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); endEdit(); app.focusView(); }
    else if (e.key === 'Enter') {
      e.preventDefault();
      if (sugActive >= 0 && sugItems[sugActive]) input.value = sugItems[sugActive].value;
      go();
    } else if (e.key === 'ArrowDown' && sugItems.length) { e.preventDefault(); sugActive = Math.min(sugItems.length - 1, sugActive + 1); markSug(); }
    else if (e.key === 'ArrowUp' && sugItems.length) { e.preventDefault(); sugActive = Math.max(-1, sugActive - 1); markSug(); }
    else if (e.key === 'Tab' && sugItems.length && !e.shiftKey) {
      e.preventDefault();
      const s = sugItems[Math.max(0, sugActive)];
      if (s) { input.value = s.value.endsWith('/') ? s.value : `${s.value}/`; suggest(); }
    }
  });
  input.addEventListener('input', suggest);
  input.addEventListener('blur', () => setTimeout(() => { if (document.activeElement !== input) endEdit(); }, 150));

  /* ---- search ---- */
  const search = SearchBox({
    placeholder: t('searchIn', { name: '' }),
    onInput: debounce((v) => app.setFilter(v), 120),
    onSubmit: (v) => app.searchSubmit(v),
  });
  search.classList.add('xp-search');

  const el = h('div', { class: 'xp-addrrow' },
    h('div', { class: 'xp-navbtns' }, backBtn, fwdBtn, upBtn, refreshBtn), addr, search);

  function update() {
    const tab = app.tab;
    if (!tab) return;
    backBtn.disabled = !app.canBack();
    fwdBtn.disabled = !app.canForward();
    const parent = app.parentLoc(tab.loc);
    upBtn.disabled = !parent;
    backBtn.setLabel(app.canBack() ? t('backTo', { name: app.locLabel(tab.history[tab.hIndex - 1]) }) : t('back'));
    fwdBtn.setLabel(app.canForward() ? t('forwardTo', { name: app.locLabel(tab.history[tab.hIndex + 1]) }) : t('forward'));
    upBtn.setLabel(parent ? t('upTo', { name: app.locLabel(parent) }) : t('up'));
    if (!input.hidden && document.activeElement === input) { /* keep editing */ } else { endEdit(); renderCrumbs(); }
    const ph = t('searchIn', { name: app.locLabel(tab.loc) });
    search.input.placeholder = ph;
    search.input.setAttribute('aria-label', ph);
    if (search.input.value !== (tab.filter || '') && document.activeElement !== search.input) search.setValue(tab.filter || '');
  }

  let resizeT = 0;
  if ('ResizeObserver' in window) new ResizeObserver(() => { cancelAnimationFrame(resizeT); resizeT = requestAnimationFrame(() => { if (input.hidden) renderCrumbs(); }); }).observe(addr);

  return {
    el, update,
    focusAddress: editAddress,
    focusSearch() { search.input.focus(); search.input.select(); },
    get searchValue() { return search.input.value; },
    clearSearch() { search.setValue(''); },
    isEditing: () => !input.hidden || isTextInput(document.activeElement),
  };
}

/* ===================================================================== command bar */

function cmdBtn({ glyph, label, text, chevron = false, onClick, cls, accentIcon = false }) {
  const b = h('button', {
    type: 'button', class: ['xp-cmd', text && 'has-text', cls],
    'aria-label': label, dataset: text ? {} : { tooltip: label, tooltipPlacement: 'bottom' },
    'aria-haspopup': chevron ? 'menu' : null,
  },
  h('span', { class: ['xp-cmd__icon', accentIcon && 'is-accent'] }, typeof glyph === 'string' ? icon(glyph, { size: 16 }) : glyph),
  text ? h('span', { class: 'xp-cmd__text' }, text) : null,
  chevron ? h('span', { class: 'xp-cmd__chev' }, icon('chevron-down', { size: 12 })) : null);
  b.addEventListener('click', (e) => { if (!b.disabled) onClick?.(e, b); });
  return b;
}

const sep = () => h('span', { class: 'xp-cmdsep', 'aria-hidden': 'true' });

/** "Nieuw" glyph: accent-filled circle with a plus, like Windows. */
function newGlyph() {
  return h('svg', { viewBox: '0 0 16 16', width: 16, height: 16, class: 'xp-newglyph', 'aria-hidden': 'true' },
    h('circle', { cx: 8, cy: 8, r: 7.25 }), h('path', { d: 'M8 4.75v6.5M4.75 8h6.5' }));
}

export function CommandBar(app) {
  const menu = (fn) => (e, b) => {
    b.classList.add('is-open');
    showContextMenu({ anchor: b, items: fn(), onClose: () => b.classList.remove('is-open') });
  };
  const B = {
    new: cmdBtn({ glyph: newGlyph(), label: t('newHint'), text: t('new'), chevron: true, onClick: menu(() => app.menus.newMenu()), cls: 'xp-cmd--new' }),
    cut: cmdBtn({ glyph: 'cut', label: t('cut'), onClick: () => app.act.cut() }),
    copy: cmdBtn({ glyph: 'copy', label: t('copy'), onClick: () => app.act.copy() }),
    paste: cmdBtn({ glyph: 'paste', label: t('paste'), onClick: () => app.act.paste() }),
    rename: cmdBtn({ glyph: 'rename', label: t('rename'), onClick: () => app.act.rename() }),
    share: cmdBtn({ glyph: 'share', label: t('share'), onClick: () => app.act.share() }),
    delete: cmdBtn({ glyph: 'delete', label: t('delete'), onClick: () => app.act.trash() }),
    sort: cmdBtn({ glyph: 'sort', label: t('sort'), text: t('sort'), chevron: true, onClick: menu(() => app.menus.sortMenu()) }),
    view: cmdBtn({ glyph: 'view', label: t('view'), text: t('view'), chevron: true, onClick: menu(() => app.menus.viewMenu()) }),
    more: cmdBtn({ glyph: 'more', label: t('more'), onClick: menu(() => app.menus.moreMenu()) }),
    emptyTrash: cmdBtn({ glyph: 'trash', label: t('emptyTrash'), text: t('emptyTrash'), onClick: () => app.act.emptyTrash() }),
    restore: cmdBtn({ glyph: 'undo', label: t('restoreAll'), text: t('restoreAll'), onClick: () => app.act.restore() }),
    details: cmdBtn({ glyph: 'properties', label: t('detailsHint'), text: t('details'), onClick: () => app.act.toggleCfg('detailsPane'), cls: 'xp-cmd--details' }),
  };
  B.details.dataset.tooltip = t('detailsHint');
  B.details.dataset.tooltipPlacement = 'bottom';
  const groupNew = h('div', { class: 'xp-cmdgroup' }, B.new, sep());
  const groupClip = h('div', { class: 'xp-cmdgroup' }, B.cut, B.copy, B.paste, B.rename, B.share, B.delete, sep());
  const groupTrash = h('div', { class: 'xp-cmdgroup' }, B.emptyTrash, B.restore, sep());
  const el = h('div', { class: 'xp-cmdbar', role: 'toolbar', 'aria-label': t('commandBar') },
    groupNew, groupClip, groupTrash,
    h('div', { class: 'xp-cmdgroup' }, B.sort, B.view, sep(), B.more),
    h('div', { class: 'xp-cmdbar__spacer' }), B.details);

  // roving arrow keys inside the toolbar
  el.addEventListener('keydown', (e) => {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    const btns = [...el.querySelectorAll('.xp-cmd')].filter((b) => !b.disabled && b.offsetParent);
    const i = btns.indexOf(document.activeElement);
    if (i < 0) return;
    e.preventDefault();
    btns[(i + (e.key === 'ArrowRight' ? 1 : -1) + btns.length) % btns.length].focus();
  });

  function update() {
    const tab = app.tab;
    if (!tab) return;
    const kind = tab.loc.kind;
    const sel = app.selectedItems();
    const n = sel.length;
    const isTrash = kind === 'trash';
    const inFolder = kind === 'folder';
    const fileish = inFolder || kind === 'search';
    groupNew.hidden = isTrash;
    groupClip.hidden = isTrash;
    groupTrash.hidden = !isTrash;
    B.new.disabled = !inFolder;
    B.cut.disabled = !(fileish && n);
    B.copy.disabled = !((fileish || kind === 'home' || kind === 'gallery') && n);
    B.paste.disabled = !(inFolder && app.S.clipboard.paths.length);
    B.rename.disabled = !(fileish && n === 1);
    B.share.disabled = !(n && kind !== 'thispc');
    B.delete.disabled = !(fileish && n) && !(kind === 'gallery' && n);
    B.sort.disabled = !(fileish || isTrash);
    B.view.disabled = !(fileish || isTrash);
    B.emptyTrash.disabled = !(tab.items?.length);
    const restoreLabel = n ? t('restoreSel') : t('restoreAll');
    B.restore.querySelector('.xp-cmd__text').textContent = restoreLabel;
    B.restore.setAttribute('aria-label', restoreLabel);
    B.restore.disabled = !(tab.items?.length);
    const on = !!app.cfg('detailsPane', false);
    B.details.classList.toggle('is-on', on);
    B.details.setAttribute('aria-pressed', String(on));
  }

  return { el, update };
}
