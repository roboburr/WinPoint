// Schijven — "Eigenschappen" of a volume and of a disk (Windows 11 Instellingen › Schijven en volumes ›
// Eigenschappen): label, koppelpunt, versleuteling, formatteren, verwijderen, gegevens; schijfstatus (SMART /
// NVMe health), veilig verwijderen, initialiseren, nieuw volume.

import { h, clear } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { bytes, number, percent } from '../../core/format.js';
import { Button, SettingsCard, SettingsGroup, InfoBar, Expander, ProgressRing, Toast, severityIcon } from '../../core/components.js';
import { t } from './i18n.js';
import { driveArt, kindLabel, tableLong, busLabel, healthOf, titleWithMount, detailsGrid, volumeStatus } from './util.js';
import { DiskBar, UsageBar, diskRows } from './overview.js';
import { diskBusy } from './util.js';
import * as act from './actions.js';

const reasonText = (reasons) => [...new Set((reasons || []).filter((r) => r !== 'boot_entry').map((r) => t(`r_${r}`)))].join(' ');
const bootNames = (names) => (names || []).map((n) => `‘${n}’`).join(', ');

/* ------------------------------------------------------------------------------------------- volume */

export function VolumePage(ctx, d, v) {
  const out = [];
  const enc = v.encrypted;
  const locked = enc && !enc.unlocked;

  /* header card: art, name, usage */
  const sub = [d.name, v.partNumber ? t('partition', { n: v.partNumber }) : null, v.fsName, bytes(v.size)].filter(Boolean).join(' · ');
  const openBtn = (v.mounted || v.canMount) && !locked ? Button({ label: t('open'), icon: 'folder-open', onClick: () => act.openInExplorer(ctx, v) }) : null;
  out.push(h('section', { class: 'dk-hero' },
    h('div', { class: 'dk-hero__top' },
      h('span', { class: 'dk-hero__art' }, driveArt(d.kind, 56, { system: v.isSystem })),
      h('div', { class: 'dk-hero__text' },
        h('div', { class: 'dk-hero__title' }, titleWithMount(v)),
        h('div', { class: 'dk-hero__sub' }, sub),
        h('div', { class: 'dk-hero__state' }, volumeStatus(v))),
      openBtn ? h('div', { class: 'dk-hero__end' }, openBtn) : null),
    v.total ? h('div', { class: 'dk-hero__usage' },
      UsageBar(v, { large: true }),
      h('div', { class: 'dk-hero__nums' },
        h('span', null, h('b', null, bytes(v.used)), ` ${t('used').toLowerCase()}`),
        h('span', null, h('b', null, bytes(v.free)), ` ${t('free').toLowerCase()}`))) : null));

  /* why some buttons are off */
  if (v.isSystem) {
    out.push(InfoBar({ severity: 'info', title: t('protectedTitle'), message: `${reasonText(v.protected)} ${t('protectedBody')}`, closable: false }));
  } else if (v.bootEntries?.length) {
    out.push(InfoBar({ severity: 'info', title: t('bootTitle'), message: t('bootBody', { names: bootNames(v.bootEntries) }), closable: false }));
  } else if (v.role === 'system') {
    out.push(InfoBar({ severity: 'warning', title: t('systemPartTitle'), message: t('systemPartBody'), closable: false }));
  } else if (d.dynamic) {
    out.push(InfoBar({ severity: 'warning', title: t('dynamicTitle'), message: t('dynamicBody'), closable: false }));
  }

  /* actions */
  const cards = [];
  if (v.fsType && !v.isSwap && v.fsType !== 'crypto_LUKS') {
    cards.push(SettingsCard({
      icon: 'rename', title: t('label'), description: v.label || t('noLabel'),
      control: Button({ label: t('changeLabel'), disabled: !v.canLabel || !!v.busy, onClick: () => act.changeLabel(ctx, v) }),
    }));
  }
  if (enc) {
    cards.push(SettingsCard({
      icon: locked ? 'lock' : 'lock-open', title: t('encryption', { type: (enc.type || 'LUKS').replace('crypto_', '') }),
      description: locked ? t('encLocked') : t('encUnlocked'),
      control: locked ? Button({ label: `${t('unlock')}…`, variant: 'accent', onClick: () => act.unlock(ctx, v) })
        : Button({ label: t('lock'), disabled: v.isSystem || !!v.busy, onClick: () => act.lock(ctx, v) }),
    }));
  }
  if (!locked && v.fsType && !v.isSwap) {
    cards.push(SettingsCard({
      icon: 'folder', title: t('mountPoint'),
      description: v.mounted ? h('span', { class: 'dk-mono' }, v.mountPoints.join(', ')) : t('mountPointDescOff'),
      control: h('div', { class: 'dk-ctl-row' },
        v.mounted ? Button({ label: t('unmount'), disabled: !v.canUnmount || !!v.busy, onClick: () => act.unmount(ctx, v) })
          : Button({ label: t('mount'), disabled: !v.canMount || !!v.busy, onClick: () => act.mount(ctx, v) })),
    }));
  }
  cards.push(SettingsCard({
    icon: 'storage', title: t('format'), description: t('formatDesc'),
    control: Button({ label: `${t('format')}…`, disabled: !v.canFormat || !!v.busy, onClick: () => act.format(ctx, d, v) }),
  }));
  if (v.kind === 'partition') {
    cards.push(SettingsCard({
      icon: 'delete', title: t('deleteVolume'), description: t('deleteDesc'),
      control: Button({ label: `${t('deleteVolume')}…`, disabled: !v.canDelete || !!v.busy, onClick: () => act.deleteVolume(ctx, d, v) }),
    }));
  }
  out.push(SettingsGroup({ title: t('volume'), children: cards }));

  /* details */
  const rows = [
    [t('d_fs'), v.fsName ? `${v.fsName}${v.fsVersion ? ` (${v.fsVersion})` : ''}` : '—'],
    [t('d_size'), `${bytes(v.size)} (${t('bytes', { n: number(v.size), count: v.size })})`],
    v.total ? [t('capacity'), `${bytes(v.total)} · ${bytes(v.used)} ${t('used').toLowerCase()} (${percent((v.used / v.total) * 100)})`] : null,
    [t('d_type'), v.partTypeName || (v.kind === 'whole' ? kindLabel(d) : null)],
    v.partName ? [t('d_partName'), v.partName] : null,
    v.partNumber ? [t('d_partition'), String(v.partNumber)] : null,
    v.kind === 'partition' ? [t('d_offset'), `${bytes(v.offset)} (${t('bytes', { n: number(v.offset), count: v.offset })})`] : null,
    [t('d_device'), h('span', { class: 'dk-mono' }, enc?.cleartextDevice ? `${v.device} → ${enc.cleartextDevice}` : v.device)],
    v.uuid ? [t('d_uuid'), h('span', { class: 'dk-mono' }, v.uuid)] : null,
    v.mountPoints.length ? [t('d_mounts'), h('span', { class: 'dk-mono' }, v.mountPoints.join(', '))] : null,
    [t('d_disk'), h('button', { type: 'button', class: 'dk-linkbtn', onClick: () => ctx.openDisk(d.id) }, `${d.name} · ${d.model}`)],
  ];
  out.push(SettingsGroup({ title: t('details'), children: [h('div', { class: 'fl-settings-card dk-details-card' }, detailsGrid(rows))] }));
  return out;
}

