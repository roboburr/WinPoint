// Taakbeheer — actions on processes (shared by Processen and Details):
// end task (with a friendly confirmation and a "force" second step), restart (the host says first what it would
// do — sysmon.restartPlan — and Taakbeheer asks the matching question: an app closes with all its windows, always
// asked; one process or a service after a confirmation; our own shell: "Win11 Shell opnieuw starten", like
// Windows' "Windows Verkenner opnieuw starten"), efficiency mode, open file location, search online, properties,
// and the context menu.

import { h } from '../../core/dom.js';
import { bridge, health, isTransportError } from './hostcall.js';
import { Dialog, Toast, Checkbox, AppIcon } from '../../core/components.js';
import { getSetting, setSetting } from '../../core/theme.js';
import { makeT } from '../../core/i18n.js';
import { date, time, relTime, number } from '../../core/format.js';
import { icon } from '../../core/icons.js';
import { cpuPct, memMB, diskMBs, exePath, dirName, leafOutline, endTaskIcon, load, save } from './util.js';

export const t = makeT({
  nl: {
    endTask: 'Taak beëindigen', forceEnd: 'Geforceerd beëindigen', cancel: 'Annuleren',
    endTitle: '‘{name}’ beëindigen?',
    endBody: '{name} wordt meteen gesloten. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.',
    endBodyMany: { one: 'Dit sluit {n} proces van {name}. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.',
      other: 'Dit sluit {n} processen van {name}. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.' },
    endTip: 'Tip: lukt het nog om de app zelf te sluiten? Dan is dat veiliger.',
    endTitleMulti: { one: '{n} proces beëindigen?', other: '{n} processen beëindigen?' },
    endBodyMulti: 'Deze processen worden meteen gesloten: {list}. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.',
    dontAsk: 'Niet meer vragen',
    hangTitle: '‘{name}’ reageert niet',
    hangBody: '{name} heeft het verzoek om te stoppen genegeerd. Je kunt de app geforceerd beëindigen; werk dat niet is opgeslagen, gaat dan verloren.',
    ended: '{name} is beëindigd', endFailed: '{name} kon niet worden beëindigd',
    dryRun: 'Testmodus: er is niets echt beëindigd',
    systemProc: 'Systeemproces – kan niet worden beëindigd',
    effTitle: 'Efficiëntiemodus inschakelen voor ‘{name}’?',
    effBody: '{name} krijgt minder voorrang van de processor. Andere apps blijven daardoor vlot en je pc verbruikt minder stroom. De app zelf kan wat trager reageren.',
    effOn: 'Efficiëntiemodus inschakelen', efficiency: 'Efficiëntiemodus',
    effEnabled: 'Efficiëntiemodus staat aan voor {name}', effDisabled: 'Efficiëntiemodus staat uit voor {name}',
    effFailed: 'Efficiëntiemodus kon niet worden aangepast',
    details: 'Details bekijken', fileLocation: 'Bestandslocatie openen', searchOnline: 'Online zoeken', properties: 'Eigenschappen',
    expand: 'Uitvouwen', collapse: 'Samenvouwen', noLocation: 'Locatie onbekend',
    propsTitle: 'Eigenschappen van {name}', close: 'Sluiten',
    pName: 'Naam', pDesc: 'Omschrijving', pPid: 'Proces-id (PID)', pPids: 'Processen', pUser: 'Gebruiker', pStatus: 'Status',
    pStarted: 'Gestart', pCpu: 'Processor', pMem: 'Geheugen', pDisk: 'Schijf', pThreads: 'Threads', pCmd: 'Opdrachtregel', pLoc: 'Locatie',
    pWindows: 'Vensters', pCount: { one: '{n} proces', other: '{n} processen' }, copy: 'Kopiëren', copied: 'Gekopieerd',
    st_running: 'Actief', st_notResponding: 'Reageert niet', st_suspended: 'Onderbroken', st_efficiency: 'Efficiëntiemodus',
    st_zombie: 'Afgesloten', st_waiting: 'Wacht',
    failedGeneric: 'Er ging iets mis. Probeer het opnieuw.',
    restart: 'Opnieuw starten', restartShell: 'Win11 Shell opnieuw starten',
    restartTitle: '‘{name}’ opnieuw starten?',
    restartBody: '{name} wordt gestopt en meteen weer gestart. Dat helpt vaak als iets is blijven hangen.',
    restartBodyService: 'De service {name} wordt gestopt en meteen weer gestart. Dat helpt vaak als iets is blijven hangen.',
    restartHint: 'Werk dat nog niet is opgeslagen, kan verloren gaan.',
    restartBodyApp: '‘{proc}’ hoort bij {name}. Daarom wordt {name} helemaal gesloten en meteen weer geopend, met al zijn vensters.',
    restartBodyAppRow: '{name} wordt helemaal gesloten en meteen weer geopend, met al zijn vensters.',
    restarting: '{name} wordt opnieuw gestart…', restarted: '{name} is opnieuw gestart',
    restartFailed: '{name} kon niet opnieuw worden gestart', restartDryRun: 'Testmodus: er is niets echt opnieuw gestart',
    dry_exec: '‘{name}’ zou stoppen en met dezelfde opdracht opnieuw starten.', dry_app: '{name} zou sluiten en opnieuw openen.',
    dry_service: 'De service {name} zou opnieuw starten.', dry_shell: 'Win11 Shell zou opnieuw starten.',
    restartSystem: 'Systeemproces – kan niet opnieuw worden gestart',
    restartPickOne: 'Vouw de groep uit en kies één proces om opnieuw te starten',
    restartOne: 'Selecteer één proces om opnieuw te starten',
    restartUnsupported: 'Deze versie van Win11 Shell kan nog geen processen opnieuw starten.',
    restartAnyway: 'Toch opnieuw starten',
    shellFailed: 'Win11 Shell kon niet opnieuw worden gestart', shellDryRun: 'Testmodus: Win11 Shell is niet echt opnieuw gestart',
    shellTip: 'De taakbalk en het bureaublad starten opnieuw.',
  },
  en: {
    endTask: 'End task', forceEnd: 'Force end', cancel: 'Cancel',
    endTitle: 'End ‘{name}’?',
    endBody: '{name} will close right away. Any work you haven’t saved might be lost.',
    endBodyMany: { one: '{n} process of {name} will close. Any work you haven’t saved might be lost.',
      other: 'All {n} processes of {name} will close. Any work you haven’t saved might be lost.' },
    endTip: 'Tip: if you can still close the app yourself, that’s safer.',
    endTitleMulti: { one: 'End {n} process?', other: 'End {n} processes?' },
    endBodyMulti: 'These processes will close right away: {list}. Any work you haven’t saved might be lost.',
    dontAsk: 'Don’t ask again',
    hangTitle: '‘{name}’ is not responding',
    hangBody: '{name} ignored the request to stop. You can force it to end; unsaved work will then be lost.',
    ended: '{name} has ended', endFailed: '{name} couldn’t be ended',
    dryRun: 'Test mode: nothing was actually ended',
    systemProc: 'System process – can’t be ended',
    effTitle: 'Turn on efficiency mode for ‘{name}’?',
    effBody: '{name} gets a lower priority on the processor. Other apps stay responsive and your PC uses less power. The app itself may respond a little slower.',
    effOn: 'Turn on efficiency mode', efficiency: 'Efficiency mode',
    effEnabled: 'Efficiency mode is on for {name}', effDisabled: 'Efficiency mode is off for {name}',
    effFailed: 'Efficiency mode couldn’t be changed',
    details: 'Go to details', fileLocation: 'Open file location', searchOnline: 'Search online', properties: 'Properties',
    expand: 'Expand', collapse: 'Collapse', noLocation: 'Location unknown',
    propsTitle: '{name} Properties', close: 'Close',
    pName: 'Name', pDesc: 'Description', pPid: 'Process ID (PID)', pPids: 'Processes', pUser: 'User', pStatus: 'Status',
    pStarted: 'Started', pCpu: 'CPU', pMem: 'Memory', pDisk: 'Disk', pThreads: 'Threads', pCmd: 'Command line', pLoc: 'Location',
    pWindows: 'Windows', pCount: { one: '{n} process', other: '{n} processes' }, copy: 'Copy', copied: 'Copied',
    st_running: 'Running', st_notResponding: 'Not responding', st_suspended: 'Suspended', st_efficiency: 'Efficiency mode',
    st_zombie: 'Exited', st_waiting: 'Waiting',
    failedGeneric: 'Something went wrong. Try again.',
    restart: 'Restart', restartShell: 'Restart Win11 Shell',
    restartTitle: 'Restart ‘{name}’?',
    restartBody: '{name} will stop and start again right away. This often helps when something is stuck.',
    restartBodyService: 'The service {name} will stop and start again right away. This often helps when something is stuck.',
    restartHint: 'Any work you haven’t saved might be lost.',
    restartBodyApp: '‘{proc}’ belongs to {name}, so {name} will close completely, with all its windows, and open again right away.',
    restartBodyAppRow: '{name} will close completely, with all its windows, and open again right away.',
    restarting: 'Restarting {name}…', restarted: '{name} has restarted',
    restartFailed: '{name} couldn’t be restarted', restartDryRun: 'Test mode: nothing was actually restarted',
    dry_exec: '‘{name}’ would stop and start again with the same command.', dry_app: '{name} would close and open again.',
    dry_service: 'The service {name} would restart.', dry_shell: 'Win11 Shell would restart.',
    restartSystem: 'System process – can’t be restarted',
    restartPickOne: 'Expand the group and choose one process to restart',
    restartOne: 'Select one process to restart',
    restartUnsupported: 'This version of Win11 Shell can’t restart processes yet.',
    restartAnyway: 'Restart anyway',
    shellFailed: 'Win11 Shell couldn’t be restarted', shellDryRun: 'Test mode: Win11 Shell wasn’t actually restarted',
    shellTip: 'The taskbar and the desktop will restart.',
  },
});

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function listNames(procs) {
  const names = procs.map((p) => `${p.name} (${p.pid})`);
  return names.length > 6 ? `${names.slice(0, 6).join(', ')} …` : names.join(', ');
}

