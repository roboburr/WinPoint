# Map: GNOME extension, scripts, packaging and Codex's test method

Scope: `G/extension/**`, `G/scripts/**`, README/LICENSE/RELEASE_PLAN/PACKAGING_VALIDATION, desktop entries,
and how Codex tested (`work/*`, `VALIDATION*.md`). G = `/home/robin/Documents/Codex/2026-11-01/chat/outputs/win11-gnome`,
W = `/home/robin/Documents/Codex/2026-11-01/chat/work`. Read-only pass on 2026-11-05. The only file written is this report.
The only command run against project files was `python3 -B scripts/verify-ui.py`, which reads and hashes files (result: "OK: 176 interfacebestanden").

## Summary

- **The extension is almost all Dutch, with no language switch.** It has 9 distinct Dutch on-screen or screen-reader strings, a Dutch date format,
  2 Dutch error messages, and a Dutch `metadata.json` name and description. The only bilingual piece is the test notification
  (`notificationsBridge.js:214-220`), which follows `args.language`. The host injects that value (`host/src/services/notifications.rs:420`).
  About 30 English technical error messages can show up inside Dutch toasts (`vendor/ui/js/shell/shell.js:461`).
- **The extension can learn the language from `settings.json`.** It already reads `$XDG_CONFIG_HOME/win11-gnome/settings.json`, but only once
  in `enable()` (`extension.js:20-31,49`). It has no file monitor. Adding a directory monitor plus `settings.language` (missing or unknown means `nl`)
  is enough. Do **not** use the GNOME locale: Robin's live gnome-shell runs with `LANG=en_US.UTF-8` while the Win11 default is `nl`.
- **All installer, uninstaller and build output is Dutch only.** That is about 37 user-facing messages across 7 files, and the 10 desktop
  entries have only Dutch `Name=` and `Comment=` keys. There is no `--lang` option and no English path.
- **A language test exists but was never run.** `W/desktop-language-test/checks.py` (4 checks: Deze pc/This PC, Prullenbak/Recycle Bin, launcher
  `Name[nl]`/`Name[en]`, de-duplication of an old shortcut) is a plug-in for the private desktop harness. The only trace of it being used is a
  `py_compile`. No command in any Codex log set `WIN11_LANGUAGE_HELPER`, no results file survives, and it tests nothing in the extension.
- **The flow is build, package, then install user-locally.** `build.sh` runs `cargo build --release --locked`. `package.sh` makes a tar.gz with source and 12 binaries.
  `install.sh` installs everything under `~/.local`, with no root. After installing or changing the extension you must log out and back in;
  binaries and UI changes only need the project's own processes restarted.
- **The NucBox route is the tarball.** Unpack it and run `./scripts/install.sh`, then log out and in, then enable the extension. The installer
  checks only its own files, not the runtime libraries (WebKitGTK 4.1, GTK3, libarchive.so.13, python3-gi). Uninstalling needs the unpacked
  tarball, because no uninstaller is installed. Running `install.sh` from a G checkout today fails: it prefers the stale 6-binary `G/bin/`.
- **Codex tested GUI work in a private headless GNOME Shell.** Each run used `dbus-run-session`, `gnome-shell --headless --virtual-monitor=WxH --no-x11`,
  private HOME and XDG directories, keyfile GSettings, an extension copy patched with `unsafe_mode` for `org.gnome.Shell.Eval`, Clutter virtual
  input, WebKit control through `WIN11_DEV_COMMAND`/`WIN11_DEV_REPORT`, and screenshots through `org.gnome.Shell.Screenshot`. Codex used no Xvfb, `mutter --nested` or devkit.
  31 harnesses seeded `"language":"en"`, so English screens were exercised often, even though no test checked English text.
- **Codex changed the live session at least 12 times.** These were live installs, SIGTERM and relaunch of win11-panel, win11-desktop and win11-start,
  `gsettings` changes to the icon theme, disabling Ubuntu Dock, Dash to Panel and Eleven Start, a full dconf reset, `systemctl --user daemon-reload`,
  edits to Fluent GTK theme files, and opening Settings on the live display. Separately, the live `~/.local/bin/win11-panel` was replaced on
  2026-11-04 23:32 by an unknown source. Its hash `17437f50…` is in no Codex record.
- **Upgrading Ubuntu will very likely disable the extension.** `metadata.json` says `"shell-version": ["50"]`, and the code uses private Shell
  internals (`Main.layoutManager._trackedActors`, `_queueUpdateRegions`, `_startingUp`). Ubuntu 26.10 probably ships GNOME 51 (not checked).
  The "Ubuntu 26.10 is beschikbaar" compatibility check must test for this.

## Findings

### (a) User-visible strings, counts, and how the extension can know the language

#### Extension, on screen or read by a screen reader

The fallback St taskbar is only visible while the native `win11-panel` is absent: at startup, after a crash, or while the panel restarts
(`extension.js:150-163`). Its texts still show on real screens.

