// Instellingen › Systeem › Opslag: system drive usage, what fills the personal folder
// (coloured stacked bar + category list like Windows 11) and all drives.

import { h, clear } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { bytes, percent } from '../../../core/format.js';
import { SettingsCard, SettingsGroup, Button, ProgressRing } from '../../../core/components.js';
import { StackedBar } from '../../../core/charts.js';
import { icon } from '../../../core/icons.js';
import { call, get, run, notice, UsageBar, volumePercent } from '../common.js';

const t = makeT({
  nl: {
    used: '{u} gebruikt', free: '{f} vrij', of: '{f} vrij van {t}',
    home: 'Je persoonlijke map',
    homeDesc: 'Wat er in {p} ruimte inneemt',
    calculating: 'Bezig met berekenen…',
    calcDesc: 'Win11 Shell bekijkt welke bestanden de meeste ruimte innemen. Dit kan even duren.',
    more: 'Meer categorieën weergeven',
    fewer: 'Minder categorieën weergeven',
    cleanup: 'Ruimte vrijmaken',
    cleanupDesc: 'Bekijk wat veilig kan worden opgeruimd, zoals de Prullenbak en tijdelijke bestanden',
    cleanupBtn: 'Ruimte vrijmaken',
    drives: 'Stations',
    drivesDesc: 'Alle schijven en USB-sticks op deze pc',
    almostFull: 'Bijna vol',
    removable: 'Verwisselbaar',
    na: 'Opslaginformatie is op deze computer niet beschikbaar.',
    categoriesNA: 'Kan niet berekenen wat er in je persoonlijke map staat.',
    systemDrive: 'Systeemschijf',
    openFolder: 'Openen in Verkenner',
    c_documents: 'Documenten',
    c_pictures: 'Afbeeldingen',
    c_videos: 'Video’s',
    c_music: 'Muziek',
    c_downloads: 'Downloads',
    c_desktop: 'Bureaublad',
    c_trash: 'Prullenbak',
    c_browser: 'Browsers',
    c_chat: 'Chat-apps',
    c_dev: 'Ontwikkeltools',
    c_appdata: 'App-gegevens',
    c_snaps: 'Snap-apps',
    c_flatpak: 'Flatpak-apps',
    c_cache: 'Tijdelijke bestanden (cache)',
    c_other: 'Overige',
  },
  en: {
    used: '{u} used', free: '{f} free', of: '{f} free of {t}',
    home: 'Your personal folder',
    homeDesc: 'What takes up space in {p}',
    calculating: 'Calculating…',
    calcDesc: 'Win11 Shell is looking at which files take up the most space. This can take a moment.',
    more: 'Show more categories',
    fewer: 'Show fewer categories',
    cleanup: 'Free up space',
    cleanupDesc: 'See what can safely be cleaned up, such as the Recycle Bin and temporary files',
    cleanupBtn: 'Free up space',
    drives: 'Drives',
    drivesDesc: 'All disks and USB sticks on this PC',
    almostFull: 'Almost full',
    removable: 'Removable',
    na: 'Storage information isn’t available on this computer.',
    categoriesNA: 'Can’t calculate what’s in your personal folder.',
    systemDrive: 'System drive',
    openFolder: 'Open in File Explorer',
    c_documents: 'Documents',
    c_pictures: 'Pictures',
    c_videos: 'Videos',
    c_music: 'Music',
    c_downloads: 'Downloads',
    c_desktop: 'Desktop',
    c_trash: 'Recycle Bin',
    c_browser: 'Browsers',
    c_chat: 'Chat apps',
    c_dev: 'Developer tools',
    c_appdata: 'App data',
    c_snaps: 'Snap apps',
    c_flatpak: 'Flatpak apps',
    c_cache: 'Temporary files (cache)',
    c_other: 'Other',
  },
});

const CAT_ICON = {
  documents: 'document', pictures: 'image', videos: 'video', music: 'music', downloads: 'download', desktop: 'desktop',
  trash: 'trash', browser: 'globe', chat: 'people', dev: 'code', appdata: 'app', snaps: 'apps', flatpak: 'apps', cache: 'clean', other: 'folder',
};
const CAT_COLOR = {
  documents: 'var(--chart-1)', pictures: 'var(--chart-2)', videos: 'var(--chart-4)', music: 'var(--chart-5)',
  downloads: 'var(--chart-6)', desktop: 'var(--chart-3)', trash: 'var(--chart-8)', browser: 'var(--chart-7)',
  chat: '#e74856', dev: '#8e8cd8', appdata: '#00b7c3', snaps: '#ea6d2c', flatpak: '#4f9ce8', cache: '#a0a0a0', other: '#6e7c8c',
};

const catName = (c) => { const k = `c_${c.id}`; const v = t(k); return v !== k ? v : (c.name || c.id); };

function volumeCard(v, onOpen) {
  const pct = volumePercent(v);
  const full = pct >= 90;
  // same naming as Verkenner › Deze pc: no raw /run/media/<user>/<UUID> folders (jargon); other mounts keep their folder
  const name = v.isSystem ? (v.name || t('systemDrive')) : (v.name || v.label || v.device);
  const title = /^\/(media|run\/media|mnt)\//.test(v.mount || '') || name.includes(v.mount) ? name : `${name} (${v.mount})`;
  const card = SettingsCard({
    icon: v.removable || v.kind === 'usb' ? 'usb' : 'drive',
    title,
    description: h('span', { class: 'st-vol' },
      UsageBar({ pct }),
      h('span', { class: ['st-vol__text', full && 'is-full'] }, t('of', { f: bytes(v.free), t: bytes(v.total) }), full ? ` · ${t('almostFull')}` : '')),
    control: h('span', { class: 'st-value tabular' }, percent(pct)),
    onClick: onOpen,
  });
  card.classList.add('st-vol-card');
  return card;
}

