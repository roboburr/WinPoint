// Win11 Shell — context menus (owned by ui-core). SPEC §4.
//
//   showContextMenu({x, y, items, topActions, anchor, onClose})
//   items: [{label, icon, shortcut, action(item), submenu:[…], separator:true, header:'Tekst',
//            checked, radio, disabled, danger, description}]
//   topActions: [{icon:'cut', label:'Knippen', action, disabled, danger}]  (Explorer-style icon row)
//   bindContextMenu(el, (event) => ({items, topActions}) | null)  → unbind()
//
// Acrylic, 8 px radius, 4 px item radius; keyboard: arrows, Home/End, Enter/Space,
// Esc, Left/Right for submenus, first letter jumps. Marks itself data-region.
// Animations keep the glass frosted: the menu itself only slides + reveals (transform,
// clip-path), its items fade in (fadeContent); closing fades the glass itself.

import { h, clamp, animate, finished, fadeContent } from './dom.js';
import { icon } from './icons.js';
import { iconNode, themeScope, hideTooltip } from './components.js';
import { regionsActive, holdKeyboard } from './regions.js';

let current = null;

function buildMenu(items = [], topActions = [], { isStatic = false, theme } = {}) {
  const list = items.filter(Boolean);
  const hasIcons = list.some((it) => it.icon || it.checked !== undefined || it.radio);
  // Win11 (WinUI MenuFlyout): when a menu mixes icons and check/radio items, the selection mark gets its
  // own narrow column in front of the icon (Explorer's "Weergeven" menu: • ▦ Details), so a selected
  // item keeps its icon.
  const hasState = list.some((it) => !it.separator && !it.header && (it.radio || it.checked !== undefined));
  const stateColumn = hasState && list.some((it) => !it.separator && !it.header && it.icon);
  const menu = h('div', {
    class: ['fl-menu', 'acrylic', 'surface-flyout', hasIcons && 'has-icons', stateColumn && 'has-state',
      isStatic && 'is-static'],
    role: 'menu', tabindex: -1, 'data-popup': '', 'data-region': isStatic ? null : '',
    dataset: theme ? { theme } : {},
  });
  const entries = [];
  if (topActions && topActions.length) {
    const row = h('div', { class: 'fl-menu__top', role: 'group' });
    for (const a of topActions) {
      const b = h('button', {
        type: 'button', class: ['fl-menu__top-btn', a.danger && 'is-danger'], tabindex: -1, role: 'menuitem',
        'aria-label': a.label, disabled: !!a.disabled, dataset: { tooltip: a.label, tooltipPlacement: 'bottom' },
      }, iconNode(a.icon, 16));
      row.append(b);
      entries.push({ el: b, item: a, top: true });
    }
    menu.append(row);
    if (list.length) menu.append(h('div', { class: 'fl-menu__sep', role: 'separator' }));
  }
  for (const it of list) {
    if (it.separator) { menu.append(h('div', { class: 'fl-menu__sep', role: 'separator' })); continue; }
    if (it.header) { menu.append(h('div', { class: 'fl-menu__header', role: 'presentation' }, it.header)); continue; }
    const iconCell = h('span', { class: 'fl-menu__icon', 'aria-hidden': 'true' });
    let stateCell = null;
    if (stateColumn) {
      stateCell = h('span', { class: 'fl-menu__state', 'aria-hidden': 'true' });
      if (it.radio && it.checked) stateCell.append(h('span', { class: 'fl-menu__radio' }));
      else if (it.checked) stateCell.append(icon('check', { size: 12 }));
      if (it.icon) iconCell.append(iconNode(it.icon, 16));
    } else if (it.radio && it.checked) iconCell.append(h('span', { class: 'fl-menu__radio' }));
    else if (it.checked) iconCell.append(icon('check', { size: 16 }));
    else if (it.icon) iconCell.append(iconNode(it.icon, 16));
    const el = h('div', {
      class: ['fl-menu__item', it.disabled && 'is-disabled', it.danger && 'is-danger'],
      role: it.radio ? 'menuitemradio' : (it.checked !== undefined ? 'menuitemcheckbox' : 'menuitem'),
      'aria-checked': it.checked !== undefined || it.radio ? String(!!it.checked) : null,
      'aria-disabled': it.disabled ? 'true' : null,
      'aria-haspopup': it.submenu ? 'menu' : null,
      tabindex: -1,
    },
    stateCell,
    iconCell,
    h('span', { class: 'fl-menu__label' }, it.label,
      it.description ? h('span', { class: 'fl-menu__desc' }, it.description) : null),
    it.shortcut ? h('span', { class: 'fl-menu__shortcut' }, it.shortcut) : null,
    it.submenu ? h('span', { class: 'fl-menu__sub', 'aria-hidden': 'true' }, icon('chevron-right', { size: 12 })) : null);
    entries.push({ el, item: it });
    menu.append(el);
  }
  return { menu, entries };
}

