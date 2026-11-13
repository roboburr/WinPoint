# Adversarial critique of `windows-update-gnome.md` (GE0d, 2026-11-05)

Target: `notes/claude/windows-update-gnome.md` (1,246 lines). Only this file was written. Nothing was built, run,
launched or signalled. No PackageKit call, no apt/apt-cache/pkcon, no do-release-upgrade, no gsettings set, no busctl.

**Read-only checks used** (all on this PC): the cited `host/src`, `vendor/ui`, `extension`, `scripts` lines; `strings`/`nm -D`
on `/usr/libexec/packagekitd`, `libpk_backend_apt.so`, `/usr/lib/polkit-1/polkitd`, `/usr/bin/pkexec` and
`systemd-xdg-autostart-generator`; the PackageKitGlib typelib (`python3 -I`, no D-Bus); the polkit action XML and rules;
`/usr/bin/do-release-upgrade`, `DistUpgradeMain.py`, `DistUpgradeController.py`, `MetaRelease.py`,
`check-new-release-gtk`, `UpdateManager.py`, `Dialogs.py`; the update-notifier and update-manager schemas; the GNOME Shell 50
`extensionUtils.js`/`extensionSystem.js` (`gresource extract` to stdout); `xdg-terminal-exec` and the Ptyxis desktop
entry; `/var/log/apt/history*`; `/proc/*/comm`, `/proc/6196/maps` (read only); the apt changelog.

**Git status:** at the start and before writing, the only changes were the expected round-0 files (`vendor/ui/js/mock/index.js`,
`vendor/ui/js/mock/gnome.js`, `tests/`, `tools/`, `notes/claude/{plan-critique.md,round0/,windows-update-gnome.md}`).

## Summary

| # | Sev | Finding | Where in the design |
|---|---|---|---|
| B1 | **blocker** | A test that escapes its seams runs a **real, password-less** update, linger change or reboot on Robin's PC | §4.11, §9.1, §9.3, GW7/GW8/GX1 |
| M1 | major | PackageKit's apt backend has no phasing logic, so "phased = info 9, never in Alles" is very likely false | §3.2, §3.3, §4.5, §4.10 |
| M2 | major | The GNOME-51 "hold" does not hold: Ubuntu's own update-manager and check-new-release-gtk still offer 26.10 | §3.5, §6.2, D-G12, §12 C4/4 |
| M3 | major | Settings is `NON_UNIQUE` and `shell.openApp` always spawns, so two full-mode `update.*` services run at once | §4.1, §4.4, §4.7 |
| M4 | major | R2 `./scripts/install.sh --system-files` does a **full user-local deploy** on the live session (install.sh drops all args) | §7, §11 R2 |
| M5 | major | Hand-over has no "did not start" or "ended without restart" state, and the Ptyxis window closes on any early exit | §3.5, §6.4, §6.6 |
| M6 | major | `update.check/install/repair` are not refused while the release upgrader runs | §4.9, §6.4 step 7 |
| M7 | major | The D2 toggle overpromises: update-notifier still auto-opens at once for security updates | §3.6, §4.12, R8 |
| M8 | major | `shell.restartAll` guard is too narrow: Software jobs block shutdown by `join`, and a download may be cancelled when Settings vanishes | §5, §4.7, C2 |
| m1–m24 | minor | Code facts, fidelity texts, test seams, ownership gaps (below) | various |

## Blocker

### B1. A seam escape becomes a real system change, with no prompt

**Design claim.** Seams are "honoured only when euid ≠ 0 and the directory contains a marker" (§4.11); `cfg(test)` makes
`pk::system_bus()` panic; GW8 runs the real Settings binary in GE0c's harness and says "the real system bus is never used for
PackageKit" (§9.3). GE0c's harness shares the real system bus (gnome-map §5, §6: "shared system bus").

**Evidence.**
* `polkitd` falls back to the uid's display session for processes outside any session: `strings /usr/lib/polkit-1/polkitd`
  lists `sd_pid_get_session` **and** `sd_uid_get_display` (polkitd 127-2ubuntu1.1). A harness process of uid 1000 is
  therefore judged as Robin's active local seat0 session.
