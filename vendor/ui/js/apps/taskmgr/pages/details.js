// Taakbeheer — Details: every process in one flat, sortable table (multi-select).

import { h, clear } from '../../../core/dom.js';
import { Button, AppIcon } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { makeT } from '../../../core/i18n.js';
import { number } from '../../../core/format.js';
import { DataTable } from '../table.js';
import { health } from '../hostcall.js';
import { cpuPct, memMB, endTaskIcon, leafOutline, tipWrap } from '../util.js';
import { endTask, setEfficiency, showProperties, processMenu, normStatus, targetOf, restartTask, restartTargetOf } from '../actions.js';

const t = makeT({
  nl: {
    title: 'Details', newTask: 'Nieuwe taak', endTask: 'Taak beëindigen', efficiency: 'Efficiëntiemodus', properties: 'Eigenschappen',
    name: 'Naam', pid: 'PID', status: 'Status', user: 'Gebruiker', cpu: 'CPU', memory: 'Geheugen', threads: 'Threads', cmd: 'Opdrachtregel',
    desc: 'Omschrijving', started: 'Gestart',
    running: 'Actief', waiting: 'Wacht', suspended: 'Onderbroken', zombie: 'Afgesloten', hung: 'Reageert niet', idle: 'Actief',
    many: { one: '{n} proces', other: '{n} processen' }, loading: 'Processen laden…', waiting: 'Wachten op Win11 Shell…', noMatch: 'Geen processen gevonden voor “{q}”.',
    systemProc: 'Systeemproces – kan niet worden beëindigd', selectFirst: 'Selecteer eerst een proces',
    restart: 'Opnieuw starten', restartShell: 'Win11 Shell opnieuw starten',
  },
  en: {
    title: 'Details', newTask: 'Run new task', endTask: 'End task', efficiency: 'Efficiency mode', properties: 'Properties',
    name: 'Name', pid: 'PID', status: 'Status', user: 'User name', cpu: 'CPU', memory: 'Memory', threads: 'Threads', cmd: 'Command line',
    desc: 'Description', started: 'Started',
    running: 'Running', waiting: 'Waiting', suspended: 'Suspended', zombie: 'Exited', hung: 'Not responding', idle: 'Running',
    many: { one: '{n} process', other: '{n} processes' }, loading: 'Loading processes…', waiting: 'Waiting for Win11 Shell…', noMatch: 'No processes found for “{q}”.',
    systemProc: 'System process – can’t be ended', selectFirst: 'Select a process first',
    restart: 'Restart', restartShell: 'Restart Win11 Shell',
  },
});

function stateLabel(p, hung) {
  if (hung) return t('hung');
  const word = { running: 'R', sleeping: 'S', idle: 'I', waiting: 'D', stopped: 'T', zombie: 'Z' }[p.state];
  switch (word || String(p.state || 'S').charAt(0)) {
    case 'R': case 'S': case 'I': return t('running');
    case 'D': return t('waiting');
    case 'T': case 't': return t('suspended');
    case 'Z': case 'X': return t('zombie');
    default: return t('running');
  }
}

/** Combine several selected rows into one action target. */
function combine(rows) {
  if (rows.length === 1) return rows[0];
  const procs = rows.flatMap((r) => r.procs);
  return { name: t('many', { n: rows.length }), icon: null, pids: procs.map((p) => p.pid), procs, cmd: '', multi: true };
}