| File:line | Text | Lang | Where it shows |
|---|---|---|---|
| extension.js:62 | `'Start'` | neutral | accessible name |
| extension.js:66 | `'Zoeken'` | nl | accessible name of the search button |
| extension.js:66, 275 | `'⌕   Zoeken'` (and `'⌕'` compact, :278) | nl | fallback bar label, reset on every `_layout()` |
| extension.js:73 | `'Taakweergave'` | nl | accessible name |
| extension.js:108 | `'Netwerk, geluid en snelle instellingen'` | nl | accessible name of the tray button |
| extension.js:121 | `'Win11-taakbalk'` | nl | Ctrl+Alt+Tab group name (visible popup) |
| extension.js:234 | `Main.notify('Win11 voor GNOME', 'Deze app is nog niet geïnstalleerd.')` | nl | GNOME notification, also mirrored into the Win11 notification centre |
| shortcutBridge.js:161 | `Main.notify('Win11 voor GNOME', 'Taakbeheer is nog niet geïnstalleerd.')` | nl | same |
| extension.js:387 | clock `'%H:%M'` / `'%H:%M:%S'` + `'\n%d-%m-%Y'` | nl format | fallback bar clock; en-GB would be `%d/%m/%Y` (H design D1) |
| notificationsBridge.js:218-220 | `'Zo ziet een melding eruit'` / `'This is what a notification looks like'`, body, `'Prima'` / `'Got it'` | **nl+en** | test notification; `nl = args.language !== 'en'` (:214) |
| notificationsBridge.js:134 | `'Notification'` | en | fallback appName for nameless sources (shown in the native notification UI) |
| notificationsBridge.js:215 | `'Win11 Shell'` | neutral | MessageTray source title |
| notificationsBridge.js:235, 240 | `'Deze melding bestaat niet meer.'`, `'Deze actie bestaat niet meer.'` | nl | errors, can reach a toast |
| panelBridge.js:384-390 | `'Win11 Verkenner'` (nl), `'Win11 Settings'`, `'Win11 Task Manager'`, `'Everything'`, `'Win11 Disks'`, `'Win11 Default Apps'`, `'Win11 App Installer'` (en) | mixed | display name of a throw-away `/usr/bin/true` AppInfo, used only when the `.desktop` is missing (:395-396). "Everything" conflicts with H decision D5 ("keep Alles") |
| metadata.json:3-4 | name `"Win11 voor GNOME — Bureaublad en taakbalk"`, description in Dutch | nl | GNOME Extensions app / `gnome-extensions info`; cannot be localised per language (no per-locale keys; not verified in the Extensions app) |
| schemas/…gschema.xml:6,10,14,18,19 | 4 summaries + 1 description | en | only in dconf-editor / `gsettings describe` |

**Technical error messages, all English.** These come back through `Call()` as `{ok:false,error:{code,message}}` (`panelBridge.js:804-812`).
The host keeps the message (`host/src/gnome.rs:16-24,197-202`), and `ctx.toastError` shows `error.message` as the toast body (`vendor/ui/js/shell/shell.js:459-464`).

- panelBridge.js: 29 `failure()` calls with 25 distinct messages (for example `'The requested application is not installed.'` :402,
  `'Apps can only open in the unlocked user session.'` :382, `'This window cannot be maximized.'` ×3), plus `'Client protocol geometry is not ready'` (:262).
- notificationsBridge.js:189, 245; desktopBridge.js:125; frostedBridge.js:64, 69.
- `console.warn` lines (shortcutBridge.js:58, desktopBridge.js:56, panelBridge.js:105, 341, 361) go to the journal only.

**Counts for the extension.**

- Dutch: 9 distinct UI strings, 1 date format, 2 error messages, 1 fallback app name and 2 metadata fields (15 in total).
- Already bilingual: 3 pairs.
- English: 6 fallback app names, about 32 technical error messages and 5 schema texts.
- Neutral: 3 (Start, Win11 Shell, the ⌕ glyph).
- `stylesheet.css` has no text; it sets `font-family: "Segoe UI"`.

#### Installer, uninstaller and build output (Dutch only; 0 English)

| File | Lines | Count |
|---|---|---|
| install.py | SystemExit :22, 25, 50, 53, 55, 62, 80; print :133-138 (6 lines, e.g. `"De taakbalk is geïnstalleerd, maar nog niet aangezet."`) | 13 |
| uninstall.py | :18, 20, 91 | 3 |
| icon_bundle.py | ValueError :22, 29, 31, 32, 37, 39, 40, 42, 75, 94 | 10 |
| file_portal.py | SystemExit :26, 30, 75, 179, 196, 199 (+4 internal tags :166-175 in English) | 6 |
| build.sh | :6, 10 | 2 |
| package.py | :22 | 1 |
| verify-ui.py | :19, 25 | 2 |
| **Total** | | **37** |

**Desktop entries.** These are generated in `install.py:111-130`, not shipped as files:

- 10 entries with Dutch-only `Name=`: Win11 Bureaublad, Win11 Taakbalk, Win11 Start, Win11 Instellingen, Win11 Standaardapps,
  Win11 App-installatie, Alles, Win11 Schijfbeheer, Win11 Taakbeheer and Win11 Verkenner.
- One developer-style Dutch `Comment=` shared by all 10 (:126).
- No `Name[nl]` or `Name[en]`, no `GenericName`, no `Keywords`.
- No metainfo or appdata file exists anywhere in G.
- `desktop-file-validate` on the installed live entries gives only 2 hints (DefaultApps has two main categories; Installer `PackageManager`).

