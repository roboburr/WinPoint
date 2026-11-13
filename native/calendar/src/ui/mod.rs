//! The window, laid out like the Windows 11 Calendar app (Mail and Calendar / Outlook): an undecorated window
//! with its own 32 px title bar (app glyph, "Agenda", the three caption buttons of vendor/ui/js/apps/appframe.js),
//! dragged by the title bar (gtk::WindowHandle) and resized from its edges; on the left the navigation pane with
//! "Nieuwe gebeurtenis", the mini month and the calendars; on the right a layer with the toolbar ("Vandaag",
//! previous / next, the month or week title, Dag / Week / Maand) above the view (month.rs, timeline.rs).
//! Event dialogs (editor.rs) are in-window like WinUI's ContentDialog.
//!
//! One App per process (`app()`); a second launch reaches it through GApplication (main.rs).
pub mod editor;
pub mod mini;
pub mod month;
pub mod timeline;

use crate::config::{self, Config, t};
use crate::dates::{self, DAY, Ymd};
use crate::glyphs;
use crate::gnome::Remote;
use crate::store::{self, CalendarInfo, Event, Store};
use crate::theme;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum View {
    Day,
    Week,
    Month,
}

impl View {
    pub fn key(self) -> &'static str {
        match self {
            View::Day => "day",
            View::Week => "week",
            View::Month => "month",
        }
    }

    pub fn parse(s: &str) -> Option<View> {
        match s.trim().to_ascii_lowercase().as_str() {
            "day" | "dag" => Some(View::Day),
            "week" => Some(View::Week),
            "month" | "maand" => Some(View::Month),
            _ => None,
        }
    }
}

/// What a launch asks for (main.rs): a date, a view, a new event.
#[derive(Default, Debug, Clone)]
pub struct Request {
    pub date: Option<Ymd>,
    pub view: Option<View>,
    pub new_event: bool,
}

pub struct State {
    pub view: View,
    pub sel: Ymd,
    pub mini: (i32, u32),
    pub pane: bool,
    /// Event chips that fit in a month cell (month.rs measures it).
    pub fit: i32,
    /// The timeline's scroll position, kept across re-renders.
    pub scroll: Option<f64>,
    pub today: Ymd,
}

pub struct Widgets {
    pub overlay: gtk::Overlay,
    pub content: gtk::Box,
    pub title: gtk::Label,
    pub prev: gtk::Button,
    pub next: gtk::Button,
    pub prev_glyph: gtk::DrawingArea,
    pub next_glyph: gtk::DrawingArea,
    pub views: Vec<(View, gtk::ToggleButton)>,
    pub pane: gtk::Box,
    pub pane_full: gtk::Box,
    pub new_small: gtk::Button,
    pub ham: gtk::Button,
    pub mini: Rc<mini::MiniMonth>,
    pub cal_list: gtk::Box,
    pub view_host: gtk::Box,
    pub max_button: gtk::Button,
    pub max_glyph: gtk::DrawingArea,
    pub edges: Vec<gtk::Widget>,
}

pub struct App {
    pub window: gtk::ApplicationWindow,
    pub store: Store,
    pub remote: Rc<Remote>,
    pub cfg: RefCell<Config>,
    pub st: RefCell<State>,
    w: RefCell<Option<Rc<Widgets>>>,
    /// Month cells and week day headers by date, for in-place selection.
    pub cells: RefCell<Vec<(Ymd, gtk::Widget)>>,
    /// Today's timeline column, redrawn for the "now" line.
    pub now_areas: RefCell<Vec<glib::WeakRef<gtk::DrawingArea>>>,
    dialog: RefCell<Option<gtk::Widget>>,
    pub updating: Cell<bool>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_pending: Cell<bool>,
}

thread_local! {
    static APP: RefCell<Option<Rc<App>>> = const { RefCell::new(None) };
}

pub fn app() -> Option<Rc<App>> {
    APP.with(|a| a.borrow().clone())
}

/// Run `f` on the app after the current event (views rebuild themselves from their own handlers).
pub fn defer(f: impl FnOnce(&Rc<App>) + 'static) {
    glib::idle_add_local_once(move || {
        if let Some(a) = app() {
            f(&a);
        }
    });
}

