// Win11 Shell — Fluent-style outline icons (owned by ui-core). Original drawings.
//
//   icon('settings')                    → <svg class="fl-icon"> 16px, currentColor
//   icon('wifi-2', {size: 20, cls: 'x', title: 'Wifi'})
//   startLogo(24)                       → filled circular Start logo (accent blue gradient)
//
// All glyphs are drawn on a 20×20 grid with round caps/joins; stroke width comes
// from the CSS variable --icon-stroke (default 1.35, see components.css).

const NS = 'http://www.w3.org/2000/svg';

/* ---------- tiny builders --------------------------------------------- */
const P = (d, extra = '') => `<path d="${d}"${extra}/>`;
const C = (cx, cy, r, extra = '') => `<circle cx="${cx}" cy="${cy}" r="${r}"${extra}/>`;
const R = (x, y, w, h, rx = 0, extra = '') => `<rect x="${x}" y="${y}" width="${w}" height="${h}" rx="${rx}"${extra}/>`;
const FILL = ' fill="currentColor" stroke="none"';
const FAINT = ' opacity=".3"';
const dot = (cx, cy, r = 1) => C(cx, cy, r, FILL);

function gear(cx = 10, cy = 10, R1 = 7.6, R2 = 5.7, teeth = 8) {
  const step = (Math.PI * 2) / teeth;
  let d = '';
  for (let i = 0; i < teeth; i++) {
    const a = i * step - Math.PI / 2;
    [[R1, a - step * 0.17], [R1, a + step * 0.17], [R2, a + step * 0.33], [R2, a + step * 0.67]].forEach(([r, ang], j) => {
      d += `${i === 0 && j === 0 ? 'M' : 'L'}${(cx + r * Math.cos(ang)).toFixed(2)} ${(cy + r * Math.sin(ang)).toFixed(2)}`;
    });
  }
  return d + 'Z';
}

function rays(cx, cy, r1, r2, n = 8) {
  let d = '';
  for (let i = 0; i < n; i++) {
    const a = (i * Math.PI * 2) / n;
    d += `M${(cx + r1 * Math.cos(a)).toFixed(2)} ${(cy + r1 * Math.sin(a)).toFixed(2)}L${(cx + r2 * Math.cos(a)).toFixed(2)} ${(cy + r2 * Math.sin(a)).toFixed(2)}`;
  }
  return d;
}

/* ---------- shared parts ----------------------------------------------- */
const SPEAKER = 'M3.5 7.25h2.4l3.7-2.95a.6.6 0 0 1 .98.47v10.46a.6.6 0 0 1-.98.47l-3.7-2.95H3.5a1 1 0 0 1-1-1v-3.5a1 1 0 0 1 1-1Z';
const WAVE1 = 'M12.32 7.54a3 3 0 0 1 0 4.92';
const WAVE2 = 'M13.81 5.41a5.6 5.6 0 0 1 0 9.18';
const WAVE3 = 'M15.3 3.28a8.2 8.2 0 0 1 0 13.44';
const WIFI_A1 = 'M7.45 13.2a3.6 3.6 0 0 1 5.1 0';
const WIFI_A2 = 'M4.91 10.66a7.2 7.2 0 0 1 10.18 0';
const WIFI_A3 = 'M2.36 8.11a10.8 10.8 0 0 1 15.28 0';
const WIFI_DOT = dot(10, 15.75, 1.2);
const BELL = 'M10 2.75a5 5 0 0 0-5 5v2.9l-1.3 2.9a.8.8 0 0 0 .73 1.12h11.14a.8.8 0 0 0 .73-1.12L15 10.65v-2.9a5 5 0 0 0-5-5Z';
const PAGE = 'M6 2.5h5.1c.4 0 .78.16 1.06.44l3.4 3.4c.28.28.44.66.44 1.06V16c0 .83-.67 1.5-1.5 1.5H6c-.83 0-1.5-.67-1.5-1.5V4c0-.83.67-1.5 1.5-1.5Z';
const PAGE_FOLD = 'M11 2.75V6c0 .55.45 1 1 1h3.25';
const FOLDER = 'M2.5 5.75c0-1.1.9-2 2-2h3.08c.53 0 1.04.21 1.41.59L10.5 5.75h5c1.1 0 2 .9 2 2v6.5c0 1.1-.9 2-2 2h-11c-1.1 0-2-.9-2-2Z';
const FOLDER_TAB = 'M2.5 8.25h5.4c.4 0 .78-.16 1.06-.44l1.56-1.56';
const MONITOR = R(2.5, 3.5, 15, 10.5, 1.75);
const STAND = P('M7.25 16.75h5.5M10 14v2.75');
const TRASH_CAN = 'M5 5.25l.85 10.4c.07.9.82 1.6 1.73 1.6h4.84c.9 0 1.66-.7 1.73-1.6L15 5.25';
const BATTERY = R(2, 5.5, 14, 9, 2) + P('M18 8.5v3');
const PIN = '<g transform="rotate(45 10 10)"><path d="M7.25 2.5h5.5M8.25 2.5v4.25L5.75 10h8.5l-2.5-3.25V2.5"/><path d="M10 10v7.5"/></g>';
const CIRCLE = C(10, 10, 7.25);
const SLASH = P('M3 3l14 14');
const CLOCK_HANDS = P('M10 5.75V10l2.75 1.75');
const STAR4 = (cx, cy, s) => `M${cx} ${cy - s}c${(s * 0.18).toFixed(2)} ${(s * 0.62).toFixed(2)} ${(s * 0.38).toFixed(2)} ${(s * 0.82).toFixed(2)} ${s} ${s}c${(-s * 0.62).toFixed(2)} ${(s * 0.18).toFixed(2)} ${(-s * 0.82).toFixed(2)} ${(s * 0.38).toFixed(2)} ${-s} ${s}c${(-s * 0.18).toFixed(2)} ${(-s * 0.62).toFixed(2)} ${(-s * 0.38).toFixed(2)} ${(-s * 0.82).toFixed(2)} ${-s} ${-s}c${(s * 0.62).toFixed(2)} ${(-s * 0.18).toFixed(2)} ${(s * 0.82).toFixed(2)} ${(-s * 0.38).toFixed(2)} ${s} ${-s}Z`;

