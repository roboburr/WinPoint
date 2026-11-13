// Win11 Shell — mock display.* (owned by the settings agent). SPEC §6.8 + host-services-a additions.
// Two monitors: 27" 2560×1440 @ 144 Hz (main, left) and 24" 1920×1080 @ 60 Hz (right).
// Shapes mirror host/services/display.py: monitor extras `mode` (current, "WxH@R" with R trimmed), `make`, `model`,
// `transform`, `vrr`, `disabled`, `logicalWidth/Height`, `readOnly` (GNOME); `display.set` → `{ok, previous:{mode, scale},
// applied}`; event `display.changed {monitors}`.
// Test overrides: ?mockDisplays=0 → unsupported, no monitors · ?mockDisplays=1 → one monitor ·
// ?mockDisplays=gnome → unsupported but the monitors are listed read-only (what the host reports under GNOME).

const trimHz = (hz) => String(Math.round(Number(hz) * 100) / 100);   // 143.998 → "144", 59.951 → "59.95" like the host
const mode = (w, h, hz) => `${w}x${h}@${trimHz(hz)}`;

const MON_27 = {
  name: 'DP-1',
  description: 'Generic Q27G (27 inch)',
  make: 'GEN', model: 'Q27G',
  width: 2560, height: 1440, refresh: 143.998, scale: 1, x: 0, y: 0, focused: true,
  transform: 0, vrr: false, disabled: false,
  modes: [
    mode(2560, 1440, 143.998), mode(2560, 1440, 119.998), mode(2560, 1440, 99.946), mode(2560, 1440, 59.951),
    mode(1920, 1080, 143.855), mode(1920, 1080, 119.88), mode(1920, 1080, 60), mode(1680, 1050, 59.954),
    mode(1280, 1024, 75.025), mode(1280, 720, 60), mode(1024, 768, 60.004), mode(800, 600, 60.317),
  ],
};
const MON_24 = {
  name: 'HDMI-A-1',
  description: 'Generic P24H (24 inch)',
  make: 'GEN', model: 'P24H',
  width: 1920, height: 1080, refresh: 60, scale: 1, x: 2560, y: 180, focused: false,
  transform: 0, vrr: false, disabled: false,
  modes: [
    mode(1920, 1080, 60), mode(1920, 1080, 59.94), mode(1920, 1080, 50), mode(1680, 1050, 59.88),
    mode(1600, 900, 60), mode(1280, 1024, 60.02), mode(1280, 720, 60), mode(1024, 768, 60),
  ],
};

function err(code, message) { const e = new Error(message); e.code = code; return e; }

const parseMode = (m) => {
  const r = String(m).match(/^(\d+)x(\d+)(?:@([\d.]+))?$/);
  return r ? { w: +r[1], h: +r[2], hz: r[3] ? +r[3] : null } : null;
};

/** Derived fields the host computes for every monitor. */
function finish(mon, readOnly = false) {
  mon.mode = mode(mon.width, mon.height, mon.refresh);
  mon.logicalWidth = Math.round(mon.width / (mon.scale || 1));
  mon.logicalHeight = Math.round(mon.height / (mon.scale || 1));
  if (readOnly) mon.readOnly = true;
  return mon;
}

function state(m) {
  if (!m.state.display) {
    const n = m.params.get('mockDisplays');
    const gnome = n === 'gnome';
    const monitors = n === '0' ? [] : n === '1' || gnome ? [structuredClone(MON_27)] : [structuredClone(MON_27), structuredClone(MON_24)];
    monitors.forEach((mon) => finish(mon, gnome));
    m.state.display = { supported: n !== '0' && !gnome, monitors };
  }
  return m.state.display;
}

/** Keep the right monitor next to the left one after a size change. */
function relayout(st) {
  let x = 0;
  for (const mon of st.monitors) {
    mon.x = x;
    finish(mon);
    x += mon.logicalWidth;
  }
}

export default {
  methods: {
    'display.list': async (args, m) => state(m),

    'display.set': async ({ name, mode: wanted, scale } = {}, m) => {
      const st = state(m);
      if (!st.supported) throw err('unsupported', 'Beeldschermen instellen kan alleen in de Win11 Shell-sessie (Hyprland).');
      const mon = st.monitors.find((x) => x.name === name);
      if (!mon) throw err('not_found', 'Dit beeldscherm is niet gevonden.');
      const previous = { mode: mon.mode, scale: mon.scale };
      if (wanted != null) {
        const p = parseMode(wanted);
        if (!p) throw err('invalid_mode', 'Deze resolutie wordt niet ondersteund.');
        const match = mon.modes.map(parseMode).find((q) => q.w === p.w && q.h === p.h && (p.hz == null || Math.abs(q.hz - p.hz) < 0.5));
        if (!match) throw err('invalid_mode', 'Deze resolutie wordt niet ondersteund.');
        mon.width = match.w; mon.height = match.h; mon.refresh = match.hz;
      }
      if (scale != null) {
        const s = Number(scale);
        if (!(s >= 0.5 && s <= 4)) throw err('invalid_scale', 'Deze schaal wordt niet ondersteund.');
        mon.scale = s;
      }
      relayout(st);
      m.emit('display.changed', { monitors: st.monitors });
      return { ok: true, previous, applied: `${mon.name},${mon.mode},${mon.x}x${mon.y},${mon.scale}` };
    },
  },
  start() {},
};