* `/usr/share/polkit-1/rules.d/org.freedesktop.packagekit.rules`: `system-update` → `YES` for `subject.active && subject.local
  && isInGroup("sudo")`. `id -nG` lists `sudo`. `system-sources-refresh` is `yes` for active.
  `org.freedesktop.login1.reboot` and `reboot-multiple-sessions` are `yes` for active; `set-self-linger` is `yes/yes/yes`.
* The seam rule says nothing about what happens when a variable is set but rejected; the natural reading falls back to the
  system bus. `cfg(test)` does not cover the real binary in GW8/GX1.
* Tools that bypass PATH stubs: `/usr/lib/ubuntu-release-upgrader/check-new-release` (§6.1, absolute and not on PATH),
  `/usr/bin/pkexec` in the terminal argv (§6.4), `nice`/`ionice`/`screen`. `check-new-release` is also run **automatically
  on page open** when the last run is older than 24 h (§6.1 step 3), which is a network download in any harness pass that
  opens the update page (also GE0c's or round 7's generic screenshot passes, which set no update seams).
* "Nu opnieuw opstarten" (H10, C08) goes to the real panel through `shell.command` (`shell.rs:128-146`). The panel's
  `power.rs:39` runs `systemctl reboot` by PATH, then logind on the real system bus answers `yes`.
* Precedent: GE0b's sandbox already sets `DBUS_SYSTEM_BUS_ADDRESS=unix:path=<sandbox>/run/no-system-bus` and puts a
  `guard-bin/` first on PATH (`tests/rust-sandbox/run-tests.sh:9-13, 87, 98-106`). The update design takes over neither.

**Failure.** One wrong marker path, a missing `WIN11_UPDATE_TOOL_DIR`, or a driver click runs a real `RefreshCache` +
`UpdatePackages`, a real `loginctl enable-linger` (persistent), a real `check-new-release` download, or a real reboot of the live
PC. None of these shows a password prompt. This breaks the plan's hard rules even if nobody is careless.

**Fix (before GW1/GW2 start, owned by GW1 + GE0c):**
1. **Fail closed.** If any `WIN11_UPDATE_*` variable is set and its check fails, the service enters `unsupported` with
   `error.code="test_seam"` and opens **no** bus and runs **no** tool. Add a unit test for it.
2. **Every Win11 process in the GNOME harness gets `DBUS_SYSTEM_BUS_ADDRESS`** pointing at the fake bus (GW1) or a dead socket
   (GE0b style). Do this through the private prefix's `.desktop` `Exec=env DBUS_SYSTEM_BUS_ADDRESS=… …` lines, or for
   gnome-shell itself if GE0c shows headless gnome-shell 50 starts without the real system bus. Then `systemctl`, `pkexec`
   and PackageKit cannot reach the real daemons even when a seam is wrong.
3. **Put GE0b's `guard-bin` first on PATH for the whole harness session**, including `systemctl`, `loginctl`,
   `gnome-session-quit`, `pkexec`, `xdg-terminal-exec`, `screen`, `apt*`, `update-manager`, `gnome-software`, `snap-store`.
   **All tools resolve through `paths.rs::Tools`**, which in test mode points only at the stub dir. No literal absolute exec path
   may appear outside `paths.rs` (enforce it with a grep test in GW2).
4. Make `check-new-release` **click-only** (drop the auto-run on page open), or skip it whenever `backend::dry_run()` or any test
   marker is set.
5. Harness self-test: after each GW8/GX1 run, the guard log must be empty of real calls, and the fake's `CallCount()` must
   equal the expected count.

## Major

### M1. Phased updates: the apt backend knows nothing about phasing

**Design claim.** "Phased packages (`info` 9 blocked) … are never part of Alles"; "Only phased updates → hero upToDate";
"`system.updates.available` counts non-phased only (PackageKit)" (§3.2, §3.3, §3.9, §4.10). R1 is supposed to settle the
representation.

**Evidence.** `strings libpk_backend_apt.so | grep -i phas` is empty. `nm -D --undefined-only` imports no phasing API
(no `PhasingApplied`), only `pkgCacheFile::BuildPolicy`. apt 3.2.0 implements phasing in its upgrade and auto-install logic
(changelog 2.7.10 "Do not mark updates for install that are still phasing", 2.7.11 "Add public phased update API"), not in
the candidate policy. `info.rs:306` `apt list --upgradable` already counts phased packages. do-release-upgrade itself filters
on the `Phased-Update-Percentage` record (`do-release-upgrade:175-191`).

