//! The app (6ebf169f:vendor/ui/js/apps/disks/main.js): the snapshot of disks.list, the pages (overview, a volume's
//! and a disk's properties) with Back and the breadcrumb, the selection, live updates (disks.changed, a 30 s poll,
//! window focus, F5), the language switch and the deep links (--params {volume|disk|select|page|id|open}).
use crate::model::{self, find_disk, find_volume, st, truthy};
use crate::strings::s;
use crate::widgets::{Clamp, hbox, label, vbox};
use crate::{actions, art, overview, properties};
use kit::controls::{self, Breadcrumb, InfoBar, Severity, Variant};
use kit::frame::{AppWindow, WindowOptions};
use kit::prelude::*;
use kit::{CallError, Ctx};
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

const POLL_SECS: u32 = 30;

/// The page on screen.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Overview,
    Volume(String),
    Disk(String),
}

impl View {
    fn key(&self) -> String {
        match self {
            View::Overview => "overview:".into(),
            View::Volume(id) => format!("volume:{id}"),
            View::Disk(id) => format!("disk:{id}"),
        }
    }
    fn name(&self) -> &'static str {
        match self {
            View::Overview => "overview",
            View::Volume(_) => "volume",
            View::Disk(_) => "disk",
        }
    }
}

/// What a selectable widget is: a bar segment (with its bar), a volume / free row, an extended partition.
#[derive(Clone, Copy, PartialEq)]
pub enum MarkKind {
    Seg(u32),
    Row,
    Group,
}

struct Mark {
    id: String,
    kind: MarkKind,
    widget: glib::WeakRef<gtk::Widget>,
}

pub struct App {
    pub ctx: Rc<Ctx>,
    pub win: AppWindow,
    me: Weak<App>,
    title: gtk::Label,
    scroller: gtk::ScrolledWindow,
    clamp: Clamp,
    snap: RefCell<Option<Value>>,
    load_error: RefCell<Option<CallError>>,
    view: RefCell<View>,
    history: RefCell<Vec<(View, f64)>>,
    selected: RefCell<Option<String>>,
    loading: Cell<bool>,
    loading_lang: Cell<&'static str>,
    generation: Cell<u64>,
    waiters: RefCell<Vec<Box<dyn FnOnce(&Rc<App>)>>>,
    pending_render: Cell<bool>,
    last_key: RefCell<String>,
    /// What the page on screen was built from (a render with the same inputs is skipped).
    rendered: RefCell<Option<(String, Option<Value>, Option<String>)>>,
    dry_run: Cell<bool>,
    width: Cell<i32>,
    want_scroll: Rc<Cell<Option<f64>>>,
    marks: RefCell<Vec<Mark>>,
    next_bar: Cell<u32>,
    menu: RefCell<glib::WeakRef<gtk::Popover>>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

/// The query keys of `--params JSON` (the WebKit host's run_inner turned them into the page's query: `git show
/// 6ebf169f:host/src/lib.rs`).
pub fn parse_args(args: &[String]) -> Value {
    let mut out = serde_json::Map::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--params"
            && let Some(raw) = args.get(i + 1)
        {
            if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(raw) {
                for (k, v) in o {
                    let text = match v {
                        Value::String(s) => s,
                        other => other.to_string(),
                    };
                    out.insert(k, Value::String(text));
                }
            }
            i += 2;
        } else {
            i += 1;
        }
    }
    Value::Object(out)
}

impl App {
    /// The running app (main thread).
    pub fn get() -> Option<Rc<App>> {
        APP.with(|a| a.borrow().clone())
    }

