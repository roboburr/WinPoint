// Win11 Shell — mock update.* methods (Windows Update, GW3; notes/claude/windows-update-gnome.md §3.2-§3.4, §4.3, §4.4).
//
// update.status answers with the snapshot of the Rust service (§4.3) for one page state. ?updateScenario=<id> picks it
// (tests/i18n/surfaces.json settings-update.<id>, the ids of §9.3); without it the page starts in H03:
//   H01 idle · H02 checking · H03 available (mixed rows, 2 phased) · H04 simulating · H05 downloading 45 % with Cancel ·
//   H06 installing 30 % · H07 waiting for the lock (unattended-upgrades) · H08 waiting for permission · H09 foreign ·
//   H10 restartRequired (gnome-shell; libsoup is covered by the reboot) · H11 restartRequired + Ook beschikbaar ·
//   H12 error network (+ Technische details) · H13 unsupported (no PackageKit) · H14 offline · H15 repairRequired ·
//   H16 only phased + one held package (phasedLater, keptBack) · H17 metered InfoBar · H18 metered confirm ·
//   H19 partial refresh warning · H20 Win11 restart InfoBar (libsoup) · H21 removals confirm · H22 cancelled ·
//   H23 cannot cancel · H24 not authorized · H25 releaseRunning · H26 foreignOwn (an own Software job) ·
//   H27 restartRequired + the WebKitGTK InfoBar next to it (§3.4 exception) · H28 a host without update.* (the
//   page's fallback to system.updates: update.status is an unknown method) ·
//   R01 pending download / downloading / pending install · R02 installing + installed + pending restart ·
//   R03 install error with Retry per row · R04 a row's packages (offered later, not installed now) · R05 the
//   "Wat is er nieuw" details (update.details).
// Text variants of those states (GW3 fix round; a letter after the id): H05b downloading without an ETA · H05c
//   downloading without a known total (no caption) · H07b waiting for another program's lock · H10b
//   restartRequired without a package list · H10c with 5 packages ("en 2 andere") · H10d H10 with "Nu opnieuw
//   opstarten" clicked (the page's restart confirm) · H10e and confirmed (the page calls system.power.action
//   itself: the mock answers dryRun, so the test-mode toast shows) · H14b offline, never checked · H20b the Win11 restart InfoBar
//   with every restart.shellParts code · H25b releaseRunning without a version · R05b "Wat is er nieuw" of a row
//   without details · R06 the GTK 3, libsoup and GLib rows.
// The subpages: update.history, update.options and update.setAutoOpen have their own states (GW4, the "S" ids in the
// section "Updategeschiedenis and Geavanceerde opties" below); update.release.* answer with idle results here (GW6
// adds the release card's scenarios to this file).
// The "Nieuwe Ubuntu-versies" row of Meer opties (GW5, §3.6; tests/i18n surfaces settings-update-prompt.<id>, the
// page itself idle as H01): M01 Prompt normal (with "Ubuntu 26.10 is uitgebracht; …" and the /bin/sh InfoBar of a PC
// without the root helper: today's state; every other scenario shows the row like this, H25 disabled while Ubuntu's
// upgrader runs) · M01b the file says never ("Nooit melden") · M02 lts on an interim release (25.10) · M03 overridden
// by a release-upgrades.d file (the ComboBox disabled, no InfoBar) · M04 a change waiting for Ubuntu's password
// dialog without the root helper (the surface picks "Alleen LTS-versies"; the ComboBox disabled, the InfoBar stays) ·
// M04b a change with the root helper and the password dialog dismissed (pkexec 126: the host's error InfoBar).
// update.release.setPrompt changes release.prompt live (no password asked in the mock).
// The feature-update card (GW6, §3.5; tests/i18n surfaces settings-update-card.<id>, the ids of §9.3 plus letters):
// C01 released but not offered (no card; the Prompt row's line) · C02 checking · C03 ok (9 months) · C03b ok, an LTS
// offer from an interim release (5 years) · C04 warn (the surface opens "Let op voordat je upgradet": ffmpeg, two
// third-party sources) · C05 hold, the archive's GNOME 51 named · C05b hold by releases.json alone (no major named:
// today's verdict once 26.10 is offered) · C05c hold with allowAnyway ("Toch upgraden…") · C05d C05c's link
// clicked (the start dialog with the hold warning) · C06 blocked (libarchive removed, WebKitGTK and JavaScriptCore
// missing) · C07 updatesFirst · C07b updatesFirst with a held package named · C08 rebootFirst (the hero's
// restartRequired too) · C09 the start dialog of C03 on battery, metered, with the English-first line (the surface
// clicks Downloaden en installeren; ?mockBattery=1) · C10 running while our own update job still downloads (with
// the hero's releaseRunning the card stays away: H25) · C11 waiting · C12 ubuntuHold · C13 unknown (apt) · C13b
// unknown, offline · C13c unknown, metered · C14 startFailed (pkexec 126) · C14b C14 after Retry (back to the offer)
// · C15 ended with Logboek openen · C16 hold after "Ook de melding van Ubuntu uitstellen" · C16b C05 with that link
// clicked (live). Live: update.release.compat {refresh} → checking 1.5 s → ok; update.release.ubuntuHold flips
// ubuntuPromptHeld; update.release.start → running (the hero's releaseRunning).
//
// Live behaviour (on a click, also with ?mockStatic=1): update.check → checking 1.5 s → H03's rows; update.install
// → simulate, a confirm (metered when ?mockMetered=1, see below), download with update.progress, install, then
// restartRequired (when a kernel row was installed) or idle; update.cancel while allowCancel → available + the
// cancelled InfoBar; update.confirm; update.repair → idle; update.release.attach → {started:true}.
// Events as the host sends them (§4.4): update.changed {rev, phase, totals, restart, job, error, release:{state,
// version}} on every structural change, update.progress {jobId, status, pct, speed, remaining, downloadLeft,
// allowCancel, rows:{id:{state, pct}}} at most 4 per second.
//
// Host texts (error and warning messages, §3.7) are rendered from HOST below in the language of the call, with
// key + params next to the message (contracts C2): the page matches on key/code, never on message. The snapshot's
// error and warnings carry key + params; a thrown error (an immediate refusal) keeps code, message, key, params and
// data too (mock/index.js mkErr, round 3), as the real bridge, core/bridge.js, does. The
// rows must equal host/i18n/update.tsv (tests/i18n/test_update_mock_rows.py).
// Console: __mockUpdate.scenario('H05'), __mockUpdate.state().

import { lang } from '../core/i18n.js';

const clone = (v) => JSON.parse(JSON.stringify(v));
const err = (code, message, extra = {}) => Object.assign(new Error(message), { code }, extra);

