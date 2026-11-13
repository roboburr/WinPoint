// Instellingen › Systeem › Beeldscherm: monitors (arrangement, scale, resolution, refresh
// rate with the Windows "Wijzigingen behouden?" countdown), brightness and night light.

import { h, clear, animate } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { getSetting, setSetting, getShellInfo } from '../../../core/theme.js';
import {
  SettingsCard, SettingsGroup, SettingsExpander, Slider, ToggleSwitch, Dropdown, Dialog, Button, Toast,
} from '../../../core/components.js';
import { call, run, notice, unavailableCard, linkCard, showError } from '../common.js';

const t = makeT({
  nl: {
    arrangement: 'Beeldschermen rangschikken',
    arrangementHint: 'Selecteer een beeldscherm om de instellingen ervan te wijzigen.',
    identify: 'Identificeren',
    identifyToast: 'Beeldscherm {n}',
    brightnessColor: 'Helderheid en kleur',
    brightness: 'Helderheid',
    brightnessDesc: 'De helderheid van het ingebouwde beeldscherm aanpassen',
    brightnessNA: 'Alleen beschikbaar voor ingebouwde schermen, zoals van een laptop',
    night: 'Nachtlampje',
    nightDesc: 'Warmere kleuren gebruiken om blauw licht te verminderen',
    strength: 'Sterkte',
    strengthDesc: 'Hoe warmer, hoe minder blauw licht',
    nightNA: 'Nachtlampje werkt alleen in de Win11 Shell-sessie.',
    nightMissing: 'Nachtlampje is op deze computer nog niet beschikbaar: er ontbreekt een onderdeel.',
    nightFailed: 'Nachtlampje kon niet worden ingesteld',
    scaleLayout: 'Schaal en indeling', scaleLayoutN: 'Schaal en indeling · Beeldscherm {n}',
    scale: 'Schaal',
    scaleDesc: 'De grootte van tekst, apps en andere items wijzigen',
    resolution: 'Beeldschermresolutie',
    resolutionDesc: 'De resolutie aanpassen aan het aangesloten beeldscherm',
    refresh: 'Vernieuwingsfrequentie',
    refreshDesc: 'Een hogere frequentie geeft vloeiendere beweging, maar kost meer stroom',
    recommended: '{v} (aanbevolen)',
    hz: '{n} Hz',
    displaysNA: 'Beeldschermen instellen kan alleen in de Win11 Shell-sessie. Gebruik hier de app Instellingen van Ubuntu.',
    keepTitle: 'Deze beeldscherminstellingen behouden?',
    // glossary-ok: Terugzetten (Windows' own en text says "Reverting")
    keepBody: { one: 'Terugzetten naar de vorige beeldscherminstellingen over {n} seconde.',
      other: 'Terugzetten naar de vorige beeldscherminstellingen over {n} seconden.' },
    keep: 'Wijzigingen behouden',
    revert: 'Terugzetten',
    reverted: 'De vorige beeldscherminstellingen zijn teruggezet',
    displayN: 'Beeldscherm {n}',
    primaryLabel: 'Hoofdbeeldscherm',
    related: 'Gerelateerde instellingen',
    textSizeDesc: 'Tekst groter maken in Win11 Shell',
  },
  en: {
    arrangement: 'Rearrange your displays',
    arrangementHint: 'Select a display to change its settings.',
    identify: 'Identify',
    identifyToast: 'Display {n}',
    brightnessColor: 'Brightness & colour',
    brightness: 'Brightness',
    brightnessDesc: 'Adjust the brightness of the built-in display',
    brightnessNA: 'Only available for built-in screens, such as a laptop’s',
    night: 'Night light',
    nightDesc: 'Use warmer colours to help block blue light',
    strength: 'Strength',
    strengthDesc: 'The warmer, the less blue light',
    nightNA: 'Night light only works in the Win11 Shell session.',
    nightMissing: 'Night light isn’t available on this computer yet: a component is missing.',
    nightFailed: 'Night light couldn’t be set',
    scaleLayout: 'Scale & layout', scaleLayoutN: 'Scale & layout · Display {n}',
    scale: 'Scale',
    scaleDesc: 'Change the size of text, apps and other items',
    resolution: 'Display resolution',
    resolutionDesc: 'Adjust the resolution to fit your connected display',
    refresh: 'Refresh rate',
    refreshDesc: 'A higher rate gives smoother motion but uses more power',
    recommended: '{v} (recommended)',
    hz: '{n} Hz',
    displaysNA: 'Displays can only be set up in the Win11 Shell session. Here, use Ubuntu’s Settings app.',
    keepTitle: 'Keep these display settings?',
    keepBody: { one: 'Reverting to previous display settings in {n} second.', other: 'Reverting to previous display settings in {n} seconds.' },
    keep: 'Keep changes',
    revert: 'Revert',
    reverted: 'The previous display settings were restored',
    displayN: 'Display {n}',
    primaryLabel: 'Main display',
    related: 'Related settings',
    textSizeDesc: 'Make text bigger in Win11 Shell',
  },
});

