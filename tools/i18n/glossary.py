"""Glossary report: which English the project uses for each Dutch term (H: notes/english-option-design.md §4.1, E0b;
ported to Win11 voor GNOME in GE0a, notes/claude/round0/ge0a.md).

    python3 tools/i18n/glossary.py              the report (text)
    python3 tools/i18n/glossary.py --json       the same as JSON
    python3 tools/i18n/glossary.py --term Schijf --examples 20

Pairs read: every UI makeT key (per plural form), inline {nl, en} pair and settings-registry P() title/description
(jsdicts.mjs over vendor/ui/js, needs node), every TSV catalog row (i18n/**/*.tsv, none at 3b898e6), and the Rust
host's embedded JSON {nl, en} tables (corpus.host_json_tables: sysmon, disks, storage, system). H's Python-host
reader (ast over host/*.py) is dropped. Search keywords are left out: they list synonyms on purpose.

For every term of tools/i18n/glossary.tsv, plus the context-dependent terms and the two "(check)" rows that the
glossary leaves out (OPEN below), the report counts the pairs whose nl text has the term (at word boundaries) and
how their en text renders it: each candidate English phrase with the casing used, or "other". The case of the
first letter does not count (sentence case: "Taskbar alignment", "on the taskbar"), any other does ("Quick
settings" and "Quick Settings" are two renderings). The report lists

1. *variants*: terms with more than one rendering (E8 or Robin picks one);
2. the open terms (not in glossary.tsv) with their renderings, so the choice can be made;
3. terms whose glossary phrase is missing from some pairs ("other": inflected, rephrased, or wrong; E8 reviews);
4. the known deviations of §4.2 ("Trash", "Airplane mode", "Log out"/"Log in"), with every place they occur;
5. the context rows of tools/i18n/glossary-context.tsv (R3-UI): a pair at a context row's place that has the row's
   en phrase counts as the rendering "context", not as "other"; rows flagged "known" are accepted deviations.
"""

from __future__ import annotations

import argparse
import collections
import json
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402
import paths  # noqa: E402

ROOT = paths.ROOT
CONTEXT = HERE / "glossary-context.tsv"
UI_PREFIX = paths.UI_REL + "/"
GLOSSARY = HERE / "glossary.tsv"

# Terms the glossary leaves out (context decides), with the renderings to look for. The first is the design's
# choice where it has one (§4.2).
OPEN = {
    "Snelle instellingen": ["Quick Settings", "Quick settings"],
    "Batterijbesparing": ["Battery saver", "Energy saver"],
    "Schijf": ["Disk", "Drive", "Storage"],
    "Station": ["Drive", "Station"],
    "Info": ["About", "Info", "Information"],
    "Weergave": ["View", "Display", "Appearance", "Layout"],
    "Afsluiten": ["Shut down", "Close", "Exit", "Quit"],
    "Start": ["Start", "Home"],             # case-sensitive (the menu, Verkenner's Home; not the verb "start")
    "Alles": ["Alles", "All", "Everything"],
    "Volume": ["Volume"],
    "Vliegtuigmodus": ["Flight mode", "Airplane mode"],   # the nl UI says this, not the glossary's "Vliegtuigstand"
}
CASE_SENSITIVE = {"Start", "Alles", "Info"}
# Known deviations from the glossary (§4.2): en phrase → what it should be
DEVIATIONS = {                      # matched case-insensitively, at word boundaries
    "Trash": "Recycle Bin",
    "Airplane mode": "Flight mode",
    "Log out": "Sign out",
    "Log in": "Sign in",
    "Logout": "Sign out",
}


@dataclass
class Pair:
    nl: str
    en: str
    where: str          # file:line key
    file: str = ""      # below the repo (a context row's place)
    key: str = ""       # makeT key, P(<id>).title|description, pair, or a host table key


@dataclass
class Term:
    nl: str
    candidates: list[str]
    name: bool = False
    in_glossary: bool = True
    pairs: list = field(default_factory=list)
    renderings: collections.Counter = field(default_factory=collections.Counter)
    examples: dict = field(default_factory=lambda: collections.defaultdict(list))

    @property
    def variants(self) -> list[str]:
        return [r for r in self.renderings if r not in ("other", "context")]

    @property
    def uneven(self) -> bool:
        return len(self.variants) > 1

    @property
    def missing(self) -> int:
        return self.renderings.get("other", 0) if self.in_glossary else 0


# ------------------------------------------------------------------------------------------------ pairs
def _s(v) -> str | None:
    if isinstance(v, dict) and "s" in v:
        return v["s"]
    if isinstance(v, dict) and "fn" in v:
        return " … ".join(x for x in v["fn"] if isinstance(x, str)) or None
    return None


