//! Win11 GNOME Task Manager in GTK 4 (../../APPS.md; the WebKit app: vendor/ui/js/apps/taskmgr/main.js).
//!
//! The Win11 22H2-style Task Manager: a navigation pane with the hamburger (compact below 1000 px or by
//! choice), the search box in the title bar, the pages with their command bar (Samenvatting, Processen,
//! Prestaties, Opslag, Opstart-apps, Services, Details, Instellingen; made on first use), the "Win11 Shell
//! reageert niet" banner above them, deep links, and the keys Ctrl+F / Ctrl+E (search), Esc (clear the search),
//! F5 (refresh) and Ctrl+Tab / Ctrl+Shift+Tab (next / previous page).
//!
//! Arguments (as win11-app passes them, after --app taskmgr): `--params JSON` with the WebKit app's query
//! keys (page = summary|processes|performance|storage|startup|services|details|settings and friendly aliases,
//! pid, sort, device, volume, path, tab|section, open), or `--page ID`. A second launch is handed to the running
//! process: it brings the window to the front and follows the deep link (main.js app.navigate).
mod actions;
mod app;
mod hostbanner;
mod hostcall;
mod pages;
mod store;
mod table;
mod util;

use app::App;
use kit::controls::{NavItem, NavView, SearchBox};
use kit::frame::{AppWindow, WindowOptions};
use kit::prelude::*;
use pages::{PAGE_IDS, Page};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

const SPEC: kit::AppSpec = kit::AppSpec::new("taskmgr", "io.github.win11shell.TaskManager", "Taakbeheer", "Task Manager").icon("chart").usage(
    "win11-gtk-taskmgr [--params JSON] [--page ID]\n\
     \n\
     Win11 GNOME Task Manager (GTK 4).\n\
     \x20 --params JSON   deep link: {\"page\": \"processes\", \"pid\": 1234, \"sort\": \"cpu\"}, {\"page\": \"storage\", \"tab\": \"cleanup\"}, ...\n\
     \x20 --page ID       summary | processes | performance | storage | startup | services | details | settings\n\
     A second launch brings the running window to the front and follows its deep link.",
);

const CSS: &str = include_str!("../css/taskmgr.css");
const CSS_PERFORMANCE: &str = include_str!("../css/performance.css");
const CSS_SUMMARY: &str = include_str!("../css/summary.css");
const CSS_STORAGE: &str = include_str!("../css/storage.css");

/// Below this window width the navigation pane is compact (main.js).
const COMPACT_BELOW: i32 = 1000;
/// Command bar labels hide when the page is this narrow (taskmgr.css container queries).
const NARROW_PAGE: i32 = 820;

fn main() -> glib::ExitCode {
    kit::run(SPEC, |_ctx, args, first| {
        let params = norm_params(parse_args(&args));
        if !first {
            if let Some(ui) = UI.with(|u| u.borrow().clone()) {
                ui.app.window.present();
                if let Some(page) = params["page"].as_str().map(str::to_owned) {
                    ui.go(&page, params.clone());
                }
            }
            return;
        }
        build(params);
    })
}

// ------------------------------------------------------------------------------------------------ deep links

fn parse_args(args: &[String]) -> Value {
    let mut out = json!({});
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--params" => {
                if let Some(raw) = args.get(i + 1).filter(|r| r.len() <= 64 * 1024)
                    && let Ok(v) = serde_json::from_str::<Value>(raw)
                    && let Some(o) = v.as_object()
                {
                    for (k, v) in o {
                        out[k.as_str()] = v.clone();
                    }
                }
                i += 2;
            }
            "--page" => {
                if let Some(p) = args.get(i + 1) {
                    out["page"] = json!(p);
                }
                i += 2;
            }
            _ => i += 1,
        }
    }
    out
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn page_alias(p: &str) -> Option<&'static str> {
    Some(match p {
        "apps" | "process" | "procs" | "processen" => "processes",
        "perf" | "prestaties" => "performance",
        "opslag" | "disk" | "disks" | "cleanup" | "opruimen" | "free-space" => "storage",
        "autostart" | "startup-apps" | "opstart" => "startup",
        "service" => "services",
        "samenvatting" | "home" => "summary",
        "instellingen" | "options" => "settings",
        _ => return None,
    })
}

