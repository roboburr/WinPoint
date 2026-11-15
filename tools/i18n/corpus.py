"""The project's language data as flat text records (H: notes/english-option-design.md §3.2, §4.1, step E0b; ported
to Win11 voor GNOME in GE0a, notes/claude/round0/ge0a.md).

    from corpus import load            (tools/i18n/ on sys.path)
    texts = load(ROOT)                 [Text(lang, text, file, line, key, kind)], nl and en

Sources: every nl/en pair of the native GTK 4 programs (shell, Calendar, kit, the seven apps: their calls, match
tables, row tables, the settings registry's titles, descriptions and keywords, NL/EN arrays and JSON objects, read by
check_native.py; ``kind`` is the pair's kind, ``keywords`` for a search list), the nl/en columns of every
``i18n/**/*.tsv`` row (none: the host's catalogues are host/i18n, read by check_rust.py), and the Rust host's
embedded JSON ``{nl, en}`` tables (``paths.HOST_JSON_TABLES``: sysmon, disks, storage, system; H read Python
``_STR`` dicts instead; ``kind`` ``str``). Used by dutch_words.py (the vocabulary of the Dutch word test),
glossary.py and check_rust.py (the label rule).

Dropped from H: the Python host ``_STR`` reader (G's host is Rust). Dropped with the WebKit apps: the UI dictionaries
of the HTML apps (jsdicts.mjs, node; ``git show 6ebf169f:tools/i18n/corpus.py``).
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path

import paths

HERE = paths.HERE
ROOT = paths.ROOT
PH = re.compile(r"\{[A-Za-z_]\w*\}|\$\{[^}]*\}|\$\{…\}")


@dataclass(frozen=True)
class Text:
    lang: str
    text: str
    file: str
    line: int
    key: str
    kind: str


def native_texts(root: Path) -> list[Text]:
    import check_native  # noqa: PLC0415  (it imports check, which imports this module)
    pairs, _ = check_native.read_pairs(root)
    out: list[Text] = []
    for p in pairs:
        kind = "keywords" if p.keywords else p.kind
        for lang, v in (("nl", p.nl), ("en", p.en)):
            for s in v if isinstance(v, list) else [v]:
                if isinstance(s, str) and s:
                    out.append(Text(lang, s, p.file, p.line, p.key, kind))
    return out


def tsv_texts(root: Path) -> list[Text]:
    out: list[Text] = []
    base = Path(root) / "i18n"
    for f in sorted(base.rglob("*.tsv")) if base.exists() else []:
        rel = f.relative_to(root).as_posix()
        for no, line in enumerate(f.read_text(encoding="utf-8").split("\n"), 1):
            if not line or line.startswith("#"):
                continue
            parts = line.split("\t")
            if len(parts) < 3:
                continue
            key, nl, en = parts[0], parts[1], parts[2]
            out.append(Text("nl", nl.replace("\\n", "\n").replace("\\t", "\t"), rel, no, key, "tsv"))
            out.append(Text("en", en.replace("\\n", "\n").replace("\\t", "\t"), rel, no, key, "tsv"))
    return out


def host_json_tables(root: Path) -> dict[str, dict[str, dict[str, str]]]:
    """file → {"nl": {key: text}, "en": {key: text}} of the Rust host's embedded JSON tables (paths.HOST_JSON_TABLES);
    a file that is missing (GE1b moved it to TSV) or has no such table is left out."""
    out: dict[str, dict[str, dict[str, str]]] = {}
    for rel, where in paths.HOST_JSON_TABLES.items():
        f = Path(root) / rel
        try:
            v = json.loads(f.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            continue
        for k in where:
            v = v.get(k) if isinstance(v, dict) else None
        if isinstance(v, dict) and isinstance(v.get("nl"), dict):
            out[rel] = {lg: {k: s for k, s in t.items() if isinstance(s, str)}
                        for lg, t in v.items() if lg in ("nl", "en") and isinstance(t, dict)}
    return out


def host_str_texts(root: Path) -> list[Text]:
    out: list[Text] = []
    for rel, tables in host_json_tables(root).items():
        for lang, rows in tables.items():
            for key, v in rows.items():
                out.append(Text(lang, v, rel, 0, key, "str"))
    return out


def load(root: Path | str = ROOT) -> list[Text]:
    root = Path(root)
    return native_texts(root) + tsv_texts(root) + host_str_texts(root)


WORD = re.compile(r"[A-Za-zÀ-ÖØ-öø-ÿ]+(?:['’][a-zë]+)?")


def words(s: str) -> list[str]:
    """Lower-case words of a text, without its {placeholders} and ${…} expressions."""
    return [w.lower().replace("’", "'") for w in WORD.findall(PH.sub(" ", s))]


ENGLISH: set[str] | None = None


def english_words() -> set[str]:
    """/usr/share/dict/british-english + american-english, lower-case (empty when neither is installed)."""
    global ENGLISH
    if ENGLISH is None:
        ENGLISH = set()
        for f in ("/usr/share/dict/british-english", "/usr/share/dict/american-english"):
            try:
                with open(f, encoding="utf-8", errors="replace") as fh:
                    ENGLISH.update(w.strip().lower() for w in fh if w.strip())
            except OSError:
                pass
    return ENGLISH
