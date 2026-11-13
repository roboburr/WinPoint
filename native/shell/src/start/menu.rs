//! Start's own menus (tile and app menus, the user and power menus, search result menus), drawn inside the Start
//! window like the HTML's in-page context menus: a layer over the menu that closes on a click outside.
use gtk::graphene;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

pub enum Item {
    Header(String),
    Separator,
    Action { label: String, glyph: &'static str, name: String, enabled: bool, tooltip: Option<String>, run: Rc<dyn Fn()> },
}

impl Item {
    pub fn action(label: String, glyph: &'static str, name: &str, run: impl Fn() + 'static) -> Item {
        Item::Action { label, glyph, name: name.to_owned(), enabled: true, tooltip: None, run: Rc::new(run) }
    }

    pub fn disabled(label: String, glyph: &'static str, name: &str, tooltip: Option<String>) -> Item {
        Item::Action { label, glyph, name: name.to_owned(), enabled: false, tooltip, run: Rc::new(|| {}) }
    }
}

#[derive(Clone)]
pub enum Anchor {
    /// At a point inside this widget (a right click).
    At(gtk::Widget, f64, f64),
    /// Above this widget, left-aligned (false) or centred (true).
    Above(gtk::Widget, bool),
    /// Below the top-left corner of this widget (keyboard menu key).
    Over(gtk::Widget),
}

pub struct MenuLayer {
    overlay: gtk::Overlay,
    layer: gtk::Fixed,
    menu: RefCell<Option<gtk::Box>>,
    anchor: RefCell<Option<Anchor>>,
}

impl MenuLayer {
    pub fn new(overlay: &gtk::Overlay) -> Rc<MenuLayer> {
        let layer = gtk::Fixed::new();
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_visible(false);
        layer.add_css_class("w11-smenu-layer");
        overlay.add_overlay(&layer);
        let me = Rc::new(MenuLayer { overlay: overlay.clone(), layer: layer.clone(), menu: RefCell::new(None), anchor: RefCell::new(None) });
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(&me);
        click.connect_pressed(move |gesture, _, x, y| {
            let Some(me) = weak.upgrade() else { return };
            let inside = me
                .menu
                .borrow()
                .as_ref()
                .and_then(|m| m.compute_bounds(&me.layer))
                .is_some_and(|b| b.contains_point(&graphene::Point::new(x as f32, y as f32)));
            if !inside {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                me.close();
            }
        });
        layer.add_controller(click);
        me
    }

    pub fn is_open(&self) -> bool {
        self.menu.borrow().is_some()
    }

    pub fn close(&self) -> bool {
        let Some(menu) = self.menu.borrow_mut().take() else { return false };
        self.layer.remove(&menu);
        self.layer.set_visible(false);
        true
    }

