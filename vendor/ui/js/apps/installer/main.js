// Standalone installer using the original Fluent AppFrame and reviewed native software plans.
import { createAppFrame } from '../appframe.js';
import { h, clear, loadCss } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { makeT, onLangChange } from '../../core/i18n.js';
import { AppIcon, Button, EmptyState, InfoBar, ProgressRing, SettingsCard, SettingsGroup } from '../../core/components.js';
import { softwareT, softwareDetails, softwareActionLabel, softwareProvider, softwareJobFinished, reviewSoftware, createSoftwareJobView } from '../../core/software.js';

const t = makeT({
  nl: {title: 'App-installatieprogramma', intro: 'Installeer een app uit een bestand of bekijk de voortgang van je installaties.',
    choose: 'Bestand kiezen', drop: 'Sleep een installatiebestand hierheen', formats: 'AppImage, Debian (.deb), Flatpak en Snap',
    filter: 'Installatiebestanden', inspecting: 'Bestand controleren…', inspectFailed: 'Het bestand kon niet worden gecontroleerd',
    one: 'Kies één installatiebestand tegelijk.', nativePath: 'Kies een bestand met de knop of sleep het vanuit Verkenner.',
    review: 'Installatie controleren', removeReview: 'Verwijderen controleren', unavailable: 'Deze actie is niet beschikbaar.',
    activity: 'Activiteit', empty: 'Nog geen installaties', emptyBody: 'De voortgang van je installaties en verwijderingen verschijnt hier.',
    jobsFailed: 'De activiteit kon niet worden geladen.', retry: 'Opnieuw proberen', show: 'Voortgang bekijken', remove: 'Verwijderen',
    original: 'Het oorspronkelijke installatiebestand blijft behouden.',},
  en: {title: 'App Installer', intro: 'Install an app from a file or view the progress of your installations.',
    choose: 'Choose file', drop: 'Drag an installation file here', formats: 'AppImage, Debian (.deb), Flatpak and Snap',
    filter: 'Installation files', inspecting: 'Checking file…', inspectFailed: 'The file couldn’t be checked',
    one: 'Choose one installation file at a time.', nativePath: 'Use the button to choose a file, or drag it from File Explorer.',
    review: 'Review installation', removeReview: 'Review removal', unavailable: 'This action isn’t available.',
    activity: 'Activity', empty: 'No installations yet', emptyBody: 'Your installation and removal progress will appear here.',
    jobsFailed: 'Activity couldn’t be loaded.', retry: 'Try again', show: 'View progress', remove: 'Uninstall',
    original: 'The original installation file will be kept.',},
});
const patterns = ['*.AppImage', '*.appimage', '*.deb', '*.flatpakref', '*.flatpak', '*.snap'];

