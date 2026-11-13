// Instellingen › Update en herstel › Updategeschiedenis (notes/claude/windows-update-gnome.md §3.6; registry page
// update-history, main.js MODULES; GW4, notes/claude/wu/gw4.md).
//
// Windows 11 24H2's Update history: one expander per group with its count ("Updates (12)"), each entry as a two-line
// row (name and version; "Geïnstalleerd op {date}" or "Kon niet worden geïnstalleerd op {date}" with who installed
// it), newest first. The groups are H's Updates / Ubuntu-versies / Software (C8). update.history ({before?, limit?}
// → {entries:[{group, name, version, date, ok, source, cursor}], more}) gives at most 200 entries and 90 days per
// call; "Meer laden" asks for the entries before the oldest one shown (its unique cursor, history.rs). Every text is
// rendered here from codes (source: ubuntu | us | app | terminal | null), so a language switch needs no host round
// trip (main.js re-renders the page). Under the groups, Windows' "Gerelateerde instellingen" with its Recovery row
// ("Herstel": this project's backup and reset, on Update en herstel; there is no "Updates verwijderen" here) (R3-UI).

import { h, clear } from '../../../core/dom.js';
import { makeT } from '../../../core/i18n.js';
import { date } from '../../../core/format.js';
import { SettingsExpander, SettingsCard, SettingsGroup, Button } from '../../../core/components.js';
import { call, ct, showError } from '../common.js';

const t = makeT({
  nl: {
    product: 'Win11 Shell',
    hEmpty: 'Er is nog geen updategeschiedenis.',
    hUpdates: 'Updates',
    hReleases: 'Ubuntu-versies',
    hSoftware: 'Software',
    hInstalledOn: 'Geïnstalleerd op {date}',
    hFailedOn: 'Kon niet worden geïnstalleerd op {date}',
    hByUbuntu: 'automatisch door Ubuntu',
    hByUs: 'via {product}',
    hByApp: 'via een app',
    hByTerminal: 'in een terminal (apt)',
    hLoadMore: 'Meer laden',
    hRecovery: 'Herstel',
    hRecoveryDesc: 'Back-up van instellingen, alles terugzetten',
  },
  en: {
    product: 'Win11 Shell',
    hEmpty: 'There’s no update history yet.',
    hUpdates: 'Updates',
    hReleases: 'Ubuntu versions',
    hSoftware: 'Software',
    hInstalledOn: 'Successfully installed on {date}',
    hFailedOn: 'Failed to install on {date}',
    hByUbuntu: 'automatically by Ubuntu',
    hByUs: 'via {product}',
    hByApp: 'via an app',
    hByTerminal: 'in a terminal (apt)',
    hLoadMore: 'Load more',
    hRecovery: 'Recovery',
    hRecoveryDesc: 'Settings backup, reset everything',
  },
});

/* the groups in Windows' order (updates first), their title keys and whether they start open */
const GROUPS = [['updates', 'hUpdates', true], ['releases', 'hReleases', true], ['software', 'hSoftware', false]];
const SOURCES = { ubuntu: 'hByUbuntu', us: 'hByUs', app: 'hByApp', terminal: 'hByTerminal' };

/** "Geïnstalleerd op 25-10-2026 automatisch door Ubuntu" (the source is left out when unknown) */
function dateLine(e) {
  const when = t(e.ok ? 'hInstalledOn' : 'hFailedOn', { date: date(e.date, 'short') });
  const by = SOURCES[e.source] ? t(SOURCES[e.source], { product: t('product') }) : '';
  return by ? `${when} ${by}` : when;
}

export const pages = {
  async 'update-history'(ctx) {
    const first = await call('update.history', {});
    if (!first.ok) {
      const e = first.error;
      return [h('p', { class: 'st-lead' }, e?.code === 'unknown_method' ? ct('unavailableHere') : (e?.message || ct('failed')))];
    }
    const entries = [...(first.value?.entries || [])];
    let more = !!first.value?.more;
    const open = Object.fromEntries(GROUPS.map(([id, , o]) => [id, o]));
    const root = h('div', { class: 'st-updhist' });

    function group(id, key, mine) {
      const exp = SettingsExpander({
        title: `${t(key)} (${mine.length})`,
        expanded: open[id],
        onToggle: (o) => { open[id] = o; },
        children: [h('ul', { class: 'st-updhist__list' }, mine.map((e) => h('li', { class: ['st-updhist__item', !e.ok && 'is-failed'] },
          h('div', { class: 'st-updhist__name selectable' }, [e.name, e.version].filter(Boolean).join(' ')),
          h('div', { class: 'st-updhist__date' }, dateLine(e)))))],
      });
      exp.classList.add('st-updhist__group');
      exp.dataset.group = id;
      return exp;
    }

    function paint() {
      clear(root);
      if (!entries.length) {
        root.append(h('p', { class: 'st-lead' }, t('hEmpty')));
        return;
      }
      root.append(h('div', { class: 'st-updhist__groups' }, GROUPS.map(([id, key]) => {
        const mine = entries.filter((e) => e.group === id);
        return mine.length ? group(id, key, mine) : null;
      })));
      if (more) {
        const btn = Button({ label: t('hLoadMore'), onClick: () => loadMore(btn) });
        root.append(h('div', { class: 'st-updhist__more' }, btn));
      }
      return root.querySelector('.st-updhist__more .fl-btn');
    }

    async function loadMore(btn) {
      const focused = btn === document.activeElement;
      btn.setDisabled(true);
      const last = entries[entries.length - 1];
      // the cursor (an older host: the date in ms, the design's form)
      const r = await call('update.history', last ? { before: last.cursor ?? last.date } : {});
      if (!ctx.alive()) return;
      if (!r.ok) { showError(r.error); btn.setDisabled(false); return; }
      entries.push(...(r.value?.entries || []));
      more = !!r.value?.more;
      // keyboard focus stays on "Meer laden" while there is more (paint() rebuilds the list)
      const next = paint();
      if (next && focused) next.focus();
    }

    paint();
    // Windows' Related settings: Recovery opens Update en herstel at its backup and reset group
    const related = SettingsGroup({ title: ct('related'), children: [
      SettingsCard({ icon: 'restart', title: t('hRecovery'), description: t('hRecoveryDesc'), cls: 'st-updhist__recovery',
        onClick: () => ctx.go('update', { section: 'recovery' }) }),
    ] });
    return [root, related];
  },
};
