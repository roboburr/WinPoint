// Schijven — read a number the way the current language writes it (no imports: tests run it in node).
//   parseLocaleNumber('1.234,56', 'nl-NL') → 1234.56      parseLocaleNumber('1,234.56', 'en-GB') → 1234.56
//   parseLocaleNumber('500,5', 'nl-NL') → 500.5           parseLocaleNumber('500.5', 'en-GB') → 500.5
// The group separator only groups when every group has three digits; otherwise a single one was meant as the
// decimal point ('500.5' typed in Dutch → 500.5, not 5005). Anything else → NaN.
// (Could move next to number() in core/format.js.)

const SEP = new Map();

function separators(loc) {
  let s = SEP.get(loc);
  if (!s) {
    const parts = new Intl.NumberFormat(loc).formatToParts(12345.6);
    s = { group: parts.find((p) => p.type === 'group')?.value || '', decimal: parts.find((p) => p.type === 'decimal')?.value || '.' };
    SEP.set(loc, s);
  }
  return s;
}

export function parseLocaleNumber(text, loc = 'nl-NL') {
  const { group, decimal } = separators(loc);
  const s = String(text ?? '').trim().replace(/[\s  ]/g, '');
  if (!s) return NaN;
  const g = group && group.trim() ? group : null;
  let [int, frac, ...rest] = s.split(decimal);
  if (rest.length) return NaN;
  if (g && int.includes(g)) {
    const chunks = int.split(g);
    if (/^\+?\d{1,3}$/.test(chunks[0]) && chunks.slice(1).every((c) => /^\d{3}$/.test(c))) int = chunks.join('');
    else if (chunks.length === 2 && frac === undefined) [int, frac] = chunks;
    else return NaN;
  }
  const num = frac === undefined ? int : `${int}.${frac}`;
  if (!/^\+?(\d+(\.\d*)?|\.\d+)$/.test(num)) return NaN;
  return Number(num);
}
