// Instellingen › Update en herstel: Ubuntu updates, settings backup (export / import),
// reset everything, about Win11 Shell.

import { h, clear } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { getShellInfo } from '../../../core/theme.js';
import { relTime, date, time } from '../../../core/format.js';
import { icon } from '../../../core/icons.js';
import { SettingsCard, SettingsGroup, SettingsExpander, Button, Dialog, Toast, ProgressRing, InfoBar } from '../../../core/components.js';
import { call, run, showError } from '../common.js';

const t = makeT({
  nl: {
    upToDate: 'Je bent up-to-date',
    available: { one: 'Er is 1 update beschikbaar', other: 'Er zijn {n} updates beschikbaar' },
    unknown: 'Kan niet controleren op updates',
    unknownDesc: 'Gebruik de Software-updater van Ubuntu om te kijken of er updates zijn.',
    checked: 'Laatst gecontroleerd: {when}',
    check: 'Controleren',
    checking: 'Controleren…',
    openUpdater: 'Software-updater openen',
    updaterTitle: 'Software-updater openen?',
    updaterBody: 'De Software-updater van Ubuntu wordt geopend. Als je updates installeert, vraagt Ubuntu om je wachtwoord. Dat is normaal: updates veranderen het systeem. Sla je werk op, want soms moet de computer daarna opnieuw opstarten.',
    open: 'Openen',
    backup: 'Back-up van instellingen',
    backupDesc: 'Bewaar je Win11 Shell-instellingen in een bestand, of zet ze terug',
    export: 'Instellingen exporteren',
    exportDesc: 'Sla al je instellingen op in een bestand (bijvoorbeeld op een USB-stick)',
    exportBtn: 'Exporteren',
    exported: 'Instellingen opgeslagen als “{name}” in je map Downloads', exportName: 'win11shell-instellingen-{date}.json',
    exportedCopy: 'Er is ook een reservekopie bewaard: {path}',
    import: 'Instellingen importeren',
    importDesc: 'Zet instellingen terug uit een eerder geëxporteerd bestand',
    importBtn: 'Bestand kiezen',
    importQ: 'Instellingen importeren?',
    importBody: 'Je huidige instellingen worden vervangen door die uit “{name}”. Tip: exporteer eerst je huidige instellingen als je ze wilt bewaren.',
    importOk: 'Importeren',
    imported: 'Instellingen geïmporteerd',
    badFile: 'Dit bestand bevat geen instellingen van Win11 Shell.',
    reset: 'Alles terugzetten',
    resetDesc: 'Alle instellingen van Win11 Shell terugzetten naar de standaardwaarden. Je bestanden blijven bewaard.',
    resetBtn: 'Terugzetten',
    resetQ: 'Alle instellingen terugzetten?',
    resetBody: 'Achtergrond, kleuren, taakbalk, Start en alle andere instellingen van Win11 Shell gaan terug naar de standaard. Je bestanden en apps blijven gewoon bewaard. Dit kan niet ongedaan worden gemaakt, tenzij je eerst exporteert.',
    resetDone: 'Alle instellingen zijn teruggezet',
    about: 'Over Win11 Shell',
    aboutDesc: 'Een Windows 11-achtig bureaublad voor Ubuntu',
    version: 'Versie', mode: 'Weergave', compositor: 'Vensterbeheer', testMode: 'Testmodus',
    yes: 'Aan (er wordt niets echt gewijzigd)',
    m_layer: 'Bureaublad', m_preview: 'Voorbeeldvenster', m_serve: 'Browser', m_app: 'Los venster',
  },
  en: {
    upToDate: 'You’re up to date',
    available: { one: '1 update is available', other: '{n} updates are available' },
    unknown: 'Can’t check for updates',
    unknownDesc: 'Use Ubuntu’s Software Updater to see whether updates are available.',
    checked: 'Last checked: {when}',
    check: 'Check',
    checking: 'Checking…',
    openUpdater: 'Open Software Updater',
    updaterTitle: 'Open Software Updater?',
    updaterBody: 'Ubuntu’s Software Updater will open. When you install updates, Ubuntu asks for your password. That’s normal: updates change the system. Save your work, because the computer sometimes needs to restart afterwards.',
    open: 'Open',
    backup: 'Settings backup',
    backupDesc: 'Save your Win11 Shell settings in a file, or restore them',
    export: 'Export settings',
    exportDesc: 'Save all your settings in a file (for example on a USB stick)',
    exportBtn: 'Export',
    exported: 'Settings saved as “{name}” in your Downloads folder', exportName: 'win11shell-settings-{date}.json',
    exportedCopy: 'A backup copy was also kept: {path}',
    import: 'Import settings',
    importDesc: 'Restore settings from a file you exported earlier',
    importBtn: 'Choose file',
    importQ: 'Import settings?',
    importBody: 'Your current settings will be replaced by those in “{name}”. Tip: export your current settings first if you want to keep them.',
    importOk: 'Import',
    imported: 'Settings imported',
    badFile: 'This file doesn’t contain Win11 Shell settings.',
    reset: 'Reset everything',
    resetDesc: 'Reset all Win11 Shell settings to their defaults. Your files are kept.',
    resetBtn: 'Reset',
    resetQ: 'Reset all settings?',
    resetBody: 'Background, colours, taskbar, Start and all other Win11 Shell settings go back to the defaults. Your files and apps are kept. This can’t be undone unless you export first.',
    resetDone: 'All settings were reset',
    about: 'About Win11 Shell',
    aboutDesc: 'A Windows 11-style desktop for Ubuntu',
    version: 'Version', mode: 'View', compositor: 'Window manager', testMode: 'Test mode',
    yes: 'On (nothing is actually changed)',
    m_layer: 'Desktop', m_preview: 'Preview window', m_serve: 'Browser', m_app: 'Separate window',
  },
});