/* ---------- the set ------------------------------------------------------ */
const ICONS = {
  // shell
  start: R(3, 3, 6.25, 6.25, 1.1, FILL) + R(10.75, 3, 6.25, 6.25, 1.1, FILL) + R(3, 10.75, 6.25, 6.25, 1.1, FILL) + R(10.75, 10.75, 6.25, 6.25, 1.1, FILL),
  search: C(8.5, 8.5, 5.5) + P('M12.6 12.6 17 17'),
  taskview: R(2.75, 6.25, 11, 10, 1.75) + P('M6.25 3.25h9.25c.97 0 1.75.78 1.75 1.75v8.25'),
  widgets: R(3, 3, 6, 8, 1.5) + R(11, 3, 6, 4.5, 1.5) + R(11, 9.5, 6, 7.5, 1.5) + R(3, 13, 6, 4, 1.5),
  apps: R(3, 3, 5.5, 5.5, 1.4) + R(3, 11.5, 5.5, 5.5, 1.4) + R(11.5, 11.5, 5.5, 5.5, 1.4) + P('M14.25 2.6l3.15 3.15-3.15 3.15-3.15-3.15Z'),
  'start-menu': R(4, 2.75, 12, 14.5, 2) + R(6.5, 5.25, 3, 3, 0.75) + R(10.5, 5.25, 3, 3, 0.75) + P('M6.5 11.5h7M6.5 14h4.5'),
  taskbar: R(2.5, 3.5, 15, 13, 2) + P('M2.5 13h15') + dot(7, 14.85, 0.75) + dot(10, 14.85, 0.75) + dot(13, 14.85, 0.75),
  window: R(2.5, 3.5, 15, 13, 2) + P('M2.5 7h15'),
  snap: R(2.5, 3.5, 15, 13, 2) + P('M10 3.5v13M10 10h7.5'),
  desktop: MONITOR + P('M2.5 11.25h15') + STAND,
  display: MONITOR + STAND,
  pc: R(2.5, 4.5, 8.5, 7, 1.25) + P('M5.25 14.5h3M6.75 11.5v3') + R(13, 3.5, 4.5, 13, 1.25) + P('M14.5 6.25h1.5') + dot(15.25, 13.5, 0.7),

  // network & devices
  wifi: WIFI_DOT + P(WIFI_A1) + P(WIFI_A2) + P(WIFI_A3),
  'wifi-3': WIFI_DOT + P(WIFI_A1) + P(WIFI_A2) + P(WIFI_A3),
  'wifi-2': WIFI_DOT + P(WIFI_A1) + P(WIFI_A2) + P(WIFI_A3, FAINT),
  'wifi-1': WIFI_DOT + P(WIFI_A1) + P(WIFI_A2, FAINT) + P(WIFI_A3, FAINT),
  'wifi-off': WIFI_DOT + P(WIFI_A1, FAINT) + P(WIFI_A2, FAINT) + P(WIFI_A3, FAINT) + P('M3.25 3.25l13.5 13.5'),
  ethernet: P('M2.75 5.25c0-.97.78-1.75 1.75-1.75h11c.97 0 1.75.78 1.75 1.75v7.5c0 .97-.78 1.75-1.75 1.75H13.5v1.75c0 .28-.22.5-.5.5H7a.5.5 0 0 1-.5-.5V14.5H4.5c-.97 0-1.75-.78-1.75-1.75Z') + P('M7 7.25v3M10 7.25v3M13 7.25v3'),
  network: R(7, 2.5, 6, 4.5, 1.25) + R(2.5, 13, 6, 4.5, 1.25) + R(11.5, 13, 6, 4.5, 1.25) + P('M10 7v3M5.5 13v-3h9v3'),
  globe: CIRCLE + `<ellipse cx="10" cy="10" rx="3" ry="7.25"/>` + P('M2.75 10h14.5'),
  bluetooth: P('M6 6.5l8 7L10 17V3l4 3.5-8 7'),
  airplane: P('M10 2.25c.83 0 1.5.67 1.5 1.5v3.9l5.2 3.05c.19.11.3.31.3.53v1.02c0 .3-.29.52-.58.44L11.5 11.3v3.45l1.75 1.3c.16.12.25.3.25.5v.68c0 .26-.25.45-.5.38L10 16.9l-3 .71c-.25.07-.5-.12-.5-.38v-.68c0-.2.09-.38.25-.5l1.75-1.3V11.3l-4.92 1.39c-.29.08-.58-.14-.58-.44v-1.02c0-.22.11-.42.3-.53l5.2-3.05v-3.9c0-.83.67-1.5 1.5-1.5Z'),
  hotspot: dot(10, 10, 1.4) + P('M7.17 12.83a4 4 0 0 1 0-5.66M12.83 7.17a4 4 0 0 1 0 5.66M4.7 15.3a7.5 7.5 0 0 1 0-10.6M15.3 4.7a7.5 7.5 0 0 1 0 10.6'),
  cast: P('M2.75 7V5.5c0-.97.78-1.75 1.75-1.75h11c.97 0 1.75.78 1.75 1.75v9c0 .97-.78 1.75-1.75 1.75H11') + P('M2.75 10.5a5.75 5.75 0 0 1 5.75 5.75M2.75 13.5a2.75 2.75 0 0 1 2.75 2.75') + dot(3.25, 16, 0.9),
  usb: P('M6.5 8h7v8c0 .83-.67 1.5-1.5 1.5H8c-.83 0-1.5-.67-1.5-1.5Z') + P('M8 8V3.5a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1V8') + P('M9.1 5h.01M10.9 5h.01'),
  drive: R(3, 4.5, 14, 11, 2) + P('M3 11h14') + dot(14, 13.25, 0.8),
  disk: R(3, 4.5, 14, 11, 2) + P('M3 11h14') + dot(14, 13.25, 0.8),
  storage: R(3, 3.5, 14, 5.5, 1.5) + R(3, 11, 14, 5.5, 1.5) + dot(13.75, 6.25, 0.8) + dot(13.75, 13.75, 0.8),
  cloud: P('M6 15.5a3.5 3.5 0 0 1-.3-7 4.5 4.5 0 0 1 8.7-1 3.75 3.75 0 0 1-.15 8Z'),
  keyboard: R(2, 5, 16, 10, 2) + dot(5.5, 8, 0.8) + dot(8.5, 8, 0.8) + dot(11.5, 8, 0.8) + dot(14.5, 8, 0.8) + dot(7, 10.25, 0.8) + dot(10, 10.25, 0.8) + dot(13, 10.25, 0.8) + P('M6.5 12.5h7'),
  mouse: R(5.25, 2.5, 9.5, 15, 4.75) + P('M10 2.75v4.5'),
  cpu: R(5.5, 5.5, 9, 9, 1.5) + R(8, 8, 4, 4, 0.6) + P('M8 2.75V5.5M12 2.75V5.5M8 14.5v2.75M12 14.5v2.75M2.75 8H5.5M2.75 12H5.5M14.5 8h2.75M14.5 12h2.75'),
  memory: R(2.5, 5.5, 15, 8, 1.5) + P('M5.5 13.5v2.75M10 13.5v2.75M14.5 13.5v2.75') + R(5.25, 8, 3.5, 3, 0.6) + R(11.25, 8, 3.5, 3, 0.6),
  gpu: R(2.5, 5, 15, 9, 1.5) + C(8, 9.5, 2.5) + P('M12 8h3M12 11h3M2.5 14v2.5'),
  phone: R(5.5, 2.5, 9, 15, 2) + P('M9 15h2'),
  devices: R(2.5, 4, 10.5, 8, 1.5) + P('M5 15h5.5M7.75 12v3') + R(14, 6.5, 3.5, 9, 1),
  headphones: P('M3.5 13v-2.5a6.5 6.5 0 0 1 13 0V13') + R(3, 11.5, 3.5, 5.5, 1.5) + R(13.5, 11.5, 3.5, 5.5, 1.5),
  mic: R(7.25, 2.5, 5.5, 9.5, 2.75) + P('M4.5 9.5a5.5 5.5 0 0 0 11 0M10 15v2.5'),
  camera: P('M2.5 7c0-.97.78-1.75 1.75-1.75h1.9l1.1-1.6c.24-.34.62-.65 1.04-.65h3.42c.42 0 .8.31 1.04.65l1.1 1.6h1.9c.97 0 1.75.78 1.75 1.75v7.5c0 .97-.78 1.75-1.75 1.75H4.25c-.97 0-1.75-.78-1.75-1.75Z') + C(10, 10.5, 3),
  print: P('M5.5 7V3.5a1 1 0 0 1 1-1h7a1 1 0 0 1 1 1V7') + P('M5.5 14H4a1.5 1.5 0 0 1-1.5-1.5v-4A1.5 1.5 0 0 1 4 7h12a1.5 1.5 0 0 1 1.5 1.5v4A1.5 1.5 0 0 1 16 14h-1.5') + R(5.5, 11, 9, 6.5, 1),

  // volume / brightness / power
  volume: P(SPEAKER) + P(WAVE1) + P(WAVE2) + P(WAVE3),
  'volume-2': P(SPEAKER) + P(WAVE1) + P(WAVE2),
  'volume-1': P(SPEAKER) + P(WAVE1),
  'volume-mute': P(SPEAKER) + P('M13 8l4 4M17 8l-4 4'),
  sound: P(SPEAKER) + P(WAVE1) + P(WAVE2),
  brightness: C(10, 10, 3.25) + P('M10 6.75a3.25 3.25 0 0 0 0 6.5Z', FILL) + P(rays(10, 10, 5.5, 7)),
  sun: C(10, 10, 3.25) + P(rays(10, 10, 5.6, 7.6)),
  moon: P('M16.2 12.6A6.75 6.75 0 0 1 7.4 3.8a6.75 6.75 0 1 0 8.8 8.8Z'),
  sleep: P('M16.2 12.6A6.75 6.75 0 0 1 7.4 3.8a6.75 6.75 0 1 0 8.8 8.8Z'),
  'night-light': P('M14.75 13.4A6.25 6.25 0 0 1 6.6 5.25a6.25 6.25 0 1 0 8.15 8.15Z') + P(STAR4(14.25, 5.75, 2.25)),
  battery: BATTERY,
  'battery-charging': BATTERY + P('M9.75 7.25 7.75 10h2.75l-2 2.75'),
  'battery-saver': BATTERY + P('M6 12c0-2.6 1.9-4.25 5.25-4.25 0 3-1.7 4.25-3.75 4.25M6 12l2.25-1.85'),
  lightning: P('M11 2.5 4.75 11h5l-1 6.5L15.25 9h-5Z'),
  power: P('M10 2.75v6.5') + P('M6.4 4.75a6.25 6.25 0 1 0 7.2 0'),
  restart: P('M16.25 10A6.25 6.25 0 1 1 14.42 5.58') + P('M14.42 2.08V5.58H10.92'),
  refresh: P('M16.25 10A6.25 6.25 0 1 1 14.42 5.58') + P('M14.42 2.08V5.58H10.92'),
  update: P('M15.75 7.75A6.25 6.25 0 0 0 4.35 6.5') + P('M15.75 3.75v4h-4') + P('M4.25 12.25a6.25 6.25 0 0 0 11.4 1.25') + P('M4.25 16.25v-4h4'),
  lock: R(4.5, 8.75, 11, 8.5, 2) + P('M7 8.75V6.5a3 3 0 0 1 6 0v2.25') + dot(10, 13, 1),
  'lock-open': R(4.5, 8.75, 11, 8.5, 2) + P('M7 8.75V6.5a3 3 0 0 1 5.85-.95') + dot(10, 13, 1),
  key: C(6.5, 13.5, 3.5) + P('M9 11l7.5-7.5M14 6l2 2M12 8l1.5 1.5'),
  signout: P('M8.5 3.25H5.75C4.78 3.25 4 4.03 4 5v10c0 .97.78 1.75 1.75 1.75H8.5') + P('M9 10h8M14 7l3 3-3 3'),
  shield: P('M10 2.5 4.25 4.6c-.3.11-.5.4-.5.72v3.93c0 4.16 2.6 6.84 5.94 8.15.2.08.42.08.62 0 3.34-1.31 5.94-3.99 5.94-8.15V5.32c0-.32-.2-.61-.5-.72Z'),

  // people
  user: C(10, 6.25, 3.25) + P('M5.25 11.5h9.5c.97 0 1.75.78 1.75 1.75 0 2.4-2.6 4-6.5 4s-6.5-1.6-6.5-4c0-.97.78-1.75 1.75-1.75Z'),
  people: C(7.5, 6.5, 2.75) + C(14, 7.25, 2.25) + P('M2.5 15.5c0-2.3 2-3.75 5-3.75s5 1.45 5 3.75') + P('M13.25 11.8c2.4-.1 4.25 1.1 4.25 3.2'),
  accessibility: C(10, 4.25, 1.5) + P('M4 7.25c2 .65 4 .95 6 .95s4-.3 6-.95M10 8.2v3.8M10 12l-2.75 5.25M10 12l2.75 5.25'),
  heart: P('M10 16.5S3 12.6 3 7.6A3.6 3.6 0 0 1 10 6a3.6 3.6 0 0 1 7 1.6c0 5-7 8.9-7 8.9Z'),
  emoji: CIRCLE + P('M7 11.75c.7 1.1 1.75 1.75 3 1.75s2.3-.65 3-1.75') + dot(7.5, 8, 1) + dot(12.5, 8, 1),

  // navigation & chrome
  'chevron-right': P('M7.5 4.25 13.25 10 7.5 15.75'),
  'chevron-left': P('M12.5 4.25 6.75 10l5.75 5.75'),
  'chevron-down': P('M4.25 7.25 10 13l5.75-5.75'),
  'chevron-up': P('M4.25 12.75 10 7l5.75 5.75'),
  'arrow-left': P('M16.5 10H3.75M9 4.75 3.75 10 9 15.25'),
  'arrow-right': P('M3.5 10h12.75M11 4.75 16.25 10 11 15.25'),
  'arrow-up': P('M10 16.5V3.75M4.75 9 10 3.75 15.25 9'),
  'arrow-down': P('M10 3.5v12.75M4.75 11 10 16.25 15.25 11'),
  back: P('M16.5 10H3.75M9 4.75 3.75 10 9 15.25'),
  forward: P('M3.5 10h12.75M11 4.75 16.25 10 11 15.25'),
  close: P('M4.75 4.75l10.5 10.5M15.25 4.75 4.75 15.25'),
  minimize: P('M4 10h12'),
  maximize: R(4, 4, 12, 12, 1.75),
  restore: R(4, 6.5, 9.5, 9.5, 1.5) + P('M6.5 4h7.75c.97 0 1.75.78 1.75 1.75v7.75'),
  more: dot(4.5, 10, 1.25) + dot(10, 10, 1.25) + dot(15.5, 10, 1.25),
  'more-vertical': dot(10, 4.5, 1.25) + dot(10, 10, 1.25) + dot(10, 15.5, 1.25),
  menu: P('M3 5.5h14M3 10h14M3 14.5h14'),
  plus: P('M10 3.75v12.5M3.75 10h12.5'),
  minus: P('M3.75 10h12.5'),
  check: P('M4 10.5l3.75 3.75L16 6'),
  external: P('M8.5 4.5H5.5C4.67 4.5 4 5.17 4 6v8.5c0 .83.67 1.5 1.5 1.5H14c.83 0 1.5-.67 1.5-1.5v-3') + P('M11.5 3.5h5v5M16.5 3.5l-7 7'),
  open: P('M4 10V5.5C4 4.67 4.67 4 5.5 4h9c.83 0 1.5.67 1.5 1.5v9c0 .83-.67 1.5-1.5 1.5H10') + P('M3.5 16.5l6-6M5.5 10.5h4v4'),
  link: P('M8.5 11.5a3 3 0 0 0 4.24 0l2.5-2.5a3 3 0 0 0-4.24-4.24l-.75.75') + P('M11.5 8.5a3 3 0 0 0-4.24 0l-2.5 2.5a3 3 0 0 0 4.24 4.24l.75-.75'),
  'zoom-in': C(8.5, 8.5, 5.5) + P('M12.6 12.6 17 17M8.5 6.25v4.5M6.25 8.5h4.5'),
  'zoom-out': C(8.5, 8.5, 5.5) + P('M12.6 12.6 17 17M6.25 8.5h4.5'),
  undo: P('M4.5 7.5h7.75a4.25 4.25 0 0 1 0 8.5H8') + P('M7.5 4.5l-3 3 3 3'),
  redo: P('M15.5 7.5H7.75a4.25 4.25 0 0 0 0 8.5H12') + P('M12.5 4.5l3 3-3 3'),

  // files
  folder: P(FOLDER) + P(FOLDER_TAB),
  'folder-open': P('M2.75 14.5V5.75c0-1.1.9-2 2-2h2.83c.53 0 1.04.21 1.41.59L10.25 5.6h4.5c1.1 0 2 .9 2 2V9') + P('M2.85 15.1 5 10.2c.24-.55.78-.9 1.38-.9h10.4c.72 0 1.2.74.92 1.4l-1.95 4.5c-.24.55-.78.9-1.38.9H3.8c-.72 0-1.2-.74-.95-1.4Z'),
  file: P(PAGE) + P(PAGE_FOLD),
  document: P(PAGE) + P(PAGE_FOLD) + P('M7.5 10.5h5M7.5 13.5h5'),
  image: R(2.75, 3.75, 14.5, 12.5, 2.25) + C(13, 7.5, 1.5) + P('M3 14.5l4.4-4.4a1.25 1.25 0 0 1 1.77 0l6.1 6.1'),
  music: P('M8 15V4.75l8.5-2v10.25M8 7.75l8.5-2') + C(6, 15, 2) + C(14.5, 12.75, 2),
  video: R(2.5, 5, 10.5, 10, 2) + P('M13 8.6l3.72-2.24a.5.5 0 0 1 .78.43v6.42a.5.5 0 0 1-.78.43L13 11.4'),
  download: P('M10 3.25v9.5M6 9l4 4 4-4M4.25 16.5h11.5'),
  upload: P('M10 13.5V4M6 7.75l4-4 4 4M4.25 16.5h11.5'),
  home: P('M3.25 8.6 9.2 3.5a1.25 1.25 0 0 1 1.6 0l5.95 5.1c.16.14.25.34.25.55v6.6c0 .69-.56 1.25-1.25 1.25H12.5v-4.5a1 1 0 0 0-1-1h-3a1 1 0 0 0-1 1V17H4.25C3.56 17 3 16.44 3 15.75V9.15c0-.21.09-.41.25-.55Z'),
  trash: P('M3.5 5.25h13') + P('M8 5.25V4c0-.69.56-1.25 1.25-1.25h1.5c.69 0 1.25.56 1.25 1.25v1.25') + P(TRASH_CAN) + P('M8.5 8.5v5.5M11.5 8.5v5.5'),
  delete: P('M3.5 5.25h13') + P('M8 5.25V4c0-.69.56-1.25 1.25-1.25h1.5c.69 0 1.25.56 1.25 1.25v1.25') + P(TRASH_CAN) + P('M8.5 8.5v5.5M11.5 8.5v5.5'),
  'trash-full': P('M3.5 5.25h13') + P('M6.25 5.25 7.4 2.6l2.6 1.3 2.3-1.5 1.45 2.85') + P(TRASH_CAN) + P('M8.5 8.5v5.5M11.5 8.5v5.5'),
  cut: C(5.75, 14.25, 2.5) + C(14.25, 14.25, 2.5) + P('M7.5 12.4 14.5 2.75M12.5 12.4 5.5 2.75'),
  copy: R(6.5, 5.5, 9.5, 12, 1.75) + P('M4 14V4.25C4 3.28 4.78 2.5 5.75 2.5H12'),
  paste: P('M7 3.75H5.5C4.67 3.75 4 4.42 4 5.25v10.5c0 .83.67 1.5 1.5 1.5H8.5') + R(7, 2.25, 6, 3, 1) + P('M13 3.75h1.5c.83 0 1.5.67 1.5 1.5V8') + R(10.25, 9.5, 7, 8, 1.5),
  clipboard: R(4.5, 3.5, 11, 14, 1.75) + R(7.25, 2.25, 5.5, 2.75, 1) + P('M7.5 9h5M7.5 12h5M7.5 15h3'),
  rename: P('M10.5 6.25H4.25c-.97 0-1.75.78-1.75 1.75v4c0 .97.78 1.75 1.75 1.75h6.25') + P('M15.5 6.25h.25c.97 0 1.75.78 1.75 1.75v4c0 .97-.78 1.75-1.75 1.75h-.25') + P('M13 3.5v13M11.25 3.5h3.5M11.25 16.5h3.5'),
  share: P('M12.75 3.5l4 4-4 4') + P('M16.5 7.5h-4a6 6 0 0 0-6 6v.25') + P('M8 4.25H5.25c-.97 0-1.75.78-1.75 1.75v8.75c0 .97.78 1.75 1.75 1.75H14c.97 0 1.75-.78 1.75-1.75V13'),
  properties: R(3, 3, 14, 14, 2) + P('M6.5 7h1.25M9.75 7h3.75M6.5 10h1.25M9.75 10h3.75M6.5 13h1.25M9.75 13h3.75'),
  info: CIRCLE + P('M10 9v4.75') + dot(10, 6.5, 1),
  help: CIRCLE + P('M7.75 7.75a2.25 2.25 0 1 1 3.4 1.95c-.65.4-1.15.95-1.15 1.7v.35') + dot(10, 14, 1),
  sort: P('M6.25 3.5v13M3.25 6.5l3-3 3 3M13.75 16.5v-13M10.75 13.5l3 3 3-3'),
  filter: P('M3.5 4.5h13l-5 6v5l-3 1.5v-6.5Z'),
  view: R(2.75, 3.75, 14.5, 12.5, 2) + P('M2.75 7.75h14.5M7.25 7.75v8.5'),
  grid: R(3, 3, 6, 6, 1.5) + R(11, 3, 6, 6, 1.5) + R(3, 11, 6, 6, 1.5) + R(11, 11, 6, 6, 1.5),
  list: dot(4.5, 5.5, 1) + dot(4.5, 10, 1) + dot(4.5, 14.5, 1) + P('M8 5.5h8.5M8 10h8.5M8 14.5h8.5'),
  new: CIRCLE + P('M10 6.75v6.5M6.75 10h6.5'),
  edit: P('M13.2 3.6a1.8 1.8 0 0 1 2.55 0l.65.65a1.8 1.8 0 0 1 0 2.55L7.6 15.6l-3.6.9.9-3.6Z') + P('M12 4.8l3.2 3.2'),
  save: P('M4.5 3h9l3 3v9.5a1.5 1.5 0 0 1-1.5 1.5H4.5A1.5 1.5 0 0 1 3 15.5v-11A1.5 1.5 0 0 1 4.5 3Z') + P('M6.5 3v3.5h6V3M6 17v-5h8v5'),
  tag: P('M3 4.5A1.5 1.5 0 0 1 4.5 3h5.38c.4 0 .78.16 1.06.44l5.62 5.62a1.5 1.5 0 0 1 0 2.12l-5.38 5.38a1.5 1.5 0 0 1-2.12 0L3.44 10.94A1.5 1.5 0 0 1 3 9.88Z') + dot(6.75, 6.75, 1.1),
  star: P('M10 2.8l2.2 4.5 4.95.7-3.58 3.5.85 4.95L10 14.1l-4.42 2.35.85-4.95L2.85 8l4.95-.7Z'),
  history: P('M3.75 10a6.25 6.25 0 1 0 1.83-4.42') + P('M3.75 3.25v3.5h3.5') + P('M10 6.75V10l2.5 1.5'),
  pin: PIN,
  unpin: PIN + P('M2.5 2.5l15 15'),
  terminal: R(2.5, 3.5, 15, 13, 2) + P('M6 8l2.5 2.25L6 12.5M10.5 12.5h3.5'),
  code: P('M7 5.5 2.75 10 7 14.5M13 5.5l4.25 4.5L13 14.5'),
  mail: R(2.5, 4.5, 15, 11, 2) + P('M3 5.5l7 5 7-5'),
  eye: P('M2.25 10S5.05 4.75 10 4.75 17.75 10 17.75 10 14.95 15.25 10 15.25 2.25 10 2.25 10Z') + C(10, 10, 2.5),
  'eye-off': P('M2.25 10S5.05 4.75 10 4.75 17.75 10 17.75 10 14.95 15.25 10 15.25 2.25 10 2.25 10Z') + C(10, 10, 2.5) + SLASH,
  location: P('M10 17.5s-5.5-4.9-5.5-9.25a5.5 5.5 0 0 1 11 0C15.5 12.6 10 17.5 10 17.5Z') + C(10, 8.25, 2),

  // settings & personalisation
  settings: P(gear()) + C(10, 10, 2.6),
  personalize: P('M10 2.5a7.5 7.5 0 0 0 0 15c1.1 0 1.8-.8 1.8-1.7 0-.9-.7-1.3-.7-2.1 0-.8.6-1.4 1.4-1.4h1.7a3.3 3.3 0 0 0 3.3-3.3C17.5 5.5 14.1 2.5 10 2.5Z') + dot(6.5, 9, 1) + dot(8.75, 5.9, 1) + dot(12.6, 6.2, 1),
  brush: P('M16.4 3.6a1.4 1.4 0 0 0-2 0L8.5 9.5l2 2 5.9-5.9a1.4 1.4 0 0 0 0-2Z') + P('M8 10.5l1.5 1.5c.5 2-1 4.5-5.5 4.5 1-1 .8-2.6 1.5-4 .6-1.2 1.6-2 2.5-2Z'),
  color: C(10, 7.25, 4) + C(7.25, 12, 4) + C(12.75, 12, 4),
  theme: C(10, 10, 7) + P('M10 3a7 7 0 0 1 0 14Z', FILL),
  clock: CIRCLE + CLOCK_HANDS,
  timer: C(10, 11, 6.5) + P('M10 7.75V11l2 1.25M8 2.5h4M15.25 5.25l1 1'),
  calendar: R(3, 3.75, 14, 13.5, 2) + P('M3 7.75h14M7 2.5v2.5M13 2.5v2.5') + dot(7, 11, 0.85) + dot(10, 11, 0.85) + dot(13, 11, 0.85) + dot(7, 14, 0.85) + dot(10, 14, 0.85),
  language: P('M2.75 16.5 6.25 7l3.5 9.5M3.9 13.25h4.7') + P('M11 4h6M14 2.75V4M12.25 7.75c1.4 1.9 3 3 4.75 3.5M16.1 4c-.6 3-2.3 5.3-4.7 6.7'),
  notification: P(BELL) + P('M8 15a2 2 0 0 0 4 0'),
  'notification-off': P(BELL) + P('M8 15a2 2 0 0 0 4 0') + SLASH,
  focus: CIRCLE + C(10, 10, 3.5),
  gamepad: P('M6.25 5.5h7.5a4.25 4.25 0 0 1 4.2 4.9l-.5 3.15a2.25 2.25 0 0 1-3.85 1.25L12 13H8l-1.6 1.8a2.25 2.25 0 0 1-3.85-1.25l-.5-3.15A4.25 4.25 0 0 1 6.25 5.5Z') + P('M6.5 8.25v2.5M5.25 9.5h2.5') + dot(13, 8.75, 0.85) + dot(14.5, 10.5, 0.85),
  bulb: P('M7 14.5v-1.6c0-.8-.45-1.5-1.05-2.05a5.25 5.25 0 1 1 8.1 0c-.6.55-1.05 1.25-1.05 2.05v1.6Z') + P('M7.75 17.25h4.5'),
  flag: P('M4.5 17.5V3.5M4.5 4h10.25l-2.25 3.75 2.25 3.75H4.5'),
  app: R(3, 3, 14, 14, 3) + R(6.5, 6.5, 7, 7, 1.5),

  // task manager
  process: R(2.75, 2.75, 14.5, 14.5, 2) + P('M2.75 7.25h14.5M6.25 10.5h7.5M6.25 13.5h5'),
  startup: P('M3.5 14.5a6.5 6.5 0 1 1 13 0') + P('M10 14.5l3-4.25') + dot(10, 14.5, 1.1),
  services: P('M8 3.5H5A1.5 1.5 0 0 0 3.5 5v3h1a1.75 1.75 0 0 1 0 3.5h-1v3A1.5 1.5 0 0 0 5 16h3v-1a1.75 1.75 0 0 1 3.5 0v1h3a1.5 1.5 0 0 0 1.5-1.5v-3h-1a1.75 1.75 0 0 1 0-3.5h1V5A1.5 1.5 0 0 0 14.5 3.5h-3v1a1.75 1.75 0 0 1-3.5 0Z'),
  summary: P('M8.75 4.15A6.25 6.25 0 1 0 15.85 11.25H8.75Z') + P('M11.25 2.75v6h6A6.25 6.25 0 0 0 11.25 2.75Z'),
  chart: P('M3.5 16.5h13') + R(4.75, 10, 2.5, 4, 0.75) + R(8.75, 5, 2.5, 9, 0.75) + R(12.75, 8, 2.5, 6, 0.75),
  clean: P(STAR4(9, 8, 5.25)) + P(STAR4(14.75, 13.75, 2.5)),
  sparkle: P(STAR4(9, 8, 5.25)) + P(STAR4(14.75, 13.75, 2.5)),
  broom: '<g transform="rotate(40 10 10)"><path d="M10 1.75v7"/><path d="M7 8.75h6l1.25 8.5h-8.5Z"/><path d="M8.75 12.5l-.25 4.75M11.25 12.5l.25 4.75"/></g>',

  // status
  warning: P('M8.7 3.3a1.5 1.5 0 0 1 2.6 0l6.3 11.1a1.5 1.5 0 0 1-1.3 2.2H3.7a1.5 1.5 0 0 1-1.3-2.2Z') + P('M10 7.5v4') + dot(10, 14, 0.95),
  error: CIRCLE + P('M7.5 7.5l5 5M12.5 7.5l-5 5'),
  success: CIRCLE + P('M6.8 10.2l2.2 2.2 4.2-4.6'),

  // media
  play: P('M6 3.8v12.4a.8.8 0 0 0 1.2.7l10-6.2a.8.8 0 0 0 0-1.4l-10-6.2A.8.8 0 0 0 6 3.8Z'),
  pause: R(5, 3.5, 3.5, 13, 1) + R(11.5, 3.5, 3.5, 13, 1),
  next: P('M4.5 4.6v10.8a.7.7 0 0 0 1.1.6l7.6-5.4a.7.7 0 0 0 0-1.2L5.6 4a.7.7 0 0 0-1.1.6Z') + P('M15.75 4v12'),
  previous: P('M15.5 4.6v10.8a.7.7 0 0 1-1.1.6l-7.6-5.4a.7.7 0 0 1 0-1.2L14.4 4a.7.7 0 0 1 1.1.6Z') + P('M4.25 4v12'),
};

