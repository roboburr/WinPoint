#!/usr/bin/env python3
"""Converter for Rust host literals into host/i18n/<stem>.tsv rows (GE1b; plan-critique F13, round2-gate.md).

    python3 -B tools/i18n/extract_rs.py list FILE [--all]
        the file's literals with their ordinal (the site id), line, enclosing fn, GE0b class, conversion pattern and
        H's English when H's Dutch is byte-identical; without --all only user-visible ones
    python3 -B tools/i18n/extract_rs.py convert FILE --stem STEM [--lang EXPR] --site ORD=KEY[:names][@LANG] …
                                        [--en ORD=TEXT] [--note ORD=TEXT] [--plan PLAN.tsv] [--apply]
        converts the sites: prints the call-site patch and the catalogue rows; --apply writes both
    python3 -B tools/i18n/extract_rs.py add --stem STEM --key KEY --nl TEXT --en TEXT [--note TEXT] [--from PATH#ORD]
        [--apply]    a row with new Dutch text (needs a key: line in tests/i18n/BASELINE-CHANGES.rust), or with --from
        one whose nl is a 3b898e6 literal the code no longer holds as such (e.g. moved into a table)
    python3 -B tools/i18n/extract_rs.py en KEY TEXT [--apply]
        set the English of an existing row after a review (nl and the "#@" lines stay)
    python3 -B tools/i18n/extract_rs.py map KEY --from PATH#ORD [--en] [--spec '{}=name' …] [--apply]
        record that a row replaced a 3b898e6 literal you converted by hand (e.g. both halves of an inline
        `if nl {"…"} else {"…"}` pair: once plain/fmt for the nl half, once --en for the English half)

What it does per site:
* decodes the Rust literal (escapes, raw strings, line continuations) and takes nl from it byte for byte;
* for format!-strings: turns every {}, {0}, {name} into a named {placeholder} and records the map in the "#@" line
  above the row (tools/i18n/drift_rust.py rebuilds the 3b898e6 literal from it); {{ and }} become { and };
  refuses {:?}, {x:>5}, {:.2} and any other format spec, and a literal "{word}" text, for manual handling;
* en: --en, else H's row when H's nl equals this nl (placeholders compared by position; plural rows are never taken,
  they would change Dutch for n == 1: D-G1a), else empty (check_rust.py then fails until it is written);
* rewrites the call site:
    [path::]ApiError::new(code, "…")              → [path::]ApiError::tr(code, LANG, "key", crate::i18n::NO_PARAMS)
    [path::]ApiError::new(code, &format!("…", a))  → [path::]ApiError::tr(code, LANG, "key", &serde_json::json!({"a": a.to_string()}))
    format!("…", a)                                 → crate::i18n::trf(LANG, "key", &serde_json::json!({…}))
    "…".to_string() / .to_owned() / String::from("…") → crate::i18n::tr(LANG, "key");  "…".into() → crate::i18n::tr(…).into()
    .ok_or("…")                                     → .ok_or_else(|| crate::i18n::tr(LANG, "key"))
    "…" as a json! value                            → crate::i18n::tr(LANG, "key")
    any other "…"                                   → &crate::i18n::tr(LANG, "key")   (marked: check the type)
  LANG is --lang (default crate::i18n::current_language()) or the site's @LANG: e.g. self.ctx.lang(), &lang;
* refuses literals that tests/i18n/allow-host.txt lists (never translate, C9), byte/C strings and test code.
Run `cargo check` after --apply; the patch never touches anything but the converted expressions.
"""
from __future__ import annotations

import argparse
import difflib
import fnmatch
import os
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import drift_rust as d  # noqa: E402

ROOT = d.ROOT
H_DEFAULT = Path("/srv/codex-claude/claude-scratch/c2c10c4c-ae6d-4f6a-a907-342de497d54f/"
                 "931db6b9-2976-4e14-8e5a-041952887025/scratch-2026-10-25-a089b6")
ALLOW = "tests/i18n/allow-host.txt"
KEY_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[A-Za-z0-9_]+)+$")
PLAIN_EXPR = re.compile(r"[A-Za-z_][A-Za-z0-9_]*(\.[A-Za-z_][A-Za-z0-9_]*(\(\))?)*")
DEFAULT_LANG = "crate::i18n::current_language()"
NAME_NOISE = {"to_string", "display", "as_str", "to_owned", "clone", "unwrap", "as_ref", "to_string_lossy",
              "into_owned", "trim", "len", "self", "crate", "super"}


# ------------------------------------------------------------------------------------------------ allow-host

@dataclass
class Rule:
    line: int
    path: str
    selectors: list[tuple[str, str]]
    reason: str
    file: str = ALLOW


def load_allow(root: Path = ROOT) -> tuple[list[Rule], list[str]]:
    """tests/i18n/allow-host.txt plus the round-2 agents' allow-host-r2<x>.txt files."""
    rules: list[Rule] = []
    problems: list[str] = []
    main = root / ALLOW
    if not main.is_file():
        return rules, [f"{ALLOW} is missing"]
    files = [main] + sorted(main.parent.glob("allow-host-*.txt"))
    for p in files:
        rel = p.relative_to(root).as_posix()
        for number, line in enumerate(p.read_text(encoding="utf-8").split("\n"), 1):
            parsed = parse_allow_line(rel, number, line, problems)
            if parsed:
                rules.append(parsed)
    return rules, problems