fn tab_alias(t: &str) -> Option<&'static str> {
    Some(match t {
        "explore" | "verkennen" | "overview" => "explore",
        "categories" | "categorieen" | "categorieën" => "categories",
        "large" | "largefiles" | "large-files" | "grote-bestanden" | "grotebestanden" => "large",
        "cleanup" | "clean" | "opruimen" | "free-space" | "freeup" => "cleanup",
        _ => return None,
    })
}

/// main.js normParams: page aliases, tab aliases, and the page inferred from page-specific params.
fn norm_params(raw: Value) -> Value {
    let mut p = if raw.is_object() { raw } else { json!({}) };
    let mut page = p["page"].as_str().map(str::to_lowercase).unwrap_or_default();
    if !page.is_empty() && !PAGE_IDS.contains(&page.as_str()) {
        if (page == "cleanup" || page == "opruimen") && !truthy(&p["tab"]) && !truthy(&p["section"]) {
            p["tab"] = json!("cleanup");
        }
        page = page_alias(&page).unwrap_or("").to_owned();
    }
    let tab_raw = if truthy(&p["tab"]) { util::text(&p["tab"]) } else { util::text(&p["section"]) };
    let tab = tab_raw.to_lowercase();
    if !tab.is_empty() {
        p["tab"] = json!(tab_alias(&tab).unwrap_or(&tab));
    }
    if page.is_empty() {
        let t = p["tab"].as_str().unwrap_or("");
        page = if !t.is_empty() && tab_alias(t) == Some(t) {
            "storage"
        } else if truthy(&p["volume"]) || (truthy(&p["path"]) && !truthy(&p["pid"])) {
            "storage"
        } else if truthy(&p["device"]) {
            "performance"
        } else if truthy(&p["sort"]) {
            "processes"
        } else if truthy(&p["pid"]) {
            "details"
        } else {
            ""
        }
        .to_owned();
    }
    if let Some(o) = p.as_object_mut() {
        if page.is_empty() {
            o.remove("page");
        } else {
            o.insert("page".into(), json!(page));
        }
    }
    p
}

// ---------------------------------------------------------------------------------------------------- the UI

struct Host {
    page: Rc<dyn Page>,
    wrap: gtk::Box,
    cmdbar: gtk::Box,
}

struct Ui {
    app: Rc<App>,
    nav: NavView,
    search: SearchBox,
    search_box: gtk::Box,
    slot: gtk::Box,
    instances: RefCell<HashMap<&'static str, Rc<Host>>>,
    current: RefCell<Option<&'static str>>,
    user_compact: Cell<bool>,
    width: Cell<i32>,
}

thread_local! {
    static UI: RefCell<Option<Rc<Ui>>> = const { RefCell::new(None) };
}

fn page_id(id: &str) -> Option<&'static str> {
    PAGE_IDS.iter().copied().find(|p| *p == id)
}

fn nav_icon(id: &str) -> &'static str {
    match id {
        "summary" => "summary",
        "processes" => "apps",
        "performance" => "chart",
        "storage" => "storage",
        "startup" => "startup",
        "services" => "services",
        "details" => "list",
        _ => "settings",
    }
}

fn nav_items() -> (Vec<NavItem>, Vec<NavItem>) {
    let items = PAGE_IDS.iter().filter(|id| **id != "settings").map(|id| NavItem::new(id, pages::title_of(id), nav_icon(id))).collect();
    (items, vec![NavItem::new("settings", pages::title_of("settings"), nav_icon("settings"))])
}

fn search_placeholder() -> &'static str {
    kit::t("Typ een naam of PID om te zoeken", "Type a name or PID to search")
}