export const pages = {
  async storage(ctx) {
    const [volsR, catsR] = await Promise.all([call('storage.volumes'), call('storage.categories', { refresh: false })]);
    const vols = volsR.ok && Array.isArray(volsR.value) ? volsR.value : null;
    const out = [];
    if (!vols) out.push(notice({ message: t('na') }));

    const sys = vols?.find((v) => v.isHome) || vols?.find((v) => v.isSystem) || vols?.[0];
    const openPath = (path) => run('shell.openApp', { app: 'explorer', params: { path } }, { quietDry: true });

    /* ---- summary of the drive with the home folder ------------------------ */
    if (sys) {
      const pct = volumePercent(sys);
      out.push(h('section', { class: 'st-storage-sum' },
        h('div', { class: 'st-storage-sum__head' },
          h('div', { class: 'st-storage-sum__title' }, `${sys.name || t('systemDrive')} (${sys.mount}) – ${bytes(sys.total)}`),
          h('div', { class: 'st-storage-sum__nums' },
            h('span', null, t('used', { u: bytes(sys.used) })),
            h('span', null, t('free', { f: bytes(sys.free) })))),
        UsageBar({ pct })));
    }

    /* ---- categories ------------------------------------------------------------ */
    const catSlot = h('div', { class: 'st-slot' });
    let showAll = false;
    let catData = catsR.ok && catsR.value?.ready ? catsR.value.data : null;

    function renderCats() {
      clear(catSlot);
      if (!catsR.ok && !catData) {
        catSlot.append(SettingsGroup({ title: t('home'), children: [notice({ message: t('categoriesNA') })] }));
        return;
      }
      if (!catData) {
        catSlot.append(SettingsGroup({ title: t('home'), children: [
          SettingsCard({ icon: ProgressRing({ size: 20 }), title: t('calculating'), description: t('calcDesc') }),
        ] }));
        return;
      }
      const cats = (catData.categories || []).filter((c) => c.size > 0).sort((a, b) => b.size - a.size);
      const total = cats.reduce((a, c) => a + c.size, 0);
      const max = cats[0]?.size || 1;
      const shown = showAll ? cats : cats.slice(0, 6);
      const bar = StackedBar({
        segments: cats.map((c) => ({ value: c.size, color: CAT_COLOR[c.id] || 'var(--chart-8)', label: catName(c) })),
        total, height: 10, legend: false, format: (v) => bytes(v),
      });
      const rows = shown.map((c) => {
        const card = SettingsCard({
          icon: CAT_ICON[c.id] || 'folder',
          title: catName(c),
          description: h('span', { class: 'st-cat__bar' }, UsageBar({ pct: (c.size / max) * 100, color: CAT_COLOR[c.id], warnAt: 101 })),
          control: h('span', { class: 'st-value tabular' }, bytes(c.size)),
          onClick: c.path ? () => openPath(c.path) : undefined,
        });
        card.classList.add('st-cat');
        card.style.setProperty('--cat', CAT_COLOR[c.id] || 'var(--chart-8)');
        if (c.path) card.dataset.tooltip = t('openFolder');
        return card;
      });
      const moreToggle = cats.length > 6 ? SettingsCard({
        title: showAll ? t('fewer') : t('more'),
        onClick: () => { showAll = !showAll; renderCats(); },
        chevron: false,
        control: h('span', { class: ['st-more-chev', showAll && 'is-open'] }, icon('chevron-down', { size: 12 })),
      }) : null;
      if (moreToggle) moreToggle.classList.add('st-more');
      catSlot.append(SettingsGroup({
        title: t('home'),
        description: t('homeDesc', { p: catData.home || '~' }),
        children: [
          h('div', { class: 'st-catbar-card' }, bar,
            h('div', { class: 'st-catbar-legend' }, cats.slice(0, 8).map((c) => h('span', { class: 'st-catbar-key' },
              h('i', { style: { background: CAT_COLOR[c.id] || 'var(--chart-8)' } }), catName(c))))),
          ...rows, moreToggle,
        ].filter(Boolean),
      }));
    }
    renderCats();
    if (!catData) {
      ctx.on('storage.categoriesReady', (d) => { if (d?.data) { catData = d.data; renderCats(); } });
      // fall back to polling in case the event was missed
      ctx.timeout(async () => {
        if (catData) return;
        const r = await get('storage.categories', { refresh: false });
        if (r?.ready && !catData) { catData = r.data; renderCats(); }
      }, 4000);
    }
    out.push(catSlot);

    /* ---- clean up ------------------------------------------------------------- */
    out.push(SettingsGroup({ children: [SettingsCard({
      icon: 'broom', title: t('cleanup'), description: t('cleanupDesc'),
      control: Button({ label: t('cleanupBtn'), variant: 'accent', onClick: () => run('shell.openApp', { app: 'taskmgr', params: { page: 'storage' } }, { quietDry: true }) }),
    })] }));

    /* ---- drives ----------------------------------------------------------------- */
    if (vols?.length) {
      out.push(SettingsGroup({
        title: t('drives'), description: t('drivesDesc'),
        children: vols.map((v) => volumeCard(v, () => openPath(v.mount))),
      }));
    }
    return out;
  },
};