export async function mountApp(root, params = {}) {
  await Promise.all([loadCss('installer'), loadCss('software')]);
  const frame = createAppFrame({app: 'installer', title: t('title'), icon: 'download'});
  frame.el.classList.add('si-app'); root.append(frame.el);
  let selection = null, loading = false, error = '', requestId = 0, busy = false;
  let jobs = [], jobsError = '', jobView = null, jobTimer = 0, jobsPending = false, disposed = false;
  const heading = h('h1', {class: 'si-title'}, t('title'));
  const intro = h('p', {class: 'si-intro'}, t('intro'));
  const chooser = Button({label: t('choose'), icon: 'folder-open', onClick: browse});
  const dropLabel = h('h2', {class: 'si-drop-title'}, t('drop'));
  const formats = h('p', {class: 'si-formats'}, t('formats'));
  const drop = h('section', {class: 'si-drop', 'aria-label': t('drop')}, AppIcon({name: 'application-x-executable', size: 48}), dropLabel, formats, chooser);
  const detail = h('section', {class: 'si-detail', 'aria-live': 'polite'});
  const progress = h('section', {class: 'si-progress'});
  const activityTitle = h('h2', {class: 'si-section-title'}, t('activity'));
  const activity = h('div', {class: 'si-activity', 'aria-live': 'polite'});
  frame.setContent(h('div', {class: 'si-page'}, h('header', {class: 'si-header'}, heading, intro), drop, detail, progress,
    h('section', {class: 'si-activity-section'}, activityTitle, activity)));

  function renderDetail() {
    clear(detail); chooser.disabled = busy;
    if (loading) { detail.append(h('div', {class: 'si-loading'}, ProgressRing({size: 24}), h('span', null, t('inspecting')))); return; }
    if (error) { detail.append(InfoBar({severity: 'error', title: t('inspectFailed'), message: error, closable: false})); return; }
    if (!selection) return;
    const {info, action} = selection;
    detail.append(SettingsGroup({children: [SettingsCard({icon: AppIcon({name: 'application-x-executable', size: 40}), title: info.name,
      description: info.description, control: h('span', {class: 'si-provider'}, softwareProvider(info.provider))})]}),
      softwareDetails(info),
      ...(info.notes || []).map((note) => h('p', {class: 'sw-note'}, note)));
    if (action === 'uninstall' && info.action === 'removeShortcut') detail.append(InfoBar({message: softwareT('removeOnly'), closable: false}));
    if (!info.supported) detail.append(InfoBar({severity: 'warning', message: info.reason || t('unavailable'), closable: false}));
    else detail.append(Button({label: t(action === 'install' ? 'review' : 'removeReview'), variant: 'accent', disabled: busy,
      onClick: async () => {
        if (busy) return;
        busy = true; renderDetail();
        try {
          const job = await reviewSoftware(action === 'install' ? {action, path: info.sourcePath} : {action, id: info.id});
          if (job) { showJob(job); loadJobs(); }
        } finally { busy = false; renderDetail(); }
      }}));
  }
  async function inspect(path) {
    if (busy || disposed) return;
    const current = ++requestId; loading = true; selection = null; error = ''; renderDetail();
    try {
      if (typeof path !== 'string' || !path.startsWith('/')) throw new Error(t('nativePath'));
      const info = await bridge.call('software.inspect', {path});
      if (current !== requestId || disposed) return;
      selection = {action: 'install', info};
    } catch (e) { if (current === requestId) error = e.message; }
    finally { if (current === requestId && !disposed) { loading = false; renderDetail(); } }
  }
  async function removal(id) {
    if (busy || disposed) return;
    const current = ++requestId; loading = true; selection = null; error = ''; renderDetail();
    try { const info = await bridge.call('software.uninstallInfo', {id}); if (current === requestId) selection = {action: 'uninstall', info}; }
    catch (e) { if (current === requestId) error = e.message; }
    finally { if (current === requestId && !disposed) { loading = false; renderDetail(); } }
  }
  async function browse() {
    if (busy) return;
    try { const value = await bridge.call('shell.pickFile', {title: t('choose'), filters: [{name: t('filter'), patterns}]}); if (value?.path) inspect(value.path); }
    catch (e) { error = e.message; renderDetail(); }
  }
  function showJob(job) {
    jobView?.dispose(); clear(progress);
    jobView = createSoftwareJobView(job, {onChange: (value) => {
      const index = jobs.findIndex((item) => item.jobId === value.jobId);
      if (index < 0) jobs.unshift(value); else jobs[index] = value;
      renderJobs();
    }});
    progress.append(jobView.el);
  }
  function renderJobs() {
    clear(activity);
    if (jobsError) { activity.append(InfoBar({severity: 'warning', message: jobsError, closable: false, action: {label: t('retry'), onClick: loadJobs}})); return; }
    if (!jobs.length) { activity.append(EmptyState({icon: 'download', title: t('empty'), message: t('emptyBody')})); return; }
    activity.append(SettingsGroup({children: jobs.map((job) => {
      const state = job.dryRun && job.state === 'completed' ? 'preview' : (job.state === 'failed' ? 'failedJob' : job.state);
      const row = SettingsCard({icon: 'download', title: job.name,
        description: `${softwareActionLabel(job.action)} · ${softwareProvider(job.provider)} · ${softwareT(state)}`,
        control: Button({label: t('show'), onClick: () => showJob(job)})});
      row.dataset.jobId = job.jobId; row.dataset.state = job.state; return row;
    })}));
  }
  async function loadJobs() {
    if (disposed || jobsPending) return;
    jobsPending = true; clearTimeout(jobTimer);
    try { const value = await bridge.call('software.jobs'); if (disposed) return; jobs = Array.isArray(value) ? value : (value.jobs || []); jobsError = ''; }
    catch (e) { if (!disposed) jobsError = e.message || t('jobsFailed'); }
    finally { jobsPending = false; if (!disposed) { renderJobs(); if (jobsError || jobs.some((job) => !softwareJobFinished(job))) jobTimer = setTimeout(loadJobs, 500); } }
  }
  function onNativeDrop(event) {
    if (busy) return;
    const paths = event.detail?.paths;
    if (!Array.isArray(paths) || paths.length !== 1) { ++requestId; loading = false; selection = null; error = t('one'); renderDetail(); return; }
    inspect(paths[0]);
  }
  // DOM File.name has no trustworthy native path. Native Wry drop carries the absolute path.
  drop.addEventListener('dragover', (event) => { event.preventDefault(); drop.classList.add('is-dragging'); });
  drop.addEventListener('dragleave', () => drop.classList.remove('is-dragging'));
  drop.addEventListener('drop', (event) => { event.preventDefault(); drop.classList.remove('is-dragging'); });
  addEventListener('win11:files-dropped', onNativeDrop);
  const offJobs = bridge.on('software.changed', loadJobs);
  const offNavigate = frame.onAppNavigate((value) => { if (value.uninstall) removal(value.uninstall); else if (value.path) inspect(value.path); });
  const offLanguage = onLangChange(() => {
    frame.setTitle(t('title')); heading.textContent = t('title'); intro.textContent = t('intro');
    chooser.setLabel(t('choose')); dropLabel.textContent = t('drop'); drop.setAttribute('aria-label', t('drop')); formats.textContent = t('formats');
    activityTitle.textContent = t('activity'); renderDetail(); renderJobs();
  });
  addEventListener('pagehide', () => { disposed = true; ++requestId; clearTimeout(jobTimer); jobView?.dispose(); offJobs(); offNavigate(); offLanguage(); removeEventListener('win11:files-dropped', onNativeDrop); }, {once: true});
  renderDetail(); renderJobs(); loadJobs();
  if (params.uninstall) await removal(params.uninstall);
  else if (params.path) await inspect(params.path);
}
