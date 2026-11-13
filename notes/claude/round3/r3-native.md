# R3-NATIVE (round 3, native side): host fixes, Task Manager always-on-top, harness hardening D-G42, README.en

Worktree `.wt/r3-native` (branch `r3/r3-native`, from 18c8ffbe). Start check: `git status --short` empty, HEAD
18c8ffbe706d. Nothing committed. Scratch files only under `.rt/r3-native/` (logs, the repro runner, wrappers,
format proofs). Every heavy command went through `tools/heavy.sh`; nothing touched the live session (no live bus,
no `win11-*` outside the sandboxes, no gsettings/dconf/apt/snap/pkexec).

## 1. Files changed

| File | Change |
|---|---|
| `host/src/settings.rs` | `settings.import`: a language that `settings.set` would refuse (D-G24) is dropped and the current one stays (D-G31); test `an_import_drops_an_invalid_language_and_keeps_the_current_one` |
| `host/i18n/core.tsv` | the 4 en rows with ’ (`core.noInput`, `core.referenceOffline`, `core.referenceUnreachable`, `core.unknownMethod`), through `extract_rs.py en … --apply` |
| `tools/i18n/check_rust.py` | rule `apostrophe` (error): no straight `'` in an en row (D-G23) |
| `host/src/runtime.rs`, `window.rs`, `i18n.rs` | rustfmt only; plus, in runtime.rs, the 4 test expectations of the core rows (deviation 1) |
| `tools/rustfmt-check.sh` (new) | `rustfmt --check` over every host .rs file, with the 31 files that were not clean at 18c8ffbe listed as exceptions |
| `host/src/files/read.rs` | `current_theme()` reads the icon theme at most every 2 s under one lock (`read_theme()` is the old body); a test-only `.init_array` set-up of GIO's process defaults; test `the_icon_theme_is_read_once_for_many_threads_and_the_catalogue_is_shared` |
| `host/src/files/mod.rs` | `Files::busy()`, `IDLE_EVENT = "files.idle"` (local, once per armed wait), `for_tests`/`job_for_tests` helpers; test `busy_arms_one_local_files_idle_event_for_the_end_of_the_last_job` |
| `host/src/lib.rs` | the `shell.restartAll` receiver's Explorer branch: `defer_while_busy` + `watch_files_idle`; test `a_restarting_explorer_waits_for_its_file_operation_and_restarts_on_files_idle` |
| `tests/rust-sandbox/run-tests.sh` | `GIO_USE_VFS=local` in the `env -i` list |
| `extension/extension.js` | `TaskManagerOnTop` (D-G21), `alwaysOnTopOf()`, `LanguageWatch` keeps the parsed file and calls `onSettings`; en group "Win11 Taskbar" |
| `vendor/ui/js/apps/taskmgr/pages/settings.js` | switch enabled, note dropped (nl/en) |
| `vendor/ui-sha256.json` | `tools/ui-manifest.py` (1 changed) |
| `tests/i18n/BASELINE-CHANGES.ext` | one R3-NATIVE behaviour line (always-on-top) |
| `tests/gnome/harness.py` | D-G42 layer, socket masking and guard, `--strace`, `strace-check`, socket cleanup, D-G44 seams, `GIO_USE_VFS=local`, RT in the sandbox's /tmp (deviation 2), 15 new self-test cases |
| `tests/gnome/baseline/guard-selftest.json` | rewritten by the self-test run (its own output) |
| `tests/gnome/scenarios/r3-ontop.py` (new) + `r3-ontop.out/`, `r3-ontop.en.out/` | the always-on-top scenario and its two runs |
| `tests/gnome/scenarios/r3-cross-r1.out/`, `r3-ge1c-extension.out/`, `r3-cross-r1.group.out/`, `r3-ge1c-extension.group.out/` | runs of the existing scenarios in the hardened harness |
| `tests/gnome/scenarios/example.py` + `example.out/` | C6 polled, every check hard |
| `tests/update-ui/shoot.sh` | an unknown surface id is a failed shot |
| `README.en.md` (new), `README.md` | English README; README.md: a link and a Dutch section "Taal" |
| `notes/claude/round3/r3-native.md`, `r3-native-baseline.txt` | this note; what the coordinator re-records in tests/i18n |

## 2. As built

**settings.import (D-G31).** `import()` drops `language` from the sanitised file when it is not exactly "nl"/"en"
(also "", "EN", an object, a number, null, a list), and inside `apply` (under the store lock) puts back the
current stored language when that is valid; a hand-edited current value that is no language leaves the default
(nl, what C1 reads anyway). A file without a language still gets the default, as before. The test covers 7 bad
values from en, JSON text from nl, a valid import and a missing key.

