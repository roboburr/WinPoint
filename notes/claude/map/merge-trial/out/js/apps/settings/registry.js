// Win11 Shell — Instellingen: page registry (owned by the settings agent). SPEC §11.
//
//   import { SETTINGS_PAGES, searchSettings } from '../apps/settings/registry.js';
//   searchSettings('helder', 'nl') → [{page, score}]   (used by Start search and the Settings search box)
//   Deep link: bridge.call('shell.openApp', {app: 'settings', params: {page: id}})
//
// Pure data + one tiny helper; its only import is core/i18n.js (itself import-free): cheap to load from the shell surface.
// `icon` names come from core/icons.js. `parent` is the id of the top-level page (or null).

import { collator } from '../../core/i18n.js';

const P = (id, parent, icon, nlTitle, enTitle, nlDesc, enDesc, nlKeys = [], enKeys = []) => ({
  id, parent, icon,
  title: { nl: nlTitle, en: enTitle },
  description: { nl: nlDesc, en: enDesc },
  keywords: { nl: nlKeys, en: enKeys },
});

export const SETTINGS_PAGES = [
  P('home', null, 'home', 'Startpagina', 'Home',
    'Aanbevolen instellingen, apparaten en thema in één oogopslag', 'Recommended settings, devices and theme at a glance',
    ['start', 'overzicht', 'begin', 'aanbevolen'], ['home', 'overview', 'start', 'recommended']),

  /* ---- Systeem ------------------------------------------------------------ */
  P('system', null, 'pc', 'Systeem', 'System',
    'Beeldscherm, geluid, meldingen, energie, opslag', 'Display, sound, notifications, power, storage',
    ['computer', 'pc', 'apparaat', 'systeem'], ['computer', 'pc', 'device', 'system']),
  P('display', 'system', 'display', 'Beeldscherm', 'Display',
    'Beeldschermen, helderheid, nachtlampje, schaal en resolutie', 'Monitors, brightness, night light, scale and resolution',
    ['scherm', 'monitor', 'helderheid', 'licht', 'nachtlampje', 'nachtlicht', 'blauw licht', 'resolutie', 'schaal', 'zoom', 'vernieuwingsfrequentie', 'hz', 'beeldschermen', 'tweede scherm'],
    ['screen', 'monitor', 'brightness', 'night light', 'blue light', 'resolution', 'scale', 'zoom', 'refresh rate', 'hz', 'multiple displays']),
  P('sound', 'system', 'sound', 'Geluid', 'Sound',
    'Volume, uitvoer en geluidsapparaten', 'Volume, output and sound devices',
    ['geluid', 'volume', 'luidspreker', 'speaker', 'koptelefoon', 'hoofdtelefoon', 'audio', 'dempen', 'stil', 'uitvoer'],
    ['sound', 'volume', 'speaker', 'headphones', 'headset', 'audio', 'mute', 'output']),
  P('notifications', 'system', 'notification', 'Meldingen', 'Notifications',
    'Waarschuwingen van apps en systeem, niet storen', 'Alerts from apps and system, do not disturb',
    ['meldingen', 'notificaties', 'niet storen', 'stil', 'pop-up', 'berichten', 'geluid melding', 'testmelding'],
    ['notifications', 'alerts', 'do not disturb', 'dnd', 'toast', 'banner', 'test notification']),
  P('power', 'system', 'power', 'Energie en batterij', 'Power & battery',
    'Energiestand, scherm uitschakelen, automatisch slapen en batterij', 'Power mode, screen timeout, automatic sleep and battery',
    ['energie', 'energiestand', 'energie-instellingen', 'energiebesparing', 'energie-efficiëntie', 'prestaties', 'gebalanceerd', 'stroom', 'batterij', 'accu',
      'scherm uitschakelen', 'scherm dimmen', 'schermtimeout', 'automatisch slapen', 'slaapinstellingen', 'inactiviteit',
      'afsluiten', 'uitzetten', 'opnieuw opstarten', 'herstarten', 'slaapstand', 'vergrendelen', 'afmelden', 'uitloggen'],
    ['power', 'power mode', 'power settings', 'energy', 'energy settings', 'power saver', 'efficiency', 'performance', 'balanced', 'battery',
      'screen timeout', 'turn off screen', 'dim screen', 'automatic sleep', 'sleep settings', 'idle',
      'shut down', 'turn off', 'restart', 'reboot', 'sleep', 'lock', 'sign out', 'log out']),
  P('storage', 'system', 'storage', 'Opslag', 'Storage',
    'Opslagruimte, stations, ruimte vrijmaken', 'Storage space, drives, free up space',
    ['opslag', 'schijf', 'harde schijf', 'ruimte', 'vol', 'vrijmaken', 'opruimen', 'stations', 'usb', 'disk', 'ssd'],
    ['storage', 'disk', 'drive', 'space', 'full', 'free up', 'clean up', 'drives', 'usb', 'ssd']),
  P('clipboard', 'system', 'clipboard', 'Klembord', 'Clipboard',
    'Knippen en kopiëren, klembordgeschiedenis', 'Cut and copy history',
    ['klembord', 'kopiëren', 'plakken', 'knippen', 'geschiedenis', 'windows v', 'super v'],
    ['clipboard', 'copy', 'paste', 'cut', 'history', 'win v', 'super v']),
  P('about', 'system', 'info', 'Info', 'About',
    'Apparaatspecificaties, processor, geheugen, versie', 'Device specifications, processor, memory, version',
    ['info', 'over', 'specificaties', 'processor', 'cpu', 'geheugen', 'ram', 'videokaart', 'gpu', 'versie', 'ubuntu', 'kernel', 'computernaam', 'pc-naam'],
    ['about', 'specifications', 'specs', 'processor', 'cpu', 'memory', 'ram', 'graphics', 'gpu', 'version', 'ubuntu', 'kernel', 'device name']),

  /* ---- Bluetooth ---------------------------------------------------------- */
  P('bluetooth', null, 'bluetooth', 'Bluetooth en apparaten', 'Bluetooth & devices',
    'Bluetooth, koptelefoons, muizen en andere apparaten', 'Bluetooth, headphones, mice and other devices',
    ['bluetooth', 'apparaten', 'koppelen', 'verbinden', 'koptelefoon', 'muis', 'toetsenbord', 'oortjes', 'speaker'],
    ['bluetooth', 'devices', 'pair', 'connect', 'headphones', 'mouse', 'keyboard', 'earbuds', 'speaker']),

  /* ---- Netwerk ------------------------------------------------------------ */
  P('network', null, 'globe', 'Netwerk en internet', 'Network & internet',
    'Wi-Fi, ethernet, vliegtuigmodus', 'Wi-Fi, Ethernet, airplane mode',
    ['netwerk', 'internet', 'verbinding', 'online', 'vliegtuigmodus', 'offline'],
    ['network', 'internet', 'connection', 'online', 'airplane mode', 'offline']),
  P('wifi', 'network', 'wifi', 'Wi-Fi', 'Wi-Fi',
    'Draadloze netwerken, verbinden, wachtwoord', 'Wireless networks, connect, password',
    ['wifi', 'wi-fi', 'wlan', 'draadloos', 'netwerk', 'wachtwoord', 'verbinden', 'hotspot'],
    ['wifi', 'wi-fi', 'wlan', 'wireless', 'network', 'password', 'connect', 'hotspot']),
  P('ethernet', 'network', 'ethernet', 'Ethernet', 'Ethernet',
    'Kabelverbinding en IP-adres', 'Wired connection and IP address',
    ['ethernet', 'kabel', 'lan', 'bekabeld', 'ip-adres', 'ip'],
    ['ethernet', 'cable', 'lan', 'wired', 'ip address', 'ip']),

  /* ---- Persoonlijke instellingen ----------------------------------------- */
  P('personalization', null, 'personalize', 'Persoonlijke instellingen', 'Personalisation',
    'Achtergrond, kleuren, thema’s, Start, taakbalk', 'Background, colours, themes, Start, taskbar',
    ['persoonlijk', 'aanpassen', 'uiterlijk', 'personaliseren'], ['personalize', 'personalise', 'customize', 'appearance']),
  P('background', 'personalization', 'image', 'Achtergrond', 'Background',
    'Bureaubladachtergrond, foto of effen kleur', 'Desktop background, picture or solid colour',
    ['achtergrond', 'bureaublad', 'wallpaper', 'foto', 'afbeelding', 'plaatje', 'effen kleur', 'behang'],
    ['background', 'desktop', 'wallpaper', 'picture', 'photo', 'image', 'solid colour', 'solid color']),
  P('colors', 'personalization', 'color', 'Kleuren', 'Colours',
    'Licht of donker, accentkleur, transparantie', 'Light or dark, accent colour, transparency',
    ['kleur', 'kleuren', 'donker', 'licht', 'donkere modus', 'lichte modus', 'nachtmodus', 'accent', 'accentkleur', 'transparantie', 'doorzichtig',
      'vensterknoppen', 'sluitknop', 'minimaliseren', 'maximaliseren', 'thema voor apps', 'gtk-thema', 'yaru', 'fluent'],
    ['colour', 'color', 'colours', 'colors', 'dark', 'light', 'dark mode', 'light mode', 'accent', 'transparency',
      'caption buttons', 'close button', 'minimize', 'maximize', 'app theme', 'gtk theme', 'yaru', 'fluent']),
  P('themes', 'personalization', 'theme', 'Thema’s', 'Themes',
    'Achtergrond, kleur en modus in één keer', 'Background, colour and mode in one go',
    ['thema', "thema's", 'stijl', 'uiterlijk'], ['theme', 'themes', 'style', 'look']),
  P('start', 'personalization', 'start-menu', 'Start', 'Start',
    'Indeling van Start, recente apps, mappen', 'Start layout, recent apps, folders',
    ['start', 'startmenu', 'menu start', 'vastgemaakt', 'aanbevolen', 'mappen', 'recente apps'],
    ['start', 'start menu', 'pinned', 'recommended', 'folders', 'recent apps']),
  P('taskbar', 'personalization', 'taskbar', 'Taakbalk', 'Taskbar',
    'Uitlijning, zoeken, widgets, automatisch verbergen', 'Alignment, search, widgets, auto-hide',
    ['taakbalk', 'balk', 'links', 'midden', 'gecentreerd', 'uitlijning', 'verbergen', 'zoekvak', 'widgets', 'taakweergave', 'klok', 'seconden',
      'systeemvak', 'systeemvakpictogrammen', 'meldingsgebied', 'verborgen pictogrammen', 'pijltje', 'overloop', 'tray'],
    ['taskbar', 'left', 'centre', 'center', 'alignment', 'hide', 'auto-hide', 'search box', 'widgets', 'task view', 'clock', 'seconds',
      'system tray', 'tray icons', 'notification area', 'hidden icons', 'hidden icon menu', 'overflow']),
  P('desktopicons', 'personalization', 'desktop', 'Bureaubladpictogrammen', 'Desktop icons',
    'Deze pc, Prullenbak en Persoonlijke map op het bureaublad', 'This PC, Recycle Bin and user folder on the desktop',
    ['bureaublad', 'pictogrammen', 'iconen', 'deze pc', 'prullenbak', 'persoonlijke map'],
    ['desktop', 'icons', 'this pc', 'recycle bin', 'trash', 'home folder']),

  /* ---- Apps ----------------------------------------------------------------- */
  P('apps', null, 'apps', 'Apps', 'Apps',
    'Geïnstalleerde apps, standaard-apps, opstarten', 'Installed apps, default apps, startup',
    ['apps', 'programma', 'programma’s', 'toepassingen', 'software'], ['apps', 'programs', 'applications', 'software']),
  P('installed', 'apps', 'apps', 'Geïnstalleerde apps', 'Installed apps',
    'Alle apps op deze pc, openen en vastmaken', 'All apps on this PC, open and pin',
    ['geïnstalleerd', 'apps', 'programma', 'vastmaken', 'lijst', 'verwijderen'], ['installed', 'apps', 'programs', 'pin', 'list', 'uninstall']),
  P('defaultapps', 'apps', 'open', 'Standaard-apps', 'Default apps',
    'Welke app opent je webpagina’s, muziek en foto’s', 'Which app opens your web pages, music and photos',
    ['standaard', 'standaardapp', 'browser', 'openen met', 'standaardbrowser', 'mail', 'muziekspeler'],
    ['default', 'default apps', 'browser', 'open with', 'default browser', 'mail', 'music player']),
  P('startup', 'apps', 'startup', 'Opstarten', 'Startup',
    'Apps die starten wanneer je je aanmeldt', 'Apps that start when you sign in',
    ['opstarten', 'automatisch starten', 'autostart', 'aanmelden', 'opstartprogramma'], ['startup', 'autostart', 'start automatically', 'sign in']),

  /* ---- Accounts ------------------------------------------------------------- */
  P('accounts', null, 'user', 'Accounts', 'Accounts',
    'Je account, naam en profielfoto', 'Your account, name and picture',
    ['account', 'gebruiker', 'profiel', 'naam', 'foto', 'wachtwoord', 'aanmelden'], ['account', 'user', 'profile', 'name', 'picture', 'password', 'sign in']),

  /* ---- Tijd en taal ------------------------------------------------------- */
  P('time', null, 'clock', 'Tijd en taal', 'Time & language',
    'Datum, tijd, tijdzone en taal', 'Date, time, time zone and language',
    ['tijd', 'taal', 'datum', 'klok'], ['time', 'language', 'date', 'clock']),
  P('datetime', 'time', 'calendar', 'Datum en tijd', 'Date & time',
    'Tijdzone, 24-uursnotatie, seconden, eerste dag van de week', 'Time zone, 24-hour clock, seconds, first day of week',
    ['datum', 'tijd', 'klok', 'tijdzone', '24 uur', '24-uurs', 'am pm', 'seconden', 'week', 'maandag', 'zondag', 'kalender'],
    ['date', 'time', 'clock', 'time zone', '24 hour', '24-hour', 'am pm', 'seconds', 'week', 'monday', 'sunday', 'calendar']),
  P('language', 'time', 'language', 'Taal en regio', 'Language & region',
    'Weergavetaal: Nederlands of English', 'Display language: Dutch or English',
    ['taal', 'nederlands', 'engels', 'english', 'regio', 'land', 'vertaling'], ['language', 'dutch', 'english', 'region', 'country', 'translation']),

  /* ---- Toegankelijkheid -------------------------------------------------- */
  P('accessibility', null, 'accessibility', 'Toegankelijkheid', 'Accessibility',
    'Tekstgrootte, visuele effecten, contrast', 'Text size, visual effects, contrast',
    ['toegankelijkheid', 'slechtziend', 'leesbaarheid', 'hulp'], ['accessibility', 'low vision', 'readability', 'ease of access']),
  P('textsize', 'accessibility', 'zoom-in', 'Tekstgrootte', 'Text size',
    'Tekst groter maken', 'Make text bigger',
    ['tekst', 'groter', 'lettergrootte', 'letters', 'vergroten', 'grootte', 'lezen', 'zoom'], ['text', 'bigger', 'font size', 'larger', 'enlarge', 'size', 'read', 'zoom']),
  P('visualeffects', 'accessibility', 'sparkle', 'Visuele effecten', 'Visual effects',
    'Animaties en transparantie', 'Animations and transparency',
    ['animatie', 'animaties', 'beweging', 'transparantie', 'effecten', 'rustig'], ['animation', 'animations', 'motion', 'transparency', 'effects', 'reduce motion']),
  P('contrast', 'accessibility', 'eye', 'Contrastthema’s', 'Contrast themes',
    'Meer contrast voor beter leesbare tekst', 'More contrast for easier reading',
    ['contrast', 'hoog contrast', 'leesbaar', 'kleurenblind'], ['contrast', 'high contrast', 'readable', 'colour blind', 'color blind']),

  /* ---- Privacy ------------------------------------------------------------ */
  P('privacy', null, 'shield', 'Privacy en beveiliging', 'Privacy & security',
    'Recente bestanden, beveiliging', 'Recent files, security',
    ['privacy', 'beveiliging', 'recente bestanden', 'geschiedenis', 'wissen', 'veilig'], ['privacy', 'security', 'recent files', 'history', 'clear', 'safe']),

  /* ---- Update en herstel ------------------------------------------------- */
  P('update', null, 'update', 'Update en herstel', 'Update & recovery',
    'Updates, back-up van instellingen, alles terugzetten', 'Updates, settings backup, reset',
    ['update', 'updates', 'bijwerken', 'software-updater', 'back-up', 'backup', 'exporteren', 'importeren', 'terugzetten', 'herstellen', 'resetten', 'versie'],
    ['update', 'updates', 'upgrade', 'software updater', 'backup', 'export', 'import', 'reset', 'restore', 'recovery', 'version']),
];

