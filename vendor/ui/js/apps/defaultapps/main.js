// Standalone Default Apps. Reuses the original window chrome and Fluent Settings controls.
import { createAppFrame } from '../appframe.js';
import { h, clear, loadCss } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { makeT, onLangChange } from '../../core/i18n.js';
import { AppIcon, Breadcrumb, Button, Dialog, EmptyState, InfoBar, ProgressRing, SearchBox, SettingsCard, SettingsGroup, Toast } from '../../core/components.js';

const t = makeT({
  nl: {
    title: 'Standaard-apps', intro: 'Kies welke app een bestandstype of koppeling opent.',
    search: 'Zoek een extensie of bestandstype', example: 'Bijvoorbeeld .pdf, .jpg of text/plain',
    choose: 'Kies een standaard-app', none: 'Geen standaard-app', current: 'Huidige standaard',
    loading: 'Bestandstypen laden…', loadingApps: 'Beschikbare apps laden…',
    empty: 'Geen bestandstypen gevonden', emptyBody: 'Probeer een andere extensie of zoekterm.',
    failed: 'Standaard-apps konden niet worden geladen', retry: 'Opnieuw proberen',
    range: '{from}–{to} van {total} bestandstypen', previous: 'Vorige', next: 'Volgende',
    use: 'Als standaard instellen', cancel: 'Annuleren',
    chooseFor: 'Een app kiezen voor {type}', relevant: 'Apps die dit bestandstype kunnen openen',
    noApps: 'Geen geschikte apps gevonden', noAppsBody: 'Installeer een app die dit bestandstype ondersteunt en probeer het opnieuw.',
    saved: '{app} is nu de standaard voor {type}', saveFailed: 'De standaard-app kon niet worden gewijzigd.',
    overridden: 'De app is opgeslagen, maar een andere bureaubladinstelling heeft voorrang.',
    overrides: 'Deze bestanden hebben voorrang: {files}',
  },
  en: {
    title: 'Default apps', intro: 'Choose which app opens a file type or link.',
    search: 'Search for an extension or file type', example: 'For example .pdf, .jpg or text/plain',
    choose: 'Choose a default app', none: 'No default app', current: 'Current default',
    loading: 'Loading file types…', loadingApps: 'Loading available apps…',
    empty: 'No file types found', emptyBody: 'Try another extension or search term.',
    failed: 'Default apps couldn’t be loaded', retry: 'Try again',
    range: '{from}–{to} of {total} file types', previous: 'Previous', next: 'Next',
    use: 'Set default', cancel: 'Cancel',
    chooseFor: 'Choose an app for {type}', relevant: 'Apps that can open this file type',
    noApps: 'No suitable apps found', noAppsBody: 'Install an app that supports this file type and try again.',
    saved: '{app} is now the default for {type}', saveFailed: 'The default app couldn’t be changed.',
    overridden: 'The app was saved, but another desktop setting takes precedence.',
    overrides: 'These files take precedence: {files}',
  },
});
const PAGE_SIZE = 100;

