//! The tab strip in the title bar (last-webkit:vendor/ui/js/apps/explorer/tabs.js): a click activates, the middle
//! button or the cross closes, "+" opens Start, Left / Right / Enter / Delete on a focused tab; the tabs share the room
//! (240 px each at most, 52 at least). Dragging files over a tab (dnd.rs) and tabs between windows
//! (transfer.rs) attach to each tab widget.
use crate::actbox::ActBox;
use crate::explorer::{Explorer, WeakExplorer};
use crate::i18n::s;
use crate::{dnd, transfer};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// An inverted 8 px corner in the toolbar's colour (CSS `color` of `.xp-tab-flare`).
fn flare(left: bool) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.add_css_class("xp-tab-flare");
    a.set_content_width(8);
    a.set_content_height(8);
    a.set_valign(gtk::Align::End);
    a.set_can_target(false);
    a.set_draw_func(move |area, cr, w, h| {
        let c = area.color();
        let (w, h) = (w as f64, h as f64);
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        cr.rectangle(0.0, 0.0, w, h);
        cr.new_sub_path();
        let cx = if left { 0.0 } else { w };
        cr.arc(cx, 0.0, 8.0, 0.0, std::f64::consts::TAU);
        cr.set_fill_rule(gtk::cairo::FillRule::EvenOdd);
        let _ = cr.fill();
    });
    kit::theme::track(&a);
    a
}

struct TabWidgets {
    root: gtk::Box,
    icon: gtk::Box,
    label: gtk::Label,
    icon_key: RefCell<String>,
}

struct Inner {
    x: WeakExplorer,
    root: gtk::Overlay,
    list: gtk::Box,
    add: gtk::Button,
    tabs: RefCell<HashMap<u64, Rc<TabWidgets>>>,
    width: Cell<i32>,
    attached: Cell<bool>,
}

/// The tab strip; a cheap handle.
#[derive(Clone)]
pub struct TabStrip {
    inner: Rc<Inner>,
}

impl TabStrip {
    pub fn new(x: WeakExplorer) -> TabStrip {
        let root = gtk::Overlay::new();
        root.add_css_class("xp-tabs");
        root.set_hexpand(true);
        root.set_valign(gtk::Align::Fill);
        root.set_widget_name("explorer-tabs");
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_valign(gtk::Align::End);
        let list = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        list.add_css_class("xp-tabs-list");
        list.set_valign(gtk::Align::End);
        let add = gtk::Button::new();
        add.add_css_class("xp-tabs-add");
        add.set_child(Some(&kit::icons::glyph("plus", 14)));
        add.set_valign(gtk::Align::End);
        add.set_focus_on_click(false);
        add.set_widget_name("explorer-new-tab");
        row.append(&list);
        row.append(&add);
        root.set_child(Some(&row));
        let probe = gtk::DrawingArea::new();
        probe.set_can_target(false);
        root.add_overlay(&probe);
        let inner = Rc::new(Inner {
            x,
            root,
            list,
            add,
            tabs: RefCell::new(HashMap::new()),
            width: Cell::new(0),
            attached: Cell::new(false),
        });
        let me = TabStrip { inner };
        let w = me.inner.x.clone();
        me.inner.add.connect_clicked(move |_| {
            if let Some(x) = w.upgrade() {
                x.open_tab(crate::model::Loc::Home, true);
            }
        });
        let wk = Rc::downgrade(&me.inner);
        probe.connect_resize(move |_, width, _| {
            if let Some(i) = wk.upgrade() {
                i.width.set(width);
                let me = TabStrip { inner: i };
                glib::idle_add_local_once(move || me.size_tabs());
            }
        });
        me
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.inner.root
    }

    fn x(&self) -> Option<Explorer> {
        self.inner.x.upgrade()
    }