const SCALES = [1, 1.25, 1.5, 1.75, 2, 2.25, 2.5, 3];

const parseMode = (m) => {
  const r = String(m).match(/^(\d+)x(\d+)(?:@([\d.]+))?/);
  return r ? { w: +r[1], h: +r[2], hz: r[3] ? +r[3] : 60, raw: m } : null;
};

function currentMode(mon) {
  const modes = (mon.modes || []).map(parseMode).filter(Boolean);
  const same = modes.filter((m) => m.w === mon.width && m.h === mon.height);
  if (!same.length) return `${mon.width}x${mon.height}@${Math.round(mon.refresh || 60)}`;
  same.sort((a, b) => Math.abs(a.hz - mon.refresh) - Math.abs(b.hz - mon.refresh));
  return same[0].raw;
}

/** Windows-style 15 s confirmation; resolves true to keep the change. */
export function keepChangesDialog(seconds = 15) {
  let n = seconds;
  const text = h('p', { class: 'st-countdown', 'aria-live': 'polite' }, t('keepBody', { n }));
  const p = Dialog.show({ title: t('keepTitle'), content: text, primary: t('keep'), close: t('revert'), defaultButton: 'primary' });
  const tm = setInterval(() => {
    n -= 1;
    text.textContent = t('keepBody', { n: Math.max(0, n) });
    if (n <= 0) { clearInterval(tm); p.dialog.close('close'); }
  }, 1000);
  return p.then((r) => { clearInterval(tm); return r === 'primary'; });
}

function Arrangement(monitors, selected, onSelect) {
  const area = h('div', { class: 'st-arrange', role: 'radiogroup', 'aria-label': t('arrangement') });
  const tiles = [];
  const draw = () => {
    clear(area);
    tiles.length = 0;
    const rects = monitors.map((m) => ({ m, x: m.x || 0, y: m.y || 0, w: m.width / (m.scale || 1), h: m.height / (m.scale || 1) }));
    const minX = Math.min(...rects.map((r) => r.x)), minY = Math.min(...rects.map((r) => r.y));
    const maxX = Math.max(...rects.map((r) => r.x + r.w)), maxY = Math.max(...rects.map((r) => r.y + r.h));
    const aw = area.clientWidth || 640, ah = 190;
    const k = Math.min((aw - 48) / (maxX - minX), (ah - 40) / (maxY - minY));
    const ox = (aw - (maxX - minX) * k) / 2, oy = (ah - (maxY - minY) * k) / 2;
    rects.forEach((r, i) => {
      const on = r.m.name === selected();
      const tile = h('button', {
        type: 'button', class: ['st-arrange__mon', on && 'is-selected'], role: 'radio', 'aria-checked': String(on),
        'aria-label': `${t('displayN', { n: i + 1 })}: ${r.m.description || r.m.name}`,
        dataset: { tooltip: r.m.description || r.m.name },
        style: { left: `${ox + (r.x - minX) * k + 2}px`, top: `${oy + (r.y - minY) * k + 2}px`, width: `${r.w * k - 4}px`, height: `${r.h * k - 4}px` },
        onClick: () => onSelect(r.m.name),
      }, h('span', { class: 'st-arrange__num' }, String(i + 1)));
      tiles.push(tile);
      area.append(tile);
    });
  };
  area.redraw = draw;
  area.flash = (name) => {
    const i = monitors.findIndex((m) => m.name === name);
    if (tiles[i]) animate(tiles[i], [{ transform: 'scale(1)' }, { transform: 'scale(1.06)' }, { transform: 'scale(1)' }], { duration: 400 });
  };
  if ('ResizeObserver' in window) new ResizeObserver(() => draw()).observe(area);
  requestAnimationFrame(draw);
  return area;
}

