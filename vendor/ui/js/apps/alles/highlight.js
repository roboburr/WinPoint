// Alles — highlight the matched parts of a name (Everything shows them in bold).
//
//   highlightInto(el, 'Vakantiefoto Texel.jpg', { terms: ['foto', 'texel'], regex: null, case: false })

let rxCache = { src: null, flags: '', rx: null };

function regexFor(src, cs) {
  const flags = cs ? 'g' : 'gi';
  if (rxCache.src === src && rxCache.flags === flags) return rxCache.rx;
  let rx = null;
  try { rx = new RegExp(src, flags); } catch { rx = null; }
  rxCache = { src, flags, rx };
  return rx;
}

/** Matched [start, end) ranges in name, merged and sorted. */
export function matchRanges(name, hl) {
  if (!name || !hl) return [];
  const out = [];
  const cs = !!hl.case;
  const hay = cs ? name : name.toLowerCase();
  const sameLen = hay.length === name.length;
  for (const term of hl.terms || []) {
    if (!term || term.length > name.length + 2) continue;
    const needle = cs ? term : term.toLowerCase();
    if (!sameLen || !needle) continue;
    let p = hay.indexOf(needle);
    let guard = 0;
    while (p >= 0 && guard++ < 50) {
      out.push([p, p + needle.length]);
      p = hay.indexOf(needle, p + needle.length);
    }
  }
  if (hl.regex) {
    const rx = regexFor(hl.regex, cs);
    if (rx) {
      rx.lastIndex = 0;
      let m;
      let guard = 0;
      while ((m = rx.exec(name)) && guard++ < 50) {
        if (m[0].length === 0) { rx.lastIndex++; continue; }
        out.push([m.index, m.index + m[0].length]);
      }
    }
  }
  if (out.length < 2) return out;
  out.sort((a, b) => a[0] - b[0] || b[1] - a[1]);
  const merged = [out[0]];
  for (let i = 1; i < out.length; i++) {
    const last = merged[merged.length - 1];
    if (out[i][0] <= last[1]) last[1] = Math.max(last[1], out[i][1]);
    else merged.push(out[i]);
  }
  return merged;
}

/** Replace el's content with name, matched parts wrapped in <b class="alles-hl">. */
export function highlightInto(el, name, hl) {
  const ranges = matchRanges(name, hl);
  if (!ranges.length) {
    el.textContent = name;
    return;
  }
  const frag = document.createDocumentFragment();
  let pos = 0;
  for (const [a, b] of ranges) {
    if (a > pos) frag.append(name.slice(pos, a));
    const mark = document.createElement('b');
    mark.className = 'alles-hl';
    mark.textContent = name.slice(a, b);
    frag.append(mark);
    pos = b;
  }
  if (pos < name.length) frag.append(name.slice(pos));
  el.replaceChildren(frag);
}
