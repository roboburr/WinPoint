// Win11 Shell — mock disks.* (SPEC §6.16), shaped exactly like host/services/disks.py (tests/test_disks.py
// compares the key sets). A realistic fake machine:
//   Schijf 0  Samsung SSD 980 PRO 1TB (NVMe, GPT): EFI (/boot/efi) · Ubuntu (/) · swap (active) · unallocated
//   Schijf 1  WD Blue 4 TB HDD (SATA, GPT): Microsoft reserved · NTFS "Media" · LUKS2 "Kluis" (locked) · unallocated
//   Schijf 2  SanDisk Ultra 32 GB USB stick (MBR): exFAT "SANDISK"
//   Schijf 3  Kingston A400 240 GB SSD: not initialised
// ?mockDisks=more adds
//   Schijf 4  Seagate 1 TB HDD (MBR): NTFS "Oud" · extended partition (ext4 "Archief", NTFS "Foto's", free space
//             inside) · unallocated
//   Schijf 5  Samsung Portable SSD T7 (USB, GPT): EFI with the firmware entry "Windows Boot Manager" · MSR · NTFS
// Every action changes this machine (mount, format with a fake UDisks job and progress, delete, new volume,
// unlock with the password "wachtwoord", safe removal …) and emits disks.changed, like the host does.
// The guards of the host apply with the same messages: system volumes and the system disk, another OS's boot
// volume, system partitions (no delete), one operation per drive, nothing removed while a job runs.
// ?mockDisks=empty → UDisks not available.

const BD = '/org/freedesktop/UDisks2/block_devices/';
const DR = '/org/freedesktop/UDisks2/drives/';
const MiB = 1024 * 1024;
const GiB = 1024 * MiB;
const MIN_VOLUME = 8 * MiB;

const PROTECTED = { '/': 'root', '/boot': 'boot', '/boot/efi': 'efi', '/efi': 'efi', '/home': 'home', '/usr': 'usr', '/var': 'var' };
const SOFT = new Set(['boot_entry']);
const hard = (reasons) => reasons.filter((r) => !SOFT.has(r));
const REASON = {
  root: 'Ubuntu draait vanaf dit volume.', boot: 'Dit volume is nodig om de computer op te starten.',
  efi: 'Dit volume is nodig om de computer op te starten.', home: 'Je persoonlijke map staat op dit volume.',
  usr: 'Onderdelen van Ubuntu staan op dit volume.', var: 'Onderdelen van Ubuntu staan op dit volume.',
  swap: 'Dit volume wordt gebruikt als wisselgeheugen.', shell: 'Win11 Shell draait vanaf dit volume.',
  boot_entry: (n) => `De computer kan vanaf dit volume opstarten (${n}).`,
  disk: 'Ubuntu heeft deze schijf nodig om te werken.',
  disk_boot: (n) => `Op deze schijf staat een volume waarmee de computer opstart (${n}).`,
};
const NO = {
  format: 'Je kunt het daarom niet formatteren.', delete: 'Je kunt het daarom niet verwijderen.',
  unmount: 'Je kunt het daarom niet ontkoppelen.', label: 'Je kunt de naam daarom niet wijzigen.',
  lock: 'Je kunt het daarom niet vergrendelen.', eject: 'Je kunt hem daarom niet verwijderen.',
  initialize: 'Je kunt hem daarom niet initialiseren.',
};
const GPT = {
  efi: 'c12a7328-f81f-11d2-ba4b-00a0c93ec93b', basic: 'ebd0a0a2-b9e5-4433-87c0-68b6b72699c7',
  msr: 'e3c9e316-0b5c-4db8-817d-f92df00215ae', linux: '0fc63daf-8483-4772-8e79-3d69d8477de4',
  swap: '0657fd6d-a4ab-43c4-84e5-0933c84b4f4f', luks: 'ca7d7ccb-63ed-4c53-861c-1742536059cc',
  recovery: 'de94bba4-06d1-4d40-a16a-bfd50179d6ac',
};
const TYPE_ROLE = { efi: 'system', msr: 'system', recovery: 'system', basic: 'data', linux: 'data', swap: 'swap', luks: 'data', ntfs: 'data', fat: 'data', extended: 'container' };
const TYPE_NAME = {
  efi: 'EFI-systeempartitie', basic: 'Basisgegevenspartitie', msr: 'Gereserveerde partitie van Microsoft',
  recovery: 'Herstelpartitie van Windows', linux: 'Linux-bestandssysteem', swap: 'Linux-wisselgeheugen',
  luks: 'Linux LUKS (versleuteld)', ntfs: 'NTFS of exFAT', fat: 'FAT', extended: 'Uitgebreide partitie',
};
const DOS = { ntfs: '0x07', fat: '0x0c', linux: '0x83', swap: '0x82', extended: '0x05' };
const FS_NAMES = { ntfs: 'NTFS', exfat: 'exFAT', vfat: 'FAT32', ext4: 'ext4', btrfs: 'Btrfs', xfs: 'XFS', swap: 'Wisselgeheugen', crypto_LUKS: 'LUKS' };
const OFFLINE_LABEL = new Set(['ntfs', 'exfat', 'vfat', 'btrfs', 'xfs']);
const FILESYSTEMS = [
  { id: 'ntfs', name: 'NTFS', available: true, maxLabel: 128, labelRule: 'chars', linux: false, description: "Voor Windows-pc's en grote bestanden" },
  { id: 'exfat', name: 'exFAT', available: true, maxLabel: 15, labelRule: 'chars', linux: false, description: 'Werkt op bijna alle apparaten (Windows, Mac, Linux)' },
  { id: 'vfat', name: 'FAT32', available: true, maxLabel: 11, labelRule: 'fat', linux: false, description: 'Voor oudere apparaten; bestanden tot 4 GB' },
  { id: 'ext4', name: 'ext4', available: true, maxLabel: 16, labelRule: 'bytes', linux: true, description: 'Voor Linux (aanbevolen voor Ubuntu)' },
  { id: 'btrfs', name: 'Btrfs', available: false, maxLabel: 255, labelRule: 'bytes', linux: true, description: 'Linux, met momentopnamen' },
  { id: 'xfs', name: 'XFS', available: false, maxLabel: 12, labelRule: 'bytes', linux: true, description: 'Linux, voor heel grote bestanden' },
];
const LABEL_RULES = Object.fromEntries(FILESYSTEMS.map((f) => [f.id, [f.maxLabel, f.labelRule]]));
const SYSTEM_PART = (type) => `Dit is een systeempartitie (${type}). Die kun je hier niet formatteren of verwijderen. Wil je de hele schijf opnieuw gebruiken, kies dan ‘Schijf wissen en initialiseren’.`;