/** Static menu element (galleries / previews). */
export function renderMenu(items, topActions, opts = {}) {
  return buildMenu(items, topActions, { isStatic: true, theme: opts.theme }).menu;
}

function defaultInset() {
  const s = document.documentElement.dataset.surface;
  const tb = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--taskbar-height')) || 48;
  return { bottom: s === 'desktop' || s === 'preview' ? tb : 0 };
}

function placeRoot(menu, { x, y, anchor, align, placement, inset }) {
  const m = 4;
  const vw = innerWidth, vh = innerHeight - (inset.bottom || 0);
  menu.style.maxHeight = '';
  let w = menu.offsetWidth, hgt = menu.offsetHeight;
  if (hgt > vh - 2 * m) { menu.style.maxHeight = `${vh - 2 * m}px`; hgt = menu.offsetHeight; }
  let left, top, up = false;
  if (anchor) {
    const r = anchor.getBoundingClientRect ? anchor.getBoundingClientRect() : anchor;
    left = align === 'end' ? r.right - w : (align === 'center' ? r.left + r.width / 2 - w / 2 : r.left);
    const below = r.bottom + 4, above = r.top - 4 - hgt;
    if (placement === 'top' ? above >= m : below + hgt > vh - m && above >= m) { top = above; up = true; } else top = below;
  } else {
    left = x; top = y;
    if (left + w > vw - m) left = x - w;
    if (top + hgt > vh - m) {
      if (y - hgt >= m) { top = y - hgt; up = true; } else top = vh - hgt - m;
    }
  }
  left = clamp(left, m, Math.max(m, vw - w - m));
  top = clamp(top, m, Math.max(m, vh - hgt - m));
  menu.style.left = `${Math.round(left)}px`;
  menu.style.top = `${Math.round(top)}px`;
  return up;
}

function placeSub(menu, parentItem, parentMenu, inset) {
  const m = 4;
  const vw = innerWidth, vh = innerHeight - (inset.bottom || 0);
  const ir = parentItem.getBoundingClientRect(), pr = parentMenu.getBoundingClientRect();
  menu.style.maxHeight = '';
  const w = menu.offsetWidth;
  let hgt = menu.offsetHeight;
  if (hgt > vh - 2 * m) { menu.style.maxHeight = `${vh - 2 * m}px`; hgt = menu.offsetHeight; }
  let left = pr.right - 4;
  let fromLeft = false;
  if (left + w > vw - m) { left = pr.left - w + 4; fromLeft = true; }
  let top = ir.top - 5;
  if (top + hgt > vh - m) top = vh - hgt - m;
  menu.style.left = `${Math.round(clamp(left, m, vw - w - m))}px`;
  menu.style.top = `${Math.round(Math.max(m, top))}px`;
  return fromLeft;
}

export function closeContextMenu() {
  current?.close('replaced');
  current = null;
}

export function isContextMenuOpen() { return !!current; }

/**
 * showContextMenu({x, y, items, topActions, anchor, align:'start'|'end'|'center', placement, onClose,
 *                  keyboard (focus first item), theme, inset:{bottom}}) → {close, update(items), el}
 * update(items) shows new items in the open menu in place (no re-open, open submenus and the highlight stay).
 */