/** Dialog.show puts focus on the first input (our "don't ask again" box); prefer Cancel. */
function focusCancel(promise) {
  setTimeout(() => promise.dialog?.el.querySelector('[data-result="close"]')?.focus({ preventScroll: true }), 30);
  return promise;
}

/** The host's status code of an app (sysmon: running | notResponding | suspended | efficiency); older or other
 *  spellings (the mock's 'not-responding', 'stopped', 'hung') by their English words. Never by display text. */
const STATUS_CODES = { running: 'running', notresponding: 'notResponding', suspended: 'suspended', efficiency: 'efficiency' };
export function normStatus(s) {
  const code = STATUS_CODES[String(s || '').toLowerCase()];
  if (code) return code;
  const v = String(s || '').toLowerCase().replace(/[\s_]/g, '-');
  if (/not-?responding|unresponsive|hung/.test(v)) return 'notResponding';
  if (/suspend|stopped/.test(v)) return 'suspended';
  if (/efficien/.test(v)) return 'efficiency';
  return 'running';
}

/** The action target for a table row: {name, icon, pids, procs, canKill, efficiency}. */
export function targetOf(row) {
  const procs = row.procs || [];
  return {
    name: row.name, icon: row.icon, pids: row.pids || procs.map((p) => p.pid), procs,
    canKill: procs.length ? procs.every((p) => p.canKill !== false) : row.canKill !== false,
    efficiency: procs.length ? procs.every((p) => p.efficiency) : !!row.efficiency,
    cmd: row.cmd || procs[0]?.cmd || '', exe: row.exe || procs[0]?.exe || '', row,
  };
}

