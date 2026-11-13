# Win11 for GNOME, development version 0.8.0

Nederlands: [README.md](README.md). The commands below are the same in both files, Dutch example paths included (`/pad/naar/map` is any folder, a path that starts with `/absoluut/pad` any absolute path).

Win11 for GNOME gives GNOME a Windows 11 shell. The taskbar, its flyouts, Start and the desktop are one native GTK 4 process in Rust: `win11-shell` (GTK 4.22, gtk4-rs), without WebKit. File Explorer, Settings, Task Manager, Alles, Disks, Default apps and the Installer stay HTML, CSS and JavaScript in WebKitGTK (GTK 3), with the familiar Win11Shell interface, icons and window buttons. A GNOME Shell extension connects the taskbar, the desktop, keyboard shortcuts and real window actions to GNOME. The current development target is Ubuntu with GNOME Shell 50 and Wayland.

The default runtime runs entirely in Rust, including the original public services, HTTP/event stream, the Alles indexer and the image helpers. The shell gets apps, pins, settings, the notification area, notifications, sound, network, power and files from `win11-panel --http-only`, the Rust backend without a window or WebKit. The original Python sources under `vendor/host` stay as a reference for comparison tests. Python is still used for the installation, package and development tools; the shell and apps you open do not start a Python backend for that. An explicitly chosen reference backend remains available for development tests.

This is work in progress. No public alpha has been published from this working tree. The current checks, their environment and their limits are at the top of [VALIDATION.md](VALIDATION.md). Those checks prove the scenarios they name; full one-to-one parity on every machine has not been shown yet. The earlier HTML version of the taskbar, Start and the desktop in WebKit has been removed; the reports about it are history.

## Standalone programs

| Program | Purpose |
|---|---|
| `win11-panel` | Starts `win11-shell`; with `--http-only` it serves the shell's services (apps, settings, notification area, notifications, sound, network, power, files) without a window |
| `win11-shell` | The native GTK 4 shell: taskbar, flyouts (Quick Settings, notifications and calendar, jump lists, Win+X, Task view), Start and search, the desktop with its icons and file actions |
| `win11-explorer` | File Explorer with tabs, tab transfer and native file handling |
| `win11-app --app settings` | Settings |
| `win11-app --app taskmgr` | Task Manager |
| `win11-app --app alles` | Alles, with its own index process while this app is open |
| `win11-app --app disks` | Disks |
| `win11-default-apps` | Default apps per file type or extension |
| `win11-calendar` | A Windows 11 style Calendar (GTK 4), opened on a day from the taskbar calendar |
| `win11-installer` | Install and remove proposals with explicit confirmation; the new interface is still being integrated and checked |
| `win11-file-portal` | A FileChooser backend for desktop portals, activated on request |

The index, image and software metadata helpers are internal programs that start only on request. Separate apps do not need a visible panel. Tab transfer between existing File Explorer windows stays available; dropping a tab on the desktop and the real screen position need the GNOME integration.

The extension hides GNOME’s top bar and conflicting old desktop windows while the new surfaces are active. Turning it off, or a failure, restores the original desktop. If `win11-shell` is missing or does not start, the extension shows its simple fallback taskbar after 6 seconds. Installing does not turn off other dock extensions. The Windows key opens Start, Ctrl+Alt+Delete opens Task Manager and Alt+Tab uses Task view. The settings control, among other things, the language, taskbar alignment, wallpaper, colours and theme. Dutch and English also apply to desktop labels such as Prullenbak/Recycle Bin.

## Files, apps and settings

The native file core covers navigation, metadata, search, folder watching, open with, creating, renaming, copying/moving, the clipboard, the Recycle Bin, ZIP/TAR archives and properties. Archives use `libarchive`; the Recycle Bin and app associations use GIO. Atomic operations refuse to overwrite existing targets. The stored settings use private, atomically written files and shared file locks.

**Settings › System › Power & battery** controls the power mode, turning off the screen, dimming and automatic sleep on mains power or battery. The Rust service uses the same GNOME preferences and the PowerProfiles service as Ubuntu. Existing custom times and ‘Never’ are kept; changes made in GNOME also show in this window. Availability and administrator locks are shown per setting. Battery options appear only when there is a battery.

