#!/usr/bin/env python3
"""The language checker of Win11 voor GNOME: a thin dispatcher over one module per layer (F39; GE1a,
notes/claude/round1/ge1a.md; the rules came from H's check.py, ported in GE0a).

    python3 -B tools/i18n/check.py [--root DIR] [--json] [--warnings] [--no-js] [--layers ui,rust,pairs]

Exit code 0 when no rule found an *error*, 1 otherwise. Warnings never fail; ``--warnings`` lists them (by default
only their counts per rule are shown). Nothing is written.

Layers (each one file next to this one; a layer whose file does not exist yet is skipped and named in the stats):

* ``ui``    — ``check_ui.py`` (GE1a): ``makeT`` dictionaries, ``{nl, en}`` pairs, ``P()`` and ``X_NL``/``X_EN``
  tables in ``vendor/ui/js`` (needs node)
* ``rust``  — ``check_rust.py`` (GE1b): the Rust catalogues and call sites
* ``pairs`` — ``check_pairs.py`` (GE1c): the extension's and the installer's inline ``{nl, en}`` tables

A layer module either defines ``check_layer(root, rep, gloss, opts)`` (``rep`` a :class:`Report`, ``gloss`` the
:class:`Glossary`, ``opts`` ``{"js": bool, "js_data": dict | None}``; it may define ``DEFERRED``, rule → the step that
makes it an error, merged into :data:`DEFERRED`, and ``REPLACES``, the shared rules below it implements itself:
``"tsv"``, ``"host-json"``; it imports this module for the shared pieces, ``import check``), or is a script with its
own ``main()``: then it runs as ``python3 -B FILE --warnings`` (always on this tree, whatever ``--root`` says), and
its output lines ``error: …``/``ERROR …`` and ``warning: …``/``warn …`` become errors and warnings of that layer,
its last line goes into the stats, and a non-zero exit without an error line is an error. ``check_rust.py`` (a
script) replaces the shared ``tsv`` rule (:data:`SCRIPT_REPLACES`).

Shared rules kept here until a layer replaces them (they run unless ``--layers`` is given):

* ``tsv`` — TSV catalogues (contracts C3, D-G8: ``host/i18n/<stem>.tsv``; also any ``i18n/**/*.tsv``): three fields,
  no empty nl or en, no duplicate keys, first key segment = file stem, the same ``{placeholders}`` in nl and en,
  complete plurals (a ``.one``/``.zero`` row needs ``.other``), only ``\\n``/``\\t``/``\\\\`` escapes, glossary.

The glossary rule (``tools/i18n/glossary.tsv``: when an nl text has a glossary phrase, its en text has the Windows 11
en-GB phrase) is an error since the round-3 lock-in (R3-UI). Where a second rendering is right, a *context row* of
``tools/i18n/glossary-context.tsv`` names the places (file and key) and the en phrase expected there instead; a
context row that no checked text uses any more, or that names a place this checker does not visit, is an error too
(rule ``glossary-context``). The rows live in their own file because ``check_rust.py`` reads every row of
``glossary.tsv`` as a rule for the whole host.
* ``host-json`` — the Rust host's ``{nl, en}`` JSON tables (``paths.HOST_JSON_TABLES``): key and placeholder parity
  (``str-table`` warnings).
* ``readme`` — README.md and README.en.md (once it exists, round 3) have the same heading levels and fenced blocks.
"""

from __future__ import annotations

import argparse
import fnmatch
import importlib
import json
import re
import subprocess
import sys
from dataclasses import asdict, dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
if __name__ == "__main__":                       # layer modules `import check`: give them this module, not a copy
    sys.modules.setdefault("check", sys.modules["__main__"])
import corpus  # noqa: E402
import paths  # noqa: E402

ROOT = paths.ROOT

LAYERS = (("ui", "check_ui"), ("rust", "check_rust"), ("pairs", "check_pairs"))

PH = re.compile(r"\{(\w+)\}", re.ASCII)
# the first segment is the file stem (files, files_launch …); then any number of name segments, which may contain
# "-" after their first character (storage's cleanup ids: storage.k_user-cache-safe)
KEY_RE = re.compile(r"^[a-z][A-Za-z0-9_]*(?:\.[A-Za-z0-9_][A-Za-z0-9_-]*)+$")

