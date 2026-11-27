# Changelog

What changed in each release of WinPoint, newest first. `tools/release.py` takes a release’s notes from its
section here.

## Unreleased

## 0.9.11

2026-11-27

- **Use the desktop's bottom and right edges for icons.** Spread leftover space between grid cells instead of
  leaving an unused strip beside the last column and above the taskbar. Saved cell coordinates and icon sizes
  stay the same; dragging and grid alignment use the same spacing, including with the auto-hide preference enabled.
  Reserve the taskbar only once when Mutter has already constrained the desktop to the available work area.
- **Keep edge placement consistent.** Drawing, dragging and placing new or pasted items use the same rounded
  grid boundaries, including fractional spacing. Icon size and text scaling retain their reserved tile footprint.
- **Verify placement and persistence.** Isolated GNOME tests cover real drags with all three icon sizes and
  100%, 150% and 200% text sizes, the actual taskbar boundary, free placement and saved positions after restarting
  only the test shell. Shared geometry tests also cover different logical screen scales and small work areas.

## 0.9.10

2026-11-27

- **Restore window snapping on affected fresh Ubuntu installs.** At the start of the WinPoint panel, repair the
  specific configuration where native edge snapping is off and both left/right tile shortcuts are empty. This
  restores dragging windows to screen edges and Win+Left/Right in WinPoint.
- **Keep existing snap setups.** The repair runs only in the WinPoint GNOME session. A bounded, read-only GNOME
  Shell check skips it when Tiling Assistant is active, changing state, or cannot be verified. Custom shortcut
  bindings and other configurations are left as they are.
- **Preserve GNOME's previous settings.** Verify the saved values before changing the three preferences; if a
  write fails, roll back only completed writes that still hold WinPoint's values. Choosing Ubuntu restores the
  recorded preferences when they still match WinPoint's managed values.
- **Cover the repair and recovery paths.** Regression tests cover the exact disabled profile, session and dry-run
  guards, Tiling Assistant states, saved-value restoration, partial-write rollback and refusal without a confirmed
  backup. The login-session mode files and external extension settings remain unchanged.

## 0.9.9

- **Kernel-assisted file transfers.** Explorer uses Linux's `copy_file_range` API between already-open files
  where supported, in chunks of at most 8 MiB. Other filesystems use a bounded buffered fallback at the current
  file offsets, with one reusable 1 MiB buffer per job. Short copies, interrupted operations and partially
  completed writes retain correct byte accounting; destination ownership is checked before writes.
- **Move replacements on the same filesystem without recopying their contents.** After the existing replace
  confirmation, eligible moves use rename with the existing destination checks and rollback safeguards.
  Cross-filesystem moves retain the validated copy-before-delete path; directory merges keep their existing rules.
- **Handle limited rename support on NTFS/FUSE mounts.** File and link replacements use exclusively writable
  staging and no-replace publication when the filesystem rejects Linux's no-replace rename flag. Unfinished
  private aliases are retained as recovery paths. Ordinary file and folder moves can use the validated
  copy-before-delete path. Merging existing folders keeps its earlier rules.
- **Keep the Windows-style transfer window and its controls.** The compact percentage bar, expanded green speed
  graph, details and folder links remain, together with pause, resume, cancel, replace, skip, keep both and retry.
  Cancellation and changed-file checks remain part of each bounded transfer step.

## 0.9.8

- **A separate file-transfer window.** Long copies, moves and pastes use a Windows-style progress window instead
  of the wide inline Explorer card. Switch between the compact percentage bar and expanded green speed graph,
  with the current file, remaining time and recursive item count. Source and destination folders are links.
- **Working pause, resume and cancel.** Controls affect only their own transfer. Closing the progress window or
  pressing Escape requests cancellation; minimising it keeps the transfer running. Closing the source Explorer
  does not interrupt it. Cancellation preserves completed items and detects changed partial destinations;
  cross-filesystem moves validate the copied destination before removing the source.
  Retried or discarded staging copies do not count twice toward completion; uncertain totals stay unknown.

- **Choose what happens when files already exist.** Explorer asks whether to replace, skip or keep both, with an
  option to apply the choice to later conflicts. Existing folders merge their contents; unrelated files stay
  in place. Recoverable errors offer retry, skip or cancel when retrying is safe.
- **Completed updates no longer remain stuck as running.** When the completed snapshot proves that the same
  update ended, an older unfinished task record cannot keep the progress screen or block WinPoint updates.
  Real active updates retain their safety checks.

## 0.9.7

- **Type in the taskbar, see results in Start.** The full search box is editable. Start mirrors the query while
  the caret stays in the taskbar; clearing, keyboard selection, Escape and switching between both fields retain
  the same search. Closing search releases the keyboard and preserves the chosen app's focus.
- **A clearer volume slider.** Quick Settings uses a compact light thumb with an accent centre, a subtle border
  and a thin track, with the existing keyboard controls and larger pointer target retained.
- **Keep external app helpers with their app in Task Manager.** Recognize the application from its windows and
  executable before using a launcher scope name. Helpers remain with the same app across launcher services;
  different apps stay separate, and each process contributes its memory once. Unidentified external launcher
  groups use “External apps” instead of “Win11 GNOME external”.
- **Activate the bundled Windows icons on a fresh install.** Select the Win11Shell icon aliases installed by
  WinPoint, with the matching light or dark variant. Separately installed Win11 themes remain a fallback.
- **Keep the taskbar visible when Start opens or closes.** Restore the compositor glass layers below their
  native windows before the next frame after GNOME restacks windows. Previously an idle callback could run
  one frame late, briefly covering the taskbar icons and tint with the glass background. The correction only
  restores layer order; it preserves the existing geometry, content snapshots and Start frame gates.

## 0.9.6

- **Glass shows the windows behind it.** Start, the taskbar and focused app glass compose cached window contents
  in stacking order, instead of showing only the wallpaper as in 0.9.5. Only visible underlying windows on the
  current desktop enter the source; the glass and its own client are excluded. Window updates refresh shared
  snapshots on demand, without reading the foreground framebuffer or changing global redraw flags.
- **No bare glass when Start opens or closes.** The native client enables the backdrop after painting its fresh
  opening frame. Delayed opening callbacks are cancelled by a close or another open. Compositor fades, hides and
  transforms suspend Start's independent backdrop immediately.

## 0.9.5

- **Compact Start profile photos.** A large or rectangular account picture stays inside the 32-pixel circle in
  Start's footer, without increasing the footer height or squeezing the account name.
- **Complete app groups in Task Manager.** Helpers in an application's service are included in that app's row
  instead of a separate background launcher group. This applies to all identified apps, including reparented
  helpers; different apps in a shared service follow their own process trees. Every process is counted once.
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
