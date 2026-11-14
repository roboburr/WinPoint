//! Menu flyouts and context menus (contextmenu.js, Win11 MenuFlyout): Acrylic, 8 px radius, items with a
//! glyph or an app icon, label, accelerator text, description, check or radio mark (in their own column when
//! the menu mixes them with glyphs), disabled and danger items, separators, headers, submenus (hover or Right
//! opens, Left closes) and Explorer's row of icon buttons on top. At the pointer (`show_menu_at`) or under a widget
//! (`show_menu_for`); `bind_context_menu` adds right-click, the Menu key and Shift+F10 to a widget.
use crate::icons;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Item,
    Separator,
    Header,
}

/// One menu entry (build with `MenuItem::new` and the chained setters).
#[derive(Clone)]
pub struct MenuItem {
    kind: Kind,
    pub label: String,
    pub glyph: String,
    /// An app's icon (`icons::app_icon`: a theme icon name or a path) in the glyph's place.
    pub app_icon: String,
    pub accel: String,
    pub description: String,
    pub enabled: bool,
    pub checked: Option<bool>,
    pub radio: bool,
    pub danger: bool,
    pub submenu: Vec<MenuItem>,
    pub name: String,
    action: Option<Rc<dyn Fn()>>,
}

impl MenuItem {
    /// An item that runs `action` (after the menu has closed).
    pub fn new(label: &str, action: impl Fn() + 'static) -> MenuItem {
        MenuItem { action: Some(Rc::new(action)), ..MenuItem::blank(Kind::Item, label) }
    }

    fn blank(kind: Kind, label: &str) -> MenuItem {
        MenuItem {
            kind,
            label: label.to_owned(),
            glyph: String::new(),
            app_icon: String::new(),
            accel: String::new(),
            description: String::new(),
            enabled: true,
            checked: None,
            radio: false,
            danger: false,
            submenu: Vec::new(),
            name: String::new(),
            action: None,
        }
    }

    pub fn separator() -> MenuItem {
        MenuItem::blank(Kind::Separator, "")
    }

    /// A caption between items (`.fl-menu__header`).
    pub fn header(text: &str) -> MenuItem {
        MenuItem::blank(Kind::Header, text)
    }

    /// An item that opens a submenu.
    pub fn submenu(label: &str, items: Vec<MenuItem>) -> MenuItem {
        MenuItem { submenu: items, ..MenuItem::blank(Kind::Item, label) }
    }

    pub fn glyph(mut self, g: &str) -> MenuItem {
        self.glyph = g.to_owned();
        self
    }

    /// An app's icon at 16 px in the glyph column (Explorer's "Openen met"; contextmenu.js `icon: {url, size}`),
    /// from the icon theme or the backend's `/icon` (`icons::app_icon`). An empty name: the generic app icon.
    pub fn app_icon(mut self, icon: &str) -> MenuItem {
        self.app_icon = if icon.trim().is_empty() { "application-x-executable".to_owned() } else { icon.to_owned() };
        self
    }

    fn has_icon(&self) -> bool {
        !self.glyph.is_empty() || !self.app_icon.is_empty()
    }

    /// The shortcut text on the right ("Ctrl+C").
    pub fn accel(mut self, a: &str) -> MenuItem {
        self.accel = a.to_owned();
        self
    }

    pub fn description(mut self, d: &str) -> MenuItem {
        self.description = d.to_owned();
        self
    }

    pub fn enabled(mut self, e: bool) -> MenuItem {
        self.enabled = e;
        self
    }

    /// A check mark (Some(true) checked, Some(false) room for one).
    pub fn checked(mut self, c: bool) -> MenuItem {
        self.checked = Some(c);
        self
    }

    /// A radio item (a dot when `checked`).
    pub fn radio(mut self, checked: bool) -> MenuItem {
        self.radio = true;
        self.checked = Some(checked);
        self
    }

    pub fn danger(mut self) -> MenuItem {
        self.danger = true;
        self
    }

    /// The item button's widget name (dev channel).
    pub fn name(mut self, n: &str) -> MenuItem {
        self.name = n.to_owned();
        self
    }
}

/// One icon button of the row on top (Explorer: cut, copy, rename, share, delete).
#[derive(Clone)]
pub struct TopAction {
    pub glyph: String,
    pub label: String,
    pub enabled: bool,
    pub danger: bool,
    action: Rc<dyn Fn()>,
}

