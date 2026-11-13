#!/usr/bin/env python3
"""UI layer of the language checker (GE1a, notes/claude/round1/ge1a.md): the rules for ``vendor/ui/js``, moved out of
``tools/i18n/check.py`` (which now only dispatches, F39), plus the dictionary-level Dutch snapshot.

    python3 -B tools/i18n/check.py                      # all layers (this one included)
    python3 -B tools/i18n/check_ui.py [--warnings]      # the UI rules alone
    python3 -B tools/i18n/check_ui.py --dict-snapshot OUT.tsv [--ui-js DIR]
    python3 -B tools/i18n/check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv [--changes FILE] [--ui-js DIR]

Rules (``tools/i18n/jsdicts.mjs`` over ``vendor/ui/js``; needs node, skipped with a warning without it): every
``makeT`` dictionary has nl and en with the same keys, the same placeholders and the plural forms makeT knows;
inline ``{nl, en}`` pairs and ``P()`` registry pairs are complete; ``X_NL``/``X_EN`` tables have no en key that nl
lacks; the glossary rule (an error since the round-3 lock-in; context rows in ``tools/i18n/glossary-context.tsv``,
see check.py), article and plural-noun as before.

Dictionary snapshot (F25): one row per Dutch text of the UI's language data, ``file<TAB>kind<TAB>key<TAB>type<TAB>nl``
(TSV escapes ``\\\\ \\n \\t \\r``):

* ``dict``  a ``makeT`` key (``key``, plural forms as ``key.one``/``key.other``/``key.zero``, nested objects as
  ``key.sub``);
* ``P``     a settings-registry ``P()`` call: ``P(<id>).title|description|keywords`` (keywords as a JSON list);
* ``pair``  an inline ``{nl, en}`` object: ``pair#<n>`` (n-th in the file); the diff compares these as a multiset per
  file, so adding or moving a pair is not a change;
* ``table`` an ``X_NL`` table entry: ``X_NL.<key>``.

``type`` is ``s`` (a string), ``tpl`` (a template literal's static text with ``${…}``), ``fn`` (a function's string
literals, JSON), ``expr`` (other source) or ``null``.

``--dict-diff`` lists every row whose nl changed or that disappeared, and fails unless each one is named in the
BASELINE-CHANGES file (default ``tests/i18n/BASELINE-CHANGES.ui``) as ``STEP<TAB>key:<file>:<key><TAB><new nl or
->`` with a reason in a fourth field. New rows (new keys) are listed and need no line; a line may still name one (a
new Dutch text worth a reason), and then its text must match. A line that matches no changed, removed or added row
is stale (a failure too).
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402
import paths  # noqa: E402

if "check" not in sys.modules and "__main__" in sys.modules and \
        Path(getattr(sys.modules["__main__"], "__file__", "") or "").name == "check.py":
    sys.modules["check"] = sys.modules["__main__"]
import check  # noqa: E402

ROOT = paths.ROOT
UI_PREFIX = paths.UI_REL + "/"
MAKET_FORMS = {"zero", "one", "other"}
DEFAULT_CHANGES = paths.tests_dir() / "BASELINE-CHANGES.ui"

# rules of this layer that are warnings until the named step (merged into check.DEFERRED by the dispatcher)
DEFERRED = {
    "article": "round 3",
    "table-gap": "round 3 (optional)",
}


# ------------------------------------------------------------------------------------------------------------------
# extraction (node)
# ------------------------------------------------------------------------------------------------------------------

def find_node() -> str | None:
    return corpus.find_node()


def run_jsdicts(root: Path, node: str, files: list[str] | None = None, ui_js: Path | None = None) -> dict:
    cmd = [node, str(HERE / "jsdicts.mjs"), str(ui_js or paths.ui_js(root)), *(files or [])]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=300)
    if r.returncode != 0:
        raise RuntimeError(f"jsdicts.mjs failed ({r.returncode}): {r.stderr[-2000:]}")
    return json.loads(r.stdout)


# jsdicts.mjs's extract() for dicts and pairs, plus what it does not return: the P() ids and the X_NL table values
SNAP_JS = r"""
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
const [here, dir] = process.argv.slice(1);
const { extract } = await import(pathToFileURL(path.join(here, 'jsdicts.mjs')).href);
const { tokenize } = await import(pathToFileURL(path.join(here, 'tok.mjs')).href);
const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p); else if (p.endsWith('.js') || p.endsWith('.mjs')) files.push(p);
  }
})(dir);
files.sort();
const out = [];
for (const f of files) {
  const rel = path.relative(dir, f).split(path.sep).join('/');
  const src = fs.readFileSync(f, 'utf8');
  const r = extract(rel, src);
  const T = tokenize(src).filter((t) => !t.inTpl);
  const pIds = {};
  const tables = [];
  for (let k = 0; k < T.length; k++) {
    const t = T[k];
    if (t.type === 'id' && t.value === 'P' && T[k + 1]?.value === '(' && T[k - 1]?.value !== '.' && T[k - 1]?.value !== 'const'
        && T[k + 2]?.type === 'str') pIds[t.line] = T[k + 2].value;
    if (t.type === 'id' && /^[A-Z][A-Z0-9_]*_NL$/.test(t.value) && T[k + 1]?.value === '=' && T[k + 2]?.value === '{'
        && ['const', 'let', 'var'].includes(T[k - 1]?.value)) {
      const entries = {};
      let d = 0, j = k + 3, key = null, vals = [];
      for (; j < T.length; j++) {
        const q = T[j];
        if (q.type === 'punct' && ['(', '[', '{'].includes(q.value)) { d++; if (key !== null) vals.push(q); continue; }
        if (q.type === 'punct' && [')', ']', '}'].includes(q.value)) {
          if (d === 0) break;
          d--; if (key !== null) vals.push(q); continue;
        }
        if (d === 0 && q.value === ',' ) { if (key !== null) entries[key] = vals; key = null; vals = []; continue; }
        if (d === 0 && key === null && T[j + 1]?.value === ':') { key = q.value; j++; continue; }
        if (key !== null) vals.push(q);
      }
      if (key !== null) entries[key] = vals;
      const vs = {};
      for (const [kk, v] of Object.entries(entries)) {
        vs[kk] = v.length === 1 && v[0].type === 'str' ? { s: v[0].value }
          : { expr: v.length ? src.slice(v[0].start, v[v.length - 1].end).slice(0, 200) : '' };
      }
      tables.push({ name: t.value, line: t.line, entries: vs });
    }
  }
  out.push({ file: rel, dicts: r.dicts, pairs: r.pairs, pIds, tables, errors: r.errors });
}
process.stdout.write(JSON.stringify(out));
"""


def snapshot_data(node: str, ui_js: Path) -> list[dict]:
    r = subprocess.run([node, "--input-type=module", "-e", SNAP_JS, "--", str(HERE), str(ui_js)],
                       capture_output=True, text=True, timeout=300)
    if r.returncode != 0:
        raise RuntimeError(f"snapshot extraction failed ({r.returncode}): {r.stderr[-2000:]}")
    return json.loads(r.stdout)


def _val(v: dict | None) -> tuple[str, str]:
    """(type, text) of one extracted value."""
    if v is None:
        return "null", ""
    if "s" in v:
        return ("tpl" if v.get("tpl") else "s"), v["s"]
    if "fn" in v:
        return "fn", json.dumps(v["fn"], ensure_ascii=False)
    if "expr" in v:
        return ("null", "") if v["expr"] == "null" else ("expr", v["expr"])
    return "expr", json.dumps(v, ensure_ascii=False, sort_keys=True)


def snapshot_rows(data: list[dict]) -> list[tuple[str, str, str, str, str]]:
    """[(file, kind, key, type, nl)] in a stable order (file, then source order)."""
    rows: list[tuple[str, str, str, str, str]] = []
    for f in data:
        rel = f["file"]
        for d in f["dicts"]:
            nl = d.get("langs", {}).get("nl", {})

            def walk(prefix: str, obj: dict) -> None:
                for k, v in obj.items():
                    key = f"{prefix}{k}"
                    if "plural" in v:
                        for form, x in v["plural"].items():
                            rows.append((rel, "dict", f"{key}.{form}", *_val(x)))
                    elif "object" in v:
                        walk(key + ".", v["object"])
                    else:
                        rows.append((rel, "dict", key, *_val(v)))
            walk("", nl)
        n = 0
        for p in f["pairs"]:
            if p["kind"] == "P":
                pid = f["pIds"].get(str(p["line"]), f"line{p['line']}")
                val = p["nl"]
                if p["what"] == "keywords":
                    rows.append((rel, "P", f"P({pid}).keywords", *(("null", "") if val is None else
                                                                 ("json", json.dumps(val, ensure_ascii=False)))))
                else:
                    rows.append((rel, "P", f"P({pid}).{p['what']}", *(("null", "") if val is None else ("s", val))))
            else:
                n += 1
                if p.get("nlExpr"):
                    rows.append((rel, "pair", f"pair#{n}", "expr", p["nlExpr"]))
                else:
                    rows.append((rel, "pair", f"pair#{n}", *(("null", "") if p["nl"] is None else ("s", p["nl"]))))
        for t in f["tables"]:
            for k, v in t["entries"].items():
                rows.append((rel, "table", f"{t['name']}.{k}", *_val(v)))
    return rows


def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace("\n", "\\n").replace("\t", "\\t").replace("\r", "\\r")


def unesc(s: str) -> str:
    return re.sub(r"\\(.)", lambda m: {"n": "\n", "t": "\t", "r": "\r", "\\": "\\"}.get(m.group(1), m.group(0)), s)


SNAP_HEADER = ("# Dictionary-level Dutch snapshot of vendor/ui/js (tools/i18n/check_ui.py --dict-snapshot; F25,"
               " notes/claude/round1/ge1a.md).\n"
               "# file<TAB>kind<TAB>key<TAB>type<TAB>nl  (kinds dict, P, pair, table; TSV escapes \\\\ \\n \\t \\r)\n")


def write_snapshot(rows, out: Path, source: str) -> None:
    with open(out, "w", encoding="utf-8", newline="\n") as fh:
        fh.write(SNAP_HEADER)
        fh.write(f"# source: {source}; {len(rows)} rows\n")
        for r in rows:
            fh.write("\t".join((r[0], r[1], r[2], r[3], esc(r[4]))) + "\n")


def read_snapshot(path: Path) -> list[tuple[str, str, str, str, str]]:
    rows = []
    for line in path.read_text(encoding="utf-8").split("\n"):
        if not line or line.startswith("#"):
            continue
        f, kind, key, typ, nl = line.split("\t")
        rows.append((f, kind, key, typ, unesc(nl)))
    return rows


def read_changes(path: Path) -> dict[str, tuple[str, str, int]]:
    """key:<file>:<key> rows of a BASELINE-CHANGES file → {file:key: (new nl, step, line)}"""
    out: dict[str, tuple[str, str, int]] = {}
    if not path.exists():
        return out
    for no, line in enumerate(path.read_text(encoding="utf-8").split("\n"), 1):
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) >= 4 and parts[1].startswith("key:"):
            out[parts[1][4:]] = (unesc(parts[2]), parts[0], no)
    return out


def dict_diff(old_rows, new_rows) -> dict[str, list]:
    """changed / removed / added rows between two snapshots ({kind: [...]}); pairs as a multiset per file."""
    res = {"changed": [], "removed": [], "added": []}
    old = {(r[0], r[2]): r for r in old_rows if r[1] != "pair"}
    new = {(r[0], r[2]): r for r in new_rows if r[1] != "pair"}
    for k, r in old.items():
        if k not in new:
            res["removed"].append(r)
        elif (new[k][3], new[k][4]) != (r[3], r[4]):
            res["changed"].append((r, new[k]))
    for k, r in new.items():
        if k not in old:
            res["added"].append(r)
    po = collections.Counter((r[0], r[3], r[4]) for r in old_rows if r[1] == "pair")
    pn = collections.Counter((r[0], r[3], r[4]) for r in new_rows if r[1] == "pair")
    for (f, typ, nl), c in sorted((po - pn).items()):
        for _ in range(c):
            res["removed"].append((f, "pair", "pair(nl)", typ, nl))
    for (f, typ, nl), c in sorted((pn - po).items()):
        for _ in range(c):
            res["added"].append((f, "pair", "pair(nl)", typ, nl))
    return res


def check_dict_diff(snapshot: Path, changes: Path, ui_js: Path, node: str, verbose: bool = True) -> int:
    old = read_snapshot(snapshot)
    new = snapshot_rows(snapshot_data(node, ui_js))
    d = dict_diff(old, new)
    listed = read_changes(changes)
    bad = []
    used = set()
    for o, n in d["changed"]:
        k = f"{o[0]}:{o[2]}"
        want = listed.get(k)
        if want is None or want[0] != n[4]:
            bad.append(f"CHANGED  {k}: {o[4]!r} → {n[4]!r}" + ("" if want is None else f" (listed as {want[0]!r})"))
        used.add(k)
    for r in d["removed"]:
        k = f"{r[0]}:{r[2]}" if r[1] != "pair" else f"{r[0]}:pair({r[4]})"
        want = listed.get(k)
        if want is None or want[0] != "-":
            bad.append(f"REMOVED  {k}: {r[4]!r}")
        used.add(k)
    for r in d["added"]:
        if r[1] == "pair":
            continue
        k = f"{r[0]}:{r[2]}"
        want = listed.get(k)
        if want is not None:
            if want[0] != r[4]:
                bad.append(f"ADDED    {k}: {r[4]!r} (listed as {want[0]!r})")
            used.add(k)
    unused = sorted(set(listed) - used)
    if verbose:
        for o, n in d["changed"]:
            print(f"changed  {o[0]}:{o[2]}: {o[4]!r} → {n[4]!r}")
        for r in d["removed"]:
            print(f"removed  {r[0]}:{r[2]}: {r[4]!r}")
        for r in d["added"]:
            print(f"added    {r[0]}:{r[2]}: {r[4]!r}")
    for b in bad:
        print("NOT LISTED " + b)
    for k in unused:
        print(f"STALE    {changes.name} line {listed[k][2]}: key:{k} matches no changed or removed row")
    print(f"dict diff vs {snapshot.name}: {len(old)} → {len(new)} rows; {len(d['changed'])} changed, "
          f"{len(d['removed'])} removed, {len(d['added'])} added; {len(bad)} not in {changes.name}, "
          f"{len(unused)} stale")
    return 1 if bad or unused else 0


# ------------------------------------------------------------------------------------------------------------------
# rules
# ------------------------------------------------------------------------------------------------------------------

def _line_has(lines: list[str], no: int, word: str) -> set[str]:
    """glossary-ok terms in a // comment on line `no` or the line above."""
    out: set[str] = set()
    for i in (no - 2, no - 1):
        if 0 <= i < len(lines):
            m = re.search(r"glossary-ok:\s*([^*/]+)", lines[i])
            if m:
                out |= {t.strip() for t in m.group(1).split(",") if t.strip()}
    return out


