// Instellingen › Systeem › Energie en batterij: native GNOME power settings,
// battery status and the existing, confirmed power actions.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { getSetting } from '../../../core/theme.js';
import { duration, number } from '../../../core/format.js';
import { icon } from '../../../core/icons.js';
import { SettingsCard, SettingsGroup, Button, Dialog, Dropdown, ToggleSwitch } from '../../../core/components.js';
import { call, run, toggleCard, UsageBar, showError, noteDryRun } from '../common.js';

const t = makeT({
  nl: {
    energy: 'Energie',
    mode: 'Energiestand', modeDesc: 'Kies de balans tussen energieverbruik en prestaties',
    'profile-power-saver': 'Beste energie-efficiëntie',
    'profile-balanced': 'Gebalanceerd', 'profile-performance': 'Beste prestaties',
    profileUnavailable: 'Op deze computer kan de energiestand niet worden aangepast.',
    profileDegraded: 'De prestaties zijn momenteel beperkt door de computer.',
    profileInhibited: 'Beste prestaties is momenteel niet beschikbaar.',
    hot: 'De computer is te warm.', lap: 'De computer staat op schoot.',
    screenSleep: 'Scherm en slaapstand',
    screen: 'Scherm uitschakelen na', screenDesc: 'Als je de computer niet gebruikt',
    dim: 'Scherm dimmen bij inactiviteit', dimDesc: 'De helderheid van het ingebouwde scherm verlagen om stroom te besparen',
    sleepAC: 'Op netstroom in slaapstand na', sleepACDesc: 'Automatisch slapen als je de computer niet gebruikt',
    sleepBattery: 'Op batterij in slaapstand na', sleepBatteryDesc: 'Automatisch slapen om de batterij te sparen',
    never: 'Nooit', customTime: '{time} (aangepast)', seconds: { one: '1 seconde', other: '{n} seconden' },
    currentAction: 'Huidige instelling: {action} na {time}',
    'action-hibernate': 'sluimerstand', 'action-shutdown': 'afsluiten', 'action-blank': 'scherm uit',
    'action-interactive': 'vragen', 'action-logout': 'afmelden', 'action-suspend-then-hibernate': 'slaapstand, daarna sluimerstand',
    unavailable: 'Niet beschikbaar', unavailableDesc: 'Deze instelling is op deze computer niet beschikbaar.',
    suspendUnavailable: 'Deze computer ondersteunt automatisch slapen momenteel niet.',
    readOnly: 'Deze instelling wordt beheerd en kan hier niet worden gewijzigd.',
    settingsFailed: 'De energie-instellingen konden niet worden geladen.', retry: 'Opnieuw proberen',
    powerGroup: 'Aan/uit',
    lock: 'Vergrendelen', lockDesc: 'Het scherm vergrendelen; je apps blijven open',
    signout: 'Afmelden', signoutDesc: 'Je sessie afsluiten; open apps worden gesloten',
    sleep: 'Slaapstand', sleepDesc: 'Zuinig pauzeren; je werk blijft open staan',
    restart: 'Opnieuw opstarten', restartDesc: 'De computer uit- en weer aanzetten',
    shutdown: 'Afsluiten', shutdownDesc: 'De computer helemaal uitzetten',
    confirm: 'Altijd eerst vragen',
    confirmDesc: 'Vragen voordat de computer wordt afgesloten, opnieuw opgestart of afgemeld',
    qShutdown: 'Computer afsluiten?', bShutdown: 'Sla eerst je werk op. Alle open apps worden gesloten en de computer gaat uit.',
    qRestart: 'Opnieuw opstarten?', bRestart: 'Sla eerst je werk op. Alle open apps worden gesloten en de computer start opnieuw op.',
    qSignout: 'Afmelden?', bSignout: 'Sla eerst je werk op. Alle open apps worden gesloten.',
    battery: 'Batterij',
    charging: 'Wordt opgeladen', discharging: 'Op batterij', full: 'Volledig opgeladen', onAC: 'Aangesloten op netstroom',
    left: 'Nog ongeveer {d}', toFull: 'Vol over ongeveer {d}',
    health: 'Batterijconditie', healthDesc: 'Hoeveel de batterij nog kan opslaan vergeleken met nieuw',
    cycles: 'Laadcycli', cyclesDesc: 'Hoe vaak de batterij volledig is opgeladen',
    rate: 'Huidig stroomverbruik', rateDesc: 'Hoeveel energie de computer nu gebruikt',
    noBattery: 'Netstroom',
    noBatteryDesc: 'Deze computer heeft geen batterij en werkt altijd op netstroom.',
    healthGood: 'Goed', healthFair: 'Redelijk', healthWorn: 'Versleten',
  },
  en: {
    energy: 'Power',
    mode: 'Power mode', modeDesc: 'Choose the balance between power use and performance',
    'profile-power-saver': 'Best power efficiency',
    'profile-balanced': 'Balanced', 'profile-performance': 'Best performance',
    profileUnavailable: 'The power mode can’t be changed on this computer.',
    profileDegraded: 'Performance is currently limited by the computer.',
    profileInhibited: 'Best performance is currently unavailable.',
    hot: 'The computer is too hot.', lap: 'The computer is on your lap.',
    screenSleep: 'Screen & sleep',
    screen: 'Turn off my screen after', screenDesc: 'When you’re not using the computer',
    dim: 'Dim the screen when inactive', dimDesc: 'Lower the brightness of the built-in screen to save power',
    sleepAC: 'When plugged in, put my device to sleep after', sleepACDesc: 'Sleep automatically when you’re not using the computer',
    sleepBattery: 'On battery, put my device to sleep after', sleepBatteryDesc: 'Sleep automatically to save battery power',
    never: 'Never', customTime: '{time} (custom)', seconds: { one: '1 second', other: '{n} seconds' },
    currentAction: 'Current setting: {action} after {time}',
    'action-hibernate': 'hibernate', 'action-shutdown': 'shut down', 'action-blank': 'turn off the screen',
    'action-interactive': 'ask', 'action-logout': 'sign out', 'action-suspend-then-hibernate': 'sleep, then hibernate',
    unavailable: 'Not available', unavailableDesc: 'This setting isn’t available on this computer.',
    suspendUnavailable: 'This computer doesn’t support automatic sleep right now.',
    readOnly: 'This setting is managed and can’t be changed here.',
    settingsFailed: 'The power settings couldn’t be loaded.', retry: 'Try again',
    powerGroup: 'Shut down or sign out',
    lock: 'Lock', lockDesc: 'Lock the screen; your apps stay open',
    signout: 'Sign out', signoutDesc: 'End your session; open apps will close',
    sleep: 'Sleep', sleepDesc: 'Pause and save power; your work stays open',
    restart: 'Restart', restartDesc: 'Turn the computer off and on again',
    shutdown: 'Shut down', shutdownDesc: 'Turn the computer off completely',
    confirm: 'Always ask first',
    confirmDesc: 'Ask before the computer shuts down, restarts or signs out',
    qShutdown: 'Shut down the computer?', bShutdown: 'Save your work first. All open apps will close and the computer will turn off.',
    qRestart: 'Restart?', bRestart: 'Save your work first. All open apps will close and the computer will restart.',
    qSignout: 'Sign out?', bSignout: 'Save your work first. All open apps will close.',
    battery: 'Battery',
    charging: 'Charging', discharging: 'On battery', full: 'Fully charged', onAC: 'Plugged in',
    left: 'About {d} left', toFull: 'Full in about {d}',
    health: 'Battery health', healthDesc: 'How much the battery can still hold compared to new',
    cycles: 'Charge cycles', cyclesDesc: 'How often the battery has been fully charged',
    rate: 'Current power use', rateDesc: 'How much energy the computer uses right now',
    noBattery: 'Mains power',
    noBatteryDesc: 'This computer has no battery and always runs on mains power.',
    healthGood: 'Good', healthFair: 'Fair', healthWorn: 'Worn',
  },
});

