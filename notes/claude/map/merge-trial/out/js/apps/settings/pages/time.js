// Instellingen › Tijd en taal (overview with a live clock) + Datum en tijd + Taal en regio.

import { h } from '../../../core/dom.js';
import { makeT, lang } from '../../../core/i18n.js';
import { bridge } from '../../../core/bridge.js';
import { getSetting, setSetting } from '../../../core/theme.js';
import { time, date, number, bytes } from '../../../core/format.js';
import { SettingsCard, SettingsGroup, SettingsExpander, ToggleSwitch, Dropdown } from '../../../core/components.js';
import { get, toggleCard, dropdownCard, linkCard, showError, notice } from '../common.js';

const t = makeT({
  nl: {
    tz: 'Tijdzone: {tz}',
    nl: 'Nederlands', en: 'English',
    langLabel: 'Taal: {l}',
    autoTime: 'Tijd automatisch instellen',
    autoTimeOn: 'Aan – de klok wordt via internet gelijkgezet',
    autoTimeOff: 'Uit',
    managedByUbuntu: 'Dit regelt Ubuntu. Wijzigen kan in de app Instellingen van Ubuntu: Systeem › Datum en tijd.',
    timezone: 'Tijdzone',
    tzUnknown: 'Onbekend',
    display: 'Weergave',
    h24: '24-uursnotatie', h24Desc: 'Bijvoorbeeld {a} in plaats van {b}',
    seconds: 'Seconden weergeven', secondsDesc: 'In de klok op de taakbalk',
    showDate: 'Datum weergeven op de taakbalk', showDateDesc: 'Onder de tijd, rechtsonder',
    firstDay: 'Eerste dag van de week', firstDayDesc: 'In de agenda van de taakbalk',
    mon: 'Maandag', sun: 'Zondag', sat: 'Zaterdag',
    displayLang: 'Weergavetaal van Win11 Shell',
    displayLangDesc: 'De taal van Instellingen, Start, de taakbalk en de andere onderdelen van Win11 Shell',
    langNote: 'Apps van Ubuntu gebruiken hun eigen taalinstelling.',
    formats: 'Notaties',
    formatsDesc: 'Zo worden datum, tijd en getallen weergegeven',
    fDate: 'Korte datum', fLong: 'Lange datum', fTime: 'Tijd', fNumber: 'Getal', fSize: 'Bestandsgrootte',
    related: 'Gerelateerde instellingen',
  },
  en: {
    tz: 'Time zone: {tz}',
    nl: 'Nederlands', en: 'English',
    langLabel: 'Language: {l}',
    autoTime: 'Set time automatically',
    autoTimeOn: 'On – the clock is synchronised over the internet',
    autoTimeOff: 'Off',
    managedByUbuntu: 'Ubuntu manages this. Change it in Ubuntu’s Settings app: System › Date & Time.',
    timezone: 'Time zone',
    tzUnknown: 'Unknown',
    display: 'Display',
    h24: '24-hour clock', h24Desc: 'For example {a} instead of {b}',
    seconds: 'Show seconds', secondsDesc: 'In the taskbar clock',
    showDate: 'Show date on the taskbar', showDateDesc: 'Below the time, bottom right',
    firstDay: 'First day of week', firstDayDesc: 'In the taskbar calendar',
    mon: 'Monday', sun: 'Sunday', sat: 'Saturday',
    displayLang: 'Win11 Shell display language',
    displayLangDesc: 'The language of Settings, Start, the taskbar and the other parts of Win11 Shell',
    langNote: 'Ubuntu apps use their own language setting.',
    formats: 'Formats',
    formatsDesc: 'How dates, times and numbers are shown',
    fDate: 'Short date', fLong: 'Long date', fTime: 'Time', fNumber: 'Number', fSize: 'File size',
    related: 'Related settings',
  },
});

const tzName = (tz) => (tz ? tz.replace(/_/g, ' ').replace('/', ' / ') : '');

function secondsToggle(ctx) {
  const read = () => !!(getSetting('time.showSeconds', false) || getSetting('taskbar.showSeconds', false));
  const sw = ToggleSwitch({
    checked: read(), ariaLabel: t('seconds'),
    onChange: async (on) => {
      try { await bridge.call('settings.patch', { patch: { time: { showSeconds: on }, taskbar: { showSeconds: on } } }); }
      catch (e) { sw.setChecked(!on); showError(e); }
    },
  });
  ctx.onSettings(() => sw.setChecked(read()));
  return sw;
}

