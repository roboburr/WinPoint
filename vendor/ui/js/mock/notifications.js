// Win11 Shell — mock notifications.* methods (owned by the flyouts agent). SPEC §6.6.
//
// Shapes mirror host/services/notifications.py: urgency 'low'|'normal'|'critical', hasDefault (clicking =
// action 'default'), desktopId, category, transient, resident, value, silent, source; list newest first
// with `dnd`; dismiss → {ok}, clear → {cleared}, test → {notification}; events notifications.new/closed
// ({id, reason: 1 expired | 2 dismissed | 3 closed}) and notifications.action {id, key}.
// Six sample notifications (Firefox, Telegram ×2, Software-updates, Discord, Win11 Shell)
// and a new one every ~40 s from a small pool (emitted as notifications.new).
// Test switches (URL): ?mockNotifications=0 (start empty) · ?mockNotifyEvery=<sec> ·
//   ?mockToast=1 (emit one new notification ~1 s after start, for toast screenshots)

const clone = (v) => JSON.parse(JSON.stringify(v));
const err = (code, message) => Object.assign(new Error(message), { code });

/** A small original "photo" (SVG data URL) for image notifications. */
function picture(h1 = 205, h2 = 28) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 320 180">
<defs><linearGradient id="s" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="hsl(${h1} 80% 62%)"/><stop offset="1" stop-color="hsl(${h2} 95% 78%)"/></linearGradient></defs>
<rect width="320" height="180" fill="url(#s)"/><circle cx="238" cy="58" r="22" fill="#fff8e1" opacity=".9"/>
<path d="M0 120 C60 88 110 104 160 116 S260 92 320 104 V180 H0Z" fill="hsl(${h1 + 120} 35% 42%)"/>
<path d="M0 148 C70 128 140 156 210 140 S290 128 320 136 V180 H0Z" fill="hsl(${h1 + 110} 40% 28%)"/></svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