def parse_allow_line(rel: str, number: int, line: str, problems: list[str]) -> Rule | None:
    if not line or line.startswith("#"):
        return None
    f = line.split("\t")
    if len(f) != 3 or not f[2].strip():
        problems.append(f"{rel}:{number}: expected path<TAB>selector<TAB>reason")
        return None
    sels = []
    for part in f[1].split("&"):
        if part == "file":
            sels.append(("file", ""))
            continue
        kind, _, value = part.partition(":")
        if kind not in ("text", "sha", "fn", "item", "macro", "call", "class", "label", "en") or not value:
            problems.append(f"{rel}:{number}: unknown selector {part!r}")
            return None
        sels.append((kind, value))
    return Rule(number, f[0], sels, f[2], rel)


# Selector kinds about catalogue texts (check_rust.py), not about Rust literals: such a rule never matches a literal.
TEXT_KINDS = ("label", "en")


def rule_matches(rule: Rule, lit: d.Lit, cls: str) -> bool:
    if not fnmatch.fnmatch(lit.path, rule.path) or any(kind in TEXT_KINDS for kind, _ in rule.selectors):
        return False
    for kind, value in rule.selectors:
        if kind == "file":
            continue
        got = {"text": lit.text, "fn": lit.fn, "item": lit.item, "macro": lit.macro, "call": lit.call,
               "class": cls}.get(kind)
        if kind == "sha":
            if not lit.sha.startswith(value):
                return False
        elif got != value:
            return False
    return True


# ------------------------------------------------------------------------------------------------ H catalogues

def h_rows(h: Path) -> dict[str, list[tuple[str, str, str]]]:
    """normalized nl → [(key, nl, en)] of H's i18n/host/*.tsv (plural forms excluded)."""
    out: dict[str, list[tuple[str, str, str]]] = {}
    base = h / "i18n" / "host"
    if not base.is_dir():
        return out
    for f in sorted(base.glob("*.tsv")):
        for line in f.read_text(encoding="utf-8").split("\n"):
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 3 or not parts[2]:
                continue
            key, nl, en = parts[0], parts[1].replace("\\n", "\n").replace("\\t", "\t"), \
                parts[2].replace("\\n", "\n").replace("\\t", "\t")
            out.setdefault(norm(nl), []).append((key, nl, en))
    return out


def norm(text: str) -> str:
    return d.PLACEHOLDER.sub("{}", text)


def h_english(rows: dict, nl: str) -> tuple[str | None, str]:
    """(en or None, note) for a G nl text."""
    cands = rows.get(norm(nl), [])
    plural = [k for k, _, _ in cands if re.search(r"\.(one|other|zero)$", k)]
    cands = [c for c in cands if not re.search(r"\.(one|other|zero)$", c[0])]
    gnames = d.PLACEHOLDER.findall(nl)
    ens = set()
    for key, hnl, hen in cands:
        hnames = d.PLACEHOLDER.findall(hnl)
        if len(hnames) != len(gnames):
            continue
        rename: dict[str, str] = {}
        ok = True
        for a, b in zip(hnames, gnames):
            if rename.setdefault(a, b) != b:
                ok = False
        if not ok:
            continue
        def sub(m):
            return "{" + rename[m.group(1)] + "}" if m.group(1) in rename else m.group(0)
        en = d.PLACEHOLDER.sub(sub, hen)
        if set(d.PLACEHOLDER.findall(en)) - set(gnames):
            continue
        ens.add((en, key))
    if len({e for e, _ in ens}) == 1:
        en, key = sorted(ens)[0]
        return en, f"H {key}"
    if len({e for e, _ in ens}) > 1:
        return None, "H has different English for this text: " + "; ".join(f"{k}: {e}" for e, k in sorted(ens))
    if plural:
        return None, "H has only plural rows for this text (" + ", ".join(plural) + "): consider trn (D-G1a)"
    return None, ""


# ------------------------------------------------------------------------------------------------ format strings

class Refused(Exception):
    pass


@dataclass
class Placeholder:
    spec: str          # as written: {}, {0}, {name}
    arg: str           # the argument it refers to: "#0", "#1" … or the name
    name: str = ""


def parse_format(text: str) -> tuple[list[tuple[str, str]], list[Placeholder]]:
    """[(kind, text)] pieces (kind "text" or "ph") and the placeholders, of a format! string's decoded text."""
    pieces: list[tuple[str, str]] = []
    phs: list[Placeholder] = []
    i, implicit, buf = 0, 0, []
    while i < len(text):
        c = text[i]
        if c == "{" and text.startswith("{{", i):
            buf.append("{")
            i += 2
        elif c == "}" and text.startswith("}}", i):
            buf.append("}")
            i += 2
        elif c == "{":
            j = text.find("}", i)
            if j < 0:
                raise Refused("unbalanced { in the format string")
            inner = text[i + 1:j]
            if ":" in inner:
                raise Refused(f"format spec {text[i:j + 1]} (debug, width or precision): convert by hand")
            if inner == "":
                arg = f"#{implicit}"
                implicit += 1
            elif inner.isdigit():
                arg = f"#{int(inner)}"
            elif re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", inner):
                arg = inner
            else:
                raise Refused(f"placeholder {text[i:j + 1]} is not plain")
            if buf:
                pieces.append(("text", "".join(buf)))
                buf = []
            phs.append(Placeholder(text[i:j + 1], arg))
            pieces.append(("ph", str(len(phs) - 1)))
            i = j + 1
        elif c == "}":
            raise Refused("unbalanced } in the format string")
        else:
            buf.append(c)
            i += 1
    if buf:
        pieces.append(("text", "".join(buf)))
    for kind, t in pieces:
        if kind == "text" and d.PLACEHOLDER.search(t):
            raise Refused("the text contains a literal {word} (from {{word}}): convert by hand")
    return pieces, phs


