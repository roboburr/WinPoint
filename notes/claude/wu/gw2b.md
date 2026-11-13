# GW2b + GW2c: bus tests, the two system hooks, contract C10, the page's snapshot fields (2026-11-05)

Worktree `.wt/gw2b` (branch `wu/gw2b`, from f01710cc; at the start `git status --short` was empty and HEAD was
f01710cc). Design: `notes/claude/windows-update-gnome.md` §3.2-3.4, §4.1-4.11, §9.1, §9.2, §10 (GW2 row);
`plan.md` D-G25..D-G29, D-G38..D-G42; contract C10; `wu/gw2.md` §4, §6, §8.4; `wu/gw1.md`; `wu/gw3.md` §8 D1.
Nothing was committed. No real system bus or live session bus was contacted, no PackageKit, apt, pkcon, network,
gsettings, win11-* binary, gnome-shell, pkexec or sudo was used outside a sandbox. Every heavy command went through
`tools/heavy.sh` (tags `gw2b-*`). Scratch files are in `.rt/gw2b/` (logs, the timeline generator `gen_stubs.py`).

## 1. Files

| File | What changed |
|---|---|
| `host/src/services/system/power.rs` | GW2c hook: `action()` first asks `update::guard::power_blocked(ctx, action)` (refuses reboot/shutdown/suspend/logout, never lock); test module `update_guard_tests` |
| `host/src/services/system/info.rs` | GW2c hook: `Updates::get` returns `update::system_updates()` once a snapshot with a finished check exists; the apt path adds `rebootRequired`; `Updates::check` returns early when a snapshot appeared; test `system_updates_come_from_the_update_snapshot_once_one_exists` |
| `host/src/services/update.rs` | the two `#[allow(dead_code)]` above `system_updates`/`reboot_required` removed; `compose` emits `job.simulating` (also from job.json), `job.lockHolder`, `job.foreignOwn`, `job.downloadSize`, a job object for a foreign transaction, `totals.downloadSize` from the running job; `Stats.held` (the keep-alive, read by the bus tests); `shutdown()` waits at most 80 ms for the pk thread to drop its engine |
| `update/guard.rs` | the `#[allow(dead_code)]` above `power_blocked` removed |
| `update/paths.rs` | contract C10 in `check_bus` (below); `Env` carries `$XDG_RUNTIME_DIR` and the system-bus socket paths; `SHELL_MAPS` with the page's component codes |
| `update/restart.rs` | history fallback: JavaScriptCore is `jsc` |
| `update/pk.rs` | `Handle::stop(wait)` with a done channel; the engine closes a *private test* bus connection on drop (never the shared system bus); `Job::view`/`job.json` carry `downloadSize`/`simulating`; the first `DownloadSizeRemaining` of a download fills a missing total; a successful job clears the stored error and the job's total; the keep-alive flag lives in `Stats.held`; `unboxed()` (fix below) |
| `update/net.rs` | `unboxed()` for the NM reads |
| `update/tests.rs` | `Fixture::env()` meets C10; new `the_test_bus_follows_contract_c10`, `shell_parts_use_the_page_codes`, `the_snapshot_job_carries_the_fields_the_page_reads` |
| `update/bus_tests.rs` | the 8 skeletons filled (1305 lines, §3) |
| `tests/update/run.sh` | `WIN11_TEST_HOST_NETNS` (read outside, before bwrap) in every layer; seam dirs `fixture`/`tools` 0700 whatever the umask; `exec --allow-layer-dgrams`; new `bus-tests` subcommand; `W11_KEEP_RUNS`; usage prints the whole header |
| `tests/update/fake_packagekit.py` | the layer driver chmods `fixture`/`tools` 0700; stub matchers may carry `"count"` |
| `tests/update/timelines/*.json` (30) | a dual-format `python3` phasing answer (§5, deviation 2) |
| `tests/update/stubs/bus-tests.json` (new) | per bus test: expected stub log, calls since the last Load, timeout, datagrams |
| `tests/update/README.md` | the above |
| this note | |

Not changed: `host/i18n/update.tsv` (no new text was needed), `api-methods.json` (the `update.status` doc names
the snapshot generically; the new fields are additive), every frozen file, `update/history.rs`/`options.rs` (GW4),
`shell.rs`, `lib.rs`, `file_portal.rs` (GX1), `vendor/ui`. No `BASELINE-CHANGES.rust-gw2b` or
`allow-host-gw2b.txt` was needed (drift and ratchet are clean).

## 2. The four parts of the task

**GW2c (gw2.md §4).** The patch applied as written to R2-A's converted files (both still had the shapes gw2.md
assumed). The three `#[allow(dead_code)]` lines are gone. Tests:
* `power_actions_are_refused_while_our_update_job_commits_and_lock_passes` (nl and en): a `job.json` of this boot,
  `committing:true`, owner = this test process → `guard::power_blocked` refuses reboot, shutdown, suspend and
  logout with `update.guard.power`, and `action()` itself returns `update_busy` with the exact nl/en text. `action`
  is only called inside GE0b's sandbox (guard-bin first on PATH) and only after the guard refused, so no `systemctl`
  can run even if the guard were broken; `guard-calls.log` stayed 0 bytes. `lock` passes the guard; `bogus` passes
  it and gets `system.power.unknownAction`; a finished job no longer blocks.
