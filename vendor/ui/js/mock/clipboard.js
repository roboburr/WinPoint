// Win11 Shell — mock clipboard.* methods (owned by the flyouts agent). SPEC §6.9.
//
// Eight history items (texts, a link, an address, a long paragraph and one image
// drawn as an inline SVG data URL); one item is pinned. "clear" keeps pinned items,
// like Windows. Test switch (URL): ?mockClipboard=off (unsupported, like GNOME) ·
// ?mockClipboard=empty (supported, but nothing copied yet)
// Shapes mirror host/services/clipboard.py: items {id, kind, text|preview, time, pinned, size, mime?};
// unsupported → every method answers {supported:false} (no error); paste → {ok}, remove → {ok},
// clear → {removed}, pin → {id, pinned} (max 25 pinned, error 'too_many').

const clone = (v) => JSON.parse(JSON.stringify(v));
const err = (code, message) => Object.assign(new Error(message), { code });

function screenshot() {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 270">
<defs><linearGradient id="b" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2b6fd6"/><stop offset="1" stop-color="#7a3fd1"/></linearGradient></defs>
<rect width="480" height="270" fill="url(#b)"/>
<rect x="60" y="40" width="360" height="200" rx="10" fill="#f3f3f3"/>
<rect x="60" y="40" width="360" height="30" rx="10" fill="#e6e6e6"/><rect x="60" y="58" width="360" height="12" fill="#e6e6e6"/>
<circle cx="392" cy="55" r="5" fill="#c42b1c"/><circle cx="374" cy="55" r="5" fill="#9a9a9a"/><circle cx="356" cy="55" r="5" fill="#9a9a9a"/>
<rect x="80" y="90" width="150" height="12" rx="6" fill="#0067c0"/>
<rect x="80" y="114" width="300" height="8" rx="4" fill="#c8c8c8"/><rect x="80" y="130" width="260" height="8" rx="4" fill="#c8c8c8"/>
<rect x="80" y="146" width="280" height="8" rx="4" fill="#c8c8c8"/>
<rect x="80" y="172" width="96" height="44" rx="6" fill="#ffb900"/><rect x="186" y="172" width="96" height="44" rx="6" fill="#00b294"/><rect x="292" y="172" width="96" height="44" rx="6" fill="#e74856"/>
</svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

function S(m) {
  if (!m.state.clipboard) {
    const now = Date.now(), min = 60000;
    const mode = m.params.get('mockClipboard');
    const items = mode === 'empty' ? [] : [
      { id: 'c8', kind: 'text', text: 'Afspraak tandarts: dinsdag 4 november om 10:30', time: now - 2 * min, pinned: false },
      { id: 'c7', kind: 'image', preview: screenshot(), mime: 'image/svg+xml', size: 1420, time: now - 9 * min, pinned: false },
      { id: 'c6', kind: 'text', text: 'https://www.rijksmuseum.nl/nl/bezoeken', time: now - 25 * min, pinned: false },
      { id: 'c5', kind: 'text', text: 'Beste buren,\n\nZaterdag 1 november geven wij een klein feestje in de tuin. Het kan daardoor iets drukker zijn dan normaal. Mocht u last hebben, belt u dan gerust aan. Natuurlijk bent u ook van harte welkom vanaf 16:00!\n\nGroeten, Robin en Sanne', time: now - 70 * min, pinned: false },
      { id: 'c4', kind: 'text', text: 'Kerkstraat 12\n3511 AB Utrecht', time: now - 3 * 60 * min, pinned: true },
      { id: 'c3', kind: 'text', text: 'Track & trace: 3SWIN11SHELL2026', time: now - 5 * 60 * min, pinned: false },
      { id: 'c2', kind: 'text', text: 'Boodschappen: melk, brood, kaas, appels, koffie', time: now - 26 * 60 * min, pinned: false },
      { id: 'c1', kind: 'text', text: '06 12 34 56 78', time: now - 50 * 60 * min, pinned: false },
    ];
    for (const it of items) if (it.kind === 'text') it.size = new TextEncoder().encode(it.text).length;
    m.state.clipboard = { supported: mode !== 'off', items, nextId: 9 };
  }
  return m.state.clipboard;
}

const MAX_PINNED = 25;

const changed = (m) => m.emit('clipboard.changed', {});

export default {
  methods: {
    'clipboard.list': async (a, m) => {
      const s = S(m);
      return { supported: s.supported, items: s.supported ? clone(s.items) : [] };
    },
    'clipboard.paste': async ({ id } = {}, m) => {
      const s = S(m);
      if (!s.supported) return { supported: false };
      const i = s.items.findIndex((x) => x.id === id);
      if (i < 0) throw err('not_found', 'Dit item staat niet meer in het klembord.');
      // copying an item back makes it the newest entry
      const [item] = s.items.splice(i, 1);
      item.time = Date.now();
      s.items.unshift(item);
      changed(m);
      return { ok: true, dryRun: true };
    },
    'clipboard.remove': async ({ id } = {}, m) => {
      const s = S(m);
      if (!s.supported) return { supported: false };
      const before = s.items.length;
      s.items = s.items.filter((x) => x.id !== id);
      const ok = s.items.length < before;
      if (ok) changed(m);
      return { ok };
    },
    'clipboard.clear': async (a, m) => {
      const s = S(m);
      if (!s.supported) return { supported: false };
      const before = s.items.length;
      s.items = s.items.filter((x) => x.pinned);
      changed(m);
      return { removed: before - s.items.length };
    },
    'clipboard.pin': async ({ id, pinned = true } = {}, m) => {
      const s = S(m);
      if (!s.supported) return { supported: false };
      const item = s.items.find((x) => x.id === id);
      if (!item) throw err('not_found', 'Dit item staat niet meer in het klembord.');
      if (pinned && !item.pinned && s.items.filter((x) => x.pinned).length >= MAX_PINNED) {
        throw err('too_many', `Je kunt maximaal ${MAX_PINNED} items vastmaken.`);
      }
      item.pinned = !!pinned;
      changed(m);
      return { id: item.id, pinned: item.pinned };
    },
  },
  start(m) {
    const s = S(m);
    try {
      window.__mockClipboard = {
        copy(text) { s.items.unshift({ id: `c${s.nextId++}`, kind: 'text', text: String(text), size: new TextEncoder().encode(String(text)).length, time: Date.now(), pinned: false }); changed(m); },
      };
    } catch { /* ignore */ }
  },
};