/* host rows (key → [nl, en]) of host/i18n/update.tsv that the mock sends; checked by tests/i18n/test_update_mock_rows.py */
/* host-rows:begin */
const HOST = {
  'update.product': ['Win11 Shell', 'Win11 Shell'],
  'update.err.network': ['Er is geen verbinding met de updateserver. Controleer je internetverbinding en probeer het opnieuw.', 'Can’t reach the update server. Check your internet connection and try again.'],
  'update.warn.source': ['Een updatebron kon niet worden bijgewerkt: {source}. De andere updates zijn wel gecontroleerd.', 'One update source couldn’t be refreshed: {source}. The other updates were checked.'],
  'update.err.offline': ['Je bent offline. Maak verbinding met internet en probeer het opnieuw.', 'You’re offline. Connect to the internet and try again.'],
  'update.err.repair': ['Een eerdere installatie is niet afgerond. Ubuntu moet dit eerst herstellen.', 'An earlier installation didn’t finish. Ubuntu needs to repair this first.'],
  'update.err.cannotCancel': ['De installatie is al begonnen en kan nu niet meer worden gestopt.', 'Installing has already started and can’t be stopped now.'],
  'update.err.notAuthorized': ['Je hebt geen toestemming gegeven, dus er is niets veranderd.', 'Permission wasn’t given, so nothing was changed.'],
  'update.err.dpkg': ['Een update is niet goed geïnstalleerd. Probeer het opnieuw; helpt dat niet, open dan de Software-updater.', 'An update didn’t install correctly. Try again; if that doesn’t help, open Software Updater.'],
  'update.err.cancelled': ['Het installeren is gestopt. Er is niets half geïnstalleerd.', 'Installing was stopped. Nothing was left half-installed.'],
  'update.err.noPackageKit': ['PackageKit ontbreekt, dus {product} kan zelf geen updates installeren.', 'PackageKit is missing, so {product} can’t install updates itself.'],
  'update.err.busy': ['Er wordt al een update geïnstalleerd.', 'An update is already being installed.'],
  'update.err.stale': ['De lijst met updates is veranderd. Controleer opnieuw op updates.', 'The list of updates changed. Check for updates again.'],
  'update.err.badArgs': ['Ongeldige invoer voor updates.', 'Invalid update input.'],
  'update.err.details': ['De details van deze update zijn niet beschikbaar.', 'The details of this update aren’t available.'],
  'update.guard.upgrade': ['Ubuntu wordt bijgewerkt naar een nieuwe versie. Wacht tot het upgradevenster om een herstart vraagt.', 'Ubuntu is being upgraded to a new version. Wait until the upgrade window asks you to restart.'],
  'update.release.err.noPassword': ['Je hebt het wachtwoord niet ingevuld, dus er is niets veranderd.', 'You didn’t enter the password, so nothing was changed.'],
  // WU-FIX (GW2b review n6, GW4 review m8): an install refused during a check; setAutoOpen's own error
  'update.err.checking': ['Er wordt nog op updates gecontroleerd. Probeer het daarna opnieuw.', 'Still checking for updates. Try again afterwards.'],
  'update.options.err.autoOpen': ['De instelling voor de Software-updater kon niet worden opgeslagen.', 'The setting for Software Updater couldn’t be saved.'],
  // GW6: the card's reasons (update.compat.* with their update.part.* / update.feature.* texts)
  'update.compat.missing': ['{part} zit niet in Ubuntu {version}.', '{part} isn’t in Ubuntu {version}.'],
  'update.compat.removed': ['Ubuntu {version} verwijdert {part}.', 'Ubuntu {version} removes {part}.'],
  'update.compat.degraded': ['{part} ontbreekt in Ubuntu {version}; {feature} werkt dan niet.', '{part} is missing from Ubuntu {version}; {feature} won’t work then.'],
  'update.compat.thirdParty.one': ['Ubuntu zet tijdens de upgrade 1 extra softwarebron uit ({name}).', 'Ubuntu turns off 1 extra software source during the upgrade ({name}).'],
  'update.compat.thirdParty.other': ['Ubuntu zet tijdens de upgrade {n} extra softwarebronnen uit, zoals {name}.', 'Ubuntu turns off {n} extra software sources during the upgrade, such as {name}.'],
  'update.compat.offline': ['Je bent offline.', 'You’re offline.'],
  'update.compat.metered': ['Je gebruikt een verbinding met een datalimiet; zet Updates downloaden via verbindingen met een datalimiet aan om toch te controleren.', 'You’re on a metered connection; turn on Download updates over metered connections to check anyway.'],
  'update.compat.apt': ['Het pakketoverzicht van Ubuntu {version} kon niet worden geladen.', 'The package list of Ubuntu {version} couldn’t be loaded.'],
  'update.part.webkit': ['WebKitGTK (webweergave)', 'WebKitGTK (web view)'],
  'update.part.jsc': ['JavaScriptCore', 'JavaScriptCore'],
  'update.part.archive': ['libarchive (archieven openen)', 'libarchive (opening archives)'],
  'update.part.ffmpeg': ['ffmpeg', 'ffmpeg'],
  'update.feature.ffmpeg': ['video-miniaturen', 'video thumbnails'],
};
/* host-rows:end */

/** A host message in the language of the call (i18n::trf: {product} and the params filled in). */
function hostText(key, params = {}) {
  const row = HOST[key] || [key, key];
  const s = row[lang() === 'en' ? 1 : 0];
  const all = { product: HOST['update.product'][lang() === 'en' ? 1 : 0], ...params };
  return s.replace(/\{(\w+)\}/g, (m, k) => (all[k] == null ? m : String(all[k])));
}
/** The error object of the snapshot / an ApiError envelope (C2): code, rendered message, key, params, detail. */
const hostError = (code, key, params = {}, detail = null) => ({ code, key, params, detail });
const renderErr = (e) => (e ? { code: e.code, message: hostText(e.key, e.params), key: e.key, params: e.params || {}, detail: e.detail ?? null } : null);
const apiErr = (code, key, params = {}) => err(code, hostText(key, params), { key, params });

/* ------------------------------------------------------------------ package data (26.04.1 LTS, 25 Oct 2026) */