impl TopAction {
    pub fn new(glyph: &str, label: &str, action: impl Fn() + 'static) -> TopAction {
        TopAction { glyph: glyph.to_owned(), label: label.to_owned(), enabled: true, danger: false, action: Rc::new(action) }
    }
    pub fn enabled(mut self, e: bool) -> TopAction {
        self.enabled = e;
        self
    }
    pub fn danger(mut self) -> TopAction {
        self.danger = true;
        self
    }
}

thread_local! {
    static OPEN: RefCell<Vec<glib::WeakRef<gtk::Popover>>> = const { RefCell::new(Vec::new()) };
}

/// Close every open menu.
pub fn close_all() {
    let open: Vec<gtk::Popover> = OPEN.with(|o| o.borrow_mut().drain(..).filter_map(|w| w.upgrade()).collect());
    for p in open.into_iter().rev() {
        p.popdown();
    }
}

/// Show a menu with its top-left corner at (x, y) in `widget`'s coordinates.
pub fn show_menu_at(widget: &impl IsA<gtk::Widget>, x: f64, y: f64, items: Vec<MenuItem>) -> gtk::Popover {
    show_menu_with_top(widget, x, y, Vec::new(), items)
}

/// `show_menu_at` with Explorer's icon row on top.
pub fn show_menu_with_top(widget: &impl IsA<gtk::Widget>, x: f64, y: f64, top: Vec<TopAction>, items: Vec<MenuItem>) -> gtk::Popover {
    close_all();
    let p = build(widget.as_ref(), &top, &items, None);
    p.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
    p.set_position(gtk::PositionType::Bottom);
    p.set_halign(gtk::Align::Start);
    p.popup();
    focus_first(&p);
    p
}

/// Show a menu under `anchor`, start-aligned with it (DropDownButton, a "..." button).
pub fn show_menu_for(anchor: &impl IsA<gtk::Widget>, items: Vec<MenuItem>) -> gtk::Popover {
    close_all();
    let a = anchor.as_ref();
    let p = build(a, &[], &items, None);
    p.set_position(gtk::PositionType::Bottom);
    p.set_halign(gtk::Align::Start);
    p.set_offset(0, 4);
    p.popup();
    focus_first(&p);
    p
}

