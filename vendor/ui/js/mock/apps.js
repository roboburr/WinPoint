// Win11 Shell — mock apps.* (owned by the shell agent). SPEC §6.3.
// ~50 realistic Ubuntu 26.04 apps (snap / deb / flatpak style ids, Dutch names like a
// nl_NL desktop would show), recent files, "open with", default apps.
// apps.launch spawns a fake window through mock/windows.js (m.state.mockWindows).
// Shapes mirror host/services/apps.py: App extras `own`, `added`/`installedAt` (ms, 0 = unknown),
// apps.recent items with `isDir` + `app`, apps.forFile lists Verkenner first for folders,
// apps.defaults {candidates:true}, apps.clearRecent (empties the history, emits apps.changed).

const DAY0 = 86400e3;
const A = (id, name, icon, categories, { g = '', c = '', k = [], added = 0 } = {}) => ({
  id, name, genericName: g, comment: c, icon, categories, keywords: k, noDisplay: false,
  own: /^win11shell-/.test(id), added: added ? Date.now() - added * DAY0 : 0, installedAt: added ? Date.now() - added * DAY0 : 0,
});

const APPS = [
  // our own apps (SPEC §5)
  A('win11shell-settings.desktop', 'Instellingen', 'win11shell-settings', ['Settings', 'System'],
    { g: 'Instellingen', c: 'Pas je computer aan', k: ['instellingen', 'settings', 'configuratiescherm', 'control panel', 'voorkeuren'] }),
  A('win11shell-taskmgr.desktop', 'Taakbeheer', 'win11shell-taskmgr', ['System', 'Monitor'],
    { g: 'Taakbeheer', c: 'Bekijk wat je computer aan het doen is', k: ['task manager', 'processen', 'prestaties', 'cpu', 'geheugen'] }),
  A('win11shell-explorer.desktop', 'Verkenner', 'win11shell-explorer', ['System', 'FileManager'],
    { g: 'Bestandsbeheer', c: 'Bekijk en beheer je bestanden en mappen', k: ['bestanden', 'files', 'explorer', 'mappen'] }),
  A('win11shell-alles.desktop', 'Alles', 'win11shell-alles', ['Utility'],
    { g: 'Bestanden zoeken', c: 'Vind elk bestand op je computer in een oogwenk', k: ['everything', 'zoeken', 'search', 'vinden'] }),
  A('win11shell-disks.desktop', 'Schijven', 'win11shell-disks', ['System'],
    { g: 'Schijfbeheer', c: 'Bekijk je schijven, koppel ze, formatteer ze en verwijder ze veilig', k: ['schijfbeheer', 'disk management', 'partities', 'formatteren', 'usb-stick'] }),

  // browsers & internet
  A('firefox_firefox.desktop', 'Firefox', 'firefox', ['Network', 'WebBrowser'],
    { g: 'Webbrowser', c: 'Surf op het internet', k: ['internet', 'www', 'browser', 'web'] }),
  A('google-chrome.desktop', 'Google Chrome', 'google-chrome', ['Network', 'WebBrowser'],
    { g: 'Webbrowser', c: 'Toegang tot het internet', k: ['internet', 'browser'] }),
  A('chromium_chromium.desktop', 'Chromium', 'chromium', ['Network', 'WebBrowser'],
    { g: 'Webbrowser', c: 'Toegang tot het internet', k: ['browser'] }),
  A('brave_brave.desktop', 'Brave Web Browser', 'brave-browser', ['Network', 'WebBrowser'],
    { g: 'Webbrowser', c: 'Browse sneller en veiliger', k: ['browser', 'privacy'] }),
  A('thunderbird_thunderbird.desktop', 'Thunderbird', 'thunderbird', ['Network', 'Email'],
    { g: 'E-mailprogramma', c: 'Lees en schrijf e-mail', k: ['mail', 'e-mail', 'post'] }),
  A('discord_discord.desktop', 'Discord', 'discord', ['Network', 'InstantMessaging'],
    { g: 'Chat', c: 'Praat en chat met vrienden', k: ['chat', 'voice'] }),
  A('telegram-desktop_telegram-desktop.desktop', 'Telegram Desktop', 'telegram', ['Network', 'InstantMessaging'],
    { g: 'Berichten', c: 'Snelle en veilige berichten', k: ['chat', 'berichten'] }),
  A('whatsapp-desktop-linux_whatsapp-desktop-linux.desktop', 'WhatsApp', 'whatsapp', ['Network', 'InstantMessaging'],
    { g: 'Berichten', c: 'Chat met familie en vrienden', k: ['chat', 'berichten', 'bellen'] }),
  A('nordvpn_nordvpn-gui.desktop', 'NordVPN', 'nordvpn', ['Network'],
    { g: 'VPN', c: 'Beveilig je internetverbinding', k: ['vpn'] }),

  // office
  A('libreoffice-startcenter.desktop', 'LibreOffice', 'libreoffice-startcenter', ['Office'],
    { g: 'Kantoorpakket', c: 'Start een nieuw document', k: ['office'] }),
  A('libreoffice-writer.desktop', 'LibreOffice Writer', 'libreoffice-writer', ['Office', 'WordProcessor'],
    { g: 'Tekstverwerker', c: 'Maak en bewerk tekstdocumenten', k: ['word', 'brief', 'document'] }),
  A('libreoffice-calc.desktop', 'LibreOffice Calc', 'libreoffice-calc', ['Office', 'Spreadsheet'],
    { g: 'Rekenblad', c: 'Werk met rekenbladen en tabellen', k: ['excel', 'spreadsheet', 'tabel'] }),
  A('libreoffice-impress.desktop', 'LibreOffice Impress', 'libreoffice-impress', ['Office', 'Presentation'],
    { g: 'Presentaties', c: 'Maak presentaties', k: ['powerpoint', 'dia'] }),
  A('libreoffice-draw.desktop', 'LibreOffice Draw', 'libreoffice-draw', ['Office', 'Graphics'],
    { g: 'Tekenprogramma', c: 'Maak tekeningen en diagrammen', k: ['tekenen', 'diagram'] }),
  A('org.gnome.Papers.desktop', 'Papers', 'org.gnome.Papers', ['Office', 'Viewer'],
    { g: 'Documentweergave', c: 'Bekijk pdf-bestanden en andere documenten', k: ['pdf', 'lezen'] }),

  // gnome core
  A('org.gnome.Nautilus.desktop', 'Bestanden', 'org.gnome.Nautilus', ['GNOME', 'System', 'FileManager'],
    { g: 'Bestandsbeheer', c: 'Toegang tot en ordening van bestanden', k: ['map', 'bestanden', 'nautilus'] }),
  A('org.gnome.TextEditor.desktop', 'Teksteditor', 'org.gnome.TextEditor', ['Utility', 'TextEditor'],
    { g: 'Teksteditor', c: 'Tekstbestanden bewerken', k: ['tekst', 'kladblok', 'notepad'] }),
  A('org.gnome.Calculator.desktop', 'Rekenmachine', 'org.gnome.Calculator', ['Utility', 'Calculator'],
    { g: 'Rekenmachine', c: 'Rekenkundige, wetenschappelijke of financiële berekeningen uitvoeren', k: ['rekenen', 'calculator'] }),
  A('org.gnome.Ptyxis.desktop', 'Terminal', 'org.gnome.Ptyxis', ['System', 'TerminalEmulator'],
    { g: 'Terminal', c: 'Opdrachten uitvoeren', k: ['shell', 'prompt', 'opdrachtregel', 'command line'] }),
  A('org.gnome.Calendar.desktop', 'Agenda', 'org.gnome.Calendar', ['GNOME', 'Office', 'Calendar'],
    { g: 'Agenda', c: 'Beheer je afspraken', k: ['kalender', 'afspraak'] }),
  A('org.gnome.Loupe.desktop', 'Afbeeldingenviewer', 'org.gnome.Loupe', ['Graphics', 'Viewer'],
    { g: 'Afbeeldingenviewer', c: "Foto's en afbeeldingen bekijken", k: ['foto', 'afbeelding', 'plaatje'] }),
  A('org.gnome.Showtime.desktop', "Video's", 'org.gnome.Showtime', ['AudioVideo', 'Video', 'Player'],
    { g: 'Videospeler', c: 'Films en filmpjes afspelen', k: ['film', 'video', 'speler'] }),
  A('org.gnome.Rhythmbox3.desktop', 'Rhythmbox', 'org.gnome.Rhythmbox3', ['AudioVideo', 'Audio', 'Player'],
    { g: 'Muziekspeler', c: 'Je muziekverzameling afspelen en ordenen', k: ['muziek', 'mp3', 'radio'] }),
  A('org.gnome.Snapshot.desktop', 'Camera', 'org.gnome.Snapshot', ['AudioVideo', 'Video'],
    { g: 'Camera', c: "Foto's en video's maken", k: ['webcam', 'foto'] }),
  A('org.gnome.clocks.desktop', 'Klokken', 'org.gnome.clocks', ['GNOME', 'Utility', 'Clock'],
    { g: 'Klokken', c: 'Klokken voor wereldtijden, wekkers, stopwatch en timer', k: ['wekker', 'alarm', 'timer'] }),
  A('org.gnome.Weather.desktop', 'Weer', 'org.gnome.Weather', ['GNOME', 'Utility'],
    { g: 'Weer', c: 'Toont het weer voor je locatie', k: ['weerbericht', 'regen', 'temperatuur'] }),
  A('org.gnome.Characters.desktop', 'Tekens', 'org.gnome.Characters', ['GNOME', 'Utility'],
    { g: 'Tekentabel', c: 'Speciale tekens en emoji zoeken en invoegen', k: ['emoji', 'symbool', 'teken'] }),
  A('org.gnome.FileRoller.desktop', 'Archiefbeheer', 'org.gnome.FileRoller', ['GNOME', 'Utility', 'Archiving'],
    { g: 'Archiefbeheer', c: 'Archieven aanmaken en wijzigen', k: ['zip', 'uitpakken', 'inpakken', 'rar'] }),
  A('org.gnome.DiskUtility.desktop', 'Schijven (GNOME)', 'org.gnome.DiskUtility', ['GNOME', 'System'],
    { g: 'Schijfbeheer', c: 'Schijven en media beheren', k: ['partitie', 'formatteren', 'usb'] }),
  A('org.gnome.baobab.desktop', 'Schijfgebruikanalyse', 'org.gnome.baobab', ['GNOME', 'System'],
    { g: 'Schijfgebruik', c: 'Zien wat ruimte inneemt op je schijven', k: ['opslag', 'ruimte', 'schijf vol'] }),
  A('org.gnome.Logs.desktop', 'Logboeken', 'org.gnome.Logs', ['GNOME', 'System', 'Monitor'],
    { g: 'Logboekweergave', c: 'Systeemlogboeken bekijken', k: ['log', 'fouten', 'journal'] }),
  A('org.gnome.SystemMonitor.desktop', 'Systeemmonitor', 'org.gnome.SystemMonitor', ['GNOME', 'System', 'Monitor'],
    { g: 'Systeemmonitor', c: 'Huidige processen bekijken en systeemstatus controleren', k: ['processen', 'cpu'] }),
  A('yelp.desktop', 'Help', 'help-browser', ['GNOME', 'Core'],
    { g: 'Hulp', c: 'Hulp bij het gebruik van je computer', k: ['hulp', 'handleiding', 'uitleg'] }),
  A('snap-store_snap-store.desktop', 'App Center', 'snap-store', ['System', 'PackageManager'],
    { g: 'Software', c: 'Apps zoeken en installeren', k: ['store', 'winkel', 'installeren', 'software'] }),
  A('firmware-updater_firmware-updater-app.desktop', 'Firmware-updater', 'firmware-updater', ['System'],
    { g: 'Firmware bijwerken', c: 'Firmware van je apparaten bijwerken', k: ['firmware', 'update', 'bios'] }),
  A('nvidia-settings.desktop', 'NVIDIA X Server Settings', 'nvidia-settings', ['System', 'Settings'],
    { g: 'Videokaartinstellingen', c: 'De NVIDIA-videokaart instellen', k: ['gpu', 'videokaart'] }),

  // media & creative
  A('vlc_vlc.desktop', 'VLC media player', 'vlc', ['AudioVideo', 'Player', 'Video'],
    { g: 'Mediaspeler', c: 'Lees, neem op, en stream verschillende multimediabestanden', k: ['film', 'muziek', 'dvd'] }),
  A('mpv_mpv.desktop', 'mpv Media Player', 'mpv', ['AudioVideo', 'Player'],
    { g: 'Mediaspeler', c: 'Video- en audiobestanden afspelen', k: ['video'] }),
  A('spotify_spotify.desktop', 'Spotify', 'spotify-client', ['Audio', 'Music', 'Player'],
    { g: 'Muziekspeler', c: 'Muziek voor iedereen', k: ['muziek', 'podcast'] }),
  A('org.gimp.GIMP.desktop', 'GNU Image Manipulation Program', 'gimp', ['Graphics', 'RasterGraphics'],
    { g: 'Afbeeldingsbewerker', c: "Foto's bewerken en afbeeldingen maken", k: ['gimp', 'paint', 'photoshop', 'foto bewerken'] }),
  A('blender.desktop', 'Blender', 'blender', ['Graphics', '3DGraphics'],
    { g: '3D-modelleren', c: '3D-modellen en animaties maken', k: ['3d', 'animatie'] }),
  A('com.obsproject.Studio.desktop', 'OBS Studio', 'com.obsproject.Studio', ['AudioVideo', 'Recorder'],
    { g: 'Opnemen en streamen', c: 'Je scherm opnemen en live streamen', k: ['opnemen', 'stream', 'schermopname'], added: 5 }),
  A('org.kde.kdenlive.desktop', 'Kdenlive', 'kdenlive', ['AudioVideo', 'AudioVideoEditing'],
    { g: 'Videobewerker', c: 'Video’s monteren', k: ['video bewerken', 'montage'], added: 2 }),

  // games & dev
  A('steam.desktop', 'Steam', 'steam', ['Game'],
    { g: 'Gamebibliotheek', c: 'Games spelen en kopen', k: ['games', 'spellen'] }),
  A('org.gnome.Mines.desktop', 'Mijnenveger', 'org.gnome.Mines', ['Game', 'LogicGame'],
    { g: 'Puzzelspel', c: 'Ruim de verborgen mijnen op', k: ['minesweeper', 'spel'] }),
  A('org.gnome.Solitaire.desktop', 'Patience', 'org.gnome.Aisleriot', ['Game', 'CardGame'],
    { g: 'Kaartspel', c: 'Speel patience-kaartspellen', k: ['solitaire', 'kaarten', 'spel'] }),
  A('code_code.desktop', 'Visual Studio Code', 'vscode', ['Development', 'IDE', 'TextEditor'],
    { g: 'Code-editor', c: 'Code bewerken en debuggen', k: ['code', 'programmeren', 'vscode'] }),
  A('org.gnome.Boxes.desktop', 'Boxes', 'org.gnome.Boxes', ['System', 'Emulator'],
    { g: 'Virtuele machines', c: 'Virtuele machines bekijken en gebruiken', k: ['vm', 'virtueel'] }),
];

