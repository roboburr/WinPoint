// Win11 Shell — Verkenner: Eigenschappen (file/folder/multi/drive/Prullenbak), Delen and Opties dialogs.

import { h, clear, appendChildren } from '../../core/dom.js';
import { Dialog, Tabs, TextBox, Checkbox, Button, AppIcon } from '../../core/components.js';
import { Donut } from '../../core/charts.js';
import { number, date, time } from '../../core/format.js';
import { t } from './i18n.js';
import { fileIcon, typeName, bytesFull, dirname, basename, splitExt, copyText, volumePercent } from './util.js';

const U_basename = basename;

const longDate = (ms) => (ms ? `${date(ms, 'long')}, ${time(ms, { seconds: true })}` : '–');

function row(label, value) {
  return h('div', { class: 'xp-prop' }, h('span', { class: 'xp-prop__label' }, label),
    h('span', { class: 'xp-prop__value selectable' }, value instanceof Node ? value : (value ?? '–')));
}
const divider = () => h('div', { class: 'xp-props__sep', role: 'separator' });

/** Make the dialog's secondary button an "Apply" that does not close the dialog. */
function wireApply(promise, apply) {
  const box = promise.dialog?.el;
  const btn = box?.querySelector('[data-result="secondary"]');
  if (!btn) return null;
  btn.disabled = true;
  // Windows order: OK · Annuleren · Toepassen (the core dialog puts the secondary button in the middle)
  const cancel = box.querySelector('[data-result="close"]');
  if (cancel && cancel.parentElement === btn.parentElement) btn.parentElement.insertBefore(cancel, btn);
  box.addEventListener('click', async (e) => {
    if (!e.target.closest('[data-result="secondary"]')) return;
    e.stopPropagation(); e.preventDefault();
    if (await apply()) btn.disabled = true;
  }, true);
  return btn;
}

/* ============================================================ file / folder */

