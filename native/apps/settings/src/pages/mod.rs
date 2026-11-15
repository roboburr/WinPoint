//! # The Settings pages and their interface
//!
//! Each file here ports one 6ebf169f:vendor/ui/js/apps/settings/pages/<name>.js (screen by screen, same backend
//! calls, same texts) and exports its pages in a table:
//!
//! ```ignore
//! use super::{Builder, Content, Page, PageFuture};
//! pub const PAGES: &[(&str, Builder)] = &[("display", display)];   // every page id the module serves
//! pub const CSS: &str = "";                                         // its own GTK CSS (".st-..."), or ""
//!
//! fn display(p: Page) -> PageFuture {
//!     Box::pin(async move {
//!         let (a, b) = (p.call("display.list", json!({})), p.call("system.brightness.get", json!({})));
//!         let (a, b) = (a.await, b.await);          // started together, awaited together
//!         if !p.alive() { return Vec::new(); }      // the user went elsewhere meanwhile
//!         vec![group.widget().clone().upcast()]     // the body: groups, cards, slots, notices
//!     })
//! }
//! ```
//!
//! A page id belongs to exactly one module (main.js MODULES: network.rs serves network, wifi and ethernet;
//! time.rs time, datetime and language; accessibility.rs accessibility, textsize, visualeffects and contrast;
//! update.rs update; update_history.rs update-history; update_advanced.rs update-advanced). `MODULES` below
//! lists the tables; `builder(id)` finds a page. A page is listed in registry.rs (title, parent, glyph,
//! description, search keywords): that list is the navigation, the breadcrumb and the search, so a module never
//! registers search entries itself. A new page needs a registry.rs entry and a PAGES entry.
//!
//! ## How a page is built
//!
//! The router (app.rs) calls the builder with a fresh [`Page`] when the page is shown; it puts the page's title
//! (or "Parent › Page" breadcrumb, from the registry) in the fixed header at once, shows a spinner after 180 ms,
//! and fills the scrolling column (max 1080 px, 40 px side padding, 22 px between blocks) with the returned
//! widgets when the future resolves, with the enter animation. Groups following groups get the 28 px of
//! components.css automatically. The builder makes every widget on the main thread; awaiting `p.call(..)`
//! does not block. Long-lived handlers must check `p.alive()` (false once another page is shown) or use the
//! page-scoped helpers below, which stop by themselves.
//!
//! ## Backend
//!
//! - `p.call(method, args)` -> future of `Result<Value, kit::CallError>`; the call starts at once, so several
//!   calls made before awaiting run in parallel. `p.get(method, args)` -> `Option<Value>` (common.js get:
//!   None on error, unknown_method and unsupported quietly). `p.run(method, args)` and `p.run_with(method,
//!   args, error_title, quiet_dry)` -> `Option<Value>` (common.js run: an error toast "Dat lukte niet" with the
//!   host's message when it fails, the "Testmodus" toast for a dryRun answer). `p.spawn(async move { .. })` for
//!   work started from a click.
//! - Errors in the UI language: `p.error(&err, Some(title))` (common.js showError). Toasts: `p.toast(kit::
//!   controls::Toast::new(..))`. Dialogs: `kit::controls::Dialog::new(..).show(&p.anchor(), |r| ..)`.
//!
//! ## Settings
//!
//! - Read: `p.setting("theme.mode")` (Null when absent), `common::setting_bool(path, default)`, `setting_str`,
//!   `setting_f64`; the whole object `kit::ctx().settings()`.
//! - Write: `common::set_setting(&p, path, value, |ok| ..)` (optimistic, reverted on error, error toast).
//!   Bound controls: `common::setting_toggle(&p, path, invert)`, `common::setting_dropdown(&p, path, options,
//!   fallback)` with `options: Vec<(Value, String)>` (the value is written back as that JSON, a string or a
//!   number), `common::toggle_card(..)`, `common::dropdown_card(..)` (they follow later changes themselves).
//! - React: `p.on_settings(|settings, path| ..)` runs after every change (path None: everything may have
//!   changed) while the page is shown.
//!
//! ## Events, timers, cleanup
//!
//! - `p.on("system.audio.changed", |data| ..)`: backend SSE and extension events (prefix match), while shown.
//! - `p.timeout(ms, f)`, `p.interval(ms, f)` (stops when f returns false or the page goes), `p.on_cleanup(f)`.
//! - From a callback API to `.await`: `let (fill, wait) = super::later::<bool>(); some_dialog(.., fill);
//!   wait.await`; `super::sleep(ms).await`.
//!
//! ## Navigation, title, breadcrumb, params
//!
//! - `p.go("wifi")` / `p.go_with("bluetooth", json!({"add": 1}))` opens another page (a sub-page: its registry
//!   parent makes the breadcrumb "Netwerk en internet › Wi-Fi"; Back returns). `common::link_card(&p, id)` is
//!   the clickable card with the registry's title, description and glyph.
//! - `p.refresh()` builds the page again in place (keeps the scroll position, no animation): after a change
//!   that alters the whole page.
//! - The header is set by the router from the registry; `p.set_title(text)` replaces the title text (the last
//!   crumb on a sub-page) for a page that names itself after its data.
//! - `p.params()` is the deep link's object ({"page": "bluetooth", "add": 1} minus "page"); `p.param("q")`
//!   reads one as text; `p.take_param("add")` reads and removes a one-shot parameter, so a refresh does not
//!   act on it again.
//!
//! ## Look
//!
//! Use the kit's controls (`kit::controls::*`, SettingsGroup / SettingsCard / SettingsExpander, ToggleSwitch,
//! Dropdown, Slider (`set_value_visible(false)`: the HTML slider shows no value text), ...), the app's widgets
//! (`crate::preview::{DesktopPreview, PreviewOpts, DeviceArt}` for the mini desktop and the monitor,
//! `crate::colorpicker::pick_color` for "Een aangepaste kleur kiezen", `crate::widgets::{Wrap, Clamp}` for
//! flex-wrap rows and max-width columns, `crate::navicons::cat_icon`) and common.rs's helpers (notice,
//! unavailable_card, value_label, UsageBar, hero, hero_tile, avatar, color_name, accent_for, twin_for,
//! resolve_wallpaper_path, wallpaper_texture, os_label, copy_text, kbd, slot / set_slot, card_extra, ...).
//! Shared with home.rs, keep their signatures: bluetooth.rs `device_glyph`, `device_status`,
//! `is_not_responding`, `toggle_device`, `reset_bluetooth`, `open_add_device`; themes.rs `THEMES`,
//! `current_theme`, `apply_theme`, `theme_card`, `theme_grid`; system.rs `device_hero`, `rename_hint`.
//! Classes from css/settings.css (`st-*`, as settings.css) work in every page; a module's own CSS goes in its
//! `CSS` constant, which main.rs installs with the app sheet (GTK CSS: no grid or gap, so layout is code). Texts are `kit::t(nl, en)` with the exact wording of the page's JS (no em-dashes); a
//! language switch rebuilds the page, so texts need no relabelling of their own.
use crate::app::Router;
use crate::common;
use kit::CallError;
use kit::frame::AppWindow;
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

