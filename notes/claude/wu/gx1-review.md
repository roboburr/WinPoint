# GX1 review: restart all Win11 processes (`shell.restartAll`), adversarial, 2026-11-05

Reviewer: an independent agent. I did not write GX1. Worktree `.wt/gx1` (branch `wu/gx1`, HEAD f01710cc, nothing
committed). I changed no product file. I wrote only this note and scratch files under `.rt/review-gx1/`: logs, an
extra harness scenario `rv_gx1.py`, a node test of the real mock in `mockrun/`, and the moved harness outputs. The
harness only writes `--out` inside `tests/gnome/`, so my three runs went to `tests/gnome/rv-gx1-*.out`. I moved them
to `.rt/review-gx1/` afterwards and left nothing in `tests/gnome/`. Every heavy command went through
`tools/heavy.sh` with a `gx1-rv-*` tag. I ran no win11-* binary, gnome-shell, D-Bus tool, apt or PackageKit outside
the sandboxes. The only processes I signalled were my own: a hung unit-test binary of my own run (SIGTERM, see §1)
and two dummy processes my scenario had started.

**Verdict: fix-needed (one major, four minors, twelve notes, no blocker).** GX1 is close to the design, and every done-when check
I re-ran passes: harness nl and en both 54/54, the unit suite on its second run 379/16 + 16/1, the mock, the i18n
tools and the manifest. My adversarial harness confirms several things the step only tested at unit level or not
at all:
* refusal while a release upgrader runs;
* deferral on an update job, then resume through the pidfd when its owner dies without an event;
* two Explorer processes each restoring their own tabs;
* the private bus environment surviving the exec.

I found no path to the real system bus, the network, the live session or real dconf.

* **M1 (major):** a second `shell.restartAll` while the first is under way (a double click on the InfoBar button)
  restarts the desktop, Taakbeheer and Explorer a second time. This is confirmed in the harness.
* **m2–m5 (minor):**
  * m2: a windowless app host drops the restart and never restarts later.
  * m3: the portal can quit while a dialog is still opening.
  * m4: the stub-log allowlist is wider than the observed calls.
  * m5: a copying Explorer disappears until its copy ends.
* **n1:** the sandboxed unit suite can deadlock in pre-existing tests. That happened in my first run and in GW4's
  run. It is not GX1, but it blocks every step's done-when.

## 1. Checks I ran (from W's root)

