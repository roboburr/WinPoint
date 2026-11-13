"""The "no visible string without a key" ratchet (H: notes/english-option-design.md §3.2, step E0b; ported to Win11
voor GNOME in GE0a, notes/claude/round0/ge0a.md).

    python3 tools/i18n/ratchet.py [ui …]                   check (default: every scan); exit 1 on a new or a stale entry
    python3 tools/i18n/ratchet.py --summary [ui …]         entries per file (to convert / keep)
    python3 tools/i18n/ratchet.py --prune [ui …] [--file F …]
                                                            drop the occurrences the tree no longer has (never adds);
                                                            with --file only those files' entries
    python3 tools/i18n/ratchet.py --allow FILE:HASH --reason WHY [--keep]
                                                            accept ONE text (the hash is printed by the check): with
                                                            --keep it is meant to stay (a D14 inline pair, a Dutch-only
                                                            fallback by design …), else it is a text still to convert
    python3 tools/i18n/ratchet.py --write [ui …]           show what regenerating the allowlist from the tree would add
         … --write --yes                                    and do it (GE0a, or a scanner change; keeps every reason)

Scans: ``ui`` = tools/i18n/scan_ui.mjs over vendor/ui/js (needs node: $NODE or the PATH). H's ``host`` (Python) and
``sh`` (bash) scans are dropped: G's host is Rust and its installer is Python. A Rust scan (GE0b/W6) is one more entry
in ALLOW, PREFIX, WHAT and scan(). Allowlist: tests/i18n/allow-ui.txt. An entry is ``file, sha1(text)[:12], count, kind,
text[, status]``: the text of a literal with its whitespace collapsed, so moving a line keeps its entry, and the
count, so a second copy of an allowed text in the same file is new. The status column is empty for a text that a
step still has to convert, ``keep: <reason>`` for a text that stays by design (never counted as work left: a step's
"allow-ui has no entries left for these files" means no entries without ``keep``), or ``todo: <reason>`` for a
text accepted one by one with ``--allow``.

A check fails when a (file, text) occurs more often than its entry allows (new text: convert it, or ``--allow`` it
after review) and when an entry allows more than the tree has (stale: the text was converted or removed; a stale
count would let a new copy in unseen, so ``--prune`` it in the same step). tests/i18n/test_ge0a.py runs the check.
Every write holds a lock on tests/i18n/ (parallel steps share these files) and re-reads the allowlist under it.
"""

from __future__ import annotations

import argparse
import collections
import contextlib
import fcntl
import hashlib
import json
import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import paths  # noqa: E402

ROOT = paths.ROOT
ALLOW = {"ui": "allow-ui.txt"}
# the file prefix that names the scan of a FILE:HASH (--allow)
PREFIX = {"ui": paths.UI_REL + "/"}
WHAT = {
    "ui": f"string literals in {paths.UI_REL} (not mock/, gallery.js) with a Dutch word outside makeT dictionaries, "
          "{nl, en} pairs, P() and X_NL/X_EN tables",
}
FIX = ("put the text in a makeT dictionary (UI) or, for the Rust host, a catalog row of i18n/host/<stem>.tsv "
       "(notes/claude/plan.md D-G8; H: notes/english-option-design.md §2.2). A text that is meant to stay (an inline "
       "pair by design, an input alias, a search keyword) is accepted one by one after review: "
       "tools/i18n/ratchet.py --allow FILE:HASH --keep --reason '…'")


def norm(text: str) -> str:
    return " ".join(str(text).split())


def digest(text: str) -> str:
    return hashlib.sha1(norm(text).encode("utf-8")).hexdigest()[:12]


@dataclass
class Entry:
    file: str
    hash: str
    count: int
    kind: str
    text: str
    status: str = ""                 # "" (to convert) | "keep: why" | "todo: why"

    @property
    def keep(self) -> bool:
        return self.status.startswith("keep")


def scan(name: str, root: Path = ROOT) -> list[dict]:
    if name == "ui":
        import corpus
        node = corpus.find_node()
        if not node:
            raise FileNotFoundError("node not found (set $NODE): the UI scan is tools/i18n/scan_ui.mjs")
        r = subprocess.run([node, str(HERE / "scan_ui.mjs"), str(root)], capture_output=True, text=True, timeout=300)
        if r.returncode != 0:
            raise RuntimeError(f"scan_ui.mjs failed ({r.returncode}): {r.stderr[-2000:]}")
        return json.loads(r.stdout)
    raise ValueError(name)


def entries(hits: list[dict]) -> dict[tuple[str, str], Entry]:
    out: dict[tuple[str, str], Entry] = {}
    for h in hits:
        k = (h["file"], digest(h["text"]))
        if k in out:
            out[k].count += 1
        else:
            out[k] = Entry(h["file"], k[1], 1, h["kind"], norm(h["text"]))
    return out


