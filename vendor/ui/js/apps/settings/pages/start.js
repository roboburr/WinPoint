// Instellingen › Persoonlijke instellingen › Start: layout, recent apps, recommendations,
// folders next to the power button.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { getSetting, setSetting } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, SettingsExpander, ToggleSwitch } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { toggleCard, showError, notice, linkCard } from '../common.js';

const t = makeT({
  nl: {
    layout: 'Indeling',
    layoutDesc: 'Hoeveel ruimte Start geeft aan vastgemaakte apps en aan aanbevelingen',
    morePins: 'Meer vastgemaakte items', default: 'Standaard', moreRecs: 'Meer aanbevelingen',
    recentApps: 'Onlangs toegevoegde apps weergeven',
    recentAppsDesc: 'Nieuwe apps verschijnen bovenaan Aanbevolen',
    recommended: 'Aanbevolen bestanden weergeven in Start',
    recommendedDesc: 'Je recente bestanden in het deel Aanbevolen',
    recentOff: 'Recente bestanden staan uit bij Privacy en beveiliging. Aanbevolen blijft dan leeg.',
    folders: 'Mappen',
    foldersDesc: 'Deze mappen staan naast de aan/uit-knop',
    f_settings: 'Instellingen', f_explorer: 'Verkenner', f_documents: 'Documenten', f_downloads: 'Downloads',
    f_music: 'Muziek', f_pictures: 'Afbeeldingen', f_videos: 'Video’s', f_home: 'Persoonlijke map',
  },
  en: {
    layout: 'Layout',
    layoutDesc: 'How much room Start gives to pinned apps and to recommendations',
    morePins: 'More pins', default: 'Default', moreRecs: 'More recommendations',
    recentApps: 'Show recently added apps',
    recentAppsDesc: 'New apps appear at the top of Recommended',
    recommended: 'Show recommended files in Start',
    recommendedDesc: 'Your recent files in the Recommended section',
    recentOff: 'Recent files are turned off in Privacy & security, so Recommended stays empty.',
    folders: 'Folders',
    foldersDesc: 'These folders appear next to the power button',
    f_settings: 'Settings', f_explorer: 'File Explorer', f_documents: 'Documents', f_downloads: 'Downloads',
    f_music: 'Music', f_pictures: 'Pictures', f_videos: 'Videos', f_home: 'Personal folder',
  },
});

/** start.layout values (shared with the Start menu): 'morePins' | 'default' | 'moreRecommendations' */
const LAYOUTS = [
  { value: 'morePins', key: 'morePins', pins: 3, recs: 1 },
  { value: 'default', key: 'default', pins: 2, recs: 2 },
  { value: 'moreRecommendations', key: 'moreRecs', pins: 1, recs: 3 },
];

const FOLDERS = [
  ['settings', 'settings'], ['explorer', 'folder'], ['documents', 'document'], ['downloads', 'download'],
  ['music', 'music'], ['pictures', 'image'], ['videos', 'video'], ['home', 'home'],
];

function layoutArt(l) {
  return h('span', { class: 'st-start-art', 'aria-hidden': 'true' },
    h('span', { class: 'st-start-art__search' }),
    h('span', { class: 'st-start-art__pins' }, Array.from({ length: l.pins * 6 }, () => h('i'))),
    h('span', { class: 'st-start-art__recs' }, Array.from({ length: l.recs * 2 }, () => h('i'))),
    h('span', { class: 'st-start-art__foot' }));
}

export const pages = {
  async start(ctx) {
    const cur = () => getSetting('start.layout', 'default');
    const opts = LAYOUTS.map((l) => {
      const b = h('button', {
        type: 'button', class: 'st-layout', role: 'radio', dataset: { value: l.value },
        onClick: async () => { try { await setSetting('start.layout', l.value); } catch (e) { showError(e); } },
      }, layoutArt(l), h('span', { class: 'st-layout__label' }, h('span', { class: 'st-radio-dot' }), t(l.key)));
      return b;
    });
    const syncLayout = () => opts.forEach((b) => {
      const on = b.dataset.value === cur();
      b.classList.toggle('is-selected', on);
      b.setAttribute('aria-checked', String(on));
    });
    syncLayout();
    const layoutCard = h('div', { class: 'st-layout-card fl-settings-card' },
      h('div', { class: 'fl-sc__inner' },
        h('span', { class: 'fl-sc__icon' }, icon('start-menu', { size: 20 })),
        h('div', { class: 'fl-sc__text' }, h('div', { class: 'fl-sc__title' }, t('layout')), h('div', { class: 'fl-sc__desc' }, t('layoutDesc')))),
      h('div', { class: 'st-layout-row', role: 'radiogroup', 'aria-label': t('layout') }, opts));

    const recentNote = notice({ message: t('recentOff') });
    const syncNote = () => { recentNote.hidden = getSetting('privacy.recentFiles', true) !== false; };
    syncNote();

    /* folders */
    const folderCards = FOLDERS.map(([id, ic]) => {
      const has = () => (getSetting('start.folders', []) || []).includes(id);
      const sw = ToggleSwitch({
        checked: has(), ariaLabel: t(`f_${id}`),
        onChange: async (on) => {
          const list = new Set(getSetting('start.folders', []) || []);
          if (on) list.add(id); else list.delete(id);
          const ordered = FOLDERS.map(([f]) => f).filter((f) => list.has(f));
          try { await setSetting('start.folders', ordered); } catch (e) { sw.setChecked(!on); showError(e); }
        },
      });
      sw.sync = () => sw.setChecked(has());
      const c = SettingsCard({ icon: ic, title: t(`f_${id}`), control: sw });
      c.sw = sw;
      return c;
    });
    const folders = SettingsExpander({ icon: 'folder', title: t('folders'), description: t('foldersDesc'), children: folderCards });

    ctx.onSettings(() => { syncLayout(); syncNote(); folderCards.forEach((c) => c.sw.sync()); });

    return [
      SettingsGroup({ children: [
        layoutCard,
        toggleCard(ctx, { icon: 'apps', title: t('recentApps'), description: t('recentAppsDesc'), path: 'start.showRecentApps' }),
        toggleCard(ctx, { icon: 'history', title: t('recommended'), description: t('recommendedDesc'), path: 'start.showRecommended' }),
        recentNote,
        folders,
      ] }),
      SettingsGroup({ children: [linkCard(ctx, 'privacy')] }),
    ];
  },
};