| Command | Result |
|---|---|
| `git status --short`, `git diff --stat`, `git diff f01710cc -- <frozen paths>` | only owned files (§6); frozen diff 0 lines |
| `tools/heavy.sh --tag gx1-rv-build -- cargo build --offline --locked -j2`, then `sha256sum target/debug/win11-*` against `tests/build/gx1-debug/SHA256SUMS` | rc 0, "Finished in 0.15s" (nothing to rebuild); **12/12 hashes equal**, so the snapshot is the current source |
| `tools/heavy.sh --min-mb 2048 --tag gx1-rv-test -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | **hung** after 95 tests in pre-existing `apps::catalog`/`app_params`/`apps::recent`/`files::*` tests (n1); I stopped my own test binary, rc 101; `guard-calls.log` 0 lines |
| `tools/heavy.sh --min-mb 2048 --tag gx1-rv-unit2 -- tests/rust-sandbox/run-tests.sh --no-fail-fast` | rc 0: lib **379 passed / 16 ignored**, launch **16 passed / 1 ignored**, 0 failed, 3.6 s; all 13 GX1 tests ok; `guard-calls.log` **0 lines** |
| `tools/heavy.sh --tag gx1-rv-check -- cargo check --offline --locked -j2 --all-targets --message-format=short` | rc 0; lib 28, lib test 26, win11-launch 2, launch test 1 warnings (= the step's baseline). The one hit in a GX1 file, `shell.rs:80` `set_runtime_info is never used`, is f01710cc's `shell.rs:76`, shifted by the 4 inserted lines. 0 in `lib.rs`/`restart.rs` |
| `rustfmt --edition 2024 --check host/src/services/shell/restart.rs` | clean |
| `python3 -B tools/i18n/drift_rust.py --warnings` | 0 errors, 0 warnings (13450 frozen literals, 2202 new) |
| `PATH=…node… python3 -B tools/i18n/check_rust.py --warnings` | 0 errors, 72 warnings, none in a GX1 file |
| `python3 -B tools/i18n/ratchet.py`; `tools/i18n/check.py` | `ui: ok (89 allowed occurrences, 0 still to convert)`; 0 errors |
| `node --check --input-type=module < vendor/ui/js/mock/shell.js` | ok |
| `node .rt/review-gx1/mockrun/test.mjs` (the **real** mock modules copied into a `"type":"module"` tree, not a fake `m`) | **13/13**: H20 accepted, event sent, `restart.shell` cleared after 3.6 s; H04/H05/H07 nl and H06 en refused with the exact §3.7 text; H25 `update.guard.upgrade` text; H02/H09 not refused; bad reason en "Invalid input."; H26 not refused (n7) |
| `python3 -B tools/ui-manifest.py --check` | up to date |
| `tools/heavy.sh --min-mb 2560 --tag gx1-rv-harness -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/gx1-debug --source worktree --scenario tests/gnome/scenarios/gx1-restart.py --out tests/gnome/rv-gx1-nl.out` | rc 0, **54/54 checks**, 142.2 s, peak PSS 1219 MiB, leftover processes **[]**, stub calls outside the read-only list **[]** (wpctl 61, pw-dump 38, systemctl --user 60, apt list 3) |
| same with `--lang en --out tests/gnome/rv-gx1-en.out` (tag `gx1-rv-harness-en`) | rc 0, **54/54 checks**, 137.2 s, peak PSS 1216 MiB, leftover **[]**, stub calls outside the list **[]** (wpctl 60, pw-dump 38, systemctl 58, apt 3); exec desktop 31, Taakbeheer 326, Explorer 533, panel 1251, Settings 2048 ms; old Start gone 1200, new 1485 ms; bar 1.58–1.69 s; deferral exec 42 ms after the job's end |
| `tools/heavy.sh --min-mb 2560 --tag gx1-rv-adv -- python3 -B tests/gnome/harness.py launch run --lang nl --build-dir tests/build/gx1-debug --source worktree --scenario .rt/review-gx1/rv_gx1.py --out tests/gnome/rv-gx1-adv.out` (my adversarial cases R1–R5) | rc 0, 13 checks, **2 failed** (R2 = M1, and R5's handoff variable = n12). R4 a fake `python3 -I /tmp/rv-gx1/do-release-upgrade` (a sleeping script in the sandbox's tmpfs) → `update_busy` + `update.guard.upgrade` with the exact §3.7 nl text, no event. R1 two Explorers (2 and 3 tabs) each restored their own tabs. R5 bus and runtime environment unchanged in all 6 hosts, and no process holds the Explorer lock. R3 update-job deferral and pidfd resume ok (§4). End: 2 Explorers, one of each other host, no zombie, leftover [], no extra stub call (wpctl 93, pw-dump 41, systemctl 92, apt 4). 163.9 s, peak PSS 1263 MiB |

My nl numbers (ms after the event): exec desktop 7, Taakbeheer 332, Explorer 545, panel 1238, Settings 2056;
WebView ready 824 / 1214 / 1339 / 2102 / 2797, Start 2287. Old Start gone at 1214 ms, one new Start (a child of
the panel) at 1501 ms, never two. Fallback bar seen at 1.61–1.72 s, gone (native panel) by 3.15 s. Deferral: the
job was written 0.4 ms after the event, and Taakbeheer exec'd 47 ms after the job's end. These match the step's
table within run-to-run noise.

## 2. Findings

### M1 (major, confirmed in the harness): a second click restarts the desktop and the apps a second time

* **Where:** `restart.rs:165-189`. The origin's `restart_all` emits a new event with a new nonce on every call,
  even while its own restart is already scheduled. Receivers that have already exec'd are new images with
  `scheduled = false` (`restart.rs:237-243`), so they act on the second event. The page's InfoBar button stays
  enabled after the first answer: `update.js:589` `write()` → `holding()` → `load()` re-renders it, and
  `restart.shell` is still true for the 2 s until Settings itself goes.
* **Evidence (my R2, `.rt/review-gx1/rv-gx1-adv.out/results.json`):** a second `shell.restartAll` 900 ms after the
  first, which is what a double click or a second click looks like, since Settings shows no change for 2 s. Both
  calls answered `{ok:true}` with different nonces, and 2 events went out (at 42 and 942 ms). WebView boots:
  desktop **786 and 1788 ms**, Taakbeheer **1187 and 2218 ms**, Explorer **1405, 2313 and 2480 ms** (2 processes,
  one of them restarted twice). Panel, Start and Settings booted once each, because they were still waiting in
  their old image. So the user sees the desktop, Taakbeheer and Explorer windows close and reopen twice.
* **Fix (restart.rs, small):** keep the scheduled nonce in `State`. In `restart_all`, if this process already has a
  restart scheduled or deferred, answer `{ok:true, nonce:<that nonce>, pending:true}` and emit nothing. Optionally
  the page disables the button once the answer is ok (that is GW3's file, so it goes to the coordinator). Add a unit
  test: two `shell.restartAll` calls give one event.

### m2 (minor): an app host without a window drops the restart and never restarts later

* **Where:** `lib.rs:613-616`, `restart_on_main`:
  `if windows.is_empty() && !matches!(kind, "panel"|"desktop"|"start") { take_restart_all(); return; }`.
  `restart::State.scheduled` stays `true` (`restart.rs:239-242`), and nothing resets it.
* **Scenario:** an Explorer whose last window was closed while a long copy still runs (`Files::shutdown` keeps the
  process alive), or Settings in its keep-alive after a finished job (GW2 deviation 22, up to 60 s). It gets a
  `shell.restartAll` and stays on the old build. Every later `shell.restartAll` is also ignored, because
  `scheduled` is set. Its stale mappings keep the InfoBar up, so a click restarts everything else again and never
  this host. It does end once its job is done, so this is a narrow case. But the restart is lost without any trace:
  no `shell.restartDeferred` and no log line.
* **Fix:** reset `scheduled` (and log one line) when `restart_on_main` drops the plan. Or better, treat "no window
  yet, process kept alive" as one more deferral: keep the pending restart and re-check when the hold ends, which is
  the process exit anyway. At least `eprintln!` so it can be found.

### m3 (minor): win11-file-portal can quit with a request that has no window yet

* **Where:** `lib.rs:636-648`. `quit_when_idle` treats "no GTK window" as idle. In `portal.rs:185-197` a new request
  is `Phase::Preparing` with `parent: None` until its picker exists, so for that moment `app.windows()` is empty
  although `portal.pending` is not.
* **Scenario:** an app opens a file dialog during the event. The portal quits between the D-Bus request and its
  window, and the dialog fails to appear (the caller gets the shutdown's answer). D4's own promise, "an open dialog
  of another app is never cancelled", does not cover a dialog that is opening.
* **Fix:** decide on `portal.pending.is_empty()` instead of on windows. That needs a flag the portal's 16 ms loop
  honours (`portal.rs`, a one-liner next to `quitting`), or a pending count the portal publishes. If `portal.rs`
  stays frozen, write the reading down in gx1.md D4.

### m4 (minor): the harness "guard log empty" allowlist is wider than what the runs call

* **Where:** `gx1-restart.py:54-57` `READ_ONLY_STUBS` accepts `gsettings get`, `pactl subscribe/list/get-`,
  `nmcli -t`, `bluetoothctl show`, `rfkill list` and `udisksctl status`. None of these occurred in any run (step's
  nl/en and mine: only `wpctl get-volume`, `pw-dump --monitor`, `systemctl --user …` and `apt list --upgradable`).
  The matching is also prefix-based (`args.startswith`), so `systemctl --user is-…` covers whatever `is-` starts.
* **Impact:** a GX1 regression that, for example, ran `gsettings get` or `pactl` during a restart would pass the
  self-test. Design §4.11 wants "exactly the expected calls".
* **Fix:** list only the four observed calls with their exact argv, or diff the stub log against a run of the same
  scenario with the restart click removed. Move the list to `tests/gnome/lib/` for GW8 (the step's C4).

### m5 (minor): an Explorer that is copying loses its windows until the copy ends

* **Where:** `restart_on_main` closes Explorer's windows first (`lib.rs:621-630`). Then `runtime.shutdown()` →
  `Files::shutdown` waits for every in-flight `files.*` call (`files/mod.rs:373-383`) before the exec.
* **Scenario:** a several-minute copy in Explorer while Robin clicks "Win11 Shell opnieuw starten". Explorer's
  windows and the copy's progress disappear. The copy finishes invisibly, and only then does Explorer come back.
  §5's deferral rule ("so `Software::shutdown`'s join never waits") applies just as well to `Files::shutdown`'s
  wait. The step lists this under "Not covered" as existing behaviour, but it is `shell.restartAll` that makes it
  reachable from Settings.
* **Fix:** add "a `files.*` job in flight" to the receiver's re-check (a `Files::busy()` counter read, resumed on
  the job's end). Or at least say in the InfoBar body or in R3/R9 that a running copy delays Explorer.

### n1 (note, not GX1): the sandboxed unit suite hangs in pre-existing tests

* My first full run (`gx1-rv-test`, 22:21) stopped making progress after 95 tests. The same 16 tests kept
  "running for over 60 seconds": `app_params::…`, 8 × `apps::catalog::…`, 2 × `apps::recent::…`,
  `files::actions/places/properties::…`. 7 threads of `apps::catalog::tests::concurrent_catalog_builds_merge_seen_entries_without_lost_updates`
  waited on `flock(.apps-seen.lock)` (`/proc/locks`: holder and 7 waiters all in the test process), and the rest
  waited on futexes, with load 0.9 and CPU idle.
* GW4's run of its own tree a few minutes earlier hung on **the same 16 tests** and ended at the 900 s guard
  (`.wt/gw4/tests/rust-sandbox/logs/test.time`: `exit=124`, 15:00). GW4 has no GX1 code, and none of these tests
  touch GX1 code. The step's own run at 21:51 passed.
* I sent SIGTERM to my own hung test binary (a descendant of my chain) to give the lock back, and ran the suite
  again (`gx1-rv-unit2`): lib **379 passed / 16 ignored**, launch 16 passed / 1 ignored, 0 failed, wall time 3.6 s
  (the lib binary itself took 2.42 s), `guard-calls.log` 0 lines. So the suite normally finishes in seconds, and the
  hang is an occasional deadlock.
* This is for the coordinator: it looks like a lock-order inversion between the `.apps-seen.lock` flock and a
  process-wide mutex, which only bites sometimes. It blocks every step's done-when.

### n2 (note): "no poll" is shown by reading the code, not by a test

The unit test and the harness E step both end the job by writing the file and then sending the event. A poll
would pass them as well. I read the deferral path: it is event- and pidfd-driven only (`restart.rs:192-213`,
`312-369`). My R3 shows the pidfd resume (no event, no file change). A test that only rewrites the file, waits
for example 1 s and expects no restart would pin it.

### n3 (note): when the old Start needs more than 2 s, the new panel ends up with no Start

`wait_prewarm_exit(2 s)` (`lib.rs:554`). If the SIGTERM'd Start is still registered when the new panel prewarms
(`try_prewarm_start`), the new `win11-start --preload` finds `io.github.win11shell.Start` remote and exits
(`lib.rs:368-381`). The panel's child watch then drops it, and nothing prewarms again until the bridge's name
reappears. The harness never got near this (the old Start was gone about 0 ms after the panel quit).

### n4 (note): small robustness edges

* `State.exited` is never cleared (`restart.rs:361`). Within one image, a later update job whose owner has reused
  the same pid would be ignored by `wait_for`. This is very unlikely.
* `take_handoff` returns `None` and leaves the inherited fd open when its inode is not the lock file's
  (`lib.rs:768-790`), for example when the lock file was recreated in between. `reexec` cleared its close-on-exec,
  so this fd then leaks into every child (WebKit, launched apps). It is only a stray fd, since the lock now lives on
  a new inode. Close the fd on every `None` path after the parse.
* `explorer_handoff` polls `flock(LOCK_NB)` every 50 ms for up to 5 s on the main thread (`lib.rs:741-766`). That
  is acceptable, but it is the one poll in the restart path.

### n5 (note): after a deferral the InfoBar stays (the step's C2)

The step's harness E confirms the InfoBar comes back. That it stays afterwards follows from GW2's rescan triggers. It needs a decision from GW2b/GW4.

### n6 (note): unit-test gaps

There is no unit test for: the windowless path of `restart_on_main`, `take_handoff`'s validation (another fd, the
wrong inode, an fd ≤ 2), the `/proc/self/exe` fallback of `reexec`, or `reap_inherited_children`. The harness covers
the last two indirectly (no zombies; renamed binaries).

### n7 (note): the mock allows the restart in H26 (`foreignOwn`), where the host refuses

In the host a `foreignOwn` transaction is our own Software job, so `restart_all_blocked` answers
`update.guard.software`. The mock skips `job.foreign` and finds no active job in `software.jobs` (my mock test:
H26 → `{ok:true}`). The InfoBar is not shown in H26, so this is unreachable today. Model it with
`j.foreignOwn → software_busy`.

### n8 (note): activation tokens raise every restarted Win11 app window

Each restarted app window maps focused. Win11 windows that were behind a user's other app come to the front, and
Settings, which maps last, ends up on top. This is better than GNOME's "is ready" banners (D5), but it changes the
stacking. Robin should know (R3/R9).

### n9 (note): MOCK-CHANGES.md

`tests/i18n/MOCK-CHANGES.md` needs a row for `mock/shell.js` `shell.restartAll` (round 3, D-G41 D4).

### n10 (note): what the other hosts reopen on

A Settings that is not the origin, Alles and the installer re-exec with their *launch* argv
(`restart_args` `_ => args.to_vec()`). So they reopen on the page, query or `.deb` they were started with, not on
what they showed. For Settings this is a visible change (the window jumps back to its first page). The fix is
small: Settings could pass its current page the way the origin does.

### n11 (note): a Software job in the prewarmed Start, started after the guard, ends with the panel

The prewarmed Start ignores the event and so never re-checks. Start can run uninstalls (`shell/startmenu.js` uses
`core/software.js`). The panel re-checks only its *own* jobs (`wait_for(.., me, ..)`), and its shutdown then
SIGTERMs the Start, which has no SIGTERM handler. Its worker dies through `PR_SET_PDEATHSIG` (`software/worker.rs:75`).
The window is short: from the click until the panel's 1200 ms. A job running at click time is refused by the
guard. Fix: the panel's re-check also counts jobs whose `_owner.pid` is its prewarmed child.

### n12 (note): `WIN11_RESTART_HANDOFF_FD` stays visible in an Explorer's `/proc/<pid>/environ`

`take_handoff` removes it from the live environment (`lib.rs:772`), so children do not get it. But the exec's
original block, which `/proc/<pid>/environ` shows, still has it (R5: Explorer `76`, Explorer 2 `42`). This is
harmless: a tool that re-launches from `/proc/<pid>/environ` would pass a number that `take_handoff` rejects
unless it is the lock's inode. It is worth one comment line next to `HANDOFF`.

## 3. Safety (check 1)

I found no path from GX1's code or tests to the real system bus, the live session bus, the network, real dconf or
gsettings, or a live-session process.

* **Product code.** The new code reaches the outside world in four ways:
  * `guard::restart_all_blocked`, which reads files and `/proc` only;
  * `pidfd_open` plus `poll` on the update owner (`restart.rs:329-369`), with no `pidfd_send_signal`;
  * `waitpid` on this process's own inherited children (`lib.rs:793-833`);
  * one best-effort `apps.activationToken` call on the session bus for app kinds (`restart.rs:375-378` →
    `gnome::call`, 3 s timeout).

  The exec runs `exec_path(current_exe())`, or `/proc/self/exe` when that path is gone. It never runs a PATH or
  absolute tool. No other process is signalled. The only signal is the panel's existing SIGTERM to its own
  prewarmed Start (`lib.rs:937`), which design §5 expects.
* **Unit tests.** The fixtures build events directories under `std::env::temp_dir()` (the sandbox's private
  TMPDIR), so an emitted `shell.restartAll` reaches only those sockets. The `request_shutdown` closures are test
  closures. The two tests that reach `restart_now` use kind `desktop`, which is not in `APPS`, so no activation-token
  D-Bus call is made (`restart.rs:375`). One test starts `sleep` from PATH and kills that child of its own
  (`restart.rs:845-867`). `tests/rust-sandbox/logs/guard-calls.log` had 0 lines after both of my runs.
* **Harness scenario.** It runs inside `harness.py`'s bwrap layer (`harness.py:475-482`: `--tmpfs /tmp /run/user
  /run/dbus`, `--unshare-pid --unshare-net`). Its `/proc` scans and its `/tmp/win11-events-*` glob therefore see
  only the sandbox. It sets `WIN11_UPDATE_FIXTURE_ROOT` (with the marker file) for its Settings only, and no
  `WIN11_UPDATE_PK_BUS`. With that seam `paths.rs:395-398` gives `BusTarget::Disabled`, so no bus is opened.
* **Re-exec'd processes keep the layer's environment.** `reexec` only removes `XDG_ACTIVATION_TOKEN` and
  `DESKTOP_STARTUP_ID` and adds the handoff fd (`lib.rs:691-731`). My R5 check compared `/proc/<pid>/environ`
  before and after the restart. `DBUS_SYSTEM_BUS_ADDRESS` (`unix:path=<W>/tests/gnome/run/rt/sbus`),
  `DBUS_SESSION_BUS_ADDRESS`, `XDG_RUNTIME_DIR`, `HOME`, `PATH` (stubs first) and `WAYLAND_DISPLAY` were identical for
  all 6 hosts after the exec. The only difference my check flagged is `WIN11_RESTART_HANDOFF_FD` in both Explorers'
  `/proc/<pid>/environ` (n12). That file shows the exec's original block, while `take_handoff` removes the variable
  from the live environment before any thread or child exists. No process or descendant held the lock fd afterwards
  (my check passed).
* The layer still shows `/run/snapd.socket` and `/run/systemd/*` (GW1 review N3, D-G42). GX1 does not use the fake,
  and no GX1 path opens those sockets. This is for GW8, not a GX1 defect.

## 4. Conformance (§5, §4.9 and the §10 GX1 row)

| Requirement | As built | Verdict |
|---|---|---|
| `shell.restartAll {reason:'libraries'\|'update'}`, guard first, then `shell.restartAll {nonce, origin, at}` | `restart.rs:165-189`: only `reason` is accepted (`bad_args` otherwise). Then `restart_all_blocked`, then dry-run, then the event, which also carries `reason` | ok (D1: §5 says `power_blocked`, while §4.9's restartAll rules and the task say `restart_all_blocked`. The latter is right) |
| A handler in `NativeCore::new` for every Rust host | `shell.rs` `build(start=true)` → `watch_restart_all`. That covers GUI hosts, `--http-only` hosts and the portal's `Runtime` | ok |
| Re-exec through `request_shutdown(true)` after the per-kind delay | `plan()` `restart.rs:52-62`. On the main thread `restart_on_main` quits, and Explorer closes its windows first. Then `reexec` | ok, except **m2** (a windowless app host never restarts) |
| Delays: desktop 0, apps 300, panel 1200, origin 2000 ms | unit test plus harness (my nl run: 7 / 332 / 545 / 1238 / 2056 ms) | ok |
| The prewarmed Start ignores the event. The panel prewarms exactly one new Start | `plan("start", _, true) = Ignore`. The panel waits ≤ 2 s for its old Start (`lib.rs:554`) | ok in the harness (old Start gone 1214 ms, one new Start 1501 ms). Note **n3** (more than 2 s means no Start) |
| Fallback bar shows and hides | step's harness, sampled through `Main.extensionManager` | ok (my nl run: shown 1.61–1.72 s, hidden by 3.15 s) |
| Receivers re-check before the exec and defer until the job ends, by subscription and not by polling | `restart_when_free` re-checks `job.json` and its own Software jobs, resumes on `software.changed`/`update.changed`/`update.owner`, and watches the owner's pidfd. I found no poll in the deferral path | ok. The harness covers a Software job (step) and an update job. My R3: job written 0.4 ms after the event. Taakbeheer, both Explorers, the panel and Settings announced `wait:'update'` at 397/397/404/1297/2098 ms, and none exec'd. After I killed the owner (no event, no file change) they exec'd 49/49/85/266/401 ms later, through the pidfd |
| The origin shows its InfoBar again if a receiver deferred | through the restarted Settings' own rescan (D6) | ok. The InfoBar then stays after the deferred host has restarted (**n5**, the step's C2) |
| `" (deleted)"` strip, exec the path | `exec_path`, plus a `/proc/self/exe` fallback | ok. The fallback is not unit-tested (**n6**) |
| argv override: Settings `--app settings --params {"page":"update"}` | `restart_args` | ok (harness argv check) |
| Explorer re-execs with its argv and restores its tabs | It drops its navigation argv (D3b) and serialises the shared window-state slot with a lock handed to the new image (D3c) | deviation, justified. Two Explorers (my R1: 2 and 3 tabs, one of them restarted twice by R2): **each restored its own tabs and active tab** |
| win11-file-portal exits on the event | exits only while no window is open (D4) | deviation, accepted, but see **m3** (a request in its Preparing phase has no window yet) |
| Workers end with their hosts. We never signal another process | yes | ok |
| `mock/shell.js` `shell.restartAll` (GW3 D3) | guard on the mock's update/software state, event, simulated restart, InfoBar cleared | ok: 13/13 on the **real** mock modules (`.rt/review-gx1/mockrun/test.mjs`, see §1). Note **n7** (H26) |
| §4.11 self-test after the run: the stub log holds exactly the expected calls | an allowlist of read-only stubs (D8) | partly: **m4** (the allowlist is wider than what was observed) |
| Done-when: unit tests for the guard refusal, the delays, the strip and the argv override | 10 + 3 tests | ok. Missing: windowless path, `take_handoff` validation, the `/proc/self/exe` fallback, the reaper (**n6**) |
| Done-when: harness nl + en, every listed behaviour | step's scenario, 54/54 checks | re-run by me: nl **54/54**, en **54/54**, both rc 0, leftover [], no extra stub call |

Additions beyond the design (D5): activation tokens, the reaper, the panel's wait for its Start, and the
`shell.restartDeferred` event. All of them are in `lib.rs`/`restart.rs`. They are sound. The reaper fixes a real
zombie leak of today's `shell.restart` (the harness counts no zombie under any host). One side effect of the tokens:
every restarted app window maps focused, so Win11 app windows rise above the user's other windows. Settings maps
last and ends up in front (**n8**).

## 5. Language (check 4)

* `drift_rust.py --warnings`: 0 errors, 0 warnings. `--new --paths lib.rs shell/restart.rs shell.rs`: 99 new
  literals, of which 3 are logs (English stderr lines: `lib.rs:701`, `lib.rs:759`, `restart.rs:297`) and 96 are
  technical. There is no new Dutch literal. The existing `format!("Herstarten: {error}")` is unchanged.
* `check_rust.py --warnings`: 0 errors, 72 warnings (64 `[unused]` GW4–GW9 rows, 4 glossary, 4 space). None is in a
  GX1 file. `host/i18n/shell.tsv` is unchanged, and no new row was needed.
* The refusal texts come from `host/i18n/update.tsv`. `update.guard.restart` nl "Er worden updates geïnstalleerd.
  Win11 Shell kan opnieuw starten zodra dat klaar is." and en "Updates are being installed. Win11 Shell can restart
  as soon as that’s done." (typographic ’), and `update.guard.software` and `update.guard.upgrade` in both
  languages, equal design §3.7 character for character. I checked this in the harness replies (step's runs, mine)
  and in the real mock.
* Errors are matched on code and key (`update_busy` + `update.guard.restart`/`update.guard.upgrade`, `software_busy`
  + `update.guard.software`, `bad_args`), in the unit tests and in the harness.
* The mock's texts (`mock/shell.js`) equal the host rows in both languages through the existing `L(m, nl, en)`
  pattern of `mock/gnome.js`. `tools/i18n/ratchet.py`: `ui: ok (89 allowed occurrences, 0 still to convert)`.
  `tools/i18n/check.py`: 0 errors. No Dutch of an existing surface changed: the only vendor/ui change is a new mock
  method that renders nothing until it is clicked.
* `tests/i18n/MOCK-CHANGES.md` has no row for the new mock method (D-G41 puts those rows in round 3): **n9**.

## 6. Ownership (check 6)

* `git status --short`: modified `host/src/lib.rs`, `host/src/services/shell.rs`, `vendor/ui/js/mock/shell.js`,
  `vendor/ui-sha256.json`, `vendor/ui-patches.json`; new `host/src/services/shell/restart.rs`,
  `tests/gnome/scenarios/gx1-restart.py`, `gx1-restart.out/`, `gx1-restart.en.out/`, `notes/claude/wu/gx1.md`
  (and this review). Every path is on the step's list. `git diff --stat`: 5 files, +516/−16.
* `git diff f01710cc -- tests/gnome/harness.py tests/gnome/lib tools extension scripts host/src/i18n.rs
  host/src/window.rs host/src/runtime.rs host/i18n` is 0 lines. Nothing frozen was edited, and neither was
  `update.rs`/`update/*`, `system/*` or another vendor file.
* The screenshots in the out directories are ignored (`.git/info/exclude:17`). `tests/build/gx1-debug/` is ignored.
  My incremental `cargo build` was a no-op (0.15 s), and every one of the 12 `target/debug` binaries equals
  `tests/build/gx1-debug/SHA256SUMS`. So the harness runs used exactly the current source.
* `tools/ui-manifest.py --check`: up to date (181 files; against the original 161: 66 changed, 20 added). The
  manifest diff is the one changed hash plus `js/mock/shell.js` in `ui-patches.json`.
