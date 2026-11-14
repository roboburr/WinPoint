"""tests/i18n/dutch-words.txt: the Dutch vocabulary of the ratchet scans (H: notes/english-option-design.md §3.2,
E0b; ported to Win11 voor GNOME in GE0a, notes/claude/round0/ge0a.md).

G uses H's committed list unchanged (1,295 words, generated from H's tree on 2026-11-05); this script only shows what
a regeneration from G's tree would add or drop. Regenerate (--write) only as a reviewed decision.


    python3 tools/i18n/dutch_words.py            stats, and the words a regeneration would add or drop (no write)
    python3 tools/i18n/dutch_words.py --write    (re)write tests/i18n/dutch-words.txt

The list was generated once in E0b and is then committed: later steps do not regenerate it (a new Dutch word in a
dictionary does not need to be in it; the dynamic en dump of §3.4 covers recall). Rule:

1. every word of every nl text (in E0b the UI dictionaries, inline pairs and settings-registry P() data; since the
   WebKit removal the native programs' nl/en pairs and the settings registry; the nl column of the TSV catalogs, the
   Rust host's JSON tables: tools/i18n/corpus.py), lower-case, without {placeholders};
2. minus words of fewer than 3 letters (except SHORT), minus every word of /usr/share/dict/british-english and
   american-english (except ADD), minus words the en texts use at least a third as often (names, units, loanwords);
3. minus DROP: the hand review of E0b (2026-11-05) — loanwords and technical terms that English UI text uses too.

Needs the two word lists (Ubuntu: wbritish, wamerican).
"""

from __future__ import annotations

import argparse
import collections
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import corpus  # noqa: E402
import paths  # noqa: E402

ROOT = paths.ROOT
OUT = paths.tests_dir(ROOT) / "dutch-words.txt"

# Two-letter words that only Dutch text uses ("je", "de", "te" …). Not: en/of/op/is/in/er/al/af/na (English words,
# language codes, abbreviations).
SHORT = {"je", "de", "te", "nu", "om", "ze", "zo", "ja", "ga"}

# In an English word list, but in this project's string literals they are Dutch (hand review): function words and
# nouns that English UI text does not use. Not added (common English as well): door, even, via, taken, past, let,
# lang, land, hand, begin, direct, basis, per, pas, los, vol, map, pad, dan, die, datum, agenda, station(s), diverse,
# raster, reserve, defect, intern, laptop, gem, ding, vast.
ADD = {"van", "met", "dat", "weer", "tot", "hoe", "wilt", "laden", "paar", "elk", "heel", "erin", "bent", "toe",
       "hun", "hem", "meld", "melding", "zit", "koppel", "doe", "moet", "steeds", "balk", "engels", "klein", "punt",
       "twee", "video's", "menu's", "tv's", "pc's"}

# Generated but not Dutch-only (hand review): loanwords and technical terms that English text uses too, compounds
# that look like identifiers, and "alles": the app name Alles (D5) in both languages, and the stem of its CSS
# classes and ids ('alles-row', 'win11shell.alles.view'); "Alles selecteren" still has "selecteren".
DROP = {"realtime", "tpm", "itemtype", "topapps", "alles"}


def generate(root: Path = ROOT, texts=None) -> tuple[list[str], dict]:
    texts = texts if texts is not None else corpus.load(root)
    nl, en = collections.Counter(), collections.Counter()
    for t in texts:
        (nl if t.lang == "nl" else en).update(corpus.words(t.text))
    eng = corpus.english_words()
    if not eng:
        raise RuntimeError("no English word list (/usr/share/dict/british-english, american-english)")
    out = set()
    for w, c in nl.items():
        if w in DROP:
            continue
        if len(w) < 3 and w not in SHORT:
            continue
        if w in eng and w not in ADD:
            continue
        if w not in ADD and w not in SHORT and en.get(w, 0) >= c / 3:
            continue
        out.add(w)
    stats = {"texts": len(texts), "nl_words": len(nl), "en_words": len(en), "kept": len(out)}
    return sorted(out), stats


HEADER = """# Dutch words of the i18n ratchet scans (notes/english-option-design.md §3.2), one per line, lower case.
# Generated once in E0b (2026-11-05) by tools/i18n/dutch_words.py from every nl dictionary value, inline pair,
# settings-registry text, TSV nl column and host _STR value, minus /usr/share/dict/{british,american}-english, then
# reviewed by hand (the SHORT / ADD / DROP sets in that script). Committed: later steps do not regenerate it.
"""


def read(path: Path = OUT) -> set[str]:
    if not path.exists():
        return set()
    return {ln.strip() for ln in path.read_text(encoding="utf-8").split("\n") if ln.strip() and not ln.startswith("#")}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--root", default=str(ROOT))
    a = ap.parse_args(argv)
    words, stats = generate(Path(a.root))
    if a.write:
        OUT.write_text(HEADER + "\n".join(words) + "\n", encoding="utf-8")
        print(f"wrote {OUT} ({len(words)} words)")
        return 0
    have = read()
    print(" ".join(f"{k}={v}" for k, v in stats.items()), f"committed={len(have)}")
    new, gone = sorted(set(words) - have), sorted(have - set(words))
    if new:
        print("a regeneration would add:", " ".join(new))
    if gone:
        print("a regeneration would drop:", " ".join(gone))
    return 0


if __name__ == "__main__":
    sys.exit(main())