/// A launch (first or forwarded): make the window once, go to the date (today without one), present it.
pub fn show(gapp: &gtk::Application, req: Request) {
    let a = match app() {
        Some(a) => a,
        None => {
            let a = App::new(gapp);
            APP.with(|x| *x.borrow_mut() = Some(a.clone()));
            a.build();
            a
        }
    };
    {
        let mut st = a.st.borrow_mut();
        st.today = dates::today();
        let d = req.date.unwrap_or(st.today);
        st.sel = d;
        st.mini = (d.0, d.1);
        if let Some(v) = req.view {
            st.view = v;
        }
    }
    if req.view.is_some() {
        a.save_ui_state();
    }
    a.render();
    a.window.present();
    if req.new_event {
        let d = a.st.borrow().sel;
        a.new_event_on(d, None);
    }
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn ui_state_path() -> std::path::PathBuf {
    config::data_dir().join("state.json")
}

fn load_ui_state() -> (View, bool) {
    let v: Value = std::fs::read(ui_state_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let view = v["view"].as_str().and_then(View::parse).unwrap_or(View::Month);
    (view, v["pane"].as_bool().unwrap_or(true))
}

/// A subtle square icon button (components.css .fl-icon-btn).
pub fn icon_button(glyph: &'static str, size: i32, tip: &str) -> (gtk::Button, gtk::DrawingArea) {
    let b = gtk::Button::new();
    b.add_css_class("w11-icon-button");
    b.add_css_class("cal-icon-button");
    let g = glyphs::glyph(glyph, 16);
    b.set_child(Some(&g));
    b.set_size_request(size, size);
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tip));
    (b, g)
}

impl App {
    fn new(gapp: &gtk::Application) -> Rc<App> {
        let cfg = config::load();
        let window = gtk::ApplicationWindow::builder()
            .application(gapp)
            .decorated(false)
            .default_width(1180)
            .default_height(780)
            .title(t("Agenda", "Calendar"))
            .build();
        window.set_widget_name("cal-window");
        window.add_css_class("cal-window");
        window.set_size_request(560, 420);
        let today = dates::today();
        let (view, pane) = load_ui_state();
        Rc::new(App {
            window,
            store: Store::load(),
            remote: Remote::new(),
            cfg: RefCell::new(cfg),
            st: RefCell::new(State {
                view,
                sel: today,
                mini: (today.0, today.1),
                pane,
                fit: 3,
                scroll: None,
                today,
            }),
            w: RefCell::new(None),
            cells: RefCell::new(Vec::new()),
            now_areas: RefCell::new(Vec::new()),
            dialog: RefCell::new(None),
            updating: Cell::new(false),
            monitor: RefCell::new(None),
            reload_pending: Cell::new(false),
        })
    }

    fn build(self: &Rc<Self>) {
        theme::apply(&self.cfg.borrow(), &self.store.calendars.borrow());
        self.build_ui();
        self.install_keys();
        self.watch_settings();
        self.remote.start(|| {
            if let Some(a) = app() {
                a.remote_changed();
            }
        });
        self.start_clock();
        crate::backdrop::start(&self.window, &self.cfg.borrow());
        self.window.connect_maximized_notify(|_| {
            if let Some(a) = app() {
                a.update_square();
            }
        });
        self.window.connect_fullscreened_notify(|_| {
            if let Some(a) = app() {
                a.update_square();
            }
        });
        self.window.connect_realize(|w| {
            if let Some(surface) = w.surface()
                && let Ok(top) = surface.downcast::<gdk::Toplevel>()
            {
                top.connect_state_notify(|_| {
                    if let Some(a) = app() {
                        a.update_square();
                    }
                });
            }
        });
        // GNOME's calendar server keeps one time range for all its clients: ask again when we come back.
        self.window.connect_is_active_notify(|w| {
            if w.is_active()
                && let Some(a) = app()
            {
                a.request_remote(true);
            }
        });
    }

    pub fn widgets(&self) -> Option<Rc<Widgets>> {
        self.w.borrow().clone()
    }

    pub fn first_dow(&self) -> u32 {
        // Windows by locale: Monday in Dutch, Sunday in English (US).
        if self.cfg.borrow().en { 0 } else { 1 }
    }

    pub fn h24(&self) -> bool {
        self.cfg.borrow().h24
    }

    // ------------------------------------------------------------------------------------------ building

