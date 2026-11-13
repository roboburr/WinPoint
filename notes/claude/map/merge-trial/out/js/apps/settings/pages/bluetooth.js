// Instellingen › Bluetooth en apparaten (SPEC §6.10): power toggle, device tiles (connect/disconnect, remove,
// battery), the Win11 "Een apparaat toevoegen" dialog (search → pick → confirm code / passkey / PIN → done or
// failed) and the "Bluetooth reageert niet" banner with "Bluetooth opnieuw starten" (system.bluetooth.reset).
//
// Deep link: shell.openApp {app:'settings', params:{page:'bluetooth', add:1}} opens "Een apparaat toevoegen"
// (quick settings › Bluetooth › Apparaat toevoegen). While the dialog lists devices it renews
// system.bluetooth.discover every 20 s and stops the search when it closes; pairing progress arrives as
// system.bluetooth.pairing {address, stage, passkey?, pin?, entered?}; questions are answered with pairReply.

import { h, clear } from '../../../core/dom.js';
import { bridge } from '../../../core/bridge.js';
import { makeT, collator } from '../../../core/i18n.js';
import { icon } from '../../../core/icons.js';
import {
  SettingsCard, SettingsGroup, ToggleSwitch, IconButton, Button, Dialog, EmptyState, InfoBar, ProgressBar, ProgressRing,
  TextBox, severityIcon,
} from '../../../core/components.js';
import { showContextMenu } from '../../../core/contextmenu.js';
import { call, get, notice, linkCard, showError } from '../common.js';

const t = makeT({
  nl: {
    bluetooth: 'Bluetooth',
    discoverable: 'Detecteerbaar als “{name}”',
    off: 'Bluetooth staat uit',
    add: 'Apparaat toevoegen',
    addTitle: 'Een apparaat toevoegen',
    connected: 'Verbonden', paired: 'Gekoppeld', notConnected: 'Niet verbonden',
    connect: 'Verbinden', disconnect: 'Verbinding verbreken',
    connecting: 'Verbinden…', disconnecting: 'Verbinding verbreken…',
    remove: 'Apparaat verwijderen', removing: 'Verwijderen…',
    removeTitle: 'Weet je zeker dat je dit apparaat wilt verwijderen?',
    removeBody: '“{name}” wordt vergeten, ook de koppeling. Wil je het later weer gebruiken, zet het dan in de koppelstand en voeg het opnieuw toe.',
    yes: 'Ja', no: 'Nee',
    more: 'Meer opties',
    battery: 'Batterij {n}%',
    batteryShort: '{n}%',
    devices: 'Apparaten',
    noDevices: 'Nog geen apparaten',
    noDevicesBody: 'Kies “Apparaat toevoegen” om een koptelefoon, muis, toetsenbord of ander Bluetooth-apparaat te koppelen.',
    na: 'Er is geen Bluetooth gevonden op deze computer.',
    naBody: 'Heeft je computer geen Bluetooth, dan kun je een kleine Bluetooth-USB-stick gebruiken.',
    thisPc: 'deze pc',
    related: 'Gerelateerde instellingen',
    soundDesc: 'Kies je koptelefoon of speaker als geluidsapparaat',
    powerFailed: 'Bluetooth aan- of uitzetten lukte niet',
    actionFailed: 'Dat lukte niet',
    // the stuck adapter
    stuckTitle: 'Bluetooth reageert niet',
    stuckMsg: 'Je apparaten kunnen nu geen verbinding maken. Start Bluetooth opnieuw om dit op te lossen.',
    reset: 'Bluetooth opnieuw starten',
    resetting: 'Bluetooth wordt opnieuw gestart…',
    resettingMsg: 'Dit duurt een paar seconden.',
    resetOk: 'Bluetooth werkt weer',
    resetOkMsg: 'Je kunt je apparaten weer verbinden.',
    resetFailed: 'Bluetooth opnieuw starten lukte niet',
    resetFailedMsg: 'Probeer het nog een keer. Lukt het dan niet, start de computer opnieuw op.',
    resetCardDesc: 'Helpt als Bluetooth niet meer reageert of apparaten geen verbinding meer maken',
    resetBtn: 'Opnieuw starten',
    retry: 'Opnieuw proberen',
    // "Een apparaat toevoegen"
    chooseLead: 'Kies het type apparaat dat je wilt toevoegen.',
    kindBt: 'Bluetooth',
    kindBtDesc: 'Muizen, toetsenborden, pennen of audio- en andere soorten Bluetooth-apparaten',
    kindCast: 'Draadloos beeldscherm of dock',
    kindCastDesc: 'Draadloze beeldschermen, tv’s of pc’s die Miracast gebruiken, of draadloze docks',
    kindOther: 'Alle overige',
    kindOtherDesc: 'Xbox-controllers met draadloze adapter, DLNA en meer',
    notAvailable: 'Niet beschikbaar',
    listLead: 'Zorg ervoor dat je apparaat is ingeschakeld en detecteerbaar is. Selecteer hieronder een apparaat om verbinding te maken.',
    searching: 'Zoeken naar apparaten…',
    notFoundYet: 'Zie je je apparaat niet? Zet het in de koppelstand: meestal houd je de aan-knop of de Bluetooth-knop een paar seconden ingedrukt, tot een lampje knippert.',
    searchFailed: 'Zoeken naar apparaten lukte niet.',
    btOffTitle: 'Bluetooth staat uit',
    btOffMsg: 'Zet Bluetooth aan om apparaten te zoeken.',
    turnOn: 'Aanzetten',
    pairing: 'Verbinding maken…',
    pairedConnecting: 'Gekoppeld. Verbinding maken…',
    cancelling: 'Annuleren…',
    confirmText: 'Druk op Verbinden als de pincode op je apparaat overeenkomt.',
    authorizeText: '“{name}” wil verbinding maken met deze pc. Druk op Verbinden om dat toe te staan.',
    passkeyText: 'Voer de pincode in die op “{name}” wordt weergegeven.',
    pinText: 'Voer de pincode voor “{name}” in. Staat die niet op het apparaat of in de handleiding, probeer dan 0000 of 1234.',
    displayText: 'Typ deze pincode op “{name}” en druk daarna op Enter.',
    displayPinText: 'Typ deze pincode op “{name}”.',
    pinLabel: 'Pincode',
    badPasskey: 'Typ de zes cijfers die het apparaat laat zien.',
    badPin: 'Typ een pincode van 1 tot 16 letters of cijfers.',
    connectBtn: 'Verbinden',
    cancel: 'Annuleren',
    doneTitle: 'Je apparaat is klaar om te gebruiken!',
    doneBtn: 'Gereed',
    failedTitle: 'Verbinden is niet gelukt',
    failedGeneric: 'Koppelen lukte niet. Probeer het opnieuw.',
    dryRun: 'Testmodus: er is niets gekoppeld.',
    k_headphones: 'Koptelefoon', k_headset: 'Headset', k_speaker: 'Luidspreker', k_phone: 'Telefoon',
    k_computer: 'Computer', k_keyboard: 'Toetsenbord', k_mouse: 'Muis', k_gamepad: 'Controller', k_pen: 'Pen',
    k_display: 'Beeldscherm', k_camera: 'Camera', k_printer: 'Printer', k_watch: 'Horloge', k_network: 'Netwerk',
    k_media: 'Audio',
  },
  en: {
    bluetooth: 'Bluetooth',
    discoverable: 'Discoverable as “{name}”',
    off: 'Bluetooth is off',
    add: 'Add device',
    addTitle: 'Add a device',
    connected: 'Connected', paired: 'Paired', notConnected: 'Not connected',
    connect: 'Connect', disconnect: 'Disconnect',
    connecting: 'Connecting…', disconnecting: 'Disconnecting…',
    remove: 'Remove device', removing: 'Removing…',
    removeTitle: 'Are you sure you want to remove this device?',
    removeBody: '“{name}” will be forgotten, including its pairing. To use it again later, put it in pairing mode and add it again.',
    yes: 'Yes', no: 'No',
    more: 'More options',
    battery: 'Battery {n}%',
    batteryShort: '{n}%',
    devices: 'Devices',
    noDevices: 'No devices yet',
    noDevicesBody: 'Choose “Add device” to pair headphones, a mouse, a keyboard or another Bluetooth device.',
    na: 'No Bluetooth was found on this computer.',
    naBody: 'If your computer has no Bluetooth, you can use a small Bluetooth USB adapter.',
    thisPc: 'this PC',
    related: 'Related settings',
    soundDesc: 'Choose your headphones or speaker as the sound device',
    powerFailed: 'Couldn’t turn Bluetooth on or off',
    actionFailed: 'That didn’t work',
    stuckTitle: 'Bluetooth isn’t responding',
    stuckMsg: 'Your devices can’t connect right now. Restart Bluetooth to fix this.',
    reset: 'Restart Bluetooth',
    resetting: 'Restarting Bluetooth…',
    resettingMsg: 'This takes a few seconds.',
    resetOk: 'Bluetooth works again',
    resetOkMsg: 'You can connect your devices again.',
    resetFailed: 'Couldn’t restart Bluetooth',
    resetFailedMsg: 'Try once more. If that doesn’t help, restart the computer.',
    resetCardDesc: 'Helps when Bluetooth stops responding or devices no longer connect',
    resetBtn: 'Restart',
    retry: 'Try again',
    chooseLead: 'Choose the kind of device you want to add.',
    kindBt: 'Bluetooth',
    kindBtDesc: 'Mice, keyboards, pens, or audio and other kinds of Bluetooth devices',
    kindCast: 'Wireless display or dock',
    kindCastDesc: 'Wireless monitors, TVs, or PCs that use Miracast, or wireless docks',
    kindOther: 'Everything else',
    kindOtherDesc: 'Xbox controllers with Wireless Adapter, DLNA, and more',
    notAvailable: 'Not available',
    listLead: 'Make sure your device is turned on and discoverable. Select a device below to connect.',
    searching: 'Looking for devices…',
    notFoundYet: 'Don’t see your device? Put it in pairing mode: usually you hold the power or Bluetooth button for a few seconds until a light blinks.',
    searchFailed: 'Searching for devices failed.',
    btOffTitle: 'Bluetooth is off',
    btOffMsg: 'Turn on Bluetooth to look for devices.',
    turnOn: 'Turn on',
    pairing: 'Connecting…',
    pairedConnecting: 'Paired. Connecting…',
    cancelling: 'Cancelling…',
    confirmText: 'Press Connect if the PIN on your device matches this one.',
    authorizeText: '“{name}” wants to connect to this PC. Press Connect to allow it.',
    passkeyText: 'Enter the PIN that “{name}” shows.',
    pinText: 'Enter the PIN for “{name}”. If it isn’t on the device or in its manual, try 0000 or 1234.',
    displayText: 'Type this PIN on “{name}”, then press Enter.',
    displayPinText: 'Type this PIN on “{name}”.',
    pinLabel: 'PIN',
    badPasskey: 'Type the six digits the device shows.',
    badPin: 'Type a PIN of 1 to 16 letters or digits.',
    connectBtn: 'Connect',
    cancel: 'Cancel',
    doneTitle: 'Your device is ready to go!',
    doneBtn: 'Done',
    failedTitle: 'Couldn’t connect',
    failedGeneric: 'Pairing failed. Try again.',
    dryRun: 'Test mode: nothing was paired.',
    k_headphones: 'Headphones', k_headset: 'Headset', k_speaker: 'Speaker', k_phone: 'Phone', k_computer: 'Computer',
    k_keyboard: 'Keyboard', k_mouse: 'Mouse', k_gamepad: 'Controller', k_pen: 'Pen', k_display: 'Display',
    k_camera: 'Camera', k_printer: 'Printer', k_watch: 'Watch', k_network: 'Network', k_media: 'Audio',
  },
});

