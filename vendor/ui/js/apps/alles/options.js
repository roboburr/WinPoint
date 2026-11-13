// Alles — "Opties" dialog: which drives/folders to index, excluded folders, hidden files,
// live updates and "Index opnieuw opbouwen".

import { h, clear } from '../../core/dom.js';
import { icon } from '../../core/icons.js';
import { bridge } from '../../core/bridge.js';
import { Button, IconButton, ToggleSwitch, SettingsCard, SettingsGroup, Dialog, Toast, TextBox, InfoBar } from '../../core/components.js';
import { getSetting, setSetting, getShellInfo } from '../../core/theme.js';
import { bytes, number, relTime } from '../../core/format.js';
import { t } from './i18n.js';

async function loadLocations(status) {
  const [locs, vols] = await Promise.allSettled([bridge.call('index.locations'), bridge.call('storage.volumes')]);
  const names = new Map();
  if (vols.status === 'fulfilled' && Array.isArray(vols.value)) {
    for (const v of vols.value) if (v?.mount && (v.name || v.label)) names.set(v.mount, v.isSystem ? t('opt_systemDrive') : (v.name || v.label));
  }
  if (locs.status === 'fulfilled' && Array.isArray(locs.value) && locs.value.length) {
    return locs.value.map((l) => (names.has(l.path) && l.kind !== 'folder' ? { ...l, label: names.get(l.path) } : l));
  }
  const roots = getSetting('alles.roots', 'auto');
  try {
    const vols2 = vols.status === 'fulfilled' ? vols.value : null;
    if (Array.isArray(vols2) && vols2.length) {
      return vols2.filter((v) => v.mount).map((v) => ({
        path: v.mount, label: v.isSystem ? t('opt_systemDrive') : (v.label || v.name || v.mount), fsType: v.fsType,
        total: v.total, free: v.free, auto: true, kind: 'drive', removable: !!v.removable,
        indexed: roots === 'auto' || (Array.isArray(roots) && roots.includes(v.mount)),
      }));
    }
  } catch { /* ignore */ }
  return (status?.roots || []).map((r) => ({ path: r.path, label: r.path.split('/').pop() || r.path, fsType: r.fsType, total: 0, free: 0, auto: true, kind: 'drive', indexed: true }));
}

async function pickFolder(title) {
  try {
    const r = await bridge.call('shell.pickFile', { title, folder: true });
    if (r && r.path) return r.path;
  } catch { /* fall through */ }
  const mode = getShellInfo()?.mode;
  if (!bridge.isMock && mode !== 'serve') return null;
  // no native dialog (browser / test): ask for a path
  const box = TextBox({ placeholder: '/home/robin/…', ariaLabel: t('opt_pathPrompt') });
  const res = await Dialog.show({
    title: t('opt_pathPromptTitle'),
    content: h('div', { class: 'alles-prompt' }, h('p', null, t('opt_pathPrompt')), box),
    primary: t('opt_pathAdd'), close: t('opt_cancel'),
  });
  const v = (box.getValue() || '').trim();
  return res === 'primary' && v ? v : null;
}

function driveIcon(loc) {
  if (loc.kind === 'folder') return 'folder';
  return loc.removable ? 'usb' : 'drive';
}

