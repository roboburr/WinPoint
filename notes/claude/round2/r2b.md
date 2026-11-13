# R2-B: sysmon, storage, disks, index (round 2, 2026-11-05)

W = `.wt/r2-b` (branch `r2/b`, from d666df54; `git status --short` was empty and HEAD d666df54 at the start).
Nothing committed. No win11-* binary, gnome-shell, installer, gsettings or dconf ran outside the sandboxes; every
cargo and harness run went through W's `tools/heavy.sh` (tags `r2b-*` in `tests/logs/heavy-events.log`).

## 1. Result

| Item | State |
|---|---|
| Conversion | every user-visible literal of my files is a catalogue row or allow-listed (C9): **165 Dutch sites + 5 English pair halves, 139 rows** (sysmon 70, storage 23, disks 1, index 45; fix round: +1 index row, M1) |
| Dutch | byte-identical. **0 Dutch text changes.** 15 BASELINE lines: 14 removed technical literals and 1 `key:` line for a row whose Dutch equals 3b898e6's output (section 3; fix round +1, n2) |
| Extras 1-4 | done: language-keyed disks snapshot and storage.volumes cache (+ scan trees, scan errors, categories, health names), clean English Disks body, index status as key + params with a re-send on a switch, D-G9 unit names |
| Extras 5-6 (GNOME) | **blocked by a harness bug in a worktree** (section 6.3); build snapshot `tests/build/r2b-debug` ready, unit tests cover both host behaviours, the minimal harness patch is in section 8 |
| Checks | drift 0/0 (paths and full), check_rust 0 errors / 1 explained warning, check.py 0 errors, cargo check no new warning, tests **292 passed (271 + 21), 8 ignored** (fix round, section 9), 16 oracles ok, guard log empty |

## 2. What was converted, per file

Keys are `<stem>.<area>.<meaning>`. Language: `lang` threaded from `self.ctx.lang()` / `ctx.lang()` everywhere a
ServiceContext exists; `crate::i18n::current_language()` only in the index worker (engine.rs, query.rs, cache.rs),
which has no context and whose texts the host renders again from the key (extra 3).

| File | Sites | Keys (examples) | Hand work (refused or restructured) |
|---|---|---|---|
| services/sysmon.rs | 1 | `sysmon.process.single` | `common::err` → `ApiError::new` first (literal unchanged) so the converter made `ApiError::tr` (key + params, C2); `lang` passed to the actions |
| sysmon/actions.rs | 34 | `sysmon.service.*`, `sysmon.process.*`, `sysmon.endTask.*`, `sysmon.efficiency.*`, `sysmon.restart.*` | `err`/`refusal` → `ApiError::new` / `with_data` before converting. `check_own` returned a Dutch string and `restart_plan` matched `"bestaat niet"` in it to choose the code: now `check_own → Option<Own>` (Critical, Gone, OtherUser, Kernel) with `key()`, `message(lang)`, `refusal(lang)`, so English keeps `not_found`. `CONFIRM_TTY` (a `const` text) moved into `confirm(…, lang)`. "een ander programma" stays its own row (`sysmon.restart.otherProgram`, the {parent} piece, gate §5). `restartPlan`'s own_names pair now uses `i18n::pick` |
| sysmon/common.rs | 38 nl + 5 en | `sysmon.unit.*` (28), `sysmon.duration.*` | D-G9 below. `friendly()`'s gsd pair → `sysmon.unit.gnomeSettings` (+ `map --en`). `fmt_duration`: seconds one row (+ en half), minutes a plural `trn` row whose `.one` is 3b898e6's own "1 minuut" / "1 minute" (no Dutch change), the hour unit u/h one row inside the unchanged `"{} {} {} min"` (pieces keep Dutch byte-identical). "Stationsbewaking ({})" and "Bluetooth-mediabediening" needed `--force` (GE0b called them technical). `err`/`refusal` removed (unused) |
| sysmon/startup.rs | 4 | `sysmon.startup.*` | `set(…, lang)` |
| sysmon/text.json | – | – | `_GSD_NL` / `_GVFS_NL` (Dutch-only) became `_GSD` / `_GVFS` with `{nl, en}` per part; nl byte-identical (still frozen there), en new |
| sysmon/process.rs, health.rs | 0 | – | health names (extra 1); `own_names` stays an inline pair table (gate §2 `pick`) |
| sysmon/performance.rs | 0 | – | "Advanced Micro Devices" is a PCI vendor matcher: allow-listed |
| storage/cleanup.rs | 14 | `storage.nothingChosen`, `storage.closeFirst` {names}, `storage.noAdmin` … | `res(…, lang)`, `run_admin(…, lang)`; 4 `&tr(…)` temporaries in if/else tails fixed (edition 2024) |
| storage/common.rs | 4 | `storage.noFolder`, `storage.notAbsolute`, `storage.notScannable`, `storage.noSuchFolder` | `check_dir(path, lang)`; `err` removed (unused; clipboard uses only `which`/`atomic`) |
| storage/scan.rs | 12 | `storage.badArgs`, `storage.busy`, `storage.scanStopped`, `storage.folderDenied` … | errors stored as key + params (extra 1); `walk`'s two parallel `if`s → one `if`/`else` |
| storage/categories.rs | 1 | `storage.noHome` | names re-rendered at read (extra 1) |
| storage.rs, volumes.rs | 0 | – | the volumes cache (extra 1) |
| services/disks.rs | 0 | `disks.unavailableBody` (new row) | all disk texts are the `_STR` table (D-G8); extra 1 and 2 |
| services/index.rs | 23 | `index.restarting`, `index.openAlles`, `index.noSuchFolder` {path}, `index.sizedDrive` {size} … | extra 3; `request()`'s `if` inside `ApiError::new` split; `size_label(lang, …)` |
| index/query.rs | 19 | `index.query.*`, `index.badArgs` | `query::Error` is now `pub type Error = ApiError` (`Error::new` → `ApiError::new`, `&format!`), so every parser error carries key + params to the host |
| index/engine.rs | 5 | `index.rootNotFound`, `index.rootNotFolder`, `index.cancelled`, `index.unknownCommand` | root errors are `ApiError`s (extra 3) |
| index/cache.rs | 9 | `index.cache.*` (8) | the Dutch io::Error texts are `Fault(key)` inside the io::Error; `root_error()` keeps the key up to the status |
| bin/index_worker.rs | 0 | – | no text |