Effect of locale keys: the host catalog reads desktop entries in the Win11 language, not the GNOME language. It translates when the Win11
language differs from the system language (`host/src/apps/catalog.rs:95-131,393-470`). Adding `Name[nl]` and `Name[en]` therefore fits
the catalog. GNOME's own surfaces (app grid, the extension fallback bar through `app.get_name()`) follow `LANG=en_US`. If the plain `Name=`
becomes English, Robin will see English names there even in Dutch mode. Decide whether that is acceptable (H design: plain keys English, `[nl]` Dutch).

#### How the extension can know the language

1. **Recommended: read `settings.language` from the file it already reads.** `readSettings()` (`extension.js:20-31`) loads
   `WIN11_REFERENCE_SETTINGS` (a test override), then `$XDG_CONFIG_HOME/win11-gnome/settings.json`, then the old
   `~/.config/win11shell/settings.json`. The default is `"language": "nl"` (`host/src/settings-defaults.json:3`).
   The store writes atomically with a rename (`host/src/settings.rs:351,375`). So watch the directory with `Gio.File.monitor_directory`
   (`WATCH_MOVES`), debounce, then re-label. `this._settings` is read only once today (:49); only the Start alignment is re-read (:150).
2. **Optional: push from the host over the existing bridge.** For example, a `shell.language` Call from win11-panel on start and on change.
   That pattern already exists for notifications (`notifications.rs:420`). It cannot be the only source, because the fallback bar matters
   exactly when the panel is gone.
3. **Not usable: `GLib.get_language_names()` or `LANG`.** Robin's gnome-shell environment says `LANG=en_US.UTF-8` (read from `/proc/<gnome-shell>/environ`).

For the technical errors, the cleanest fix is UI-side mapping by `error.code`. Translating 30 extension messages is the alternative.

### (b) The existing language test

- File: `W/desktop-language-test/checks.py` (60 lines, 2026-11-01 22:23). It defines `run_language_checks(env, work, desktop_window,
  launch_settings, capture, record)` for the private desktop harness (`W/desktop-bridge-test-0.7.0/run-combined.py`).
- What it does:
  - It opens the original Settings language dropdown (`.st-view[data-page=language] .fl-dropdown`, options `data-index` 0 and 1) (:22-26).
  - It writes two `.desktop` fixtures onto the private Desktop (`Deze PC.desktop`, and `Localized fixture.desktop` with
    `Name[nl]=Nederlandse naam` and `Name[en]=English name`) and marks them trusted with `gio set … metadata::trusted` (:27-33).
  - It asserts 4 cases:
    1. Dutch: `::thispc=='Deze pc'`, `::trash=='Prullenbak'`, launcher `'Nederlandse naam'` (:38-41).
    2. English: `'This PC'`, `'Recycle Bin'`, `'English name'` (:42-45).
    3. The legacy "Deze PC" shortcut is hidden only while the built-in item shows, and its file stays byte-identical (:46-49).
    4. The legacy shortcut follows the language (:50-52).
  - It writes `work/language-results.json` (the harness ROOT was `/tmp/win11-desktop-test`) (:59).
- Was it run? There is no evidence it was.
  - The only `.pyc` comes from `python3 -m py_compile` (Codex log `~/.codex/sessions/2026/11/01/rollout-2026-11-01T22-26-33-…jsonl`).
  - The hook `if os.environ.get('WIN11_LANGUAGE_HELPER'): …` was added to a generated copy of the harness (log `…T21-35-52-…jsonl`).
    It is **not** in the saved `run-combined.py`, and no logged command sets `WIN11_LANGUAGE_HELPER=` (searched all Nov 1-5 logs).
    The wallpaper helper, by contrast, ran 9 times.
  - `/tmp/win11-desktop-test` no longer exists. `validation/desktop-0.7.0.json` lists 54 checks and none is about language.
    `VALIDATION_DESKTOP.md` has no language section.
- Not covered by it: the extension, Start, taskbar, the installer, desktop entries in GNOME, the Dutch-unchanged guarantee.
- Related: `VALIDATION.md` says "Standaardapps … EN/NL" was checked in `work/native-defaultapps-test`. 31 harnesses seed `language: 'en'`
  (for example `settings-native-test/run.py:68`, `desktop-bridge-test-0.7.0/run-combined.py:12`). Only `start-full-animation-test` seeds `nl`.
  So English rendering ran often, but nothing asserted English text, and Dutch was less covered in private runs.

### (c) The build, package, install and uninstall flow

**Build.** Run `./scripts/build.sh` (`build.sh:1-13`).

- It checks `cargo` and `pkg-config`, then `gtk+-3.0 >= 3.24` and `webkit2gtk-4.1 >= 2.40`, then runs
  `cargo build --release --locked -j ${WIN11_BUILD_JOBS:-4}`.
- Profiles (`Cargo.toml`): release is `strip=true, lto="thin"`; dev is `debug=0`.
- The workspace has one crate, `host/` (version 0.7.0, edition 2024, wry 0.55, webkit2gtk 2.0, gtk 0.18).
- The toolchain is present: cargo 1.97.1 and rustc 1.97.1.
- `G/target/` holds only `debug/{build,deps,…}`, with **no binaries and no `target/release`**. Codex built in staged copies or under `/tmp`
  (for example `BIN=/tmp/win11-gnome-build-0.6.2/debug` in `run-combined.py:5`).
- The live binaries are about 28 MB each, against 13.8 MB in `G/bin`. They are probably dev-profile builds (not verified).

