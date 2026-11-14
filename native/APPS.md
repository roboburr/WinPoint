# The native apps: Explorer, Settings, Task Manager, Alles, Disks, Default apps, Installer in GTK 4

Robin (13-11): "zet de apps ook om naar GTK 4". The shell (native/shell) and the Calendar (native/calendar) were
GTK 4 already; the apps were HTML in GTK 3 web views, served by the host (host/, `win11-app`, `win11-explorer`,
`win11-default-apps`, `win11-installer`). Each app was rebuilt as a native GTK 4 program with the same look, the
same behaviour and the same backend. Since D-G72 (14-11) these are the only apps: the HTML apps, the web engine
and the host's GTK 3 are gone (notes/claude/plan.md D-G72). The last commit that has them is tagged `last-webkit`.
Comments cite the HTML sources as `last-webkit:vendor/ui/<path>`; `git show last-webkit:vendor/ui/<path>` prints
one (the shell's own HTML went earlier, with D-G70: `git show 418ea594:<path>`).

## How an app starts

1. Everything that launches an app today stays as it is: desktop files, `shell.openApp`, MIME handlers and
   launch scopes run `win11-app --app <kind> [args]`, `win11-explorer [paths]`, `win11-default-apps`,
   `win11-installer`.
2. That host process, once inside its own scope (host/src/lib.rs `exec_native_app`), execs
   `win11-gtk-<kind>` from its own directory with the same arguments, minus the `--app <kind>` of win11-app.
   The pid, the launch scope and the slice stay. Without `win11-gtk-<kind>` the launcher prints
   `core.nativeAppMissing` (the path, and "reinstall WinPoint") on stderr and exits 127; a failed exec exits 1.
   There is no fallback.
3. The native app starts its backend as a child: `win11-app --app <kind> --http-only` from its own directory, with
   `WIN11_NATIVE_INSTANCE=<file>`; the backend writes `{port, token}` to that file and serves the host's services
   (the API the HTML pages used): `POST /api/call/<method>` (`{"args":...}` -> `{"ok","result"|"error"}`), header
   `X-Shell-Token`, `X-Shell-Lang`, the SSE stream `GET /api/events?token=`, and GET paths such as
   `/icon?name=..&size=..` and thumbnails. The child gets PR_SET_PDEATHSIG, so it dies with the app.
   `WIN11_APP_BACKEND=port:token` names a running backend instead (tests). Same code as native/shell/src/backend.rs.
4. The GNOME extension is reached directly over D-Bus (`io.github.win11shell.GnomeBridge`, `Call(method,
   argsJSON)`, signal `Event(name, dataJSON)`): window actions, the blur (`nativeBackdrop.configure` with the app's
   own pid and kind), `explorer.tabPosition`, activation tokens.

| kind | binary | GApplication id (= Wayland app id) | one process | HTML sources at `last-webkit` (the reference) |
|---|---|---|---|---|
| explorer | win11-gtk-explorer | io.github.win11shell.Explorer | yes, a window per launch | js/apps/explorer/, css/explorer.css, css/picker.css |
| settings | win11-gtk-settings | io.github.win11shell.Settings | yes, a relaunch navigates | js/apps/settings/, css/settings.css |
| taskmgr | win11-gtk-taskmgr | io.github.win11shell.TaskManager | yes | js/apps/taskmgr/, css/taskmgr.css |
| alles | win11-gtk-alles | io.github.win11shell.Search | yes (deep links forward) | js/apps/alles/, css/alles.css |
| disks | win11-gtk-disks | io.github.win11shell.Disks | yes | js/apps/disks/, css/disks.css |
| defaultapps | win11-gtk-defaultapps | io.github.win11shell.DefaultApps | yes | js/apps/defaultapps/, css/defaultapps.css |
| installer | win11-gtk-installer | io.github.win11shell.Installer | yes | js/apps/installer/, css/installer.css, css/software.css |

Paths are under `vendor/ui/` at the tag. Shared JS the apps were built on: `js/core/` (components, icons, i18n,
api) and `js/apps/appframe.js` (title bar, caption buttons); shared CSS: `tokens.css`, `base.css`,
`components.css`, `appframe.css` (now native/kit/css). Arguments: what the HTML app got as URL query (the host's
`run_inner` built it, `git show last-webkit:host/src/lib.rs`: `--params JSON` became query keys, `--tab-ticket T`,
or paths); each app parses the same arguments itself, and host/src/app_params.rs still validates them.
A second launch of a one-process app is handed to the running one (GApplication HANDLES_COMMAND_LINE).

## Layout

```
native/kit/                 win11-kit (lib `kit`): shared by all apps. Owned by the kit builder.
native/apps/<kind>/         win11-gtk-<kind>: one crate per app. src/main.rs, src/*.rs, css/<kind>.css.
```

The kit's API is documented in `native/kit/src/lib.rs` (module docs, one line per public item) once it is built:
read that file first. What it covers: backend client (call off the main thread, result on the main thread; events),
settings (language, dark/light, accent, transparency, high contrast, text scale; live updates), text `t(nl, en)`,
theme (Win11 tokens and controls as GTK CSS, dark and light, plus the app's own sheet), the Windows 11 app window
(32 px title bar with app icon and title, room for the app's own title bar content such as Explorer's tabs, the
three caption buttons, drag and double-click to maximise, resize edges, square corners when maximised, Mica with
the extension's blur, window size remembered), icons (Fluent-style line glyphs, app and file icons from the
backend, textures), Win11 controls (buttons, toggle switch, dropdown, slider, settings cards, navigation list,
search box, info bar, content dialog, context menu, progress, breadcrumb, a sortable table on gtk::ColumnView,
toast), the harness dev channel and the Wayland app id.

## Rules for every builder

- Own files only: the crate or files your task names. Do not edit another app, the shell, the calendar, host/,
  scripts/ or tests/ unless your task says so. The kit is read-only for app builders: put app-specific helpers
  in your own crate; if the kit has a real bug that blocks you, make the smallest fix without changing a public
  signature and say so in your report.
- Same behaviour as the HTML app at `last-webkit`: read its JS for every screen, the backend calls it makes, their
  arguments and the answers it expects, empty and error states, keyboard shortcuts and context menus. Same backend
  methods (host/src/api-methods.json lists them; the services are in host/src/services/, host/src/files/,
  host/src/apps/).
  Never invent a backend method; if the HTML app did something in the page itself, do it natively.
- Same look: port the app's CSS and the parts of components.css it uses to GTK CSS (class names prefixed with the
  kind, e.g. `.taskmgr-...`), using the kit's tokens. Dark and light. Sizes, spacing, radii and fonts as in the CSS.
- Text in Dutch and English with `t(nl, en)` and the same wording as the app's i18n.js (or the strings in its JS).
  No em-dashes in UI text. The language follows `settings.language` live.
- No `unwrap()`/`expect()` on data from the backend, the extension or files; a missing service shows a calm empty
  state, a failed action an info bar or dialog with the error text.
- Long work never blocks the main thread: backend calls go through the kit's async call; big lists use
  gtk::ListView/ColumnView with a model, not thousands of widgets.
- Give the main widgets a name (`widget.set_widget_name("taskmgr-processes")`) so the harness can find and click
  them through the dev channel; report `{"tag":"boot","kind":"native-app","app":"<kind>"}` at start (the kit does
  this when the dev channel is set).
- The dev command file is shared by the shell and every app: an app takes (and clears) only commands whose "app"
  is its own kind; the shell takes only commands without an "app" key (tests/gnome/harness.py `native_app`).
- GSK_RENDERER=cairo unless it is set or WIN11_APP_GL=1, for RAM, and GDK_DEBUG=portals unless it is set, so
  gtk::FileDialog goes through the desktop portal (the Win11 picker in the WinPoint session) instead of GTK's own
  dialog; the kit's run does both and keeps the user's own values in WIN11SHELL_USER_GPU for what the app launches.
- Build (from the repo root; one heavy job at a time on this machine, the lock in tools/heavy.sh queues you):

      tools/heavy.sh --min-mb 2000 --tag <you> -- env CARGO_HOME=/run/media/robin/22TB/win11-gnome-build/cargo-home \
        CARGO_TARGET_DIR=/run/media/robin/22TB/win11-gnome-build/target \
        cargo build --offline --manifest-path native/Cargo.toml -p win11-gtk-<kind>

  The host (host/, GLib, GIO and GDK Pixbuf; no GTK since D-G72) has its own crate cache and target on another disk:

      B=/run/media/robin/extra/win11-gnome-build
      tools/heavy.sh --min-mb 2500 --tag <you> -- env CARGO_HOME=$B/cargo-home CARGO_TARGET_DIR=$B/target \
        cargo build --release --offline --locked -j2 --bin <win11-app|win11-file-portal|...>

  Offline: only the crates already in native/Cargo.lock (gtk4 and its family, serde, serde_json, libc, futures-*).
  Build when a coherent piece is written, not after every edit; fix every warning in your own files.
- Do not start a GNOME session or the test harness; the integration step tests all apps together. A smoke run
  without a display is fine (`win11-gtk-<kind> --help`).
- Comments in English, like the rest of native/. Match the shell's and the calendar's code style.

## Report

End with: what works (screens, actions), what is missing or different from the HTML app, kit fixes you made,
and anything the integration step must know (arguments, files, environment).

## State

### Native only (D-G72, 14-11)

- The HTML apps (index.html, js/ and css/ under vendor/ui), the host's web view windows and its GTK 3 are gone;
  no binary links a web engine any more (the acceptance check of D-G72 in tools/ looks at the tree, the lockfiles
  and the 19 binaries). vendor/ui keeps the wallpapers and the Settings icon, which the backend
  serves under /ui/assets/.
- A launcher without its `win11-gtk-<kind>` exits 127 with `core.nativeAppMissing`; the environment switch back to
  the HTML apps is gone, and so is the portal's GTK 3 picker window (the portal runs `win11-gtk-explorer
  --picker-stdio` only).
- `shell.pickFile` is gone from the backend; Settings and Task Manager use gtk::FileDialog. The kit sets
  GDK_DEBUG=portals (see the rules above): an unsandboxed GTK 4 app otherwise shows GTK's own dialog in-process,
  never the Win11 picker.
- The picker's title follows the HTML picker's rules (trimmed, at most 200 characters, else the mode's title;
  picker.rs `titles_follow_the_mode_and_the_caller_s_own_title`). A non-modal picker gets no taskbar button of
  its own (the extension hides a FilePicker window with a transient parent from the window list).
- Explorer accepts only its own tab type in drops; the plain-text tab envelope only an HTML Explorer made is gone.
  `window-state.json` stays where it was, so the tabs an earlier version saved come back.
- Verified in the harness (nl, most also in en): native-gtk-apps (105 checks, with each app backend's environment),
  native-shell, calendar-app, session-mode, r2d-picker, frosted-glass, native-apps, r2b-disks-race, r3-ontop,
  ge1c-extension, native-restart, taskbar-app-icons, kit-ui, snap-accent, launch-attention and readme-shots.
- Memory (PSS of the app with its backend, native-gtk-apps, private 1440x900 session, cairo renderer; 14-11):
  Explorer 42.2 MiB, Settings 91.0, Task Manager 49.0, Alles 73 to 143 (its index), Disks 30.2, Default apps
  40.6, Installer 28.6. On 13-11 one run with both versions gave Explorer 51.7 against 219.8 MiB for the HTML app,
  Settings 101.6 against 242.1 and Task Manager 59.1 against 220.3 (the HTML app with its web and network process).

### Integration of 13-11

All seven apps are ported and build in release with the rest of the native workspace, with
no warnings in native/kit or native/apps (`win11-shell` still has 19 dead-code warnings of its own). Every app was
checked headless by its builder (Broadway, a private session bus, private XDG dirs, a mock or `--dry-run` backend,
screenshots through the dev channel); none had run in a GNOME session or under tests/gnome/harness.py yet
(tests/gnome/scenarios/native-gtk-apps.py is the scenario for that; the second round and D-G72 ran them there).
Popover menus, drag and drop and pointer hover could not be exercised headless in any app.

### Build, install

- `scripts/build.sh` builds the native workspace (shell, calendar, kit and the seven apps) into the host's target
  directory, so target/release holds all 19 binaries. `scripts/install.py` installs them (`BINS`, `NATIVE_BINS`,
  `WIN11_NATIVE_BUILD_DIR` first); `scripts/uninstall.py` removes them and `scripts/package.py` ships them through
  the same `BINS`. The harness's `BINS_FALLBACK` and `NATIVE_BINS` know them too.
- Launch stays as it was: `win11-app --app <kind>`, `win11-explorer`, `win11-default-apps`, `win11-installer`. Once
  in its unit the host execs `win11-gtk-<kind>` from its own directory (host/src/lib.rs `exec_native_app`).
- Until D-G72 an environment switch in the launcher's environment, or a missing `win11-gtk-*` binary, fell back to
  the HTML app, and the portal's file picker followed it. Since then there is no fallback (see above).
- Smoke (13-11): every binary prints its usage with `--help` without a display and, with no display at all
  (private runtime dir, private bus), ends with exit 1 and GTK's "Failed to open display", leaving no backend
  behind. `win11-app --app taskmgr --help` reaches the native app.
  `cargo test` on the native workspace: kit doctest, Disks 3 and Explorer 7 unit tests pass.

### Fixed at integration

- kit backend.rs: the runtime folder `$XDG_RUNTIME_DIR/win11-gnome` is made 0700. Made with the umask's mode (when
  an app started before anything else made it) every backend refused to start ("De runtime-map moet privé zijn").
  Alles's zip glyph (alles art.rs) writes into the same folder and makes it the same way.
- kit backend.rs: `--dry-run` on the app's command line goes on to its backend (it was dropped, so
  `win11-app --app <kind> --dry-run` ran a live backend).
- kit ctx.rs `dispatch`: on the main thread the event is queued instead of run at once. With `WIN11_APP_BACKEND`,
  "backend.ready" came before the context existed, so the settings were never fetched from the backend.
- kit lib.rs: the module example compiles as a doctest.
- host lib.rs `run`: the backend of a native app (`--http-only` with `WIN11_NATIVE_INSTANCE`, not the panel) is not
  moved into a unit of its own. Started outside its launcher, the app saw its backend exit at once and the moved
  copy ran on without it.
- Settings › Achtergrond "Foto's zoeken" and Task Manager › Opslag › Verkennen "Map kiezen" used `shell.pickFile`,
  which a `--http-only` backend answers with `{path: null}`, so both did nothing. They use `gtk::FileDialog` now,
  as the Installer, Alles and the Settings import do (the desktop's FileChooser portal: Win11's picker when
  winpoint-portals.conf chooses it; since D-G72 the kit's GDK_DEBUG=portals makes GTK use that portal).
- Explorer: revealing an item in Details view no longer scrolls sideways (with the details pane open the Naam
  column went out of view); the command bar and navigation pane separators have a CSS size, so they draw.
- Settings: the unused page `placeholder` is gone.
- Host fixes by the builders, kept: services/index.rs starts the index worker from one long-lived thread
  (PR_SET_PDEATHSIG fired when the short-lived request thread ended, so the worker died after every answer; the
  HTML Alles had the same bug); picker.rs `show_native` and portal.rs (the native picker).

### Per app

**Explorer** (`win11-gtk-explorer`, one process, a window per launch)
- Has: title-bar tabs (keyboard, middle-click, drag to reorder, to another window or out to a new window, with the
  backend's tab tickets for tabs from other processes), session restore in window-state.json, address row
  (history menus, breadcrumb with chevrons and overflow, edit mode with suggestions), live search filter and Enter
  to Alles, the command bar enabled as in toolbar.js, the navigation pane (lazy tree, splitter), all eight view
  modes with grouping, check boxes, thumbnails, Windows selection rules, rubber band, type-to-select, inline
  rename, Start / Deze pc / Galerij, every context menu and main.js shortcut, live `files.*` updates; all file
  operations with undo, progress cards and the trash; the properties, drive, trash, share and folder-options
  dialogs; the details and preview panes; drag and drop in and out; the portal file picker (`--picker-stdio`).
- Missing or different: no tab, content or Galerij animations, sticky month headers, hover zoom or drop shadows;
  no menu on a group header; nearest kit glyphs in the View menu; Shift+Delete goes to the trash (as in the HTML
  app: the host has no permanent delete); the explorer.ready backlog handshake is not ported (relaunches come
  through GApplication); a torn-off tab opens where mutter places it; an `x11:` picker parent is ignored; a picker
  that fails after it started answers "cancelled"; Escape in a picker with a menu open closes only the menu.
- Untested: drag and drop of files and tabs, the progress card on a slow job, the picker parented to a real
  Wayland caller, and whether the extension's `explorer.tabPosition` finds native windows.

**Settings** (`win11-gtk-settings`, one process, a relaunch navigates)
- Has: the frame (title bar with Back, user card, search with suggestions and a results page, the 11 categories,
  breadcrumb, Back history) and every page of the HTML app: Start, Systeem, Beeldscherm (with the 15 s keep
  dialog), Geluid, Meldingen, Energie, Opslag, Klembord, Info, Bluetooth with the add-device dialog and pairing,
  Netwerk, Wi-Fi, Ethernet, Persoonlijke instellingen, Achtergrond, Kleuren with the custom colour picker,
  Thema's, Start, Taakbalk with the live mini taskbar and tray icons, Bureaubladpictogrammen, Toegankelijkheid
  (Tekstgrootte, Visuele effecten, Contrastthema's), Privacy, Accounts, Tijd en taal (Datum en tijd, Taal en
  regio), Apps, Geïnstalleerde apps (with the reviewed uninstall), Standaard-apps, Opstarten, Windows Update with
  every hero state, history, advanced options and the Ubuntu release card, and settings backup.
- Missing or different: the last page of a session is not restored; recent pages, photos and colours live in
  `settings-app.json`; the mini desktop and taskbar are a tint without blur; sliders show no drag tooltip; the
  narrow-window rules of settings.css (a card's control under its text) are not ported; export writes into the XDG
  Downloads folder;
  import and "Foto's zoeken" use the portal's file dialog.
- Untested against the real host: every page ran on a canned mock backend only; the prompt change on Update,
  picking a default app, the startup switch and the job dialog were not exercised.

**Task Manager** (`win11-gtk-taskmgr`, one process)
- Has: navigation pane (compact below 1000 px), title-bar search, per-page command bars; Processen (groups,
  expandable app rows, heat cells, totals, status, View menu), Details (multi-select), Services (start, stop,
  restart, confirmed), Opstart-apps, Instellingen, Prestaties (device list with mini graphs, CPU per core, memory,
  disks, network, GPU), Samenvatting (health, fixes, tiles, top apps, disks), Opslag (volume cards, Verkennen with
  the sunburst, Categorieën, Grote bestanden, Opruimen); end task with force, restart, efficiency mode, properties;
  the "Win11 Shell reageert niet" banner; deep links; live language. The `open=end|restart|efficiency` test helpers
  always ask first.
- Missing or different: no priority or affinity (no backend method, as in the HTML app); a stuck shell is restarted
  through the backend only; column minimum widths and the Settings page's 1000 px maximum are not applied; group
  heading rows can show a hover highlight and may flicker once; Task Manager's own icons in menus and toasts are
  swapped in after they open (unverified); the Opruimen footer is an overlay; the sunburst tooltip is a GTK tooltip
  and hover does not fade; the Weergave menu opens start-aligned; Kopiëren copies the visible figures only; the
  Samenvatting tiles use the kit's 30-point Sparkline (the HTML has 60); after a language switch an old Alt+Up
  shortcut and theme listener stay registered (they hold the page weakly and do nothing).
- Untested: menus, dialogs, the trash and clean-up runs, sunburst hover.

**Alles** (`win11-gtk-alles`, one process, deep links forward)
- Has: the title bar with the big search box, the command bar (kind filter, the four match toggles saved in
  `alles.*`, Sorteren, Zoektips, Voorbeeld, Opties), a lazy result table (pages of 200 on a gio::ListModel,
  tested with 30,000 results), match highlighting, sortable and resizable columns remembered in
  `native-alles.json`, welcome / first-run / nothing-found / error overlays, the status bar with the index state,
  preview and search-tips panes, the context menu, copy path or name, properties, trash, the Opties dialog
  (locations, drives, exclusions, hidden files, live updates, rebuild), every keyboard shortcut, deep links.
- Missing or different: a language switch relabels in place (the HTML app reloaded); the sort arrow sits right of the
  header text; Grootte stays left-aligned; a double-click on a column edge does not reset its width; row inset and
  focus ring are drawn on the cells; the kind drop-down shows the kind's glyph; no entrance animations; the
  welcome sits 64 px from the top; "Map toevoegen / uitsluiten" uses the portal's file dialog, with a typed-path
  dialog as fallback; a second launch with a bare folder also searches it.
- Untested: two Alles backends sharing one config and cache folder (seen once in a test setup: one exited).

**Disks** (`win11-gtk-disks`, one process)
- Has: the overview (disk cards with proportional partition bars, MBR extended frames, volume and free-space rows,
  legend), keyboard selection and menus, loading / unreachable / unsupported / empty states with the dry-run and
  read-only notices, the volume and disk pages (SMART health with the attribute table), the label, format (LUKS),
  delete, new volume, unlock and initialise dialogs with their checks and job progress, mount, unmount, lock, safe
  remove, eject, open in Explorer, refresh on `disks.changed`, a 30 s poll, focus and F5, deep links.
- Missing or different: no resize (none in the HTML app either); the "…" menu opens below its button; updates wait
  while a menu or dialog is open (a re-render would close it); the 760/680 px breakpoints re-render; the slider's
  value is a tooltip; legend and bar captions are accessible labels; dialogs have no entrance animation.
- Untested: everything against the real `win11-app` backend (it ran on the HTML app's disks mock).

**Default apps** (`win11-gtk-defaultapps`, one process)
- Has: search, 100 file types per page with paging, loading / empty / error states, the "Een app kiezen" dialog
  with `apps.forType` / `apps.setDefaultForType` and the success, override and failure answers, reload on
  `apps.changed` and on focus, live language, the narrow layouts, deep links on relaunch.
- Missing or different: GTK radio rows with the kit's look; the card container query (control under the text at
  narrow card widths) is not done; no dialog or toast animations; the window opens without the search box focused.
  MIME type names come from the backend in `settings.language`, so `WIN11_LANG=en` screenshots show Dutch names.
- Untested: actually setting a default, reload on focus, `apps.changed`.

**Installer** (`win11-gtk-installer`, one process)
- Has: the drop zone, `software.inspect` with the app card and details, removal review (`software.uninstallInfo`),
  the review dialog (`software.prepare`, only its own button runs `software.execute`), the job view polling
  `software.status`, Annuleren, the Activity list, drop of one file on the window, relaunch with a new file,
  live language, the narrow layout.
- Missing or different: "Bestand kiezen" is the portal's file dialog; the drop uses a gtk::DropTarget.
- Untested: real installs and removals (dry run only), Annuleren on a running job, the file dialog, drag and drop.

### Second round (13-11): lifecycle, kit UI, taskbar icons, Task Manager

- Lifecycle: a native backend never re-execs itself; on `shell.restartAll` it leaves `<instance>.restart` and exits
  with 75, and the GUI execs the installed build again with its windows (Explorer with its tabs, the Settings that
  asked on Update en herstel); any other end of the backend starts a new one (backoff, at most 5 a minute). Calls
  no longer time out while the backend lives. `kit::on_restart` / `Restart` / `RestartPlan`. An exec that fails
  keeps the program on a new backend and takes its application id back (exec_self released it), and ignores
  SIGPIPE again (kit app.rs and native/shell/src/main.rs `after_failed_exec`).
- Settings' update keep-alive (keepalive.rs) tells its backend whether a window is shown (`update.setWatching`):
  the event stream stays open without a window, so the job's "installed"/"failed" notification (host pk.rs
  `notify`) asks `ServiceContext::watching` instead of the subscriber count.
- Kit UI: one wrapping InfoBar label with the action inline when it fits (`set_action_below`), app icons in menus
  (`MenuItem::app_icon`, Explorer's Openen met), Explorer's column headers over their columns and a resize that
  follows the pointer, no stray header pixels in Alles.
- Taskbar and Task View show each app's own desktop entry icon; the host's /icon route falls back to the installed
  Win11 icons for names the theme lacks. The entries scripts/install.py writes have a file of the bundled Win11
  icons as Icon= (`BUNDLED_ICONS`), so GNOME's Alt+Tab and overview find them under Adwaita too. The app catalogue
  (host apps/catalog.rs) gives its own apps (`win11shell-<kind>.desktop`) the icon of their installed entry
  (io.github.win11shell.<Name>.desktop, host launch.rs `OWN_APP_IDS`) and names it in `entry`, which the taskbar
  reads; our entries themselves are not listed, so Start's All apps and search show each own app once.
- Task Manager (host sysmon): every native app is one Apps row with its window, its backend and their children (the
  walk up from a helper stops at a process with a window of its own: an app opened from Explorer without a systemd
  unit of its own stays its own row); the portal's file picker is never Explorer. Task Manager's own backend is the host and stays the background row
  "Win11 Shell", as the HTML Task Manager's own process was.
- The dev channel's click, type and key take a visible widget of that name first (the frame's hidden title-bar
  Back is also called "<kind>-back", so "explorer-back" reached it instead of Explorer's Back).
- Verified in the harness: native-gtk-apps.py (launch, memory, close and a short use of every app through the dev
  channel, with a UDisks2 stand-in for Disks, and the Processen rows), native-restart.py, taskbar-app-icons.py,
  kit-ui.py, native-shell.py, calendar-app.py and session-mode.py.

### Known gaps (all apps)

- Kit: dialogs, toasts and menus have no entrance animation; drop-downs and menus open below or start-aligned;
  `Dialog::width()` is a minimum only (Default apps and Installer wrap the body in their own clamp); `CallError`
  carries the host error's `key` (13-11; Bluetooth's "not responding" and the update page's refusals match on it);
  `Sparkline` is fixed at 30 points;
  `SettingsCard` has one description label (Default apps appends its MIME line through the kit's widget tree).
- GSK's cairo renderer with GTK's automatic font rendering inks a glyph's top pixel row outside its extents (lost
  i-dots in small text, a row left behind where text moved); `kit::run` therefore sets manual font rendering when
  the renderer is cairo. The shell (native/shell) does not use the kit and keeps GTK's default.
- GTK 4.22: changing CSS classes on a ColumnView row while a cell is being bound crashes when rows are added to a
  visible table; Task Manager defers it. A margin on a ColumnView row prints "Trying to measure
  GtkColumnViewRowWidget" warnings on sideways scrolling (the kit's table too).
- Headless testing: Broadway's socket path must stay under 108 bytes; the dev `screenshot` command sometimes says
  "nothing drawn" right after a change (retry) and does not capture popovers; CSS transitions do not advance
  without frames.
- `shell.pickFile` is gone (D-G72; a `--http-only` backend only ever answered `{path: null}`); the apps use
  `gtk::FileDialog` through the portal (or could start `win11-gtk-explorer --picker-stdio` themselves).
