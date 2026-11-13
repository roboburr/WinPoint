# GE1b: Rust i18n core, the round-2 gate and the core pilot (2026-11-05)

G = this repo, branch `claude/gnome-english-update` (HEAD b431c627). Nothing committed. No win11-* binary,
gnome-shell, installer or gsettings/dconf ran outside the sandboxes. Every cargo run went through `tools/heavy.sh`
(20 runs in `tests/logs/heavy-events.log`, tags `ge1b-*`; the 2 with rc 101 are a wrong expectation in my own new test, since fixed, and the deliberate mutation check).

The round-2 procedure and the frozen API are in `notes/claude/round2-gate.md`.

## 1. Result

**Gate.** Every item on the F8 list is delivered:
* the API;
* 24 stem files plus the include table;
* the converter, the checker and the drift guard;
* the `core` pilot, plus `window` (C4).

**Contracts.** C4, C5 and C6 are done on the host side. The `win11-launch` test-target compile error is fixed.

**Checks:**

| Check | Result |
|---|---|
| `cargo test` (all targets) | green: lib 269 passed / 8 ignored; `win11-launch` 5 passed / 1 ignored |
| Oracles | 16 of 16 green |
| `drift_rust.py` | 0 errors |
| `check_rust.py` | 0 errors, 0 warnings |
| Binaries | 12 debug binaries in `tests/build/r1-debug/` (with `SHA256SUMS`), byte-identical to the final `target/debug` |

## 2. Changed files (all inside my allowlist)