/* ------------------------------------------------------------------------------------------- disk */

function healthCard(ctx, d) {
  const s = healthOf(d);
  const smart = d.smart || {};
  const sev = { ok: 'success', warning: 'warning', critical: 'error', unknown: 'info' }[s];
  const body = h('div', { class: 'dk-healthcard__grid' });
  const advanced = h('div', { class: 'dk-healthcard__adv' });
  const checkBtn = Button({ label: t('healthCheck'), icon: 'refresh', disabled: !smart.supported, onClick: () => load(true) });
  const ring = ProgressRing({ size: 16 });
  ring.hidden = true;

  function fill(x) {
    const rows = [
      x.lifeRemaining != null ? [t('lifeRemaining'), percent(x.lifeRemaining)] : null,
      x.availableSpare != null ? [t('spare'), percent(x.availableSpare)] : null,
      x.temperature != null ? [t('temperature'), `${number(x.temperature, 0)} °C`] : null,
      x.powerOnHours != null ? [t('powerOn'), t('hours', { n: number(x.powerOnHours), count: x.powerOnHours })] : null,
      x.dataWritten != null ? [t('written'), bytes(x.dataWritten)] : null,
      x.dataRead != null ? [t('read'), bytes(x.dataRead)] : null,
      x.powerCycles != null ? [t('powerCycles'), number(x.powerCycles)] : null,
      x.unsafeShutdowns != null ? [t('unsafeShutdowns'), number(x.unsafeShutdowns)] : null,
      x.badSectors != null ? [t('badSectors'), number(x.badSectors)] : null,
      x.mediaErrors != null ? [t('mediaErrors'), number(x.mediaErrors)] : null,
      x.selftest ? [t('selftest'), x.selftest === 'success' ? t('selftestOk') : x.selftest] : null,
    ];
    clear(body).append(detailsGrid(rows));
    clear(advanced);
    if (Array.isArray(x.attributes) && x.attributes.length) {
      const table = h('table', { class: 'dk-attrs' },
        h('thead', null, h('tr', null, ...['ID', t('attr'), t('attrValue'), t('attrWorst'), t('attrThreshold'), t('attrRaw')].map((c) => h('th', null, c)))),
        h('tbody', null, ...x.attributes.map((a) => h('tr', { class: a.failing ? 'is-failing' : null, title: a.failing ? t('failingNow') : null },
          h('td', null, String(a.id)), h('td', null, a.name), h('td', null, a.value ?? '–'), h('td', null, a.worst ?? '–'),
          h('td', null, a.threshold ?? '–'), h('td', { class: 'dk-mono' }, number(a.pretty))))));
      advanced.append(Expander({ header: t('smartAll'), description: t('smartAllDesc'), icon: 'chart', content: table }));
    }
  }
  async function load(refresh) {
    ring.hidden = false;
    checkBtn.disabled = true;
    try {
      const x = await bridge.call('disks.smart', { disk: d.id, refresh });
      if (ctx.alive()) { fill({ ...smart, ...x }); if (refresh) Toast.show({ severity: 'success', title: t('healthChecked'), timeout: 2500 }); }
    } catch (e) {
      if (refresh) Toast.show({ severity: 'error', title: t('failed'), message: e.message, timeout: 6000 });
    } finally {
      ring.hidden = true;
      checkBtn.disabled = !smart.supported;
    }
  }
  fill(smart);
  if (smart.supported) load(false);
  return h('div', { class: ['fl-settings-card', 'dk-healthcard', `is-${s}`] },
    h('div', { class: 'dk-healthcard__head' },
      h('span', { class: 'dk-healthcard__icon' }, severityIcon(sev, 24)),
      h('div', { class: 'dk-healthcard__text' },
        h('div', { class: 'dk-healthcard__title' }, t(`h_${s}`)),
        h('div', { class: 'dk-healthcard__desc' }, t(`h_${s}_long`))),
      h('div', { class: 'dk-ctl-row' }, ring, checkBtn)),
    body, advanced);
}