function iconFor(tg, size = 32) {
  return tg.icon ? AppIcon({ name: tg.icon, size, fallback: 'process' }) : h('span', { class: 'tm-dlg-glyph' }, icon('process', { size: Math.round(size * 0.7) }));
}

/* --------------------------------------------------------------- end task */

export async function endTask(ctx, row) {
  const tg = targetOf(row);
  if (!tg.canKill || !tg.pids.length) return false;
  if (getSetting('taskmgr.confirmEndTask', true)) {
    const dontAsk = Checkbox({ label: t('dontAsk') });
    const content = h('div', { class: 'tm-dlg' },
      h('div', { class: 'tm-dlg__row' }, iconFor(tg, 32),
        h('div', { class: 'tm-dlg__text' },
          h('p', null, row.multi ? t('endBodyMulti', { list: listNames(tg.procs) })
            : tg.pids.length > 1 ? t('endBodyMany', { name: tg.name, n: tg.pids.length }) : t('endBody', { name: tg.name })),
          h('p', { class: 'tm-dlg__hint' }, t('endTip')))),
      dontAsk);
    const r = await focusCancel(Dialog.show({ title: row.multi ? t('endTitleMulti', { n: tg.pids.length }) : t('endTitle', { name: tg.name }), content, primary: t('endTask'), close: t('cancel'), danger: true, width: 460 }));
    if (r !== 'primary') return false;
    if (dontAsk.checked) setSetting('taskmgr.confirmEndTask', false).catch(() => {});
  }
  return doEnd(ctx, tg, false);
}

