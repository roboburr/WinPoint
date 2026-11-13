# GW1: fake PackageKit + NetworkManager, test layer, timelines, probe (2026-11-05)

Worktree `.wt/gw1` (branch `wu/gw1`, HEAD `d666df54`; at the start only `notes/claude/plan.md` was modified).
Design: `windows-update-gnome.md` 9.1, 4.11 (binding, D-G25), 4.6–4.8, GW1 row of section 10. Nothing was committed.
No real system bus, no live session bus, no PackageKit, no apt, no network, no win11-* binary, no gsettings was used.

## Files

| File | Lines | What |
|---|---|---|
| `tests/update/run.sh` | 289 | the bwrap layer; `--self-test`, `exec … -- CMD`; guard cases; strace connect() check |
| `tests/update/bus.conf` | 26 | `unix:dir=/tmp/w11-update`, EXTERNAL, allow-all, no `servicedir`/`standard_*_servicedirs`/`includedir`, AppArmor off |
| `tests/update/fake_packagekit.py` | 2441 | `serve` (the fake), `layer` (in-layer driver), `selftest` (reference client + 10 checks), `guard`, `check-stubs` |
| `tests/update/timelines/*.json` | 31 files | the 31 timelines of 9.1, self-contained (packages, scripts, reference-client flow, expected CallCount and stubs) |
| `tests/update/enums.json`, `test_enums.py` | 119 | pinned enum values; equal to PackageKitGlib-1.0 1.3.4 and NM-1.0 (no D-Bus) |
| `tests/update/stubs/stub.py`, `stubs/names.json` | 115 | one stub (log argv as JSON, then exit 97); 14 tool names, 88 guard-bin names |
| `tests/update/README.md` | 145 | usage, layer, control API, timeline format |
| `tools/update/probe.py` | 275 | R1 probe; without `--live` only a marked private bus |
| `tests/gnome/harness.py` | +1 | `STUBS` += `xdg-terminal-exec screen apt-cache apt-config check-new-release ptyxis gnome-terminal kgx unattended-upgrade` |

`fake_packagekit.py` is four times the design's 600-line estimate because the owned-file list has no separate
client file: the reference client and its checks (about 900 lines) live in the same file as the fake.

## Verification (exact commands, all from W)

1. `tests/update/run.sh --self-test` → exit 0, `SELF-TEST PASSED`, 90 s wall, largest process 40 MB RSS:
   * **31/31 timelines** against the Python reference client inside the layer; 25,154 signals seen by the observer
     connection, 285 fake calls in total; per timeline the fake's `Calls()` equals, in order, exactly what the
     client sent (method + path), `CallCount() == len(Calls())`, and equals the hand-written `expect.callCount`.
     Flood: 12,000 ItemProgress + 12,000 PropertiesChanged in 63.1 s, all received.
   * **10/10 checks**: 31 timelines load; CallCount 0 after Load, 1 after one GetAll, NO_AUTO_START recorded;
     paused mode (nothing before `Step()`; 47 steps = 47 signals to Finished/Destroy, then `"idle"`); unicast
     control (a unicast signal reaches only its destination: B 2, observer 1); forbidden id gives 1 violation;
     fake Notifications on the private session bus; **stub log** (pkexec, systemctl, apt-get, busctl via PATH
     and `env -i <tools>/loginctl` all exit 97 and are logged; the check reports all 5 as unexpected, and passes
     with them listed; `nice -n 10 python3` passes through to the tool stub, `nice /usr/bin/id` is refused);
     guard-bin covers all 43 GE0b and 50 GE0c names; `test_enums.py`; probe.py inside the layer.
   * **Broadcast**: every client connection's signal sequence is a subsequence of a second (observer)
     connection's, in all 31 timelines; the observer saw no unicast signal; plus the unicast control above.
   * **daemon-absent**: GetTransactionList with NO_AUTO_START → `org.freedesktop.DBus.Error.NameHasNoOwner`,
     without the flag → `org.freedesktop.DBus.Error.ServiceUnknown` (nothing activatable); neither reaches the fake.
   * **strace** (`strace -f -ff --seccomp-bpf -e trace=connect` over bwrap and everything inside, 48 processes):
     98 connect() calls: 78 succeeded, all to `/tmp/w11-update/dbus-*` (the private buses); 20 failed with ENOENT
     (`/var/run/nscd/socket` 18, `/var/lib/sss/pipes/nss` 2: NSS user lookups); **0 to
     `/run/dbus/system_bus_socket` or `/run/user/1000/bus`**.
   * **Guard 24/24**: positive control accepted; refused: a file at `/run/dbus/system_bus_socket` inside the layer
     (`system-bus-visible`), `XDG_RUNTIME_DIR=/run/user/1000`, the live session bus plain / `%2F`-escaped / second
     in a list, `DBUS_SYSTEM_BUS_ADDRESS` = the real socket, `WIN11_UPDATE_PK_BUS` under /run/user, a preset seam,
     abstract and tcp buses, system bus unset, real HOME, DISPLAY, WAYLAND_DISPLAY, AT_SPI_BUS_ADDRESS,
     XDG_SESSION_ID, NO_AT_BRIDGE missing, dconf backend, PATH without guard-bin, `~/.local/bin` on PATH, a variable
     naming /run/user, the composite live environment, and the inner stage started without the layer.
   * **Outside the layer 7/7** (under strace): probe.py without args, with `--get-updates`, with the real
     system-bus and the live session-bus address, `fake serve` on the real socket and on a layer path, `fake layer`:
     all refused (exit 2), none attempted a bus socket, no connect() succeeded.
