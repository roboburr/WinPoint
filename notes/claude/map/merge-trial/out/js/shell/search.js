// Win11 Shell — search (owned by the shell agent). SPEC §11: setup(ctx), flyout 'search',
// commands 'toggle-search' (args {query?, tab?}) and 'run'.
//
// createSearchPanel(ctx, {input?, embedded?, onDone?}) is shared with the Start menu (typing in
// Start turns it into this panel in place). Results: best match, Apps (fuzzy, prefix / word-start
// boosted, Dutch + English keywords and synonyms), Instellingen (settings/registry.js), Acties
// (command palette), Bestanden (index.search, falls back to recent files), Zoeken op internet,
// and "Meer resultaten in Alles". Right pane: details + actions for the selected result.

import { bridge } from '../core/bridge.js';
import { onFlyoutChange } from '../core/flyouts.js';
import { h, debounce, animate, clamp } from '../core/dom.js';
import { icon, hasIcon } from '../core/icons.js';
import { makeT, lang, tr, onLangChange, collator } from '../core/i18n.js';
import { getSetting, setSetting, themeMode, onSettings } from '../core/theme.js';
import { Dialog, Toast } from '../core/components.js';
import { bytes, date as fmtDate, time as fmtTime } from '../core/format.js';
import { showContextMenu } from '../core/contextmenu.js';
import { copyText, whenLabel } from './startmenu.js';
import { searchSettings, SETTINGS_PAGES } from '../apps/settings/registry.js';

const t = makeT({
  nl: {
    placeholder: 'Typ hier om te zoeken', search: 'Zoeken', best: 'Beste overeenkomst', apps: 'Apps', settings: 'Instellingen',
    actions: 'Acties', files: 'Bestanden', web: 'Zoeken op internet', webTab: 'Web', all: 'Alle', recent: 'Recent', topApps: 'Topapps',
    quick: 'Snel zoeken', app: 'App', setting: 'Instelling', action: 'Actie', file: 'Bestand', folder: 'Map',
    webSub: 'Zoekresultaten bekijken', webType: 'Zoeken op internet', moreInAlles: 'Meer resultaten in Alles',
    open: 'Openen', run: 'Uitvoeren', openFolder: 'Map openen in Verkenner', openFile: 'Bestand openen', openLocation: 'Bestandslocatie openen', copyPath: 'Pad kopiëren', copied: 'Pad gekopieerd',
    pinStart: 'Vastmaken aan Start', unpinStart: 'Losmaken van Start', pinTaskbar: 'Vastmaken aan taakbalk',
    unpinTaskbar: 'Losmaken van taakbalk', openBrowser: 'Openen in de browser', location: 'Locatie', modified: 'Gewijzigd',
    size: 'Grootte', searching: 'Bestanden zoeken…', noResults: 'Geen resultaten voor “{q}”',
    noResultsHint: 'Controleer de spelling of probeer andere woorden. Je kunt ook op internet of in Alles zoeken.',
    clear: 'Wissen', runHint: 'Typ de naam van een app, map of opdracht', recentEmpty: 'Je recente apps en bestanden verschijnen hier.',
    trashTitle: 'Prullenbak leegmaken?', trashMsg: 'Alles in de Prullenbak wordt definitief verwijderd. Dit kun je niet ongedaan maken.',
    trashCount: { one: 'Er staat 1 item ({size}) in de Prullenbak.', other: 'Er staan {n} items ({size}) in de Prullenbak.' },
    trashEmpty: 'De Prullenbak is al leeg.', trashDone: 'Prullenbak leeggemaakt', trashConfirm: 'Prullenbak leegmaken',
    failed: 'Dat lukte niet',
    // actions
    a_dark: 'Donkere modus inschakelen', a_dark_d: 'Donkere kleuren voor Windows en apps',
    a_light: 'Lichte modus inschakelen', a_light_d: 'Lichte kleuren voor Windows en apps',
    a_taskmgr: 'Taakbeheer openen', a_taskmgr_d: 'Zie welke apps je computer bezighouden',
    a_trash: 'Prullenbak leegmaken', a_trash_d: 'Maakt ruimte vrij; vraagt eerst om bevestiging',
    a_lock: 'Vergrendelen', a_lock_d: 'Je scherm vergrendelen',
    a_logout: 'Afmelden', a_logout_d: 'Je sessie afsluiten',
    a_shutdown: 'Afsluiten', a_shutdown_d: 'De computer uitzetten',
    a_reboot: 'Opnieuw opstarten', a_reboot_d: 'De computer opnieuw opstarten',
    a_suspend: 'Slaapstand', a_suspend_d: 'De computer laten slapen',
    a_settings: 'Instellingen', a_settings_d: 'Instellingen openen',
    a_cleanup: 'Ruimte vrijmaken', a_cleanup_d: 'Ruim tijdelijke bestanden en de Prullenbak op',
    a_storage: 'Schijfruimte bekijken', a_storage_d: 'Zie wat ruimte inneemt op je schijven',
    a_clipboard: 'Klembordgeschiedenis', a_clipboard_d: 'Eerder gekopieerde tekst en afbeeldingen',
    a_alles: 'Alles zoeken', a_alles_d: 'Elk bestand op je computer vinden',
    a_quick: 'Snelle instellingen', a_quick_d: 'Wifi, geluid, helderheid en meer',
    a_notif: 'Meldingen weergeven', a_notif_d: 'Meldingen en agenda',
    a_dndOn: 'Niet storen inschakelen', a_dndOff: 'Niet storen uitschakelen', a_dnd_d: 'Meldingen tijdelijk stil',
    a_transOn: 'Transparantie-effecten inschakelen', a_transOff: 'Transparantie-effecten uitschakelen', a_trans_d: 'Doorschijnende vensters en menu’s',
    a_taskview: 'Taakweergave', a_taskview_d: 'Alle open vensters bekijken',
    a_desktop: 'Bureaublad weergeven', a_desktop_d: 'Alle vensters minimaliseren',
    a_alignLeft: 'Taakbalk links uitlijnen', a_alignCenter: 'Taakbalk centreren', a_align_d: 'Plaats van Start en de apps op de taakbalk',
    a_explorer: 'Verkenner openen', a_explorer_d: 'Je bestanden en mappen',
    q_dark: 'Donkere modus', q_storage: 'Schijfruimte', q_taskmgr: 'Taakbeheer', q_clip: 'Klembord', q_display: 'Beeldscherm', q_alles: 'Bestanden zoeken',
    // what a quick-search chip types into the box
    homeFolder: 'Persoonlijke map',
    qq_dark: 'donkere modus', qq_storage: 'schijfruimte', qq_taskmgr: 'taakbeheer', qq_clip: 'klembord', qq_display: 'beeldscherm', qq_alles: 'alles zoeken',
  },
  en: {
    placeholder: 'Type here to search', search: 'Search', best: 'Best match', apps: 'Apps', settings: 'Settings',
    actions: 'Actions', files: 'Files', web: 'Search the web', webTab: 'Web', all: 'All', recent: 'Recent', topApps: 'Top apps',
    quick: 'Quick searches', app: 'App', setting: 'Setting', action: 'Action', file: 'File', folder: 'Folder',
    webSub: 'See search results', webType: 'Search the web', moreInAlles: 'More results in Alles',
    open: 'Open', run: 'Run', openFolder: 'Open folder in File Explorer', openFile: 'Open file', openLocation: 'Open file location', copyPath: 'Copy path', copied: 'Path copied',
    pinStart: 'Pin to Start', unpinStart: 'Unpin from Start', pinTaskbar: 'Pin to taskbar',
    unpinTaskbar: 'Unpin from taskbar', openBrowser: 'Open in browser', location: 'Location', modified: 'Modified',
    size: 'Size', searching: 'Searching files…', noResults: 'No results for “{q}”',
    noResultsHint: 'Check the spelling or try other words. You can also search the web or Alles.',
    clear: 'Clear', runHint: 'Type the name of an app, folder or command', recentEmpty: 'Your recent apps and files will appear here.',
    trashTitle: 'Empty the Recycle Bin?', trashMsg: 'Everything in the Recycle Bin will be deleted for good. This cannot be undone.',
    trashCount: { one: 'There is 1 item ({size}) in the Recycle Bin.', other: 'There are {n} items ({size}) in the Recycle Bin.' },
    trashEmpty: 'The Recycle Bin is already empty.', trashDone: 'Recycle Bin emptied', trashConfirm: 'Empty Recycle Bin',
    failed: 'That did not work',
    a_dark: 'Turn on dark mode', a_dark_d: 'Dark colours for Windows and apps',
    a_light: 'Turn on light mode', a_light_d: 'Light colours for Windows and apps',
    a_taskmgr: 'Open Task Manager', a_taskmgr_d: 'See which apps keep your computer busy',
    a_trash: 'Empty Recycle Bin', a_trash_d: 'Frees up space; asks first',
    a_lock: 'Lock', a_lock_d: 'Lock your screen',
    a_logout: 'Sign out', a_logout_d: 'End your session',
    a_shutdown: 'Shut down', a_shutdown_d: 'Turn the computer off',
    a_reboot: 'Restart', a_reboot_d: 'Restart the computer',
    a_suspend: 'Sleep', a_suspend_d: 'Put the computer to sleep',
    a_settings: 'Settings', a_settings_d: 'Open Settings',
    a_cleanup: 'Free up space', a_cleanup_d: 'Clean temporary files and the Recycle Bin',
    a_storage: 'View disk space', a_storage_d: 'See what takes up space on your drives',
    a_clipboard: 'Clipboard history', a_clipboard_d: 'Text and images you copied earlier',
    a_alles: 'Search with Alles', a_alles_d: 'Find any file on your computer',
    a_quick: 'Quick settings', a_quick_d: 'Wi-Fi, sound, brightness and more',
    a_notif: 'Show notifications', a_notif_d: 'Notifications and calendar',
    a_dndOn: 'Turn on do not disturb', a_dndOff: 'Turn off do not disturb', a_dnd_d: 'Silence notifications for now',
    a_transOn: 'Turn on transparency effects', a_transOff: 'Turn off transparency effects', a_trans_d: 'See-through windows and menus',
    a_taskview: 'Task view', a_taskview_d: 'See all open windows',
    a_desktop: 'Show desktop', a_desktop_d: 'Minimise all windows',
    a_alignLeft: 'Align taskbar left', a_alignCenter: 'Centre taskbar', a_align_d: 'Where Start and apps sit on the taskbar',
    a_explorer: 'Open File Explorer', a_explorer_d: 'Your files and folders',
    q_dark: 'Dark mode', q_storage: 'Disk space', q_taskmgr: 'Task Manager', q_clip: 'Clipboard', q_display: 'Display', q_alles: 'Find files',
    homeFolder: 'Home',
    qq_dark: 'dark mode', qq_storage: 'disk space', qq_taskmgr: 'task manager', qq_clip: 'clipboard', qq_display: 'display', qq_alles: 'find files',
  },
});