pub mod about;
pub mod clipboard;
pub mod display;
pub mod home;
pub mod notifications;
pub mod power;
pub mod sound;
pub mod storage;
pub mod system;

// builder B
pub mod accessibility;
pub mod accounts;
pub mod background;
pub mod bluetooth;
pub mod colors;
pub mod desktopicons;
pub mod network;
pub mod personalization;
pub mod privacy;
pub mod start;
pub mod taskbar;
pub mod themes;
pub mod time;

// builder C
pub mod apps;
pub mod defaultapps;
pub mod installed;
pub mod startup;
pub mod update;
pub mod update_advanced;
pub mod update_history;
pub mod update_release;
pub mod update_winpoint;

/// A page's body: the widgets of the scrolling column, top to bottom.
pub type Content = Vec<gtk::Widget>;
/// What a builder returns.
pub type PageFuture = Pin<Box<dyn Future<Output = Content>>>;
/// A page builder.
pub type Builder = fn(Page) -> PageFuture;

/// Every module's page table.
const MODULES: &[&[(&str, Builder)]] = &[
    home::PAGES,
    system::PAGES,
    display::PAGES,
    sound::PAGES,
    notifications::PAGES,
    power::PAGES,
    storage::PAGES,
    clipboard::PAGES,
    about::PAGES,
    bluetooth::PAGES,
    network::PAGES,
    personalization::PAGES,
    background::PAGES,
    colors::PAGES,
    themes::PAGES,
    start::PAGES,
    taskbar::PAGES,
    desktopicons::PAGES,
    accounts::PAGES,
    time::PAGES,
    accessibility::PAGES,
    privacy::PAGES,
    apps::PAGES,
    installed::PAGES,
    defaultapps::PAGES,
    startup::PAGES,
    update::PAGES,
    update_history::PAGES,
    update_advanced::PAGES,
    update_release::PAGES,
];

