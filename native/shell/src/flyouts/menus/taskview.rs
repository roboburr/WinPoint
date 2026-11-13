//! Taakweergave (vendor/ui/js/shell/taskview.js): the whole monitor above the taskbar, tinted, with a card per open
//! window (app icon and title, a little window mock-up with the app icon large as the "thumbnail"), the window you
//! were using first. Click = focus, the cross = close, keyboard: arrows, Tab, Home/End, Enter, Delete, Esc.
use super::util;
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

pub struct TaskView {
    shell: Rc<Shell>,
    root: gtk::Box,
    flow: gtk::FlowBox,
    scroller: gtk::ScrolledWindow,
    empty: gtk::Box,
    wins: RefCell<Vec<Value>>,
    cards: RefCell<Vec<gtk::Box>>,
    sel: Cell<usize>,
    cols: Cell<usize>,
    /// Most recently used first (window ids), from the focus events.
    mru: RefCell<Vec<String>>,
}

thread_local! {
    static TV: RefCell<Option<Rc<TaskView>>> = const { RefCell::new(None) };
}

struct Handle(Rc<TaskView>);

impl Flyout for Handle {
    fn name(&self) -> &'static str {
        "taskview"
    }
    fn widget(&self) -> gtk::Widget {
        self.0.root.clone().upcast()
    }
    fn placement(&self) -> Placement {
        Placement::BottomCenter
    }
    fn opened(&self) {
        let tv = self.0.clone();
        tv.fill_frame();
        tv.sel.set(0);
        *tv.wins.borrow_mut() = tv.gather();
        tv.render();
        glib::idle_add_local_once(move || {
            tv.root.grab_focus();
        });
    }
    fn closed(&self) {
        self.0.flow.remove_all();
        self.0.cards.borrow_mut().clear();
    }
}

