#!/usr/bin/env python3
"""Write native/shell/src/start/settings_pages.json (the Settings pages Start's search offers) from the Settings
app's registry, native/apps/settings/src/registry.rs: every page except Settings' own Home page and the data pages
(registry.rs DATA_PAGES), in the registry's order, with its parent, icon, titles, descriptions and keywords
(notes/claude/settings-parity.md §3: only batch B0 and the integration step run it with --write).

    python3 -B tools/settings-pages.py            exit 1 when the JSON differs from the registry (prints why)
    python3 -B tools/settings-pages.py --write    rewrite the JSON

tests/i18n/test_native.py checks the same (SettingsPages); this tool reads the registry with the same lexer.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests" / "i18n"))
sys.path.insert(0, str(ROOT / "tools" / "i18n"))

import test_native  # noqa: E402


def wanted() -> list[dict]:
    return [p for p in test_native.registry_pages(ROOT) if p["id"] not in test_native.NOT_IN_START]


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--write", action="store_true", help="rewrite settings_pages.json")
    args = ap.parse_args()
    path = ROOT / test_native.START_PAGES
    pages = wanted()
    text = json.dumps(pages, ensure_ascii=False, separators=(",", ":"))
    if args.write:
        path.write_text(text, encoding="utf-8")
        print(f"settings-pages: wrote {len(pages)} pages to {test_native.START_PAGES}")
        return 0
    current = json.loads(path.read_text(encoding="utf-8"))
    diff = test_native.differences(test_native.registry_pages(ROOT), current)
    for line in diff:
        print(f"settings-pages: {line}")
    return 1 if diff else 0


if __name__ == "__main__":
    sys.exit(main())