def derive_name(expr: str) -> str:
    e = expr.strip().lstrip("&*").strip()
    e = re.sub(r"\([^()]*\)", "()", e)
    parts = [p for p in re.split(r"[^A-Za-z0-9_]+", e) if p and p not in NAME_NOISE and not p[0].isdigit()]
    return parts[-1] if parts else "x"


# ------------------------------------------------------------------------------------------------ token helpers

def enclosing_open(toks: list[d.Tok], i: int) -> int | None:
    depth = 0
    for k in range(i - 1, -1, -1):
        t = toks[k]
        if t.kind != "punct":
            continue
        if t.text in d.CLOSE:
            depth += 1
        elif t.text in d.OPEN:
            if depth == 0:
                return k
            depth -= 1
    return None


def matching_close(toks: list[d.Tok], k: int) -> int:
    depth = 0
    for j in range(k, len(toks)):
        t = toks[j]
        if t.kind != "punct":
            continue
        if t.text in d.OPEN:
            depth += 1
        elif t.text in d.CLOSE:
            depth -= 1
            if depth == 0:
                return j
    raise Refused("unbalanced brackets")


def split_args(toks: list[d.Tok], open_idx: int, close_idx: int) -> list[tuple[int, int]]:
    """Token ranges [a, b) of the top-level comma-separated arguments inside toks[open_idx]…toks[close_idx]."""
    out, depth, start = [], 0, open_idx + 1
    for j in range(open_idx + 1, close_idx):
        t = toks[j]
        if t.kind == "punct" and t.text in d.OPEN:
            depth += 1
        elif t.kind == "punct" and t.text in d.CLOSE:
            depth -= 1
        elif depth == 0 and t.kind == "punct" and t.text == ",":
            out.append((start, j))
            start = j + 1
    if start < close_idx:
        out.append((start, close_idx))
    return out


def src_of(fl: d.FileLits, a: int, b: int) -> str:
    """Source text of tokens [a, b)."""
    return fl.src[fl.toks[a].start:fl.toks[b - 1].end]


# ------------------------------------------------------------------------------------------------ site analysis

@dataclass
class Site:
    lit: d.Lit
    key: str
    lang: str
    names: list[str] = field(default_factory=list)
    en: str | None = None
    note: str = ""
    # results
    pattern: str = ""
    span: tuple[int, int] = (0, 0)
    replacement: str = ""
    nl: str = ""
    kind: str = "plain"
    specs: list[tuple[str, str]] = field(default_factory=list)
    review: str = ""
    en_note: str = ""
    frozen_ord: int | None = None
    context: str = ""


def analyse(fl: d.FileLits, site: Site) -> None:
    toks = fl.toks
    i = site.lit.tok
    lit = site.lit
    if lit.flavor not in ("str", "raw"):
        raise Refused(f"a {lit.flavor} literal is not text")
    if "\r" in lit.text:
        raise Refused("the text contains a carriage return")
    expr_start = i                   # first token of the expression that yields the text
    expr_end = i + 1                 # one past its last token
    params = "crate::i18n::NO_PARAMS"
    fmt = False
    o = enclosing_open(toks, i)
    if o is not None and toks[o].text == "(" and o >= 2 and toks[o - 1].text == "!" and toks[o - 2].text == "format" \
            and o + 1 == i:
        fmt = True
        close = matching_close(toks, o)
        args = split_args(toks, o, close)
        pieces, phs = parse_format(lit.text)
        positional: list[str] = []
        named: dict[str, str] = {}
        for a, b in args[1:]:
            if b - a >= 3 and toks[a].kind == "ident" and toks[a + 1].text == "=" and toks[a + 1].kind == "punct":
                named[toks[a].text] = src_of(fl, a + 2, b)
            else:
                positional.append(src_of(fl, a, b))
        exprs: dict[str, str] = {}
        order: list[str] = []
        for ph in phs:
            if ph.arg.startswith("#"):
                idx = int(ph.arg[1:])
                if idx >= len(positional):
                    raise Refused(f"{ph.spec} has no argument")
                exprs[ph.arg] = positional[idx]
            else:
                exprs[ph.arg] = named.get(ph.arg, ph.arg)
            if ph.arg not in order:
                order.append(ph.arg)
        names: dict[str, str] = {}
        used: set[str] = set()
        for k, arg in enumerate(order):
            if k < len(site.names) and site.names[k]:
                name = site.names[k]
            elif not arg.startswith("#"):
                name = arg
            else:
                name = derive_name(exprs[arg])
            base, n = name, 2
            while name in used:
                name, n = f"{base}{n}", n + 1
            if not re.fullmatch(r"[A-Za-z0-9_]+", name):
                raise Refused(f"placeholder name {name!r} is not [A-Za-z0-9_]+")
            used.add(name)
            names[arg] = name
        nl_parts = []
        for kind, t in pieces:
            nl_parts.append(t if kind == "text" else "{" + names[phs[int(t)].arg] + "}")
        site.nl = "".join(nl_parts)
        site.kind = "fmt"
        site.specs = [(ph.spec, names[ph.arg]) for ph in phs]
        if order:
            items = []
            for arg in order:
                e = exprs[arg].strip()
                e = e if PLAIN_EXPR.fullmatch(e) else f"({e})"
                items.append(f'"{names[arg]}": {e}.to_string()')
            params = "&serde_json::json!({" + ", ".join(items) + "})"
        expr_start, expr_end = o - 2, close + 1
    else:
        site.nl = lit.text
        site.kind = "plain"
    lang = site.lang
    key = site.key
    # ApiError::new(code, <expr>) with <expr> = "…" or &format!(…)
    s = expr_start - 1
    if s >= 0 and toks[s].text == "&" and fmt:
        s -= 1
    ap = enclosing_open(toks, expr_start)
    if (s >= 0 and toks[s].text == "," and ap is not None and toks[ap].text == "(" and ap >= 3
            and toks[ap - 1].text == "new" and toks[ap - 2].text == "::" and toks[ap - 3].text == "ApiError"):
        ac = matching_close(toks, ap)
        args = split_args(toks, ap, ac)
        if len(args) == 2 and args[1][0] <= expr_start and args[1][1] == expr_end:
            start = ap - 3
            while start >= 2 and toks[start - 1].text == "::" and toks[start - 2].kind == "ident":
                start -= 2
            path = fl.src[toks[start].start:toks[ap - 3].end]
            code = src_of(fl, args[0][0], args[0][1])
            site.pattern = "ApiError::new"
            site.span = (toks[start].start, toks[ac].end)
            site.replacement = f'{path}::tr({code}, {lang}, "{key}", {params})'
            site.context = f"{lit.fn or '?'}(): ApiError {code}"
            return
    if fmt:
        site.pattern = "format!"
        site.span = (toks[expr_start].start, toks[expr_end - 1].end)
        site.replacement = f'crate::i18n::trf({lang}, "{key}", {params})'
        site.context = f"{lit.fn or '?'}()"
        return
    nxt = toks[i + 1] if i + 1 < len(toks) else None
    nxt2 = toks[i + 2] if i + 2 < len(toks) else None
    nxt3 = toks[i + 3] if i + 3 < len(toks) else None
    prev = toks[i - 1] if i >= 1 else None
    call = f'crate::i18n::tr({lang}, "{key}")'
    site.context = f"{lit.fn or '?'}()"
    if nxt and nxt.text == "." and nxt2 and nxt2.text in ("to_string", "to_owned", "into") and nxt3 \
            and nxt3.text == "(" and i + 4 < len(toks) and toks[i + 4].text == ")":
        site.pattern = f".{nxt2.text}()"
        site.span = (toks[i].start, toks[i + 4].end)
        site.replacement = call + (".into()" if nxt2.text == "into" else "")
        return
    if prev and prev.text == "(" and i >= 3 and toks[i - 2].text == "from" and toks[i - 3].text == "::" \
            and toks[i - 4].text == "String" and nxt and nxt.text == ")":
        site.pattern = "String::from"
        site.span = (toks[i - 4].start, nxt.end)
        site.replacement = call
        return
    if prev and prev.text == "(" and i >= 3 and toks[i - 2].text == "ok_or" and toks[i - 3].text == "." \
            and nxt and nxt.text == ")":
        site.pattern = ".ok_or"
        site.span = (toks[i - 2].start, nxt.end)
        site.replacement = f"ok_or_else(|| {call})"
        return
    if prev and prev.text == ":" and lit.macro == "json!":
        site.pattern = "json! value"
        site.span = (toks[i].start, toks[i].end)
        site.replacement = call
        return
    site.pattern = "bare literal"
    site.span = (toks[i].start, toks[i].end)
    site.replacement = "&" + call
    site.review = "a &str context: check the type (cargo check), the lifetime and whether a String fits better"


