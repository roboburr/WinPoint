#!/usr/bin/env bash
# GW1: the bubblewrap test layer for every test that runs update code (design 4.11, 9.1; fix round: gw1.md).
#
#   tests/update/run.sh --self-test [--only NAME ... [--meta]] [--no-strace]
#       Plays all 32 timelines against the Python reference client inside the layer, plus the checks beyond
#       the timelines (paused Step(), broadcast, CallCount(), Load() transitions, activation, stub log, enums,
#       probe.py, a host socket outside the layer), then the guard refusals, then the refusals of probe.py and
#       the fake outside run.sh's layer (in a decoy sandbox whose /run/dbus/system_bus_socket and
#       /run/user/<uid>/bus are listening decoys that count connections), everything under strace. Exit 0 =
#       everything passed; without strace the self-test fails.
#   tests/update/run.sh exec [--timeline NAME [--run]] [--expect-stubs FILE.json] [--expect-calls N|FILE.json]
#                       [--timeout SEC] [--allow-layer-dgrams] [--no-strace] -- CMD [ARGS...]
#       Starts the private buses and the fake inside the layer, optionally Load()s (and Run()s) a timeline,
#       runs CMD (for GW2b: a Rust test binary built outside, inside this worktree, e.g.
#       target/debug/deps/win11_host-<hash>) with the seams exported, then checks the stub log (default: no
#       stub call at all), optionally the fake's calls since the last Load(), the fake's violations, that the
#       fake is still alive and (unless --no-strace) the strace check. Exit = CMD's exit code, or 1 when a
#       check fails. --allow-layer-dgrams (GW2b): addressed datagrams to sockets under the layer's own TMPDIR
#       ($RT/tmp/, a tmpfs that exists only inside the layer) pass the strace check: the host's event fan-out
#       between two services of one test (events.rs); every other addressed datagram still fails it.
#   tests/update/run.sh bus-tests [--only NAME ...] -- BIN
#       GW2b: every bus test of host/src/services/update/bus_tests.rs (BIN = the lib test binary built outside)
#       in its own `exec` (fresh run directory, buses and fake), with the expectations of
#       tests/update/stubs/bus-tests.json (stub log in order, the fake's calls since the last Load, timeout,
#       datagrams). First `BIN --list --ignored` (inside the layer) must name exactly the tests of that JSON (or
#       include every --only name); each run must end with "test result: ok. 1 passed" (a misspelt or missing
#       test runs 0 tests and fails). Exit 0 = every test and every check of its run passed.
#
# The layer (bwrap) is built from an allowlist, not `--ro-bind / /`: read-only /usr (+ the /bin, /sbin, /lib,
# /lib64 links), /etc, /sys and this worktree W; tests/update/.run/<stamp>/ as the only writable bind; private
# /dev, /proc, /tmp, /var (tmpfs, /var/run -> /run) and /run (so /run/dbus and /run/user are empty); every
# listening host socket that would still be visible (in W, /usr, /etc, /sys: from /proc/net/unix, read outside)
# is masked with /dev/null; `/` is read-only; --unshare-pid --unshare-net --unshare-ipc --die-with-parent
# --new-session (bwrap also sets no_new_privs). The real home, /var/snap, /var/lib, /srv, /opt, /mnt and
# /media do not exist inside. Every process gets DBUS_SYSTEM_BUS_ADDRESS = the fake's private bus,
# <run>/guard-bin first on PATH, NO_AT_BRIDGE=1 GTK_A11Y=none GSETTINGS_BACKEND=keyfile, GIO_USE_VFS=local (WU-FIX,
# D-G46: GIO's local VFS only, so no GVfs client contacts a session bus), a private HOME/XDG
# tree in <run>, XDG_RUNTIME_DIR=/tmp/w11-update, no DISPLAY/WAYLAND_DISPLAY. Exported seams:
# WIN11_UPDATE_PK_BUS, WIN11_UPDATE_FIXTURE_ROOT, WIN11_UPDATE_TOOL_DIR (each directory holds
# .win11-update-fixture). The caller's own environment is dropped (env -i), never inherited.
# Nothing here needs the heavy lock (no cargo, no gnome-shell, about 60 MB); build Rust test binaries outside
# with tools/heavy.sh -- tests/rust-sandbox/run-tests.sh ... --no-run and pass the binary to `exec`.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
W="$(cd "$HERE/../.." && pwd -P)"
PY=/usr/bin/python3
FAKE="$HERE/fake_packagekit.py"
RT=/tmp/w11-update
UIDN="$(id -u)"
# GW2b (review F10): the host's network namespace, read here, outside the layer; bus_tests.rs refuses to run when
# its own namespace equals it (--unshare-net missing). Not a WIN11_UPDATE_* name: those are the service's seams.
HOST_NETNS="$(readlink /proc/self/ns/net)"
DGRAM_OK=
REAL_HOME="$(getent passwd "$UIDN" | cut -d: -f6)"

refuse() { echo "GUARD REFUSED [$1] $2" >&2; exit 2; }

