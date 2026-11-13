// Win11 Shell — mock system.* methods (owned by the flyouts agent). SPEC §6.10.
//
// Realistic fake state for a desktop pc: Wi-Fi primary ("Thuisnetwerk") with 6 networks,
// Bluetooth with 3 devices, volume 45 with 3 outputs, brightness available (70),
// no battery (desktop). Every setter emits the matching system.*.changed event with the
// full get-payload, exactly like the host does after polling.
//
// Test switches (URL): ?mockBattery=1 (laptop battery present, 76 %, discharging)
//   ?mockBrightness=0 (no brightness control)   ?mockBluetooth=0 (no adapter)
//   ?mockWifi=0 (wired only: primary ethernet)    ?mockAudio=0 (no audio)
//   ?mockNightLight=0 (night light unsupported)   ?mockAirplane=1
// From the console: __mockSystem.setBattery({present:true, percent:20}) etc.
// Shapes mirror host/services/system.py: volume/brightness integer percent 0–100; network.get extras
// (available, wifiHardwareEnabled, primary.device/connection); network.connect and bluetooth.connect/
// disconnect answer {ok, error?, code?} instead of throwing (codes: wrong_password, not_found, timeout,
// denied, failed); system.info extras (prettyHostname, osId, architecture, memory.installed, sessionType).
//
// Bluetooth pairing (SPEC §6.10): system.bluetooth.discover {on} finds these devices over time (they stay
// in `nearby` while discovering and 60 s after, auto-stop after 60 s unless renewed):
//   WH-1000XM6 (headphones, just works → becomes the default sound output) · Galaxy S24 van Sanne (phone,
//   stage 'confirm' 482913; paired, but Connect fails: done {connected:false, error}) · Keyboard K380 (stage
//   'display' 715302, the digits are typed one by one: `entered`) · JBL Flip 6 (speaker, AuthenticationFailed
//   the first time, works on "Opnieuw proberen") · Forerunner 265 (watch, stage 'passkey': type 205117) ·
//   Opel Multimedia (car kit, stage 'pin': 1234).
// pair runs ≤ 60 s, emits system.bluetooth.pairing {address, stage, passkey?, pin?, entered?, connected?, error?,
// code?} and answers like the host: {ok, paired, connected, error?, code?} | {ok:false, error, code, cancelled};
// pairReply answers a confirm/passkey/pin request (30 s, then "niet op tijd"); cancelPair aborts. reset restarts
// a stuck adapter. Errors carry the host's Dutch text + code (not_ready, off, busy, not_found, auth_failed,
// cancelled, timeout, connect_failed, failed).
//   ?mockBtStuck=1 (adapter stuck in 'on-disabling', every call → "Bluetooth reageert niet")
//   ?mockBtDry=1 (pair/reset answer {ok, dryRun} at once, like the host in --dry-run)
//   ?mockBtResetFail=1 (reset fails)   console: __mockSystem.btStuck()

import { lang } from '../core/i18n.js';

const clone = (v) => JSON.parse(JSON.stringify(v));
const err = (code, message) => Object.assign(new Error(message), { code });
const clampInt = (v, lo, hi) => Math.round(Math.min(hi, Math.max(lo, Number(v) || 0)));

function initialState(params) {
  const off = (k) => params.get(k) === '0';
  const on = (k) => params.get(k) === '1';
  const now = Date.now();
  return {
    startedAt: now,
    audio: {
      available: !off('mockAudio'),
      volume: 45,
      muted: false,
      sinks: [
        { id: 52, name: 'Luidsprekers (Realtek USB Audio)', default: true },
        { id: 61, name: 'LG ULTRAGEAR (NVIDIA High Definition Audio)', default: false },
        { id: 74, name: 'Headset (Logitech G435)', default: false },
      ],
    },
    brightness: { available: !off('mockBrightness'), value: 70 },
    battery: on('mockBattery')
      ? { present: true, percent: 76, state: 'discharging', timeToEmpty: 3 * 3600 + 20 * 60, timeToFull: null, onAC: false, health: 92, cycles: 143, energyRate: 9.8 }
      : { present: false, percent: null, state: 'unknown', timeToEmpty: null, timeToFull: null, onAC: true, health: null, cycles: null, energyRate: null },
    network: {
      wifiAvailable: !off('mockWifi'),
      wifiEnabled: !off('mockWifi'),
      airplane: on('mockAirplane'),
      ethernet: off('mockWifi'), // a cable is plugged in (only used when Wi-Fi is absent)
    },
    wifi: [
      { ssid: 'Thuisnetwerk', strength: 84, secure: true, security: 'WPA2', active: true, saved: true, password: 'welkom123' },
      { ssid: 'Ziggo-5G-3F2A1', strength: 66, secure: true, active: false, saved: false, password: 'ziggo2026' },
      { ssid: 'VanDijk', strength: 52, secure: true, active: false, saved: false, password: 'vandijk!' },
      { ssid: 'iPhone van Sanne', strength: 41, secure: true, active: false, saved: true, password: 'sanne1234' },
      { ssid: 'Gastnetwerk', strength: 33, secure: false, active: false, saved: false },
      { ssid: 'TP-Link_7C21', strength: 17, secure: true, active: false, saved: false, password: 'tplink7c21' },
    ],
    bluetooth: {
      available: !off('mockBluetooth'),
      powered: true,
      powerState: on('mockBtStuck') ? 'on-disabling' : 'on',
      stuck: on('mockBtStuck'),
      rfBlocked: false,
      dry: on('mockBtDry'),
      resetFail: on('mockBtResetFail'),
      discovering: false, discoverStarted: 0, discoverUntil: 0, seenUntil: 0,
      found: [],          // addresses of BT_NEARBY found by the current discovery (in order)
      rssi: {},           // address → current RSSI (drifts while searching, like BlueZ reports it)
      pairing: null,      // the running pair job
      devices: [
        { address: 'F8:4E:17:2A:9C:01', name: 'WH-1000XM4', icon: 'audio-headphones', kind: 'headphones', paired: true, connected: !on('mockBtStuck'), battery: 80 },
        { address: 'D4:A3:3D:61:0B:7E', name: 'MX Master 3S', icon: 'input-mouse', kind: 'mouse', paired: true, connected: false, battery: 55 },
        { address: '98:7A:14:C2:55:30', name: 'Xbox Wireless Controller', icon: 'input-gaming', kind: 'gamepad', paired: true, connected: false },
      ],
    },
    nightLight: { available: !off('mockNightLight'), enabled: false, temperature: 4500 },
    powerSettings: {
      screen: { available: true, writable: true, idleDelay: 300 },
      sleepAC: { available: true, writable: true, action: 'nothing', timeout: 3600 },
      sleepBattery: { available: true, writable: true, action: 'suspend', timeout: 900 },
      dim: { available: true, writable: true, enabled: true },
      profiles: { available: !off('mockPowerProfiles'), writable: !off('mockPowerProfiles'), active: 'balanced',
        choices: off('mockPowerProfiles') ? [] : ['power-saver', 'balanced', 'performance'].map((id) => ({ id })), degraded: '', inhibited: '' },
      canSuspend: true,
    },
    updates: { available: 3, checkedAt: now - 2 * 3600 * 1000, tool: 'apt' },
  };
}

