//! Task Manager's pages and the interface every page implements (main.js: `createPage(ctx)` returning `{id,
//! title, el, commands, mount, unmount, navigate, onSearch, refresh}`).
//!
//! # How a page is built
//!
//! `create(id, &app)` makes a page once, the first time it is shown, and keeps it until the language changes
//! (then every page is made again, like main.js `instances.clear()`). A page is an `Rc<dyn Page>`; its widgets
//! hold their own state, and signal closures hold the page's state weakly (`Rc::downgrade`) so a page that is
//! thrown away really goes. main.rs wraps the page:
//!
//! ```text
//! taskmgr-page taskmgr-page-<id>      (vertical box, fills the content layer)
//!   taskmgr-page-header               title (`title()`, 20 px semibold) and the command bar (`commands()`)
//!   taskmgr-page-body                 `widget()`: as it is when `fill()` (tables, panes that scroll inside),
//!                                     else in a vertical ScrolledWindow (the page scrolls as a whole)
//! ```
//!
//! Above every page the content layer shows the "Win11 Shell reageert niet" banner (hostbanner.rs) while the host
//! does not answer.
//!
//! # The lifecycle
//!
//! - `mount(params)`: the page is shown. Subscribe to the store here and keep the `store::Subscription`s in the
//!   page (a `RefCell<Vec<Subscription>>`); `params` are the deep-link params (a JSON object, main.rs
//!   `norm_params`: `pid`, `sort`, `device`, `volume`, `path`, `tab`, ...).
//! - `unmount()`: another page is shown: drop the subscriptions (that stops the polling nobody needs) and any
//!   timers of your own.
//! - `navigate(params)`: a deep link to the page that is already shown (select a device, a volume, a pid).
//! - `searchable()` / `on_search(query)`: pages that filter by the title bar's search box return true; on a
//!   page that does not search, typing in the box goes to Processen (main.js `onSearch`). `on_search` also runs
//!   right after `mount` with the current query (`app.query()`).
//! - `refresh()`: F5. Return true when the page refreshed itself; false lets main.rs refresh every store channel
//!   that has subscribers.
//!
//! # Shared data: the store
//!
//! `app.store` (store.rs) polls the backend only while a page is subscribed: `app.store.subscribe(channel,
//! |data: &Rc<Value>| ..)` delivers the last data at once (when there is some) and after every poll;
//! `subscribe_with_error` also gets failed polls. Channels: "processes" (sysmon.processes), "performance"
//! (sysmon.performance), "health" (sysmon.health), "volumes" (storage.volumes; storage.volumesChanged events
//! are injected), "startup" (sysmon.startup), "services" (sysmon.services). Realtime channels poll at
//! taskmgr.updateSpeed (`store.speed_ms()`, 0 = paused, `store.paused()`, `store.on_speed(|ms, key| ..)`), slower
//! while the host does not answer; nothing polls while the window is hidden. `store.refresh(ch)` /
//! `refresh_then(ch, |data| ..)` poll now, `store.get(ch)` is the last data, `store.fetch(ch, max_age, f)` once.
//! `store.history()` is the performance history the graphs read: `t`, `cpu`, `cpu_cores[i]`, `freq`, `mem`,
//! `mem_used`, `disks[name] {active, read, write}`, `nets[iface] {rx, tx}`, `gpus[i] {usage, mem}` (VecDeques,
//! oldest first, at most store::MAX_HISTORY; `store::tail(&series, n)` for the last n), `count`.
//!
//! # Calls, events, settings
//!
//! - Backend calls go through `hostcall::call(method, args, |result| ..)` (or `call_with` and `Opts {timeout,
//!   source, health}`): every call has a timeout (`hostcall::timeout_of`) and counts for the host's health; the
//!   result arrives on the main thread. Errors are `kit::CallError` (`e.message` is the text to show;
//!   `hostcall::is_transport(&e)`: the host did not answer).
//! - Events of the backend (storage.scanProgress, storage.categoriesReady, ...): `let sub = app.on(prefix, |name,
//!   data| ..)`; keep `sub` (an `app::EventSub`), dropping it stops listening. Host health:
//!   `app.on_health(|state, info| ..)` (an `app::HealthSub`), `hostcall::health().state()`.
//! - Settings: `app.setting("taskmgr.graphSeconds")`, `app.set_setting(path, value, revert)` (a toast and
//!   `revert()` on failure), `app.on_settings(|settings, path| ..)` (an `app::SettingsSub`). Per-viewer
//!   conveniences (what the WebKit app kept in localStorage): `util::load(key)`, `util::save(key, value)`.
//! - The shell's info: `app.user()`, `app.dry_run()`, `app.info()` (event "taskmgr.info" when it arrives).
//!
//! # Dialogs, toasts, menus, navigation
//!
//! - Dialogs: `kit::controls::Dialog::new(title).body(&w).primary(..).close_label(..).danger().width(460)
//!   .show(&app.parent(), |result| ..)`; `app.alert_error(title, message, done)`.
//! - Toasts: `app.toast(kit::controls::Toast::new(..))`, `app.toast_error(title, message)`,
//!   `app.toast_success(title, ms)`, `app.toast_info(title, message)`.
//! - Menus: `kit::controls::show_menu_for(&button, items)` / `show_menu_at(&w, x, y, items)`; Task Manager's
//!   own glyphs (leaf, end task, stop) in kit menus with `util::fix_menu_glyphs(&anchor, names)`.
//! - Another page: `app.navigate("storage", json!({"volume": id}))`; "Nieuwe taak": `app.new_task()`.
//!
//! # The actions that exist
//!
//! actions.rs (process rows, `actions::ProcRow`): `end_task`, `restart_task`, `restart_shell`,
//! `set_efficiency`, `open_file_location`, `search_online`, `show_properties`, `process_menu` (the context
//! menu), `restart_target_of`, `target_of`, `combine`, `shell_pid`, `norm_status`. The backend has no method
//! for priority or affinity, so Task Manager offers neither (as the WebKit app).
//!
//! # Shared helpers
//!
//! util.rs: `cpu_pct`, `mem_mb`, `disk_mbs`, `vol_pct`, `ghz`, `pct_int`, `kind_color(kind)` (graph token, accent
//! when taskmgr.colorKeyedGraphs is off), `category_color`, `ring_color`, `chart_color`, `app_color` (the
//! `--tm-*` tokens of css/taskmgr.css for cairo), `heat_bucket`, the glyphs (`icon(TmGlyph::StatusOk, 56)`,
//! `draw_tm_glyph`), `cmd_button(Icon, label)`, `cmd_sep()`, `tip_wrap`, `set_tip`, `pretty_path`, `dir_name`,
//! `base_name`, `fill(template, &[("name", v)])`, `plural(n, one, other)`. table.rs (`TmTable`): the grid with
//! groups, tree rows, heat cells, header totals and remembered columns.
//!
//! # CSS
//!
//! css/taskmgr.css holds the shell, the grid, dialogs and the shared classes (`taskmgr-card`, `taskmgr-section`,
//! `taskmgr-pill`, `taskmgr-bar`, the `--tm-*` tokens); css/performance.css, css/summary.css and css/storage.css
//! are the pages' own sheets (main.rs installs them all after the kit's).
use crate::app::App;
use kit::prelude::*;
use std::rc::Rc;