fn build(params: Value) {
    kit::theme::install_app_css(&[CSS, CSS_PERFORMANCE, CSS_SUMMARY, CSS_STORAGE].concat());
    let store = store::Store::new();
    let window = AppWindow::new(WindowOptions { key: "main".into(), width: 1040, height: 720, min_width: 500, min_height: 360, ..Default::default() });
    window.window().add_css_class("taskmgr-app");
    replace_app_icon(&window);
    let app = App::new(store, window.clone());
    hostcall::health();
    {
        let a = app.clone();
        hostcall::call("shell.info", json!({}), move |r| {
            if let Ok(info) = r {
                a.set_info(info);
            }
        });
    }

    // ---- title bar: the search box
    let search = SearchBox::new(Some(search_placeholder()));
    search.widget().add_css_class("taskmgr-search");
    search.text_widget().set_widget_name("taskmgr-search");
    search.widget().set_size_request(440, -1);
    let search_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    search_box.set_hexpand(true);
    search_box.set_halign(gtk::Align::Center);
    search_box.append(search.widget());
    window.set_titlebar_content(Some(&search_box));

    // ---- navigation and content layer
    let (items, footer) = nav_items();
    let nav = NavView::new(Vec::new());
    nav.set_items(items, footer);
    nav.set_pane_toggle(true);
    nav.widget().set_widget_name("taskmgr-nav");
    let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    layer.add_css_class("w11-content");
    layer.add_css_class("w11-layer");
    layer.add_css_class("taskmgr-layer");
    layer.set_hexpand(true);
    layer.set_vexpand(true);
    let slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
    slot.set_vexpand(true);
    slot.set_hexpand(true);
    let banner = hostbanner::HostBanner::new(&app);
    layer.append(banner.widget());
    layer.append(&slot);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_vexpand(true);
    row.append(nav.widget());
    row.append(&layer);
    // a zero-height strip across the window: its width drives the compact pane and the narrow command bar
    let sizer = gtk::DrawingArea::new();
    sizer.set_content_height(0);
    sizer.set_hexpand(true);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&sizer);
    content.append(&row);
    window.set_content(&content);

    let ui = Rc::new(Ui {
        app: app.clone(),
        nav: nav.clone(),
        search: search.clone(),
        search_box,
        slot,
        instances: RefCell::new(HashMap::new()),
        current: RefCell::new(None),
        user_compact: Cell::new(util::load_bool("navCompact", false)),
        width: Cell::new(1040),
    });
    UI.with(|u| *u.borrow_mut() = Some(ui.clone()));

    let w = Rc::downgrade(&ui);
    app.set_navigator(move |id, params| {
        if let Some(ui) = w.upgrade() {
            ui.go(id, params);
        }
    });
    let w = Rc::downgrade(&ui);
    nav.connect_selected(move |id| {
        if let (Some(ui), Some(id)) = (w.upgrade(), page_id(id)) {
            ui.show(id, json!({}));
        }
    });
    hook_hamburger(&ui);
    let w = Rc::downgrade(&ui);
    sizer.connect_resize(move |_, width, _| {
        if let Some(ui) = w.upgrade() {
            ui.width.set(width);
            ui.apply_compact();
        }
    });

    // ---- search
    let w = Rc::downgrade(&ui);
    search.connect_changed(move |q| {
        if let Some(ui) = w.upgrade() {
            ui.on_search(q, false);
        }
    });
    let w = Rc::downgrade(&ui);
    search.connect_submit(move |q| {
        if let Some(ui) = w.upgrade() {
            ui.on_search(q, true);
        }
    });
    let esc = gtk::EventControllerKey::new();
    let w = Rc::downgrade(&ui);
    esc.connect_key_pressed(move |_, key, _, _| {
        let Some(ui) = w.upgrade() else { return glib::Propagation::Proceed };
        if key == gdk::Key::Escape && !ui.search.text().is_empty() {
            ui.search.set_text("");
            ui.on_search("", false);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    search.text_widget().add_controller(esc);

    // ---- live updates pushed by the host
    {
        let a = app.clone();
        kit::ctx().on("storage.volumesChanged", move |_, d| {
            if let Some(v) = d["volumes"].as_array() {
                a.store.inject("volumes", Value::Array(v.clone()));
            }
        });
    }

    install_keys(&ui);
    watch_visibility(&ui);
    let w = Rc::downgrade(&ui);
    kit::i18n::on_change(move || {
        if let Some(ui) = w.upgrade() {
            ui.lang_changed();
        }
    });
    window.window().connect_close_request(|_| {
        util::flush();
        glib::Propagation::Proceed
    });

    let fallback = app.setting("taskmgr.defaultPage").as_str().and_then(page_id).unwrap_or("summary");
    let initial = params["page"].as_str().and_then(page_id).unwrap_or(fallback);
    ui.nav.select(initial);
    ui.show(initial, params.clone());
    window.present();
    if let Some(what) = params["open"].as_str() {
        debug_open(&ui, what.to_owned(), params["target"].as_str().unwrap_or("").to_lowercase());
    }
}

/// The title bar icon of main.js appGlyph (a green tile with a pulse line) instead of the kit's glyph.
fn replace_app_icon(window: &AppWindow) {
    fn find(w: &gtk::Widget) -> Option<gtk::Box> {
        if w.has_css_class("w11-appicon") {
            return w.downcast_ref::<gtk::Box>().cloned();
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            if let Some(b) = find(&child) {
                return Some(b);
            }
            c = child.next_sibling();
        }
        None
    }
    if let Some(slot) = find(window.titlebar_box().upcast_ref()) {
        kit::controls::clear(&slot);
        slot.append(&util::icon(util::TmGlyph::AppTile, 16));
    }
}

/// main.js: the hamburger opens the pane for good on a wide window, and closes it for good.
fn hook_hamburger(ui: &Rc<Ui>) {
    let Some(toggle) = util::find_named(ui.nav.widget().upcast_ref(), "nav-toggle").and_downcast::<gtk::Button>() else { return };
    let w = Rc::downgrade(ui);
    toggle.connect_clicked(move |_| {
        let Some(ui) = w.upgrade() else { return };
        if ui.width.get() >= COMPACT_BELOW {
            let compact = ui.nav.compact();
            ui.user_compact.set(compact);
            util::save("navCompact", json!(compact));
        }
        ui.apply_narrow();
    });
}

fn install_keys(ui: &Rc<Ui>) {
    let win = &ui.app.window;
    let w = Rc::downgrade(ui);
    let focus_search = move || {
        if let Some(ui) = w.upgrade() {
            ui.search.focus();
            ui.search.text_widget().select_region(0, -1);
        }
        true
    };
    win.add_shortcut("<Control>f", focus_search.clone());
    win.add_shortcut("<Control>e", focus_search);
    let w = Rc::downgrade(ui);
    win.add_shortcut("F5", move || {
        if let Some(ui) = w.upgrade() {
            let page = ui.current_host().map(|h| h.page.clone());
            if !page.is_some_and(|p| p.refresh()) {
                ui.app.store.refresh_all();
            }
        }
        true
    });
    let w = Rc::downgrade(ui);
    win.add_shortcut("<Control>Tab", move || {
        if let Some(ui) = w.upgrade() {
            ui.cycle(1);
        }
        true
    });
    let w = Rc::downgrade(ui);
    win.add_shortcut("<Control><Shift>Tab", move || {
        if let Some(ui) = w.upgrade() {
            ui.cycle(PAGE_IDS.len() - 1);
        }
        true
    });
    // Ctrl+Tab moves the focus in GTK before the window's shortcuts see it: take it in the capture phase
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let w = Rc::downgrade(ui);
    keys.connect_key_pressed(move |_, key, _, state| {
        let Some(ui) = w.upgrade() else { return glib::Propagation::Proceed };
        if ui.app.window.dialog_open() || !state.contains(gdk::ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Tab | gdk::Key::KP_Tab if !state.contains(gdk::ModifierType::SHIFT_MASK) => {
                ui.cycle(1);
                glib::Propagation::Stop
            }
            gdk::Key::ISO_Left_Tab | gdk::Key::Tab | gdk::Key::KP_Tab => {
                ui.cycle(PAGE_IDS.len() - 1);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    win.window().add_controller(keys);
}

/// No polling and no heartbeat while the window is minimized or hidden (main.js: document.hidden).
fn watch_visibility(ui: &Rc<Ui>) {
    let apply = |window: &gtk::ApplicationWindow| {
        let mut visible = window.is_visible() && window.is_mapped();
        if let Some(top) = window.surface().and_downcast::<gdk::Toplevel>() {
            let st = top.state();
            if st.contains(gdk::ToplevelState::MINIMIZED) || st.contains(gdk::ToplevelState::SUSPENDED) {
                visible = false;
            }
        }
        if let Some(ui) = UI.with(|u| u.borrow().clone()) {
            ui.app.store.set_visible(visible);
        }
        hostcall::health().set_visible(visible);
    };
    let window = ui.app.window.window().clone();
    window.connect_realize(move |w| {
        if let Some(top) = w.surface().and_downcast::<gdk::Toplevel>() {
            let wk = w.downgrade();
            top.connect_state_notify(move |_| {
                if let Some(w) = wk.upgrade() {
                    apply(&w);
                }
            });
        }
    });
    window.connect_map(move |w| apply(w));
    window.connect_unmap(move |w| apply(w));
}

impl Ui {
    fn current_host(&self) -> Option<Rc<Host>> {
        let id = (*self.current.borrow())?;
        self.instances.borrow().get(id).cloned()
    }

    fn host(self: &Rc<Self>, id: &'static str) -> Option<Rc<Host>> {
        if let Some(h) = self.instances.borrow().get(id).cloned() {
            return Some(h);
        }
        let page = pages::create(id, &self.app)?;
        let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wrap.add_css_class("taskmgr-page");
        wrap.add_css_class(&format!("taskmgr-page-{}", page.id()));
        wrap.set_widget_name(&format!("taskmgr-page-{}", page.id()));
        wrap.set_vexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        header.add_css_class("taskmgr-page-header");
        let title = util::label(&page.title(), "taskmgr-page-title");
        title.set_hexpand(true);
        header.append(&title);
        let cmdbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        cmdbar.add_css_class("taskmgr-cmdbar");
        cmdbar.set_valign(gtk::Align::Center);
        for c in page.commands() {
            cmdbar.append(&c);
        }
        header.append(&cmdbar);
        wrap.append(&header);
        let body = page.widget();
        body.set_vexpand(true);
        if page.fill() {
            body.add_css_class("taskmgr-page-body");
            wrap.append(&body);
        } else {
            let sc = kit::controls::scrolled(&body);
            sc.add_css_class("taskmgr-page-body");
            wrap.append(&sc);
        }
        let host = Rc::new(Host { page, wrap, cmdbar });
        self.instances.borrow_mut().insert(id, host.clone());
        Some(host)
    }

    /// Show a page (main.js show): a new page is mounted with the params, the current one navigates.
    fn show(self: &Rc<Self>, id: &'static str, params: Value) {
        let Some(host) = self.host(id) else {
            if let Some(old) = self.current_host() {
                old.page.unmount();
            }
            *self.current.borrow_mut() = Some(id);
            kit::controls::clear(&self.slot);
            let ok = kit::controls::button("OK", kit::controls::Variant::Standard);
            let w = Rc::downgrade(self);
            ok.connect_clicked(move |_| {
                if let Some(ui) = w.upgrade() {
                    ui.show(id, json!({}));
                }
            });
            let e = kit::controls::empty_state(
                "warning",
                kit::t("Deze pagina kon niet worden geladen", "This page couldn\u{2019}t be loaded"),
                kit::t("Probeer het opnieuw. Helpt dat niet, sluit Taakbeheer dan en open het opnieuw.", "Try again. If that doesn\u{2019}t help, close Task Manager and open it again."),
                Some(ok.upcast_ref()),
            );
            e.set_vexpand(true);
            self.slot.append(&e);
            return;
        };
        let same = *self.current.borrow() == Some(id) && host.wrap.parent().is_some();
        if same {
            host.page.navigate(&params);
        } else {
            if let Some(old) = self.current_host() {
                old.page.unmount();
            }
            *self.current.borrow_mut() = Some(id);
            kit::controls::clear(&self.slot);
            self.slot.append(&host.wrap);
            host.page.mount(&params);
            if host.page.searchable() {
                host.page.on_search(&self.app.query());
            }
            animate_in(&host.wrap);
        }
        self.nav.select(id);
        self.app.window.window().set_title(Some(&format!("{} \u{2013} {}", kit::t("Taakbeheer", "Task Manager"), host.page.title())));
        self.apply_narrow();
    }

    /// Go to a page (main.js go): selects it in the navigation too.
    fn go(self: &Rc<Self>, id: &str, params: Value) {
        let Some(id) = page_id(id) else { return };
        self.show(id, params);
    }

    fn cycle(self: &Rc<Self>, step: usize) {
        let cur = (*self.current.borrow()).unwrap_or("summary");
        let i = PAGE_IDS.iter().position(|p| *p == cur).unwrap_or(0);
        let next = PAGE_IDS[(i + step) % PAGE_IDS.len()];
        self.show(next, json!({}));
    }

    /// The search box (main.js onSearch): a page that does not search sends a query to Processen.
    fn on_search(self: &Rc<Self>, q: &str, submit: bool) {
        self.app.set_query(q);
        match self.current_host() {
            Some(h) if h.page.searchable() => h.page.on_search(q),
            _ => {
                if q.is_empty() && !submit {
                    return;
                }
                self.go("processes", json!({}));
            }
        }
    }

    fn apply_compact(&self) {
        let width = self.width.get();
        let compact = self.user_compact.get() || width < COMPACT_BELOW;
        if compact != self.nav.compact() {
            self.nav.set_compact(compact);
        }
        // the search box: min(100%, 440 px) of the title bar's middle
        let room = (width - 420).clamp(160, 440);
        self.search.widget().set_size_request(room, -1);
        self.search_box.queue_resize();
        self.apply_narrow();
    }

    /// Hide the command bar's labels while the page is narrow.
    fn apply_narrow(&self) {
        let nav_w = if self.nav.compact() { 48 } else { 240 };
        let narrow = self.width.get() - nav_w <= NARROW_PAGE;
        let Some(host) = self.current_host() else { return };
        util::set_class(&host.cmdbar, "taskmgr-narrow", narrow);
        fn walk(w: &gtk::Widget, narrow: bool, in_button: bool) {
            let is_button = w.is::<gtk::Button>() && w.has_css_class("w11-subtle");
            if in_button && let Some(l) = w.downcast_ref::<gtk::Label>() {
                l.set_visible(!narrow);
            }
            let mut c = w.first_child();
            while let Some(child) = c {
                walk(&child, narrow, in_button || is_button);
                c = child.next_sibling();
            }
        }
        walk(host.cmdbar.upcast_ref(), narrow, false);
    }

    /// The language changed: relabel the shell and make every page again (main.js onLangChange).
    fn lang_changed(self: &Rc<Self>) {
        self.search.set_placeholder(search_placeholder());
        for id in PAGE_IDS {
            self.nav.set_label(id, pages::title_of(id));
        }
        self.apply_compact();
        let id = *self.current.borrow();
        if let Some(h) = self.current_host() {
            h.page.unmount();
        }
        *self.current.borrow_mut() = None;
        self.instances.borrow_mut().clear();
        kit::controls::clear(&self.slot);
        // Opstart-apps, Services, Opslag: the host's names and descriptions in the new language
        self.app.store.lang_changed();
        if let Some(id) = id {
            self.show(id, json!({}));
        }
    }
}

/// The page's entrance (main.js: 300 ms, 28 px up, fading in); none with reduced motion.
fn animate_in(w: &gtk::Box) {
    if kit::ctx().reduce_motion() {
        return;
    }
    w.set_opacity(0.0);
    w.set_margin_top(28);
    // without frames (a hidden window) the page still ends up in place
    let wk = w.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(450), move || {
        if let Some(w) = wk.upgrade() {
            w.set_opacity(1.0);
            w.set_margin_top(0);
        }
    });
    // the frame clock and glib's monotonic time share a base: late frames jump to the end
    let t0 = glib::monotonic_time();
    w.add_tick_callback(move |w, clock| {
        let p = ((clock.frame_time() - t0) as f64 / 300_000.0).clamp(0.0, 1.0);
        // cubic-bezier(0, 0, 0, 1), close enough: a strong ease-out
        let e = 1.0 - (1.0 - p).powi(4);
        w.set_opacity(e);
        w.set_margin_top((28.0 * (1.0 - e)).round() as i32);
        if p >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// Screenshot and test helpers (main.js debugOpen): `open` = end | restart | efficiency | properties | menu
/// (with `target`, part of a row's key or name; default the first row) opens that question for an app row. It
/// always asks, also when the settings skip the question ("Vragen voordat een taak wordt beëindigd" off), so it
/// never ends, restarts or changes anything without a click.
fn debug_open(ui: &Rc<Ui>, what: String, target: String) {
    let app = ui.app.clone();
    app.store.clone().fetch("processes", 60000, move |data| {
        let Some(data) = data else { return };
        let rows = pages::processes::all_rows(&data);
        let row = rows
            .iter()
            .find(|r| !target.is_empty() && format!("{} {}", r.key, r.name).to_lowercase().contains(&target))
            .or_else(|| rows.first())
            .cloned();
        let Some(row) = row else { return };
        glib::timeout_add_local_once(Duration::from_millis(400), move || match what.as_str() {
            "end" => actions::ask_end_task(&app, &row),
            "restart" => actions::ask_restart_task(&app, &row),
            "efficiency" => actions::ask_efficiency(&app, &row),
            "properties" => actions::show_properties(&app, &row),
            "menu" => {
                let anchor = app.window.window().clone();
                util::fix_menu_glyphs(&anchor, actions::menu_glyphs(&row));
                let (top, items) = actions::process_menu(&app, &row, None);
                kit::controls::show_menu_with_top(&anchor, 420.0, 260.0, top, items);
            }
            _ => {}
        });
    });
}
