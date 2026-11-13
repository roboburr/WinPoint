// Instellingen › Apps › Standaard-apps: which app opens web pages, mail, folders, music…

import { bridge } from '../../../core/bridge.js';
import { makeT } from '../../../core/i18n.js';
import { SettingsCard, SettingsGroup, Dropdown, Toast } from '../../../core/components.js';
import { call, run, notice } from '../common.js';

const t = makeT({
  nl: {
    intro: 'Kies welke app een soort bestand of koppeling opent. Je kunt dit altijd weer wijzigen.',
    browser: 'Webbrowser', browserDesc: 'Webpagina’s en internetkoppelingen',
    mail: 'E-mail', mailDesc: 'E-mailadressen (mailto-koppelingen)',
    files: 'Bestandsbeheer', filesDesc: 'Mappen openen',
    music: 'Muziekspeler', musicDesc: 'Liedjes en geluidsbestanden',
    video: 'Videospeler', videoDesc: 'Films en video’s',
    image: 'Fotoviewer', imageDesc: 'Foto’s en afbeeldingen',
    text: 'Teksteditor', textDesc: 'Tekstbestanden (.txt)',
    terminal: 'Terminal', terminalDesc: 'Opdrachtregel',
    none: 'Geen app gekozen',
    changed: '{app} is nu je {kind}',
    overridden: '{app} is opgeslagen, maar {current} blijft je {kind}',
    overriddenBy: 'Een instelling voor deze bureaubladomgeving gaat voor: {files}. Pas de app daar aan of verwijder dat bestand.',
    overriddenUnknown: 'Een andere instelling gaat voor.',
    na: 'Standaard-apps kunnen hier niet worden gelezen.',
  },
  en: {
    intro: 'Choose which app opens a kind of file or link. You can always change this again.',
    browser: 'Web browser', browserDesc: 'Web pages and internet links',
    mail: 'Email', mailDesc: 'Email addresses (mailto links)',
    files: 'File manager', filesDesc: 'Opening folders',
    music: 'Music player', musicDesc: 'Songs and audio files',
    video: 'Video player', videoDesc: 'Films and videos',
    image: 'Photo viewer', imageDesc: 'Photos and pictures',
    text: 'Text editor', textDesc: 'Text files (.txt)',
    terminal: 'Terminal', terminalDesc: 'Command line',
    none: 'No app chosen',
    changed: '{app} is now your {kind}',
    overridden: '{app} was saved, but {current} stays your {kind}',
    overriddenBy: 'A setting for this desktop environment takes precedence: {files}. Change the app there or remove that file.',
    overriddenUnknown: 'Another setting takes precedence.',
    na: 'Default apps can’t be read here.',
  },
});

const KINDS = [
  { kind: 'browser', icon: 'globe', cats: ['WebBrowser'] },
  { kind: 'mail', icon: 'mail', cats: ['Email'] },
  { kind: 'files', icon: 'folder', cats: ['FileManager'] },
  { kind: 'music', icon: 'music', cats: ['Audio', 'Music', 'Player'], not: ['Video'] },
  { kind: 'video', icon: 'video', cats: ['Video', 'Player'] },
  { kind: 'image', icon: 'image', cats: ['Viewer', 'Photography', '2DGraphics', 'RasterGraphics', 'Graphics'] },
  { kind: 'text', icon: 'document', cats: ['TextEditor'] },
  { kind: 'terminal', icon: 'terminal', cats: ['TerminalEmulator'] },
];

export const pages = {
  async defaultapps(ctx) {
    const [dr, lr] = await Promise.all([call('apps.defaults'), call('apps.list')]);
    if (!dr.ok) return [notice({ message: t('na') })];
    const defs = dr.value || {};
    const apps = lr.ok && Array.isArray(lr.value) ? lr.value : [];

    const cards = KINDS.map((k) => {
      const cur = defs[k.kind];
      let curId = cur?.id ?? '';
      let cands = apps.filter((a) => (a.categories || []).some((c) => k.cats.includes(c)) && !(k.not || []).some((c) => (a.categories || []).includes(c)));
      if (k.kind === 'image') cands = cands.filter((a) => (a.categories || []).some((c) => ['Viewer', 'Photography'].includes(c)) || cands.length < 3);
      if (cur && !cands.some((a) => a.id === cur.id)) cands.unshift({ id: cur.id, name: cur.name, icon: cur.icon });
      const opts = cands.map((a) => ({ value: a.id, label: a.name, icon: { url: bridge.iconUrl(a.icon || a.id, 32), size: 16 } }));
      if (!opts.length) opts.push({ value: '', label: t('none'), disabled: true });
      const dd = Dropdown({
        options: opts, value: curId, width: 240, placeholder: t('none'), ariaLabel: t(k.kind),
        onChange: async (id) => {
          const r = await run('apps.setDefault', { kind: k.kind, id }, { quietDry: false });
          if (!r) { dd.setValue(curId); return; }
          const app = cands.find((a) => a.id === id);
          const kind = t(k.kind).toLowerCase();
          if (r.effective === false) {
            // written to mimeapps.list, but a <desktop>-mimeapps.list wins: say so and show what really opens
            const now = await call('apps.defaults');
            const eff = now.ok ? now.value?.[k.kind] : null;
            curId = eff?.id ?? r.current ?? curId;
            dd.setValue(curId);
            const files = Array.isArray(r.overriddenBy) ? r.overriddenBy.filter(Boolean) : [];
            const current = eff?.name || cands.find((a) => a.id === curId)?.name || r.current || t('none');
            Toast.show({ severity: 'warning', title: t('overridden', { app: app?.name || id, current, kind }),
              message: files.length ? t('overriddenBy', { files: files.join(', ') }) : t('overriddenUnknown'), timeout: 10000 });
            return;
          }
          curId = id;
          if (!r.dryRun) Toast.show({ severity: 'success', title: t('changed', { app: app?.name || id, kind }), timeout: 2500 });
        },
      });
      return SettingsCard({ icon: k.icon, title: t(k.kind), description: t(`${k.kind}Desc`), control: dd });
    });
    return [SettingsGroup({ description: t('intro'), children: cards })];
  },
};
