// Schijven — "Schijven en volumes": one card per disk (Windows 11 Instellingen) with the Schijfbeheer bar
// (partitions and unallocated space side by side, proportional, click to select, right-click for actions; an MBR
// extended partition is a frame around its logical drives and the free space in it) and the volume list below it
// (usage bar, Eigenschappen). Keyboard: one tab stop per bar (roving tabindex), arrows move, Enter opens,
// Shift+F10 / menu key for the actions.

import { h } from '../../core/dom.js';
import { bytes } from '../../core/format.js';
import { Button, IconButton } from '../../core/components.js';
import { icon } from '../../core/icons.js';
import { t } from './i18n.js';
import { driveArt, volumeIcon, kindLabel, tableLabel, healthOf, titleWithMount, volumeStatus, busyText, diskBusy } from './util.js';
import * as act from './actions.js';

/** Segment kind for colours: data · system · container · swap · encrypted · free · raw · busy */
export function segKind(v) {
  if (!v) return 'free';
  if (v.busy) return 'busy';
  if (v.encrypted) return 'encrypted';
  if (v.isSwap) return 'swap';
  if (v.role === 'container') return 'container';
  if (v.role === 'system') return 'system';
  if (!v.fsType) return 'raw';
  return 'data';
}

export function UsageBar(v, { large = false } = {}) {
  const pct = v.total ? Math.max(0, Math.min(100, (v.used / v.total) * 100)) : 0;
  const fill = h('span', { class: 'dk-usage__fill', style: { width: `${pct}%` } });
  const el = h('span', {
    class: ['dk-usage', large && 'is-large', pct >= 90 && 'is-full'], role: 'progressbar',
    'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': Math.round(pct),
  }, fill);
  return el;
}

function healthChip(d) {
  const s = healthOf(d);
  if (s === 'unknown' && !d.smart?.supported) return null;
  return h('span', { class: ['dk-health', `is-${s}`] }, h('i', { class: 'dk-health__dot', 'aria-hidden': 'true' }), t(`h_${s}`));
}

/** The disk's layout in offset order. Top level: primary / GPT partitions, extended partitions (with their logical
 *  drives and the free space inside as `children`) and unallocated space. */
export function layout(d) {
  const inner = new Map();          // container id → [items]
  const top = [];
  const containers = d.volumes.filter((v) => v.isContainer);
  const holder = (offset) => containers.find((c) => c.offset <= offset && offset < c.offset + c.size);
  for (const v of d.volumes) {
    const item = { kind: 'volume', v, offset: v.offset, size: v.size };
    const c = v.isContained ? holder(v.offset) : null;
    if (c) { if (!inner.has(c.id)) inner.set(c.id, []); inner.get(c.id).push(item); } else top.push(item);
  }
  for (const f of d.free) {
    const item = { kind: 'free', f, offset: f.offset, size: f.size };
    const c = f.logical ? (containers.find((x) => x.id === f.container) || holder(f.offset)) : null;
    if (c) { if (!inner.has(c.id)) inner.set(c.id, []); inner.get(c.id).push(item); } else top.push(item);
  }
  const byOffset = (a, b) => a.offset - b.offset;
  top.sort(byOffset);
  for (const it of top) if (it.kind === 'volume' && it.v.isContainer) it.children = (inner.get(it.v.id) || []).sort(byOffset);
  return top;
}

/** Flat list in display order (container, then its contents). */
export function flatLayout(d) {
  return layout(d).flatMap((it) => [it, ...(it.children || [])]);
}

export const segId = (d, it) => (it.kind === 'volume' ? it.v.id : `free:${d.id}:${it.f.offset}`);