* `system_updates_come_from_the_update_snapshot_once_one_exists`: a snapshot with `checkedAt`, `backend:packagekit`
  and 5 packages (1 security, 2 normal, 1 phased, 1 held) → `{available:3, security:1, tool:'packagekit', checkedAt,
  checking:false, rebootRequired}`; neither `get(true)` nor `check()` started `apt list` (state untouched, guard log
  empty).

**C10 (contracts.md, D-G39).** `paths::check_bus` now accepts `WIN11_UPDATE_PK_BUS` only as one `unix:path=` with at
most one `,guid=<32 hex>`; the socket directly in `$XDG_RUNTIME_DIR`; that directory absolute, not under `/run` or
`/var/run` (also not through a symlink: the canonical path is checked too), owned by the euid, mode exactly 0700,
holding the marker; the socket a socket of the euid (not a symlink); and only while
`/run/dbus/system_bus_socket` and `/var/run/dbus/system_bus_socket` are not visible. The old rule refused the
`,guid=` that dbus-daemon prints, so GW1's address would have failed. `the_test_bus_follows_contract_c10` has one
case per refusal, each asserting its reason (28): tcp, abstract, `runtime=yes`, a second address, a guid that is
not 32 hex, two guids, another key, an escaped path, an empty path, a relative path, `..`, the system bus, the live
session bus, the system-bus socket visible, `XDG_RUNTIME_DIR` unset / relative / under /run / a symlink to /run /
missing / mode 0750 / mode 0711 / another uid / without marker, the socket in a sub directory, the socket outside the
runtime dir, no socket, a plain file, a symlink to the socket; plus the two accepted forms, and the whole service
failing closed (`unsupported`, `test_seam`) for each. The default `Env` checks the real socket paths (in GE0b's
sandbox the same valid seams fail with "the real system bus socket is visible").

