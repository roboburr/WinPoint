// Taakbeheer › Opslag — Opruimen: a Storage-Sense-like checklist with sizes and plain
// explanations. Items that need admin rights carry a shield and an explanation first.
// Old downloads are reviewed file by file and go to the Trash (never deleted outright).
//
// Item shape (storage.cleanup.list): {id, name, description, size|null, needsAdmin, safe,
// defaultSelected, available, details}. details may be {count} | {items:[…]} | {total, keepDays}
// | an array of strings | a string.

import { h, clear } from '../../../core/dom.js';
import { Button, Checkbox, InfoBar, Dialog, Toast, ProgressRing, EmptyState } from '../../../core/components.js';
import { icon } from '../../../core/icons.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, relTime, number } from '../../../core/format.js';
import { prettyPath, dirName } from '../util.js';

const t = makeT({
  nl: {
    intro: 'Maak ruimte vrij door bestanden op te ruimen die je pc niet meer nodig heeft. Je ziet precies wat er weggaat.',
    adminTitle: 'Hiervoor vraagt Ubuntu je wachtwoord',
    adminBody: 'Een paar onderdelen staan in systeemmappen. Ubuntu vraagt dan eerst om je wachtwoord – dat is normaal en veilig.',
    adminTag: 'Wachtwoord nodig', reviewTag: 'Eerst bekijken', trashTag: 'Gaat naar de Prullenbak', recommended: 'Aanbevolen', other: 'Ook mogelijk',
    selected: 'Geselecteerd: {size}', selectedUnknown: 'Geselecteerd: {size} + onbekend', nothing: 'Niets geselecteerd', clean: 'Opruimen', cleaning: 'Bezig met opruimen…',
    confirmTitle: '{size} opruimen?', confirmTitleUnknown: 'Opruimen?', confirmBody: 'Dit wordt opgeruimd:',
    confirmAdmin: 'Ubuntu vraagt daarna één keer om je wachtwoord voor de onderdelen met een schildje.',
    confirmPermanent: 'Let op: wat je uit de Prullenbak opruimt, kun je niet meer terugzetten.',
    confirmDownloads: 'De gekozen downloads gaan naar de Prullenbak; je kunt ze daar nog terugzetten.',
    cancel: 'Annuleren', freed: '{size} vrijgemaakt', freedBody: 'Je pc heeft weer wat meer ademruimte.', partly: 'Niet alles is gelukt', freedPartly: '{size} vrijgemaakt – niet alles is gelukt',
    movedDownloads: { one: '1 download staat nu in de Prullenbak ({size})', other: '{n} downloads staan nu in de Prullenbak ({size})' },
    skipped: 'Overgeslagen omdat ze nog open zijn: {names}. Sluit ze en probeer het opnieuw.',
    failedItem: '{name}: {error}', nothingToDo: 'Er valt niets op te ruimen', nothingBody: 'Je pc is al netjes opgeruimd.',
    loading: 'Berekenen hoeveel ruimte je kunt vrijmaken…', refresh: 'Opnieuw berekenen', more: 'Wat zit hierin?', review: 'Bestanden bekijken',
    failedLoad: 'Kon de lijst niet ophalen', dryRun: 'Testmodus: er is niets echt verwijderd',
    unknownSize: 'Onbekend', items: { one: '1 item', other: '{n} items' }, files: { one: '1 bestand', other: '{n} bestanden' },
    journalInfo: 'Logboeken samen {total}; alles ouder dan {days} dagen gaat weg.', running: 'nog geopend',
    pickAll: 'Alles kiezen', pickNone: 'Niets kiezen', picked: '{n} van {m} gekozen', notOpened: 'niet geopend sinds {when}',
    andMore: 'en nog {n}', revision: 'versie {r}',
  },
  en: {
    intro: 'Free up space by cleaning up files your PC no longer needs. You’ll see exactly what will be removed.',
    adminTitle: 'Ubuntu will ask for your password',
    adminBody: 'A few items are in system folders, so Ubuntu asks for your password first – that’s normal and safe.',
    adminTag: 'Password needed', reviewTag: 'Review first', trashTag: 'Goes to the Recycle Bin', recommended: 'Recommended', other: 'Also possible',
    selected: 'Selected: {size}', selectedUnknown: 'Selected: {size} + unknown', nothing: 'Nothing selected', clean: 'Clean up', cleaning: 'Cleaning up…',
    confirmTitle: 'Clean up {size}?', confirmTitleUnknown: 'Clean up?', confirmBody: 'This will be cleaned up:',
    confirmAdmin: 'Ubuntu will then ask once for your password for the items with a shield.',
    confirmPermanent: 'Note: what you clean up from the Recycle Bin can’t be restored.',
    confirmDownloads: 'The chosen downloads will be moved to the Recycle Bin; you can still restore them from there.',
    cancel: 'Cancel', freed: '{size} freed up', freedBody: 'Your PC has a bit more room to breathe.', partly: 'Not everything worked', freedPartly: '{size} freed up – not everything worked',
    movedDownloads: { one: '1 download is now in the Recycle Bin ({size})', other: '{n} downloads are now in the Recycle Bin ({size})' },
    skipped: 'Skipped because they’re still open: {names}. Close them and try again.',
    failedItem: '{name}: {error}', nothingToDo: 'Nothing to clean up', nothingBody: 'Your PC is already tidy.',
    loading: 'Working out how much space you can free…', refresh: 'Calculate again', more: 'What’s in here?', review: 'View files',
    failedLoad: 'Couldn’t load the list', dryRun: 'Test mode: nothing was actually removed',
    unknownSize: 'Unknown', items: { one: '1 item', other: '{n} items' }, files: { one: '1 file', other: '{n} files' },
    journalInfo: 'Logs take up {total}; everything older than {days} days will be removed.', running: 'still open',
    pickAll: 'Choose all', pickNone: 'Choose none', picked: '{n} of {m} chosen', notOpened: 'not opened since {when}',
    andMore: 'and {n} more', revision: 'revision {r}',
  },
});

