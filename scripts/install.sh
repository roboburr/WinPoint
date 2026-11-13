#!/usr/bin/env bash
# User-local installation only; activation and desktop settings are left to the user.
# Arguments go to install.py: [--lang nl|en] (Win11's language on a first install), -h/--help.
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
exec python3 "$ROOT/scripts/install.py" "$ROOT" "$@"