function S(m) {
  if (!m.state.system) m.state.system = initialState(m.params);
  return m.state.system;
}

/* ------------------------------------------------------------ payloads */

const audioPayload = (s) => clone(s.audio);
const brightnessPayload = (s) => ({ available: s.brightness.available, value: s.brightness.available ? s.brightness.value : null });
const batteryPayload = (s) => clone(s.battery);

function networkPayload(s) {
  const n = s.network;
  const active = n.wifiAvailable && n.wifiEnabled && !n.airplane ? s.wifi.find((w) => w.active) : null;
  let primary = { type: 'none', name: '', strength: null };
  if (active) primary = { type: 'wifi', name: active.ssid, strength: active.strength, device: 'wlp9s0', connection: active.ssid };
  else if (n.ethernet && !n.airplane) primary = { type: 'ethernet', name: 'Ethernet', strength: null, device: 'enp10s0', connection: 'Bekabelde verbinding 1' };
  return {
    connectivity: primary.type === 'none' ? 'none' : 'full',
    wifiAvailable: n.wifiAvailable,
    wifiEnabled: n.wifiAvailable && n.wifiEnabled,
    airplane: n.airplane,
    primary,
    available: true,
    wifiHardwareEnabled: n.wifiAvailable,
  };
}

function wifiListPayload(s) {
  const n = s.network;
  if (!n.wifiAvailable || !n.wifiEnabled || n.airplane) return [];
  return s.wifi
    .map(({ ssid, strength, secure, active, saved }) => ({ ssid, strength, secure, security: secure ? 'WPA2' : '', active, saved }))
    .sort((a, b) => (b.active - a.active) || (b.strength - a.strength));
}

/* Bluetooth devices in pairing mode near the pc. `after`: ms after the discovery started until BlueZ sees it;
   `pair`: what the device asks while pairing (see the header). */
