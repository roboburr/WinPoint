# GW2b + GW2c adversarial review (2026-11-05)

Worktree `.wt/gw2b` (branch `wu/gw2b`, HEAD f01710cc, nothing committed). Reviewed against design §3.2-3.4,
§4.1-4.11, §9.1-9.2 and the GW2 row of §10, plan D-G25..D-G29 and D-G38..D-G42, contract C10, `wu/gw2.md` §4, §6 and
§8.4, `wu/gw3.md` D1, the page `vendor/ui/js/apps/settings/pages/update.js` and its mock. I edited no product file.
My scratch files are in `.rt/review-gw2b/` (logs, extracted strace logs, `neg.sh`, a `git archive HEAD` copy). Every
heavy command went through `tools/heavy.sh` (tags `gw2b-rv-*`). Nothing touched the real system bus, the live
session bus, PackageKit, apt, the network, gsettings or a win11-* binary outside a sandbox.

**Verdict: fix-needed.** I found 1 major, 8 minor and 9 notes, and no blocker. Safety holds. Strace, nine
negative controls inside the layer and the C10 unit cases found no path to the real system bus, the live session
or the network. Every done-when number in the step's report reproduced exactly. The major finding is that the bus
tests poll `update.status`, while the page only reloads on `update.changed`. Because of that, the tests miss that
the hero's phase and `job.lockHolder` never reach the page while a job runs.

## 1. Done-when, run by me (from W's root)

`BIN = target/debug/deps/win11_host-12500e7eb53cf9c1`. No file under `host/src`, `host/i18n` or `tests/update` is
newer than BIN.

