//! A Win11 MenuFlyout (core/contextmenu.js, the .fl-menu look) and the reusable "jumplist" flyout that shows one:
//! jump lists, the taskbar's context menu and the tray icons' dbusmenus. A submenu opens in place (with a row
//! back to the parent), as there is one flyout at a time.
use super::util;
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Clone)]
pub enum Icon {
    None,
    Glyph(&'static str),
    App(String),
}

#[derive(Clone)]
pub struct Item {
    pub label: String,
    pub icon: Icon,
    pub name: String,
    pub disabled: bool,
    /// Some(checked) for a checkmark or radio item.
    pub check: Option<bool>,
    pub radio: bool,
    pub shortcut: Option<String>,
    pub submenu: Option<Vec<Item>>,
    pub separator: bool,
    pub action: Option<Rc<dyn Fn()>>,
}

impl Item {
    pub fn new(label: impl Into<String>, icon: Icon, action: impl Fn() + 'static) -> Item {
        Item {
            label: label.into(),
            icon,
            name: String::new(),
            disabled: false,
            check: None,
            radio: false,
            shortcut: None,
            submenu: None,
            separator: false,
            action: Some(Rc::new(action)),
        }
    }

    pub fn plain(label: impl Into<String>) -> Item {
        Item {
            label: label.into(),
            icon: Icon::None,
            name: String::new(),
            disabled: false,
            check: None,
            radio: false,
            shortcut: None,
            submenu: None,
            separator: false,
            action: None,
        }
    }

    pub fn sep() -> Item {
        let mut item = Item::plain("");
        item.separator = true;
        item
    }

    pub fn named(mut self, name: impl Into<String>) -> Item {
        self.name = name.into();
        self
    }
}

/// One menu row: [icon] [state] label [shortcut] [>]. `markup` replaces the label text (access keys).
pub fn row(shell: &Shell, item: &Item, has_icons: bool, has_state: bool, markup: Option<String>) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("w11-mn-item");
    button.set_focus_on_click(false);
    if !item.name.is_empty() {
        button.set_widget_name(&item.name);
    }
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    if has_state {
        let state = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        state.add_css_class("w11-mn-state");
        state.set_size_request(12, 16);
        state.set_valign(gtk::Align::Center);
        match item.check {
            Some(true) if item.radio => {
                let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                dot.add_css_class("w11-mn-radio");
                dot.set_size_request(6, 6);
                dot.set_halign(gtk::Align::Center);
                dot.set_valign(gtk::Align::Center);
                dot.set_hexpand(true);
                state.append(&dot);
            }
            Some(true) => state.append(&crate::icons::glyph("check", 12)),
            _ => {}
        }
        content.append(&state);
    }
    if has_icons {
        let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        slot.add_css_class("w11-mn-icon");
        slot.set_size_request(16, 16);
        slot.set_valign(gtk::Align::Center);
        match &item.icon {
            Icon::Glyph(name) => slot.append(&crate::icons::glyph(name, 16)),
            Icon::App(src) => slot.append(&util::icon(shell, src, 16)),
            Icon::None => {}
        }
        content.append(&slot);
    }
    let label = gtk::Label::new(None);
    match markup {
        Some(m) => label.set_markup(&m),
        None => label.set_text(&item.label),
    }
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(48);
    label.add_css_class("w11-mn-label");
    content.append(&label);
    if let Some(shortcut) = &item.shortcut {
        let s = gtk::Label::new(Some(shortcut));
        s.add_css_class("w11-mn-shortcut");
        s.set_margin_start(12);
        content.append(&s);
    }
    if item.submenu.is_some() {
        let chevron = crate::icons::glyph("chevron-right", 12);
        chevron.add_css_class("w11-mn-sub");
        content.append(&chevron);
    }
    button.set_child(Some(&content));
    button.set_sensitive(!item.disabled);
    button
}

pub fn separator() -> gtk::Widget {
    let sep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    sep.add_css_class("w11-mn-sep");
    sep.set_size_request(-1, 1);
    sep.upcast()
}