function err(code, message) { const e = new Error(message); e.code = code; return e; }
const clone = (v) => JSON.parse(JSON.stringify(v));
let uuidSeq = 0x3a1f;
const newUuid = () => `${(uuidSeq++).toString(16).padStart(4, '0')}a2c4-7d1e-4b6f-9c3a-5e8d21f0b7c${uuidSeq % 10}`;
const query = () => (typeof location !== 'undefined' ? new URLSearchParams(location.search) : new URLSearchParams(''));

/* ------------------------------------------------------------------------------------------- the machine */

function machine() {
  const p = (n, offset, size, o) => ({ n, offset, size, label: '', uuid: newUuid(), mounts: [], used: 0, ...o });
  let off = MiB;
  const next = (size) => { const at = off; off += size; return at; };
  const nvme = {
    drive: DR + 'Samsung_SSD_980_PRO_1TB_S6B0NL0W412873K', block: BD + 'nvme0n1', device: '/dev/nvme0n1',
    model: 'Samsung SSD 980 PRO 1TB', vendor: null, serial: 'S6B0NL0W412873K', revision: '5B2QGXA7', wwn: 'eui.002538b411b2c3d4',
    size: 1000204886016, kind: 'nvme', bus: 'nvme', rotationRate: 0, removable: false, mediaRemovable: false,
    ejectable: false, canPowerOff: false, table: 'gpt',
    smart: { kind: 'nvme', status: 'ok', temperature: 41, powerOnHours: 8123, criticalWarnings: [], selftest: null },
    parts: [],
  };
  nvme.parts.push(p(1, next(512 * MiB), 512 * MiB, { type: 'efi', fs: 'vfat', fsVersion: 'FAT32', uuid: '4C2A-91F0', mounts: ['/boot/efi'], used: 38 * MiB, boot: ['ubuntu'] }));
  nvme.parts.push(p(2, next(500 * GiB), 500 * GiB, { type: 'linux', fs: 'ext4', fsVersion: '1.0', mounts: ['/'], used: 187 * GiB }));
  nvme.parts.push(p(3, next(16 * GiB), 16 * GiB, { type: 'swap', fs: 'swap', fsVersion: '1', swapActive: true }));

  off = MiB;
  const hdd = {
    drive: DR + 'WDC_WD40EZAZ_00SF3B0_WD_WX12D81PLK7T', block: BD + 'sda', device: '/dev/sda',
    model: 'WDC WD40EZAZ-00SF3B0', vendor: null, serial: 'WD-WX12D81PLK7T', revision: '80.00A80', wwn: '0x50014ee2bf8a3c12',
    size: 4000787030016, kind: 'hdd', bus: null, rotationRate: 5400, removable: false, mediaRemovable: false,
    ejectable: false, canPowerOff: false, table: 'gpt',
    smart: { kind: 'ata', status: 'ok', temperature: 38, powerOnHours: 21034, badSectors: 0, failingAttributes: 0, failedInPast: 0, selftest: 'success' },
    parts: [],
  };
  hdd.parts.push(p(1, next(16 * MiB), 16 * MiB, { type: 'msr', fs: null }));
  hdd.parts.push(p(2, next(2500 * GiB), 2500 * GiB, { type: 'basic', fs: 'ntfs', fsVersion: '3.1', label: 'Media', mounts: ['/run/media/robin/Media'], used: 1873 * GiB, uuid: '5E3C18B03C188A77' }));
  hdd.parts.push(p(3, next(500 * GiB), 500 * GiB, { type: 'luks', fs: 'crypto_LUKS', fsVersion: '2', luks: { unlocked: false, inner: 'ext4', label: 'Kluis', used: 96 * GiB, cleartextDevice: '/dev/dm-1' } }));

  const usb = {
    drive: DR + 'SanDisk_Ultra_4C530001250322115331', block: BD + 'sdb', device: '/dev/sdb',
    model: 'SanDisk Ultra', vendor: 'SanDisk', serial: '4C530001250322115331', revision: '1.00', wwn: null,
    size: 30752636928, kind: 'usb', bus: 'usb', rotationRate: -1, removable: true, mediaRemovable: false,
    ejectable: false, canPowerOff: true, table: 'dos',
    smart: { kind: null, status: 'unknown' },
    parts: [p(1, MiB, 30752636928 - MiB, { type: 'ntfs', fs: 'exfat', fsVersion: '1.0', label: 'SANDISK', uuid: '6A21-0F3D', mounts: ['/run/media/robin/SANDISK'], used: 11.2 * GiB })],
  };

  const fresh = {
    drive: DR + 'KINGSTON_SA400S37240G_50026B7784C1A3F2', block: BD + 'sdc', device: '/dev/sdc',
    model: 'KINGSTON SA400S37240G', vendor: null, serial: '50026B7784C1A3F2', revision: 'S3E00100', wwn: '0x50026b7784c1a3f2',
    size: 240057409536, kind: 'ssd', bus: null, rotationRate: 0, removable: false, mediaRemovable: false,
    ejectable: false, canPowerOff: false, table: null,
    smart: { kind: 'ata', status: 'ok', temperature: 33, powerOnHours: 112, badSectors: 0, failingAttributes: 0, failedInPast: 0, selftest: 'success' },
    parts: [],
  };
  const out = [nvme, hdd, usb, fresh];
  if (query().get('mockDisks') === 'more') {
    const ext = MiB + 300 * GiB;
    const old = {
      drive: DR + 'ST1000DM003_1ER162_Z4Y7MK2L', block: BD + 'sdd', device: '/dev/sdd',
      model: 'ST1000DM003-1ER162', vendor: null, serial: 'Z4Y7MK2L', revision: 'CC45', wwn: '0x5000c5007a1b2c3d',
      size: 1000204886016, kind: 'hdd', bus: null, rotationRate: 7200, removable: false, mediaRemovable: false,
      ejectable: false, canPowerOff: false, table: 'dos',
      smart: { kind: 'ata', status: 'warning', temperature: 36, powerOnHours: 41870, badSectors: 8, failingAttributes: 0, failedInPast: 0, selftest: 'success' },
      parts: [
        p(1, MiB, 300 * GiB, { type: 'ntfs', fs: 'ntfs', fsVersion: '3.1', label: 'Oud', mounts: ['/run/media/robin/Oud'], used: 211 * GiB }),
        p(2, ext, 600 * GiB, { type: 'extended', fs: null, container: true }),
        p(5, ext + MiB, 200 * GiB, { type: 'linux', fs: 'ext4', fsVersion: '1.0', label: 'Archief', contained: true }),
        p(6, ext + MiB + 200 * GiB + MiB, 250 * GiB, { type: 'ntfs', fs: 'ntfs', fsVersion: '3.1', label: "Foto's", contained: true, mounts: ["/run/media/robin/Foto's"], used: 188 * GiB }),
      ],
    };
    off = MiB;
    const t7 = {
      drive: DR + 'Samsung_PSSD_T7_S5SXNS0R700123A', block: BD + 'sde', device: '/dev/sde',
      model: 'Samsung Portable SSD T7', vendor: 'Samsung', serial: 'S5SXNS0R700123A', revision: '0', wwn: null,
      size: 1000204886016, kind: 'usb', bus: 'usb', rotationRate: 0, removable: true, mediaRemovable: false,
      ejectable: false, canPowerOff: true, table: 'gpt',
      smart: { kind: null, status: 'unknown' },
      parts: [],
    };
    t7.parts.push(p(1, next(100 * MiB), 100 * MiB, { type: 'efi', fs: 'vfat', fsVersion: 'FAT32', uuid: '9A1C-44E2', boot: ['Windows Boot Manager'] }));
    t7.parts.push(p(2, next(16 * MiB), 16 * MiB, { type: 'msr', fs: null }));
    t7.parts.push(p(3, next(930 * GiB), 930 * GiB, { type: 'basic', fs: 'ntfs', fsVersion: '3.1', label: 'Windows', uuid: '8E2A11C02A11A5F3' }));
    out.push(old, t7);
  }
  return out;
}