const ACTIONS = [
  { action: 'lock', icon: 'lock', confirm: false },
  { action: 'logout', key: 'signout', icon: 'signout', confirm: true, q: 'qSignout', b: 'bSignout' },
  { action: 'suspend', key: 'sleep', icon: 'sleep', confirm: false },
  { action: 'reboot', key: 'restart', icon: 'restart', confirm: true, q: 'qRestart', b: 'bRestart' },
  { action: 'shutdown', key: 'shutdown', icon: 'power', confirm: true, q: 'qShutdown', b: 'bShutdown' },
];

export async function powerAction(a) {
  const key = a.key || a.action;
  if (a.confirm && getSetting('power.confirm', true) !== false) {
    const ok = await Dialog.confirm({ title: t(a.q), message: t(a.b), confirmLabel: t(key), danger: a.action !== 'logout' });
    if (!ok) return;
  }
  await run('system.power.action', { action: a.action });
}

function batteryHero(b) {
  const pct = Math.round(b.percent ?? 0);
  const state = b.state === 'charging' ? t('charging') : b.state === 'fully-charged' || b.state === 'full' ? t('full') : b.onAC ? t('onAC') : t('discharging');
  const time = b.state === 'charging' && b.timeToFull ? t('toFull', { d: duration(b.timeToFull, { long: true }) })
    : !b.onAC && b.timeToEmpty ? t('left', { d: duration(b.timeToEmpty, { long: true }) }) : '';
  const bar = UsageBar({ pct, color: pct <= 15 ? 'var(--sys-critical)' : pct <= 30 ? 'var(--sys-caution)' : 'var(--sys-success)', warnAt: 101 });
  return h('section', { class: 'st-battery-hero' },
    h('div', { class: 'st-battery-hero__icon' }, icon(b.state === 'charging' ? 'battery-charging' : 'battery', { size: 40 })),
    h('div', { class: 'st-battery-hero__text' },
      h('div', { class: 'st-battery-hero__pct' }, `${pct}%`),
      h('div', { class: 'st-battery-hero__state' }, [state, time].filter(Boolean).join(' · ')),
      bar));
}

