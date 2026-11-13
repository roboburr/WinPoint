#!/bin/bash
# UI text dump of Win11 voor GNOME (GE0a, notes/claude/round0/ge0a.md; ported from H's tests/i18n/run.sh, E0b):
# every surface of tests/i18n/surfaces.json in mock mode, rendered by headless Chrome from vendor/ui, as text.
#
#   tests/i18n/run.sh                 nl dump → compare with tests/i18n/baseline-nl.txt (exit 1 on a difference)
#   tests/i18n/run.sh --lang en       en dump → compare with tests/i18n/baseline-en.txt (exit 1 on a difference)
#   tests/i18n/run.sh --record        dump → write baseline-<lang>.txt when the only differences are new surfaces
#                                     (blocks); a changed or removed block is printed and refused unless
#   tests/i18n/run.sh --record --accept ID,ID   names it (ids of surfaces.json, "taskmgr.*" for a prefix, "all"),
#                                     so a re-record never silently covers a Dutch change (BASELINE-CHANGES says why)
#   tests/i18n/run.sh --out FILE      write the dump to FILE instead (no compare)
#   tests/i18n/run.sh --only ID,ID    a subset (ids from surfaces.json, "settings.*" for a prefix); printed, never compared
#   tests/i18n/run.sh --twice         determinism check: two dumps must be identical (nothing recorded or compared)
#   tests/i18n/run.sh --timing        add "# timing:" lines (when each page got ready); debugging, never compared
#   tests/i18n/run.sh --notes FILE    also write the run's notes (JSON: frames without window.__i18n, console errors
#                                     and warnings per surface, e.g. "[mock] unknown method …"); never in the dump
#
# Serves vendor/ui (as /ui/) and tests/i18n read-only with tests/i18n/serve.py (127.0.0.1, a free port; it puts
# tests/i18n/freeze.js in front of the page: fixed clock Sun 25 Oct 2026 14:35 Europe/Amsterdam, seeded Math.random,
# logical timers) and runs dump.html in headless Chrome with --virtual-time-budget (timers in virtual time, so
# polling pages are deterministic) and a throwaway profile. The dump is engine-specific: its first line names the
# Chrome version, and a compare against a baseline of another version says so first. (The product renders in
# WebKitGTK; Chrome is H's reference engine for this text dump. Intl output can differ between the two.)
#
# Live-session safety (notes/claude/plan.md): the whole run is inside tests/update-ui/nonet.sh's layer (a bare call
# re-executes itself through tools/heavy.sh and nonet.sh), and Chrome runs only through tests/i18n/private.sh (env -i, private HOME
# and XDG dirs inside tests/i18n/.run/, its own D-Bus session on an abstract socket with no service activation,
# NO_AT_BRIDGE=1, GSETTINGS_BACKEND=keyfile, no DISPLAY/WAYLAND_DISPLAY), resolves no host name but 127.0.0.1
# (no network), and writes nothing to /tmp (its singleton socket goes to a relative TMPDIR inside the work dir).
# Memory: each Chrome run waits (polling every minute, up to an hour) while MemAvailable is under 2 GB and holds
# flock on $G/.heavy.lock only while Chrome runs. The run kills only its own server and Chrome when it ends.
set -euo pipefail
T=$(cd "$(dirname "$0")" && pwd -P); R=$(cd "$T/../.." && pwd -P)
LANG_UI=nl MODE=compare OUT="" ONLY="" EXTRA="" ACCEPT="" NOTES="" BUDGET="${BUDGET:-3600000}"
LOCK="${HEAVY_LOCK:-${WIN11_HEAVY_LOCK:-$R/.heavy.lock}}"
# Under tools/heavy.sh (WIN11_HEAVY_HELD names this lock and someone holds it) the lock is already ours.
LOCKCMD=(flock -w 3600 "$LOCK")
if [[ -n "${WIN11_HEAVY_HELD:-}" && "$(realpath -m "$WIN11_HEAVY_HELD")" == "$(realpath -m "$LOCK")" ]] \
   && ! flock -n "$LOCK" true 2>/dev/null; then
  LOCKCMD=()
