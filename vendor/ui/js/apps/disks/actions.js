// Schijven — what the buttons and menus do: dialogs (Windows wording) + calls to disks.*.
// Destructive ones (formatteren, verwijderen, initialiseren) explain that data is lost and need the volume's (or
// disk's) name typed before the red button works; Enter in those dialogs means Cancel, and while the operation runs
// Esc does not close them. The host checks again: system volumes are refused there too, and `expect {device, size}`
// makes sure the disk did not change meanwhile.

import { h, clear } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { bytes, number } from '../../core/format.js';
import { locale } from '../../core/i18n.js';
import { Dialog, TextBox, Dropdown, Checkbox, RadioGroup, InfoBar, ProgressBar, Toast, Slider } from '../../core/components.js';
import { showContextMenu } from '../../core/contextmenu.js';
import { t } from './i18n.js';
import { run, confirmWord, busyText, diskBusy } from './util.js';
import { parseLocaleNumber } from './numparse.js';

const GB = 1024 ** 3;
const expectOf = (o) => ({ device: o.device, size: o.size, ...(o.uuid ? { uuid: o.uuid } : {}) });

/* ------------------------------------------------------------------------------------------- helpers */

function primaryButton(promise) {
  return promise.dialog?.el?.querySelector('[data-result="primary"]') || null;
}

/** An inline error area for dialogs. */
function errorSlot() {
  const slot = h('div', { class: 'dk-dlg__error' });
  slot.show = (e) => { clear(slot).append(InfoBar({ severity: 'error', title: t('failed'), message: e?.message || String(e || ''), closable: false })); };
  slot.clear = () => clear(slot);
  return slot;
}

/** A progress line that follows disks.changed for one object while an operation runs. */
function progressSlot(objectId) {
  const bar = ProgressBar({ indeterminate: true, ariaLabel: t('op_other') });
  const text = h('div', { class: 'dk-dlg__progress-text' });
  const slot = h('div', { class: 'dk-dlg__progress', hidden: true }, text, bar);
  let off = null;
  slot.start = (label) => {
    slot.hidden = false;
    text.textContent = label;
    off = bridge.on('disks.changed', (snap) => {
      for (const d of snap?.disks || []) {
        const hit = d.block === objectId || d.id === objectId ? d : d.volumes.find((v) => v.id === objectId);
        if (hit?.busy) {
          if (typeof hit.busy.progress === 'number' && hit.busy.progress > 0) bar.setValue(Math.round(hit.busy.progress * 100));
          text.textContent = busyText(hit.busy);
        }
      }
    });
  };
  slot.stop = () => { off?.(); off = null; slot.hidden = true; bar.setIndeterminate(true); };
  return slot;
}

/** Typed-name confirmation field. onValid(bool) whenever it changes. */
function confirmField(word, onValid, { disk = false } = {}) {
  const tb = TextBox({
    header: t('confirmType', { name: word }), description: t(disk ? 'confirmTypeDescDisk' : 'confirmTypeDesc'),
    onInput: (v) => onValid(v.trim().toLocaleLowerCase() === word.toLocaleLowerCase()),
  });
  tb.classList.add('dk-confirm');
  tb.isValid = () => tb.getValue().trim().toLocaleLowerCase() === word.toLocaleLowerCase();
  return tb;
}

function fsOptions(snap) {
  return (snap?.filesystems || []).map((f) => ({
    value: f.id, label: f.available ? f.name : t('fsMissing', { fs: f.name }), disabled: !f.available,
  }));
}

/** File-system dropdown with the description of the chosen one below it (Windows lists only names). */
function fsPicker(snap, value, onChange) {
  const desc = h('div', { class: 'dk-fs-desc' });
  const show = (id) => { desc.textContent = (snap?.filesystems || []).find((f) => f.id === id)?.description || ''; };
  const dd = Dropdown({ header: t('fsField'), options: fsOptions(snap), value, width: '100%', onChange: (x) => { show(x); onChange(x); } });
  show(value);
  return { dd, desc };
}

