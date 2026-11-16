<p align="center">
  <img src="logo-circle.png" width="72" alt="">
</p>

<h1 align="center">WinPoint</h1>

<p align="center">
  <strong>A Windows 11 style desktop for GNOME, in Rust and GTK 4</strong>
</p>

<p align="center">
  <a href="https://github.com/roboburr/WinPoint/releases/latest"><img src="https://img.shields.io/github/v/release/roboburr/WinPoint" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="Licence: MIT"></a>
  <img src="https://img.shields.io/badge/GNOME-50-4A86CF?logo=gnome&logoColor=white" alt="GNOME 50">
  <img src="https://img.shields.io/badge/Ubuntu-26.04-E95420?logo=ubuntu&logoColor=white" alt="Ubuntu 26.04"><br><br>
  <a href="#screenshots">Screenshots</a> · <a href="#features">Features</a> · <a href="#install">Install</a> · <a href="#build-from-source">Build</a> · <a href="README.nl.md">Nederlands</a>
</p>

WinPoint gives GNOME 50 a Windows 11 style taskbar, Start menu, quick settings, notification centre and desktop, plus its own File Explorer, Settings and Task Manager. A GNOME Shell extension fits it into GNOME; the rest is native GTK 4 and Rust.

<p align="center">
  <img src="docs/screenshots/start.png" width="860" alt="WinPoint on GNOME 50: Start open above the taskbar, over the Lotus wallpaper">
</p>

## Screenshots

<table>
  <tr>
    <td align="center" width="50%"><img src="docs/screenshots/desktop.png" width="100%" alt="The desktop with icons and the taskbar"><br><sub><b>Desktop</b> with icons and the taskbar</sub></td>
    <td align="center" width="50%"><img src="docs/screenshots/winx.png" width="100%" alt="The Win+X menu above the Start button"><br><sub><b>Win+X menu</b>: right-click Start</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/quick-settings.png" width="100%" alt="Quick settings flyout"><br><sub><b>Quick settings</b></sub></td>
    <td align="center"><img src="docs/screenshots/notifications.png" width="100%" alt="Notification centre and calendar"><br><sub><b>Notifications and calendar</b></sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/task-view.png" width="100%" alt="Task View with three windows"><br><sub><b>Task View</b>: Alt+Tab</sub></td>
    <td align="center"><img src="docs/screenshots/explorer.png" width="100%" alt="File Explorer on the home folder"><br><sub><b>File Explorer</b> with tabs</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/settings.png" width="100%" alt="Settings, Personalisation page"><br><sub><b>Settings</b>: Personalisation</sub></td>
    <td align="center"><img src="docs/screenshots/task-manager.png" width="100%" alt="Task Manager, Performance page"><br><sub><b>Task Manager</b>: Performance</sub></td>
  </tr>
  <tr>
    <td align="center"><img src="docs/screenshots/calendar.png" width="100%" alt="Calendar app, month view"><br><sub><b>Calendar</b>: month view</sub></td>
    <td align="center"><img src="docs/screenshots/light.png" width="100%" alt="Start in the light theme"><br><sub><b>Light theme</b></sub></td>
  </tr>
</table>

## Features

