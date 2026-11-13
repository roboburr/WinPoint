"""The project's language data as flat text records (H: notes/english-option-design.md §3.2, §4.1, step E0b; ported
to Win11 voor GNOME in GE0a, notes/claude/round0/ge0a.md).

    from corpus import load            (tools/i18n/ on sys.path)
    texts = load(ROOT)                 [Text(lang, text, file, line, key, kind)], nl and en

Sources: every UI ``makeT`` dictionary value (plural forms, function strings and template text included), every
inline ``{nl, en}`` pair and settings-registry ``P()`` title/description/keywords (all from ``jsdicts.mjs``, so node
is needed: ``$NODE`` or the PATH), the nl/en columns of every ``i18n/**/*.tsv`` row (none yet at 3b898e6; GE1b
adds ``i18n/host/*.tsv``), and the Rust host's embedded JSON ``{nl, en}`` tables (``paths.HOST_JSON_TABLES``: sysmon,
disks, storage, system; H read Python ``_STR`` dicts instead). ``kind`` is ``dict``, ``obj`` (an inline pair),
``P``, ``tsv`` or ``str``. Used by dutch_words.py (the vocabulary of the ratchet scans) and glossary.py.

Dropped from H: the Python host ``_STR`` reader (G's host is Rust).
"""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path

import paths

HERE = paths.HERE
ROOT = paths.ROOT
UI_PREFIX = paths.UI_REL + "/"
PH = re.compile(r"\{[A-Za-z_]\w*\}|\$\{[^}]*\}|\$\{…\}")


@dataclass(frozen=True)
class Text:
    lang: str
    text: str
    file: str
    line: int
    key: str
    kind: str


def find_node() -> str | None:
    cand = os.environ.get("NODE") or shutil.which("node")
    if cand:
        return cand
    for p in (Path.home() / ".local/node/bin/node", Path("/home/robin/.local/node/bin/node"),
              Path("/opt/node/bin/node")):                      # (a sandbox may bind node at either place)
        if p.exists():
            return str(p)
    return None


def jsdicts(root: Path, node: str | None = None) -> dict:
    node = node or find_node()
    if not node:
        raise RuntimeError("node not found (set $NODE): the UI dictionaries are read with tools/i18n/jsdicts.mjs")
    r = subprocess.run([node, str(HERE / "jsdicts.mjs"), str(paths.ui_js(root))], capture_output=True, text=True,
                       timeout=300)
    if r.returncode != 0:
        raise RuntimeError(f"jsdicts.mjs failed ({r.returncode}): {r.stderr[-2000:]}")
    return json.loads(r.stdout)


def _strings(v) -> list[str]:
    """All literal text of one jsdicts value."""
    if not isinstance(v, dict):
        return [v] if isinstance(v, str) else []
    if "s" in v:
        return [v["s"]]
    if "plural" in v:
        return [s for f in v["plural"].values() for s in _strings(f)]
    if "fn" in v:
        return [s for s in v["fn"] if isinstance(s, str)]
    return [s for s in v.get("strings") or [] if isinstance(s, str)]


def ui_texts(root: Path, data: dict | None = None) -> list[Text]:
    data = data if data is not None else jsdicts(root)
    out: list[Text] = []
    for d in data.get("dicts", []):
        for lang, entries in (d.get("langs") or {}).items():
            if lang not in ("nl", "en"):
                continue
            for key, v in entries.items():
                for s in _strings(v):
                    out.append(Text(lang, s, UI_PREFIX + d["file"], (v or {}).get("line", d["line"]), key, "dict"))
    for p in data.get("pairs", []):
        for lang in ("nl", "en"):
            v = p.get(lang)
            vals = v if isinstance(v, list) else [v]
            for s in vals:
                if isinstance(s, str):
                    out.append(Text(lang, s, UI_PREFIX + p["file"], p["line"], p.get("what") or "", p["kind"]))
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


def load(root: Path | str = ROOT, data: dict | None = None) -> list[Text]:
    root = Path(root)
    return ui_texts(root, data) + tsv_texts(root) + host_str_texts(root)


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