**D-G41 / GW3 D1.** Names exactly as `update.js` reads them: `job.simulating`, `job.lockHolder` (`'unattended'`
while status 30 and a fixture/real `comm` is `unattended-upgr`, else null), `job.foreignOwn`, `restart.shellParts`
(webkit, jsc, gtk, gdk, soup, glib, gio, gobject, pixbuf, cairo, archive, self), `totals.downloadSize` (the
simulate's `DownloadSizeRemaining`, else the running job's first one). In the `foreign` phase without a job of ours
`job` is `{jobId:null, foreign:true, foreignOwn, …}` (the mock's H09/H26 shape). The GW2a top-level `foreignOwn`/
`waitLockAuto` stay (additive). Unit tests: `shell_parts_use_the_page_codes` (also checks every code against
update.js's `PART` table when the repository is there), `the_snapshot_job_carries_the_fields_the_page_reads`; the
bus tests check them against the fake (lock-wait, foreign-transaction, removals/metered, download-install).

**GW2b.** §3. Run: `tools/heavy.sh --min-mb 1024 --tag gw2b-bus -- tests/update/run.sh bus-tests -- "$BIN"`.

## 3. The bus tests (`host/src/services/update/bus_tests.rs`)

Every test runs in its own `run.sh exec` (fresh run dir, buses, fake). `fake_bus()` refuses to run unless the
system-bus socket and `/run/user/<uid>/bus` are invisible, `DBUS_SESSION_BUS_ADDRESS` is not under /run/user, the
network namespace differs from `WIN11_TEST_HOST_NETNS`, and the process seams pass C10. A scenario: reset the
fixture root (boot id, os-release, `run/`, `var/log/apt/`), `Load` + `Run`, start the service(s) with
`Update::new` (the process's own seams), drive the API as the page does, then `verify()`: the fake's PackageKit
calls **in order** (`CreateTransaction`, `SetHints`, method, …), NetworkManager reads and `Notify` calls by count,
`CallCount() == len(CallsJson())`, fixed arguments (RefreshCache false, GetUpdates 2, UpdatePackages 6 then 2 with
the simulated ids, RepairSystem 2, GetOldTransactions 5, NO_AUTO_START on every NM read, GetTransactionList,
GetAll and Get), the stub log since the scenario began (only `python3 -I -c` in the tool dir, rc 0, the expected
number), and `State().violations == []`. Texts are compared with `update.tsv` via `i18n::trf` in the language.

| Test | Timeline(s) | Asserts |
|---|---|---|
| `every_timeline_in_nl_and_en` | the 31 below × nl, en | the timeline set on disk is exactly these 31 + `flood` (32); failures of all 62 scenarios are collected and reported together |
| ↳ idle-none | | idle, no rows, no stub call (empty list), Refresh + GetUpdates only |
| ↳ available-mixed | | **F16**: install during the check → `update.err.busy`, no UpdatePackages; 8 rows in design order with counts 4,4,2,2,2,3,3,2 and versions (26.04.1 LTS, 50.1, 2.52.7, 3.24.52, 3.6.6, 2.88.0, 7.0.0.43.43, –); totals 22/11 security/2 phased; phased = ubuntu-drivers-common, fwupd; `from` from the classifier; GetDetails only for the 22 offered; stub stdin 24 `name\tarch` lines |
| ↳ only-phased | | idle, totals 0 / 2 phased, no GetDetails |
| ↳ phased-as-normal | | rows security+other, install → both installed, the phased id never in UpdatePackages (forbiddenIds: no violation) |
| ↳ refresh-partial-fail | | available, one `update.warn.source` with `{source: dl.google.com}` |
| ↳ refresh-offline | | check → `update.err.offline`, phase offline, network none; no PackageKit call |
| ↳ update-details | | NM absent; sizes from `download-size`, totals.downloadSize 22,345,750; `update.details` → 3 entries (text, https CVE/bug links, changelog, restart 2), cached on the second call (no call), unknown id → `update.err.stale` |
| ↳ updates-changed | | the test plays apt's hook (`StateHasChanged`): UpdatesChanged → a new rev, no transaction |
| ↳ daemon-absent | | unsupported, `update.err.noPackageKit` in the snapshot and for check/install, no activation |
| ↳ daemon-idle-activatable | | backend packagekit while idle, the check activates it (activations 1) |
| ↳ download-install | | **F2/F3**: the first snapshot after Finished has `checking:true`, no package of the job, keep-alive still held; the last one `checking:false`, not held; a snapshot during the job has `downloadSize`/`totals.downloadSize` = 134,545,408; rows webkit/gtk/other installed; job.json outcome exit 1; one jobs.jsonl line; one notification |
| ↳ cancel-download | | cancel while status 8 and AllowCancel → `Cancel` on the bus; `update.err.cancelled` warning, rows pendingDownload, no notification |
| ↳ cannot-cancel | | in the commit phase `update.cancel` → `update.err.cannotCancel` **without** a Cancel call; the test is the timeline's second client whose Cancel PackageKit refuses; then installed |
| ↳ auth-wait / lock-wait | | phase waiting with status 31 / 30; lock-wait nl with `unattended-upgr` in the fixture procfs → `job.lockHolder:'unattended'`, en → null; then installed |
| ↳ not-authorized, no-space, untrusted, install-blocked, cancelled-priority, conflict | | phase error with `update.err.notAuthorized` / `noSpace` / `untrusted` / `conflict` / `locked` / `conflict`, the detail, every remaining row `failed` with the rendered error; conflict fails in the simulate (no real UpdatePackages); one "failed" notification |
| ↳ dpkg-fail-halfway | | rows gtk installed, glib failed, other failed; `update.err.dpkg` with dpkg's detail; ids in design order |
| ↳ success-rows-remain | | exit 1 while GetUpdates keeps jsc: webkit pendingDownload (count 1), other installed, no error |
| ↳ removals-in-simulate | | confirm `{kind:removals, removals:[libwebkit2gtk-4.0-37, libjavascriptcoregtk-4.0-18, tzdata-legacy]}`; **F7**: nothing held during the confirm; nl accepts (held again at once, installed), en declines (no transaction, no re-check, no notification, rows pending) |
| ↳ metered | | confirm `{kind:metered, size:29074626}`, totals.downloadSize the same, not held; `Step()` turns NM GUESS_YES (one re-read, still metered); accept → held, installed |
| ↳ require-restart-system | | restartRequired, rebootPkgs = the two of `/run/reboot-required.pkgs` + RequireRestart's; rows security installed, kernel **restart** |
| ↳ repair-required | | repairRequired with `update.err.repair`; `update.repair` → RepairSystem(2); **F14**: repairRequired false and the error gone, phase available |
| ↳ foreign-transaction | | started on the defaults, then the foreign uid-0 role-22 transaction: phase foreign, `job.foreign:true`, `job.foreignOwn` true in nl (a live Software job file) and false in en; install → `update.err.locked`; idle again when it ends; one NO_AUTO_START `Get Role` |
| ↳ caller-vanishes-reattach | | owner A at the vanish gate, `shutdown()` < 100 ms, A's connection vanishes, owner B reattaches (GetTransactionList + GetAll, both NO_AUTO_START; `job.reattached`), installed rows from job.json's rowInfo |
| ↳ finished-in-gap | | the transaction ends in the gap; B: GetTransactionList empty → GetOldTransactions(5) → success → installed |
| ↳ vanish-cancels-download | | the fake cancels on vanish; B: GetOldTransactions says failed → `update.err.failed`, rows failed |
| `progress_under_the_flood_timeline` | flood, nl and en | the owner's page (its observer sees every `emit_local` and `emit`) ≤ 4 events in any second, a second Events in the same runtime dir (real datagram fan-out) ≤ 1 in any 900 ms; then installed |
| `two_services_one_owner_forwarding_and_takeover` | caller-vanishes-reattach | one owner + one mirror on one cache and events dir; only the owner connects; forwarded check → reply ok, exactly one RefreshCache; forwarded install; the mirror shows the owner's job; owner `shutdown()` < 100 ms → the mirror takes over and reattaches; rev never goes back in either host and continues past the old owner's; 2 forwarded writes; exactly one `Notify` (the new owner's) |
| `upgrade_running_refuses_every_write` | cancel-download, nl and en | with `python3 -s /tmp/ubuntu-release-upgrader-…/stonking` in the fixture procfs: check, install (2 forms), confirm accept, repair, release.check/compat/setPrompt/start → `update.guard.upgrade` (text), `CallCount()` unchanged; our download keeps showing (phase downloading, release.state running); a declined confirm, setAutoOpen, ubuntuHold, attach are not refused by this guard, details work, **cancel reaches the fake** (F17); afterwards releaseRunning, then available when the upgrader is gone |
| `dry_run_makes_no_call_on_the_fake` | available-mixed | a dry settings host, every method → its dry-run envelope; stats (0,0,0,0); no new unique name on the bus; CallCount 0; no stub call |
| `reader_mode_never_connects` | available-mixed | panel, start, desktop, taskmgr, explorer: status `mode:reader`, every write `update.err.notHere`; the power guard passes with no job and refuses with a committing job of a live owner, lock passes; stats (0,0,0,0); no new unique name; CallCount 0 |
| `shutdown_is_fast_and_the_next_owner_reattaches` | cancel-download | `shutdown()` < 100 ms during the download; B reattaches with the transaction's own AllowCancel (from GetAll) and cancels it (PackageKit compares uids); idle shutdown < 100 ms too |
| `a_vanished_daemon_is_resolved_through_old_transactions` | cancel-download, nl and en | `SetDaemon(false)` mid-download: GetOldTransactions activates a new daemon (activations 1), no record → `update.err.unknownOutcome`, rows stay pendingDownload, no re-check |

## 4. Verification (from W's root)

All on the final sources (logs in `.rt/gw2b/`). `BIN=target/debug/deps/win11_host-12500e7eb53cf9c1`.

| Check | Command | Result |
|---|---|---|
| Build | `tools/heavy.sh --tag gw2b-build -- tests/rust-sandbox/run-tests.sh --lib -- --list` | rc 0; lib test warnings 26 (= baseline), none in a file of mine; 387 tests listed |
| **Bus tests** | `tools/heavy.sh --min-mb 2048 --tag gw2b-final -- .rt/gw2b/final.sh`, i.e. `tests/update/run.sh bus-tests -- "$BIN"` | **8 passed, 0 failed** (~3 min): `every_timeline_in_nl_and_en` **62/62 scenarios ok** in 33.3 s, stub log 78 calls OK, last Load 32 calls OK; `progress_under_the_flood_timeline` nl and en **252 page events, at most 4 in any second; 63 fan-out events, at most 1 in any second** (126.7 s), stubs 2, calls 23; `two_services_…` stubs 1, calls 27; `upgrade_running_…` stubs 4, calls 29; `dry_run_…` 0/0; `reader_mode_…` 0/0; `shutdown_is_fast_…` stubs 2, calls 30; `a_vanished_daemon_…` stubs 2, calls 23. Every run: stub log OK, CallCount check OK, no violation, fake alive |
| shutdown < 100 ms | (inside those runs) | during a transaction **0.33–0.52 ms** (caller-vanishes 0.43/0.38, finished-in-gap 0.40/0.41, vanish-cancels 0.39/0.42, two services 0.52, shutdown test 0.33 ms), idle 0.26 ms |
| **strace** | default of every `exec` | all 8 runs `strace: OK`: nothing reached `/run/dbus/system_bus_socket` or `/run/user/1000/bus`; every successful `connect()` went to `/tmp/w11-update/dbus-*` (the run's private buses); failed NSS connects only (`/var/run/nscd/socket`, `/var/lib/sss/pipes/nss`, ENOENT); no addressed datagram, except in the two tests with `--allow-layer-dgrams`, whose datagrams all went to `/tmp/w11-update/tmp/*` (e.g. every_timeline: 20,683 calls in 431 processes, 299 with a unix address; flood: 96,923 / 46 / 176) |
| GLib criticals | `grep -c CRITICAL .rt/gw2b/final-bus.log` | **0** (317 before the `unboxed()` fix, §6) |
| Negative controls | `run.sh exec --expect-calls 1 -- "$BIN" …dry_run…` / two-services `exec` without `--allow-layer-dgrams` / `refresh-partial-fail.json` with another source host | exit 1 (`CallCount check MISMATCH`) / exit 1 (`strace: FAIL, a datagram sent with a unix address`) / exit 1, exactly **2 of 62** scenarios failed (the nl and en warning text), file restored (`sha256sum -c` OK) |
| **GW1 self-test** | `tools/heavy.sh --min-mb 1024 --tag gw2b-selftest -- tests/update/run.sh --self-test` | **exit 0, SELF-TEST PASSED**, 103 s: 32/32 timelines, 15/15 checks, strace OK (107,451 calls), guard 25/25, outside the layer 26/26 |
| **Unit suite** | `tools/heavy.sh --min-mb 2048 --tag gw2b-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0. lib **371 passed** (366 + 5 new), 0 failed, **16 ignored** (8 + the 8 bus tests); win11-launch 16 passed, 1 ignored; 11 other bins 0 tests; oracles ok (8 `oracle_tests`, 7 `disks::tests` python cases, `reference_contracts`); **`guard-calls.log` 0 bytes**; tmp entries left 0 |
| Drift + ratchet | `python3 -B tools/i18n/drift_rust.py --warnings` | **0 errors, 0 warnings** (541 rows, 2160 new literals; HEAD: 2112) |
| Drift, my paths | `python3 -B tools/i18n/drift_rust.py --paths host/src/services/update.rs host/src/services/update/*.rs host/src/services/system/info.rs host/src/services/system/power.rs --warnings` | 0 errors, 0 warnings |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings, the same 72 as HEAD f01710cc (`git archive` copy): 64 `[unused]` rows of GW4–GW9, 4 `[space]`, 4 `[glossary]` (round 2, GW2a, GW3); no TSV row changed |
| cargo check | `tools/heavy.sh --tag gw2b-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, win11-launch 2, lib test 26, launch test 1 warnings (= baseline); **none in a file of mine** |
| Mock rows | `python3 -B tests/i18n/test_update_mock_rows.py` | **OK**, 5 tests (1 skipped by design: update.tsv has rows, the catalogue comparison decides) |
| Ownership | `git status --short` | only owned paths: the 10 host files, `tests/update/{run.sh, fake_packagekit.py, README.md}`, 30 timelines, new `tests/update/stubs/bus-tests.json`, this note; nothing committed |

## 5. Deviations from the design or contracts

1. **The strace check allows datagrams inside the layer's tmpfs, only on request.** Two hosts of one test (the
   two-services and the flood tests) share an events directory, so `events.rs` sends addressed datagrams to
   sockets under `/tmp/w11-update/tmp/` (the layer's own tmpfs TMPDIR; no host socket can be there). GW1's check
   fails every addressed datagram; `exec --allow-layer-dgrams` exempts exactly that prefix and nothing else, and only
   those two tests pass it (`bus-tests.json` `datagrams`). Without it the same run fails (negative control 2). The
   connect rules and the /run checks are unchanged.
2. **The python3 stub answers in two formats.** GW2a's `phasing.rs` prints 7 columns plus `@@end`, GW1's
   `probe.py` and reference client read 5 (gw2.md §6). `probe.py` is frozen (tools/**), so every timeline's answer
   holds both: the 5-column lines unchanged and 7-column lines with the same phased/pup/held, each parser skipping
   the other's lines. 27 timelines had no answer at all; without one every non-security package would get
   phasing.rs's safe "phased" rule (F5), so they now answer "nothing phased" (installed versions derived from the
   candidate). GW1's self-test is unaffected (32/32).
3. **`run-tests.sh --lib --no-run` no longer works** (since D-G37 `run-tests.sh` passes `--no-run` itself and
   cargo refuses it twice). The build step is `run-tests.sh --lib -- --list` (builds, lists, runs nothing). README
   says so.
4. **`flood` runs in its own test** in both languages (60 s each) instead of inside `every_timeline_in_nl_and_en`.
5. **`shutdown()` waits up to 80 ms** for the pk thread to drop its engine (design §4.1 says "returns"): no join,
   but the job lock is free and a private test-bus connection closed before `update.owner {gone}` goes out, so the
   mirror's reattach cannot overtake the old owner's vanish. Measured times are in §4. On the real system bus the
   connection is the process-wide shared one and is not closed (the process exits right after).
6. **Two scenarios need the test as a second client**, as their timelines' reference flows do: `cannot-cancel`
   waits for a `Cancel` call that our service rightly never sends (AllowCancel false), so the test sends it and
   PackageKit refuses it; `updates-changed` needs apt's hook (`StateHasChanged`). Both calls are in the expected
   call lists.
7. **`vanish-cancels-download` ends as `update.err.failed`**, not "cancelled": GetOldTransactions reports only
   `succeeded:false`. That is GW2a's mapping (and what the design expects R3 to measure); noted, not changed.

## 6. Bugs found and fixed on the way (GW2a code)

* **GLib-CRITICAL per transaction property.** `Tx::properties` and the reattach `GetAll` reader called
  `Variant::as_variant()` on values that `HashMap<String, Variant>` had already unboxed; glib-rs calls
  `g_variant_get_variant` unchecked, so each property logged `g_variant_get_variant: assertion … failed` (317 in
  the first four bus tests; the values were still right). `pk::unboxed()` checks the type first; 0 criticals since.
* **A successful job kept the previous failure's error** (`repair-required`: after a good RepairSystem the hero
  stayed "error"). `finish_job` now clears the stored error on success (§4.5 "until the next check or success").
* **The fixture/tool dirs of a run were group-writable** under Robin's umask 002, so the seam check refused them;
  run.sh and the layer driver now make them 0700.
* `totals.downloadSize` kept the last job's total after the job; it is reset when the job ends.

## 7. Decisions for the coordinator

1. Accept deviation 1 (`--allow-layer-dgrams`, layer tmpfs only, two tests) or ask for the two-host tests to be
   rewritten without the real fan-out (the design asks for "the real Events").
2. Accept the dual-format stub answers (deviation 2), or let a later step make `probe.py` read phasing.rs's format.
3. Accept the bounded 80 ms wait in `shutdown()` (deviation 5).
4. The design text (§9.1 31 timelines, §9.2 `run.sh cargo`) should say: 32 timelines, `run.sh bus-tests -- BIN`.
5. `gw1.md`/README's old build recipe (`--lib --no-run`) is corrected in the README only (gw1.md is GW1's note).

## 8. Fix round (after `gw2b-review.md`, 2026-11-05)

At the start `git status --short` / `git diff --stat` matched the review's view (the 10 host files, run.sh,
fake_packagekit.py, README.md, 30 timelines, `stubs/bus-tests.json`, the two notes; 43 files). Logs and scripts of
this round are in `.rt/gw2b/fix/` (`final.sh`, `bus.sh`, `build-bus.sh`, `neg-m1.sh`, `neg-m3.sh`, `gw2c.sh`).
Nothing was committed; nothing outside W was touched; every heavy command went through `tools/heavy.sh`
(tags `gw2b-fix-*`).

### 8.1 Findings

| Id | Verdict | What changed |
|---|---|---|
| **M1** | fixed | `pk.rs`: `Engine::hero_key()` = what the hero shows of our job, as `compose` derives it: `model::stage(status, committing, simulate‖confirm)`, `committing`, auth wait (status 31), and whether unattended-upgrades holds the lock (`Tx.holder`, a `/proc` scan only while status 30, at most every 2 s). `persist()` records it; `job_progress` calls `persist()` when it differs (or when the download filled the total, m1), else only rewrites `job.json` as before. So `update.changed` now goes out at waiting → downloading → installing, lock → auth and lockHolder changes; 1 → 2 (wait/setup) is not a change (the page shows the same text). Under the flood the test asserts at most 6 `update.changed` during the job (by construction 3: install, end of the simulate, commit), not one per PropertiesChanged. Tests: unit `a_new_stage_of_the_running_job_is_published` (the exact sequence of phase/status/lockHolder/totals.downloadSize on `update.changed`); bus: `Seen` keeps the whole event; download-install and flood: phases on events = `[downloading, installing]` before Finished (an `installing` event with `committing:true`); lock-wait: `[downloading, waiting, downloading, installing]`, exactly one `waiting` event with status 30 and `lockHolder` `'unattended'` (nl) / null (en); auth-wait: the same with status 31; the mirror in the two-services test follows the owner's job through the fan-out |
| m1 | fixed | `compose`: a total of 0 (snapshot or job) is unknown, never a total; `on_simulated`: `DownloadSizeRemaining` 0 → None; the fill in `job_progress` persists once. Fake: `emitSimulate: {downloadSize: N}` (validated, ≥ 0); `phased-as-normal`'s simulate reports 0. Bus: on `update.changed` during that job totals.downloadSize is `null` first, then 9,732,096 (the 3 sent packages, = `job.downloadSize`), never 0. Unit: the compose cases (0 in the snapshot → the job's; 0 in both → null) and `on_simulated` with 0 |
| m2 | fixed (coordinator to confirm, see 8.4) | `UpdatesChanged` → rescan + persist as before, then a debounced (2 s after the last signal) re-read: GetUpdates without RefreshCache, the classifier, GetDetails for sizes, exactly the re-check after a job (`checking` true meanwhile, so an install answers `update.err.busy`, F16). Not when a GetUpdates started after the signal; again later while a check or our job runs; never before the first check (the first list stays the user's, §4.10). Timeline `updates-changed`: its StateHasChanged script first removes libsoup-3.0-common (as if apt installed it), its reference client now expects 1 package. Bus: check (2 updates) → StateHasChanged → the page's list has 1 update, `checkedAt` later, pk calls `check + StateHasChanged + GetUpdates + GetDetails`, 2 stub calls. Unit `the_reread_after_updates_changed_waits_and_is_not_repeated` |
| m3 | fixed | `run.sh bus-tests`: first `BIN services::update::bus_tests:: --list --ignored` inside the layer (its own `exec`, strace on); the listed tests must equal the JSON's keys (with `--only`: include each name); each test's output must contain `test result: ok. 1 passed; 0 failed`, else rc 1 |
| m4 | fixed | `Stats.fanout` (cfg(test)) records the throttle's own `now` of every `Delivery::Both`; the flood test asserts ≤ 1 send in any 1 s window on that clock, `arrived == sends`, ≤ 1 arrival in any 900 ms, and n arrivals span ≥ n−1 s (−0.1 s) |
| m5 | fixed | `update.rs` `reboot_flag()`: `/run/reboot-required` or a `RequireRestart` record of this boot (types 3–6, `restart::require_restart_applies`), used by `compose` (`restart.reboot`, hence the owner's `system.updates.changed`) and by `system_updates_from` (`system.updates`). The apt path (no snapshot) keeps the file only, as §4.10 says. Unit `reboot_required_is_one_rule_for_the_snapshot_and_system_updates` (6 records + the file: both rules agree) |
| m6 | fixed | `Stats.notified` (cfg(test)) records the session-bus unique name of each `Notify`; two-services and the new crash test assert the fake's `sender` is the new owner's recorded name, the old owner sent none, and bus opens old 1 / new 2. (GIO's bus singleton is a weak reference, so the process does not keep one session connection: the first attempt to compare with `bus_get_sync(Session)` from the test got `:1.2` against the Notify's `:1.1`.) New bus test `a_mirror_takes_over_by_its_retry_after_the_owner_crashed`: `MIRROR_RETRY_TEST` = 1 s, `Update::crash()` (cfg(test): pk thread stopped, owner lock dropped, no `update.owner {gone}`), the test thread iterates the default main context (the GTK main thread's role); the retry loses for 2.5 s against the live owner, wins 52–604 ms after the crash (three runs), reattaches (GetTransactionList + GetAll), finishes, notifies once |
| m7 | fixed | `power.rs` test: skips (with a message) when any `WIN11_UPDATE_*` seam is set; calls `action()` only when guard-bin is first on PATH, `systemctl`, `gnome-session-quit` and `loginctl` resolve into it, `DBUS_SYSTEM_BUS_ADDRESS` is `unix:path=<sandbox>/run/no-system-bus` and that path does not exist, and no Hyprland is set. In GE0b's sandbox all gates pass (action() runs, guard log 0 bytes); inside the bus layer the test skips instead of panicking |
| m8 | not mine | `api-methods.json` `system.updates` is not an `update.*` doc string: coordinator (8.4) |
| n1, n5 | accepted | unchanged (`--allow-layer-dgrams`, the bounded 80 ms wait) |
| n2 | not done | optional; every scenario's calls, stubs and violations are already asserted inside the Rust test, the run-level check stays a second net for the last scenario |
| n3, n6, n7, n9 | coordinator / R3 | unchanged (8.4) |
| n4 | fixed | `paths::maybe_present()`: any error but NotFound counts as present; `check_bus` and `fake_bus()` use it (also for `/var/run/dbus/…` and `/run/user/<uid>/bus`). Unit: a socket path under a mode-0000 directory is refused ("the real system bus socket is visible"), a missing directory is absent |
| n8 | none | — |

### 8.2 Files changed in this round

`host/src/services/update.rs` (Stats.fanout/notified, `MIRROR_RETRY_TEST`, `crash()`, `reboot_flag()`, compose's
0 rule), `update/pk.rs` (M1, m1, m2, m4 clock, two unit tests), `update/paths.rs` (n4), `update/tests.rs` (m1, m5,
n4 cases), `update/bus_tests.rs` (M1/m1/m2/m4/m6 assertions, the crash test), `system/power.rs` (m7, test only),
`tests/update/run.sh` (m3), `tests/update/fake_packagekit.py` (`emitSimulate.downloadSize`),
`tests/update/README.md`, `tests/update/timelines/{phased-as-normal,updates-changed}.json`,
`tests/update/stubs/bus-tests.json` (the crash test; every_timeline stubs 78 → 82), this note.
`info.rs`, `guard.rs`, `net.rs`, `restart.rs`, `update.tsv`, `api-methods.json`, `vendor/ui` unchanged in this round.

### 8.3 Verification (W's root, final sources)

| Check | Result |
|---|---|
| Build `tools/heavy.sh --min-mb 2048 --tag gw2b-fix-final -- .rt/gw2b/fix/final.sh` (= `run-tests.sh --lib -- --list`, then `run.sh bus-tests -- "$BIN"`, then `run-tests.sh --no-fail-fast`) | build rc 0, **391** tests listed, lib test warnings 26 (= baseline), none in my files; BIN = `target/debug/deps/win11_host-12500e7eb53cf9c1` |
| Bus test list (m3) | 9 bus tests in the binary, the same as in `bus-tests.json` |
| **Bus tests** | **9 passed, 0 failed**, each `test result: ok. 1 passed`: `every_timeline_in_nl_and_en` **62/62** in 38.2 s (updates-changed now 2.5 s each: the debounce), stub log 82 OK, calls 32 OK; flood nl/en **252/251 page events, ≤ 4 in any second; 63 fan-out sends, ≤ 1 in any second on the throttle's clock (closest pair 1.00 s); 63 arrived in 62.3 s** (126.7 s); two-services stubs 1, calls 27; crash takeover stubs 1, calls 27, takeover 52 ms after the crash; upgrade-running 4/29; dry run 0/0; reader 0/0; shutdown/reattach 2/30; vanished daemon 2/23. GLib-CRITICAL 0 |
| shutdown < 100 ms | during a transaction 0.34–4.42 ms (one 4.4 ms: finished-in-gap nl; the rest ≤ 0.45 ms), idle 0.33 ms |
| strace | all 9 runs and the listing run OK: nothing reached `/run/dbus/system_bus_socket` or `/run/user/1000/bus`; every successful connect() to `/tmp/w11-update/dbus-*`; failed NSS connects only (ENOENT); datagrams only in flood and two-services, all to `/tmp/w11-update/tmp/*` (e.g. every_timeline 21,104 calls in 443 processes, 307 with a unix address; flood 96,925 / 46 / 178) |
| Negative controls | (1) `bus-tests.json` with an extra `reader_mode_never_connectz` → exit 1 before any test, "only in the JSON: reader_mode_never_connectz"; `--only reader_mode_never_connectz` → exit 1 "has no bus test"; file restored, `sha256sum -c` OK. (2) `pk.rs` without the structural persist in `job_progress` (`if false && (…)`) → **exactly 8 of 62** scenarios fail (phased-as-normal, download-install, auth-wait, lock-wait × nl/en; e.g. phases `["downloading"]`, totals `[null, null]`); pk.rs restored (`sha256sum -c` OK), rebuilt, `every_timeline` 62/62 again |
| GW2c in both sandboxes (`.rt/gw2b/fix/gw2c.sh`) | GE0b: both tests ok, the power test calls `action()` (every gate passed), guard-calls.log 0 bytes; inside the bus layer: the info test ok, the power test "skipped: WIN11_UPDATE_PK_BUS is set", stub log 0, calls 0, strace OK |
| **GW1 self-test** `tools/heavy.sh --min-mb 1024 --tag gw2b-fix-selftest -- tests/update/run.sh --self-test` | **exit 0, SELF-TEST PASSED**, 102 s: 32/32 timelines (phased-as-normal 15 calls, updates-changed 4), 15/15 checks, strace OK (107,447 calls), guard 25/25, outside the layer 26/26 |
| **Unit suite** (in `final.sh`) | rc 0. lib **374 passed** (366 + 5 GW2b + 3 fix round), 0 failed, **17 ignored** (8 + 9 bus tests); win11-launch 16 passed, 1 ignored; oracles ok; **guard-calls.log 0 bytes**; tmp entries left 0 |
| Drift | `python3 -B tools/i18n/drift_rust.py --warnings` → 0 errors, 0 warnings (2162 new literals); `--paths` my files → 0/0 |
| Catalogue | `PATH=/home/robin/.local/node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` → **0 errors**, 72 warnings (the same 72 as HEAD) |
| cargo check | `tools/heavy.sh --tag gw2b-fix-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` → rc 0; lib 28, win11-launch 2, launch test 1, lib test 26 warnings (= baseline), none in my files |
| Mock rows | `python3 -B tests/i18n/test_update_mock_rows.py` → OK (5 tests, 1 skipped by design) |
| Ownership | `git status --short`: only owned paths (as at the start; no new path besides those listed in 8.2) |

### 8.4 Deviations and decisions for the coordinator

New deviations (beyond §5):

8. **The re-read after `UpdatesChanged` (m2)** is behaviour the design does not state (§4.8 only names the
   rescan). It is local (no RefreshCache, no network), debounced, shows `checking` for its ~1–2 s, and never makes
   the first list. Please confirm, and add it to design §4.6/§4.8.
9. **Two timelines changed beyond additions:** `updates-changed` removes one update in its StateHasChanged script
   and its reference client expects 1 package (was 2); `phased-as-normal`'s simulate reports
   `DownloadSizeRemaining` 0 (new fake option). GW1's self-test stays 32/32.
10. **Test-only hooks in update.rs** (all `#[cfg(test)]`): `MIRROR_RETRY_TEST`, `Update::crash()`,
    `Stats.fanout`, `Stats.notified`; in pk.rs the `fanout` push. Release code is unchanged by them.
11. `run.sh bus-tests` starts one more layer for the listing (~1 s).
12. The GW2c power test skips in a seam environment instead of failing.

Decisions:

1. Confirm deviation 8 (m2), or have it removed (then `updates-changed` goes back to a rev check).
2. m8: the `system.updates` doc string in `host/src/api-methods.json` should read
   `{available:int|null, checkedAt, tool:'packagekit'|'apt'|null, checking, security, rebootRequired} (update/snapshot.json once a check finished; else apt list, cached 30 min)`.
3. n3: one shared phasing script for `phasing.rs` and `probe.py` (then drop the 5-column stub lines, deviation 2).
4. n6: whether `update.install` during a check (or the re-read of deviation 8) wants its own text instead of
   `update.err.busy` ("An update is already being installed.").
5. n7, n9: for R3 (vanish → failed; our GetOldTransactions re-activates packagekitd).
6. Still open from §7: `--allow-layer-dgrams` (n1: accept), the dual-format stub, the 80 ms wait (n5: accept), the
   design text (32 timelines, `run.sh bus-tests -- BIN`), gw1.md's old build recipe.
