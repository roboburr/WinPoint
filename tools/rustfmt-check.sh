#!/usr/bin/env bash
# rustfmt check of the host crate (round 2 "Round 3 picks up"; R3-NATIVE).
#
#   tools/rustfmt-check.sh            exit 0: every host .rs file is rustfmt-clean, except the files listed below
#   tools/rustfmt-check.sh --list     also print every file rustfmt would change
#
# Runs `rustfmt --edition 2024 --check` on each crate root (host/src/lib.rs and every host/src/bin/*.rs; rustfmt
# follows their `mod` declarations, so every module file is covered) and changes nothing. A file with a diff fails
# the check unless it is in UNFORMATTED: the files that were not rustfmt-clean at 18c8ffbe (the round-3 start),
# minus runtime.rs, window.rs and i18n.rs, which R3-NATIVE formatted, and picker.rs, formatted when the WebKit picker
# went (window.rs went with it). So a file that is clean today stays clean, and a new file must be clean. A listed
# file that has become clean is named, so it can leave the list. Cheap: no cargo, no build (rustfmt only parses);
# about 2 s.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
RUSTFMT="${RUSTFMT:-$(command -v rustfmt || echo "$HOME/.cargo/bin/rustfmt")}"
[[ -x $RUSTFMT ]] || { echo "rustfmt-check: rustfmt not found" >&2; exit 2; }
LIST=0
[[ ${1:-} == --list ]] && LIST=1

# Not rustfmt-clean at 18c8ffbe; leave them to their owners (formatting only, in a step that owns the file).
UNFORMATTED='
host/src/apps/catalog.rs
host/src/apps/defaults.rs
host/src/apps/mod.rs
host/src/apps/pertype.rs
host/src/apps/recent.rs
host/src/bin/launch.rs
host/src/desktop_entries.rs
host/src/events.rs
host/src/gnome.rs
host/src/launch.rs
host/src/launch_context/portal/tests.rs
host/src/process_scope.rs
host/src/services/mod.rs
host/src/services/system/info.rs
host/src/services/system/power.rs
host/src/services/update.rs
host/src/services/update/bus_tests.rs
host/src/services/update/guard.rs
host/src/services/update/history.rs
host/src/services/update/model.rs
host/src/services/update/net.rs
host/src/services/update/options.rs
host/src/services/update/paths.rs
host/src/services/update/phasing.rs
host/src/services/update/pk.rs
host/src/services/update/release.rs
host/src/services/update/restart.rs
host/src/services/update/store.rs
host/src/services/update/tests.rs
host/src/transfers.rs
'

cd "$ROOT"
out=$(mktemp)
trap 'rm -f "$out"' EXIT
status=0
for crate_root in host/src/lib.rs host/src/bin/*.rs; do
  # exit 1 means "would reformat" (the Diff lines say where); anything printed without a Diff line is an error
  "$RUSTFMT" --edition 2024 --check "$crate_root" >> "$out" 2>&1 || true
done
if grep -qvE '^(Diff in .*:[0-9]+:$|[ +-].*|$)' "$out"; then
  echo "rustfmt-check: rustfmt reported an error:" >&2
  grep -vE '^(Diff in .*:[0-9]+:$|[ +-].*|$)' "$out" | head -5 >&2
  exit 2
fi
mapfile -t changed < <(sed -n 's/^Diff in \(.*\):[0-9][0-9]*:$/\1/p' "$out" | while IFS= read -r f; do
  realpath --relative-to="$ROOT" -- "$f"; done | sort -u)
listed=0 bad=0
for f in "${changed[@]}"; do
  if grep -qxF -- "$f" <<<"$UNFORMATTED"; then
    listed=$((listed + 1))
    [[ $LIST == 1 ]] && echo "unformatted (listed): $f"
  else
    bad=$((bad + 1))
    echo "rustfmt-check: FAIL $f is not rustfmt-clean (rustfmt --edition 2024 $f, formatting only)"
    status=1
  fi
done
total=$(find host -name '*.rs' | wc -l)
while IFS= read -r f; do
  [[ -z $f ]] && continue
  if [[ ! -e $f ]]; then
    echo "rustfmt-check: note: listed $f no longer exists"
  elif ! printf '%s\n' "${changed[@]}" | grep -qxF -- "$f"; then
    echo "rustfmt-check: note: listed $f is clean now; it can leave UNFORMATTED"
  fi
done <<<"$UNFORMATTED"
echo "rustfmt-check: $total host .rs files; $((total - ${#changed[@]})) clean, $listed listed as not formatted yet, $bad failing"
exit $status
