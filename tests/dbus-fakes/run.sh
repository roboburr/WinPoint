#!/usr/bin/env bash
# Settings parity (notes/claude/settings-parity.md §7): the bubblewrap layer of tests/update/run.sh (GW1, D-G42),
# generalised for every batch's D-Bus fakes: private system and session buses, the batch's fakes on them, stubs first
# on PATH, the caller's command inside, and (with --strace) the check that nothing reached the real buses.
#
#   tests/dbus-fakes/run.sh --self-test
#       B0's fake (fakes/b0/echo.py) on both private buses, a probe that calls it (a plain call and one that allows
#       interactive authorization, as host/src/services/system/common.rs call_interactive does), under strace.
#   tests/dbus-fakes/run.sh [--batch NAME] [--fake FILE ...] [--strace] [--timeout SEC] -- CMD [ARGS...]
#       Starts every fakes/<NAME>/*.py (NAME: b0 a b c d e f) and each --fake, waits until each one printed
#       "ready", runs CMD (e.g. a Rust test binary built outside with tools/heavy.sh -- tests/rust-sandbox/run-tests.sh
#       --lib --no-run, then `BIN --ignored --exact <test>`), and exits with CMD's code (1 when a check fails).
#
# A fake is a Python file run as `python3 -I -B FILE CALLS.jsonl`: it owns its names on the bus(es) it wants
# (DBUS_SYSTEM_BUS_ADDRESS and DBUS_SESSION_BUS_ADDRESS point at the private ones), appends one JSON line per call it
# got to CALLS.jsonl (exported to CMD as WIN11_FAKE_CALLS_<NAME>, NAME = the file's stem in upper case), and prints
# "ready" once it owns them. fakes/b0/echo.py is the template.
#
# The layer: read-only /usr (+ its links), /etc, /sys and this worktree; the run directory .run/<stamp>/ as the only
# writable bind; private /dev, /proc, /tmp, /run (so /run/dbus and /run/user are empty) and /var; every listening host
# socket that would still be visible is masked (/proc/net/unix, read outside); / read-only; --unshare-pid
# --unshare-net --unshare-ipc --die-with-parent --new-session. env -i: the caller's environment is never inherited;
# PATH starts with the run's stub directory, which WIN11_SYSTEM_TOOL_DIR names too (host/src/tools.rs). Nothing here
# needs the heavy lock (no cargo, no gnome-shell).
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
W="$(cd "$HERE/../.." && pwd -P)"
PY=/usr/bin/python3
RT=/tmp/w11-fakes
UIDN="$(id -u)"

refuse() { echo "GUARD REFUSED [$1] $2" >&2; exit 2; }
usage() { sed -n '4,/^# A fake is/{/^# A fake is/!p}' "$0" | sed 's/^# \{0,1\}//'; exit 2; }

# ------------------------------------------------------------------------------------------- inner stage
if [ "${1:-}" = __inner ]; then
  shift
  [ "$(id -u)" != 0 ] || refuse root "running as root"
  [ ! -e /run/dbus/system_bus_socket ] || refuse system-bus-visible "/run/dbus/system_bus_socket is visible"
  [ "$(cat /proc/1/comm 2>/dev/null)" = bwrap ] || refuse not-sandboxed-pid "PID 1 is not bwrap"
  run="${WIN11_FAKE_RUN_DIR:?}"
  while IFS= read -r p; do
    [ -z "$p" ] || [ ! -S "$p" ] || refuse host-socket-visible "$p is still a socket inside the layer"
  done < "$run/masked-sockets.txt"
  mkdir -p "$RT/tmp"
  for bus in system session; do
    cat > "$run/$bus.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig><type>$bus</type><keep_umask/><apparmor mode="disabled"/>
<listen>unix:path=$RT/dbus-$bus</listen><auth>EXTERNAL</auth>
<policy context="default"><allow send_destination="*"/><allow receive_type="method_call"/>
<allow receive_type="method_return"/><allow receive_type="error"/><allow receive_type="signal"/><allow own="*"/></policy>
</busconfig>
CONF
    /usr/bin/dbus-daemon --config-file="$run/$bus.conf" --nofork --nopidfile >"$run/logs/dbus-$bus.log" 2>&1 &
  done
  for i in $(seq 100); do [ -S "$RT/dbus-system" ] && [ -S "$RT/dbus-session" ] && break; sleep 0.05; done
  export DBUS_SYSTEM_BUS_ADDRESS="unix:path=$RT/dbus-system" DBUS_SESSION_BUS_ADDRESS="unix:path=$RT/dbus-session"
  pids=()
  while IFS= read -r fake; do
    [ -n "$fake" ] || continue
    stem="$(basename "$fake" .py)"
    calls="$run/calls-$stem.jsonl"; : > "$calls"
    export "WIN11_FAKE_CALLS_$(tr 'a-z-' 'A-Z_' <<<"$stem")=$calls"
    "$PY" -I -B "$fake" "$calls" > "$run/logs/fake-$stem.out" 2> "$run/logs/fake-$stem.err" &
    pids+=($!)
    for i in $(seq 200); do grep -qx ready "$run/logs/fake-$stem.out" 2>/dev/null && break; sleep 0.05; done
    grep -qx ready "$run/logs/fake-$stem.out" || { echo "run.sh: fake $stem did not get ready" >&2; tail -3 "$run/logs/fake-$stem.err" >&2; exit 1; }
  done < "$run/fakes.txt"
  set +e
  timeout -k 5 "${WIN11_FAKE_TIMEOUT:-300}" "$@"
  rc=$?
  set -e
  for pid in "${pids[@]}"; do kill "$pid" 2>/dev/null || true; done
  exit $rc
