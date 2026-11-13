// Win11 Shell — mock media.* methods (owned by the flyouts agent). SPEC §6.7.
//
// One Spotify-like player (status 'playing', lowercase like the host) with a small playlist; next/previous cycle the
// tracks, playPause toggles. Album art = original generated SVG artwork (data URLs).
// Shape mirrors host/services/media.py: artPath (local file or null), canSeek, canControl, length/position (s),
// rate, desktopId, icon, updated (ms); players sorted playing → paused → stopped; control → {ok}.
// Test switches (URL): ?mockMedia=0 (no player) · ?mockMedia=2 (a second, paused player)

const clone = (v) => JSON.parse(JSON.stringify(v));
const err = (code, message) => Object.assign(new Error(message), { code });

function art(h1, h2, shape) {
  const shapes = {
    sun: `<circle cx="100" cy="92" r="46" fill="hsl(${h2} 95% 70%)"/><path d="M0 140 C50 118 90 150 140 132 S190 120 200 124 V200 H0Z" fill="hsl(${h1} 60% 22%)" opacity=".85"/>`,
    waves: [0, 1, 2, 3].map((i) => `<path d="M0 ${70 + i * 30} C40 ${50 + i * 30} 80 ${92 + i * 30} 120 ${70 + i * 30} S180 ${50 + i * 30} 200 ${68 + i * 30}" fill="none" stroke="hsl(${h2} 90% ${80 - i * 10}%)" stroke-width="10" stroke-linecap="round" opacity="${0.9 - i * 0.15}"/>`).join(''),
    rings: [70, 52, 34, 16].map((r, i) => `<circle cx="100" cy="100" r="${r}" fill="none" stroke="hsl(${h2 + i * 12} 85% ${72 - i * 6}%)" stroke-width="9"/>`).join(''),
    blocks: `<rect x="34" y="34" width="62" height="62" rx="10" fill="hsl(${h2} 90% 66%)"/><rect x="104" y="34" width="62" height="62" rx="31" fill="hsl(${h2 + 40} 90% 72%)"/><rect x="34" y="104" width="62" height="62" rx="31" fill="hsl(${h2 + 80} 80% 70%)"/><rect x="104" y="104" width="62" height="62" rx="10" fill="hsl(${h2 + 120} 85% 64%)"/>`,
  };
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1">` +
    `<stop offset="0" stop-color="hsl(${h1} 70% 38%)"/><stop offset="1" stop-color="hsl(${h1 + 40} 75% 18%)"/></linearGradient></defs>` +
    `<rect width="200" height="200" fill="url(#g)"/>${shapes[shape] || ''}</svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

const TRACKS = [
  { title: 'Zomeravond aan zee', artist: 'De Lichtjes', album: 'Kustlijn', artUrl: art(262, 32, 'sun'), length: 214 },
  { title: 'Midnight City Lights', artist: 'Northern Echo', album: 'Neon Harbor', artUrl: art(200, 190, 'waves'), length: 247 },
  { title: 'Golven', artist: 'Mira Visser', album: 'Getijden', artUrl: art(330, 300, 'rings'), length: 189 },
  { title: 'Kleurenblind', artist: 'Studio Pastel', album: 'Vormen', artUrl: art(150, 40, 'blocks'), length: 232 },
];
const ORDER = { playing: 0, paused: 1, stopped: 2 };

function S(m) {
  if (!m.state.media) {
    const mode = m.params.get('mockMedia');
    const players = [];
    if (mode !== '0') {
      players.push({ id: 'org.mpris.MediaPlayer2.spotify', identity: 'Spotify', desktopId: 'spotify_spotify.desktop', icon: 'spotify', status: 'playing', track: 0, pos: 37, since: Date.now() });
      if (mode === '2') players.push({ id: 'org.mpris.MediaPlayer2.firefox.instance_1_42', identity: 'Firefox', desktopId: 'firefox_firefox.desktop', icon: 'firefox', status: 'paused', track: 1, pos: 82, since: Date.now() });
    }
    m.state.media = { players };
  }
  return m.state.media;
}

function shape(p) {
  const t = TRACKS[p.track % TRACKS.length];
  const pos = p.status === 'playing' ? Math.min(t.length, (p.pos || 0) + (Date.now() - (p.since || Date.now())) / 1000) : (p.pos || 0);
  return {
    id: p.id, identity: p.identity, status: p.status,
    title: t.title, artist: t.artist, album: t.album, artUrl: t.artUrl, artPath: null,
    canNext: true, canPrevious: true, canPlay: true, canPause: true, canSeek: true, canControl: true,
    length: t.length, position: Math.round(pos), rate: 1.0,
    desktopId: p.desktopId, icon: p.icon, updated: Date.now(),
  };
}

const list = (m) => S(m).players.map(shape).sort((a, b) => (ORDER[a.status] ?? 3) - (ORDER[b.status] ?? 3));
function setStatus(p, status) {
  if (p.status === 'playing') p.pos = shape(p).position;
  p.status = status;
  p.since = Date.now();
}

export default {
  methods: {
    'media.list': async (a, m) => list(m),
    'media.control': async ({ id, action } = {}, m) => {
      const p = S(m).players.find((x) => x.id === id);
      if (!p) throw err('not_found', 'Deze speler is gesloten.');
      if (action === 'playPause') setStatus(p, p.status === 'playing' ? 'paused' : 'playing');
      else if (action === 'play') setStatus(p, 'playing');
      else if (action === 'pause') setStatus(p, 'paused');
      else if (action === 'stop') { setStatus(p, 'stopped'); p.pos = 0; }
      else if (action === 'next') { p.track = (p.track + 1) % TRACKS.length; p.pos = 0; setStatus(p, 'playing'); }
      else if (action === 'previous') { p.track = (p.track + TRACKS.length - 1) % TRACKS.length; p.pos = 0; setStatus(p, 'playing'); }
      else throw err('bad_args', 'Onbekende actie.');
      m.emit('media.changed', { players: clone(list(m)) });
      return { ok: true };
    },
  },
  start(m) {
    S(m);
    // the "song" ends every 3½ minutes while playing
    setInterval(() => {
      const p = S(m).players.find((x) => x.status === 'playing');
      if (!p) return;
      p.track = (p.track + 1) % TRACKS.length;
      p.pos = 0;
      p.since = Date.now();
      m.emit('media.changed', { players: clone(list(m)) });
    }, 210000);
  },
};