fi
# Chrome runs only inside tests/update-ui/nonet.sh's bubblewrap layer: no system bus, no live session, no network but
# loopback, no host sockets (GW3 review B1). A bare call starts again under tools/heavy.sh, then inside the layer.
if [[ ${W11_NONET:-} != 1 ]]; then
  (( ${#LOCKCMD[@]} == 0 )) || exec "$R/tools/heavy.sh" --tag i18n-run -- "$0" "$@"
  exec "$R/tests/update-ui/nonet.sh" "$0" "$@"
fi
[[ -z "$(ls -A /run/dbus 2>/dev/null)" ]] \
  || { echo "run.sh: W11_NONET=1 but this is not nonet.sh's layer (/run/dbus is not empty)" >&2; exit 70; }
while (( $# )); do
  case "$1" in
    --lang) LANG_UI="$2"; shift 2 ;;
    --lang=*) LANG_UI="${1#*=}"; shift ;;
    --record) MODE=record; shift ;;
    --accept) ACCEPT="$2"; shift 2 ;;
    --accept=*) ACCEPT="${1#*=}"; shift ;;
    --twice) MODE=twice; shift ;;
    --out) OUT="$2"; MODE=out; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --timing) EXTRA+="&timing=1"; shift ;;
    --notes) NOTES="$2"; shift 2 ;;
    -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "run.sh: unknown option $1" >&2; exit 2 ;;
  esac
done
[[ $LANG_UI == nl || $LANG_UI == en ]] || { echo "run.sh: --lang nl|en" >&2; exit 2; }
CHROME="${CHROME:-$(command -v google-chrome || true)}"
[[ -n $CHROME && -x $CHROME ]] || { echo "run.sh: no google-chrome found (set CHROME=/path)" >&2; exit 3; }
CHROME=$(readlink -f "$CHROME")
[[ $MODE == record && ( -n $ONLY || -n $EXTRA ) ]] && { echo "run.sh: --record records every surface (no --only/--timing)" >&2; exit 2; }
[[ -n $ACCEPT && $MODE != record ]] && { echo "run.sh: --accept goes with --record" >&2; exit 2; }
BASE="$T/baseline-$LANG_UI.txt"

mem_ok() { awk '/^MemAvailable:/ { exit !($2 >= 2*1024*1024) }' /proc/meminfo; }
wait_mem() {
  local i
  for (( i = 0; i < 60; i++ )); do
    mem_ok && return 0
    echo "run.sh: MemAvailable under 2 GB, waiting a minute ($((i + 1))/60)" >&2
    sleep 60
  done
  echo "run.sh: MemAvailable stayed under 2 GB for an hour; not starting Chrome" >&2
  return 1
}

mkdir -p "$T/.run"
W=$(mktemp -d "$T/.run/run.XXXXXX")
SRV="" CPID=""
cleanup() {
  [[ -n $CPID ]] && kill -- "-$CPID" 2>/dev/null || true
  [[ -n $SRV ]] && kill "$SRV" 2>/dev/null || true
  wait 2>/dev/null || true
  rm -rf "$W"
  rmdir "$T/.run" 2>/dev/null || true
}
trap cleanup EXIT
trap 'exit 130' INT TERM
env -i PATH=/usr/bin:/bin python3 -I "$T/serve.py" "$R" >"$W/port" 2>"$W/serve.err" &
SRV=$!
for _ in $(seq 1 50); do [[ -s $W/port ]] && break; sleep 0.1; done
PORT=$(head -n 1 "$W/port")
[[ -n $PORT ]] || { echo "run.sh: server did not start: $(cat "$W/serve.err")" >&2; exit 3; }
VERSION=$("$CHROME" --version 2>/dev/null | head -n 1 | tr -s ' ' | sed 's/ *$//')

# chrome URL OUTFILE: one headless Chrome run in a fresh private session under the heavy lock
chrome() {
  local url="$1" out="$2" i rc
  for (( i = 0; i < 60; i++ )); do
    wait_mem
    rm -rf "$W/s"; mkdir -p "$W/s"
    # setsid: the whole tree (flock, dbus-run-session, Chrome) is one process group that cleanup can stop
    setsid "${LOCKCMD[@]}" bash -c '
      awk "/^MemAvailable:/ { exit !(\$2 >= 2*1024*1024) }" /proc/meminfo || exit 75
      exec "$@"' heavy "$T/private.sh" "$W/s" /bin/bash -c '
      cd "$TMPDIR/.." || exit 70
      # a relative TMPDIR keeps the singleton socket path short (sun_path) without using /tmp
      TMPDIR=tmp exec "$@"' chrome "$CHROME" --headless=new --disable-gpu \
        --hide-scrollbars --no-first-run --no-default-browser-check --disable-extensions --disable-sync \
        --disable-background-networking --disable-component-update --disable-breakpad --disable-domain-reliability \
        --no-pings --mute-audio --lang=en-US --host-resolver-rules="MAP * ~NOTFOUND , EXCLUDE 127.0.0.1" \
        --user-data-dir="$W/s/profile" --disk-cache-dir="$W/s/profile/cache" --window-size=1600,1000 \
        --virtual-time-budget="$BUDGET" --dump-dom "$url" >"$out" 2>"$W/chrome.err" &
    CPID=$!
    rc=0; wait "$CPID" || rc=$?
    CPID=""
    [[ $rc == 75 ]] && { echo "run.sh: memory dropped under 2 GB while waiting for the lock; trying again" >&2; sleep 60; continue; }
    [[ $rc == 70 ]] && { echo "run.sh: the private session refused to start: $(tail -3 "$W/chrome.err")" >&2; return 1; }
    return 0
  done
  return 1
}