def ui_pairs(data: dict) -> list[Pair]:
    out = []
    for d in data.get("dicts", []):
        nl, en = (d.get("langs") or {}).get("nl") or {}, (d.get("langs") or {}).get("en") or {}
        for key, nv in nl.items():
            ev = en.get(key)
            if ev is None:
                continue
            where = f"{UI_PREFIX}{d['file']}:{nv.get('line', d['line'])} {key}"
            file = f"{UI_PREFIX}{d['file']}"
            if "plural" in nv and "plural" in ev:
                for form, fv in nv["plural"].items():
                    a, b = _s(fv), _s(ev["plural"].get(form) or ev["plural"].get("other"))
                    if a and b:
                        out.append(Pair(a, b, f"{where}.{form}", file, key))
                continue
            a, b = _s(nv), _s(ev)
            if a and b:
                out.append(Pair(a, b, where, file, key))
    for p in data.get("pairs", []):
        if p.get("what") == "keywords":
            continue
        if isinstance(p.get("nl"), str) and isinstance(p.get("en"), str):
            key = f"P({p.get('id')}).{p['what']}" if p["kind"] == "P" else "pair"
            out.append(Pair(p["nl"], p["en"], f"{UI_PREFIX}{p['file']}:{p['line']} {p.get('what') or p['kind']}",
                            f"{UI_PREFIX}{p['file']}", key))
    return out


def tsv_pairs(root: Path) -> list[Pair]:
    out = []
    base = Path(root) / "i18n"
    for f in sorted(base.rglob("*.tsv")) if base.exists() else []:
        rel = f.relative_to(root).as_posix()
        for no, line in enumerate(f.read_text(encoding="utf-8").split("\n"), 1):
            parts = line.split("\t")
            if line.startswith("#") or len(parts) < 3:
                continue
            out.append(Pair(parts[1], parts[2], f"{rel}:{no} {parts[0]}", rel, parts[0]))
    return out


def host_pairs(root: Path) -> list[Pair]:
    """The Rust host's embedded JSON {nl, en} tables (G; H read {"nl": …, "en": …} literals of its Python host)."""
    out = []
    for rel, tables in corpus.host_json_tables(root).items():
        nl, en = tables.get("nl", {}), tables.get("en", {})
        for k, v in nl.items():
            if isinstance(en.get(k), str):
                out.append(Pair(v, en[k], f"{rel} {k}", rel, k))
    return out


def all_pairs(root: Path = ROOT, data: dict | None = None) -> list[Pair]:
    data = data if data is not None else corpus.jsdicts(root)
    return ui_pairs(data) + tsv_pairs(root) + host_pairs(root)


# ------------------------------------------------------------------------------------------------ report
def _rx(phrase: str, case: bool) -> re.Pattern:
    return re.compile(r"(?<![\w’'])" + re.escape(phrase) + r"(?!\w)", 0 if case else re.IGNORECASE)


def load_terms(path: Path = GLOSSARY) -> list[Term]:
    terms = []
    for line in path.read_text(encoding="utf-8").split("\n"):
        if not line.strip() or line.startswith("#"):
            continue
        p = line.split("\t")
        if len(p) >= 2:
            terms.append(Term(p[0], [p[1]], name="name" in (p[2] if len(p) > 2 else "")))
    have = {t.nl for t in terms}
    for nl, cands in OPEN.items():
        if nl not in have:
            terms.append(Term(nl, cands, name=nl in CASE_SENSITIVE, in_glossary=False))
    return terms


def load_context(path: Path = CONTEXT) -> list:
    """The context rows (check.Glossary's reader, so the report and the checker agree)."""
    sys.path.insert(0, str(HERE))
    import check  # noqa: E402
    return check.Glossary(GLOSSARY, path).context


def analyse(pairs: list[Pair], terms: list[Term], context: list | None = None) -> list[Term]:
    # longest nl phrase first: "Prullenbak leegmaken" claims its pairs before "Prullenbak" counts them
    for t in sorted(terms, key=lambda t: -len(t.nl)):
        rx = _rx(t.nl, t.name)
        longer = [o for o in terms if o is not t and len(o.nl) > len(t.nl) and t.nl.lower() in o.nl.lower()]
        for p in pairs:
            m = rx.search(p.nl)
            if not m:
                continue
            masked = p.nl
            for o in longer:          # this occurrence is part of a longer term
                masked = _rx(o.nl, o.name).sub(lambda x: "\0" * len(x.group(0)), masked)
            if not rx.search(masked):
                continue
            t.pairs.append(p)
            # a context row of this term (or of a longer phrase with it) at this pair's place
            crow = next((c for c in context or [] if t.nl.lower() in c.nl.lower() and c.applies(p.file, p.key)), None)
            if crow is not None and crow.rx_en.search(p.en):
                t.renderings["context"] += 1
                t.examples["context"].append(p)
                continue
            got = None
            for c in t.candidates:
                mm = _rx(c, False).search(p.en) or re.search(r"(?<![\w’'])" + re.escape(c), p.en, re.IGNORECASE)
                if mm:
                    got = mm.group(0)
                    break
            key = (got[0].upper() + got[1:]) if got else "other"     # sentence case: the first letter does not count
            t.renderings[key] += 1
            t.examples[key].append(p)
    return terms


