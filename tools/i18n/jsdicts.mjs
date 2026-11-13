// Static extractor of the UI's language data (notes/english-option-design.md §3.1), for tools/i18n/check.py.
//
//   node tools/i18n/jsdicts.mjs UI_JS_DIR [FILE.js …]     → JSON on stdout
//   import { extract } from './jsdicts.mjs'                (E0b's scan_ui.mjs: the same parse, plus dictionary ranges)
//
// Never imports a UI module (they need a DOM); it tokenizes the source (./tok.mjs) and reads object literals:
//   dicts   every makeT({nl: {…}, en: {…}}) call: per language {key: value}
//   pairs   every inline {nl: '…', en: '…'} object (tr() data, theme names …) and every settings-registry
//           P(id, parent, icon, nlTitle, enTitle, nlDesc, enDesc, nlKeys, enKeys) call
//   tables  every `const X_NL = {…}` / `const X_EN = {…}` pair (TYPES_*, CAT_*, …): their keys
// A value is {s, line} (a string or a template: s holds its static text, ${…} for an expression, tpl: true),
// {plural: {zero?, one?, other?: value}, line}, {fn: [strings], line} (an arrow function or method), or
// {expr: 'source', strings: [...], line} (anything else).
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { tokenize } from './tok.mjs';

const OPEN = new Set(['(', '[', '{']);
const CLOSE = new Set([')', ']', '}']);
const PLURAL_FORMS = new Set(['zero', 'one', 'two', 'few', 'many', 'other']);

/** extract('apps/x.js', source) → {dicts, pairs, tables, errors, ranges}; ranges: [start, end] source offsets of the
 * makeT dictionaries (for scans that skip dictionary text). */