/* -------------------------------------------------------------- glyphs */

export function deviceGlyph(name = '') {
  const n = String(name).toLowerCase();
  if (/head(phone|set)|earbud|audio-head/.test(n)) return 'headphones';
  if (/mouse/.test(n)) return 'mouse';
  if (/keyboard/.test(n)) return 'keyboard';
  if (/phone|smartphone/.test(n)) return 'phone';
  if (/gam(e|ing)|joystick/.test(n)) return 'gamepad';
  if (/speaker|audio|sound/.test(n)) return 'sound';
  if (/print/.test(n)) return 'print';
  if (/camera|video/.test(n)) return 'camera';
  return 'devices';
}

// host bt_kind() → icon name, or an own drawing (same 20×20 grid and stroke as core/icons.js)
const KIND_ICON = {
  headphones: 'headphones', phone: 'phone', computer: 'pc', keyboard: 'keyboard', mouse: 'mouse', gamepad: 'gamepad',
  pen: 'edit', display: 'display', camera: 'camera', printer: 'print', network: 'network', media: 'sound', other: 'bluetooth',
};
const KIND_DRAWING = {
  headset: '<path d="M3.5 12.5V10a6.5 6.5 0 0 1 13 0v2.5"/><rect x="3" y="11" width="3.5" height="5" rx="1.5"/>'
    + '<rect x="13.5" y="11" width="3.5" height="5" rx="1.5"/><path d="M15.25 16v.5a2 2 0 0 1-2 2H11"/>',
  speaker: '<rect x="5" y="2.5" width="10" height="15" rx="2"/><circle cx="10" cy="11.75" r="2.75"/>'
    + '<circle cx="10" cy="6" r=".9" fill="currentColor" stroke="none"/>',
  watch: '<rect x="5" y="5" width="10" height="10" rx="3"/><path d="M7 5l.5-2.5h5L13 5M7 15l.5 2.5h5L13 15"/>'
    + '<path d="M10 7.75V10l1.5 1"/>',
};