/// Every module's own CSS.
const MODULE_CSS: &[&str] = &[
    home::CSS,
    system::CSS,
    display::CSS,
    sound::CSS,
    notifications::CSS,
    power::CSS,
    storage::CSS,
    clipboard::CSS,
    about::CSS,
    bluetooth::CSS,
    network::CSS,
    personalization::CSS,
    background::CSS,
    colors::CSS,
    themes::CSS,
    start::CSS,
    taskbar::CSS,
    desktopicons::CSS,
    accounts::CSS,
    time::CSS,
    accessibility::CSS,
    privacy::CSS,
    apps::CSS,
    installed::CSS,
    defaultapps::CSS,
    startup::CSS,
    update::CSS,
    update_history::CSS,
    update_advanced::CSS,
    update_release::CSS,
    update_winpoint::CSS,
];

/// The builder of a page id.
pub fn builder(id: &str) -> Option<Builder> {
    MODULES.iter().flat_map(|m| m.iter()).find(|(pid, _)| *pid == id).map(|(_, b)| *b)
}

/// The pages' own CSS, appended to css/settings.css.
pub fn css() -> String {
    MODULE_CSS.iter().filter(|c| !c.is_empty()).copied().collect::<Vec<_>>().join("\n")
}

// ---------------------------------------------------------------------------------------- the waiting

struct Slot<T> {
    value: RefCell<Option<T>>,
    waker: RefCell<Option<Waker>>,
}

/// A value that arrives later on the main thread (a backend answer): await it.
pub struct Later<T>(Rc<Slot<T>>);

impl<T> Future for Later<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<T> {
        match self.0.value.borrow_mut().take() {
            Some(v) => Poll::Ready(v),
            None => {
                *self.0.waker.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// A `Later` and the function that fills it.
pub fn later<T: 'static>() -> (impl FnOnce(T) + 'static, Later<T>) {
    let slot = Rc::new(Slot { value: RefCell::new(None), waker: RefCell::new(None) });
    let s = slot.clone();
    let fill = move |v: T| {
        *s.value.borrow_mut() = Some(v);
        if let Some(w) = s.waker.borrow_mut().take() {
            w.wake();
        }
    };
    (fill, Later(slot))
}

/// A backend call that starts now; await its answer.
pub fn call(method: &str, args: Value) -> Later<Result<Value, CallError>> {
    let (fill, wait) = later();
    kit::ctx().call(method, args, fill);
    wait
}

/// Wait `ms` on the main loop.
#[allow(dead_code)] // for the pages of builders B and C
pub fn sleep(ms: u64) -> Later<()> {
    let (fill, wait) = later();
    glib::timeout_add_local_once(Duration::from_millis(ms), move || fill(()));
    wait
}

// ------------------------------------------------------------------------------------------------ Page

type SettingsFn = Rc<dyn Fn(&Value, Option<&str>)>;

pub(crate) struct PageInner {
    pub(crate) id: &'static str,
    params: RefCell<Value>,
    pub(crate) seq: u64,
    router: Weak<Router>,
    window: AppWindow,
    cleanups: RefCell<Vec<Box<dyn FnOnce()>>>,
    settings: RefCell<Vec<SettingsFn>>,
    dead: Cell<bool>,
}

/// The context of one showing of a page (main.js makeCtx). Cheap to clone.
#[derive(Clone)]
pub struct Page(pub(crate) Rc<PageInner>);

// The page interface: some methods are there for the pages of builders B and C and have no caller yet.
#[allow(dead_code)]
impl Page {
    pub(crate) fn new(id: &'static str, params: Value, seq: u64, router: Weak<Router>, window: AppWindow) -> Page {
        let params = if params.is_object() { params } else { json!({}) };
        Page(Rc::new(PageInner {
            id,
            params: RefCell::new(params),
            seq,
            router,
            window,
            cleanups: RefCell::new(Vec::new()),
            settings: RefCell::new(Vec::new()),
            dead: Cell::new(false),
        }))
    }

    /// The page id ("display", "search", ...).
    pub fn id(&self) -> &'static str {
        self.0.id
    }

    /// The registry entry (None for "search").
    pub fn info(&self) -> Option<&'static crate::registry::PageInfo> {
        crate::registry::page(self.0.id)
    }