async function doEnd(ctx, tg, force) {
  let res;
  try {
    res = await bridge.call('sysmon.endTask', { pids: tg.pids, force });
  } catch (e) {
    Toast.show({ title: t('endFailed', { name: tg.name }), message: e.message || t('failedGeneric'), severity: 'error' });
    return false;
  }
  const results = res?.results || [];
  const failed = results.filter((r) => !r.ok);
  if (res?.dryRun && !bridge.isMock) {
    Toast.show({ title: t('dryRun'), severity: 'info' });
    ctx.store.refresh('processes');
    return true;
  }
  if (failed.length && failed.length === results.length) {
    await Dialog.alert({ title: t('endFailed', { name: tg.name }), message: failed[0].error || t('failedGeneric'), icon: 'error' });
    ctx.store.refresh('processes');
    return false;
  }
  if (force) {
    await sleep(400);
    await ctx.store.refresh('processes');
    Toast.show({ title: t('ended', { name: tg.name }), severity: 'success', timeout: 3500 });
    return true;
  }
  // give the app a moment to close itself, then check
  await sleep(1800);
  const data = await ctx.store.refresh('processes');
  const alive = new Set((data?.processes || []).map((p) => p.pid));
  const still = tg.pids.filter((p) => alive.has(p));
  if (!still.length) {
    Toast.show({ title: t('ended', { name: tg.name }), severity: 'success', timeout: 3500 });
    return true;
  }
  const content = h('div', { class: 'tm-dlg' },
    h('div', { class: 'tm-dlg__row' }, iconFor(tg, 32), h('div', { class: 'tm-dlg__text' }, h('p', null, t('hangBody', { name: tg.name })))));
  const r = await Dialog.show({ title: t('hangTitle', { name: tg.name }), content, primary: t('forceEnd'), close: t('cancel'), danger: true, width: 460 });
  if (r !== 'primary') return false;
  return doEnd(ctx, { ...tg, pids: still }, true);
}

/* ---------------------------------------------------------------- restart */

const SHELL_KEY = 'win11shell';

/** Processes of a group whose parent is not in the group (the "main" processes), lowest pid first. */
export function rootsOf(procs) {
  const pids = new Set(procs.map((p) => p.pid));
  return procs.filter((p) => !pids.has(p.ppid)).sort((a, b) => a.pid - b.pid);
}

/** The host's pid (root of the 'Win11 Shell' group) from a sysmon.processes answer, or null. */
export function shellPid(data) {
  if (!data) return null;
  const byPid = new Map((data.processes || []).map((p) => [p.pid, p]));
  const g = (data.background || []).find((b) => b.key === SHELL_KEY);
  const procs = g ? (g.pids || []).map((pid) => byPid.get(pid)).filter(Boolean)
    : (data.processes || []).filter((p) => p.groupKey === SHELL_KEY);
  return rootsOf(procs)[0]?.pid ?? g?.pids?.[0] ?? null;
}