const OS_VERSION = '26.04.1 LTS';
const pkg = (name, from, to, row, info = 5, summary = '', extra = {}) => ({
  id: `${name};${to};amd64;ubuntu-resolute-${info === 8 ? 'security' : 'updates'}`,
  name, from, to, info, summary, row, phased: false, pup: null, held: false, ...extra,
});
const PACKAGES = [
  pkg('libssl3t64', '3.5.1-1ubuntu2', '3.5.1-1ubuntu2.1', 'security', 8, 'Secure Sockets Layer toolkit - shared libraries'),
  pkg('openssl', '3.5.1-1ubuntu2', '3.5.1-1ubuntu2.1', 'security', 8, 'Secure Sockets Layer toolkit - cryptographic utility'),
  pkg('libxml2-16', '2.14.5+dfsg-0ubuntu1', '2.14.5+dfsg-0ubuntu1.1', 'security', 8, 'GNOME XML library'),
  pkg('libtiff6', '4.7.0-3ubuntu2', '4.7.0-3ubuntu2.1', 'security', 8, 'Tag Image File Format (TIFF) library'),
  pkg('libexpat1', '2.7.1-2', '2.7.1-2ubuntu0.1', 'security', 8, 'XML parsing C library - runtime library'),
  pkg('libpng16-16t64', '1.6.50-1', '1.6.50-1ubuntu0.1', 'security', 8, 'PNG library - runtime (version 1.6)'),
  pkg('curl', '8.14.1-2ubuntu1', '8.14.1-2ubuntu1.2', 'security', 8, 'command line tool for transferring data with URL syntax'),
  pkg('libcurl4t64', '8.14.1-2ubuntu1', '8.14.1-2ubuntu1.2', 'security', 8, 'easy-to-use client-side URL transfer library (OpenSSL flavour)'),
  pkg('libcurl3t64-gnutls', '8.14.1-2ubuntu1', '8.14.1-2ubuntu1.2', 'security', 8, 'easy-to-use client-side URL transfer library (GnuTLS flavour)'),
  pkg('libnss3', '2:3.114-1', '2:3.114-1ubuntu0.1', 'security', 8, 'Network Security Service libraries'),
  pkg('python3.14', '3.14.0-2', '3.14.0-2ubuntu0.1', 'security', 8, 'Interactive high-level object-oriented language (version 3.14)'),
  pkg('libpython3.14-stdlib', '3.14.0-2', '3.14.0-2ubuntu0.1', 'security', 8, 'Interactive high-level object-oriented language (standard library, version 3.14)'),
  pkg('libwebkit2gtk-4.1-0', '2.52.6-0ubuntu0.26.04.1', '2.52.7-0ubuntu0.26.04.1', 'webkit', 8, 'Web content engine library for GTK'),
  pkg('libjavascriptcoregtk-4.1-0', '2.52.6-0ubuntu0.26.04.1', '2.52.7-0ubuntu0.26.04.1', 'webkit', 8, 'JavaScript engine library from WebKitGTK'),
  pkg('gir1.2-webkit2-4.1', '2.52.6-0ubuntu0.26.04.1', '2.52.7-0ubuntu0.26.04.1', 'webkit', 8, 'JavaScript engine library from WebKitGTK - GObject introspection data'),
  pkg('gnome-shell', '50.1-0ubuntu1.5', '50.2-0ubuntu1', 'gnome', 6, 'graphical shell for the GNOME desktop'),
  pkg('gnome-shell-common', '50.1-0ubuntu1.5', '50.2-0ubuntu1', 'gnome', 6, 'common files for the GNOME graphical shell'),
  pkg('libmutter-18-0', '50.1-0ubuntu2.5', '50.2-0ubuntu1', 'gnome', 6, 'window manager library from the Mutter window manager'),
  pkg('mutter-common', '50.1-0ubuntu2.5', '50.2-0ubuntu1', 'gnome', 6, 'shared files for the Mutter window manager'),
  pkg('libgtk-3-0t64', '3.24.50-1ubuntu1', '3.24.50-1ubuntu1.1', 'gtk', 6, 'GTK graphical user interface library'),
  pkg('gir1.2-gtk-3.0', '3.24.50-1ubuntu1', '3.24.50-1ubuntu1.1', 'gtk', 6, 'GTK graphical user interface library -- gir bindings'),
  pkg('libsoup-3.0-0', '3.6.6-1ubuntu0.2', '3.6.6-1ubuntu0.3', 'soup', 8, 'HTTP library implementation in C -- Shared library'),
  pkg('libsoup-3.0-common', '3.6.6-1ubuntu0.2', '3.6.6-1ubuntu0.3', 'soup', 8, 'HTTP library implementation in C -- Common files'),
  pkg('libglib2.0-0t64', '2.86.1-1', '2.86.1-1ubuntu0.1', 'glib', 5, 'GLib library of C routines'),
  pkg('libglib2.0-bin', '2.86.1-1', '2.86.1-1ubuntu0.1', 'glib', 5, 'Programs for the GLib library'),
  pkg('libglib2.0-data', '2.86.1-1', '2.86.1-1ubuntu0.1', 'glib', 5, 'Common files for GLib library'),
  pkg('linux-image-7.0.0-43-generic', '-', '7.0.0-43.43', 'kernel', 8, 'Signed kernel image generic'),
  pkg('linux-modules-7.0.0-43-generic', '-', '7.0.0-43.43', 'kernel', 8, 'Linux kernel extra modules for version 7.0.0'),
  pkg('linux-generic', '7.0.0-42.42', '7.0.0-43.43', 'kernel', 8, 'Complete Generic Linux kernel and headers'),
  pkg('tzdata', '2026a-1ubuntu1', '2026b-0ubuntu0.26.04', 'other', 5, 'time zone and daylight-saving time data'),
  pkg('ubuntu-drivers-common', '1:0.10.4', '1:0.10.4.1', 'other', 6, 'Detect and install additional Ubuntu driver packages'),
  pkg('fwupd', '2.0.16-1', '2.0.16-1ubuntu0.1', 'other', 6, 'Firmware update daemon'),
  pkg('ubuntu-pro-client', '37', '37.1', 'other', 5, 'Management tools for Ubuntu Pro'),
  pkg('libgl1-mesa-dri', '26.0.2-1ubuntu1', '26.0.3-0ubuntu0.26.04.1', 'other', 5, 'free implementation of the OpenGL API -- DRI modules', { phased: true, pup: 30 }),
  pkg('mesa-vulkan-drivers', '26.0.2-1ubuntu1', '26.0.3-0ubuntu0.26.04.1', 'other', 5, 'Mesa Vulkan graphics drivers', { phased: true, pup: 30 }),
  pkg('nvidia-driver-580', '580.95.05-0ubuntu1', '580.105.08-0ubuntu0.26.04.1', 'other', 9, 'NVIDIA driver metapackage', { held: true }),
];
const ROW_ORDER = ['security', 'gnome', 'webkit', 'gtk', 'soup', 'glib', 'kernel', 'other'];
const ROW_VERSION = { security: OS_VERSION, gnome: '50.2', webkit: '2.52.7', gtk: '3.24.50', soup: '3.6.6', glib: '2.86.1', kernel: '7.0.0-43', other: null };
const SIZES = { security: 18874368, gnome: 9961472, webkit: 41943040, gtk: 4194304, soup: 1048576, glib: 2097152, kernel: 132120576, other: 6291456 };

/** rows of §4.3 from the non-phased, non-held packages of the given kinds; sizes only when `sized` (rowCaption) */
function rowsOf(kinds, { sized = false, state = 'pendingDownload' } = {}) {
  return ROW_ORDER.filter((k) => kinds.includes(k)).map((kind) => {
    const pk = PACKAGES.filter((p) => p.row === kind && !p.phased && !p.held);
    return {
      id: kind, kind, version: ROW_VERSION[kind] ?? null, count: pk.length, size: sized ? SIZES[kind] : null,
      security: pk.some((p) => p.info === 8), state, pct: null, error: null,
    };
  });
}
const packagesOf = (rows) => PACKAGES.filter((p) => rows.some((r) => r.id === p.row) || p.phased || p.held);
const totalsOf = (rows, { phased = 0, held = 0, size = null } = {}) => ({
  count: rows.reduce((a, r) => a + r.count, 0),
  security: rows.filter((r) => r.security).reduce((a, r) => a + r.count, 0),
  phased, held, downloadSize: size,
});

/* ------------------------------------------------------------------ scenarios */

const H = 3600e3;
/* a release-upgrades.d file that sets Prompt (M03): Ubuntu reads it after the main file, so it decides */
const PROMPT_OVERRIDE = '/etc/update-manager/release-upgrades.d/50-organisation.cfg';

function base(now) {
  return {
    backend: 'packagekit', phase: 'idle', rev: 17, checkedAt: now - 2 * H, refreshing: false,
    owner: { pid: 4242, mirror: false },
    rows: [], packages: [], totals: totalsOf([]),
    job: null,
    restart: { reboot: false, rebootPkgs: [], gnome: false, shell: false, shellParts: [], since: null },
    error: null, warnings: [],
    network: { state: 'full', online: true, metered: false },
    release: {
      state: 'none', ubuntuPromptHeld: false, startRc: null, offered: false,
      released: { version: '26.10', codename: 'stonking' }, version: null, codename: null, eol: null,
      supportMonths: null, notesUrl: null, gnomeMajor: null, reasons: [], prompt: 'normal',
      promptFile: '/etc/update-manager/release-upgrades', override: null, helper: { prompt: false, attach: false },
      checkedAt: now - 2 * H, currentLts: true,
      // GW6 (compat.rs fill; GW7's startError/logPath/heldNames: notes/claude/wu/gw6.md "Contract with GW7")
      verdict: 'none', allowAnyway: false, compat: { running: false, needed: false, checkedAt: null }, upgraderNl: false,
      startError: null, logPath: null, heldNames: [],
    },
    dryRun: false,
  };
}

const MIXED = ['security', 'webkit', 'other'];
const job = (now, extra = {}) => ({
  jobId: '5f0c2a9e8b7d4c3fa1e6d2b9c8a7f6e5', role: 'update', status: 8, pct: null, speed: null, remaining: null,
  downloadLeft: null, allowCancel: true, committing: false, startedAt: now - 90e3, reattached: false,
  foreign: false, foreignOwn: false, lockHolder: null, simulating: false, confirm: null, ...extra,
});

function available(s, kinds = MIXED, opts = {}) {
  s.phase = 'available';
  s.rows = rowsOf(kinds, opts);
  s.totals = totalsOf(s.rows, { phased: 2, held: 0, size: opts.size ?? null });
  return s;
}

/** Per-row state/pct overrides: {id: [state, pct]} */
function rowStates(s, map) {
  for (const r of s.rows) if (map[r.id]) [r.state, r.pct] = map[r.id];
  return s;
}

