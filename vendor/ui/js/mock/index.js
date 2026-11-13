// Win11 Shell — in-browser mock backend (owned by ui-core).
//
// Every mock file exports:
//   export default { methods: { 'apps.list': async (args, m) => …, … }, start(m) { … } }
// m = { emit(event, data), state, settings(), call(method, args), params, sleep(ms) }
//
// - Methods of all files are merged; unknown method → reject {code:'unknown_method'}.
// - Calls get 30–120 ms of simulated latency and are (de)serialized like the real host.
// - Cross-frame events (settings.changed, shell.openApp, shell.command, app.navigate) are
//   relayed between the preview page and its embedded app iframes via BroadcastChannel.
// - ?mockreset=1 clears the persisted mock settings.
// - A thrown error keeps its code and message, and its key, params and data (contracts C2, D-G41), like the real bridge;
//   the mock's own two texts (unknown method, a method that failed without a message) follow the UI language.
// Also provides generated icons/thumbnails for bridge.iconUrl() etc. in mock mode. A mock file may
// add artwork for its own icon names/paths with `export default { …, icons: {'<name or path>': url} }`
// (e.g. mock/tray.js: tray icons as the host would name them).

import { lang } from '../core/i18n.js';

const FILES = ['settings', 'shell', 'apps', 'windows', 'system', 'notifications', 'media',
  'clipboard', 'files', 'display', 'sysmon', 'storage', 'index_search', 'tray', 'disks',
  // the Win11 voor GNOME host methods (software, per-type defaults, tab transfer, native backdrop): GE0a
  'gnome',
  // Windows Update (update.*, notes/claude/windows-update-gnome.md): GW3
  'update'];

const RELAY = new Set(['settings.changed', 'shell.openApp', 'shell.command', 'app.navigate']);
/** the mock's own error texts, in the language of the call (the host's core.unknownMethod is another text) */
const TEXT = {
  nl: { unknown: 'Onbekende functie: {method}', failed: 'Er ging iets mis.' },
  en: { unknown: 'Unknown function: {method}', failed: 'Something went wrong.' },
};
const text = (k, method = '') => (TEXT[lang()] || TEXT.nl)[k].replace('{method}', method);

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function clone(v) {
  if (v === undefined) return undefined;
  try { return structuredClone(v); } catch { return JSON.parse(JSON.stringify(v)); }
}

function mkErr(code, message, from = null) {
  const e = new Error(message);
  e.code = code;
  // C2: the catalogue key and its params travel with the error (core/bridge.js copies them onto its own Error)
  if (from?.key) e.key = String(from.key);
  if (from?.params && typeof from.params === 'object') e.params = clone(from.params);
  // C2's data (settings.set's {path}, …) too: bridge.mockCall hands it to makeError like the real bridge
  if (from?.data !== undefined) e.data = clone(from.data);
  return e;
}