2. `python3 -I -B tests/update/test_enums.py` → `264 values checked against PackageKitGlib 1.3.4 (NM-1.0
   compared), 0 differences` (also run inside the layer by the self-test).
3. `python3 -B tests/gnome/harness.py guard-selftest` with the STUBS line → **110/110**. W has no
   `tests/build/3b898e6-debug` (ignored, only in G), so the 5 lock-decision cases first refuse with
   `build-dir-incomplete` (105/110); with the line removed the result was identical case by case. The 110/110
   run used 12 placeholder executables in `tests/build/3b898e6-debug` (never executed: those cases stop at
   `--lock-check-only`), removed afterwards. The self-test rewrites `tests/gnome/baseline/guard-selftest.json`; I
   restored it byte for byte (git shows it unmodified).
4. GW2b's path, with a std-only Rust test binary (`tools/heavy.sh --min-mb 1024 --tag gw1-rustc-tiny --
   ~/.cargo/bin/rustc --edition 2021 --test -o tests/update/.run/rust-probe/tiny tests/update/.run/rust-probe/tiny.rs`,
   rc 0; source and binary stay in the ignored `.run/`):
   * `tests/update/run.sh exec --strace --timeline available-mixed --run -- tests/update/.run/rust-probe/tiny
     --test-threads=1` → 1 passed (seams, marker files, no /run/dbus socket, XDG_RUNTIME_DIR, PATH), 1 ignored;
     stub log OK (0 calls); CallCount 0; strace OK.
   * `… exec --strace --expect-stubs tests/update/.run/rust-probe/expect-ignored.json -- … tiny --ignored
     --test-threads=1` → the `#[ignore]` test ran inside the layer and passed (tool stub with `env_clear()` and
     `systemctl reboot` via PATH both exit 97, private bus socket connectable); stub log OK (2 expected calls).
   * the same without `--expect-stubs` → the tests pass but `exec` exits 1: `stub log MISMATCH … unexpected
     [loginctl show-user (tools), systemctl reboot (guard-bin)]`.

## For GW2b (and GW8)

