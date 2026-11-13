// Win11 Shell — mock of the host methods that only Win11 voor GNOME has (its Rust host, Codex's work) and the
// older mocks did not know (GE0a: tests/i18n/MOCK-CHANGES.md). Without them these surfaces showed
// "Onbekende functie" in mock mode:
//
//   software.inspect / uninstallInfo / prepare / execute / status / cancel / jobs
//       App-installatieprogramma (?app=installer&path=…|uninstall=…), "Verwijderen" in Start and in
//       Instellingen › Apps › Geïnstalleerde apps (core/software.js)
//   apps.mimeTypes / forType / setDefaultForType      Standaard-apps (?app=defaultapps)
//   explorer.tabOffer / tabClaim / tabCommit / tabRelease / tabStatus / tabDetach / tabPosition
//       dragging a Verkenner tab into another window (tabtransfer.js, ?tabTicket=)
//   nativeBackdrop.configure                          GNOME's background blur (only asked under the Rust host)
//
// Shapes and host texts follow host/src/services/software.rs (+ software/inspect.rs, resolve.rs),
// host/src/apps/pertype.rs, host/src/transfers.rs and host/src/runtime.rs. Like the other mocks (mock/files.js),
// host texts follow settings.language (the Rust host's t(lang, nl, en)). Nothing is installed, removed or changed:
// every job is a dry run that ends in "Voorbeeld voltooid". ?mockJobs=1 starts with two finished jobs in the
// activity list (one preview, one failure), so their rows can be dumped without running a job first.

const err = (code, message) => Object.assign(new Error(message), { code });
const T0 = Date.UTC(2026, 9, 25, 9, 12, 0);          // seeded jobs: the morning of the dump's fixed day

/** The Rust host's t(lang, nl, en): the language of the settings (mock/settings.js). */
function L(m, nl, en) {
  let lang = 'nl';
  try { lang = m?.settings?.()?.language === 'en' ? 'en' : 'nl'; } catch { /* keep nl */ }
  return lang === 'en' ? en : nl;
}

/* ================================================================ software.* */

// installation files the mock knows (anything else under /home is "not a supported package")
const PACKAGES = {
  deb: (m, path) => ({
    name: 'Visual Studio Code', package: 'code', version: '1.106.2-1763073473', architecture: 'amd64',
    description: 'Code editing. Redefined.', installedSize: 412876800, essential: false, priority: 'optional',
    notes: [], format: 'deb', provider: 'packagekit', scope: 'system', size: 108412930,
  }),
  appimage: (m, path) => ({
    name: base(path), version: '', architecture: 'x86_64',
    description: L(m, 'Een zelfstandige AppImage-app. Er wordt een beheerde kopie met een snelkoppeling geïnstalleerd.',
      'A standalone AppImage application. A managed copy and launcher will be installed.'),
    notes: [L(m, 'Het gedownloade bestand blijft staan.', 'The downloaded file will be kept.')],
    format: 'appimage', provider: 'appimage', scope: 'user', size: 132907008,
  }),
  flatpakref: (m, path) => ({
    name: 'Kdenlive', version: 'stable', architecture: '', description: 'Video editor', appId: 'org.kde.kdenlive',
    origin: 'https://dl.flathub.org/repo/', notes: [], format: 'flatpakref', provider: 'flatpak', scope: 'user', size: 612,
  }),
  flatpak: (m, path) => ({
    name: base(path), version: '', architecture: '',
    description: L(m, 'Lokaal Flatpak-app-pakket', 'Local Flatpak application bundle'),
    notes: [L(m, 'Flatpak controleert het pakket en de handtekening tijdens de installatie.',
      'Flatpak checks the bundle and its signature during installation.')],
    format: 'flatpak', provider: 'flatpak', scope: 'user', size: 48230400,
  }),
  snap: (m, path) => ({
    name: 'hello-world', snapName: 'hello-world', version: '6.4', architecture: '', description: 'The ‘hello-world’ of snaps',
    type: 'app', confinement: 'strict', format: 'snap', provider: 'snapd', scope: 'system', size: 20480,
    notes: [
      L(m, 'Een lokaal Snap-pakket kan zonder winkelhandtekening worden geïnstalleerd. Installeer het alleen als je de bron vertrouwt.',
        'A local Snap package can be installed without a store signature. Install it only if you trust the source.'),
      L(m, 'Dit pakket gebruikt strikte Snap-isolatie.', 'This package uses strict Snap confinement.'),
    ],
  }),
};

