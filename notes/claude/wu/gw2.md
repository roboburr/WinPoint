# GW2a: the Rust `update.*` service core (unit level), 2026-11-05

Worktree `.wt/gw2` (branch `wu/gw2`, from d666df54). Design: `notes/claude/windows-update-gnome.md` §4 and the
GW2 row of §10; decisions D-G25..D-G29 of `plan.md`. Nothing was committed. No bus was contacted, no
PackageKit/apt/pkcon call was made, no win11-* binary, gnome-shell, gsettings or installer ran. Every heavy
command went through `tools/heavy.sh`.

Start check: `git status --short` showed only `notes/claude/plan.md` modified; HEAD was d666df54.

## 1. Files

New (all mine):

| File | Lines | What |
|---|---|---|
| `host/src/services/update.rs` | 692 | `Update`: modes (owner, mirror, reader, dry, unsupported), owner election, forwarding (`update.request` / `update.reply` / `update.owner`), per-method argument allowlists, the dry-run envelopes, `compose()` (the `update.status` snapshot from the files + live job), notifications, the keep-alive hold, `system_updates()` / `reboot_required()` for GW2c |
| `update/paths.rs` | 423 | the one `Paths` / `Tools` table (17 tools incl. the GW5 helpers and do-release-upgrade), the test re-rooting, the fail-closed seam check (`resolve`), the only `Command::new` |
| `update/pk.rs` | 1543 | the `update-pk` thread (own `MainContext` + `MainLoop`, engine in a thread-local, `ask()` ≤ 5 s), the event-driven PackageKit client (CreateTransaction → SetHints → method; signals, `PropertiesChanged`, 30 s/120 s watchdog, no poll, no cap), check/install(simulate → confirm → update)/confirm/cancel/repair/details, reattach (GetTransactionList / GetAll NO_AUTO_START, GetOldTransactions), foreign transactions, daemon vanish, RequireRestart, FileMonitors, progress throttle (250 ms local, 1 s fan-out), `system.updates.changed` |
| `update/model.rs` | 531 | pure: PackageKit enum table, `Phase`, `stage()`, `derive_phase()`, `Pkg`, row grouping, totals, install ids, error mapping (§3.7), `source_host()`, `Throttle`, reattach and foreign decisions, the release-wrapper outcome table (`upgrade_outcome`) |
| `update/phasing.rs` | 230 | the batched read-only `python3 -I -c <SCRIPT>` apt_pkg classifier (niced, idle I/O, 30 s timeout, env cleared), the `apt-cache show` fallback, both parsers, `apply()` |
| `update/store.rs` | 272 | state dir 0700, files 0600 by temp + rename, `read_private`, `jobs.jsonl` (90 days), `try_lock`/`locked`, boot id, process start, owner liveness, `elect()`, `next_rev()` |
| `update/restart.rs` | 298 | reboot-required parse, RequireRestart rule, the deleted-mapping scan (watch lists, noise), apt history parse + fallback |
| `update/net.rs` | 98 | NM `Connectivity`/`Metered` (NO_AUTO_START), `PropertiesChanged`, `Net` mapping |
| `update/guard.rs` | 180 | `power_blocked`, `restart_all_blocked`, `job_blocks_power`, the upgrader detection (`is_upgrader`, `upgrader_running`), `unattended_running`, `/proc/locks` (`parse_locks`, `apt_lock_held`), `software_job_active` |
| `update/{history,options,release,admin,compat,elf,upgrade}.rs` | 111 | stubs with the final signatures (all take `&Shared`); they answer `ApiError::new("unsupported", <method>)` |
| `update/tests.rs` | 1212 | 35 unit tests |
| `update/bus_tests.rs` | 93 | 8 `#[ignore]` skeletons for GW2b (`fake_bus()` refuses to run outside the bwrap layer) |
| `host/i18n/update.tsv` | +314 | 100 rows (99 of §3.7 + `update.err.notHere`), written only by `extract_rs.py add --apply` from `notes/claude/wu/gw2-rows.md` |
| `tests/i18n/BASELINE-CHANGES.rust-gw2` | | 100 `key:` lines (every row is `#@ new`) |
| `tests/i18n/allow-host-gw2.txt` | | 2 rules: `restart.rs item:NOISE`, `phasing.rs item:SCRIPT` (C9) |
| `notes/claude/wu/gw2-rows.md`, this note | | |

Edited: `host/src/services/mod.rs` (+11: `mod update`, `LazyService<update::Update>`, the `update.` route,
shutdown first), `host/src/events.rs` (+1: prefix `update.`), `host/src/api-methods.json` (+17 entries: the 16
methods of §4.3 incl. `update.release.ubuntuHold`, plus `shell.restartAll`; still one sorted ASCII line, the
rest byte-identical), `host/src/settings-defaults.json` (one new last top-level key `update`:
`notifyRestart: true, metered: false, autoOpenPrevious: null, ubuntuHoldPrevious: null`).

