#!/usr/bin/env python3
"""Verify the shipped UI snapshot, retaining the original baseline separately.

Since the WebKit removal the snapshot is the assets the native shell and apps fetch over /ui/assets/ (wallpapers,
the Settings icon); a path that vendor/ui-patches.json names in "removed_paths" (html_apps_removed: index.html, js/,
css/, assets/img/) may not come back, in the manifest or in the folder.
Messages in Dutch, or English with WIN11_LANG=en (a developer script; it reads no user settings)."""
import hashlib
import json
import os
from pathlib import Path
import sys

# i18n: pairs (tools/i18n/check_pairs.py).
T = {
    "ok": {"nl": "OK: {count} interfacebestanden komen overeen met het distributiemanifest.", "en": "OK: {count} interface files match the distribution manifest."},
    "baseline": {"nl": "Oorspronkelijke baseline: {files} bestanden; bewuste wijzigingen: {changed}, toegevoegd: {added}. Zie vendor/ui-patches.json.",
                 "en": "Original baseline: {files} files; intended changes: {changed}, added: {added}. See vendor/ui-patches.json."},
}
LANGUAGE = "en" if os.environ.get("WIN11_LANG", "").strip().lower() == "en" else "nl"


def t(key, **params):
    return T[key][LANGUAGE].format_map(params)


root = Path(__file__).resolve().parent.parent
manifest = json.loads((root/"vendor/ui-sha256.json").read_text())
ui = Path(sys.argv[1]) if len(sys.argv) > 1 else root/"vendor/ui"
actual = {str(path.relative_to(ui)): hashlib.sha256(path.read_bytes()).hexdigest()
          for path in ui.rglob("*") if path.is_file()}
missing = sorted(manifest.keys() - actual.keys())
extra = sorted(actual.keys() - manifest.keys())
changed = sorted(name for name in manifest.keys() & actual.keys() if manifest[name] != actual[name])
patches = root/"vendor/ui-patches.json"
removed_paths = [path for entry in (json.loads(patches.read_text()).values() if patches.is_file() else [])
                 if isinstance(entry, dict) for path in entry.get("removed_paths", [])]
back = sorted(name for name in manifest.keys() | actual.keys()
              if any(name == path or (path.endswith("/") and name.startswith(path)) for path in removed_paths))
if missing or extra or changed or back:
    print(json.dumps({"missing": missing, "extra": extra, "changed": changed, "removed_paths": back}, indent=2))
    raise SystemExit(1)
print(t("ok", count=len(manifest)))
original = root/"vendor/ui-original-sha256.json"
if original.is_file():
    baseline = json.loads(original.read_text())
    changed = sorted(name for name in baseline.keys() & actual.keys() if baseline[name] != actual[name])
    added = sorted(actual.keys() - baseline.keys())
    print(t("baseline", files=len(baseline), changed=len(changed), added=len(added)))