/** GW6: Ubuntu 26.10 offered (meta-release, distro-info) with the card's state; `extra` patches the release object. */
function offer(s, now, state, extra = {}) {
  s.release = {
    ...s.release, offered: true, version: '26.10', codename: 'stonking', name: 'Stonking Stingray', eol: '2027-07-15',
    supportMonths: 9, source: 'cache',
    notesUrl: 'http://archive.ubuntu.com/ubuntu/dists/stonking/main/dist-upgrader-all/current/ReleaseAnnouncement.html',
    upgradeBroken: null, released: { version: '26.10', codename: 'stonking' }, checkedAt: now - 3 * H,
    state, verdict: state, compat: { running: false, needed: false, checkedAt: now - 3 * H }, ...extra,
  };
  return s;
}

const SCENARIOS = {
  H01: (s) => s,
  H02: (s, now) => Object.assign(s, { phase: 'checking', refreshing: true, job: job(now, { role: 'refresh', status: 7, allowCancel: false }) }),
  H03: (s) => available(s),
  H04: (s, now) => Object.assign(available(s), { phase: 'downloading', job: job(now, { status: 13, simulating: true }) }),
  H05: (s, now) => rowStates(Object.assign(available(s, MIXED, { size: 40265318 }), {
    phase: 'downloading',
    job: job(now, { status: 8, pct: 45, speed: 1200000, remaining: 140, downloadLeft: 22020096 }),
  }), { security: ['downloading', 45] }),
  H05b: (s, now) => Object.assign(SCENARIOS.H05(s, now), { job: { ...s.job, remaining: null } }),
  H05c: (s, now) => { SCENARIOS.H05(s, now); s.totals.downloadSize = null; return s; },
  H06: (s, now) => rowStates(Object.assign(available(s, MIXED, { size: 40265318 }), {
    phase: 'installing',
    job: job(now, { status: 10, pct: 30, allowCancel: false, committing: true, downloadLeft: 0 }),
  }), { security: ['installing', 30], webkit: ['pendingInstall', null], other: ['pendingInstall', null] }),
  H07: (s, now) => Object.assign(available(s), { phase: 'waiting', job: job(now, { status: 30, lockHolder: 'unattended' }) }),
  H07b: (s, now) => Object.assign(available(s), { phase: 'waiting', job: job(now, { status: 30, lockHolder: null }) }),
  H08: (s, now) => Object.assign(available(s), { phase: 'waiting', job: job(now, { status: 31, allowCancel: false }) }),
  H09: (s, now) => Object.assign(available(s), { phase: 'foreign', job: job(now, { jobId: null, role: null, status: 9, foreign: true, allowCancel: false }) }),
  H10: (s, now) => Object.assign(s, {
    phase: 'restartRequired', checkedAt: now - 7 * 60e3,
    restart: { reboot: true, rebootPkgs: ['gnome-shell'], gnome: true, shell: true, shellParts: ['soup'], since: now - 7.7 * H },
  }),
  H10b: (s, now) => { SCENARIOS.H10(s, now); s.restart.rebootPkgs = []; return s; },
  H10c: (s, now) => {
    SCENARIOS.H10(s, now);
    s.restart.rebootPkgs = ['linux-image-7.0.0-43-generic', 'libc6', 'gnome-shell', 'dbus', 'systemd'];
    return s;
  },
  H10d: (s, now) => SCENARIOS.H10(s, now),     // the surface clicks "Nu opnieuw opstarten"
  H10e: (s, now) => SCENARIOS.H10(s, now),     // … and then the confirm's "Opnieuw opstarten" (system.power.action)
  H11: (s, now) => Object.assign(available(s, ['gnome', 'other']), {
    phase: 'restartRequired', checkedAt: now - 20 * 60e3,
    restart: { reboot: true, rebootPkgs: ['linux-image-7.0.0-43-generic', 'linux-base', 'libc6'], gnome: false, shell: false, shellParts: [], since: now - 30 * 60e3 },
  }),
  H12: (s) => Object.assign(available(s), {
    phase: 'error',
    error: hostError('network', 'update.err.network', {},
      'E: Failed to fetch http://archive.ubuntu.com/ubuntu/dists/resolute-updates/InRelease  Temporary failure resolving \'archive.ubuntu.com\'\nE: Some index files failed to download. They have been ignored, or old ones used instead.'),
  }),
  H13: (s) => Object.assign(s, { backend: 'apt', phase: 'unsupported', error: hostError('no_packagekit', 'update.err.noPackageKit') }),
  H14: (s, now) => Object.assign(s, { phase: 'offline', checkedAt: now - 26 * H, network: { state: 'none', online: false, metered: false } }),
  H14b: (s) => Object.assign(s, { phase: 'offline', checkedAt: null, network: { state: 'none', online: false, metered: false } }),
  H15: (s) => Object.assign(s, { phase: 'repairRequired', error: hostError('repair', 'update.err.repair') }),
  H16: (s) => Object.assign(s, { phase: 'idle', totals: totalsOf([], { phased: 2, held: 1 }) }),
  H17: (s) => Object.assign(available(s), { network: { state: 'full', online: true, metered: true } }),
  H18: (s, now) => Object.assign(available(s, MIXED, { size: 40265318 }), {
    phase: 'downloading', network: { state: 'full', online: true, metered: true },
    job: job(now, { status: 13, simulating: true, confirm: { kind: 'metered', size: 40265318 } }),
  }),
  H19: (s) => Object.assign(available(s), { warnings: [hostError('source', 'update.warn.source', { source: 'dl.google.com' })] }),
  H20: (s) => Object.assign(s, { restart: { reboot: false, rebootPkgs: [], gnome: false, shell: true, shellParts: ['soup'], since: s.checkedAt } }),
  H20b: (s) => Object.assign(s, { restart: { reboot: false, rebootPkgs: [], gnome: false, shell: true, shellParts: ['webkit', 'jsc', 'gtk', 'gdk', 'soup', 'glib', 'gio', 'gobject', 'pixbuf', 'cairo', 'archive', 'self'], since: s.checkedAt } }),
  H21: (s, now) => Object.assign(available(s, MIXED, { size: 40265318 }), {
    phase: 'downloading',
    job: job(now, { status: 13, simulating: true, confirm: { kind: 'removals', removals: ['libmozjs-128-0', 'python3-distutils-extra'] } }),
  }),
  H22: (s) => Object.assign(available(s), { error: hostError('cancelled', 'update.err.cancelled') }),
  H23: (s, now) => Object.assign(SCENARIOS.H06(s, now), { error: hostError('cannot_cancel', 'update.err.cannotCancel') }),
  H24: (s) => Object.assign(available(s), { error: hostError('not_authorized', 'update.err.notAuthorized') }),
  H25: (s, now) => Object.assign(s, {
    phase: 'releaseRunning',
    release: { ...s.release, state: 'running', offered: true, version: '26.10', codename: 'stonking', checkedAt: now - 40 * 60e3 },
  }),
  H25b: (s) => Object.assign(s, { phase: 'releaseRunning', release: { ...s.release, state: 'running', released: null, version: null } }),
  H26: (s, now) => Object.assign(available(s), { phase: 'foreign', job: job(now, { jobId: null, role: null, status: 9, foreign: true, foreignOwn: true, allowCancel: false }) }),
  H27: (s, now) => Object.assign(SCENARIOS.H10(s, now), {
    restart: { reboot: true, rebootPkgs: ['gnome-shell'], gnome: true, shell: true, shellParts: ['webkit', 'jsc', 'soup'], since: now - 7.7 * H },
  }),
  H28: null,          // no update.* service: update.status is an unknown method (the page's fallback)
  R01: (s, now) => rowStates(Object.assign(available(s, ['security', 'webkit', 'kernel', 'other'], { sized: true, size: 198967296 }), {
    phase: 'downloading',
    job: job(now, { status: 8, pct: 52, speed: 3400000, remaining: 28, downloadLeft: 95420416 }),
  }), { security: ['pendingInstall', null], webkit: ['downloading', 45] }),
  R02: (s, now) => rowStates(Object.assign(available(s, ['security', 'webkit', 'kernel', 'other']), {
    phase: 'installing',
    job: job(now, { status: 10, pct: 70, allowCancel: false, committing: true, downloadLeft: 0 }),
  }), { security: ['installed', null], kernel: ['restart', null], webkit: ['installing', 60], other: ['pendingInstall', null] }),
  R03: (s) => rowStates(Object.assign(available(s), {
    phase: 'error',
    error: hostError('dpkg', 'update.err.dpkg', {},
      'dpkg: error processing package libwebkit2gtk-4.1-0:amd64 (--configure):\n installed libwebkit2gtk-4.1-0:amd64 package post-installation script subprocess returned error exit status 1\nErrors were encountered while processing:\n libwebkit2gtk-4.1-0:amd64'),
  }), { security: ['installed', null], webkit: ['failed', 80], other: ['installed', null] }),
  R04: (s) => available(s),
  R05: (s) => available(s),
  R05b: (s) => available(s),       // the surface opens "Wat is er nieuw" of the Overige updates row (no details)
  R06: (s) => available(s, ['gtk', 'soup', 'glib']),
  // the Prompt row (GW5; ids outside the [HR] pattern of tests/i18n/test_update_mock_rows.py: their surfaces are
  // settings-update-prompt.<id>)
  M01: (s) => s,
  M01b: (s) => Object.assign(s, { release: { ...s.release, prompt: 'never' } }),
  M02: (s) => Object.assign(s, { release: { ...s.release, prompt: 'lts', currentLts: false, released: null } }),
  M03: (s) => Object.assign(s, {
    release: { ...s.release, prompt: 'lts', released: null, promptFile: PROMPT_OVERRIDE, override: PROMPT_OVERRIDE },
  }),
  M04: (s) => Object.assign(s, { release: { ...s.release, helper: { prompt: false, attach: false } } }),
  M04b: (s) => Object.assign(s, { release: { ...s.release, helper: { prompt: true, attach: true } } }),
  // the feature-update card (GW6; surfaces settings-update-card.<id>, outside the [HR] pattern of
  // tests/i18n/test_update_mock_rows.py)
  C01: (s) => s,
  C02: (s, now) => offer(s, now, 'checking', { compat: { running: true, needed: false, checkedAt: null } }),
  C03: (s, now) => offer(s, now, 'ok'),
  C03b: (s, now) => offer(s, now, 'ok', {
    version: '26.04.1 LTS', codename: 'resolute', name: 'Resolute Raccoon', eol: '2031-05-29', supportMonths: 61,
    notesUrl: 'http://archive.ubuntu.com/ubuntu/dists/resolute-updates/main/dist-upgrader-all/current/ReleaseAnnouncement.html',
    prompt: 'normal', currentLts: false, released: { version: '26.04', codename: 'resolute' },
  }),
  C04: (s, now) => offer(s, now, 'warn', { reasons: [
    hostError('degraded', 'update.compat.degraded', { part: 'update.part.ffmpeg', feature: 'update.feature.ffmpeg', version: '26.10' }),
    hostError('thirdParty', 'update.compat.thirdParty', { n: 2, name: 'dl.google.com' }),
  ] }),
  C05: (s, now) => offer(s, now, 'hold', { gnomeMajor: '51' }),
  C05b: (s, now) => offer(s, now, 'hold'),
  C05c: (s, now) => offer(s, now, 'hold', { allowAnyway: true }),
  C05d: (s, now) => offer(s, now, 'hold', { allowAnyway: true }),     // the surface clicks "Toch upgraden…"
  C06: (s, now) => offer(s, now, 'blocked', { reasons: [
    hostError('removed', 'update.compat.removed', { part: 'update.part.archive', version: '26.10' }),
    hostError('missing', 'update.compat.missing', { part: 'update.part.jsc', version: '26.10' }),
    hostError('missing', 'update.compat.missing', { part: 'update.part.webkit', version: '26.10' }),
  ] }),
  C07: (s, now) => offer(available(s), now, 'updatesFirst', { verdict: 'ok' }),
  C07b: (s, now) => offer(available(s, ['security']), now, 'updatesFirst', { verdict: 'ok', heldNames: ['nvidia-driver-580'] }),
  C08: (s, now) => offer(Object.assign(s, {
    phase: 'restartRequired', checkedAt: now - 20 * 60e3,
    restart: { reboot: true, rebootPkgs: ['linux-image-7.0.0-43-generic', 'linux-base'], gnome: false, shell: false, shellParts: [], since: now - 30 * 60e3 },
  }), now, 'rebootFirst', { verdict: 'ok' }),
  C09: (s, now) => offer(Object.assign(s, { network: { state: 'full', online: true, metered: true } }), now, 'ok'),
  C10: (s, now) => offer(rowStates(Object.assign(available(s, MIXED, { size: 40265318 }), {
    phase: 'downloading',
    job: job(now, { status: 8, pct: 45, speed: 1200000, remaining: 140, downloadLeft: 22020096 }),
  }), { security: ['downloading', 45] }), now, 'running', { verdict: 'ok' }),
  C11: (s, now) => offer(Object.assign(s, { phase: 'releaseRunning' }), now, 'waiting', { verdict: 'ok' }),
  C12: (s, now) => offer(s, now, 'ubuntuHold', { upgradeBroken: 'Upgrades to Ubuntu 26.10 are on hold while a bug is fixed.' }),
  C13: (s, now) => offer(s, now, 'unknown', { reasons: [hostError('apt', 'update.compat.apt', { version: '26.10' })] }),
  C13b: (s, now) => offer(Object.assign(s, { phase: 'offline', network: { state: 'none', online: false, metered: false } }), now, 'unknown', {
    reasons: [hostError('offline', 'update.compat.offline')],
  }),
  C13c: (s, now) => offer(Object.assign(s, { network: { state: 'full', online: true, metered: true } }), now, 'unknown', {
    reasons: [hostError('metered', 'update.compat.metered')],
  }),
  C14: (s, now) => offer(s, now, 'startFailed', { verdict: 'ok', startRc: 126, startError: hostError('auth_cancelled', 'update.release.err.noPassword') }),
  C14b: (s, now) => SCENARIOS.C14(s, now),     // the surface clicks Opnieuw proberen
  C15: (s, now) => offer(s, now, 'ended', { verdict: 'ok', startRc: 1, logPath: '/var/log/dist-upgrade/main.log' }),
  C16: (s, now) => offer(s, now, 'hold', { gnomeMajor: '51', ubuntuPromptHeld: true }),
  C16b: (s, now) => offer(s, now, 'hold', { gnomeMajor: '51' }),     // the surface clicks "Ook de melding van Ubuntu uitstellen"
};

