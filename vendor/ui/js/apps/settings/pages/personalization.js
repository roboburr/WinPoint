// Instellingen › Persoonlijke instellingen (overview): live mini desktop + theme picker,
// then links to Achtergrond, Kleuren, Thema’s, Start, Taakbalk, Bureaubladpictogrammen.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { SettingsGroup } from '../../../core/components.js';
import { DesktopPreview, linkCard } from '../common.js';
import { ThemeGrid } from './themes.js';

const t = makeT({
  nl: { pick: 'Selecteer een thema om toe te passen' },
  en: { pick: 'Select a theme to apply' },
});

export const pages = {
  async personalization(ctx) {
    const prev = DesktopPreview({ cls: 'is-large' });
    ctx.onSettings((s) => prev.update({ settings: s }));
    const hero = h('section', { class: 'st-pers-hero' },
      h('div', { class: 'st-pers-hero__preview' }, prev),
      h('div', { class: 'st-pers-hero__themes' },
        h('div', { class: 'st-pers-hero__label' }, t('pick')),
        ThemeGrid(ctx, { compact: true })));
    return [
      hero,
      SettingsGroup({ children: ['background', 'colors', 'themes', 'start', 'taskbar', 'desktopicons'].map((id) => linkCard(ctx, id)) }),
    ];
  },
};
