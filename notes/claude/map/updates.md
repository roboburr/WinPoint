# Windows Update for Win11 voor GNOME: what exists, what carries over from H, what is missing

Read-only map, 2026-11-05. G = `win11-gnome` at commit 3b898e6 (branch `claude/gnome-english-update`, tracked files
unmodified). H = the Hyprland tree, `notes/windows-update-design.md` read completely (1366 lines). No PackageKit, apt,
pkcon or do-release-upgrade call was made, no build, nothing launched. System facts come from file reads, `dpkg-query`,
`readelf`, `gsettings get`, `loginctl show-user`, `/proc/<pid>/{maps,cgroup}` and `systemctl list-timers`.

## Summary

* **G has no Windows Update feature yet.** Updates are a read-only count: `system.updates` runs
  `apt list --upgradable` every 30 min (`host/src/services/system/info.rs:270-324`). The page offers "Controleren",
  which only re-reads that cache, and "Software-updater openen" (`power.rs:100-119`). There is nothing for applying
  updates, progress, restart-required, release upgrades or the LTS/normal choice.
* **G does have a reviewed PackageKit client** (`host/src/services/software/packagekit.rs`, 309 lines) for installing a
  local `.deb` file and removing a package. It already does `CreateTransaction`, `SetHints interactive=true`,
  simulate-then-run, `Package`/`Packages`/`ErrorCode`/`Finished`, `Percentage`/`Status`/`AllowCancel` and Cancel, on a
  private `MainContext` worker thread. The job, plan and lock files of `software.rs` can be reused as well. About a
  quarter of H's W2 "PackageKit core" already exists in Rust.
* **Most of H's design carries over unchanged.** That covers the PackageKit flow and enums, the hero, row, card and
  error states with their nl/en texts, the polkit facts, reboot-required, the release detection from Ubuntu's own data,
  the apt-tree compatibility check, `pkexec do-release-upgrade` in a terminal under GNU screen, temporary linger, the
  attach helper and the Prompt helper with its policy.
* **What changes for GNOME.** GNOME Shell's built-in polkit agent replaces hyprpolkitagent. The "sign-out" trigger is
  gnome-shell, not Hyprland. The compatibility check must compare the GNOME Shell major in the new release with the
  extension's `shell-version: ["50"]`. Ubuntu's GTK upgrader becomes a real option, because on GNOME it is Ubuntu's
  standard path. The process model is several Rust hosts, not one Python host. update-notifier duplicates our
  notices, and it really runs here. The post-upgrade check cannot live in the extension, because the extension is what
  stops loading.
* **What no longer applies:** the Python thread and GIL design, the S7 taskbar-flood gate, the Hyprland plugin
  (win11-csd) rebuild, the hyprland/hyprland-dev pairing, hyprland.lock, the hypridle/hyprlock rationale for the idle
  inhibitor, hyprpolkitagent, `_safety.py`, `_MACHINE_METHODS`, `i18n/host/update.tsv` with `L()`, the install.sh
  bash-array parse and nested-Hyprland screenshots.
* **This machine shows the problem today.** unattended-upgrades installed gnome-shell 50.1-0ubuntu1.5 at 06:54, so
  `/run/reboot-required.pkgs` = `gnome-shell`. The running gnome-shell (pid 6196, started 05:30) maps a deleted
  `/usr/bin/gnome-shell`, `libshell-18.so` and `libst-18.so`. win11-panel (pid 7256) and its WebKitWebProcess map a
  deleted `libsoup-3.0.so.0.7.4`.
* **Biggest G-specific risk: 26.10 will most likely ship GNOME 51.** The extension declares only `"50"` and uses
  shell internals (138 `Main.`/`global.` references), and `ubuntu-dock` is disabled here. So an upgrade without an
  updated extension leaves Robin with no taskbar and no dock. The card must say "on its way" (hold) until a tested
  extension exists, and a safety net must run outside the extension.
* **Effort: about 84 agent-hours for the full feature (range 70-100).** Phase A, in-app updates with progress and the
  restart banner, is about 46 h. Phase B, the 26.10 card, compatibility check and LTS choice, is about 32 h. Review and
  docs take about 6 h. Porting the extension to GNOME 51 is separate, 4-16 h, and is a precondition for an "ok"
  verdict.

## Findings

### F1. What G has today for updates and software