/* update.details (GetUpdateDetail): the apt backend's changelog text is Ubuntu's own (English, never translated) */
const DETAILS = {
  webkit: {
    updateText: 'New upstream stable release 2.52.7: security fixes and stability improvements.',
    changelog: 'webkit2gtk (2.52.7-0ubuntu0.26.04.1) resolute-security; urgency=medium\n\n  * New upstream release.\n  * Fixes CVE-2026-43101, CVE-2026-43117, CVE-2026-43122.\n\n -- Ubuntu Security Team  Tue, 21 Oct 2026 14:02:11 +0200',
    cves: ['https://ubuntu.com/security/CVE-2026-43101', 'https://ubuntu.com/security/CVE-2026-43117', 'https://ubuntu.com/security/CVE-2026-43122'],
    bugs: ['https://bugs.launchpad.net/bugs/2128841'],
  },
  security: {
    updateText: 'Security update.',
    changelog: 'openssl (3.5.1-1ubuntu2.1) resolute-security; urgency=medium\n\n  * SECURITY UPDATE: timing side channel in RSA decryption.',
    cves: ['https://ubuntu.com/security/CVE-2026-40211'],
    bugs: [],
  },
};

/* ------------------------------------------------------------------ state + methods */

function U(m) {
  if (!m.state.update) {
    const id = m.params.get('updateScenario') || 'H03';
    m.state.update = { scenario: id, snap: build(id), timers: [], seq: 0 };
  }
  return m.state.update;
}

function build(id) {
  const now = Date.now();
  const s = base(now);
  const f = Object.prototype.hasOwnProperty.call(SCENARIOS, id) ? SCENARIOS[id] : SCENARIOS.H03;
  if (!f) return null;
  f(s, now);
  s.packages = packagesOf(s.rows);
  return s;
}

