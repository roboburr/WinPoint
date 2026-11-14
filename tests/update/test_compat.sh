#!/usr/bin/env bash
# GW6: the compatibility check's private apt tree against a signed file: archive (plan.md D-G27, design §6.2, C17).
#
#   tests/update/test_compat.sh [--record] [--only CASE ...] -- BIN
#       BIN = the lib test binary (<target>/debug/deps/win11_host-<hash>, built outside with
#       tools/heavy.sh --tag gw6-build -- tests/rust-sandbox/run-tests.sh --lib -- --list), copied into this worktree
#       when the target directory lies elsewhere (the layer sees only the worktree). --record also copies the apt
#       outputs to tests/update/fixtures/compat/recorded/ (the unit tests replay them).
#   tests/update/test_compat.sh --readelf
#       Only step 1 below.
#   tests/update/test_compat.sh --record-readelf
#       Writes tests/update/fixtures/compat/readelf-needed.txt from the build directory (after a rebuild of it).
#
# The programs are scripts/install.py `BINS` (every binary an install has) in W11_TEST_BUILD_DIR, by default
# tests/build/native-release (19 binaries and SHA256SUMS; elf.rs's unit test reads the same directory). A missing one
# fails step 1.
#
# D-G27's exception and nothing more: apt-get, apt-cache and apt-config run as this user, never as root, against a
# private tree in the test's own cache (under tests/update/.run/compat-<stamp>/), inside bwrap with --unshare-net,
# a private /run (empty /run/dbus and /run/user), a private /var (the system's dpkg state, lists and cache do not
# exist inside), a read-only allowlisted root, and the fixture's /etc/apt in place of the system's (it carries
# Ubuntu's Post-Invoke hooks and a proxy line, so the test can show that the tree's own APT_CONFIG reads neither
# hook). The archive is file:, signed with a throwaway ed25519 key made by tests/update/mkarchive.py in a GnuPG
# home under the run directory (its agent's sockets live in the layer's private /run/user/<uid>).
#
# Steps:
#   1. readelf -d on the BINS of the build directory equals tests/update/fixtures/compat/readelf-needed.txt (the
#      list elf.rs's unit test checks the parser against), and later the layer test parses the same binaries.
#   2. Stage 1 (bwrap, no network): the key, the five archives (ok, warn, hold, blocked, ubuntuHold) and the fixture
#      dpkg status.
#   3. Stage 2 (bwrap, no network, under strace): per case a fixture root (this machine's release files with the
#      archive's codename offered, the extension's metadata.json, ubuntu.sources pointing at the case's archive,
#      the throwaway keyring at the re-rooted ubuntu-archive-keyring.gpg path, the fixture status, our binaries)
#      and BIN's ignored test services::update::compat::tests::apt_tree_against_the_fixture_archive, which runs
#      compat.rs's full check with the real apt through the tool table's wrappers. A sixth case, cheap, uses the
#      embedded releases.json (stonking untested): the check must hold without starting any program.
#   4. Checks: each verdict; the system's apt-config dump shows the fixture's hooks while the tree's shows none and
#      keeps every Dir::Etc* inside the tree; the tree's sources carry each stanza's own components and the forced
#      Signed-By; apt ran exactly update, policy and the simulation, each once, with the tree's APT_CONFIG; strace:
#      every program started is on the allowlist (no hook program, no dpkg, no network method), no connect()
#      succeeded; no socket file in the worktree before or after; the tree is gone.
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
W="$(cd "$HERE/../.." && pwd -P)"
FIX="$HERE/fixtures/compat"
REL="$HERE/fixtures/release"
UIDN="$(id -u)"
HOST_NETNS="$(readlink /proc/self/ns/net)"
RT=/tmp/w11-compat
CASES=(ok warn hold blocked ubuntuHold cheap)
declare -A EXPECT=([ok]=ok [warn]=warn [hold]=hold [blocked]=blocked [ubuntuHold]=ubuntuHold [cheap]=hold)
RECORD=0 READELF_ONLY=0 READELF_RECORD=0 ONLY=() BIN=""
BUILD="${W11_TEST_BUILD_DIR:-$W/tests/build/native-release}"
mapfile -t BINS < <(/usr/bin/python3 -I -B -c 'import sys; sys.path.insert(0, sys.argv[1]); import install; print("\n".join(install.BINS))' "$W/scripts")
pass=0 fail=0
ok() { pass=$((pass + 1)); echo "  ok   $*"; }
bad() { fail=$((fail + 1)); echo "  FAIL $*"; }

