// Instellingen › Update en herstel: Windows Update for Ubuntu (notes/claude/windows-update-gnome.md §3.1-§3.4, §3.6
// "Meer opties"; the update.* service of §4.3/§4.4), settings backup (export / import), reset everything, about
// Win11 Shell.
//
// The page renders every text itself from the snapshot's codes and params ({phase, rows:[{kind, version, count, size,
// security, state, pct}], job, restart, …}), so a language switch needs no host round trip; host-rendered messages
// (error.message, warnings[].message) come back with the next update.status (main.js re-renders the page on a
// switch). Errors are matched on key or code (contracts C2), never on their text. Without an update.* service (an
// older host: update.status is an unknown method) the hero falls back to system.updates (count known: "Updates
// beschikbaar" with the Software Updater button, or "Je bent up-to-date" with Check), else "Kan niet controleren".
// The feature-update card and the "Nieuwe Ubuntu-versies" row live in update-release.js (GW6); Updategeschiedenis
// and Geavanceerde opties are the subpages update-history / update-advanced.

import { h, clear } from '../../../core/dom.js';
import { makeT, list } from '../../../core/i18n.js';
import { getShellInfo, getSetting } from '../../../core/theme.js';
import { relTime, date, time, bytes, duration } from '../../../core/format.js';
import { icon } from '../../../core/icons.js';
import {
  SettingsCard, SettingsGroup, SettingsExpander, Expander, Button, HyperlinkButton, Dialog, Toast, ProgressRing,
  ProgressBar, InfoBar,
} from '../../../core/components.js';
import { call, run, showError, noteDryRun } from '../common.js';
import { releaseCard, promptRow } from './update-release.js';

