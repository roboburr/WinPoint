//! The navigation view (components.js NavView, `.fl-nav`): the left list of 36 px items with 16 px glyphs, the
//! 3 x 16 accent pill on the selected one, headings, separators, child items under an expandable parent,
//! badges, footer items, an optional header (a search box), and a compact mode (48 px, glyphs only, with the
//! hamburger to open it again). Arrow keys, Home and End move through the items; Enter selects.
use crate::i18n;
use crate::icons;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NavKind {
    Item,
    Separator,
    Heading,
}

/// One entry of the navigation view.
#[derive(Clone, Debug)]
pub struct NavItem {
    pub kind: NavKind,
    pub id: String,
    pub label: String,
    /// A glyph name, or an app / theme icon name.
    pub icon: String,
    pub children: Vec<NavItem>,
    pub disabled: bool,
    pub badge: Option<String>,
    pub expanded: bool,
    /// false: clicking only opens and closes the children.
    pub selectable: bool,
}

impl NavItem {
    pub fn new(id: &str, label: &str, icon: &str) -> NavItem {
        NavItem {
            kind: NavKind::Item,
            id: id.into(),
            label: label.into(),
            icon: icon.into(),
            children: Vec::new(),
            disabled: false,
            badge: None,
            expanded: false,
            selectable: true,
        }
    }
    pub fn separator() -> NavItem {
        NavItem { kind: NavKind::Separator, ..NavItem::new("", "", "") }
    }
    pub fn heading(text: &str) -> NavItem {
        NavItem { kind: NavKind::Heading, ..NavItem::new("", text, "") }
    }
    pub fn children(mut self, children: Vec<NavItem>) -> NavItem {
        self.children = children;
        self
    }
    pub fn disabled(mut self, d: bool) -> NavItem {
        self.disabled = d;
        self
    }
    pub fn badge(mut self, b: &str) -> NavItem {
        self.badge = Some(b.into());
        self
    }
    pub fn expanded(mut self, e: bool) -> NavItem {
        self.expanded = e;
        self
    }
    pub fn selectable(mut self, s: bool) -> NavItem {
        self.selectable = s;
        self
    }
}

struct Node {
    item: NavItem,
    button: gtk::Button,
    pill: gtk::Box,
    label: gtk::Label,
    badge: gtk::Box,
    chevron: Option<gtk::Image>,
    group: Option<gtk::Box>,
    parent: Option<String>,
}

struct NavInner {
    root: glib::WeakRef<gtk::Box>,
    toggle_row: gtk::Box,
    header: gtk::Box,
    list: gtk::Box,
    footer: gtk::Box,
    nodes: RefCell<HashMap<String, Node>>,
    order: RefCell<Vec<String>>,
    selected: RefCell<Option<String>>,
    expanded: RefCell<HashSet<String>>,
    compact: Cell<bool>,
    handlers: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    compact_below: Cell<i32>,
    user_compact: Cell<Option<bool>>,
}

/// The navigation view. `widget()` goes left of the content (it scrolls by itself).
#[derive(Clone)]
pub struct NavView {
    root: gtk::Box,
    inner: Rc<NavInner>,
}