Not touched: `i18n.rs` (stem `update` was already in `STEMS`, D-G28), lib.rs, window.rs, runtime.rs, Cargo.*,
every round-2 file incl. `system/info.rs` and `system/power.rs` (patch in §4), extension, scripts, tools.

## 2. Verification (exact commands, from W's root)

| Check | Command | Result |
|---|---|---|
| Unit tests + oracles | `tools/heavy.sh --min-mb 2048 --tag gw2-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **306 passed** (271 + 35), **16 ignored** (8 + my 8 bus skeletons), 0 failed; win11-launch 5 passed, 1 ignored; 11 other bins 0 tests. Oracles **16/16** ok. `logs/guard-calls.log` **0 bytes**; tmp entries left 0. 15 s wall, 1.0 GB peak RSS |
| Flakiness of the threaded tests | `tools/heavy.sh --min-mb 2048 --tag gw2-test -- bash -c 'for i in 1 2 3 4 5; do tests/rust-sandbox/run-tests.sh --lib services::update -- --test-threads=8 …; done'` | 5 × `34 passed; 8 ignored` (before the 35th test was added), guard log 0 each time |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors, 0 warnings** (127 rows, 1396 new literals) |
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths host/src/services/update.rs host/src/services/update/*.rs host/src/services/mod.rs host/src/events.rs host/src/api-methods.json host/src/settings-defaults.json --warnings` | 0 errors, 0 warnings |
| Catalogue + call sites | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 65 warnings: 64 `unused` (rows whose call sites belong to GW4–GW7/GW9: `update.notify.{restartNow,later,view}`, every `update.release.*` row except `err.noPassword`/`err.wrongPassword`/`err.refused` (already named by `model::start_failure_key`), all `update.compat.*`, `update.part.*`, `update.feature.*` and `update.post.*`) and 1 `glossary` (`update.part.pixbuf`: nl "afbeeldingen" means images here, not the Pictures folder, so en "images" is right) |
| cargo check | `tools/heavy.sh --tag gw2-check -- cargo check --offline --locked -j2 --all-targets` | rc 0; lib **28** warnings, win11-launch **2** (= baseline); lib test 26, win11-launch test 1; **no warning in any file I touched** (`--message-format=short` file list) |
| Build | `tools/heavy.sh --tag gw2-build -- cargo build --offline --locked -j2` | rc 0, all **12** `target/debug/win11-*` rebuilt (19:08:26, after the last source change) |

The 35 unit tests cover (design §9.2 and the GW2a done-when): no seam → system table; valid seams re-root every
path and take tools only from the stub dir (a missing stub is an error); **20 failing-seam cases** (unknown
variable, euid 0, empty value, relative / `..` / missing / unmarked / world-writable / `/` / foreign-uid fixture
root, unmarked / missing tool dir, non-`unix:path=`, extra keys, missing socket, plain file, unmarked socket dir,
`/run/dbus`, abstract, one bad among good) each → `unsupported`/`test_seam` for every method, no shared state,
stub log empty, the bus socket never connected; the seam text in nl and en; `pk::system_bus()` panics without
`WIN11_UPDATE_PK_BUS`; the **exec-path grep** (patterns `Command::new("/`, `"/usr/bin/`, `"/usr/lib/`, `"/bin/`,
`"/usr/libexec/` outside the one `// table:begin … // table:end` block, and no `Command::new` outside paths.rs;
it caught its own pattern text once, so it works); the **phasing parser** (script output, end marker, the
`apt-cache show` fallback, phased/held never in a row or the install ids, another version's classification
ignored, the classifier through the stub only with names on stdin, the fallback call, a failing classifier);
**owner logic** (one flock, Lost/Won, rev continuation, liveness) and a **two-services test without a bus**
(owner + mirror on one cache, forwarded `update.check` answered by `update.reply`, a bad forwarded request
refused by the owner's allowlist, the mirror reads the owner's snapshot, takeover after the owner's shutdown,
`(bus, threads, tools, forwarded) = (0, 1, 0, 1)`); **dry-run zero calls** (every method, stats `(0,0,0,0)`,
stub log empty, no connection); **reader mode never connects** (5 kinds, writes `not_here`); the power guard on
files only (live owner, dead owner < 10 min, other boot, finished, not committing, upgrader running); the
**`/proc/locks` rule** (POSIX/OFD/FLOCK, waiter lines, unrelated inode, the archives lock); the upgrader
detection (not our wrapper, not the pkexec prompt); every write refused while it runs; the argument allowlist (20
bad cases); phase derivation; grouping in the §3.3 order; the error map; the throttle under a 2000-event flood (≤ 4
local, ≤ 1 remote per second); reattach/foreign decisions; the **wrapper outcomes** (14 cases); store
round-trips, modes, symlink and group-readable refusals, `jobs.jsonl` pruning, the job lock, boot id;
reboot-required; the maps scan on a fixture procfs with today's noise (libpcre2, gvfs, memfd, a cache file); the
history parser and fallback; the snapshot view (offered-only count, phased/held excluded, restart from
RequireRestart, rendered warnings and errors in nl/en, unsupported, offline); NM states; en texts at the real call
sites; the GW2c `system_updates()` helper.