**Verify UI.** `python3 scripts/verify-ui.py` compares `vendor/ui` with `vendor/ui-sha256.json` (176 files). It also reports against the
original baseline (161 files: 24 changed, 15 added).

**Package.** Run `./scripts/package.sh`, which calls `package.py`:

- It runs verify-ui and `icon_bundle.validate`.
- It requires all 12 `BINS` (`install.py:17`) in `WIN11_BUILD_DIR` (default `target/release`).
- It stages `scripts/ extension/ host/ vendor/`, the Cargo files, LICENSE, README, RELEASE_PLAN, the reports listed in :36 (the list
  misses `VALIDATION_RESOURCES.md` and names a non-existent `VALIDATION_PROCESS_MEMORY.md`), `validation/`, the previews and `bin/`.
- Output: `dist/win11-gnome-<ver>-linux-<arch>.tar.gz` plus `.sha256`. Archive paths start with `win11-gnome/`.
- `dist/` holds only 0.2.0 to 0.6.1 (largest 14.7 MB). **There is no 0.7.0 archive**, even though `PACKAGING_VALIDATION.md:7-11`
  says one was checked.
- The next archive will be much larger, because 0.7.0 added `vendor/icons` (81 MB, 16,839 files plus 18,071 symlinks) and `vendor/icon-source` (8.8 MB).

**Install.** Run `./scripts/install.sh`, which calls `install.py <root>`:

- Build directory: `WIN11_BUILD_DIR`, else `root/bin` if `bin/win11-start` exists, else `target/release` (:42).
  **G has a stale `G/bin/` with only 6 binaries from Nov 1** (excluded from git through `.git/info/exclude`). A plain
  `./scripts/install.sh` in G therefore stops at :48-50 ("Ontbreekt: …/bin/win11-index-worker").
- It refuses to overwrite unmarked files (:54-62). It stages the payload and swaps it atomically (:66-92). Binaries are replaced through
  `.name.new` plus rename, so running processes keep their inode (:94-100).
- Files land in these places (defaults are `PREFIX=~/.local`, `XDG_DATA_HOME=~/.local/share`, `XDG_CONFIG_HOME=~/.config`):
  - `$PREFIX/bin/`: 12 binaries.
  - `$PREFIX/share/win11-gnome/`: `ui/`, `host/` (the Python reference, still required at :51-53), the 3 UI manifests, `icons/Win11{,-dark}`,
    `icon-source/`, `icon-bundle.json`, `ICON_PROVENANCE.md`, marker files `.win11-gnome-owned`, `.win11-icon-aliases.json`,
    `.win11-icon-install-state.json` and `file-portal.json` (106 MB live).
  - `$XDG_DATA_HOME/gnome-shell/extensions/win11-gnome@win11shell.local/`: 9 files, plus `schemas/` with the XML and `gschemas.compiled`
    built by `glib-compile-schemas --strict` (:103-109).
  - `$XDG_DATA_HOME/applications/io.github.win11shell.{Desktop,Panel,Start,Settings,DefaultApps,Installer,Search,Disks,TaskManager,Explorer}.desktop`,
    then `update-desktop-database` (:110-132).
  - `$XDG_DATA_HOME/icons/Win11Shell{,-dark}`: symlinks into the payload (`icon_bundle.register_aliases`).
  - The FileChooser portal (`file_portal.register`): `$XDG_DATA_HOME/xdg-desktop-portal/portals/win11shell.portal`,
    `$XDG_DATA_HOME/dbus-1/services/org.freedesktop.impl.portal.desktop.win11shell.service`,
    `$XDG_CONFIG_HOME/systemd/user/app-win11-file-portal.service`, and only the `FileChooser=` key in
    `$XDG_CONFIG_HOME/xdg-desktop-portal/gnome-portals.conf` (the original bytes are kept in `file-portal.json`).
- It never writes GSettings and never enables the extension. It prints 6 Dutch lines (as in `W/native-live-update/install.log`).
- Runtime data is created later by the apps: `~/.config/win11-gnome/settings.json`, `~/.cache/win11-gnome`, `$XDG_RUNTIME_DIR/win11-gnome`.

**Activate.**

- On a fresh machine: log out and in so GNOME discovers the extension directory, then run
  `gnome-extensions enable win11-gnome@win11shell.local` (README "Installeren en openen"; `install.py:136-137`).
- Optionally run `gnome-extensions disable ubuntu-dock@ubuntu.com`.
- Codex instead marked the extension enabled for the next session (`W/local-install-0.6.1/activation.json`: `"enabledForNextSession": true`).
  It also disabled ubuntu-dock, dash-to-panel and eleven-start live.
- `enable()` launches win11-panel and win11-desktop itself (`extension.js:167-169`).

**What needs a GNOME re-login.**

- The first install.
- Any change to `extension/*.js`, `stylesheet.css`, `metadata.json` or the schema. GNOME on Wayland cannot reload extension code.
  Codex recorded this as `"requiresGnomeReload": ["notificationsBridge","windowGeometry"]` (`W/native-live-update/deployed.json`) and
  `"requiresNormalGnomeRelogin": ["notification integration","window position restoration","portal frontend backend selection"]`
  (`W/native-tested-update/deployed.json`).