function updateHero(ctx) {
  const glyph = h('span', { class: 'st-upd__glyph' });
  const title = h('div', { class: 'st-upd__title' });
  const sub = h('div', { class: 'st-upd__sub' });
  const checkBtn = Button({ label: t('check'), variant: 'accent' });
  const updBtn = Button({ label: t('openUpdater') });
  const el = h('section', { class: 'st-upd' }, glyph, h('div', { class: 'st-upd__text' }, title, sub), h('div', { class: 'st-upd__actions' }, checkBtn, updBtn));
  function paint(u, busy = false) {
    clear(glyph);
    if (busy) {
      glyph.append(ProgressRing({ size: 40 }));
      title.textContent = t('checking');
      sub.textContent = '';
      return;
    }
    const n = u?.available;
    el.classList.toggle('has-updates', n > 0);
    checkBtn.setVariant(n > 0 ? 'standard' : 'accent');
    updBtn.setVariant(n > 0 ? 'accent' : 'standard');
    el.classList.toggle('is-unknown', n == null);
    glyph.append(h('span', { class: 'st-upd__disc' }, icon(n == null ? 'warning' : n > 0 ? 'update' : 'success', { size: 28 })));
    title.textContent = n == null ? t('unknown') : n > 0 ? t('available', { n }) : t('upToDate');
    sub.textContent = n == null ? t('unknownDesc') : u.checkedAt ? t('checked', { when: `${relTime(u.checkedAt)} (${date(u.checkedAt, 'short')} ${time(u.checkedAt)})` }) : '';
  }
  let loading = false;
  async function load() {
    checkBtn.disabled = true;
    loading = true;
    paint(null, true);
    const r = await call('system.updates');
    loading = false;
    checkBtn.disabled = false;
    if (!ctx.alive()) return;
    paint(r.ok ? r.value : null);
  }
  checkBtn.addEventListener('click', load);
  // the host re-checks on its own now and then (system.updates.changed {available, checkedAt, checking, …})
  ctx.on('system.updates.changed', (d) => { if (d && !loading && ctx.alive()) paint(d.checking && d.available == null ? null : d, !!d.checking && d.available == null); });
  updBtn.addEventListener('click', async () => {
    const ok = await Dialog.confirm({ title: t('updaterTitle'), message: t('updaterBody'), confirmLabel: t('open'), icon: 'info' });
    if (ok) run('system.openUpdater');
  });
  el.load = load;
  return el;
}