/** The snapshot as update.status returns it: packages only with {packages:true}; messages in the call's language. */
function view(s, withPackages) {
  const out = clone({ ...s, error: null, warnings: [], packages: withPackages ? s.packages : undefined });
  if (!withPackages) delete out.packages;
  out.error = renderErr(s.error);
  out.warnings = (s.warnings || []).map(renderErr);
  // GW6: the card's host texts (compat.rs render: part/feature keys as texts, thirdParty a plural)
  if (out.release) {
    out.release.reasons = (s.release.reasons || []).map(renderReason);
    out.release.startError = renderErr(s.release.startError);
  }
  return out;
}

/** compat.rs render(): {code, key, params, message}; part and feature params that are update.part / update.feature keys become their texts */
function renderReason(r) {
  const params = { ...(r.params || {}) };
  for (const f of ['part', 'feature']) if (typeof params[f] === 'string' && HOST[params[f]]) params[f] = hostText(params[f]);
  const key = r.code === 'thirdParty' ? `${r.key}.${params.n === 1 ? 'one' : 'other'}` : r.key;
  return { code: r.code, key: r.key, params: r.params || {}, message: hostText(key, params) };
}

function changed(m, u) {
  const s = u.snap;
  s.rev += 1;
  m.emit('update.changed', {
    rev: s.rev, phase: s.phase, totals: clone(s.totals), restart: clone(s.restart), job: clone(s.job),
    error: renderErr(s.error), release: { state: s.release.state, version: s.release.version },
  });
}

function progress(m, s) {
  const j = s.job;
  if (!j) return;
  m.emit('update.progress', {
    jobId: j.jobId, status: j.status, pct: j.pct, speed: j.speed, remaining: j.remaining, downloadLeft: j.downloadLeft,
    allowCancel: j.allowCancel, rows: Object.fromEntries(s.rows.map((r) => [r.id, { state: r.state, pct: r.pct }])),
  });
}

function stopTimers(u) { for (const t of u.timers) clearTimeout(t); u.timers = []; }
function later(u, ms, fn) { u.timers.push(setTimeout(fn, ms)); }

const BUSY = new Set(['checking', 'downloading', 'installing', 'waiting']);

/** Our own install job: simulate → (confirm) → download → install → outcome (restartRequired with a kernel row). */
function runJob(m, u, ids) {
  const s = u.snap;
  const rows = s.rows.filter((r) => ids.includes(r.id));
  const size = rows.reduce((a, r) => a + (SIZES[r.id] || 4 << 20), 0);
  s.error = null;
  s.phase = 'downloading';
  s.job = job(Date.now(), { jobId: (++u.seq).toString(16).padStart(32, 'a'), status: 13, simulating: true, startedAt: Date.now() });
  s.totals.downloadSize = size;
  for (const r of rows) { r.state = 'pendingDownload'; r.pct = null; r.error = null; }
  changed(m, u);
  later(u, 700, () => {
    if (s.network.metered && !(m.settings()?.update?.metered)) {
      s.job.confirm = { kind: 'metered', size };
      changed(m, u);
      return;
    }
    download(m, u, rows, size);
  });
}

function download(m, u, rows, size) {
  const s = u.snap;
  Object.assign(s.job, { simulating: false, confirm: null, status: 8, pct: 0, downloadLeft: size, speed: 2400000, remaining: Math.round(size / 2400000) });
  changed(m, u);
  let done = 0;
  const step = () => {
    done = Math.min(size, done + Math.max(size / 16, 1));
    const pct = Math.round((done / size) * 100);
    Object.assign(s.job, { pct, downloadLeft: size - done, remaining: Math.max(0, Math.round((size - done) / 2400000)) });
    let acc = 0;
    for (const r of rows) {
      const rs = SIZES[r.id] || 4 << 20;
      const p = Math.max(0, Math.min(1, (done - acc) / rs));
      r.state = p >= 1 ? 'pendingInstall' : p > 0 ? 'downloading' : 'pendingDownload';
      r.pct = r.state === 'downloading' ? Math.round(p * 100) : null;
      acc += rs;
    }
    progress(m, s);
    if (done < size) later(u, 250, step);
    else later(u, 250, () => install(m, u, rows));
  };
  later(u, 250, step);
}

function install(m, u, rows) {
  const s = u.snap;
  Object.assign(s.job, { status: 10, pct: 0, allowCancel: false, committing: true, downloadLeft: 0, remaining: null, speed: null });
  s.phase = 'installing';
  changed(m, u);
  let i = 0, pct = 0;
  const step = () => {
    const r = rows[i];
    if (!r) { finish(m, u, rows); return; }
    pct = Math.min(100, pct + 25);
    r.state = 'installing'; r.pct = pct;
    s.job.pct = Math.round(((i + pct / 100) / rows.length) * 100);
    if (pct >= 100) { r.state = r.kind === 'kernel' ? 'restart' : 'installed'; r.pct = null; i += 1; pct = 0; }
    progress(m, s);
    later(u, 250, step);
  };
  later(u, 250, step);
}

function finish(m, u, rows) {
  const s = u.snap;
  const kernel = rows.some((r) => r.kind === 'kernel');
  const gnome = rows.some((r) => r.kind === 'gnome');
  s.job = null;
  s.rows = s.rows.filter((r) => !rows.includes(r));
  s.totals = totalsOf(s.rows, { phased: s.totals.phased, held: s.totals.held });
  s.packages = packagesOf(s.rows);
  s.checkedAt = Date.now();
  if (kernel || gnome) {
    s.restart = { ...s.restart, reboot: true, rebootPkgs: [...(kernel ? ['linux-image-7.0.0-43-generic'] : []), ...(gnome ? ['gnome-shell'] : [])], gnome, since: Date.now() };
  }
  s.phase = s.restart.reboot ? 'restartRequired' : s.rows.length ? 'available' : 'idle';
  changed(m, u);
}

/** The scenario's state; without a snapshot (H28: no update.* service) every update.* method is unknown */
function live(m, method) {
  const u = U(m);
  if (!u.snap) throw err('unknown_method', `Onbekende functie: ${method}`);
  return u;
}

/** The update.* write methods refuse while Ubuntu's release upgrader runs (§4.3) */
function guard(s) {
  if (s.phase === 'releaseRunning') throw apiErr('update_busy', 'update.guard.upgrade');
}

/* ------------------------------------------------------------------ GW4: Updategeschiedenis and Geavanceerde opties */
// update.history, update.options and update.setAutoOpen as host/src/services/update/{history,options}.rs answer them.
// Their states come from ?updateScenario= as well (tests/i18n/surfaces.json settings-update-history.<id> and
// settings-update-advanced.<id>; any other id gives the default, S01 or S03):
//   S01 history with the three groups and every source (the registry page's default) · S01b older entries than 90
//   days: "Meer laden" · S01c S01b after "Meer laden" (the surface clicks it: the 90 days before the oldest entry) ·
//   S01d failed entries (an update and a release upgrade) · S02 no history yet · S02b a host without update.history ·
//   S03 advanced options (security updates on, the Snap Store times, auto-open every 7 days; the default) · S03b the
//   off states (security updates off, auto-open off; the surface seeds notifyRestart off and metered on) · S03c no
//   update-notifier schema (the auto-open row is hidden) · S03d security updates and snaps unknown · S03e auto-open
//   every day, snapd has never refreshed · S03f S03 with the auto-open toggle clicked (live: off, the first sentence
//   goes) · S03g auto-open at once (0 days) · S03h a host without update.options · S03i update.options has not
//   answered yet (its two programs are slow: the page is there, its three rows say "Even geduld…") · S03j snapd
//   holds the refreshes for 3 days (snaps.held; next = the hold's end; R3-UI) · S03k held with no end (next null).