/* ==========================================================================
   Matching
   ========================================================================== */
const WORD_SPLIT = /[\s\-_.:/()'’,+]+/;
export function normalize(s) {
  return String(s ?? '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '').trim();
}

function editDistance(a, b, max) {
  if (Math.abs(a.length - b.length) > max) return max + 1;
  let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
  let prev2 = null;
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    let rowMin = i;
    for (let j = 1; j <= b.length; j++) {
      let v = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      if (prev2 && i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) v = Math.min(v, prev2[j - 2] + 1);
      cur.push(v);
      if (v < rowMin) rowMin = v;
    }
    if (rowMin > max) return max + 1;
    prev2 = prev;
    prev = cur;
  }
  return prev[b.length];
}

function fuzzy(q, text) {
  let from = 0, gaps = 0, first = -1, last = -1;
  for (const ch of q) {
    if (ch === ' ') continue;
    const j = text.indexOf(ch, from);
    if (j < 0) return 0;
    if (first < 0) first = j;
    if (last >= 0 && j > last + 1) gaps += j - last - 1;
    last = j;
    from = j + 1;
  }
  const s = 46 - gaps * 2 - Math.min(10, first);
  return s > 18 ? s : 0;
}

/**
 * scoreText(normalizedQuery, normalizedText, loose=true) → 0…100
 * exact > prefix > word start > all words > acronym > substring > (loose only) one typo > fuzzy subsequence.
 */
export function scoreText(q, text, loose = true) {
  if (!q || !text) return 0;
  if (text === q) return 100;
  if (text.startsWith(q)) return 92 - Math.min(8, (text.length - q.length) / 3);
  const words = text.split(WORD_SPLIT).filter(Boolean);
  for (let i = 0; i < words.length; i++) if (words[i].startsWith(q)) return 84 - Math.min(6, i * 2);
  const qw = q.split(/\s+/).filter(Boolean);
  if (qw.length > 1 && qw.every((x) => words.some((w) => w.startsWith(x)))) return 78;
  if (q.length >= 2 && !q.includes(' ')) {
    const acr = words.map((w) => w[0]).join('');
    if (acr.length > 1 && acr.startsWith(q)) return 72;
  }
  const idx = text.indexOf(q);
  if (idx >= 0) return 64 - Math.min(14, idx);
  if (!loose) return 0;
  if (q.length >= 6 && !q.includes(' ')) {
    const max = q.length >= 9 ? 2 : 1;
    for (const w of words) if (w.length >= q.length - 1 && editDistance(q, w.slice(0, q.length), max) <= max) return 52;
  }
  if (q.length >= 3) return fuzzy(q, text);
  return 0;
}

/* Synonyms (Dutch + English) so people find apps by what they do or by their Windows name. */
const SYNONYMS = [
  [/WebBrowser|firefox|chrome|chromium|brave|epiphany/i, ['browser', 'internet', 'webbrowser', 'surfen', 'web', 'edge', 'internet explorer']],
  [/FileManager|nautilus|win11shell-explorer|dolphin|thunar/i, ['verkenner', 'explorer', 'bestanden', 'files', 'mappen', 'bestandsbeheer', 'file explorer', 'deze pc']],
  [/TerminalEmulator|ptyxis|gnome-terminal|console|konsole/i, ['terminal', 'opdrachtprompt', 'cmd', 'command prompt', 'powershell', 'console', 'shell', 'opdrachtregel']],
  [/TextEditor|gedit|mousepad|kate/i, ['kladblok', 'notepad', 'teksteditor', 'text editor', 'tekst', 'notities']],
  [/Calculator/i, ['rekenmachine', 'calculator', 'rekenen', 'calc']],
  [/PackageManager|snap-store|gnome\.software|discover/i, ['store', 'winkel', 'microsoft store', 'app center', 'software', 'apps installeren', 'programma installeren']],
  [/win11shell-settings|^Settings$/i, ['configuratiescherm', 'control panel', 'instellingen', 'settings', 'opties']],
  [/win11shell-taskmgr|SystemMonitor|resources/i, ['taakbeheer', 'task manager', 'processen', 'prestaties', 'ctrl alt delete']],
  [/Email|thunderbird|evolution|geary/i, ['mail', 'e-mail', 'email', 'outlook', 'post']],
  [/Calendar/i, ['agenda', 'calendar', 'kalender', 'afspraken']],
  [/loupe|eog|ImageViewer|shotwell|gthumb/i, ["foto's", 'fotos', 'photos', 'afbeeldingen', 'images', 'fotoweergave', 'plaatjes']],
  [/Player|Video|vlc|showtime|totem|celluloid|mpv/i, ['video', 'films', 'media player', 'mediaspeler', 'filmpjes', 'windows media player']],
  [/Audio|Music|rhythmbox|spotify|lollypop/i, ['muziek', 'music', 'liedjes', 'media player']],
  [/WordProcessor|writer/i, ['word', 'tekstverwerker', 'brief', 'document schrijven']],
  [/Spreadsheet|libreoffice-calc/i, ['excel', 'spreadsheet', 'rekenblad', 'werkblad', 'tabel']],
  [/Presentation|impress/i, ['powerpoint', 'presentatie', 'dia', 'slides']],
  [/RasterGraphics|gimp|krita|pinta/i, ['paint', 'tekenen', 'foto bewerken', 'photoshop']],
  [/Game|steam|lutris|heroic/i, ['games', 'spellen', 'spelletjes', 'xbox']],
  [/InstantMessaging|Chat|discord|telegram|whatsapp|signal/i, ['chat', 'berichten', 'teams', 'bellen', 'messenger']],
  [/snapshot|cheese|camera/i, ['camera', 'webcam']],
  [/clocks/i, ['klok', 'wekker', 'alarm', 'timer', 'stopwatch', 'alarmen en klok']],
  [/weather/i, ['weer', 'weather', 'weerbericht']],
  [/win11shell-alles/i, ['everything', 'zoeken', 'bestanden zoeken', 'alles', 'find']],
  [/DiskUtility|baobab/i, ['schijfbeheer', 'schijven', 'disk', 'opslag']],
  [/win11shell-disks/i, ['schijfbeheer', 'disk management', 'diskmgmt', 'partities', 'partitie', 'formatteren', 'format',
    'usb-stick', 'harde schijf', 'ssd', 'volume', 'stations', 'gnome-disks', 'schijven en volumes']],
  [/Archiving|file-?roller|ark/i, ['zip', 'winzip', '7zip', 'winrar', 'uitpakken']],
  [/Characters/i, ['emoji', 'speciale tekens', 'tekenkaart']],
];

/* Settings pages used when ui/js/apps/settings/registry.js is not available: [id, icon, title, keywords], the
 * keywords per language like the registry's (both are searched, as there). */
const FALLBACK_SETTINGS = [
  ['system', 'pc', { nl: 'Systeem', en: 'System' }, { nl: 'computer systeem pc', en: 'computer system pc' }],
  ['display', 'display', { nl: 'Beeldscherm', en: 'Display' },
    { nl: 'resolutie schaal helderheid monitor scherm', en: 'resolution scale brightness monitor display screen' }],
  ['sound', 'sound', { nl: 'Geluid', en: 'Sound' }, { nl: 'volume luidspreker microfoon audio', en: 'volume speakers microphone audio' }],
  ['notifications', 'notification', { nl: 'Meldingen', en: 'Notifications' },
    { nl: 'niet storen meldingen', en: 'do not disturb notifications dnd' }],
  ['storage', 'storage', { nl: 'Opslag', en: 'Storage' }, { nl: 'schijf ruimte opslagbeheer', en: 'disk space storage sense' }],
  ['network', 'globe', { nl: 'Netwerk en internet', en: 'Network & internet' },
    { nl: 'wifi wi-fi ethernet vliegtuigmodus internet', en: 'wifi wi-fi ethernet flight airplane internet' }],
  ['bluetooth', 'bluetooth', { nl: 'Bluetooth en apparaten', en: 'Bluetooth & devices' },
    { nl: 'muis toetsenbord printer koptelefoon', en: 'mouse keyboard printer headphones' }],
  ['personalization', 'personalize', { nl: 'Persoonlijke instellingen', en: 'Personalisation' },
    { nl: 'achtergrond thema kleuren donker licht', en: 'background theme colours colors dark light wallpaper' }],
  ['taskbar', 'taskbar', { nl: 'Taakbalk', en: 'Taskbar' }, { nl: 'taakbalk uitlijnen verbergen', en: 'taskbar alignment hide' }],
  ['start', 'start-menu', { nl: 'Start', en: 'Start' }, { nl: 'startmenu vastgemaakt aanbevolen', en: 'start menu pinned recommended' }],
  ['apps', 'apps', { nl: 'Apps', en: 'Apps' }, { nl: 'installeren verwijderen standaard-apps', en: 'install uninstall default apps' }],
  ['accounts', 'user', { nl: 'Accounts', en: 'Accounts' }, { nl: 'gebruiker account wachtwoord', en: 'user account password' }],
  ['time', 'clock', { nl: 'Tijd en taal', en: 'Time & language' }, { nl: 'datum tijdzone taal', en: 'date time zone language' }],
  ['accessibility', 'accessibility', { nl: 'Toegankelijkheid', en: 'Accessibility' },
    { nl: 'tekstgrootte vergroten animaties', en: 'text size magnifier animations' }],
  ['privacy', 'shield', { nl: 'Privacy en beveiliging', en: 'Privacy & security' },
    { nl: 'privacy beveiliging recente bestanden', en: 'privacy security recent files' }],
  ['update', 'update', { nl: 'Updates', en: 'Updates' }, { nl: 'bijwerken update software', en: 'update updates software' }],
  ['about', 'info', { nl: 'Info', en: 'About' }, { nl: 'pc-naam specificaties over deze computer', en: 'pc name specifications about this computer' }],
].map(([id, ic, title, kw]) => ({ id, title, description: { nl: '', en: '' }, icon: ic, parent: null,
  keywords: { nl: kw.nl.split(' '), en: kw.en.split(' ') } }));

const EXT_MIME = {
  jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml', bmp: 'image/bmp', heic: 'image/heic',
  pdf: 'application/pdf', txt: 'text/plain', md: 'text/markdown', log: 'text/plain', csv: 'text/csv',
  odt: 'application/vnd.oasis.opendocument.text', ods: 'application/vnd.oasis.opendocument.spreadsheet', odp: 'application/vnd.oasis.opendocument.presentation',
  doc: 'application/msword', docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xls: 'application/vnd.ms-excel', xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  ppt: 'application/vnd.ms-powerpoint', pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  mp3: 'audio/mpeg', flac: 'audio/flac', ogg: 'audio/ogg', wav: 'audio/x-wav', m4a: 'audio/mp4', m3u: 'audio/x-mpegurl',
  mp4: 'video/mp4', mkv: 'video/x-matroska', avi: 'video/x-msvideo', mov: 'video/quicktime', webm: 'video/webm',
  zip: 'application/zip', gz: 'application/gzip', tgz: 'application/gzip', tar: 'application/x-tar', '7z': 'application/x-7z-compressed', rar: 'application/vnd.rar',
  deb: 'application/vnd.debian.binary-package', sh: 'application/x-shellscript', py: 'text/x-python', js: 'text/javascript', html: 'text/html', json: 'application/json', iso: 'application/x-cd-image',
};
const IMAGE_EXT = /^(jpe?g|png|gif|webp|bmp|svg|avif|heic|tiff?)$/;

/* ==========================================================================
   setup
   ========================================================================== */
export function setup(ctx) {
  let panel = null;
  let shown = false;
  ctx.registerFlyout('search', {
    slide: 40,
    create(container) {
      panel = createSearchPanel(ctx, { embedded: false, onDone: () => ctx.closeFlyout('search'), onQuery(query) {
        const input = document.querySelector('.sh-tb-searchbox__input');
        if (input && input.value !== query) input.value = query;
      } });
      return {
        el: panel.el,
        onOpen(opts = {}) {
          const again = shown;
          shown = true;
          panel.open({ ...opts, again });
        },
        onClose() {
          shown = false;
          panel.cancelPending(); // a deferred Enter must never fire after the flyout closed
          setTimeout(() => { if (!ctx.isOpen('search')) panel.reset(); }, 200);
        },
      };
    },
  });
  const anchor = () => (getSetting('taskbar.alignment', 'center') === 'left' ? 'left' : 'center');
  let taskbarEpoch = 0;
  ctx.onCommand('taskbar-search', async ({ input } = {}) => {
    if (!(input instanceof HTMLInputElement) || !input.matches('.sh-tb-searchbox__input')) return;
    const epoch = ++taskbarEpoch;
    const selection = [input.selectionStart, input.selectionEnd, input.selectionDirection];
    await ctx.openFlyout('search', { anchor: anchor(), query: input.value, focus: false });
    if (epoch !== taskbarEpoch || !ctx.isOpen('search') || !input.isConnected) return;
    panel.setQuery(input.value);
    input.focus({ preventScroll: true });
    try { input.setSelectionRange(...selection); } catch { /* search inputs may not expose a caret range */ }
  });
  ctx.onCommand('taskbar-search-key', ({ event } = {}) => {
    if (event && ctx.isOpen('search')) panel?.handleKey(event);
  });
  onFlyoutChange(({ name, open }) => { if (name === 'search' && !open) taskbarEpoch++; });
  ctx.onCommand('toggle-search', (args = {}) => {
    if (ctx.isOpen('search') && args.query == null && !args.tab) { ctx.closeFlyout('search'); return; }
    ctx.openFlyout('search', { anchor: anchor(), query: args.query, tab: args.tab });
  });
  ctx.onCommand('run', (args = {}) => ctx.openFlyout('search', { anchor: anchor(), query: args.query ?? '', run: true }));
}

/* ==========================================================================
   Panel
   ========================================================================== */
export function createSearchPanel(ctx, { input: extInput = null, embedded = false, onDone, onQuery } = {}) {
  /* ---------------------------------------------------------------- data */
  let appIndex = [];
  let settingsIndex = [];
  let recentFiles = null;
  let query = '';
  let qn = '';
  let tab = 'all';
  let flat = [];        // selectable results in order
  let sel = 0;
  let fileState = { q: null, items: [], loading: false, fallback: false };
  let fileSeq = 0;
  // Enter pressed while the file hits for this query were still on their way: when no app,
  // setting or action matched, the best result may still become a file, so wait for them
  // (briefly) instead of opening the web search by mistake.
  let pendingRun = null;
  const FILE_WAIT_MS = 900;
  let userMoved = false; // keep the user's choice when late results (files) arrive

  function buildAppIndex() {
    appIndex = ctx.apps.list().map((app) => {
      const hay = `${app.id} ${(app.categories || []).join(' ')}`;
      const syn = SYNONYMS.filter(([re]) => re.test(hay)).flatMap(([, words]) => words);
      return {
        app,
        n: normalize(app.name),
        g: normalize(app.genericName),
        k: [...(app.keywords || []), ...syn].map(normalize).filter(Boolean),
        c: normalize(app.comment),
        id: normalize(String(app.id).replace(/\.desktop$/, '').replace(/[._-]+/g, ' ')),
      };
    });
  }
  buildAppIndex();
  ctx.apps.onChange(() => { buildAppIndex(); if (query) update(); });
  // default apps (browser, mail, …) win ties, like Windows ranks the default browser first
  let defaultIds = new Set();
  bridge.call('apps.defaults').then((d) => {
    defaultIds = new Set(Object.values(d || {}).filter(Boolean).map((x) => x.id));
  }).catch(() => { /* optional */ });

  // Instellingen: the Settings app's own registry + searchSettings() (same results as its search box).
  const registryPages = Array.isArray(SETTINGS_PAGES) && SETTINGS_PAGES.length ? SETTINGS_PAGES : FALLBACK_SETTINGS;
  function buildSettingsIndex(pages) {
    const list = Array.isArray(pages) && pages.length ? pages : FALLBACK_SETTINGS;
    settingsIndex = list.map((p) => ({
      page: p,
      title: tr(p.title) || p.id,
      n: normalize(tr(p.title)),
      ne: normalize(p.title?.en || ''),
      k: [...(p.keywords?.nl || []), ...(p.keywords?.en || [])].map(normalize).filter(Boolean),
      d: normalize(tr(p.description)),
      parent: p.parent ? list.find((x) => x.id === p.parent) : null,
    }));
  }
  buildSettingsIndex(registryPages);
  const settingEntry = (page) => settingsIndex.find((e) => e.page.id === page.id) || {
    page, title: tr(page.title) || page.id, parent: page.parent ? registryPages.find((x) => x.id === page.parent) : null,
  };

  const ACTIONS = [
    { id: 'dark', icon: 'moon', k: 'donker donkere modus dark mode thema nachtmodus zwart', when: () => themeMode() !== 'dark', run: () => setSetting('theme.mode', 'dark') },
    { id: 'light', icon: 'sun', k: 'licht lichte modus light mode thema wit', when: () => themeMode() !== 'light', run: () => setSetting('theme.mode', 'light') },
    { id: 'taskmgr', icon: 'chart', k: 'taakbeheer task manager processen vastgelopen reageert niet ctrl alt delete', run: () => ctx.openApp('taskmgr') },
    { id: 'trash', icon: 'trash', k: 'prullenbak leegmaken empty recycle bin trash verwijderen leeg', run: () => emptyTrash() },
    { id: 'lock', icon: 'lock', k: 'vergrendelen lock scherm op slot', run: () => ctx.runCommand('power', { action: 'lock' }) },
    { id: 'logout', icon: 'signout', k: 'afmelden uitloggen sign out log out', run: () => ctx.runCommand('power', { action: 'logout' }) },
    { id: 'shutdown', icon: 'power', k: 'afsluiten uitzetten shut down power off computer uit', run: () => ctx.runCommand('power', { action: 'shutdown' }) },
    { id: 'reboot', icon: 'restart', k: 'opnieuw opstarten herstarten restart reboot', run: () => ctx.runCommand('power', { action: 'reboot' }) },
    { id: 'suspend', icon: 'sleep', k: 'slaapstand sleep suspend slapen', run: () => ctx.runCommand('power', { action: 'suspend' }) },
    { id: 'settings', icon: 'settings', k: 'instellingen settings configuratiescherm control panel', run: () => ctx.openApp('settings') },
    { id: 'cleanup', icon: 'broom', k: 'ruimte vrijmaken opruimen schijfopruiming disk cleanup free up space schijf vol', run: () => ctx.openApp('taskmgr', { page: 'storage', tab: 'cleanup' }) },
    { id: 'storage', icon: 'storage', k: 'schijfruimte opslag disk space storage schijven', run: () => ctx.openApp('taskmgr', { page: 'storage' }) },
    { id: 'clipboard', icon: 'clipboard', k: 'klembord klembordgeschiedenis clipboard history kopieren plakken', run: () => ctx.runCommand('clipboard') },
    { id: 'alles', icon: 'search', k: 'alles zoeken everything bestanden zoeken find files', run: (q) => ctx.openApp('alles', q ? { query: q } : {}) },
    { id: 'quick', icon: 'settings', k: 'snelle instellingen quick settings wifi geluid helderheid', run: () => ctx.runCommand('quick-settings') },
    { id: 'notif', icon: 'notification', k: 'meldingen notifications agenda kalender berichten', run: () => ctx.runCommand('notifications') },
    { id: 'dnd', icon: 'notification-off', k: 'niet storen do not disturb stil focus meldingen uit', title: () => (getSetting('notifications.dnd', false) ? t('a_dndOff') : t('a_dndOn')), run: () => setSetting('notifications.dnd', !getSetting('notifications.dnd', false)) },
    { id: 'trans', icon: 'theme', k: 'transparantie transparency doorzichtig blur effecten', title: () => (getSetting('theme.transparency', true) ? t('a_transOff') : t('a_transOn')), run: () => setSetting('theme.transparency', !getSetting('theme.transparency', true)) },
    { id: 'taskview', icon: 'taskview', k: 'taakweergave task view vensters alt tab', run: () => ctx.runCommand('task-view') },
    { id: 'desktop', icon: 'desktop', k: 'bureaublad weergeven show desktop minimaliseren', run: () => ctx.runCommand('show-desktop') },
    { id: 'align', icon: 'taskbar', k: 'taakbalk uitlijnen links centreren taskbar alignment left center', title: () => (getSetting('taskbar.alignment', 'center') === 'left' ? t('a_alignCenter') : t('a_alignLeft')), run: () => setSetting('taskbar.alignment', getSetting('taskbar.alignment', 'center') === 'left' ? 'center' : 'left') },
    { id: 'explorer', icon: 'folder', k: 'verkenner openen file explorer bestanden mappen', run: () => ctx.openApp('explorer') },
  ].map((a) => ({ ...a, kn: normalize(a.k).split(' ') }));
  const actionTitle = (a) => (a.title ? a.title() : t(`a_${a.id}`));
  const actionDesc = (a) => t(`a_${a.id}_d`);

  async function emptyTrash() {
    let info = null;
    try { info = await bridge.call('files.trashInfo'); } catch { info = null; }
    if (info && !info.count) { Toast.show({ title: t('trashEmpty'), severity: 'info', timeout: 3000 }); return; }
    const extra = info ? `${t('trashCount', { n: info.count, size: bytes(info.size) })} ` : '';
    const ok = await Dialog.confirm({ title: t('trashTitle'), message: extra + t('trashMsg'), confirmLabel: t('trashConfirm'), danger: true });
    if (!ok) return;
    try {
      await bridge.call('files.emptyTrash');
      Toast.show({ title: t('trashDone'), severity: 'success', timeout: 3000 });
    } catch (e) { ctx.toastError(t('failed'), e); }
  }

  /* ----------------------------------------------------------------- DOM */
  const input = extInput || h('input', {
    class: 'sh-sbox__input', type: 'text', placeholder: t('placeholder'), spellcheck: false, autocomplete: 'off', 'aria-label': t('search'),
  });
  let clearBtn = null;
  let top = null;
  if (!extInput) {
    clearBtn = h('button', { type: 'button', class: 'sh-sbox__clear', tabindex: -1, 'aria-label': t('clear'), hidden: true }, icon('close', { size: 12 }));
    clearBtn.addEventListener('click', (e) => { e.preventDefault(); setQuery(''); input.focus(); });
    top = h('div', { class: 'sh-search__top' },
      h('label', { class: 'sh-sbox' }, h('span', { class: 'sh-sbox__icon' }, icon('search', { size: 16 })), input, clearBtn));
    input.addEventListener('input', () => setQuery(input.value, { fromInput: true }));
    input.addEventListener('keydown', (e) => { handleKey(e); });
  }
  input.setAttribute('aria-controls', 'sh-search-results');
  input.setAttribute('aria-autocomplete', 'list');

  const TABS = ['all', 'apps', 'files', 'settings', 'actions', 'web'];
  const tabsEl = h('div', { class: 'sh-search__tabs', role: 'tablist' });
  function renderTabs() {
    tabsEl.replaceChildren(...TABS.map((id) => {
      const b = h('button', {
        type: 'button', class: ['sh-tab', id === tab && 'is-active'], role: 'tab', tabindex: -1,
        'aria-selected': String(id === tab), dataset: { tab: id },
      }, t(id === 'web' ? 'webTab' : id));
      b.addEventListener('click', () => setTab(id));
      return b;
    }));
  }
  const listEl = h('div', { class: 'sh-sr__list', role: 'listbox', id: 'sh-search-results', 'aria-label': t('search') });
  const detailEl = h('div', { class: 'sh-sr__detail' });
  const results = h('div', { class: 'sh-sr' }, listEl, detailEl);
  const homeEl = h('div', { class: 'sh-search__home' });
  const content = h('div', { class: 'sh-search__content' }, homeEl, results);
  const el = h('div', {
    class: ['sh-search', !embedded && 'sh-acrylic', embedded && 'is-embedded'], role: embedded ? null : 'dialog', 'aria-label': t('search'),
  }, top, tabsEl, content);

  /* -------------------------------------------------------------- search */
  function setQuery(q, { fromInput = false } = {}) {
    cancelPendingRun();
    query = String(q ?? '');
    onQuery?.(query);
    if (!fromInput && input.value !== query && !extInput) input.value = query;
    if (clearBtn) clearBtn.hidden = !query;
    qn = normalize(query);
    sel = 0;
    userMoved = false;
    update();
    if (qn) loadFiles(qn);
  }

  function setTab(id) {
    if (!TABS.includes(id)) return;
    cancelPendingRun();
    tab = id;
    sel = 0;
    userMoved = false;
    renderTabs();
    update();
  }

  const loadFiles = debounce(async (q) => {
    const seq = ++fileSeq;
    fileState = { q, items: fileState.q === q ? fileState.items : [], loading: true, fallback: false };
    update({ keepSel: true });
    let items = [];
    let fallback = false;
    try {
      // channel 'start': a newer query cancels this one in the indexer (error code 'cancelled')
      const r = await bridge.call('index.search', { query: q, limit: tab === 'files' ? 30 : 8, sort: 'name', channel: 'start' });
      items = (r?.items || []).map((f) => ({ path: f.path, name: f.name, dir: f.dir || f.path.replace(/\/[^/]*$/, ''), isDir: !!f.isDir, size: f.size, modified: f.modified, mime: f.mime, ext: f.ext || (f.name.match(/\.([^.]+)$/)?.[1] || '') }));
    } catch (e) {
      if (seq !== fileSeq) return; // superseded by a newer search in this panel
      if (e?.code === 'cancelled') { // superseded elsewhere on the 'start' channel: show no file hits
        if (q === qn) { fileState = { q, items: [], loading: false, fallback: false }; update({ keepSel: true }); }
        return;
      }
      fallback = true;
      if (!recentFiles) {
        try { recentFiles = await bridge.call('apps.recent', { limit: 60 }); } catch { recentFiles = []; }
      }
      items = (recentFiles || []).map((f) => ({ f, s: scoreText(q, normalize(f.name)) })).filter((x) => x.s >= 40)
        .sort((a, b) => b.s - a.s).slice(0, 6)
        .map(({ f }) => ({ path: f.path, name: f.name, dir: f.path.replace(/\/[^/]*$/, ''), isDir: false, modified: f.modified, mime: f.mime, icon: f.icon, ext: (f.name.match(/\.([^.]+)$/)?.[1] || '') }));
    }
    if (seq !== fileSeq || q !== qn) return;
    fileState = { q, items, loading: false, fallback };
    update({ keepSel: true });
    if (pendingRun && pendingRun.q === q) { cancelPendingRun(); runItem(flat[sel]); }
  }, 110);

  const filesPending = () => !!qn && (fileState.loading || fileState.q !== qn);
  function cancelPendingRun() {
    if (pendingRun) { clearTimeout(pendingRun.timer); pendingRun = null; }
  }
  /** Enter: open the selected result — or wait a moment for file hits (see pendingRun). */
  function runSelected() {
    const item = flat[sel];
    if (!item) return false;
    // the web fallback, or a file hit still from the previous query (inside the 110 ms debounce)
    const staleFile = item.kind === 'file' && fileState.q !== qn;
    if ((item.kind === 'web' || staleFile) && !userMoved && filesPending()) {
      cancelPendingRun();
      const q = qn;
      pendingRun = { q, timer: setTimeout(() => { if (pendingRun?.q === q) { pendingRun = null; runItem(flat[sel]); } }, FILE_WAIT_MS) };
      return true;
    }
    runItem(item);
    return true;
  }

  function matchApps(limit) {
    const out = [];
    for (const e of appIndex) {
      let s = scoreText(qn, e.n);
      if (s < 100) {
        s = Math.max(s, scoreText(qn, e.g) * 0.86, scoreText(qn, e.id, false) * 0.7);
        for (const k of e.k) { const v = scoreText(qn, k, false) * 0.84; if (v > s) s = v; }
        if (qn.length >= 3 && e.c.includes(qn)) s = Math.max(s, 40);
      }
      if (s >= 40) out.push({ s: s + 3 + (defaultIds.has(e.app.id) ? 1.5 : 0), kind: 'app', key: `app:${e.app.id}`, app: e.app });
    }
    const coll = collator();
    out.sort((a, b) => b.s - a.s || coll.compare(a.app.name, b.app.name));
    return out.slice(0, limit);
  }
  function matchSettings(limit) {
    let found = [];
    try { found = searchSettings(query.trim(), lang() === 'en' ? 'en' : 'nl', limit) || []; } catch (e) { console.warn('[search] searchSettings failed', e); }
    const words = Math.max(1, qn.split(/\s+/).filter(Boolean).length);
    const out = [];
    for (const { page, score } of found) {
      if (!page?.id || page.id === 'home') continue;
      // searchSettings sums 0–100 per word: title match 50–100, keyword 30–60, description 20
      const s = Math.min(96, (Number(score) || 0) / words);
      if (s >= 20) out.push({ s, kind: 'setting', key: `set:${page.id}`, setting: settingEntry(page) });
    }
    return out.slice(0, limit);
  }
  function matchActions(limit) {
    const out = [];
    for (const a of ACTIONS) {
      if (a.when && !a.when()) continue;
      let s = scoreText(qn, normalize(actionTitle(a)));
      for (const k of a.kn) { const v = scoreText(qn, k, false) * 0.8; if (v > s) s = v; }
      if (qn.includes(' ')) { const joined = a.kn.join(' '); if (joined.includes(qn)) s = Math.max(s, 70); }
      if (s >= 45) out.push({ s, kind: 'action', key: `act:${a.id}`, action: a });
    }
    out.sort((a, b) => b.s - a.s);
    return out.slice(0, limit);
  }
  function fileResults(limit) {
    return fileState.items.slice(0, limit).map((f) => ({ s: 40, kind: 'file', key: `file:${f.path}`, file: f }));
  }
  const webResult = () => ({ s: 1, kind: 'web', key: `web:${query.trim()}`, q: query.trim() });
  /** Win+R style: a typed folder/file path or web address opens directly. */
  function directResult() {
    const raw = query.trim();
    if (/^(https?:\/\/|www\.)\S+$/i.test(raw)) return { s: 99, kind: 'url', key: `url:${raw}`, url: /^www\./i.test(raw) ? `https://${raw}` : raw };
    if (/^(~\/|~$|\/|file:\/\/)/.test(raw)) {
      const home = ctx.user().name ? `/home/${ctx.user().name}` : '';
      let path = raw.replace(/^file:\/\//, '');
      if (path === '~' || path.startsWith('~/')) path = home + path.slice(1);
      try { path = decodeURI(path); } catch { /* keep */ }
      const leaf = path.replace(/\/+$/, '').split('/').pop() || '/';
      return { s: 99, kind: 'path', key: `path:${path}`, path, isDir: path.endsWith('/') || !/\.[a-z0-9]{1,5}$/i.test(leaf), name: leaf };
    }
    return null;
  }
  const allesResult = () => ({ s: 0, kind: 'alles', key: `alles:${query.trim()}`, q: query.trim() });

  /* -------------------------------------------------------------- render */
  function update({ keepSel = false } = {}) {
    const showHome = !qn;
    homeEl.hidden = !showHome || embedded;
    results.hidden = showHome;
    tabsEl.hidden = showHome && !embedded ? false : showHome;
    if (showHome) { listEl.replaceChildren(); detailEl.replaceChildren(); detailKey = null; if (!embedded) renderHome(); flat = []; return; }
    const prevKey = keepSel && userMoved ? flat[sel]?.key : null;
    const sections = [];
    if (tab === 'all') {
      const apps = matchApps(8), sets = matchSettings(5), acts = matchActions(4), files = fileResults(6);
      const cands = [apps[0], sets[0], acts[0]].filter(Boolean).sort((a, b) => b.s - a.s);
      let best = directResult() || (cands[0] && cands[0].s >= 50 ? cands[0] : (files[0] || null));
      if (!best) best = webResult();
      const rest = (arr) => arr.filter((x) => x.key !== best.key);
      sections.push({ id: 'best', title: t('best'), items: [best] });
      if (rest(apps).length) sections.push({ id: 'apps', title: t('apps'), items: rest(apps).slice(0, 5) });
      if (rest(sets).length) sections.push({ id: 'settings', title: t('settings'), items: rest(sets).slice(0, 4) });
      if (rest(acts).length) sections.push({ id: 'actions', title: t('actions'), items: rest(acts).slice(0, 3) });
      if (rest(files).length || fileState.loading) sections.push({ id: 'files', title: t('files'), items: rest(files), loading: fileState.loading && !files.length });
      if (best.kind !== 'web') sections.push({ id: 'web', title: t('web'), items: [webResult()] });
    } else if (tab === 'apps') sections.push({ id: 'apps', title: t('apps'), items: matchApps(40) });
    else if (tab === 'settings') sections.push({ id: 'settings', title: t('settings'), items: matchSettings(30) });
    else if (tab === 'actions') sections.push({ id: 'actions', title: t('actions'), items: matchActions(30) });
    else if (tab === 'files') sections.push({ id: 'files', title: t('files'), items: fileResults(30), loading: fileState.loading });
    else if (tab === 'web') sections.push({ id: 'web', title: t('web'), items: [webResult()] });

    flat = [];
    const nodes = [];
    for (const sec of sections) {
      if (!sec.items.length && !sec.loading) continue;
      nodes.push(h('div', { class: 'sh-sr__head', role: 'presentation' }, sec.title));
      for (const item of sec.items) {
        const i = flat.length;
        flat.push(item);
        nodes.push(row(item, i, sec.id === 'best'));
      }
      if (sec.loading) nodes.push(h('div', { class: 'sh-sr__loading' }, h('span', { class: 'sh-sr__spinner' }), t('searching')));
    }
    if (!flat.length && !fileState.loading) {
      nodes.push(h('div', { class: 'sh-sr__empty' },
        h('div', { class: 'sh-sr__empty-title' }, t('noResults', { q: query.trim() })),
        h('div', { class: 'sh-sr__empty-hint' }, t('noResultsHint'))));
      flat.push(webResult());
      nodes.push(row(flat[0], 0, false));
    }
    if (tab === 'all' || tab === 'files') {
      const al = allesResult();
      const i = flat.length;
      flat.push(al);
      nodes.push(row(al, i, false));
    }
    listEl.replaceChildren(...nodes);
    if (prevKey) { const j = flat.findIndex((x) => x.key === prevKey); sel = j >= 0 ? j : 0; }
    sel = clamp(sel, 0, Math.max(0, flat.length - 1));
    select(sel, { scroll: false });
  }

  function itemIcon(item, size, request) {
    switch (item.kind) {
      case 'app': return ctx.appIcon(item.app.icon, size, { request });
      case 'setting': return glyphTile(hasIcon(item.setting.page.icon) ? item.setting.page.icon : 'settings', size, 'setting');
      case 'action': return glyphTile(item.action.icon, size, 'action');
      case 'web': return glyphTile('globe', size, 'web');
      case 'url': return glyphTile('link', size, 'web');
      case 'path': return item.isDir ? ctx.appIcon('folder', size, { request, glyph: 'folder' }) : fileIcon({ path: item.path, name: item.name, ext: (item.name.match(/\.([^.]+)$/)?.[1] || '') }, size);
      case 'alles': return ctx.appIcon('win11shell-alles', size, { request, glyph: 'search' });
      case 'file': return fileIcon(item.file, size);
      default: return glyphTile('app', size);
    }
  }
  function glyphTile(name, size, kind = '') {
    return h('span', { class: ['sh-glyph-tile', kind && `is-${kind}`], style: { width: `${size}px`, height: `${size}px` } },
      icon(name, { size: Math.round(size * (size > 32 ? 0.5 : 0.66)) }));
  }
  function fileIcon(f, size) {
    const wrap = h('span', { class: 'sh-appicon', style: { width: `${size}px`, height: `${size}px` } });
    const px = Math.round(size * Math.min(3, Math.max(1.5, devicePixelRatio || 1)));
    const mime = f.mime || (f.isDir ? 'inode/directory' : EXT_MIME[String(f.ext || '').toLowerCase()] || 'application/octet-stream');
    let url = '';
    if (f.isDir) url = bridge.iconUrl('folder', px);
    else if (size >= 48 && IMAGE_EXT.test(String(f.ext).toLowerCase())) url = bridge.thumbUrl(f.path, px * 2) || bridge.mimeIconUrl(mime, px);
    else url = f.icon ? bridge.iconUrl(f.icon, px) : bridge.mimeIconUrl(mime, px);
    const img = h('img', { src: url, alt: '', width: size, height: size, draggable: false, class: size >= 48 && IMAGE_EXT.test(String(f.ext).toLowerCase()) ? 'is-thumb' : null });
    let tries = 0;
    img.addEventListener('error', () => {
      tries++;
      const alt = bridge.mimeIconUrl(mime, px);
      if (tries === 1 && alt && img.src !== alt) { img.src = alt; img.classList.remove('is-thumb'); } else wrap.replaceChildren(icon(f.isDir ? 'folder' : 'document', { size: Math.round(size * 0.8) }));
    });
    wrap.append(img);
    return wrap;
  }
  function titleOf(item) {
    switch (item.kind) {
      case 'app': return item.app.name;
      case 'setting': return item.setting.title;
      case 'action': return actionTitle(item.action);
      case 'file': return item.file.name;
      case 'web': return item.q;
      case 'url': return item.url;
      case 'path': return item.path;
      case 'alles': return t('moreInAlles');
      default: return '';
    }
  }
  function typeOf(item) {
    switch (item.kind) {
      case 'app': return t('app');
      case 'setting': return item.setting.parent ? `${t('setting')} · ${tr(item.setting.parent.title)}` : t('setting');
      case 'action': return t('action');
      case 'file': return item.file.isDir ? t('folder') : t('file');
      case 'web': return t('webSub');
      case 'url': return t('openBrowser');
      case 'path': return item.isDir ? t('openFolder') : t('openFile');
      default: return '';
    }
  }
  function prettyDir(dir) {
    const home = (ctx.user().name && `/home/${ctx.user().name}`) || '';
    let d = String(dir || '');
    if (home && (d === home || d.startsWith(`${home}/`))) {
      const rel = d.slice(home.length + 1);
      return rel ? rel.split('/').join(' › ') : t('homeFolder');
    }
    return d || '/';
  }
  function subOf(item, best) {
    if (item.kind === 'file') return prettyDir(item.file.dir);
    if (best) return typeOf(item);
    if (item.kind === 'web') return t('webSub');
    return '';
  }

  function row(item, i, best) {
    const sub = subOf(item, best);
    const r = h('div', {
      class: ['sh-sr-row', best && 'is-best', `is-${item.kind}`], role: 'option', id: `sh-sr-${i}`, dataset: { i },
      'aria-selected': 'false',
    }, h('span', { class: 'sh-sr-row__icon' }, itemIcon(item, best ? 32 : 24, 48)),
    h('span', { class: 'sh-sr-row__text' }, h('span', { class: 'sh-sr-row__title' }, titleOf(item)), sub ? h('span', { class: 'sh-sr-row__sub' }, sub) : null),
    item.kind === 'web' || item.kind === 'alles' ? h('span', { class: 'sh-sr-row__chev' }, icon('chevron-right', { size: 12 })) : null);
    r.addEventListener('click', () => { select(i); runItem(item); });
    r.addEventListener('pointermove', () => r.classList.add('is-hover'));
    r.addEventListener('pointerleave', () => r.classList.remove('is-hover'));
    r.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      select(i);
      const acts = actionsFor(item);
      if (acts.length) showContextMenu({ x: e.clientX, y: e.clientY, items: acts.map((a) => ({ label: a.label, icon: a.icon, action: a.run })) });
    });
    return r;
  }

  function select(i, { scroll = true } = {}) {
    sel = i;
    listEl.querySelectorAll('.sh-sr-row').forEach((r) => {
      const on = Number(r.dataset.i) === i;
      r.classList.toggle('is-selected', on);
      r.setAttribute('aria-selected', String(on));
      if (on && scroll) r.scrollIntoView({ block: 'nearest' });
    });
    input.setAttribute('aria-activedescendant', `sh-sr-${i}`);
    if (flat[i]?.key !== detailKey || !detailEl.firstChild) renderDetail(flat[i]);
  }

  function actionsFor(item) {
    const out = [];
    if (!item) return out;
    switch (item.kind) {
      case 'app': {
        const id = item.app.id;
        out.push({ label: t('open'), icon: 'open', run: () => runItem(item) });
        out.push(ctx.pins.start.has(id)
          ? { label: t('unpinStart'), icon: 'unpin', run: () => ctx.pins.start.remove(id).then(() => renderDetail(item)) }
          : { label: t('pinStart'), icon: 'pin', run: () => ctx.pins.start.add(id).then(() => renderDetail(item)) });
        out.push(ctx.pins.taskbar.has(id)
          ? { label: t('unpinTaskbar'), icon: 'unpin', run: () => ctx.pins.taskbar.remove(id).then(() => renderDetail(item)) }
          : { label: t('pinTaskbar'), icon: 'pin', run: () => ctx.pins.taskbar.add(id).then(() => renderDetail(item)) });
        break;
      }
      case 'setting': out.push({ label: t('open'), icon: 'open', run: () => runItem(item) }); break;
      case 'action': out.push({ label: t('run'), icon: 'play', run: () => runItem(item) }); break;
      case 'file':
        out.push({ label: t('open'), icon: 'open', run: () => runItem(item) });
        out.push({ label: t('openLocation'), icon: 'folder-open', run: () => { done(); ctx.openApp('explorer', { path: item.file.dir, select: item.file.path }); } });
        out.push({ label: t('copyPath'), icon: 'copy', run: async () => { if (await copyText(item.file.path)) Toast.show({ title: t('copied'), severity: 'success', timeout: 2500 }); } });
        break;
      case 'web': out.push({ label: t('openBrowser'), icon: 'external', run: () => runItem(item) }); break;
      case 'url': out.push({ label: t('openBrowser'), icon: 'external', run: () => runItem(item) }); break;
      case 'path':
        out.push({ label: t('open'), icon: 'open', run: () => runItem(item) });
        out.push({ label: t('copyPath'), icon: 'copy', run: async () => { if (await copyText(item.path)) Toast.show({ title: t('copied'), severity: 'success', timeout: 2500 }); } });
        break;
      case 'alles': out.push({ label: t('open'), icon: 'open', run: () => runItem(item) }); break;
      default:
    }
    return out;
  }

  let detailKey = null;
  function renderDetail(item) {
    detailKey = item?.key ?? null;
    if (!item) { detailEl.replaceChildren(); return; }
    const info = [];
    if (item.kind === 'file') {
      info.push([t('location'), prettyDir(item.file.dir)]);
      if (item.file.modified) info.push([t('modified'), `${fmtDate(item.file.modified, 'short')} ${fmtTime(item.file.modified)}`]);
      if (!item.file.isDir && item.file.size != null) info.push([t('size'), bytes(item.file.size)]);
    }
    let desc = '';
    if (item.kind === 'app') desc = item.app.comment || item.app.genericName || '';
    else if (item.kind === 'setting') desc = tr(item.setting.page.description) || '';
    else if (item.kind === 'action') desc = actionDesc(item.action);
    else if (item.kind === 'alles') desc = t('a_alles_d');
    const card = h('div', { class: 'sh-detail' },
      h('div', { class: 'sh-detail__hero' },
        h('div', { class: 'sh-detail__icon' }, itemIcon(item, 64, 96)),
        h('div', { class: 'sh-detail__name' }, titleOf(item)),
        h('div', { class: 'sh-detail__type' }, item.kind === 'web' ? t('webType') : typeOf(item)),
        desc ? h('div', { class: 'sh-detail__desc' }, desc) : null),
      info.length ? h('div', { class: 'sh-detail__info' }, ...info.map(([k, v]) => h('div', { class: 'sh-detail__inforow' }, h('span', { class: 'k' }, k), h('span', { class: 'v' }, v)))) : null,
      h('div', { class: 'sh-detail__actions' }, ...actionsFor(item).map((a) => {
        const b = h('button', { type: 'button', class: 'sh-detail__action', tabindex: -1 }, icon(a.icon, { size: 16 }), h('span', null, a.label));
        b.addEventListener('click', () => a.run());
        return b;
      })));
    detailEl.replaceChildren(card);
  }

  function done() { onDone?.(); }

  function runItem(item) {
    cancelPendingRun(); // whatever runs now replaces a deferred Enter (click, menu, detail button)
    if (!item) return;
    switch (item.kind) {
      case 'app': done(); ctx.apps.launch(item.app.id); break;
      case 'setting': done(); ctx.openApp('settings', { page: item.setting.page.id }); break;
      case 'action': done(); Promise.resolve(item.action.run(query.trim())).catch((e) => ctx.toastError(t('failed'), e)); break;
      case 'file':
        done();
        if (item.file.isDir) ctx.openApp('explorer', { path: item.file.path });
        else bridge.call('files.open', { path: item.file.path }).catch((e) => ctx.toastError(item.file.name, e));
        break;
      case 'url':
        done();
        bridge.call('apps.openUri', { uri: item.url }).catch((e) => ctx.toastError(t('failed'), e));
        break;
      case 'path':
        done();
        if (item.isDir) ctx.openApp('explorer', { path: item.path });
        else bridge.call('files.open', { path: item.path }).catch((e) => ctx.toastError(item.name, e));
        break;
      case 'web': {
        done();
        let engine = String(getSetting('search.webEngine', 'https://duckduckgo.com/?q=') || '');
        if (!/^https?:\/\//i.test(engine)) engine = 'https://duckduckgo.com/?q=';
        bridge.call('apps.openUri', { uri: engine + encodeURIComponent(item.q) }).catch((e) => ctx.toastError(t('failed'), e));
        break;
      }
      case 'alles': done(); ctx.openApp('alles', item.q ? { query: item.q } : {}); break;
      default:
    }
  }

  /* ---------------------------------------------------------- home view */
  async function renderHome() {
    const recentApps = ctx.apps.recentLaunches(5).map(({ app, time }) => ({ kind: 'app', app, time }));
    let files = [];
    try { files = await bridge.call('apps.recent', { limit: 8 }); } catch { files = []; }
    if (qn) return;
    const recent = [...recentApps, ...(files || []).map((f) => ({ kind: 'file', file: { ...f, dir: f.path.replace(/\/[^/]*$/, ''), ext: f.name.match(/\.([^.]+)$/)?.[1] || '' }, time: f.modified }))]
      .sort((a, b) => (b.time || 0) - (a.time || 0)).slice(0, 8);
    const left = h('div', { class: 'sh-home__col' }, h('div', { class: 'sh-sr__head' }, t('recent')),
      recent.length ? recent.map((it) => {
        const r = h('button', { type: 'button', class: 'sh-home-row', tabindex: -1 },
          it.kind === 'app' ? ctx.appIcon(it.app.icon, 24, { request: 32 }) : fileIcon(it.file, 24),
          h('span', { class: 'sh-home-row__text' }, h('span', { class: 'sh-home-row__title' }, it.kind === 'app' ? it.app.name : it.file.name),
            h('span', { class: 'sh-home-row__sub' }, it.kind === 'app' ? t('app') : `${prettyDir(it.file.dir)}`)),
          h('span', { class: 'sh-home-row__time' }, whenLabel(it.time)));
        r.addEventListener('click', () => runItem(it.kind === 'app' ? { kind: 'app', app: it.app } : { kind: 'file', file: it.file }));
        return r;
      }) : h('div', { class: 'sh-home__empty' }, t('recentEmpty')));
    const tops = ctx.apps.topApps(6);
    const quick = [
      ['q_dark', 'moon'], ['q_storage', 'storage'], ['q_taskmgr', 'chart'],
      ['q_clip', 'clipboard'], ['q_display', 'display'], ['q_alles', 'search'],
    ];
    const right = h('div', { class: 'sh-home__col' },
      h('div', { class: 'sh-sr__head' }, t('topApps')),
      h('div', { class: 'sh-topapps' }, ...tops.map((app) => {
        const b = h('button', { type: 'button', class: 'sh-topapp', tabindex: -1 }, ctx.appIcon(app.icon, 32, { request: 48 }), h('span', { class: 'sh-topapp__name' }, app.name));
        b.addEventListener('click', () => runItem({ kind: 'app', app }));
        return b;
      })),
      h('div', { class: 'sh-sr__head' }, t('quick')),
      h('div', { class: 'sh-quick' }, ...quick.map(([key, ic]) => {
        const b = h('button', { type: 'button', class: 'sh-chip', tabindex: -1 }, icon(ic, { size: 16 }), h('span', null, t(key)));
        b.addEventListener('click', () => { const v = t(`q${key}`); if (!extInput) input.value = v; setQuery(v); input.focus(); });
        return b;
      })));
    homeEl.replaceChildren(left, right);
  }

  /* ------------------------------------------------------------ keyboard */
  function handleKey(e) {
    if (e.defaultPrevented) return false;
    if (e.ctrlKey && e.key === 'Tab') {
      e.preventDefault();
      setTab(TABS[(TABS.indexOf(tab) + (e.shiftKey ? -1 : 1) + TABS.length) % TABS.length]);
      return true;
    }
    if (!qn || !flat.length) return false;
    if (/^(ArrowDown|ArrowUp|PageDown|PageUp)$/.test(e.key)) cancelPendingRun();
    if (e.key === 'ArrowDown') { e.preventDefault(); userMoved = true; select(Math.min(flat.length - 1, sel + 1)); return true; }
    if (e.key === 'ArrowUp') { e.preventDefault(); userMoved = true; select(Math.max(0, sel - 1)); return true; }
    if (e.key === 'PageDown') { e.preventDefault(); userMoved = true; select(Math.min(flat.length - 1, sel + 5)); return true; }
    if (e.key === 'PageUp') { e.preventDefault(); userMoved = true; select(Math.max(0, sel - 5)); return true; }
    if (e.key === 'Enter') { e.preventDefault(); runSelected(); return true; }
    if (e.key === 'Tab' && !e.shiftKey && document.activeElement === input) {
      const first = detailEl.querySelector('.sh-detail__action');
      if (first) { e.preventDefault(); first.focus(); return true; }
    }
    return false;
  }
  detailEl.addEventListener('keydown', (e) => {
    const btns = [...detailEl.querySelectorAll('.sh-detail__action')];
    const i = btns.indexOf(document.activeElement);
    if (i < 0) return;
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); btns[clamp(i + (e.key === 'ArrowDown' ? 1 : -1), 0, btns.length - 1)].focus(); }
    else if (e.key === 'Tab' && e.shiftKey) { e.preventDefault(); input.focus(); }
    else if (e.key === 'ArrowLeft') { e.preventDefault(); input.focus(); }
  });
  el.addEventListener('keydown', (e) => {
    if (extInput || document.activeElement === input) return;
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      input.focus();
      input.value += e.key;
      setQuery(input.value, { fromInput: true });
    }
  });

  onLangChange(() => {
    input.placeholder = t('placeholder');
    buildSettingsIndex(registryPages);
    detailKey = null;
    renderTabs();
    update();
  });
  onSettings((s, path) => { if (query && (!path || /^(theme|notifications|taskbar)/.test(path))) update({ keepSel: true }); });

  renderTabs();
  update();

  return {
    el,
    input,
    handleKey,
    setQuery(q) {
      if (!extInput) input.value = q;
      setQuery(q, { fromInput: !!extInput });
    },
    getQuery: () => query,
    activate: () => runSelected(),
    cancelPending: () => cancelPendingRun(),
    open(opts = {}) {
      if (opts.tab) { tab = opts.tab; renderTabs(); }
      input.placeholder = opts.run ? t('runHint') : t('placeholder');
      if (opts.query != null) { input.value = opts.query; setQuery(opts.query, { fromInput: true }); }
      else update();
      for (const part of opts.again ? [] : [top, tabsEl, content]) {
        if (part) animate(part, [{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], { duration: 300, delay: 30 });
      }
      requestAnimationFrame(() => {
        if (opts.focus === false) return;
        input.focus({ preventScroll: true });
        const n = input.value.length;
        try { input.setSelectionRange(n, n); } catch { /* ignore */ }
      });
    },
    reset() {
      cancelPendingRun();
      tab = 'all';
      renderTabs();
      if (!extInput) input.value = '';
      setQuery('', { fromInput: true });
      recentFiles = null;
    },
    focus() { input.focus(); },
  };
}