    /// Show a menu; returns its box so late items (software removal) can be added.
    pub fn show(self: &Rc<Self>, items: Vec<Item>, anchor: Anchor) -> gtk::Box {
        self.close();
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.add_css_class("w11-smenu");
        for item in items {
            menu.append(&self.widget(item));
        }
        self.layer.set_visible(true);
        self.layer.put(&menu, 0.0, 0.0);
        *self.menu.borrow_mut() = Some(menu.clone());
        *self.anchor.borrow_mut() = Some(anchor.clone());
        self.place(&menu, &anchor);
        // Placed again once the first frame gave the menu its real size (fonts, icons).
        let weak = Rc::downgrade(self);
        let m = menu.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(me) = weak.upgrade()
                && me.menu.borrow().as_ref() == Some(&m)
            {
                me.place(&m, &anchor);
                focus_first(&m);
            }
        });
        menu
    }

    /// Add an item to an open menu.
    pub fn append(self: &Rc<Self>, menu: &gtk::Box, item: Item) {
        if self.menu.borrow().as_ref() != Some(menu) {
            return;
        }
        menu.append(&self.widget(item));
        let weak = Rc::downgrade(self);
        let m = menu.clone();
        gtk::glib::idle_add_local_once(move || {
            if let Some(me) = weak.upgrade()
                && me.menu.borrow().as_ref() == Some(&m)
                && let Some(anchor) = me.anchor.borrow().clone()
            {
                me.place(&m, &anchor);
            }
        });
    }

    fn place(&self, menu: &gtk::Box, anchor: &Anchor) {
        let (w, h) = (self.overlay.width() as f64, self.overlay.height() as f64);
        let (_, mw, _, _) = menu.measure(gtk::Orientation::Horizontal, -1);
        let (_, mh, _, _) = menu.measure(gtk::Orientation::Vertical, mw);
        let (mw, mh) = (mw as f64, mh as f64);
        let (mut x, mut y) = match anchor {
            Anchor::At(widget, px, py) => {
                let p = widget
                    .compute_point(&self.overlay, &graphene::Point::new(*px as f32, *py as f32))
                    .map(|p| (p.x() as f64, p.y() as f64))
                    .unwrap_or((w / 2.0, h / 2.0));
                let x = if p.0 + mw > w - 4.0 { p.0 - mw } else { p.0 };
                let y = if p.1 + mh > h - 4.0 { p.1 - mh } else { p.1 };
                (x, y)
            }
            Anchor::Above(widget, centred) => {
                let b = widget.compute_bounds(&self.overlay);
                let (bx, by, bw) = b.map(|b| (b.x() as f64, b.y() as f64, b.width() as f64)).unwrap_or((0.0, h, 0.0));
                (if *centred { bx + bw / 2.0 - mw / 2.0 } else { bx }, by - mh - 4.0)
            }
            Anchor::Over(widget) => {
                let b = widget.compute_bounds(&self.overlay);
                let (bx, by, bh) = b.map(|b| (b.x() as f64, b.y() as f64, b.height() as f64)).unwrap_or((0.0, 0.0, 0.0));
                let y = if by + bh + mh > h - 4.0 { by - mh } else { by + bh };
                (bx + 8.0, y)
            }
        };
        x = x.min(w - mw - 4.0).max(4.0);
        y = y.min(h - mh - 4.0).max(4.0);
        self.layer.move_(menu, x, y);
    }

    fn widget(self: &Rc<Self>, item: Item) -> gtk::Widget {
        match item {
            Item::Header(text) => {
                let l = gtk::Label::new(Some(&text));
                l.add_css_class("w11-smenu-header");
                l.set_xalign(0.0);
                l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                l.set_max_width_chars(32);
                l.upcast()
            }
            Item::Separator => {
                let s = gtk::Separator::new(gtk::Orientation::Horizontal);
                s.add_css_class("w11-smenu-sep");
                s.upcast()
            }
            Item::Action { label, glyph, name, enabled, tooltip, run } => {
                let b = gtk::Button::new();
                b.add_css_class("w11-smenu-item");
                if !name.is_empty() {
                    b.set_widget_name(&name);
                }
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                row.append(&crate::icons::glyph(glyph, 16));
                let l = gtk::Label::new(Some(&label));
                l.set_xalign(0.0);
                l.set_hexpand(true);
                row.append(&l);
                b.set_child(Some(&row));
                b.set_sensitive(enabled);
                if let Some(t) = tooltip {
                    b.set_tooltip_text(Some(&t));
                }
                let weak = Rc::downgrade(self);
                b.connect_clicked(move |_| {
                    if let Some(me) = weak.upgrade() {
                        me.close();
                    }
                    run();
                });
                b.upcast()
            }
        }
    }
}

fn focus_first(menu: &gtk::Box) {
    let mut child = menu.first_child();
    while let Some(c) = child {
        if c.is::<gtk::Button>() && c.is_sensitive() {
            c.grab_focus();
            return;
        }
        child = c.next_sibling();
    }
}