export async function createMock({ dispatch }) {
  const params = new URLSearchParams(location.search);
  const loaded = await Promise.allSettled(FILES.map((f) => import(`./${f}.js`)));
  const modules = {};
  loaded.forEach((r, i) => {
    if (r.status === 'fulfilled') modules[FILES[i]] = r.value.default || {};
    else console.error(`[mock] ${FILES[i]}.js failed to load`, r.reason);
  });

  const settingsMod = modules.settings || {};
  if (params.get('mockreset') === '1') { try { settingsMod.resetStorage?.(); } catch { /* ignore */ } }

  const methods = Object.create(null);
  for (const [name, mod] of Object.entries(modules)) {
    for (const [method, fn] of Object.entries(mod.methods || {})) {
      if (methods[method]) console.warn(`[mock] method ${method} defined twice (also in ${name}.js)`);
      if (typeof fn === 'function') methods[method] = fn;
    }
    // the file's own artwork for bridge.iconUrl(name) (see the header)
    for (const [icon, url] of Object.entries(mod.icons || {})) {
      if (typeof url === 'string' && url) iconCache.set(String(icon).toLowerCase(), url);
    }
  }

  let channel = null;
  const senderId = Math.random().toString(36).slice(2);
  try { channel = new BroadcastChannel('win11shell-mock'); } catch { channel = null; }

  const m = {
    state: {},
    params,
    sleep,
    emit(event, data) {
      const payload = clone(data);
      // deliver asynchronously, like a real event stream
      setTimeout(() => dispatch(event, payload), 0);
      if (channel && RELAY.has(event)) {
        try { channel.postMessage({ from: senderId, event, data: payload }); } catch { /* ignore */ }
      }
    },
    settings() { return settingsMod.current ? settingsMod.current() : {}; },
    call(method, args) { return invoke(method, args, false); },
  };

  if (channel) {
    channel.onmessage = (ev) => {
      const msg = ev.data || {};
      if (msg.from === senderId || !msg.event) return;
      if (msg.event === 'settings.changed' && msg.data?.settings && settingsMod.replace) {
        settingsMod.replace(msg.data.settings);
      }
      dispatch(msg.event, msg.data);
    };
  }

  async function invoke(method, args, latency) {
    if (latency) await sleep(30 + Math.random() * 90);
    const fn = methods[method];
    if (!fn) {
      // a method no mock file serves: name it in the console (tests/i18n/run.sh --notes lists it per surface)
      console.warn(`[mock] unknown method ${method}`);
      throw mkErr('unknown_method', text('unknown', method));
    }
    try {
      const r = await fn(clone(args ?? {}), m);
      return r === undefined ? {} : clone(r);
    } catch (e) {
      throw mkErr(e?.code || 'internal', e?.message || text('failed'), e);
    }
  }

  // start modules once bridge listeners had a chance to register
  setTimeout(() => {
    for (const [name, mod] of Object.entries(modules)) {
      try { mod.start?.(m); } catch (e) { console.error(`[mock] ${name}.start failed`, e); }
    }
  }, 0);

  return { call: (method, args) => invoke(method, args, true), m, methods };
}

/* ==========================================================================
   Generated artwork for mock mode (original drawings)
   ========================================================================== */

const iconCache = new Map();

function svgUrl(inner, defs = '') {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48">${defs ? `<defs>${defs}</defs>` : ''}${inner}</svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

function hash(s) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
  return h >>> 0;
}

const lg = (id, a, b, vertical = true) =>
  `<linearGradient id="${id}" x1="0" y1="0" x2="${vertical ? 0 : 1}" y2="${vertical ? 1 : 1}"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient>`;

function gearPath(cx, cy, R, r, teeth = 8) {
  let d = '';
  const step = (Math.PI * 2) / teeth;
  for (let i = 0; i < teeth; i++) {
    const a = i * step;
    const pts = [[R, a - step * 0.2], [R, a + step * 0.2], [r, a + step * 0.32], [r, a + step * 0.68]];
    pts.forEach(([rad, ang], j) => {
      const x = (cx + rad * Math.cos(ang)).toFixed(2), y = (cy + rad * Math.sin(ang)).toFixed(2);
      d += (i === 0 && j === 0 ? 'M' : 'L') + x + ' ' + y + ' ';
    });
  }
  return d + 'Z';
}

const FOLDER_DEFS = lg('fb', '#f2b640', '#dd9516') + lg('ff', '#ffdc72', '#ffc22e');
function folder(emblem = '') {
  return svgUrl(
    `<path d="M4 11.5A3.5 3.5 0 0 1 7.5 8h10.3a3.5 3.5 0 0 1 2.5 1l3 3h17.2A3.5 3.5 0 0 1 44 15.5v21A3.5 3.5 0 0 1 40.5 40h-33A3.5 3.5 0 0 1 4 36.5Z" fill="url(#fb)"/>` +
    `<path d="M4 20A3.5 3.5 0 0 1 7.5 16.5h33A3.5 3.5 0 0 1 44 20v16.5A3.5 3.5 0 0 1 40.5 40h-33A3.5 3.5 0 0 1 4 36.5Z" fill="url(#ff)"/>` +
    `<path d="M4 20A3.5 3.5 0 0 1 7.5 16.5h33A3.5 3.5 0 0 1 44 20" fill="none" stroke="#fff" stroke-opacity=".45"/>` + emblem,
    FOLDER_DEFS);
}
const E = (d, color = '#c98a10', w = 2.2) => `<path d="${d}" fill="none" stroke="${color}" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"/>`;
const FOLDER_EMBLEMS = {
  documents: E('M19 24h10M19 28h10M19 32h6'),
  download: E('M24 22v11M19.5 28.5 24 33l4.5-4.5'),
  pictures: E('M17 34l5-5 3 3 2-2 4 4M28.5 25.5h.01', '#c98a10'),
  music: E('M22 33v-9l7-2v8') + '<circle cx="20.5" cy="33" r="2" fill="#c98a10"/><circle cx="27.5" cy="30.5" r="2" fill="#c98a10"/>',
  videos: '<path d="M21 23.5v10l8.5-5Z" fill="#c98a10"/>',
  desktop: '<rect x="17" y="23" width="14" height="9" rx="1.5" fill="#2b88e6"/><path d="M21 35h6" stroke="#c98a10" stroke-width="2" stroke-linecap="round"/>',
  home: E('M18 28.5 24 23l6 5.5V35h-12Z'),
  public: E('M21 26.5a2.5 2.5 0 1 0 0-.01M27 26.5a2.5 2.5 0 1 0 0-.01M17 34c.5-2.5 2-3.5 4-3.5s3.5 1 4 3.5M23 34c.5-2.5 2-3.5 4-3.5s3.5 1 4 3.5'),
  templates: E('M18 23h12v12H18ZM18 27h12'),
};

