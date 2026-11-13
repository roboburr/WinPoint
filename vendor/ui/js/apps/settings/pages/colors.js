// Instellingen › Persoonlijke instellingen › Kleuren: mode, transparency, accent colour
// (manual / automatic from the wallpaper, Windows’ 48 swatches, recent and custom colours),
// accent on Start and taskbar, the GTK theme family of other apps (theme.gtkFamily: Fluent or Ubuntu's own Yaru,
// light/dark follows the mode), Windows 11 caption buttons in other apps (theme.captionButtons).

import { h, clear } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { bridge } from '../../../core/bridge.js';
import { getSetting, setSetting, themeMode, ACCENT_PRESETS, normalizeHex } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, SettingsExpander, Dropdown, Button, ColorSwatchGrid } from '../../../core/components.js';
import { settingDropdown, dropdownCard, toggleCard, showError, linkCard } from '../common.js';
import { pickColor } from '../colorpicker.js';

const t = makeT({
  nl: {
    mode: 'Kies je modus',
    modeDesc: 'De kleuren van Win11 Shell en je apps: licht of donker',
    light: 'Licht', dark: 'Donker',
    transparency: 'Transparantie-effecten',
    transparencyDesc: 'Vensters en oppervlakken zijn een beetje doorschijnend',
    accent: 'Accentkleur',
    accentDesc: 'De kleur van knoppen, schakelaars en de selectie',
    manual: 'Handmatig', automatic: 'Automatisch',
    autoNote: 'De accentkleur wordt gekozen uit je bureaubladachtergrond.',
    recent: 'Recente kleuren',
    windowsColors: 'Kleuren',
    customColors: 'Aangepaste kleuren',
    customBtn: 'Kleuren weergeven',
    onTaskbar: 'Accentkleur weergeven op Start en taakbalk',
    onTaskbarDesc: 'Start, de taakbalk en het meldingencentrum krijgen je accentkleur',
    onTaskbarLight: 'Alleen beschikbaar in de donkere modus',
    appTheme: 'Thema voor apps',
    appThemeDesc: 'Hoe andere apps eruitzien; licht en donker volgen je modus. Zonder Fluent-thema houden apps het Ubuntu-thema.',
    appThemeFluent: 'Windows 11 (Fluent)',
    appThemeSystem: 'Ubuntu-standaard (Yaru)',
    captionButtons: 'Windows 11-vensterknoppen in apps',
    captionButtonsDesc: 'Minimaliseren, maximaliseren en sluiten zoals in Windows 11, in licht en donker. Geldt voor apps die je hierna opent; Snap- en Flatpak-apps houden hun eigen knoppen.',
    related: 'Gerelateerde instellingen',
  },
  en: {
    mode: 'Choose your mode',
    modeDesc: 'The colours of Win11 Shell and your apps: light or dark',
    light: 'Light', dark: 'Dark',
    transparency: 'Transparency effects',
    transparencyDesc: 'Windows and surfaces appear slightly translucent',
    accent: 'Accent colour',
    accentDesc: 'The colour of buttons, switches and selection',
    manual: 'Manual', automatic: 'Automatic',
    autoNote: 'The accent colour is picked from your desktop background.',
    recent: 'Recent colours',
    windowsColors: 'Colours',
    customColors: 'Custom colours',
    customBtn: 'View colours',
    onTaskbar: 'Show accent colour on Start and taskbar',
    onTaskbarDesc: 'Start, the taskbar and the notification centre use your accent colour',
    onTaskbarLight: 'Only available in dark mode',
    appTheme: 'Theme for apps',
    appThemeDesc: 'How other apps look; light and dark follow your mode. Without the Fluent theme, apps keep the Ubuntu theme.',
    appThemeFluent: 'Windows 11 (Fluent)',
    appThemeSystem: 'Ubuntu default (Yaru)',
    captionButtons: 'Windows 11 caption buttons in apps',
    captionButtonsDesc: 'Minimise, maximise and close like Windows 11, in light and dark. Applies to apps you open after this; Snap and Flatpak apps keep their own buttons.',
    related: 'Related settings',
  },
});

const RECENT_KEY = 'win11shell.settings.recentColors';
function recentColors() {
  try { const v = JSON.parse(localStorage.getItem(RECENT_KEY) || '[]'); return Array.isArray(v) ? v.filter(normalizeHex).slice(0, 5) : []; } catch { return []; }
}
function pushRecent(hex) {
  const list = [hex, ...recentColors().filter((x) => x.toLowerCase() !== hex.toLowerCase())].slice(0, 5);
  try { localStorage.setItem(RECENT_KEY, JSON.stringify(list)); } catch { /* ignore */ }
}

