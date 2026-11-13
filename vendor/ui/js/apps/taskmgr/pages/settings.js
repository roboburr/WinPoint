// Taakbeheer — Instellingen (settings.taskmgr.*).

import { h } from '../../../core/dom.js';
import { SettingsGroup, SettingsCard, Dropdown, ToggleSwitch, Toast } from '../../../core/components.js';
import { getSetting, setSetting, onSettings, getShellInfo } from '../../../core/theme.js';
import { makeT } from '../../../core/i18n.js';
import { load, save, leafOutline } from '../util.js';

const t = makeT({
  nl: {
    title: 'Instellingen', general: 'Algemeen', graphs: 'Grafieken', safety: 'Veiligheid', about: 'Over Taakbeheer',
    speed: 'Realtime-updatesnelheid', speedDesc: 'Hoe vaak Taakbeheer de gegevens vernieuwt. Langzamer gebruikt minder stroom.',
    high: 'Hoog (elke 0,5 seconde)', normal: 'Normaal (elke seconde)', low: 'Laag (elke 4 seconden)', paused: 'Gepauzeerd',
    page: 'Standaardstartpagina', pageDesc: 'De pagina die je ziet als je Taakbeheer opent.',
    onTop: 'Altijd op voorgrond', onTopDesc: 'Taakbeheer blijft boven andere vensters en is op elk bureaublad te zien.',
    confirm: 'Vragen voordat een taak wordt beëindigd', confirmDesc: 'Je krijgt eerst uitleg, zodat je geen werk kwijtraakt.',
    effConfirm: 'Vragen voordat efficiëntiemodus aangaat', effConfirmDesc: 'Uitleg over wat efficiëntiemodus doet.',
    colors: 'Kleurgecodeerde grafieken', colorsDesc: 'Processor blauw, geheugen paars, schijf groen, netwerk oranje en videokaart groenblauw. Uit = alles in je accentkleur.',
    duration: 'Grafiekduur', durationDesc: 'Hoeveel geschiedenis de grafieken laten zien.',
    s30: '30 seconden', s60: '60 seconden', s120: '2 minuten', s300: '5 minuten',
    aboutTitle: 'Taakbeheer voor Win11 Shell', aboutDesc: 'Versie {v}. Alle gegevens komen van je eigen pc; er wordt niets verstuurd.',
    privacy: 'Over je privacy', saveFailed: 'Kon de instelling niet opslaan',
    summary: 'Samenvatting', processes: 'Processen', performance: 'Prestaties', storage: 'Opslag', startup: 'Opstart-apps', services: 'Services', details: 'Details',
    testMode: 'Testmodus: acties worden niet echt uitgevoerd',
  },
  en: {
    title: 'Settings', general: 'General', graphs: 'Graphs', safety: 'Safety', about: 'About Task Manager',
    speed: 'Real-time update speed', speedDesc: 'How often Task Manager refreshes its data. Slower uses less power.',
    high: 'High (every 0.5 seconds)', normal: 'Normal (every second)', low: 'Low (every 4 seconds)', paused: 'Paused',
    page: 'Default start page', pageDesc: 'The page you see when you open Task Manager.',
    onTop: 'Always on top', onTopDesc: 'Keeps Task Manager above other windows and on every desktop.',
    confirm: 'Ask before ending a task', confirmDesc: 'You’ll get an explanation first, so you don’t lose any work.',
    effConfirm: 'Ask before turning on efficiency mode', effConfirmDesc: 'Explains what efficiency mode does.',
    colors: 'Colour-keyed graphs', colorsDesc: 'Processor blue, memory purple, disk green, network orange and graphics teal. Off = everything in your accent colour.',
    duration: 'Graph length', durationDesc: 'How much history the graphs show.',
    s30: '30 seconds', s60: '60 seconds', s120: '2 minutes', s300: '5 minutes',
    aboutTitle: 'Task Manager for Win11 Shell', aboutDesc: 'Version {v}. All data comes from your own PC; nothing is sent anywhere.',
    privacy: 'About your privacy', saveFailed: 'Couldn’t save the setting',
    summary: 'Summary', processes: 'Processes', performance: 'Performance', storage: 'Storage', startup: 'Startup apps', services: 'Services', details: 'Details',
    testMode: 'Test mode: actions aren’t really carried out',
  },
});