| Capability | In G? | Where (file:line) |
|---|---|---|
| Count available updates | yes, read-only | `host/src/services/system/info.rs:270-324` (`Updates`): `apt list --upgradable` (line 306), 30 min TTL (`Duration::from_secs(1800)` in `get`, 281-303), `security` = lines containing `-security`, emits `system.updates.changed` only when `available` changes (316-322). Counts phased packages too. |
| Refresh the package lists ("Controleren op updates") | **no** | `update.js:122` calls `system.updates` without `refresh`. Even with `refresh`, `check()` only re-runs `apt list`, never a refresh. |
| Apply updates | **no** | – |
| Progress bar for updates | **no** (exists for software jobs only) | `vendor/ui/js/core/software.js:118-162` `createSoftwareJobView` (ProgressBar, cancel, polling every 500 ms) |
| Restart required | **no** | nothing reads `/run/reboot-required`. grep finds no `reboot-required`, `RequireRestart` or `(deleted)` scan in `host/src`, `vendor/ui/js` or `extension`. |
| Release upgrade / "Ubuntu 26.10 is beschikbaar" | **no** | no meta-release, `do-release-upgrade` or `release-upgrades` code anywhere |
| LTS/normal choice | **no** | – |
| Open Ubuntu's updater | yes | `host/src/services/system/power.rs:100-119` tries `update-manager` (installed here), then `gnome-software --mode=updates` (not installed), then `snap-store --mode=updates`. The error text at line 116 is Dutch only. It is dangerous and dry-run guarded (`system.rs:340`, `342-357`). |
| Update page | yes, read-only hero | `vendor/ui/js/apps/settings/pages/update.js:93-137` (`updateHero`): disc glyph, "Je bent up-to-date" / "Er zijn {n} updates beschikbaar" / "Kan niet controleren", "Controleren" plus "Software-updater openen" behind a confirm dialog (131-134). nl/en `makeT` at 12-91. Below the hero: backup, reset, about (147-218). The CSS `.st-upd*` is at `vendor/ui/css/settings.css:759-763`. Registry: `registry.js:163-166`; aliases `updates`, `backup`, `reset`, `recovery` and `windows-update` at 178-179 and 191. It is identical to H's page apart from the export file name. |
| Other users of the count | yes | `settings/pages/home.js:87`, `system.js:75-96`, `about.js:61`, `taskmgr/pages/summary.js:110` (`system.openUpdater` as an action), `host/src/services/sysmon/health.rs:316-334` (health item that links to the `update` page) |
| List installed apps | yes, desktop entries | `installed.js:77` → `apps.list` (GIO desktop files). No package-level list. |
| Install software | yes, **from a local file only** | `software.prepare/execute` (`software.rs:362-489`) for AppImage, `.deb` (PackageKit `InstallFiles`), `.snap` (snapd REST), `.flatpak`/`.flatpakref`. UI: `vendor/ui/js/apps/installer/main.js:65-117`. There is no catalogue, search or store. |
| Remove software | yes | `software.uninstallInfo` → `resolve.rs:20-286` (PackageKit `RemovePackages` at `software.rs:724-735`, protected list `common.rs:307-332`, simulate then validate at `799-816`). UI: Settings › Installed apps context menu (`installed.js:96-97`), Start menu (`shell/startmenu.js:515`). |
| Update apps or snaps | **no** | – |
| PackageKit available? | checked without activation | `packagekit.rs:277-297` (`ListNames`/`ListActivatableNames` on the bus daemon) |

**The PackageKit client** (`host/src/services/software/packagekit.rs`) in detail:

* `run()` (43-258) runs one transaction on its own `glib::MainContext` (49-51, thread-default) on a worker thread that
  `software.rs:483` spawned. `SetHints` sends `interactive=true` and **`locale=C.UTF-8` hard-coded** (130-140).
* It handles the `Package`, `Packages` (batched), `Finished` and `ErrorCode` signals (82-112), and only the `Percentage`,
  `AllowCancel` and `Status` properties (26-42). It ignores `ItemProgress`, `RequireRestart`, `Speed`,
  `RemainingTime`, `DownloadSizeRemaining` and `LastPackage`.
* Progress is polled: `GetAll` every second (170-196) inside a loop with a **20 ms sleep** (251). A job lasting an hour
  therefore causes about 50 wake-ups per second.
* A **2-hour hard cap** turns the result into "unknown" (229-233). Cancel is sent only while `AllowCancel` is true
  (236-250).
* `uncertain()` (299-309) maps NoReply, Disconnected and NameHasNoOwner to `outcome_unknown`.
* The flags are SIMULATE (`4u64`) first, then `0u64` for the real run (`software.rs:712`, `734`, `752`, `760`).
  Updates must use ONLY_TRUSTED (`2`).
* Jobs are files owned by a pid plus its start time (`software.rs:91-109`). If the owner dies, the job becomes
  `interrupted`; there is no reattach (`512-521`).