function defaultFs(snap, disk) {
  const ok = (id) => (snap?.filesystems || []).find((f) => f.id === id && f.available);
  const removable = disk && (disk.removable || disk.kind === 'usb' || disk.kind === 'sd');
  for (const id of removable ? ['exfat', 'ntfs', 'vfat', 'ext4'] : ['ext4', 'ntfs', 'exfat']) if (ok(id)) return id;
  return (snap?.filesystems || []).find((f) => f.available)?.id || 'ext4';
}

function labelLimit(snap, fs) {
  const f = (snap?.filesystems || []).find((x) => x.id === fs);
  return f ? f.maxLabel : 32;
}

/** Password pair (with checks); returns {el, value(), valid()}. */
function passwordPair(onChange) {
  const p1 = TextBox({ header: t('pw1'), type: 'password', onInput: () => onChange() });
  const p2 = TextBox({ header: t('pw2'), type: 'password', onInput: () => onChange() });
  const el = h('div', { class: 'dk-dlg__pw', hidden: true }, p1, p2);
  return {
    el, p1, p2,
    value: () => p1.getValue(),
    problem() {
      if (p1.getValue().length < 8) return t('pwShort');
      if (p1.getValue() !== p2.getValue()) return t('pwMismatch');
      return '';
    },
  };
}

/* ------------------------------------------------------------------------------------------- simple actions */

export async function mount(ctx, v) {
  const r = await run('disks.mount', { id: v.id });
  if (r && !r.dryRun) Toast.show({ severity: 'success', title: t('mounted', { name: v.name }), message: r.mountPoint || '', timeout: 3000 });
  ctx.reload();
}

export async function unmount(ctx, v) {
  const r = await run('disks.unmount', { id: v.id });
  if (r && !r.dryRun) Toast.show({ severity: 'success', title: t('unmounted', { name: v.name }), timeout: 3000 });
  ctx.reload();
}

export async function openInExplorer(ctx, v) {
  await run('disks.open', { id: v.id }, { quiet: true });
  ctx.reload();
}

export async function lock(ctx, v) {
  const r = await run('disks.lock', { id: v.id });
  if (r && !r.dryRun) Toast.show({ severity: 'success', title: t('locked', { name: v.name }), timeout: 3000 });
  ctx.reload();
}

export async function safeRemove(ctx, d) {
  const r = await run(d.canPowerOff || !d.ejectable ? 'disks.powerOff' : 'disks.eject', { disk: d.id });
  if (r && !r.dryRun) {
    Toast.show({ severity: 'success', title: t('safeRemoved', { name: d.model }), message: t('safeRemovedBody'), timeout: 6000 });
    ctx.diskGone?.(d.id);
  }
  ctx.reload();
}

export async function eject(ctx, d) {
  const r = await run('disks.eject', { disk: d.id });
  if (r && !r.dryRun) { Toast.show({ severity: 'success', title: t('ejected', { name: d.model }), timeout: 4000 }); ctx.diskGone?.(d.id); }
  ctx.reload();
}

/* ------------------------------------------------------------------------------------------- dialogs */

export async function changeLabel(ctx, v) {
  const fs = v.fsType;
  const limit = labelLimit(ctx.snap, fs);
  const err = errorSlot();
  const tb = TextBox({
    header: t('labelField'), value: v.label || '', maxLength: limit,
    description: fs === 'vfat' ? t('labelHintFat') : t('labelHint', { n: limit }),
  });
  if (fs === 'vfat') tb.input.style.textTransform = 'uppercase';
  const note = v.labelRemount ? InfoBar({ severity: 'info', title: '', message: t('labelRemount'), closable: false }) : null;
  const content = h('div', { class: 'dk-dlg' }, tb, note, err);
  const p = Dialog.show({
    title: t('labelTitle'), content, primary: t('save'), close: t('cancel'),
    onPrimary: async () => {
      err.clear();
      try {
        const r = await bridge.call('disks.setLabel', { id: v.id, label: tb.getValue() });
        if (r?.dryRun) Toast.show({ severity: 'info', title: t('dryDone'), timeout: 3500 });
        else Toast.show({ severity: r?.mountError ? 'warning' : 'success', title: t('labelDone'), message: r?.mountError || '', timeout: r?.mountError ? 7000 : 3000 });
        return true;
      } catch (e) { err.show(e); return false; }
    },
  });
  setTimeout(() => tb.select(), 50);
  await p;
  ctx.reload();
}

