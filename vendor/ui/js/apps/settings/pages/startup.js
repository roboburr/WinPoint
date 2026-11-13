// Instellingen › Apps › Opstarten: apps that start when you sign in (sysmon.startup).

import { h, clear } from '../../../core/dom.js';
import { makeT, collator } from '../../../core/i18n.js';
import { SettingsCard, SettingsGroup, ToggleSwitch, AppIcon, Dropdown, EmptyState } from '../../../core/components.js';
import { call, run, notice } from '../common.js';

const t = makeT({
  nl: {
    intro: 'Apps kunnen automatisch starten als je je aanmeldt. Zet apps uit die je niet meteen nodig hebt: je computer is dan sneller klaar.',
    sort: 'Sorteren op', byName: 'Naam', byState: 'Status',
    user: 'Alleen voor jou ingesteld', system: 'Ingesteld voor iedereen op deze pc',
    na: 'De lijst met opstart-apps is niet beschikbaar.',
    none: 'Geen opstart-apps',
    noneBody: 'Er starten geen apps automatisch als je je aanmeldt.',
    taskmgr: 'Meer details in Taakbeheer',
    taskmgrDesc: 'Bekijk opstart-apps en hun opdrachten',
  },
  en: {
    intro: 'Apps can start automatically when you sign in. Turn off apps you don’t need right away: your computer will be ready sooner.',
    sort: 'Sort by', byName: 'Name', byState: 'Status',
    user: 'Set up just for you', system: 'Set up for everyone on this PC',
    na: 'The list of startup apps isn’t available.',
    none: 'No startup apps',
    noneBody: 'No apps start automatically when you sign in.',
    taskmgr: 'More details in Task Manager',
    taskmgrDesc: 'View startup apps and their commands',
  },
});

export const pages = {
  async startup(ctx) {
    const r = await call('sysmon.startup');
    if (!r.ok || !Array.isArray(r.value)) return [notice({ message: t('na') })];
    const items = r.value;
    let sort = 'name';
    const list = h('div', { class: 'st-app-list' });
    function paint() {
      clear(list);
      if (!items.length) { list.append(EmptyState({ icon: 'startup', title: t('none'), message: t('noneBody') })); return; }
      const coll = collator();
      const sorted = [...items].sort((a, b) => (sort === 'state' ? (b.enabled - a.enabled) : 0) || coll.compare(a.name, b.name));
      for (const it of sorted) {
        const sw = ToggleSwitch({
          checked: !!it.enabled, ariaLabel: it.name,
          onChange: async (on) => {
            const ok = await run('sysmon.setStartup', { id: it.id, enabled: on }, { quietDry: false });
            if (!ok) sw.setChecked(!on); else it.enabled = on;
          },
        });
        const card = SettingsCard({ icon: AppIcon({ name: it.icon || it.id, size: 32 }), title: it.name, description: it.source === 'system' ? t('system') : t('user'), control: sw });
        card.classList.add('st-app-row');
        if (it.command) card.dataset.tooltip = it.command;
        list.append(card);
      }
    }
    paint();
    const sortDd = Dropdown({ options: [{ value: 'name', label: t('byName') }, { value: 'state', label: t('byState') }], value: sort, ariaLabel: t('sort'), onChange: (v) => { sort = v; paint(); } });
    return [
      h('p', { class: 'st-lead' }, t('intro')),
      h('div', { class: 'st-toolbar' }, h('label', { class: 'st-toolbar__field' }, h('span', null, t('sort')), sortDd)),
      SettingsGroup({ children: [list] }),
      SettingsGroup({ children: [SettingsCard({ icon: 'startup', title: t('taskmgr'), description: t('taskmgrDesc'), chevron: 'external',
        onClick: () => run('shell.openApp', { app: 'taskmgr', params: { page: 'startup' } }, { quietDry: true }) })] }),
    ];
  },
};