/* ------------------------------------------------------------------------------------------- derivation */

function alignUp(v) { return Math.ceil(v / MiB) * MiB; }
function alignDown(v) { return Math.floor(v / MiB) * MiB; }

function freeRegions(size, table, parts, container = null) {
  const first = container ? container.offset : (table === 'gpt' ? 34 * 512 : 512);
  const last = container ? container.offset + container.size : (table === 'gpt' ? size - 33 * 512 : size);
  const lead = container ? MiB : 0;
  const out = [];
  const gap = (x, y) => {
    const a = alignUp(x) + lead, b = alignDown(y) - lead;
    if (b - a >= MIN_VOLUME) out.push({ offset: a, size: b - a, logical: !!container });
  };
  let cur = first;
  for (const q of [...parts].sort((a, b) => a.offset - b.offset)) {
    if (q.offset - cur >= MIN_VOLUME) gap(cur, q.offset);
    cur = Math.max(cur, q.offset + q.size);
  }
  gap(cur, last);
  return out;
}

function smartOut(s) {
  if (!s || !s.kind) return { supported: false, status: 'unknown', healthy: null };
  const base = {
    supported: true, kind: s.kind, enabled: true, updated: Math.floor(Date.now() / 1000) - 120,
    healthy: s.status === 'unknown' ? null : s.status === 'ok', status: s.status,
    temperature: s.temperature ?? null, powerOnHours: s.powerOnHours ?? null, criticalWarnings: s.criticalWarnings || [],
    failingAttributes: s.failingAttributes || 0, badSectors: s.kind === 'ata' ? (s.badSectors ?? 0) : null,
    percentUsed: null, selftest: s.selftest ?? null,
  };
  if (s.kind === 'ata') base.failedInPast = s.failedInPast || 0;
  return base;
}

