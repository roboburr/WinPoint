#!/usr/bin/env python3
"""Checker for the Rust host's catalogues and their call sites (GE1b; plan-critique F8, F16; round2-gate.md).

    python3 -B tools/i18n/check_rust.py              # errors only; exit 1 when there is one
    python3 -B tools/i18n/check_rust.py --warnings   # also warnings (glossary, unused rows, extra params …)
    python3 -B tools/i18n/check_rust.py --no-labels  # skip the label rule (say so in your note)

Errors:
  table      host/src/i18n.rs STEMS lists exactly the files host/i18n/*.tsv, each as include_str!("../i18n/<stem>.tsv")
  header     each catalogue starts with "# host/i18n/<stem>.tsv"
  row        key<TAB>nl<TAB>en, exactly three fields; key [a-z][a-z0-9_]*(.name)+ with the stem as first segment;
             no key twice (in all catalogues); nl and en not empty; escapes \\n \\t \\\\ only; no CR
  map        "#@" lines are well-formed and directly above a row (the drift check verifies what they say)
  ph         nl and en have the same {placeholders}
  apostrophe an en text uses the typographic apostrophe ’ (U+2019), never a straight ' (U+0027), like the UI and the
             extension (plan.md D-G23; Windows 11 en-GB). nl is not checked: it is 3b898e6's own text
  plural     key.one / key.other / key.zero: both .one and .other exist
  key        every key literal passed to i18n::tr/trf/trn or ApiError::tr/trn exists (a plural: key.one and key.other)
  params     a call's json!({…}) params give every placeholder of the row (a missing one would show "{name}")
  label      a quoted name in an en row (“…”, ‘…’, "…") is an en text of the native programs (native/: the nl/en
             pairs that tools/i18n/check_native.py reads; until the WebKit removal the UI dictionaries), or is
             allow-listed in tests/i18n/allow-host.txt with label:<text>. When the native texts cannot be read,
             that is an error too, unless --no-labels is given
  en-dutch   no Dutch in an English text, in the catalogues and in host/src/desktop-entries.json: no en word is in
             tests/i18n/dutch-words.txt (the list was built to hold no English word), and en is not a copy of a
             Dutch nl. A word or phrase that is Dutch on purpose (a name: D-G5 "Alles" is not in the list) is
             allow-listed with en:<text> (path host/i18n/<stem>.tsv or host/src/desktop-entries.json)
  allow      tests/i18n/allow-host.txt is well-formed
  glossary   (WU-FIX, R3-UI review m5; an error like check.py's since the round-3 lock-in, D-G48) when an nl text has a
             phrase of tools/i18n/glossary.tsv, its en text has that phrase's Windows 11 en-GB rendering. A host
             context row of tools/i18n/glossary-context.tsv (a "#host" line, below) at that row replaces the rendering
             of the same nl phrase, or adds a longer phrase that masks a shorter one; a row flagged "known" (an accepted
             deviation for its owner, e.g. the disks text for Robin, D-G1b) turns its finding into a warning
  glossary-context  a host context row is well-formed (where, nl, en, flags "-"/"name"/"known", why), names only
             host/i18n/<stem>.tsv:<key glob> places, each file glob matches a catalogue, and some row at its places
             still has its nl phrase (else it is stale)
Host context rows are the lines of tools/i18n/glossary-context.tsv that start with "#host<TAB>" (then the five
columns of check.py's context rows). They are comment lines for check.py, which visits only native/ and the host's
JSON tables and would refuse a host/i18n place; check_rust.py reads only these.
Warnings: unused rows, params without a placeholder, dynamic keys, spaces at the start or end of a text, known
glossary deviations, allow-host rules that match nothing, en words that only GE0b's wider Dutch heuristics
(scan_rust.py: its extra words and Dutch word shapes) call Dutch, and (apostrophe, R3-NATIVE review m9) a straight '
in an en text of the services' JSON tables (host/src/**/*.json: the en side of a {"nl": …, "en": …} object and the
second string of a [nl, en] pair). Those tables belong to their service steps (disks/data.json is Robin's, D-G1b), so
this stays a warning until they are converted.
Keys are found only in calls through the i18n:: path (crate::i18n::tr, i18n::trf …) and ApiError::tr/trn, so call
them that way (not a bare `tr(`, which the services also use for their own JSON tables).
"""
from __future__ import annotations

