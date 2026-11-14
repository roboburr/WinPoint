//! # win11-kit (`kit`): the shared GTK 4 toolkit of the native Win11 GNOME apps
//!
//! Explorer, Settings, Task Manager, Alles, Disks, Default apps and the Installer are built on this crate (see
//! ../APPS.md). It gives them the process setup, the backend client, the settings with live language and theme,
//! the Win11 look as GTK CSS, the Windows 11 app window, icons, the Win11 controls, a sortable table, charts,
//! formatting and the harness dev channel. Read this page first; do not re-implement what it offers.
//!
//! ## How an app uses the kit
//!
//! ```no_run
//! use kit::prelude::*;
//!
//! const SPEC: kit::AppSpec = kit::AppSpec::new("taskmgr", "io.github.win11shell.TaskManager", "Taakbeheer", "Task Manager")
//!     .icon("process")
//!     .usage("win11-gtk-taskmgr [--page processes|performance|startup|services]");
//!
//! fn main() -> glib::ExitCode {
//!     kit::run(SPEC, |ctx, args, first| {
//!         if !first {
//!             // a second launch: bring the window to the front (or open another one)
//!             if let Some(w) = kit::frame::AppWindow::active() { w.present(); }
//!             return;
//!         }
//!         kit::theme::install_app_css(".taskmgr-page { padding: 12px; }"); // an app: include_str!("../css/taskmgr.css")
//!         let win = kit::frame::AppWindow::new(kit::frame::WindowOptions { width: 900, height: 640, ..Default::default() });
//!         let table = kit::table::Table::new(vec![
//!             kit::table::Column::new("name", kit::t("Naam", "Name")).expand().icon_text("icon", kit::table::IconKind::App, 16),
//!             kit::table::Column::new("cpu", "CPU").width(80).align(kit::table::Align::End)
//!                 .format(|v| kit::format::percent(v["cpu"].as_f64().unwrap_or(0.0), 1)),
//!         ], "pid", kit::table::Selection::Multi);
//!         table.view().set_widget_name("taskmgr-processes");
//!         win.set_content(table.widget());
//!         win.present();
//!         let t = table.clone();
//!         ctx.call("sysmon.processes", serde_json::json!({}), move |r| match r {
//!             Ok(v) => t.set_rows(v["processes"].as_array().cloned().unwrap_or_default()),
//!             Err(e) => eprintln!("{e}"),
//!         });
//!         let _ = args;
//!     })
//! }
//! ```
//!
//! ## Modules and items
//!
//! - [`run`]`(spec, handler) -> glib::ExitCode`: GSK_RENDERER=cairo (unless set or WIN11_APP_GL=1; with cairo,
//!   manual font rendering), prgname and GApplication id = `spec.id`, HANDLES_COMMAND_LINE (a second launch
//!   reaches the running process); at startup the settings, the theme, the backend (`win11-app --app <kind>
//!   --http-only`, or WIN11_APP_BACKEND=port:token) and the dev channel. `handler(&Rc<Ctx>, args: Vec<String>
//!   (argv[1..]), first: bool)` runs per launch; the app makes its windows (synchronously on the first launch).
//!   `--help`/`-h` print `spec.usage`, also headless.
//! - [`AppSpec`]: `kind`, `id`, `title` (nl, en), `icon`, `usage`, `backend`; `AppSpec::new(kind, id, nl, en)`,
//!   `.icon(..)`, `.usage(..)`, `.without_backend()` (all const).
//! - Restarts: [`on_restart`]`(f(&Restart) -> RestartPlan)`: what the app does when its backend restarts
//!   (`shell.restartAll` of Windows Update: [`Restart`] `all`, `origin` (this app's backend asked for it);
//!   `shell.restart`: `all` false). [`RestartPlan`]: `Exec(args)` (the installed build again, with a new backend and
//!   the activation token; window sizes saved), `Stay` (a new backend), `Quit`. Without a hook
//!   `app::default_restart_plan`: restartAll execs with the start arguments (quits without a window), a plain
//!   restart stays. Any other end of the backend starts a new one (at most 5 in a row, with backoff); meanwhile
//!   calls wait and "bridge.status" is offline, then "backend.ready" {respawn: true} comes (re-subscribe).
//!   `app::exec_target`, `app::exec_path`: the binary a restart execs.
//! - [`ctx`]`() -> Rc<Ctx>` / [`try_ctx`]: the context (main thread). [`Ctx`]:
//!   - backend: `call(method, args, done(Result<Value, CallError>))` (off the main thread, `done` on it; waits
//!     while the backend starts; a call under way does not time out while the backend lives; untrusted .desktop
//!     launchers get bridge.js's "Deze snelkoppeling starten?"),
//!     `call_async(method, args).await`, `fire(method, args)`, `get(path, done(Result<Vec<u8>, _>))` (icons,
//!     thumbnails), `call_blocking(method, &args)`, `backend() -> Backend` (Send + Sync: `call`, `get` for
//!     worker threads), `connected()`.
//!   - events: `on(prefix, f(&str, &Value)) -> ListenerId` ("files" gets "files.changed"; "" everything): SSE
//!     events of the backend and the extension's Event signal, plus "backend.ready" and "bridge.status"
//!     {connected}; `off(id)`; `emit(event, &data)`.
//!   - extension: `gnome_call(method, args, done)`, `gnome_send(method, args)`, `gnome() -> &Gnome`
//!     (`call_sync`, `call`, `send`, `bus`).
//!   - settings: `settings() -> Value`, `setting("a.b") -> Value` (Null if absent), `set_setting(path, value)`
//!     and `set_setting_then(path, value, done)` (settings.set, optimistic, reverted on error, theme.js),
//!     `on_settings(f(&settings, Option<path>))`, `reload_settings()`.
//!   - look: `lang()` "nl"/"en", `t(nl, en)`, `dark()`, `accent()` (theme.accent hex), `text_scale()`,
//!     `high_contrast()`, `transparency()`, `reduce_motion()`, `h24()`; `app()`, `spec()`, `kind()`.
//! - [`spawn`]`(work, done)`: run `work` on a worker thread, `done(result)` on the main thread.
//! - [`CallError`] {`code`, `message`, `data`}: Display is the message (in the UI language); codes "offline",
//!   "unauthorized", "bad_response", "unavailable" (extension), "cancelled", and the host's own.
//! - [`t`]`(nl, en)`, [`lang`]`()`: the free text helpers; [`i18n`]: `en()`, `locale()`, `on_change(f)`,
//!   `bind(&widget, f)` (re-run on a language switch), `label(nl, en)` (a live gtk::Label), `tooltip(&w, nl,
//!   en)`, `list(&[..])` ("A, B en C"), `set_forced_lang(Some("en"))` (screenshots; env WIN11_LANG=nl|en).
//! - [`theme`]: `install_app_css(css)` (the app's sheet above the kit's), `look() -> Look`, `dark()`,
//!   `token(name) -> gdk::RGBA` and `token_str` (any tokens.css name, theme in effect, for cairo), `accent_rgba()`,
//!   `rgba(css)`, `palette() -> Palette`, `accent_palette(hex)`, `on_change(f)`, `track(&widget)` (queue_draw
//!   on changes), `style_window(&w)`, `normalize_hex`, `rgb`, `contrast`, `text_on`, `ACCENT_PRESETS`,
//!   `set_forced_dark(Some(bool))` / `forced_dark()` (screenshots; env WIN11_THEME=light|dark).
//! - [`frame`]: `AppWindow::new(WindowOptions)` (key, title, icon, width, height, min_width, min_height,
//!   titlebar_height, back) with `window()`, `set_title`, `title`, `set_icon`, `set_title_widget`,
//!   `set_titlebar_content`, `add_titlebar_end`, `titlebar()`, `titlebar_box()`, `set_content`, `content()`,
//!   `layer()` (overlay for dialogs, toasts), `set_back_visible`, `set_back_enabled`, `connect_back(f)` (also
//!   Alt+Left), `add_shortcut(accel, f() -> bool)`, `run_shortcut`, `shortcuts`, `present`, `close`,
//!   `toggle_maximize`, `save_size`, `dialog_open`, `kind`; `AppWindow::of(&widget)`, `all()`, `active()`.
//! - [`icons`]: `glyph(name, px) -> gtk::Image` (every icons.js name, the shell's extras, "cap-*"),
//!   `glyph_stroke`, `set_glyph`, `glyph_name`, `glyph_paintable`, `has_glyph`, `glyph_names`, `draw_glyph(cr,
//!   ..)`, `app_icon(name, px)`, `file_icon(icon or mime, px)`, `file_item_icon(&item, px, thumb)`,
//!   `can_thumb`, `thumbnail(path, px, done(Option<Texture>))`, `fetch(query, done)`, `texture(bytes)`,
//!   `severity_icon(sev, px)`, `wifi_icon`, `volume_icon`, `battery_icon`, `STROKE`.
//! - [`controls`] (see its page): `button`, `button_with_glyph`, `icon_button`, `hyperlink`, `Variant`,
//!   `DropDownButton`, `SplitButton`; `ToggleSwitch`, `checkbox`, `checkbox_desc`, `RadioGroup`; `Dropdown` /
//!   `DropdownOption`; `Slider`; `text_box`, `text_area`, `field`, `number_box`, `SearchBox` / `Suggestion`;
//!   `SettingsCard` / `Chevron`, `SettingsExpander`, `Expander`, `SettingsGroup`; `NavView` / `NavItem`;
//!   `Breadcrumb`; `InfoBar` / `Severity` (title and message as one wrapping label, the action inline when it
//!   fits, `set_action_below`); `Dialog` / `DialogResult`, `confirm`, `alert`; `MenuItem` (`glyph`, `app_icon`),
//!   `TopAction`, `show_menu_at`, `show_menu_for`, `show_menu_with_top`, `bind_context_menu`, `menu::close_all`;
//!   `ProgressBar` / `ProgressState`, `ProgressRing`; `Toast` / `ToastHandle`; `Pivot`, `Segmented`;
//!   `badge`, `avatar`, `empty_state`, `page_header`, `list_item`, `misc::list_box`, `ColorSwatches`, `clear`,
//!   `scrolled`, `label`.
//! - [`table`]: `Table::new(columns, id_key, Selection)`, `Column::new(id, title)` with `width`, `expand`,
//!   `align(Align)`, `sortable`, `resizable`, `format(f)`, `sort_by(f -> SortKey)`, `icon_text(field, IconKind,
//!   px)`, `custom(setup, bind)`, `cell_class(f)`, `tooltip(f)`; `set_rows(Vec<Value>)` (in place),
//!   `update_row`, `remove_row`, `row(id)`, `rows()`, `ids()`, `len()`, `selected_ids()`, `select_ids`,
//!   `clear_selection`, `select_all`, `connect_selection_changed`, `connect_activate`, `connect_context_menu`,
//!   `row_at`, `scroll_to`, `sort()`, `set_sort`, `connect_sort_changed`, `set_external_sort`,
//!   `set_column_visible`, `set_column_title`, `column`, `set_compact`, `set_empty_widget`, `view()`,
//!   `widget()`; `value_text`, `TableCell`, `Row`.
//! - [`chart`]: `LineChart::new(LineOptions)` (`push(&[..])`, `update(Vec<Series>)`, `set_max`, `set_color`,
//!   `set_points`, `set_options`), `Sparkline`, `Donut`, `StackedBar`, `Series`, `Segment`, `colors::*`,
//!   `resolve(color)`.
//! - [`format`]: `number`, `number_min`, `bytes`, `bytes_digits`, `rate`, `bitrate`, `percent`, `percent_of`,
//!   `duration`, `duration_long`, `uptime`, `date(ms, DateFormat)`, `time(ms, seconds, h24)`, `file_time`,
//!   `rel_time(ms, now)`, `now_ms`, `local_parts`, `month_name`, `weekday_name`, `compare_natural`.
//! - [`dev`]: the harness channel (WIN11_DEV_COMMAND / WIN11_DEV_REPORT): the boot report, and the commands
//!   "state", "click", "type", "key" and "screenshot" (every visible window rendered offscreen to PNG), each
//!   with `"app": "<kind>"` (commands without it are the shell's); `dev::run(&command)`,
//!   `dev::screenshot(&window, path)`.
//! - [`wayland`]: `set_app_id(&window, id)` (another Wayland app id for one window).
//! - [`backend`], [`gnome`]: the clients behind `Ctx` (`Backend`, `Gnome`, `backend::encode`).
//!
//! ## Conventions
//!
//! - Main thread only (GTK); long work goes through `Ctx::call` / `kit::spawn` / worker threads with a
//!   `Backend`. Never block the main thread.
//! - Control handles are cheap clones (`Rc` inside). The widget owns its control's state, so dropping a
//!   handle keeps the control working (`Dialog::new(..).show(..)` and `layout.append(ToggleSwitch::new(on)
//!   .widget())` are fine); keep a handle only to call its methods later.
//! - Handlers registered with `connect_*` run for user actions only; the setters (`set_active`, `set_value`,
//!   `select`, ...) are silent.
//! - Text the kit owns (On/Off, Search, Cancel, Close, Back, caption tooltips, ...) follows the language
//!   live; text an app passes is the app's to relabel (`i18n::bind` / `on_settings`).
//! - Give the widgets the harness needs a name (`set_widget_name("taskmgr-processes")`); the kit names its
//!   own: "<kind>-window", "<kind>-minimize" / "-maximize" / "-close" / "-back", "nav-<id>", "tab-<id>",
//!   "crumb-<id>", "w11-dialog", "w11-dialog-primary" / "-secondary" / "-close", and menu items by
//!   `MenuItem::name`.
//! - The gallery (`cargo build --example gallery`; `gallery [--light] [--en] [--section ID] [--open dialog]`)
//!   shows every control; it runs without a backend.
//!
//! ## CSS classes
//!
//! Windows: `w11`, `w11-dark`/`w11-light`, `w11-transparent`/`w11-opaque`, `w11-hc`, `w11-reduce-motion`,
//! `w11-glass` (blurred by the extension), `w11-square` (maximized, tiled), `w11-app-window`, `<kind>-window`.
//! Frame: `w11-frame`, `w11-titlebar`, `w11-appicon`, `w11-apptitle`, `w11-titlebar-center`, `w11-cap`,
//! `w11-cap-close`, `w11-back`, `w11-content` (+ `w11-layer` for the Settings-style content layer).
//! Type: `w11-caption`, `w11-body`, `w11-body-strong`, `w11-body-large`, `w11-subtitle`, `w11-title`,
//! `w11-title-large`, `w11-display`, `w11-semibold`, `w11-text-1/2/3`, `w11-text-disabled`, `w11-text-accent`,
//! `w11-text-critical`, `w11-mono`, `w11-tabular`; `w11-card`, `w11-divider`, `w11-page`, `w11-page-title`.
//! Controls: `w11-button` (+ `w11-accent`, `w11-subtle`, `w11-danger`, `w11-small`, `w11-large`),
//! `w11-icon-button`, `w11-link`, `w11-switch`, `w11-check`, `w11-radio`, `w11-entry`, `w11-searchbox`,
//! `w11-numberbox`, `w11-slider`, `w11-dropdown`, `w11-settings-card`, `w11-expander`, `w11-settings-group`,
//! `w11-nav`, `w11-nav-item`, `w11-pivot`, `w11-segmented`, `w11-breadcrumb`, `w11-infobar`, `w11-dialog`,
//! `w11-smoke`, `w11-menu`, `w11-menu-item`, `w11-popup`, `w11-list-item`, `w11-list`, `w11-list-row`,
//! `w11-toast`, `w11-badge`, `w11-empty`, `w11-progress`, `w11-ring`, `w11-table` (cells `w11-cell`,
//! `w11-col-<id>`), `w11-pill`, `w11-selected`, `w11-active`. App sheets prefix their own classes with the kind
//! (`.taskmgr-...`) and use the tokens (`var(--text-2)`, `var(--accent)`, ...).
pub mod app;
mod backdrop;
pub mod backend;
pub mod chart;
pub mod controls;
pub mod ctx;
pub mod dev;
pub mod format;
pub mod frame;
mod glyph_data;
pub mod gnome;
pub mod i18n;
pub mod icons;
mod svg;
pub mod table;
pub mod theme;
pub mod wayland;

pub use app::{AppSpec, Restart, RestartPlan, on_restart, run};
pub use backend::{Backend, CallError};
pub use ctx::{Ctx, ListenerId, ctx, spawn, try_ctx};
pub use i18n::{lang, t};

/// The common imports of an app: gtk, its prelude and glib, serde_json's Value and json!.
pub mod prelude {
    pub use gtk::prelude::*;
    pub use gtk::{gdk, gio, glib};
    pub use serde_json::{Value, json};
}

pub use gtk;
pub use serde_json;
