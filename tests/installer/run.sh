#!/usr/bin/env bash
# GE1c installer and extension-text tests. Every install runs in tests/installer/sandbox.py's bubblewrap case dirs;
# the whole suite runs inside tests/i18n/private.sh (no live session bus, display or HOME), and, when the two REAL
# cases are included (the default), under tools/heavy.sh (they copy the real 81 MB icon tree and 228 MB of
# binaries under strace).
#   tests/installer/run.sh                       all tests
#   W11_INSTALLER_REAL=0 tests/installer/run.sh  without the REAL cases, not under the heavy lock
#   tests/installer/run.sh Units                 one class (unittest names, as for python -m unittest)
set -euo pipefail
G=$(cd "$(dirname "$0")/../.." && pwd -P)
mkdir -p "$G/tests/installer/run"
REAL=${W11_INSTALLER_REAL:-1}
NODE=${NODE:-/home/robin/.local/node/bin/node}
inner=(env W11_INSTALLER_REAL="$REAL" NODE="$NODE" /usr/bin/python3 -B "$G/tests/installer/tests.py" "$@")
wrapped=("$G/tests/i18n/private.sh" "$G/tests/installer/run/session" "${inner[@]}")
if [ "$REAL" = 0 ]; then
  exec "${wrapped[@]}"
fi
exec "$G/tools/heavy.sh" --min-mb 1024 --tag ge1c-installer -- "${wrapped[@]}"
