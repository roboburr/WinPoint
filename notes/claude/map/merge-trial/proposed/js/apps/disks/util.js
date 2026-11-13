// Schijven — shared helpers: drive artwork (original drawings), labels, API calls with friendly errors.

import { h } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { bytes, number } from '../../core/format.js';
import { Toast } from '../../core/components.js';
import { icon } from '../../core/icons.js';
import { t } from './i18n.js';

const NS = 'http://www.w3.org/2000/svg';
let gid = 0;

function svg(size, inner, view = 48) {
  const s = document.createElementNS(NS, 'svg');
  s.setAttribute('viewBox', `0 0 ${view} ${view}`);
  s.setAttribute('width', size);
  s.setAttribute('height', size);
  s.setAttribute('aria-hidden', 'true');
  s.setAttribute('class', 'dk-art');
  s.innerHTML = inner;
  return s;
}

/** The app icon: a drive with its partition bar (title bar, 16 px; also used large in empty states). */
export function appGlyph(size = 16) {
  const id = `dkapp${++gid}`;
  return svg(size, `<defs><linearGradient id="${id}" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#4b9fea"/><stop offset="1" stop-color="#1d5fbf"/></linearGradient></defs>`
    + `<rect x=".5" y="2.5" width="15" height="11" rx="2.5" fill="url(#${id})"/>`
    + '<rect x="2.5" y="4.5" width="4" height="3.2" rx=".7" fill="#fff"/><rect x="7.3" y="4.5" width="4" height="3.2" rx=".7" fill="#9be7f2"/>'
    + '<rect x="12.1" y="4.5" width="1.6" height="3.2" rx=".5" fill="#c9d6e6"/><path d="M2.6 10.6h6.4" stroke="#fff" stroke-opacity=".75" stroke-width="1.1" stroke-linecap="round"/>'
    + '<circle cx="12.6" cy="10.6" r=".95" fill="#5ef0a0"/>', 16);
}

