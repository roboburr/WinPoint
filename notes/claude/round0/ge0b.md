# GE0b: Rust baseline for G at 3b898e6 (2026-11-05)

G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`, branch `claude/gnome-english-update`
(HEAD 7fc86a6d = 3b898e6 + `notes/claude/` only; `git diff 3b898e6 HEAD` touches nothing outside `notes/claude/`,
so `host/` is exactly the Codex baseline). Nothing was committed. No product code was changed.

## 1. What I built

| Path | What |
|---|---|
| `tests/rust-sandbox/run-tests.sh` | sandbox runner for `cargo test` (guards below) |
| `tests/rust-sandbox/audit_tests.py` | static audit of all 256 `#[test]` functions (risky patterns per test) |
| `tests/rust-sandbox/test-audit.tsv` | its output |
| `tests/rust-sandbox/logs/` | build/test logs, `/usr/bin/time -v` files, sandbox env dumps, `guard-calls.log`, `scan-summary.json` |
| `tests/rust-sandbox/.gitignore` | ignores the per-run generated dirs (home, config, data, cache, run, guard-bin, inner.sh, bus config) |
| `tools/i18n/scan_rust.py` | Rust user-visible-literal scanner |
| `tests/i18n/rust-literals.tsv` | every literal + every embedded-table string, classified (13,845 rows) |
| `tests/i18n/rust-nl-frozen.tsv` | every Dutch user-visible text, keyed by file + sha256[:12] (1,079 rows) |
| `target/debug/win11-*` | the 12 debug binaries (gitignored) |

## 2. Reproduce

```bash
G=/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome; cd $G
grep MemAvailable /proc/meminfo                       # must be > 2 GB
flock -w 3600 $G/.heavy.lock /usr/bin/time -v -o tests/rust-sandbox/logs/build.time \
  cargo build --offline --locked -j2                  # debug, all 12 binaries
tests/rust-sandbox/run-tests.sh --lib                 # the baseline test run (see 4.1 for why --lib)
tests/rust-sandbox/run-tests.sh                       # all targets: fails to compile (4.1)
python3 -B tools/i18n/scan_rust.py --summary          # writes both TSVs, prints counts
```

## 3. Build

* `cargo fetch --offline --locked`: OK; **no crate is missing** from `~/.cargo/registry`.
* `cargo build --offline --locked -j2` (dev profile, `debug = 0`, `incremental = false`): **exit 0**, 91 crates compiled,
  **wall 42.8 s, 175 % CPU, peak RSS 1.25 GB** (largest single rustc; `/usr/bin/time -v`). 30 warnings
  (28 lib, 2 in bin `win11-launch`: unused `configure_portal_child_environment`/`restore_portal_child_environment`).
* All 12 binaries of `scripts/install.py:17` exist in `target/debug` (13:04): win11-start, -explorer, -panel, -app,
  -desktop, -default-apps, -installer, -file-portal (≈28.4 MB each), -index-worker 5.3 MB, -image-worker 3.5 MB,
  -software-worker 3.4 MB, -launch 1.7 MB. sha256 list: `sha256sum target/debug/win11-*` (see 7).
* **Finding: the "unknown" live `win11-panel` is a debug build of exactly this source.** My debug `win11-panel` is
  28,363,968 bytes with sha256 `17437f50a1bfb9dc2c8d27c2e98cb253356de3318da30214068ba2312f665e07`; extension-packaging.md
  :35/:325 records the live `~/.local/bin/win11-panel` as 28,363,968 bytes, sha `17437f50a1bf…`. The build is
  reproducible, so the live panel was built from 3b898e6 in this tree **in the debug profile** (unoptimised). I did not
  read the live file; the comparison uses the recorded size and 48-bit hash prefix. The other 11 live binaries
  (e.g. win11-start `1ee8b61d…`) do not match my debug build (mine `f86c1de9…`).

## 4. Tests

### 4.1 Results