* `Software::shutdown` **joins** the running worker threads (`777-787`; called from `services/mod.rs:254-257`). A host
  that is closed during a long transaction waits until it ends.
* There are fixture tests: `work/native-software-test/suite.rs:41-56` contains a minimal fake PackageKit
  (`CreateTransaction`, `Resolve`, `InstallFiles`, `RemovePackages`, `Cancel`, with no properties and no
  PropertiesChanged) and 10 PackageKit checks (`summary.json`). Its `run.sh` uses `dbus-run-session`, whose default
  config has service dirs. The auto-memory note warns that such a bus can take over the a11y bus unless
  `NO_AT_BRIDGE=1` is set.

**Plumbing a new `update.*` service needs:**

* routing in `host/src/services/mod.rs:135-215` (a `LazyService` per process, as for `software`);
* the event prefix allowlist in `host/src/events.rs:335-355`, because events are shared between the Rust hosts;
* `host/src/api-methods.json` entries with `dangerous`;
* dry-run through `ctx.dry_run || crate::backend::dry_run()` (`backend.rs:122-130`).

Host texts in G are inline nl/en pairs, `t(lang, nl, en)` (`software/common.rs:15-20`, `ctx.language()` in
`services/mod.rs:62`). There is no TSV catalogue.

**Existing precedents to reuse:**

* pkexec: `host/src/services/storage/cleanup.rs:661-735` (root-owned helper check, `/bin/sh -c` fallback, exit codes
  126/127). Its messages are Dutch only (689, 699, 729-732).
* Terminal: `host/src/files/actions.rs:611-672` (`terminal_command`, ptyxis `--new-window -d`; `open_terminal` with
  sanitised env and activation token).
* Placement: `process_scope.rs:17-20` puts Win11 processes in `app-win11shell.slice` with **MemoryMax 1000 MiB /
  MemoryHigh 768 MiB**. `launch::spawn` uses role `app`, which means `app.slice` with no cap (`launch.rs:59-61`,
  `process_scope.rs:266-271`).
* Restart: `shell.restart` (`services/shell.rs:216-224`) only restarts the calling host process, not panel, desktop,
  start and the apps together.

### F2. This machine (GNOME session, read-only, 2026-11-05)

