//! Pivot tabs (components.js Tabs / Pivot, SelectorBar look: text tabs with the 16 x 3 accent pill under the
//! selected one) and the segmented control (WinUI Segmented). Left / Right / Home / End move the selection.
use crate::icons;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::rc::Rc;

struct Item {
    id: String,
    button: gtk::Button,
    pill: Option<gtk::Box>,
    label: gtk::Label,
}

struct Inner {
    items: RefCell<Vec<Item>>,
    selected: RefCell<Option<String>>,
    handlers: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    segmented: bool,
}

fn build(segmented: bool) -> (gtk::Box, Rc<Inner>) {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, if segmented { 2 } else { 0 });
    root.add_css_class(if segmented { "w11-segmented" } else { "w11-pivot" });
    root.set_halign(gtk::Align::Start);
    let inner = Rc::new(Inner {
        items: RefCell::new(Vec::new()),
        selected: RefCell::new(None),
        handlers: RefCell::new(Vec::new()),
        segmented,
    });
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&inner);
    keys.connect_key_pressed(move |_, key, _, _| {
        let Some(inner) = weak.upgrade() else { return glib::Propagation::Proceed };
        let ids: Vec<String> = inner.items.borrow().iter().filter(|i| i.button.is_sensitive()).map(|i| i.id.clone()).collect();
        if ids.is_empty() {
            return glib::Propagation::Proceed;
        }
        let cur = inner.selected.borrow().clone().and_then(|s| ids.iter().position(|i| *i == s)).unwrap_or(0);
        let next = match key {
            gdk::Key::Right => (cur + 1) % ids.len(),
            gdk::Key::Left => (cur + ids.len() - 1) % ids.len(),
            gdk::Key::Home => 0,
            gdk::Key::End => ids.len() - 1,
            _ => return glib::Propagation::Proceed,
        };
        choose(&inner, &ids[next], true);
        if let Some(i) = inner.items.borrow().iter().find(|i| i.id == ids[next]) {
            i.button.grab_focus();
        }
        glib::Propagation::Stop
    });
    root.add_controller(keys);
    super::own(&root, &inner);
    (root, inner)
}

fn set_items(root: &gtk::Box, inner: &Rc<Inner>, items: &[(&str, &str, &str)], selected: Option<&str>) {
    super::misc::clear(root);
    let mut out = Vec::new();
    for (id, label, glyph) in items {
        let button = gtk::Button::new();
        button.add_css_class(if inner.segmented { "w11-segment" } else { "w11-pivot-item" });
        button.set_widget_name(&format!("tab-{id}"));
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        body.add_css_class(if inner.segmented { "w11-segment-body" } else { "w11-pivot-body" });
        body.set_halign(gtk::Align::Center);
        if !glyph.is_empty() {
            body.append(&icons::glyph(glyph, 16));
        }
        let l = gtk::Label::new(Some(label));
        body.append(&l);
        button.set_child(Some(&body));
        let id_s = id.to_string();
        let weak = Rc::downgrade(inner);
        button.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                choose(&inner, &id_s, true);
            }
        });
        let pill = if inner.segmented {
            button.set_hexpand(true);
            root.append(&button);
            None
        } else {
            let col = gtk::Box::new(gtk::Orientation::Vertical, 0);
            button.set_valign(gtk::Align::Start);
            col.append(&button);
            let pill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            pill.add_css_class("w11-pill");
            pill.add_css_class("w11-pill-h");
            pill.set_halign(gtk::Align::Center);
            pill.set_margin_top(1);
            pill.set_opacity(0.0);
            col.append(&pill);
            root.append(&col);
            Some(pill)
        };
        out.push(Item { id: id.to_string(), button, pill, label: l });
    }
    *inner.items.borrow_mut() = out;
    let sel = selected.map(str::to_owned).or_else(|| items.first().map(|i| i.0.to_owned()));
    *inner.selected.borrow_mut() = sel;
    render(inner);
}

fn render(inner: &Inner) {
    let sel = inner.selected.borrow().clone();
    for it in inner.items.borrow().iter() {
        let on = sel.as_deref() == Some(it.id.as_str());
        if on {
            it.button.add_css_class("w11-selected");
        } else {
            it.button.remove_css_class("w11-selected");
        }
        if let Some(p) = &it.pill {
            p.set_opacity(if on { 1.0 } else { 0.0 });
        }
    }
}

fn choose(inner: &Rc<Inner>, id: &str, fire: bool) {
    if inner.selected.borrow().as_deref() == Some(id) {
        return;
    }
    *inner.selected.borrow_mut() = Some(id.to_owned());
    render(inner);
    if fire {
        let hs: Vec<Rc<dyn Fn(&str)>> = inner.handlers.borrow().clone();
        for h in hs {
            h(id);
        }
    }
}

macro_rules! tab_api {
    ($t:ident) => {
        impl $t {
            pub fn widget(&self) -> &gtk::Box {
                &self.root
            }
            /// Replace the tabs: (id, label, glyph or "").
            pub fn set_items(&self, items: &[(&str, &str, &str)], selected: Option<&str>) {
                set_items(&self.root, &self.inner, items, selected);
            }
            /// Select without running the handlers.
            pub fn select(&self, id: &str) {
                choose(&self.inner, id, false);
            }
            pub fn selected(&self) -> Option<String> {
                self.inner.selected.borrow().clone()
            }
            /// Run `f(id)` when the user picks a tab.
            pub fn connect_selected(&self, f: impl Fn(&str) + 'static) {
                self.inner.handlers.borrow_mut().push(Rc::new(f));
            }
            /// Relabel a tab.
            pub fn set_label(&self, id: &str, label: &str) {
                if let Some(i) = self.inner.items.borrow().iter().find(|i| i.id == id) {
                    i.label.set_label(label);
                }
            }
            /// The button of a tab (named "tab-<id>").
            pub fn button(&self, id: &str) -> Option<gtk::Button> {
                self.inner.items.borrow().iter().find(|i| i.id == id).map(|i| i.button.clone())
            }
        }
    };
}

/// Pivot tabs (SelectorBar).
#[derive(Clone)]
pub struct Pivot {
    root: gtk::Box,
    inner: Rc<Inner>,
}

impl Pivot {
    pub fn new(items: &[(&str, &str, &str)], selected: Option<&str>) -> Pivot {
        let (root, inner) = build(false);
        set_items(&root, &inner, items, selected);
        Pivot { root, inner }
    }
}

tab_api!(Pivot);

/// A segmented control: equal buttons in one bordered strip, the selected one raised.
#[derive(Clone)]
pub struct Segmented {
    root: gtk::Box,
    inner: Rc<Inner>,
}

impl Segmented {
    pub fn new(items: &[(&str, &str, &str)], selected: Option<&str>) -> Segmented {
        let (root, inner) = build(true);
        root.set_homogeneous(true);
        set_items(&root, &inner, items, selected);
        Segmented { root, inner }
    }
}

tab_api!(Segmented);