while (($#)); do
  case "$1" in
    --record) RECORD=1; shift ;;
    --readelf) READELF_ONLY=1; shift ;;
    --record-readelf) READELF_ONLY=1 READELF_RECORD=1; shift ;;
    --only) shift; while (($#)) && [[ $1 != --* ]]; do ONLY+=("$1"); shift; done ;;
    --) shift; BIN="${1:-}"; shift || true ;;
    *) echo "usage: $0 [--record] [--only CASE ...] -- BIN | --readelf | --record-readelf" >&2; exit 2 ;;
  esac
done
((${#ONLY[@]})) && CASES=("${ONLY[@]}")

[ "$UIDN" != 0 ] || { echo "test_compat.sh: never as root" >&2; exit 2; }
((${#BINS[@]})) || { echo "test_compat.sh: no BINS in scripts/install.py" >&2; exit 2; }
for tool in /usr/bin/bwrap /usr/bin/strace /usr/bin/gpg /usr/bin/readelf /usr/bin/apt-get /usr/bin/python3; do
  [ -x "$tool" ] || { echo "test_compat.sh: $tool is missing" >&2; exit 3; }
done
sockets_in_w() { find "$W" -xdev -type s 2>/dev/null | head -5; }

# ------------------------------------------------------------------------------------------- 1. readelf
echo "== 1. readelf -d on the ${#BINS[@]} BINS of $BUILD against $FIX/readelf-needed.txt"
live="$(mktemp)"
trap 'rm -f "$live"' EXIT
n=0 missing=()
for name in $(printf '%s\n' "${BINS[@]}" | LC_ALL=C sort); do
  f="$BUILD/$name"
  if [ -f "$f" ] && [ -x "$f" ]; then n=$((n + 1)); else missing+=("$name"); continue; fi
  printf '%s: %s\n' "$name" "$(readelf -d "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | tr '\n' ' ' | sed 's/ $//')"
done > "$live"
if ((${#missing[@]})); then
  bad "$BUILD lacks ${missing[*]}"
elif ((READELF_RECORD)); then
  { printf '# readelf -d on the BINS of scripts/install.py in a build directory (NEEDED, in order), recorded by tests/update/test_compat.sh --record-readelf from %s; test_compat.sh checks it again on every run, elf.rs'"'"'s unit test compares the parser with it\n' "${BUILD#"$W"/}"
    cat "$live"; } > "$FIX/readelf-needed.txt"
  ok "$n binaries recorded into $FIX/readelf-needed.txt"
elif diff <(grep -v '^#' "$FIX/readelf-needed.txt") "$live" >/dev/null; then
  ok "$n binaries, readelf's NEEDED lists equal the recording"
else
  bad "readelf ($n binaries) differs from the recording:"; diff <(grep -v '^#' "$FIX/readelf-needed.txt") "$live" || true
fi
if ((READELF_ONLY)); then echo "== $pass passed, $fail failed"; ((fail == 0)); exit; fi
[ -n "$BIN" ] || { echo "test_compat.sh: no BIN (-- target/debug/deps/win11_host-<hash>)" >&2; exit 2; }
case "$(realpath -e "$BIN")" in "$W"/*) ;; *) echo "test_compat.sh: BIN must lie in this worktree" >&2; exit 2 ;; esac
BIN="$(realpath -e "$BIN")"
case "$(realpath -e "$BUILD")" in "$W"/*) ;; *) echo "test_compat.sh: the build directory must lie in this worktree (the layer sees only it)" >&2; exit 2 ;; esac
BUILD="$(realpath -e "$BUILD")"
s="$(sockets_in_w)"; [ -z "$s" ] || { echo "test_compat.sh: socket files in the worktree: $s" >&2; exit 3; }

# ------------------------------------------------------------------------------------------- run directory
RUNS="$HERE/.run"
mkdir -p "$RUNS"
[ -e "$RUNS/.gitignore" ] || printf '*\n' > "$RUNS/.gitignore"
RUN="$RUNS/compat-$(date +%Y%m%d-%H%M%S)-$$"
mkdir -m 0700 "$RUN"
exec {lockfd}>"$RUN/.lock"
flock -n "$lockfd"
# keep the newest four compat runs (never one in use)
mapfile -t old < <(ls -1d "$RUNS"/compat-* 2>/dev/null | sort | head -n -4)
for d in "${old[@]}"; do [ "$d" != "$RUN" ] && flock -n "$d/.lock" true 2>/dev/null && rm -rf --one-file-system -- "$d"; done
mkdir -m 0700 "$RUN/home" "$RUN/gnupg" "$RUN/tools" "$RUN/out" "$RUN/strace"
echo "== run directory $RUN"

# The allowlisted root of tests/update/run.sh's layer (read-only /usr, /etc, /sys, the worktree; private /dev,
# /proc, /tmp, /run, /var), every listening host socket that would still be visible masked with /dev/null.
MASKS=()
while IFS= read -r p; do
  case "$p" in /*) ;; *) continue ;; esac
  [ -S "$p" ] || continue
  real="$(realpath -e -- "$p" 2>/dev/null)" || continue
  case "$real" in /usr/*|/etc/*|/sys/*|"$W"/*) MASKS+=(--ro-bind /dev/null "$real") ;; esac
done < <(sed -n '2,$ s/^ *[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+[^ ]* \+//p' /proc/net/unix | sort -u)
layer_argv() {   # layer_argv EXTRA_BWRAP_ARGS... → BW (the bwrap argv, ending in --)
  local root=(--ro-bind /usr /usr) d
  for d in bin sbin lib lib64 lib32 libx32; do
    if [ -L "/$d" ]; then root+=(--symlink "$(readlink "/$d")" "/$d"); elif [ -d "/$d" ]; then root+=(--ro-bind "/$d" "/$d"); fi
  done
  root+=(--ro-bind /etc /etc --ro-bind /sys /sys --dev /dev --proc /proc --tmpfs /tmp
         --tmpfs /run --dir /run/dbus --dir /run/user --tmpfs /var --symlink ../run /var/run --dir /var/tmp
         --ro-bind "$W" "$W" --bind "$RUN" "$RUN")
  BW=(/usr/bin/bwrap "${root[@]}" "$@" "${MASKS[@]}" --remount-ro / --unshare-pid --unshare-net --unshare-ipc
      --die-with-parent --new-session --chdir "$W" --)
}
LAYER_ENV=(PATH=/usr/bin:/bin "HOME=$RUN/home" LANG=C.UTF-8 TERM=dumb "XDG_RUNTIME_DIR=$RT" "TMPDIR=$RT"
           "XDG_CONFIG_HOME=$RUN/home/.config" "XDG_DATA_HOME=$RUN/home/.local/share" "XDG_CACHE_HOME=$RUN/home/.cache"
           NO_AT_BRIDGE=1 GTK_A11Y=none GSETTINGS_BACKEND=keyfile PYTHONDONTWRITEBYTECODE=1 RUST_BACKTRACE=1
           "DBUS_SESSION_BUS_ADDRESS=unix:path=$RT/no-session-bus" "DBUS_SYSTEM_BUS_ADDRESS=unix:path=$RT/no-system-bus"
           "WIN11_TEST_HOST_NETNS=$HOST_NETNS")

# ------------------------------------------------------------------------------------------- 2. stage 1
echo "== 2. stage 1: throwaway key, archives, dpkg status (bwrap --unshare-net, GnuPG home $RUN/gnupg)"
stage1="$RUN/stage1.sh"
{
  echo 'set -eu'
  echo "[ ! -e /run/dbus/system_bus_socket ] || { echo 'system bus visible'; exit 70; }"
  echo "mkdir -p -m 0700 $RT"
  echo "python3 -I -B '$HERE/mkarchive.py' key --gnupghome '$RUN/gnupg' --keyring '$RUN/keyring.gpg'"
  for c in ok warn hold blocked ubuntuHold; do
    echo "python3 -I -B '$HERE/mkarchive.py' archive --gnupghome '$RUN/gnupg' --case $c --out '$RUN/archive/$c'"
  done
  echo "python3 -I -B '$HERE/mkarchive.py' status --out '$RUN/dpkg-status' --extended '$RUN/extended_states'"
  echo "gpgconf --homedir '$RUN/gnupg' --kill all >/dev/null 2>&1 || true"
} > "$stage1"
layer_argv --perms 0700 --dir "/run/user/$UIDN"
if env -i "${BW[@]}" /usr/bin/env "${LAYER_ENV[@]}" /bin/sh "$stage1" > "$RUN/stage1.log" 2>&1; then
  ok "key ($(stat -c %s "$RUN/keyring.gpg") bytes), 5 archives, status"
else
  bad "stage 1 failed:"; tail -20 "$RUN/stage1.log"; exit 1
fi
left="$(find "$RUN" -type s)"; [ -z "$left" ] || { bad "sockets left by GnuPG: $left"; find "$RUN" -type s -delete; }

# ------------------------------------------------------------------------------------------- fixtures
# the tool table: real apt through wrappers (PATH reset: apt's own helpers), a recorded dpkg-query, every other
# tool a stub that logs and fails
LOG="$RUN/stub-calls.log"
: > "$LOG"
: > "$RUN/tools/.win11-update-fixture"
for t in apt-get apt-cache apt-config; do
  cat > "$RUN/tools/$t" <<EOF
#!/bin/sh
printf '%s %s APT_CONFIG=%s\n' "$t" "\$*" "\${APT_CONFIG:-}" >> '$LOG'
PATH=/usr/bin:/bin; export PATH
exec /usr/bin/$t "\$@"
EOF
done
cat > "$RUN/tools/dpkg-query" <<EOF
#!/bin/sh
printf 'dpkg-query %s\n' "\$*" >> '$LOG'
while IFS= read -r l; do printf '%s\n' "\$l"; done < '$FIX/dpkg-query-S.txt'
EOF
for t in loginctl xdg-terminal-exec pkexec check-new-release snap python3 screen nice ionice sh do-release-upgrade release-upgrades-helper release-upgrade-attach; do
  printf '#!/bin/sh\nprintf "%%s %%s\\n" %s "$*" >> %s\nexit 97\n' "$t" "'$LOG'" > "$RUN/tools/$t"
done
chmod 0755 "$RUN"/tools/*
chmod 0700 "$RUN/tools"
fixture() {   # fixture CASE → $RUN/fixture-CASE
  local c="$1" root="$RUN/fixture-$1" archive="$RUN/archive/$1"
  [ "$c" = cheap ] && archive="$RUN/archive/ok"
  mkdir -m 0700 "$root"
  : > "$root/.win11-update-fixture"
  mkdir -p "$root/etc/apt/sources.list.d" "$root/etc/update-manager" "$root/usr/share/distro-info" \
           "$root/usr/share/keyrings" "$root/var/lib/dpkg" "$root/home/.cache/update-manager-core" \
           "$root/home/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local" "$root/bin" "$root/proc/sys/kernel/random"
  cp "$FIX/root/etc/os-release" "$root/etc/os-release"
  sed "s|/srv/w11-archive|$archive|g" "$FIX/root/etc/apt/sources.list.d/ubuntu.sources" > "$root/etc/apt/sources.list.d/ubuntu.sources"
  if [ "$c" = warn ]; then cp "$FIX"/root/etc/apt/sources.list.d/{google-chrome.sources,vscode.list} "$root/etc/apt/sources.list.d/"; fi
  cp "$FIX/root/home/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/metadata.json" \
     "$root/home/.local/share/gnome-shell/extensions/win11-gnome@win11shell.local/"
  cp "$REL/etc/update-manager/release-upgrades" "$root/etc/update-manager/"
  cp "$REL/distro-info/ubuntu.csv" "$root/usr/share/distro-info/"
  if [ "$c" = ubuntuHold ]; then meta=stonking-broken; else meta=stonking-supported; fi
  cp "$REL/meta-release/$meta" "$root/home/.cache/update-manager-core/meta-release"
  cp "$RUN/keyring.gpg" "$root/usr/share/keyrings/ubuntu-archive-keyring.gpg"
  cp "$RUN/dpkg-status" "$root/var/lib/dpkg/status"
  mkdir -p "$root/var/lib/apt" && cp "$RUN/extended_states" "$root/var/lib/apt/extended_states"
  echo 0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0 > "$root/proc/sys/kernel/random/boot_id"
  for name in "${BINS[@]}"; do ln -s "$BUILD/$name" "$root/bin/"; done
  # every case but cheap: a releases.json that marks the codename tested with GNOME 50 (the extension's major)
  [ "$c" = cheap ] || printf '%s\n' '{"version":1,"extension":"win11-gnome@win11shell.local","releases":{"stonking":{"version":"26.10","gnomeShell":"50","status":"tested","allowAnyway":false}}}' > "$root/releases.json"
  mkdir -m 0700 "$RUN/out/$c"
}

# ------------------------------------------------------------------------------------------- 3. stage 2
echo "== 3. stage 2: the real apt in the private tree (bwrap --unshare-net, the fixture /etc/apt, strace)"
stage2="$RUN/stage2.sh"
{
  echo 'set -u'
  echo "[ ! -e /run/dbus/system_bus_socket ] && [ ! -e /var/lib/dpkg/status ] && [ ! -e /run/user/$UIDN ] || { echo 'GUARD: a system path is visible'; exit 70; }"
  echo "[ \"\$(cat /proc/1/comm)\" = bwrap ] || { echo 'GUARD: PID 1 is not bwrap'; exit 70; }"
  echo "mkdir -p -m 0700 $RT"
  for c in "${CASES[@]}"; do
    echo "echo '### case $c'"
    echo "/usr/bin/env WIN11_UPDATE_FIXTURE_ROOT='$RUN/fixture-$c' WIN11_UPDATE_TOOL_DIR='$RUN/tools' W11_COMPAT_CASE=$c W11_COMPAT_OUT='$RUN/out/$c' \\"
    echo "  '$BIN' services::update::compat::tests::apt_tree_against_the_fixture_archive --exact --ignored --test-threads=1 --nocapture"
    echo "echo \"### rc \$?\""
  done
} > "$stage2"
for c in "${CASES[@]}"; do fixture "$c"; done
layer_argv --ro-bind "$FIX/system-apt" /etc/apt
env -i /usr/bin/strace -f -qq -e trace=execve,connect,sendto,sendmsg -o "$RUN/strace/trace" \
  "${BW[@]}" /usr/bin/env "${LAYER_ENV[@]}" /bin/sh "$stage2" > "$RUN/stage2.log" 2>&1 || true

# ------------------------------------------------------------------------------------------- 4. checks
echo "== 4. checks"
grep -q '^GUARD' "$RUN/stage2.log" && { bad "$(grep '^GUARD' "$RUN/stage2.log")"; }
PYCHECK='
import json, sys
case, want, log = sys.argv[1], sys.argv[2], open(sys.argv[3], encoding="utf-8").read()
block = log.split(f"### case {case}\n", 1)[1].split("### rc", 1)[0]
lines = [l[l.index("COMPAT-RESULT "):] for l in block.splitlines() if "COMPAT-RESULT " in l]
if not lines:
    print("no COMPAT-RESULT line"); sys.exit(1)
r = json.loads(lines[-1][len("COMPAT-RESULT "):])
rel, rec = r["release"], r["record"]
keys = ",".join(x["key"] + ":" + str(x["params"].get("part", "")) for x in rel["reasons"])
print("state=%s gnomeMajor=%s reasons=[%s] notes=%s archiveDate=%s" % (rel["state"], rel["gnomeMajor"], keys, rec.get("notes"), rec.get("archiveDate")))
sys.exit(0 if rel["state"] == want and "test result: ok. 1 passed" in block else 1)
'
for c in "${CASES[@]}"; do
  if out="$(/usr/bin/python3 -I -c "$PYCHECK" "$c" "${EXPECT[$c]}" "$RUN/stage2.log" 2>&1)"; then ok "$c → ${EXPECT[$c]}: $out"
  else bad "$c (want ${EXPECT[$c]}): $out"; fi
done
for c in "${CASES[@]}"; do
  o="$RUN/out/$c"
  if [ "$c" = cheap ]; then
    if grep -q 'cheap: update.release.compat answered {"started":false,"skipped":"hold"}, 0 programs started' "$RUN/stage2.log"; then
      ok "cheap: the embedded releases.json holds stonking; update.release.compat skipped it, 0 programs started, no tree"
    else bad "cheap: no 'cheap:' line with 0 programs"; fi
    continue
  fi
  grep -q 'Post-Invoke' "$o/apt-config-system.txt" && grep -q '^Acquire::http::Proxy "http://proxy.invalid:3128/";' "$o/apt-config-system.txt" \
    && ok "$c: the system's apt-config dump (fixture /etc/apt) shows its hooks and proxy" || bad "$c: the system dump lacks the fixture's hooks"
  tree="$(sed -n 's/^Dir "\(.*\)\/";$/\1/p' "$o/apt-config-tree.txt")"
  if [ -n "$tree" ] && ! grep -qiE 'invoke|::hooks|pre-install-pkgs' "$o/apt-config-tree.txt" \
     && grep -qi "^Dir::Etc::Parts \"$tree/etc/apt/apt.conf.d/\";" "$o/apt-config-tree.txt" \
     && grep -q "^Acquire::http::Proxy \"http://proxy.invalid:3128/\";" "$o/apt-config-tree.txt"; then
    ok "$c: apt-config dump under the tree: no hook, Dir::Etc::Parts inside the tree, the proxy copied"
  else bad "$c: the tree's apt-config dump"; fi
  if grep -qx 'Components: main restricted universe multiverse' "$o/release.sources" && grep -qx 'Components: main restricted universe' "$o/release.sources" \
     && grep -qx 'Suites: stonking stonking-updates' "$o/release.sources" && grep -qx 'Suites: stonking-security' "$o/release.sources" \
     && [ "$(grep -c "^Signed-By: $RUN/fixture-$c/usr/share/keyrings/ubuntu-archive-keyring.gpg$" "$o/release.sources")" = 2 ]; then
    ok "$c: the tree's sources: each stanza's own components, the codename's suites, Signed-By forced"
  else bad "$c: the tree's sources"; cat "$o/release.sources"; fi
done
# the stub log: per full case exactly dpkg-query, apt-config ×2, apt-get update, apt-cache policy, apt-get -s
n_full=0; for c in "${CASES[@]}"; do [ "$c" != cheap ] && n_full=$((n_full + 1)); done
count() { grep -c -- "$1" "$LOG" || true; }
if [ "$(count '^apt-get -q update APT_CONFIG=/.*/release-check/stonking/etc/apt/apt.conf$')" = "$n_full" ] \
   && [ "$(count '^apt-get -s -q dist-upgrade APT_CONFIG=/.*/release-check/stonking/etc/apt/apt.conf$')" = "$n_full" ] \
   && [ "$(count '^apt-cache policy gnome-shell .* APT_CONFIG=/.*/release-check/stonking/etc/apt/apt.conf$')" = "$n_full" ] \
   && [ "$(count '^apt-config dump APT_CONFIG=$')" = "$n_full" ] && [ "$(count '^apt-config dump APT_CONFIG=/')" = "$n_full" ] \
   && [ "$(count '^dpkg-query -S ')" = "$n_full" ] && [ "$(wc -l < "$LOG")" = $((6 * n_full)) ]; then
  ok "stub log: $n_full × (dpkg-query -S, apt-config dump without and with the tree, apt-get -q update, apt-cache policy, apt-get -s -q dist-upgrade), nothing else"
else bad "stub log:"; cat "$LOG"; fi
# strace: the programs started (successful execve) and connect()
/usr/bin/python3 -I - "$RUN/strace/trace" "$BIN" "$RUN/tools" > "$RUN/strace/summary.txt" <<'EOF' && ok "strace: $(head -1 "$RUN/strace/summary.txt")" || { bad "strace:"; cat "$RUN/strace/summary.txt"; }
import re, sys
trace, binary, tools = sys.argv[1], sys.argv[2], sys.argv[3]
allowed = {"/usr/bin/bwrap", "/usr/bin/env", "/bin/sh", "/usr/bin/sh", "/usr/bin/dash", "/bin/bash", "/usr/bin/bash", binary,
           "/usr/bin/apt-get", "/usr/bin/apt-cache", "/usr/bin/apt-config", "/usr/bin/gpgv", "/usr/bin/sqv",
           "/usr/lib/apt/methods/file", "/usr/lib/apt/methods/gpgv", "/usr/lib/apt/methods/store",
           "/usr/lib/apt/methods/copy", "/usr/bin/mkdir", "/bin/mkdir", "/usr/bin/cat", "/bin/cat"}
allowed |= {f"{tools}/{t}" for t in ("apt-get", "apt-cache", "apt-config", "dpkg-query")}
allowed |= {"/usr/bin/dpkg", "/usr/bin/ischroot"}
# apt's own read-only probes, allowed only with exactly these arguments
probes = {"/usr/bin/dpkg": '["/usr/bin/dpkg", "--print-foreign-architectures"]', "/usr/bin/ischroot": '["/usr/bin/ischroot", "-t"]'}
ran, bad, connects, wrong = {}, [], [], []
for line in open(trace, encoding="utf-8", errors="replace"):
    m = re.search(r'execve\("([^"]+)", (\[[^]]*\]), .*\) = 0', line)
    if m:
        ran[m.group(1)] = ran.get(m.group(1), 0) + 1
        if m.group(1) in probes and m.group(2) != probes[m.group(1)]:
            wrong.append(line.strip())
    m = re.search(r'(connect|sendto|sendmsg)\((.*)\) = (\d+)', line)
    if m and "sun_path" in m.group(2) or (m and "sin" in m.group(2)):
        connects.append(line.strip())
bad = sorted(p for p in ran if p not in allowed)
print(f"{len(ran)} programs started, {sum(ran.values())} execve, {len(bad)} not allowed, {len(wrong)} probes with other arguments, {len(connects)} successful connect/send with an address")
for p in sorted(ran):
    print(f"  {ran[p]:4} {p}")
for p in bad:
    print(f"  NOT ALLOWED {p}")
for c in connects[:20]:
    print(f"  CONNECT {c}")
for w in wrong[:20]:
    print(f"  PROBE {w}")
sys.exit(1 if bad or connects or wrong else 0)
EOF
s="$(sockets_in_w)"; [ -z "$s" ] && ok "no socket file in the worktree" || bad "socket files in the worktree: $s"

if ((RECORD)) && ((fail == 0)); then
  for c in "${CASES[@]}"; do
    [ "$c" = cheap ] && continue
    mkdir -p "$FIX/recorded/$c"
    for f in update.txt policy.txt simulate.txt; do
      sed "s|$RUN/archive/$c|/srv/w11-archive|g; s|$RUN/fixture-$c|/@ROOT@|g; s|$RUN/[^ ]*release-check/stonking|@TREE@|g" "$RUN/out/$c/$f" > "$FIX/recorded/$c/$f"
    done
  done
  t="$(sed -n 's/^Dir "\(.*\)\/";$/\1/p' "$RUN/out/ok/apt-config-tree.txt")"
  sed "s|$t|@TREE@|g; s|$RUN/fixture-ok|/@ROOT@|g" "$RUN/out/ok/apt-config-tree.txt" > "$FIX/recorded/apt-config-tree.txt"
  cp "$RUN/out/ok/apt-config-system.txt" "$FIX/recorded/apt-config-system.txt"
  echo "== recorded into $FIX/recorded"
fi
echo "== $pass passed, $fail failed (run: $RUN)"
((fail == 0))