const SCREEN_TIMES = [60, 120, 180, 300, 600, 900, 1800, 3600];
const SLEEP_TIMES = [...SCREEN_TIMES, 7200];
const PROFILE_NAMES = new Set(['power-saver', 'balanced', 'performance']);
const OTHER_ACTIONS = new Set(['hibernate', 'shutdown', 'blank', 'interactive', 'logout', 'suspend-then-hibernate']);
const validTime = (v) => Number.isInteger(v) && v >= 0;
const timeLabel = (v) => v % 60 ? t('seconds', { n: v }) : duration(v, { long: true });

function timeOptions(times, current) {
  const values = [...times];
  if (validTime(current) && current > 0 && !values.includes(current)) values.push(current);
  return [...values.sort((a, b) => a - b).map((value) => ({
    value, label: times.includes(value) ? timeLabel(value) : t('customTime', { time: timeLabel(value) }),
  })), { value: 0, label: t('never') }];
}

function sleepValue(s) {
  if (s?.action === 'nothing' || s?.timeout === 0) return 0;
  if (s?.action === 'suspend' && validTime(s.timeout)) return s.timeout;
  return `current:${s?.action || ''}:${s?.timeout}`;
}

function sleepOptions(s) {
  const value = sleepValue(s);
  const options = timeOptions(SLEEP_TIMES, typeof value === 'number' ? value : null);
  if (typeof value === 'string') {
    const action = OTHER_ACTIONS.has(s?.action) ? t(`action-${s.action}`) : String(s?.action || '–');
    options.unshift({ value, label: t('currentAction', { action, time: validTime(s?.timeout) ? timeLabel(s.timeout) : '–' }), disabled: true });
  }
  return options;
}

/** Keep the same Fluent controls mounted while GNOME settings change elsewhere.
 * A popup keeps its option snapshot until it closes; writes are serialized and
 * followed by native readback, without changing the page or losing keyboard focus.
 */