def _forms(v: dict) -> dict[str, str] | None:
    """A plural value → {form: text} (string forms only)."""
    if "plural" not in v:
        return None
    return {k: x.get("s") for k, x in v["plural"].items() if isinstance(x, dict) and "s" in x}


def check_js(root: Path, rep, gloss, data: dict) -> None:
    ui = UI_PREFIX
    src_lines: dict[str, list[str]] = {}
    placeholders = check.placeholders
    ARTICLE = check.ARTICLE

    def lines_of(rel: str) -> list[str]:
        if rel not in src_lines:
            try:
                src_lines[rel] = (paths.ui_js(root) / rel).read_text(encoding="utf-8").split("\n")
            except OSError:
                src_lines[rel] = []
        return src_lines[rel]

    for e in data.get("errors", []):
        rep.add("js-extract", ui + e["file"], e.get("line", 0), e["msg"])
    n_keys = 0
    for d in data.get("dicts", []):
        f = ui + d["file"]
        langs = d.get("langs", {})
        if "nl" not in langs or "en" not in langs:
            rep.add("js-langs", f, d["line"], f"makeT dictionary without {'nl' if 'nl' not in langs else 'en'}")
            continue
        nl, en = langs["nl"], langs["en"]
        for k in sorted(set(nl) - set(en)):
            rep.add("js-parity", f, nl[k].get("keyLine", d["line"]), f"key {k!r} has no en value")
        for k in sorted(set(en) - set(nl)):
            rep.add("js-parity", f, en[k].get("keyLine", d["line"]), f"key {k!r} has no nl value")
        for k in sorted(set(nl) & set(en)):
            n_keys += 1
            a, b = nl[k], en[k]
            line = a.get("keyLine", a.get("line", d["line"]))
            for lg, v in (("nl", a), ("en", b)):
                if "plural" in v:
                    bad = sorted(set(v["plural"]) - MAKET_FORMS)
                    if bad:
                        rep.add("js-plural", f, line, f"{k} ({lg}): plural forms {bad} are not supported by makeT "
                                                      "(zero, one, other)")
                    if "other" not in v["plural"] and "one" not in v["plural"]:
                        rep.add("js-plural", f, line, f"{k} ({lg}): plural without one/other")
            fa, fb = _forms(a), _forms(b)
            pairs: list[tuple[str, str]] = []
            if fa is not None and fb is not None:
                if set(fa) != set(fb):
                    rep.add("plural-shape", f, line, f"{k}: nl forms {sorted(fa)}, en forms {sorted(fb)}")
                for form in sorted(set(fa) & set(fb)):
                    if fa[form] is not None and fb[form] is not None:
                        pairs.append((fa[form], fb[form]))
                ua = set().union(*(placeholders(x) for x in fa.values() if x)) if fa else set()
                ub = set().union(*(placeholders(x) for x in fb.values() if x)) if fb else set()
                if ua != ub:
                    rep.add("js-placeholders", f, line, f"{k}: nl has {sorted(ua)}, en has {sorted(ub)}")
            elif (fa is None) != (fb is None) and ("s" in a or "s" in b):
                rep.add("plural-shape", f, line, f"{k}: a plural object in {'nl' if fa is not None else 'en'} only")
            elif "s" in a and "s" in b:
                pairs.append((a["s"], b["s"]))
                pa, pb = placeholders(a["s"]), placeholders(b["s"])
                if pa != pb:
                    rep.add("js-placeholders", f, line, f"{k}: nl has {sorted(pa)}, en has {sorted(pb)}")
            ok = _line_has(lines_of(d["file"]), line, "glossary-ok")
            for s_nl, s_en in pairs:
                if ARTICLE.search(s_en):
                    rep.add("article", f, line, f"{k}: en uses an article before a placeholder ({s_en!r})")
                for gnl, gen in gloss.check(s_nl, s_en, ok, where=(f, k)):
                    rep.add("glossary", f, line, f"{k}: nl “{gnl}” → en should contain “{gen}”")
            if "s" in b and fb is None:
                noun = check.plural_noun_after_count(b["s"])
                if noun:
                    rep.add("plural-noun", f, line, f"{k}: en {b['s']!r}: '{noun}' after a count, but not a plural "
                                                    "object (gives '1 {noun}')")
    rep.stats["js_dicts"] = len(data.get("dicts", []))
    rep.stats["js_keys"] = n_keys
    n_pairs = 0
    for p in data.get("pairs", []):
        f = ui + p["file"]
        n_pairs += 1
        if p["kind"] == "P":
            if p["what"] == "keywords":
                if (p["nl"] is None) != (p["en"] is None):
                    rep.add("js-pair", f, p["line"], "P(): keyword list in one language only")
                continue
            if p["what"] == "title" and (not p["nl"] or not p["en"]):
                rep.add("js-pair", f, p["line"], f"P(): {p['what']} needs nl and en")
                continue
            if (p["nl"] is None) != (p["en"] is None):
                rep.add("js-pair", f, p["line"], f"P(): {p['what']} in one language only")
                continue
        else:
            if p.get("nlExpr") or p.get("enExpr"):
                rep.add("pair-expr", f, p["line"], f"{{nl, en}} pair with a computed half ({p.get('nlExpr') or p.get('enExpr')})")
                continue
            if bool(p["nl"]) != bool(p["en"]):
                rep.add("js-pair", f, p["line"], "{nl, en} pair with an empty half")
                continue
        if isinstance(p["nl"], str) and isinstance(p["en"], str):
            pn, pe = placeholders(p["nl"]), placeholders(p["en"])
            if pn != pe:
                rep.add("js-placeholders", f, p["line"], f"pair: nl has {sorted(pn)}, en has {sorted(pe)}")
            # context-row key: P(<id>).title|description for a settings-registry page, "pair" for an inline pair
            pkey = f"P({p.get('id')}).{p['what']}" if p["kind"] == "P" else "pair"
            for gnl, gen in gloss.check(p["nl"], p["en"], _line_has(lines_of(p["file"]), p["line"], "glossary-ok"),
                                        where=(f, pkey)):
                rep.add("glossary", f, p["line"], f"pair: nl “{gnl}” → en should contain “{gen}”")
    rep.stats["js_pairs"] = n_pairs
    tables: dict[tuple[str, str], dict[str, dict]] = {}
    for t in data.get("tables", []):
        tables.setdefault((t["file"], t["name"]), {})[t["lang"]] = t
    for (file, name), byl in sorted(tables.items()):
        f = ui + file
        if "nl" not in byl or "en" not in byl:
            continue
        nlk, enk = set(byl["nl"]["keys"]), set(byl["en"]["keys"])
        extra = sorted(enk - nlk)
        if extra:
            rep.add("js-table", f, byl["en"]["line"], f"{name}_EN has keys that {name}_NL lacks: {extra[:10]}")
        gap = sorted(nlk - enk)
        if gap:
            rep.add("table-gap", f, byl["en"]["line"], f"{name}_EN lacks {len(gap)} of {len(nlk)} keys "
                                                       f"(e.g. {', '.join(gap[:6])}); en shows its fallback there")
    rep.stats["js_tables"] = len(tables)


