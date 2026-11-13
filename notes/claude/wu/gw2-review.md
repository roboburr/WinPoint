# GW2a adversarial review: the `update.*` service core (unit level), 2026-11-05

Reviewer: a separate agent. I did not write GW2a. Worktree W = `.wt/gw2` (branch `wu/gw2`, HEAD d666df54). I changed
no product file. I wrote only this note and the scratch files under `W/.rt/review-gw2/` (logs, strace traces, two
small Python helpers). Every heavy command went through `tools/heavy.sh` with a `gw2-rv-*` tag. I made no bus,
PackageKit, apt, network, win11-* or gsettings call outside the GE0b sandbox.

**Verdict: fix-needed.** I found no blocker. Every done-when check passes with the claimed numbers. The fail-closed
seam check (`paths::resolve`) holds against every bypass I tried. But one test-isolation layer of design §4.11 has
a hole (F1), and five design requirements are missing or behave wrongly in ways a user would see (F2–F6).

## 1. Done-when, run by the reviewer

| Check | Command (from W's root) | Result |
|---|---|---|
| Unit tests + oracles | `tools/heavy.sh --min-mb 2048 --tag gw2-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **306 passed** (271 + 35 `services::update::tests`), **16 ignored** (8 + the 8 `bus_tests` skeletons), 0 failed. win11-launch 5 passed, 1 ignored. The **16 oracles** are ok: 8 `oracle_tests::*`, 7 `disks::tests` python cases, `reference_contracts`. `guard-calls.log` is **0 bytes**, 0 tmp entries left. The run was repeated at the end (`gw2-rv-test2`) with the same result. Log: `.rt/review-gw2/test.log` |
| Flakiness | `tools/heavy.sh --tag gw2-rv-repeat -- bash -c 'for i in 1..6: run-tests.sh --lib services::update -- --test-threads=16'` | 6 × `35 passed; 8 ignored`, guard log 0 each time |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors, 0 warnings** (127 rows, 1396 new literals) |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 65 warnings: 64 `[unused]` for rows of GW4–GW9, plus 1 `[glossary]` `update.part.pixbuf`, which is explained correctly |
| cargo check | `tools/heavy.sh --tag gw2-rv-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0. lib 28 warnings and win11-launch 2, the same as the baseline. **None is in an update file**, `events.rs` or `services/mod.rs` |
| Build | `tools/heavy.sh --tag gw2-rv-build -- cargo build --offline --locked -j2` | rc 0, all **12** `target/debug/win11-*` binaries are up to date (19:08:26–27). `find host/src host/i18n -newer target/debug/win11-app` is empty, so the build covers the reviewed sources |
| Texts against design §3.7 | `python3 -I .rt/review-gw2/cmp_tsv.py W` | 100 rows. Every nl and en text is in §3.7 byte for byte, except the new `update.err.notHere`. No ASCII apostrophe anywhere |
| strace, all 35 update tests | `run-tests.sh --lib --config 'target.x86_64-unknown-linux-gnu.runner=["strace","-f","-v","-e","trace=execve,connect",…]' services::update` | **No connect to `/run/dbus/system_bus_socket` or `/run/user/*`.** There is 1 connect, to the GE0b private session bus (see F9). The execve calls are the test binary, 3 × the `python3` stub, 1 × the `apt-cache` stub and 2 × `/bin/cat`. No real tool ran. The stubs' environment is **exactly `["LANG=C.UTF-8"]`** (see F1) |
| Ownership | `git -C W status --short`, `git diff --stat` | Only owned files: `update.tsv`, `api-methods.json` (17 entries added, the 172 old ones byte-identical, still one sorted ASCII line), `events.rs` (+1), `services/mod.rs` (+11), `settings-defaults.json` (`update` added as the last top-level key), plus the new `update.rs`, `update/`, `BASELINE-CHANGES.rust-gw2`, `allow-host-gw2.txt` and `notes/claude/wu/`. `plan.md` is the coordinator's pre-existing change. Nothing frozen was touched |

## 2. Safety (design §4.11, D-G25)

What holds:
* **The seam check.** I tried these against `paths.rs`:
  * unset, empty and unknown `WIN11_UPDATE_*` variables;
  * euid 0;
  * relative paths, `..` and `/`;
  * a symlinked directory, a symlinked socket, an unmarked directory or one of another uid;
  * a world-writable directory;
  * `tcp:`, `unix:abstract=` and extra address keys;
  * `/run/dbus/…`;
  * a hard link to the system socket (refused by the socket's uid check and by `protected_hardlinks`);
  * a stub symlinked to `/usr/bin/pkexec` (refused by the canonicalize "inside" check).

  Every case fails closed. A failed seam leaves `shared` as None, so no store, thread, bus or tool exists for the
  process's whole life. Test mode never resolves to `BusTarget::System`.
* **`pk::system_bus()` under `cfg(test)`** panics without a valid process-level `WIN11_UPDATE_PK_BUS`. The only
  real `bus_get_sync(System)` is behind `cfg(not(test))`.
* **Dry-run, reader mode and the power guard** use files only. The guard never connects (tests read; strace agrees).
* **No absolute exec path** appears outside the `paths.rs` table, and no `Command::new` appears outside `paths.rs`.

What does not hold: F1 (child environment), F9 (the GVfs session-bus connection), and the narrower test gaps F10
and F11.

## 3. Findings

### Major

**F1. Tool-table children lose `DBUS_SYSTEM_BUS_ADDRESS` and the stubs-first `PATH`.**
* Where: `host/src/services/update/paths.rs:302-309` (`Tools::command`).
* The cause: `command.env_clear(); command.env("LANG", "C.UTF-8")` applies in test mode too. strace shows every
  stub runs with envp `["LANG=C.UTF-8"]` (`.rt/review-gw2/strace/trace.log`).
* Why it matters:
  * A child of a stub, or any bus client in it, falls back to the default system bus,
    `/run/dbus/system_bus_socket`.
  * Its PATH is the shell's default. `env -i LANG=C.UTF-8 /bin/sh -c 'command -v pkexec'` prints `/usr/bin/pkexec`,
    the real one, not `guard-bin/pkexec`.
  * In GE0b's non-bwrap unit sandbox, where these stubs run, the real system bus is reachable.
  * Design §4.11 requires `DBUS_SYSTEM_BUS_ADDRESS` to be the private bus "of **every** process (… workers,
    terminals)" and stubs first on PATH.
* Why not a blocker: today's stubs (`echo >> log; exit 97`, `cat`) never reach a bus. The hole opens as soon as a
  GW1, GW5 or GW7 stub is a wrapper or uses Gio.
* Fix: in test mode (`Tools::stubs`), set these on every command:
  * `PATH=<tool dir>` only;
  * `DBUS_SYSTEM_BUS_ADDRESS=<the checked seam bus>` (or `unix:path=<tool dir>/no-system-bus` when the bus is
    Disabled);
  * `DBUS_SESSION_BUS_ADDRESS` set to the same kind of dead address.

  In production, also set `PATH=/usr/bin:/bin`. Add a unit test in which a stub prints its env.

**F2. After a job, the snapshot keeps the old packages and `checking:true` when the keep-alive process exits.**
* Where: `pk.rs:1243-1278` (`finish_job`).
* The order of events:
  * `finish_job` sets `checking = true` and starts the post-job `GetUpdates` (1268-1269).
  * It then calls `persist()` (1272), which still writes the pre-install `packages` and `checking:true`.
  * Then `notify()` runs, and `hold_app(false)` at 1277 releases the hold.
  * When the window was closed, which is the C2 keep-alive path built for exactly this case, the use count drops to
    0. `run_with_args` returns, `Update::shutdown` quits the loop, and the GetUpdates reply is lost.
* What the user sees:
  * snapshot.json still lists the just-installed updates with `checking:true`;
  * readers show phase `checking`;
  * the next owner `load()`s the stale package list and offers "Alles downloaden en installeren" for packages
    that are already installed;
  * after GW2c, Home counts them, while the notification says "Je pc is up-to-date".
* Fix:
  * On success, drop the job's ids from `self.pkgs`. Rows whose packages are gone become "Geïnstalleerd", as
    §4.6 says.
  * Persist `checking:false`, or only persist it while an owner is alive.
  * Release the hold only after the recheck's `finish_check`, with a bounded timeout of about 60 s.
  * Flush the session bus after `Notify` in that path.

**F3. Rows never reach `failed` or `restart`, `installed` is lost at `Finished`, and `rows[].error` is always null.**
* Where: `pk.rs:291-317` (`row_states` only knows `installed`, `installing`, `downloading`, `pendingInstall` and
  `pendingDownload`) and `update.rs:603` (`"error":null`).
* The cause: `finish_job` drops the job, so every row is `pendingDownload` again right after `Finished`.
* What design requires:
  * §4.6 "After Finished": rows whose packages are gone show "Geïnstalleerd", and the rest show "Installatiefout"
    after a failure;
  * the §3.3 row states `restart` ("Wachten op opnieuw opstarten") and `failed`, with a per-row Retry;
  * state R03 ("install error with Retry per row").

  The host can never produce `failed`, `restart` or R03. `installed` (R02, "installing + installed") exists only
  while the job runs, and it disappears at `Finished`.
* Fix: keep a `lastJob` record in the snapshot (job id, row ids, outcome, failed and installed ids) until the next
  check. Derive `installed` for job ids that GetUpdates no longer lists, `failed` (with a row error key) for job ids
  that are still listed after a failure, and `restart` for rows whose packages sent RequireRestart. Test it as a pure
  function in model.rs.

**F4. The GW2c `system_updates()` treats the owner's first snapshot as authoritative.**
* Where: `update.rs:677-687`. The cause starts in `Engine::init` (`pk.rs:410-433`), which calls `persist()` at once.
* What happens:
  * Merely opening Settings › Update on a fresh install writes a snapshot with `checkedAt:null` and
    `packages:[]`, before any check.
  * `system_updates()` does not look at `checkedAt` or `backend`, so after the GW2c patch Home, About and
    Taakbeheer show **0 updates, tool `packagekit`**, while `apt list` would say N.
  * On a machine without PackageKit, the snapshot has `backend:null`. §4.5 says "`backend:'apt'` then keeps the
    count", but the count drops to 0 instead.
* Fix: return None unless `snapshot.checkedAt` is a number and `backend == "packagekit"`. Add that case to
  `system_updates_counts_the_snapshot_once_one_exists`.

**F5. The phasing fallback only runs when python3-apt is missing.**
* Where: `phasing.rs:207-215` and `pk.rs:1176` (`let _ = phasing::classify(…)`).
* The cause: `apt-cache show` runs only on exit code 3.
* What happens:
  * Any other classifier failure means no fallback at all: a traceback (exit 1, for example when
    `apt_pkg.Cache()` cannot parse lists that unattended-upgrades is rewriting), the 30 s timeout, or a missing
    `@@end`.
  * The error is also discarded, so every phased package enters a row and "Alles". Design §4.6 says phased
    packages **never** enter a row or `UpdatePackages`.
  * In test mode, a missing `python3` stub is swallowed the same way, but §4.11 says a missing stub is a
    `test_seam` error.
* Fix:
  * Run the fallback on every script failure.
  * If both fail, store `phasing:'unknown'` in the snapshot and apply the safe policy of the step's own
    decision 2: refuse `update.install`, or offer only security rows, with one new text.
  * Map `Failure::Seam` to the snapshot error `test_seam`.

**F6. `update.progress`, `update.request`, `update.reply` and `update.owner` reach every WebView of every host.**
* Where: `events.rs:355` (the prefix is allowed globally) and `events.rs:107-136` plus `dispatch()` (every received
  datagram goes to all SSE subscribers; `runtime.rs:428` has no name filter).
* Design §4.4 says panel, desktop, Start and Explorer "parse at most one small datagram a second and **never
  forward it to their WebViews**". In the current code, every shell WebView is woken once a second during a job. A
  mirror's forwarded `args` also reach every page.
* The cost is low, but it is a stated design requirement.
* Fix: in the receive loop, call only the observers (not the subscribers) for `update.request`, `update.reply` and
  `update.owner`. Deliver `update.progress` to subscribers only in `settings` hosts, through a small kind-aware
  filter set by `ServiceContext` (events.rs is GW2's to edit).

### Minor

**F7. The keep-alive is held during the Confirm pause.**
* Where: `pk.rs:679` (hold at install) and `pk.rs:1229-1232` (Confirm only persists).
* What happens: if the window closes while a removals or metered confirm is pending, the Settings process stays
  window-less (30–50 MB) until someone reopens Settings and answers through a mirror. No transaction runs in that
  state, so there is nothing to protect.
* Fix: `hold_app(false)` when entering `Step::Confirm`, and `hold_app(true)` again on accept.

**F8. The owner's own page can get 5 progress events per second.**
* Where: `pk.rs:555-572`.
* The cause: when the 1/s remote throttle fires, `emit()` also dispatches locally, but the local throttle is not
  updated.
* Evidence: `.rt/review-gw2/throttle_sim.py` re-implements `Throttle`, `progress()` and the 250 ms timer exactly. It
  finds page events at 0.924, 1.174, 1.236, 1.424 and 1.674 s, which is 5 within one second.
* Why the test passes: the unit test `progress_is_throttled_to_four_local_and_one_remote_per_second` checks the two
  `Throttle`s separately, not the real send path.
* Fix: when the remote throttle fires, mark the local one as sent too (`last = now`, `pending = false`), and test the
  combined counts.

**F9. The owner opens a session-bus connection through GVfs that the stats do not count.**
* Where: `pk.rs:478` (`gio::File::for_path(…).monitor_file`).
* Evidence: with `strace -k`, `g_file_new_for_path` → `g_vfs_get_default` → `libgvfsdbus.so` → `g_bus_get_sync`
  connects to the session bus (`.rt/review-gw2/strace/trace-k.log`). This happens in
  `two_settings_hosts_…`, which asserts "no bus opened" because `Stats.bus_opens` counts only the PackageKit target.
* Risk: in the sanctioned runners this is a private bus. A test run outside them would touch the live session bus.
  It is also GVfs work in Settings for two local files.
* Fix: `gio::Vfs::local().file_for_path(&path)`. Count every bus connection, `notify()` included, in `Stats`.

**F10. `bus_tests::fake_bus()` checks only one of the needed isolation properties.**
* Where: `bus_tests.rs:13-22`. It refuses a visible `/run/dbus/system_bus_socket` and nothing else.
* What it does not refuse: a visible `/run/user/<uid>/bus`, a `DBUS_SESSION_BUS_ADDRESS` under `/run/user/`, or the
  host's network namespace (abstract sockets). The service uses the session bus for `notify()` and through GVfs (F9).
* Fix: add those three checks, and compare `/proc/self/ns/net` with a value the runner records outside the layer.

**F11. The exec-path grep misses some paths and other ways to start a program.**
* Where: `tests.rs:283-291`.
* Missing path patterns: `"/sbin/`, `"/usr/sbin/` and `"/usr/local/`.
* Missing start APIs:
  * `gio::Subprocess`, `glib::spawn_async` and `spawn_command_line_*`, each with a bare name, which is a PATH
    lookup;
  * `CommandExt::exec`, `libc::exec*` and `posix_spawn`;
  * an aliased `use std::process::Command as …`.

  GW5 and GW7 add spawning code, so the gap matters there.
* Fix: add those patterns, and forbid any `process::Command` or spawn API outside `paths.rs`.

**F12. The upgrader detection has false positives, and they refuse power actions too.**
* Where: `guard.rs:134`. It accepts `DistUpgradeMain` or `/tmp/ubuntu-release-upgrader-` anywhere in argv[0..4].
* What matches: `less …/DistUpgrade/DistUpgradeMain.py`, `grep -r DistUpgradeMain`, or
  `ls /tmp/ubuntu-release-upgrader-x`.
* What follows: every update write is refused, and after GW2c reboot, shutdown and logout are refused with
  "Ubuntu wordt bijgewerkt naar een nieuwe versie".
* Fix: match only a python interpreter in argv[0] whose script argument is the unpacked upgrader or
  `do-release-upgrade`, plus the SCREEN rule. Add negative tests for an editor, a pager and grep.

**F13. Waiting for a lock or for permission shows as "downloading" during the simulate step.**
* Where: `model.rs:108-119`. `simulating` is checked before statuses 30 and 31, so a lock or auth wait during the
  simulate, or during a confirm, maps to Downloading.
* Effect: H07 and H08 ("waiting") cannot show in that step, but `waitLockAuto` can be true at the same time.
* Fix: test 30 and 31 before `simulating`.

**F14. `repairRequired` only clears through our own RepairSystem.**
* Where: `pk.rs:1085-1089`.
* What happens: a repair done elsewhere (Software Updater, `dpkg --configure -a`), or a later successful
  `UpdatePackages`, leaves the repair hero and its password-prompting Repair button in place.
* Fix: also clear it on a successful Update job.

**F15. The details cache has no limit.**
* Where: `pk.rs:352` and `pk.rs:1060-1065`.
* What happens: each cached id holds up to 70 KB of changelog and update text, and the cache is never cleared. That
  is far above §4.1's "about 1 MB of state".
* Fix: keep at most about 64 ids (LRU) and clear the cache on every new GetUpdates.

**F16. `update.install` is accepted while a check runs.**
* Where: `pk.rs:630-633`.
* What happens: the job uses the previous snapshot's ids, and a GetUpdates result races with the job.
* Fix: refuse with `update.err.busy` while `checking`.

**F17. Writes that should work during an upgrade are refused.**
* Where: `update.rs:335`. The upgrader guard covers every write except `release.attach`, so `update.cancel`,
  `details`, `setAutoOpen` and `ubuntuHold` are refused too.
* What design says:
  * §3.7 lists only check, install, confirm, repair and `release.check`/`compat`/`setPrompt`/`start`;
  * §4.5 1a lets an own job keep running and showing progress while the upgrader runs.

  The user then cannot cancel their own download.
* Fix: exempt `update.cancel` (and `details`), or ask the coordinator which of the two design texts wins.

### Notes

* N1: the new row `update.err.notHere` is listed in BASELINE-CHANGES with a reason. Its en text, "Updates can only
  be installed from Settings.", is also shown for `update.check` and `release.setPrompt` from a reader. That is
  acceptable; the coordinator decides on the row (the step's decision 1).
* N2: the snapshot format differs from §4.5:
  * it has no `source`, no `os.versionId` and no stored `rows`;
  * sizes are kept per package, and `requireRestart` is at the top level.

  This is consistent internally. Record it in the design's "as built" section.
* N3: §4.8's "page open" rescan trigger (when the last scan is older than 60 s) is missing. Scans run only at owner
  start, after check and job, on `UpdatesChanged` and on file events.
* N4: GetDetails always runs after a check, not only "if the page is open" as §4.6 says. It costs one extra daemon
  call.
* N5: `job.json` `seen:true` → `jobs.jsonl` (§4.5) cannot be done, because there is no API to acknowledge an
  outcome. Jobs go to `jobs.jsonl` at once. This is a design gap; GW3 and GW4 should decide.
* N6: `update.changed` is not held to ≤ 8 KiB. `error.detail` can be 4000 characters, and a removals confirm can
  hold 200 names. The datagram limit (60 KiB) still holds.
* N7: some signal callbacks call `Variant::child_value(0|1)` without a bounds check (`UpdateDetails`, the
  PropertiesChanged trampolines). A panic inside a GLib `extern "C"` trampoline aborts the process. Only the
  sender-filtered daemon (or GW1's fake) can trigger it.
* N8: `owner.lock` lives in `~/.cache`. If someone deletes `~/.cache/win11-gnome/update` while it is held, a second
  Settings host becomes owner too. `$XDG_RUNTIME_DIR` would be safer; this is the design's choice.
* N9: test mode without `WIN11_UPDATE_FIXTURE_ROOT` reads the live `/proc`, `/run/reboot-required` and
  `/var/log/apt` (read-only), and watches them with a FileMonitor. That is acceptable, but GW1's runner should always
  set the root.
* N10: `notify()` on `unknown_outcome` sends "Updates konden niet worden geïnstalleerd", although the outcome is
  unknown.
* N11: the keep-alive answer (lib.rs) checks out:
  * closing the window only drops the use count (`window.rs:880-910`);
  * `quit()` is reached only through Lifecycle (`window.rs:419`), `desktop-quit` (`lib.rs:359`), `start-quit`
    (`window.rs:456`) and a terminated web process of desktop, panel or Start (`window.rs:671-682`);
  * `NON_UNIQUE` is at `lib.rs:351`, and `runtime.shutdown()` at `lib.rs:527`.
* N12: the GW2c patch fits the current `info.rs` (`Updates` has `ctx`) and `power.rs` (the action names match).
  Apply F4's fix together with it.
* N13: the GW3 look check is not applicable: GW2a has no UI. Also, `system.updates` would parse the whole snapshot
  on every call in every process after GW2c. Consider caching it by (mtime, size).

## 4. What the step's report claims, checked

| Claim | Status |
|---|---|
| 306 / 16 / 0, oracles 16/16, guard log 0, 0 tmp entries | confirmed (twice) |
| 20 failing-seam cases, each `test_seam` on every method, no shared state, empty stub log, no bus connection | confirmed by reading and by the run. "No bus connection" holds for the PackageKit bus; see F9 for the session bus |
| The exec-path grep works | confirmed for its patterns; it has gaps (F11) |
| Phased and held never reach a row or `UpdatePackages` | true when the classifier answers; false on classifier failure (F5) |
| Two Settings hosts without a bus: forwarding, reply, takeover | confirmed |
| Dry-run makes zero calls; reader mode never connects | confirmed (stats, stub log, socket accept, strace) |
| Progress ≤ 4 local and ≤ 1 remote per second | ≤ 1 remote is confirmed; local can reach 5 (F8) |
| drift 0/0, check_rust 0 errors and 65 warnings, cargo check at the baseline, 12 binaries built | confirmed |
| Keep-alive: no lib.rs change needed | confirmed (N11), but see F2 and F7 for what the hold does |