**Consequence.** Expect GetUpdates to list phased packages as normal/security updates. Then one of two things happens.
`UpdatePackages(ids)` installs them early, bypassing Ubuntu's phasing, which is something the Software Updater never does.
Or libapt silently skips them, so the job "succeeds" while those rows stay. That second case is an undefined row state:
§4.6 "After Finished" defines leftover rows only after a failure. Either way H16, the `phasedLater` line and the count
rules are built on a representation that probably does not exist.

**Fix.** In GW2, classify phasing ourselves instead of waiting for R1. Make one batched local read of the candidate records
(`apt-cache show pkg=ver …`, the same call §6.4 already plans), or use python3-apt's `DepCache.phasing_applied()` (python3-apt
3.1 is installed). Treat any package whose candidate has `Phased-Update-Percentage` (do-release-upgrade's rule) or
`phasing_applied` as `phased`. Never pass those ids to `UpdatePackages`. Define the "success but rows remain" outcome
(`available`, no error). Add a fake timeline where GetUpdates returns a phased package with info 5. R1 then only confirms.
Note: info 9 `blocked` can also mean a held package, so give it a neutral text, not "offered later".

### M2. "Wacht op een compatibele Win11-versie" blocks only our card

**Design claim.** An untested GNOME major blocks the upgrade (D-G12, §3.5 `hold`, C4). Prompt stays `normal` (§12 row 4).

**Evidence.** `check-new-release-gtk:98-107` (started by update-notifier) offers every new release unless
`com.ubuntu.update-manager check-new-release-ignore` equals its codename. `UpdateManager.py:503-512` shows
`DistUpgradeDialog` ("However, %s %s is now available", `Dialogs.py:252`) with an upgrade button whenever
`check-dist-upgrades` is true (default true). update-notifier auto-opens update-manager (§4.12). The GTK path has no screen,
so a gnome-shell crash kills it mid-dpkg (§6.4).

**Consequence.** With `Prompt=normal`, Ubuntu itself will invite Robin to 26.10 / GNOME 51 while our page says "wait". The
post-upgrade safety net is then the only protection.

**Fix.** While the verdict is `hold` or `blocked`, offer the action "Ubuntu laten wachten met deze upgrade" / "Make Ubuntu
hold this upgrade". It writes `check-new-release-ignore=<codename>` through `gio::Settings`: user level, reversible,
honoured by Ubuntu's own dialog. Clear it when the verdict becomes `ok`. Alternatively, recommend `Prompt=lts` while held (on an
LTS, MetaRelease then reads `meta-release-lts`, `MetaRelease.py:165-171`). Write in the card that Ubuntu's own Software
Updater may still offer the upgrade. This needs a decision from Robin next to R8. The CLI `do-release-upgrade` stays
available on purpose.

### M3. Two Settings processes, two full-mode update services

**Evidence.** `lib.rs:295-301`: every kind except start/panel/desktop/alles gets `ApplicationFlags::NON_UNIQUE`.
`shell.rs:177`: `shell.openApp` always launches a new process (Taakbeheer's health item opens `settings` `page:update`,
`health.rs:333`). §4.1 gives full mode to every `kind == "settings"`.

**Consequence.** Two `update-pk` threads, two sets of FileMonitors, two `RefreshCache` calls on two clicks, notifications sent
twice, and two in-memory `rev` counters broadcast through `update.changed` to the same consumers. A UI that compares `rev`
flaps. The job lock prevents two installs, but the reattached instance and the owner both write `job.json`.

**Fix.** Elect an owner with a non-blocking `flock` on `update/owner.lock`, taken at LazyService create. Non-owners run reader
mode and forward writes to the owner, which already publishes a backend instance file with port and token
(`backend::write_instance`). Or they answer `not_here` with a "Open het andere Instellingen-venster" hint. Keep `rev` in
`snapshot.json` so it is monotonic across processes. Only the owner sends notifications. Add a GW2 test with two services.

### M4. R2 deploys a round-5 build onto the live session

**Evidence.** `scripts/install.sh:5` is `exec python3 "$ROOT/scripts/install.py" "$ROOT"` and drops `"$@"`. So
`./scripts/install.sh --system-files` runs a normal user-local install: it replaces `~/.local/bin/win11-*` and the payload in
Robin's running session and installs no system files. `uninstall.py:19-20` rejects any argument except `--files-only`, so
`uninstall.sh --system-files` fails. No step owns `install.sh` (GW5 lists only install.py/uninstall.py). §7 also says
`--system-files` runs *after the user-local install*, so even when fixed, R2 deploys code before R3.

