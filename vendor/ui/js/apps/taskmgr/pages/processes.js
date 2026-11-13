// Taakbeheer — Processen: Win11 grouping (Apps / Achtergrondprocessen / Systeemprocessen),
// expandable app rows, heat-map cells, header totals, live updates, context menu, restart (and "Win11 Shell
// opnieuw starten" when our own shell is selected, like "Windows Verkenner opnieuw starten").

import { h, clear } from '../../../core/dom.js';
import { Button, AppIcon } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { showContextMenu } from '../../../core/contextmenu.js';
import { makeT, lang } from '../../../core/i18n.js';
import { DataTable } from '../table.js';
import { health } from '../hostcall.js';
import { cpuPct, memMB, diskMBs, leafIcon, leafOutline, endTaskIcon, pauseIcon, tipWrap, load, save } from '../util.js';
import { endTask, setEfficiency, showProperties, processMenu, normStatus, targetOf, restartTask, restartTargetOf } from '../actions.js';

const t = makeT({
  nl: {
    title: 'Processen', newTask: 'Nieuwe taak', endTask: 'Taak beëindigen', efficiency: 'Efficiëntiemodus', view: 'Weergave',
    groupByType: 'Groeperen op type', expandAll: 'Alles uitvouwen', collapseAll: 'Alles samenvouwen', columns: 'Kolommen',
    apps: 'Apps', background: 'Achtergrondprocessen', system: 'Systeemprocessen', all: 'Processen',
    name: 'Naam', status: 'Status', cpu: 'CPU', memory: 'Geheugen', disk: 'Schijf', pid: 'PID', procName: 'Procesnaam', cmd: 'Opdrachtregel', user: 'Gebruiker',
    notResponding: 'Reageert niet', suspended: 'Onderbroken', effTip: 'Efficiëntiemodus staat aan',
    noMatch: 'Geen processen gevonden voor “{q}”.', loading: 'Processen laden…', waiting: 'Wachten op Win11 Shell…',
    systemProc: 'Systeemproces – kan niet worden beëindigd', selectFirst: 'Selecteer eerst een app of proces',
    cpuTip: 'Hoeveel rekenkracht van de processor dit gebruikt', memTip: 'Hoeveel werkgeheugen dit gebruikt', diskTip: 'Hoeveel dit van en naar de schijf leest en schrijft',
    newTaskTip: 'Een app zoeken en starten',
    restart: 'Opnieuw starten', restartShell: 'Win11 Shell opnieuw starten',
    restartTip: 'Stoppen en meteen weer starten – handig als iets is blijven hangen',
  },
  en: {
    title: 'Processes', newTask: 'Run new task', endTask: 'End task', efficiency: 'Efficiency mode', view: 'View',
    groupByType: 'Group by type', expandAll: 'Expand all', collapseAll: 'Collapse all', columns: 'Columns',
    apps: 'Apps', background: 'Background processes', system: 'System processes', all: 'Processes',
    name: 'Name', status: 'Status', cpu: 'CPU', memory: 'Memory', disk: 'Disk', pid: 'PID', procName: 'Process name', cmd: 'Command line', user: 'User',
    notResponding: 'Not responding', suspended: 'Suspended', effTip: 'Efficiency mode is on',
    noMatch: 'No processes found for “{q}”.', loading: 'Loading processes…', waiting: 'Waiting for Win11 Shell…',
    systemProc: 'System process – can’t be ended', selectFirst: 'Select an app or process first',
    cpuTip: 'How much processor power this uses', memTip: 'How much memory this uses', diskTip: 'How much this reads from and writes to disk',
    newTaskTip: 'Find and start an app',
    restart: 'Restart', restartShell: 'Restart Win11 Shell',
    restartTip: 'Stop and start again right away – handy when something got stuck',
  },
});

/* ------------------------------------------------------------- row model */