export function showContextMenu({ x = 0, y = 0, items = [], topActions = [], anchor = null, onClose,
  align = 'start', placement, keyboard = false, theme, inset } = {}) {
  closeContextMenu();
  hideTooltip();
  const scopeTheme = theme || themeScope(anchor instanceof Element ? anchor : document.elementFromPoint?.(x, y));
  const ins = inset || defaultInset();
  const levels = []; // {menu, entries, active, parentEntry, subTimer}
  const restoreTo = document.activeElement;
  let dismiss = null;
  let release = null;
  let closed = false;

  function openLevel(itemsL, topL, parentLevel, parentEntry) {
    const { menu, entries } = buildMenu(itemsL, topL, { theme: scopeTheme });
    document.body.appendChild(menu);
    const level = { menu, entries, active: -1, parentEntry, subTimer: 0 };
    levels.push(level);
    // Win11 MenuFlyout entrance: a short slide + reveal from the anchor side (negative
    // insets keep the drop shadow visible), the items fade in. No opacity on the glass.
    const ease = 'cubic-bezier(0,0,0,1)';
    if (!parentLevel) {
      const up = placeRoot(menu, { x, y, anchor, align, placement, inset: ins });
      const dir = up ? 8 : -8;
      const from = up ? 'inset(45% -32px -32px -32px)' : 'inset(-32px -32px 45% -32px)';
      level.anims = [
        animate(menu, [{ transform: `translateY(${dir}px)`, clipPath: from }, { transform: 'none', clipPath: 'inset(-32px -32px -32px -32px)' }],
          { duration: 200, easing: ease }),
        ...fadeContent(menu, 'in', { duration: 133, easing: 'linear' }),
      ];
    } else {
      const fromLeft = placeSub(menu, parentEntry.el, parentLevel.menu, ins);
      level.anims = [
        animate(menu, [{ transform: `translateX(${fromLeft ? 8 : -8}px)` }, { transform: 'none' }], { duration: 167, easing: ease }),
        ...fadeContent(menu, 'in', { duration: 133, easing: 'linear' }),
      ];
      parentEntry.el.classList.add('has-open-submenu');
    }
    finished(level.anims);
    wireLevel(level);
    menu.focus({ preventScroll: true });
    return level;
  }

  function closeLevelsFrom(index) {
    while (levels.length > index) {
      const l = levels.pop();
      clearTimeout(l.subTimer);
      l.parentEntry?.el.classList.remove('has-open-submenu');
      l.menu.removeAttribute('data-region');
      for (const a of l.anims || []) { try { a?.cancel(); } catch { /* ignore */ } }
      // the glass fades itself (own opacity keeps its blur); never an ancestor
      finished(fadeContent(l.menu, 'out', { duration: 83, easing: 'linear', fill: 'forwards' })).then(() => l.menu.remove());
    }
  }

  function setActive(level, i, { focus = true } = {}) {
    level.entries.forEach((e, j) => e.el.classList.toggle('is-active', j === i));
    level.active = i;
    const e = level.entries[i];
    if (e && focus) e.el.focus({ preventScroll: false });
    else if (!e) level.menu.focus({ preventScroll: true });
  }

  function isEnabled(e) { return e && !e.item.disabled; }

  function move(level, delta) {
    const n = level.entries.length;
    if (!n) return;
    let i = level.active;
    for (let k = 0; k < n; k++) {
      i = i < 0 ? (delta > 0 ? 0 : n - 1) : (i + delta + n) % n;
      if (isEnabled(level.entries[i])) { setActive(level, i); return; }
    }
  }

  function openSub(level, entry) {
    const idx = levels.indexOf(level);
    closeLevelsFrom(idx + 1);
    if (!entry.item.submenu?.length || entry.item.disabled) return null;
    return openLevel(entry.item.submenu, [], level, entry);
  }

  function invoke(level, entry) {
    if (closed || !isEnabled(entry)) return; // a menu that is fading out takes no clicks
    if (entry.item.submenu) {
      const sub = openSub(level, entry);
      if (sub) move(sub, 1);
      return;
    }
    close('action');
    try { entry.item.action?.(entry.item); } catch (err) { console.error('[contextmenu] action failed', err); }
  }

  function wireLevel(level) {
    const { menu } = level; // level.entries is read on every event: update() replaces it
    menu.addEventListener('contextmenu', (e) => e.preventDefault());
    menu.addEventListener('pointermove', (e) => {
      const entries = level.entries;
      const el = e.target.closest('.fl-menu__item, .fl-menu__top-btn');
      const i = entries.findIndex((x) => x.el === el);
      if (i < 0 || i === level.active) return;
      if (entries[i].item.disabled) setActive(level, -1, { focus: false });
      else setActive(level, i, { focus: true });
      clearTimeout(level.subTimer);
      const entry = entries[i];
      const idx = levels.indexOf(level);
      level.subTimer = setTimeout(() => {
        if (closed || levels.indexOf(level) < 0) return;
        if (entry.item.submenu && !entry.item.disabled) {
          if (levels[idx + 1]?.parentEntry !== entry) openSub(level, entry);
        } else closeLevelsFrom(idx + 1);
      }, entry.item.submenu ? 250 : 400);
    });
    menu.addEventListener('pointerleave', () => {
      const idx = levels.indexOf(level);
      if (levels.length - 1 === idx) setActive(level, -1, { focus: false });
    });
    menu.addEventListener('pointerdown', (e) => {
      const el = e.target.closest('.fl-menu__item');
      if (el && !el.classList.contains('is-disabled')) el.classList.add('is-pressed');
      e.preventDefault(); // keep focus in the menu
    });
    menu.addEventListener('pointerup', () => menu.querySelectorAll('.is-pressed').forEach((x) => x.classList.remove('is-pressed')));
    menu.addEventListener('click', (e) => {
      const el = e.target.closest('.fl-menu__item, .fl-menu__top-btn');
      const entry = level.entries.find((x) => x.el === el);
      if (entry) invoke(level, entry);
    });
  }

  /**
   * Show new items in the open menu without re-opening it (a live refresh): every level keeps its element,
   * place and data-region; an open submenu stays open while the entry at its index still has one; the
   * highlighted entry and the focus stay. No entrance animation.
   */
  function update(newItems) {
    if (closed) return;
    const hadFocus = levels.some((l) => l.menu.contains(document.activeElement));
    let itemsL = newItems;
    for (let d = 0; d < levels.length; d++) {
      const level = levels[d];
      const next = levels[d + 1];
      const pathIdx = next ? level.entries.indexOf(next.parentEntry) : -1;
      const w0 = level.menu.offsetWidth, h0 = level.menu.offsetHeight;
      clearTimeout(level.subTimer); // it would act on an entry that is gone
      const { menu: fresh, entries } = buildMenu(itemsL, d ? [] : topActions, { theme: scopeTheme });
      level.menu.className = fresh.className;
      level.menu.replaceChildren(...fresh.childNodes);
      level.entries = entries;
      const a = Math.min(level.active, entries.length - 1);
      level.active = a >= 0 && isEnabled(entries[a]) ? a : -1;
      entries.forEach((e, j) => e.el.classList.toggle('is-active', j === level.active));
      if (level.menu.offsetWidth !== w0 || level.menu.offsetHeight !== h0) {
        if (d === 0) placeRoot(level.menu, { x, y, anchor, align, placement, inset: ins }); // keeps an upward menu's bottom
        else placeSub(level.menu, level.parentEntry.el, levels[d - 1].menu, ins);
      }
      if (!next) break;
      const pe = entries[pathIdx];
      if (!pe || !pe.item.submenu?.length || pe.item.disabled) { closeLevelsFrom(d + 1); break; }
      next.parentEntry = pe;
      pe.el.classList.add('has-open-submenu');
      itemsL = pe.item.submenu;
    }
    if (hadFocus) {
      const deep = levels[levels.length - 1];
      const e = deep?.entries[deep.active];
      (e ? e.el : deep?.menu)?.focus({ preventScroll: true });
    }
  }

  function onKey(e) {
    if (closed) return;
    const level = levels[levels.length - 1];
    if (!level) return;
    const entry = level.entries[level.active];
    const handled = () => { e.preventDefault(); e.stopPropagation(); };
    switch (e.key) {
      case 'Escape':
        handled();
        if (levels.length > 1) {
          const parent = levels[levels.length - 2];
          closeLevelsFrom(levels.length - 1);
          setActive(parent, parent.active);
        } else close('escape');
        break;
      case 'ArrowDown': handled(); move(level, 1); break;
      case 'ArrowUp': handled(); move(level, -1); break;
      case 'Home': handled(); level.active = -1; move(level, 1); break;
      case 'End': handled(); level.active = -1; move(level, -1); break;
      case 'ArrowRight':
        handled();
        if (entry?.top) move(level, 1);
        else if (entry?.item.submenu) { const sub = openSub(level, entry); if (sub) move(sub, 1); }
        break;
      case 'ArrowLeft':
        handled();
        if (entry?.top) move(level, -1);
        else if (levels.length > 1) {
          const parent = levels[levels.length - 2];
          closeLevelsFrom(levels.length - 1);
          setActive(parent, parent.active);
        }
        break;
      case 'Enter': case ' ':
        handled();
        if (entry) invoke(level, entry);
        break;
      case 'Tab': handled(); move(level, e.shiftKey ? -1 : 1); break;
      default:
        if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
          const ch = e.key.toLowerCase();
          const start = level.active;
          const n = level.entries.length;
          for (let k = 1; k <= n; k++) {
            const j = (start + k + n) % n;
            const it = level.entries[j];
            if (!it.top && isEnabled(it) && String(it.item.label || '').toLowerCase().startsWith(ch)) {
              handled(); setActive(level, j); break;
            }
          }
        }
    }
  }

  function inAnyMenu(target) { return levels.some((l) => l.menu.contains(target)); }
  const onDown = (e) => { if (!inAnyMenu(e.target)) close('outside'); };
  const onBlur = () => close('blur');
  const vw0 = innerWidth, vh0 = innerHeight;
  const onResize = () => { if (innerWidth !== vw0 || innerHeight !== vh0) close('resize'); };
  const onWheel = (e) => { if (!inAnyMenu(e.target)) close('scroll'); };

  function close(reason = 'close') {
    if (closed) return;
    closed = true;
    document.removeEventListener('keydown', onKey, true);
    document.removeEventListener('pointerdown', onDown, true);
    document.removeEventListener('wheel', onWheel, true);
    removeEventListener('blur', onBlur);
    removeEventListener('resize', onResize);
    closeLevelsFrom(0);
    dismiss?.remove();
    release?.();
    if (current === api) current = null;
    if (reason !== 'outside' && reason !== 'blur' && restoreTo && restoreTo !== document.body) {
      try { restoreTo.focus({ preventScroll: true }); } catch { /* ignore */ }
    }
    try { onClose?.(reason); } catch (e) { console.error(e); }
  }

  if (regionsActive()) {
    // Layer mode: clicks outside our input region never reach us → full-screen catcher.
    dismiss = h('div', { class: 'fl-dismiss', 'data-region': '', 'data-region-margin': '0', style: { zIndex: 'calc(var(--z-menu) - 1)' } });
    dismiss.addEventListener('pointerdown', (e) => { e.preventDefault(); close('outside'); });
    dismiss.addEventListener('contextmenu', (e) => e.preventDefault());
    document.body.appendChild(dismiss);
    release = holdKeyboard();
  }

  const root = openLevel(items, topActions, null, null);
  if (keyboard) move(root, 1);

  setTimeout(() => {
    if (closed) return;
    document.addEventListener('pointerdown', onDown, true);
    document.addEventListener('wheel', onWheel, true);
    addEventListener('blur', onBlur);
    addEventListener('resize', onResize);
  }, 0);
  document.addEventListener('keydown', onKey, true);

  const api = { close, update, el: root.menu };
  current = api;
  return api;
}

/**
 * bindContextMenu(el, provider) — right click / Menu key / Shift+F10 on `el` opens the menu
 * returned by provider(event) ({items, topActions, …} or null to fall through). Returns unbind().
 */
export function bindContextMenu(el, provider) {
  const onCtx = (e) => {
    const opts = provider(e);
    if (!opts) return;
    e.preventDefault();
    e.stopPropagation();
    showContextMenu({ x: e.clientX, y: e.clientY, ...opts });
  };
  const onKey = (e) => {
    if (!(e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10'))) return;
    const opts = provider(e);
    if (!opts) return;
    e.preventDefault();
    const target = document.activeElement && el.contains(document.activeElement) ? document.activeElement : el;
    const r = target.getBoundingClientRect();
    showContextMenu({ x: r.left + Math.min(24, r.width / 2), y: r.top + Math.min(r.height, 28), keyboard: true, ...opts });
  };
  el.addEventListener('contextmenu', onCtx);
  el.addEventListener('keydown', onKey);
  return () => { el.removeEventListener('contextmenu', onCtx); el.removeEventListener('keydown', onKey); };
}