    /// The first launch: the window, the first page, the first load, then the deep link.
    pub fn start(ctx: &Rc<Ctx>, params: Value) -> Rc<App> {
        let win = AppWindow::new(WindowOptions {
            key: "main".into(),
            title: String::new(),
            icon: "-".into(),
            width: 1060,
            height: 760,
            min_width: 500,
            min_height: 400,
            back: true,
            ..Default::default()
        });
        // the title bar: util.js appGlyph and the app's name
        let tb = hbox(0, "dk-titlebox");
        let icon = art::app_glyph(16);
        icon.add_css_class("w11-appicon");
        tb.append(&icon);
        let title = label(&s("appTitle"), "dk-apptitle", false);
        title.set_valign(gtk::Align::Center);
        tb.append(&title);
        win.set_title_widget(Some(&tb));
        win.window().add_css_class("dk-app");

        let clamp = Clamp::new(1040);
        let scroller = controls::scrolled(&clamp);
        scroller.set_widget_name("disks-scroller");
        win.set_content(&scroller);

        let app = Rc::new_cyclic(|me| App {
            ctx: ctx.clone(),
            win: win.clone(),
            me: me.clone(),
            title,
            scroller,
            clamp,
            snap: RefCell::new(None),
            load_error: RefCell::new(None),
            view: RefCell::new(View::Overview),
            history: RefCell::new(Vec::new()),
            selected: RefCell::new(None),
            loading: Cell::new(false),
            loading_lang: Cell::new("nl"),
            generation: Cell::new(0),
            waiters: RefCell::new(Vec::new()),
            pending_render: Cell::new(false),
            last_key: RefCell::new(String::new()),
            rendered: RefCell::new(None),
            dry_run: Cell::new(false),
            width: Cell::new(0),
            want_scroll: Rc::new(Cell::new(None)),
            marks: RefCell::new(Vec::new()),
            next_bar: Cell::new(0),
            menu: RefCell::new(glib::WeakRef::new()),
        });
        APP.with(|a| *a.borrow_mut() = Some(app.clone()));
        app.wire();
        win.set_title(&s("appTitle"));
        app.render(false, None, true);
        win.present();
        // shell.info: the dry-run notice (theme.js getShellInfo)
        let weak = app.me.clone();
        ctx.call("shell.info", json!({}), move |r| {
            if let (Some(app), Ok(info)) = (weak.upgrade(), r)
                && truthy(&info["dryRun"])
            {
                app.dry_run.set(true);
                app.render(false, None, true);
            }
        });
        let has_link = ["volume", "disk", "select", "open"].iter().any(|k| truthy(&params[*k]));
        app.load_then(false, move |app| {
            if has_link {
                app.apply_params(params);
            }
        });
        app
    }

    fn wire(self: &Rc<Self>) {
        let weak = self.me.clone();
        self.win.connect_back(move || {
            if let Some(app) = weak.upgrade() {
                app.back();
            }
        });
        let weak = self.me.clone();
        self.win.add_shortcut("F5", move || {
            if let Some(app) = weak.upgrade() {
                app.load(true);
            }
            true
        });
        let weak = self.me.clone();
        self.ctx.on("disks.changed", move |_, d| {
            if let Some(app) = weak.upgrade()
                && d["disks"].is_array()
            {
                *app.snap.borrow_mut() = Some(d.clone());
                *app.load_error.borrow_mut() = None;
                app.render(false, None, false);
            }
        });
        let weak = self.me.clone();
        self.ctx.on("app.navigate", move |_, d| {
            if let Some(app) = weak.upgrade()
                && (!truthy(&d["app"]) || d["app"] == "disks")
            {
                app.apply_params(d["params"].clone());
            }
        });
        let weak = self.me.clone();
        glib::timeout_add_seconds_local(POLL_SECS, move || {
            let Some(app) = weak.upgrade() else { return glib::ControlFlow::Break };
            if app.win.window().is_visible() && !app.win.dialog_open() {
                app.load(false);
            }
            glib::ControlFlow::Continue
        });
        let weak = self.me.clone();
        self.win.window().connect_is_active_notify(move |w| {
            if w.is_active()
                && let Some(app) = weak.upgrade()
            {
                app.load(false);
            }
        });
        // a language switch: our labels at once, then the host's texts in the new language (refresh: the host's
        // snapshot of up to 1 s old is not kept per language)
        let weak = self.me.clone();
        kit::i18n::on_change(move || {
            if let Some(app) = weak.upgrade() {
                app.title.set_label(&s("appTitle"));
                app.render(false, None, true);
                app.load(true);
            }
        });
        // the media queries of disks.css (760 px: no usage column; 680 px: narrow page padding)
        let weak = self.me.clone();
        self.clamp.connect_width(move |w| {
            let Some(app) = weak.upgrade() else { return };
            let old = app.width.replace(w);
            let class = |x: i32| (x > 0 && x <= 680, x > 0 && x <= 760);
            if class(old) != class(w) {
                app.render(false, None, true);
            }
        });
        let want = self.want_scroll.clone();
        self.scroller.vadjustment().connect_changed(move |adj| {
            if let Some(v) = want.get() {
                adj.set_value(v);
            }
        });
    }