import argparse
import fnmatch
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import drift_rust as d  # noqa: E402
import extract_rs as x  # noqa: E402

ROOT = d.ROOT
KEY_RE = re.compile(r"^[a-z][a-z0-9_]*(\.[A-Za-z0-9_]+)+$")
STEM_ROW = re.compile(r'\(\s*"([a-z0-9_]+)"\s*,\s*include_str!\(\s*"\.\./i18n/([a-z0-9_]+)\.tsv"\s*\)\s*\)')
QUOTED = re.compile(r"“([^”]+)”|‘([^’]+)’|\"([^\"]+)\"")
WORD = re.compile(r"[A-Za-zÀ-ÖØ-öø-ÿ]+(?:['’][a-zë]+)?")             # as tools/i18n/scan_en.py
DUTCH_WORDS = "tests/i18n/dutch-words.txt"
ENTRIES_TABLE = "host/src/desktop-entries.json"
PLURAL = re.compile(r"\.(one|other|zero)$")


class Report:
    def __init__(self) -> None:
        self.errors: list[str] = []
        self.warnings: list[str] = []

    def error(self, rule: str, where: str, msg: str) -> None:
        self.errors.append(f"[{rule}] {where}: {msg}")

    def warn(self, rule: str, where: str, msg: str) -> None:
        self.warnings.append(f"[{rule}] {where}: {msg}")


def check_table(r: Report, root: Path) -> list[str]:
    src = (root / d.SRC / "i18n.rs").read_text(encoding="utf-8")
    m = re.search(r"pub\(crate\) const STEMS: &\[\(&str, &str\)\] = &\[(.*?)\];", src, re.S)
    if not m:
        r.error("table", "host/src/i18n.rs", "no STEMS table")
        return []
    listed = []
    for stem, file in STEM_ROW.findall(m.group(1)):
        if stem != file:
            r.error("table", "host/src/i18n.rs", f"stem {stem} includes {file}.tsv")
        listed.append(stem)
    files = sorted(p.stem for p in (root / d.CATALOGUES).glob("*.tsv"))
    for s in sorted(set(files) - set(listed)):
        r.error("table", "host/src/i18n.rs", f"host/i18n/{s}.tsv is not in STEMS (it would never be loaded)")
    for s in sorted(set(listed) - set(files)):
        r.error("table", "host/src/i18n.rs", f"STEMS lists {s} but host/i18n/{s}.tsv does not exist")
    if len(listed) != len(set(listed)):
        r.error("table", "host/src/i18n.rs", "a stem is listed twice")
    return listed


def placeholders(text: str) -> set[str]:
    return set(d.PLACEHOLDER.findall(text))


# WU-FIX (R3-UI review m5): the host context rows of tools/i18n/glossary-context.tsv
CONTEXT = "tools/i18n/glossary-context.tsv"
HOST_PREFIX = "#host\t"
CONTEXT_FLAGS = {"-", "name", "known"}


class HostContext:
    """One "#host" row of glossary-context.tsv: at these catalogue places, this nl phrase is rendered with this en."""

    def __init__(self, line: int, places: list[tuple[str, str]], nl: str, en: str, flags: set[str], why: str):
        self.line, self.places, self.nl, self.en, self.why = line, places, nl, en, why
        self.name, self.known = "name" in flags, "known" in flags
        self.hits = 0

    def applies(self, file: str, key: str) -> bool:
        return any(fnmatch.fnmatchcase(file, f) and fnmatch.fnmatchcase(key, k) for f, k in self.places)