const t = makeT({
  nl: {
    upToDate: 'Je bent up-to-date',
    unknown: 'Kan niet controleren op updates',
    unknownDesc: 'Gebruik de Software-updater van Ubuntu om te kijken of er updates zijn.',
    checked: 'Laatst gecontroleerd: {when}',
    // Windows' own -ing form in en ("Checking for updates…"): a context row of tools/i18n/glossary-context.tsv
    checking: 'Controleren op updates…',
    openUpdater: 'Software-updater openen',
    updaterTitle: 'Software-updater openen?',
    updaterBody: 'De Software-updater van Ubuntu wordt geopend. Als je updates installeert, vraagt Ubuntu om je wachtwoord. Dat is normaal: updates veranderen het systeem. Sla je werk op, want soms moet de computer daarna opnieuw opstarten.',
    open: 'Openen',
    // Windows Update (§3.2-§3.4, §3.6)
    product: 'Win11 Shell',
    checkFor: 'Controleren op updates',
    updatesAvailable: 'Updates beschikbaar',
    installAll: 'Alles downloaden en installeren',
    downloading: 'Updates downloaden',
    progressDl: '{done} van {total} · nog ongeveer {eta}',
    progressDlNoEta: '{done} van {total}',
    preparing: 'Voorbereiden…',
    cancel: 'Annuleren',
    installing: 'Updates installeren',
    keepOn: 'Laat je pc aan staan.',
    waitLock: 'Wachten tot een andere installatie klaar is',
    waitLockAuto: 'Ubuntu installeert op dit moment automatische updates.',
    waitLockOther: 'Een ander programma installeert software.',
    waitAuth: 'Wachten op toestemming',
    waitAuthDesc: 'Bevestig in het venster dat om je wachtwoord vraagt.',
    foreign: 'Er wordt software geïnstalleerd door een ander programma',
    foreignOwn: '{product} installeert of verwijdert een app',
    foreignDesc: 'Je kunt updates installeren zodra dat klaar is.',
    relRunning: 'Ubuntu wordt bijgewerkt naar {version}',
    relRunningNoVer: 'Ubuntu wordt bijgewerkt',
    relRunningBody: 'Het venster van Ubuntu doet de upgrade. Sluit het niet.',
    relShowWindow: 'Venster weer openen',
    restartRequired: 'Opnieuw opstarten vereist',
    restartDescFor: 'Start je pc opnieuw op om de update van {list} af te ronden.',
    restartDesc: 'Start je pc opnieuw op om de updates af te ronden.',
    restartNow: 'Nu opnieuw opstarten',
    rebootQ: 'Wil je de pc opnieuw opstarten?',
    rebootBody: 'Alle apps worden gesloten en de pc start daarna vanzelf weer op. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.',
    reboot: 'Opnieuw opstarten',
    errorTitle: 'Er zijn problemen opgetreden bij het installeren van sommige updates',
    techDetails: 'Technische details',
    retry: 'Opnieuw proberen',
    unsupportedDesc: '{product} kan updates hier niet zelf installeren. Gebruik de Software-updater van Ubuntu.',
    offline: 'Je bent offline',
    offlineDesc: 'Maak verbinding met internet om te controleren op updates. Laatst gecontroleerd: {when}',
    offlineDescNoWhen: 'Maak verbinding met internet om te controleren op updates.',
    repairTitle: 'Een eerdere installatie is niet afgerond',
    repairDesc: 'Ubuntu moet dit eerst herstellen. Daarna kun je weer updates installeren.',
    repair: 'Herstellen',
    alsoAvailable: 'Ook beschikbaar',
    phasedLater: { one: '1 update wordt later aangeboden', other: '{n} updates worden later aangeboden' },
    keptBack: { one: '1 update wordt nu niet geïnstalleerd', other: '{n} updates worden nu niet geïnstalleerd' },
    pkgKernel: 'Linux-kernel',
    pkgLibc: 'systeembibliotheek',
    pkgGnome: 'GNOME Shell',
    andMore: { one: 'en 1 andere', other: 'en {n} andere' },
    meteredInfo: 'Je gebruikt een verbinding met een datalimiet. Downloaden kan kosten met zich meebrengen.',
    meteredQ: 'Toch downloaden?',
    meteredBody: 'Er wordt ongeveer {size} gedownload.',
    download: 'Downloaden',
    removeQ: 'Pakketten verwijderen?',
    removeBody: 'Voor deze updates moet {list} worden verwijderd. Toch installeren?',
    installAnyway: 'Toch installeren',
    rowSecurity: 'Beveiligingsupdates voor Ubuntu {version}',
    rowGnome: 'GNOME Shell {version}',
    rowWebkit: 'WebKitGTK {version} (webweergave van {product})',
    rowGtk: 'GTK 3 {version}',
    rowSoup: 'libsoup {version}',
    rowGlib: 'GLib {version}',
    rowKernel: 'Linux-kernel {version}',
    rowOther: 'Overige updates',
    rowCaption: { one: '1 pakket · {size}', other: '{n} pakketten · {size}' },
    rowCaptionNoSize: { one: '1 pakket', other: '{n} pakketten' },
    securityTag: 'Beveiligingsupdate',
    rsPendingDownload: 'Wachten op downloaden',
    rsDownloading: 'Downloaden - {pct}%',
    rsPendingInstall: 'Wachten op installatie',
    rsInstalling: 'Installeren - {pct}%',
    rsInstalled: 'Geïnstalleerd',
    rsRestart: 'Wachten op opnieuw opstarten',
    rsFailed: 'Installatiefout',
    offeredLater: 'Wordt later aangeboden',
    keptBackRow: 'Wordt nu niet geïnstalleerd',
    whatsNew: 'Wat is er nieuw',
    changes: 'Wijzigingen',
    cves: 'Beveiligingsproblemen',
    bugs: 'Opgeloste fouten',
    noDetails: 'Ubuntu heeft voor deze update geen beschrijving.',
    close: 'Sluiten',
    shellRestartTitle: '{product} moet opnieuw starten',
    shellRestartBody: 'Onderdelen van het bureaublad zijn bijgewerkt ({list}). Opnieuw starten duurt een paar seconden; vensters van {product} gaan even dicht.',
    shellRestartNow: '{product} opnieuw starten',
    cWebkit: 'WebKitGTK', cGtk: 'GTK', cSoup: 'libsoup', cGlib: 'GLib', cPixbuf: 'GDK Pixbuf', cCairo: 'cairo',
    cArchive: 'libarchive', cSelf: '{product}',
    moreOptions: 'Meer opties',
    historyRow: 'Updategeschiedenis',
    advancedRow: 'Geavanceerde opties',
    advancedRowDesc: 'Melding bij opnieuw opstarten, datalimiet, Software-updater van Ubuntu',
    // backup, reset, about (unchanged)
    backup: 'Back-up van instellingen',
    backupDesc: 'Bewaar je Win11 Shell-instellingen in een bestand, of zet ze terug',
    export: 'Instellingen exporteren',
    exportDesc: 'Sla al je instellingen op in een bestand (bijvoorbeeld op een USB-stick)',
    exportBtn: 'Exporteren',
    exported: 'Instellingen opgeslagen als “{name}” in je map Downloads', exportName: 'win11shell-instellingen-{date}.json',
    exportedCopy: 'Er is ook een reservekopie bewaard: {path}',
    import: 'Instellingen importeren',
    importDesc: 'Zet instellingen terug uit een eerder geëxporteerd bestand',
    importBtn: 'Bestand kiezen',
    importQ: 'Instellingen importeren?',
    importBody: 'Je huidige instellingen worden vervangen door die uit “{name}”. Tip: exporteer eerst je huidige instellingen als je ze wilt bewaren.',
    importOk: 'Importeren',
    imported: 'Instellingen geïmporteerd',
    badFile: 'Dit bestand bevat geen instellingen van Win11 Shell.',
    reset: 'Alles terugzetten',
    resetDesc: 'Alle instellingen van Win11 Shell terugzetten naar de standaardwaarden. Je bestanden blijven bewaard.',
    resetBtn: 'Terugzetten',
    resetQ: 'Alle instellingen terugzetten?',
    resetBody: 'Achtergrond, kleuren, taakbalk, Start en alle andere instellingen van Win11 Shell gaan terug naar de standaard. Je bestanden en apps blijven gewoon bewaard. Dit kan niet ongedaan worden gemaakt, tenzij je eerst exporteert.',
    resetDone: 'Alle instellingen zijn teruggezet',
    about: 'Over Win11 Shell',
    aboutDesc: 'Een Windows 11-achtig bureaublad voor Ubuntu',
    version: 'Versie', mode: 'Weergave', compositor: 'Vensterbeheer', testMode: 'Testmodus',
    yes: 'Aan (er wordt niets echt gewijzigd)',
    m_layer: 'Bureaublad', m_preview: 'Voorbeeldvenster', m_serve: 'Browser', m_app: 'Los venster',
  },
  en: {
    upToDate: 'You’re up to date',
    unknown: 'Can’t check for updates',
    unknownDesc: 'Use Ubuntu’s Software Updater to see whether updates are available.',
    checked: 'Last checked: {when}',
    checking: 'Checking for updates…',
    openUpdater: 'Open Software Updater',
    updaterTitle: 'Open Software Updater?',
    updaterBody: 'Ubuntu’s Software Updater will open. When you install updates, Ubuntu asks for your password. That’s normal: updates change the system. Save your work, because the computer sometimes needs to restart afterwards.',
    open: 'Open',
    product: 'Win11 Shell',
    checkFor: 'Check for updates',
    updatesAvailable: 'Updates available',
    installAll: 'Download & install all',
    downloading: 'Downloading updates',
    progressDl: '{done} of {total} · about {eta} left',
    progressDlNoEta: '{done} of {total}',
    preparing: 'Getting ready…',
    cancel: 'Cancel',
    installing: 'Installing updates',
    keepOn: 'Keep your PC turned on.',
    waitLock: 'Waiting for another installation to finish',
    waitLockAuto: 'Ubuntu is installing automatic updates right now.',
    waitLockOther: 'Another program is installing software.',
    waitAuth: 'Waiting for permission',
    waitAuthDesc: 'Confirm in the window that asks for your password.',
    foreign: 'Another program is installing software',
    foreignOwn: '{product} is installing or removing an app',
    foreignDesc: 'You can install updates as soon as that’s finished.',
    relRunning: 'Upgrading Ubuntu to {version}',
    relRunningNoVer: 'Upgrading Ubuntu',
    relRunningBody: 'Ubuntu’s window does the upgrade. Don’t close it.',
    relShowWindow: 'Show the upgrade window',
    restartRequired: 'Restart required',
    restartDescFor: 'Restart your PC to finish the update of {list}.',
    restartDesc: 'Restart your PC to finish installing updates.',
    restartNow: 'Restart now',
    rebootQ: 'Do you want to restart the PC?',
    rebootBody: 'All apps will close and the PC will restart by itself. Any work you haven’t saved might be lost.',
    reboot: 'Restart',
    errorTitle: 'There were problems installing some updates',
    techDetails: 'Technical details',
    retry: 'Retry',
    unsupportedDesc: '{product} can’t install updates here itself. Use Ubuntu’s Software Updater.',
    offline: 'You’re offline',
    offlineDesc: 'Connect to the internet to check for updates. Last checked: {when}',
    offlineDescNoWhen: 'Connect to the internet to check for updates.',
    repairTitle: 'An earlier installation didn’t finish',
    repairDesc: 'Ubuntu needs to repair this first. Then you can install updates again.',
    repair: 'Repair',
    alsoAvailable: 'Also available',
    phasedLater: { one: '1 update will be offered later', other: '{n} updates will be offered later' },
    keptBack: { one: '1 update isn’t being installed right now', other: '{n} updates aren’t being installed right now' },
    pkgKernel: 'Linux kernel',
    pkgLibc: 'system library',
    pkgGnome: 'GNOME Shell',
    andMore: { one: 'and 1 other', other: 'and {n} others' },
    meteredInfo: 'You’re on a metered connection. Downloading may cost you data charges.',
    meteredQ: 'Download anyway?',
    meteredBody: 'About {size} will be downloaded.',
    download: 'Download',
    removeQ: 'Remove packages?',
    removeBody: 'These updates need {list} to be removed. Install anyway?',
    installAnyway: 'Install anyway',
    rowSecurity: 'Security updates for Ubuntu {version}',
    rowGnome: 'GNOME Shell {version}',
    rowWebkit: 'WebKitGTK {version} ({product}’s web view)',
    rowGtk: 'GTK 3 {version}',
    rowSoup: 'libsoup {version}',
    rowGlib: 'GLib {version}',
    rowKernel: 'Linux kernel {version}',
    rowOther: 'Other updates',
    rowCaption: { one: '1 package · {size}', other: '{n} packages · {size}' },
    rowCaptionNoSize: { one: '1 package', other: '{n} packages' },
    securityTag: 'Security update',
    rsPendingDownload: 'Pending download',
    rsDownloading: 'Downloading - {pct}%',
    rsPendingInstall: 'Pending install',
    rsInstalling: 'Installing - {pct}%',
    rsInstalled: 'Installed',
    rsRestart: 'Pending restart',
    rsFailed: 'Install error',
    offeredLater: 'Offered later',
    keptBackRow: 'Not being installed right now',
    whatsNew: 'What’s new',
    changes: 'Changes',
    cves: 'Security issues',
    bugs: 'Fixed bugs',
    noDetails: 'Ubuntu has no description for this update.',
    close: 'Close',
    shellRestartTitle: '{product} needs to restart',
    shellRestartBody: 'Parts of the desktop were updated ({list}). Restarting takes a few seconds; {product} windows close briefly.',
    shellRestartNow: 'Restart {product}',
    cWebkit: 'WebKitGTK', cGtk: 'GTK', cSoup: 'libsoup', cGlib: 'GLib', cPixbuf: 'GDK Pixbuf', cCairo: 'cairo',
    cArchive: 'libarchive', cSelf: '{product}',
    moreOptions: 'More options',
    historyRow: 'Update history',
    advancedRow: 'Advanced options',
    advancedRowDesc: 'Restart notifications, metered connections, Ubuntu Software Updater',
    backup: 'Settings backup',
    backupDesc: 'Save your Win11 Shell settings in a file, or restore them',
    export: 'Export settings',
    exportDesc: 'Save all your settings in a file (for example on a USB stick)',
    exportBtn: 'Export',
    exported: 'Settings saved as “{name}” in your Downloads folder', exportName: 'win11shell-settings-{date}.json',
    exportedCopy: 'A backup copy was also kept: {path}',
    import: 'Import settings',
    importDesc: 'Restore settings from a file you exported earlier',
    importBtn: 'Choose file',
    importQ: 'Import settings?',
    importBody: 'Your current settings will be replaced by those in “{name}”. Tip: export your current settings first if you want to keep them.',
    importOk: 'Import',
    imported: 'Settings imported',
    badFile: 'This file doesn’t contain Win11 Shell settings.',
    reset: 'Reset everything',
    resetDesc: 'Reset all Win11 Shell settings to their defaults. Your files are kept.',
    resetBtn: 'Reset',
    resetQ: 'Reset all settings?',
    resetBody: 'Background, colours, taskbar, Start and all other Win11 Shell settings go back to the defaults. Your files and apps are kept. This can’t be undone unless you export first.',
    resetDone: 'All settings were reset',
    about: 'About Win11 Shell',
    aboutDesc: 'A Windows 11-style desktop for Ubuntu',
    version: 'Version', mode: 'View', compositor: 'Window manager', testMode: 'Test mode',
    yes: 'On (nothing is actually changed)',
    m_layer: 'Desktop', m_preview: 'Preview window', m_serve: 'Browser', m_app: 'Separate window',
  },
});