fi

[ "$UIDN" != 0 ] || refuse root "run.sh refuses to run as root"
command -v /usr/bin/bwrap >/dev/null || { echo "run.sh: /usr/bin/bwrap is missing" >&2; exit 3; }

# ------------------------------------------------------------------------------------------ run directory
RUNS="$HERE/.run"
new_run() {
  mkdir -p "$RUNS"
  [ -e "$RUNS/.gitignore" ] || printf '*\n' > "$RUNS/.gitignore"
  RUN="$RUNS/$(date +%Y%m%d-%H%M%S)-$$-$RANDOM"
  mkdir -m 0700 "$RUN"
  mkdir -p "$RUN"/{home,config,data,cache,logs,guard-bin}
  chmod 0700 "$RUN/home" "$RUN/guard-bin"
  for name in pkexec systemctl systemd-run loginctl gsettings dconf gdbus busctl dbus-send nmcli grdctl tecla \
              bluetooth-sendto passwd localedef ubuntu-insights simple-scan snap snap-store gnome-software apt \
              apt-get pkcon update-manager orca ibus ibus-setup; do
    cat > "$RUN/guard-bin/$name" <<STUB
#!/bin/sh
printf '%s\t%s\n' "\$(date -Is)" "$name \$*" >> "$RUN/logs/stub-calls.log"
echo "dbus-fakes stub: $name is blocked" >&2
exit 97
STUB
    chmod 0755 "$RUN/guard-bin/$name"
  done
  : > "$RUN/logs/stub-calls.log"
  # keep the newest six runs
  ls -1d "$RUNS"/20* 2>/dev/null | head -n -6 | while IFS= read -r old; do rm -rf --one-file-system -- "$old"; done
}