    // --------------------------------------------------------------------------------------------- state

    pub fn snap(&self) -> Option<Value> {
        self.snap.borrow().clone()
    }

    pub fn selected(&self) -> Option<String> {
        self.selected.borrow().clone()
    }

    /// Below the 760 px breakpoint (no usage column, the disk card's buttons on their own row).
    pub fn narrow(&self) -> bool {
        let w = self.width.get();
        w > 0 && w <= 760
    }

    /// Below the 680 px breakpoint (the page's narrow padding).
    fn narrow_page(&self) -> bool {
        let w = self.width.get();
        w > 0 && w <= 680
    }

    /// The window (parent of dialogs, toasts and menus).
    pub fn parent(&self) -> gtk::Widget {
        self.win.window().clone().upcast()
    }

    pub fn rc(&self) -> Option<Rc<App>> {
        self.me.upgrade()
    }

    // --------------------------------------------------------------------------------------------- selection

    /// Remember a selectable widget of the page being built.
    pub fn mark(&self, id: &str, kind: MarkKind, w: &impl IsA<gtk::Widget>) {
        self.marks.borrow_mut().push(Mark { id: id.to_owned(), kind, widget: w.as_ref().downgrade() });
    }

    /// A new bar id (roving tab stop per bar).
    pub fn new_bar(&self) -> u32 {
        let n = self.next_bar.get();
        self.next_bar.set(n + 1);
        n
    }

    /// main.js ctx.select: mark the place selected everywhere; only the bar holding it moves its tab stop.
    pub fn select(&self, id: &str, focus: bool) {
        *self.selected.borrow_mut() = Some(id.to_owned());
        let marks: Vec<(String, MarkKind, gtk::Widget)> =
            self.marks.borrow().iter().filter_map(|m| m.widget.upgrade().map(|w| (m.id.clone(), m.kind, w))).collect();
        let bar = marks.iter().find_map(|(mid, k, _)| match k {
            MarkKind::Seg(b) if mid == id => Some(*b),
            _ => None,
        });
        for (mid, kind, w) in &marks {
            let on = mid == id;
            if on {
                w.add_css_class("dk-selected");
            } else {
                w.remove_css_class("dk-selected");
            }
            if let (Some(b), MarkKind::Seg(mb)) = (bar, kind)
                && b == *mb
            {
                w.set_focusable(on);
            }
            if on && focus {
                w.set_focusable(true);
                w.grab_focus();
            }
        }
    }

    // --------------------------------------------------------------------------------------------- navigation

    pub fn open_volume(&self, id: &str) {
        self.go(View::Volume(id.to_owned()), true);
    }

    pub fn open_disk(&self, id: &str) {
        self.go(View::Disk(id.to_owned()), true);
    }

    fn scroll_now(&self) -> f64 {
        self.scroller.vadjustment().value()
    }