fn id_of(w: &Value) -> String {
    match &w["id"] {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

pub fn register(shell: &Rc<Shell>) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("w11-taskview");
    root.set_widget_name("menus-taskview");
    root.set_focusable(true);
    root.set_hexpand(true);
    root.set_vexpand(true);

    let flow = gtk::FlowBox::new();
    flow.add_css_class("w11-tv-grid");
    flow.set_selection_mode(gtk::SelectionMode::None);
    flow.set_homogeneous(true);
    flow.set_column_spacing(24);
    flow.set_row_spacing(28);
    flow.set_halign(gtk::Align::Center);
    flow.set_valign(gtk::Align::Center);
    flow.set_activate_on_single_click(false);
    flow.set_can_focus(false);
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_vexpand(true);
    scroller.set_hexpand(true);
    scroller.set_child(Some(&flow));
    scroller.add_css_class("w11-tv-scroll");

    let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
    empty.add_css_class("w11-tv-empty");
    empty.set_halign(gtk::Align::Center);
    empty.set_valign(gtk::Align::Center);
    empty.set_vexpand(true);
    let title = gtk::Label::new(Some(&shell.t("Er zijn geen geopende vensters", "There are no open windows")));
    title.add_css_class("w11-tv-empty-title");
    let hint = gtk::Label::new(Some(&shell.t("Apps die je opent, verschijnen hier.", "Apps you open appear here.")));
    hint.add_css_class("w11-tv-empty-hint");
    empty.append(&title);
    empty.append(&hint);
    empty.set_visible(false);

    root.append(&scroller);
    root.append(&empty);

    let tv = Rc::new(TaskView {
        shell: shell.clone(),
        root: root.clone(),
        flow,
        scroller,
        empty,
        wins: RefCell::new(Vec::new()),
        cards: RefCell::new(Vec::new()),
        sel: Cell::new(0),
        cols: Cell::new(1),
        mru: RefCell::new(Vec::new()),
    });

    // a click beside the cards closes Task View
    let click = gtk::GestureClick::new();
    let weak = Rc::downgrade(&tv);
    click.connect_released(move |_, _, x, y| {
        let Some(tv) = weak.upgrade() else { return };
        let target = tv.root.pick(x, y, gtk::PickFlags::DEFAULT);
        let on_card = target.is_some_and(|t| {
            let mut w = Some(t);
            while let Some(c) = w {
                if c.has_css_class("w11-tv-card") {
                    return true;
                }
                w = c.parent();
            }
            false
        });
        if !on_card {
            tv.shell.flyouts().close();
        }
    });
    root.add_controller(click);

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&tv);
    keys.connect_key_pressed(move |_, key, _, state| {
        let Some(tv) = weak.upgrade() else { return glib::Propagation::Proceed };
        if tv.key(key, state) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    root.add_controller(keys);

    shell.flyouts().register(Rc::new(Handle(tv.clone())));
    tv.fill_frame();
    TV.with(|t| *t.borrow_mut() = Some(tv));

    // the focus history, and an open Task View follows the window list
    shell.on("windows", |event, data| {
        let Some(tv) = TV.with(|t| t.borrow().clone()) else { return };
        if let Some(list) = data["windows"].as_array() {
            tv.touch(list);
        }
        if event == "windows.focused"
            && let Some(id) = data["id"].as_str().map(str::to_owned).or_else(|| data["id"].as_u64().map(|n| n.to_string()))
        {
            tv.touch_id(&id);
        }
        if tv.shell.flyouts().is_open("taskview") {
            tv.refresh();
        }
    });
}

impl TaskView {
    /// Task View takes the whole monitor above the taskbar: the host's frame fills the overlay.
    fn fill_frame(&self) {
        if let Some(frame) = self.root.parent() {
            frame.set_halign(gtk::Align::Fill);
            frame.set_valign(gtk::Align::Fill);
            frame.set_margin_start(0);
            frame.set_margin_end(0);
            frame.set_margin_top(0);
            frame.set_margin_bottom(crate::panel::BAR);
        }
    }

    fn touch(&self, list: &[Value]) {
        if let Some(f) = list.iter().find(|w| w["focused"] == true && w["minimized"] != true) {
            self.touch_id(&id_of(f));
        }
    }

    fn touch_id(&self, id: &str) {
        if id.is_empty() {
            return;
        }
        let mut mru = self.mru.borrow_mut();
        if mru.first().map(String::as_str) == Some(id) {
            return;
        }
        mru.retain(|x| x != id);
        mru.insert(0, id.to_owned());
        mru.truncate(200);
    }

    /// The open windows, the one in use first, then by focus history, then as listed.
    fn gather(&self) -> Vec<Value> {
        let mut seen = Vec::<String>::new();
        let all: Vec<Value> = self
            .shell
            .gnome
            .windows()
            .into_iter()
            .filter(|w| {
                let id = id_of(w);
                if id.is_empty() || seen.contains(&id) {
                    return false;
                }
                seen.push(id);
                true
            })
            .collect();
        self.touch(&all);
        let mru = self.mru.borrow();
        let rank = |w: &Value| mru.iter().position(|x| *x == id_of(w)).unwrap_or(usize::MAX);
        let active = |w: &Value| w["focused"] == true && w["minimized"] != true;
        let mut indexed: Vec<(usize, Value)> = all.into_iter().enumerate().collect();
        indexed.sort_by(|(ia, a), (ib, b)| active(b).cmp(&active(a)).then(rank(a).cmp(&rank(b))).then(ia.cmp(ib)));
        indexed.into_iter().map(|(_, w)| w).collect()
    }

    /// Columns and card width for n windows (taskview.js layoutFor).
    fn layout(&self, n: usize) -> (usize, i32) {
        let vw = util::monitor_width(&self.root) as f64;
        let vh = (util::monitor_height(&self.root) - crate::panel::BAR) as f64;
        let cols = match n {
            0 | 1 => 1,
            2 | 3 => n,
            4 => 2,
            5 | 6 => 3,
            _ => 4,
        };
        let rows = n.div_ceil(cols).max(1) as f64;
        let gap = 24.0;
        let max_w = 360f64.min((vw * 0.84 - gap * (cols as f64 - 1.0)) / cols as f64);
        let max_h = (vh * 0.78 - gap * (rows - 1.0)) / rows - 36.0;
        let w = 180f64.max(max_w.min(max_h * 1.6));
        (cols, w.round() as i32)
    }

    fn title_of(&self, w: &Value) -> String {
        let t = util::str_of(w, "title");
        if !t.is_empty() {
            return t;
        }
        let a = util::str_of(w, "appId");
        if !a.is_empty() { a } else { self.shell.t("Venster", "Window") }
    }

    fn icon_of(w: &Value) -> String {
        let icon = util::str_of(w, "icon");
        if !icon.is_empty() {
            return icon;
        }
        let desktop = util::str_of(w, "desktopId");
        if !desktop.is_empty() {
            return desktop.trim_end_matches(".desktop").to_owned();
        }
        let app = util::str_of(w, "appId");
        if app.is_empty() { "application-x-executable".into() } else { app }
    }

    fn render(self: &Rc<Self>) {
        self.flow.remove_all();
        self.cards.borrow_mut().clear();
        let wins = self.wins.borrow().clone();
        self.empty.set_visible(wins.is_empty());
        self.scroller.set_visible(!wins.is_empty());
        if wins.is_empty() {
            return;
        }
        let (cols, w) = self.layout(wins.len());
        self.cols.set(cols);
        self.flow.set_min_children_per_line(cols as u32);
        self.flow.set_max_children_per_line(cols as u32);
        self.sel.set(self.sel.get().min(wins.len() - 1));
        let mut cards = Vec::new();
        for (i, win) in wins.iter().enumerate() {
            let card = self.card(i, win, w);
            self.flow.insert(&card, -1);
            if let Some(child) = self.flow.child_at_index(i as i32) {
                child.set_focusable(false);
                child.add_css_class("w11-tv-slot");
            }
            cards.push(card);
        }
        *self.cards.borrow_mut() = cards;
        self.sync_sel();
    }

    fn card(self: &Rc<Self>, i: usize, win: &Value, w: i32) -> gtk::Box {
        let title = self.title_of(win);
        let icon = Self::icon_of(win);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
        card.add_css_class("w11-tv-card");
        card.set_widget_name(&format!("menus-taskview-{}", id_of(win)));
        card.set_size_request(w, -1);
        card.set_tooltip_text(Some(&title));
        if win["minimized"] == true {
            card.add_css_class("is-minimized");
        }

        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.add_css_class("w11-tv-head");
        head.set_size_request(-1, 24);
        let small = util::icon(&self.shell, &icon, 16);
        small.set_valign(gtk::Align::Center);
        head.append(&small);
        let label = gtk::Label::new(Some(&title));
        label.add_css_class("w11-tv-title");
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_width_chars(1);
        head.append(&label);
        let close = gtk::Button::new();
        close.add_css_class("w11-tv-close");
        close.set_child(Some(&crate::icons::glyph("close", 12)));
        close.set_focusable(false);
        close.set_valign(gtk::Align::Center);
        close.set_size_request(24, 24);
        close.set_tooltip_text(Some(&self.shell.t("Sluiten", "Close")));
        close.set_widget_name(&format!("menus-taskview-close-{}", id_of(win)));
        let weak = Rc::downgrade(self);
        close.connect_clicked(move |_| {
            if let Some(tv) = weak.upgrade() {
                tv.do_close(i);
            }
        });
        head.append(&close);
        card.append(&head);

        // the "thumbnail": a little window with the app icon large in the middle
        let thumb_w = (w - 12).max(60);
        let thumb = gtk::Box::new(gtk::Orientation::Vertical, 0);
        thumb.add_css_class("w11-tv-thumb");
        thumb.set_size_request(thumb_w, thumb_w * 10 / 16);
        thumb.set_overflow(gtk::Overflow::Hidden);
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bar.add_css_class("w11-tv-thumb-bar");
        bar.set_size_request(-1, 18);
        bar.set_halign(gtk::Align::Fill);
        let marks = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        marks.set_halign(gtk::Align::End);
        marks.set_hexpand(true);
        marks.set_valign(gtk::Align::Center);
        for _ in 0..3 {
            let m = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            m.add_css_class("w11-tv-thumb-mark");
            m.set_size_request(7, 2);
            marks.append(&m);
        }
        bar.append(&marks);
        thumb.append(&bar);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.set_vexpand(true);
        body.set_valign(gtk::Align::Fill);
        let big_px = (96f64.min(w as f64 * 0.26)).round() as i32;
        let big = util::icon(&self.shell, &icon, big_px);
        big.set_halign(gtk::Align::Center);
        big.set_valign(gtk::Align::Center);
        big.set_vexpand(true);
        body.append(&big);
        thumb.append(&body);
        card.append(&thumb);

        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(self);
        let close_weak = close.downgrade();
        click.connect_released(move |g, _, x, y| {
            let Some(tv) = weak.upgrade() else { return };
            // a click on the cross is the cross's
            if let (Some(card), Some(close)) = (g.widget(), close_weak.upgrade())
                && let Some(hit) = card.pick(x, y, gtk::PickFlags::DEFAULT)
                && (hit == close.clone().upcast::<gtk::Widget>() || hit.is_ancestor(&close))
            {
                return;
            }
            tv.activate(i);
        });
        card.add_controller(click);
        let middle = gtk::GestureClick::new();
        middle.set_button(2);
        let weak = Rc::downgrade(self);
        middle.connect_released(move |_, _, _, _| {
            if let Some(tv) = weak.upgrade() {
                tv.do_close(i);
            }
        });
        card.add_controller(middle);
        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_enter(move |_, _, _| {
            if let Some(tv) = weak.upgrade() {
                tv.sel.set(i);
                tv.sync_sel();
            }
        });
        card.add_controller(motion);
        card
    }

    fn sync_sel(&self) {
        let sel = self.sel.get();
        for (i, c) in self.cards.borrow().iter().enumerate() {
            if i == sel { c.add_css_class("is-selected") } else { c.remove_css_class("is-selected") }
        }
    }

    fn activate(&self, i: usize) {
        let Some(w) = self.wins.borrow().get(i).cloned() else { return };
        // the keyboard back first, then the focus
        self.shell.flyouts().close();
        self.shell.gnome.send("windows.focus", json!({"id": w["id"].clone()}));
    }

    fn do_close(self: &Rc<Self>, i: usize) {
        let Some(w) = self.wins.borrow().get(i).cloned() else { return };
        if let Some(card) = self.cards.borrow().get(i) {
            card.add_css_class("is-closing");
            card.set_sensitive(false);
        }
        self.shell.gnome.send("windows.close", json!({"id": w["id"].clone()}));
        let weak = Rc::downgrade(self);
        let id = id_of(&w);
        glib::timeout_add_local_once(Duration::from_millis(180), move || {
            let Some(tv) = weak.upgrade() else { return };
            if !tv.shell.flyouts().is_open("taskview") {
                return;
            }
            tv.wins.borrow_mut().retain(|x| id_of(x) != id);
            tv.render();
            tv.refresh();
        });
    }

    /// The window list again, keeping the selected window selected.
    fn refresh(self: &Rc<Self>) {
        let sel_id = self.wins.borrow().get(self.sel.get()).map(id_of);
        let wins = self.gather();
        if let Some(id) = sel_id
            && let Some(i) = wins.iter().position(|w| id_of(w) == id)
        {
            self.sel.set(i);
        }
        let same = {
            let old = self.wins.borrow();
            old.len() == wins.len()
                && old.iter().zip(&wins).all(|(a, b)| {
                    id_of(a) == id_of(b) && a["title"] == b["title"] && a["minimized"] == b["minimized"] && a["icon"] == b["icon"]
                })
        };
        *self.wins.borrow_mut() = wins;
        if same { self.sync_sel() } else { self.render() }
    }

    fn key(self: &Rc<Self>, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        use gtk::gdk::Key;
        if key == Key::Escape {
            self.shell.flyouts().close();
            return true;
        }
        let n = self.wins.borrow().len();
        if n == 0 {
            return false;
        }
        let sel = self.sel.get().min(n - 1);
        let cols = self.cols.get().max(1);
        let next = match key {
            Key::Right => (sel + 1) % n,
            Key::Left => (sel + n - 1) % n,
            Key::Down => (sel + cols).min(n - 1),
            Key::Up => sel.saturating_sub(cols),
            Key::Tab | Key::ISO_Left_Tab => {
                if state.contains(gtk::gdk::ModifierType::SHIFT_MASK) || key == Key::ISO_Left_Tab {
                    (sel + n - 1) % n
                } else {
                    (sel + 1) % n
                }
            }
            Key::Home => 0,
            Key::End => n - 1,
            Key::Return | Key::KP_Enter | Key::space => {
                self.activate(sel);
                return true;
            }
            Key::Delete | Key::KP_Delete => {
                self.do_close(sel);
                return true;
            }
            _ => return false,
        };
        self.sel.set(next);
        self.sync_sel();
        true
    }
}
