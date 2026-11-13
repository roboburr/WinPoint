// Ratchet scan of the UI source (H: notes/english-option-design.md §3.2, step E0b; ported to Win11 voor GNOME in
// GE0a, notes/claude/round0/ge0a.md: the UI lives in vendor/ui/js, UI_REL below).
//
//   node tools/i18n/scan_ui.mjs ROOT        → JSON [{file, line, kind, text}] on stdout (file: vendor/ui/js/…)
//
// Flags every string literal and template text in vendor/ui/js (not mock/, not gallery.js) that contains a word of
// tests/i18n/dutch-words.txt and is not part of the language data: a makeT dictionary, an inline {nl, en} object
// (also {nl: [...], en: [...]} keyword lists), a settings-registry P() call or an X_NL / X_EN table. Arguments of
// console.* calls are developer logs (D10) and module paths (import … from '…') are not flagged. Language ternaries (`en ? '…' : '…'`) are flagged:
// they may stay, but no new ones (§2.3), so the existing ones are in the allowlist.
// tools/i18n/ratchet.py compares the result with tests/i18n/allow-ui.txt (file + sha1 of the text + count).
import fs from 'node:fs';
import path from 'node:path';
import { tokenize } from './tok.mjs';
import { extract } from './jsdicts.mjs';

const OPEN = new Set(['(', '[', '{']);
const CLOSE = new Set([')', ']', '}']);
const WORD = /[A-Za-zÀ-ÖØ-öø-ÿ]+(?:['’][a-zë]+)?/g;
const PH = /\{[A-Za-z_]\w*\}|\$\{…\}|\$\{[^}]*\}/g;
/** The UI source tree, relative to the project root (H had ui/js). */
export const UI_REL = 'vendor/ui/js';

export function loadWords(root) {
  const f = path.join(root, 'tests', 'i18n', 'dutch-words.txt');
  return new Set(fs.readFileSync(f, 'utf8').split('\n').map((s) => s.trim()).filter((s) => s && !s.startsWith('#')));
}

export function dutchWords(s, words) {
  return (String(s).replace(PH, ' ').match(WORD) || []).map((w) => w.toLowerCase().replace('’', "'")).filter((w) => words.has(w));
}

/** [start, end] source ranges that hold language data or developer logs */
function excluded(rel, src, all) {
  const T = all.filter((t) => !t.inTpl);
  const out = [];
  try { out.push(...extract(rel, src).ranges); } catch { /* the checker reports extract errors */ }
  const match = (i) => {
    let d = 0;
    for (let j = i; j < T.length; j++) {
      if (T[j].type !== 'punct') continue;
      if (OPEN.has(T[j].value)) d++;
      else if (CLOSE.has(T[j].value)) { d--; if (d === 0) return j; }
    }
    return T.length - 1;
  };
  for (let k = 0; k < T.length; k++) {
    const t = T[k];
    const prev = T[k - 1];
    // P(id, parent, icon, nlTitle, enTitle, …) in the settings registry
    if (t.type === 'id' && t.value === 'P' && T[k + 1]?.value === '(' && prev?.value !== '.' && prev?.value !== 'const') {
      out.push([t.start, T[match(k + 1)].end]);
    }
    // const X_NL = {…} / X_EN tables
    if (t.type === 'id' && /^[A-Z][A-Z0-9_]*_(NL|EN)$/.test(t.value) && T[k + 1]?.value === '=' && T[k + 2]?.value === '{') {
      out.push([T[k + 2].start, T[match(k + 2)].end]);
    }
    // console.log/info/warn/error/debug(…): developer logs
    if (t.type === 'id' && t.value === 'console' && T[k + 1]?.value === '.' && T[k + 3]?.value === '(') {
      out.push([t.start, T[match(k + 3)].end]);
    }
    // {…, nl: …, en: …} objects
    if ((t.type === 'id' || t.type === 'str') && t.value === 'nl' && T[k + 1]?.value === ':' && prev && (prev.value === '{' || prev.value === ',')) {
      let d = 0, open = -1;
      for (let j = k - 1; j >= 0; j--) {
        const q = T[j];
        if (q.type !== 'punct') continue;
        if (CLOSE.has(q.value)) d++;
        else if (OPEN.has(q.value)) { if (d === 0) { open = q.value === '{' ? j : -1; break; } d--; }
      }
      if (open < 0) continue;
      const end = match(open);
      // an `en:` key at the same depth
      let depth = 0, hasEn = false;
      for (let j = open + 1; j < end; j++) {
        const q = T[j];
        if (q.type === 'punct' && OPEN.has(q.value)) depth++;
        else if (q.type === 'punct' && CLOSE.has(q.value)) depth--;
        else if (depth === 0 && (q.type === 'id' || q.type === 'str') && q.value === 'en' && T[j + 1]?.value === ':') hasEn = true;
      }
      if (hasEn) out.push([T[open].start, T[end].end]);
    }
  }
  return out;
}

export function scanFile(rel, src, words) {
  const all = tokenize(src);
  const ex = excluded(rel, src, all);
  const inEx = (t) => ex.some(([a, b]) => t.start >= a && t.end <= b);
  const hits = [];
  for (let i = 0; i < all.length; i++) {
    const t = all[i];
    if (t.type !== 'str' && t.type !== 'tpl') continue;
    const p = all[i - 1];
    if (p && (p.value === 'from' || (p.value === '(' && all[i - 2]?.value === 'import'))) continue;   // module paths
    if (!dutchWords(t.value, words).length) continue;
    if (inEx(t)) continue;
    hits.push({ file: UI_REL + '/' + rel, line: t.line, kind: t.type === 'tpl' ? 'template' : 'string', text: t.value });
  }
  return hits;
}

export function scan(root) {
  const words = loadWords(root);
  const base = path.join(root, ...UI_REL.split('/'));
  const files = [];
  (function walk(d) {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p); else if (p.endsWith('.js') || p.endsWith('.mjs')) files.push(p);
    }
  })(base);
  const out = [];
  for (const f of files.sort()) {
    const rel = path.relative(base, f).split(path.sep).join('/');
    if (rel.startsWith('mock/') || rel === 'gallery.js') continue;
    out.push(...scanFile(rel, fs.readFileSync(f, 'utf8'), words));
  }
  return out;
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  const root = process.argv[2] || process.cwd();
  process.stdout.write(JSON.stringify(scan(root)));
}