    pub fn go(&self, next: View, push: bool) {
        if push {
            let cur = self.view.borrow().clone();
            self.history.borrow_mut().push((cur, self.scroll_now()));
        }
        *self.view.borrow_mut() = next;
        self.win.set_back_enabled(!self.history.borrow().is_empty());
        self.render(true, Some(0.0), true);
    }

    pub fn back(&self) {
        let prev = self.history.borrow_mut().pop();
        self.win.set_back_enabled(!self.history.borrow().is_empty());
        let Some((view, scroll)) = prev else { return };
        *self.view.borrow_mut() = view;
        self.render(true, Some(scroll), true);
    }

    fn home(&self) {
        self.history.borrow_mut().clear();
        self.win.set_back_enabled(false);
        *self.view.borrow_mut() = View::Overview;
        self.render(true, Some(0.0), true);
    }

    /// A deleted volume: leave its page.
    pub fn volume_gone(&self, id: &str) {
        let on = matches!(&*self.view.borrow(), View::Volume(v) if v == id);
        if on {
            self.back();
        }
    }

    /// A removed disk: leave its page and its volumes' pages.
    pub fn disk_gone(&self, id: &str) {
        let view = self.view.borrow().clone();
        let hit = match &view {
            View::Overview => false,
            View::Disk(d) => d == id,
            View::Volume(v) => {
                v == id || self.snap().and_then(|s| find_volume(&s, v)).is_some_and(|(d, _)| st(&d["id"]) == id)
            }
        };
        if hit {
            self.go(View::Overview, false);
        }
    }

    // --------------------------------------------------------------------------------------------- data

    /// disks.list (refresh: past the host's 1 s snapshot).
    pub fn load(&self, refresh: bool) {
        self.load_then(refresh, |_| {});
    }

    /// A load is sent in the language of that moment (X-Shell-Lang): after a language switch one still running is
    /// not reused, and its late answer never overwrites the newer one. `then` runs after the load that answers.
    pub fn load_then(&self, refresh: bool, then: impl FnOnce(&Rc<App>) + 'static) {
        self.waiters.borrow_mut().push(Box::new(then));
        let lang = kit::lang();
        if self.loading.get() && self.loading_lang.get() == lang {
            return;
        }
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.loading.set(true);
        self.loading_lang.set(lang);
        let weak = self.me.clone();
        let args = if refresh { json!({ "refresh": true }) } else { json!({}) };
        self.ctx.call("disks.list", args, move |r| {
            let Some(app) = weak.upgrade() else { return };
            if app.generation.get() != generation {
                return;
            }
            match r {
                Ok(v) if v.is_object() => {
                    *app.snap.borrow_mut() = Some(v);
                    *app.load_error.borrow_mut() = None;
                }
                Ok(_) => *app.load_error.borrow_mut() = Some(CallError::new("bad_response", s("unavailable"))),
                Err(e) => *app.load_error.borrow_mut() = Some(e),
            }
            app.loading.set(false);
            app.render(false, None, false);
            let waiters: Vec<Box<dyn FnOnce(&Rc<App>)>> = app.waiters.borrow_mut().drain(..).collect();
            for w in waiters {
                w(&app);
            }
        });
    }

    // --------------------------------------------------------------------------------------------- menus

    /// A context menu at (x, y) in `w`; while it is open, renders wait (the page stays stable under it).
    pub fn menu_at(&self, w: &impl IsA<gtk::Widget>, x: f64, y: f64, items: Vec<kit::controls::MenuItem>) {
        let p = kit::controls::show_menu_at(w, x, y, items);
        self.track_menu(&p);
    }

    /// A menu below `anchor` (the "..." button).
    pub fn menu_for(&self, anchor: &impl IsA<gtk::Widget>, items: Vec<kit::controls::MenuItem>) {
        let p = kit::controls::show_menu_for(anchor, items);
        self.track_menu(&p);
    }