function base(path) {
  const leaf = String(path).split('/').pop() || '';
  return leaf.replace(/\.[^.]*$/, '') || leaf;
}

function kindOf(path) {
  const ext = (String(path).split('/').pop().match(/\.([^.]+)$/)?.[1] || '').toLowerCase();
  return ext === 'appimage' ? 'appimage' : ['deb', 'snap', 'flatpakref', 'flatpak'].includes(ext) ? ext : null;
}

function inspect(path, m) {
  if (typeof path !== 'string' || !path.startsWith('/') || path.includes('\0')) {
    throw err('no_access', L(m, 'Dit bestand kan niet worden gelezen.', 'This file cannot be read.'));
  }
  const kind = kindOf(path);
  if (!kind) {
    throw err('unsupported', L(m, 'Dit bestand is geen ondersteund installatiepakket.',
      'This file is not a supported installation package.'));
  }
  return { ...PACKAGES[kind](m, path), sourcePath: path, supported: true };
}

// installed apps → how the host would remove them (software/resolve.rs)
const OWN = /^win11shell-/;
const BLOCKED_SOURCE = new Set(['steam.desktop', 'nvidia-settings.desktop', 'blender.desktop']);
const FLATPAK = /^(org\.gimp\.GIMP|com\.obsproject\.Studio|org\.kde\.kdenlive)\.desktop$/;
const PACKAGE_OF = {
  'org.gnome.TextEditor.desktop': 'gnome-text-editor', 'org.gnome.Calculator.desktop': 'gnome-calculator',
  'org.gnome.Nautilus.desktop': 'nautilus', 'org.gnome.Ptyxis.desktop': 'ptyxis', 'org.gnome.Loupe.desktop': 'loupe',
  'org.gnome.Papers.desktop': 'papers', 'org.gnome.Rhythmbox3.desktop': 'rhythmbox', 'yelp.desktop': 'yelp',
  'google-chrome.desktop': 'google-chrome-stable', 'org.gnome.FileRoller.desktop': 'file-roller',
};