| Fact | Value | Consequence for G |
|---|---|---|
| Session | `XDG_CURRENT_DESKTOP=GNOME`, wayland. Sessions 2 (seat0, tty2) and 3 (class `manager`); `Linger=no` | Same linger problem as in H §5.4: the "manager" session does not keep `user@1000` alive. |
| cgroups | win11-panel/desktop/start: `user@1000.service/app.slice/app-win11shell.slice/…`; gnome-shell: `session.slice/org.gnome.Shell@user.service`; update-notifier: `app.slice/app-gnome-update\x2dnotifier-6392.scope` | A terminal for the upgrade must be started with role `app` (no 1000 MiB cap). |
| PackageKit | `packagekit` 1.3.4-3ubuntu1.2, inactive (D-Bus activated). `gir1.2-packagekitglib-1.0` present, `packagekit-tools` (pkcon) absent. `/etc/apt/apt.conf.d/20packagekit` calls `StateHasChanged` after every dpkg run and every apt update | As in H: `UpdatesChanged` arrives without polling. |
| GNOME Software | **not installed**. The `snap-store` snap (App Center, rev 1468), `firmware-updater`, `snapd-desktop-integration` and `desktop-security-center` snaps are installed | No GNOME Software update notifications or offline updates to clash with. App Center's background behaviour is not checked. |
| Ubuntu updaters | update-manager 1:26.04.5 (aptdaemon backend), ubuntu-release-upgrader-core/-gtk 1:26.04.27, unattended-upgrades 2.12ubuntu9 (`Update-Package-Lists "1"`, `Unattended-Upgrade "1"`), update-notifier 3.207.2 **running** (pid 6392; autostart `NotShowIn=KDE;`, delay 60 s) | Three daemons share the dpkg lock: aptdaemon, PackageKit and unattended-upgrades. |
| update-notifier / update-manager settings | `regular-auto-launch-interval=7`, `check-dist-upgrades=true`, update-manager `launch-time` = 2026-11-05 00:20:58 | update-manager does open on its own in Robin's sessions. That duplicates our page. |
| GNOME's own updates | GNOME Shell and gnome-settings-daemon have no update notifier. Notifications come from update-notifier through GNOME's server, which the extension only **observes** (`extension/notificationsBridge.js:2`) and mirrors into the Win11 centre | Ubuntu's update and restart notices will appear in our notification centre next to ours. |
| Restart flag | `/run/reboot-required` + `.pkgs` = `gnome-shell` (06:54, from unattended-upgrades; `history.log` 06:54:04 gnome-shell 1.4 → 1.5) | The first real view of the new page will show the restart banner. |
| Stale mappings now | gnome-shell 6196: deleted `/usr/bin/gnome-shell`, `/usr/lib/gnome-shell/libshell-18.so`, `libst-18.so`, `Shell-18.typelib`, `*.gresource`, `libsoup-3.0.so.0.7.4`. win11-panel 7256 and WebKitWebProcess 7535: deleted `libsoup-3.0.so.0.7.4`, `libpcre2-8.so.0.14.0`, gvfs modules. Noise in maps: `memfd:*`, `/dev/shm/*`, dconf `user`, `gschemas.compiled`, `icon-theme.cache` | A needrestart-style scan works here and is useful. It must filter that noise. |
| polkit | `packagekit.system-update` auth_admin_keep, plus `org.freedesktop.packagekit.rules` → YES for an active local member of sudo/wheel. Robin is in `sudo`. `system-sources-refresh` yes; `upgrade-system` auth_admin; `repair-system` auth_admin; `cancel-foreign` auth_admin_keep; `release-upgrader.release-upgrade` auth_admin, `exec.path=/usr/bin/do-release-upgrade`; login1 `set-self-linger` yes, `inhibit-block-idle` yes, `inhibit-block-sleep` yes (active), `reboot` yes (active), `reboot-ignore-inhibit` auth_admin_keep | Updates need no password for Robin. A release upgrade always asks. Linger needs no password. |
| Release data | `Prompt=normal` (+ `release-upgrades.d/ubuntu-advantage-upgrades.cfg`). distro-info: 26.10 stonking released 2026-10-15, EOL 2027-07-15; 26.04 EOL 2031-05-29. User meta-release cache 28 Oct, root copy 25 Oct: newest entry `resolute 26.04.1 LTS`, **no stonking**. motd stamp empty (28 Oct); `update-notifier-motd.timer` next run Mon 9 Nov | "Released but not offered" is the real state today (H §5.1.4). No card yet. |
| Our binaries' ABI | NEEDED across the 12 `~/.local/bin/win11-*`: `libwebkit2gtk-4.1.so.0`, `libjavascriptcoregtk-4.1.so.0`, `libgtk-3.so.0`, `libgdk-3.so.0`, `libsoup-3.0.so.0`, `libgio/gobject/glib-2.0.so.0`, `libgdk_pixbuf-2.0.so.0`, `libcairo.so.2`, `libz.so.1`, `libgcc_s.so.1`, `libc.so.6`. Highest symbol GLIBC_2.39 (system has 2.43). `libarchive.so.13` is required (README:76) but is not in NEEDED, so it is loaded at runtime. Packages: libwebkit2gtk-4.1-0, libjavascriptcoregtk-4.1-0 (2.52.6), libgtk-3-0t64 (3.24.52), libsoup-3.0-0, libglib2.0-0t64, libgdk-pixbuf-2.0-0, libcairo2, zlib1g, libarchive13t64 | These package names are the "blocking" list of the compatibility check (they replace H's `preflight.PARTS`). |
| Extension | `extension/metadata.json:5-7` `"shell-version": ["50"]`, gnome-shell 50.1. `disable-extension-version-validation=false`. **`ubuntu-dock@ubuntu.com` is in `disabled-extensions`** (with dash-to-panel and eleven-start). Imports: `ui/main.js` ×6, `ui/messageTray.js`, `extensions/extension.js`; 138 `Main.`/`global.` uses in the 6 bridge files | On a new GNOME major the extension is not loaded. Then Robin has no Win11 taskbar **and** no Ubuntu dock. |
| apt sources | `ubuntu.sources` plus 10 third-party files (chatgpt, claude-desktop, google-chrome, signal, steam-beta, steam-stable, tailscale, winehq-resolute, xtradeb) | As in H: partial refresh failures, and do-release-upgrade disables them. |
| Ubuntu's upgrader on GNOME | `DistUpgradeController.py:2217-2258` inhibits the lock screen with `org.freedesktop.ScreenSaver.Inhibit` on the user's session bus. It takes a logind sleep inhibitor (243, `utils.py:357-391`) and reboots with `--check-inhibitors=no` (2170, 2211). The font quirk uses `org.gnome.desktop.interface` (`DistUpgradeQuirks.py:1195-1320`) | The lock screen during a GNOME release upgrade is handled by Ubuntu itself. |

### F3. H's design measured against G

**Carries over unchanged (reuse the text and the states):**