/** True for the Win11 Shell row, its child processes and the host's processes in Details. */
export function isShellRow(row) {
  if (!row) return false;
  if (row.key === `b:${SHELL_KEY}` || row.parentRow?.key === `b:${SHELL_KEY}`) return true;
  const procs = row.procs || [];
  return procs.length > 0 && procs.every((p) => p.groupKey === SHELL_KEY);
}

/**
 * What "Opnieuw starten" does for a row, as far as the table knows:
 * {ok, kind:'shell'|'app'|'service'|'process', name, icon, pid, via, reason}. An app row → the app's main process; a
 * process of an app (Details, an expanded app) → that process: the host decides (sysmon.restartPlan) whether the
 * app restarts or only that process (one with a terminal of its own: a script in a terminal app).
 */
export function restartTargetOf(row) {
  if (!row) return { ok: false, reason: t('restartOne') };
  if (isShellRow(row)) return { ok: true, kind: 'shell', name: 'Win11 Shell', icon: null };
  if (row.multi) return { ok: false, reason: t('restartOne') };
  const app = row.kind === 'app' ? row : (row.parentRow?.kind === 'app' ? row.parentRow : row.appRow || null);
  if (app) {
    const procs = app.procs || [];
    const root = rootsOf(procs)[0];
    // the selected process itself (the host knows whether it is part of the app or a program of its own)
    const ownPids = row.kind === 'app' ? [] : (row.pids || (row.procs || []).map((p) => p.pid));
    const own = ownPids.length === 1 ? (row.procs || []).find((p) => p.pid === ownPids[0]) || { pid: ownPids[0] } : null;
    const pid = own?.pid ?? root?.pid ?? (app.pids || [])[0];
    const ok = !!pid && (own ? own.canKill !== false : root ? root.canKill !== false : app.canKill !== false);
    const via = row.kind === 'app' || (app.pids || []).length <= 1 || (root && pid === root.pid) ? null
      : (row.name || row.displayName || null);
    return { ok, kind: 'app', name: app.name, icon: app.icon, pid, via, reason: ok ? null : t('restartSystem') };
  }
  const tg = targetOf(row);
  if (!tg.canKill) return { ok: false, reason: t('restartSystem') };
  const procs = tg.procs;
  const gkey = String(procs[0]?.groupKey || '');
  const unit = /^unit:.+\.service$/.test(gkey) && procs.every((p) => p.groupKey === gkey);
  const roots = rootsOf(procs);
  if (roots.length > 1 && !unit) return { ok: false, reason: t('restartPickOne') };
  const pid = roots[0]?.pid ?? (tg.pids.length === 1 ? tg.pids[0] : null);
  if (!pid) return { ok: false, reason: t('restartOne') };
  return { ok: true, kind: unit ? 'service' : 'process', name: tg.name, icon: tg.icon, pid, reason: null };
}

/** "Win11 Shell opnieuw starten" (Taakbeheer's own banner, menu and command bar). */
export async function restartShell(ctx) {
  if (health.state === 'restarting') return null;
  try {
    const r = await health.restartShell({ pid: shellPid(ctx.store.get('processes')) });
    if (r.how === 'dryRun') Toast.show({ title: t('shellDryRun'), severity: 'info' });
    return r;
  } catch (e) {
    await Dialog.alert({ title: t('shellFailed'), message: e.code === 'unknown_method' ? t('restartUnsupported') : (e.message || t('failedGeneric')), icon: 'error' });
    return null;
  }
}

/** The host's plan for "Opnieuw starten" ({how, name, via?, count?, tty?}); null from an older host. Refusals throw. */
async function restartPlanOf(pid) {
  try {
    return await bridge.call('sysmon.restartPlan', { pid }, { timeout: 6000 });
  } catch (e) {
    if (e.code === 'unknown_method') return null;
    throw e;
  }
}