# ------------------------------------------------------------------------------------------------ catalogue edits

def frozen_ordinals(path: str, lits: list[d.Lit], root: Path = ROOT) -> tuple[dict[int, int], dict[int, str]]:
    """current literal ordinal → 3b898e6 ordinal (aligned), and 3b898e6 ordinal → class."""
    frozen = d.load_frozen(root).get(path, [])
    rows, _ = d.load_catalogues(root)
    used = {m.ordinal for r in rows for m in r.maps if m.path == path and m.ordinal}
    changes, _ = d.load_baseline(root)
    used |= {c.ordinal for c in changes if c.path == path and c.ordinal}
    a = [f.sha for f in frozen]
    b = [lit.sha for lit in lits]
    out: dict[int, int] = {}
    sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
    for tag, i1, i2, j1, j2 in sm.get_opcodes():
        if tag == "equal":
            for k in range(i2 - i1):
                out[lits[j1 + k].ordinal] = frozen[i1 + k].ordinal
    taken = set(out.values()) | used
    for lit in lits:
        if lit.ordinal in out:
            continue
        cands = [f for f in frozen if f.sha == lit.sha and f.ordinal not in taken]
        if cands:
            best = min(cands, key=lambda f: abs(f.line - lit.line))
            out[lit.ordinal] = best.ordinal
            taken.add(best.ordinal)
    # an ordinal already used by an earlier conversion must not be reused
    for cur, fo in list(out.items()):
        if fo in used:
            alt = [f for f in frozen if f.sha == next(l.sha for l in lits if l.ordinal == cur)
                   and f.ordinal not in taken]
            if alt:
                out[cur] = alt[0].ordinal
                taken.add(alt[0].ordinal)
            else:
                del out[cur]
    return out, {f.ordinal: f.cls for f in frozen}


def section_insert(text: str, path: str, block: str) -> str:
    header = f"# --- {path}"
    lines = text.split("\n")
    if text.endswith("\n"):
        lines = lines[:-1]
    if header not in lines:
        return "\n".join(lines) + f"\n\n{header}\n" + block + "\n"
    at = lines.index(header) + 1
    while at < len(lines) and not lines[at].startswith("# --- "):
        at += 1
    while at > 0 and lines[at - 1] == "":
        at -= 1
    return "\n".join(lines[:at] + block.split("\n") + lines[at:]) + "\n"


def add_mapping_above(text: str, key: str, mapping_line: str) -> str:
    lines = text.split("\n")
    for k, line in enumerate(lines):
        if line.split("\t", 1)[0] == key and not line.startswith("#"):
            return "\n".join(lines[:k] + [mapping_line] + lines[k:])
    raise KeyError(key)


