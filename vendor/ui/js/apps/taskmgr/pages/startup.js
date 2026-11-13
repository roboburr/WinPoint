// Taakbeheer — Opstart-apps: which apps start when you sign in, with a switch per row.

import { h } from '../../../core/dom.js';
import { Button, ToggleSwitch, AppIcon, Toast } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { makeT } from '../../../core/i18n.js';
import { DataTable } from '../table.js';

const t = makeT({
  nl: {
    title: 'Opstart-apps', newTask: 'Nieuwe taak', enable: 'Inschakelen', disable: 'Uitschakelen', refresh: 'Vernieuwen',
    name: 'Naam', source: 'Bron', status: 'Status', impact: 'Invloed bij opstarten', command: 'Opdracht',
    user: 'Persoonlijk', system: 'Systeem', on: 'Ingeschakeld', off: 'Uitgeschakeld', notMeasured: 'Niet gemeten',
    impact_none: 'Geen', impact_low: 'Laag', impact_medium: 'Gemiddeld', impact_high: 'Hoog',
    summary: { one: '1 van {m} apps start automatisch als je je aanmeldt.', other: '{n} van {m} apps starten automatisch als je je aanmeldt.' },
    tip: 'Minder opstart-apps betekent dat je pc sneller klaar is na het aanmelden.',
    failed: 'Kon dit niet wijzigen', loading: 'Opstart-apps laden…', none: 'Er zijn geen opstart-apps.',
    enabledToast: '{name} start voortaan automatisch', disabledToast: '{name} start niet meer automatisch',
    sourceTipUser: 'Door jou of een app ingesteld (~/.config/autostart)', sourceTipSystem: 'Ingesteld door Ubuntu; uitzetten geldt alleen voor jou',
    selectFirst: 'Selecteer eerst een app',
  },
  en: {
    title: 'Startup apps', newTask: 'Run new task', enable: 'Enable', disable: 'Disable', refresh: 'Refresh',
    name: 'Name', source: 'Source', status: 'Status', impact: 'Startup impact', command: 'Command',
    user: 'Personal', system: 'System', on: 'Enabled', off: 'Disabled', notMeasured: 'Not measured',
    impact_none: 'None', impact_low: 'Low', impact_medium: 'Medium', impact_high: 'High',
    summary: { one: '1 of {m} apps starts automatically when you sign in.', other: '{n} of {m} apps start automatically when you sign in.' },
    tip: 'Fewer startup apps means your PC is ready sooner after signing in.',
    failed: 'Couldn’t change this', loading: 'Loading startup apps…', none: 'There are no startup apps.',
    enabledToast: '{name} will start automatically', disabledToast: '{name} will no longer start automatically',
    sourceTipUser: 'Set by you or an app (~/.config/autostart)', sourceTipSystem: 'Set by Ubuntu; turning it off only affects you',
    selectFirst: 'Select an app first',
  },
});

const IMPACT_ORDER = { none: 0, low: 1, medium: 2, high: 3 };