export const pages = {
  async display(ctx) {
    const [dispR, brightR, nightR] = await Promise.all([call('display.list'), call('system.brightness.get'), call('system.nightLight.get')]);
    const out = [];

    /* ---- monitors ------------------------------------------------------------ */
    let data = dispR.ok ? dispR.value : null;
    let supported = !!(data && data.supported && data.monitors?.length);
    let sel = supported ? (data.monitors.find((m) => m.focused) || data.monitors[0]).name : null;
    const scaleGroupSlot = h('div', { class: 'st-slot' });
    let arrangement = null;

    async function reload() {
      const r = await call('display.list');
      if (r.ok && r.value?.supported) data = r.value;
      if (arrangement) { arrangement.remove(); }
      renderMonitors();
    }

    async function applyChange(mon, change) {
      const before = { mode: currentMode(mon), scale: mon.scale || 1 };
      const res = await run('display.set', { name: mon.name, ...change }, { quietDry: true });
      if (!res) { renderScale(); return; }
      await reload();
      const keep = await keepChangesDialog(15);
      if (!keep) {
        const back = 'scale' in change ? { scale: before.scale } : { mode: before.mode };
        await run('display.set', { name: mon.name, ...back }, { quietDry: true });
        Toast.show({ severity: 'info', title: t('reverted'), timeout: 3000 });
        await reload();
      }
    }

    function renderScale() {
      clear(scaleGroupSlot);
      if (!supported) {
        // GNOME etc.: the host still reports the monitors read-only → show the current values greyed out (like Windows
        // shows settings it cannot change), otherwise "not available" cards
        const ro = data?.monitors?.length ? (data.monitors.find((m) => m.focused) || data.monitors[0]) : null;
        const fixed = (label) => Dropdown({ options: [{ value: 'cur', label }], value: 'cur', disabled: true, width: 220 });
        scaleGroupSlot.append(SettingsGroup({
          title: t('scaleLayout'),
          children: ro ? [
            notice({ message: t('displaysNA') }),
            SettingsCard({ icon: 'zoom-in', title: t('scale'), description: t('scaleDesc'), control: fixed(`${Math.round((ro.scale || 1) * 100)}%`) }),
            SettingsCard({ icon: 'display', title: t('resolution'), description: t('resolutionDesc'), control: fixed(`${ro.width} × ${ro.height}`) }),
            SettingsCard({ icon: 'refresh', title: t('refresh'), description: t('refreshDesc'), control: fixed(t('hz', { n: Math.round(ro.refresh || 60) })) }),
          ] : [
            notice({ message: t('displaysNA') }),
            unavailableCard({ icon: 'zoom-in', title: t('scale'), description: t('scaleDesc') }),
            unavailableCard({ icon: 'display', title: t('resolution'), description: t('resolutionDesc') }),
          ],
        }));
        return;
      }
      const mon = data.monitors.find((m) => m.name === sel) || data.monitors[0];
      const modes = (mon.modes || []).map(parseMode).filter(Boolean);
      const cur = parseMode(currentMode(mon)) || { w: mon.width, h: mon.height, hz: mon.refresh };

      // scale
      const scaleOpts = [...new Set([...SCALES, mon.scale || 1])].sort((a, b) => a - b);
      const recScale = mon.width >= 3800 ? 1.5 : mon.width >= 3000 ? 1.25 : 1;
      const scaleDd = Dropdown({
        options: scaleOpts.map((s) => ({ value: s, label: s === recScale ? t('recommended', { v: `${Math.round(s * 100)}%` }) : `${Math.round(s * 100)}%` })),
        value: mon.scale || 1, width: 220, ariaLabel: t('scale'),
        onChange: (v) => applyChange(mon, { scale: v }),
      });

      // resolution
      const resList = [];
      for (const m of modes) if (!resList.some((r) => r.w === m.w && r.h === m.h)) resList.push({ w: m.w, h: m.h });
      resList.sort((a, b) => b.w * b.h - a.w * a.h);
      if (!resList.some((r) => r.w === cur.w && r.h === cur.h)) resList.unshift({ w: cur.w, h: cur.h });
      const best = resList[0];
      const resDd = Dropdown({
        options: resList.map((r) => {
          const v = `${r.w} × ${r.h}`;
          return { value: `${r.w}x${r.h}`, label: r === best ? t('recommended', { v }) : v };
        }),
        value: `${cur.w}x${cur.h}`, width: 220, ariaLabel: t('resolution'),
        onChange: (v) => {
          const [w, hh] = v.split('x').map(Number);
          const cands = modes.filter((m) => m.w === w && m.h === hh).sort((a, b) => b.hz - a.hz);
          const pick = cands.find((m) => Math.round(m.hz) === Math.round(cur.hz)) || cands[0];
          applyChange(mon, { mode: pick ? pick.raw : `${w}x${hh}` });
        },
      });

      // refresh rate
      const rates = modes.filter((m) => m.w === cur.w && m.h === cur.h).sort((a, b) => b.hz - a.hz);
      const rateDd = Dropdown({
        options: (rates.length ? rates : [cur]).map((m) => ({ value: m.raw || `${cur.w}x${cur.h}@${cur.hz}`, label: t('hz', { n: Math.round(m.hz) }) })),
        value: rates.length ? currentMode(mon) : `${cur.w}x${cur.h}@${cur.hz}`, width: 220, ariaLabel: t('refresh'),
        onChange: (v) => applyChange(mon, { mode: v }),
      });

      const idx = data.monitors.indexOf(mon) + 1;
      scaleGroupSlot.append(SettingsGroup({
        title: data.monitors.length > 1 ? t('scaleLayoutN', { n: idx }) : t('scaleLayout'),
        children: [
          SettingsCard({ icon: 'zoom-in', title: t('scale'), description: t('scaleDesc'), control: scaleDd }),
          SettingsCard({ icon: 'display', title: t('resolution'), description: t('resolutionDesc'), control: resDd }),
          SettingsCard({ icon: 'refresh', title: t('refresh'), description: t('refreshDesc'), control: rateDd }),
        ],
      }));
    }

    const monitorsSlot = h('div', { class: 'st-slot' });
    function renderMonitors() {
      clear(monitorsSlot);
      if (supported && data.monitors.length > 1) {
        arrangement = Arrangement(data.monitors, () => sel, (name) => { sel = name; arrangement.redraw(); renderScale(); });
        const mon = () => data.monitors.find((m) => m.name === sel);
        const card = h('div', { class: 'st-arrange-card' },
          arrangement,
          h('div', { class: 'st-arrange-card__foot' },
            h('span', { class: 'st-arrange-card__hint' }, t('arrangementHint')),
            Button({
              label: t('identify'),
              onClick: () => {
                const n = data.monitors.findIndex((m) => m.name === sel) + 1;
                arrangement.flash(sel);
                Toast.show({ title: t('identifyToast', { n }), message: `${mon()?.description || sel} · ${mon()?.width} × ${mon()?.height}`, timeout: 2500 });
              },
            })));
        monitorsSlot.append(card);
      }
      renderScale();
    }
    renderMonitors();
    out.push(monitorsSlot);
    // a monitor plugged in or out, or changed elsewhere (host: display.changed {monitors})
    ctx.on('display.changed', (d) => {
      if (!Array.isArray(d?.monitors) || !data) return;
      data = { ...data, monitors: d.monitors };
      supported = !!(data.supported && data.monitors.length);
      if (supported && !data.monitors.some((m) => m.name === sel)) sel = (data.monitors.find((m) => m.focused) || data.monitors[0]).name;
      renderMonitors();
    });

    /* ---- brightness & night light -------------------------------------------- */
    const b = brightR.ok ? brightR.value : null;
    let brightnessCard;
    if (b && b.available) {
      const slider = Slider({
        min: 0, max: 100, step: 1, value: Math.round(b.value ?? 50), width: 220, ariaLabel: t('brightness'), format: (v) => String(v),
        onInput: (() => { let tm = 0; return (v) => { clearTimeout(tm); tm = setTimeout(() => call('system.brightness.set', { value: v }), 60); }; })(),
        onChange: (v) => run('system.brightness.set', { value: v }, { quietDry: true }),
      });
      ctx.on('system.brightness.changed', (d) => { if (d?.value != null) slider.setValue(Math.round(d.value)); });
      brightnessCard = SettingsCard({ icon: 'brightness', title: t('brightness'), description: t('brightnessDesc'), control: slider });
    } else {
      brightnessCard = unavailableCard({ icon: 'brightness', title: t('brightness'), description: t('brightnessNA') });
    }

    const tempToStrength = (k) => Math.round(Math.max(0, Math.min(100, (6500 - (k || 4500)) / 40)));
    const strengthToTemp = (s) => Math.round(6500 - s * 40);
    let nightWarned = false;
    async function applyNight(enabled, temperature) {
      const r = await call('system.nightLight.set', { enabled, temperature });
      if (!r.ok && !nightWarned) {
        nightWarned = true;
        if (r.error?.code === 'unknown_method' || r.error?.code === 'unsupported') nightNote.hidden = false;
        else showError(r.error, t('nightFailed'));
      }
    }
    // in the Win11 Shell session the only reason left is the missing night-light helper
    const nightNAText = () => (getShellInfo()?.compositor === 'hyprland' ? t('nightMissing') : t('nightNA'));
    const nightNote = notice({ message: nightNAText() });
    nightNote.hidden = true;
    const nightToggle = ToggleSwitch({
      checked: !!getSetting('nightLight.enabled', false), ariaLabel: t('night'),
      onChange: async (on) => {
        try { await setSetting('nightLight.enabled', on); } catch (e) { showError(e); }
        applyNight(on, getSetting('nightLight.temperature', 4500));
      },
    });
    const strength = Slider({
      min: 0, max: 100, step: 1, value: tempToStrength(getSetting('nightLight.temperature', 4500)), width: 220, ariaLabel: t('strength'),
      onChange: async (v) => {
        const k = strengthToTemp(v);
        try { await setSetting('nightLight.temperature', k); } catch (e) { showError(e); }
        if (getSetting('nightLight.enabled', false)) applyNight(true, k);
      },
    });
    ctx.onSettings(() => {
      nightToggle.setChecked(!!getSetting('nightLight.enabled', false));
      strength.setValue(tempToStrength(getSetting('nightLight.temperature', 4500)));
    });
    // system.nightLight.get().supported (host-services-b) decides; an older host without the method keeps the toggle
    const nl = nightR.ok ? nightR.value : null;
    const nightSupported = !nl || (nl.supported ?? nl.available) !== false;
    let nightCard;
    if (nightSupported) {
      nightCard = SettingsExpander({
        icon: 'night-light', title: t('night'), description: t('nightDesc'), control: nightToggle,
        children: [
          SettingsCard({ title: t('strength'), description: t('strengthDesc'), control: strength }),
          h('div', { class: 'st-expander-note' }, nightNote),
        ],
      });
      // show the note only when needed
      nightCard.content.lastElementChild.hidden = true;
      const showNote = () => { nightCard.content.lastElementChild.hidden = nightNote.hidden; };
      new MutationObserver(showNote).observe(nightNote, { attributes: true, attributeFilter: ['hidden'] });
    } else {
      nightCard = unavailableCard({ icon: 'night-light', title: t('night'), description: nightNAText() });
    }

    out.push(SettingsGroup({ title: t('brightnessColor'), children: [brightnessCard, nightCard] }));
    out.push(scaleGroupSlot);

    out.push(SettingsGroup({
      title: t('related'),
      children: [
        linkCard(ctx, 'textsize', { description: t('textSizeDesc') }),
        linkCard(ctx, 'colors'),
      ],
    }));
    return out;
  },
};