# rule → the step of notes/claude/plan.md that makes it an error (until then: a warning); layers add their own.
# "glossary" became an error in round 3 (R3-UI, the glossary lock-in).
DEFERRED = {
    "article": "round 3",
    "str-table": "GE1b",
}


@dataclass
class Issue:
    level: str
    rule: str
    file: str
    line: int
    msg: str

    def text(self) -> str:
        where = f"{self.file}:{self.line}" if self.line else self.file
        return f"{self.level:7} {self.rule:20} {where}: {self.msg}"


@dataclass
class Report:
    issues: list[Issue] = field(default_factory=list)
    stats: dict = field(default_factory=dict)

    def add(self, rule: str, file: str, line: int, msg: str, level: str | None = None) -> None:
        if level is None:
            level = "warning" if rule in DEFERRED else "error"
        self.issues.append(Issue(level, rule, file, int(line or 0), msg))

    @property
    def errors(self) -> list[Issue]:
        return [i for i in self.issues if i.level == "error"]

    @property
    def warnings(self) -> list[Issue]:
        return [i for i in self.issues if i.level == "warning"]

    def to_json(self) -> dict:
        return {"errors": [asdict(i) for i in self.errors], "warnings": [asdict(i) for i in self.warnings],
                "stats": self.stats}


def placeholders(s: str) -> set[str]:
    return set(PH.findall(s))


# ------------------------------------------------------------------------------------------------------------------
# shared helpers: glossary, English plural nouns, articles
# ------------------------------------------------------------------------------------------------------------------

@dataclass
class ContextRow:
    """One row of glossary-context.tsv: at these places, this nl phrase is rendered with this en phrase."""
    line: int
    places: list[tuple[str, str]]           # (file glob below the repo, key glob)
    nl: str
    en: str
    name: bool
    known: bool                             # an accepted deviation (not a right second rendering): for the report
    why: str
    rx_nl: re.Pattern
    rx_en: re.Pattern
    hits: int = 0                           # checked texts at its places that contain its nl phrase

    def applies(self, file: str, key: str) -> bool:
        return any(fnmatch.fnmatchcase(file, f) and fnmatch.fnmatchcase(key, k) for f, k in self.places)