def read_allow(path: Path) -> dict[tuple[str, str], Entry]:
    out: dict[tuple[str, str], Entry] = {}
    if not path.exists():
        return out
    for line in path.read_text(encoding="utf-8").split("\n"):
        if not line or line.startswith("#"):
            continue
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        f, hx, c = parts[0], parts[1], parts[2]
        out[(f, hx)] = Entry(f, hx, int(c), parts[3] if len(parts) > 3 else "", parts[4] if len(parts) > 4 else "",
                             parts[5] if len(parts) > 5 else "")
    return out


def write_allow(path: Path, name: str, ents: dict[tuple[str, str], Entry]) -> None:
    files = collections.Counter()
    keep = 0
    for e in ents.values():
        files[e.file] += e.count
        keep += e.count if e.keep else 0
    head = [
        f"# i18n ratchet allowlist '{name}' (H: notes/english-option-design.md §3.2; G: notes/claude/round0/ge0a.md), "
        "generated by tools/i18n/ratchet.py.",
        f"# Scan: {WHAT[name]}.",
        "# Columns: file, sha1(text)[:12], count, kind, text (shortened), status (empty: still to convert; 'keep: why':",
        "# stays by design; 'todo: why': accepted one by one). A step that converts a text prunes its entry",
        "# (ratchet.py --prune); a new Dutch-only text fails tests/i18n/test_ge0a.py instead of landing here.",
        f"# {sum(files.values())} occurrences in {len(files)} files ({keep} keep).",
    ]
    body = []
    for e in sorted(ents.values(), key=lambda e: (e.file, e.text, e.hash)):
        text = e.text.replace("\t", " ")
        if len(text) > 110:
            text = text[:109] + "…"
        cols = [e.file, e.hash, str(e.count), e.kind, text] + ([e.status.replace("\t", " ")] if e.status else [])
        body.append("\t".join(cols))
    tmp = path.with_name(f".{path.name}.tmp")
    tmp.write_text("\n".join(head + body) + "\n", encoding="utf-8")
    os.replace(tmp, path)


@contextlib.contextmanager
def locked(directory: Path):
    """An exclusive lock on the allowlist directory for one read-modify-write (parallel steps share the files)."""
    fd = os.open(str(directory), os.O_RDONLY)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally:
        os.close(fd)


@dataclass
class Result:
    name: str
    new: list[dict]             # hits beyond the allowlist
    stale: list[Entry]          # allowlist entries with fewer (or no) hits now
    current: dict
    allowed: dict

    @property
    def ok(self) -> bool:
        return not self.new and not self.stale


def check(name: str, root: Path = ROOT, allow_dir: Path | None = None, hits: list[dict] | None = None) -> Result:
    hits = scan(name, root) if hits is None else hits
    cur = entries(hits)
    allow = read_allow((allow_dir or paths.tests_dir(root)) / ALLOW[name])
    new = []
    by_key = collections.defaultdict(list)
    for h in hits:
        by_key[(h["file"], digest(h["text"]))].append(h)
    for k, e in cur.items():
        over = e.count - (allow[k].count if k in allow else 0)
        if over > 0:
            new.extend(sorted(by_key[k], key=lambda h: h["line"])[-over:])
    stale = []
    for k, a in allow.items():
        have = cur[k].count if k in cur else 0
        if have < a.count:
            stale.append(Entry(a.file, a.hash, a.count - have, a.kind, a.text, a.status))
    return Result(name, sorted(new, key=lambda h: (h["file"], h["line"])), stale, cur, allow)


def report(r: Result) -> str:
    lines = []
    if r.new:
        lines.append(f"{r.name}: {len(r.new)} new text(s) without a key — {FIX}:")
        for h in r.new:
            lines.append(f"  {h['file']}:{h['line']} [{h['kind']}] {h['file']}:{digest(h['text'])} "
                         f"{norm(h['text'])[:140]}")
    if r.stale:
        lines.append(f"{r.name}: {sum(e.count for e in r.stale)} allowlist occurrence(s) no longer in the tree: a "
                     f"converted text leaves its entry behind; drop it in the same step "
                     f"(tools/i18n/ratchet.py --prune {r.name} [--file F]):")
        for e in r.stale[:20]:
            lines.append(f"  {e.file} {e.hash} ×{e.count} {e.text[:100]}")
        if len(r.stale) > 20:
            lines.append(f"  … and {len(r.stale) - 20} more")
    if not lines:
        todo = sum(e.count for e in r.allowed.values() if not e.keep)
        lines.append(f"{r.name}: ok ({sum(e.count for e in r.current.values())} allowed occurrences, {todo} still to "
                     "convert)")
    return "\n".join(lines)


