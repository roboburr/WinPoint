//! One Explorer window (last-webkit:vendor/ui/js/apps/explorer/main.js createExplorer): the tabs in the title bar, the
//! address row and command bar, the navigation pane, the content (Start, Galerij, Deze pc, a folder, the
//! Prullenbak or search results), the details pane and the status bar; navigation with history per tab,
//! loading through files.*, live updates (files.dirChanged, clipboard, drives, Prullenbak), the settings
//! (explorer.*), the keyboard, and the window's public API for the other modules (see main.rs).
use crate::details::{DetailsPane, PaneMode};
use crate::fileview::{Column, FileView, Group, Handlers, Mode, VIEW_MODES, ViewOpts};
use crate::i18n::{f, n, s};
use crate::model::{Loc, Status, Tab};
use crate::navpane::NavPane;
use crate::tabs::TabStrip;
use crate::toolbar::{AddressRow, CommandBar};
use crate::util::{self, Item, flag, num, st};
use crate::{menus, ops, pages, transfer};
use gtk::prelude::*;
use gtk::{gdk, glib};
use kit::controls::{InfoBar, Severity, Toast, ToastHandle};
use kit::frame::{AppWindow, WindowOptions};
use kit::{CallError, ListenerId};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

/// The special folders of Snelle toegang (in this order) and of Deze pc.
pub const PLACE_IDS: [&str; 6] = ["desktop", "downloads", "documents", "pictures", "music", "videos"];
const PC_FOLDERS: [&str; 6] = ["desktop", "documents", "downloads", "music", "pictures", "videos"];
/// Ctrl + wheel steps through these.
const ZOOM_ORDER: [&str; 6] = ["details", "list", "small", "medium", "large", "xl"];
const MAX_EXTRA_TABS: usize = 20;

/// What one window knows about the machine (main.js `S`).
#[derive(Default)]
pub struct Shared {
    /// files.places by id: {id, name, path, icon, exists}; "trash" has the path "trash:///".
    pub places: HashMap<String, Value>,
    /// The home folder.
    pub home: String,
    /// shell.info user: {name, realName}.
    pub user: Value,
    /// storage.volumes (shown ones, sorted).
    pub volumes: Vec<Value>,
    /// files.clipboardGet: the paths and "copy" / "cut".
    pub clipboard_paths: Vec<String>,
    pub clipboard_op: Option<String>,
    pub trash_full: bool,
}

/// One step of Ongedaan maken (Ctrl+Z).
pub struct Undo {
    #[allow(dead_code)] // for ops.rs (builder B)
    pub label: String,
    pub run: Rc<dyn Fn(&Explorer)>,
}

/// Hooks of the file picker (picker.rs) in this window.
#[derive(Clone)]
pub struct Picker {
    /// Which listed items are shown (folders always).
    pub filter: Rc<dyn Fn(&Value) -> bool>,
    /// The selection changed.
    pub on_selection: Rc<dyn Fn(&[Item])>,
    /// A file was opened (double click / Enter): choose it.
    pub on_select: Rc<dyn Fn(&Item)>,
    /// The window title.
    pub title: Rc<dyn Fn() -> String>,
}

/// How `navigate` goes somewhere.
#[derive(Clone, Default)]
pub struct NavOpts {
    /// Keep the history as it is (back / forward).
    pub no_push: bool,
    /// Focus the list afterwards.
    pub focus: bool,
    /// Select these paths (and scroll to the first).
    pub select: Option<Vec<String>>,
    /// Then open Eigenschappen for them.
    pub props: bool,
    /// Another tab than the active one.
    pub tab: Option<u64>,
}

/// Options of `toast`.
#[derive(Default)]
pub struct ToastOpts {
    pub message: String,
    pub severity: Option<Severity>,
    pub action: Option<(String, Rc<dyn Fn()>)>,
    /// ms; 0 is the default 5000.
    pub timeout: u32,
}

/// What a window was opened with (main.rs parses argv into this; the WebKit app's URL params).
#[derive(Clone, Default, Debug)]
pub struct Params {
    pub path: Option<String>,
    pub place: Option<String>,
    /// One or more names, paths or file:// URIs to select (a string or an array).
    pub select: Option<Value>,
    /// More folders, one tab each: [path | {path, select?}].
    pub tabs: Option<Value>,
    pub properties: bool,
    pub tab_ticket: Option<String>,
    pub picker: Option<Value>,
    /// Screenshot helpers: menu | menu-background | options | share | drive | trash.
    pub open: Option<String>,
    /// The tabs of a window before a restart ({version: 1, tabs, active}, transfer.rs take_restart_sessions):
    /// they come back instead of the saved session.
    pub session: Option<Value>,
}

impl Params {
    /// Whether the launch named a place (else a saved session may come back).
    pub fn explicit(&self) -> bool {
        self.path.is_some()
            || self.place.is_some()
            || self.tabs.is_some()
            || self.select.is_some()
            || self.properties
            || self.tab_ticket.is_some()
    }
}

/// What typed address text means.
pub enum Resolved {
    /// Go there.
    Go(Loc),
    /// Taken care of (a file opened, a web address handed on).
    Handled,
    /// Nothing there: "Kan ... niet vinden".
    NotFound,
}

/// A sub folder (address bar, navigation pane).
#[derive(Clone)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    pub icon: String,
}

pub(crate) struct Inner {
    pub(crate) ctx: Rc<kit::Ctx>,
    pub(crate) win: AppWindow,
    s: RefCell<Shared>,
    clip_rev: Cell<Option<u64>>,
    tabs: RefCell<Vec<Rc<RefCell<Tab>>>>,
    active: Cell<u64>,
    seq: Cell<u64>,
    view: RefCell<Option<FileView>>,
    page_items: RefCell<Vec<(Item, glib::WeakRef<gtk::Widget>)>>,
    collapsed_secs: RefCell<HashSet<String>>,
    group_collapse: RefCell<HashMap<String, Rc<RefCell<HashSet<String>>>>>,
    watched: RefCell<HashSet<String>>,
    undo: RefCell<Vec<Undo>>,
    dir_cache: RefCell<HashMap<String, (f64, Vec<DirEntry>)>>,
    reload_soon: RefCell<HashMap<u64, glib::SourceId>>,
    render_after_rename: Cell<bool>,
    loading_timer: RefCell<Option<glib::SourceId>>,
    listeners: RefCell<Vec<ListenerId>>,
    picker: RefCell<Option<Picker>>,
    started: Cell<bool>,
    key_in_flight: Cell<bool>,
    vol_busy: Cell<bool>,
    vol_at: Cell<f64>,
    vol_waiters: RefCell<Vec<Box<dyn FnOnce(&Explorer)>>>,
    dry_shown: Cell<bool>,
    closed: Cell<bool>,
    nav_width: Cell<i32>,
    narrow: Cell<u8>,
    // widgets
    pub(crate) root: gtk::Box,
    pub(crate) tab_strip: TabStrip,
    pub(crate) address: AddressRow,
    pub(crate) cmdbar: CommandBar,
    pub(crate) nav: NavPane,
    pub(crate) details: DetailsPane,
    infobars: gtk::Box,
    content_host: gtk::Box,
    status_left: gtk::Box,
    st_details: gtk::Button,
    st_large: gtk::Button,
    splitter: gtk::Box,
    footer: gtk::Box,
}

/// One Explorer window; a cheap handle (clone).
#[derive(Clone)]
pub struct Explorer {
    pub(crate) inner: Rc<Inner>,
}

/// A weak handle (closures in widgets hold this).
#[derive(Clone)]
pub struct WeakExplorer(Weak<Inner>);

impl WeakExplorer {
    pub fn upgrade(&self) -> Option<Explorer> {
        self.0.upgrade().map(|inner| Explorer { inner })
    }
}

thread_local! {
    static OPEN: RefCell<Vec<Explorer>> = const { RefCell::new(Vec::new()) };
}

impl Explorer {
    // =========================================================================================== frame

    /// A new window (shown at once; it fills in as the backend answers).
    pub fn new(params: Params) -> Explorer {
        let ctx = kit::ctx();
        // a picker (picker.rs) remembers its own size, 980 x 680 at first like the WebKit picker
        let picker = params.picker.is_some();
        let win = AppWindow::new(WindowOptions {
            key: if picker { "picker" } else { "main" }.into(),
            icon: "-".into(),
            width: if picker { 980 } else { 1100 },
            height: if picker { 680 } else { 720 },
            min_width: 500,
            min_height: 360,
            titlebar_height: 40,
            ..Default::default()
        });
        win.window().add_css_class("xp-window");
        let inner = Rc::new_cyclic(|weak: &Weak<Inner>| {
            let wx = WeakExplorer(weak.clone());
            Inner {
                ctx: ctx.clone(),
                win: win.clone(),
                s: RefCell::new(Shared::default()),
                clip_rev: Cell::new(None),
                tabs: RefCell::new(Vec::new()),
                active: Cell::new(0),
                seq: Cell::new(0),
                view: RefCell::new(None),
                page_items: RefCell::new(Vec::new()),
                collapsed_secs: RefCell::new(HashSet::new()),
                group_collapse: RefCell::new(HashMap::new()),
                watched: RefCell::new(HashSet::new()),
                undo: RefCell::new(Vec::new()),
                dir_cache: RefCell::new(HashMap::new()),
                reload_soon: RefCell::new(HashMap::new()),
                render_after_rename: Cell::new(false),
                loading_timer: RefCell::new(None),
                listeners: RefCell::new(Vec::new()),
                picker: RefCell::new(None),
                started: Cell::new(false),
                key_in_flight: Cell::new(false),
                vol_busy: Cell::new(false),
                vol_at: Cell::new(0.0),
                vol_waiters: RefCell::new(Vec::new()),
                dry_shown: Cell::new(false),
                closed: Cell::new(false),
                nav_width: Cell::new(248),
                narrow: Cell::new(0),
                root: gtk::Box::new(gtk::Orientation::Vertical, 0),
                tab_strip: TabStrip::new(wx.clone()),
                address: AddressRow::new(wx.clone()),
                cmdbar: CommandBar::new(wx.clone()),
                nav: NavPane::new(wx.clone()),
                details: DetailsPane::new(wx.clone()),
                infobars: gtk::Box::new(gtk::Orientation::Vertical, 6),
                content_host: gtk::Box::new(gtk::Orientation::Vertical, 0),
                status_left: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                st_details: gtk::Button::new(),
                st_large: gtk::Button::new(),
                splitter: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                footer: gtk::Box::new(gtk::Orientation::Vertical, 0),
            }
        });
        let x = Explorer { inner };
        x.build();
        OPEN.with(|o| o.borrow_mut().push(x.clone()));
        x.install_events();
        x.install_keys();
        if let Some(options) = &params.picker {
            crate::picker::attach(&x, options);
        }
        win.present();
        let me = x.clone();
        glib::spawn_future_local(async move { me.start(params).await });
        x
    }