## 3. Deviations from the design and readings (for the coordinator)

1. **New row `update.err.notHere`** ("Updates installeren kan alleen vanuit Instellingen." / "Updates can only
   be installed from Settings."): §4.1 says reader-mode writes return `ApiError("not_here", …)` but §3.7 has no
   text for it. Listed in BASELINE-CHANGES.rust-gw2 with "(not in §3.7)". Alternative: `update.err.badArgs`.
2. **`update.status` after a failed seam** answers Ok with the snapshot `{phase:'unsupported', error:{code:
   'test_seam', …}}` (built from nothing, no file read); every other method answers the `test_seam` error. §4.3
   says "every method"; §4.5 needs the phase to be visible.
3. **`guard::power_blocked(ctx, action)`** takes the action and returns None for `lock` (and anything else
   that is not reboot/shutdown/suspend/logout), instead of `power_blocked(ctx)` — it cannot then be misused.
4. **Keep-alive**: the `Application::hold()` is taken when our job starts (install, repair, reattach) and
   released when it finishes, whatever the window does — the same effect as "hold when the window closes during a
   job" without a lib.rs/window.rs hook (they are frozen). It runs on the GTK main thread through
   `MainContext::default().invoke`; without an application (tests, `--http-only`) it does nothing. See §5.
5. **`update.details`** waits ≤ 5 s (§4.1); on a timeout it returns the cached part with `pending:true` (additive
   field); the page asks again.
6. **Forwarding**: a mirror write returns `{forwarded:true, nonce}` at once; the owner answers **every**
   forwarded request with `update.reply {nonce, ok, error?}` (the design names only the refusal), off the event
   receiver thread.
7. **Notifications** (§3.8): installed/failed only when no page is subscribed (`subscriber_count() == 0`, the
   closest the service can see to "not focused" without window.rs); restart required always (if
   `settings.update.notifyRestart`); none after a declined confirm or a cancel; never in dry-run. **No action
   buttons** yet: the rows `update.notify.restartNow/.later/.view` exist (unused) for GW9/GX1, which own the
   action handling.
8. **No `locale -a` program**: the locale hint checks libc `newlocale()` (cached), same answer, no subprocess.
9. **nice/ionice**: the classifier lowers its own priority in `pre_exec` (`nice(10)`, `ioprio_set` idle); the
   `nice`/`ionice` tools stay in the table for GW6's apt runs.
10. **Phasing script prints a 7th column, the installed version**, because GetUpdates has no "from"; the
    snapshot's `packages[].from` comes from it.
11. **Wrapper outcome reading** (`model::upgrade_outcome`): the reboot rule applies only when the upgrader was
    seen or the wrapper reported rc 0; otherwise today's already pending gnome-shell reboot would hide a
    dismissed pkexec dialog (§6.4 step 5 lists the reboot rule first without that condition).
12. **Upgrader detection** counts `do-release-upgrade` only as argv[0] (or the script of a `python*` argv[0]),
    any `/tmp/ubuntu-release-upgrader-*` / `DistUpgradeMain` argument, or the root `screen … -S
    ubuntu-release-upgrade-screen-window`; our own `sh -c <wrapper> … pkexec do-release-upgrade` and the pkexec
    prompt do not count, so a dismissed prompt ends as `startFailed`, not `ended`.
13. **`gnome` watch list** also has `/usr/lib/x86_64-linux-gnu/gnome-shell/` (where `libshell-18.so`,
    `libst-18.so` live; today's real deleted mappings).
14. **Owner liveness uses the real `/proc`** even with a fixture root (pid + start of real processes); the
    fixture procfs is for the scans, the boot id and `/proc/locks`.
15. **The tool table already holds everything GW5–GW7 start** (`PromptHelper`, `AttachHelper`,
    `DoReleaseUpgrade`, `Screen`, `Sh`, …) so they never edit paths.rs; the stubs all take `&Shared` (ctx, setup
    with paths and tools, store).
16. Test mode **without** `WIN11_UPDATE_PK_BUS` is `BusTarget::Disabled`: nothing is ever connected, PackageKit
    counts as absent (`unsupported`/`update.err.noPackageKit`), not a seam error.
17. A cache directory that cannot be made private also ends in `unsupported` (it then shows the test-seam text;
    rare, documented here only).
18. The snapshot's `release` object comes from the `release::status` stub (`state:'none'`, `helper` flags from
    `admin::trusted_helper`, always false until GW5); `compose` sets `state:'running'` while the upgrader runs.

## 4. GW2c: the patch for `system/info.rs` and `system/power.rs` (after the round-2 merge)

Both hooks call only files-and-/proc code (no D-Bus), so they are safe in the panel's reader mode.

```diff
--- a/host/src/services/system/power.rs
+++ b/host/src/services/system/power.rs
@@ pub(super) fn action(action: &str, ctx: &ServiceContext) -> ApiResult {
+    // design §4.9: refused while our update job commits or Ubuntu's release upgrader runs (never `lock`)
+    if let Some(error) = crate::services::update::guard::power_blocked(ctx, action) {
+        return Err(error);
+    }
     let comp = compositor();
--- a/host/src/services/system/info.rs
+++ b/host/src/services/system/info.rs
@@ impl Updates {
     pub fn get(self: &Arc<Self>, refresh: bool) -> Value {
+        // design §4.10: once a snapshot exists it is the only count (no apt list, no event from here)
+        if let Some(count) = crate::services::update::system_updates(&self.ctx) {
+            return count;
+        }
         if !available("apt") {
-            return json!({"available":null,"checkedAt":null,"tool":null,"checking":false});
+            return json!({"available":null,"checkedAt":null,"tool":null,"checking":false,
+                          "rebootRequired":crate::services::update::reboot_required()});
         }
@@
         let mut r =
             s.0.clone()
                 .unwrap_or(json!({"available":null,"checkedAt":null,"tool":"apt","security":null}));
         r["checking"] = json!(s.2);
+        r["rebootRequired"] = json!(crate::services::update::reboot_required());
         r
     }
     fn check(&self) {
+        // a snapshot appeared while this apt run was queued: the owner counts and emits now (§4.4)
+        if crate::services::update::system_updates(&self.ctx).is_some() {
+            self.state.lock().unwrap_or_else(|e| e.into_inner()).2 = false;
+            return;
+        }
         let r = run(&["apt", "list", "--upgradable"], 60, true);
```

Then remove the two `#[allow(dead_code)]` lines above `system_updates` / `reboot_required` in update.rs and
the one above `guard::power_blocked`. Tests for GW2c: `system.power.action {action:'reboot'}` with a fixture
`job.json` (committing, live owner) → `update_busy`/`update.guard.power` and the guard stub log empty; `lock`
passes; `system.updates` returns the snapshot's count when one exists (the update test
`system_updates_counts_the_snapshot_once_one_exists` already covers the helper). `shell.restartAll` (GX1) calls
`guard::restart_all_blocked(ctx)`.

