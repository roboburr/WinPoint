"""Where the language data of Win11 voor GNOME lives (GE0a, notes/claude/round0/ge0a.md).

H (the frozen Hyprland project these tools come from) had its UI in ``ui/js`` and a Python host; G has a Rust host in
``host/src`` with its catalogues in ``host/i18n/*.tsv`` (decision D-G8) and the native GTK 4 programs in ``native/``
(the shell, Calendar, the kit and the seven apps; their texts are inline nl/en pairs, check_native.py). The
vendored HTML UI went with the WebKit apps (commit ``6ebf169f``). Every tool imports its paths from here, so a move
is one edit.
"""

from __future__ import annotations

from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent

NATIVE_REL = "native"                   # H: ui/js; G up to 6ebf169f: the vendored UI
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


def native(root: Path | str = ROOT) -> Path:
    return Path(root).joinpath(*NATIVE_REL.split("/"))


def tests_dir(root: Path | str = ROOT) -> Path:
    return Path(root).joinpath(*TESTS_REL.split("/"))
