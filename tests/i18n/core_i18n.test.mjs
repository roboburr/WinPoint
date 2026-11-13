// Unit tests of vendor/ui/js/core/i18n.js under node (node 20, no DOM). Ported from H's tests/i18n/core_i18n.test.mjs
// (E0a) in GE1a (notes/claude/round1/ge1a.md): only the paths changed (G's UI lives in vendor/ui; the temporary copy
// goes to .rt/ inside the repo, not /tmp).
//
//   node tests/i18n/core_i18n.test.mjs        → prints "ok N" and exits 0, or the failures and exits 1
//
// Values that depend on the Intl engine (list(), collator() order) are checked in JavaScriptCore instead
// (tests/i18n/intl_jsc.py): node bundles its own ICU and is not the reference (notes/english-option-design.md §2.8.1).
import { pathToFileURL, fileURLToPath } from 'node:url';
import fs from 'node:fs';
import path from 'node:path';

// vendor/ui/js has no package.json, so node would read core/i18n.js as CommonJS: test a copy under a
// {"type": "module"} directory (same relative path, so the CHECK report's stack parsing sees …/js/core/i18n.js).
// The copy lives in $WIN11SHELL_TEST_TMP or ROOT/.rt/i18n-test (gitignored) and is removed at exit.
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.join(here, '..', '..');
const tmpBase = process.env.WIN11SHELL_TEST_TMP || path.join(root, '.rt', 'i18n-test');
fs.mkdirSync(tmpBase, { recursive: true });
const tmp = fs.mkdtempSync(path.join(tmpBase, 'w11-i18n-js-'));
fs.mkdirSync(path.join(tmp, 'ui', 'js', 'core'), { recursive: true });
fs.writeFileSync(path.join(tmp, 'package.json'), '{"type": "module"}\n');
fs.copyFileSync(path.join(root, 'vendor', 'ui', 'js', 'core', 'i18n.js'), path.join(tmp, 'ui', 'js', 'core', 'i18n.js'));
process.on('exit', () => { try { fs.rmSync(tmp, { recursive: true, force: true }); } catch { /* ignore */ } });
const I18N = pathToFileURL(path.join(tmp, 'ui', 'js', 'core', 'i18n.js')).href;

let n = 0;
const fails = [];
function eq(got, want, what) {
  n++;
  const g = JSON.stringify(got), w = JSON.stringify(want);
  if (g !== w) fails.push(`${what}: got ${g}, want ${w}`);
}

// the rule makeT used before E0a (core/i18n.js, 2026-10-27; G's 3b898e6 has it too)
const OLD = (p) => Number(p?.n ?? p?.count ?? 0);
const pickForm = (v, n) => ((n === 0 && v.zero !== undefined) ? v.zero : (n === 1 ? (v.one ?? v.other) : (v.other ?? v.one)));

// ---------------------------------------------------------------- normal mode (no CHECK)
globalThis.location = { search: '' };
const m = await import(I18N + '?mode=plain');
const { countOf, makeT, list, collator, setLang, lang, locale, i18nReport } = m;

// countOf equals the old rule for every numeric count (and for no count at all)
const NUMS = [0, 1, 2, 3, 5, 10, 21, 100, 999, 1000, 1001, 1e6, -1, -2, 0.5, 1.5, 1.0000001, 0.9999999, -0, NaN,
  Infinity, -Infinity, Number.MAX_SAFE_INTEGER, Number.MIN_VALUE];
for (let i = -1000; i <= 1000; i++) NUMS.push(i);
let seed = 7;
const rnd = () => { seed = (seed * 16807) % 2147483647; return seed / 2147483647; };
for (let i = 0; i < 2000; i++) NUMS.push(Math.round((rnd() - 0.5) * 1e7) / (rnd() < 0.5 ? 1 : 100));
const OTHERS = [undefined, null, 'x', '1.000', '1', '7', 3, 1, 0];
const PL = { zero: 'Z', one: 'O', other: 'X' }, PL2 = { one: 'O', other: 'X' };
let same = 0;
for (const v of NUMS) {
  for (const p of [{ n: v }, { count: v }, { n: v, count: 1 }, { n: v, count: 'x' }, { n: null, count: v }, { n: undefined, count: v }]) {
    const a = OLD(p), b = countOf(p);
    if (!(Object.is(a, b) || (Number.isNaN(a) && Number.isNaN(b)))) fails.push(`countOf(${JSON.stringify(p)}) = ${b}, old ${a}`);
    if (pickForm(PL, a) !== pickForm(PL, b) || pickForm(PL2, a) !== pickForm(PL2, b)) fails.push(`form differs for ${JSON.stringify(p)}`);
    same++;
  }
}
for (const o of OTHERS) for (const p of [{}, { n: o }, { count: o }]) {
  if (p.n !== undefined && typeof p.n !== 'number' && p.n !== null) continue;          // strings: below
  if (p.count !== undefined && typeof p.count !== 'number' && p.count !== null) continue;
  eq(pickForm(PL, countOf(p)), pickForm(PL, OLD(p)), `form of ${JSON.stringify(p)}`);
}
eq(countOf(undefined), 0, 'countOf(undefined): no params = 0 (zero form) as before');
eq(countOf({}), 0, 'countOf({})');
eq(pickForm(PL, countOf({ n: '1.000', count: 1000 })), 'X', '{n:"1.000", count:1000} → other');
eq(pickForm(PL, countOf({ n: '1.000' })), 'X', '{n:"1.000"} → other (was one: Number("1.000") === 1)');
eq(pickForm(PL, countOf({ n: '1' })), 'O', '{n:"1"} → one');
eq(pickForm(PL, countOf({ n: '0' })), 'Z', '{n:"0"} → zero');
eq(pickForm(PL, countOf({ n: '2 vensters' })), 'X', '{n:"2 vensters"} → other');
eq(pickForm(PL, countOf({ n: '1,000' })), 'X', 'en "1,000" → other (NaN before and after)');
eq(pickForm(PL, countOf({ count: '12' })), 'X', '{count:"12"} → other');
eq(countOf({ n: 'x', count: 1 }), 1, 'a numeric count wins over a formatted n');
eq(countOf({ n: 5, count: 1 }), 5, 'n first when both are numbers (as before)');