def summary(name: str, ents: dict) -> str:
    files = collections.Counter()
    keeps = collections.Counter()
    kinds = collections.Counter()
    for e in ents.values():
        files[e.file] += e.count
        kinds[e.kind] += e.count
        if e.keep:
            keeps[e.file] += e.count
    out = [f"{name}: {sum(files.values())} occurrences in {len(files)} files, {sum(keeps.values())} of them keep "
           f"({', '.join(f'{k} {v}' for k, v in kinds.most_common())})"]
    out += [f"  {n - keeps[f]:5d} to convert {keeps[f]:4d} keep  {f}" for f, n in files.most_common()]
    return "\n".join(out)


def prune(r: Result, files: set[str] | None = None) -> dict[tuple[str, str], Entry]:
    keep = {}
    for k, e in r.allowed.items():
        have = r.current[k].count if k in r.current else 0
        if files and e.file not in files:
            keep[k] = e
        elif have:
            keep[k] = Entry(e.file, e.hash, min(e.count, have), e.kind, e.text, e.status)
    return keep


def regenerate(r: Result) -> tuple[dict[tuple[str, str], Entry], list[Entry]]:
    """The allowlist the tree gives now, with the reasons of the entries that are still there; and what it adds."""
    out, added = {}, []
    for k, e in r.current.items():
        old = r.allowed.get(k)
        out[k] = Entry(e.file, e.hash, e.count, e.kind, e.text, old.status if old else "")
        if old is None or e.count > old.count:
            added.append(Entry(e.file, e.hash, e.count - (old.count if old else 0), e.kind, e.text))
    return out, added


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("scans", nargs="*", help=f"{', '.join(ALLOW)} (default: all)")
    ap.add_argument("--write", action="store_true")
    ap.add_argument("--yes", action="store_true", help="with --write: really write")
    ap.add_argument("--prune", action="store_true")
    ap.add_argument("--file", action="append", default=[], help="with --prune: only this file's entries")
    ap.add_argument("--allow", metavar="FILE:HASH", help="accept one text (FILE:HASH from the check's report)")
    ap.add_argument("--keep", action="store_true", help="with --allow: the text stays by design")
    ap.add_argument("--reason", help="with --allow: why (required)")
    ap.add_argument("--summary", action="store_true")
    ap.add_argument("--root", default=str(ROOT))
    a = ap.parse_args(argv)
    root = Path(a.root)
    allow_dir = paths.tests_dir(root)
    if a.allow:
        if not a.reason or not a.reason.strip():
            ap.error("--allow needs --reason (why this text may stay without a key)")
        f, _, hx = a.allow.rpartition(":")
        name = next((n for n, p in PREFIX.items() if f.startswith(p)), None)
        if name is None:
            ap.error(f"--allow: {f} belongs to no scan (file prefixes: {', '.join(PREFIX.values())})")
        with locked(allow_dir):
            r = check(name, root)
            k = (f, hx)
            if k not in r.current:
                print(f"{name}: no text {hx} in {f} in the tree (see the check's report for FILE:HASH)")
                return 1
            ents = dict(r.allowed)
            e = r.current[k]
            status = f"{'keep' if a.keep else 'todo'}: {' '.join(a.reason.split())}"
            ents[k] = Entry(e.file, e.hash, e.count, e.kind, e.text, status)
            write_allow(allow_dir / ALLOW[name], name, ents)
        print(f"{name}: allowed {f} {hx} ×{e.count} ({status}): {e.text[:100]}")
        return 0
    names = a.scans or list(ALLOW)
    bad = [n for n in names if n not in ALLOW]
    if bad:
        ap.error(f"unknown scan(s): {', '.join(bad)} ({', '.join(ALLOW)})")
    rc = 0
    for n in names:
        try:
            hits = scan(n, root)
        except FileNotFoundError as e:
            print(f"{n}: skipped ({e})")
            continue
        path = allow_dir / ALLOW[n]
        if a.write:
            with locked(allow_dir):
                r = check(n, root, hits=hits)
                ents, added = regenerate(r)
                for e in sorted(added, key=lambda e: (e.file, e.text)):
                    print(f"  + {e.file} {e.hash} ×{e.count} [{e.kind}] {e.text[:110]}")
                kept = sum(e.count for e in prune(r).values())
                print(f"{n}: --write would add {sum(e.count for e in added)} occurrence(s) and drop "
                      f"{sum(e.count for e in r.allowed.values()) - kept} stale one(s)")
                if not a.yes:
                    print(f"{n}: nothing written (review the list, then --write --yes)")
                    continue
                write_allow(path, n, ents)
            print(f"wrote {path}")
            print(summary(n, ents))
            continue
        if a.prune:
            with locked(allow_dir):
                r = check(n, root, hits=hits)
                keep = prune(r, set(a.file) or None)
                write_allow(path, n, keep)
            print(f"pruned {path}: {sum(e.count for e in r.allowed.values()) - sum(e.count for e in keep.values())} "
                  "occurrence(s) dropped")
            continue
        r = check(n, root, hits=hits)
        if a.summary:
            print(summary(n, r.allowed))
            continue
        print(report(r))
        if not r.ok:
            rc = 1
    return rc


if __name__ == "__main__":
    sys.exit(main())