/** Top-level page ids in navigation order. */
export const SETTINGS_TOP = SETTINGS_PAGES.filter((p) => !p.parent).map((p) => p.id);

/** Old / alternative ids that deep links may use. */
export const SETTINGS_ALIASES = {
  nightlight: 'display', brightness: 'display', monitors: 'display', volume: 'sound', audio: 'sound',
  battery: 'power', energy: 'power', disk: 'storage', info: 'about', devices: 'bluetooth',
  internet: 'network', wallpaper: 'background', accent: 'colors', personalize: 'personalization',
  'default-apps': 'defaultapps', defaults: 'defaultapps', autostart: 'startup', user: 'accounts',
  clock: 'datetime', date: 'datetime', lang: 'language', updates: 'update', backup: 'update',
  reset: 'update', recovery: 'update', ease: 'accessibility', text: 'textsize', animations: 'visualeffects',
  security: 'privacy', taskmgr: 'storage', theme: 'themes',
  // ids other modules (shell, desktop, flyouts, host health actions) or Windows' ms-settings: names may use
  personalisation: 'personalization', colours: 'colors', colour: 'colors', color: 'colors',
  lockscreen: 'personalization', fonts: 'personalization', 'start-menu': 'start', startmenu: 'start',
  'taskbar-settings': 'taskbar', notification: 'notifications', meldingen: 'notifications',
  'date-time': 'datetime', dateandtime: 'datetime', 'datum-tijd': 'datetime', region: 'language',
  'apps-features': 'installed', appsfeatures: 'installed', 'installed-apps': 'installed', installedapps: 'installed', programs: 'installed',
  'startup-apps': 'startup', startupapps: 'startup', 'your-info': 'accounts', yourinfo: 'accounts', account: 'accounts',
  wireless: 'wifi', 'wi-fi': 'wifi', wired: 'ethernet', 'network-status': 'network',
  bluetoothdevices: 'bluetooth', 'sound-devices': 'sound', 'power-sleep': 'power', powersleep: 'power', sleep: 'power',
  'storage-sense': 'storage', storagesense: 'storage', clipboardhistory: 'clipboard', 'about-pc': 'about', pc: 'about',
  'windows-update': 'update', windowsupdate: 'update', 'privacy-security': 'privacy', 'text-size': 'textsize',
  'visual-effects': 'visualeffects', 'contrast-themes': 'contrast', 'desktop-icons': 'desktopicons',
  'night-light': 'display',
};