/// Arrow keys over the menu's rows (Up/Down wrap, Home/End), as contextmenu.js.
pub fn move_focus(rows: &[gtk::Button], key: gtk::gdk::Key) -> bool {
    use gtk::gdk::Key;
    let live: Vec<&gtk::Button> = rows.iter().filter(|b| b.is_sensitive() && b.is_visible()).collect();
    if live.is_empty() {
        return false;
    }
    let at = live.iter().position(|b| b.has_focus());
    let n = live.len();
    let next = match key {
        Key::Down => at.map(|i| (i + 1) % n).unwrap_or(0),
        Key::Up => at.map(|i| (i + n - 1) % n).unwrap_or(n - 1),
        Key::Home => 0,
        Key::End => n - 1,
        _ => return false,
    };
    live[next].grab_focus();
    true
}

/// The "jumplist" flyout: whatever menu was handed to `show`.
pub struct MenuFlyout {
    root: gtk::Box,
    shell: Rc<Shell>,
    rows: RefCell<Vec<(gtk::Button, Option<Item>)>>,
    /// Parent menus while a submenu is shown: (title, items).
    stack: RefCell<Vec<(String, Vec<Item>)>>,
    current: RefCell<Vec<Item>>,
    on_close: RefCell<Option<Box<dyn FnOnce()>>>,
    /// The anchor of the open menu: (x, centred over it).
    anchor: Cell<(i32, bool)>,
}

thread_local! {
    static MENU: RefCell<Option<Rc<MenuFlyout>>> = const { RefCell::new(None) };
}

struct Handle(Rc<MenuFlyout>);

impl Flyout for Handle {
    fn name(&self) -> &'static str {
        "jumplist"
    }
    fn widget(&self) -> gtk::Widget {
        self.0.root.clone().upcast()
    }
    fn placement(&self) -> Placement {
        Placement::BottomAt(12)
    }
    fn opened(&self) {
        let me = self.0.clone();
        glib::idle_add_local_once(move || {
            me.root.grab_focus();
        });
    }
    fn closed(&self) {
        self.0.stack.borrow_mut().clear();
        let f = self.0.on_close.borrow_mut().take();
        if let Some(f) = f {
            f();
        }
    }
}