const COLL = new Intl.Collator('nl', { sensitivity: 'base', numeric: true });
const SORTED = [...APPS].sort((a, b) => COLL.compare(a.name, b.name));
const byId = new Map(APPS.map((a) => [a.id, a]));

const HOME = '/home/robin';
const MIN = 60e3, HOUR = 3600e3, DAY = 86400e3;
const RECENT = [
  ['Documenten/Begroting 2026.ods', 'application/vnd.oasis.opendocument.spreadsheet', 'x-office-spreadsheet', 14 * MIN],
  ['Afbeeldingen/Vakantie Zeeland/Strand bij Domburg.jpg', 'image/jpeg', 'image-jpeg', 2 * HOUR + 11 * MIN],
  ['Documenten/Sollicitatiebrief.odt', 'application/vnd.oasis.opendocument.text', 'x-office-document', 26 * HOUR],
  ['Downloads/Handleiding wasmachine.pdf', 'application/pdf', 'application-pdf', 2 * DAY + 3 * HOUR],
  ['Documenten/Boodschappenlijst.txt', 'text/plain', 'text-x-generic', 3 * DAY],
  ["Video's/Verjaardag oma.mp4", 'video/mp4', 'video-x-generic', 5 * DAY],
  ['Documenten/Presentatie vereniging.odp', 'application/vnd.oasis.opendocument.presentation', 'x-office-presentation', 8 * DAY],
  ['Muziek/Zomer 2026.m3u', 'audio/x-mpegurl', 'audio-x-generic', 11 * DAY],
  ["Downloads/Foto's familiedag.zip", 'application/zip', 'package-x-generic', 15 * DAY],
  ['Documenten/Notities/Recepten.md', 'text/markdown', 'text-x-generic', 21 * DAY],
  ['Afbeeldingen/Schermafbeeldingen/Schermafbeelding 2026-10-01.png', 'image/png', 'image-png', 24 * DAY],
  ['Documenten/Belastingaangifte 2025.pdf', 'application/pdf', 'application-pdf', 33 * DAY],
];
const T0 = Date.now();