## 5. lib.rs keep-alive answer (design §4.1, C2)

Closing the Settings window **only drops the GApplication use count; nothing calls `quit()`**:

* The native close (`window.rs:880` `connect_delete_event`) first stops the delete, sends `win11:close` to the
  page and closes the window again 200 ms later (`window.rs:907`); the second delete returns `Proceed`, so GTK
  destroys the window and `GtkApplication` releases its hold on the application. The titlebar's `close` IPC
  (`window.rs:523-535`) is a plain `window.close()`.
* `quit()` is called only by: the Lifecycle request (`window.rs:419`, sent by `request_shutdown`, i.e.
  `shell.restart`/`shell.quit`), the desktop's `desktop-quit` action (`lib.rs:359`), Start's `start-quit`
  (`window.rs:456`) and a terminated web process of desktop/panel/start (`window.rs:680`). None applies to a
  Settings window close.
* Settings is `NON_UNIQUE` (`lib.rs:351`); `app.run_with_args` (`lib.rs:522`) returns when the use count reaches
  0, then `runtime.shutdown()` (`lib.rs:527`) runs `Update::shutdown`.

So an `Application::hold()` keeps the process (and the pk thread) alive after its window closes, and **no lib.rs
change is needed for GX1**. Note for GX1: `shell.restart`'s `quit()` ignores holds, which is why
`restart_all_blocked` refuses while a job runs. To verify in R3 (`pgrep -a win11-app` after closing the window
during a download), together with `gio::Application::default()` returning Settings' application (GLib sets the
first GApplication as default when it is constructed).

## 6. What GW2b needs from GW1

