# GX1: restart all Win11 processes (`shell.restartAll`), 2026-11-05

Worktree `.wt/gx1` (branch `wu/gx1`, from f01710cc). Design: `notes/claude/windows-update-gnome.md` §5, §4.9 and the
GX1 row of §10; decisions D-G25..D-G29, D-G38..D-G42 of `plan.md`; contracts C8 and C10. Start check: `git status
--short` empty, HEAD f01710cc. Nothing committed. No real system bus or live session bus was contacted, no
PackageKit/apt/pkcon/gsettings/installer ran on the real system, no win11-* binary ran outside the sandboxes, nothing
was signalled. Every heavy command went through `tools/heavy.sh`.

## 1. Files

| File | What |
|---|---|
| `host/src/services/shell/restart.rs` (new, ~330 lines + tests) | `shell.restartAll` on the origin side (argument allowlist, `guard::restart_all_blocked`, dry-run, the event), the receiver side registered in `NativeCore::new` (per-kind plan and delay, the prewarmed-Start rule, the portal exit, the re-check before the exec, deferral with event resume and a pidfd watch, `shell.restartDeferred`), 10 unit tests |
| `host/src/services/shell.rs` | `mod restart;`, the `restart` state field, `watch_restart_all()` in `build(start=true)` (= `NativeCore::new`), the `shell.restartAll` route (5 lines) |
| `host/src/lib.rs` | the `" (deleted)"` strip (`exec_path`) and `/proc/self/exe` fallback, the argv override (`restart_args`), `reexec` for GUI and `--http-only` hosts, the restart plan (`RestartAll`, `prepare_restart_all`/`take_restart_all`), the main-thread restart (`restart_on_main`: Explorer closes its windows like a user, the others quit), `quit_when_idle` (file portal), the Explorer restart lock handed to the new image (`explorer_handoff`/`take_handoff`), the reaper of inherited children, the panel's bounded wait for its prewarmed Start, 3 unit tests |
| `vendor/ui/js/mock/shell.js` | `shell.restartAll` in mock mode (GW3 D3): the guard on the mock's update and software state with the host's texts, the event, the mock host restart, then the InfoBar's `restart.shell` cleared |
| `vendor/ui-sha256.json`, `vendor/ui-patches.json` | regenerated with `tools/ui-manifest.py` |
| `tests/gnome/scenarios/gx1-restart.py` (new) | the harness scenario (A–F below) |
| `tests/gnome/scenarios/gx1-restart.out/`, `gx1-restart.en.out/` | results.json + logs (text, committed); 8 PNGs each (excluded from git, D-G22) |
| `notes/claude/wu/gx1.md` | this note |