const RECENT_APP = {
  'application/vnd.oasis.opendocument.spreadsheet': 'libreoffice-calc.desktop', 'image/jpeg': 'org.gnome.Loupe.desktop',
  'application/vnd.oasis.opendocument.text': 'libreoffice-writer.desktop', 'application/pdf': 'org.gnome.Papers.desktop',
  'text/plain': 'org.gnome.TextEditor.desktop', 'video/mp4': 'org.gnome.Showtime.desktop', 'image/png': 'org.gnome.Loupe.desktop',
  'application/vnd.oasis.opendocument.presentation': 'libreoffice-impress.desktop', 'audio/x-mpegurl': 'org.gnome.Rhythmbox3.desktop',
  'application/zip': 'org.gnome.FileRoller.desktop', 'text/markdown': 'org.gnome.TextEditor.desktop',
};
let recentCleared = false;
function recentItems() {
  if (recentCleared) return [];
  return RECENT.map(([rel, mime, icon, ago]) => {
    const path = `${HOME}/${rel}`;
    return { path, uri: 'file://' + encodeURI(path), name: rel.split('/').pop(), mime, icon, modified: T0 - ago, isDir: false, app: RECENT_APP[mime] || null };
  });
}

const ext = (p) => (String(p).match(/\.([a-z0-9]+)$/i)?.[1] || '').toLowerCase();
const FOR_EXT = {
  pdf: ['org.gnome.Papers.desktop', 'firefox_firefox.desktop', 'libreoffice-draw.desktop'],
  jpg: ['org.gnome.Loupe.desktop', 'org.gimp.GIMP.desktop', 'firefox_firefox.desktop'],
  jpeg: ['org.gnome.Loupe.desktop', 'org.gimp.GIMP.desktop'],
  png: ['org.gnome.Loupe.desktop', 'org.gimp.GIMP.desktop', 'firefox_firefox.desktop'],
  txt: ['org.gnome.TextEditor.desktop', 'code_code.desktop', 'libreoffice-writer.desktop'],
  md: ['org.gnome.TextEditor.desktop', 'code_code.desktop'],
  odt: ['libreoffice-writer.desktop', 'org.gnome.TextEditor.desktop'],
  docx: ['libreoffice-writer.desktop'],
  ods: ['libreoffice-calc.desktop'],
  xlsx: ['libreoffice-calc.desktop'],
  odp: ['libreoffice-impress.desktop'],
  mp4: ['org.gnome.Showtime.desktop', 'vlc_vlc.desktop', 'mpv_mpv.desktop'],
  mkv: ['vlc_vlc.desktop', 'org.gnome.Showtime.desktop', 'mpv_mpv.desktop'],
  mp3: ['org.gnome.Rhythmbox3.desktop', 'vlc_vlc.desktop'],
  m3u: ['org.gnome.Rhythmbox3.desktop', 'vlc_vlc.desktop'],
  zip: ['org.gnome.FileRoller.desktop'],
  html: ['firefox_firefox.desktop', 'google-chrome.desktop', 'code_code.desktop'],
};

