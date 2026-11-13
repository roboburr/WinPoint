// Inventory script of 2026-10-27, copied unchanged from notes/i18n-tools/pairs.mjs (notes/english-option-design.md §3);
// it reads its tokens from ./tok.mjs. Usage: node tools/i18n/pairs.mjs ui OUT.json
// Evaluate each makeT dict by extracting its object literal source and pairing nl/en values per key.
import fs from 'node:fs'; import path from 'node:path';
import { tokenize } from './tok.mjs';
const ROOT = process.argv[2];
const files = [];
(function walk(d) { for (const e of fs.readdirSync(d, { withFileTypes: true })) { const p = path.join(d, e.name); if (e.isDirectory()) walk(p); else if (p.endsWith('.js')) files.push(p); } })(path.join(ROOT, 'js'));
const rows = [];
for (const f of files) {
  const src = fs.readFileSync(f, 'utf8');
  const toks = tokenize(src).filter(t => !t.inTpl);
  for (let k = 0; k < toks.length; k++) {
    if (!(toks[k].value === 'makeT' && toks[k + 1]?.value === '(' && toks[k + 2]?.value === '{')) continue;
    // find nl: { ... } and en: { ... } at depth 1
    let depth = 0, cur = null; const vals = { nl: {}, en: {} }; let key = null;
    for (let j = k + 2; j < toks.length; j++) {
      const t = toks[j];
      if (t.value === '{' || t.value === '(' || t.value === '[') {
        if (depth === 1 && t.value === '{' && toks[j - 1]?.value === ':' && ['nl', 'en'].includes(toks[j - 2]?.value)) cur = toks[j - 2].value;
        depth++; continue;
      }
      if (t.value === '}' || t.value === ')' || t.value === ']') { depth--; if (depth === 1) cur = null; if (depth === 0) break; continue; }
      if (cur && depth === 2 && (t.type === 'id' || t.type === 'str') && toks[j + 1]?.value === ':' && [',', '{'].includes(toks[j - 1]?.value)) { key = t.value; vals[cur][key] = []; continue; }
      if (cur && key && (t.type === 'str' || t.type === 'tpl')) vals[cur][key].push(t.value);
    }
    for (const key of Object.keys(vals.nl)) rows.push({ file: path.relative(ROOT, f), key, nl: vals.nl[key].join(' | '), en: (vals.en[key] || []).join(' | ') });
  }
}
fs.writeFileSync(process.argv[3], JSON.stringify(rows));
console.log('pairs', rows.length, 'identical', rows.filter(r => r.nl === r.en).length);