    fn make_tab(&self, x: &Explorer, id: u64) -> Rc<TabWidgets> {
        // [flare] [tab body] [flare]: the active tab flows into the toolbar with inverted corners
        let root = ActBox::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("xp-tabwrap");
        root.set_focusable(true);
        root.set_widget_name(&format!("tab-{id}"));
        root.set_valign(gtk::Align::End);
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        body.add_css_class("xp-tab");
        body.set_hexpand(true);
        let sep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        sep.add_css_class("xp-tab-sep");
        sep.set_size_request(1, 16);
        sep.set_halign(gtk::Align::Start);
        sep.set_valign(gtk::Align::Center);
        sep.set_can_target(false);
        let ov = gtk::Overlay::new();
        ov.set_child(Some(&body));
        ov.add_overlay(&sep);
        ov.set_hexpand(true);
        root.append(&flare(true));
        root.append(&ov);
        root.append(&flare(false));
        // the CSS pulls the flares over the neighbours (margin 0 -8px): keep the size positive
        root.set_size_request(68, 32);
        // the label expands inside the tab; the tab keeps the width size_tabs gives it
        root.set_hexpand(false);
        let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon.add_css_class("xp-tab-icon");
        icon.set_valign(gtk::Align::Center);
        let label = gtk::Label::new(None);
        label.add_css_class("xp-tab-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_width_chars(1);
        let close = gtk::Button::new();
        close.add_css_class("xp-tab-close");
        close.set_child(Some(&kit::icons::glyph("close", 10)));
        close.set_can_focus(false);
        close.set_focus_on_click(false);
        close.set_valign(gtk::Align::Center);
        close.set_tooltip_text(Some(s("closeTab")));
        close.set_widget_name(&format!("tab-{id}-close"));
        body.append(&icon);
        body.append(&label);
        body.append(&close);
        let w = self.inner.x.clone();
        close.connect_clicked(move |_| {
            if let Some(x) = w.upgrade() {
                x.close_tab(id);
            }
        });
        // press activates; middle closes; a double click must not maximize the window
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let w = self.inner.x.clone();
        click.connect_pressed(move |g, n, _, _| {
            let Some(x) = w.upgrade() else { return };
            match g.current_button() {
                gdk::BUTTON_PRIMARY => {
                    if n >= 2 {
                        g.set_state(gtk::EventSequenceState::Claimed);
                    }
                    x.activate_tab(id);
                }
                gdk::BUTTON_MIDDLE => {
                    g.set_state(gtk::EventSequenceState::Claimed);
                    x.close_tab(id);
                }
                _ => {}
            }
        });
        root.add_controller(click);
        let w = self.inner.x.clone();
        root.connect_activated(move |_| {
            if let Some(x) = w.upgrade() {
                x.activate_tab(id);
            }
        });
        let keys = gtk::EventControllerKey::new();
        let wk = Rc::downgrade(&self.inner);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(me) = wk.upgrade().map(|inner| TabStrip { inner }) else { return glib::Propagation::Proceed };
            let Some(x) = me.x() else { return glib::Propagation::Proceed };
            let ids = x.tab_ids();
            let i = ids.iter().position(|t| *t == id).unwrap_or(0);
            match key {
                gdk::Key::Right | gdk::Key::Left => {
                    let n = ids.len();
                    let j = if key == gdk::Key::Right { (i + 1) % n } else { (i + n - 1) % n };
                    x.activate_tab(ids[j]);
                    if let Some(t) = me.inner.tabs.borrow().get(&ids[j]) {
                        t.root.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                    x.activate_tab(id);
                    x.focus_view();
                    glib::Propagation::Stop
                }
                gdk::Key::Delete => {
                    x.close_tab(id);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        root.add_controller(keys);
        dnd::attach_tab(x, root.upcast_ref(), id);
        transfer::attach_tab(x, root.upcast_ref(), id);
        Rc::new(TabWidgets { root: root.upcast(), icon, label, icon_key: RefCell::new(String::new()) })
    }

    /// Bring the strip in line with the tabs (order, labels, icons, the active one).
    pub fn render(&self) {
        let Some(x) = self.x() else { return };
        if !self.inner.attached.replace(true) {
            transfer::attach_strip(&x, self.inner.root.upcast_ref());
        }
        let ids = x.tab_ids();
        let active = x.active_id();
        // gone tabs
        let gone: Vec<u64> = self.inner.tabs.borrow().keys().filter(|k| !ids.contains(k)).copied().collect();
        for id in gone {
            if let Some(t) = self.inner.tabs.borrow_mut().remove(&id) {
                self.inner.list.remove(&t.root);
            }
        }
        let mut prev: Option<gtk::Widget> = None;
        for id in &ids {
            let t = {
                let existing = self.inner.tabs.borrow().get(id).cloned();
                match existing {
                    Some(t) => t,
                    None => {
                        let t = self.make_tab(&x, *id);
                        self.inner.tabs.borrow_mut().insert(*id, t.clone());
                        self.inner.list.append(&t.root);
                        t
                    }
                }
            };
            // order
            let want_after = prev.clone();
            let cur_prev = t.root.prev_sibling();
            if cur_prev != want_after {
                self.inner.list.reorder_child_after(&t.root, want_after.as_ref());
            }
            prev = Some(t.root.clone().upcast());
            let Some(tab) = x.tab(*id) else { continue };
            let loc = tab.borrow().loc.clone();
            let label = x.loc_label(&loc, false);
            if t.label.label() != label {
                t.label.set_label(&label);
            }
            t.root.set_tooltip_text(Some(&label));
            let key = format!("{}|{}", loc.key(), x.state().trash_full);
            if *t.icon_key.borrow() != key {
                kit::controls::clear(&t.icon);
                t.icon.append(&x.loc_icon(&loc, 16));
                *t.icon_key.borrow_mut() = key;
            }
            if *id == active {
                t.root.add_css_class("is-active");
            } else {
                t.root.remove_css_class("is-active");
            }
            t.root.update_state(&[gtk::accessible::State::Selected(Some(*id == active))]);
        }
        // the separator after a tab sits before the next inactive tab (CSS can't select "next sibling")
        let order: Vec<Rc<TabWidgets>> = ids.iter().filter_map(|id| self.inner.tabs.borrow().get(id).cloned()).collect();
        for (n, t) in order.iter().enumerate() {
            let prev_active = n > 0 && order[n - 1].root.has_css_class("is-active");
            let show = n > 0 && !t.root.has_css_class("is-active") && !prev_active;
            if show {
                t.root.add_css_class("has-sep");
            } else {
                t.root.remove_css_class("has-sep");
            }
        }
        if ids.len() == 1 {
            self.inner.root.add_css_class("is-single");
        } else {
            self.inner.root.remove_css_class("is-single");
        }
        self.inner.add.set_tooltip_text(Some(s("newTab")));
        self.size_tabs();
    }

    /// Share the room: 240 px each at most, 52 at least.
    fn size_tabs(&self) {
        let n = self.inner.tabs.borrow().len().max(1) as i32;
        let avail = (self.inner.width.get() - 48).max(52);
        let w = (avail / n).clamp(52, 240);
        for t in self.inner.tabs.borrow().values() {
            t.root.set_size_request(w + 16, 32);
        }
    }


    /// The widget of a tab (transfer.rs: where a dropped tab goes).
    #[allow(dead_code)] // the API of builders B and C (see main.rs)
    pub fn tab_widget(&self, id: u64) -> Option<gtk::Widget> {
        self.inner.tabs.borrow().get(&id).map(|t| t.root.clone().upcast())
    }
}