def existing_rows(text: str) -> dict[str, tuple[str, str]]:
    out = {}
    for line in text.split("\n"):
        if line and not line.startswith("#"):
            f = line.split("\t")
            if len(f) >= 2:
                out[f[0]] = (d.tsv_unescape(f[1]), d.tsv_unescape(f[2]) if len(f) > 2 else "")
    return out


# ------------------------------------------------------------------------------------------------ commands

def resolve(file: str) -> tuple[str, Path]:
    p = Path(file)
    if not p.is_absolute():
        p = (Path.cwd() / p) if (Path.cwd() / p).exists() else ROOT / p
    p = p.resolve()
    rel = p.relative_to(ROOT.resolve()).as_posix()
    if not rel.startswith(d.SRC + "/") or not rel.endswith(".rs"):
        raise SystemExit(f"{file}: not a Rust file under {d.SRC}")
    return rel, p


def file_lits(rel: str) -> d.FileLits:
    files = d.tree_files()
    scanned, tests = d.scan_tree({k: v for k, v in files.items() if k.endswith(".rs")})
    short = rel[len(d.SRC) + 1:]
    if short in tests:
        raise SystemExit(f"{rel} is test code: nothing to convert")
    return scanned[short]


def new_classes(rel: str, lits: list[d.Lit], fmap: dict[int, int]) -> dict[int, str]:
    """GE0b's class, on the working tree, of the file's literals that are not 3b898e6 literals (the ratchet's
    classification): so a literal added since then is not offered or converted blindly as "new"."""
    if all(lit.ordinal in fmap for lit in lits):
        return {}
    return {n.ordinal: n.cls for n in d.new_literals() if n.path == rel}


LANG_NAMES = ("ctx", "lang", "language")


def lang_in_scope(fl: d.FileLits, lit: d.Lit) -> str | None:
    """A language expression the enclosing fn has in scope (review m10), e.g. "self.ctx.lang()", or None.

    Looks from the fn's `fn name` to the literal for `self.ctx`, `ctx`, `lang` or `language`."""
    toks = fl.toks
    start = None
    for k in range(lit.tok - 1, 0, -1):
        if toks[k - 1].text == "fn" and toks[k].kind == "ident" and toks[k].text == lit.fn:
            start = k
            break
    if start is None:
        return None
    seen: set[str] = set()
    # a method (`self` among the parameters) of a type whose methods use self.ctx anywhere in the file
    params_end = next((k for k in range(start, lit.tok) if toks[k].text == "{"), lit.tok)
    if any(toks[k].text == "self" for k in range(start, params_end)) and any(
            toks[k].text == "self" and toks[k + 1].text == "." and toks[k + 2].text == "ctx"
            for k in range(len(toks) - 2)):
        seen.add("self.ctx")
    for k in range(start, lit.tok):
        t = toks[k]
        if t.kind != "ident":
            continue
        if t.text == "self" and k + 2 < lit.tok and toks[k + 1].text == "." and toks[k + 2].text == "ctx":
            seen.add("self.ctx")
        elif t.text in LANG_NAMES and not (k >= 1 and toks[k - 1].text in (".", "::")):
            seen.add(t.text)
    for name, expr in (("self.ctx", "self.ctx.lang()"), ("ctx", "ctx.lang()"), ("lang", "&lang"),
                       ("language", "&language")):
        if name in seen:
            return expr
    return None


def cmd_list(a) -> int:
    rel, _ = resolve(a.file)
    fl = file_lits(rel)
    rules, _ = load_allow()
    fmap, classes = frozen_ordinals(rel, fl.lits)
    now_classes = new_classes(rel, fl.lits, fmap)
    hrows = h_rows(Path(a.h))
    catalogue_keys = {r.key for r in d.load_catalogues()[0]}
    catalogue_keys |= {re.sub(r"\.(one|other|zero)$", "", k) for k in catalogue_keys}
    shown = 0
    for lit in fl.lits:
        fo = fmap.get(lit.ordinal)
        cls = classes.get(fo, "new") if fo else "new:" + now_classes.get(lit.ordinal, "?")
        allow = next((r for r in rules if rule_matches(r, lit, cls.removeprefix("new:"))), None)
        if not a.all and (allow or cls.removeprefix("new:") in ("technical", "log") or lit.text in catalogue_keys):
            continue
        site = Site(lit, "stem.key", "LANG")
        try:
            analyse(fl, site)
            pattern = site.pattern + (" (review)" if site.review else "")
            en, note = h_english(hrows, site.nl)
        except Refused as e:
            pattern, en, note = f"REFUSED: {e}", None, ""
        flag = f"keep ({allow.reason})" if allow else pattern
        scope = lang_in_scope(fl, lit) if not allow else None
        print(f"{lit.ordinal}\tL{lit.line}\t{lit.fn or '-'}\t{cls}\t{flag}\t{d.esc(lit.text)}"
              + (f"\ten: {d.esc(en)}" if en else "") + (f"\t[{note}]" if note and not en else "")
              + (f"\t@{scope}" if scope else ""))
        shown += 1
    print(f"# {shown} of {len(fl.lits)} literals shown ({rel})", file=sys.stderr)
    return 0


def parse_site(spec: str, default_lang: str) -> tuple[str, str, list[str], str]:
    m = re.fullmatch(r"(L?\d+)=([^:@]+)(?::([^@]*))?(?:@(.+))?", spec)
    if not m:
        raise SystemExit(f"--site {spec!r}: want ORD=KEY[:name,name][@LANG] (ORD: an ordinal, or L<line>)")
    names = [n.strip() for n in (m.group(3) or "").split(",") if n.strip()]
    return m.group(1), m.group(2), names, m.group(4) or default_lang