- Changes to the portal backend selection.
- Not needed for binaries and UI: Codex did SIGTERM and relaunched its own win11-panel, win11-desktop and win11-start
  (`W/native-live-update/deploy.py:33-48`). Apps pick up the new build when next opened.

**Uninstall.** Run `./scripts/uninstall.sh [--files-only]`, which calls `uninstall.py`:

- It requires the marker (:17-18). Without `--files-only` it runs `gnome-extensions disable` (:21-22).
- It then restores the logout and Alt+Tab keys from the extension's saved state through **real GSettings** (:25-68). The `from gi.repository import Gio`
  at :27 means **python3-gi is required**.
- It removes the icon aliases and **resets `org.gnome.desktop.interface icon-theme`** if it points at our alias (`icon_bundle.py:110-129`).
  Then it unregisters the portal, removes binaries, desktop entries, extension files and the payload.
- **`--files-only` skips only the `gnome-extensions` call.** The GSettings writes at :25-69 still run against whatever backend is active
  (README: "bedoeld voor een geïsoleerde testinstallatie zonder bediening van een desktopsessie").
- It keeps `~/.config/win11-gnome`, the dconf values under `/org/gnome/shell/extensions/win11-gnome/`, the cache, and the window state.
- The uninstaller is not installed. It imports `install`, `icon_bundle` and `file_portal` from `scripts/`, so the unpacked package or the repo must be kept.

**Second machine (NucBox; assumed Ubuntu 26.04, GNOME 50, x86_64).**

- This PC: Ubuntu 26.04.1 LTS, GNOME Shell 50.1, WebKitGTK 2.52.6, GTK 3.24.52, libarchive 3.8.5, python3-gi 3.56.2, Python 3.14,
  NVIDIA GPU (`nvidia-drm` in `W/gnome-test/shell.log`).
- Binary needs (`readelf -d`): libwebkit2gtk-4.1.so.0, libjavascriptcoregtk-4.1, libsoup-3.0, libgtk-3, libgdk-3, gdk-pixbuf, cairo,
  gio, gobject, glib, libz and glibc ≥ 2.39. libarchive.so.13 is loaded with dlopen (found by `strings`; not in NEEDED).
- Steps:
  1. `sha256sum -c win11-gnome-X-linux-x86_64.tar.gz.sha256`
  2. `tar xzf …`
  3. `cd win11-gnome`
  4. `sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0t64 libarchive13t64 python3-gi libglib2.0-bin desktop-file-utils`
     (whether WebKitGTK 4.1 is on a stock 26.04 desktop is not verified)
  5. `./scripts/install.sh`
  6. Log out and in.
  7. `gnome-extensions enable win11-gnome@win11shell.local`
  8. Optionally disable ubuntu-dock.
- Optional runtime extras: ffmpeg/ffprobe, PackageKit, flatpak, snapd, dpkg-deb, unsquashfs, bubblewrap (README "Bouwen").
- GPU: WebKit uses software rendering by default (`host/src/window.rs:588-593` sets `HardwareAccelerationPolicy::Never` unless
  `WIN11SHELL_GPU=safe|env`), so a different GPU mainly affects Mutter and the compositor-side frosted taskbar strip. Neither is tested on AMD or Intel.
- Codex tested virtual 1440×900, 1280×800+1024×768, 1920×1080 and 3840×2160 monitors, and scale 2 (`W/panel-test-scale2`).
  Mixed DPI was not verified (`validation/desktop-0.7.0.json` limits).

### (d) How Codex tested GUI changes without the live session, and how to do the same for English

**The pattern.** The most complete examples are `W/desktop-bridge-test-0.7.0/run-combined.py`, `W/settings-native-test/run.py` and `W/native-tested-update/install-proof.py`.

1. Start under a private bus. Commands as written in the scripts and logs:
   - `dbus-run-session -- /usr/bin/python3 work/settings-native-test/run.py`
   - `dbus-run-session -- env WIN11_PRIVATE_FROSTED_TEST=1 python3 work/native-frosted-test/run.py 1920x1080`
   - `dbus-run-session -- env WIN11_PRIVATE_ENERGY_UI=1 python3 work/native-energy-test/native-ui.py`
   - `dbus-run-session -- /usr/bin/python3 work/native-installer-launch-test/run.py`
2. Use a private environment (`run-combined.py:24-26`):
   - Remove `DISPLAY`, `WAYLAND_DISPLAY`, `HYPRLAND_INSTANCE_SIGNATURE` and the `WIN11_*` and `WIN11SHELL_*` overrides.
   - Set `HOME`, `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME` and `XDG_RUNTIME_DIR` (mode 0700) to private paths.
   - Set `XDG_DATA_DIRS=/usr/local/share:/usr/share` (excludes the live `~/.local/share`), `GSETTINGS_BACKEND=keyfile`,
     `WIN11_INSTALL_PREFIX` and `WIN11_BUILD_DIR`.
   - Set the dev hooks `WIN11_DEV_COMMAND`, `WIN11_DEV_REPORT` and `WIN11_DEV_SCRIPT`, plus `WIN11SHELL_INDEX_AUTOSTART=0`.
   - Seed `config/win11-gnome/settings.json`, for example `{"language":"en",…}` (:12).