The original `apps.*` functions, recent files and the app catalogue run in Rust. **Default apps** adds a separate program in which you search by extension or MIME type and link exactly the chosen file type to a relevant installed app. The effective desktop priority is read back. Opening or installing does not change these associations automatically.

Normal app windows keep their last normal position, size and maximised state. File Explorer also keeps the meaning of up to twenty tabs. A path or tab transfer given explicitly wins over the restore. A restore keeps the window inside an available screen. Start, the taskbar, the desktop and temporary file pickers are excluded.

The File Explorer details pane asks for a video thumbnail only for the selected video that is visible. Existing freedesktop thumbnails are reused; otherwise a short-lived native helper makes one still frame with FFmpeg. No video player runs in the background. Without a suitable decoder, or on an error, the original file icon stays. The new video route is an explicit extension; the original image routes keep their behaviour. See [host/src/images/README.md](host/src/images/README.md).

## Installing and removing apps

The new software service recognises AppImage, DEB, Flatpak references and bundles, and Snap packages. Inspecting a downloaded AppImage does not run it. An install proposal keeps the bytes to review privately; only a separate confirmation carries out the one-time proposal. Proposals expire after ten minutes. One software change can be active at a time. Status and cancelling are kept for other windows; an unknown outcome is reported as interrupted.

AppImages get a managed copy under `~/.local/opt/win11shell-appimages` and their own launcher. The original downloaded file stays where it is. Removing touches only the registered app copy and launcher; unexpected extra files are kept. For a separate user launcher the action is called **Remove shortcut**, and the app files stay.

DEB uses typed PackageKit transactions and a check simulation; additional or protected package removals are refused. Snap uses snapd’s Unix API and its progress and cancelling. Flatpak uses fixed arguments and the user or system installation that was determined. Any authentication goes through the operating system. No passwords are collected. Personal Flatpak data is not removed, and removing a Snap does not purge it.

The engine has been checked with generated files and imitated package services. Real system installs, removals and every distribution-specific authentication dialog have not been shown with that yet. The new Installer interface and its links with Settings and Start are validated separately before a release.

## File picker through desktop portals

`win11-file-portal` implements OpenFile, SaveFile and SaveFiles behind `xdg-desktop-portal`. It uses the same File Explorer interface and native validation as the built-in picker. Only this FileChooser backend is selected; the portal front end stays responsible for app identity and document access. The picker does not create or overwrite the chosen file itself.

Apps must use the desktop portal to see this picker. An app’s own built-in picker does not change. The backend stops after fifteen seconds without requests and shares one WebKit context only between its own temporary picker windows. Details and protocol limits are in [host/src/portal/README.md](host/src/portal/README.md). Registration, backend selection and installation are checked separately.

## Memory and frosted glass

Services start only when a surface needs them. The Alles indexer is a separate Rust process and belongs only to the open Alles app. The original v2 index cache stays usable. The taskbar, flyouts, Start and the desktop are windows of one GTK 4 process, so opening Start starts no new process. Normal apps stay separate processes.

In a private 1440×900 session with software rendering, `win11-shell`, its backend and the SVG loader together used **119 MiB PSS and 76 MiB anonymous memory plus swap**, against 443 and 269 MiB for the earlier WebKit taskbar, Start and desktop with all their WebKit processes (`tests/gnome/scenarios/native-shell.py`). The shell uses GTK's cairo renderer by default (`GSK_RENDERER=cairo`); with the GL renderer (llvmpipe) it was 244 and 113 MiB. PSS counts shared memory proportionally and differs from a summed RSS column. This measurement does not include compositor memory or GPU VRAM. See [notes/claude/gtk-native.md](notes/claude/gtk-native.md).

The shell's backend (`win11-panel --http-only`) runs in its own limited systemd group when the user manager is available; `win11-shell` itself is still outside it. Apps that are started get their own groups. This policy and these measurements are no general memory guarantee for every video decoder, extra app or screen configuration.

