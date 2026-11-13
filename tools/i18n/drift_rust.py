#!/usr/bin/env python3
"""Dutch drift guard for the Rust host (GE1b; plan-critique F12, notes/claude/round2-gate.md).

    python3 -B tools/i18n/drift_rust.py                 # check the working tree (exit 1 on drift)
    python3 -B tools/i18n/drift_rust.py --summary       # also print per-file counts
    python3 -B tools/i18n/drift_rust.py --freeze        # (re)write tests/i18n/rust-frozen-all.tsv from git 3b898e6
    python3 -B tools/i18n/drift_rust.py --paths host/src/runtime.rs …   # report only these files (round-2 agents)

The frozen file holds EVERY string literal of host/src/**/*.rs outside test code, plus every string of the
embedded JSON tables except their English halves, as they were at 3b898e6: decoded text, keyed by
(path, ordinal within the file, sha256). The check passes when each frozen literal

  1. is still present byte-identical in the same file (counted as a multiset per file and text), or
  2. is the 3b898e6 literal that a ``#@`` line in host/i18n/<stem>.tsv names, and the nl of the row under that line
     (its en for the kinds en/enfmt: the English half of a former inline pair), after the recorded placeholder map,
     rebuilds it byte for byte (and the file now uses the row's key), or
  3. is listed in tests/i18n/BASELINE-CHANGES.rust with a reason.

Every catalogue row must come from a 3b898e6 literal (rule 2) or carry ``#@ new`` and a ``key:`` line in
BASELINE-CHANGES.rust: Dutch text that is new is a Dutch change, and Dutch changes are listed.

This module is also the shared Rust lexer of extract_rs.py and check_rust.py (``literals()``, ``FileLits``).
Standard library only; reads git objects with ``git show`` (read-only).
"""
from __future__ import annotations

import argparse
import difflib
import hashlib
import json
import re
import subprocess
import sys
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
BASE = "3b898e6"
SRC = "host/src"
CATALOGUES = "host/i18n"
FROZEN = "tests/i18n/rust-frozen-all.tsv"
BASELINE = "tests/i18n/BASELINE-CHANGES.rust"
CLASSES = "tests/i18n/rust-literals.tsv"            # GE0b's classification (scan_rust.py), joined by file:line:col
# The embedded JSON text tables (the same seven scan_rust.py reads). Their English halves are not frozen.
JSON_TABLES = ("apps/catalog-data.json", "files/launcher-names.json", "files/types.json",
               "services/disks/data.json", "services/storage/data.json", "services/sysmon/text.json",
               "services/system/strings.json")
# [nl, en] arrays: the second element is English
JSON_PAIR_ARRAYS = {"files/types.json": ((),), "services/sysmon/text.json": (("_FRIENDLY",),),
                    "services/system/strings.json": (("wifi",),)}
SHA_PREFIX = 16


# ------------------------------------------------------------------------------------------------------- lexer

@dataclass
class Tok:
    kind: str      # ident, str, char, num, punct, lifetime
    text: str      # identifier / punct text, or the decoded value of a string literal
    start: int     # offsets into the source text
    end: int
    line: int
    col: int
    flavor: str = ""   # str, raw, bytes, rawbytes, c, rawc


PUNCT3 = ("..=", "...", "<<=", ">>=")
PUNCT2 = ("::", "=>", "==", "!=", "->", "<=", ">=", "&&", "||", "..", "+=", "-=", "*=", "/=", "%=", "^=", "&=",
          "|=", "<<", ">>")
RAW_START = re.compile(r'(b|c)?r(#*)"')
STR_START = re.compile(r'(b|c)?"')
IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
NUMBER = re.compile(r"[0-9][0-9A-Za-z_]*(?:\.[0-9][0-9A-Za-z_]*)?")
ESC = {"n": "\n", "r": "\r", "t": "\t", "\\": "\\", "0": "\0", "'": "'", '"': '"'}


def unescape(body: str) -> str:
    """The value of a (non-raw) Rust string literal body."""
    out: list[str] = []
    i = 0
    while i < len(body):
        c = body[i]
        if c != "\\":
            out.append(c)
            i += 1
            continue
        d = body[i + 1] if i + 1 < len(body) else ""
        if d in "\n\r":
            i += 2
            while i < len(body) and body[i] in " \t\n\r":
                i += 1
        elif d in ESC:
            out.append(ESC[d])
            i += 2
        elif d == "x":
            out.append(chr(int(body[i + 2:i + 4], 16)))
            i += 4
        elif d == "u":
            j = body.index("}", i)
            out.append(chr(int(body[i + 3:j].replace("_", ""), 16)))
            i = j + 1
        else:
            out.append("\\" + d)
            i += 2
    return "".join(out)


