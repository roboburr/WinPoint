#!/usr/bin/env bash
# GE0b: run the host crate's Rust tests inside a private sandbox.
#
# * clean environment (env -i): no DISPLAY, WAYLAND_DISPLAY, AT-SPI, keyring or live bus variables
# * private HOME / XDG_{CONFIG,DATA,CACHE}_HOME / XDG_RUNTIME_DIR (0700) under tests/rust-sandbox/
# * private session bus (dbus-run-session) with a minimal config: no service directories, so nothing
#   is auto-activated on it; its socket lives in the short private TMPDIR (the repo path is too long
#   for sockaddr_un)
# * DBUS_SYSTEM_BUS_ADDRESS points at a socket that does not exist, so an accidental system-bus
#   connection fails instead of reaching the real system bus
# * GSETTINGS_BACKEND=keyfile, NO_AT_BRIDGE=1, GTK_A11Y=none
# * guard-bin/ stubs (pkexec, systemctl, gsettings, gnome-extensions, ...) first on PATH; every call is
#   logged to logs/guard-calls.log and fails
# * TMPDIR is a short private directory (/tmp/w11-ge0b-<pid>): Events::new binds Unix sockets below
#   std::env::temp_dir(), and sockaddr_un is limited to 107 bytes, which the repo path would exceed.
#   It is created 0700 and removed after the run.
# * MemAvailable must be above 2 GB; cargo runs under flock on $G/.heavy.lock with -j2 --offline --locked
# * hang guard: the tests are built first (--no-run), then the run gets WIN11_TEST_TIMEOUT seconds (default
#   900); a hung test binary is killed with exit 124 instead of holding the heavy lock (round 2, R2-D m1)
#
# * WIN11_CARGO_HOME / WIN11_CARGO_TARGET_DIR (optional): another cargo home (an offline crate cache) and target
#   directory, e.g. on a second disk when the main disk is nearly full; default $HOME/.cargo and $G/target
# * W11_TEST_BUILD_DIR (optional, passed on as an absolute path): the build directory with every scripts/install.py
#   BINS binary that the ELF tests read (post_upgrade.rs, services/update/elf.rs); default tests/build/native-release
#
# Usage: tests/rust-sandbox/run-tests.sh [cargo test args]   baseline: --lib (all-targets fails to compile, see
#        notes/claude/round0/ge0b.md); to skip a test: --lib -- --skip name
# Writes logs/test.log, logs/test.time, logs/guard-calls.log.
set -euo pipefail
SANDBOX="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
G="$(cd "$SANDBOX/../.." && pwd)"
LOCK="${WIN11_HEAVY_LOCK:-$G/.heavy.lock}"
# Under tools/heavy.sh (WIN11_HEAVY_HELD names this lock and someone holds it) the lock is already ours.
LOCKCMD=(flock -w 3600 "$LOCK")
if [[ -n "${WIN11_HEAVY_HELD:-}" && "$(realpath -m "$WIN11_HEAVY_HELD")" == "$(realpath -m "$LOCK")" ]] \
   && ! flock -n "$LOCK" true 2>/dev/null; then
  LOCKCMD=()
fi
REAL_HOME="${HOME:?}"
BUILD_DIR="${W11_TEST_BUILD_DIR:+$(realpath -m -- "$W11_TEST_BUILD_DIR")}"

mem_kb=$(awk '/^MemAvailable:/{print $2}' /proc/meminfo)
if (( mem_kb < 2 * 1024 * 1024 )); then
  echo "refusing: MemAvailable ${mem_kb} kB is below 2 GB" >&2
  exit 3
fi

mkdir -p "$SANDBOX"/{home,config,data,cache,run,guard-bin,logs}
chmod 0700 "$SANDBOX/run" "$SANDBOX/home"
: > "$SANDBOX/logs/guard-calls.log"

# Stubs for commands that could reach the live session or the system if a test went astray.
for name in pkexec systemctl systemd-run loginctl gsettings dconf gnome-extensions gnome-shell \
            gnome-session-quit apt apt-get pkcon do-release-upgrade update-manager gnome-software \
            notify-send xdg-open gio gtk-launch gdbus busctl dbus-send nmcli bluetoothctl \
            wpctl pactl udisksctl pkill killall update-desktop-database xdg-mime \
            win11-shell win11-explorer win11-panel win11-app win11-calendar win11-launch \
            win11-index-worker win11-image-worker win11-default-apps win11-file-portal \
            win11-installer win11-software-worker win11-gtk-explorer win11-gtk-settings win11-gtk-taskmgr \
            win11-gtk-alles win11-gtk-disks win11-gtk-defaultapps win11-gtk-installer; do
  cat > "$SANDBOX/guard-bin/$name" <<EOF