3. Install the project into that prefix with `subprocess.run([PROJECT/'scripts/install.sh'], env=ENV)` (:27).
4. Make the extension controllable, in the private copy only. Patch it with
   `p.write_text(p.read_text().replace('enable() {','enable() {\n global.context.unsafe_mode=true;'))` (:33; also `gnome-test/run.py:22-23`).
   `settings-native-test` installs a separate harness extension instead (:74-78). Then enable it in keyfile GSettings:
   `gsettings set org.gnome.shell enabled-extensions …`.
5. Start the shell:
   `gnome-shell --headless --virtual-monitor=1280x800 --virtual-monitor=1024x768 --wayland-display=desktop-test --no-x11` (:35),
   or `--virtual-monitor=1440x900 --wayland-display=settings-native-test` (`settings-native-test/run.py:260`).
   `gnome-test/run.py:12` added `LIBGL_ALWAYS_SOFTWARE=1 MUTTER_DEBUG_DISABLE_HW_CURSORS=1`.
6. Drive and inspect the shell:
   - Shell state: `org.gnome.Shell.Eval` on the private bus (`run-combined.py:46-50`), for example
     `Main.extensionManager.lookup(UUID).stateObj._bar.height`.
   - Real input: Clutter virtual devices, `seat.create_virtual_device(...)`, `notify_absolute_motion` and `notify_keyval` (:96-102).
   - Launching apps: run `prefix/bin/win11-app --app settings --params '{"page":…}'` with `WAYLAND_DISPLAY=<private> GDK_BACKEND=wayland` (:69-75).
   - In-page JS: written to the `WIN11_DEV_COMMAND` file, which the host polls every 100 ms (`host/src/window.rs:736-760`).
     Results come back as `ipc.postMessage({cmd:'test-report'})` lines in `WIN11_DEV_REPORT` (`host/src/lib.rs:228-240`).
     The init script comes from `WIN11_DEV_SCRIPT` (`window.rs:526-531`).
7. Screenshots: `org.gnome.Shell.Screenshot.Screenshot(false,false,path)` on the private bus (`gnome-test/run.py:80-81`,
   `settings-native-test/run.py:252`). This needs `unsafe_mode` because of the sender allowlist.
8. A pure WebKit variant without the Rust host: `W/start-corners-test/webkit.py`. It starts a headless gnome-shell, serves `vendor/ui`
   over a Python `http.server`, and opens a PyGObject `WebKit2 4.1` window with `GSETTINGS_BACKEND=memory`.
   Not used anywhere: Xvfb (not installed), `mutter --nested`, devkit, a pure-headless WebKit.
9. Install, upgrade and uninstall were tested in a temporary prefix whose path contains spaces, `%` and `$`. They ran with `--files-only`
   and keyfile GSettings (`W/native-tested-update/install-proof.py:6-77`).

**Weak spots in Codex's harnesses.**

- None sets `NO_AT_BRIDGE=1`. They rely on the private `XDG_RUNTIME_DIR`, which keeps at-spi off `/run/user/1000/at-spi/bus_0`
  (see the memory note "Private D-Bus clobbers a11y bus").
- The system bus is shared: the headless shell logs "Registering session with GDM" and "Will monitor session 42" (`W/gnome-test/shell.log`).
- The first harness, `W/gnome-test/run.py` (2026-11-01 09:25), was less isolated:
  - it kept the real HOME;
  - `XDG_DATA_DIRS` included `~/.local/share` (:16);
  - `WIN11_REFERENCE_SETTINGS` pointed at the live `~/.config/win11shell/settings.json` (:17);
  - `WIN11_REFERENCE_INSTANCE` pointed at the live `/run/user/1000/win11shell/instance.json` (:8);
  - it wrote `preview-*.png` into G.

**Steps that touched the live session.** File times come from `W/*`.

| When | Where | What |
|---|---|---|
| 11-01 20:28 | `local-install-0.6` | live install into `~/.local` |
| 11-01 20:37 | `opaque-gtk-fix/changes.json` | edited `~/.themes/Fluent-round-*/gtk-*/gtk.css` |
| 11-01 20:42 | `fluent-uninstall` | removed the Fluent theme |
| 11-01 20:47 | `gnome-reset/results.json` | reset GNOME settings and Ubuntu extensions; backup `gnome-reset/backup/dconf-all.ini` |
| 11-01 21:13 | `local-install-0.6.1/activation.json` | disabled `ubuntu-dock`, `dash-to-panel`, `eleven-start`; enabled our extension for the next session |
| 11-01 22:29 | `local-install-0.7.0` | live quick install |
| 11-02 08:17 | `start-corners-install` | replaced and restarted live Start (`oldPid` → `activePid`) |
| 11-02 10:15 | `interim-ui-update` | live UI files plus a panel restart |
| 11-02 11:09 | `native-live-update/deploy.py:23-48` | live install, `gsettings icon-theme=Win11Shell-dark`, SIGTERM and relaunch of panel and desktop |
| 11-02 12:52 | `native-tested-update/deploy.py` | live install, `systemctl --user daemon-reload`, SIGTERM of panel, desktop and start, relaunch |
| 11-02 13:38 | `native-effects-update` | live install |
| 11-02 22:24 | `native-energy-update/deploy.py:71` | live install, **opened Settings on Robin's display** (`settingsPid` 2707991) |
| 11-04 23:32 | `~/.local/bin/win11-panel` | replaced by an unknown source; sha256 `17437f50a1bf…` is in no work record or Codex log (the other 11 binaries still match the 11-02 22:24 cohort, e.g. win11-start `1ee8b61d…`) |

