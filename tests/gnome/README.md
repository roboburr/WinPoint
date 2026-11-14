# tests/gnome: private headless GNOME Shell harness (GE0c, scenarios GE1h)

`harness.py` starts its own GNOME Shell 50 (headless, a 1440×900 virtual monitor), with a private install of a
build directory (`--build-dir`, default `tests/build/native-release`) and an unmodified copy of `extension/`. It
waits for the native shell (`win11-shell`, D-G70: the taskbar, its flyouts, Start and the desktop in one GTK 4
process, no WebKit) to report its boot, opens each Win11 surface (the shell's windows and the native GTK 4 apps,
`win11-gtk-<kind>`; nothing in WinPoint runs WebKit any more), captures its widget state and text from its dev
channel and takes screenshots. It can also switch the language live, and it runs scenario files
(`--scenario`, section "Scenarios") that later steps write for their own checks. Robin's live session runs this
same GNOME version, so the harness is built so that it cannot reach that session at all.

Python 3 standard library plus `gi` (Gio, GLib, GdkPixbuf; Gtk 4 for the picker scenario's helper app). It also
uses `bwrap`, `dbus-run-session`, `dbus-daemon`, `strace` and `gnome-shell` (and `xdg-desktop-portal` in
`r2d-picker.py`), all from the system.

## Run it

From the repository root (G):

```bash
# 1. Guard self-test: starts no GUI process (only python, bwrap and a heavy.sh/flock probe on a scratch lock
#    file); writes baseline/guard-selftest.json. About 8 s.
python3 -B tests/gnome/harness.py guard-selftest

# 2. One private session (about 2.5-4 min and 1.1-1.3 GB peak PSS). Always through tools/heavy.sh (contracts C8).
tools/heavy.sh --min-mb 2560 --tag <step> -- python3 -B tests/gnome/harness.py launch run --lang nl --switch
tools/heavy.sh --min-mb 2560 --tag <step> -- python3 -B tests/gnome/harness.py launch run --lang en --out tests/gnome/<dir>

# 3. A scenario (section "Scenarios" below); results in tests/gnome/scenarios/native-shell.out/.
tools/heavy.sh --min-mb 2560 --tag <step> -- python3 -B tests/gnome/harness.py launch run --lang nl \
    --source worktree --scenario tests/gnome/scenarios/native-shell.py

# 4. The same under strace (section "strace"): the run fails when a connect() reached a socket outside the sandbox
#    or a real pkexec, check-new-release, apt-get, apt-cache, do-release-upgrade or loginctl was executed.
tools/heavy.sh --min-mb 2560 --tag <step> -- python3 -B tests/gnome/harness.py launch run --lang nl \
    --scenario tests/gnome/scenarios/native-shell.py --out tests/gnome/scenarios/native-shell.out \
    --strace tests/gnome/scenarios/native-shell.out/strace.txt
python3 -B tests/gnome/harness.py strace-check tests/gnome/scenarios/native-shell.out/strace.txt   # the check again
```

* **Heavy lock.** `launch run` needs `.heavy.lock` held by an ancestor: `tools/heavy.sh` (recognised by its
  script path plus an open fd on the lock file) or `flock -w 3600 .heavy.lock …` (GE0c's form). The lock must
  really be held at that moment. Without one it refuses (exit 4) and prints the heavy.sh command;
  `launch --own-lock run …` lets it take the lock itself (waits up to 3600 s), as GE0c did by default.
  `WIN11_HEAVY_LOCK` overrides the lock path, as in `tools/heavy.sh` (the self-test uses that).
* `run` refuses to start if MemAvailable is below 2560 MB (C8). The sampler stops the private session if
  MemAvailable drops below 1.5 GB (earlyoom here kills at 8 %); heavy.sh's watchdog does the same from outside.
* `--build-dir DIR` picks the 19 binaries to install (`scripts/install.py` `BINS`): the host's (`win11-panel`,
  `win11-app`, `win11-explorer`, the workers, the file portal, ...) plus, from the native workspace (`native/`, its
  own target dir), `win11-shell`, `win11-calendar` and the seven apps `win11-gtk-{explorer,settings,taskmgr,alles,
  disks,defaultapps,installer}`. Default `tests/build/native-release` (gitignored, a local copy with a
  `SHA256SUMS`). It must be inside G and hold all 19 executables. An older build dir may still hold `win11-start`
  and `win11-desktop`; they are not installed (`BINS` no longer names them).
* `--source worktree|<rev>` picks the tree whose `scripts/install.py`, `vendor/ui` assets, icons and `extension/`
  are installed. `worktree` (default) is G as it is now, including other steps' unfinished edits.
  A revision (`3b898e6`, `HEAD`, a branch) is exported with `git archive` into `run/src` (about 84 MB, read-only
  for the repository), so a run is not disturbed by concurrent edits. The Settings page titles
  (`native/apps/settings/src/registry.rs`) and the seeded `settings-defaults.json` come from the same tree.
* `--surfaces desktop,panel,start,explorer,settings,settings-<page>,taskmgr,alles,disks` limits the built-in pass.
  `desktop`, `panel` and `start` are the native shell's windows, the others the native apps: each a screenshot
  plus its dev state (labels, Dutch-word hits and, for an app, its language; `{"native":"state"}`).
* `--out DIR` writes the screenshots and `results.json` somewhere else. It must be inside `tests/gnome/` and
  outside `run/`; it is bound writable into the sandbox, next to `run/` (the only two writable places). Without
  `--out` the built-in pass writes `baseline/<lang>/` (so it overwrites the committed baseline; `--switch` puts its
  shots in `baseline/switch/`), a scenario writes `<scenario dir>/<stem>.out/`. With `--out`, the `--switch` shots go to
  `<out>/switch/`.
* `--strace FILE` (inside the output directory) runs the whole session under
  `strace -f --seccomp-bpf -e trace=connect,execve,execveat` (section "strace").
* `--seed minimal` seeds `settings.json` with only `{"language": …}`. This reproduces the partial-settings
  finding in `findings/`.
* `--patch-extension` is a fallback for a shell without `--unsafe-mode`. It patches only the private extension
  copy, the way Codex did. It is not needed on GNOME 50.1.
* `python3 -B tests/gnome/harness.py launch probe` shows what the guard sees inside the sandbox.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Every surface was captured. |
| 1 | A surface or the install check failed (see `results.json`). |
| 2 | The guard refused. |
| 3 | Not enough memory. |
| 4 | No heavy lock. |
| 5 | Aborted: low memory, or a socket file appeared outside `run/` during the run (socket watch). |

## What it guards

There are four layers. Each layer works without the others.

1. **Sandbox (`launch`).** `bwrap` is the only process started with the caller's environment, and it gets a
   complete replacement environment (`private_env()`). The root is an allowlist (D-G42, D-G50; GW1's layer of
   `tests/update/run.sh`), not `--ro-bind / /`:
   * `/` is a read-only bind of the empty directory `run/root` on the checkout's own file system (so the storage
     service sees a system drive at `/`; harness drive screenshots therefore show only that drive). Read-only on
     top: `/usr` with the `/bin`, `/sbin`, `/lib`… links, `/etc`, `/sys`, the system font cache, udev's database
     (`/run/udev`, which mutter needs to find the GPU) and the checkout G. Writable: only `run/` and the run's
     output directory (`--out`). Robin's real home (beyond the path to G), `/var/lib`, `/var/snap`, `/srv`, `/opt`,
     `/mnt` and `/media` do not exist inside, so `~/.local/bin/win11-*`, `~/.local/share/gnome-shell/extensions`,
     `~/.config/win11-gnome` and `~/.config/dconf` can be neither read nor changed.
   * `/tmp` (with `XDG_RUNTIME_DIR=/tmp/w11-rt`: short socket paths, and no runtime socket is ever left in the
     checkout), `/run` (with empty `dbus`, `user` and `systemd` directories) and `/var` (`/var/run` → `/run`,
     `/var/tmp`) are private tmpfs mounts. The live session bus, the Wayland and X11 sockets, PipeWire, at-spi and
     the system-bus socket are not there.
   * **Host sockets are masked.** A read-only bind does not stop `connect()`. `launch` reads the host's
     `/proc/net/unix` (outside the sandbox) and covers every bound socket the binds would still show (in `/usr`,
     `/etc`, `/sys`, `/run/udev`, the font cache, G) with `/dev/null`; the guard refuses a run when one of them is
     still a socket inside, or when any other socket file sits in the checkout or a bound host directory
     [`host-socket-visible`].
   * `/dev` holds only the GPU nodes (`/dev/dri`, `/dev/nvidia*`).
   * It has its own PID namespace. Nothing outside can be signalled, and every process dies with the sandbox
     (`--die-with-parent`).
   * It has its own network namespace with loopback only. Nothing can be downloaded, and the live session's
     abstract sockets cannot be reached.
   * **Socket watch.** During the run a thread scans the checkout and the bound host directories for socket files
     every 2 s; one that appears outside `run/` stops the private session (exit 5). At the end the run removes the
     socket files its session left in `run/` (an ibus or D-Bus socket) and fails when a socket file is still in
     the checkout.
   * **Update seams (D-G44).** Every process gets `GIO_USE_VFS=local` (no GVfs client contacts a session bus) and
     the update service's private test seams: `WIN11_UPDATE_TOOL_DIR=run/update-tools` (a stub for every program
     of the service's tool table; each only logs to `logs/update-tool-calls.log` and fails) and
     `WIN11_UPDATE_FIXTURE_ROOT=run/update-fixture` (the real `/proc`, update-notifier's compiled schema, nothing
     else: no apt history, no `reboot-required`, no dpkg state). So no Settings of a harness run reads the real
     `/var/log/apt`, runs `/usr/bin/snap` or `apt-config`, or reaches check-new-release or pkexec.