const norm = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '').replace(/[’']/g, '').trim();

/**
 * searchSettings(query, lang='nl', limit=8) → [{page, score}] best first.
 * Matches title (prefix/word/substring), keywords and description in the given language
 * (and, weaker, in the other language).
 */
export function searchSettings(query, lang = 'nl', limit = 8) {
  const q = norm(query);
  if (!q) return [];
  const words = q.split(/\s+/).filter(Boolean);
  const other = lang === 'en' ? 'nl' : 'en';
  const scored = [];
  for (const page of SETTINGS_PAGES) {
    let total = 0;
    for (const w of words) {
      let best = 0;
      const title = norm(page.title[lang]);
      if (title === w) best = 100;
      else if (title.startsWith(w)) best = 80;
      else if (title.split(/[\s-]+/).some((x) => x.startsWith(w))) best = 65;
      else if (title.includes(w)) best = 50;
      for (const k of page.keywords[lang] || []) {
        const nk = norm(k);
        if (nk === w) best = Math.max(best, 60);
        else if (nk.startsWith(w)) best = Math.max(best, 45);
        else if (nk.includes(w) && w.length > 2) best = Math.max(best, 30);
      }
      if (!best && w.length > 2 && norm(page.description[lang]).includes(w)) best = 20;
      if (!best) {
        if (norm(page.title[other]).startsWith(w)) best = 25;
        else if ((page.keywords[other] || []).some((k) => norm(k).startsWith(w))) best = 15;
      }
      if (!best) { total = 0; break; }
      total += best;
    }
    if (total > 0) scored.push({ page, score: total + (page.parent ? 0 : 2) });
  }
  const coll = collator();
  scored.sort((a, b) => b.score - a.score || coll.compare(a.page.title[lang], b.page.title[lang]));
  return scored.slice(0, limit);
}

export function settingsPage(id) {
  return SETTINGS_PAGES.find((p) => p.id === id) || null;
}

export default SETTINGS_PAGES;