| # | Command | Result |
|---|---|---|
| 1 | `tools/heavy.sh --min-mb 2048 --tag gw2b-rv-build -- tests/rust-sandbox/run-tests.sh --lib -- --list` | rc 0; 387 tests listed (8 `bus_tests::*`); BIN unchanged |
| 2 | `W11_KEEP_RUNS=12 tools/heavy.sh --min-mb 2048 --tag gw2b-rv-bus -- tests/update/run.sh bus-tests -- $BIN` | rc 0, **8 passed, 0 failed**, 2:51 wall. `every_timeline_in_nl_and_en`: **62/62** ok in 33.35 s, stub log 78 OK, CallCount 32 OK. Flood nl and en: **252 page events, at most 4 in any second; 63 fan-out events, at most 1 in any second** (126.77 s). Shutdown during a transaction **0.36-0.48 ms**, idle 0.31 ms. Two-services, upgrade-running, dry-run (0/0), reader (0/0), shutdown and vanished-daemon all passed with their stub and call checks. `CRITICAL` in the log: **0**. `strace: OK` in all 8 runs |
| 3 | My own tally of the 8 extracted strace logs (`.rt/review-gw2b/strace/`) | 188 successful `connect()` calls, all to `/tmp/w11-update/dbus-*`. 324 failed `connect()` calls, **all ENOENT** (`/var/run/nscd/socket`, `/var/lib/sss/pipes/nss`). 0 abstract sockets, 0 AF_INET. Addressed datagrams only in the flood run (144) and the two-services run (17), all to `/tmp/w11-update/tmp/b*/events/p16-*.sock` (the layer's tmpfs) |
| 4 | `W11_KEEP_RUNS=12 tools/heavy.sh --min-mb 1024 --tag gw2b-rv-selftest -- tests/update/run.sh --self-test` | rc 0, **SELF-TEST PASSED**, 1:42.7. **32/32** timelines, **15/15** checks, strace OK (107,451 calls in 76 processes), guard **25/25**, outside the layer **26/26** (decoy connections 0) |
| 5 | `tools/heavy.sh --min-mb 2048 --tag gw2b-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **371 passed**, 0 failed, **16 ignored**; win11-launch 16 passed, 1 ignored; 11 other bins 0 tests; the oracle tests ok. The 5 new tests ok. **guard-calls.log 0 bytes**, tmp left 0 |
| 6 | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors, 0 warnings** (541 rows, 2160 new literals; the HEAD copy has 2112) |
| 7 | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings. The sorted warning lines are **identical** to those of a `git archive HEAD` copy (`.rt/review-gw2b/check_rust-head.log`) |
| 8 | `tools/heavy.sh --min-mb 2048 --tag gw2b-rv-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0. Warnings: lib 28, lib test 26, launch 2, launch test 1 (= GW2a's baseline). **0** in `update/*`, `system/info.rs`, `system/power.rs` |
| 9 | `python3 -B tests/i18n/test_update_mock_rows.py` | OK (5 tests, 1 skipped) |
| 10 | `tools/heavy.sh --min-mb 1024 --tag gw2b-rv-neg -- tests/update/run.sh exec --expect-calls 0 -- /bin/sh .rt/review-gw2b/neg.sh $BIN` (negative controls, §2) | **9/9 seam breaks fail closed** (rc 101 each) and the positive control passes; stub log 0, CallCount 0, strace OK |
| 11 | `tools/heavy.sh --min-mb 1024 --tag gw2b-rv-power -- tests/update/run.sh exec --timeline idle-none --expect-calls 0 -- $BIN services::system::power::update_guard_tests services::system::info::tests::system_updates_come --test-threads=1` | The info test is ok. Inside the layer the power test panics at `power.rs:256` (no boot id under the fixture root) **before** any `action()`; stub log 0, strace OK (fails closed; see m7) |
| 12 | `tools/heavy.sh --min-mb 1024 --tag gw2b-rv-noname -- tests/update/run.sh exec --expect-calls 0 -- $BIN services::update::bus_tests::reader_mode_never_connectz --exact --ignored --test-threads=1` | `running 0 tests`, **rc 0** (m3) |
| 13 | `git status --short`, `git diff --stat` | 43 files, all owned: the 10 host files, `tests/update/{run.sh, fake_packagekit.py, README.md}`, 30 timelines, new `stubs/bus-tests.json` and `gw2b.md`. No frozen path. `update.tsv`, `api-methods.json` and `vendor/ui` are unchanged |

## 2. Safety (design §4.11, D-G25, C10)

* **The C10 check (`paths.rs:473` `check_bus`)** is exact. It accepts one `unix:path=` with at most one 32-hex
  `,guid=`. The socket must sit directly in `$XDG_RUNTIME_DIR`, which must not be under `/run` (also checked through
  `canonicalize`), must be a 0700 directory of the euid and must hold the marker. The check refuses while
  `/run/dbus/system_bus_socket` or `/var/run/dbus/system_bus_socket` is visible. The 28 unit cases each assert their
  reason, and the whole service ends `unsupported`/`test_seam` for each one.
* **Negative controls in the real layer** (`neg.sh`, one bus test with one seam broken each). Each case panics in
  `fake_bus()`/`process_setup()` before any service starts, with the expected reason:
  * `XDG_RUNTIME_DIR=/run/user/1000`
  * `XDG_RUNTIME_DIR` unset
  * an unknown `WIN11_UPDATE_EXTRA`
  * `DBUS_SESSION_BUS_ADDRESS` under `/run/user`
  * `WIN11_TEST_HOST_NETNS` equal to the layer's own namespace
  * `WIN11_TEST_HOST_NETNS` unset
  * a tcp `WIN11_UPDATE_PK_BUS`
  * the marker removed
  * the runtime directory at mode 0750
* **Under `cfg(test)`, `pk::system_bus()` panics without a valid test bus** (`pk.rs:52-63`). An unset seam can never
  reach the daemon from the lib test binary.
* **`notify()` uses the session bus from the environment.** Since C10 now requires an invisible system-bus socket,
  test mode implies a private `/run`. The live session bus is then invisible as well (abstract sockets are cut off by
  `--unshare-net`).
* **GW2b adds no `Command::new`, spawn, `set_var`, kill, gsettings or dconf call** (grep of every added line).
* **`--allow-layer-dgrams`** exempts only `/tmp/w11-update/tmp/` inside the layer's own `--tmpfs /tmp`. strace
  resolves that path in the layer's mount namespace, so the exemption cannot reach a host socket. Only the two
  tests with two hosts use it. I recommend accepting it (n1).
* **The power test** calls the real `action("reboot"|…)` behind the guard. It stays safe in GE0b's sandbox (guard +
  guard-bin stubs) and fails closed in the bus layer, but the gate is weak (m7).

## 3. Findings

### M1 (major): during a job the page never learns about phase changes, so `job.lockHolder` never reaches it and the bus tests cannot see this

* **Evidence.**
  * **Host:** `pk.rs:1015-1070` (`job_progress`) folds every `PropertiesChanged` (Status 30/31/8/10, AllowCancel,
    committing) into the job. It then only rewrites `job.json` (`write_job`, `pk.rs:1066`) and sends
    `update.progress`. `on_tx_signal` calls `persist()` (the only place that emits `update.changed`) only on
    `Finished`/`Destroy`.
  * **Page:** `update.js:753` reloads only on `update.changed`. The `update.progress` handler (`update.js:756-763`)
    patches only `status, pct, speed, remaining, downloadLeft, allowCancel` and the rows. The hero switches on
    `s.phase` (`update.js:361-397`).
  * **Mock:** the page's contract emits `update.changed` at the download → install transition (`mock/update.js:395`
    `s.phase = 'installing'; changed(m, u)`).
  * **Observed on the real Events** (flood run, strace payloads of the owner → second host datagrams):
    `3 update.changed, 1 system.updates.changed, 1 update.changed, 1 update.progress, 1 update.changed,
    **62 update.progress**, 1 update.changed, 1 system.updates.changed, 1 update.changed`. The flood timeline goes
    from `download` to `update` (`AllowCancel false`, committing) after the 60 s, yet no `update.changed` comes
    between the progress events.