    fn build(&self) {
        let i = &self.inner;
        let win = &i.win;
        // tabs in the title bar instead of the icon and title
        let blank = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        win.set_title_widget(Some(&blank));
        let strip = i.tab_strip.widget();
        win.set_titlebar_content(Some(strip));
        if let Some(center) = strip.parent() {
            center.set_valign(gtk::Align::Fill);
        }
        win.set_title(s("appName"));

        let root = &i.root;
        root.add_css_class("xp");
        let toolbar = gtk::Box::new(gtk::Orientation::Vertical, 0);
        toolbar.add_css_class("xp-toolbar");
        toolbar.append(i.address.widget());
        toolbar.append(i.cmdbar.widget());
        root.append(&toolbar);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("xp-body");
        body.set_vexpand(true);
        body.append(i.nav.widget());
        i.splitter.add_css_class("xp-splitter");
        i.splitter.set_size_request(5, -1);
        i.splitter.set_hexpand(false);
        i.splitter.set_cursor_from_name(Some("col-resize"));
        i.splitter.set_focusable(true);
        i.splitter.update_property(&[gtk::accessible::Property::Label(s("splitter"))]);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.add_css_class("xp-splitter-line");
        line.set_size_request(1, -1);
        line.set_halign(gtk::Align::Center);
        line.set_hexpand(true);
        i.splitter.append(&line);
        body.append(&i.splitter);
        let center = gtk::Box::new(gtk::Orientation::Vertical, 0);
        center.add_css_class("xp-center");
        center.set_hexpand(true);
        i.infobars.add_css_class("xp-infobars");
        center.append(&i.infobars);
        i.content_host.add_css_class("xp-main");
        i.content_host.set_vexpand(true);
        i.content_host.set_hexpand(true);
        center.append(&i.content_host);
        body.append(&center);
        body.append(i.details.widget());
        i.details.widget().set_visible(false);
        root.append(&body);

        let status = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status.add_css_class("xp-status");
        status.set_widget_name("explorer-status");
        i.status_left.add_css_class("xp-status-text");
        i.status_left.set_hexpand(true);
        status.append(&i.status_left);
        let btns = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        for (b, glyph, name) in [(&i.st_details, "list", "explorer-status-details"), (&i.st_large, "grid", "explorer-status-large")] {
            b.add_css_class("xp-status-btn");
            b.set_child(Some(&kit::icons::glyph(glyph, 14)));
            b.set_widget_name(name);
            b.set_focus_on_click(false);
            btns.append(b);
        }
        let w = self.weak();
        i.st_details.connect_clicked(move |_| {
            if let Some(x) = w.upgrade() {
                x.set_view("details");
            }
        });
        let w = self.weak();
        i.st_large.connect_clicked(move |_| {
            if let Some(x) = w.upgrade() {
                x.set_view("large");
            }
        });
        status.append(&btns);
        root.append(&status);
        i.footer.add_css_class("xp-footer-slot");
        root.append(&i.footer);
        win.set_content(root);

        // the splitter
        let drag = gtk::GestureDrag::new();
        let w0 = Rc::new(Cell::new(0));
        let w = self.weak();
        let w0b = w0.clone();
        drag.connect_drag_begin(move |g, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            if let Some(x) = w.upgrade() {
                w0b.set(x.inner.nav.widget().width());
            }
        });
        let w = self.weak();
        let w0c = w0.clone();
        drag.connect_drag_update(move |_, dx, _| {
            if let Some(x) = w.upgrade() {
                x.set_nav_width((w0c.get() as f64 + dx).round() as i32);
            }
        });
        let w = self.weak();
        drag.connect_drag_end(move |_, _, _| {
            if let Some(x) = w.upgrade() {
                x.set_cfg("navWidth", json!(x.inner.nav_width.get()));
            }
        });
        i.splitter.add_controller(drag);
        let keys = gtk::EventControllerKey::new();
        let w = self.weak();
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(x) = w.upgrade() else { return glib::Propagation::Proceed };
            let d = match key {
                gdk::Key::Left => -16,
                gdk::Key::Right => 16,
                _ => return glib::Propagation::Proceed,
            };
            x.set_nav_width(x.inner.nav_width.get() + d);
            x.set_cfg("navWidth", json!(x.inner.nav_width.get()));
            glib::Propagation::Stop
        });
        i.splitter.add_controller(keys);

        // narrow windows (explorer.css container queries)
        let probe = gtk::DrawingArea::new();
        probe.set_can_target(false);
        probe.set_halign(gtk::Align::Fill);
        probe.set_valign(gtk::Align::Fill);
        let w = self.weak();
        probe.connect_resize(move |_, width, _| {
            if let Some(x) = w.upgrade() {
                x.responsive(width);
            }
        });
        win.layer().add_overlay(&probe);
    }

    fn set_nav_width(&self, w: i32) {
        let w = w.clamp(160, 480);
        self.inner.nav_width.set(w);
        self.inner.nav.widget().set_size_request(w, -1);
    }

    /// Container-query steps: <860 smaller search, <700 no command texts and no details pane, <560 no nav.
    fn responsive(&self, width: i32) {
        let level = if width < 560 {
            3
        } else if width < 700 {
            2
        } else if width < 860 {
            1
        } else {
            0
        };
        if level == self.inner.narrow.get() {
            return;
        }
        self.inner.narrow.set(level);
        let root = &self.inner.root;
        for (cls, on) in [("xp-narrow-860", level >= 1), ("xp-narrow-700", level >= 2), ("xp-narrow-560", level >= 3)] {
            if on {
                root.add_css_class(cls);
            } else {
                root.remove_css_class(cls);
            }
        }
        self.inner.address.set_narrow(level);
        self.inner.cmdbar.set_narrow(level >= 2);
        self.apply_layout();
    }


    pub fn weak(&self) -> WeakExplorer {
        WeakExplorer(Rc::downgrade(&self.inner))
    }

    /// Every open Explorer window of this process.
    pub fn all() -> Vec<Explorer> {
        OPEN.with(|o| o.borrow().clone())
    }

    /// The Explorer of the active window.
    pub fn active() -> Option<Explorer> {
        let all = Explorer::all();
        all.iter().find(|x| x.inner.win.window().is_active()).cloned().or_else(|| all.last().cloned())
    }

    // ============================================================================== public accessors

    pub fn ctx(&self) -> Rc<kit::Ctx> {
        self.inner.ctx.clone()
    }

    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn window(&self) -> &AppWindow {
        &self.inner.win
    }

    /// A widget in this window (parent for dialogs, toasts and menus).
    pub fn anchor(&self) -> gtk::Widget {
        self.inner.root.clone().upcast()
    }

    /// The machine state of this window (places, home, user, drives, clipboard, Prullenbak).
    pub fn state(&self) -> std::cell::Ref<'_, Shared> {
        self.inner.s.borrow()
    }

    /// An explorer.* setting (Null when unset).
    pub fn cfg(&self, key: &str) -> Value {
        self.inner.ctx.setting(&format!("explorer.{key}"))
    }

    pub fn cfg_bool(&self, key: &str, def: bool) -> bool {
        match self.cfg(key) {
            Value::Null => def,
            Value::Bool(b) => b,
            Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
            Value::String(s) => !s.is_empty(),
            _ => true,
        }
    }

    pub fn cfg_str(&self, key: &str, def: &str) -> String {
        self.cfg(key).as_str().filter(|s| !s.is_empty()).unwrap_or(def).to_owned()
    }

    /// Change an explorer.* setting (live; errors go to stderr).
    pub fn set_cfg(&self, key: &str, v: Value) {
        let k = key.to_owned();
        self.inner.ctx.set_setting_then(&format!("explorer.{key}"), v, move |r| {
            if let Err(e) = r {
                eprintln!("win11-gtk-explorer: setting {k} failed: {e}");
            }
        });
    }

    /// Change any setting; a failure shows as an info bar.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn set_setting(&self, path: &str, v: Value) {
        let w = self.weak();
        self.inner.ctx.set_setting_then(path, v, move |r| {
            if let (Err(e), Some(x)) = (r, w.upgrade()) {
                x.show_error(&e.to_string(), None);
            }
        });
    }

    /// privacy.recentFiles (Recent on Start).
    pub fn privacy_recent(&self) -> bool {
        self.inner.ctx.setting("privacy.recentFiles") != Value::Bool(false)
    }

    /// A backend call whose answer runs with this window (dropped when it closed).
    pub fn call(&self, method: &str, args: Value, done: impl FnOnce(&Explorer, Result<Value, CallError>) + 'static) {
        let w = self.weak();
        self.inner.ctx.call(method, args, move |r| {
            if let Some(x) = w.upgrade() {
                done(&x, r);
            }
        });
    }

    /// The active tab's id.
    pub fn active_id(&self) -> u64 {
        self.inner.active.get()
    }

    /// The tab ids in strip order.
    pub fn tab_ids(&self) -> Vec<u64> {
        self.inner.tabs.borrow().iter().map(|t| t.borrow().id).collect()
    }

    /// A tab by id.
    pub fn tab(&self, id: u64) -> Option<Rc<RefCell<Tab>>> {
        self.inner.tabs.borrow().iter().find(|t| t.borrow().id == id).cloned()
    }

    /// The active tab.
    pub fn active_tab(&self) -> Option<Rc<RefCell<Tab>>> {
        self.tab(self.inner.active.get())
    }

    /// The active tab's location (Start when there is none yet).
    pub fn loc(&self) -> Loc {
        self.active_tab().map(|t| t.borrow().loc.clone()).unwrap_or(Loc::Home)
    }

    /// The active tab's listing (empty for pages).
    pub fn active_items(&self) -> Vec<Item> {
        self.active_tab().and_then(|t| t.borrow().items.clone()).unwrap_or_default()
    }

    /// The current file view (folder, Prullenbak, search), None on pages.
    pub fn view(&self) -> Option<FileView> {
        self.inner.view.borrow().clone()
    }

    /// The folder of the active tab (None outside a folder).
    pub fn cur_dir(&self) -> Option<String> {
        match self.loc() {
            Loc::Folder(p) => Some(p),
            _ => None,
        }
    }

    /// The area above the content for info bars and progress cards.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn infobars(&self) -> &gtk::Box {
        &self.inner.infobars
    }

    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn details(&self) -> &DetailsPane {
        &self.inner.details
    }

    pub fn is_picker(&self) -> bool {
        self.inner.picker.borrow().is_some()
    }

    /// Turn this window into the file picker (picker.rs).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn set_picker(&self, p: Picker) {
        *self.inner.picker.borrow_mut() = Some(p);
        self.inner.root.add_css_class("xp-picker");
        self.update_chrome();
        self.render_content(false, true);
    }

    /// A widget under the status bar (the picker's footer); None removes it.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn set_footer(&self, w: Option<&gtk::Widget>) {
        kit::controls::clear(&self.inner.footer);
        if let Some(w) = w {
            self.inner.footer.append(w);
        }
    }

    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn push_undo(&self, label: &str, run: impl Fn(&Explorer) + 'static) {
        self.inner.undo.borrow_mut().push(Undo { label: label.to_owned(), run: Rc::new(run) });
    }

    pub fn pop_undo(&self) -> Option<Undo> {
        self.inner.undo.borrow_mut().pop()
    }

    pub fn has_undo(&self) -> bool {
        !self.inner.undo.borrow().is_empty()
    }

    // ===================================================================================== labels

    pub fn place(&self, id: &str) -> Option<Value> {
        self.inner.s.borrow().places.get(id).cloned()
    }

    /// The path of a place ("" when unknown).
    pub fn place_path(&self, id: &str) -> String {
        self.place(id).map(|p| st(&p, "path").to_owned()).unwrap_or_default()
    }

    fn place_for_path(&self, p: &str) -> Option<Value> {
        self.inner.s.borrow().places.values().find(|pl| !st(pl, "path").is_empty() && st(pl, "path") == p && st(pl, "id") != "root").cloned()
    }

    fn user_label(&self) -> String {
        let s = self.inner.s.borrow();
        for k in ["realName", "name"] {
            if let Some(v) = s.user.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()) {
                return v.to_owned();
            }
        }
        let b = util::basename(&s.home);
        if b.is_empty() || b == "/" { crate::i18n::s("homeFolder").into() } else { b }
    }

    /// The name of a folder: a place, a drive, "Systeemschijf (/)" or its base name.
    pub fn folder_label(&self, p: &str) -> String {
        if let Some(pl) = self.place_for_path(p) {
            let id = st(&pl, "id");
            if id == "home" {
                return self.user_label();
            }
            if PLACE_IDS.contains(&id) {
                return s(id).to_owned();
            }
            let name = st(&pl, "name");
            return if name.is_empty() { util::basename(p) } else { name.to_owned() };
        }
        if let Some(v) = self.inner.s.borrow().volumes.iter().find(|v| st(v, "mount") == p) {
            return util::volume_label(v);
        }
        if p == "/" {
            return format!("{} (/)", s("root"));
        }
        util::basename(p)
    }

    /// A location's name (tab, title, history); `full`: the whole path of a folder.
    pub fn loc_label(&self, loc: &Loc, full: bool) -> String {
        match loc {
            Loc::Home => s("home").into(),
            Loc::Gallery => s("gallery").into(),
            Loc::ThisPc => s("thisPC").into(),
            Loc::Trash => s("trash").into(),
            Loc::Search { path, .. } => f("searchResults", &[("name", &self.folder_label(path))]),
            Loc::Folder(p) => {
                if full {
                    p.clone()
                } else {
                    self.folder_label(p)
                }
            }
        }
    }

    /// The theme icon name of a folder (places, drives, "folder").
    pub fn icon_name_for(&self, p: &str) -> String {
        if let Some(pl) = self.place_for_path(p) {
            let icon = st(&pl, "icon");
            if !icon.is_empty() && !icon.ends_with("-symbolic") {
                return icon.to_owned();
            }
            return match st(&pl, "id") {
                "home" => "user-home",
                "desktop" => "user-desktop",
                "documents" => "folder-documents",
                "downloads" => "folder-download",
                "music" => "folder-music",
                "pictures" => "folder-pictures",
                "videos" => "folder-videos",
                "trash" => "user-trash",
                _ => "folder",
            }
            .into();
        }
        if let Some(v) = self.inner.s.borrow().volumes.iter().find(|v| st(v, "mount") == p) {
            return util::volume_icon(v).into();
        }
        if p == "/" {
            return "drive-harddisk-system".into();
        }
        "folder".into()
    }

    /// The icon of a location (`px` square): accent glyphs for Start, Galerij and searches, theme icons else.
    pub fn loc_icon(&self, loc: &Loc, px: i32) -> gtk::Widget {
        let glyph = |name: &str, cls: &str| -> gtk::Widget {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            b.add_css_class("xp-glyphicon");
            if !cls.is_empty() {
                b.add_css_class(cls);
            }
            b.set_size_request(px, px);
            b.set_hexpand(false);
            let g = kit::icons::glyph(name, if px > 32 { (px as f64 * 0.7).round() as i32 } else { px });
            g.set_hexpand(true);
            b.append(&g);
            b.upcast()
        };
        match loc {
            Loc::Home => glyph("home", "is-home"),
            Loc::Gallery => glyph("image", "is-gallery"),
            Loc::Search { .. } => glyph("search", "is-search"),
            Loc::ThisPc => kit::icons::file_icon("computer", px).upcast(),
            Loc::Trash => kit::icons::file_icon(if self.inner.s.borrow().trash_full { "user-trash-full" } else { "user-trash" }, px).upcast(),
            Loc::Folder(p) => kit::icons::file_icon(&self.icon_name_for(p), px).upcast(),
        }
    }

    /// The breadcrumb segments of a location: the nearest place (or Deze pc > drive) and the folders below.
    pub fn crumbs_for(&self, loc: &Loc) -> Vec<(String, Loc)> {
        let p = match loc {
            Loc::Folder(p) | Loc::Search { path: p, .. } => p.clone(),
            other => return vec![(self.loc_label(other, false), other.clone())],
        };
        let mut out = Vec::new();
        let base = {
            let s = self.inner.s.borrow();
            let mut places: Vec<&Value> = s
                .places
                .values()
                .filter(|pl| {
                    let (id, pp) = (st(pl, "id"), st(pl, "path"));
                    !pp.is_empty() && !["trash", "root", "home"].contains(&id) && util::is_under(&p, pp)
                })
                .collect();
            places.sort_by_key(|pl| std::cmp::Reverse(st(pl, "path").len()));
            let mut base = places.first().map(|pl| st(pl, "path").to_owned());
            if base.is_none() && !s.home.is_empty() && util::is_under(&p, &s.home) {
                base = Some(s.home.clone());
            }
            base
        };
        let base = match base {
            Some(b) => b,
            None => {
                out.push((s("thisPC").to_owned(), Loc::ThisPc));
                let s = self.inner.s.borrow();
                util::volume_for(&p, &s.volumes).map(|v| st(v, "mount").to_owned()).unwrap_or_else(|| "/".into())
            }
        };
        out.push((self.folder_label(&base), Loc::Folder(base.clone())));
        if p != base {
            let rest = if base == "/" { &p[1..] } else { p.get(base.len() + 1..).unwrap_or("") };
            let mut cur = base.clone();
            for seg in rest.split('/').filter(|s| !s.is_empty()) {
                cur = util::join_path(&cur, seg);
                out.push((seg.to_owned(), Loc::Folder(cur.clone())));
            }
        }
        if let Loc::Search { .. } = loc {
            out.push((f("searchResults", &[("name", &self.folder_label(&p))]), loc.clone()));
        }
        out
    }

    /// Up: the folder above, Deze pc above a drive, Start above the pages.
    pub fn parent_loc(&self, loc: &Loc) -> Option<Loc> {
        match loc {
            Loc::Search { path, .. } => Some(Loc::Folder(path.clone())),
            Loc::Trash | Loc::Gallery | Loc::ThisPc => Some(Loc::Home),
            Loc::Home => None,
            Loc::Folder(p) => {
                if p == "/" {
                    return Some(Loc::ThisPc);
                }
                if self.inner.s.borrow().volumes.iter().any(|v| st(v, "mount") == p && p != "/") {
                    return Some(Loc::ThisPc);
                }
                Some(Loc::Folder(util::dirname(p)))
            }
        }
    }

    /// "Opgeslagen op deze pc" / "Opgeslagen op BACKUP".
    pub fn stored_on(&self, p: &str) -> String {
        let s = self.inner.s.borrow();
        match util::volume_for(p, &s.volumes) {
            Some(v) if !(flag(v, "isSystem") || flag(v, "isHome") || st(v, "mount") == "/") => {
                f("savedOn", &[("name", &util::volume_label(v))])
            }
            _ => crate::i18n::s("savedOnPC").into(),
        }
    }

    /// A folder as people say it: "Documenten > Werk", the place name, or the path.
    pub fn friendly_dir(&self, dir: &str) -> String {
        if dir.is_empty() {
            return String::new();
        }
        if let Some(pl) = self.place_for_path(dir) {
            return if st(&pl, "id") == "home" { self.user_label() } else { self.folder_label(dir) };
        }
        let home = self.inner.s.borrow().home.clone();
        if !home.is_empty() && home != "/" && util::is_under(dir, &home) && dir != home {
            let rel = &dir[home.len() + 1..];
            let mut parts = rel.split('/');
            let first = parts.next().unwrap_or("");
            let rest: Vec<&str> = parts.collect();
            let fp = util::join_path(&home, first);
            if self.place_for_path(&fp).is_some() {
                let mut v = vec![self.folder_label(&fp)];
                v.extend(rest.iter().map(|s| s.to_string()));
                return v.join(" \u{203a} ");
            }
            return format!("{} \u{203a} {}", self.user_label(), rel.split('/').collect::<Vec<_>>().join(" \u{203a} "));
        }
        dir.to_owned()
    }

    /// The folder a Prullenbak item came from.
    pub fn orig_dir(item: &Value) -> String {
        let d = st(item, "originalDir");
        if !d.is_empty() {
            return d.to_owned();
        }
        let p = st(item, "originalPath");
        if p.is_empty() { String::new() } else { util::dirname(p) }
    }

    /// The name of an item as shown (drives by label, the extension setting).
    pub fn item_name(&self, item: &Value) -> String {
        if let Some(v) = item.get("volume").filter(|v| v.is_object()) {
            return util::volume_label(v);
        }
        util::display_name(item, self.cfg_bool("showExtensions", true))
    }

    /// Snelle toegang: (path, label, icon name); the special folders until the user changes the list.
    pub fn pinned_list(&self) -> Vec<(String, String, String)> {
        let list: Vec<String> = match self.cfg("pinned") {
            Value::Array(a) => a.iter().filter_map(Value::as_str).filter(|p| !p.is_empty()).map(str::to_owned).collect(),
            _ => PLACE_IDS.iter().map(|id| self.place_path(id)).filter(|p| !p.is_empty()).collect(),
        };
        list.into_iter().map(|p| (p.clone(), self.folder_label(&p), self.icon_name_for(&p))).collect()
    }

    /// Deze pc's folders: (path, label, icon name).
    pub fn default_folders(&self) -> Vec<(String, String, String)> {
        PC_FOLDERS
            .iter()
            .map(|id| self.place_path(id))
            .filter(|p| !p.is_empty())
            .map(|p| (p.clone(), self.folder_label(&p), self.icon_name_for(&p)))
            .collect()
    }

    pub fn is_pinned(&self, path: &str) -> bool {
        self.pinned_list().iter().any(|(p, _, _)| p == path)
    }

    pub fn pin(&self, path: &str) {
        let mut list: Vec<String> = self.pinned_list().into_iter().map(|(p, _, _)| p).collect();
        if !list.iter().any(|p| p == path) {
            list.push(path.to_owned());
            self.set_cfg("pinned", json!(list));
        }
    }

    pub fn unpin(&self, path: &str) {
        let list: Vec<String> = self.pinned_list().into_iter().map(|(p, _, _)| p).filter(|p| p != path).collect();
        self.set_cfg("pinned", json!(list));
    }

    fn favorites(&self) -> Vec<String> {
        self.cfg("favorites").as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default()
    }

    pub fn is_fav(&self, path: &str) -> bool {
        self.favorites().iter().any(|p| p == path)
    }

    pub fn fav(&self, path: &str) {
        let mut f = self.favorites();
        if !f.iter().any(|p| p == path) {
            f.push(path.to_owned());
            self.set_cfg("favorites", json!(f));
        }
    }

    pub fn unfav(&self, path: &str) {
        let f: Vec<String> = self.favorites().into_iter().filter(|p| p != path).collect();
        self.set_cfg("favorites", json!(f));
    }

    /// Favourite paths (Start).
    pub fn favorite_paths(&self) -> Vec<String> {
        self.favorites()
    }

    // ================================================================================ data loading

    fn set_volumes(&self, vols: &Value) {
        let mut list: Vec<Value> =
            vols.as_array().map(|a| a.iter().filter(|v| !st(v, "mount").is_empty() && !flag(v, "hidden")).cloned().collect()).unwrap_or_default();
        list.sort_by(|a, b| {
            flag(b, "isSystem")
                .cmp(&flag(a, "isSystem"))
                .then(flag(a, "removable").cmp(&flag(b, "removable")))
                .then_with(|| util::compare_names(&util::volume_label(a), &util::volume_label(b)))
        });
        self.inner.s.borrow_mut().volumes = list;
    }

    /// storage.volumes (at most once per 4 s); `done` runs when it is in.
    pub fn load_volumes(&self, done: impl FnOnce(&Explorer) + 'static) {
        if !self.inner.vol_busy.get() && kit::format::now_ms() - self.inner.vol_at.get() < 4000.0 {
            done(self);
            return;
        }
        self.inner.vol_waiters.borrow_mut().push(Box::new(done));
        if self.inner.vol_busy.replace(true) {
            return;
        }
        self.call("storage.volumes", json!({}), |x, r| {
            match r {
                Ok(v) => x.set_volumes(&v),
                Err(e) => eprintln!("win11-gtk-explorer: storage.volumes failed: {e}"),
            }
            x.inner.vol_busy.set(false);
            x.inner.vol_at.set(kit::format::now_ms());
            x.inner.nav.render();
            let waiters: Vec<Box<dyn FnOnce(&Explorer)>> = x.inner.vol_waiters.borrow_mut().drain(..).collect();
            for f in waiters {
                f(x);
            }
        });
    }

    /// files.clipboardChanged / clipboardGet answers: newer revisions only (main.js createClipboardGate).
    pub fn apply_clipboard(&self, value: &Value) {
        let raw = value.get("_revision");
        let rev = raw.and_then(Value::as_u64);
        if raw.is_some() && rev.is_none() {
            return;
        }
        match rev {
            Some(r) => {
                if self.inner.clip_rev.get().is_some_and(|newest| r < newest) {
                    return;
                }
                self.inner.clip_rev.set(Some(r));
            }
            None => {
                if self.inner.clip_rev.get().is_some() {
                    return;
                }
            }
        }
        {
            let mut s = self.inner.s.borrow_mut();
            s.clipboard_paths = value["paths"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            s.clipboard_op = value["op"].as_str().map(str::to_owned);
        }
        if let Some(v) = self.view() {
            v.set_cut(self.cut_set());
        }
        self.inner.cmdbar.update();
    }

    fn cut_set(&self) -> HashSet<String> {
        let s = self.inner.s.borrow();
        if s.clipboard_op.as_deref() == Some("cut") { s.clipboard_paths.iter().cloned().collect() } else { HashSet::new() }
    }

    pub fn load_clipboard(&self) {
        self.call("files.clipboardGet", json!({}), |x, r| {
            if let Ok(v) = r {
                x.apply_clipboard(&v);
            }
        });
    }

    /// files.trashInfo: the Prullenbak icon (empty / full).
    pub fn load_trash_info(&self) {
        self.call("files.trashInfo", json!({}), |x, r| {
            if let Ok(info) = r {
                let full = num(&info, "count") > 0.0;
                if full != x.inner.s.borrow().trash_full {
                    x.inner.s.borrow_mut().trash_full = full;
                    x.inner.nav.render();
                    x.inner.tab_strip.render();
                }
            }
        });
    }

    /// The sub folders of a folder (cached 4 s), sorted, hidden ones left out.
    pub fn list_dirs(&self, path: &str, done: impl FnOnce(&Explorer, Result<Vec<DirEntry>, CallError>) + 'static) {
        let now = kit::format::now_ms();
        let hit = self.inner.dir_cache.borrow().get(path).filter(|(at, _)| now - at < 4000.0).map(|(_, l)| l.clone());
        if let Some(list) = hit {
            done(self, Ok(list));
            return;
        }
        let p = path.to_owned();
        self.call("files.list", json!({ "path": path, "showHidden": false }), move |x, r| match r {
            Ok(res) => {
                let mut items: Vec<Item> = res["items"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|it| flag(it, "isDir") && !flag(it, "hidden") && !st(it, "name").starts_with('.'))
                            .map(|it| Rc::new(it.clone()))
                            .collect()
                    })
                    .unwrap_or_default();
                util::sort_items(&mut items, "name", false, None);
                let list: Vec<DirEntry> = items
                    .iter()
                    .map(|it| DirEntry {
                        name: if st(it, "displayName").is_empty() { st(it, "name").to_owned() } else { st(it, "displayName").to_owned() },
                        path: st(it, "path").to_owned(),
                        icon: st(it, "icon").to_owned(),
                    })
                    .collect();
                x.inner.dir_cache.borrow_mut().insert(p, (kit::format::now_ms(), list.clone()));
                done(x, Ok(list));
            }
            Err(e) => done(x, Err(e)),
        });
    }

    pub fn invalidate_dir(&self, path: &str) {
        self.inner.dir_cache.borrow_mut().remove(path);
    }

    /// The newest pictures under a folder (Galerij): breadth first, 40 folders, 3 levels deep.
    pub fn collect_images(&self, root: &str, max: usize, done: impl FnOnce(&Explorer, Vec<Item>) + 'static) {
        let w = self.weak();
        let ctx = self.ctx();
        let root = root.to_owned();
        glib::spawn_future_local(async move {
            let mut out: Vec<Item> = Vec::new();
            let mut queue: std::collections::VecDeque<(String, u32)> = std::collections::VecDeque::from([(root, 0)]);
            let mut dirs = 0;
            while let Some((p, d)) = queue.pop_front() {
                if dirs >= 40 || out.len() >= 4000 {
                    break;
                }
                dirs += 1;
                let Ok(res) = ctx.call_async("files.list", json!({ "path": p, "showHidden": false })).await else { continue };
                for it in res["items"].as_array().into_iter().flatten() {
                    if flag(it, "hidden") || st(it, "name").starts_with('.') {
                        continue;
                    }
                    if flag(it, "isDir") {
                        if d < 3 {
                            queue.push_back((st(it, "path").to_owned(), d + 1));
                        }
                        continue;
                    }
                    if util::is_image(it) {
                        out.push(Rc::new(it.clone()));
                    }
                }
            }
            out.sort_by(|a, b| num(b, "modified").partial_cmp(&num(a, "modified")).unwrap_or(std::cmp::Ordering::Equal));
            out.truncate(max);
            if let Some(x) = w.upgrade() {
                done(&x, out);
            }
        });
    }

    // ============================================================================================= tabs

    fn next_id(&self) -> u64 {
        let n = self.inner.seq.get() + 1;
        self.inner.seq.set(n);
        n
    }

    fn new_tab(&self, loc: Loc) -> Rc<RefCell<Tab>> {
        Rc::new(RefCell::new(Tab::new(self.next_id(), loc)))
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.inner.tabs.borrow().iter().position(|t| t.borrow().id == id)
    }

    /// Open a tab right after the active one; returns its id.
    pub fn open_tab(&self, loc: Loc, activate: bool) -> u64 {
        let tab = self.new_tab(loc);
        let id = tab.borrow().id;
        let at = self.index_of(self.active_id()).map(|i| i + 1).unwrap_or(self.inner.tabs.borrow().len());
        self.save_scroll_active();
        self.inner.tabs.borrow_mut().insert(at, tab);
        if activate {
            self.inner.active.set(id);
        }
        self.load(id, false);
        self.inner.tab_strip.render();
        if activate {
            self.render_content(true, false);
            self.update_chrome();
        }
        self.sync_watches();
        id
    }

    /// Close a tab (the window with the last one).
    pub fn close_tab(&self, id: u64) {
        let Some(i) = self.index_of(id) else { return };
        if self.inner.tabs.borrow().len() == 1 {
            self.inner.win.close();
            return;
        }
        let removed = self.inner.tabs.borrow_mut().remove(i);
        removed.borrow_mut().generation += 1;
        if let Some(src) = self.inner.reload_soon.borrow_mut().remove(&id) {
            src.remove();
        }
        if self.active_id() == id {
            let len = self.inner.tabs.borrow().len();
            let next = self.inner.tabs.borrow()[i.min(len - 1)].borrow().id;
            self.inner.active.set(next);
            self.render_content(false, false);
            self.update_chrome();
        }
        self.inner.tab_strip.render();
        self.sync_watches();
    }

    pub fn activate_tab(&self, id: u64) {
        if id == self.active_id() || self.tab(id).is_none() {
            return;
        }
        self.save_scroll_active();
        self.inner.active.set(id);
        let idle = self.tab(id).is_some_and(|t| t.borrow().status == Status::Idle);
        if idle {
            self.load(id, false);
        }
        self.render_content(false, false);
        self.update_chrome();
        self.inner.tab_strip.render();
    }

    /// Ctrl+Tab / Ctrl+Shift+Tab.
    pub fn cycle_tab(&self, delta: i64) {
        let ids = self.tab_ids();
        if ids.is_empty() {
            return;
        }
        let i = ids.iter().position(|t| *t == self.active_id()).unwrap_or(0) as i64;
        let n = ids.len() as i64;
        self.activate_tab(ids[((i + delta) % n + n) as usize % ids.len()]);
    }

    /// The semantic state of a tab for another window or the session (no listings).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn export_tab(&self, id: u64) -> Result<Value, String> {
        let tab = self.tab(id).ok_or_else(|| s("tabGone").to_owned())?;
        if id == self.active_id() && self.view().is_some_and(|v| v.renaming()) {
            return Err(s("tabRenaming").into());
        }
        if id == self.active_id() {
            self.save_scroll_active();
        }
        let v = tab.borrow().state();
        Ok(v)
    }

    /// The state of every tab (at most 20) and the active index: {version: 1, tabs, active}.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn session_state(&self) -> Value {
        self.save_scroll_active();
        let tabs = self.inner.tabs.borrow();
        let mut chosen: Vec<&Rc<RefCell<Tab>>> = tabs.iter().take(20).collect();
        let active = self.active_id();
        if !chosen.iter().any(|t| t.borrow().id == active)
            && let Some(a) = tabs.iter().find(|t| t.borrow().id == active)
            && let Some(last) = chosen.last_mut()
        {
            *last = a;
        }
        let idx = chosen.iter().position(|t| t.borrow().id == active).unwrap_or(0);
        json!({ "version": 1, "tabs": chosen.iter().map(|t| t.borrow().state()).collect::<Vec<_>>(), "active": idx })
    }

    /// Put a tab from another window (or the session) in this one, before `before` (None: at the end), make
    /// it active and load it; `done(Ok(id))` once it loaded, Err when it could not (it is taken out again).
    /// A pristine placeholder tab (the window was opened for this) is replaced.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn import_tab(&self, state: &Value, before: Option<u64>, done: impl FnOnce(&Explorer, Result<u64, String>) + 'static) {
        let id = self.next_id();
        let Some(mut tab) = Tab::from_state(id, state) else {
            done(self, Err(s("tabInvalid").into()));
            return;
        };
        tab.want_focus = true;
        let prev = self.active_id();
        self.save_scroll_active();
        let tab = Rc::new(RefCell::new(tab));
        {
            let mut tabs = self.inner.tabs.borrow_mut();
            let at = before.and_then(|b| tabs.iter().position(|t| t.borrow().id == b)).unwrap_or(tabs.len());
            tabs.insert(at, tab);
        }
        self.inner.active.set(id);
        self.render_content(false, false);
        self.update_chrome();
        self.inner.tab_strip.render();
        self.sync_watches();
        self.load_then(id, false, move |x| {
            let ok = x.tab(id).map(|t| t.borrow().status != Status::Error).unwrap_or(false);
            if !ok {
                let msg = x.tab(id).and_then(|t| t.borrow().error.clone()).unwrap_or_else(|| s("tabLoadFailed").into());
                x.rollback_imported_tab(id, prev);
                done(x, Err(msg));
                return;
            }
            // the placeholder tab a ticket window opened with goes
            let placeholder: Vec<u64> = x.inner.tabs.borrow().iter().filter(|t| t.borrow().placeholder).map(|t| t.borrow().id).collect();
            for p in placeholder {
                if let Some(i) = x.index_of(p) {
                    x.inner.tabs.borrow_mut().remove(i);
                }
            }
            x.render_content(false, false);
            x.update_chrome();
            x.inner.tab_strip.render();
            x.sync_watches();
            done(x, Ok(id));
        });
    }

    /// Take an imported tab out again (its transfer failed) and go back to `prev`.
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn rollback_imported_tab(&self, id: u64, prev: u64) {
        let Some(i) = self.index_of(id) else { return };
        let t = self.inner.tabs.borrow_mut().remove(i);
        t.borrow_mut().generation += 1;
        if self.inner.tabs.borrow().is_empty() {
            let home = self.new_tab(Loc::Home);
            self.inner.tabs.borrow_mut().push(home);
        }
        let next = if self.tab(prev).is_some() {
            prev
        } else {
            let tabs = self.inner.tabs.borrow();
            tabs[i.min(tabs.len() - 1)].borrow().id
        };
        self.inner.active.set(next);
        if self.tab(next).is_some_and(|t| t.borrow().status == Status::Idle) {
            self.load(next, false);
        }
        self.render_content(false, false);
        self.update_chrome();
        self.inner.tab_strip.render();
        self.sync_watches();
    }

    /// Move a tab before another (None: to the end).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn move_tab_before(&self, id: u64, before: Option<u64>) {
        if Some(id) == before {
            return;
        }
        let Some(from) = self.index_of(id) else { return };
        let t = self.inner.tabs.borrow_mut().remove(from);
        {
            let mut tabs = self.inner.tabs.borrow_mut();
            let at = before.and_then(|b| tabs.iter().position(|x| x.borrow().id == b)).unwrap_or(tabs.len());
            tabs.insert(at, t);
        }
        self.inner.tab_strip.render();
    }

    // ======================================================================================= navigation

    fn save_scroll_active(&self) {
        if let (Some(tab), Some(v)) = (self.active_tab(), self.view()) {
            let mut t = tab.borrow_mut();
            let k = t.loc.key();
            t.scroll_memo.insert(k, v.scroll());
        }
    }

    /// Go somewhere in a tab (the active one by default).
    pub fn navigate(&self, loc: Loc, o: NavOpts) {
        let id = o.tab.unwrap_or(self.active_id());
        let Some(tab) = self.tab(id) else { return };
        let loc = match loc {
            Loc::Folder(p) => Loc::Folder(util::norm_path(&p)),
            l => l,
        };
        if id == self.active_id() {
            self.save_scroll_active();
        }
        {
            let mut t = tab.borrow_mut();
            let prev = t.loc.clone();
            if !o.no_push && t.history.get(t.h_index).map(Loc::key) != Some(loc.key()) {
                let keep = t.h_index + 1;
                t.history.truncate(keep);
                t.history.push(loc.clone());
                if t.history.len() > 60 {
                    t.history.remove(0);
                }
                t.h_index = t.history.len() - 1;
            }
            t.loc = loc.clone();
            t.filter.clear();
            t.selection.clear();
            t.focus = None;
            t.items = None;
            t.error = None;
            t.want_focus = o.focus;
            t.placeholder = false;
            // going up: select the folder we came from (Windows does this)
            t.pending_select = o.select.clone().or_else(|| match (&prev, &loc) {
                (Loc::Folder(a), Loc::Folder(b)) if util::dirname(a.as_str()) == b.as_str() && a.as_str() != b.as_str() => {
                    Some(vec![a.to_string()])
                }
                _ => None,
            });
            t.pending_props = o.props && o.select.is_some();
            t.pending_reveal = None;
            t.pending_props_items = None;
        }
        self.load(id, false);
        if id == self.active_id() {
            self.render_content(true, false);
            self.update_chrome();
        }
        self.inner.tab_strip.render();
        self.sync_watches();
    }

    pub fn go_to(&self, loc: Loc) {
        self.navigate(loc, NavOpts::default());
    }



    /// Back (-1) / forward (+1) through the history, any number of steps.
    pub fn go(&self, delta: i64) {
        let Some(tab) = self.active_tab() else { return };
        let (target, select) = {
            let mut t = tab.borrow_mut();
            let idx = t.h_index as i64 + delta;
            if idx < 0 || idx >= t.history.len() as i64 {
                return;
            }
            t.h_index = idx as usize;
            let target = t.history[t.h_index].clone();
            let select = match (&t.loc, &target) {
                (Loc::Folder(a), Loc::Folder(b)) if util::dirname(a.as_str()) == b.as_str() => Some(vec![a.to_string()]),
                _ => None,
            };
            (target, select)
        };
        self.navigate(target, NavOpts { no_push: true, focus: true, select, ..Default::default() });
    }

    pub fn back(&self) {
        self.go(-1);
    }

    pub fn forward(&self) {
        self.go(1);
    }

    pub fn up(&self) {
        if let Some(p) = self.parent_loc(&self.loc()) {
            self.navigate(p, NavOpts { focus: true, ..Default::default() });
        }
    }

    /// F5: forget caches, ask for drives and the Prullenbak again, reload the tab.
    pub fn refresh(&self) {
        self.inner.dir_cache.borrow_mut().clear();
        self.load_volumes(|_| {});
        self.load_trash_info();
        self.reload(self.active_id(), 0);
    }

    /// The history menu of Back (-1) or Forward (+1): (label, location, steps).
    pub fn history_items(&self, dir: i64) -> Vec<(String, Loc, i64)> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        let t = tab.borrow();
        let mut out = Vec::new();
        if dir < 0 {
            let mut i = t.h_index as i64 - 1;
            while i >= 0 && out.len() < 12 {
                out.push(i);
                i -= 1;
            }
        } else {
            let mut i = t.h_index + 1;
            while i < t.history.len() && out.len() < 12 {
                out.push(i as i64);
                i += 1;
            }
        }
        out.into_iter().map(|i| (self.loc_label(&t.history[i as usize], false), t.history[i as usize].clone(), i - t.h_index as i64)).collect()
    }

    /// Recente locaties (the address bar's history button): every tab's history, newest first.
    pub fn recent_locations(&self) -> Vec<Loc> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut tabs: Vec<Rc<RefCell<Tab>>> = self.active_tab().into_iter().collect();
        tabs.extend(self.inner.tabs.borrow().iter().cloned());
        for tab in tabs {
            let t = tab.borrow();
            for l in t.history.iter().rev() {
                if seen.insert(l.key()) {
                    out.push(l.clone());
                }
            }
        }
        out.truncate(12);
        out
    }

    // ============================================================================================ load

    fn load(&self, id: u64, keep: bool) {
        self.load_then(id, keep, |_| {});
    }

    /// Load a tab's listing; `then` runs when it is in (or failed).
    fn load_then(&self, id: u64, keep: bool, then: impl FnOnce(&Explorer) + 'static) {
        let Some(tab) = self.tab(id) else { return };
        let (generation, loc) = {
            let mut t = tab.borrow_mut();
            t.generation += 1;
            t.reload_after_rename = false;
            (t.generation, t.loc.clone())
        };
        if !loc.has_listing() {
            let mut t = tab.borrow_mut();
            t.items = None;
            t.by_path.clear();
            t.status = Status::Ready;
            drop(t);
            then(self);
            return;
        }
        let path = if loc == Loc::Trash { self.place_path("trash") } else { loc.path().unwrap_or("").to_owned() };
        if path.is_empty() {
            {
                let mut t = tab.borrow_mut();
                t.status = Status::Error;
                t.error = Some(s("noTrash").into());
            }
            if id == self.active_id() {
                self.render_content(false, false);
                self.update_chrome();
            }
            then(self);
            return;
        }
        tab.borrow_mut().status = Status::Loading;
        if !keep && id == self.active_id() {
            let w = self.weak();
            if let Some(old) = self.inner.loading_timer.borrow_mut().take() {
                old.remove();
            }
            let src = glib::timeout_add_local_once(Duration::from_millis(220), move || {
                if let Some(x) = w.upgrade() {
                    x.inner.loading_timer.borrow_mut().take();
                    let still = x.tab(id).is_some_and(|t| {
                        let t = t.borrow();
                        t.generation == generation && t.status == Status::Loading
                    });
                    if still && x.active_id() == id {
                        x.render_content(false, false);
                    }
                }
            });
            *self.inner.loading_timer.borrow_mut() = Some(src);
        }
        let ctx = self.ctx();
        let w = self.weak();
        let show_hidden = self.cfg_bool("showHidden", false);
        glib::spawn_future_local(async move {
            let result = match &loc {
                Loc::Search { query, .. } => ctx.call_async("files.find", json!({ "path": path, "query": query, "limit": 500 })).await,
                _ => ctx.call_async("files.list", json!({ "path": path, "showHidden": show_hidden })).await,
            };
            let Some(x) = w.upgrade() else { return };
            let Some(tab) = x.tab(id) else { return };
            let stale = tab.borrow().generation != generation;
            if stale {
                return;
            }
            match result {
                Ok(res) => {
                    let items: Vec<Item> = res["items"].as_array().map(|a| a.iter().map(|v| Rc::new(v.clone())).collect()).unwrap_or_default();
                    let mut t = tab.borrow_mut();
                    t.by_path = items.iter().map(|it| (st(it, "path").to_owned(), it.clone())).collect();
                    t.items = Some(items);
                    t.truncated = matches!(loc, Loc::Search { .. }) && res["truncated"] == true;
                    if !matches!(loc, Loc::Search { .. }) {
                        t.list_path = Some(res["path"].as_str().unwrap_or(&path).to_owned());
                    }
                    t.status = Status::Ready;
                    t.error = None;
                }
                Err(e) => {
                    // a file instead of a folder: open its folder and select it
                    if let (Loc::Folder(_), false) = (&loc, keep) {
                        let props = ctx.call_async("files.properties", json!({ "path": path })).await.ok();
                        let Some(x) = w.upgrade() else { return };
                        let current = tab.borrow().generation == generation;
                        if let Some(p) = props
                            && current
                            && st(&p, "type") != "folder"
                            && p.get("contains").is_none_or(Value::is_null)
                        {
                            {
                                let mut t = tab.borrow_mut();
                                t.loc = Loc::Folder(util::dirname(&path));
                                let hi = t.h_index;
                                let l = t.loc.clone();
                                if let Some(h) = t.history.get_mut(hi) {
                                    *h = l;
                                }
                                t.pending_select = Some(vec![path.clone()]);
                            }
                            x.load_then(id, false, then);
                            if id == x.active_id() {
                                x.update_chrome();
                            }
                            return;
                        }
                    }
                    let mut t = tab.borrow_mut();
                    t.status = Status::Error;
                    t.error = Some(e.message.clone());
                    t.items = None;
                }
            }
            let renaming = x.view().is_some_and(|v| v.renaming());
            if id == x.active_id() && keep && renaming {
                x.inner.render_after_rename.set(true);
                x.update_status();
            } else if id == x.active_id() {
                x.render_content(false, keep);
                x.update_chrome();
            }
            x.inner.tab_strip.render();
            then(&x);
        });
    }

    /// Reload a tab soon (debounced per tab); while a name is being typed, right after the rename.
    pub fn reload(&self, id: u64, delay_ms: u64) {
        if let Some(old) = self.inner.reload_soon.borrow_mut().remove(&id) {
            old.remove();
        }
        let w = self.weak();
        let src = glib::timeout_add_local_once(Duration::from_millis(delay_ms), move || {
            let Some(x) = w.upgrade() else { return };
            x.inner.reload_soon.borrow_mut().remove(&id);
            let Some(tab) = x.tab(id) else { return };
            if id == x.active_id() && x.view().is_some_and(|v| v.renaming()) {
                tab.borrow_mut().reload_after_rename = true;
                return;
            }
            let listing = tab.borrow().loc.has_listing();
            if listing {
                x.load(id, true);
            } else if id == x.active_id() {
                x.render_content(false, true);
            }
        });
        self.inner.reload_soon.borrow_mut().insert(id, src);
    }

    /// Reload the active tab.
    pub fn reload_active(&self, delay_ms: u64) {
        self.reload(self.active_id(), delay_ms);
    }

    /// The rename box closed: run what waited for it.
    fn rename_ended(&self, id: u64) {
        let postponed = self.inner.render_after_rename.replace(false);
        let Some(tab) = self.tab(id) else { return };
        let again = std::mem::replace(&mut tab.borrow_mut().reload_after_rename, false);
        if again {
            self.reload(id, 0);
        } else if postponed && id == self.active_id() && !self.inner.reload_soon.borrow().contains_key(&id) {
            self.render_content(false, true);
        }
    }

    /// files.watch the folders of all tabs, files.unwatch the others.
    fn sync_watches(&self) {
        let want: HashSet<String> = self
            .inner
            .tabs
            .borrow()
            .iter()
            .filter_map(|t| match &t.borrow().loc {
                Loc::Folder(p) if p.starts_with('/') => Some(p.clone()),
                _ => None,
            })
            .collect();
        let have = self.inner.watched.borrow().clone();
        for p in want.difference(&have) {
            self.inner.watched.borrow_mut().insert(p.clone());
            self.inner.ctx.call("files.watch", json!({ "path": p }), |_| {});
        }
        for p in have.difference(&want) {
            self.inner.watched.borrow_mut().remove(p);
            self.inner.ctx.call("files.unwatch", json!({ "path": p }), |_| {});
        }
    }

    // ========================================================================================== render

    fn columns_for(&self, kind: &str) -> Vec<Column> {
        let widths = self.cfg("columns");
        let hidden: HashSet<String> =
            self.cfg("hiddenColumns").as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
        let wd = |id: &str, def: f64| widths.get(id).and_then(Value::as_f64).unwrap_or(def);
        let col = |id: &str, label: &str, width: f64, end: bool, render: Rc<dyn Fn(&Value) -> String>| Column {
            id: id.into(),
            label: label.into(),
            width,
            min: if id == "name" { 120.0 } else { 60.0 },
            align_end: end,
            render,
        };
        let date: Rc<dyn Fn(&Value) -> String> = Rc::new(|it: &Value| util::date_time(num(it, "modified")));
        let size: Rc<dyn Fn(&Value) -> String> = Rc::new(|it: &Value| util::size_text(it));
        let none: Rc<dyn Fn(&Value) -> String> = Rc::new(|_: &Value| String::new());
        let w = self.weak();
        match kind {
            "trash" => {
                let orig: Rc<dyn Fn(&Value) -> String> =
                    Rc::new(move |it: &Value| w.upgrade().map(|x| x.friendly_dir(&Explorer::orig_dir(it))).unwrap_or_default());
                let deleted: Rc<dyn Fn(&Value) -> String> = Rc::new(|it: &Value| {
                    let d = num(it, "deletedAt");
                    util::date_time(if d > 0.0 { d } else { num(it, "deleted") })
                });
                vec![
                    col("name", s("colName"), wd("name", 280.0), false, none),
                    col("orig", s("colOrig"), wd("orig", 220.0), false, orig),
                    col("deleted", s("colDeleted"), wd("deleted", 150.0), false, deleted),
                    col("size", s("colSize"), wd("size", 100.0), true, size),
                ]
            }
            "search" => {
                let location: Rc<dyn Fn(&Value) -> String> =
                    Rc::new(move |it: &Value| w.upgrade().map(|x| x.friendly_dir(&util::dirname(st(it, "path")))).unwrap_or_default());
                vec![
                    col("name", s("colName"), wd("name", 280.0), false, none),
                    col("location", s("colLocation"), wd("location", 240.0), false, location),
                    col("modified", s("colModified"), wd("modified", 150.0), false, date),
                    col("size", s("colSize"), wd("size", 100.0), true, size),
                ]
            }
            _ => {
                let mut v = vec![col("name", s("colName"), wd("name", 300.0), false, none)];
                if !hidden.contains("modified") {
                    v.push(col("modified", s("colModified"), wd("modified", 150.0), false, date));
                }
                if !hidden.contains("type") {
                    v.push(col("type", s("colType"), wd("type", 150.0), false, Rc::new(|it: &Value| util::type_name(it))));
                }
                if !hidden.contains("size") {
                    v.push(col("size", s("colSize"), wd("size", 100.0), true, size));
                }
                v
            }
        }
    }

    /// (sort key, descending) for a location kind.
    pub fn sort_state(&self, kind: &str) -> (String, bool) {
        let cols: Vec<String> = self.columns_for(kind).into_iter().map(|c| c.id).collect();
        let mut key = self.cfg_str("sortBy", "name");
        if !cols.contains(&key) && key != "type" {
            key = "name".into();
        }
        (key, self.cfg_bool("sortDesc", false))
    }

    /// The sort keys the Sorteren menu offers: (id, label).
    pub fn sort_keys(&self, kind: &str) -> Vec<(String, String)> {
        let cols = self.columns_for(kind);
        let mut out: Vec<(String, String)> = cols.iter().map(|c| (c.id.clone(), c.label.clone())).collect();
        if kind == "folder" && !cols.iter().any(|c| c.id == "type") {
            out.push(("type".into(), s("colType").into()));
        }
        out
    }

    fn visible_items(&self, tab: &Tab) -> Vec<Item> {
        let show_hidden = self.cfg_bool("showHidden", false);
        let q = tab.filter.to_lowercase();
        let picker = self.inner.picker.borrow().clone();
        let mut items: Vec<Item> = tab
            .items
            .iter()
            .flatten()
            .filter(|x| show_hidden || !(flag(x, "hidden") || st(x, "name").starts_with('.')))
            .filter(|x| {
                q.is_empty() || {
                    let dn = st(x, "displayName");
                    (if dn.is_empty() { st(x, "name") } else { dn }).to_lowercase().contains(&q)
                }
            })
            .filter(|x| picker.as_ref().is_none_or(|p| (p.filter)(x)))
            .cloned()
            .collect();
        let (key, desc) = self.sort_state(tab.loc.kind());
        let getter: Option<Box<dyn Fn(&Value) -> util::SortVal>> = match key.as_str() {
            "orig" => Some(Box::new(|x: &Value| util::SortVal::Text(st(x, "originalPath").to_owned()))),
            "deleted" => Some(Box::new(|x: &Value| {
                let d = num(x, "deletedAt");
                util::SortVal::Num(if d > 0.0 { d } else { num(x, "deleted") })
            })),
            "location" => Some(Box::new(|x: &Value| util::SortVal::Text(util::dirname(st(x, "path"))))),
            _ => None,
        };
        util::sort_items(&mut items, &key, desc, getter.as_deref());
        items
    }

    fn groups_for(&self, items: &[Item]) -> Option<Vec<Group>> {
        let by = self.cfg_str("groupBy", "none");
        if by == "none" || items.is_empty() {
            return None;
        }
        let now = kit::format::now_ms();
        let mut order: Vec<String> = Vec::new();
        let mut map: HashMap<String, (util::GroupKey, Vec<Item>)> = HashMap::new();
        for it in items {
            let g = match by.as_str() {
                "modified" => util::date_group(num(it, "modified"), now),
                "deleted" => {
                    let d = num(it, "deletedAt");
                    util::date_group(if d > 0.0 { d } else { num(it, "deleted") }, now)
                }
                "size" => util::size_group(it),
                "type" => {
                    let n = util::type_name(it);
                    util::GroupKey { key: n.clone(), label: n, order: if flag(it, "isDir") { -1 } else { 0 } }
                }
                "name" => {
                    let dn = st(it, "displayName");
                    let name = if dn.is_empty() { st(it, "name") } else { dn };
                    let c = name.chars().next().map(|c| c.to_uppercase().collect::<String>()).unwrap_or_else(|| "?".into());
                    let k = match c.chars().next() {
                        Some(ch) if ch.is_ascii_uppercase() => c.clone(),
                        Some(ch) if ch.is_ascii_digit() => "0 - 9".into(),
                        _ => "#".into(),
                    };
                    util::GroupKey { key: k.clone(), label: k, order: 0 }
                }
                _ => util::GroupKey { key: String::new(), label: String::new(), order: 0 },
            };
            let e = map.entry(g.key.clone()).or_insert_with(|| {
                order.push(g.key.clone());
                (g.clone(), Vec::new())
            });
            e.1.push(it.clone());
        }
        let mut list: Vec<(util::GroupKey, Vec<Item>)> = order.into_iter().filter_map(|k| map.remove(&k)).collect();
        list.sort_by(|a, b| a.0.order.cmp(&b.0.order).then_with(|| util::compare_names(&a.0.label, &b.0.label)));
        if self.cfg_bool("sortDesc", false) && (by == "name" || by == "type") {
            list.reverse();
        }
        Some(list.into_iter().map(|(g, items)| Group { key: g.key, label: g.label, items }).collect())
    }

    /// Draw the active tab's content again (`keep`: keep the scroll position, a refresh).
    pub fn render_content(&self, animate_in: bool, keep: bool) {
        let _ = animate_in;
        if let Some(t) = self.inner.loading_timer.borrow_mut().take() {
            t.remove();
        }
        self.inner.render_after_rename.set(false);
        let Some(tab) = self.active_tab() else { return };
        let had_focus = self.inner.content_host.focus_child().is_some();
        let prev_scroll = if keep { self.view().map(|v| v.scroll()) } else { None };
        if let Some(v) = self.inner.view.borrow_mut().take() {
            v.destroy();
        }
        self.inner.page_items.borrow_mut().clear();
        let (loc, status, has_items, want_focus) = {
            let t = tab.borrow();
            (t.loc.clone(), t.status, t.items.is_some(), t.want_focus)
        };
        for k in ["home", "gallery", "thispc", "trash", "folder", "search"] {
            self.inner.root.remove_css_class(&format!("xp-kind-{k}"));
        }
        self.inner.root.add_css_class(&format!("xp-kind-{}", loc.kind()));
        let node: gtk::Widget = match loc {
            Loc::Home => pages::home_page(self),
            Loc::ThisPc => pages::this_pc_page(self),
            Loc::Gallery => pages::gallery_page(self),
            _ if status == Status::Error => self.error_view(&tab),
            _ if status == Status::Loading && !has_items => {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
                b.add_css_class("xp-loading");
                let ring = kit::controls::ProgressRing::new(32, None);
                ring.widget().set_halign(gtk::Align::Center);
                ring.widget().set_valign(gtk::Align::Center);
                ring.widget().set_vexpand(true);
                ring.widget().update_property(&[gtk::accessible::Property::Label(s("loading"))]);
                b.append(ring.widget());
                b.upcast()
            }
            _ if !has_items => {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
                b.add_css_class("xp-loading");
                b.upcast()
            }
            _ => self.file_view(&tab, prev_scroll, had_focus || want_focus),
        };
        node.set_vexpand(true);
        node.set_hexpand(true);
        kit::controls::clear(&self.inner.content_host);
        self.inner.content_host.append(&node);
        if matches!(loc, Loc::Home | Loc::ThisPc | Loc::Gallery) && want_focus {
            tab.borrow_mut().want_focus = false;
            let n = node.clone();
            glib::idle_add_local_once(move || {
                n.grab_focus();
            });
        }
        self.repaint_selection();
        self.update_selection_ui();
    }

    fn file_view(&self, tab: &Rc<RefCell<Tab>>, prev_scroll: Option<(f64, f64)>, focus: bool) -> gtk::Widget {
        let loc = tab.borrow().loc.clone();
        let kind = loc.kind();
        let items = self.visible_items(&tab.borrow());
        let cfg_view = self.cfg_str("view", "");
        let mode = if matches!(loc, Loc::Search { .. }) && cfg_view.is_empty() { Mode::Details } else { Mode::parse(if cfg_view.is_empty() { "details" } else { &cfg_view }) };
        let gkey = loc.key();
        let collapsed = self.inner.group_collapse.borrow_mut().entry(gkey).or_insert_with(|| Rc::new(RefCell::new(HashSet::new()))).clone();
        let filter = tab.borrow().filter.clone();
        let empty: Option<gtk::Widget> = if items.is_empty() {
            Some(if !filter.is_empty() || matches!(loc, Loc::Search { .. }) {
                empty_text(s("noMatch"))
            } else if loc == Loc::Trash {
                big_empty(kit::icons::file_icon("user-trash", 64).upcast(), s("trashEmpty"), s("trashEmptyMsg"), None)
            } else {
                empty_text(s("folderEmpty"))
            })
        } else {
            None
        };
        // the selection asked for (Weergeven in map, ShowItems): select them all, focus and scroll to the first
        {
            let mut t = tab.borrow_mut();
            let wanted: Vec<String> = t.pending_select.take().unwrap_or_default().into_iter().filter(|p| t.by_path.contains_key(p)).collect();
            if !wanted.is_empty() {
                t.selection = wanted.iter().cloned().collect();
                t.focus = wanted.first().cloned();
                t.pending_reveal = wanted.first().cloned();
                if t.pending_props {
                    t.pending_props_items = Some(wanted.clone());
                }
            }
            t.pending_props = false;
        }
        let (selection, focus_path) = {
            let t = tab.borrow();
            (t.selection.clone(), t.focus.clone())
        };
        let id = tab.borrow().id;
        let w = self.weak();
        let handlers = Handlers {
            on_open: {
                let w = w.clone();
                Rc::new(move |item| {
                    if let Some(x) = w.upgrade() {
                        x.open_item(item, true);
                    }
                })
            },
            on_middle: {
                let w = w.clone();
                Rc::new(move |item| {
                    if let Some(x) = w.upgrade()
                        && flag(item, "isDir")
                    {
                        x.open_tab(Loc::Folder(st(item, "path").to_owned()), false);
                    }
                })
            },
            on_select: {
                let w = w.clone();
                Rc::new(move |sel, focus| {
                    if let Some(x) = w.upgrade()
                        && let Some(t) = x.tab(id)
                    {
                        {
                            let mut t = t.borrow_mut();
                            t.selection = sel.clone();
                            t.focus = focus;
                        }
                        x.update_selection_ui();
                    }
                })
            },
            on_context: {
                let w = w.clone();
                Rc::new(move |px, py, item| {
                    let Some(x) = w.upgrade() else { return };
                    let Some(v) = x.view() else { return };
                    let canvas = v.canvas();
                    if item.is_some() {
                        let items = x.selected_items();
                        menus::item_menu(&x, items, move |top, list| {
                            menus::show(&canvas, px, py, top, list);
                        });
                    } else {
                        let (top, list) = menus::background_menu(&x);
                        menus::show(&canvas, px, py, top, list);
                    }
                })
            },
            on_rename: {
                let w = w.clone();
                Rc::new(move |item, name, done| match w.upgrade() {
                    Some(x) => ops::rename_item(&x, item, name, done),
                    None => done(Err(String::new())),
                })
            },
            on_rename_end: {
                let w = w.clone();
                Rc::new(move || {
                    if let Some(x) = w.upgrade() {
                        x.rename_ended(id);
                    }
                })
            },
            on_sort: {
                let w = w.clone();
                Rc::new(move |key| {
                    if let Some(x) = w.upgrade() {
                        x.set_sort(key);
                    }
                })
            },
            on_column_resize: {
                let w = w.clone();
                Rc::new(move |id, width| {
                    if let Some(x) = w.upgrade() {
                        let mut c = x.cfg("columns");
                        if !c.is_object() {
                            c = json!({});
                        }
                        c[id] = json!(width.round());
                        x.set_cfg("columns", c);
                    }
                })
            },
            on_header_context: {
                let w = w.clone();
                Rc::new(move |px, py| {
                    if let Some(x) = w.upgrade()
                        && let Some(v) = x.view()
                    {
                        menus::show(&v.canvas(), px, py, Vec::new(), menus::columns_menu(&x));
                    }
                })
            },
            on_zoom: {
                let w = w.clone();
                Rc::new(move |d| {
                    if let Some(x) = w.upgrade() {
                        x.zoom(d);
                    }
                })
            },
        };
        let groups = self.groups_for(&items);
        let w2 = self.weak();
        let view = FileView::new(ViewOpts {
            mode,
            items,
            columns: self.columns_for(kind),
            groups,
            selection,
            focus: focus_path,
            cut: self.cut_set(),
            sort: self.sort_state(kind),
            compact: self.cfg_bool("compact", false),
            checkboxes: self.cfg_bool("checkboxes", false),
            show_ext: self.cfg_bool("showExtensions", true),
            empty,
            collapsed,
            label: self.loc_label(&loc, false),
            src_dir: loc.path().filter(|_| loc.is_folder()).map(str::to_owned),
            draggable: loc != Loc::Trash,
            type_of: Rc::new(|it: &Value| util::type_name(it)),
            size_of: Rc::new(|it: &Value| util::size_text(it)),
            content_props: Rc::new(|it: &Value| {
                let m = num(it, "modified");
                let a = if m > 0.0 { format!("{}: {}", s("colModified"), util::date_time(m)) } else { String::new() };
                let b = if flag(it, "isDir") { String::new() } else { format!("{}: {}", s("colSize"), util::size_text(it)) };
                (a, b)
            }),
            handlers,
        });
        *self.inner.view.borrow_mut() = Some(view.clone());
        crate::dnd::attach_view(self, &view);
        // scroll, reveal, rename and Eigenschappen wait for the first layout of a view that is still current
        let scroll = prev_scroll.or_else(|| tab.borrow().scroll_memo.get(&loc.key()).copied());
        if let Some(sc) = scroll {
            view.restore(sc);
        }
        let reveal = tab.borrow_mut().pending_reveal.take();
        if let Some(p) = &reveal {
            view.reveal(p, focus);
        }
        let v2 = view.clone();
        view.after_layout(move || {
            let Some(x) = w2.upgrade() else { return };
            if !x.view().is_some_and(|cur| cur.same(&v2)) {
                return;
            }
            let Some(tab) = x.tab(id) else { return };
            let rename = {
                let mut t = tab.borrow_mut();
                match t.pending_rename.take() {
                    Some(p) if t.by_path.contains_key(&p) => Some(p),
                    _ => None,
                }
            };
            if let Some(p) = rename {
                v2.start_rename(&p);
            } else if focus {
                v2.focus();
            }
            let props: Vec<Item> = {
                let mut t = tab.borrow_mut();
                let want = t.pending_props_items.take().unwrap_or_default();
                want.iter().filter_map(|p| t.by_path.get(p).cloned()).collect()
            };
            if !props.is_empty() {
                ops::properties(&x, props);
            }
        });
        tab.borrow_mut().want_focus = false;
        // the bar over filtered folders and search results
        let truncated = tab.borrow().truncated;
        let bar: Option<gtk::Box> = match &loc {
            Loc::Search { path, query } => {
                let b = search_bar("search");
                b.append(&bar_text(&format!("{} \u{b7} \u{201c}{}\u{201d}", f("searchResults", &[("name", &self.folder_label(path))]), query)));
                if truncated {
                    let n = kit::controls::label(s("truncated"), "xp-searchbar-note", false);
                    b.append(&n);
                }
                let q = query.clone();
                let link = kit::controls::hyperlink(s("searchEverywhere"), None);
                let w = self.weak();
                link.connect_clicked(move |_| {
                    if let Some(x) = w.upgrade() {
                        x.open_alles(&q);
                    }
                });
                b.append(&link);
                Some(b)
            }
            Loc::Folder(p) if !filter.is_empty() => {
                let b = search_bar("filter");
                b.append(&bar_text(&f("filtered", &[("name", &self.folder_label(p))])));
                let sub = kit::controls::hyperlink(s("searchSub"), None);
                let (w, pp, q) = (self.weak(), p.clone(), filter.clone());
                sub.connect_clicked(move |_| {
                    if let Some(x) = w.upgrade() {
                        x.navigate(Loc::Search { path: pp.clone(), query: q.clone() }, NavOpts { focus: true, ..Default::default() });
                    }
                });
                b.append(&sub);
                let all = kit::controls::hyperlink(s("searchEverywhere"), None);
                let (w, q) = (self.weak(), filter.clone());
                all.connect_clicked(move |_| {
                    if let Some(x) = w.upgrade() {
                        x.open_alles(&q);
                    }
                });
                b.append(&all);
                Some(b)
            }
            _ => None,
        };
        match bar {
            None => view.widget(),
            Some(b) => {
                let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
                wrap.add_css_class("xp-viewwrap");
                wrap.append(&b);
                wrap.append(&view.widget());
                wrap.upcast()
            }
        }
    }

    fn error_view(&self, tab: &Rc<RefCell<Tab>>) -> gtk::Widget {
        let msg = tab.borrow().error.clone().unwrap_or_default();
        let id = tab.borrow().id;
        let icon = kit::icons::glyph("warning", 40);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.set_halign(gtk::Align::Center);
        let again = kit::controls::button(s("tryAgain"), kit::controls::Variant::Accent);
        let w = self.weak();
        again.connect_clicked(move |_| {
            if let Some(x) = w.upgrade() {
                x.load(id, false);
            }
        });
        let home = kit::controls::button(s("goHome"), kit::controls::Variant::Standard);
        let w = self.weak();
        home.connect_clicked(move |_| {
            if let Some(x) = w.upgrade() {
                x.go_to(Loc::Home);
            }
        });
        row.append(&again);
        row.append(&home);
        let b = big_empty(icon.upcast(), s("errOpenFolder"), &msg, Some(row.upcast()));
        b.add_css_class("is-error");
        b
    }

    /// Address row, command bar, navigation pane, title and status after a change.
    pub fn update_chrome(&self) {
        if self.active_tab().is_none() {
            return;
        }
        self.inner.address.update();
        self.inner.cmdbar.update();
        self.inner.nav.mark_selected();
        let title = match self.inner.picker.borrow().as_ref() {
            Some(p) => (p.title)(),
            None => format!("{} - {}", self.loc_label(&self.loc(), false), s("appName")),
        };
        self.inner.win.set_title(&title);
        self.update_status();
    }

    /// The selected items in view order (file view) or as registered (pages).
    pub fn selected_items(&self) -> Vec<Item> {
        let Some(tab) = self.active_tab() else { return Vec::new() };
        let sel = tab.borrow().selection.clone();
        if let Some(v) = self.view() {
            return v.order().into_iter().filter(|it| sel.contains(st(it, "path"))).collect();
        }
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for (it, _) in self.inner.page_items.borrow().iter() {
            let p = st(it, "path");
            if sel.contains(p) && seen.insert(p.to_owned()) {
                out.push(it.clone());
            }
        }
        out
    }

    /// How many items the view shows (None on pages).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn visible_count(&self) -> Option<usize> {
        self.view().map(|v| v.order().len())
    }

    fn update_status(&self) {
        let Some(tab) = self.active_tab() else { return };
        let left = &self.inner.status_left;
        kit::controls::clear(left);
        let sel = self.selected_items();
        let (status, loc) = {
            let t = tab.borrow();
            (t.status, t.loc.clone())
        };
        let count = match self.view() {
            Some(v) => v.order().len(),
            None if matches!(loc, Loc::Folder(_) | Loc::Trash) => self.visible_items(&tab.borrow()).len(),
            None => {
                let set: HashSet<String> = self.inner.page_items.borrow().iter().map(|(it, _)| st(it, "path").to_owned()).collect();
                set.len()
            }
        };
        if status != Status::Error {
            left.append(&kit::controls::label(&n("items", count, &[]), "", false));
        }
        if !sel.is_empty() {
            let sep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            sep.add_css_class("xp-status-sep");
            sep.set_valign(gtk::Align::Center);
            left.append(&sep);
            left.append(&kit::controls::label(&n("selected", sel.len(), &[]), "", false));
            let files: Vec<&Item> = sel.iter().filter(|x| !flag(x, "isDir")).collect();
            if !files.is_empty() {
                let total: f64 = files.iter().map(|x| num(x, "size")).sum();
                let l = kit::controls::label(&util::size_text_n(total), "xp-status-size", false);
                left.append(&l);
            }
        }
        let m = self.cfg_str("view", "details");
        let set = |b: &gtk::Button, on: bool| {
            if on {
                b.add_css_class("is-on");
            } else {
                b.remove_css_class("is-on");
            }
        };
        set(&self.inner.st_details, m == "details");
        set(&self.inner.st_large, m == "large");
        let show = self.view().is_some();
        self.inner.st_details.set_visible(show);
        self.inner.st_large.set_visible(show);
        self.inner.st_details.set_tooltip_text(Some(s("statusDetails")));
        self.inner.st_large.set_tooltip_text(Some(s("statusLarge")));
    }

    /// After every selection change: command bar, picker, status bar, details pane.
    pub fn update_selection_ui(&self) {
        self.inner.cmdbar.update();
        let picker = self.inner.picker.borrow().clone();
        if let Some(p) = picker {
            (p.on_selection)(&self.selected_items());
        }
        self.update_status();
        if self.cfg_bool("detailsPane", false) || self.cfg_bool("previewPane", false) {
            self.inner.details.update();
        }
    }

    /// Paint the selection of the page items (Start, Deze pc, Galerij).
    pub fn repaint_selection(&self) {
        let Some(tab) = self.active_tab() else { return };
        let sel = tab.borrow().selection.clone();
        for (it, w) in self.inner.page_items.borrow().iter() {
            if let Some(w) = w.upgrade() {
                if sel.contains(st(it, "path")) {
                    w.add_css_class("is-selected");
                } else {
                    w.remove_css_class("is-selected");
                }
            }
        }
        self.update_status();
    }

    /// Select these paths (in the view, or on a page).
    pub fn set_selection(&self, paths: &[String], focus: Option<&str>) {
        if let Some(v) = self.view() {
            v.select(paths, focus, false);
            return;
        }
        if let Some(tab) = self.active_tab() {
            let mut t = tab.borrow_mut();
            t.selection = paths.iter().cloned().collect();
            t.focus = focus.map(str::to_owned);
        }
        self.repaint_selection();
        self.update_selection_ui();
    }

    /// A page item (Start, Deze pc, Galerij) with its widget, for the selection and the status bar.
    pub fn register_page_item(&self, item: Item, widget: &impl IsA<gtk::Widget>) {
        self.inner.page_items.borrow_mut().push((item, widget.as_ref().downgrade()));
    }

    /// The registered page items in page order.
    pub fn page_items(&self) -> Vec<(Item, gtk::Widget)> {
        self.inner.page_items.borrow().iter().filter_map(|(it, w)| w.upgrade().map(|w| (it.clone(), w))).collect()
    }

    pub fn is_collapsed(&self, key: &str) -> bool {
        self.inner.collapsed_secs.borrow().contains(key)
    }

    pub fn set_collapsed(&self, key: &str, on: bool) {
        if on {
            self.inner.collapsed_secs.borrow_mut().insert(key.to_owned());
        } else {
            self.inner.collapsed_secs.borrow_mut().remove(key);
        }
    }

    /// Focus the file view, else the first page item.
    pub fn focus_view(&self) {
        if let Some(v) = self.view() {
            v.focus();
        } else if let Some((_, w)) = self.page_items().into_iter().next() {
            w.grab_focus();
        } else {
            self.inner.content_host.child_focus(gtk::DirectionType::TabForward);
        }
    }

    /// Navigation pane, details / preview pane and their widths (settings).
    fn apply_layout(&self) {
        let nav_on = self.cfg_bool("navPane", true) && self.inner.narrow.get() < 3;
        let det_on = self.cfg_bool("detailsPane", false);
        let prev_on = !det_on && self.cfg_bool("previewPane", false);
        let w = self.cfg("navWidth").as_f64().filter(|w| *w > 0.0).unwrap_or(248.0).round() as i32;
        self.set_nav_width(w);
        self.inner.nav.widget().set_visible(nav_on);
        self.inner.splitter.set_visible(nav_on);
        self.inner.details.set_mode(if prev_on { PaneMode::Preview } else { PaneMode::Details });
        let pane = (det_on || prev_on) && self.inner.narrow.get() < 2;
        self.inner.details.widget().set_visible(pane);
        if pane {
            self.inner.details.render();
        }
    }

    // ========================================================================================= messages

    /// A toast at the bottom of the window.
    pub fn toast(&self, title: &str, o: ToastOpts) -> ToastHandle {
        let mut t = Toast::new(title).timeout(if o.timeout == 0 { 5000 } else { o.timeout });
        if !o.message.is_empty() {
            t = t.message(&o.message);
        }
        if let Some(sev) = o.severity {
            t = t.severity(sev);
        }
        if let Some((label, f)) = o.action {
            t = t.action(&label, move || f());
        }
        t.show(&self.inner.root)
    }

    /// An error info bar above the content (at most three; each goes after 15 s).
    pub fn show_error(&self, msg: &str, title: Option<&str>) {
        let bars = &self.inner.infobars;
        let mut count = 0;
        let mut c = bars.first_child();
        while let Some(w) = c {
            if w.has_css_class("w11-infobar") {
                count += 1;
            }
            c = w.next_sibling();
        }
        while count >= 3 {
            let mut c = bars.first_child();
            while let Some(w) = c {
                if w.has_css_class("w11-infobar") {
                    bars.remove(&w);
                    break;
                }
                c = w.next_sibling();
            }
            count -= 1;
        }
        let bar = InfoBar::new(Severity::Error, title.unwrap_or(s("errTitle")), msg);
        let widget = bar.widget().clone();
        let b2 = bars.downgrade();
        let w2 = widget.downgrade();
        bar.connect_closed(move || {
            if let (Some(b), Some(w)) = (b2.upgrade(), w2.upgrade())
                && w.parent().as_ref() == Some(b.upcast_ref())
            {
                b.remove(&w);
            }
        });
        bars.append(&widget);
        let b2 = bars.downgrade();
        let w2 = widget.downgrade();
        glib::timeout_add_local_once(Duration::from_secs(15), move || {
            if let (Some(b), Some(w)) = (b2.upgrade(), w2.upgrade())
                && w.parent().as_ref() == Some(b.upcast_ref())
            {
                b.remove(&w);
            }
        });
    }

    /// A dry-run answer: say once that nothing really changed.
    pub fn check_dry(&self, res: &Value) {
        if res["dryRun"] == true && !self.inner.dry_shown.replace(true) {
            self.toast(s("dryRun"), ToastOpts { severity: Some(Severity::Info), ..Default::default() });
        }
    }

    // ======================================================================================= operations

    /// Select these paths once the active tab reloaded (and open the rename box on `rename`).
    pub fn select_after_reload(&self, paths: &[String], rename: Option<String>) {
        let Some(tab) = self.active_tab() else { return };
        let id = {
            let mut t = tab.borrow_mut();
            t.pending_select = paths.first().map(|p| vec![p.clone()]);
            t.selection = paths.iter().cloned().collect();
            t.focus = paths.first().cloned();
            t.pending_rename = rename;
            t.id
        };
        self.reload(id, 0);
    }

    /// Open an item: a folder or drive navigates, a file opens (the picker chooses it), in the Prullenbak
    /// Eigenschappen.
    pub fn open_item(&self, item: &Item, focus: bool) {
        if let Some(v) = item.get("volume").filter(|v| v.is_object()) {
            self.navigate(Loc::Folder(st(v, "mount").to_owned()), NavOpts { focus, ..Default::default() });
            return;
        }
        if self.loc() == Loc::Trash {
            ops::properties(self, vec![item.clone()]);
            return;
        }
        if flag(item, "isDir") {
            self.navigate(Loc::Folder(st(item, "path").to_owned()), NavOpts { focus, ..Default::default() });
            return;
        }
        let picker = self.inner.picker.borrow().clone();
        if let Some(p) = picker {
            (p.on_select)(item);
            return;
        }
        ops::open_file(self, item);
    }

    /// Openen: one item opens, several open (folders in new tabs, at most 10).
    pub fn open_selection(&self) {
        let sel = self.selected_items();
        if sel.len() == 1 {
            self.open_item(&sel[0], true);
            return;
        }
        for it in sel.iter().take(10) {
            if flag(it, "isDir") {
                let p = it.get("volume").map(|v| st(v, "mount").to_owned()).filter(|p| !p.is_empty()).unwrap_or_else(|| st(it, "path").to_owned());
                self.open_tab(Loc::Folder(p), false);
            } else {
                ops::open_file(self, it);
            }
        }
    }

    pub fn open_new_tabs(&self, items: &[Item]) {
        for it in items.iter().filter(|x| flag(x, "isDir")).take(10) {
            let p = it.get("volume").map(|v| st(v, "mount").to_owned()).filter(|p| !p.is_empty()).unwrap_or_else(|| st(it, "path").to_owned());
            self.open_tab(Loc::Folder(p), false);
        }
    }

    /// Bestandslocatie openen: the item's folder with the item selected.
    pub fn open_location(&self, item: &Value) {
        let p = st(item, "path").to_owned();
        self.navigate(Loc::Folder(util::dirname(&p)), NavOpts { select: Some(vec![p]), focus: true, ..Default::default() });
    }

    /// Overal zoeken: the Alles app with this query.
    pub fn open_alles(&self, query: &str) {
        self.call("shell.openApp", json!({ "app": "alles", "params": { "query": query } }), |x, r| {
            if let Err(e) = r {
                x.show_error(&e.to_string(), None);
            }
        });
    }

    /// Naam wijzigen (F2): the rename box on the one selected item.
    pub fn rename_selected(&self) {
        let sel = self.selected_items();
        if sel.len() != 1 || self.loc() == Loc::Trash {
            return;
        }
        if let Some(v) = self.view() {
            v.start_rename(st(&sel[0], "path"));
        }
    }

    /// Kopiëren als pad: the quoted paths (or the folder) on the clipboard.
    pub fn copy_path(&self, items: &[Value]) {
        let paths: Vec<String> = if items.is_empty() {
            self.cur_dir().into_iter().collect()
        } else {
            items
                .iter()
                .map(|x| x.get("volume").map(|v| st(v, "mount").to_owned()).filter(|p| !p.is_empty()).unwrap_or_else(|| st(x, "path").to_owned()))
                .collect()
        };
        if paths.is_empty() {
            return;
        }
        let text = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("\n");
        if util::copy_text(&self.inner.root, &text) {
            self.toast(&n("pathCopied", paths.len(), &[]), ToastOpts { timeout: 2500, ..Default::default() });
        }
    }

    /// Openen in Terminal.
    pub fn terminal(&self, dir: Option<String>) {
        if let Some(d) = dir.or_else(|| self.cur_dir()) {
            self.call("files.openTerminal", json!({ "dir": d }), |x, r| match r {
                Ok(v) => x.check_dry(&v),
                Err(e) => x.show_error(&e.to_string(), None),
            });
        }
    }

    pub fn select_all(&self) {
        if let Some(v) = self.view() {
            v.select_all();
        } else {
            let paths: Vec<String> = self.page_items().iter().map(|(it, _)| st(it, "path").to_owned()).collect();
            self.set_selection(&paths, None);
        }
    }

    pub fn select_none(&self) {
        self.set_selection(&[], None);
    }

    pub fn invert_selection(&self) {
        if let Some(v) = self.view() {
            v.invert();
        }
    }

    pub fn set_view(&self, mode: &str) {
        if VIEW_MODES.contains(&mode) {
            self.set_cfg("view", json!(mode));
        }
    }

    fn zoom(&self, d: i32) {
        let cur = self.cfg_str("view", "details");
        let i = ZOOM_ORDER.iter().position(|m| *m == cur).unwrap_or(0) as i32;
        let j = (i + d).clamp(0, ZOOM_ORDER.len() as i32 - 1);
        if j != i {
            self.set_view(ZOOM_ORDER[j as usize]);
        }
    }

    /// A header click: the same column flips the order, another sorts by it (dates and sizes newest first).
    pub fn set_sort(&self, key: &str) {
        if self.cfg_str("sortBy", "name") == key {
            self.set_cfg("sortDesc", json!(!self.cfg_bool("sortDesc", false)));
        } else {
            self.set_cfg("sortBy", json!(key));
            self.set_cfg("sortDesc", json!(matches!(key, "modified" | "deleted" | "size")));
        }
    }

    pub fn set_sort_dir(&self, desc: bool) {
        self.set_cfg("sortDesc", json!(desc));
    }

    pub fn set_group(&self, key: &str) {
        self.set_cfg("groupBy", json!(key));
    }

    /// Toggle a boolean setting; the details and preview pane exclude each other.
    pub fn toggle_cfg(&self, key: &str) {
        let def = matches!(key, "navPane" | "showExtensions");
        let on = !self.cfg_bool(key, def);
        if on && key == "detailsPane" && self.cfg_bool("previewPane", false) {
            self.set_cfg("previewPane", json!(false));
        }
        if on && key == "previewPane" && self.cfg_bool("detailsPane", false) {
            self.set_cfg("detailsPane", json!(false));
        }
        self.set_cfg(key, json!(on));
    }

    // ===================================================================================== address bar

    /// The text the address bar edits: the path of a folder, the name of a page.
    pub fn address_text(&self, loc: &Loc) -> String {
        match loc {
            Loc::Folder(p) | Loc::Search { path: p, .. } => p.clone(),
            other => self.loc_label(other, false),
        }
    }

    /// What typed text in the address bar means (a file that exists opens, in its folder).
    pub fn resolve_address(&self, text: &str, done: impl FnOnce(&Explorer, Resolved) + 'static) {
        let mut sv = text.trim().to_owned();
        if sv.len() >= 2 && sv.starts_with('"') && sv.ends_with('"') {
            sv = sv[1..sv.len() - 1].to_owned();
        }
        if sv.is_empty() {
            done(self, Resolved::NotFound);
            return;
        }
        let low = sv.to_lowercase();
        let named: [(Loc, &[&str]); 4] = [
            (Loc::Home, &["start", "home", "snelle toegang", "quick access"]),
            (Loc::ThisPc, &["deze pc", "this pc", "computer", "mijn computer"]),
            (Loc::Trash, &["prullenbak", "recycle bin", "trash"]),
            (Loc::Gallery, &["galerij", "gallery"]),
        ];
        for (loc, names) in named {
            if names.contains(&low.as_str()) {
                done(self, Resolved::Go(loc));
                return;
            }
        }
        for id in PLACE_IDS {
            if let Some(pl) = self.place(id) {
                let p = st(&pl, "path");
                if !p.is_empty() && (low == s(id).to_lowercase() || low == st(&pl, "name").to_lowercase()) {
                    done(self, Resolved::Go(Loc::Folder(p.to_owned())));
                    return;
                }
            }
        }
        if low.starts_with("http://") || low.starts_with("https://") {
            self.call("apps.openUri", json!({ "uri": sv }), |x, r| {
                if let Err(e) = r {
                    x.show_error(&e.to_string(), None);
                }
            });
            done(self, Resolved::Handled);
            return;
        }
        if low.starts_with("trash:") {
            done(self, Resolved::Go(Loc::Trash));
            return;
        }
        if low.starts_with("computer:") {
            done(self, Resolved::Go(Loc::ThisPc));
            return;
        }
        if sv.starts_with("file://") {
            sv = util::uri_to_path(&sv).unwrap_or(sv);
        }
        let home = self.inner.s.borrow().home.clone();
        if sv == "~" || sv.starts_with("~/") {
            sv = format!("{home}{}", &sv[1..]);
        }
        if !sv.starts_with('/') {
            let base = self.cur_dir().unwrap_or(home);
            sv = util::join_path(&base, &sv);
        }
        let path = util::norm_path(&sv);
        self.call("files.list", json!({ "path": path, "showHidden": false }), move |x, r| match r {
            Ok(res) => done(x, Resolved::Go(Loc::Folder(res["path"].as_str().unwrap_or(&path).to_owned()))),
            Err(_) => {
                let p = path.clone();
                x.call("files.properties", json!({ "path": path }), move |x, r| {
                    if r.is_ok() {
                        x.navigate(Loc::Folder(util::dirname(&p)), NavOpts { select: Some(vec![p.clone()]), focus: true, ..Default::default() });
                        ops::open_file(x, &Rc::new(json!({ "path": p, "name": util::basename(&p) })));
                        done(x, Resolved::Handled);
                    } else {
                        done(x, Resolved::NotFound);
                    }
                });
            }
        });
    }

    /// Suggestions while typing in the address bar: (label, value, icon widget maker).
    pub fn address_suggestions(&self, text: &str, done: impl FnOnce(&Explorer, Vec<(String, String, Loc)>) + 'static) {
        let mut sv = text.to_owned();
        if sv.is_empty() {
            done(self, Vec::new());
            return;
        }
        let home = self.inner.s.borrow().home.clone();
        if sv == "~" || sv.starts_with("~/") {
            sv = format!("{home}{}", &sv[1..]);
        }
        if !sv.starts_with('/') {
            let low = sv.to_lowercase();
            let mut named: Vec<(String, String, Loc)> = vec![
                (s("home").into(), s("home").into(), Loc::Home),
                (s("thisPC").into(), s("thisPC").into(), Loc::ThisPc),
                (s("trash").into(), s("trash").into(), Loc::Trash),
                (s("gallery").into(), s("gallery").into(), Loc::Gallery),
            ];
            for id in PLACE_IDS {
                let p = self.place_path(id);
                if !p.is_empty() {
                    named.push((s(id).into(), p.clone(), Loc::Folder(p)));
                }
            }
            let out: Vec<_> = named.into_iter().filter(|(l, _, _)| l.to_lowercase().starts_with(&low)).take(8).collect();
            done(self, out);
            return;
        }
        let (dir, prefix) = if sv.ends_with('/') { (util::norm_path(&sv), String::new()) } else { (util::dirname(&sv), util::basename(&sv).to_lowercase()) };
        self.list_dirs(&dir, move |x, r| {
            let list = r.unwrap_or_default();
            let out: Vec<_> = list
                .into_iter()
                .filter(|d| d.name.to_lowercase().starts_with(&prefix))
                .take(12)
                .map(|d| (d.path.clone(), d.path.clone(), Loc::Folder(d.path)))
                .collect();
            done(x, out);
        });
    }

    // ========================================================================================= keyboard

    fn install_keys(&self) {
        let window = self.inner.win.window();
        // physical keys: capture phase, so Ctrl+Tab beats GTK's focus moving and the checks mirror main.js
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.weak();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(x) = w.upgrade() else { return glib::Propagation::Proceed };
            x.inner.key_in_flight.set(true);
            let w2 = x.weak();
            glib::idle_add_local_once(move || {
                if let Some(x) = w2.upgrade() {
                    x.inner.key_in_flight.set(false);
                }
            });
            // Windows draws the focus rectangle only after keyboard navigation
            let nav = matches!(
                key,
                gdk::Key::Up | gdk::Key::Down | gdk::Key::Left | gdk::Key::Right | gdk::Key::Home | gdk::Key::End | gdk::Key::Page_Up | gdk::Key::Page_Down | gdk::Key::Tab | gdk::Key::ISO_Left_Tab | gdk::Key::F6
            ) || (key.to_unicode().is_some_and(|ch| !ch.is_control()) && !state.intersects(gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::ALT_MASK));
            if nav {
                x.inner.win.window().add_css_class("xp-kbd");
            }
            if x.on_key(key, state) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        window.add_controller(keys);
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        click.set_button(0);
        let w = self.weak();
        click.connect_pressed(move |_, _, _, _| {
            if let Some(x) = w.upgrade() {
                x.inner.win.window().remove_css_class("xp-kbd");
            }
        });
        window.add_controller(click);
        // the same keys for the harness's dev channel ("key" runs AppWindow shortcuts)
        const ACCELS: &[&str] = &[
            "<Control>t", "<Control>n", "<Control>w", "<Control>Tab", "<Control><Shift>Tab", "<Control>l", "<Alt>d", "F4",
            "<Control>f", "<Control>e", "F3", "<Alt>Left", "<Alt>Right", "<Alt>Up", "F5", "<Control>r", "<Alt>p", "<Alt><Shift>p",
            "<Control><Shift>1", "<Control><Shift>2", "<Control><Shift>3", "<Control><Shift>4", "<Control><Shift>5",
            "<Control><Shift>6", "<Control><Shift>7", "<Control><Shift>8", "<Control><Shift>n", "<Control><Shift>c",
            "<Alt>Return", "BackSpace", "Return", "F2", "Delete", "<Shift>Delete", "<Control>c", "<Control>x", "<Control>v",
            "<Control>z", "<Control>a", "Escape",
        ];
        for accel in ACCELS {
            let Some((key, mods)) = gtk::accelerator_parse(*accel) else { continue };
            let w = self.weak();
            self.inner.win.add_shortcut(accel, move || {
                let Some(x) = w.upgrade() else { return false };
                // a physical key was handled (or deliberately left) in the capture phase already
                if x.inner.key_in_flight.get() {
                    return false;
                }
                x.on_key(key, mods)
            });
        }
    }

    fn focus_widget(&self) -> Option<gtk::Widget> {
        gtk::prelude::GtkWindowExt::focus(self.inner.win.window())
    }

    /// Whether a menu (popover) has the focus.
    fn menu_open(&self) -> bool {
        self.focus_widget().is_some_and(|f| f.ancestor(gtk::Popover::static_type()).is_some())
    }

    fn in_text(&self) -> bool {
        self.focus_widget().is_some_and(|f| f.is::<gtk::Text>() || f.is::<gtk::TextView>() || f.ancestor(gtk::Entry::static_type()).is_some())
    }

    /// main.js onKey: true when handled.
    fn on_key(&self, key: gdk::Key, state: gdk::ModifierType) -> bool {
        if self.inner.win.dialog_open() || self.menu_open() {
            return false;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let low = key.to_lower();
        let picker = self.is_picker();
        let is = |k: gdk::Key| low == k;
        if picker && ctrl && (is(gdk::Key::t) || is(gdk::Key::n)) {
            return true;
        }
        if ctrl && !alt && !shift && is(gdk::Key::t) {
            self.open_tab(Loc::Home, true);
            return true;
        }
        if ctrl && !alt && !shift && is(gdk::Key::n) {
            self.open_tab(self.loc(), true);
            return true;
        }
        if ctrl && !alt && is(gdk::Key::w) {
            self.close_tab(self.active_id());
            return true;
        }
        if ctrl && (key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab) {
            self.cycle_tab(if shift || key == gdk::Key::ISO_Left_Tab { -1 } else { 1 });
            return true;
        }
        if (ctrl && is(gdk::Key::l)) || (alt && is(gdk::Key::d)) || (key == gdk::Key::F4 && !alt && !ctrl) {
            self.inner.address.focus_address();
            return true;
        }
        if (ctrl && (is(gdk::Key::f) || is(gdk::Key::e))) || key == gdk::Key::F3 {
            self.inner.address.focus_search();
            return true;
        }
        if alt && !ctrl && key == gdk::Key::Left {
            self.back();
            return true;
        }
        if alt && !ctrl && key == gdk::Key::Right {
            self.forward();
            return true;
        }
        if alt && !ctrl && key == gdk::Key::Up {
            self.up();
            return true;
        }
        if key == gdk::Key::F5 || (ctrl && is(gdk::Key::r)) {
            self.refresh();
            return true;
        }
        if alt && is(gdk::Key::p) {
            self.toggle_cfg(if shift { "detailsPane" } else { "previewPane" });
            return true;
        }
        if ctrl && shift {
            let digit = match key {
                gdk::Key::_1 | gdk::Key::exclam => Some(1),
                gdk::Key::_2 | gdk::Key::at => Some(2),
                gdk::Key::_3 | gdk::Key::numbersign => Some(3),
                gdk::Key::_4 | gdk::Key::dollar => Some(4),
                gdk::Key::_5 | gdk::Key::percent => Some(5),
                gdk::Key::_6 | gdk::Key::asciicircum => Some(6),
                gdk::Key::_7 | gdk::Key::ampersand => Some(7),
                gdk::Key::_8 | gdk::Key::asterisk => Some(8),
                _ => None,
            };
            if let Some(d) = digit {
                self.set_view(VIEW_MODES[d - 1]);
                return true;
            }
        }
        if self.in_text() {
            return false;
        }
        let focus = self.focus_widget();
        let in_content = focus.as_ref().is_none_or(|f| f.is_ancestor(&self.inner.content_host) || f == self.inner.content_host.upcast_ref::<gtk::Widget>());
        let on_button = focus.as_ref().is_some_and(|f| {
            f.is::<gtk::Button>() || f.has_css_class("xp-node") || f.has_css_class("xp-tab") || f.ancestor(gtk::Button::static_type()).is_some()
        });
        if ctrl && shift && is(gdk::Key::n) {
            ops::new_folder(self);
            return true;
        }
        if ctrl && shift && is(gdk::Key::c) {
            let items: Vec<Value> = self.selected_items().iter().map(|i| (**i).clone()).collect();
            self.copy_path(&items);
            return true;
        }
        if alt && (key == gdk::Key::Return || key == gdk::Key::KP_Enter) {
            ops::properties(self, self.selected_items());
            return true;
        }
        if key == gdk::Key::BackSpace && !on_button {
            self.up();
            return true;
        }
        let enter = key == gdk::Key::Return || key == gdk::Key::KP_Enter;
        if on_button && (enter || key == gdk::Key::space) {
            return false;
        }
        if !in_content && on_button {
            return false;
        }
        if enter && !alt && in_content {
            match self.view() {
                Some(v) if v.renaming() => return false,
                Some(_) => {
                    self.open_selection();
                    return true;
                }
                None => {}
            }
        }
        if key == gdk::Key::F2 {
            self.rename_selected();
            return true;
        }
        if key == gdk::Key::Delete {
            let paths: Vec<Item> = self.selected_items();
            if shift {
                ops::delete_permanently(self, paths);
            } else {
                ops::trash(self);
            }
            return true;
        }
        if ctrl && !shift && is(gdk::Key::c) {
            ops::copy(self);
            return true;
        }
        if ctrl && !shift && is(gdk::Key::x) {
            ops::cut(self);
            return true;
        }
        if ctrl && !shift && is(gdk::Key::v) {
            ops::paste(self, None);
            return true;
        }
        if ctrl && !shift && is(gdk::Key::z) {
            ops::undo(self);
            return true;
        }
        if ctrl && !shift && is(gdk::Key::a) {
            self.select_all();
            return true;
        }
        if key == gdk::Key::Escape && self.active_tab().is_some_and(|t| !t.borrow().filter.is_empty()) {
            self.inner.address.clear_search();
            self.set_filter("");
            return true;
        }
        false
    }

    // ========================================================================================== filter

    /// Whether the search box filters here (folders, the Prullenbak, search results).
    pub fn can_filter(&self) -> bool {
        self.loc().has_listing()
    }

    /// The search box's live filter.
    pub fn set_filter(&self, v: &str) {
        let Some(tab) = self.active_tab() else { return };
        if !self.can_filter() || tab.borrow().filter == v {
            return;
        }
        tab.borrow_mut().filter = v.to_owned();
        self.render_content(false, true);
        self.update_status();
    }

    /// Enter in the search box: Alles with the text.
    pub fn search_submit(&self, v: &str) {
        let q = v.trim();
        if !q.is_empty() {
            self.open_alles(q);
        }
    }

    /// The active tab's filter text.
    pub fn filter(&self) -> String {
        self.active_tab().map(|t| t.borrow().filter.clone()).unwrap_or_default()
    }

    // ========================================================================================== events

    fn listen(&self, prefix: &str, f: impl Fn(&Explorer, &str, &Value) + 'static) {
        let w = self.weak();
        let id = self.inner.ctx.on(prefix, move |name, data| {
            if let Some(x) = w.upgrade() {
                f(&x, name, data);
            }
        });
        self.inner.listeners.borrow_mut().push(id);
    }

    fn install_events(&self) {
        self.listen("files.dirChanged", |x, _, d| {
            let Some(p) = d["path"].as_str() else { return };
            x.invalidate_dir(p);
            let trash = x.place_path("trash");
            let ids: Vec<u64> = x
                .inner
                .tabs
                .borrow()
                .iter()
                .filter(|t| {
                    let t = t.borrow();
                    t.loc == Loc::Folder(p.to_owned()) || (t.loc == Loc::Trash && trash == p)
                })
                .map(|t| t.borrow().id)
                .collect();
            for id in ids {
                x.reload(id, 250);
            }
            if !trash.is_empty() && (p == trash || util::is_under(p, &util::dirname(&trash))) {
                x.load_trash_info();
            }
            x.inner.nav.invalidate(Some(p));
        });
        self.listen("files.desktopChanged", |x, _, _| {
            let dp = x.place_path("desktop");
            if dp.is_empty() {
                return;
            }
            let ids: Vec<u64> = x.inner.tabs.borrow().iter().filter(|t| t.borrow().loc == Loc::Folder(dp.clone())).map(|t| t.borrow().id).collect();
            for id in ids {
                x.reload(id, 250);
            }
        });
        self.listen("files.clipboardChanged", |x, _, d| x.apply_clipboard(d));
        self.listen("storage.volumesChanged", |x, _, d| {
            if !d["volumes"].is_array() {
                return;
            }
            x.set_volumes(&d["volumes"]);
            x.inner.nav.render();
            if x.loc() == Loc::ThisPc {
                x.render_content(false, true);
            }
        });
        self.listen("files.trashChanged", |x, _, _| {
            x.load_trash_info();
            let ids: Vec<u64> = x.inner.tabs.borrow().iter().filter(|t| t.borrow().loc == Loc::Trash).map(|t| t.borrow().id).collect();
            for id in ids {
                x.reload(id, 250);
            }
        });
        // another app asks for a place (app.navigate {app: "explorer", params}): a new tab in the active window
        self.listen("app.navigate", |x, _, d| {
            if d["app"] != "explorer" || Explorer::active().is_none_or(|a| !Rc::ptr_eq(&a.inner, &x.inner)) {
                return;
            }
            let p = crate::params_from_json(&d["params"]);
            x.handle_params(p, true);
        });
        // back on the backend after a restart: the clipboard and drives again; a new backend after the last one
        // ended ({respawn: true}) has none of this window's folder watches
        self.listen("backend.ready", |x, _, d| {
            x.load_clipboard();
            x.load_trash_info();
            if d["respawn"] == true {
                let watched: Vec<String> = x.inner.watched.borrow().iter().cloned().collect();
                for p in watched {
                    x.inner.ctx.call("files.watch", json!({ "path": p }), |_| {});
                }
            }
        });
        // settings
        let w = self.weak();
        self.inner.ctx.on_settings(move |_, path| {
            if let Some(x) = w.upgrade() {
                x.settings_changed(path);
            }
        });
        // the language: every label again, and the listings (typeName comes from the backend)
        let w = self.weak();
        kit::i18n::on_change(move || {
            if let Some(x) = w.upgrade() {
                x.relabel();
            }
        });
        // the clipboard again when the window comes to the front
        let w = self.weak();
        self.inner.win.window().connect_is_active_notify(move |win| {
            if win.is_active()
                && let Some(x) = w.upgrade()
            {
                x.load_clipboard();
            }
        });
        // closing: the session (transfer.rs), the watches, the listeners
        let w = self.weak();
        self.inner.win.window().connect_close_request(move |_| {
            if let Some(x) = w.upgrade() {
                x.closing();
            }
            glib::Propagation::Proceed
        });
    }

    fn closing(&self) {
        if self.inner.closed.replace(true) {
            return;
        }
        if !self.is_picker() && self.inner.started.get() {
            transfer::save_session(self);
        }
        for p in self.inner.watched.borrow_mut().drain() {
            self.inner.ctx.call("files.unwatch", json!({ "path": p }), |_| {});
        }
        for id in self.inner.listeners.borrow_mut().drain(..) {
            self.inner.ctx.off(id);
        }
        if let Some(v) = self.inner.view.borrow_mut().take() {
            v.destroy();
        }
        let me = self.inner.clone();
        OPEN.with(|o| o.borrow_mut().retain(|x| !Rc::ptr_eq(&x.inner, &me)));
    }

    fn settings_changed(&self, path: Option<&str>) {
        if self.inner.closed.get() || !self.inner.started.get() {
            return;
        }
        if let Some(p) = path {
            if p == "explorer.columns" || p == "explorer.navWidth" {
                return;
            }
            if !(p.starts_with("explorer") || p.starts_with("privacy") || p.starts_with("theme") || p.starts_with("language")) {
                return;
            }
        }
        self.apply_layout();
        if path == Some("explorer.pinned") || path.is_none() {
            self.inner.nav.render();
        }
        if path == Some("explorer.showHidden") {
            self.inner.dir_cache.borrow_mut().clear();
            self.reload_active(0);
            return;
        }
        if matches!(path, Some("explorer.detailsPane" | "explorer.previewPane" | "explorer.navPane")) {
            self.update_chrome();
            return;
        }
        if self.view().is_some_and(|v| v.renaming()) {
            self.inner.render_after_rename.set(true);
            self.update_chrome();
            return;
        }
        self.render_content(false, true);
        self.update_chrome();
    }

    /// A language switch: every text again (the WebKit app reloaded its window), listings reloaded.
    fn relabel(&self) {
        if self.inner.closed.get() {
            return;
        }
        self.inner.address.relabel();
        self.inner.cmdbar.relabel();
        self.inner.nav.render();
        self.inner.tab_strip.render();
        self.inner.details.render();
        let ids: Vec<u64> = self.tab_ids();
        for id in ids {
            let listing = self.tab(id).is_some_and(|t| t.borrow().loc.has_listing());
            if listing {
                self.reload(id, 0);
            }
        }
        self.render_content(false, true);
        self.update_chrome();
    }

    // ===================================================================================== parameters

    /// A place word (params.place, or a non-path params.path such as "thispc") as a location.
    fn place_to_loc(&self, word: &str) -> Option<Loc> {
        let w = word.trim().to_lowercase();
        if w.is_empty() {
            return None;
        }
        if w.starts_with("trash:") || ["trash", "prullenbak", "recyclebin", "recycle-bin"].contains(&w.as_str()) {
            return Some(Loc::Trash);
        }
        if w.starts_with("computer:") || ["thispc", "this-pc", "computer", "mypc", "deze-pc", "drives"].contains(&w.as_str()) {
            return Some(Loc::ThisPc);
        }
        if w.starts_with("recent:") || ["home", "start", "quickaccess", "quick-access", "recent"].contains(&w.as_str()) {
            return Some(Loc::Home);
        }
        if w == "gallery" || w == "galerij" {
            return Some(Loc::Gallery);
        }
        let alias = match w.as_str() {
            "pictures" | "afbeeldingen" => Some("pictures"),
            "documents" | "documenten" => Some("documents"),
            "downloads" => Some("downloads"),
            "music" | "muziek" => Some("music"),
            "videos" | "video's" => Some("videos"),
            "desktop" | "bureaublad" => Some("desktop"),
            "homefolder" | "home-folder" | "user" | "personal" => Some("home"),
            _ => None,
        };
        if let Some(a) = alias {
            let p = self.place_path(a);
            if !p.is_empty() {
                return Some(Loc::Folder(p));
            }
        }
        if w != "trash" && w != "root" {
            let p = self.place_path(&w);
            if !p.is_empty() && p.starts_with('/') {
                return Some(Loc::Folder(p));
            }
        }
        None
    }

    fn params_to_loc(&self, path: Option<&str>, place: Option<&str>) -> Option<Loc> {
        if let Some(raw) = path.filter(|p| !p.is_empty()) {
            let mut p = if raw.starts_with('/') { raw.to_owned() } else { raw.trim().to_owned() };
            let low = p.to_lowercase();
            let wordish = low.starts_with("trash:") || low.starts_with("computer:") || low.starts_with("recent:") || !(p.starts_with('/') || p.starts_with('~') || low.starts_with("file:"));
            if wordish && let Some(l) = self.place_to_loc(&p) {
                return Some(l);
            }
            if low.starts_with("file://") {
                p = util::uri_to_path(&p).unwrap_or(p);
            }
            let home = self.inner.s.borrow().home.clone();
            if p == "~" || p.starts_with("~/") {
                p = format!("{home}{}", &p[1..]);
            }
            if !p.starts_with('/') {
                return place.and_then(|w| self.place_to_loc(w));
            }
            return Some(Loc::Folder(util::norm_path(&p)));
        }
        place.and_then(|w| self.place_to_loc(w))
    }

    /// params.select: names, paths or file:// URIs as absolute paths.
    fn select_for(&self, sel: Option<&Value>, loc: &Loc) -> Option<Vec<String>> {
        let v = jsonish(sel?);
        let list: Vec<Value> = match v {
            Value::Array(a) => a,
            Value::Null => return None,
            other => vec![other],
        };
        let mut out: Vec<String> = Vec::new();
        for s in list.iter().take(1000).filter_map(Value::as_str).filter(|s| !s.is_empty()) {
            let mut s = s.to_owned();
            if s.starts_with("file://") {
                s = util::uri_to_path(&s).unwrap_or(s);
            }
            if !s.starts_with('/')
                && let Loc::Folder(dir) = loc
            {
                s = util::join_path(dir, &s);
            }
            if s.starts_with('/') {
                let n = util::norm_path(&s);
                if !out.contains(&n) {
                    out.push(n);
                }
            }
        }
        if out.is_empty() { None } else { Some(out) }
    }

    /// "Bestandslocatie openen" passes the file itself: open its folder with the file selected.
    async fn resolve_file_param(&self, loc: Loc, select: Option<Vec<String>>) -> (Loc, Option<Vec<String>>) {
        let Loc::Folder(p) = &loc else { return (loc, select) };
        if select.is_some() || self.place_for_path(p).is_some() {
            return (loc, select);
        }
        if let Ok(st) = self.ctx().call_async("files.stat", json!({ "path": p })).await
            && st["isDir"] == false
        {
            return (Loc::Folder(util::dirname(p)), Some(vec![p.clone()]));
        }
        (loc, select)
    }

    /// params.tabs: more folders, each in its own tab after the active one.
    async fn extra_tabs(&self, tabs: Option<&Value>) -> usize {
        let Some(v) = tabs else { return 0 };
        let Value::Array(list) = jsonish(v) else { return 0 };
        let mut at = self.index_of(self.active_id()).map(|i| i + 1).unwrap_or(0);
        let mut n = 0;
        for e in list.iter().take(MAX_EXTRA_TABS) {
            let (path, sel) = match e {
                Value::String(s) => (Some(s.clone()), None),
                Value::Object(_) => (e["path"].as_str().map(str::to_owned), e.get("select").cloned()),
                _ => continue,
            };
            let Some(loc) = self.params_to_loc(path.as_deref(), None) else { continue };
            let select = self.select_for(sel.as_ref(), &loc);
            let (loc, select) = self.resolve_file_param(loc, select).await;
            let tab = self.new_tab(loc);
            tab.borrow_mut().pending_select = select;
            let id = tab.borrow().id;
            {
                let mut tabs = self.inner.tabs.borrow_mut();
                at = at.min(tabs.len());
                tabs.insert(at, tab);
            }
            at += 1;
            self.load(id, false);
            n += 1;
        }
        if n > 0 {
            self.inner.tab_strip.render();
            self.sync_watches();
        }
        n
    }

    /// A launch for this window (a relaunch's new tab with `new_tab_if_busy`, or the first place).
    pub fn handle_params(&self, p: Params, new_tab_if_busy: bool) {
        let me = self.clone();
        glib::spawn_future_local(async move {
            let Some(loc) = me.params_to_loc(p.path.as_deref(), p.place.as_deref()) else {
                me.extra_tabs(p.tabs.as_ref()).await;
                return;
            };
            let select = me.select_for(p.select.as_ref(), &loc);
            let (loc, select) = me.resolve_file_param(loc, select).await;
            let props = p.properties;
            let pristine = me.active_tab().is_some_and(|t| {
                let t = t.borrow();
                t.loc == Loc::Home && t.history.len() == 1
            });
            if new_tab_if_busy && !pristine {
                let id = me.open_tab(loc, true);
                if let Some(t) = me.tab(id) {
                    let mut t = t.borrow_mut();
                    t.pending_select = select.clone();
                    t.pending_props = props && select.is_some();
                }
                if select.is_some() {
                    me.render_content(false, true);
                }
            } else {
                me.navigate(loc, NavOpts { focus: true, select, props, ..Default::default() });
            }
            me.extra_tabs(p.tabs.as_ref()).await;
        });
    }

    async fn start(&self, params: Params) {
        let ctx = self.ctx();
        let call = |m: &'static str| {
            let c = ctx.clone();
            glib::spawn_future_local(async move { c.call_async(m, json!({})).await })
        };
        let (hp, hv, hc, hi) = (call("files.places"), call("storage.volumes"), call("files.clipboardGet"), call("shell.info"));
        let gone = || CallError::new("internal", "");
        let places = hp.await.unwrap_or_else(|_| Err(gone()));
        let vols = hv.await.unwrap_or_else(|_| Err(gone()));
        let clip = hc.await.unwrap_or_else(|_| Err(gone()));
        let info = hi.await.unwrap_or_else(|_| Err(gone()));
        if self.inner.closed.get() {
            return;
        }
        {
            let mut s = self.inner.s.borrow_mut();
            if let Ok(Value::Array(list)) = &places {
                for pl in list {
                    if let Some(id) = pl["id"].as_str() {
                        s.places.insert(id.to_owned(), pl.clone());
                    }
                }
            } else if let Err(e) = &places {
                eprintln!("win11-gtk-explorer: files.places failed: {e}");
            }
            if let Ok(i) = &info {
                s.user = i["user"].clone();
            }
            let home = s.places.get("home").map(|h| st(h, "path").to_owned()).filter(|p| !p.is_empty());
            let user = s.user["name"].as_str().filter(|n| !n.is_empty()).map(|n| format!("/home/{n}"));
            s.home = home.or(user).unwrap_or_else(|| "/".into());
        }
        if let Ok(v) = &vols {
            self.set_volumes(v);
            self.inner.vol_at.set(kit::format::now_ms());
        }
        if let Ok(c) = &clip {
            self.apply_clipboard(c);
        }
        self.load_trash_info();

        let loc0 = self.params_to_loc(params.path.as_deref(), params.place.as_deref()).unwrap_or(Loc::Home);
        let sel0 = self.select_for(params.select.as_ref(), &loc0);
        let (loc, select) = self.resolve_file_param(loc0, sel0).await;
        // a saved session (transfer.rs) when the launch named nothing
        let mut restored: Option<(Vec<Rc<RefCell<Tab>>>, usize)> = None;
        let saved = || (params.picker.is_none() && !params.explicit()).then(transfer::restored_session).flatten();
        if let Some(state) = params.session.clone().or_else(saved) {
            let tabs: Vec<Rc<RefCell<Tab>>> = state["tabs"]
                .as_array()
                .map(|a| a.iter().take(20).filter_map(|s| Tab::from_state(self.next_id(), s)).map(|t| Rc::new(RefCell::new(t))).collect())
                .unwrap_or_default();
            if !tabs.is_empty() {
                let a = (state["active"].as_u64().unwrap_or(0) as usize).min(tabs.len() - 1);
                restored = Some((tabs, a));
            }
        }
        let first_id;
        match restored {
            Some((tabs, a)) => {
                first_id = tabs[a].borrow().id;
                tabs[a].borrow_mut().want_focus = true;
                *self.inner.tabs.borrow_mut() = tabs;
            }
            None => {
                let first = self.new_tab(loc);
                {
                    let mut t = first.borrow_mut();
                    t.pending_select = select.clone();
                    t.pending_props = params.properties && select.is_some();
                    t.want_focus = true;
                    t.placeholder = params.tab_ticket.is_some();
                }
                first_id = first.borrow().id;
                self.inner.tabs.borrow_mut().push(first);
            }
        }
        self.inner.active.set(first_id);
        self.inner.started.set(true);
        self.apply_layout();
        self.inner.nav.render();
        self.inner.tab_strip.render();
        self.load(first_id, false);
        self.render_content(false, false);
        self.update_chrome();
        self.sync_watches();
        if let Some(ticket) = &params.tab_ticket {
            transfer::claim_ticket(self, ticket);
        }
        self.extra_tabs(params.tabs.as_ref()).await;
        if let Some(what) = params.open.clone() {
            let w = self.weak();
            glib::timeout_add_local_once(Duration::from_millis(900), move || {
                if let Some(x) = w.upgrade() {
                    x.debug_open(&what);
                }
            });
        }
    }

    /// Screenshot helpers (params.open).
    fn debug_open(&self, what: &str) {
        let items = self.selected_items();
        let anchor = self.view().map(|v| v.canvas()).unwrap_or_else(|| self.anchor());
        match what {
            "menu" if !items.is_empty() => {
                let a = anchor.clone();
                menus::item_menu(self, items, move |top, list| menus::show(&a, 24.0, 60.0, top, list));
            }
            "menu-background" => {
                let (top, list) = menus::background_menu(self);
                menus::show(&anchor, 24.0, 60.0, top, list);
            }
            "options" => crate::dialogs::show_options(self),
            "share" if !items.is_empty() => crate::dialogs::show_share(self, items),
            "drive" => {
                let v = self.inner.s.borrow().volumes.first().cloned();
                if let Some(v) = v {
                    crate::dialogs::show_drive_properties(self, &v);
                }
            }
            "trash" => crate::dialogs::show_trash_properties(self),
            _ => {}
        }
    }
}