    fn build_ui(self: &Rc<Self>) {
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("cal-root");
        overlay.set_overflow(gtk::Overflow::Hidden);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        overlay.set_child(Some(&content));

        let (titlebar, max_button, max_glyph) = self.title_bar();
        content.append(&titlebar);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        content.append(&body);

        // ---- navigation pane
        let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pane.add_css_class("cal-pane");
        // Fixed width: without this the expanding labels inside (mini month title, calendar names) make the pane
        // share the extra width with the main layer, and its width changed with the month shown.
        pane.set_hexpand(false);
        let (ham, _) = icon_button("menu", 36, t("Navigatievenster", "Navigation pane"));
        ham.add_css_class("cal-ham");
        ham.set_halign(gtk::Align::Start);
        ham.connect_clicked(|_| {
            if let Some(a) = app() {
                a.toggle_pane();
            }
        });
        pane.append(&ham);
        let (new_small, _) = icon_button("add", 36, t("Nieuwe gebeurtenis", "New event"));
        new_small.add_css_class("cal-new-small");
        new_small.set_halign(gtk::Align::Start);
        new_small.connect_clicked(|_| {
            if let Some(a) = app() {
                let d = a.st.borrow().sel;
                a.new_event_on(d, None);
            }
        });
        pane.append(&new_small);

        let pane_full = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pane_full.set_vexpand(true);
        let new_button = gtk::Button::new();
        new_button.set_widget_name("cal-new");
        new_button.add_css_class("w11-button");
        new_button.add_css_class("w11-accent");
        new_button.add_css_class("cal-new");
        let nb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        nb.set_halign(gtk::Align::Center);
        nb.append(&glyphs::glyph("add", 16));
        nb.append(&gtk::Label::new(Some(t("Nieuwe gebeurtenis", "New event"))));
        new_button.set_child(Some(&nb));
        new_button.connect_clicked(|_| {
            if let Some(a) = app() {
                let d = a.st.borrow().sel;
                a.new_event_on(d, None);
            }
        });
        pane_full.append(&new_button);

        let pane_scroll = gtk::ScrolledWindow::new();
        pane_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        pane_scroll.set_vexpand(true);
        let pane_inner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let mini = mini::MiniMonth::new();
        mini.root.add_css_class("cal-pane-mini");
        mini.connect_pick(|d| {
            defer(move |a| a.pick_date(d));
        });
        mini.connect_month(|m| {
            if let Some(a) = app() {
                a.st.borrow_mut().mini = m;
            }
        });
        pane_inner.append(&mini.root);
        let sep = gtk::Separator::new(gtk::Orientation::Horizontal);
        sep.add_css_class("w11-divider");
        sep.add_css_class("cal-pane-sep");
        pane_inner.append(&sep);
        let cal_list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        cal_list.add_css_class("cal-callist");
        pane_inner.append(&cal_list);
        pane_scroll.set_child(Some(&pane_inner));
        pane_full.append(&pane_scroll);
        pane.append(&pane_full);
        body.append(&pane);

        // ---- main layer
        let main = gtk::Box::new(gtk::Orientation::Vertical, 0);
        main.add_css_class("cal-main");
        main.set_hexpand(true);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        toolbar.add_css_class("cal-toolbar");
        let today = gtk::Button::new();
        today.set_widget_name("cal-today");
        today.add_css_class("w11-button");
        today.add_css_class("w11-subtle");
        today.add_css_class("cal-tool");
        let tb = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tb.append(&glyphs::glyph("calendar-today", 16));
        tb.append(&gtk::Label::new(Some(t("Vandaag", "Today"))));
        today.set_child(Some(&tb));
        today.set_tooltip_text(Some(t("Naar vandaag gaan", "Go to today")));
        today.connect_clicked(|_| {
            if let Some(a) = app() {
                a.go_today();
            }
        });
        toolbar.append(&today);
        let (prev, prev_glyph) = icon_button("arrow-up", 32, t("Vorige", "Previous"));
        prev.set_widget_name("cal-prev");
        prev.connect_clicked(|_| {
            if let Some(a) = app() {
                a.shift(-1);
            }
        });
        let (next, next_glyph) = icon_button("arrow-down", 32, t("Volgende", "Next"));
        next.set_widget_name("cal-next");
        next.connect_clicked(|_| {
            if let Some(a) = app() {
                a.shift(1);
            }
        });
        toolbar.append(&prev);
        toolbar.append(&next);
        let title = gtk::Label::new(None);
        title.set_widget_name("cal-title");
        title.add_css_class("cal-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        toolbar.append(&title);

        let switch = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        switch.add_css_class("cal-viewswitch");
        let mut views = Vec::new();
        let mut group: Option<gtk::ToggleButton> = None;
        for (view, glyph, nl, en, name) in [
            (View::Day, "day", "Dag", "Day", "cal-view-day"),
            (View::Week, "week", "Week", "Week", "cal-view-week"),
            (View::Month, "month", "Maand", "Month", "cal-view-month"),
        ] {
            let b = gtk::ToggleButton::new();
            b.set_widget_name(name);
            b.add_css_class("cal-viewbtn");
            let bx = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            bx.append(&glyphs::glyph(glyph, 16));
            bx.append(&gtk::Label::new(Some(t(nl, en))));
            b.set_child(Some(&bx));
            if let Some(g) = &group {
                b.set_group(Some(g));
            } else {
                group = Some(b.clone());
            }
            b.connect_toggled(move |b| {
                if !b.is_active() {
                    return;
                }
                if let Some(a) = app()
                    && !a.updating.get()
                {
                    defer(move |a| a.set_view(view));
                }
            });
            switch.append(&b);
            views.push((view, b));
        }
        toolbar.append(&switch);
        main.append(&toolbar);

        let view_host = gtk::Box::new(gtk::Orientation::Vertical, 0);
        view_host.add_css_class("cal-viewhost");
        view_host.set_vexpand(true);
        view_host.set_hexpand(true);
        main.append(&view_host);
        body.append(&main);

        let edges = edges(&overlay);
        self.window.set_child(Some(&overlay));
        *self.dialog.borrow_mut() = None;
        *self.w.borrow_mut() = Some(Rc::new(Widgets {
            overlay,
            content,
            title,
            prev,
            next,
            prev_glyph,
            next_glyph,
            views,
            pane,
            pane_full,
            new_small,
            ham,
            mini,
            cal_list,
            view_host,
            max_button,
            max_glyph,
            edges,
        }));
        self.update_square();
    }

    fn title_bar(self: &Rc<Self>) -> (gtk::WindowHandle, gtk::Button, gtk::DrawingArea) {
        let handle = gtk::WindowHandle::new();
        handle.add_css_class("cal-titlebar");
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let icon = glyphs::glyph("calendar", 16);
        icon.add_css_class("cal-appicon");
        bar.append(&icon);
        let label = gtk::Label::new(Some(t("Agenda", "Calendar")));
        label.add_css_class("cal-apptitle");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        bar.append(&label);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        bar.append(&spacer);
        let caption = |glyph: &'static str, tip: &str, close: bool| {
            let b = gtk::Button::new();
            b.add_css_class("cal-cap");
            if close {
                b.add_css_class("cal-cap-close");
            }
            b.set_size_request(46, 32);
            b.set_valign(gtk::Align::Start);
            b.set_can_focus(false);
            b.set_focus_on_click(false);
            b.set_tooltip_text(Some(tip));
            let g = glyphs::glyph(glyph, 10);
            g.add_css_class("cal-cap-glyph");
            b.set_child(Some(&g));
            (b, g)
        };
        let (min, _) = caption("cap-min", t("Minimaliseren", "Minimise"), false);
        min.set_widget_name("cal-minimize");
        min.connect_clicked(|_| {
            if let Some(a) = app() {
                a.window.minimize();
            }
        });
        let (max, max_glyph) = caption("cap-max", t("Maximaliseren", "Maximise"), false);
        max.set_widget_name("cal-maximize");
        max.connect_clicked(|_| {
            if let Some(a) = app() {
                if a.window.is_maximized() {
                    a.window.unmaximize();
                } else {
                    a.window.maximize();
                }
            }
        });
        let (close, _) = caption("cap-close", t("Sluiten", "Close"), true);
        close.set_widget_name("cal-close");
        close.connect_clicked(|_| {
            if let Some(a) = app() {
                a.window.close();
            }
        });
        bar.append(&min);
        bar.append(&max);
        bar.append(&close);
        handle.set_child(Some(&bar));
        (handle, max, max_glyph)
    }