export function createPage(ctx) {
  const { store } = ctx;
  let data = null;
  let query = '';
  const offs = [];

  const columns = [
    {
      id: 'name', label: t('name'), width: 200, minWidth: 120, required: true,
      value: (r) => r.name.toLowerCase(), searchText: (r) => r.name,
      render(c, r, { isNew }) {
        if (isNew || c._ik !== r.icon) {
          clear(c);
          c.classList.add('tm-name');
          c._t = h('span', { class: 'tm-name__text' });
          c.append(h('span', { class: 'tm-name__icon' }, r.icon ? AppIcon({ name: r.icon, size: 16, fallback: 'app' }) : h('span', { class: 'tm-glyph' }, icon(r.group === 'system' ? 'settings' : 'window', { size: 16 }))), c._t);
          c._ik = r.icon;
        }
        if (c._t.textContent !== r.name) c._t.textContent = r.name;
      },
      tip: (r) => (r.displayName && r.displayName !== r.name ? r.displayName : ''),
    },
    { id: 'pid', label: t('pid'), width: 80, align: 'right', value: (r) => r.pid, text: (r) => String(r.pid) },
    { id: 'status', label: t('status'), width: 120, value: (r) => r.stateText, text: (r) => r.stateText },
    { id: 'user', label: t('user'), width: 120, value: (r) => r.user, text: (r) => r.user || '' },
    { id: 'cpu', label: t('cpu'), width: 76, align: 'right', defaultDesc: true, value: (r) => r.cpu, text: (r) => cpuPct(r.cpu) },
    { id: 'memory', label: t('memory'), width: 110, align: 'right', defaultDesc: true, value: (r) => r.memory, text: (r) => memMB(r.memory) },
    { id: 'threads', label: t('threads'), width: 80, align: 'right', defaultDesc: true, value: (r) => r.threads, text: (r) => number(r.threads || 0) },
    { id: 'desc', label: t('desc'), width: 200, hidden: true, value: (r) => r.displayName, text: (r) => r.displayName || '' },
    { id: 'cmd', label: t('cmd'), width: 360, flex: true, value: (r) => r.cmd, text: (r) => r.cmd || '', cellClass: 'tm-cell--mono', tip: (r) => r.cmd || '' },
  ];

  const table = DataTable({
    columns, storageKey: 'details', ariaLabel: t('title'), multiSelect: true,
    rowKey: (r) => r.key, defaultSort: { id: 'name', desc: false },
    rowClass: (r) => [r.hung && 'is-hung', !r.canKill && 'is-system'],
    onSelectionChange: () => updateCommands(),
    onActivate: (r) => showProperties(r),
    onDelete: (rows) => { const tg = combine(rows); if (targetOf(tg).canKill) endTask(ctx, tg); },
    contextMenu: (rows) => processMenu(ctx, [combine(rows)]),
    emptyText: t('loading'),
  });

  const newBtn = Button({ label: t('newTask'), icon: 'new', variant: 'subtle', onClick: () => ctx.newTask() });
  const endBtn = Button({ label: t('endTask'), icon: endTaskIcon(16), variant: 'subtle', disabled: true, onClick: () => { const rows = table.getSelection(); if (rows.length) endTask(ctx, combine(rows)); } });
  const endWrap = tipWrap(endBtn, t('selectFirst'));
  const restartBtn = Button({ label: t('restart'), icon: 'restart', variant: 'subtle', disabled: true, onClick: () => { const rows = table.getSelection(); if (rows.length) restartTask(ctx, combine(rows)); } });
  const restartWrap = tipWrap(restartBtn, t('selectFirst'));
  const effBtn = Button({ label: t('efficiency'), icon: leafOutline(16), variant: 'subtle', disabled: true, onClick: () => { const rows = table.getSelection(); if (rows.length) { const c = combine(rows); setEfficiency(ctx, c, !targetOf(c).efficiency); } } });
  const effWrap = tipWrap(effBtn, t('selectFirst'));
  const propBtn = Button({ label: t('properties'), icon: 'properties', variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) showProperties(r); } });

  function updateCommands() {
    const rows = table.getSelection();
    const rt = rows.length ? restartTargetOf(combine(rows)) : { ok: false, reason: t('selectFirst') };
    const restartLabel = rt.kind === 'shell' ? t('restartShell') : t('restart');
    if (restartBtn.textContent !== restartLabel) restartBtn.setLabel(restartLabel);
    restartBtn.setDisabled(!rt.ok);
    restartWrap.setTip(rt.ok ? '' : rt.reason);
    if (!rows.length) {
      endBtn.setDisabled(true); effBtn.setDisabled(true); propBtn.setDisabled(true);
      endWrap.setTip(t('selectFirst')); effWrap.setTip(t('selectFirst'));
      effBtn.classList.remove('is-pressed');
      return;
    }
    const tg = targetOf(combine(rows));
    endBtn.setDisabled(!tg.canKill); endWrap.setTip(tg.canKill ? '' : t('systemProc'));
    effBtn.setDisabled(!tg.canKill); effWrap.setTip(tg.canKill ? '' : t('systemProc'));
    effBtn.classList.toggle('is-pressed', tg.efficiency);
    propBtn.setDisabled(rows.length !== 1);
  }

  function rows() {
    if (!data) return [];
    const iconOf = new Map();
    const hungPids = new Set();
    const appOf = new Map();
    const byPid = new Map((data.processes || []).map((p) => [p.pid, p]));
    for (const a of data.apps || []) {
      // "Opnieuw starten" on one process of an app restarts the app
      const appRow = { kind: 'app', name: a.name, icon: a.icon, pids: a.pids || [], procs: (a.pids || []).map((pid) => byPid.get(pid)).filter(Boolean), canKill: a.canKill };
      for (const pid of a.pids || []) { iconOf.set(pid, a.icon); appOf.set(pid, appRow); }
      if (normStatus(a.status) === 'notResponding' && a.pids?.length) hungPids.add(a.pids[0]);
    }
    for (const b of data.background || []) for (const pid of b.pids || []) if (b.icon) iconOf.set(pid, b.icon);
    for (const b of data.system || []) for (const pid of b.pids || []) if (b.icon) iconOf.set(pid, b.icon);
    return (data.processes || []).map((p) => ({
      key: `d:${p.pid}`, kind: 'proc', name: p.name, displayName: p.displayName, icon: iconOf.get(p.pid) || null, group: p.group,
      pid: p.pid, pids: [p.pid], procs: [p], cpu: p.cpu || 0, memory: p.memory || 0, threads: p.threads || 0,
      user: p.user, cmd: p.cmd, exe: p.exe, canKill: p.canKill !== false, efficiency: !!p.efficiency, hung: hungPids.has(p.pid), appRow: appOf.get(p.pid) || null,
      stateText: stateLabel(p, hungPids.has(p.pid)), status: hungPids.has(p.pid) ? 'notResponding' : (p.efficiency ? 'efficiency' : 'running'),
    }));
  }

  function render() {
    table.setRows(rows());
    updateCommands();
  }

  function applyFilter() {
    const q = query.trim().toLowerCase();
    table.setEmptyText(data ? t('noMatch', { q: query.trim() }) : t(health.state === 'ok' ? 'loading' : 'waiting'));
    if (!q) { table.setFilter(null); return; }
    const isNum = /^\d+$/.test(q);
    table.setFilter((r) => (isNum && String(r.pid).startsWith(q)) || r.name.toLowerCase().includes(q)
      || String(r.displayName || '').toLowerCase().includes(q) || String(r.cmd || '').toLowerCase().includes(q));
  }

  const el = h('div', { class: 'tm-page__fill tm-details' }, table.el);
  el.addEventListener('keydown', (e) => {
    if (e.altKey && e.key === 'Enter') { const r = table.getSelection()[0]; if (r) { e.preventDefault(); showProperties(r); } }
  });

  let pendingPid = null;
  return {
    id: 'details', title: t('title'), el,
    commands: [newBtn, h('span', { class: 'tm-cmd-sep' }), endWrap, restartWrap, effWrap, propBtn],
    mount(params = {}) {
      pendingPid = params.pid ? Number(params.pid) : null;
      offs.push(store.subscribe('processes', (d) => {
        data = d;
        render();
        applyFilter();
        if (pendingPid && table.select(`d:${pendingPid}`, { focus: true })) pendingPid = null;
      }));
      offs.push(health.onChange(() => { if (!data) applyFilter(); }));
      if (!data) applyFilter();
    },
    navigate(params = {}) { if (params.pid) table.select(`d:${Number(params.pid)}`, { focus: true }); },
    unmount() { while (offs.length) offs.pop()(); },
    onSearch(q) { query = q || ''; applyFilter(); },
    refresh() { return store.refresh('processes'); },
  };
}