/** The Schijfbeheer bar for one disk. */
export function DiskBar(ctx, d, { compact = false } = {}) {
  const items = layout(d);
  const bar = h('div', { class: ['dk-bar', compact && 'is-compact'], role: 'listbox', 'aria-label': `${d.name}: ${d.model}`, dataset: { disk: d.id } });
  if (!items.length) {
    bar.append(h('div', { class: 'dk-seg dk-seg--empty' }, h('div', { class: 'dk-seg__body' },
      h('div', { class: 'dk-seg__name' }, d.media === false ? '—' : t('unallocated')))));
    return bar;
  }
  const grow = (it, whole) => {
    const share = whole ? it.size / whole : 0;
    const small = it.kind === 'volume' && it.v.partTypeKey === 'msr';
    return Math.max(share, small ? 0.025 : 0.07);
  };
  const ids = flatLayout(d).filter((it) => !(it.kind === 'volume' && it.v.isContainer)).map((it) => segId(d, it));
  const tabStop = ids.includes(ctx.selected) ? ctx.selected : ids[0];

  function segment(it, whole) {
    const id = segId(d, it);
    const v = it.v;
    const inExt = it.kind === 'free' && it.f.logical;
    const kind = it.kind === 'free' ? 'free' : segKind(v);
    const name = it.kind === 'free' ? (d.partitionTable ? (inExt ? t('freeSpace') : t('unallocated')) : t('noTable')) : titleWithMount(v);
    const line2 = it.kind === 'free' ? bytes(it.size) : [bytes(v.size), v.fsName].filter(Boolean).join(' ');
    const line3 = it.kind === 'free' ? '' : (v.busy ? busyText(v.busy) : volumeStatus(v));
    const seg = h('div', {
      class: ['dk-seg', `dk-seg--${kind}`, inExt && 'is-in-extended', ctx.selected === id && 'is-selected'], role: 'option',
      tabindex: id === tabStop ? 0 : -1,
      'aria-selected': String(ctx.selected === id), dataset: { id }, style: { flexGrow: String(grow(it, whole)) },
      title: [name, line2, line3].filter(Boolean).join('\n'),
    },
    h('div', { class: 'dk-seg__body' },
      h('div', { class: 'dk-seg__name' }, it.kind === 'volume' && v.encrypted ? icon(v.encrypted.unlocked ? 'lock-open' : 'lock', { size: 12 }) : null, h('span', null, name)),
      h('div', { class: 'dk-seg__line' }, line2),
      line3 ? h('div', { class: 'dk-seg__line dk-seg__status' }, line3) : null),
    v?.busy && typeof v.busy.progress === 'number' ? h('div', { class: 'dk-seg__progress', style: { width: `${Math.round(v.busy.progress * 100)}%` } }) : null);
    const menu = () => (it.kind === 'volume' ? act.volumeMenu(ctx, d, v) : act.freeMenu(ctx, d, it.f));
    const open = (e) => { if (it.kind === 'volume') ctx.openVolume(v.id); else act.freeDefault(ctx, d, it.f, e, seg); };
    seg.addEventListener('click', (e) => { e.stopPropagation(); ctx.select(id); });
    seg.addEventListener('dblclick', (e) => { e.stopPropagation(); open(e); });
    seg.addEventListener('contextmenu', (e) => { e.stopPropagation(); ctx.select(id); act.openMenu(e, menu(), seg); });
    seg.addEventListener('keydown', (e) => {
      const segs = [...bar.querySelectorAll('.dk-seg[data-id]')];
      const k = segs.indexOf(seg);
      if (e.key === 'ArrowRight' || e.key === 'ArrowLeft' || e.key === 'Home' || e.key === 'End') {
        e.preventDefault();
        const to = e.key === 'Home' ? 0 : e.key === 'End' ? segs.length - 1 : k + (e.key === 'ArrowRight' ? 1 : -1);
        const n = segs[Math.max(0, Math.min(segs.length - 1, to))];
        ctx.select(n.dataset.id, { focus: true });
      } else if (e.key === 'Enter') {
        e.preventDefault();
        open(e);
      } else if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) {
        act.openMenu(e, menu(), seg);
      }
    });
    return seg;
  }

  for (const it of items) {
    if (it.kind === 'volume' && it.v.isContainer) {
      // Schijfbeheer: the extended partition is a frame around its logical drives and the free space in it
      const c = it.v;
      const group = h('div', {
        class: ['dk-seg-group', ctx.selected === c.id && 'is-selected'], role: 'group', dataset: { id: c.id },
        'aria-label': `${c.name}, ${bytes(c.size)}`, title: `${c.name}\n${bytes(c.size)}`, style: { flexGrow: String(grow(it, d.size)) },
      },
      h('div', { class: 'dk-seg-group__label' }, h('span', null, c.name), h('span', { class: 'dk-seg-group__size' }, bytes(c.size))),
      h('div', { class: 'dk-seg-group__inner' }, ...(it.children.length ? it.children.map((x) => segment(x, c.size))
        : [h('div', { class: 'dk-seg dk-seg--free is-in-extended', style: { flexGrow: '1' } })])));
      group.addEventListener('click', () => ctx.select(c.id));
      group.addEventListener('dblclick', () => ctx.openVolume(c.id));
      group.addEventListener('contextmenu', (e) => { ctx.select(c.id); act.openMenu(e, act.volumeMenu(ctx, d, c), group); });
      bar.append(group);
    } else {
      bar.append(segment(it, d.size));
    }
  }
  return bar;
}

