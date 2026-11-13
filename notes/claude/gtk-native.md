# Native GTK 4 shell (D-G63)

Robin, 2026-11-12: "we gaan voor de GTK route", "kan het in combinatie met rust?", "we gaan voor de laatste GTK en
met rust". The resident surfaces (taskbar, its flyouts, Start, desktop) move from HTML in WebKit to native GTK 4
widgets in Rust. Goal: less RAM (no WebKit web/network process per surface, no growing web process), the same
Windows 11 look and behaviour, and the HTML version kept until each part is as good. Kept until D-G70 (13-11, Robin:
"ik wil de GTK4 versie op github hebben, verwijder de GTK3 variant"): the HTML panel, Start and desktop are removed
and win11-shell is the only shell (section D-G70 below).

## Measured starting point (live, 12-11, a 29 min old session)

| Surface | in RAM (PSS) | in swap | own (anon + swap) |
|---|---|---|---|
| Taskbar: host 33 + web 60 + network 3 | 96 MiB | 134 MiB | 196 MiB |
| Start: host 11 + web 10 + network 3 | 24 MiB | 202 MiB | 213 MiB |
| Desktop | 44 MiB | 71 MiB | 97 MiB |

The taskbar's web process grew to 508 MiB own memory in 26 h (D-G6x leak, cause not found; the bare private
session does not grow). Target for the native process with taskbar, flyouts and Start: well under 100 MiB in all,
measured the same way (PSS + SwapPss and Anonymous + Swap from smaps_rollup).

## Toolchain (done 12-11)

- GTK 4.22.4 (Ubuntu 26.04, GNOME 50); Robin installed `libgtk-4-dev`.
- gtk4-rs 0.11.5 with feature `gnome_50` (v4_22, gio 2.88); Rust 1.97.
- `native/` is its own Cargo workspace: gtk4-rs 0.11 needs glib 0.22-era crates, the host (gtk 0.18 for GTK 3,
  webkit2gtk 2.0 through wry) is fixed on glib 0.18, and Cargo allows one `glib-sys` (links = "glib-2.0") per
  workspace. The root workspace excludes `native`.
- Crates and build output on the 22 TB disk (Robin: "naar 22TB"): `CARGO_HOME` and `CARGO_TARGET_DIR` under
  `/run/media/robin/22TB/win11-gnome-build`. Builds through `tools/heavy.sh` as before (C8).

Build (from G/native):
    tools/heavy.sh --min-mb 2500 --tag native -- env CARGO_HOME=/run/media/robin/22TB/win11-gnome-build/cargo-home \
        CARGO_TARGET_DIR=/run/media/robin/22TB/win11-gnome-build/target cargo build --locked

## Architecture

- One process, `win11-shell` (GTK 4), holds the taskbar window and later Start, the flyouts and the desktop as
  its own windows. One GTK 4 instance instead of three WebKit hosts.
- The GNOME extension stays as it is. The taskbar window keeps the panel's identity
  (`io.github.win11shell.Panel`), its full-monitor transparent window with an input region, and the D-Bus calls
  the panel host makes today (`panel.geometry`, `panel.keyboard`, `nativeBackdrop.configure` with the shell's own
  pid, `panel.toggleStart`, window events). The shell makes those calls itself, so the pid the extension checks is
  its own.
- Content services (apps and pins, settings.json, i18n, tray, notifications, audio, network, power, files) stay
  in the host's Rust code. During the move the shell uses them through the host's existing JSON API and event
  stream, from a host started in a service mode without a window or WebKit. When the remaining HTML apps move to
  WebKitGTK 6.0 (GTK 4) the whole project can share one glib, and the services can move into the shell process.
- Look: the same tokens as tokens.css/shell.css, as GTK CSS (GTK 4.16+ has CSS custom properties). Fluent icons
  from the UI's own assets. Blur stays the extension's (the taskbar strip is the same `kind: 'taskbar'`).

## Steps

1. Taskbar strip: Start button, search pill, pinned and running app buttons (grouping, focus, minimize, middle
   click, indicator), tray icons, clock and date, notification bell, quick settings button. Same size (48 px),
   same alignment setting, same blur.
2. Flyouts: quick settings, calendar and notifications, tray overflow, jump lists, Win+X, task view.
3. Start (pinned, all apps, search, recommended, user and power menus), keeping the power hand-off (D-G58).
4. Desktop (wallpaper, icons, context menus, rename, drag and drop).
5. Switch the default and remove WebKit from these surfaces; the HTML apps (Explorer, Settings, Task Manager)
   stay HTML and start on demand. Done (D-G70).

Each step: a harness scenario (the shell gets a dev channel like the WebView hosts: WIN11_DEV_COMMAND /
WIN11_DEV_REPORT, reporting its widget state and taking synthetic clicks through the real pointer), screenshots
against the HTML version, and a memory measurement against the table above. The HTML surface stayed the default
until D-G70; since then there is no switch (shell.native is ignored) and no WebKit fallback.

## State after the first build (12-11)

Built by eight parallel agents and integrated (workflow wf_5358eb5c-c6e). `native-shell` 20/20: the switch, the
taskbar at the monitor's bottom edge as the extension's Panel, quick settings and notifications open and close,
Start opens above the taskbar with its glass and closes, the desktop shows wallpaper and icons, the WebKit panel,
Start and desktop are gone. Memory in the private session (1440x900, software rendering):

| | PSS | anonymous + swap |
|---|---|---|
| WebKit panel + Start + desktop trees | 443 MiB | 269 MiB |
| native, GL renderer (llvmpipe) | 244 MiB | 113 MiB |
| native, cairo renderer (default now) | 119 MiB | 76 MiB |

Known gaps (not exercised by the smoke): jump lists, Win+X, tray overflow, task view, the power dialog, Start
search and All apps, desktop menus, rename and drag, toasts, tray drag, arrow-key navigation, setIconRects,
start-quit only hides, the glyph SVGs go through the glycin loader process (about 8 MiB). Closed by D-G70: uninstall.py
removes win11-shell, and there is no WebKit Start pid any more.

## D-G70: the only shell (13-11)

Removed: the WebKit panel, Start and desktop surfaces (host bins win11-start and win11-desktop, their adapters and
entries, vendor/ui/js/shell/, js/desktop/, shell.css, flyouts.css and the WebKit-only scenarios and reports). Kept: the
backend (`win11-panel --http-only`, host/src/lib.rs run_http_only), the HTML apps in WebKitGTK (win11-app, -explorer,
-default-apps, -installer and the workers), native/calendar and what they import from vendor/ui. The old HTML stays
readable as `git show 418ea594:<path>`.

New gaps, now the default:
- `shell.restartAll` (Windows Update) re-execs only the backend, not win11-shell; the update restart scan counts
  win11-shell as an own process, so the "Win11 Shell needs to restart" InfoBar can stay.
- The `shell.command` events (Task Manager toggle-search, Settings Clipboard "open") have no consumer.
- No WebKit fallback: a missing or failing win11-shell ends win11-panel, and the extension shows its St bar after 6 s.
  A `--dry-run` given to win11-panel reaches win11-shell, which ignores it.
- win11-shell is exec'd before ensure_native, so it runs outside app-win11shell.slice (its backend is inside).
- A live install keeps the old ~/.local/bin/win11-start and win11-desktop, their entries and shell.native until the
  next install.py run cleans them.
- The gx1-restart and wf-restart-copy scenarios assumed restartable WebKit hosts; Windows Update restart has no
  scenario coverage until they are ported.