2. **Guard (`run`, `_inner`).** These refuse to start when any of the following holds (codes in brackets):
   * HOME is the real home [`home-real`].
   * `XDG_RUNTIME_DIR` is `/run/user/<uid>` [`runtime-live`].
   * The session bus address points into `/run/user/<uid>`, including escaped forms and the second entry of a
     list [`session-bus-live`].
   * `DISPLAY` or `WAYLAND_DISPLAY` is set, even when empty [`display-set`, `wayland-display-set`].
   * `GSETTINGS_BACKEND` is not `keyfile` [`gsettings-not-keyfile`].
   * `GIO_USE_VFS` is not `local` [`gio-vfs-not-local`]; `WIN11_UPDATE_TOOL_DIR` or `WIN11_UPDATE_FIXTURE_ROOT` is
     not the private one [`update-seam-not-private`], or another `WIN11_UPDATE_*` variable is set
     [`update-seam-extra`].
   * Stricter checks:
     * `NO_AT_BRIDGE=1` and `GTK_A11Y=none` must be set (any other value, empty included, is refused)
       [`at-bridge`, `gtk-a11y`].
     * Every XDG directory, `TMPDIR` and `WIN11_INSTALL_PREFIX` must be the private path.
     * No `XDG_DATA_DIRS` entry may be in `~/.local/share` (the live extension, entries and payload)
       [`xdg-data-dirs-home`].
     * PATH must not contain `~/.local/bin` or any other entry in the real home.
     * The system bus must be the private one.
     * plan-critique F24, each with its own code, refused even when set to an empty or private-looking value:
       `WIN11_REFERENCE_SETTINGS` [`reference-settings-set`], `WIN11_REFERENCE_INSTANCE`
       [`reference-instance-set`], `WIN11_GNOME_CONFIG_DIR` [`gnome-config-dir-set`], `AT_SPI_BUS_ADDRESS`
       [`at-spi-bus-set`], `XDG_SESSION_ID` [`xdg-session-id-set`].
     * None of `GNOME_SHELL_SESSION_MODE`, `SSH_AUTH_SOCK`, `XDG_SEAT`, `XDG_VTNR`, `GNOME_SETUP_DISPLAY`, … may
       be set [`live-pointer-var`].
     * `WIN11_BUILD_DIR` must be inside G and not in `~/.local` [`build-dir-not-repo`], and equal `--build-dir`.
     * The extension copy that is installed, patched (`--patch-extension`) and loaded by the shell (its `path`
       through Eval) must be `run/data/gnome-shell/extensions/<uuid>` [`extension-copy-not-private`].
     * In the inner stage, the session bus must be exactly `/tmp/w11-rt/bus`.
   * The sandbox checks fail outside bwrap: the home is writable, the live sockets are visible, the network is
     not loopback-only, PID 1 is not bwrap, `/` is writable, `/run`'s `dbus`/`user`/`systemd` directories are not
     empty, the real `~/.config`, `~/.local`, `~/.cache`, `/var/lib` or `/var/snap` is visible
     [`not-sandboxed-*`], or a host socket is not masked [`host-socket-visible`].

   `guard-selftest` proves each refusal at three levels: in-process, as a real process outside the sandbox
   (including the caller's own live environment), and inside the real sandbox with each variable changed. It
   also proves the heavy-lock decision (no ancestor: refused; a `flock` or `tools/heavy.sh` ancestor holding the
   lock: accepted; a `flock` ancestor on another file: refused; `--own-lock`: taken) with a scratch lock file in
   `tests/logs/`, so the real `.heavy.lock` is never taken or waited for. It also checks the host-socket masking
   with a decoy socket, the socket watch, and the two strace checks on synthetic logs (section "strace"). 110
   cases at GE1h (56 at GE0c), 135 at round 3, 136 with WU-FIX's execve case.
3. **Private buses.** The session bus comes from `dbus-run-session` with `run/session-bus.conf`, and the system
   bus is a second `dbus-daemon` with `run/system-bus.conf`. Neither has a `<servicedir>`, so nothing is ever
   auto-activated. That means no PackageKit, no logind, no polkit, no NetworkManager, no portals and no GDM.
   Both have AppArmor mediation off, because `/sys` is read-only in the sandbox.
4. **Stubs.** `run/stubs/` comes first on PATH. It blocks and logs (to `logs/guard-calls.log`): `apt`,
   `apt-get`, `apt-cache`, `pkcon`, `do-release-upgrade`, `check-new-release`, `update-manager`, `systemctl`,
   `systemd-run`, `loginctl`, `pkexec`, `sudo`, `gsettings`, `dconf`, `gnome-extensions`, `xdg-open`,
   `xdg-terminal-exec`, `screen`, `wpctl`, `pactl`, `pw-dump`, `nmcli`, `udisksctl` and a few more. The baseline
   runs called `wpctl`/`pw-dump` (panel volume), `systemctl --user list-units` (Task Manager) and
   `apt list --upgradable` (Update page). The update service never searches PATH: its tool table points at
   `run/update-tools/` (layer 1, update seams).

## strace

`--strace FILE` puts the session (`FILE`) under `strace -f -qq --seccomp-bpf -e trace=connect,execve,execveat`; the
install has its own `logs/install.strace` (`trace=%file,connect`). After the session two checks read both logs, and
either one fails the run (runs before the WebKit removal also had `FILE.warm` and `FILE.warm2`, the GStreamer
warm-up's):

* **connect** (D-G42, `strace_connect_report`, `FILE.json`, `results.json` `straceConnect`): a `connect()` that
  succeeded (or whose outcome strace did not see) may only reach a socket of the sandbox itself: under `run/` or in
  its own tmpfs mounts (`/tmp`, `/run`, `/var`, but not the host directories bound into them), an abstract name (the
  sandbox's own network namespace) or loopback. Paths are folded as the kernel resolves them inside (`..`, `.`, `//`,
  `/var/run`, the top-level links); a relative path counts as outside.
* **execve** (WU-FIX, D-G51; `strace_exec_report`, `FILE.exec.json`, `results.json` `straceExec`): no `pkexec`,
  `check-new-release`, `apt-get`, `apt-cache`, `do-release-upgrade`, `loginctl`, `apt` or `pkcon` may be executed
  from anywhere but this run's own stub directories under `run/` (`run/stubs`, `run/update-tools`). A failed
  attempt (ENOENT while PATH is searched) ran nothing and does not count; paths are folded as above, and a relative
  path counts as outside.

`python3 -B tests/gnome/harness.py strace-check FILE [FILE …]` runs both checks again on saved logs. Run the
scenarios that touch the update page (or anything that could reach a root helper) with `--strace`.

Other facts about a run:

* **Install.** `scripts/install.py` runs under `strace -f -e trace=%file`. Every mutating syscall path must be
  inside `tests/gnome/run/`. The 19 binaries (`install.py` `BINS`, with `win11-shell`, `win11-calendar` and the
  seven `win11-gtk-*` apps) must
  match the build dir byte for byte, the extension copy must match `extension/` byte for byte, there must be 9
  desktop entries that point into the prefix (no Start and no Desktop entry; Calendar), and no `win11-start` or
  `win11-desktop` may be installed (`logs/install-verify.json`).
* **Shell.** The shell runs in GNOME's `user` session mode, not `ubuntu`. So no Ubuntu extension loads, and in
  particular not DING from `/usr/share`, whose stale-process matcher could otherwise hit Robin's DING. It runs
  with `--unsafe-mode` for Eval and Screenshot, so the extension copy is not patched. The keyfile turns off
  idle, lock and the welcome dialog.
* **Shutdown.** It stops only its own descendants: launched apps, then the shell's clients, then
  `gnome-shell`, then the system bus, then a final sweep. The PID namespace makes anything else impossible.

## Layout

All state lives in `tests/gnome/run/`, which is rebuilt at the start of every run and ignored through
`.git/info/exclude`. It takes about 750 MB, mostly the icon tree and the 19 binaries. `XDG_RUNTIME_DIR`
(`bus`, `sbus`, the Wayland socket `w11-wl`, `win11-gnome/` with the hosts' event sockets) is `/tmp/w11-rt` inside
the sandbox's own `/tmp`, so it ends with the sandbox.
`rm -rf tests/gnome/run` is safe at any time when no run is active.

| Path in `run/` | What |
|---|---|
| `root/` | The empty directory bound read-only as the sandbox's `/` (never emptied by a run) |
| `update-tools/`, `update-fixture/` | The update service's test seams (layer 1): its stubs and its fixture root |
| `home/`, `config/`, `data/`, `cache/`, `tmp/` | HOME, XDG and `TMPDIR`: fixtures, keyfile GSettings, `win11-gnome/settings.json` |
| `prefix/` | `WIN11_INSTALL_PREFIX`: `bin/` (19 binaries) and `share/win11-gnome/` |
| `src/` | Only with `--source <rev>`: the `git archive` of that commit, the tree that is installed |
| `logs/` | `harness.log`, `shell.log`, `app-*.log`, `install.log`, `install.strace`, `install-verify.json`, `memory.tsv` (totals every 0.5 s; `results.json` `memory.seenByName` has every process name with its highest PSS), `command.json` and `report.jsonl` (the dev channel), `guard-calls.log`, `inner-results.json`, `scenario-results.json` (written after every scenario step) |
| `shots/` | Raw screenshots |

Committed results:

| Path | What |
|---|---|
| `baseline/nl/`, `baseline/en/` | PNGs plus `results.json`: per surface its language, window title, its labels, Dutch-word hits, pixel statistics, windows, memory, install check and stub calls. `logs/` holds copies of the main logs. (The committed ones are from the WebKit apps: their `lang` attribute and `innerText`; history.) |
| `baseline/switch/` | The live nl→en switch with Settings (Taal en regio) and Start open. |
| `baseline/native-nl/`, `baseline/native-en/` | Not committed: the folders a run with `--out baseline/native-nl` (or `-en`) writes. The built-in pass with `--switch` on the native apps: the same per-surface facts from the dev state, the switch timings (`switch-after`: Settings' language, its page title, the shell's labels). |
| `baseline/guard-selftest.json` | The guard self-test result. |
| `findings/minimal-seed-live-switch/` | The same switch with a partial `settings.json`: theme and pins are lost. |
| `lib/scenario.py` | The scenario helper (`h`), loaded by path (GE1h) |
| `scenarios/native-shell.py`, `scenarios/native-shell.out/` | The native shell smoke and its last results (D-G63, D-G70) |
| `scenarios/native-gtk-apps.py`, `native-restart.py`, `taskbar-app-icons.py`, `kit-ui.py` | The native GTK 4 apps (13-11): launch, memory, a short use of each app through the dev channel (Disks with a UDisks2 stand-in on the private system bus) and Task Manager's Processen rows; `shell.restartAll` re-execs every native program (Explorer with two windows, Settings back on its update page); each app's icon on the taskbar, in GNOME's window list, in the catalogue and in Start (All apps, search), each own app listed once; the kit UI fixes (Explorer's column headers, one colour under Alles's command bar, manual font rendering under cairo) |
| `scenarios/r2d-picker.py`, `scenarios/picker-helper.py` | Win11's file picker as other apps reach it (13-11): the scenario starts `win11-file-portal` and `xdg-desktop-portal` itself (no activation on the private bus), then the native Settings (whose kit sets `GDK_DEBUG=portals` for itself since 14-11) and a GTK 4 helper app (`org.example.PickerTest`, started with it) open GTK's file dialog (unsandboxed GTK 4 shows its own chooser without it); the picker's identity (`io.github.win11shell.FilePicker`, the portal's `win11-gtk-explorer` child, transient for the caller, modal or not, skip-taskbar, also the non-modal one: the extension hides a picker with a parent from the window list), the taskbar, and the cancel on close |
| `scenarios/readme-shots.py`, `scenarios/readme-shots.out/` | The README screenshots, run with `--lang en --out tests/gnome/scenarios/readme-shots.out` (English, dark and light, the private 1440×900 monitor; `readme-shots.nl.out/` is a Dutch run). `docs/screenshots/` holds copies saved as RGB without metadata; `notifications.png` and `quick-settings.png` are the bottom-right 760×840 of their shots and `light.png` is `light-start.png`. Re-shot on 14-11 after the WebKit apps went |
| `scenarios/<step>-*.py`, `scenarios/<step>-*.out/` | Later steps' scenarios and results (owned by those steps) |

The `baseline/` results and the scenario outputs from before D-G70 show the WebKit panel, Start and desktop; they
are history and nothing compares against them. The scenarios that tested only those surfaces (boot-only,
start-hover, power-handoff, desktop-selfheal, ge1a-switch, ge1a-start-switch, ge1b-core, ge1b-perf, cross-r1,
r5-fullscreen, leak-soak/-events/-churn, example) and the restart scenarios that assumed restartable panel, Start
and desktop hosts (gx1-restart, wf-restart-copy) were removed with D-G70; git history keeps them. Since the WebKit
apps went (tag `last-webkit`) every scenario drives the native apps; the outputs from before that (the `*.out`,
`*.a1`, `*.r3` … folders of frosted-glass, ge1c-extension, r2d-picker, r3-ontop and the others, `findings/`) show the
WebKit apps and are history too. ge1a-disks-race (superseded by r2b-disks-race) was removed with them.

## Scenarios

A scenario is a Python file with `def run(h):`. `--scenario FILE` (a `.py` file inside G) runs it inside the
sandbox and the private `dbus-run-session`, after GNOME Shell is up, the extension is active, the native shell has
reported its boot (`{"event":"test","windowId":"native","data":{"tag":"boot","kind":"native-shell","pid":…}}` on
`WIN11_DEV_REPORT`, `native/shell/src/dev.rs`; the row is `h.session.facts['nativeBoot']`) and GNOME has mapped its
Panel window, and after GNOME's own notices are dismissed. It
replaces the built-in surface pass (add `--surfaces …` to run both, built-in first). Results go to
`<scenario dir>/<stem>.out/` (or `--out`): every screenshot, `results.json` and `logs/` (shell, harness, install,
memory, app logs). `results.json` has the usual run facts (install check, memory, stub calls, build dir, source)
plus `scenario`: `steps` (every helper call with its time and result), `checks`, `records`, `shots`, `error`
and `traceback`. The exit code is 1 when the scenario raised (also through `h.require`), not for a failed
`h.check`.

Write your own as `tests/gnome/scenarios/<step>-<what>.py`; `scenarios/native-shell.py` shows the native shell's
helpers, `scenarios/ge1c-extension.py` the language and extension ones. The helper `h`
(`tests/gnome/lib/scenario.py`, class `Helper`):

| Call | What it does |
|---|---|
| `h.open(spec, path=None, tabs=1, name=None, env=None)` | `panel`, `desktop` (resident windows of the native shell; returns them), `start` (opens the native Start with the extension's `_toggleStart` and waits for the Start window of the shell's pid), `explorer` (`win11-explorer <path or private home>`; `tabs=2` clicks the real "Nieuw tabblad" button, `explorer-new-tab`), `settings` / `settings:<page>` (a page id of `native/apps/settings/src/registry.rs`; when Settings is open already, one more launch with `--params {"page":…}` that GApplication hands to it, as Start's search does; then the page title), `taskmgr`, `alles`, `disks` (`win11-app --app …`), `defaultapps` (`win11-default-apps`), `installer` (`win11-installer [path]`). The launcher becomes `win11-gtk-<kind>` in place; the harness waits for the app's boot row (`native/kit/src/dev.rs`, `windowId` `native-<kind>`) and its window, and focuses it like a launch from Start. `env` adds variables for the app. Returns a `Surface` (`.window`, `.pid` (the app's, from its boot row), `.page`). |
| `h.close(name)`, `h.new_tab('explorer')`, `h.spawn(name, argv, env=None)` | Close an app like a user (window delete) or hide Start; one more Explorer tab; start a helper process on the private display (stopped with the session, `logs/app-<name>.log`). |
| `h.state(surface)`, `h.find(surface, name=None, label=None, cls=None, where=None)` | The surface's dev state (every toplevel's title, visibility, size and widget tree: name, type, CSS classes, label or tooltip, rect in the window, visible, sensitive, a switch's or check's `active`); the visible widgets that match, depth first, each with the index of its toplevel as `top`. A popover's widgets are in the tree of the widget it belongs to, with their rects in the window. |
| `h.click(surface, name=None, node=None, real=True)` | Click a widget, by its `set_widget_name` or a `node` from `h.find`. `real=True`: a Clutter virtual pointer in the private shell moves to the widget's centre (its window's frame rectangle as Mutter has it, the window geometry without a CSD shadow, plus its rect) and clicks (real Wayland input); `real=False` (or no rect): the dev channel's click by name. |
| `h.type_text(surface, name, text)`, `h.key(surface, key, name=None)`, `h.dev(surface, command)` | Set an entry's text, a key (`"Return"`, `"Escape"`, `"<Control>f"`, …) and any dev command (`native/kit/src/dev.rs`, `native/shell/src/dev.rs`) for that surface's process. |
| `h.backend(surface, method, args=None)` | A call to the app's own backend (`win11-app --app <kind> --http-only`) through its instance file (`$XDG_RUNTIME_DIR/win11-gnome/native-<kind>-<pid>.json`), as the app calls it. |
| `h.screenshot(name, surface=None, area=None)` | PNG of the screen, of one surface's window (panel: the 48 px strip; desktop: above the taskbar), or of `(x, y, w, h)`. |
| `h.page_text(surface)`, `h.text(surface)` | The surface's language (an app's: the tooltip of its close caption button, "Sluiten" or "Close", which the kit relabels on every switch; harness `app_lang`), window title(s), its visible labels and tooltips as one text, and `dutchWordHits` (`tests/i18n/dutch-words.txt`); `h.text` is the same without the hits and without a step in `results.json`, for polling. |
| `h.native(command)`, `h.native_state()`, `h.native_pid` | The native shell's dev channel (same `WIN11_DEV_COMMAND`/`WIN11_DEV_REPORT` files; the shell takes only commands with a `native` key and no `app` key): `{"native":"state"}`, `{"native":"click","name":"tb-start"}` (a widget by its `set_widget_name`), `{"native":"flyout","name":"quick"}` (toggle a flyout). `h.native_pid` is the running shell's pid. |
| `h.wait_text(surface, text, timeout=20, required=False)` | Wait until `text` is in the labels, tooltips or window titles; records a check; `required=True` raises on timeout. `h.wait_lang(surface, lang)` waits for the app's language. |
| `h.set_language(lang, via, skip=())` | Times the open apps (the native shell has no language marker). `via='dropdown'`: the real Settings › Tijd en taal › Taal en regio drop-down (`language-display`; opens Settings there if needed; real pointer clicks on the button and on the option in its popover, a dev click on the button as recorded fallback); Settings then writes `settings.json` itself through `settings.set`. `via='rename'`: the harness writes the private file as the store does (lock, temp file, rename). `via='rewrite'`: in place (same inode, `O_TRUNC` and one write). Returns per app the seconds until it showed the language, the Settings page title time, the file before/after (language, inode) and the input path used. |
| `h.window_titles()` | `global.get_window_actors().map(a => a.meta_window.get_title())` with the WM class (what the overview, Alt+Tab and the "is ready" banners show). |
| `h.app_names(ids=None)` | `Shell.AppSystem.get_default().lookup_app(id).get_name()`; default the 9 own `io.github.win11shell.*.desktop` ids. |
| `h.extension_labels()` | Through Eval, from the extension's own St widgets (the actor tree, not field names): fallback bar visible or not, native panel present, every label and accessible name, the clock text, the Ctrl+Alt+Tab group name(s), Dutch-word hits. |
| `h.kill_panel()` | Stops the private `win11-shell` (the process of the Panel window, a descendant of this harness; its backend `win11-panel --http-only` dies with it through `PR_SET_PDEATHSIG`), waits for the extension's fallback bar, screenshots it and returns its labels before and after. `h.restart_panel()` starts the taskbar again from its `.desktop` entry (`win11-panel` execs `win11-shell`) and waits for the shell's next boot row and its Panel window. |
| `h.check(name, ok, known=None, **detail)`, `h.require(…)`, `h.record(key, value)`, `h.log(msg)` | Record an expectation (`known='F1'` marks a known baseline defect), a hard requirement (raises), a value, a note. |
| `h.eval(js)`, `h.until(fn, label, timeout)`, `h.sleep(s)`, `h.lang`, `h.settings_file()`, `h.notifications()` | Low level: Shell Eval, polling, the current language, the private `settings.json` (language, inode), the message tray. `h.session` and `h.hm` give the harness Session and module. |

Rules for scenario authors: use only `h` (and the harness objects it exposes) to start or stop anything; the
sandbox (read-only root, private buses, own PID and network namespaces) holds regardless, but a scenario must not
try to write outside `tests/gnome/`. Close the apps you no longer need (each native app with its backend is
35-100 MB PSS, native-gtk-apps.py measures them).

## How surfaces are opened

| Surface | How it is opened |
|---|---|
| Panel, desktop, Start | The native shell, started by the extension (`win11-panel` execs `win11-shell`). Start is opened with `stateObj._panelBridge._toggleStart()` through Eval (closing falls back to a click on the shell's `tb-start`). Their state comes from the shell's dev channel (`{"native":"state"}`), not from a page. |
| Explorer | `prefix/bin/win11-explorer <private home>` |
| Settings | `win11-app --app settings --params {"page":…}`, then the same launch with the next page for the next pages (the running Settings takes it). The harness waits for the page title from `native/apps/settings/src/registry.rs` in Settings' header (`st-title`). |
| Taakbeheer, Alles, Schijven | `win11-app --app taskmgr`, `--app alles` and `--app disks` |
| Standaard-apps, App-installatie (scenarios only) | `win11-default-apps`, `win11-installer` |

Each launcher execs `win11-gtk-<kind>` in place (host `native_app_switch`), which starts its backend
`win11-app --app <kind> --http-only` as its child.

* Every app and the native shell write a boot row to `WIN11_DEV_REPORT` (`native/kit/src/dev.rs`,
  `native/shell/src/dev.rs`).
* State and text come back through `WIN11_DEV_COMMAND` (`run/logs/command.json`) and `WIN11_DEV_REPORT` (an app
  takes only commands whose `app` is its kind and answers as `native-<kind>`, the shell takes those without `app`).
* Screenshots are taken with `org.gnome.Shell.Screenshot` on the private bus.
* Apps started this way have no activation token, so GNOME maps them unfocused and posts '“<title>” is ready'.
  The harness records that text in `attentionNotifications`, then focuses the window and drops the banner.
* The built-in `--switch` rewrites `run/config/win11-gnome/settings.json` the way the host does: under
  `.settings.lock`, atomically, mode 0600. Scenarios can also use the real dropdown and an in-place rewrite
  (`h.set_language`).

## Known differences from the live session

* **Shell theme.** The GNOME chrome uses the stock theme, not Yaru, because the session mode is `user`. The
  Win11 apps and the native shell have their own GTK CSS (the kit's, `native/kit/css`); neither is affected.
* **No services.** There is no network, Bluetooth, UPower, logind, PackageKit, audio or UDisks, because the
  buses are private and empty (no portals either: `r2d-picker.py` starts the file portal and xdg-desktop-portal
  itself). So, for example:
  * Settings shows "Netwerk: Niet verbonden" and "Geen Bluetooth gevonden".
  * Schijven shows "UDisks is op deze computer niet beschikbaar".
  * Update shows "Kan niet controleren op updates".
  * The panel has no volume icon.
* **Sandbox-only views.** Task Manager sees only the sandbox's processes. Explorer, "Deze pc" and Storage see the
  sandbox's mounts: the system drive (`/`, the empty `run/root`) and the read-only binds, not the real data drives.
* **GPU.** The GPU render node is visible, as in Codex's runs; the native shell and the apps use GTK's cairo
  renderer (`GSK_RENDERER=cairo`, their default since D-G63).
* **Fresh cache.** Each run starts with an empty cache. (Until the WebKit apps went, the GStreamer registry was
  built once before the session: their first WebKit processes otherwise ran three `gst-plugin-scanner` processes,
  about 530 MB PSS. No native program builds it.)