function drawing(body, size) {
  const el = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [k, v] of Object.entries({ viewBox: '0 0 20 20', fill: 'none', stroke: 'currentColor', 'stroke-linecap': 'round',
    'stroke-linejoin': 'round', 'aria-hidden': 'true', focusable: 'false', width: size, height: size, class: 'fl-icon' })) el.setAttribute(k, String(v));
  el.innerHTML = body; // static markup from KIND_DRAWING
  return el;
}

/** Icon for a device: the host's kind (bt_kind), else guessed from its BlueZ icon name or its name. */
export function deviceIcon(d = {}, size = 24) {
  if (KIND_DRAWING[d.kind]) return drawing(KIND_DRAWING[d.kind], size);
  if (KIND_ICON[d.kind] && !(d.kind === 'other' && d.icon && deviceGlyph(d.icon) !== 'devices')) return icon(KIND_ICON[d.kind], { size });
  return icon(deviceGlyph(d.icon || d.name), { size });
}

const KINDS = new Set(['headphones', 'headset', 'speaker', 'phone', 'computer', 'keyboard', 'mouse', 'gamepad', 'pen', 'display',
  'camera', 'printer', 'watch', 'network', 'media']);
const kindLabel = (kind) => (KINDS.has(kind) ? t(`k_${kind}`) : '');

/* ------------------------------------------------------------- helpers */

const errText = (e) => (e && typeof e === 'object' ? e.message || e.error || '' : String(e || ''));
/** The host's NotReady answer ("Bluetooth reageert niet. …"): its code (every BT answer has one, also the {ok:false}
 *  results) or its catalog key (E2), never its text, which is in the user's language (§2.10). */
export function isNotResponding(e) {
  return !!e && typeof e === 'object' && (e.code === 'not_ready' || e.code === 'not_responding' || e.key === 'system.bt_not_responding');
}

export function deviceStatus(d) {
  return [d.connected ? t('connected') : d.paired ? t('paired') : t('notConnected'), d.battery != null ? t('battery', { n: Math.round(d.battery) }) : '']
    .filter(Boolean).join(' · ');
}

/** call() → {ok, value, error}; a host answer {ok:false, error} counts as failed too. */
async function attempt(method, args) {
  const r = await call(method, args);
  if (!r.ok) return { ok: false, error: r.error };
  if (r.value && r.value.ok === false) return { ok: false, error: { message: r.value.error || '', code: r.value.code }, value: r.value };
  return { ok: true, value: r.value ?? {} };
}

/** Connect / disconnect a paired device → {ok, error?} (the host answers {ok:false, error, code} on failure;
 *  error.code 'not_ready' = the adapter is stuck: see isNotResponding). */
export async function toggleDevice(d) {
  const r = await attempt(d.connected ? 'system.bluetooth.disconnect' : 'system.bluetooth.connect', { address: d.address });
  return r.ok ? { ok: true, value: r.value } : { ok: false, error: r.error };
}

/** Win11 "Apparaat verwijderen": ask first, then forget the device and its pairing → true | false | {error}.
 *  onConfirmed() runs after "Ja", before the host is asked (a tile shows "Verwijderen…" only from then on). */
export async function removeDevice(d, { onConfirmed } = {}) {
  const name = d.name || d.address;
  const sure = await Dialog.confirm({ title: t('removeTitle'), message: t('removeBody', { name }), confirmLabel: t('yes'), cancelLabel: t('no') });
  if (!sure) return false;
  onConfirmed?.();
  const r = await attempt('system.bluetooth.remove', { address: d.address });
  return r.ok ? true : { error: r.error };
}

/* ---------------------------------------------- "Bluetooth opnieuw starten" */

// One reset at a time, shared by the page banner, the reset card and the add-device dialog.
let resetJob = null;
const resetSubs = new Set();
const resetNotify = (s) => { for (const cb of [...resetSubs]) { try { cb(s); } catch (e) { console.error('[bluetooth] reset listener', e); } } };
export const resetRunning = () => !!resetJob;
// "… lukte niet" is the banner's title already: for code 'failed' the message says what to do next (once more,
// then restart the computer, matching its "Opnieuw proberen" button); other codes (blocked, unavailable,
// offline) keep the host's own text
const resetErrorText = (e) => (!e || e.code === 'failed' || !errText(e) ? t('resetFailedMsg') : errText(e));

/** system.bluetooth.reset → {ok, state?, error?}; listeners hear {busy:true} first, then the outcome. */
export function resetBluetooth() {
  if (resetJob) return resetJob;
  resetNotify({ busy: true });
  resetJob = call('system.bluetooth.reset').then((r) => {
    const v = r.ok ? r.value : null;
    const out = r.ok ? { ok: true, state: v && v.available != null ? v : null } : { ok: false, error: resetErrorText(r.error) };
    resetJob = null;
    resetNotify(out);
    return out;
  });
  return resetJob;
}

/** The "Bluetooth reageert niet" InfoBar: modes null | 'stuck' | 'busy' | 'ok' | 'error'. */
function stuckBar({ compact = false } = {}) {
  const bar = InfoBar({ severity: 'warning', title: '', message: '', closable: false });
  bar.classList.add('st-notice', 'st-bt-stuck');
  if (compact) bar.classList.add('is-compact');
  let mode;
  bar.setMode = (m, error = '') => {
    const key = `${m}|${error}`;
    if (key === mode) return;
    mode = key;
    bar.hidden = !m;
    if (!m) return;
    if (m === 'busy') {
      bar.setSeverity('info');
      bar.setTitle(t('resetting'));
      bar.setMessage(t('resettingMsg'));
      bar.setAction(h('span', { class: 'st-bt-stuck__busy' }, ProgressRing({ size: 20, ariaLabel: t('resetting') })));
    } else if (m === 'ok') {
      bar.setSeverity('success');
      bar.setTitle(t('resetOk'));
      bar.setMessage(t('resetOkMsg'));
      bar.setAction(null);
    } else if (m === 'error') {
      bar.setSeverity('error');
      bar.setTitle(t('resetFailed'));
      bar.setMessage(error || t('resetFailedMsg'));
      bar.setAction(Button({ label: t('retry'), onClick: () => resetBluetooth() }));
    } else {
      bar.setSeverity('warning');
      bar.setTitle(t('stuckTitle'));
      bar.setMessage(t('stuckMsg'));
      bar.setAction(Button({ label: t('reset'), onClick: () => resetBluetooth() }));
    }
  };
  bar.setMode(null);
  return bar;
}

/* ------------------------------------------------- "Een apparaat toevoegen" */

const SEARCH_RENEW_MS = 20000;   // the host stops searching 60 s after the last discover {on:true}
const HINT_AFTER_MS = 12000;     // "Zie je je apparaat niet?" after this long without devices
const QUESTIONS = ['confirm', 'passkey', 'pin'];   // stages that hold an agent question (answered with pairReply)
const LOST = new Set(['offline', 'bad_response']); // bridge codes: the host's answer never arrived
let wizard = null;               // the open dialog (one at a time)