async function uninstallInfo(id, m) {
  if (typeof id !== 'string' || !id || id.length > 300 || !id.endsWith('.desktop') || id.includes('/')) {
    throw err('bad_args', L(m, 'Ongeldige invoer voor softwarebeheer.', 'Invalid software management input.'));
  }
  let known = [];
  try { known = await m.call('apps.list', {}); } catch { known = []; }
  const app = (Array.isArray(known) ? known : []).find((a) => a.id === id);
  const v = { id, name: app?.name || id, provider: 'unknown', scope: 'system', supported: false, action: 'uninstall', description: '' };
  const blocked = (code, nl, en, extra = {}) => ({ ...v, ...extra, supported: false, reasonCode: code, reason: L(m, nl, en) });
  if (OWN.test(id)) {
    return blocked('protected', 'Win11Shell zelf kan hier niet worden verwijderd.', 'Win11Shell itself cannot be removed here.');
  }
  if (!app) return blocked('not_installed', 'Deze app is niet meer geïnstalleerd.', 'This application is no longer installed.');
  if (FLATPAK.test(id)) {
    return { ...v, provider: 'flatpak', scope: 'user', supported: true,
      description: L(m, 'Verwijdert de Flatpak-app. Persoonlijke app-gegevens blijven staan.',
        'Removes the Flatpak application. Personal application data is kept.') };
  }
  if (/^[a-z0-9-]+_[a-z0-9-]+\.desktop$/.test(id)) {
    if (id.startsWith('snap-store_') || id.startsWith('firmware-updater_')) {
      return blocked('protected', 'Deze Snap hoort bij de systeembasis.', 'This Snap is part of the base system.', { provider: 'snapd' });
    }
    return { ...v, provider: 'snapd', supported: true,
      description: L(m, 'Verwijdert de Snap-app. Snapd behoudt de standaard gegevens-snapshot.',
        'Removes the Snap application. Snapd keeps its default data snapshot.') };
  }
  if (BLOCKED_SOURCE.has(id)) {
    return blocked('unsupported_provider',
      'De installatiebron is niet betrouwbaar vast te stellen. Gebruik de oorspronkelijke pakketbeheerder.',
      'The installation source could not be reliably identified. Use its original package manager.');
  }
  const pkg = PACKAGE_OF[id] || id.replace(/\.desktop$/, '').split('.').pop().toLowerCase();
  if (pkg === 'nautilus') {
    return blocked('protected', 'Dit pakket is essentieel voor het systeem en kan hier niet worden verwijderd.',
      'This package is essential to the system and cannot be removed here.', { provider: 'packagekit' });
  }
  return { ...v, provider: 'packagekit', supported: true, description: `${L(m, 'Verwijdert het pakket', 'Removes package')} ${pkg}` };
}

const plans = new Map();     // planId → plan (single use)
const jobs = new Map();      // jobId → job
let serial = 0;
let seeded = false;

function seed(m) {
  if (seeded) return;
  seeded = true;
  if (m.params?.get?.('mockJobs') !== '1') return;
  jobs.set('mock-job-2', { jobId: 'mock-job-2', planId: 'mock-plan-2', action: 'uninstall', name: 'Kdenlive', provider: 'flatpak',
    state: 'failed', createdAt: Math.floor((T0 + 3600e3) / 1000), progress: 40, cancellable: false, cancelRequested: false, dryRun: false,
    message: L(m, 'De actie is niet voltooid. De pakketbeheerder geeft meer informatie hieronder.',
      'The operation did not complete. The package manager provides more information below.'),
    error: { code: 'provider_error', message: 'error: org.kde.kdenlive/x86_64/stable not installed' } });
  jobs.set('mock-job-1', { jobId: 'mock-job-1', planId: 'mock-plan-1', action: 'install', name: 'Visual Studio Code', provider: 'packagekit',
    state: 'completed', createdAt: Math.floor(T0 / 1000), progress: 100, cancellable: false, cancelRequested: false, dryRun: true,
    message: L(m, 'Voorbeeld voltooid. Er is niets geïnstalleerd of verwijderd.', 'Preview completed. Nothing was installed or removed.'),
    result: { dryRun: true } });
}

const ACTIVE = new Set(['queued', 'running', 'authenticating']);
const copy = (job) => JSON.parse(JSON.stringify(job));

function update(m, job, patch) {
  Object.assign(job, patch);
  m.emit('software.changed', copy(job));
}

/** a dry run in steps of 400 ms: queued → running 35 % → 80 % → completed (or cancelled) */
function run(m, job) {
  const step = (fn, ms) => setTimeout(() => { if (ACTIVE.has(job.state)) fn(); }, ms);
  step(() => update(m, job, { state: 'running', progress: 35,
    message: L(m, 'Software wijzigen…', 'Changing software…') }), 400);
  step(() => job.cancelRequested
    ? update(m, job, { state: 'cancelled', cancellable: false, message: L(m, 'Geannuleerd.', 'Cancelled.') })
    : update(m, job, { progress: 80 }), 800);
  step(() => update(m, job, { state: 'completed', progress: 100, cancellable: false, result: { dryRun: true },
    message: L(m, 'Voorbeeld voltooid. Er is niets geïnstalleerd of verwijderd.', 'Preview completed. Nothing was installed or removed.') }), 1200);
}

