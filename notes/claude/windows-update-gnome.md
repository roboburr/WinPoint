# Windows Update for Win11 voor GNOME: buildable design (GE0d, 2026-11-05)

Design only. Nothing outside this file was changed. G = this repo (`claude/gnome-english-update`, base `3b898e6`),
H = the frozen Hyprland tree (`.../scratch-2026-10-25-a089b6`), W = Codex's test dirs. Paths without a prefix are
relative to G. This note replaces H's `notes/windows-update-design.md` for G: it keeps H's states, texts and flow
where they carry over (`updates.md` F3, `gnome-map.md` §3) and rewrites everything that touches processes, files,
polkit, the compatibility check and the hand-over for this codebase. Decisions D-G1..D-G14 of `plan.md` apply;
D-G12 is implemented as written, and every place where this note reads or extends it is listed in §12.

Read-only checks made for this note (2026-11-05, ~13:10): `dpkg-query` (versions in §1.2), the polkit action XML
and rules in `/usr/share/polkit-1`, `/etc/update-manager/release-upgrades{,.d}`, both meta-release caches,
`/var/lib/ubuntu-release-upgrader/`, `/run/reboot-required*`, `/var/log/apt/history.log`, `/usr/share/distro-info/
ubuntu.csv`, the shipped D-Bus interface XML of PackageKit and `org.gnome.Shell.Extensions`, the GNOME Shell 50
`extensionUtils.js`/`extensionSystem.js` (via `gresource extract`, read-only), `/usr/bin/do-release-upgrade`,
`DistUpgradeMain.py`, `DistUpgradeController.py`, `check-new-release`, the update-notifier schema,
`xdg-terminal-exec`, `org.gnome.Ptyxis.desktop`, `locale -a`, and a `(deleted)` grep of `/proc/<pid>/maps` of
gnome-shell, win11-panel/desktop/start and their WebKitWebProcess children. No PackageKit call (not even a property
read), no apt, pkcon, do-release-upgrade, gsettings set, busctl or build. Nothing launched; nothing signalled.

*[review]* **Revised 2026-11-05 after the adversarial critique** (`notes/claude/windows-update-gnome-critique.md`:
1 blocker, 8 major, 24 minor). Every changed place carries the mark *[review]*; §15 lists each finding and what was
done. Extra read-only checks for the revision: `tests/gnome/harness.py` as it stands after round 0 (bwrap layer,
private system bus, stub list), `tests/rust-sandbox/run-tests.sh` and its `guard-bin/`, `scripts/install.sh`,
`scripts/uninstall.py`, `host/src/{lib.rs, services/shell.rs, services/software.rs, files/actions.rs, events.rs,
backend.rs, bin/launch.rs, process_scope.rs, services/system/info.rs}`, `/usr/bin/do-release-upgrade:175-207`,
`check-new-release-gtk:97-115`, `UpdateManager.py:493-512`, the update-manager and update-notifier schemas,
`strings` on `libapt-private.so` and `/usr/bin/bwrap`, and the PackageKitGlib typelib plus `apt_pkg` attribute
names through `python3 -I` (no D-Bus, no cache opened). Still no PackageKit call, apt run, build or launch.

**Updated 2026-11-13: WinPoint without WebKit.** The WebKit hosts and the page code under `vendor/ui/js` are gone
(the last commit with them is tagged `last-webkit`: `git show last-webkit:vendor/ui/js/apps/settings/pages/update.js`).
The page is the native Settings (`native/apps/settings/src/pages/update*.rs`), and the owner of the update service is
its backend `win11-app --app settings --http-only`. Rewritten to that: §3.4 (the restart InfoBar and its component
names), §4.1 (the keep-alive), §4.8 (the scan, without WebKit children and without the history fallback) and the
blocking parts of §6.2 and §6.6 (GTK 4 instead of WebKitGTK, JavaScriptCore and libsoup). The `webkit` and `soup`
rows of §3.3 stay: they group Ubuntu's packages, which other apps use; the native page titles them neutrally
(`WebKitGTK {version}`, `libsoup {version}`). Everything else, including the "today" observations, is the design as
it was built in GW1–GW9.