**Fix.** R2 becomes `python3 scripts/install-system.py` (system files only, one pkexec; `--remove` to undo), with no
user-local install. Add `install.sh` `"$@"` passthrough to GE1c's or GW5's file list. Make uninstall's form explicit
(`--system-files-only`), separate from a full uninstall, which disables the extension and writes GSettings.

### M5. Release hand-over: missing states and a window that vanishes

**Evidence.** Ptyxis's entry has `X-ExecArg=--` and **no** `X-TerminalArgHold`; `xdg-terminal-exec:1381-1385` then ignores
`--hold` with only a debug line. do-release-upgrade exits early with a printed reason:
"install all available updates" (`:187-191`), "not rebooted" (`:203-207`), "not possible right now" (`:162-167`). pkexec
also returns 126 when the dialog is dismissed. §6.4 step 5 only knows `running`/`waiting`/gone.

**Consequence.** Robin clicks "Upgrade starten" and a window flashes and closes. The card silently returns to "Ubuntu 26.10 is
beschikbaar". An upgrade that ends without asking for a restart (aborted, or failed after "Restoring original system state")
is also shown as nothing. The post-upgrade `installParts` terminal (§6.6, `--hold`) has the same problem.

**Fix.** Use a fixed wrapper argv: `/bin/sh -c '<constant>' win11-gnome-upgrade <status-file>`. It runs pkexec, then writes
`{rc, endedAt}` to a user-owned file under `$XDG_RUNTIME_DIR/win11-gnome/`, then holds the window
(`read`) when rc ≠ 0. Add card states `startFailed` ("De upgrade is niet gestart" + the rc reason, `retry`) and `ended`
("De upgrade is gestopt voordat hij klaar was" + "Logboek openen" for `/var/log/dist-upgrade/main.log`, which is 0644
under a 0755 dir here). Detect "upgrader never appeared within 120 s". Clear `upgrade.json` in post-upgrade step 6 as well.

### M6. Normal updates can start during a release upgrade

**Evidence.** §4.9/§6.4 step 7 guard only power actions and `shell.restartAll`. `update.check` (RefreshCache), `update.install`
and `update.repair` have no upgrader check. The upgrader rewrites sources and releases the apt lock between phases
(`apt-get update`, download, install).

**Fix.** Every write method returns `update.guard.upgrade` while the §6.4 detection sees the upgrader. Add a hero phase
`releaseRunning` (text of `relRunning`) so the page does not offer "Controleren op updates". Add a fake/fixture test.

### M7. The update-notifier toggle does not do what its text says

**Evidence.** The schema description of `regular-auto-launch-interval`: "Please note that it will auto launch for security
updates immediately." There is no other auto-launch key (the full key list: show-apport-crashes, end-system-uids,
regular-auto-launch-interval, hide-reboot-notification, no-show-notifications, release-check-time, …).

**Fix.** Writing 36500 only stops the 7-day launch for normal updates, so rename the row "Software-updater van Ubuntu
openen voor gewone updates", or say in its description that security updates still open it. If Robin wants it fully off
(R8), the only lever is an autostart `Hidden=true` override, which §4.12 rules out and which also hides reboot and crash
notices. So present that as a Robin decision, not as a default.

### M8. `shell.restartAll` guard misses Software jobs and download-phase vanish

**Evidence.**
* `software.rs:777-787`: `Software::shutdown` **joins** every worker. `runtime.rs:98-104` calls it on every restart. Software
  jobs run in Settings (`pages/installed.js`), the installer and Start (`shell/startmenu.js`), all through
  `core/software.js`. A restart during an uninstall blocks until the PackageKit job ends (2 h cap), so GX1's "within 5 s"
  fails. The panel then SIGTERMs the half-stopped Start (`lib.rs:515, 559-585`).
* packagekitd links `g_bus_watch_name` and has `pk_transaction_cancel_bg`. Whether a vanished caller cancels a
  still-cancellable transaction is **unverified**. If it does, closing Settings or a Settings re-exec during the download
  cancels the update, and C2 ("detach") and the reattach story fail for the download phase.
