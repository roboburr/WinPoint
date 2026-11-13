// Taakbeheer — Services: the user's own systemd services (systemctl --user) with a status
// pill and start / stop / restart (always confirmed, with a plain explanation).

import { h, clear } from '../../../core/dom.js';
import { Button, IconButton, Dialog, Toast } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { makeT } from '../../../core/i18n.js';
import { DataTable } from '../table.js';
import { stopIcon } from '../util.js';

const t = makeT({
  nl: {
    title: 'Services', start: 'Starten', stop: 'Stoppen', restart: 'Opnieuw starten', refresh: 'Vernieuwen',
    name: 'Naam', description: 'Beschrijving', status: 'Status', actions: 'Acties',
    active: 'Actief', inactive: 'Gestopt', failed: 'Mislukt', activating: 'Bezig met starten', deactivating: 'Bezig met stoppen', unknown: 'Onbekend',
    intro: 'Achtergrondservices van je eigen account',
    introText: 'Deze kleine programma’s werken op de achtergrond voor je apps. Systeemservices staan hier niet, zodat je niets belangrijks per ongeluk stopt.',
    confirmStop: '‘{name}’ stoppen?', confirmRestart: '‘{name}’ opnieuw starten?', confirmStart: '‘{name}’ starten?',
    stopBody: 'Apps die deze service gebruiken, werken misschien niet goed tot je de service weer start.',
    protectedTip: 'Nodig voor je bureaublad – kan niet worden gestopt',
    restartBody: 'De service stopt heel even en start dan opnieuw. Dat helpt vaak als iets is blijven hangen.',
    startBody: 'De service wordt gestart en blijft actief tot je je afmeldt of hem stopt.',
    done_start: '{name} is gestart', done_stop: '{name} is gestopt', done_restart: '{name} is opnieuw gestart',
    failedAction: 'Dat is niet gelukt', loading: 'Services laden…', none: 'Geen services gevonden.', selectFirst: 'Selecteer eerst een service',
    counts: '{a} actief · {i} gestopt', countsFailed: '{a} actief · {i} gestopt · {f} mislukt',
  },
  en: {
    title: 'Services', start: 'Start', stop: 'Stop', restart: 'Restart', refresh: 'Refresh',
    name: 'Name', description: 'Description', status: 'Status', actions: 'Actions',
    active: 'Running', inactive: 'Stopped', failed: 'Failed', activating: 'Starting', deactivating: 'Stopping', unknown: 'Unknown',
    intro: 'Background services of your own account',
    introText: 'These small programs work in the background for your apps. System services aren’t listed, so you can’t stop anything important by accident.',
    confirmStop: 'Stop ‘{name}’?', confirmRestart: 'Restart ‘{name}’?', confirmStart: 'Start ‘{name}’?',
    stopBody: 'Apps that use this service may not work properly until you start it again.',
    protectedTip: 'Needed by your desktop – can’t be stopped',
    restartBody: 'The service stops briefly and starts again. This often helps when something is stuck.',
    startBody: 'The service starts and keeps running until you sign out or stop it.',
    done_start: '{name} has started', done_stop: '{name} has stopped', done_restart: '{name} has restarted',
    failedAction: 'That didn’t work', loading: 'Loading services…', none: 'No services found.', selectFirst: 'Select a service first',
    counts: '{a} running · {i} stopped', countsFailed: '{a} running · {i} stopped · {f} failed',
  },
});

const short = (name) => String(name || '').replace(/\.service$/, '');
const shown = (s) => s.displayName || short(s.name);

function stateOf(s) {
  const st = String(s.state || '').toLowerCase();
  if (st === 'active') return 'active';
  if (st === 'failed') return 'failed';
  if (st === 'activating' || st === 'reloading') return 'activating';
  if (st === 'deactivating') return 'deactivating';
  if (st === 'inactive') return 'inactive';
  return 'unknown';
}

