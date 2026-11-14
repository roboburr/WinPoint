#!/usr/bin/env bash
# Run one heavy command (cargo, headless Chrome, private gnome-shell) safely on Robin's machine.
#
#   tools/heavy.sh [--min-mb N] [--tag NAME] -- command [args...]
#
# 1. Takes the machine-wide lock (one heavy thing at a time, also across git worktrees).
# 2. Inside the lock: checks MemAvailable >= --min-mb (default 2048) and >= 5 GB free disk.
# 3. Runs the command in its own process group, niced, under /usr/bin/time -v.
# 4. Watchdog: if MemAvailable drops below 1500 MB, kills only that process group (earlyoom here
#    never kills win11-*/WebKit processes and would pick one of Robin's apps instead).
# Exit codes: the command's own; 75 = lock timeout or not enough memory/disk; 137 = killed by the watchdog.
# The command gets WIN11_HEAVY_LOCK=WIN11_HEAVY_HELD=<lock path>. A nested heavy.sh, tests/rust-sandbox/run-tests.sh
# and tests/gnome/harness.py then skip their own flock (it would wait an hour on the lock we hold), also when they
# run from a git worktree.
set -uo pipefail

LOCK="${WIN11_HEAVY_LOCK:-/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome/.heavy.lock}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MIN_MB=2048
TAG=heavy
WATCH_MB="${WIN11_HEAVY_WATCH_MB:-1500}"
while [ $# -gt 0 ]; do
  case "$1" in
    --min-mb) MIN_MB="$2"; shift 2 ;;
    --tag) TAG="$2"; shift 2 ;;
    --) shift; break ;;
    *) break ;;
  esac
done
[ $# -gt 0 ] || { echo "usage: tools/heavy.sh [--min-mb N] [--tag NAME] -- command..." >&2; exit 2; }

mem_mb() { awk '/^MemAvailable:/ {print int($2/1024)}' /proc/meminfo; }

# Nested: an ancestor heavy.sh holds this lock and its watchdog guards the whole group.
if [ -n "${WIN11_HEAVY_HELD:-}" ] && [ "$(realpath -m "$WIN11_HEAVY_HELD")" = "$(realpath -m "$LOCK")" ] \
   && ! flock -n "$LOCK" true 2>/dev/null; then
  exec "$@"
fi
LOGDIR="$ROOT/tests/logs"
mkdir -p "$LOGDIR"
STAMP="$(date +%Y%m%d-%H%M%S)-$$"
TIMEF="$LOGDIR/heavy-$TAG-$STAMP.time"
EVENTS="$LOGDIR/heavy-events.log"

exec 9>"$LOCK"
if ! flock -w 3600 9; then
  echo "heavy.sh: lock timeout after 3600 s ($LOCK)" | tee -a "$EVENTS" >&2
  exit 75
fi

avail="$(mem_mb)"
disk_mb="$(df -Pm "$ROOT" | awk 'NR==2 {print $4}')"
if [ "$avail" -lt "$MIN_MB" ]; then
  echo "heavy.sh: only ${avail} MB available, need ${MIN_MB} MB; not running: $*" | tee -a "$EVENTS" >&2
  exit 75
fi
if [ "$disk_mb" -lt 5120 ]; then
  echo "heavy.sh: only ${disk_mb} MB free disk, need 5120 MB; not running: $*" | tee -a "$EVENTS" >&2
  exit 75
fi

export CARGO_NET_OFFLINE=true
# Children (also tools in a git worktree, whose own .heavy.lock would be another file) use this same lock.
export WIN11_HEAVY_LOCK="$LOCK" WIN11_HEAVY_HELD="$LOCK"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
export WIN11_BUILD_JOBS="${WIN11_BUILD_JOBS:-2}"

# No job control in a script, so the background child is not a group leader and setsid does not fork:
# $! is the new process group id.
setsid nice -n 10 ionice -c3 /usr/bin/time -v -o "$TIMEF" "$@" 9>&- &
PG=$!
cleanup() { kill -TERM -- "-$PG" 2>/dev/null; sleep 3; kill -KILL -- "-$PG" 2>/dev/null; }
trap 'cleanup; exit 130' INT TERM HUP

killed=0
while kill -0 "$PG" 2>/dev/null; do
  now="$(mem_mb)"
  if [ "$now" -lt "$WATCH_MB" ]; then
    echo "$(date -Is) heavy.sh watchdog: MemAvailable ${now} MB < ${WATCH_MB} MB, killing group $PG ($TAG): $*" | tee -a "$EVENTS" >&2
    cleanup
    killed=1
    break
  fi
  sleep 1
done
wait "$PG"
rc=$?
[ "$killed" = 1 ] && rc=137
echo "$(date -Is) heavy.sh rc=$rc tag=$TAG mem_start=${avail}MB time=$TIMEF cmd=$*" >> "$EVENTS"
exit "$rc"