def load_host_context(r: Report, root: Path) -> list[HostContext]:
    out: list[HostContext] = []
    p = root / CONTEXT
    if not p.is_file():
        return out
    for no, line in enumerate(p.read_text(encoding="utf-8").split("\n"), 1):
        if not line.startswith(HOST_PREFIX):
            continue
        where = f"{CONTEXT}:{no}"
        parts = line[len(HOST_PREFIX):].split("\t")
        if len(parts) != 5 or not all(x.strip() for x in (parts[0], parts[1], parts[2], parts[4])):
            r.error("glossary-context", where, "expected #host<TAB>where<TAB>nl<TAB>en<TAB>flags<TAB>why")
            continue
        place_text, nl, en, flag_text, why = parts
        flags = {f.strip() for f in flag_text.split(",") if f.strip()}
        if flags - CONTEXT_FLAGS:
            r.error("glossary-context", where, f"unknown flags {sorted(flags - CONTEXT_FLAGS)} (name, known or -)")
        places = []
        for w in place_text.split(","):
            f, _, k = w.strip().partition(":")
            if not (f.startswith("host/i18n/") and f.endswith(".tsv")):
                r.error("glossary-context", where, f"{f}: a host row names host/i18n/<stem>.tsv places only")
            elif not any(fnmatch.fnmatchcase(c.relative_to(root).as_posix(), f) for c in (root / d.CATALOGUES).glob("*.tsv")):
                r.error("glossary-context", where, f"{f} matches no host catalogue")
            places.append((f, k or "*"))
        out.append(HostContext(no, places, nl, en, flags, why))
    return out


def glossary_findings(row: d.Row, glossary: list[tuple[str, str, str]], context: list[HostContext]):
    """(nl phrase, en phrase, the context row or None) that the row's en lacks: the glossary with the context rows of
    its place, longest phrase first; a found phrase is masked, so a shorter one inside it does not count again."""
    here = [c for c in context if c.applies(row.file, row.key)]
    own = {c.nl.lower() for c in here}
    rules = [(nl, en, "name" in flags, None) for nl, en, flags in glossary if nl.lower() not in own]
    rules += [(c.nl, c.en, c.name, c) for c in here]
    rules.sort(key=lambda t: -len(t[0]))
    masked, out = row.nl, []
    for nl_phrase, en_phrase, name, crow in rules:
        rx = re.compile(r"(?<![\w-])" + re.escape(nl_phrase) + r"(?![\w-])", 0 if name else re.I)
        if not rx.search(masked):
            continue
        masked = rx.sub(lambda m: "\0" * len(m.group(0)), masked)
        if crow is not None:
            crow.hits += 1
        if (en_phrase not in row.en) if name else (en_phrase.lower() not in row.en.lower()):
            out.append((nl_phrase, en_phrase, crow))
    return out
# WU-FIX end


