// Instellingen › Update en herstel: Ubuntu release upgrades on the Update page (notes/claude/windows-update-gnome.md
// §3.5, the feature-update card "Ubuntu 26.10 is beschikbaar", and the "Nieuwe Ubuntu-versies" row of §3.6).
//
// GW3 made the stubs with their final exports; GW5 filled the Prompt row; GW6 fills the card (states C01-C16 and the
// start dialog) and adds its keys to this file's makeT dictionary; update.js does not change for that.
//
//   releaseCard(ctx, host) → HTMLElement   the card slot between the hero's InfoBars and the update rows; hidden
//                                          while no card applies ("not offered": no card at all)
//   promptRow(ctx, host)   → HTMLElement   the "Nieuwe Ubuntu-versies" row, the first item of Meer opties
//
// Each element has update(snapshot): update.js calls it after every update.status it reads (the snapshot of §4.3,
// whose `release` object carries the card's state; null when the host has no update.* service). Language switches
// re-render the whole page, so the elements are built again then.
// host = { call(method, args) → Promise<{ok, value, error}> (common.js call, never throws),
//          reload() → Promise: re-read update.status and repaint the page,
//          restartNow() → Promise: the hero's "Nu opnieuw opstarten" flow (for the rebootFirst card state),
//          product: the product name ("Win11 Shell") }.
//
// The card (GW6, mock states C01-C16 of mock/update.js; Windows 11 24H2's feature-update card: the release's logo, a
// bold title, the text, the accent button at the right, links under the text). It renders release.state of the
// snapshot: the host's verdict (compat.rs: checking, ok, warn, hold, ubuntuHold, blocked, unknown) and the
// hand-over states (GW7's upgrade.rs: updatesFirst, rebootFirst, running, waiting, startFailed, ended); the field
// contract is notes/claude/wu/gw6.md "Contract with GW7". Every text is rendered here from codes and params
// (version, eol, supportMonths, gnomeMajor); only the reasons (release.reasons[].message, the update.compat.* rows)
// and a start failure (release.startError.message) are host texts, refetched with every update.status.
//   checking + compat.needed   the page asks update.release.compat once (the host decides whether apt may run)
//   ok / warn                  "Downloaden en installeren" → the start dialog (battery, metered and, in Dutch while
//                              Ubuntu's upgrader has no Dutch catalogue, English-first InfoBars) → release.start;
//                              warn adds the attached Expander "Let op voordat je upgradet" with the reasons
//   hold / blocked             no button (D-G12); "De Software-updater van Ubuntu kan …" and the click-only C14 link
//                              update.release.ubuntuHold {on}; hold with releases.json allowAnyway adds "Toch
//                              upgraden…" → the same start dialog with the hold text as a warning InfoBar
//   updatesFirst               update.install, then the start dialog once the updates are in (this page only)
//   rebootFirst                the hero's restart flow (host.restartNow); running / waiting: update.release.attach
//                              (hidden while the hero shows releaseRunning: it says the same); startFailed: Retry goes
//                              back to the offer (the verdict) on this page; ended: "Logboek openen" (files.open) and
//                              Sluiten (hidden until the next check); unknown: Retry = update.release.compat {refresh}
// A refusal of any of these writes is an error InfoBar under the card (closable); a dry run gets the usual toast.
//
// The Prompt row (GW5, mock states M01-M04 of mock/update.js) takes the place of Windows' "Get the latest updates as
// soon as they're available": Ubuntu's own Prompt= (release.prompt, from /etc/update-manager/release-upgrades and its
// .d files) as a ComboBox, "Alleen LTS-versies" / "Elke nieuwe versie", and "Nooit melden" only while the file says
// never. The description says what the choice means (and the GNOME line under "Elke nieuwe versie"), then, when they
// apply: lts on an interim release (Ubuntu treats it as normal), a release that is out but not offered, and the .d
// file that decides (the ComboBox is disabled then, and while Ubuntu's upgrader runs: every update write is refused
// then). A change is update.release.setPrompt (pkexec: Ubuntu asks for the password); without the root helper
// (release.helper.prompt false) an InfoBar under the row explains, before and during a change, the "/bin/sh" in
// Ubuntu's password dialog (design §3.6 "helper missing"). A refusal (no password, a wrong one) is an error InfoBar
// with the host's message, and the ComboBox shows the file's value again. A mirror Settings window forwards the change
// to the owning one and waits for its update.reply. The row is hidden without a release object or without Ubuntu's
// Prompt file (prompt null).

