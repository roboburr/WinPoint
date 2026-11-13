// Win11 Shell — mock index.* (Alles) for ?mock=1. Owned by the alles agent. SPEC §6.13.
//
// A realistic fake index of ~20 000 entries under /home/robin, /run/media/robin/22TB and
// /srv/vm, generated lazily on the first index.* call (other surfaces pay nothing), with
// the same query syntax as host/indexer.py (simplified): space = AND, | = OR, ! = NOT,
// "exact", < > grouping, * ?, ext: size: dm: folder: file: path: parent: in: regex: case:
// ww: empty: audio: zip: doc: exe: pic: video:.
//
// Test params:  ?mockIndex=building  (a refresh in progress)  ·  ?mockIndex=first  (first run, empty)
//               ?mockIndex=error     (indexer unavailable)

const HOME = '/home/robin';
const DAY = 86400000;
const NOW = Date.now();

/* ------------------------------------------------------------------ PRNG */
function mulberry32(a) {
  return function rnd() {
    a |= 0; a = (a + 0x6D2B79F5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/* ------------------------------------------------------------ categories */
const CAT_EXTS = {
  audio: 'aac aiff ape flac m4a mid midi mka mp2 mp3 ogg oga opus wav wma wv',
  compressed: '7z bz2 cab gz jar lz lzma rar tar tgz txz xz z zip zst deb rpm iso dmg apk',
  document: 'c cpp css csv doc docx epub h htm html ini java js json log md odp ods odt pdf php pot ppt pptx ps py rb rtf sql tex toml ts tsv txt xls xlsx xml yaml yml',
  executable: 'appimage bat cmd com desktop exe msi ps1 run sh',
  picture: 'avif bmp cr2 dng gif heic heif ico jpeg jpg jxl nef png psd raw svg tif tiff webp xcf',
  video: '3gp avi flv m2ts m4v mkv mov mp4 mpeg mpg mts ogv vob webm wmv',
};
const EXT_CAT = {};
for (const [c, s] of Object.entries(CAT_EXTS)) for (const e of s.split(' ')) if (!EXT_CAT[e]) EXT_CAT[e] = c;
EXT_CAT.ts = 'video';
const MIME = {
  jpg: 'image/jpeg', jpeg: 'image/jpeg', png: 'image/png', heic: 'image/heic', gif: 'image/gif', webp: 'image/webp',
  svg: 'image/svg+xml', dng: 'image/x-adobe-dng', mp3: 'audio/mpeg', flac: 'audio/flac', m4a: 'audio/mp4',
  ogg: 'audio/ogg', wav: 'audio/x-wav', opus: 'audio/ogg', mp4: 'video/mp4', mkv: 'video/x-matroska', mov: 'video/quicktime',
  avi: 'video/x-msvideo', webm: 'video/webm', srt: 'application/x-subrip', pdf: 'application/pdf',
  docx: 'application/vnd.openxmlformats-officedocument.wordprocessingml.document', doc: 'application/msword',
  xlsx: 'application/vnd.openxmlformats-officedocument.spreadsheetml.sheet', ods: 'application/vnd.oasis.opendocument.spreadsheet',
  odt: 'application/vnd.oasis.opendocument.text', pptx: 'application/vnd.openxmlformats-officedocument.presentationml.presentation',
  txt: 'text/plain', md: 'text/markdown', csv: 'text/csv', html: 'text/html', css: 'text/css', js: 'text/javascript',
  json: 'application/json', py: 'text/x-python', sh: 'application/x-shellscript', zip: 'application/zip',
  gz: 'application/gzip', '7z': 'application/x-7z-compressed', rar: 'application/vnd.rar', iso: 'application/x-cd-image',
  deb: 'application/vnd.debian.binary-package', appimage: 'application/vnd.appimage', desktop: 'application/x-desktop',
  qcow2: 'application/x-qemu-disk', vdi: 'application/x-virtualbox-vdi', xcf: 'image/x-xcf', epub: 'application/epub+zip',
  log: 'text/plain', ini: 'text/plain', torrent: 'application/x-bittorrent', ics: 'text/calendar', vcf: 'text/vcard',
  kdbx: 'application/x-keepass2', exe: 'application/x-ms-dos-executable', msi: 'application/x-msi',
};

/* ----------------------------------------------------------- fake index */
let INDEX = null;   // { entries, byPath, dirs }

function extOf(name, isDir) {
  if (isDir) return '';
  const i = name.lastIndexOf('.');
  return i > 0 && i < name.length - 1 ? name.slice(i + 1).toLowerCase() : '';
}

function buildIndex() {
  const rnd = mulberry32(20261025);
  const ri = (a, b) => a + Math.floor(rnd() * (b - a + 1));
  const pick = (arr) => arr[Math.floor(rnd() * arr.length)];
  const pad = (n, w = 2) => String(n).padStart(w, '0');
  const entries = [];
  const byPath = new Map();
  const ago = (maxDays, minDays = 0) => NOW - Math.floor((minDays + rnd() * (maxDays - minDays)) * DAY);

  function add(parentPath, name, { dir = false, size = 0, modified, hidden } = {}) {
    const path = parentPath === '/' ? '/' + name : `${parentPath}/${name}`;
    if (byPath.has(path)) return byPath.get(path);
    const parent = byPath.get(parentPath);
    const e = {
      name, dir: parentPath, path, isDir: dir, size: dir ? 0 : size,
      modified: modified ?? ago(900), ext: extOf(name, dir),
      hidden: hidden ?? (name.startsWith('.') || (parent ? parent.hidden : false)),
      kids: dir ? [] : null, parent: parent || null,
    };
    e.mime = dir ? 'inode/directory' : (MIME[e.ext] || (e.ext ? 'application/octet-stream' : 'application/x-executable'));
    e.cat = dir ? '' : (EXT_CAT[e.ext] || '');
    e.lname = name.toLowerCase();
    e.lpath = path.toLowerCase();
    entries.push(e);
    byPath.set(path, e);
    if (parent) parent.kids.push(e);
    return e;
  }
  function mkdirs(path, opts = {}) {
    const parts = path.split('/').filter(Boolean);
    let cur = '';
    for (const p of parts) {
      const parent = cur || '/';
      cur = `${cur}/${p}`;
      if (!byPath.has(cur)) add(parent, p, { dir: true, ...opts });
    }
    return cur;
  }
  const root = { name: '/', dir: '', path: '/', isDir: true, kids: [], hidden: false, size: 0, modified: NOW - 40 * DAY, ext: '', mime: 'inode/directory', cat: '', lname: '/', lpath: '/' };
  byPath.set('/', root);
  const KB = 1024, MB = 1024 * KB, GB = 1024 * MB;
  const file = (dir, name, size, modified) => add(mkdirs(dir), name, { size, modified });

  /* --- home: pictures ------------------------------------------------- */
  const pics = `${HOME}/Afbeeldingen`;
  const trips = [['Vakantie Texel 2024', '20240712', 2024, 6], ['Vakantie Italië 2023', '20230804', 2023, 7],
    ['Weekendje Ardennen', '20251017', 2025, 9], ['Kerst 2025', '20251224', 2025, 11], ['Verjaardag oma 80', '20250311', 2025, 2],
    ['Bruiloft Sanne en Mark', '20240607', 2024, 5], ['Zomer in de tuin', '20250621', 2025, 5]];
  for (const [folder, stamp, year, month] of trips) {
    const d = `${pics}/${folder}`;
    const n = ri(40, 120);
    const base = new Date(year, month, Number(stamp.slice(6, 8)), 9 + ri(0, 3), ri(0, 59)).getTime();
    for (let i = 0; i < n; i++) {
      const t = base + i * ri(2, 40) * 60000;
      const hh = pad(new Date(t).getHours()), mm = pad(new Date(t).getMinutes()), ss = pad(ri(0, 59));
      file(d, `IMG_${stamp}_${hh}${mm}${ss}.jpg`, ri(1800, 6200) * KB, t);
      if (rnd() < 0.08) file(d, `VID_${stamp}_${hh}${mm}${ss}.mp4`, ri(20, 400) * MB, t);
    }
  }
  for (let i = 0; i < 900; i++) {
    const t = ago(700);
    const dt = new Date(t);
    file(`${pics}/Foto's telefoon`, `PXL_${dt.getFullYear()}${pad(dt.getMonth() + 1)}${pad(dt.getDate())}_${pad(dt.getHours())}${pad(dt.getMinutes())}${pad(ri(0, 59))}${ri(100, 999)}.jpg`, ri(1500, 5200) * KB, t);
  }
  for (let i = 0; i < 140; i++) {
    const t = i < 4 ? ago(0.3) : ago(160);
    const dt = new Date(t);
    file(`${pics}/Schermafbeeldingen`, `Schermafbeelding ${dt.getFullYear()}-${pad(dt.getMonth() + 1)}-${pad(dt.getDate())} om ${pad(dt.getHours())}.${pad(dt.getMinutes())}.${pad(ri(0, 59))}.png`, ri(80, 2400) * KB, t);
  }
  file(`${pics}`, 'Fotoboek oma - omslag.png', 3 * MB, ago(200));
  file(`${pics}`, 'profielfoto.jpg', 420 * KB, ago(60));
  file(`${pics}`, 'Vakantiefoto Texel - strand.heic', 2400 * KB, ago(400, 380));
  file(`${pics}/Wallpapers`, 'Bergen bij zonsopkomst.jpg', 6 * MB, ago(300));
  file(`${pics}/Wallpapers`, 'Noorderlicht.webp', 2 * MB, ago(300));
  file(`${pics}/Wallpapers`, 'Abstract blauw.png', 4 * MB, ago(300));
  file(`${pics}/Bewerkt`, 'Familiefoto 2025 (bewerkt).xcf', 88 * MB, ago(30));
  file(`${pics}/Bewerkt`, 'Familiefoto 2025.jpg', 7 * MB, ago(30));
  mkdirs(`${pics}/Nog sorteren`);

  /* --- home: documents ------------------------------------------------- */
  const docs = `${HOME}/Documenten`;
  const months = ['januari', 'februari', 'maart', 'april', 'mei', 'juni', 'juli', 'augustus', 'september', 'oktober', 'november', 'december'];
  for (const y of [2023, 2024, 2025, 2026]) {
    file(`${docs}/Administratie/Belasting ${y}`, `Aangifte inkomstenbelasting ${y}.pdf`, ri(180, 600) * KB, Math.min(NOW - DAY, new Date(y + 1, 3, ri(1, 28), ri(9, 20), ri(0, 59)).getTime()));
    file(`${docs}/Administratie/Belasting ${y}`, `Jaaropgave ${y} ING.pdf`, ri(60, 200) * KB, Math.min(NOW - DAY, new Date(y + 1, 1, ri(1, 28), ri(9, 20), ri(0, 59)).getTime()));
    file(`${docs}/Administratie/Belasting ${y}`, `WOZ-beschikking ${y}.pdf`, ri(60, 200) * KB, new Date(y, 1, ri(1, 28), ri(9, 20), ri(0, 59)).getTime());
    for (let m = 0; m < 12; m++) {
      const t = new Date(y, m, ri(1, 28), ri(8, 21), ri(0, 59)).getTime();
      if (t > NOW) continue;
      for (const firm of ['Eneco', 'Ziggo', 'Vitens', 'KPN', 'Zilveren Kruis']) {
        if (rnd() < 0.55) file(`${docs}/Administratie/Facturen/${y}`, `Factuur ${firm} ${months[m]} ${y}.pdf`, ri(40, 300) * KB, t);
      }
      if (rnd() < 0.7) file(`${docs}/Administratie/Loonstroken/${y}`, `Loonstrook ${months[m]} ${y}.pdf`, ri(50, 120) * KB, t);
    }
  }
  const workDocs = ['Offerte keukenrenovatie.docx', 'Begroting 2026.xlsx', 'Presentatie kwartaalcijfers Q3.pptx',
    'Notulen teamoverleg.docx', 'Projectplan nieuwe website.docx', 'Urenregistratie oktober.xlsx', 'Contract dienstverlening.pdf',
    'Rapport klanttevredenheid 2025.pdf', 'Planning najaar.ods', 'Sollicitatiebrief.odt', 'CV Robin 2026.pdf', 'Referenties.docx'];
  for (const n of workDocs) file(`${docs}/Werk`, n, ri(30, 4000) * KB, ago(400));
  for (const n of ['Appeltaart van oma.docx', 'Pompoensoep.txt', 'Lasagne.pdf', 'Bananenbrood.md', 'Kerstmenu 2025.docx', 'Boodschappenlijst.txt'])
    file(`${docs}/Recepten`, n, ri(2, 900) * KB, ago(700));
  for (const n of ['Huurcontract.pdf', 'Polis inboedelverzekering.pdf', 'Garantiebewijs wasmachine.pdf', 'Paspoort scan.pdf', 'Rijbewijs scan.jpg', 'Diploma HBO.pdf'])
    file(`${docs}/Belangrijk`, n, ri(200, 2400) * KB, ago(1200));
  file(docs, 'Fotoboek oma 80 jaar.pdf', 48 * MB, ago(200));
  file(docs, 'Wachtwoorden.kdbx', 96 * KB, ago(10));
  file(docs, 'notities.txt', 4 * KB, ago(0.05));
  file(docs, 'Verlanglijstje Sinterklaas.docx', 24 * KB, ago(5));
  file(docs, 'Reisplanning Italië.xlsx', 60 * KB, ago(600));
  mkdirs(`${docs}/Lege map`);

  /* --- home: downloads -------------------------------------------------- */
  const dl = `${HOME}/Downloads`;
  const dls = [['ubuntu-26.04-desktop-amd64.iso', 6.1 * GB], ['google-chrome-stable_current_amd64.deb', 118 * MB],
    ['Obsidian-1.9.12.AppImage', 128 * MB], ['steam_latest.deb', 4 * MB], ['foto\'s bruiloft (Sanne).zip', 2.4 * GB],
    ['Handleiding wasmachine.pdf', 6 * MB], ['vakantiefoto-s-texel.zip', 640 * MB], ['Gemeente - parkeervergunning.pdf', 220 * KB],
    ['Belastingdienst - voorlopige aanslag.pdf', 180 * KB], ['setup-printer-driver.run', 42 * MB], ['concertkaartjes.pdf', 340 * KB],
    ['Spotify-installer.exe', 1.2 * MB], ['Recept appeltaart.pdf', 90 * KB], ['muziek-mix-2025.zip', 380 * MB],
    ['factuur_2026_10_ziggo.pdf', 120 * KB], ['Screenshot_20261024-213233.png', 900 * KB], ['bonnetje.jpg', 1.1 * MB],
    ['thunderbird-140.tar.xz', 96 * MB], ['font-inter.zip', 22 * MB], ['vergaderstukken.7z', 14 * MB]];
  dls.forEach(([n, s], i) => file(dl, n, Math.round(s), i < 3 ? ago(0.4) : ago(220)));
  for (let i = 0; i < 40; i++) file(dl, `document (${i + 1}).pdf`, ri(30, 3000) * KB, ago(400));

  /* --- home: music ------------------------------------------------------- */
  const artists = [['Doe Maar', ['Skunk', 'Lijf aan lijf']], ['Marco Borsato', ['Luid en duidelijk', 'Dromen zijn bedrog']],
    ['Racoon', ['Liverpool Rain', 'Before You Leave']], ['Golden Earring', ['Moontan', 'Cut']], ['Within Temptation', ['Mother Earth', 'Resist']],
    ['Davina Michelle', ['My Own Way']], ['Snelle', ['Vierentwintig']], ['Froukje', ['Uitzinnig']], ['Kensington', ['Control']],
    ['André Hazes', ['Zij gelooft in mij']], ['Anouk', ['Graduated Cum Laude', 'Urban Solitude']]];
  const songWords = ['Liefde', 'Regen', 'Zomer', 'Nacht', 'Thuis', 'Vrij', 'Dansen', 'Altijd', 'Morgen', 'Zee', 'Licht', 'Stad', 'Weg', 'Hart', 'Tijd'];
  for (const [artist, albums] of artists) {
    for (const album of albums) {
      const d = `${HOME}/Muziek/${artist}/${album}`;
      const n = ri(9, 14);
      const t = ago(1500, 200);
      for (let i = 1; i <= n; i++) {
        const title = `${pick(songWords)}${rnd() < 0.4 ? ' ' + pick(songWords).toLowerCase() : ''}`;
        const ext = rnd() < 0.25 ? 'flac' : 'mp3';
        file(d, `${pad(i)} - ${title}.${ext}`, (ext === 'flac' ? ri(22, 48) : ri(4, 11)) * MB, t);
      }
      file(d, 'cover.jpg', ri(200, 900) * KB, t);
    }
  }
  file(`${HOME}/Muziek/Afspeellijsten`, 'Favorieten.m3u', 4 * KB, ago(40));
  file(`${HOME}/Muziek/Podcasts`, 'De Dag - aflevering 812.mp3', 38 * MB, ago(8));
  file(`${HOME}/Muziek/Podcasts`, 'Zwarte Schapen - afl. 3.mp3', 52 * MB, ago(20));

  /* --- home: videos ------------------------------------------------------- */
  for (const [n, s] of [['Vakantiefilm Texel 2024.mp4', 3.2 * GB], ['Bruiloft Sanne en Mark - montage.mp4', 6.8 * GB],
    ['Eerste stapjes Noor.mov', 820 * MB], ['Kerstdiner 2025.mp4', 1.4 * GB], ['Schermopname 2026-10-12.webm', 260 * MB],
    ['Diavoorstelling oma 80.mp4', 980 * MB]]) file(`${HOME}/Video's`, n, Math.round(s), ago(500));

  /* --- home: desktop, projects, hidden ------------------------------------- */
  for (const n of ['Firefox.desktop', 'Notities.txt', 'Te doen.md', 'Spelregels kwartet.pdf']) file(`${HOME}/Bureaublad`, n, ri(1, 400) * KB, ago(50));
  const projects = ['fotoalbum-site', 'budget-app', 'recepten'];
  for (const p of projects) {
    const d = `${HOME}/Projecten/${p}`;
    for (const n of ['README.md', 'package.json', 'index.html', 'style.css', 'LICENSE']) file(d, n, ri(1, 40) * KB, ago(300));
    for (const n of ['main.js', 'app.js', 'utils.js', 'gallery.js', 'api.js', 'router.js']) file(`${d}/src`, n, ri(2, 60) * KB, ago(300));
    for (let i = 0; i < 700; i++) file(`${d}/node_modules/${pick(['react', 'lodash', 'vite', 'esbuild', 'chalk', 'debug', 'ms', 'semver'])}/${pick(['lib', 'dist', 'src'])}`, `${pick(['index', 'utils', 'core', 'helpers', 'parse', 'format'])}${i}.js`, ri(1, 90) * KB, ago(400));
    file(`${d}/.git`, 'config', 1 * KB, ago(300));
    file(`${d}/.git`, 'HEAD', 1 * KB, ago(3));
  }
  for (let i = 0; i < 2200; i++) file(`${HOME}/.cache/mozilla/firefox/cache2/entries`, Math.floor(rnd() * 1e16).toString(16).toUpperCase(), ri(1, 800) * KB, ago(30));
  for (const n of ['settings.json', 'monitors.xml', 'user-dirs.dirs', 'mimeapps.list']) file(`${HOME}/.config`, n, ri(1, 30) * KB, ago(90));
  file(`${HOME}/.config/win11shell`, 'settings.json', 6 * KB, ago(0.1));
  file(HOME, '.bashrc', 4 * KB, ago(400));
  file(HOME, '.profile', 1 * KB, ago(900));
  file(`${HOME}/.local/share/Trash/files`, 'oude-notities.txt', 2 * KB, ago(12));

  /* --- 22TB media drive ----------------------------------------------------- */
  const big = '/run/media/robin/22TB';
  const films = ['De Tweeling (2002)', 'Zwartboek (2006)', 'Soldaat van Oranje (1977)', 'Alles is liefde (2007)', 'Turks fruit (1973)',
    'Borgman (2013)', 'Hoe duur was de suiker (2013)', 'Oorlogswinter (2008)', 'Bankier van het verzet (2018)', 'De Oost (2020)',
    'Interstellar (2014)', 'Inception (2010)', 'Dune (2021)', 'Dune Part Two (2024)', 'Oppenheimer (2023)', 'The Martian (2015)',
    'Arrival (2016)', 'Blade Runner 2049 (2017)', 'Spirited Away (2001)', 'Coco (2017)'];
  for (const f of films) {
    const d = `${big}/Films/${f}`;
    const t = ago(2000, 100);
    file(d, `${f}.mkv`, ri(4, 38) * GB, t);
    file(d, `${f}.nl.srt`, ri(60, 140) * KB, t);
    if (rnd() < 0.5) file(d, 'poster.jpg', ri(200, 900) * KB, t);
  }
  for (const [show, seasons] of [['Baantjer', 4], ['Flikken Maastricht', 3], ['Penoza', 2], ['Undercover', 3], ['The Last of Us', 2], ['Planet Earth', 1]]) {
    for (let s = 1; s <= seasons; s++) {
      const n = ri(6, 12);
      for (let e = 1; e <= n; e++) file(`${big}/Series/${show}/Seizoen ${s}`, `${show} S${pad(s)}E${pad(e)}.mkv`, ri(800, 4200) * MB, ago(1800, 60));
    }
  }
  for (let y = 2008; y <= 2022; y++) {
    for (let m = 1; m <= 12; m++) {
      if (rnd() < 0.35) continue;
      const d = `${big}/Foto-archief/${y}/${pad(m)} ${months[m - 1]}`;
      const n = ri(30, 110);
      const t0 = new Date(y, m - 1, ri(1, 25), 10).getTime();
      for (let i = 0; i < n; i++) file(d, `DSC_${pad(ri(1, 9999), 4)}.JPG`, ri(3, 14) * MB, t0 + i * 3600000);
      if (rnd() < 0.15) file(d, `foto's ${months[m - 1]} ${y} (selectie).zip`, ri(80, 900) * MB, t0);
    }
  }
  for (const n of ['Volledige backup laptop 2025-01.tar.gz', 'Backup telefoon Pixel 8.zip', 'Thunderbird profiel.zip', 'Fotos oude computer.7z'])
    file(`${big}/Backups`, n, ri(8, 900) * GB, ago(700));
  file(`${big}/Muziekarchief`, 'Top 2000 - 2025 (compleet).zip', 32 * GB, ago(300));
  add(mkdirs(`${big}/$RECYCLE.BIN`), 'desktop.ini', { size: 129, hidden: true });
  add(mkdirs(`${big}/System Volume Information`), 'IndexerVolumeGuid', { size: 76, hidden: true });

  /* --- virtual machines ------------------------------------------------------ */
  const vm = '/srv/vm';
  for (const [n, s] of [['ubuntu-24.04.qcow2', 64 * GB], ['windows11.qcow2', 120 * GB], ['debian-test.qcow2', 18 * GB], ['homeassistant.qcow2', 32 * GB]])
    file(`${vm}/images`, n, s, ago(30));
  for (const [n, s] of [['ubuntu-26.04-desktop-amd64.iso', 6.1 * GB], ['Win11_24H2_Dutch_x64.iso', 5.8 * GB], ['debian-13.1.0-amd64-netinst.iso', 760 * MB]])
    file(`${vm}/iso`, n, Math.round(s), ago(200));
  for (const n of ['ubuntu-24.04.xml', 'windows11.xml', 'debian-test.xml', 'homeassistant.xml']) file(`${vm}/config`, n, ri(4, 12) * KB, ago(30));

  /* --- system (a sample of /usr so plain searches look like a real disk) ----- */
  const sysFiles = ['python3.14', 'bash', 'ls', 'cp', 'firefox', 'nautilus', 'gnome-shell', 'hyprctl', 'wpctl', 'nmcli', 'grim', 'slurp'];
  for (const n of sysFiles) {
    const e = file('/usr/bin', n, ri(40, 9000) * KB, ago(60));
    e.mime = 'application/x-executable';
    e.cat = 'executable';
  }
  for (let i = 0; i < 1400; i++) file(`/usr/share/icons/Win11-dark/${pick(['apps', 'mimes', 'places', 'status', 'actions'])}/${pick(['16', '24', '32', '48', 'scalable'])}`, `${pick(['folder', 'text', 'image', 'audio', 'video', 'package', 'user', 'network', 'battery', 'firefox', 'document'])}-${pick(['x-generic', 'open', 'symbolic', 'new', 'full'])}${i}.svg`, ri(1, 30) * KB, ago(120));
  for (let i = 0; i < 1500; i++) file(`/usr/share/doc/${pick(['bash', 'coreutils', 'python3', 'gnome-shell', 'pipewire', 'hyprland', 'mesa', 'libc6', 'grep'])}`, `${pick(['changelog', 'README', 'copyright', 'NEWS', 'examples'])}${i % 7 ? '' : '.gz'}${i}`, ri(1, 200) * KB, ago(300));

  // folder sizes = total of their contents; folder dates = newest item
  const all = [root, ...entries];
  for (let i = all.length - 1; i >= 0; i--) {
    const e = all[i];
    if (e.isDir) {
      let s = 0, m = 0;
      for (const k of e.kids) { s += k.size; m = Math.max(m, k.modified); }
      e.size = s;
      if (m) e.modified = Math.max(e.modified ?? 0, m);
    }
  }
  return { entries, byPath, root };
}

function idx() {
  if (!INDEX) {
    INDEX = buildIndex();
    // ?mockScale=N: N copies of the tree under extra folders (to try 100k+ results)
    const scale = Math.min(50, Math.max(1, parseInt(new URLSearchParams(location.search).get('mockScale') || '1', 10) || 1));
    if (scale > 1) {
      const base = INDEX.entries.slice();
      for (let k = 2; k <= scale; k++) {
        for (const e of base) {
          const path = `/run/media/robin/3TB/Archief ${k}${e.path}`;
          const dir = `/run/media/robin/3TB/Archief ${k}${e.dir}`;
          const c = { ...e, path, dir, lpath: path.toLowerCase(), kids: e.kids ? [] : null };
          INDEX.entries.push(c);
        }
      }
    }
  }
  return INDEX;
}

/* --------------------------------------------------------- query language */
const MOD = {
  ext: 'ext', extensie: 'ext', size: 'size', grootte: 'size', dm: 'dm', datemodified: 'dm', gewijzigd: 'dm', datum: 'dm',
  folder: 'folder', map: 'folder', dir: 'folder', mappen: 'folder', file: 'file', bestand: 'file', bestanden: 'file',
  path: 'path', pad: 'path', parent: 'parent', infolder: 'parent', inmap: 'parent', in: 'in', under: 'in', onder: 'in',
  regex: 'regex', case: 'case', nocase: 'nocase', ww: 'ww', wholeword: 'ww', heelwoord: 'ww', empty: 'empty', leeg: 'empty',
  audio: 'cat:audio', muziek: 'cat:audio', music: 'cat:audio', zip: 'cat:compressed', compressed: 'cat:compressed', archief: 'cat:compressed',
  doc: 'cat:document', document: 'cat:document', documenten: 'cat:document', exe: 'cat:executable', executable: 'cat:executable',
  programma: 'cat:executable', pic: 'cat:picture', picture: 'cat:picture', image: 'cat:picture', afbeelding: 'cat:picture',
  afbeeldingen: 'cat:picture', video: 'cat:video', videos: 'cat:video',
};
const TERM_MODS = new Set(['folder', 'file', 'path', 'regex', 'case', 'nocase', 'ww']);

function qErr(message, code = 'bad_query') { return Object.assign(new Error(message), { code }); }

const UNITS = { '': 1, b: 1, k: 1024, kb: 1024, m: 1048576, mb: 1048576, g: 1073741824, gb: 1073741824, t: 1099511627776, tb: 1099511627776 };
const SIZE_WORDS = { empty: [0, 0], leeg: [0, 0], tiny: [1, 10240], klein: [10241, 102400], small: [10241, 102400],
  medium: [102401, 1048576], large: [1048577, 16777216], groot: [1048577, 16777216], huge: [16777217, 134217728],
  enorm: [16777217, 134217728], gigantic: [134217729, Infinity], gigantisch: [134217729, Infinity] };
function sizeVal(s) {
  const m = /^\s*(\d+(?:[.,]\d+)?)\s*([a-z]*)\s*$/.exec(s.toLowerCase());
  if (!m || !(m[2] in UNITS)) throw qErr(`Ik begrijp de grootte “${s}” niet. Probeer bijvoorbeeld size:>10mb.`);
  return Math.round(parseFloat(m[1].replace(',', '.')) * UNITS[m[2]]);
}
function parseSize(v) {
  v = (v || '').trim().toLowerCase();
  if (!v) throw qErr('Geef een grootte op, bijvoorbeeld size:>10mb.');
  if (SIZE_WORDS[v]) return SIZE_WORDS[v];
  for (const sep of ['..', '-']) if (v.includes(sep) && !v.startsWith(sep)) { const [a, b] = v.split(sep); return [sizeVal(a), sizeVal(b)]; }
  for (const op of ['>=', '<=', '>', '<', '=']) {
    if (v.startsWith(op)) {
      const n = sizeVal(v.slice(op.length));
      return { '>=': [n, Infinity], '<=': [-Infinity, n], '>': [n + 1, Infinity], '<': [-Infinity, n - 1], '=': [n, n] }[op];
    }
  }
  const n = sizeVal(v);
  return [n, n];
}
function dayStart(d, add = 0) { return new Date(d.getFullYear(), d.getMonth(), d.getDate() + add).getTime(); }
function dateWord(w) {
  const now = new Date();
  const wd = (now.getDay() + 6) % 7;
  const map = {
    today: [dayStart(now), dayStart(now, 1)], vandaag: [dayStart(now), dayStart(now, 1)],
    yesterday: [dayStart(now, -1), dayStart(now)], gisteren: [dayStart(now, -1), dayStart(now)],
    thisweek: [dayStart(now, -wd), dayStart(now, 1)], dezeweek: [dayStart(now, -wd), dayStart(now, 1)],
    lastweek: [dayStart(now, -wd - 7), dayStart(now, -wd)], vorigeweek: [dayStart(now, -wd - 7), dayStart(now, -wd)],
    thismonth: [new Date(now.getFullYear(), now.getMonth(), 1).getTime(), dayStart(now, 1)],
    dezemaand: [new Date(now.getFullYear(), now.getMonth(), 1).getTime(), dayStart(now, 1)],
    lastmonth: [new Date(now.getFullYear(), now.getMonth() - 1, 1).getTime(), new Date(now.getFullYear(), now.getMonth(), 1).getTime()],
    vorigemaand: [new Date(now.getFullYear(), now.getMonth() - 1, 1).getTime(), new Date(now.getFullYear(), now.getMonth(), 1).getTime()],
    thisyear: [new Date(now.getFullYear(), 0, 1).getTime(), dayStart(now, 1)], ditjaar: [new Date(now.getFullYear(), 0, 1).getTime(), dayStart(now, 1)],
    lastyear: [new Date(now.getFullYear() - 1, 0, 1).getTime(), new Date(now.getFullYear(), 0, 1).getTime()],
    vorigjaar: [new Date(now.getFullYear() - 1, 0, 1).getTime(), new Date(now.getFullYear(), 0, 1).getTime()],
  };
  if (map[w]) return map[w];
  const m = /^(?:last|afgelopen)?(\d+)(days?|dagen|weeks?|weken|months?|maanden|years?|jaar)$/.exec(w);
  if (m) {
    const n = +m[1];
    const unit = m[2].startsWith('d') ? 1 : m[2].startsWith('w') ? 7 : m[2].startsWith('m') ? 30.44 : 365.25;
    return [m[2].startsWith('d') ? dayStart(now, -(n - 1)) : Date.now() - n * unit * DAY, dayStart(now, 1)];
  }
  return null;
}
function datePoint(s) {
  s = s.trim();
  let m;
  if ((m = /^(\d{4})$/.exec(s))) return [new Date(+m[1], 0, 1).getTime(), new Date(+m[1] + 1, 0, 1).getTime()];
  if ((m = /^(\d{4})[-/](\d{1,2})$/.exec(s))) return [new Date(+m[1], m[2] - 1, 1).getTime(), new Date(+m[1], +m[2], 1).getTime()];
  let y, mo, d;
  if ((m = /^(\d{4})[-/](\d{1,2})[-/](\d{1,2})$/.exec(s))) [y, mo, d] = [+m[1], +m[2], +m[3]];
  else if ((m = /^(\d{1,2})[-/.](\d{1,2})[-/.](\d{4})$/.exec(s))) [d, mo, y] = [+m[1], +m[2], +m[3]];
  else {
    const w = dateWord(s.toLowerCase());
    if (w) return w;
    throw qErr(`Ik begrijp de datum “${s}” niet. Probeer bijvoorbeeld dm:vandaag of dm:2026-10-01..2026-10-25.`);
  }
  return [new Date(y, mo - 1, d).getTime(), new Date(y, mo - 1, d + 1).getTime()];
}
function parseDate(v) {
  v = (v || '').trim().toLowerCase();
  if (!v) throw qErr('Geef een datum op, bijvoorbeeld dm:vandaag.');
  const w = dateWord(v);
  if (w) return w;
  if (v.includes('..')) { const [a, b] = v.split('..'); return [a ? datePoint(a)[0] : -Infinity, b ? datePoint(b)[1] : Infinity]; }
  for (const op of ['>=', '<=', '>', '<', '=']) {
    if (v.startsWith(op)) {
      const [a, b] = datePoint(v.slice(op.length));
      return { '>=': [a, Infinity], '>': [b, Infinity], '<=': [-Infinity, b], '<': [-Infinity, a], '=': [a, b] }[op];
    }
  }
  return datePoint(v);
}

const reEsc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

class Parser {
  constructor(s, o) { this.s = s; this.i = 0; this.depth = 0; this.o = o; }
  ws() { while (this.i < this.s.length && /\s/.test(this.s[this.i])) this.i++; }
  parse() { return { and: this.parseAnd() }; }
  parseAnd() {
    const items = [];
    for (;;) {
      this.ws();
      if (this.i >= this.s.length) break;
      const c = this.s[this.i];
      if (c === '>' && this.depth > 0) break;
      if (c === '|') { this.i++; continue; }
      const it = this.parseOr();
      if (it) items.push(it);
    }
    return items;
  }
  parseOr() {
    const alts = [this.parseUnary()];
    for (;;) {
      let j = this.i;
      while (j < this.s.length && /\s/.test(this.s[j])) j++;
      if (this.s[j] === '|') {
        this.i = j + 1; this.ws();
        if (this.i >= this.s.length || (this.s[this.i] === '>' && this.depth)) break;
        alts.push(this.parseUnary());
        continue;
      }
      break;
    }
    const a = alts.filter(Boolean);
    return a.length ? (a.length === 1 ? a[0] : { or: a }) : null;
  }
  parseUnary() {
    const c = this.s[this.i];
    if (c === undefined) return null;
    if (c === '!') { this.i++; const inner = this.parseUnary(); return inner ? { not: inner } : null; }
    if (c === '<') {
      this.i++; this.depth++;
      const items = this.parseAnd();
      if (this.s[this.i] === '>') this.i++;
      this.depth--;
      return items.length ? (items.length === 1 ? items[0] : { and: items }) : null;
    }
    return this.parseWord();
  }
  parseWord() {
    const start = this.i;
    let buf = '', inq = false, quoted = false;
    while (this.i < this.s.length) {
      const c = this.s[this.i];
      if (c === '"') { inq = !inq; quoted = true; this.i++; continue; }
      if (!inq && (/\s/.test(c) || c === '|' || (c === '>' && this.depth > 0))) break;
      buf += c; this.i++;
    }
    const raw = this.s.slice(start, this.i);
    if (!raw) { this.i++; return null; }
    return this.make(raw, buf, quoted, { ...this.o, regex: false });
  }
  make(raw, text, quoted, o) {
    const m = /^([A-Za-z]+):/.exec(raw);
    if (m && MOD[m[1].toLowerCase()]) {
      const kind = MOD[m[1].toLowerCase()];
      const plen = m[1].length + 1;
      const vraw = raw.slice(plen);
      const vtext = text.slice(plen);
      if (TERM_MODS.has(kind) || kind.startsWith('cat:')) {
        const o2 = { ...o };
        if (kind === 'case') o2.case = true;
        if (kind === 'nocase') o2.case = false;
        if (kind === 'path') o2.path = true;
        if (kind === 'ww') o2.whole = true;
        if (kind === 'regex') o2.regex = true;
        let inner = null;
        if (vraw) inner = o2.regex ? { term: vtext, ...o2 } : this.make(vraw, vtext, vraw.includes('"'), o2);
        if (kind === 'folder' || kind === 'file' || kind.startsWith('cat:')) {
          const mod = { mod: kind };
          return inner ? { and: [mod, inner] } : mod;
        }
        return inner;
      }
      return { mod: kind, value: vtext };
    }
    if (!text) return null;
    return { term: text, ...o };
  }
}

function compileNode(n, hl) {
  if (!n) return () => true;
  if (n.and) {
    const fs = n.and.map((x) => compileNode(x, hl));
    return (e) => fs.every((f) => f(e));
  }
  if (n.or) {
    const fs = n.or.map((x) => compileNode(x, hl));
    return (e) => fs.some((f) => f(e));
  }
  if (n.not) {
    const f = compileNode(n.not, []);
    return (e) => !f(e);
  }
  if (n.mod) return compileMod(n.mod, n.value);
  return compileTerm(n, hl);
}

function compileTerm(t, hl) {
  const text = t.term;
  const hay = (e) => (t.path ? (t.case ? e.path : e.lpath) : (t.case ? e.name : e.lname));
  if (t.regex) {
    let rx;
    try { rx = new RegExp(text, t.case ? '' : 'i'); } catch (err) { throw qErr(`Deze reguliere expressie klopt niet: ${err.message}.`, 'bad_regex'); }
    return (e) => rx.test(t.path ? e.path : e.name);
  }
  if (/[*?]/.test(text)) {
    if (!text.replace(/\*/g, '')) return () => true;
    const src = '^' + text.split('').map((c) => (c === '*' ? '.*' : c === '?' ? '.' : reEsc(c))).join('') + '$';
    const rx = new RegExp(src, t.case ? 's' : 'si');
    for (const p of text.split(/[*?]/)) if (p && !p.includes('/')) hl.push(p);
    return (e) => rx.test(t.path ? e.path : e.name);
  }
  const lit = t.case ? text : text.toLowerCase();
  if (!lit.includes('/')) hl.push(text);
  if (t.whole) {
    const rx = new RegExp(`(?<![\\p{L}\\p{N}_])${reEsc(text)}(?![\\p{L}\\p{N}_])`, t.case ? 'u' : 'iu');
    return (e) => rx.test(t.path ? e.path : e.name);
  }
  return (e) => hay(e).includes(lit);
}

function compileMod(kind, v) {
  if (kind === 'folder') return (e) => e.isDir;
  if (kind === 'file') return (e) => !e.isDir;
  if (kind.startsWith('cat:')) { const c = kind.slice(4); return (e) => e.cat === c; }
  if (kind === 'ext') {
    const exts = (v || '').split(/[;,|]/).map((x) => x.trim().toLowerCase().replace(/^\*?\.?/, '')).filter(Boolean);
    if (!exts.length) throw qErr('Geef een extensie op, bijvoorbeeld ext:jpg of ext:mp3;flac.');
    return (e) => exts.some((x) => e.lname.endsWith('.' + x));
  }
  if (kind === 'size') { const [lo, hi] = parseSize(v); return (e) => e.size >= lo && e.size <= hi; }
  if (kind === 'dm') { const [lo, hi] = parseDate(v); return (e) => e.modified >= lo && e.modified < hi; }
  if (kind === 'parent' || kind === 'in') {
    let p = (v || '').trim().replace(/^~(?=\/|$)/, HOME).replace(/\/+$/, '') || '/';
    if (!v) throw qErr('Geef een map op, bijvoorbeeld parent:/home/robin/Downloads.');
    if (kind === 'parent') return (e) => e.dir === p;
    const pre = p === '/' ? '/' : p + '/';
    return (e) => e.path.startsWith(pre);
  }
  if (kind === 'empty') return (e) => e.isDir && e.kids.length === 0;
  throw qErr(`Onbekende zoekfunctie “${kind}:”.`);
}

function compileQuery(query, { matchCase, matchPath, regex, wholeWord, filter } = {}) {
  const hl = [];
  let test;
  const q = String(query || '').trim();
  if (regex && q) {
    let rx;
    try { rx = new RegExp(q, matchCase ? '' : 'i'); } catch (err) { throw qErr(`Deze reguliere expressie klopt niet: ${err.message}.`, 'bad_regex'); }
    test = (e) => rx.test(matchPath ? e.path : e.name);
  } else {
    const node = new Parser(q, { case: !!matchCase, path: !!matchPath, whole: !!wholeWord }).parse();
    test = compileNode(node, hl);
  }
  if (filter && filter !== 'all') {
    const f = filter === 'folder' ? (e) => e.isDir : (e) => e.cat === filter;
    const t0 = test;
    test = (e) => f(e) && t0(e);
  }
  return { test, highlight: [...new Set(hl)].sort((a, b) => b.length - a.length), regex: regex ? q : null };
}

/* ---------------------------------------------------------------- sorting */
const collator = new Intl.Collator('nl', { sensitivity: 'base', numeric: false });
function sorter(sort) {
  const byName = (a, b) => collator.compare(a.name, b.name) || (a.path < b.path ? -1 : 1);
  switch (sort) {
    case 'size': return (a, b) => a.size - b.size || byName(a, b);
    case 'modified': return (a, b) => a.modified - b.modified || byName(a, b);
    case 'path': return (a, b) => collator.compare(a.dir, b.dir) || byName(a, b);
    case 'type': return (a, b) => collator.compare(a.isDir ? '' : a.ext || '~', b.isDir ? '' : b.ext || '~') || byName(a, b);
    default: return byName;
  }
}

/* ------------------------------------------------------------------ state */
const mode = (() => { try { return new URLSearchParams(location.search).get('mockIndex') || ''; } catch { return ''; } })();
const status = {
  state: mode === 'first' || mode === 'building' ? 'building' : (mode === 'error' ? 'error' : 'ready'),
  progress: null, builtAt: NOW - 18 * 60000, watching: 31842, liveUpdate: true, refreshing: false,
  generation: 1, overlay: 0, excluded: [],
};
let firstRun = mode === 'first';
let buildTimer = 0;
let cache = { key: '', gen: -1, list: null };
const extraFiles = [];

function rootsInfo() {
  const roots = [
    ['/', 'ext4', 0.32], ['/srv/vm', 'ext4', 0.02], ['/run/media/robin/22TB', 'fuseblk', 0.38],
    ['/run/media/robin/3TB', 'fuseblk', 0.18], ['/run/media/robin/BACKUP', 'fuseblk', 0.04],
    ['/run/media/robin/E2CEC932CEC8FFB1', 'fuseblk', 0.05], ['/run/media/robin/extra', 'exfat', 0.01]];
  const total = firstRun ? 0 : idx().entries.length;
  return roots.map(([path, fsType, share]) => {
    const n = Math.round(total * share);
    return { path, fsType, files: Math.round(n * 0.92), folders: Math.round(n * 0.08), state: status.state === 'building' ? 'building' : 'ready', builtAt: status.builtAt };
  });
}

function counts() {
  if (firstRun) return { files: 0, folders: 0 };
  const e = idx().entries;
  let f = 0, d = 0;
  for (const x of e) { if (x.isDir) d++; else f++; }
  return { files: f, folders: d };
}

function statusPayload() {
  return { ...status, ...counts(), roots: rootsInfo() };
}

function startBuild(m, seconds = 9) {
  clearInterval(buildTimer);
  status.state = 'building';
  const total = 1_284_311;
  const t0 = Date.now();
  const dirs = ['/home/robin/Afbeeldingen/Vakantie Texel 2024', '/home/robin/Documenten/Administratie', '/home/robin/.cache/mozilla/firefox',
    '/run/media/robin/22TB/Films', '/run/media/robin/22TB/Foto-archief/2015', '/srv/vm/images', '/usr/share/icons/Win11-dark', '/run/media/robin/3TB/Backups'];
  const tick = () => {
    const p = Math.min(1, (Date.now() - t0) / (seconds * 1000));
    status.progress = { done: Math.round(total * p), total, percent: Math.min(99, Math.round(p * 100)), current: dirs[Math.min(dirs.length - 1, Math.floor(p * dirs.length))] };
    if (p >= 1) {
      clearInterval(buildTimer);
      status.state = 'ready';
      status.progress = null;
      status.builtAt = Date.now();
      status.generation++;
      firstRun = false;
      m.emit('index.changed', { generation: status.generation });
    }
    m.emit('index.status', statusPayload());
  };
  tick();
  buildTimer = setInterval(tick, 500);
}

function itemOf(e) {
  return { path: e.path, name: e.name, dir: e.dir, isDir: e.isDir, size: e.size, modified: e.modified, ext: e.ext, mime: e.mime, hidden: e.hidden, isSymlink: false };
}

export default {
  methods: {
    'index.status': async () => statusPayload(),

    'index.search': async (args = {}, m) => {
      if (mode === 'error') throw Object.assign(new Error('Alles is op dit moment niet beschikbaar.'), { code: 'index_unavailable' });
      const t0 = performance.now();
      const { query = '', offset = 0, limit = 200, sort = 'name', desc = false, filter = 'all' } = args;
      const showHidden = args.showHidden ?? !!m.settings()?.alles?.showHidden;
      if (firstRun) return { total: 0, items: [], tookMs: 0.2, offset, state: status.state, highlight: { terms: [], regex: null, case: false } };
      const key = JSON.stringify([query, filter, args.matchCase, args.matchPath, args.regex, args.wholeWord, showHidden, sort, desc]);
      let list;
      let hl;
      if (cache.key === key && cache.gen === status.generation) { list = cache.list; hl = cache.hl; } else {
        const c = compileQuery(query, args);
        hl = c;
        const all = idx().entries;
        list = [];
        for (const e of all) if ((showHidden || !e.hidden) && c.test(e)) list.push(e);
        const cmp = sorter(sort);
        list.sort(desc ? (a, b) => cmp(b, a) : cmp);
        cache = { key, gen: status.generation, list, hl };
      }
      const items = list.slice(offset, offset + Math.max(0, Math.min(1000, limit))).map(itemOf);
      return { total: list.length, items, offset, tookMs: Math.round((performance.now() - t0) * 10) / 10 + 3.1, state: status.state, generation: status.generation, highlight: { terms: hl.highlight, regex: hl.regex, case: !!args.matchCase } };
    },

    'index.rebuild': async (args, m) => { startBuild(m, 8); return { ok: true }; },

    'index.setRoots': async ({ roots = 'auto', excluded } = {}, m) => {
      if (roots !== 'auto' && (!Array.isArray(roots) || !roots.length)) throw Object.assign(new Error('Kies minstens één locatie om te doorzoeken.'), { code: 'bad_args' });
      try { await m.call('settings.set', { path: 'alles.roots', value: roots }); } catch { /* ignore */ }
      if (Array.isArray(excluded)) { try { await m.call('settings.set', { path: 'alles.excluded', value: excluded }); } catch { /* ignore */ } status.excluded = excluded; }
      startBuild(m, 4);
      return { ok: true };
    },

    'index.locations': async (args, m) => {
      const s = m.settings()?.alles || {};
      const roots = Array.isArray(s.roots) ? s.roots : null;
      const GB = 1073741824;
      const list = [
        { path: '/', label: 'Systeemschijf', fsType: 'ext4', total: 232.6 * GB, free: 10.7 * GB, auto: true, kind: 'drive', removable: false },
        { path: '/run/media/robin/E2CEC932CEC8FFB1', label: '563 GB-schijf', fsType: 'fuseblk', total: 563.3 * GB, free: 265 * GB, auto: true, kind: 'drive', removable: false },
        { path: '/srv/vm', label: 'Virtuele machines', fsType: 'ext4', total: 383.3 * GB, free: 18.9 * GB, auto: true, kind: 'drive', removable: false },
        { path: '/run/media/robin/BACKUP', label: 'BACKUP', fsType: 'fuseblk', total: 223.6 * GB, free: 134.6 * GB, auto: true, kind: 'drive', removable: true },
        { path: '/run/media/robin/3TB', label: '3TB', fsType: 'fuseblk', total: 2794 * GB, free: 1581 * GB, auto: true, kind: 'drive', removable: false },
        { path: '/run/media/robin/22TB', label: '22TB', fsType: 'fuseblk', total: 22352 * GB, free: 22172 * GB, auto: true, kind: 'drive', removable: false },
        { path: '/run/media/robin/extra', label: 'extra', fsType: 'exfat', total: 465 * GB, free: 290.8 * GB, auto: true, kind: 'drive', removable: true },
      ];
      for (const l of list) l.indexed = roots ? roots.includes(l.path) : true;
      if (roots) for (const r of roots) if (!list.some((l) => l.path === r)) list.push({ path: r, label: r.split('/').pop() || r, fsType: '', total: 0, free: 0, auto: false, indexed: true, kind: 'folder', removable: false, isHome: r === '/home/robin' });
      return list;
    },
  },

  start(m) {
    if (mode === 'first') startBuild(m, 24);
    else if (mode === 'building') { status.state = 'ready'; startBuild(m, 30); }
    // now and then a new screenshot appears (live updates)
    setInterval(() => {
      if (!INDEX || firstRun) return;
      const d = new Date();
      const p2 = (n) => String(n).padStart(2, '0');
      const name = `Schermafbeelding ${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())} om ${p2(d.getHours())}.${p2(d.getMinutes())}.${p2(d.getSeconds())}.png`;
      const dir = `${HOME}/Afbeeldingen/Schermafbeeldingen`;
      const parent = INDEX.byPath.get(dir);
      const e = { name, dir, path: `${dir}/${name}`, isDir: false, size: 812345, modified: Date.now(), ext: 'png', mime: 'image/png', cat: 'picture', hidden: false, kids: null, parent, lname: name.toLowerCase(), lpath: `${dir}/${name}`.toLowerCase() };
      INDEX.entries.push(e);
      INDEX.byPath.set(e.path, e);
      if (parent) parent.kids.push(e);
      extraFiles.push(e);
      status.generation++;
      m.emit('index.changed', { generation: status.generation });
    }, 90000);
  },
};
