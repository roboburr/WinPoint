// Instellingen › Update en herstel › Geavanceerde opties (notes/claude/windows-update-gnome.md §3.6, §4.12; registry
// page update-advanced, main.js MODULES; GW4, notes/claude/wu/gw4.md).
//
// Windows 11 24H2's Advanced options: the toggle rows first, without icons ("Updates downloaden via verbindingen met
// een datalimiet", "Melden wanneer opnieuw opstarten vereist is …", both settings.update.* through settings.set), then
// Ubuntu's own row D2 "Software-updater van Ubuntu automatisch openen voor gewone updates" (update.setAutoOpen; its
// description names the security-update exception, and drops the first sentence when off; the row is hidden when
// update-notifier's schema is missing), then the rows with icons, as Windows' "Additional options": automatic
// security updates and Snap Store apps (read-only, update.options) and the Software Updater ("Openen", the hero's old
// second button with the same dialog as update.js's `unsupported` hero). A mirror Settings window forwards
// setAutoOpen to the owning one (update.reply), a dry-run host changes nothing (the toggle shows the real state again).
// The page is there at once (Windows): update.options runs two programs, so the security, Snap Store and auto-open
// rows say "Even geduld…" until it answers (review m5). Refreshes held by snapd (snaps.held; `next` is then when
// the hold ends, null = no end) read in Windows 11 24H2's pause wording, "Updates zijn onderbroken tot …" (R3-UI,
// D-G44). The 24H2 parts §3.6 lacked: a description under the restart-notification row and the "Aanvullende
// opties" header above the rows with icons (R3-UI, notes/claude/round3/r3-ui.md).

import { makeT } from '../../../core/i18n.js';
import { relTime, date, time } from '../../../core/format.js';
import { SettingsCard, SettingsGroup, Button, Dialog, ToggleSwitch } from '../../../core/components.js';
import { call, get, run, ct, toggleCard, showError, noteDryRun } from '../common.js';

const t = makeT({
  nl: {
    advMetered: 'Updates downloaden via verbindingen met een datalimiet',
    advMeteredDesc: 'Er kunnen kosten in rekening worden gebracht',
    // Windows' own sentence in en ("… when a restart is required …"): a context row of tools/i18n/glossary-context.tsv
    advNotify: 'Melden wanneer opnieuw opstarten vereist is om het bijwerken te voltooien',
    advNotifyDesc: 'Een melding weergeven wanneer je pc opnieuw moet worden opgestart om het bijwerken te voltooien',
    advAutoOpen: 'Software-updater van Ubuntu automatisch openen voor gewone updates',
    // the first sentence of the description, only while on ({n} = update-notifier's interval in days; 0 = at once)
    advAutoOpenDays: {
      zero: 'Ubuntu opent de Software-updater meteen als er gewone updates zijn.',
      // "om de 1 dag" reads as "every other day" (review m7)
      one: 'Ubuntu opent de Software-updater elke dag als er gewone updates zijn.',
      other: 'Ubuntu opent de Software-updater om de {n} dagen als er gewone updates zijn.',
    },
    advAutoOpenDesc: 'Voor beveiligingsupdates opent Ubuntu hem altijd meteen. Updates kun je ook hier installeren.',
    advSecurity: 'Automatische beveiligingsupdates',
    advSecurityOn: 'Aan: Ubuntu installeert ze elke dag',
    advSecurityOff: 'Uit',
    advSnaps: 'Apps uit de Snap Store',
    advSnapsDesc: 'Worden automatisch bijgewerkt door Ubuntu. Laatst: {when}, volgende: {next}.',
    // snapd holds the refreshes (snap refresh --hold): Windows' pause wording; {until} = when the hold ends
    advSnapsHeld: 'Updates zijn onderbroken tot {until}. Laatst: {when}.',
    advSnapsHeldForever: 'Updates zijn onderbroken. Laatst: {when}.',
    advMore: 'Aanvullende opties',
    advUpdater: 'Software-updater van Ubuntu',
    open: 'Openen',
    updaterTitle: 'Software-updater openen?',
    updaterBody: 'De Software-updater van Ubuntu wordt geopend. Als je updates installeert, vraagt Ubuntu om je wachtwoord. Dat is normaal: updates veranderen het systeem. Sla je werk op, want soms moet de computer daarna opnieuw opstarten.',
  },
  en: {
    advMetered: 'Download updates over metered connections',
    advMeteredDesc: 'Data charges may apply',
    advNotify: 'Notify me when a restart is required to finish updating',
    advNotifyDesc: 'Show a notification when your PC requires a restart to finish updating',
    advAutoOpen: 'Open Ubuntu’s Software Updater automatically for regular updates',
    advAutoOpenDays: {
      zero: 'Ubuntu opens Software Updater right away when regular updates are available.',
      one: 'Ubuntu opens Software Updater every day when regular updates are available.',
      other: 'Ubuntu opens Software Updater every {n} days when regular updates are available.',
    },
    advAutoOpenDesc: 'For security updates, Ubuntu always opens it right away. You can also install updates here.',
    advSecurity: 'Automatic security updates',
    advSecurityOn: 'On: Ubuntu installs them every day',
    advSecurityOff: 'Off',
    advSnaps: 'Snap Store apps',
    advSnapsDesc: 'Updated automatically by Ubuntu. Last: {when}, next: {next}.',
    advSnapsHeld: 'Updates are paused until {until}. Last: {when}.',
    advSnapsHeldForever: 'Updates are paused. Last: {when}.',
    advMore: 'Additional options',
    advUpdater: 'Ubuntu Software Updater',
    open: 'Open',
    updaterTitle: 'Open Software Updater?',
    updaterBody: 'Ubuntu’s Software Updater will open. When you install updates, Ubuntu asks for your password. That’s normal: updates change the system. Save your work, because the computer sometimes needs to restart afterwards.',
  },
});