def pick_literal(lits: list[d.Lit], sel: str, classes_of) -> d.Lit | str:
    """The literal an ORD or L<line> selector names; L<line> must name exactly one non-technical literal."""
    if sel.startswith("L"):
        line = int(sel[1:])
        here = [lit for lit in lits if lit.line == line and classes_of(lit) not in ("technical", "log")]
        if len(here) != 1:
            return f"line {line} has {len(here)} user-visible literals: use the ordinal from `list`"
        return here[0]
    hit = next((lit for lit in lits if lit.ordinal == int(sel)), None)
    return hit if hit else f"no literal #{sel}"


def cmd_convert(a) -> int:
    rel, src_path = resolve(a.file)
    fl = file_lits(rel)
    by_ord = {lit.ordinal: lit for lit in fl.lits}
    rules, _ = load_allow()
    fmap, classes = frozen_ordinals(rel, fl.lits)
    hrows = h_rows(Path(a.h))
    specs: list[tuple[str, str, list[str], str]] = [parse_site(s, a.lang) for s in a.site or []]
    en_over = dict((k, v) for k, v in (x.split("=", 1) for x in a.en or []))
    notes = dict((k, v) for k, v in (x.split("=", 1) for x in a.note or []))
    if a.plan:
        text = sys.stdin.read() if a.plan == "-" else Path(a.plan).read_text(encoding="utf-8")
        for line in text.split("\n"):
            if not line or line.startswith("#"):
                continue
            f = line.split("\t") + [""] * 6
            ordn = f[0]
            specs.append((ordn, f[1], [n for n in f[2].split(",") if n], f[3] or a.lang))
            if f[4]:
                en_over[ordn] = f[4]
            if f[5]:
                notes[ordn] = f[5]
    stem_file = ROOT / d.CATALOGUES / f"{a.stem}.tsv"
    if not stem_file.is_file():
        raise SystemExit(f"{stem_file.relative_to(ROOT)} does not exist (stems are fixed: round2-gate.md)")
    tsv = stem_file.read_text(encoding="utf-8")
    rows = existing_rows(tsv)
    sites: list[Site] = []
    refused = 0
    now_classes: dict[int, str] | None = None

    def class_of(lit: d.Lit) -> str:
        nonlocal now_classes
        fo = fmap.get(lit.ordinal)
        if fo:
            return classes.get(fo, "new")
        if now_classes is None:
            now_classes = new_classes(rel, fl.lits, fmap)
        return now_classes.get(lit.ordinal, "new")

    for sel, key, names, lang in specs:
        lit = pick_literal(fl.lits, sel, class_of)
        if isinstance(lit, str):
            print(f"error: {rel}: {lit}", file=sys.stderr)
            refused += 1
            continue
        ordn = lit.ordinal
        if not KEY_RE.match(key) or key.split(".")[0] != a.stem:
            print(f"error: #{ordn}: key {key!r} must look like {a.stem}.name", file=sys.stderr)
            refused += 1
            continue
        fo = fmap.get(ordn)
        cls = class_of(lit)
        if cls in ("technical", "log") and not a.force:
            where = "at 3b898e6" if fo else "on the working tree (a literal added since 3b898e6)"
            print(f"refused: #{ordn} (line {lit.line}) {lit.text!r} is {cls} {where} (GE0b's class); "
                  "check the ordinal with `list`, or pass --force when it really is user text", file=sys.stderr)
            refused += 1
            continue
        if a.expect and lit.text != a.expect.get(sel, lit.text):
            print(f"refused: #{ordn} is {lit.text!r}, not the expected {a.expect[sel]!r}", file=sys.stderr)
            refused += 1
            continue
        allow = next((r for r in rules if rule_matches(r, lit, cls)), None)
        if allow:
            print(f"refused: #{ordn} {lit.text!r}: never translated ({allow.file}:{allow.line}: {allow.reason})",
                  file=sys.stderr)
            refused += 1
            continue
        site = Site(lit, key, lang, names, en_over.get(sel), notes.get(sel, ""))
        site.frozen_ord = fo
        try:
            analyse(fl, site)
        except Refused as e:
            print(f"refused: #{ordn} (line {lit.line}) {lit.text!r}: {e}", file=sys.stderr)
            refused += 1
            continue
        if site.en is None:
            site.en, site.en_note = h_english(hrows, site.nl)
        if lang == DEFAULT_LANG:
            scope = lang_in_scope(fl, lit)
            if scope:
                print(f"warning: #{ordn} (line {lit.line}): no @LANG, so the site reads {DEFAULT_LANG}, the "
                      f"settings file of the process; {lit.fn}() has {scope.split('.lang')[0].lstrip('&')} in "
                      f"scope: pass @{scope} unless the process language is meant (a service test with its own "
                      "config dir would read the wrong file)", file=sys.stderr)
        sites.append(site)
    # overlapping spans (two literals of one format! call) are an error
    spans = sorted(s.span for s in sites)
    for (a1, b1), (a2, b2) in zip(spans, spans[1:]):
        if a2 < b1:
            raise SystemExit("error: two sites overlap (one expression): convert them by hand")
    # source patch
    new_src = fl.src
    for s in sorted(sites, key=lambda s: s.span[0], reverse=True):
        new_src = new_src[:s.span[0]] + s.replacement + new_src[s.span[1]:]
    # catalogue rows
    blocks: dict[str, list[str]] = {}            # key → [comment, #@ lines …, row] of rows new in this run
    new_tsv = tsv
    for s in sites:
        ordinal = str(s.frozen_ord) if s.frozen_ord else "new"
        mline = f"#@ {rel} {ordinal} {s.lit.sha[:d.SHA_PREFIX]} {s.kind}" + "".join(
            f" {spec}={name}" for spec, name in s.specs)
        if s.frozen_ord is None:
            print(f"warning: #{s.lit.ordinal} is not a 3b898e6 literal; the row is new Dutch text and needs a "
                  f"key:{s.key} line in {d.BASELINE}", file=sys.stderr)
            mline = "#@ new"
        if s.key in blocks:
            if rows[s.key][0] != s.nl:
                raise SystemExit(f"error: {s.key} is used twice in this run with different nl")
            blocks[s.key].insert(-1, mline)
            continue
        if s.key in rows:
            if rows[s.key][0] != s.nl:
                raise SystemExit(f"error: {s.key} exists with different nl: {rows[s.key][0]!r} != {s.nl!r}")
            if s.en and rows[s.key][1] and rows[s.key][1] != s.en:
                print(f"warning: {s.key} keeps its en {rows[s.key][1]!r} (not {s.en!r})", file=sys.stderr)
            new_tsv = add_mapping_above(new_tsv, s.key, mline)
            continue
        comment = f"# {s.context}" + (f": {s.note}" if s.note else "")
        row = f"{s.key}\t{d.tsv_escape(s.nl)}\t{d.tsv_escape(s.en or '')}"
        rows[s.key] = (s.nl, s.en or "")
        blocks[s.key] = [comment, mline, row]
    if blocks:
        new_tsv = section_insert(new_tsv, rel, "\n".join("\n".join(b) for b in blocks.values()))
    # report
    for s in sites:
        print(f"# #{s.lit.ordinal} L{s.lit.line} {s.pattern} -> {s.key}"
              + (f"  en: {s.en}" if s.en else "  en: (EMPTY: write it)")
              + (f"  [{s.en_note}]" if s.en_note else "") + (f"  REVIEW: {s.review}" if s.review else ""))
    sys.stdout.writelines(difflib.unified_diff(fl.src.splitlines(True), new_src.splitlines(True),
                                               f"a/{rel}", f"b/{rel}"))
    srel = stem_file.relative_to(ROOT).as_posix()
    sys.stdout.writelines(difflib.unified_diff(tsv.splitlines(True), new_tsv.splitlines(True),
                                               f"a/{srel}", f"b/{srel}"))
    if a.apply and sites:
        src_path.write_text(new_src, encoding="utf-8")
        stem_file.write_text(new_tsv, encoding="utf-8")
        print(f"applied: {len(sites)} site(s) in {rel}, {len(blocks)} new row(s) in {srel}", file=sys.stderr)
    elif sites:
        print(f"dry run: {len(sites)} site(s); add --apply to write", file=sys.stderr)
    empty = [s.key for s in sites if not s.en]
    if empty:
        print(f"note: no English yet for {', '.join(empty)} (check_rust.py fails until it is written)",
              file=sys.stderr)
    return 1 if refused else 0