def check_rows(r: Report, root: Path, glossary: list[tuple[str, str, str]]) -> dict[str, d.Row]:
    rows, problems = d.load_catalogues(root)
    context = load_host_context(r, root)
    for p in problems:
        r.error("map", p.split(":")[0], p)
    keys: dict[str, d.Row] = {}
    for f in sorted((root / d.CATALOGUES).glob("*.tsv")):
        text = f.read_text(encoding="utf-8")
        rel = f.relative_to(root).as_posix()
        if not text.startswith(f"# host/i18n/{f.stem}.tsv"):
            r.error("header", rel, f"must start with '# host/i18n/{f.stem}.tsv'")
        if "\r" in text:
            r.error("row", rel, "contains a carriage return")
    for row in rows:
        where = f"{row.file}:{row.line}"
        stem = Path(row.file).stem
        if len(row.raw) != 3:
            r.error("row", where, f"{len(row.raw)} fields, want key<TAB>nl<TAB>en")
        if not KEY_RE.match(row.key):
            r.error("row", where, f"key {row.key!r} is not [a-z][a-z0-9_]*(.name)+")
        elif row.key.split(".")[0] != stem:
            r.error("row", where, f"key {row.key} does not start with {stem}.")
        if row.key in keys:
            r.error("row", where, f"{row.key} also at {keys[row.key].file}:{keys[row.key].line}")
        keys[row.key] = row
        for name, raw, value in (("nl", row.raw[1] if len(row.raw) > 1 else "", row.nl),
                                 ("en", row.raw[2] if len(row.raw) > 2 else "", row.en)):
            if not value:
                r.error("row", where, f"{row.key}: {name} is empty")
                continue
            for m in re.finditer(r"\\(.?)", raw):
                if m.group(1) not in ("n", "t", "\\"):
                    r.error("row", where, f"{row.key}: {name} has the escape \\{m.group(1)} (only \\n \\t \\\\)")
            if value != value.strip(" "):
                r.warn("space", where, f"{row.key}: {name} starts or ends with a space")
        if "'" in row.en:
            r.error("apostrophe", where, f"{row.key}: en has a straight apostrophe ' (write ’, D-G23): {row.en!r}")
        if row.nl and row.en and placeholders(row.nl) != placeholders(row.en):
            r.error("ph", where, f"{row.key}: nl {sorted(placeholders(row.nl))} != en {sorted(placeholders(row.en))}")
        # WU-FIX (R3-UI review m5): an error, as in check.py; a "known" host context row keeps it a warning
        for nl_phrase, en_phrase, crow in glossary_findings(row, glossary, context):
            msg = f"{row.key}: nl has {nl_phrase!r}, en lacks {en_phrase!r}"
            if crow is not None and crow.known:
                r.warn("glossary", where, f"{msg} (known deviation, {CONTEXT}:{crow.line}: {crow.why})")
            else:
                r.error("glossary", where, msg + (f" ({CONTEXT}:{crow.line})" if crow is not None else ""))
    for c in context:
        if c.hits == 0:
            r.error("glossary-context", f"{CONTEXT}:{c.line}", f"“{c.nl}” → “{c.en}”: no row at its places has the nl "
                                                              "phrase any more (stale row)")
    for key in keys:
        m = PLURAL.search(key)
        if m:
            base = key[:m.start()]
            for form in ("one", "other"):
                if f"{base}.{form}" not in keys:
                    r.error("plural", f"{keys[key].file}:{keys[key].line}", f"{base}.{form} is missing")
    return keys


def load_glossary(root: Path) -> list[tuple[str, str, str]]:
    out = []
    p = root / "tools/i18n/glossary.tsv"
    if not p.is_file():
        return out
    for line in p.read_text(encoding="utf-8").split("\n"):
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) >= 2:
            out.append((f[0], f[1], f[2] if len(f) > 2 else ""))
    return sorted(out, key=lambda t: -len(t[0]))


def call_path(toks: list[d.Tok], open_idx: int) -> str:
    k = open_idx - 1
    if k < 0 or toks[k].kind != "ident":
        return ""
    parts = [toks[k].text]
    k -= 1
    while k >= 1 and toks[k].text == "::" and toks[k - 1].kind == "ident":
        parts.insert(0, toks[k - 1].text)
        k -= 2
    return "::".join(parts)


def arg_index(toks: list[d.Tok], open_idx: int, i: int) -> tuple[int, bool]:
    """(index of the top-level argument that contains toks[i], whether toks[i] is that whole argument)."""
    close = x.matching_close(toks, open_idx)
    for n, (a, b) in enumerate(x.split_args(toks, open_idx, close)):
        if a <= i < b:
            return n, (b - a == 1)
    return -1, False


def json_keys(toks: list[d.Tok], a: int, b: int) -> set[str] | None:
    """The top-level keys of `&serde_json::json!({…})` / `&json!({…})` in toks[a:b]; set() for NO_PARAMS; None
    for anything else (a variable: not checked)."""
    texts = [t.text for t in toks[a:b]]
    if texts and texts[-1] == "NO_PARAMS":
        return set()
    try:
        j = texts.index("json")
    except ValueError:
        return None
    if texts[j + 1:j + 4] != ["!", "(", "{"]:
        return None
    brace = a + j + 3
    end = x.matching_close(toks, brace)
    keys, depth = set(), 0
    for k in range(brace + 1, end):
        t = toks[k]
        if t.kind == "punct" and t.text in d.OPEN:
            depth += 1
        elif t.kind == "punct" and t.text in d.CLOSE:
            depth -= 1
        elif depth == 0 and t.kind == "str" and k + 1 < end and toks[k + 1].text == ":":
            keys.add(t.text)
    return keys