    /// Whether this page is still the one shown.
    pub fn alive(&self) -> bool {
        !self.0.dead.get() && self.0.router.upgrade().is_some_and(|r| r.seq() == self.0.seq)
    }

    /// The deep link's parameters (an object).
    pub fn params(&self) -> Value {
        self.0.params.borrow().clone()
    }

    /// One parameter as text (numbers and booleans too).
    pub fn param(&self, key: &str) -> Option<String> {
        match &self.0.params.borrow()[key] {
            Value::String(s) => Some(s.clone()),
            Value::Null => None,
            v => Some(v.to_string()),
        }
    }

    /// Read and remove a one-shot parameter (a refresh will not see it again).
    pub fn take_param(&self, key: &str) -> Option<Value> {
        self.0.params.borrow_mut().as_object_mut().and_then(|o| o.remove(key))
    }

    /// Open another page (ctx.go).
    pub fn go(&self, id: &str) {
        self.go_with(id, json!({}));
    }

    /// Open another page with parameters.
    pub fn go_with(&self, id: &str, params: Value) {
        if let Some(r) = self.0.router.upgrade() {
            let id = id.to_owned();
            // not from inside a handler that the new page's teardown may drop
            glib::idle_add_local_once(move || r.show(&id, params, crate::app::Show::default()));
        }
    }

    /// Build this page again in place, keeping the scroll position (ctx.refresh).
    pub fn refresh(&self) {
        if !self.alive() {
            return;
        }
        if let Some(r) = self.0.router.upgrade() {
            let params = self.params();
            let id = self.0.id;
            glib::idle_add_local_once(move || r.refresh(id, params));
        }
    }

    /// Replace the header's title text (ctx.setTitle).
    pub fn set_title(&self, text: &str) {
        if self.alive()
            && let Some(r) = self.0.router.upgrade()
        {
            r.set_title_text(text);
        }
    }

    /// The window.
    pub fn window(&self) -> &AppWindow {
        &self.0.window
    }

    /// A widget in the window, for dialogs, toasts and menus.
    pub fn anchor(&self) -> gtk::Widget {
        self.0.window.window().clone().upcast()
    }

    // ---- backend

    /// A backend call; it starts now.
    pub fn call(&self, method: &str, args: Value) -> Later<Result<Value, CallError>> {
        call(method, args)
    }