import { h, clear } from '../../../core/dom.js';
import { makeT, lang } from '../../../core/i18n.js';
import { date } from '../../../core/format.js';
import { icon } from '../../../core/icons.js';
import { bridge } from '../../../core/bridge.js';
import {
  SettingsCard, Dropdown, InfoBar, Button, HyperlinkButton, Expander, ProgressRing, Dialog,
} from '../../../core/components.js';
import { noteDryRun, showError, run } from '../common.js';

const t = makeT({
  nl: {
    promptRow: 'Nieuwe Ubuntu-versies',
    promptLts: 'Alleen LTS-versies',
    promptNormal: 'Elke nieuwe versie',
    promptNever: 'Nooit melden',
    promptLtsDesc: 'Om de twee jaar een upgrade; elke versie krijgt 5 jaar updates.',
    promptNormalDesc: 'Elk half jaar een upgrade; een tussenversie krijgt 9 maanden updates.',
    promptNormalGnome: 'Elke tussenversie heeft een nieuwe GNOME, en {product} moet daar eerst klaar voor zijn.',
    promptLtsInterim: 'Je gebruikt een tussenversie, dus Ubuntu biedt toch elke nieuwe versie aan tot de volgende LTS.',
    relNotOffered: 'Ubuntu {version} is uitgebracht; Ubuntu biedt de upgrade nog niet aan.',
    promptOverride: 'Deze instelling wordt bepaald door {file}.',
    promptShInfo: 'Ubuntu vraagt om je wachtwoord om ‘/bin/sh’ als beheerder uit te voeren. Dat is {product}, dat alleen de regel Prompt= aanpast.',
    // GW6: the feature-update card and its start dialog (design §3.5)
    relAvailable: 'Ubuntu {version} is beschikbaar',
    relChecking: 'Controleren of deze pc klaar is voor Ubuntu {version}…',
    relOkBody: 'De nieuwste versie van Ubuntu, met nieuwe functies. Krijgt {support} updates, tot {eol}.',
    relCompatOk: '✓ {product} werkt met Ubuntu {version}.',
    relInstall: 'Downloaden en installeren',
    relWhatsNew: 'Bekijk wat er in deze update zit',
    relBefore: 'Let op voordat je upgradet',
    relOnWay: 'Ubuntu {version} is onderweg',
    relHoldGnome: 'Wacht op een compatibele Win11-versie. {product} werkt nog niet met GNOME {major}, dat bij Ubuntu {version} hoort.',
    relHoldBody: 'Wacht op een compatibele Win11-versie. Zodra {product} klaar is voor deze versie, zie je de upgrade hier.',
    relAnyway: 'Toch upgraden…',
    relAnywayStart: 'Toch upgraden',
    relUbuntuMayOffer: 'De Software-updater van Ubuntu kan deze upgrade nog wel aanbieden. Wacht daar ook mee.',
    relUbuntuWait: 'Ook de melding van Ubuntu uitstellen',
    relUbuntuWaiting: 'Ubuntu biedt deze upgrade niet meer zelf aan tot {product} er klaar voor is.',
    relUbuntuUndo: 'Ongedaan maken',
    relUbuntuHold: 'Ubuntu heeft de upgrade tijdelijk stopgezet.',
    relBlocked: 'Deze pc is nog niet klaar voor Ubuntu {version}',
    relUpdatesFirst: 'Installeer eerst de beschikbare updates.',
    relUpdatesThen: 'Updates installeren en daarna upgraden',
    relRebootFirst: 'Start je pc eerst opnieuw op om eerdere updates af te ronden. Daarna kun je upgraden.',
    restartNow: 'Nu opnieuw opstarten',
    relRunning: 'Ubuntu wordt bijgewerkt naar {version}',
    relRunningBody: 'Het venster van Ubuntu doet de upgrade. Sluit het niet.',
    relShowWindow: 'Venster weer openen',
    relWaitingBody: 'Ubuntu wacht misschien op een antwoord in het upgradevenster.',
    relNotStarted: 'De upgrade is niet gestart',
    retry: 'Opnieuw proberen',
    relEnded: 'De upgrade is gestopt voordat hij klaar was',
    relEndedBody: 'Ubuntu heeft de upgrade niet afgemaakt. In het logboek staat wat er is gebeurd.',
    relOpenLog: 'Logboek openen',
    close: 'Sluiten',
    relCheckFailed: 'De controle is niet gelukt. {reason}',
    supportMonths: { one: '1 maand', other: '{n} maanden' },
    supportYears: { one: '1 jaar', other: '{n} jaar' },
    relStartQ: 'Upgraden naar Ubuntu {version}?',
    relStartBody: 'Het upgraden doet Ubuntu zelf, in een terminalvenster. Het duurt meestal 30 tot 90 minuten en Ubuntu vraagt om je wachtwoord. Sla je werk op, sluit een laptop aan op de lader en sluit het venster niet tot Ubuntu om een herstart vraagt.',
    relStart: 'Upgrade starten',
    cancel: 'Annuleren',
    relBattery: 'Je laptop werkt op de accu. Sluit hem aan op de lader voordat je begint.',
    relMetered: 'Je gebruikt een verbinding met een datalimiet. Een upgrade downloadt vaak meer dan 2 GB.',
    relEnglishFirst: 'De eerste regels in het venster zijn in het Engels.',
  },
  en: {
    promptRow: 'New Ubuntu versions',
    promptLts: 'LTS versions only',
    promptNormal: 'Every new version',
    promptNever: 'Never notify',
    promptLtsDesc: 'An upgrade every two years; each version gets 5 years of updates.',
    promptNormalDesc: 'An upgrade every six months; an interim version gets 9 months of updates.',
    promptNormalGnome: 'Every interim version has a new GNOME, and {product} has to be ready for it first.',
    promptLtsInterim: 'You’re on an interim version, so Ubuntu still offers every new version until the next LTS.',
    relNotOffered: 'Ubuntu {version} is out; Ubuntu doesn’t offer the upgrade yet.',
    promptOverride: 'This setting is managed by {file}.',
    promptShInfo: 'Ubuntu asks for your password to run ‘/bin/sh’ as administrator. That’s {product}, changing only the Prompt= line.',
    // GW6
    relAvailable: 'Ubuntu {version} is available',
    relChecking: 'Checking whether this PC is ready for Ubuntu {version}…',
    relOkBody: 'The latest version of Ubuntu, with new features. Gets {support} of updates, until {eol}.',
    relCompatOk: '✓ {product} works with Ubuntu {version}.',
    relInstall: 'Download and install',
    relWhatsNew: 'See what’s in this update',
    relBefore: 'Before you upgrade',
    relOnWay: 'Ubuntu {version} is on its way',
    relHoldGnome: 'Wait for a compatible Win11 version. {product} doesn’t work with GNOME {major} yet, which comes with Ubuntu {version}.',
    relHoldBody: 'Wait for a compatible Win11 version. As soon as {product} is ready for this version, you’ll see the upgrade here.',
    relAnyway: 'Upgrade anyway…',
    relAnywayStart: 'Upgrade anyway',
    relUbuntuMayOffer: 'Ubuntu’s Software Updater may still offer this upgrade. Wait with it there too.',
    relUbuntuWait: 'Postpone Ubuntu’s own prompt too',
    relUbuntuWaiting: 'Ubuntu won’t offer this upgrade itself until {product} is ready for it.',
    relUbuntuUndo: 'Undo',
    relUbuntuHold: 'Ubuntu has paused the upgrade for now.',
    relBlocked: 'This PC isn’t ready for Ubuntu {version} yet',
    relUpdatesFirst: 'Install the available updates first.',
    relUpdatesThen: 'Install updates, then upgrade',
    relRebootFirst: 'Restart your PC first to finish earlier updates. Then you can upgrade.',
    restartNow: 'Restart now',
    relRunning: 'Upgrading Ubuntu to {version}',
    relRunningBody: 'Ubuntu’s window does the upgrade. Don’t close it.',
    relShowWindow: 'Show the upgrade window',
    relWaitingBody: 'Ubuntu may be waiting for an answer in the upgrade window.',
    relNotStarted: 'The upgrade didn’t start',
    retry: 'Retry',
    relEnded: 'The upgrade stopped before it finished',
    relEndedBody: 'Ubuntu didn’t finish the upgrade. The log shows what happened.',
    relOpenLog: 'Open log',
    close: 'Close',
    relCheckFailed: 'The check didn’t work. {reason}',
    supportMonths: { one: '1 month', other: '{n} months' },
    supportYears: { one: '1 year', other: '{n} years' },
    relStartQ: 'Upgrade to Ubuntu {version}?',
    relStartBody: 'Ubuntu does the upgrade itself, in a terminal window. It usually takes 30 to 90 minutes and Ubuntu asks for your password. Save your work, plug a laptop into its charger, and don’t close the window until Ubuntu asks to restart.',
    relStart: 'Start upgrade',
    cancel: 'Cancel',
    relBattery: 'Your laptop is running on battery. Plug it in before you start.',
    relMetered: 'You’re on a metered connection. An upgrade often downloads more than 2 GB.',
    relEnglishFirst: 'The first lines in the window are in English.',
  },
});

