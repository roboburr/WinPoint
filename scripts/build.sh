#!/usr/bin/env bash
# Developer build. Messages in Dutch, or English with --lang en (or WIN11_LANG=en).
set -euo pipefail
ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
MSG_LANG="${WIN11_LANG:-nl}"
while [ $# -gt 0 ]; do
    case "$1" in
        --lang) MSG_LANG="${2:-}"; shift 2 || shift ;;
        --lang=*) MSG_LANG="${1#--lang=}"; shift ;;
        *) shift ;;
    esac
done
# i18n: pairs (tools/i18n/check_pairs.py): pair NL EN prints the text of the chosen language.
pair() { if [ "$MSG_LANG" = en ]; then printf '%s\n' "$2"; else printf '%s\n' "$1"; fi; }
if ! command -v cargo >/dev/null || ! command -v pkg-config >/dev/null; then
    pair 'Rust/Cargo en pkg-config zijn vereist.' 'Rust/Cargo and pkg-config are required.' >&2
    exit 1
fi
if ! pkg-config --exists 'glib-2.0 >= 2.70' gio-2.0 gdk-pixbuf-2.0 'gtk4 >= 4.22'; then
    pair 'Bouwpakketten ontbreken. Op Ubuntu: sudo apt install build-essential pkg-config libglib2.0-dev libgdk-pixbuf-2.0-dev libgtk-4-dev' \
         'Build packages are missing. On Ubuntu: sudo apt install build-essential pkg-config libglib2.0-dev libgdk-pixbuf-2.0-dev libgtk-4-dev' >&2
    exit 1
fi
# Offline, locked, two jobs by default (notes/claude/contracts.md C8, plan-critique F34): no download, and a
# release build with thin LTO stays inside the memory plan. WIN11_BUILD_JOBS still overrides the job count.
# The root workspace (host/): the host's 10 binaries, i.e. the HTTP backend, file portal, workers and launchers.
cargo build --release --offline --locked -j "${WIN11_BUILD_JOBS:-2}"
# D-G70: the native GTK 4 shell, Calendar and apps (native/, its own workspace: gtk4-rs needs another glib than the
# host; native/APPS.md), into the same target directory, so target/release holds all 19 binaries scripts/install.py
# installs: the host's 10, win11-shell and win11-calendar, and the seven win11-gtk-<kind> apps (10 + 2 + 7).
cargo build --release --offline --locked -j "${WIN11_BUILD_JOBS:-2}" --manifest-path native/Cargo.toml \
    --target-dir "${CARGO_TARGET_DIR:-$ROOT/target}"
