# Review of R3-NATIVE (round 3, native side)

Adversarial review of worktree `.wt/r3-native` (branch `r3/r3-native`, base 18c8ffbe), 2026-11-06. I changed no
product file. I wrote only this note and scratch files under `.rt/review-r3-native/` (scripts, logs, harness outputs
moved out of `tests/gnome/` after each run, the self-test JSON of my run). Every heavy command went through
`tools/heavy.sh` with an `r3-native-rv-*` tag. Nothing touched the live session: no `win11-*` outside the sandboxes,
no live bus, no gsettings/dconf/apt/snap/pkexec. The only connect() from inside the hardened layer to something
outside it went to my own decoy socket (§2.1, m2). Host sockets were only stat()ed from inside; the one probe that
called connect() on a host socket (`/run/udev/control`, root 0600) was refused by the kernel's permission check
before udevd saw anything.

**Verdict: fix-needed.** No blocker: I found no path from the hardened layer, the unit sandbox or the new tools to
the real system bus, the live session, dconf, pkexec, the network or a host socket that existed when the run
started. Two majors: an existing scenario's English expectation breaks with the ’ change and nobody is told (M1),
and the allowlisted root changes what Explorer, Settings and Task Manager show about drives in every harness run,
also in the screenshots round 3 relies on (M2). Also several minors, mostly in the harness's own detection logic.

## 1. Findings