* `tests/update/run.sh cargo`: build with `cargo test --no-run --offline --locked -j2` in GE0b's sandbox under
  `tools/heavy.sh`, then run the test binary inside the bwrap layer with filter `services::update::bus_tests::`
  and `-- --ignored` (never `--include-ignored`), env:
  * `WIN11_UPDATE_PK_BUS=unix:path=<dir>/<socket>` where `<dir>` is ours, not group/world writable, holds the
    marker `.win11-update-fixture`, and the socket exists (a real `dbus-daemon`, because the client connects with
    `MESSAGE_BUS_CONNECTION` and uses `NameHasOwner`/`ListActivatableNames`/`NameOwnerChanged`); not under
    `/run/dbus`;
  * `WIN11_UPDATE_FIXTURE_ROOT=<dir>` (marker) with at least `proc/sys/kernel/random/boot_id`, `etc/os-release`
    (`VERSION="26.04.1 LTS (Resolute Raccoon)"`), optional `run/reboot-required{,.pkgs}`, `proc/locks`,
    `proc/<pid>/{cmdline,comm,status,maps,cgroup,exe,stat}`, `proc/stat` (btime), `var/log/apt/history.log`;
  * `WIN11_UPDATE_TOOL_DIR=<dir>` (marker) with a `python3` stub that prints the **7-column** classifier format
    of `phasing.rs` (`name\tarch\tcandidate\tphased\tpup\theld\tinstalled`, then `@@end`) and logs its argv; every
    other stub logs and fails;
  * no other `WIN11_UPDATE_*` variable (an unknown one fails closed).
* The fake PackageKit (`org.freedesktop.PackageKit` on that bus): `CreateTransaction`, `GetTransactionList`
  (`ao`), manager signals `UpdatesChanged`, `TransactionListChanged` (`as`); transaction methods `SetHints`,
  `RefreshCache(b)`, `GetUpdates(t=2)`, `GetDetails(as)` (signal `Details a{sv}` with `package-id`,
  `download-size` optional), `GetUpdateDetail(as)` (`UpdateDetail` 12-tuple and/or `UpdateDetails`),
  `UpdatePackages(t=6, as)` then `UpdatePackages(t=2, as)`, `RepairSystem(t=2)`, `GetOldTransactions(u=5)`
  (`Transaction (o s b u u s u s)`), `Cancel`; signals `Package`, `Packages`, `ItemProgress`, `RequireRestart`,
  `ErrorCode`, `Finished`, `Destroy`, `PropertiesChanged` (`Status`, `Percentage` with 101 = unknown,
  `AllowCancel`, `Speed`, `RemainingTime`, `DownloadSizeRemaining`, `LastPackage`); `Properties.GetAll` and
  `Get` (`Role`, `Uid`) on transaction paths, also with NO_AUTO_START; package ids `name;version;arch;data` with
  Debian names; the 31 timelines incl. phased-with-info-5, success-while-rows-remain, vanish (name owner gone
  mid-transaction, then back with a `GetOldTransactions` record or none), flood; `CallCount()` per method.
* The fake NetworkManager: `Connectivity`/`Metered` via `Properties.Get` (u) and `PropertiesChanged` on
  `/org/freedesktop/NetworkManager` (arg0 = `org.freedesktop.NetworkManager`).
* A fake `org.freedesktop.Notifications.Notify` on the run's session bus (two-services test: only the owner
  calls it).
* `tests/update/enums.json` to compare with `model.rs`'s tables (role, status, info, exit, restart types, filter
  and flag bits).

## 7. Decisions for the coordinator

1. Accept the new row `update.err.notHere` (deviation 1), or map reader writes to `update.err.badArgs`.
2. Phasing failure policy: when both the classifier and the fallback fail, packages stay as PackageKit reports
   them (all offered), so a phased package could be installed early (what `apt full-upgrade` with phasing off
   would do). The safer alternative is to refuse `update.install` (or offer only security rows) while the
   snapshot is unclassified; it needs one more text. Default kept: offered.
3. Confirm deviation 2 (status answers the unsupported snapshot after a failed seam) and deviation 4 (hold at
   job start, no lib.rs hook).
4. GW2c is the small patch of §4 on R2-A's merged files; after it the three `#[allow(dead_code)]` lines go.
5. Unused rows are expected until GW4–GW7/GW9 write their call sites (check_rust warnings, not errors).

## 8. Fix round (2026-11-05, after `gw2-review.md`)

Start check: `git status --short` and `git diff --stat` matched the review's view exactly (the same owned files,
`plan.md` the coordinator's change, HEAD d666df54). Nothing committed. No bus, PackageKit, apt or win11-* binary was
touched; every heavy command went through `tools/heavy.sh` (tags `gw2-fix-*`, `gw2-test`, `gw2-build`). Scratch
logs are in `.rt/gw2-fix-*`.

Changed this round (all owned): `host/src/services/update.rs`, `update/{paths,pk,model,phasing,guard,restart,tests,
bus_tests}.rs`, `host/src/events.rs` (the page filter and its test), this note. Not changed: `update.tsv` (no new
row), `BASELINE-CHANGES.rust-gw2`, `allow-host-gw2.txt`, `api-methods.json`, `settings-defaults.json`,
`services/mod.rs`, the seven stubs, every frozen file.

