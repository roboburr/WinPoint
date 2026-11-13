// Shared software review and job tracking. Only an explicit reviewed-plan button executes a job.
import { bridge } from './bridge.js';
import { h, clear, loadCss } from './dom.js';
import { makeT, onLangChange } from './i18n.js';
import { Button, Dialog, InfoBar, ProgressBar, Toast } from './components.js';

export const softwareT = makeT({
  nl: {
    install: 'Installeren', uninstall: 'Verwijderen', removeShortcut: 'Snelkoppeling verwijderen',
    installName: '‘{name}’ installeren', uninstallName: '‘{name}’ verwijderen', removeShortcutName: 'Snelkoppeling ‘{name}’ verwijderen',
    review: 'Installatie controleren', reviewRemove: 'Verwijderen controleren', provider: 'Pakketbeheerder', scope: 'Installatie voor',
    user: 'Alleen mijzelf', system: 'Alle gebruikers', version: 'Versie', architecture: 'Architectuur', size: 'Grootte', source: 'Bestand', action: 'Actie',
    auth: 'Er kan om je wachtwoord worden gevraagd.', cancel: 'Annuleren', close: 'Sluiten', checking: 'Beschikbaarheid controleren…',
    unavailable: 'Deze actie is niet beschikbaar.', failed: 'De actie kon niet worden gestart',
    removeOnly: 'Alleen de snelkoppeling wordt verwijderd. Het programma en het oorspronkelijke bestand blijven behouden.',
    queued: 'Wachten op starten…', running: 'Bezig…', authenticating: 'Wachten op toestemming…', completed: 'Voltooid', failedJob: 'Mislukt',
    cancelled: 'Geannuleerd', interrupted: 'Uitkomst onbekend', interruptedBody: 'Controleer eerst of de app is geïnstalleerd of verwijderd voordat je het opnieuw probeert.',
    preview: 'Voorbeeld voltooid', previewBody: 'Er is niets geïnstalleerd of verwijderd.', cancelRequested: 'Annulering aangevraagd. Wachten op de pakketbeheerder…',
    cancelFailed: 'Annuleren is nu niet mogelijk', statusFailed: 'De voortgang is tijdelijk niet beschikbaar. Opnieuw verbinden…',
    shortcutProvider: 'Snelkoppeling', retryReview: 'Controleer Activiteit voordat je opnieuw probeert. Controleer de actie opnieuw voor een nieuwe poging.',
  },
  en: {
    install: 'Install', uninstall: 'Uninstall', removeShortcut: 'Remove shortcut',
    installName: 'Install “{name}”', uninstallName: 'Uninstall “{name}”', removeShortcutName: 'Remove shortcut “{name}”',
    review: 'Review installation', reviewRemove: 'Review removal', provider: 'Package manager', scope: 'Install for',
    user: 'Just me', system: 'All users', version: 'Version', architecture: 'Architecture', size: 'Size', source: 'File', action: 'Action',
    auth: 'You may be asked for your password.', cancel: 'Cancel', close: 'Close', checking: 'Checking availability…',
    unavailable: 'This action isn’t available.', failed: 'The operation couldn’t be started',
    removeOnly: 'Only the shortcut will be removed. The program and the original file will be kept.',
    queued: 'Waiting to start…', running: 'Working…', authenticating: 'Waiting for authorisation…', completed: 'Completed', failedJob: 'Failed',
    cancelled: 'Cancelled', interrupted: 'Outcome unknown', interruptedBody: 'Check whether the app was installed or removed before trying again.',
    preview: 'Preview completed', previewBody: 'Nothing was installed or removed.', cancelRequested: 'Cancellation requested. Waiting for the package manager…',
    cancelFailed: 'Can’t cancel right now', statusFailed: 'Progress is temporarily unavailable. Reconnecting…',
    shortcutProvider: 'Shortcut', retryReview: 'Check Activity and review the operation once more before you try again.',
  },
});
const t = softwareT;
const PROVIDERS = {appimage: 'AppImage', packagekit: 'PackageKit', flatpak: 'Flatpak', snapd: 'Snap', shortcut: 'Shortcut'};
const TERMINAL = new Set(['completed', 'failed', 'cancelled', 'interrupted']);
export const softwareJobFinished = (job) => TERMINAL.has(job?.state);
export const softwareActionLabel = (action, name) => t(name ? `${action}Name` : action, {name});
export const softwareProvider = (provider) => provider === 'shortcut' ? t('shortcutProvider') : (PROVIDERS[provider] || String(provider || ''));