dump() { # dump FILE — one full run of dump.html into FILE (header + blocks)
  local file="$1" q="lang=$LANG_UI"
  [[ -n $ONLY ]] && q+="&only=$ONLY"
  q+="$EXTRA"
  chrome "http://127.0.0.1:$PORT/tests/i18n/dump.html?$q" "$W/dom.html" || return 1
  python3 -I - "$W/dom.html" "$file" "$VERSION" "$LANG_UI" "$NOTES" <<'PY' || { tail -5 "$W/chrome.err" >&2; return 1; }
import html, json, re, sys
src, dst, version, lang, notes_file = sys.argv[1:]
dom = open(src, encoding="utf-8").read()
m = re.search(r'<pre id="out">(.*?)</pre>', dom, re.S)
text = html.unescape(m.group(1)) if m else ""
if not re.search(r"^== END \d+ surfaces ==$", text, re.M):
    sys.stderr.write("run.sh: the dump did not finish (raise BUDGET?): " + (text[-300:] or "no output") + "\n")
    sys.exit(1)
m = re.search(r'<pre id="notes">(.*?)</pre>', dom, re.S)
try:
    notes = json.loads(html.unescape(m.group(1))) if m else {}
except ValueError:
    notes = {}
if notes.get("noReport"):
    sys.stderr.write(f"run.sh: note: {len(notes['noReport'])} of {notes.get('surfaces')} frames without window.__i18n "
                     "(core/i18n.js has no ?i18ncheck=1 before GE1a)\n")
con = notes.get("console") or {}
if con:
    sys.stderr.write(f"run.sh: note: console errors/warnings on {len(con)} surface(s) "
                     f"({sum(len(v) for v in con.values())} messages; --notes FILE lists them):\n")
    for sid, msgs in list(con.items())[:8]:
        sys.stderr.write(f"  {sid}: {msgs[0][:160]}" + (f" (+{len(msgs) - 1})" if len(msgs) > 1 else "") + "\n")
if notes_file:
    with open(notes_file, "w", encoding="utf-8") as f:
        json.dump(notes, f, ensure_ascii=False, indent=1, sort_keys=True)
        f.write("\n")
n = len(re.findall(r"^== (?!END )", text, re.M))
with open(dst, "w", encoding="utf-8") as f:
    f.write(f"# UI text dump · lang={lang} · {version} · {n} surfaces · tests/i18n/run.sh\n{text}")
PY
}

case $MODE in
  out) dump "$OUT"; echo "run.sh: wrote $OUT ($(wc -l <"$OUT") lines)" >&2 ;;
  record)
    dump "$W/new.txt"
    # per surface block: a new block is fine, a changed or removed block must be named in --accept
    python3 -I "$T/blocks.py" "$BASE" "$W/new.txt" "$ACCEPT" || exit 1
    cp "$W/new.txt" "$BASE"; echo "recorded $BASE ($(wc -l <"$BASE") lines)" ;;
  twice)
    dump "$W/a.txt"; dump "$W/b.txt"
    if cmp -s "$W/a.txt" "$W/b.txt"; then echo "run.sh: two $LANG_UI dumps identical ($(wc -l <"$W/a.txt") lines, sha256 $(sha256sum <"$W/a.txt" | cut -c1-16))"
    else diff -u "$W/a.txt" "$W/b.txt" | head -60; echo "run.sh: the two dumps differ" >&2; exit 1; fi ;;
  compare)
    dump "$W/now.txt"
    if [[ -n $ONLY || -n $EXTRA ]]; then cat "$W/now.txt"; exit 0; fi
    [[ -e $BASE ]] || { echo "run.sh: no $BASE yet (run.sh --record --lang $LANG_UI)" >&2; exit 1; }
    if [[ "$(head -n 1 "$BASE" | cut -d'·' -f3)" != "$(head -n 1 "$W/now.txt" | cut -d'·' -f3)" ]]; then
      echo "run.sh: the baseline was recorded with another engine:$(head -n 1 "$BASE" | cut -d'·' -f3)vs$(head -n 1 "$W/now.txt" | cut -d'·' -f3)" >&2
      echo "run.sh: re-record only when the diff touches nothing but Intl output (dates, numbers, lists)" >&2
    fi
    if cmp -s "$BASE" "$W/now.txt"; then echo "run.sh: $LANG_UI dump equals $(basename "$BASE") ($(wc -l <"$BASE") lines)"
    else diff -u "$BASE" "$W/now.txt" | head -200; echo "run.sh: the $LANG_UI dump differs from $(basename "$BASE")" >&2; exit 1; fi ;;
esac
