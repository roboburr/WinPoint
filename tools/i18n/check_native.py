#!/usr/bin/env python3
"""Native layer of the language checker: the Dutch and English texts of the GTK 4 programs in ``native/`` (the shell,
Calendar, the kit and the seven ``win11-gtk-*`` apps), read from their Rust sources and JSON tables. It took over
from the UI layer of the WebKit apps (``git show 6ebf169f:tools/i18n/check_ui.py``).

    python3 -B tools/i18n/check.py                            # all layers (this one included)
    python3 -B tools/i18n/check_native.py [--warnings]        # the native rules alone
    python3 -B tools/i18n/check_native.py --pairs [--json]    # every pair it reads: file:line kind key, nl → en
    python3 -B tools/i18n/check_native.py --uncovered         # Dutch-looking literals outside every pair

Pairs: one Dutch and one English text that the code picks by the language. The idioms of native/ it reads:

* ``call``   a call of a function, method or closure whose definition in native/ names its text parameters
  ``nl``/``en`` (``en_text``; or ``<x>_nl``/``<x>_en``, AppSpec::new's ``title_nl``), both of a ``str`` type, and
  passes two literals there, two ``&format!("…", …)`` or two ``if c { "…" } else { "…" }`` (branch by branch):
  ``kit::t``, ``kit::i18n::label``/``tooltip``, ``Ctx::t``, the shell's ``Shell::t``, Calendar's ``config::t``,
  the apps' own ``text``, ``pill``, ``head``, ``let with = |nl: &str, en_text: &str| …``. A definition in
  native/kit counts in every crate, any other one only in its own crate (native/<dir> or native/apps/<dir>), a
  closure only in its file; a method only for method calls, a function only for path calls. A path picks the
  definition: ``kit::…`` the kit's, ``Type::f`` the one in ``impl Type``, ``module::f`` the one in that file, a
  bare name the crate's own before the kit's; a name that is a text pair in one place and not in another (the kit's
  ``label(nl, en)`` and an app's ``label(text, class)``) is no pair;
* ``arm``    ``let (nl, en) = match key { "k" => ("…", "…"), … }`` (Task Manager's ``tr()``, the Start menu), and
  ``match (key, en) { ("k", false) => "…", ("k", true) => "…", … }`` (a tuple scrutinee with a language bool:
  ``en``, ``nl``, ``kit::en()``; arms with the same other elements pair up; a literal, ``format!`` or a call
  with literal arguments per arm);
* ``row``    a const or static table of tuples whose columns are named: by its doc comment (``/// (key, nl, en)``,
  ``/// (id, glyph, nl, en)``, a type alias's ``/// (example nl, example en, …)`` for a nested ``&[Alias]``
  column) or by a tuple pattern of the same arity next to the table's name in the same function
  (``.map(|(_, nl, en)| …)``, ``for (k, dnl, den) in KEYS``). A column ``nl``/``en`` or ``<x> nl``/``<x>nl``/
  ``<x>_nl`` pairs with the ``en`` column of the same ``<x>``; the key is the column ``key``/``id``/``k`` or else
  the first other literal column. Also rows: an inline ``for (view, glyph, nl, en, name) in [(…), …]``, and the
  struct literals ``Name { …, title: ("…", "…") }`` of a struct whose field is typed ``(&str, &str)``
  (``kit::AppSpec``, Instellingen' power actions and themes; key ``Name(<first literal field>).field``);
* ``macro``  a ``macro_rules!`` whose matcher names ``$nl``/``$en`` fragments, with a common prefix for more
  (``$dnl``/``$den``, lists ``[$($knl),*]``/``[$($ken),*]``): Instellingen' ``page!`` registry; the key is
  ``<macro>(<first literal>).<field>`` (the field the macro body assigns, e.g. ``page(update).description``);
* ``if``     ``if en { "…" } else { "…" }`` with a condition on the language (``en``, ``kit::en()``,
  ``lang == "en"``, ``== Some("nl")``, ``!en`` …) and in each branch a literal, a ``format!("…", …)`` or one call
  whose literal arguments pair up by position (``if nl { w(d, "dag", "dagen") } else { w(d, "day", "days") }``);
* ``table``  NL and EN arrays by name (``MONTHS_NL``/``MONTHS_EN``, ``NL_DAYS``/``EN_DAYS``, ``CAT_NL``/``CAT_EN``):
  element by element, or by key for ``(key, text)`` rows;
* ``json``   the ``{"nl": …, "en": …}`` objects of native/**/*.json (the Start menu's ``settings_pages.json``).

Test code is skipped (drift_rust.py's scan_file: ``#[cfg(test)]``, ``#[test]``), and so is native/kit/examples (the
kit's gallery, a developer example that is not shipped). The key of a call or an ``if`` is
``<enclosing function>/<nl text>`` (the source gives it no name; glossary-context.tsv places use ``fn/*`` globs).

Rules (errors unless named a warning):

* ``native-pair``          a pair with one empty half (``("", "")`` fallbacks are no pair);
* ``native-placeholders``  nl and en have the same ``{placeholders}`` (two ``format!`` strings: the same named
  arguments; rustc checks each against its own);
* ``native-table``         NL/EN tables of the same length and the same keys;
* ``native-dutch``         no Dutch in an English text: no en word is in tests/i18n/dutch-words.txt, and en is not a
  copy of a Dutch nl (the en-dutch rule of check_rust.py); a name or phrase that is Dutch on purpose (Alles,
  D-G5) is allow-listed in tests/i18n/allow-native.txt (``<file glob><TAB>en:<phrase><TAB>why``);
* ``native-apostrophe``    an en text uses the typographic apostrophe ’ (D-G23, as the WebKit UI did since R3-UI);
* ``native-allow``         tests/i18n/allow-native.txt is well-formed and each row still removes a phrase somewhere;
* ``native-json``          a JSON file of native/ parses;
* ``glossary``             check.py's glossary rule, with the context rows of tools/i18n/glossary-context.tsv
  whose places are native files (``native/…:<key glob>``); ``// glossary-ok: Term`` on the pair's line or the
  line above accepts one term there;
* ``article``              (a warning until round 3) no ``a``/``an`` before a placeholder in en;
* ``native-spelling``      (a warning) US spellings in en (D-G15: en-GB), as check_pairs.py;
* ``native-pair-expr``     (a warning) a call with a literal in one text parameter and an expression in the other;
* ``native-dutch-only``    (a warning) a literal outside every pair that reads as Dutch (scan_rust.py's word test):
  a text that may lack its English, or a search keyword or alias that is Dutch on purpose; ``--uncovered``
  lists them.

Pair keywords (search lists) are only checked for being present in both languages: they list synonyms on purpose.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import drift_rust as d  # noqa: E402  (the shared Rust lexer: lex, scan_file)
import paths  # noqa: E402

if "check" not in sys.modules and "__main__" in sys.modules and \
        Path(getattr(sys.modules["__main__"], "__file__", "") or "").name == "check.py":
    sys.modules["check"] = sys.modules["__main__"]
import check  # noqa: E402

ROOT = paths.ROOT
SKIP = ("native/kit/examples/",)
ALLOW = "tests/i18n/allow-native.txt"
DUTCH_WORDS = "tests/i18n/dutch-words.txt"

# rules of this layer that are warnings until the named step (merged into check.DEFERRED by the dispatcher)
DEFERRED = {
    "article": "round 3",
}

LANG_CODES = {"nl", "en", "nl-NL", "en-GB", "en-US", "nl_NL", "en_GB", "nl_NL.UTF-8", "en_GB.UTF-8"}
# US spellings that Windows 11 en-GB does not use (check_pairs.py's list, D-G15)
US_SPELLING = re.compile(r"\b(?:(?:minimi|maximi|organi|customi|personali|recogni|initiali|synchroni|optimi|authori|"
                         r"priori|summari|categori|finali)z(?:e|ed|es|ing|ation)|colors?|colored|centers?|centered|"
                         r"canceled|canceling|behaviors?|favorites?|gray)\b", re.IGNORECASE)
WORD = re.compile(r"[A-Za-zÀ-ÖØ-öø-ÿ]+(?:['’][a-zë]+)?")             # as tools/i18n/check_rust.py
OPEN, CLOSE = "([{", ")]}"
FMT = re.compile(r"\{[A-Za-z_]\w*(?::[^{}]*)?\}")                    # format! arguments by name


@dataclass
class Pair:
    file: str                       # below the repo
    line: int
    kind: str                       # call, arm, row, macro, if, table, json
    key: str                        # the place in the file (context rows match it)
    nl: str | list[str] | None
    en: str | list[str] | None
    ok: set[str] = field(default_factory=set)       # glossary-ok terms of a comment on the line or the line above
    fmt: bool = False               # two format! strings: only their named {arguments} are compared

    @property
    def keywords(self) -> bool:
        return isinstance(self.nl, list) or isinstance(self.en, list)


@dataclass
class Sig:
    """A fn definition of native/; nl/en are the indexes of its text parameters (None: no text pair)."""
    crate: str
    file: str
    name: str
    method: bool
    arity: int
    nl: int | None
    en: int | None
    closure: bool = False           # a `let name = |nl: &str, en: &str| …` closure: for bare calls in its file
    owner: str = ""                 # the type of the impl block it is defined in (AppSpec::new)


@dataclass
class Source:
    rel: str
    text: str
    toks: list
    lines: list[str]
    lits: dict[int, object]         # token index → drift_rust Lit (non-test literals only)
    close: dict[int, int]           # opening bracket index → its closing index
    fn_at: list[str]                # token index → enclosing fn ("" at item level)
    used: set[int] = field(default_factory=set)      # literal tokens that belong to a pair


# ------------------------------------------------------------------------------------------------------------------
# reading the tree
# ------------------------------------------------------------------------------------------------------------------

def crate_of(rel: str) -> str:
    parts = rel.split("/")
    return "/".join(parts[:3]) if len(parts) > 3 and parts[1] == "apps" else "/".join(parts[:2])


def rust_files(root: Path) -> dict[str, str]:
    base = Path(root) / paths.NATIVE_REL
    out = {}
    for p in sorted(base.rglob("*.rs")) if base.is_dir() else []:
        rel = p.relative_to(root).as_posix()
        if "/target/" in rel or rel.startswith(SKIP):
            continue
        out[rel] = p.read_text(encoding="utf-8")
    return out


def json_files(root: Path) -> dict[str, str]:
    base = Path(root) / paths.NATIVE_REL
    out = {}
    for p in sorted(base.rglob("*.json")) if base.is_dir() else []:
        rel = p.relative_to(root).as_posix()
        if "/target/" not in rel and not rel.startswith(SKIP):
            out[rel] = p.read_text(encoding="utf-8")
    return out


def test_files(scanned: dict[str, object]) -> set[str]:
    """Files that only test code reaches (a #[cfg(test)] file, or a child of a #[cfg(test)] mod x;)."""
    tests = {rel for rel, fl in scanned.items() if fl.whole_file_test}
    queue = [d.normalize(c) for rel, fl in scanned.items() for m in fl.mods if m.test
             for c in d.module_children(rel, m)]
    while queue:
        c = queue.pop()
        if c in tests or c not in scanned:
            continue
        tests.add(c)
        queue += [d.normalize(k) for m in scanned[c].mods for k in d.module_children(c, m)]
    return tests


def brackets(toks) -> dict[int, int]:
    close, stack = {}, []
    for i, t in enumerate(toks):
        if t.kind != "punct":
            continue
        if t.text in OPEN:
            stack.append(i)
        elif t.text in CLOSE and stack:
            close[stack.pop()] = i
    return close


def enclosing_fns(toks, close: dict[int, int]) -> list[str]:
    out = [""] * len(toks)
    spans = []
    for i, t in enumerate(toks):
        if t.kind == "ident" and t.text == "fn" and i + 1 < len(toks) and toks[i + 1].kind == "ident":
            depth = 0
            for j in range(i + 2, len(toks)):
                x = toks[j]
                if x.kind == "punct" and x.text in "([":
                    depth += 1
                elif x.kind == "punct" and x.text in ")]":
                    depth -= 1
                elif x.text == ";" and depth == 0:
                    break                                   # a declaration without a body
                elif x.text == "{" and depth == 0:
                    spans.append((j, close.get(j, len(toks) - 1), toks[i + 1].text))
                    break
    for a, b, name in spans:                                # outer first: inner functions overwrite
        for k in range(a, b + 1):
            out[k] = name
    return out


def load(root: Path) -> tuple[list[Source], dict[str, str]]:
    files = rust_files(root)
    scanned = {rel: d.scan_file(rel, src) for rel, src in files.items()}
    tests = test_files(scanned)
    out = []
    for rel, fl in scanned.items():
        if rel in tests:
            continue
        close = brackets(fl.toks)
        out.append(Source(rel, fl.src, fl.toks, fl.src.split("\n"), {lit.tok: lit for lit in fl.lits}, close,
                          enclosing_fns(fl.toks, close)))
    return out, json_files(root)


# ------------------------------------------------------------------------------------------------------------------
# token helpers
# ------------------------------------------------------------------------------------------------------------------

def split_top(toks, a: int, b: int) -> list[tuple[int, int]]:
    """The comma-separated parts of toks[a:b] (inside one bracket pair), as (start, end) index ranges."""
    parts, depth, start = [], 0, a
    for k in range(a, b):
        t = toks[k]
        if t.kind == "punct" and t.text in OPEN:
            depth += 1
        elif t.kind == "punct" and t.text in CLOSE:
            depth -= 1
        elif depth == 0 and t.text == ",":
            parts.append((start, k))
            start = k + 1
    if start < b:
        parts.append((start, b))
    return [(x, y) for x, y in parts if x < y]


def lit_at(src: Source, a: int, b: int) -> int | None:
    """The token index when toks[a:b] is one non-test string literal (a leading & allowed)."""
    if b - a == 2 and src.toks[a].text == "&":
        a += 1
    if b - a == 1 and src.toks[a].kind == "str" and a in src.lits:
        return a
    return None


def text_of(src: Source, k: int) -> str:
    return src.toks[k].text


def place(src: Source, at: int, nl: int) -> str:
    """The key of a call or an if: <enclosing function>/<nl text> (no symbolic name in the source)."""
    return f"{src.fn_at[at] or '-'}/{src.toks[nl].text}"


def glossary_ok(src: Source, line: int) -> set[str]:
    out: set[str] = set()
    for no in (line - 1, line):
        if 1 <= no <= len(src.lines):
            m = re.search(r"//.*glossary-ok:\s*(.+)$", src.lines[no - 1])
            if m:
                out |= {t.strip() for t in m.group(1).split(",") if t.strip()}
    return out


def doc_above(src: Source, line: int) -> str:
    """The comment lines directly above a line (/// and //), joined."""
    out = []
    no = line - 1
    while no >= 1 and src.lines[no - 1].strip().startswith("//"):
        out.insert(0, src.lines[no - 1].strip().lstrip("/!").strip())
        no -= 1
    return " ".join(out)


def lang_of(name: str) -> tuple[str, str] | None:
    """A column or parameter name → (prefix, "nl"|"en"): nl, en, en_text, gnl, gen_, dnl, example nl, title_en …"""
    n = name.strip().strip("_").lower()
    for lang in ("nl", "en"):
        if n == lang or n == f"{lang}_text":
            return "", lang
        for sep in (" ", "_", ""):
            if n.endswith(sep + lang) and len(n) > len(sep + lang):
                return n[:-len(sep + lang)].strip(" _"), lang
            if sep and n.startswith(lang + sep):
                return n[len(lang + sep):].strip(" _"), lang
    return None


def columns(names: list[str]) -> tuple[list[tuple[int, int, str]], int | None]:
    """[(nl column, en column, prefix)] and the key column of a legend."""
    langs = {i: lang_of(n) for i, n in enumerate(names)}
    pairs = []
    for i, li in langs.items():
        if li and li[1] == "nl":
            j = next((j for j, lj in langs.items() if lj == (li[0], "en")), None)
            if j is not None:
                pairs.append((i, j, li[0]))
    used = {i for i, j, _ in pairs} | {j for _, j, _ in pairs}
    key = next((i for i, n in enumerate(names) if n.strip().lower() in ("key", "id", "k") and i not in used), None)
    if key is None:
        key = next((i for i in range(len(names)) if i not in used and not langs[i]), None)
    return pairs, key


LEGEND = re.compile(r"\(([^()]*)\)")


def legend(text: str) -> list[str] | None:
    """The first parenthesised column list of a doc comment that names an nl and an en column."""
    for m in LEGEND.finditer(text):
        names = [n.strip() for n in m.group(1).split(",")]
        if len(names) >= 2 and columns(names)[0]:
            return names
    return None


# ------------------------------------------------------------------------------------------------------------------
# idioms
# ------------------------------------------------------------------------------------------------------------------

def impl_owners(src: Source) -> list[str]:
    """token index → the type of the enclosing impl block ("" outside one)."""
    toks = src.toks
    out = [""] * len(toks)
    for i, t in enumerate(toks):
        if t.text != "impl" or t.kind != "ident":
            continue
        j, ids = i + 1, []
        while j < len(toks) and toks[j].text not in ("{", ";"):
            if toks[j].text == "for":
                ids = []
            elif toks[j].text == "<":                       # skip generics
                depth = 0
                while j < len(toks):
                    depth += {"<": 1, ">": -1, ">>": -2}.get(toks[j].text, 0)
                    if depth <= 0:
                        break
                    j += 1
            elif toks[j].kind == "ident" and toks[j].text not in ("where", "dyn"):
                ids.append(toks[j].text)
            j += 1
        if j < len(toks) and toks[j].text == "{" and j in src.close and ids:
            for k in range(j, src.close[j] + 1):
                out[k] = ids[-1]
    return out


def signatures(sources: list[Source]) -> list[Sig]:
    out = []
    for src in sources:
        toks = src.toks
        owners = impl_owners(src)
        for i, t in enumerate(toks):
            if not (t.kind == "ident" and t.text == "fn" and i + 1 < len(toks) and toks[i + 1].kind == "ident"):
                continue
            j, depth = i + 2, 0
            if j < len(toks) and toks[j].text == "<":               # generics, with >> as two closes
                while j < len(toks):
                    depth += {"<": 1, ">": -1, ">>": -2}.get(toks[j].text, 0)
                    j += 1
                    if depth <= 0:
                        break
            if j >= len(toks) or toks[j].text != "(" or j not in src.close:
                continue
            params, method = [], False
            angle, depth, start = 0, 0, j + 1
            end = src.close[j]
            for k in range(j + 1, end + 1):
                x = toks[k].text
                if toks[k].kind == "punct":
                    angle += {"<": 1, ">": -1, ">>": -2}.get(x, 0)
                    depth += (x in OPEN) - (x in CLOSE)
                if k == end or (x == "," and angle <= 0 and depth == 0):
                    part = toks[start:k]
                    start = k + 1
                    if not part:
                        continue
                    names = [p.text for p in part if p.kind == "ident" and p.text != "mut"]
                    if names and names[0] == "self":
                        method = True
                        continue
                    params.append((names[0] if names else "", "str" in names[1:]))
            cols, _key = columns([name if typed else "" for name, typed in params])
            nl, en = cols[0][:2] if cols else (None, None)
            out.append(Sig(crate_of(src.rel), src.rel, toks[i + 1].text, method, len(params), nl, en,
                           owner=owners[i]))
        # closures: let name = [move] |nl: &str, en_text: &str| …
        for i in range(len(toks) - 4):
            if not (toks[i].text == "let" and toks[i + 1].kind == "ident" and toks[i + 2].text == "="):
                continue
            j = i + 3 + (toks[i + 3].text == "move")
            if toks[j].text != "|":
                continue
            k = j + 1
            while k < len(toks) and toks[k].text != "|":
                k += 1
            names, typed = [], []
            for a, b in split_top(toks, j + 1, k):
                ids = [x.text for x in toks[a:b] if x.kind == "ident" and x.text != "mut"]
                names.append(ids[0] if ids else "")
                typed.append(len(ids) == 1 or "str" in ids[1:])
            nl = next((n for n, name in enumerate(names) if typed[n] and lang_of(name) == ("", "nl")), None)
            en = next((n for n, name in enumerate(names) if typed[n] and lang_of(name) == ("", "en")), None)
            if nl is not None and en is not None:
                out.append(Sig(crate_of(src.rel), src.rel, toks[i + 1].text, False, len(names), nl, en, True))
    return out


def resolve(sigs: list[Sig], crate: str, file: str, path: list[str], name: str, method: bool,
            arity: int) -> Sig | None:
    """The text-pair definition a call means, or None: kit::… is the kit's; a bare or module path (util::, layout::,
    crate::…) its own crate's when it defines the name, else the kit's (use kit::…, the prelude); a module segment
    picks the definition in that file. Mixed candidates (a text pair and another function) are no pair."""
    def pick(cands: list[Sig]) -> Sig | None:
        if path and path[-1][:1].isupper() and path[-1] != "Self":
            cands = [s for s in cands if s.owner == path[-1]]          # Type::new
        elif path and path[-1] not in ("kit", "crate", "super", "self"):
            own = [s for s in cands if Path(s.file).stem == path[-1] or Path(s.file).parent.name == path[-1]]
            cands = own or cands
        if cands and all(s.nl is not None for s in cands):
            return cands[0]
        return None

    same = [s for s in sigs if s.name == name and s.method == method and s.arity == arity and not s.closure]
    if not path and not method:
        local = [s for s in sigs if s.closure and s.file == file and s.name == name and s.arity == arity]
        if local:
            return local[0]
    if path[:1] == ["kit"]:
        return pick([s for s in same if s.crate == "native/kit"])
    own = [s for s in same if s.crate == crate]
    if own:
        return pick(own)
    return pick([s for s in same if s.crate == "native/kit"])


def calls(src: Source, sigs: list[Sig], pairs: list[Pair], rep: check.Report | None) -> None:
    toks = src.toks
    crate = crate_of(src.rel)
    names = {s.name for s in sigs if s.nl is not None}
    for i, t in enumerate(toks):
        if t.text != "(" or t.kind != "punct" or i == 0 or toks[i - 1].kind != "ident" or i not in src.close:
            continue
        name = toks[i - 1].text
        if name not in names or (i >= 2 and toks[i - 2].text == "fn"):
            continue
        method = i >= 2 and toks[i - 2].text == "."
        path, k = [], i - 2
        while not method and k >= 1 and toks[k].text == "::" and toks[k - 1].kind == "ident":
            path.insert(0, toks[k - 1].text)
            k -= 2
        args = split_top(toks, i + 1, src.close[i])
        sig = resolve(sigs, crate, src.rel, path, name, method, len(args))
        if sig is None:
            continue
        a, b = lit_at(src, *args[sig.nl]), lit_at(src, *args[sig.en])
        fa, fb = format_literal(src, *args[sig.nl]), format_literal(src, *args[sig.en])
        ia, ib = if_literals(src, *args[sig.nl]), if_literals(src, *args[sig.en])
        if ia and ib and ia[0] == ib[0]:
            # t(if c { "A" } else { "B" }, if c { "C" } else { "D" }): A/C and B/D
            for x, y in ((ia[1], ib[1]), (ia[2], ib[2])):
                src.used |= {x, y}
                pairs.append(Pair(src.rel, toks[x].line, "call", place(src, i, x), text_of(src, x), text_of(src, y),
                                  glossary_ok(src, toks[x].line)))
        elif fa is not None and fb is not None:
            # t(&format!("…", x), &format!("…", x))
            src.used |= {fa, fb}
            pairs.append(Pair(src.rel, toks[fa].line, "call", place(src, i, fa), text_of(src, fa), text_of(src, fb),
                              glossary_ok(src, toks[fa].line), fmt=True))
        elif a is not None and b is not None:
            src.used |= {a, b}
            line = toks[a].line
            pairs.append(Pair(src.rel, line, "call", place(src, i, a), text_of(src, a), text_of(src, b),
                              glossary_ok(src, line)))
        elif (a is None) != (b is None) and rep is not None:
            k = a if a is not None else b
            rep.add("native-pair-expr", src.rel, toks[k].line, f"{name}(): a literal {'nl' if a is not None else 'en'}"
                    f" text {text_of(src, k)!r} with a computed {'en' if a is not None else 'nl'} text", level="warning")


def format_literal(src: Source, a: int, b: int) -> int | None:
    """toks[a:b] is `[&]format!("…", …)`: the literal."""
    toks = src.toks
    if a < b and toks[a].text == "&":
        a += 1
    if b - a >= 4 and toks[a].text == "format" and toks[a + 1].text == "!" and toks[a + 2].text == "(" \
            and src.close.get(a + 2) == b - 1:
        parts = split_top(toks, a + 3, b - 1)
        return lit_at(src, *parts[0]) if parts else None
    return None


def if_literals(src: Source, a: int, b: int) -> tuple[tuple[str, ...], int, int] | None:
    """toks[a:b] is `if COND { "x" } else { "y" }`: (COND's texts, x, y)."""
    toks = src.toks
    if toks[a].text != "if":
        return None
    j = a + 1
    while j < b and toks[j].text != "{":
        j += 1
    e = src.close.get(j)
    if e is None or e + 2 >= b or toks[e + 1].text != "else" or toks[e + 2].text != "{" \
            or src.close.get(e + 2) != b - 1:
        return None
    x, y = lit_at(src, j + 1, e), lit_at(src, e + 3, b - 1)
    if x is None or y is None:
        return None
    return tuple(t.text for t in toks[a + 1:j]), x, y


def arms(src: Source, pairs: list[Pair]) -> None:
    """let (nl, en) = match … { "k" => ("…", "…"), … }"""
    toks = src.toks
    for i in range(len(toks) - 8):
        if not (toks[i].text == "let" and toks[i + 1].text == "(" and toks[i + 3].text == ","
                and toks[i + 5].text == ")" and toks[i + 6].text == "=" and toks[i + 7].text == "match"):
            continue
        a, b = lang_of(toks[i + 2].text), lang_of(toks[i + 4].text)
        if not (a and b and {a[1], b[1]} == {"nl", "en"}):
            continue
        en_first = a[1] == "en"
        k = i + 8
        while k < len(toks) and not (toks[k].text == "{" and toks[k].kind == "punct"):
            k = src.close.get(k, k) + 1 if toks[k].text in "([" else k + 1
        if k not in src.close:
            continue
        end, j = src.close[k], k + 1
        while j < end:
            arrow = j
            while arrow < end and toks[arrow].text != "=>":
                arrow = src.close.get(arrow, arrow) + 1 if toks[arrow].text in OPEN else arrow + 1
            if arrow >= end:
                break
            keys = [toks[q].text for q in range(j, arrow) if toks[q].kind == "str"]
            body = arrow + 1
            stop = body
            while stop < end and toks[stop].text != ",":
                stop = src.close.get(stop, stop) + 1 if toks[stop].text in OPEN else stop + 1
            if toks[body].text == "(" and src.close.get(body) == stop - 1:
                parts = split_top(toks, body + 1, stop - 1)
                if len(parts) == 2:
                    x, y = lit_at(src, *parts[0]), lit_at(src, *parts[1])
                    if x is not None and y is not None:
                        nl, en = (y, x) if en_first else (x, y)
                        src.used |= {x, y}
                        if text_of(src, nl) or text_of(src, en):
                            line = toks[x].line
                            pairs.append(Pair(src.rel, line, "arm", "|".join(keys) or "_", text_of(src, nl),
                                              text_of(src, en), glossary_ok(src, line)))
            j = stop + 1


def items(src: Source) -> list[tuple[str, int, int, int, int]]:
    """const/static items: (name, index of `const`, type start, type end, initializer start)."""
    toks, out = src.toks, []
    for i in range(len(toks) - 3):
        if toks[i].text in ("const", "static") and toks[i + 1].kind == "ident" and toks[i + 2].text == ":":
            j = i + 3
            while j < len(toks) and toks[j].text != "=" and toks[j].text != ";":
                j = src.close.get(j, j) + 1 if toks[j].text in OPEN else j + 1
            if j < len(toks) and toks[j].text == "=":
                out.append((toks[i + 1].text, i, i + 3, j, j + 1))
    return out


def array_at(src: Source, k: int) -> tuple[int, int] | None:
    """The inside (start, end) of an array literal at toks[k] (`&[…]`, `[…]`, `&mut […]`)."""
    toks = src.toks
    while k < len(toks) and toks[k].text in ("&", "mut"):
        k += 1
    if k < len(toks) and toks[k].text == "[" and k in src.close:
        return k + 1, src.close[k]
    return None


def aliases(src: Source) -> dict[str, list[str]]:
    """type Alias = (…); with a column legend in its doc comment."""
    toks, out = src.toks, {}
    for i in range(len(toks) - 2):
        if toks[i].text == "type" and toks[i + 1].kind == "ident" and toks[i + 2].text == "=":
            names = legend(doc_above(src, toks[i].line))
            if names:
                out[toks[i + 1].text] = names
    return out


def pattern_legends(src: Source) -> list[tuple[int, list[str]]]:
    """(token index, names) of tuple patterns that name an nl and an en element: |(_, nl, en)|, for (k, dnl, den)."""
    toks, out = src.toks, []
    for i, t in enumerate(toks):
        if t.text != "(" or i not in src.close:
            continue
        inner = toks[i + 1:src.close[i]]
        if not inner or len(inner) % 2 == 0:
            continue
        names = [x.text for x in inner[0::2]]
        if all(x.kind == "ident" for x in inner[0::2]) and all(x.text == "," for x in inner[1::2]) \
                and len(names) >= 2 and columns(names)[0]:
            out.append((i, names))
    return out


def rows(src: Source, pairs: list[Pair], rep: check.Report | None) -> None:
    toks = src.toks
    alias = aliases(src)
    patterns = pattern_legends(src)
    for name, at, ta, tb, init in items(src):
        arr = array_at(src, init)
        if arr is None:
            continue
        elems = split_top(toks, *arr)
        tuples = [(a, b) for a, b in elems if toks[a].text == "(" and src.close.get(a) == b - 1]
        if not tuples or len(tuples) != len(elems):
            continue
        arity = len(split_top(toks, tuples[0][0] + 1, tuples[0][1] - 1))
        names = legend(doc_above(src, toks[at].line))
        if names is None or len(names) != arity:
            uses = [q for q, t in enumerate(toks) if t.kind == "ident" and t.text == name and q != at + 1]
            names = next((ns for q, ns in patterns if len(ns) == arity and src.fn_at[q]
                          and any(abs(u - q) <= 40 and src.fn_at[u] == src.fn_at[q] for u in uses)), None)
        # a column typed &[Alias] holds a nested table with the alias's legend
        elem = array_at(src, ta)                            # the element type of &[(…)] or [(…); N]
        col_types = []
        if elem and toks[elem[0]].text == "(" and elem[0] in src.close:
            col_types = split_top(toks, elem[0] + 1, src.close[elem[0]])
        nested = {}
        for c, (a, b) in enumerate(col_types):
            ids = [toks[q].text for q in range(a, b) if toks[q].kind == "ident"]
            if ids and ids[-1] in alias:
                nested[c] = alias[ids[-1]]
        if names is None and not nested:
            continue
        table_rows(src, name, tuples, names, nested, pairs, rep)


def table_rows(src: Source, name: str, tuples, names, nested, pairs: list[Pair], rep) -> None:
    toks = src.toks
    cols, key_col = columns(names) if names else ([], None)
    for n, (a, b) in enumerate(tuples):
        parts = split_top(toks, a + 1, b - 1)
        key_tok = lit_at(src, *parts[key_col]) if key_col is not None and key_col < len(parts) else None
        key = text_of(src, key_tok) if key_tok is not None else f"{name}#{n}"
        for i, j, prefix in cols:
            if max(i, j) >= len(parts):
                continue
            x, y = lit_at(src, *parts[i]), lit_at(src, *parts[j])
            if x is None or y is None:
                continue
            src.used |= {x, y}
            line = toks[x].line
            pairs.append(Pair(src.rel, line, "row", key + (f".{prefix}" if prefix else ""), text_of(src, x),
                              text_of(src, y), glossary_ok(src, line)))
        for c, legend_names in nested.items():
            if c < len(parts):
                inner = array_at(src, parts[c][0])
                if inner:
                    elems = [(p, q) for p, q in split_top(toks, *inner) if toks[p].text == "(" and src.close.get(p) == q - 1]
                    table_rows(src, f"{key}", elems, legend_names, {}, pairs, rep)


def lang_pair_name(name: str) -> tuple[str, str] | None:
    parts = name.split("_")
    for lang, other in (("NL", "EN"), ("EN", "NL")):
        if lang in parts:
            return lang.lower(), "_".join(other if p == lang else p for p in parts)
    return None


def tables(src: Source, pairs: list[Pair], rep: check.Report | None) -> None:
    """NAME_NL / NAME_EN arrays: element by element, or by key for (key, text) rows."""
    toks = src.toks
    found = {name: init for name, _at, _ta, _tb, init in items(src)}
    for name, init in found.items():
        lp = lang_pair_name(name)
        if not lp or lp[0] != "nl" or lp[1] not in found:
            continue
        a, b = array_at(src, init), array_at(src, found[lp[1]])
        if a is None or b is None:
            continue
        ea, eb = split_top(toks, *a), split_top(toks, *b)
        base = "_".join(p for p in name.split("_") if p != "NL")

        def entries(elems) -> list[tuple[str | None, int | None]]:
            out = []
            for p, q in elems:
                if toks[p].text == "(" and src.close.get(p) == q - 1:
                    kv = split_top(toks, p + 1, q - 1)
                    if len(kv) == 2:
                        k, v = lit_at(src, *kv[0]), lit_at(src, *kv[1])
                        out.append((text_of(src, k) if k is not None else None, v))
                        continue
                out.append((None, lit_at(src, p, q)))
            return out

        na, nb = entries(ea), entries(eb)
        keyed = all(k is not None for k, _ in na + nb)
        line = toks[ea[0][0]].line if ea else toks[init].line
        if keyed:
            ka, kb = [k for k, _ in na], [k for k, _ in nb]
            if rep is not None and set(ka) != set(kb):
                rep.add("native-table", src.rel, line, f"{name}/{lp[1]}: keys only in {name} "
                        f"{sorted(set(ka) - set(kb))[:8]}, only in {lp[1]} {sorted(set(kb) - set(ka))[:8]}")
            en_of = dict(nb)
            matched = [(k, v, en_of.get(k)) for k, v in na]
        else:
            if rep is not None and len(na) != len(nb):
                rep.add("native-table", src.rel, line, f"{name} has {len(na)} entries, {lp[1]} {len(nb)}")
            matched = [(f"{base}[{n}]", v, w) for n, ((_, v), (_, w)) in enumerate(zip(na, nb))]
        for key, v, w in matched:
            if v is None or w is None:
                continue
            src.used |= {v, w}
            pairs.append(Pair(src.rel, toks[v].line, "table", key, text_of(src, v), text_of(src, w),
                              glossary_ok(src, toks[v].line)))


def macros(src: Source, pairs: list[Pair]) -> None:
    """macro_rules! NAME { (… $nl:expr, $en:expr …) => { … field: ($nl, $en) … } } and its NAME!(…) calls."""
    toks = src.toks
    for i in range(len(toks) - 3):
        if not (toks[i].text == "macro_rules" and toks[i + 1].text == "!" and toks[i + 2].kind == "ident"
                and toks[i + 3].text in "({" and i + 3 in src.close):
            continue
        name = toks[i + 2].text
        k = i + 4
        if toks[k].text != "(" or k not in src.close:
            continue
        mend = src.close[k]
        params: dict[str, int] = {}
        for n, (a, b) in enumerate(split_top(toks, k + 1, mend)):
            for q in range(a, b - 1):
                if toks[q].text == "$" and toks[q + 1].kind == "ident":
                    params[toks[q + 1].text] = n
                    break
        body = mend + 2 if toks[mend + 1].text == "=>" else None
        groups = []
        for pname, n in params.items():
            lp = lang_of(pname)
            if lp and lp[1] == "nl":
                other = next((p for p in params if lang_of(p) == (lp[0], "en")), None)
                if other is not None:
                    groups.append((n, params[other], field_name(src, body, pname) or lp[0] or "text"))
        if not groups:
            continue
        for c in range(len(toks) - 2):
            if toks[c].text == name and toks[c + 1].text == "!" and toks[c + 2].text == "(" and c != i + 2 \
                    and c + 2 in src.close:
                args = split_top(toks, c + 3, src.close[c + 2])
                first = next((lit_at(src, a, b) for a, b in args if lit_at(src, a, b) is not None), None)
                ident = text_of(src, first) if first is not None else f"#{c}"
                for nl_i, en_i, fname in groups:
                    if max(nl_i, en_i) >= len(args):
                        continue
                    x, y = macro_value(src, *args[nl_i]), macro_value(src, *args[en_i])
                    if x is None or y is None:
                        continue
                    line = toks[args[nl_i][0]].line
                    pairs.append(Pair(src.rel, line, "macro", f"{name}({ident}).{fname}", x[0], y[0],
                                      glossary_ok(src, line)))
                    src.used |= x[1] | y[1]


def field_name(src: Source, body: int | None, param: str) -> str | None:
    """The struct field the macro body assigns the parameter to: `title: ($nl, $en)` → title."""
    if body is None or body not in src.close:
        return None
    toks = src.toks
    for q in range(body, src.close[body]):
        if toks[q].text == "$" and toks[q + 1].text == param:
            depth = 0
            for r in range(q - 1, body, -1):
                if toks[r].text in CLOSE:
                    depth += 1
                elif toks[r].text in OPEN:
                    depth -= 1
                elif depth <= 0 and toks[r].text == ":" and toks[r - 1].kind == "ident" and toks[r - 2].text != ":":
                    return toks[r - 1].text
            return None
    return None


def macro_value(src: Source, a: int, b: int) -> tuple[str | list[str], set[int]] | None:
    k = lit_at(src, a, b)
    if k is not None:
        return text_of(src, k), {k}
    arr = array_at(src, a)
    if arr is not None and src.close.get(a if src.toks[a].text == "[" else a + 1, -1) == b - 1:
        ks = [lit_at(src, p, q) for p, q in split_top(src.toks, *arr)]
        if all(x is not None for x in ks):
            return [text_of(src, x) for x in ks], set(ks)
    return None


def lang_condition(toks) -> str | None:
    """'en' when the then-branch of `if <toks>` is English, 'nl' when it is Dutch, None when it is no language test."""
    texts = [t.text for t in toks]
    strs = [t.text for t in toks if t.kind == "str"]
    idents = [t.text for t in toks if t.kind == "ident"]
    if "&&" in texts or "||" in texts:
        return None
    neg = texts[:1] == ["!"]
    if strs:
        lang = next((s for s in strs if s in ("en", "nl")), None)
        if lang is None or not ({"==", "!="} & set(texts)):
            return None
        if "!=" in texts:
            lang = "nl" if lang == "en" else "en"
        return lang
    last = idents[-1] if idents else ""
    if last in ("en", "is_en", "english", "nl", "dutch") and len(idents) <= 4:
        lang = "en" if last in ("en", "is_en", "english") else "nl"
        return ("nl" if lang == "en" else "en") if neg else lang
    return None


def branch_values(src: Source, a: int, b: int) -> dict[str, int]:
    """The literals of one branch toks[a:b] by position: a literal ("" ; with .into() and the like, after return),
    format!("…", …) ("fmt"), or a call f(…, "…", …) ("f#<arg>")."""
    toks = src.toks
    if a < b and toks[a].text == "return":
        a += 1
    while b - a >= 4 and [t.text for t in toks[b - 4:b]][0] == "." and toks[b - 2].text == "(" \
            and toks[b - 1].text == ")" and toks[b - 3].text in ("into", "to_owned", "to_string"):
        b -= 4
    k = lit_at(src, a, b)
    if k is not None:
        return {"": k}
    if b - a >= 4 and toks[a].kind == "ident" and toks[a + 1].text == "!" and toks[a + 2].text == "(" \
            and src.close.get(a + 2) == b - 1:
        parts = split_top(toks, a + 3, b - 1)
        k = lit_at(src, *parts[0]) if parts else None
        return {"fmt": k} if toks[a].text == "format" and k is not None else {}
    q = a
    while q + 2 < b and toks[q].kind == "ident" and toks[q + 1].text in ("::", "."):
        q += 2
    if q + 1 < b and toks[q].kind == "ident" and toks[q + 1].text == "(" and src.close.get(q + 1) == b - 1:
        out = {}
        for n, (x, y) in enumerate(split_top(toks, q + 2, b - 1)):
            k = lit_at(src, x, y)
            if k is not None:
                out[f"{toks[q].text}#{n}"] = k
        return out
    return {}


def ifs(src: Source, pairs: list[Pair]) -> None:
    """if <language> { … } else { … }: a literal, format!("…") or the literal arguments of one call per branch."""
    toks = src.toks
    for i, t in enumerate(toks):
        if t.text != "if" or t.kind != "ident":
            continue
        j = i + 1
        while j < len(toks) and toks[j].text != "{":
            j = src.close.get(j, j) + 1 if toks[j].text in ("(", "[") else j + 1
        if j >= len(toks) or j not in src.close:
            continue
        side = lang_condition(toks[i + 1:j])
        e = src.close[j]
        if side is None or e + 2 >= len(toks) or toks[e + 1].text != "else" or toks[e + 2].text != "{" \
                or e + 2 not in src.close:
            continue
        then, other = branch_values(src, j + 1, e), branch_values(src, e + 3, src.close[e + 2])
        for pos in then.keys() & other.keys():
            a, b = then[pos], other[pos]
            nl, en = (b, a) if side == "en" else (a, b)
            if text_of(src, nl) in LANG_CODES and text_of(src, en) in LANG_CODES:
                continue
            src.used |= {a, b}
            line = toks[a].line
            pairs.append(Pair(src.rel, line, "if", place(src, i, nl), text_of(src, nl), text_of(src, en),
                              glossary_ok(src, line), fmt=pos == "fmt"))


def lang_matches(src: Source, pairs: list[Pair]) -> None:
    """match (key, en) { ("k", false) => "…", ("k", true) => "…", … }: a scrutinee tuple with a language bool
    (en, is_en, nl, en(), kit::en() …); arms with the same other elements pair up."""
    toks = src.toks
    for i, t in enumerate(toks):
        if t.text != "match" or i + 1 >= len(toks) or toks[i + 1].text != "(" or i + 1 not in src.close:
            continue
        close = src.close[i + 1]
        if close + 1 >= len(toks) or toks[close + 1].text != "{" or close + 1 not in src.close:
            continue
        elems = split_top(toks, i + 2, close)
        lang_at, side = None, None
        for n, (a, b) in enumerate(elems):
            ids = [x.text for x in toks[a:b] if x.kind == "ident"]
            texts = [x.text for x in toks[a:b]]
            if ids and ids[-1] in ("en", "is_en", "nl") and (texts == [ids[-1]] or texts[-2:] == ["(", ")"]):
                lang_at, side = n, ("nl" if ids[-1] == "nl" else "en")
        if lang_at is None or len(elems) < 2:
            continue
        end, j = src.close[close + 1], close + 2
        by_key: dict[tuple, dict[str, dict[str, int]]] = {}
        while j < end:
            arrow = j
            while arrow < end and toks[arrow].text != "=>":
                arrow = src.close.get(arrow, arrow) + 1 if toks[arrow].text in OPEN else arrow + 1
            if arrow >= end:
                break
            stop = arrow + 1
            while stop < end and toks[stop].text != ",":
                stop = src.close.get(stop, stop) + 1 if toks[stop].text in OPEN else stop + 1
            if toks[j].text == "(" and src.close.get(j) == arrow - 1:
                pat = split_top(toks, j + 1, arrow - 1)
                if len(pat) == len(elems):
                    flag = " ".join(x.text for x in toks[pat[lang_at][0]:pat[lang_at][1]])
                    if flag in ("true", "false"):
                        key = tuple(" ".join(x.text for x in toks[a:b]) for n, (a, b) in enumerate(pat) if n != lang_at)
                        lang = side if flag == "true" else ("nl" if side == "en" else "en")
                        body_end = stop
                        if toks[arrow + 1].text == "{" and src.close.get(arrow + 1) == stop - 1:
                            body = (arrow + 2, stop - 1)
                        else:
                            body = (arrow + 1, body_end)
                        vals = branch_values(src, *body)
                        if vals:
                            by_key.setdefault(key, {})[lang] = vals
            j = stop + 1
        for key, langs in by_key.items():
            if "nl" not in langs or "en" not in langs:
                continue
            for pos in langs["nl"].keys() & langs["en"].keys():
                a, b = langs["nl"][pos], langs["en"][pos]
                src.used |= {a, b}
                line = toks[a].line
                name = ",".join(k.strip('"') for k in key)
                pairs.append(Pair(src.rel, line, "arm", name, text_of(src, a), text_of(src, b),
                                  glossary_ok(src, line), fmt=pos == "fmt"))


def for_arrays(src: Source, pairs: list[Pair]) -> None:
    """for (view, glyph, nl, en, name) in [(…), …]: an inline table named by its pattern."""
    toks = src.toks
    for i in range(len(toks) - 2):
        if toks[i].text != "for" or toks[i + 1].text != "(" or i + 1 not in src.close:
            continue
        pc = src.close[i + 1]
        inner = toks[i + 2:pc]
        names = [x.text for x in inner[0::2]]
        if not (inner and all(x.kind == "ident" for x in inner[0::2]) and all(x.text == "," for x in inner[1::2])
                and columns(names)[0]) or pc + 1 >= len(toks) or toks[pc + 1].text != "in":
            continue
        arr = array_at(src, pc + 2)
        if arr is None:
            continue
        elems = [(a, b) for a, b in split_top(toks, *arr) if toks[a].text == "(" and src.close.get(a) == b - 1]
        table_rows(src, src.fn_at[i] or "for", elems, names, {}, pairs, None)


def struct_fields(sources: list[Source]) -> dict[tuple[str, str], set[str]]:
    """(crate, struct) → its fields of type (&str, &str): a text pair (title: ("Verkenner", "File Explorer"))."""
    out: dict[tuple[str, str], set[str]] = {}
    for src in sources:
        toks = src.toks
        for i in range(len(toks) - 2):
            if toks[i].text != "struct" or toks[i + 1].kind != "ident" or toks[i + 2].text != "{" \
                    or i + 2 not in src.close:
                continue
            for a, b in split_top(toks, i + 3, src.close[i + 2]):
                texts = [x.text for x in toks[a:b] if x.kind != "lifetime"]
                if texts[:1] == ["pub"]:
                    texts = texts[1:]
                if len(texts) >= 2 and texts[1] == ":" and texts[2:] == ["(", "&", "str", ",", "&", "str", ")"]:
                    out.setdefault((crate_of(src.rel), toks[i + 1].text), set()).add(texts[0])
    return out


def struct_pairs(src: Source, fields: dict[tuple[str, str], set[str]], pairs: list[Pair]) -> None:
    """Struct literals Name { …, title: ("…", "…"), … } of a struct with such fields (its crate's or the kit's)."""
    toks = src.toks
    crate = crate_of(src.rel)
    for i in range(len(toks) - 1):
        if toks[i].kind != "ident" or toks[i + 1].text != "{" or i + 1 not in src.close:
            continue
        names = fields.get((crate, toks[i].text)) or fields.get(("native/kit", toks[i].text))
        if not names or (i and toks[i - 1].text == "struct"):
            continue
        parts = split_top(toks, i + 2, src.close[i + 1])
        first = next((lit_at(src, a + 2, b) for a, b in parts if b - a == 3 and toks[a + 1].text == ":"
                      and lit_at(src, a + 2, b) is not None), None)
        ident = text_of(src, first) if first is not None else f"#{toks[i].line}"
        for a, b in parts:
            if b - a < 3 or toks[a].text not in names or toks[a + 1].text != ":" or toks[a + 2].text != "(" \
                    or src.close.get(a + 2) != b - 1:
                continue
            tup = split_top(toks, a + 3, b - 1)
            if len(tup) != 2:
                continue
            x, y = lit_at(src, *tup[0]), lit_at(src, *tup[1])
            if x is None or y is None:
                continue
            src.used |= {x, y}
            line = toks[x].line
            pairs.append(Pair(src.rel, line, "row", f"{toks[i].text}({ident}).{toks[a].text}", text_of(src, x),
                              text_of(src, y), glossary_ok(src, line)))


def json_pairs(rel: str, text: str, pairs: list[Pair], rep: check.Report | None) -> None:
    try:
        data = json.loads(text)
    except ValueError as e:
        if rep is not None:
            rep.add("native-json", rel, 0, f"not JSON: {e}")
        return

    def walk(v, path: list[str]) -> None:
        if isinstance(v, dict):
            if "nl" in v and "en" in v and all(isinstance(v[k], (str, list)) for k in ("nl", "en")):
                pairs.append(Pair(rel, 0, "json", ".".join(path) or ".", v["nl"], v["en"]))
                return
            for k, x in v.items():
                walk(x, path + [str(k)])
        elif isinstance(v, list):
            for n, x in enumerate(v):
                name = x.get("id") if isinstance(x, dict) and isinstance(x.get("id"), str) else f"#{n}"
                walk(x, path + [name])

    walk(data, [])


def read_pairs(root: Path | str = ROOT, rep: check.Report | None = None) -> tuple[list[Pair], list[Source]]:
    """Every pair of native/, and the sources (with the literal tokens the pairs used)."""
    sources, jsons = load(Path(root))
    sigs = signatures(sources)
    fields = struct_fields(sources)
    pairs: list[Pair] = []
    for src in sources:
        calls(src, sigs, pairs, rep)
        arms(src, pairs)
        lang_matches(src, pairs)
        rows(src, pairs, rep)
        for_arrays(src, pairs)
        struct_pairs(src, fields, pairs)
        tables(src, pairs, rep)
        macros(src, pairs)
        ifs(src, pairs)
    for rel, text in jsons.items():
        json_pairs(rel, text, pairs, rep)
    pairs.sort(key=lambda p: (p.file, p.line))
    if rep is not None:
        rep.stats["native_files"] = len(sources) + len(jsons)
        rep.stats["native_signatures"] = sum(s.nl is not None for s in sigs)
    return pairs, sources


# ------------------------------------------------------------------------------------------------------------------
# rules
# ------------------------------------------------------------------------------------------------------------------

def load_allow(root: Path, rep: check.Report) -> list[tuple[str, str]]:
    """tests/i18n/allow-native.txt: (file glob, en phrase) rows."""
    p = Path(root) / ALLOW
    out = []
    if not p.is_file():
        return out
    for no, line in enumerate(p.read_text(encoding="utf-8").split("\n"), 1):
        if not line.strip() or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) != 3 or not parts[1].startswith("en:") or not parts[1][3:] or not parts[2].strip():
            rep.add("native-allow", ALLOW, no, "expected <file glob><TAB>en:<phrase><TAB>why")
            continue
        out.append((parts[0], parts[1][3:]))
    return out