export const pages = {
  async time(ctx) {
    const st = await get('system.time');
    const clock = h('div', { class: 'st-clock__time tabular' });
    const day = h('div', { class: 'st-clock__date' });
    const tick = () => {
      const now = Date.now();
      clock.textContent = time(now, { h24: getSetting('time.h24', true) !== false, seconds: !!getSetting('time.showSeconds', false) });
      day.textContent = date(now, 'long');
    };
    tick();
    ctx.interval(tick, 1000);
    ctx.onSettings(tick);
    const hero = h('section', { class: 'st-clock' }, clock, day,
      h('div', { class: 'st-clock__meta' },
        st?.timezone ? h('span', null, t('tz', { tz: tzName(st.timezone) })) : null,
        h('span', null, t('langLabel', { l: t(lang()) }))));
    return [
      hero,
      SettingsGroup({ children: [linkCard(ctx, 'datetime'), linkCard(ctx, 'language')] }),
    ];
  },

  async datetime(ctx) {
    const st = await get('system.time');
    const ntp = ToggleSwitch({ checked: !!st?.ntp, disabled: true, ariaLabel: t('autoTime') });
    const tzDd = Dropdown({ options: [{ value: 'tz', label: st?.timezone ? tzName(st.timezone) : t('tzUnknown') }], value: 'tz', disabled: true, width: 260, ariaLabel: t('timezone') });
    const now = new Date(2026, 9, 25, 14, 35).getTime();
    return [
      SettingsGroup({ children: [
        SettingsCard({ icon: 'clock', title: t('autoTime'), description: st ? (st.ntp ? t('autoTimeOn') : t('autoTimeOff')) : t('managedByUbuntu'), control: st ? ntp : null }),
        SettingsCard({ icon: 'globe', title: t('timezone'), description: t('managedByUbuntu'), control: tzDd }),
      ] }),
      SettingsGroup({ title: t('display'), children: [
        toggleCard(ctx, { icon: 'clock', title: t('h24'), description: t('h24Desc', { a: time(now, { h24: true }), b: time(now, { h24: false }) }), path: 'time.h24' }),
        SettingsCard({ icon: 'timer', title: t('seconds'), description: t('secondsDesc'), control: secondsToggle(ctx) }),
        toggleCard(ctx, { icon: 'calendar', title: t('showDate'), description: t('showDateDesc'), path: 'time.showDate' }),
        dropdownCard(ctx, { icon: 'calendar', title: t('firstDay'), description: t('firstDayDesc'), path: 'time.firstDayOfWeek', width: 180, fallback: 1,
          options: [{ value: 1, label: t('mon') }, { value: 0, label: t('sun') }, { value: 6, label: t('sat') }] }),
      ] }),
    ];
  },

  async language(ctx) {
    const dd = Dropdown({
      // endonyms, as Windows lists display languages (D6): each language in its own name, whatever the UI language
      options: [{ value: 'nl', label: 'Nederlands (Nederland)' }, { value: 'en', label: 'English (United Kingdom)' }],
      value: getSetting('language', 'nl'), width: 260, ariaLabel: t('displayLang'),
      onChange: async (v) => { try { await setSetting('language', v); } catch (e) { dd.setValue(getSetting('language', 'nl')); showError(e); } },
    });
    ctx.onSettings(() => dd.setValue(getSetting('language', 'nl')));
    const sample = new Date(2026, 9, 25, 14, 35).getTime();
    const rows = [
      [t('fDate'), date(sample, 'short')], [t('fLong'), date(sample, 'long')],
      [t('fTime'), time(sample, { h24: getSetting('time.h24', true) !== false })],
      [t('fNumber'), number(1234567.89, 2)], [t('fSize'), bytes(1288490188)],
    ];
    return [
      SettingsGroup({ children: [
        SettingsCard({ icon: 'language', title: t('displayLang'), description: t('displayLangDesc'), control: dd }),
        notice({ message: t('langNote') }),
      ] }),
      SettingsGroup({ children: [
        SettingsExpander({ icon: 'globe', title: t('formats'), description: t('formatsDesc'), expanded: true,
          children: [h('dl', { class: 'st-specs' }, rows.map(([k, v]) => [h('dt', null, k), h('dd', null, v)]))] }),
      ] }),
    ];
  },
};