| Id | Severity | Finding | Evidence | Fix |
|---|---|---|---|---|
| M1 | major | `tests/gnome/scenarios/ge1b-core.py:62` still expects the en `core.unknownMethod` text with a straight apostrophe (`"This function doesn't exist."`). The step changed that row to ’. Two checks of `ge1b-core` per surface (the forced `unknown` error and the `bridge.call` error) fail in en, and neither `r3-native.md` §5 nor `r3-native-baseline.txt` lists it. The step did not run ge1b-core. | `grep -rn "doesn't exist" tests/` gives `ge1b-core.py:62` only; ge1b-core.py:188-200 compare `error.get('message') == message`. My hardened run: ge1b-core 89/94; the 4 en failures are exactly these ("This function doesn’t exist." ≠ the expected straight apostrophe), for Settings and File Explorer each | Add to `r3-native-baseline.txt` item 3: `tests/gnome/scenarios/ge1b-core.py:62` → `"This function doesn’t exist."` (coordinator; not R3-NATIVE's file). Before the merge, grep every scenario, `tests/installer/*` and `tests/i18n/*` for the old en text of each changed row. |
| M2 | major | The allowlisted root makes `/` a tmpfs, and the product reads its drives from `/proc/self/mountinfo` (`services/storage/volumes.rs:36,128`). In every hardened harness run the system drive is gone. Settings › Home › Storage shows "233 GB-schijf (/etc)" / "233 GB Drive (/etc)" instead of "Systeemschijf (/)" / "System drive (/)". Explorer › This PC lists one "233 GB-schijf (/etc)" with a generic icon instead of "Systeemschijf (/)" and the six data drives. Task Manager › Summary shows 1 drive instead of 7. No check fails, but these surfaces now look wrong in every harness screenshot, and round 3 takes its nl/en screenshots of every surface from there. The step reports "page pixel stats equal to cross-r1.a2 except clock/content" and does not mention it. | Text dumps: `tests/gnome/scenarios/cross-r1.a2/results.json` (old layer) has `Ruimte op je systeemschijf\nSysteemschijf (/)`, `Deze pc\n\nSysteemschijf (/)`, 7 drives in Taakbeheer. `r3-cross-r1.group.out/results.json` and my run have `233 GB-schijf (/etc)`, `This PC\n\n233 GB Drive (/etc)`, 1 drive. Screenshot `.rt/review-r3-native/rv-r3n-cross.out/D-en-explorer.png` | The coordinator decides. (a) Record it as a known difference of the hardened harness and keep drive texts out of screenshot comparisons. (b) A product-side test seam for the mount table (a fixture mountinfo read only when a marked private seam directory is set, like the update seams), so the harness shows the real layout. (c) Not recommended: going back to `--ro-bind / /` and putting tmpfs over everything else breaks D-G42. In any case, list it in r3-native.md §7 and in the round-3 screenshot plan. |
| m1 | minor | Done-when not met with the tree's own scenario files: cross-r1 125/133, ge1c-extension 58/64, `test_extension_text.py` 6/8. Every failure is the en Ctrl+Alt+Tab group "Win11 Taskbar" against the old expectation "Win11 taskbar", in files R3-NATIVE does not own. The step documents this correctly (decision 1). | My runs: cross-r1 125/133 (8 failures: `C-en`, `E-EN-back`, `E-object-back`, `E-back` × `ext.group shows en` / `all layers agree on en`); ge1c 58/64 (4 × `Ctrl+Alt+Tab group en`, `c1-back` and `fallback bar labels English`, both through `labels_ok`, which compares the group); `test_extension_text.py`: 2 failures `'Win11 Taskbar' != 'Win11 taskbar'` | Coordinator: apply `r3-native-baseline.txt` item 3 (plus M1) in the merge commit, then rerun the three. |
| m2 | minor | A socket that appears in the checkout after `launch`'s `/proc/net/unix` scan can be reached from inside the sandbox. The guard checks the checkout only once, at start-up. Only the optional `--strace` check notices afterwards, or the end-of-run scan if the socket still exists then. A short-lived socket made by a host tool (an editor, LSP, `git fsmonitor`, another agent's helper under `.rt/`) during a run without `--strace` goes unseen. | Adversarial run `.rt/review-r3-native/rv_escape.py` with the outside helper `late_decoy.py`: the decoy `.rt/review-r3-native/l.s`, bound outside 118 s into the run, was a socket inside the sandbox, and `connect` returned `ok`. The helper counted **1 connection** (`late-decoy.json`). The run failed, but only through `strace-connect` and `socket-cleanup` (`left: [.rt/review-r3-native/l.s]`), rc 1 | Let the Sampler (already a 0.5 s thread) re-run `checkout_sockets()` every few seconds, skipping `target/` and `tests/build/`, and abort on a socket that was not there at start-up. Or bind only the subtrees a run needs (build dir, `vendor/`, `extension/`, `scripts/`, `host/src/*.json`, `host/i18n`, `tests/gnome`, `tests/update/fixtures`) instead of the whole checkout, with tmpfs over `.rt/` and `tests/logs/`. Make `--strace` the default for runs that serve as evidence. |
| m3 | minor | The inner guard trusts the outside scan for every visible host directory except the checkout. If a socket in `/run/udev`, `/etc`, `/usr`, `/var/cache/fontconfig` or the bound git directory is not masked (missed by the scan, created later, or left out with the hidden `--selftest-unmasked`, which `launch run` accepts too), the guard passes. The task's "refuses if a masked path is still a socket" branch (`harness.py:334-340`) is never run by the self-test: the "unmasked" case also drops the path from `WIN11_GE0C_MASKED`, so its refusal comes from the checkout scan (`:341`). | `tools/heavy.sh … -- python3 -B tests/gnome/harness.py launch --selftest-unmasked /run/udev/control probe --connect /run/udev/control`: rc 0, `problems []`, `connectTarget {isSocket: true}`, connect `Permission denied` (root 0600; `.rt/review-r3-native/probe-unmasked-udev.out`) | Inside, also scan the small host binds (`/run/udev`, `/var/cache/fontconfig`, `/etc`, the `extra_ro` dirs) for S_ISSOCK entries that are not covered by /dev/null. Accept `--selftest-unmasked` only with `probe`. Add a self-test case "listed in WIN11_GE0C_MASKED but not masked" (e.g. unmask `/run/udev/io.systemd.Udev` and keep it listed; no connect). |
| m4 | minor | The strace connect check classifies by unnormalised string prefixes (`harness.py:747,761`). A successful connect to `/var/run/udev/control` (that is `/run/udev/control`, a host socket) counts as "inside the sandbox". So does any path through `..` (`/run/../home/robin/x.sock`, `/var/cache/../../home/robin/.cache/ibus/dbus-x`). | A synthetic log through `harness.py strace-check` (`.rt/review-r3-native/synthetic.strace`): 5 successful connects, 3 of them to host paths, and only `/run/udev/control` was reported | `os.path.normpath` each path and map `/var/run/` → `/run/` before matching `private` / `host_binds`. Also flag relative `sun_path`s explicitly. |
| m5 | minor | The hang fix is not proven, and the test-only `.init_array` set-up (`files/read.rs:795`) runs in every invocation of the lib test binary. That includes `--list` and single-test runs, and `tests/update/run.sh bus-tests`, where GIO_USE_VFS is not set until WU-FIX. Outside any sandbox (never allowed) it would build GVfs's daemon VFS and a dconf GSettings before `main`, even for `--list`. The 20 runs show no regression only: 18c8ffbe also passed 20 + 550 runs. | My 20 runs: 20/20, 411 passed each, 6.27-6.29 s, guard log 0 bytes. The step's own base runs: `.rt/r3-native/repro-base/summary.txt` 20/20. Bus tests with the new binary: `tests/update/run.sh bus-tests` 9/9 passed, strace OK, so the constructor does no harm there today; the risk is the unconditional pre-main GIO set-up, not a failure seen | Run the constructor only when `GIO_USE_VFS=local` and `GSETTINGS_BACKEND=keyfile` (the sandbox's env), else return. Keep the stack-capturing runner (`.rt/r3-native/hang-runner.py`) as the coordinator's run-tests.sh option, so the next hang leaves evidence. |
| m6 | minor | The committed text output of the cross-r1 run says the strace check failed: `tests/gnome/scenarios/r3-cross-r1.out/results.json` `straceConnect.ok: false`, violations `/var/lib/sss/pipes/nss: UNFINISHED` ×2 (the first rule). r3-native.md says "strace 1258 connects OK (re-checked with the final rule)". Also, `r3-cross-r1.group.out/` and `r3-ge1c-extension.group.out/` are results of wrappers that live only in `.rt/r3-native/wrap/` and are not in the repo. | `python3 -c` on that results.json (the violations above) | Rerun `strace-check` and rewrite `straceConnect` (or rerun the scenario) before committing. Don't commit the two `.group.out` directories, or commit them only after the coordinator's one-string fixes, rerun from the real scenario files. |
| m7 | minor | README wording. README.en.md has 11 straight apostrophes in new English prose ("GNOME's", "project's", "polkit's", "PackageKit's", "NetworkManager's", "WebKit's", "Mutter's", "snapd's", "app's"), against D-G23 / the task rule for new English (’). The Dutch section quotes GNOME's English banner "(“X” is ready)". In a Dutch GNOME that banner is in Dutch, so the example reads as if it stays English. Both READMEs say the extension and the installer "always read `~/.config/win11-gnome`"; they read `$XDG_CONFIG_HOME/win11-gnome` (`extension.js` `GLib.get_user_config_dir()`, `scripts/install.py:457`). | `README.en.md` (regex over prose without code spans); `README.md:131`; `README.en.md` Language section | Use ’ in README.en.md prose. nl: "GNOME's melding dat een venster klaarstaat (in het Engels “X” is ready)" or the Dutch GNOME text. Both: "`~/.config/win11-gnome` (of `$XDG_CONFIG_HOME/win11-gnome`)". |
| m8 | minor | Scenarios that "the harness runs today" were not run by the step in the hardened layer: gx1-restart, ge1a-*, r2b/r2d, ge1b-core, ge1b-perf, and a `--source <rev>` run (the new git-directory bind is untested). I ran gx1-restart (56/56), `--source 18c8ffbe` example.py (30/30) and ge1b-core (89/94: M1, plus a picker check that is probably older than this step, §3). Still not run: ge1a-switch, ge1a-start-switch, ge1a-disks-race, r2b-disks-race, r2d-picker, ge1b-perf. | `r3-native.md` §7; my runs in §3 | Coordinator: run ge1a-switch, ge1a-start-switch, ge1a-disks-race, r2b-disks-race, r2d-picker and ge1b-perf once in the hardened harness before the merge. |
| m9 | minor | D-G23 is locked in only for the TSV catalogues. check_rust's new rule reads `host/i18n/*.tsv` rows. Host English with straight apostrophes stays in the `_STR` JSON tables D-G8 keeps: `services/system/strings.json` (6 Wi-Fi errors: "Couldn't connect.", "You're not allowed …", "isn't supported" ×2, "didn't give an address", "isn't available") and `services/storage/data.json` ("browsers'"). `services/disks/data.json` (21) is "for Robin" (D-G1b). | `grep -n "[a-z]'[a-z]" host/src/services/system/strings.json`; a JSON walk over the `en` values | Coordinator: convert system/storage in a step that owns those files, and extend the rule to the `[nl, en]` / `{nl, en}` JSON tables (disks stays listed for Robin). |
| n1 | note | D-G31: the step keeps the *current* language on an invalid import (task text). H (`tests/test_i18n.py:1068`, "dropped → the default") and the mock fall back to nl. The step raised this correctly (decision 2). An import of a file without `language` still resets to nl, as before. | `settings.rs:262-284`; H test | Coordinator decides; R3-UI's mock must follow the same rule. |
| n2 | note | The Explorer deferral is unit-tested only. No harness case runs a copy during `shell.restartAll`, and the `run_inner` wiring (`explorer_files.set`, `watch_files_idle`) has no test. The deferral is silent for Settings (no `shell.restartDeferred`, decision 5). | `lib.rs:393-431`; gx1-restart has no copy step | GX1's successor: add an R3/R9 case (a slow copy in Explorer, click restart, Explorer comes back after the copy) and `Wait::Files` in restart.rs. |
| n3 | note | `launch_problems` (`harness.py:712`) accepts `--out tests/gnome` (out == HERE). The bind list then makes the whole `tests/gnome` (harness.py, lib/) writable inside. This is no host-safety issue. | `_inside(HERE, HERE)` is true, `_inside(HERE, RUN)` false | Refuse an out directory that is HERE or contains RUN. |
| n4 | note | Ctrl+Alt+Tab group: en "Win11 Taskbar" now matches the window title. nl "Win11-taakbalk" still differs from the nl window title "Win11 Taakbalk" (`ge1b-core.py:51`). Dutch is frozen, so this is for Robin. | `extension.js:31` | List for Robin with the D-G1b items. |
| n5 | note | `tests/gnome/README.md` still describes `--ro-bind / /` and a writable `tests/gnome` (not R3-NATIVE's file). `tests/gnome/baseline/guard-selftest.json` is the self-test's output (rewritten with new pids and times). `Files::for_tests` / `job_for_tests` are test-only helpers beyond "only Files::busy() and files.idle". All acceptable. | step §4.5, §7 | Coordinator updates tests/gnome/README.md. |
| n6 | note | Not R3-NATIVE's: `ge1b-core.py`'s picker part (D) times out ("two pickers mapped"). It looks for Settings-class picker windows (`ge1b-core.py:46-48,133`), while D-G36 (R2-D) gave the in-app picker WM class FilePicker. Its committed output is from round 1 (r1-debug, 99/99). | my ge1b-core run, check `D: pickers: completed without an exception` | The owner of ge1b-core matches pickers by title only, or by the FilePicker class. |

## 2. Safety (check 1)

### 2.1 What I tried

* **Hardened harness, from inside** (`rv_escape.py`, one GNOME session, `--strace`, r3-native-debug). Results in
  `.rt/review-r3-native/rv-r3n-escape.out/results.json`:
  * Network namespace `net:[…]` differs from the host's `net:[4026531833]`; `/proc/net/dev` lists only `lo`. None
    of the 18 host abstract names from the host's `/proc/net/unix` is in the sandbox's. 27 PIDs visible.
  * Of the 114 host path sockets from the host's `/proc/net/unix` (live session bus, at-spi, system bus, snapd,
    systemd/journal/userdb/resolve, docker/containerd/libvirt, tailscale, cups, gdm, ibus in the real `~/.cache`, …),
    112 do not exist inside. The 2 that do (`/run/udev/control`, `/run/udev/io.systemd.Udev`) are `/dev/null` (char 1:3).
  * A walk of the whole visible tree (`/proc /sys /dev` skipped) finds 7 socket files, all the sandbox's own
    (`/tmp/w11-rt/{bus,sbus,w11-wl}`, 3 event sockets in `/tmp/w11-rt/win11-gnome/events`, `run/cache/ibus/db`).
  * Live paths (`/run/user/1000/*`, `/run/dbus/system_bus_socket`, `/var/run/dbus/…`, `/run/systemd/{private,journal/*}`,
    `/run/snapd*.socket`, `/tmp/.X11-unix/X0`, `~/.config`, `~/.local/share/gnome-shell`, `~/.config/dconf/user`,
    `/var/lib/dpkg/status`, `/var/log/apt`, `/run/reboot-required`, `/var/snap`, `/snap`) are absent. The udev
    sockets are /dev/null; `/etc/dconf` (the system db) is a read-only directory.
  * Mounts: `/` tmpfs ro; `/usr`, `/etc`, `/sys` (+ submounts), `/var/cache/fontconfig`, `/run/udev` and the checkout
    ro; rw only `tests/gnome/run`, the out directory, and the tmpfs `/tmp`, `/run`, `/var`, `/dev`. `access(W_OK)`
    is false for W, harness.py, tests/gnome, `.rt/`, `.git`, `/etc`, `/usr`, `/home/robin`, `/`.
  * Env: `GIO_USE_VFS=local`, `GSETTINGS_BACKEND=keyfile`, session bus `/tmp/w11-rt/bus`, system bus `/tmp/w11-rt/sbus`,
    `WIN11_UPDATE_TOOL_DIR`/`WIN11_UPDATE_FIXTURE_ROOT` in run/ (0700, marker, the 17 stub names of the paths.rs Tool
    table), no `WIN11_UPDATE_PK_BUS` (so paths.rs's test mode is `BusTarget::Disabled`: no system-bus connection at
    all), `updateToolCalls` [].
  * The late decoy: reachable (m2).
* **strace**: every run of mine with `--strace` had 0 violations except the deliberate late decoy (escape run). The
  rest were connects to the private buses, Wayland, event sockets, loopback, plus ENOENT on `/run/pipewire`,
  `/run/systemd/journal/stdout`, `/var/run/nscd/socket`, `/var/lib/sss/pipes/nss` (all in the private tmpfs).
  Counts: r3-ontop 445 connects, cross-r1 1259, gx1-restart 837, `--source 18c8ffbe` 614, all OK; the escape run 187 with 1 violation, the decoy.
* **Guard self-test**: 125/125 (`.rt/review-r3-native/selftest.out`). The decoy got exactly 1 connection, the positive
  control from outside. `tests/gnome/baseline/guard-selftest.json` was restored to the step's version after my run
  (mine: `.rt/review-r3-native/guard-selftest.review.json`).
* **Guard gaps**: m3 (probe with an unmasked udev socket passes), m4 (strace prefix logic).
* **Unit sandbox** (`run-tests.sh`): unchanged layer plus `GIO_USE_VFS=local`; `guard-calls.log` 0 bytes in 21 runs.
* **Socket left-overs**: `find W -type s` after each of my harness runs: none after any of the 7 runs (`.rt/review-r3-native/chain-sockets.txt` is empty); every run removed exactly 1 file in run/ (`cache/ibus/db`) and left 0, except the escape run, which left my decoy and failed as it should. `tests/update/run.sh bus-tests`
  after the harness runs (it refuses on a visible socket in the worktree, GX1's problem): it started at 02:08:49, 1 s after the ge1b-core run ended, and did not refuse (9 bus tests, 9 passed), so the GX1 left-over problem is fixed.

### 2.2 Verdict on safety

No blocker. Everything that exists at launch is hidden or masked, the network namespace is private, and the update
service in the harness opens no system bus and runs no real tool. The remaining holes are a socket appearing during
a run (m2) and silent gaps in the second line of defence (m3, m4).

## 3. Done-when, rerun by me (check 3)

| Check | Command (from W) | Result |
|---|---|---|
| Unit suite | `tools/heavy.sh --min-mb 2048 --tag r3-native-rv-test -- .rt/review-r3-native/unit.sh` (= `tests/rust-sandbox/run-tests.sh --no-fail-fast`, then 20 × `run-tests.sh --lib`) | full: rc 0, lib **411 passed / 17 ignored / 0 failed**, win11-launch 16 passed / 1 ignored, other bins 0; `guard-calls.log` **0 bytes**; 8.2 s |
| 20 lib runs | same script | **20/20** rc 0, 411 passed each, 6.27-6.29 s each, guard 0 bytes each, no hang (no proof of the fix, m5) |
| drift_rust | `python3 -B tools/i18n/drift_rust.py --warnings` | 0 errors, 2 warnings (update/options.rs, update/paths.rs, as at HEAD) |
| check_rust | `PATH=…node/bin:$PATH python3 -B tools/i18n/check_rust.py --warnings` | **0 errors**, 72 warnings |
| rustfmt | `tools/rustfmt-check.sh` | 142 files, 111 clean, 31 listed, **0 failing** |
| rustfmt only? | rustfmt on the 18c8ffbe copies of runtime/window/i18n (`.rt/review-r3-native/fmt/`) | window.rs, i18n.rs byte-identical to W; runtime.rs differs only in the 4 test strings (’) |
| check.py | `PATH=…node/bin:$PATH python3 -B tools/i18n/check.py` | **0 errors**, 124 warnings (glossary 52, rust 72); readme rule passes |
| UI dict diff | `check_ui.py --dict-diff tests/i18n/dict-nl-3b898e6.tsv --changes …` | with `.rt/r3-native/BASELINE-CHANGES.ui.merged`: 0 not listed, 0 stale; with the tree's file: 1 NOT LISTED (onTopDesc), as the step says; the merged file differs only in the two lines |
| Manifest | `python3 -B tools/ui-manifest.py --check` | up to date (181 files) |
| Extension text | `python3 -B tests/installer/test_extension_text.py` | 6/8, the 2 failures are "Win11 Taskbar" vs "Win11 taskbar" (m1) |
| Guard self-test | `tools/heavy.sh --tag r3-native-rv-selftest -- python3 -B tests/gnome/harness.py guard-selftest` | **125/125** |
| r3-ontop (nl first, covers nl and en) | `… launch run --lang nl --build-dir tests/build/r3-native-debug --source worktree --scenario tests/gnome/scenarios/r3-ontop.py --strace …` | rc 0, **31/31**, 142 s, peak PSS 1166 MiB; strace 445 connects OK; socket cleanup 1 removed (`cache/ibus/db`), 0 left; update tool calls 0. Maximised Task Manager stays above (stack 3 > Explorer 1) in nl and en |
| cross-r1 | `… --surfaces desktop,panel,start,explorer,settings,taskmgr,alles,disks --scenario tests/gnome/scenarios/cross-r1.py --strace …` | rc 0, **125/133** (8 = the en group, m1); 298 s, peak PSS 1131 MiB; strace 1259 connects OK; 1/0 sockets |
| ge1c-extension | `… --scenario tests/gnome/scenarios/ge1c-extension.py` | rc 0, **58/64** (all 6 are the en group, m1); 157 s, peak PSS 972 MiB; 1/0 sockets |
| gx1-restart (not run by the step) | `… --scenario tests/gnome/scenarios/gx1-restart.py --strace …` | rc 0, **56/56**; 142 s, peak PSS 1232 MiB; strace 837 connects OK; 1/0 sockets; update tool calls 0. The restart, with Explorer’s new deferral path (nothing in flight), works in the hardened layer |
| `--source 18c8ffbe` + example.py (not run by the step) | `… --build-dir tests/build/base-debug --source 18c8ffbe --scenario tests/gnome/scenarios/example.py --strace …` | rc 0, **30/30**; git archive of 18c8ffbe (commit 18c8ffbe706d) through the bound git directory in 9.5 s; strace 614 connects OK; 1/0 sockets. The untested `--source <rev>` path works |
| ge1b-core (not run by the step) | `… --scenario tests/gnome/scenarios/ge1b-core.py` | rc 0, **89/94**: 4 × the en `core.unknownMethod` text (M1). The fifth, `D: pickers: completed without an exception` ("two pickers mapped" timed out), is probably older than this step: ge1b-core looks for pickers with WM class Settings (`ge1b-core.py:133`), and D-G36 (R2-D) gave the in-app picker WM class FilePicker. Its committed output is from round 1 (r1-debug, 99/99), and nobody has run it since round 2. 203 s, peak PSS 1417 MiB; 1/0 sockets |
| bus tests with the new test binary | `tools/heavy.sh --tag r3-native-rv-bustests -- tests/update/run.sh bus-tests -- target/debug/deps/win11_host-12500e7eb53cf9c1` | rc 0, **9 passed, 0 failed**; strace OK in each exec (nothing reached `/run/dbus/system_bus_socket` or `/run/user/1000/bus`). My `tests/update/.run/` (it did not exist before) was removed afterwards |
| shoot.sh unknown id | `tools/heavy.sh --tag r3-native-rv-shoot -- tests/update-ui/shoot.sh taskmgr.settings,no-such-surface` | "FAILED unknown surface id 'no-such-surface'", "2 shots, 1 failed (1 unknown surface ids)", **exit 1**; the 2 known shots were taken |

## 4. Conformance (check 2)

| Item | Status |
|---|---|
| 1 settings.import (D-G31) + test | done (current language kept; n1 for the H/mock difference) |
| 1 core.tsv 4 rows ’ + check_rust rule | done; the rule covers TSV rows only (m9). Missed expectation in ge1b-core (M1) |
| 1 read.rs theme cache; reproduce first | done; cache under one lock, 2 s TTL; no reproduction (m5) |
| 1 rustfmt runtime/window/i18n + `tools/rustfmt-check.sh` | done, formatting only (plus 4 test strings, deviation 1) |
| 2 Task Manager always-on-top (make_above + stick, WM class, live, restored) | done, GNOME 50 public Meta.Window API only, no new file (C7), inline text tables untouched |
| 2 switch enabled, note dropped (D-G21 Dutch change) | done; BASELINE-CHANGES.ui lines and the nl/en dump re-record handed to the coordinator (not owned) |
| 2 "Win11 Taskbar" en | done; 4 expectations elsewhere (m1) |
| 3 allowlisted root, masking, inner guard, decoy | done; added `/run/udev` and the font cache (deviation 3, needed for the GPU); gaps m2-m4 |
| 3 cross-r1 133/133, ge1c pass, against base-debug | only with the coordinator's string updates (m1); the step ran r3-native-debug (host code changed, allowed) |
| 3 no socket left; checked in the runner | done (`socketCleanup`, the run fails on a left socket) |
| 4 README.en.md + README.md link and Dutch "Taal" | done; wording m7; D-G11 list complete (polkit, PackageKit, NetworkManager, WebKit, GNOME's banner); `WIN11_GNOME_CONFIG_DIR` test-only |
| 5 example.py C6 poll, hard checks | done (30/30 per the step; my `--source 18c8ffbe` run 30/30) |
| 5 shoot.sh unknown id fails | done (exit 1 with one unknown and one known id) |
| 6 GIO_USE_VFS + update seams for every harness run; run-tests.sh GIO_USE_VFS | done (env checked from inside, §2.1) |
| 6 Files::busy + files.idle + Explorer deferral + unit test | done; no end-to-end case (n2) |
| 6 hang: shared cause, fix, proof | cause argued, three mitigations, no proof (m5) |
| `r3-native.md` | written; misses M1 and M2 |

## 5. Language (check 4)

* New English: onTopDesc "Keeps Task Manager above other windows and on every desktop." (3b898e6's own first
  sentence, glossary Bureaublad → Desktop); the group "Win11 Taskbar" (the window title and the desktop entry's
  name); the 4 core rows with ’ ("This action doesn’t accept any input.", "Can’t reach the reference service…",
  "This function doesn’t exist."). README.en.md: en-GB ("colours", "maximised", "licence"), sentence-case headings, Windows names as the UI shows them ("Settings › Time & language › Language & region",
  "Settings › System › Power & battery", File Explorer, Recycle Bin, Task view); the paths match
  `settings/registry.js:40,134,141`. Straight apostrophes: m7.
* Dutch: the only changed Dutch UI text is `taskmgr/pages/settings.js:onTopDesc`. It is now exactly the first
  sentence of 3b898e6's text ("Taakbeheer blijft boven andere vensters en is op elk bureaublad te zien."), a D-G21
  change ready for BASELINE-CHANGES.ui. The extension's nl group "Win11-taakbalk" and every other nl text are
  unchanged; `README.md` only gains a link line and the new "Taal" section, which the task allows. No "for Robin"
  item was changed: storage.trashFailed "(ApiError)", the disks tables' apostrophes, "Nederlands (Nederland)" and
  Afbeelding are untouched.
* Errors stay matched on code/key: the step changed no matcher, and ge1b-core matches message too, which is why M1
  bites.

## 6. Look (check 5)

* Harness screenshots of my r3-ontop run (`.rt/review-r3-native/rv-r3n-ontop-nl.out/*.png`): Task Manager's
  Settings page in nl and en has the switch enabled and "Aan"/"On" when on. The description fits on one line at the
  harness width in both languages. Task Manager stays in front of the focused File Explorer and shows on workspace 2.
  Explorer is in front again after the switch is off. Windows 11 look as before.
* Mock UI shot (headless Chrome in nonet.sh) of `taskmgr.settings` at 1280×1400: nl and en (`.rt/review-r3-native/shots/{nl,en}/taskmgr.settings.png`): the "Altijd op voorgrond"/"Always on top" card has an enabled toggle "Uit"/"Off". The description is one line ("Taakbeheer blijft boven andere vensters en is op elk bureaublad te zien." / "Keeps Task Manager above other windows and on every desktop."), with no clipping, the same height as the other cards, Windows 11 look.
* M2 is visible in the screenshots: `D-en-explorer.png` shows "233 GB Drive (/etc)" with a generic file icon under
  This PC.

## 7. Ownership (check 6)

`git -C W status --short` / `git diff --stat`: 25 modified files (+2994/−1684, the same before and after my review), all inside the step's allowlist: host/src
settings, files/mod.rs, files/read.rs, runtime, window, i18n, lib.rs; core.tsv; check_rust.py; run-tests.sh; harness.py;
example.py and example.out; shoot.sh; BASELINE-CHANGES.ext; extension.js; taskmgr settings.js and ui-sha256.json;
README.md. Untracked: README.en.md, notes/claude/round3/, r3-ontop.py and r3-*.out, tools/rustfmt-check.sh. Also
`tests/gnome/baseline/guard-selftest.json` (the owned self-test's output). Borderline: the `r3-cross-r1*.out` and
`r3-ge1c-extension*.out` directories are outputs of scenarios R3-NATIVE does not own (m6). After my runs the status
is unchanged apart from this note: my harness outputs were moved to `.rt/review-r3-native/`, the self-test JSON was
restored, and `tests/update/.run/`, which my bus-test run created, was removed (it did not exist before).