Frosted glass comes from GNOME Shell itself, not from GTK or WebKit: behind the taskbar, behind the open native Start menu and behind the focused own app window, with this opacity (taskbar 70 %, app windows and Start 90 %). At most one window blur exists at a time; a closed Start keeps none. In a private 1440×900 session with software rendering it cost gnome-shell 3.8 MiB PSS with Settings and 8.9 MiB with Start open, still measured with the WebKit Start (`tests/gnome/scenarios/frosted-glass.py`). With Transparency effects off or high contrast everything stays opaque. Blur inside the page of an HTML app (menus within an app) stays off: it needs WebKit on the GPU, which froze on NVIDIA before.

## Building

Requires Rust/Cargo with support for edition 2024 (Rust 1.92 or newer), a C/C++ build environment, GTK 3 and WebKitGTK 4.1 for the apps, and GTK 4.22 or newer (GNOME 50) for `win11-shell` and `win11-calendar`. On the tested Ubuntu environment (Ubuntu 26.04, GTK 4.22):

```bash
sudo apt install build-essential pkg-config libgtk-3-dev libwebkit2gtk-4.1-dev libgtk-4-dev libarchive13t64
cargo fetch --locked
cargo fetch --locked --manifest-path native/Cargo.toml
./scripts/build.sh
```

`./scripts/build.sh` builds the host (the workspace in the top folder) and then the native workspace `native/` with `win11-shell` and `win11-calendar`. These are two workspaces because gtk4-rs 0.11 uses a newer glib than the GTK 3 and WebKit host, and Cargo allows one `glib-sys` per workspace. Both build offline and `--locked`, so `cargo fetch` gets the crates once beforehand. Crates and build output can live elsewhere, on another disk for example, through the usual Cargo variables. Set them before `cargo fetch` as well, and give the installer the folder with the binaries:

```bash
export CARGO_HOME=/absoluut/pad/cargo-home CARGO_TARGET_DIR=/absoluut/pad/target
./scripts/build.sh
WIN11_BUILD_DIR="$CARGO_TARGET_DIR/release" ./scripts/install.sh
```

The archive core needs the system library `libarchive.so.13`; package names can differ per distribution. Network, audio, storage, power and notifications use the matching Linux and GNOME services. Video thumbnails optionally use FFmpeg/FFprobe. Installing software uses PackageKit, Flatpak or snapd, depending on the package; DEB/Snap metadata uses `dpkg-deb`/`unsquashfs`. Where bubblewrap isolation is available, it is used for untrusted metadata and image decoders, next to separate helpers and resource limits.

Python 3 is needed for the current installation and package tools and some development tests. PyGObject is needed only when the original Python reference is tested explicitly. The package bundles no Python environment and no copies of system libraries.

Check the stored original UI baseline and the current distribution manifests with:

```bash
python3 scripts/verify-ui.py
```

`vendor/ui-original-sha256.json` keeps the original hashes; `vendor/ui-sha256.json` and `vendor/ui-patches.json` describe the current interface and the targeted changes. The original source files are a starting point, not a claim that every file stayed unchanged.

## Installing and opening

From a built project or a prepared package with `bin/`:

```bash
./scripts/install.sh
~/.local/bin/win11-explorer /pad/naar/map
~/.local/bin/win11-app --app settings
~/.local/bin/win11-default-apps
```

The installer checks the required programs first and puts the marked project files under `~/.local` by default. Existing personal settings are kept. An install over an earlier version with the WebKit shell removes the old `win11-start` and `win11-desktop` and their app entries. For the shell integration GNOME must load the new extension code; after a new install or an extension change you may need to sign out and back in once. Then turn the integration on explicitly:

```bash
gnome-extensions enable win11-gnome@win11shell.local
```

An Ubuntu Dock or another taskbar extension that is already active can stay visible at the same time. If you want, turn off a conflicting taskbar extension before you activate the new shell, for example:

```bash
gnome-extensions disable ubuntu-dock@ubuntu.com
```

No Fluent GTK theme is installed. The default app theme family is `system`. Installing does not choose a new default file manager and does not change personal MIME associations.

For another install location, `WIN11_INSTALL_PREFIX` sets where `bin/` and `share/win11-gnome/` go; `XDG_DATA_HOME` sets where the application entries, icon aliases and the extension go. Use absolute paths. Removing needs the same values:

```bash
WIN11_INSTALL_PREFIX="/absoluut/pad/met spaties" ./scripts/install.sh
WIN11_INSTALL_PREFIX="/absoluut/pad/met spaties" ./scripts/uninstall.sh
```

Removing touches the project’s own marked files and keeps personal settings. `./scripts/uninstall.sh --files-only` is meant for an isolated test install without controlling a desktop session.

## Language

Win11 for GNOME has its own language setting: Dutch (the default) or English. It is separate from the language of GNOME and Ubuntu. Choose the language in **Settings › Time & language › Language & region**. Every open Win11 window switches at once: the taskbar, Start, the desktop, File Explorer, the apps and their window titles. The shell's backend (`win11-panel --http-only`) also renames the project’s own app entries, so GNOME shows them in the same language. Notifications that had already arrived keep their language. A first install can set the language straight away; a choice that is already in `settings.json` is kept:

```bash
./scripts/install.sh --lang en
```

A few texts do not come from Win11 and stay in the language of GNOME or the system: polkit’s password dialogs, PackageKit’s messages and package descriptions, NetworkManager’s texts, WebKit’s own texts and the parts of GNOME itself, such as Quick Settings and GNOME’s notice that a window is ready (“X” is ready). Alles is called Alles in both languages, and folders on disk, such as Bureaublad and Documenten, are never renamed.

The variable `WIN11_GNOME_CONFIG_DIR` makes the Rust programs use another settings folder. It is meant for tests and development only: the GNOME extension and the installer always read `$XDG_CONFIG_HOME/win11-gnome` (normally `~/.config/win11-gnome`), so in a normal session the taskbar and the windows could each show a different language. So do not set it in a normal session.

## Source code and release

Win11 for GNOME's own code is covered by [LICENSE](LICENSE) (MIT): the Rust host, the native GTK 4 shell and Calendar under `native/`, the installation code and also the preserved Win11Shell sources under `vendor/ui`, `vendor/host` and `vendor/config`, which are the author's own earlier work. The exceptions are listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The bundled Win11 icon theme has its own GPL-3.0 licence, author details and source archive under `vendor/`; the package manifests check its origin and symlinks. See, among others, [vendor/ui/assets/icons/NOTICE.txt](vendor/ui/assets/icons/NOTICE.txt) and `vendor/icon-bundle.json`. The project licence grants no additional rights to third-party material.

The public profile comes from `host/src/settings-defaults.json`. Personal history, accounts, index caches, machine-specific paths and runtime tokens do not belong in a download package. `./scripts/package.sh` makes an archive under `dist/` after building and testing, which also contains `LICENSE`, `THIRD_PARTY_NOTICES.md` and `LICENSES/`; it does not generate test evidence or a public release by itself.

The conditions for publication are in [RELEASE_PLAN.md](RELEASE_PLAN.md). Before a release, the final integrated build, front end, installing and removing, the supported systems and the asset notices must be checked together. Historical reports stay in the project with their original version and measurement date.

## Licences

- Win11 for GNOME's own code and assets are licensed under the MIT licence: see [LICENSE](LICENSE). This includes `vendor/ui`, `vendor/host` and `vendor/config`, own work from the earlier Win11 Shell project, except the third-party parts listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
- The bundled Win11 icon theme (`vendor/icons/`, `vendor/icon-source/` and `vendor/ui/assets/icons/settings.svg`) is licensed under GPL-3.0-only, not MIT. Its changes are listed in [vendor/ICON_PROVENANCE.md](vendor/ICON_PROVENANCE.md).
- The default Lotus wallpapers (`vendor/ui/assets/wallpapers/lotus-dark.svg` and `lotus-light.svg`) are AI-generated (OpenAI ChatGPT image generation) and licensed under CC0-1.0: see [LICENSES/CC0-1.0.txt](LICENSES/CC0-1.0.txt).
- Third-party parts keep their own licences. Which they are, where they are and what their licences ask for is in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). The full licence texts are in [LICENSES/](LICENSES/), and the Rust crates in the programs are listed in [LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md).