**Refused by the converter:** none of my user texts has a format spec (`{:.1}` etc. are technical number
formatting). **Not converted on purpose:** the four `_STR` JSON tables (D-G8), `_FRIENDLY` and `own_names` (inline
data pairs), the two allow-listed matchers/details (section 4), "GNOME Shell" (a name) and the CPU model fallback
"Processor" (technical, same word in both languages).

## 3. Dutch changes

None. `tests/i18n/BASELINE-CHANGES.rust-r2b` has 15 lines (fix round: +1, process.rs), all with a "no Dutch change" reason:
* 10 sysmon technical literals: `"bestaat niet"` (the message matcher, replaced by `Own`), 7 × `"en"` (the inline
  pairs now rows or `i18n::pick`, and unit_friendly's `return None` for English, D-G9), `"_GSD_NL"`, `"_GVFS_NL"`.
* 2 index.rs literals of `size_label` (`","`, `",0"`): Dutch "1,5 TB" / "2 TB" unchanged, English now "1.5 TB".
* 2 query.rs json keys `"code"`, `"message"` (the old `Error::json()`).
* `key:disks.unavailableBody`: a `#@ new` row whose Dutch is 3b898e6's text byte for byte (copied from data.json by
  script, not typed). It exists so English can have its own body (extra 2). Decision 2.

D-G1a: the minutes row is a plural, but 3b898e6 already had "1 minuut", so nl does not change.

## 4. Allow-list rules (`tests/i18n/allow-host-r2b.txt`)

| Rule | Reason |
|---|---|
| `host/src/services/sysmon/performance.rs  fn:nice_pci&class:user-visible-english` | "Advanced Micro Devices" is matched in the GPU vendor string and turned into "AMD"; never shown |
| `host/src/services/index/query.rs  fn:new&class:user-visible-english` | the 5 regex error details in Python re wording ("nothing to repeat" …) fill `{detail}` of `index.query.regexInvalid` in both languages, like GLib's own message for other codes (unchanged 3b898e6 behaviour) |

## 5. Extras

1. **Language-keyed caches** (round2-gate §2):
   * `disks.list` 1 s snapshot: `snap: Mutex<Option<(Instant, Lang, Value)>>`, used only in the language it was
     built in (`snapshot()` reads `ctx.lang()` once and passes it to `ParseOptions` and the filesystem labels).
     Test `disks::tests::the_snapshot_is_used_only_in_the_language_it_was_built_in` (an nl snapshot, an atomic
     rename to en, `snapshot(false)` within the second must not serve it; the new one is stored as en).
   * `storage.volumes` 2 s cache: `VolumeCache { time, lang, … }` with `get(lang, include, now)` / `put`. Tests
     `storage::i18n_tests::the_volume_cache_is_used_only_in_its_language` (unit) and
     `storage_volumes_follow_a_switch_within_the_cache_window` (through `Storage::call`: "Systeemschijf" →
     "System drive" right after a switch, when the root file system is listed).
   * Also found and fixed in my files: the scan tree cache (`(path, depth, topN)`, not language) holds the
     "Overige" rows → `localize()` sets them whenever a tree is handed out; a scan's stored error (shown later by
     `storage.scanResult`) is kept as the `ApiError` (key + params) and rendered at read (`message()`); the
     categories cache (15 min, and on disk) gets its names re-rendered in `get()`; the health page's busy-process
     names come from the last process list (`last_names`), so `health()` rebuilds that list when its language
     (`names_lang`) differs. Tests: `scan::tests::stored_errors_and_cached_trees_follow_the_language`,
     `categories::tests::cached_category_names_follow_the_language`.
   * The sysmon desktop-entry cache and the cleanup list cache were already keyed by language.
