// Taakbeheer — Opslag: volume cards with big usage bars, then four views:
// Verkennen (sunburst explorer), Categorieën, Grote bestanden and Opruimen.

import { h, clear, animate } from '../../../core/dom.js';
import { Button, Tabs, AppIcon } from '../../../core/components.js';
import { makeT } from '../../../core/i18n.js';
import { bytes } from '../../../core/format.js';
import { load, save, prettyPath, volPct } from '../util.js';
import { createScanner, createExplorer } from '../storage/explorer.js';
import { createLargeFiles } from '../storage/largefiles.js';
import { createCleanup } from '../storage/cleanup.js';
import { createCategories } from '../storage/categories.js';
import { arcColor } from '../storage/sunburst.js';

const t = makeT({
  nl: {
    title: 'Opslag', refresh: 'Vernieuwen', explore: 'Verkennen', categories: 'Categorieën', large: 'Grote bestanden', cleanup: 'Opruimen',
    free: '{free} vrij van {total}', almostFull: 'Bijna vol', full: 'Vol', home: 'Persoonlijke map', removable: 'Verwisselbaar',
    used: '{used} gebruikt', otherUsed: 'Overig gebruikt', freeLabel: 'Vrij', volumes: 'Schijven', loading: 'Schijven laden…',
  },
  en: {
    title: 'Storage', refresh: 'Refresh', explore: 'Explore', categories: 'Categories', large: 'Large files', cleanup: 'Clean up',
    free: '{free} free of {total}', almostFull: 'Almost full', full: 'Full', home: 'Home folder', removable: 'Removable',
    used: '{used} used', otherUsed: 'Other used', freeLabel: 'Free', volumes: 'Disks', loading: 'Loading disks…',
  },
});

const TABS = ['explore', 'categories', 'large', 'cleanup'];
/** "ext4" → "EXT4", already display-ready names ("NTFS", "exFAT") stay as they are. */
const fsLabel = (fs) => (!fs ? '' : /^[a-z0-9]+$/.test(fs) ? fs.toUpperCase() : fs);