// fire and forget; keepalive when the page is going away
const send = (method, args, opts) => { try { bridge.call(method, args, opts).catch(() => {}); } catch { /* ignore */ } };

/**
 * Open the Win11 "Een apparaat toevoegen" dialog (or focus the open one). onClose() runs when it closed.
 * → Promise that resolves when the dialog closed.
 */
export function openAddDevice({ onClose } = {}) {
  if (wizard) { wizard.onClose.add(onClose); return wizard.closed; }
  wizard = addDeviceDialog();
  if (onClose) wizard.onClose.add(onClose);
  return wizard.closed;
}
/** Kept for older callers (Startpagina): now opens the real dialog. */
export const addDeviceHelp = () => openAddDevice();

function codeView(code, entered) {
  const el = h('div', { class: 'st-add__code', role: 'text', 'aria-label': String(code).split('').join(' ') });
  el.setEntered = (n) => {
    clear(el);
    [...String(code)].forEach((c, i) => el.append(h('span', { class: ['st-add__digit', n != null && i < n && 'is-typed'] }, c)));
  };
  el.setEntered(entered);
  return el;
}

function addDeviceDialog() {
  const self = { onClose: new Set() };
  let bt = null;                   // last system.bluetooth.get
  let step = 'choose';             // choose | list | done | failed
  const found = new Map();         // address → nearby device, in the order they appeared (no jumping rows)
  let sel = null;                  // the device being paired {dev, stage, qid, passkey, pin, entered, token, …}
  let lastSel = null;              // address picked last: focus goes back to its row
  let token = 0;
  let result = null;               // {dev, connected, error} for the done / failed step
  let note = null;                 // {severity, title, message} above the list (test mode)
  let searchErr = null;
  let notReady = false;            // a call answered "Bluetooth reageert niet"
  let resetMode = resetRunning() ? 'busy' : null;
  let resetError = '';
  let searchWanted = false;        // our discover {on:true} runs (or is under way): {on:false} at the end
  let listSince = 0;
  let closed = false;
  let released = false;            // the host was told to stop this dialog's search / pairing already
  const timers = [];
  const offs = [];

  const lead = h('p', { class: 'st-add__lead' });
  const body = h('div', { class: 'st-add__step' });
  const root = h('div', { class: 'st-add' }, lead, body);

  // Enter in the PIN box answers the question (the dialog would close on Enter otherwise)
  const onKey = (e) => {
    if (e.key !== 'Enter' || closed || !dlg) return;
    const layers = document.querySelectorAll('.fl-dialog-layer');
    if (layers[layers.length - 1] !== dlg.el.parentElement) return;
    const tgt = e.target instanceof Element ? e.target : null;
    if (tgt?.closest('button')) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (tgt?.matches('input') && sel?.submit) sel.submit();
    else if (step === 'done') dlg.close('close');
  };
  document.addEventListener('keydown', onKey, true);

  const shown = Dialog.show({ title: t('addTitle'), content: root, close: false, width: 548 });
  const dlg = shown.dialog;
  dlg.el.classList.add('st-add-dialog');
  const footer = dlg.el.querySelector('.fl-dialog__footer');
  footer.classList.add('st-add__footer');
  const close = () => dlg.close('close');

  // the footer "Annuleren" is one node for every step: a live update (devices, RSSI) never replaces the
  // focused button or swallows a click that spans a re-render
  const cancelBtn = Button({ label: t('cancel'), onClick: close });
  function setFooter(...btns) {
    const want = btns.filter(Boolean);
    const cur = [...footer.children];
    if (want.length === cur.length && want.every((b, i) => b === cur[i])) return;
    clear(footer).append(...want);
    footer.classList.toggle('is-single', want.length === 1);
  }

  /* ---- state ---- */
  const live = () => !!(bt && bt.available && bt.powered && !bt.stuck && !notReady);
  const btUsable = () => (bt ? bt.available !== false : true);

  function takeState(s) {
    if (!s || typeof s !== 'object' || s.available == null) return;
    const wasLive = live();
    bt = s;
    if (s.stuck === false) notReady = false;
    const near = new Map((s.nearby || []).map((d) => [String(d.address).toUpperCase(), d]));
    for (const [a, d] of near) found.set(a, { ...found.get(a), ...d });
    for (const a of [...found.keys()]) if (!near.has(a) && sel?.dev.address.toUpperCase() !== a) found.delete(a);
    if (!wasLive && live() && step === 'list' && !sel) search();
  }

  /* ---- searching ---- */
  async function search() {
    if (closed || !live()) return;
    searchWanted = true;
    const r = await attempt('system.bluetooth.discover', { on: true });
    if (closed || !searchWanted) {
      // the dialog closed (or the search was stopped) while this was under way: its {on:false} may have
      // reached the host first, so stop once more
      if (r.ok) send('system.bluetooth.discover', { on: false });
      return;
    }
    searchErr = r.ok ? null : (errText(r.error) || t('searchFailed'));
    if (!r.ok && isNotResponding(r.error)) { notReady = true; searchErr = null; }
    render();
  }
  function stopSearch(opts) {
    if (!searchWanted) return;
    searchWanted = false;
    send('system.bluetooth.discover', { on: false }, opts);
  }

  // Everything the host still does for this dialog: an open question, our pairing, the search. Runs when the
  // dialog closes, and when the page goes away without that (pagehide; 'win11:close' from an app window that
  // is being closed) with keepalive requests, so a closed window leaves no search, question or agent behind.
  function releaseHost(keepalive = false) {
    if (released) return;
    released = true;
    const opts = keepalive ? { keepalive: true } : undefined;
    if (sel) {
      const a = sel.dev.address;
      if (QUESTIONS.includes(sel.stage)) send('system.bluetooth.pairReply', { address: a, accept: false, qid: sel.qid ?? undefined }, opts);
      send('system.bluetooth.cancelPair', { address: a }, opts);
    }
    stopSearch(opts);
  }
  const onPageGone = () => { if (!closed) releaseHost(true); };
  addEventListener('pagehide', onPageGone);
  addEventListener('win11:close', onPageGone);

  /* ---- pairing ---- */
  async function startPair(dev) {
    if (sel || closed) return;
    const my = ++token;
    step = 'list';
    note = null;
    result = null;
    sel = { dev: { ...dev, address: String(dev.address).toUpperCase() }, stage: 'start', qid: null, token: my };
    lastSel = sel.dev.address;
    render();
    const r = await call('system.bluetooth.pair', { address: dev.address });
    if (!r.ok && LOST.has(r.error?.code) && !released) {
      // the answer got lost (the connection dropped, the page is going away: a navigation aborts the call
      // before pagehide), so the host may still be pairing: stop it, the UI will not wait for it any more
      const s = sel && sel.token === my ? sel : null;
      if (s && QUESTIONS.includes(s.stage)) send('system.bluetooth.pairReply', { address: s.dev.address, accept: false, qid: s.qid ?? undefined }, { keepalive: true });
      send('system.bluetooth.cancelPair', { address: dev.address }, { keepalive: true });
    }
    if (closed || !sel || sel.token !== my) return;
    finishPair(r.ok ? (r.value || {}) : { ok: false, error: errText(r.error), code: r.error?.code });
  }

  function finishPair(res) {
    const s = sel;
    sel = null;
    if (res.dryRun) { note = { severity: 'info', message: t('dryRun') }; render(); return; }
    if (res.ok !== false) {
      result = { dev: s.dev, connected: res.connected !== false, error: res.error || null };
      step = 'done';
      stopSearch();   // the device is ready: no inquiry next to its fresh connection (the host stops too)
      render();
      return;
    }
    if (s.userCancelled && (res.cancelled || res.code === 'cancelled')) { render(); return; }   // by code, not text
    if (isNotResponding(res)) notReady = true;
    result = { dev: s.dev, error: res.error || t('failedGeneric') };
    step = 'failed';
    render();
  }

  function onPairing(d) {
    if (!sel || !d || String(d.address || '').toUpperCase() !== sel.dev.address) return;
    const stage = d.stage;
    if (['done', 'failed', 'cancelled'].includes(stage)) return;   // the pair call answers with the outcome
    if (stage === 'display' && sel.stage === 'display' && (d.passkey || d.pin) === (sel.passkey || sel.pin)) {
      sel.entered = d.entered;
      sel.codeEl?.setEntered(d.pin ? null : d.entered);
      return;
    }
    Object.assign(sel, { stage, qid: d.qid ?? null, passkey: d.passkey || null, pin: d.pin || null, entered: d.entered, error: null, busy: false });
    render();
  }

  // answer the question on screen: its qid goes along, so the host never applies it to a newer question
  async function reply(accept, extra = {}) {
    const s = sel;
    if (!s || s.busy) return;
    const qid = s.qid;
    s.busy = true;
    render();
    const r = await call('system.bluetooth.pairReply', { address: s.dev.address, accept, qid: qid ?? undefined, ...extra });
    if (closed || sel !== s || s.qid !== qid) return;   // a newer question replaced this one: it is on screen
    s.busy = false;
    if (!r.ok || r.value?.ok === false) {
      if (r.value?.code === 'stale') { render(); return; }   // replaced at the host; its event is on the way
      s.error = errText(r.ok ? r.value : r.error) || t('failedGeneric');
      render();
      s.input?.focus();
      return;
    }
    if (accept) Object.assign(s, { stage: 'pairing', qid: null, passkey: null, pin: null, error: null });
    render();
  }

  async function cancelPairing() {
    const s = sel;
    if (!s || s.cancelling) return;
    s.cancelling = true;
    s.userCancelled = true;
    render();
    if (QUESTIONS.includes(s.stage)) await call('system.bluetooth.pairReply', { address: s.dev.address, accept: false, qid: s.qid ?? undefined });
    await call('system.bluetooth.cancelPair', { address: s.dev.address });
    // the pair call ends with "geannuleerd" → back to the list; never hang here if it does not
    timers.push(setTimeout(() => { if (sel === s && !closed) { sel = null; render(); } }, 10000));
  }

  /* ---- rendering ---- */
  function render() {
    if (closed) return;
    if (step === 'choose') renderChoose();
    else if (step === 'list') renderList();
    else renderResult();
  }

  function renderChoose() {
    lead.hidden = false;
    lead.textContent = t('chooseLead');
    const btOk = btUsable();
    const opt = (ic, title, desc, onClick, enabled) => h('button', {
      type: 'button', class: ['st-add__kind', !enabled && 'is-disabled'], disabled: !enabled, onClick,
    },
    h('span', { class: 'st-add__kind-icon' }, icon(ic, { size: 24 })),
    h('span', { class: 'st-add__kind-text' }, h('span', { class: 'st-add__kind-title' }, title), h('span', { class: 'st-add__kind-desc' }, desc)),
    enabled ? null : h('span', { class: 'st-add__kind-na' }, t('notAvailable')));
    clear(body).append(h('div', { class: 'st-add__kinds' },
      opt('bluetooth', t('kindBt'), btOk ? t('kindBtDesc') : t('na'), () => goList(), btOk),
      opt('cast', t('kindCast'), t('kindCastDesc'), null, false),
      opt('plus', t('kindOther'), t('kindOtherDesc'), null, false)));
    setFooter(cancelBtn);
    setTimeout(() => (body.querySelector('.st-add__kind:not([disabled])') || cancelBtn).focus({ preventScroll: true }), 0);
  }

  function goList() {
    step = 'list';
    listSince = Date.now();
    render();
    search();
    // renewed only while the list is shown: not on "klaar" (the search stopped) or "niet gelukt" (the host
    // keeps searching ≤ 60 s for a retry, which renews it)
    timers.push(setInterval(() => { if (!closed && step === 'list') search(); }, SEARCH_RENEW_MS));
    timers.push(setTimeout(() => { if (step === 'list') render(); }, HINT_AFTER_MS + 50));
    setTimeout(() => cancelBtn.focus({ preventScroll: true }), 0);
  }

  // list step: persistent nodes so a live update never steals focus or resets a half-typed PIN
  const noteSlot = h('div', { class: 'st-add__note' });
  const stuck = stuckBar({ compact: true });
  const list = h('div', { class: 'st-add__list', role: 'list' });
  const searchRow = h('div', { class: 'st-add__searching', role: 'status' },
    ProgressRing({ size: 16, ariaLabel: t('searching') }), h('span', null, t('searching')));
  const hint = h('p', { class: 'st-add__hint' }, t('notFoundYet'));
  const rows = new Map();          // address → {row, head, iconEl, name, sub, panel, panelKey, kind}
  let noteKey = '';

  function infoBar(key, opts) {
    if (key === noteKey) return;
    noteKey = key;
    clear(noteSlot);
    if (opts) { const b = InfoBar({ ...opts, closable: false }); b.classList.add('st-notice'); noteSlot.append(b); }
  }

  function renderList() {
    lead.hidden = false;
    lead.textContent = t('listLead');
    if (!body.contains(list)) clear(body).append(stuck, noteSlot, list, searchRow, hint);
    // notes
    const isStuck = !!(bt && (bt.stuck || notReady));
    stuck.setMode(resetMode === 'busy' ? 'busy' : resetMode === 'error' && isStuck ? 'error' : isStuck ? 'stuck' : null, resetError);
    if (isStuck || resetMode === 'busy') infoBar('', null);
    else if (bt && !bt.powered) {
      infoBar('off', { severity: 'warning', title: t('btOffTitle'), message: t('btOffMsg'), action: { label: t('turnOn'), onClick: turnOn } });
    } else if (searchErr) {
      infoBar(`err|${searchErr}`, { severity: 'error', title: '', message: searchErr, action: { label: t('retry'), onClick: () => search() } });
    } else if (note) infoBar(`note|${note.message}`, { severity: note.severity, title: note.title || '', message: note.message });
    else infoBar('', null);

    // rows: every nearby device (first-seen order) + the one being paired
    const devs = [...found.values()];
    if (sel && !found.has(sel.dev.address)) devs.unshift(sel.dev);
    const keep = new Set();
    devs.forEach((d, i) => {
      const a = String(d.address).toUpperCase();
      keep.add(a);
      let r = rows.get(a);
      if (!r) { r = makeRow(a); rows.set(a, r); }
      updateRow(r, d, !!sel && sel.dev.address === a);
      if (list.children[i] !== r.row) list.insertBefore(r.row, list.children[i] || null);
    });
    for (const [a, r] of rows) if (!keep.has(a)) { r.row.remove(); rows.delete(a); }
    list.hidden = !devs.length;
    // no spinner next to "Zoeken naar apparaten lukte niet": the host is not searching for us then
    const searchingNow = live() && !searchErr && (bt.discovering || searchWanted) && !sel;
    searchRow.hidden = !searchingNow;
    hint.hidden = !(live() && !devs.length && Date.now() - listSince >= HINT_AFTER_MS);
    setFooter(cancelBtn);
    keepFocus();
  }

  // The focused control may have gone (a rebuilt panel) or be disabled now (the picked row's head): keep the
  // focus in the dialog, on the question's control, the progress panel, the row picked last or Annuleren.
  function keepFocus() {
    setTimeout(() => {
      if (closed || step !== 'list') return;
      const ae = document.activeElement;
      if (ae && ae !== document.body && ae.isConnected && dlg.el.contains(ae) && !ae.disabled) return;
      let target = null;
      if (sel) {
        const r = rows.get(sel.dev.address);
        target = r && (r.panel.querySelector('input:not([disabled]), .fl-btn:not([disabled])') || r.panel);
      } else {
        const r = lastSel ? rows.get(lastSel) : null;
        target = (r && !r.head.disabled ? r.head : null) || list.querySelector('.st-add__dev-head:not([disabled])') || cancelBtn;
      }
      target?.focus({ preventScroll: true });
    }, 0);
  }

  function makeRow(address) {
    const iconEl = h('span', { class: 'st-add__dev-icon' });
    const name = h('span', { class: 'st-add__dev-name' });
    const sub = h('span', { class: 'st-add__dev-sub' });
    const head = h('button', { type: 'button', class: 'st-add__dev-head', onClick: () => { const d = found.get(address); if (d) startPair(d); } },
      iconEl, h('span', { class: 'st-add__dev-text' }, name, sub));
    const panel = h('div', { class: 'st-add__panel', tabindex: -1 });
    const row = h('div', { class: 'st-add__dev', role: 'listitem', dataset: { address } }, head, panel);
    return { row, head, iconEl, name, sub, panel, panelKey: null, kind: null };
  }

  function updateRow(r, d, isSel) {
    if (r.kind !== `${d.kind}|${d.icon}`) { r.kind = `${d.kind}|${d.icon}`; clear(r.iconEl).append(deviceIcon(d, 24)); }
    r.name.textContent = d.name || d.address;
    const sub = kindLabel(d.kind);
    r.sub.textContent = sub;
    r.sub.hidden = !sub;
    // a stuck / switched-off adapter cannot pair: the host still lists what the last search found (60 s)
    const usable = live();
    r.row.classList.toggle('is-selected', !!isSel);
    r.row.classList.toggle('is-dim', !isSel && (!!sel || !usable));
    r.head.disabled = !!sel || !usable;
    r.head.setAttribute('aria-expanded', String(!!isSel));
    if (!isSel) { if (r.panelKey) { clear(r.panel); r.panelKey = null; } return; }
    const s = sel;
    const key = [s.stage, s.qid, s.passkey, s.pin, s.cancelling, s.busy && s.stage !== 'passkey' && s.stage !== 'pin', s.token].join('|');
    if (key !== r.panelKey) { r.panelKey = key; clear(r.panel).append(...panelFor(s)); }
    if (s.input) s.input.setError(s.error || '');
    else r.panel.querySelector('.st-add__err')?.replaceChildren(s.error || '');
  }

  function progress(text) {
    return [h('div', { class: 'st-add__status' }, text), ProgressBar({ indeterminate: true, ariaLabel: text })];
  }

  function panelFor(s) {
    const name = s.dev.name || s.dev.address;
    s.input = null;
    s.submit = null;
    s.codeEl = null;
    if (s.cancelling) return progress(t('cancelling'));
    const actions = (...btns) => h('div', { class: 'st-add__actions' }, ...btns);
    const cancelBtnRow = Button({ label: t('cancel'), onClick: () => cancelPairing() });
    const errEl = h('div', { class: 'st-add__err', role: 'alert' }, s.error || '');
    switch (s.stage) {
      case 'confirm': {
        // the code shown goes along with the yes: the host checks it against the question it holds
        const ok = Button({ label: t('connectBtn'), variant: 'accent', disabled: s.busy, onClick: () => reply(true, s.passkey ? { passkey: s.passkey } : {}) });
        setTimeout(() => ok.focus({ preventScroll: true }), 0);
        return s.passkey
          ? [h('p', { class: 'st-add__text' }, t('confirmText')), codeView(s.passkey), errEl, actions(ok, cancelBtnRow)]
          : [h('p', { class: 'st-add__text' }, t('authorizeText', { name })), errEl, actions(ok, cancelBtnRow)];
      }
      case 'passkey':
      case 'pin': {
        const isPin = s.stage === 'pin';
        // a passkey is always the six digits the device shows (leading zeros too); "205 117" is fine
        const tb = TextBox({ placeholder: t('pinLabel'), ariaLabel: t('pinLabel'), maxLength: isPin ? 16 : 7, width: 296, clearable: false });
        tb.input.inputMode = isPin ? 'text' : 'numeric';
        tb.classList.add('st-add__pin');
        const ok = Button({ label: t('connectBtn'), variant: 'accent' });
        s.input = tb;
        s.submit = () => {
          const raw = tb.getValue().trim();
          const v = isPin ? raw : raw.replace(/\s+/g, '');
          const bad = isPin ? !/^[A-Za-z0-9]{1,16}$/.test(v) : !/^\d{6}$/.test(v);
          if (bad) { s.error = t(isPin ? 'badPin' : 'badPasskey'); tb.setError(s.error); tb.focus(); return; }
          s.error = null;
          reply(true, isPin ? { pin: v } : { passkey: v });
        };
        ok.addEventListener('click', () => s.submit());
        tb.input.addEventListener('input', () => { if (s.error) { s.error = null; tb.setError(''); } });
        setTimeout(() => tb.focus({ preventScroll: true }), 0);
        return [h('p', { class: 'st-add__text' }, t(isPin ? 'pinText' : 'passkeyText', { name })), tb, actions(ok, cancelBtnRow)];
      }
      case 'display': {
        s.codeEl = codeView(s.pin || s.passkey, s.pin ? null : s.entered);
        return [h('p', { class: 'st-add__text' }, t(s.pin ? 'displayPinText' : 'displayText', { name })), s.codeEl, actions(cancelBtnRow)];
      }
      default:
        return progress(s.stage === 'connecting' ? t('pairedConnecting') : t('pairing'));
    }
  }

  // done / failed: built once per outcome; a live update (failed + "reageert niet") only refreshes the
  // reset banner and the retry button, so focus and a click in progress survive
  let resultEl = null;
  let resultFor = null;
  let resultKey = '';
  let againBtn = null;
  function renderResult() {
    const ok = step === 'done';
    const r = result || {};
    lead.hidden = true;
    const isStuck = !ok && (notReady || bt?.stuck || isNotResponding(r.error));
    if (isStuck) stuck.setMode(resetMode === 'busy' ? 'busy' : resetMode === 'error' ? 'error' : 'stuck', resetError);
    const againOff = isStuck && resetMode !== 'ok' && !live();
    const key = JSON.stringify([step, r.connected, r.error, isStuck]);
    if (resultEl && resultFor === result && resultKey === key && body.contains(resultEl)) {
      againBtn?.setDisabled(againOff);
      return;
    }
    resultFor = result;
    resultKey = key;
    const d = r.dev || {};
    const status = ok ? (r.connected ? t('connected') : t('paired')) : '';
    const devCard = h('div', { class: ['st-add__devcard', ok && r.connected && 'is-connected'] },
      h('span', { class: 'st-add__dev-icon' }, deviceIcon(d, 24)),
      h('span', { class: 'st-add__dev-text' },
        h('span', { class: 'st-add__dev-name' }, d.name || d.address || ''),
        status ? h('span', { class: 'st-add__dev-sub' }, status) : (kindLabel(d.kind) ? h('span', { class: 'st-add__dev-sub' }, kindLabel(d.kind)) : null)));
    resultEl = h('div', { class: ['st-add__result', ok ? 'is-done' : 'is-failed'] },
      h('div', { class: 'st-add__result-head' },
        severityIcon(ok ? 'success' : 'error', 24),
        h('div', { class: 'st-add__result-title', role: 'status' }, ok ? t('doneTitle') : t('failedTitle'))),
      !ok ? h('p', { class: 'st-add__result-msg' }, r.error || t('failedGeneric')) : null,
      devCard,
      ok && r.error ? h('p', { class: 'st-add__result-note' }, r.error) : null,
      isStuck ? stuck : null);
    clear(body).append(resultEl);
    if (ok) {
      againBtn = null;
      const doneBtn = Button({ label: t('doneBtn'), variant: 'accent', onClick: close });
      setFooter(doneBtn);
      setTimeout(() => doneBtn.focus({ preventScroll: true }), 0);
    } else {
      againBtn = Button({ label: t('retry'), variant: 'accent', onClick: retry, disabled: againOff });
      setFooter(againBtn, cancelBtn);
      setTimeout(() => (againBtn.disabled ? cancelBtn : againBtn).focus({ preventScroll: true }), 0);
    }
  }

  function retry() {
    const d = result?.dev;
    result = null;
    step = 'list';
    if (live()) search();            // renew the search the host kept going after the failure
    if (d && found.has(d.address) && live()) startPair(found.get(d.address));
    else render();
  }

  async function turnOn() {
    const r = await attempt('system.bluetooth.setPowered', { powered: true });
    if (closed) return;
    if (!r.ok) {
      if (isNotResponding(r.error)) notReady = true;
      else showError(r.error, t('powerFailed'));
    } else if (r.value?.available != null) takeState(r.value);
    else { const s = await get('system.bluetooth.get'); if (s) takeState(s); }
    render();
    if (live()) search();
  }

  /* ---- live updates ---- */
  offs.push(bridge.on('system.bluetooth.changed', (d) => {
    const usableBefore = btUsable();
    takeState(d);
    if (step === 'list' || (step === 'failed' && notReady)) render();
    else if (step === 'choose' && btUsable() !== usableBefore) render();   // the adapter (dis)appeared
  }));
  offs.push(bridge.on('system.bluetooth.pairing', onPairing));
  const onReset = (s) => {
    if (closed) return;
    if (s.busy) resetMode = 'busy';
    else if (s.ok) {
      resetMode = 'ok';
      notReady = false;
      if (s.state) takeState(s.state);
      if (step === 'failed') step = 'list';
    } else { resetMode = 'error'; resetError = s.error || ''; }
    render();
    if (s.ok && step === 'list') search();
  };
  resetSubs.add(onReset);

  /* ---- start / end ---- */
  get('system.bluetooth.get').then((s) => { if (s) takeState(s); else bt = { available: false }; if (!closed) render(); });
  render();

  self.closed = shown.then(() => {
    closed = true;
    document.removeEventListener('keydown', onKey, true);
    removeEventListener('pagehide', onPageGone);
    removeEventListener('win11:close', onPageGone);
    offs.forEach((off) => off());
    resetSubs.delete(onReset);
    timers.forEach((tm) => { clearTimeout(tm); clearInterval(tm); });
    releaseHost();
    wizard = null;
    for (const cb of self.onClose) { try { cb?.(); } catch (e) { console.error('[bluetooth] onClose', e); } }
  });
  return self;
}