    /// Square corners and no resize edges while maximized, fullscreen or tiled; the caption button follows.
    fn update_square(&self) {
        let w = &self.window;
        let mut square = w.is_maximized() || w.is_fullscreen();
        let mut fixed = square;
        if let Some(surface) = w.surface()
            && let Ok(top) = surface.downcast::<gdk::Toplevel>()
        {
            let s = top.state();
            let tiled = gdk::ToplevelState::TILED
                | gdk::ToplevelState::TOP_TILED
                | gdk::ToplevelState::RIGHT_TILED
                | gdk::ToplevelState::BOTTOM_TILED
                | gdk::ToplevelState::LEFT_TILED;
            if s.intersects(tiled) {
                square = true;
            }
            if s.intersects(gdk::ToplevelState::MAXIMIZED | gdk::ToplevelState::FULLSCREEN) {
                square = true;
                fixed = true;
            }
        }
        if square {
            w.add_css_class("cal-square");
        } else {
            w.remove_css_class("cal-square");
        }
        let Some(widgets) = self.widgets() else { return };
        for e in &widgets.edges {
            e.set_visible(!fixed);
        }
        let maximized = w.is_maximized();
        glyphs::set(&widgets.max_glyph, if maximized { "cap-restore" } else { "cap-max" });
        widgets.max_button.set_tooltip_text(Some(if maximized {
            t("Vorige grootte", "Restore down")
        } else {
            t("Maximaliseren", "Maximise")
        }));
    }

