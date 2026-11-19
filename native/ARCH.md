# win11-shell: how the parts fit (for whoever builds a module)

One GTK 4 process (gtk4-rs 0.11, `gtk` = gtk4 crate; `gtk::gio`, `gtk::glib`, `gtk::gdk`, `gtk::cairo` re-exports)
holding the taskbar window, its flyouts, Start and the desktop. Since D-G70 (13-11) it is the only shell, and
win11-panel always execs it; since D-G72 (14-11) WinPoint has no web engine at all and the host links no GTK (the
apps are GTK 4 too, native/APPS.md). Read `notes/claude/gtk-native.md`.

## Shared API (do not change signatures; add new helpers in your own files)

- `crate::core::shell() -> Rc<Shell>` on the main thread. `Shell`:
  - `call(method, args: Value, done: FnOnce(Result<Value,String>))` backend API off the main thread;
    `fire(method, args)` when the answer does not matter; `backend.call(..)` blocking (only in spawn_blocking).
  - `backend.get("/icon?name=firefox&size=32") -> Result<Vec<u8>>` (icons, thumbnails, tray pixmaps).
  - `gnome.call(method, &args) -> Result<Value>` the GNOME extension (sync, short); `gnome.send(method, args)`.
  - `on(prefix, |event, data|)` events from both the backend stream and the extension (`windows.changed`,
    `windows.focused`, `settings.changed`, `tray.changed`, `notifications.*`, `system.audio.changed`,
    `system.network.changed`, `system.battery.changed`, `apps.changed`, `media.changed`, ...).
  - `settings()`, `setting("taskbar.pinned")`, `set_setting(path, value)`, `lang()` "nl"/"en", `t(nl, en)`,
    `dark()`.
  - `flyouts()` -> `FlyoutHost`: `register(Rc<dyn Flyout>)`, `open(name)`, `close()`, `toggle(name)`,
    `is_open(name)`, `current()`, `set_placement_x(name, x)`. A `Flyout` gives `name()`, `widget()`,
    `placement()` (BottomRight, BottomLeft, BottomCenter, BottomAt(x)), `opened()`, `closed()`.
- `crate::start::{toggle(), open_search(query)}`, `crate::flyouts::menus::{jump_list(..), taskbar_menu(..)}`,
  `crate::flyouts::power::confirm(shell, action)`.
- `crate::icons::{glyph(name, px), app_icon(icon, px), texture(bytes)}`; `crate::wayland::set_app_id(window, id)`.
- Backend methods: host/src/api-methods.json (apps.list, apps.launch {id, uris}, settings.get/set, tray.list,
  tray.activate {id, x, y}, tray.contextMenu, tray.menu/menuEvent, system.network.get/setWifi/wifiList,
  system.audio.get/set {volume|muted}, system.battery.get, system.brightness.get/set, system.bluetooth.*,
  system.power.action {action}, notifications.list/dismiss/clear/invokeAction, media.list/control,
  shell.openApp {app, params}, apps.recent, winpoint.status/check/install/cancel (WinPoint's own updater, D-G74a
  and D-G74: the check, and after a click the download and install; events winpoint.changed {rev} and
  winpoint.progress {jobId, state, done, total}), files.* ...). The HTML modules that showed how each is used and what
  the answers look like are gone since D-G70; for the reference behaviour read them from git:
  `git show 418ea594:vendor/ui/js/shell/<file>` (or `js/desktop/<file>`). Comments cite the HTML sources as
  `6ebf169f:vendor/ui/<path>` (the tag of D-G72; `git show 6ebf169f:<path>` prints one); the shell's own
  modules were removed before that tag, so their citations need 418ea594 instead.
- Extension methods (gnome.call): windows.list, windows.focus/minimize/close/restore {id}, windows.showDesktop,
  windows.setIconRects {layer, hidden, rects, monitor?} (where a minimised window shrinks to; `monitor` for a
  taskbar of another monitor), panel.geometry, panel.monitors (every monitor, the primary, whether the panel is up;
  also an event), panel.keyboard {exclusive}, panel.inputChanged (repick after a new input region),
  panel.toggleStart, panel.openTaskView, apps.activationToken,
  nativeBackdrop.configure {pid, kind, transparency, highContrast, autoHide}, nativeBackdrop.start {pid, phase},
  desktop.ready {pid}, desktop.geometry (the primary monitor, `areas`: every monitor) (extension/panelBridge.js
  `_call`).
- More than one monitor (16-11): the main taskbar and the desktop with its icons stay on the primary monitor; every
  other monitor gets a taskbar with only app buttons (panel/strips.rs, app ID
  io.github.win11shell.SecondaryTaskbar.Monitor<N>) and a wallpaper window (desktop/screens.rs,
  io.github.win11shell.SecondaryDesktop.Monitor<N>), which the extension adopts by those app IDs. Mutter keeps a
  desktop window in one monitor's work area, so one window cannot span them; the wallpaper fit "span" (Uitspreiden)
  covers the monitors' bounding box with one picture and each window shows its own part (desktop/wallpaper.rs
  `spans`). The extension keeps each taskbar's setIconRects as sent (in that taskbar's window) and places them on
  the stage again when a monitor or the primary changes (a taskbar that moves without changing size sends nothing
  new); every taskbar sends them again when the extension adopts the panel anew (after the lock screen).

## Look

GTK CSS, one sheet per module in `shell/css/` (loaded by theme.rs). Windows get classes `w11` and `w11-dark` or
`w11-light`. Palette as custom properties on `window.w11` (css/base.css): `--text-1/2/3`, `--accent`,
`--layer` (flyout/Start background), `--stroke`, `--hover`, `--press`, `--radius-control` (4px),
`--radius-overlay` (8px), and the taskbar's `--tb-*`. The WinUI values are the ones of `native/kit/css/tokens.css`
(the apps' tokens; the shell does not load the kit, so it keeps its own copy in css/base.css). The HTML originals:
`git show 6ebf169f:vendor/ui/css/tokens.css`, and for shell.css and flyouts.css (gone since D-G70)
`git show 418ea594:vendor/ui/css/shell.css` (dark and light). Sizes as in the old HTML (taskbar 48 px, buttons 40 px,
flyouts 12 px off the edges). Text in Dutch and English through
`shell.t(nl, en)`, same wording as the old HTML modules. No em-dashes in UI text.

## Rules

- Own files only (listed in your task); do not edit other modules or the shared files above.
- Build: `tools/heavy.sh --min-mb 2000 --tag <you> -- env CARGO_HOME=/run/media/robin/22TB/win11-gnome-build/cargo-home
  CARGO_TARGET_DIR=/run/media/robin/22TB/win11-gnome-build/target cargo build --offline` from the workspace
  (`native/`). Offline: no new crates. Elsewhere any CARGO_HOME / CARGO_TARGET_DIR works (README, Bouwen).
- No `unwrap()` on data from the backend or the extension; a missing service shows a calm empty state.
