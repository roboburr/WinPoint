// Win11 Shell — mock storage.* (owned by the taskmgr agent). SPEC §6.12.
// Shapes mirror host/services/storage.py incl. its additive fields (volume percent/model/hidden,
// cached scan answers, scanDone path/bytes/files/tookMs/errorCode, tree truncated/count,
// categories running/progress, cleanup available/details objects, old-download paths).
//
// Volumes mirror the user's real layout (/ 233 GB at 94 %, /srv/vm 384 GB at 95 %, NTFS
// 564 GB, BACKUP 224 GB, 3 TB, 22 TB, exFAT 466 GB). Scans walk an in-memory tree of
// plausible folders (Dutch XDG names) with live progress events; categories, cleanup
// items and large files are derived from the same data.

const KB = 1024, MB = 1024 ** 2, GB = 1024 ** 3;
const G = (x) => Math.round(x * GB);
const M = (x) => Math.round(x * MB);
const HOME = '/home/robin';
const DAY = 86400000;
const NOW = Date.now();

/* ------------------------------------------------------------------ PRNG */
function hash(s) {
  let h = 2166136261;
  for (let i = 0; i < s.length; i++) { h ^= s.charCodeAt(i); h = Math.imul(h, 16777619); }
  return h >>> 0;
}
function seeded(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/* ----------------------------------------------------------- folder specs */
// object = folder, number = file (name with extension) or auto-filled folder (no extension)

const HOME_SPEC = {
  '.local': {
    share: {
      Steam: {
        steamapps: {
          common: { 'Counter-Strike 2': G(34.6), 'Portal 2': G(12.4), 'Stardew Valley': G(0.62), 'Proton 9.0 (Beta)': G(1.1), SteamLinuxRuntime_sniper: G(0.84) },
          shadercache: G(2.3), workshop: G(1.4), downloading: G(0.2),
        },
        ubuntu12_32: G(0.42), ubuntu12_64: G(0.31), logs: M(48),
      },
      flatpak: G(1.6),
      Trash: { files: G(3.1), info: M(0.4) },
      tracker3: M(240), fonts: M(86), 'gnome-shell': M(12), 'recently-used.xbel': M(0.3),
    },
    state: M(120), bin: M(64),
  },
  "Video's": {
    'Vakantie Italië 2025': { 'IMG_4512.MOV': G(2.8), 'IMG_4520.MOV': G(1.9), 'IMG_4533.MOV': G(1.2), 'Samenvatting Italië.mp4': G(0.9), 'IMG_4541.MOV': M(640) },
    'Bruiloft Sanne en Tim.mp4': G(6.4), 'Interstellar (2014).mkv': G(9.8), Schermopnamen: G(1.6), Kinderen: G(1.7),
  },
  Downloads: {
    'ubuntu-26.04-desktop-amd64.iso': G(6.2), 'Win11_24H2_Dutch_x64.iso': G(5.8), 'oude-backup-telefoon.zip': G(3.2),
    'cs2-maps.zip': G(2.4), drivers: G(1.1), 'Fotoboek-2024.pdf': M(310), 'VirtualBox-7.1.deb': M(112),
    'steam_latest.deb': M(4), 'belastingaangifte-2025.pdf': M(2.1), 'factuur-oktober.pdf': M(0.4),
  },
  '.cache': {
    mozilla: { firefox: G(2.1) }, BraveSoftware: G(1.8), spotify: G(1.9), JetBrains: G(1.6), pip: G(1.2), 'go-build': G(1.1),
    thumbnails: G(0.9), yarn: G(0.7), 'vscode-cpptools': G(0.6), mesa_shader_cache: M(420), nvidia: M(380), tracker3: M(310),
    typescript: M(260), electron: M(240), 'node-gyp': M(90), fontconfig: M(12), 'gstreamer-1.0': M(4),
  },
  Afbeeldingen: {
    'Camera-uploads': G(5.6), 'Vakantie 2024': G(0.9), Schermafbeeldingen: M(820), Wallpapers: M(410), Scans: M(390),
  },
  snap: {
    'telegram-desktop': G(2.6), firefox: G(2.4), discord: G(1.6), code: G(1.2), spotify: G(1.0), 'snap-store': M(210), 'snapd-desktop-integration': M(30),
  },
  projecten: {
    'rust-game': { target: G(3.9), src: M(3), '.git': M(150) },
    datasets: G(1.4),
    'website-oma': { node_modules: G(1.1), '.git': M(220), dist: M(40) },
    win11shell: { node_modules: M(410), '.git': M(96), ui: M(4), host: M(1) },
    scripts: M(12),
  },
  '.config': {
    BraveSoftware: G(2.2), Code: G(1.4), discord: G(1.3), 'google-chrome': M(940), libreoffice: M(180), syncthing: M(120),
    nvim: M(3), hypr: M(0.1), win11shell: M(1),
  },
  '.var': { app: { 'org.signal.Signal': G(1.9), 'com.obsproject.Studio': G(1.8), 'org.gimp.GIMP': G(0.4), 'com.github.tchx84.Flatseal': M(12) } },
  Documenten: {
    Werk: G(1.6), Hypotheek: M(670), Scans: M(560), Belastingen: M(420), Recepten: M(80),
    'CV Robin.docx': M(2), 'Begroting 2027.odt': M(1.2),
  },
  '.rustup': { toolchains: G(3.0), downloads: M(80) },
  Muziek: { Lievelingsnummers: G(1.2), Podcasts: G(0.9), Rock: G(0.7) },
  '.cargo': { registry: G(1.9), bin: M(280) },
  '.npm': { _cacache: G(1.1) },
  go: { pkg: M(860), bin: M(40) },
  Bureaublad: { 'Nieuwe map': M(540), 'presentatie.odp': M(42), 'Foto oma.jpg': M(6), 'notities.txt': 9 * KB },
  '.bash_history': 180 * KB, '.bashrc': 4 * KB, '.profile': 1 * KB,
};

const TREES = {
  '/': {
    home: { robin: HOME_SPEC },
    usr: { lib: G(9.6), share: G(6.8), bin: G(1.9), libexec: M(820), include: M(310), src: G(1.3), local: M(120) },
    var: { lib: { docker: G(6.4), snapd: G(4.1), flatpak: G(2.9), apt: M(420), libvirt: M(180) }, log: G(1.1), cache: { apt: M(910), 'debconf': M(4) }, crash: M(310), tmp: M(40) },
    snap: { 'gnome-42-2204': M(980), 'core22': M(310), 'firefox': M(980), 'code': M(1150), 'telegram-desktop': M(420), 'discord': M(380), 'gtk-common-themes': M(330) },
    opt: { 'brave.com': { brave: M(620) }, google: { chrome: M(410) }, nvidia: G(1.6) },
    boot: M(350), root: M(410), tmp: M(120), etc: M(24),
  },
  '/srv/vm': {
    images: { 'win11.qcow2': G(162), 'kali-2025.qcow2': G(41), 'ubuntu-server-24.04.qcow2': G(48), 'fedora-41.qcow2': G(36), 'homeassistant.qcow2': G(32) },
    snapshots: { 'win11-voor-update.qcow2': G(28) },
    iso: { 'Win11_24H2_Dutch_x64.iso': G(5.8), 'ubuntu-24.04-live-server-amd64.iso': G(2.6), 'Fedora-Workstation-Live-41.iso': G(2.2) },
    backups: G(7),
  },
  '/run/media/robin/E2CEC932CEC8FFB1': {
    Games: G(66), Users: { robin: { Videos: G(52), Pictures: G(36), Downloads: G(21), AppData: G(17), Documents: G(8) } },
    'Program Files': G(38), Windows: G(26), 'Program Files (x86)': G(14), 'pagefile.sys': G(8), 'hiberfil.sys': G(6.4), ProgramData: G(4), Recovery: G(1),
  },
  '/run/media/robin/BACKUP': { timeshift: { snapshots: G(52) }, 'telefoon-backup-2025': G(22), 'documenten-sync': G(14) },
  '/run/media/robin/3TB': { 'Steam-bibliotheek': G(560), 'Foto-archief': G(350), 'Oude schijf kopie 2019': G(190), Opnames: G(112) },
  '/run/media/robin/22TB': { Films: G(95), Series: G(50), 'Back-ups': G(20), 'Foto-archief (RAW)': G(10), 'Muziek (FLAC)': G(4.6) },
  '/run/media/robin/extra': { Overdracht: G(80), 'Installatie-USB': G(22), "Foto's van Lisa": G(71.6) },
};

const VOLUMES = [
  { id: 'fa7665e9-9308-487c-be15-02c02abde18a', device: '/dev/nvme1n1p2', label: null, name: 'Systeemschijf', mount: '/', fsType: 'ext4', fsTypeRaw: 'ext4', total: G(232.6), used: G(207.0), removable: false, isSystem: true, isHome: true, kind: 'nvme', model: 'GIGABYTE GP-GSM2NE8256GNTD' },
  { id: 'D44C7C8D4C7C6BDC', device: '/dev/sdc3', label: '22TB', name: '22TB', mount: '/run/media/robin/22TB', fsType: 'NTFS', fsTypeRaw: 'fuseblk', total: G(22352), used: G(180), removable: false, isSystem: false, isHome: false, kind: 'hdd', model: 'ST24000DM001-3Y7103' },
  { id: '8E96DB7696DB5D71', device: '/dev/sdd3', label: '3TB', name: '3TB', mount: '/run/media/robin/3TB', fsType: 'NTFS', fsTypeRaw: 'fuseblk', total: G(2794), used: G(1213), removable: false, isSystem: false, isHome: false, kind: 'hdd', model: 'ST3000VX009-2AY10G' },
  { id: 'E2CEC932CEC8FFB1', device: '/dev/nvme0n1p2', label: null, name: '563 GB-schijf', mount: '/run/media/robin/E2CEC932CEC8FFB1', fsType: 'NTFS', fsTypeRaw: 'fuseblk', total: G(563.3), used: G(298.3), removable: false, isSystem: false, isHome: false, kind: 'nvme', model: 'Lexar SSD NM620 1TB' },
  { id: '1b1aacba-b845-4664-92c9-e94e30fcfcd1', device: '/dev/nvme0n1p3', label: 'vmstore', name: 'Virtuele machines', mount: '/srv/vm', fsType: 'ext4', fsTypeRaw: 'ext4', total: G(383.3), used: G(344.5), removable: false, isSystem: false, isHome: false, kind: 'nvme', model: 'Lexar SSD NM620 1TB' },
  { id: '5EE05716E056F42D', device: '/dev/sdb1', label: 'BACKUP', name: 'BACKUP', mount: '/run/media/robin/BACKUP', fsType: 'NTFS', fsTypeRaw: 'fuseblk', total: G(223.6), used: G(89), removable: true, isSystem: false, isHome: false, kind: 'usb', model: 'KINGSTON SA400S37240G' },
  { id: '6C01-2BA5', device: '/dev/sda3', label: 'extra', name: 'extra', mount: '/run/media/robin/extra', fsType: 'exFAT', fsTypeRaw: 'exfat', total: G(465), used: G(174.2), removable: true, isSystem: false, isHome: false, kind: 'usb', model: 'HGST HTS545050A7E680' },
].map((v) => {
  // like statvfs on ext4: 5 % of the blocks is reserved for root, so used + free < total (df's Use% counts it as taken)
  const reserve = v.fsTypeRaw === 'ext4' ? Math.round(v.total * 0.05) : 0;
  return { ...v, reserve, free: Math.max(0, v.total - v.used - reserve), readOnly: false, hidden: false };
});
const withPercent = ({ reserve, ...v }) => ({ ...v, percent: Math.round((v.used / Math.max(1, v.used + v.free)) * 1000) / 10 });

/* ------------------------------------------------------------ tree model */

const EXT_MIME = {
  iso: 'application/x-cd-image', mkv: 'video/x-matroska', mp4: 'video/mp4', mov: 'video/quicktime', webm: 'video/webm',
  qcow2: 'application/x-qemu-disk', zip: 'application/zip', deb: 'application/vnd.debian.binary-package', pdf: 'application/pdf',
  jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', heic: 'image/heic', cr3: 'image/x-canon-cr3', vpk: 'application/octet-stream',
  sys: 'application/octet-stream', docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document',
  odt: 'application/vnd.oasis.opendocument.text', odp: 'application/vnd.oasis.opendocument.presentation', flac: 'audio/flac',
  mp3: 'audio/mpeg', txt: 'text/plain', xbel: 'application/xml', bin: 'application/octet-stream', pak: 'application/octet-stream',
  rlib: 'application/octet-stream', so: 'application/x-sharedlib', db: 'application/vnd.sqlite3', sqlite: 'application/vnd.sqlite3',
  tar: 'application/x-tar', zst: 'application/zstd', gz: 'application/gzip', log: 'text/x-log', json: 'application/json', js: 'text/javascript',
};
const extOf = (name) => (name.match(/\.([A-Za-z0-9]+)$/)?.[1] || '').toLowerCase();
const isFileName = (name) => (!name.startsWith('.') && Object.prototype.hasOwnProperty.call(EXT_MIME, extOf(name)))
  || /^\.(bash_history|bashrc|profile)$/.test(name);
const mimeOf = (name) => EXT_MIME[extOf(name)] || 'application/octet-stream';

function fileNamesFor(dirName, path, n, r) {
  const d = (dirName + ' ' + path).toLowerCase();
  const pick = (arr) => arr[Math.floor(r() * arr.length)];
  const out = [];
  for (let i = 0; i < n; i++) {
    let name;
    if (/video|film|series|opnames|kinderen|schermopnamen/.test(d)) name = /series/.test(d) ? `S0${1 + (i % 6)}E${String(1 + i).padStart(2, '0')}.mkv` : pick(['Verjaardag', 'Strand', 'Kerst', 'Sinterklaas', 'Zwemles', 'Dagje uit', 'Opname']) + ` ${2019 + (i % 7)}-${String(1 + i).padStart(2, '0')}.mp4`;
    else if (/foto|camera|afbeeld|picture|wallpaper|vakantie|scans|lisa/.test(d)) name = /raw/.test(d) ? `IMG_${4000 + i * 37}.CR3` : `IMG_2025${String(1001 + i * 13)}.jpg`;
    else if (/muziek|music|podcast|rock|lieveling/.test(d)) name = /flac/.test(d) ? `${String(i + 1).padStart(2, '0')} - Track.flac` : `${pick(['Avond', 'Zomer', 'Weg', 'Licht', 'Thuis', 'Afl.'])} ${i + 1}.mp3`;
    else if (/steam|counter|portal|stardew|proton|runtime|games|workshop/.test(d)) name = pick(['pak01', 'pak02', 'textures', 'sound', 'maps', 'shaders', 'models']) + `_${String(i).padStart(3, '0')}.vpk`;
    else if (/cache|thumbnail|mesa|nvidia|electron|pip|yarn|npm|_cacache|go-build|typescript|gyp|jetbrains/.test(d)) name = `${(hash(path + i) >>> 0).toString(16).padStart(8, '0')}${(hash(i + path) >>> 8).toString(16)}`;
    else if (/target|toolchains|registry|pkg|rustup|cargo/.test(d)) name = pick(['libserde', 'libtokio', 'libbevy', 'librustc_driver', 'libstd', 'libcore']) + `-${(hash(path + i) % 0xffff).toString(16)}.rlib`;
    else if (/backup|timeshift|snapshots|sync|kopie|overdracht|installatie/.test(d)) name = `backup-2025-${String(1 + (i % 12)).padStart(2, '0')}-${String(1 + i).padStart(2, '0')}.tar.zst`;
    else if (/werk|hypotheek|belasting|recepten|document/.test(d)) name = pick(['Offerte', 'Notulen', 'Contract', 'Overzicht', 'Brief', 'Jaaropgave']) + ` ${2021 + (i % 5)}.pdf`;
    else if (/node_modules|dist|src|ui|host|\.git|scripts|datasets/.test(d)) name = /datasets/.test(d) ? `data-${i}.csv` : `chunk-${(hash(path + i) % 0xfffff).toString(16)}.js`;
    else if (/flatpak|snap|signal|obs|gimp|flatseal|telegram|firefox|discord|code|spotify|brave|chrome|libreoffice/.test(d)) name = pick(['data', 'storage', 'IndexedDB', 'Cache_Data', 'blob_storage', 'Local Storage']) + `-${i}.db`;
    else if (/program|windows|appdata|recovery|programdata/.test(d)) name = pick(['setup', 'update', 'resources', 'driver', 'data']) + `${i}.bin`;
    else name = `bestand-${i + 1}.bin`;
    out.push(name);
  }
  return out;
}

function avgFileSize(path) {
  const d = path.toLowerCase();
  if (/video|film|series|opnames/.test(d)) return 900 * MB;
  if (/qcow2|iso|images/.test(d)) return 8 * GB;
  if (/foto|camera|afbeeld|picture/.test(d)) return 4 * MB;
  if (/muziek|music|podcast/.test(d)) return 8 * MB;
  if (/cache|thumbnail|node_modules|npm|cargo|target/.test(d)) return 60 * KB;
  if (/steam|games/.test(d)) return 25 * MB;
  return 2 * MB;
}

const nodeCache = new Map();

/** Build (and cache) the full node for path from a spec value. */
function buildNode(name, path, spec) {
  if (nodeCache.has(path)) return nodeCache.get(path);
  let node;
  if (typeof spec === 'number') {
    if (isFileName(name)) {
      node = { name, path, size: spec, files: 1, isDir: false };
    } else {
      // auto-filled folder: a handful of big files + many small ones
      const r = seeded(hash(path));
      const n = 3 + Math.floor(r() * 6);
      const names = fileNamesFor(name, path, n, r);
      let left = spec;
      const children = [];
      names.forEach((fn, i) => {
        const share = i === n - 1 ? 0.5 : 0.18 + r() * 0.3;
        const sz = Math.max(1, Math.round(left * share));
        left -= sz;
        children.push({ name: fn, path: `${path}/${fn}`, size: sz, files: 1, isDir: false });
      });
      const avg = avgFileSize(path);
      const restFiles = Math.max(0, Math.round(left / avg));
      if (left > 0) children.push({ name: '…', path, size: left, files: restFiles, isDir: false, rest: true });
      children.sort((a, b) => b.size - a.size);
      node = { name, path, size: spec, files: n + restFiles, isDir: true, children };
    }
  } else {
    const children = Object.entries(spec).map(([k, v]) => buildNode(k, `${path === '/' ? '' : path}/${k}`, v));
    children.sort((a, b) => b.size - a.size);
    node = { name, path, size: children.reduce((a, c) => a + c.size, 0), files: children.reduce((a, c) => a + c.files, 0), isDir: true, children };
  }
  nodeCache.set(path, node);
  return node;
}

const ROOTS = Object.fromEntries(Object.entries(TREES).map(([mount, spec]) => [mount, buildNode(mount === '/' ? '/' : mount.split('/').pop(), mount, spec)]));

/** Find the full node for an absolute path (or a generated one for unknown paths). */
function findNode(path) {
  path = String(path || HOME).replace(/\/+$/, '') || '/';
  const mounts = Object.keys(ROOTS).sort((a, b) => b.length - a.length);
  const mount = mounts.find((mt) => path === mt || path.startsWith(mt === '/' ? '/' : mt + '/'));
  if (mount) {
    let node = ROOTS[mount];
    const rest = path === mount ? [] : path.slice(mount === '/' ? 1 : mount.length + 1).split('/');
    for (const seg of rest) {
      const next = node?.children?.find((c) => c.name === seg && !c.rest);
      if (!next) { node = null; break; }
      node = next;
    }
    if (node && node.isDir) return node;
  }
  // unknown folder: generate something plausible
  const r = seeded(hash(path));
  return buildNode(path.split('/').pop() || '/', path, Math.round((0.3 + r() * 6) * GB));
}

/** Prune a full node to maxDepth/topN like the real scanner returns it. */
function prune(node, depth, maxDepth, topN) {
  const out = { name: node.name, path: node.path, size: node.size, files: node.files, isDir: node.isDir, children: [] };
  if (node.isDir) out.dirs = (node.children || []).filter((c) => c.isDir).length;
  if (!node.isDir) return out;
  if (depth >= maxDepth) { if ((node.children || []).length) out.truncated = true; return out; }
  const kids = (node.children || []).filter((c) => !c.rest);
  const restNode = (node.children || []).find((c) => c.rest);
  const top = kids.slice(0, topN).map((c) => prune(c, depth + 1, maxDepth, topN));
  const other = kids.slice(topN);
  let otherSize = other.reduce((a, c) => a + c.size, 0) + (restNode?.size || 0);
  let otherFiles = other.reduce((a, c) => a + c.files, 0) + (restNode?.files || 0);
  out.children = top;
  if (otherSize > 0) out.children.push({ name: 'Overige', other: true, size: otherSize, files: otherFiles, count: other.length + (restNode?.files || 0), isDir: false, path: node.path });
  return out;
}

function allFiles(node, out = []) {
  if (!node.isDir) { if (!node.rest) out.push(node); return out; }
  for (const c of node.children || []) allFiles(c, out);
  return out;
}

function sampleDirs(node, out = [], depth = 0) {
  if (!node.isDir || depth > 5) return out;
  out.push(node.path);
  for (const c of node.children || []) sampleDirs(c, out, depth + 1);
  return out;
}

/* -------------------------------------------------------------- scanning */

const scanCache = new Map(); // path → tree
const jobs = new Map();      // jobId → {cancelled, timer}
const results = new Map();   // jobId → {state, progress, tree, path, error, errorCode} (storage.scanResult)
let jobSeq = 0;
function remember(jobId, r) {
  results.set(jobId, r);
  if (results.size > 24) results.delete(results.keys().next().value);
}

function startScan(m, { path, maxDepth = 4, topN = 40, refresh = false }) {
  const jobId = `scan-${++jobSeq}`;
  const node = findNode(path);
  const key = `${node.path}|${maxDepth}|${topN}`;
  const job = { cancelled: false, timer: 0 };
  jobs.set(jobId, job);
  if (!refresh && scanCache.has(key)) {
    const c = scanCache.get(key);
    remember(jobId, { state: 'done', progress: { bytes: c.tree.size, files: c.tree.files, current: node.path }, tree: c.tree, path: node.path });
    setTimeout(() => { if (!job.cancelled) m.emit('storage.scanDone', { jobId, tree: c.tree, path: node.path, bytes: c.tree.size, files: c.tree.files, tookMs: c.tookMs }); jobs.delete(jobId); }, 150);
    return { jobId, cached: true, tree: c.tree, scannedAt: c.at };
  }
  remember(jobId, { state: 'running', progress: { bytes: 0, files: 0, current: node.path }, path: node.path });
  const dirs = sampleDirs(node);
  const total = node.size, files = node.files;
  // bigger trees take a little longer (1.2 – 3.5 s)
  const duration = Math.min(3500, 1200 + Math.log2(1 + total / GB) * 260);
  const t0 = performance.now();
  const tick = () => {
    if (job.cancelled) return;
    const p = Math.min(1, (performance.now() - t0) / duration);
    const eased = 1 - (1 - p) ** 1.6;
    const progress = {
      bytes: Math.round(total * eased), files: Math.round(files * eased),
      current: dirs[Math.min(dirs.length - 1, Math.floor(eased * dirs.length))] || node.path,
    };
    if (results.has(jobId)) results.get(jobId).progress = progress;
    m.emit('storage.scanProgress', { jobId, ...progress });
    if (p >= 1) {
      const tree = prune(node, 0, maxDepth, topN);
      const tookMs = Math.round(performance.now() - t0);
      scanCache.set(key, { tree, at: Date.now(), tookMs });
      remember(jobId, { state: 'done', progress: { bytes: tree.size, files: tree.files, current: node.path }, tree, path: node.path });
      // like the host, a big tree may be left out of the event (?mockScanEventTree=0 tests the storage.scanResult path)
      const withTree = m.params?.get?.('mockScanEventTree') !== '0';
      m.emit('storage.scanDone', { jobId, tree: withTree ? tree : null, path: node.path, bytes: tree.size, files: tree.files, tookMs });
      jobs.delete(jobId);
      return;
    }
    job.timer = setTimeout(tick, 140);
  };
  job.timer = setTimeout(tick, 80);
  return { jobId };
}

/* ------------------------------------------------------------- categories */

const sizeAt = (p) => findNode(p).size;
function categoriesData() {
  const h = (p) => `${HOME}/${p}`;
  const cats = [
    { id: 'documents', name: 'Documenten', path: h('Documenten') },
    { id: 'pictures', name: 'Afbeeldingen', path: h('Afbeeldingen') },
    { id: 'videos', name: "Video's", path: h("Video's") },
    { id: 'music', name: 'Muziek', path: h('Muziek') },
    { id: 'downloads', name: 'Downloads', path: h('Downloads') },
    { id: 'desktop', name: 'Bureaublad', path: h('Bureaublad') },
    { id: 'trash', name: 'Prullenbak', path: h('.local/share/Trash'), size: cleanupState.trash ? sizeAt(h('.local/share/Trash')) : 0 },
    {
      id: 'browser', name: 'Browsers', path: h('.cache'), items: [
        { name: 'Brave', path: h('.config/BraveSoftware'), size: sizeAt(h('.config/BraveSoftware')) + sizeAt(h('.cache/BraveSoftware')) },
        { name: 'Firefox', path: h('snap/firefox'), size: sizeAt(h('snap/firefox')) + sizeAt(h('.cache/mozilla')) },
        { name: 'Google Chrome', path: h('.config/google-chrome'), size: sizeAt(h('.config/google-chrome')) },
      ],
    },
    {
      id: 'chat', name: 'Chat-apps', path: h('snap'), items: [
        { name: 'Telegram', path: h('snap/telegram-desktop'), size: sizeAt(h('snap/telegram-desktop')) },
        { name: 'Discord', path: h('.config/discord'), size: sizeAt(h('.config/discord')) + sizeAt(h('snap/discord')) },
        { name: 'Signal', path: h('.var/app/org.signal.Signal'), size: sizeAt(h('.var/app/org.signal.Signal')) },
      ],
    },
    {
      id: 'dev', name: 'Ontwikkeltools', path: h('projecten'), items: [
        { name: 'Rust (.rustup, .cargo)', path: h('.rustup'), size: sizeAt(h('.rustup')) + sizeAt(h('.cargo')) },
        { name: 'Bouwmappen (target)', path: h('projecten/rust-game/target'), size: sizeAt(h('projecten/rust-game/target')) },
        { name: 'node_modules', path: h('projecten'), size: sizeAt(h('projecten/website-oma/node_modules')) + sizeAt(h('projecten/win11shell/node_modules')) },
        { name: 'npm-cache', path: h('.npm'), size: sizeAt(h('.npm')) },
        { name: 'pip-cache', path: h('.cache/pip'), size: sizeAt(h('.cache/pip')) },
        { name: 'Go', path: h('go'), size: sizeAt(h('go')) + sizeAt(h('.cache/go-build')) },
      ],
    },
    { id: 'appdata', name: 'App-gegevens', path: h('.config'), size: sizeAt(h('.config')) - sizeAt(h('.config/BraveSoftware')) - sizeAt(h('.config/google-chrome')) - sizeAt(h('.config/discord')) + G(2.3) },
    { id: 'snaps', name: 'Snaps', path: h('snap'), size: sizeAt(h('snap')) - sizeAt(h('snap/telegram-desktop')) - sizeAt(h('snap/discord')) - sizeAt(h('snap/firefox')) },
    { id: 'flatpak', name: 'Flatpaks', path: h('.var/app'), size: sizeAt(h('.var/app')) - sizeAt(h('.var/app/org.signal.Signal')) + sizeAt(h('.local/share/flatpak')) },
    { id: 'cache', name: 'Cache', path: h('.cache'), size: sizeAt(h('.cache')) - sizeAt(h('.cache/BraveSoftware')) - sizeAt(h('.cache/mozilla')) - sizeAt(h('.cache/pip')) - sizeAt(h('.cache/go-build')) },
  ];
  for (const c of cats) if (c.size == null) c.size = c.items ? c.items.reduce((a, i) => a + i.size, 0) : sizeAt(c.path);
  const home = sizeAt(HOME);
  const known = cats.reduce((a, c) => a + c.size, 0);
  cats.push({ id: 'other', name: 'Overig', path: HOME, size: Math.max(0, home - known) });
  const other = cats.pop();
  cats.sort((a, b) => b.size - a.size);
  cats.push(other);
  return { home: HOME, total: home, files: findNode(HOME).files, partial: false, scannedAt: Date.now() - 4 * 60000, tookMs: 2100, categories: cats };
}

let categoriesReady = false;
let categoriesTimer = 0;

/* --------------------------------------------------------------- cleanup */

const cleanupState = { trash: true, thumbnails: true, 'user-cache-safe': true, 'browser-cache': true, 'apt-cache': true, 'old-snaps': true, journal: true, 'flatpak-unused': true, 'crash-reports': true };
const trashedDownloads = new Set();
const RUNNING_BROWSERS = new Set(['firefox']); // Firefox is open → its cache is skipped

function oldDownloadItems() {
  const dl = findNode(`${HOME}/Downloads`);
  return allFiles(dl)
    .filter((f) => !trashedDownloads.has(f.path) && /\.(iso|zip|deb)$/i.test(f.name))
    .map((f, i) => ({ path: f.path, name: f.name, size: f.size, isDir: false, modified: NOW - (120 + i * 37) * DAY, accessed: NOW - (100 + i * 41) * DAY }))
    .sort((a, b) => b.size - a.size);
}

function cleanupItems() {
  const userCaches = [
    { name: 'Spotify', path: `${HOME}/.cache/spotify`, size: G(1.9) },
    { name: 'Shadercache (NVIDIA, Mesa)', path: `${HOME}/.cache/nvidia`, size: M(800) },
    { name: 'Electron-apps', path: `${HOME}/.cache/electron`, size: M(240) },
  ];
  const browsers = [
    { browser: 'brave', name: 'Brave', path: `${HOME}/.cache/BraveSoftware`, size: G(1.8), running: RUNNING_BROWSERS.has('brave') },
    { browser: 'firefox', name: 'Firefox', path: `${HOME}/snap/firefox/common/.cache`, size: G(1.1), running: RUNNING_BROWSERS.has('firefox') },
    { browser: 'chrome', name: 'Google Chrome', path: `${HOME}/.cache/google-chrome`, size: M(200), running: false },
  ];
  const busy = browsers.filter((b) => b.running).map((b) => b.name);
  const old = oldDownloadItems();
  const snaps = [{ name: 'firefox', revision: '6966', size: M(980) }, { name: 'code', revision: '201', size: G(1.1) }, { name: 'gnome-42-2204', revision: '176', size: M(980) }, { name: 'core22', revision: '1663', size: M(310) }, { name: 'telegram-desktop', revision: '6420', size: M(420) }];
  const mk = (id, name, description, size, o = {}) => ({
    id, name, description, size, needsAdmin: !!o.admin, safe: o.safe !== false,
    defaultSelected: !!(o.default && (size == null || size > 0)), details: o.details || {}, available: o.available !== false,
  });
  const live = (id, size) => (cleanupState[id] ? size : 0);
  return [
    mk('trash', 'Prullenbak', 'Bestanden die je hebt verwijderd. Na het legen zijn ze echt weg.', live('trash', G(3.1)), { details: { count: cleanupState.trash ? 412 : 0 } }),
    mk('thumbnails', 'Miniaturen', "Kleine voorbeeldplaatjes van je foto's en video's. Die worden vanzelf opnieuw gemaakt.", live('thumbnails', G(0.9)), { default: true }),
    mk('user-cache-safe', 'Tijdelijke app-bestanden', 'Caches die apps zelf opnieuw aanmaken, zoals shader- en downloadcaches.', live('user-cache-safe', userCaches.reduce((a, c) => a + c.size, 0)), { default: true, details: { items: cleanupState['user-cache-safe'] ? userCaches : [] } }),
    mk('browser-cache', 'Tijdelijke internetbestanden', 'De cache van je browsers. Wachtwoorden, bladwijzers en geschiedenis blijven bewaard.' + (busy.length ? ` Sluit eerst ${busy.join(', ')}, anders wordt die overgeslagen.` : ''),
      live('browser-cache', browsers.filter((b) => !b.running).reduce((a, b) => a + b.size, 0)), { default: true, details: { items: browsers, running: busy, total: browsers.reduce((a, b) => a + b.size, 0) } }),
    mk('old-downloads', 'Oude downloads', 'Bestanden in Downloads die je al meer dan 90 dagen niet hebt geopend. Bekijk ze eerst: ze gaan naar de prullenbak.',
      old.reduce((a, f) => a + f.size, 0), { safe: false, details: { items: old, count: old.length, days: 90, toTrash: true } }),
    mk('apt-cache', 'Opgeslagen updatebestanden', 'Installatiebestanden van updates die al zijn geïnstalleerd. Vraagt om je wachtwoord.', live('apt-cache', M(910)), { admin: true }),
    mk('old-snaps', 'Oude versies van Snap-apps', 'Eerdere versies die Snap bewaart om terug te kunnen gaan. Vraagt om je wachtwoord.', live('old-snaps', snaps.reduce((a, x) => a + x.size, 0)), { admin: true, details: { items: cleanupState['old-snaps'] ? snaps : [] } }),
    mk('journal', 'Oude systeemlogboeken', 'Logboeken van meer dan 14 dagen oud. Alleen nodig om oude problemen te onderzoeken. Vraagt om je wachtwoord.', live('journal', G(1.6)), { admin: true, details: { total: G(2.2), keepDays: 14, estimate: true } }),
    mk('flatpak-unused', 'Ongebruikte Flatpak-onderdelen', 'Onderdelen die geen enkele Flatpak-app meer gebruikt.', cleanupState['flatpak-unused'] ? null : 0, { details: { scope: 'user' } }),
    mk('crash-reports', 'Foutrapporten', 'Rapporten van apps die zijn vastgelopen.', live('crash-reports', M(310)), { default: true, details: { count: cleanupState['crash-reports'] ? 6 : 0 } }),
  ];
}

/* -------------------------------------------------------------- methods */

const err = (code, message) => Object.assign(new Error(message), { code });

export default {
  methods: {
    'storage.volumes': async () => VOLUMES.map(withPercent),

    'storage.scan': async (args = {}, m) => {
      if (!args.path) throw err('bad_request', 'Geen map opgegeven.');
      return startScan(m, args);
    },

    'storage.scanCancel': async ({ jobId } = {}, m) => {
      const job = jobs.get(jobId);
      if (job) {
        job.cancelled = true;
        clearTimeout(job.timer);
        jobs.delete(jobId);
        remember(jobId, { ...(results.get(jobId) || {}), state: 'error', error: 'Het scannen is gestopt.', errorCode: 'cancelled' });
        m.emit('storage.scanDone', { jobId, tree: null, error: 'Het scannen is gestopt.', errorCode: 'cancelled' });
      }
      return {};
    },

    'storage.scanResult': async ({ jobId } = {}) => {
      const r = results.get(jobId);
      if (!r) throw err('not_found', 'Deze scan bestaat niet meer. Start het scannen opnieuw.');
      return { jobId, state: r.state, progress: r.progress, ...(r.tree ? { tree: r.tree } : {}), ...(r.error ? { error: r.error, errorCode: r.errorCode } : {}) };
    },

    'storage.trashSize': async () => {
      const it = cleanupItems().find((x) => x.id === 'trash');
      return { size: it?.size || 0, count: it?.details?.count || 0 };
    },

    'storage.categories': async ({ refresh = false } = {}, m) => {
      const had = categoriesReady;
      if (categoriesReady && !refresh) return { ready: true, data: categoriesData(), running: false };
      if (!categoriesTimer) {
        let n = 0;
        const total = findNode(HOME);
        const tick = () => {
          n++;
          m.emit('storage.categoriesProgress', { files: Math.round(total.files * n / 6), bytes: Math.round(total.size * n / 6) });
          if (n < 6) { categoriesTimer = setTimeout(tick, 220); return; }
          categoriesTimer = 0;
          categoriesReady = true;
          m.emit('storage.categoriesReady', { data: categoriesData() });
        };
        categoriesTimer = setTimeout(tick, 200);
      }
      return { ready: had && !refresh, data: had ? categoriesData() : null, running: true };
    },

    'storage.cleanup.list': async () => cleanupItems(),

    'storage.cleanup.run': async ({ ids = [], paths } = {}) => {
      await new Promise((r) => setTimeout(r, 1400));
      const items = cleanupItems();
      const sys = VOLUMES[0];
      const results = ids.map((id) => {
        const it = items.find((x) => x.id === id);
        if (!it) return { id, ok: false, freed: 0, error: 'Onbekend onderdeel.' };
        if (id === 'old-downloads') {
          const old = it.details.items;
          const chosen = Array.isArray(paths) ? old.filter((f) => paths.includes(f.path)) : old;
          chosen.forEach((f) => trashedDownloads.add(f.path));
          return { id, ok: true, freed: 0, movedToTrash: chosen.reduce((a, f) => a + f.size, 0), count: chosen.length, error: null };
        }
        const freed = it.size ?? M(640);
        if (id === 'browser-cache') {
          const skipped = it.details.items.filter((b) => b.running).map((b) => b.name);
          return Object.assign(done(id, freed), skipped.length ? { skipped } : {});
        }
        return done(id, freed);
      });
      function done(id, freed) {
        cleanupState[id] = false;
        sys.used = Math.max(0, sys.used - freed);
        sys.free = Math.max(0, sys.total - sys.used - (sys.reserve || 0));
        return { id, ok: true, freed };
      }
      return { results, freed: results.reduce((a, r) => a + (r.freed || 0), 0), dryRun: true };
    },

    'storage.largeFiles': async ({ path = HOME, limit = 50, minSize = 100 * MB } = {}) => {
      const node = findNode(path);
      const r = seeded(hash(path));
      return allFiles(node)
        .filter((f) => f.size >= minSize && !f.path.includes('/.local/share/Trash'))
        .sort((a, b) => b.size - a.size)
        .slice(0, limit)
        .map((f) => ({
          path: f.path, name: f.name, size: f.size, mime: mimeOf(f.name), icon: mimeOf(f.name).replace('/', '-'),
          modified: NOW - Math.floor((hash(f.path) % 900) * DAY * (0.5 + r() * 0.5)),
        }));
    },
  },
  start() {},
};
