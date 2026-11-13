#!/bin/bash
# Screenshots of the Windows Update page in every mock state, nl and en (GW3, notes/claude/wu/gw3.md).
#
#   tools/heavy.sh --tag gw3-shots -- tests/update-ui/shoot.sh            the full set (below)
#   tools/heavy.sh --tag gw3-shots -- tests/update-ui/shoot.sh ID,ID …    only these surface ids, nl + en, light
#     (env LANGS="en", THEME=dark, SIZE=800x1400, SUFFIX=-narrow: other languages, theme, size, shots/<lang><suffix>/)
#
# Each shot is one headless Chrome run (--screenshot after --virtual-time-budget) of tests/update-ui/shoot.html, which
# shows one tests/i18n/surfaces.json surface in mock mode (?mock=1&mockStatic=1, tests/i18n/freeze.js's fixed clock)
# and runs the surface's "do" actions. The whole run (server and every Chrome) is inside tests/update-ui/nonet.sh's
# bubblewrap layer: read-only system, empty /run/dbus and /run/user, a network namespace with loopback only (review
# B1: Chrome opened the real system-bus socket and probed internet addresses on its own). In there Chrome runs
# through tests/i18n/private.sh (env -i, private HOME and XDG dirs inside tests/update-ui/.work/, its own D-Bus
# session without service activation, no display) with DBUS_SYSTEM_BUS_ADDRESS pointing nowhere, and resolves no
# host but 127.0.0.1; the page is served read-only by tests/update-ui/serve.py (tests/i18n/serve.py + this
# directory). Must run under tools/heavy.sh (it takes no lock of its own and refuses without one).
#
# Full set: settings.update, settings.update-history, settings.update-advanced and every settings-update.* surface
# at 1280×1400 in nl and en (shots/nl, shots/en), the long-text states again in en at 800×1400 (shots/en-narrow), and
# five states in dark mode (shots/nl-dark).
set -euo pipefail
U=$(cd "$(dirname "$0")" && pwd -P); R=$(cd "$U/../.." && pwd -P)
[[ -n "${WIN11_HEAVY_HELD:-}" ]] && ! flock -n "$WIN11_HEAVY_HELD" true 2>/dev/null \
  || { echo "shoot.sh: run me under tools/heavy.sh (no heavy lock held)" >&2; exit 2; }
# outside the bubblewrap layer: start again inside it (nonet.sh checks the layer before it runs this script)
[[ ${W11_NONET:-} == 1 ]] || exec "$U/nonet.sh" "$U/shoot.sh" "$@"
[[ -z "$(ls -A /run/dbus)" && $(awk -F: 'NR > 2 { gsub(/ /, "", $1); print $1 }' /proc/net/dev | tr '\n' ' ') == "lo " ]] \
  || { echo "shoot.sh: W11_NONET=1 but this is not nonet.sh's layer (/run/dbus or a network interface is there)" >&2; exit 70; }
CHROME="${CHROME:-$(command -v google-chrome || true)}"
[[ -n $CHROME && -x $CHROME ]] || { echo "shoot.sh: no google-chrome found (set CHROME=/path)" >&2; exit 3; }
CHROME=$(readlink -f "$CHROME")
BUDGET="${BUDGET:-3600000}"
mkdir -p "$U/.work"
W=$(mktemp -d "$U/.work/shoot.XXXXXX")
SRV="" CPID=""
cleanup() {
  [[ -n $CPID ]] && kill -- "-$CPID" 2>/dev/null || true
  [[ -n $SRV ]] && kill "$SRV" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$W"
}
trap cleanup EXIT
trap 'exit 130' INT TERM

ids=$(python3 -I -c 'import json,sys; print("\n".join(s["id"] for s in json.load(open(sys.argv[1]))["surfaces"]
  if s["id"] in ("settings.update", "settings.update-history", "settings.update-advanced")
  or s["id"].startswith("settings-update.")))' "$R/tests/i18n/surfaces.json")