/* ------------------------------------------------------------------ texts from codes */

const prod = () => ({ product: t('product') });
const whenText = (at) => `${relTime(at)} (${date(at, 'short')} ${time(at)})`;
const pctOf = (v) => (Number.isFinite(v) ? Math.max(0, Math.min(100, Math.round(v))) : 0);

/** §3.3 row titles by kind (a missing version leaves no stray space) */
const ROW_TITLE = { security: 'rowSecurity', gnome: 'rowGnome', webkit: 'rowWebkit', gtk: 'rowGtk', soup: 'rowSoup', glib: 'rowGlib', kernel: 'rowKernel' };
function rowTitle(r) {
  const key = ROW_TITLE[r.kind];
  if (!key) return t('rowOther');
  return t(key, { version: r.version ?? '', ...prod() }).replace(/\s+\)/, ')').replace(/\s{2,}/g, ' ').trim();
}
function rowCaption(r) {
  const n = Number(r.count) || 0;
  const c = r.size > 0 ? t('rowCaption', { n, size: bytes(r.size) }) : t('rowCaptionNoSize', { n });
  return r.security && r.kind !== 'security' ? `${t('securityTag')} · ${c}` : c;
}
const ROW_STATE = {
  pendingDownload: 'rsPendingDownload', downloading: 'rsDownloading', pendingInstall: 'rsPendingInstall',
  installing: 'rsInstalling', installed: 'rsInstalled', restart: 'rsRestart', failed: 'rsFailed',
};
const rowState = (r) => t(ROW_STATE[r.state] || 'rsPendingDownload', { pct: pctOf(r.pct) });
const MOVING = new Set(['downloading', 'installing']);
const BUSY = new Set(['checking', 'downloading', 'installing', 'waiting', 'foreign', 'releaseRunning']);