The extension files installed live are byte-identical to `G/extension` (checked with `cmp`, all 10 files).

**A safe recipe for English screens and the extension.** Proposed; nothing here was run.

```bash
# T must be a fresh private directory kept inside the project (tests/gnome/run/<stamp>), not /tmp (reboot wipes it)
dbus-run-session -- env -u DISPLAY -u WAYLAND_DISPLAY -u WIN11_GNOME_CONFIG_DIR -u WIN11_REFERENCE_SETTINGS \
  NO_AT_BRIDGE=1 GSETTINGS_BACKEND=keyfile HOME=$T/home XDG_CONFIG_HOME=$T/config XDG_DATA_HOME=$T/data \
  XDG_CACHE_HOME=$T/cache XDG_RUNTIME_DIR=$T/runtime XDG_DATA_DIRS=/usr/local/share:/usr/share \
  WIN11_INSTALL_PREFIX=$T/prefix WIN11_BUILD_DIR=<built bins> \
  python3 tests/gnome/harness.py --lang en --shots $T/shots
```

- The harness should abort if `DBUS_SESSION_BUS_ADDRESS` is the live `unix:path=/run/user/1000/bus` or `XDG_RUNTIME_DIR=/run/user/1000`.
  It should signal only its own child PIDs, run one shell at a time (about 0.5-1 GB with a few WebKit hosts; 3.7 GB available now),
  and never call `uninstall.sh` without the keyfile backend.
- Extension checks through Eval:
  - `stateObj._searchButton.child.text` and `.accessible_name`
  - the Ctrl+Alt+Tab group name (`Main.ctrlAltTabManager._items`)
  - `stateObj._launch('missing.desktop')`, then read `Main.messageTray.getSources()` titles and bodies
  - the clock text
- To see the fallback bar, terminate only the private win11-panel child.
- To test a live switch, rewrite `$T/config/win11-gnome/settings.json` `language`.
- `WIN11_REFERENCE_SETTINGS` (`extension.js:21`) can point the extension at a fixture file.
- Port Codex's `desktop-language-test/checks.py` into the project harness, then add the extension, Start, taskbar and installer checks.

## Work needed (estimates in agent-hours)

1. **Extension language support:** 3 h.
   - A `lang()` reader for `settings.json` with `nl` as the default, plus a directory monitor.
   - An inline `{nl,en}` table (H decision D14) for the 9 strings and the clock format.
   - Re-label on change, including the accessible names and a `removeGroup`/`addGroup` for Ctrl+Alt+Tab.
   - Fix `'Everything'` → `'Alles'` (panelBridge.js:387).
   - Dutch output stays byte-identical.
2. **Error texts shown in toasts:** 2 h. Map the extension's `error.code` to texts in the UI language (in the UI or host), or add pairs for
   the 2 Dutch and the most visible English messages. Decide which is better.
3. **`metadata.json` name and description:** 0.25 h. These cannot be localised. Decide on wording (a neutral or bilingual name).
4. **Desktop entries:** 2 h. Add `Name[nl]`/`Name[en]` and `Comment[nl]`/`[en]` (+ `GenericName[en]=File Search` for Alles) in `install.py:111-130`.
   Decide which language the plain keys use. Check Start names in both languages through the host catalog.