/** Drive artwork per kind (48-unit viewBox, drawn to `size`). */
export function driveArt(kind, size = 40, { system = false } = {}) {
  const id = `dk${++gid}`;
  const lg = (a, b, vertical = true) => `<linearGradient id="${id}" x1="0" y1="0" x2="${vertical ? 0 : 1}" y2="1"><stop offset="0" stop-color="${a}"/><stop offset="1" stop-color="${b}"/></linearGradient>`;
  let inner;
  switch (kind) {
    case 'nvme':
      inner = `<defs>${lg('#2f8f5b', '#1c6b42', false)}</defs>`
        + `<rect x="3" y="15" width="42" height="18" rx="2.5" fill="url(#${id})"/>`
        + '<rect x="40" y="18" width="5" height="12" fill="#e4c56a"/><path d="M41 20v8M43 20v8" stroke="#b8942f" stroke-width=".8"/>'
        + '<rect x="7" y="18.5" width="12" height="11" rx="1.2" fill="#1d2228"/><rect x="21" y="18.5" width="12" height="11" rx="1.2" fill="#1d2228"/>'
        + '<rect x="8.5" y="20" width="9" height="3" rx=".6" fill="#3a4450"/><rect x="22.5" y="20" width="9" height="3" rx=".6" fill="#3a4450"/>'
        + '<rect x="34.5" y="20" width="4" height="8" rx=".8" fill="#262c33"/><circle cx="5.2" cy="24" r="1.3" fill="#0f3d26"/>';
      break;
    case 'ssd':
      inner = `<defs>${lg('#4b5563', '#2b3139')}</defs>`
        + `<rect x="6" y="9" width="36" height="30" rx="3" fill="url(#${id})"/>`
        + '<rect x="10" y="13" width="28" height="13" rx="1.5" fill="#e8edf2"/><rect x="13" y="16" width="14" height="2.4" rx="1" fill="#2563eb"/>'
        + '<rect x="13" y="20.5" width="20" height="1.6" rx=".8" fill="#9aa6b2"/><rect x="10" y="33" width="9" height="3" rx="1" fill="#1c2127"/>'
        + '<rect x="21" y="33" width="5" height="3" rx="1" fill="#1c2127"/>';
      break;
    case 'usb':
      inner = `<defs>${lg('#3f4652', '#20252c', false)}</defs>`
        + `<rect x="4" y="16" width="30" height="16" rx="3.5" fill="url(#${id})"/>`
        + '<rect x="33" y="18.5" width="11" height="11" rx="1.2" fill="#cfd6de"/><rect x="36" y="21.5" width="3" height="2" fill="#7b8590"/>'
        + '<rect x="36" y="24.5" width="3" height="2" fill="#7b8590"/><rect x="8" y="20" width="16" height="8" rx="1.5" fill="#d1272e"/>'
        + '<circle cx="28" cy="24" r="1.6" fill="#4ce07a"/>';
      break;
    case 'sd':
      inner = `<defs>${lg('#2f6bd8', '#1b47a0')}</defs>`
        + `<path d="M12 6h18l8 8v26a2.5 2.5 0 0 1-2.5 2.5h-23A2.5 2.5 0 0 1 10 40V8a2 2 0 0 1 2-2Z" fill="url(#${id})"/>`
        + '<path d="M15 6v7M19 6v7M23 6v7M27 6v7" stroke="#e4c56a" stroke-width="2"/><rect x="14" y="22" width="20" height="12" rx="1.5" fill="#fff" fill-opacity=".9"/>'
        + '<text x="24" y="31" font-family="Segoe UI, Ubuntu Sans, sans-serif" font-size="8" font-weight="700" fill="#1b47a0" text-anchor="middle">SD</text>';
      break;
    case 'optical':
      inner = `<defs><radialGradient id="${id}" cx=".35" cy=".3" r=".9"><stop offset="0" stop-color="#f5f7fa"/><stop offset=".5" stop-color="#c7d2de"/><stop offset=".75" stop-color="#e9d8f5"/><stop offset="1" stop-color="#b9c7d6"/></radialGradient></defs>`
        + `<circle cx="24" cy="24" r="19" fill="url(#${id})" stroke="#9aa9b8"/><circle cx="24" cy="24" r="6" fill="#fff" stroke="#9aa9b8"/><circle cx="24" cy="24" r="2.4" fill="#9aa9b8"/>`;
      break;
    case 'loop':
      inner = '<rect x="5" y="13" width="38" height="22" rx="4" fill="none" stroke="#7d8a99" stroke-width="2" stroke-dasharray="4 3"/>'
        + '<rect x="11" y="19" width="16" height="10" rx="1.5" fill="#7d8a99" fill-opacity=".5"/><circle cx="36" cy="29" r="1.8" fill="#7d8a99"/>';
      break;
    default:   // hdd
      inner = `<defs>${lg('#cfd5dc', '#9ea8b3')}</defs>`
        + `<rect x="5" y="7" width="38" height="34" rx="3.5" fill="url(#${id})"/>`
        + '<circle cx="21" cy="22" r="11" fill="#e9edf1" stroke="#8c97a3"/><circle cx="21" cy="22" r="2.5" fill="#8c97a3"/>'
        + '<path d="M36 12l-9 13" stroke="#5f6b78" stroke-width="2.2" stroke-linecap="round"/><circle cx="36" cy="12" r="2.4" fill="#5f6b78"/>'
        + '<rect x="9" y="35" width="30" height="3" rx="1.2" fill="#7f8a96"/><circle cx="38" cy="36.5" r="1" fill="#4ce07a"/>';
  }
  if (system) {
    inner += '<rect x="30" y="30" width="16" height="16" rx="3" fill="#0067c0" stroke="#fff" stroke-width="1.5"/>'
      + '<path d="M34 34h3.4v3.4H34zM38.6 34H42v3.4h-3.4zM34 38.6h3.4V42H34zM38.6 38.6H42V42h-3.4z" fill="#fff"/>';
  }
  return svg(size, inner);
}