/** Restart the app or process of a row: the host says what that means, Taakbeheer asks the matching question. */
export async function restartTask(ctx, row) {
  const target = restartTargetOf(row);
  if (target.kind === 'shell') return restartShell(ctx);
  if (!target.ok) return false;
  let plan;
  try {
    plan = await restartPlanOf(target.pid);
  } catch (e) {
    const message = e.message || t('failedGeneric');
    if (isTransportError(e)) Toast.show({ title: t('restartFailed', { name: target.name }), message, severity: 'error' });
    else await Dialog.alert({ title: t('restartFailed', { name: target.name }), message, icon: 'error' });
    return false;
  }
  if (plan?.how === 'shell') return restartShell(ctx);
  const how = plan?.how || (target.kind === 'process' ? 'exec' : target.kind);
  const rt = { ...target, kind: how === 'exec' ? 'process' : how, name: (plan && plan.name) || target.name,
    via: plan ? plan.via || null : target.via };
  if (how === 'app') {
    // always asked: the app closes with all its windows (work that was not saved can be lost)
    const content = h('div', { class: 'tm-dlg' },
      h('div', { class: 'tm-dlg__row' }, iconFor(rt, 32),
        h('div', { class: 'tm-dlg__text' },
          h('p', null, rt.via ? t('restartBodyApp', { name: rt.name, proc: rt.via }) : t('restartBodyAppRow', { name: rt.name })),
          h('p', { class: 'tm-dlg__hint' }, t('restartHint')))));
    const r = await focusCancel(Dialog.show({ title: t('restartTitle', { name: rt.name }), content, primary: t('restart'), close: t('cancel'), defaultButton: 'close', width: 460 }));
    if (r !== 'primary') return false;
  } else if (!load('restartNoConfirm', false)) {
    const dontAsk = Checkbox({ label: t('dontAsk') });
    const content = h('div', { class: 'tm-dlg' },
      h('div', { class: 'tm-dlg__row' }, iconFor(rt, 32),
        h('div', { class: 'tm-dlg__text' },
          h('p', null, t(how === 'service' ? 'restartBodyService' : 'restartBody', { name: rt.name })),
          h('p', { class: 'tm-dlg__hint' }, t('restartHint')))),
      dontAsk);
    const r = await focusCancel(Dialog.show({ title: t('restartTitle', { name: rt.name }), content, primary: t('restart'), close: t('cancel'), defaultButton: 'close', width: 460 }));
    if (r !== 'primary') return false;
    if (dontAsk.checked) save('restartNoConfirm', true);
  }
  let busy = Toast.show({ title: t('restarting', { name: rt.name }), icon: 'restart', timeout: 0 });
  let res;
  try {
    try {
      res = await bridge.call('sysmon.restartProcess', { pid: rt.pid });
    } catch (e) {
      // the host asks first for some processes (e.g. one that runs in a terminal): its own question
      if (e.code !== 'confirm_required') throw e;
      busy.close();
      const content = h('div', { class: 'tm-dlg' }, h('div', { class: 'tm-dlg__row' }, iconFor(rt, 32),
        h('div', { class: 'tm-dlg__text' }, h('p', null, e.message))));
      const r = await focusCancel(Dialog.show({ title: t('restartTitle', { name: rt.name }), content, primary: t('restartAnyway'), close: t('cancel'), defaultButton: 'close', width: 460 }));
      if (r !== 'primary') return false;
      busy = Toast.show({ title: t('restarting', { name: rt.name }), icon: 'restart', timeout: 0 });
      res = await bridge.call('sysmon.restartProcess', { pid: rt.pid, confirm: true });
    }
  } catch (e) {
    busy.close();
    const message = e.code === 'unknown_method' ? t('restartUnsupported') : (e.message || t('failedGeneric'));
    if (isTransportError(e)) Toast.show({ title: t('restartFailed', { name: rt.name }), message, severity: 'error' });
    else await Dialog.alert({ title: t('restartFailed', { name: rt.name }), message, icon: 'error' });
    ctx.store.refresh('processes');
    return false;
  }
  busy.close();
  if (res?.dryRun && !bridge.isMock) {
    // the host's --dry-run says what it would have done (sysmon.restartProcess → {how, name})
    const plan = ['exec', 'app', 'service', 'shell'].includes(res.how) ? t(`dry_${res.how}`, { name: res.name || rt.name }) : undefined;
    Toast.show({ title: t('restartDryRun'), message: plan, severity: 'info' });
  } else if (res && res.ok === false) {
    await Dialog.alert({ title: t('restartFailed', { name: rt.name }), message: res.error || t('failedGeneric'), icon: 'error' });
    ctx.store.refresh('processes');
    return false;
  } else if (res?.how === 'shell') {
    health.expectRestart('api');
  } else {
    Toast.show({ title: t('restarted', { name: rt.name }), severity: 'success', timeout: 3500 });
  }
  await sleep(300);
  ctx.store.refresh('processes');
  return true;
}

