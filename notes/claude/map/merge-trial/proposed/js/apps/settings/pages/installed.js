// Instellingen › Apps › Geïnstalleerde apps: searchable, sortable list with an “…” menu
// (Openen, Vastmaken aan Start / taakbalk, native verwijderen met gecontroleerd plan).

import { h, clear } from '../../../core/dom.js';
import { makeT, lang, collator } from '../../../core/i18n.js';
import { getSetting, setSetting } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, SearchBox, Dropdown, IconButton, AppIcon, EmptyState, Toast } from '../../../core/components.js';
import { showContextMenu } from '../../../core/contextmenu.js';
import { populateSoftwareRemoval, softwareRemovalPlaceholder, reviewAndTrackSoftware } from '../../../core/software.js';
import { call, run, notice, showError } from '../common.js';

const t = makeT({
  nl: {
    search: 'Apps zoeken',
    sort: 'Sorteren op',
    nameAz: 'Naam (A-Z)', nameZa: 'Naam (Z-A)',
    filter: 'Filteren op',
    all: 'Alle apps', pinnedStart: 'Vastgemaakt aan Start', pinnedTaskbar: 'Vastgemaakt aan taakbalk',
    found: { zero: 'Geen apps gevonden', one: '1 app gevonden', other: '{n} apps gevonden' },
    more: 'Meer opties',
    open: 'Openen',
    pinStart: 'Vastmaken aan Start', unpinStart: 'Losmaken van Start',
    pinTaskbar: 'Vastmaken aan taakbalk', unpinTaskbar: 'Losmaken van taakbalk',
    pinnedS: 'Vastgemaakt aan Start', unpinnedS: 'Losgemaakt van Start',
    pinnedT: 'Vastgemaakt aan de taakbalk', unpinnedT: 'Losgemaakt van de taakbalk',
    opening: '{name} wordt geopend',
    na: 'De lijst met apps is niet beschikbaar.',
    noMatch: 'Geen apps gevonden',
    noMatchBody: 'Probeer een andere naam.',
    ourApp: 'Onderdeel van Win11 Shell',
  },
  en: {
    search: 'Search apps',
    sort: 'Sort by',
    nameAz: 'Name (A-Z)', nameZa: 'Name (Z-A)',
    filter: 'Filter by',
    all: 'All apps', pinnedStart: 'Pinned to Start', pinnedTaskbar: 'Pinned to taskbar',
    found: { zero: 'No apps found', one: '1 app found', other: '{n} apps found' },
    more: 'More options',
    open: 'Open',
    pinStart: 'Pin to Start', unpinStart: 'Unpin from Start',
    pinTaskbar: 'Pin to taskbar', unpinTaskbar: 'Unpin from taskbar',
    pinnedS: 'Pinned to Start', unpinnedS: 'Unpinned from Start',
    pinnedT: 'Pinned to the taskbar', unpinnedT: 'Unpinned from the taskbar',
    opening: 'Opening {name}',
    na: 'The list of apps isn’t available.',
    noMatch: 'No apps found',
    noMatchBody: 'Try another name.',
    ourApp: 'Part of Win11 Shell',
  },
});

const CAT_NL = { WebBrowser: 'Webbrowser', Email: 'E-mail', FileManager: 'Bestandsbeheer', TextEditor: 'Teksteditor', TerminalEmulator: 'Terminal', Game: 'Spel', Development: 'Ontwikkeling', Graphics: 'Grafisch', AudioVideo: 'Audio en video', Audio: 'Audio', Video: 'Video', Office: 'Kantoor', Utility: 'Hulpmiddel', System: 'Systeem', Settings: 'Instellingen', Network: 'Internet', Education: 'Onderwijs', Science: 'Wetenschap' };
const CAT_EN = { WebBrowser: 'Web browser', Email: 'Email', FileManager: 'File manager', TextEditor: 'Text editor', TerminalEmulator: 'Terminal', Game: 'Game', Development: 'Development', Graphics: 'Graphics', AudioVideo: 'Audio & video', Audio: 'Audio', Video: 'Video', Office: 'Office', Utility: 'Utility', System: 'System', Settings: 'Settings', Network: 'Internet', Education: 'Education', Science: 'Science' };
const PRIORITY = ['WebBrowser', 'Email', 'FileManager', 'TextEditor', 'TerminalEmulator', 'Game', 'Office', 'Development', 'Graphics', 'Audio', 'Video', 'AudioVideo', 'Network', 'Education', 'Science', 'Utility', 'System', 'Settings'];

function describe(app) {
  if (String(app.id).startsWith('win11shell-')) return t('ourApp');
  const names = lang() === 'en' ? CAT_EN : CAT_NL;
  const cat = PRIORITY.find((c) => (app.categories || []).includes(c));
  const kind = app.genericName && app.genericName !== app.name ? app.genericName : (cat ? names[cat] : '');
  return [kind, app.comment].filter((v) => v && v !== app.name).filter((v, i, a) => a.indexOf(v) === i).join(' · ');
}

