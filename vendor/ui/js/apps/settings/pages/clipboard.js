// Instellingen › Systeem › Klembord: explains Win+V, opens the history, clears it.

import { h } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { SettingsCard, SettingsGroup, Button, Dialog, Toast } from '../../../core/components.js';
import { call, run, notice } from '../common.js';

const t = makeT({
  nl: {
    history: 'Klembordgeschiedenis',
    historyDesc: 'Bewaar de laatste 25 dingen die je hebt gekopieerd, zodat je ze later opnieuw kunt plakken',
    howTo: 'Druk op',
    howTo2: 'om je klembordgeschiedenis te bekijken en te plakken.',
    open: 'Openen',
    count: { zero: 'Nog niets gekopieerd', one: '1 item in de geschiedenis', other: '{n} items in de geschiedenis' },
    pinnedNote: 'Vastgemaakte items blijven bewaard.',
    clear: 'Klembordgegevens wissen',
    clearDesc: 'Alles uit de geschiedenis verwijderen, behalve vastgemaakte items',
    clearBtn: 'Wissen',
    clearQ: 'Klembordgeschiedenis wissen?',
    clearBody: 'Alle gekopieerde items worden uit de geschiedenis verwijderd. Vastgemaakte items blijven bewaard. Dit kan niet ongedaan worden gemaakt.',
    cleared: 'Klembordgeschiedenis gewist',
    // D-G2: GNOME has no clipboard-history protocol for apps (host clipboard.rs: clipboard.list is always unsupported)
    na: 'Klembordgeschiedenis is hier niet beschikbaar, omdat GNOME apps niet laat bijhouden wat je kopieert. Kopiëren en plakken werken gewoon.',
    tip: 'Tip',
    tipBody: 'Kopiëren doe je met Ctrl+C, knippen met Ctrl+X en plakken met Ctrl+V.',
  },
  en: {
    history: 'Clipboard history',
    historyDesc: 'Keep the last 25 things you copied so you can paste them again later',
    howTo: 'Press',
    howTo2: 'to view and paste from your clipboard history.',
    open: 'Open',
    count: { zero: 'Nothing copied yet', one: '1 item in history', other: '{n} items in history' },
    pinnedNote: 'Pinned items are kept.',
    clear: 'Clear clipboard data',
    clearDesc: 'Remove everything from the history except pinned items',
    clearBtn: 'Clear',
    clearQ: 'Clear clipboard history?',
    clearBody: 'All copied items will be removed from the history. Pinned items are kept. This can’t be undone.',
    cleared: 'Clipboard history cleared',
    na: 'Clipboard history isn’t available here, because GNOME doesn’t let apps keep track of what you copy. Copy and paste work as usual.',
    tip: 'Tip',
    tipBody: 'Copy with Ctrl+C, cut with Ctrl+X and paste with Ctrl+V.',
  },
});

const kbd = (...keys) => h('span', { class: 'st-keys' }, keys.map((k) => h('kbd', { class: 'st-kbd' }, k)));

export const pages = {
  async clipboard(ctx) {
    const r = await call('clipboard.list');
    const supported = r.ok && r.value?.supported !== false;
    const items = supported ? r.value.items || [] : [];

    const countEl = h('span', { class: 'st-value' }, t('count', { n: items.length }));
    const clearBtn = Button({
      label: t('clearBtn'), disabled: !supported || !items.length,
      onClick: async () => {
        const ok = await Dialog.confirm({ title: t('clearQ'), message: t('clearBody'), confirmLabel: t('clearBtn'), danger: true });
        if (!ok) return;
        if (await run('clipboard.clear')) {
          Toast.show({ severity: 'success', title: t('cleared'), timeout: 2500 });
          ctx.refresh();
        }
      },
    });
    ctx.on('clipboard.changed', () => ctx.refresh());

    return [
      supported ? null : notice({ message: t('na') }),
      SettingsGroup({ children: [
        SettingsCard({
          icon: 'clipboard', title: t('history'),
          description: h('span', null, t('historyDesc'), h('br'), t('howTo'), ' ', kbd('⊞ Win', 'V'), ' ', t('howTo2')),
          control: h('div', { class: 'st-row' }, countEl,
            Button({ label: t('open'), disabled: !supported, onClick: () => run('shell.command', { command: 'clipboard' }, { quietDry: true }) })),
        }),
        SettingsCard({ icon: 'delete', title: t('clear'), description: t('clearDesc'), control: clearBtn }),
      ] }),
      SettingsGroup({ children: [SettingsCard({ icon: 'bulb', title: t('tip'), description: t('tipBody') })] }),
    ].filter(Boolean);
  },
};
