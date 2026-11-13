# win11-shell: how the parts fit (for whoever builds a module)

One GTK 4 process (gtk4-rs 0.11, `gtk` = gtk4 crate; `gtk::gio`, `gtk::glib`, `gtk::gdk`, `gtk::cairo` re-exports)
holding the taskbar window, its flyouts, Start and the desktop. No WebKit. Since D-G70 (13-11) it is the only shell:
the WebKit panel, Start and desktop are gone, and win11-panel always execs win11-shell. Read `notes/claude/gtk-native.md`.

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
  shell.openApp {app, params}, apps.recent, files.* ...). The HTML modules that showed how each is used and what
  the answers look like (vendor/ui/js/shell/, vendor/ui/js/desktop/) are gone since D-G70; for the reference
  behaviour read them from git: `git show 418ea594:vendor/ui/js/shell/<file>` (or `js/desktop/<file>`).
- Extension methods (gnome.call): windows.list, windows.focus/minimize/close/restore {id}, windows.showDesktop,
  panel.geometry, panel.keyboard {exclusive}, panel.toggleStart, panel.openTaskView, apps.activationToken,
  nativeBackdrop.configure {pid, kind, transparency, highContrast, autoHide}, nativeBackdrop.start {pid, phase},
  desktop.ready {pid}, desktop.geometry (extension/panelBridge.js `_call`).

## Look

GTK CSS, one sheet per module in `shell/css/` (loaded by theme.rs). Windows get classes `w11` and `w11-dark` or
`w11-light`. Palette as custom properties on `window.w11` (css/base.css): `--text-1/2/3`, `--accent`,
`--layer` (flyout/Start background), `--stroke`, `--hover`, `--press`, `--radius-control` (4px),
`--radius-overlay` (8px), and the taskbar's `--tb-*`. Take values from vendor/ui/css/tokens.css (kept, the apps use it);
shell.css and flyouts.css are gone since D-G70: `git show 418ea594:vendor/ui/css/shell.css` (dark and light). Sizes as
in the old HTML (taskbar 48 px, buttons 40 px, flyouts 12 px off the edges). Text in Dutch and English through
`shell.t(nl, en)`, same wording as the old HTML modules. No em-dashes in UI text.

## Rules

- Own files only (listed in your task); do not edit other modules or the shared files above.
- Build: `tools/heavy.sh --min-mb 2000 --tag <you> -- env CARGO_HOME=/run/media/robin/22TB/win11-gnome-build/cargo-home
  CARGO_TARGET_DIR=/run/media/robin/22TB/win11-gnome-build/target cargo build --offline` from the workspace
  (`native/`). Offline: no new crates. Elsewhere any CARGO_HOME / CARGO_TARGET_DIR works (README, Bouwen).
- No `unwrap()` on data from the backend or the extension; a missing service shows a calm empty state.
