// Win11 Shell — Schijven (Disks). SPEC §11: export async function mountApp(root, params).
//
// Windows 11 Instellingen › Systeem › Opslag › "Schijven en volumes" merged with Schijfbeheer: a card per disk
// with a proportional partition bar (click to select, right-click for actions) and its volumes; "Eigenschappen"
// pages for a volume (label, koppelpunt, versleuteling, formatteren, verwijderen) and a disk (schijfstatus,
// veilig verwijderen, initialiseren, nieuw volume). Live: event disks.changed (hotplug, mounts, job progress).
// Deep links: ?volume=<id|/dev/…> ?disk=<id|/dev/…> ?select=<id|/dev/…> (+ debug/mock: open=format|label|delete|
// unlock|new|initialize, menu (the selected volume's context menu), disk-menu, free-menu (the first free region's))
// and app.navigate {volume|disk|select}.

import { createAppFrame } from '../appframe.js';
import { h, loadCss, animate } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { Breadcrumb, Button, InfoBar, EmptyState, ProgressRing } from '../../core/components.js';
import { getShellInfo } from '../../core/theme.js';
import { onLangChange, lang } from '../../core/i18n.js';
import { t } from './i18n.js';
import { appGlyph } from './util.js';
import { DiskCard, Legend } from './overview.js';
import { VolumePage, DiskPage } from './properties.js';
import * as act from './actions.js';
import { showContextMenu } from '../../core/contextmenu.js';

const POLL_MS = 30000;

function findVolume(snap, key) {
  if (!snap || !key) return null;
  for (const d of snap.disks || []) {
    for (const v of d.volumes) {
      if (v.id === key || v.device === key || v.encrypted?.cleartext === key || v.label === key) return { d, v };
    }
  }
  return null;
}

function findDisk(snap, key) {
  if (!snap || !key) return null;
  return (snap.disks || []).find((d) => d.id === key || d.block === key || d.device === key || d.name === key) || null;
}

