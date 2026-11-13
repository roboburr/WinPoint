// Inventory script of 2026-10-27, copied unchanged from notes/i18n-tools/analyze.mjs (notes/english-option-design.md §3);
// it reads its tokens from ./tok.mjs. Usage: node tools/i18n/analyze.mjs ui OUT.json
// Inventory of user-visible strings in ui/js: which are inside makeT nl/en dicts, inline {nl,en} pairs,
// language ternaries, and which Dutch strings are hard-coded outside any i18n structure.
import fs from 'node:fs';
import path from 'node:path';
import { tokenize } from './tok.mjs';

const ROOT = process.argv[2];
const OUT = process.argv[3];
const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p); else if (p.endsWith('.js')) files.push(p);
  }
})(path.join(ROOT, 'js'));
files.sort();

// English dictionary
const ENGDICT = new Set();
for (const f of ['/usr/share/dict/british-english', '/usr/share/dict/american-english']) {
  try { for (const w of fs.readFileSync(f, 'utf8').split('\n')) if (w && !w.includes("'")) ENGDICT.add(w.toLowerCase()); } catch { }
}
const WORD_RE = /[A-Za-zÀ-ÿ]+(?:[’'][a-z]+)?/g;
const words = (s) => (s.match(WORD_RE) || []).map((w) => w.toLowerCase());

// ---------- pass 1: tokenize + structure ----------
const all = []; // string records
const dicts = []; // makeT records
for (const file of files) {
  const rel = path.relative(ROOT, file);
  const src = fs.readFileSync(file, 'utf8');
  let toks;
  try { toks = tokenize(src); } catch (e) { console.error('tokenize failed', rel, e); continue; }
  const stack = [{ kind: 'root', lang: null, label: null }];
  const top = () => stack[stack.length - 1];
  for (let k = 0; k < toks.length; k++) {
    const t = toks[k], p1 = toks[k - 1], p2 = toks[k - 2], nx = toks[k + 1];
    if (t.type === 'punct' && (t.value === '{' || t.value === '(' || t.value === '[')) {
      const fr = { kind: t.value, lang: top().lang, label: null, dict: top().dict, keys: null, line: t.line, varName: top().varName };
      if (p1?.value === '=' && p2?.type === 'id') fr.varName = p2.value;
      if (t.value === '{' && p1?.value === '(' && p2?.type === 'id' && p2.value === 'makeT') {
        const rec = { file: rel, line: t.line, nl: new Map(), en: new Map(), other: [] };
        dicts.push(rec); fr.label = 'makeTroot'; fr.dict = rec;
      } else if (t.value === '{' && p1?.value === ':' && p2?.type === 'id' && (p2.value === 'nl' || p2.value === 'en')) {
        fr.lang = p2.value;
        if (top().label === 'makeTroot') { fr.label = 'dict'; fr.keys = top().dict[p2.value]; }
      } else if (t.value === '{' && top().label === 'makeTroot' && p1?.value === ':' && p2?.type === 'id') {
        top().dict.other.push(p2.value);
      }
      stack.push(fr);
      continue;
    }
    if (t.type === 'punct' && (t.value === '}' || t.value === ')' || t.value === ']')) {
      if (stack.length > 1) stack.pop();
      continue;
    }
    // dict key
    if ((t.type === 'id' || t.type === 'str' || t.type === 'num') && nx?.value === ':' && (p1?.value === '{' || p1?.value === ',') && top().label === 'dict') {
      top().keys.set(t.value, t.line);
    }
    if (t.type !== 'str' && t.type !== 'tpl') continue;
    const isKey = nx?.value === ':' && (p1?.value === '{' || p1?.value === ',');
    if (isKey) continue;
    // import specifiers
    if (p1?.type === 'id' && (p1.value === 'from' || p1.value === 'import')) continue;
    if (p1?.value === '(' && p2?.type === 'id' && p2.value === 'import') continue;
    let lang = top().lang;
    let ctx = lang ? (top().dict || stack.some((f) => f.label === 'dict') ? 'dict' : 'inlineObj') : null;
    if (p1?.value === ':' && p2?.type === 'id' && (p2.value === 'nl' || p2.value === 'en')) { lang = p2.value; ctx = 'inlineObj'; }
    // if inside a dict frame anywhere in the stack, ctx=dict
    if (stack.some((f) => f.label === 'dict')) ctx = 'dict';
    // calling context: nearest id before '(' of current paren frame
    let callee = null;
    for (let j = stack.length - 1; j >= 0; j--) if (stack[j].kind === '(') { break; }
    // find the callee by scanning back for the opening paren of top frame
    if (top().kind === '(' || top().kind === '{') {
      let depth = 0;
      for (let j = k - 1; j >= 0 && j > k - 400; j--) {
        const q = toks[j];
        if (q.type !== 'punct') continue;
        if (q.value === ')' || q.value === ']' || q.value === '}') depth++;
        else if (q.value === '(' || q.value === '[' || q.value === '{') {
          if (depth === 0) { if (q.value === '(' && toks[j - 1]?.type === 'id') callee = toks[j - 1].value; break; }
          depth--;
        }
      }
    }
    const lineText = src.split('\n')[t.line - 1] || '';
    all.push({ file: rel, line: t.line, type: t.type, value: t.value, lang, ctx, callee, inTpl: !!t.inTpl, lineText, prev: p1?.value, prev2: p2?.value, next: nx?.value, varName: top().varName, k, toks });
  }
}

// ---------- pass 2: vocab from dicts ----------
const nlVocab = new Map(), enVocab = new Map();
for (const r of all) {
  if (r.lang === 'nl') for (const w of words(r.value)) nlVocab.set(w, (nlVocab.get(w) || 0) + 1);
  if (r.lang === 'en') for (const w of words(r.value)) enVocab.set(w, (enVocab.get(w) || 0) + 1);
}
const SEED_NL = 'de het een van niet je jouw uw op voor met naar die dat dit deze kan kun kunt wordt worden zijn ook geen bij aan uit als dan nog wel maar hier er waar is wil wilt zoeken openen sluiten nieuw nieuwe instellingen verwijderen alles geen bezig gelukt mislukt bestand bestanden map mappen schijf venster vensters taakbalk bureaublad'.split(' ');
const DUTCH = new Set();
for (const [w, c] of nlVocab) {
  if (w.length < 2) continue;
  if (enVocab.has(w) && (enVocab.get(w) >= c / 3)) continue; // shared word (brand, unit)
  if (ENGDICT.has(w) && !SEED_NL.includes(w)) continue;
  DUTCH.add(w);
}
for (const w of SEED_NL) DUTCH.add(w);
for (const w of ['is', 'in', 'of', 'die', 'van', 'op', 'er', 'aan', 'had', 'met', 'dan']) { /* ambiguous: keep only strong */ }
['is', 'in'].forEach((w) => DUTCH.delete(w));
const ENGLISH = new Set();
for (const [w] of enVocab) if (!nlVocab.has(w) && w.length >= 2) ENGLISH.add(w);
for (const w of 'the a an of to and or is are was not you your this that with for from on in at by be can will no yes open close new settings delete file files folder'.split(' ')) ENGLISH.add(w);
['de', 'van', 'die', 'op', 'er'].forEach((w) => ENGLISH.delete(w));

const DIACRITIC = /[ëïéèóöüäâêîôû]/i;
function classify(s) {
  const ws = words(s.replace(/\$\{…\}/g, ' '));
  let nl = 0, en = 0;
  for (const w of ws) { if (DUTCH.has(w)) nl++; else if (ENGLISH.has(w)) en++; }
  if (DIACRITIC.test(s) && !/[A-Za-z]+é\b/.test(s.replace(/café|résumé/gi, ''))) nl += 1;
  if (/[ëï]/.test(s)) nl += 1;
  if (nl === 0 && en === 0) return 'neutral';
  if (nl > en) return 'nl';
  if (en > nl) return 'en';
  return 'mixed';
}
function codeLike(s) {
  if (/^[a-zA-Z_$][\w$]*(\.[\w$-]+)+$/.test(s)) return true; // dotted ids: alles.showHidden
  if (/^[a-z]+[A-Z][A-Za-z0-9]*$/.test(s)) return true; // camelCase keys

  if (!/\s/.test(s) && /^[a-z0-9_.:\/#@?=&%*+-]*$/.test(s)) return true; // ids, classes, paths
  if (/^[.#\[]?[a-z][\w-]*(\s+[.#>]?[a-z][\w-]*)*$/.test(s) && /[-_.]/.test(s)) return true;
  if (/^(M|m)[\d .,-]+/.test(s)) return true; // svg path
  if (/^(rgba?|hsla?|var|calc|url|translate|scale|rotate)\(/.test(s)) return true;
  if (/^[\d.]+(px|%|em|rem|ms|s|deg)?$/.test(s)) return true;
  if (/^[a-z-]+:[^ ]/.test(s) && !/\s/.test(s)) return true;
  if (/^\/|^\.\/|^\.\.\//.test(s)) return true;
  if (/^[A-Z_][A-Z0-9_]+$/.test(s)) return true;
  return false;
}
for (const r of all) r.cls = classify(r.value);
const clsOf = (tk) => (tk && (tk.type === 'str' || tk.type === 'tpl')) ? classify(tk.value) : null;
for (const r of all) {
  const T = r.toks, k = r.k;
  const a1 = T[k - 1], a2 = T[k - 2], b1 = T[k + 1], b2 = T[k + 2];
  r.pair = null;
  if (a1?.value === ':' && T[k - 3]?.value === '?' && ['en', 'neutral'].includes(clsOf(a2))) r.pair = 'ternary';
  else if (b1?.value === ':' && ['en', 'neutral'].includes(clsOf(b2)) && T[k - 1]?.value === '?') r.pair = 'ternary';
  else if (b1?.value === ',' && clsOf(b2) === 'en') r.pair = 'positional';
  else if (a1?.value === ',' && clsOf(a2) === 'nl' && r.cls === 'en') r.pair = 'positional';
  if (!r.pair && r.varName && /(^|_)NL$|Nl$|_nl$/.test(r.varName)) r.pair = 'map:' + r.varName;
}
for (const r of all) {
  r.code = codeLike(r.value);
  r.console = /console\.(log|warn|error|info|debug)|new Error\(|throw /.test(r.lineText);
  r.mock = r.file.startsWith('js/mock/');
  r.ternary = /\blang\(\)|\b(en|nl|isEn|EN|NL)\s*\?|===\s*'en'|!==\s*'en'|===\s*'nl'/.test(r.lineText);
}

// ---------- areas ----------
function area(f) {
  if (f.startsWith('js/shell/')) {
    const b = path.basename(f, '.js');
    const map = { taskbar: 'Taskbar', tray: 'Taskbar tray', startmenu: 'Start', search: 'Search', quicksettings: 'Quick settings', notifcenter: 'Notifications/calendar', toasts: 'Toasts', taskview: 'Task View', winx: 'Win+X', power: 'Power menu', clipboard: 'Clipboard history', osd: 'OSD', shell: 'Shell glue', flyouthost: 'Shell glue' };
    return 'shell/' + (map[b] || b);
  }
  if (f.startsWith('js/apps/settings/pages/')) return 'Instellingen pages';
  if (f.startsWith('js/apps/settings/')) return 'Instellingen core';
  if (f.startsWith('js/apps/taskmgr/')) return 'Taakbeheer';
  if (f.startsWith('js/apps/explorer/')) return 'Verkenner';
  if (f.startsWith('js/apps/alles/')) return 'Alles';
  if (f.startsWith('js/apps/disks/')) return 'Schijven';
  if (f.startsWith('js/apps/')) return 'App frame';
  if (f.startsWith('js/desktop/')) return 'Desktop';
  if (f.startsWith('js/core/')) return 'Core (components/bridge/format)';
  if (f.startsWith('js/mock/')) return 'Mock (preview/tests)';
  if (f.startsWith('js/preview/')) return 'Preview WM';
  if (f === 'js/gallery.js') return 'Gallery (dev)';
  if (f === 'js/main.js') return 'Router';
  return f;
}

const out = { dicts: [], files: {}, areas: {}, samples: {} };
for (const d of dicts) {
  const nlK = [...d.nl.keys()], enK = new Set(d.en.keys());
  const missing = nlK.filter((k) => !enK.has(k));
  const extra = [...enK].filter((k) => !d.nl.has(k));
  out.dicts.push({ file: d.file, line: d.line, nl: nlK.length, en: enK.size, missingEn: missing, extraEn: extra, otherLangs: d.other });
}
for (const r of all) {
  const F = (out.files[r.file] ||= { area: area(r.file), dictNl: 0, dictEn: 0, dictNlTpl: 0, inlineNl: 0, inlineEn: 0, ternaryNl: 0, hardNl: 0, hardNlMock: 0, hardMixed: 0, hardEnVisible: 0, enInDictLooksNl: 0, consoleNl: 0, lcSingleNl: 0 });
  if (r.ctx === 'dict' && r.lang === 'nl') { F.dictNl++; continue; }
  if (r.ctx === 'dict' && r.lang === 'en') { F.dictEn++; if (r.cls === 'nl') { F.enInDictLooksNl++; (out.samples.enLooksNl ||= []).push(r); } continue; }
  if (r.ctx === 'inlineObj') { if (r.lang === 'nl') F.inlineNl++; else F.inlineEn++; continue; }
  if (r.code) continue;
  if (r.cls === 'nl' || r.cls === 'mixed') {
    if (r.console) { F.consoleNl++; continue; }
    if (r.pair) { F.pairNl = (F.pairNl || 0) + 1; (out.samples.pair ||= []).push(r); continue; }
    if (r.ternary) { F.ternaryNl++; (out.samples.ternary ||= []).push(r); continue; }
    const lcSingle = !/\s/.test(r.value) && r.value === r.value.toLowerCase();
    if (lcSingle) { F.lcSingleNl++; (out.samples.lcSingle ||= []).push(r); continue; }
    if (r.cls === 'mixed') { F.hardMixed++; (out.samples.mixed ||= []).push(r); continue; }
    if (r.mock) { F.hardNlMock++; (out.samples.hardMock ||= []).push(r); }
    else { F.hardNl++; (out.samples.hard ||= []).push(r); }
  }
}
for (const k in out.samples) out.samples[k] = out.samples[k].map(({ toks, ...r }) => r);
fs.writeFileSync(OUT, JSON.stringify(out, (k, v) => (v instanceof Map ? Object.fromEntries(v) : v instanceof Set ? [...v] : v), 1));
console.log('files', files.length, 'strings', all.length, 'dicts', dicts.length, 'DUTCH vocab', DUTCH.size, 'ENGLISH vocab', ENGLISH.size);
fs.writeFileSync(OUT.replace('.json', '.all.json'), JSON.stringify(all.map(({ lineText, toks, ...r }) => r)));
fs.writeFileSync(OUT.replace('.json', '.vocab.txt'), [...DUTCH].sort().join('\n'));
