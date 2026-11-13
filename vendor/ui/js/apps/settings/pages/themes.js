// Instellingen › Persoonlijke instellingen › Thema’s: six presets (wallpaper + mode + accent).

import { h } from '../../../core/dom.js';
import { makeT, tr } from '../../../core/i18n.js';
import { bridge } from '../../../core/bridge.js';
import { getSettings, themeMode } from '../../../core/theme.js';
import { icon } from '../../../core/icons.js';
import { SettingsGroup, Toast } from '../../../core/components.js';
import { DesktopPreview, wallpaperUrl, twinFor, linkCard, showError, colorName } from '../common.js';

const t = makeT({
  nl: {
    current: 'Huidig thema: {name}',
    custom: 'Aangepast',
    apply: 'Een thema toepassen',
    applyDesc: 'Een thema verandert in één keer de achtergrond, de kleurmodus en de accentkleur.',
    background: 'Achtergrond', color: 'Kleur', mode: 'Modus',
    light: 'Licht', dark: 'Donker',
    applied: 'Thema “{name}” toegepast',
    solid: 'Effen kleur', picture: 'Eigen foto',
    related: 'Gerelateerde instellingen',
  },
  en: {
    current: 'Current theme: {name}',
    custom: 'Custom',
    apply: 'Apply a theme',
    applyDesc: 'A theme changes the background, colour mode and accent colour in one go.',
    background: 'Background', color: 'Colour', mode: 'Mode',
    light: 'Light', dark: 'Dark',
    applied: 'Theme “{name}” applied',
    solid: 'Solid colour', picture: 'Own photo',
    related: 'Related settings',
  },
});

export const THEMES = [
  { id: 'lotus-light', name: { nl: 'Lotus (licht)', en: 'Lotus (light)' }, wallpaper: 'builtin:lotus-light', mode: 'light', accent: '#0078D4' },
  { id: 'lotus-dark', name: { nl: 'Lotus (donker)', en: 'Lotus (dark)' }, wallpaper: 'builtin:lotus-dark', mode: 'dark', accent: '#0078D4' },
  { id: 'flow', name: { nl: 'Stroming', en: 'Flow' }, wallpaper: 'builtin:flow-dark', mode: 'dark', accent: '#8764B8' },
  { id: 'glow', name: { nl: 'Gloed', en: 'Glow' }, wallpaper: 'builtin:glow', mode: 'dark', accent: '#E3008C' },
  { id: 'sunrise', name: { nl: 'Zonsopgang', en: 'Sunrise' }, wallpaper: 'builtin:sunrise', mode: 'light', accent: '#CA5010' },
  { id: 'aurora', name: { nl: 'Noorderlicht', en: 'Aurora' }, wallpaper: 'builtin:aurora', mode: 'dark', accent: '#00B294' },
];

/** Friendly names of the built-in wallpapers. */
export const WALLPAPERS = [
  { id: 'lotus-light', name: { nl: 'Lotus licht', en: 'Lotus light' } },
  { id: 'lotus-dark', name: { nl: 'Lotus donker', en: 'Lotus dark' } },
  { id: 'flow-light', name: { nl: 'Stroming licht', en: 'Flow light' } },
  { id: 'flow-dark', name: { nl: 'Stroming donker', en: 'Flow dark' } },
  { id: 'glow', name: { nl: 'Gloed', en: 'Glow' } },
  { id: 'sunrise', name: { nl: 'Zonsopgang', en: 'Sunrise' } },
  { id: 'aurora', name: { nl: 'Noorderlicht', en: 'Aurora' } },
  { id: 'dunes', name: { nl: 'Duinen', en: 'Dunes' } },
];

const sameAccent = (a, b) => {
  const n = (x) => String(x || '').toLowerCase().replace('#0067c0', '#0078d4');
  return n(a) === n(b);
};

export function currentTheme(s = getSettings()) {
  if (s.wallpaper?.type === 'color') return null;
  return THEMES.find((th) => s.theme?.mode === th.mode && sameAccent(s.theme?.accent, th.accent) && !s.theme?.accentFromWallpaper
    && twinFor(s.wallpaper?.path, th.mode, s.wallpaper?.followTheme !== false) === twinFor(th.wallpaper, th.mode)) || null;
}

