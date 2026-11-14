# tests/update: fake PackageKit, fake NetworkManager and the test layer (GW1)

Design: `notes/claude/windows-update-gnome.md` sections 4.11 (test rules, binding by plan D-G25) and 9.1; the fix
round after the review is in `notes/claude/wu/gw1.md`. Robin's live session is this GNOME version, and polkit
treats a uid-1000 process outside a session as his active session. So nothing here may reach the real system bus:
everything runs inside a bubblewrap layer in which the real bus socket, the real home and the host's other unix
sockets do not exist.

Python 3 + `gi.repository.Gio` only, always `python3 -I`. Uses `/usr/bin/bwrap`, `/usr/bin/dbus-daemon`, `strace`
(required: the self-test fails without it) and the PackageKit interface XML that the `packagekit` package installs
in `/usr/share/dbus-1/interfaces/` (the fake registers exactly those interfaces, so signatures match packagekitd
1.3.4).

| File | What |
|---|---|
| `run.sh` | the layer and its entry points (`--self-test`, `exec`) |
| `bus.conf` | private dbus-daemon config: `unix:dir=/tmp/w11-update`, EXTERNAL, allow send/receive/own, no eavesdropping, no service dirs (the system bus adds exactly one, see below) |
| `fake_packagekit.py` | the fake (`serve`), its D-Bus activation helper (`activate`), the in-layer driver (`layer`), the reference client (`selftest`), `guard`, `check-stubs`, `decoy` |
| `timelines/*.json` | 32 timelines: the 31 of design 9.1 plus `daemon-idle-activatable` |
| `enums.json`, `test_enums.py` | pinned PackageKit/NM enum values; equal to the PackageKitGlib-1.0 and NM-1.0 typelibs (no D-Bus) |
| `stubs/stub.py`, `stubs/names.json` | the one stub program (its answers per argv come from the timeline) and the stub names (guard-bin on PATH, the update tool dir) |
| `stubs/bus-tests.json` | GW2b: the expectations of each Rust bus test for `run.sh bus-tests` (stub log, calls, timeout, datagrams) |
| `../../tools/update/probe.py` | R1 probe; without `--live` it accepts only a marked private bus inside a sandbox |

## Run it

```bash
tests/update/run.sh --self-test              # about 105 s (the flood timeline alone is 60 s); exit 0 = all green
tests/update/run.sh --self-test --only metered daemon-absent [--meta] [--no-strace]   # quick, selected timelines
tests/update/run.sh exec [--timeline NAME [--run]] [--expect-stubs F.json] [--expect-calls N|F.json] \
                         [--timeout S] [--allow-layer-dgrams] [--no-strace] -- CMD ARGS
tests/update/run.sh bus-tests [--only NAME ...] -- BIN   # GW2b: each Rust bus test in its own exec
```

No heavy lock is needed (no cargo, no gnome-shell; the layer is about 60 MB; the self-test's largest process is
about 160 MB while the strace logs are checked). Each run gets `tests/update/.run/<stamp>/` (0700, locked with
`flock` on `<run>/.lock` while any process of the run lives; the newest 6 are kept, a locked one is never deleted;
`.run/` ignores itself): `logs/fake.log`, `logs/bus-*.log`, `logs/socket-scan.txt`, `masked-sockets.txt`,
`bus-system.conf`, `services/`, `stub-calls.jsonl`, `calls.json`, `env.txt`, `result.json`, `selftest.json`,
`strace/*.tar.gz`.

## The layer (`run.sh`)