2. **Disks English wording:** the Schijven body under "Disk Management doesn’t work" is now "UDisks isn’t available
   on this computer." (`disks.unavailableBody`, typographic apostrophe). Dutch stays "Schijfbeheer werkt niet:
   UDisks is op deze computer niet beschikbaar.". The API error `unavailable` (map_error) keeps _STR's text in both
   languages, because `disks/reference-tests.json` pins the English "Disk management doesn't work: …" (`errors`
   cases; the oracle is never edited). `disks.list` uses the body via `unsupported(lang, e)`; actions that find
   UDisks gone (`fresh()`) reuse it, so their toast reads the same. The rest of the `_STR` en column keeps its ASCII
   apostrophes: the oracle has 214 English cases and 486 strings with an ASCII apostrophe. Test
   `the_unavailable_body_does_not_repeat_its_heading_in_english_and_keeps_dutch`.
3. **index.status as key + params:**
   * Worker side: root errors are `ApiError`s; `status()` sends `error` (its own rendering) plus `errorKey` /
     `errorParams`; request errors (`e.json()`) carry `code, message, key, params`.
   * Host side: `localized(status, lang)` renders every root error again from its key when a status is relayed
     (event), returned (`index.status`) or re-sent; `last_status` keeps the worker's form. `worker_error(json,
     lang)` renders a worker error from its key with `ApiError::tr` (C2 envelope with key/params). Pending requests
     that fail when the worker restarts get `key: index.restarting`.
   * Re-send: the existing 500 ms config watcher compares `ctx.lang()`; on a change it calls `resend_status(lang)`:
     the gave-up status (`gave_up_status(lang)`) after five crashes, else the last status localized. (A running
     worker also sends a status every 500 ms, which the host now renders in the current language.)
   * Input words (C9): `index/query.rs` aliases and date words are untouched. G's parser accepts `dm:today` as well
     as `dm:vandaag` (`date_word`: `"today" | "vandaag"`), so the English examples say `dm:today`; test
     `query::tests::the_examples_of_the_messages_parse` parses every example of the messages.
   * Tests: `index::i18n_tests::{index_errors_render_in_english_and_dutch_through_the_service,
     worker_errors_and_root_errors_are_rendered_in_the_hosts_language,
     a_language_switch_sends_the_status_again_in_the_new_language}` (the last one: a status with a root error, a
     real settings rename, the `index.status` event arrives in English within 3 s; then the gave-up message in
     Dutch).
4. **D-G9:** `unit_friendly` no longer returns `None` for English. Its table is `UNIT_NAMES` (unit → key, 28),
   rendered only for the unit found; `sysmon.unit.gnomeSettingsPart` / `sysmon.unit.volumeMonitor` take the part
   from text.json `_GSD` / `_GVFS` `{nl, en}`. English follows `_FRIENDLY`'s English where both name the same
   program. Tests: `sysmon::i18n_tests::unit_friendly_names_have_english_rows_and_dutch_is_unchanged`,
   `common::i18n_tests::every_unit_key_exists_and_every_part_name_has_english`, and
   `unit_friendly_matches_3b898e6_in_dutch_and_costs_no_more` (3b898e6's function, copied into the test, gives the
   same Dutch for all 58 test units: the 28 table units, every GSD/GVFS part and 7 patterns or unknown units).
5. **Performance (F28):** build snapshot made (6.1). The GNOME A/B (`ge1b-perf.py` on r1-debug and r2b-debug)
   could not run (6.3). In-process measurement of the only per-row text code that changed (`unit_friendly`, once
   per unit process in `sysmon.processes`), debug build, 58 units, three runs: 3b898e6 0.48-0.53 µs per call, now
   1.42-1.47 µs (a catalogue lookup instead of a stack table); English 0.013 µs (it returned `None`) → 1.42 µs.
   With about 100 unit processes that is ≈ 0.1 ms on a 36-37 ms call (GE1b's r1 median), below the harness's 1 ms
   timer resolution. `processes()` still reads the language once per call; the only other change in it is one
   `String` move (`names_lang`).
6. **ge1a-disks-race:** blocked (6.3). Prepared `tests/gnome/scenarios/r2b-disks-race.py`: GE1a's scenario with
   R2-B's English body, and its host evidence (`cachedKeepsOldLanguage`) turned into a check, plus a check that the
   English body does not repeat the heading. GE1a's own copy cannot pass its English checks on this host: its
   `HOST_TEXT['en']` is the 3b898e6 wording with an ASCII apostrophe (decision 6).

## 6. Verification (from W's root)

### 6.1 Commands and results

| Check | Command | Result |
|---|---|---|
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths <the 27 host/src files of my stems incl. text.json and the two data.json> --warnings` | 13450 frozen; 4770 present, 242 mapped or listed; 165 rows; 221 new literals; **0 errors, 0 warnings** |
| Drift, all | `python3 -B tools/i18n/drift_rust.py --warnings` | 13208 present, 242 mapped or listed; 165 rows; 395 new literals; **0 errors, 0 warnings** (no `new:` warning). `--new`: 395 new = 381 technical + GE1b's 14 (bilingual-pair 6, log 4, user-visible-dutch 3, user-visible-english 1, all i18n.rs/window.rs/desktop_entries.rs); 195 catalogue keys, 26 allow-listed |
| Catalogues | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 165 rows; **0 errors, 1 warning**: `[glossary] disks.unavailableBody: nl has 'Schijfbeheer', en lacks 'Disk Management'`, intended (extra 2: the English body must not repeat its heading) |
| All layers | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 53 warnings: 52 glossary (UI and the unchanged data.json `timeout`/`unavailable` rows, "until round 3") + the rust one above |
| Build check | `tools/heavy.sh --tag r2b-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28, lib test 26, win11-launch 2 (+1 test): the baseline counts. In my files the same 7 pre-existing warnings as at d666df54 (`err` fields of storage/sysmon common.rs, `last_changed`, `counts`, `state_name`, unused `CString`, `NOSHOW`), no new one |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2b-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **287 passed, 8 ignored** (271 + 16 new); win11-launch 5 passed, 1 ignored; other bins 0. Oracles 16/16: `oracle_tests::*` 8, the 7 python `disks::tests`, `reference_contracts`. `every_converted_site_renders_its_3b898e6_dutch` ok. `guard-calls.log` 0 bytes, 0 tmp entries left; 16.1 s wall, 984 MB peak RSS |
| Cost test | `tools/heavy.sh --min-mb 2048 --tag r2b-bench -- tests/rust-sandbox/run-tests.sh --lib -- unit_friendly_matches --nocapture` (3×) | the numbers in extra 5 |
| Build | `tools/heavy.sh --tag r2b-build -- bash -c 'cargo build … && find target/debug -maxdepth 1 -type f -executable -name "win11-*" -exec cp {} tests/build/r2b-debug/ \; && sha256sum …'` | rc 0; 12 executables (the same 12 names as `tests/build/r1-debug`), `SHA256SUMS` 12/12 OK, `cmp` = `target/debug`; built at 19:03 after the last source edit (18:56); `strings win11-panel` holds `disks.unavailableBody`; 231 MB |
| No frozen text | `grep -rnE 'static \|OnceLock\|lazy\|LazyLock\|thread_local\|const [A-Z_]+: &str'` over my files | only JSON data tables, the lsblk/GPU/boot-time data caches, regexes, D-Bus names and `UNIT_NAMES` (keys, not text). Every text cache is keyed by language or rendered at read (extra 1) |
| Formatting | `rustfmt --edition 2024 sysmon.rs storage.rs disks.rs index.rs` (in host/src/services) | all four module trees were rustfmt-clean at d666df54 (checked on a `git archive` copy); clean again |
| Status | `git -C W status --short` | only my files (section 7) |

New tests (16): sysmon 3 + 2 (common.rs), storage 3 + 1 (scan) + 1 (categories), disks 2, index 3 + 1 (query).
The en render test per stem goes through the real call sites: `Sysmon::call`, `Storage::call`, `Index::call` (+
`worker_error`), and for disks `map_error` + `unsupported` (the code `snapshot()` uses).

### 6.2 Not verified

No GNOME run (6.3): the live Disks page, the Alles status line after a switch and the process-list timing in the
real host are unit-tested only. Health's names after a switch are not tested (it needs a process busy for 30 s).

### 6.3 The harness cannot run from this worktree

`tools/heavy.sh --min-mb 2560 --tag r2b-perf-r1 -- python3 -B tests/gnome/harness.py launch run --lang nl
--build-dir tests/build/r1-debug --source worktree --scenario tests/gnome/scenarios/ge1b-perf.py --out
tests/gnome/scenarios/ge1b-perf.out/r2b/r1` → exit 1 after 99.8 s (the private install was fine: 87,844 paths, 0
outside the run dir): `FATAL: RuntimeError: private system bus reported ''`; `system-bus.log`: "dbus-daemon: Failed
to start message bus: Socket name too long" (copied into `ge1b-perf.out/r2b/r1/logs/`). `RT = RUN/'rt'` gives
`W/tests/gnome/run/rt/system-bus` = 102 bytes; dbus-daemon refuses socket paths of 100 bytes and more, while the
harness only asserts `< 104`. From G the path is 93 bytes, which is why every earlier run worked. harness.py is
frozen for me, and running G's copy would write into G's tree, so I stopped there. Patch: section 8, decision 1.

## 7. Files changed (all owned)

* `host/src/services/sysmon.rs`, `sysmon/{actions,common,health,process,startup}.rs`, `sysmon/text.json`
* `host/src/services/storage.rs`, `storage/{categories,cleanup,common,scan}.rs`
* `host/src/services/disks.rs`, `disks/tests.rs`
* `host/src/services/index.rs`, `index/{cache,engine,query}.rs`
* `host/i18n/{sysmon,storage,disks,index}.tsv` (converter only: `convert`, `map --en`, `add` for the one new row)
* `tests/i18n/BASELINE-CHANGES.rust-r2b`, `tests/i18n/allow-host-r2b.txt` (new)
* `tests/gnome/scenarios/r2b-disks-race.py` (new), `tests/gnome/scenarios/ge1b-perf.out/r2b/r1/` (the failed
  run, as evidence)
* git-ignored: `tests/build/r2b-debug/` (12 binaries + SHA256SUMS), `target/`, `tests/logs/`, `tests/gnome/run/`
* this note

## 8. Deviations, left, decisions for the coordinator

**Deviations**
1. Extras 5 and 6 not run in GNOME (6.3); replaced by unit tests and an in-process cost measurement.
2. `rustfmt` on my four module trees (formatting only; they were clean at d666df54; the pilot had left converter
   lines unformatted).
3. The build recipe's `cp target/debug/win11-*` would also copy the 12 `.d` files; I copied only the executables
   (`find -type f -executable`), as r1-debug holds.
4. Behaviour changes in English or shape only (Dutch output unchanged): restartPlan keeps `not_found` for a gone
   process in English, which a plain conversion would have broken (3b898e6's check_own text was Dutch in both
   languages, so its "bestaat niet" matcher worked in English too; fix round, review m4); `index.status` roots
   carry `errorKey`/`errorParams` (additive); worker and storage/sysmon errors carry key + params (C2); English
   location labels are new (3b898e6 showed the Dutch label in both languages), with a decimal point;
   `sysmon.health` rebuilds its process list only after a switch while a process is busy (fix round, review n3).
5. Gate §5 says the parser "accepts only dm:vandaag, not dm:today": wrong for G (both are accepted, tested).

**Left**
* The GNOME runs, after the patch (decision 1).
* Disks `_STR` en apostrophes stay ASCII (oracle); D-G23 for the JSON tables needs a round-3 decision.

**Decisions for the coordinator**
1. **Harness patch** (`tests/gnome/harness.py`, frozen for me): inside the sandbox /tmp is a private tmpfs
   (`bwrap_argv`: `--tmpfs /tmp`) and RT is used only inside it, so `RT = Path('/tmp/w11-rt')` (comment: short for
   sockaddr_un and dbus-daemon's 99-byte limit, also from a worktree), and `prepare()`'s assert `< 104` → `< 100`.
   Then, from W (or G after the merge):
   `tools/heavy.sh --min-mb 2560 --tag r2b-perf-r1 -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/r1-debug --source worktree --scenario tests/gnome/scenarios/ge1b-perf.py --out tests/gnome/scenarios/ge1b-perf.out/r2b/r1`,
   the same with `r2b-debug` → `…/r2b/r2b`, then `ge1a-disks-race.py` and `r2b-disks-race.py` with
   `--build-dir tests/build/r2b-debug` (`--out tests/gnome/scenarios/ge1a-disks-race.r2b` and
   `tests/gnome/scenarios/r2b-disks-race.out`).
2. **`disks.unavailableBody`** is a `#@ new` row with a `key:` line although its Dutch is unchanged: a row mapped to
   the data.json literal fails drift: an error, `data.json does not use this key` (drift_rust.py:862, data.json does
   not hold the key), plus a warning, `literal is still present` (it stays for map_error); re-checked in the fix
   round. Accept, or teach drift_rust.py a "copy of a frozen literal" mapping.