### 8.1 Findings

| # | Status | What changed | Test |
|---|---|---|---|
| F1 | fixed | `paths::Tools` now gives every child `LANG` plus a fixed env: production `PATH=/usr/bin:/bin` (in the table); test mode `PATH=<tool dir>`, `DBUS_SYSTEM_BUS_ADDRESS=` the checked seam bus or `unix:path=<tool dir>/no-system-bus`, `DBUS_SESSION_BUS_ADDRESS=unix:path=<tool dir>/no-session-bus` | `tool_children_get_the_stub_path_and_the_test_buses_only` (a stub prints its env; `command -v pkexec` is the stub); strace: every stub and its `/bin/cat` child have exactly those 4 variables |
| F2 | fixed | `finish_job`: on success the job's ids leave `pkgs` before the persist; the keep-alive is released only when the re-check's check ends (`checked()`), at most 60 s (`HOLD_AFTER_JOB` timer), never while a new job holds it; `notify()` flushes the session bus. `checking` is still persisted during the re-check, but `compose` counts it only while `owner.json` is alive and is the snapshot's writer (the review's "only while an owner is alive") | `a_finished_job_leaves_no_installed_package_in_the_snapshot`, `the_keep_alive_lasts_until_the_recheck_and_not_through_a_confirm`, `checking_counts_only_while_its_owner_lives` |
| F3 | fixed | the snapshot keeps `lastJob {jobId, role, finishedAt, outcome, rows:[row_info with ids]}` until the next `update.check` (also cleared by install/repair); pure `model::row_views` derives `installed`, `restart` (a package of the row in `/run/reboot-required.pkgs` or RequireRestart of this boot), `failed` with a rendered `error` (only rows still holding packages of the failed job), `pendingDownload` after a cancel, decline or unknown outcome | `rows_after_a_job_are_installed_restart_or_failed`, `a_failed_job_keeps_its_packages_and_marks_its_rows_failed`, `checking_counts_only_while_its_owner_lives` (reader sees R03 text) |
| F4 | fixed | `system_updates()` (→ pure `system_updates_from`) answers None unless `checkedAt` is a number and `backend == "packagekit"`, so the init snapshot and a machine without PackageKit keep the apt count. The GW2c patch of §4 is unchanged | `system_updates_counts_the_snapshot_once_one_exists` (+ init and backend-null cases), `system_updates_needs_a_finished_check_with_packagekit` |
| F5 | fixed | `phasing::classify` runs the `apt-cache show` fallback on every script failure (exit ≠ 0, no `@@end`, timeout, spawn error). Whatever neither answered for the offered version (incl. an apt candidate of another version) gets the safe rule `phasing::unknown`: offered only from the `-security` pocket (Ubuntu never phases it), every other package is treated as phased ("Wordt later aangeboden", the existing `phasedLater`/`offeredLater` UI texts, so **no new text**). The snapshot has `phasing: apt|fallback|unknown`. A missing stub stops at once (no further program) and the snapshot error is `test_seam` | `the_classifier_runs_only_the_stub_with_names_on_stdin` (traceback, missing marker, both failing, missing stub), `a_package_without_an_answer_for_its_version_gets_the_safe_rule` |
| F6 | fixed | `Events::show_update_events()` (set by `Update` in settings hosts, owner or mirror): `update.progress` and `update.reply` reach the WebViews only there; `update.request` and `update.owner` reach observers only, in every host. Every other event is unchanged | `events::tests::update_progress_and_forwarding_reach_only_settings_pages` (owner, a second settings host, a panel host over the real datagram fan-out) |
| F7 | fixed | entering `Step::Confirm` releases the hold; `update.confirm {accept:true}` takes it again. A job that was paused for a confirm when its owner ended is ended as `declined` by the next owner (`model::Reattach::Expired`), because GetOldTransactions would report the simulate's success as an install | `the_keep_alive_…`, `a_job_paused_for_a_confirm_ends_as_declined_in_the_next_owner`, `reattach_and_foreign_decisions` |
| F8 | fixed | `model::Throttle` ×2 is replaced by `model::Progress` (`Delivery::{None, Local, Both}`): every delivery keeps the 250 ms page interval, at most one per second is also `Both` (the fan-out, which reaches the own page too); pending values go out on later ticks | `progress_is_throttled_…` now drives the real send path with an out-of-phase timer and checks every sliding second: ≤ 4 page, ≤ 1 fan-out, the last value delivered on both |
| F9 | fixed, differently | the two GIO `FileMonitor`s are replaced by one inotify descriptor (`restart::Watch`) on a `glib::unix_fd_source_new` on the pk context. The review's `gio::Vfs::local()` was tried first: it passed alone but in the full lib suite the pk thread panicked twice at `gio-0.18.4/src/auto/vfs.rs:9` (`assertion failed: instance_of::<Self>`) and one run died with SIGSEGV, so GIO's local-VFS singleton is not safe to rely on in a process that also uses the default VFS. `notify()` now counts in `Stats.bus_opens` | `the_file_watch_sees_only_the_watched_files`; strace of the update tests: **0 connect calls** (the review saw 1, GVfs) |
| F10 | fixed | `fake_bus()` also refuses a visible `/run/user/<uid>/bus`, a `DBUS_SESSION_BUS_ADDRESS` under `/run/user/`, and the host network namespace: `/proc/self/ns/net` must differ from `WIN11_TEST_HOST_NETNS`, which GW1's runner records outside the layer (not a `WIN11_UPDATE_*` name, which would fail the seam check) | skeletons only (never run here) |
| F11 | fixed | the grep has `"/sbin/`, `"/usr/sbin/`, `"/usr/local/` and forbids, outside `paths.rs`, the std command type as a word in code (so grouped and aliased imports too; `CommandExt` is another word), `Subprocess*`, `spawn_async/sync/command_line*`, `glib::spawn*`, `.exec()`, `libc::exec*`, `execvp(`/`execve(`, `posix_spawn`, `libc::system(`, `libc::fork(`, `popen(` | the test now also feeds 17 bad lines to its own matcher (each must hit) and good lines that must not; it caught a grouped import in its first version and two lines of this round's own code |
| F12 | fixed | `is_upgrader`: a `python*` argv[0] whose script argument (options skipped; none for `-c`/`-m`) is `do-release-upgrade` or under `/tmp/ubuntu-release-upgrader-`; that program as argv[0]; or `screen`/`SCREEN` with the pair `-S ubuntu-release-upgrade-screen-window`. `DistUpgradeMain` anywhere no longer counts | negative cases: an editor, a pager, `grep`, `ls`, `python3 -c`, another python script, `python3 -m`, the attach client `screen -d -r -p …`, `screen -ls …` |
| F13 | fixed | `model::stage`: committing first, then statuses 30/31 → waiting, then the simulate step | 4 new asserts in `phase_derivation_…` |
| F14 | fixed | any successful job (UpdatePackages, RepairSystem, a reattached one) clears `repairRequired` in `finish_job` | `a_finished_job_leaves_…` |
| F15 | fixed | `model::Lru` (64 ids, least recently used out) for `update.details`; cleared on every GetUpdates result; only ids of the current list are cached | `the_details_cache_is_bounded` |
| F16 | fixed (code) | `update.install` answers `update.err.busy` while a check runs | needs a bus: listed for GW2b in `bus_tests::every_timeline_in_nl_and_en` |
| F17 | fixed, needs a word | `update::upgrade_guarded`: §3.7's list (check, install, confirm with `accept` not false, repair, release.check/compat/setPrompt/start) is refused while the upgrader runs; cancel, details, a declined confirm, setAutoOpen, ubuntuHold and attach are not (§4.5 1a: an own job keeps running) | `every_write_is_refused_while_the_release_upgrader_runs` now asserts both sets and the exact guarded list |
| N1–N13 | notes, not changed | N3 (page-open rescan), N5 (`seen`), N6 (8 KiB), N7 (`child_value` bounds in callbacks), N8 (`owner.lock` in `~/.cache`), N9 (fixture root always set by GW1), N10 (unknown outcome notifies "couldn't be installed") stay open for GW2b/GW3/GW4; N2 belongs in the design's "as built" (now also `lastJob`, `phasing`); N11/N12 confirmed |