export async function mountApp(root, params = {}) {
  await loadCss('disks');
  const frame = createAppFrame({ app: 'disks', title: t('appTitle'), icon: appGlyph(16), back: true });
  frame.el.classList.add('dk-app');
  root.append(frame.el);

  let snap = null;
  let loadError = null;
  let view = { view: 'overview' };
  const history = [];
  let selected = null;
  let lastRenderKey = '';

  const ctx = {
    get snap() { return snap; },
    get selected() { return selected; },
    alive: () => true,
    select(id, { focus = false } = {}) {
      selected = id;
      for (const el of frame.body.querySelectorAll('.dk-seg[data-id], .dk-vol[data-id], .dk-seg-group[data-id]')) {
        const on = el.dataset.id === id;
        el.classList.toggle('is-selected', on);
        if (el.classList.contains('dk-seg')) el.setAttribute('aria-selected', String(on));
        if (on && focus) el.focus({ preventScroll: false });
      }
      // roving tabindex per bar: only the bar holding the selection moves its tab stop; every other bar keeps one
      const seg = [...frame.body.querySelectorAll('.dk-seg[data-id]')].find((el) => el.dataset.id === id);
      const bar = seg?.closest('.dk-bar');
      if (bar) for (const el of bar.querySelectorAll('.dk-seg[data-id]')) el.tabIndex = el === seg ? 0 : -1;
    },
    openVolume: (id) => go({ view: 'volume', id }),
    openDisk: (id) => go({ view: 'disk', id }),
    reload: () => load(),
    volumeGone: (id) => { if (view.view === 'volume' && view.id === id) back(); },
    diskGone: (id) => { if (view.view !== 'overview' && (view.id === id || findVolume(snap, view.id)?.d.id === id)) go({ view: 'overview' }, { push: false }); },
  };

  /* ------------------------------------------------------------ navigation */
  function go(next, { push = true } = {}) {
    if (push) history.push({ ...view, scroll: frame.body.scrollTop });
    view = next;
    frame.setBackEnabled(history.length > 0);
    render({ animate: true, scrollTop: 0 });
  }
  function back() {
    const prev = history.pop();
    frame.setBackEnabled(history.length > 0);
    if (!prev) return;
    view = prev;
    render({ animate: true, scrollTop: prev.scroll || 0 });
  }
  frame.onBack(back);

  /* ------------------------------------------------------------ data */
  // A load is sent in the language of that moment (X-Shell-Lang): after a language switch one still running is not
  // reused, and its late answer never overwrites the newer one.
  let loading = null, loadingLang = null, loadGen = 0;
  async function load(refresh = false) {
    if (loading && loadingLang === lang()) return loading;
    const gen = ++loadGen;
    loadingLang = lang();
    loading = (async () => {
      let res, err = null;
      try {
        res = await bridge.call('disks.list', refresh ? { refresh: true } : {});
      } catch (e) {
        err = e;
      }
      if (gen !== loadGen) return;
      if (err) loadError = err; else { snap = res; loadError = null; }
      loading = null;
      render();
    })();
    return loading;
  }
  bridge.on('disks.changed', (d) => { if (d && Array.isArray(d.disks)) { snap = d; render(); } });
  setInterval(() => { if (document.visibilityState === 'visible' && !document.documentElement.classList.contains('has-dialog')) load(); }, POLL_MS);
  addEventListener('focus', () => load());

  /* ------------------------------------------------------------ rendering */
  function header(crumbs, cmds = []) {
    const bc = Breadcrumb({
      items: crumbs, size: 'title',
      onSelect: (id) => { if (id === 'overview') { history.length = 0; frame.setBackEnabled(false); view = { view: 'overview' }; render({ animate: true, scrollTop: 0 }); } else if (id?.startsWith('disk:')) go({ view: 'disk', id: id.slice(5) }); },
    });
    return h('header', { class: 'dk-head' }, bc, h('div', { class: 'dk-cmds' }, ...cmds));
  }

  function notices() {
    const out = [];
    const info = getShellInfo() || {};
    if (!bridge.isMock && info.dryRun) out.push(InfoBar({ severity: 'info', title: '', message: t('dryRun'), closable: true }));
    if (snap?.readOnly) out.push(InfoBar({ severity: 'info', title: '', message: t('readOnly'), closable: true }));
    return out;
  }

  function overviewPage() {
    const refreshBtn = Button({ label: t('refresh'), icon: 'refresh', variant: 'subtle', onClick: () => load(true) });
    const body = [];
    if (!snap) {
      body.push(loadError
        ? EmptyState({ icon: 'warning', title: t('unavailable'), message: loadError.message, action: { label: t('refresh'), onClick: () => load(true) } })
        : h('div', { class: 'dk-loading' }, ProgressRing({ size: 32 }), h('div', null, t('loading'))));
    } else if (!snap.supported) {
      body.push(EmptyState({ icon: 'warning', title: t('unavailable'), message: snap.error || '', action: { label: t('refresh'), onClick: () => load(true) } }));
    } else if (!snap.disks.length) {
      body.push(EmptyState({ icon: 'disk', title: t('noDisks'), message: t('noDisksBody') }));
    } else {
      body.push(h('div', { class: 'dk-disks' }, ...snap.disks.map((d) => DiskCard(ctx, d))), Legend(snap));
    }
    return [header([{ id: 'overview', label: t('overview') }], [refreshBtn]), ...notices(), ...body];
  }

  function volumePage() {
    const hit = findVolume(snap, view.id);
    if (!hit) return null;
    return [header([{ id: 'overview', label: t('overview') }, { id: `disk:${hit.d.id}`, label: hit.d.name }, { id: 'v', label: hit.v.name }]),
      ...notices(), ...VolumePage(ctx, hit.d, hit.v)];
  }

  function diskPage() {
    const d = findDisk(snap, view.id);
    if (!d) return null;
    return [header([{ id: 'overview', label: t('overview') }, { id: 'd', label: `${d.name}` }]), ...notices(), ...DiskPage(ctx, d)];
  }

  function render({ animate: anim = false, scrollTop } = {}) {
    if (document.documentElement.classList.contains('has-dialog') && !anim) {
      // a dialog is open (format running …): refresh after it closes, keep the page stable behind it
      pendingRender = true;
      return;
    }
    let nodes = null;
    if (snap && view.view === 'volume') nodes = volumePage();
    else if (snap && view.view === 'disk') nodes = diskPage();
    if (!nodes) {
      if (view.view !== 'overview' && snap) { view = { view: 'overview' }; history.length = 0; frame.setBackEnabled(false); }
      nodes = overviewPage();
    }
    const keepScroll = scrollTop ?? frame.body.scrollTop;
    const page = h('div', { class: ['dk-page', `dk-page--${view.view}`] }, ...nodes);
    frame.setContent(page);
    frame.body.scrollTop = keepScroll;
    const key = `${view.view}:${view.id || ''}`;
    if (anim && key !== lastRenderKey) {
      animate(page, [{ opacity: 0, transform: 'translateY(28px)' }, { opacity: 1, transform: 'none' }], { duration: 300, easing: 'cubic-bezier(0,0,0,1)' });
    }
    lastRenderKey = key;
    document.title = view.view === 'overview' ? t('appTitle') : `${t('appTitle')} – ${nodes[0]?.querySelector?.('.is-current')?.textContent || ''}`;
  }
  let pendingRender = false;
  new MutationObserver(() => {
    if (pendingRender && !document.documentElement.classList.contains('has-dialog')) { pendingRender = false; render(); }
  }).observe(document.documentElement, { attributes: true, attributeFilter: ['class'] });

  /* ------------------------------------------------------------ deep links */
  function applyParams(p = {}) {
    if (!snap) return;
    const vk = p.volume || (p.page === 'volume' ? p.id : null);
    const dk = p.disk || (p.page === 'disk' ? p.id : null);
    if (vk) { const hit = findVolume(snap, vk); if (hit) go({ view: 'volume', id: hit.v.id }); }
    else if (dk) { const d = findDisk(snap, dk); if (d) go({ view: 'disk', id: d.id }); }
    else if (p.page === 'overview') { history.length = 0; view = { view: 'overview' }; frame.setBackEnabled(false); render({ animate: true }); }
    if (p.select) {
      const hit = findVolume(snap, p.select);
      ctx.select(hit ? hit.v.id : p.select);
    }
    const dlg = p.open;
    if (dlg) {
      const hit = findVolume(snap, p.select || vk);
      const d = hit?.d || findDisk(snap, dk);
      setTimeout(() => {
        if (dlg === 'format' && hit) act.format(ctx, hit.d, hit.v);
        else if (dlg === 'label' && hit) act.changeLabel(ctx, hit.v);
        else if (dlg === 'delete' && hit) act.deleteVolume(ctx, hit.d, hit.v);
        else if (dlg === 'unlock' && hit) act.unlock(ctx, hit.v);
        else if (dlg === 'new' && d?.free?.length) act.newVolume(ctx, d, d.free[0]);
        else if (dlg === 'initialize' && d) act.initialize(ctx, d);
        else if (dlg === 'menu' && hit) debugMenu(act.volumeMenu(ctx, hit.d, hit.v), hit.v.id);
        else if (dlg === 'disk-menu' && d) debugMenu(act.diskMenu(ctx, d), null);
        else if (dlg === 'free-menu' && d?.free?.length) debugMenu(act.freeMenu(ctx, d, d.free[0]), null);
      }, 350);
    }
  }
  /** ?open=menu|disk-menu|free-menu: a context menu, at its segment when it is on screen. */
  function debugMenu(items, id) {
    const el = id ? frame.body.querySelector(`.dk-seg[data-id="${CSS.escape(id)}"]`) : frame.body.querySelector('.dk-seg, .dk-card');
    const r = el?.getBoundingClientRect();
    showContextMenu({ x: Math.round(r ? r.left + Math.min(r.width / 2, 80) : 300), y: Math.round(r ? r.top + r.height / 2 : 240), items });
  }
  frame.onAppNavigate((p) => applyParams(p || {}));
  // a language switch: our labels at once, then the host's texts (partition types, health …) in the new language
  onLangChange(() => { frame.setTitle(t('appTitle')); render(); load(); });

  document.addEventListener('keydown', (e) => {
    if (document.documentElement.classList.contains('has-dialog')) return;
    if (e.key === 'F5') { e.preventDefault(); load(true); }
  });

  render();
  await load();
  // the first page: a deep link opens on top of the overview (Back / the breadcrumb lead to it)
  if (params.volume || params.disk || params.select || params.open) applyParams({ ...params });
  return frame;
}