# (call path suffix, key argument, params argument or None, plural)
CALLS = {
    "i18n::tr": (1, None, False), "i18n::trf": (1, 2, False), "i18n::trn": (1, 3, True),
    "ApiError::tr": (2, 3, False), "ApiError::trn": (2, 4, True),
}


def check_code(r: Report, root: Path, keys: dict[str, d.Row], stems: list[str]) -> None:
    files = d.tree_files(root)
    scanned, tests = d.scan_tree({k: v for k, v in files.items() if k.endswith(".rs")})
    referenced: set[str] = set()
    stem_set = set(stems)
    for rel, fl in sorted(scanned.items()):
        if rel in tests or rel == "i18n.rs":
            continue
        toks = fl.toks
        for lit in fl.lits:
            if "." in lit.text and lit.text.split(".")[0] in stem_set:
                referenced.add(lit.text)
            o = x.enclosing_open(toks, lit.tok)
            if o is None or toks[o].text != "(":
                continue
            path = call_path(toks, o)
            spec = next((v for k, v in CALLS.items() if path == k or path.endswith("::" + k)
                         or (k.startswith("ApiError") and path.endswith(k))), None)
            if spec is None:
                continue
            key_arg, params_arg, plural = spec
            n, whole = arg_index(toks, o, lit.tok)
            if n != key_arg:
                continue
            where = f"{d.SRC}/{rel}:{lit.line}"
            if not whole:
                r.warn("key", where, f"dynamic key in {path}(): cannot verify")
                continue
            key = lit.text
            if plural:
                missing = [f for f in ("one", "other") if f"{key}.{f}" not in keys]
                if missing:
                    r.error("key", where, f"{path}(): {key}.{'/'.join(missing)} not in any catalogue")
                    continue
                row_ph = set().union(*(placeholders(keys[f"{key}.{f}"].nl) for f in ("one", "other")
                                       if f"{key}.{f}" in keys)) - {"n"}
            else:
                if key not in keys:
                    r.error("key", where, f"{path}(): {key} is not in any catalogue")
                    continue
                row_ph = placeholders(keys[key].nl)
            if params_arg is None:
                if row_ph:
                    r.error("params", where, f"{path}(): {key} has {sorted(row_ph)}; use trf with params")
                continue
            close = x.matching_close(toks, o)
            args = x.split_args(toks, o, close)
            if params_arg >= len(args):
                continue
            given = json_keys(toks, *args[params_arg])
            if given is None:
                continue
            given -= {"n"} if plural else set()
            if row_ph - given:
                r.error("params", where, f"{path}(): {key} needs {sorted(row_ph - given)} in its params")
            if given - row_ph:
                r.warn("params", where, f"{path}(): {key} has no placeholder for {sorted(given - row_ph)}")
    for key, row in keys.items():
        base = PLURAL.sub("", key)
        if key not in referenced and base not in referenced:
            r.warn("unused", f"{row.file}:{row.line}", f"{key} is not used in host/src")