/// Right-click, the Menu key and Shift+F10 on `widget` open the menu `f(x, y)` builds (None: no menu). For
/// the keys, (x, y) is the widget's top-left corner plus 8 px.
pub fn bind_context_menu(widget: &impl IsA<gtk::Widget>, f: impl Fn(f64, f64) -> Option<(Vec<TopAction>, Vec<MenuItem>)> + 'static) {
    let f = Rc::new(f);
    let w = widget.as_ref();
    let click = gtk::GestureClick::new();
    click.set_button(gdk::BUTTON_SECONDARY);
    let f1 = f.clone();
    let wk = w.downgrade();
    click.connect_pressed(move |g, _, x, y| {
        let Some(w) = wk.upgrade() else { return };
        if let Some((top, items)) = f1(x, y) {
            g.set_state(gtk::EventSequenceState::Claimed);
            show_menu_with_top(&w, x, y, top, items);
        }
    });
    w.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    let wk = w.downgrade();
    keys.connect_key_pressed(move |_, key, _, state| {
        let menu_key = key == gdk::Key::Menu || (key == gdk::Key::F10 && state.contains(gdk::ModifierType::SHIFT_MASK));
        if !menu_key {
            return glib::Propagation::Proceed;
        }
        let Some(w) = wk.upgrade() else { return glib::Propagation::Proceed };
        if let Some((top, items)) = f(8.0, 8.0) {
            show_menu_with_top(&w, 8.0, 8.0, top, items);
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    w.add_controller(keys);
}

fn focus_first(p: &gtk::Popover) {
    let p = p.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(p) = p.upgrade()
            && let Some(first) = items_of(&p).into_iter().find(|b| b.is_sensitive())
        {
            first.grab_focus();
        }
    });
}

fn items_of(p: &gtk::Popover) -> Vec<gtk::Button> {
    let mut out = Vec::new();
    fn walk(w: &gtk::Widget, out: &mut Vec<gtk::Button>) {
        if let Some(b) = w.downcast_ref::<gtk::Button>()
            && (b.has_css_class("w11-menu-item") || b.parent().is_some_and(|p| p.has_css_class("w11-menu-top")))
        {
            out.push(b.clone());
            return;
        }
        let mut c = w.first_child();
        while let Some(child) = c {
            if !child.is::<gtk::Popover>() {
                walk(&child, out);
            }
            c = child.next_sibling();
        }
    }
    if let Some(child) = p.child() {
        walk(&child, &mut out);
    }
    out
}

fn build(parent: &gtk::Widget, top: &[TopAction], items: &[MenuItem], owner: Option<&gtk::Popover>) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("w11-menu");
    popover.set_has_arrow(false);
    popover.set_autohide(true);
    let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu.add_css_class("w11-menu-box");
    let list: Vec<&MenuItem> = items.iter().collect();
    let has_state = list.iter().any(|it| it.kind == Kind::Item && (it.radio || it.checked.is_some()));
    let state_column = has_state && list.iter().any(|it| it.kind == Kind::Item && it.has_icon());
    let has_icons = list.iter().any(|it| it.has_icon() || it.checked.is_some() || it.radio);
    if state_column {
        menu.add_css_class("w11-has-state");
    }
    let root_popover = owner.cloned().unwrap_or_else(|| popover.clone());
    if !top.is_empty() {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        row.add_css_class("w11-menu-top");
        for a in top {
            let b = gtk::Button::new();
            b.set_child(Some(&icons::glyph(&a.glyph, 16)));
            b.set_tooltip_text(Some(&a.label));
            b.set_sensitive(a.enabled);
            if a.danger {
                b.add_css_class("w11-danger");
            }
            let action = a.action.clone();
            let rp = root_popover.downgrade();
            b.connect_clicked(move |_| {
                if let Some(p) = rp.upgrade() {
                    p.popdown();
                }
                let action = action.clone();
                glib::idle_add_local_once(move || action());
            });
            row.append(&b);
        }
        menu.append(&row);
        if !list.is_empty() {
            let sep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            sep.add_css_class("w11-menu-sep");
            menu.append(&sep);
        }
    }
    let open_sub: Rc<RefCell<Option<gtk::Popover>>> = Rc::new(RefCell::new(None));
    for it in list {
        match it.kind {
            Kind::Separator => {
                let sep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                sep.add_css_class("w11-menu-sep");
                menu.append(&sep);
            }
            Kind::Header => {
                menu.append(&super::label(&it.label, "w11-menu-header", false));
            }
            Kind::Item => {
                let b = item_button(it, state_column, has_icons);
                if !it.name.is_empty() {
                    b.set_widget_name(&it.name);
                }
                if it.submenu.is_empty() {
                    let action = it.action.clone();
                    let rp = root_popover.downgrade();
                    b.connect_clicked(move |_| {
                        if let Some(p) = rp.upgrade() {
                            p.popdown();
                        }
                        close_all();
                        if let Some(a) = action.clone() {
                            glib::idle_add_local_once(move || a());
                        }
                    });
                } else {
                    let sub_items = it.submenu.clone();
                    let os = open_sub.clone();
                    let rp = root_popover.clone();
                    let opener = Rc::new(move |b: &gtk::Button| {
                        if let Some(old) = os.borrow_mut().take() {
                            old.popdown();
                        }
                        let sub = build(b.upcast_ref(), &[], &sub_items, Some(&rp));
                        sub.set_position(gtk::PositionType::Right);
                        sub.set_halign(gtk::Align::Start);
                        sub.set_valign(gtk::Align::Start);
                        b.add_css_class("w11-active");
                        let bw = b.downgrade();
                        sub.connect_closed(move |_| {
                            if let Some(b) = bw.upgrade() {
                                b.remove_css_class("w11-active");
                            }
                        });
                        sub.popup();
                        *os.borrow_mut() = Some(sub);
                    });
                    let o = opener.clone();
                    b.connect_clicked(move |b| o(b));
                    let motion = gtk::EventControllerMotion::new();
                    let o = opener.clone();
                    let bw = b.downgrade();
                    let pending: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
                    let pend = pending.clone();
                    motion.connect_enter(move |_, _, _| {
                        let o = o.clone();
                        let bw = bw.clone();
                        let pend2 = pend.clone();
                        let id = glib::timeout_add_local_once(Duration::from_millis(250), move || {
                            pend2.borrow_mut().take();
                            if let Some(b) = bw.upgrade() {
                                o(&b);
                            }
                        });
                        if let Some(old) = pend.borrow_mut().replace(id) {
                            old.remove();
                        }
                    });
                    motion.connect_leave(move |_| {
                        if let Some(id) = pending.borrow_mut().take() {
                            id.remove();
                        }
                    });
                    b.add_controller(motion);
                    let keys = gtk::EventControllerKey::new();
                    let o = opener.clone();
                    let bw = b.downgrade();
                    keys.connect_key_pressed(move |_, key, _, _| {
                        if key == gdk::Key::Right
                            && let Some(b) = bw.upgrade()
                        {
                            o(&b);
                            return glib::Propagation::Stop;
                        }
                        glib::Propagation::Proceed
                    });
                    b.add_controller(keys);
                }
                menu.append(&b);
            }
        }
    }
    let sc = gtk::ScrolledWindow::new();
    sc.set_hscrollbar_policy(gtk::PolicyType::Never);
    sc.set_propagate_natural_height(true);
    sc.set_propagate_natural_width(true);
    sc.set_max_content_height(640);
    sc.set_child(Some(&menu));
    popover.set_child(Some(&sc));
    popover.set_parent(parent);
    // Up / Down / Home / End move through the items; Left closes a submenu.
    let keys = gtk::EventControllerKey::new();
    let pw = popover.downgrade();
    let is_sub = owner.is_some();
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(p) = pw.upgrade() else { return glib::Propagation::Proceed };
        let items: Vec<gtk::Button> = items_of(&p).into_iter().filter(|b| b.is_sensitive()).collect();
        if items.is_empty() {
            return glib::Propagation::Proceed;
        }
        let cur = items.iter().position(|b| b.has_focus());
        let next = match (key, cur) {
            (gdk::Key::Down, Some(i)) => Some((i + 1) % items.len()),
            (gdk::Key::Down, None) => Some(0),
            (gdk::Key::Up, Some(i)) => Some((i + items.len() - 1) % items.len()),
            (gdk::Key::Up, None) => Some(items.len() - 1),
            (gdk::Key::Home, _) => Some(0),
            (gdk::Key::End, _) => Some(items.len() - 1),
            (gdk::Key::Left, _) if is_sub => {
                p.popdown();
                return glib::Propagation::Stop;
            }
            _ => None,
        };
        match next {
            Some(i) => {
                items[i].grab_focus();
                glib::Propagation::Stop
            }
            None => glib::Propagation::Proceed,
        }
    });
    popover.add_controller(keys);
    popover.connect_closed(move |p| {
        let p = p.clone();
        glib::idle_add_local_once(move || {
            if p.parent().is_some() {
                p.unparent();
            }
        });
    });
    OPEN.with(|o| {
        let mut o = o.borrow_mut();
        o.retain(|w| w.upgrade().is_some());
        o.push(popover.downgrade());
    });
    popover
}