* The panel SIGTERMs its preloaded Start on shutdown and spawns a new one after exec (`lib.rs:515, 559-585`). GX1's
  "start 600 ms" restart is therefore wasted: Start restarts twice, a second WebKit start-up.

**Fix.** `guard::power_blocked` for `restartAll` also blocks while **any** own update job is active (not only `committing`)
and while any `software` job is `queued|running|authenticating`. Receivers re-check the guard file before re-exec. A
preloaded Start ignores `shell.restartAll` (the panel handles it). Add fake timeline `vanish-cancels-download` and settle it in
R3 (already "close Settings during the download"). Write the C2 fallback now: if R3 shows a cancel, Settings keeps its process
alive (window hidden) until the job ends.

## Minor

* **m1 Ownership: `user_environment` is private.** `files/actions.rs:910` `fn user_environment` (no `pub`). §6.4 calls
  `files::actions::user_environment(true)` from `update/upgrade.rs`. GW7 must edit `files/actions.rs`, which is not in its
  file list. Fix: add the one-line visibility change to GW2's list.
* **m2 Wrong `include_str!` depth.** The catalogues live at `G/i18n/host/` (rust-services.md F5). From `host/src/post_upgrade.rs`
  the path is `../../i18n/host/update.tsv`, not `../i18n/host/update.tsv` (§6.6).
* **m3 WebKit network process comm.** Real comm is `WebKitNetworkPr` (15 chars; pids 7533/7534/7800), not
  `WebKitNetworkP` (§4.8). Match on a prefix or rely on the exe rule. Also: WebKit children here are direct children of the UI
  process (no bwrap), so the PPid walk is one level.
* **m4 Extension state parsing.** `extensionUtils.js:56-66` packs every number as `d`, so `state` arrives as a double (1.0).
  `org.gnome.Shell-disable-extensions.service` sets `disable-user-extensions true` after a flagged crash. Our extension is
  then INACTIVE with `enabled:false` (`extensionSystem.js:550-553`), which the §6.6 rule ("2 while enabled") misses. Fix: read
  `org.gnome.shell disable-user-extensions` and `enabled-extensions`, and offer "Extensies weer aanzetten" for that case.
* **m5 Autostart semantics.** `systemd-xdg-autostart-generator` has no Delay handling (strings: only `X-GNOME-Autostart-Phase`,
  `X-systemd-skip`, `OnlyShowIn`). Generated units are `Type=exec`, `ExitType=cgroup`, `PartOf=graphical-session.target`,
  `TimeoutStopSec=5s`, `Slice=app.slice`. `X-GNOME-Autostart-Delay=15` is ignored, and a terminal spawned directly from that
  unit (installParts) is stopped at logout. Fix: drop the Delay claim, keep the ShellVersion retry, and spawn the terminal via
  `process_scope::spawn` (its own unit).
* **m6 P01–P03 cannot use a fake `org.gnome.Shell.Extensions`** on the harness session bus: the real headless gnome-shell 50
  owns that name, and it must also show the banner. Fix: P02 = a private extension copy with `shell-version ["49"]` (real
  OUT_OF_DATE). P03 = a fixture seam for "missing sonames". P01 = a changed `last-os-release.json`.
* **m7 GW6 fixture vs forced keyring.** §6.2 forces `Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg` "never
  taken from a caller", but GW6 tests a throwaway-key `file:` archive. Add the keyring path to the `FIXTURE_ROOT` re-rooting
  list. Components are forced to `main universe`, while `ubuntu.sources` here has `main restricted universe multiverse`:
  derive them from the file. "tmp GNUPGHOME" must live under `tests/update/.run/` (plan: never /tmp).
* **m8 Compat cost vs value (C13).** `releases.json` marks stonking `untested`, so the verdict is `hold` whatever the archive
  says. The 15–20 MB index download and 250–400 MB apt RSS run weekly for a known hold. Fix: run the apt-tree part only when
  the cheap verdict is not already `hold` (GNOME major in `shell-version` **and** status `tested`). Keep the full check for
  future versions. This also lowers GW6 risk.
* **m9 Reader mode makes a D-Bus call.** The power guard "makes at most one NO_AUTO_START call" (§4.9) and runs in the panel
  (`system/power.rs`), contradicting "reader mode: no D-Bus" and GW2's "panel-kind never connects" test. Fix: files plus
  owner liveness only. If the owner is dead and the job unfinished in this boot, block with `update.guard.power`.