/** Ubuntu's main.log of the release upgrader (design §3.5 ended; GW7 may name another in release.logPath). */
const UPGRADE_LOG = '/var/log/dist-upgrade/main.log';
/** GW7's states (upgrade.rs): shown also when meta-release no longer offers the release (mid-upgrade). */
const HANDOVER = new Set(['updatesFirst', 'rebootFirst', 'running', 'waiting', 'startFailed', 'ended']);
/** How long a forwarded card write waits for the owning window's update.reply before the page re-reads anyway. */
const CARD_REPLY_MS = 60 * 1000;

/** "9 maanden" / "5 jaar" (design §3.5: months under 24, else years). */
function supportText(months) {
  if (!Number.isFinite(months) || months <= 0) return '';
  return months < 24 ? t('supportMonths', { n: months }) : t('supportYears', { n: Math.round(months / 12) });
}
/** distro-info's "2027-07-15" → "juli 2027" / "July 2027" */
function eolText(eol) {
  const at = typeof eol === 'string' ? Date.parse(`${eol}T12:00:00`) : NaN;
  return Number.isFinite(at) ? date(at, 'month') : '';
}

/** The feature-update card (§3.5). */
export function releaseCard(ctx, host) {
  const product = { product: host?.product || 'Win11 Shell' };
  let snap = null;              // the last update.status snapshot
  let asked = null;             // the codename update.release.compat was asked for (once per page, state checking)
  let backFrom = null;          // the startFailed/ended state the user left with Retry/Sluiten (this page only)
  let upgradeAfter = null;      // "Updates installeren en daarna upgraden": the codename whose start dialog follows
  let busy = false;             // a card write in flight: its controls stay disabled
  let failure = null;           // the last refusal (an ApiError envelope), until the next write or its close
  let dialogOpen = false;
  const el = h('div', { class: 'st-updrel' });
  el.hidden = true;

  /** the snapshot's release object, or null */
  const relOf = (s) => (s && typeof s.release === 'object' && s.release ? s.release : null);
  const versionOf = (r) => r?.version || r?.released?.version || '';
  /** the state left behind on this page (Retry after startFailed, Sluiten after ended) */
  const leftKey = (r) => `${r.state}|${r.codename}|${r.startRc ?? ''}|${r.lastCheck ?? ''}|${r.checkedAt ?? ''}`;

  /** a forwarded write (mirror window): the owner answers update.reply {nonce, ok, error?}; null when it never does */
  function reply(nonce) {
    return new Promise((resolve) => {
      let off = null;
      const timer = setTimeout(() => done(null), CARD_REPLY_MS);
      function done(d) { clearTimeout(timer); off?.(); resolve(d); }
      off = ctx.on('update.reply', (d) => { if (d?.nonce === nonce) done(d); });
    });
  }
  /** a card write: refusals become the InfoBar (by the envelope), a dry run its toast, then the page re-reads */
  async function write(method, args = {}, { quiet = false } = {}) {
    if (busy && !quiet) return null;
    if (!quiet) { busy = true; failure = null; paint(); }
    const r = await host.call(method, args);
    if (!ctx.alive()) return null;
    let value = null;
    if (!r.ok) {
      if (!quiet) { if (r.error?.code === 'unknown_method') showError(r.error); else failure = r.error || null; }
    } else if (r.value?.forwarded) {
      const d = await reply(r.value.nonce);
      if (!ctx.alive()) return null;
      if (d && d.ok === false && d.error && !quiet) failure = d.error;
      else value = r.value;
    } else {
      if (!quiet) noteDryRun(r.value);
      value = r.value;
    }
    if (!quiet) busy = false;
    await host.reload();
    if (ctx.alive()) paint();
    return value;
  }

  /* ---- the start dialog (§3.5): battery, metered and English-first InfoBars; with `anyway` the hold warning first */
  async function startDialog(anyway) {
    const r = relOf(snap);
    if (!r || dialogOpen) return;
    dialogOpen = true;
    const version = versionOf(r);
    const battery = await host.call('system.battery.get', {});
    if (!ctx.alive()) { dialogOpen = false; return; }
    const onBattery = battery.ok && battery.value?.present === true && battery.value?.onAC === false;
    const bar = (severity, message) => { const b = InfoBar({ severity, message, closable: false }); b.classList.add('st-notice'); return b; };
    const content = h('div', { class: 'st-updrel__dialog' },
      anyway ? bar('warning', holdText(r)) : null,
      h('p', { class: 'st-updrel__dialogtext' }, t('relStartBody')),
      onBattery ? bar('warning', t('relBattery')) : null,
      snap?.network?.metered ? bar('warning', t('relMetered')) : null,
      lang() === 'nl' && r.upgraderNl === false ? bar('info', t('relEnglishFirst')) : null);
    const answer = await Dialog.show({
      title: t('relStartQ', { version }), content, primary: anyway ? t('relAnywayStart') : t('relStart'),
      close: t('cancel'), icon: anyway ? 'warning' : null, width: 520,
    });
    dialogOpen = false;
    // GW7's start takes {anyway:true} only after this warning (hold + allowAnyway); ok/warn start with {}
    if (answer === 'primary' && ctx.alive()) await write('update.release.start', anyway ? { anyway: true } : {});
  }

  function holdText(r) {
    const p = { version: versionOf(r), ...product };
    return r.gnomeMajor ? t('relHoldGnome', { ...p, major: r.gnomeMajor }) : t('relHoldBody', p);
  }

  /* ---- the card's content per state (§3.5) */
  function view(r) {
    const version = versionOf(r);
    const v = { version };
    const phase = snap?.phase;
    let state = r.state;
    // Retry after startFailed / Sluiten after ended: back to the offer on this page (the host keeps its state)
    if (backFrom && backFrom === leftKey(r)) {
      if (state === 'ended') return null;
      state = r.verdict && r.verdict !== 'none' ? r.verdict : 'ok';
    } else if (backFrom && !['startFailed', 'ended'].includes(state)) backFrom = null;
    const link = (label, onClick) => { const a = HyperlinkButton({ label, onClick }); a.disabled = busy; return a; };
    const button = (label, variant, onClick) => Button({ label, variant, disabled: busy, onClick });
    const ubuntuLinks = () => (r.ubuntuPromptHeld
      ? { lines: [{ text: t('relUbuntuWaiting', product), muted: true }], links: [link(t('relUbuntuUndo'), () => write('update.release.ubuntuHold', { on: false }))] }
      : { lines: [{ text: t('relUbuntuMayOffer'), muted: true }], links: [link(t('relUbuntuWait'), () => write('update.release.ubuntuHold', { on: true }))] });
    const notes = () => (/^https?:\/\//.test(r.notesUrl || '') ? [link(t('relWhatsNew'), () => run('apps.openUri', { uri: r.notesUrl }))] : []);
    const offerBody = () => {
      const support = supportText(r.supportMonths);
      const eol = eolText(r.eol);
      return [
        support && eol ? { text: t('relOkBody', { support, eol }) } : null,
        { text: t('relCompatOk', { ...product, ...v }), ok: true },
      ].filter(Boolean);
    };
    switch (state) {
      case 'checking':
        return { title: t('relAvailable', v), checking: t('relChecking', v) };
      case 'ok':
      case 'warn':
        return {
          title: t('relAvailable', v), lines: offerBody(), links: notes(), before: state === 'warn' ? (r.reasons || []) : null,
          action: button(t('relInstall'), 'accent', () => startDialog(false)),
        };
      case 'hold': {
        const u = ubuntuLinks();
        const anyway = r.allowAnyway === true ? [link(t('relAnyway'), () => startDialog(true))] : [];
        return { title: t('relOnWay', v), lines: [{ text: holdText(r) }, ...u.lines], links: [...anyway, ...u.links] };
      }
      case 'ubuntuHold':
        return { title: t('relOnWay', v), lines: [{ text: t('relUbuntuHold') }] };
      case 'blocked': {
        const u = ubuntuLinks();
        const reasons = (r.reasons || []).filter((x) => x?.message).map((x) => ({ text: x.message }));
        return { title: t('relBlocked', v), lines: [...reasons, ...u.lines], links: u.links };
      }
      case 'updatesFirst': {
        const held = Array.isArray(r.heldNames) && r.heldNames.length ? [{ text: r.heldNames.join(', '), muted: true }] : [];
        return {
          title: t('relAvailable', v), lines: [{ text: t('relUpdatesFirst') }, ...held],
          action: button(t('relUpdatesThen'), 'accent', async () => {
            upgradeAfter = r.codename;
            const ok = await write('update.install', {});
            if (ok === null) upgradeAfter = null;
          }),
        };
      }
      case 'rebootFirst':
        return {
          title: t('relAvailable', v), lines: [{ text: t('relRebootFirst') }],
          action: button(t('restartNow'), 'standard', async () => { busy = true; paint(); try { await host.restartNow(); } finally { busy = false; if (ctx.alive()) paint(); } }),
        };
      case 'running':
        // the hero says exactly this while it shows releaseRunning (§3.2); the card adds it only next to our own job
        if (phase === 'releaseRunning' || !version) return null;
        return {
          title: t('relRunning', v), lines: [{ text: t('relRunningBody') }], busyGlyph: true,
          // GW7 sets attachable:false while Ubuntu's screen session is not there (yet): no button then
          action: r.attachable === false ? null : button(t('relShowWindow'), 'standard', () => write('update.release.attach', {})),
        };
      case 'waiting':
        if (!version) return null;
        return {
          title: t('relRunning', v), lines: [{ text: t('relWaitingBody') }], busyGlyph: true,
          // one accent button on the page: the hero has its own while it shows releaseRunning
          action: r.attachable === false ? null
            : button(t('relShowWindow'), phase === 'releaseRunning' ? 'standard' : 'accent', () => write('update.release.attach', {})),
        };
      case 'startFailed': {
        const why = r.startError?.message || (r.reasons || []).find((x) => x?.message)?.message || '';
        return {
          title: t('relNotStarted'), lines: why ? [{ text: why }] : [],
          action: button(t('retry'), 'standard', () => { backFrom = leftKey(r); failure = null; paint(); }),
        };
      }
      case 'ended':
        return {
          title: t('relEnded'), lines: [{ text: t('relEndedBody') }],
          action: [
            button(t('relOpenLog'), 'standard', () => run('files.open', { path: r.logPath || UPGRADE_LOG })),
            button(t('close'), 'standard', () => { backFrom = leftKey(r); paint(); }),
          ],
        };
      case 'unknown': {
        const reason = (r.reasons || []).find((x) => x?.message)?.message || '';
        return {
          title: t('relAvailable', v), lines: [{ text: t('relCheckFailed', { reason }).trim() }],
          action: button(t('retry'), 'standard', () => write('update.release.compat', { refresh: true })),
        };
      }
      default:
        return null;
    }
  }

  /** the release's logo (distributor-logo, then Ubuntu's pixmap), else the update glyph in a disc */
  function logo() {
    const box = h('div', { class: 'st-updrel__logo', 'aria-hidden': 'true' });
    const glyph = () => clear(box).append(h('span', { class: 'st-updrel__disc' }, icon('update', { size: 24 })));
    const url = bridge.iconUrl('distributor-logo', 96);
    // only a themed system icon from the host (the mock has none: its tiles are letters)
    if (!url.startsWith('/icon?')) { glyph(); return box; }
    const img = h('img', { src: url, alt: '', width: 48, height: 48, draggable: false });
    let tried = false;
    img.addEventListener('error', () => {
      if (!tried) { tried = true; img.src = bridge.iconUrl('/usr/share/pixmaps/ubuntu-logo.svg', 96); } else glyph();
    });
    box.append(img);
    return box;
  }
  const logoEl = logo();

  function paint() {
    const r = relOf(snap);
    // the offer's states need an offer; GW7's hand-over states show also when the offer went away meanwhile
    const shown = r && r.state && r.state !== 'none' && (r.offered !== false || HANDOVER.has(r.state));
    const m = shown ? view(r) : null;
    clear(el);
    el.hidden = !m;
    if (!m) return;
    const lines = (m.lines || []).map((x) => h('div', { class: ['st-updrel__line', x.muted && 'is-muted', x.ok && 'is-ok'] }, x.text));
    const actions = [].concat(m.action || []);
    const text = h('div', { class: 'st-updrel__text' },
      h('div', { class: 'st-updrel__title' }, m.title),
      m.checking ? h('div', { class: 'st-updrel__checking' }, ProgressRing({ size: 16 }), h('span', null, m.checking)) : null,
      ...lines,
      m.links?.length ? h('div', { class: 'st-updrel__links' }, ...m.links) : null);
    const card = h('section', { class: ['st-updrel__card', m.busyGlyph && 'is-busy'] },
      h('div', { class: 'st-updrel__inner' }, logoEl, text,
        actions.length ? h('div', { class: 'st-updrel__control' }, ...actions) : null));
    el.append(card);
    if (m.before && m.before.length) {
      const list = h('ul', { class: 'st-updrel__reasons' }, m.before.filter((x) => x?.message).map((x) => h('li', null, x.message)));
      el.append(Expander({ header: t('relBefore'), icon: 'warning', content: list, cls: 'st-updrel__before' }));
    }
    if (failure?.message) {
      const b = InfoBar({ severity: 'error', message: failure.message, closable: true, onClose: () => { failure = null; paint(); } });
      b.classList.add('st-notice');
      el.append(b);
    }
  }

  el.update = (s) => {
    snap = s || null;
    const r = relOf(snap);
    paint();
    if (!r) return;
    // an offered, tested release without a verdict: ask for the check once (the host refuses what it must not run)
    if (r.state === 'checking' && r.compat?.needed === true && r.codename && asked !== r.codename) {
      asked = r.codename;
      void write('update.release.compat', {}, { quiet: true });
    }
    // "Updates installeren en daarna upgraden": the start dialog once the updates are in and the offer stands
    if (upgradeAfter && r.codename === upgradeAfter && ['ok', 'warn'].includes(r.state) && snap.phase !== 'error') {
      upgradeAfter = null;
      void startDialog(false);
    } else if (upgradeAfter && snap.phase === 'error') upgradeAfter = null;
  };
  return el;
}

/** How long a mirror window waits for the owning window's answer (the owner waits for Ubuntu's password dialog). */
const REPLY_WAIT_MS = 10 * 60 * 1000;

/** The "Nieuwe Ubuntu-versies" row (§3.6). */
export function promptRow(ctx, host) {
  const product = { product: host?.product || 'Win11 Shell' };
  let rel = null;               // the snapshot's release object
  let running = false;          // Ubuntu's release upgrader runs (phase releaseRunning): the host refuses every write
  let wanted = null;            // the value of the setPrompt in flight (null: none)
  let failure = null;           // the last refusal (an ApiError envelope), until the next change or its close
  const dropdown = Dropdown({ width: 200, ariaLabel: t('promptRow'), onChange: (v) => change(v) });
  const card = SettingsCard({ icon: 'sparkle', title: t('promptRow'), control: dropdown });
  // a narrow card (components.css's 476 px container rule wraps the control under the text): the description is long,
  // so without this its text block took a line of its own below the icon; with a zero basis it stays beside the icon
  // and the ComboBox goes below, as on Windows. A wide card lays out exactly as before (the text fills the rest).
  const text = card.querySelector('.fl-sc__text');
  if (text) text.style.flexBasis = '0px';
  // the card and its InfoBars, spaced like the group's own items (no CSS of its own)
  const bars = h('div', { class: 'fl-settings-group__items' });
  bars.hidden = true;
  const el = h('div', { class: 'st-updprompt fl-settings-group__items' }, card, bars);
  el.hidden = true;

  const options = (value) => [
    { value: 'lts', label: t('promptLts') },
    { value: 'normal', label: t('promptNormal') },
    // "Nooit melden" exists only while Ubuntu's file says never (or no): Windows has no such choice
    ...(value === 'never' ? [{ value: 'never', label: t('promptNever') }] : []),
  ];

  function description(r) {
    const lines = [];
    if (r.prompt === 'lts') lines.push(t('promptLtsDesc'));
    if (r.prompt === 'normal') lines.push(t('promptNormalDesc'), t('promptNormalGnome', product));
    if (r.prompt === 'lts' && r.currentLts === false) lines.push(t('promptLtsInterim'));
    if (!r.offered && r.released?.version && r.prompt !== 'never') lines.push(t('relNotOffered', { version: r.released.version }));
    if (r.override) lines.push(t('promptOverride', { file: r.override }));
    return lines.map((line) => h('div', null, line));
  }

  /** the ComboBox can change the file now: no .d file decides and Ubuntu's upgrader does not run */
  const changeable = () => !rel?.override && !running;

  function paintBars() {
    clear(bars);
    // without the root helper Ubuntu's dialog names /bin/sh: say what it is before the choice and while it waits
    if (rel?.helper && rel.helper.prompt === false && (wanted || changeable())) {
      const info = InfoBar({ severity: 'info', message: t('promptShInfo', product), closable: false });
      info.classList.add('st-notice');
      bars.append(info);
    }
    if (failure?.message) {
      const bar = InfoBar({ severity: 'error', message: failure.message, closable: true, onClose: () => { failure = null; paintBars(); } });
      bar.classList.add('st-notice');
      bars.append(bar);
    }
    bars.hidden = !bars.childElementCount;
  }

  function paint() {
    if (!rel || typeof rel.prompt !== 'string') { el.hidden = true; return; }
    el.hidden = false;
    const shown = wanted || rel.prompt;
    dropdown.setOptions(options(rel.prompt === 'never' ? 'never' : shown), shown);
    dropdown.setDisabled(!!wanted || !changeable());
    card.setDescription(description(rel));
    paintBars();
  }

  /** a mirror window: the owning one answers with update.reply {nonce, ok, error?}; null when it never does */
  function reply(nonce) {
    return new Promise((resolve) => {
      let off = null;
      const timer = setTimeout(() => done(null), REPLY_WAIT_MS);
      function done(d) { clearTimeout(timer); off?.(); resolve(d); }
      off = ctx.on('update.reply', (d) => { if (d?.nonce === nonce) done(d); });
    });
  }

  async function change(value) {
    if (wanted || !rel || value === rel.prompt) return;
    wanted = value;
    failure = null;
    paint();
    const r = await host.call('update.release.setPrompt', { prompt: value });
    if (!ctx.alive()) return;
    if (!r.ok) {
      if (r.error?.code === 'unknown_method') showError(r.error);
      else failure = r.error || null;
    } else if (r.value?.forwarded) {
      const d = await reply(r.value.nonce);
      if (!ctx.alive()) return;
      if (d && d.ok === false && d.error) failure = d.error;
    } else {
      noteDryRun(r.value);
    }
    wanted = null;
    // the file's value again (also after a refusal or a dry run): the page reads update.status again
    await host.reload();
    if (ctx.alive()) paint();
  }

  el.update = (snapshot) => {
    rel = snapshot && typeof snapshot.release === 'object' ? snapshot.release : null;
    running = snapshot?.phase === 'releaseRunning' || rel?.state === 'running';
    paint();
  };
  return el;
}