export function DiskPage(ctx, d) {
  const out = [];
  const meta = [kindLabel(d), bytes(d.size), tableLong(d)].filter(Boolean).join(' · ');
  out.push(h('section', { class: 'dk-hero' },
    h('div', { class: 'dk-hero__top' },
      h('span', { class: 'dk-hero__art' }, driveArt(d.kind, 56, { system: d.isSystem })),
      h('div', { class: 'dk-hero__text' },
        h('div', { class: 'dk-hero__title' }, `${d.name}: ${d.model}`),
        h('div', { class: 'dk-hero__sub' }, meta))),
    h('div', { class: 'dk-hero__bar' }, DiskBar(ctx, d, { compact: true }))));
  if (d.isSystem) out.push(InfoBar({ severity: 'info', title: t('protectedDiskTitle'), message: t('r_disk'), closable: false }));
  else if (d.bootEntries?.length) out.push(InfoBar({ severity: 'info', title: t('bootDiskTitle'), message: t('bootDiskBody', { names: bootNames(d.bootEntries) }), closable: false }));
  else if (d.dynamic) out.push(InfoBar({ severity: 'warning', title: t('dynamicTitle'), message: t('dynamicBody'), closable: false }));
  const busy = diskBusy(d);

  out.push(SettingsGroup({ title: t('health'), children: [healthCard(ctx, d)] }));

  const cards = [];
  if (d.removable) {
    cards.push(SettingsCard({ icon: 'usb', title: t('safeRemove'), description: t('safeRemoveDesc'),
      control: Button({ label: t('safeRemove'), disabled: busy || !d.canEject, title: busy ? t('busyDisk') : null, onClick: () => act.safeRemove(ctx, d) }) }));
  }
  if (d.mediaRemovable || d.kind === 'optical') {
    cards.push(SettingsCard({ icon: 'arrow-up', title: t('eject'), description: t('ejectDesc'),
      control: Button({ label: t('eject'), disabled: busy || !d.canEject, title: busy ? t('busyDisk') : null, onClick: () => act.eject(ctx, d) }) }));
  }
  for (const f of d.partitionTable ? d.free : []) {
    cards.push(SettingsCard({ icon: 'plus', title: t('newVolume'), description: t(f.logical ? 'freeSpaceDesc' : 'newVolumeDesc', { s: bytes(f.size) }),
      control: Button({ label: t('newVolumeDots'), disabled: !act.canCreateIn(d, f), onClick: () => act.newVolume(ctx, d, f) }) }));
  }
  if (d.kind !== 'optical') {
    cards.push(SettingsCard({
      icon: 'disk', title: d.partitionTable || d.volumes.length ? t('reinitialize') : t('initialize'),
      description: d.partitionTable || d.volumes.length ? t('reinitializeDesc') : t('initializeDesc'),
      control: Button({ label: `${t('initialize')}…`, disabled: !act.canInit(d), onClick: () => act.initialize(ctx, d) }),
    }));
  }
  out.push(SettingsGroup({ title: t('diskSection'), children: cards }));

  const vols = diskRows(ctx, d).filter((r) => !r.classList.contains('dk-vol--free'));
  if (vols.length) out.push(SettingsGroup({ title: t('volumesOnDisk'), children: [h('div', { class: 'dk-vols is-standalone' }, ...vols)] }));

  const rows = [
    [t('d_model'), d.model], d.vendor ? [t('d_vendor'), d.vendor] : null,
    d.serial ? [t('d_serial'), h('span', { class: 'dk-mono' }, d.serial)] : null,
    d.revision ? [t('d_firmware'), d.revision] : null,
    [t('d_kind'), kindLabel(d)], [t('d_bus'), busLabel(d)],
    [t('capacity'), `${bytes(d.size)} (${t('bytes', { n: number(d.size), count: d.size })})`],
    [t('d_style'), tableLong(d)],
    [t('d_device'), h('span', { class: 'dk-mono' }, d.device)],
    d.wwn ? [t('d_wwn'), h('span', { class: 'dk-mono' }, d.wwn)] : null,
    d.loopFile ? [t('d_file'), h('span', { class: 'dk-mono' }, d.loopFile)] : null,
  ];
  out.push(SettingsGroup({ title: t('details'), children: [h('div', { class: 'fl-settings-card dk-details-card' }, detailsGrid(rows))] }));
  return out;
}