class Glossary:
    """nl phrase → en phrase; longest phrase first, at word boundaries; case-sensitive only for rows flagged name.
    Context rows (glossary-context.tsv next to glossary.tsv) replace the row of the same nl phrase, or add a longer
    phrase, at their places (``check(…, where=(file, key))``)."""

    CONTEXT = "glossary-context.tsv"

    def __init__(self, path: Path, context: Path | None = None):
        self.rows: list[tuple[str, str, bool, re.Pattern, re.Pattern, ContextRow | None]] = []
        self.context: list[ContextRow] = []
        self.context_problems: list[tuple[int, str]] = []
        self.seen_files: set[str] = set()
        if path.exists():
            for line in path.read_text(encoding="utf-8").split("\n"):
                if not line.strip() or line.startswith("#"):
                    continue
                parts = line.split("\t")
                if len(parts) < 2:
                    continue
                nl, en, flags = parts[0], parts[1], (parts[2] if len(parts) > 2 else "")
                name = "name" in flags
                fl = 0 if name else re.IGNORECASE
                # en: no end boundary, so inflected forms count ("Format" in "Formatting…", "Cancel" in "Cancelling")
                self.rows.append((nl, en, name, self._rx(nl, fl), self._rx(en, fl, end=False), None))
            self.rows.sort(key=lambda r: -len(r[0]))
        cpath = context if context is not None else path.with_name(self.CONTEXT)
        if cpath.exists():
            self._load_context(cpath)

    def _load_context(self, path: Path) -> None:
        for no, line in enumerate(path.read_text(encoding="utf-8").split("\n"), 1):
            if not line.strip() or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) != 5 or not all(x.strip() for x in (parts[0], parts[1], parts[2], parts[4])):
                self.context_problems.append((no, "expected 5 tab-separated fields: where, nl, en, flags, why"))
                continue
            where, nl, en, flags, why = parts
            bad = sorted(set(f for f in flags.split(",") if f.strip()) - {"name", "known", "-"})
            if bad:
                self.context_problems.append((no, f"unknown flags {bad} (name, known or -)"))
            places = []
            for w in where.split(","):
                w = w.strip()
                f, _, k = w.partition(":")
                places.append((f, k or "*"))
            name = "name" in flags
            fl = 0 if name else re.IGNORECASE
            self.context.append(ContextRow(no, places, nl, en, name, "known" in flags, why,
                                           self._rx(nl, fl), self._rx(en, fl, end=False)))

    @staticmethod
    def _rx(phrase: str, flags: int, end: bool = True) -> re.Pattern:
        return re.compile(r"(?<![\w’'])" + re.escape(phrase) + (r"(?!\w)" if end else ""), flags)

    def check(self, nl: str, en: str, ok: set[str], where: tuple[str, str] | None = None) -> list[tuple[str, str]]:
        """[(nl phrase, expected en phrase)] that the en text lacks; where = (file below the repo, key) lets the
        context rows of that place apply."""
        if (not self.rows and not self.context) or not nl or not en:
            return []
        rows = self.rows
        if where is not None:
            self.seen_files.add(where[0])
            ctx = [c for c in self.context if c.applies(*where)]
            if ctx:
                own = {c.nl.lower() for c in ctx}
                rows = sorted([r for r in self.rows if r[0].lower() not in own]
                              + [(c.nl, c.en, c.name, c.rx_nl, c.rx_en, c) for c in ctx], key=lambda r: -len(r[0]))
        masked = nl
        out = []
        for gnl, gen, _name, rx_nl, rx_en, crow in rows:
            if not rx_nl.search(masked):
                continue
            masked = rx_nl.sub(lambda m: "\0" * len(m.group(0)), masked)
            if crow is not None:
                crow.hits += 1
            if gnl in ok or gnl.lower() in {o.lower() for o in ok}:
                continue
            if not rx_en.search(en):
                out.append((gnl, gen))
        return out

    def check_context(self, rep: "Report", checked: dict[str, bool]) -> None:
        """Rule glossary-context: malformed rows, rows outside the checked places, stale rows. checked: path prefix →
        whether that part of the tree was checked in this run (a row of a part not checked is not judged)."""
        rel = f"tools/i18n/{self.CONTEXT}"
        for no, msg in self.context_problems:
            rep.add("glossary-context", rel, no, msg)
        for c in self.context:
            parts = {next((pre for pre in checked if f.startswith(pre)), None) for f, _k in c.places}
            if None in parts:
                rep.add("glossary-context", rel, c.line, f"“{c.nl}”: a place outside {sorted(checked)} (this checker "
                                                          "does not visit it; check_pairs.py has GLOSSARY_OK)")
                continue
            if not all(checked[pre] for pre in parts):
                continue
            for f, _k in c.places:
                if not any(fnmatch.fnmatchcase(s, f) for s in self.seen_files):
                    rep.add("glossary-context", rel, c.line, f"“{c.nl}”: {f} is no checked file")
            if c.hits == 0:
                rep.add("glossary-context", rel, c.line, f"“{c.nl}” → “{c.en}”: no checked text at its places has "
                                                          "the nl phrase any more (stale row)")


COUNT_NOUN = re.compile(r"\{(n|count|c)\}\s+([A-Za-z]+)")


def plural_noun_after_count(en: str) -> str | None:
    """'{n} hours' → 'hours' when that word is an English plural noun (best effort: dictionary + singular stem)."""
    words = corpus.english_words()
    for m in COUNT_NOUN.finditer(en):
        w = m.group(2).lower()
        if len(w) < 4 or not w.endswith("s") or w.endswith("ss"):
            continue
        if not words:                       # no word list installed: every -s word after a count
            return m.group(2)
        stems = [w[:-1], w[:-2] if w.endswith("es") else None, (w[:-3] + "y") if w.endswith("ies") else None]
        if any(s and s in words for s in stems) and (not words or w in words or w[:-1] in words):
            return m.group(2)
    return None


ARTICLE = re.compile(r"(?:^|\s)(a|an|A|An) \{\w+\}")


# ------------------------------------------------------------------------------------------------------------------
# shared rule "tsv": TSV catalogues
# ------------------------------------------------------------------------------------------------------------------