**Updated 2026-11-14 (D-G73): the phasing classifier without Python.** `phasing.rs` no longer runs a python3-apt
script (`git show last-webkit:host/data/phasing.py`). It asks apt itself, with three read-only programs from the tool
table, in this order: `apt-cache policy ?phasing` (apt's own per-machine decision), `apt-cache policy -o
APT::Cmd::Pattern-Only=true <name:arch>…` and `apt-mark showhold`. `Tool::Python3` and `probe.py --phasing` are gone, and so is vendor/host, the Python reference host
(`git show last-webkit:vendor/host/<path>`). Rewritten to that, each place marked *[D-G73]*: §0 (the phasing line),
§1.1 (the cleanup-helper template), §4.2 (the `phasing.rs` row), §4.6 (phasing and held packages), §4.11 (the stub
list), §6.4 (the `updatesPending` rule), §9.1 (the phased-as-normal timeline, probe.py), §9.2, §9.4, §11 R1 and §14. §1.2's observations and §15's
review table stay as they were written.

---

## 0. Summary

* **What gets built.** A new Rust service `update.*` (`host/src/services/update.rs` + `update/*.rs`) that runs its
  PackageKit work only in **the one** Settings host that holds `update/owner.lock` (`win11-app --app settings`,
  kind `settings`; Settings is `NON_UNIQUE`, so a second Settings process is a *mirror* that forwards its writes to
  the owner) *[review]*. Other hosts load it in *reader mode*: files only, no D-Bus. The Settings page "Update en herstel" (`update.js`) becomes Windows 11 24H2's
  Windows Update: one hero with one accent button, update rows with "Downloaden - 45%" and a 160 px ProgressBar,
  restart states, a feature-update card "Ubuntu 26.10 is beschikbaar", "Meer opties" with the LTS/normal choice,
  Updategeschiedenis and Geavanceerde opties.
* **Flow.** "Controleren op updates" = `RefreshCache(false)` → `GetUpdates(none)`. "Alles downloaden en
  installeren" = `UpdatePackages(SIMULATE|ONLY_TRUSTED)` → optional confirm (removals, metered) →
  `UpdatePackages(ONLY_TRUSTED)`. The client is event-driven (signals and `PropertiesChanged` on its own
  `MainContext`/`MainLoop`), with no 20 ms poll and no 2-hour cap. Robin needs no password for updates
  (`org.freedesktop.packagekit.rules`, verified). *[review]* PackageKit's apt backend has no phasing logic, so the
  service classifies phased updates itself with apt's own rule (*[D-G73]* `apt-cache policy ?phasing`, §4.6) and
  never passes them to `UpdatePackages`.
* **Shared state is files** in `~/.cache/win11-gnome/update/` (snapshot, job with boot id, lock, release, compat),
  so a crashed or restarted Settings reattaches through `GetTransactionList` (NO_AUTO_START) or
  `GetOldTransactions`. Threads are detached, never joined. *[review]* Closing the Settings window during an own job
  closes the window but keeps the process (no window, about 30–50 MB) until `Finished`, because PackageKit may
  cancel a transaction whose caller vanishes (unverified, R3).
* **Restart detection** has three inputs: a `gio::FileMonitor` on `/run/reboot-required`, PackageKit's
  `RequireRestart`, and a needrestart-style scan of `(deleted)` mappings. A replaced gnome-shell or mutter gives the
  hero "Opnieuw opstarten vereist" (D-G12/D6). Replaced WebKitGTK, JavaScriptCore, GTK, libsoup, GLib or one of our
  own `win11-*` binaries gives the InfoBar "Win11 Shell moet opnieuw starten" with a button that restarts **all**
  Win11 processes (new `shell.restartAll`, GX1). This is live today: gnome-shell maps a deleted `/usr/bin/gnome-shell`,
  and every Win11 process maps a deleted `libsoup-3.0.so.0.7.4`.
* **Release upgrades** use only Ubuntu's data and code. Detection reads `Prompt=`, the motd stamp, both meta-release
  caches and distro-info; only a click on "Controleren op updates" runs `check-new-release -q`, at most daily (no
  run on page open) *[review]*. The compatibility check compares the **GNOME Shell major of the candidate release
  with the installed extension's `shell-version`** (the decisive test, today `["50"]`). When `releases.json` already
  says hold, nothing is downloaded *[review]*; otherwise it also checks our binaries' NEEDED sonames, libarchive and
  a `dist-upgrade` simulation in a private apt tree with its own `APT_CONFIG`. An untested GNOME major holds the card
  with "Wacht op een compatibele Win11-versie" (D-G12). *[review]* Ubuntu's own Software Updater would still offer
  the upgrade; the hold card says so and offers to postpone Ubuntu's own prompt as well (two user GSettings keys,
  reversible, only on a click; Robin decides whether that becomes automatic, C14).
* **Hand-over.** `xdg-terminal-exec -- /bin/sh -c '<constant wrapper>' win11-gnome-upgrade <status> pkexec
  do-release-upgrade` *[review]* is started through the `process_scope::spawn` path (Role::App → `app.slice`, never
  `app-win11shell.slice`). The wrapper records pkexec's exit code and keeps the window open on failure (Ptyxis has no
  `--hold`), so the card can show "De upgrade is niet gestart" or "De upgrade is gestopt voordat hij klaar was".
  Ubuntu's text frontend runs itself under GNU screen. Linger is turned on temporarily and turned off only if we
  turned it on. While the upgrader runs, every update write is refused *[review]*. "Venster weer openen" uses a root
  attach helper (`screen -d -r`, always a password *[review]*). After the upgrade, an XDG autostart entry runs
  `win11-launch --post-upgrade-check` (GIO only, independent of the extension and of WebKit). It reads the
  extension state over `org.gnome.Shell.Extensions` and offers "Ubuntu Dock aanzetten" through a GNOME notification.
* **Root files.** Two helpers and two polkit policies ship in `host/data/`. A new, standalone
  `scripts/install-system.py` installs them in one `pkexec` step and nothing else (`python3 scripts/install-system.py`;
  `--remove` undoes it) *[review]*. Without them, the Prompt change falls back to `pkexec /bin/sh -c`, and attach
  shows the manual command.
* **i18n.** The UI uses `makeT` in `update.js`, `update-release.js`, `update-history.js` and `update-advanced.js`.
  The host uses `i18n/host/update.tsv` through GE1b's `i18n.rs`, read by `win11-launch` with a 20-line local lookup.
  The policies carry `xml:lang="nl"` plus default English. Job and snapshot files store codes, never rendered text.
* **Tests.** A fake PackageKit (Python + Gio, no new crate) plus a fake NetworkManager runs on a private
  `dbus-daemon` without service dirs, with `NO_AT_BRIDGE=1`. It has a control interface and 31 timelines *[review]*.
  All 58 page states *[review]* are screenshotted in nl and en from the real host in GE0c's headless GNOME harness
  (`tests/gnome/`), and every state is also in the mock for the i18n dumps. *[review]* **Fail closed:** every test
  that runs Win11 or update code runs inside a bubblewrap layer with a private `/run/dbus`, no network and stubs first
  on PATH (GE0c's layer as built); a test seam that fails its check makes the service do nothing (`test_seam`), and
  every tool resolves through one table that points only at stubs in test mode (§4.11). This is a precondition for
  GW1, GW2 and GW8.
* **Effort: GW1–GW8 + GX1 ≈ 104 agent-hours** *[review]* (Phase A: GW1–GW4 + GX1 ≈ 58 h; Phase B: GW5–GW7 ≈ 37 h;
  GW8 9 h). Optional GW9 is 6 h. The extension port to GNOME 51 (GX2, 4–16 h) is a precondition for an "ok" verdict
  on 26.10, not part of this feature. Robin-only actions: R1–R10 (§11).

---

## 1. What exists (verified today)

### 1.1 In G

| Area | Fact | Where |
|---|---|---|
| Count only | `system.updates` = `apt list --upgradable` in a thread, TTL 1800 s, `security` = lines with `-security`, emits `system.updates.changed` only when `available` changes | `host/src/services/system/info.rs` `Updates` (`get`, `check`), `system.rs:468` |
| "Controleren" | calls `system.updates` without `refresh`, so it re-reads the cache and never refreshes apt | `vendor/ui/js/apps/settings/pages/update.js:118-128` |
| Software-updater | `system.openUpdater` tries update-manager → gnome-software → snap-store through `launch::spawn`; the error text is Dutch only | `system/power.rs:100-118` |
| Page | hero `.st-upd` (64 px disc, title, caption, two buttons), then backup/reset/about; `makeT` nl/en at `update.js:12-91`; CSS `settings.css` "Update hero" block; registry `P('update', …)` and aliases `updates`, `backup`, `reset`, `recovery`, `windows-update`, `windowsupdate` | `update.js`, `settings.css`, `registry.js`, `main.js` `MODULES.update` |
| PackageKit client (software) | one transaction per call on a fresh `MainContext`; SetHints `interactive=true`, `locale=C.UTF-8` hard-coded; `Package`/`Packages`/`ErrorCode`/`Finished`; `Percentage`/`AllowCancel`/`Status`; `GetAll` every 1 s inside a 20 ms sleep loop; 2 h cap; Cancel only while AllowCancel; flags `4` (SIMULATE) then `0` | `services/software/packagekit.rs:43-258`, `software.rs:704-772` |
| Jobs | files with owner `{pid, start}` (`/proc/<pid>/stat` field 22); `interrupted` when the owner died; no reattach; `Software::shutdown` **joins** workers | `software.rs:91-109, 501-523, 777-787` |
| Services | `LazyService` per process, routing by prefix, `ctx.language()` read fresh per call, `ctx.dry_run` | `services/mod.rs:62-69, 94-134, 203-253` |
| Events | Unix datagrams to every `*.sock` in the runtime dir, 60 KiB limit, prefix allowlist **without** `update.` | `events.rs:20, 160-200, 334-362`; `Events::on` observers `:248` |
| API registry | 172 methods with `dangerous`; dangerous + dry host → envelope `dryRun:true` (the method itself must avoid effects) | `api-methods.json`, `runtime.rs:524-546` |
| pkexec precedent | root-owned helper check (file and parent uid 0, no g/o write), else `pkexec /bin/sh -c <constant> win11shell-cleanup <args>`; rc 126/127 → "geen wachtwoord"/"wachtwoord klopte niet"; markers `@@ok`/`@@fail`; helper path `/usr/libexec/win11shell/cleanup-helper` is **not installed** by `install.py` | `services/storage/cleanup.rs:661-735`; template *[D-G73]* `git show last-webkit:vendor/host/data/{nl.win11shell.cleanup.policy, win11shell-cleanup-helper}` (vendor/host is deleted) |
| Terminal precedent | `default_terminal()` (xdg-terminals.list, then Ptyxis, GNOME Terminal, Console, …), `terminal_command`, sanitised `user_environment(true)`, activation token, spawn through `process_scope::spawn(…, "file-open", Some(env))`. *[review]* `user_environment` is private (`fn`, no `pub`), so `update/upgrade.rs` needs a one-line visibility change (GW7) | `host/src/files/actions.rs:531-685, 910-916` |
| Single instance? *[review]* | every kind except start/panel/desktop/alles gets `ApplicationFlags::NON_UNIQUE`, so each Settings launch is a new process; `shell.openApp` always spawns (Taakbeheer's health item opens `settings` `page:update`) | `lib.rs:295-301`, `services/shell.rs:177`, `sysmon/health.rs:333` |
| Preloaded Start *[review]* | the panel prewarms Start as its child, SIGTERMs it on its own shutdown and prewarms a new one after it starts again | `lib.rs:515, 559-585` |
| Installer entry *[review]* | `scripts/install.sh:5` is `exec python3 "$ROOT/scripts/install.py" "$ROOT"` and **drops all arguments**; `uninstall.py:19-20` accepts only `--files-only` | `scripts/install.sh`, `scripts/uninstall.py` |
| apt count *[review]* | `Updates::check` counts `apt list --upgradable` lines, which include phased packages, and emits `system.updates.changed` when the number moves | `services/system/info.rs:305-321` |
| Test layers *[review]* | GE0b: `env -i`, dead `DBUS_SYSTEM_BUS_ADDRESS`, `guard-bin/` first on PATH, no bwrap (`tests/rust-sandbox/run-tests.sh:9-13, 87-106`). GE0c as built after round 0: everything inside `bwrap --ro-bind / / --tmpfs /run/dbus --tmpfs /run/user --unshare-pid --unshare-net --die-with-parent --new-session`, an empty private system bus at `RT/system-bus` (allow-all policy, no service dirs), stubs first on PATH (pkexec, sudo, su, systemctl, systemd-run, loginctl, busctl, gnome-session-quit, gsettings, dconf, apt, apt-get, pkcon, do-release-upgrade, update-manager, gnome-software, snap, shutdown, reboot, …), and a 56-case guard self-test | `tests/gnome/harness.py:76-115, 222-243, 404-414, 1016-1023`, `tests/gnome/baseline/guard-selftest.json` |
| Placement | panel/desktop/start → `app-win11shell.slice` (MemoryMax 1000 MiB, High 768 MiB); every other kind and `process_scope::spawn` → `app.slice` (no cap). Each process is a transient `app-win11-<tag>-….service` (`Type=exec`, `ExitType=cgroup`) | `process_scope.rs:17-20, 28-40, 195-300, 311-333` |
| Restart | `shell.restart` (dangerous) → `request_shutdown(true)` → after `runtime.shutdown()` the process `exec`s `current_exe()` with its own argv. **Latent bug:** after our binaries are replaced, `current_exe()` returns `…/win11-x (deleted)` and the exec fails | `services/shell.rs:213-224`, `lib.rs:336-352, 479-486, 636-643` |
| Extension supervision | starts the desktop (`desktopBridge.launch()`) and the panel (`open_new_window`) once; no respawn; shows its St fallback bar while the panel is absent | `extension.js:150-169` |
| Notifications | GNOME's own server owns `org.freedesktop.Notifications`; the extension mirrors into the Win11 centre; `notifications.test` goes through the extension | `services/notifications.rs:1, 441-447` |
| win11-launch | GIO-only binary (NEEDED: libgobject, libgio, libglib, libgcc_s, libc; checked on `target/debug/win11-launch`), one JSON argument | `host/src/bin/launch.rs` |
| Settings deep link | `--params {"page":"update"}` → query `page=update` → `mountApp(params)` | `lib.rs:401-415`, `settings/main.js:119` |
| Mocks | `mock/system.js:102` `updates:{available:3,…}`, `:696-697`; `mock/index.js` FILES list (no `update`) | |
| Absent | grep: no `reboot-required`, `RequireRestart`, `do-release-upgrade`, `meta-release`, `update.` event prefix in `host/src`, `vendor/ui/js`, `extension` | |

### 1.2 On this machine (read-only, today)

| Fact | Value | Consequence |
|---|---|---|
| Packages | packagekit 1.3.4-3ubuntu1.2 (D-Bus activated), gir1.2-packagekitglib-1.0 1.3.4, no packagekit-tools; gnome-shell 50.1-0ubuntu1.5, libmutter-18-0 50.1-0ubuntu2.5; ubuntu-release-upgrader-core/-gtk 1:26.04.27; update-manager 1:26.04.5; update-notifier 3.207.2; unattended-upgrades 2.12ubuntu9; screen 4.9.1; ptyxis 50.1; xdg-terminal-exec 0.14.0; bubblewrap 0.11.1; dbus-daemon 1.16.2 (no dbus-broker); libwebkit2gtk-4.1-0 / libjavascriptcoregtk-4.1-0 2.52.6; libgtk-3-0t64 3.24.52; libsoup-3.0-0 3.6.6-1ubuntu0.2; libglib2.0-0t64 2.88.0; libarchive13t64 3.8.5; distro-info-data 2026.10.12; no language-pack-nl | Everything the design needs is present; `screen`, `xdg-terminal-exec`, `bwrap`, `dbus-daemon` exist |
| Reboot flag | `/run/reboot-required` + `.pkgs` = `gnome-shell` (06:54, unattended-upgrades: gnome-shell 1.4 → 1.5) | The first real view shows "Opnieuw opstarten vereist" |
| Stale mappings | gnome-shell 6196: deleted `/usr/bin/gnome-shell`, `libshell-18.so`, `libst-18.so`, `libgvc.so`, `libgnome-shell-menu.so`, gvfs/gio modules. win11-panel 7256, win11-desktop 7257, win11-start 7648 and WebKitWebProcess 7535/7536/7801: deleted `libsoup-3.0.so.0.7.4` (unattended-upgrades 06:53:43) and `libpcre2-8.so.0.14.0`, gvfs modules | Watch lists, not deny lists (§4.8). Today both the hero restart state and the Win11 restart InfoBar apply; the reboot covers both, so only the hero shows |
| polkit | packagekit `system-update` auth_admin_keep + rules.d YES for active local sudo/wheel; `system-sources-refresh` yes (active); `upgrade-system` auth_admin; `repair-system` auth_admin; `cancel-foreign` auth_admin_keep; `com.ubuntu.release-upgrader.release-upgrade` auth_admin, `exec.path=/usr/bin/do-release-upgrade`, `allow_gui=true`; login1 `set-self-linger` yes, `inhibit-block-idle` yes, `inhibit-block-sleep` yes (active), `reboot` yes (active), `reboot-ignore-inhibit` auth_admin_keep. *[review]* polkitd 127 imports `sd_uid_get_display` next to `sd_pid_get_session`: a uid-1000 process outside any session is judged as Robin's active local seat0 session | No password for updates or linger; always a password for release upgrade and our helpers. *[review]* A test process that reaches the real system bus gets the same password-less answers, so tests must not be able to reach it at all (§4.11) |
| PackageKit apt backend *[review]* | `strings libpk_backend_apt.so \| grep -i phas` is empty and it imports no `PhasingApplied`; it imports `pk_backend_job_details` but not `_full`. apt 3.2.0 does phasing in its upgrade logic (`pkgDepCache::PhasingApplied`; apt-private prints "Not upgrading yet due to phasing:"), python3-apt 3.1 has `DepCache.phasing_applied` | GetUpdates very likely lists phased packages as normal updates; Details very likely has no `download-size` (§4.6) |
| Release data | `Prompt=normal`; `release-upgrades.d/ubuntu-advantage-upgrades.cfg` has no Prompt. distro-info: 26.10 Stonking Stingray released 2026-10-15, EOL 2027-07-15; 26.04 LTS EOL 2031-05-29. User meta-release cache (28 Oct) and root copy `/var/lib/update-manager/meta-release` (25 Oct): newest `resolute 26.04.1 LTS`, **no stonking**. Stamp `/var/lib/ubuntu-release-upgrader/release-upgrade-available` empty (28 Oct) | Today: "released but not offered", so no card (§3.5) |
| do-release-upgrade | refuses while a non-phased upgradable package exists (`Phased-Update-Percentage` rule, lines 175-190) and while `reboot-required.pkgs` lists `libc6`, `linux-base` or `linux-image-*` (192-206); runs `xhost +si:localuser:root` for GTK on Wayland (217-218). The text view has `needs_screen = True`; the screen name is `ubuntu-release-upgrade-screen-window` (`DistUpgradeMain.py:164`) | `updatesFirst`, `rebootFirst` and attach exactly as Ubuntu does them |
| Upgrader session bus | `DistUpgradeController` finds the invoking user via `SUDO_UID`/`PKEXEC_UID` and inhibits `org.freedesktop.ScreenSaver` on that user's bus itself | No idle inhibitor of ours needed |
| Locales | `locale -a`: C, POSIX and en_* only (no `nl_NL`); `LANG=en_US.UTF-8`; Dutch `.mo` for apt, libapt-pkg, debconf, update-notifier, but not for ubuntu-release-upgrader | PackageKit locale hint only when generated; "first lines English" notice in nl |
| pkexec env | `pkexec` binary keeps `LANGUAGE`, `LINGUAS`, `LC_MESSAGES`, `COLORTERM` (strings) | `LANGUAGE=nl:en` reaches the upgrader |
| apt config | Post-Invoke hooks in `15update-stamp`, `20dbus`, `20packagekit`, `50appstream`, `50command-not-found`, `99update-notifier` (+ `20apt-esm-hook.conf`); 10 third-party source files | The private tree needs its own `APT_CONFIG` (§6.2) |
| Terminal | `xdg-terminal-exec` supports `--title=`, `--dir=`, `--hold`; Ptyxis entry has `X-ExecArg=--`, `X-TerminalArgTitle=--title`. *[review]* Ptyxis has **no** `X-TerminalArgHold`, so `xdg-terminal-exec:1381-1385` drops `--hold` with only a debug line | One fixed argv works for any default terminal, but the hold must be ours: a small constant wrapper (§6.4) *[review]* |
| Extensions D-Bus | `org.gnome.Shell.Extensions.{GetExtensionInfo, EnableExtension, ListExtensions}`; `ExtensionState` ACTIVE 1, INACTIVE 2, ERROR 3, OUT_OF_DATE 4; out of date = no `shell-version` entry with the running major as prefix. *[review]* `extensionUtils.js:56-66` packs every number as `d`, so `state` arrives as a double (1.0). After a flagged crash `org.gnome.Shell-disable-extensions.service` sets `org.gnome.shell disable-user-extensions true`; our extension is then INACTIVE with `enabled:false` (`extensionSystem.js:550-553`) | The post-upgrade check reads the state as a number and also reads `disable-user-extensions` and `enabled-extensions` (§6.6) *[review]* |
| Ubuntu Dock | `ubuntu-dock@ubuntu.com` from `gnome-shell-ubuntu-extensions`, disabled here (updates.md F2) | The safety net offers to enable it |
| update-notifier | autostart `NotShowIn=KDE;`, delay 60 s; schema keys `regular-auto-launch-interval` (int, default 7), `hide-reboot-notification`, `no-show-notifications`, …. *[review]* The key's own description: "Please note that it will auto launch for security updates immediately"; there is no other auto-launch key | D2 toggle in §3.6 covers regular updates only, and says so *[review]* |
| Ubuntu's own release offer *[review]* | `check-new-release-gtk:97-107` (started by update-notifier) exits silently only when `com.ubuntu.update-manager check-new-release-ignore` equals the new codename (and the current release is still supported). `UpdateManager.py:503-512` shows `DistUpgradeDialog` with an upgrade button when `check-dist-upgrades` (deprecated key, default true) is true. Both are user GSettings | Our "hold" card does not stop Ubuntu from offering 26.10; §3.5 offers to postpone Ubuntu's prompt too |
| Autostart *[review]* | `systemd-xdg-autostart-generator` knows `X-GNOME-Autostart-Phase`, `X-systemd-skip` and `OnlyShowIn`, but no Delay; generated units are `Type=exec`, `ExitType=cgroup`, `PartOf=graphical-session.target`, `TimeoutStopSec=5s`, `Slice=app.slice` | `X-GNOME-Autostart-Delay` is ignored; a child spawned inside that unit is stopped at logout (§6.6) |
| apt history | `/var/log/apt/history.log` 0644 plus `.1-.4.gz`; unattended entries have `Commandline: /usr/bin/unattended-upgrade` and no `Requested-By` | History parser fixtures |

---

## 2. Goals and non-goals

**Goals.** The page looks and behaves like Windows 11 24H2 Windows Update. Ubuntu's mechanisms do the work:
PackageKit and polkit for updates, do-release-upgrade for releases. No sudo, no own upgrader, no own HTTP client.
A closed Settings window, a Win11 restart or a gnome-shell crash never breaks an update. A release upgrade
survives them while linger holds (verified only in R5). Nothing real happens in dry-run or in tests. nl and en from
the first commit, with Dutch elsewhere byte-identical. No extension change is needed, so the feature itself needs no
GNOME re-login.

**Non-goals.** No automatic installs (unattended-upgrades does security updates). No pause, active hours, scheduled
restart, offline updates (PackageKit Offline), Insider programme, own polkit agent (UAC look), installing arbitrary
packages, or snap updates of our own (snapd refreshes itself). No background watcher in v1 (D-G12/D5).

---

## 3. The page: Windows 11 24H2 fidelity

### 3.1 Layout (page id `update`, title stays "Update en herstel" / "Update & recovery")

```
┌ Update en herstel ──────────────────────────────────────────────────────────────────────────┐
│ ( ⟳✓ )  Je bent up-to-date                                        [ Controleren op updates ] │ hero .st-upd
│         Laatst gecontroleerd: 2 uur geleden (05-11-2026 11:02)                               │
│ ⚠ InfoBar: Win11 Shell moet opnieuw starten … [Win11 Shell opnieuw starten]                  │ §3.4
│ ┌ [Ubuntu] Ubuntu 26.10 is beschikbaar ─────────────────────────────────────────────────────┐ │ §3.5 card
│ │ De nieuwste versie van Ubuntu … tot juli 2027.                [Downloaden en installeren] │ │
│ │ ✓ Win11 Shell werkt met Ubuntu 26.10 · Bekijk wat er in deze update zit                   │ │
│ └───────────────────────────────────────────────────────────────────────────────────────────┘ │
│ ┌ Beveiligingsupdates voor Ubuntu 26.04.1 LTS                Downloaden - 45%            ˅ ┐   │ rows §3.3
│ │ 12 pakketten                                                ▬▬▬▬▬▭▭▭▭▭ (160 px)          │   │ [review]
│ ├ WebKitGTK 2.52.7 (webweergave van Win11 Shell)             Wachten op downloaden       ˅ ┤   │
│ ├ Overige updates                                            Wachten op downloaden       ˅ ┤   │
│ └ 2 updates worden later aangeboden (muted line)                                          ┘   │
│ Meer opties                                                                                  │ §3.6
│  ⟳ Nieuwe Ubuntu-versies   Elk half jaar een upgrade; …              [Elke nieuwe versie ▾]  │
│  ⟲ Updategeschiedenis                                                                    ›   │ subpage
│  ⚙ Geavanceerde opties     Melding bij opnieuw opstarten, datalimiet, …                   ›   │ subpage
│ Back-up van instellingen   (unchanged)                                                       │
│ Over Win11 Shell           (unchanged)                                                       │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

* The hero keeps `.st-upd` and its 64 px disc; **one** accent button, as in Windows. "Software-updater openen"
  moves to Geavanceerde opties. Disc colours: success green (`idle`), accent (`available`, busy states: the glyph
  turns, unless reduced motion), orange `--sys-caution` (`restartRequired`), red badge `--sys-critical` (`error`,
  `offline`, `repairRequired`), caution (`unsupported`). While checking, a 40 px ProgressRing replaces the glyph
  (as today).
* The card sits above the rows, as in Windows. Rows are `SettingsExpander`s, with the status text right-aligned
  and the existing `ProgressBar({width:160})` under it. The bar shows only while that row moves; it gets
  `error:true` on "Installatiefout".
* Every text below is rendered by the UI from codes and params (`{kind, version, count, size}`), so a language
  switch is live without the host. Host-rendered messages (errors, compat reasons) are refetched on `onLangChange`.
* `{product}` is filled from the key `product` ("Win11 Shell" in both languages, the name the Settings pages use
  today). A branding change is then one row per dictionary (open question 7 in gnome-map §6).
* `{when}` keeps today's rendering `relTime (date short time)`.

### 3.2 Hero states (`update.js` makeT; key → nl / en)

| Phase (host) | Glyph | Title | Caption | Button |
|---|---|---|---|---|
| `idle` | ⟳ + green ✓ | `upToDate` Je bent up-to-date / You’re up to date *(existing)* | `checked` Laatst gecontroleerd: {when} / Last checked: {when} *(existing)* | `checkFor` Controleren op updates / Check for updates |
| `checking` | ProgressRing | `checkingFor` Controleren op updates… / Checking for updates… | – | (none) |
| `available` | ⟳ accent | `updatesAvailable` Updates beschikbaar / Updates available | `checked` | `installAll` Alles downloaden en installeren / Download & install all |
| `downloading` (incl. simulate) | ⟳ accent, turning | `downloading` Updates downloaden / Downloading updates | `progressDl` {done} van {total} · nog ongeveer {eta} / {done} of {total} · about {eta} left; without ETA `progressDlNoEta` {done} van {total} / {done} of {total}; while simulating `preparing` Voorbereiden… / Getting ready… | `cancel` Annuleren / Cancel (only while `allowCancel`) |
| `installing` | ⟳ accent, turning | `installing` Updates installeren / Installing updates | `keepOn` Laat je pc aan staan. / Keep your PC turned on. | – |
| `waiting` (lock) | ⟳ accent | `waitLock` Wachten tot een andere installatie klaar is / Waiting for another installation to finish | `waitLockAuto` Ubuntu installeert op dit moment automatische updates. / Ubuntu is installing automatic updates right now. (unattended-upgr running) · else `waitLockOther` Een ander programma installeert software. / Another program is installing software. | `cancel` (while `allowCancel`) |
| `waiting` (auth) | ⟳ accent | `waitAuth` Wachten op toestemming / Waiting for permission | `waitAuthDesc` Bevestig in het venster dat om je wachtwoord vraagt. / Confirm in the window that asks for your password. | – |
| `foreign` | ⟳ accent | `foreign` Er wordt software geïnstalleerd door een ander programma / Another program is installing software · *[review]* while one of our own Software jobs is active (job files of `software.rs`): `foreignOwn` {product} installeert of verwijdert een app / {product} is installing or removing an app | `foreignDesc` Je kunt updates installeren zodra dat klaar is. / You can install updates as soon as that’s finished. | `installAll` disabled |
| `releaseRunning` *[review]* | ⟳ accent, turning | `relRunning` Ubuntu wordt bijgewerkt naar {version} / Upgrading Ubuntu to {version} | `relRunningBody` Het venster van Ubuntu doet de upgrade. Sluit het niet. / Ubuntu’s window does the upgrade. Don’t close it. | `relShowWindow` Venster weer openen / Show the upgrade window (no `checkFor`: every update write is refused, §4.9) |
| `restartRequired` | ⟳ orange | `restartRequired` Opnieuw opstarten vereist / Restart required | `restartDescFor` Start je pc opnieuw op om de update van {list} af te ronden. / Restart your PC to finish the update of {list}. · without a list `restartDesc` Start je pc opnieuw op om de updates af te ronden. / Restart your PC to finish installing updates. | `restartNow` Nu opnieuw opstarten / Restart now |
| `error` | ⟳ + red ✕ | `errorTitle` Er zijn problemen opgetreden bij het installeren van sommige updates / There were problems installing some updates *[review]* (Windows' first clause; its "…, but we'll try again later" is left out because nothing here retries by itself) | the host's `error.message` (§3.7) + Expander `techDetails` Technische details / Technical details (`error.detail`, monospace, selectable) | `retry` Opnieuw proberen / Retry |
| `unsupported` | ⚠ caution | `unknown` Kan niet controleren op updates / Can’t check for updates *(existing)* | `unsupportedDesc` {product} kan updates hier niet zelf installeren. Gebruik de Software-updater van Ubuntu. / {product} can’t install updates here itself. Use Ubuntu’s Software Updater. | `openUpdater` Software-updater openen / Open Software Updater *(existing, with its existing dialog)* |
| `offline` | ⟳ + red ✕ | `offline` Je bent offline / You’re offline | `offlineDesc` Maak verbinding met internet om te controleren op updates. Laatst gecontroleerd: {when} / Connect to the internet to check for updates. Last checked: {when} | `retry` |
| `repairRequired` | ⟳ + red ✕ | `repairTitle` Een eerdere installatie is niet afgerond / An earlier installation didn’t finish | `repairDesc` Ubuntu moet dit eerst herstellen. Daarna kun je weer updates installeren. / Ubuntu needs to repair this first. Then you can install updates again. | `repair` Herstellen / Repair (`update.repair`, admin prompt) |

Rules:
* `restartRequired` wins over `available` (Windows shows the restart first). New updates while a restart is pending
  get a list header `alsoAvailable` Ook beschikbaar / Also available with a **standard** `installAll` button.
* Only phased updates: the hero says `upToDate`. The list shows one muted line `phasedLater` {one: 1 update wordt later
  aangeboden, other: {n} updates worden later aangeboden} / {one: 1 update will be offered later, other: {n} updates
  will be offered later}. `system.updates.available` counts non-phased updates only (Q22 of H). *[review]* "Phased"
  is our own classification with apt's rule (§4.6), not a PackageKit info value. Held packages (`info` 9 blocked, or
  apt-mark hold) are counted neither as available nor as phased and get the neutral line `keptBack` {one: 1 update
  wordt nu niet geïnstalleerd, other: {n} updates worden nu niet geïnstalleerd} / {one: 1 update isn’t being
  installed right now, other: {n} updates aren’t being installed right now}.
* `{list}` in `restartDescFor` comes from `restart.rebootPkgs` with friendly names: `pkgKernel` Linux-kernel /
  Linux kernel (`linux-image-*`, `linux-modules-*`, `linux-base`), `pkgLibc` systeembibliotheek / system library
  (`libc6`), `pkgGnome` GNOME Shell (`gnome-shell`, `mutter*`, `libmutter-*`), and otherwise the package name.
  The list is joined with `list()` (GE1a's `core/i18n.js`), at most 3 names plus `andMore` {one: en 1 andere, other:
  en {n} andere} / {one: and 1 other, other: and {n} others}.
* "Nu opnieuw opstarten" runs the same flow as Start › Aan/uit › Opnieuw opstarten:
  `shell.command {command:'power', args:{action:'reboot'}}`, which the panel's `shell/power.js` handles (confirm, then
  the fade, then `system.power.action`). When `shell.info` shows no panel, the page calls
  `system.power.action {action:'reboot'}` itself after its own confirm. The power guard (§4.9) applies on both paths.
* After a successful install without a restart, the hero goes back to `idle` (Windows shows no extra message).

**Dialogs and InfoBars on the main page** (`update.js`):

| Key | nl | en |
|---|---|---|
| `meteredInfo` (InfoBar info) | Je gebruikt een verbinding met een datalimiet. Downloaden kan kosten met zich meebrengen. | You’re on a metered connection. Downloading may cost you data charges. |
| `meteredQ` / `meteredBody` / `download` | Toch downloaden? / Er wordt ongeveer {size} gedownload. / Downloaden | Download anyway? / About {size} will be downloaded. / Download |
| `removeQ` / `removeBody` / `installAnyway` | Pakketten verwijderen? / Voor deze updates moet {list} worden verwijderd. Toch installeren? / Toch installeren | Remove packages? / These updates need {list} to be removed. Install anyway? / Install anyway |
| `cancel` | Annuleren | Cancel |
| warnings (InfoBar warning) | the host's `warnings[].message` (e.g. `update.warn.source`) | same |

### 3.3 Update rows

The host groups packages into rows (`kind`). The UI renders the title from `{kind, version, count, size, security}`.
Component rows take precedence over `security`; a component row whose packages are security updates gets the
`securityTag` in its caption. The order is: security, gnome, webkit, gtk, soup, glib, kernel, other.

| `kind` | Packages | Title nl / en |
|---|---|---|
| `security` | `info` 8 security (or 7 important, 26 critical), or `data` containing `-security`; not in a component row | `rowSecurity` Beveiligingsupdates voor Ubuntu {version} / Security updates for Ubuntu {version} (`{version}` = os-release `VERSION` with a trailing ` (…)` codename removed by the host *[review]*: "26.04.1 LTS (Resolute Raccoon)" → "26.04.1 LTS") |
| `gnome` | `gnome-shell*`, `mutter*`, `libmutter-*`, `gir1.2-mutter-*` | `rowGnome` GNOME Shell {version} / GNOME Shell {version} |
| `webkit` | `libwebkit2gtk-4.1-*`, `libjavascriptcoregtk-4.1-*`, `gir1.2-webkit2-4.1`, `gir1.2-javascriptcoregtk-4.1` | `rowWebkit` WebKitGTK {version} (webweergave van {product}) / WebKitGTK {version} ({product}’s web view); since 2026-11-13 (no WebKit of ours) the native page's neutral WebKitGTK {version} in both languages |
| `gtk` | `libgtk-3-*`, `gir1.2-gtk-3.0` | `rowGtk` GTK 3 {version} / GTK 3 {version} |
| `soup` | `libsoup-3.0-*`, `gir1.2-soup-3.0` | `rowSoup` libsoup {version} / libsoup {version} |
| `glib` | `libglib2.0-*` | `rowGlib` GLib {version} / GLib {version} |
| `kernel` | `linux-image-*`, `linux-modules-*`, `linux-headers-*`, `linux-generic*`, `linux-base` | `rowKernel` Linux-kernel {version} / Linux kernel {version} |
| `other` | the rest | `rowOther` Overige updates / Other updates |

Caption: `rowCaption` {one: 1 pakket · {size}, other: {n} pakketten · {size}} / {one: 1 package · {size}, other: {n}
packages · {size}}; without a size `rowCaptionNoSize` {one: 1 pakket, other: {n} pakketten} / {one: 1 package,
other: {n} packages}; prefixed by `securityTag` Beveiligingsupdate · / Security update · when it applies.
*[review]* `rowCaptionNoSize` is the **normal** case: the apt backend reports `Details` through `pk_backend_job_details`
(no `_full`), so per-package `download-size` is very likely absent (R1 confirms). The total for the hero and the
metered confirm comes from the simulate's `DownloadSizeRemaining`; `rowCaption` is used only when sizes do arrive.

Row status (combined from `Package` info and `ItemProgress`; a row's percentage is the mean of its packages):

| State | nl | en |
|---|---|---|
| `pendingDownload` | Wachten op downloaden | Pending download |
| `downloading` | Downloaden - {pct}% | Downloading - {pct}% |
| `pendingInstall` | Wachten op installatie | Pending install |
| `installing` | Installeren - {pct}% | Installing - {pct}% |
| `installed` | Geïnstalleerd | Installed |
| `restart` | Wachten op opnieuw opstarten *[review]* | Pending restart *[review]* |
| `failed` | Installatiefout | Install error |

* A failed row gets its own `retry` Opnieuw proberen / Retry (`update.install {rows:[id]}`).
* **Details** (expanded row): one line per package, `{name} {from} → {to}` (not translated) with its summary. The
  link `whatsNew` Wat is er nieuw / What’s new opens a Dialog fed by `update.details {ids}` (`GetUpdateDetail`; the
  apt backend fetches changelogs, which is PackageKit's network access, not ours). Dialog keys: `changes` Wijzigingen
  / Changes; `cves` Beveiligingsproblemen / Security issues; `bugs` Opgeloste fouten / Fixed bugs; `noDetails`
  Ubuntu heeft voor deze update geen beschrijving. / Ubuntu has no description for this update.; `close` Sluiten /
  Close. CVE and Launchpad links open in the default browser through the existing `apps.openUri` method (only
  `https:` URLs from the PackageKit detail).
* *[review]* Phased packages (our classification, §4.6: apt would defer them on this machine) show in the details as
  `offeredLater` Wordt later aangeboden / Offered later and are never part of "Alles" or of any row's ids. Held
  packages (`info` 9 blocked, or apt-mark hold) show as `keptBackRow` Wordt nu niet geïnstalleerd / Not being
  installed right now, a neutral text, because info 9 does not say why. R1 only confirms what GetUpdates reports for
  a phased package; the design no longer depends on it.

### 3.4 Restart banners

| Cause (§4.8) | Shown as | nl | en | Action |
|---|---|---|---|---|
| `/run/reboot-required`, `RequireRestart` system/security-system/session/security-session, or a replaced gnome-shell/mutter in the running gnome-shell | hero `restartRequired` (§3.2) | – | – | `restartNow` |
| A Win11 process maps a replaced GTK 4, GTK/GDK 3, GLib/GIO/GObject, GDK Pixbuf, cairo, libarchive, or is a replaced `win11-*` binary (2026-11-13: WebKitGTK, JavaScriptCore and libsoup left the list with our WebKit hosts) | InfoBar (warning) above the rows; the hero is unchanged; **hidden while a reboot is pending** (a reboot covers it). *[review]* The WebKit pages made one exception, a replaced WebKitGTK/JavaScriptCore (new web processes started by old UI processes broke at once, and the reboot may be days away); the native apps have no such part, so the native page (`shell_bar_shown`) keeps only the rule | `shellRestartTitle` {product} moet opnieuw starten · `shellRestartBody` Onderdelen van het bureaublad zijn bijgewerkt ({list}). Opnieuw starten duurt een paar seconden; vensters van {product} gaan even dicht. | {product} needs to restart · Parts of the desktop were updated ({list}). Restarting takes a few seconds; {product} windows close briefly. | `shellRestartNow` {product} opnieuw starten / Restart {product} → `shell.restartAll {reason:'libraries'}` (GX1, §5) |

`{list}` uses component names from the UI dictionary: `cWebkit` WebKitGTK, `cGtk` GTK, `cSoup` libsoup, `cGlib`
GLib, `cPixbuf` GDK Pixbuf, `cCairo` cairo, `cArchive` libarchive, `cSelf` {product}. The same in both languages,
but kept as keys so the checker sees them. 2026-11-13: the native page's `part_name` (GTK 4, GTK, GLib, GDK Pixbuf,
cairo, libarchive, {product}); the host's unit test `shell_parts_use_the_page_codes` checks that it names every code
of `paths.rs` `SHELL_MAPS` and `self`.

### 3.5 Feature-update card "Ubuntu 26.10 is beschikbaar" (`update-release.js` makeT)

Logo: `/icon?name=distributor-logo`, then `/icon?name=/usr/share/pixmaps/ubuntu-logo.svg`, then the `update`
glyph. `{version}` = meta-release `Version` (e.g. "26.10"), `{support}` = `supportMonths` {one: 1 maand, other: {n}
maanden} / {one: 1 month, other: {n} months} under 24 months, else `supportYears` {one: 1 jaar, other: {n} jaar} /
{one: 1 year, other: {n} years}; `{eol}` = `date(eol,'month')`.

| State | Title | Body | Controls |
|---|---|---|---|
| not offered (no newer supported release for this Prompt, or Prompt=never) | *no card* | – | – |
| `checking` | `relAvailable` Ubuntu {version} is beschikbaar / Ubuntu {version} is available | ProgressRing + `relChecking` Controleren of deze pc klaar is voor Ubuntu {version}… / Checking whether this PC is ready for Ubuntu {version}… | – |
| `ok` | `relAvailable` | `relOkBody` De nieuwste versie van Ubuntu, met nieuwe functies. Krijgt {support} updates, tot {eol}. / The latest version of Ubuntu, with new features. Gets {support} of updates, until {eol}. + `relCompatOk` ✓ {product} werkt met Ubuntu {version}. / ✓ {product} works with Ubuntu {version}. | accent `relInstall` Downloaden en installeren / Download and install; link `relWhatsNew` Bekijk wat er in deze update zit / See what’s in this update (meta-release `ReleaseNotesHtml`) |
| `warn` | `relAvailable` | as `ok` + Expander `relBefore` Let op voordat je upgradet / Before you upgrade, listing host reasons (§3.7 `update.compat.*`) | as `ok` |
| `hold` (GNOME major untested: candidate major ∉ installed `shell-version`, or `releases.json` status ≠ tested) | `relOnWay` Ubuntu {version} is onderweg / Ubuntu {version} is on its way | `relHoldGnome` Wacht op een compatibele Win11-versie. {product} werkt nog niet met GNOME {major}, dat bij Ubuntu {version} hoort. / Wait for a compatible Win11 version. {product} doesn’t work with GNOME {major} yet, which comes with Ubuntu {version}. · when the major is the same but untested: `relHoldBody` Wacht op een compatibele Win11-versie. Zodra {product} klaar is voor deze versie, zie je de upgrade hier. / Wait for a compatible Win11 version. As soon as {product} is ready for this version, you’ll see the upgrade here. | none (D-G12 "blocks"); `releases.json` `allowAnyway:true` for a codename adds the link `relAnyway` Toch upgraden… / Upgrade anyway… → warning dialog (§12 C4). *[review]* `{major}` comes from the archive check, or from `releases.json` `gnomeShell` when the cheap verdict already holds (§6.2); a major that only `releases.json` gives is shown through `relHoldBody`, never named. *[review]* Below the body a muted line `relUbuntuMayOffer` De Software-updater van Ubuntu kan deze upgrade nog wel aanbieden. Wacht daar ook mee. / Ubuntu’s Software Updater may still offer this upgrade. Wait with it there too. and the link `relUbuntuWait` Ook de melding van Ubuntu uitstellen / Postpone Ubuntu’s own prompt too → `update.release.ubuntuHold {on:true}` (§4.12). After it: `relUbuntuWaiting` Ubuntu biedt deze upgrade niet meer zelf aan tot {product} er klaar voor is. / Ubuntu won’t offer this upgrade itself until {product} is ready for it. plus the link `relUbuntuUndo` Ongedaan maken / Undo |
| `ubuntuHold` (meta-release `UpgradeBroken`, or `check-new-release` "not possible right now") | `relOnWay` | `relUbuntuHold` Ubuntu heeft de upgrade tijdelijk stopgezet. / Ubuntu has paused the upgrade for now. | – |
| `blocked` | `relBlocked` Deze pc is nog niet klaar voor Ubuntu {version} / This PC isn’t ready for Ubuntu {version} yet | one host line per missing part (`update.compat.missing` / `.removed`) | – *[review]*: the same `relUbuntuMayOffer` line and `relUbuntuWait` link as `hold` |
| `updatesFirst` (a non-phased upgradable package exists, do-release-upgrade's rule) | `relAvailable` | `relUpdatesFirst` Installeer eerst de beschikbare updates. / Install the available updates first. | accent `relUpdatesThen` Updates installeren en daarna upgraden / Install updates, then upgrade |
| `rebootFirst` (`reboot-required.pkgs` has `libc6`, `linux-base` or `linux-image-*`) | `relAvailable` | `relRebootFirst` Start je pc eerst opnieuw op om eerdere updates af te ronden. Daarna kun je upgraden. / Restart your PC first to finish earlier updates. Then you can upgrade. | `restartNow` |
| `running` | `relRunning` Ubuntu wordt bijgewerkt naar {version} / Upgrading Ubuntu to {version} | `relRunningBody` Het venster van Ubuntu doet de upgrade. Sluit het niet. / Ubuntu’s window does the upgrade. Don’t close it. | `relShowWindow` Venster weer openen / Show the upgrade window |
| `waiting` (upgrader running, `/var/log/dist-upgrade/main.log` unchanged ≥ 10 min) | `relRunning` | `relWaitingBody` Ubuntu wacht misschien op een antwoord in het upgradevenster. / Ubuntu may be waiting for an answer in the upgrade window. | accent `relShowWindow` |
| `startFailed` *[review]* (the wrapper's status file has rc ≠ 0 and the upgrader was never seen; or no upgrader and no status file 120 s after the start) | `relNotStarted` De upgrade is niet gestart / The upgrade didn’t start | the host reason (§3.7): rc 126 `update.release.err.noPassword`, rc 127 `.wrongPassword`, any other rc `.refused` (Ubuntu's own reason stays visible in the held terminal window), no window `.noWindow` | `retry` (back to the offer state and its checks) |
| `ended` *[review]* (the upgrader was seen, is gone, no reboot is pending, and rc ≠ 0 or no status file) | `relEnded` De upgrade is gestopt voordat hij klaar was / The upgrade stopped before it finished | `relEndedBody` Ubuntu heeft de upgrade niet afgemaakt. In het logboek staat wat er is gebeurd. / Ubuntu didn’t finish the upgrade. The log shows what happened. | `relOpenLog` Logboek openen / Open log (`/var/log/dist-upgrade/main.log`, 0644 here, opened read-only with the default text app through the existing file-open path of `files/actions.rs`); `close` hides the card until the next check |
| `unknown` (compat check impossible: offline, metered without permission, apt error) | `relAvailable` | `relCheckFailed` De controle is niet gelukt. {reason} / The check didn’t work. {reason} | `retry` |

`updatesFirst` is checked before `rebootFirst`, the same order as do-release-upgrade. *[review]* An upgrader exit with
rc 0 and no pending reboot (for example Robin answered "no" at Ubuntu's first question) returns to the offer state
without a message; what the text frontend returns in each case after it appeared is unverified until R5.

**Start dialog** (before `update.release.start`): `relStartQ` Upgraden naar Ubuntu {version}? / Upgrade to Ubuntu
{version}?; `relStartBody` Het upgraden doet Ubuntu zelf, in een terminalvenster. Het duurt meestal 30 tot 90 minuten
en Ubuntu vraagt om je wachtwoord. Sla je werk op, sluit een laptop aan op de lader en sluit het venster niet tot
Ubuntu om een herstart vraagt. / Ubuntu does the upgrade itself, in a terminal window. It usually takes 30 to 90
minutes and Ubuntu asks for your password. Save your work, plug a laptop into its charger, and don’t close the window
until Ubuntu asks to restart.; buttons `relStart` Upgrade starten / Start upgrade and `cancel`. Extra InfoBars:
`relBattery` Je laptop werkt op de accu. Sluit hem aan op de lader voordat je begint. / Your laptop is running on
battery. Plug it in before you start. (UPower `OnBattery`, via `system.battery.get`); `relMetered` Je gebruikt een
verbinding met een datalimiet. Een upgrade downloadt vaak meer dan 2 GB. / You’re on a metered connection. An upgrade
often downloads more than 2 GB.; in Dutch only while `/usr/share/locale*/nl/LC_MESSAGES/ubuntu-release-upgrader.mo`
is absent: `relEnglishFirst` De eerste regels in het venster zijn in het Engels. / The first lines in the window are
in English. (the en text exists for parity and is never shown).

Card-level host refusals of `update.release.start` reach the page as `error.message` (§3.7 `update.release.err.*`).

### 3.6 Meer opties, Updategeschiedenis, Geavanceerde opties

Main page (`update.js` + `update-release.js`): header `moreOptions` Meer opties / More options.

* **Nieuwe Ubuntu-versies** (`update-release.js`, where Windows has "Ontvang de nieuwste updates…"): `promptRow`
  Nieuwe Ubuntu-versies / New Ubuntu versions; Dropdown `promptLts` Alleen LTS-versies / LTS versions only,
  `promptNormal` Elke nieuwe versie / Every new version, and `promptNever` Nooit melden / Never notify (only while the
  file says never/no). Descriptions: `promptLtsDesc` Om de twee jaar een upgrade; elke versie krijgt 5 jaar updates. /
  An upgrade every two years; each version gets 5 years of updates.; `promptNormalDesc` Elk half jaar een upgrade;
  een tussenversie krijgt 9 maanden updates. / An upgrade every six months; an interim version gets 9 months of
  updates.; G-specific second line under normal: `promptNormalGnome` Elke tussenversie heeft een nieuwe GNOME, en
  {product} moet daar eerst klaar voor zijn. / Every interim version has a new GNOME, and {product} has to be ready
  for it first.; on an interim release with Prompt=lts: `promptLtsInterim` Je gebruikt een tussenversie, dus Ubuntu
  biedt toch elke nieuwe versie aan tot de volgende LTS. / You’re on an interim version, so Ubuntu still offers every
  new version until the next LTS.; released-not-offered: `relNotOffered` Ubuntu {version} is uitgebracht; Ubuntu
  biedt de upgrade nog niet aan. / Ubuntu {version} is out; Ubuntu doesn’t offer the upgrade yet.; override:
  `promptOverride` Deze instelling wordt bepaald door {file}. / This setting is controlled by {file}. (control
  disabled); helper missing: InfoBar `promptShInfo` Ubuntu vraagt om je wachtwoord om ‘/bin/sh’ als beheerder uit te
  voeren. Dat is {product}, dat alleen de regel Prompt= aanpast. / Ubuntu asks for your password to run ‘/bin/sh’ as
  administrator. That’s {product}, changing only the Prompt= line.
* `historyRow` Updategeschiedenis / Update history (chevron → subpage `update-history`).
* `advancedRow` Geavanceerde opties / Advanced options, `advancedRowDesc` Melding bij opnieuw opstarten,
  datalimiet, Software-updater van Ubuntu / Restart notifications, metered connections, Ubuntu Software Updater
  (chevron → subpage `update-advanced`).

**Updategeschiedenis** (`update-history.js`, registry `P('update-history', 'update', 'history', …)`), from
`/var/log/apt/history.log` and `.1.gz`–`.4.gz` (0644), at most 200 entries, 90 days, then `hLoadMore`:

| Key | nl | en |
|---|---|---|
| `hTitle` | Updategeschiedenis | Update history |
| `hUpdates` / `hReleases` / `hSoftware` (groups) | Updates / Ubuntu-versies / Software | Updates / Ubuntu versions / Software |
| `hInstalledOn` | Geïnstalleerd op {date} | Successfully installed on {date} |
| `hFailedOn` | Kon niet worden geïnstalleerd op {date} | Failed to install on {date} |
| `hByUbuntu` / `hByUs` / `hByApp` / `hByTerminal` | automatisch door Ubuntu / via {product} / via een app / in een terminal (apt) | automatically by Ubuntu / via {product} / via an app / in a terminal (apt) |
| `hLoadMore` / `hEmpty` | Meer laden / Er is nog geen updategeschiedenis. | Load more / There’s no update history yet. |

An entry reads `{name} {version}` with the date line below it. "via {product}" matches our job's recorded
`startedAt`/`finishedAt` window (`jobs.jsonl`, §4.5) against `Start-Date`/`End-Date` and `Requested-By` = our uid,
because PackageKit writes only `Commandline: packagekit role='update-packages'`.

**Geavanceerde opties** (`update-advanced.js`, registry `P('update-advanced', 'update', 'settings', …)`):

| Key | nl | en | Control |
|---|---|---|---|
| `advTitle` | Geavanceerde opties | Advanced options | – |
| `advNotify` | Melden wanneer opnieuw opstarten vereist is om het bijwerken te voltooien *[review]* | Notify me when a restart is required to finish updating | toggle `settings.update.notifyRestart` (default true) |
| `advMetered` / `advMeteredDesc` | Updates downloaden via verbindingen met een datalimiet / Er kunnen kosten in rekening worden gebracht | Download updates over metered connections / Data charges may apply | toggle `settings.update.metered` (default false) |
| `advAutoOpen` / `advAutoOpenDesc` (D2) *[review]* | Software-updater van Ubuntu automatisch openen voor gewone updates / Ubuntu opent de Software-updater om de {n} dagen als er gewone updates zijn. Voor beveiligingsupdates opent Ubuntu hem altijd meteen. Updates kun je ook hier installeren. | Open Ubuntu’s Software Updater automatically for regular updates / Ubuntu opens Software Updater every {n} days when regular updates are available. For security updates, Ubuntu always opens it right away. You can also install updates here. | toggle → `update.setAutoOpen {on}` (§4.12); `{n}` plural {one: dag, other: dagen} / {one: day, other: days}; in the off state the description drops the first sentence |
| `advSecurity` / `advSecurityOn` / `advSecurityOff` | Automatische beveiligingsupdates / Aan: Ubuntu installeert ze elke dag / Uit | Automatic security updates / On: Ubuntu installs them every day / Off | read-only (`apt-config dump APT::Periodic::Unattended-Upgrade`) |
| `advSnaps` / `advSnapsDesc` | Apps uit de Snap Store / Worden automatisch bijgewerkt door Ubuntu. Laatst: {when}, volgende: {next}. | Snap Store apps / Updated automatically by Ubuntu. Last: {when}, next: {next}. | read-only (`snap refresh --time`, local REST) |
| `advUpdater` | Software-updater van Ubuntu | Ubuntu Software Updater | `open` Openen / Open → the `updaterTitle`/`updaterBody` dialog → `system.openUpdater`. The keys are copied with unchanged text; `update.js` keeps its own copy for the `unsupported` hero. GW3's stub of this file already contains this row, so the updater stays reachable between rounds 4 and 5 |

There is no "Updates onderbreken", no Insider programme and no carbon footer.

### 3.7 Host texts (`i18n/host/update.tsv`, stem `update`)

All rows are written in GW2, so later steps never edit the file in parallel. `{product}` is the row `update.product`.

| Key | When | nl | en |
|---|---|---|---|
| `update.product` | – | Win11 Shell | Win11 Shell |
| `update.err.network` | no-network 2, no-more-mirrors 43, repo-not-available 37 (Ubuntu URI or `GetUpdates` failed too) | Er is geen verbinding met de updateserver. Controleer je internetverbinding en probeer het opnieuw. | Can’t reach the update server. Check your internet connection and try again. |
| `update.warn.source` | the same codes for a non-Ubuntu source while `GetUpdates` succeeds | Een updatebron kon niet worden bijgewerkt: {source}. De andere updates zijn wel gecontroleerd. | One update source couldn’t be refreshed: {source}. The other updates were checked. |
| `update.err.offline` | NM connectivity none before a check | Je bent offline. Maak verbinding met internet en probeer het opnieuw. | You’re offline. Connect to the internet and try again. |
| `update.err.repair` | exit repair-required 11, error unfinished-transaction 66, or a failed configure that leaves half-done packages | Een eerdere installatie is niet afgerond. Ubuntu moet dit eerst herstellen. | An earlier installation didn’t finish. Ubuntu needs to repair this first. |
| `update.err.cannotCancel` | cannot-cancel 25, or Cancel while `AllowCancel` false | De installatie is al begonnen en kan nu niet meer worden gestopt. | Installing has already started and can’t be stopped now. |
| `update.err.notAuthorized` | not-authorized 48 | Je hebt geen toestemming gegeven, dus er is niets veranderd. | Permission wasn’t given, so nothing was changed. |
| `update.err.noSpace` | no-space-on-device 46 | Er is niet genoeg ruimte op de systeemschijf. Maak ruimte vrij en probeer het opnieuw. | There isn’t enough space on the system drive. Free up some space and try again. |
| `update.err.download` | package-download-failed 10 | Een update kon niet worden gedownload. Probeer het later opnieuw. | An update couldn’t be downloaded. Try again later. |
| `update.err.locked` | cannot-get-lock 26, lock-required 67, *[review]* cancelled-priority 65 / exit cancelled-priority 9 | Er wordt al software geïnstalleerd. Probeer het over een paar minuten opnieuw. | Software is already being installed. Try again in a few minutes. |
| `update.err.untrusted` | gpg-failure 5, bad/missing-gpg-signature 30/31, cannot-update-repo-unsigned 51, exit need-untrusted 8 | Deze update is niet ondertekend en wordt daarom niet geïnstalleerd. | This update isn’t signed, so it won’t be installed. |
| `update.err.conflict` | dep-resolution-failed 13, package-conflicts 36, file-conflicts 35, *[review]* package-install-blocked 39 | Ubuntu kan deze updates nu niet samen installeren. Probeer het later opnieuw, of gebruik de Software-updater. | Ubuntu can’t install these updates together right now. Try again later, or use Software Updater. |
| `update.err.dpkg` | package-failed-to-configure 56, -install 58, update-failed-due-to-running-process 60 | Een update is niet goed geïnstalleerd. Probeer het opnieuw; helpt dat niet, open dan de Software-updater. | An update didn’t install correctly. Try again; if that doesn’t help, open Software Updater. |
| `update.err.cancelled` | transaction-cancelled 17, exit cancelled 3 (only during download) | Het installeren is gestopt. Er is niets half geïnstalleerd. | Installing was stopped. Nothing was left half-installed. |
| `update.err.failed` | anything else, incl. EulaRequired/MediaChange/RepoSignature | Het installeren van de updates is mislukt. | The updates couldn’t be installed. |
| `update.err.checkFailed` | `RefreshCache` and `GetUpdates` both failed for a non-network reason | Controleren op updates is mislukt. | Checking for updates didn’t work. |
| `update.err.noPackageKit` | no PackageKit name, not activatable | PackageKit ontbreekt, dus {product} kan zelf geen updates installeren. | PackageKit is missing, so {product} can’t install updates itself. |
| `update.err.unknownOutcome` | PackageKit vanished mid-transaction and `GetOldTransactions` has no record | Het is niet bekend of de updates zijn geïnstalleerd. Controleer opnieuw op updates. | It isn’t known whether the updates were installed. Check for updates again. |
| `update.err.busy` | our job already running (lock held) | Er wordt al een update geïnstalleerd. | An update is already being installed. |
| `update.err.stale` | ids or rows not in the current snapshot; package-database-changed 61 | De lijst met updates is veranderd. Controleer opnieuw op updates. | The list of updates changed. Check for updates again. |
| `update.err.badArgs` | invalid input | Ongeldige invoer voor updates. | Invalid update input. |
| `update.err.testSeam` *[review]* | a `WIN11_UPDATE_*` variable is set but fails its check (§4.11); only tests ever see it | Een testinstelling voor updates klopt niet, dus er is niets gedaan. | An update test setting is wrong, so nothing was done. |
| `update.err.details` | `GetUpdateDetail` failed | De details van deze update zijn niet beschikbaar. | The details of this update aren’t available. |
| `update.guard.power` | power action while our job is past `AllowCancel` | Er worden updates geïnstalleerd. Opnieuw opstarten kan zodra dat klaar is. | Updates are being installed. You can restart as soon as that’s done. |
| `update.guard.upgrade` | power action, `shell.restartAll` *[review]* or any update write (`update.check`, `.install`, `.confirm`, `.repair`, `.release.check`, `.release.compat`, `.release.setPrompt`, `.release.start`) while the release upgrader runs | Ubuntu wordt bijgewerkt naar een nieuwe versie. Wacht tot het upgradevenster om een herstart vraagt. | Ubuntu is being upgraded to a new version. Wait until the upgrade window asks you to restart. |
| `update.guard.restart` *[review]* | `shell.restartAll` while any own update job is active (any phase) | Er worden updates geïnstalleerd. {product} kan opnieuw starten zodra dat klaar is. | Updates are being installed. {product} can restart as soon as that’s done. |
| `update.guard.software` *[review]* | `shell.restartAll` while a Software job is `queued`, `running` or `authenticating` | Er wordt nog een app geïnstalleerd of verwijderd. Probeer het daarna opnieuw. | An app is still being installed or removed. Try again afterwards. |
| `update.notify.installed` / `.installedBody` | job done, Settings open but not focused | Updates zijn geïnstalleerd / Je pc is up-to-date. | Updates installed / Your PC is up to date. |
| `update.notify.restart` / `.restartBody` / `.restartNow` / `.later` | job done with restart, `notifyRestart` on | Opnieuw opstarten vereist / Start je pc opnieuw op om de updates af te ronden. / Nu opnieuw opstarten / Later | Restart required / Restart your PC to finish installing updates. / Restart now / Later |
| `update.notify.failed` / `.view` | job failed, Settings not focused | Updates konden niet worden geïnstalleerd / Bekijken | Updates couldn’t be installed / View |
| `update.release.terminalTitle` | terminal window title | Ubuntu-upgrade | Ubuntu upgrade |
| `update.release.err.updatesPending` | start refused | Installeer eerst de beschikbare updates. | Install the available updates first. |
| `update.release.err.rebootFirst` | start refused | Start je pc eerst opnieuw op. | Restart your PC first. |
| `update.release.err.busy` | start refused: a transaction or dpkg runs | Er wordt nog software geïnstalleerd. Probeer het daarna opnieuw. | Software is still being installed. Try again afterwards. |
| `update.release.err.hold` | start refused: hold/blocked | {product} is nog niet klaar voor deze Ubuntu-versie. | {product} isn’t ready for this Ubuntu version yet. |
| `update.release.err.notOffered` | start refused: nothing offered | Ubuntu biedt op dit moment geen nieuwe versie aan. | Ubuntu isn’t offering a new version right now. |
| `update.release.err.noTerminal` | no xdg-terminal-exec | Er is geen terminal gevonden. Installeer Ptyxis (Terminal) en probeer het opnieuw. | No terminal was found. Install Ptyxis (Terminal) and try again. |
| `update.release.err.notRunning` | attach refused | De upgrade van Ubuntu loopt niet meer. | The Ubuntu upgrade isn’t running any more. |
| `update.release.err.noAttachHelper` | attach without helper | Open een terminal en typ: sudo screen -d -r ubuntu-release-upgrade-screen-window | Open a terminal and type: sudo screen -d -r ubuntu-release-upgrade-screen-window |
| `update.release.err.noPassword` | pkexec rc 126 | Je hebt het wachtwoord niet ingevuld, dus er is niets veranderd. | You didn’t enter the password, so nothing was changed. |
| `update.release.err.wrongPassword` | pkexec rc 127 | Het wachtwoord klopte niet. Er is niets veranderd. | The password was wrong. Nothing was changed. |
| `update.release.err.promptFailed` | helper `@@fail` | De instelling voor nieuwe Ubuntu-versies kon niet worden opgeslagen. | The setting for new Ubuntu versions couldn’t be saved. |
| `update.release.err.checkFailed` | `check-new-release` failed | Controleren op een nieuwe Ubuntu-versie is mislukt. | Checking for a new Ubuntu version didn’t work. |
| `update.release.err.refused` *[review]* | wrapper rc ∉ {0, 126, 127} before the upgrader appeared | Ubuntu heeft de upgrade niet gestart. De reden staat in het terminalvenster. | Ubuntu didn’t start the upgrade. The reason is in the terminal window. |
| `update.release.err.noWindow` *[review]* | no upgrader and no status file 120 s after the start | Het upgradevenster is niet verschenen. Probeer het opnieuw. | The upgrade window didn’t appear. Try again. |
| `update.release.pressEnter` *[review]* | last line of the wrapper when it holds the window | Druk op Enter om dit venster te sluiten. | Press Enter to close this window. |
| `update.release.err.ubuntuHold` *[review]* | `update.release.ubuntuHold` could not write the update-manager keys (schema missing or write failed) | De melding van Ubuntu kon niet worden uitgesteld. | Ubuntu’s own prompt couldn’t be postponed. |
| `update.compat.missing` | blocking part has no candidate | {part} zit niet in Ubuntu {version}. | {part} isn’t in Ubuntu {version}. |
| `update.compat.removed` | the simulation removes a blocking part | Ubuntu {version} verwijdert {part}. | Ubuntu {version} removes {part}. |
| `update.compat.degraded` | a degraded part is missing or removed | {part} ontbreekt in Ubuntu {version}; {feature} werkt dan niet. | {part} is missing from Ubuntu {version}; {feature} won’t work then. |
| `update.compat.thirdParty` | third-party sources exist | {one: Ubuntu zet tijdens de upgrade 1 extra softwarebron uit ({name})., other: Ubuntu zet tijdens de upgrade {n} extra softwarebronnen uit, zoals {name}.} | {one: Ubuntu turns off 1 extra software source during the upgrade ({name})., other: Ubuntu turns off {n} extra software sources during the upgrade, such as {name}.} |
| `update.compat.offline` / `.metered` / `.apt` | compat `unknown` reasons | Je bent offline. / Je gebruikt een verbinding met een datalimiet; zet Updates downloaden via verbindingen met een datalimiet aan om toch te controleren. / Het pakketoverzicht van Ubuntu {version} kon niet worden geladen. | You’re offline. / You’re on a metered connection; turn on Download updates over metered connections to check anyway. / The package list of Ubuntu {version} couldn’t be loaded. |
| `update.part.webkit` … | part labels | WebKitGTK (webweergave) · JavaScriptCore · GTK 3 · libsoup · GLib · GDK Pixbuf (afbeeldingen) · cairo (tekenen) · zlib · libarchive (archieven openen) · GNOME Shell | WebKitGTK (web view) · JavaScriptCore · GTK 3 · libsoup · GLib · GDK Pixbuf (images) · cairo (drawing) · zlib · libarchive (opening archives) · GNOME Shell |
| `update.part.ffmpeg` / `bubblewrap` / `squashfs` / `flatpak` / `packagekit` / `terminal` / `screen` / `dock` (+ `update.feature.*`) | degraded parts and their features | ffmpeg / video-miniaturen · bubblewrap / veilige voorbeelden · squashfs-tools / snap-pakketten bekijken · flatpak / Flatpak-apps installeren · PackageKit / updates en software installeren · een terminal / de volgende Ubuntu-upgrade · screen / de volgende Ubuntu-upgrade · Ubuntu Dock / het vangnet na een upgrade | ffmpeg / video thumbnails · bubblewrap / safe previews · squashfs-tools / inspecting snap packages · flatpak / installing Flatpak apps · PackageKit / installing updates and software · a terminal / the next Ubuntu upgrade · screen / the next Ubuntu upgrade · Ubuntu Dock / the safety net after an upgrade |
| `update.post.upgraded` | first login on a new release, extension active | Ubuntu is bijgewerkt naar {version} | Ubuntu was upgraded to {version} |
| `update.post.extOff` / `.extOffBody` / `.dockOn` / `.later` | extension OUT_OF_DATE (4) or INACTIVE (2) although enabled | GNOME {major} heeft {product} uitgezet / De taakbalk van {product} werkt nog niet met deze GNOME-versie. Zet de Ubuntu Dock aan tot er een update is. / Ubuntu Dock aanzetten / Later | GNOME {major} turned {product} off / {product}’s taskbar doesn’t work with this GNOME version yet. Turn on the Ubuntu Dock until there’s an update. / Turn on Ubuntu Dock / Later |
| `update.post.extError` / `.extErrorBody` | extension ERROR (3) | {product} kon niet starten / GNOME meldt een fout in {product}. Zet de Ubuntu Dock aan of installeer {product} opnieuw. | {product} couldn’t start / GNOME reports an error in {product}. Turn on the Ubuntu Dock or reinstall {product}. |
| `update.post.extsOff` / `.extsOffBody` / `.extsOn` *[review]* | `org.gnome.shell disable-user-extensions` is true (GNOME switched all user extensions off after a crash) | GNOME heeft extensies uitgezet / Na een fout heeft GNOME alle extensies uitgezet, ook {product}. / Extensies weer aanzetten | GNOME turned extensions off / After an error, GNOME turned off all extensions, including {product}. / Turn extensions back on |
| `update.post.missing` / `.missingBody` / `.installParts` | blocking parts missing after the upgrade | {product} mist onderdelen / Na de upgrade ontbreekt: {parts}. / Onderdelen installeren… | {product} is missing components / Missing after the upgrade: {parts}. / Install components… |

* `error.detail` (PackageKit's detail string, ≤ 4000 chars) is shown under "Technische details" as is.
* There is no `update.err.noAgent` (H): GNOME Shell's own polkit agent is always there while the shell runs.
  `not-authorized` therefore always maps to `update.err.notAuthorized`.
* `update.err.cancelled` may only say "nothing half-installed" because cancel is only offered while `AllowCancel`
  is true (download phase).

### 3.8 Other surfaces

* Settings Home, System and About keep calling `system.updates`. The host fills it from the update snapshot (§4.10),
  with an additive `rebootRequired`. Taakbeheer's health item (`sysmon/health.rs:316-334`) works unchanged.
  *[review]* Once a snapshot exists, its count (with its `checkedAt`) is the only count any process shows, so Home and
  Taakbeheer never alternate between the snapshot's number and `apt list`'s.
* Notifications go through GNOME's own server (`org.freedesktop.Notifications.Notify` on the session bus, app name
  `{product}`, hint `desktop-entry=io.github.win11shell.Settings`), so the extension mirrors them into the Win11
  centre. In v1 they are sent only by the owning Settings host *[review]* (never by a mirror) while it runs, including
  its window-less keep-alive after the window was closed during a job: installed, restart required (when
  `notifyRestart`), failed. With D5 (no background watcher) there is nothing while Settings is closed; Ubuntu's
  update-notifier still shows its own "restart required".
* Optional (GW9): an orange dot on Start's power button and "Bijwerken en opnieuw opstarten" / "Update and restart",
  read through `update.status` in reader mode.

### 3.9 Dutch that changes on the existing page (`tests/i18n/BASELINE-CHANGES`, GW3)

| Key (update.js) | Before | After | Reason |
|---|---|---|---|
| `check` → `checkFor` | Controleren | Controleren op updates | Windows wording |
| `available` (plural) → `updatesAvailable` | Er is 1 update / Er zijn {n} updates beschikbaar | Updates beschikbaar | Windows hero; the rows show the count |
| hero `openUpdater` button | in the hero | moves to Geavanceerde opties (same text and dialog); stays in the hero only for `unsupported` | one accent button, as in Windows |
| `checking` | Controleren… | Controleren op updates… | Windows wording |
| Home/System/About count | counts phased packages (`apt list`) | non-phased, non-held only (the snapshot, §4.6 classification) *[review]* | Q22 of H; numbers only, no text change |

Everything else on the page (backup, reset, about) stays byte-identical.

---

## 4. The Rust `update.*` service

### 4.1 Process model

* **Full mode only in the owning Settings host** (`ctx.kind == "settings"`, `win11-app --app settings`, which runs in
  `app.slice`). Full mode starts one thread `update-pk`, which owns a `glib::MainContext` + `MainLoop` (thread
  default) for every PackageKit, NetworkManager and FileMonitor callback. HTTP handler threads never touch D-Bus.
  They send commands over a `std::sync::mpsc` channel woken by `context.invoke` and wait with a timeout (≤ 5 s;
  long operations are asynchronous and report through events).
* *[review]* **One owner among Settings processes.** Settings is `NON_UNIQUE` and `shell.openApp` always spawns
  (§1.1), so two Settings processes are normal (Taakbeheer's health item opens a second one). At LazyService create a
  `settings` host tries a non-blocking `flock(LOCK_EX|LOCK_NB)` on `update/owner.lock` and keeps the fd for its whole
  life; the kernel drops it on exit or crash, so there is no stale owner. The winner writes `update/owner.json`
  `{pid, start}` (start = `/proc/<pid>/stat` field 22) and runs full mode. A loser is a **mirror**: reader mode for
  everything it reads, while its write methods are forwarded to the owner as the event `update.request {nonce,
  method, args}`; the owner validates them with the same per-method allowlists and runs them, the result arrives as
  `update.changed`, and an immediate refusal comes back as `update.reply {nonce, error}`. A mirror retries the lock
  every 30 s (a timeout on its main loop, not a thread) and on the event `update.owner {state:'gone'}` that an owner
  sends from `shutdown()`; when it wins, it switches to full mode and reattaches (§4.7). Only the owner sends
  notifications. `rev` lives in `snapshot.json`, so a new owner continues from the last value and `rev` stays
  monotonic across processes. A dry-run host never forwards (it answers with the dry-run envelope). This is no new
  privilege: any process of Robin's could already run `pkcon update` without a password.
* **Reader mode in every other kind** (panel, start, desktop, taskmgr, …): `update.status` reads the snapshot and job
  files and stats `/run/reboot-required`. There is no D-Bus, no thread and no FileMonitor. Writes return
  `ApiError("not_here", …)`. This keeps the 1000 MiB shell slice and the taskbar free of update work.
* **No join on shutdown.** `Update::shutdown` sets a flag, quits the loop and returns. A running transaction belongs
  to packagekitd. The next owner reattaches (§4.7).
* *[review]* **Keep-alive while an own job runs (C2).** Whether packagekitd cancels a cancellable transaction when its
  caller leaves the bus is unverified (it links `g_bus_watch_name` and `pk_transaction_cancel_bg`), so the owner never
  leaves during a job by choice. *2026-11-13, native:* the owner is the Settings backend (`win11-app --app settings
  --http-only`), which has no GApplication and dies with win11-gtk-settings (`PR_SET_PDEATHSIG`,
  `native/kit/src/backend.rs`). So win11-gtk-settings holds its own GApplication
  (`native/apps/settings/src/keepalive.rs`): it follows `update.changed`, `update.owner` and (until it holds)
  `update.progress`, and reads `update.status`. While its backend is the owner (`mode` owner) and a job of ours runs
  that does not wait for a confirm (review F7), and after the job until its re-check answered (`refreshing`, at most
  60 s as `pk.rs` `HOLD_AFTER_JOB`), closing the window keeps the process and its backend; then 1 s more, so the
  job's notification has left, and it exits. It tells the backend whether a window is shown (`update.setWatching`),
  because its event stream stays open without one: the outcome notification ("installed", "failed") goes out only
  when nobody watches. `services/update.rs` `hold_application` (a `gio::Application::hold()` in a host that is a
  GApplication: the WebKit Settings of GW2, whose window and WebKit view went away while the process stayed) does
  nothing in a backend. Reattach stays for crashes and re-execs.
* **One transaction per user:** `flock` on `~/.cache/win11-gnome/update/lock`, held by the job thread for the job's
  lifetime. A new owner (after a crash) sees the lock or the unfinished `job.json` and reattaches instead of starting
  a new job.
* Memory: about 1 MB of state in Settings; nothing in the shell processes. packagekitd itself (root, 150–300 MB
  while the apt cache is loaded) runs only during a check or a job and quits about 5 min after the last call.

### 4.2 Module layout and file ownership

| File | Content | Step |
|---|---|---|
| `host/src/services/update.rs` | `pub(crate) struct Update`, `new(ctx)` (full or reader mode), `call(method, args)` with argument allowlists per method (pattern of `software.rs:142-188`), `shutdown()`; `#[path]` submodules | GW2 |
| `update/paths.rs` | every system path and tool path in one `Paths`/`Tools` struct; the test re-rooting and the fail-closed seam check of §4.11 *[review]*; the only file under `update/` allowed to contain an absolute exec path | GW2 |
| `update/pk.rs` | event-driven PackageKit client: `Client::connect(bus)`, `transaction(role)`, signal and property handling, NO_AUTO_START reads, enum table | GW2 |
| `update/model.rs` | enums, row grouping, phase derivation, progress throttling (pure functions, unit-tested) | GW2 |
| `update/phasing.rs` *[review]* | the phasing/held classifier of §4.6 (*[D-G73]* three batched read-only apt programs: `apt-cache policy <name:arch>…`, `apt-cache policy ?phasing`, `apt-mark showhold`, joined on apt's package name; fallback `apt-cache show`), parsers unit-tested | GW2 |
| `update/store.rs` | snapshot, job and `jobs.jsonl` files (atomic write via temp file + rename, 0600), lock, boot id, *[review]* `owner.lock`/`owner.json`, persisted `rev`, `update.request` forwarding | GW2 |
| `update/restart.rs` | reboot-required FileMonitor and parse; deleted-mapping scan; history fallback | GW2 |
| `update/net.rs` | NetworkManager `Connectivity`/`Metered` (NO_AUTO_START reads, `PropertiesChanged`) | GW2 |
| `update/guard.rs` | `pub(crate) fn power_blocked(ctx) -> Option<ApiError>` (used by `system/power.rs` and GX1) | GW2 |
| `update/history.rs` | apt history parser, `update.history` | stub in GW2, filled by GW4 |
| `update/options.rs` | `update.options`, `update.setAutoOpen`, unattended and snap rows | stub GW2 → GW4 |
| `update/release.rs` | Prompt, motd stamp, meta-release caches, distro-info, `check-new-release`, `setPrompt` | stub GW2 → GW5 |
| `update/admin.rs` | trusted-helper check and pkexec runner (126/127, markers) | stub GW2 → GW5 |
| `update/compat.rs`, `update/elf.rs` | compatibility check, ELF NEEDED parser | stub GW2 → GW6 |
| `update/upgrade.rs` | release start, linger, running/waiting detection, attach | stub GW2 → GW7 |
| `update/tests.rs`, `update/bus_tests.rs` | unit tests; `#[ignore]` bus tests against the fake | GW2 (each later step adds its own `#[cfg(test)] mod tests` inside its own file) |

Each stub has its final signature, for example
`pub(super) fn history(ctx: &ServiceContext, paths: &Paths, args: &Value, lang: &str) -> ApiResult` returning
`Err(ApiError::new("unsupported", …))`, so later steps never edit `update.rs`.

### 4.3 API (`host/src/api-methods.json`, all entries added in GW2)

| Method | Dangerous | Args → result | Dry-run |
|---|---|---|---|
| `update.status` | no | `{packages?:bool}` → snapshot (below) | serves files or `{backend:'packagekit', phase:'idle', rows:[], dryRun:true}`; no D-Bus |
| `update.check` | yes | `{}` → `{started:true}`; phase `checking`, then events. Refuses `offline` before any transaction | returns the current snapshot, no D-Bus |
| `update.details` | yes (packagekitd downloads changelogs) | `{ids:[≤20 ids from the snapshot]}` → `{details:[{id, updateText, changelog, cves:[url], bugs:[url], restart, issued, updated}]}` (cached per id) | cached or `{details:[], dryRun:true}` |
| `update.install` | yes | `{rows?:[rowId]}` (default: all rows; rows never contain phased or held packages *[review]*) → `{jobId}`; a confirm pauses the job (`job.confirm`) | `{jobId:null, dryRun:true}` |
| `update.confirm` | yes | `{jobId, accept:bool}` → resumes or cancels a paused job | `{dryRun:true}` |
| `update.cancel` | yes | `{jobId}` → `{ok}`; error `cannot_cancel` (`update.err.cannotCancel`) unless `allowCancel` | `{dryRun:true}` |
| `update.repair` | yes | `{}` → `{jobId}`; `RepairSystem(ONLY_TRUSTED)`, only in `repairRequired` | `{dryRun:true}` |
| `update.history` | no | `{before?:ms, limit?:≤200}` → `{entries:[{group, name, version, date, ok, source}], more}` | same (read-only) |
| `update.options` | no | `{}` → `{unattended:bool\|null, snaps:{last,next}\|null, autoOpen:{supported, on, days}}` | same |
| `update.setAutoOpen` | yes | `{on:bool}` → `update.options` result | `{dryRun:true}` |
| `update.release.check` | yes (network through Ubuntu's checker) | `{}` → `{started:true}` | cached release state |
| `update.release.compat` | yes (apt download) | `{refresh?:bool}` → `{started:true}` | cached or `{state:'unknown'}` |
| `update.release.setPrompt` | yes | `{prompt:'lts'\|'normal'\|'never'}` → `{prompt, override?}` | `{dryRun:true}` |
| `update.release.ubuntuHold` *[review]* | yes | `{on:bool}` → `{on, codename}`; only while the verdict is `hold`/`blocked` (on) or always (off); writes the two update-manager keys of §4.12 | `{dryRun:true}` |
| `update.release.start` | yes | `{}` → `{started:true}`; refusals §3.7 | `{dryRun:true}`, no spawn |
| `update.release.attach` | yes | `{}` → `{started:true}` | `{dryRun:true}` |
| `shell.restartAll` (GX1, in `NativeCore`) | yes | `{reason:'libraries'\|'update'}` → `{ok}`; refused by the guard | `{dryRun:true}` |

*[review]* Every write method above (and a forwarded one) first runs the upgrader detection of §6.4 and returns
`update.guard.upgrade` while Ubuntu's release upgrader runs; the snapshot then has `phase:'releaseRunning'`. A
service whose test seam failed (§4.11) answers every method with `update.err.testSeam`.

Snapshot (`update.status`):

```json
{"backend":"packagekit|apt|null", "phase":"idle|checking|available|downloading|installing|waiting|foreign|
 restartRequired|error|unsupported|offline|repairRequired|releaseRunning", "rev":17, "checkedAt":1762341720000,
 "refreshing":false, "owner":{"pid":4242,"mirror":false},
 "rows":[{"id":"security","kind":"security","version":"26.04.1 LTS","count":12,"size":40265318,"security":true,
          "state":"downloading","pct":45,"error":null}],
 "packages":[{"id":"libsoup-3.0-0;3.6.6-1ubuntu0.3;amd64;ubuntu-resolute-security","name":"libsoup-3.0-0",
              "from":"3.6.6-1ubuntu0.2","to":"3.6.6-1ubuntu0.3","info":8,"summary":"…","row":"soup","phased":false,
              "pup":null,"held":false}],
 "totals":{"count":14,"security":12,"phased":2,"held":0,"downloadSize":40265318},
 "job":{"jobId":"…32 hex…","role":"update|repair|refresh","status":8,"pct":45,"speed":1200000,"remaining":20,
        "downloadLeft":22000000,"allowCancel":true,"committing":false,"startedAt":…,"reattached":false,
        "foreign":false,"confirm":null},
 "restart":{"reboot":true,"rebootPkgs":["gnome-shell"],"gnome":true,"shell":true,
            "shellParts":["soup"],"since":1762321000000},
 "error":{"code":"network","message":"…","key":"update.err.network","params":{},"detail":"…"},
 "warnings":[{"code":"source","message":"…","key":"update.warn.source","params":{"source":"dl.google.com"}}],
 "network":{"state":"full","online":true,"metered":false},
 "release":{"state":"none|checking|ok|warn|hold|ubuntuHold|blocked|updatesFirst|rebootFirst|running|waiting|
            startFailed|ended|unknown", "ubuntuPromptHeld":false, "startRc":null,
            "offered":false,"released":{"version":"26.10","codename":"stonking"},"version":null,"codename":null,
            "eol":null,"supportMonths":null,"notesUrl":null,"gnomeMajor":null,"reasons":[],
            "prompt":"normal","promptFile":"/etc/update-manager/release-upgrades","override":null,
            "helper":{"prompt":false,"attach":false},"checkedAt":…},
 "dryRun":false}
```

`packages` is only included with `{packages:true}` (the page asks once when it opens a row's details), so the
regular payload stays under 8 KiB. *[review]* `network.state` is NM's connectivity (`none|portal|limited|full|unknown`);
`online` is true only for `full` and `unknown` (connectivity checking off), so a captive portal or a limited link never
reads as online. `metered` is true for NM `Metered` YES (1) **and** GUESS_YES (3).

### 4.4 Events (prefix `update.` added to `events.rs:334-357` in GW2)

* `update.changed {rev, phase, totals, restart, job, error, release:{state, version}}`: structural changes only,
  ≤ 8 KiB, never package lists. Consumers call `update.status` when `rev` moves.
* `update.progress {jobId, status, pct, speed, remaining, downloadLeft, allowCancel, rows:{rowId:{state, pct}}}`:
  coalesced to at most 4 per second (250 ms timer on the pk context). *[review]* The owner's own page gets it through
  `emit_local` at that rate; the datagram fan-out (`emit`) carries at most **1 per second**, for mirror Settings
  windows, so panel, desktop, Start and Explorer parse at most one small datagram a second and never forward it to
  their WebViews.
* *[review]* `update.request {nonce, method, args}` (mirror → owner), `update.reply {nonce, error}` (owner → mirror) and
  `update.owner {pid, state:'gone'}` (owner shutdown), §4.1. Only `settings` hosts act on them.
* `system.updates.changed`: kept. *[review]* The owner emits it from the snapshot when the non-phased count changes;
  `info.rs` emits it only while no snapshot exists (§4.10).
* `shell.restartAll {nonce, origin, at}` (GX1, existing `shell.` prefix).

The datagram fan-out reaches every Win11 host. Reader-mode hosts ignore `update.progress`, so the event costs one
parse there.

### 4.5 State files (`~/.cache/win11-gnome/update/`, dir 0700, files 0600, atomic rename)

| File | Content | Lifetime |
|---|---|---|
| `snapshot.json` | `{version:1, rev, bootId, checkedAt, source, os:{versionId, version}, packages[], rows[], totals, warnings[], sizes:{id:bytes}, restart:{requireRestart:{type, pkgs, bootId}}}` (*[review]* `rev` persisted) | replaced on each GetUpdates and on each structural change |
| `owner.lock` / `owner.json` *[review]* | flock target held by the owning Settings for its life / `{pid, start, since}` | kernel releases the lock on exit; `owner.json` is only a hint for readers (liveness by pid + start) |
| `job.json` | `{version:1, jobId, tid, bootId, role, ids[], rows[], startedAt, owner:{pid,start}, simulate:{installs[], removals[], downloadSize}, confirm, committing, lastStatus, finishedAt?, outcome?:{exit, code, key, params, detail}}` | written **before** `UpdatePackages`, kept after `Finished` until the page has shown the outcome (`seen:true`), then moved to `jobs.jsonl` |
| `jobs.jsonl` | one line per finished job `{jobId, role, startedAt, finishedAt, outcome}` | 90 days (history match) |
| `lock` | flock target | – |
| `release.json` | detection result + `checkedAt` + last `check-new-release` time | replaced |
| `compat-<codename>.json` | verdict, reasons, gnomeMajor, archive date, `checkedAt` | 7 days or until a new extension version |
| `upgrade.json` | `{startedAt, bootId, version, codename, lingerByUs, seenRunning}` while a release upgrade is ours | removed when the upgrader is gone and no reboot is pending, after the card showed `startFailed`/`ended` if it applies; *[review]* also removed by the post-upgrade check (§6.6 step 6) |
| `$XDG_RUNTIME_DIR/win11-gnome/upgrade-status.json` *[review]* | `{rc, endedAt}` written by the terminal wrapper (§6.4) after pkexec returns | tmpfs, so it never outlives the boot; removed when `upgrade.json` is |
| `release-check/<codename>/` | private apt tree | deleted after each check |

In `~/.config/win11-gnome/`: `update-linger-by-us` (stamp, survives the cache) and `last-os-release.json`
(`{versionId, codename, gnomeMajor, extensionVersion}`, for the post-upgrade check). A job file from another boot
(`/proc/sys/kernel/random/boot_id` differs) is never used for reattach, only for the outcome and the history.

**Phase derivation** (`model.rs`, first match wins):
1. `unsupported`: no PackageKit name and not activatable, *[review]* or a failed test seam (`update.err.testSeam`);
   `backend:'apt'` then keeps the count.
1a. *[review]* `releaseRunning`: Ubuntu's release upgrader is detected (§6.4 step 5). It wins over everything below
   except an own job that is still running (step 2, whose progress keeps showing; `release.start` refuses while one
   runs, so this only happens when the upgrade was started elsewhere), because every write is refused then.
2. Own job: `waiting` (status 30 lock, 31 auth, 1/2 wait/setup before the download), `downloading` (status
   3/4/5/7/8/13/14/20–25/27 or the simulate step), `installing` (status 6/9/10/11/12/15/16/36 or `committing`).
3. A foreign transaction → `foreign`.
4. `repairRequired`, then `restartRequired`, then `error` (last outcome failed, until the next check or success),
   then `offline`, then `checking`, then `available` (non-phased rows), else `idle`.

### 4.6 The PackageKit flow (`pk.rs`)

* **Bus.** `gio::bus_get_sync(System)` on the pk thread, or the test address (§4.11). Manager signals
  `UpdatesChanged`, `TransactionListChanged`, `RestartSchedule` are subscribed with sender
  `org.freedesktop.PackageKit` (this does not activate; GLib ≥ 2.80 tracks the owner). Name presence:
  `NameHasOwner` + `ListActivatableNames` (as `packagekit.rs:277-297`).
* **Hints** on every transaction: `interactive=true` (required for polkit interaction), `background=false`,
  `locale=<nl_NL.UTF-8|en_GB.UTF-8>` only if `locale -a` (cached once) lists it. Otherwise there is no locale hint
  and the daemon's `LANG` applies. Here that means `en_GB.UTF-8` for en and no hint for nl.
* **Check.** `RefreshCache(false)` (polkit `system-sources-refresh` = yes) → status 7/21 → `Finished`. Then
  `GetUpdates(1<<1)` (filter none) collects `Package` and `Packages`. A failed refresh still runs `GetUpdates`;
  error codes 2/37/43 become `update.warn.source` when the detail names a non-Ubuntu URI and `GetUpdates` succeeds,
  else `update.err.network`. If the page is open, sizes come from one `GetDetails(ids)` (signal `Details a{sv}`,
  key `download-size`); *[review]* with the apt backend that key is very likely absent (§1.2), which is the normal
  case of §3.3.
* *[review]* **Phasing and held packages (ours, not PackageKit's).** The apt backend has no phasing code (§1.2), so
  GetUpdates very likely lists phased packages as normal (info 5) or security updates, and `UpdatePackages(ids)` would
  mark them explicitly and install them early, which Ubuntu's Software Updater never does. *[D-G73]* After every
  GetUpdates, `phasing.rs` asks apt itself with three batched, read-only programs from the tool table (each niced,
  idle I/O, 30 s timeout, environment cleared plus `LANG=C.UTF-8`): `apt-cache policy -o APT::Cmd::Pattern-Only=true
  <name:arch>…` (in GetUpdates order, deduplicated, names only: per package the installed version, the candidate and
  the candidate's `(phased N%)`), `apt-cache policy ?phasing` (the packages apt's own rule holds back on this machine: `pkgDepCache::PhasingApplied`
  with the machine id, `APT::Get::Always/Never-Include-Phased-Updates` and the `-security` exemption, the rule `apt
  upgrade` and unattended-upgrades follow; exit 0 with no output when nothing is phased, exit 100 on an apt older
  than 2.7.11) and `apt-mark showhold` (the selection holds). The three answers join on apt's package name (the bare
  name for the native architecture and `all`, `name:arch` for a foreign one). `?phasing` runs first, so a change of
  the lists between the calls always shows as a candidate that differs from the later answer or from PackageKit's
  version, and gets the safe rule. Listed by `?phasing` → `phased:true`; a
  hold or info 9 → `held:true`; apt's stored percentage is kept as `pup` for do-release-upgrade's coarser rule
  (§6.4; do-release-upgrade itself reads the raw field of the candidate's first record, which differs only for a
  literal 100, a malformed field or one version whose records disagree). Any exit code, timeout or spawn failure of one of the three runs the fallback, one `apt-cache show
  --no-all-versions <name>=<ver> …` call: any `Phased-Update-Percentage` counts as phased (do-release-upgrade's rule,
  conservative); a missing stub (test mode) is a seam error at once, with no further program. Phased and held
  packages never enter a row, the default `update.install` set or a simulate. Cost: `apt-cache policy '?phasing'`
  took 0.23 s on this PC; the classifier runs only in the owner, only after a check.
* **Install.** `UpdatePackages(SIMULATE|ONLY_TRUSTED = (1<<2)|(1<<1) = 6, ids)` → `Package` info 12/11/13/15
  (installing/updating/removing/obsoleting) and `DownloadSizeRemaining`. *[review]* Confirm-worthy in a simulate are
  removing 13, obsoleting 15, downgrading 20, remove 28, obsolete 29 and downgrade 30 (the typelib has all of them;
  install 27 is not). If any exists → `job.confirm = {kind:'removals', removals}`. If metered and `settings.update.metered` is false → `job.confirm = {kind:'metered',
  size}`. The UI answers with `update.confirm`. Then `UpdatePackages(ONLY_TRUSTED = 2, ids)`. The flags are fixed;
  callers never pass flags. The ids are only those in the current snapshot and never phased ones; anything else →
  `update.err.stale`.
* **Progress** comes from `PropertiesChanged` on the transaction path (`Status`, `Percentage` (101 = unknown),
  `AllowCancel`, `Speed`, `RemainingTime`, `DownloadSizeRemaining`, `LastPackage`) plus the `ItemProgress`,
  `Package`, `RequireRestart`, `ErrorCode`, `Finished` and `Destroy` signals. Subscriptions name the sender and the
  path. A 30 s watchdog source calls `Properties.GetAll` only when no signal came for 120 s. There is no busy loop and
  no time cap: a transaction ends on `Finished`, `Destroy` or the owner vanishing (`NameOwnerChanged`). A vanished
  owner gives `outcome unknown`, resolved by `GetOldTransactions` when the daemon is back.
* `committing = true` from the first status in {6, 9, 10, 11, 12, 15, 16, 36} or `AllowCancel` turning false after
  the download.
* **After `Finished`.** Exit 1 → `GetUpdates` again (rows whose packages are gone show "Geïnstalleerd"; the rest show
  "Installatiefout" after a failure) → restart scan → `update.changed`. Exit 11 or error 66 → `repairRequired`.
  Exit 3 → `available` + `update.err.cancelled` InfoBar. Error 27 (no-packages-to-update) counts as success.
  *[review]* **Success while rows remain** (exit 1, but GetUpdates still lists some of the job's packages, for example
  because libapt kept one back): those packages' rows go back to `pendingDownload`, the phase is `available` and
  there is no error; the installed ones show "Geïnstalleerd". Exit 9 / error 65 (cancelled-priority) →
  `update.err.locked`, error 39 → `update.err.conflict`.
* **Repair.** `RepairSystem(2)` (polkit `repair-system`, always a password) with the same events.
* **Enums** (pinned in `model.rs`, mirrored in `tests/update/enums.json`, compared with the typelib by GW1's test;
  the values below were read today from the local PackageKitGlib 1.3.4 typelib with `python3 -I`, no D-Bus):
  * Role: get-details 3, get-update-detail 8, get-updates 9, install-files 10, install-packages 11, refresh-cache
    13, remove-packages 14, update-packages 22, get-old-transactions 28, **repair-system 29**, upgrade-system 33.
  * Status: wait 1, setup 2, running 3, query 4, info 5, remove 6, refresh-cache 7, download 8, install 9, update 10,
    cleanup 11, obsolete 12, dep-resolve 13, sig-check 14, test-commit 15, commit 16, finished 18, cancel 19,
    download-repository 20, download-packagelist 21, download-filelist 22, download-changelog 23, download-group 24,
    download-updateinfo 25, loading-cache 27, waiting-for-lock 30, waiting-for-auth 31, run-hook 36.
  * Info: installed 1, available 2, low 3, enhancement 4, normal 5, bugfix 6, important 7, security 8, blocked 9,
    downloading 10, updating 11, installing 12, removing 13, cleanup 14, obsoleting 15, finished 18, *[review]*
    downgrading 20, preparing 21, decompressing 22, critical 26, *[review]* install 27, remove 28, obsolete 29,
    downgrade 30.
  * Exit: success 1, failed 2, cancelled 3, killed 6, need-untrusted 8, *[review]* cancelled-priority 9,
    repair-required 11. Restart: none 1,
    application 2, session 3, system 4, security-session 5, security-system 6. Network: offline 1, online 2, wired 3,
    wifi 4, mobile 5. Filter none = 1 (bitfield `1<<1`). Transaction flags (bit numbers): only-trusted 1, simulate 2,
    only-download 3, so ONLY_TRUSTED = `1<<1` = 2 and SIMULATE|ONLY_TRUSTED = 6.
  * Errors mapped in §3.7: 2, 5, 10, 13, 17, 25, 26, 27, 30, 31, 35, 36, 37, *[review]* 39, 43, 46, 48, 51, 56, 58,
    60, 61, *[review]* 65, 66, 67.
* **Not handled interactively:** `EulaRequired`, `MediaChangeRequired` and `RepoSignatureRequired` → `update.err.failed`
  with the detail. Conffiles: the apt backend runs `--force-confdef --force-confold` (H §4.3; R3 confirms).

The software path (`software/packagekit.rs`) stays as it is: it has a 20 ms loop, `locale=C.UTF-8`, a 2 h cap and
join-on-shutdown. Refactoring it onto `update/pk.rs` is a separate backlog item (§12 C12), so this feature never
touches a reviewed install/remove path.

### 4.7 Reattach, foreign transactions, cancel

1. On the first `update.*` call in Settings (LazyService create): if `job.json` exists, has the same boot id and has
   no `finishedAt`, call `GetTransactionList` with `DBusCallFlags::NO_AUTO_START`. `ServiceUnknown`/`NameHasNoOwner`
   means the daemon is gone, so go to step 4.
2. For each path, `Properties.GetAll` (NO_AUTO_START). Keep `Role == 22` (update-packages) or `29` (repair-system)
   and `Uid == getuid()`; prefer the file's `tid`.
3. Subscribe to it; rebuild rows from the job file plus `LastPackage`; `job.reattached = true`.
4. If the tid is gone, create a transaction for `GetOldTransactions(5)` and match the `Transaction` signal by path
   (`succeeded`, `role`, `data`). Write the outcome, run `GetUpdates` and the restart scan.
* `TransactionListChanged` with a role 22/11/14 transaction that is not ours → `job.foreign = true`, phase `foreign`,
  `installAll` disabled. Cancelling it (`cancel-foreign`) is never offered. *[review]* The Transaction interface has
  no sender property, so "not ours" means "not the tid in `job.json`"; that includes our own Software installer. When
  a Software job file of ours (`software.rs:91-109`, owner alive) is `running`/`authenticating`, the hero uses
  `foreignOwn` instead of `foreign` (§3.2).
* unattended-upgrades holds the dpkg lock without PackageKit: it shows as our status 30 or as error 26. A read-only
  `/proc` scan for `comm == "unattended-upgr"` picks `waitLockAuto`.
* **Cancel** is sent only while `AllowCancel` is true; otherwise `cannot_cancel`. PackageKit compares uids, so a
  reattached connection may cancel.
* Known gap (as in H): if Settings closes while polkit is still checking (status 31), the transaction ends
  `not-authorized`. For Robin this does not happen (no prompt). *[review]* Closing the window no longer ends the
  process during a job (keep-alive, §4.1), so this gap and the possible cancel-on-vanish only apply to a crash.

### 4.8 Restart detection (`restart.rs`)

1. **Reboot.** `gio::File::for_path("/run/reboot-required").monitor_file(NONE)` on the pk context (CREATED,
   CHANGED, DELETED), plus a read of `/run/reboot-required.pkgs`. `RequireRestart` types system 4,
   security-system 6, session 3 and security-session 5 are stored in the snapshot with the boot id. A new boot id
   clears them. Application 2 is ignored, because the scan covers our own processes.
2. **Scan** (on the pk thread, one file per step, at most 30 processes and 4 MiB per file):
   * our processes: `/proc/<pid>` with our uid whose `exe` link (minus " (deleted)") is in
     `current_exe().parent()` and named `win11-*`: the host binaries, the native shell and apps (`win11-shell`,
     `win11-gtk-*`) and their `--http-only` backends. *2026-11-14:* not `win11-calendar` (`restart.rs`
     `NOT_RESTARTED`): `shell.restartAll` cannot restart it (no backend, no event stream), so its replaced mappings
     kept the InfoBar up while it was open; it picks up replaced libraries when it is next started. *2026-11-13:*
     their children are no longer read. Until then the scan also read the direct WebKit children of a UI process
     (`comm` `WebKitWebProces` / `WebKitNetworkPr`, or an exe under WebKitGTK's helper directory), with a fallback to
     `history.log` entries newer than such a child's start when its maps were unreadable; our programs start no such
     helpers any more;
   * gnome-shell: our uid, `comm == "gnome-shell"`, cgroup containing `org.gnome.Shell@`;
   * only lines ending in ` (deleted)` count, matched against **watch lists**. Anything else is noise: `memfd:`,
     `/dev/shm/`, `/SYSV`, dconf, `gschemas.compiled`, `*.cache`, `gio/modules/`, gvfs, and libpcre2 (deleted in
     every process today).
   * `shell` list (`paths.rs` `SHELL_MAPS`, by component code): `libgtk-4.so` (gtk4), `libglib-2.0.so`,
     `libgobject-2.0.so`, `libgio-2.0.so`, `libgdk_pixbuf-2.0.so`, `libcairo.so`, `libarchive.so`, and a deleted own
     `win11-*` exe (self). These map to the components of §3.4. *2026-11-13:* `libwebkit2gtk-4.1.so`,
     `libjavascriptcoregtk-4.1.so`, `/webkit2gtk-4.1/` and `libsoup-3.0.so` left it with our WebKit hosts;
     *2026-11-14:* `libgtk-3.so` (gtk) and `libgdk-3.so` (gdk) with the host's GTK 3 (a GTK 3 update needs no Win11
     restart; GTK 4 does).
   * `gnome` list: `/usr/bin/gnome-shell`, `/usr/lib/gnome-shell/`, `libmutter-`, `/usr/lib/x86_64-linux-gnu/mutter-`.
   * Triggers: page open (if the last scan is older than 60 s), after our `Finished`, on `UpdatesChanged`, and on a
     FileMonitor event on `/var/log/apt/history.log` (0644).
3. This is what needrestart does. It also catches unattended-upgrades, which is how today's libsoup came in.

### 4.9 Power guard (`guard.rs`, called from `system/power.rs::action` and `shell.restartAll`)

* It blocks `reboot`, `shutdown`, `suspend` and `logout` (never `lock`):
  * while `job.json` says `committing` and its owner is alive (`owner {pid, start}` against `/proc/<pid>/stat`)
    → `ApiError("update_busy", update.guard.power)`; *[review]* if the owner is dead, the job is unfinished in this
    boot and `job.json` was written less than 10 min ago (the owner rewrites it on every status change and at least
    every 60 s while a job runs), the same refusal; an older unfinished job does not block, so a crash can never lock
    Robin out of restarting (packagekitd's own logind inhibitor, `power.rs:81-85`, still covers a real commit);
  * while a release upgrader process runs (§6.4 detection) → `update.guard.upgrade`.
* *[review]* For `shell.restartAll` (GX1) it also blocks:
  * while any own update job is active in any phase (`job.json` without `finishedAt`, owner alive) →
    `update.guard.restart`, because the Settings re-exec would end the owner;
  * while any Software job file is `queued`, `running` or `authenticating` with a live owner (`software.rs:91-109`,
    the same `active()` rule as `software.rs:788-790`) → `update.guard.software`, because `Software::shutdown` joins its
    workers (`software.rs:777-787`) and a restart would wait up to the 2 h cap.
* *[review]* The guard reads files and `/proc` only. It makes **no** D-Bus call, so it can run in the panel's reader
  mode (`system/power.rs`) and GW2's "panel-kind never connects" test holds.
* Reason: our logout is `gnome-session-quit --logout --no-prompt` (`power.rs:45-49`), which skips GNOME's inhibitor
  dialog, and reboot is `systemctl reboot` (`power.rs:39`). packagekitd's own logind inhibitor shows up as the
  existing "inhibited" error (`power.rs:81-85`).
* No idle inhibitor of ours (updates.md F3): the GNOME lock screen is gnome-shell itself, and Ubuntu's upgrader
  inhibits the screensaver itself (§1.2).
* *[review]* (The earlier "at most one NO_AUTO_START call" is dropped: see above.)

### 4.10 `system.updates` delegation (`system/info.rs`, GW2)

*[review]* `Updates::get` first reads `update/snapshot.json`. **Whenever a snapshot exists**, whatever its age, it
returns `{available: totals.count, checkedAt, tool:'packagekit', security: totals.security, checking:false,
rebootRequired}`, and `Updates::check` neither runs `apt list` nor emits `system.updates.changed` (the owner emits it,
§4.4). The page and Home show the snapshot's `checkedAt` as today ("Laatst gecontroleerd"). Only while no snapshot
exists (fresh install, never checked) does the existing `apt list` path run, plus `rebootRequired` = exists
`/run/reboot-required`. So a process never alternates between the two counting rules (the `apt list` count
includes phased packages, `info.rs:305-321`). `system.updates.changed` stays. No PackageKit call happens from `info.rs`, so Home, About and
sysmon in any process stay cheap. `system.openUpdater` stays (Geavanceerde opties).

### 4.11 Security, dry-run and test seams

* **No privilege beyond the user's own** (H §4.10): the token and origin protect the API, ids come only from the
  snapshot, flags are fixed, there is no InstallPackages/RemovePackages/repo/key method, `release.start` and `attach`
  run fixed argv, `setPrompt` takes an enum, and polkit decides.
* **Dry-run** (`ctx.dry_run || backend::dry_run()`): no D-Bus call to PackageKit, NetworkManager or logind on any
  bus, no subprocess (`loginctl`, `apt-get`, `check-new-release`, terminal, pkexec), and no gsettings write. Reads of
  files stay allowed.
* *[review]* **Why the test rules below are strict.** polkitd judges a uid-1000 process outside any session as Robin's
  active local session (§1.2), so a test that reaches the real system bus gets a **password-less** `UpdatePackages`,
  `RefreshCache`, `enable-linger` or reboot. Each of the following layers alone would stop that; together they make a
  wrong path, a missing variable or a driver click harmless.
* **Test seams** (`paths.rs`), honoured only when euid ≠ 0 **and** the directory contains a marker file
  `.win11-update-fixture`:
  * `WIN11_UPDATE_PK_BUS=<unix:path=…>`: PackageKit, NetworkManager and logind calls go to this private bus.
    The service then never opens the system bus at all.
  * `WIN11_UPDATE_FIXTURE_ROOT=<dir>`: re-roots `/run/reboot-required*`, `/etc/update-manager/`,
    `/var/lib/update-manager/`, `/var/lib/ubuntu-release-upgrader/`, `/usr/share/distro-info/ubuntu.csv`,
    `/var/log/apt/`, `/var/log/dist-upgrade/`, `/etc/os-release`, `/etc/apt/sources.list.d/`, *[review]*
    `/usr/share/keyrings/ubuntu-archive-keyring.gpg` (GW6's throwaway key), `/proc/locks` and the dpkg/apt lock
    files (§6.4), the soname directory `/usr/lib/x86_64-linux-gnu/` for the post-upgrade check (P03), and a fixture
    procfs `proc/<pid>/{comm,exe,status,cgroup,maps,cmdline}`.
  * `WIN11_UPDATE_TOOL_DIR=<dir>`: stubs for `loginctl`, `xdg-terminal-exec`, `pkexec`, `check-new-release`,
    `apt-get`, `apt-cache`, `apt-config`, `dpkg-query`, `snap`, *[D-G73]* `apt-mark` (`apt-cache` and `apt-mark`
    answer the phasing calls), `screen`, `nice`, `ionice` and `sh` (the wrapper). GSettings writes go through
    `gio::Settings`, which the tests point at the keyfile backend in the private HOME.
  * *[review]* **Fail closed.** "Test mode" = any `WIN11_UPDATE_*` variable is set. If any of them fails its check
    (marker missing, euid 0, a bus address that is not `unix:path=` under the test run dir, a missing directory), the
    service enters `unsupported` with `error.code = "test_seam"` (`update.err.testSeam`), opens **no** bus, starts
    **no** thread and runs **no** tool, for the whole process life. It never falls back to the system bus or to PATH.
    A unit test covers each failing variable.
  * *[review]* **One tool table.** Every program the service and `post_upgrade.rs` start is
    resolved through `paths.rs::Tools` (in `post_upgrade.rs`, its own constant table at the top of the file, because
    win11-launch does not link the lib crate). In test mode `Tools` points only into `WIN11_UPDATE_TOOL_DIR`; a stub
    that is not there is a `test_seam` error, never a PATH or absolute fallback. No literal absolute exec path may
    appear outside those two tables: GW2 adds a grep test over `host/src/services/update/` (GW7 adds the same test for
    `host/src/post_upgrade.rs` in that file's own test module) for `Command::new("/`, `"/usr/bin/`, `"/usr/lib/`, `"/bin/` and `"/usr/libexec/` (the tables are the allowed
    lines). This covers `/usr/lib/ubuntu-release-upgrader/check-new-release`, `/usr/bin/pkexec` and `/bin/sh`.
  * Under `cfg(test)`, `pk::system_bus()` panics unless `WIN11_UPDATE_PK_BUS` is set. A unit test can never reach
    the real daemon.
* *[review]* **No real system bus, no network, no setuid in any test run.** Every test that starts update code (GW1's
  `tests/update/run.sh`, GW2's bus tests, GW4–GW7 bus tests, GX1 and GW8 in `tests/gnome/`) runs inside a bubblewrap
  layer like GE0c's as built (`tests/gnome/harness.py:404-414`): `--ro-bind / /`, `--tmpfs /run/dbus` (the real
  system-bus socket is not there), `--tmpfs /run/user`, `--unshare-net` (no downloads, no abstract sockets of the live
  session), `--unshare-pid`, `--die-with-parent`, `--new-session`; bwrap also sets `no_new_privs`, so a setuid
  `pkexec` that slips through cannot elevate. Inside, `DBUS_SYSTEM_BUS_ADDRESS` of **every** process (gnome-shell,
  every Win11 host, workers, terminals) is the fake's private bus (GW1) or GE0c's empty private system bus, so
  `systemctl`, `loginctl` and PackageKit calls can only reach the fake. The env redirect alone is not enough, because a
  setuid program does not take its system bus from the caller's environment; that is why the bwrap layer is required,
  not optional. GE0b's non-bwrap sandbox stays for unit tests only; bus tests do not run there. GE0c's layer makes the
  real HOME read-only, so Rust bus tests are built outside it (`cargo test --no-run --offline --locked -j2` under the
  heavy lock, in GE0b's sandbox) and only the built test binary runs inside, with `tests/update/.run/<stamp>/` as the
  only writable bind.
* *[review]* **Stubs first on PATH for the whole harness session.** GE0c's stub list (§1.1) already has `pkexec`,
  `sudo`, `su`, `systemctl`, `systemd-run`, `loginctl`, `busctl`, `gnome-session-quit`, `apt`, `apt-get`, `pkcon`,
  `do-release-upgrade`, `update-manager`, `gnome-software`, `snap`, `shutdown`, `reboot`, `poweroff`; GW1 adds
  `xdg-terminal-exec`, `screen`, `apt-cache`, `apt-config`, `check-new-release`, `ptyxis`, `gnome-terminal`, `kgx` and
  `unattended-upgrade` to `tests/gnome/harness.py` `STUBS` (one line, coordinated with GE0c's owner). Every stub logs
  its argv and fails.
* *[review]* **Self-test after each run.** After every GW1 self-test, bus-test run, GX1 run and GW8 pass, the driver
  asserts that the stub log holds exactly the expected calls (for example one `systemctl reboot` after H10's "Nu
  opnieuw opstarten", nothing else) and that the fake's `CallCount()` equals the expected count. Any extra call fails
  the run.

### 4.12 Living next to Ubuntu's updaters

* unattended-upgrades keeps doing security updates; we show it, share the dpkg lock and read its history.
* The Software Updater (aptdaemon) stays one click away (Geavanceerde opties).
* update-notifier keeps running. D2: the toggle "Software-updater van Ubuntu automatisch openen voor gewone updates"
  *[review]* writes `com.ubuntu.update-notifier regular-auto-launch-interval`. Off writes 36500 and keeps the previous
  value in `settings.update.autoOpenPrevious`; on restores it (default 7). It uses `gio::Settings` with a schema
  lookup (the row is hidden when the schema is missing) and never autostart overrides. The initial state reflects the
  real value. *[review]* This key covers only regular updates: update-notifier still opens the Software Updater right
  away for security updates (the key's own description), and the row text says so (§3.6). Switching that off too
  would need an autostart `Hidden=true` override, which also hides Ubuntu's reboot and crash notices; that is not a
  default and only Robin can ask for it (R8). Whether the product switches the regular launch off by itself is also
  Robin's call (R8); the default is "leave as is".
* *[review]* **Ubuntu's own release prompt while we hold (C14).** `update.release.ubuntuHold {on:true}` (only from
  the `hold`/`blocked` card, only on a click) writes, through `gio::Settings` with schema lookups:
  `com.ubuntu.update-manager check-new-release-ignore = <codename>` (check-new-release-gtk then stays silent for that
  codename) and `com.ubuntu.update-manager check-dist-upgrades = false` (update-manager's own dialog then shows no
  "However, Ubuntu 26.10 is now available" upgrade button). The previous values go to
  `settings.update.ubuntuHoldPrevious {ignore, checkDist, codename}`. `{on:false}`, a verdict that becomes `ok`/`warn`,
  or a new codename restores exactly those values (only if the keys still hold what we wrote). Both keys are
  user-level and reversible; `/etc` and `Prompt=` are untouched, and the CLI `do-release-upgrade` stays available on
  purpose. The alternative lever is the existing Prompt row: `Prompt=lts` on 26.04 LTS makes Ubuntu read
  `meta-release-lts` (`MetaRelease.py:165-171`), so 26.10 is not offered at all (and our card disappears too).
* Settings defaults (`settings-defaults.json`, GW2): `"update": {"notifyRestart": true, "metered": false,
  "autoOpenPrevious": null, "ubuntuHoldPrevious": null}` *[review]*.

---

## 5. Restart all Win11 processes (GX1)

* `shell.restartAll {reason}` (NativeCore, any kind) calls `guard::power_blocked` (*[review]* an upgrader running, an
  own update job in any phase, or an active Software job, §4.9) and emits the event `shell.restartAll {nonce, origin:
  pid, at}`. Every Rust host registers `ctx.events.on("shell.restartAll", …)` in `NativeCore::new`, and each re-execs
  itself through the existing `request_shutdown(true)` after a per-kind delay, so the screen stays usable:
  desktop 0 ms → explorer, taskmgr, alles, disks, defaultapps, installer 300 ms → panel 1200 ms → the origin
  (Settings) 2000 ms. *[review]* There is no separate Start step: a Start whose parent is the panel (the prewarmed
  child, `lib.rs:515, 559-585`) ignores the event, because the panel SIGTERMs it on its own shutdown and prewarms a
  new one after its exec; restarting it too would start WebKit twice. While the panel is away, the extension shows
  its St fallback bar (`onPanelChanged(false)`, `extension.js:150-163`).
* *[review]* **Receivers re-check before they exec.** A job can start between the origin's guard check and a
  receiver's delay. Each receiver checks, just before `request_shutdown(true)`, the update job files and its **own**
  Software jobs; if one is active it defers its re-exec until that job ends (it subscribes to its own job's end, no
  poll), so `Software::shutdown`'s join never waits. The origin shows the normal InfoBar again if a receiver deferred.
* **Fix in `lib.rs`:** strip a trailing `" (deleted)"` from `current_exe()` before `exec`, and exec the path. Without
  this, a restart after a Win11 update fails, because the binary was replaced by rename (`install.py:93-99`).
* **Settings reopens on the update page:** the restart path accepts an argv override. Settings re-execs with
  `--app settings --params {"page":"update"}`. Explorer re-execs with its argv and restores its tabs from its saved
  window-state `session` (`window_state.rs:300-383`); GX1 verifies this.
* `win11-file-portal` (D-Bus activated) exits on the event and is activated again on the next request.
* Workers (index, image, software) end with their hosts' `runtime.shutdown()`, as on every restart today.
* We never signal another process: every process restarts itself.
* After a deploy of new binaries, the same InfoBar (deleted own exe) lets Robin pick up the new Win11 build without
  a re-login, as long as the extension did not change. *[review]* When the extension changed (round 1 changes it,
  GE1c), a re-login is required instead; the InfoBar cannot tell, so R3/R9 say so explicitly.

---

## 6. Release upgrade

### 6.1 Detection (`release.rs`, GW5)

1. **Prompt:** `/etc/update-manager/release-upgrades`, then `release-upgrades.d/*.cfg` sorted; the last `Prompt=`
   wins. `never`/`no` → never, `lts` → lts, anything else → normal. `lts` on a non-LTS release behaves as normal
   (Ubuntu's rule). We remember which file decided.
2. **Sources, cheapest first:** the stamp `/var/lib/ubuntu-release-upgrader/release-upgrade-available` (match only
   the version, `(\d\d\.\d\d(\.\d)?( LTS)?)`, because the text is in the timer's locale); the user cache
   `~/.cache/update-manager-core/meta-release(-lts)` and root's `/var/lib/update-manager/meta-release(-lts)`, the
   newer one wins. Parse them as `MetaRelease.parse` does: the first `Dist` with `Date` after the current release and
   `Supported: 1`, reading `Version`, `Name`, `ReleaseNotesHtml`, `UpgradeBroken`. The current release comes from
   `/etc/os-release` `VERSION_CODENAME`.
3. *[review]* **Only on a click** of "Controleren op updates", at most once per 24 h and not while metered (unless
   `update.metered` is on): `Tools.check_new_release -q` (real path
   `/usr/lib/ubuntu-release-upgrader/check-new-release`, only in `paths.rs`) as the user, nice 10, 60 s timeout,
   `LANG=C`. There is no run on page open, so a harness pass that only opens the page never downloads. Never in
   dry-run; in test mode only the stub. Exit 0 = new release, 1 = none; then read the caches again. **Agents never
   run it** (it uses the network).
4. **distro-info** `/usr/share/distro-info/ubuntu.csv` (read directly): `release` and `eol` dates for the card and
   the Prompt descriptions.
5. "Released but not offered" (today) → no card, `release.offered:false`, and `relNotOffered` under the Prompt row.

### 6.2 Compatibility check (`compat.rs`, `elf.rs`, `host/data/releases.json`, GW6)

**Inputs, all derived, none hand-kept except `releases.json`:**

* **Decisive:** the candidate `gnome-shell` version in the new archive (major = digits before the first `.`) against
  the **installed** extension's `shell-version` in
  `$XDG_DATA_HOME/gnome-shell/extensions/win11-gnome@win11shell.local/metadata.json` (today `["50"]`). GNOME's rule is
  a prefix match of the running major (`extensionSystem.js:409-411`, verified). A mismatch → `hold`.
* **Blocking packages:** the packages that own our binaries' NEEDED sonames, plus `libarchive13t64` (loaded at runtime,
  `README:76`). NEEDED comes from parsing `DT_NEEDED` of every `win11-*` next to `current_exe()` with `elf.rs`
  (64-bit LE ELF, `PT_DYNAMIC`, ~80 lines, tested against `readelf -d` on the `BINS` of a build directory,
  `tests/update/fixtures/compat/readelf-needed.txt`). The libc, libm (2026-11-13: the native programs), libgcc_s
  and ld-linux entries are skipped (the highest symbol GLIBC_2.39 is far below any new libc). Soname → package uses
  `dpkg-query -S /usr/lib/x86_64-linux-gnu/<soname>` (local, read-only). Today that gives libwebkit2gtk-4.1-0,
  libjavascriptcoregtk-4.1-0, libgtk-3-0t64, libsoup-3.0-0, libglib2.0-0t64, libgdk-pixbuf-2.0-0, libcairo2 and
  zlib1g. *2026-11-13:* without our WebKit hosts, and with the native shell and apps: libgtk-4-1,
  libgraphene-1.0-0, libpango-1.0-0, libglib2.0-0t64, libgdk-pixbuf-2.0-0, libcairo2 and zlib1g (libgtk-3-0t64
  until 2026-11-14, when the host left GTK 3) (`tests/update/fixtures/compat/dpkg-query-S.txt`); `compat.rs`
  `part_of` labels the GTK 4 three `update.part.gtk4`.
* **Degraded packages** (warn): ffmpeg, bubblewrap, squashfs-tools, flatpak, packagekit, a terminal (ptyxis or
  gnome-terminal or gnome-console), screen, xdg-terminal-exec, gnome-shell-ubuntu-extensions (the dock safety net).
* **`releases.json`** (embedded with `include_str!`, so there is no missing-file path):

  ```json
  {"version":1,"extension":"win11-gnome@win11shell.local","releases":{
    "resolute":{"version":"26.04","gnomeShell":"50","status":"tested"},
    "stonking":{"version":"26.10","gnomeShell":"51","status":"untested","allowAnyway":false}}}
  ```

  "51" is expected, **not verified** (no network). The archive check overrides it. A difference between
  `releases.json` and the archive gives a `warn` note for us, not for the user.

*[review]* **Cheap verdict first (C13/C16).** Before any download, `releases.json` and the installed `shell-version`
give a cheap verdict: if the codename's `status` is not `tested`, or its `gnomeShell` major is not in `shell-version`,
the verdict is `hold` and the apt tree below is **not** built (no 15–20 MB index download, no 250–400 MB apt RSS for
a known hold). Today that is the case for stonking. Only a codename that `releases.json` marks `tested` and whose
major matches gets the full check, which then confirms the real candidate major and the libraries. The full check
stays for future versions.

**How (H option A, rewritten for G):** a private, unprivileged apt tree `~/.cache/win11-gnome/update/release-check/<codename>/`
with `sources.list` derived from `/etc/apt/sources.list.d/ubuntu.sources` (Robin's mirror, suites `<c> <c>-updates
<c>-security`, *[review]* components copied from that file's Ubuntu stanza (here `main restricted universe
multiverse`) instead of a forced `main universe`, `Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg`, forced,
never taken from a caller; `paths.rs` re-roots the keyring path in test mode so GW6's throwaway key works) and **its
own `APT_CONFIG`** file `<tree>/etc/apt/apt.conf`:

```
Dir "<tree>/"; Dir::Etc "<tree>/etc/apt/"; Dir::Etc::Parts "<tree>/etc/apt/apt.conf.d/";
Dir::Etc::SourceParts "<tree>/etc/apt/sources.list.d/"; Dir::Etc::Preferences "<tree>/etc/apt/preferences";
Dir::Etc::PreferencesParts "<tree>/etc/apt/preferences.d/"; Dir::State::status "/var/lib/dpkg/status";
Dir::State::extended_states "/var/lib/apt/extended_states"; Debug::NoLocking "1"; APT::Architectures { "amd64"; };
Acquire::Languages "none"; Acquire::IndexTargets::deb::Contents-deb::DefaultEnabled "false";
Acquire::IndexTargets::deb::DEP-11::DefaultEnabled "false"; Acquire::IndexTargets::deb::CNF::DefaultEnabled "false";
```

(plus `Acquire::http::Proxy` copied when `apt-config dump` shows one). Then, as the user, under `nice -n 10 ionice -c3`:

1. `APT_CONFIG=<tree>/etc/apt/apt.conf apt-get -q update`, about 15 MB of xz, never the system's Post-Invoke hooks
   (the empty `apt.conf.d` is ours);
2. `apt-cache policy gnome-shell <blocking> <degraded>`: a candidate `(none)` is missing;
3. `apt-get -s -q dist-upgrade` (simulation; non-root works with NoLocking): `Remv <pkg>` lines for blocking
   packages without an `Inst` of a package that ships the same soname → `blocked`; the `Inst gnome-shell […] (51.x …)`
   line gives the candidate major. This is an approximation of do-release-upgrade (no quirks, no third-party).
4. The tree is deleted; only `compat-<codename>.json` stays. Peak about 85 MB on disk and 250–400 MB RSS for apt
   during the simulation. It runs at most once per release per 7 days, only when the release is offered, only when
   the cheap verdict is not already `hold` *[review]*, only while unmetered (or `update.metered` on), never in
   dry-run, and **never by an agent against the real archive** (R6). *[review]* Running even the private-tree apt
   against a `file:` fixture is "apt" under the plan's rules, so GW6 does it only after the coordinator writes an
   explicit exception into `plan.md` (inside bwrap `--unshare-net`, private tree, `file:` archive only; C17). Without
   it, GW6 tests the tree logic with recorded `apt-get`/`apt-cache` outputs only.

**Verdict:** `blocked` (a blocking package is missing or removed) → `hold` (GNOME major ∉ shell-version, or
`releases.json` status ≠ tested, D-G12) → `ubuntuHold` (UpgradeBroken) → `warn` (degraded missing or removed,
third-party sources counted from `/etc/apt/sources.list.d` minus `ubuntu.sources`) → `ok`. A failure gives `unknown`
with a reason. **Cheap variant** (if the coordinator drops the apt tree, −5 h): `releases.json` + GNOME major only,
which cannot see a renamed or dropped library. *[review]* The default is now "cheap first, full only when the cheap
verdict would allow the upgrade", which keeps the library check for the case where it matters.

### 6.3 The LTS/normal choice (`release.rs` + `admin.rs`, GW5)

`update.release.setPrompt {prompt}` → `pkexec /usr/libexec/win11-gnome/release-upgrades-helper set-prompt
<lts|normal|never>` (action `io.github.win11shell.gnome.release-upgrades`, auth_admin_keep). The helper (`/bin/sh`,
`set -u`, fixed `PATH`, `umask 022`):

* accepts exactly `set-prompt` plus one enum value;
* rewrites only the `Prompt=` line of `[DEFAULT]` (or appends it), keeping comments;
* writes `release-upgrades.win11-gnome.tmp` in the same directory, then `chown root:root`, `chmod 0644`, `sync` and
  `mv -f`;
* prints `@@ok prompt=<v>`, `@@override <file>` for each `.d/*.cfg` that sets Prompt, or `@@fail <code>`;
* honours `WIN11_HELPER_ETC=<dir>` **only when not root**, for the temp-etc tests. pkexec clears the environment
  anyway.

`admin.rs` checks the helper as `cleanup.rs:663-679` does (regular file, uid 0, no g/o write, parent the same).
Without the helper, it falls back to `pkexec /bin/sh -c '<constant script>' win11-gnome-prompt <value>`, and the
page shows `promptShInfo`. rc 126 → `update.release.err.noPassword`, rc 127 → `.wrongPassword` (only when no `@@`
marker was printed).

### 6.4 Hand-over to Ubuntu's upgrader (`upgrade.rs`, GW7; D-G12/D1)

`update.release.start {}`:

1. **Preconditions**, in Ubuntu's order:
   * release offered (`notOffered`);
   * a non-phased upgradable package exists (`updatesPending`): *[review]* do-release-upgrade's own, coarser rule
     (`do-release-upgrade:175-191`): any upgradable package whose candidate has **no** `Phased-Update-Percentage`
     record, taken from the snapshot's `pup` field (§4.6), so no extra call. *[D-G73]* `pup` is apt's stored
     percentage, so this matches do-release-upgrade's raw-record reading except for a literal
     `Phased-Update-Percentage: 100` (we block, it does not), a malformed value, or one version whose records disagree. Held packages count too, as in Ubuntu's
     check; if only held packages block, the card's `relUpdatesFirst` lists their names, because "Alles" will not
     install them;
   * `reboot-required.pkgs` has `libc6`, `linux-base` or `linux-image-*` (`rebootFirst`);
   * our job, a foreign PackageKit transaction, or *[review]* a held dpkg/apt lock (`busy`): `/proc/locks` (readable
     without root) is matched against the inodes of `/var/lib/dpkg/lock-frontend`, `/var/lib/dpkg/lock`,
     `/var/lib/apt/lists/lock` and `/var/cache/apt/archives/lock`. A comm scan is not used for this, because every
     Win11 host runs `apt list --upgradable` (comm `apt`, `info.rs:306`) until a snapshot exists; the comm
     `unattended-upgr` only picks the `waitLockAuto` text;
   * the verdict is not `blocked` or `hold` (`hold`; with `allowAnyway` only after the warning dialog).
2. Write `upgrade.json`.
3. **Linger:** `loginctl show-user <uid> -p Linger --value` → if `no`, run `loginctl enable-linger` (self, polkit
   `set-self-linger` = yes, no prompt) and write `~/.config/win11-gnome/update-linger-by-us`. Without linger, a
   gnome-shell crash or a sign-out ends the last session, and PID 1 stops `user@1000.service` after
   `UserStopDelaySec` (10 s) with `KillMode=mixed`, killing the root screen and dpkg too (H §5.4 review).
4. **Terminal:** *[review]* `xdg-terminal-exec --title=<update.release.terminalTitle> -- <Tools.sh> -c '<WRAPPER>'
   win11-gnome-upgrade <status-file> fail <Tools.pkexec> <Tools.do_release_upgrade>` (fixed argv: never `-f`, `-d`,
   `-p` or `--allow-third-party`, and no sudo; all three paths come from `paths.rs::Tools`). `WRAPPER` is one constant
   string in `upgrade.rs`, the same idea as `cleanup.rs`'s `/bin/sh -c <constant>`:

   ```sh
   status=$1; hold=$2; shift 2
   "$@"; rc=$?
   printf '{"rc":%d,"endedAt":%s}\n' "$rc" "$(date +%s)" > "$status.tmp" && mv -f "$status.tmp" "$status"
   if [ "$hold" = always ] || [ "$rc" -ne 0 ]; then printf '\n%s\n' "$WIN11_PRESS_ENTER"; read -r _; fi
   exit "$rc"
   ```

   It runs as the user; only pkexec's child is root. `status-file` is
   `$XDG_RUNTIME_DIR/win11-gnome/upgrade-status.json` (directory 0700, our own). `WIN11_PRESS_ENTER` carries
   `update.release.pressEnter` in the Win11 language. The hold is ours because Ptyxis has no hold argument (§1.2), so
   a pkexec dismissal (126), a wrong password (127) or do-release-upgrade's early refusals ("install all available
   updates", "not rebooted", "not possible right now", `do-release-upgrade:162-207`) stay readable. The environment
   is `files::actions::user_environment(true)` (*[review]* made `pub(crate)` by GW7, one line in
   `host/src/files/actions.rs:910`) plus `LANGUAGE=nl:en` (nl) or `LANGUAGE=en` (en), `WIN11_PRESS_ENTER`, an
   activation token for the default terminal's desktop id, and `cwd=$HOME`. It is started with `process_scope::spawn(command,
   "release-upgrade", Some(env))`: the `launch::spawn` path, Role::App, transient unit in **`app.slice`**, never
   `app-win11shell.slice`, whose MemoryMax 1000 MiB would apply to pkexec's root child and could OOM-kill dpkg.
   Ptyxis runs the command in its own `vte-spawn-*.scope`, also in `app.slice`. pkexec asks through GNOME Shell's
   agent (Ubuntu's own "Perform a release upgrade" text, en_US here), or through its text agent if no agent answers.
   Ubuntu's text frontend re-execs itself inside `screen -S ubuntu-release-upgrade-screen-window`.
5. **Watching**, every 5 s while `upgrade.json` exists *[review]* (and once on every update write and page open, for
   the guard and `releaseRunning`): a `/proc/*/cmdline` scan (any uid, read-only) for `do-release-upgrade`,
   `/tmp/ubuntu-release-upgrader-*/` or `DistUpgradeMain`, and the `SCREEN … -S ubuntu-release-upgrade-screen-window`
   daemon; the first hit sets `upgrade.json.seenRunning`. `main.log` mtime unchanged ≥ 10 min → `waiting`.
   *[review]* Outcomes, in this order:
   * upgrader gone and `/run/reboot-required` exists → card hidden, hero `restartRequired`;
   * never seen, and the status file has rc ≠ 0 → card `startFailed` with the rc reason (§3.5);
   * never seen, no status file, 120 s after the start → `startFailed` / `noWindow`;
   * seen, now gone, no reboot pending, and rc ≠ 0 or no status file → card `ended` with "Logboek openen";
   * rc 0, never or no longer running, no reboot pending → back to the offer state.
   In every outcome except the reboot one: `disable-linger` if the stamp is ours, then remove `upgrade.json` and the
   status file once the card has shown it.
6. **"Venster weer openen"** → `update.release.attach`: the same terminal path and wrapper (hold `fail`) with
   `Tools.pkexec /usr/libexec/win11-gnome/release-upgrade-attach` (`exec /usr/bin/screen -d -r
   ubuntu-release-upgrade-screen-window`, action `io.github.win11shell.gnome.release-upgrade-attach`, *[review]*
   **`auth_admin`, no keep**: the upgrader runs `screen -e \0\0` (`DistUpgradeMain.py:184-187`), so in the attached
   session Ctrl+Space then `c` opens a root shell, and a kept authorization would let any process of the session
   re-run the helper silently for 5 minutes). Running `do-release-upgrade` again mid-upgrade does not work (H review).
   Without the helper → `update.release.err.noAttachHelper` with the manual command; the user types sudo, our code
   never does.
7. During the upgrade: `shell.restartAll`, our power actions *[review]* and every update write (`update.check`,
   `.install`, `.confirm`, `.repair`, `.release.*` except `attach`) are refused with `update.guard.upgrade` (§4.3,
   §4.9); the hero shows `releaseRunning`, so the page offers no "Controleren op updates". The upgrader rewrites the
   sources and drops the apt lock between its phases, so a RefreshCache or install in a gap would mix the releases. No
   Win11 host is started by our own code paths. Explorer and Settings opened by the user still start (their libraries
   may be half-unpacked; the start dialog tells the user to save work and leave the window alone).

**GTK alternative (D1, not default):** `update-manager -c` → Upgrade, or `do-release-upgrade -f
DistUpgradeViewGtk3`, which runs `xhost +si:localuser:root` and re-execs through pkexec. It is Ubuntu's standard path
on GNOME, but it has no screen, so a gnome-shell crash ends it mid-dpkg. Only after R7.

**Hardening if R5 fails (§12 C9):** if R5 shows that GNOME's logout stops the terminal's `vte-spawn` scope (which
would kill root's screen even with linger), GW7b adds a third helper,
`release-upgrade-start` = `exec systemd-run --scope --unit=win11-gnome-release-upgrade --collect --
/usr/bin/do-release-upgrade`. Run as root, it moves the upgrader into a system-level scope that survives the user
manager. `PKEXEC_UID` is inherited, so Ubuntu still finds the user's session bus. It costs +2 h and changes the
polkit text to ours.

### 6.5 During the upgrade

The upgrade replaces GNOME Shell, GTK, WebKitGTK and our libraries under the running session, the same exposure
Ubuntu has. Our hosts do not import code at runtime (Rust), so there is no Python-stdlib risk. New WebKit children
of old UI processes may crash, which shows as blank pages; a reboot follows anyway. Settings, if restarted, finds
the upgrader again by the `/proc` scan and shows `running`.

### 6.6 After the upgrade: XDG autostart check outside the extension (GW7)

* `install.py` writes `$XDG_CONFIG_HOME/autostart/io.github.win11shell.PostUpgradeCheck.desktop` (and
  `uninstall.py` removes it):
  `Type=Application`, `Name=<Win11-controle na een Ubuntu-upgrade | Win11 check after an Ubuntu upgrade>` (plain Name
  in the current language, D-G4), `Exec=/usr/bin/env -- "<prefix>/bin/win11-launch" --post-upgrade-check`,
  `OnlyShowIn=GNOME;`, `NoDisplay=true`. *[review]* No `X-GNOME-Autostart-Delay`: under systemd the generator ignores
  it (§1.2); the ≤ 30 s `ShellVersion` retry below does the waiting. It exists always, not only after our upgrade,
  so it also catches upgrades done through Ubuntu's Software Updater.
* `win11-launch --post-upgrade-check` (new module `host/src/post_upgrade.rs`, `#[path]`-included by
  `bin/launch.rs`, GIO only, so it runs even when a library of our other programs is broken or the extension is
  rejected):
  1. Read `/etc/os-release` (`VERSION_ID`, `VERSION_CODENAME`, `VERSION`), the GNOME major from the `org.gnome.Shell`
     `ShellVersion` property (session bus, read, ≤ 30 s retries) or `dpkg-query -W gnome-shell`, and
     `~/.config/win11-gnome/last-os-release.json`. If there is no record: write it and exit. If nothing changed and
     there is no linger stamp: exit (≈ 5 ms).
  2. On a change: *[review]* first `org.gnome.shell disable-user-extensions` (GIO, read): true →
     `update.post.extsOff` with the action `extsOn` (writes `false`, only on that click) and `dockOn`. Otherwise
     `org.gnome.Shell.Extensions.GetExtensionInfo("win11-gnome@win11shell.local")` → `state`, *[review]* read as a
     double (`extensionUtils.js` packs numbers as `d`) and rounded; a missing or non-numeric state counts as ERROR.
     1 → `update.post.upgraded` once. 4 OUT_OF_DATE, or 2 INACTIVE while our uuid is in `org.gnome.shell
     enabled-extensions` *[review]* (the info's own `enabled` can be false after GNOME's crash rule) →
     `update.post.extOff`; 3 ERROR → `update.post.extError`. Both carry the action `dockOn`.
  3. Blocking sonames missing on disk (the §6.2 list) → `update.post.missing` with `installParts`.
  4. Notify through GNOME's `org.freedesktop.Notifications` (our extension may be off) and wait ≤ 10 min for
     `ActionInvoked`. `dockOn` → `org.gnome.Shell.Extensions.EnableExtension("ubuntu-dock@ubuntu.com")`, only on that
     click. *[review]* `installParts` → the §6.4 wrapper with hold `always`: `xdg-terminal-exec -- <sh> -c '<WRAPPER>'
     win11-gnome-parts <status> always <pkexec> <apt-get> install <names>`, where `<names>` are resolved **on the new
     release** first (a t64-style rename would make the old names fail): for each missing part, `apt-cache policy
     <old name>` (local, read-only); if it has no candidate, the single package listed under "Reverse Provides:" of
     `apt-cache showpkg <old name>`; if there is none or more than one, the old name stays and apt's own message
     explains it in the held window. apt asks Y/n in the terminal. The terminal is started through
     `process_scope::spawn` (*[review]* `#[path]`-included by `bin/launch.rs` next to `launch_context.rs`; it needs only
     glib/gio and `launch_context`, so win11-launch stays GIO-only, checked with `readelf -d`), so it gets its own
     transient unit in `app.slice` instead of living in the autostart unit, which is `PartOf=graphical-session.target`
     with `TimeoutStopSec=5s` and would be stopped at logout.
  5. If `update-linger-by-us` exists and no upgrader runs: `loginctl disable-linger`, remove the stamp.
  6. Write `last-os-release.json`, remove `compat-*.json`, `release.json` *[review]* and `upgrade.json`.
  * Language: `settings.json` `language` (default nl, D-G3). Texts: *[review]* `include_str!("../../i18n/host/update.tsv")`
    (the macro resolves from `host/src/post_upgrade.rs`, and the catalogues live at `G/i18n/host/`, rust-services.md F5)
    with a local 20-line lookup (no dependency on the lib crate; same rows as the host). The tool paths are the
    file's own constant table (§4.11).
* After a good upgrade, `releases.json` gets the codename as `tested` in the next Win11 version (from R5).

---

## 7. Root helpers, polkit policies and their installation (GW5)

| Repo file | Installed as | Mode | Action id (defaults any/inactive/active) |
|---|---|---|---|
| `host/data/release-upgrades-helper` | `/usr/libexec/win11-gnome/release-upgrades-helper` | 0755 root:root | – |
| `host/data/release-upgrade-attach` | `/usr/libexec/win11-gnome/release-upgrade-attach` | 0755 root:root | – |
| `host/data/io.github.win11shell.gnome.release-upgrades.policy` | `/usr/share/polkit-1/actions/` | 0644 | `io.github.win11shell.gnome.release-upgrades` (auth_admin / auth_admin / auth_admin_keep), `exec.path` = the helper |
| `host/data/io.github.win11shell.gnome.release-upgrade-attach.policy` | same | 0644 | `io.github.win11shell.gnome.release-upgrade-attach` (*[review]* auth_admin / auth_admin / **auth_admin**, no keep: the attached screen can open a root shell, §6.4 step 6), `exec.path` = attach |

Policy texts (default English, `xml:lang="nl"`):

* release-upgrades: description "Choose which Ubuntu versions are offered" / "Kiezen welke Ubuntu-versies worden
  aangeboden"; message "Win11 Shell wants to set whether you get only LTS versions or every new version of Ubuntu." /
  "Win11 Shell wil instellen of je alleen LTS-versies of elke nieuwe versie van Ubuntu krijgt."
* attach: description "Show the Ubuntu upgrade window again" / "Het venster van de Ubuntu-upgrade weer tonen";
  message "Win11 Shell wants to show the window of the running Ubuntu upgrade again." / "Win11 Shell wil het venster
  van de lopende Ubuntu-upgrade weer tonen."
* GNOME Shell's agent runs with `LANG=en_US`, so the prompt is English here even in Dutch mode (D-G11, documented).

**Install step:** *[review]* `scripts/install-system.py` (new) is **standalone**: `python3 scripts/install-system.py`
installs the system files and nothing else, `python3 scripts/install-system.py --remove` removes them. It never runs
`install.py`, never touches `~/.local`, the extension or GSettings, and `install.py`/`uninstall.py` never call it.
They only print one hint line in the installer language when the files are missing (install) or still present
(uninstall). Reason: `scripts/install.sh:5` drops its arguments, so the earlier `./scripts/install.sh --system-files`
would have run a full user-local install onto the live session and installed no system files, and `uninstall.py`
rejects any flag but `--files-only` (§1.1). A full uninstall (which disables the extension and writes GSettings)
and `--remove` stay separate commands.

* It refuses to run as root and refuses any argument except `--remove` and `--dry-run`.
* It stages the four files into a private temp dir under `$XDG_RUNTIME_DIR` with fixed names.
* It runs **one** `pkexec /bin/sh -c '<constant install script>' win11-gnome-system-install <stage>`. The script
  accepts only the four known names and installs with `install -o root -g root -m 0755|0644`.
* It prints in the installer language (GE1c) what it installs and why.
* `--dry-run` prints the argv and checksums only.
* Re-running is idempotent. Removal deletes exactly these four paths and the empty directory.
* `package.py` ships `host/data/`. Without the step, everything works except the nicer prompt text and attach (§6.3).
* The old `cleanup-helper` path (`/usr/libexec/win11shell/cleanup-helper`, used by `cleanup.rs`) is left alone. A
  different directory (`win11-gnome`) and id prefix avoid clashing with an old Hyprland Win11Shell install on the
  NucBox (§12 C5).

---

## 8. i18n carriers

* **UI:** `makeT({nl, en})` in `update.js` (hero, rows, banners, main-page dialogs, Meer opties shell),
  `update-release.js` (card, Prompt row, start dialog), `update-history.js` and `update-advanced.js`. Plurals use
  `{one, other}` with a numeric `count` next to a formatted `n` (GE1a's `core/i18n.js`). `list()` and `collator()`
  come from GE1a. Every key of §3 exists in both languages from the first commit.
* **Host:** `i18n/host/update.tsv` (stem `update`, `key<TAB>nl<TAB>en`, `{name}` placeholders, `.one/.other`),
  embedded via GE1b's `i18n.rs` (one catalogue line added by GW2). Call sites use `tr(lang, "update.err.network")`
  or `ApiError::tr("network", lang, "update.err.network", params)`, never a module-level text. Errors carry
  `key`/`params` (GE1b envelope), so the UI can re-render after a switch. Job and snapshot files store codes, keys
  and params, never rendered strings (the frozen-language lesson of `software.rs:471-615`).
* **win11-launch:** the same TSV through `include_str!` and a local lookup (§6.6).
* **Polkit policies:** default English plus `xml:lang="nl"`. The helpers print ASCII machine markers only.
* **Mock:** `mock/update.js` carries the host rows it needs (errors, compat reasons) as a small `{nl, en}` table. A check by GW3
  (`tests/i18n/test_update_mock_rows.py`) asserts that these rows equal `update.tsv`.
* **Glossary** (`tools/i18n/glossary.tsv`, GW3): Controleren op updates / Check for updates; Alles downloaden en
  installeren / Download & install all; Downloaden en installeren / Download and install; Opnieuw opstarten vereist /
  Restart required; Nu opnieuw opstarten / Restart now; Updategeschiedenis / Update history; Geavanceerde opties /
  Advanced options; Meer opties / More options; Wachten op downloaden / Pending download; Wachten op installatie /
  Pending install; *[review]* Wachten op opnieuw opstarten / Pending restart; Opnieuw proberen / Retry; Technische
  details / Technical details.
* **Checker:** H's `check.py` (ported in GE0a) covers the new dictionaries and the TSV. The ratchet gets no new
  allowances.

---

## 9. Tests

### 9.1 Fake PackageKit and fake NetworkManager (GW1)

* `tests/update/fake_packagekit.py` (Python 3, `gi.repository.Gio`, always run as `python3 -I`, about 600 lines).
  It owns `org.freedesktop.PackageKit` and optionally `org.freedesktop.NetworkManager` on a **private**
  `dbus-daemon --config-file=tests/update/bus.conf --nofork --print-address`. `bus.conf` has
  `<listen>unix:dir=…</listen>`, EXTERNAL auth and an allow-all policy, and **no `<servicedir>` and no
  `<standard_*_servicedirs/>`**, so nothing can be activated (no second at-spi bus).
* The manager implements `CreateTransaction`, `GetTransactionList`, `StateHasChanged`, the properties `Locked`,
  `NetworkState`, `BackendName`, and the signals `TransactionListChanged`, `UpdatesChanged`, `RestartSchedule`.
* Transaction objects (dynamic paths) implement `SetHints`, `GetUpdates`, `GetUpdateDetail`, `GetDetails`,
  `RefreshCache`, `UpdatePackages` (simulate and real), `RepairSystem`, `Cancel`, `GetOldTransactions`, every
  property of §4.6 with `PropertiesChanged`, and the signals `Package`, `Packages`, `ItemProgress`, `RequireRestart`,
  `ErrorCode`, `Finished`, `UpdateDetail(s)`, `Details`, `Transaction`, `Destroy`. All are emitted **broadcast**
  (destination None), like packagekitd, so the reattach test cannot pass for the wrong reason.
* The fake NM provides `Connectivity` and `Metered` with `PropertiesChanged`.
* **Control interface** `io.github.win11shell.FakePackageKit` at `/Control`:
  * `Load(s timeline)`;
  * `Step() → s` (advance one scripted event, for screenshots);
  * `Run()` (play the rest at the scripted pace);
  * `SetDaemon(b present)` (release the name to simulate "not running");
  * `Calls() → a(ssss)` (method, path, sender, args JSON);
  * `CallCount() → u`.
* **Timelines** (`tests/update/timelines/*.json`, *[review]* 31):
  * check states: idle-none, available-mixed (security, kernel, gnome-shell, webkit, gtk, soup, glib, other, 2
    phased), only-phased, refresh-partial-fail (third-party error 37 + successful GetUpdates), refresh-offline (NM
    connectivity 1), update-details (`UpdateDetails` with changelog, CVE and bug URLs);
  * install path: lock-wait (status 30 for 6 steps), auth-wait (31), not-authorized (48 + exit 2), download-install
    (ItemProgress per package, Speed/RemainingTime), cancel-download (AllowCancel true → Cancel → exit 3),
    cannot-cancel (commit phase);
  * restart, failures and repair: require-restart-system (RequireRestart 4), dpkg-fail-halfway (58, then GetUpdates
    lists the rest), repair-required (exit 11 / error 66, then RepairSystem ok), no-space (46), untrusted (exit 8),
    conflict (13), removals-in-simulate (info 13 *[review]* and 28, plus a downgrade 30), metered (NM Metered 1,
    NetworkState 5; *[review]* a second step with GUESS_YES 3);
  * *[review]* phasing and odd outcomes: phased-as-normal (GetUpdates lists a phased package with info 5; *[D-G73]*
    the `apt-cache` stub lists it under `?phasing`; the row and the install ids must not contain it),
    success-rows-remain (exit 1 while GetUpdates still lists one job package), cancelled-priority (exit 9 /
    error 65), install-blocked (error 39);
  * reattach and other callers: caller-vanishes-reattach, *[review]* vanish-cancels-download (the fake cancels a
    transaction with `AllowCancel` true when its caller's name vanishes, which is what R3 may show), finished-in-gap
    (GetOldTransactions), foreign-transaction (other uid, role 22), daemon-absent (NO_AUTO_START → ServiceUnknown),
    updates-changed (broadcast UpdatesChanged after an apt hook);
  * flood: 200 ItemProgress + PropertiesChanged per second for 60 s.
* `tests/update/run.sh` starts the bus and the fake inside `tests/update/.run/<stamp>/` (0700) with
  `NO_AT_BRIDGE=1 GTK_A11Y=none GSETTINGS_BACKEND=keyfile`, a private HOME/XDG dirs, and `DISPLAY`/`WAYLAND_DISPLAY`
  unset. It **refuses** when `XDG_RUNTIME_DIR` is `/run/user/<uid>` or when `DBUS_SESSION_BUS_ADDRESS` or the given
  bus address points under `/run/user/`. It exports `WIN11_UPDATE_PK_BUS`, `WIN11_UPDATE_FIXTURE_ROOT` (with the
  marker) and `WIN11_UPDATE_TOOL_DIR`, and kills only the pids it started. *[review]* It runs everything inside the
  bwrap layer of §4.11 (GE0c's `bwrap_argv` binds, `--unshare-net --unshare-pid`, private `/run/dbus` and
  `/run/user`), sets `DBUS_SYSTEM_BUS_ADDRESS` = the fake's bus for every child, puts GE0b's `guard-bin/` plus the
  update stubs first on PATH, and refuses to start when `/run/dbus/system_bus_socket` is visible inside. Any GPG home
  (GW6) lives under `tests/update/.run/<stamp>/gnupg`, never in /tmp. Heavy runs (cargo) take `flock -w 3600
  $G/.heavy.lock` only around the command itself, after the MemAvailable > 2 GB check.
* `tests/update/enums.json` + `test_enums.py`: the Rust table equals the typelib (`gi.require_version('PackageKitGlib',
  '1.0')`, no D-Bus), skipped when the typelib is absent.
* `tools/update/probe.py` (for R1): without flags it refuses to touch anything but a private bus. With
  `--live --get-updates [--simulate-all]` it runs `CreateTransaction` → `GetUpdates`, optionally `UpdatePackages
  (SIMULATE|ONLY_TRUSTED)` and `GetDetails`, and writes JSON. No RefreshCache, no real install. *[D-G73]* It has no
  `--phasing` any more: the classifier is apt's own read-only CLI, which anyone can run (`apt-cache policy
  '?phasing'`).

### 9.2 Rust tests

* **Unit tests (no bus), each in its own file's `mod tests`:**
  * grouping and phase derivation, with all phases from table-driven snapshots;
  * the throttle (≤ 4 events/s under the flood timeline replayed in-process);
  * snapshot/job round-trips, boot-id rules, the lock;
  * reattach decisions as pure functions;
  * the maps scan on fixture procfs (today's real noise: libpcre2, gvfs, memfd);
  * reboot-required parsing;
  * the history parser (`.gz`, `Error:`, unattended/PackageKit/apt Commandline, our time window);
  * the meta-release parser (fixtures: today's real shape, stonking supported/unsupported, UpgradeBroken, lts),
    Prompt precedence with `.d`, the stamp text, distro-info;
  * the compat verdict matrix with a faked apt runner (`apt-cache policy` and `apt-get -s` output fixtures);
  * the tree's `APT_CONFIG` (`apt-config dump` under it, read-only: no system `Post-Invoke`, no system `Dir::Etc::Parts`);
  * the ELF parser against `readelf -d` on the `BINS` of a build directory (2026-11-13; was `target/debug/win11-*`);
  * the rebootFirst and updatesFirst rules;
  * linger bookkeeping with a PATH-stub `loginctl` (enable only when off, disable only by us, the stamp survives a
    restart);
  * stalled `waiting` from `main.log` mtimes;
  * the power guard;
  * the helper scripts in a temp etc (as user, with `WIN11_HELPER_ETC`);
  * the post-upgrade decision table (fixture os-release / last-os-release / extension states, *[review]* state as a
    double, `disable-user-extensions` true, our uuid in `enabled-extensions` with `enabled:false`);
  * *[review]* fail-closed seams: each `WIN11_UPDATE_*` variable set but invalid → `test_seam`, no bus opened, no tool
    started (the tool stub log stays empty);
  * *[review]* the no-absolute-exec-path grep over `services/update/` and `post_upgrade.rs`;
  * *[review]* the phasing classifier: *[D-G73]* the parsers of `apt-cache policy`, of `?phasing` and of `apt-mark
    showhold`, the join on apt's package name, the fallback on every failure and the parser of `apt-cache show`; the
    same answers as the old python3-apt script on the recorded real outputs of tests/update/fixtures/phasing; and
    the rule that phased/held ids never reach a row or `UpdatePackages`;
  * *[review]* the wrapper outcomes (status-file fixtures × seen/not seen × reboot flag → `startFailed`/`ended`/offer)
    and the `/proc/locks` busy rule on a fixture `locks` file;
  * *[review]* the owner election as pure logic (lock won/lost, takeover, `rev` continuation).
* **Bus tests** (`#[ignore]`, run by `tests/update/run.sh cargo`, which wraps `flock -w 3600 $G/.heavy.lock cargo test
  --offline --locked -j2 services::update::bus_tests:: -- --ignored` *[review]* (a module path, not the substring
  `update::`)): every timeline through the real service in nl and en. The asserted messages come from `update.tsv`.
  * *[review]* **Two-services test:** two `settings`-kind services on the same fake and the same cache dir; only one
    owns; a `update.check` sent to the mirror is forwarded and gives exactly one `RefreshCache` in `CallCount()`; `rev`
    seen by both only grows; after the owner's shutdown the mirror takes over and reattaches to the running job; only
    the owner calls the fake Notifications server.
  * *[review]* **Upgrade-running test:** with a fixture procfs that shows `DistUpgradeMain`, every write returns
    `update.guard.upgrade` and the phase is `releaseRunning`; `CallCount()` stays 0.
  * **Dry-run test:** a dry Settings host opens the page and calls `update.check`, `.install`, `.cancel`,
    `.release.*`; `CallCount()` stays 0.
  * **Reader-mode test:** a `panel`-kind service never connects to the bus.
  * **Shutdown test:** `shutdown()` returns in < 100 ms while a fake transaction runs, and the next instance
    reattaches.

### 9.3 Every state screenshotted in nl and en (GW8, through GE0c's `tests/gnome/`)

The harness starts headless gnome-shell 50 with the extension in a private prefix (GE0c). For the update states it
starts `win11-app --app settings --params {"page":"update"}` (or `update-history`/`update-advanced`) with the three
test variables of §4.11 set for that process only. The private session has no systemd user manager, so
`process_scope` falls back to a plain spawn and inherits them. *[review]* The fake PackageKit and fake NM own their
names on GE0c's private system bus (`RT/system-bus`), which is already every sandbox process's
`DBUS_SYSTEM_BUS_ADDRESS`, so `WIN11_UPDATE_PK_BUS` is that same address; logind and systemd are absent there, and the
bwrap layer hides the real system bus (§4.11). For each state in `tests/gnome/update/states.json` the driver
(`tests/gnome/update/run.py`):

1. loads the timeline and steps it to the state;
2. writes the fixture files;
3. waits for `update.changed` (or 2 s);
4. screenshots with `org.gnome.Shell.Screenshot` on the private bus into `tests/gnome/update/shots/nl/<id>.png`;
5. rewrites the private `settings.json` language to en, waits 300 ms and screenshots `shots/en/<id>.png`.
6. *[review]* checks the stub log and `CallCount()` against the state's expected calls (§4.11 self-test).

The real system bus cannot be reached from the harness at all *[review]* (bwrap's private `/run/dbus`), so neither
PackageKit nor logind nor polkit can be.

| Group | States (id) |
|---|---|
| Hero (26) | H01 idle · H02 checking · H03 available (mixed rows) · H04 simulating · H05 downloading 45 % with Cancel · H06 installing 30 % · H07 waiting lock (unattended) · H08 waiting auth · H09 foreign · H10 restartRequired (gnome-shell, today's state) · H11 restartRequired + Ook beschikbaar · H12 error network + Technische details open · H13 unsupported (no PackageKit) · H14 offline · H15 repairRequired · H16 only phased · H17 metered InfoBar · H18 metered confirm · H19 partial refresh warning · H20 Win11 restart InfoBar (libsoup) · H21 removals confirm · H22 cancelled info · H23 cannot cancel · H24 not authorized · *[review]* H25 releaseRunning · H26 foreignOwn (own Software job) |
| Rows (5) | R01 pending download / downloading / pending install mix · R02 installing + installed · R03 install error with Retry per row · R04 row expanded with packages, "Wordt later aangeboden" and *[review]* "Wordt nu niet geïnstalleerd" · R05 "Wat is er nieuw" dialog |
| Card (16) | C01 released-not-offered line · C02 checking · C03 ok · C04 warn (expander open) · C05 hold GNOME 51 ("Wacht op een compatibele Win11-versie", *[review]* with "De Software-updater van Ubuntu kan deze upgrade nog wel aanbieden" and the postpone link) · C06 blocked · C07 updatesFirst · C08 rebootFirst · C09 start dialog (+ battery, metered, English-first) · C10 running · C11 waiting · C12 ubuntuHold · C13 check failed · *[review]* C14 startFailed (rc 126) · C15 ended with "Logboek openen" · C16 hold after "Ook de melding van Ubuntu uitstellen" |
| Meer opties (4) | M01 Prompt normal · M02 lts on an interim release · M03 overridden by `.d` (disabled) · M04 `/bin/sh` fallback InfoBar |
| Subpages (3) | S01 history with 3 groups · S02 history empty · S03 advanced options |
| Post-upgrade (4) *[review]* | GNOME banners from the real headless gnome-shell 50 (it owns `org.gnome.Shell.Extensions` and the notification server on the harness session bus, so no fake is possible there): P01 "Ubuntu is bijgewerkt naar 26.10" (a fixture `last-os-release.json` that names an older release) · P02 extension out of date + "Ubuntu Dock aanzetten" (a private extension copy with `shell-version ["49"]`, a real OUT_OF_DATE) · P03 missing components (fixture seam for the soname directory) · P04 "GNOME heeft extensies uitgezet" (`disable-user-extensions` true in the private keyfile) |

*[review]* Total: 58 states × 2 = 116 screenshots, reviewed against a real Windows 11 24H2 reference set (layout,
the single accent button, bar width, status wording; the 24H2 texts in §3 are from memory until then). Memory:
*[review]* the extension also starts panel, desktop and the preloaded Start, so plan for about 1.6–2.0 GB (headless
gnome-shell + three shell WebKits + Settings), under `flock` with the MemAvailable > 2 GB check and GE0c's sampler,
which aborts below 1.5 GB available. GW3 renders the same ids in mock mode (`?mock=1&updateScenario=<id>`) for the i18n
dumps (`tests/i18n/surfaces.json`).

### 9.4 Never tested by an agent

Real PackageKit (even reads), a real polkit prompt, a real install, `RefreshCache`, `check-new-release`, the real
archive download, a real Prompt change, real linger, a real `RepairSystem`, a real release upgrade, the GTK
frontend, the post-upgrade notification on a real GNOME 51, and gsettings writes outside a keyfile backend. These
are R1–R10. *[review]* Also never: the real phasing calls in a test (*[D-G73]* the tests use stubs and the recorded
outputs of tests/update/fixtures/phasing), and the private-tree apt run against anything but a `file:` fixture
inside bwrap, and that only with the plan exception of C17.

---

## 10. Steps GW1..GW9 (rounds 4–6 of plan.md), file ownership, done-when, effort

**Ownership rule:** within a round no two steps touch the same file. Later steps may only touch files listed for
them. A step that needs a new `update.tsv` row writes it to `notes/claude/update-tsv-additions-<step>.tsv` for the
coordinator to merge. *[review]* The same side-file rule covers the two shared i18n lists: new mock surfaces go to
`notes/claude/surfaces-additions-<step>.json` (for `tests/i18n/surfaces.json`) and new nl keys to
`notes/claude/baseline-additions-<step>.txt` (for `tests/i18n/BASELINE-ADDITIONS`, the list of **new** keys, next to
`BASELINE-CHANGES`, the list of **changed** keys); GW3 creates both target files. The coordinator alone regenerates
`vendor/ui-sha256.json`/`ui-patches.json` at the end of each round. Every Rust build runs `flock -w 3600
$G/.heavy.lock cargo build --offline --locked -j2` after the MemAvailable > 2 GB check.

*[review]* **Precondition (B1):** GW1, GW2 and GW8 (and GX1's harness run) start only after the coordinator accepts
§4.11's test rules: fail-closed seams, one tool table, bwrap for every bus test, stubs first on PATH, the
after-run self-test.

| Step | Round | Depends on | Files it may touch | Done when | h |
|---|---|---|---|---|---|
| **GW1** Fake PackageKit + NM, bus, control, timelines, probe | 4 (first) | GE0b (sandbox rules), GE0c (bwrap layer) | `tests/update/{bus.conf, fake_packagekit.py, run.sh, timelines/*.json, enums.json, test_enums.py, README.md}`, `tools/update/probe.py`, *[review]* `tests/update/stubs/*`, one line in `tests/gnome/harness.py` (`STUBS` additions of §4.11, agreed with GE0c's owner) | `run.sh --self-test` plays all 31 *[review]* timelines against a Python client **inside the bwrap layer** *[review]*; NO_AUTO_START with the daemon absent → ServiceUnknown; a second connection receives broadcast signals; the guard refuses a live environment *[review]* and refuses when `/run/dbus/system_bus_socket` is visible; the stub log check works; enums equal the typelib; probe without `--live` refuses the system bus | 9 *[review]* |
| **GW2** Service core | 4 | GE1b (`i18n.rs`, ApiError key/params), round 2 done for `system/*`; *[review]* GW2b after GW1 | new: `host/src/services/update.rs`, `update/{paths,pk,model,phasing,store,restart,net,guard,tests,bus_tests}.rs` *[review]* (`phasing`), stubs `update/{history,options,release,admin,compat,elf,upgrade}.rs`, `i18n/host/update.tsv` (all rows of §3.7); edits: `services/mod.rs`, `events.rs` (prefix), `api-methods.json` (all of §4.3 incl. `shell.restartAll` and `update.release.ubuntuHold` *[review]*), `services/system/info.rs`, `services/system/power.rs` (guard call), `settings-defaults.json` (`update` block), `i18n.rs` (one line) | *[review]* **GW2a** (parallel with GW1): unit tests green, incl. fail-closed seams, the exec-path grep, phasing parser, owner logic, wrapper outcomes, the `/proc/locks` rule; dry-run zero calls; reader mode never connects (also the power guard); Rust nl-frozen and ratchet green; offline `-j2` build of all 12 binaries. **GW2b** (after GW1 has landed, same round, serial): bus tests green for all 31 timelines in nl and en inside bwrap, the two-services and upgrade-running tests, shutdown < 100 ms, ≤ 4 local and ≤ 1 remote progress events/s under the flood | 22 *[review]* |
| **GW3** Main page + mock | 4 | GE1a (`core/i18n.js`, `format.js`), GE0a (dump harness) | `vendor/ui/js/apps/settings/pages/update.js`, new stubs `pages/update-release.js`, `pages/update-history.js`, `pages/update-advanced.js` (final exports, placeholder bodies), `vendor/ui/css/settings.css` (only the update block + new `.st-updrow*`), new `vendor/ui/js/mock/update.js`, `mock/index.js` (FILES), `mock/system.js` (`rebootRequired`), `mock/settings.js` (`update` defaults), `apps/settings/registry.js` (2 subpages + aliases), `apps/settings/main.js` (3 MODULES lines), `tests/i18n/surfaces.json`, `tests/i18n/BASELINE-CHANGES`, *[review]* new `tests/i18n/BASELINE-ADDITIONS`, new `tests/i18n/test_update_mock_rows.py`, `tools/i18n/glossary.tsv` | every §3.2–§3.4 state renders in mock nl and en (*[review]* incl. `releaseRunning`, `foreignOwn`, `keptBack`); *[review]* the nl dump differs from the baseline only by §3.9 rows (`BASELINE-CHANGES`) and by keys listed in `BASELINE-ADDITIONS`; the en dump has no Dutch (check.py, ratchet); the CSS diff is confined to the update blocks; `node --check` on all modules; mock screenshots reviewed against 24H2 | 14 *[review]* |
| **GW4** History, advanced options, D2 | 5 | GW2, GW3 | `update/history.rs`, `update/options.rs`, `pages/update-history.js`, `pages/update-advanced.js`, `mock/update.js` (history/options scenarios), `tests/update/fixtures/history/*`, *[review]* side files `notes/claude/{surfaces,baseline}-additions-gw4.*` | parser fixtures pass; toggles persist (settings keys; gsettings in a keyfile backend in a private HOME); snaps row from a `snap` stub; subpage mock states nl/en; *[review]* the D2 row text names "gewone updates" and the security exception | 7 |
| **GX1** Restart all Win11 processes | 5 | GW2 (API entry, guard) | `services/shell.rs`, `lib.rs` (restart argv override, " (deleted)" fix, *[review]* prewarmed Start ignores the event, receivers' re-check and deferral, the keep-alive one-liner if GW2 found one is needed), `bin/file_portal.rs` (exit on the event), `mock/shell.js` | in `tests/gnome`: desktop, panel (with a freshly prewarmed Start, *[review]* exactly one new Start process), Settings, Explorer (2 tabs) and Taakbeheer restart in order within 5 s; the fallback bar shows and hides; Settings reopens on `update`; Explorer restores its tabs; refused while a fake job is active (any phase) *[review]* and while a Software job is active; a receiver with its own Software job defers *[review]*; restart works after replacing the binaries in the private prefix; stub-log self-test | 6 *[review]* |
| **GW5** Release detection, Prompt, attach, system-file step | 5 | GW2, GE1c (installer `--lang`) | `update/release.rs` (*[review]* incl. `update.release.ubuntuHold`), `update/admin.rs`, new `host/data/{release-upgrades-helper, release-upgrade-attach, io.github.win11shell.gnome.release-upgrades.policy, io.github.win11shell.gnome.release-upgrade-attach.policy}`, new standalone `scripts/install-system.py`, `scripts/install.py` + `scripts/uninstall.py` (*[review]* one hint line each, no flag), `scripts/package.py` (ship `host/data`), `tests/update/fixtures/release/*`, `tests/update/test_helpers.sh` | parsers match fixtures incl. today's real cache shape; the helper passes temp-etc tests; the policies parse (Python `xml.etree`) *[review]* and the attach policy has no `_keep`; `install-system.py --dry-run` prints the exact pkexec argv *[review]* and the script refuses any other argument and never writes outside its stage dir; `check-new-release` only through a stub and only on the click path; `ubuntuHold` on/off round-trips both keys in a keyfile backend and restores only our own values; installer messages bilingual | 12 *[review]* |
| **GW6** Compatibility check + card | 6 | GW5; *[review]* the plan.md exception of C17 for the real-apt part | `update/compat.rs`, `update/elf.rs`, new `host/data/releases.json`, `pages/update-release.js`, `mock/update.js` (card scenarios), new `tests/update/mkarchive.py`, `tests/update/fixtures/compat/*`, *[review]* side files `notes/claude/{surfaces,baseline}-additions-gw6.*` | verdict matrix green (*[review]* incl. cheap-first: a non-`tested` codename never builds the tree); the apt tree in `bwrap --unshare-net` against a signed `file:` fixture archive (throwaway key re-rooted by `paths.rs`, GNUPGHOME under `tests/update/.run/` *[review]*) gives ok/warn/hold/blocked, *only with the C17 exception, else from recorded outputs*; components come from the fixture's `ubuntu.sources`; `apt-config dump` under the tree shows no system hook; ELF parser = readelf; card states C01–C16 *[review]* and the Prompt-row states M01–M04 *[review]* in mock nl/en | 12 *[review]* |
| **GW7** Hand-over + post-upgrade check | 6 | GW5, GW6 contract | `update/upgrade.rs`, new `host/src/post_upgrade.rs`, `bin/launch.rs` (flag, *[review]* `#[path]` include of `process_scope.rs`), *[review]* `host/src/files/actions.rs` (only `pub(crate) fn user_environment`), `scripts/install.py` + `scripts/uninstall.py` (autostart entry), `tests/update/fixtures/upgrade/*` | stub run records the exact argv (*[review]* wrapper + Tools paths) and env (`LANGUAGE`, `WIN11_PRESS_ENTER`) and that the spawn took the Role::App path; *[review]* wrapper rc fixtures give `startFailed`/`ended`/offer, and the wrapper holds on rc ≠ 0; linger stub rules; running/waiting from fixture mtimes; post-upgrade logic against a fake `org.gnome.Shell.Extensions` and a fake Notifications server (states 1/2/3/4 as doubles, *[review]* `disable-user-extensions`), `EnableExtension` only after the action; *[review]* `installParts` name resolution from `apt-cache` fixtures; win11-launch NEEDED unchanged (`readelf -d`) | 13 *[review]* |
| **GW8** Screenshot pass, review, docs | after 6 | all, GE0c | `tests/gnome/update/{states.json, run.py, shots/}`, this note's "as built" section, `README.md` update section (and `README.en.md` if it exists by then) | 116 *[review]* screenshots reviewed against 24H2; stub-log and `CallCount()` self-test green for every state *[review]*; independent review findings fixed or listed | 9 *[review]* |
| GW9 (optional) | later | GW3 | `shell/startmenu.js`, `update/options.rs`, `pages/update-advanced.js` | Start power badge + "Bijwerken en opnieuw opstarten"; notification actions; fwupd read-only row + `fwupdmgr update` in a terminal | 6 |
| GW7b (only if R5 fails, §6.4) | after R5 | GW7 | `host/data/release-upgrade-start` + its policy, `update/upgrade.rs`, `scripts/install-system.py` | VM: sign-out mid-upgrade no longer kills it | 2 |
| GX2 (not part of the feature) | separate | – | `extension/*` | extension ported and tested on GNOME 51, `shell-version` += "51", `releases.json` stonking → tested | 4–16 |

**Totals** *[review]*: GW1 9 + GW2 22 + GW3 14 + GW4 7 + GX1 6 + GW5 12 + GW6 12 + GW7 13 + GW8 9 = **104 h**
(range 88–122). Phase A (GW1–GW4 + GX1) is 58 h, Phase B (GW5–GW7) 37 h. The +15 h over the first version: test
isolation and self-tests (GW1 +2, GW2 +1), the own phasing classifier (GW2 +2), the owner lock (GW2 +2), the new
hero states (GW3 +1), Ubuntu's prompt hold and the new card and Prompt-row states (GW5 +1, GW6 +1), the upgrade
wrapper and its states (GW7 +2), the restart guard and Start handling (GX1 +2), and the larger screenshot set
(GW8 +1). updates.md had 84 h.

**Round contents (disjoint):**
* Round 4: GW1 (`tests/update`, `tools/update`, one `STUBS` line in `tests/gnome/harness.py`) ∥ GW2a (host + TSV,
  unit level) ∥ GW3 (vendor/ui, tests/i18n, glossary); then *[review]* GW2b (bus tests) after GW1 has landed. GW1 goes
  first because GW2b needs its fake; the schedule risk is GW1's 9 h before GW2's last third.
* Round 5: GW4 ∥ GX1 ∥ GW5 (history/options + subpages + mock/update.js | shell.rs, lib.rs, file_portal, mock/shell.js |
  release/admin + host/data + scripts).
* Round 6: GW6 ∥ GW7 (compat + update-release.js + mock/update.js + releases.json | upgrade.rs + post_upgrade.rs +
  launch.rs + install.py/uninstall.py).
* Then GW8.

---

## 11. Robin-only actions

| # | Machine | When | Exact command / action | Why only Robin |
|---|---|---|---|---|
| R1 | this PC | after GW1 | `cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome && python3 -I tools/update/probe.py --live --get-updates --simulate-all --out tests/update/probe-results/$(date +%F).json` | activates root packagekitd on the real bus (no refresh, no install). Answers: what GetUpdates reports for a phased package *[review]* (the design no longer depends on it), removals in an update simulation, `data` field for the security row, whether `Details` has `download-size` *[review]*, count against `apt list --upgradable`, and *[review]* the time and RSS of the real phasing call (*[D-G73]* no longer through probe.py: the classifier is apt's own `apt-cache policy`, and `apt-cache policy '?phasing'` took 0.23 s on this PC) |
| R2 | this PC | after GW5 is reviewed | *[review]* `cd /home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome && python3 scripts/install-system.py --dry-run && python3 scripts/install-system.py` (one password; installs only the four system files, no user-local install), then `ls -l /usr/libexec/win11-gnome/ /usr/share/polkit-1/actions/io.github.win11shell.gnome.*`. Undo: `python3 scripts/install-system.py --remove` | writes system files |
| R3 | this PC | after Phase A is deployed | deploy: `WIN11_BUILD_DIR=/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome/target/release ./scripts/install.sh`, then *[review]* **log out and in** (round 1 changed the extension, so "Win11 Shell opnieuw starten" is not enough). Settings › Update en herstel › Controleren op updates → Alles downloaden en installeren; during the commit `systemd-inhibit --list > tests/update/probe-results/r3-inhibit.txt`; close the Settings window during the download *[review]* and check with `pgrep -a win11-app > tests/update/probe-results/r3-keepalive.txt` that the Settings process stays until the job ends. *[review]* On a later update, to settle C2: while the page still says "Downloaden", end that Settings process (`pkill -TERM -f -- '--app settings'`), reopen Settings and note whether the job went on (reattach) or ended `cancelled` (nothing is half-installed in the download phase). Afterwards `journalctl -u packagekit --since -30min > tests/update/probe-results/r3-journal.txt` | the first real system change; also confirms no prompt for him, conffile behaviour and packagekitd's own inhibitor, *[review]* and whether packagekitd cancels a transaction whose caller vanished (settles C2) |
| R4 | this PC | after R2 | Settings › Update en herstel › Nieuwe Ubuntu-versies → "Alleen LTS-versies" (password) → `grep -n '^Prompt' /etc/update-manager/release-upgrades` shows `Prompt=lts` → back to "Elke nieuwe versie" (or keep lts, see decision 4) | password; changes /etc |
| R5 | VM | after GW7, before the card is used on this PC | VM with Ubuntu 26.04.1 desktop (Robin downloads the ISO; `virt-manager` or GNOME Boxes). In the VM: unpack the 0.8.0 tarball, `./scripts/install.sh && python3 scripts/install-system.py` *[review]*, log out and in, `gnome-extensions enable win11-gnome@win11shell.local`. Use a VM-only build whose `releases.json` marks stonking `tested` with `allowAnyway:true`. Card → Downloaden en installeren; mid-download close the terminal and use "Venster weer openen"; mid-dpkg from VT3 `sudo pkill -KILL -x gnome-shell`, log in again and check `sudo screen -ls`; then sign out normally mid-dpkg once (tests the vte scope, §6.4 C9). After the reboot: the post-upgrade notification shows, "Ubuntu Dock aanzetten" works, `loginctl show-user $USER -p Linger` = no. *[review]* Also once: dismiss the pkexec dialog (card shows "De upgrade is niet gestart", the terminal stays open), and answer "no" at Ubuntu's first question (note the exit code in the status file). Write the results to `notes/claude/r5-vm-report.md` | ISO download, hours, a real upgrade; never first on his own PC |
| R6 | this PC | only once Ubuntu offers 26.10 (today not offered) | Settings › Update en herstel › card → let the check run (about 15 MB from nl.archive.ubuntu.com) | a real network download of the archive indexes |
| R7 | VM (optional) | only if D1's GTK path is wanted | in the VM: `update-manager -c` → Upgrade | pkexec, a real upgrade start |
| R8 | this PC | after Phase A deploy | decide D2: leave Settings › Geavanceerde opties › "Software-updater van Ubuntu automatisch openen voor gewone updates" *[review]* on, or switch it off (writes `com.ubuntu.update-notifier regular-auto-launch-interval 36500`, reversible). *[review]* Security updates still open the Software Updater right away either way; stopping that too needs an autostart override that also hides Ubuntu's reboot and crash notices, so it is only done if Robin asks for it | his session's settings |
| R9 | this PC | final deploy (plan round 7) | back up `~/.local/bin/win11-*` and `~/.local/share/win11-gnome`, install 0.8.0, R2 if not done, log out and in (the extension changes for English need a re-login anyway) | live session |
| R10 *[review]* | this PC | before the card is used on this PC (after GW6) | decide C14: while the card says "Wacht op een compatibele Win11-versie", should Win11 also postpone Ubuntu's own prompt **by itself** (writes the two update-manager keys of §4.12 automatically while on hold), only on Robin's click (default), or never; or switch this PC to "Alleen LTS-versies" (R4) so Ubuntu offers 26.10 nowhere | his session's settings and his upgrade path |

---

## 12. Decisions (defaults used in this design)

| # | Decision | Default here | Who |
|---|---|---|---|
| C1 | Where the service runs | full mode only in the Settings host *[review]* that owns `update/owner.lock`; mirrors forward; reader mode elsewhere | coordinator |
| C2 | Settings closed during a job | *[review]* the window closes, the host process stays without a window (`Application::hold`, about 30–50 MB) until `Finished`, sends the notification, then exits; no join; reattach only after a crash or re-exec. Reason: whether packagekitd cancels on caller vanish is unverified (R3 settles it; if it never cancels, the keep-alive may be limited to the download phase) | coordinator |
| C3 | D-G12 "Opnieuw opstarten vereist when gnome-shell or our libraries were updated" | gnome-shell/mutter → hero `restartRequired`; our libraries → InfoBar with the Win11 restart (seconds, no reboot); a pending reboot hides the InfoBar *[review]* except when WebKitGTK/JavaScriptCore changed (new web processes of old UI processes break now). Making libraries drive the hero too is a one-line change in `model.rs` | coordinator |
| C4 | Untested GNOME major | blocks, no "Toch upgraden…" (D-G12); `releases.json allowAnyway` per codename can enable H's hold-with-warning | coordinator / Robin |
| C5 | System file names | `/usr/libexec/win11-gnome/`, ids `io.github.win11shell.gnome.*` (not H's `nl.win11shell.*`, to avoid an old H install on the NucBox); the cleanup helper stays as is | coordinator |
| C6 | Fake PackageKit language | Python + Gio (no new crate, no lockfile under `--locked`) | coordinator |
| C7 | `{product}` | "Win11 Shell", the current Settings name; branding stays a separate decision | Robin (gnome-map §6 q7) |
| C8 | History groups | H's "Updates / Ubuntu-versies / Software"; Windows' "Kwaliteitsupdates / Onderdelenupdates / Andere updates" is the alternative for the 24H2 check | coordinator |
| C9 | Upgrade survival | D-G12: terminal + screen + temporary linger; GW7b (`systemd-run --scope` start helper) only if R5 shows that a sign-out kills it | coordinator after R5 |
| C10 | "Nu opnieuw opstarten" | through the panel's power flow (confirm + fade, like Start); Windows itself does not confirm | coordinator |
| C11 | Page title | keep "Update en herstel" (H Q12) | coordinator |
| C12 | `software/packagekit.rs` | untouched in this feature (its 20 ms loop, `C.UTF-8` hint, 2 h cap and join stay; separate backlog item) | coordinator |
| C13 | Compat check depth | full (apt tree + simulation, 11 h) vs cheap (`releases.json` + GNOME major, 6 h). *[review]* Default now: cheap first, the apt tree only when the cheap verdict is not `hold` (C16) | coordinator |
| C14 *[review]* | Ubuntu's own release prompt while our card holds (M2) | the card says that Ubuntu's Software Updater may still offer the upgrade and offers "Ook de melding van Ubuntu uitstellen" (writes `check-new-release-ignore=<codename>` and `check-dist-upgrades=false`, user GSettings, restored when the verdict becomes ok); only on a click. Alternatives: automatic while on hold, or recommend `Prompt=lts`, or leave Ubuntu's offer alone | Robin (R10) |
| C15 *[review]* | Two Settings processes (M3) | owner election by `flock` on `update/owner.lock`, mirrors forward writes by event; alternative: make Settings single-instance (`lib.rs` flags + a command-line handler for `--params`, closer to Windows, but it touches GApplication activation for every Settings deep link) | coordinator |
| C16 *[review]* | Compat cost vs value (m8) | the apt-tree part runs only when `releases.json` says `tested` and the major matches; today stonking is a known hold, so nothing is downloaded | coordinator |
| C17 *[review]* | GW6's private-tree apt runs (m23) | they need an explicit `plan.md` exception (bwrap `--unshare-net`, private tree, `file:` fixture archive only); without it GW6 uses recorded apt outputs only and the real run moves to R6 | coordinator, before GW6 |
| C18 *[review]* | Test isolation (B1) | §4.11's rules are a precondition for GW1, GW2, GX1's harness run and GW8 | coordinator |
| C19 *[review]* | `install.sh` passthrough (M4) | R2 no longer needs it (`install-system.py` is standalone), but GE1c's installer `--lang` does: GE1c's file list must include `scripts/install.sh` (`exec … "$ROOT" "$@"`) | coordinator (plan.md) |
| D2 | update-notifier auto-open | toggle exists *[review]* for regular updates only, with honest text; the product does not change it by itself; a full off (autostart override) only on Robin's request | Robin (R8) |
| 4 | Prompt for this PC | stays `normal` (as now); `lts` would mean an extension port every 2 years instead of every 6 months | Robin |

---

## 13. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| 26.10 ships GNOME 51 (expected, unverified) and the extension says `["50"]` | after an upgrade: no Win11 taskbar, no dock (disabled), only Activities | GNOME-major hold (D-G12), GX2 before "ok", the post-upgrade autostart check with "Ubuntu Dock aanzetten" |
| WebKitGTK/libsoup security updates under a running UI (live today) | new web processes of old UI processes crash (blank Start/panel) | maps scan → InfoBar → `shell.restartAll` (GX1) |
| A sign-out or gnome-shell crash during a release upgrade | dpkg killed half-way | screen + temporary linger; GW7b if R5 shows a scope stop; power guard on our logout |
| `apt-get -s dist-upgrade` in the private tree is not the upgrader | a dropped library missed, or a false alarm | both the candidate check and the simulation; Ubuntu's own pre-checks; R5 |
| Three daemons on one dpkg lock (aptdaemon, PackageKit, unattended-upgrades) | jobs wait for minutes | status 30 with its own text; no timeout; cancel while waiting |
| update-manager auto-opens every 7 days next to our page | duplicate UI | D2 toggle (R8) *[review]*; for security updates it still opens at once, which the toggle text says |
| *[review]* Ubuntu's own Software Updater offers 26.10 while our card holds | Robin upgrades to an untested GNOME through Ubuntu's GTK path (no screen, so a gnome-shell crash also kills it mid-dpkg) | card note + "Ook de melding van Ubuntu uitstellen" (C14, R10); `Prompt=lts` as the strong lever; the post-upgrade check as the last net |
| The PackageKit apt backend differs from the Software Updater (conffiles, phasing, deb2snap) | *[review]* phasing: the backend has none, so without our classifier "Alles" would install phased packages early | *[review]* own classification with apt's rule (§4.6), phased ids never sent; R1 confirms; conffiles in R3; Software Updater one click away |
| *[review]* Two Settings processes at once | duplicate checks, notifications twice, flapping `rev` | owner lock, mirrors forward, `rev` persisted (§4.1) |
| polkit prompt in English (gnome-shell `LANG=en_US`) for a Dutch UI | language mismatch in one dialog | documented (D-G11); policies carry nl for other agents |
| A memory spike: packagekitd 150–300 MB plus apt simulation 250–400 MB on a 3.5 GB-free machine | earlyoom pressure | the check runs only on demand and only when offered; nice/ionice; never two at once (the lock) |
| A test escapes to the real daemon | *[review]* a **password-less** real update, linger change, `check-new-release` download or reboot of Robin's PC (polkitd treats a sessionless uid-1000 process as his active session) | *[review]* fail-closed seams; one tool table that points only at stubs in test mode (grep-enforced); bwrap layer with private `/run/dbus`, no network and `no_new_privs` for every bus and harness test; `DBUS_SYSTEM_BUS_ADDRESS` = fake bus for every process; stubs first on PATH; `check-new-release` click-only; stub-log and `CallCount()` self-test after each run; `cfg(test)` panic without the private bus; dry-run makes zero D-Bus calls |
| *[review]* A restart of all Win11 processes during a Software job | the restart waits for the job (join, up to 2 h); a half-stopped Start | `update.guard.software`; receivers defer their own re-exec; the prewarmed Start follows the panel |
| Agents editing the same file in one working tree | lost edits | §10 ownership table; TSV additions via side files; the coordinator owns the manifests |

---

## 14. What this note did not check

* No PackageKit call of any kind, so the real `GetUpdates` output, the phasing representation, removals in
  `UpdatePackages`, the `data` field and packagekitd's inhibitor mode are unverified (R1/R3).
* Enum values were read from the local typelib (§4.6); whether packagekitd 1.3.4 on Ubuntu emits every signal listed
  there (for example `ItemProgress` per package during download) is unverified until R1/R3.
* The 26.10 archive (no network): its gnome-shell major, soname package names, and the upgrader tarball's behaviour
  on GNOME.
* Whether GNOME's logout stops `vte-spawn-*.scope` units in `app.slice` (C9, R5).
* Whether ptyxis is single-instance here, so the upgrade runs in ptyxis-agent's scope rather than our transient
  unit. Either is in `app.slice`, so the memory-cap reasoning holds.
* The real 24H2 wording of the busy hero states ("Updates downloaden"/"Updates installeren" are H's; 24H2 may keep
  "Updates beschikbaar" during the install). Settled by GW8's comparison.
* GE0c's harness API: `tests/gnome/` did not exist yet when this was written. §9.3 states what it must offer
  (per-process extra env, private Screenshot, language rewrite). *[review]* By the revision, `tests/gnome/harness.py`
  exists (bwrap layer, private system bus, stubs, private Screenshot, language rewrite, guard self-test); a
  per-process extra env for one Settings launch and the extended `STUBS` line are still to be added (GW1/GW8).
* The NucBox (Ubuntu 26.04 Dutch): whether packagekit, xdg-terminal-exec and screen are installed there, and its
  polkit setup.
* *[review]* Whether packagekitd cancels a transaction whose caller leaves the bus (C2, R3), and the text frontend's
  exit codes after it has started (R5).
* *[review]* The real GetUpdates output for a phased package and the real cost of the phasing call (R1). The design
  classifies phasing itself, so neither changes the flow. *[D-G73]* The phasing calls are now three apt programs;
  `apt-cache policy '?phasing'` took 0.23 s on this PC.
* *[review]* The Windows 11 24H2 texts in §3 (including the review's changes "Pending restart", "There were problems
  installing some updates" and the advNotify sentence) are from memory; GW8 checks them against a reference set.
* *[review]* That GE0c's bwrap layer, as it stands after round 0, keeps its current binds when GE0c is finished; the
  rules of §4.11 are written so GW1 can reproduce them in `tests/update/run.sh` if it changes.

---

## 15. Review outcome *[review]*

Source: `notes/claude/windows-update-gnome-critique.md` (2026-11-05). Every blocker and major finding is fixed in the
text above; every minor finding is fixed, or the line says why not. "§" points to the changed places.

| # | Finding (short) | What was done | Where |
|---|---|---|---|
| B1 | A seam escape is a real, password-less update, linger change or reboot | Fixed. Fail-closed seams (`test_seam`, no bus, no tool); one tool table, grep-enforced, stubs only in test mode; every bus/harness test inside a bwrap layer with private `/run/dbus`, `--unshare-net`, `no_new_privs`; `DBUS_SYSTEM_BUS_ADDRESS` = fake bus for every process; stubs first on PATH (GE0c's list + 9 more); `check-new-release` click-only; stub-log and `CallCount()` self-test after each run; precondition C18. Note: GE0c as built in round 0 already has the bwrap layer and an empty private system bus, so the remaining hole was GW1/GW2's runs and the absolute paths | §0, §1.1, §1.2, §4.11, §6.1, §9.1–§9.3, §10, §12 C18, §13 |
| M1 | apt backend has no phasing; "info 9 = phased" is likely false | Fixed. Own classifier `phasing.rs` with apt's rule (`apt_pkg.DepCache.phasing_applied`, one batched read-only call; `apt-cache show` fallback); phased/held ids never sent; "success but rows remain" defined; info 9 / holds get the neutral `keptBack` text; new timelines; R1 only confirms | §1.2, §3.2, §3.3, §4.2, §4.3, §4.6, §6.4, §9.1, §9.2, §13, R1 |
| M2 | Our hold does not stop Ubuntu's own offer | Fixed as a default plus a Robin decision: the card says Ubuntu may still offer it and offers "Ook de melding van Ubuntu uitstellen" (`check-new-release-ignore` + `check-dist-upgrades=false`, user-level, restored on ok); `Prompt=lts` named as the strong lever; automatic or not is C14/R10 | §0, §1.2, §3.5, §4.3, §4.12, §12 C14, §13, R10 |
| M3 | Two Settings processes run two full services | Fixed. Owner `flock` on `update/owner.lock`; mirrors forward writes as `update.request`; `rev` persisted; only the owner notifies; takeover on owner exit; two-services bus test; single-instance Settings kept as the alternative (C15) | §0, §1.1, §3.8, §4.1, §4.4, §4.5, §9.2, §12 C1/C15, §13 |
| M4 | R2 `install.sh --system-files` would deploy onto the live session | Fixed. `install-system.py` is standalone (`python3 scripts/install-system.py`, `--remove`, `--dry-run`, refuses other args); install/uninstall only print a hint; R2 and R5 rewritten; `install.sh` passthrough moved to GE1c's needs (C19) | §0, §1.1, §7, §10 GW5, §11 R2/R5, §12 C19 |
| M5 | No "did not start"/"stopped early" states; the Ptyxis window vanishes | Fixed. Constant `/bin/sh -c` wrapper records rc in `$XDG_RUNTIME_DIR/win11-gnome/upgrade-status.json` and holds the window on failure; card states `startFailed` and `ended` ("Logboek openen"); 120 s "no window" rule; `installParts` uses the same wrapper (hold always); `upgrade.json` also cleared post-upgrade | §0, §1.2, §3.5, §3.7, §4.3, §4.5, §6.4, §6.6, §9.3, §10 GW7, R5 |
| M6 | Normal updates can start during a release upgrade | Fixed. Every update write returns `update.guard.upgrade` while the upgrader is detected; hero phase `releaseRunning` (H25); upgrade-running bus test | §3.2, §3.7, §4.3, §4.5, §6.4, §9.2, §9.3 |
| M7 | The auto-open toggle overpromises | Fixed with honest text ("voor gewone updates" + the security exception); the autostart override stays a Robin-only request (R8), not a default | §1.2, §3.6, §4.12, §11 R8, §12 D2, §13 |
| M8 | `shell.restartAll` guard too narrow; vanish may cancel the download | Fixed. Guard also blocks on any own update job and any active Software job; receivers re-check and defer their own re-exec; the prewarmed Start ignores the event (no double Start); C2 now keeps the Settings process alive without a window until `Finished`; `vanish-cancels-download` timeline; R3 settles the vanish behaviour | §0, §3.7, §4.1, §4.7, §4.9, §5, §9.1, §10 GX1, §11 R3, §12 C2, §13 |
| m1 | `user_environment` is private | Fixed: GW7 gets the one-line `pub(crate)` change in `host/src/files/actions.rs` (GW7 rather than GW2: it is the only consumer, and no other round-6 step touches that file) | §1.1, §6.4, §10 GW7 |
| m2 | Wrong `include_str!` depth | Fixed: `../../i18n/host/update.tsv` | §6.6 |
| m3 | WebKit network comm name | Fixed: prefix `WebKit` (real `WebKitWebProces`/`WebKitNetworkPr`), one PPid level | §4.8 |
| m4 | Extension state is a double; GNOME's crash rule disables all extensions | Fixed: state read as a double; `disable-user-extensions` and `enabled-extensions` read; new `update.post.extsOff` with "Extensies weer aanzetten"; P04 | §1.2, §3.7, §6.6, §9.2, §9.3 |
| m5 | Autostart Delay ignored; child stopped at logout | Fixed: Delay dropped, ShellVersion retry kept, terminal via `process_scope::spawn` (`#[path]` into win11-launch, still GIO-only) | §1.2, §6.6, §10 GW7 |
| m6 | P01–P03 cannot use a fake `org.gnome.Shell.Extensions` | Fixed: real headless shell; P01 fixture `last-os-release.json`, P02 extension copy with `["49"]`, P03 soname fixture seam | §4.11, §9.3 |
| m7 | GW6 keyring, forced components, tmp GNUPGHOME | Fixed: keyring re-rooted in test mode, components from `ubuntu.sources`, GNUPGHOME under `tests/update/.run/` | §4.11, §6.2, §9.1, §10 GW6 |
| m8 | Compat download for a known hold | Fixed as the default (cheap verdict first; apt tree only when `tested` and the major matches); recorded as C16 so the coordinator can revert | §0, §3.5, §6.2, §12 C13/C16 |
| m9 | Power guard makes a D-Bus call in reader mode | Fixed: files and `/proc` only; dead owner + unfinished job blocks only while `job.json` is under 10 min old | §4.9 |
| m10 | Count flapping between snapshot and `apt list` | Fixed: once a snapshot exists it is the only count, `info.rs` never emits | §3.8, §4.4, §4.10 |
| m11 | Progress fan-out wakes every host | Fixed: local 4/s through `emit_local`, fan-out at most 1/s (for mirror windows) | §4.4, §10 GW2 |
| m12 | Attach helper with `auth_admin_keep` | Fixed: attach is `auth_admin` (no keep); the enum-only Prompt helper keeps `_keep` | §6.4, §7, §10 GW5 |
| m13 | Fidelity of four texts | Fixed: "Wachten op opnieuw opstarten / Pending restart"; error title "…sommige updates / There were problems installing some updates" (Windows' "we'll try again later" left out on purpose: nothing retries by itself); advNotify nl completed; codename stripped from `{version}`; GW8 still checks against 24H2 | §3.1, §3.2, §3.3, §3.6, §8, §14 |
| m14 | False "busy" from the comm scan | Fixed: `/proc/locks` against the dpkg/apt lock inodes; comm only picks the text | §4.11, §6.4, §9.2 |
| m15 | Simulate classification and exits | Fixed: infos 20/28/29/30 confirm-worthy; 28 and 30 in `removals-in-simulate`; exit 9 / error 65 → `update.err.locked`, 39 → `update.err.conflict`; enum lists extended; two timelines | §3.7, §4.6, §9.1 |
| m16 | No `download-size` from the apt backend | Fixed: `rowCaptionNoSize` is the normal case, totals from `DownloadSizeRemaining`; R1 checks | §3.1, §3.3, §4.6, R1 |
| m17 | "Foreign" includes our own Software installer | Fixed: `foreignOwn` text when an own Software job is active (H26) | §3.2, §4.7, §9.3 |
| m18 | R3 restart path after a round-1 extension change | Fixed: R3 says log out and in; §5 notes the limit of the InfoBar | §5, §11 R3 |
| m19 | GW3 done-when impossible with new keys | Fixed: `tests/i18n/BASELINE-ADDITIONS` for new keys next to `BASELINE-CHANGES`; side files for later steps | §10 |
| m20 | Ownership gaps and GW1→GW2 order | Fixed: side files for `surfaces.json`; M01–M04 owned by GW6; GW2 split into GW2a (parallel) and GW2b (after GW1) | §10 |
| m21 | Substring test filter | Fixed: `services::update::bus_tests::` | §9.2 |
| m22 | Harness memory too low | Fixed: plan 1.6–2.0 GB (shell + three shell WebKits + Settings) under GE0c's sampler. Not done: starting only Settings, because H10's restart flow and the InfoBar states need the panel | §9.3 |
| m23 | Private-tree apt vs the plan's "no apt" rule | Not decidable here: recorded as C17 (explicit plan.md exception before GW6, or recorded outputs only) and wired into GW6's done-when | §6.2, §9.4, §10 GW6, §12 C17 |
| m24 | Metered guess, portal/limited, renamed parts, WebKit InfoBar | Fixed: GUESS_YES counts as metered; online only for full/unknown; `installParts` names resolved on the new release; the Win11 InfoBar shows next to a pending reboot when WebKitGTK/JSC changed (C3) | §3.4, §4.3, §6.6, §12 C3 |

**Not done in this revision.** Nothing was built, run or launched; no product file was changed. Still unverified:
packagekitd's behaviour when a caller vanishes (R3), the real phased-package output and the `apt_pkg` call's cost
(R1), the text frontend's exit codes (R5), the 26.10 archive, and the 24H2 reference texts (GW8).

**For the coordinator / Robin:** C14 + R10 (Ubuntu's own prompt while on hold, Robin), C15 (owner lock vs
single-instance Settings), C16 (cheap-first compat, default on), C17 (plan.md exception for GW6's private apt), C18 (B1
rules as a precondition for GW1/GW2/GX1/GW8), C19 (`install.sh` passthrough goes to GE1c's file list), D2 + R8
(auto-open text and any full-off, Robin).
