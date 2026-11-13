"""Dutch words in an English UI text dump (H: notes/english-option-design.md §3.4, E0b; ported to Win11 voor GNOME
in GE0a, notes/claude/round0/ge0a.md; warning-only).

    python3 tools/i18n/scan_en.py tests/i18n/baseline-en.txt
    tests/i18n/run.sh --lang en --out FILE && python3 tools/i18n/scan_en.py FILE
    python3 tools/i18n/scan_en.py FILE --strict        exit 1 on any hit (round 3 makes it blocking)

Every text line of the dump (not the "== id url ==" headers, not "# …" lines) is matched against
tests/i18n/dutch-words.txt; words and phrases in tests/i18n/allow-en.txt (one per line: mock user data such as file
names, the proper names "Alles" and "Win11 Shell") are removed first. Also reports the dump's own
"# i18n missing/unfilled" lines. Prints, per surface, the lines with Dutch and their words, then a word count.
allow-en.txt does not exist yet in G: it comes with the English review (round 3).
"""

from __future__ import annotations

import argparse
import collections
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent.parent
WORD = re.compile(r"[A-Za-zÀ-ÖØ-öø-ÿ]+(?:['’][a-zë]+)?")


def load(path: Path) -> list[str]:
    if not path.exists():
        return []
    return [ln.strip() for ln in path.read_text(encoding="utf-8").split("\n") if ln.strip() and not ln.startswith("#")]


def scan(dump: str, words: set[str], allow: list[str]) -> tuple[dict[str, list[tuple[str, list[str]]]], list[str]]:
    per: dict[str, list[tuple[str, list[str]]]] = collections.OrderedDict()
    report = []
    cur = "?"
    allow_rx = [re.compile(r"(?<!\w)" + re.escape(a) + r"(?!\w)") for a in sorted(allow, key=len, reverse=True)]
    for ln in dump.split("\n"):
        if ln.startswith("== "):
            cur = ln.split(" ")[1]
            continue
        if ln.startswith("# i18n "):
            report.append(f"{cur}: {ln[2:]}")
            continue
        if not ln.strip() or ln.startswith("#"):
            continue
        text = re.sub(r"^\s*\[[a-z-]+\] ", "", ln)
        for rx in allow_rx:
            text = rx.sub(" ", text)
        hits = [w for w in WORD.findall(text) if w.lower().replace("’", "'") in words]
        if hits:
            per.setdefault(cur, []).append((ln.strip(), hits))
    return per, report


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("dump")
    ap.add_argument("--strict", action="store_true")
    ap.add_argument("--lines", type=int, default=5, help="lines shown per surface")
    a = ap.parse_args(argv)
    words = set(load(ROOT / "tests" / "i18n" / "dutch-words.txt"))
    allow = load(ROOT / "tests" / "i18n" / "allow-en.txt")
    per, report = scan(Path(a.dump).read_text(encoding="utf-8"), words, allow)
    total = collections.Counter(w.lower() for hits in per.values() for _ln, ws in hits for w in ws)
    for sid, hits in per.items():
        print(f"{sid}: {len(hits)} line(s) with Dutch")
        for ln, ws in hits[: a.lines]:
            print(f"    {ln[:140]}   ← {', '.join(ws)}")
    for r in report:
        print("report:", r)
    print(f"{sum(len(h) for h in per.values())} lines on {len(per)} surfaces; words: "
          + ", ".join(f"{w} {n}" for w, n in total.most_common(40)))
    return 1 if a.strict and (per or report) else 0


if __name__ == "__main__":
    sys.exit(main())