export function softwareDetails(info = {}, {plan = false} = {}) {
  const rows = [];
  const add = (key, value) => { if (value !== undefined && value !== null && value !== '') rows.push(h('dt', null, t(key)), h('dd', null, String(value))); };
  if (plan) add('action', softwareActionLabel(info.action));
  add('provider', softwareProvider(info.provider));
  add('scope', t(info.scope === 'system' ? 'system' : 'user'));
  const meta = info.info || info;
  add('version', meta.version); add('architecture', meta.architecture);
  if (Number.isFinite(meta.size)) add('size', new Intl.NumberFormat(document.documentElement.lang || 'en', {style: 'unit', unit: 'megabyte', maximumFractionDigits: 2}).format(meta.size / 1e6));
  add('source', info.sourcePath || meta.sourcePath);
  return h('dl', {class: 'sw-details'}, rows);
}

let reviewing = false;
/** Preparation may inspect/capture bytes; it never authorises execution. */
export async function reviewSoftware(args) {
  if (reviewing) return null;
  reviewing = true;
  try { return await showReview(args); }
  finally { reviewing = false; }
}
async function showReview(args) {
  await loadCss('software');
  let plan;
  try { plan = await bridge.call('software.prepare', args); }
  catch (e) { Toast.show({severity: 'error', title: t('failed'), message: e.message}); return null; }
  const status = h('div', {class: 'sw-status', 'aria-live': 'polite'});
  const content = h('div', {class: 'sw-review', dataset: {planId: plan.planId || '', action: plan.action, provider: plan.provider}},
    h('h3', {class: 'sw-name'}, plan.name),
    plan.description ? h('p', {class: 'sw-description'}, plan.description) : null,
    softwareDetails(plan, {plan: true}),
    plan.action === 'removeShortcut' ? InfoBar({message: t('removeOnly'), closable: false}) : null,
    ...(plan.notes || []).map((note) => h('p', {class: 'sw-note'}, note)),
    plan.requiresAuth ? InfoBar({message: t('auth'), closable: false}) : null,
    !plan.canExecute ? InfoBar({severity: 'warning', message: plan.reason || t('unavailable'), closable: false}) : null, status);
  let job = null, attempted = false;
  const executable = plan.canExecute === true && typeof plan.planId === 'string' && !!plan.planId;
  const dialog = Dialog.show({title: t(plan.action === 'install' ? 'review' : 'reviewRemove'), content, width: 560,
    primary: executable ? softwareActionLabel(plan.action, plan.name) : undefined,
    close: t('cancel'), danger: plan.action !== 'install', defaultButton: 'close',
    async onPrimary() {
      if (!executable || attempted) return false;
      attempted = true;
      clear(status);
      try { job = await bridge.call('software.execute', {planId: plan.planId}); return true; }
      catch (e) {
        status.append(InfoBar({severity: 'error', title: t('failed'), message: e.message, closable: false}), h('p', {class: 'sw-note'}, t('retryReview')));
        // A failed request may have consumed the single-use plan. Require another review to retry.
        const primary = dialog.dialog.el.querySelector('[data-result="primary"]');
        setTimeout(() => { if (primary) primary.disabled = true; }, 0);
        return false;
      }
    }});
  const off = onLangChange(() => { if (!dialog.dialog.el.classList.contains('is-busy')) dialog.dialog.close(); });
  await dialog; off();
  return job;
}

export async function softwareRemovalItem(app, onRemove) {
  try {
    const info = await bridge.call('software.uninstallInfo', {id: app.id});
    return {label: softwareActionLabel(info.action || 'uninstall'), icon: 'delete',
      disabled: info.supported !== true, description: info.supported ? undefined : (info.reason || t('unavailable')),
      action: () => onRemove(info)};
  } catch (e) { return {label: t('uninstall'), icon: 'delete', disabled: true, description: e.message || t('unavailable')}; }
}