/* ---------- API -------------------------------------------------------- */

const templates = new Map();
const warned = new Set();

function template(name) {
  let t = templates.get(name);
  if (t) return t;
  const body = ICONS[name];
  t = document.createElementNS(NS, 'svg');
  t.setAttribute('viewBox', '0 0 20 20');
  t.setAttribute('fill', 'none');
  t.setAttribute('stroke', 'currentColor');
  t.setAttribute('stroke-linecap', 'round');
  t.setAttribute('stroke-linejoin', 'round');
  t.setAttribute('aria-hidden', 'true');
  t.setAttribute('focusable', 'false');
  t.innerHTML = body ?? (R(4, 4, 12, 12, 2) + P('M8 8.25a2 2 0 1 1 2.75 1.85c-.45.2-.75.6-.75 1.1v.3') + dot(10, 13.75, 0.75));
  templates.set(name, t);
  return t;
}

/** icon(name, {size=16, cls, title}) → SVGElement */
export function icon(name, { size = 16, cls = '', title = '' } = {}) {
  if (!ICONS[name] && !warned.has(name)) { warned.add(name); console.warn(`[icons] unknown icon "${name}"`); }
  const el = template(name).cloneNode(true);
  el.setAttribute('width', String(size));
  el.setAttribute('height', String(size));
  el.setAttribute('class', `fl-icon${cls ? ' ' + cls : ''}`);
  el.dataset.icon = name;
  if (title) {
    el.removeAttribute('aria-hidden');
    el.setAttribute('role', 'img');
    el.setAttribute('aria-label', title);
  }
  return el;
}