/* ======================================================= apps per file type */

// [mime, extensions, nl name, en name, default app id or '', app ids that can open it]
const TYPES = [
  ['application/pdf', ['.pdf'], 'PDF-document', 'PDF document', 'org.gnome.Papers.desktop',
    ['org.gnome.Papers.desktop', 'firefox_firefox.desktop', 'google-chrome.desktop', 'libreoffice-draw.desktop']],
  ['application/vnd.oasis.opendocument.text', ['.odt'], 'ODT-document', 'ODT document', 'libreoffice-writer.desktop',
    ['libreoffice-writer.desktop']],
  ['application/x-compressed-tar', ['.tar.gz', '.tgz'], 'Tar-archief (gzip-gecomprimeerd)', 'Tar archive (gzip-compressed)',
    'org.gnome.FileRoller.desktop', ['org.gnome.FileRoller.desktop']],
  ['application/x-ms-dos-executable', ['.exe'], 'DOS/Windows-uitvoerbaar bestand', 'DOS/Windows executable', '', []],
  ['application/zip', ['.zip'], 'Zip-archief', 'Zip archive', 'org.gnome.FileRoller.desktop', ['org.gnome.FileRoller.desktop']],
  ['audio/mpeg', ['.mp3', '.mpga'], 'MP3-audio', 'MP3 audio', 'org.gnome.Rhythmbox3.desktop',
    ['org.gnome.Rhythmbox3.desktop', 'vlc_vlc.desktop', 'mpv_mpv.desktop']],
  ['image/jpeg', ['.jpg', '.jpeg', '.jpe'], 'JPEG-afbeelding', 'JPEG image', 'org.gnome.Loupe.desktop',
    ['org.gnome.Loupe.desktop', 'org.gimp.GIMP.desktop', 'firefox_firefox.desktop']],
  ['image/png', ['.png'], 'PNG-afbeelding', 'PNG image', 'org.gnome.Loupe.desktop',
    ['org.gnome.Loupe.desktop', 'org.gimp.GIMP.desktop']],
  ['text/html', ['.html', '.htm'], 'HTML-document', 'HTML document', 'firefox_firefox.desktop',
    ['firefox_firefox.desktop', 'google-chrome.desktop', 'org.gnome.TextEditor.desktop']],
  ['text/plain', ['.txt', '.asc'], 'Platte-tekstdocument', 'Plain text document', 'org.gnome.TextEditor.desktop',
    ['org.gnome.TextEditor.desktop', 'code_code.desktop']],
  ['video/mp4', ['.mp4', '.m4v'], 'MPEG-4-video', 'MPEG-4 video', 'vlc_vlc.desktop', ['vlc_vlc.desktop', 'mpv_mpv.desktop']],
];
const defaultFor = new Map(TYPES.map((t) => [t[0], t[4]]));

async function briefs(m) {
  let list = [];
  try { list = await m.call('apps.list', {}); } catch { list = []; }
  return new Map((Array.isArray(list) ? list : []).map((a) => [a.id, { id: a.id, name: a.name, icon: a.icon }]));
}

function record(t, byId, m) {
  const def = defaultFor.get(t[0]);
  return { mime: t[0], name: L(m, t[2], t[3]), extensions: t[1], default: (def && byId.get(def)) || null };
}

function mimeArg(args, m) {
  const mime = args?.mime;
  if (typeof mime !== 'string' || !/^[a-z0-9.+-]+\/[a-z0-9.+_-]+$/i.test(mime)) {
    throw err('bad_args', L(m, 'Ongeldig bestandstype.', 'Invalid file type.'));
  }
  return mime;
}

/* ================================================================== the file */

