#!/usr/bin/env python3
"""Regenerate the vendored UI's manifests (GE1a, notes/claude/round1/ge1a.md; plan-critique F5, D-G17).

Since the WebKit removal ``vendor/ui`` holds only the assets the native GTK 4 shell and apps fetch over /ui/assets/
(the wallpapers and the Settings icon); the HTML apps went (``html_apps_removed`` in vendor/ui-patches.json).

    python3 -B tools/ui-manifest.py            write vendor/ui-sha256.json and the file lists of vendor/ui-patches.json
    python3 -B tools/ui-manifest.py --check    write nothing; exit 1 when either file is out of date (lists the drift)

* ``vendor/ui-sha256.json``: ``{relative path: sha256}`` of every file under ``vendor/ui``, found by the same walk as
  ``scripts/verify-ui.py`` (``Path.rglob`` + ``is_file``), sorted, two-space indent, one trailing newline: the format
  ``scripts/verify-ui.py`` and ``scripts/package.py`` read.
* ``vendor/ui-patches.json``: only its ``changed``, ``added`` and ``removed`` lists are recomputed, against the
  original baseline ``vendor/ui-original-sha256.json`` (Codex's 161 files of the 2026-10-27 interim UI). Every other
  entry (the reasons, ``lotus_wallpapers`` …) is kept as written, in its order. Entries of the form
  ``{"files": [...], "reason": ...}`` are checked: each named file must exist under ``vendor/ui``. Entries with
  ``"removed_paths"`` (``html_apps_removed``: ``index.html``, ``js/``, ``css/``, ``assets/img/``) are checked the
  other way: no file may be under a removed path (one ending in ``/`` is a folder), so the HTML cannot come back
  unnoticed; scripts/verify-ui.py refuses it too.

The tool never edits ``vendor/ui`` itself and never touches ``scripts/`` (GE1c owns ``scripts/verify-ui.py``).
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
UI = ROOT / "vendor" / "ui"
MANIFEST = ROOT / "vendor" / "ui-sha256.json"
PATCHES = ROOT / "vendor" / "ui-patches.json"
ORIGINAL = ROOT / "vendor" / "ui-original-sha256.json"


def hashes(ui: Path) -> dict[str, str]:
    return {p.relative_to(ui).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(ui.rglob("*")) if p.is_file()}


def manifest_text(h: dict[str, str]) -> str:
    return json.dumps(dict(sorted(h.items())), indent=2) + "\n"


def patches_doc(h: dict[str, str], doc: dict, original: dict[str, str]) -> dict:
    out = dict(doc)
    out["baseline_files"] = len(original)
    out["changed"] = sorted(n for n in original.keys() & h.keys() if original[n] != h[n])
    out["added"] = sorted(h.keys() - original.keys())
    out["removed"] = sorted(original.keys() - h.keys())
    return out


def patches_text(doc: dict) -> str:
    return json.dumps(doc, indent=2, ensure_ascii=False) + "\n"


def under(name: str, paths: list[str]) -> bool:
    """name is one of paths, or below one that ends in "/"."""
    return any(name == p or (p.endswith("/") and name.startswith(p)) for p in paths)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--check", action="store_true", help="write nothing; exit 1 when a file is out of date")
    a = ap.parse_args(argv)
    h = hashes(UI)
    original = json.loads(ORIGINAL.read_text(encoding="utf-8"))
    old_manifest = MANIFEST.read_text(encoding="utf-8") if MANIFEST.exists() else ""
    old_patches_text = PATCHES.read_text(encoding="utf-8") if PATCHES.exists() else "{}"
    old_patches = json.loads(old_patches_text)
    new_manifest = manifest_text(h)
    new_doc = patches_doc(h, old_patches, original)
    new_patches = patches_text(new_doc)

    problems = []
    for key, entry in new_doc.items():
        if isinstance(entry, dict) and isinstance(entry.get("files"), list):
            for f in entry["files"]:
                if f not in h:
                    problems.append(f"ui-patches.json {key}: file {f!r} is not under vendor/ui")
        if isinstance(entry, dict) and isinstance(entry.get("removed_paths"), list):
            for f in sorted(n for n in h if under(n, entry["removed_paths"])):
                problems.append(f"ui-patches.json {key}: {f!r} is under a removed path {entry['removed_paths']}")

    old_h = json.loads(old_manifest) if old_manifest else {}
    drift = {
        "changed": sorted(n for n in old_h.keys() & h.keys() if old_h[n] != h[n]),
        "extra": sorted(h.keys() - old_h.keys()),
        "missing": sorted(old_h.keys() - h.keys()),
    }
    lists = {k: (old_patches.get(k), new_doc[k]) for k in ("changed", "added", "removed")}
    stale = old_manifest != new_manifest or old_patches_text != new_patches
    for p in problems:
        print(p, file=sys.stderr)
    summary = (f"vendor/ui: {len(h)} files; against the original {len(original)}: {len(new_doc['changed'])} changed, "
               f"{len(new_doc['added'])} added, {len(new_doc['removed'])} removed")
    if a.check:
        if stale:
            print(json.dumps({"manifest_drift": drift,
                              "patches_lists": {k: {"now": v[1], "recorded": v[0]} for k, v in lists.items()
                                                if v[0] != v[1]}}, indent=2))
            print(f"ui-manifest: out of date ({summary}); run python3 -B tools/ui-manifest.py", file=sys.stderr)
            return 1
        print(f"ui-manifest: up to date ({summary})")
        return 1 if problems else 0
    if old_manifest != new_manifest:
        MANIFEST.write_text(new_manifest, encoding="utf-8")
    if old_patches_text != new_patches:
        PATCHES.write_text(new_patches, encoding="utf-8")
    print(f"ui-manifest: wrote {MANIFEST.relative_to(ROOT)} ({len(drift['changed'])} changed, {len(drift['extra'])} new, "
          f"{len(drift['missing'])} gone since the last manifest) and {PATCHES.relative_to(ROOT)} lists ({summary})")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