def lex(src: str) -> list[Tok]:
    toks: list[Tok] = []
    i, n, line, ls = 0, len(src), 1, 0

    def advance(a: int, b: int) -> None:
        nonlocal line, ls
        k = src.count("\n", a, b)
        if k:
            line += k
            ls = src.rfind("\n", a, b) + 1

    while i < n:
        c = src[i]
        if c == "\n":
            line += 1
            ls = i + 1
            i += 1
            continue
        if c in " \t\r":
            i += 1
            continue
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
            continue
        if src.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if src.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif src.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            advance(i, j)
            i = j
            continue
        sl, sc = line, i - ls + 1
        m = RAW_START.match(src, i)
        if m:
            hashes = m.group(2)
            body = m.end()
            end = src.index('"' + hashes, body)
            flavor = {"b": "rawbytes", "c": "rawc"}.get(m.group(1) or "", "raw")
            toks.append(Tok("str", src[body:end], i, end + 1 + len(hashes), sl, sc, flavor))
            advance(i, end)
            i = end + 1 + len(hashes)
            continue
        m = STR_START.match(src, i)
        if m:
            j = m.end()
            while src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            flavor = {"b": "bytes", "c": "c"}.get(m.group(1) or "", "str")
            toks.append(Tok("str", unescape(src[m.end():j]), i, j + 1, sl, sc, flavor))
            advance(i, j)
            i = j + 1
            continue
        if c == "'" or (c == "b" and src.startswith("b'", i)):
            q = i + (1 if c == "'" else 2)
            if src[q] == "\\":
                if src[q + 1] == "u":
                    j = src.index("}", q) + 1
                elif src[q + 1] == "x":
                    j = q + 4
                else:
                    j = q + 2
                toks.append(Tok("char", src[q:j], i, j + 1, sl, sc))
                i = j + 1
                continue
            if q + 1 < n and src[q + 1] == "'":
                toks.append(Tok("char", src[q], i, q + 2, sl, sc))
                i = q + 2
                continue
            if c == "'":
                m = IDENT.match(src, i + 1)
                e = m.end() if m else i + 1
                toks.append(Tok("lifetime", src[i:e], i, e, sl, sc))
                i = e
                continue
        if src.startswith("r#", i) and IDENT.match(src, i + 2):
            m = IDENT.match(src, i + 2)
            toks.append(Tok("ident", m.group(0), i, m.end(), sl, sc))
            i = m.end()
            continue
        m = IDENT.match(src, i)
        if m:
            toks.append(Tok("ident", m.group(0), i, m.end(), sl, sc))
            i = m.end()
            continue
        m = NUMBER.match(src, i)
        if m:
            text = m.group(0)
            if "." in text and src.startswith("..", i + text.index(".")):
                text = text[:text.index(".")]
            toks.append(Tok("num", text, i, i + len(text), sl, sc))
            i += len(text)
            continue
        for p in PUNCT3 + PUNCT2:
            if src.startswith(p, i):
                toks.append(Tok("punct", p, i, i + len(p), sl, sc))
                i += len(p)
                break
        else:
            toks.append(Tok("punct", c, i, i + 1, sl, sc))
            i += 1
    return toks


# --------------------------------------------------------------------------------------------- literals of a file

@dataclass
class Lit:
    path: str          # host/src/…
    ordinal: int       # 1-based among the file's non-test literals
    tok: int           # index into FileLits.toks
    text: str
    flavor: str
    line: int
    col: int
    fn: str = ""       # innermost enclosing fn
    item: str = ""     # enclosing const/static item name
    macro: str = ""    # innermost enclosing macro invocation (format!, json!, eprintln! …)
    call: str = ""     # innermost enclosing call (ApiError::new, .expect, …)

    @property
    def sha(self) -> str:
        return sha(self.text)