// makeT: output unchanged for ordinary use
const t = makeT({
  nl: { hi: 'Hallo {name}', files: { one: '1 bestand', other: '{n} bestanden' }, objs: { zero: 'Geen', one: '1 object', other: '{c} objecten' },
    fn: (p) => `fn ${p.x ?? '-'}`, only: 'alleen nl', amp: 'A & B {v}' },
  en: { hi: 'Hello {name}', files: { one: '1 file', other: '{n} files' }, objs: { zero: 'None', one: '1 object', other: '{c} objects' },
    fn: (p) => `fn-en ${p.x ?? '-'}` },
});
eq(t('hi', { name: 'Robin' }), 'Hallo Robin', 'placeholder');
eq(t('hi'), 'Hallo {name}', 'no params: placeholder stays');
eq(t('hi', { name: null }), 'Hallo {name}', 'null param: placeholder stays');
eq(t('files', { n: 1 }), '1 bestand', 'plural one');
eq(t('files', { n: 3 }), '3 bestanden', 'plural other');
eq(t('files', { n: '1.000', count: 1000 }), '1.000 bestanden', 'formatted n + count');
eq(t('objs', { n: 0, c: '0' }), 'Geen', 'zero');
eq(t('objs'), 'Geen', 'no params: zero as before');
eq(t('fn', { x: 2 }), 'fn 2', 'function value');
eq(t('nokey', { a: 1 }), 'nokey', 'missing key → key');
eq(t('amp', { v: '$&' }), 'A & B $&', 'replacement text is literal');
setLang('en');
eq([lang(), locale()], ['en', 'en-GB'], 'setLang');
eq(t('hi', { name: 'R' }), 'Hello R', 'en');
eq(t('only'), 'alleen nl', 'en falls back to nl');
eq(t('files', { n: 1 }), '1 file', 'en one');
eq(t('fn', {}), 'fn-en -', 'en function');
eq(JSON.stringify(i18nReport), '{"missing":[],"unfilled":[],"plural":[]}', 'no report without ?i18ncheck=1');

// list / collator: shape only here (values: intl_jsc.py)
eq(list([]), '', 'list([])');
eq(list(['A']), 'A', 'list one');
eq(typeof list(['A', 'B', 'C']), 'string', 'list three');
const c1 = collator({ numeric: true, sensitivity: 'base' });
eq(c1 === collator({ numeric: true, sensitivity: 'base' }), true, 'collator cached per locale + options');
eq(c1 === collator(), false, 'different options → different collator');
eq(c1.resolvedOptions().numeric, true, 'collator keeps its options');
eq(['file10', 'file2'].sort(c1.compare), ['file2', 'file10'], 'numeric collation');
setLang('nl');
eq(c1 === collator({ numeric: true, sensitivity: 'base' }), false, 'a language switch gives a new collator');
eq(collator().resolvedOptions().locale.startsWith('nl'), true, 'nl collator');

// ---------------------------------------------------------------- CHECK mode (?i18ncheck=1)
globalThis.location = { search: '?x=1&i18ncheck=1' };
const warned = [];
console.warn = (...a) => warned.push(a.join(' '));
globalThis.window = globalThis;
const c = await import(I18N + '?mode=check');
const tc = c.makeT({ nl: { greet: 'Hoi {name}', items: { one: '1 item', other: '{n} items' }, both: 'b' }, en: { greet: 'Hi {name}', both: 'b' } });
const MOD = 'core_i18n.test.mjs';
eq(tc('greet', { name: 'A' }), 'Hoi A', 'check mode: output unchanged');
tc('greet', {});
tc('greet');
tc('nokey');
tc('items', { n: '1.000' });
tc('items', { n: '2', count: 2 });
c.setLang('en');
eq(tc('items', { n: 1 }), '1 item', 'check mode: en falls back to nl');
const rep = JSON.parse(JSON.stringify(globalThis.__i18n));
eq(rep.missing.sort(), [`${MOD}:en:items`, `${MOD}:nl:nokey`].sort(), 'missing keys');
eq(rep.unfilled, [`${MOD}:greet:{name}`], 'unfilled placeholder (once per entry)');
eq(rep.plural, [`${MOD}:items`], 'plural with a string n and no numeric count');
eq(globalThis.__i18n === c.i18nReport, true, 'window.__i18n is the report');
eq(m.i18nReport.missing.size, 0, 'the plain instance reports nothing');
eq(warned.length, 4, 'console.warn once per entry (2 missing, 1 unfilled, 1 plural)');

if (fails.length) {
  console.log(fails.slice(0, 40).join('\n'));
  console.log(`FAILED ${fails.length} of ${n + same}`);
  process.exit(1);
}
console.log(`ok ${n + same}`);