const BT_NEARBY = [
  { address: 'AC:80:0A:5E:21:F3', name: 'WH-1000XM6', icon: 'audio-headphones', kind: 'headphones', rssi: -47, after: 1200, pair: 'just', battery: 90, sink: true },
  { address: '5C:CB:99:1D:7A:42', name: 'Galaxy S24 van Sanne', icon: 'phone', kind: 'phone', rssi: -61, after: 2600, pair: 'confirm', code: '482913', noConnect: true },
  { address: 'E4:5F:01:9B:33:C8', name: 'Keyboard K380', icon: 'input-keyboard', kind: 'keyboard', rssi: -55, after: 3800, pair: 'display', code: '715302', battery: 70 },
  { address: '00:42:79:F1:0E:6D', name: 'JBL Flip 6', icon: 'audio-card', kind: 'speaker', rssi: -72, after: 5200, pair: 'fail-once', sink: true },
  { address: 'C4:9A:02:7B:5D:10', name: 'Forerunner 265', icon: 'watch', kind: 'watch', rssi: -78, after: 7000, pair: 'passkey', code: '205117', battery: 64 },
  { address: '88:C6:26:A0:4F:97', name: 'Opel Multimedia', icon: 'audio-card', kind: 'other', rssi: -84, after: 9000, pair: 'pin', code: '1234' },
];
const BT_REPLY_MS = 30000;   // host BT_AGENT_REPLY_S: an unanswered question is cancelled after this
// the host's texts (host/services/system.py bt_*), in the UI language like the host's _tr (settings.language)
const BT_TEXT = {
  nl: {
    notResponding: 'Bluetooth reageert niet. Kies “Bluetooth opnieuw starten”.',
    off: 'Zet Bluetooth eerst aan.',
    noAdapter: 'Er is geen Bluetooth-adapter gevonden.',
    resetFailed: 'Bluetooth opnieuw starten lukte niet. Start de computer opnieuw op.',
    connectFailed: 'Verbinden lukte niet. Staat het apparaat aan en in de buurt?',
    auth: 'De koppelcode klopte niet. Probeer het opnieuw.',
    cancelled: 'Koppelen is geannuleerd.',
    timeout: 'Het apparaat reageerde niet. Staat het nog in de koppelstand?',
    replyTimeout: 'Er is niet op tijd bevestigd. Probeer het opnieuw.',
    gone: 'Het apparaat is niet meer zichtbaar. Zet het opnieuw in de koppelstand.',
    busy: 'Er wordt al een apparaat gekoppeld. Even geduld.',
    btBusy: 'Bluetooth is even bezig. Probeer het zo opnieuw.',
    notConnected: 'Gekoppeld, maar verbinden lukte niet. Kies later “Verbinden”.',
    noRequest: 'Er is geen koppelverzoek (meer) voor dit apparaat.',
    badPasskey: 'Typ de zes cijfers die het apparaat laat zien.',
    badPin: 'Typ een pincode van 1 tot 16 letters of cijfers.',
    notFound: 'Dit apparaat is niet gevonden.',
    badDevice: 'Ongeldig apparaat.',
  },
  en: {
    notResponding: 'Bluetooth is not responding. Choose “Restart Bluetooth”.',
    off: 'Turn on Bluetooth first.',
    noAdapter: 'No Bluetooth adapter was found.',
    resetFailed: 'Restarting Bluetooth failed. Restart the computer.',
    connectFailed: 'Connecting failed. Is the device on and nearby?',
    auth: 'The pairing code was wrong. Try again.',
    cancelled: 'Pairing was cancelled.',
    timeout: 'The device did not respond. Is it still in pairing mode?',
    replyTimeout: 'The pairing was not confirmed in time. Try again.',
    gone: 'The device is no longer visible. Put it in pairing mode again.',
    busy: 'Another device is being paired. Please wait.',
    btBusy: 'Bluetooth is busy. Try again in a moment.',
    notConnected: 'Paired, but connecting failed. Choose “Connect” later.',
    noRequest: 'There is no (longer a) pairing request for this device.',
    badPasskey: 'Type the six digits the device shows.',
    badPin: 'Type a PIN of 1 to 16 letters or digits.',
    notFound: 'This device was not found.',
    badDevice: 'Invalid device.',
  },
};
const BT_ERRORS = new Proxy({}, { get: (_o, k) => (BT_TEXT[lang()] || BT_TEXT.nl)[k] ?? BT_TEXT.nl[k] });
const given = (v) => v != null && String(v).trim() !== '';
const btPaired = (b, address) => b.devices.some((d) => d.address === address);

function bluetoothPayload(s) {
  const b = s.bluetooth;
  if (!b.available) return { available: false, powered: false, powerState: 'off', stuck: false, discovering: false, name: '', blocked: false, devices: [], nearby: [] };
  const live = b.powered && !b.stuck;
  // like the host: what the last search found stays listed for 60 s, also while the adapter hangs
  const showNearby = b.powered && (b.discovering || Date.now() < b.seenUntil);
  const nearby = showNearby
    ? b.found.filter((a) => !btPaired(b, a)).map((a) => BT_NEARBY.find((x) => x.address === a)).filter(Boolean)
      .map(({ address, name, icon, kind, rssi }) => ({ address, name, icon, kind, rssi: b.rssi?.[address] ?? rssi }))
      .sort((x, y) => (y.rssi ?? -999) - (x.rssi ?? -999) || x.name.localeCompare(y.name))
    : [];
  return {
    available: true,
    powered: b.powered,
    powerState: b.powerState,
    stuck: b.stuck,
    devices: clone(b.devices).map(({ sinkId, ...d }) => (live ? d : { ...d, connected: false })),
    nearby,
    name: 'robin-pc', discovering: live && b.discovering, blocked: b.rfBlocked || (!b.powered && s.network.airplane),
  };
}

const emitAudio = (m) => m.emit('system.audio.changed', audioPayload(S(m)));
const emitBrightness = (m) => m.emit('system.brightness.changed', brightnessPayload(S(m)));
const emitBattery = (m) => m.emit('system.battery.changed', batteryPayload(S(m)));
const emitNetwork = (m) => m.emit('system.network.changed', networkPayload(S(m)));
const emitBluetooth = (m) => m.emit('system.bluetooth.changed', bluetoothPayload(S(m)));

const dry = (extra = {}) => ({ ok: true, dryRun: true, ...extra });

/* ------------------------------------------------------ bluetooth pairing */

function btStopDiscovery(b) {
  if (!b.discovering) return;
  b.discovering = false;
  b.seenUntil = Date.now() + 60000;
}