| Run | Result |
|---|---|
| `cargo test --offline --locked -j2` (all targets) | **does not compile**: bin `win11-launch` test target, `error[E0433]: cannot find events in crate` at `host/src/launch_context/portal/tests.rs:21` (`crate::events::nonce()`; `src/bin/launch.rs` includes `launch_context.rs` by `#[path]`, and that bin crate has no `events` module). Wall 9.0 s, peak RSS 937 MB (this run also compiled the lib and the other bin test harnesses). |
| `--lib` (the baseline) | **248 passed, 0 failed, 8 ignored**, 0 filtered, 1.42 s (wall 1.8 s incl. cargo) |
| `--bin` × the 11 other binaries | compile; 0 tests each |
| doc tests | none: no code block in any doc comment (grep) |

So **every non-ignored test passes at baseline**; the only failure is the compile error of the `win11-launch` test
target, which is a Codex defect (a one-line fix in a test file; product code, so not done here). Until it is fixed,
the baseline command is `cargo test --lib`.

The 8 ignored tests (not run): `files::actions::dry_tests::native_dry_validates_but_never_spawns_at_leaf_boundaries`,
`launch_context::portal::tests::isolated_launch_environment`,
`portal::tests::portal_local_gtk_override_does_not_escape_command_or_gio_launch_context`,
`process_scope::manager_tests::actual_manager_separates_app_from_bounded_shell_slice` (uses the *real* user manager),
`services::disks::tests::private_bus_tests::native_disks_use_private_udisks_protocol`,
`services::shell::tests::private_bus_tests::native_core_uses_private_filemanager_protocol_and_events`,
`services::system::energy::tests::energy_profiles_use_real_typed_dbus_and_readback`,
`services::system::private_bus_tests::native_system_uses_private_protocols`.

### 4.2 Safety audit (before running)

I read every test module (`audit_tests.py` lists each `#[test]` with risky patterns; I then read the bodies and the
fixtures they call). Findings:
* every fixture uses its own temp root (`std::env::temp_dir()/win11-…-<nonce>`), its own `Events` socket dir and
  `ServiceContext` config dir; no test reads `~/.config/win11-gnome` or the live runtime dir;
* `NativeCore::build(.., start = false)`: the FileManager1 bus name is not claimed; shell launches go to a test closure;
* energy and wallpaper tests use `gio::memory_settings_backend_new()`/`null_settings_backend_new()` and pass `None` as bus;
* the disks guard tests run on parsed fixtures (`readonly: true`), no UDisks call;
* the only signal-related test (`protected_processes_never_reach_signal`) stops in `check_own` for pid 1 and self;
* `process_scope::environment()` is only inspected, not spawned; the cgroup test is ignored;
* the only non-test bus users reachable from tests are read-only default-backend GSettings (`files/read.rs:762`,
  `images/icons.rs:290`, icon theme lookup).

**No non-ignored test can touch the real session bus, Robin's HOME or the live extension, so no `--skip` was needed.**

Defence in depth in `run-tests.sh` anyway: `env -i` (no DISPLAY/WAYLAND_DISPLAY/AT-SPI/keyring vars); private HOME and
XDG_{CONFIG,DATA,CACHE,RUNTIME}_HOME under `tests/rust-sandbox/` (runtime 0700); `dbus-run-session` with a minimal bus
config **without service directories** (nothing can be auto-activated); `DBUS_SYSTEM_BUS_ADDRESS` set to a socket that
does not exist (an accidental system-bus call fails instead of reaching UDisks/NM/logind); `GSETTINGS_BACKEND=keyfile`,
`NO_AT_BRIDGE=1`, `GTK_A11Y=none`; stubs for pkexec, systemctl, gsettings, dconf, gnome-extensions, gnome-shell, apt,
pkcon, notify-send, xdg-open, gio, nmcli, udisksctl, all `win11-*` … first on PATH (each call logged and refused);
an inner guard script that aborts unless the bus address, runtime dir and HOME are the private ones; MemAvailable check;
flock.

Evidence after the runs: `guard-calls.log` empty (no test ran a guarded command); the private config got only an empty
`glib-2.0/settings/` dir (keyfile backend initialised by the read-only lookups, **no key written**; the dir was removed afterwards); 0 entries left in
the private TMPDIR; no leftover processes.