const TYPE_LABELS = {
  nl: { renderer: 'weergave', 'gpu-process': 'GPU-proces', utility: 'hulpproces', zygote: 'starter', 'crashpad-handler': 'foutrapportage', broker: 'broker', tab: 'tabblad', rdd: 'media', socket: 'netwerk', gpu: 'GPU-proces', content: 'inhoud' },
  en: { renderer: 'renderer', 'gpu-process': 'GPU process', utility: 'utility', zygote: 'zygote', 'crashpad-handler': 'crash reporter', broker: 'broker', tab: 'tab', rdd: 'media', socket: 'network', gpu: 'GPU process', content: 'content' },
};
const isStopped = (state) => state === 'T' || state === 't' || state === 'stopped';

/** A helpful name for a child process of an app ("Firefox (tabblad)", "Web Content", "tsserver"). */
function childName(p, parentName) {
  const labels = TYPE_LABELS[lang() === 'en' ? 'en' : 'nl'];
  if (p.displayName && p.displayName !== parentName) return p.displayName;
  const cmd = String(p.cmd || '');
  const m = /--type=([\w-]+)/.exec(cmd);
  if (m) return `${parentName} (${labels[m[1]] || m[1]})`;
  if (/-contentproc/.test(cmd)) {
    const k = /\b(tab|rdd|socket|utility|gpu)\s*$/.exec(cmd);
    return `${parentName} (${labels[k ? k[1] : 'content'] || 'content'})`;
  }
  if (p.name && p.name.toLowerCase() !== String(parentName).toLowerCase()) return p.name;
  return parentName;
}

function procRow(p, parent) {
  return {
    kind: 'proc', key: `p:${p.pid}`, name: parent ? childName(p, parent.name) : (p.displayName || p.name), icon: parent?.icon || null, iconKind: parent?.iconKind || 'glyph',
    pids: [p.pid], procs: [p], cpu: p.cpu || 0, memory: p.memory || 0, disk: (p.diskRead || 0) + (p.diskWrite || 0),
    status: isStopped(p.state) ? 'suspended' : (p.efficiency ? 'efficiency' : (parent?.status === 'notResponding' && p.pid === parent.pids[0] ? 'notResponding' : 'running')),
    canKill: p.canKill !== false, efficiency: !!p.efficiency, cmd: p.cmd, exe: p.exe, user: p.user, procName: p.name, pid: p.pid, children: null,
    parentRow: parent || null,
  };
}

function groupRow(kind, g, byPid) {
  const procs = (g.pids || []).map((pid) => byPid.get(pid)).filter(Boolean);
  const row = {
    kind, key: `${kind[0]}:${g.key}`, name: g.name, icon: g.icon || null, iconKind: g.icon ? 'app' : 'glyph',
    pids: procs.length ? procs.map((p) => p.pid) : (g.pids || []), procs, cpu: g.cpu || 0, memory: g.memory || 0, disk: g.diskRate || 0,
    status: normStatus(g.status), windows: g.windows || [],
    canKill: procs.length ? procs.every((p) => p.canKill !== false) : g.canKill !== false,
    efficiency: procs.length ? procs.every((p) => p.efficiency) : !!g.efficiency, cmd: procs[0]?.cmd || '', exe: procs[0]?.exe || '', user: procs[0]?.user || g.user || '',
    procName: procs[0]?.name || '', pid: procs.length === 1 ? procs[0].pid : null,
  };
  if (row.status === 'running' && row.efficiency) row.status = 'efficiency';
  row.children = procs.length > 1 ? procs.map((p) => procRow(p, row)) : null;
  return row;
}