const DEFAULTS = {
  browser: 'firefox_firefox.desktop', mail: 'thunderbird_thunderbird.desktop', files: 'org.gnome.Nautilus.desktop',
  music: 'org.gnome.Rhythmbox3.desktop', video: 'org.gnome.Showtime.desktop', image: 'org.gnome.Loupe.desktop',
  text: 'org.gnome.TextEditor.desktop', terminal: 'org.gnome.Ptyxis.desktop',
};
const defaults = { ...DEFAULTS };
const CANDIDATE_CATS = {
  browser: ['WebBrowser'], mail: ['Email'], files: ['FileManager'], music: ['Audio', 'Music'],
  video: ['Video'], image: ['Viewer', 'RasterGraphics'], text: ['TextEditor'], terminal: ['TerminalEmulator'],
};

const OWN = /^win11shell-(settings|taskmgr|explorer|alles|disks)\.desktop$/;
// the mime types apps.setDefault writes per kind (host/services/apps.py DEFAULT_KINDS)
const KIND_TYPES = {
  browser: ['x-scheme-handler/http', 'x-scheme-handler/https', 'text/html', 'application/xhtml+xml'],
  mail: ['x-scheme-handler/mailto'], files: ['inode/directory', 'application/x-gnome-saved-search'],
  music: ['audio/mpeg'], video: ['video/mp4'], image: ['image/png'], text: ['text/plain'], terminal: ['terminal'],
};

