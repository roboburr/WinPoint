#!/usr/bin/env bash
# Removes WinPoint (scripts/uninstall.py), the login choice and the system files with one password included; your
# settings stay. After an install, ~/.local/bin/winpoint-uninstall does the same without this folder.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec /usr/bin/python3 -E -s -B "$ROOT/scripts/uninstall.py" "$@"