5. **Bilingual installer, uninstaller and build output:** 3-4 h. Add `--lang nl|en` (default from the existing `settings.json`, else `nl`)
   for the 37 messages, with inline pairs. A fresh `--lang en` writes `{"language":"en"}` only if no `settings.json` exists.
   Add a Dutch byte-for-byte freeze test (H's `tests/i18n/baseline-nl.txt` approach).
6. **Project-kept private GNOME harness:** 5 h. Write `tests/gnome/harness.py` from Codex's `run-combined.py` and `settings-native-test` patterns,
   adding `NO_AT_BRIDGE`, live-bus guards and a screenshot helper.
7. **Language tests:** 4-6 h, plus about 1 h per full run. Revive `desktop-language-test` and run it for real. Add checks for the extension
   (fallback bar, notifications, Ctrl+Alt+Tab, clock, live switch) and for Dutch staying unchanged.
8. **Screenshot pass over every surface in both languages:** 3-4 h. Covers Start, taskbar, Settings pages, Explorer, Taakbeheer, Alles,
   Schijfbeheer, Installer and Standaardapps, for fidelity review.
9. **Install fixes:** 3 h in total.
   - 0.5 h: the stale 6-binary `G/bin` makes a G-checkout install fail. Prefer `target/release`, or require all 12 BINS before choosing `bin/`.
   - 2 h: runtime preflight in `install.py` for `libwebkit2gtk-4.1.so.0`, GTK3, `libarchive.so.13`, python3-gi, `glib-compile-schemas` and
     the GNOME Shell version against `metadata.json`, with bilingual messages.
   - 1 h: install the uninstaller into the payload (for example `~/.local/share/win11-gnome/scripts` plus a `win11-gnome-uninstall`
     wrapper), so the NucBox does not need the tarball.
10. **First-time activation helper:** 1.5 h. An explicit `--enable` that appends the UUID to `enabled-extensions` for the next login,
    with an optional, separately confirmed ubuntu-dock disable. Clear relogin messages in nl and en.
11. **`--files-only` must not touch GSettings:** 1 h. Fix `uninstall.py:25-69` and `icon_bundle._restore_icon_choice`, or document it and
    guard it in tests.
12. **Package hygiene:** 1 h. Bump the version (`host/Cargo.toml`, `metadata.json` `version`/`version-name`, README). Fix the report list
    (`package.py:36`). Check the archive size after adding the icons (81 MB raw). Repeat a private install, upgrade and uninstall proof
    (reuse `install-proof.py`).
13. **Release build within the memory limit:** 1 h of agent time plus build wall time (not measured). `WIN11_BUILD_JOBS=2 ./scripts/build.sh`,
    then `./scripts/package.sh`.
14. **NucBox procedure, bilingual:** 1.5 h. A short guide covering the apt dependencies, checksum, install, relogin, enable, ubuntu-dock,
    a GPU sanity check and rollback. Robin does the actual install.
15. **Live deploy plan for this PC:** 1 h prep. First record and back up the current live state, including the unknown win11-panel.
    Robin approves. Log out and in for the extension changes.
16. **Input for the Windows Update compatibility check:** 1 h.
    - `shell-version ["50"]` and the private Shell APIs mean a GNOME 51 upgrade disables or breaks the extension. Check that before
      handing over to Ubuntu's upgrader.
    - A polkit rule for password-less updates would need a root install, which `install.py` does not do (user-local only).
    - If update banners use `Main.notify`, they need item 1's language support.

Total: about 34-40 agent-hours, excluding build and test wall time and Robin's relogins.

## Risks

- **Re-login dependency.** No extension change can be verified live without Robin logging out and in. Private headless GNOME is the only
  pre-check, and it is a different session type: virtual monitors, no real GPU output, shared system bus.
- **Live-session leaks from test commands.**
  - `uninstall.sh` without `--files-only` runs `gnome-extensions disable` on whatever session bus it sees (`uninstall.py:21-22`).
  - Even with `--files-only` it writes GSettings: the logout key, the Alt+Tab keys and the icon theme.
  - A test that forgets `dbus-run-session`, `GSETTINGS_BACKEND=keyfile` or a private `XDG_CONFIG_HOME` would change Robin's desktop.
- **a11y clobber.** Without a private `XDG_RUNTIME_DIR` and `NO_AT_BRIDGE=1`, a private bus can take over the real at-spi bus, and every
  new GTK or WebKit app, including the live panel, crashes (see the memory note).
- **GNOME locale versus Win11 language.** Anything that follows GNOME's locale (`app.get_name()`, plain desktop entry `Name=`, GLib date
  formats) shows English to Robin. Changing the plain keys to English changes what he sees in GNOME's own UI even in Dutch mode.
- **Unknown live binary.** The live `win11-panel` has unknown provenance. Deploying a new build overwrites it without a known rollback point
  unless it is backed up first.
- **Packaging gaps.** No 0.7.0 archive is in `dist/`, no `target/release` exists, and `G/bin` is stale. The next package needs a full
  release build, and compiling with 3.7 GB available risks out-of-memory at `-j4`.
- **NucBox libraries.** WebKitGTK 4.1 may be missing on a stock 26.04 (not verified). The installer will not notice, and the apps will fail at launch.
- **Ubuntu 26.10 / GNOME 51.** The extension is limited to `["50"]` and uses private internals (`extension.js:189,199`; `shortcutBridge.js:43`).
- **Thin Dutch coverage.** Most Codex GUI tests ran in English, so the Dutch default has less private coverage than it seems. English work
  needs a Dutch regression run too.
- **Mixed-language errors already today.** English bridge errors show inside the Dutch UI (`shell.js:461`), and Dutch errors would show inside the English UI.

## Not checked

- I ran nothing: no build, no package, no install, no gnome-shell, no tests. The only execution against project files was `verify-ui.py`.
- `panelBridge.js` (1058 lines), `desktopBridge.js` and `frostedBridge.js` were scanned for strings, errors and launch code only, not read in full.
- Whether every bridge error actually reaches a toast was not traced per call site.
- Whether `gnome-extensions enable` works before the first re-login on GNOME 50, and whether the Extensions app localises `metadata.json`.
- Which GNOME version Ubuntu 26.10 ships, and the NucBox hardware and package set.
- Who replaced `~/.local/bin/win11-panel` on 2026-11-04 23:32. I only searched the work records and Codex logs for its hash.
- Of the 125 `W/*` entries I examined `gnome-test`, `desktop-language-test`, `desktop-bridge-test-0.7.0`, `settings-native-test`,
  `start-corners-test/webkit.py`, `native-live-update`, `native-tested-update`, `native-energy-update` (deploy and prepare), `local-install-*`,
  `gnome-reset`, `opaque-gtk-fix`, `interim-ui-update`, `start-corners-install` and `packaging-audit-0.7.0/check.py` (partly).
  Other harnesses were only grepped (for `dbus-run-session`, `--headless`, `NO_AT_BRIDGE` and seeded `language`).
- The Codex session logs (2.2 GB) were only grepped for `run_language_checks`, `WIN11_LANGUAGE_HELPER`, `language-results` and the panel hash.
- The VALIDATION*.md reports were read only in part: the top of VALIDATION.md, VALIDATION_DESKTOP.md and the top of PACKAGING_VALIDATION.md.