export function createPage(ctx) {
  const set = (path, value, revert) => setSetting(path, value).catch((e) => {
    Toast.show({ title: t('saveFailed'), message: e.message || '', severity: 'error' });
    revert?.();
  });

  const speed = Dropdown({
    options: ['high', 'normal', 'low', 'paused'].map((v) => ({ value: v, label: t(v) })),
    value: getSetting('taskmgr.updateSpeed', 'normal'), width: 240, ariaLabel: t('speed'),
    onChange: (v) => set('taskmgr.updateSpeed', v, () => speed.setValue(getSetting('taskmgr.updateSpeed', 'normal'))),
  });
  const page = Dropdown({
    options: ['summary', 'processes', 'performance', 'storage', 'startup', 'services', 'details'].map((v) => ({ value: v, label: t(v) })),
    value: getSetting('taskmgr.defaultPage', 'summary'), width: 240, ariaLabel: t('page'),
    onChange: (v) => set('taskmgr.defaultPage', v, () => page.setValue(getSetting('taskmgr.defaultPage', 'summary'))),
  });
  const duration = Dropdown({
    options: [30, 60, 120, 300].map((v) => ({ value: v, label: t(`s${v}`) })),
    value: getSetting('taskmgr.graphSeconds', 60), width: 240, ariaLabel: t('duration'),
    onChange: (v) => set('taskmgr.graphSeconds', v, () => duration.setValue(getSetting('taskmgr.graphSeconds', 60))),
  });
  const onTop = ToggleSwitch({ checked: getSetting('taskmgr.alwaysOnTop', false), onChange: (on) => set('taskmgr.alwaysOnTop', on, () => onTop.setChecked(!on)) });
  const confirm = ToggleSwitch({ checked: getSetting('taskmgr.confirmEndTask', true), onChange: (on) => set('taskmgr.confirmEndTask', on, () => confirm.setChecked(!on)) });
  const colors = ToggleSwitch({ checked: getSetting('taskmgr.colorKeyedGraphs', true), onChange: (on) => set('taskmgr.colorKeyedGraphs', on, () => colors.setChecked(!on)) });
  const effConfirm = ToggleSwitch({ checked: !load('effNoConfirm', false), onChange: (on) => save('effNoConfirm', !on) });

  onSettings(() => {
    speed.setValue(getSetting('taskmgr.updateSpeed', 'normal'));
    page.setValue(getSetting('taskmgr.defaultPage', 'summary'));
    duration.setValue(getSetting('taskmgr.graphSeconds', 60));
    onTop.setChecked(getSetting('taskmgr.alwaysOnTop', false));
    confirm.setChecked(getSetting('taskmgr.confirmEndTask', true));
    colors.setChecked(getSetting('taskmgr.colorKeyedGraphs', true));
  });

  const info = getShellInfo() || {};
  const el = h('div', { class: 'tm-scroll tm-settings' },
    SettingsGroup({ title: t('general'), children: [
      SettingsCard({ icon: 'refresh', title: t('speed'), description: t('speedDesc'), control: speed }),
      SettingsCard({ icon: 'home', title: t('page'), description: t('pageDesc'), control: page }),
      SettingsCard({ icon: 'window', title: t('onTop'), description: t('onTopDesc'), control: onTop }),
    ] }),
    SettingsGroup({ title: t('graphs'), children: [
      SettingsCard({ icon: 'color', title: t('colors'), description: t('colorsDesc'), control: colors }),
      SettingsCard({ icon: 'clock', title: t('duration'), description: t('durationDesc'), control: duration }),
    ] }),
    SettingsGroup({ title: t('safety'), children: [
      SettingsCard({ icon: 'shield', title: t('confirm'), description: t('confirmDesc'), control: confirm }),
      SettingsCard({ icon: leafOutline(20), title: t('effConfirm'), description: t('effConfirmDesc'), control: effConfirm }),
    ] }),
    SettingsGroup({ title: t('about'), children: [
      SettingsCard({ icon: 'info', title: t('aboutTitle'), description: t('aboutDesc', { v: info.version || '–' }) + (info.dryRun ? ` ${t('testMode')}.` : '') }),
    ] }),
  );

  return { id: 'settings', title: t('title'), el, commands: [], mount() {}, unmount() {} };
}