const tickets = new Map();   // explorer.tab*: ticket → {state, claimed, committed}

export default {
  methods: {
    'software.inspect': async ({ path } = {}, m) => inspect(path, m),
    'software.uninstallInfo': async ({ id } = {}, m) => uninstallInfo(id, m),
    'software.prepare': async (args = {}, m) => {
      let info;
      if (args.action === 'install') info = inspect(args.path, m);
      else if (args.action === 'uninstall') {
        if (typeof args.id !== 'string') throw err('bad_args', L(m, 'Kies één geïnstalleerde app.', 'Choose one installed application.'));
        info = await uninstallInfo(args.id, m);
      } else throw err('bad_args', L(m, 'Ongeldige softwareactie.', 'Invalid software action.'));
      const allowed = info.supported === true;
      const planId = allowed ? `mock-plan-${++serial}` : null;
      const action = args.action === 'install' ? 'install' : (info.action || 'uninstall');
      const plan = { planId, action, name: info.name, provider: info.provider, scope: info.scope, description: info.description,
        requiresAuth: info.scope === 'system' && info.provider !== 'shortcut', canExecute: allowed, notes: info.notes || [], info };
      for (const k of ['sourcePath', 'reasonCode', 'reason']) if (info[k] != null) plan[k] = info[k];
      if (planId) plans.set(planId, plan);
      return plan;
    },
    'software.execute': async ({ planId } = {}, m) => {
      seed(m);
      const plan = plans.get(planId);
      if (!plan) throw err('not_found', L(m, 'Dit voorstel bestaat niet meer. Kies de app opnieuw.', 'This plan is no longer available. Select the application again.'));
      if ([...jobs.values()].some((j) => ACTIVE.has(j.state))) {
        throw err('busy', L(m, 'Er wordt al software gewijzigd. Wacht tot die actie is voltooid.', 'Another software operation is active. Wait for it to finish.'));
      }
      plans.delete(planId);
      const job = { jobId: `mock-job-${100 + serial}`, planId, action: plan.action, name: plan.name, provider: plan.provider,
        state: 'queued', createdAt: Math.floor(Date.now() / 1000), progress: null, cancellable: true, cancelRequested: false,
        message: L(m, 'Wachten op starten…', 'Waiting to start…'), dryRun: true };
      jobs.set(job.jobId, job);
      m.emit('software.changed', copy(job));
      run(m, job);
      return copy(job);
    },
    'software.status': async ({ jobId } = {}, m) => {
      seed(m);
      const job = jobs.get(jobId);
      if (!job) throw err('not_found', L(m, 'Deze softwaretaak is niet gevonden.', 'This software job was not found.'));
      return copy(job);
    },
    'software.cancel': async ({ jobId } = {}, m) => {
      seed(m);
      const job = jobs.get(jobId);
      if (!job) throw err('not_found', L(m, 'Deze softwaretaak is niet gevonden.', 'This software job was not found.'));
      if (!ACTIVE.has(job.state) || job.cancellable !== true) {
        throw err('cannot_cancel', L(m, 'Deze actie kan nu niet worden geannuleerd.', 'This operation cannot be cancelled now.'));
      }
      update(m, job, { cancelRequested: true,
        message: L(m, 'Annuleren aangevraagd. Wachten op de pakketbeheerder…', 'Cancellation requested. Waiting for the package manager…') });
      return copy(job);
    },
    'software.jobs': async (args, m) => {
      seed(m);
      return [...jobs.values()].sort((a, b) => b.createdAt - a.createdAt || (a.jobId < b.jobId ? 1 : -1)).map(copy);
    },

    'apps.mimeTypes': async ({ query = '', offset = 0, limit = 100 } = {}, m) => {
      if (typeof query !== 'string' || query.length > 200) throw err('bad_args', L(m, 'Ongeldige zoekterm.', 'Invalid search term.'));
      const byId = await briefs(m);
      const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
      const items = TYPES.filter((t) => {
        const s = `${t[0]} ${L(m, t[2], t[3])} ${t[1].join(' ')}`.toLowerCase();
        return terms.every((q) => s.includes(q));
      });
      const off = Math.max(0, Number(offset) || 0), lim = Math.min(200, Math.max(1, Number(limit) || 100));
      return { items: items.slice(off, off + lim).map((t) => record(t, byId, m)), total: items.length, offset: off, limit: lim };
    },
    'apps.forType': async (args = {}, m) => {
      const mime = mimeArg(args, m);
      const byId = await briefs(m);
      const t = TYPES.find((x) => x[0] === mime) || [mime, [], mime, mime, '', []];
      const out = record(t, byId, m);
      const def = out.default?.id;
      const coll = new Intl.Collator('nl', { sensitivity: 'base' });
      out.apps = t[5].map((id) => byId.get(id)).filter(Boolean).map((a) => ({ ...a, isDefault: a.id === def }))
        .sort((a, b) => (b.isDefault - a.isDefault) || coll.compare(a.name, b.name));
      return out;
    },
    'apps.setDefaultForType': async (args = {}, m) => {
      const mime = mimeArg(args, m);
      const id = args.id;
      if (typeof id !== 'string' || !id || id.length > 300 || id.includes('/')) throw err('bad_args', L(m, 'Ongeldige app.', 'Invalid app.'));
      const byId = await briefs(m);
      if (!byId.has(id)) throw err('not_found', L(m, 'Deze app is niet gevonden.', 'This app was not found.'));
      defaultFor.set(mime, id);
      return { ok: true, dryRun: true, types: [mime], effective: true, current: id };
    },

    'explorer.tabOffer': async ({ ticket, state } = {}) => {
      if (typeof ticket !== 'string' || ticket.length < 32) throw err('tab_transfer', 'Ongeldige tab-overdrachtscode');
      tickets.set(ticket, { state, claimed: false, committed: false });
      return { ticket, sourceWindow: 'mock' };
    },
    'explorer.tabClaim': async ({ ticket } = {}) => {
      const o = tickets.get(ticket);
      if (!o) throw err('tab_transfer', 'Tab-overdracht is verlopen; de brontab blijft behouden');
      if (o.claimed) throw err('tab_transfer', 'De tab wordt al door een ander venster overgenomen');
      o.claimed = true;
      return { state: o.state, sourceWindow: 'mock' };
    },
    'explorer.tabCommit': async ({ ticket } = {}) => {
      const o = tickets.get(ticket);
      if (!o?.claimed) throw err('tab_transfer', 'Tab-overdracht is verlopen; de brontab blijft behouden');
      o.committed = true;
      return { committed: true };
    },
    'explorer.tabRelease': async ({ ticket } = {}) => {
      const o = tickets.get(ticket);
      if (o?.committed) throw err('tab_transfer', 'Een bevestigde overdracht kan niet worden ingetrokken');
      if (o && !o.claimed) tickets.delete(ticket); else if (o) o.claimed = false;
      return {};
    },
    'explorer.tabStatus': async ({ ticket } = {}) => {
      const o = tickets.get(ticket);
      if (!o) throw err('tab_transfer', 'Tab-overdracht is verlopen; de brontab blijft behouden');
      return { committed: o.committed };
    },
    // the real host starts a new win11-explorer window with --tab-ticket; the mock has no second window
    'explorer.tabDetach': async ({ ticket } = {}) => {
      if (!tickets.has(ticket)) throw err('tab_transfer', 'Tab-overdracht is verlopen; de brontab blijft behouden');
      return {};
    },
    'explorer.tabPosition': async () => ({}),

    // GNOME Shell's background blur behind our windows: the mock has no compositor
    'nativeBackdrop.configure': async () => ({ available: false, generation: 0, serial: 0, pid: 0, windows: [] }),
  },
};