def cmd_add(a) -> int:
    stem_file = ROOT / d.CATALOGUES / f"{a.stem}.tsv"
    if not KEY_RE.match(a.key) or a.key.split(".")[0] != a.stem:
        raise SystemExit(f"key {a.key!r} must look like {a.stem}.name")
    tsv = stem_file.read_text(encoding="utf-8")
    if a.key in existing_rows(tsv):
        raise SystemExit(f"{a.key} exists")
    for t in (a.nl, a.en):
        if "\r" in t:
            raise SystemExit("no carriage returns")
    mline = "#@ new"
    if a.source:
        # the row's nl is a 3b898e6 literal that the code no longer has as such (e.g. moved into a table)
        path, _, ordn = a.source.rpartition("#")
        frozen = {f.ordinal: f for f in d.load_frozen().get(path, [])}
        f = frozen.get(int(ordn)) if ordn.isdigit() else None
        if f is None:
            raise SystemExit(f"--from {a.source}: no such 3b898e6 literal")
        if f.text != a.nl:
            raise SystemExit(f"--from {a.source}: the 3b898e6 literal is {f.text!r}, not {a.nl!r}")
        mline = f"#@ {path} {f.ordinal} {f.sha[:d.SHA_PREFIX]} plain"
    block = "\n".join(([f"# {a.note}"] if a.note else []) + [mline,
                                                               f"{a.key}\t{d.tsv_escape(a.nl)}\t{d.tsv_escape(a.en)}"])
    new_tsv = section_insert(tsv, a.section or "new texts", block)
    srel = stem_file.relative_to(ROOT).as_posix()
    sys.stdout.writelines(difflib.unified_diff(tsv.splitlines(True), new_tsv.splitlines(True),
                                               f"a/{srel}", f"b/{srel}"))
    if a.apply:
        stem_file.write_text(new_tsv, encoding="utf-8")
    if not a.source:
        print(f"reminder: new Dutch text needs 'key:{a.key}' in {d.BASELINE} (drift_rust.py checks it)",
              file=sys.stderr)
    return 0