/** Windows' "Formatteren" dialog + typed-name confirmation (+ optional LUKS). */
export async function format(ctx, d, v, { openOnly = false } = {}) {
  const snap = ctx.snap;
  const word = confirmWord(v);
  let fs = (snap?.filesystems || []).find((f) => f.id === v.fsType && f.available) ? v.fsType : defaultFs(snap, d);
  const err = errorSlot();
  const prog = progressSlot(v.id);
  const label = TextBox({ header: t('labelField'), value: v.label || (v.fsType ? '' : t('newVolumeLabel')), maxLength: labelLimit(snap, fs) });
  const fsp = fsPicker(snap, fs, (x) => { fs = x; label.input.maxLength = labelLimit(snap, fs); validate(); });
  const quick = Checkbox({ checked: true, label: t('quick'), description: t('quickDesc') });
  const pw = passwordPair(() => validate());
  const enc = Checkbox({
    checked: false, label: t('encrypt'), description: t('encryptDesc'),
    onChange: (on) => { pw.el.hidden = !on; validate(); },
  });
  if (!snap?.encryption?.supported) enc.hidden = true;
  let typedOk = false;
  const confirm = confirmField(word, (ok) => { typedOk = ok; validate(); });
  const content = h('div', { class: 'dk-dlg' },
    InfoBar({ severity: 'warning', title: '', message: t('formatWarn'), closable: false }),
    h('div', { class: 'dk-dlg__grid' }, label, h('div', { class: 'dk-dlg__fs' }, fsp.dd, fsp.desc)), quick, enc, pw.el, confirm, err, prog);
  let p;
  function validate() {
    const btn = p && primaryButton(p);
    const pwProblem = enc.checked ? pw.problem() : '';
    pw.p2.setError?.(enc.checked && pw.p2.getValue() ? pwProblem : '');
    if (btn) btn.disabled = !(typedOk && !pwProblem);
  }
  p = Dialog.show({
    title: t('formatTitle', { name: v.name }), content, primary: t('formatBtn'), close: t('cancel'), danger: true, width: 520,
    onPrimary: async () => {
      if (!confirm.isValid()) return false;
      err.clear();
      prog.start(t('formatting'));
      try {
        const r = await bridge.call('disks.format', {
          id: v.id, fsType: fs, label: label.getValue(), quick: quick.checked, expect: expectOf(v),
          mount: v.mounted || d.removable, ...(enc.checked ? { encrypt: { passphrase: pw.value() } } : {}),
        });
        prog.stop();
        if (r?.dryRun) Toast.show({ severity: 'info', title: t('dryDone'), timeout: 3500 });
        else Toast.show({ severity: 'success', title: t('formatDone', { name: label.getValue() || v.name }), message: r?.mountError || '', timeout: 4000 });
        return true;
      } catch (e) { prog.stop(); err.show(e); validate(); return false; }
    },
  });
  setTimeout(validate, 0);
  if (openOnly) return p;
  await p;
  ctx.reload();
  return null;
}