function EnergySettings(ctx, initial, batteryPresent) {
  let state = initial || {}, pending = false, revision = 0;
  const controls = [];
  const profile = Dropdown({ ariaLabel: t('mode'), width: 230, placeholder: t('unavailable'),
    onChange: (value) => save({ powerProfile: value }, () => state.profiles?.available && state.profiles?.writable
      && state.profiles.choices?.some((p) => p.id === value && !p.disabled)) });
  const screen = Dropdown({ ariaLabel: t('screen'), width: 200, placeholder: t('unavailable'),
    onChange: (value) => save({ idleDelay: value }, () => state.screen?.available && state.screen?.writable) });
  const dim = ToggleSwitch({ ariaLabel: t('dim'),
    onChange: (value) => save({ idleDim: value }, () => state.dim?.available && state.dim?.writable) });
  const sleepControls = {};
  for (const key of ['sleepAC', 'sleepBattery']) {
    sleepControls[key] = Dropdown({ ariaLabel: t(key), width: 200, placeholder: t('unavailable'),
      onChange: (value) => {
        const current = state[key] || {};
        const timeout = value === 0 ? (validTime(current.timeout) ? current.timeout : 1800) : value;
        save({ [key]: { action: value === 0 ? 'nothing' : 'suspend', timeout } },
          () => typeof value === 'number' && state.canSuspend && current.available && current.writable);
      } });
  }
  const profileCard = SettingsCard({ icon: 'lightning', title: t('mode'), control: profile });
  const screenCard = SettingsCard({ icon: 'display', title: t('screen'), control: screen });
  const dimCard = SettingsCard({ icon: 'brightness', title: t('dim'), control: dim });
  const sleepCards = Object.fromEntries(['sleepAC', 'sleepBattery'].map((key) => [key,
    SettingsCard({ icon: 'sleep', title: t(key), control: sleepControls[key] }),
  ]));
  for (const [key, control] of Object.entries({ profile, screen, dim, ...sleepControls })) {
    control.dataset.energyControl = key;
    controls.push(control);
  }
  const groups = [SettingsGroup({ title: t('energy'), children: [profileCard] }),
    SettingsGroup({ title: t('screenSleep'), children: [screenCard, dimCard, sleepCards.sleepAC, sleepCards.sleepBattery] })];

  const isOpen = () => controls.some((c) => (c.button || c).getAttribute('aria-expanded') === 'true');
  const settingDescription = (setting, description, supported = true) => !setting?.available ? t('unavailableDesc')
    : !supported ? t('suspendUnavailable') : !setting.writable ? t('readOnly') : description;

  function sync() {
    if (!ctx.alive() || pending || isOpen()) return;
    const p = state.profiles || {};
    const options = (p.choices || []).map((choice) => ({ value: choice.id,
      label: PROFILE_NAMES.has(choice.id) ? t(`profile-${choice.id}`) : String(choice.id), disabled: !!choice.disabled }));
    if (p.available && p.active && !options.some((o) => o.value === p.active)) {
      options.push({ value: p.active, label: String(p.active), disabled: true });
    }
    profile.setOptions(options, p.active);
    profile.setDisabled(pending || !p.available || !p.writable || !options.length);
    let profileDesc = !p.available ? t('profileUnavailable') : !p.writable ? t('readOnly') : t('modeDesc');
    if (p.available && (p.inhibited || p.degraded)) {
      profileDesc += ` ${t(p.inhibited ? 'profileInhibited' : 'profileDegraded')}`;
      const reason = p.inhibited || p.degraded;
      if (reason === 'high-operating-temperature') profileDesc += ` ${t('hot')}`;
      if (reason === 'lap-detected') profileDesc += ` ${t('lap')}`;
    }
    profileCard.setDescription(profileDesc);
    screen.setOptions(state.screen?.available ? timeOptions(SCREEN_TIMES, state.screen.idleDelay) : [], state.screen?.idleDelay);
    screen.setDisabled(pending || !state.screen?.available || !state.screen?.writable);
    screenCard.setDescription(settingDescription(state.screen, t('screenDesc')));
    dim.setChecked(!!state.dim?.enabled);
    dim.setDisabled(pending || !state.dim?.available || !state.dim?.writable);
    dimCard.hidden = !state.dim?.available;
    dimCard.setDescription(settingDescription(state.dim, t('dimDesc')));
    for (const key of ['sleepAC', 'sleepBattery']) {
      const s = state[key];
      sleepControls[key].setOptions(s?.available ? sleepOptions(s) : [], sleepValue(s));
      sleepControls[key].setDisabled(pending || !s?.available || !s.writable || !state.canSuspend);
      sleepCards[key].setDescription(settingDescription(s, t(`${key}Desc`), !!state.canSuspend));
    }
    sleepCards.sleepBattery.hidden = !batteryPresent;
  }

  async function save(patch, writable) {
    if (!ctx.alive()) return;
    if (pending || !writable()) { sync(); return; }
    pending = true;
    for (const control of controls) control.setDisabled(true);
    const beforeWrite = revision;
    const result = await call('system.power.settings.set', patch);
    if (!ctx.alive()) return;
    if (!result.ok) showError(result.error);
    else {
      noteDryRun(result.value);
      if (revision === beforeWrite && result.value) state = result.value;
    }
    const beforeRead = revision;
    const fresh = await call('system.power.settings.get');
    if (!ctx.alive()) return;
    if (fresh.ok && revision === beforeRead) state = fresh.value || {};
    pending = false;
    sync();
  }

  const observer = new MutationObserver(() => { if (!isOpen()) sync(); });
  for (const control of controls) observer.observe(control.button || control, { attributes: true, attributeFilter: ['aria-expanded'] });
  ctx.onCleanup(() => observer.disconnect());
  sync();
  return {
    groups,
    accept(value) { if (!ctx.alive() || !value || typeof value !== 'object') return; state = value; revision++; if (!pending) sync(); },
    setBatteryPresent(value) { batteryPresent = !!value; sync(); },
  };
}