export function createPage(ctx) {
  const { store, bridge } = ctx;
  const scanner = createScanner(bridge);
  let vols = null;
  let tab = TABS.includes(load('storage.tab', 'explore')) ? load('storage.tab', 'explore') : 'explore';
  let scope = null;
  const offs = [];

  const homePath = () => categories.home || (ctx.user ? `/home/${ctx.user}` : '/home');
  const homeScope = () => ({ path: homePath(), label: t('home') });

  const explorer = createExplorer(ctx, {
    scanner,
    onLargeFiles: (path, name) => { setScope({ path, label: name }, { scan: false }); setTab('large'); },
  });
  const large = createLargeFiles(ctx, { onChanged: () => store.refresh('volumes') });
  const cleanup = createCleanup(ctx, {
    onChanged: () => { store.refresh('volumes'); store.refresh('health'); },
    onShowFiles: () => { setScope({ path: `${homePath()}/Downloads`, label: 'Downloads' }, { scan: false }); setTab('large'); },
  });
  const categories = createCategories(ctx, {
    onExplore: (path, name) => { setScope({ path, label: name }); setTab('explore'); },
  });

  const volRow = h('div', { class: 'tm-vols', role: 'list', 'aria-label': t('volumes') }, h('div', { class: 'tm-vols__loading caption text-2' }, t('loading')));
  // The tab strip is rebuilt when a deep link picks another tab before it is on screen: core Tabs places its pill
  // in a deferred frame with the tab it was created with, so select() right after creation could leave it behind.
  const tabsWrap = h('div', { class: 'tm-storage__tabs' });
  let tabs = null;
  function makeTabs() {
    tabs = Tabs({
      items: TABS.map((id) => ({ id, label: t(id) })), selected: tab, ariaLabel: t('title'),
      onSelect: (id) => setTab(id, false),
    });
    clear(tabsWrap).append(tabs);
  }
  makeTabs();
  const view = h('div', { class: 'tm-storage__view' });
  const el = h('div', { class: 'tm-scroll tm-storage' }, volRow, tabsWrap, view);

  scanner.onTree((path) => { if (vols && vols.some((v) => v.mount === path)) renderVols(); });

  /* ---------------------------------------------------------------- volumes */
  function volOf(path) {
    if (!vols || !path) return null;
    return [...vols].sort((a, b) => b.mount.length - a.mount.length)
      .find((v) => path === v.mount || path.startsWith(v.mount === '/' ? '/' : `${v.mount}/`)) || null;
  }

  function renderVols() {
    if (!vols) return;
    clear(volRow);
    const sel = volOf(scope?.path || homePath());
    const full = (v) => volPct(v) >= 90;
    // system disk first, then disks that are (almost) full, then the rest in the host's order
    const list = vols.filter((v) => !v.hidden).sort((a, b) => (b.isSystem - a.isSystem) || (b.isHome - a.isHome) || (full(b) - full(a)));
    for (const v of list) {
      const pct = volPct(v);
      const level = pct >= 97 ? 'critical' : pct >= 90 ? 'warning' : 'ok';
      const bar = h('div', { class: 'tm-vol__bar', role: 'img', 'aria-label': `${Math.round(pct)}%` });
      const tree = scanner.trees.get(v.mount);
      if (tree && tree.children?.length && level === 'ok') {
        let shown = 0;
        tree.children.filter((c) => !c.other && c.size > 0).slice(0, 5).forEach((c, i) => {
          shown += c.size;
          bar.append(h('span', { style: { width: `${(c.size / v.total) * 100}%`, background: arcColor(i, 1, c) }, dataset: { tooltip: `${c.name}: ${bytes(c.size)}` } }));
        });
        const rest = Math.max(0, v.used - shown);
        bar.append(h('span', { class: 'is-rest', style: { width: `${(rest / v.total) * 100}%` }, dataset: { tooltip: `${t('otherUsed')}: ${bytes(rest)}` } }));
      } else {
        bar.append(h('span', { class: 'is-used', style: { width: `${Math.min(100, pct)}%` }, dataset: { tooltip: t('used', { used: bytes(v.used) }) } }));
      }
      const card = h('button', {
        type: 'button', class: ['tm-vol', `is-${level}`, sel === v && 'is-selected'], role: 'listitem',
        'aria-pressed': String(sel === v), dataset: { mount: v.mount },
      },
      h('div', { class: 'tm-vol__top' },
        AppIcon({ name: v.removable ? 'drive-removable-media' : 'drive-harddisk', size: 32, fallback: 'drive' }),
        h('div', { class: 'tm-vol__names' },
          h('div', { class: 'tm-vol__name' }, v.name || v.label || v.mount),
          // automounted drives (/run/media/<user>/<label or UUID>) are known by their name already: the folder
          // only in the tooltip, so no raw UUIDs on screen
          h('div', { class: 'tm-vol__mount', dataset: { tooltip: v.mount } },
            [/^\/(?:run\/media|media|mnt)\//.test(v.mount) ? null : v.mount, fsLabel(v.fsType)].filter(Boolean).join(' · '))),
        h('div', { class: 'tm-vol__pct' }, `${Math.round(pct)}%`)),
      bar,
      h('div', { class: 'tm-vol__foot' },
        h('span', null, t('free', { free: bytes(v.free), total: bytes(v.total) })),
        level !== 'ok' ? h('span', { class: `tm-pill is-${level === 'critical' ? 'critical' : 'warning'}` }, level === 'critical' ? t('full') : t('almostFull')) : null));
      card.addEventListener('click', () => {
        setScope({ path: v.mount, label: v.name || v.mount });
        if (tab !== 'explore' && tab !== 'large') setTab('explore');
      });
      volRow.append(card);
    }
  }

  /* ------------------------------------------------------------------- views */
  function setScope(s, { scan = true } = {}) {
    scope = s;
    renderVols();
    if (tab === 'explore' && scan) explorer.setScope(scope);
    else if (tab === 'large') large.setScope(scope);
  }

  function setTab(id, select = true) {
    if (!TABS.includes(id)) return;
    tab = id;
    save('storage.tab', id);
    if (select) tabs.select(id);
    showTab(true);
  }

  function showTab(anim = false) {
    clear(view);
    if (!scope) scope = homeScope();
    let node;
    if (tab === 'explore') { node = explorer.el; explorer.setScope(scope); }
    else if (tab === 'categories') { node = categories.el; categories.load(); }
    else if (tab === 'large') { node = large.el; large.setScope(scope); }
    else { node = cleanup.el; cleanup.load(); }
    view.append(node);
    if (anim) animate(node, [{ opacity: 0, transform: 'translateY(12px)' }, { opacity: 1, transform: 'none' }], { duration: 250 });
    renderVols();
  }

  function applyParams(p = {}) {
    if (p.volume || p.path) {
      // health actions pass the volume id; deep links may pass a mount point
      const v = p.volume ? vols?.find((x) => x.id === p.volume || x.mount === p.volume) : null;
      const path = p.path || v?.mount || (String(p.volume || '').startsWith('/') ? p.volume : null);
      if (path) scope = { path, label: v ? (v.name || v.mount) : (p.label || prettyPath(path)) };
    }
    if (p.tab && TABS.includes(p.tab) && p.tab !== tab) { tab = p.tab; save('storage.tab', tab); makeTabs(); }
    showTab(false);
  }

  const refreshBtn = Button({
    label: t('refresh'), icon: 'refresh', variant: 'subtle', onClick: () => {
      store.refresh('volumes');
      if (tab === 'explore') explorer.refresh();
      else if (tab === 'categories') categories.refresh();
      else if (tab === 'large') large.refresh();
      else cleanup.refresh();
    },
  });

  return {
    id: 'storage',
    title: t('title'),
    el,
    commands: [refreshBtn],
    mount(params = {}) {
      // with ?volume= wait for the volume list so the card and label are right
      let applied = !params.volume;
      offs.push(store.subscribe('volumes', (d) => {
        vols = Array.isArray(d) ? d : [];
        if (!applied) { applied = true; applyParams(params); } else renderVols();
      }));
      if (!params.volume) applyParams(params);
    },
    navigate(params) { applyParams(params); },
    unmount() { while (offs.length) offs.pop()(); },
    refresh() { refreshBtn.click(); },
  };
}