**Apostrophes (D-G23).** check_rust's rule `apostrophe` errors on `'` in any en catalogue row. Proof: on a scratch
root with `This can't be done.` it reports `[apostrophe] host/i18n/core.tsv:2: core.x: …`; on the tree 0 errors.

**rustfmt.** runtime.rs, window.rs, i18n.rs formatted. Proof that nothing else changed: rustfmt applied to the
18c8ffbe copies (runtime.rs with the 4 test strings) gives byte-identical files (`cmp`, `.rt/r3-native/fmtproof`).
`tools/rustfmt-check.sh` runs rustfmt --check on host/src/lib.rs and every host/src/bin/*.rs (rustfmt follows the
`mod` declarations), maps each `Diff in` to its file and fails for any file outside the frozen exception list.
New files are therefore checked too; a listed file that becomes clean is named. My edits in the four files that
were clean (lib.rs, settings.rs, files/mod.rs, files/read.rs) are formatted (on a copy, so no child module was
touched).

**Task Manager always-on-top (D-G21).** `LanguageWatch` (the C1 watch on `~/.config/win11-gnome`, WATCH_MOVES,
100 ms debounce) now keeps the parsed document and calls `onSettings(settings)` after every read; the language
callback is unchanged. `TaskManagerOnTop` watches `window-created` and every existing window
(`list_all_windows`), re-checks on `notify::wm-class` and `shown`, and for WM class
`io.github.win11shell.TaskManager` while `taskmgr.alwaysOnTop === true` records `{above, sticky}` and calls
`make_above()`/`stick()`; off (or disable) restores exactly those (`unmake_above()` only if it was not above,
`unstick()` only if it was not sticky). Only public `Meta.Window` API of GNOME 50. No new file (C7), no new text.
A maximised Task Manager stays on top too (r3-ontop C, both runs: stack 3 above Explorer's 1), so the description
needs no "not while maximised" (3b898e6's text had that for the Hyprland session).

**Ctrl+Alt+Tab group.** en "Win11 Taskbar" (the window title and the desktop entry's name); nl unchanged.

**Harness D-G42** (`tests/gnome/harness.py`):
* `root_argv()`: `/usr` + the /bin, /sbin, /lib, /lib64 links, `/etc`, `/sys`, `/var/cache/fontconfig`, `/run/udev`
  (deviation 3), the checkout, all read-only; `--dev` with the GPU nodes; `--proc`; tmpfs `/tmp`, `/run` (with empty
  `/run/dbus`, `/run/user`, `/run/systemd`) and `/var` (`/var/run -> /run`, `/var/tmp`); `--remount-ro /`.
  Writable: `tests/gnome/run/` and the run's output directory (plus `<out>/../switch` for a built-in `--switch`
  run). `--source <rev>` adds the repository's git directory read-only (git archive). No /opt (not needed).
* `host_sockets()`: every bound path socket of `/proc/net/unix` (read in `launch`, outside), by real path, that the
  binds would show; each gets `--ro-bind /dev/null`. A host socket inside run/ is refused
  (`host-socket-in-run-dir`: prepare() empties run/). The list goes in as `WIN11_GE0C_MASKED`.
* Inner guard (`sandbox_problems`): `not-sandboxed-run` (a /run directory not empty), `not-sandboxed-root` (the
  real home's .config/.local/.cache, /var/lib or /var/snap visible), `not-sandboxed-no-mask-list`, and
  `host-socket-visible` when a listed path is still a socket or the checkout (outside run/) holds a socket file.
* `run --strace FILE` (FILE inside the output directory): the GStreamer warm-up and the whole session under
  `strace -f --seccomp-bpf -e trace=connect`, the install's own strace gets `connect` too; the run fails when a
  connect that succeeded (or ended unseen) reached a socket that is not the sandbox's own (run/, its tmpfs /tmp,
  /run, /var, minus the bound host directories), an abstract name or loopback. `harness.py strace-check FILE…`
  re-checks logs. (A first version traced bwrap from outside; that blocks the install's own strace, so it moved
  inside.)
* End of `run`: socket files left in run/ are removed (every run so far: `cache/ibus/db`, ibus's private bus whose
  path is cut at 108 bytes), then the whole checkout is scanned; `socketCleanup` in results.json, a left socket
  fails the run.
* Self-test: 110 → 125 cases. New: 5 env cases (GIO_USE_VFS unset/gvfs, tool dir unset, fixture root elsewhere,
  PK bus set; each in-process and in the sandbox) and 5 socket cases with a listening decoy in the checkout
  (`tests/logs/d<pid>.s`): a direct connect reaches it (1); inside the sandbox it is masked (not a socket, connect
  refused, 0 connections) with /run/dbus, /run/user, /run/systemd empty, no real home, no /var/lib or snapd
  socket; unmasked ("appeared after the scan") the guard refuses before any connect; no mask list is refused;
  decoy total 1.

**D-G44 / D-G45.** Every harness process gets `GIO_USE_VFS=local`, `WIN11_UPDATE_TOOL_DIR=run/update-tools` (0700,
marker, a logging stub per name of the paths.rs Tool table, all failing with 97 into
`logs/update-tool-calls.log`) and `WIN11_UPDATE_FIXTURE_ROOT=run/update-fixture` (0700, marker, `proc -> /proc`,
update-notifier's compiled schema from `tests/update/fixtures/options/schemas`, nothing else). The guard refuses
other values (`gio-vfs-not-local`, `update-seam-not-private`, `update-seam-extra`). The Settings update page text is
byte-identical to cross-r1.a2's; `updateToolCalls` is [] in every run. `Files::busy()` arms one local
`files.idle` (emit_local: observers and this process's pages, never other processes) for the end of the last
`files.*` call in flight; the Explorer receiver parks its restart and runs it on that event, or at once when
nothing runs. A one-line stderr note is written; no `shell.restartDeferred` (restart.rs is not mine).

**The intermittent unit-suite hang.** Not reproduced: 18c8ffbe gave 20/20 sandboxed lib runs (407 passed each),
then 300 runs of `apps:: app_params files::` with 32 threads and 250 runs pinned to 2 CPUs, each a cold process,
with a runner that would have attached gdb at 45 s (`.rt/r3-native/hang-runner.py`): 0 hangs. The shared cause,
from the three recorded hangs (R2-D `hang-1.txt`, GX1 `unit1-hung-test.log`, GW4): the 16 stuck tests are always
the first users of GIO's process-wide lazily created defaults — app_params and files::actions through
`gio::File::for_uri` (g_vfs_get_default), apps::catalog, files::places and files::properties through
`icon_catalog()` → `current_theme()` (a new GSettings per call: schema source, the default backend, the keyfile
backend's GFile and monitors), apps::recent through the MIME database — and all are futex-blocked with no
`gdbus` thread, while the other ~80 tests finished. With `GIO_USE_VFS` unset the default VFS is gvfs's
GDaemonVfs, whose construction does session-bus work while GIO holds its default-module lock. Every host sets
these defaults up on its main thread (gtk::init) before worker threads run; the test binary did it from up to 16
threads at once. Fixes: `GIO_USE_VFS=local` in run-tests.sh (no GDaemonVfs), the theme read once per 2 s under
one lock (1 GSettings instead of 16 at the cold start: the new test checks ≤ 1 read for 16 threads), and the test
binary sets up GIO's defaults once on one thread before libtest starts (`.init_array`, test-only). Since the base
did not hang here either, the 20 runs after the fix show no regression, not a proven cure.

## 3. Verification (from W's root)

| Check | Command | Result |
|---|---|---|
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag r3-native-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **411 passed / 17 ignored / 0 failed** (407 + 4 new), win11-launch 16 passed, the other bins 0; oracles green (shell `oracle_tests` 8, the 7 disks python cases, `reference_contracts`), `every_converted_site_renders_its_3b898e6_dutch` ok; `guard-calls.log` **0 bytes**; warnings lib 28, win11-launch 2 (baseline) |
| 20 lib runs, fixed | `tools/heavy.sh --min-mb 2048 --tag r3-native-repeat -- .rt/r3-native/repeat.sh repeat-fixed 20` (run-tests.sh --lib each, hang runner) | **20/20** rc 0, 411 passed each, 6.8 s each, guard 0, no hang |
| 20 lib runs, base | same on the unchanged tree (`repro-base`) | 20/20, 407 passed, no hang |
| Stress, base | run-tests.sh `--lib --config runner=[hang-runner --repeat 300] -- apps:: app_params files:: --test-threads=32`; then 250 × with `--cpus 0,1` | 300/300 and 250/250, 0 hangs |
| drift_rust | `python3 -B tools/i18n/drift_rust.py --warnings` | 0 errors, 2 warnings (the same 2 at HEAD: update/options.rs, update/paths.rs); `--new --paths` my files: 0 errors, 0 warnings |
| check_rust | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings (72 at HEAD) |
| rustfmt | `tools/rustfmt-check.sh` | 142 files, 111 clean, 31 listed, **0 failing**; on an export of HEAD: 3 failing (runtime, window, i18n) |
| check.py | `PATH=… python3 -B tools/i18n/check.py` | **0 errors**, 124 warnings (glossary 52, rust 72); readme rule passes, and fails on a copy with one fenced block changed |
| UI dict diff | `check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv --changes .rt/r3-native/BASELINE-CHANGES.ui.merged --quiet` | 0 not listed, 0 stale (with the two lines of r3-native-baseline.txt; without them onTopDesc is NOT LISTED) |
| Extension text | `python3 -B tests/installer/test_extension_text.py` | 6/8: the 2 failures are the asserted "Win11 taskbar"; a copy with those 2 strings changed: **8/8** |
| Guard self-test | `python3 -B tests/gnome/harness.py guard-selftest` | **125/125** |
| r3-ontop nl | `tools/heavy.sh --min-mb 2560 --tag r3-native-ontop -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/r3-native-debug --source worktree --scenario tests/gnome/scenarios/r3-ontop.py --out tests/gnome/scenarios/r3-ontop.out --strace tests/gnome/scenarios/r3-ontop.out/connect.strace` | rc 0, **31/31** (nl then en), 145 s, peak PSS 1159 MiB; strace 443 connects OK; socket cleanup 1 removed, 0 left; update tool calls 0 |
| r3-ontop en | same, `--lang en --out …/r3-ontop.en.out` | rc 0, **31/31** (en then nl), 140 s, peak PSS 1168 MiB; strace 444 connects OK; socket cleanup 1/0; update tool calls 0. (Its first run, 01:06, also 31/31, exited 1 only because the first connect rule counted an unfinished connect to the sandbox's empty `/var/run/nscd/socket`; rule fixed, logs re-checked OK, rerun) |
| cross-r1 | `… launch run --lang nl --build-dir tests/build/r3-native-debug --source worktree --surfaces desktop,panel,start,explorer,settings,taskmgr,alles,disks --scenario tests/gnome/scenarios/cross-r1.py --out tests/gnome/scenarios/r3-cross-r1.out --strace …/connect.strace` | 15 surfaces ok, **125/133**: all 8 failures are `ext.group` en (C-en, E-EN-back, E-object-back, E-back, 2 each: "Win11 Taskbar" ≠ the scenario's "Win11 taskbar"); 296 s, peak PSS 1111 MiB; strace 1258 connects OK (re-checked with the final rule); socket cleanup 1/0; GPU renderer as before (`gbm renderer for /dev/dri/renderD128`), page pixel stats equal to cross-r1.a2 except clock/content |
| cross-r1, group expectation updated | `.rt/r3-native/wrap/cross_r1_group.py` (loads cross-r1.py, sets `EXT['en']['group']`) → `r3-cross-r1.group.out` | rc 0, **133/133**, 15 surfaces ok, 256 s, peak PSS 1164 MiB; socket cleanup 1/0 |
| ge1c-extension | `… --scenario tests/gnome/scenarios/ge1c-extension.py --out tests/gnome/scenarios/r3-ge1c-extension.out` | rc 0, **58/64**: all 6 failures compare the en group (4 group checks, `c1-back` and the fallback check via `labels_ok`) |
| ge1c-extension, group updated | `.rt/r3-native/wrap/ge1c_extension_group.py` → `r3-ge1c-extension.group.out` | rc 0, **64/64**, 132 s, peak PSS 971 MiB |
| example.py | `… --scenario tests/gnome/scenarios/example.py --out tests/gnome/scenarios/example.out` | rc 0, **30/30** (no check marked known; the last one requires every other to pass), 141 s; C6 polled: all 10 entries English |
| shoot.sh | `tools/heavy.sh --tag r3-native-shoot -- tests/update-ui/shoot.sh settings.no-such-surface` | exit 1, "0 shots, 1 failed (1 unknown surface ids)" |
| Build snapshot | `tools/heavy.sh --tag r3-native-build -- cargo build --offline --locked -j2`, copied to `tests/build/r3-native-debug` | 12 binaries, `sha256sum -c` 12 OK, `cmp`-equal to target/debug |

## 4. Deviations

1. **runtime.rs** has 4 changed test strings besides rustfmt: its tests assert the English of the 4 core rows,
   which now use ’. Without them the suite fails.
2. **RT moved to `/tmp/w11-rt` inside the sandbox** (was `tests/gnome/run/rt`). From `.wt/r3-native` RT/bus was 100
   bytes, over dbus-daemon's 99 (the first run stopped at prepare()'s assert); the sandbox's /tmp is its own tmpfs,
   so no runtime socket can be left in the checkout either. Scenarios use `h.hm.RT`, so nothing else changed.
3. **`/run/udev` read-only inside the private /run** (mutter finds the render node through udev's seat tag; its two
   sockets are masked like every other host socket). The task listed only empty dbus/user/systemd under /run.
4. The en group name breaks 4 expectations in files I do not own (cross-r1.py, ge1c-extension.py,
   tests/installer/test_extension_text.py ×2); I left them and ran wrapped copies (section 3).
5. `tests/gnome/README.md` is not mine: its "What it guards" section still describes `--ro-bind / /` and a writable
   tests/gnome; the harness docstring is updated.
6. D-G31 says "the current one stays, as the mock and H do"; H (`tests/test_i18n.py:1068`) and the mock drop the bad
   value to the **default** (nl), not the current language. I implemented the task's "keeps the current one".

## 5. Decisions for the coordinator

1. Apply `notes/claude/round3/r3-native-baseline.txt`: the two BASELINE-CHANGES.ui lines (replacing GE1a's 67-68),
   the re-record of `taskmgr.settings` in nl and en, and the one-string updates of the group name in cross-r1.py,
   ge1c-extension.py and test_extension_text.py.
2. D-G31: keep "current language stays" (then R3-UI's mock should do the same) or take H's default (remove the
   restore in `import()`, 4 lines).
3. Accept the test-only `.init_array` GIO set-up in files/read.rs, or drop it and rely on `GIO_USE_VFS=local` and the
   theme cache alone (the hang never reproduced here, so I cannot rank the three).
4. `gx1-restart.py` sets and then pops `WIN11_UPDATE_FIXTURE_ROOT` itself (its fixture is the same
   `run/update-fixture`); after its pop, later launches of that scenario run without the harness seam. It should
   restore the previous value instead (not my file; not run here).
5. `files.idle` deferral is not announced (`shell.restartDeferred` lives in restart.rs); Settings' InfoBar does not
   say that Explorer waits for a copy. Add `Wait::Files` there if wanted (GX1 m5's second half).
6. rustfmt-check.sh also checks new files: other steps' new .rs files must be rustfmt-clean (or listed).

## 6. For Robin (not changed)

Unchanged D-G1b items: "Dat lukte niet (ApiError)." (`storage.trashFailed` nl), the English regex details inside
the Dutch index message, the disks tables' apostrophes, "Nederlands (Nederland)", "Afbeelding". Dutch changes of
this step: only `onTopDesc` (D-G21, the note dropped: "Taakbeheer blijft boven andere vensters en is op elk
bureaublad te zien."), and the new Dutch section "Taal" in README.md.

## 7. Left / not verified

* The hang fix is not proven (no reproduction in 20 + 550 base runs).
* Not run in the hardened harness: gx1-restart, ge1a-*, r2b/r2d scenarios, ge1b-perf, and a `--source <rev>` run
  (the git-directory bind is untested).
* `--strace` traces connect() only; datagrams to host sockets are covered by the masking and the guard, not by
  strace.

## 8. Fix round (after the adversarial review, `r3-native-review.md`)

**State check first.** W matched the review's view: 25 modified files (+2994/−1684), the same untracked list,
`tests/gnome/baseline/guard-selftest.json` as the step left it, no socket file in W. Everything below is in W;
scratch (patch files, the run chain, the moved outputs, logs) is in `.rt/r3-native/fix/`. Nothing committed.

### 8.1 Findings

| Id | Status | What changed |
|---|---|---|
| M1 | fixed (listed) | `r3-native-baseline.txt` item 3b: `tests/gnome/scenarios/ge1b-core.py:62` → `'This function doesn’t exist.'`. A grep of every scenario, tests/installer, tests/i18n, tests/update*, tools, vendor/ui/js and host/src for the old en text of each of the 4 changed core rows finds no other expectation (the rest are recorded results and notes). Hardened run, ge1b-core as is: 89/94 (the 4 en C2 checks + the picker step); with the one string changed (`.rt/r3-native/wrap/ge1b_core_apostrophe.py`): **93/94**, the one left is the picker step (n6, not R3-NATIVE). |
| M2 | fixed | The sandbox's `/` is no longer a tmpfs: `--bind run/root /` first, then everything as before, then `--remount-ro /`. `run/root` is an empty directory on the checkout's own file system (bwrap creates the mount points in it; `prepare()` leaves it alone; launch refuses it if it is a link). services/storage/volumes.rs now finds a real file system at `/`: Settings › Storage "Ruimte op je systeemschijf / Systeemschijf (/) … van 233 GB", This PC "Systeemschijf (/)" / "System drive (/)" (cross-r1, nl and en). **Known difference, by design:** the old `--ro-bind / /` layer showed the host's other mounts too (This PC listed 7 drives: 3TB, 22TB, a 563 GB disk, "Virtuele machines (/srv/vm)", BACKUP, extra); the hardened layer shows only the system drive, so harness screenshots of This PC, Storage and Task Manager's disks no longer show Robin's private drives and must not be compared with older ones there (for the round-3 screenshot plan). No product seam was added. Guard self-test case: `/` is ext4, read-only, the checkout's device, `root` = run/root. |
| m1 | coordinator (unchanged) | The group-name strings stay in files R3-NATIVE does not own (baseline item 3a). Hardened reruns: cross-r1 125/133 → wrapper **133/133**; ge1c-extension 58/64 → wrapper **64/64**; test_extension_text 6/8 (8/8 with the 2 strings, step run). |
| m2 | fixed | `SocketWatch` (harness.py): during every run, every 2 s, a scan of the checkout (minus run/) and of the bound host directories (/etc, /run/udev, the font cache, the `--source` git dir) for socket files (directory-entry types decide; 0.04–0.29 s per scan). The first new one stops the session (`SOCKET ABORT`), the run exits 5 and results.json `socketWatch` names it. run/ is not acted on (its sockets are the sandbox's own and cannot be told from a host's without connecting); its socket files that no process of the sandbox's network namespace had bound when first seen are listed as `runUnlisted` (0 in every run). A host program that re-creates a masked socket (which unmasks it) is caught the same way. End to end: new `tests/gnome/scenarios/r3-late-socket.py` + its outside helper `tests/gnome/lib/late_socket.py` (binds a listening socket in tests/logs 118 s into the run, after launch's scan): the watch stopped the session 2 s later, **exit 5, 0 connections**, strace 179 connects OK, the late socket reported as left (the run fails), and it was gone after the run. Self-test case: a socket that appears in a watched directory is found within two scans (0.38 s at a 0.5 s interval) and the stop handler runs once. |
| m3 | fixed | The inner guard also scans the bound host directories for sockets not covered by /dev/null (`host-socket-visible … in the host directory`), and refuses a writable `/`. `--selftest-unmasked` and the new `--selftest-listed-unmasked` are refused unless the command is `launch probe`, `--selftest-mutate` unless it is `probe` or `run --guard-only` (`selftest-option-misuse`, checked before the lock and before bwrap). New self-test cases: a listed but unmasked host socket (the "still a socket" branch) is refused before any connect; an unmasked `/run/udev/control` is refused (the review's probe now fails); the three option misuses are refused. |
| m4 | fixed | `strace_connect_report` resolves each socket path as the kernel would inside: left to right, following the sandbox's own top-level links (`/var/run` → `/run`, `/bin` → `usr/bin` …), then `.`, `..` and `//` folded; a relative sun_path counts as outside. The review's synthetic log now gives 4 violations (`/var/run/udev/control`, `/run/udev/control`, `/run/../home/…`, `/var/cache/../../home/…`), `/tmp/w11-rt/bus` OK. Self-test case with 11 synthetic connects (incl. `/var/run/../home/x.s`, which a plain normpath would call private). All earlier strace logs (step and review) re-check OK. Links deeper in the private tmpfs or run/ are not known offline (said in the docstring). |
| m5 | fixed (as asked) / not proven | The test-only `.init_array` GIO set-up in files/read.rs runs only when `GIO_USE_VFS=local` and `GSETTINGS_BACKEND=keyfile` (the sandboxed suite); `--list`, a plain `cargo test` or the update bus tests skip it. The hang is still not proven fixed: it never reproduced (base 20 + 550, fixed 20 + 20). The stack-capturing runner is `.rt/r3-native/hang-runner.py` (used by `.rt/r3-native/repeat.sh`); putting it into run-tests.sh is the coordinator's (not R3-NATIVE's file). |
| m6 | fixed | The step's `r3-cross-r1*.out` and `r3-ge1c-extension*.out` (outputs of scenarios R3-NATIVE does not own, one with the stale `straceConnect.ok: false`) are moved to `.rt/r3-native/fix/old-step-runs/`; all reruns of not-owned scenarios went to `tests/gnome/fix-runs/` and were moved to `.rt/r3-native/fix/runs/` after each run. In the tree only owned outputs: `r3-ontop.out`, `r3-ontop.en.out`, `r3-late-socket.out`, `example.out` (all rewritten in this round). |
| m7 | fixed | README.en.md: the 11 straight apostrophes in prose are ’ (none in code). Both READMEs: the extension and the installer read `$XDG_CONFIG_HOME/win11-gnome` (normally `~/.config/win11-gnome`). README.md: "… klaarstaat (in een Engelstalige GNOME: “X” is ready)" (no Dutch GNOME translation is installed here to quote; new Dutch README text is allowed). check.py's readme rule still passes. |
| m8 | fixed | Every scenario the harness runs today was run in the hardened layer with this round's build (§8.3). |
| m9 | partly (warning) | check_rust's `apostrophe` rule also walks host/src's JSON string tables (the en side of `{nl, en}` objects and `[nl, en]` pairs) and **warns**: 28 (disks/data.json 21, marked "D-G1b: for Robin"; storage/data.json 1; system/strings.json 6). Not an error, because those tables belong to their service steps (and disks to Robin): an error would fail check_rust/check.py for text R3-NATIVE may not change. The owners convert, then the rule can become an error. Warnings: check_rust 72 → 100, check.py 124 → 152. |
| n1 | coordinator | D-G31 unchanged (current language kept). |
| n2 | not done | No end-to-end copy-during-restart case and no `Wait::Files` (restart.rs is GX1's successor's); the deferral stays unit-tested. gx1-restart 56/56 in nl and en shows the receiver path with nothing in flight. |
| n3 | fixed | `--out` that is tests/gnome itself or contains run/ is refused (`out-invalid`, launch and run); 2 self-test cases. |
| n4 | for Robin | nl group "Win11-taakbalk" vs nl window title "Win11 Taakbalk": listed in §8.4 (Dutch stays frozen). |
| n5, n6 | not mine | tests/gnome/README.md (old layer description) and ge1b-core's picker match: coordinator / their owners. |

**Found in this round (F1, fixed):** the step's `launch` bound the default output directory with a *relative*
path when `--scenario` was given without `--out` (the usage most scenario docstrings show), so
`<scenario>.out` was read-only inside the sandbox and such a run crashed at its end (first r3-late-socket attempt:
"Read-only file system" on connect.strace). `scenario_out()` now returns an absolute path; r3-late-socket (no
`--out`) ran with it.

**Build.** files/read.rs changed (test-only code and comments), so `cargo build` relinked the binaries; 8 of 12
differ from the step's snapshot (the later line numbers in read.rs moved, which debug info and panic locations
carry). The new build is in `tests/build/r3-native-debug` (12 binaries `cmp`-equal to target/debug, `sha256sum -c`
12 OK); the step's snapshot is kept in `.rt/r3-native/fix/r3-native-debug.step/`. Every run in §8.3 except the
first r3-ontop (step snapshot) used the new one.

### 8.2 Files changed in this round

`tests/gnome/harness.py` (M2 root, m2 SocketWatch, m3 guard/options, m4 paths, n3, F1, probe facts, 10 new
self-test cases, docstring), `tests/gnome/baseline/guard-selftest.json` (self-test output), new
`tests/gnome/lib/late_socket.py` and `tests/gnome/scenarios/r3-late-socket.py` (+ `r3-late-socket.out/`),
`r3-ontop.out/`, `r3-ontop.en.out/`, `example.out/` (reruns), `host/src/files/read.rs` (m5 gate),
`tools/i18n/check_rust.py` (m9 warnings), `README.en.md`, `README.md` (m7), `notes/claude/round3/r3-native.md`,
`r3-native-baseline.txt` (M1), `tests/build/r3-native-debug/` (rebuilt). Moved out of the tree: the 4
not-owned `r3-*.out` directories (m6).

### 8.3 Done-when, rerun (from W; all heavy commands through `tools/heavy.sh`, chain `.rt/r3-native/fix/chain.sh`)

| Check | Result |
|---|---|
| Unit suite `tools/heavy.sh --min-mb 2048 --tag r3-native-fix-unit -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **411 passed / 17 ignored / 0 failed**, win11-launch 16 / 1 ignored, other bins 0; `guard-calls.log` **0 bytes** |
| 20 lib runs (`.rt/r3-native/repeat.sh fix-repeat 20`, run-tests.sh --lib with the hang runner) | **20/20** rc 0, 411 passed each, 6.8–6.9 s, guard 0 lines, no hang |
| drift_rust / check_rust | 0 errors (2 warnings) / **0 errors** (100 warnings: 72 + 28 m9), apostrophe rule on |
| `tools/rustfmt-check.sh` | 142 files, 111 clean, 31 listed, **0 failing** |
| `tools/i18n/check.py` | **0 errors**, 152 warnings; readme rule passes |
| `tools/ui-manifest.py --check` | up to date |
| Guard self-test (final, after every harness change) | **135/135** (125 + 10) |
| r3-ontop nl / en (`--build-dir tests/build/r3-native-debug --strace …`) | **31/31** / **31/31**; strace 442 / 441 connects OK; socket cleanup 1 removed, 0 left; watch 20 scans, 0 appeared; update tool calls 0 |
| r3-late-socket (m2) | exit **5** as designed: watch stopped the session 2 s after the late bind, 0 connections, strace OK |
| example.py | **30/30** |
| cross-r1 / with the group string | 125/133 (8 = group) / **133/133**; strace 1257 / 1163 OK; drive texts as in §8.1 M2 |
| ge1c-extension / with the group string | 58/64 (6 = group) / **64/64**; strace 373 / 317 OK |
| ge1b-core / with the ’ string | 89/94 / **93/94** (picker step, n6); strace 1041 / 1050 OK |
| gx1-restart nl / en | **56/56** / **56/56**; strace 835 / 853 OK |
| ge1a-start-switch, ge1a-disks-race, r2b-disks-race, r2d-picker, ge1b-perf | **16/16, 7/7, 9/9, 16/16, 2/2** (strace OK on all but ge1b-perf, which runs without it for its timings) |
| ge1a-switch (2 runs with `--strace`) | 59/61 twice: the one check "en: Start opened 0.1 s after a rename is en in every frame from 1 s after opening on" (+ the summary check). The Start page's own texts are English in every frame; the app tiles' names (Instellingen, Taakbeheer, Teksteditor, Verkenner, from the host catalogue's 150 ms poll) were still Dutch in the recorder's last frame (≈570 ms), and the settled check passes. The same check failed the same way in the old layer at phase A1 (`ge1a-switch.a1`, last dirty frame 644 ms; `a1-2` passed): a timing race of the scenario |
| ge1a-switch, third run without `--strace` | **61/61** (the tiles were English from 331 ms): strace's stops make the race likelier; not a layer regression. Use ge1a-switch without `--strace` |
| `--source 18c8ffbe` + example.py, base-debug | **30/30**, strace 619 OK |
| Sockets | `find W -type s` after every step: **0**; every run removed exactly `cache/ibus/db` from run/ and left 0 |
| Update service | `updateToolCalls` [] in every run |

### 8.4 Left, decisions, for Robin

* **Left / not verified:** the hang fix is still not proven (it never reproduced); no end-to-end case of a copy
  during `shell.restartAll` (n2); tests/gnome/README.md still describes the old layer (n5, not mine); the JSON
  tables' straight apostrophes are warnings only (m9); a scenario killed by the socket watch or the memory abort
  leaves its checks only in harness.log (inner-results.json is not written).
* **Decisions for the coordinator:** (1) apply `r3-native-baseline.txt` (now with ge1b-core.py:62, item 3b);
  (2) D-G31, as before (n1); (3) keep the gated test-only GIO set-up or drop it; move
  `.rt/r3-native/hang-runner.py` into run-tests.sh if wanted (m5); (4) the drive list of harness screenshots is
  now the sandbox's (only "System drive (/)"), so This PC/Storage/Task Manager disk shots must not be compared with
  shots from the old layer (M2); (5) the service owners convert the JSON tables' apostrophes (system, storage), then
  check_rust's JSON apostrophe warning can become an error; (6) the earlier §5 items 4–6 stand (gx1-restart's
  fixture-root pop, `shell.restartDeferred`, rustfmt-check on new files); (7) `ge1a-switch` is best run without
  `--strace` (its 0.1 s fast-start check races the host catalogue's poll).
* **For Robin (not changed, Dutch frozen):** besides §6's list, the extension's nl Ctrl+Alt+Tab group
  "Win11-taakbalk" differs from the nl window title "Win11 Taakbalk" (review n4).