function page(emblem = '', fold = '#e8ecf1') {
  return svgUrl(
    `<path d="M12.5 4H28l10 10v27a3 3 0 0 1-3 3H12.5a3 3 0 0 1-3-3V7a3 3 0 0 1 3-3Z" fill="#fff" stroke="#c4ccd6" stroke-width="1.2"/>` +
    `<path d="M28 4v7.5a2.5 2.5 0 0 0 2.5 2.5H38" fill="${fold}" stroke="#c4ccd6" stroke-width="1.2" stroke-linejoin="round"/>` + emblem);
}
const PAGES = {
  text: page(E('M15 21h18M15 26h18M15 31h18M15 36h11', '#9aa6b4', 2)),
  pdf: page('<rect x="13" y="28" width="22" height="10" rx="2" fill="#d83b2b"/>' + E('M15 21h18', '#9aa6b4', 2) + '<text x="24" y="35.6" font-family="Segoe UI, Ubuntu Sans, sans-serif" font-size="7.5" font-weight="700" fill="#fff" text-anchor="middle">PDF</text>'),
  image: page('<rect x="14" y="20" width="20" height="16" rx="2" fill="#4aa3f0"/><path d="M14 33l6-6 4 4 3-3 7 7v.5a1.5 1.5 0 0 1-1.5 1.5h-17A1.5 1.5 0 0 1 14 35.5Z" fill="#2e8b57"/><circle cx="29" cy="24.5" r="2" fill="#ffe27a"/>'),
  audio: page(E('M22 34v-12l9-2v11', '#ef7d22', 2.4) + '<circle cx="20" cy="34" r="2.6" fill="#ef7d22"/><circle cx="29" cy="31" r="2.6" fill="#ef7d22"/>'),
  video: page('<rect x="13" y="21" width="22" height="15" rx="2.5" fill="#8a5cf6"/><path d="M22 25.5v6.5l5.5-3.25Z" fill="#fff"/>'),
  archive: page('<path d="M21 6h4v4h-4zM25 10h4v4h-4zM21 14h4v4h-4zM25 18h4v4h-4z" fill="#d9a521"/><rect x="20.5" y="23" width="9" height="9" rx="2" fill="#d9a521"/><rect x="23" y="26" width="4" height="3" rx="1" fill="#fff"/>'),
  code: page(E('M20 23l-5 5 5 5M28 23l5 5-5 5M25.5 21l-3 14', '#2b7de9', 2.2)),
  html: page('<circle cx="24" cy="29" r="8" fill="none" stroke="#2b7de9" stroke-width="2"/>' + E('M16 29h16M24 21c-3 4.5-3 11.5 0 16M24 21c3 4.5 3 11.5 0 16', '#2b7de9', 1.6)),
  sheet: page('<rect x="14" y="20" width="20" height="16" rx="2" fill="#1f9d55"/>' + E('M14 25.5h20M14 30.5h20M21 20v16M27.5 20v16', '#fff', 1.2)),
  slides: page('<rect x="13" y="21" width="22" height="14" rx="2" fill="#e8662a"/><circle cx="21" cy="28" r="4" fill="#fff"/><path d="M21 24v4h4" fill="#ffd2bd"/>'),
  exec: page(`<path d="${gearPath(24, 29, 8, 6.2)}" fill="#7a8796"/><circle cx="24" cy="29" r="2.8" fill="#fff"/>`),
};