/** host/explorer_args.py (how='auto') in short: file:// URIs / paths → {path, select?, tabs?}. */
async function explorerParams(uris, m) {
  const entries = [];
  const byFolder = new Map();
  for (const u of uris.slice(0, 2000)) {
    let p = typeof u === 'string' ? u : '';
    if (p.startsWith('file://')) { try { p = decodeURIComponent(new URL(p).pathname); } catch { p = ''; } }
    if (!p.startsWith('/')) continue;
    let isDir = false;
    try { isDir = !!(await m.call('files.stat', { path: p }))?.isDir; } catch { isDir = !/\.[^/]+$/.test(p); }
    const folder = isDir ? p : (p.slice(0, p.lastIndexOf('/')) || '/');
    let e = byFolder.get(folder);
    if (!e) { e = { path: folder, select: [] }; byFolder.set(folder, e); entries.push(e); }
    if (!isDir && !e.select.includes(p)) e.select.push(p);
  }
  const one = (e) => (e.select.length ? { path: e.path, select: e.select.slice(0, 500) } : { path: e.path });
  if (!entries.length) return {};
  const out = one(entries[0]);
  if (entries.length > 1) out.tabs = entries.slice(1, 20).map(one);
  return out;
}

function err(code, message) { const e = new Error(message); e.code = code; return e; }

