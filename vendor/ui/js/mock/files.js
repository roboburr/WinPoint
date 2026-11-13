// Win11 Shell — mock files.* (owned by the desktop agent). SPEC §6.5.
//
// An in-memory fake home folder (/home/robin) with a Dutch desktop (Bureaublad), the usual
// XDG folders, a non-empty trash and a file clipboard. Every files.* method of the SPEC is
// implemented on this fake — nothing ever touches the real disk.
//
// The fake is shared between the preview page and its app iframes (Explorer in the preview
// window manager): it is kept in sessionStorage and changes are announced on a
// BroadcastChannel, so a file trashed in Explorer disappears from the desktop too.
// ?mockreset=1 starts from the pristine fake again.
//
// Shapes mirror host/services/files.py (notes/host-services-a.md):
//   FileItem extra `typeName` (Windows-style, nl/en); Prullenbak items (files.list {path:'trash:///'})
//   have path = the file inside ~/.local/share/Trash/files, name = its trash name, displayName = the
//   original name, + originalPath, originalDir, deletedAt (ms), trashName, inTrash:true
//   files.list → {path, parent, items, truncated, writable} ; trash → {path:'trash:///', parent:null, items, writable:false, trash:true}
//   files.places: trash path 'trash:///', `exists`; files.stat; files.setAttributes {path, readOnly?, hidden?}
//   files.trash → {ok, failed:[{path, error, code}], trashed:[{path, name}]} ; files.restoreTrash {names|paths|all}
//   files.createFolder/createFile/rename → {path, item} ; files.clipboardSet → {paths, op} ; files.clipboardClear
//   files.properties {path} (host fields) or {paths} (multi) ; jobId → files.progress events
//   events: files.dirChanged, files.desktopChanged, files.trashChanged, files.clipboardChanged {paths, op}

const HOME = '/home/robin';
const DESKTOP = `${HOME}/Bureaublad`;
const TRASH = 'trash:///';
const TRASH_FILES = `${HOME}/.local/share/Trash/files`;
const KEY = 'win11shell.mock.files.v1';
const DAY = 86400000;

const now = Date.now();
const ago = (days, hours = 0) => now - days * DAY - hours * 3600000;

/* ------------------------------------------------------------------ types */

const MIME = {
  txt: 'text/plain', md: 'text/markdown', log: 'text/plain', csv: 'text/csv',
  pdf: 'application/pdf',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif', webp: 'image/webp', svg: 'image/svg+xml',
  mp3: 'audio/mpeg', flac: 'audio/flac', ogg: 'audio/ogg', wav: 'audio/x-wav',
  mp4: 'video/mp4', webm: 'video/webm', mkv: 'video/x-matroska',
  zip: 'application/zip', gz: 'application/gzip', tar: 'application/x-tar', '7z': 'application/x-7z-compressed',
  iso: 'application/x-cd-image', deb: 'application/vnd.debian.binary-package',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet',
  pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  odt: 'application/vnd.oasis.opendocument.text', ods: 'application/vnd.oasis.opendocument.spreadsheet',
  desktop: 'application/x-desktop', sh: 'application/x-shellscript', py: 'text/x-python',
  json: 'application/json', html: 'text/html',
};

const ICON = {
  'text/plain': 'text-x-generic', 'text/markdown': 'text-x-generic', 'text/csv': 'x-office-spreadsheet',
  'application/pdf': 'application-pdf',
  'image/png': 'image-png', 'image/jpeg': 'image-jpeg', 'image/gif': 'image-gif', 'image/webp': 'image-x-generic', 'image/svg+xml': 'image-svg+xml',
  'audio/mpeg': 'audio-mpeg', 'audio/flac': 'audio-x-flac', 'audio/ogg': 'audio-x-generic', 'audio/x-wav': 'audio-x-wav',
  'video/mp4': 'video-mp4', 'video/webm': 'video-webm', 'video/x-matroska': 'video-x-matroska',
  'application/zip': 'application-zip', 'application/gzip': 'application-x-compressed-tar', 'application/x-tar': 'application-x-tar',
  'application/x-7z-compressed': 'application-x-7z-compressed', 'application/x-cd-image': 'application-x-cd-image',
  'application/vnd.debian.binary-package': 'application-x-deb',
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document': 'x-office-document',
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet': 'x-office-spreadsheet',
  'application/vnd.openxmlformats-officedocument.presentationml.presentation': 'x-office-presentation',
  'application/vnd.oasis.opendocument.text': 'x-office-document',
  'application/vnd.oasis.opendocument.spreadsheet': 'x-office-spreadsheet',
  'application/x-shellscript': 'text-x-script', 'text/x-python': 'text-x-python', 'application/json': 'text-x-generic',
  'text/html': 'text-html', 'application/x-desktop': 'application-x-executable',
};

