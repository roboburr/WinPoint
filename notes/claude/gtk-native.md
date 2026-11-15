# Native GTK 4 shell (D-G63)

Robin, 2026-11-12: "we gaan voor de GTK route", "kan het in combinatie met rust?", "we gaan voor de laatste GTK en
met rust". The resident surfaces (taskbar, its flyouts, Start, desktop) move from HTML in WebKit to native GTK 4
widgets in Rust. Goal: less RAM (no WebKit web/network process per surface, no growing web process), the same
Windows 11 look and behaviour, and the HTML version kept until each part is as good. Kept until D-G70 (13-11, Robin:
"ik wil de GTK4 versie op github hebben, verwijder de GTK3 variant"): the HTML panel, Start and desktop are removed
and win11-shell is the only shell (section D-G70 below). Since D-G72 (14-11) the apps are GTK 4 as well and WinPoint
has no WebKit; the host links no GTK (section D-G72 below).

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
  webkit2gtk 2.0 through wry; since D-G72 only glib, gio and gdk-pixbuf 0.18) is fixed on glib 0.18, and Cargo
  allows one `glib-sys` (links = "glib-2.0") per workspace. The root workspace excludes `native`.
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
  (Not what happened: D-G72 rebuilt the apps as native GTK 4 programs instead and removed WebKit; the host's
  services stayed on glib 0.18 in their own processes, so the two workspaces stay.)
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
- ~~`shell.restartAll` (Windows Update) re-execs only the backend, not win11-shell~~ Resolved 13-11 (lifecycle):
  the backend leaves a restart note and exits 75, win11-shell and every native app exec the installed build again
  (tests/gnome/scenarios/native-restart.py).
- The `shell.command` events (Task Manager toggle-search, Settings Clipboard "open") have no consumer.
- No WebKit fallback: a missing or failing win11-shell ends win11-panel, and the extension shows its St bar after 6 s.
  A `--dry-run` given to win11-panel reaches win11-shell, which ignores it.
- win11-shell is exec'd before ensure_native, so it runs outside app-win11shell.slice (its backend is inside).
- A live install keeps the old ~/.local/bin/win11-start and win11-desktop, their entries and shell.native until the
  next install.py run cleans them.
- The gx1-restart and wf-restart-copy scenarios assumed restartable WebKit hosts; Windows Update restart has no
  scenario coverage until they are ported.

## D-G72: native apps, no WebKit, no GTK 3 in the host (14-11)

The seven apps (Explorer, Settings, Task Manager, Alles, Disks, Default apps, Installer) are native GTK 4 programs
on a shared kit (native/apps, native/kit; native/APPS.md has their design, state and gaps). The launchers stay
(`win11-app --app <kind>`, `win11-explorer`, `win11-default-apps`, `win11-installer`): in their own scope they exec
`win11-gtk-<kind>`, which starts `win11-app --app <kind> --http-only` as its backend, the same API the shell uses.
The file portal shows `win11-gtk-explorer --picker-stdio` per request and runs no GTK itself. The last commit with
the HTML apps and WebKit is `6ebf169f`; native comments cite those sources as `6ebf169f:<path>`
(`git show 6ebf169f:<path>`), the shell's own HTML (gone with D-G70) as `git show 418ea594:<path>`.

Gone: the host's WebKit windows, its hand-off and window state, wry, webkit2gtk, javascriptcore, soup3 and gtk (the
host links GLib, GIO and GDK Pixbuf only), vendor/ui's HTML, JS and CSS (the wallpapers and the Settings icon stay),
the WIN11_WEBKIT_APPS fallback (a missing `win11-gtk-<kind>` exits 127 with a message that names it), and the
GTK 3 picker window of the portal. The GPU knobs of the WebKit hosts do nothing any more: `<config>/no-gpu`,
WIN11SHELL_GPU and WIN11_WEBKIT_BROWSER_CACHE. The GTK 4 programs take GSK_RENDERER=cairo unless it is set or
WIN11_SHELL_GL / WIN11_CALENDAR_GL / WIN11_APP_GL is 1, and the apps GDK_DEBUG=portals unless it is set (their
gtk::FileDialog then goes through the desktop portal, so the Win11 picker in the WinPoint session); both keep the
user's own values in WIN11SHELL_USER_GPU, which the host's launch paths restore for what they start.
tools/check-no-webkit.py is the acceptance check over the whole repository, the READMEs, notices, vendor/ and the
records included (crate patterns, lockfiles, vendor/ui, readelf and ldd of the 19 binaries, an allow list for
history, Ubuntu's update rows, fixtures and frozen i18n data; exit 2 when the binaries could not be checked).

Memory of the apps (PSS of the app with its backend, tests/gnome/scenarios/native-gtk-apps.py, private 1440x900
session, software rendering, cairo):

| App | GTK 4, final build (14-11, nl / en) | GTK 4, 13-11 | WebKit app, 13-11 |
|---|---|---|---|
| Explorer | 42.2 / 41.4 MiB | 51.7 MiB | 219.8 MiB |
| Settings | 91.0 / 91.2 MiB | 101.6 MiB | 242.1 MiB |
| Task Manager | 49.0 / 48.0 MiB | 59.1 MiB | 220.3 MiB |
| Alles | 73.0 to 143.2 MiB | | |
| Disks | 30.2 / 29.9 MiB | | |
| Default apps | 40.6 / 40.4 MiB | | |
| Installer | 28.6 / 28.7 MiB | | |

The 13-11 columns are one run with both versions, before the last kit fixes (anonymous memory plus swap: 24.9,
73.3 and 30.9 MiB native against 90.2, 112.2 and 89.0 for the WebKit apps with their web and network processes). The
results.json kept at that commit (`git show 6ebf169f:tests/gnome/scenarios/native-gtk-apps.out/results.json`) is
another run of that day: 48.2, 96.6 and 53.8 MiB native against 170.3, 192.5 and 169.7. Alles varies with how far
its index of the sandbox's / has got.

Still open: the `shell.command` events (Task Manager toggle-search, Settings Clipboard "open") have no consumer;
win11-shell runs outside app-win11shell.slice; the taskbar window's own title stays "Taakbalk" after a live switch
to English (not visible on screen); in a session without xdg-desktop-portal the apps log GDK's "Failed to read
portal settings" (harmless).