function volumeName(p, disk, protectedBy) {
  const label = (p.luks?.unlocked ? p.luks.label : '') || p.label;
  if (label) return label;
  if (p.mounts.includes('/') || protectedBy.includes('root')) return 'Systeemschijf';
  if (p.mounts.includes('/home')) return 'Persoonlijke bestanden';
  if (p.type === 'efi' || p.mounts.includes('/boot/efi')) return 'EFI-systeempartitie';
  if (p.fs === 'swap' || p.type === 'swap') return 'Wisselgeheugen';
  if (p.type === 'msr') return 'Gereserveerd door Microsoft';
  if (p.type === 'recovery') return 'Herstelpartitie';
  if (p.type === 'extended') return 'Uitgebreide partitie';
  if (p.luks && !p.luks.unlocked) return 'Versleuteld volume';
  if (!p.fs) return 'Niet-geformatteerd';
  return disk.kind === 'usb' || disk.kind === 'sd' ? 'USB-station' : 'Lokaal station';
}

function partTypeRaw(disk, p) { return disk.table === 'dos' ? (DOS[p.type] || '0x83') : (GPT[p.type] || GPT.basic); }
const volId = (disk, n) => BD + disk.device.slice(5) + (disk.kind === 'nvme' ? 'p' : '') + n;

function buildVolume(disk, p, jobs) {
  const enc = p.luks || null;
  const fsType = enc ? (enc.unlocked ? enc.inner : 'crypto_LUKS') : (p.fs || null);
  const mounts = enc ? (enc.unlocked ? (enc.mounts || []) : []) : p.mounts;
  const protectedBy = [];
  for (const m of mounts) if (PROTECTED[m] && !protectedBy.includes(PROTECTED[m])) protectedBy.push(PROTECTED[m]);
  if (mounts.includes('/')) protectedBy.push('shell');            // Win11 Shell is installed in the home folder on /
  if (p.swapActive) protectedBy.push('swap');
  if (p.boot?.length) protectedBy.push('boot_entry');
  const hardBy = hard(protectedBy);
  const role = TYPE_ROLE[p.type] || 'data';
  const total = mounts.length ? (enc ? p.size - 16 * MiB : p.size) : null;
  const used = mounts.length ? Math.round(enc ? enc.used : p.used) : null;
  const id = volId(disk, p.n);
  const job = jobs.find((j) => j.objects.includes(id));
  const canFs = !!fsType && !['crypto_LUKS', 'swap'].includes(fsType);
  const hasLogical = !!p.container && disk.parts.some((x) => x.contained);
  return {
    id, device: disk.device + (disk.kind === 'nvme' ? 'p' : '') + p.n,
    label: (enc?.unlocked ? enc.label : p.label) || '', name: volumeName(p, disk, protectedBy),
    fsType, fsName: fsType ? (FS_NAMES[fsType] || fsType.toUpperCase()) : null, fsVersion: fsType ? (p.fsVersion || null) : null,
    uuid: p.uuid || null, size: p.size, offset: p.offset, kind: 'partition', partNumber: p.n,
    partType: partTypeRaw(disk, p), partTypeKey: disk.table === 'dos' ? (p.type === 'ntfs' ? 'ntfs' : p.type) : p.type,
    partTypeName: TYPE_NAME[disk.table === 'dos' && p.type === 'ntfs' ? 'ntfs' : p.type] || null, partName: null, role,
    isContainer: !!p.container, isContained: !!p.contained,
    mountPoints: [...mounts], mounted: mounts.length > 0, mountPoint: mounts[0] || null,
    used, free: total != null ? total - used : null, total,
    isSystem: hardBy.length > 0, protected: protectedBy, bootEntries: [...(p.boot || [])],
    isSwap: fsType === 'swap' || p.type === 'swap', swapActive: !!p.swapActive,
    encrypted: enc ? { type: 'crypto_LUKS 2', unlocked: !!enc.unlocked, cleartext: enc.unlocked ? BD + 'dm_2d1' : null,
      cleartextDevice: enc.unlocked ? enc.cleartextDevice : null, inner: enc.unlocked ? enc.inner : null } : null,
    readOnly: false, hidden: p.type === 'msr', fstab: mounts.includes('/') || mounts.includes('/boot/efi'), fstabByLabel: false,
    canMount: canFs && !mounts.length && !(enc && !enc.unlocked), canUnmount: canFs && mounts.length > 0 && !hardBy.length,
    canLabel: !!fsType && !!LABEL_RULES[fsType] && !protectedBy.length,
    labelRemount: mounts.length > 0 && OFFLINE_LABEL.has(fsType),
    canFormat: !protectedBy.length && !['system', 'container'].includes(role),
    canDelete: !protectedBy.length && role !== 'system' && !hasLogical,
    busy: job ? { operation: job.operation, progress: job.progress } : null,
  };
}