export async function showProperties(app, items) {
  if (!items?.length) return;
  if (items.length === 1 && items[0].volume) return showDriveProperties(app, items[0].volume);
  if (items.length > 1) return showMultiProperties(app, items);
  const item = items[0];
  const name = item.name || basename(item.path);
  const general = h('div', { class: 'xp-props' });
  const details = h('div', { class: 'xp-props', hidden: true });
  const tabs = Tabs({
    items: [{ id: 'general', label: t('general') }, { id: 'details', label: t('details') }],
    selected: 'general',
    onSelect: (id) => { general.hidden = id !== 'general'; details.hidden = id !== 'details'; },
  });
  const isTrash = app.tab?.loc.kind === 'trash';
  const nameBox = TextBox({ value: name, ariaLabel: t('colName'), readOnly: isTrash, onInput: () => markDirty() });
  const head = h('div', { class: 'xp-props__head' }, fileIcon(item, 32, { thumb: true }), nameBox);
  const sizeVal = h('span', null, item.isDir ? t('calculating') : bytesFull(item.size));
  const diskVal = h('span', null, item.isDir ? t('calculating') : bytesFull(item.size ? Math.ceil(item.size / 4096) * 4096 : 0));
  const containsVal = h('span', null, t('calculating'));
  const createdVal = h('span', null, '…'), modVal = h('span', null, longDate(item.modified)), accVal = h('span', null, '…');
  const roBox = Checkbox({ label: t('readOnly'), disabled: true, onChange: () => { attrDirty.readOnly = true; markDirty(); } });
  const hiddenBox = Checkbox({ label: t('hiddenAttr'), checked: name.startsWith('.') || !!item.hidden, disabled: isTrash, onChange: () => { attrDirty.hidden = true; markDirty(); } });
  const attrDirty = {};
  const opensWith = h('span', { class: 'xp-prop__app' }, '–');
  const [, ext] = splitExt(name);
  appendChildren(general, [head, divider(),
    row(t('type'), `${typeName(item)}${ext && !item.isDir ? ` (${ext})` : ''}`),
    item.isDir ? null : row(t('opensWith'), opensWith),
    divider(),
    row(t('location'), isTrash ? app.origDir(item) : dirname(item.path)),
    row(t('size'), sizeVal), item.isDir ? null : row(t('sizeOnDisk'), diskVal),
    item.isDir ? row(t('contains'), containsVal) : null,
    divider(),
    row(t('created'), createdVal), row(t('modified'), modVal), row(t('accessed'), accVal),
    divider(),
    h('div', { class: 'xp-prop' }, h('span', { class: 'xp-prop__label' }, t('attributes')),
      h('span', { class: 'xp-prop__value xp-prop__checks' }, roBox, hiddenBox))]);

  const content = h('div', { class: 'xp-propsdlg' }, tabs, general, details);
  let dirty = false;
  let applyBtn = null;
  function markDirty() { dirty = true; if (applyBtn) applyBtn.disabled = false; }

  let curPath = item.path;
  async function apply() {
    if (!dirty) return true;
    let base = nameBox.getValue().trim();
    if (!base) { nameBox.setError(t('nameEmpty')); return false; }
    if (base.includes('/')) { nameBox.setError(t('nameSlash')); return false; }
    // attributes first (host: files.setAttributes; read-only = write bit, hidden = .hidden list)
    if (attrDirty.readOnly || attrDirty.hidden) {
      const args = { path: curPath };
      if (attrDirty.readOnly) args.readOnly = roBox.checked;
      if (attrDirty.hidden && !name.startsWith('.')) args.hidden = hiddenBox.checked;
      const res = await app.setAttributes(args, { hiddenFallback: attrDirty.hidden });
      if (res === 'rename-hidden') {
        // no attribute support: hide by renaming with a leading dot (Linux convention)
        if (hiddenBox.checked && !base.startsWith('.')) base = `.${base}`;
      } else if (res !== true) { nameBox.setError(typeof res === 'string' ? res : t('errTitle')); return false; }
      if (attrDirty.hidden && !hiddenBox.checked && name.startsWith('.') && base.startsWith('.')) base = base.replace(/^\.+/, '');
      attrDirty.readOnly = false; attrDirty.hidden = false;
    }
    if (base !== U_basename(curPath)) {
      const res = await app.renameItem({ ...item, path: curPath, name: U_basename(curPath) }, base);
      if (res !== true) { nameBox.setError(typeof res === 'string' ? res : t('errTitle')); return false; }
      curPath = `${dirname(curPath)}/${base}`.replace(/^\/\//, '/');
    }
    nameBox.setError('');
    dirty = false;
    return true;
  }

  const p = Dialog.show({
    title: t('propsTitle', { name }), content, primary: t('ok'), secondary: isTrash ? undefined : t('apply'), close: t('cancel'),
    width: 460, onPrimary: apply,
  });
  applyBtn = isTrash ? null : wireApply(p, apply);

  // fill in what the host knows
  app.call('files.properties', { path: item.path }).then((pr) => {
    if (!pr) return;
    if (pr.size != null) { sizeVal.textContent = bytesFull(pr.size); diskVal.textContent = bytesFull(Math.ceil(pr.size / 4096) * 4096); }
    if (pr.contains) containsVal.textContent = t('containsVal', { files: number(pr.contains.files || 0), folders: number(pr.contains.folders || 0) });
    else containsVal.textContent = '–';
    createdVal.textContent = longDate(pr.created);
    modVal.textContent = longDate(pr.modified || item.modified);
    accVal.textContent = longDate(pr.accessed);
    roBox.setChecked(!!pr.readOnly);
    if (!isTrash && pr.owner && app.S.user?.name && pr.owner === app.S.user.name) roBox.setDisabled(false);
    if (pr.hidden != null) hiddenBox.setChecked(!!pr.hidden);
    appendChildren(clear(details), [
      row(t('colName'), pr.displayName || pr.name || name), row(t('type'), pr.type || typeName(item)),
      row(t('mime'), pr.mime || item.mime || '–'), row(t('fullPath'), pr.path || item.path),
      row(t('size'), pr.size != null ? bytesFull(pr.size) : '–'),
      row(t('owner'), pr.owner || '–'), row(t('permissions'), pr.permissions || '–'),
      row(t('created'), longDate(pr.created)), row(t('modified'), longDate(pr.modified)), row(t('accessed'), longDate(pr.accessed))]);
  }).catch((e) => {
    sizeVal.textContent = item.isDir ? '–' : sizeVal.textContent;
    containsVal.textContent = '–'; createdVal.textContent = '–'; accVal.textContent = '–';
    details.append(row(t('fullPath'), item.path));
    console.warn('[explorer] properties failed', e);
  });
  if (!item.isDir && !isTrash) {
    app.call('apps.forFile', { path: item.path }).then((apps) => {
      const def = (apps || []).find((a) => a.isDefault) || (apps || [])[0];
      if (def) clear(opensWith).append(AppIcon({ name: def.icon, size: 16 }), h('span', null, def.name));
    }).catch(() => {});
  }
  return p;
}

async function showMultiProperties(app, items) {
  const types = new Set(items.map((x) => typeName(x)));
  const dirs = new Set(items.map((x) => dirname(x.path)));
  const files = items.filter((x) => !x.isDir).length;
  const folders = items.length - files;
  let total = items.reduce((a, x) => a + (x.isDir ? 0 : (x.size || 0)), 0);
  const sizeVal = h('span', null, folders ? t('calculating') : bytesFull(total));
  const content = h('div', { class: 'xp-propsdlg' }, h('div', { class: 'xp-props' },
    h('div', { class: 'xp-props__head' }, fileIcon(items[0], 32), h('span', { class: 'body-strong' },
      t('containsVal', { files: number(files), folders: number(folders) }))),
    divider(),
    row(t('type'), types.size === 1 ? t('allOfType', { type: [...types][0] }) : t('multiTypes')),
    row(t('location'), dirs.size === 1 ? t('allIn', { dir: [...dirs][0] }) : t('variousFolders')),
    row(t('size'), sizeVal)));
  const p = Dialog.show({ title: t('propsTitle', { name: t('multiName', { n: items.length }) }), content, close: t('ok'), width: 460 });
  if (folders) {
    // the host can total a whole selection in one call; otherwise ask per folder
    const multi = await app.call('files.properties', { paths: items.map((x) => x.path) }).catch(() => null);
    if (multi && typeof multi.size === 'number' && multi.count) total = multi.size;
    else {
      const res = await Promise.all(items.filter((x) => x.isDir).slice(0, 50).map((x) => app.call('files.properties', { path: x.path }).catch(() => null)));
      total += res.reduce((a, r) => a + (r?.size || 0), 0);
    }
    sizeVal.textContent = bytesFull(total);
  }
  return p;
}

/* ============================================================== drives */

export function showDriveProperties(app, v) {
  const pct = volumePercent(v);
  const usedColor = pct >= 90 ? 'var(--sys-critical)' : 'var(--accent)';
  const donut = Donut({
    segments: [{ value: v.used, color: usedColor, label: t('used') }, { value: v.free, color: 'var(--chart-track)', label: t('free') }],
    size: 132, thickness: 18, label: `${Math.round(pct)}%`, sublabel: t('used').toLowerCase(),
  });
  const swatch = (c) => h('span', { class: 'xp-swatch', style: { background: c } });
  const content = h('div', { class: 'xp-propsdlg' }, h('div', { class: 'xp-props' },
    h('div', { class: 'xp-props__head' }, app.namedIcon(app.volIcon(v), 32, 'drive'), h('span', { class: 'body-strong' }, app.volLabel(v))),
    divider(),
    row(t('type'), v.removable ? t('removable') : t('localDisk')),
    row(t('fsType'), (v.fsType || '–').toUpperCase()),
    row(t('mount'), v.mount),
    divider(),
    h('div', { class: 'xp-prop' }, h('span', { class: 'xp-prop__label' }, swatch(usedColor), t('used')), h('span', { class: 'xp-prop__value' }, bytesFull(v.used))),
    h('div', { class: 'xp-prop' }, h('span', { class: 'xp-prop__label' }, swatch('var(--chart-track)'), t('free')), h('span', { class: 'xp-prop__value' }, bytesFull(v.free))),
    divider(),
    row(t('capacity'), bytesFull(v.total)),
    h('div', { class: 'xp-props__donut' }, donut,
      Button({ label: t('freeUp'), icon: 'broom', onClick: () => { p.dialog?.close('close'); app.call('shell.openApp', { app: 'settings', params: { page: 'storage' } }).catch(() => {}); } }))));
  const p = Dialog.show({ title: t('driveTitle', { name: app.volLabel(v) }), content, close: t('ok'), width: 460 });
  return p;
}

export async function showTrashProperties(app) {
  const info = await app.call('files.trashInfo', {}).catch(() => null);
  const content = h('div', { class: 'xp-propsdlg' }, h('div', { class: 'xp-props' },
    h('div', { class: 'xp-props__head' }, app.namedIcon(info?.count ? 'user-trash-full' : 'user-trash', 32, 'trash'), h('span', { class: 'body-strong' }, t('trash'))),
    divider(),
    row(t('trashSize'), info ? t('items', { n: info.count || 0 }) : '–'),
    row(t('size'), info ? bytesFull(info.size || 0) : '–')));
  return Dialog.show({ title: t('trashTitle'), content, close: t('ok'), width: 420 });
}

/* =============================================================== share */

export function showShare(app, items) {
  const paths = items.map((x) => x.path);
  const list = h('div', { class: 'xp-share__list' }, items.slice(0, 6).map((x) => h('div', { class: 'xp-share__item' }, fileIcon(x, 20), h('span', { class: 'ellipsis' }, x.name))));
  const content = h('div', { class: 'xp-share' }, h('p', { class: 'text-2' }, t('shareMsg')), list);
  return Dialog.show({ title: t('shareTitle'), content, primary: t('copyPathBtn'), close: t('close'), width: 440 })
    .then(async (r) => {
      if (r === 'primary') { await copyText(paths.map((p) => `"${p}"`).join('\n')); app.toast(t('pathCopied', { n: paths.length })); }
    });
}

/* ============================================================= options */

export function showOptions(app) {
  const opt = (label, key, def) => Checkbox({ label, checked: !!app.cfg(key, def), onChange: (v) => app.setCfg(key, v) });
  const recent = Checkbox({
    label: t('optRecent'), checked: app.privacyRecent(),
    onChange: (v) => app.setSetting('privacy.recentFiles', v),
  });
  const content = h('div', { class: 'xp-options' },
    h('p', { class: 'text-2' }, t('optHint')),
    opt(t('hidden'), 'showHidden', false), opt(t('extensions'), 'showExtensions', true),
    opt(t('checkboxes'), 'checkboxes', false), opt(t('compact'), 'compact', false),
    opt(t('navPane'), 'navPane', true), opt(t('detailsPane'), 'detailsPane', false), recent);
  return Dialog.show({ title: t('optionsTitle'), content, close: t('close'), width: 420 });
}