@dataclass
class Row:
    file: str
    line: int
    key: str
    nl: str
    en: str
    glossary_ok: set = field(default_factory=set)


def catalog_files(root: Path) -> list[Path]:
    """host/i18n/*.tsv (D-G8) and any i18n/**/*.tsv, sorted."""
    out: list[Path] = []
    for base, pattern in ((root / "host" / "i18n", "*.tsv"), (root / "i18n", "**/*.tsv")):
        if base.exists():
            out.extend(sorted(base.glob(pattern)))
    return out


def check_catalogs(root: Path, rep: Report, gloss: Glossary, files: list[Path] | None = None) -> dict[str, Row]:
    rows: dict[str, Row] = {}
    files = catalog_files(root) if files is None else files
    rep.stats["tsv_files"] = len(files)
    for path in files:
        rel = path.relative_to(root).as_posix()
        stem = path.stem
        data = path.read_bytes()
        try:
            text = data.decode("utf-8")
        except UnicodeDecodeError as e:
            rep.add("tsv-encoding", rel, 0, f"not UTF-8: {e}")
            continue
        if "\r" in text:
            rep.add("tsv-format", rel, 0, "CR characters (LF line endings only)")
        if text and not text.endswith("\n"):
            rep.add("tsv-format", rel, text.count("\n") + 1, "no newline at the end of the file", level="warning")
        gok: set[str] = set()
        own: dict[str, int] = {}
        for no, line in enumerate(text.split("\n"), 1):
            line = line.rstrip("\r")
            if not line.strip():
                gok = set()
                continue
            if line.startswith("#"):
                m = re.match(r"#\s*glossary-ok:\s*(.+)$", line)
                if m:
                    gok |= {t.strip() for t in m.group(1).split(",") if t.strip()}
                continue
            parts = line.split("\t")
            row_gok, gok = gok, set()
            if len(parts) != 3:
                rep.add("tsv-fields", rel, no, f"expected 3 tab-separated fields (key, nl, en), found {len(parts)}")
                continue
            key, nl, en = parts
            if not KEY_RE.match(key):
                rep.add("tsv-key", rel, no, f"bad key {key!r} (prefix.name, letters, digits and _ only)")
            elif key.split(".")[0] != stem:
                rep.add("tsv-prefix", rel, no, f"key {key!r} must start with '{stem}.' (the file stem)")
            if key in own:
                rep.add("tsv-duplicate", rel, no, f"duplicate key {key!r} (also line {own[key]})")
            elif key in rows:
                o = rows[key]
                rep.add("tsv-duplicate", rel, no, f"duplicate key {key!r} (also {o.file}:{o.line})")
            own[key] = no
            for col, val in (("nl", nl), ("en", en)):
                if not val.strip():
                    rep.add("tsv-empty", rel, no, f"{key}: empty {col} value (a step is done only when its English is "
                                                  "written)")
                    continue
                if val != val.strip(" "):
                    rep.add("tsv-space", rel, no, f"{key}: {col} value starts or ends with a space", level="warning")
                if re.search(r"\\(?![nt\\])", val.replace("\\\\", "")):
                    rep.add("tsv-escape", rel, no, f"{key}: {col}: stray backslash (only \\n, \\t and \\\\ exist)")
            if nl.strip() and en.strip():
                pn, pe = placeholders(nl), placeholders(en)
                if pn != pe:
                    rep.add("tsv-placeholders", rel, no, f"{key}: nl has {sorted(pn)}, en has {sorted(pe)}")
                if ARTICLE.search(en):
                    rep.add("article", rel, no, f"{key}: en uses an article before a placeholder ({en!r}); "
                                                "write 'New {type}' / 'Create {type}'")
                for gnl, gen in gloss.check(nl, en, row_gok, where=(rel, key)):
                    rep.add("glossary", rel, no, f"{key}: nl “{gnl}” → en should contain “{gen}”")
            rows.setdefault(key, Row(rel, no, key, nl, en, row_gok))
    # plurals: .one / .zero need .other
    for key, row in rows.items():
        head, _, form = key.rpartition(".")
        if form in ("one", "zero") and f"{head}.other" not in rows:
            rep.add("tsv-plural", row.file, row.line, f"{key} has no {head}.other row")
    rep.stats["tsv_rows"] = len(rows)
    return rows