    fn install_keys(self: &Rc<Self>) {
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(|_, key, _, state| {
            let Some(a) = app() else { return glib::Propagation::Proceed };
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let alt = state.contains(gdk::ModifierType::ALT_MASK);
            if a.dialog_open() {
                if key == gdk::Key::Escape {
                    a.close_dialog();
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }
            let handled = match key {
                gdk::Key::n | gdk::Key::N if ctrl => {
                    let d = a.st.borrow().sel;
                    a.new_event_on(d, None);
                    true
                }
                gdk::Key::t | gdk::Key::T if ctrl => {
                    a.go_today();
                    true
                }
                gdk::Key::Page_Up => {
                    a.shift(-1);
                    true
                }
                gdk::Key::Page_Down => {
                    a.shift(1);
                    true
                }
                gdk::Key::_1 if ctrl && alt => {
                    a.set_view(View::Day);
                    true
                }
                gdk::Key::_2 | gdk::Key::_3 if ctrl && alt => {
                    a.set_view(View::Week);
                    true
                }
                gdk::Key::_4 if ctrl && alt => {
                    a.set_view(View::Month);
                    true
                }
                _ => false,
            };
            if handled { glib::Propagation::Stop } else { glib::Propagation::Proceed }
        });
        self.window.add_controller(keys);
    }

    fn watch_settings(self: &Rc<Self>) {
        let file = gio::File::for_path(config::settings_path());
        let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, None::<&gio::Cancellable>) else { return };
        monitor.connect_changed(|_, _, _, event| {
            if matches!(event, gio::FileMonitorEvent::AttributeChanged | gio::FileMonitorEvent::PreUnmount) {
                return;
            }
            let Some(a) = app() else { return };
            if a.reload_pending.replace(true) {
                return;
            }
            glib::timeout_add_local_once(Duration::from_millis(150), || {
                if let Some(a) = app() {
                    a.reload_pending.set(false);
                    a.reload_settings();
                }
            });
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    fn reload_settings(self: &Rc<Self>) {
        let cfg = config::load();
        let old = self.cfg.borrow().clone();
        if cfg == old {
            return;
        }
        let relabel = cfg.en != old.en;
        *self.cfg.borrow_mut() = cfg;
        theme::apply(&self.cfg.borrow(), &self.store.calendars.borrow());
        crate::backdrop::settings(&self.cfg.borrow());
        if relabel {
            self.window.set_title(Some(t("Agenda", "Calendar")));
            self.build_ui();
        }
        self.render();
    }

    fn start_clock(self: &Rc<Self>) {
        glib::timeout_add_seconds_local(30, || {
            let Some(a) = app() else { return glib::ControlFlow::Break };
            let today = dates::today();
            let changed = a.st.borrow().today != today;
            if changed {
                a.st.borrow_mut().today = today;
                if !a.dialog_open() {
                    a.render();
                }
            } else {
                a.now_areas.borrow_mut().retain(|w| w.upgrade().is_some());
                for area in a.now_areas.borrow().iter().filter_map(|w| w.upgrade()) {
                    area.queue_draw();
                }
            }
            glib::ControlFlow::Continue
        });
    }

    fn save_ui_state(&self) {
        let (view, pane) = {
            let st = self.st.borrow();
            (st.view, st.pane)
        };
        let text = json!({ "view": view.key(), "pane": pane }).to_string();
        let _ = store::write_atomic(&ui_state_path(), text.as_bytes());
    }

    // ------------------------------------------------------------------------------------------ GNOME events

    fn remote_changed(self: &Rc<Self>) {
        *self.store.remote.borrow_mut() = self.remote.events.borrow().clone();
        self.store.remote_present.set(self.remote.present());
        self.render_calendars();
        self.render_view();
    }

    fn request_remote(&self, force: bool) {
        let (first, n) = self.view_range();
        let from = dates::day_number(first) * DAY;
        let since = dates::civil_to_unix(from);
        let until = dates::civil_to_unix(from + n as i64 * DAY);
        self.remote.request(since, until, force);
    }

    // ------------------------------------------------------------------------------------------ rendering

    /// The days the current view shows: its first day and how many.
    pub fn view_range(&self) -> (Ymd, usize) {
        let fd = self.first_dow();
        let st = self.st.borrow();
        match st.view {
            View::Month => {
                let first = (st.sel.0, st.sel.1, 1);
                let start = dates::week_start(first, fd);
                let lead = dates::day_number(first) - dates::day_number(start);
                let dim = dates::days_in_month(st.sel.0, st.sel.1) as i64;
                let weeks = (lead + dim + 6) / 7;
                (start, (weeks * 7) as usize)
            }
            View::Week => (dates::week_start(st.sel, fd), 7),
            View::Day => (st.sel, 1),
        }
    }

    fn title_text(&self) -> String {
        let en = config::en();
        let fd = self.first_dow();
        let st = self.st.borrow();
        match st.view {
            View::Month => dates::month_title(st.sel.0, st.sel.1, en),
            View::Week => {
                let a = dates::week_start(st.sel, fd);
                let b = dates::add_days(a, 6);
                if (a.0, a.1) == (b.0, b.1) {
                    dates::month_title(a.0, a.1, en)
                } else if a.0 == b.0 {
                    format!("{} \u{2013} {} {}", dates::month_name(a.1, en), dates::month_name(b.1, en), b.0)
                } else {
                    format!("{} \u{2013} {}", dates::month_title(a.0, a.1, en), dates::month_title(b.0, b.1, en))
                }
            }
            View::Day => dates::full_date(st.sel, en),
        }
    }

    pub fn render(self: &Rc<Self>) {
        self.render_chrome();
        self.render_calendars();
        self.render_view();
        self.request_remote(false);
    }

    fn render_chrome(self: &Rc<Self>) {
        let Some(w) = self.widgets() else { return };
        let (view, sel, mini_m, pane, today) = {
            let st = self.st.borrow();
            (st.view, st.sel, st.mini, st.pane, st.today)
        };
        self.updating.set(true);
        w.title.set_label(&self.title_text());
        for (v, b) in &w.views {
            b.set_active(*v == view);
        }
        let (pg, ng, pt, nt) = match view {
            View::Month => ("arrow-up", "arrow-down", t("Vorige maand", "Previous month"), t("Volgende maand", "Next month")),
            View::Week => ("arrow-left", "arrow-right", t("Vorige week", "Previous week"), t("Volgende week", "Next week")),
            View::Day => ("arrow-left", "arrow-right", t("Vorige dag", "Previous day"), t("Volgende dag", "Next day")),
        };
        glyphs::set(&w.prev_glyph, pg);
        glyphs::set(&w.next_glyph, ng);
        w.prev.set_tooltip_text(Some(pt));
        w.next.set_tooltip_text(Some(nt));
        w.pane_full.set_visible(pane);
        w.new_small.set_visible(!pane);
        if pane {
            w.pane.remove_css_class("is-collapsed");
        } else {
            w.pane.add_css_class("is-collapsed");
        }
        w.ham.set_tooltip_text(Some(if pane {
            t("Navigatievenster samenvouwen", "Collapse navigation pane")
        } else {
            t("Navigatievenster uitvouwen", "Expand navigation pane")
        }));
        let (first, n) = self.view_range();
        let hl = if view == View::Month {
            None
        } else {
            Some((dates::day_number(first), dates::day_number(first) + n as i64))
        };
        w.mini.set(mini_m, sel, today, self.first_dow(), hl);
        self.updating.set(false);
    }

    pub fn render_calendars(self: &Rc<Self>) {
        let Some(w) = self.widgets() else { return };
        clear(&w.cal_list);
        self.updating.set(true);
        let header = gtk::Label::new(Some(t("Mijn agenda's", "My calendars")));
        header.add_css_class("cal-pane-header");
        header.set_xalign(0.0);
        w.cal_list.append(&header);
        let cals = self.store.calendars.borrow().clone();
        for c in cals.iter().filter(|c| c.id != store::REMOTE_ID) {
            w.cal_list.append(&self.calendar_row(c));
        }
        if self.store.remote_present.get()
            && let Some(c) = cals.iter().find(|c| c.id == store::REMOTE_ID)
        {
            let header = gtk::Label::new(Some("GNOME"));
            header.add_css_class("cal-pane-header");
            header.set_xalign(0.0);
            w.cal_list.append(&header);
            let row = self.calendar_row(c);
            row.set_tooltip_text(Some(t(
                "Gebeurtenissen uit GNOME Agenda en online accounts (alleen-lezen)",
                "Events from GNOME Calendar and online accounts (read-only)",
            )));
            w.cal_list.append(&row);
        }
        self.updating.set(false);
    }

    fn calendar_row(self: &Rc<Self>, c: &CalendarInfo) -> gtk::CheckButton {
        let check = gtk::CheckButton::new();
        check.add_css_class("w11-check");
        check.add_css_class("cal-cal-check");
        check.add_css_class(&c.css());
        check.set_widget_name(&format!("cal-calendar-{}", c.id));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.append(&glyphs::dot(theme::color_rgba(&c.color), 10));
        let label = gtk::Label::new(Some(&c.name()));
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&label);
        check.set_child(Some(&row));
        check.set_active(c.visible);
        let id = c.id.clone();
        check.connect_toggled(move |b| {
            let Some(a) = app() else { return };
            if a.updating.get() {
                return;
            }
            a.store.set_visible(&id, b.is_active());
            let _ = a.save_store();
            defer(|a| a.render_view());
        });
        check
    }

    pub fn render_view(self: &Rc<Self>) {
        let Some(w) = self.widgets() else { return };
        clear(&w.view_host);
        self.cells.borrow_mut().clear();
        self.now_areas.borrow_mut().clear();
        let (view, sel) = {
            let st = self.st.borrow();
            (st.view, st.sel)
        };
        let widget = match view {
            View::Month => month::build(self),
            View::Week => {
                let (first, _) = self.view_range();
                timeline::build(self, first, 7)
            }
            View::Day => timeline::build(self, sel, 1),
        };
        w.view_host.append(&widget);
    }

    // ------------------------------------------------------------------------------------------ navigation

    pub fn shift(self: &Rc<Self>, sign: i32) {
        {
            let mut st = self.st.borrow_mut();
            st.sel = match st.view {
                View::Month => {
                    let (y, m) = dates::add_months(st.sel.0, st.sel.1, sign);
                    (y, m, st.sel.2.min(dates::days_in_month(y, m)))
                }
                View::Week => dates::add_days(st.sel, 7 * sign as i64),
                View::Day => dates::add_days(st.sel, sign as i64),
            };
            st.mini = (st.sel.0, st.sel.1);
        }
        self.render();
    }

    pub fn go_today(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.today = dates::today();
            st.sel = st.today;
            st.mini = (st.sel.0, st.sel.1);
            st.scroll = None;
        }
        self.render();
    }

    pub fn set_view(self: &Rc<Self>, view: View) {
        self.st.borrow_mut().view = view;
        self.save_ui_state();
        self.render();
    }

    /// Show a day in a view (week header click, "+N meer").
    pub fn open_day(self: &Rc<Self>, d: Ymd, view: View) {
        {
            let mut st = self.st.borrow_mut();
            st.sel = d;
            st.mini = (d.0, d.1);
            st.view = view;
        }
        self.save_ui_state();
        self.render();
    }

    fn toggle_pane(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.pane = !st.pane;
        }
        self.save_ui_state();
        self.render_chrome();
    }

