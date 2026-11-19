#!/usr/bin/env bash
# User-local installation. On a terminal it adds WinPoint's own choice at the login screen and the system files
# (scripts/install-session.py --system; pkexec asks the password once) and makes WinPoint the session your next
# sign-in starts; your normal Ubuntu session stays as it was. -h/--help lists the options.
# /usr/bin/python3 -E -s -B, as WinPoint's updater runs install.py: Ubuntu's own Python, without PYTHON* variables,
# user site-packages or bytecode files in this folder.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec /usr/bin/python3 -E -s -B "$ROOT/scripts/install.py" "$ROOT" "$@"