def check_labels(r: Report, root: Path, keys: dict[str, d.Row], allowed: set[str], skip: bool = False) -> None:
    if skip:
        r.warn("label", "native", "label check skipped (--no-labels)")
        return
    try:
        import corpus  # noqa: PLC0415  (tools/i18n/corpus.py: the native pairs of check_native.py)
        texts = corpus.native_texts(root)
    except Exception as e:  # noqa: BLE001
        # fail closed (review m4): an unreadable source must not give "0 errors" for an unchecked rule
        r.error("label", "native", f"the label check could not read the native texts ({e}); pass --no-labels "
                "and say so in your note")
        return
    ui_en = {t.text.strip() for t in texts if t.lang == "en"}
    ui_nl = {t.text.strip() for t in texts if t.lang == "nl"}

    def known(label: str, pool: set[str]) -> bool:
        for v in (label, label.rstrip("…").rstrip("."), label + "…"):
            if v in pool:
                return True
        return False

    for key, row in keys.items():
        for side, text, pool in (("en", row.en, ui_en), ("nl", row.nl, ui_nl)):
            for m in QUOTED.finditer(text):
                label = next(g for g in m.groups() if g is not None)
                if "{" in label or label in allowed:
                    continue
                if not known(label, pool):
                    if side == "en":
                        r.error("label", f"{row.file}:{row.line}", f"{key}: en names “{label}”, which is no en text "
                                "of the native programs (fix it, or allow-list it: label:<text> in "
                                "tests/i18n/allow-host.txt)")
                    else:
                        r.warn("label", f"{row.file}:{row.line}", f"{key}: nl names “{label}”, which is no nl text "
                               "of the native programs")


def load_dutch_words(root: Path) -> set[str]:
    p = root / DUTCH_WORDS
    if not p.is_file():
        return set()
    return {w.strip() for w in p.read_text(encoding="utf-8").split("\n") if w.strip() and not w.startswith("#")}


def english_texts(root: Path, keys: dict[str, d.Row]) -> list[tuple[str, str, str, str, str]]:
    """(where, file, key, nl, en) of every catalogue row and of the desktop-entry table (C6)."""
    out = [(f"{row.file}:{row.line}", row.file, key, row.nl, row.en) for key, row in keys.items()]
    p = root / ENTRIES_TABLE
    if p.is_file():
        import json  # noqa: PLC0415
        data = json.loads(p.read_text(encoding="utf-8"))
        pairs = [("comment", data.get("comment") or {})]
        pairs += [(f"entries.{name}", v) for name, v in sorted((data.get("entries") or {}).items())]
        for name, v in pairs:
            out.append((f"{ENTRIES_TABLE} {name}", ENTRIES_TABLE, name, v.get("nl") or "", v.get("en") or ""))
    return out


JSON_SKIP = ("reference-tests.json", "html-entities.json", "api-methods.json", "settings-defaults.json")


