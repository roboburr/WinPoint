#!/bin/bash
# Run one command in a private session that cannot reach Robin's live desktop (GE0a; notes/claude/plan.md ground
# rules; memory note "Private D-Bus clobbers a11y bus").
#
#   tests/i18n/private.sh WORKDIR CMD [ARG …]
#
# * env -i: nothing of the caller's session survives (no DISPLAY, WAYLAND_DISPLAY, DBUS_SESSION_BUS_ADDRESS,
#   XDG_SESSION_*, SSH_AUTH_SOCK …). Only PATH, the locale and TZ (both fixed) and the variables below are set.
# * HOME, XDG_CONFIG_HOME, XDG_DATA_HOME, XDG_CACHE_HOME, XDG_RUNTIME_DIR (0700) and TMPDIR live in WORKDIR, which
#   must be inside this repository's tests/ directory; GSETTINGS_BACKEND=keyfile, NO_AT_BRIDGE=1, GTK_A11Y=none.
# * Its own D-Bus session (dbus-run-session) with a generated config: an abstract socket (no file in /tmp) and no
#   service directories, so nothing can be D-Bus-activated on it (no at-spi bus launcher, no portal, no gvfs).
# * Guards, checked inside the session right before CMD runs: no /run/user/<uid> bus or runtime dir, no display,
#   keyfile GSettings, NO_AT_BRIDGE=1, HOME and XDG_RUNTIME_DIR inside WORKDIR. Any failure: exit 70, CMD not run.
#
# The caller holds the heavy lock and checks memory (run.sh); this script only isolates.
set -euo pipefail
[[ $# -ge 2 ]] || { echo "private.sh: usage: private.sh WORKDIR CMD [ARG …]" >&2; exit 2; }
T=$(cd "$(dirname "$0")" && pwd -P)
TESTS=$(cd "$T/.." && pwd -P)
W=$(realpath -m -- "$1"); shift
case "$W/" in "$TESTS"/?*) ;; *) echo "private.sh: WORKDIR $W is not inside $TESTS" >&2; exit 70 ;; esac
mkdir -p "$W"; W=$(cd "$W" && pwd -P)
case "$W/" in "$TESTS"/?*) ;; *) echo "private.sh: WORKDIR $W is not inside $TESTS" >&2; exit 70 ;; esac
mkdir -p "$W/home" "$W/config" "$W/data" "$W/cache" "$W/tmp" "$W/runtime"
chmod 0700 "$W/runtime" "$W/home"
BUS_NAME="w11-i18n-$(od -An -N8 -tx1 /dev/urandom | tr -d ' \n')"
cat >"$W/bus.conf" <<EOF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <keep_umask/>
  <listen>unix:abstract=$BUS_NAME</listen>
  <auth>EXTERNAL</auth>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
EOF
LIVE_UID=$(id -u)
exec env -i \
  PATH="/usr/local/bin:/usr/bin:/bin" \
  LANG=en_US.UTF-8 LC_ALL=en_US.UTF-8 LANGUAGE= TZ=Europe/Amsterdam \
  HOME="$W/home" XDG_CONFIG_HOME="$W/config" XDG_DATA_HOME="$W/data" XDG_CACHE_HOME="$W/cache" \
  XDG_RUNTIME_DIR="$W/runtime" TMPDIR="$W/tmp" \
  XDG_DATA_DIRS=/usr/local/share:/usr/share XDG_CONFIG_DIRS=/etc/xdg \
  GSETTINGS_BACKEND=keyfile NO_AT_BRIDGE=1 GTK_A11Y=none GIO_USE_VFS=local GVFS_DISABLE_FUSE=1 \
  W11_PRIVATE_W="$W" W11_LIVE_UID="$LIVE_UID" \
  dbus-run-session --config-file="$W/bus.conf" -- /bin/bash -c '
    set -u
    bad() { echo "private.sh: refusing to run: $*" >&2; exit 70; }
    [[ -z ${DISPLAY:-} && -z ${WAYLAND_DISPLAY:-} ]] || bad "a display is set"
    case ${DBUS_SESSION_BUS_ADDRESS:-} in
      "") bad "no private bus" ;;
      *"/run/user/"*) bad "the bus is in /run/user: $DBUS_SESSION_BUS_ADDRESS" ;;
      unix:abstract=w11-i18n-*) ;;
      *) bad "unexpected bus address $DBUS_SESSION_BUS_ADDRESS" ;;
    esac
    [[ ${XDG_RUNTIME_DIR:-} == "$W11_PRIVATE_W/runtime" && $XDG_RUNTIME_DIR != /run/user/* ]] || bad "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-}"
    [[ ${HOME:-} == "$W11_PRIVATE_W/home" ]] || bad "HOME=${HOME:-}"
    [[ ${GSETTINGS_BACKEND:-} == keyfile && ${NO_AT_BRIDGE:-} == 1 ]] || bad "GSETTINGS_BACKEND/NO_AT_BRIDGE"
    [[ $(stat -c %a "$XDG_RUNTIME_DIR") == 700 ]] || bad "XDG_RUNTIME_DIR is not 0700"
    unset W11_PRIVATE_W W11_LIVE_UID
    exec "$@"
  ' private-session "$@"