pub fn register(shell: &Rc<Shell>) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("w11-mn");
    root.set_widget_name("menus-jumplist");
    root.set_focusable(true);
    let menu = Rc::new(MenuFlyout {
        root: root.clone(),
        shell: shell.clone(),
        rows: RefCell::new(Vec::new()),
        stack: RefCell::new(Vec::new()),
        current: RefCell::new(Vec::new()),
        on_close: RefCell::new(None),
        anchor: Cell::new((12, false)),
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&menu);
    keys.connect_key_pressed(move |_, key, _, _| {
        use gtk::gdk::Key;
        let Some(m) = weak.upgrade() else { return glib::Propagation::Proceed };
        if matches!(key, Key::Up | Key::Down | Key::Home | Key::End) {
            let buttons: Vec<gtk::Button> = m.rows.borrow().iter().map(|(b, _)| b.clone()).collect();
            if move_focus(&buttons, key) {
                return glib::Propagation::Stop;
            }
        } else if matches!(key, Key::Left | Key::BackSpace) || (key == Key::Escape && !m.stack.borrow().is_empty()) {
            if m.back() {
                return glib::Propagation::Stop;
            }
        } else if key == Key::Right {
            let item = m.rows.borrow().iter().find(|(b, _)| b.has_focus()).and_then(|(_, i)| i.clone());
            if let Some(item) = item.filter(|x| x.submenu.is_some()) {
                m.enter(&item);
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    root.add_controller(keys);
    shell.flyouts().register(Rc::new(Handle(menu.clone())));
    MENU.with(|m| *m.borrow_mut() = Some(menu));
}

impl MenuFlyout {
    fn fill(self: &Rc<Self>, items: Vec<Item>) {
        util::clear(&self.root);
        let mut rows: Vec<(gtk::Button, Option<Item>)> = Vec::new();
        // a submenu: a row back to its parent first
        if !self.stack.borrow().is_empty() {
            let title = self.current_title();
            let back = gtk::Button::new();
            back.add_css_class("w11-mn-item");
            back.add_css_class("w11-mn-back");
            back.set_widget_name("menus-jumplist-back");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            content.append(&crate::icons::glyph("chevron-left", 12));
            let label = gtk::Label::new(Some(&title));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.add_css_class("w11-mn-label");
            content.append(&label);
            back.set_child(Some(&content));
            let weak = Rc::downgrade(self);
            back.connect_clicked(move |_| {
                if let Some(m) = weak.upgrade() {
                    m.back();
                }
            });
            self.root.append(&back);
            self.root.append(&separator());
            rows.push((back, None));
        }
        let has_icons = items.iter().any(|x| !matches!(x.icon, Icon::None));
        let has_state = items.iter().any(|x| x.check.is_some());
        if has_state {
            self.root.add_css_class("has-state");
        } else {
            self.root.remove_css_class("has-state");
        }
        let mut last_sep = true;
        let count = items.len();
        for (i, item) in items.iter().enumerate() {
            if item.separator {
                // no leading, trailing or doubled lines
                if !last_sep && i + 1 < count {
                    self.root.append(&separator());
                    last_sep = true;
                }
                continue;
            }
            last_sep = false;
            let button = row(&self.shell, item, has_icons, has_state, None);
            let weak = Rc::downgrade(self);
            let it = item.clone();
            button.connect_clicked(move |_| {
                if let Some(m) = weak.upgrade() {
                    m.invoke(&it);
                }
            });
            self.root.append(&button);
            rows.push((button, Some(item.clone())));
        }
        if rows.is_empty() {
            let empty = gtk::Label::new(Some(&self.shell.t("(Leeg)", "(Empty)")));
            empty.add_css_class("w11-mn-empty");
            empty.set_xalign(0.0);
            self.root.append(&empty);
        }
        *self.rows.borrow_mut() = rows;
        *self.current.borrow_mut() = items;
    }

    fn current_title(&self) -> String {
        self.stack.borrow().last().map(|(t, _)| t.clone()).unwrap_or_default()
    }

    fn invoke(self: &Rc<Self>, item: &Item) {
        if item.submenu.is_some() {
            self.enter(item);
            return;
        }
        let action = item.action.clone();
        self.shell.flyouts().close();
        if let Some(action) = action {
            action();
        }
    }

    fn enter(self: &Rc<Self>, item: &Item) {
        let Some(sub) = item.submenu.clone() else { return };
        let parent = self.current.borrow().clone();
        self.stack.borrow_mut().push((item.label.clone(), parent));
        self.fill(sub);
        self.replace();
        self.focus_first();
    }

    fn back(self: &Rc<Self>) -> bool {
        let Some((_, parent)) = self.stack.borrow_mut().pop() else { return false };
        self.fill(parent);
        self.replace();
        self.focus_first();
        true
    }

    fn focus_first(&self) {
        let rows: Vec<gtk::Button> = self.rows.borrow().iter().map(|(b, _)| b.clone()).collect();
        glib::idle_add_local_once(move || {
            if let Some(b) = rows.iter().find(|b| b.is_sensitive()) {
                b.grab_focus();
            }
        });
    }

    /// Keep the menu on screen at its anchor (after the content changed).
    fn replace(&self) {
        let (x, center) = self.anchor.get();
        let (_, natural, _, _) = self.root.measure(gtk::Orientation::Horizontal, -1);
        let width = natural.max(1);
        let monitor = util::monitor_width(&self.root);
        let left = if center { x - width / 2 } else { x };
        let left = left.min(monitor - width - 12).max(12);
        self.shell.flyouts().set_placement_x("jumplist", left);
    }
}

/// Show `items` in the jumplist flyout: centred over `x` (a taskbar button) or starting at `x` (a pointer).
/// `on_close` runs once when this menu goes away (closed, or replaced by another menu).
pub fn show(shell: &Rc<Shell>, items: Vec<Item>, x: i32, center: bool, on_close: Option<Box<dyn FnOnce()>>) {
    let Some(menu) = MENU.with(|m| m.borrow().clone()) else { return };
    let previous = menu.on_close.borrow_mut().take();
    if let Some(f) = previous {
        f();
    }
    menu.stack.borrow_mut().clear();
    menu.fill(items);
    menu.anchor.set((x, center));
    menu.replace();
    *menu.on_close.borrow_mut() = on_close;
    shell.flyouts().open("jumplist");
}

/// `items` as the same menu in a popover of `parent` (a taskbar of another monitor, panel/strips.rs, which has no
/// flyout host): above it, centred over `x` or starting at it (`parent`'s coordinates). A row closes the menu and runs
/// its action; Up, Down, Home and End move between the rows, Escape or a click elsewhere closes it. Submenus are not
/// needed there (jump lists and the taskbar menu have none).
pub fn popup(shell: &Rc<Shell>, parent: &gtk::Widget, items: Vec<Item>, x: i32, center: bool) -> gtk::Popover {
    let popover = gtk::Popover::new();
    popover.add_css_class("w11-strip-menu");
    popover.set_autohide(true);
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Top);
    let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
    frame.add_css_class("w11-flyout");
    frame.add_css_class("w11-flyout-jumplist");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("w11-mn");
    root.set_widget_name("menus-strip-menu");
    root.set_focusable(true);
    frame.append(&root);
    let has_icons = items.iter().any(|x| !matches!(x.icon, Icon::None));
    let has_state = items.iter().any(|x| x.check.is_some());
    if has_state {
        root.add_css_class("has-state");
    }
    let mut rows: Vec<gtk::Button> = Vec::new();
    let mut last_sep = true;
    let count = items.len();
    for (i, item) in items.iter().enumerate() {
        if item.separator {
            if !last_sep && i + 1 < count {
                root.append(&separator());
                last_sep = true;
            }
            continue;
        }
        last_sep = false;
        let button = row(shell, item, has_icons, has_state, None);
        let (weak, action) = (popover.downgrade(), item.action.clone());
        button.connect_clicked(move |_| {
            if let Some(p) = weak.upgrade() {
                p.popdown();
            }
            if let Some(action) = &action {
                action();
            }
        });
        root.append(&button);
        rows.push(button);
    }
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let focusable = rows.clone();
    keys.connect_key_pressed(move |_, key, _, _| {
        use gtk::gdk::Key;
        if matches!(key, Key::Up | Key::Down | Key::Home | Key::End) && move_focus(&focusable, key) {
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    root.add_controller(keys);
    popover.set_child(Some(&frame));
    popover.set_parent(parent);
    // centred over x by GTK; a menu that starts at x points at its middle
    let (_, natural, _, _) = frame.measure(gtk::Orientation::Horizontal, -1);
    let at = if center { x } else { x + natural / 2 };
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(at, 0, 1, 1)));
    popover.connect_closed(|p| {
        let p = p.clone();
        glib::idle_add_local_once(move || p.unparent());
    });
    popover.popup();
    let first = rows.into_iter().find(|b| b.is_sensitive());
    glib::idle_add_local_once(move || {
        if let Some(b) = first {
            b.grab_focus();
        }
    });
    popover
}

/// Replace the items of the open menu (a tray app rebuilt its menu), keeping it where it is.
pub fn update(items: Vec<Item>) {
    let Some(menu) = MENU.with(|m| m.borrow().clone()) else { return };
    if !crate::core::shell().flyouts().is_open("jumplist") {
        return;
    }
    menu.stack.borrow_mut().clear();
    menu.fill(items);
    menu.replace();
}