    /// common.js get: the answer, or None (logged unless the method is unknown or unsupported).
    pub fn get(&self, method: &str, args: Value) -> impl Future<Output = Option<Value>> + 'static {
        let wait = call(method, args);
        let method = method.to_owned();
        async move {
            match wait.await {
                Ok(v) => Some(v),
                Err(e) => {
                    if e.code != "unknown_method" && e.code != "unsupported" {
                        eprintln!("win11-gtk-settings: {method} failed: {e}");
                    }
                    None
                }
            }
        }
    }

    /// Several calls at once, answers in the same order.
    pub fn all(&self, calls: Vec<(&str, Value)>) -> impl Future<Output = Vec<Result<Value, CallError>>> + 'static {
        let waits: Vec<_> = calls.into_iter().map(|(m, a)| call(m, a)).collect();
        async move {
            let mut out = Vec::with_capacity(waits.len());
            for w in waits {
                out.push(w.await);
            }
            out
        }
    }

    /// common.js run: the answer (`{}` for null), or None after an error toast; a dryRun answer gets the
    /// "Testmodus" toast.
    pub fn run(&self, method: &str, args: Value) -> impl Future<Output = Option<Value>> + 'static {
        self.run_with(method, args, None, false)
    }

    /// `run` with an own error title and without the dry-run toast (`quiet_dry`).
    pub fn run_with(&self, method: &str, args: Value, error_title: Option<&str>, quiet_dry: bool) -> impl Future<Output = Option<Value>> + 'static {
        let wait = call(method, args);
        let anchor = self.anchor();
        let title = error_title.map(str::to_owned);
        async move {
            match wait.await {
                Ok(v) => {
                    if !quiet_dry {
                        common::note_dry_run(&anchor, &v);
                    }
                    Some(if v.is_null() { json!({}) } else { v })
                }
                Err(e) => {
                    common::show_error(&anchor, &e, title.as_deref());
                    None
                }
            }
        }
    }

    /// Run a future on the main loop (work started from a click).
    pub fn spawn(&self, fut: impl Future<Output = ()> + 'static) {
        glib::spawn_future_local(fut);
    }

    /// An error toast (common.js showError; `title` None: "Dat lukte niet").
    pub fn error(&self, err: &CallError, title: Option<&str>) {
        common::show_error(&self.anchor(), err, title);
    }

    /// Show a toast in this window.
    pub fn toast(&self, toast: kit::controls::Toast) {
        let _ = toast.show(&self.anchor());
    }

    // ---- settings, events, timers

    /// A setting by dotted path (Null when absent).
    pub fn setting(&self, path: &str) -> Value {
        kit::ctx().setting(path)
    }

    /// Run `f(settings, path)` after every settings change while the page is shown (ctx.onSettings).
    pub fn on_settings(&self, f: impl Fn(&Value, Option<&str>) + 'static) {
        if self.alive() {
            self.0.settings.borrow_mut().push(Rc::new(f));
        }
    }

    pub(crate) fn dispatch_settings(&self, s: &Value, path: Option<&str>) {
        let list: Vec<SettingsFn> = self.0.settings.borrow().clone();
        for f in list {
            if !self.alive() {
                break;
            }
            f(s, path);
        }
    }

    /// Run `f(data)` for a backend or extension event while the page is shown (ctx.on; prefix match).
    pub fn on(&self, event: &str, f: impl Fn(&Value) + 'static) {
        let weak = Rc::downgrade(&self.0);
        let ctx = kit::ctx();
        let id = ctx.on(event, move |_, data| {
            if let Some(inner) = weak.upgrade()
                && Page(inner).alive()
            {
                f(data);
            }
        });
        self.on_cleanup(move || kit::ctx().off(id));
    }

    /// Run `f` when the page goes (ctx.onCleanup); at once when it is already gone.
    pub fn on_cleanup(&self, f: impl FnOnce() + 'static) {
        if self.alive() {
            self.0.cleanups.borrow_mut().push(Box::new(f));
        } else {
            f();
        }
    }

    /// Run `f` once after `ms` while the page is shown (ctx.timeout).
    pub fn timeout(&self, ms: u64, f: impl FnOnce() + 'static) {
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local_once(Duration::from_millis(ms), move || {
            if let Some(inner) = weak.upgrade()
                && Page(inner).alive()
            {
                f();
            }
        });
    }

    /// Run `f` every `ms` while the page is shown and `f` returns true (ctx.interval).
    pub fn interval(&self, ms: u64, f: impl Fn() -> bool + 'static) {
        let weak = Rc::downgrade(&self.0);
        glib::timeout_add_local(Duration::from_millis(ms), move || {
            let alive = weak.upgrade().is_some_and(|inner| Page(inner).alive());
            if alive && f() { glib::ControlFlow::Continue } else { glib::ControlFlow::Break }
        });
    }

    /// The router's teardown: the cleanups run, the handlers stop.
    pub(crate) fn finish(&self) {
        self.0.dead.set(true);
        self.0.settings.borrow_mut().clear();
        let list: Vec<Box<dyn FnOnce()>> = self.0.cleanups.borrow_mut().drain(..).collect();
        for f in list {
            f();
        }
    }
}