An allowlist, not `--ro-bind / /`: `bwrap --ro-bind /usr /usr` (+ the `/bin /sbin /lib /lib64` links)
`--ro-bind /etc /etc --ro-bind /sys /sys --dev /dev --proc /proc --tmpfs /tmp --tmpfs /run --dir /run/dbus --dir
/run/user --tmpfs /var --symlink ../run /var/run --dir /var/tmp --ro-bind <W> <W> --bind <run> <run>`, then
`--ro-bind /dev/null <path>` for every listening host socket that would still be visible (outside the layer,
`run.sh` reads `/proc/net/unix` and masks the ones under `/usr`, `/etc`, `/sys` or the worktree; the list goes to
`<run>/masked-sockets.txt`), `--remount-ro / --unshare-pid --unshare-net --unshare-ipc --die-with-parent
--new-session` (bwrap sets `no_new_privs`). So the real home (and its `~/.cache/ibus` bus), `/var/snap` (nordvpnd,
CUPS), `/var/lib` (libvirt), `/srv`, `/opt`, `/mnt` and `/media` do not exist inside. The environment is built
from nothing (`env -i`); the caller's own environment is never looked at:

| Variable | Value inside |
|---|---|
| `PATH` | `<run>/guard-bin:/usr/bin:/bin` (guard-bin: 97 stubs, a superset of GE0b's guard-bin and GE0c's `STUBS`) |
| `HOME`, `XDG_{CONFIG,DATA,CACHE,STATE}_HOME` | `<run>/home`, `<run>/config`, … |
| `XDG_RUNTIME_DIR` | `/tmp/w11-update` (the sandbox's own tmpfs, 0700; it holds the bus sockets and the marker) |
| `TMPDIR` | `/tmp/w11-update/tmp` |
| `DBUS_SYSTEM_BUS_ADDRESS` | the fake's private bus, `unix:path=/tmp/w11-update/dbus-XXXXXXXX,guid=…` |
| `DBUS_SESSION_BUS_ADDRESS` | a second private bus (`bus.conf` as it is) with a fake `org.freedesktop.Notifications` |
| `WIN11_UPDATE_PK_BUS` | equal to `DBUS_SYSTEM_BUS_ADDRESS` |
| `WIN11_UPDATE_FIXTURE_ROOT` | `<run>/fixture` (holds `.win11-update-fixture`) |
| `WIN11_UPDATE_TOOL_DIR` | `<run>/tools` (holds `.win11-update-fixture`; the 14 tool stubs of design 4.11) |
| `WIN11_FAKE_RUN_DIR` | `<run>` (not a `WIN11_UPDATE_*` name, so the service's seam check ignores it) |
| `WIN11_TEST_HOST_NETNS` | GW2b: `readlink /proc/self/ns/net` read by `run.sh` **outside** the layer; the Rust bus tests refuse to run when their own namespace equals it (review F10 of GW2a) |
| `NO_AT_BRIDGE=1 GTK_A11Y=none GSETTINGS_BACKEND=keyfile LANG=C.UTF-8` | no `DISPLAY`, no `WAYLAND_DISPLAY` |

Guard (in `run.sh`'s inner stage, then `fake_packagekit.py guard_problems`/`sandbox_problems`, again once the seams
are set): refuses root; a visible `/run/dbus/system_bus_socket`, `/run/user/<uid>`, `/run/systemd/private`,
`/run/snapd.socket`, `/run/reboot-required` or X11 socket; a masked path that is still a socket, or any socket in
the worktree (`host-socket-visible`), or no `masked-sockets.txt`; PID 1 not bwrap; any network interface but `lo`;
a writable real HOME; no `no_new_privs`; `XDG_RUNTIME_DIR` under `/run/user/`; any bus address (also escaped, also
second in a list) under `/run/user/` or `/run/dbus/`, abstract/tcp/`runtime=yes`, or a socket outside
`/tmp/w11-update`; `WIN11_UPDATE_*` set before the layer sets them; seams without marker; `DISPLAY`/`WAYLAND_DISPLAY`;
missing `NO_AT_BRIDGE`/`GTK_A11Y`/keyfile backend; PATH not starting with guard-bin or containing the real home;
session pointers (`XDG_SESSION_ID`, `AT_SPI_BUS_ADDRESS`, …); any other variable naming `/run/user/`; a service dir
holding anything but the fake's activation file.

**The bus rule** (`private_bus_problems`, also in `probe.py`; contract C10, which `paths.rs` enforces since GW2b):
a bus address is accepted only as one `unix:path=` (an optional `,guid=`) whose socket lies directly in
`$XDG_RUNTIME_DIR`, which is not under `/run`, is owned by the euid with mode 0700 and holds
`.win11-update-fixture`, and only while `/run/dbus/system_bus_socket` is not visible. The fixture and tool
directories of a run are created 0700 whatever the umask: the service refuses a group-writable seam directory.

**strace** (default for `exec` and the self-test): `connect`, `sendto`, `sendmsg`, `sendmmsg` of bwrap and every
process inside. Fails on any attempt at `/run/dbus/`, `/var/run/dbus/` or `/run/user/`, on a successful
`connect()` anywhere but `/tmp/w11-update/dbus-*`, and on any datagram sent with a unix address. One exception,
only with `exec --allow-layer-dgrams` (GW2b): datagrams to sockets under `/tmp/w11-update/tmp/`, the layer's own
tmpfs TMPDIR, where two Win11 hosts of one test share an events directory (`events.rs` fan-out). Nothing of the
host can be there; without the option such a datagram fails the run as before.

## For a Rust test binary (GW2b)

Build outside the layer (the layer's HOME is read-only), inside this worktree, then run only the binary inside:

```bash
tools/heavy.sh --tag gw2b-build -- tests/rust-sandbox/run-tests.sh --lib -- --list   # builds; lists, runs nothing
BIN=$(grep -o 'target/debug/deps/win11_host-[0-9a-f]*' tests/rust-sandbox/logs/test.log | tail -1)
tools/heavy.sh --min-mb 1024 --tag gw2b-bus -- tests/update/run.sh bus-tests -- "$BIN"        # every bus test
tests/update/run.sh exec [--expect-stubs F.json] -- "$BIN" services::update::bus_tests::NAME --exact --ignored --test-threads=1
```

With `WIN11_CARGO_TARGET_DIR` on another disk (the build rules of the WebKit removal, 13-11) the binary lies outside
the worktree: copy it into `tests/update/.run/` (git-ignored) first and pass that copy, e.g.
`install -D "$BIN" tests/update/.run/bin/win11_host && BIN=tests/update/.run/bin/win11_host`; the same for the
`win11-launch` that `test_post_upgrade.py run --launch` takes. `test_compat.sh` reads the program binaries from
`W11_TEST_BUILD_DIR` (default `tests/build/native-release`, all of scripts/install.py `BINS`; also inside the
worktree), as do the unit tests `elf.rs our_binaries_equal_readelf` and `post_upgrade.rs
missing_parts_come_from_the_soname_directory`; after that directory is rebuilt, `tests/update/test_compat.sh
--record-readelf` re-records `fixtures/compat/readelf-needed.txt`.

(`run-tests.sh --lib --no-run` no longer works: since the round-2 hang guard, `run-tests.sh` passes `--no-run`
itself, and cargo refuses it twice.) `bus-tests` runs every test of `stubs/bus-tests.json` in its own `exec` with
`--expect-stubs` (the whole stub log in order; a matcher may carry `"count": N` for N consecutive calls, and a
`"group"` of matchers stands for one round of calls, `"count"` rounds of them),
`--expect-calls` (the fake's calls since the test's last `Load()`), `--timeout` and, for the two tests whose hosts
share an events directory, `--allow-layer-dgrams`. Each test also checks every one of its scenarios itself.
Before the first test, `BIN services::update::bus_tests:: --list --ignored` runs inside the layer: its bus tests
must be exactly the JSON's (with `--only`: must include each name), and each test's run must end with
`test result: ok. 1 passed` (a misspelt or removed test runs 0 tests and fails; GW2b fix round, review m3).

* `--test-threads=1`: there is one fake; every test starts with `Load(<timeline>)` (which resets everything,
  including `CallCount()`), then usually `Run()`.
* The binary must be inside the worktree (or `/usr`): nothing else exists in the layer (`run.sh exec` refuses).
  It gets the environment above, `stdin=/dev/null`, cwd = the repository root, and can write only in `<run>`
  (HOME, XDG dirs, fixture root) and `/tmp/w11-update`.
* `--expect-stubs FILE`: a JSON list of expected stub calls in order: `"tool"` or
  `{"tool", "dir": "tools"|"guard-bin", "argv": [...], "argvPrefix": [...], "count": N}` (`count`, GW2b: N
  consecutive calls of that matcher, default 1), or `{"group": [matchers], "count": N}` (D-G73: N rounds of the
  group's calls in order, such as the phasing classifier's three programs per GetUpdates). Without it, any stub
  call fails the run. `python3 -I fake_packagekit.py check-stubs --log <run>/stub-calls.jsonl --expect F --start
  N` does the same check from a driver.
* `--expect-calls N|FILE`: the fake's calls since the last `Load()` at the end of the run: exactly `N`, or a JSON
  list in order of `"Method"` / `{"method", "path"?, "to"?: "pk"|"nm"|"notify", "noAutoStart"?, "args"?,
  "argsPrefix"?}`. Per-test counts belong in the test itself (`CallCount()` after its own `Load()`).
* The run also fails when the fake is unreachable or dead at the end, when `State().violations` is not empty, or
  when the strace check fails. `<run>/calls.json` holds every call the fake received since the last `Load()`.

## Control interface

Name and interface `io.github.win11shell.FakePackageKit`, path `/Control`, on `WIN11_UPDATE_PK_BUS`:

| Method | |
|---|---|
| `Load(s)` | reset everything (transactions, calls, updates, history, stub responses, fixtures, activations) and load `timelines/<s>.json`; `""` = defaults. Paused afterwards. `Calls()` is cleared when the Load message is read, so a call counts exactly when it is handled under the new timeline. A client that stays connected sees `Destroy` for each transaction that goes away (plus `TransactionListChanged` if one was active) and `PropertiesChanged` for every manager / NM property that changes; a daemon or NM that appears or disappears shows as `NameOwnerChanged` |
| `Step() → s` | paused: play one scripted event (`"<stream> <event>"`), or release the first `{"until": "step"}` gate; `"idle"` / `"blocked <stream>:<gate>"` when nothing can play. In Run mode it only releases a step gate (else `"auto"`) |
| `Run()` | play everything at the scripted pace, also scripts queued later, until the next `Load` |
| `SetDaemon(b)` | `true`: own `org.freedesktop.PackageKit` on a new connection (a new unique name, like a new packagekitd); `false`: close that connection (the name and the unique name vanish, all transactions are gone without `Destroy`: packagekitd exited) |
| `Calls() → a(ssss)` | `(method, path, sender, args JSON)` of every call that reached the fake's PackageKit, NetworkManager or Notifications objects since `Load`, in arrival order; `Properties.Get`/`GetAll`/`Introspectable.Introspect` included (method `Properties.GetAll` etc.) |
| `CallCount() → u` | `len(Calls())` |
| `State() → s` | extra: JSON `{timeline, mode, daemon, nm, pkUnique, activatable, activations, updates, violations, transactions, streams}`; `violations` lists forbidden ids sent to `UpdatePackages` and failed events |
| `CallsJson() → s` | extra: `Calls()` with `interface`, `member`, `destination`, `noAutoStart`, `noReplyExpected`, `bus`, `to` |
| `Activate()` | extra, called by `fake_packagekit.py activate` only: `SetDaemon(true)` and count one D-Bus activation |

Calls rejected by the bus (no owner: `NameHasNoOwner` with NO_AUTO_START, `ServiceUnknown` without when nothing is
activatable) never reach the fake and are not counted.

## What the fake does

* **Three connections** on the private system bus, as three processes in reality: the control name, NetworkManager,
  and PackageKit (opened and closed by `SetDaemon`).
* **D-Bus activation** (the "installed but idle" packagekitd): the system bus uses `<run>/bus-system.conf` =
  `<include>` of `bus.conf` + one `<servicedir>` `<run>/services`. With `"daemon": {"activatable": true}` (the
  default) that directory holds `org.freedesktop.PackageKit.service` (`Exec=/usr/bin/python3 -I -B …/fake_packagekit.py
  activate`), so a call without NO_AUTO_START while the daemon is absent makes the bus start `activate`, which asks
  the fake to own the name; the call is then delivered. `"activatable": false` (`daemon-absent`) empties it (the
  fake calls `ReloadConfig`). The private session bus can activate nothing.
* **Manager** (`/org/freedesktop/PackageKit`): `CreateTransaction`, `GetTransactionList` (role set, not finished),
  `StateHasChanged` (→ `UpdatesChanged` after 100 ms), `CanAuthorize` (2), `GetTimeSinceAction`, `GetDaemonState`;
  properties as packagekitd 1.3.4 (`BackendName` apt, `Locked`, `NetworkState`, …); signals `TransactionListChanged`
  (when a role is set and when a transaction finishes), `UpdatesChanged`, `RestartSchedule`.
* **Transactions** (`/<n>_<8 hex>`): `SetHints`, `GetUpdates`, `GetUpdateDetail`, `GetDetails`, `RefreshCache`,
  `UpdatePackages` (simulate when flags contain 4), `RepairSystem`, `GetOldTransactions`, `Cancel`; all 13
  properties with `PropertiesChanged`; `Uid` from `GetConnectionUnixUser`, `Sender` = caller. Errors use
  packagekitd's names: `…Transaction.CannotCancel` (AllowCancel false), `RefusedByPolicy` (other uid),
  `NotRunning`, `TransactionExistsWithRole`, `PackageIdInvalid`, `NotSupported`. When the caller's unique name
  vanishes: `CallerActive` false, and with `"onVanish": "cancel"` the fake cancels a cancellable transaction.
  Finished → `Destroy` after `destroyMs` (default 50 ms; packagekitd keeps a finished transaction for some seconds,
  which `download-install` models with 5000 ms) → object removed; finished transactions go into the history for
  `GetOldTransactions` (newest first).
* **Plural signals only with the hint**: `Packages` and `UpdateDetails` are sent only to a transaction whose hints
  contain `supports-plural-signals=true` (as packagekitd 1.3.4 documents); otherwise the same content goes out as
  `Package` / `UpdateDetail` signals. With the hint a singular signal can still come.
* **NetworkManager** (own connection, so its own unique name): `Connectivity`, `Metered`, `State`, … with
  `PropertiesChanged`; `CheckConnectivity`, `state`. `"nm": null` in a timeline = not on the bus.
* **Notifications** on the private session bus: `Notify`, `CloseNotification`, `GetCapabilities`,
  `GetServerInformation` (counted in `Calls()` as well).
* Every signal is emitted with destination None (broadcast), like packagekitd.

## Timelines

Top-level keys: `name`, `group`, `title`, `description`, `states` (GW8 screen ids it serves), `daemon`
(`present`, `activatable`, `locked`, `networkState`, `distroId`, `timeSinceAction`), `nm` (initial properties or
null), `onVanish`, `forbiddenIds`, `stubs` (`{tool: {rc, stdout, stderr}}`, or per tool a list of such responses
each with `argv` or `argvPrefix`, see below; written to `<tools>/.responses/` on Load), `fixtures` (`{relative
path: text|null}` written into the fixture root on Load), `batch` (GetUpdates as one `packages` event), `destroyMs`,
`packages` (`{package id: {info, summary, size, downloadSize?, row}}`), `updates` (what GetUpdates lists now),
`updateDetails`, `history`, `methods` (`{method key: script}` or
`{"each": [script, …]}` per call; keys `RefreshCache`, `GetUpdates`, `GetDetails`, `GetUpdateDetail`,
`UpdatePackages:simulate`, `UpdatePackages`, `RepairSystem`, `GetOldTransactions`, `Cancel`, `StateHasChanged`;
missing keys use built-in defaults), `script` (global events queued at Load), `client` (the reference client's
flow) and `expect` (`callCount`, `stubs`, `violations`).

Events (one action each, optional `ms` delay in Run mode, default 20): `props`, `package`, `packages`, `item`,
`restart`, `error`, `details`, `updateDetail(s)`, `transaction`, `finish` (ends the script; Destroy follows;
`destroyMs`), `until` (`call:<Method>`, `vanish`, `step`), `setUpdates`, `removeUpdates`, `manager`,
`managerProps`, `nm`, `daemon`, `fixture`, `foreign` (a transaction of another uid with its own script), `flood`,
`note`, and the generators `emitUpdates`, `emitDetails`, `emitUpdateDetails`, `emitSimulate` (`extra`;
`downloadSize`: the simulate's `DownloadSizeRemaining` instead of the sizes' sum, e.g. 0 as the apt backend may
report; `phased-as-normal` uses it), `emitInstall` (`failAt`, `error`, `keep`, `restart`, `destroyMs`),
`emitHistory`. Enum values may be numbers or the nicks of
`enums.json` (PackageKit's own strings, e.g. `failed-due-to-running-process` for error 60).

Reference-client steps: `tx` (with `args`, `plural`, `hints`, `cancelWhen`, `detachWhen`, `detachOn`, `save`,
`saveTid`), `manager`, `getAll`/`get` (with `flags: "no-auto-start"`), `driver` (`NameHasOwner`,
`ListActivatableNames`, `GetNameOwner`, `ListNames`; never counted), `state` (compare `State()` fields), `control`,
`phasing`, `connect`/`disconnect`, `attach`, `waitSignal`, `waitObserver`, `observerExpect`, `fixtureExists`,
`sleep`.

A tool stub's response is one object `{rc, stdout, stderr}` (every key optional; `rc` 0, empty texts) that answers
every argv, or a list of them in which each entry has exactly one of `argv` (the whole argv) and `argvPrefix` (its
first arguments): `stubs/stub.py` answers with the first entry that matches, and an argv that no entry matches
exits 97 with a message, like a tool without a response. Load refuses any other shape. The stubs never read
stdin, and the stub log has no `stdin` field (D-G73: every program the service starts takes its input as argv).

The phasing classifier (`host/src/services/update/phasing.rs`, D-G73) runs three programs per GetUpdates that
listed packages, and the reference client's `phasing` step runs the same three through the tool stubs (and checks
that apt's candidate is the offered version), in this order: `apt-cache policy ?phasing`, `apt-cache policy -o
APT::Cmd::Pattern-Only=true <name:arch>…` and `apt-mark showhold`. Every timeline with packages answers them in apt's C-locale format
(`tests/update/fixtures/phasing/recorded/policy.txt` shows the real one):
* `apt-cache`: first `{"argv": ["policy", "?phasing"]}`, the blocks of the packages phasing holds back on this
  machine (empty when none is); then `{"argvPrefix": ["policy"]}`, a block for every package of the timeline (the
  classifier reads only the ones it asked for). A block is `<name>:` (`<name>:<arch>:` for a foreign
  architecture), `  Installed: <version>|(none)`, `  Candidate: <version>|(none)`, `  Version table:`, then per
  version ` *** <version> <priority>` (the installed one) or five spaces and `<version> <priority>`, with
  ` (phased N%)` on a version whose Phased-Update-Percentage is below 100, each followed by its source lines
  (indented further). No timeline has an `apt-cache show` entry: none needs the fallback.
* `apt-mark`: `{"argv": ["showhold"]}`, one apt name per held package (empty in every timeline).

These responses replaced the one answer of the script classifier before D-G73 (`git show
last-webkit:tests/update/timelines/<name>.json`) and keep every package's candidate, phased flag, percentage, hold
and installed version. `available-mixed` (2 phased), `only-phased` (2) and `phased-as-normal` (1) are the
timelines with phased packages; their `expect.stubs` are the three calls of their client's `phasing` step.