def deviations(pairs: list[Pair]) -> dict[str, list[Pair]]:
    out: dict[str, list[Pair]] = {}
    for bad in DEVIATIONS:
        rx = re.compile(r"(?<![\w’'])" + re.escape(bad) + r"(?!\w)", re.IGNORECASE)
        hits = [p for p in pairs if rx.search(p.en)]
        if hits:
            out[bad] = hits
    return out


def text_report(terms: list[Term], devs: dict, examples: int = 3, only: str | None = None,
                context: list | None = None) -> str:
    lines = []
    sel = [t for t in terms if (only is None or t.nl.lower() == only.lower())]
    used = [t for t in sel if t.pairs]
    var = [t for t in used if t.in_glossary and t.uneven]
    opn = [t for t in sel if not t.in_glossary]
    miss = [t for t in used if t.missing]
    unused = [t for t in sel if not t.pairs]
    lines.append(f"Glossary report: {len(sel)} terms ({len(opn)} open), {len(used)} used in a pair, {len(var)} with "
                 f"variants, {len(miss)} with pairs that lack the glossary phrase, {len(unused)} unused.")

    def show(t: Term, all_kinds: bool) -> None:
        mark = "" if t.in_glossary else "  [open: not in glossary.tsv]"
        lines.append(f"{t.nl} → {t.candidates[0]}{mark}: {len(t.pairs)} pairs; " +
                     ", ".join(f"{r!r} {n}" for r, n in t.renderings.most_common()))
        for r, n in t.renderings.most_common():
            if r == "other" and not all_kinds:
                continue
            for p in t.examples[r][:examples]:
                lines.append(f"    [{r}] {p.where}: {p.nl!r} → {p.en!r}")

    lines += ["", "== 1. Variants: more than one rendering (the case of the first letter does not count) =="]
    for t in var:
        show(t, False)
    if not var:
        lines.append("none")
    lines += ["", "== 2. Open terms (glossary.tsv leaves them out; context decides) =="]
    for t in opn:
        show(t, True)
    lines += ["", "== 3. Pairs without the glossary phrase (inflected, rephrased or wrong; E8 reviews) =="]
    for t in miss:
        lines.append(f"{t.nl} → {t.candidates[0]}: {t.missing} of {len(t.pairs)}")
        for p in t.examples["other"][:examples]:
            lines.append(f"    {p.where}: {p.nl!r} → {p.en!r}")
    if unused:
        lines += ["", "== Not used in any pair ==", ", ".join(t.nl for t in unused)]
    lines += ["", "== 4. Known deviations (§4.2) =="]
    for bad, hits in devs.items():
        lines.append(f"{bad!r} (should be {DEVIATIONS[bad]!r}): {len(hits)}")
        for p in hits[: max(examples, 30)]:
            lines.append(f"    {p.where}: {p.en!r}")
    if not devs:
        lines.append("none")
    lines += ["", "== 5. Context rows (tools/i18n/glossary-context.tsv) =="]
    for c in context or []:
        mark = "  [known deviation]" if c.known else ""
        places = ", ".join(f"{f}:{k}" for f, k in c.places)
        lines.append(f"{c.nl} → {c.en}{mark}: {places} ({c.why})")
    if not context:
        lines.append("none")
    return "\n".join(lines)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--json", action="store_true")
    ap.add_argument("--term")
    ap.add_argument("--examples", type=int, default=3)
    ap.add_argument("--root", default=str(ROOT))
    a = ap.parse_args(argv)
    root = Path(a.root)
    pairs = all_pairs(root)
    context = load_context()
    terms = analyse(pairs, load_terms(), context)
    devs = deviations(pairs)
    if a.json:
        print(json.dumps({
            "pairs": len(pairs),
            "terms": [{"nl": t.nl, "want": t.candidates[0], "in_glossary": t.in_glossary, "pairs": len(t.pairs),
                       "variants": t.uneven, "missing": t.missing, "renderings": dict(t.renderings),
                       "examples": {r: [p.where for p in ps[:a.examples]] for r, ps in t.examples.items()}}
                      for t in terms],
            "deviations": {k: [p.where for p in v] for k, v in devs.items()},
            "context": [{"line": c.line, "places": [f"{f}:{k}" for f, k in c.places], "nl": c.nl, "en": c.en,
                         "known": c.known, "why": c.why} for c in context],
        }, ensure_ascii=False, indent=1))
        return 0
    print(f"{len(pairs)} nl/en pairs read.")
    print(text_report(terms, devs, a.examples, a.term, context))
    return 0


if __name__ == "__main__":
    sys.exit(main())