function tile(a, b, glyph, defs = '') {
  return svgUrl(`<rect x="4" y="4" width="40" height="40" rx="9" fill="url(#t)"/>` +
    `<rect x="4.5" y="4.5" width="39" height="39" rx="8.5" fill="none" stroke="#fff" stroke-opacity=".18"/>` + glyph, lg('t', a, b) + defs);
}
const W = (d, w = 3) => `<path d="${d}" fill="none" stroke="#fff" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"/>`;

const APPS = {
  settings: tile('#7d8fa6', '#4f5f74', `<path d="${gearPath(24, 24, 12.5, 9.6)}" fill="#fff" stroke="#fff" stroke-width="2" stroke-linejoin="round"/><circle cx="24" cy="24" r="4.6" fill="#5e6e83"/>`),
  taskmgr: tile('#2fbf7a', '#138a52', '<rect x="11" y="12" width="26" height="24" rx="3" fill="#fff" fill-opacity=".18"/>' + W('M12 30l6-7 5 4 6-10 7 8', 2.6)),
  explorer: folder(),
  alles: tile('#ff9a3d', '#e2560a', '<circle cx="21.5" cy="21.5" r="7.5" fill="none" stroke="#fff" stroke-width="3.2"/>' + W('M27 27l7.5 7.5', 3.6)),
  // Schijven: a drive with its partition bar (blue / teal / unallocated), like Schijfbeheer
  disks: tile('#4b9fea', '#1d5fbf', '<rect x="10" y="13" width="28" height="22" rx="3.5" fill="#fff" fill-opacity=".95"/>'
    + '<rect x="13" y="16" width="7" height="6" rx="1" fill="#1d6fd6"/><rect x="21" y="16" width="9" height="6" rx="1" fill="#17a2b8"/>'
    + '<rect x="31" y="16" width="4" height="6" rx="1" fill="#9aa7b6"/><path d="M13 29h13" stroke="#9aa7b6" stroke-width="2" stroke-linecap="round"/>'
    + '<circle cx="33" cy="29" r="1.8" fill="#2ecc71"/>'),
  terminal: tile('#3a3a3a', '#141414', W('M13 18l6 6-6 6', 3) + W('M23 31h11', 3)),
  browser: svgUrl('<circle cx="24" cy="24" r="19" fill="url(#o)"/><path d="M10 27c3 9 15 13 23 7 5-4 6-11 3-16-1 6-6 9-11 9-6 0-10-4-10-9-3 2-5 5-5 9Z" fill="url(#p)"/><circle cx="24" cy="24" r="19" fill="none" stroke="#000" stroke-opacity=".08"/>',
    '<radialGradient id="o" cx=".3" cy=".25" r=".9"><stop offset="0" stop-color="#ffe066"/><stop offset=".45" stop-color="#ff8a1f"/><stop offset="1" stop-color="#d6246e"/></radialGradient>' + lg('p', '#7b3fe4', '#3a1d9e')),
  // A generic globe for other browsers (no product logo)
  web: tile('#4aa8f5', '#1c64d0', '<circle cx="24" cy="24" r="12" fill="none" stroke="#fff" stroke-width="2.4"/><ellipse cx="24" cy="24" rx="5" ry="12" fill="none" stroke="#fff" stroke-width="2.2"/>' + W('M12.5 20h23M12.5 28h23', 2.2)),
  editor: tile('#5aa8ff', '#2a6fd6', '<path d="M15 11h13l6 6v20a2 2 0 0 1-2 2H15a2 2 0 0 1-2-2V13a2 2 0 0 1 2-2Z" fill="#fff"/>' + E('M17 22h13M17 27h13M17 32h9', '#2a6fd6', 2)),
  calculator: tile('#4a4a4a', '#222', '<rect x="13" y="11" width="22" height="7" rx="1.5" fill="#cfd8e3"/>' + [0, 1, 2].map((r) => [0, 1, 2].map((c) => `<rect x="${13 + c * 8}" y="${21 + r * 7}" width="6" height="5" rx="1.2" fill="${c === 2 ? '#ff9f2e' : '#8b8b8b'}"/>`).join('')).join('')),
  mail: tile('#3aa0ff', '#1662d4', '<rect x="11" y="15" width="26" height="18" rx="2.5" fill="#fff"/>' + E('M12 17l12 8.5L36 17', '#1662d4', 2)),
  music: tile('#ff5b7f', '#d6245a', W('M21 31V17l11-3v14', 2.8) + '<circle cx="18.5" cy="31" r="3.2" fill="#fff"/><circle cx="29.5" cy="28" r="3.2" fill="#fff"/>'),
  video: tile('#9b6bff', '#5d2fd6', '<path d="M19 15.5v17l14-8.5Z" fill="#fff"/>'),
  photos: tile('#4cc2ff', '#1677d8', '<path d="M10 34l9-10 6 6 4-4 9 8v2H10Z" fill="#fff"/><circle cx="31" cy="17" r="3.5" fill="#fff"/>'),
  code: tile('#3c9df5', '#1259b8', W('M19 17l-7 7 7 7M29 17l7 7-7 7', 3)),
  game: tile('#2b3a67', '#141c3a', '<path d="M15 18h18a6 6 0 0 1 6 6.5l-.6 5.5a3.5 3.5 0 0 1-6 2L30 29H18l-2.4 3a3.5 3.5 0 0 1-6-2L9 24.5A6 6 0 0 1 15 18Z" fill="#fff"/>' + E('M16 22v5M13.5 24.5h5', '#2b3a67', 2) + '<circle cx="30" cy="23" r="1.4" fill="#2b3a67"/><circle cx="33" cy="26" r="1.4" fill="#2b3a67"/>'),
  // A generic shopping bag for software centres (no product logo)
  store: tile('#2cc0b0', '#0f7d73', '<path d="M14 18h20l-1.5 17a2 2 0 0 1-2 1.8H17.5a2 2 0 0 1-2-1.8Z" fill="#fff"/>' + W('M19 18v-2a5 5 0 0 1 10 0v2', 2.4) + E('M20 25a4 4 0 0 0 8 0', '#0f7d73', 2.2)),
  monitor: svgUrl('<rect x="4" y="7" width="40" height="27" rx="3" fill="#3b4048"/><rect x="6.5" y="9.5" width="35" height="22" rx="1.5" fill="url(#s)"/><path d="M6.5 25c8-6 16 3 35-4v9a1.5 1.5 0 0 1-1.5 1.5H8A1.5 1.5 0 0 1 6.5 30Z" fill="#fff" fill-opacity=".18"/><rect x="21.5" y="34" width="5" height="6" fill="#8a919b"/><path d="M16 41h16" stroke="#8a919b" stroke-width="3" stroke-linecap="round"/>', lg('s', '#3ba2ff', '#1257cc')),
  drive: svgUrl('<rect x="5" y="15" width="38" height="19" rx="4" fill="url(#d)"/><rect x="5" y="27" width="38" height="7" rx="3" fill="#8e98a3"/><circle cx="37" cy="30.5" r="1.7" fill="#4ce07a"/><path d="M10 30.5h14" stroke="#c9d0d8" stroke-width="1.6" stroke-linecap="round"/>', lg('d', '#e3e7ec', '#b5bec8')),
  usb: svgUrl('<rect x="9" y="17" width="26" height="15" rx="3" fill="url(#d)"/><rect x="35" y="20" width="7" height="9" rx="1" fill="#c3cad2"/><rect x="37" y="22.5" width="2.5" height="1.6" fill="#7b8590"/><rect x="37" y="25" width="2.5" height="1.6" fill="#7b8590"/><circle cx="14" cy="24.5" r="1.6" fill="#4ce07a"/>', lg('d', '#5b6573', '#3a424d')),
  trash: svgUrl('<path d="M11.5 15h25l-2.3 24.3A3 3 0 0 1 31.2 42H16.8a3 3 0 0 1-3-2.7Z" fill="url(#g)" stroke="#9aa9b8" stroke-width="1"/><rect x="9" y="10.5" width="30" height="5" rx="2.2" fill="#a7b6c5"/><path d="M19 21v15M24 21v15M29 21v15" stroke="#8fa0b1" stroke-width="2" stroke-linecap="round"/>', lg('g', '#eef3f8', '#c3cfdb', false)),
  trashFull: svgUrl('<path d="M15 14l3-8 6 3.5 5-5 4.5 9.5Z" fill="#fff" stroke="#c4ced8"/><path d="M20 13.5l4-6 4 6Z" fill="#dbe8f5"/><path d="M11.5 15h25l-2.3 24.3A3 3 0 0 1 31.2 42H16.8a3 3 0 0 1-3-2.7Z" fill="url(#g)" stroke="#9aa9b8" stroke-width="1"/><rect x="9" y="12.5" width="30" height="4" rx="2" fill="#a7b6c5"/><path d="M19 21v15M24 21v15M29 21v15" stroke="#8fa0b1" stroke-width="2" stroke-linecap="round"/>', lg('g', '#eef3f8', '#c3cfdb', false)),
};