/* ---------------------------------------------------------------- tiles */

function batteryView(n) {
  return h('span', { class: 'st-dev__battery', 'aria-label': t('battery', { n: Math.round(n) }) },
    icon('battery', { size: 16 }), t('batteryShort', { n: Math.round(n) }));
}

function deviceTile(d, { busy, onAct, onRemove }) {
  const state = busy.get(d.address);
  const statusText = state === 'connect' ? t('connecting') : state === 'disconnect' ? t('disconnecting') : state === 'remove' ? t('removing')
    : d.connected ? t('connected') : d.paired ? t('paired') : t('notConnected');
  const status = h('span', { class: 'st-dev__status' }, h('span', null, statusText), !state && d.battery != null ? batteryView(d.battery) : null);
  const moreBtn = IconButton({ icon: 'more', label: t('more'), size: 32, cls: 'st-dev__more', disabled: !!state });
  const tile = h('div', {
    class: ['st-dev', d.connected && 'is-connected', state && 'is-busy'], tabindex: 0, role: 'button',
    'aria-label': `${d.name}: ${deviceStatus(d)}`, dataset: { address: d.address },
  },
  h('span', { class: 'st-dev__icon' }, deviceIcon(d, 28)),
  h('span', { class: 'st-dev__text' }, h('span', { class: 'st-dev__name' }, d.name || d.address), status),
  moreBtn);
  moreBtn.addEventListener('click', (e) => {
    e.stopPropagation();
    showContextMenu({ anchor: moreBtn, align: 'end', items: [
      { label: d.connected ? t('disconnect') : t('connect'), icon: d.connected ? 'close' : 'link', action: () => onAct(d) },
      { separator: true },
      { label: t('remove'), icon: 'trash', action: () => onRemove(d) },
    ] });
  });
  tile.addEventListener('dblclick', () => { if (!busy.has(d.address)) onAct(d); });
  tile.addEventListener('keydown', (e) => {
    if (e.target !== tile) return;
    if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (!busy.has(d.address)) onAct(d); }
  });
  return tile;
}

