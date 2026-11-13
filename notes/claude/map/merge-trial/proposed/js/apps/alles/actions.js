// Alles — actions on results: open, open containing folder, copy, properties, trash.

import { h } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { Dialog, Toast, Checkbox } from '../../core/components.js';
import { bytes, number, fileTime } from '../../core/format.js';
import { t, typeName } from './i18n.js';

const fail = (title) => (e) => {
  if (e?.code === 'cancelled') return;
  Toast.show({ title, message: e?.message || '', severity: 'error' });
};

/** Folders: files.open — the host opens them in our Verkenner (also in dry-run). The in-browser mock
 *  cannot open windows from files.open, so there we ask the shell directly. */
function openFolder(path) {
  if (bridge.isMock) return bridge.call('shell.openApp', { app: 'explorer', params: { path } }).catch(fail(t('openFailed')));
  return bridge.call('files.open', { path }).catch(fail(t('openFailed')));
}

export function openItem(item) {
  if (!item) return Promise.resolve();
  if (item.isDir) return openFolder(item.path);
  return bridge.call('files.open', { path: item.path }).catch(fail(t('openFailed')));
}

/** Open the folder that contains the item in Verkenner, with the item selected. */
export function openContaining(item) {
  if (!item) return Promise.resolve();
  return bridge.call('shell.openApp', { app: 'explorer', params: { path: item.dir || '/', select: item.path } })
    .catch(fail(t('openFailed')));
}

/** Folders open themselves in Verkenner; files open their folder. */
export function openInExplorer(item) {
  if (!item) return Promise.resolve();
  if (!item.isDir) return openContaining(item);
  return openFolder(item.path);
}

export async function copyText(text) {
  try {
    if (navigator.clipboard?.writeText) { await navigator.clipboard.writeText(text); return true; }
  } catch { /* fall back below */ }
  try {
    const ta = h('textarea', { style: { position: 'fixed', left: '-9999px', top: '0', opacity: '0' } });
    ta.value = text;
    document.body.append(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  } catch { return false; }
}

export async function copyPaths(items, { names = false, capped = false } = {}) {
  if (!items.length) return;
  const text = items.map((i) => (names ? i.name : i.path)).join('\n');
  const ok = await copyText(text);
  if (!ok) { Toast.show({ title: t('copyFailed'), severity: 'error' }); return; }
  Toast.show({
    title: t(names ? 'namesCopied' : 'copied', { n: items.length, c: number(items.length) }),
    message: capped ? t('copyCapped', { n: number(items.length), count: items.length }) : (items.length === 1 ? (names ? items[0].name : items[0].path) : ''),
    severity: 'success', timeout: 2600,
  });
}

/* ------------------------------------------------------------- properties */
function propRow(label, value) {
  return h('div', { class: 'alles-props__row' }, h('dt', null, label), h('dd', { class: 'selectable' }, value ?? '–'));
}

export async function showProperties(item) {
  if (!item) return;
  let p = null;
  let partial = false;
  try { p = await bridge.call('files.properties', { path: item.path }); } catch { partial = true; }
  p = p || {};
  const size = p.size ?? item.size;
  const px = Math.round(32 * Math.min(2, Math.max(1, devicePixelRatio || 1)));
  const url = item.isDir ? bridge.iconUrl('folder', px) : bridge.mimeIconUrl(item.mime || 'application/octet-stream', px);
  const img = h('img', { src: url, width: 32, height: 32, alt: '', class: 'alles-props__icon' });
  img.addEventListener('error', () => img.replaceWith(icon(item.isDir ? 'folder' : 'file', { size: 32 })), { once: true });
  const ro = Checkbox({ label: t('pr_readonly'), checked: !!p.readOnly, disabled: true });
  const hid = Checkbox({ label: t('pr_hidden'), checked: !!(p.hidden ?? item.hidden), disabled: true });
  const contains = p.contains ? t('pr_containsVal', { files: number(p.contains.files), folders: number(p.contains.folders) }) : null;
  const content = h('div', { class: 'alles-props' },
    h('div', { class: 'alles-props__head' }, img, h('div', { class: 'alles-props__name selectable' }, item.name)),
    h('hr', { class: 'alles-props__sep' }),
    h('dl', { class: 'alles-props__list' },
      propRow(t('pr_type'), p.type || typeName(item)),
      propRow(t('pr_location'), item.dir || '/'),
      propRow(t('pr_size'), size == null ? '–' : (size < 1024 ? bytes(size) : t('pr_sizeBytes', { h: bytes(size), b: number(size) }))),
      contains ? propRow(t('pr_contains'), contains) : null),
    h('hr', { class: 'alles-props__sep' }),
    h('dl', { class: 'alles-props__list' },
      p.created ? propRow(t('pr_created'), fileTime(p.created)) : null,
      propRow(t('pr_modified'), fileTime(p.modified ?? item.modified)),
      p.accessed ? propRow(t('pr_accessed'), fileTime(p.accessed)) : null,
      p.owner ? propRow(t('pr_owner'), p.owner) : null,
      p.permissions ? propRow(t('pr_perms'), h('span', { class: 'mono' }, p.permissions)) : null),
    h('hr', { class: 'alles-props__sep' }),
    h('div', { class: 'alles-props__attrs' }, h('span', { class: 'alles-props__attrlabel' }, t('pr_attrs')), ro, hid),
    partial ? h('p', { class: 'alles-props__note' }, t('pr_partial')) : null);
  const pr = Dialog.show({ title: t('pr_title', { name: item.name }), content, close: t('pr_ok') });
  pr.dialog?.el.classList.add('alles-props-dialog');
  await pr;
}

/* ------------------------------------------------------------------- trash */
const TRASH_MAX = 500;

/** Confirm, then move to the trash. Resolves true when something was moved. */
export async function trashItems(items) {
  if (!items.length) return false;
  if (items.length > TRASH_MAX) {
    await Dialog.alert({ title: t('trash'), message: t('trashTooMany', { n: TRASH_MAX }) });
    return false;
  }
  const one = items.length === 1;
  const message = h('div', { class: 'alles-confirm' },
    h('p', null, one ? t('trashBodyOne', { name: items[0].name }) : t('trashBodyMany', { n: number(items.length), count: items.length })),
    items.some((i) => i.isDir) ? h('p', { class: 'alles-confirm__note' }, t('trashBodyFolder')) : null);
  const ok = await Dialog.confirm({ title: t('trashTitle', { n: items.length, c: number(items.length) }), content: message, confirmLabel: t('trashConfirm'), danger: true });
  if (!ok) return false;
  try {
    const r = await bridge.call('files.trash', { paths: items.map((i) => i.path) });
    const failed = Array.isArray(r?.failed) ? r.failed.length : 0;
    if (failed) {
      Toast.show({ title: t('trashFailed'), message: t('trashFailedBody', { n: failed }), severity: 'warning' });
    } else {
      Toast.show({ title: t('trashed', { n: items.length, c: number(items.length) }), message: r?.dryRun ? t('dryRun') : '', severity: 'success', timeout: 3500 });
    }
    return failed < items.length;
  } catch (e) {
    fail(t('trashFailed'))(e);
    return false;
  }
}