/* ------------------------------------------------------------- efficiency */

export async function setEfficiency(ctx, row, enable) {
  const tg = targetOf(row);
  if (!tg.pids.length) return false;
  if (enable && !load('effNoConfirm', false)) {
    const dontAsk = Checkbox({ label: t('dontAsk') });
    const content = h('div', { class: 'tm-dlg' },
      h('div', { class: 'tm-dlg__row' }, h('span', { class: 'tm-dlg-leaf' }, leafOutline(28)),
        h('div', { class: 'tm-dlg__text' }, h('p', null, t('effBody', { name: tg.name })))),
      dontAsk);
    const r = await focusCancel(Dialog.show({ title: t('effTitle', { name: tg.name }), content, primary: t('effOn'), close: t('cancel'), width: 460 }));
    if (r !== 'primary') return false;
    if (dontAsk.checked) save('effNoConfirm', true);
  }
  try {
    const res = await bridge.call('sysmon.setEfficiency', { pids: tg.pids, enabled: !!enable });
    const failed = (res?.results || []).filter((r) => !r.ok);
    if (failed.length && failed.length === (res.results || []).length) {
      Toast.show({ title: t('effFailed'), message: failed[0].error || '', severity: 'error' });
    } else {
      Toast.show({ title: t(enable ? 'effEnabled' : 'effDisabled', { name: tg.name }), icon: leafOutline(20), timeout: 3000 });
    }
  } catch (e) {
    Toast.show({ title: t('effFailed'), message: e.message || '', severity: 'error' });
  }
  ctx.store.refresh('processes');
  return true;
}

/* -------------------------------------------------------------- the rest */

const exeOf = (tg) => (tg.exe && tg.exe.startsWith('/') ? tg.exe : exePath(tg.cmd));

export function openFileLocation(row) {
  const exe = exeOf(targetOf(row));
  if (!exe) return;
  bridge.call('files.open', { path: dirName(exe) }).catch((e) => Toast.show({ title: t('fileLocation'), message: e.message, severity: 'error' }));
}

export function searchOnline(row) {
  const tg = targetOf(row);
  const proc = tg.procs[0];
  const term = proc && proc.name && proc.name !== tg.name ? `${tg.name} ${proc.name}` : tg.name;
  const engine = getSetting('search.webEngine', 'https://duckduckgo.com/?q=');
  bridge.call('apps.openUri', { uri: engine + encodeURIComponent(`${term} linux`) })
    .catch((e) => Toast.show({ title: t('searchOnline'), message: e.message, severity: 'error' }));
}

function statusText(row) {
  const s = normStatus(row.status);
  return t(`st_${s}`);
}