export async function setAccent(hex) {
  const n = normalizeHex(hex);
  if (!n) return;
  try {
    if (getSetting('theme.accentFromWallpaper', false)) {
      await bridge.call('settings.patch', { patch: { theme: { accent: n.toUpperCase(), accentFromWallpaper: false } } });
    } else {
      await setSetting('theme.accent', n.toUpperCase());
    }
    pushRecent(n.toUpperCase());
  } catch (e) { showError(e); }
}

export const pages = {
  async colors(ctx) {
    const modeCard = SettingsCard({
      icon: 'theme', title: t('mode'), description: t('modeDesc'),
      control: settingDropdown(ctx, 'theme.mode', [{ value: 'light', label: t('light') }, { value: 'dark', label: t('dark') }], { width: 160 }),
    });
    const transCard = toggleCard(ctx, { icon: 'sparkle', title: t('transparency'), description: t('transparencyDesc'), path: 'theme.transparency' });

    /* accent expander */
    const auto = () => !!getSetting('theme.accentFromWallpaper', false);
    const modeDd = Dropdown({
      options: [{ value: 'manual', label: t('manual') }, { value: 'auto', label: t('automatic') }],
      value: auto() ? 'auto' : 'manual', width: 160, ariaLabel: t('accent'),
      onChange: async (v) => {
        try { await setSetting('theme.accentFromWallpaper', v === 'auto'); } catch (e) { modeDd.setValue(auto() ? 'auto' : 'manual'); showError(e); }
      },
    });
    const grid = ColorSwatchGrid({ colors: ACCENT_PRESETS, value: getSetting('theme.accent'), onChange: (c) => setAccent(c), columns: 8, size: 40, ariaLabel: t('windowsColors') });
    const recentSlot = h('div', { class: 'st-recent-colors' });
    function paintRecent() {
      clear(recentSlot);
      const list = recentColors();
      recentSlot.parentElement && (recentSlot.parentElement.hidden = !list.length);
      if (!list.length) return;
      recentSlot.append(ColorSwatchGrid({ colors: list, value: getSetting('theme.accent'), onChange: (c) => setAccent(c), columns: 5, size: 40, ariaLabel: t('recent') }));
    }
    const autoNote = h('div', { class: 'st-sub-note' }, t('autoNote'));
    const recentBlock = h('div', { class: 'st-color-block' }, h('div', { class: 'st-color-block__title' }, t('recent')), recentSlot);
    const presetBlock = h('div', { class: 'st-color-block' }, h('div', { class: 'st-color-block__title' }, t('windowsColors')), grid);
    const customCard = SettingsCard({
      title: t('customColors'),
      control: Button({ label: t('customBtn'), onClick: async () => {
        const hex = await pickColor({ value: getSetting('theme.accent', '#0078d4') });
        if (hex) setAccent(hex);
      } }),
    });
    const exp = SettingsExpander({
      icon: 'color', title: t('accent'), description: t('accentDesc'), control: modeDd, expanded: true,
      children: [autoNote, recentBlock, presetBlock, customCard],
    });
    exp.classList.add('st-accent-exp');
    paintRecent();

    const tbCard = toggleCard(ctx, { icon: 'taskbar', title: t('onTaskbar'), description: t('onTaskbarDesc'), path: 'theme.accentOnTaskbar' });
    const appThemeCard = dropdownCard(ctx, {
      icon: 'brush', title: t('appTheme'), description: t('appThemeDesc'), path: 'theme.gtkFamily', width: 220, fallback: 'fluent',
      options: [{ value: 'fluent', label: t('appThemeFluent') }, { value: 'system', label: t('appThemeSystem') }],
    });
    const captionCard = toggleCard(ctx, { icon: 'window', title: t('captionButtons'), description: t('captionButtonsDesc'), path: 'theme.captionButtons' });

    function sync() {
      const a = auto();
      modeDd.setValue(a ? 'auto' : 'manual');
      grid.setValue(getSetting('theme.accent'));
      exp.classList.toggle('is-auto', a);
      autoNote.hidden = !a;
      paintRecent();
      const light = themeMode() === 'light';
      tbCard.setDisabled(light);
      tbCard.toggle.setDisabled(light);
      tbCard.setDescription(light ? t('onTaskbarLight') : t('onTaskbarDesc'));
    }
    sync();
    ctx.onSettings(sync);

    return [
      SettingsGroup({ children: [modeCard, transCard, exp, tbCard, appThemeCard, captionCard] }),
      SettingsGroup({ title: t('related'), children: [linkCard(ctx, 'contrast'), linkCard(ctx, 'themes')] }),
    ];
  },
};