def dutch_words(root: Path) -> set[str]:
    p = Path(root) / DUTCH_WORDS
    if not p.is_file():
        return set()
    return {w.strip() for w in p.read_text(encoding="utf-8").split("\n") if w.strip() and not w.startswith("#")}


def check_pairs(root: Path, rep: check.Report, gloss: check.Glossary, pairs: list[Pair]) -> None:
    words = dutch_words(root)
    if not words:
        rep.add("native-dutch", DUTCH_WORDS, 0, "missing or empty: the English texts cannot be checked")
    allowed = load_allow(root, rep)
    allow_hits: dict[tuple[str, str], int] = {a: 0 for a in allowed}
    placeholders, ARTICLE = check.placeholders, check.ARTICLE

    def dutch_in(text: str) -> list[str]:
        return sorted({w for w in WORD.findall(text) if w.lower().replace("’", "'") in words})

    counts: dict[str, int] = {}
    for p in pairs:
        counts[p.kind] = counts.get(p.kind, 0) + 1
        where = f"{p.key}" if p.kind != "json" else p.key
        if p.keywords:
            if not p.nl or not p.en:
                rep.add("native-pair", p.file, p.line, f"{where}: a keyword list in one language only")
            continue
        if not p.nl and not p.en:
            continue
        if not p.nl or not p.en:
            rep.add("native-pair", p.file, p.line, f"{where}: {'nl' if not p.nl else 'en'} is empty "
                                                    f"({p.nl or p.en!r})")
            continue
        pn, pe = placeholders(p.nl), placeholders(p.en)
        if p.fmt:                   # each format! is checked by rustc against its own arguments: names only
            pn, pe = set(FMT.findall(p.nl)), set(FMT.findall(p.en))
        if pn != pe:
            rep.add("native-placeholders", p.file, p.line, f"{where}: nl has {sorted(pn)}, en has {sorted(pe)}")
        for gnl, gen in gloss.check(p.nl, p.en, p.ok, where=(p.file, p.key)):
            rep.add("glossary", p.file, p.line, f"{where}: nl “{gnl}” → en should contain “{gen}”")
        if ARTICLE.search(p.en):
            rep.add("article", p.file, p.line, f"{where}: en uses an article before a placeholder ({p.en!r})")
        if "'" in p.en:
            rep.add("native-apostrophe", p.file, p.line, f"{where}: en has a straight apostrophe ' (write ’, "
                                                          f"D-G23): {p.en!r}")
        m = US_SPELLING.search(p.en)
        if m:
            rep.add("native-spelling", p.file, p.line, f"{where}: en {m.group(0)!r} is a US spelling (D-G15: en-GB)",
                    level="warning")
        if words:
            text = check.PH.sub(" ", p.en)
            phrases = sorted((a for a in allowed if fnmatch.fnmatchcase(p.file, a[0])), key=lambda a: -len(a[1]))
            for a in phrases:
                text, n = re.subn(r"(?<!\w)" + re.escape(a[1]) + r"(?!\w)", " ", text)
                if n:
                    allow_hits[a] += 1
            hits = dutch_in(text)
            if hits:
                rep.add("native-dutch", p.file, p.line, f"{where}: en has Dutch word(s) {', '.join(hits)}: {p.en!r} "
                        f"(translate it, or allow-list a name in {ALLOW})")
            elif p.en == p.nl and text.strip() and dutch_in(check.PH.sub(" ", p.nl)):
                rep.add("native-dutch", p.file, p.line, f"{where}: en is the Dutch text {p.nl!r}")
    for (glob, phrase), n in allow_hits.items():
        if n == 0:
            rep.add("native-allow", ALLOW, 0, f"{glob} en:{phrase} matches no en text (stale row)")
    rep.stats["native_pairs"] = len(pairs)
    rep.stats["native_kinds"] = "/".join(f"{k} {v}" for k, v in sorted(counts.items()))