impl NavView {
    pub fn new(items: Vec<NavItem>) -> NavView {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("w11-nav");
        root.set_vexpand(true);
        root.set_hexpand(false);
        let toggle_row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        toggle_row.add_css_class("w11-nav-toggle-row");
        let toggle = super::buttons::icon_button("menu", "", 36);
        toggle.set_size_request(40, 36);
        toggle.set_halign(gtk::Align::Start);
        toggle.set_widget_name("nav-toggle");
        toggle_row.append(&toggle);
        toggle_row.set_visible(false);
        let header = gtk::Box::new(gtk::Orientation::Vertical, 0);
        header.add_css_class("w11-nav-header");
        header.set_visible(false);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_vexpand(true);
        scroller.set_child(Some(&list));
        let footer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        footer.add_css_class("w11-nav-footer");
        footer.set_visible(false);
        root.append(&toggle_row);
        root.append(&header);
        root.append(&scroller);
        root.append(&footer);
        let inner = Rc::new(NavInner {
            root: root.downgrade(),
            toggle_row,
            header,
            list,
            footer,
            nodes: RefCell::new(HashMap::new()),
            order: RefCell::new(Vec::new()),
            selected: RefCell::new(None),
            expanded: RefCell::new(HashSet::new()),
            compact: Cell::new(false),
            handlers: RefCell::new(Vec::new()),
            compact_below: Cell::new(0),
            user_compact: Cell::new(None),
        });
        super::own(&root, &inner);
        let me = NavView { root, inner };
        let weak = Rc::downgrade(&me.inner);
        i18n::bind(&toggle, move |b| {
            let compact = weak.upgrade().map(|i| i.compact.get()).unwrap_or(false);
            b.set_tooltip_text(Some(i18n::t(
                if compact { "Navigatie openen" } else { "Navigatie sluiten" },
                if compact { "Open navigation" } else { "Close navigation" },
            )));
        });
        let weak = Rc::downgrade(&me.inner);
        toggle.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade().and_then(NavView::of) {
                let c = !me.inner.compact.get();
                me.inner.user_compact.set(Some(c));
                me.set_compact(c);
            }
        });
        me.install_keys();
        me.set_items(items, Vec::new());
        me
    }

    fn of(inner: Rc<NavInner>) -> Option<NavView> {
        let root = inner.root.upgrade()?;
        Some(NavView { root, inner })
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Replace the items (and footer items); the selection stays when its id is still there.
    pub fn set_items(&self, items: Vec<NavItem>, footer_items: Vec<NavItem>) {
        super::misc::clear(&self.inner.list);
        super::misc::clear(&self.inner.footer);
        self.inner.nodes.borrow_mut().clear();
        self.inner.order.borrow_mut().clear();
        for it in &items {
            if it.expanded {
                self.inner.expanded.borrow_mut().insert(it.id.clone());
            }
            let w = self.build(it, 0, None);
            self.inner.list.append(&w);
        }
        for it in &footer_items {
            let w = self.build(it, 0, None);
            self.inner.footer.append(&w);
        }
        self.inner.footer.set_visible(!footer_items.is_empty());
        self.mark();
        self.apply_compact();
    }

    fn build(&self, it: &NavItem, depth: i32, parent: Option<&str>) -> gtk::Widget {
        match it.kind {
            NavKind::Separator => {
                let s = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                s.add_css_class("w11-nav-sep");
                return s.upcast();
            }
            NavKind::Heading => {
                let l = super::label(&it.label, "w11-nav-heading", false);
                l.add_css_class("w11-nav-label");
                return l.upcast();
            }
            NavKind::Item => {}
        }
        let button = gtk::Button::new();
        button.add_css_class("w11-nav-item");
        button.set_widget_name(&format!("nav-{}", it.id));
        let ov = gtk::Overlay::new();
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        body.add_css_class("w11-nav-item-body");
        body.set_margin_start(12 + depth * 28);
        body.set_valign(gtk::Align::Center);
        let icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_box.set_size_request(16, 16);
        icon_box.set_valign(gtk::Align::Center);
        if !it.icon.is_empty() {
            if icons::has_glyph(&it.icon) {
                icon_box.append(&icons::glyph(&it.icon, 16));
            } else {
                icon_box.append(&icons::app_icon(&it.icon, 16));
            }
        }
        let label = super::label(&it.label, "w11-nav-label", false);
        label.set_hexpand(true);
        label.set_max_width_chars(28);
        let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        badge.set_valign(gtk::Align::Center);
        body.append(&icon_box);
        body.append(&label);
        body.append(&badge);
        let has_kids = !it.children.is_empty();
        let chevron = if has_kids {
            let g = icons::glyph("chevron-down", 12);
            let cb = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            cb.add_css_class("w11-nav-chevron");
            cb.set_valign(gtk::Align::Center);
            cb.set_hexpand(false);
            g.set_halign(gtk::Align::Center);
            g.set_hexpand(true);
            cb.append(&g);
            let click = gtk::GestureClick::new();
            let id = it.id.clone();
            let weak = Rc::downgrade(&self.inner);
            click.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                if let Some(me) = weak.upgrade().and_then(NavView::of) {
                    let open = !me.inner.expanded.borrow().contains(&id);
                    me.expand(&id, open);
                }
            });
            cb.add_controller(click);
            body.append(&cb);
            Some(g)
        } else {
            None
        };
        ov.set_child(Some(&body));
        let pill = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pill.add_css_class("w11-pill");
        pill.set_halign(gtk::Align::Start);
        pill.set_valign(gtk::Align::Center);
        pill.set_visible(false);
        ov.add_overlay(&pill);
        button.set_child(Some(&ov));
        button.set_sensitive(!it.disabled);
        button.set_tooltip_text(None);
        let id = it.id.clone();
        let weak = Rc::downgrade(&self.inner);
        button.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade().and_then(NavView::of) {
                me.user_select(&id);
            }
        });
        let out: gtk::Widget;
        let group = if has_kids {
            let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
            wrap.append(&button);
            let g = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for c in &it.children {
                let w = self.build(c, depth + 1, Some(&it.id));
                g.append(&w);
            }
            g.set_visible(self.inner.expanded.borrow().contains(&it.id));
            wrap.append(&g);
            out = wrap.upcast();
            Some(g)
        } else {
            out = button.clone().upcast();
            None
        };
        self.inner.order.borrow_mut().push(it.id.clone());
        let node = Node {
            item: it.clone(),
            button,
            pill,
            label,
            badge,
            chevron,
            group,
            parent: parent.map(str::to_owned),
        };
        if let Some(b) = &it.badge {
            set_badge_box(&node.badge, Some(b));
        }
        self.inner.nodes.borrow_mut().insert(it.id.clone(), node);
        out
    }

    fn user_select(&self, id: &str) {
        let (has_kids, selectable, disabled) = match self.inner.nodes.borrow().get(id) {
            Some(n) => (n.group.is_some(), n.item.selectable, n.item.disabled),
            None => return,
        };
        if disabled {
            return;
        }
        if has_kids && !selectable {
            let open = !self.inner.expanded.borrow().contains(id);
            self.expand(id, open);
            return;
        }
        if has_kids && !self.inner.expanded.borrow().contains(id) {
            self.expand(id, true);
        }
        if self.inner.selected.borrow().as_deref() == Some(id) {
            return;
        }
        *self.inner.selected.borrow_mut() = Some(id.to_owned());
        self.mark();
        let hs: Vec<Rc<dyn Fn(&str)>> = self.inner.handlers.borrow().clone();
        for h in hs {
            h(id);
        }
    }

    fn mark(&self) {
        let sel = self.inner.selected.borrow().clone();
        let nodes = self.inner.nodes.borrow();
        // a collapsed parent shows the selection of its hidden child
        let mut visible = sel.clone();
        if let Some(s) = &sel
            && let Some(n) = nodes.get(s)
            && let Some(p) = &n.parent
            && !self.inner.expanded.borrow().contains(p)
        {
            visible = Some(p.clone());
        }
        for (id, n) in nodes.iter() {
            let on = visible.as_deref() == Some(id.as_str());
            n.pill.set_visible(on);
            if on {
                n.button.add_css_class("w11-selected");
            } else {
                n.button.remove_css_class("w11-selected");
            }
        }
    }

    /// Select without running the handlers (an unknown id clears the mark).
    pub fn select(&self, id: &str) {
        let parent = self.inner.nodes.borrow().get(id).and_then(|n| n.parent.clone());
        if let Some(p) = parent
            && !self.inner.expanded.borrow().contains(&p)
        {
            self.expand(&p, true);
        }
        *self.inner.selected.borrow_mut() = Some(id.to_owned());
        self.mark();
    }

    pub fn selected(&self) -> Option<String> {
        self.inner.selected.borrow().clone()
    }

    /// Run `f(id)` when the user selects an item.
    pub fn connect_selected(&self, f: impl Fn(&str) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }

    /// Open or close a parent's children.
    pub fn expand(&self, id: &str, open: bool) {
        if open {
            self.inner.expanded.borrow_mut().insert(id.to_owned());
        } else {
            self.inner.expanded.borrow_mut().remove(id);
        }
        if let Some(n) = self.inner.nodes.borrow().get(id) {
            if let Some(g) = &n.group {
                g.set_visible(open && !self.inner.compact.get());
            }
            if let Some(c) = &n.chevron {
                icons::set_glyph(c, if open { "chevron-up" } else { "chevron-down" });
            }
        }
        self.mark();
    }

    /// Show a badge on an item (None removes it): a number or "" for a dot.
    pub fn set_badge(&self, id: &str, badge: Option<&str>) {
        if let Some(n) = self.inner.nodes.borrow().get(id) {
            set_badge_box(&n.badge, badge);
        }
    }

    /// Relabel an item.
    pub fn set_label(&self, id: &str, label: &str) {
        if let Some(n) = self.inner.nodes.borrow().get(id) {
            n.label.set_label(label);
            if self.inner.compact.get() {
                n.button.set_tooltip_text(Some(label));
            }
        }
    }

    /// A widget above the list (Settings' user card and search box); None removes it.
    pub fn set_header(&self, w: Option<&impl IsA<gtk::Widget>>) {
        super::misc::clear(&self.inner.header);
        if let Some(w) = w {
            self.inner.header.append(w.as_ref());
        }
        self.inner.header.set_visible(w.is_some() && !self.inner.compact.get());
    }

    /// Compact: 48 px wide, glyphs only (with tooltips), the hamburger above.
    pub fn set_compact(&self, compact: bool) {
        self.inner.compact.set(compact);
        self.apply_compact();
    }

    pub fn compact(&self) -> bool {
        self.inner.compact.get()
    }

    /// Always show the hamburger (WinUI PaneToggleButton); by default only while compact.
    pub fn set_pane_toggle(&self, visible: bool) {
        self.inner.toggle_row.set_visible(visible || self.inner.compact.get());
        if visible {
            self.inner.toggle_row.add_css_class("w11-always");
        } else {
            self.inner.toggle_row.remove_css_class("w11-always");
        }
    }

    /// Go compact by itself while the window is narrower than `px` (appframe.js compactBelow, 760), unless the
    /// user chose with the hamburger.
    pub fn set_compact_below(&self, px: i32) {
        self.inner.compact_below.set(px);
        let weak = Rc::downgrade(&self.inner);
        self.root.connect_map(move |root| {
            let Some(window) = root.root().and_downcast::<gtk::Window>() else { return };
            let w2 = weak.clone();
            let check = move |win: &gtk::Window| {
                if let Some(me) = w2.upgrade().and_then(NavView::of) {
                    let limit = me.inner.compact_below.get();
                    if limit > 0 && me.inner.user_compact.get().is_none() {
                        let narrow = win.width() > 0 && win.width() < limit;
                        if narrow != me.inner.compact.get() {
                            me.set_compact(narrow);
                        }
                    }
                }
            };
            check(&window);
            let c2 = check.clone();
            window.connect_default_width_notify(move |w| c2(w));
            window.connect_maximized_notify(move |w| {
                let w = w.clone();
                let c = check.clone();
                glib::idle_add_local_once(move || c(&w));
            });
        });
    }

    fn apply_compact(&self) {
        let compact = self.inner.compact.get();
        let root = &self.root;
        if compact {
            root.add_css_class("w11-compact");
        } else {
            root.remove_css_class("w11-compact");
        }
        self.inner.toggle_row.set_visible(compact || self.inner.toggle_row.has_css_class("w11-always"));
        self.inner.header.set_visible(!compact && self.inner.header.first_child().is_some());
        let expanded = self.inner.expanded.borrow().clone();
        for (id, n) in self.inner.nodes.borrow().iter() {
            n.label.set_visible(!compact);
            n.badge.set_visible(!compact && n.badge.first_child().is_some());
            if let Some(c) = &n.chevron
                && let Some(p) = c.parent()
            {
                p.set_visible(!compact);
            }
            if let Some(g) = &n.group {
                g.set_visible(!compact && expanded.contains(id));
            }
            let text = n.label.label();
            n.button.set_tooltip_text(if compact { Some(text.as_str()) } else { None });
        }
        let mut child = self.inner.list.first_child();
        while let Some(c) = child {
            if c.has_css_class("w11-nav-heading") {
                c.set_visible(!compact);
            }
            child = c.next_sibling();
        }
        self.mark();
    }

    fn install_keys(&self) {
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&self.inner);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(me) = weak.upgrade().and_then(NavView::of) else { return glib::Propagation::Proceed };
            let visible: Vec<gtk::Button> = {
                let nodes = me.inner.nodes.borrow();
                me.inner
                    .order
                    .borrow()
                    .iter()
                    .filter_map(|id| nodes.get(id))
                    .map(|n| n.button.clone())
                    .filter(|b| b.is_drawable() && b.is_sensitive())
                    .collect()
            };
            let focused = visible.iter().position(|b| b.has_focus());
            let target = match (key, focused) {
                (gdk::Key::Down, Some(i)) => visible.get(i + 1),
                (gdk::Key::Up, Some(i)) if i > 0 => visible.get(i - 1),
                (gdk::Key::Home, Some(_)) => visible.first(),
                (gdk::Key::End, Some(_)) => visible.last(),
                _ => None,
            };
            if let Some(b) = target {
                b.grab_focus();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.root.add_controller(keys);
    }

    /// The button of an item (named "nav-<id>" for the dev channel).
    pub fn item_button(&self, id: &str) -> Option<gtk::Button> {
        self.inner.nodes.borrow().get(id).map(|n| n.button.clone())
    }
}

fn set_badge_box(slot: &gtk::Box, badge: Option<&str>) {
    super::misc::clear(slot);
    if let Some(b) = badge {
        slot.append(&super::misc::badge(b, "attention"));
    }
    slot.set_visible(badge.is_some());
}