* §1.3 PackageKit 1.3.4 interface and enums; §4.3 flow: `RefreshCache(false)` → `GetUpdates(1<<1)` → simulate
  `UpdatePackages(SIMULATE|ONLY_TRUSTED)` → `UpdatePackages(ONLY_TRUSTED)`, `interactive=true`, locale only if
  `locale -a` lists it, conffiles keep the local version.
* §3.1-§3.4 and §3.6-§3.8: hero states (incl. offline, repairRequired, metered, partial refresh, only-phased, "Ook
  beschikbaar"), rows (security/kernel/component/other) with "Downloaden - 45%", feature card states (incl.
  rebootFirst, updatesFirst with the phased rule, waiting), "Meer opties", history from `/var/log/apt/history.log`,
  error mapping. All texts already have nl/en pairs.
* §4.4-§4.6: snapshot and job file in `<cache>` with boot id, reattach through `GetTransactionList` with NO_AUTO_START
  and `GetOldTransactions`, foreign transactions, cancel only while `AllowCancel`.
* §4.7.1 reboot detection (FileMonitor on `/run/reboot-required`, `RequireRestart`).
* §4.9 no offline updates; §4.10 the API only accepts ids from the current snapshot; §4.12 snaps read-only; §4.13
  fwupd optional.
* §5.1 release detection (Prompt precedence, motd stamp, both meta-release caches, `check-new-release -q` at most daily,
  distro-info CSV); §5.2 apt-tree option A with its own `APT_CONFIG`, so no system hooks run; §5.3 Prompt helper plus
  policy; §5.4 `pkexec /usr/bin/do-release-upgrade` in a terminal under screen, temporary linger (Q18), the attach
  helper, `main.log` watching, `LANGUAGE`; §5.6 restart and linger undo.
* §9 Robin-only actions R1-R6, and R5 (the upgrade test in a VM) is still essential.

**Changes because the session is GNOME:**