visible_inside() { case "$1" in /usr/*|/etc/*|/sys/*|"$W"/*) return 0;; esac; return 1; }
scan_sockets() {
  MASKS=()
  local list="$RUN/masked-sockets.txt" p real
  : > "$list"
  while IFS= read -r p; do
    case "$p" in /*) ;; *) continue ;; esac
    [ -S "$p" ] || continue
    real="$(realpath -e -- "$p" 2>/dev/null)" || continue
    if visible_inside "$real"; then MASKS+=(--ro-bind /dev/null "$real"); printf '%s\n' "$real" >> "$list"; fi
  done < <(sed -n '2,$ s/^ *[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+//p' /proc/net/unix | sort -u)
}

bwrap_argv() {
  BW=(/usr/bin/bwrap --ro-bind /usr /usr)
  local d
  for d in bin sbin lib lib64 lib32 libx32; do
    if [ -L "/$d" ]; then BW+=(--symlink "$(readlink "/$d")" "/$d"); elif [ -d "/$d" ]; then BW+=(--ro-bind "/$d" "/$d"); fi
  done
  BW+=(--ro-bind /etc /etc --ro-bind /sys /sys --dev /dev --proc /proc --tmpfs /tmp --tmpfs /run --dir /run/dbus
       --dir /run/user --tmpfs /var --symlink ../run /var/run --ro-bind "$W" "$W" --bind "$RUN" "$RUN"
       "${MASKS[@]}" --remount-ro / --unshare-pid --unshare-net --unshare-ipc --die-with-parent --new-session
       --chdir "$W" -- /bin/bash "$HERE/run.sh" __inner)
}

layer_env() {
  local name; name="$(id -un)"
  ENVV=("PATH=$RUN/guard-bin:/usr/bin:/bin" "HOME=$RUN/home" "USER=$name" "LOGNAME=$name" "SHELL=/bin/sh"
        "LANG=C.UTF-8" "TERM=dumb" "XDG_CONFIG_HOME=$RUN/config" "XDG_DATA_HOME=$RUN/data" "XDG_CACHE_HOME=$RUN/cache"
        "XDG_RUNTIME_DIR=$RT" "XDG_DATA_DIRS=/usr/local/share:/usr/share" "TMPDIR=$RT/tmp"
        "GSETTINGS_BACKEND=keyfile" "NO_AT_BRIDGE=1" "GTK_A11Y=none" "GIO_USE_VFS=local"
        "WIN11_SYSTEM_TOOL_DIR=$RUN/guard-bin" "WIN11_FAKE_RUN_DIR=$RUN" "PYTHONDONTWRITEBYTECODE=1"
        "RUST_BACKTRACE=1" "WIN11_FAKE_TIMEOUT=${TIMEOUT:-300}")
}

STRACE=()
strace_check() {
  local files bad conn
  files=$(ls -1 "$RUN"/strace.* 2>/dev/null || true)
  [ -n "$files" ] || { echo "strace: FAIL, no log"; return 1; }
  # shellcheck disable=SC2086
  bad=$(cat $files | grep -E '^connect\(.*sun_path="(/run/dbus/|/var/run/dbus/|/run/user/|/var/run/user/)' || true)
  # shellcheck disable=SC2086
  conn=$(cat $files | grep -E '^connect\(.*sun_path=' | grep -E '= 0$' | grep -vF "sun_path=\"$RT/dbus-" || true)
  if [ -n "$bad" ]; then echo "strace: FAIL, an attempt at the real system bus or /run/user:"; head -5 <<<"$bad"; return 1; fi
  if [ -n "$conn" ]; then echo "strace: FAIL, a successful connect() outside the private buses:"; head -5 <<<"$conn"; return 1; fi
  echo "strace: OK, every successful connect() went to the private buses ($RT/dbus-*)"
}

run_layer() {   # fakes... -- cmd...
  local fakes=()
  while [ $# -gt 0 ] && [ "$1" != -- ]; do fakes+=("$1"); shift; done
  shift
  new_run
  printf '%s\n' "${fakes[@]}" > "$RUN/fakes.txt"
  scan_sockets
  layer_env
  bwrap_argv
  if [ "$TRACE" = 1 ]; then
    command -v strace >/dev/null || { echo "run.sh: strace is missing" >&2; exit 3; }
    STRACE=(strace -f -ff -qq -e trace=connect -e signal=none -o "$RUN/strace")
  fi
  echo "run.sh: run directory $RUN" >&2
  set +e
  env -i "${ENVV[@]}" "${STRACE[@]}" "${BW[@]}" "$@"
  local rc=$?
  set -e
  if [ "$TRACE" = 1 ]; then strace_check >&2 || [ $rc -ne 0 ] || rc=1; fi
  return $rc
}

TRACE=0 TIMEOUT=300
if [ "${1:-}" = --self-test ]; then
  TRACE=1
  probe=$(cat <<'PROBE'
import json, os, sys
from gi.repository import Gio, GLib
calls = os.environ["WIN11_FAKE_CALLS_ECHO"]
for kind in (Gio.BusType.SYSTEM, Gio.BusType.SESSION):
    bus = Gio.bus_get_sync(kind, None)
    plain = bus.call_sync("io.github.win11shell.Echo", "/io/github/win11shell/Echo", "io.github.win11shell.Echo",
                          "Echo", GLib.Variant("(s)", ("hallo",)), None, Gio.DBusCallFlags.NONE, 2000, None)
    assert plain.unpack() == ("hallo",), plain
    try:
        bus.call_sync("io.github.win11shell.Echo", "/io/github/win11shell/Echo", "io.github.win11shell.Echo", "Deny",
                      None, None, Gio.DBusCallFlags.ALLOW_INTERACTIVE_AUTHORIZATION, 2000, None)
        sys.exit("Deny answered")
    except GLib.Error as error:
        assert "NotAuthorized" in error.message, error.message
seen = [json.loads(l) for l in open(calls)]
assert [c["method"] for c in seen] == ["Echo", "Deny", "Echo", "Deny"], seen
assert [c["interactive"] for c in seen] == [False, True, False, True], seen
assert os.environ["WIN11_SYSTEM_TOOL_DIR"].endswith("/guard-bin")
assert not os.path.exists("/run/dbus/system_bus_socket")
print("probe: OK, the fake answered on both private buses and saw the interactive flag")
PROBE
)
  run_layer "$HERE/fakes/b0/echo.py" -- "$PY" -I -B -c "$probe"
  echo "self-test: OK"
  exit 0
fi

BATCH= EXTRA=()
while [ $# -gt 0 ]; do
  case "$1" in
    --batch) BATCH="$2"; shift 2 ;;
    --fake) EXTRA+=("$(realpath "$2")"); shift 2 ;;
    --strace) TRACE=1; shift ;;
    --timeout) TIMEOUT="$2"; shift 2 ;;
    --) shift; break ;;
    *) usage ;;
  esac
done
[ $# -gt 0 ] || usage
FAKES=()
if [ -n "$BATCH" ]; then
  [[ "$BATCH" =~ ^(b0|a|b|c|d|e|f)$ ]] || { echo "run.sh: --batch b0|a|b|c|d|e|f" >&2; exit 2; }
  for f in "$HERE/fakes/$BATCH"/*.py; do [ -e "$f" ] && FAKES+=("$f"); done
fi
FAKES+=("${EXTRA[@]}")
if [[ "$1" == */* ]]; then
  target="$(realpath -m -- "$1")"
  case "$target" in "$W"/*|/usr/*|/bin/*) ;; *) echo "run.sh: $1 is not visible inside the layer" >&2; exit 2 ;; esac
fi
run_layer "${FAKES[@]}" -- "$@"