export const pages = {
  async power(ctx) {
    let energy = null, queuedEnergy = null;
    ctx.on('system.power.settings.changed', (value) => { if (energy) energy.accept(value); else queuedEnergy = value; });
    const [br, er] = await Promise.all([call('system.battery.get'), call('system.power.settings.get')]);
    if (!ctx.alive()) return [];
    let b = br.ok ? br.value : null;
    const out = [];
    const batterySlot = h('div', { class: 'st-slot' });
    const mainsSlot = h('div', { class: 'st-slot' });
    function updateBattery(value) {
      b = value;
      batterySlot.replaceChildren();
      mainsSlot.replaceChildren();
      energy?.setBatteryPresent(!!b?.present);
      if (!b?.present) {
        mainsSlot.append(SettingsGroup({ title: t('battery'), children: [SettingsCard({ icon: 'lightning', title: t('noBattery'), description: t('noBatteryDesc') })] }));
        return;
      }
      batterySlot.append(batteryHero(b));
      const cards = [];
      if (b.health != null) {
        const hv = Math.round(b.health);
        cards.push(SettingsCard({ icon: 'heart', title: t('health'), description: t('healthDesc'),
          control: h('span', { class: 'st-value' }, `${hv}% · ${hv >= 80 ? t('healthGood') : hv >= 60 ? t('healthFair') : t('healthWorn')}`) }));
      }
      if (b.cycles != null) cards.push(SettingsCard({ icon: 'refresh', title: t('cycles'), description: t('cyclesDesc'), control: h('span', { class: 'st-value' }, number(b.cycles)) }));
      if (b.energyRate != null) cards.push(SettingsCard({ icon: 'lightning', title: t('rate'), description: t('rateDesc'), control: h('span', { class: 'st-value' }, `${number(b.energyRate, 1)} W`) }));
      if (cards.length) batterySlot.append(SettingsGroup({ title: t('battery'), children: cards }));
    }
    out.push(batterySlot);
    if (er.ok || queuedEnergy) {
      energy = EnergySettings(ctx, queuedEnergy || er.value, !!b?.present);
      out.push(...energy.groups);
    } else out.push(SettingsGroup({ title: t('energy'), children: [SettingsCard({ icon: 'lightning', title: t('settingsFailed'),
      description: er.error?.message || t('unavailableDesc'), control: Button({ label: t('retry'), onClick: () => ctx.refresh() }) })] }));
    updateBattery(b);
    ctx.on('system.battery.changed', (value) => updateBattery(value));

    out.push(SettingsGroup({
      title: t('powerGroup'),
      children: [
        ...ACTIONS.map((a) => {
          const key = a.key || a.action;
          return SettingsCard({
            icon: a.icon, title: t(key), description: t(`${key}Desc`),
            control: Button({ label: t(key), onClick: () => powerAction(a) }),
          });
        }),
        toggleCard(ctx, { icon: 'shield', title: t('confirm'), description: t('confirmDesc'), path: 'power.confirm' }),
      ],
    }));

    out.push(mainsSlot);
    return out;
  },
};