| H item | G version |
|---|---|
| §4.11 polkit agent = hyprpolkitagent; `update.err.noAgent` "Installeer hyprpolkitagent" | GNOME Shell's built-in agent is always there while the shell runs. Drop `noAgent`, or reword it to "the system window for passwords is not available". Prompts look like GNOME, not like Win11 UAC (Q17 stays a backlog item). |
| §4.7.2 sign-out trigger = replaced Hyprland | Trigger = the running gnome-shell maps a deleted `/usr/bin/gnome-shell`, `/usr/lib/gnome-shell/*` or `libmutter-*`, as it does live now. Ubuntu's gnome-shell package already writes `reboot-required`, so the reboot banner covers it. A "sign-out is enough" hint is optional (decision D6). |
| §4.7.3 "Win11 Shell opnieuw starten" after WebKitGTK/GTK/gtk-layer-shell/python3-gi | Watch `libwebkit2gtk-4.1`, `libjavascriptcoregtk-4.1`, `libgtk-3`, `libsoup-3.0` and `libglib-2.0` in **every** win11-* process and their WebKitWebProcess/NetworkProcess children. The action must restart all Win11 processes; today's `shell.restart` restarts one host only (gap X1). |
| §4.8 logind idle inhibitor because hypridle would start hyprlock mid-dpkg | GNOME's lock screen is gnome-shell itself (no separate binary that reloads libraries), packagekitd takes its own inhibitor, and Ubuntu's upgrader inhibits the screensaver itself. Keep only the power guard. Optionally use `org.gnome.SessionManager.Inhibit` (logout\|suspend) during our transaction, so GNOME's own logout dialog warns too. |
| §4.8 power guard | Still needed, and more so: our logout is `gnome-session-quit --logout --no-prompt` (`power.rs:44-49`), which skips GNOME's inhibitor dialog, and reboot is `systemctl reboot` (`power.rs:39`). |
| §4.14 / Q9 update-notifier "may" run | It **does** run, and update-manager auto-opened at 00:20 today. A decision is needed (D2): leave it, set `com.ubuntu.update-notifier regular-auto-launch-interval` and/or `com.ubuntu.update-manager check-dist-upgrades` per user behind an explicit toggle, or hide its autostart. Its notifications reach our centre through `notificationsBridge.js`. |
| §5.2 blocking list = `preflight.PARTS` (python3-gi, gir1.2-*, hyprland) + install.sh arrays | Blocking = the packages behind our NEEDED sonames plus libarchive13t64 (F2). Degraded = ffmpeg, bubblewrap, squashfs-tools, flatpak, packagekit, a terminal. **New, the decisive one:** `gnome-shell` candidate version → its major must be in the installed extension's `metadata.json` `shell-version`, otherwise the verdict is `hold`. |
| §5.2 `releases.json` per codename with Hyprland/plugin versions | Per codename with `gnomeShell` major and `status`: `{"resolute":{"gnomeShell":"50","status":"tested"},"stonking":{"gnomeShell":"51","status":"untested"}}`. The 51 is expected, not verified (no network). |
| §5.4 Q4 frontend: text in a terminal; GTK only after R7 (XWayland under Hyprland untested) | On GNOME the GTK frontend (`update-manager -c` → Upgrade, or `do-release-upgrade -f DistUpgradeViewGtk3`, which re-execs through pkexec and runs `xhost +si:localuser:root`) is **Ubuntu's own standard path**. It is no longer exotic. It still has no screen: a gnome-shell crash on Wayland ends every client, including that window. Decision D1; recommendation: keep text + screen + linger as the default and offer the GTK path as "Ubuntu's own window". |
| §5.5 the Python stdlib swap can crash the host | Not relevant (Rust). What remains: new win11-* processes or WebKit children started mid-upgrade load half-unpacked GTK/WebKit. During a running upgrade, don't spawn new Win11 hosts from our own code paths. |
| §5.6 first login after the upgrade: the session preflight and fallback exist; the host's `start()` checks `last-os-release` | In G **nothing of ours runs** if GNOME rejects the extension, because the extension starts panel, desktop and start. The post-upgrade check must be an XDG autostart entry (e.g. `win11-launch --post-upgrade-check`). It reads `org.gnome.Shell.Extensions.GetExtensionInfo` (a read), notifies through GNOME and offers to turn the Ubuntu dock back on (with consent). |
| §5.4 process placement (host scope) | The terminal goes through `launch::spawn` (role `app`, `app.slice`). It must **not** be in `app-win11shell.slice`: MemoryMax 1000 MiB (`process_scope.rs:19`) would apply to pkexec's root child, which inherits the cgroup, and could OOM-kill dpkg. |
| §4.1 one shell host with a PK thread | Several Rust hosts: panel, desktop, start, `win11-app --app settings`, taskmgr. Recommendation (D5): transactions and reattach in the process that runs Settings; a light read-only watcher (reboot-required FileMonitor plus `UpdatesChanged`, no activation) only if Robin wants "restart required" notifications while Settings is closed. Job and snapshot files as the shared state, as `software` already does. Don't join threads on shutdown: detach and rely on reattach. |
| §8 W5 root step in install.sh | G's installer is user-local (`scripts/install.py`, no root step). The 2 policies + 2 helpers need a new pkexec step or the planned final package (goal 4, a `.deb`). Fallback without them: `/bin/sh -c` pkexec as in `cleanup.rs`, with the ugly "run /bin/sh" prompt (decision D4). |
| §6 English: `i18n/host/update.tsv`, `L()`, BASELINE-CHANGES | G uses inline `t(lang, nl, en)` in Rust and `makeT` in the UI. The nl/en text tables of H §3 can be pasted as pairs. Robin's i18n checks in G are whatever the English work decides. |

**No longer applies (drop):**

* Python `update-pk` thread with its own `GLib.MainContext`, the GIL notes, the worker role `update` (§4.1) and the S7
  bench gate (§4.1, W8). In G the panel does not even call `system.updates` today.
* The Hyprland plugin (win11-csd), `plugins()` key and `plugin_report`, `hyprland`+`hyprland-dev` always together
  (§4.3), the hyprland.lock pid lookup (§4.7.2), the Hyprland minor-version `warn` (§5.2) and the config-keyword risk.
* hypridle/hyprlock (§4.8), hyprpolkitagent and the `win11-polkit` window rule (§4.11).
* `tests/_safety.py`, `_MACHINE_METHODS`, `host/util.py`, `host/preflight.py`, the install.sh bash-array parse and its
  test, `win11shell-session fix-packages`, `apps.default_terminal()`, `priority.start_scope`.
* Nested-Hyprland screenshots (§7). G needs its own screenshot path (mock page in a private WebKit or a nested GNOME
  Shell); not decided.

## Work needed