/** openOptions({status}) — resolves when the dialog is closed. */
export async function openOptions({ status } = {}) {
  const locations = await loadLocations(status);
  const origRoots = getSetting('alles.roots', 'auto');
  const state = {
    on: new Map(locations.map((l) => [l.path, !!l.indexed])),
    folders: locations.filter((l) => l.kind === 'folder').map((l) => l.path),
    excluded: [...(getSetting('alles.excluded', []) || [])],
    hidden: !!getSetting('alles.showHidden', false),
    live: getSetting('alles.liveUpdate', true) !== false,
  };
  const drives = locations.filter((l) => l.kind !== 'folder');

  const error = InfoBar({ severity: 'error', title: t('opt_needOne'), closable: false, open: false });

  /* --- locations ---------------------------------------------------------- */
  const locGroup = SettingsGroup({ title: t('opt_locations'), description: t('opt_locationsDesc') });
  function renderLocations() {
    clear(locGroup.items);
    for (const d of drives) {
      const desc = [d.path !== d.label ? d.path : null,
        d.total ? t('opt_free', { free: bytes(d.free), total: bytes(d.total) }) : null,
        d.removable ? t('opt_removable') : null].filter(Boolean).join(' · ');
      const sw = ToggleSwitch({
        checked: state.on.get(d.path), ariaLabel: d.label,
        onChange: (v) => { state.on.set(d.path, v); error.hidden = true; },
      });
      locGroup.add(SettingsCard({ icon: driveIcon(d), title: d.label, description: desc, control: sw }));
    }
    for (const f of state.folders) {
      const rm = IconButton({ icon: 'close', label: t('opt_removeFolder'), size: 32, iconSize: 12,
        onClick: () => { state.folders = state.folders.filter((x) => x !== f); state.on.delete(f); renderLocations(); } });
      locGroup.add(SettingsCard({ icon: 'folder', title: f.split('/').pop() || f, description: f, control: rm }));
    }
    locGroup.add(h('div', { class: 'alles-opt__actions' }, Button({
      label: t('opt_addFolder'), icon: 'plus',
      onClick: async () => {
        const p = await pickFolder(t('opt_addFolder'));
        if (p && !state.folders.includes(p) && !drives.some((d) => d.path === p)) {
          state.folders.push(p);
          state.on.set(p, true);
          renderLocations();
        }
      },
    })));
  }
  renderLocations();

  /* --- exclusions ------------------------------------------------------- */
  const exGroup = SettingsGroup({ title: t('opt_excluded'), description: t('opt_excludedDesc') });
  function renderExcluded() {
    clear(exGroup.items);
    if (!state.excluded.length) {
      exGroup.add(h('div', { class: 'alles-opt__empty' }, icon('shield', { size: 16 }), h('span', null, t('opt_excludedNone'))));
    }
    for (const p of state.excluded) {
      const rm = IconButton({ icon: 'close', label: t('opt_removeFolder'), size: 32, iconSize: 12,
        onClick: () => { state.excluded = state.excluded.filter((x) => x !== p); renderExcluded(); } });
      exGroup.add(SettingsCard({ icon: 'eye-off', title: p.split('/').pop() || p, description: p, control: rm }));
    }
    exGroup.add(h('div', { class: 'alles-opt__actions' },
      Button({
        label: t('opt_exclude'), icon: 'plus',
        onClick: async () => {
          const p = await pickFolder(t('opt_exclude'));
          if (p && !state.excluded.includes(p)) { state.excluded.push(p); renderExcluded(); }
        },
      }),
      h('span', { class: 'alles-opt__note' }, t('opt_excludedSystem'))));
  }
  renderExcluded();

  /* --- view + index ------------------------------------------------------- */
  const viewGroup = SettingsGroup({
    title: t('opt_view'),
    children: [
      SettingsCard({ icon: 'eye', title: t('opt_hidden'), description: t('opt_hiddenDesc'),
        control: ToggleSwitch({ checked: state.hidden, ariaLabel: t('opt_hidden'), onChange: (v) => { state.hidden = v; } }) }),
      SettingsCard({ icon: 'lightning', title: t('opt_live'), description: t('opt_liveDesc'),
        control: ToggleSwitch({ checked: state.live, ariaLabel: t('opt_live'), onChange: (v) => { state.live = v; } }) }),
    ],
  });
  const stats = status && (status.files || status.folders)
    ? `${t('opt_stats', { files: number(status.files || 0), folders: number(status.folders || 0) })}${status.builtAt ? ' · ' + t('st_builtAt', { when: relTime(status.builtAt) }) : ''}`
    : '';
  const rebuildBtn = Button({
    label: t('opt_rebuildBtn'), icon: 'refresh',
    onClick: async () => {
      const ok = await Dialog.confirm({ title: t('opt_rebuildConfirmTitle'), message: t('opt_rebuildConfirm'), confirmLabel: t('opt_rebuildBtn') });
      if (!ok) return;
      try {
        await bridge.call('index.rebuild');
        Toast.show({ title: t('opt_rebuildStarted'), severity: 'info', timeout: 3500 });
        rebuildBtn.setDisabled(true);
      } catch (e) { Toast.show({ title: t('opt_saveFailed'), message: e.message, severity: 'error' }); }
    },
  });
  const indexGroup = SettingsGroup({
    title: t('opt_index'),
    children: [SettingsCard({ icon: 'refresh', title: t('opt_rebuild'), description: [t('opt_rebuildDesc'), stats ? h('span', { class: 'alles-opt__stats' }, stats) : null].filter(Boolean), control: rebuildBtn })],
  });

  const content = h('div', { class: 'alles-opt' },
    h('p', { class: 'alles-opt__intro' }, t('opt_intro')), error, locGroup, exGroup, viewGroup, indexGroup);

  async function save() {
    const enabledDrives = drives.filter((d) => state.on.get(d.path)).map((d) => d.path);
    const enabled = [...enabledDrives, ...state.folders.filter((f) => state.on.get(f) !== false)];
    if (!enabled.length) {
      error.hidden = false;
      error.scrollIntoView?.({ block: 'nearest' });
      return false;
    }
    const allAuto = drives.length > 0 && drives.every((d) => !d.auto || state.on.get(d.path)) && !state.folders.length
      && drives.filter((d) => d.auto).length === enabledDrives.length;
    const roots = allAuto ? 'auto' : enabled;
    const sameRoots = JSON.stringify(roots) === JSON.stringify(origRoots);
    const sameEx = JSON.stringify(state.excluded) === JSON.stringify(getSetting('alles.excluded', []) || []);
    try {
      if (!sameRoots || !sameEx) {
        const r = await bridge.call('index.setRoots', { roots, excluded: state.excluded });
        if (r?.overridden) Toast.show({ title: t('opt_saved'), message: t('opt_override'), severity: 'info' });
      }
      if (state.hidden !== !!getSetting('alles.showHidden', false)) await setSetting('alles.showHidden', state.hidden);
      if (state.live !== (getSetting('alles.liveUpdate', true) !== false)) await setSetting('alles.liveUpdate', state.live);
      return true;
    } catch (e) {
      Toast.show({ title: t('opt_saveFailed'), message: e.message, severity: 'error' });
      return false;
    }
  }

  const pr = Dialog.show({ title: t('opt_title'), content, primary: t('opt_save'), close: t('opt_cancel'), onPrimary: save });
  const box = pr.dialog?.el;
  if (box) {
    box.classList.add('alles-opt-dialog');
    box.style.width = 'min(680px, calc(100vw - 48px))';
    box.style.maxWidth = 'min(680px, calc(100vw - 48px))';
  }
  return pr;
}