export async function mountApp(root, params = {}) {
  await loadCss('defaultapps');
  const frame = createAppFrame({ app: 'defaultapps', title: t('title'), icon: 'apps', back: false });
  frame.el.classList.add('da-app');
  root.append(frame.el);
  let query = String(params.query || params.extension || params.mime || '');
  let offset = 0, snapshot = null, failure = null, serial = 0, timer = 0, activeDialog = null;
  let selecting = false;
  const list = h('div', { class: 'da-list', 'aria-live': 'polite', 'aria-busy': 'true' });
  const pageInfo = h('span', { class: 'da-range' });
  const previous = Button({ label: t('previous'), icon: 'chevron-left', onClick: () => { offset = Math.max(0, offset - PAGE_SIZE); load(); } });
  const next = Button({ label: t('next'), icon: 'chevron-right', onClick: () => { offset += PAGE_SIZE; load(); } });
  const search = SearchBox({ value: query, placeholder: t('search'), ariaLabel: t('search'),
    onInput(value) { query = value; offset = 0; clearTimeout(timer); timer = setTimeout(load, 180); },
    onSubmit(value) { query = value; offset = 0; clearTimeout(timer); load(); } });
  const title = h('h1', { class: 'da-title' }, t('title'));
  const intro = h('p', { class: 'da-intro' }, t('intro'));
  const example = h('p', { class: 'da-example' }, t('example'));
  const page = h('section', { class: 'da-page' }, h('header', { class: 'da-header' }, title, intro),
    h('section', { class: 'da-search' }, search, example), list,
    h('nav', { class: 'da-pagination', 'aria-label': t('title') }, pageInfo, h('div', { class: 'da-paging-buttons' }, previous, next)));
  frame.setContent(page);

  function labels() {
    frame.setTitle(t('title')); title.textContent = t('title'); intro.textContent = t('intro'); example.textContent = t('example');
    search.input.placeholder = t('search'); search.input.setAttribute('aria-label', t('search'));
    previous.setLabel(t('previous')); previous.setAttribute('aria-label', t('previous'));
    next.setLabel(t('next')); next.setAttribute('aria-label', t('next'));
    page.querySelector('.da-pagination').setAttribute('aria-label', t('title'));
  }
  function render() {
    clear(list); list.setAttribute('aria-busy', String(!snapshot && !failure));
    const items = snapshot?.items || [];
    if (failure) list.append(EmptyState({ icon: 'warning', title: t('failed'), message: failure.message, action: { label: t('retry'), onClick: load } }));
    else if (!snapshot) list.append(h('div', { class: 'da-loading' }, ProgressRing({ size: 28 }), h('span', null, t('loading'))));
    else if (!items.length) list.append(EmptyState({ icon: 'search', title: t('empty'), message: t('emptyBody') }));
    else list.append(SettingsGroup({ children: items.map((item) => {
      const name = (item.extensions || []).slice(0, 5).join(', ') || item.name || item.mime;
      const current = item.default;
      const control = h('span', { class: 'da-default' }, current ? AppIcon({ name: current.icon || current.id, size: 24 }) : AppIcon({ mime: item.mime, size: 24 }),
        h('span', { class: 'da-default-name' }, current?.name || t('choose')));
      const row = SettingsCard({ icon: AppIcon({ mime: item.mime, size: 32 }), title: name,
        description: h('span', null, h('span', { class: 'da-type-name' }, item.name), h('span', { class: 'da-mime' }, item.mime)),
        control, onClick: () => choose(item), chevron: true, cls: 'da-type' });
      row.dataset.mime = item.mime; row.setAttribute('aria-label', `${name}: ${current?.name || t('none')}`);
      return row;
    }) }));
    const total = snapshot?.total || 0;
    pageInfo.textContent = snapshot ? t('range', { from: total ? offset + 1 : 0, to: Math.min(offset + items.length, total), total }) : '';
    previous.disabled = !snapshot || offset <= 0; next.disabled = !snapshot || offset + items.length >= total;
  }
  async function load() {
    const request = ++serial;
    list.setAttribute('aria-busy', 'true');
    try {
      const value = await bridge.call('apps.mimeTypes', { query, offset, limit: PAGE_SIZE });
      if (request !== serial) return;
      snapshot = value; failure = null;
      if (offset && !value.items?.length && value.total > 0) { offset = Math.floor((value.total - 1) / PAGE_SIZE) * PAGE_SIZE; return load(); }
    } catch (e) { if (request !== serial) return; failure = e; }
    render();
  }
  async function choose(item) {
    if (selecting) return;
    selecting = true;
    let detail;
    try { detail = await bridge.call('apps.forType', { mime: item.mime }); }
    catch (e) { selecting = false; Toast.show({ severity: 'error', title: t('failed'), message: e.message }); return; }
    selecting = false;
    let selected = detail.default?.id || '';
    const choices = h('div', { class: 'da-choices', role: 'radiogroup', 'aria-label': t('relevant') });
    const status = h('div', { class: 'da-dialog-status', 'aria-live': 'polite' });
    let dialog;
    function updateSelection() {
      choices.querySelectorAll('.da-choice').forEach((row) => row.classList.toggle('is-selected', row.querySelector('input').value === selected));
      const primary = dialog?.dialog.el.querySelector('[data-result="primary"]');
      if (primary) primary.disabled = !selected || selected === detail.default?.id;
    }
    for (const app of detail.apps || []) {
      const input = h('input', { type: 'radio', name: 'da-default-app', value: app.id, checked: app.id === selected,
        onChange: () => { selected = app.id; updateSelection(); } });
      const row = h('label', { class: 'da-choice' }, input, AppIcon({ name: app.icon || app.id, size: 32 }),
        h('span', { class: 'da-choice-text' }, h('span', { class: 'da-choice-name' }, app.name),
          app.isDefault ? h('span', { class: 'da-choice-current' }, t('current')) : null));
      choices.append(row);
    }
    const hasApps = (detail.apps || []).length > 0;
    const content = h('div', { class: 'da-dialog-content' }, h('p', { class: 'da-dialog-type' }, detail.name, h('span', { class: 'da-mime' }, detail.mime)),
      hasApps ? h('p', { class: 'da-dialog-intro' }, t('relevant')) : null,
      hasApps ? choices : EmptyState({ icon: 'apps', title: t('noApps'), message: t('noAppsBody') }), status);
    dialog = Dialog.show({ title: t('chooseFor', { type: (detail.extensions || [])[0] || detail.mime }), content,
      primary: hasApps ? t('use') : undefined, close: t('cancel'), width: 520,
      async onPrimary() {
        if (!selected || selected === detail.default?.id) return false;
        clear(status);
        try {
          const result = await bridge.call('apps.setDefaultForType', { mime: detail.mime, id: selected });
          const app = detail.apps.find((a) => a.id === selected);
          if (result.effective === false) Toast.show({ severity: 'warning', title: t('overridden'), message: result.overriddenBy?.length ? t('overrides', { files: result.overriddenBy.join(', ') }) : '', timeout: 10000 });
          else Toast.show({ severity: 'success', title: t('saved', { app: app?.name || selected, type: (detail.extensions || [])[0] || detail.mime }), timeout: 3500 });
          load(); return true;
        } catch (e) { status.append(InfoBar({ severity: 'error', title: t('saveFailed'), message: e.message })); return false; }
      } });
    activeDialog = dialog; updateSelection();
    await dialog;
    if (activeDialog === dialog) activeDialog = null;
  }
  frame.onAppNavigate((p) => {
    if (p.query != null || p.extension != null || p.mime != null) { query = String(p.query || p.extension || p.mime || ''); search.setValue(query); offset = 0; load(); }
  });
  bridge.on('apps.changed', () => { if (!activeDialog) load(); });
  addEventListener('focus', () => { if (!activeDialog) load(); });
  onLangChange(() => { activeDialog?.dialog.close('close'); labels(); load(); });
  labels(); render(); await load();
}
