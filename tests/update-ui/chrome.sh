#!/bin/bash
# google-chrome with --no-sandbox, only inside tests/update-ui/nonet.sh's bubblewrap layer (GW3 fix round, review B1).
# nonet.sh sets CHROME to this script, so tests/i18n/run.sh and tests/update-ui/shoot.sh use it unchanged.
#
# Chrome's own sandbox cannot start in that layer: its user-namespace sandbox needs capabilities inside the new
# namespace, which Ubuntu's AppArmor profile for children of bwrap (unpriv_bwrap: "audit deny capability") refuses,
# and the setuid chrome-sandbox cannot elevate under bwrap's no_new_privs ("The SUID sandbox helper binary was found,
# but is not configured correctly", then abort). The layer takes its place for these runs: read-only system, no
# network but loopback, no system or session bus. The pages are this repository's own UI, served from 127.0.0.1.
# Outside the layer this script refuses (exit 70), so --no-sandbox never runs on the live system.
set -u
bad() { echo "chrome.sh: refusing --no-sandbox outside nonet.sh's layer: $*" >&2; exit 70; }
[[ -d /run/dbus && -z "$(ls -A /run/dbus)" ]] || bad "/run/dbus is not empty"
[[ -d /run/user && -z "$(ls -A /run/user)" ]] || bad "/run/user is not empty"
ifs=$(awk -F: 'NR > 2 { gsub(/ /, "", $1); print $1 }' /proc/net/dev | sort | tr '\n' ' ')
[[ $ifs == "lo " ]] || bad "network interfaces: $ifs"
[[ $(cat /proc/1/comm 2>/dev/null) == bwrap ]] || bad "PID 1 is not bwrap"
REAL=$(readlink -f "$(command -v google-chrome)")
[[ -x $REAL ]] || { echo "chrome.sh: no google-chrome found" >&2; exit 3; }
exec "$REAL" --no-sandbox "$@"