const ICON_RULES = [
  [/win11shell-settings|gnome-control-center|preferences-system|systemsettings|^settings$/, () => APPS.settings],
  [/win11shell-taskmgr|system-monitor|utilities-system-monitor|taskmgr/, () => APPS.taskmgr],
  [/win11shell-explorer|nautilus|file-manager|system-file-manager|^files?$|explorer/, () => APPS.explorer],
  [/win11shell-alles|alles|everything|search/, () => APPS.alles],
  [/win11shell-disks|diskutility|gnome-disks|^disks$/, () => APPS.disks],
  [/user-trash-full|trash-full/, () => APPS.trashFull],
  [/user-trash|trash/, () => APPS.trash],
  [/folder-documents|documents/, () => folder(FOLDER_EMBLEMS.documents)],
  [/folder-download|downloads?/, () => folder(FOLDER_EMBLEMS.download)],
  [/folder-pictures|pictures|afbeeldingen/, () => folder(FOLDER_EMBLEMS.pictures)],
  [/folder-music/, () => folder(FOLDER_EMBLEMS.music)],
  [/folder-videos/, () => folder(FOLDER_EMBLEMS.videos)],
  [/user-desktop|folder-desktop/, () => folder(FOLDER_EMBLEMS.desktop)],
  [/user-home|folder-home|go-home/, () => folder(FOLDER_EMBLEMS.home)],
  [/folder-publicshare/, () => folder(FOLDER_EMBLEMS.public)],
  [/folder-templates/, () => folder(FOLDER_EMBLEMS.templates)],
  [/folder|directory/, () => folder()],
  [/computer|user-desktop-symbolic|video-display|this-?pc/, () => APPS.monitor],
  [/drive-removable|media-removable|usb|media-flash/, () => APPS.usb],
  [/drive-|harddisk|disk/, () => APPS.drive],
  [/firefox|web-browser|browser|epiphany/, () => APPS.browser],
  [/chrom|brave|edge/, () => APPS.web],
  [/terminal|console|ptyxis|kgx|konsole/, () => APPS.terminal],
  [/gedit|text-editor|texteditor|kate|mousepad|notepad|accessories-text/, () => APPS.editor],
  [/calculator|calc/, () => APPS.calculator],
  [/thunderbird|evolution|mail|geary/, () => APPS.mail],
  [/rhythmbox|spotify|music|lollypop|audio-player/, () => APPS.music],
  [/totem|vlc|mpv|video|celluloid|showtime/, () => APPS.video],
  [/eog|loupe|shotwell|image-viewer|photos|gthumb|gimp/, () => APPS.photos],
  [/code|vscode|codium|gnome-builder|jetbrains/, () => APPS.code],
  [/steam|game|lutris|heroic/, () => APPS.game],
  [/software|store|snap-store/, () => APPS.store],
  [/pdf/, () => PAGES.pdf],
  [/image-|x-generic-image/, () => PAGES.image],
  [/audio-/, () => PAGES.audio],
  [/video-/, () => PAGES.video],
  [/zip|tar|compress|archive|7z|rar/, () => PAGES.archive],
  [/html|web/, () => PAGES.html],
  [/python|javascript|script|x-c|source|json|xml|code/, () => PAGES.code],
  [/spreadsheet|calc|excel|csv/, () => PAGES.sheet],
  [/presentation|impress|powerpoint/, () => PAGES.slides],
  [/executable|sharedlib|x-sh|application-x/, () => PAGES.exec],
  [/text|document|generic|plain|office/, () => PAGES.text],
];