// explorer.ready (host/services/filemanager1.py NavBacklog): requests a just-started Verkenner window may have
// missed. Mock: none, unless a check page passes ?mocknav=[params…] (&mocknavdelay=ms before explorer.backlog).
let mockNav = null;
function mockNavList(m) {
  if (mockNav) return mockNav;
  let list = [];
  try { list = JSON.parse(m.params?.get?.('mocknav') || '[]'); } catch { list = []; }
  mockNav = (Array.isArray(list) ? list : []).filter((p) => p && typeof p === 'object').slice(0, 16)
    .map((params, i) => ({ seq: i + 1, params }));
  return mockNav;
}
const pick = (a) => (a ? { id: a.id, name: a.name, icon: a.icon } : null);

export default {
  methods: {
    'apps.list': async () => SORTED.filter((a) => !a.noDisplay),

    'apps.launch': async ({ id, uris } = {}, m) => {
      const app = byId.get(id);
      if (!app) throw err('not_found', 'Deze app is niet gevonden. Misschien is hij verwijderd.');
      const own = OWN.exec(id);
      if (own) {
        // like host/services/apps.py _own_params: Verkenner opens folders as themselves, files in their folder
        // (selected), more folders as tabs (host/explorer_args.py)
        const params = own[1] === 'explorer' && Array.isArray(uris) && uris.length ? await explorerParams(uris, m) : {};
        m.emit('shell.openApp', { app: own[1], params });
        return {};
      }
      m.state.mockWindows?.spawn?.(app, uris);
      return { ok: true, dryRun: true };
    },

    'apps.recent': async ({ limit = 12 } = {}, m) => {
      if (m.settings()?.privacy?.recentFiles === false) return [];
      return recentItems().slice(0, Math.max(0, Number(limit) || 12));
    },

    'apps.forFile': async ({ path } = {}) => {
      if (typeof path !== 'string' || !path.startsWith('/')) throw err('bad_args', 'Ongeldig pad.');
      if (!ext(path) && !/\.[^/]+$/.test(path)) {
        // folders (no extension): our Verkenner is always the default, like the host
        return ['win11shell-explorer.desktop', 'org.gnome.Nautilus.desktop'].map((id) => byId.get(id)).filter(Boolean)
          .map((a, i) => ({ id: a.id, name: a.name, icon: a.icon, isDefault: i === 0 }));
      }
      const ids = FOR_EXT[ext(path)] || ['org.gnome.TextEditor.desktop'];
      return ids.map((id) => byId.get(id)).filter(Boolean)
        .map((a, i) => ({ id: a.id, name: a.name, icon: a.icon, isDefault: i === 0 }));
    },

    'apps.defaults': async ({ candidates } = {}) => {
      const out = Object.fromEntries(Object.entries(defaults).map(([k, id]) => [k, pick(byId.get(id))]));
      if (candidates) {
        out.candidates = Object.fromEntries(Object.keys(DEFAULTS).map((kind) => [kind,
          APPS.filter((a) => CANDIDATE_CATS[kind].some((c) => a.categories.includes(c)))
            .map(pick).sort((a, b) => COLL.compare(a.name, b.name))]));
      }
      return out;
    },

    'apps.clearRecent': async (args, m) => {
      recentCleared = true;
      m.emit('apps.changed', {});
      return { ok: true, dryRun: true };
    },

    'explorer.ready': async ({ id, ack } = {}, m) => {
      if (typeof id !== 'string' || !id || id.length > 64) throw err('bad_args', 'Ongeldige invoer.');
      let list = mockNavList(m);
      if (Number.isInteger(ack)) {
        mockNav = list = list.filter((x) => x.seq > ack);
        if (!list.length) return {};
      }
      const items = list.slice();
      const seq = items.length ? items[items.length - 1].seq : 0;
      setTimeout(() => m.emit('explorer.backlog', { id, boot: 'mock', seq, navigate: items }),
        Number(m.params?.get?.('mocknavdelay')) || 0);
      return {};
    },

    'apps.setDefault': async ({ kind, id } = {}, m) => {
      if (!(kind in DEFAULTS)) throw err('bad_args', 'Onbekend soort standaard-app.');
      const own = OWN.exec(String(id || ''));
      if (own && !(kind === 'files' && own[1] === 'explorer')) throw err('bad_args', 'Deze app kan hier niet als standaard worden gekozen.');
      if (!byId.has(id)) throw err('not_found', 'Deze app is niet gevonden.');
      const types = KIND_TYPES[kind] || [kind];
      if (m?.params?.get?.('mockdefaultoverride') === kind && defaults[kind] !== id) {
        // like the host when ~/.config/<desktop>-mimeapps.list wins over mimeapps.list
        return { ok: true, dryRun: true, types, cleared: [], effective: false, current: defaults[kind],
          overriddenBy: ['/home/robin/.config/hyprland-mimeapps.list'] };
      }
      defaults[kind] = id;
      return { ok: true, dryRun: true, types, cleared: [], effective: true };
    },

    'apps.openUri': async ({ uri } = {}) => {
      if (typeof uri !== 'string' || !/^(https?:|mailto:|file:)/i.test(uri)) {
        throw err('bad_uri', 'Dit soort link kan niet worden geopend.');
      }
      console.info('[mock] apps.openUri', uri);
      return { ok: true, dryRun: true };
    },
  },
  start() { /* no periodic events */ },
};