/** Headphones/speakers paired in this session are sound outputs while connected (makeDefault: like the host). */
function btSyncSinks(m, makeDefault = null) {
  const s = S(m);
  const b = s.bluetooth;
  b.btSinks ||= [];
  const before = JSON.stringify(s.audio.sinks);
  const live = b.powered && !b.stuck;
  const want = new Map(b.devices.filter((d) => d.sinkId && d.connected && live).map((d) => [d.sinkId, d]));
  s.audio.sinks = s.audio.sinks.filter((x) => !b.btSinks.includes(x.id) || want.has(x.id));
  for (const [id, d] of want) {
    if (!s.audio.sinks.some((x) => x.id === id)) s.audio.sinks.push({ id, name: d.name, default: false });
  }
  b.btSinks = [...new Set([...b.btSinks, ...want.keys()])];
  const target = makeDefault && b.devices.find((d) => d.address === makeDefault)?.sinkId;
  if (target && s.audio.sinks.some((x) => x.id === target)) s.audio.sinks.forEach((x) => { x.default = x.id === target; });
  if (s.audio.sinks.length && !s.audio.sinks.some((x) => x.default)) s.audio.sinks[0].default = true;
  if (JSON.stringify(s.audio.sinks) !== before) emitAudio(m);
}

/** The host's pair: agent requests become pairing events, answered by pairReply; ends with {ok, error?, code?}. */
async function btPair(m, b, dev) {
  const { address } = dev;
  const ev = (stage, extra = {}) => m.emit('system.bluetooth.pairing', { address, stage, ...extra });
  const fail = (key, code) => Object.assign(new Error(BT_ERRORS[key]), { code });
  const job = { address, cancelled: false, answer: null, wakers: new Set(), qid: 0, stage: 'pairing' };
  job.cancel = () => { job.cancelled = true; [...job.wakers].forEach((w) => w()); };
  const wait = (ms) => new Promise((resolve, reject) => {
    if (job.cancelled) { reject(fail('cancelled', 'cancelled')); return; }
    const waker = () => { clearTimeout(tm); job.wakers.delete(waker); reject(fail('cancelled', 'cancelled')); };
    const tm = setTimeout(() => { job.wakers.delete(waker); resolve(); }, ms);
    job.wakers.add(waker);
  });
  // an Agent1 request (RequestConfirmation / RequestPasskey / RequestPinCode), held until pairReply; like the
  // host's agent it is cancelled after 30 s; an answer → stage 'pairing' again
  const ask = (stage, extra = {}) => new Promise((resolve, reject) => {
    const done = () => { clearTimeout(tm); job.answer = null; job.kind = null; job.wakers.delete(waker); };
    const waker = () => { done(); reject(fail('cancelled', 'cancelled')); };
    const tm = setTimeout(() => { done(); reject(fail('replyTimeout', 'timeout')); }, BT_REPLY_MS);
    job.kind = stage;
    job.qid += 1;                        // like the host: every question has its own id (pairReply {qid})
    job.shown = extra.passkey || null;
    job.answer = (r) => { done(); if (r.accept) { ev('pairing'); resolve(r); } else reject(fail('cancelled', 'rejected')); };
    job.wakers.add(waker);
    ev(stage, { ...extra, qid: job.qid });
  });
  b.pairing = job;
  let finish;
  job.finished = new Promise((r) => { finish = r; });
  b.failedOnce ||= [];
  try {
    ev('pairing');
    await wait(1400);
    if (dev.pair === 'confirm') {
      await ask('confirm', { passkey: dev.code });
      await wait(900);
    } else if (dev.pair === 'passkey' || dev.pair === 'pin') {
      const r = await ask(dev.pair);
      const given = dev.pair === 'pin' ? String(r.pin ?? '').trim() : String(r.passkey ?? '').trim().padStart(6, '0');
      await wait(900);
      if (given !== dev.code) throw fail('auth', 'auth_failed');
    } else if (dev.pair === 'display') {
      // DisplayPasskey: the user types it on the keyboard, BlueZ reports every key press (`entered`)
      ev('display', { passkey: dev.code, entered: 0 });
      await wait(2600);
      for (let i = 1; i <= dev.code.length; i++) { ev('display', { passkey: dev.code, entered: i }); await wait(450); }
      await wait(700);
    } else if (dev.pair === 'fail-once' && !b.failedOnce.includes(address)) {
      b.failedOnce.push(address);
      await wait(2400);
      throw fail('auth', 'auth_failed');
    } else {
      await wait(800);
    }
    // paired → Trusted=true → Connect (a phone has no profile to connect to: it stays paired); a cancel while
    // connecting leaves nothing behind (the host removes the new bond again)
    job.stage = 'connecting';
    ev('connecting');
    await wait(dev.noConnect ? 2600 : 1400);
    const connected = !dev.noConnect;
    b.devices.push({
      address, name: dev.name, icon: dev.icon, kind: dev.kind, paired: true, connected,
      ...(dev.battery != null ? { battery: dev.battery } : {}),
      ...(dev.sink ? { sinkId: 90 + BT_NEARBY.indexOf(dev) } : {}),
    });
    btSyncSinks(m, dev.sink ? address : null);
    emitBluetooth(m);
    const extra = connected ? {} : { error: BT_ERRORS.notConnected, code: 'connect_failed' };
    ev('done', { connected, ...extra });
    return { ok: true, paired: true, connected, ...extra };
  } catch (e) {
    if (e.code === 'cancelled' || e.code === 'rejected') {
      ev('cancelled', { error: BT_ERRORS.cancelled, code: 'cancelled' });
      return { ok: false, error: BT_ERRORS.cancelled, code: 'cancelled', cancelled: true };
    }
    ev('failed', { error: e.message, code: e.code || 'failed' });
    return { ok: false, error: e.message, code: e.code || 'failed', cancelled: false };
  } finally {
    if (b.pairing === job) b.pairing = null;
    finish();
  }
}