PLAN=()   # "id lang theme WxH outfile"
unknown=0
if (( $# )); then
  IFS=, read -r -a only <<<"$1"
  # every id must be a surface of tests/i18n/surfaces.json: shoot.html would show an empty page for any other
  # id and the shot would "succeed" (round 2 "Round 3 picks up"); an unknown id counts as a failed shot
  known=$(python3 -I -c 'import json,sys; print("\n".join(s["id"] for s in json.load(open(sys.argv[1]))["surfaces"]))' \
    "$R/tests/i18n/surfaces.json")
  for id in "${only[@]}"; do
    if ! grep -qxF -- "$id" <<<"$known"; then
      echo "shoot.sh: FAILED unknown surface id '$id' (not in tests/i18n/surfaces.json)" >&2
      unknown=$((unknown + 1))
      continue
    fi
    for l in ${LANGS:-nl en}; do
    PLAN+=("$id $l ${THEME:-light} ${SIZE:-1280x1400} $l${SUFFIX:-}/$id.png"); done; done
else
  for l in nl en; do for id in $ids; do PLAN+=("$id $l light 1280x1400 $l/$id.png"); done; done
  for st in H03 H05 H07 H09 H10c H11 H12 H19 H20 H20b H26 H27 R01 R03 R06; do PLAN+=("settings-update.$st en light 800x1400 en-narrow/settings-update.$st.png"); done
  for st in H01 H05 H10 H12 R03; do PLAN+=("settings-update.$st nl dark 1280x1400 nl-dark/settings-update.$st.png"); done
fi

env -i PATH=/usr/bin:/bin python3 -I "$U/serve.py" "$R" >"$W/port" 2>"$W/serve.err" &
SRV=$!
for _ in $(seq 1 50); do [[ -s $W/port ]] && break; sleep 0.1; done
PORT=$(head -n 1 "$W/port")
[[ -n $PORT ]] || { echo "shoot.sh: server did not start: $(cat "$W/serve.err")" >&2; exit 3; }

n=0 bad=$unknown
for p in "${PLAN[@]}"; do
  read -r id l th size out <<<"$p"
  awk '/^MemAvailable:/ { exit !($2 >= 2*1024*1024) }' /proc/meminfo || { echo "shoot.sh: MemAvailable under 2 GB, stopping" >&2; exit 75; }
  mkdir -p "$U/shots/$(dirname "$out")"
  rm -f "$U/shots/$out"
  rm -rf "$W/s"; mkdir -p "$W/s"
  setsid "$R/tests/i18n/private.sh" "$W/s" /bin/bash -c '
      cd "$TMPDIR/.." || exit 70
      # private.sh starts from env -i: no system bus for libdbus to fall back to (the layer has no /run/dbus either)
      export DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent/system_bus_socket
      TMPDIR=tmp exec "$@"' chrome "$CHROME" --headless=new --disable-gpu \
      --hide-scrollbars --no-first-run --no-default-browser-check --disable-extensions --disable-sync \
      --disable-background-networking --disable-component-update --disable-breakpad --disable-domain-reliability \
      --no-pings --mute-audio --lang=en-US --host-resolver-rules="MAP * ~NOTFOUND , EXCLUDE 127.0.0.1" \
      --user-data-dir="$W/s/profile" --disk-cache-dir="$W/s/profile/cache" --window-size="${size/x/,}" \
      --virtual-time-budget="$BUDGET" --screenshot="$U/shots/$out" \
      "http://127.0.0.1:$PORT/tests/update-ui/shoot.html?id=$id&lang=$l&theme=$th" >"$W/chrome.out" 2>"$W/chrome.err" &
  CPID=$!
  rc=0; wait "$CPID" || rc=$?
  CPID=""
  n=$((n + 1))
  if [[ $rc != 0 || ! -s $U/shots/$out ]]; then bad=$((bad + 1)); echo "shoot.sh: FAILED $out (rc $rc): $(tail -2 "$W/chrome.err")" >&2
  else echo "shot $out"; fi
done
echo "shoot.sh: $n shots, $bad failed$( ((unknown)) && echo " ($unknown unknown surface ids)") (tests/update-ui/shots/)"
(( bad == 0 ))