export function showProperties(row) {
  const tg = targetOf(row);
  const p = tg.procs[0] || row;
  const many = tg.procs.length > 1;
  const exe = exeOf(tg);
  const kv = (k, v, cls) => [h('dt', null, k), h('dd', { class: cls }, v)];
  const cmdEl = h('code', { class: 'tm-props__cmd selectable' }, tg.cmd || '–');
  const copyBtn = h('button', { type: 'button', class: 'fl-link tm-props__copy' }, icon('copy', { size: 14 }), h('span', null, t('copy')));
  copyBtn.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(tg.cmd); copyBtn.lastChild.textContent = t('copied'); } catch { /* ignore */ }
  });
  const started = p.started ? `${date(p.started, 'medium')} ${time(p.started)} (${relTime(p.started)})` : '–';
  const totalMem = tg.procs.length ? tg.procs.reduce((a, x) => a + (x.memory || 0), 0) : row.memory;
  const totalCpu = tg.procs.length ? tg.procs.reduce((a, x) => a + (x.cpu || 0), 0) : row.cpu;
  const totalDisk = tg.procs.length ? tg.procs.reduce((a, x) => a + (x.diskRead || 0) + (x.diskWrite || 0), 0) : row.disk;
  const threads = tg.procs.reduce((a, x) => a + (x.threads || 0), 0);
  const dl = h('dl', { class: 'tm-props__grid' },
    kv(t('pName'), p.name || tg.name),
    p.displayName && p.displayName !== p.name ? kv(t('pDesc'), p.displayName) : null,
    many ? kv(t('pPids'), t('pCount', { n: tg.procs.length })) : kv(t('pPid'), String(p.pid ?? '–')),
    kv(t('pUser'), p.user || '–'),
    kv(t('pStatus'), statusText(row)),
    kv(t('pStarted'), started),
    kv(t('pCpu'), cpuPct(totalCpu)),
    kv(t('pMem'), memMB(totalMem)),
    kv(t('pDisk'), diskMBs(totalDisk)),
    threads ? kv(t('pThreads'), number(threads)) : null,
    row.windows?.length ? kv(t('pWindows'), row.windows.join('\n'), 'tm-props__pre') : null,
    kv(t('pLoc'), exe ? dirName(exe) : t('noLocation'), 'selectable'),
    kv(t('pCmd'), h('div', { class: 'tm-props__cmdwrap' }, cmdEl, copyBtn)),
  );
  const content = h('div', { class: 'tm-props' },
    h('div', { class: 'tm-props__head' }, iconFor(tg, 40), h('div', null,
      h('div', { class: 'subtitle' }, tg.name),
      h('div', { class: 'caption text-2' }, many ? t('pCount', { n: tg.procs.length }) : `PID ${p.pid ?? '–'}`))),
    dl);
  Dialog.show({ title: t('propsTitle', { name: tg.name }), content, close: t('close'), width: 560 });
}

/** Context menu items for one or more selected rows. */
export function processMenu(ctx, rows, { table, item } = {}) {
  const row = rows[0];
  if (!row) return null;
  const tg = targetOf(row);
  const canEff = tg.procs.length ? tg.procs.every((p) => p.user === ctx.user || p.canKill) : tg.canKill;
  const items = [];
  const rt = restartTargetOf(row);
  if (rt.kind === 'shell') {
    // like "Windows Verkenner opnieuw starten": first in the menu of our own shell
    items.push({ label: t('restartShell'), icon: 'restart', description: t('shellTip'), action: () => restartShell(ctx) }, { separator: true });
  }
  if (item?.hasKids && table) {
    const open = table.rowEl(item.key)?.classList.contains('is-expanded');
    items.push({ label: open ? t('collapse') : t('expand'), icon: open ? 'chevron-up' : 'chevron-down', action: () => table.expand(item.key, !open) });
    items.push({ separator: true });
  }
  items.push(
    { label: t('endTask'), icon: endTaskIcon(16), shortcut: 'Del', disabled: !tg.canKill, description: tg.canKill ? null : t('systemProc'), action: () => endTask(ctx, row) },
    ...(rt.kind === 'shell' ? [] : [{ label: t('restart'), icon: 'restart', disabled: !rt.ok, description: rt.ok ? null : rt.reason, action: () => restartTask(ctx, row) }]),
    { label: t('efficiency'), icon: tg.efficiency ? null : leafOutline(16), checked: tg.efficiency ? true : undefined, disabled: !canEff, action: () => setEfficiency(ctx, row, !tg.efficiency) },
    { separator: true },
    { label: t('details'), icon: 'list', action: () => ctx.navigate('details', { pid: tg.pids[0] }) },
    { label: t('fileLocation'), icon: 'folder-open', disabled: !exeOf(tg), action: () => openFileLocation(row) },
    { label: t('searchOnline'), icon: 'globe', action: () => searchOnline(row) },
    { separator: true },
    { label: t('properties'), icon: 'properties', shortcut: 'Alt+Enter', action: () => showProperties(row) },
  );
  return { items };
}