export async function deleteVolume(ctx, d, v) {
  const word = confirmWord(v);
  const err = errorSlot();
  const prog = progressSlot(v.id);
  let p;
  const confirm = confirmField(word, (ok) => { const b = primaryButton(p); if (b) b.disabled = !ok; });
  const warnings = [h('p', { class: 'dk-dlg__text' }, t('deleteWarn'))];
  if (d.dynamic) warnings.push(InfoBar({ severity: 'warning', title: t('dynamicTitle'), message: t('dynamicBody'), closable: false }));
  const content = h('div', { class: 'dk-dlg' }, ...warnings, confirm, err, prog);
  p = Dialog.show({
    title: t('deleteTitle', { name: v.name }), content, primary: t('deleteBtn'), close: t('cancel'), danger: true, width: 500,
    onPrimary: async () => {
      if (!confirm.isValid()) return false;
      err.clear();
      prog.start(t('op_delete'));
      try {
        const r = await bridge.call('disks.deletePartition', { id: v.id, expect: expectOf(v) });
        prog.stop();
        Toast.show({ severity: r?.dryRun ? 'info' : 'success', title: r?.dryRun ? t('dryDone') : t('deleteDone'), timeout: 3500 });
        if (!r?.dryRun) ctx.volumeGone?.(v.id);
        return true;
      } catch (e) { prog.stop(); err.show(e); return false; }
    },
  });
  setTimeout(() => { const b = primaryButton(p); if (b) b.disabled = true; }, 0);
  await p;
  ctx.reload();
}

/** "Nieuw eenvoudig volume" in a free region. */
export async function newVolume(ctx, d, region, { openOnly = false } = {}) {
  const snap = ctx.snap;
  let fs = defaultFs(snap, d);
  const maxGb = region.size / GB;
  const err = errorSlot();
  const prog = progressSlot(d.block);
  const round = (x) => Math.max(0.01, Math.floor(x * 100) / 100);
  let sizeGb = round(maxGb);
  const sizeBox = TextBox({
    header: t('sizeField'), value: number(sizeGb, 2), description: t('sizeHint', { max: `${number(sizeGb, 2)} GB` }),
    onInput: (val) => {
      const n = parseLocaleNumber(val, locale());
      sizeGb = isFinite(n) && n > 0 ? Math.min(n, maxGb) : 0;
      slider.setValue?.(Math.round((sizeGb / maxGb) * 1000));
      validate();
    },
  });
  const slider = Slider({
    min: 1, max: 1000, step: 1, value: 1000, ariaLabel: t('sizeField'), format: (pm) => bytes((pm / 1000) * region.size),
    onInput: (pm) => { sizeGb = round((pm / 1000) * maxGb); sizeBox.setValue(number(sizeGb, 2)); validate(); },
  });
  const label = TextBox({ header: t('labelField'), value: t('newVolumeLabel'), maxLength: labelLimit(snap, fs) });
  const fsp = fsPicker(snap, fs, (x) => { fs = x; label.input.maxLength = labelLimit(snap, fs); });
  const pw = passwordPair(() => validate());
  const enc = Checkbox({ checked: false, label: t('encrypt'), description: t('encryptDesc'), onChange: (on) => { pw.el.hidden = !on; validate(); } });
  if (!snap?.encryption?.supported) enc.hidden = true;
  const content = h('div', { class: 'dk-dlg' },
    h('p', { class: 'dk-dlg__text' }, t(region.logical ? 'newIntroLogical' : 'newIntro', { disk: `${d.name} (${d.model})` })),
    h('div', { class: 'dk-dlg__size' }, sizeBox, slider),
    h('div', { class: 'dk-dlg__grid' }, label, h('div', { class: 'dk-dlg__fs' }, fsp.dd, fsp.desc)), enc, pw.el, err, prog);
  let p;
  function validate() {
    const b = p && primaryButton(p);
    const bad = sizeGb * GB < 8 * 1024 * 1024 || (enc.checked && pw.problem());
    pw.p2.setError?.(enc.checked && pw.p2.getValue() ? pw.problem() : '');
    if (b) b.disabled = !!bad;
  }
  p = Dialog.show({
    title: t('newTitle'), content, primary: t('createBtn'), close: t('cancel'), width: 520,
    onPrimary: async () => {
      err.clear();
      prog.start(t('creating'));
      try {
        const size = sizeGb >= maxGb - 0.005 ? 0 : Math.round(sizeGb * GB);
        const r = await bridge.call('disks.createPartition', {
          disk: d.id, offset: region.offset, size, fsType: fs, label: label.getValue(), expect: expectOf(d),
          ...(enc.checked ? { encrypt: { passphrase: pw.value() } } : {}),
        });
        prog.stop();
        if (r?.dryRun) Toast.show({ severity: 'info', title: t('dryDone'), timeout: 3500 });
        else Toast.show({ severity: 'success', title: t('createDone', { name: label.getValue() || t('newVolumeLabel') }), message: r?.mountPoint || r?.mountError || '', timeout: 4000 });
        return true;
      } catch (e) { prog.stop(); err.show(e); return false; }
    },
  });
  setTimeout(validate, 0);
  if (openOnly) return p;
  await p;
  ctx.reload();
  return null;
}

