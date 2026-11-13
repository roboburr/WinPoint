// Instellingen › Privacy en beveiliging: recent files (and clearing them), how Win11 Shell
// keeps you safe.

import { makeT } from '../../../core/i18n.js';
import { SettingsCard, SettingsGroup, Button, Dialog, Toast } from '../../../core/components.js';
import { call, toggleCard, showError, noteDryRun } from '../common.js';

const t = makeT({
  nl: {
    activity: 'Recente activiteit',
    recent: 'Recente bestanden bijhouden',
    recentDesc: 'Start en Verkenner laten de bestanden zien die je onlangs hebt geopend. Alleen jij ziet deze lijst; hij blijft op deze computer.',
    clear: 'Geschiedenis van recente bestanden wissen',
    clearDesc: 'De lijst met onlangs geopende bestanden leegmaken. De bestanden zelf blijven bestaan.',
    clearBtn: 'Wissen',
    clearQ: 'Recente bestanden wissen?',
    clearBody: 'De lijst met onlangs geopende bestanden wordt leeggemaakt, ook in andere apps die deze lijst gebruiken. Je bestanden zelf worden niet verwijderd.',
    cleared: 'De lijst met recente bestanden is gewist',
    clearNA: 'Wissen is in deze versie nog niet beschikbaar',
    clearNABody: 'Zet “Recente bestanden bijhouden” uit om de lijst te verbergen.',
    security: 'Zo houdt Win11 Shell je veilig',
    trash: 'Verwijderen gaat naar de Prullenbak',
    trashDesc: 'Bestanden worden nooit meteen gewist. Je kunt ze terugzetten tot je de Prullenbak leegmaakt.',
    ask: 'Eerst vragen',
    askDesc: 'Voordat er iets belangrijks gebeurt, zoals afsluiten of opruimen, vraagt Win11 Shell of je het zeker weet.',
    admin: 'Beheerdersrechten',
    adminDesc: 'Sommige taken hebben je wachtwoord nodig. Win11 Shell legt altijd eerst uit waarom, daarna vraagt Ubuntu om je wachtwoord.',
    local: 'Alles blijft op deze computer',
    localDesc: 'Win11 Shell stuurt geen gegevens over jou of je bestanden naar internet.',
  },
  en: {
    activity: 'Recent activity',
    recent: 'Keep track of recent files',
    recentDesc: 'Start and File Explorer show the files you opened recently. Only you see this list; it stays on this computer.',
    clear: 'Clear recent files history',
    clearDesc: 'Empty the list of recently opened files. The files themselves stay.',
    clearBtn: 'Clear',
    clearQ: 'Clear recent files?',
    clearBody: 'The list of recently opened files will be emptied, including in other apps that use this list. Your files themselves aren’t deleted.',
    cleared: 'The recent files list was cleared',
    clearNA: 'Clearing isn’t available in this version yet',
    clearNABody: 'Turn off “Keep track of recent files” to hide the list.',
    security: 'How Win11 Shell keeps you safe',
    trash: 'Deleted files go to the Recycle Bin',
    trashDesc: 'Files are never erased straight away. You can restore them until you empty the Recycle Bin.',
    ask: 'Asking first',
    askDesc: 'Before something important happens, like shutting down or cleaning up, Win11 Shell asks if you’re sure.',
    admin: 'Administrator rights',
    adminDesc: 'Some tasks need your password. Win11 Shell always explains why first, then Ubuntu asks for your password.',
    local: 'Everything stays on this computer',
    localDesc: 'Win11 Shell doesn’t send any data about you or your files to the internet.',
  },
});

export const pages = {
  async privacy(ctx) {
    const clearBtn = Button({
      label: t('clearBtn'),
      onClick: async () => {
        const ok = await Dialog.confirm({ title: t('clearQ'), message: t('clearBody'), confirmLabel: t('clearBtn'), danger: true });
        if (!ok) return;
        const r = await call('apps.clearRecent', {});
        if (r.ok && r.value?.dryRun) noteDryRun(r.value);
        else if (r.ok) Toast.show({ severity: 'success', title: t('cleared'), timeout: 2500 });
        else if (r.error?.code === 'unknown_method') Toast.show({ severity: 'info', title: t('clearNA'), message: t('clearNABody'), timeout: 5000 });
        else showError(r.error);
      },
    });
    return [
      SettingsGroup({ title: t('activity'), children: [
        toggleCard(ctx, { icon: 'history', title: t('recent'), description: t('recentDesc'), path: 'privacy.recentFiles' }),
        SettingsCard({ icon: 'delete', title: t('clear'), description: t('clearDesc'), control: clearBtn }),
      ] }),
      SettingsGroup({ title: t('security'), children: [
        SettingsCard({ icon: 'trash', title: t('trash'), description: t('trashDesc') }),
        SettingsCard({ icon: 'shield', title: t('ask'), description: t('askDesc') }),
        SettingsCard({ icon: 'key', title: t('admin'), description: t('adminDesc') }),
        SettingsCard({ icon: 'lock', title: t('local'), description: t('localDesc') }),
      ] }),
    ];
  },
};