    /// A day picked in the mini calendar.
    pub fn pick_date(self: &Rc<Self>, d: Ymd) {
        {
            let mut st = self.st.borrow_mut();
            st.sel = d;
            st.mini = (d.0, d.1);
        }
        self.render();
    }

    /// Select a day in the current view without rebuilding it (a click on a month cell or a timeline column).
    pub fn select_in_place(self: &Rc<Self>, d: Ymd) {
        let (view, sel) = {
            let st = self.st.borrow();
            (st.view, st.sel)
        };
        if view == View::Month && (d.0, d.1) != (sel.0, sel.1) {
            defer(move |a| a.pick_date(d));
            return;
        }
        {
            let mut st = self.st.borrow_mut();
            st.sel = d;
            st.mini = (d.0, d.1);
        }
        for (day, cell) in self.cells.borrow().iter() {
            if *day == d {
                cell.add_css_class("is-selected");
            } else {
                cell.remove_css_class("is-selected");
            }
        }
        self.render_chrome();
    }

    // ------------------------------------------------------------------------------------------ events

    pub fn save_store(&self) -> Result<(), String> {
        let r = self.store.save();
        if let Err(e) = &r {
            eprintln!("win11-calendar: saving {}: {e}", store::events_path().display());
        }
        r
    }

    /// A new timed event on a day: at `minute`, or the next whole hour today, or 09:00 on another day.
    pub fn new_event_on(self: &Rc<Self>, d: Ymd, minute: Option<i64>) {
        let today = dates::today();
        let m = minute.unwrap_or_else(|| {
            if d == today { ((dates::now_minutes().rem_euclid(DAY) / 60 + 1) * 60).min(23 * 60) } else { 9 * 60 }
        });
        let start = dates::at(d, m);
        editor::open(self, editor::Init::New { start, end: start + 60, all_day: false });
    }

