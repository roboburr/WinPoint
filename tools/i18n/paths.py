"""Where the language data of Win11 voor GNOME lives (GE0a, notes/claude/round0/ge0a.md).

H (the frozen Hyprland project these tools come from) had its UI in ``ui/js`` and a Python host; G has the vendored
UI in ``vendor/ui/js``, a Rust host in ``host/src`` and future TSV catalogs in ``i18n/**/*.tsv`` (decision D-G8).
Every tool imports its paths from here, so a move is one edit.
"""

from __future__ import annotations

from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

UI_REL = "vendor/ui/js"                 # H: ui/js
TESTS_REL = "tests/i18n"

# The Rust host's embedded {nl, en} string tables (H had them as Python _STR dicts). GE1b moves them into
# i18n/host/<stem>.tsv; until then check.py checks their parity and corpus.py reads them.
# file → JSON path of the {"nl": {...}, "en": {...}} table inside it
HOST_JSON_TABLES = {
    "host/src/services/sysmon/text.json": ("_STR",),
    "host/src/services/disks/data.json": ("_STR",),
    "host/src/services/storage/data.json": ("_STR",),
    "host/src/services/system/strings.json": ("system",),
}


def ui_js(root: Path | str = ROOT) -> Path:
    return Path(root).joinpath(*UI_REL.split("/"))


def tests_dir(root: Path | str = ROOT) -> Path:
    return Path(root).joinpath(*TESTS_REL.split("/"))