/** A friendly initials avatar (square SVG data URL) — shown round, left of the text. */
function avatar(initials, hue) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96"><defs><linearGradient id="a" x1="0" y1="0" x2="1" y2="1">` +
    `<stop offset="0" stop-color="hsl(${hue} 75% 62%)"/><stop offset="1" stop-color="hsl(${hue + 30} 70% 45%)"/></linearGradient></defs>` +
    `<rect width="96" height="96" fill="url(#a)"/><text x="48" y="60" font-family="Segoe UI, Ubuntu Sans, sans-serif" font-size="36" font-weight="600" fill="#fff" text-anchor="middle">${initials}</text></svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

function samples(now) {
  const min = 60000;
  return [
    {
      appName: 'Win11 Shell', appIcon: 'win11shell', summary: 'Welkom bij Win11 Shell',
      body: 'Druk op de Windows-toets om Start te openen. In Instellingen pas je alles aan zoals jij het wilt.',
      actions: [{ key: 'settings', label: 'Instellingen openen' }], time: now - 180 * min, urgency: 'normal', image: null,
    },
    {
      appName: 'Discord', appIcon: 'discord', summary: 'Mark in #algemeen',
      body: 'Kijk wat een mooie zonsondergang gisteren! Wie doet er vanavond mee met een potje?',
      actions: [{ key: 'reply', label: 'Antwoorden' }], time: now - 95 * min, urgency: 'normal', image: picture(205, 28),
    },
    {
      appName: 'Software-updates', appIcon: 'system-software-update', summary: 'Updates beschikbaar',
      body: 'Er staan 3 updates klaar. Installeren duurt ongeveer 5 minuten.',
      actions: [{ key: 'open', label: 'Bekijken' }, { key: 'later', label: 'Later' }], time: now - 62 * min, urgency: 'normal', image: null,
    },
    {
      appName: 'Telegram', appIcon: 'telegram', summary: 'Familie',
      body: 'Mama: Zondag om 13:00 lunch bij oma, nemen jullie de taart mee? 🎂',
      actions: [{ key: 'reply', label: 'Antwoorden' }, { key: 'read', label: 'Gelezen' }], time: now - 34 * min, urgency: 'normal', image: null,
    },
    {
      appName: 'Firefox', appIcon: 'firefox', summary: 'Download voltooid',
      body: 'vakantiefoto’s-2026.zip (248 MB) staat in Downloads.',
      actions: [{ key: 'open', label: 'Openen' }, { key: 'folder', label: 'Map weergeven' }], time: now - 12 * min, urgency: 'normal', image: null,
    },
    {
      appName: 'Telegram', appIcon: 'telegram', summary: 'Sanne',
      body: 'Hoi! Zie ik je zaterdag bij de verjaardag? 🎉',
      actions: [{ key: 'reply', label: 'Antwoorden' }, { key: 'read', label: 'Gelezen' }], time: now - 4 * min, urgency: 'normal', image: avatar('S', 330),
    },
  ];
}

const DESKTOP_ID = {
  'Win11 Shell': 'win11shell-settings.desktop', Discord: 'discord_discord.desktop', Telegram: 'telegram-desktop_telegram-desktop.desktop',
  Firefox: 'firefox_firefox.desktop', Thunderbird: 'thunderbird_thunderbird.desktop', Agenda: 'org.gnome.Calendar.desktop',
  'Software-updates': 'update-manager.desktop',
};
/** Fill in the host's extra fields so the UI sees exactly what the real service sends. */
function full(n, m) {
  return {
    actions: [], hasDefault: true, urgency: 'normal', image: null,
    desktopId: DESKTOP_ID[n.appName] || null, category: null, transient: false, resident: false, value: null,
    expireTimeout: -1, silent: !!m.settings()?.notifications?.dnd, source: 'test',
    ...n,
  };
}

const POOL = [
  { appName: 'Telegram', appIcon: 'telegram', summary: 'Sanne', body: 'Neem jij de slingers mee? Ik heb de ballonnen al 🎈', actions: [{ key: 'reply', label: 'Antwoorden' }, { key: 'read', label: 'Gelezen' }] },
  { appName: 'Agenda', appIcon: 'x-office-calendar', summary: 'Tandarts om 10:30', body: 'Over 15 minuten · Kerkstraat 12, Utrecht', actions: [{ key: 'snooze', label: 'Uitstellen' }, { key: 'dismiss', label: 'Sluiten' }] },
  { appName: 'Thunderbird', appIcon: 'thunderbird', summary: 'Nieuwe e-mail van Bibliotheek Utrecht', body: 'Je gereserveerde boek ligt klaar om op te halen.', actions: [] },
  { appName: 'Firefox', appIcon: 'firefox', summary: 'Download voltooid', body: 'handleiding-wasmachine.pdf (2,4 MB)', actions: [{ key: 'open', label: 'Openen' }, { key: 'folder', label: 'Map weergeven' }] },
  { appName: 'Discord', appIcon: 'discord', summary: 'Lisa in #muziek', body: 'Deze plaat moet je echt eens luisteren, echt prachtig.', actions: [{ key: 'reply', label: 'Antwoorden' }] },
  { appName: 'Win11 Shell', appIcon: 'win11shell', summary: 'Tip: klembordgeschiedenis', body: 'Druk op Windows-toets + V om eerder gekopieerde tekst en afbeeldingen terug te vinden.', actions: [] },
];

function S(m) {
  if (!m.state.notifications) {
    const start = m.params.get('mockNotifications') === '0' ? [] : samples(Date.now());
    let nextId = 1;
    m.state.notifications = { nextId: 0, items: start.map((n) => full({ id: nextId++, hasDefault: n.appName !== 'Software-updates', ...n }, m)) };
    m.state.notifications.nextId = nextId;
  }
  return m.state.notifications;
}

function push(m, n) {
  const s = S(m);
  const item = full({ ...clone(n), id: s.nextId++, time: Date.now() }, m);
  s.items.push(item);
  if (s.items.length > 50) {
    const old = s.items.shift();
    m.emit('notifications.closed', { id: old.id, reason: 1 });
  }
  m.emit('notifications.new', { notification: clone(item) });
  return item;
}

function close(m, id, reason = 2) {
  const s = S(m);
  const i = s.items.findIndex((x) => x.id === id);
  if (i < 0) return false;
  s.items.splice(i, 1);
  m.emit('notifications.closed', { id, reason });
  return true;
}

export default {
  methods: {
    'notifications.list': async (a, m) => ({
      supported: true, dnd: !!m.settings()?.notifications?.dnd,
      items: clone(S(m).items).sort((x, y) => (y.time - x.time) || (y.id - x.id)),
    }),
    'notifications.dismiss': async ({ id } = {}, m) => {
      if (!Number.isFinite(Number(id))) throw err('bad_args', 'Ongeldige melding.');
      return { ok: close(m, Number(id), 2) };
    },
    'notifications.clear': async (a, m) => {
      const ids = S(m).items.map((n) => n.id);
      for (const nid of ids) close(m, nid, 2);
      return { cleared: ids.length };
    },
    'notifications.invokeAction': async ({ id, key = 'default' } = {}, m) => {
      const n = S(m).items.find((x) => x.id === Number(id));
      if (!n) throw err('not_found', 'Deze melding bestaat niet meer.');
      console.info('[mock] notification action', n.appName, key);
      m.emit('notifications.action', { id: n.id, key });
      if (key === 'settings') m.emit('shell.openApp', { app: 'settings', params: {} });
      if (n.appName === 'Software-updates' && key === 'open') m.emit('shell.openApp', { app: 'settings', params: { page: 'update' } });
      if (!n.resident) close(m, n.id, 2);
      return {};
    },
    'notifications.test': async (a, m) => {
      const en = m.settings()?.language === 'en';
      const item = push(m, {
        appName: 'Win11 Shell', appIcon: 'preferences-system-notifications', desktopId: 'win11shell-settings.desktop',
        summary: en ? 'This is what a notification looks like' : 'Zo ziet een melding eruit',
        body: en ? 'Notifications from your apps appear here and stay in the notification centre.'
          : 'Meldingen van je apps verschijnen hier en blijven bewaard in het meldingencentrum.',
        actions: [{ key: 'ok', label: en ? 'Got it' : 'Prima' }],
      });
      return { notification: clone(item) };
    },
  },

  start(m) {
    S(m);
    let i = 0;
    const every = Math.max(5, Number(m.params.get('mockNotifyEvery')) || 40) * 1000;
    setInterval(() => { push(m, POOL[i++ % POOL.length]); }, every);
    if (m.params.get('mockToast') === '1') {
      setTimeout(() => push(m, {
        appName: 'Telegram', appIcon: 'telegram', summary: 'Sanne', image: avatar('S', 330),
        body: 'Hoi! Zie ik je zaterdag bij de verjaardag? Ik neem taart mee 🎉',
        actions: [{ key: 'reply', label: 'Antwoorden' }, { key: 'read', label: 'Markeren als gelezen' }],
      }), 900);
    }
  },
};