export function createPage(ctx) {
  const { store, bridge } = ctx;
  let items = [];
  let query = '';
  const offs = [];
  const pending = new Set();

  async function setEnabled(it, on) {
    if (pending.has(it.id) || it.enabled === on) return;
    pending.add(it.id);
    const prev = it.enabled;
    it.enabled = on;
    render();
    try {
      await bridge.call('sysmon.setStartup', { id: it.id, enabled: on });
      Toast.show({ title: t(on ? 'enabledToast' : 'disabledToast', { name: it.name }), severity: 'success', timeout: 2500 });
    } catch (e) {
      it.enabled = prev;
      Toast.show({ title: t('failed'), message: e.message || '', severity: 'error' });
    } finally {
      pending.delete(it.id);
      render();
    }
  }

  const columns = [
    {
      id: 'name', label: t('name'), width: 260, minWidth: 160, flex: true, required: true,
      value: (r) => r.name.toLowerCase(), searchText: (r) => r.name,
      render(c, r, { isNew }) {
        if (isNew) {
          c.classList.add('tm-name');
          c._t = h('span', { class: 'tm-name__text' });
          c.append(h('span', { class: 'tm-name__icon tm-name__icon--24' }, AppIcon({ name: r.icon, size: 24, fallback: 'startup' })), c._t);
        }
        if (c._t.textContent !== r.name) c._t.textContent = r.name;
      },
      tip: (r) => r.description || '',
    },
    {
      id: 'source', label: t('source'), width: 130,
      value: (r) => r.source, text: (r) => (r.source === 'system' ? t('system') : t('user')),
      tip: (r) => (r.source === 'system' ? t('sourceTipSystem') : t('sourceTipUser')),
    },
    {
      id: 'status', label: t('status'), width: 190, value: (r) => (r.enabled ? 0 : 1),
      render(c, r, { isNew }) {
        if (isNew) {
          c._sw = ToggleSwitch({ checked: r.enabled, onLabel: t('on'), offLabel: t('off'), ariaLabel: r.name, onChange: (on) => setEnabled(c._row, on) });
          c.append(c._sw);
        }
        c._row = r;
        if (c._sw.checked !== r.enabled) c._sw.setChecked(r.enabled);
        c._sw.setDisabled(pending.has(r.id));
      },
    },
    // Ubuntu does not measure how much an app slows down signing in, so this column has no data: hidden by default
    // (header menu › Kolommen), an em dash with a tooltip instead of "Niet gemeten" in every row
    { id: 'impact', label: t('impact'), width: 170, hidden: true, value: (r) => IMPACT_ORDER[r.impact] ?? -1,
      text: (r) => (IMPACT_ORDER[r.impact] != null ? t(`impact_${r.impact}`) : '—'), tip: (r) => (IMPACT_ORDER[r.impact] != null ? null : t('notMeasured')), cellClass: 'tm-cell--muted' },
    { id: 'command', label: t('command'), width: 360, hidden: true, value: (r) => r.command, text: (r) => r.command, cellClass: 'tm-cell--mono', tip: (r) => r.command },
  ];

  const table = DataTable({
    columns, storageKey: 'startup', ariaLabel: t('title'), rowHeight: 44,
    rowKey: (r) => r.id, defaultSort: { id: 'name', desc: false },
    onSelectionChange: () => updateCommands(),
    onActivate: (r) => setEnabled(r, !r.enabled),
    contextMenu: (rows) => {
      const r = rows[0];
      return { items: [
        { label: r.enabled ? t('disable') : t('enable'), icon: r.enabled ? 'close' : 'check', action: () => setEnabled(r, !r.enabled) },
      ] };
    },
    emptyText: t('loading'),
  });

  const summary = h('div', { class: 'tm-intro' }, h('span', { class: 'tm-intro__icon' }, icon('startup', { size: 20 })),
    h('div', null, h('div', { class: 'tm-intro__title' }), h('div', { class: 'tm-intro__text' }, t('tip'))));

  const enableBtn = Button({ label: t('enable'), icon: 'check', variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) setEnabled(r, true); } });
  const disableBtn = Button({ label: t('disable'), icon: 'close', variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) setEnabled(r, false); } });
  const newBtn = Button({ label: t('newTask'), icon: 'new', variant: 'subtle', onClick: () => ctx.newTask() });

  function updateCommands() {
    const r = table.getSelection()[0];
    enableBtn.setDisabled(!r || r.enabled);
    disableBtn.setDisabled(!r || !r.enabled);
  }

  function render() {
    const q = query.trim().toLowerCase();
    table.setFilter(q ? (r) => r.name.toLowerCase().includes(q) || String(r.command || '').toLowerCase().includes(q) : null);
    table.setRows(items);
    const n = items.filter((i) => i.enabled).length;
    summary.querySelector('.tm-intro__title').textContent = t('summary', { n, m: items.length });
    table.setEmptyText(items.length ? '' : t('none'));
    updateCommands();
  }

  const el = h('div', { class: 'tm-page__fill tm-startup' }, h('div', { class: 'tm-page__pad' }, summary), table.el);

  return {
    id: 'startup', title: t('title'), el,
    commands: [newBtn, h('span', { class: 'tm-cmd-sep' }), enableBtn, disableBtn],
    mount() {
      offs.push(store.subscribe('startup', (d) => { items = Array.isArray(d) ? d.map((x) => ({ ...x })) : []; render(); }));
    },
    unmount() { while (offs.length) offs.pop()(); },
    onSearch(q) { query = q || ''; render(); },
    refresh() { return store.refresh('startup'); },
  };
}