/** "7 uur geleden (25-10-2026 07:35)", as the Update page writes {when}; "–" when snapd has no time */
const whenText = (at) => (Number.isFinite(at) ? `${relTime(at)} (${date(at, 'short')} ${time(at)})` : '–');
/** the end of a hold, "28-10-2026 15:00" (after "tot": no relative time) */
const untilText = (at) => `${date(at, 'short')} ${time(at)}`;

/** the Snap Store row: snapd's timer, or the hold (snaps.held: `next` is the hold's end, null = until you resume) */
function snapsText(s) {
  if (!s) return ct('unavailable');
  if (s.held) {
    return Number.isFinite(s.next) ? t('advSnapsHeld', { until: untilText(s.next), when: whenText(s.last) })
      : t('advSnapsHeldForever', { when: whenText(s.last) });
  }
  return t('advSnapsDesc', { when: whenText(s.last), next: whenText(s.next) });
}

/** D2's description: the interval sentence while on, then the security-update exception */
function autoText(a) {
  if (!a.on) return t('advAutoOpenDesc');
  const n = Number.isFinite(a.days) ? a.days : 7;
  return `${t('advAutoOpenDays', { n })} ${t('advAutoOpenDesc')}`;
}

export const pages = {
  async 'update-advanced'(ctx) {
    let o = null;   // update.options; null: not answered yet, an older host without it, or a failure

    const metered = toggleCard(ctx, { title: t('advMetered'), description: t('advMeteredDesc'), path: 'update.metered' });
    const notify = toggleCard(ctx, { title: t('advNotify'), description: t('advNotifyDesc'), path: 'update.notifyRestart' });

    const autoSw = ToggleSwitch({ checked: false, ariaLabel: t('advAutoOpen'), onChange: (on) => setAuto(on) });
    // the switch appears with its real state (not a false "Uit" while update.options runs)
    autoSw.setDisabled(true);
    autoSw.hidden = true;
    const auto = SettingsCard({ title: t('advAutoOpen'), description: ct('loading'), control: autoSw, cls: 'st-updadv__auto' });
    // the three toggle rows have no icon (Windows): narrow, their switch starts under the title (settings.css)
    for (const row of [metered, notify, auto]) row.classList.add('st-updadv__row');
    const security = SettingsCard({ icon: 'shield', title: t('advSecurity'), description: ct('loading') });
    const snaps = SettingsCard({ icon: 'apps', title: t('advSnaps'), description: ct('loading') });
    const open = Button({
      label: t('open'),
      onClick: async () => {
        const ok = await Dialog.confirm({ title: t('updaterTitle'), message: t('updaterBody'), confirmLabel: t('open'), icon: 'info' });
        if (ok) run('system.openUpdater');
      },
    });
    const updater = SettingsCard({ icon: 'update', title: t('advUpdater'), control: open });

    function apply(v) {
      o = v || o;
      const a = o?.autoOpen;
      // without update-notifier's schema the row goes (Ubuntu would not open the Software Updater by itself)
      if (!a?.supported) auto.remove();
      else {
        autoSw.setChecked(!!a.on);
        autoSw.hidden = false;
        auto.setDescription(autoText(a));
      }
      security.setDescription(o?.unattended === true ? t('advSecurityOn') : o?.unattended === false ? t('advSecurityOff') : ct('unavailable'));
      snaps.setDescription(snapsText(o?.snaps));
    }

    /** a mirror Settings window: the owning one answers with update.reply {nonce, ok, error?} (at most 5 s) */
    function reply(nonce) {
      return new Promise((resolve) => {
        let off = null;
        const timer = setTimeout(() => done(null), 5000);
        function done(d) { clearTimeout(timer); off?.(); resolve(d); }
        off = ctx.on('update.reply', (d) => { if (d?.nonce === nonce) done(d); });
      });
    }

    async function setAuto(on) {
      autoSw.setDisabled(true);
      const r = await call('update.setAutoOpen', { on });
      if (!ctx.alive()) return;
      let fresh = null;
      let lost = false;
      if (!r.ok) showError(r.error);
      else if (r.value?.forwarded) {
        const d = await reply(r.value.nonce);
        if (!d) lost = true;
        else if (d.ok === false && d.error) showError(d.error);
      } else if (r.value?.dryRun) noteDryRun(r.value);
      // the answer's autoOpen only: its other rows are the host's last ones (options.rs)
      else if (r.value?.autoOpen) fresh = { ...o, autoOpen: r.value.autoOpen };
      // the real state, also after a refusal, a dry run or a forwarded write
      if (!fresh) fresh = await get('update.options', {}, o);
      if (!ctx.alive()) return;
      // the owning window did not answer (it closed, review m6): say so unless the change arrived anyway
      if (lost && fresh?.autoOpen?.on !== on) showError(null);
      apply(fresh);
      autoSw.setDisabled(false);
    }

    // the page first, then the rows that wait for update.options' programs
    get('update.options', {}, null).then((v) => {
      if (!ctx.alive()) return;
      apply(v);
      autoSw.setDisabled(false);
    });
    return [
      SettingsGroup({ children: [metered, notify, auto] }),
      // Windows 11 24H2's "Additional options": the rows with icons under their own header
      SettingsGroup({ title: t('advMore'), children: [security, snaps, updater] }),
    ];
  },
};
