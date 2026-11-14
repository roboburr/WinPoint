#!/usr/bin/env bash
# User-local installation; on a terminal it first offers WinPoint's own choice at the login screen
# (scripts/install-session.py, pkexec asks the password once).
# Arguments go to install.py: [--lang nl|en] (Win11's language on a first install), [--no-session], -h/--help.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/install.py" "$ROOT" "$@"
