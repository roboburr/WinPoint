// Instellingen › Persoonlijke instellingen › Bureaubladpictogrammen.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { SettingsGroup } from '../../../core/components.js';
import { AppIcon } from '../../../core/components.js';
import { toggleCard, dropdownCard } from '../common.js';

const t = makeT({
  nl: {
    show: 'Bureaubladpictogrammen weergeven',
    showDesc: 'Bestanden en snelkoppelingen op je bureaublad laten zien',
    which: 'Welke pictogrammen staan op het bureaublad',
    thisPc: 'Deze pc', thisPcDesc: 'Je schijven en USB-sticks',
    trash: 'Prullenbak', trashDesc: 'Verwijderde bestanden, die je nog kunt terugzetten',
    home: 'Persoonlijke map', homeDesc: 'Je documenten, afbeeldingen en downloads',
    layout: 'Indeling',
    size: 'Pictogramgrootte', small: 'Klein', medium: 'Normaal', large: 'Groot',
    grid: 'Pictogrammen uitlijnen op raster', gridDesc: 'Pictogrammen staan netjes in rijen en kolommen',
    arrange: 'Pictogrammen automatisch schikken', arrangeDesc: 'Pictogrammen worden vanzelf van boven naar beneden gezet',
  },
  en: {
    show: 'Show desktop icons',
    showDesc: 'Show files and shortcuts on your desktop',
    which: 'Which icons appear on the desktop',
    thisPc: 'This PC', thisPcDesc: 'Your drives and USB sticks',
    trash: 'Recycle Bin', trashDesc: 'Deleted files you can still restore',
    home: 'Personal folder', homeDesc: 'Your documents, pictures and downloads',
    layout: 'Layout',
    size: 'Icon size', small: 'Small', medium: 'Medium', large: 'Large',
    grid: 'Align icons to grid', gridDesc: 'Icons line up neatly in rows and columns',
    arrange: 'Auto arrange icons', arrangeDesc: 'Icons are placed from top to bottom automatically',
  },
});

export const pages = {
  async desktopicons(ctx) {
    const img = (name) => h('span', { class: 'st-di-icon' }, AppIcon({ name, size: 24 }));
    return [
      SettingsGroup({ children: [toggleCard(ctx, { icon: 'desktop', title: t('show'), description: t('showDesc'), path: 'desktop.showIcons' })] }),
      SettingsGroup({ title: t('which'), children: [
        toggleCard(ctx, { icon: img('computer'), title: t('thisPc'), description: t('thisPcDesc'), path: 'desktop.showThisPC' }),
        toggleCard(ctx, { icon: img('user-trash'), title: t('trash'), description: t('trashDesc'), path: 'desktop.showTrash' }),
        toggleCard(ctx, { icon: img('user-home'), title: t('home'), description: t('homeDesc'), path: 'desktop.showHome' }),
      ] }),
      SettingsGroup({ title: t('layout'), children: [
        dropdownCard(ctx, { icon: 'grid', title: t('size'), path: 'desktop.iconSize', width: 160, fallback: 'medium',
          options: [{ value: 'small', label: t('small') }, { value: 'medium', label: t('medium') }, { value: 'large', label: t('large') }] }),
        toggleCard(ctx, { icon: 'view', title: t('grid'), description: t('gridDesc'), path: 'desktop.alignToGrid' }),
        toggleCard(ctx, { icon: 'sort', title: t('arrange'), description: t('arrangeDesc'), path: 'desktop.autoArrange' }),
      ] }),
    ];
  },
};