/** The host's reset (rfkill cycle when stuck, else Powered off → on); see the method. */
async function btReset(m, s, b) {
  const job = b.pairing;
  if (job) { job.cancel(); await job.finished; }   // like the host: our pairing stops first
  const wasStuck = b.stuck;
  const reconnect = b.devices.filter((d) => d.connected || (wasStuck && d.kind === 'headphones')).map((d) => d.address);
  btStopDiscovery(b);
  b.devices.forEach((d) => { d.connected = false; });
  btSyncSinks(m);
  if (wasStuck) {
    // like the host: rfkill block + unblock (the adapter is gone for a moment, then powers up again)
    await m.sleep(900);
    Object.assign(b, { rfBlocked: true, powered: false, powerState: 'off-blocked', stuck: false });
    emitBluetooth(m);
    await m.sleep(1100);
    if (b.resetFail) {
      Object.assign(b, { rfBlocked: false, powerState: 'on-disabling', powered: true, stuck: true });
      emitBluetooth(m);
      throw err('failed', BT_ERRORS.resetFailed);
    }
    Object.assign(b, { rfBlocked: false, powerState: 'off-enabling' });
    emitBluetooth(m);
    await m.sleep(1300);
  } else {
    Object.assign(b, { powered: false, powerState: 'off' });
    emitBluetooth(m);
    await m.sleep(900);
    Object.assign(b, { powerState: 'off-enabling' });
    await m.sleep(700);
  }
  Object.assign(b, { powered: true, powerState: 'on', stuck: false });
  emitBluetooth(m);
  // devices that were connected come back by themselves a moment later
  setTimeout(() => {
    if (!b.powered || b.stuck) return;
    b.devices.forEach((d) => { if (reconnect.includes(d.address)) d.connected = true; });
    btSyncSinks(m);
    emitBluetooth(m);
  }, 1800);
  return bluetoothPayload(s);
}

/* -------------------------------------------------------------- methods */