def cmd_map(a) -> int:
    """Record that an existing row replaced a 3b898e6 literal converted by hand (an inline pair's halves, a site the
    converter refused): writes the "#@" line after checking that the row's text rebuilds the literal."""
    stem = a.key.split(".")[0]
    stem_file = ROOT / d.CATALOGUES / f"{stem}.tsv"
    tsv = stem_file.read_text(encoding="utf-8")
    rows = existing_rows(tsv)
    if a.key not in rows:
        raise SystemExit(f"{a.key} is not in {stem_file.relative_to(ROOT)}")
    path, _, ordn = a.source.rpartition("#")
    f = {x.ordinal: x for x in d.load_frozen().get(path, [])}.get(int(ordn)) if ordn.isdigit() else None
    if f is None:
        raise SystemExit(f"{a.source}: no such 3b898e6 literal (tests/i18n/rust-frozen-all.tsv)")
    specs = []
    for spec in a.spec or []:
        m = d.MAP_SPEC.match(spec)
        if not m:
            raise SystemExit(f"--spec {spec!r}: want {{spec}}=name, e.g. {{}}=name")
        specs.append((m.group(1), m.group(2)))
    kind = ("enfmt" if specs else "en") if a.en else ("fmt" if specs else "plain")
    mapping = d.Mapping("", 0, path, f.ordinal, f.sha[:d.SHA_PREFIX], kind, specs)
    text = rows[a.key][1] if a.en else rows[a.key][0]
    if d.reconstruct(text, mapping) != f.text:
        raise SystemExit(f"{a.key}: its {'en' if a.en else 'nl'} {text!r} does not rebuild {path}#{f.ordinal} "
                         f"{f.text!r} with the map {specs}")
    mline = f"#@ {path} {f.ordinal} {f.sha[:d.SHA_PREFIX]} {kind}" + "".join(f" {sp}={nm}" for sp, nm in specs)
    new_tsv = add_mapping_above(tsv, a.key, mline)
    srel = stem_file.relative_to(ROOT).as_posix()
    sys.stdout.writelines(difflib.unified_diff(tsv.splitlines(True), new_tsv.splitlines(True),
                                               f"a/{srel}", f"b/{srel}"))
    if a.apply:
        stem_file.write_text(new_tsv, encoding="utf-8")
    return 0


def cmd_en(a) -> int:
    """Set the en text of an existing row (after a review); nl and the "#@" lines stay as they are."""
    stem = a.key.split(".")[0]
    stem_file = ROOT / d.CATALOGUES / f"{stem}.tsv"
    if not stem_file.is_file():
        raise SystemExit(f"no catalogue for {a.key}")
    if "\r" in a.text or not a.text:
        raise SystemExit("en must be non-empty and have no carriage return")
    tsv = stem_file.read_text(encoding="utf-8")
    lines = tsv.split("\n")
    for k, line in enumerate(lines):
        f = line.split("\t")
        if not line.startswith("#") and f[0] == a.key and len(f) >= 2:
            lines[k] = "\t".join([f[0], f[1], d.tsv_escape(a.text)])
            break
    else:
        raise SystemExit(f"{a.key} is not in {stem_file.relative_to(ROOT)}")
    new_tsv = "\n".join(lines)
    srel = stem_file.relative_to(ROOT).as_posix()
    sys.stdout.writelines(difflib.unified_diff(tsv.splitlines(True), new_tsv.splitlines(True),
                                               f"a/{srel}", f"b/{srel}"))
    if a.apply:
        stem_file.write_text(new_tsv, encoding="utf-8")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--h", default=os.environ.get("WIN11_I18N_H", str(H_DEFAULT)),
                    help="H, the frozen Hyprland tree with i18n/host/*.tsv (read-only)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("list")
    p.add_argument("file")
    p.add_argument("--all", action="store_true")
    p = sub.add_parser("convert")
    p.add_argument("file")
    p.add_argument("--stem", required=True)
    p.add_argument("--lang", default=DEFAULT_LANG, help=f"language expression (default {DEFAULT_LANG})")
    p.add_argument("--site", action="append", help="ORD=KEY[:name,name][@LANG]")
    p.add_argument("--en", action="append", help="ORD=English text")
    p.add_argument("--note", action="append", help="ORD=translator note (goes into the comment above the row)")
    p.add_argument("--plan", help="TSV: ordinal, key, names, lang, en, note ('-' = stdin)")
    p.add_argument("--force", action="store_true", help="also convert literals GE0b classed technical or log")
    p.add_argument("--expect", action="append", help="ORD=TEXT: refuse unless site ORD is exactly this text")
    p.add_argument("--apply", action="store_true")
    p = sub.add_parser("add")
    p.add_argument("--stem", required=True)
    p.add_argument("--key", required=True)
    p.add_argument("--nl", required=True)
    p.add_argument("--en", required=True)
    p.add_argument("--note", default="")
    p.add_argument("--section", default="", help="section name (default: new texts)")
    p.add_argument("--from", dest="source", default="",
                   help="PATH#ORD: the 3b898e6 literal (rust-frozen-all.tsv) whose text the nl is")
    p.add_argument("--apply", action="store_true")
    p = sub.add_parser("map", help="record a 3b898e6 literal that an existing row replaced (hand conversions)")
    p.add_argument("key")
    p.add_argument("--from", dest="source", required=True, help="PATH#ORD of tests/i18n/rust-frozen-all.tsv")
    p.add_argument("--en", action="store_true", help="the literal is the row's English (a pair's en half)")
    p.add_argument("--spec", action="append", help="placeholder map, per placeholder in order: {}=name")
    p.add_argument("--apply", action="store_true")
    p = sub.add_parser("en", help="set the en text of an existing row")
    p.add_argument("key")
    p.add_argument("text")
    p.add_argument("--apply", action="store_true")
    a = ap.parse_args(argv)
    if a.cmd == "convert":
        a.expect = dict(x.split("=", 1) for x in a.expect or [])
    return {"list": cmd_list, "convert": cmd_convert, "add": cmd_add, "en": cmd_en, "map": cmd_map}[a.cmd](a)


if __name__ == "__main__":
    sys.exit(main())