function buildDisk(disk, n, jobs) {
  const volumes = disk.parts.slice().sort((a, b) => a.offset - b.offset).map((p) => buildVolume(disk, p, jobs));
  const reasons = [];
  const boot = [];
  for (const v of volumes) {
    for (const r of v.protected) if (!reasons.includes(r)) reasons.push(r);
    for (const b of v.bootEntries) if (!boot.includes(b)) boot.push(b);
  }
  const top = disk.parts.filter((x) => !x.contained);
  const primaries = top.length;
  const full = disk.table === 'dos' && primaries >= 4;
  let free;
  if (!disk.table) free = [{ offset: 0, size: disk.size, logical: false, canCreate: false, container: null }];
  else {
    free = freeRegions(disk.size, disk.table, top).map((f) => ({ ...f, canCreate: !full, container: null }));
    for (const c of disk.parts.filter((x) => x.container)) {
      const inside = disk.parts.filter((x) => x.contained && x.offset >= c.offset && x.offset < c.offset + c.size);
      free.push(...freeRegions(disk.size, disk.table, inside, c).map((f) => ({ ...f, canCreate: true, container: volId(disk, c.n) })));
    }
    free.sort((a, b) => a.offset - b.offset);
  }
  const job = jobs.find((j) => j.objects.includes(disk.block)) || volumes.find((v) => v.busy)?.busy;
  return {
    id: disk.block, drive: disk.drive, block: disk.block, device: disk.device, number: n, name: `Schijf ${n}`, model: disk.model,
    vendor: disk.vendor, serial: disk.serial, revision: disk.revision, wwn: disk.wwn, size: disk.size, kind: disk.kind,
    bus: disk.bus, rotationRate: disk.rotationRate, removable: disk.removable, mediaRemovable: disk.mediaRemovable, media: true,
    ejectable: disk.ejectable, canPowerOff: disk.canPowerOff, readOnly: false, loopFile: null,
    isSystem: hard(reasons).length > 0, protected: reasons, bootEntries: boot,
    canEject: !hard(reasons).length, canInitialize: !reasons.length && disk.kind !== 'optical',
    smart: smartOut(disk.smart),
    partitionTable: disk.table, initialized: !!disk.table || volumes.length > 0,
    canPartition: !!disk.table && free.some((f) => f.canCreate), tableFull: full,
    dynamic: false, volumes, free,
    busy: job ? { operation: job.operation, progress: job.progress } : null,
  };
}

/* ------------------------------------------------------------------------------------------- state */

const S = { disks: null, jobs: [], jobSeq: 1, busyDrives: new Set() };
function state() { if (!S.disks) S.disks = machine(); return S; }

function snapshot() {
  if (query().get('mockDisks') === 'empty') {
    return { supported: false, error: 'Schijfbeheer werkt niet: UDisks is op deze computer niet beschikbaar.', disks: [], jobs: [],
      filesystems: clone(FILESYSTEMS), encryption: { supported: false, types: [] }, version: null, readOnly: false };
  }
  const st = state();
  return {
    supported: true, disks: st.disks.map((d, i) => buildDisk(d, i, st.jobs)), jobs: clone(st.jobs),
    filesystems: clone(FILESYSTEMS), encryption: { supported: true, types: ['luks1', 'luks2'], default: 'luks2' },
    version: '2.10.91', readOnly: false,
  };
}

function changed(m) { m.emit('disks.changed', snapshot()); }

function findVolume(id) {
  const st = state();
  for (const [n, d] of st.disks.entries()) {
    for (const p of d.parts) {
      const v = buildVolume(d, p, st.jobs);
      if (v.id === id || v.encrypted?.cleartext === id) return { d, p, v, disk: buildDisk(d, n, st.jobs) };
    }
  }
  throw err('not_found', 'Dit volume bestaat niet meer. Misschien is de schijf losgekoppeld.');
}

function findDisk(id) {
  const st = state();
  let i = st.disks.findIndex((d) => d.block === id);
  if (i < 0 && st.disks.filter((d) => d.drive === id).length === 1) i = st.disks.findIndex((d) => d.drive === id);
  if (i < 0) throw err('not_found', 'Deze schijf bestaat niet meer. Misschien is hij losgekoppeld.');
  return { d: st.disks[i], disk: buildDisk(st.disks[i], i, st.jobs) };
}

function refuse(reasons, op, { disk = false, names = [] } = {}) {
  if (!reasons?.length) return;
  const n = names.map((x) => `‘${x}’`).join(', ') || '?';
  const h = hard(reasons);
  let why;
  if (disk) why = h.length ? REASON.disk : REASON.disk_boot(n);
  else { const r = (h.length ? h : reasons)[0]; why = typeof REASON[r] === 'function' ? REASON[r](n) : REASON[r]; }
  throw err('protected', `${why} ${NO[op]}`);
}

function expect(obj, ex) {
  if (!ex || ex.size == null || ex.device == null) throw err('bad_args', 'Kies het volume opnieuw en probeer het nog eens.');
  if (Number(ex.size) !== obj.size || ex.device !== obj.device) throw err('changed', 'Deze schijf is veranderd sinds je hem koos. Kijk opnieuw en probeer het nog eens.');
}

/** Nothing is unmounted, relabelled, locked or removed while a UDisks job runs on it. */
function noJobs(objects, name) {
  if (state().jobs.some((j) => j.objects.some((o) => objects.includes(o)))) throw err('busy_job', `Er loopt nog een bewerking op ‘${name}’. Wacht tot die klaar is.`);
}
const diskObjects = (disk) => [disk.block, ...disk.volumes.flatMap((v) => [v.id, v.encrypted?.cleartext].filter(Boolean))];