### 8.2 New or changed deviations (adds to §3)

19. **No GIO FileMonitor** (design §4.8 names `gio::File::for_path(…).monitor_file`): one inotify fd on the
    parent directories of `/run/reboot-required` and `history.log`, events create/close-write/delete/moved (F9).
20. **Unclassified packages** (both classifiers failed, or no answer for the offered version) are treated as
    phased unless they come from the `-security` pocket (F5; this settles §7 decision 2 without a new text).
21. **`lastJob`** in `snapshot.json` (F3) and `phasing` in the snapshot and in `update.status` (F5).
22. **Keep-alive** (replaces deviation 4): held while a transaction of ours runs (simulate, update, repair,
    reattach), released at a confirm pause and taken again on accept (F7), and after `Finished` kept until the
    re-check answered, at most 60 s (F2).
23. **Upgrader guard** covers §3.7's list only, plus confirm with `accept:true` (F17; §4.3 says "every write").
24. **`update.status` `refreshing`/`checking`** count only while the snapshot's owner is alive (F2).
25. **Page events**: `update.progress`/`update.reply` reach WebViews only in settings hosts that created the
    update service; `update.request`/`update.owner` never reach a page (F6).

### 8.3 Verification (exact commands, from W's root)

| Check | Command | Result |
|---|---|---|
| Unit tests + oracles | `tools/heavy.sh --min-mb 2048 --tag gw2-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **317 passed** (271 + 45 update + 1 events), **16 ignored** (8 + the 8 bus skeletons), 0 failed; win11-launch 5 passed, 1 ignored; 11 other bins 0 tests. Oracles **16/16** ok (8 `oracle_tests`, 7 `disks::tests` python cases, `reference_contracts`). `guard-calls.log` **0 bytes**, tmp entries left 0. 14.2 s wall, 0.99 GB peak RSS |
| Full lib, repeated | `tools/heavy.sh --min-mb 2048 --tag gw2-test -- tests/rust-sandbox/run-tests.sh --lib` × 3 (after the F9 change) | 3 × `317 passed; 16 ignored`, guard log 0, tmp 0 |
| Flakiness | `tools/heavy.sh --min-mb 2048 --tag gw2-fix-repeat -- bash -c 'for i in 1 2 3 4 5; do tests/rust-sandbox/run-tests.sh --lib -- services::update events::tests --test-threads=16; done'` | 5 × `49 passed; 8 ignored`, guard log 0 each time |
| strace | `tools/heavy.sh --min-mb 2048 --tag gw2-fix-strace -- tests/rust-sandbox/run-tests.sh --lib --config 'target.x86_64-unknown-linux-gnu.runner=["strace","-f","-v","-s","256","-e","trace=execve,connect","-o","<W>/.rt/gw2-fix-strace/trace.log"]' -- services::update` | 45 passed. **0 connect calls.** execve: the test binary, 7 × the `python3` stub, 4 × the `apt-cache` stub, 2 × `/bin/cat` (from a stub). Every stub env is exactly `LANG=C.UTF-8`, `PATH=<tools>`, `DBUS_SYSTEM_BUS_ADDRESS=<fixture bus or tools/no-system-bus>`, `DBUS_SESSION_BUS_ADDRESS=<tools>/no-session-bus`; the `cat` children inherit them |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors, 0 warnings** (127 rows, 1507 new literals) |
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths host/src/services/update.rs host/src/services/update/*.rs host/src/services/mod.rs host/src/events.rs host/src/api-methods.json host/src/settings-defaults.json --warnings` | 0 errors, 0 warnings |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 65 warnings: the same 64 `[unused]` rows of GW4–GW9 and the `update.part.pixbuf` glossary note (§2). No row added or changed this round; `update.tsv`, `BASELINE-CHANGES.rust-gw2` and `allow-host-gw2.txt` are untouched |
| cargo check | `tools/heavy.sh --tag gw2-fix-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, win11-launch 2, lib test 26, launch test 1 (= baseline); **none in update/*, events.rs or services/mod.rs** |
| Build | `tools/heavy.sh --tag gw2-build -- cargo build --offline --locked -j2` | rc 0, all **12** `target/debug/win11-*` at 20:15:31–33; `find host/src host/i18n -newer target/debug/win11-app` is empty |

### 8.4 What GW2b needs in addition (adds to §6)

* `tests/update/run.sh cargo` sets `WIN11_TEST_HOST_NETNS="$(readlink /proc/self/ns/net)"` **before** entering
  bwrap, and the layer has `--tmpfs /run/user` and a `DBUS_SESSION_BUS_ADDRESS` outside `/run/user/` (F10).
* Bus tests for the engine paths that need the fake: install during a check → `update.err.busy` and no
  transaction (F16); after `Finished` the job's packages are gone from `snapshot.json` before the re-check answers,
  the rows are `installed`/`restart`/`failed`, the hold ends after the re-check (F2, F3); a confirm pause holds
  nothing and accept holds again (F7); a success clears `repairRequired` (F14); the owner's own page and a mirror
  see ≤ 4 / ≤ 1 progress per second (F8, on the real `Events`).

### 8.5 Decisions for the coordinator (replaces §7 where they differ)

1. Unchanged: accept `update.err.notHere` or map reader writes to `update.err.badArgs`.
2. §7 decision 2 is now settled by the safe rule (deviation 20), with no new text. Confirm, or ask for the
   stricter "refuse `update.install` while unclassified" (needs one new row).
3. F17: which design text wins, §3.7's list (implemented) or §4.3's "every write"? With §4.3 the user cannot
   cancel their own download while an upgrade started elsewhere runs.
4. F9: accept inotify instead of a GIO `FileMonitor` for design §4.8 (deviation 19).
5. Unchanged: deviation 2 (status after a failed seam); the keep-alive is now deviation 22.