/** Adds capability in place without reopening a menu or making a stale response steal focus. */
export function populateSoftwareRemoval(menu, items, app, onRemove) {
  softwareRemovalItem(app, onRemove).then((item) => menu?.update([...items, {separator: true}, item]));
}
export const softwareRemovalPlaceholder = () => ({label: t('uninstall'), icon: 'delete', disabled: true, description: t('checking')});

/** A job can execute in another host; poll the persisted native snapshot as well as listening for events. */
export function createSoftwareJobView(initial, {onChange} = {}) {
  loadCss('software');
  let job = initial, disposed = false, timer = 0, pending = false, failure = '', cancelling = false, revision = 0;
  const el = h('section', {class: 'sw-job', 'aria-live': 'polite', dataset: {jobId: initial.jobId}});
  function render() {
    clear(el); el.dataset.state = job.state;
    const terminal = softwareJobFinished(job);
    const state = job.state === 'failed' ? 'failedJob' : job.state;
    el.append(h('h3', {class: 'sw-name'}, job.name), h('p', {class: 'sw-job-action'}, `${softwareActionLabel(job.action)} · ${softwareProvider(job.provider)}`),
      h('p', {class: 'sw-job-state'}, t(job.dryRun && job.state === 'completed' ? 'preview' : state)),
      ProgressBar({value: Number.isFinite(job.progress) ? job.progress : 0, indeterminate: !terminal && !Number.isFinite(job.progress), error: job.state === 'failed', ariaLabel: job.name}));
    if (job.message) el.append(h('p', {class: 'sw-job-message'}, job.message));
    if (job.dryRun && job.state === 'completed') el.append(InfoBar({message: t('previewBody'), closable: false}));
    if (job.state === 'interrupted') el.append(InfoBar({severity: 'warning', message: t('interruptedBody'), closable: false}));
    if (job.error?.message) el.append(InfoBar({severity: 'error', message: job.error.message, closable: false}));
    if (job.cancelRequested && !terminal) el.append(h('p', {class: 'sw-note'}, t('cancelRequested')));
    if (failure) el.append(InfoBar({severity: 'warning', message: failure, closable: false}));
    if (!terminal) el.append(Button({label: t('cancel'), disabled: cancelling || job.cancelRequested || !job.cancellable,
      async onClick() {
        if (cancelling || job.cancelRequested || !job.cancellable) return;
        cancelling = true; ++revision; render();
        try { const value = await bridge.call('software.cancel', {jobId: job.jobId}); if (value?.jobId === job.jobId) job = value; failure = ''; }
        catch (e) { failure = e.message || t('cancelFailed'); }
        finally { cancelling = false; ++revision; if (!disposed) { render(); poll(); } }
      }}));
  }
  function schedule() { clearTimeout(timer); if (!disposed && !softwareJobFinished(job)) timer = setTimeout(poll, 500); }
  async function poll() {
    if (disposed || pending) return;
    pending = true; clearTimeout(timer);
    const requestRevision = revision;
    try {
      const value = await bridge.call('software.status', {jobId: job.jobId});
      if (disposed || requestRevision !== revision) return;
      job = value; failure = ''; render(); onChange?.(job);
    } catch { if (!disposed && requestRevision === revision) { failure = t('statusFailed'); render(); } }
    finally { pending = false; schedule(); }
  }
  const offEvent = bridge.on('software.changed', () => { if (!softwareJobFinished(job)) poll(); });
  const offLanguage = onLangChange(render);
  const dispose = () => { disposed = true; clearTimeout(timer); offEvent(); offLanguage(); removeEventListener('pagehide', dispose); };
  addEventListener('pagehide', dispose, {once: true});
  render(); if (!softwareJobFinished(job)) poll();
  return {el, dispose, get job() {return job;}};
}

export async function reviewAndTrackSoftware(args) {
  const job = await reviewSoftware(args);
  if (!job) return null;
  const view = createSoftwareJobView(job);
  await Dialog.show({title: softwareActionLabel(job.action, job.name), content: view.el, close: t('close'), width: 560});
  view.dispose(); return view.job;
}