# ------------------------------------------------------------------------------------------------------------------
# shared rule "host-json": the Rust host's JSON {nl, en} tables
# ------------------------------------------------------------------------------------------------------------------

def check_host_json(root: Path, rep: Report, gloss: Glossary) -> None:
    n = 0
    for rel, tables in corpus.host_json_tables(root).items():
        nl, en = tables.get("nl", {}), tables.get("en", {})
        for k in sorted(set(nl) ^ set(en)):
            rep.add("str-table", rel, 0, f"key {k!r} only in {'nl' if k in nl else 'en'}")
        for k in sorted(set(nl) & set(en)):
            n += 1
            if not nl[k].strip() or not en[k].strip():
                rep.add("str-table", rel, 0, f"{k}: empty {'nl' if not nl[k].strip() else 'en'} value")
                continue
            pn, pe = placeholders(nl[k]), placeholders(en[k])
            if pn != pe:
                rep.add("str-table", rel, 0, f"{k}: nl has {sorted(pn)}, en has {sorted(pe)}")
            for gnl, gen in gloss.check(nl[k], en[k], set(), where=(rel, k)):
                rep.add("glossary", rel, 0, f"{k}: nl “{gnl}” → en should contain “{gen}”")
    rep.stats["host_json_rows"] = n


# ------------------------------------------------------------------------------------------------------------------
# shared rule "readme"
# ------------------------------------------------------------------------------------------------------------------

def check_shapes(root: Path, rep: Report) -> None:
    a, b = root / "README.md", root / "README.en.md"
    if a.exists() and b.exists():
        def shape(p: Path) -> tuple[list[int], list[str]]:
            heads, fences, cur, inside = [], [], [], False
            for line in p.read_text(encoding="utf-8").split("\n"):
                if line.startswith("```"):
                    if inside:
                        fences.append("\n".join(cur))
                        cur = []
                    inside = not inside
                    continue
                if inside:
                    cur.append(line)
                elif re.match(r"#{1,6} ", line):
                    heads.append(len(line) - len(line.lstrip("#")))
            return heads, fences
        (ha, fa), (hb, fb) = shape(a), shape(b)
        if ha != hb:
            rep.add("readme", "README.en.md", 0, f"heading levels differ from README.md ({len(ha)} vs {len(hb)})")
        if fa != fb:
            rep.add("readme", "README.en.md", 0, "fenced command blocks differ from README.md")


# ------------------------------------------------------------------------------------------------------------------
# dispatcher
# ------------------------------------------------------------------------------------------------------------------

# what a layer that runs as its own script takes over from the shared rules (its file has no check_layer())
SCRIPT_REPLACES = {"rust": {"tsv"}}
ERROR_LINE = re.compile(r"^(?:error:?|ERROR)\s+(.*)$")
WARNING_LINE = re.compile(r"^(?:warning:|warn)\s+(.*)$")


def load_layers(names: list[str] | None = None):
    """→ ([(layer, module or None, path)], [missing layer], [(layer, import error)]); module None = run as a script"""
    mods, missing, broken = [], [], []
    for layer, modname in LAYERS:
        if names is not None and layer not in names:
            continue
        path = HERE / f"{modname}.py"
        if not path.exists():
            missing.append(layer)
            continue
        if "def check_layer(" not in path.read_text(encoding="utf-8"):
            mods.append((layer, None, path))
            continue
        try:
            mods.append((layer, importlib.import_module(modname), path))
        except Exception as e:                    # a broken layer is an error, never a silent skip
            broken.append((layer, f"{type(e).__name__}: {e}"))
    return mods, missing, broken