pub mod details;
pub mod performance;
pub mod processes;
pub mod services;
pub mod settings;
pub mod startup;
pub mod storage;
pub mod summary;

/// The pages in their navigation order (settings in the footer).
pub const PAGE_IDS: [&str; 8] = ["summary", "processes", "performance", "storage", "startup", "services", "details", "settings"];

/// A Task Manager page (see the module docs).
pub trait Page {
    /// The page id (PAGE_IDS).
    fn id(&self) -> &'static str;
    /// The page title in the current language (header and window title).
    fn title(&self) -> String;
    /// The page body.
    fn widget(&self) -> gtk::Widget;
    /// The command bar's widgets (made once, with the page).
    fn commands(&self) -> Vec<gtk::Widget> {
        Vec::new()
    }
    /// The body fills the page and scrolls inside (tables); false: the page scrolls as a whole.
    fn fill(&self) -> bool {
        false
    }
    /// Shown, with the deep-link params.
    fn mount(&self, _params: &Value) {}
    /// Hidden.
    fn unmount(&self) {}
    /// A deep link while the page is shown.
    fn navigate(&self, _params: &Value) {}
    /// The page filters by the search box.
    fn searchable(&self) -> bool {
        false
    }
    fn on_search(&self, _query: &str) {}
    /// F5; false: refresh the store's channels in use.
    fn refresh(&self) -> bool {
        false
    }
}

/// Make a page (None: an unknown id).
pub fn create(id: &str, app: &Rc<App>) -> Option<Rc<dyn Page>> {
    Some(match id {
        "summary" => summary::create(app),
        "processes" => processes::create(app),
        "performance" => performance::create(app),
        "storage" => storage::create(app),
        "startup" => startup::create(app),
        "services" => services::create(app),
        "details" => details::create(app),
        "settings" => settings::create(app),
        _ => return None,
    })
}

/// A page's title by id (navigation labels).
pub fn title_of(id: &str) -> &'static str {
    let (nl, en) = match id {
        "summary" => ("Samenvatting", "Summary"),
        "processes" => ("Processen", "Processes"),
        "performance" => ("Prestaties", "Performance"),
        "storage" => ("Opslag", "Storage"),
        "startup" => ("Opstart-apps", "Startup apps"),
        "services" => ("Services", "Services"),
        "details" => ("Details", "Details"),
        "settings" => ("Instellingen", "Settings"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

/// The "Nieuwe taak" command (Samenvatting, Prestaties, Processen, Details, Opstart-apps).
pub fn new_task_button(app: &Rc<App>) -> gtk::Button {
    let b = crate::util::cmd_button(crate::util::Icon::Glyph("new"), kit::t("Nieuwe taak", "Run new task"));
    b.set_widget_name("taskmgr-new-task");
    let a = Rc::downgrade(app);
    b.connect_clicked(move |_| {
        if let Some(a) = a.upgrade() {
            a.new_task();
        }
    });
    b
}

/// main.js's state for a page that could not be loaded (also what a stub page shows until it is built).
#[allow(dead_code)] // every page is built now; kept for pages under construction
pub fn placeholder() -> gtk::Widget {
    let e = kit::controls::empty_state(
        "warning",
        kit::t("Deze pagina kon niet worden geladen", "This page couldn\u{2019}t be loaded"),
        kit::t(
            "Probeer het opnieuw. Helpt dat niet, sluit Taakbeheer dan en open het opnieuw.",
            "Try again. If that doesn\u{2019}t help, close Task Manager and open it again.",
        ),
        None,
    );
    e.set_vexpand(true);
    e.upcast()
}