/** restart.rebootPkgs → friendly names (§3.2): at most 3, then "en 2 andere" */
function pkgName(name) {
  if (/^linux-(image|modules)-|^linux-base$/.test(name)) return t('pkgKernel');
  if (name === 'libc6') return t('pkgLibc');
  if (/^(gnome-shell|mutter|libmutter-)/.test(name)) return t('pkgGnome');
  return name;
}
function shortList(names) {
  const xs = [...new Set(names)];
  return xs.length > 3 ? `${xs.slice(0, 3).join(', ')} ${t('andMore', { n: xs.length - 3 })}` : list(xs);
}
function restartText(r) {
  const names = (r?.rebootPkgs || []).map(pkgName);
  return names.length ? t('restartDescFor', { list: shortList(names) }) : t('restartDesc');
}

/** shell restart parts (§3.4, restart.shellParts) → component names */
const PART = {
  webkit: 'cWebkit', jsc: 'cWebkit', gtk: 'cGtk', gdk: 'cGtk', soup: 'cSoup', glib: 'cGlib', gio: 'cGlib',
  gobject: 'cGlib', pixbuf: 'cPixbuf', cairo: 'cCairo', archive: 'cArchive', self: 'cSelf',
};

/* Host errors are told apart by their catalogue key (C2: key or code, never the text). The §3.7 key names are fixed
   by the design; of the codes only network (§4.3) and test_seam (§4.11) are, so those two also count by code. */
const isTestSeam = (e) => e?.key === 'update.err.testSeam' || e?.code === 'test_seam';
/** InfoBar severity of a host error shown next to the hero; default warning */
function barSeverity(e) {
  if (e?.key === 'update.err.cancelled') return 'info';
  return isTestSeam(e) ? 'error' : 'warning';
}
/** errors after which the hero's Retry checks again instead of installing: no server, offline, the check failed */
const RECHECK_KEYS = new Set(['update.err.network', 'update.err.offline', 'update.err.checkFailed']);
const isRecheck = (e) => RECHECK_KEYS.has(e?.key) || e?.code === 'network';

/* ------------------------------------------------------------------ hero model (§3.2) */

/** legacy: system.updates of a host without update.* ({available, checkedAt}); null when that failed too */
function legacyModel(u) {
  const n = u?.available;
  const sub = u?.checkedAt ? t('checked', { when: whenText(u.checkedAt) }) : '';
  if (Number.isFinite(n) && n > 0) return { cls: 'is-accent', title: t('updatesAvailable'), sub, action: 'openUpdater' };
  if (n === 0) return { cls: 'is-ok', badge: 'ok', title: t('upToDate'), sub, action: 'legacyCheck' };
  return { cls: 'is-unknown', glyph: 'warning', title: t('unknown'), sub: t('unknownDesc'), action: 'openUpdater' };
}

function heroModel(s, legacy) {
  if (!s) return legacyModel(legacy);
  const j = s.job || {};
  const checked = s.checkedAt ? t('checked', { when: whenText(s.checkedAt) }) : '';
  switch (s.phase) {
    case 'idle':
      return { cls: 'is-ok', badge: 'ok', title: t('upToDate'), sub: checked, action: 'checkFor' };
    case 'checking':
      return { cls: 'is-accent', ring: true, title: t('checking'), sub: '' };
    case 'available':
      return { cls: 'is-accent', title: t('updatesAvailable'), sub: checked, action: 'installAll' };
    case 'downloading': {
      // the simulate (and its confirm): "Voorbereiden…"; then "{done} van {total}" while the total is known (the
      // simulate's DownloadSizeRemaining, §3.3), else no caption: the rows show "Downloaden - 45%"
      let sub = '';
      const total = s.totals?.downloadSize;
      if (j.simulating || j.confirm) sub = t('preparing');
      else if (total > 0 && Number.isFinite(j.downloadLeft)) {
        const p = { done: bytes(Math.max(0, total - j.downloadLeft)), total: bytes(total) };
        sub = j.remaining > 0 ? t('progressDl', { ...p, eta: duration(j.remaining, { long: true }) }) : t('progressDlNoEta', p);
      }
      return { cls: 'is-accent', busy: true, title: t('downloading'), sub, action: j.allowCancel ? 'cancel' : null };
    }
    case 'installing':
      return { cls: 'is-accent', busy: true, title: t('installing'), sub: t('keepOn') };
    case 'waiting':
      if (j.status === 31) return { cls: 'is-accent', title: t('waitAuth'), sub: t('waitAuthDesc') };
      return { cls: 'is-accent', title: t('waitLock'), sub: t(j.lockHolder === 'unattended' ? 'waitLockAuto' : 'waitLockOther'), action: j.allowCancel ? 'cancel' : null };
    case 'foreign':
      return { cls: 'is-accent', title: j.foreignOwn ? t('foreignOwn', prod()) : t('foreign'), sub: t('foreignDesc'), action: 'installAll', disabled: true };
    case 'releaseRunning': {
      const v = s.release?.version || s.release?.released?.version;
      return { cls: 'is-accent', busy: true, title: v ? t('relRunning', { version: v }) : t('relRunningNoVer'), sub: t('relRunningBody'), action: 'attach' };
    }
    case 'restartRequired':
      return { cls: 'is-restart', title: t('restartRequired'), sub: restartText(s.restart), action: 'restartNow' };
    case 'error':
      return { cls: 'is-accent', badge: 'error', title: t('errorTitle'), sub: s.error?.message || '', action: 'retry' };
    case 'unsupported':
      return { cls: 'is-unknown', glyph: 'warning', title: t('unknown'), sub: isTestSeam(s.error) && s.error.message ? s.error.message : t('unsupportedDesc', prod()), action: 'openUpdater' };
    case 'offline':
      return { cls: 'is-accent', badge: 'error', title: t('offline'), sub: s.checkedAt ? t('offlineDesc', { when: whenText(s.checkedAt) }) : t('offlineDescNoWhen'), action: 'retry' };
    case 'repairRequired':
      return { cls: 'is-accent', badge: 'error', title: t('repairTitle'), sub: t('repairDesc'), action: 'repair' };
    default:
      return { cls: 'is-unknown', glyph: 'warning', title: t('unknown'), sub: t('unknownDesc'), action: 'openUpdater' };
  }
}