/** One row of the volume list (Windows 11 Settings style). */
export function VolumeRow(ctx, d, v) {
  const id = v.id;
  const logical = v.isContainer ? d.volumes.filter((x) => x.isContained && x.offset >= v.offset && x.offset < v.offset + v.size).length : 0;
  const desc = v.isContainer ? t('containerDesc', { n: logical }) : [v.fsName !== v.name ? v.fsName : null, volumeStatus(v),
    v.mounted ? null : (v.fsType && !v.encrypted && v.role === 'data' && !v.isSwap ? t('notMounted') : null)]
    .filter(Boolean).join(' · ');
  const usage = v.total ? h('div', { class: 'dk-vol__usage' }, UsageBar(v),
    h('span', { class: 'dk-vol__usage-text' }, t('freeOf', { f: bytes(v.free), t: bytes(v.total) }))) : h('div', { class: 'dk-vol__usage' },
    h('span', { class: 'dk-vol__usage-text' }, bytes(v.size)));
  const props = Button({ label: t('properties'), onClick: (e) => { e.stopPropagation(); ctx.openVolume(id); } });
  const row = h('div', {
    class: ['dk-vol', v.isContained && 'is-contained', ctx.selected === id && 'is-selected', v.busy && 'is-busy'], tabindex: 0, role: 'button', dataset: { id },
    'aria-label': `${titleWithMount(v)}, ${desc}`,
  },
  h('span', { class: ['dk-vol__icon', `is-${segKind(v)}`] }, v.isContainer ? icon('storage', { size: 20 }) : volumeIcon(v, d)),
  h('div', { class: 'dk-vol__text' },
    h('div', { class: 'dk-vol__title' }, titleWithMount(v)),
    h('div', { class: 'dk-vol__desc' }, desc || v.partTypeName || '')),
  usage,
  h('div', { class: 'dk-vol__end' }, props));
  row.addEventListener('click', () => ctx.select(id));
  row.addEventListener('dblclick', () => ctx.openVolume(id));
  row.addEventListener('keydown', (e) => { if (e.key === 'Enter' && e.target === row) ctx.openVolume(id); });
  row.addEventListener('contextmenu', (e) => { ctx.select(id); act.openMenu(e, act.volumeMenu(ctx, d, v), row); });
  return row;
}

