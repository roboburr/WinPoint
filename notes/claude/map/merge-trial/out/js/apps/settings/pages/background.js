// Instellingen › Persoonlijke instellingen › Achtergrond: picture or solid colour,
// built-in wallpaper grid, "Foto’s zoeken", fit, follow light/dark.

import { h, clear } from '../../../core/dom.js';
import { makeT, tr } from '../../../core/i18n.js';
import { bridge } from '../../../core/bridge.js';
import { getSettings, getSetting, getShellInfo, themeMode, resolveWallpaperPath, normalizeHex } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, SettingsExpander, Dropdown, Button, ColorSwatchGrid, Toast } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { DesktopPreview, wallpaperUrl, twinFor, settingDropdown, toggleCard, showError, linkCard } from '../common.js';
import { WALLPAPERS } from './themes.js';
import { pickColor } from '../colorpicker.js';

const t = makeT({
  nl: {
    personalize: 'Je achtergrond aanpassen',
    personalizeDesc: 'Een afbeelding of een effen kleur op je bureaublad',
    picture: 'Afbeelding', solid: 'Effen kleur',
    pictures: 'Afbeeldingen',
    choose: 'Een foto kiezen',
    browse: 'Foto’s zoeken',
    pickTitle: 'Een foto kiezen voor je bureaublad',
    pickNA: 'In deze weergave kun je geen bestand kiezen. Open Instellingen als app om een eigen foto te kiezen.',
    filter: 'Afbeeldingen',
    fit: 'Kies hoe de afbeelding het scherm vult',
    fill: 'Opvullen', fitOpt: 'Passend maken', stretch: 'Uitrekken', tile: 'Tegel', center: 'Centreren', span: 'Uitspreiden',
    follow: 'Afbeelding aanpassen aan licht en donker',
    followDesc: 'Bloei en Stroming hebben een lichte en een donkere versie die meewisselt met je modus',
    colorTitle: 'Kies je achtergrondkleur',
    custom: 'Aangepaste kleuren',
    customBtn: 'Kleuren weergeven',
    related: 'Gerelateerde instellingen',
    ownPhoto: 'Eigen foto',
  },
  en: {
    personalize: 'Personalise your background',
    personalizeDesc: 'A picture or a solid colour on your desktop',
    picture: 'Picture', solid: 'Solid colour',
    pictures: 'Pictures',
    choose: 'Choose a photo',
    browse: 'Browse photos',
    pickTitle: 'Choose a photo for your desktop',
    pickNA: 'You can’t choose a file in this view. Open Settings as an app to choose your own photo.',
    filter: 'Pictures',
    fit: 'Choose a fit for your desktop image',
    fill: 'Fill', fitOpt: 'Fit', stretch: 'Stretch', tile: 'Tile', center: 'Centre', span: 'Span',
    follow: 'Match the picture to light and dark',
    followDesc: 'Bloom and Flow have a light and a dark version that switches with your mode',
    colorTitle: 'Choose your background colour',
    custom: 'Custom colours',
    customBtn: 'View colours',
    related: 'Related settings',
    ownPhoto: 'Own photo',
  },
});

const SOLID = ['#FF8C00', '#E81123', '#D13438', '#C30052', '#BF0077', '#9A0089', '#881798', '#744DA9',
  '#10893E', '#107C10', '#018574', '#2D7D9A', '#0063B1', '#6B69D6', '#8E8CD8', '#8764B8',
  '#038387', '#486860', '#525E54', '#7E735F', '#4C4A48', '#515C6B', '#4A5459', '#000000'];

const RECENT_KEY = 'win11shell.settings.recentPhotos';
const recentPhotos = () => { try { const v = JSON.parse(localStorage.getItem(RECENT_KEY) || '[]'); return Array.isArray(v) ? v.slice(0, 4) : []; } catch { return []; } };
const pushPhoto = (p) => { try { localStorage.setItem(RECENT_KEY, JSON.stringify([p, ...recentPhotos().filter((x) => x !== p)].slice(0, 4))); } catch { /* ignore */ } };

const patch = (p) => bridge.call('settings.patch', { patch: p }).catch(showError);