export function buildProcessGroups(data, grouped = true) {
  const byPid = new Map((data?.processes || []).map((p) => [p.pid, p]));
  const apps = (data?.apps || []).map((a) => groupRow('app', a, byPid));
  const bg = (data?.background || []).map((b) => groupRow('bg', b, byPid));
  let sys;
  if (Array.isArray(data?.system)) sys = data.system.map((g) => groupRow('sys', g, byPid));
  else {
    // SPEC-only payloads: group other users' processes by groupKey/name
    const sysMap = new Map();
    for (const p of data?.processes || []) {
      if (p.group !== 'system') continue;
      const k = p.groupKey || p.name;
      if (!sysMap.has(k)) sysMap.set(k, { key: k, name: p.displayName || p.name, icon: null, pids: [], cpu: 0, memory: 0, diskRate: 0, status: 'running' });
      const g = sysMap.get(k);
      g.pids.push(p.pid); g.cpu += p.cpu || 0; g.memory += p.memory || 0; g.diskRate += (p.diskRead || 0) + (p.diskWrite || 0);
    }
    sys = [...sysMap.values()].map((g) => groupRow('sys', g, byPid));
  }
  if (grouped) {
    return [
      { id: 'apps', label: t('apps'), rows: apps },
      { id: 'background', label: t('background'), rows: bg },
      { id: 'system', label: t('system'), rows: sys },
    ];
  }
  return [{ id: 'all', label: '', rows: [...apps, ...bg, ...sys] }];
}

/* ------------------------------------------------------------------ page */