const DAY = 86400e3;
const OFF_DAYS = 36500;
/* [days ago, group, name, version, ok, source]: newest first; one apt run per days-ago value */
const HISTORY_RECENT = [
  [0.27, 'updates', 'libssl3t64', '3.5.1-1ubuntu2.1', true, 'ubuntu'],
  [0.27, 'updates', 'openssl', '3.5.1-1ubuntu2.1', true, 'ubuntu'],
  [1.1, 'updates', 'libwebkit2gtk-4.1-0', '2.52.7-0ubuntu0.26.04.1', true, 'us'],
  [1.1, 'updates', 'libjavascriptcoregtk-4.1-0', '2.52.7-0ubuntu0.26.04.1', true, 'us'],
  [3.2, 'updates', 'fwupd', '2.0.16-1ubuntu0.1', true, 'app'],
  [5.3, 'software', 'gnome-boxes', '49.1-1', true, 'app'],
  [6.8, 'updates', 'vim-common', '2:9.1.0967-1ubuntu4', true, 'terminal'],
  [6.8, 'software', 'vim', '2:9.1.0967-1ubuntu4', true, 'terminal'],
  [12.4, 'updates', 'tzdata', '2026b-0ubuntu0.26.04', true, 'ubuntu'],
  [23.1, 'updates', 'ubuntu-drivers-common', '1:0.10.4.1', true, 'app'],
  [40.6, 'updates', 'gnome-shell', '50.1-0ubuntu1.4', true, 'us'],
  [40.6, 'updates', 'gnome-shell-common', '50.1-0ubuntu1.4', true, 'us'],
  [40.6, 'updates', 'libmutter-18-0', '50.1-0ubuntu2.4', true, 'us'],
  [66.2, 'releases', 'Ubuntu', '26.04', true, null],
  [71.9, 'software', 'htop', '3.4.1-1', true, 'terminal'],
];
/* more than 90 days before the newest entry: page two */
const HISTORY_OLDER = [
  [95.3, 'updates', 'libexpat1', '2.7.1-1ubuntu0.1', true, 'ubuntu'],
  [118.7, 'software', 'gimp', '3.0.4-1', true, 'terminal'],
  [131.2, 'updates', 'libtiff6', '4.7.0-3ubuntu1.1', true, 'ubuntu'],
];
/* a failed dpkg run of unattended-upgrades and an earlier release upgrade that failed */
const HISTORY_FAILED = [
  [0.1, 'updates', 'libsoup-3.0-0', '3.6.6-1ubuntu0.2', false, 'ubuntu'],
  [80.9, 'releases', 'Ubuntu', '26.04', false, null],
];

/* history.rs: cursor = second · 2^20 + slot (unique, below 2^53); a `before` below 2^48 is a time in ms */
const SLOTS = 2 ** 20;
const CURSOR_MIN = 2 ** 48;
function historyOf(id, now) {
  const rows = id === 'S02' ? []
    : id === 'S01d' ? [...HISTORY_FAILED, ...HISTORY_RECENT]
      : id === 'S01b' || id === 'S01c' ? [...HISTORY_RECENT, ...HISTORY_OLDER] : HISTORY_RECENT;
  // dates in whole seconds; the slots order the entries of one second (newest first, as listed here)
  const used = new Map();
  return rows.map(([ago, group, name, version, ok, source]) => {
    const second = Math.round((now - ago * DAY) / 1000);
    const slot = (used.get(second) ?? SLOTS) - 1;
    used.set(second, slot);
    return { group, name, version, date: second * 1000, ok, source, cursor: second * SLOTS + slot };
  }).sort((a, b) => b.cursor - a.cursor);
}
/** history.rs page(): before `before`, at most `limit` and 90 days from the newest of them; more when older remain */
function historyPage(list, before, limit = 200) {
  const older = list.filter((e) => before == null || (before >= CURSOR_MIN ? e.cursor < before : e.date < before));
  if (!older.length) return { entries: [], more: false };
  const floor = older[0].date - 90 * DAY;
  const take = Math.min(older.filter((e) => e.date >= floor).length, limit);
  return { entries: older.slice(0, take), more: older.length > take };
}

const OPTIONS = {
  S03: (now) => ({ unattended: true, snaps: { last: now - 7 * H, next: now + 3 * H, held: false }, autoOpen: { supported: true, on: true, days: 7 } }),
  S03b: (now) => ({ unattended: false, snaps: { last: now - 7 * H, next: now + 3 * H, held: false }, autoOpen: { supported: true, on: false, days: OFF_DAYS } }),
  S03c: (now) => ({ unattended: true, snaps: { last: now - 7 * H, next: now + 3 * H, held: false }, autoOpen: { supported: false, on: false, days: null } }),
  S03d: () => ({ unattended: null, snaps: null, autoOpen: { supported: true, on: true, days: 7 } }),
  S03e: (now) => ({ unattended: true, snaps: { last: null, next: now + 50 * 60e3, held: false }, autoOpen: { supported: true, on: true, days: 1 } }),
  S03f: (now) => OPTIONS.S03(now),
  S03g: (now) => ({ ...OPTIONS.S03(now), autoOpen: { supported: true, on: true, days: 0 } }),
  // options.rs: while held, `next` is when the hold ends (snap refresh --hold=72h), null for --hold (forever)
  S03j: (now) => ({ ...OPTIONS.S03(now), snaps: { last: now - 7 * H, next: now + 72 * H, held: true } }),
  S03k: (now) => ({ ...OPTIONS.S03(now), snaps: { last: now - 7 * H, next: null, held: true } }),
};
/** update.options' state of this page: the scenario's, changed by update.setAutoOpen (previous = autoOpenPrevious) */
function opts(m) {
  if (!m.state.updateOptions) {
    const f = OPTIONS[m.params.get('updateScenario')] || OPTIONS.S03;
    m.state.updateOptions = { value: f(Date.now()), previous: null };
  }
  return m.state.updateOptions;
}
/** S02b / S03h: an older host without these methods (the real bridge answers unknown_method) */
function absent(m, id, method) {
  if (m.params.get('updateScenario') === id) throw err('unknown_method', method);
}