/* ------------------------------------------------------------------ the Windows Update section */

function updateSection(ctx) {
  let s = null;                 // the last update.status snapshot; null = no update.* service here (or it failed)
  let legacy = null;            // without update.*: the system.updates answer ({available, checkedAt}) or null
  let legacyMode = false;       // update.status is an unknown method on this host
  let pkgCache = null;          // {rev, list} of update.status {packages:true}
  let pkgReq = null;            // {rev, promise} of the packages request in flight (one per rev, §4.3 "asks once")
  let callError = null;         // an immediate refusal of a write (busy, stale, cannot_cancel, …): an InfoBar
  let confirmOpen = null;       // {key, dialog} of the open metered/removals confirm
  let confirmAnswered = null;   // the key of the confirm answered last: not shown again while the job still has it
  const pending = new Set();    // buttons whose action is in flight: they stay disabled through every repaint
  // shell.restartAll answered ok (not a dry run): every Win11 process restarts, this page too, so the InfoBar's
  // button stays disabled until the page is built again (gx1.md §7 decision 1; a second call would only answer
  // pending:true)
  let restartAllSent = false;
  ctx.onCleanup(() => { confirmOpen?.dialog?.close('close'); confirmOpen = null; });

  /* ---- hero ---- */
  const glyph = h('span', { class: 'st-upd__glyph' });
  const title = h('div', { class: 'st-upd__title' });
  const sub = h('div', { class: 'st-upd__sub' });
  const btn = Button({ label: t('checkFor'), variant: 'accent' });   // label and variant follow the state
  const actions = h('div', { class: 'st-upd__actions' });
  const hero = h('section', { class: 'st-upd' }, glyph, h('div', { class: 'st-upd__text' }, title, sub), actions);
  let glyphKey = '';
  let model = null;

  const bars = h('div', { class: 'st-updbars' });
  const rowsEl = h('div', { class: 'st-updrows' });
  const host = { call, reload: () => load(), restartNow: () => restartNow(), product: t('product') };
  const release = releaseCard(ctx, host);
  const prompt = promptRow(ctx, host);
  const more = SettingsGroup({ title: t('moreOptions'), children: [
    prompt,
    SettingsCard({ icon: 'history', title: t('historyRow'), onClick: () => ctx.go('update-history') }),
    SettingsCard({ icon: 'settings', title: t('advancedRow'), description: t('advancedRowDesc'), onClick: () => ctx.go('update-advanced') }),
  ] });

  /* ---- host calls ---- */
  let loading = null, again = false;
  async function load() {
    if (loading) { again = true; return loading; }
    loading = (async () => {
      do {
        again = false;
        // the regular snapshot never carries the package list; packages() asks for it when a row needs it
        const r = await call('update.status', {});
        if (!ctx.alive()) return;
        s = r.ok && r.value && typeof r.value === 'object' ? r.value : null;
        legacyMode = !r.ok && r.error?.code === 'unknown_method';
        legacy = null;
        if (legacyMode) {
          const u = await call('system.updates');
          if (!ctx.alive()) return;
          legacy = u.ok && u.value && typeof u.value === 'object' ? u.value : null;
        }
        paint(true);
      } while (again && ctx.alive());
    })();
    try { await loading; } finally { loading = null; }
  }

  /** the package list of the current rev: one update.status {packages:true} per rev, shared by every open row */
  async function packages() {
    const rev = s?.rev ?? null;
    if (pkgCache && pkgCache.rev === rev) return pkgCache.list;
    if (pkgReq && pkgReq.rev === rev) return pkgReq.promise;
    const promise = (async () => {
      const r = await call('update.status', { packages: true });
      if (r.ok && Array.isArray(r.value?.packages)) { pkgCache = { rev: r.value.rev, list: r.value.packages }; return pkgCache.list; }
      return [];
    })();
    pkgReq = { rev, promise };
    try { return await promise; } finally { if (pkgReq?.promise === promise) pkgReq = null; }
  }

  /** runs fn with `button` disabled; a second click while fn runs does nothing (every repaint keeps it disabled) */
  async function holding(button, fn) {
    if (!button) return fn();
    if (pending.has(button)) return null;
    pending.add(button);
    button.setDisabled(true);
    try { return await fn(); } finally { pending.delete(button); }
  }

  /** a write method: refusals become the InfoBar (by key/code), dry-run gets its toast, then the page re-reads (and
   *  repaints every button's own disabled state); onOk(value) runs on an ok answer, before that repaint */
  async function write(method, args = {}, button = null, onOk = null) {
    if (button && pending.has(button)) return null;
    const r = await holding(button, () => call(method, args));
    if (!ctx.alive()) return null;
    if (!r.ok) {
      if (r.error?.code === 'unknown_method') showError(r.error);
      else callError = r.error || null;
    } else {
      callError = null;
      noteDryRun(r.value);
      onOk?.(r.value);
    }
    await load();
    return r.ok ? r.value : null;
  }

  async function restartNow() {
    // Settings' own restart, like Systeem › Aan/uit (pages/power.js) and Start › Aan/uit (shell/power.js): the confirm
    // (unless power.confirm is off), then system.power.action reboot, which the power guard (§4.9) checks. Not
    // shell.command power (§3.2's first path): that is a broadcast that every Win11 host with shell/power.js handles
    // (the panel and the prewarmed Start), and no host reports whether a panel took it (gw3.md, fix round M1).
    if (getSetting('power.confirm', true) !== false) {
      const ok = await Dialog.confirm({ title: t('rebootQ'), message: t('rebootBody'), confirmLabel: t('reboot'), cancelLabel: t('cancel') });
      if (!ok || !ctx.alive()) return;
    }
    await run('system.power.action', { action: 'reboot' });
  }

  async function openUpdater() {
    const ok = await Dialog.confirm({ title: t('updaterTitle'), message: t('updaterBody'), confirmLabel: t('open'), icon: 'info' });
    if (ok) await run('system.openUpdater');
  }

  /** hero actions that are no update.* write: the button stays disabled until they end, then follows the state again */
  const heroHold = (b, fn) => holding(b, fn).finally(() => { if (ctx.alive()) paintHero(); });

  const ACTIONS = {
    // WU-FIX (GW5 review M1, design §6.1 step 3): the click also asks Ubuntu for a new version
    // (update.release.check: at most daily, so its {started:false, skipped} and any refusal stay silent; the
    // result reaches the release card through update.changed)
    checkFor: { label: () => t('checkFor'), run: (b) => { void call('update.release.check', {}); return write('update.check', {}, b); } },
    installAll: { label: () => t('installAll'), run: (b) => write('update.install', {}, b) },
    cancel: { label: () => t('cancel'), variant: 'standard', run: (b) => write('update.cancel', { jobId: s?.job?.jobId ?? null }, b) },
    attach: { label: () => t('relShowWindow'), run: (b) => write('update.release.attach', {}, b) },
    restartNow: { label: () => t('restartNow'), run: (b) => heroHold(b, restartNow) },
    repair: { label: () => t('repair'), run: (b) => write('update.repair', {}, b) },
    openUpdater: { label: () => t('openUpdater'), run: (b) => heroHold(b, openUpdater) },
    // a host without update.*: "Controleren op updates" reads system.updates again (the 3b898e6 page's Check)
    legacyCheck: { label: () => t('checkFor'), run: (b) => heroHold(b, load) },
    retry: {
      label: () => t('retry'),
      // a failed install is tried again; a failed check (or nothing left to install) checks again
      run: (b) => (s?.phase === 'error' && s.rows?.length && !isRecheck(s.error)
        ? write('update.install', {}, b) : write('update.check', {}, b)),
    },
  };
  btn.addEventListener('click', () => { const a = model && ACTIONS[model.action]; if (a && !btn.disabled) a.run(btn); });

  /* ---- painting ---- */
  function paintHero() {
    model = heroModel(s, legacy);
    hero.className = ['st-upd', model.cls, model.busy && 'is-busy'].filter(Boolean).join(' ');
    const key = `${model.ring ? 'ring' : model.glyph || 'update'}|${model.badge || ''}`;
    if (key !== glyphKey) {
      glyphKey = key;
      clear(glyph);
      if (model.ring) glyph.append(ProgressRing({ size: 40 }));
      else {
        glyph.append(h('span', { class: 'st-upd__disc' }, icon(model.glyph || 'update', { size: 32 }),
          model.badge ? h('span', { class: ['st-upd__badge', model.badge === 'error' && 'is-error'] },
            icon(model.badge === 'error' ? 'close' : 'check', { size: 12 })) : null));
      }
    }
    title.textContent = model.title;
    sub.textContent = model.sub || '';
    const a = ACTIONS[model.action];
    if (!a) { btn.remove(); return; }           // states without a button (checking, installing, …)
    if (btn.parentNode !== actions) actions.append(btn);
    btn.setLabel(a.label());
    btn.setVariant(a.variant || 'accent');
    btn.setDisabled(!!model.disabled || pending.has(btn));
  }

  function bar(severity, message, { title: tt = '', action, closable = false, onClose } = {}) {
    const b = InfoBar({ severity, title: tt, message, action, closable, onClose });
    b.classList.add('st-notice');
    return b;
  }

  function paintBars() {
    clear(bars);
    if (!s) return;
    if (s.phase === 'error' && s.error?.detail) {
      bars.append(Expander({
        header: t('techDetails'), cls: 'st-upd__details',
        content: h('pre', { class: 'st-upd__detail selectable' }, s.error.detail),
      }));
    }
    const r = s.restart;
    const parts = r?.shellParts || [];
    // the Win11 restart InfoBar (§3.4): hidden while a reboot is pending, except when WebKitGTK/JavaScriptCore changed
    if (r?.shell && (!r.reboot || parts.some((p) => p === 'webkit' || p === 'jsc'))) {
      // every component by name (at most 8 short names: §3.4 has no "en N andere"; jsc/gdk/gio/gobject fold into theirs)
      const names = [...new Set(parts.map((p) => (PART[p] ? t(PART[p], prod()) : p)))];
      const restartBtn = Button({
        label: t('shellRestartNow', prod()),
        onClick: () => {
          if (restartAllSent) return;
          write('shell.restartAll', { reason: 'libraries' }, restartBtn, (v) => { if (!v?.dryRun) restartAllSent = true; });
        },
      });
      restartBtn.setDisabled(restartAllSent);
      bars.append(bar('warning', t('shellRestartBody', { list: names.length ? list(names) : t('cSelf', prod()), ...prod() }),
        { title: t('shellRestartTitle', prod()), action: restartBtn }));
    }
    if (s.network?.metered && (s.rows || []).some((x) => x.state === 'pendingDownload' || x.state === 'downloading')) {
      bars.append(bar('info', t('meteredInfo')));
    }
    for (const w of s.warnings || []) if (w?.message) bars.append(bar('warning', w.message));
    if (s.error?.message && !['error', 'repairRequired', 'unsupported', 'offline'].includes(s.phase)) {
      bars.append(bar(barSeverity(s.error), s.error.message));
    }
    if (callError?.message) {
      bars.append(bar(barSeverity(callError), callError.message, { closable: true, onClose: () => { callError = null; } }));
    }
  }

  /* rows (§3.3): one SettingsExpander per row, kept across repaints so an opened row stays open */
  const views = new Map();
  function rowView(row) {
    let cur = row;
    const stateEl = h('div', { class: 'st-updrow__state' });
    // the bar and the row's Retry exist only while they show (a hidden one would still be read and dumped)
    const pbar = ProgressBar({ width: 160, ariaLabel: rowTitle(row) });
    const status = h('div', { class: 'st-updrow__status' }, stateEl);
    const retry = Button({ label: t('retry'), onClick: () => write('update.install', { rows: [cur.id] }, retry) });
    const retrySlot = h('span', { class: 'st-updrow__retry' });
    const details = h('div', { class: 'st-updrow__details' });
    const exp = SettingsExpander({
      title: rowTitle(row), description: rowCaption(row), control: [status, retrySlot], children: [details],
      onToggle: (open) => { if (open) fill(); },
    });
    exp.classList.add('st-updrow');
    const titleEl = exp.header.querySelector('.fl-sc__title');
    const descEl = exp.header.querySelector('.fl-sc__desc');
    let filledRev = null;

    async function fill() {
      const all = await packages();
      if (!ctx.alive()) return;
      filledRev = s?.rev ?? null;
      const mine = (all || []).filter((p) => p.row === cur.id);
      clear(details);
      details.append(h('ul', { class: 'st-updrow__pkgs' }, mine.map((p) => h('li', { class: 'st-updrow__pkg' },
        h('div', { class: 'st-updrow__pkgtext' },
          h('div', { class: 'st-updrow__pkgname selectable' }, p.from && p.from !== '-' ? `${p.name} ${p.from} → ${p.to}` : `${p.name} ${p.to}`),
          p.summary ? h('div', { class: 'st-updrow__pkgsum' }, p.summary) : null),
        p.phased || p.held ? h('span', { class: 'st-updrow__tag' }, t(p.phased ? 'offeredLater' : 'keptBackRow')) : null))));
      const ids = mine.filter((p) => !p.phased && !p.held).map((p) => p.id).slice(0, 20);
      if (ids.length) {
        const link = HyperlinkButton({ label: t('whatsNew'), onClick: () => whatsNew(cur, ids, mine) });
        link.classList.add('st-updrow__whatsnew');
        details.append(h('div', { class: 'st-updrow__links' }, link));
      }
    }

    exp.update = (r) => {
      cur = r;
      titleEl.textContent = rowTitle(r);
      if (descEl) descEl.textContent = rowCaption(r);
      pbar.setAttribute('aria-label', rowTitle(r));
      stateEl.textContent = rowState(r);
      const failed = r.state === 'failed';
      if (MOVING.has(r.state) || failed) {
        if (pbar.parentNode !== status) status.append(pbar);
        pbar.setValue(failed ? (Number.isFinite(r.pct) ? pctOf(r.pct) : 100) : pctOf(r.pct));
        pbar.setState(failed ? 'error' : 'normal');
      } else pbar.remove();
      if (failed) { if (retry.parentNode !== retrySlot) retrySlot.append(retry); } else retry.remove();
      retry.setDisabled((!!s && BUSY.has(s.phase)) || pending.has(retry));
      exp.classList.toggle('is-failed', failed);
      if (exp.expanded && filledRev !== (s?.rev ?? null)) fill();
    };
    exp.update(row);
    return exp;
  }

  function paintRows(structural) {
    const rows = s && s.phase !== 'checking' ? (s.rows || []) : [];
    if (!structural) {
      for (const r of rows) views.get(r.id)?.update(r);
      return;
    }
    const keep = new Set(rows.map((r) => r.id));
    for (const id of [...views.keys()]) if (!keep.has(id)) views.delete(id);
    clear(rowsEl);
    if (rows.length && s.phase === 'restartRequired') {
      const all = Button({ label: t('installAll'), onClick: () => write('update.install', {}, all) });
      rowsEl.append(h('div', { class: 'st-updrows__head' }, h('div', { class: 'st-updrows__title' }, t('alsoAvailable')), all));
    }
    if (rows.length) {
      rowsEl.append(h('div', { class: 'st-updrows__list' }, rows.map((r) => {
        let v = views.get(r.id);
        if (v) v.update(r); else { v = rowView(r); views.set(r.id, v); }
        return v;
      })));
    }
    const tot = s?.totals || {};
    if (s && s.phase !== 'checking' && tot.phased > 0) rowsEl.append(h('div', { class: 'st-updrows__note' }, t('phasedLater', { n: tot.phased })));
    if (s && s.phase !== 'checking' && tot.held > 0) rowsEl.append(h('div', { class: 'st-updrows__note' }, t('keptBack', { n: tot.held })));
  }

  /* metered / removals confirm of a paused job (§3.2 dialogs, job.confirm). Once answered, the same confirm is not
     shown again while a reload still sees it (update.confirm in flight); a failed answer shows it again. */
  function paintConfirm() {
    const c = s?.job?.confirm;
    const key = c && s.job.jobId ? `${s.job.jobId}:${c.kind}` : null;
    if (confirmOpen && confirmOpen.key !== key) { confirmOpen.dialog?.close('close'); confirmOpen = null; }
    if (confirmAnswered && confirmAnswered !== key) confirmAnswered = null;
    if (!key || confirmOpen || key === confirmAnswered) return;
    const jobId = s.job.jobId;
    const metered = c.kind === 'metered';
    const p = Dialog.show({
      title: metered ? t('meteredQ') : t('removeQ'),
      content: metered ? t('meteredBody', { size: bytes(c.size ?? s.totals?.downloadSize) }) : t('removeBody', { list: list(c.removals || []) }),
      primary: metered ? t('download') : t('installAnyway'), close: t('cancel'), icon: 'warning',
    });
    confirmOpen = { key, dialog: p.dialog };
    p.then(async (r) => {
      if (confirmOpen?.key !== key) return;
      confirmOpen = null;
      confirmAnswered = key;
      if (!ctx.alive()) return;
      const v = await write('update.confirm', { jobId, accept: r === 'primary' });
      if (v === null && ctx.alive() && confirmAnswered === key) { confirmAnswered = null; paintConfirm(); }
    });
  }

  function paint(structural) {
    paintHero();
    if (structural) {
      paintBars();
      release.update?.(s);
      prompt.update?.(s);
      paintConfirm();
    }
    paintRows(structural);
  }

  /* "Wat is er nieuw" (update.details, GetUpdateDetail; the apt backend fetches Ubuntu's changelogs) */
  async function whatsNew(row, ids, pkgs) {
    const body = h('div', { class: 'st-updnews' }, h('div', { class: 'st-updnews__wait' }, ProgressRing({ size: 24 })));
    Dialog.show({ title: rowTitle(row), content: body, close: t('close'), width: 560 });
    const r = await call('update.details', { ids });
    clear(body);
    if (!r.ok) { body.append(h('p', { class: 'st-updnews__none' }, r.error?.message || t('noDetails'))); return; }
    const links = (urls) => h('ul', { class: 'st-updnews__links' }, urls.filter((u) => /^https:\/\//.test(u)).map((u) => h('li', null,
      HyperlinkButton({ label: u.replace(/^https:\/\/(www\.)?/, ''), onClick: () => run('apps.openUri', { uri: u }) }))));
    let any = false;
    for (const d of r.value?.details || []) {
      const text = [d.updateText, d.changelog].filter(Boolean).join('\n\n');
      const cves = d.cves || [], bugs = d.bugs || [];
      if (!text && !cves.length && !bugs.length) continue;
      any = true;
      const p = pkgs.find((x) => x.id === d.id);
      body.append(h('section', { class: 'st-updnews__pkg' },
        h('div', { class: 'st-updnews__name' }, p ? `${p.name} ${p.to}` : String(d.id).split(';').slice(0, 2).join(' ')),
        text ? [h('div', { class: 'st-updnews__h' }, t('changes')), h('pre', { class: 'st-updnews__text selectable' }, text)] : null,
        cves.length ? [h('div', { class: 'st-updnews__h' }, t('cves')), links(cves)] : null,
        bugs.length ? [h('div', { class: 'st-updnews__h' }, t('bugs')), links(bugs)] : null));
    }
    if (!any) body.append(h('p', { class: 'st-updnews__none' }, t('noDetails')));
  }

  /* ---- events (§4.4): structural changes re-read the snapshot, progress patches it in place ---- */
  ctx.on('update.changed', (d) => { if (!s || d?.rev !== s.rev) load(); });
  // a host without update.*: its own re-checks arrive as system.updates.changed (as on the 3b898e6 page)
  ctx.on('system.updates.changed', () => { if (legacyMode) load(); });
  ctx.on('update.progress', (d) => {
    if (!s?.job || !d || d.jobId !== s.job.jobId) { load(); return; }
    for (const k of ['status', 'pct', 'speed', 'remaining', 'downloadLeft', 'allowCancel']) if (k in d) s.job[k] = d[k];
    for (const r of s.rows || []) {
      const p = d.rows?.[r.id];
      if (p) { r.state = p.state; r.pct = p.pct; }
    }
    paint(false);
  });

  return { hero, bars, release, rowsEl, more, load };
}

