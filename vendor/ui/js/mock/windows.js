// Win11 Shell — mock windows.* (owned by the shell agent). SPEC §6.4.
// Pretends to be Hyprland: supported:true, a handful of realistic windows, focus that
// moves now and then (events windows.focused + windows.changed), and fake windows for
// apps started with apps.launch (via m.state.mockWindows.spawn).
// Window shape mirrors host/services/windows.py (extras: workspaceName, maximized, urgent,
// snapped, monitor, xwayland, pinned, initialClass, x/y/w/h). close/minimize/toggleMaximize/snap/setPinned
// without id/pid act on the active window; windows.restore; windows.showDesktop → {ok, desktop}.
// windows.setIconRects (taskbar button rects for the plugin's animations) and windows.focus/restore
// {fromRect} are accepted and only remembered: m.state.iconRects (last message), m.state.iconRectsCalls,
// m.state.fromRects (the fromRect of every focus/restore, newest last), also on window.__mockWindowAnim.
// setIconRects answers anim: true (the plugin would animate) unless a test sets m.state.anim = false.
// ?mockStatic=1 disables the periodic focus changes (stable screenshots).

let seq = 0x5f0000;
let pidSeq = 3100;
const mk = (appId, desktopId, icon, title, extra = {}) => ({
  id: `0x${(seq += 0x1a40).toString(16)}`,
  appId, title, workspace: 1, workspaceName: '1', minimized: false, focused: false, desktopId, icon,
  pid: (pidSeq += 137), fullscreen: false, maximized: false, floating: true, urgent: false, snapped: null,
  monitor: 0, xwayland: false, pinned: false, initialClass: appId,
  x: 180 + (seq % 7) * 40, y: 90 + (seq % 5) * 30, w: 1100, h: 720, ...extra,
});

const wins = [
  mk('firefox', 'firefox_firefox.desktop', 'firefox', 'Weerbericht Utrecht – Buienradar — Mozilla Firefox'),
  mk('firefox', 'firefox_firefox.desktop', 'firefox', 'Stamppot boerenkool – Recept – Allerhande — Mozilla Firefox'),
  mk('org.gnome.Nautilus', 'org.gnome.Nautilus.desktop', 'org.gnome.Nautilus', 'Downloads'),
  mk('org.gnome.Ptyxis', 'org.gnome.Ptyxis.desktop', 'org.gnome.Ptyxis', 'robin@thuis-pc: ~'),
  mk('discord', 'discord_discord.desktop', 'discord', '#algemeen | Familie – Discord', { xwayland: true }),
  mk('org.gnome.TextEditor', 'org.gnome.TextEditor.desktop', 'org.gnome.TextEditor', 'Boodschappenlijst.txt',
    { minimized: true, workspace: -99, workspaceName: 'special:minimized' }),
];
wins[0].focused = true;

let shown = null; // ids minimized by showDesktop (to restore on the second call)
const EDGES = ['left', 'right', 'top-left', 'top-right', 'bottom-left', 'bottom-right', 'maximize', 'restore', 'up', 'down'];

function err(code, message) { const e = new Error(message); e.code = code; return e; }
function find(args = {}, allowActive = false) {
  const hasKey = args && (args.id != null || args.pid != null);
  const w = hasKey
    ? wins.find((x) => (args.id != null && x.id === args.id) || (args.pid != null && x.pid === args.pid))
    : (allowActive ? wins.find((x) => x.focused) : null);
  if (!w) throw err('not_found', 'Dit venster bestaat niet meer.');
  return w;
}
function snapshot() { return { supported: true, windows: wins.map((w) => ({ ...w })) }; }
function changed(m) { m.emit('windows.changed', { windows: wins.map((w) => ({ ...w })) }); }
function animState(m) {
  if (!m.state.fromRects) { m.state.fromRects = []; m.state.iconRects = null; m.state.iconRectsCalls = 0; }
  window.__mockWindowAnim = m.state;
  return m.state;
}
function noteFromRect(m, w, args) {
  const r = args?.fromRect;
  if (r == null) return;
  if (!['x', 'y', 'w', 'h'].every((k) => Number.isFinite(r[k])) || r.w <= 0 || r.h <= 0 || r.w > 4096 || r.h > 4096) {
    throw err('bad_args', 'Ongeldige rechthoeken.');
  }
  animState(m).fromRects.push({ id: w.id, minimized: w.minimized, rect: { x: r.x, y: r.y, w: r.w, h: r.h } });
}
function setMin(w, on) {
  w.minimized = on;
  w.workspace = on ? -99 : 1;
  w.workspaceName = on ? 'special:minimized' : '1';
}
function setFocus(m, w) {
  for (const x of wins) x.focused = false;
  if (w) { w.focused = true; setMin(w, false); w.urgent = false; }
  m.emit('windows.focused', { id: w ? w.id : null });
  changed(m);
}