3. **GSD/GVFS part names** stay text.json data with `{nl, en}` (like `_FRIENDLY`), not 23 rows: rows mapped to
   JSON literals need the key inside the JSON, and check_rust.py counts key references only in .rs files (23
   "unused" warnings). Accept, or extend check_rust's `referenced` to the JSON tables and I convert them.
4. **own_names** (process.rs) stays an inline pair; it duplicates `window.title.*`. Using those keys would need 10
   BASELINE lines; leave for round 3.
5. Fix the gate §5 sentence on `dm:today`.
6. **ge1a-disks-race.py** (GE1a's) pins the 3b898e6 English body; on R2-B's host use `r2b-disks-race.py`, or
   update GE1a's `HOST_TEXT['en']` to "UDisks isn’t available on this computer.".
7. The index worker renders its errors in its process language (`current_language()`, the settings file it
   shares with the host) and the host renders them again from the key; only `index.query.invalidPattern` (a
   parameter, reached when GLib reports no error text) stays in the worker's language.

## 9. Fix round (after `notes/claude/round2/r2b-review.md`, 2026-11-05)

**Start state.** `git -C W status --short` and `git -C W diff --stat` matched the review's view (22 modified files
plus 5 new paths, all owned). No owned file was newer than this note except the review itself and the reviewer's
ignored `tests/gnome/scenarios/__pycache__/`. Nothing committed; no win11-* binary, gnome-shell, installer, gsettings
or dconf ran; every cargo run went through W's `tools/heavy.sh` (tags `r2b-check`, `r2b-test`, `r2b-test-nocap`,
`r2b-build`).