function download(name, text) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const a = h('a', { href: url, download: name, style: { display: 'none' } });
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(url); a.remove(); }, 2000);
}

export const pages = {
  async update(ctx) {
    const upd = updateSection(ctx);
    await upd.load();
    const resultSlot = h('div', { class: 'st-slot' });

    const exportBtn = Button({
      label: t('exportBtn'),
      onClick: async () => {
        const r = await call('settings.export');
        if (!r.ok) { showError(r.error); return; }
        const stamp = new Date().toISOString().slice(0, 10);
        const name = r.value.name || t('exportName', { date: stamp });
        download(name, r.value.json);
        const b = await call('settings.backupFile');
        clear(resultSlot).append(InfoBar({
          severity: 'success', title: t('exported', { name }),
          message: b.ok && b.value?.path ? t('exportedCopy', { path: b.value.path }) : '',
        }));
      },
    });

    const fileInput = h('input', { type: 'file', accept: '.json,application/json', style: { display: 'none' } });
    fileInput.addEventListener('change', async () => {
      const f = fileInput.files?.[0];
      fileInput.value = '';
      if (!f) return;
      let text = '';
      try { text = await f.text(); JSON.parse(text); } catch { Toast.show({ severity: 'error', title: t('badFile'), timeout: 5000 }); return; }
      const ok = await Dialog.confirm({ title: t('importQ'), message: t('importBody', { name: f.name }), confirmLabel: t('importOk') });
      if (!ok) return;
      const r = await call('settings.import', { json: text });
      if (r.ok) { Toast.show({ severity: 'success', title: t('imported'), timeout: 2500 }); clear(resultSlot); }
      else showError(r.error, t('badFile'));
    });
    const importBtn = Button({ label: t('importBtn'), onClick: () => fileInput.click() });

    const resetBtn = Button({
      label: t('resetBtn'),
      onClick: async () => {
        const ok = await Dialog.confirm({ title: t('resetQ'), message: t('resetBody'), confirmLabel: t('resetBtn'), danger: true, icon: 'warning' });
        if (!ok) return;
        const r = await call('settings.reset', {});
        if (r.ok) Toast.show({ severity: 'success', title: t('resetDone'), timeout: 2500 });
        else showError(r.error);
      },
    });

    const sh = getShellInfo() || {};
    const mName = (m) => { const k = `m_${m}`; const v = t(k); return v === k ? m : v; };
    const aboutRows = [
      [t('version'), sh.version],
      [t('mode'), sh.mode ? mName(sh.mode) : null],
      [t('compositor'), sh.compositor ? sh.compositor[0].toUpperCase() + sh.compositor.slice(1) : null],
      [t('testMode'), sh.dryRun ? t('yes') : null],
    ];

    const backup = SettingsGroup({ title: t('backup'), description: t('backupDesc'), children: [
      SettingsCard({ icon: 'upload', title: t('export'), description: t('exportDesc'), control: exportBtn }),
      resultSlot,
      SettingsCard({ icon: 'download', title: t('import'), description: t('importDesc'), control: [importBtn, fileInput] }),
      SettingsCard({ icon: 'restart', title: t('reset'), description: t('resetDesc'), control: resetBtn }),
    ] });
    backup.classList.add('st-upd__backup');
    // Updategeschiedenis' "Herstel" row (Windows' Recovery) opens this page at the backup and reset group. A one-shot
    // param (like bluetooth.js' add): consumed here, so a re-render with the same params (a language switch, a
    // refresh: keepScroll) keeps the user's scroll position instead of jumping back to the group
    if (ctx.params?.section === 'recovery') {
      delete ctx.params.section;
      setTimeout(() => { if (ctx.alive() && backup.isConnected) backup.scrollIntoView({ block: 'start' }); }, 0);
    }

    return [
      upd.hero,
      upd.bars,
      upd.release,
      upd.rowsEl,
      upd.more,
      backup,
      SettingsGroup({ children: [
        SettingsExpander({ icon: 'info', title: t('about'), description: t('aboutDesc'),
          control: h('span', { class: 'st-value' }, sh.version ? `${t('version')} ${sh.version}` : ''),
          children: [h('dl', { class: 'st-specs selectable' }, aboutRows.filter((r) => r[1]).map(([k, v]) => [h('dt', null, k), h('dd', null, v)]))] }),
      ] }),
    ];
  },
};