const ICONS = {
  trash: 'trash', thumbnails: 'image', 'user-cache-safe': 'app', 'browser-cache': 'globe', 'old-downloads': 'download',
  'flatpak-unused': 'apps', 'crash-reports': 'warning', journal: 'document', 'apt-cache': 'update', 'old-snaps': 'history',
};

export function createCleanup(ctx, { onChanged } = {}) {
  const { bridge } = ctx;
  let items = [];
  const selected = new Set();
  const downloadPaths = new Set();   // ticked old downloads
  let loaded = false;
  let busy = false;

  const adminBar = InfoBar({ severity: 'info', title: t('adminTitle'), message: t('adminBody'), closable: false });
  adminBar.hidden = true;
  const resultBar = h('div', { class: 'tm-clean__result' });
  const list = h('div', { class: 'tm-clean__list' });
  const totalEl = h('div', { class: 'tm-clean__total' });
  const cleanBtn = Button({ label: t('clean'), icon: 'broom', variant: 'accent', size: 'large', disabled: true, onClick: () => run() });
  const footer = h('div', { class: 'tm-clean__footer' }, h('div', { class: 'tm-clean__totalwrap' }, icon('storage', { size: 20 }), totalEl), cleanBtn);
  const el = h('div', { class: 'tm-clean' },
    h('p', { class: 'tm-clean__intro text-2' }, t('intro')), resultBar, adminBar, list, footer);

  const oldDownloads = () => items.find((i) => i.id === 'old-downloads');
  const dlItems = () => (Array.isArray(oldDownloads()?.details?.items) ? oldDownloads().details.items : []);
  const known = (it) => typeof it.size === 'number';

  /** Effective size of an item given the current selection (old downloads: the ticked files). */
  function sizeOf(it) {
    if (it.id === 'old-downloads' && dlItems().length) return dlItems().filter((f) => downloadPaths.has(f.path)).reduce((a, f) => a + (f.size || 0), 0);
    return it.size;
  }
  const selectable = (it) => it.available !== false && (it.size == null || it.size > 0);

  async function load() {
    clear(list).append(h('div', { class: 'tm-list__empty' }, ProgressRing({ size: 28 }), h('div', { class: 'caption', style: { marginTop: '8px' } }, t('loading'))));
    try {
      items = await bridge.call('storage.cleanup.list', {});
    } catch (e) {
      clear(list).append(InfoBar({ severity: 'error', title: t('failedLoad'), message: e.message || '', closable: false, action: { label: t('refresh'), onClick: () => load() } }));
      return;
    }
    items = (Array.isArray(items) ? items : []).filter((i) => i.available !== false);
    if (!loaded) {
      selected.clear(); downloadPaths.clear();
      items.forEach((it) => { if (it.defaultSelected && selectable(it)) selected.add(it.id); });
    } else {
      for (const id of [...selected]) if (!items.some((i) => i.id === id && selectable(i))) selected.delete(id);
      const valid = new Set(dlItems().map((f) => f.path));
      for (const p of [...downloadPaths]) if (!valid.has(p)) downloadPaths.delete(p);
    }
    loaded = true;
    render();
  }

  /* ------------------------------------------------------------ details */
  function detailsNode(it) {
    const d = it.details;
    if (d == null) return null;
    if (typeof d === 'string') return d.trim() ? h('div', { class: 'tm-clean__details selectable' }, d) : null;
    if (Array.isArray(d)) return d.length ? h('ul', { class: 'tm-clean__details' }, d.map((x) => h('li', null, typeof x === 'string' ? x : x.name || x.path || ''))) : null;
    if (it.id === 'old-downloads' && Array.isArray(d.items)) return downloadsPicker(it);
    const rows = [];
    if (Array.isArray(d.items)) {
      const shown = d.items.filter((x) => (x.size ?? 1) > 0).slice(0, 8);
      for (const x of shown) {
        const label = [x.name || x.browser || prettyPath(x.path || ''), x.revision ? t('revision', { r: x.revision }) : '', x.running ? `(${t('running')})` : ''].filter(Boolean).join(' ');
        rows.push(h('li', null, h('span', { class: 'grow' }, label), x.size != null ? h('span', { class: 'tm-size' }, bytes(x.size)) : null));
      }
      if (d.items.length > shown.length) rows.push(h('li', { class: 'text-3' }, t('andMore', { n: d.items.length - shown.length })));
    }
    if (d.count != null && !Array.isArray(d.items)) rows.push(h('li', null, t('items', { n: number(d.count), count: d.count })));
    if (d.total != null && d.keepDays != null) rows.push(h('li', null, t('journalInfo', { total: bytes(d.total), days: d.keepDays })));
    return rows.length ? h('ul', { class: 'tm-clean__details' }, rows) : null;
  }

  function downloadsPicker(it) {
    const files = dlItems();
    const head = h('div', { class: 'tm-clean__pickhead' });
    const updHead = () => { head.textContent = t('picked', { n: downloadPaths.size, m: files.length }); };
    const boxes = [];
    const ul = h('ul', { class: 'tm-clean__details tm-clean__picks' }, files.slice(0, 300).map((f) => {
      const cb = Checkbox({
        checked: downloadPaths.has(f.path), label: f.name || prettyPath(f.path),
        description: [prettyPath(dirName(f.path)), f.accessed || f.modified ? t('notOpened', { when: relTime(f.accessed || f.modified) }) : ''].filter(Boolean).join(' · '),
        onChange: (on) => { if (on) downloadPaths.add(f.path); else downloadPaths.delete(f.path); syncDownloads(); updHead(); },
      });
      boxes.push([cb, f.path]);
      return h('li', null, cb, h('span', { class: 'tm-size' }, bytes(f.size || 0)));
    }));
    const all = h('button', { type: 'button', class: 'fl-link tm-clean__more', onClick: () => { files.forEach((f) => downloadPaths.add(f.path)); boxes.forEach(([b]) => b.setChecked(true)); syncDownloads(); updHead(); } }, t('pickAll'));
    const none = h('button', { type: 'button', class: 'fl-link tm-clean__more', onClick: () => { downloadPaths.clear(); boxes.forEach(([b]) => b.setChecked(false)); syncDownloads(); updHead(); } }, t('pickNone'));
    updHead();
    const wrap = h('div', { class: 'tm-clean__pickwrap' }, h('div', { class: 'tm-clean__pickbar' }, head, all, none), ul);
    wrap._boxes = boxes;
    return wrap;
  }

  function syncDownloads() {
    if (downloadPaths.size) selected.add('old-downloads'); else selected.delete('old-downloads');
    update();
  }

  /* ---------------------------------------------------------------- rows */
  function itemRow(it) {
    const ok = selectable(it);
    const cb = Checkbox({
      checked: selected.has(it.id) && ok, disabled: !ok || busy,
      onChange: (on) => toggle(it, on),
    });
    cb.setAttribute('aria-label', it.name);
    const tags = h('span', { class: 'tm-clean__tags' },
      it.needsAdmin ? h('span', { class: 'tm-pill is-accent', dataset: { tooltip: t('adminBody') } }, icon('shield', { size: 12 }), t('adminTag')) : null,
      it.id === 'old-downloads' ? h('span', { class: 'tm-pill' }, icon('trash', { size: 12 }), t('trashTag'))
        : (!it.safe && it.id !== 'trash' ? h('span', { class: 'tm-pill is-warning' }, t('reviewTag')) : null));
    const det = detailsNode(it);
    let detailsEl = null;
    if (det) {
      const isPicker = it.id === 'old-downloads';
      const toggleBtn = h('button', { type: 'button', class: 'fl-link tm-clean__more', 'aria-expanded': 'false' }, icon('chevron-down', { size: 12 }), h('span', null, isPicker ? t('review') : t('more')));
      det.hidden = true;
      toggleBtn.addEventListener('click', () => setOpen(det.hidden));
      function setOpen(open) {
        det.hidden = !open;
        toggleBtn.setAttribute('aria-expanded', String(open));
        toggleBtn.classList.toggle('is-open', open);
      }
      detailsEl = h('div', { class: 'tm-clean__detailwrap' }, toggleBtn, det);
      detailsEl._open = setOpen;
    }
    const sizeEl = h('div', { class: 'tm-clean__size' });
    const row = h('div', { class: ['tm-clean__row', !ok && 'is-empty', selected.has(it.id) && 'is-on'], dataset: { id: it.id } },
      cb,
      h('div', { class: 'tm-clean__icon' }, icon(it.needsAdmin ? 'shield' : (ICONS[it.id] || 'clean'), { size: 20 })),
      h('div', { class: 'tm-clean__text' },
        h('div', { class: 'tm-clean__name' }, h('span', null, it.name), tags),
        h('div', { class: 'tm-clean__desc' }, it.description || ''),
        detailsEl),
      sizeEl);
    row.addEventListener('click', (e) => {
      if (!ok || busy || e.target.closest('.fl-check, button, a, .tm-clean__details')) return;
      cb.setChecked(!cb.checked);
      toggle(it, cb.checked);
    });
    row._cb = cb; row._size = sizeEl; row._item = it; row._details = detailsEl;
    return row;
  }

  function toggle(it, on) {
    if (it.id === 'old-downloads' && dlItems().length) {
      // ticking the row selects every old download; the list opens so you can review it
      const row = list.querySelector('.tm-clean__row[data-id="old-downloads"]');
      if (on) dlItems().forEach((f) => downloadPaths.add(f.path)); else downloadPaths.clear();
      row?._details?._open?.(true);
      row?._details?.querySelector('.tm-clean__pickwrap')?._boxes?.forEach(([b, p]) => b.setChecked(downloadPaths.has(p)));
      const head = row?._details?.querySelector('.tm-clean__pickhead');
      if (head) head.textContent = t('picked', { n: downloadPaths.size, m: dlItems().length });
      syncDownloads();
      return;
    }
    if (on) selected.add(it.id); else selected.delete(it.id);
    update();
  }

  function render() {
    clear(list);
    const rec = items.filter((i) => i.safe !== false);
    const rest = items.filter((i) => i.safe === false);
    if (!items.some(selectable)) {
      list.append(EmptyState({ icon: 'success', title: t('nothingToDo'), message: t('nothingBody') }));
    } else {
      if (rec.length) list.append(h('h3', { class: 'tm-section__title tm-clean__group' }, t('recommended')), h('div', { class: 'tm-card tm-clean__card' }, rec.map(itemRow)));
      if (rest.length) list.append(h('h3', { class: 'tm-section__title tm-clean__group' }, t('other')), h('div', { class: 'tm-card tm-clean__card' }, rest.map(itemRow)));
    }
    update();
  }

  function selection() { return items.filter((i) => selected.has(i.id) && selectable(i)); }

  function update() {
    const sel = selection();
    const total = sel.reduce((a, i) => a + (sizeOf(i) || 0), 0);
    const unknown = sel.some((i) => !known(i));
    totalEl.textContent = sel.length ? (unknown ? t('selectedUnknown', { size: bytes(total) }) : t('selected', { size: bytes(total) })) : t('nothing');
    cleanBtn.setDisabled(!sel.length || busy);
    adminBar.hidden = !sel.some((i) => i.needsAdmin);
    for (const row of list.querySelectorAll('.tm-clean__row')) {
      const it = row._item;
      const on = selected.has(it.id);
      row.classList.toggle('is-on', on);
      if (row._cb.checked !== on && selectable(it)) row._cb.setChecked(on);
      if (it.id === 'old-downloads' && dlItems().length) {
        row._cb.setIndeterminate(downloadPaths.size > 0 && downloadPaths.size < dlItems().length);
        row._size.textContent = downloadPaths.size && downloadPaths.size < dlItems().length ? `${bytes(sizeOf(it))} / ${bytes(it.size)}` : bytes(it.size || 0);
      } else row._size.textContent = known(it) ? (it.size > 0 ? bytes(it.size) : '0 bytes') : t('unknownSize');
    }
  }

  async function run() {
    const sel = selection();
    if (!sel.length) return;
    const total = sel.reduce((a, i) => a + (sizeOf(i) || 0), 0);
    const anyAdmin = sel.some((i) => i.needsAdmin);
    const content = h('div', { class: 'tm-dlg' },
      h('p', null, t('confirmBody')),
      h('ul', { class: 'tm-dlg__list' }, sel.map((i) => h('li', null, i.needsAdmin ? icon('shield', { size: 14 }) : icon(ICONS[i.id] || 'clean', { size: 14 }),
        h('span', { class: 'grow' }, i.id === 'old-downloads' && dlItems().length ? `${i.name} – ${t('files', { n: downloadPaths.size })}` : i.name),
        h('span', { class: 'tm-size' }, known(i) ? bytes(sizeOf(i)) : t('unknownSize'))))),
      sel.some((i) => i.id === 'trash') ? h('p', { class: 'tm-dlg__hint' }, t('confirmPermanent')) : null,
      sel.some((i) => i.id === 'old-downloads') ? h('p', { class: 'tm-dlg__hint' }, t('confirmDownloads')) : null,
      anyAdmin ? h('p', { class: 'tm-dlg__hint' }, t('confirmAdmin')) : null);
    const ok = await Dialog.confirm({
      title: total > 0 ? t('confirmTitle', { size: bytes(total) }) : t('confirmTitleUnknown'),
      content, confirmLabel: t('clean'), cancelLabel: t('cancel'),
    });
    if (!ok) return;
    busy = true;
    cleanBtn.setDisabled(true);
    cleanBtn.setLabel(t('cleaning'));
    clear(resultBar);
    const args = { ids: sel.map((i) => i.id) };
    if (sel.some((i) => i.id === 'old-downloads') && dlItems().length) args.paths = [...downloadPaths];
    let res;
    try {
      res = await bridge.call('storage.cleanup.run', args);
    } catch (e) {
      busy = false;
      cleanBtn.setLabel(t('clean'));
      resultBar.append(InfoBar({ severity: 'error', title: t('partly'), message: e.message || '' }));
      update();
      return;
    }
    busy = false;
    cleanBtn.setLabel(t('clean'));
    showResult(res);
    selected.clear();
    downloadPaths.clear();
    loaded = false;
    await load();
    onChanged?.();
  }

  function showResult(res) {
    const results = res?.results || [];
    const failed = results.filter((r) => !r.ok);
    const freed = res?.freed ?? results.reduce((a, r) => a + (r.freed || 0), 0);
    if (res?.dryRun && !bridge.isMock) { resultBar.append(InfoBar({ severity: 'info', title: t('dryRun') })); return; }
    const names = new Map(items.map((i) => [i.id, i.name]));
    const lines = [];
    const dl = results.find((r) => r.id === 'old-downloads' && r.movedToTrash);
    if (dl) lines.push(t('movedDownloads', { n: dl.count || 0, size: bytes(dl.movedToTrash) }));
    const skipped = results.flatMap((r) => r.skipped || []);
    if (skipped.length) lines.push(t('skipped', { names: skipped.join(', ') }));
    for (const f of failed) lines.push(t('failedItem', { name: names.get(f.id) || f.id, error: f.error || '' }));
    if (!failed.length) {
      resultBar.append(InfoBar({ severity: 'success', title: t('freed', { size: bytes(freed) }), message: [t('freedBody'), ...lines].join(' ') }));
      Toast.show({ title: t('freed', { size: bytes(freed) }), severity: 'success', timeout: 4000 });
    } else {
      resultBar.append(InfoBar({
        severity: freed > 0 ? 'warning' : 'error',
        title: freed > 0 ? t('freedPartly', { size: bytes(freed) }) : t('partly'),
        message: lines.join(' · '),
      }));
    }
  }

  return { el, load: () => (loaded ? update() : load()), refresh: load };
}