export const pages = {
  async background(ctx) {
    const prev = DesktopPreview({ cls: 'is-large' });
    const s0 = getSettings();

    const typeDd = Dropdown({
      options: [{ value: 'image', label: t('picture') }, { value: 'color', label: t('solid') }],
      value: s0.wallpaper?.type === 'color' ? 'color' : 'image', width: 180, ariaLabel: t('personalize'),
      onChange: (v) => patch({ wallpaper: { type: v } }),
    });

    /* picture grid */
    const grid = h('div', { class: 'st-wp-grid', role: 'radiogroup', 'aria-label': t('pictures') });
    function tiles() {
      const s = getSettings();
      const follow = s.wallpaper?.followTheme !== false;
      const mode = themeMode();
      const list = [];
      const cur = String(s.wallpaper?.path || '');
      const customs = [...new Set([...(cur && !cur.startsWith('builtin:') ? [cur] : []), ...recentPhotos()])];
      for (const p of customs) list.push({ path: p, name: p.split('/').pop() || t('ownPhoto') });
      const seen = new Set();
      for (const w of WALLPAPERS) {
        const path = twinFor(`builtin:${w.id}`, mode, follow);
        if (seen.has(path)) continue;
        seen.add(path);
        const wp = WALLPAPERS.find((x) => `builtin:${x.id}` === path) || w;
        list.push({ path, name: follow && /^(bloom|flow)-/.test(w.id) ? tr(wp.name).replace(/\s+(licht|donker|light|dark)$/i, '') : tr(wp.name) });
      }
      return list;
    }
    function paintGrid() {
      clear(grid);
      const s = getSettings();
      const curResolved = s.wallpaper?.type === 'image' ? resolveWallpaperPath(s) : null;
      for (const tl of tiles()) {
        const on = curResolved === twinFor(tl.path, themeMode(), s.wallpaper?.followTheme !== false);
        const img = h('img', { src: wallpaperUrl(tl.path, 320), alt: '', draggable: false, loading: 'lazy' });
        img.addEventListener('error', () => img.replaceWith(h('span', { class: 'st-wp__missing' }, icon('image', { size: 24 }))), { once: true });
        grid.append(h('button', {
          type: 'button', class: ['st-wp', on && 'is-selected'], role: 'radio', 'aria-checked': String(on), 'aria-label': tl.name,
          dataset: { tooltip: tl.name }, onClick: () => patch({ wallpaper: { type: 'image', path: tl.path } }),
        }, img, h('span', { class: 'st-wp__check' }, icon('check', { size: 12 }))));
      }
    }

    const browse = Button({
      label: t('browse'),
      onClick: async () => {
        let r = null;
        try {
          r = await bridge.call('shell.pickFile', { title: t('pickTitle'), filters: [{ name: t('filter'), patterns: ['*.jpg', '*.jpeg', '*.png', '*.webp', '*.gif', '*.bmp', '*.svg', 'image/*'] }] });
        } catch (e) { showError(e); return; }
        if (r?.path) { pushPhoto(r.path); patch({ wallpaper: { type: 'image', path: r.path } }); }
        else if (bridge.isMock || (getShellInfo()?.mode === 'serve' && window.__win11shellHost?.runtime !== 'rust')) Toast.show({ severity: 'info', title: t('choose'), message: t('pickNA'), timeout: 5000 });
      },
    });

    /* solid colours */
    const colorGrid = ColorSwatchGrid({
      colors: SOLID, value: s0.wallpaper?.color, columns: 8, size: 40, ariaLabel: t('colorTitle'),
      onChange: (c) => patch({ wallpaper: { type: 'color', color: c } }),
    });
    const customBtn = Button({
      label: t('customBtn'),
      onClick: async () => {
        const hex = await pickColor({ value: getSetting('wallpaper.color', '#0b2a4a'), accent: false });
        if (hex) patch({ wallpaper: { type: 'color', color: normalizeHex(hex) } });
      },
    });

    const imageBlock = [
      h('div', { class: 'st-color-block' }, h('div', { class: 'st-color-block__title' }, t('pictures')), grid),
      SettingsCard({ title: t('choose'), control: browse }),
    ];
    const colorBlock = [
      h('div', { class: 'st-color-block' }, h('div', { class: 'st-color-block__title' }, t('colorTitle')), colorGrid),
      SettingsCard({ title: t('custom'), control: customBtn }),
    ];
    const exp = SettingsExpander({
      icon: 'image', title: t('personalize'), description: t('personalizeDesc'), control: typeDd, expanded: true,
      children: [...imageBlock, ...colorBlock],
    });

    const fitCard = SettingsCard({
      icon: 'view', title: t('fit'),
      control: settingDropdown(ctx, 'wallpaper.fit', [
        { value: 'fill', label: t('fill') }, { value: 'fit', label: t('fitOpt') }, { value: 'stretch', label: t('stretch') },
        { value: 'tile', label: t('tile') }, { value: 'center', label: t('center') }, { value: 'span', label: t('span') },
      ], { width: 180 }),
    });
    const followCard = toggleCard(ctx, { icon: 'theme', title: t('follow'), description: t('followDesc'), path: 'wallpaper.followTheme' });

    function sync() {
      const s = getSettings();
      const isColor = s.wallpaper?.type === 'color';
      typeDd.setValue(isColor ? 'color' : 'image');
      imageBlock.forEach((el) => { el.hidden = isColor; });
      colorBlock.forEach((el) => { el.hidden = !isColor; });
      fitCard.hidden = isColor;
      followCard.hidden = isColor;
      colorGrid.setValue(s.wallpaper?.color);
      prev.update({ settings: s });
      paintGrid();
    }
    sync();
    ctx.onSettings(sync);

    return [
      h('div', { class: 'st-preview-top' }, prev),
      SettingsGroup({ children: [exp, fitCard, followCard] }),
      SettingsGroup({ title: t('related'), children: [linkCard(ctx, 'colors'), linkCard(ctx, 'themes')] }),
    ];
  },
};