Not changed: `host/src/bin/file_portal.rs` (the portal runs a `Runtime`, so it already gets `NativeCore::new`'s
handler; the exit lives in lib.rs, see D4), `host/i18n/shell.tsv` (no new text: refusals are `update.guard.*`,
a wrong reason is `shell.badArgs`), no BASELINE-CHANGES/allow file (no Dutch change, no new user-visible literal).
Build snapshot: `tests/build/gx1-debug/` (12 binaries + `SHA256SUMS` in a1-debug's format; gitignored).

## 2. As built

**Origin.** `shell.restartAll {reason:'libraries'|'update'}` (no other key) → `bad_args`/`shell.badArgs` otherwise;
then `guard::restart_all_blocked(ctx)` (files and /proc only) → its `ApiError` (`update_busy` +
`update.guard.upgrade`/`update.guard.restart`, `software_busy` + `update.guard.software`, `test_seam`); dry-run →
`{ok:true, dryRun:true}` and no event (the guard runs first, so a dry host still shows a refusal); a stopping host →
`shutting_down`. Otherwise `emit("shell.restartAll", {nonce, origin: pid, at: ms, reason})` → `{ok:true, nonce}`.

**Receivers** (every `NativeCore::new`: all GUI hosts, `--http-only` hosts and the portal's runtime). A dry-run host
ignores the event. One restart per process (later events are ignored until the exec).

| Kind | Plan |
|---|---|
| desktop | exec after 0 ms |
| explorer, taskmgr, alles, disks, defaultapps, installer, a Settings that is not the origin | 300 ms |
| panel; a Start that is **not** the panel's child | 1200 ms |
| the origin (any kind, in practice Settings) | 2000 ms |
| a Start whose parent's exe is `win11-panel` (the prewarmed one, also when the panel's binary is "(deleted)") | ignore |
| picker (= win11-file-portal) | exit at once when no file dialog is open (D4) |

Just before the exec the receiver re-checks (`restart::wait_for`): `update/job.json` without `finishedAt` with a live
owner (the guard's rule), then its **own** Software jobs (`_owner.pid` = this process, state
queued/running/authenticating, owner alive; private files only, `store::read_private`). If one is active it stores
the pending restart, announces it once (`shell.restartDeferred {nonce, pid, kind, wait:'update'|'software'}` + one
stderr line) and waits: `software.changed`, `update.changed` and `update.owner` resume a re-check; an update owner in
another process is also watched through a pidfd (a crashed owner sends no event; a pidfd that reported the exit
counts the owner as gone even while it is a zombie). After storing the deferral it checks once more, so a job that
ended in between is not missed. No poll anywhere.

**The exec** (`lib.rs`). `restart_now` asks the extension for an activation token for app kinds
(`launch::activation_token`, best effort), stores `RestartAll {origin, token}` and calls `request_shutdown(true)`.
On the main thread (`restart_on_main`): Explorer closes its windows like a user does (`win11:close` → the page sends
its session → window.rs closes 200 ms later; `app.quit()` after 3 s if a page hangs); every other kind quits at once;
an app host without any window (Settings kept alive by its update job, design C2) does not come back. After
`run_with_args`: Explorer takes the restart lock, the window state is flushed, `runtime.shutdown()`, the panel waits
≤ 2 s for its prewarmed Start to exit, then `reexec`: `exec_path(current_exe())` (strip a trailing `" (deleted)"`;
`/proc/self/exe` when that path is gone), argv from `restart_args` (origin Settings: `--app settings [--dry-run]
--params {"page":"update"}`; Explorer: only `--dry-run`/`--http-only`, so the page restores the saved session instead
of an explicit location; every other host its own argv), the stale activation token removed and the new one set,
Explorer's lock fd (close-on-exec cleared) passed as `WIN11_RESTART_HANDOFF_FD`. A plain `shell.restart` keeps its own
argv but now also gets the `(deleted)` fix.

**New image.** `run()` takes the handed-over lock first (before any thread; only if the fd's device and inode are
the lock file's, then close-on-exec again) and drops it right after `WindowState::open`, so concurrent Explorers
flush and read the one `apps.explorer` window-state slot one at a time and each restores its own tabs. Then it reaps
the children it inherited (`/proc/self/task/<pid>/children`, a /proc scan otherwise): the old image's WebKit processes
and the old prewarmed Start, which exit after the exec and would otherwise stay zombies of the new image.

## 3. Deviations from the design and readings (for the coordinator)

* **D1 guard name.** §5 says `guard::power_blocked`; the task and §4.9 say `restart_all_blocked`, which is what is
  called (power_blocked has no update-job-any-phase or Software rule).
* **D2 delays not in §5.** The origin goes last whatever its kind; a Settings that is not the origin is an app
  (300 ms) and re-execs with its own argv (only the origin reopens on `update`); a Start outside the panel restarts
  with the panel (1200 ms).
* **D3 Explorer needs more than "its argv".** (a) `app.quit()` does not fire `win11:close`, so a quit Explorer would
  flush a stale session: Explorer closes its windows instead. (b) An explicit location (path, `--params`, a used
  `--tab-ticket`) wins over the saved session in `explorer/main.js start()`, so the restart argv drops them.
  (c) All Explorer processes share one window-state slot; without the handed-over lock a second Explorer would restore
  the first one's tabs. Not covered: a file copy in progress keeps that Explorer windowless until the copy ends
  (`Files::shutdown` waits, as on any restart today).
* **D4 file portal.** "Exits on the event" is implemented as: exit when no file dialog is open; with a dialog open
  it stays and ends through its own idle exit (15 s after the last request), so an open dialog of another app is never
  cancelled. Done in lib.rs (`quit_when_idle`) through the portal's `NativeCore`, not in `bin/file_portal.rs`.
* **D5 additions.** Activation tokens for restarted app windows (no "is ready" banner; the origin maps last and ends
  in front; harness: `notificationsAtEnd` empty). The reaper (a latent zombie leak of every `shell.restart` today).
  The panel's ≤ 2 s wait for the old Start (one Start; the application id is free before the new panel prewarms).
  The `shell.restartDeferred` event (observability; the page does not use it).
* **D6 "the origin shows its InfoBar again".** No extra signal: the re-exec'd Settings' update engine rescans at
  start and finds the deferring receiver's stale mappings, so the InfoBar is back (harness E: checked). It then stays
  until the update engine's next rescan (GW2 rules: forced only at engine start and after a check), also after the
  deferred receiver restarted. See C2.
* **D7 mock.** `shell.restartAll` answers `{ok, nonce, dryRun:true}` (the mock is a dry host; same as mock
  `shell.restart`) and still simulates the restart: the mock host goes away after 2 s for 1.5 s, then
  `restart.shell` of mock/update.js's state is cleared with an `update.changed`. Refusals have code + message only
  (mock/index.js drops key/params, D-G41). Software jobs are read through `m.call('software.jobs')`.
* **D8 harness guard log.** "Empty" read as "no stub call outside the read-only calls every harness session makes"
  (`wpctl get-volume`, `pw-dump --monitor`, `systemctl --user …` of Taakbeheer, `apt list --upgradable` of the update
  page without a snapshot); the scenario fails on any other call. The rust-sandbox `guard-calls.log` is empty (0 lines).
* **D9 harness seam.** This PC has `/run/reboot-required` (visible read-only in the sandbox), and a pending reboot
  hides the Win11 restart InfoBar (§3.4). The scenario starts Settings with `WIN11_UPDATE_FIXTURE_ROOT` (marker file,
  an empty `run/`, `proc → /proc`; no bus variable, so the update service opens no bus at all, C10). Only the scenario's
  Settings process gets it.

## 4. Verification (from W's root; numbers as measured)

| Check | Command | Result |
|---|---|---|
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gx1-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | exit 0; lib **379 passed / 16 ignored** (366 + 13 new), launch 16 passed / 1 ignored, other bins 0; `tests/rust-sandbox/logs/guard-calls.log` **0 lines**; tmp entries left 0 |
| New tests | (in that run) `services::shell::restart::tests::*` (10) and `restart_tests::*` (3) | all ok: delays and order, the prewarmed Start/portal plans, reason allowlist, panel exe incl. "(deleted)", job rules = guard rules, the re-check (own vs other jobs, finished, pid reuse, non-private file, exited owner), **guard refusal in 4 update-job phases and for a Software job (key + nl/en text), no event**, dry-run, **a receiver defers on its own Software job until the job's event and restarts once, the deferral announced once**, an update owner's crash resumes through the pidfd, a dry host ignores the event, **`(deleted)` strip** (spaces, non-UTF-8, one suffix), **argv override** (origin/second Settings, `--dry-run`, Explorer navigation args dropped, others unchanged), the plan taken once |
| cargo check | `tools/heavy.sh --tag gx1-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, win11-launch 2, lib test 26, launch test 1 (= baseline); **0 warnings in lib.rs or shell/restart.rs** |
| rustfmt | `rustfmt --edition 2024 --check` on restart.rs; lib.rs and shell.rs checked on copies with stub child modules | clean (HEAD's lib.rs was clean too) |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` (and `--paths` lib.rs shell.rs shell/restart.rs bin/file_portal.rs) | **0 errors, 0 warnings**; new literals in my files: 3 logs (English stderr lines) + technical |
| check_rust | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings, none in my files (64 `[unused]` GW4–GW9 rows, 4 `[glossary]`, 4 `[space]`, all pre-existing) |
| Mock | `node --check --input-type=module < vendor/ui/js/mock/shell.js`; `node .rt/gx1/mock-restart.test.mjs` (a copy as .mjs, fake `m`) | ok; **13 cases** (upgrader, 4 job phases en, 3 Software states nl, 2 bad args, check/foreign not blocking, InfoBar cleared after the simulated restart) |
| Manifest | `python3 -B tools/ui-manifest.py` then `--check` | regenerated (1 changed), check exit 0 |
| Build | `tools/heavy.sh --tag gx1-build -- cargo build --offline --locked -j2`, copied to `tests/build/gx1-debug` + `SHA256SUMS` | 12 binaries, `sha256sum -c` ok |
| Harness nl | `tools/heavy.sh --min-mb 2560 --tag gx1-harness -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/gx1-debug --source worktree --scenario tests/gnome/scenarios/gx1-restart.py --out tests/gnome/scenarios/gx1-restart.out` | exit 0, **54/54 checks**, install ok, leftover processes after inner **[]**, peak PSS 1218 MiB, 141.8 s |
| Harness en | same with `--lang en --out tests/gnome/scenarios/gx1-restart.en.out` | exit 0, **54/54 checks**, leftover [] |

Two earlier nl runs failed and led to fixes: run 1 (InfoBar hidden by this PC's pending reboot → D9), run 2 (51/54:
Settings started from the replaced files, so its exec was invisible to the exe-flip sampler → the scenario replaces
the files a second time before the click; and the deferral was announced on every re-check → announced once).

Harness numbers (ms after the event; exec = `/proc/<pid>/exe` stops being "(deleted)", ready = the new WebView's
boot row):

| | desktop | taskmgr | explorer | panel | Start | Settings |
|---|---|---|---|---|---|---|
| exec nl / en | 4 / 6 | 349 / 331 | 574 / 540 | 1240 / 1239 | – | 2032 / 2029 |
| ready nl / en | 1149 / 777 | 1401 / 1477 | 1613 / 1364 | 2032 / 2309 | 2309 / 2470 | 2790 / 2816 |

* Order desktop < apps < panel < Settings, every exec < 5 s, every surface ready < 3 s; same pids (exec); each new
  image runs the replaced file (path without "(deleted)", the new inode).
* Start: the prewarmed child (pid 664 nl) gone at 1218 ms, exactly one new Start (2612) at 1450 ms, child of the same
  panel pid, `--preload`, never two at once (en: 1215 / 1729 ms).
* Fallback bar visible at 1.61–1.72 s (nl) and 1.59–2.01 s (en), hidden again with the native panel back.
* Settings re-exec'd with `--app settings --params {"page":"update"}`, title "Update en herstel"/"Update & recovery";
  the InfoBar is gone after the restart. Explorer re-exec'd without arguments and restored both tabs with the same
  active tab ("gx1-A", "Start"/"Home"), also after the second restart. win11-file-portal exited with 0.
* Refusals (fixture `job.json`, owner = the scenario, phases waiting/downloading/confirm/installing): `update_busy` +
  `update.guard.restart` with the exact nl/en text; Software job queued/running/authenticating: `software_busy` +
  `update.guard.software`; wrong reason `bad_args`; one refusal through the real InfoBar button (screenshot
  `c-refused-update.png`); no event and no restart while refused.
* Deferral: the fixture job for Taakbeheer was written 0.4–0.5 ms after the event; Taakbeheer announced
  `shell.restartDeferred {wait:'software'}` (one log line), the others restarted (Settings 2136/2121 ms), the new
  Settings showed the InfoBar again, Taakbeheer stayed on the old build until the job ended, then exec'd **46/42 ms**
  after the `software.changed` event.
* No second copy of any host, no zombie under a host, no stub call outside the read-only list (nl 166 calls: wpctl 65,
  systemctl --user 60, pw-dump 38, apt list 3; en 158), no "Herstarten"/panic line in the logs, no "is ready" banner
  at the end.

## 5. Decisions for the coordinator

1. **C1 accept D3–D5** (Explorer window close + argv + handed-over lock; portal exits only when idle; activation
   tokens; reaper; panel wait for its Start). They are lib.rs-only and keep window.rs, window_state.rs and portal.rs
   untouched.
2. **C2 InfoBar after a deferred receiver** (D6): GW2b/GW4 could rescan on `shell.restartDeferred` and on the
   deferred receiver's later exec (e.g. the owner rescans when it sees a `shell.restartAll` it did not send, or on a
   short timer after its own start), so the InfoBar goes away once the last process has restarted. Today it stays
   until the next check; clicking it again restarts everything once more (harmless).
3. **C3 GW8**: reuse D9's fixture-root setup (or its own) for every Win11-restart InfoBar shot on this PC, since the
   live `/run/reboot-required` hides it.
4. **C4** the harness's `READ_ONLY_STUBS` list in the scenario is the working definition of "guard log empty" for
   GX1; GW8 may want the same list in `tests/gnome/lib/`.
5. **C5** GW3's D3 is done (mock `shell.restartAll`); mock refusals carry only code + message until round 3 lets
   mock/index.js keep key/params (D-G41).

## 6. Left / not verified

* Not run on the live session (R3/R9 are Robin's): the real extension, systemd scopes (`ensure_native` keeps the
  cgroup across the exec; in the harness there is no user manager) and the real xdg-desktop-portal activation of the
  portal after its exit.
* Alles, Schijven, Standaard-apps and the installer were not opened in the harness (same 300 ms path as Taakbeheer;
  covered by the unit tests' plan table). An update job deferral was tested at unit level only (the harness covers the
  Software case and the update-job refusals).
* A file copy in progress delays that Explorer's exec until the copy ends (D3, existing behaviour).

## 7. Fix round (review `notes/claude/wu/gx1-review.md`, 2026-11-05)

Start check: `git status --short` and `git diff --stat` matched the review's view (5 modified files, +516/−16; new
`restart.rs`, the scenario, its two out directories, `gx1.md`, the review). Pre-fix copies of the edited files are in
`.rt/gx1/fix/*.before`; every scratch file of this round is under `.rt/gx1/fix/`. Nothing committed. Frozen paths
(`harness.py`, `tests/gnome/lib`, `tools`, `extension`, `scripts`, `i18n.rs`, `window.rs`, `runtime.rs`, `portal.rs`,
`files/`, `bin/`, `update.rs`, `update/`, `system/`, `host/i18n`, `tests/rust-sandbox`, `tests/i18n`):
`git diff f01710cc -- …` is 0 lines.