* **m10 Count flapping.** `info.rs:305-321` still runs `apt list` (counts phased) in processes whose snapshot is older than
  30 min and broadcasts `system.updates.changed`. The snapshot path counts non-phased. Home/Taakbeheer then alternate between
  two numbers. Fix: once a snapshot exists, the apt path never emits. Show the snapshot with its `checkedAt`.
* **m11 Progress fan-out.** `events.rs:160-200` sends every event to every `*.sock` and every SSE subscriber. Four
  `update.progress` per second wake panel, desktop, Start, Explorer and their WebViews. Fix: `emit_local` for `update.progress`
  (only Settings consumes it).
* **m12 Attach helper with `auth_admin_keep`.** The upgrader runs `screen -e \0\0` (`DistUpgradeMain.py:184-187`). Ctrl+Space
  sends NUL, so `Ctrl+Space c` opens a **root shell** inside the attached session. polkit keeps temporary authorizations per
  session, so for 5 min any process of Robin's session can re-run the helper silently. Fix: `auth_admin` (no keep) for
  `release-upgrade-attach`. Keep `_keep` only for the enum-only Prompt helper.
* **m13 Fidelity of texts.**
  * Row state `restart`: Windows 11 says "Pending restart", nl likely "Wachten op opnieuw opstarten", not "Restart required"
    (the hero keeps "Opnieuw opstarten vereist").
  * Error hero en: Windows reads "There were problems installing some updates, but we'll try again later." (the design has
    "There were some problems installing updates").
  * `advNotify` nl is cut short ("…vereist is") while en is the full Windows sentence: use "Melden wanneer opnieuw opstarten
    vereist is om het bijwerken te voltooien".
  * `rowSecurity {version}` from os-release `VERSION` yields "26.04.1 LTS (Resolute Raccoon)". Strip the parenthesised
    codename.
  * GW8 must still confirm these against a real 24H2 nl/en reference.
* **m14 False "busy".** The §6.4 comm scan includes `apt`, and every Win11 host runs `apt list --upgradable` (`info.rs:306`,
  comm `apt`). Fix: test the dpkg locks via `/proc/locks` against the inodes of `/var/lib/dpkg/lock-frontend` and `lock`
  (both readable without root).
* **m15 Simulate classification.** The typelib also has `DOWNGRADING 20`, `INSTALL 27`, `REMOVE 28`, `OBSOLETE 29` and
  `DOWNGRADE 30`. Treat 20/28/29/30 as confirm-worthy next to 13/15, and add 28 to `removals-in-simulate`. Map exit
  `CANCELLED_PRIORITY 9` / error 65 and `PACKAGE_INSTALL_BLOCKED 39` explicitly.
* **m16 Sizes.** The apt backend imports only `pk_backend_job_details` (no `_full`), so expect no `download-size` in `Details`.
  Plan `rowCaptionNoSize` as the normal case and take the total from the simulate's `DownloadSizeRemaining`.
* **m17 "Foreign" detection.** The Transaction interface has no sender property. Every tid that is not ours is foreign, and that
  includes our own Software installer, for which "een ander programma" is slightly wrong.
* **m18 R3 restart path.** Round 1 changes the extension (GE1c). A deploy then needs a re-login, not "Win11 Shell opnieuw
  starten", or the new binaries talk to the old loaded extension.
* **m19 GW3 done-when.** "nl dump differs from the baseline only by §3.9 rows" is impossible once dozens of new nl keys render
  on the page. Define an additions list (new keys) next to `BASELINE-CHANGES` (changed keys).
* **m20 Ownership gaps.** No step owns `tests/i18n/surfaces.json` for GW4/GW6's new mock states. No step owns the Prompt-row UI
  states M01–M04 (`update-release.js` belongs to GW6, whose done-when lists only C01–C13). GW2's done-when needs GW1's fake
  inside the same round, so they run serially (schedule risk).
* **m21 Test filter.** `cargo test … update:: -- --ignored` is a substring filter. Use `services::update::bus_tests::`.
* **m22 Harness memory.** The extension also starts panel, desktop and preloaded Start (three more WebKit processes). 1.0–1.3 GB
  is low. Plan for about 1.6–2.0 GB, or have GE0c start only Settings.
* **m23 Plan rule for GW6.** `apt-get update`/`-s dist-upgrade` in a private tree (even `file:` in `bwrap --unshare-net`) is
  literally "apt", which the plan forbids for agents. The coordinator must carve this out in plan.md before GW6.