**Run a built Rust test binary inside the layer.** Build outside (the layer's HOME is read-only):

```bash
tools/heavy.sh --tag gw2b-build -- tests/rust-sandbox/run-tests.sh --lib --no-run
BIN=$(grep -o 'target/debug/deps/win11_host-[0-9a-f]*' tests/rust-sandbox/logs/test.log | tail -1)
tests/update/run.sh exec [--strace] [--expect-stubs FILE.json] [--timeline NAME [--run]] [--timeout SEC] -- \
    "$BIN" services::update::bus_tests:: --ignored --test-threads=1
```

Exit code = the binary's, or 1 if the stub log does not match (default expectation: no stub call at all).
`--test-threads=1` is required (one fake; `Load()` is global). stdin is /dev/null, cwd is W, only `<run>`
(`tests/update/.run/<stamp>/`) and `/tmp/w11-update` are writable. Results: `<run>/calls.json` (every fake call
with `noAutoStart`), `<run>/stub-calls.jsonl`, `<run>/logs/fake.log`, `<run>/result.json`. Not run under the heavy
lock (about 60 MB); the cargo build is.

**Environment of every process in the layer** (built with `env -i`):

| Variable | Value |
|---|---|
| `DBUS_SYSTEM_BUS_ADDRESS` = `WIN11_UPDATE_PK_BUS` | `unix:path=/tmp/w11-update/dbus-XXXXXXXXXX,guid=…` (printed by dbus-daemon; parse the `,guid=`) |
| `DBUS_SESSION_BUS_ADDRESS` | a second private bus in the same directory, with a fake `org.freedesktop.Notifications` |
| `WIN11_UPDATE_FIXTURE_ROOT` | `<run>/fixture` (contains `.win11-update-fixture`) |
| `WIN11_UPDATE_TOOL_DIR` | `<run>/tools` (contains `.win11-update-fixture`; stubs `loginctl xdg-terminal-exec pkexec check-new-release apt-get apt-cache apt-config dpkg-query snap python3 screen nice ionice sh`) |
| `XDG_RUNTIME_DIR` | `/tmp/w11-update` (0700, contains `.win11-update-fixture`; holds the bus sockets) |
| `PATH` | `<run>/guard-bin:/usr/bin:/bin` |
| `HOME`, `XDG_{CONFIG,DATA,CACHE,STATE}_HOME` | `<run>/home`, `<run>/config`, `<run>/data`, `<run>/cache`, `<run>/state` (so the update cache is `<run>/cache/win11-gnome/update/`) |
| `TMPDIR` | `/tmp/w11-update/tmp` |
| others | `NO_AT_BRIDGE=1 GTK_A11Y=none GSETTINGS_BACKEND=keyfile LANG=C.UTF-8 TERM=dumb RUST_BACKTRACE=1 WIN11_FAKE_RUN_DIR=<run>`; no DISPLAY/WAYLAND_DISPLAY |

`WIN11_FAKE_RUN_DIR` deliberately does not start with `WIN11_UPDATE_`, so the service's "any `WIN11_UPDATE_*` set
= test mode" rule sees exactly the three seams.

**Control API** (`io.github.win11shell.FakePackageKit`, path `/Control`, on `WIN11_UPDATE_PK_BUS`): `Load(s)`,
`Step() → s`, `Run()`, `SetDaemon(b)`, `Calls() → a(ssss)`, `CallCount() → u`, plus read-only extras `State() → s`
and `CallsJson() → s` (README). Semantics a bus test must know:
* `Load()` resets everything (transactions, `Calls()`, updates, history, stub responses, fixture files) and leaves
  the fake **paused**: scripted events wait for `Step()`/`Run()`. A bus test normally does `Load(name)`, `Run()`,
  then drives the service.
* `Calls()` counts every call that reached the fake's PackageKit, NetworkManager and Notifications objects,
  including `Properties.Get`/`GetAll` (method `Properties.GetAll`). Calls the bus rejects (no owner) are not counted.
  So "dry-run → CallCount 0" also covers property reads.
* `State().violations` lists forbidden ids that reached `UpdatePackages` (timeline `forbiddenIds`, e.g. the phased
  package in `phased-as-normal`) and scripted events that failed; a test should assert it is empty.
* Each timeline's `client` array is the reference flow (what a correct service sends, in order), and
  `expect.callCount` the resulting count; GW2b's service may differ (hints, extra reads), so assert its own counts.

**Phasing stub output** (the `stubs.python3` response; also the output of `PHASING_SCRIPT` in
`tools/update/probe.py`): one line per package name on stdin,
`name<TAB>candidate version|-<TAB>phasing_applied 0|1<TAB>Phased-Update-Percentage|-<TAB>held 0|1`.
GW2's `phasing.rs` should run exactly `PHASING_SCRIPT` (copy it byte for byte) and parse this format; if GW2a chose
another format, the timelines' `stubs.python3.stdout` (3 files: `available-mixed`, `only-phased`,
`phased-as-normal`) must follow.

**Expected-stub file** (`--expect-stubs`, also `fake_packagekit.py check-stubs`): a JSON list in call order of
`"tool"` or `{"tool", "dir": "tools"|"guard-bin", "argv": [...], "argvPrefix": [...]}`.