* **Effect on the real page.** From the start of the real `UpdatePackages` until `Finished`, the hero stays in the
  phase persisted at the update start (stage of the simulate's last status = "downloading"):
  * H06 "Installing - keep your PC on" is never shown through events;
  * H07 "Ubuntu is installing automatic updates right now" / H08 "waiting for permission" are never shown either;
  * D-G41's `job.lockHolder:'unattended'` is computed by `compose` but never delivered, because status 30 arrives
    between persists.
* **Why the tests miss it.** The bus tests poll `update.status` every 5 ms (`bus_tests.rs:295-305`), which composes
  afresh. `lock-wait`/`auth-wait` (`bus_tests.rs:724-744`) and `download-install` therefore pass, while a page that
  follows the events would not show these states.
* **Fix (owned files).**
  * In `job_progress`, compare `model::stage(status, committing, simulating||confirm)`, `committing` and the
    lock-holder flag (status 30 and `guard::unattended_running`) before and after the fold. Call `self.persist()`
    when any of them changes (a handful per job, far below any rate limit).
  * In `bus_tests.rs`, record `data["phase"]` and `data["job"]` of every `update.changed` in `Seen`. Assert that
    `lock-wait` (nl) sees `waiting` with `job.lockHolder == 'unattended'`, that `auth-wait` sees `waiting` with
    status 31, and that `download-install`/`flood` see `downloading` and then `installing` before `Finished`.
* **Alternative.** The page derives the stage from `update.progress.status`. That needs a GW3 change and still
  leaves `lockHolder` undelivered.

### m1 (minor): the `totals.downloadSize` fill for a simulate without a total does not reach the page and is untested

* **Evidence.**
  * `pk.rs:1308`/`1315`: a simulate reporting `DownloadSizeRemaining 0` stores and persists `Some(0)`.
    `update.rs:623` then lets `snap["downloadSize"]` = 0 win over the job's later value. The unit test asserts
    "a snapshot total wins".
  * The fill (`pk.rs:1029-1037`) sets `job.download_size`/`self.download_size` without `persist()`. Even in the
    `None` case, the page's last loaded `totals.downloadSize` stays null for the whole download (see M1: no
    `update.changed` until `Finished`).
  * The fake always reports the simulate's total from the package sizes (`fake_packagekit.py:1116-1123`), so no
    timeline reaches this path. `download-install` only covers the simulate-total path.