function checkLabel(fs, label) {
  label = String(label ?? '').trim();
  const rule = LABEL_RULES[fs];
  if (!rule) return label;
  const [limit, kind] = rule;
  const name = FS_NAMES[fs] || fs;
  if (/[\x00-\x1f]/.test(label)) throw err('bad_label', `Deze naam bevat tekens die ${name} niet toestaat.`);
  let size = label.length;
  if (kind === 'fat') {
    label = label.toUpperCase();
    if (/["*+,./:;<=>?[\\\]|]/.test(label) || /[^\x00-\x7f]/.test(label)) throw err('bad_label', `Deze naam bevat tekens die ${name} niet toestaat.`);
  } else if (kind === 'bytes') size = new TextEncoder().encode(label).length;
  if (size > limit) throw err('bad_label', `Deze naam is te lang voor ${name} (maximaal ${limit} tekens).`);
  return label;
}

function fsOk(fs) {
  const f = FILESYSTEMS.find((x) => x.id === fs);
  if (!f) throw err('bad_args', 'Kies een bestandssysteem.');
  if (!f.available) throw err('fs_missing', `${f.name} kan op deze computer niet worden gemaakt: het programma daarvoor ontbreekt.`);
  return fs;
}

function passphrase(args) {
  if (!args.encrypt) return null;
  const pw = args.encrypt.passphrase;
  if (typeof pw !== 'string' || pw.length < 8) throw err('bad_args', 'Kies een wachtwoord van minstens 8 tekens.');
  return pw;
}

const mountPathFor = (label, uuid) => `/run/media/robin/${label || uuid}`;

/** One operation per drive (like the host): op_running for a second one. */
async function onDrive(d, fn) {
  if (S.busyDrives.has(d.drive)) throw err('op_running', 'Er loopt al een bewerking op deze schijf. Wacht tot die klaar is.');
  S.busyDrives.add(d.drive);
  try { return await fn(); } finally { S.busyDrives.delete(d.drive); }
}

/** A fake UDisks job with progress (emits disks.changed while it runs). */
async function job(m, operation, objects, ms) {
  const j = { id: `/org/freedesktop/UDisks2/jobs/${S.jobSeq++}`, operation, progress: 0, objects, cancelable: true, rate: null, expectedEnd: null };
  state().jobs.push(j);
  try {
    const steps = Math.max(2, Math.round(ms / 250));
    for (let i = 1; i <= steps; i++) {
      j.progress = Math.min(1, i / steps);
      changed(m);
      await new Promise((r) => setTimeout(r, ms / steps));
    }
  } finally {
    S.jobs = S.jobs.filter((x) => x !== j);
  }
}

/* ------------------------------------------------------------------------------------------- methods */

export default {
  methods: {
    'disks.list': async () => snapshot(),

    'disks.mount': async ({ id } = {}, m) => {
      const { p, v } = findVolume(id);
      if (v.mounted) return { ok: true, mountPoint: v.mountPoint, already: true };
      if (p.luks && !p.luks.unlocked) throw err('locked', 'Ontgrendel dit volume eerst.');
      if (!v.fsType || ['swap', 'crypto_LUKS'].includes(v.fsType)) throw err('no_fs', 'Op dit volume staat geen bestandssysteem dat kan worden gekoppeld.');
      noJobs([v.id], v.name);
      const mp = mountPathFor(v.label, p.uuid);
      if (p.luks) p.luks.mounts = [mp]; else p.mounts = [mp];
      changed(m);
      return { ok: true, mountPoint: mp };
    },

    'disks.unmount': async ({ id } = {}, m) => {
      const { d, p, v } = findVolume(id);
      refuse(hard(v.protected), 'unmount');
      if (!v.mounted) return { ok: true, already: true };
      return onDrive(d, async () => {
        noJobs([v.id], v.name);
        if (p.luks) p.luks.mounts = []; else p.mounts = [];
        changed(m);
        return { ok: true };
      });
    },

    'disks.open': async ({ id } = {}, m) => {
      let { v } = findVolume(id);
      let mounted = false;
      if (!v.mounted) {
        await m.call('disks.mount', { id });
        ({ v } = findVolume(id));
        mounted = true;
      }
      await m.call('shell.openApp', { app: 'explorer', params: { path: v.mountPoint } }).catch(() => {});
      return { ok: true, path: v.mountPoint, mounted };
    },

    'disks.setLabel': async ({ id, label } = {}, m) => {
      const { d, p, v } = findVolume(id);
      refuse(v.protected, 'label', { names: v.bootEntries });
      if (p.luks && !p.luks.unlocked) throw err('locked', 'Ontgrendel dit volume eerst.');
      if (!LABEL_RULES[v.fsType]) throw err('no_label_fs', 'Dit volume heeft geen bestandssysteem met een naam.');
      const l = checkLabel(v.fsType, label);
      return onDrive(d, async () => {
        noJobs([v.id], v.name);
        if (p.luks) p.luks.label = l; else p.label = l;
        let mp = null;
        if (v.labelRemount) {                   // ntfslabel & co.: unmounted, relabelled, mounted again
          await new Promise((r) => setTimeout(r, 300));
          mp = mountPathFor(l, p.uuid);
          if (p.luks) p.luks.mounts = [mp]; else p.mounts = [mp];
        }
        changed(m);
        return { ok: true, label: l, remounted: v.labelRemount, mountPoint: mp, mountError: null };
      });
    },

    'disks.format': async (args = {}, m) => {
      const { d, p, v } = findVolume(args.id);
      refuse(v.protected, 'format', { names: v.bootEntries });
      expect(v, args.expect);
      if (['system', 'container'].includes(v.role)) throw err('system_partition', SYSTEM_PART(v.partTypeName));
      const fs = fsOk(args.fsType);
      const label = checkLabel(fs, args.label || '');
      const pw = passphrase(args);
      return onDrive(d, async () => {
        noJobs([v.id], v.name);
        const was = v.mounted;
        if (p.luks) p.luks.mounts = []; else p.mounts = [];
        await job(m, args.quick === false ? 'format-erase' : 'format-mkfs', [v.id], args.quick === false ? 4000 : 1400);
        p.uuid = newUuid();
        if (pw) {
          p.fs = 'crypto_LUKS'; p.type = 'luks';
          p.luks = { unlocked: true, inner: fs, label, used: 0.02 * GiB, cleartextDevice: '/dev/dm-1', mounts: [], pw };
          p.label = '';
        } else {
          p.fs = fs; p.luks = null; p.label = label; p.used = fs === 'ntfs' ? 70 * MiB : 0.15 * GiB;
          p.fsVersion = { ntfs: '3.1', exfat: '1.0', vfat: 'FAT32', ext4: '1.0' }[fs] || null;
          if (['efi', 'msr', 'swap', 'luks'].includes(p.type)) p.type = fs === 'ext4' ? 'linux' : 'basic';
        }
        let mp = null;
        if (was || args.mount) {
          mp = mountPathFor(label, p.uuid);
          if (p.luks) p.luks.mounts = [mp]; else p.mounts = [mp];
        }
        changed(m);
        return { ok: true, id: v.id, mountPoint: mp, mountError: null };
      });
    },

    'disks.deletePartition': async (args = {}, m) => {
      const { d, p, v } = findVolume(args.id);
      refuse(v.protected, 'delete', { names: v.bootEntries });
      expect(v, args.expect);
      if (v.role === 'system') throw err('system_partition', SYSTEM_PART(v.partTypeName));
      if (v.isContainer && d.parts.some((x) => x.contained)) throw err('not_empty', 'Verwijder eerst de logische stations in deze partitie.');
      return onDrive(d, async () => {
        noJobs([v.id], v.name);
        p.mounts = [];
        await job(m, 'partition-delete', [v.id], 600);
        d.parts = d.parts.filter((x) => x !== p);
        changed(m);
        return { ok: true };
      });
    },

    'disks.createPartition': async (args = {}, m) => {
      const { d, disk } = findDisk(args.disk);
      if (args.expect != null) expect(disk, args.expect);
      if (!disk.partitionTable) throw err('no_table', 'Deze schijf is nog niet geïnitialiseerd. Initialiseer hem eerst.');
      const offset = Number(args.offset);
      const region = disk.free.find((r) => r.offset <= offset && offset < r.offset + r.size);
      if (!region) throw err('no_free', 'Deze vrije ruimte bestaat niet meer. Kijk opnieuw en probeer het nog eens.');
      if (!region.canCreate) throw err('table_full', 'Op deze schijf kan geen partitie meer bij: een MBR-schijf heeft er maximaal vier.');
      const room = region.offset + region.size - offset;
      let size = Number(args.size) || 0;
      if (size <= 0 || size > room) size = room;
      if (size < MIN_VOLUME) throw err('too_small', 'Een volume moet minstens 8 MB groot zijn.');
      if (size < room && room - size < MiB) size = room;
      const fs = fsOk(args.fsType);
      const label = checkLabel(fs, args.label || '');
      const pw = passphrase(args);
      return onDrive(d, async () => {
        noJobs(diskObjects(disk), disk.name);
        await job(m, 'partition-create', [disk.block], 1500);
        const nums = d.parts.map((x) => x.n);
        const n = region.logical ? Math.max(4, ...nums) + 1 : ([1, 2, 3, 4].find((k) => !nums.includes(k)) || 1 + Math.max(0, ...nums));
        const uuid = newUuid();
        const mp = args.mount === false ? null : mountPathFor(label, uuid);
        const dosType = fs === 'ext4' ? 'linux' : (fs === 'vfat' ? 'fat' : 'ntfs');
        const part = pw
          ? { n, offset, size, type: 'luks', fs: 'crypto_LUKS', fsVersion: '2', label: '', uuid, mounts: [], used: 0, contained: region.logical,
            luks: { unlocked: true, inner: fs, label, used: 0.02 * GiB, cleartextDevice: '/dev/dm-2', mounts: mp ? [mp] : [], pw } }
          : { n, offset, size, type: d.table === 'dos' ? dosType : (fs === 'ext4' ? 'linux' : 'basic'), contained: region.logical,
            fs, fsVersion: null, label, uuid, mounts: mp ? [mp] : [], used: fs === 'ntfs' ? 70 * MiB : 0.15 * GiB };
        d.parts.push(part);
        changed(m);
        return { ok: true, id: volId(d, n), mountPoint: mp, mountError: null };
      });
    },

    'disks.initialize': async (args = {}, m) => {
      const { d, disk } = findDisk(args.disk);
      refuse(disk.protected, 'initialize', { disk: true, names: disk.bootEntries });
      expect(disk, args.expect);
      if (!['gpt', 'dos'].includes(args.table || 'gpt')) throw err('bad_args', 'Kies GPT of MBR.');
      return onDrive(d, async () => {
        noJobs(diskObjects(disk), disk.name);
        await job(m, 'format-mkfs', [disk.block], 800);
        d.parts = [];
        d.table = args.table || 'gpt';
        changed(m);
        return { ok: true };
      });
    },

    'disks.eject': async ({ disk: id, id: id2 } = {}, m) => {
      const { d, disk } = findDisk(id || id2);
      refuse(hard(disk.protected), 'eject', { disk: true });
      if (!disk.removable) throw err('not_removable', 'Deze schijf kan niet worden uitgeworpen.');
      return onDrive(d, async () => {
        noJobs(diskObjects(disk), disk.model);
        state().disks = state().disks.filter((x) => x !== d);
        changed(m);
        replug(m, d);
        return { ok: true };
      });
    },

    'disks.powerOff': async ({ disk: id, id: id2 } = {}, m) => {
      const { d, disk } = findDisk(id || id2);
      refuse(hard(disk.protected), 'eject', { disk: true });
      if (!disk.removable) throw err('not_removable', 'Deze schijf kan niet worden uitgeworpen.');
      return onDrive(d, async () => {
        noJobs(diskObjects(disk), disk.model);
        await new Promise((r) => setTimeout(r, 400));
        state().disks = state().disks.filter((x) => x !== d);
        changed(m);
        replug(m, d);
        return { ok: true, how: d.canPowerOff ? 'poweroff' : (d.ejectable ? 'eject' : 'unmounted') };
      });
    },

    'disks.unlock': async ({ id, passphrase: pw, mount = true } = {}, m) => {
      const { p, v } = findVolume(id);
      if (!p.luks) throw err('not_encrypted', 'Dit volume is niet versleuteld.');
      if (p.luks.unlocked) return { ok: true, cleartext: v.encrypted.cleartext, already: true, mountPoint: v.mountPoint };
      if (typeof pw !== 'string' || !pw) throw err('bad_args', 'Typ het wachtwoord.');
      await new Promise((r) => setTimeout(r, 700));
      if (pw !== (p.luks.pw || 'wachtwoord')) throw err('bad_passphrase', 'Het wachtwoord klopt niet. Probeer het opnieuw.');
      p.luks.unlocked = true;
      const mp = mount ? mountPathFor(p.luks.label, p.uuid) : null;
      p.luks.mounts = mp ? [mp] : [];
      changed(m);
      return { ok: true, cleartext: BD + 'dm_2d1', mountPoint: mp, mountError: null };
    },

    'disks.lock': async ({ id } = {}, m) => {
      const { d, p, v } = findVolume(id);
      if (!p.luks) throw err('not_encrypted', 'Dit volume is niet versleuteld.');
      refuse(hard(v.protected), 'lock');
      return onDrive(d, async () => {
        noJobs([v.id, v.encrypted?.cleartext].filter(Boolean), v.name);
        p.luks.unlocked = false;
        p.luks.mounts = [];
        changed(m);
        return { ok: true };
      });
    },

    'disks.smart': async ({ disk: id, id: id2, refresh } = {}) => {
      const { disk } = findDisk(id || id2);
      const out = { ...disk.smart };
      if (!out.supported) return { ...out, message: 'Deze schijf geeft geen gezondheidsgegevens door.' };
      if (refresh) await new Promise((r) => setTimeout(r, 900));
      if (out.kind === 'nvme') {
        Object.assign(out, {
          percentUsed: 3, lifeRemaining: 97, availableSpare: 100, spareThreshold: 10, dataRead: 41.7e12, dataWritten: 28.3e12,
          powerCycles: 1488, unsafeShutdowns: 37, mediaErrors: 0, errorLogEntries: 0, warningTempMinutes: 0,
          criticalTempMinutes: 0, sensors: [41, 52], details: true,
        });
      } else {
        const hours = disk.smart.powerOnHours || 0;
        const a = (id_, name, value, worst, threshold, pretty, unit, prefail) => ({ id: id_, name, prefail, value, worst, threshold, pretty, unit, failing: false, failedInPast: false });
        out.attributes = [
          a(1, 'raw-read-error-rate', 200, 200, 51, 0, 1, true), a(3, 'spin-up-time', 178, 172, 21, 6050, 2, true),
          a(4, 'start-stop-count', 100, 100, 0, 2210, 1, false), a(5, 'reallocated-sector-count', 200, 200, 140, disk.smart.badSectors || 0, 3, true),
          a(7, 'seek-error-rate', 200, 200, 0, 0, 1, false), a(9, 'power-on-hours', 72, 72, 0, hours * 3600000, 2, false),
          a(10, 'spin-retry-count', 100, 100, 0, 0, 1, false), a(12, 'power-cycle-count', 100, 100, 0, 1812, 1, false),
          a(194, 'temperature-celsius-2', 112, 104, 0, 311150, 4, false), a(197, 'current-pending-sector', 200, 200, 0, 0, 3, false),
          a(198, 'offline-uncorrectable', 100, 253, 0, 0, 3, false), a(199, 'udma-crc-error-count', 200, 200, 0, 0, 1, false),
        ];
      }
      return out;
    },
  },
  start() {},
};

/** ?mockReplug=SEC: a removed USB stick comes back after SEC seconds (hotplug demo). */
function replug(m, d) {
  const sec = Number(query().get('mockReplug'));
  if (!(sec > 0) || !d.removable) return;
  setTimeout(() => { state().disks.splice(Math.min(2, state().disks.length), 0, d); changed(m); }, sec * 1000);
}