export function createPage(ctx) {
  let grouped = load('proc.grouped', true);
  let data = null;
  let perf = null;
  let query = '';
  const offs = [];

  const memTotal = () => data?.totals?.memoryTotal || perf?.memory?.total || 1;

  const columns = [
    {
      id: 'name', label: t('name'), width: 280, minWidth: 160, flex: true, required: true,
      value: (r) => r.name.toLowerCase(), searchText: (r) => r.name,
      render(content, r, { isNew }) {
        if (isNew || content._iconKey !== `${r.iconKind}:${r.icon}`) {
          clear(content);
          const ic = r.iconKind === 'app' || (r.kind === 'proc' && r.icon)
            ? AppIcon({ name: r.icon, size: 16, fallback: 'app' })
            : h('span', { class: 'tm-glyph' }, icon(r.kind === 'sys' ? 'settings' : 'window', { size: 16 }));
          content._name = h('span', { class: 'tm-name__text' });
          content._count = h('span', { class: 'tm-name__count' });
          content.append(h('span', { class: 'tm-name__icon' }, ic), content._name, content._count);
          content.classList.add('tm-name');
          content._iconKey = `${r.iconKind}:${r.icon}`;
        }
        if (content._name.textContent !== r.name) content._name.textContent = r.name;
        const cnt = r.children ? ` (${r.children.length})` : '';
        if (content._count.textContent !== cnt) content._count.textContent = cnt;
      },
      tip: (r) => (r.windows?.length ? r.windows.join('\n') : ''),
    },
    {
      id: 'status', label: t('status'), width: 140, minWidth: 64,
      value: (r) => ({ notResponding: 0, suspended: 1, efficiency: 2, running: 3 }[r.status] ?? 3),
      render(content, r) {
        if (content._st === r.status) return;
        content._st = r.status;
        clear(content);
        content.className = 'tm-cell__content tm-status';
        if (r.status === 'efficiency') { content.append(leafIcon(16)); content.dataset.tooltip = t('effTip'); }
        else if (r.status === 'notResponding') { content.append(h('span', { class: 'tm-status__hang' }, t('notResponding'))); delete content.dataset.tooltip; }
        else if (r.status === 'suspended') { content.append(pauseIcon(14), h('span', null, t('suspended'))); delete content.dataset.tooltip; }
        else delete content.dataset.tooltip;
      },
    },
    { id: 'pid', label: t('pid'), width: 76, align: 'right', hidden: true, value: (r) => r.pid ?? 0, text: (r) => (r.pid ?? '') },
    { id: 'procName', label: t('procName'), width: 150, hidden: true, value: (r) => r.procName, text: (r) => r.procName },
    { id: 'user', label: t('user'), width: 110, hidden: true, value: (r) => r.user, text: (r) => r.user },
    {
      id: 'cpu', label: t('cpu'), width: 86, minWidth: 64, align: 'right', defaultDesc: true, headerTip: t('cpuTip'),
      value: (r) => r.cpu, text: (r) => cpuPct(r.cpu), heat: (r) => r.cpu / 35,
    },
    {
      id: 'memory', label: t('memory'), width: 104, minWidth: 72, align: 'right', defaultDesc: true, headerTip: t('memTip'),
      value: (r) => r.memory, text: (r) => memMB(r.memory), heat: (r) => r.memory / (memTotal() * 0.1),
    },
    {
      id: 'disk', label: t('disk'), width: 96, minWidth: 64, align: 'right', defaultDesc: true, headerTip: t('diskTip'),
      value: (r) => r.disk, text: (r) => diskMBs(r.disk), heat: (r) => r.disk / (25 * 1048576),
    },
    { id: 'cmd', label: t('cmd'), width: 320, hidden: true, value: (r) => r.cmd, text: (r) => r.cmd, cellClass: 'tm-cell--mono' },
  ];

  const table = DataTable({
    columns, storageKey: 'processes', ariaLabel: t('title'),
    rowKey: (r) => r.key,
    childrenOf: (r) => r.children,
    rowClass: (r) => [r.status === 'notResponding' && 'is-hung', r.status === 'efficiency' && 'is-efficient', !r.canKill && 'is-system'],
    onSelectionChange: () => updateCommands(),
    onDelete: (rows) => { if (rows[0]) endTask(ctx, rows[0]); },
    contextMenu: (rows) => processMenu(ctx, rows, { table, item: table.getSelectedItems()[0] }),
    emptyText: t('loading'),
  });

  /* ---------------------------------------------------------- commands */
  const newBtn = Button({ label: t('newTask'), icon: 'new', variant: 'subtle', onClick: () => ctx.newTask() });
  newBtn.dataset.tooltip = t('newTaskTip');
  const endBtn = Button({ label: t('endTask'), icon: endTaskIcon(16), variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) endTask(ctx, r); } });
  const endWrap = tipWrap(endBtn, t('selectFirst'));
  const restartBtn = Button({ label: t('restart'), icon: 'restart', variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) restartTask(ctx, r); } });
  const restartWrap = tipWrap(restartBtn, t('selectFirst'));
  const effBtn = Button({ label: t('efficiency'), icon: leafOutline(16), variant: 'subtle', disabled: true, onClick: () => { const r = table.getSelection()[0]; if (r) setEfficiency(ctx, r, !targetOf(r).efficiency); } });
  effBtn.setAttribute('aria-pressed', 'false');
  const effWrap = tipWrap(effBtn, t('selectFirst'));
  const viewBtn = Button({ label: t('view'), icon: 'view', iconAfter: 'chevron-down', variant: 'subtle' });
  viewBtn.addEventListener('click', () => {
    showContextMenu({
      anchor: viewBtn, align: 'end',
      items: [
        { label: t('groupByType'), checked: grouped, action: () => { grouped = !grouped; save('proc.grouped', grouped); render(); } },
        { separator: true },
        { label: t('expandAll'), icon: 'chevron-down', action: () => table.expandAll(true) },
        { label: t('collapseAll'), icon: 'chevron-up', action: () => table.expandAll(false) },
        { separator: true },
        {
          label: t('columns'), icon: 'grid', submenu: columns.filter((c) => !c.required).map((c) => ({
            label: c.label, checked: !table.isHidden(c.id), action: () => table.setColumnHidden(c.id, !table.isHidden(c.id)),
          })),
        },
      ],
    });
  });

  function updateCommands() {
    const r = table.getSelection()[0];
    const rt = restartTargetOf(r);
    const restartLabel = rt.kind === 'shell' ? t('restartShell') : t('restart');
    if (restartBtn.textContent !== restartLabel) restartBtn.setLabel(restartLabel);
    restartBtn.setDisabled(!rt.ok);
    restartWrap.setTip(!r ? t('selectFirst') : rt.ok ? (rt.kind === 'shell' ? '' : t('restartTip')) : rt.reason);
    if (!r) {
      endBtn.setDisabled(true); effBtn.setDisabled(true);
      endWrap.setTip(t('selectFirst')); effWrap.setTip(t('selectFirst'));
      effBtn.classList.remove('is-pressed'); effBtn.setAttribute('aria-pressed', 'false');
      return;
    }
    const tg = targetOf(r);
    endBtn.setDisabled(!tg.canKill);
    endWrap.setTip(tg.canKill ? '' : t('systemProc'));
    effBtn.setDisabled(!tg.canKill);
    effWrap.setTip(tg.canKill ? '' : t('systemProc'));
    effBtn.classList.toggle('is-pressed', tg.efficiency);
    effBtn.setAttribute('aria-pressed', String(tg.efficiency));
  }

  /* ------------------------------------------------------------- render */
  function render() {
    if (!data) return;
    table.setGroups(buildProcessGroups(data, grouped));
    updateHeader();
    updateCommands();
  }

  function updateHeader() {
    const tot = data?.totals || {};
    const memPct = tot.memoryTotal ? (tot.memoryUsed / tot.memoryTotal) * 100 : null;
    let disk = null;
    if (tot.diskActive != null) disk = `${Math.round(tot.diskActive)}%`;
    else if (perf?.disks?.length) disk = `${Math.round(Math.max(...perf.disks.map((d) => d.activePct || 0)))}%`;
    else if (tot.diskRead != null) disk = diskMBs((tot.diskRead || 0) + (tot.diskWrite || 0));
    table.setHeaderValues({
      cpu: tot.cpu != null ? `${Math.round(tot.cpu)}%` : '',
      memory: memPct != null ? `${Math.round(memPct)}%` : '',
      disk: disk || '',
    });
  }

  function applyFilter() {
    const q = query.trim().toLowerCase();
    table.setEmptyText(data ? t('noMatch', { q: query.trim() }) : t(health.state === 'ok' ? 'loading' : 'waiting'));
    if (!q) { table.setFilter(null); return; }
    const isNum = /^\d+$/.test(q);
    table.setFilter((r) => {
      if (isNum && (r.pids || []).some((p) => String(p).startsWith(q))) return true;
      return r.name.toLowerCase().includes(q) || String(r.procName || '').toLowerCase().includes(q);
    });
  }

  const el = h('div', { class: 'tm-page__fill tm-processes' }, table.el);
  el.addEventListener('keydown', (e) => {
    if (e.altKey && e.key === 'Enter') { const r = table.getSelection()[0]; if (r) { e.preventDefault(); showProperties(r); } }
  });

  return {
    id: 'processes',
    title: t('title'),
    el,
    commands: [newBtn, endWrap, restartWrap, effWrap, h('span', { class: 'tm-cmd-sep' }), viewBtn],
    mount(params = {}) {
      offs.push(ctx.store.subscribe('processes', (d) => {
        const first = !data;
        data = d;
        render();
        if (first) {
          applyFilter();
          if (params.pid) selectPid(params.pid);
        }
      }));
      offs.push(ctx.store.subscribe('performance', (p) => { perf = p; updateHeader(); }));
      offs.push(health.onChange(() => { if (!data) applyFilter(); }));
      if (!data) applyFilter();
      if (params.pid && data) selectPid(params.pid);
      if (params.sort) sortBy(params.sort);
    },
    navigate(params = {}) { if (params.pid && data) selectPid(params.pid); if (params.sort) sortBy(params.sort); },
    unmount() { while (offs.length) offs.pop()(); },
    onSearch(q) { query = q || ''; applyFilter(); },
    refresh() { return ctx.store.refresh('processes'); },
  };

  function sortBy(col) {
    const id = { memory: 'memory', cpu: 'cpu', disk: 'disk', name: 'name' }[col];
    if (id) table.sortBy(id, id !== 'name');
  }

  function selectPid(pid) {
    const n = Number(pid);
    for (const g of buildProcessGroups(data, grouped)) {
      for (const r of g.rows) {
        if (r.pids.includes(n)) {
          if (r.children && r.pids.length > 1) { table.expand(r.key, true); table.select(`p:${n}`, { focus: true }); } else table.select(r.key, { focus: true });
          return;
        }
      }
    }
  }
}