* **Fix.**
  * Treat 0 as "unknown": `snap["downloadSize"].as_u64().filter(|s| *s > 0)` in `compose`, and
    `tx.download_left.filter(|s| *s > 0)` in `on_simulated`.
  * Call `persist()` once when the fill happens.
  * Add a fake/timeline option for a simulate without a size, and assert on the `update.changed` payloads.

### m2 (minor): `UpdatesChanged` never re-reads the update list; the bus test asserts the stale behaviour

* **Evidence.**
  * `pk.rs:1532-1535` only rescans the restart state and calls `persist()`.
  * `updates-changed` (`bus_tests.rs:638-645`) asserts `pk == [StateHasChanged]`, but the timeline's own reference
    flow re-reads with `GetUpdates` after `UpdatesChanged`
    (`{"tx":"GetUpdates","args":["filter:none"],"expect":{"packages":2}}`).
* **Effect.** Packages that apt or unattended-upgrades install while Settings is open (apt's 20packagekit hook)
  stay offered on the page and in `system.updates` until the next manual check.
* **Fix.** On `UpdatesChanged`, while no check and no job of ours runs, run a debounced (~2 s) `GetUpdates` without
  `RefreshCache`, followed by the classifier. The bus test then expects `StateHasChanged + tx("GetUpdates")` and
  one stub call. The design does not spell this out, so this is a coordinator call.

### m3 (minor): `run.sh bus-tests` passes a test that does not exist, and nothing checks that the JSON lists every bus test

* **Evidence.** Check 12: a misspelt name gives `running 0 tests` and rc 0. `cmd_bus_tests` (`run.sh:512-560`) counts
  that as passed. For `dry_run_*` and `reader_mode_*` (expected stubs `[]`, calls 0), a renamed test would turn
  green silently. A new `#[ignore]` test in `bus_tests.rs` that is missing from `stubs/bus-tests.json` never runs.
* **Fix.**
  * Require `test result: ok. 1 passed` in the test's output.
  * Before the loop, compare `BIN --list --ignored` (filtered to `services::update::bus_tests::`) with the JSON keys
    and fail on any difference.

### m4 (minor): the fan-out rate is asserted over 900 ms, not one second

* **Evidence.** `bus_tests.rs:1011` uses `most_in(&wire, 900 ms) <= 1`. Two datagrams 950 ms apart pass, so a
  regression to about 1.05/s would stay green. The product is fine today: the logged 1 s window gave at most 1.
* **Fix.** Count the owner's `Delivery::Both` sends at their send time with a 1 s window. Alternatively, assert
  `wire.len() <= elapsed_seconds + 1` next to the jitter-tolerant window.

### m5 (minor): three different `rebootRequired` rules

* **Evidence.**
  * `system.updates` (the GW2c hook, `update.rs:742-750`) uses `/run/reboot-required` only.
  * The owner's `system.updates.changed` (`pk.rs:564`) sends `view["restart"]["reboot"]`, which is the file or a
    RequireRestart record of this boot.
  * `update.status` `restart.reboot` uses the latter as well.
* **Effect.** After a job whose RequireRestart(system) came without the file, the event says true and the next
  `system.updates` says false. The mock's comment says "/run/reboot-required or the snapshot".
* **Fix.** In `system_updates_from`, use the same rule as `compose`: the file, or a `requireRestart` of this boot of
  type 4/6.

### m6 (minor): two-services gaps

* **Evidence.**
  * "Only the owner calls Notifications" is checked by count only (`notify: 1`), not by sender; `CallsJson()`
    records the sender.
  * Takeover is tested only through `shutdown()` with `update.owner {gone}`. The 30 s retry of a mirror whose
    owner died without `shutdown()` (the crash case of design §4.1) has no bus test.
* **Fix.**
  * Assert that the `Notify` sender is the former mirror's unique name.
  * Add a takeover-by-retry case with a test-only short retry, or record the gap in `gw2b.md`.

### m7 (minor, defence in depth): the power test runs the real power commands behind a weak gate

* **Evidence.** `power.rs:268` `in_unit_sandbox()` only checks that PATH's first entry ends in `/guard-bin`. In that
  case `action("reboot"|"shutdown"|"suspend"|"logout")` runs after the guard refused. In GE0b's sandbox the real
  system bus is visible, so the guard plus the guard-bin stubs are the only protection. Inside the bus layer the
  test fails at `power.rs:256` (check 11): it assumes no seams. It fails closed, but the result depends on the
  environment.
* **Fix.**
  * Also require `systemctl`, `gnome-session-quit` and `loginctl` to resolve to files in that guard-bin, and
    `DBUS_SYSTEM_BUS_ADDRESS` to be the sandbox placeholder.
  * Skip, with a message, when seams are set instead of panicking.
  * Or test the early return through `guard::power_blocked_in` plus a check that `action` returns before
    `compositor()`.

### m8 (minor, coordinator): the `system.updates` doc string is out of date

* **Evidence.** `api-methods.json` still says `{available:int|null, checkedAt, tool:'apt'|null, checking, security}`.
  After GW2c, `tool` can be `'packagekit'` and `rebootRequired` is always present. GW2b may only edit `update.*`
  doc strings, so it could not change this one.
* **Fix.** The coordinator updates that doc string at merge.

## 4. Notes

* **n1. `--allow-layer-dgrams`:** safe as built (§2); accept. It could be narrowed to
  `$RT/tmp/b*/events/`.
* **n2. The run-level `--expect-calls` covers only each test's last scenario** (calls since the last `Load()`).
  The per-scenario checks of the other 61 scenarios live inside the Rust test, so they are not an independent
  oracle. The run-level stub check is whole-run (78) and does catch drift. A per-scenario summary file written to
  `<run>/` and checked by `run.sh` would make it independent.
* **n3. The dual-format phasing stub hides a real divergence.** `probe.py`'s `PHASING_SCRIPT` (5 columns) is not
  `phasing.rs`'s `SCRIPT` (7 columns + `@@end`). R1's `probe.py --live --phasing` would therefore not measure the
  script that ships, although design §9.1 says "the same constant script". Coordinator: one shared script, read
  by both.
* **n4. EACCES counts as "not visible".** `paths.rs:500` and `fake_bus()` (`Path::exists`) treat EACCES on the
  system-socket path as "not visible". That only matters where the socket is unreachable anyway. Stricter: refuse
  on any error but NotFound.
* **n5. The bounded 80 ms wait in `shutdown()`** keeps "no join" and measured < 0.5 ms; accept.
* **n6. Wording of `update.err.busy`.** It is returned during a *check* (F16, GW2a) and says "An update is already
  being installed.". §3.7 defines the key for "our job already running". The coordinator may want a check-specific
  text.
* **n7. `vanish-cancels-download` ends as `update.err.failed`** with rows `failed` although nothing was installed
  (deviation 7, GW2a's mapping). Leave it until R3.
* **n8. The power test carries the Dutch expected text** as an oracle, like the round-2 tests (`system.rs:659`);
  drift is clean.
* **n9. `a_vanished_daemon_*` asserts that we re-activate packagekitd ourselves** through GetOldTransactions
  (activations 1), while §4.6 says "when the daemon is back". That is GW2a's behaviour and fine for a crash; note
  it for R3.

## 5. Conformance summary

**Met:**
* GW2c: the guard comes first in `action()`; lock passes; `system.updates` comes from the snapshot without
  `apt list`; three `#[allow(dead_code)]` lines removed; tests.
* C10 exactly, with a unit test per refusal (28).
* `job.simulating`, `job.foreignOwn`, `restart.shellParts` (the 12 page codes, cross-checked against update.js's
  `PART`) and `totals.downloadSize` on the simulate path: all present in `update.status` and delivered at persist
  points.
* The 8 bus tests cover every item of the task list in nl and en inside the layer, with stub log, `CallCount()` and
  `violations` checks.
* F10 (`WIN11_TEST_HOST_NETNS`, tmpfs `/run/user`, a session bus outside `/run/user`).
* GW1's self-test is still green.

**Not met:**
* `job.lockHolder` and the in-job phase changes do not reach the page (M1).
* The `totals.downloadSize` fill path does not reach the page (m1).

**Deviations 1-7 of `gw2b.md`:** acceptable as described (n1, n3, n5, n7). Decision 4 (the design text: 32
timelines, `run.sh bus-tests -- BIN`) is correct.