fn item_button(it: &MenuItem, state_column: bool, has_icons: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-menu-item");
    if it.danger {
        b.add_css_class("w11-danger");
    }
    b.set_sensitive(it.enabled);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let lead = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    if state_column {
        let st = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        st.add_css_class("w11-menu-state");
        st.set_size_request(12, 16);
        st.set_valign(gtk::Align::Center);
        st.set_hexpand(false);
        if it.radio && it.checked == Some(true) {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("w11-menu-radio");
            dot.set_halign(gtk::Align::Center);
            dot.set_valign(gtk::Align::Center);
            dot.set_hexpand(true);
            st.append(&dot);
        } else if it.checked == Some(true) {
            st.append(&icons::glyph("check", 12));
        }
        lead.append(&st);
    }
    if has_icons {
        let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ic.add_css_class("w11-menu-icon");
        ic.set_size_request(16, 16);
        ic.set_valign(gtk::Align::Center);
        ic.set_hexpand(false);
        if !state_column && it.radio && it.checked == Some(true) {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("w11-menu-radio");
            dot.set_halign(gtk::Align::Center);
            dot.set_valign(gtk::Align::Center);
            dot.set_hexpand(true);
            ic.append(&dot);
        } else if !state_column && it.checked == Some(true) {
            ic.append(&icons::glyph("check", 16));
        } else if !it.app_icon.is_empty() {
            let a = icons::app_icon(&it.app_icon, 16);
            a.add_css_class("w11-menu-appicon");
            ic.append(&a);
        } else if !it.glyph.is_empty() {
            let g = icons::glyph(&it.glyph, 16);
            ic.append(&g);
        }
        lead.append(&ic);
    }
    if lead.first_child().is_some() {
        row.append(&lead);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&super::label(&it.label, "w11-menu-label", false));
    if !it.description.is_empty() {
        let d = super::label(&it.description, "w11-menu-desc", true);
        d.set_max_width_chars(40);
        text.append(&d);
    }
    row.append(&text);
    if !it.accel.is_empty() {
        let a = gtk::Label::new(Some(&it.accel));
        a.add_css_class("w11-menu-shortcut");
        a.set_valign(gtk::Align::Center);
        row.append(&a);
    }
    if !it.submenu.is_empty() {
        let s = icons::glyph("chevron-right", 12);
        s.add_css_class("w11-menu-sub");
        row.append(&s);
    }
    b.set_child(Some(&row));
    b
}