| File | What |
|---|---|
| `host/src/i18n.rs` (new) | The C1-C3 runtime (round2-gate.md §2). `Lang`, `AsLang` (Lang or "nl"/"en"). `current_language()` / `language_in(dir)`: one `stat`, parse only when (dev, ino, size, mtime, ctime) changes; a file written in the last 50 ms is re-read once (coarse fs clocks). `tr`/`trf`/`trn` (`{name}`, `.zero`/`.one`/`.other`, n == 1), `NO_PARAMS`, `decimal`/`number`/`join`/`join_or`/`pick`. Fallback lang → nl → key, logged once. `STEMS` embeds all 24 catalogues with `include_str!`. 10 unit tests, incl. a generic test that renders every `#@`-mapped row with sample values and compares with the 3b898e6 literal (F12). No `crate::` imports, so a bin crate can include it by `#[path]`. |
| `host/i18n/*.tsv` (24 new) | Header in every stem file. `core.tsv`: 14 rows / 16 site mappings. `window.tsv`: 13 rows (C4), 1 nl mapping, 7 English-half mappings, 12 `#@ new`. The other 22 have a header only. |
| `host/src/files/mod.rs` (ApiError block only) | `key`, `params` fields. `ApiError::tr(code, lang, key, params)`, `ApiError::trn(…, n, …)`. `json()` adds key + params (params always an object) only for catalogue errors (C2). `ApiError::new` and its JSON are unchanged. |
| `host/src/runtime.rs` | The pilot, 15 sites converted by `extract_rs.py`. The 403/405/400/411/413/502 envelopes were first rewritten by hand as `failure(ApiError::new(…))` (same JSON, literals byte-identical) so the converter could take them; now they carry key/params. `unknown(lang, method)`. Tests `runtime::i18n_tests` (nl = 3b898e6 bytes, en, envelope shapes). |
| `host/src/services/mod.rs` | `ctx.lang()`, and `ctx.language()` on top of it: a cached stat instead of a settings snapshot under the lock. "De dienst wordt gestart." → `core.starting`. Cost test. |
| `host/src/window.rs` | C4: `title_key`/`title` per kind from `window.tsv`. Picker title per mode (folder/save/open, as picker.rs and the UI), and a portal caller's own `title` wins. Registry of host-titled windows; `language_changed(lang)` re-applies and writes a `title` dev-report line. 3 tests. |
| `host/src/lib.rs` | `mod i18n; mod desktop_entries;`. `watch_language()`: observes `settings.changed` (the SettingsRuntime's 150 ms poll, other hosts' invalidation datagrams, own writes), acts only on a language change, then `glib::idle_add_once` → titles on the GTK thread, and in the panel a C6 rewrite. `pub fn enable_desktop_entries()`. |
| `host/src/desktop_entries.rs` (new) | C6 writer. Reads `host/src/desktop-entries.json` (`include_str!`). In `$XDG_DATA_HOME/applications` it rewrites only `Name=`/`Comment=` of `[Desktop Entry]` in the 10 own files. It requires a regular file owned by the user, ≤ 64 KiB, UTF-8, and a matching `StartupWMClass`. Temp file + rename, mode kept, written only when different. Debounced (start: immediately; switch: 1 s; newest wins). 4 tests on a temp dir: install.py's exact 3b898e6 bytes, nl→en→nl byte-identical, foreign/symlink/missing/other groups/CRLF untouched, escaping. |
| `host/src/bin/panel.rs` | Calls `enable_desktop_entries()` before `run("panel")`, so the panel is the single writer (F3). |
| `host/src/services/notifications.rs` | C5: an empty appName is stored as `""`. `LEGACY_APP_NAME` "Melding" maps to `""` in `add()` and in the extension's `notifications.list` items. The literal stays, so there is no drift. 1 test. |
| `host/src/launch_context/portal/tests.rs` | A local `nonce()` instead of `crate::events::nonce()` (the bin crate has no `events`). |
| `tools/i18n/drift_rust.py` (new) | A Rust lexer with offsets, test-code exclusion, fn/item/macro/call context, JSON tables, `--freeze` (from git 3b898e6), the check, `--propose`, `--paths`. It is also the shared library of the two other tools. |
| `tools/i18n/extract_rs.py` (new) | The converter: `list`, `convert`, `add [--from]`, `map [--en]`, `en`. Refuses format specs, `{{word}}`, allow-listed and technical sites; `--expect` and `L<line>` are guards. |
| `tools/i18n/check_rust.py` (new) | Rules: table, header, row, map, ph, plural, key, params, label (UI en dictionaries via GE0a's `corpus.py`/`jsdicts.mjs`), allow; warnings for glossary, unused, space. |
| `tests/i18n/rust-frozen-all.tsv` (new) | 13,450 entries: 12,454 Rust literals and 996 JSON values (7 tables, English halves left out). sha256 `e72ad6ad75966c02…`. |
| `tests/i18n/BASELINE-CHANGES.rust` (new) | 12 `key:` lines for the new Dutch window titles. 3 English title changes (`Everything`→`Alles`, `App installer`→`App Installer`, `File selection`→mode titles). 33 technical literals removed by the pilot refactor (json! keys, `"language"`/`"en"`/`"nl"`). |
| `tests/i18n/allow-host.txt` (new) | C9 rules: logs and asserts, start-up errors, HTTP reason phrases, CSS/monitors.conf markers, query input words, daemon matchers, unit ids, XDG folder names, api-methods.json, plus the `label:` kind. |
| `tests/build/r1-debug/` | The 12 binaries + `SHA256SUMS` (230 MB, ignored via `.git/info/exclude`) |
| `target/` | builds; `target/ge1b-heavy-wrapper.lock` (see deviation 1) |
| `notes/claude/round2-gate.md`, this file | |

## 3. Verification (commands from G; numbers from the final runs)

| What | Command | Result |
|---|---|---|
| Frozen file | `python3 -B tools/i18n/drift_rust.py --freeze` | 13,450 rows (12,454 Rust + 996 JSON). Two runs give the same sha. My lexer's per-file literal counts equal GE0b's `scan_rust.py` in every file (0 differences; 9 test files, 112 test items skipped). Every Rust row got GE0b's class by line:col. |
| Drift | `python3 -B tools/i18n/drift_rust.py --summary --warnings` | 0 errors, 0 warnings. 13,390 present, 60 mapped or listed (runtime 44, services/mod 5, window 11). |
| Catalogues | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 27 rows; 0 errors, 0 warnings. One glossary warning (core.windowNotResponding) was fixed with `extract_rs.py en`. |
| Mutation | core.tsv nl changed by one character, then restored | drift: 2 errors; `every_converted_site_renders_its_3b898e6_dutch`: FAILED with the diff; after the restore both are clean (sha unchanged) |
| Converter | `extract_rs.py list host/src/runtime.rs`, then `convert … --apply` | 15 sites in runtime.rs and 1 in services/mod.rs; H's en reused for 7 rows (9 sites, exact nl match); a synthetic file covers all 9 patterns plus refusals of `{:?}`, `{n:>5}` and `{{x}}` |
| Check | `tools/heavy.sh --tag ge1b-check -- cargo check --offline --locked -j2 --all-targets` | 0 errors. Warnings: lib 28, win11-launch 2, the baseline counts; all 4 in my files are pre-existing (runtime.rs:5, :10, :114; files/mod.rs native_watch). |
| Tests | `WIN11_HEAVY_LOCK=$G/target/ge1b-heavy-wrapper.lock tools/heavy.sh --min-mb 2048 --tag ge1b-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib 269 passed (baseline 248 + 21 new), 8 ignored. win11-launch 5 passed, 1 ignored (it did not compile at baseline). 11 other bins: 0 tests. Oracles 16/16 (shell 8, disks 7, system 1). guard-calls.log 0 bytes, 0 temp entries left. 15.0 s wall, 928 MB peak RSS. |
| Cost of the language (task 7) | `… run-tests.sh --lib -- language_is_a_cached --nocapture` | `ctx.lang()` 2.334 µs per call against `ctx.settings()` (the 3b898e6 `language()`) 30.481 µs, 10,000 calls each, debug build. No parse of an unchanged file. No call site reads the language per item: the sysmon process list once per refresh, disks/storage once per call or loop header (grep audit of all 31 sites). |
| Build | `tools/heavy.sh --tag ge1b-build -- cargo build --offline --locked -j2` | rc 0. 12 binaries copied; `diff` of the sha lists is empty; the embedded window.tsv is present in win11-panel (`strings`). |

**Not verified:** no run in the private GNOME Shell. GE1h adds those scenarios, and while I worked GE1a/GE1c were
mid-edit in vendor/ui, extension/ and scripts/, which a harness run installs. So the live title switch (C4) and the
GNOME names after a switch (C6) are unit-tested only.

## 4. Deviations

1. **C8: the test runs held the machine lock through run-tests.sh, not through heavy.sh.**
   * `run-tests.sh` takes `$G/.heavy.lock` itself. `heavy.sh` holds the same lock on another open file
     description, so `heavy.sh -- run-tests.sh` would wait 3600 s for itself.
   * I ran `WIN11_HEAVY_LOCK=$G/target/ge1b-heavy-wrapper.lock heavy.sh … -- run-tests.sh`. heavy.sh supplies the
     watchdog, the process group and nice/ionice; run-tests.sh takes the real lock.
   * The memory check therefore ran before the lock wait. I probed the real lock with `flock -n` before most
     runs. cargo check/build used heavy.sh with the real lock.
   * In a round-2 worktree the problem disappears, because run-tests.sh locks the worktree's own file. See
     round2-gate §6.
2. **`ctx.language()` is reimplemented** (C1 says so).
   * Results are equal for every input: missing file, object form, unknown string → nl.
   * One side effect is lost on this path: a broken settings.json used to be renamed to `settings.broken-*` by the
     snapshot. `ctx.settings()` and the settings service still do that.
3. **C2 envelope JSON** of the HTTP errors (403/405/400/411/413/502) and `unknown_method` now also carries `key`
   and `params`. `code` and `message` are byte-identical in Dutch.
4. **Freezing every literal includes technical ones.** A refactor that removes `"ok"`/`"code"` json keys needs
   BASELINE-CHANGES lines: 33 for this pilot. `--propose` writes them. Contract-conform (F12) but noisy.

## 5. Left / for others

* **GE1a:** render `appName ""` as "Melding"/"Notification" (C5). Do not send `setTitle` in native windows (C4).
  The host still honours the existing `setTitle` IPC; see decision 3.
* **Round 2:** everything in round2-gate.md, including:
  * `files::Context::language`, a full settings read per files.* call (R2-C);
  * the caches that are not keyed by language (R2-B);
  * the 4 `_STR` tables stay (D-G8).
* **Review stage:** harness scenarios for C4 (Meta window titles before and after a switch, incl. the hidden Start
  and an open picker) and C6 (entry contents and `Shell.AppSystem` names after a switch, and after a panel
  restart).
* lib.rs/runtime.rs start-up errors stay Dutch on stderr (D-G15, allow-listed).
* An incident, fixed and guarded:
  * My `lang()` edit shifted the ordinals in services/mod.rs, and one `convert --apply` hit `"software."`.
  * I reverted it by hand; drift is clean.
  * The converter now refuses technical literals, and has `--expect` and `L<line>`.

## 6. Decisions for the coordinator

1. **Nested lock:** let `run-tests.sh` skip its `flock` when heavy.sh holds the lock (e.g. heavy.sh exports
   `WIN11_HEAVY_HELD=1`), or keep my wrapper-lock recipe (round2-gate §6). GE0b's file is frozen, so the fix is
   yours to assign.
2. **Window titles** (BASELINE-CHANGES `key:` lines; Robin sees them):
   * nl panel/desktop "Win11 Taakbalk"/"Win11 Bureaublad", as their desktop entries;
   * installer = the frame title "App-installatieprogramma" / "App Installer", not the catalogue's "App-installatie";
   * picker per mode "Openen"/"Opslaan als"/"Map kiezen" (en "Open"/"Save as"/"Choose a folder"), and a portal
     caller's own title wins;
   * Alles stays "Alles" (D-G5).
3. **`setTitle` IPC** (window.rs): keep it (my default, no behaviour change), or ignore it for host-titled kinds so
   a stray UI call cannot override C4 until the next switch.
4. **C6 in dry-run** (`--dry-run`/`WIN11SHELL_DRY_RUN`): the panel does not rewrite the entries (my default). No
   `update-desktop-database` call (Name/Comment are not in mimeinfo.cache).
5. **Drift strictness:** keep "every literal" (my default, F12), or exempt GE0b's `technical` class from BASELINE
   lines. That is less noise, but a misclassified Dutch text would slip through.
6. **Round-2 worktrees:** run-tests.sh's `env -i` drops `CARGO_TARGET_DIR`. Each worktree builds its own `target/`
   (about 1 GB), so check the disk budget (F35).
7. **Stem ownership for unlisted files:** process_scope → `launch`, portal → `picker`. lib.rs, events.rs, backend,
   images, window_state, app_params and bin/app.rs stay unconverted in round 2.

## 7. Fix round (2026-11-05, after `notes/claude/round1/ge1b-review.md`)

The review found no blocker, 2 majors and 10 minors. Both majors and 8 of the 10 minors are fixed; m8 is Robin's
decision and m9 is measured, not changed. Nothing committed. No win11-* binary, gnome-shell, installer or
gsettings/dconf ran outside the sandboxes. Before I started, all my files were as GE1b left them (mtimes before
15:29, sha list in my scratchpad). At the end, only the files listed below had changed.

### 7.1 Findings and what I did

| # | Finding | Fix |
|---|---|---|
| **M1** | No Rust ratchet: a new inline Dutch literal passed every check | **The ratchet in `drift_rust.py`.** It runs by default (`--new` lists only, `--no-ratchet` skips). It takes every working-tree literal beyond the 3b898e6 multiset of its file (a new file counts as new as a whole; the alignment picks which equal copy is new) and classifies it with GE0b's `scan_rust.py` on the working tree. **Error:** a `user-visible-dutch` literal, or the Dutch half of a new inline pair. **Warning:** a new English-only user text, or Dutch-looking "technical" text (a word of 4 letters or more). Catalogue keys and `allow-host*.txt` rules are exempt. `extract_rs.py list`/`convert` now classify a literal added since 3b898e6 the same way (`new:<class>`), so `convert` refuses a new technical literal such as `"appName"`. round2-gate §1/§6 updated. |
| **M2** | Nothing checked for Dutch in the English column | **Rule `[en-dutch]` in `check_rust.py`**, an error. It covers every catalogue row and `host/src/desktop-entries.json`. It fires when an en word is in `tests/i18n/dutch-words.txt` (the list was built to hold no English word), or when en is a copy of a Dutch nl. A warning reports en words that only `scan_rust.Words` calls Dutch (its extra words, Dutch word shapes). New allow kind `en:<text>` for a name that must stay Dutch ("Alles" is not in the list, so it needs none). |
| m1 | `LEGACY_APP_NAME` was offered for conversion | `allow-host.txt`: `host/src/services/notifications.rs	item:LEGACY_APP_NAME	matcher …`. `extract_rs.py list` now hides it (`--all` shows `keep (…)`). |
| m2 | C5 mapped the extension's list but not its events, so the toast said "Melding" and the centre "Notification" for the same source | The reviewer's option (b). The host passes the extension's `notifications.list` on unmapped, as the relay already does for `notifications.new`; the mapping stays only in `add()`, the host's own store. In GNOME (part B): the "Melding" source shows "Melding" in its toast **and** in the centre, and a nameless notification shows "Notification" in both. **C5 wording change needed (decision 1).** |
| m3 | A picker opened with the caller's own title kept it (Dutch) after a switch | `window::title`: the caller's own title wins only with `portal: true` (portal/model.rs sets it for another app through the file portal). Win11's own `shell.pickFile` callers get the mode title. Unit test extended. GNOME (part D): both Settings pickers are Openen → Open → Openen. No new Dutch key: in-app pickers now show the listed `window.title.picker*` (3b898e6 showed "File selection"); the BASELINE-CHANGES reason of `window.rs#42` says so. |
| m4 | The label rule failed open without node | Now an error ("the UI label check could not run …") unless `--no-labels` is given, which is a warning. |
| m5 | `watch_language` applied the event's language, so two switches dispatched out of order could leave the old language | `lib.rs`: a `settings.changed` event only queues one idle callback on the GTK thread. That callback reads `ctx.lang()` (the file) when it applies the language, keeps the applied value on the main thread, and re-titles only on a real change. `desktop_entries::schedule(delay, language)` now takes a closure that the worker calls when it runs. New test `a_scheduled_rewrite_uses_the_language_at_run_time_and_only_the_newest_one_runs`. GNOME (part C-double): en and nl renames 30 ms apart; titles end Dutch, and the entries are byte-identical to the install. |
| m6 | A settings file with a future mtime was parsed on every call | `i18n::trusted_at` now tests ctime, which the kernel sets on every change and no writer can choose, with a symmetric 50 ms window. A file stays untrusted only within 50 ms of its last change, in the past or the future. New test `a_parse_is_trusted_only_outside_the_racy_window_of_the_last_change` (8 synthetic stamps). The cache test now runs for an mtime 5 s in the past and 1 h in the future (under 100 parses per 1000 lookups). |
| m7 | `setTitle` IPC still honoured for host-titled windows | `window.rs`: ignored for every window in `TITLED` (every window `build` makes). Logged once per process (`Win11 settings: setTitle genegeerd (ignored): …`, seen in `app-settings.log`) plus a `title-ignored` dev report. GNOME (part A): the stray title never appeared and Settings stayed "Instellingen". |
| m8 | Grid names vs window titles differ ("Win11 Schijfbeheer" vs "Schijven") | **Not changed.** It is a Dutch change and predates GE1b, so it is Robin's decision. Also, `desktop-entries.json` is not in this stage's allowlist. |
| m9 | Process-list cost not measured | **Measured.** `tests/gnome/scenarios/ge1b-perf.py` (new) times 40 sequential `sysmon.processes` calls from the panel page after 5 warm-ups, with `settings.get` as a reference. I ran it with the 3b898e6 binaries and with GE1b's, same UI (worktree). Result in 7.2: no difference within the 1 ms timer resolution. R2-B reruns it (round2-gate §6). |
| m10 | The converter's default language hid mistakes | `extract_rs.py convert` warns when a site has no `@LANG` while its fn has `self.ctx` (a `&self` method in a file that uses `self.ctx`), `ctx`, `lang` or `language` in scope, and names the expression. `list` prints it as a last column (`@self.ctx.lang()`). |
| o1 | Untitled picker did not close on a WM close request | **Traced; no product change.** Both pickers are modal. GTK3's `gtk_main_do_event` delivers GDK_DELETE only to the toplevel of the current grab, and the picker opened second holds it, so a close request to the first one is dropped (as Windows disables a dialog under a modal one). GNOME (part D): first one ignored for 3 s while the second was open; the second closed; then the first closed; both `shell.pickFile` calls settled `{path: null}`. A user cannot open a second modal picker over the first, so this is a test artefact. |
| o2 | Pickers carry the Settings WM class | Not my files (picker.rs, R2-D). Untested guess: `picker::configure` sets the Wayland app id in `connect_realize`, but GTK3 creates the xdg_toplevel only at map. |
| o3, o4 | Grouping in the centre; deploy backup of the entries | Not my files (GE1a; round 7). With m2 (b), the "Melding" source now has its own group ("Melding" in both languages); nameless items group by icon. |

### 7.2 Verification (from G, final state)

| What | Command | Result |
|---|---|---|
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --summary --warnings` | 13,450 frozen; 13,390 present, 60 mapped or listed; 27 rows; **174 new literals (160 technical, 6 pair, 4 log, 3 user-visible-dutch, 1 user-visible-english), 29 catalogue keys, 20 allow-listed; 0 errors, 0 warnings**. 6.4 s (0.99 s with `--no-ratchet`). `rust-frozen-all.tsv` unchanged (sha `e72ad6ad75966c02`). |
| Ratchet mutations (scratch copy) | the reviewer's `fn extra() -> String { "Deze melding is nieuw en Nederlands.".into() }` in notifications.rs; a second copy of an existing `ApiError::new("bad_args", "Ongeldige melding.")` in the same file; a new file with a Dutch and an English text; a frozen Dutch text of picker.rs copied into gnome.rs | 1 error each (`new: … new Dutch literal outside the catalogues`); the new file also gave 2 English-only warnings. `--paths host/src/services/notifications.rs` reports the first; `--paths host/src/runtime.rs` gives 0 errors. Restored: 0/0. |
| Checker | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 27 rows; **0 errors, 0 warnings** |
| en-dutch mutations (scratch copy) | `core.tooLarge` en set to "Te veel gegevens." / "Te veel data, probeer opnieuw." / "Too much data (gegevens)." / "Too many items in the map."; desktop-entries.json Explorer en "Win11 Verkenner" | errors (Te, gegevens, veel / Te, opnieuw, probeer, veel / gegevens), a warning only for "map", an error for Verkenner |
| Label rule | `NODE=/nonexistent/node python3 -B tools/i18n/check_rust.py` (scratch copy); then with `--no-labels` | 1 error; then 0 errors and 1 warning |
| Converter | `extract_rs.py convert host/src/services/notifications.rs --stem notifications --site 124=notifications.invalid` (dry run, scratch copy) | warning: `call() has self.ctx in scope: pass @self.ctx.lang()`; with `@self.ctx.lang()` the patch uses it. `--site 3=…` (a new `"appName"`) is refused as technical. |
| Check | `tools/heavy.sh --tag ge1b-fix-check -- cargo check --offline --locked -j2 --all-targets` | 0 errors; warnings lib 28, win11-launch 2 (the baseline counts) |
| Tests | `WIN11_HEAVY_LOCK=$G/target/ge1b-heavy-wrapper.lock tools/heavy.sh --min-mb 2048 --tag ge1b-fix-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **271 passed** (269 + 2 new), 8 ignored; win11-launch 5 passed, 1 ignored; oracles **16/16** (oracle_tests 8, disks 7, reference_contracts 1); `guard-calls.log` 0 bytes; 24.1 s wall, 918 MB peak RSS. run-tests.sh waited for the real lock: GE1a held it for a Chrome dump. |
| Build | `tools/heavy.sh --tag ge1b-fix-build -- cargo build --offline --locked -j2` | rc 0. 12 binaries copied to `tests/build/r1-debug/`, `SHA256SUMS` rewritten (12/12 OK, `cmp` = `target/debug`). Built after the last Rust edit; `strings win11-panel` holds the new log line. |
| GNOME scenario | `tools/heavy.sh --min-mb 2560 --tag ge1b-fix-core -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/r1-debug --scenario tests/gnome/scenarios/ge1b-core.py` | exit 0, 161.2 s. **99 checks, 99 passed, none marked known.** Install: 87,844 mutating paths, 0 outside `run/`. Peak PSS 1,397 MiB, lowest MemAvailable 6,799 MiB, no leftover process. No panic and no `i18n:` miss in any app log. Stubs refused: wpctl 102, systemctl 80, pw-dump 30, apt 2. GNOME app names followed the files after 5.4 s (B), 5.0 s (C), 4.2 s (E) and 6.0 s (E-back). |
| Process-list cost | `tools/heavy.sh --min-mb 2560 --tag ge1b-perf-base -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/3b898e6-debug --scenario tests/gnome/scenarios/ge1b-perf.py --out tests/gnome/scenarios/ge1b-perf.out/3b898e6`, the same with `r1-debug` → `…/r1` | `sysmon.processes` median **36 ms (3b898e6) vs 37 ms and 36 ms (r1)** (mean 36.1 vs 36.9 and 35.9; p90 37 vs 38 and 37); `settings.get` 1 ms in both. WebKit's `performance.now()` has 1 ms resolution, and the language read was 30 µs vs 2 µs, so the difference is below what this can show, as expected. |

**What changed in the scenario.** The checks that the review marked known (`review:event-path`,
`review:picker-own-title`) now expect the fixed behaviour:
* **C5:** "Melding" source = "Melding", nameless = the label. A new check compares toast and centre.
* **Pickers:** both pickers carry the mode title.

New checks:
* A: `setTitle` is ignored.
* C-double: the quick double switch.
* D: the o1 order (lower picker ignored, then LIFO) and both picker promises settled.

The review's version of the file is described in `ge1b-review.md` §3.

The first `ge1b-perf` run with `--source 3b898e6` (the 3b898e6 UI as well) stopped at boot with "desktop WebView
ready (dataset.ready)" after 90 s. GE0c's `example.py` booted that same combination at 15:00, so I did not trace
it. The worktree-UI run above is the cleaner A/B anyway: only the host binaries differ.

### 7.3 Changed files in this round

* **Rust:**
  * `host/src/i18n.rs` (m6)
  * `host/src/lib.rs` (m5)
  * `host/src/desktop_entries.rs` (m5, `schedule_into`, 1 test)
  * `host/src/window.rs` (m3, m7, test)
  * `host/src/services/notifications.rs` (m2, m1 doc)
* **Tools:**
  * `tools/i18n/drift_rust.py` (M1)
  * `tools/i18n/check_rust.py` (M2, m4)
  * `tools/i18n/extract_rs.py` (m10, `en:` kind, new-literal classes)
* **Lists:**
  * `tests/i18n/allow-host.txt`: m1, plus 4 rules for i18n.rs's own log lines and join words, which the ratchet sees as new
  * `tests/i18n/BASELINE-CHANGES.rust`: the reason of the `window.rs#42` line only; no new key, no Dutch change
* **Notes:**
  * `notes/claude/round2-gate.md` (§1, §4 step 6, §6: ratchet, en-dutch, `--no-labels`, the `@LANG` hint, the R2-B perf row)
  * this section
* **Scenarios:**
  * `tests/gnome/scenarios/ge1b-core.py` (updated) and `ge1b-core.out/` (rerun, 2.4 MB)
  * `tests/gnome/scenarios/ge1b-perf.py` (new) and `ge1b-perf.out/{3b898e6,r1}/`
* **Build output:** `tests/build/r1-debug/` (12 binaries + SHA256SUMS), `target/`

### 7.4 Deviations

1. **C8:** the test run still used the wrapper-lock recipe (deviation 1 above); the memory check ran before the
   real lock wait. The cargo check/build and the 4 GNOME runs used plain `heavy.sh`.
2. **Allowlist:** I edited `tests/gnome/scenarios/ge1b-core.py` and created `ge1b-perf.py`. Neither is in my
   listed paths, but the task asks me to re-run the step's scenario, and it encoded the defects as known
   failures. Both follow the `<step>-*` scenario rule (F36).
3. **C5:** the extension's list is no longer mapped (m2 b). The contract's "on load" now means "into the host's own
   store (`add()`)".
4. **C4:** the host now enforces "the UI does not send `setTitle` for native windows" by ignoring it.

### 7.5 Left

* m8 (Robin).
* o2 (R2-D, picker.rs).
* o3 (GE1a, if the per-icon grouping is unwanted).
* o4 (round 7 deploy recipe).
* The ratchet's quality rests on GE0b's classifier: a Dutch text it calls technical is only a warning.

### 7.6 Decisions for the coordinator

1. **C5 wording:** "The host stores its own notifications with `appName: ""`; an item that enters the host's store
   with `"Melding"` is stored as `""`. The extension's list and events pass through unmapped." I implemented this.
2. **Picker titles:** a caller's own title wins only for file-portal callers (implemented). The page's own title
   for in-app callers is GE1a's.
3. **`setTitle`:** now ignored for host-titled windows (implemented, as the review recommends). Supersedes my
   decision 3.
4. **o1:** accept as GTK modal semantics; no product change.
5. **Lock:** unchanged; `WIN11_HEAVY_HELD` in heavy.sh/run-tests.sh is still open, and those files are not mine.
6. **Commit:**
   * `tests/gnome/scenarios/ge1b-core.out/` and `ge1b-perf.out/` (at least their `results.json`);
   * `ge1b-perf.py` as R2-B's baseline.
7. **Ratchet cost:** a plain `drift_rust.py` now takes about 6 s instead of 1 s (scan_rust on the whole tree). I
   kept it on by default so "drift clean" includes "ratchet clean".