export function hasIcon(name) { return Object.prototype.hasOwnProperty.call(ICONS, name); }
export function iconNames() { return Object.keys(ICONS); }

/** Wi-Fi glyph name for a signal strength 0–100. */
export function wifiIcon(strength) {
  if (strength == null) return 'wifi';
  if (strength >= 67) return 'wifi-3';
  if (strength >= 34) return 'wifi-2';
  return 'wifi-1';
}

/** Speaker glyph name for a volume 0–100 (+ muted). */
export function volumeIcon(volume, muted = false) {
  if (muted || volume <= 0) return 'volume-mute';
  if (volume < 34) return 'volume-1';
  if (volume < 67) return 'volume-2';
  return 'volume';
}

let logoCounter = 0;
/**
 * The Start logo: a filled circle with the existing blue gradient.
 * startLogo(size=20, {monochrome:false})
 */
export function startLogo(size = 20, { monochrome = false } = {}) {
  const id = `fl-start-grad-${++logoCounter}`;
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('class', 'fl-start-logo');
  svg.setAttribute('aria-hidden', 'true');
  const fill = monochrome ? 'currentColor' : `url(#${id})`;
  svg.innerHTML =
    (monochrome ? '' :
      `<defs><linearGradient id="${id}" x1="0" y1="0" x2="24" y2="24" gradientUnits="userSpaceOnUse">` +
      `<stop offset="0" stop-color="#5ad1ff"/><stop offset=".48" stop-color="#1a9df5"/><stop offset="1" stop-color="#0063d1"/></linearGradient>` +
      `<linearGradient id="${id}-h" x1="0" y1="0" x2="0" y2="24" gradientUnits="userSpaceOnUse">` +
      `<stop offset="0" stop-color="#fff" stop-opacity=".22"/><stop offset=".5" stop-color="#fff" stop-opacity="0"/></linearGradient></defs>`) +
    `<circle cx="12" cy="12" r="10.5" fill="${fill}"/>` +
    (monochrome ? '' : `<circle cx="12" cy="12" r="10.5" fill="url(#${id}-h)"/>`);
  return svg;
}
