#!/bin/bash
# Run one heavy test command inside a bubblewrap layer that cannot reach the real system bus, the live session or
# the network (GW3 fix round, review finding B1; the layer of notes/claude/windows-update-gnome.md §4.11, built like
# tests/gnome/harness.py's bwrap_argv).
#
#   tools/heavy.sh --tag gw3-<what> -- tests/update-ui/nonet.sh CMD [ARG …]
#
#   e.g. tests/update-ui/nonet.sh tests/i18n/run.sh --lang en     the UI text dump, unchanged, inside the layer
#        tests/update-ui/shoot.sh                                 re-executes itself through this script
#
# Why: headless Chrome (started by tests/i18n/run.sh and tests/update-ui/shoot.sh through tests/i18n/private.sh)
# opens /run/dbus/system_bus_socket on its own (private.sh's env -i leaves DBUS_SYSTEM_BUS_ADDRESS unset, so libdbus
# falls back to the real socket) and probes internet addresses (literal IPs, which --host-resolver-rules does not
# cover). Inside this layer neither exists:
#
# * An allowlisted root, read-only (coordinator, after GW1 review F1: a read-only bind does not stop connect() on a
#   socket): /usr (+ the /bin, /sbin, /lib, /lib64 links), /etc, /sys, /opt (google-chrome), /var/cache/fontconfig
#   and this checkout. The real home, /var/snap, /var/lib and the rest of /var are not there. Every bound host
#   socket that would still be visible (from /proc/net/unix, read outside) is masked with /dev/null, and the inner
#   guard refuses if one is still a socket. Writable are only this checkout's tests/i18n/, tests/update-ui/ and .rt/
#   (the work dirs, baselines, screenshots and --out files) and the layer's own tmpfs /tmp, /var/tmp, HOME.
# * /run is a tmpfs with empty /run/dbus, /run/user, /run/systemd: no system-bus socket, no live session bus or
#   runtime dir, and none of systemd's sockets (journal, userdb varlink: user lookups stay in /etc/passwd).
# * --unshare-net: a network namespace with only loopback (127.0.0.1 for the test server). The live session's
#   abstract sockets belong to the host namespace, so they are unreachable too.
# * --unshare-pid, --unshare-ipc, --die-with-parent, --new-session. bwrap sets no_new_privs, so no setuid helper
#   (pkexec, chrome-sandbox) can elevate.
# * env -i: only PATH, LANG, TZ, HOME (tmpfs), DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent/system_bus_socket,
#   W11_NONET=1, the heavy-lock variables and the knobs run.sh/shoot.sh read (BUDGET, CHROME, LANGS, THEME, SIZE,
#   SUFFIX). CHROME defaults to tests/update-ui/chrome.sh: Chrome's own sandbox cannot start inside bwrap here, so
#   that wrapper adds --no-sandbox, and only after it has checked that it runs inside this layer.
# * Guards inside, right before CMD (exit 70, CMD not run): /run/dbus and /run/user empty, loopback is the only
#   network interface, the real HOME is not there, no masked path is a socket, PID 1 is bwrap.
# Verified with strace (notes/claude/wu/gw3.md, fix round): no connect to /run/dbus, /run/user or /run/systemd
# succeeds, and every AF_INET connect goes to 127.0.0.1 (Chrome's IPv6 route probe gets ENETUNREACH).
#
# Must run under tools/heavy.sh (it takes no lock of its own and refuses without one held). Exit: CMD's own; 2 usage;
# 70 a guard failed.
set -euo pipefail
U=$(cd "$(dirname "$0")" && pwd -P); R=$(cd "$U/../.." && pwd -P)
[[ $# -ge 1 ]] || { echo "nonet.sh: usage: nonet.sh CMD [ARG …]" >&2; exit 2; }
[[ -n "${WIN11_HEAVY_HELD:-}" ]] && ! flock -n "$WIN11_HEAVY_HELD" true 2>/dev/null \
  || { echo "nonet.sh: run me under tools/heavy.sh (no heavy lock held)" >&2; exit 2; }
BWRAP=/usr/bin/bwrap
[[ -x $BWRAP ]] || { echo "nonet.sh: $BWRAP is missing" >&2; exit 70; }
# from the file itself, not through NSS (getent would ask systemd-userdbd over its varlink sockets)
REAL_HOME=$(awk -F: -v u="$EUID" '$3 == u { print $6; exit }' /etc/passwd)
[[ -n $REAL_HOME ]] || { echo "nonet.sh: uid $EUID is not in /etc/passwd" >&2; exit 70; }
mkdir -p "$R/.rt"
ENVV=(PATH=/usr/local/bin:/usr/bin:/bin LANG=C.UTF-8 TZ=Europe/Amsterdam HOME=/tmp/home
  DBUS_SYSTEM_BUS_ADDRESS=unix:path=/nonexistent/system_bus_socket W11_NONET=1 W11_NONET_REAL_HOME="$REAL_HOME")
[[ -n "${CHROME+x}" ]] || ENVV+=("CHROME=$U/chrome.sh")
for v in WIN11_HEAVY_HELD WIN11_HEAVY_LOCK HEAVY_LOCK BUDGET CHROME LANGS THEME SIZE SUFFIX; do
  [[ -n "${!v+x}" ]] && ENVV+=("$v=${!v}")
done
# The heavy lock lives in the main checkout; from a git worktree it is outside R, so bind it (read-only) too.
EXTRA=()
LOCKF="${WIN11_HEAVY_HELD:-}"
if [[ -f $LOCKF ]]; then
  LOCKF=$(realpath -e -- "$LOCKF")
  case "$LOCKF" in "$R"/*) ;; *) EXTRA+=(--ro-bind "$LOCKF" "$LOCKF") ;; esac
fi
ROOT=(--ro-bind /usr /usr)
for d in bin sbin lib lib64 lib32 libx32; do
  if [[ -L /$d ]]; then ROOT+=(--symlink "$(readlink "/$d")" "/$d")
  elif [[ -d /$d ]]; then ROOT+=(--ro-bind "/$d" "/$d"); fi
done
ROOT+=(--ro-bind /etc /etc --ro-bind /sys /sys --dev /dev --proc /proc)
[[ -d /opt ]] && ROOT+=(--ro-bind /opt /opt)
ROOT+=(--tmpfs /tmp --dir /tmp/home --tmpfs /run --dir /run/dbus --dir /run/user --dir /run/systemd
       --tmpfs /var --symlink ../run /var/run --dir /var/tmp)
[[ -d /var/cache/fontconfig ]] && ROOT+=(--ro-bind /var/cache/fontconfig /var/cache/fontconfig)
ROOT+=(--ro-bind "$R" "$R" "${EXTRA[@]}")
# Host sockets: /proc/net/unix (here, outside; inside, --unshare-net lists only the layer's own) names every bound
# unix socket of the host. Mask each one that the binds above would show.
MASKS=() MASKED=""
while IFS= read -r p; do
  [[ $p == /* && -S $p ]] || continue
  real=$(realpath -e -- "$p" 2>/dev/null) || continue
  case "$real" in /usr/*|/etc/*|/sys/*|/opt/*|/var/cache/fontconfig/*|"$R"/*) ;; *) continue ;; esac
  MASKS+=(--ro-bind /dev/null "$real"); MASKED+="$real"$'\n'
done < <(sed -n '2,$ s/^ *[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+//p' /proc/net/unix | sort -u)
ENVV+=("W11_NONET_MASKED=$MASKED")
exec env -i "${ENVV[@]}" "$BWRAP" \
  "${ROOT[@]}" \
  --bind "$R/tests/i18n" "$R/tests/i18n" --bind "$R/tests/update-ui" "$R/tests/update-ui" --bind "$R/.rt" "$R/.rt" \
  "${MASKS[@]}" \
  --unshare-pid --unshare-net --unshare-ipc --die-with-parent --new-session \
  --chdir "$R" -- /bin/bash -c '
    set -u
    bad() { echo "nonet.sh: refusing to run: $*" >&2; exit 70; }
    [[ -z "$(ls -A /run/dbus)" ]] || bad "/run/dbus is not empty"
    [[ -z "$(ls -A /run/user)" ]] || bad "/run/user is not empty"
    [[ -z "$(ls -A /run/systemd)" ]] || bad "/run/systemd is not empty"
    ifs=$(awk -F: "NR > 2 { gsub(/ /, \"\", \$1); print \$1 }" /proc/net/dev | sort | tr "\n" " ")
    [[ $ifs == "lo " ]] || bad "network interfaces: $ifs(expected lo only)"
    case "$PWD/" in "$W11_NONET_REAL_HOME"/*) home_inner=1 ;; *) home_inner=0 ;; esac
    [[ $home_inner == 1 || ! -e $W11_NONET_REAL_HOME ]] || bad "$W11_NONET_REAL_HOME is visible"
    for d in "$W11_NONET_REAL_HOME/.cache" "$W11_NONET_REAL_HOME/.config" "$W11_NONET_REAL_HOME/.local" /var/snap /var/lib; do
      [[ ! -e $d ]] || bad "$d is visible"
    done
    while IFS= read -r m; do
      [[ -z $m || ! -S $m ]] || bad "host socket $m is still a socket"
    done <<< "$W11_NONET_MASKED"
    [[ $(cat /proc/1/comm) == bwrap ]] || bad "PID 1 is $(cat /proc/1/comm)"
    [[ $DBUS_SYSTEM_BUS_ADDRESS == unix:path=/nonexistent/* ]] || bad "DBUS_SYSTEM_BUS_ADDRESS=$DBUS_SYSTEM_BUS_ADDRESS"
    unset W11_NONET_REAL_HOME W11_NONET_MASKED
    exec "$@"
  ' nonet "$@"