#!/bin/sh
printf '%s\t%s\n' "\$(date -Is)" "$name \$*" >> "$SANDBOX/logs/guard-calls.log"
echo "guard-bin: $name blocked in GE0b sandbox" >&2
exit 97
EOF
  chmod 0755 "$SANDBOX/guard-bin/$name"
done

TMP_PRIVATE="/tmp/w11-ge0b-$$"
mkdir -m 0700 "$TMP_PRIVATE"
cleanup() { rm -rf --one-file-system "$TMP_PRIVATE"; }
trap cleanup EXIT

cat > "$SANDBOX/session-bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:dir=$TMP_PRIVATE</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF

cat > "$SANDBOX/inner.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
fail() { echo "sandbox guard: $*" >&2; exit 4; }
[[ -z "${DISPLAY:-}" && -z "${WAYLAND_DISPLAY:-}" ]] || fail "display variable set"
[[ "$XDG_RUNTIME_DIR" == "$SANDBOX/run" ]] || fail "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR"
[[ "$HOME" == "$SANDBOX/home" ]] || fail "HOME=$HOME"
[[ "$GSETTINGS_BACKEND" == keyfile ]] || fail "GSETTINGS_BACKEND=$GSETTINGS_BACKEND"
[[ "${NO_AT_BRIDGE:-}" == 1 ]] || fail "NO_AT_BRIDGE"
[[ "${DBUS_SESSION_BUS_ADDRESS:-}" == "unix:path=$TMPDIR/dbus-"* ]] || fail "session bus $DBUS_SESSION_BUS_ADDRESS"
[[ "$DBUS_SESSION_BUS_ADDRESS" != *"/run/user/"* ]] || fail "live session bus"
[[ "$DBUS_SYSTEM_BUS_ADDRESS" == "unix:path=$SANDBOX/run/no-system-bus" ]] || fail "system bus"
[[ ! -e "$SANDBOX/run/no-system-bus" ]] || fail "system bus placeholder exists"
env | sort > "$SANDBOX/logs/test-env.txt"
cd "$G"
cargo_args=()
for a in "$@"; do [[ "$a" == -- ]] && break; cargo_args+=("$a"); done
cargo test --offline --locked -j2 --no-run "${cargo_args[@]}"
exec /usr/bin/time -v -o "$SANDBOX/logs/test.time" \
  timeout -k 10 "$WIN11_TEST_TIMEOUT" cargo test --offline --locked -j2 "$@"
EOF
chmod 0755 "$SANDBOX/inner.sh"

set +e
"${LOCKCMD[@]}" env -i \
  PATH="$SANDBOX/guard-bin:$REAL_HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin" \
  CARGO_HOME="${WIN11_CARGO_HOME:-$REAL_HOME/.cargo}" RUSTUP_HOME="$REAL_HOME/.rustup" \
  ${WIN11_CARGO_TARGET_DIR:+CARGO_TARGET_DIR="$WIN11_CARGO_TARGET_DIR"} \
  ${BUILD_DIR:+W11_TEST_BUILD_DIR="$BUILD_DIR"} \
  HOME="$SANDBOX/home" USER="${USER:-robin}" LOGNAME="${LOGNAME:-robin}" SHELL=/bin/bash \
  LANG=en_US.UTF-8 TERM=dumb \
  XDG_CONFIG_HOME="$SANDBOX/config" XDG_DATA_HOME="$SANDBOX/data" XDG_CACHE_HOME="$SANDBOX/cache" \
  XDG_RUNTIME_DIR="$SANDBOX/run" XDG_DATA_DIRS=/usr/local/share:/usr/share XDG_CONFIG_DIRS=/etc/xdg \
  TMPDIR="$TMP_PRIVATE" \
  GSETTINGS_BACKEND=keyfile NO_AT_BRIDGE=1 GTK_A11Y=none GIO_USE_VFS=local \
  DBUS_SYSTEM_BUS_ADDRESS="unix:path=$SANDBOX/run/no-system-bus" \
  SANDBOX="$SANDBOX" G="$G" WIN11_TEST_TIMEOUT="${WIN11_TEST_TIMEOUT:-900}" \
  dbus-run-session --config-file="$SANDBOX/session-bus.conf" -- "$SANDBOX/inner.sh" "$@" \
  > "$SANDBOX/logs/test.log" 2>&1
status=$?
set -e
echo "exit=$status" >> "$SANDBOX/logs/test.log"
(( status != 124 )) || echo "hang guard: the test run exceeded ${WIN11_TEST_TIMEOUT:-900} s and was killed" | tee -a "$SANDBOX/logs/test.log" >&2
leftover=$(find "$TMP_PRIVATE" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l)
echo "tmp entries left by tests: $leftover" >> "$SANDBOX/logs/test.log"
find "$TMP_PRIVATE" -mindepth 1 -maxdepth 1 -printf 'left: %f\n' 2>/dev/null | sort >> "$SANDBOX/logs/test.log" || true
exit $status
