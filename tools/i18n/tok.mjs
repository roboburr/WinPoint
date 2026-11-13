// Minimal JS tokenizer: strings, template literals (with nested ${} expressions), regex, comments.
// Produces a flat token list; template literal tokens carry their static text with ${…} placeholders.
//
// Ported from the 2026-10-27 inventory (notes/i18n-tools/tok.mjs) for tools/i18n (notes/english-option-design.md §3).
// Changes: string and template escapes are decoded like JS does (\n, \t, \r, \b, \f, \v, \0, \xHH, \uXXXX, \u{…},
// line continuations), so extracted values equal the runtime strings; tokens of a template's ${…} expressions keep
// inTpl: true (as before) and a `tpl` token lists them in .exprs. No other behaviour change: analyze.mjs and
// pairs.mjs (the inventory scripts, next to this file) still work on it.
export function tokenize(src) {
  const toks = [];
  let i = 0, line = 1;
  const n = src.length;
  const isIdStart = (c) => /[A-Za-z_$À-￿]/.test(c);
  const isId = (c) => /[A-Za-z0-9_$À-￿]/.test(c);
  const KW_BEFORE_RE = new Set(['return', 'typeof', 'case', 'do', 'else', 'in', 'of', 'new', 'delete', 'void', 'throw', 'instanceof', 'yield', 'await']);
  function prevSig() { return toks.length ? toks[toks.length - 1] : null; }
  function regexAllowed() {
    const p = prevSig();
    if (!p) return true;
    if (p.type === 'num' || p.type === 'str' || p.type === 'tpl' || p.type === 'regex') return false;
    if (p.type === 'id') return KW_BEFORE_RE.has(p.value);
    if (p.type === 'punct') return !(p.value === ')' || p.value === ']' || p.value === '}');
    return true;
  }
  // one escape sequence at src[i] === '\\' → [decoded text, chars consumed]
  function escape() {
    const c = src[i + 1];
    const simple = { n: '\n', t: '\t', r: '\r', b: '\b', f: '\f', v: '\v' };
    if (c in simple) return [simple[c], 2];
    if (c === '0' && !/[0-9]/.test(src[i + 2] || '')) return ['\0', 2];
    if (c === 'x' && /^[0-9a-fA-F]{2}$/.test(src.slice(i + 2, i + 4))) return [String.fromCharCode(parseInt(src.slice(i + 2, i + 4), 16)), 4];
    if (c === 'u' && src[i + 2] === '{') {
      const close = src.indexOf('}', i + 3);
      if (close > 0 && /^[0-9a-fA-F]+$/.test(src.slice(i + 3, close))) return [String.fromCodePoint(parseInt(src.slice(i + 3, close), 16)), close + 1 - i];
    }
    if (c === 'u' && /^[0-9a-fA-F]{4}$/.test(src.slice(i + 2, i + 6))) return [String.fromCharCode(parseInt(src.slice(i + 2, i + 6), 16)), 6];
    if (c === '\r' && src[i + 2] === '\n') { line++; return ['', 3]; }
    if (c === '\n' || c === ' ' || c === ' ') { line++; return ['', 2]; }
    return [c ?? '', 2];
  }
  // returns [token, newIndex]
  function readString(q) {
    const start = i, startLine = line;
    i++;
    let val = '';
    while (i < n && src[i] !== q) {
      if (src[i] === '\\') { const [s, k] = escape(); val += s; i += k; continue; }
      if (src[i] === '\n') line++;
      val += src[i++];
    }
    i++;
    return { type: 'str', value: val, line: startLine, start, end: i };
  }
  function readTemplate() {
    const start = i, startLine = line;
    i++;
    let val = '';
    const exprs = [];
    while (i < n && src[i] !== '`') {
      if (src[i] === '\\') { const [s, k] = escape(); val += s; i += k; continue; }
      if (src[i] === '$' && src[i + 1] === '{') {
        i += 2;
        const exprStart = toks.length;
        // tokenize until matching }
        let depth = 0;
        while (i < n) {
          const before = toks.length;
          const r = step();
          if (r === 'eof') break;
          const tk = toks[toks.length - 1];
          if (toks.length === before + 1 && tk.type === 'punct') {
            if (tk.value === '{') depth++;
            else if (tk.value === '}') { if (depth === 0) { toks.pop(); break; } depth--; }
          }
        }
        const inner = toks.splice(exprStart);
        for (const t of inner) t.inTpl = true;
        exprs.push(inner);
        val += '${…}';
        continue;
      }
      if (src[i] === '\n') line++;
      val += src[i++];
    }
    i++;
    return { type: 'tpl', value: val, line: startLine, start, end: i, exprs };
  }
  function step() {
    // skip ws/comments
    while (i < n) {
      const c = src[i];
      if (c === '\n') { line++; i++; continue; }
      if (c === ' ' || c === '\t' || c === '\r') { i++; continue; }
      if (c === '/' && src[i + 1] === '/') { while (i < n && src[i] !== '\n') i++; continue; }
      if (c === '/' && src[i + 1] === '*') { i += 2; while (i < n && !(src[i] === '*' && src[i + 1] === '/')) { if (src[i] === '\n') line++; i++; } i += 2; continue; }
      break;
    }
    if (i >= n) return 'eof';
    const c = src[i];
    if (c === '"' || c === "'") { toks.push(readString(c)); return; }
    if (c === '`') {
      const tk = readTemplate();
      // nested expression tokens: push after
      toks.push(tk);
      for (const e of tk.exprs) for (const t of e) toks.push(t);
      return;
    }
    if (c === '/' && regexAllowed()) {
      const start = i; i++;
      let inClass = false;
      while (i < n) {
        if (src[i] === '\\') { i += 2; continue; }
        if (src[i] === '[') inClass = true;
        else if (src[i] === ']') inClass = false;
        else if (src[i] === '/' && !inClass) break;
        else if (src[i] === '\n') break;
        i++;
      }
      i++;
      while (i < n && /[a-z]/.test(src[i])) i++;
      toks.push({ type: 'regex', value: src.slice(start, i), line, start, end: i });
      return;
    }
    if (/[0-9]/.test(c) || (c === '.' && /[0-9]/.test(src[i + 1]))) {
      const start = i; while (i < n && /[0-9a-fA-FxXob._eEn]/.test(src[i])) i++;
      toks.push({ type: 'num', value: src.slice(start, i), line, start, end: i }); return;
    }
    if (isIdStart(c)) {
      const start = i; while (i < n && isId(src[i])) i++;
      toks.push({ type: 'id', value: src.slice(start, i), line, start, end: i }); return;
    }
    // punct: multi-char ops
    const three = src.slice(i, i + 3), two = src.slice(i, i + 2);
    if (['===', '!==', '...', '**=', '<<=', '>>=', '&&=', '||=', '??='].includes(three)) { toks.push({ type: 'punct', value: three, line, start: i, end: i + 3 }); i += 3; return; }
    if (['=>', '==', '!=', '<=', '>=', '&&', '||', '??', '?.', '++', '--', '+=', '-=', '*=', '/=', '**', '<<', '>>'].includes(two)) { toks.push({ type: 'punct', value: two, line, start: i, end: i + 2 }); i += 2; return; }
    toks.push({ type: 'punct', value: c, line, start: i, end: i + 1 }); i++;
  }
  while (step() !== 'eof') { /* loop */ }
  return toks;
}