def uncovered(sources: list[Source], root: Path) -> list[tuple[str, int, str]]:
    """Literals outside every pair that read as Dutch (scan_rust.py's word test): (file, line, text). Not counted:
    log and panic texts, language codes, technical strings (scan_rust.py's patterns: ids, paths, CSS …) and the
    literals a comparison or a match pattern tests input against (aliases, deep-link names)."""
    import scan_rust  # noqa: PLC0415  (GE0b's word lists and patterns)
    words = scan_rust.Words(Path(root))
    out = []
    for src in sources:
        toks = src.toks
        for k, lit in src.lits.items():
            if k in src.used or lit.macro in scan_rust.LOG_MACROS or lit.macro == "matches!" \
                    or lit.call in (".expect",) or lit.text in LANG_CODES:
                continue
            prev = toks[k - 1].text if k else ""
            nxt = toks[k + 1].text if k + 1 < len(toks) else ""
            if prev in ("==", "!=", "|") or nxt in ("=>", "|", "==", "!="):
                continue
            if any(rx.search(lit.text) for rx, _why in scan_rust.TECH_PATTERNS):
                continue
            if words.lang(lit.text) == "nl":
                out.append((src.rel, lit.line, lit.text))
    return out


def check_layer(root: Path, rep, gloss, opts: dict | None = None) -> None:
    """The dispatcher's entry point (tools/i18n/check.py): add this layer's issues and stats to rep."""
    root = Path(root)
    pairs, sources = read_pairs(root, rep)
    check_pairs(root, rep, gloss, pairs)
    loose = uncovered(sources, root)
    for rel, line, text in loose:
        rep.add("native-dutch-only", rel, line, f"Dutch outside an nl/en pair: {text[:120]!r}", level="warning")
    rep.stats["native_uncovered"] = len(loose)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--warnings", action="store_true")
    ap.add_argument("--pairs", action="store_true", help="list every pair (no checks)")
    ap.add_argument("--uncovered", action="store_true", help="list the Dutch-looking literals outside every pair")
    ap.add_argument("--json", action="store_true", help="with --pairs: as JSON")
    a = ap.parse_args(argv)
    if a.pairs:
        pairs, _ = read_pairs(a.root)
        if a.json:
            print(json.dumps([p.__dict__ | {"ok": sorted(p.ok)} for p in pairs], ensure_ascii=False, indent=1))
        else:
            for p in pairs:
                print(f"{p.file}:{p.line}\t{p.kind}\t{p.key}\t{p.nl!r} → {p.en!r}")
            print(f"{len(pairs)} pairs")
        return 0
    if a.uncovered:
        _, sources = read_pairs(a.root)
        loose = uncovered(sources, Path(a.root))
        for rel, line, text in loose:
            print(f"{rel}:{line}\t{text!r}")
        print(f"{len(loose)} Dutch-looking literals outside every pair")
        return 0
    return check.main(["--root", a.root, "--layers", "native"] + (["--warnings"] if a.warnings else []))


if __name__ == "__main__":
    sys.exit(main())