# ---------------------------------------------------------------------------------------------------------
# Inner stage: runs inside bwrap. Cheap checks first (before any Python), then fake_packagekit.py's guard.
# ---------------------------------------------------------------------------------------------------------
if [ "${1:-}" = __inner ]; then
  shift
  [ "$(id -u)" != 0 ] || refuse root "running as root"
  if [ -e /run/dbus/system_bus_socket ] || [ -L /run/dbus/system_bus_socket ]; then
    refuse system-bus-visible "/run/dbus/system_bus_socket is visible"
  fi
  case "${XDG_RUNTIME_DIR:-}" in /run/user/*) refuse runtime-live "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR";; esac
  for v in DBUS_SESSION_BUS_ADDRESS DBUS_SYSTEM_BUS_ADDRESS WIN11_UPDATE_PK_BUS; do
    val="${!v:-}"
    lower="${val,,}"
    case "$lower" in *"/run/user/"*|*"%2frun%2fuser"*) refuse bus-live "$v points under /run/user/: $val";; esac
  done
  [ "$(cat /proc/1/comm 2>/dev/null)" = bwrap ] || refuse not-sandboxed-pid "PID 1 is not bwrap"
  # F1: every host socket run.sh found must be masked, and the worktree (the only visible tree that is not
  # root's) must hold no socket at all.
  run="${WIN11_FAKE_RUN_DIR:-}"
  [ -n "$run" ] && [ -f "$run/masked-sockets.txt" ] || refuse masked-list-missing "no <run>/masked-sockets.txt"
  while IFS= read -r p; do
    [ -z "$p" ] || [ ! -S "$p" ] || refuse host-socket-visible "$p is still a socket inside the layer"
  done < "$run/masked-sockets.txt"
  # find -type s goes by the directory entry, which still says "socket" for a masked path: re-check with stat.
  while IFS= read -r -d '' s; do
    [ ! -S "$s" ] || refuse host-socket-visible "$s is a socket inside the layer (in the worktree)"
  done < <(find "$W" -type s -print0 2>/dev/null || true)
  exec "$PY" -I -B "$FAKE" layer "$@"
fi

[ "$UIDN" != 0 ] || refuse root "run.sh refuses to run as root"
command -v /usr/bin/bwrap >/dev/null || { echo "run.sh: /usr/bin/bwrap is missing" >&2; exit 3; }

usage() { sed -n '4,/^# The layer/{/^# The layer/!p}' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

# ---------------------------------------------------------------------------------------------------------
# Run directories: tests/update/.run/<stamp>/ (0700), locked for the run's lifetime (F12)
# ---------------------------------------------------------------------------------------------------------
RUNS="$HERE/.run"
KEEP_RUNS="${W11_KEEP_RUNS:-6}"
new_run() {
  mkdir -p "$RUNS"
  [ -e "$RUNS/.gitignore" ] || printf '*\n' > "$RUNS/.gitignore"
  RUN="$RUNS/$(date +%Y%m%d-%H%M%S)-$$-$RANDOM"
  mkdir -m 0700 "$RUN"
  local fd
  # Held by this shell and inherited by everything it starts (bwrap and the whole layer), so the lock lasts
  # exactly as long as some process of this run is alive.
  exec {fd}>"$RUN/.lock"
  flock -n "$fd" || { echo "run.sh: cannot lock $RUN" >&2; exit 3; }
  mkdir -p "$RUN"/{home,config,data,cache,state,logs,fixture,tools,guard-bin}
  # 0700 whatever the umask: the service's seam check refuses a group- or world-writable fixture or tool dir
  chmod 0700 "$RUN/home" "$RUN/fixture" "$RUN/tools"
  prune_runs
}
# Keep the newest $KEEP_RUNS run directories (the new one included); never delete one that is in use.
prune_runs() {
  local all=() d i
  for d in "$RUNS"/20[0-9][0-9][01][0-9][0-3][0-9]-[0-9][0-9][0-9][0-9][0-9][0-9]-*; do [ -d "$d" ] && all+=("$d"); done
  for ((i = 0; i < ${#all[@]} - KEEP_RUNS; i++)); do
    d="${all[$i]}"
    [ "$d" != "$RUN" ] || continue
    if flock -n "$d/.lock" true 2>/dev/null; then
      rm -rf --one-file-system -- "$d"
    fi
  done
}

# The complete environment of the layer (nothing inherited). Mutations KEY=VALUE / KEY=<unset> (self-test only).
layer_env() {
  local name; name="$(id -un)"
  ENVV=("PATH=$RUN/guard-bin:/usr/bin:/bin" "HOME=$RUN/home" "USER=$name" "LOGNAME=$name" "SHELL=/bin/sh"
        "LANG=C.UTF-8" "TERM=dumb" "XDG_CONFIG_HOME=$RUN/config" "XDG_DATA_HOME=$RUN/data"
        "XDG_CACHE_HOME=$RUN/cache" "XDG_STATE_HOME=$RUN/state" "XDG_RUNTIME_DIR=$RT"
        "XDG_DATA_DIRS=/usr/local/share:/usr/share" "XDG_CONFIG_DIRS=/etc/xdg" "TMPDIR=$RT/tmp"
        "GSETTINGS_BACKEND=keyfile" "NO_AT_BRIDGE=1" "GTK_A11Y=none" "GIO_USE_VFS=local"
        "DBUS_SESSION_BUS_ADDRESS=unix:path=$RT/no-session-bus" "DBUS_SYSTEM_BUS_ADDRESS=unix:path=$RT/no-system-bus"
        "PYTHONDONTWRITEBYTECODE=1" "RUST_BACKTRACE=1" "WIN11_FAKE_RUN_DIR=$RUN" "WIN11_TEST_HOST_NETNS=$HOST_NETNS")
  local m key val i
  for m in "$@"; do
    key="${m%%=*}"; val="${m#*=}"
    for i in "${!ENVV[@]}"; do [ "${ENVV[$i]%%=*}" = "$key" ] && unset 'ENVV[i]'; done
    [ "$val" = "<unset>" ] || ENVV+=("$key=$val")
  done
}

# ---------------------------------------------------------------------------------------------------------
# F1: host sockets. /proc/net/unix (read here, outside: inside, --unshare-net shows only the layer's own) lists
# every bound unix socket of the host network namespace. Each one that would still be visible inside the layer
# (under /usr, /etc, /sys or W) is masked with --ro-bind /dev/null and listed for the inner guard.
# ---------------------------------------------------------------------------------------------------------
visible_inside() { case "$1" in /usr/*|/etc/*|/sys/*|"$W"/*) return 0;; esac; return 1; }
scan_sockets() {
  MASKS=()
  local list="$RUN/masked-sockets.txt" p real relative=0 total=0
  : > "$list"
  while IFS= read -r p; do
    total=$((total + 1))
    case "$p" in
      /*) ;;
      @*) continue ;;                       # abstract: isolated by --unshare-net
      *) relative=$((relative + 1)); continue ;;
    esac
    [ -S "$p" ] || continue                 # gone, or a name we cannot see anyway
    real="$(realpath -e -- "$p" 2>/dev/null)" || continue
    if visible_inside "$real"; then
      MASKS+=(--ro-bind /dev/null "$real")
      printf '%s\n' "$real" >> "$list"
    fi
  done < <(sed -n '2,$ s/^ *[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+//p' /proc/net/unix | sort -u)
  printf 'bound host socket names: %s; masked (visible inside otherwise): %s; relative names (not locatable, the inner find covers W): %s\n' \
    "$total" "$(grep -c . "$list" || true)" "$relative" > "$RUN/logs/socket-scan.txt"
}

# The allowlisted root, shared by the layer and the decoy sandbox.
root_argv() {
  ROOT=(--ro-bind /usr /usr)
  local d
  for d in bin sbin lib lib64 lib32 libx32; do
    if [ -L "/$d" ]; then ROOT+=(--symlink "$(readlink "/$d")" "/$d")
    elif [ -d "/$d" ]; then ROOT+=(--ro-bind "/$d" "/$d"); fi
  done
  ROOT+=(--ro-bind /etc /etc --ro-bind /sys /sys --dev /dev --proc /proc --tmpfs /tmp
         --tmpfs /run --dir /run/dbus --dir /run/user --tmpfs /var --symlink ../run /var/run --dir /var/tmp
         --ro-bind "$W" "$W" --bind "$RUN" "$RUN")
}

# bwrap argv of the layer; $1 = "decoy" binds a regular file at /run/dbus/system_bus_socket (guard self-test).
bwrap_argv() {
  root_argv
  BW=(/usr/bin/bwrap "${ROOT[@]}")
  if [ "${1:-}" = decoy ]; then
    : > "$RUN/decoy-system-bus-socket"
    BW+=(--ro-bind "$RUN/decoy-system-bus-socket" /run/dbus/system_bus_socket)
  fi
  BW+=("${MASKS[@]}" --remount-ro / --unshare-pid --unshare-net --unshare-ipc --die-with-parent --new-session
       --chdir "$W" -- /bin/bash "$HERE/run.sh" __inner)
}

# ---------------------------------------------------------------------------------------------------------
# strace: every connect(), sendto(), sendmsg() and sendmmsg() of bwrap and everything inside (F11: a datagram
# sent with an address bypasses connect()).
# ---------------------------------------------------------------------------------------------------------
STRACE=()
strace_on() {   # $1 = log name
  STRACE=()
  command -v strace >/dev/null || return 1
  mkdir -p "$RUN/strace"
  STRACE=(strace -f -ff -qq --seccomp-bpf -e trace=connect,sendto,sendmsg,sendmmsg -e signal=none
          -o "$RUN/strace/${1:-net}")
}

# $1 = log name, $2 = the only prefix a successful connect() may have (default: the private buses).
# Works on files (the logs hold one line per D-Bus message, about 100,000 in a full self-test).
strace_check() {
  local dir="$RUN/strace" name="${1:-net}" ok_prefix="${2:-$RT/dbus-}" files
  files=$(cd "$dir" 2>/dev/null && ls -1 2>/dev/null | grep -E "^$name\.[0-9]+$" || true)
  [ -n "$files" ] || { echo "strace: FAIL, no log"; return 1; }
  local nproc total naddr bad conn_ok send_addr split addr="$dir/$name.addr"
  nproc=$(grep -c . <<<"$files")
  # shellcheck disable=SC2086
  total=$(cd "$dir" && cat $files | grep -cE '^(connect|sendto|sendmsg|sendmmsg)\(' || true)
  # shellcheck disable=SC2086
  (cd "$dir" && cat $files | grep -E '^(connect|sendto|sendmsg|sendmmsg)\(.*sun_path=' > "$addr" || true)
  # shellcheck disable=SC2086
  split=$(cd "$dir" && cat $files | grep -cE 'resumed>|<unfinished' || true)
  naddr=$(grep -c . "$addr" || true)
  echo "strace: $total connect/sendto/sendmsg/sendmmsg calls in $nproc processes, $naddr with a unix address:"
  sed -E -n 's#^([a-z]+)\(.*sun_path=(@?"[^"]*")[^=]*= *(-?[0-9]+)( [A-Z]+)?.*#\1 \2 \3\4#p' "$addr" |
    sed -E 's#/dbus-[A-Za-z0-9]+"#/dbus-*"#; s# [0-9]+$# ok#; s# -1 # failed #' | sort | uniq -c | sed 's/^/    /' || true
  bad=$(grep -E 'sun_path="(/run/dbus/|/var/run/dbus/|/run/user/|/var/run/user/)' "$addr" || true)
  conn_ok=$(grep -E '^connect\(' "$addr" | grep -E '= 0$' | grep -vF "sun_path=\"$ok_prefix" || true)
  send_addr=$(grep -E '^send(to|msg|mmsg)\(' "$addr" | grep -vE '= -1 ' || true)
  # --allow-layer-dgrams: the event fan-out between two hosts of one test, to sockets in the layer's own tmpfs
  if [ -n "$DGRAM_OK" ]; then send_addr=$(grep -vF "sun_path=\"$DGRAM_OK" <<<"$send_addr" || true); fi
  local rc=0
  if [ -n "$bad" ]; then echo "strace: FAIL, an attempt at the real system bus or /run/user:"; head -5 <<<"$bad"; rc=1
  elif [ -n "$conn_ok" ]; then echo "strace: FAIL, a successful connect() elsewhere:"; head -5 <<<"$conn_ok"; rc=1
  elif [ -n "$send_addr" ]; then echo "strace: FAIL, a datagram sent with a unix address:"; head -5 <<<"$send_addr"; rc=1
  elif grep '<unfinished' "$addr" | grep -vF "sun_path=\"${DGRAM_OK:-$ok_prefix}" | grep -qvF "sun_path=\"$ok_prefix"; then
    echo "strace: FAIL, an interrupted call with a unix address outside ${ok_prefix}* (outcome unknown)"; rc=1
  else
    [ "$split" = 0 ] || echo "strace: note: $split interrupted calls without such an address"
    echo "strace: OK, nothing reached /run/dbus/system_bus_socket or /run/user/$UIDN/bus; every successful connect() went to ${ok_prefix}*; $( [ -n "$DGRAM_OK" ] && echo "addressed datagrams only to ${DGRAM_OK}* (the layer's own tmpfs)" || echo "no addressed datagram was sent")"
  fi
  # Keep the logs, compressed.
  # shellcheck disable=SC2086
  (cd "$dir" && tar -czf "$name.tar.gz" $files && rm -f $files)
  return $rc
}

# A listener that only counts connections ($1 = log, rest = socket paths); sets DECOY_PID.
DECOY_PID=
start_decoy() {
  local log="$1" ready="$RUN/decoy-$RANDOM.ready" i
  shift
  : > "$log"
  "$PY" -I -B "$FAKE" decoy --log "$log" --ready "$ready" "$@" &
  DECOY_PID=$!
  for i in $(seq 100); do [ -s "$ready" ] && return 0; sleep 0.05; done
  echo "run.sh: the decoy did not start" >&2; return 1
}
stop_decoy() {
  if [ -n "$DECOY_PID" ]; then kill "$DECOY_PID" 2>/dev/null || true; wait "$DECOY_PID" 2>/dev/null || true; DECOY_PID=; fi
}

cmd_exec() {
  local opts=() trace=1
  while [ $# -gt 0 ]; do
    case "$1" in
      --timeline) opts+=(--timeline "$2"); shift 2 ;;
      --run) opts+=(--run); shift ;;
      --expect-stubs) opts+=(--expect-stubs "$(realpath "$2")"); shift 2 ;;
      --expect-calls)
        if [[ "$2" =~ ^[0-9]+$ ]]; then opts+=(--expect-calls "$2"); else opts+=(--expect-calls "$(realpath "$2")"); fi
        shift 2 ;;
      --timeout) opts+=(--timeout "$2"); shift 2 ;;
      --allow-layer-dgrams) DGRAM_OK="$RT/tmp/"; shift ;;
      --strace) trace=1; shift ;;                 # the default since the fix round; kept for old command lines
      --no-strace) trace=0; shift ;;
      --) shift; break ;;
      *) usage ;;
    esac
  done
  [ $# -gt 0 ] || usage
  # The command must be visible inside the layer: a path in this worktree or in /usr.
  if [[ "$1" == */* ]]; then
    local target; target="$(cd "$W" && realpath -m -- "$1")"
    case "$target" in
      "$W"/*|/usr/*|/bin/*|/sbin/*) ;;
      *) echo "run.sh exec: $1 is outside $W and /usr, so it does not exist inside the layer" >&2; exit 2 ;;
    esac
  fi
  new_run
  scan_sockets
  layer_env
  bwrap_argv
  if [ "$trace" = 1 ] && ! strace_on net; then
    echo "run.sh exec: strace is missing (pass --no-strace to run without the connect()/sendto() check)" >&2; exit 3
  fi
  echo "run.sh exec: run directory $RUN" >&2
  set +e
  env -i "${ENVV[@]}" "${STRACE[@]}" "${BW[@]}" "${opts[@]}" exec -- "$@"
  local rc=$?
  set -e
  if [ "$trace" = 1 ]; then strace_check net >&2 || [ $rc -ne 0 ] || rc=1; fi
  exit $rc
}

# One guard case: run the layer and expect a refusal with the given code.
GUARD_PASS=0 GUARD_FAIL=0
guard_case() {
  local label="$1" expect="$2" variant="$3"; shift 3
  local out rc
  layer_env "$@"
  bwrap_argv "$variant"
  set +e
  out=$(env -i "${ENVV[@]}" "${BW[@]}" guard-only 2>&1); rc=$?
  set -e
  local codes; codes=$(grep -o 'GUARD REFUSED \[[a-z0-9-]*\]' <<<"$out" | sed 's/.*\[\(.*\)\]/\1/' | sort -u | tr '\n' ' ' || true)
  if { [ "$expect" = accept ] && [ $rc -eq 0 ] && grep -q 'GUARD OK' <<<"$out"; } ||
     { [ "$expect" != accept ] && [ $rc -eq 2 ] && grep -q "GUARD REFUSED \[$expect\]" <<<"$out"; }; then
    GUARD_PASS=$((GUARD_PASS + 1)); echo "PASS guard: $label -> rc $rc ${codes:-GUARD OK}"
  else
    GUARD_FAIL=$((GUARD_FAIL + 1)); echo "FAIL guard: $label -> rc $rc [$codes] (expected $expect)"; echo "$out" | tail -3
  fi
}

# F2: probe.py and the fake outside run.sh's layer. They run in a sandbox where /run/dbus/system_bus_socket and
# /run/user/<uid>/bus are listening DECOYS and the environment looks live, so a regression in their refusal
# code reaches a decoy, never the real system bus: every case must refuse and the decoys must count 0.
REFUSE_PASS=0 REFUSE_FAIL=0
decoy_sandbox_argv() {
  root_argv
  DB=(/usr/bin/bwrap "${ROOT[@]}" "${MASKS[@]}"
      --bind "$DECOY_DIR/system_bus_socket" /run/dbus/system_bus_socket
      --dir "/run/user/$UIDN" --bind "$DECOY_DIR/session_bus" "/run/user/$UIDN/bus"
      --remount-ro / --unshare-pid --unshare-net --unshare-ipc --die-with-parent --new-session --chdir "$W" --)
}
refusal_case() {   # label, expected exit code, KEY=VAL... -- argv
  local label="$1" want="$2"; shift 2
  local extra=()
  while [ "$1" != -- ]; do extra+=("$1"); shift; done
  shift
  local before after out rc n trace
  before=$(wc -l < "$DECOY_LOG")
  n=$((REFUSE_PASS + REFUSE_FAIL + 1))
  set +e
  out=$(env -i PATH=/usr/bin:/bin HOME="$REAL_HOME" USER="$(id -un)" LANG=C.UTF-8 XDG_RUNTIME_DIR="/run/user/$UIDN" \
        DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/$UIDN/bus" "${extra[@]}" \
        strace -f -ff -qq --seccomp-bpf -e trace=connect,sendto,sendmsg,sendmmsg -e signal=none \
          -o "$RUN/strace/refuse$n" "${DB[@]}" "$@" </dev/null 2>&1)
  rc=$?
  set -e
  after=$(wc -l < "$DECOY_LOG")
  trace=$(strace_check "refuse$n" "/nonexistent/" 2>&1 || true)
  if [ "$rc" = "$want" ] && grep -q 'REFUSED' <<<"$out" && [ "$after" = "$before" ] && grep -q '^strace: OK' <<<"$trace"; then
    REFUSE_PASS=$((REFUSE_PASS + 1))
    echo "PASS $label -> rc $rc $(grep -o 'REFUSED \[[a-z0-9-]*\]' <<<"$out" | head -1), decoy connections 0, $(head -1 <<<"$trace" | sed 's/ in [0-9]* processes//; s/^strace: //')"
  else
    REFUSE_FAIL=$((REFUSE_FAIL + 1))
    echo "FAIL $label -> rc $rc (want $want), decoy connections $((after - before)): $(tail -1 <<<"$out")"
    grep -v '^    ' <<<"$trace" | sed 's/^/    /'
  fi
}

cmd_self_test() {
  local only=() trace=1 meta=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --only) shift; while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do only+=("$1"); shift; done ;;
      --meta) meta=(--meta); shift ;;           # with --only: also the checks beyond the timelines
      --no-strace) trace=0; shift ;;
      *) usage ;;
    esac
  done
  local t0=$SECONDS rc_strace=0 strace_state=OK
  if ! command -v strace >/dev/null; then
    echo "run.sh: strace is missing: the connect()/sendto() checks cannot run, so the self-test fails" >&2
    strace_state="MISSING"; trace=0; rc_strace=1
  fi
  new_run
  local MAIN_RUN="$RUN"
  echo "== 1. timelines and checks inside the layer (run directory $RUN)"
  # F1 check: a listening socket outside the layer, in the worktree, created before the scan. (In .run/ itself:
  # a path inside the run directory would exceed the 108-byte sockaddr_un limit.)
  local outside="$RUNS/o$$.sock"
  start_decoy "$RUN/outside-decoy.log" "$outside"
  printf '{"socket": "%s"}\n' "$outside" > "$RUN/outside-decoy.json"
  scan_sockets
  layer_env
  bwrap_argv
  [ "$trace" = 1 ] && strace_on net
  local sel=()
  [ ${#only[@]} -gt 0 ] && sel=(--only "${only[@]}" "${meta[@]}")
  set +e
  env -i "${ENVV[@]}" "${STRACE[@]}" "${BW[@]}" selftest "${sel[@]}"
  local rc_main=$?
  set -e
  stop_decoy
  local outside_hits; outside_hits=$(grep -c . "$RUN/outside-decoy.log" || true)
  echo "outside decoy socket $outside ($(cat "$RUN/logs/socket-scan.txt")): $outside_hits connection(s)"
  [ "$outside_hits" = 0 ] || rc_main=1
  if [ "$trace" = 1 ]; then
    echo "== 2. connect()/sendto() check (strace -f over bwrap and everything inside)"
    strace_check net || { rc_strace=1; strace_state=FAIL; }
  elif [ "$rc_strace" = 0 ]; then
    strace_state="skipped (--no-strace)"
  fi
  echo "== 3. guard refusals"
  new_run
  scan_sockets
  guard_case "positive control (private env in the layer)" accept normal
  guard_case "/run/dbus/system_bus_socket visible in the layer" system-bus-visible decoy
  guard_case "XDG_RUNTIME_DIR=/run/user/$UIDN" runtime-live normal "XDG_RUNTIME_DIR=/run/user/$UIDN"
  guard_case "live session bus" bus-live normal "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$UIDN/bus"
  guard_case "live session bus, escaped" bus-live normal "DBUS_SESSION_BUS_ADDRESS=unix:path=%2Frun%2Fuser%2F$UIDN%2Fbus"
  guard_case "live session bus, second in the list" bus-live normal \
    "DBUS_SESSION_BUS_ADDRESS=unix:path=$RT/no-session-bus;unix:path=/run/user/$UIDN/bus"
  guard_case "real system bus address" system-bus-real normal "DBUS_SYSTEM_BUS_ADDRESS=unix:path=/run/dbus/system_bus_socket"
  guard_case "WIN11_UPDATE_PK_BUS under /run/user" bus-live normal "WIN11_UPDATE_PK_BUS=unix:path=/run/user/$UIDN/bus"
  guard_case "WIN11_UPDATE_PK_BUS preset (private path)" seam-preset normal "WIN11_UPDATE_PK_BUS=unix:path=$RT/x"
  guard_case "abstract session bus" bus-not-private normal "DBUS_SESSION_BUS_ADDRESS=unix:abstract=/tmp/dbus-x"
  guard_case "tcp system bus" bus-not-private normal "DBUS_SYSTEM_BUS_ADDRESS=tcp:host=localhost,port=1"
  guard_case "system bus unset" bus-missing normal "DBUS_SYSTEM_BUS_ADDRESS=<unset>"
  guard_case "real HOME" home-real normal "HOME=$REAL_HOME"
  guard_case "DISPLAY set" display-set normal "DISPLAY=:0"
  guard_case "WAYLAND_DISPLAY set" wayland-display-set normal "WAYLAND_DISPLAY=wayland-0"
  guard_case "AT_SPI_BUS_ADDRESS set" live-pointer normal "AT_SPI_BUS_ADDRESS=unix:path=$RT/at-spi"
  guard_case "XDG_SESSION_ID set" live-pointer normal "XDG_SESSION_ID=2"
  guard_case "NO_AT_BRIDGE missing" at-bridge normal "NO_AT_BRIDGE=<unset>"
  guard_case "GSETTINGS_BACKEND=dconf" gsettings-not-keyfile normal "GSETTINGS_BACKEND=dconf"
  guard_case "PATH without guard-bin first" path-not-guarded normal "PATH=/usr/bin:/bin"
  guard_case "live ~/.local/bin on PATH" path-live-home normal "PATH=$RUN/guard-bin:$REAL_HOME/.local/bin:/usr/bin:/bin"
  guard_case "a variable naming /run/user" live-runtime-ref normal "PULSE_SERVER=unix:/run/user/$UIDN/pulse/native"
  guard_case "composite live environment" runtime-live normal "XDG_RUNTIME_DIR=/run/user/$UIDN" \
    "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/$UIDN/bus" "HOME=$REAL_HOME" "DISPLAY=:0" "WAYLAND_DISPLAY=wayland-0"
  # F1: a socket in the worktree that appeared after the scan (so it is not masked) is found by the inner find.
  "$PY" -I -B -c 'import socket, sys; socket.socket(socket.AF_UNIX).bind(sys.argv[1])' "$RUNS/l$$.sock"
  guard_case "an unmasked socket in the worktree (created after the scan)" host-socket-visible normal
  rm -f "$RUNS/l$$.sock"
  mv "$RUN/masked-sockets.txt" "$RUN/masked-sockets.moved"
  guard_case "no masked-sockets list (a layer started without the scan)" masked-list-missing normal
  mv "$RUN/masked-sockets.moved" "$RUN/masked-sockets.txt"
  echo "guard: $GUARD_PASS/$((GUARD_PASS + GUARD_FAIL)) passed"

  echo "== 4. outside run.sh's layer: probe.py and the fake must refuse (decoy sandbox, each under strace)"
  mkdir -p "$RUN/strace"
  DECOY_DIR="$(mktemp -d /tmp/w11-update-decoy.XXXXXXXX)"
  DECOY_LOG="$RUN/decoy-refusals.log"
  start_decoy "$DECOY_LOG" "$DECOY_DIR/system_bus_socket" "$DECOY_DIR/session_bus"
  decoy_sandbox_argv
  local M="$RUN/decoy-marked" N="$RUN/decoy-nomarker" R="$RUN/decoy-rt"
  mkdir -p "$M" "$N"; mkdir -m 0700 "$R"
  : > "$M/.win11-update-fixture"; : > "$R/.win11-update-fixture"
  ln -sfn /run/dbus/system_bus_socket "$M/link"; ln -sfn "/run/user/$UIDN/bus" "$M/link2"
  local P=(/usr/bin/python3 -I -B "$W/tools/update/probe.py") F=(/usr/bin/python3 -I -B "$FAKE")
  # Positive control: inside this sandbox the decoys really answer at the real paths (2 connections).
  local c0 c1 crc
  c0=$(grep -c . "$DECOY_LOG" || true)
  set +e
  env -i PATH=/usr/bin:/bin "${DB[@]}" /usr/bin/python3 -I -c 'import os, socket
for p in ("/run/dbus/system_bus_socket", "/run/user/%d/bus" % os.getuid()):
    s = socket.socket(socket.AF_UNIX); s.connect(p); s.send(b"control"); s.close()' </dev/null
  crc=$?
  set -e
  sleep 0.5
  c1=$(grep -c . "$DECOY_LOG" || true)
  if [ "$crc" = 0 ] && [ $((c1 - c0)) = 2 ]; then
    echo "PASS positive control: a plain connect() in the decoy sandbox reaches both decoys (2 connections)"
  else
    echo "FAIL positive control: rc $crc, $((c1 - c0)) decoy connections (want 2)"; REFUSE_FAIL=$((REFUSE_FAIL + 1))
  fi
  local S=/run/dbus/system_bus_socket U="/run/user/$UIDN/bus"
  refusal_case "probe.py (no args)" 2 -- "${P[@]}"
  refusal_case "probe.py --get-updates (no bus)" 2 -- "${P[@]}" --get-updates
  refusal_case "probe.py --live (no terminal)" 2 -- "${P[@]}" --live --get-updates
  refusal_case "probe.py --bus <real system bus>" 2 -- "${P[@]}" --bus "unix:path=$S" --get-updates
  refusal_case "probe.py --bus <live session bus>" 2 -- "${P[@]}" --bus "unix:path=$U" --get-updates
  refusal_case "probe.py --bus //run/dbus/..." 2 -- "${P[@]}" --bus "unix:path=/$S" --get-updates
  refusal_case "probe.py --bus /run/dbus/../dbus/..." 2 -- "${P[@]}" --bus "unix:path=/run/dbus/../dbus/system_bus_socket" --get-updates
  refusal_case "probe.py --bus /proc/self/root/run/dbus/..." 2 -- "${P[@]}" --bus "unix:path=/proc/self/root$S" --get-updates
  refusal_case "probe.py --bus <marked dir, symlink to the system bus>" 2 -- "${P[@]}" --bus "unix:path=$M/link" --get-updates
  refusal_case "probe.py --bus <marked dir, symlink to the session bus>" 2 -- "${P[@]}" --bus "unix:path=$M/link2" --get-updates
  refusal_case "probe.py --bus <unmarked dir>" 2 -- "${P[@]}" --bus "unix:path=$N/sock" --get-updates
  refusal_case "probe.py --bus %2f-escaped system bus" 2 -- "${P[@]}" --bus "unix:path=%2frun%2fdbus%2fsystem_bus_socket" --get-updates
  refusal_case "probe.py --bus /var/run/dbus/..." 2 -- "${P[@]}" --bus "unix:path=/var/run/dbus/system_bus_socket" --get-updates
  refusal_case "probe.py --bus <marked symlink>,guid=" 2 -- "${P[@]}" --bus "unix:path=$M/link,guid=00" --get-updates
  refusal_case "probe.py --bus unix:runtime=yes" 2 -- "${P[@]}" --bus "unix:runtime=yes" --get-updates
  refusal_case "probe.py --bus <address list>" 2 -- "${P[@]}" --bus "unix:path=$M/x;unix:path=$S" --get-updates
  refusal_case "probe.py --bus <valid private bus, real socket visible>" 2 XDG_RUNTIME_DIR="$R" -- \
    "${P[@]}" --bus "unix:path=$R/bus" --get-updates
  refusal_case "fake serve --bus <real system bus>" 2 -- "${F[@]}" serve --bus "unix:path=$S"
  refusal_case "fake serve --bus <layer path, no layer>" 2 -- "${F[@]}" serve --bus "unix:path=$RT/dbus-x"
  refusal_case "fake serve --bus <valid private bus, real socket visible>" 2 XDG_RUNTIME_DIR="$R" -- \
    "${F[@]}" serve --bus "unix:path=$R/bus"
  refusal_case "fake activate (starter = the real system bus)" 2 DBUS_STARTER_ADDRESS="unix:path=$S" -- "${F[@]}" activate
  refusal_case "fake layer guard-only (no layer)" 2 -- "${F[@]}" layer guard-only
  refusal_case "fake layer exec (no layer)" 2 -- "${F[@]}" layer exec -- /usr/bin/true
  refusal_case "fake selftest (no layer)" 2 -- "${F[@]}" selftest
  refusal_case "fake guard (no layer)" 2 -- "${F[@]}" guard
  refusal_case "run.sh inner stage (no layer)" 2 -- /bin/bash "$HERE/run.sh" __inner guard-only
  stop_decoy
  rm -rf --one-file-system -- "$DECOY_DIR"
  echo "outside the layer: $REFUSE_PASS/$((REFUSE_PASS + REFUSE_FAIL)) passed; decoy connections besides the 2 controls: $(( $(grep -c . "$DECOY_LOG" || true) - 2 ))"
  echo "== summary"
  echo "inside the layer: rc $rc_main (details in $MAIN_RUN/selftest.json)"
  echo "connect()/sendto() check: $strace_state"
  echo "guard refusals: $GUARD_PASS/$((GUARD_PASS + GUARD_FAIL))"
  echo "outside the layer (probe.py, fake, inner stage; decoy sandbox): $REFUSE_PASS/$((REFUSE_PASS + REFUSE_FAIL))"
  echo "self-test time: $((SECONDS - t0)) s"
  if [ $rc_main = 0 ] && [ $rc_strace = 0 ] && [ $GUARD_FAIL = 0 ] && [ $REFUSE_FAIL = 0 ]; then
    echo "SELF-TEST PASSED"; exit 0
  fi
  echo "SELF-TEST FAILED"; exit 1
}

# GW2b: each bus test in its own exec, with its expectations from tests/update/stubs/bus-tests.json.
cmd_bus_tests() {
  local only=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --only) shift; while [ $# -gt 0 ] && [ "${1#--}" = "$1" ]; do only+=("$1"); shift; done ;;
      --) shift; break ;;
      *) usage ;;
    esac
  done
  [ $# -eq 1 ] || usage
  local bin="$1" spec="$HERE/stubs/bus-tests.json" expdir list name calls timeout dgrams rc pass=0 fail=0 failed=()
  mkdir -p "$RUNS"
  [ -e "$RUNS/.gitignore" ] || printf '*\n' > "$RUNS/.gitignore"
  expdir="$(mktemp -d "$RUNS/bus-expect.XXXXXX")"
  # one line per test (name, calls, timeout, datagrams); its stub expectation goes to $expdir/<name>.json
  list=$("$PY" -I -B - "$spec" "$expdir" "${only[@]}" <<'PYEOF'
import json, sys
spec, out, only = json.load(open(sys.argv[1], encoding='utf-8')), sys.argv[2], sys.argv[3:]
tests = spec['tests']
for name in only:
    if name not in tests:
        sys.exit(f'bus-tests: no test {name} in {sys.argv[1]}')
for name, t in tests.items():
    if only and name not in only:
        continue
    with open(f'{out}/{name}.json', 'w', encoding='utf-8') as f:
        json.dump(t.get('stubs', []), f)
    calls = t.get('calls')
    print('\t'.join([name, '-' if calls is None else str(calls), str(t.get('timeout', 600)), '1' if t.get('datagrams') else '0']))
PYEOF
) || { rm -rf "$expdir"; exit 2; }
  # review m3: the binary's bus tests (listed inside the layer) are exactly the JSON's, or include every --only name
  echo "== bus test list ($(date +%H:%M:%S))"
  set +e
  ( cmd_exec --timeout 120 -- "$bin" services::update::bus_tests:: --list --ignored ) > "$expdir/list.out" 2> "$expdir/list.err"
  rc=$?
  set -e
  local listed wanted
  listed=$(sed -n 's/^services::update::bus_tests::\([a-z0-9_]*\): test$/\1/p' "$expdir/list.out" | sort)
  wanted=$(cut -f1 <<<"$list" | sort)
  if [ "$rc" != 0 ] || [ -z "$listed" ]; then
    echo "bus-tests: listing the bus tests of $bin failed (rc $rc):"; tail -5 "$expdir/list.err"; rm -rf "$expdir"; exit 1
  fi
  if [ ${#only[@]} -eq 0 ]; then
    if [ "$listed" != "$wanted" ]; then
      echo "bus-tests: the binary's bus tests and $spec differ:"
      diff <(echo "$listed") <(echo "$wanted") | sed -n 's/^< /    only in the binary: /p; s/^> /    only in the JSON: /p'
      rm -rf "$expdir"; exit 1
    fi
  else
    for name in "${only[@]}"; do
      grep -qxF "$name" <<<"$listed" || { echo "bus-tests: $bin has no bus test $name"; rm -rf "$expdir"; exit 1; }
    done
  fi
  echo "bus-tests: $(grep -c . <<<"$listed") bus tests in the binary$([ ${#only[@]} -eq 0 ] && echo ", the same as in $(basename "$spec")")"
  while IFS=$'\t' read -r name calls timeout dgrams; do
    [ -n "$name" ] || continue
    local args=(--expect-stubs "$expdir/$name.json" --timeout "$timeout")
    [ "$calls" = - ] || args+=(--expect-calls "$calls")
    [ "$dgrams" = 0 ] || args+=(--allow-layer-dgrams)
    echo "== bus test $name ($(date +%H:%M:%S))"
    set +e
    ( cmd_exec "${args[@]}" -- "$bin" "services::update::bus_tests::$name" --exact --ignored --test-threads=1 --nocapture ) 2>&1 |
      tee "$expdir/$name.log"
    rc=${PIPESTATUS[0]}
    set -e
    # review m3: rc 0 with "running 0 tests" (a misspelt or removed test) is no pass
    if [ "$rc" = 0 ] && ! grep -qE '^test result: ok\. 1 passed; 0 failed' "$expdir/$name.log"; then
      echo "== bus test $name: rc 0 but not exactly one test passed"
      rc=1
    fi
    if [ "$rc" = 0 ]; then pass=$((pass + 1)); else fail=$((fail + 1)); failed+=("$name(rc $rc)"); fi
    echo "== bus test $name: rc $rc"
  done <<<"$list"
  rm -rf "$expdir"
  echo "bus tests: $pass passed, $fail failed${failed[*]:+: ${failed[*]}}"
  [ "$fail" = 0 ]
}

case "${1:-}" in
  --self-test) shift; cmd_self_test "$@" ;;
  exec) shift; cmd_exec "$@" ;;
  bus-tests) shift; cmd_bus_tests "$@" ;;
  *) usage ;;
esac