def check_layer(root: Path, rep, gloss, opts: dict | None = None) -> None:
    """The dispatcher's entry point (tools/i18n/check.py): add this layer's issues and stats to rep."""
    opts = opts or {}
    root = Path(root)
    js_data = opts.get("js_data")
    if js_data is None and opts.get("js", True) and paths.ui_js(root).exists():
        node = find_node()
        if node is None:
            rep.add("js-skipped", paths.UI_REL, 0, "node not found: the UI dictionaries were not checked",
                    level="warning")
        else:
            js_data = run_jsdicts(root, node)
    if js_data is not None:
        check_js(root, rep, gloss, js_data)
        rep.stats["js_files"] = js_data.get("files")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--ui-js", help="the UI js directory (default ROOT/vendor/ui/js)")
    ap.add_argument("--warnings", action="store_true")
    ap.add_argument("--dict-snapshot", metavar="OUT", help="write the dictionary-level nl snapshot")
    ap.add_argument("--source", default="", help="text for the snapshot's source line")
    ap.add_argument("--dict-diff", metavar="SNAPSHOT", help="compare the tree with a snapshot")
    ap.add_argument("--changes", default=str(DEFAULT_CHANGES), help="BASELINE-CHANGES file for --dict-diff")
    ap.add_argument("--quiet", action="store_true", help="--dict-diff: only problems and the summary")
    a = ap.parse_args(argv)
    ui_js = Path(a.ui_js) if a.ui_js else paths.ui_js(a.root)
    if a.dict_snapshot or a.dict_diff:
        node = find_node()
        if node is None:
            print("check_ui: node not found", file=sys.stderr)
            return 2
        if a.dict_snapshot:
            rows = snapshot_rows(snapshot_data(node, ui_js))
            write_snapshot(rows, Path(a.dict_snapshot), a.source or str(ui_js))
            print(f"wrote {a.dict_snapshot}: {len(rows)} rows")
            return 0
        return check_dict_diff(Path(a.dict_diff), Path(a.changes), ui_js, node, verbose=not a.quiet)
    return check.main(["--root", a.root, "--layers", "ui"] + (["--warnings"] if a.warnings else []))


if __name__ == "__main__":
    sys.exit(main())
