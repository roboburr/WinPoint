"""Which surface blocks of a UI text dump changed (tests/i18n/run.sh --record; notes/english-option-design.md §3.3).

    python3 -I tests/i18n/blocks.py BASELINE NEW [ACCEPT]       exit 0: NEW may replace BASELINE; 1: refused

A block is the text between two ``== <id> …`` lines. A new block (an id the baseline lacks, e.g. a surface E1 adds)
is always fine; a changed or removed block must be named in ACCEPT (comma-separated ids, ``taskmgr.*`` for a
prefix, ``all``), so a re-record cannot silently take in a Dutch change. The diff of every refused block is
printed. A missing BASELINE accepts everything (the first recording).
"""

from __future__ import annotations

import difflib
import os
import re
import sys


def blocks(path: str) -> dict[str, list[str]]:
    out: dict[str, list[str]] = {}
    cur = None
    with open(path, encoding="utf-8") as f:
        for ln in f.read().split("\n"):
            if ln.startswith("== END "):
                cur = None
                continue
            m = re.match(r"^== (\S+) ", ln)
            if m:
                cur = m.group(1)
                out[cur] = []
            elif cur is not None:
                out[cur].append(ln)
    return out


def accepted(i: str, acc: list[str]) -> bool:
    return any(a == "all" or a == i or (a.endswith("*") and i.startswith(a[:-1])) for a in acc)


def main(argv: list[str]) -> int:
    base, new = argv[1], argv[2]
    acc = [a.strip() for a in (argv[3] if len(argv) > 3 else "").split(",") if a.strip()]
    if not os.path.exists(base):
        return 0
    a, b = blocks(base), blocks(new)
    added = [i for i in b if i not in a]
    changed = [i for i in a if i in b and a[i] != b[i]]
    removed = [i for i in a if i not in b]
    refused = [i for i in changed + removed if not accepted(i, acc)]
    for i in changed + removed:
        print(f"run.sh: {'changed' if i in b else 'removed'} block {i} "
              f"({'accepted' if accepted(i, acc) else 'NOT ACCEPTED'})")
        if i in refused and i in b:
            d = difflib.unified_diff(a[i], b[i], f"baseline/{i}", f"now/{i}", lineterm="", n=1)
            print("\n".join(list(d)[:80]))
    for i in added:
        print(f"run.sh: new block {i}")
    unused = [x for x in acc if x != "all" and not any(accepted(i, [x]) for i in changed + removed)]
    if unused:
        print(f"run.sh: --accept names no changed block: {', '.join(unused)}")
    if refused:
        print(f"run.sh: not recorded: {len(refused)} changed or removed block(s) not named in --accept (a Dutch "
              "change needs a tests/i18n/BASELINE-CHANGES entry first, then --record --accept ID,…)", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