export async function unlock(ctx, v, { openOnly = false } = {}) {
  const err = errorSlot();
  const pwBox = TextBox({ header: t('pw1'), type: 'password', autofocus: true });
  const content = h('div', { class: 'dk-dlg' }, h('p', { class: 'dk-dlg__text' }, t('unlockIntro')), pwBox, err);
  const p = Dialog.show({
    title: t('unlockTitle', { name: v.name }), content, primary: t('unlockBtn'), close: t('cancel'), width: 440,
    onPrimary: async () => {
      err.clear();
      const pass = pwBox.getValue();
      if (!pass) { pwBox.focus(); return false; }
      try {
        const r = await bridge.call('disks.unlock', { id: v.id, passphrase: pass });
        if (r?.dryRun) Toast.show({ severity: 'info', title: t('dryDone'), timeout: 3500 });
        else Toast.show({ severity: 'success', title: t('unlockDone', { name: v.name }), message: r?.mountPoint || '', timeout: 3500 });
        return true;
      } catch (e) {
        err.show(e);
        pwBox.setValue('');
        pwBox.focus();
        return false;
      }
    },
  });
  pwBox.input.addEventListener('keydown', (e) => { if (e.key === 'Enter') { e.preventDefault(); primaryButton(p)?.click(); } });
  if (openOnly) return p;
  await p;
  ctx.reload();
  return null;
}

export async function initialize(ctx, d) {
  const err = errorSlot();
  const prog = progressSlot(d.block);
  let table = 'gpt';
  const radios = RadioGroup({
    value: 'gpt', onChange: (x) => { table = x; },
    options: [{ value: 'gpt', label: t('initGpt'), description: t('initGptDesc') }, { value: 'dos', label: t('initDos'), description: t('initDosDesc') }],
  });
  // Always destructive: "no volumes" only means Ubuntu recognises nothing (VeraCrypt, Storage Spaces, a RAID
  // member, a raw image …). So: the warning, the typed disk name, and Enter = Cancel — also for an "empty" disk.
  const hasData = d.volumes.length > 0;
  let p;
  const confirm = confirmField(d.name, (ok) => { const b = primaryButton(p); if (b) b.disabled = !ok; }, { disk: true });
  const content = h('div', { class: 'dk-dlg' },
    h('p', { class: 'dk-dlg__text' }, t('initIntro')), radios,
    InfoBar({ severity: 'warning', title: '', message: hasData ? t('initWarn') : `${t('initWarn')} ${t('initWarnUnknown')}`, closable: false }),
    confirm, err, prog);
  p = Dialog.show({
    title: t('initTitle', { disk: d.name }), content, primary: t('initBtn'), close: t('cancel'), danger: true, width: 520,
    onPrimary: async () => {
      if (!confirm.isValid()) return false;
      err.clear();
      prog.start(t('op_other'));
      try {
        const r = await bridge.call('disks.initialize', { disk: d.id, table, expect: expectOf(d) });
        prog.stop();
        Toast.show({ severity: r?.dryRun ? 'info' : 'success', title: r?.dryRun ? t('dryDone') : t('initDone', { disk: d.name }), timeout: 3500 });
        return true;
      } catch (e) { prog.stop(); err.show(e); return false; }
    },
  });
  setTimeout(() => { const b = primaryButton(p); if (b) b.disabled = true; }, 0);
  await p;
  ctx.reload();
}

