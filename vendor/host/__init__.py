"""Win11 Shell host: Python backend (stdlib + PyGObject only).

See SPEC.md for the contract. Entry points:
  python3 -m host --mode layer|preview|serve|app ...
  python3 -m host.appwindow --app <id> --port P
  python3 -m host.ctl <command>
"""

from pathlib import Path

VERSION = "0.1.0"

#: Our own apps (each runs in its own process/window with app_id win11shell-<id>).
APP_IDS = ("settings", "taskmgr", "explorer", "alles", "disks")

#: Project root (contains ui/, host/, bin/ ...).
ROOT = Path(__file__).resolve().parent.parent

__all__ = ["VERSION", "APP_IDS", "ROOT"]
