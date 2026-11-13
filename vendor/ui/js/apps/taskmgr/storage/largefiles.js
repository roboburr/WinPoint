// Taakbeheer › Opslag — Grote bestanden: review big files (thumbnails / icons), tick the
// ones you no longer need and move them to the Trash (never deleted outright).

import { h, clear } from '../../../core/dom.js';
import { Button, IconButton, Checkbox, Dropdown, Dialog, Toast, AppIcon, EmptyState, ProgressRing, InfoBar } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, date, relTime } from '../../../core/format.js';
import { prettyPath, dirName } from '../util.js';
import { guessMime } from './explorer.js';

const t = makeT({
  nl: {
    filter: 'Groter dan', in: 'in {where}', refresh: 'Vernieuwen', loading: 'Grote bestanden zoeken…',
    none: 'Geen grote bestanden gevonden', noneBody: 'Er staan hier geen bestanden die groter zijn dan {min}.',
    name: 'Naam', modified: 'Gewijzigd', size: 'Grootte', selectAll: 'Alles selecteren',
    review: 'Controleren voor je opruimt', selected0: 'Vink de bestanden aan die je niet meer nodig hebt.',
    selected: { one: '1 bestand geselecteerd', other: '{n} bestanden geselecteerd' }, total: 'Samen {size}',
    toTrash: 'Naar Prullenbak verplaatsen', trashNote: 'Bestanden gaan naar de Prullenbak. Je kunt ze daar terugzetten, zolang je de Prullenbak niet leegmaakt.',
    confirmTitle: { one: '1 bestand naar de Prullenbak verplaatsen?', other: '{n} bestanden naar de Prullenbak verplaatsen?' },
    confirmBody: 'Dit maakt {size} vrij zodra je de Prullenbak leegmaakt. Tot die tijd kun je alles terugzetten.',
    move: 'Verplaatsen', cancel: 'Annuleren',
    moved: { one: '1 bestand staat in de Prullenbak', other: '{n} bestanden staan in de Prullenbak' },
    undoHint: 'Je kunt ze terugzetten vanuit de Prullenbak', openTrash: 'Prullenbak openen',
    failed: { one: '1 bestand kon niet worden verplaatst', other: '{n} bestanden konden niet worden verplaatst' },
    failedAll: 'Verplaatsen is niet gelukt', openLocation: 'Bestandslocatie openen', deselect: 'Selectie wissen',
    loadFailed: 'Kon de lijst niet ophalen',
    appOwned: 'Hoort bij een app', appOwnedTip: 'Dit bestand hoort bij een app of bij het systeem. Laat het staan, tenzij je zeker weet dat je het niet meer nodig hebt.',
    appWarnTitle: 'Sommige bestanden horen bij een app',
    appWarnBody: 'Bijvoorbeeld van Steam of een virtuele machine. Die app werkt daarna misschien niet meer goed.',
    dryRun: 'Testmodus: er is niets echt verplaatst.',
  },
  en: {
    filter: 'Larger than', in: 'in {where}', refresh: 'Refresh', loading: 'Looking for large files…',
    none: 'No large files found', noneBody: 'There are no files larger than {min} here.',
    name: 'Name', modified: 'Modified', size: 'Size', selectAll: 'Select all',
    review: 'Review before you clean up', selected0: 'Tick the files you no longer need.',
    selected: { one: '1 file selected', other: '{n} files selected' }, total: '{size} in total',
    toTrash: 'Move to Recycle Bin', trashNote: 'Files are moved to the Recycle Bin. You can restore them from there until you empty the Recycle Bin.',
    confirmTitle: { one: 'Move 1 file to the Recycle Bin?', other: 'Move {n} files to the Recycle Bin?' },
    confirmBody: 'This frees up {size} once you empty the Recycle Bin. Until then, you can restore everything.',
    move: 'Move', cancel: 'Cancel',
    moved: { one: '1 file is in the Recycle Bin', other: '{n} files are in the Recycle Bin' },
    undoHint: 'You can restore them from the Recycle Bin', openTrash: 'Open Recycle Bin',
    failed: { one: '1 file couldn’t be moved', other: '{n} files couldn’t be moved' },
    failedAll: 'Couldn’t move the files', openLocation: 'Open file location', deselect: 'Clear selection',
    loadFailed: 'Couldn’t load the list',
    appOwned: 'Belongs to an app', appOwnedTip: 'This file belongs to an app or to the system. Leave it unless you’re sure you no longer need it.',
    appWarnTitle: 'Some files belong to an app',
    appWarnBody: 'For example Steam or a virtual machine. That app may not work properly afterwards.',
    dryRun: 'Test mode: nothing was actually moved.',
  },
});