**One deviation from "never in /tmp":** the private TMPDIR and the private bus socket live in `/tmp/w11-ge0b-<pid>`
(0700, removed by the script's EXIT trap). Reason: `Events::new` binds Unix sockets under `std::env::temp_dir()`, and
the repo path makes them longer than sockaddr_un's 107 bytes (dbus-daemon failed with "Socket name too long" on the
first try). Product code does the same for long runtime dirs (`files/mod.rs:435-450`, fallback `/tmp/win11-events-…`).
Nothing in it outlives a run.

## 5. The three oracles (`host/src/services/{system,shell,disks}/reference-tests.json`)

| Oracle | Cases | Tests using it | Dutch it pins | Result |
|---|---|---|---|---|
| system (80 KB) | 475 (number/pct/bool coercion, Wi-Fi security, psk, nmcli split, display modes, BT kind/error keys, player dict) | `services::system::reference_contracts::retained_python_oracle_contracts_match_native_helpers` | 48 error messages, 1 unique: "Ongeldige waarde." | **pass** |
| shell (885 KB) | 1,081 in 13 sections (commands 207, policy 660, explorer 76, backlog 41 …) | all 8 in `services/shell/tests/oracle_tests.rs` | 112 messages, 4 unique ("Ongeldige invoer.", "Onbekende opdracht.", "Te veel gegevens.", "Dit item staat niet meer in het klembord.") | **pass** |
| disks (1.5 MB) | 2,876 in 7 sections (guards 2,284, labels 264, models 144, gaps 120, smart 31, errors 20, efi 13) | 7 in `services/disks/tests.rs` | 1,586 messages, 172 unique, **86 nl + 86 en** (labels, errors, guards in both languages) | **pass** |

All 16 oracle tests pass. As a Dutch guard they are narrow: of the 1,079 frozen Dutch texts only **80** occur verbatim in
an oracle (+8 as a template prefix), 58 of them disks `_STR` texts. The frozen file (section 6) is the real nl guard;
the oracles stay useful to keep the disks catalogue and the generic error messages byte-identical.

## 6. Rust literal scanner