function letterTile(name) {
  const base = String(name).split('/').pop().replace(/\.(desktop|png|svg|xpm)$/i, '').replace(/^(org|com|io|net)\.[^.]+\./i, '');
  const letter = (base.match(/[a-z0-9]/i)?.[0] || '?').toUpperCase();
  const hue = hash(base) % 360;
  return tile(`hsl(${hue} 70% 58%)`, `hsl(${(hue + 25) % 360} 72% 40%)`,
    `<text x="24" y="31.5" font-family="Segoe UI Variable Display, Segoe UI, Ubuntu Sans, sans-serif" font-size="21" font-weight="600" fill="#fff" text-anchor="middle">${letter}</text>`);
}

export function mockIconUrl(name /*, size */) {
  const key = String(name).toLowerCase();
  if (iconCache.has(key)) return iconCache.get(key);
  let url = null;
  const leaf = key.split('/').pop();
  for (const [re, gen] of ICON_RULES) { if (re.test(leaf)) { url = gen(); break; } }
  if (!url) url = letterTile(leaf);
  iconCache.set(key, url);
  return url;
}

export function mockMimeIconUrl(mime /*, size */) {
  const m = String(mime).toLowerCase();
  if (m === 'inode/directory') return mockIconUrl('folder');
  if (m.startsWith('image/')) return PAGES.image;
  if (m.startsWith('audio/')) return PAGES.audio;
  if (m.startsWith('video/')) return PAGES.video;
  if (m.includes('pdf')) return PAGES.pdf;
  if (/zip|tar|compress|archive|7z|rar/.test(m)) return PAGES.archive;
  if (/html/.test(m)) return PAGES.html;
  if (/spreadsheet|csv/.test(m)) return PAGES.sheet;
  if (/presentation/.test(m)) return PAGES.slides;
  if (/python|javascript|json|xml|x-c|x-sh|script/.test(m)) return PAGES.code;
  if (/executable|sharedlib|x-desktop/.test(m)) return PAGES.exec;
  return PAGES.text;
}