    /// A new all-day event (a double click on a month cell).
    pub fn new_all_day(self: &Rc<Self>, d: Ymd) {
        let start = dates::at(d, 0);
        editor::open(self, editor::Init::New { start, end: start + DAY, all_day: true });
    }

    pub fn open_event(self: &Rc<Self>, id: &str) {
        if let Some(e) = self.store.find(id) {
            editor::open(self, editor::Init::Edit(e));
        }
    }

    /// Store an event from the editor.
    pub fn commit(self: &Rc<Self>, e: Event) -> Result<(), String> {
        self.store.upsert(e);
        self.save_store()?;
        self.render_view();
        Ok(())
    }

    pub fn remove_event(self: &Rc<Self>, id: &str) -> Result<(), String> {
        self.store.delete(id);
        self.save_store()?;
        self.render_view();
        Ok(())
    }

    // ------------------------------------------------------------------------------------------ dialogs

    pub fn dialog_open(&self) -> bool {
        self.dialog.borrow().is_some()
    }

    pub fn show_dialog(&self, layer: &gtk::Widget) {
        self.close_dialog();
        let Some(w) = self.widgets() else { return };
        w.overlay.add_overlay(layer);
        w.content.set_can_focus(false);
        *self.dialog.borrow_mut() = Some(layer.clone());
    }