const MB = 1048576, GB = 1073741824;

/** Files that belong to an app or to the system rather than to the user: game data (Steam), app folders
 *  (any hidden folder such as ~/.local/share, ~/.var/app, ~/.cache), snaps, app-managed folders (Android SDK
 *  images, node_modules, VirtualBox VMs), system folders and virtual disks.
 *  Trashing them can break that app, so they are flagged, left out of "Alles selecteren" and warned about. */
const APP_PATH_RE = /\/\.[^/]+\/|\/snap\/|\/steamapps\/|\/Android\/Sdk\/|\/node_modules\/|\/VirtualBox VMs\/|^\/(?:usr|opt|var|etc|lib|lib64|boot)\//;
const APP_EXT_RE = /\.(?:qcow2|vdi|vmdk|vhdx?|vpk|pak|asar)$/i;
export const isAppOwned = (f) => f.appOwned ?? (APP_PATH_RE.test(f.path || '') || APP_EXT_RE.test(f.name || ''));
const SIZES = [
  { value: 50 * MB, label: '50 MB' }, { value: 100 * MB, label: '100 MB' }, { value: 500 * MB, label: '500 MB' },
  { value: GB, label: '1 GB' }, { value: 5 * GB, label: '5 GB' },
];

export function createLargeFiles(ctx, { onChanged } = {}) {
  const { bridge } = ctx;
  let scope = null;
  let minSize = 100 * MB;
  let files = [];
  const checked = new Set();
  let seq = 0;

  const sizeDd = Dropdown({ options: SIZES, value: minSize, onChange: (v) => { minSize = v; load(); }, width: 120, ariaLabel: t('filter') });
  const whereEl = h('span', { class: 'tm-lf__where' });
  const refreshBtn = Button({ label: t('refresh'), icon: 'refresh', variant: 'subtle', onClick: () => load() });
  const bar = h('div', { class: 'tm-lf__bar' }, h('span', { class: 'text-2' }, t('filter')), sizeDd, whereEl, h('span', { class: 'grow' }), refreshBtn);

  // "Alles selecteren" never ticks files that belong to an app (they can still be ticked one by one)
  const selectable = () => files.filter((f) => !isAppOwned(f));
  const allBox = Checkbox({ label: '', onChange: (on) => { if (on) selectable().forEach((f) => checked.add(f.path)); else checked.clear(); renderChecks(); } });
  allBox.setAttribute('aria-label', t('selectAll'));
  const head = h('div', { class: 'tm-lf__head' }, allBox, h('span', null, t('name')), h('span', { class: 'is-right' }, t('modified')), h('span', { class: 'is-right' }, t('size')));
  const rows = h('div', { class: 'tm-lf__rows', role: 'list' });
  const listCard = h('div', { class: 'tm-card tm-lf__list' }, head, rows);

  const revCount = h('div', { class: 'tm-lf__revcount' });
  const revTotal = h('div', { class: 'tm-lf__revtotal' });
  const revList = h('ul', { class: 'tm-lf__revlist' });
  const trashBtn = Button({ label: t('toTrash'), icon: 'trash', variant: 'accent', disabled: true, onClick: () => moveToTrash() });
  const clearBtn = Button({ label: t('deselect'), variant: 'subtle', size: 'small', onClick: () => { checked.clear(); renderChecks(); } });
  const review = h('aside', { class: 'tm-card tm-lf__review' },
    h('div', { class: 'tm-lf__revhead' }, icon('trash', { size: 20 }), h('h4', null, t('review'))),
    revCount, revTotal, revList, trashBtn, clearBtn, h('p', { class: 'tm-lf__note caption' }, t('trashNote')));

  const body = h('div', { class: 'tm-lf__body' }, listCard, review);
  const el = h('div', { class: 'tm-lf' }, bar, body);

  async function load() {
    if (!scope) return;
    const my = ++seq;
    whereEl.textContent = t('in', { where: scope.label || prettyPath(scope.path) });
    clear(rows).append(h('div', { class: 'tm-list__empty' }, ProgressRing({ size: 28 }), h('div', { class: 'caption', style: { marginTop: '8px' } }, t('loading'))));
    let list = [];
    try {
      list = await bridge.call('storage.largeFiles', { path: scope.path, limit: 100, minSize });
    } catch (e) {
      if (my !== seq) return;
      clear(rows).append(h('div', { class: 'tm-list__empty' }, `${t('loadFailed')}: ${e.message || ''}`));
      return;
    }
    if (my !== seq) return;
    files = Array.isArray(list) ? list : [];
    for (const p of [...checked]) if (!files.some((f) => f.path === p)) checked.delete(p);
    renderRows();
  }

  function thumbFor(f) {
    const mime = f.mime || guessMime(f.name);
    const thumbable = /^image\//.test(mime) || /^video\//.test(mime);
    const box = h('span', { class: ['tm-lf__thumb', thumbable && 'is-media'] });
    const fallback = () => { clear(box).append(AppIcon({ mime, size: 32, fallback: 'file' })); box.classList.remove('is-media'); };
    const url = thumbable ? bridge.thumbUrl(f.path, 96) : '';
    if (url) {
      const img = h('img', { src: url, alt: '', loading: 'lazy', decoding: 'async', draggable: false });
      img.addEventListener('error', fallback, { once: true });
      box.append(img);
      if (/^video\//.test(mime)) box.append(h('span', { class: 'tm-lf__play' }, icon('play', { size: 12 })));
    } else fallback();
    return box;
  }

  function renderRows() {
    clear(rows);
    if (!files.length) {
      rows.append(EmptyState({ icon: 'success', title: t('none'), message: t('noneBody', { min: SIZES.find((s) => s.value === minSize)?.label || bytes(minSize) }) }));
      renderChecks();
      return;
    }
    const maxSize = Math.max(...files.map((f) => f.size));
    for (const f of files) {
      const cb = Checkbox({ checked: checked.has(f.path), onChange: (on) => { if (on) checked.add(f.path); else checked.delete(f.path); renderChecks(); } });
      cb.setAttribute('aria-label', f.name);
      const locBtn = IconButton({ icon: 'folder-open', label: t('openLocation'), size: 28, onClick: (e) => { e.stopPropagation(); openLocation(f.path); } });
      const app = isAppOwned(f);
      const row = h('div', { class: ['tm-lf__row', app && 'is-app'], role: 'listitem', dataset: { path: f.path } },
        cb, thumbFor(f),
        h('div', { class: 'tm-lf__text' },
          h('div', { class: 'tm-lf__name' }, f.name),
          h('div', { class: 'tm-lf__dir' },
            app ? h('span', { class: 'tm-lf__app', dataset: { tooltip: t('appOwnedTip') } }, icon('warning', { size: 12 }), t('appOwned')) : null,
            app ? h('span', { class: 'tm-lf__sep', 'aria-hidden': 'true' }, '·') : null,
            prettyPath(dirName(f.path)))),
        locBtn,
        h('div', { class: 'tm-lf__date', title: f.modified ? date(f.modified, 'datetime') : '' }, f.modified ? relTime(f.modified) : '–'),
        h('div', { class: 'tm-lf__size' }, bytes(f.size), h('span', { class: 'tm-lf__sizebar' }, h('span', { style: { width: `${Math.max(3, (f.size / maxSize) * 100)}%` } }))));
      row._cb = cb;
      row.addEventListener('click', (e) => {
        if (e.target.closest('.fl-check, button')) return;
        cb.setChecked(!cb.checked);
        if (cb.checked) checked.add(f.path); else checked.delete(f.path);
        renderChecks();
      });
      rows.append(row);
    }
    renderChecks();
  }

  function renderChecks() {
    for (const row of rows.children) {
      if (!row._cb) continue;
      const on = checked.has(row.dataset.path);
      if (row._cb.checked !== on) row._cb.setChecked(on);
      row.classList.toggle('is-checked', on);
    }
    const sel = files.filter((f) => checked.has(f.path));
    const total = sel.reduce((a, f) => a + f.size, 0);
    const all = selectable().length || files.length;
    allBox.setChecked(sel.length > 0 && sel.length >= all);
    if (sel.length && sel.length < all) allBox.setIndeterminate(true);
    revCount.textContent = sel.length ? t('selected', { n: sel.length }) : t('selected0');
    revTotal.textContent = sel.length ? t('total', { size: bytes(total) }) : '';
    revTotal.hidden = !sel.length;
    clear(revList);
    for (const f of sel.slice(0, 50)) revList.append(h('li', null, h('span', { class: 'grow' }, f.name), h('span', { class: 'tm-size' }, bytes(f.size))));
    revList.hidden = !sel.length;
    trashBtn.setDisabled(!sel.length);
    clearBtn.hidden = !sel.length;
  }

  async function moveToTrash() {
    const sel = files.filter((f) => checked.has(f.path));
    if (!sel.length) return;
    const total = sel.reduce((a, f) => a + f.size, 0);
    const appSel = sel.some(isAppOwned);
    const content = h('div', { class: 'tm-dlg' },
      appSel ? InfoBar({ severity: 'warning', title: t('appWarnTitle'), message: t('appWarnBody'), closable: false }) : null,
      h('p', null, t('confirmBody', { size: bytes(total) })),
      h('ul', { class: 'tm-dlg__list' }, sel.map((f) => h('li', { class: isAppOwned(f) ? 'is-app' : null }, AppIcon({ mime: f.mime || guessMime(f.name), size: 16, fallback: 'file' }),
        h('span', { class: 'grow' }, f.name), isAppOwned(f) ? h('span', { class: 'tm-lf__app', 'aria-label': t('appOwned') }, icon('warning', { size: 12 })) : null, h('span', { class: 'tm-size' }, bytes(f.size))))),
      h('div', { class: 'tm-dlg__total' }, h('span', null, t('selected', { n: sel.length })), h('span', null, bytes(total))));
    const ok = await Dialog.confirm({ title: t('confirmTitle', { n: sel.length }), content, confirmLabel: t('move'), cancelLabel: t('cancel'), danger: appSel });
    if (!ok) return;
    let res;
    try {
      res = await bridge.call('files.trash', { paths: sel.map((f) => f.path) });
    } catch (e) {
      Toast.show({ title: t('failedAll'), message: e.message || '', severity: 'error' });
      return;
    }
    // test mode (--dry-run): the host only pretended, so the files stay in the list
    if (res?.dryRun) { Toast.show({ title: t('dryRun'), severity: 'info', timeout: 4000 }); return; }
    const failed = new Set((res?.failed || []).map((x) => (typeof x === 'string' ? x : x?.path)));
    const moved = sel.filter((f) => !failed.has(f.path));
    if (moved.length) {
      files = files.filter((f) => !moved.some((m) => m.path === f.path));
      moved.forEach((f) => checked.delete(f.path));
      renderRows();
      Toast.show({
        title: t('moved', { n: moved.length }), message: t('undoHint'), severity: 'success', timeout: 8000,
        action: { label: t('openTrash'), onClick: () => bridge.call('shell.openApp', { app: 'explorer', params: { path: 'trash:///' } }).catch(() => {}) },
      });
      onChanged?.();
    }
    if (failed.size) Toast.show({ title: t('failed', { n: failed.size }), severity: 'warning' });
  }

  function openLocation(path) {
    bridge.call('shell.openApp', { app: 'explorer', params: { path: dirName(path) } })
      .catch((e) => Toast.show({ title: t('openLocation'), message: e.message, severity: 'error' }));
  }

  return {
    el,
    setScope(s) {
      const same = scope && scope.path === s.path;
      scope = s;
      if (!same) { checked.clear(); files = []; }
      load();
    },
    refresh: load,
  };
}