const thumbCache = new Map();
/** A pleasant generated "photo" for image paths (mock thumbnails / wallpapers / avatars). */
export function mockThumbUrl(path /*, size */) {
  const p = String(path);
  if (!/\.(png|jpe?g|gif|webp|bmp|svg|avif|heic|tiff?)$/i.test(p) && !/avatar|face|wallpaper|picture/i.test(p)) return '';
  if (thumbCache.has(p)) return thumbCache.get(p);
  const h = hash(p), hue = h % 360, hue2 = (hue + 40 + (h >> 8) % 60) % 360;
  const sky = `<linearGradient id="k" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${hue} 75% 70%)"/><stop offset="1" stop-color="hsl(${hue2} 80% 86%)"/></linearGradient>`;
  const y1 = 24 + (h >> 4) % 8, y2 = 30 + (h >> 7) % 6;
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 48" preserveAspectRatio="xMidYMid slice"><defs>${sky}</defs>` +
    `<rect width="64" height="48" fill="url(#k)"/><circle cx="${44 + (h >> 3) % 10}" cy="${12 + (h >> 5) % 6}" r="5" fill="#fff" fill-opacity=".8"/>` +
    `<path d="M0 ${y1} C12 ${y1 - 8} 22 ${y1 - 2} 34 ${y1 + 2} S56 ${y1 - 6} 64 ${y1 - 2} V48 H0Z" fill="hsl(${(hue + 160) % 360} 35% 45%)"/>` +
    `<path d="M0 ${y2} C16 ${y2 - 5} 30 ${y2 + 4} 46 ${y2} S60 ${y2 - 3} 64 ${y2} V48 H0Z" fill="hsl(${(hue + 150) % 360} 40% 30%)"/></svg>`;
  const url = 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
  thumbCache.set(p, url);
  return url;
}
