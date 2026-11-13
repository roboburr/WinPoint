// Win11 Shell — Instellingen: colourful category icons for the navigation pane.
// Original drawings on a 24×24 grid (Windows 11 Settings uses small full-colour icons
// in its nav pane). catIcon(id, size) → <svg>. Gradient ids are unique per instance.

let n = 0;

const G = (id, a, b, x2 = 0, y2 = 1) =>
  `<linearGradient id="${id}" x1="0" y1="0" x2="${x2}" y2="${y2}"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient>`;

const W = 'stroke="#fff" fill="none" stroke-linecap="round" stroke-linejoin="round"';

const ICONS = {
  home: (u) => [
    G(`${u}a`, '#5ccbff', '#1470d8') + G(`${u}b`, '#ffb43a', '#f2581f', 1, 0),
    `<path d="M4.5 10.6 12 4.4l7.5 6.2v8.9a1.6 1.6 0 0 1-1.6 1.6h-3.4v-5.4a1 1 0 0 0-1-1h-3a1 1 0 0 0-1 1v5.4H6.1a1.6 1.6 0 0 1-1.6-1.6Z" fill="url(#${u}a)"/>` +
    `<path d="M2.8 11.2 12 3.4l9.2 7.8" stroke="url(#${u}b)" stroke-width="2.3" fill="none" stroke-linecap="round" stroke-linejoin="round"/>`,
  ],
  system: (u) => [
    G(`${u}a`, '#6dd3ff', '#1677e0') + G(`${u}b`, '#6f7b8a', '#3c4552'),
    `<rect x="2" y="3.6" width="20" height="13.6" rx="2.2" fill="url(#${u}b)"/>` +
    `<rect x="3.4" y="5" width="17.2" height="10.8" rx="1.1" fill="url(#${u}a)"/>` +
    `<path d="M3.4 13.2c4.6-3.6 9.4 1.6 17.2-2.6v3.9a1.3 1.3 0 0 1-1.3 1.3H4.7a1.3 1.3 0 0 1-1.3-1.3Z" fill="#fff" fill-opacity=".22"/>` +
    `<path d="M12 17.2v3M8.4 20.7h7.2" stroke="#8a96a5" stroke-width="1.9" stroke-linecap="round"/>`,
  ],
  bluetooth: (u) => [
    G(`${u}a`, '#4fb2ff', '#0b5bd3'),
    `<rect x="3" y="3" width="18" height="18" rx="5.2" fill="url(#${u}a)"/>` +
    `<path d="M8.3 8.9 15.4 14.6 12 17.4V6.6l3.4 2.8-7.1 5.7" ${W} stroke-width="1.7"/>`,
  ],
  network: (u) => [
    G(`${u}a`, '#4ce0f2', '#0a78d6', 1, 1),
    `<rect x="5.1" y="5.1" width="13.8" height="13.8" rx="3.2" transform="rotate(45 12 12)" fill="url(#${u}a)"/>` +
    `<path d="M8 11.3a5.7 5.7 0 0 1 8 0M9.8 13.2a3.1 3.1 0 0 1 4.4 0" ${W} stroke-width="1.55"/>` +
    `<circle cx="12" cy="15.4" r="1.05" fill="#fff"/>`,
  ],
  personalization: (u) => [
    G(`${u}a`, '#ffc23d', '#f7630c', 1, 1) + G(`${u}b`, '#ff5fb2', '#8a4fe0', 1, 1),
    `<path d="M20.2 3.8a1.9 1.9 0 0 1 0 2.7l-7.4 7.4-2.7-2.7 7.4-7.4a1.9 1.9 0 0 1 2.7 0Z" fill="url(#${u}a)"/>` +
    `<path d="M10.1 11.2l2.7 2.7-1.2 1.2-2.7-2.7Z" fill="#c3ccd6"/>` +
    `<path d="M8.4 13.1l2.5 2.5c.2 2.8-1.9 5.4-5.4 5.4-1 0-1.9-.2-2.5-.5 1.4-.9 1.1-2.4 1.4-3.6.4-2.1 2.1-3.8 4-3.8Z" fill="url(#${u}b)"/>`,
  ],
  apps: (u) => [
    G(`${u}a`, '#6fd0ff', '#2a8de6') + G(`${u}b`, '#2b8cf0', '#0a5bc4') + G(`${u}c`, '#b28cf2', '#7046c8'),
    `<rect x="3" y="3" width="8" height="8" rx="2.1" fill="url(#${u}a)"/>` +
    `<rect x="13" y="3" width="8" height="8" rx="2.1" fill="url(#${u}b)"/>` +
    `<rect x="3" y="13" width="8" height="8" rx="2.1" fill="url(#${u}b)"/>` +
    `<rect x="13" y="13" width="8" height="8" rx="2.1" fill="url(#${u}c)"/>`,
  ],
  accounts: (u) => [
    G(`${u}a`, '#3ef0a8', '#0e9e6b') + G(`${u}b`, '#2fd796', '#0a7a55'),
    `<circle cx="12" cy="7.9" r="4.4" fill="url(#${u}a)"/>` +
    `<path d="M3.8 20c0-3.7 3.7-6.4 8.2-6.4s8.2 2.7 8.2 6.4c0 .6-.4 1-1 1H4.8c-.6 0-1-.4-1-1Z" fill="url(#${u}b)"/>`,
  ],
  time: (u) => [
    G(`${u}a`, '#5ccfff', '#1262d2') + G(`${u}b`, '#ffc23d', '#f26b12'),
    `<circle cx="10.6" cy="11" r="8.4" fill="url(#${u}a)"/>` +
    `<path d="M10.6 6.6V11l3 1.9" ${W} stroke-width="1.75"/>` +
    `<path d="M15.2 14.4h5.3a1.5 1.5 0 0 1 1.5 1.5v3.2a1.5 1.5 0 0 1-1.5 1.5h-.9l-1.5 1.6-1.4-1.6h-1.5a1.5 1.5 0 0 1-1.5-1.5v-3.2a1.5 1.5 0 0 1 1.5-1.5Z" fill="url(#${u}b)"/>` +
    `<path d="M16.4 17.5h3" ${W} stroke-width="1.3"/>`,
  ],
  accessibility: (u) => [
    G(`${u}a`, '#5ccbff', '#0b5fc8'),
    `<circle cx="12" cy="12" r="9.6" fill="url(#${u}a)"/>` +
    `<circle cx="12" cy="7.1" r="1.65" fill="#fff"/>` +
    `<path d="M7.1 9.7 12 10.9l4.9-1.2M12 10.9v3.4l-2.3 4.3M12 14.3l2.3 4.3" ${W} stroke-width="1.6"/>`,
  ],
  privacy: (u) => [
    G(`${u}a`, '#9db4cf', '#4a6283'),
    `<path d="M12 2.4 19.6 5.2v6.2c0 4.8-3.2 8.5-7.6 10.2-4.4-1.7-7.6-5.4-7.6-10.2V5.2Z" fill="url(#${u}a)"/>` +
    `<path d="M12 2.4v19.2c4.4-1.7 7.6-5.4 7.6-10.2V5.2Z" fill="#fff" fill-opacity=".12"/>` +
    `<path d="M8.6 12.1l2.4 2.4 4.5-4.7" ${W} stroke-width="1.75"/>`,
  ],
  update: (u) => [
    G(`${u}a`, '#5ccfff', '#0e63cc'),
    `<circle cx="12" cy="12" r="9.6" fill="url(#${u}a)"/>` +
    `<path d="M16.5 9.6a4.9 4.9 0 0 0-9-.6M7.5 14.4a4.9 4.9 0 0 0 9 .6" ${W} stroke-width="1.6"/>` +
    `<path d="M16.8 6.9v2.9h-2.9M7.2 17.1v-2.9h2.9" ${W} stroke-width="1.6"/>`,
  ],
  search: (u) => [
    G(`${u}a`, '#9aa7b6', '#5c6878'),
    `<circle cx="10.5" cy="10.5" r="6" stroke="url(#${u}a)" stroke-width="2.4" fill="none"/>` +
    `<path d="M15 15l5 5" stroke="url(#${u}a)" stroke-width="2.6" stroke-linecap="round"/>`,
  ],
};

/** catIcon('system', 16) → coloured <svg> for a top-level Settings category. */
export function catIcon(id, size = 16) {
  const make = ICONS[id] || ICONS.system;
  const u = `stic${(++n).toString(36)}`;
  const [defs, body] = make(u);
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('width', String(size));
  svg.setAttribute('height', String(size));
  svg.setAttribute('aria-hidden', 'true');
  svg.setAttribute('class', 'st-caticon');
  svg.innerHTML = `<defs>${defs}</defs>${body}`;
  return svg;
}

export const CAT_ICON_IDS = Object.keys(ICONS);