/* ------------------------------------------------------------------ page */

const wantsAdd = (v) => v != null && !['0', 'false', 'no', ''].includes(String(v).toLowerCase());

export const pages = {
  async bluetooth(ctx) {
    const [br, info] = await Promise.all([call('system.bluetooth.get'), get('system.info')]);
    const bt = br.ok ? br.value : null;
    // deep link ?add=1: once (a refresh of this page must not open the dialog again)
    if (ctx.params && wantsAdd(ctx.params.add)) {
      delete ctx.params.add;
      ctx.timeout(() => openAddDevice({ onClose: () => { if (ctx.alive()) ctx.refresh(); } }), 280);
    }
    if (!bt || !bt.available) {
      return [
        EmptyState({ icon: 'bluetooth', title: t('na'), message: t('naBody') }),
        SettingsGroup({ children: [SettingsCard({ icon: 'plus', title: t('add'), description: t('kindBtDesc'), onClick: () => openAddDevice() })] }),
      ];
    }
    let state = bt;
    let notReady = false;            // a call answered "Bluetooth reageert niet" (until the host's next state)
    let bannerMode = resetRunning() ? 'busy' : null;
    let bannerError = '';
    let okTimer = 0;
    const busy = new Map();          // address → 'connect' | 'disconnect' | 'remove'
    const grid = h('div', { class: 'st-dev-grid' });
    const name = info?.hostname || t('thisPc');
    const banner = stuckBar();
    const emptyNote = notice({ title: t('noDevices'), message: t('noDevicesBody') });

    const failed = (error, title = t('actionFailed')) => {
      if (isNotResponding(error)) { notReady = true; paint(); return; }
      showError(typeof error === 'string' ? { message: error } : error, title);
    };

    const toggle = ToggleSwitch({
      checked: !!state.powered, ariaLabel: t('bluetooth'),
      onChange: async (on) => {
        toggle.setDisabled(true);
        const r = await attempt('system.bluetooth.setPowered', { powered: on });
        if (!ctx.alive()) return;
        toggle.setDisabled(false);
        if (!r.ok) { toggle.setChecked(!on); failed(r.error, t('powerFailed')); return; }
        state = r.value && r.value.available != null ? r.value : { ...state, powered: on, powerState: on ? 'on' : 'off' };
        paint();
      },
    });
    const btCard = SettingsCard({ icon: 'bluetooth', title: t('bluetooth'), description: '', control: toggle });
    const resetBtn = Button({ label: t('resetBtn'), onClick: () => resetBluetooth() });
    const resetCard = SettingsCard({ icon: 'restart', title: t('reset'), description: t('resetCardDesc'), control: resetBtn });

    async function reload() {
      const r = await get('system.bluetooth.get');
      if (r && ctx.alive()) { state = r; if (r.stuck === false) notReady = false; paint(); }
    }

    async function act(d) {
      const what = d.connected ? 'disconnect' : 'connect';
      busy.set(d.address, what);
      paint();
      const r = await attempt(`system.bluetooth.${what}`, { address: d.address });
      busy.delete(d.address);
      if (!ctx.alive()) return;
      if (!r.ok) failed(r.error);
      await reload();
      paint();
    }

    async function remove(d) {
      if (busy.has(d.address)) return;
      // "Verwijderen…" only once the user said "Ja" (like Windows), not behind the question
      const r = await removeDevice(d, { onConfirmed: () => { busy.set(d.address, 'remove'); if (ctx.alive()) paint(); } });
      if (r === false) return;
      busy.delete(d.address);
      if (!ctx.alive()) return;
      if (r && r.error) failed(r.error);
      await reload();
      paint();
    }

    function paint() {
      const stuck = !!(state.stuck || notReady);
      if (bannerMode === 'ok' && stuck) bannerMode = null;
      banner.setMode(bannerMode === 'busy' ? 'busy' : bannerMode === 'error' ? 'error' : bannerMode === 'ok' ? 'ok' : stuck ? 'stuck' : null, bannerError);
      toggle.setChecked(!!state.powered);
      toggle.setDisabled(bannerMode === 'busy');
      resetBtn.setDisabled(bannerMode === 'busy');
      btCard.setDescription(stuck ? t('stuckTitle') : state.powered ? t('discoverable', { name }) : t('off'));
      clear(grid);
      const addTile = h('button', { type: 'button', class: 'st-dev st-dev--add', onClick: () => openAddDevice({ onClose: reload }) },
        h('span', { class: 'st-dev__icon' }, icon('plus', { size: 24 })),
        h('span', { class: 'st-dev__text' }, h('span', { class: 'st-dev__name' }, t('add'))));
      grid.append(addTile);
      grid.classList.toggle('is-off', !state.powered);
      const coll = collator();
      const devs = state.powered ? [...(state.devices || [])].sort((a, b) => (b.connected - a.connected) || coll.compare(String(a.name), String(b.name))) : [];
      for (const d of devs) grid.append(deviceTile(stuck ? { ...d, connected: false } : d, { busy, onAct: act, onRemove: remove }));
      emptyNote.hidden = !state.powered || devs.length > 0;
    }

    const onReset = (s) => {
      if (!ctx.alive()) return;
      clearTimeout(okTimer);
      if (s.busy) bannerMode = 'busy';
      else if (s.ok) {
        notReady = false;
        if (s.state) state = s.state;
        bannerMode = 'ok';
        okTimer = setTimeout(() => { if (bannerMode === 'ok') { bannerMode = null; paint(); } }, 6000);
        if (!s.state) reload();
      } else { bannerMode = 'error'; bannerError = s.error || ''; }
      paint();
    };
    resetSubs.add(onReset);
    ctx.onCleanup(() => { resetSubs.delete(onReset); clearTimeout(okTimer); });
    ctx.on('system.bluetooth.changed', (d) => {
      if (!d || d.available == null) return;
      state = d;
      if (d.stuck != null) notReady = false;   // the host's own verdict (it counts NotReady answers too)
      if (bannerMode === 'error' && !d.stuck) bannerMode = null;
      paint();
    });
    paint();

    return [
      banner,
      grid,
      emptyNote,
      SettingsGroup({ children: [btCard] }),
      SettingsGroup({ title: t('related'), children: [linkCard(ctx, 'sound', { description: t('soundDesc') }), resetCard] }),
    ];
  },
};
