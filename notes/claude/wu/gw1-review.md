# GW1 adversarial review (2026-11-05)

Reviewed: worktree `.wt/gw1` at `d666df54` (nothing committed), step GW1 (fake PackageKit + NM, private bus, control,
31 timelines, `run.sh` bwrap layer, stubs, enums, `probe.py`, one `STUBS` line). Against: design 4.6, 4.7, 4.11, 9.1,
the GW1 row of 10, plan D-G25..D-G29, the step's own note `gw1.md`.

**Verdict: fix-needed.** The layer does what it promises for the real system bus, `/run/user` and the network:
every check I ran inside it confirmed that. But it leaves every pathname unix socket outside `/run`, `/tmp` and
`/var/tmp` reachable. That includes a live-session D-Bus server (IBus in `~/.cache`) and world-writable root daemons
(nordvpnd, CUPS). There are also four major gaps: the "outside the layer" self-test, `probe.py --live` in the idle
state, the "installed but idle" PackageKit state, and the seam rule that GW2 has to follow.

I wrote only this note and scratch files under `.rt/review-gw1/`. Two side effects are noted below: I restored a
baseline file byte for byte, and one ignored `.pyc` was rewritten.

## Checks I ran (from W, numbers)

| # | Command | Result |
|---|---|---|
| 1 | `tests/update/run.sh --self-test` (not heavy, about 40 MB) | exit 0, `SELF-TEST PASSED`, 92 s, max RSS 39.6 MB. 31/31 timelines, 25,154 observer signals, 285 fake calls (the same as gw1.md). Flood: 24,017 signals in 63.6 s. 10/10 checks. strace: 98 connect() in 48 processes, 78 ok all to `/tmp/w11-update/dbus-*`, 20 ENOENT (nscd 18, sss 2), 0 to `/run/dbus/system_bus_socket` or `/run/user/1000/bus`. Guard 24/24. Outside the layer 7/7 refused, 0 bus connects |
| 2 | `tools/heavy.sh --min-mb 1024 --tag gw1-rv-guard -- python3 -B tests/gnome/harness.py guard-selftest` | 105/110. The 5 failing cases are all `[lock]`; each refuses with `build-dir-incomplete`, because W has no `tests/build/3b898e6-debug` (confirmed by a direct `launch --lock-check-only` with a scratch `WIN11_HEAVY_LOCK`). All 50+ sandbox cases pass with the new `STUBS` line. The committed baseline has 110/110. `STUBS`: 50 names, no duplicates, all 9 additions present, a 1-line diff. The run rewrote `tests/gnome/baseline/guard-selftest.json`; restored, `sha256sum -c` OK, git clean |
| 3 | `run.sh exec --strace -- python3 -I -B tests/update/test_enums.py` | `264 values … 0 differences`; strace OK (no connect outside the private buses) |
| 4 | `run.sh exec --strace -- env -u DBUS_SYSTEM_BUS_ADDRESS python3 sysbus_noenv.py` (a child without the variable) | `/run/dbus/system_bus_socket` absent; connect ENOENT; the strace check flags even the failed attempt: exit 1. **Pass** |
| 5 | `run.sh exec -- sh -c 'pkexec true; $TOOLS/loginctl enable-linger; true'` | exit 1, `stub log MISMATCH … unexpected [pkexec (guard-bin), loginctl (tools)]`. With `--expect-stubs` listing both: exit 0. With one expected call missing: exit 1. **Pass** |
| 6 | `run.sh exec --strace [--timeline available-mixed --run] -- tests/update/.run/rust-probe/tiny [--ignored] --test-threads=1` (the author's binary; I did no new build) | `1 passed; 1 ignored`, then `1 passed`; stub log OK (0, then 2 expected calls); strace OK. GW2b's path works |
| 7 | probe.py, 12 edge-case addresses, inside a bwrap where **decoy** listening sockets owned by the review sit at `/run/dbus/system_bus_socket` and `/run/user/1000/bus` (`.rt/review-gw1/decoy-probe.sh`): `//run/…`, `/run/dbus/../dbus/…`, `/proc/self/root/run/…`, a symlink to the bus in a marked dir, `%2f` escaping, `/var/run/…`, `,guid=`, `runtime=yes`, address lists, an unmarked dir | 12/12 exit 2 REFUSED; **0 decoy connections**. probe.py's private-bus check is robust |
| 8 | `run.sh exec --strace -- python3 connect_out.py <W>/.rt/review-gw1/p.sock` (a listening socket of the review, **outside** the layer, in the ro-bound real home) | **Connected; the listener received the data from inside the layer.** strace: `FAIL, a successful connect() outside the private buses`, exit 1, only because `--strace` was given. See F1 |
| 9 | `git status --short`, `git diff --stat` | only `tests/gnome/harness.py` (+1), plan.md (the coordinator's, already modified at the start), untracked `tests/update/`, `tools/update/`, `notes/claude/wu/`. Ignored: `tests/update/.run/`, `tests/update/__pycache__/`, `tests/logs/`, `.rt/`. Nothing frozen was edited |

Side effects of the review: the guard self-test rewrite of `tests/gnome/baseline/guard-selftest.json` (restored);
`python3 -m py_compile` rewrote the ignored `tests/update/__pycache__/fake_packagekit.cpython-314.pyc`, which existed
before; my `run.sh` runs pruned the old `.run/<stamp>` dirs (rotation). No real bus, no network, no win11-* binary,
no pkexec/sudo, and no `#[ignore]` test outside the layer.

## Findings (most severe first)

### F1 blocker: pathname unix sockets outside /run, /tmp and /var/tmp are reachable from inside the layer

* `run.sh:96-97` gives `--ro-bind / /` and hides only `/tmp`, `/var/tmp` and `/run`. A read-only bind does not stop
  `connect()` on a socket inode, and `--unshare-net` isolates only abstract sockets. Check 8 shows a process inside
  `run.sh exec` connecting to and writing to a socket outside.
* The host's listening sockets (`/proc/net/unix`, read only) that are outside the hidden directories and reachable for
  uid 1000:
  * `~/.cache/ibus/dbus-DcYsY2EF`: the IBus D-Bus bus of the live session (`srwxrwxr-x robin`). Its methods include
    `Exit` and `SetGlobalEngine`.
  * `/var/snap/nordvpn/common/run/nordvpn/nordvpnd.sock`: `srw-rw-rw- root`, the VPN daemon, which controls the network.
  * `/var/snap/cups/common/run/cups.sock`: `srwxrwxrwx root`.
  * `/var/lib/libvirt/qemu/domain-3-win11/fs0-fs.sock`: group `kvm` writable, and robin is in `kvm`. This is the
    virtiofsd of the running VM.
* The strace check would catch a successful connect, but `exec` runs it only with `--strace`, and GW2b's documented
  command (gw1.md, README) does not pass it.
* No current code connects to these sockets, and none of them is PackageKit, logind or the system bus. But 4.11/9.1
  promise "no live session, no network" for every process in the layer, and GW2b, GX1 and GW8 will run a lot of code
  there.
* **Fix:**
  * (a) In `run.sh`, outside the layer, list the pathname sockets of `/proc/net/unix` that are not under
    `/run`, `/tmp` or `/var/tmp`, and mask each one inside with `--ro-bind /dev/null <path>`. Also add
    `--tmpfs` over `$REAL_HOME`, followed by `--ro-bind "$W" "$W"` and the cargo target dir, plus `/var/snap` and
    `/var/lib/libvirt`.
  * (b) Have the inner guard re-stat every listed path and refuse if one is still `S_ISSOCK`.
  * (c) Add a guard self-test case: a decoy listener outside the layer (as in check 8) that must get 0 connections.
  * (d) Make `--strace` the default for `exec`, with `--no-strace` to opt out.
* GE0c's harness has the same hole, and more (see N3).

### F2 major: the "outside the layer" self-test hands the code under test the real bus addresses

* `run.sh:247-272` runs `probe.py --bus unix:path=/run/dbus/system_bus_socket --get-updates`, the same with the live
  session bus, and `fake serve --bus unix:path=/run/dbus/system_bus_socket`. All three run in the live environment,
  outside any sandbox. Only `private_bus_problem` / `serve_problems` (the code under test) stand between the
  self-test and a real `GetAll` + `CreateTransaction` + `GetUpdates` on packagekitd, from a uid-1000 process that
  polkit treats as active. That is the exact single-layer dependence D-G25 rules out.
* The checks refuse today (checks 1 and 7), but a regression would make `run.sh --self-test` itself contact the real
  system bus.
* **Fix:** run these cases inside a bwrap with decoy listening sockets at `/run/dbus/system_bus_socket` and
  `/run/user/$UID/bus` that count connections. `.rt/review-gw1/decoy-probe.sh` does this: 12 cases, 0 connections.
  Keep only the no-address cases outside.

### F3 major: `probe.py --live` fails in the normal idle state of PackageKit (R1 would get an error report)

* `probe.py:116-120` (`manager_props`) uses `NO_AUTO_START`, and `main()` calls it first (`:248`) in both modes.
* packagekitd is D-Bus activated and exits when idle: it is not running now, `ShutdownTimeout` defaults to 300 s, and
  `/usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service` exists. So Robin's `--live --get-updates`
  will usually stop at once with `NameHasNoOwner`, setting `report.error` and rc 1, and never reach
  `CreateTransaction`. This path is untested, because the private bus always has the daemon present.
* **Fix:** in `--live`, read the manager properties without `NO_AUTO_START`, or after `CreateTransaction` (which
  activates the daemon). Keep `NO_AUTO_START` on the private bus. Add a private-bus case with `SetDaemon(false)` that
  expects a clear "packagekitd not running" result.

### F4 major (design gap, needs a coordinator decision): "installed but idle, activatable" cannot be modelled

* `bus.conf` has no servicedir (as 9.1 demands), so the daemon being absent always means "not activatable", which is
  the uninstalled case (H13). The common real state is no owner but activatable.
* Design 4.6 decides presence with `NameHasOwner` + `ListActivatableNames`. A GW2 that shows `unsupported` whenever
  packagekitd is idle would pass all 31 timelines and be wrong on Robin's PC almost all the time. A call without
  `NO_AUTO_START` can also never activate anything.
* **Fix:**
  * Add `daemon: {"present": false, "activatable": true}`: `bus.conf` gets one `<servicedir>` = `<run>/services`
    (generated, holding only `org.freedesktop.PackageKit.service`, whose `Exec` calls `Control.SetDaemon(true)`), and
    still no `standard_*_servicedirs` and no includedir. The guard checks that the directory holds exactly that file.
  * Add a timeline `daemon-idle-activatable`.
* This deviates from the wording of 9.1 but keeps its intent: nothing real can be activated.

### F5 major: the seam contract with GW2 is broken and not yet decided

* The bus sockets are in `/tmp/w11-update`. That is deviation 1, for a valid reason: the 108-byte limit.
* Design 4.11 (line 883) makes `paths.rs` fail closed on "a bus address that is not `unix:path=` under the test run
  dir". GW2a, running in parallel, follows the design, so it would reject GW1's address, and GW2b would fail or
  relax the check ad hoc.
* gw1.md proposes "the marker next to the socket and not under `/run`" (gw1.md:133). That rule is weaker than the
  design: it accepts any marked directory anywhere, for example a stale `.run/<old>/tools`, or a `/tmp/w11-update`
  that anyone created on the host. It also does not tie the bus to a layer.
* **Fix (the coordinator decides before GW2b; record it in contracts.md):** accept `WIN11_UPDATE_PK_BUS` only as one
  `unix:path=` such that:
  * the socket's directory is `$XDG_RUNTIME_DIR`;
  * that directory holds the marker, is owned by the euid and has mode 0700;
  * it is not under `/run`;
  * `/run/dbus/system_bus_socket` is not visible, so the process is inside a layer.

  GW8 applies the same rule to GE0c's `RT`.

### F6 minor: Load() changes NM and PackageKit properties and drops transactions without telling clients

* `fake_packagekit.py:714-755` builds new `nm_props` and `mgr_props`, but `set_nm(True)` and `set_daemon(True)` do
  nothing when the name is already owned. The fake emits no `PropertiesChanged`, and dropped transactions get no
  `Destroy`, `Finished` or `TransactionListChanged`.
* A long-lived client keeps stale `Metered`, `Connectivity`, `NetworkState` and `Locked` values, and waits on dead
  tids. Examples: GW8's Settings host across states (for instance H14 offline, then H17 metered), and GW2b's
  two-services test.
* **Fix:** on Load, emit `PropertiesChanged` for every changed NM or manager property, and `Destroy` plus
  `TransactionListChanged` for dropped transactions. Otherwise, document "one client process per Load".

### F7 minor: plural signals are sent to clients that did not ask for them

* The shipped `org.freedesktop.PackageKit.Transaction.xml` (1.3.4, lines 1900-1903 and 2419-2422) says `Packages` and
  `UpdateDetails` are emitted only if the client set `supports-plural-signals=true`.
* The fake ignores hints: `batch` (`:901`) and the default `GetUpdateDetail` (`:921`) always send plural signals. The
  reference client's hints (`:1664`) and design 4.6's hint list do not include the hint. `available-mixed` and
  `update-details` assert plural signals to a client that never asked, which packagekitd never does.
* The timelines exercise both forms, so a GW2 that passes handles both. But the fake differs from the daemon, and a GW2
  that sets the hint is indistinguishable from one that does not.
* **Fix:** emit the plural form only when `tx.hints` contains `supports-plural-signals=true`, and the singular form
  otherwise. Run one timeline with the hint and one without. Tell GW2 (design 4.6) whether it sets the hint.

### F8 minor: SetDaemon(false) and SetDaemon(true) keep the same unique name

* PackageKit shares its connection with `/Control` (`:556`, `:687-701`). A real packagekitd exit makes its unique name
  vanish, and a restart has a new one.
* A client that tracks the owner's unique name, for example 4.6's "owner vanishing (NameOwnerChanged)" done on the
  unique name, never sees the daemon die or restart.
* **Fix:** own `org.freedesktop.PackageKit` on its own connection, closed by SetDaemon(false) and reopened by
  SetDaemon(true). NM already does this.

### F9 minor: Calls() and CallCount() race with Load()

* The message filter records calls on the GDBus worker thread when they arrive (`:659-683`). `reset()` clears the
  list on the main thread (`:724-725`).
* A call that arrives after a `Load` message but before the main loop has processed it is handled under the new
  timeline, but it is not counted. GW2b's per-test counts could then be flaky when a service is active during
  `Load`.
* **Fix:** clear `self.calls` in the filter when the `Load` method call passes (that is wire order, the same order
  GDBus dispatches in), or record in the handlers on the main thread.

### F10 minor: the 4.11 "self-test after each run" is only half done in `exec`

* `exec` asserts the stub log, but not `CallCount()`. If the fake has died, `callCount` becomes `'unavailable: …'`
  and the run still exits with the command's rc (`:1626-1631`).
* **Fix:** add `--expect-calls N|FILE`, and make an unreachable fake at the end fail the run.

### F11 minor: the strace check sees only connect()

* `run.sh:110`, `:261`. `sendto` and `sendmsg` with an address on an unconnected `AF_UNIX` datagram socket (the
  style of `events.rs`'s fan-out) bypass it.
* **Fix:** use `-e trace=connect,sendto,sendmsg` and check the `sun_path` of `msg_name` the same way.

### F12 minor: the run-dir rotation can delete a run that is still in use, and keeps 7 runs, not 6

* `run.sh:66-69` prunes by name before creating the new run. A long GW2b `exec` is deleted (HOME, tools, stub log)
  once 7 newer runs start in W. That fails closed, but it is flaky.
* My own runs deleted the self-test's `selftest.json` within minutes.
* **Fix:** hold a `flock` on `<run>/.lock` for the run's lifetime, skip locked dirs, and prune after creating.

### F13 minor: the summary says `connect() check: OK` when strace is missing

* `run.sh:112` only warns, and `:275` prints OK because `rc_strace` stays 0.
* **Fix:** print "skipped" and fail the self-test. Section 4 needs strace anyway.

### F14 minor: the self-test runs `pkexec true` and `systemctl reboot` through PATH before it has proved guard-bin

* `meta()` order (`:2142-2144`): `meta_stub_log` runs before `meta_guard_bin`. If a name were missing from guard-bin,
  the self-test would exec the real `/usr/bin/pkexec` inside the layer. That is contained by `no_new_privs` and the
  absent system bus, but still against the "no pkexec" rule.
* The scratch `tests/update/.run/rust-probe/tiny.rs` also uses `systemctl reboot`.
* **Fix:**
  * Run `meta_guard_bin` first and abort on a gap.
  * Assert `shutil.which(name)` is in guard-bin before each exec.
  * Use a harmless argv, for example `systemctl --win11-stub-probe`.
  * Delete `tests/update/.run/rust-probe/` when GW2b no longer needs it.

### F15 minor: undocumented deviation, run.sh never refuses based on the caller's environment

* Design 9.1 (line 1346): "It **refuses** when `XDG_RUNTIME_DIR` is `/run/user/<uid>` or …".
* `run.sh` instead drops the caller's environment (`env -i`) and refuses only on the environment inside. The
  protection is equivalent, and better for agents, whose shells look live.
* **Fix:** list it in gw1.md's deviations.

### F16 minor: `bus.conf` allows eavesdropping

* `bus.conf:22-23`: `eavesdrop="true"`. The real system bus does not allow it, and allow-all does not need it.
* **Fix:** remove both `eavesdrop` attributes.

### Notes

* **N1. Done-when wording.**
  * With NO_AUTO_START and the daemon absent, dbus-daemon 1.16.2 answers `NameHasNoOwner`. It answers
    `ServiceUnknown` only without the flag.
  * GW1's timeline asserts the real behaviour (correct). Design 9.1 and 10 should say so; 4.7 already accepts both.
  * The finding that the Transaction interface has a `Sender` property is confirmed in the 1.3.4 XML (line 53), which
    contradicts 4.7 (line 782).
* **N2. Fidelity points to verify against the packagekit 1.3.4 source before GW2b relies on them:**
  * The fake sends `Destroy` 50 ms after `Finished`; as far as I know, packagekitd keeps a finished transaction for
    some seconds first.
  * `untrusted` emits `ErrorCode 51` (cannot-update-repo-unsigned) and then exit 8. packagekitd most likely turns
    that error into exit `need-untrusted` with no `ErrorCode` under ONLY_TRUSTED.
* **N3. GE0c's harness layer** (`harness.py:474-483`, used by GW8 and GX1 with the fake) hides only `/run/dbus` and
  `/run/user`.
  * `/run/snapd.socket` (`srw-rw-rw-`; snapd asks polkit itself) and `/run/systemd/*` stay visible there, plus the
    sockets of F1.
  * This is not GW1's file. GW1's `--tmpfs /run` and the F1 masking should be adopted there before GW8.
* **N4. Shebangs.** `fake_packagekit.py`, `probe.py` and `test_enums.py` start with `#!/usr/bin/env python3`.
  Running them directly, as Robin may do for R1, skips `-I` and takes `python3` from PATH. Use `#!/usr/bin/python3 -I`,
  as `stubs/stub.py` does.
* **N5. Small items.**
  * `serve --timeline` is never used (`:2416`).
  * The only Dutch literal is test data (`:2245`, the Notify payload), not a UI text.
  * There are no user-facing texts, so the en-GB and ’ rules do not apply.
  * `tests/update/__pycache__/` comes from an import without `-B`.
* **N6. Verified OK.**
  * Every manager and transaction method, property and signal of 9.1. The emitted signal signatures equal the shipped
    XML (11 of 11).
  * All 31 timelines exist, with the design's values: status 30 ×6, 31, error 48 + exit 2, AllowCancel → Cancel →
    exit 3, CannotCancel in commit, RequireRestart 4, error 58 halfway, error 66 + exit 11 then RepairSystem, 46,
    exit 8, 13, infos 13/28/30, Metered 1 + NetworkState 5 then 3, a phased package with info 5 plus the forbidden-id
    violation, keep-one success, error 65 + exit 9, 39, foreign uid 0 role 22, `onVanish: cancel`,
    GetOldTransactions, UpdatesChanged, flood 200/s × 60 s.
  * The control API, broadcast emission and the unicast control.
  * `enums.json` equals the typelib.
  * The stub semantics, `nice`/`ionice` pass-through, guard-bin ⊇ GE0b (43) ∪ GE0c (50), `no_new_privs`, PID 1 =
    bwrap, and loopback only.
  * `probe.py` without `--live` refuses everything but a marked private bus (check 7). `--live` refuses inside a test
    environment, without a tty, and without the typed word.

## Ownership and language

* **Ownership:** only owned files are touched, and the `STUBS` line is the single harness change. No i18n, host,
  extension, scripts or tools/i18n file changed, and no Dutch changed.
* **Not applicable:** the Windows 11 look and the catalogue rules (GW1 has no UI).
