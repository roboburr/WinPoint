"""Read-only static server for the Windows Update mock screenshots (GW3, notes/claude/wu/gw3.md).

    python3 -I tests/update-ui/serve.py ROOT      prints the port on the first line, then serves until killed

tests/i18n/serve.py (the UI text dump's server: /ui/ = vendor/ui with tests/i18n/freeze.js first in index.html's
<head>, /tests/i18n/, 127.0.0.1 only, GET/HEAD only, no listings, no-store) plus one more read-only mount,
/tests/update-ui/ → ROOT/tests/update-ui/, for shoot.html. Nothing else is served and nothing on disk changes.
"""

from __future__ import annotations

import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "i18n"))
import serve  # noqa: E402  (tests/i18n/serve.py)

serve.MOUNTS = tuple(serve.MOUNTS) + (("/tests/update-ui/", ("tests", "update-ui")),)

if __name__ == "__main__":
    sys.exit(serve.main(sys.argv))