**Suggested seam rule for `paths.rs`** (GW2's decision): accept `WIN11_UPDATE_PK_BUS` only as one `unix:path=`
whose socket directory holds `.win11-update-fixture` and is not under `/run`; that is what `probe.py` and
`fake serve` enforce. In this layer the marker is in `/tmp/w11-update`; GW8 must write it into GE0c's `RT` before
running the fake on `RT/system-bus` (`fake serve` refuses an unmarked directory and any sandbox where
`/run/dbus/system_bus_socket` is visible or PID 1 is not bwrap).

**GW8 screen mapping**: each timeline's `states` lists the screen ids it serves (H01–H26 and R01–R05 of 9.3).
Not covered by a PackageKit timeline (they are fixtures or other services): H20 (libsoup restart InfoBar: procfs
fixture), H25 (`releaseRunning`: procfs fixture), H26 (`foreignOwn`: a Software job file), the card, Meer opties,
subpages and post-upgrade states.

## Deviations from the design or contracts

1. **NO_AUTO_START with the daemon absent gives `NameHasNoOwner`, not `ServiceUnknown`** (dbus-daemon 1.16.2,
   measured). `ServiceUnknown` is the answer without the flag. The design's reattach rule already accepts both
   (4.7 step 1); the done-when wording is off. The timeline asserts both exactly.
2. **The bus sockets live in `/tmp/w11-update`** inside the sandbox (its own tmpfs, gone with the sandbox), not in
   `tests/update/.run/<stamp>/`: that path is 111+ bytes, over the 108-byte `sockaddr_un` limit (C8 allows /tmp
   for one run's sockets). Everything else of a run is in `.run/<stamp>/` (0700). `.run/` ignores itself through a
   generated `.run/.gitignore` (no tracked file was needed); the newest 6 runs are kept.
3. **The layer hides all of `/run`** (`--tmpfs /run --dir /run/dbus --dir /run/user`), stricter than GE0c's
   `/run/dbus` + `/run/user` only: also `/run/systemd/private`, `/run/snapd.socket` (snapd asks polkit itself) and
   the real `/run/reboot-required`. No GPU device binds (nothing renders). Otherwise GE0c's binds and flags.
4. **guard-bin is the layer's own** (`<run>/guard-bin`, 88 names from `stubs/names.json`), not GE0b's
   `tests/rust-sandbox/guard-bin/`, which `run-tests.sh` generates and git ignores; the self-test proves it is a
   superset of GE0b's 43 and GE0c's 50 names. `python3`, `sh`, `nice`, `ionice` are tool-dir stubs only (the layer
   needs the real ones on PATH; they reach nothing inside it).
5. **No `run.sh cargo`** (9.2): 4.11 requires building outside and running only the binary inside, so the
   interface is `run.sh exec -- <binary>`, verified with a real Rust test binary.
6. **Extras beyond 9.1**: `State()`/`CallsJson()`; a fake `org.freedesktop.Notifications` on a private session
   bus (for 9.2's two-services test); NM on its own connection (own unique name, as in reality); timelines can
   write stub responses and fixture files on Load and during a script (`require-restart-system` writes
   `run/reboot-required{,.pkgs}`); `forbiddenIds` violations; `onVanish`; `until` gates (`call:<Method>`,
   `vanish`, `step`).
7. **Enum names are PackageKit's own strings** (`pk_*_enum_to_string`), which differ from the GObject nick for
   error 60 (`failed-due-to-running-process` vs `update-failed-due-to-running-process`).

## Findings for the coordinator

* **The Transaction interface has a `Sender` property** in packagekit 1.3.4's shipped XML ("The D-Bus name of the
  process that started the transaction"); design 4.7 says it has none. The fake implements it. It does not
  replace the tid rule (a reattached service has a new unique name), but GW2 can use it to tell "started by this
  process" apart cheaply.
* The PackageKit D-Bus error names used by the fake are taken from `strings /usr/libexec/packagekitd`
  (`…Transaction.CannotCancel`, `RefusedByPolicy`, `NotRunning`, `TransactionExistsWithRole`, …).
* `tests/gnome/harness.py guard-selftest` cannot pass in a worktree without `tests/build/3b898e6-debug` (5 lock
  cases refuse with `build-dir-incomplete`); independent of GW1. Running it also rewrites the committed
  `baseline/guard-selftest.json` (timestamps).
* `PHASING_SCRIPT` exists once, in `tools/update/probe.py`; GW2's `phasing.rs` must carry the same text (a later
  test can compare them). Neither the script nor `probe.py --live` was ever run here (design 9.4: R1 only).

## Left / not done

* `probe.py --live` (and the real `apt_pkg` call) is untested by design; it refuses inside a test environment,
  without a terminal, and until the word `probe` is typed. R1 is Robin's.
* GW2b's real bus tests, GW8's harness use of the fake on `RT/system-bus` (needs the marker there) and the
  GX1/GW8 stub-log checks use the interfaces above; they were not exercised beyond the std-only Rust binary.

## Fix round (2026-11-05, after `gw1-review.md`)

At the start W matched the review's view: `git status --short` showed only `notes/claude/plan.md` (the
coordinator's) and `tests/gnome/harness.py` (+1, the STUBS line) modified, `tests/update/`, `tools/update/` and
`notes/claude/wu/` untracked. Nothing was committed. **Where this section and the ones above disagree, this
section is current** (the layer, the guard, the timeline count, the `exec` options, the seam rule).

### Findings

| Id | Status | What changed |
|---|---|---|
| F1 blocker | fixed | The layer is now an **allowlist root**: read-only `/usr` (+ the `/bin /sbin /lib /lib64` links), `/etc`, `/sys`, W; writable only `<run>`; tmpfs `/tmp`, `/run`, `/var` (`/var/run -> /run`); `--remount-ro /`. The real home (with `~/.cache/ibus`), `/var/snap` (nordvpnd, CUPS), `/var/lib` (libvirt), `/srv`, `/opt`, `/mnt`, `/media` do not exist inside. Outside the layer `run.sh` reads `/proc/net/unix` and masks every bound host socket that would still be visible (under `/usr`, `/etc`, `/sys`, W) with `--ro-bind /dev/null`, listing them in `<run>/masked-sockets.txt`. The inner stage re-stats each listed path and refuses one that is still a socket, refuses any socket in W (`find -type s` plus `stat`, because the directory entry of a masked path still says "socket"), and refuses a missing list. Self-test: a listening decoy in W created before the scan gets 0 connections; strace is now the default for `exec` (`--no-strace` opts out). This goes further than the review's fix (tmpfs over the home, /var/snap, /var/lib/libvirt): new socket locations outside the allowlist are hidden without being named. |
| F2 major | fixed | No refusal case runs in the live environment any more. Section 4 of the self-test runs all 26 cases (17 `probe.py`, 8 fake subcommands including `activate`, the inner stage) in a **decoy sandbox**: the same allowlist root, a live-looking environment, and listening decoys bound at `/run/dbus/system_bus_socket` and `/run/user/1000/bus`. A positive control first proves that the decoys answer there (a plain connect reaches both: 2 connections). Every case must refuse (exit 2), leave the decoy count unchanged and pass its own strace check. The review's 12 edge addresses are among them, plus "a valid private bus while the real socket is visible". |
| F3 major | fixed | `probe.py` first asks the bus driver `NameHasOwner` and `ListActivatableNames` and records `daemon: {running, activatable}`. Neither running nor activatable: `errorCode: "packagekitd-not-available"`, exit 1, nothing sent to PackageKit. Otherwise `GetAll` goes **without** NO_AUTO_START, which starts an idle packagekitd (read-only), in both modes. This differs from the review's "keep NO_AUTO_START on the private bus": with one code path, the private bus exercises R1's idle case for real (F4 makes the fake activatable). Self-test: idle and activatable gives exit 0, 1 activation and 2 updates; not installed gives exit 1, the error code and CallCount 0. The `--live` prompt now says that it may start packagekitd. |
| F4 major | fixed, **needs the coordinator** | The private system bus uses a generated `<run>/bus-system.conf`: `<include>` of `bus.conf` plus ONE `<servicedir>` `<run>/services`. That directory holds nothing, or only `org.freedesktop.PackageKit.service` (`Exec=/usr/bin/python3 -I -B …/fake_packagekit.py activate`). The `activate` helper refuses outside the layer and asks the fake (control method `Activate()`, an extra) to own the name again. `daemon.activatable` defaults to true (the real package installs the service file); `daemon-absent` sets false. `Load` writes or removes the file and calls `ReloadConfig`. The guard checks `bus.conf` (no service dirs, no include, no eavesdrop), the wrapper (byte-exact), and the directory (only that file, byte-exact). The private session bus can activate nothing. New timeline **`daemon-idle-activatable`** (32 in total): NameHasOwner false, ListActivatableNames contains PackageKit, NO_AUTO_START gives NameHasNoOwner, a plain GetAll activates (State().activations 1), and after SetDaemon(false) the next call activates a new daemon with a new unique name. |
| F5 major | rule implemented, **the coordinator decides** | `fake serve`, `fake activate` and `probe.py` (private mode) now enforce the review's rule: one `unix:path=` whose socket is directly in `$XDG_RUNTIME_DIR`; that directory is not under `/run`, is owned by the euid with mode 0700 and holds the marker; and `/run/dbus/system_bus_socket` is not visible. The layer satisfies it (`/tmp/w11-update`), and so does GE0c's `RT` (mode 0700 at `harness.py:903`) once GW8 writes the marker into it. Proposal for `contracts.md` / `paths.rs`: exactly this rule, in place of "under the test run dir" (4.11), which the 108-byte `sockaddr_un` limit makes impossible here. |
| F6 minor | fixed | `Load()` emits, for clients that stay connected: `Destroy` for every transaction object that goes away while the daemon keeps running, `TransactionListChanged([])` if one of them was active, and `PropertiesChanged` for every manager and NM property whose value changes (NM `StateChanged` as well). A daemon or NM that appears or disappears shows as `NameOwnerChanged`. Check `load_transitions`: after Load(metered) and Load(idle-none), one client sees Destroy 1, TransactionListChanged 1 (empty) and PropertiesChanged 2 (NetworkState 2, Metered no). |
| F7 minor | fixed | `Packages` and `UpdateDetails` go out only when the transaction's hints contain `supports-plural-signals=true`; otherwise the same content goes out as singular signals (`LastPackage` follows). A singular `updateDetail` event stays singular even with the hint, which the XML allows. `available-mixed`: GetUpdates with the hint gives 1 Packages; without it, 24 Package. `update-details`: without the hint 3 UpdateDetail; with it 1 UpdateDetails; a one-package answer with the hint gives 1 UpdateDetail. The reference client's default hints still lack the hint. |
| F8 minor | fixed | PackageKit is owned by its own connection (`pkconn`), opened by SetDaemon(true) and closed by SetDaemon(false), as NM already was; transactions are registered on it, and emits use it. Check `pk_restart`: owner `:1.76` becomes `:1.85`, the old unique name is gone, there are 2 NameOwnerChanged (`old→''`, `''→new`), and the control name and NM keep their owners. |
| F9 minor | fixed | `Calls()` is cleared by a message filter on the control connection when the `Load` message is read, on the same GDBus worker thread that records the other calls, and no longer in `reset()`. GDBus dispatches in the order its worker reads, so a call counts exactly when it is handled under the new timeline. Check `calls_race`: 20 rounds of Load + Get pipelined on one connection, where the reply value shows which timeline handled it; all 20 were consistent. **Negative control**: with the pre-fix clearing restored in a temporary edit, all 20 rounds failed (`Get -> 2 (handled after Load), CallCount 0`). The file was then restored, `sha256sum -c` OK. |
| F10 minor | fixed | New `exec --expect-calls N|FILE.json`, with exact count, or an ordered matcher list on method, path, `to`, `noAutoStart`, args and `argsPrefix`. A fake that is dead or unreachable at the end fails the run, and so does a non-empty `State().violations`. |
| F11 minor | fixed | strace traces `connect,sendto,sendmsg,sendmmsg`. It fails on any attempt under `/run/dbus/`, `/var/run/dbus/` or `/run/user/`, on a successful connect() anywhere other than the private buses, on any datagram sent with a unix address, and on an interrupted call with such an address. The check streams through files: the full self-test has 107,451 such calls, and the largest process stays at 33.7 MB. The logs are kept as `<run>/strace/*.tar.gz` (about 0.5 MB). |
| F12 minor | fixed | Each run dir is locked (`flock` on `<run>/.lock`) by the shell that created it, and every process of the run inherits the lock. Pruning runs after the new dir exists, keeps the newest 6 and skips locked dirs. |
| F13 minor | fixed | Without strace the self-test prints `MISSING` and fails, and `exec` exits 3 unless `--no-strace` is given. |
| F14 minor | fixed | `meta_guard_bin` runs first. It checks with `shutil.which` that every guard, GE0b and GE0c name resolves to guard-bin; with a gap, `stub_log` and `probe` are skipped and fail. `meta_stub_log` runs a stub only after `which` resolves it into guard-bin, and calls it by that absolute path with the argument `--win11-stub-probe` instead of `true` or `reboot`. The scratch Rust test now calls stubs by their absolute paths with that argument. `.run/rust-probe/` was deleted after the verification below. |
| F15 minor | documented | See deviation 12 below. |
| F16 minor | fixed | Both eavesdrop attributes are gone. Removing `<allow eavesdrop="true"/>` also removed the only receive rule, and dbus-daemon then rejected every reply ("Rejected receive message, 0 matched rules"). `bus.conf` therefore now has the four receive rules of the real `system.conf`, without eavesdrop. |
| N1 note | for the coordinator | Not GW1's file. The design text should say: NO_AUTO_START with the daemon absent gives NameHasNoOwner; ServiceUnknown is the answer without the flag only when nothing is activatable; with the real (activatable) daemon, a call without the flag starts it. |
| N2 note | modelled, not verified | The packagekit source is not on this machine, and there is no network. Evidence from the installed binary: `packagekitd` contains "already finished, so waiting to timeout", and NEWS says "Remove the config option 'TransactionKeepFinishedTimeout'", so the keep-finished delay is fixed in the code. `download-install` now has `destroyMs` 5000 on the real install. Between Finished and Destroy the client checks that GetTransactionList no longer lists the transaction, that its properties still answer (Status finished, Role update-packages), and that Destroy follows. The default stays 50 ms (5 s for each of about 100 transactions would add minutes); a timeline can set top-level `destroyMs`. `untrusted` now ends with exit 8 and **no** ErrorCode. That is the stricter model: a client that maps exit 8 alone also copes if packagekitd does send error 51. |
| N3 note | for the coordinator | Not GW1's file (`harness.py` is frozen apart from the STUBS line). GE0c's layer still has `--ro-bind / /` with only `/run/dbus` and `/run/user` hidden. It should adopt this allowlist root (or at least `--tmpfs /run`, the home and `/var`) and the socket scan before GW8 runs the fake there. `fake serve` already refuses unless that sandbox meets the F5 rule. |
| N4 note | fixed | `probe.py`, `fake_packagekit.py` and `test_enums.py` start with `#!/usr/bin/python3 -IB`. |
| N5 note | fixed | `serve --timeline` is gone. The Notify test payload is neutral (`win11-gw1-selftest` / `Test notification`). The ignored `tests/update/__pycache__` and `tools/update/__pycache__` were removed after my own py_compile check. |

### Verification (fix round, all from W, numbers)

1. `tests/update/run.sh --self-test` gave exit 0 and `SELF-TEST PASSED` in 102 s wall (`/usr/bin/time`: 1:42.6), with a largest process of 33.7 MB. Log: `.rt/gw1-fix/selftest2.log`; its run dir was later pruned by rotation.
   * **32/32 timelines** against the Python client inside the layer: 25,213 observer signals and 298 fake calls. In each timeline, Calls() equals what the client sent and CallCount() equals `expect.callCount`. Flood: 24,017 signals in 63.5 s.
   * **15/15 checks**:
     * guard_bin: 88 names, a superset of GE0b's 43 and GE0c's 50; every one resolves to guard-bin.
     * timeline_set: 32 timelines.
     * callcount.
     * calls_race: 20/20.
     * paused_step: 47 steps, 47 signals.
     * unicast_control: B 2, observer 1.
     * load_transitions.
     * pk_restart.
     * activation: daemon-absent `[org.freedesktop.DBus]`; idle `[org.freedesktop.DBus, org.freedesktop.PackageKit]`; session bus `[org.freedesktop.DBus]`.
     * forbidden_id.
     * notifications.
     * outside_socket: a char device, ECONNREFUSED, 1 path masked.
     * stub_log: 5 unexpected calls caught, all exit 97.
     * enums: 264 values, 0 differences.
     * probe: 5 refusals in the layer; private bus exit 0; idle and activatable exit 0 with 1 activation; not installed exit 1.
   * **Outside decoy** (a listening socket in `tests/update/.run/`, created before the layer): 0 connections. The socket scan saw 140 bound host socket names: 1 was masked (the decoy), and 13 were relative names, which the inner find covers for W.
   * **strace**, over bwrap and all 75 processes: 107,451 connect/sendto/sendmsg/sendmmsg calls, 132 of them with a unix address:
     * 99 connect() to `/tmp/w11-update/dbus-*`, all succeeded;
     * 24 to `/var/run/nscd/socket` and 8 to `/var/lib/sss/pipes/nss`, all ENOENT;
     * 1 to the outside decoy, ECONNREFUSED;
     * **0 to `/run/dbus/system_bus_socket` or `/run/user/1000/bus`, and 0 addressed datagrams**.
   * **Guard 25/25**: the 23 earlier in-layer cases, plus "an unmasked socket in the worktree (created after the scan)" refused as `host-socket-visible`, plus "no masked-sockets list" refused as `masked-list-missing`. The old no-layer case moved to section 4.
   * **Outside run.sh's layer, in the decoy sandbox: 26/26 refused**. Each exited 2, its strace check passed, and the decoy count did not change. The positive control reached both decoys (2 connections); there were 0 further decoy connections.
2. **daemon-absent** (not installed): NO_AUTO_START gives NameHasNoOwner, no flag gives ServiceUnknown, ListActivatableNames lacks PackageKit, and neither call reaches the fake. **daemon-idle-activatable**: NO_AUTO_START gives NameHasNoOwner; without the flag the daemon is activated and the call is answered.
3. **Second connection**: in all 32 timelines, every client's signals are a subsequence of the observer's, and the observer saw no unicast signal (unicast control B 2 / observer 1).
4. `python3 -I -B tests/update/test_enums.py` (and `./test_enums.py` through the new shebang): 264 values, 0 differences.
5. `tools/heavy.sh --min-mb 1024 --tag gw1-fix-guard -- python3 -B tests/gnome/harness.py guard-selftest` gave **110/110**.
   * As in the first round, I put 12 placeholder files (`#!/bin/sh … exit 97`, never run: the lock cases stop at `--lock-check-only`) in the ignored `tests/build/3b898e6-debug`, and deleted them afterwards.
   * `baseline/guard-selftest.json` was restored, and `sha256sum -c` says OK. The STUBS line is unchanged.
6. GW2b's path:
   * I rebuilt the scratch std-only test with `tools/heavy.sh --min-mb 1024 --tag gw1-fix-rustc-tiny -- ~/.cargo/bin/rustc --edition 2021 --test`. It now also asserts that `/home/robin/.cache`, `/var/snap` and `/var/lib` do not exist inside.
   * `exec --timeline available-mixed --run -- tiny --test-threads=1`: 1 passed, 1 ignored; stub log OK with 0 calls; strace OK.
   * `exec --expect-stubs expect-ignored.json -- tiny --ignored`: 1 passed; 2 expected stub calls; strace OK.
   * The same without `--expect-stubs` exits 1 (MISMATCH).
7. F10:
   * `--expect-calls 0 -- /usr/bin/true` exits 0, and `--expect-calls 1` exits 1.
   * A matcher file `["Properties.GetAll"]` with a one-GetAll client exits 0. The same file with `noAutoStart: false` exits 1.
   * A command that SIGKILLs the fake exits 1 (`the fake exited with -9 during the run`).
   * A command outside W and /usr exits 2 before anything starts.
8. F1 repro (the review's check 8): a listener at `.rt/gw1-fix/p.sock` started before `exec -- python3 connect_out.py <it>` got connect refused. The listener logged 0 connections, and strace was OK. The stale socket file it left behind made the next layer refuse (`host-socket-visible`, fail closed), until I removed it.
9. F11: inside the layer, a datagram sent with `sendto(..., {sun_path="/tmp/w11-update/tmp/dgram"})` made `exec` exit 1 (`strace: FAIL, a datagram sent with a unix address`).
10. F12: a 20 s `exec` stayed alive while 8 newer runs started. Its dir survived (7 dirs), and it finished with exit 0 and its `result.json`. The next run after it ended pruned it (6 dirs).
11. F13: with a PATH without strace, `exec` exits 3 and the self-test exits 1 (`connect()/sendto() check: MISSING`).
12. `git status --short` at the end shows the same as at the start. `git diff --stat` shows only `harness.py` (+1) and plan.md (the coordinator's). Nothing frozen was edited. Scratch logs and helper scripts are in the ignored `.rt/gw1-fix/`.

Never used: the real system bus, the live session bus, PackageKit, apt, the network, any win11-* binary, gsettings, pkexec or sudo. No `#[ignore]` test ran outside the layer, and `probe.py --live` was never run.

### Files changed in the fix round

| File | Lines | Change |
|---|---|---|
| `tests/update/run.sh` | 501 | rewritten: allowlist root, socket scan and masks, inner socket checks, locked runs, strace on connect/sendto/sendmsg/sendmmsg by default, `--expect-calls`, `--meta`, the decoy-sandbox section, the outside decoy |
| `tests/update/bus.conf` | 36 | no eavesdrop; explicit receive rules; comment on the generated system-bus wrapper |
| `tests/update/fake_packagekit.py` | 3057 | own PackageKit connection; activation (`--service-dir`, `activate`, `Activate()`); Load transitions; Load-time clearing in the filter; plural hint; `destroyMs`; the bus rule; socket checks; `check_calls`; `decoy`; client steps `driver`, `state`, `plural`/`hints`, `detachOn`; 5 new checks; harmless stub argv; dead `--timeline` dropped |
| `tests/update/timelines/` | 32 files | new `daemon-idle-activatable`; changed `available-mixed`, `update-details`, `untrusted`, `daemon-absent`, `download-install` (callCounts 12, 15, 9, 2, 14) |
| `tools/update/probe.py` | 321 | presence check, activation, `packagekitd-not-available`, the sandboxed bus rule, shebang |
| `tests/update/test_enums.py` | 119 | shebang only |
| `tests/update/README.md` | 191 | rewritten for the above |

Unchanged: `stubs/stub.py`, `stubs/names.json`, `enums.json`, and `tests/gnome/harness.py` (still exactly the one STUBS line).

### Deviations added in the fix round (on top of 1–7 above)

8. The layer is an allowlist root, not GE0c's `--ro-bind / /`, and it masks host sockets (F1). This is stricter.
9. There are 32 timelines, not 31, and the private system bus has one generated service dir, where 9.1 says it has none (F4; it activates only the fake).
10. In private mode, `probe.py` uses the same presence and activation logic as `--live`, not NO_AUTO_START (F3).
11. `daemon.activatable` defaults to true in every timeline. A bus test that releases the daemon and calls without NO_AUTO_START gets a re-activated fake, as with the real packagekitd. For "not installed", use `daemon-absent`.
12. (F15) `run.sh` never refuses because of the caller's own environment, although 9.1 says it refuses when `XDG_RUNTIME_DIR` is `/run/user/<uid>` and so on. It drops that environment with `env -i` and builds the layer's environment from nothing. The guard then refuses those same variables inside the layer (guard cases 3–8 and 23). Refusing the caller's environment would make `run.sh` unusable from every normal shell, since each has `XDG_RUNTIME_DIR=/run/user/1000`. The protection is equivalent.
13. `untrusted` has no ErrorCode, and `download-install` keeps its finished transaction for 5 s (N2).
14. `run.sh exec` refuses a command outside W and /usr, because nothing else exists in the layer.

### For GW2b (changes to the interface above)

* The command line is unchanged; strace is now on by default. New: `--expect-calls N|FILE`, and `--no-strace`. The test binary must be inside the worktree (`W/target/...` is).
* The run also fails on a dead fake, on `State().violations`, or on a strace finding.
* `Load()` clears `Calls()` at the moment it is read. Connections that stay open across Load see Destroy, TransactionListChanged and PropertiesChanged, so one client process may now span several Loads.
* `SetDaemon(false)` then `SetDaemon(true)` gives a new unique name, and transactions vanish without Destroy, as when packagekitd exits.
* Every timeline except `daemon-absent` is activatable. `State()` has `pkUnique`, `activatable` and `activations`.
* Plural signals come only with `supports-plural-signals=true`. Whether GW2 sets that hint is GW2's choice (design 4.6 lists only `interactive`, `background` and `locale`), but GW2's bus tests must cover the path it chooses. `available-mixed` and `update-details` cover both.
* Exit 8 (need-untrusted) comes without ErrorCode 51, so map the exit code alone. `Destroy` can come seconds after `Finished` (`download-install`), so end a transaction on `Finished`.
* Seam rule for `paths.rs`: the F5 rule above, once the coordinator confirms it.

### Decisions for the coordinator

1. **F4**: accept the one generated service dir on the private system bus, and timeline 32 (`daemon-idle-activatable`). Correct design 9.1 and section 10: "no servicedir", 31 timelines, and "daemon-absent (NO_AUTO_START → ServiceUnknown)" (N1).
2. **F5**: record the seam rule in `contracts.md` before GW2b starts. GW8 writes the marker into GE0c's `RT`.
3. **F7**: decide whether GW2 sets `supports-plural-signals=true`. My suggestion is no: singular signals always work, and they are the only form without the hint.
4. **N3**: the GE0c owner should bring `harness.py`'s layer to the same allowlist root and socket masking before GW8.
5. **N2**: check the Destroy delay and the untrusted ErrorCode against the packagekit 1.3.4 source when someone has it.
6. Unchanged from the first round: the Transaction interface does have a `Sender` property (the 1.3.4 XML), which contradicts design 4.7.

### Left

* `probe.py --live` and the real `apt_pkg` call remain untested by design: that is R1, Robin's.
* GW2b's real bus tests and GW8's use of the fake inside GE0c's harness are still untested beyond the std-only Rust binary.
* A socket that a host process creates in W *after* a layer has started is not masked for that layer (the inner check runs at start). Only this repository's own tools write in W.
