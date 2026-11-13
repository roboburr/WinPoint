// Instellingen › Toegankelijkheid (+ Tekstgrootte, Visuele effecten, Contrastthema’s).

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { getSetting, setSetting } from '../../../core/theme.js';
import { SettingsCard, SettingsGroup, Slider, Button, Toast } from '../../../core/components.js';
import { toggleCard, linkCard, settingDropdown, showError, notice } from '../common.js';

const t = makeT({
  nl: {
    vision: 'Zicht',
    sampleTitle: 'Voorbeeldtekst',
    sample: 'Sleep de schuifregelaar tot je deze tekst goed kunt lezen. Klik daarna op Toepassen.',
    size: 'Tekstgrootte',
    sizeDesc: 'Hoe groot tekst is in Win11 Shell',
    apply: 'Toepassen',
    applied: 'Tekstgrootte ingesteld op {v}%',
    reset: 'Standaard',
    transparency: 'Transparantie-effecten', transparencyDesc: 'Vensters en oppervlakken zijn een beetje doorschijnend',
    animations: 'Animatie-effecten', animationsDesc: 'Vensters, menu’s en schakelaars bewegen vloeiend. Zet dit uit voor een rustiger beeld.',
    contrastInfo: 'Speciale contrastthema’s zijn nog niet beschikbaar in Win11 Shell.',
    contrastTips: 'Tips voor beter leesbare tekst',
    tipDark: 'Licht of donker', tipDarkDesc: 'Kies de modus die voor jou het prettigst leest',
    tipTransparency: 'Transparantie uitzetten', tipTransparencyDesc: 'Effen achtergronden geven meer contrast',
    light: 'Licht', dark: 'Donker',
    quick: 'Snel aanpassen',
  },
  en: {
    vision: 'Vision',
    sampleTitle: 'Sample text',
    sample: 'Drag the slider until you can read this text comfortably. Then select Apply.',
    size: 'Text size',
    sizeDesc: 'How big text is in Win11 Shell',
    apply: 'Apply',
    applied: 'Text size set to {v}%',
    reset: 'Default',
    transparency: 'Transparency effects', transparencyDesc: 'Windows and surfaces appear slightly translucent',
    animations: 'Animation effects', animationsDesc: 'Windows, menus and switches move smoothly. Turn this off for a calmer screen.',
    contrastInfo: 'Special contrast themes aren’t available in Win11 Shell yet.',
    contrastTips: 'Tips for easier reading',
    tipDark: 'Light or dark', tipDarkDesc: 'Choose the mode that reads best for you',
    tipTransparency: 'Turn off transparency', tipTransparencyDesc: 'Solid backgrounds give more contrast',
    light: 'Light', dark: 'Dark',
    quick: 'Quick adjustments',
  },
});

export const pages = {
  async accessibility(ctx) {
    return [
      SettingsGroup({ title: t('vision'), children: [linkCard(ctx, 'textsize'), linkCard(ctx, 'visualeffects'), linkCard(ctx, 'contrast')] }),
      SettingsGroup({ title: t('quick'), children: [
        toggleCard(ctx, { icon: 'sparkle', title: t('animations'), description: t('animationsDesc'), path: 'accessibility.reduceMotion', invert: true }),
      ] }),
    ];
  },

  async textsize(ctx) {
    const cur = () => Math.round(Number(getSetting('accessibility.textScale', 100)) || 100);
    let pending = cur();
    const sample = h('div', { class: 'st-ts-sample__text' }, t('sample'));
    const valueEl = h('span', { class: 'st-value tabular' });
    const apply = Button({
      label: t('apply'), variant: 'accent',
      onClick: async () => {
        try {
          await setSetting('accessibility.textScale', pending);
          Toast.show({ severity: 'success', title: t('applied', { v: pending }), timeout: 2200 });
        } catch (e) { showError(e); }
        paint();
      },
    });
    const slider = Slider({
      min: 100, max: 225, step: 5, value: pending, width: 260, ariaLabel: t('size'), format: (v) => `${v}%`,
      onInput: (v) => { pending = v; paint(); },
      onChange: (v) => { pending = v; paint(); },
    });
    function paint() {
      // the page itself is already scaled by --ts; the sample shows the size relative to 100 %
      sample.style.fontSize = `calc(14px * ${pending / 100})`;
      sample.style.lineHeight = `calc(20px * ${pending / 100})`;
      valueEl.textContent = `${pending}%`;
      apply.disabled = pending === cur();
    }
    paint();
    ctx.onSettings(() => { if (pending === cur() || !document.activeElement?.closest?.('.fl-slider')) { pending = cur(); slider.setValue(pending); paint(); } });
    return [
      h('section', { class: 'st-ts-sample', 'aria-live': 'polite' }, h('div', { class: 'st-ts-sample__label' }, t('sampleTitle')), sample),
      SettingsGroup({ children: [SettingsCard({ icon: 'zoom-in', title: t('size'), description: t('sizeDesc'), control: h('div', { class: 'st-row' }, slider, valueEl, apply) })] }),
    ];
  },

  async visualeffects(ctx) {
    return [SettingsGroup({ children: [
      toggleCard(ctx, { icon: 'color', title: t('transparency'), description: t('transparencyDesc'), path: 'theme.transparency' }),
      toggleCard(ctx, { icon: 'sparkle', title: t('animations'), description: t('animationsDesc'), path: 'accessibility.reduceMotion', invert: true }),
    ] })];
  },

  async contrast(ctx) {
    return [
      notice({ message: t('contrastInfo') }),
      SettingsGroup({ title: t('contrastTips'), children: [
        SettingsCard({ icon: 'theme', title: t('tipDark'), description: t('tipDarkDesc'),
          control: settingDropdown(ctx, 'theme.mode', [{ value: 'light', label: t('light') }, { value: 'dark', label: t('dark') }], { width: 160 }) }),
        toggleCard(ctx, { icon: 'color', title: t('tipTransparency'), description: t('tipTransparencyDesc'), path: 'theme.transparency', invert: true }),
        linkCard(ctx, 'textsize'),
      ] }),
    ];
  },
};