/* Windows-style type names, exactly like host/services/files.py _TYPES / type_name() */
const TYPES = {
  'inode/directory': ['Bestandsmap', 'File folder'],
  'text/plain': ['Tekstdocument', 'Text Document'],
  'application/zip': ['Gecomprimeerde (gezipte) map', 'Compressed (zipped) Folder'],
  'application/pdf': ['PDF-document', 'PDF Document'],
  'application/x-desktop': ['Snelkoppeling', 'Shortcut'],
  'application/vnd.openxmlformats-officedocument.wordprocessingml.document': ['Microsoft Word-document', 'Microsoft Word Document'],
  'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet': ['Microsoft Excel-werkblad', 'Microsoft Excel Worksheet'],
  'application/vnd.openxmlformats-officedocument.presentationml.presentation': ['Microsoft PowerPoint-presentatie', 'Microsoft PowerPoint Presentation'],
  'application/vnd.oasis.opendocument.text': ['OpenDocument-tekst', 'OpenDocument Text'],
  'application/vnd.oasis.opendocument.spreadsheet': ['OpenDocument-werkblad', 'OpenDocument Spreadsheet'],
  'text/html': ['HTML-document', 'HTML Document'],
  'application/x-shellscript': ['Shell-script', 'Shell Script'],
  'application/x-cd-image': ['Schijfkopiebestand', 'Disc Image File'],
  'application/json': ['JSON-bestand', 'JSON File'],
  'text/markdown': ['Markdown-bestand', 'Markdown File'],
  'text/csv': ['CSV-bestand', 'CSV File'],
  'text/x-python': ['Python-bestand', 'Python File'],
  'application/vnd.debian.binary-package': ['Softwarepakket', 'Software Package'],
  'application/x-tar': ['Archief', 'Archive'],
  'application/x-7z-compressed': ['7Z-archief', '7Z Archive'],
  'application/gzip': ['GZ-archief', 'GZ Archive'],
};
let LANG = 'nl';   // follows settings.language (set at every call, see wrap() below)
function typeName(name, isDir) {
  const mime = mimeOf(name, isDir);
  const li = LANG === 'en' ? 1 : 0;
  if (TYPES[mime]) return TYPES[mime][li];
  const e = extOf(name);
  if (e && /^(image|audio|video)\//.test(mime) && e.length <= 6) return li ? `${e.toUpperCase()} File` : `${e.toUpperCase()}-bestand`;
  if (e && e.length <= 8) return li ? `${e.toUpperCase()} File` : `${e.toUpperCase()}-bestand`;
  return li ? 'File' : 'Bestand';
}

const SPECIAL_DIR_ICON = {
  [HOME]: 'user-home',
  [DESKTOP]: 'user-desktop',
  [`${HOME}/Documenten`]: 'folder-documents',
  [`${HOME}/Downloads`]: 'folder-download',
  [`${HOME}/Afbeeldingen`]: 'folder-pictures',
  [`${HOME}/Muziek`]: 'folder-music',
  [`${HOME}/Video's`]: 'folder-videos',
  [`${HOME}/Openbaar`]: 'folder-publicshare',
  [`${HOME}/Sjablonen`]: 'folder-templates',
};

function extOf(name) {
  const m = /\.([A-Za-z0-9]+)$/.exec(name);
  return m && !name.startsWith('.') ? m[1].toLowerCase() : '';
}
function mimeOf(name, isDir) {
  if (isDir) return 'inode/directory';
  return MIME[extOf(name)] || 'application/octet-stream';
}

/* ------------------------------------------------------------- the fake */

function dir(children = {}, opts = {}) {
  return { d: 1, c: children, m: opts.m ?? ago(12), b: opts.b ?? ago(200), a: opts.a ?? ago(1), ro: 0 };
}
function file(size, opts = {}) {
  return { d: 0, s: Math.round(size), m: opts.m ?? ago(10), b: opts.b ?? opts.m ?? ago(30), a: opts.a ?? ago(1), ro: opts.ro ? 1 : 0, e: opts.e };
}

function pristine() {
  const KB = 1024, MB = KB * 1024, GB = MB * 1024;
  const home = dir({
    Bureaublad: dir({
      Projecten: dir({
        Website: dir({ 'index.html': file(6 * KB, { m: ago(9) }), 'style.css': file(3 * KB, { m: ago(9) }) }, { m: ago(9) }),
        'Plan van aanpak.txt': file(4200, { m: ago(5) }),
        'Begroting.xlsx': file(22 * KB, { m: ago(4) }),
      }, { m: ago(4) }),
      'Vakantie 2026': dir({
        'IMG_3101.jpg': file(3.2 * MB, { m: ago(64) }),
        'IMG_3102.jpg': file(2.9 * MB, { m: ago(64) }),
        'IMG_3115.jpg': file(3.6 * MB, { m: ago(63) }),
      }, { m: ago(63) }),
      'Boodschappenlijst.txt': file(312, { m: ago(0, 3) }),
      'Notities.txt': file(1450, { m: ago(1, 2) }),
      'Belastingaangifte 2025.pdf': file(412 * KB, { m: ago(40) }),
      'Uitnodiging verjaardag.docx': file(38 * KB, { m: ago(6) }),
      'Strand Zandvoort.png': file(2.3 * MB, { m: ago(70) }),
      'Schermafbeelding 2026-10-24 201512.png': file(845 * KB, { m: ago(1, 5) }),
      'firefox_firefox.desktop': file(1200, { m: ago(90), e: { name: 'Firefox', icon: 'firefox', exec: 'firefox %u' } }),
      'steam.desktop': file(980, { m: ago(30), e: { name: 'Steam', icon: 'steam', exec: 'steam %U' } }),
    }, { m: ago(0, 3) }),
    Documenten: dir({
      Recepten: dir({
        'Appeltaart.txt': file(1800, { m: ago(120) }), 'Erwtensoep.txt': file(2100, { m: ago(300) }), 'Pannenkoeken.txt': file(950, { m: ago(30) }),
      }, { m: ago(30) }),
      Belasting: dir({ 'Aangifte 2025.pdf': file(412 * KB, { m: ago(40) }), 'Jaaropgave 2025.pdf': file(96 * KB, { m: ago(200) }) }, { m: ago(40) }),
      'Brief gemeente.docx': file(24 * KB, { m: ago(3) }),
      'Huishoudbudget 2026.xlsx': file(18 * KB, { m: ago(2) }),
      'Cv Robin.pdf': file(210 * KB, { m: ago(150) }),
      'Notulen bewonersvergadering.odt': file(31 * KB, { m: ago(16) }),
      'Verlanglijstje.txt': file(640, { m: ago(8) }),
    }, { m: ago(2) }),
    Downloads: dir({
      'ubuntu-26.04-desktop-amd64.iso': file(6.1 * GB, { m: ago(20) }),
      'handleiding-wasmachine.pdf': file(4.2 * MB, { m: ago(11) }),
      "vakantiefoto's.zip": file(184 * MB, { m: ago(60) }),
      'factuur-1043.pdf': file(88 * KB, { m: ago(2) }),
      'oude-installer.deb': file(45 * MB, { m: ago(140) }),
      'sticker-ontwerp.png': file(512 * KB, { m: ago(7) }),
    }, { m: ago(2) }),
    Afbeeldingen: dir({
      Schermafbeeldingen: dir({
        'Schermafbeelding 2026-10-20 091233.png': file(612 * KB, { m: ago(5) }),
        'Schermafbeelding 2026-10-24 201512.png': file(845 * KB, { m: ago(1) }),
      }, { m: ago(1) }),
      'Vakantie Texel': dir({
        'IMG_2041.jpg': file(3.1 * MB, { m: ago(400) }), 'IMG_2042.jpg': file(2.8 * MB, { m: ago(400) }),
        'IMG_2045.jpg': file(3.4 * MB, { m: ago(399) }), 'IMG_2046.jpg': file(3.0 * MB, { m: ago(399) }),
      }, { m: ago(399) }),
      'Achtergrond bergen.jpg': file(3.4 * MB, { m: ago(90) }),
      'Profielfoto.png': file(420 * KB, { m: ago(250) }),
    }, { m: ago(1) }),
    Muziek: dir({
      Favorieten: dir({ 'Zomeravond.mp3': file(7.4 * MB, { m: ago(500) }), 'Regen op het dak.flac': file(28 * MB, { m: ago(480) }) }, { m: ago(480) }),
      'Podcast - aflevering 12.mp3': file(48 * MB, { m: ago(9) }),
    }, { m: ago(9) }),
    "Video's": dir({
      'Verjaardag oma.mp4': file(812 * MB, { m: ago(33) }),
      'Schermopname 2026-10-01.webm': file(64 * MB, { m: ago(24) }),
    }, { m: ago(24) }),
    Openbaar: dir({}, { m: ago(500) }),
    Sjablonen: dir({}, { m: ago(500) }),
    '.bashrc': file(3771, { m: ago(500) }),
    '.profile': file(807, { m: ago(500) }),
    '.config': dir({ win11shell: dir({ 'settings.json': file(2 * KB, { m: ago(0) }) }) }, { m: ago(0) }),
    '.local': dir({ share: dir({}) }, { m: ago(3) }),
  }, { m: ago(0, 3) });

  const sys = (kids = {}) => dir(kids, { m: ago(30), b: ago(300) });
  const root = dir({
    bin: sys(), boot: sys(), etc: sys({ hostname: file(8, { ro: 1 }), 'os-release': file(386, { ro: 1 }) }),
    home: dir({ robin: home }, { m: ago(300) }), media: sys(), mnt: sys(), opt: sys(), srv: sys(), tmp: sys(), usr: sys(), var: sys(),
  }, { m: ago(30) });

  const trash = [
    { n: 'oud-concept.txt', o: `${HOME}/Documenten/oud-concept.txt`, t: ago(3), node: file(2100, { m: ago(20) }) },
    { n: 'IMG_1999.jpg', o: `${HOME}/Afbeeldingen/IMG_1999.jpg`, t: ago(6), node: file(3.1 * MB, { m: ago(410) }) },
    { n: 'Nieuwe map', o: `${DESKTOP}/Nieuwe map`, t: ago(1), node: dir({}, { m: ago(2) }) },
  ];
  return { root, trash, clip: { paths: [], op: 'copy' } };
}

/* --------------------------------------------------------- persistence */

let state = null;
try {
  if (new URLSearchParams(location.search).get('mockreset') === '1') sessionStorage.removeItem(KEY);
} catch { /* ignore */ }

function load() {
  try {
    const raw = sessionStorage.getItem(KEY);
    if (raw) {
      const s = JSON.parse(raw);
      if (s && s.root && Array.isArray(s.trash)) return s;
    }
  } catch { /* ignore */ }
  return pristine();
}
state = load();

let channel = null;
const sender = Math.random().toString(36).slice(2);
try { channel = new BroadcastChannel('win11shell-mock-files'); } catch { channel = null; }

function persist() {
  try { sessionStorage.setItem(KEY, JSON.stringify(state)); } catch { /* ignore */ }
}

/** Announce a change: local events + other frames. */
function changed(m, { dirs = [], trash = false, clip = false } = {}) {
  persist();
  const msg = { from: sender, dirs: [...new Set(dirs)], trash, clip };
  try { channel?.postMessage(msg); } catch { /* ignore */ }
  emitLocal(m, msg);
}

function emitLocal(m, { dirs = [], trash = false, clip = false }) {
  if (!m) return;
  for (const d of dirs) m.emit('files.dirChanged', { path: d });
  if (dirs.includes(DESKTOP) || trash) m.emit('files.desktopChanged', {});
  if (trash) { m.emit('files.dirChanged', { path: TRASH }); m.emit('files.trashChanged', {}); }
  if (clip) m.emit('files.clipboardChanged', { paths: [...state.clip.paths], op: state.clip.op });
}

/* ------------------------------------------------------------- helpers */

function err(code, message) { const e = new Error(message); e.code = code; return e; }

function norm(path) {
  let p = String(path ?? '');
  if (!p.startsWith('/')) p = p.trim();   // like host/services/files.py _abs: '/x/Map ' is a real name
  if (!p) return HOME;
  if (p === '~') return HOME;
  if (p.startsWith('~/')) p = HOME + p.slice(1);
  if (p.startsWith('file://')) { try { p = decodeURIComponent(new URL(p).pathname); } catch { p = p.slice(7); } }
  if (!p.startsWith('/')) p = '/' + p;
  const out = [];
  for (const seg of p.split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') out.pop(); else out.push(seg);
  }
  return '/' + out.join('/');
}
const parentOf = (p) => (p === '/' ? null : (p.slice(0, p.lastIndexOf('/')) || '/'));
const baseOf = (p) => p.slice(p.lastIndexOf('/') + 1);
const join = (d, n) => (d === '/' ? `/${n}` : `${d}/${n}`);

function trashEntryAt(path) {
  const p = String(path || '');
  if (p.startsWith(TRASH)) { let n = p.slice(TRASH.length); try { n = decodeURIComponent(n); } catch { /* keep */ } return state.trash.find((t) => t.n === n) || null; }
  const np = norm(p);
  if (!np.startsWith(TRASH_FILES + '/')) return null;
  const rest = np.slice(TRASH_FILES.length + 1);
  const top = rest.split('/')[0];
  return state.trash.find((t) => t.n === top) || null;
}
function nodeAt(path) {
  const te = trashEntryAt(path);
  if (te) {
    const rest = norm(path).slice(TRASH_FILES.length + 1).split('/').slice(1);
    let n = te.node;
    for (const seg of rest) { if (!n || !n.d || !Object.prototype.hasOwnProperty.call(n.c, seg)) return null; n = n.c[seg]; }
    return n;
  }
  const p = norm(path);
  if (p === '/') return state.root;
  let n = state.root;
  for (const seg of p.slice(1).split('/')) {
    if (!n || !n.d || !Object.prototype.hasOwnProperty.call(n.c, seg)) return null;
    n = n.c[seg];
  }
  return n;
}

function mustDir(path) {
  const n = nodeAt(path);
  if (!n) throw err('not_found', 'Deze map bestaat niet (meer).');
  if (!n.d) throw err('not_a_directory', 'Dit is geen map.');
  return n;
}
function mustNode(path) {
  const n = nodeAt(path);
  if (!n) throw err('not_found', 'Dit item bestaat niet (meer).');
  return n;
}

function sizeOf(n) {
  if (!n.d) return n.s || 0;
  let s = 0;
  for (const k of Object.keys(n.c)) s += sizeOf(n.c[k]);
  return s;
}
function countOf(n) {
  let files = 0, folders = 0;
  for (const k of Object.keys(n.c)) {
    const c = n.c[k];
    if (c.d) { folders++; const r = countOf(c); files += r.files; folders += r.folders; } else files++;
  }
  return { files, folders };
}

function iconFor(path, n, name) {
  if (n.d) {
    return SPECIAL_DIR_ICON[path] || 'folder';
  }
  if (n.e?.icon) return n.e.icon;
  return ICON[mimeOf(name, false)] || 'text-x-generic';
}

function item(path, n) {
  const name = baseOf(path) || '/';
  const isEntry = !n.d && extOf(name) === 'desktop' && !!n.e;
  return {
    name,
    displayName: isEntry ? n.e.name : name,
    path,
    uri: 'file://' + path.split('/').map(encodeURIComponent).join('/'),
    isDir: !!n.d,
    isDesktopEntry: isEntry,
    icon: iconFor(path, n, name),
    mime: mimeOf(name, n.d),
    size: n.d ? 0 : (n.s || 0),
    modified: n.m,
    hidden: name.startsWith('.') || !!n.h,
    isSymlink: false,
    typeName: isEntry ? typeName('x.desktop', false) : typeName(name, !!n.d),
  };
}

const collator = new Intl.Collator('nl', { numeric: true, sensitivity: 'base' });
function sortItems(items) {
  return items.sort((a, b) => (b.isDir - a.isDir) || collator.compare(a.displayName, b.displayName));
}

function list(path, showHidden = false) {
  const p = norm(path);
  const n = mustDir(p);
  const items = Object.keys(n.c).map((k) => item(join(p, k), n.c[k])).filter((it) => showHidden || !it.hidden);
  return sortItems(items);
}

function splitName(name, isDir) {
  if (isDir) return [name, ''];
  const m = /^(.+?)(\.[A-Za-z0-9]{1,8}(?:\.(?:gz|xz|bz2|zst))?)$/.exec(name);
  return m && !name.startsWith('.') ? [m[1], m[2]] : [name, ''];
}

/** "Nieuwe map", "Nieuwe map (2)" … */
function uniqueIn(dirNode, name, isDir, style = 'number') {
  if (!Object.prototype.hasOwnProperty.call(dirNode.c, name)) return name;
  const [stem, ext] = splitName(name, isDir);
  if (style === 'copy') {
    const first = `${stem} - kopie${ext}`;
    if (!Object.prototype.hasOwnProperty.call(dirNode.c, first)) return first;
    for (let i = 2; i < 10000; i++) {
      const cand = `${stem} - kopie (${i})${ext}`;
      if (!Object.prototype.hasOwnProperty.call(dirNode.c, cand)) return cand;
    }
  }
  for (let i = 2; i < 10000; i++) {
    const cand = `${stem} (${i})${ext}`;
    if (!Object.prototype.hasOwnProperty.call(dirNode.c, cand)) return cand;
  }
  return `${stem} ${Date.now()}${ext}`;
}

function validName(name) {
  const n = String(name ?? '').trim();
  if (!n) throw err('invalid_name', 'Geef een naam op.');
  if (n.includes('/') || n.includes('\0')) throw err('invalid_name', 'Een naam mag geen schuine streep (/) bevatten.');
  if (n === '.' || n === '..') throw err('invalid_name', 'Deze naam kan niet worden gebruikt.');
  if (n.length > 255) throw err('invalid_name', 'Deze naam is te lang.');
  return n;
}

const clone = (v) => JSON.parse(JSON.stringify(v));
const touch = (n) => { n.m = Date.now(); };

function isInside(child, parent) {
  return child === parent || child.startsWith(parent === '/' ? '/' : parent + '/');
}

function trashName(name, isDir) {
  const used = new Set(state.trash.map((t) => t.n));
  if (!used.has(name)) return name;
  const [stem, ext] = splitName(name, isDir);
  for (let i = 2; i < 10000; i++) {
    const cand = `${stem}.${i}${ext}`;
    if (!used.has(cand)) return cand;
  }
  return `${name}.${Date.now()}`;
}

function ensureDir(path) {
  const p = norm(path);
  let n = state.root, cur = '';
  for (const seg of p.slice(1).split('/').filter(Boolean)) {
    cur += '/' + seg;
    if (!n.c[seg]) n.c[seg] = dir({}, { m: Date.now(), b: Date.now() });
    n = n.c[seg];
    if (!n.d) throw err('not_a_directory', 'De oorspronkelijke map bestaat niet meer.');
  }
  return n;
}

function walk(path, n, fn, depth = 0) {
  if (depth > 32) return;
  for (const k of Object.keys(n.c)) {
    const p = join(path, k);
    if (fn(p, n.c[k]) === false) return false;
    if (n.c[k].d && walk(p, n.c[k], fn, depth + 1) === false) return false;
  }
  return true;
}

function typeLabel(name, n) {
  return typeName(name, !!n.d);
}

function trashItem(t) {
  const path = `${TRASH_FILES}/${t.n}`;
  const it = item(path, t.node);
  const orig = baseOf(t.o);
  return {
    ...it, name: t.n, displayName: it.isDesktopEntry ? it.displayName : orig, hidden: false,
    typeName: it.isDesktopEntry ? it.typeName : typeName(orig, !!t.node.d),
    originalPath: t.o, originalDir: parentOf(t.o) || '', deletedAt: t.t, trashName: t.n, inTrash: true,
    size: t.node.d ? 0 : sizeOf(t.node),
  };
}

/** files.progress for operations started with a jobId (the mock finishes at once). */
function progress(m, jobId, op, paths, out) {
  if (!jobId || !m) return;
  const total = paths.reduce((sum, p) => { const n = nodeAt(p); return sum + (n ? sizeOf(n) : 0); }, 0);
  m.emit('files.progress', { jobId, op, done: total, total, items: paths.length, count: paths.length, current: out[out.length - 1] || null, finished: true });
}

function placeNode(destDir, destNode, name, node, style) {
  const final = uniqueIn(destNode, name, !!node.d, style);
  destNode.c[final] = node;
  touch(destNode);
  return join(destDir, final);
}

/* --------------------------------------------------------------- methods */

const methods = {
  'files.desktop': async ({ showHidden = false } = {}) => {
    ensureDir(DESKTOP);
    return { dir: DESKTOP, items: list(DESKTOP, !!showHidden), exists: true, truncated: false };
  },

  'files.list': async ({ path, showHidden } = {}, m) => {
    if (typeof path === 'string' && path.startsWith('trash:')) {
      const items = state.trash.map(trashItem).sort((a, b) => (b.deletedAt || 0) - (a.deletedAt || 0));
      return { path: TRASH, parent: null, items, writable: false, trash: true };
    }
    const p = norm(path);
    const hidden = showHidden ?? !!m?.settings?.()?.explorer?.showHidden;
    const n = nodeAt(p);
    if (n && !n.d) throw err('not_dir', 'Dit is geen map.');
    return { path: p, parent: p === '/' ? null : parentOf(p), items: list(p, !!hidden), truncated: false, writable: p.startsWith(HOME) };
  },

  'files.stat': async ({ path } = {}) => {
    const te = trashEntryAt(path);
    if (te && norm(path) === `${TRASH_FILES}/${te.n}`) return trashItem(te);
    const p = norm(path);
    return item(p, mustNode(p));
  },

  'files.places': async () => {
    const en = LANG === 'en';
    return [
      { id: 'home', name: 'Robin', path: HOME, icon: 'user-home' },
      { id: 'desktop', name: en ? 'Desktop' : 'Bureaublad', path: DESKTOP, icon: 'user-desktop' },
      { id: 'documents', name: en ? 'Documents' : 'Documenten', path: `${HOME}/Documenten`, icon: 'folder-documents' },
      { id: 'downloads', name: 'Downloads', path: `${HOME}/Downloads`, icon: 'folder-download' },
      { id: 'music', name: en ? 'Music' : 'Muziek', path: `${HOME}/Muziek`, icon: 'folder-music' },
      { id: 'pictures', name: en ? 'Pictures' : 'Afbeeldingen', path: `${HOME}/Afbeeldingen`, icon: 'folder-pictures' },
      { id: 'videos', name: en ? 'Videos' : "Video's", path: `${HOME}/Video's`, icon: 'folder-videos' },
      { id: 'trash', name: en ? 'Recycle Bin' : 'Prullenbak', path: TRASH, icon: state.trash.length ? 'user-trash-full' : 'user-trash' },
      { id: 'root', name: en ? 'System drive' : 'Systeemschijf', path: '/', icon: 'drive-harddisk' },
    ].map((x) => ({ ...x, exists: true }));
  },

  // Opening/launching is a no-op in the mock (like --dry-run on the host).
  // like the host: folders (and the Prullenbak) open in our Verkenner, .desktop/files "start" (a no-op here,
  // as in --dry-run). Absolute system paths outside the fake (Taakbeheer › Bestandslocatie openen) are accepted.
  'files.open': async ({ path } = {}, m) => {
    if (typeof path === 'string' && path.startsWith('trash:')) { m?.emit('shell.openApp', { app: 'explorer', params: { path: TRASH } }); return {}; }
    const n = nodeAt(path);
    const p = norm(path);
    if (!n && p.startsWith(HOME)) throw err('not_found', 'Dit item bestaat niet (meer).');
    if (n?.d || (!n && !extOf(baseOf(p)))) { m?.emit('shell.openApp', { app: 'explorer', params: { path: p } }); return {}; }
    return { ok: true, dryRun: true };
  },
  'files.openWith': async ({ path } = {}) => { mustNode(path); return { ok: true, dryRun: true }; },
  'files.openTerminal': async () => ({ ok: true, dryRun: true }),

  'files.rename': async ({ path, newName } = {}, m) => {
    const p = norm(path);
    const n = mustNode(p);
    const name = validName(newName);
    const par = parentOf(p);
    const pn = mustDir(par);
    if (name === baseOf(p)) return { path: p, item: item(p, n) };
    if (Object.prototype.hasOwnProperty.call(pn.c, name)) {
      throw err('exists', `Er bestaat al een item met de naam ‘${name}’ in deze map.`);
    }
    if (n.ro) throw err('read_only', 'Dit item is alleen-lezen en kan niet worden hernoemd.');
    delete pn.c[baseOf(p)];
    pn.c[name] = n;
    touch(pn);
    const np = join(par, name);
    state.clip.paths = state.clip.paths.map((x) => (isInside(x, p) ? np + x.slice(p.length) : x));
    changed(m, { dirs: [par] });
    return { path: np, item: item(np, n) };
  },

  'files.createFolder': async ({ dir: d, name } = {}, m) => {
    const p = norm(d);
    const dn = mustDir(p);
    const wanted = name ? validName(name) : 'Nieuwe map';
    const np = placeNode(p, dn, wanted, dir({}, { m: Date.now(), b: Date.now(), a: Date.now() }), 'number');
    changed(m, { dirs: [p] });
    return { path: np, item: item(np, nodeAt(np)) };
  },

  'files.createFile': async ({ dir: d, name, kind = 'text' } = {}, m) => {
    const p = norm(d);
    const dn = mustDir(p);
    if (kind !== 'text' && kind !== 'empty') throw err('bad_args', 'Onbekend soort bestand.');
    const en = LANG === 'en';
    const wanted = name ? validName(name) : (kind === 'empty' ? (en ? 'New document' : 'Nieuw document') : (en ? 'New Text Document.txt' : 'Nieuw tekstdocument.txt'));
    const np = placeNode(p, dn, wanted, file(0, { m: Date.now(), b: Date.now(), a: Date.now() }), 'number');
    changed(m, { dirs: [p] });
    return { path: np, item: item(np, nodeAt(np)) };
  },

  'files.trash': async ({ paths = [] } = {}, m) => {
    const failed = [], trashed = [], dirs = [];
    for (const raw of [].concat(paths)) {
      const p = norm(raw);
      const n = nodeAt(p);
      const par = parentOf(p);
      if (!n && p.startsWith(HOME + '/') && !trashEntryAt(p)) {
        // a path the fake does not know (e.g. Taakbeheer's mock "large files"): pretend it was there
        const tn = trashName(baseOf(p), false);
        state.trash.push({ n: tn, o: p, t: Date.now(), node: file(0, { m: Date.now() }) });
        trashed.push({ path: p, name: tn });
        continue;
      }
      if (!n) { failed.push({ path: raw, error: 'Dit item bestaat niet (meer).', code: 'not_found' }); continue; }
      if (!par || p === HOME || p === '/' || !p.startsWith(HOME + '/') || trashEntryAt(p)) {
        failed.push({ path: raw, error: 'Dit item kan niet naar de Prullenbak.', code: 'denied' }); continue;
      }
      const pn = nodeAt(par);
      delete pn.c[baseOf(p)];
      touch(pn);
      const tn = trashName(baseOf(p), !!n.d);
      state.trash.push({ n: tn, o: p, t: Date.now(), node: n });
      trashed.push({ path: p, name: tn });
      dirs.push(par);
    }
    if (trashed.length) changed(m, { dirs, trash: true });
    return { ok: failed.length === 0, failed, trashed };
  },

  'files.restoreTrash': async ({ names, paths, all } = {}, m) => {
    const restored = [], failed = [], dirs = [];
    let wanted = all ? state.trash.map((t) => t.n) : [].concat(names ?? paths ?? []);
    if (!wanted.length) throw err('bad_args', 'Kies eerst een of meer items.');
    // trash names or item paths (inside ~/.local/share/Trash/files) are both accepted, like the host
    wanted = wanted.map((x) => (typeof x === 'string' && (x.startsWith('/') || x.startsWith('trash:')) ? (trashEntryAt(x)?.n ?? x) : x));
    for (const name of wanted) {
      const i = state.trash.findIndex((t) => t.n === name);
      if (i < 0) { failed.push({ name, error: 'Dit item staat niet (meer) in de Prullenbak.' }); continue; }
      const t = state.trash[i];
      try {
        const par = parentOf(t.o);
        const pn = ensureDir(par);
        const np = placeNode(par, pn, baseOf(t.o), t.node, 'number');
        state.trash.splice(i, 1);
        restored.push(np);
        dirs.push(par);
      } catch (e) {
        failed.push({ name, error: e.message });
      }
    }
    if (restored.length) changed(m, { dirs, trash: true });
    return { ok: failed.length === 0, restored, failed };
  },

  'files.trashInfo': async () => ({
    count: state.trash.length,
    size: state.trash.reduce((s, t) => s + sizeOf(t.node), 0),
    approx: false,
  }),

  'files.emptyTrash': async (args, m) => {
    const count = state.trash.length;
    state.trash = [];
    changed(m, { trash: true });
    return { ok: true, removed: count };
  },

  'files.copy': async ({ paths = [], dest, jobId } = {}, m) => {
    const d = norm(dest);
    const dn = mustDir(d);
    const out = [], failed = [];
    for (const raw of [].concat(paths)) {
      const p = norm(raw);
      const n = nodeAt(p);
      if (!n) { failed.push({ path: raw, error: 'Dit item bestaat niet (meer).' }); continue; }
      if (n.d && isInside(d, p)) { failed.push({ path: raw, error: 'Een map kan niet in zichzelf worden gekopieerd.' }); continue; }
      const c = clone(n);
      c.b = Date.now(); c.a = Date.now();
      out.push(placeNode(d, dn, baseOf(p), c, parentOf(p) === d ? 'copy' : 'number'));
    }
    if (out.length) changed(m, { dirs: [d] });
    progress(m, jobId, 'copy', out, out);
    return { ok: failed.length === 0, paths: out, failed, ...(jobId ? { jobId } : {}) };
  },

  'files.move': async ({ paths = [], dest, jobId } = {}, m) => {
    const d = norm(dest);
    const dn = mustDir(d);
    const out = [], failed = [], dirs = [d];
    for (const raw of [].concat(paths)) {
      const p = norm(raw);
      const n = nodeAt(p);
      const par = parentOf(p);
      if (!n || !par) { failed.push({ path: raw, error: 'Dit item bestaat niet (meer).' }); continue; }
      if (par === d) { out.push(p); continue; }
      if (n.d && isInside(d, p)) { failed.push({ path: raw, error: 'Een map kan niet in zichzelf worden verplaatst.' }); continue; }
      const pn = nodeAt(par);
      delete pn.c[baseOf(p)];
      touch(pn);
      out.push(placeNode(d, dn, baseOf(p), n, 'number'));
      dirs.push(par);
    }
    if (out.length) changed(m, { dirs });
    progress(m, jobId, 'move', out, out);
    return { ok: failed.length === 0, paths: out, failed, ...(jobId ? { jobId } : {}) };
  },

  'files.clipboardSet': async ({ paths = [], op = 'copy' } = {}, m) => {
    if (op !== 'copy' && op !== 'cut') throw err('bad_args', 'Ongeldige bewerking.');
    state.clip = { paths: [].concat(paths).map(norm).filter((p) => !!nodeAt(p)), op };
    changed(m, { clip: true });
    return { paths: [...state.clip.paths], op };
  },

  'files.clipboardClear': async (args, m) => {
    state.clip = { paths: [], op: 'copy' };
    changed(m, { clip: true });
    return {};
  },

  'files.clipboardGet': async () => ({
    paths: state.clip.paths.filter((p) => !!nodeAt(p)),
    op: state.clip.op,
  }),

  'files.paste': async ({ dest, jobId } = {}, m) => {
    const paths = state.clip.paths.filter((p) => !!nodeAt(p));
    if (!paths.length) throw err('empty', 'Er is niets om te plakken.');
    const r = state.clip.op === 'cut'
      ? await methods['files.move']({ paths, dest, jobId }, m)
      : await methods['files.copy']({ paths, dest, jobId }, m);
    if (state.clip.op === 'cut') { state.clip = { paths: [], op: 'copy' }; changed(m, { clip: true }); }
    return { ok: r.ok, paths: r.paths, failed: r.failed, ...(jobId ? { jobId } : {}) };
  },

  'files.properties': async ({ path, paths } = {}) => {
    if (paths && !path) {
      // multi-selection (host: _multi_properties)
      const list0 = [].concat(paths).map(norm).filter((p) => nodeAt(p));
      let size = 0, files = 0, folders = 0;
      const mimes = new Set();
      for (const p of list0) {
        const n = nodeAt(p);
        size += sizeOf(n);
        if (n.d) { const c = countOf(n); folders += 1 + c.folders; files += c.files; } else files += 1;
        mimes.add(mimeOf(baseOf(p), !!n.d));
      }
      const en = LANG === 'en';
      const type = mimes.size === 1 ? `${en ? 'All of type ' : 'Alle van het type '}${typeName(baseOf(list0[0] || 'x'), [...mimes][0] === 'inode/directory')}` : (en ? 'Multiple types' : 'Meerdere typen');
      const parents = new Set(list0.map(parentOf));
      return { name: `${list0.length} items`, path: null, type, size, contains: { files, folders }, partial: false,
        location: parents.size === 1 ? [...parents][0] : null, count: list0.length };
    }
    const te = trashEntryAt(path);
    const p = te ? norm(path) : norm(path);
    const n = mustNode(p);
    const it = te && p === `${TRASH_FILES}/${te.n}` ? trashItem(te) : item(p, n);
    const size = sizeOf(n);
    const writable = !n.ro && p.startsWith(HOME);
    return {
      name: it.name,
      displayName: it.displayName,
      path: p,
      location: parentOf(p),
      type: it.typeName,
      mime: it.mime,
      icon: it.icon,
      isDir: !!n.d,
      isDesktopEntry: it.isDesktopEntry,
      size,
      sizeOnDisk: n.d ? null : Math.ceil(size / 4096) * 4096,
      contains: n.d ? countOf(n) : null,
      partial: false,
      created: n.b,
      modified: n.m,
      accessed: n.a,
      owner: p.startsWith(HOME) ? 'robin' : 'root',
      group: p.startsWith(HOME) ? 'robin' : 'root',
      permissions: n.d ? (writable ? 'rwxr-xr-x' : 'r-xr-xr-x') : (writable ? 'rw-r--r--' : 'r--r--r--'),
      mode: n.d ? (writable ? '0o755' : '0o555') : (writable ? '0o644' : '0o444'),
      readOnly: !writable,
      hidden: it.hidden,
      isSymlink: false,
      target: null,
      executable: !n.d && /\.(sh|py)$/.test(p),
    };
  },

  'files.setAttributes': async ({ path, readOnly, hidden } = {}, m) => {
    const p = norm(path);
    const n = mustNode(p);
    if (!p.startsWith(HOME)) throw err('permission_denied', 'Je hebt geen toestemming om dit item te wijzigen.');
    if (readOnly != null) n.ro = readOnly ? 1 : 0;
    if (hidden != null) {
      if (baseOf(p).startsWith('.') && !hidden) {
        throw err('dotfile', 'Dit item is verborgen omdat de naam met een punt begint. Wijzig de naam om het zichtbaar te maken.');
      }
      n.h = hidden ? 1 : 0;
    }
    changed(m, { dirs: [parentOf(p)] });
    return { ok: true };
  },

  /* older name used before files.setAttributes existed (kept for callers that still use it) */
  'files.setReadOnly': async ({ path, readOnly } = {}, m) => methods['files.setAttributes']({ path, readOnly: !!readOnly }, m),

  'files.compress': async ({ paths = [], dest, jobId } = {}, m) => {
    const list0 = [].concat(paths).map(norm).filter((p) => nodeAt(p));
    if (!list0.length) throw err('not_found', 'Er is niets om te comprimeren.');
    const d = dest ? norm(dest) : parentOf(list0[0]);
    const dn = mustDir(d);
    const first = baseOf(list0[0]);
    const stem = list0.length === 1 ? splitName(first, !!nodeAt(list0[0]).d)[0] : (baseOf(d) || 'Archief');
    const size = Math.round(list0.reduce((s, p) => s + sizeOf(nodeAt(p)), 0) * 0.62) + 220;
    const np = placeNode(d, dn, `${stem}.zip`, file(size, { m: Date.now(), b: Date.now() }), 'number');
    changed(m, { dirs: [d] });
    progress(m, jobId, 'compress', [np], [np]);
    return { path: np, ...(jobId ? { jobId } : {}) };
  },

  'files.extract': async ({ path, dest, jobId } = {}, m) => {
    const p = norm(path);
    const n = mustNode(p);
    if (n.d || !/\.(zip|tar|gz|tgz|xz|bz2|7z)$/i.test(p)) throw err('not_archive', 'Dit is geen archief dat kan worden uitgepakt.');
    const d = dest ? norm(dest) : parentOf(p);
    const dn = mustDir(d);
    const stem = baseOf(p).replace(/\.(tar\.(gz|xz|bz2|zst)|tgz|zip|tar|7z)$/i, '');
    const kids = {
      'Lees mij.txt': file(1200, { m: Date.now() }),
      'Afbeelding 1.jpg': file(Math.round((n.s || 1e6) * 0.4), { m: Date.now() }),
      'Afbeelding 2.jpg': file(Math.round((n.s || 1e6) * 0.35), { m: Date.now() }),
    };
    const np = placeNode(d, dn, stem, dir(kids, { m: Date.now(), b: Date.now() }), 'number');
    changed(m, { dirs: [d] });
    progress(m, jobId, 'extract', [np], [np]);
    return { path: np, ...(jobId ? { jobId } : {}) };
  },

  'files.find': async ({ path, query = '', limit = 500, showHidden = false, mimePrefix = '' } = {}) => {
    const p = norm(path);
    const n = mustDir(p);
    const q = String(query).trim().toLowerCase();
    // wildcards * ? like the host (fnmatch); plain text = "name contains"
    const re = /[*?]/.test(q) ? new RegExp(`^${q.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.')}$`) : null;
    const items = [];
    let truncated = false;
    if (q || mimePrefix) {
      walk(p, n, (cp, cn) => {
        const name = baseOf(cp);
        if (!showHidden && (name.startsWith('.') || cn.h)) return true;
        if (mimePrefix && !mimeOf(name, !!cn.d).startsWith(mimePrefix)) return true;
        const low = name.toLowerCase();
        if (!q || (re ? re.test(low) : low.includes(q))) {
          if (items.length >= limit) { truncated = true; return false; }
          items.push(item(cp, cn));
        }
        return true;
      });
    }
    return { items: sortItems(items), truncated };
  },

  'files.watch': async () => ({}),
  'files.unwatch': async () => ({}),
};

/* every call first picks up the UI language (typeName, places and new-item names follow it) */
const wrapped = Object.fromEntries(Object.entries(methods).map(([name, fn]) => [name, (args, m) => {
  try { LANG = m?.settings?.()?.language === 'en' ? 'en' : 'nl'; } catch { /* keep */ }
  return fn(args || {}, m);
}]));

export default {
  methods: wrapped,
  start(m) {
    if (channel) {
      channel.onmessage = (ev) => {
        const msg = ev.data || {};
        if (!msg || msg.from === sender) return;
        state = load();
        emitLocal(m, msg);
      };
    }
  },
};

/* exported for tests / other mocks (e.g. apps.recent could use real-looking paths) */
export const MOCK_HOME = HOME;
export const MOCK_DESKTOP = DESKTOP;