export default {
  methods: {
    'windows.list': async () => snapshot(),

    'windows.focus': async (args, m) => { const w = find(args); noteFromRect(m, w, args); shown = null; setFocus(m, w); return { ok: true }; },
    'windows.restore': async (args, m) => { const w = find(args); noteFromRect(m, w, args); shown = null; setFocus(m, w); return { ok: true }; },
    // where each window's taskbar button settles (host: aims the minimize / restore animations)
    'windows.setIconRects': async (args = {}, m) => {
      const rects = Array.isArray(args.rects) ? args.rects : null;
      if (!rects || rects.length > 64 || (args.layer != null && args.layer !== 'shell')) throw err('bad_args', 'Ongeldige rechthoeken.');
      const ids = new Set();
      for (const r of rects) {
        const ok = r && ['x', 'y', 'w', 'h'].every((k) => Number.isFinite(r[k])) && r.w > 0 && r.h > 0 && r.w <= 512 && r.h <= 512;
        const list = Array.isArray(r?.ids) ? r.ids : (r?.id != null ? [r.id] : null);
        if (!ok || !list || list.length > 64) throw err('bad_args', 'Ongeldige rechthoeken.');
        for (const id of list) ids.add(String(id));
      }
      if (ids.size > 256) throw err('bad_args', 'Ongeldige rechthoeken.');
      const st = animState(m);
      st.iconRects = { hidden: !!args.hidden, rects };
      st.iconRectsCalls += 1;
      return { ok: true, count: ids.size, anim: st.anim !== false };
    },

    'windows.close': async (args, m) => {
      const w = find(args, true);
      const i = wins.indexOf(w);
      wins.splice(i, 1);
      if (w.focused) setFocus(m, wins.filter((x) => !x.minimized).slice(-1)[0] || null);
      else changed(m);
      return { ok: true };
    },

    'windows.minimize': async (args, m) => {
      const w = find(args, true);
      setMin(w, true);
      if (w.focused) {
        w.focused = false;
        const next = [...wins].reverse().find((x) => !x.minimized);
        if (next) next.focused = true;
        m.emit('windows.focused', { id: next ? next.id : null });
      }
      changed(m);
      return { ok: true };
    },

    'windows.toggleMaximize': async (args, m) => {
      const w = find(args, true);
      w.maximized = !w.maximized;
      w.snapped = null;
      changed(m);
      return { ok: true, maximized: w.maximized };
    },
    // always on top (host: Hyprland `pin`; a minimized window is pinned again when it is restored)
    'windows.setPinned': async (args = {}, m) => {
      const w = find(args, true);
      const pinned = args.pinned == null ? true : !!args.pinned;
      if (w.minimized) return { ok: true, pinned, deferred: true };
      if (!!w.pinned !== pinned) { w.pinned = pinned; changed(m); }
      return { ok: true, pinned };
    },
    'windows.snap': async (args = {}, m) => {
      if (!EDGES.includes(args.edge)) throw err('bad_args', 'Onbekende positie.');
      const w = find(args, true);
      let edge = args.edge;
      if (edge === 'up') edge = w.snapped === 'left' ? 'top-left' : w.snapped === 'right' ? 'top-right' : 'maximize';
      if (edge === 'down') edge = w.snapped === 'left' ? 'bottom-left' : w.snapped === 'right' ? 'bottom-right' : 'restore';
      w.maximized = edge === 'maximize';
      w.snapped = edge === 'maximize' || edge === 'restore' ? null : edge;
      changed(m);
      return { ok: true, edge };
    },

    'windows.showDesktop': async (args, m) => {
      if (shown) {
        for (const w of wins) if (shown.includes(w.id)) setMin(w, false);
        const f = wins.find((w) => w.id === shown[0]);
        shown = null;
        setFocus(m, f || null);
        return { ok: true, desktop: false };
      }
      shown = wins.filter((w) => !w.minimized).sort((a, b) => b.focused - a.focused).map((w) => w.id);
      for (const w of wins) { setMin(w, true); w.focused = false; }
      m.emit('windows.focused', { id: null });
      changed(m);
      return { ok: true, desktop: true };
    },
  },

  start(m) {
    m.state.mockWindows = {
      spawn(app) {
        const appId = String(app.id).replace(/\.desktop$/, '').replace(/^(.+)_\1$/, '$1');
        const w = mk(appId, app.id, app.icon, app.name);
        wins.push(w);
        setTimeout(() => setFocus(m, w), 450); // apps take a moment to show their window
      },
    };
    if (m.params.get('mockStatic') === '1') return;
    // Focus wanders now and then, like a user clicking around in other windows.
    setInterval(() => {
      const visible = wins.filter((w) => !w.minimized);
      if (visible.length < 2 || document.hidden) return;
      const cur = visible.findIndex((w) => w.focused);
      setFocus(m, visible[(cur + 1) % visible.length]);
    }, 14000);
  },
};