export function FreeRow(ctx, d, f) {
  const id = `free:${d.id}:${f.offset}`;
  const init = !d.partitionTable;
  const btn = init
    ? Button({ label: `${t('initialize')}…`, disabled: !act.canInit(d), onClick: (e) => { e.stopPropagation(); act.initialize(ctx, d); } })
    : Button({ label: t('newVolumeDots'), icon: 'plus', disabled: !act.canCreateIn(d, f), onClick: (e) => { e.stopPropagation(); act.newVolume(ctx, d, f); } });
  const row = h('div', { class: ['dk-vol', 'dk-vol--free', f.logical && 'is-contained', ctx.selected === id && 'is-selected'], tabindex: 0, role: 'button', dataset: { id } },
    h('span', { class: 'dk-vol__icon is-free' }, icon(init ? 'disk' : 'storage', { size: 20 })),
    h('div', { class: 'dk-vol__text' },
      h('div', { class: 'dk-vol__title' }, init ? t('noTable') : (f.logical ? t('freeSpace') : t('unallocated'))),
      h('div', { class: 'dk-vol__desc' }, init ? t('initializeDesc') : t(f.logical ? 'freeSpaceDesc' : 'newVolumeDesc', { s: bytes(f.size) }))),
    h('div', { class: 'dk-vol__usage' }, h('span', { class: 'dk-vol__usage-text' }, bytes(f.size))),
    h('div', { class: 'dk-vol__end' }, btn));
  row.addEventListener('click', () => ctx.select(id));
  row.addEventListener('contextmenu', (e) => { ctx.select(id); act.openMenu(e, act.freeMenu(ctx, d, f), row); });
  return row;
}

/** Volume and free-space rows of a disk in layout order (MSR partitions hidden, like Windows). */
export function diskRows(ctx, d) {
  const rows = [];
  for (const it of flatLayout(d)) {
    if (it.kind === 'volume') { if (it.v.partTypeKey !== 'msr') rows.push(VolumeRow(ctx, d, it.v)); } else rows.push(FreeRow(ctx, d, it.f));
  }
  return rows;
}

export function DiskCard(ctx, d) {
  const meta = [kindLabel(d), bytes(d.size), tableLabel(d), d.dynamic ? t('dynamic') : null].filter(Boolean).join(' · ');
  const more = IconButton({ icon: 'more', label: t('more'), onClick: (e, b) => act.openMenu(e, act.diskMenu(ctx, d), b) });
  const busy = diskBusy(d);
  const actions = h('div', { class: 'dk-disk__actions' },
    d.removable && d.canEject ? Button({ label: t('safeRemove'), icon: 'usb', disabled: busy, title: busy ? t('busyDisk') : null, onClick: () => act.safeRemove(ctx, d) }) : null,
    Button({ label: t('properties'), onClick: () => ctx.openDisk(d.id) }), more);
  const head = h('div', { class: 'dk-disk__head' },
    h('span', { class: 'dk-disk__art' }, driveArt(d.kind, 40, { system: d.isSystem })),
    h('div', { class: 'dk-disk__text' },
      h('div', { class: 'dk-disk__title' }, h('span', { class: 'dk-disk__name' }, d.name), h('span', { class: 'dk-disk__model' }, d.model)),
      h('div', { class: 'dk-disk__meta' }, h('span', null, meta), healthChip(d), d.busy ? h('span', { class: 'dk-busy' }, busyText(d.busy)) : null)),
    actions);
  head.addEventListener('contextmenu', (e) => { if (!e.target.closest('button')) act.openMenu(e, act.diskMenu(ctx, d), head); });
  const rows = diskRows(ctx, d);
  return h('section', { class: ['dk-disk', d.isSystem && 'is-system'], dataset: { id: d.id }, 'aria-label': `${d.name}: ${d.model}` },
    head, h('div', { class: 'dk-disk__bar' }, DiskBar(ctx, d)), rows.length ? h('div', { class: 'dk-vols' }, ...rows) : null);
}

export function Legend(snap) {
  const k = (cls, label) => h('span', { class: 'dk-legend__item' }, h('i', { class: `dk-legend__sw dk-seg--${cls}` }), label);
  const ext = (snap?.disks || []).some((d) => d.volumes.some((v) => v.isContainer));
  return h('div', { class: 'dk-legend', 'aria-label': t('legend') },
    k('data', t('lg_data')), k('system', t('lg_system')), ext ? k('container', t('lg_container')) : null,
    k('swap', t('lg_swap')), k('encrypted', t('lg_encrypted')), k('free', t('lg_free')));
}