/* ------------------------------------------------------------------------------------------- menus */

/** Context menu items for a volume (bar segment / row). */
export function volumeMenu(ctx, d, v) {
  const enc = v.encrypted;
  const locked = enc && !enc.unlocked;
  const busy = !!v.busy;
  return [
    { label: t('open'), icon: 'folder-open', disabled: busy || locked || !(v.mounted || v.canMount), action: () => openInExplorer(ctx, v) },
    v.mounted
      ? { label: t('unmount'), icon: 'external', disabled: busy || !v.canUnmount, action: () => unmount(ctx, v) }
      : { label: t('mount'), icon: 'link', disabled: busy || !v.canMount, action: () => mount(ctx, v) },
    enc ? (locked ? { label: `${t('unlock')}…`, icon: 'lock-open', disabled: busy, action: () => unlock(ctx, v) }
      : { label: t('lock'), icon: 'lock', disabled: busy || v.isSystem, action: () => lock(ctx, v) }) : null,
    { separator: true },
    { label: `${t('changeLabel')}…`, icon: 'rename', disabled: busy || !v.canLabel, action: () => changeLabel(ctx, v) },
    { label: `${t('format')}…`, icon: 'storage', disabled: busy || !v.canFormat, danger: true, action: () => format(ctx, d, v) },
    { label: `${t('deleteVolume')}…`, icon: 'delete', disabled: busy || !v.canDelete, danger: true, action: () => deleteVolume(ctx, d, v) },
    { separator: true },
    { label: t('properties'), icon: 'properties', action: () => ctx.openVolume(v.id) },
  ].filter(Boolean);
}

/** May a free region (or a not-initialised disk) get a new volume / be initialised right now? */
export const canInit = (d) => !!d.canInitialize && !diskBusy(d);
export const canCreateIn = (d, region) => (region?.canCreate ?? d.canPartition) && !diskBusy(d);

export function freeMenu(ctx, d, region) {
  if (!d.partitionTable) return [{ label: `${t('initialize')}…`, icon: 'disk', disabled: !canInit(d), danger: true, action: () => initialize(ctx, d) }];
  return [{ label: t('newVolumeDots'), icon: 'plus', disabled: !canCreateIn(d, region), action: () => newVolume(ctx, d, region) }];
}

/** Double-click / Enter on a free segment: the first action when it is allowed, else the menu (it says why). */
export function freeDefault(ctx, d, region, e, anchor) {
  const items = freeMenu(ctx, d, region);
  if (items[0] && !items[0].disabled) items[0].action();
  else openMenu(e, items, anchor);
}

export function diskMenu(ctx, d) {
  const busy = diskBusy(d);
  return [
    d.removable ? { label: t('safeRemove'), icon: 'usb', disabled: busy || !d.canEject, action: () => safeRemove(ctx, d) } : null,
    d.mediaRemovable || d.kind === 'optical' ? { label: t('eject'), icon: 'arrow-up', disabled: busy || !d.canEject, action: () => eject(ctx, d) } : null,
    d.kind !== 'optical' ? { label: `${d.partitionTable ? t('reinitialize') : t('initialize')}…`, icon: 'disk', disabled: !canInit(d), danger: true, action: () => initialize(ctx, d) } : null,
    { separator: true },
    { label: t('properties'), icon: 'properties', action: () => ctx.openDisk(d.id) },
  ].filter(Boolean);
}

export function openMenu(e, items, anchor) {
  e.preventDefault();
  const r = anchor?.getBoundingClientRect?.();
  const x = e.clientX || (r ? r.left + 8 : 0);
  const y = e.clientY || (r ? r.bottom : 0);
  showContextMenu({ x, y, items, anchor: e.clientX ? null : anchor });
}