export function extract(rel, src) {
  const all = tokenize(src);
  const T = all.filter((t) => !t.inTpl);
  const out = { dicts: [], pairs: [], tables: [], errors: [] };
  const err = (line, msg) => out.errors.push({ file: rel, line, msg });

  /** index of the token that closes the bracket opened at T[i] */
  function match(i) {
    let d = 0;
    for (let j = i; j < T.length; j++) {
      if (T[j].type !== 'punct') continue;
      if (OPEN.has(T[j].value)) d++;
      else if (CLOSE.has(T[j].value)) { d--; if (d === 0) return j; }
    }
    return T.length - 1;
  }
  /** end (exclusive) of the expression starting at T[i]: the first , ) ] } at depth 0 */
  function exprEnd(i) {
    for (let j = i; j < T.length; j++) {
      const t = T[j];
      if (t.type !== 'punct') continue;
      if (OPEN.has(t.value)) { j = match(j); continue; }
      if (t.value === ',' || CLOSE.has(t.value)) return j;
    }
    return T.length;
  }
  const stringsIn = (a, b) => all.filter((t) => (t.type === 'str' || t.type === 'tpl') && t.start >= T[a].start && t.end <= (T[b - 1]?.end ?? 0)).map((t) => t.value);

  function value(a, b) {
    if (b <= a) return { expr: '', strings: [], line: T[a]?.line ?? 0 };
    const t0 = T[a];
    if (b - a === 1 && (t0.type === 'str' || t0.type === 'tpl')) return t0.type === 'tpl' ? { s: t0.value, tpl: true, line: t0.line } : { s: t0.value, line: t0.line };
    if (t0.value === '{' && match(a) === b - 1) {
      const o = object(a);
      const keys = Object.keys(o.entries);
      if (keys.length && keys.every((k) => PLURAL_FORMS.has(k))) return { plural: o.entries, line: t0.line };
      return { object: o.entries, line: t0.line };
    }
    let arrow = false;
    for (let j = a; j < b; j++) {
      if (T[j].type === 'punct' && OPEN.has(T[j].value)) { const m = match(j); if (m < b && T[m + 1]?.value === '=>' && j === a) { arrow = true; break; } j = m; continue; }
      if (T[j].value === '=>') { arrow = true; break; }
    }
    if (arrow || t0.value === 'function' || (t0.value === 'async' && (T[a + 1]?.value === 'function' || T[a + 1]?.value === '('))) {
      return { fn: stringsIn(a, b), line: t0.line };
    }
    return { expr: src.slice(t0.start, T[b - 1].end).slice(0, 200), strings: stringsIn(a, b), line: t0.line };
  }

  /** parse the object literal opened at T[i] → {entries: {key: value}, end} */
  function object(i) {
    const end = match(i);
    const entries = {};
    let j = i + 1;
    while (j < end) {
      const t = T[j];
      if (t.value === ',') { j++; continue; }
      if (t.value === '...') { const e = exprEnd(j + 1); entries[`...${src.slice(T[j + 1].start, T[e - 1].end)}`] = { spread: true, line: t.line }; j = e; continue; }
      let key = null, k = j;
      if (t.value === '[') { k = match(j); key = `[${src.slice(T[j + 1].start, T[k - 1].end)}]`; }
      else if ((t.value === 'get' || t.value === 'set' || t.value === 'async') && T[j + 1] && T[j + 1].value !== ':' && T[j + 1].value !== '(' && T[j + 1].value !== ',') { k = j + 1; key = T[k].value; }
      else key = t.value;
      const nx = T[k + 1];
      if (nx?.value === ':') {
        const e = exprEnd(k + 2);
        entries[key] = { ...value(k + 2, e), keyLine: t.line };
        j = e;
      } else if (nx?.value === '(') {                                          // method
        const pe = match(k + 1);
        const be = T[pe + 1]?.value === '{' ? match(pe + 1) : pe;
        entries[key] = { fn: stringsIn(k, be + 1), line: t.line, keyLine: t.line };
        j = be + 1;
      } else {                                                                  // shorthand
        entries[key] = { expr: key, strings: [], line: t.line, keyLine: t.line };
        j = k + 1;
      }
    }
    return { entries, end };
  }

  const inDict = [];      // [start, end] token ranges of makeT dicts
  for (let k = 0; k < T.length; k++) {
    const t = T[k];
    if (t.type === 'id' && t.value === 'makeT' && T[k + 1]?.value === '(' && T[k - 1]?.value !== '.' && T[k - 1]?.value !== 'function') {
      if (T[k + 2]?.value !== '{') {
        if (T[k + 2]?.value !== ')') err(t.line, 'makeT() without an object literal: the checker cannot read it');
        continue;
      }
      const o = object(k + 2);
      inDict.push([k + 2, o.end]);
      const langs = {}, other = [];
      for (const [lk, v] of Object.entries(o.entries)) {
        if (v.object) langs[lk] = v.object;
        else other.push(lk);
        if (!v.object) err(v.line, `makeT: '${lk}' is not an object literal`);
      }
      out.dicts.push({ file: rel, line: t.line, langs, other });
      continue;
    }
    if (t.type === 'id' && t.value === 'P' && T[k + 1]?.value === '(' && T[k - 1]?.value !== '.' && T[k - 1]?.value !== 'const') {
      const close = match(k + 1);
      const args = [];
      for (let j = k + 2; j < close;) { const e = exprEnd(j); args.push(value(j, e)); j = T[e]?.value === ',' ? e + 1 : e; }
      if (args.length >= 5) {
        const pick = (v) => (v.s !== undefined ? v.s : v.expr === 'null' ? null : v.expr ?? null);
        // id: P()'s first argument, the page id (the glossary's context rows name P(<id>).title|description)
        const id = args[0]?.s ?? null;
        out.pairs.push({ file: rel, line: t.line, kind: 'P', what: 'title', id, nl: pick(args[3]), en: pick(args[4]) });
        if (args.length >= 7) out.pairs.push({ file: rel, line: t.line, kind: 'P', what: 'description', id, nl: pick(args[5]), en: pick(args[6]) });
        if (args.length >= 9) out.pairs.push({ file: rel, line: t.line, kind: 'P', what: 'keywords', id, nl: args[7].strings ?? null, en: args[8].strings ?? null });
      }
      continue;
    }
    if (t.type === 'id' && /^[A-Z][A-Z0-9_]*_(NL|EN)$/.test(t.value) && T[k + 1]?.value === '=' && T[k + 2]?.value === '{' && ['const', 'let', 'var'].includes(T[k - 1]?.value)) {
      const o = object(k + 2);
      out.tables.push({ file: rel, line: t.line, name: t.value.slice(0, -3), lang: t.value.slice(-2).toLowerCase(), keys: Object.keys(o.entries) });
    }
  }
  // inline {nl: '…', en: '…'} pairs outside makeT dicts
  const seen = new Set();
  for (let k = 0; k < T.length; k++) {
    const t = T[k];
    if (!((t.type === 'id' || t.type === 'str') && t.value === 'nl' && T[k + 1]?.value === ':' && T[k - 1] && (T[k - 1].value === '{' || T[k - 1].value === ','))) continue;
    // enclosing {
    let d = 0, open = -1;
    for (let j = k - 1; j >= 0; j--) {
      const q = T[j];
      if (q.type !== 'punct') continue;
      if (CLOSE.has(q.value)) d++;
      else if (OPEN.has(q.value)) { if (d === 0) { open = q.value === '{' ? j : -1; break; } d--; }
    }
    if (open < 0 || seen.has(open) || inDict.some(([a, b]) => open >= a && open <= b)) continue;
    seen.add(open);
    const o = object(open);
    const nl = o.entries.nl, en = o.entries.en;
    if (!en || (nl.s === undefined && en.s === undefined)) continue;     // not a text pair ({nl: [...]} keyword lists …)
    out.pairs.push({ file: rel, line: t.line, kind: 'obj', nl: nl.s ?? null, en: en.s ?? null, nlExpr: nl.s === undefined ? (nl.expr ?? 'fn') : undefined, enExpr: en.s === undefined ? (en.expr ?? 'fn') : undefined });
  }
  out.ranges = inDict.map(([a, b]) => [T[a].start, T[b].end]);
  return out;
}

function main() {
  const dir = process.argv[2];
  if (!dir) { process.stderr.write('usage: node jsdicts.mjs UI_JS_DIR [FILE.js …]\n'); process.exit(2); }
  let files = process.argv.slice(3);
  if (!files.length) {
    (function walk(d) {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const p = path.join(d, e.name);
        if (e.isDirectory()) walk(p); else if (p.endsWith('.js') || p.endsWith('.mjs')) files.push(p);
      }
    })(dir);
  }
  files = files.sort();
  const res = { dicts: [], pairs: [], tables: [], errors: [], files: 0 };
  for (const f of files) {
    const rel = path.relative(dir, f).split(path.sep).join('/');
    let r;
    try { r = extract(rel, fs.readFileSync(f, 'utf8')); } catch (e) { res.errors.push({ file: rel, line: 0, msg: `extract failed: ${e && e.message}` }); continue; }
    res.files++;
    for (const k of ['dicts', 'pairs', 'tables', 'errors']) res[k].push(...r[k]);
  }
  process.stdout.write(JSON.stringify(res));
}

// run as a script (node jsdicts.mjs …), not when imported (import { extract } from './jsdicts.mjs')
if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
