// Instellingen › Systeem › Geluid: output device, volume and mute.

import { h, clear } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { volumeIcon } from '../../../core/icons.js';
import { SettingsCard, SettingsGroup, SettingsExpander, Slider, IconButton, RadioGroup } from '../../../core/components.js';
import { call, run, notice, unavailableCard, linkCard } from '../common.js';

const t = makeT({
  nl: {
    output: 'Uitvoer',
    chooseOutput: 'Kies waar je geluid wilt afspelen',
    chooseOutputDesc: 'Apps gebruiken het apparaat dat je hier kiest',
    volume: 'Volume',
    volumeDesc: 'Hoe hard het geluid is',
    mute: 'Dempen',
    unmute: 'Dempen opheffen',
    muted: 'Gedempt',
    noDevices: 'Geen geluidsapparaten gevonden',
    na: 'Het geluid kan hier niet worden geregeld. Gebruik de geluidsinstellingen van Ubuntu.',
    noSound: 'Geen geluid',
    related: 'Gerelateerde instellingen',
    notifSounds: 'Geluiden bij meldingen',
    notifSoundsDesc: 'Meldingsgeluiden aan of uit',
  },
  en: {
    output: 'Output',
    chooseOutput: 'Choose where to play sound',
    chooseOutputDesc: 'Apps use the device you choose here',
    volume: 'Volume',
    volumeDesc: 'How loud the sound is',
    mute: 'Mute',
    unmute: 'Unmute',
    muted: 'Muted',
    noDevices: 'No sound devices found',
    na: 'Sound can’t be controlled here. Use Ubuntu’s sound settings.',
    noSound: 'No sound',
    related: 'Related settings',
    notifSounds: 'Notification sounds',
    notifSoundsDesc: 'Turn notification sounds on or off',
  },
});

export const pages = {
  async sound(ctx) {
    const r = await call('system.audio.get');
    const a = r.ok ? r.value : null;
    if (!a || !a.available) {
      return [
        notice({ message: t('na') }),
        SettingsGroup({ title: t('output'), children: [
          unavailableCard({ icon: 'sound', title: t('chooseOutput') }),
          unavailableCard({ icon: 'volume', title: t('volume') }),
        ] }),
      ];
    }
    let state = { ...a };

    /* volume + mute */
    const muteBtn = IconButton({ icon: volumeIcon(state.volume, state.muted), label: state.muted ? t('unmute') : t('mute'), size: 36, iconSize: 18 });
    const valueEl = h('span', { class: 'st-value tabular' });
    const slider = Slider({
      min: 0, max: 100, step: 1, value: Math.round(state.volume ?? 0), width: 240, ariaLabel: t('volume'),
      onInput: (() => { let tm = 0; return (v) => { state.volume = v; paint(); clearTimeout(tm); tm = setTimeout(() => call('system.audio.set', { volume: v }), 50); }; })(),
      onChange: (v) => { state.volume = v; paint(); run('system.audio.set', { volume: v }, { quietDry: true }); },
    });
    muteBtn.addEventListener('click', async () => {
      state.muted = !state.muted;
      paint();
      const ok = await run('system.audio.set', { muted: state.muted }, { quietDry: true });
      if (!ok) { state.muted = !state.muted; paint(); }
    });
    function paint() {
      muteBtn.setIcon(volumeIcon(state.volume, state.muted));
      muteBtn.setLabel(state.muted ? t('unmute') : t('mute'));
      muteBtn.classList.toggle('is-muted', !!state.muted);
      valueEl.textContent = state.muted ? t('muted') : String(Math.round(state.volume));
    }
    paint();
    const volumeCard = SettingsCard({ icon: 'volume', title: t('volume'), description: t('volumeDesc'), control: h('div', { class: 'st-volume' }, muteBtn, slider, valueEl) });

    /* output devices */
    const devSlot = h('div', { class: 'st-radio-list' });
    function renderDevices() {
      clear(devSlot);
      const sinks = state.sinks || [];
      if (!sinks.length) { devSlot.append(h('div', { class: 'text-2' }, t('noDevices'))); return; }
      const def = sinks.find((s) => s.default) || sinks[0];
      devSlot.append(RadioGroup({
        options: sinks.map((s) => ({ value: s.id, label: s.name })),
        value: def.id,
        onChange: async (id) => {
          const ok = await run('system.audio.setSink', { id }, { quietDry: true });
          if (ok) { state.sinks = sinks.map((s) => ({ ...s, default: s.id === id })); updateDesc(); }
          else renderDevices();
        },
      }));
    }
    const outExp = SettingsExpander({
      icon: 'sound', title: t('chooseOutput'), description: '', expanded: true,
      children: [devSlot],
    });
    function updateDesc() {
      const def = (state.sinks || []).find((s) => s.default);
      const desc = outExp.header.querySelector('.fl-sc__desc') || (() => {
        const d = h('div', { class: 'fl-sc__desc' });
        outExp.header.querySelector('.fl-sc__text').append(d);
        return d;
      })();
      desc.textContent = def ? def.name : t('chooseOutputDesc');
    }
    renderDevices();
    updateDesc();

    ctx.on('system.audio.changed', (d) => {
      if (!d) return;
      const sinksChanged = JSON.stringify(d.sinks) !== JSON.stringify(state.sinks);
      state = { ...state, ...d };
      slider.setValue(Math.round(state.volume ?? 0));
      paint();
      if (sinksChanged) { renderDevices(); updateDesc(); }
    });

    return [
      SettingsGroup({ title: t('output'), children: [outExp, volumeCard] }),
      SettingsGroup({ title: t('related'), children: [linkCard(ctx, 'notifications', { title: t('notifSounds'), description: t('notifSoundsDesc') })] }),
    ];
  },
};
