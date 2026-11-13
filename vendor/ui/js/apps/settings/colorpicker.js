// Win11 Shell — Instellingen: custom colour dialog ("Een aangepaste kleur kiezen").
// Hue/saturation spectrum + brightness slider + hex field, with a readability check
// for accent colours. pickColor({value, title, accent}) → Promise<'#rrggbb' | null>.

import { h, clamp } from '../../core/dom.js';
import { makeT } from '../../core/i18n.js';
import { normalizeHex, accentPalette, contrast } from '../../core/theme.js';
import { Dialog, Slider, TextBox } from '../../core/components.js';

const t = makeT({
  nl: {
    title: 'Een aangepaste kleur kiezen',
    spectrum: 'Kleur en verzadiging',
    brightness: 'Helderheid',
    hex: 'Hexadecimaal',
    badHex: 'Typ een kleur als #0078D4.',
    done: 'Gereed',
    cancel: 'Annuleren',
    preview: 'Voorbeeld',
    lightMode: 'Licht',
    darkMode: 'Donker',
    hard: 'Deze kleur is misschien moeilijk leesbaar. Kies een iets lichtere of donkerdere kleur.',
  },
  en: {
    title: 'Pick a custom colour',
    spectrum: 'Hue and saturation',
    brightness: 'Brightness',
    hex: 'Hexadecimal',
    badHex: 'Type a colour like #0078D4.',
    done: 'Done',
    cancel: 'Cancel',
    preview: 'Preview',
    lightMode: 'Light',
    darkMode: 'Dark',
    hard: 'This colour may be hard to read. Pick a slightly lighter or darker colour.',
  },
});

export function hexToHsv(hex) {
  const n = parseInt((normalizeHex(hex) || '#0078d4').slice(1), 16);
  const r = ((n >> 16) & 255) / 255, g = ((n >> 8) & 255) / 255, b = (n & 255) / 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), d = max - min;
  let hue = 0;
  if (d) {
    if (max === r) hue = ((g - b) / d) % 6;
    else if (max === g) hue = (b - r) / d + 2;
    else hue = (r - g) / d + 4;
    hue = (hue * 60 + 360) % 360;
  }
  return [hue, max ? d / max : 0, max];
}

export function hsvToHex(hh, s, v) {
  const f = (n) => {
    const k = (n + hh / 60) % 6;
    return v - v * s * Math.max(0, Math.min(k, 4 - k, 1));
  };
  return '#' + [f(5), f(3), f(1)].map((x) => Math.round(x * 255).toString(16).padStart(2, '0')).join('');
}

export async function pickColor({ value = '#0078d4', title = t('title'), accent = true } = {}) {
  let [hue, sat, val] = hexToHsv(value);
  let hex = normalizeHex(value) || '#0078d4';

  const thumb = h('span', { class: 'st-cp__thumb' });
  const dim = h('span', { class: 'st-cp__dim' });
  const spectrum = h('div', {
    class: 'st-cp__spectrum', tabindex: 0, role: 'slider', 'aria-label': t('spectrum'),
  }, dim, thumb);
  const bright = Slider({ min: 0, max: 100, step: 1, value: Math.round(val * 100), ariaLabel: t('brightness'), width: '100%',
    onInput: (v) => { val = v / 100; fromHsv(); } });
  // parsed live, so Enter (which confirms the dialog) always uses what was typed
  const hexBox = TextBox({ header: t('hex'), value: hex.toUpperCase(), width: 150, clearable: false, maxLength: 7,
    onInput: (v) => {
      const n = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.test(v.trim()) ? normalizeHex(v) : null;
      if (!n) return;
      hexBox.setError('');
      hex = n; [hue, sat, val] = hexToHsv(n); bright.setValue(Math.round(val * 100)); paint(false);
    },
    onChange: (v) => { if (!normalizeHex(v)) hexBox.setError(t('badHex')); } });
  const swLight = h('span', { class: 'st-cp__chip' }, t('lightMode'));
  const swDark = h('span', { class: 'st-cp__chip' }, t('darkMode'));
  const swatch = h('div', { class: 'st-cp__swatch' });
  const warn = h('div', { class: 'st-cp__warn', role: 'status' });

  function fromHsv() { hex = hsvToHex(hue, sat, val); paint(true); }
  function paint(updateBox) {
    spectrum.style.setProperty('--x', `${(hue / 360) * 100}%`);
    spectrum.style.setProperty('--y', `${(1 - sat) * 100}%`);
    dim.style.opacity = String(1 - val);
    thumb.style.background = hex;
    swatch.style.background = hex;
    if (updateBox) { hexBox.setValue(hex.toUpperCase()); hexBox.setError(''); }
    if (accent) {
      const p = accentPalette(hex);
      swLight.style.background = p.dark1; swLight.style.color = p.onLight;
      swDark.style.background = p.light2; swDark.style.color = p.onDark;
      const worst = Math.min(contrast(p.dark1, p.onLight), contrast(p.light2, p.onDark), contrast(p.dark1, '#f3f3f3'), contrast(p.light2, '#202020'));
      warn.textContent = worst < 2.2 ? t('hard') : '';
      warn.hidden = worst >= 2.2;
    }
  }

  const fromPoint = (e) => {
    const r = spectrum.getBoundingClientRect();
    hue = clamp((e.clientX - r.left) / r.width, 0, 1) * 360;
    sat = 1 - clamp((e.clientY - r.top) / r.height, 0, 1);
    if (val < 0.15) { val = 0.85; bright.setValue(85); }
    fromHsv();
  };
  let drag = false;
  spectrum.addEventListener('pointerdown', (e) => { drag = true; spectrum.setPointerCapture(e.pointerId); spectrum.focus(); fromPoint(e); });
  spectrum.addEventListener('pointermove', (e) => { if (drag) fromPoint(e); });
  spectrum.addEventListener('pointerup', () => { drag = false; });
  spectrum.addEventListener('keydown', (e) => {
    const step = e.shiftKey ? 10 : 2;
    if (e.key === 'ArrowLeft') hue = (hue - step + 360) % 360;
    else if (e.key === 'ArrowRight') hue = (hue + step) % 360;
    else if (e.key === 'ArrowUp') sat = clamp(sat + step / 100, 0, 1);
    else if (e.key === 'ArrowDown') sat = clamp(sat - step / 100, 0, 1);
    else return;
    e.preventDefault();
    fromHsv();
  });

  const content = h('div', { class: 'st-cp' },
    spectrum,
    h('div', { class: 'st-cp__label' }, t('brightness')), bright,
    h('div', { class: 'st-cp__row' },
      h('div', { class: 'st-cp__prev' }, swatch, accent ? h('div', { class: 'st-cp__chips' }, swLight, swDark) : null),
      hexBox),
    accent ? warn : null);
  paint(true);
  const r = await Dialog.show({ title, content, primary: t('done'), close: t('cancel'), width: 420 });
  return r === 'primary' ? hex : null;
}