* **m24 Smaller semantics.**
  * NM `Metered` GUESS_YES (3) counts as metered.
  * `LIMITED`/`PORTAL` connectivity should not show "Je bent online".
  * `installParts` installs old-release package names, so after a t64-style rename `apt-get install` fails. Resolve names on
    the new release.
  * Showing the Win11-restart InfoBar while a reboot is pending is right when the WebKit version changed: new web processes of
    old UI processes break now, and the reboot may be days away (today it has been pending since 06:54). C3 tweak.

## Verified correct (no change needed)

* Cited code exists as stated, ±2 lines:
  * `system.rs:468`, `info.rs` Updates, `update.js:118-128`, `power.rs:39, 45-49, 81-85, 100-118`;
  * `packagekit.rs:43-258` (20 ms loop, `locale=C.UTF-8`, 2 h cap), `software.rs:91-109, 501-523, 777-787`;
  * `services/mod.rs:62-69, 94-134, 203-253`, `events.rs:20, 160-200, 248, 337-357` (no `update.` prefix);
  * `runtime.rs:531-545`, `cleanup.rs:661-735`, `actions.rs:531-685`;
  * `process_scope.rs:17-20, 28-40, 195-300, 311-333`, `shell.rs:213-224`, `lib.rs:336-352, 401-415, 479-486, 636-643`;
  * `extension.js:150-169`, `health.rs:316-334`, `install.py:93-99`, `settings/main.js:118-119`, `mock/system.js:102, 696`;
  * README libarchive note (at :74).
* Remote events reach `Events::on` observers (`events.rs:105-135` → `dispatch`), so GX1's cross-process event works. The
  desktop is identified by GTK app id (`desktopBridge.js:17`), so a same-pid re-exec is re-adopted.
* The enums match the local typelib exactly: Role, Status, Info, Exit, Filter (NONE = 1 → bitfield 2), TransactionFlag
  (ONLY_TRUSTED 1, SIMULATE 2 → 2 and 6), and the error codes used.
* The polkit defaults in §1.2. `com.ubuntu.release-upgrader.release-upgrade` has `exec.path=/usr/bin/do-release-upgrade`,
  `auth_admin` only for active. The apt backend implements `pk_backend_repair_system` and uses `--force-confdef/confold`.
* The do-release-upgrade rules and line numbers. `check-new-release` is a symlink to `do-release-upgrade` (0 = available,
  1 = none; `UpgradeBroken` also gives 1, so `ubuntuHold` must come from the cache, as designed). Also verified: the screen
  name, `needs_screen`, `PKEXEC_UID`, the Prompt/`.d` precedence and "lts on non-LTS = normal" (`MetaRelease.py:147-173`).
  The upgrader takes a logind *sleep* inhibitor and reboots with `--check-inhibitors=no`.
* `pkexec` keeps `LANGUAGE`/`LC_*`/`COLORTERM`. `TERM` is tail-merged into `COLORTERM` in `strings` output, so it is kept too.
* The GNOME 50 out-of-date rule (`extensionSystem.js:409-412`, prefix match). PackageKit history entries do carry
  `Requested-By: robin (1000)`.

## Not checked

* PackageKit's behaviour when a caller vanishes (M8). It needs R3 or the source.
* Real Windows 11 24H2 strings (from memory; GW8 reference set).
* The GNOME logout effect on `vte-spawn` scopes (R5). The 26.10 archive (no network).

## Decisions for the coordinator / Robin

1. B1 fix as a precondition for GW1/GW2/GE0c (fail-closed seams, harness-wide system-bus redirection and guard-bin). Coordinator.
2. M2: write `check-new-release-ignore` while on hold, or recommend `Prompt=lts`, or leave Ubuntu's offer alone. Robin.
3. M3: owner election in Settings (recommended) or make Settings single-instance. Coordinator.
4. M4: R2 becomes `python3 scripts/install-system.py`, and `install.sh` gets an args passthrough owner. Coordinator.
5. M7: honest toggle text (default) or an autostart override (Robin, R8).
6. m8/C13: run the apt-tree check only when the cheap verdict is not `hold`. Coordinator.
7. m23: an explicit plan.md exception for the private-tree apt runs in GW6, or drop them. Coordinator.