export default {
  methods: {
    'update.status': async ({ packages } = {}, m) => view(live(m, 'update.status').snap, !!packages),
    'update.check': async (a, m) => {
      const u = live(m, 'update.check');
      const s = u.snap;
      guard(s);
      if (BUSY.has(s.phase) && s.phase !== 'checking') throw apiErr('busy', 'update.err.busy');
      if (!s.network.online) {
        Object.assign(s, { phase: 'offline' });
        changed(m, u);
        return { started: false };
      }
      stopTimers(u);
      Object.assign(s, { phase: 'checking', refreshing: true, error: null, warnings: [] });
      changed(m, u);
      later(u, 1500, () => {
        const fresh = available(base(Date.now()));
        Object.assign(s, {
          refreshing: false, checkedAt: Date.now(), rows: fresh.rows, totals: fresh.totals,
          phase: s.restart.reboot ? 'restartRequired' : 'available',
        });
        s.packages = packagesOf(s.rows);
        changed(m, u);
      });
      return { started: true };
    },
    'update.install': async ({ rows } = {}, m) => {
      const u = live(m, 'update.install');
      const s = u.snap;
      guard(s);
      // WU-FIX (GW2b review n6): a running check refuses with its own text, as pk.rs install() does
      if (s.phase === 'checking') throw apiErr('busy', 'update.err.checking');
      if (BUSY.has(s.phase) || s.phase === 'foreign') throw apiErr('busy', 'update.err.busy');
      const ids = Array.isArray(rows) && rows.length ? rows : s.rows.map((r) => r.id);
      if (!ids.length || ids.some((id) => !s.rows.some((r) => r.id === id))) throw apiErr('stale', 'update.err.stale');
      stopTimers(u);
      runJob(m, u, ids);
      return { jobId: s.job.jobId };
    },
    'update.confirm': async ({ jobId, accept } = {}, m) => {
      const u = live(m, 'update.confirm');
      const s = u.snap;
      if (!s.job || s.job.jobId !== jobId || !s.job.confirm) throw apiErr('stale', 'update.err.stale');
      if (typeof accept !== 'boolean') throw apiErr('bad_args', 'update.err.badArgs');
      const size = s.job.confirm.size ?? s.totals.downloadSize;
      if (!accept) {
        stopTimers(u);
        Object.assign(s, { job: null, phase: 'available', error: hostError('cancelled', 'update.err.cancelled') });
        for (const r of s.rows) { r.state = 'pendingDownload'; r.pct = null; }
        changed(m, u);
        return { ok: true };
      }
      download(m, u, s.rows, size || 40265318);
      return { ok: true };
    },
    'update.cancel': async ({ jobId } = {}, m) => {
      const u = live(m, 'update.cancel');
      const s = u.snap;
      if (!s.job || (jobId && s.job.jobId !== jobId)) throw apiErr('stale', 'update.err.stale');
      if (!s.job.allowCancel) throw apiErr('cannot_cancel', 'update.err.cannotCancel');
      stopTimers(u);
      Object.assign(s, { job: null, phase: 'available', error: hostError('cancelled', 'update.err.cancelled') });
      for (const r of s.rows) { r.state = 'pendingDownload'; r.pct = null; }
      changed(m, u);
      return { ok: true };
    },
    'update.repair': async (a, m) => {
      const u = live(m, 'update.repair');
      const s = u.snap;
      guard(s);
      if (s.phase !== 'repairRequired') throw apiErr('stale', 'update.err.stale');
      Object.assign(s, { phase: 'installing', error: null, job: job(Date.now(), { role: 'repair', status: 3, allowCancel: false, committing: true }) });
      changed(m, u);
      later(u, 1500, () => { Object.assign(s, { phase: 'idle', job: null, checkedAt: Date.now() }); changed(m, u); });
      return { jobId: s.job.jobId };
    },
    'update.details': async ({ ids } = {}, m) => {
      const s = live(m, 'update.details').snap;
      if (!Array.isArray(ids) || ids.length > 20) throw apiErr('bad_args', 'update.err.badArgs');
      return {
        details: ids.map((id) => {
          const p = s.packages.find((x) => x.id === id);
          const d = (p && p.name.match(/^(libwebkit2gtk|openssl)/) && DETAILS[p.row]) || null;
          return { id, updateText: d?.updateText || '', changelog: d?.changelog || '', cves: d?.cves || [], bugs: d?.bugs || [], restart: 1, issued: null, updated: null };
        }),
      };
    },
    // the subpages (GW4: history and options, above; GW6/GW5: release)
    'update.history': async ({ before, limit } = {}, m) => {
      absent(m, 'S02b', 'update.history');
      live(m, 'update.history');
      if ((before != null && !Number.isInteger(before)) || (limit != null && !(Number.isInteger(limit) && limit >= 1 && limit <= 200))) {
        throw apiErr('bad_args', 'update.err.badArgs');
      }
      return historyPage(historyOf(m.params.get('updateScenario'), Date.now()), before, limit ?? 200);
    },
    'update.options': async (a, m) => {
      absent(m, 'S03h', 'update.options');
      if (m.params.get('updateScenario') === 'S03i') return new Promise(() => {});
      live(m, 'update.options');
      return clone(opts(m).value);
    },
    'update.setAutoOpen': async ({ on } = {}, m) => {
      absent(m, 'S03h', 'update.setAutoOpen');
      live(m, 'update.setAutoOpen');
      if (typeof on !== 'boolean') throw apiErr('bad_args', 'update.err.badArgs');
      const o = opts(m);
      const a = o.value.autoOpen;
      // WU-FIX (GW4 review m8): the page hides the row without the schema; the host would refuse (update.options.err.autoOpen)
      if (a.supported && on !== a.on) {
        if (on) { a.days = Number.isInteger(o.previous) ? o.previous : 7; o.previous = null; } else { o.previous = a.days; a.days = OFF_DAYS; }
        a.on = on;
      }
      return clone(o.value);
    },
    // WU-FIX (GW5 review M1): the page's "Controleren op updates" also calls update.release.check (fire and forget).
    // The answers of release.rs check(): {started:true} (check-new-release on its own thread, then update.changed),
    // else {started:false, skipped} for Prompt=never, offline, metered without update.metered, a run in the last 24 h
    // (the mock's runs never fail) or one still running. Its own timer (not u.timers: update.check stops those).
    'update.release.check': async (a, m) => {
      const u = live(m, 'update.release.check');
      const s = u.snap;
      guard(s);
      const now = Date.now();
      const last = u.releaseCheck;
      let skipped = null;
      if (s.release.prompt === 'never') skipped = 'never';
      else if (!s.network.online) skipped = 'offline';
      else if (s.network.metered && !(m.settings()?.update?.metered)) skipped = 'metered';
      else if (last && !last.running && now - last.at < 24 * H) skipped = 'recent';
      else if (last?.running) skipped = 'running';
      if (skipped) return { started: false, skipped };
      const run = u.releaseCheck = { at: now, running: true };
      setTimeout(() => {
        run.running = false;
        Object.assign(s.release, { lastCheck: now, checkError: null });
        if (u.snap === s) changed(m, u);
      }, 800);
      return { started: true };
    },
    // GW6: compat.rs's answers; a refresh of a failed check runs it again (checking 1.5 s, then ok)
    'update.release.compat': async ({ refresh } = {}, m) => {
      const u = live(m, 'update.release.compat');
      const r = u.snap.release;
      guard(u.snap);
      if (!r.offered) return { started: false, skipped: 'notOffered' };
      if (r.state === 'hold') return { started: false, skipped: 'hold' };
      if (r.compat?.running) return { started: false, skipped: 'running' };
      if (!(r.state === 'checking' && r.compat?.needed) && !(refresh && r.state === 'unknown')) return { started: false, skipped: 'recent' };
      Object.assign(r, { state: 'checking', verdict: 'checking', reasons: [], compat: { running: true, needed: false, checkedAt: null } });
      changed(m, u);
      later(u, 1500, () => {
        Object.assign(r, { state: 'ok', verdict: 'ok', compat: { running: false, needed: false, checkedAt: Date.now() } });
        changed(m, u);
      });
      return { started: true };
    },
    'update.release.setPrompt': async ({ prompt } = {}, m) => {
      const u = live(m, 'update.release.setPrompt');
      const s = u.snap;
      guard(s);
      if (!['lts', 'normal', 'never'].includes(prompt)) throw apiErr('bad_args', 'update.err.badArgs');
      // M04: Ubuntu's password dialog is still open; M04b: it was dismissed (pkexec 126)
      if (u.scenario === 'M04') return new Promise(() => {});
      if (u.scenario === 'M04b') throw apiErr('auth_cancelled', 'update.release.err.noPassword');
      if (s.release.override) return { prompt: s.release.prompt, override: s.release.override, changed: false };
      const moved = s.release.prompt !== prompt;
      s.release.prompt = prompt;
      if (moved) changed(m, u);
      return { prompt, override: null, changed: moved };
    },
    // GW6: release.rs's ubuntuHold (the two update-manager keys) as a flag; start hands over to Ubuntu (running)
    'update.release.ubuntuHold': async ({ on } = {}, m) => {
      const u = live(m, 'update.release.ubuntuHold');
      const r = u.snap.release;
      if (r.ubuntuPromptHeld !== !!on) { r.ubuntuPromptHeld = !!on; changed(m, u); }
      return { on: !!on, codename: r.codename || 'stonking' };
    },
    'update.release.start': async (a, m) => {
      const u = live(m, 'update.release.start');
      guard(u.snap);
      Object.assign(u.snap, { phase: 'releaseRunning' });
      Object.assign(u.snap.release, { state: 'running' });
      changed(m, u);
      return { started: true };
    },
    'update.release.attach': async () => ({ started: true }),
  },

  start(m) {
    const u = U(m);
    try {
      globalThis.__mockUpdate = {
        scenario(id) { stopTimers(u); u.scenario = id; u.snap = build(id); if (u.snap) changed(m, u); return id; },
        state: () => clone(u.snap),
      };
    } catch { /* no window */ }
  },
};