| # | Item | Files (new or touched) | Size (agent-h) |
|---|---|---|---|
| G-W1 | Fake PackageKit for G: extend Codex's fixture (`work/native-software-test/suite.rs:41-56`) into a reusable Rust fake with properties + PropertiesChanged, `GetUpdates`/`Packages`, `RefreshCache`, `UpdatePackages` simulate/real, `ItemProgress`, `RequireRestart`, `GetUpdateDetail(s)`, `GetTransactionList`, `GetOldTransactions`, `RepairSystem`, broadcast signals, an activation/call counter and about 15 timelines (H §7). Minimal `dbus-daemon` config without service dirs, `NO_AT_BRIDGE=1`. A read-only probe (properties only by default) | test crate under `work/` or `host/tests/` | 7 |
| G-W2 | Update service core in Rust: a shared PK client (refactor `packagekit.rs`: event callback instead of a 20 ms poll, all progress properties and signals, NO_AUTO_START reads, a locale hint from `ctx.language()`, ONLY_TRUSTED, no 2 h cap while progress moves); `update.status/check/details/install/cancel/repair`; row grouping (component rows: WebKitGTK, GTK 3, libsoup, GNOME Shell/mutter, kernel); snapshot and job files with boot id; reattach and foreign transactions; reboot-required FileMonitor; deleted-mapping scan (gnome-shell, win11-*, WebKit children; noise filter); throttled `update.progress`; `system.updates` delegation plus `rebootRequired` and phased handling (`info.rs`); power guard (`power.rs`); `events.rs` prefix; `api-methods.json`; dry-run never touches the real bus; nl/en pairs | `host/src/services/update.rs` + `update/*.rs`, `services/mod.rs`, `events.rs`, `api-methods.json`, `system/info.rs`, `system/power.rs`, `software/packagekit.rs` | 16 |
| G-W3 | The page: hero states, rows with ProgressBar and details, restart InfoBars, error expander, "Meer opties", subpage registry, mock scenarios, CSS; screenshots of about 20 states × nl/en for the Windows look | `vendor/ui/js/apps/settings/pages/update.js`, `vendor/ui/css/settings.css`, `vendor/ui/js/mock/system.js` (or a new `mock/update.js`), `registry.js` | 13 |
| G-W4 | History and advanced options: apt history parser, `update.history`, `update.notifyRestart` and `update.metered` in `settings-defaults.json`, read-only unattended-upgrades and snap rows, "Software-updater van Ubuntu", plus the update-notifier de-duplication setting chosen in D2 (reversible, per user) | `update.rs`, `update.js`, `host/src/settings-defaults.json` | 7 |
| G-X1 | "Win11 opnieuw starten" for all processes after a WebKitGTK/GTK/libsoup update (panel, desktop, start and open apps). Today `shell.restart` restarts only the caller | `services/shell.rs` or the extension's `panelBridge.js`/`desktopBridge.js` | 3 |
| G-W5 | Release detection and the Prompt choice: Rust parsers (Prompt + `.d`, motd stamp, both meta-release caches, distro-info CSV), daily `check-new-release -q`, `setPrompt` via helper + policy, attach helper + policy, a root install step (none exists in `scripts/install.py`) or `.deb` packaging, Dutch-only pkexec texts made bilingual | `update.rs`, `host/data/*` (4 new files), `scripts/install.py`/`uninstall.py` or packaging | 11 |
| G-W6 | Compatibility check and card: apt tree with its own `APT_CONFIG`; package list = soname packages + libarchive13t64 + degraded list; **gnome-shell candidate major vs `metadata.json shell-version`**; `releases.json` per codename and GNOME major; all card states. Cheaper variant: static `releases.json` + GNOME major only, no apt tree (about 6 h), at the cost of not seeing a renamed library | `update.rs`, `host/data/releases.json`, `update.js` | 10 |
| G-W7 | Starting the upgrade and after it: preconditions (updatesFirst with the phased rule, rebootFirst, no transaction), terminal via `launch::spawn` (app.slice), `LANGUAGE`, temporary linger with a stamp and undo, `update.release.attach`, running/waiting detection (`pgrep`, `/var/log/dist-upgrade/main.log`); **post-upgrade check outside the extension** (XDG autostart; extension state via `GetExtensionInfo`; GNOME notification; offer to re-enable ubuntu-dock) | `update.rs`, `update.js`, `host/src/bin/launch.rs` or a new small bin, `scripts/install.py` | 11 |
| G-W8 | Review pass, docs (README, an "as built" note), final screenshots | various | 6 |
| G-W9 | Optional: Start power badge + "Bijwerken en opnieuw opstarten", restart-notification actions, fwupd read-only row | `startmenu.js`, `update.rs`, `update.js` | 6 |
| G-X2 | Not part of the feature, but needed before the 26.10 card can say "ok": port and test the extension on the GNOME Shell major of 26.10 and add it to `shell-version` | `extension/*` | 4-16 (unknown) |

**Totals:** Phase A (W1 + W2 + W3 + W4 + X1) ≈ 46 h; Phase B (W5 + W6 + W7) ≈ 32 h; W8 ≈ 6 h. W1-W8 + X1 ≈ **84 h**.
W9 (6 h) and X2 (4-16 h) come on top.