- **Taskbar**: pinned and running apps with grouping and jump lists, tray icons, clock, centred or left aligned, optional auto-hide.
- **Start**: pinned apps, All apps, recommended files, and search over apps, settings, files and the web.
- **Quick settings**: Wi-Fi, Bluetooth, flight mode, night light, dark mode, do not disturb, brightness, volume and media controls.
- **Notification centre** with pop-up toasts and a month calendar.
- **Win+X menu** and **Task View**.
- **Desktop** with This PC, Recycle Bin, your files, context menus, rename and drag.
- **Frosted glass** from GNOME Shell's own blur, and Windows 11 caption buttons for your other GTK apps.
- **Apps**: File Explorer (also the Open and Save dialog for apps that use the desktop portal), Settings, Task Manager, Alles (instant file search, modelled on [Everything](https://www.voidtools.com/) by voidtools), Disks, Default apps, App Installer (AppImage, DEB, Flatpak, Snap) and Calendar.
- **Dutch and English**, switchable at any time, separate from GNOME's language.
- **Shortcuts**: the Windows key opens Start, Alt+Tab Task View and Ctrl+Alt+Delete Task Manager.

## Requirements

- Ubuntu 26.04 with GNOME Shell 50 on Wayland, the only tested system. Releases are built for x86_64.
- Python 3 for the installer, which checks everything else (PyGObject included) and names any missing Ubuntu package.
- Optional: FFmpeg for video thumbnails; PackageKit, Flatpak or snapd for App Installer.

## Install

Download `winpoint-<version>-linux-x86_64.tar.gz` from the [latest release](https://github.com/roboburr/WinPoint/releases/latest), then:

```bash
tar -xzf winpoint-*-linux-x86_64.tar.gz
cd winpoint-*/
./scripts/install.sh --lang en
```

The installer puts WinPoint in your home folder (`~/.local`) and adds **WinPoint** as a choice at the login screen; `pkexec` asks for your password once. Sign out, click your name, pick **WinPoint** with the gear at the bottom right, and sign in. Your GNOME and Ubuntu sessions stay as they were.

`--lang nl` or `--lang en` sets the language on a first install; without it the installer guesses from your locale. Change it later in **Settings › Time & language › Language & region**.

To run WinPoint inside your normal GNOME session instead, install with `--no-session` (no password needed), sign out and back in once, and run `gnome-extensions enable win11-gnome@win11shell.local`. Ubuntu Dock stays on there; turn it off with `gnome-extensions disable ubuntu-dock@ubuntu.com`.

## Update

WinPoint checks GitHub once a day and tells you when a new release is out. Click **Download and install** in **Settings › Update & recovery**: WinPoint downloads it, checks it against GitHub's checksum and installs it, keeping your settings. Then restart WinPoint or sign out and back in, as the page says. Automatic checks can be turned off under **Advanced options**; installing a newer release by hand, as above, works too.

## Uninstall

Run these in the unpacked folder of the release you have installed (download that release again if you no longer have it), or in your git checkout:

```bash
./scripts/uninstall.sh
python3 scripts/install-session.py --remove
```

The first command turns the extension off, removes WinPoint's files and puts GNOME's own look back. The second removes the WinPoint choice at the login screen (`pkexec` asks for your password). Your settings (`~/.config/win11-gnome`) and Calendar events (`~/.local/share/win11-gnome-calendar`) stay; for a clean removal, delete those and `~/.cache/win11-gnome` too.

After a `--no-session` install, first turn off **Settings › Personalisation › Colours › Windows 11 caption buttons in apps**, and afterwards pick your theme and wallpaper again in GNOME's Settings.

## Build from source

You need Rust 1.92 or newer (Ubuntu's `cargo` package is new enough) and the GTK 4 development files:

```bash
sudo apt install git build-essential pkg-config cargo libglib2.0-dev libgdk-pixbuf-2.0-dev libgtk-4-dev libarchive13t64
git clone https://github.com/roboburr/WinPoint.git
cd WinPoint
cargo fetch --locked
cargo fetch --locked --manifest-path native/Cargo.toml
./scripts/build.sh --lang en
./scripts/install.sh --lang en
```

`build.sh` builds offline, so the two `cargo fetch` lines download the crates first. The programs land in `target/release`, where the installer finds them. If you set `CARGO_TARGET_DIR`, point the installer at it: `WIN11_BUILD_DIR="$CARGO_TARGET_DIR/release" ./scripts/install.sh`. `WIN11_BUILD_JOBS` sets the number of parallel build jobs (default 2).

## How it works

A GNOME Shell extension starts `win11-shell`, one GTK 4 process for the taskbar, Start, the flyouts and the desktop, and handles window placement, blur and shortcuts. A Rust backend gives it apps, settings, tray icons, notifications, sound, network, power and files over a local JSON API; every app is its own GTK 4 process. Turn the extension off and you have plain GNOME.

Settings live in `~/.config/win11-gnome/settings.json` (defaults in [host/src/settings-defaults.json](host/src/settings-defaults.json)). Contributors: start with [native/ARCH.md](native/ARCH.md), [native/APPS.md](native/APPS.md) and [tests/gnome/README.md](tests/gnome/README.md), and before a pull request run `python3 -B tools/i18n/check.py` (every UI text is a Dutch/English pair) and `tests/rust-sandbox/run-tests.sh`.

## Known limitations

- Task View shows app icons, not live window thumbnails.
- The Win+X shortcut does not work yet; right-click Start for that menu.
- Display settings are read-only in WinPoint's Settings; change them in GNOME's Settings.
- With more than one display, the taskbar, Start and the other flyouts are only on the main display, and the desktop (wallpaper and icons) spans all displays as one area. A taskbar on each display is on the way.
- App Installer has not yet been tested with real system installs.
- Password dialogs, PackageKit and NetworkManager messages and GNOME's own parts follow the system language, not WinPoint's.

## Licence

WinPoint is under the [MIT licence](LICENSE); [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) has the details and [LICENSES/](LICENSES/) the full texts. Exceptions:

- the **Win11 icon theme** by [yeyushengfan258](https://github.com/yeyushengfan258/Win11-icon-theme) (`vendor/icons/`, `vendor/icon-source/` and `vendor/ui/assets/icons/settings.svg`), which is **GPL-3.0-only** (changes in [vendor/ICON_PROVENANCE.md](vendor/ICON_PROVENANCE.md));
- the **Lotus wallpapers**, AI-generated and released under [CC0-1.0](LICENSES/CC0-1.0.txt);
- the Rust crates, which keep their own licences ([LICENSES/RUST-CRATES.md](LICENSES/RUST-CRATES.md)), and two Rust files translated from CPython (PSF-2.0).

WinPoint is an independent project, not affiliated with or endorsed by Microsoft. Windows and Windows 11 are trademarks of Microsoft Corporation. No Microsoft code, assets or fonts are included. Alles follows the look and search syntax of Everything by voidtools; it contains no voidtools code and is not affiliated with voidtools.