async function togglePin(path, id, onMsg, offMsg) {
  const list = [...(getSetting(path, []) || [])];
  const i = list.indexOf(id);
  if (i >= 0) list.splice(i, 1); else list.push(id);
  try {
    await setSetting(path, list);
    Toast.show({ severity: 'success', title: i >= 0 ? offMsg : onMsg, timeout: 2000 });
  } catch (e) { showError(e); }
}

export const pages = {
  async installed(ctx) {
    const r = await call('apps.list');
    const apps = r.ok && Array.isArray(r.value) ? r.value.filter((a) => !a.noDisplay) : null;
    if (!apps) return [notice({ message: t('na') })];

    let query = '';
    let sort = 'az';
    let filter = 'all';
    const list = h('div', { class: 'st-app-list' });
    const count = h('div', { class: 'st-count' });

    function menu(app, anchor) {
      const inStart = (getSetting('start.pinned', []) || []).includes(app.id);
      const inBar = (getSetting('taskbar.pinned', []) || []).includes(app.id);
      const items = [
        { label: t('open'), icon: 'open', action: async () => { if (await run('apps.launch', { id: app.id })) Toast.show({ title: t('opening', { name: app.name }), timeout: 1800 }); } },
        { separator: true },
        { label: inStart ? t('unpinStart') : t('pinStart'), icon: inStart ? 'unpin' : 'pin', action: () => togglePin('start.pinned', app.id, t('pinnedS'), t('unpinnedS')) },
        { label: inBar ? t('unpinTaskbar') : t('pinTaskbar'), icon: inBar ? 'unpin' : 'taskbar', action: () => togglePin('taskbar.pinned', app.id, t('pinnedT'), t('unpinnedT')) },
      ];
      const menu = showContextMenu({ anchor, align: 'end', items: [...items, {separator: true}, softwareRemovalPlaceholder()] });
      populateSoftwareRemoval(menu, items, app, () => reviewAndTrackSoftware({action: 'uninstall', id: app.id}));
    }

    function paint() {
      clear(list);
      const q = query.trim().toLowerCase();
      const pinsS = getSetting('start.pinned', []) || [];
      const pinsT = getSetting('taskbar.pinned', []) || [];
      let items = apps.filter((a) => !q || a.name.toLowerCase().includes(q) || (a.genericName || '').toLowerCase().includes(q) || (a.keywords || []).some((k) => String(k).toLowerCase().includes(q)));
      if (filter === 'start') items = items.filter((a) => pinsS.includes(a.id));
      if (filter === 'taskbar') items = items.filter((a) => pinsT.includes(a.id));
      const coll = collator({ sensitivity: 'base' });
      items.sort((a, b) => coll.compare(a.name, b.name) * (sort === 'az' ? 1 : -1));
      count.textContent = t('found', { n: items.length });
      if (!items.length) { list.append(EmptyState({ icon: 'apps', title: t('noMatch'), message: t('noMatchBody') })); return; }
      for (const app of items) {
        const more = IconButton({ icon: 'more', label: t('more'), size: 32 });
        more.addEventListener('click', (e) => { e.stopPropagation(); menu(app, more); });
        const card = SettingsCard({ icon: AppIcon({ name: app.icon || app.id, size: 32 }), title: app.name, description: describe(app), control: more });
        card.classList.add('st-app-row');
        card.addEventListener('contextmenu', (e) => { e.preventDefault(); menu(app, more); });
        list.append(card);
      }
    }

    const search = SearchBox({ placeholder: t('search'), onInput: (v) => { query = v; paint(); }, onSubmit: (v) => { query = v; paint(); } });
    search.classList.add('st-app-search');
    const sortDd = Dropdown({ options: [{ value: 'az', label: t('nameAz') }, { value: 'za', label: t('nameZa') }], value: sort, ariaLabel: t('sort'), onChange: (v) => { sort = v; paint(); } });
    const filterDd = Dropdown({ options: [{ value: 'all', label: t('all') }, { value: 'start', label: t('pinnedStart') }, { value: 'taskbar', label: t('pinnedTaskbar') }], value: filter, ariaLabel: t('filter'), onChange: (v) => { filter = v; paint(); } });
    paint();
    ctx.on('apps.changed', async () => { const r2 = await call('apps.list'); if (r2.ok && Array.isArray(r2.value)) { apps.splice(0, apps.length, ...r2.value.filter((a) => !a.noDisplay)); paint(); } });
    ctx.onSettings((s, p) => { if (filter !== 'all' && (!p || /pinned/.test(p))) paint(); });

    return [
      h('div', { class: 'st-toolbar' },
        search,
        h('label', { class: 'st-toolbar__field' }, h('span', null, t('sort')), sortDd),
        h('label', { class: 'st-toolbar__field' }, h('span', null, t('filter')), filterDd)),
      count,
      SettingsGroup({ children: [list] }),
    ];
  },
};