def sha(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8", "surrogatepass")).hexdigest()


def is_test_attr(attr: list[Tok]) -> bool:
    texts = [t.text for t in attr]
    if texts == ["test"]:
        return True
    if texts[:1] != ["cfg"] or "not" in texts or "test" not in texts:
        return False
    return texts[1:3] == ["(", "test"] or "all" in texts


ITEM_BLOCK = {"fn", "mod", "impl", "trait", "struct", "enum", "union", "macro_rules"}
ITEM_SEMI = {"const", "static", "use", "type", "let", "extern"}
MODIFIERS = {"pub", "unsafe", "async", "default", "crate"}
OPEN, CLOSE = "([{", ")]}"


def item_end(toks: list[Tok], i: int) -> int:
    """Index just past the item / field / statement that starts at toks[i] (the target of an outer attribute)."""
    n = len(toks)
    j = i
    while j < n and toks[j].kind == "ident" and toks[j].text in MODIFIERS:
        j += 1
        if j < n and toks[j].text == "(":           # pub(crate), pub(super)
            depth = 0
            while j < n:
                depth += toks[j].text == "("
                depth -= toks[j].text == ")"
                j += 1
                if depth == 0:
                    break
    head = toks[j].text if j < n else ""
    if head == "extern" and j + 1 < n and toks[j + 1].kind == "str":
        head = toks[j + 2].text if j + 2 < n else ""
    mode = "block" if head in ITEM_BLOCK else "semi" if head in ITEM_SEMI else "expr"
    depth = 0
    k = j
    while k < n:
        t = toks[k]
        if t.kind == "punct" and t.text in OPEN:
            depth += 1
        elif t.kind == "punct" and t.text in CLOSE:
            if depth == 0:
                return k                      # the enclosing bracket closes: the item had no terminator
            depth -= 1
            if depth == 0 and t.text == "}" and mode != "semi":
                nxt = toks[k + 1].text if k + 1 < n else ""
                if mode == "expr" and nxt == "else":
                    k += 1
                    continue
                if nxt in (";", ",") and mode == "expr":
                    return k + 2
                if mode == "block" or mode == "expr":
                    return k + 1
        elif depth == 0 and t.kind == "punct":
            if t.text == ";":
                return k + 1
            if t.text == "," and mode == "expr":
                return k + 1
        k += 1
    return n


@dataclass
class ModDecl:
    name: str
    path_attr: str | None
    test: bool


@dataclass
class FileLits:
    path: str
    src: str
    toks: list[Tok]
    lits: list[Lit] = field(default_factory=list)
    mods: list[ModDecl] = field(default_factory=list)
    whole_file_test: bool = False
    skipped: list[tuple[int, int]] = field(default_factory=list)   # token ranges of test items


def scan_file(path: str, src: str) -> FileLits:
    toks = lex(src)
    out = FileLits(path, src, toks)
    n = len(toks)
    # (kind, name, depth-of-opening) for fn scopes / const items; bracket stack entries: (char, macro, call)
    scopes: list[tuple[str, str, int]] = []
    brackets: list[tuple[str, str, str]] = []
    pending: tuple[str, str, int] | None = None          # a `fn name` whose body `{` has not opened yet
    attrs: list[tuple[list[Tok], list[int]]] = []         # outer attributes of the next item, with their literals
    i = 0
    ordinal = 0

    def callee(k: int) -> tuple[str, str]:
        """For the bracket at toks[k]: (macro name, call path) it opens."""
        if k >= 2 and toks[k - 1].text == "!" and toks[k - 2].kind == "ident":
            return toks[k - 2].text + "!", ""
        if toks[k].text == "(" and k >= 1 and toks[k - 1].kind == "ident" and toks[k - 1].text not in (
                "if", "while", "match", "return", "in", "for", "let", "else", "move"):
            parts = [toks[k - 1].text]
            m = k - 2
            while m >= 1 and toks[m].text == "::" and toks[m - 1].kind == "ident":
                parts.insert(0, toks[m - 1].text)
                m -= 2
            if m >= 0 and toks[m].text == ".":
                return "", "." + parts[-1]
            return "", "::".join(parts)
        return "", ""

    def add(k: int) -> None:
        nonlocal ordinal
        ordinal += 1
        macro = next((m for _, m, _ in reversed(brackets) if m), "")
        call = next((c for _, _, c in reversed(brackets) if c), "")
        out.lits.append(Lit(path, ordinal, k, toks[k].text, toks[k].flavor, toks[k].line, toks[k].col,
                            fn=scopes_fn(scopes), item=scopes_item(scopes), macro=macro, call=call))

    while i < n:
        t = toks[i]
        # attributes
        if t.text == "#" and i + 1 < n and (toks[i + 1].text == "[" or (toks[i + 1].text == "!" and i + 2 < n
                                                                         and toks[i + 2].text == "[")):
            inner = toks[i + 1].text == "!"
            start = i + (3 if inner else 2)
            depth, j = 1, start
            while j < n and depth:
                depth += toks[j].text == "["
                depth -= toks[j].text == "]"
                j += 1
            body = toks[start:j - 1]
            strs = [k for k in range(start, j - 1) if toks[k].kind == "str"]
            if inner:
                if is_test_attr(body) and not brackets:
                    out.whole_file_test = True
                for k in strs:
                    add(k)
            else:
                attrs.append((body, strs))
            i = j
            continue
        if attrs:
            is_test = any(is_test_attr(a) for a, _ in attrs)
            path_attr = None
            for a, _ in attrs:
                if len(a) == 3 and a[0].text == "path" and a[1].text == "=" and a[2].kind == "str":
                    path_attr = a[2].text
            if not is_test:
                for _, strs in attrs:
                    for k in strs:
                        add(k)
            attrs = []
            # module declarations: `[pub] mod name;`
            j = i
            while j < n and toks[j].text in MODIFIERS:
                j += 1
                if j < n and toks[j].text == "(":
                    while j < n and toks[j].text != ")":
                        j += 1
                    j += 1
            if j + 2 < n and toks[j].text == "mod" and toks[j + 1].kind == "ident" and toks[j + 2].text == ";":
                out.mods.append(ModDecl(toks[j + 1].text, path_attr, is_test))
                i = j + 3
                continue
            if is_test:
                end = item_end(toks, i)
                out.skipped.append((i, end))
                # `mod x;` inside a skipped inline test module: test files too
                for k in range(i, end - 2):
                    if toks[k].text == "mod" and toks[k + 1].kind == "ident" and toks[k + 2].text == ";":
                        out.mods.append(ModDecl(toks[k + 1].text, None, True))
                i = end
                continue
        if t.kind == "ident" and t.text == "mod" and i + 2 < n and toks[i + 1].kind == "ident" \
                and toks[i + 2].text == ";":
            out.mods.append(ModDecl(toks[i + 1].text, None, False))
            i += 3
            continue
        if t.kind == "ident" and t.text == "fn" and i + 1 < n and toks[i + 1].kind == "ident":
            pending = ("fn", toks[i + 1].text, len(brackets))
        elif t.kind == "ident" and t.text in ("const", "static") and i + 2 < n and toks[i + 1].kind == "ident" \
                and toks[i + 2].text == ":":
            scopes.append(("item", toks[i + 1].text, len(brackets)))
        if t.kind == "punct" and t.text in OPEN:
            macro, call = callee(i)
            if t.text == "{" and pending and pending[2] == len(brackets):
                brackets.append((t.text, macro, call))
                scopes.append(("fn", pending[1], len(brackets)))
                pending = None
            else:
                brackets.append((t.text, macro, call))
        elif t.kind == "punct" and t.text in CLOSE:
            while scopes and scopes[-1][0] == "fn" and scopes[-1][2] == len(brackets):
                scopes.pop()
            if brackets:
                brackets.pop()
        elif t.kind == "punct" and t.text == ";":
            if pending and pending[2] == len(brackets):
                pending = None
            while scopes and scopes[-1][0] == "item" and scopes[-1][2] == len(brackets):
                scopes.pop()
        if t.kind == "str" and not out.whole_file_test:
            add(i)
        i += 1
    if out.whole_file_test:
        out.lits = []
    return out


def scopes_fn(scopes) -> str:
    return next((name for kind, name, _ in reversed(scopes) if kind == "fn"), "")


def scopes_item(scopes) -> str:
    return next((name for kind, name, _ in reversed(scopes) if kind == "item"), "")


def module_children(rel: str, decl: ModDecl) -> list[str]:
    p = Path(rel)
    if decl.path_attr:
        return [(p.parent / decl.path_attr).as_posix()]
    root_like = p.name in ("mod.rs", "lib.rs", "main.rs") or p.parent.name == "bin"
    d = p.parent if root_like else p.parent / p.stem
    return [(d / f"{decl.name}.rs").as_posix(), (d / decl.name / "mod.rs").as_posix()]


def normalize(rel: str) -> str:
    parts: list[str] = []
    for part in rel.split("/"):
        if part == "..":
            if parts:
                parts.pop()
        elif part and part != ".":
            parts.append(part)
    return "/".join(parts)


def scan_tree(files: dict[str, str]) -> tuple[dict[str, FileLits], set[str]]:
    """files: {path relative to host/src: source}. Returns ({path: FileLits}, test files)."""
    scanned = {rel: scan_file(f"{SRC}/{rel}", src) for rel, src in files.items()}
    tests: set[str] = {rel for rel, fl in scanned.items() if fl.whole_file_test}
    queue: list[str] = []
    for rel, fl in scanned.items():
        for decl in fl.mods:
            if decl.test:
                queue += [normalize(c) for c in module_children(rel, decl)]
    while queue:
        c = queue.pop()
        if c in tests or c not in scanned:
            continue
        tests.add(c)
        for decl in scanned[c].mods:
            queue += [normalize(k) for k in module_children(c, decl)]
    return scanned, tests


def literals(files: dict[str, str]) -> dict[str, list[Lit]]:
    """Non-test literals per file path (host/src/…), test files left out."""
    scanned, tests = scan_tree(files)
    return {f"{SRC}/{rel}": fl.lits for rel, fl in sorted(scanned.items()) if rel not in tests}


# ------------------------------------------------------------------------------------------------- JSON tables

def json_values(path: str, text: str) -> list[Lit]:
    """Every string of a JSON table except English halves (key "en", or the second element of an [nl, en] pair)."""
    data = json.loads(text)
    rel = path[len(SRC) + 1:]
    pair_parents = JSON_PAIR_ARRAYS.get(rel, ())
    out: list[Lit] = []

    def walk(v, ptr: list) -> None:
        if isinstance(v, dict):
            for k, x in v.items():
                if k == "en":
                    continue
                walk(x, ptr + [k])
        elif isinstance(v, list):
            pair = (len(v) == 2 and all(isinstance(x, str) for x in v)
                    and tuple(str(p) for p in ptr[:-1]) in pair_parents)
            for k, x in enumerate(v):
                if pair and k == 1:
                    continue
                walk(x, ptr + [k])
        elif isinstance(v, str):
            pointer = "/" + "/".join(str(p).replace("~", "~0").replace("/", "~1") for p in ptr)
            out.append(Lit(path, len(out) + 1, -1, v, "json", 0, 0, fn=pointer))

    walk(data, [])
    return out


# ------------------------------------------------------------------------------------------------- tree access

def git_files(rev: str, root: Path = ROOT) -> dict[str, str]:
    names = subprocess.run(["git", "-C", str(root), "ls-tree", "-r", "--name-only", rev, SRC + "/"],
                           capture_output=True, text=True, check=True).stdout.split("\n")
    out = {}
    for name in names:
        if name.endswith(".rs") or name[len(SRC) + 1:] in JSON_TABLES:
            blob = subprocess.run(["git", "-C", str(root), "show", f"{rev}:{name}"], capture_output=True,
                                  check=True).stdout
            out[name[len(SRC) + 1:]] = blob.decode("utf-8")
    return out


def tree_files(root: Path = ROOT) -> dict[str, str]:
    base = root / SRC
    out = {p.relative_to(base).as_posix(): p.read_text(encoding="utf-8") for p in sorted(base.rglob("*.rs"))}
    for rel in JSON_TABLES:
        p = base / rel
        if p.is_file():
            out[rel] = p.read_text(encoding="utf-8")
    return out


def all_literals(files: dict[str, str]) -> dict[str, list[Lit]]:
    rust = {rel: src for rel, src in files.items() if rel.endswith(".rs")}
    out = literals(rust)
    for rel in JSON_TABLES:
        if rel in files:
            out[f"{SRC}/{rel}"] = json_values(f"{SRC}/{rel}", files[rel])
    return out


# ------------------------------------------------------------------------------------------------ frozen file

def esc(s: str) -> str:
    return s.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n").replace("\r", "\\r")


def unesc(s: str) -> str:
    out, i = [], 0
    while i < len(s):
        if s[i] == "\\" and i + 1 < len(s):
            out.append({"n": "\n", "t": "\t", "r": "\r", "\\": "\\"}.get(s[i + 1], "\\" + s[i + 1]))
            i += 2
        else:
            out.append(s[i])
            i += 1
    return "".join(out)


@dataclass
class Frozen:
    path: str
    ordinal: int
    sha: str
    line: int
    col: int
    flavor: str
    cls: str
    where: str
    text: str


FROZEN_HEADER = """# tests/i18n/rust-frozen-all.tsv: EVERY string literal of host/src/**/*.rs outside test code, plus every string of
# the embedded JSON text tables except their English halves, at 3b898e6 (GE1b; plan-critique F12). Keyed by
# path + ordinal within the file + sha256 of the decoded text; line/col are informational. class is GE0b's
# (tests/i18n/rust-literals.tsv, "-" where it has no row). Regenerate only with
# `python3 -B tools/i18n/drift_rust.py --freeze` (reads git 3b898e6, never the working tree).
# Checked by `python3 -B tools/i18n/drift_rust.py`; text escapes \\\\ \\t \\n \\r.
path\tordinal\tsha256\tline\tcol\tflavor\tclass\twhere\ttext
"""


def load_classes(root: Path = ROOT) -> dict[tuple[str, int, int], str]:
    out: dict[tuple[str, int, int], str] = {}
    p = root / CLASSES
    if not p.is_file():
        return out
    for line in p.read_text(encoding="utf-8").split("\n"):
        if not line or line.startswith("#") or line.startswith("file\t"):
            continue
        f = line.split("\t")
        if len(f) >= 6 and f[1].isdigit() and f[2].isdigit():
            out.setdefault((f[0], int(f[1]), int(f[2])), f[5])
    return out


def freeze(root: Path = ROOT) -> tuple[int, dict]:
    files = git_files(BASE, root)
    lits = all_literals(files)
    classes = load_classes(root)
    rows = []
    stats = {"rust": 0, "json": 0, "files": 0}
    for path in sorted(lits):
        stats["files"] += 1
        for lit in lits[path]:
            rel = path[len(SRC) + 1:]
            cls = classes.get((rel, lit.line, lit.col), "-") if lit.flavor != "json" else "-"
            where = lit.fn or "-"
            if lit.flavor == "json":
                stats["json"] += 1
            else:
                stats["rust"] += 1
            rows.append("\t".join([path, str(lit.ordinal), lit.sha, str(lit.line), str(lit.col), lit.flavor, cls,
                                   esc(where), esc(lit.text)]))
    (root / FROZEN).write_text(FROZEN_HEADER + "\n".join(rows) + "\n", encoding="utf-8")
    return len(rows), stats


def load_frozen(root: Path = ROOT) -> dict[str, list[Frozen]]:
    out: dict[str, list[Frozen]] = {}
    for line in (root / FROZEN).read_text(encoding="utf-8").split("\n"):
        if not line or line.startswith("#") or line.startswith("path\t"):
            continue
        f = line.split("\t")
        out.setdefault(f[0], []).append(Frozen(f[0], int(f[1]), f[2], int(f[3]), int(f[4]), f[5], f[6],
                                               unesc(f[7]), unesc(f[8])))
    return out


# --------------------------------------------------------------------------------------------- catalogues + maps

PLACEHOLDER = re.compile(r"\{([A-Za-z0-9_]+)\}")
MAP_SPEC = re.compile(r"^(\{[^{}\s]*\})=([A-Za-z0-9_]+)$")


@dataclass
class Mapping:
    file: str          # host/i18n/<stem>.tsv
    line: int
    path: str          # host/src/… or "new"
    ordinal: int | None
    sha16: str
    kind: str          # fmt, plain, new
    specs: list[tuple[str, str]]   # [(original spec, name)] per placeholder occurrence, in order


@dataclass
class Row:
    file: str
    line: int
    key: str
    nl: str
    en: str
    raw: list[str]
    maps: list[Mapping] = field(default_factory=list)
    comments: list[str] = field(default_factory=list)


def tsv_unescape(s: str) -> str:
    out, i = [], 0
    while i < len(s):
        if s[i] == "\\" and i + 1 < len(s) and s[i + 1] in "nt\\":
            out.append({"n": "\n", "t": "\t", "\\": "\\"}[s[i + 1]])
            i += 2
        else:
            out.append(s[i])
            i += 1
    return "".join(out)


def tsv_escape(s: str) -> str:
    return s.replace("\\", "\\\\").replace("\t", "\\t").replace("\n", "\\n")


def parse_mapping(file: str, number: int, line: str) -> Mapping | str:
    parts = line[2:].strip().split()
    if parts[:1] == ["new"]:
        return Mapping(file, number, "new", None, "", "new", [])
    if len(parts) < 4:
        return f"{file}:{number}: '#@ <path> <ordinal> <sha> <fmt|plain>' expected"
    path, ordinal, sha16, kind = parts[:4]
    if kind not in ("fmt", "plain", "en", "enfmt"):
        return f"{file}:{number}: kind must be fmt, plain, en or enfmt, not {kind}"
    if not (ordinal.isdigit() or ordinal == "new"):
        return f"{file}:{number}: ordinal must be a number"
    if not re.fullmatch(r"[0-9a-f]{%d}" % SHA_PREFIX, sha16):
        return f"{file}:{number}: sha must be {SHA_PREFIX} hex digits"
    specs = []
    for s in parts[4:]:
        m = MAP_SPEC.match(s)
        if not m:
            return f"{file}:{number}: bad placeholder map entry {s!r} (want {{spec}}=name)"
        specs.append((m.group(1), m.group(2)))
    return Mapping(file, number, path, int(ordinal) if ordinal.isdigit() else None, sha16, kind, specs)


def load_catalogues(root: Path = ROOT) -> tuple[list[Row], list[str]]:
    rows: list[Row] = []
    problems: list[str] = []
    for f in sorted((root / CATALOGUES).glob("*.tsv")):
        rel = f.relative_to(root).as_posix()
        maps: list[Mapping] = []
        comments: list[str] = []
        for number, line in enumerate(f.read_text(encoding="utf-8").split("\n"), 1):
            if line.startswith("#@"):
                m = parse_mapping(rel, number, line)
                if isinstance(m, str):
                    problems.append(m)
                else:
                    maps.append(m)
                continue
            if line.startswith("#"):
                comments.append(line)
                continue
            if not line:
                if maps:
                    problems.append(f"{rel}:{number}: '#@' line(s) not directly above a row")
                maps, comments = [], []
                continue
            fields = line.split("\t")
            key = fields[0]
            nl = tsv_unescape(fields[1]) if len(fields) > 1 else ""
            en = tsv_unescape(fields[2]) if len(fields) > 2 else ""
            rows.append(Row(rel, number, key, nl, en, fields, maps, comments))
            maps, comments = [], []
        if maps:
            problems.append(f"{rel}: '#@' line(s) at the end of the file")
    return rows, problems


def reconstruct(nl: str, mapping: Mapping) -> str | None:
    """The 3b898e6 literal a row's text came from, per the mapping; None when the map does not fit. Pass the row's
    nl for fmt/plain and its en for en/enfmt (the English half of a former inline {nl, en} pair)."""
    if mapping.kind in ("plain", "en"):
        return nl if not mapping.specs else None
    out, last, k = [], 0, 0
    for m in PLACEHOLDER.finditer(nl):
        out.append(nl[last:m.start()].replace("{", "{{").replace("}", "}}"))
        if k >= len(mapping.specs) or mapping.specs[k][1] != m.group(1):
            return None
        out.append(mapping.specs[k][0])
        k += 1
        last = m.end()
    if k != len(mapping.specs):
        return None
    out.append(nl[last:].replace("{", "{{").replace("}", "}}"))
    return "".join(out)


# ------------------------------------------------------------------------------------------- BASELINE-CHANGES

@dataclass
class Change:
    file: str
    line: int
    path: str          # host/src/… ; or "key:<tsv key>"
    ordinal: int | None
    sha16: str
    replacement: str
    reason: str


def baseline_files(root: Path = ROOT) -> list[Path]:
    """BASELINE-CHANGES.rust plus the round-2 agents' BASELINE-CHANGES.rust-r2<x> files."""
    main = root / BASELINE
    return [main] + sorted(p for p in main.parent.glob(main.name + "-*") if p.is_file())


def load_baseline(root: Path = ROOT) -> tuple[list[Change], list[str]]:
    out: list[Change] = []
    problems: list[str] = []
    if not (root / BASELINE).is_file():
        return out, [f"{BASELINE} is missing"]
    for p in baseline_files(root):
        rel = p.relative_to(root).as_posix()
        for number, line in enumerate(p.read_text(encoding="utf-8").split("\n"), 1):
            if not line or line.startswith("#"):
                continue
            f = line.split("\t")
            if len(f) != 5 or not f[4].strip():
                problems.append(f"{rel}:{number}: expected path<TAB>ordinal<TAB>sha16<TAB>replacement<TAB>reason")
                continue
            if f[0].startswith("key:"):
                out.append(Change(rel, number, f[0], None, "", f[3], f[4]))
                continue
            if not f[1].isdigit() or not re.fullmatch(r"[0-9a-f]{%d}" % SHA_PREFIX, f[2]):
                problems.append(f"{rel}:{number}: ordinal must be a number and sha {SHA_PREFIX} hex digits")
                continue
            out.append(Change(rel, number, f[0], int(f[1]), f[2], f[3], f[4]))
    return out, problems


# ------------------------------------------------------------------------------------------------------ check

def key_used(lits: list[Lit], key: str) -> bool:
    base = re.sub(r"\.(one|other|zero)$", "", key)
    return any(lit.text in (key, base) for lit in lits)


def check(root: Path = ROOT, only: set[str] | None = None) -> tuple[list[str], list[str], dict]:
    errors: list[str] = []
    warnings: list[str] = []
    frozen = load_frozen(root)
    current = all_literals(tree_files(root))
    rows, problems = load_catalogues(root)
    errors += problems
    changes, problems = load_baseline(root)
    errors += problems
    key_changes = {c.path[4:]: c for c in changes if c.path.startswith("key:")}
    covered: dict[tuple[str, str], set[int]] = {}
    stats = {"frozen": sum(len(v) for v in frozen.values()), "present": 0, "mapped": 0, "baseline": 0,
             "rows": len(rows), "files": len(frozen)}
    # 2. mappings
    by_key = {r.key for r in rows}
    for row in rows:
        good = False
        for m in row.maps:
            if m.kind == "new":
                if row.key not in key_changes:
                    errors.append(f"{row.file}:{row.line}: {row.key} is new Dutch text ('#@ new'): it needs a "
                                  f"'key:{row.key}' line in {BASELINE}")
                good = True
                continue
            if m.ordinal is None:
                errors.append(f"{m.file}:{m.line}: {row.key}: the literal is not in the 3b898e6 file; use '#@ new'")
                continue
            entries = {f.ordinal: f for f in frozen.get(m.path, [])}
            f = entries.get(m.ordinal)
            if f is None or not f.sha.startswith(m.sha16):
                errors.append(f"{m.file}:{m.line}: {row.key}: no 3b898e6 literal {m.path}#{m.ordinal} {m.sha16}")
                continue
            side = row.en if m.kind in ("en", "enfmt") else row.nl
            again = reconstruct(side, m)
            if again != f.text:
                which = "en" if m.kind in ("en", "enfmt") else "nl"
                errors.append(f"{m.file}:{m.line}: {row.key}: {which} does not rebuild the 3b898e6 literal "
                              f"{m.path}#{m.ordinal}: {f.text!r} != {again!r}")
                continue
            if m.path in current and not key_used(current[m.path], row.key):
                errors.append(f"{m.file}:{m.line}: {row.key}: {m.path} does not use this key")
            got = covered.setdefault((m.path, f.sha), set())
            if m.ordinal in got:
                errors.append(f"{m.file}:{m.line}: {m.path}#{m.ordinal} is mapped twice")
            got.add(m.ordinal)
            good = True
        if not row.maps:
            errors.append(f"{row.file}:{row.line}: {row.key} has no '#@' source line (rows come from extract_rs.py)")
        elif not good:
            pass
    # 3. baseline literal lines
    for c in changes:
        if c.path.startswith("key:"):
            if c.path[4:] not in by_key:
                errors.append(f"{c.file}:{c.line}: {c.path}: no such catalogue row")
            continue
        entries = {f.ordinal: f for f in frozen.get(c.path, [])}
        f = entries.get(c.ordinal)
        if f is None or not f.sha.startswith(c.sha16):
            errors.append(f"{c.file}:{c.line}: no 3b898e6 literal {c.path}#{c.ordinal} {c.sha16}")
            continue
        got = covered.setdefault((c.path, f.sha), set())
        if c.ordinal in got:
            warnings.append(f"{c.file}:{c.line}: {c.path}#{c.ordinal} is also mapped by a catalogue row")
        got.add(c.ordinal)
    # 1. multiset per file
    per_file: dict[str, dict] = {}
    uncovered: list[Frozen] = []
    for path, flist in sorted(frozen.items()):
        if only and path not in only:
            continue
        now: dict[str, int] = {}
        for lit in current.get(path, []):
            now[lit.sha] = now.get(lit.sha, 0) + 1
        # literals the alignment says are gone come first, so a report names the occurrence that was edited
        sm = difflib.SequenceMatcher(None, [f.sha for f in flist], [lit.sha for lit in current.get(path, [])],
                                     autojunk=False)
        gone = {flist[k].ordinal for tag, i1, i2, _, _ in sm.get_opcodes() if tag != "equal" for k in range(i1, i2)}
        want: dict[str, list[Frozen]] = {}
        for f in sorted(flist, key=lambda f: (f.ordinal not in gone, f.ordinal)):
            want.setdefault(f.sha, []).append(f)
        file_stats = {"frozen": len(flist), "deficit": 0, "covered": 0}
        for digest, fs in want.items():
            have = now.get(digest, 0)
            deficit = max(0, len(fs) - have)
            stats["present"] += min(len(fs), have)
            cov = covered.get((path, digest), set())
            file_stats["deficit"] += deficit
            file_stats["covered"] += min(deficit, len(cov))
            if deficit > len(cov):
                missing = [f for f in fs if f.ordinal not in cov][: deficit - len(cov)]
                uncovered += missing
                for f in missing:
                    errors.append(f"{path}#{f.ordinal} (3b898e6 line {f.line}): {f.text!r} changed or removed "
                                  "without a catalogue mapping or a BASELINE-CHANGES.rust line")
            elif len(cov) > deficit:
                warnings.append(f"{path}: {len(cov) - deficit} mapping/baseline line(s) for {fs[0].text!r} although "
                                "the literal is still present")
        per_file[path] = file_stats
    stats["mapped"] = sum(len(v) for v in covered.values())
    stats["per_file"] = per_file
    stats["uncovered"] = uncovered
    if only:
        keep = tuple(only)
        errors = [e for e in errors if any(p in e for p in keep) or e.startswith(CATALOGUES) or e.startswith(BASELINE)]
    return errors, warnings, stats


# ----------------------------------------------------------------------------------------------------- ratchet

@dataclass
class NewLit:
    path: str
    ordinal: int
    line: int
    col: int
    text: str
    flavor: str
    cls: str           # GE0b's class (scan_rust.py), computed on the working tree
    lang: str
    reason: str
    lit: Lit


# A new literal of this class is Dutch text outside the catalogues: an error, and so is the Dutch half of a new
# inline pair. A new user-visible-english text and Dutch-looking text the classifier calls technical are warnings.
RATCHET_DUTCH = ("user-visible-dutch",)


def new_literals(root: Path = ROOT, current: dict[str, list[Lit]] | None = None,
                 frozen: dict[str, list[Frozen]] | None = None) -> list[NewLit]:
    """Every literal of the working tree beyond the frozen multiset of its file (a file that did not exist at
    3b898e6 is new as a whole), classified by GE0b's scan_rust.py run on the working tree. Of several equal texts,
    the occurrences the alignment with the 3b898e6 file marks as inserted are the new ones."""
    frozen = frozen if frozen is not None else load_frozen(root)
    current = current if current is not None else all_literals(tree_files(root))
    picked: list[Lit] = []
    for path, lits in sorted(current.items()):
        flist = frozen.get(path, [])
        have: dict[str, int] = {}
        for f in flist:
            have[f.sha] = have.get(f.sha, 0) + 1
        now: dict[str, int] = {}
        for lit in lits:
            now[lit.sha] = now.get(lit.sha, 0) + 1
        excess = {h: n - have.get(h, 0) for h, n in now.items() if n > have.get(h, 0)}
        if not excess:
            continue
        sm = difflib.SequenceMatcher(None, [f.sha for f in flist], [lit.sha for lit in lits], autojunk=False)
        order = [j for tag, _, _, j1, j2 in sm.get_opcodes() if tag != "equal" for j in range(j1, j2)]
        seen = set(order)
        order += [j for j in range(len(lits)) if j not in seen]
        for j in order:
            lit = lits[j]
            if excess.get(lit.sha, 0) > 0:
                excess[lit.sha] -= 1
                picked.append(lit)
    if not picked:
        return []
    if str(HERE) not in sys.path:
        sys.path.insert(0, str(HERE))
    import scan_rust  # noqa: PLC0415  (GE0b's classifier, read-only; about 5 s for the whole tree)
    scanned, _ = scan_rust.scan(root)
    by_pos = {(f"{SRC}/{s.file}", s.line, s.col): s for s in scanned if s.source != "json"}
    by_text: dict[tuple[str, str], object] = {}
    for s in scanned:
        if s.source == "json":
            by_text.setdefault((f"{SRC}/{s.file}", s.text), s)
    out = []
    for lit in sorted(picked, key=lambda x: (x.path, x.ordinal)):
        s = by_text.get((lit.path, lit.text)) if lit.flavor == "json" else by_pos.get((lit.path, lit.line, lit.col))
        out.append(NewLit(lit.path, lit.ordinal, lit.line, lit.col, lit.text, lit.flavor,
                          getattr(s, "cls", "") or "unclassified", getattr(s, "lang", "") or "none",
                          getattr(s, "reason", "") or "", lit))
    return out


def ratchet(root: Path = ROOT, only: set[str] | None = None,
            new: list[NewLit] | None = None) -> tuple[list[str], list[str], dict]:
    """The Rust ratchet (review M1): a literal that is new since 3b898e6 and is Dutch text must not exist outside
    the catalogues. Exempt: catalogue keys, and literals tests/i18n/allow-host*.txt lists (C9: logs, markers,
    matchers …; a round-2 agent adds its own in allow-host-r2<x>.txt, with a reason)."""
    import extract_rs  # noqa: PLC0415  (it imports this module: loaded here to avoid an import cycle)
    new = new if new is not None else new_literals(root)
    rules, problems = extract_rs.load_allow(root)
    rows, _ = load_catalogues(root)
    keys = {r.key for r in rows} | {re.sub(r"\.(one|other|zero)$", "", r.key) for r in rows}
    errors: list[str] = list(problems)
    warnings: list[str] = []
    stats: dict = {"new": 0, "classes": {}, "allowed": 0, "keys": 0}
    for n in new:
        if only and n.path not in only:
            continue
        stats["new"] += 1
        stats["classes"][n.cls] = stats["classes"].get(n.cls, 0) + 1
        if n.text in keys:
            stats["keys"] += 1
            continue
        if any(extract_rs.rule_matches(rule, n.lit, n.cls) for rule in rules):
            stats["allowed"] += 1
            continue
        where = f"{n.path}:{n.line}"
        if n.cls in RATCHET_DUTCH or (n.cls == "bilingual-pair" and n.lang in ("nl", "mixed")):
            errors.append(f"{where}: new Dutch literal outside the catalogues ({n.cls}, {n.lang}: {n.reason}): "
                          f"{n.text!r}; convert it with extract_rs.py, or allow-list it with a reason (C9)")
        elif n.cls == "user-visible-english":
            warnings.append(f"{where}: new English-only text ({n.reason}): {n.text!r}; Dutch users would see it "
                            "in English: convert it, or allow-list it when no user sees it")
        elif n.lang in ("nl", "mixed") and n.cls != "log" and re.search(r"[^\W\d_]{4,}", n.text):
            # (a word of four letters or more: not the language codes and join words "en", " en ", " of ")
            warnings.append(f"{where}: new literal with Dutch words, classed {n.cls} ({n.reason}): {n.text!r}")
    return errors, warnings, stats


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--freeze", action="store_true", help=f"rewrite {FROZEN} from git {BASE}")
    ap.add_argument("--summary", action="store_true")
    ap.add_argument("--paths", nargs="*", help="report only these files (host/src/…)")
    ap.add_argument("--warnings", action="store_true", help="print warnings too")
    ap.add_argument("--propose", action="store_true",
                    help="print a BASELINE-CHANGES.rust line for every uncovered literal (edit the reason!)")
    ap.add_argument("--new", action="store_true",
                    help="the ratchet only: list every new literal that is not technical, with its class")
    ap.add_argument("--no-ratchet", action="store_true", help="skip the ratchet (frozen literals only; faster)")
    a = ap.parse_args(argv)
    if a.freeze:
        n, stats = freeze()
        print(f"froze {n} literals ({stats['rust']} Rust, {stats['json']} JSON) in {stats['files']} files -> {FROZEN}")
        return 0
    only = set(a.paths) if a.paths else None
    if a.new:
        new = new_literals()
        errors, warnings, rs = ratchet(only=only, new=new)
        for n in new:
            if n.cls != "technical" and (not only or n.path in only):
                print(f"{n.path}:{n.line}\t#{n.ordinal}\t{n.cls}\t{n.lang}\t{esc(n.text)}")
        for e in errors:
            print("error:", e)
        for w in warnings:
            print("warning:", w)
        print(f"ratchet: {rs['new']} new literals {dict(sorted(rs['classes'].items()))}; {rs['keys']} catalogue "
              f"keys, {rs['allowed']} allow-listed; {len(errors)} errors, {len(warnings)} warnings")
        return 1 if errors else 0
    errors, warnings, stats = check(only=only)
    if a.propose:
        for f in stats["uncovered"]:
            print(f"{f.path}\t{f.ordinal}\t{f.sha[:SHA_PREFIX]}\t-\tTODO reason (was {esc(f.text)!r}, "
                  f"3b898e6 line {f.line}, {f.cls})")
        return 0
    rs = None
    if not a.no_ratchet:
        r_errors, r_warnings, rs = ratchet(only=only)
        errors += ["new: " + e for e in r_errors]
        warnings += ["new: " + w for w in r_warnings]
    for e in errors:
        print("error:", e)
    if a.warnings:
        for w in warnings:
            print("warning:", w)
    if a.summary:
        for path, s in stats["per_file"].items():
            if s["deficit"]:
                print(f"  {path}: {s['frozen']} frozen, {s['deficit']} replaced, {s['covered']} covered")
        if rs:
            print(f"  ratchet: {rs['new']} new literals {dict(sorted(rs['classes'].items()))}; "
                  f"{rs['keys']} catalogue keys, {rs['allowed']} allow-listed")
    print(f"drift_rust: {stats['frozen']} frozen literals in {stats['files']} files; {stats['present']} present, "
          f"{stats['mapped']} mapped or listed; {stats['rows']} catalogue rows; "
          + ("ratchet off; " if rs is None else f"{rs['new']} new literals; ")
          + f"{len(errors)} errors, {len(warnings)} warnings")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