def check_json_apostrophes(r: Report, root: Path) -> None:
    """[apostrophe] as a warning (R3-NATIVE review m9): straight apostrophes in the en texts of host/src's JSON
    string tables."""
    import json  # noqa: PLC0415

    for p in sorted((root / "host" / "src").rglob("*.json")):
        # the api-methods fragments (host/src/api-methods/*.json, settings parity §3) are method lists like api-methods.json
        if p.name in JSON_SKIP or p.parent.name == "api-methods":
            continue
        rel = p.relative_to(root).as_posix()
        try:
            data = json.loads(p.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        note = " (D-G1b: for Robin)" if rel == "host/src/services/disks/data.json" else ""
        found: list[tuple[str, str]] = []

        def walk(v: object, path: str, en: bool) -> None:
            if isinstance(v, dict):
                if "nl" in v and "en" in v:
                    walk(v["en"], f"{path}.en", True)
                    for k, x in v.items():
                        if k not in ("nl", "en"):
                            walk(x, f"{path}.{k}", en)
                    return
                for k, x in v.items():
                    walk(x, f"{path}.{k}", en)
            elif isinstance(v, list):
                if not en and len(v) == 2 and all(isinstance(x, str) for x in v):
                    walk(v[1], f"{path}[1]", True)
                    return
                for i, x in enumerate(v):
                    walk(x, f"{path}[{i}]", en)
            elif isinstance(v, str) and en and "'" in v:
                found.append((path or ".", v))

        walk(data, "", False)
        for path, text in found:
            r.warn("apostrophe", f"{rel} {path}", f"en has a straight apostrophe ' (write ’, D-G23){note}: {text!r}")


def check_english(r: Report, root: Path, keys: dict[str, d.Row], allowed_en: list[tuple[str, str]]) -> None:
    """[en-dutch] (review M2): Dutch left in an English text."""
    words = load_dutch_words(root)
    if not words:
        r.error("en-dutch", DUTCH_WORDS, "missing or empty: the English column cannot be checked")
        return
    try:
        import scan_rust  # noqa: PLC0415  (GE0b's word rules, for the warnings only)
        wider = scan_rust.Words(root)
    except Exception:  # noqa: BLE001
        wider = None

    def dutch_in(text: str) -> list[str]:
        return sorted({w for w in WORD.findall(text) if w.lower().replace("’", "'") in words})

    for where, file, key, nl, en in english_texts(root, keys):
        if not en:
            continue
        phrases = sorted((ph for glob, ph in allowed_en if fnmatch.fnmatch(file, glob)), key=len, reverse=True)
        text = d.PLACEHOLDER.sub(" ", en)
        for ph in phrases:
            text = re.sub(r"(?<!\w)" + re.escape(ph) + r"(?!\w)", " ", text)
        hits = dutch_in(text)
        if hits:
            r.error("en-dutch", where, f"{key}: en has Dutch word(s) {', '.join(hits)}: {en!r} (translate it, or "
                    "allow-list a name with en:<text> in tests/i18n/allow-host.txt)")
            continue
        if en == nl and en not in phrases and (dutch_in(d.PLACEHOLDER.sub(" ", nl))
                                               or (wider and wider.lang(nl) in ("nl", "mixed"))):
            r.error("en-dutch", where, f"{key}: en is the Dutch text {nl!r}")
            continue
        if wider and text.strip():
            _, nl_words, _ = wider.split(text)
            if nl_words:
                r.warn("en-dutch", where, f"{key}: en may hold Dutch ({', '.join(sorted(set(nl_words)))}): {en!r}")


def check_allow(r: Report, root: Path) -> tuple[set[str], list[tuple[str, str]]]:
    rules, problems = x.load_allow(root)
    for p in problems:
        r.error("allow", x.ALLOW, p)
    labels = {v for rule in rules for k, v in rule.selectors if k == "label"}
    allowed_en = [(rule.path, v) for rule in rules for k, v in rule.selectors if k == "en"]
    lit_rules = [rule for rule in rules if not any(k in x.TEXT_KINDS for k, _ in rule.selectors)]
    current = d.all_literals(d.tree_files(root))
    frozen = d.load_frozen(root)
    classes = {(f.path, f.sha): f.cls for fl in frozen.values() for f in fl}
    for rule in lit_rules:
        if rule.selectors == [("file", "")] and not rule.path.endswith(".rs"):
            continue                                    # a non-Rust file (api-methods.json): listed for the record
        if "**" in rule.path:
            continue                                    # a generic rule (log macros …): kept for future code
        hit = False
        for path, lits in current.items():
            if not fnmatch.fnmatch(path, rule.path):
                continue
            for lit in lits:
                if x.rule_matches(rule, lit, classes.get((path, lit.sha), "new")):
                    hit = True
                    break
            if hit:
                break
        if not hit:
            r.warn("allow", f"{rule.file}:{rule.line}", f"matches no literal ({rule.path} {rule.selectors})")
    return labels, allowed_en


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--warnings", action="store_true")
    ap.add_argument("--no-labels", action="store_true", help="skip the label rule; a warning")
    a = ap.parse_args(argv)
    r = Report()
    stems = check_table(r, ROOT)
    keys = check_rows(r, ROOT, load_glossary(ROOT))
    check_code(r, ROOT, keys, stems)
    labels, allowed_en = check_allow(r, ROOT)
    check_labels(r, ROOT, keys, labels, skip=a.no_labels)
    check_english(r, ROOT, keys, allowed_en)
    check_json_apostrophes(r, ROOT)
    for e in r.errors:
        print("error:", e)
    if a.warnings:
        for w in r.warnings:
            print("warning:", w)
    print(f"check_rust: {len(stems)} stems, {len(keys)} rows; {len(r.errors)} errors, {len(r.warnings)} warnings")
    return 1 if r.errors else 0


if __name__ == "__main__":
    sys.exit(main())