def run_script_layer(layer: str, path: Path, rep: Report) -> None:
    """A layer with its own main(): run it (python3 -B FILE --warnings), take its error and warning lines over."""
    rel = path.relative_to(ROOT).as_posix()
    try:
        r = subprocess.run([sys.executable, "-B", str(path), "--warnings"], capture_output=True, text=True,
                           timeout=600, cwd=str(ROOT))
    except (OSError, subprocess.TimeoutExpired) as e:
        rep.add("layer-run", rel, 0, f"layer {layer} did not run: {e}")
        return
    errors = warnings = 0
    lines = [ln for ln in r.stdout.split("\n") if ln.strip()]
    for ln in lines:
        m = ERROR_LINE.match(ln)
        if m:
            errors += 1
            rep.add(f"{layer}", rel, 0, m.group(1), level="error")
            continue
        m = WARNING_LINE.match(ln)
        if m:
            warnings += 1
            rep.add(f"{layer}", rel, 0, m.group(1), level="warning")
    if r.returncode != 0 and not errors:
        rep.add("layer-run", rel, 0, f"layer {layer} exited {r.returncode} without an error line: "
                                     f"{(r.stderr or r.stdout).strip()[-300:]}")
    rep.stats[f"{layer}"] = (lines[-1] if lines else f"exit {r.returncode}").replace(", ", "; ")


def run(root: Path | str = ROOT, js: bool = True, js_data: dict | None = None,
        layers: list[str] | None = None) -> Report:
    root = Path(root).resolve()
    rep = Report()
    mods, missing, broken = load_layers(layers)
    replaced: set[str] = set()
    for layer, mod, _path in mods:
        if mod is None:
            replaced |= SCRIPT_REPLACES.get(layer, set())
        else:
            DEFERRED.update(getattr(mod, "DEFERRED", {}) or {})
            replaced |= set(getattr(mod, "REPLACES", ()) or ())
    gloss = Glossary(HERE / "glossary.tsv")
    rep.stats["glossary_rows"] = len(gloss.rows)
    if layers is None:
        if "tsv" not in replaced:
            check_catalogs(root, rep, gloss)
        if "host-json" not in replaced:
            check_host_json(root, rep, gloss)
        check_shapes(root, rep)
    names = dict(LAYERS)
    for layer, err in broken:
        rep.add("layer-import", f"tools/i18n/{names[layer]}.py", 0, f"layer {layer} could not be loaded: {err}")
    opts = {"js": js, "js_data": js_data}
    for layer, mod, path in mods:
        if mod is None:
            run_script_layer(layer, path, rep)
        else:
            mod.check_layer(root, rep, gloss, opts)
    rep.stats["layers"] = ",".join(layer for layer, _m, _p in mods) or "-"
    # glossary-context.tsv: judged only for the parts of the tree this run checked (the UI needs node and the ui
    # layer; the host JSON tables the shared host-json rule)
    gloss.check_context(rep, {paths.UI_REL + "/": "js_keys" in rep.stats, "host/src/": "host_json_rows" in rep.stats})
    rep.stats["glossary_context_rows"] = len(gloss.context)
    if missing:
        rep.stats["layers_missing"] = ",".join(missing)
    return rep


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--root", default=str(ROOT))
    ap.add_argument("--json", action="store_true", help="the whole report as JSON")
    ap.add_argument("--warnings", action="store_true", help="list every warning")
    ap.add_argument("--no-js", action="store_true", help="skip the UI dictionaries (no node)")
    ap.add_argument("--layers", help="only these layers (comma-separated: ui, rust, pairs); skips the shared rules")
    a = ap.parse_args(argv)
    layers = [x.strip() for x in a.layers.split(",") if x.strip()] if a.layers else None
    rep = run(a.root, js=not a.no_js, layers=layers)
    if a.json:
        print(json.dumps(rep.to_json(), ensure_ascii=False, indent=1))
        return 1 if rep.errors else 0
    for i in rep.errors:
        print(i.text())
    if a.warnings:
        for i in rep.warnings:
            print(i.text())
    counts: dict[str, int] = {}
    for i in rep.warnings:
        counts[i.rule] = counts.get(i.rule, 0) + 1
    print(f"i18n check: {len(rep.errors)} error(s), {len(rep.warnings)} warning(s)"
          + (" (" + ", ".join(f"{k} {v}" + (f" until {DEFERRED[k]}" if k in DEFERRED else "")
                              for k, v in sorted(counts.items())) + ")" if counts else ""))
    print("stats: " + ", ".join(f"{k}={v}" for k, v in rep.stats.items()))
    return 1 if rep.errors else 0


if __name__ == "__main__":
    sys.exit(main())