/** Glyph (outline icon) for a volume row. */
export function volumeIcon(v, disk) {
  if (!v) return icon('storage', { size: 20 });
  if (v.encrypted) return icon(v.encrypted.unlocked ? 'lock-open' : 'lock', { size: 20 });
  if (disk?.kind === 'usb' || disk?.kind === 'sd') return icon('usb', { size: 20 });
  return icon('drive', { size: 20 });
}

export const kindLabel = (d) => {
  const k = t(`k_${d.kind}`);
  if (d.kind === 'hdd' && d.rotationRate > 0) return `${k} (${t('rpm', { n: number(d.rotationRate), count: d.rotationRate })})`;
  return k;
};
export const tableLabel = (d) => (d.partitionTable ? t(d.partitionTable) : t('noTable'));
export const tableLong = (d) => (d.partitionTable ? t(`${d.partitionTable}Long`) : t('noTable'));
export const busLabel = (d) => {
  if (d.kind === 'nvme') return t('bus_nvme');
  if (d.bus === 'usb') return t('bus_usb');
  if (d.bus === 'sdio') return t('bus_sdio');
  if (d.bus === 'ata' || (!d.bus && (d.kind === 'hdd' || d.kind === 'ssd'))) return t('bus_sata');
  return d.bus ? String(d.bus).toUpperCase() : t('bus_other');
};

export function healthOf(d) {
  const s = d?.smart || {};
  return s.supported ? (s.status || 'unknown') : 'unknown';
}

/** Short mount point for titles: "/" "/home" "/srv/vm" — not /run/media/<user>/<UUID> (jargon). */
export function titleWithMount(v) {
  const mp = v.mountPoint;
  if (!mp || /^\/(run\/media|media|mnt)\//.test(mp) || mp.length > 24) return v.name;
  return `${v.name} (${mp})`;
}

/** The word the user types to confirm a destructive action. */
export const confirmWord = (v) => (v.label || v.name || '').trim();

/** A UDisks job runs on the disk or on one of its volumes (format, erase …): no safe removal, no initialise. */
export const diskBusy = (d) => !!d?.busy || !!d?.volumes?.some((v) => v.busy);

export function busyText(busy) {
  if (!busy) return '';
  const op = String(busy.operation || '');
  const key = op.includes('erase') ? 'b_erase' : op.startsWith('format') ? 'b_format'
    : op.includes('delete') ? 'b_delete' : op.includes('create') ? 'b_create' : 'b_other';
  const p = typeof busy.progress === 'number' ? `${Math.round(busy.progress * 100)}%` : '';
  return t(key, { p }).trim();
}

export function volumeStatus(v) {
  if (v.busy) return busyText(v.busy);
  if (v.encrypted && !v.encrypted.unlocked) return t('s_locked');
  if (v.isSwap) return v.swapActive ? t('s_swapOn') : t('s_swapOff');
  if (!v.fsType && v.role === 'data') return t('s_raw');
  return t('s_ok');
}

export function sizeShort(n) { return bytes(n); }

/** call(method, args) → result, or null after a friendly error toast. dryRun results get a note. */
export async function run(method, args = {}, { errorTitle = t('failed'), quiet = false } = {}) {
  try {
    const r = await bridge.call(method, args);
    if (r?.dryRun && !quiet) Toast.show({ severity: 'info', title: t('dryDone'), timeout: 3500 });
    return r ?? {};
  } catch (e) {
    Toast.show({ severity: 'error', title: errorTitle, message: e?.message || '', timeout: 7000 });
    return null;
  }
}

/** Like run() but throws (dialogs show the error inline). */
export async function callOrThrow(method, args = {}) {
  return bridge.call(method, args);
}

export function shortId(id) { return String(id || '').split('/').pop(); }

/** Definition list card: [[label, value], …] (null values skipped). */
export function detailsGrid(rows) {
  return h('div', { class: 'dk-details' },
    ...rows.filter((r) => r && r[1] != null && r[1] !== '').flatMap(([k, v]) => [
      h('div', { class: 'dk-details__k' }, k),
      h('div', { class: 'dk-details__v', title: typeof v === 'string' ? v : null }, v),
    ]));
}