export function createPage(ctx) {
  const { store, bridge } = ctx;
  let items = [];
  let query = '';
  const busy = new Set();
  const offs = [];

  async function act(s, action) {
    const name = shown(s);
    const title = t(action === 'stop' ? 'confirmStop' : action === 'restart' ? 'confirmRestart' : 'confirmStart', { name });
    const body = h('div', { class: 'tm-dlg' },
      s.description ? h('p', { class: 'tm-dlg__hint' }, s.description) : null,
      h('p', null, t(action === 'stop' ? 'stopBody' : action === 'restart' ? 'restartBody' : 'startBody')));
    const ok = await Dialog.confirm({ title, content: body, confirmLabel: t(action), danger: action === 'stop' });
    if (!ok) return;
    busy.add(s.name);
    render();
    try {
      await bridge.call('sysmon.serviceAction', { name: s.name, action });
      Toast.show({ title: t(`done_${action}`, { name }), severity: 'success', timeout: 3000 });
    } catch (e) {
      Toast.show({ title: t('failedAction'), message: e.message || '', severity: 'error' });
    } finally {
      busy.delete(s.name);
      await store.refresh('services');
    }
  }

  const columns = [
    {
      id: 'name', label: t('name'), width: 260, minWidth: 140, required: true,
      value: (r) => shown(r).toLowerCase(), searchText: (r) => shown(r),
      render(c, r, { isNew }) {
        if (isNew) {
          c.classList.add('tm-name');
          c._t = h('span', { class: 'tm-name__text' });
          c.append(h('span', { class: 'tm-name__icon tm-glyph' }, icon('services', { size: 16 })), c._t);
        }
        const n = shown(r);
        if (c._t.textContent !== n) c._t.textContent = n;
      },
      tip: (r) => (r.critical ? `${r.name} · ${t('protectedTip')}` : r.name),
    },
    { id: 'description', label: t('description'), width: 300, flex: true, value: (r) => r.description, text: (r) => r.description || '' },
    {
      id: 'status', label: t('status'), width: 150, value: (r) => ['failed', 'activating', 'active', 'deactivating', 'inactive', 'unknown'].indexOf(stateOf(r)),
      render(c, r) {
        const st = busy.has(r.name) ? 'activating' : stateOf(r);
        if (c._st === st) return;
        c._st = st;
        clear(c).append(h('span', { class: ['tm-pill', 'tm-state', `is-${st}`] }, h('span', { class: 'tm-state__dot' }), t(st)));
      },
    },
    {
      id: 'actions', label: t('actions'), width: 132, sortable: false,
      render(c, r, { isNew }) {
        if (isNew) {
          c._start = IconButton({ icon: 'play', label: t('start'), size: 28, iconSize: 14, onClick: (e) => { e.stopPropagation(); act(c._row, 'start'); } });
          c._stop = IconButton({ icon: stopIcon(14), label: t('stop'), size: 28, iconSize: 14, onClick: (e) => { e.stopPropagation(); act(c._row, 'stop'); } });
          c._restart = IconButton({ icon: 'restart', label: t('restart'), size: 28, iconSize: 14, onClick: (e) => { e.stopPropagation(); act(c._row, 'restart'); } });
          c.classList.add('tm-actions');
          c.append(c._start, c._stop, c._restart);
        }
        c._row = r;
        const st = stateOf(r), b = busy.has(r.name);
        c._start.setDisabled(b || st === 'active' || st === 'activating');
        c._stop.setDisabled(b || st === 'inactive' || st === 'failed' || !!r.critical);
        c._restart.setDisabled(b || st !== 'active' || !!r.critical);
        c._stop.setLabel(r.critical ? t('protectedTip') : t('stop'));
      },
    },
  ];

  const table = DataTable({
    columns, storageKey: 'services', ariaLabel: t('title'), rowHeight: 40,
    rowKey: (r) => r.name, defaultSort: { id: 'name', desc: false },
    onSelectionChange: () => updateCommands(),
    contextMenu: (rows) => {
      const r = rows[0];
      const st = stateOf(r);
      return { items: [
        { label: t('start'), icon: 'play', disabled: st === 'active', action: () => act(r, 'start') },
        { label: t('stop'), icon: stopIcon(16), disabled: st === 'inactive' || st === 'failed' || !!r.critical, description: r.critical ? t('protectedTip') : null, action: () => act(r, 'stop') },
        { label: t('restart'), icon: 'restart', disabled: st !== 'active' || !!r.critical, action: () => act(r, 'restart') },
      ] };
    },
    emptyText: t('loading'),
  });

  const counts = h('div', { class: 'tm-intro__meta' });
  const intro = h('div', { class: 'tm-intro' }, h('span', { class: 'tm-intro__icon' }, icon('services', { size: 20 })),
    h('div', { class: 'grow' }, h('div', { class: 'tm-intro__title' }, t('intro')), h('div', { class: 'tm-intro__text' }, t('introText'))), counts);

  const startBtn = Button({ label: t('start'), icon: 'play', variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) act(r, 'start'); } });
  const stopBtn = Button({ label: t('stop'), icon: stopIcon(16), variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) act(r, 'stop'); } });
  const restartBtn = Button({ label: t('restart'), icon: 'restart', variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) act(r, 'restart'); } });
  const refreshBtn = Button({ label: t('refresh'), icon: 'refresh', variant: 'subtle', onClick: () => store.refresh('services') });

  function updateCommands() {
    const r = table.getSelection()[0];
    const st = r ? stateOf(r) : null;
    startBtn.setDisabled(!r || st === 'active' || st === 'activating');
    stopBtn.setDisabled(!r || st === 'inactive' || st === 'failed' || !!r.critical);
    restartBtn.setDisabled(!r || st !== 'active' || !!r.critical);
  }

  function render() {
    const q = query.trim().toLowerCase();
    table.setFilter(q ? (r) => r.name.toLowerCase().includes(q) || shown(r).toLowerCase().includes(q) || String(r.description || '').toLowerCase().includes(q) : null);
    table.setRows(items);
    const a = items.filter((s) => stateOf(s) === 'active').length;
    const f = items.filter((s) => stateOf(s) === 'failed').length;
    counts.textContent = f ? t('countsFailed', { a, i: items.length - a - f, f }) : t('counts', { a, i: items.length - a });
    table.setEmptyText(items.length ? '' : t('none'));
    updateCommands();
  }

  const el = h('div', { class: 'tm-page__fill tm-services' }, h('div', { class: 'tm-page__pad' }, intro), table.el);

  return {
    id: 'services', title: t('title'), el,
    commands: [startBtn, stopBtn, restartBtn, h('span', { class: 'tm-cmd-sep' }), refreshBtn],
    mount() { offs.push(store.subscribe('services', (d) => { items = Array.isArray(d) ? d : []; render(); })); },
    unmount() { while (offs.length) offs.pop()(); },
    onSearch(q) { query = q || ''; render(); },
    refresh() { return store.refresh('services'); },
  };
}