    pub fn close_dialog(&self) {
        let Some(layer) = self.dialog.borrow_mut().take() else { return };
        if let Some(w) = self.widgets() {
            if layer.parent().as_ref() == Some(w.overlay.upcast_ref::<gtk::Widget>()) {
                w.overlay.remove_overlay(&layer);
            }
            w.content.set_can_focus(true);
        }
    }
}

/// Invisible strips along the window's edges and corners that start a compositor resize (the window has no
/// decorations of its own).
fn edges(overlay: &gtk::Overlay) -> Vec<gtk::Widget> {
    use gdk::SurfaceEdge as E;
    use gtk::Align::{End, Fill, Start};
    let specs = [
        (E::North, Fill, Start, -1, 4, "n-resize"),
        (E::South, Fill, End, -1, 5, "s-resize"),
        (E::West, Start, Fill, 5, -1, "w-resize"),
        (E::East, End, Fill, 5, -1, "e-resize"),
        (E::NorthWest, Start, Start, 10, 10, "nw-resize"),
        (E::NorthEast, End, Start, 10, 6, "ne-resize"),
        (E::SouthWest, Start, End, 12, 12, "sw-resize"),
        (E::SouthEast, End, End, 12, 12, "se-resize"),
    ];
    let mut out = Vec::new();
    for (edge, h, v, width, height, cursor) in specs {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        b.add_css_class("cal-edge");
        b.set_halign(h);
        b.set_valign(v);
        b.set_size_request(width, height);
        b.set_cursor_from_name(Some(cursor));
        let g = gtk::GestureClick::new();
        g.set_button(1);
        g.connect_pressed(move |g, _, x, y| begin_resize(g, edge, x, y));
        b.add_controller(g);
        overlay.add_overlay(&b);
        out.push(b.upcast());
    }
    out
}

fn begin_resize(g: &gtk::GestureClick, edge: gdk::SurfaceEdge, x: f64, y: f64) {
    let Some(w) = g.widget() else { return };
    let Some(native) = w.native() else { return };
    let Some(surface) = native.surface() else { return };
    let Ok(top) = surface.downcast::<gdk::Toplevel>() else { return };
    let Some(p) = w.compute_point(native.upcast_ref::<gtk::Widget>(), &graphene::Point::new(x as f32, y as f32)) else {
        return;
    };
    let (tx, ty) = native.surface_transform();
    g.set_state(gtk::EventSequenceState::Claimed);
    let device = g.device();
    top.begin_resize(edge, device.as_ref(), 1, p.x() as f64 + tx, p.y() as f64 + ty, g.current_event_time());
}
