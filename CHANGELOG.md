# Changelog

What changed in each release of WinPoint, newest first. `tools/release.py` takes a release’s notes from its
section here.

## 0.9.5

- **Compact Start profile photos.** A large or rectangular account picture stays inside the 32-pixel circle in
  Start's footer, without increasing the footer height or squeezing the account name.
- **Stable glass backgrounds.** Frosted surfaces use a shared snapshot of the desktop on their monitor instead
  of reading the live foreground framebuffer. Hover, Start and dropdown updates cannot feed their own contents
  back into the blur. Desktop changes refresh the source; clipping is bounded at the taskbar edge. The global
  full-screen redraw workaround from 0.9.4 is removed. Glass now reflects the desktop rather than other app windows.

## 0.9.4

- **Clean glass around icons and menus.** While a frosted backdrop is active, redraw complete frames so the
  background blur cannot reuse old icon, text or dropdown pixels. This addresses square coloured halos and
  flickering on partial updates. It does not schedule continuous repaints while the desktop is idle, and restores
  the previous redraw setting when no backdrop remains.
- **Compact Start search results.** Glyph tiles keep their square size instead of stretching the detail icon and
  search rows to fill the available height. Includes rendered allocation checks for web, settings and app searches.
- Includes the Start-close session crash fix from 0.9.3.

## 0.9.3

- **Closing Start keeps the session running.** Fixed a GNOME/Mutter crash that could return you to the login
  screen when Start closed while a layout change was pending. WinPoint now stops repositioning the window as soon
  as Mutter starts removing it, and checks that it still belongs to a workspace before moving or resizing it.
- **Repeated open and close.** A regression reproduces the original Mutter assertion on 0.9.2 and checks closing
  with the Start button, Escape and a click outside while placement changes are pending.

## 0.9.2

This release includes the Settings integration and fixes from the unpublished 0.9.1 development version.

- **Easier install.** One line in a terminal and one password: the installer puts WinPoint in your home folder, adds
  it to the login screen next to Ubuntu and installs its system files for Update & recovery, all in one step. Sign
  out and WinPoint starts at the next sign-in, already chosen at the login screen. Your Ubuntu session keeps its own
  look, dock and extensions. An account that isn’t an administrator needs an administrator’s password.
- **English from the first line.** With `--lang en`, every message of a first install is in English, also around the
  password step. The password window itself follows Ubuntu’s language.
- **Uninstall with one command.** Sign in to Ubuntu and run `~/.local/bin/winpoint-uninstall`: one password removes
  WinPoint, its choice at the login screen and its system files, and GNOME gets its own look back. Your settings stay.
- **Always its own session.** A new install always gives WinPoint its own choice at the login screen; installing it
  inside your normal GNOME session is no longer offered. Installs made that way keep getting updates; installing
  over one with the line moves WinPoint to its own session.
- **Update with one click.** On 0.9.0 or the 0.9.1 development version, **Download and install** in Settings › Update & recovery installs 0.9.2;
  restart WinPoint or sign out to finish, as the page says.

- **More Settings controls.** Display, sound, power, devices, networking, accounts, language, accessibility and privacy
  use GNOME's settings and services in Windows-style pages. Hardware and installed services decide which controls
  are available; Settings and Start share the same page list and capability requirements.
- **Input and accessibility.** Mouse and touchpad options, tablet/touchscreen mapping, input languages, shortcuts,
  magnifier, screen reader and keyboard/mouse accessibility have their own pages. Related writes are checked
  together, shortcut replacement preserves unrelated bindings, and reset protects shortcuts held by WinPoint.
- **Display, sound and power.** Change supported display layouts with a keep/revert question, configure sound devices
  and app volumes, and choose supported battery and power options. Theme wallpapers keep their light/dark pairing;
  switching to a built-in wallpaper clears an obsolete dark-path override.
- **Network and sharing.** Edit Wi-Fi/Ethernet connections, common VPN profiles, proxy and mobile-network settings,
  configure a hotspot and use supported GNOME remote-desktop, SSH and media-sharing services. Confirmation dialogs
  and credentials follow their service's rules. VPN file import saves with NetworkManager's interactive authorization,
  and APN credentials also apply to the attach bearer. Optional-plugin and real-hardware limits are documented.
- **Accounts, language and privacy.** Account and sign-in pages, language and regional format, app permissions,
  notifications, diagnostics and search settings expose the supported GNOME functions. Changes made elsewhere
  update open controls, and managed or read-only settings remain protected.
- **A taskbar on each display.** Additional displays get their own taskbar and desktop. Taskbars default to the
  current desktop's windows, while Alt+Tab initially follows GNOME's existing preference. The tray can show the
  weekday, battery percentage and the active input language when several keyboards are configured.
- **Compact media controls.** Large album artwork stays within the quick-settings card instead of stretching the
  panel across the screen. Dialog buttons, header switches and navigation breadcrumbs have dedicated regressions.


## 0.9.0

- **Version number and update check.** WinPoint now has a version number, and it checks GitHub for a new release
  once a day. Settings › Update & recovery shows whether you’re up to date, with a **Check for updates** button.
  When a new version is out, you get one notification, and the page links to its release notes and download. You
  can turn the daily check off in Update & recovery › Advanced options.
- **Updates with one click.** **Download and install** in Settings › Update & recovery downloads the new release from
  GitHub, checks it against GitHub’s checksum and installs it; restart WinPoint or sign out to finish. Nothing is
  installed without that click.
- **Smoother taskbar.** Clicks on the window behind an open flyout work again, and a flyout closes when you click
  elsewhere. The taskbar and Start no longer jump when a tray icon changes, menus of tray icons reach their app again,
  and the desktop no longer stutters every few seconds.
- **Back after the lock screen.** The taskbar and Start’s frosted glass come back after you lock and unlock.
- **Coloured small icons.** Folders, drives and This PC are in colour at small sizes too, in File Explorer’s
  navigation pane and file list and in Alles.
- **Minimise into the taskbar.** A minimised window shrinks into its taskbar button, as on Windows.