export default {
  methods: {
    /* ---- audio ---- */
    'system.audio.get': async (a, m) => audioPayload(S(m)),
    'system.audio.set': async ({ volume, muted } = {}, m) => {
      const s = S(m);
      if (!s.audio.available) throw err('unsupported', 'Er is geen geluidsapparaat gevonden.');
      if (volume != null) {
        // ctl may send 0.0–1.0 floats; the UI sends integer percent
        const v = Number(volume);
        s.audio.volume = clampInt(v > 0 && v <= 1 && !Number.isInteger(v) ? v * 100 : v, 0, 100);
        if (s.audio.volume > 0 && muted == null) s.audio.muted = false;
      }
      if (muted != null) s.audio.muted = !!muted;
      emitAudio(m);
      return dry(audioPayload(s));
    },
    'system.audio.setSink': async ({ id } = {}, m) => {
      const s = S(m);
      // wpctl node ids are numbers (host: int(id)); accept "52" too
      const sink = s.audio.sinks.find((x) => String(x.id) === String(id));
      if (!sink) throw err('not_found', 'Dit geluidsapparaat is niet meer aangesloten.');
      s.audio.sinks.forEach((x) => { x.default = x === sink; });
      emitAudio(m);
      return dry();
    },

    /* ---- brightness ---- */
    'system.brightness.get': async (a, m) => brightnessPayload(S(m)),
    'system.brightness.set': async ({ value } = {}, m) => {
      const s = S(m);
      if (!s.brightness.available) throw err('unsupported', 'De helderheid van dit scherm kan niet worden aangepast.');
      s.brightness.value = clampInt(value, 1, 100);
      emitBrightness(m);
      return dry(brightnessPayload(s));
    },

    /* ---- battery ---- */
    'system.battery.get': async (a, m) => batteryPayload(S(m)),

    /* ---- network ---- */
    'system.network.get': async (a, m) => networkPayload(S(m)),
    'system.network.wifiList': async ({ rescan } = {}, m) => {
      const s = S(m);
      if (rescan) {
        await m.sleep(900);
        // a rescan shuffles the signal strengths a little
        s.wifi.forEach((w) => { w.strength = clampInt(w.strength + (Math.random() * 8 - 4), 8, 99); });
      }
      return wifiListPayload(s);
    },
    'system.network.setWifi': async ({ enabled } = {}, m) => {
      const s = S(m);
      if (!s.network.wifiAvailable) throw err('unsupported', 'Deze pc heeft geen wifi.');
      s.network.wifiEnabled = !!enabled;
      if (enabled && !s.wifi.some((w) => w.active)) {
        const saved = s.wifi.find((w) => w.saved);
        if (saved) saved.active = true;
      }
      emitNetwork(m);
      return dry();
    },
    'system.network.setAirplane': async ({ enabled } = {}, m) => {
      const s = S(m);
      s.network.airplane = !!enabled;
      const b = s.bluetooth;
      if (enabled) {
        Object.assign(b, { powered: false, powerState: 'off-blocked' });
        b.devices.forEach((d) => { d.connected = false; });
        btStopDiscovery(b);
        btSyncSinks(m);
      } else if (!b.stuck) {
        Object.assign(b, { powered: true, powerState: 'on' });
      }
      emitNetwork(m);
      emitBluetooth(m);
      return dry();
    },
    'system.network.connect': async ({ ssid, password } = {}, m) => {
      const s = S(m);
      const w = s.wifi.find((x) => x.ssid === ssid);
      await m.sleep(1400);
      if (!w) return { ok: false, error: 'Dit netwerk is niet (meer) in de buurt.', code: 'not_found' };
      // like nmcli: a secured network without (correct) password → "secrets were required"
      if (w.secure && !w.saved && !password) return { ok: false, error: 'Het wachtwoord klopt niet. Probeer het opnieuw.', code: 'wrong_password' };
      if (w.secure && password != null && password !== '' && password !== w.password) {
        return { ok: false, error: 'Het wachtwoord klopt niet. Probeer het opnieuw.', code: 'wrong_password' };
      }
      s.wifi.forEach((x) => { x.active = false; });
      w.active = true;
      w.saved = true;
      s.network.wifiEnabled = true;
      emitNetwork(m);
      return { ok: true, dryRun: true };
    },
    'system.network.disconnect': async (a, m) => {
      const s = S(m);
      s.wifi.forEach((x) => { x.active = false; });
      emitNetwork(m);
      return dry();
    },

    /* ---- bluetooth ---- */
    'system.bluetooth.get': async (a, m) => bluetoothPayload(S(m)),
    'system.bluetooth.setPowered': async ({ powered } = {}, m) => {
      const s = S(m);
      const b = s.bluetooth;
      if (!b.available) throw err('unavailable', BT_ERRORS.noAdapter);
      await m.sleep(300);
      if (b.stuck) throw err('not_ready', BT_ERRORS.notResponding);
      b.powered = !!powered;
      b.powerState = powered ? 'on' : 'off';
      if (powered) b.rfBlocked = false;
      if (!powered) {
        b.devices.forEach((d) => { d.connected = false; });
        btStopDiscovery(b);
        btSyncSinks(m);
      }
      emitBluetooth(m);
      return dry();
    },
    'system.bluetooth.connect': async ({ address } = {}, m) => {
      const s = S(m);
      const d = s.bluetooth.devices.find((x) => x.address === address);
      await m.sleep(1200);
      if (!d) throw err('not_found', BT_ERRORS.notFound);
      if (s.bluetooth.stuck) return { ok: false, error: BT_ERRORS.notResponding, code: 'not_ready' };
      if (!s.bluetooth.powered) return { ok: false, error: BT_ERRORS.off, code: 'off' };
      if (/xbox/i.test(d.name)) return { ok: false, error: BT_ERRORS.connectFailed, code: 'connect_failed' };
      d.connected = true;
      btSyncSinks(m);
      emitBluetooth(m);
      return dry();
    },
    'system.bluetooth.disconnect': async ({ address } = {}, m) => {
      const s = S(m);
      const d = s.bluetooth.devices.find((x) => x.address === address);
      await m.sleep(500);
      if (!d) throw err('not_found', BT_ERRORS.notFound);
      if (s.bluetooth.stuck) return { ok: false, error: BT_ERRORS.notResponding, code: 'not_ready' };
      d.connected = false;
      btSyncSinks(m);
      emitBluetooth(m);
      return dry();
    },
    'system.bluetooth.remove': async ({ address } = {}, m) => {
      const s = S(m);
      const i = s.bluetooth.devices.findIndex((x) => x.address === address);
      await m.sleep(400);
      if (i < 0) return { ok: true };   // like the host: a device that is already gone counts as removed
      if (s.bluetooth.stuck) return { ok: false, error: BT_ERRORS.notResponding, code: 'not_ready' };
      s.bluetooth.devices.splice(i, 1);
      btSyncSinks(m);
      emitBluetooth(m);
      return dry();
    },
    'system.bluetooth.discover': async ({ on: start } = {}, m) => {
      const b = S(m).bluetooth;
      if (!b.available) throw err('unavailable', BT_ERRORS.noAdapter);
      if (start) {
        if (b.stuck) return { ok: false, error: BT_ERRORS.notResponding, code: 'not_ready' };
        if (!b.powered) return { ok: false, error: BT_ERRORS.off, code: 'off' };
        if (!b.discovering) {
          b.discovering = true;
          b.discoverStarted = Date.now();
          if (Date.now() > b.seenUntil) b.found = [];   // BlueZ forgot the devices of an older scan
        }
        b.discoverUntil = Date.now() + 60000;
      } else {
        btStopDiscovery(b);
      }
      emitBluetooth(m);
      return { ok: true };
    },
    'system.bluetooth.pair': async ({ address } = {}, m) => {
      const b = S(m).bluetooth;
      if (typeof address !== 'string' || !/^[0-9A-F]{2}(:[0-9A-F]{2}){5}$/i.test(address)) throw err('bad_args', BT_ERRORS.badDevice);
      if (!b.available) throw err('unavailable', BT_ERRORS.noAdapter);
      address = address.toUpperCase();
      if (b.resetJob) return { ok: false, error: BT_ERRORS.btBusy, code: 'busy', cancelled: false };
      // like the host: the address being paired again (a new dialog after the old window closed) takes over;
      // one that is connecting already is waited for
      if (b.pairing && b.pairing.address === address) {
        const old = b.pairing;
        if (old.stage !== 'connecting') old.cancel();
        await old.finished;
      }
      if (b.pairing) return { ok: false, error: BT_ERRORS.busy, code: 'busy', cancelled: false };   // no event: not ours
      if (b.dry) return dry();
      // like the host: 'pairing' first, then 'failed' with the reason
      const early = (error, code) => {
        m.emit('system.bluetooth.pairing', { address, stage: 'pairing' });
        m.emit('system.bluetooth.pairing', { address, stage: 'failed', error, code });
        return { ok: false, error, code, cancelled: false };
      };
      if (btPaired(b, address)) return { ok: true, paired: true, connected: b.devices.find((d) => d.address === address).connected };
      const dev = BT_NEARBY.find((x) => x.address === address && b.found.includes(address));
      if (!dev) return early(BT_ERRORS.gone, 'not_found');
      if (!b.powered) return early(BT_ERRORS.off, 'off');
      if (b.stuck) return early(BT_ERRORS.notResponding, 'not_ready');
      return btPair(m, b, dev);
    },
    'system.bluetooth.pairReply': async ({ address, accept, passkey, pin, qid } = {}, m) => {
      if (typeof address !== 'string' || !/^[0-9A-F]{2}(:[0-9A-F]{2}){5}$/i.test(address)) throw err('bad_args', BT_ERRORS.badDevice);
      const job = S(m).bluetooth.pairing;
      if (!job || job.address !== address.toUpperCase() || !job.answer) return { ok: false, error: BT_ERRORS.noRequest };
      // like the host's agent: a yes must fit the question that is open now (its qid, its kind, the code shown)
      const stale = { ok: false, error: BT_ERRORS.noRequest, code: 'stale' };
      if (accept) {
        if (qid != null && String(qid) !== String(job.qid)) return stale;
        if (job.kind === 'passkey' && !given(passkey) && given(pin)) return stale;
        if (job.kind === 'pin' && !given(pin) && given(passkey)) return stale;
        if (job.kind === 'confirm' && (given(pin) || (given(passkey) && String(passkey).trim().padStart(6, '0') !== job.shown))) return stale;
      }
      if (accept && job.kind === 'passkey' && !/^\d{1,6}$/.test(String(passkey ?? '').trim())) throw err('bad_args', BT_ERRORS.badPasskey);
      if (accept && job.kind === 'pin' && !/^[A-Za-z0-9]{1,16}$/.test(String(pin ?? '').trim())) throw err('bad_args', BT_ERRORS.badPin);
      job.answer({ accept: !!accept, passkey, pin });
      return { ok: true };
    },
    'system.bluetooth.cancelPair': async ({ address } = {}, m) => {
      const job = S(m).bluetooth.pairing;
      if (job && job.address === String(address || '').toUpperCase()) job.cancel();
      return { ok: true };
    },
    'system.bluetooth.reset': async (a, m) => {
      const s = S(m);
      const b = s.bluetooth;
      if (!b.available) throw err('unavailable', BT_ERRORS.noAdapter);
      if (b.dry) return dry();
      // like the host: one reset at a time; a second caller (quick settings + Instellingen) gets the same outcome
      if (b.resetJob) { await b.resetJob; return bluetoothPayload(s); }
      b.resetJob = btReset(m, s, b);
      try { return await b.resetJob; } finally { b.resetJob = null; }
    },

    /* ---- power & night light ---- */
    'system.power.settings.get': async (a, m) => clone(S(m).powerSettings),
    'system.power.settings.set': async (args = {}, m) => {
      const state = S(m).powerSettings;
      const keys = Object.keys(args);
      const key = keys[0];
      const integer = (value, max) => Number.isInteger(value) && value >= 0 && value <= max;
      if (keys.length !== 1) throw err('bad_args', 'Geef één energie-instelling op.');
      const value = args[key];
      const target = key === 'idleDelay' ? state.screen : key === 'idleDim' ? state.dim
        : key === 'powerProfile' ? state.profiles : state[key];
      if (!target || !['idleDelay', 'idleDim', 'sleepAC', 'sleepBattery', 'powerProfile'].includes(key)) throw err('bad_args', 'Onbekende energie-instelling.');
      if (!target.available) throw err('unsupported', 'Deze energie-instelling is niet beschikbaar.');
      if (!target.writable) throw err('denied', 'Deze energie-instelling kan niet worden gewijzigd.');
      if (key === 'idleDelay') {
        if (!integer(value, 4294967295)) throw err('bad_args', 'Geef een geldige schermtijd op.');
        target.idleDelay = value;
      } else if (key === 'idleDim') {
        if (typeof value !== 'boolean') throw err('bad_args', 'Geef aan of het scherm moet dimmen.');
        target.enabled = value;
      } else if (key === 'powerProfile') {
        if (!target.choices.some((p) => p.id === value && !p.disabled)) throw err('unsupported', 'Deze energiestand is niet beschikbaar.');
        target.active = value;
      } else {
        if (!value || Object.keys(value).sort().join(',') !== 'action,timeout'
          || !['nothing', 'suspend'].includes(value.action) || !integer(value.timeout, 2147483647)
          || (value.action === 'suspend' && (!state.canSuspend || !value.timeout))) throw err('bad_args', 'Geef een geldige slaaptijd op.');
        Object.assign(target, value);
      }
      const result = clone(state);
      m.emit('system.power.settings.changed', result);
      return result;
    },
    'system.power.action': async ({ action } = {}) => {
      if (!['shutdown', 'reboot', 'suspend', 'logout', 'lock'].includes(action)) throw err('invalid_action', 'Onbekende actie.');
      console.info('[mock] system.power.action (not executed):', action);
      return dry();
    },
    'system.nightLight.get': async (a, m) => {
      const n = S(m).nightLight;
      return { supported: n.available, enabled: n.enabled, temperature: n.temperature, tool: n.available ? 'hyprsunset' : null };
    },
    'system.nightLight.set': async ({ enabled, temperature } = {}, m) => {
      const s = S(m);
      if (!s.nightLight.available) throw err('unsupported', 'Nachtlampje werkt alleen in de Win11 Shell-sessie (Hyprland).');
      if (enabled != null) s.nightLight.enabled = !!enabled;
      if (temperature != null) s.nightLight.temperature = clampInt(temperature, 1500, 6500);
      return dry({ supported: true, enabled: s.nightLight.enabled, temperature: s.nightLight.temperature, tool: 'hyprsunset' });
    },

    /* ---- information ---- */
    'system.info': async (a, m) => {
      const s = S(m);
      return {
        hostname: 'robin-pc',
        prettyHostname: 'robin-pc',
        os: 'Ubuntu 26.04.1 LTS',
        osVersion: '26.04.1 LTS (Resolute Raccoon)',
        osId: 'ubuntu',
        kernel: '7.0.0-38-generic',
        architecture: 'x86_64',
        deviceModel: 'Desktop-pc (MSI MAG B650 TOMAHAWK WIFI)',
        cpu: { model: 'AMD Ryzen 9 7900X 12-Core Processor', cores: 12, threads: 24 },
        memory: { total: 63 * 1024 ** 3 + 512 * 1024 ** 2, installed: 64 * 1024 ** 3 },
        gpus: ['NVIDIA GeForce RTX 4070 Ti'],
        disks: [
          { model: 'Samsung SSD 990 PRO 2TB', size: 2000398934016 },
          { model: 'WDC WD221KFGX-68B9KN0', size: 22000969973760 },
          { model: 'Crucial MX500 1TB', size: 1000204886016 },
        ],
        uptime: Math.round((Date.now() - s.startedAt) / 1000) + 3 * 3600 + 17 * 60,
        user: { name: 'robin', realName: 'Robin', avatar: null },
        desktop: 'Win11 Shell (Hyprland)',
        sessionType: 'wayland',
        installedAt: Date.now() - 41 * 86400e3,
      };
    },
    'system.updates': async (a, m) => clone(S(m).updates),
    'system.openUpdater': async () => dry(),
    'system.time': async () => ({ timezone: 'Europe/Amsterdam', ntp: true, h24: true }),
  },

  start(m) {
    const s = S(m);
    // Wi-Fi signal drifts a little (the host only emits when something changed)
    setInterval(() => {
      const active = s.wifi.find((w) => w.active);
      if (!active || !s.network.wifiEnabled || s.network.airplane) return;
      const next = clampInt(active.strength + (Math.random() * 10 - 5), 55, 95);
      if (Math.abs(next - active.strength) >= 3) { active.strength = next; emitNetwork(m); }
    }, 12000);
    // A laptop battery slowly drains
    setInterval(() => {
      if (!s.battery.present || s.battery.onAC) return;
      s.battery.percent = Math.max(3, s.battery.percent - 1);
      s.battery.timeToEmpty = Math.max(0, s.battery.timeToEmpty - 150);
      emitBattery(m);
    }, 60000);
    // Bluetooth discovery: devices in pairing mode show up one by one; auto-stop after 60 s unless renewed
    setInterval(() => {
      const b = s.bluetooth;
      if (!b.available) return;
      const now = Date.now();
      let changed = false;
      if (b.discovering && now > b.discoverUntil) { btStopDiscovery(b); changed = true; }
      if (b.discovering && b.powered && !b.stuck) {
        for (const d of BT_NEARBY) {
          if (!b.found.includes(d.address) && now - b.discoverStarted >= d.after) { b.found.push(d.address); changed = true; }
          // the RSSI of what was found drifts, and every change is a system.bluetooth.changed (as on the host,
          // where rssi is in the payload): the UI must not lose focus or clicks over that churn
          else if (b.found.includes(d.address) && !btPaired(b, d.address) && Math.random() < 0.15) {
            b.rssi[d.address] = d.rssi + Math.round(Math.random() * 6 - 3);
            changed = true;
          }
        }
      } else if (!b.discovering && b.seenUntil && now > b.seenUntil) {
        b.seenUntil = 0;
        b.found = [];
        changed = true;
      }
      if (changed) emitBluetooth(m);
    }, 400);
    // Console helpers for manual testing
    try {
      window.__mockSystem = {
        state: s,
        setBattery(patch) { Object.assign(s.battery, { present: true, percent: 76, state: 'discharging', onAC: false }, patch); emitBattery(m); },
        setVolume(v) { s.audio.volume = clampInt(v, 0, 100); emitAudio(m); },
        setBrightness(v) { s.brightness.value = clampInt(v, 1, 100); emitBrightness(m); },
        emitAll() { emitAudio(m); emitBrightness(m); emitBattery(m); emitNetwork(m); emitBluetooth(m); },
        btStuck(on = true) {
          const b = s.bluetooth;
          Object.assign(b, on ? { stuck: true, powered: true, powerState: 'on-disabling' } : { stuck: false, powerState: b.powered ? 'on' : 'off' });
          if (on) { b.devices.forEach((d) => { d.connected = false; }); btStopDiscovery(b); }
          btSyncSinks(m);
          emitBluetooth(m);
        },
      };
    } catch { /* ignore */ }
  },
};