    fn track_menu(&self, p: &gtk::Popover) {
        *self.menu.borrow_mut() = p.downgrade();
        let weak = self.me.clone();
        p.connect_closed(move |_| {
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(app) = weak.upgrade() {
                    app.dialog_closed();
                }
            });
        });
    }

    fn menu_open(&self) -> bool {
        self.menu.borrow().upgrade().is_some_and(|p| p.is_visible())
    }

    /// After one of our dialogs or menus closed: the render it held back.
    pub fn dialog_closed(&self) {
        if self.pending_render.get() && !self.win.dialog_open() && !self.menu_open() {
            self.pending_render.set(false);
            self.render(false, None, true);
        }
    }

    // --------------------------------------------------------------------------------------------- rendering

    fn header(&self, crumbs: &[(String, String)], cmds: Vec<gtk::Widget>) -> gtk::Widget {
        let head = hbox(16, "dk-head");
        let bc = Breadcrumb::new(true);
        let items: Vec<(&str, &str)> = crumbs.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        bc.set_items(&items);
        bc.widget().set_hexpand(true);
        bc.widget().set_valign(gtk::Align::Center);
        let weak = self.me.clone();
        bc.connect_selected(move |id, _| {
            let Some(app) = weak.upgrade() else { return };
            if id == "overview" {
                app.home();
            } else if let Some(d) = id.strip_prefix("disk:") {
                app.go(View::Disk(d.to_owned()), true);
            }
        });
        head.append(bc.widget());
        let c = hbox(4, "dk-cmds");
        c.set_valign(gtk::Align::Center);
        for w in cmds {
            c.append(&w);
        }
        head.append(&c);
        head.upcast()
    }

    fn notices(&self, snap: &Value) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        if self.dry_run.get() {
            out.push(InfoBar::new(Severity::Info, "", &s("dryRun")).widget().clone().upcast());
        }
        if truthy(&snap["readOnly"]) {
            out.push(InfoBar::new(Severity::Info, "", &s("readOnly")).widget().clone().upcast());
        }
        out
    }

    fn refresh_button(&self, accent: bool) -> gtk::Button {
        let b = if accent {
            controls::button(&s("refresh"), Variant::Accent)
        } else {
            controls::button_with_glyph("refresh", &s("refresh"), Variant::Subtle)
        };
        let weak = self.me.clone();
        b.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.load(true);
            }
        });
        b
    }

    fn overview_page(&self) -> Vec<gtk::Widget> {
        let refresh = self.refresh_button(false);
        refresh.set_widget_name("disks-refresh");
        let mut out = vec![self.header(&[("overview".into(), s("overview"))], vec![refresh.upcast()])];
        let snap = self.snap();
        let Some(snap) = snap else {
            if let Some(e) = self.load_error.borrow().as_ref() {
                let a = self.refresh_button(true);
                out.push(controls::empty_state("warning", &s("unavailable"), &e.message, Some(a.upcast_ref())).upcast());
            } else {
                let b = vbox(16, "dk-loading");
                b.set_halign(gtk::Align::Center);
                b.append(kit::controls::ProgressRing::new(32, None).widget());
                let l = label(&s("loading"), "", false);
                l.set_xalign(0.5);
                b.append(&l);
                out.push(b.upcast());
            }
            return out;
        };
        out.extend(self.notices(&snap));
        if !truthy(&snap["supported"]) {
            let a = self.refresh_button(true);
            out.push(controls::empty_state("warning", &s("unavailable"), &st(&snap["error"]), Some(a.upcast_ref())).upcast());
        } else if model::arr(&snap["disks"]).is_empty() {
            out.push(controls::empty_state("disk", &s("noDisks"), &s("noDisksBody"), None).upcast());
        } else {
            let list = vbox(16, "dk-disks");
            let Some(app) = self.rc() else { return out };
            for d in model::arr(&snap["disks"]) {
                list.append(&overview::disk_card(&app, d));
            }
            out.push(list.upcast());
            out.push(overview::legend(&snap));
        }
        out
    }

    fn volume_page(&self, snap: &Value, id: &str) -> Option<(Vec<gtk::Widget>, String)> {
        let (d, v) = find_volume(snap, id)?;
        let app = self.rc()?;
        let crumbs = [("overview".to_owned(), s("overview")), (format!("disk:{}", st(&d["id"])), st(&d["name"])), ("v".into(), st(&v["name"]))];
        let mut out = vec![self.header(&crumbs, vec![])];
        out.extend(self.notices(snap));
        out.extend(properties::volume_page(&app, &d, &v));
        Some((out, st(&v["name"])))
    }

    fn disk_page(&self, snap: &Value, id: &str) -> Option<(Vec<gtk::Widget>, String)> {
        let d = find_disk(snap, id)?;
        let app = self.rc()?;
        let crumbs = [("overview".to_owned(), s("overview")), ("d".into(), st(&d["name"]))];
        let mut out = vec![self.header(&crumbs, vec![])];
        out.extend(self.notices(snap));
        out.extend(properties::disk_page(&app, &d));
        Some((out, st(&d["name"])))
    }

    /// Build the page for the view. `animate`: a navigation (also while a dialog is open; otherwise a dialog
    /// holds the render back until it closes). `scroll`: where to scroll (None: keep). `force`: also when the
    /// snapshot and the view are unchanged.
    pub fn render(&self, animate: bool, scroll: Option<f64>, force: bool) {
        if (self.win.dialog_open() || self.menu_open()) && !animate {
            // a dialog is open (format running ...) or a menu: refresh after it closes, keep the page stable
            self.pending_render.set(true);
            return;
        }
        let snap = self.snap();
        let inputs = (
            format!("{}|{}|{}|{}|{}", self.view.borrow().key(), kit::lang(), self.dry_run.get(), self.narrow_page(), self.narrow()),
            snap.clone(),
            self.load_error.borrow().as_ref().map(|e| e.message.clone()),
        );
        if !force && !animate && self.rendered.borrow().as_ref() == Some(&inputs) {
            return;
        }
        self.marks.borrow_mut().clear();
        self.next_bar.set(0);
        let view = self.view.borrow().clone();
        let mut built: Option<(Vec<gtk::Widget>, String)> = None;
        if let Some(snap) = &snap {
            built = match &view {
                View::Volume(id) => self.volume_page(snap, id),
                View::Disk(id) => self.disk_page(snap, id),
                View::Overview => None,
            };
        }
        let (nodes, current) = match built {
            Some(b) => b,
            None => {
                if view != View::Overview && snap.is_some() {
                    *self.view.borrow_mut() = View::Overview;
                    self.history.borrow_mut().clear();
                    self.win.set_back_enabled(false);
                }
                (self.overview_page(), String::new())
            }
        };
        let view = self.view.borrow().clone();
        let keep = scroll.unwrap_or_else(|| self.scroll_now());
        let page = vbox(20, &format!("dk-page dk-page-{}", view.name()));
        page.set_widget_name("disks-page");
        if self.narrow_page() {
            page.add_css_class("dk-narrow");
        }
        if self.narrow() {
            page.add_css_class("dk-n760");
        }
        for n in &nodes {
            page.append(n);
        }
        let key = view.key();
        if animate && *self.last_key.borrow() != key {
            page.add_css_class("dk-enter");
        }
        *self.last_key.borrow_mut() = key;
        self.clamp.set_child(&page);
        self.scroll_to(keep);
        let title = if view == View::Overview || current.is_empty() { s("appTitle") } else { format!("{} \u{2013} {current}", s("appTitle")) };
        self.win.window().set_title(Some(&title));
        *self.rendered.borrow_mut() = Some(inputs);
    }

    /// Scroll there now and once the new page's height is known.
    fn scroll_to(&self, y: f64) {
        self.scroller.vadjustment().set_value(y);
        self.want_scroll.set(Some(y));
        let want = self.want_scroll.clone();
        glib::timeout_add_local_once(Duration::from_millis(400), move || want.set(None));
    }

    // --------------------------------------------------------------------------------------------- deep links

    /// main.js applyParams: ?volume= / ?disk= / ?page=volume|disk&id= / ?page=overview, ?select=, ?open=.
    pub fn apply_params(&self, p: Value) {
        let Some(snap) = self.snap() else { return };
        let get = |k: &str| st(&p[k]);
        let page = get("page");
        let vk = if !get("volume").is_empty() { get("volume") } else if page == "volume" { get("id") } else { String::new() };
        let dk = if !get("disk").is_empty() { get("disk") } else if page == "disk" { get("id") } else { String::new() };
        if !vk.is_empty() {
            if let Some((_, v)) = find_volume(&snap, &vk) {
                self.go(View::Volume(st(&v["id"])), true);
            }
        } else if !dk.is_empty() {
            if let Some(d) = find_disk(&snap, &dk) {
                self.go(View::Disk(st(&d["id"])), true);
            }
        } else if page == "overview" {
            self.home();
        }
        let sel = get("select");
        if !sel.is_empty() {
            let id = find_volume(&snap, &sel).map(|(_, v)| st(&v["id"])).unwrap_or_else(|| sel.clone());
            self.select(&id, false);
        }
        let open = get("open");
        if open.is_empty() {
            return;
        }
        let hit = find_volume(&snap, if sel.is_empty() { &vk } else { &sel });
        let disk = hit.as_ref().map(|(d, _)| d.clone()).or_else(|| find_disk(&snap, &dk));
        let weak = self.me.clone();
        glib::timeout_add_local_once(Duration::from_millis(350), move || {
            let Some(app) = weak.upgrade() else { return };
            let first_free = disk.as_ref().and_then(|d| model::arr(&d["free"]).first().cloned());
            match (open.as_str(), &hit, &disk) {
                ("format", Some((d, v)), _) => actions::format(&app, d, v),
                ("label", Some((_, v)), _) => actions::change_label(&app, v),
                ("delete", Some((d, v)), _) => actions::delete_volume(&app, d, v),
                ("unlock", Some((_, v)), _) => actions::unlock(&app, v),
                ("new", _, Some(d)) if first_free.is_some() => actions::new_volume(&app, d, first_free.as_ref().unwrap_or(&Value::Null)),
                ("initialize", _, Some(d)) => actions::initialize(&app, d),
                ("menu", Some((d, v)), _) => app.debug_menu(actions::volume_menu(&app, d, v), Some(st(&v["id"]))),
                ("disk-menu", _, Some(d)) => app.debug_menu(actions::disk_menu(&app, d), None),
                ("free-menu", _, Some(d)) if first_free.is_some() => {
                    app.debug_menu(actions::free_menu(&app, d, first_free.as_ref().unwrap_or(&Value::Null)), None)
                }
                _ => {}
            }
        });
    }

    /// ?open=menu|disk-menu|free-menu: a context menu, at its segment when it is on screen.
    fn debug_menu(&self, items: Vec<kit::controls::MenuItem>, id: Option<String>) {
        let marks: Vec<(String, MarkKind, gtk::Widget)> =
            self.marks.borrow().iter().filter_map(|m| m.widget.upgrade().map(|w| (m.id.clone(), m.kind, w))).collect();
        let target = match &id {
            Some(id) => marks.iter().find(|(m, k, _)| m == id && matches!(k, MarkKind::Seg(_))).map(|(_, _, w)| w.clone()),
            None => marks.iter().find(|(_, k, _)| matches!(k, MarkKind::Seg(_))).map(|(_, _, w)| w.clone()),
        };
        match target.filter(|w| w.is_mapped()) {
            Some(w) => {
                let (x, y) = ((w.width() as f64 / 2.0).min(80.0), w.height() as f64 / 2.0);
                self.menu_at(&w, x.round(), y.round(), items);
            }
            None => self.menu_at(self.win.content(), 300.0, 240.0, items),
        }
    }
}