export async function applyTheme(th) {
  try {
    await bridge.call('settings.patch', { patch: {
      theme: { mode: th.mode, accent: th.accent, accentFromWallpaper: false },
      wallpaper: { type: 'image', path: th.wallpaper, followTheme: true },
    } });
    Toast.show({ severity: 'success', title: t('applied', { name: tr(th.name) }), timeout: 2200 });
  } catch (e) { showError(e); }
}

/** Thumbnail card for one theme. */
export function ThemeCard(th, { compact = false } = {}) {
  const prev = DesktopPreview({ mode: th.mode, accent: th.accent, url: wallpaperUrl(twinFor(th.wallpaper, th.mode)), fit: 'fill', window: !compact });
  const el = h('button', {
    type: 'button', class: ['st-theme', compact && 'is-compact'], role: 'radio', 'aria-checked': 'false',
    'aria-label': tr(th.name), dataset: { tooltip: compact ? tr(th.name) : null, theme: th.id },
    onClick: () => applyTheme(th),
  }, prev, h('span', { class: 'st-theme__check' }, icon('check', { size: 12 })), compact ? null : h('span', { class: 'st-theme__name' }, tr(th.name)));
  el.prev = prev;
  return el;
}

/** Grid of theme cards that tracks the current selection. */
export function ThemeGrid(ctx, { compact = false } = {}) {
  const grid = h('div', { class: ['st-theme-grid', compact && 'is-compact'], role: 'radiogroup' });
  const cards = THEMES.map((th) => ThemeCard(th, { compact }));
  grid.append(...cards);
  const sync = () => {
    const cur = currentTheme();
    cards.forEach((c, i) => { const on = THEMES[i] === cur; c.classList.toggle('is-selected', on); c.setAttribute('aria-checked', String(on)); });
  };
  sync();
  ctx.onSettings(sync);
  return grid;
}

export function wallpaperName(s = getSettings()) {
  const w = s.wallpaper || {};
  if (w.type === 'color') return t('solid');
  const p = String(w.path || '');
  if (p.startsWith('builtin:')) {
    const id = twinFor(p, themeMode(), w.followTheme !== false).slice(8);
    const wp = WALLPAPERS.find((x) => x.id === id);
    return wp ? tr(wp.name) : id;
  }
  return p.split('/').pop() || t('picture');
}

export const pages = {
  async themes(ctx) {
    const prev = DesktopPreview({ cls: 'is-large' });
    const nameEl = h('div', { class: 'st-hero__title' });
    const bgEl = h('span');
    const swatch = h('span', { class: 'st-swatch-dot' });
    const colorEl = h('span');
    const modeEl = h('span');
    const sync = (s = getSettings()) => {
      prev.update({ settings: s });
      const cur = currentTheme(s);
      nameEl.textContent = t('current', { name: cur ? tr(cur.name) : t('custom') });
      bgEl.textContent = wallpaperName(s);
      swatch.style.background = s.theme?.accent || '#0078d4';
      colorEl.textContent = colorName(s.theme?.accent);
      colorEl.dataset.tooltip = String(s.theme?.accent || '').toUpperCase();
      modeEl.textContent = themeMode() === 'dark' ? t('dark') : t('light');
    };
    sync();
    ctx.onSettings((s) => sync(s));

    const hero = h('section', { class: 'st-hero st-theme-hero' },
      h('div', { class: 'st-hero__art' }, prev),
      h('div', { class: 'st-hero__text' }, nameEl,
        h('div', { class: 'st-theme-facts' },
          h('button', { type: 'button', class: 'st-fact', onClick: () => ctx.go('background') }, icon('image', { size: 16 }), h('span', { class: 'st-fact__k' }, t('background')), bgEl),
          h('button', { type: 'button', class: 'st-fact', onClick: () => ctx.go('colors') }, swatch, h('span', { class: 'st-fact__k' }, t('color')), colorEl),
          h('button', { type: 'button', class: 'st-fact', onClick: () => ctx.go('colors') }, icon('theme', { size: 16 }), h('span', { class: 'st-fact__k' }, t('mode')), modeEl))));

    return [
      hero,
      SettingsGroup({ title: t('apply'), description: t('applyDesc'), children: [h('div', { class: 'st-card-pad' }, ThemeGrid(ctx))] }),
      SettingsGroup({ title: t('related'), children: [linkCard(ctx, 'desktopicons'), linkCard(ctx, 'contrast')] }),
    ];
  },
};