`tools/i18n/scan_rust.py` (stdlib Python, ~1,450 lines, about 3 s). It lexes Rust itself (raw strings, byte strings,
escapes incl. line continuations, nested comments, char vs lifetime), skips test code (files reached through
`#[cfg(test)] mod x;` and their children: 9 files; plus 78 `#[cfg(test)]`/`#[test]` items, including fields and
blocks; sanity check: all 256 `#[test]` lie in excluded code), and tracks for each literal: the enclosing call chain
(`format!#0 < ApiError::new#1 < Err#0 < fn x`), json!/struct key, `let`/`const` binding, enclosing fn, match-pattern,
comparison, index and string-op roles, and language branches (`if lang == "en" {…} else {…}`, `match (lang, n)`,
helpers whose parameters are named `nl`/`en`: `t`, software `err`, `blocked`; `{"nl":..,"en":..}`; adjacent nl/en
tuple elements such as `own_names`, places). It also reads the embedded tables: disks/storage/sysmon `_STR`, system
`strings.json` (+ Wi-Fi pairs), `_FRIENDLY`, `_GSD_NL`, `_GVFS_NL`, `apps/catalog-data.json`, `files/types.json`,
`files/launcher-names.json`. Language detection: `tests/i18n/dutch-words.txt` (sha256 `c7858ca535ee`, byte-identical to
H's), a small reviewed extra set, `/usr/share/dict/{american,british}-english`, and, for words neither list knows,
Dutch morphology and compound matching ("instellingendienst", "lettertypebestand"); camelCase tokens and words with
digits never count as Dutch.

Columns of `rust-literals.tsv`: file, line, col, id (`file#sha256[:12]#n`), source (str/raw/bytes/json), class, lang
(nl/en/mixed/none), side (pair half), context, key, reason, text. `rust-nl-frozen.tsv`: file, hash, count, class
(`user-visible-dutch`, `pair-nl`, `dutch-file-marker`), text; sorted, no line numbers. **Reproducibility:** two runs
from different working directories and with `PYTHONHASHSEED` 1, 777 and random give byte-identical files
(`rust-literals.tsv` sha256 `88162279…`, `rust-nl-frozen.tsv` `fc7e656b…`).

### 6.1 Counts (non-test code)

| | Rust literals (12,454) | JSON table strings (1,391) |
|---|---|---|
| technical | 11,558 | 420 |
| user-visible-dutch | 530 sites (424 unique texts) | 181 (Dutch-only overrides and fragments) |
| bilingual-pair | 210 (104 nl, 106 en halves) | 790 (395 nl + 395 en) |
| user-visible-english | 98 | – |
| log | 58 (16 Dutch) | – |

Frozen: 1,079 rows / 1,213 occurrences = 615 user-visible-dutch, 457 pair-nl, 4 both, 3 Dutch file markers (the two
gtk.css caption/theme markers in `settings_effects.rs:17/20` and the `monitors.conf` header comment in
`services/display.rs:387`, which must stay byte-identical).

Dutch-only sites by area: files 132, services/sysmon 70 (30 of them the `unit_friendly` systemd names), services/index 56,
services/system 41, services/storage 31, apps 28, services/tray 20, backend.rs 18, runtime.rs 17, process_scope 15,
settings.rs 15, picker 14, transfers 13, gnome 10, services/shell 9, services/display 8, wallpaper 7, notifications 6,
media 5, bin 5, launch 3, lib 3, events/mod/settings_effects/settings_runtime 1 each. Software has 0 Dutch-only sites
(all paired).

### 6.2 Against gnome-map §2.1

| Figure | gnome-map / report | Scanner | Why they differ |
|---|---|---|---|
| services scope Dutch-only (services/** + settings*, runtime, backend, events) | ≈232 UI-visible + ≈42 internal + 25 unit names = ≈299 | 300 = 242 + 28 internal (backend 18, index/cache 9, events 1) + 30 `unit_friendly` | same total; the split differs (the report counted some runtime/settings start-up texts as internal; `unit_friendly` has 30 Dutch literals for 25 units) |
| unique in that scope | ≈186 (UI-visible only) | 248 (all 300) | different base |
| rest of host Dutch-only | ≈231 (files 133, apps 28, picker 14, process_scope 15, transfers 13, gnome 10, wallpaper 7, launch/bin/lib 11) | 230 (132, 28, 14, 15, 13, 10, 7, 11) | files −1, within the reports' stated ±5 %; I did not trace which site |
| inline pairs, rest | 23 | 23 | – |
| inline pairs, services | ≈63 software + ≈10 ternaries/tuples + 3 notification | 81 nl / 83 en | probably the report's own caveat: its pair detector undercounted pairs in long one-line `json!` expressions |
| `_STR` keys disks/storage/sysmon/system, Wi-Fi, `_FRIENDLY`, MIME, own apps | 104/50/63/33, 12, 63, 47, 7×3 + twin | same (2 × 395 table halves) | – |
| English user-visible | 11 window titles (rest); EN-UI "–" for services | rest 29 (11 titles + portal/window_state errors), services 69 (software's English internal errors, which the report put under "details") | the report counted titles only; my class is an upper bound (any capitalised English sentence that can be returned) |
| Dutch logs | 7 (services) | 7 services, 9 rest | rest was not counted |
| `ApiError::new` sites non-test | 251 (§7.1 check 6, "Held") | **260** | grep gives 261 in all; 1 is in a test file. §7.1 cut each file at its first `#[cfg(test)]`, which also drops real code after `#[cfg(test)] mod tests;` in `services/disks.rs:7` (3 sites), `services/shell.rs:6` (6) and a field in `files/watch.rs:31` (1). **Check 6 did not hold.** |
| `ApiError::new` with a literal Dutch message | 203 (port-path) | 210 direct + 6 through `format!` | the scanner also normalises `files::ApiError::new` / `crate::files::ApiError::new` (12 sites) |
| literals with a Dutch word | ≈630 (port-path) | 729 | morphology/compound detection, pair nl halves and Dutch input words (patterns) are included |
| JSON Dutch-only | not counted | 181: nlNames 57, NL_NAMES 57 (same list twice), nlComments 42, `_GSD_NL` 18, `_GVFS_NL` 5, plus `FS_NAMES.swap` "Wisselgeheugen" and `_SAFE_CACHES` "Lettertypen" | two Dutch labels hide in otherwise technical tables |

### 6.3 Things the scan surfaced

* `services/index/query.rs:58-68` English regex error texts ("unterminated character set", "nothing to repeat" …) are
  inserted into a Dutch message at `:75` ("Deze reguliere expressie klopt niet: {msg}."), so Dutch users already see
  mixed text; `:39` has "…: embedded null byte." hard-coded.
* `files/actions.rs` test `malformed_uri_authority_is_rejected_before_any_handler_launch` asserts the Dutch "Deze link
  is ongeldig." for an **en** context: such tests pin Dutch-in-English and will need updating in round 2 (by design).
* `"Melding"` (notifications.rs:361), "alleen in de Win11-sessie" (display.rs:86) and "…Hyprland en hyprsunset nodig"
  (system/power.rs:154) are in the frozen file; changing them (D-G2) needs BASELINE-CHANGES rows keyed by file + hash.

### 6.4 Limits

Heuristic: classes come from context + word lists, so the `reason` column matters. Known soft spots: single
capitalised English words become technical unless a display context is proven (window titles, `"name"`/`"title"`
keys, message parameters); `user-visible-english` is an upper bound; Dutch input words in patterns/comparisons are
technical (reason says so). The frozen file must be regenerated only from a 3b898e6 tree with the same
`dutch-words.txt` (hash in `logs/scan-summary.json`), or it is no longer a baseline.

## 7. Not done

* No release build (`target/release`), no `cargo check`/clippy run separately (the build covers check).
* Ignored tests not run; no doc tests exist.
* No fix of the `win11-launch` test-target compile error (product code).
* No frozen-check tool yet (round 2: a checker that every frozen (file, hash, text) still exists as a literal or as a
  TSV nl value).
* I did not read the live `~/.local/bin/win11-panel`; the match in section 3 uses the recorded size and hash prefix.

## 8. For the coordinator

1. **`cargo test` (all targets) is broken at baseline**; use `cargo test --lib` (or `tests/rust-sandbox/run-tests.sh
   --lib`) until the one-line test fix (`launch_context/portal/tests.rs:21`, e.g. a local nonce or
   `#[cfg(test)]`-gated path) lands in round 1. Decide who owns it.
2. **gnome-map §7.1 check 6 is wrong** (251 → 260 non-test `ApiError::new` sites); update §2.1/§7.1.
3. **gnome-map §5 "This PC" / §6 item 9:** the live `win11-panel` is a debug build of 3b898e6 from this tree, not an
   unknown binary; the final deploy should replace it with a release build.
4. **Concurrent changes seen in G during my run (not mine, left untouched):** `vendor/ui/js/mock/index.js` (modified),
   `vendor/ui/js/mock/gnome.js`, `tests/i18n/*` except my two TSVs, `tools/i18n/*` except `scan_rust.py`,
   `notes/claude/plan-critique.md`, `notes/claude/windows-update-gnome.md`, `notes/claude/round0/ge0a-dump-runs.log`.
   They match GE0a/GE0d/GE0e; git status was clean when I started.
5. `scan_rust.py` reads `tests/i18n/dutch-words.txt`, which GE0a placed (identical to H's). Keep that file frozen, or
   pin the frozen TSV to the hash in `logs/scan-summary.json`.
6. Commit `tests/rust-sandbox/` (scripts, audit, logs) and both TSVs; the per-run dirs are gitignored.
7. Accept or reject the `/tmp/w11-ge0b-<pid>` exception (section 4.2); the alternative is a shorter repo path.