/// A JSON-looking string ("[...]", "{...}") parsed; anything else as it is (a name like "[concept] a.txt" stays).
fn jsonish(v: &Value) -> Value {
    match v {
        Value::String(s) if s.trim_start().starts_with('[') || s.trim_start().starts_with('{') => {
            serde_json::from_str(s).unwrap_or_else(|_| v.clone())
        }
        other => other.clone(),
    }
}

fn empty_text(text: &str) -> gtk::Widget {
    let l = kit::controls::label(text, "xp-empty-text", true);
    l.set_xalign(0.5);
    l.set_justify(gtk::Justification::Center);
    l.upcast()
}

/// The big empty / error state (`.xp-bigempty`): icon, title, text and an optional row of buttons.
pub fn big_empty(icon: gtk::Widget, title: &str, msg: &str, action: Option<gtk::Widget>) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 10);
    b.add_css_class("xp-bigempty");
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Start);
    let ib = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    ib.add_css_class("xp-bigempty-icon");
    ib.set_size_request(72, 72);
    ib.set_halign(gtk::Align::Center);
    ib.set_hexpand(false);
    icon.set_hexpand(true);
    icon.set_halign(gtk::Align::Center);
    icon.set_valign(gtk::Align::Center);
    ib.append(&icon);
    b.append(&ib);
    let t = kit::controls::label(title, "w11-subtitle xp-bigempty-title", true);
    t.set_xalign(0.5);
    t.set_justify(gtk::Justification::Center);
    b.append(&t);
    if !msg.is_empty() {
        let m = kit::controls::label(msg, "w11-text-2", true);
        m.set_xalign(0.5);
        m.set_justify(gtk::Justification::Center);
        m.set_max_width_chars(60);
        b.append(&m);
    }
    if let Some(a) = action {
        b.append(&a);
    }
    b.upcast()
}

fn search_bar(glyph: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.add_css_class("xp-searchbar");
    b.append(&kit::icons::glyph(glyph, 14));
    b
}

fn bar_text(text: &str) -> gtk::Label {
    kit::controls::label(text, "xp-searchbar-text", false)
}