function download(name, text) {
  const url = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  const a = h('a', { href: url, download: name, style: { display: 'none' } });
  document.body.append(a);
  a.click();
  setTimeout(() => { URL.revokeObjectURL(url); a.remove(); }, 2000);
}

export const pages = {
  async update(ctx) {
    const hero = updateHero(ctx);
    hero.load();
    const resultSlot = h('div', { class: 'st-slot' });

    const exportBtn = Button({
      label: t('exportBtn'),
      onClick: async () => {
        const r = await call('settings.export');
        if (!r.ok) { showError(r.error); return; }
        const stamp = new Date().toISOString().slice(0, 10);
        const name = r.value.name || t('exportName', { date: stamp });
        download(name, r.value.json);
        const b = await call('settings.backupFile');
        clear(resultSlot).append(InfoBar({
          severity: 'success', title: t('exported', { name }),
          message: b.ok && b.value?.path ? t('exportedCopy', { path: b.value.path }) : '',
        }));
      },
    });

    const fileInput = h('input', { type: 'file', accept: '.json,application/json', style: { display: 'none' } });
    fileInput.addEventListener('change', async () => {
      const f = fileInput.files?.[0];
      fileInput.value = '';
      if (!f) return;
      let text = '';
      try { text = await f.text(); JSON.parse(text); } catch { Toast.show({ severity: 'error', title: t('badFile'), timeout: 5000 }); return; }
      const ok = await Dialog.confirm({ title: t('importQ'), message: t('importBody', { name: f.name }), confirmLabel: t('importOk') });
      if (!ok) return;
      const r = await call('settings.import', { json: text });
      if (r.ok) { Toast.show({ severity: 'success', title: t('imported'), timeout: 2500 }); clear(resultSlot); }
      else showError(r.error, t('badFile'));
    });
    const importBtn = Button({ label: t('importBtn'), onClick: () => fileInput.click() });

    const resetBtn = Button({
      label: t('resetBtn'),
      onClick: async () => {
        const ok = await Dialog.confirm({ title: t('resetQ'), message: t('resetBody'), confirmLabel: t('resetBtn'), danger: true, icon: 'warning' });
        if (!ok) return;
        const r = await call('settings.reset', {});
        if (r.ok) Toast.show({ severity: 'success', title: t('resetDone'), timeout: 2500 });
        else showError(r.error);
      },
    });

    const sh = getShellInfo() || {};
    const mName = (m) => { const k = `m_${m}`; const v = t(k); return v === k ? m : v; };
    const aboutRows = [
      [t('version'), sh.version],
      [t('mode'), sh.mode ? mName(sh.mode) : null],
      [t('compositor'), sh.compositor ? sh.compositor[0].toUpperCase() + sh.compositor.slice(1) : null],
      [t('testMode'), sh.dryRun ? t('yes') : null],
    ];

    return [
      hero,
      SettingsGroup({ title: t('backup'), description: t('backupDesc'), children: [
        SettingsCard({ icon: 'upload', title: t('export'), description: t('exportDesc'), control: exportBtn }),
        resultSlot,
        SettingsCard({ icon: 'download', title: t('import'), description: t('importDesc'), control: [importBtn, fileInput] }),
        SettingsCard({ icon: 'restart', title: t('reset'), description: t('resetDesc'), control: resetBtn }),
      ] }),
      SettingsGroup({ children: [
        SettingsExpander({ icon: 'info', title: t('about'), description: t('aboutDesc'),
          control: h('span', { class: 'st-value' }, sh.version ? `${t('version')} ${sh.version}` : ''),
          children: [h('dl', { class: 'st-specs selectable' }, aboutRows.filter((r) => r[1]).map(([k, v]) => [h('dt', null, k), h('dd', null, v)]))] }),
      ] }),
    ];
  },
};