### 9.1 Per finding

| Finding | What I did |
|---|---|
| **B1** (blocker) categories `data: null` → `{"categories": null}` | **Fixed.** `Categories::get` looks the list up with `get_mut("data")…get_mut("categories")…as_array_mut()` and inserts nothing; a category is renamed only when it has a string `id`. Tests `storage::categories::tests::get_without_data_keeps_data_null` (nl and en; no cache file, a non-object file `[1,2]`, a corrupt file; the reply equals 3b898e6's shape `{"ready":false,"data":null,"running":true,"progress":{…}}`) and `get_keeps_a_cached_object_without_categories_as_it_is` (no key inserted). |
| **M1** (major) "Inconsistente indexkolom" | **Fixed.** `extract_rs.py convert host/src/services/index/cache.rs --stem index --site 76=index.cache.badColumn --en "76=Inconsistent index column" --expect "76=Inconsistente indexkolom" --force --apply` (GE0b classed it technical), then by hand `fault(io::ErrorKind::InvalidData, "index.cache.badColumn")` like its 8 siblings. Row `#@ host/src/services/index/cache.rs 74 bc889c4a0219f3ef plain`; no Dutch change. Test `index::cache::tests::a_column_of_the_wrong_length_keeps_its_key_up_to_the_root_error` (Mapping::open with a wrong length → `root_error` code `failed`, key `index.cache.badColumn`; nl/en render). Re-scan: `extract_rs.py list --all` over the 24 owned non-test .rs files (4361 literals) against `tests/i18n/dutch-words.txt`: 34 hits, all C9 parser words (`alias`, `parse_size`, `date_word`), the 4 `own_names` pair halves (decision 4) and the worker's CLI usage line `eprintln!` (engine.rs:922, class log). Nothing else is missed. |
| **M2** (major) GNOME runs blocked | **Not fixable in my files; still open.** `tests/gnome/harness.py` is frozen for me. I tried a workaround that edits nothing: the unmodified harness in an outer bubblewrap with W bound at `/mnt/w` (system-bus socket path 36 bytes). The kernel refuses the nested sandbox: `bwrap: No permissions to create a new namespace` (`kernel.apparmor_restrict_unprivileged_userns = 1`; Ubuntu's `unpriv_bwrap` profile, which bwrap's children run under, denies capabilities), while a single bwrap layer works. A patched copy of the harness would have to change `HERE`, `RT`, the assert and the self-invocations (`bwrap_argv`, the `_inner` exec): a fork of a frozen safety tool, so I did not make one. The build snapshot is rebuilt after this round (9.2), and the commands of decision 1 are unchanged. GE1a's `ge1a-disks-race.py` is GE1a's file: decision 6. |
| **m1** English rows | **Fixed** with `extract_rs.py en … --apply`: `storage.trashFailed` "That didn’t work."; `sysmon.restart.noEnviron` "This process’s environment can’t be read, so it can’t be restarted here."; `sysmon.restart.parent` "This process was started by “{parent}” and belongs to it. Restart that program." (quoted like nl; the rare fallback reads “another program”, as nl does). **Held back for Robin (cosmetic Dutch, not changed):** the nl of `storage.trashFailed` is 3b898e6's "Dat lukte niet (ApiError)." with Codex's "(ApiError)" leftover; dropping it is a Dutch change for round 3. |
| **m2** test gaps | **Fixed.** Categories without data (B1, above). Health names after a switch: `sysmon::process::tests::busy_names_are_rebuilt_only_after_a_language_switch` (see n3). `storage_volumes_follow_a_switch_within_the_cache_window` now decides from `/proc/self/mountinfo` whether `/` is a `_REAL_FS` mount (not a loop device); it is (ext4), so the check runs and is no longer skipped silently; only without a real root does it assert that no system drive is listed. Disks: `disks::tests::disks_list_follows_a_language_switch_through_the_service` calls `Disks::call("disks.list")` in nl, renames settings.json to en and calls again within the snapshot's second: the reply equals the English unsupported reply (error and file-system descriptions differ from nl; the English `failed` text leads the body). It runs only where `DBUS_SYSTEM_BUS_ADDRESS` names a missing socket, which run-tests.sh guarantees. In the sandbox the missing socket gives code `failed`, not `unavailable`, so the unavailable body itself stays covered by `the_unavailable_body_does_not_repeat_its_heading_in_english_and_keeps_dutch` (same `unsupported()` call). A `--nocapture` run of the three tests printed no skip message. |
| **m3** `key:disks.unavailableBody` | **Not done: the review's premise is wrong (evidence).** I applied the proposed fix exactly: removed the row, then `extract_rs.py add --stem disks --key disks.unavailableBody --from host/src/services/disks/data.json#66 --section host/src/services/disks.rs --apply` (nl taken by script from `rust-frozen-all.tsv`, en and note from the old row). `drift_rust.py --paths <owned> --warnings` then gives **1 error** besides the warning: `error: host/i18n/disks.tsv:15: disks.unavailableBody: host/src/services/disks/data.json does not use this key` (drift_rust.py:862-863: a mapped file that still exists must hold the key as a literal, `key_used`). So I restored the reviewed bytes (`#@ new` + `key:` line). The `key:` reason now says why, and that it is no Dutch change. Dutch byte identity is pinned by the disks test above: nl body == `map_error("nl", …)` message == data.json `_STR/nl/unavailable`, which drift checks as present at 3b898e6. Decision 2 stays. |
| **m4** wrong statements | **Fixed.** sysmon.rs test comment (3b898e6's check_own text was Dutch in both languages; a plain conversion would have broken English); §8 deviation 4 ("keeps `not_found` in English …"; "English location labels are new"); decision 2 (now: an error plus a warning, re-checked); the BASELINE reason of `index.rs#184`. |
| **n1** allow reason | **Fixed:** "user-visible English detail of 3b898e6 … kept so Dutch stays byte-identical; translating it is a Dutch change (round 3 / Robin); not C9". |
| **n2** own_names in process.rs | **Fixed:** `crate::i18n::pick(&lang, nl, en)`; the removed `"en"` is BASELINE line `host/src/services/sysmon/process.rs 59` (no Dutch change). Reusing `window.title.*` stays decision 4. |
| **n3** health's first call | **Fixed:** `health::busy(ctx, pm, lang, 30.)` rebuilds the list only when a process is busy **and** the names were built in the other language (`names_lang` non-empty and different). Before the first list (names from /proc/comm) and while nothing is busy it does what 3b898e6 did. The test covers the four cases: same language (no list), other language (list rebuilt, names_lang en, the test process named as the host), nothing busy, no list yet (comm name, no list). Seeded cached unit descriptions keep systemctl out of the test (guard log stays empty). |
| **n4** two English "UDisks missing" texts | Left for round 3 (D-G23 for the `_STR` tables together with the disks oracle). |
| **n5** | Coordinator (gate §5 sentence; JSON part names accepted). |

### 9.2 Done-when (from W's root, after the last edit)

| Check | Command | Result |
|---|---|---|
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths <the 29 owned host/src files: 26 .rs incl. disks/tests*, text.json, the two data.json: `host/src/services/{sysmon,storage,disks,index}.rs`, every .rs/.json under those four directories except `reference-tests.json`, `host/src/bin/index_worker.rs`> --warnings` | 13450 frozen; 4768 present, 244 mapped or listed; 166 rows; 222 new; **0 errors, 0 warnings** |
| Drift, all | `python3 -B tools/i18n/drift_rust.py --warnings` | 13206 present, 244 mapped; 166 rows; 396 new literals; **0 errors, 0 warnings**, no `new:` warning. `--new`: 396 = 382 technical (the +1 is the key literal `index.cache.badColumn`) + GE1b's 14 (bilingual-pair 6, log 4, user-visible-dutch 3, user-visible-english 1; i18n.rs, window.rs, desktop_entries.rs); 196 catalogue keys, 26 allow-listed |
| Catalogues | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 24 stems, 166 rows; **0 errors, 2 warnings**, both intended: `[glossary] disks.unavailableBody … lacks 'Disk Management'` (extra 2) and the new `[glossary] sysmon.restart.noEnviron: nl has 'Instellingen', en lacks 'Settings'`: here "instellingen" is the process's environment, not the Settings app (review m1) |
| All layers | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 54 warnings (52 glossary "until round 3", 2 rust above) |
| Build check | `tools/heavy.sh --tag r2b-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib 28, lib test 26, win11-launch 2, launch test 1 (baseline); in my files the same 7 pre-existing warnings (cache.rs:7 `CString`, cache.rs:735 `counts`, engine.rs:4 `NOSHOW`, engine.rs:40 `last_changed`, engine.rs:284 `state_name`, `err` fields of storage/common.rs:101 and sysmon/common.rs:118); no new one |
| Tests | `tools/heavy.sh --min-mb 2048 --tag r2b-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **292 passed, 8 ignored** (271 + 21: the 16 of round 2 + 5 of this round); win11-launch 5 passed, 1 ignored; oracles 16/16 (`oracle_tests::*` 8, the 7 python `disks::tests`, `reference_contracts`); `every_converted_site_renders_its_3b898e6_dutch` ok (now incl. `index.cache.badColumn`); `logs/guard-calls.log` 0 bytes; 0 tmp entries left |
| en render per stem | part of the run | sysmon `Sysmon::call` (+ `health::busy`), storage `Storage::call` (+ `Categories::get`), index `Index::call` / `worker_error` / `root_error`, disks `Disks::call("disks.list")` (new) + `unsupported` |
| No frozen text | `grep -nE 'static \|OnceLock\|lazy\|LazyLock\|thread_local\|const [A-Z_]+: &str'` over my files | unchanged: JSON tables, lsblk/boot/GPU data caches, regexes, D-Bus names, `ADMIN_SCRIPT`, `UNIT_NAMES` (keys); every text cache is keyed by language or rendered at read |
| Formatting | `rustfmt --edition 2024 --check sysmon.rs storage.rs disks.rs index.rs` | clean (two of my new blocks reformatted) |
| Build snapshot | `tools/heavy.sh --tag r2b-build -- bash -c 'cargo build --offline --locked -j2 && rm -rf tests/build/r2b-debug && mkdir -p tests/build/r2b-debug && find target/debug -maxdepth 1 -type f -executable -name "win11-*" -exec cp {} tests/build/r2b-debug/ \; && (cd tests/build/r2b-debug && sha256sum win11-* > SHA256SUMS)'` | rc 0; 12 executables, the same names as `tests/build/r1-debug`; `SHA256SUMS` OK; byte-identical to `target/debug`; built 19:58, after the last source edit (19:53); `win11-panel` holds `index.cache.badColumn` |
| GNOME runs (F28, extra 6) | – | **not run** (M2, 9.1) |
| Status | `git -C W status --short` | only my files: the 22 modified product files, `notes/claude/round2/` (this note + the review), `tests/gnome/scenarios/ge1b-perf.out/r2b/`, `tests/gnome/scenarios/r2b-disks-race.py`, `tests/i18n/BASELINE-CHANGES.rust-r2b`, `tests/i18n/allow-host-r2b.txt` |

### 9.3 Files changed in this round

* `host/src/services/storage/categories.rs` (B1, 2 tests), `host/src/services/storage.rs` (m2 test condition)
* `host/src/services/index/cache.rs` (M1, 1 test), `host/i18n/index.tsv` (+1 row, converter)
* `host/src/services/sysmon/health.rs` (n3 `busy`), `sysmon/process.rs` (n2 `pick`, 1 test), `sysmon.rs` (m4 comment)
* `host/src/services/disks/tests.rs` (m2, 1 test)
* `host/i18n/storage.tsv`, `host/i18n/sysmon.tsv` (m1, `extract_rs.py en`); `host/i18n/disks.tsv` unchanged in the end (m3)
* `tests/i18n/BASELINE-CHANGES.rust-r2b` (+ process.rs#59, index.rs#184 reason, disks reason), `tests/i18n/allow-host-r2b.txt` (n1)
* `tests/build/r2b-debug/` (rebuilt), this note

### 9.4 Decisions for the coordinator (updated)

1. **Harness patch** (unchanged, M2): `RT = Path('/tmp/w11-rt')` and the assert `< 100` in `tests/gnome/harness.py`;
   no workaround without it exists here (nested bwrap is refused). Then the four commands of decision 1 with the
   rebuilt `tests/build/r2b-debug`.
2. **`disks.unavailableBody`**: keep `#@ new` + `key:` (no Dutch change, stated in the reason), or add a
   "copy of a frozen literal" mapping kind to drift_rust.py that skips `key_used` (:862) and the "still present"
   warning for that line. Then the row can carry `#@ host/src/services/disks/data.json 66 05e022cb923e9fb6 …` and
   the nl render test covers it.
3. GSD/GVFS part names as text.json `{nl, en}`: accepted by the review (n5).
4. `own_names` vs `window.title.*`: round 3.
5. Gate §5 `dm:today` sentence: correct it (n5).
6. GE1a's `ge1a-disks-race.py`: retire it for `r2b-disks-race.py`, or set its `HOST_TEXT['en']` to
   "UDisks isn’t available on this computer.".
7. Round 3 for Robin: the "(ApiError)" leftover in `storage.trashFailed`'s Dutch (held back), the regex details in
   English inside a Dutch message (n1), the D-G23 apostrophes of the `_STR` tables with the disks oracle (n4).