### 7.1 Findings

| Id | Outcome | What changed / why not |
|---|---|---|
| **M1** | **fixed** | `restart::State` keeps the nonce: `scheduled` (the event this process accepted) and `sent` (the event it sent as the origin). `shell.restartAll` checks first, after the reason, whether this process already takes part in a restart and then answers `{ok:true, nonce:<that nonce>, pending:true}` with no event and no guard call; the check and `sent = nonce` happen under the state lock, so calls at the same moment send one event. A prewarmed Start that calls it itself (plan Ignore) clears `sent`. Mock: the same `pending:true` answer while its restart runs. Unit tests `a_second_restart_all_while_one_is_under_way_sends_no_second_event` (double call, 4 parallel calls, still pending until the exec, one restart at 2000 ms) and `calls_at_the_same_moment_send_one_event` (6 parallel calls → 1 event, 1 non-pending answer); harness D: a second call 0.8 s after the click answers `pending:true` with the event's nonce, one event, and every surface boots exactly once (nl and en). Disabling the button after an ok answer is GW3's file (C6) |
| **m2** | **fixed** | `prepare_restart_all(plan, dropped)` stores a callback; the windowless path of `restart_on_main` calls `drop_restart_all()`, which runs it: `restart_dropped` clears the state (scheduled, sent, deferred, announced, exited), writes one stderr line (`Win11 restart: <kind> has no window; …`) and emits `shell.restartDeferred {nonce, pid, kind, wait:'exit'}`; a later `shell.restartAll` is honoured again. The host still does not restart then: a windowless Settings is held by its update keep-alive (C2/D-G40), and quitting it would cut the re-check; app hosts are `NON_UNIQUE`, so no window comes back to that process. Tests `a_host_that_does_not_restart_after_all_says_so_and_takes_a_later_restart`, `the_restart_plan_is_taken_once` (drop path). The GTK decision itself is not unit-tested (needs a display, n6) |
| **m3** | **fixed in lib.rs** (portal.rs untouched) | `count_portal_calls()`: a GDBus filter on the portal's one session connection (installed from the portal's `NativeCore::new`, before portal.rs registers its backend on the same connection) records each incoming `org.freedesktop.impl.portal.FileChooser` call as (sender, serial) and removes it when its reply or error leaves (destination, reply serial). `quit_when_idle` quits only with no window **and** no unanswered call; otherwise one log line and the portal's own idle exit ends it later. So a request in `Phase::Preparing` (no window yet) or `Resolving` is never ended. Left: a call that arrives in the same instant as the idle decision (the portal's own idle exit has that same window). Test `the_portal_counts_a_file_chooser_call_until_its_answer`; harness: the portal still exits with 0 (nl, en) |
| **m4** | **fixed** | `READ_ONLY_STUBS` is now the set of the seven exact argv every run made (`wpctl get-volume @DEFAULT_AUDIO_SINK@`, `pw-dump --monitor --no-colors`, `apt list --upgradable`, four `systemctl --user list-units …`), matched by equality, not prefix; counts are recorded per argv. Moving the list to `tests/gnome/lib/` is frozen for GX1 (C4) |
| **m5** | **not fixed in code**, documented | The in-flight count (`Files.jobs`) is private to `files/mod.rs`, which GX1 does not own. The only signal reachable from GX1's files, `files.progress`, reaches the same observers for this process's copies and for every other process's copies (no pid in it), so a copy in another process that dies halfway would leave Explorer deferred forever, which is worse than today's late exec. Proposal for the coordinator (C7): `Files::busy()` plus a local `files.idle` event when the last job ends (files/mod.rs, ≈8 lines), then restart.rs adds `Wait::Files` resumed on that event (≈10 lines). For R3/R9: a copy that runs when Robin clicks the InfoBar makes Explorer's windows close at 300 ms and come back only after the copy ends |
| n1 | coordinator | not GX1 (the suite did not hang in this round: 7.3 s) |
| n2 | **fixed** | the deferral test now ends the job by the file alone, waits 1.2 s and expects no restart, then sends the event and expects exactly one |
| n3 | **fixed** | the new image's reaper tells an inherited old Start (exe `win11-start`) from the other inherited children and reaps it on its own thread; while one lives (`OLD_STARTS > 0`) the panel does not prewarm, and when it is reaped `glib::idle_add_once(retry_prewarm_start)` prewarms on the main loop (never on the reaper thread). Not reached in the runs (the old Start was gone at 1201/1219 ms, before the panel's exec) |
| n4 | **fixed** (2 of 3) | `exited` is a bounded list (8) and is cleared when a plan is dropped (at an exec the image ends anyway). `take_handoff` → `handoff_file`/`handoff_kind`: Ours (same dev, inode, owner) is taken; Stale (the fd's `/proc/self/fd` link is the lock path or the path `" (deleted)"`) is closed; Foreign is left alone (closing an fd number that is not ours could break an fd the launcher passed). The 50 ms flock poll stays: a blocking flock on a helper thread cannot be given up after the timeout without leaving a thread that may still take the lock (comment at `explorer_handoff`) |
| n5 | GW2b/GW4 | unchanged (C2) |
| n6 | **partly fixed** | new tests `a_handed_over_fd_must_be_the_lock_file` (Ours, Stale after a rename over the lock, Foreign file, a `/proc` file, a directory, fd −1, numbers "", "x", "-1", "0".."2", "3x", overflow; a foreign fd stays open; ours becomes close-on-exec) and `a_vanished_binary_restarts_the_running_image` (`exec_target`: "(deleted)" → path; removed → `/proc/self/exe`; a directory → `/proc/self/exe`; an error is passed on). Not unit-tested: the windowless `restart_on_main` (GTK main thread with a display) and the reaper (the children of the main thread of a test binary); the harness covers the reaper (no zombie) |
| n7 | **fixed** | mock `restartGuard`: `job.foreignOwn` → `software_busy` + the `update.guard.software` text |
| n8 | documented | for R3/R9: every restarted Win11 app window maps focused (activation token), so Win11 windows rise above other apps' windows and Settings, the last, ends in front |
| n9 | coordinator | `tests/i18n/MOCK-CHANGES.md` row for `mock/shell.js` `shell.restartAll` (incl. `pending`) in round 3 (D-G41) |
| n10 | not fixed | the host cannot know a non-origin Settings' current page: the page keeps no URL state and window_state holds a session for Explorer only. It needs a page-side change (vendor/ui, not GX1's) → C8 |
| n11 | **fixed** | the panel's re-check counts the Software jobs whose `_owner.pid` is its prewarmed Start (`PREWARMED_START`, set at spawn, cleared by the child watch and at `stop_prewarm_start`) and watches that Start's pidfd; `Wait::Software(owner)` carries the owner, so any other-process owner is watched. Test: `the_receiver_recheck_…` (a live `sleep` child as the Start: not ours alone, ours with the panel's list, gone through `exited` and after its exit) and `job_rules_match_the_guard` |
| n12 | **fixed** | comment at `HANDOFF` |

### 7.2 Files changed in this round

| File | Change |
|---|---|
| `host/src/services/shell/restart.rs` | M1 state and pending answer; `Wait::Software(owner)`, `software_job_owner`, `wait_for(cache, owners, exited)`; `exited` list; `restart_dropped`; the portal call counter hook; 3 new tests (13 in all), 3 tests updated; rustfmt |
| `host/src/lib.rs` | drop callback (`prepare_restart_all(plan, dropped)`, `drop_restart_all`); portal call filter (`count_portal_calls`, `note_portal_message`, `quit_when_idle`); `exec_target`; `handoff_file`/`handoff_kind`; reaper with the old-Start wait and prewarm retry; `prewarmed_start()`; 3 new tests (6 in all); rustfmt (on a stub copy, HEAD's lib.rs was clean) |
| `vendor/ui/js/mock/shell.js` | `pending` while the mock restart runs; `foreignOwn` refused |
| `vendor/ui-sha256.json` | regenerated (`tools/ui-manifest.py`, 1 changed) |
| `tests/gnome/scenarios/gx1-restart.py` | exact stub allowlist; D: second call at 0.8 s and "every surface booted exactly once"; E: the second restart must not be pending |
| `tests/gnome/scenarios/gx1-restart.out/`, `gx1-restart.en.out/` | new runs (the earlier ones are in `.rt/gx1/fix/prev-out/`) |
| `tests/build/gx1-debug/` | the 12 new binaries + `SHA256SUMS` (gitignored), equal to `target/debug` |

`host/src/services/shell.rs`, `bin/file_portal.rs`, `host/i18n/shell.tsv`: unchanged in this round (no new user-visible
text; the new strings are 2 English stderr lines).

### 7.3 Verification (from W's root)

| Check | Command | Result |
|---|---|---|
| cargo check | `tools/heavy.sh --tag gx1-fix-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, win11-launch 2, launch test 1, lib test 26 warnings (= baseline); 0 in lib.rs or restart.rs |
| rustfmt | `rustfmt --edition 2024 --check host/src/services/shell/restart.rs`; lib.rs on a copy with empty child modules (`.rt/gx1/fix/fmtlib`) | clean (HEAD's lib.rs: 0 diffs in lib.rs) |
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag gx1-fix-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0; lib **385 passed / 16 ignored** (379 + 6 new), launch **16 passed / 1 ignored**, 0 failed, 7.3 s wall; `guard-calls.log` **0 lines**; all 19 GX1 tests ok |
| drift_rust | `python3 -B tools/i18n/drift_rust.py --warnings`; `--new --paths lib.rs shell/restart.rs shell.rs` | 0 errors, 0 warnings; 120 new literals in these files: 5 logs (English stderr), 115 technical |
| check_rust | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | 0 errors, 72 warnings (pre-existing, none in GX1 files) |
| UI i18n | `python3 -B tools/i18n/ratchet.py`; `tools/i18n/check.py` | `ui: ok (89 allowed, 0 to convert)`; 0 errors |
| Mock | `node --check --input-type=module < vendor/ui/js/mock/shell.js`; `node .rt/gx1/fix/mockrun/test.mjs` (the real mock modules, the review's test extended); `node .rt/gx1/mock-restart.test.mjs` | ok; **16/16** (H20 accepted, event, InfoBar cleared, a later call new; a second call within the restart pending with the same nonce and one event; H04/H05/H07 nl, H06 en refused; H25 upgrader; H02/H09 not refused; H26 nl and en `software_busy`; bad reason en); **13/13** |
| Manifest | `python3 -B tools/ui-manifest.py`, then `--check` | 1 changed; up to date |
| Build | `tools/heavy.sh --tag gx1-fix-build -- cargo build --offline --locked -j2`, copied to `tests/build/gx1-debug` | 12 binaries, `sha256sum -c` 12 OK, equal to `target/debug` |
| Harness nl | `tools/heavy.sh --min-mb 2560 --tag gx1-harness -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/gx1-debug --source worktree --scenario tests/gnome/scenarios/gx1-restart.py --out tests/gnome/scenarios/gx1-restart.out` | rc 0, **56/56 checks**, 136.8 s, peak PSS 1218 MiB, leftover **[]**, stub calls outside the list **[]** (170 lines: wpctl 65, pw-dump 38, systemctl 64, apt 3) |
| Harness en | same with `--lang en --out tests/gnome/scenarios/gx1-restart.en.out` (tag `gx1-harness-en`) | rc 0, **56/56 checks**, 136.1 s, peak PSS 1216 MiB, leftover **[]**, stub calls outside the list **[]** (164 lines) |

Harness numbers (ms after the event, nl / en):

| | desktop | taskmgr | explorer | panel | Start | Settings |
|---|---|---|---|---|---|---|
| exec | 4 / 4 | 353 / 351 | 534 / 577 | 1248 / 1242 | – | 2033 / 2052 |
| ready | 1019 / 808 | 1296 / 1265 | 1296 / 1404 | 2033 / 2052 | 2240 / 2188 | 2789 / 2809 |

* Second call (double click) at 810 / 861 ms: `{ok, nonce: <the event's>, pending: true}`; one event; boots desktop 1,
  taskmgr 1, explorer 1, panel 1, Start 1, Settings 1.
* Start: old one gone at 1201 / 1219 ms, exactly one new one (a child of the same panel) at 1482 / 1452 ms, never two.
* Fallback bar shown 1.63–1.74 s (nl and en), native panel back by 3.17 s.
* Explorer restored "gx1-A" + "Start"/"Home" with the same active tab, also after the second restart; Settings reopened
  on the update page; the portal exited with 0; the new images run the replaced files (path, new inode).
* Refusals (4 update-job phases, 3 Software states, bad reason) unchanged; E: the second restart is a new one (not
  pending), Taakbeheer announced its deferral (one log line), the new Settings showed the InfoBar again, and
  Taakbeheer exec'd 25 / 31 ms after the job's event.
* No second copy of any host, no zombie, no leftover process.

### 7.4 Decisions for the coordinator (in addition to §5)

6. **C6** (M1, GW3's file): `update.js` could keep the restart button disabled after an ok answer until the page goes
   away; the host now makes a second click harmless anyway (`pending:true`).
7. **C7** (m5): accept the gap or give a later step `files/mod.rs` `Files::busy()` + a local `files.idle` event; GX1's
   receiver would then defer Explorer on a copy like on a Software job. Until then R3/R9 say so.
8. **C8** (n10): a non-origin Settings reopens on its launch page; fixing it needs the Settings page to tell the host
   its page (vendor/ui).
9. Accept m3's lib.rs-only reading (a GDBus filter instead of the one-liner in portal.rs that the review suggested).