**Decisions Robin has to make** (recommended default in bold):

* D1. Release-upgrade frontend on GNOME: **text in a terminal under screen, plus temporary linger** / Ubuntu's GTK
  upgrader window.
* D2. update-notifier: leave it alone / **per-user toggle "Software-updater automatisch openen" off, release prompts
  only on our page** / hide its autostart. Hiding it would also lose crash reports and livepatch notices.
* D3. Untested GNOME major in the new release: **"Ubuntu 26.10 is onderweg" (hold) with "Toch upgraden…" behind a
  warning** / block outright.
* D4. System files for the LTS/normal choice and attach: **install 2 policies + 2 helpers in a root step of the
  final package** / the `/bin/sh` pkexec fallback without system files.
* D5. Restart notifications while Settings is closed: **no background watcher in v1 (Ubuntu's update-notifier already
  says it)** / a light watcher in the panel process.
* D6. gnome-shell updated: **show "Opnieuw opstarten vereist" as Ubuntu does** / also offer "Afmelden is genoeg".

## Risks

* **Extension rejected after a release upgrade.** GNOME 51 (expected in 26.10) against `shell-version ["50"]`, with
  `ubuntu-dock` disabled: Robin is left with no taskbar and no dock, only Activities. Mitigation: a hold verdict on the
  GNOME major, X2 before offering the card, and the autostart safety net (G-W7).
* **WebKitGTK/libsoup security updates under a running UI.** They are frequent on Ubuntu; libsoup is a deleted mapping
  in the panel right now. New WebKit children of an old UI process can crash, which shows as blank Start or panel
  pages. Mitigation: the mapping scan plus a whole-UI restart (X1).
* **A gnome-shell crash during a release upgrade ends the Wayland session.** That kills the terminal and the GTK
  frontend. Screen keeps dpkg alive only while `user@1000` stays up (linger). Not verified: whether GNOME's logout
  path also stops transient units in `app.slice`. That is R5 in a VM.
* **Memory cap.** Starting the terminal or upgrader through the shell role puts root's do-release-upgrade under
  MemoryMax 1000 MiB.
* **Duplicate UIs.** update-manager auto-opens every 7 days, update-notifier shows restart and release prompts, and
  App Center may install too. With aptdaemon, PackageKit and unattended-upgrades on one dpkg lock, our jobs can wait
  for minutes (status 30).
* **The power path skips GNOME's inhibitor dialog** (`--no-prompt`), so a logout during dpkg is possible without the
  guard.
* **`Software::shutdown` joins workers**, so a Settings process closed during an update stays alive, invisible, until
  the transaction ends. The 2-hour cap in `packagekit.rs` labels long jobs "unknown".
* **The PackageKit apt backend is not the Software Updater** (conffiles keep the local version, phasing, no deb2snap),
  as H §10 says. The phasing representation is still unverified (R1).
* **Language gaps.** `power.rs` and `cleanup.rs` messages that the update work touches are Dutch only. polkit
  prompts follow the agent's locale (en_US here).
* **Test infrastructure.** `dbus-run-session` with the default config can activate services and take the a11y bus
  (auto-memory). The update fake needs a minimal config.
* **Release cadence.** Interim releases (Prompt=normal) need a tested extension every 6 months. With Prompt=lts this
  happens every 2 years. That argues for showing the LTS choice prominently.

## Not checked

* No PackageKit transaction, so not: the real `GetUpdates` output, how phased packages are reported, whether
  `UpdatePackages` may remove packages, or packagekitd's inhibitor mode.
* The 26.10 archive (no network): the gnome-shell version, whether `libwebkit2gtk-4.1-0` and the other soname packages
  exist under the same names, the upgrader tarball's behaviour on GNOME.
* Whether update-notifier actually shows a "restart required" notification in this GNOME session, and what App Center
  (`snap-store` snap) does in the background.
* GNOME logout and session-crash behaviour for `app.slice` transient units and screen. How the extension supervises
  or relaunches panel and desktop (only grepped `panelBridge.js`/`desktopBridge.js`).
* The NucBox: only H's notes (NucBox G3, Ubuntu 26.04, language-pack-gnome-nl). Its update and polkit setup were not
  seen.
* G's `VALIDATION*.md` files were not read in full (README/RELEASE_PLAN have no update content beyond software
  install). `installed.js`, `installer/main.js` and `startmenu.js` were only read at the call sites listed.
* Not read: `host/src/services/system/{audio,bluetooth,network,energy,hardware,agent}.rs` beyond grep. They contain no
  update code.
