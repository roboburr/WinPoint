//! The navigation pane (last-webkit:vendor/ui/js/apps/explorer/navpane.js): Start, Galerij, the pinned folders, Deze pc
//! with its drives, the Prullenbak; a tree with lazily loaded sub folders, chevrons while the pointer is on
//! the pane, the accent pill on the selected row, Win11 TreeView keys, a context menu per row and drop
//! targets (dnd.rs).
use crate::actbox::ActBox;
use crate::explorer::{DirEntry, Explorer, WeakExplorer};
use crate::i18n::s;
use crate::menus;
use crate::model::Loc;
use crate::util::{self, st};
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone)]
struct Node {
    id: String,
    loc: Loc,
    label: String,
    /// A theme icon name, else `glyph` is drawn.
    icon: String,
    glyph: &'static str,
    glyph_class: &'static str,
    pinned: bool,
    expandable: bool,
    /// The drive this row is (under Deze pc).
    drive: Option<Value>,
}

enum Kids {
    Loading,
    List(Vec<DirEntry>),
}

struct Row {
    node: Node,
    widget: gtk::Box,
    level: i32,
}

struct Inner {
    x: WeakExplorer,
    root: gtk::ScrolledWindow,
    tree: gtk::Box,
    expanded: RefCell<HashSet<String>>,
    kids: RefCell<HashMap<String, Kids>>,
    rows: RefCell<Vec<Row>>,
}

/// The navigation pane; a cheap handle.
#[derive(Clone)]
pub struct NavPane {
    inner: Rc<Inner>,
}

impl NavPane {
    pub fn new(x: WeakExplorer) -> NavPane {
        let tree = gtk::Box::new(gtk::Orientation::Vertical, 0);
        tree.add_css_class("xp-tree");
        let root = gtk::ScrolledWindow::new();
        root.add_css_class("xp-nav");
        root.set_hscrollbar_policy(gtk::PolicyType::Never);
        root.set_child(Some(&tree));
        root.set_widget_name("explorer-nav");
        root.set_size_request(248, -1);
        root.set_hexpand(false);
        let inner = Rc::new(Inner {
            x,
            root,
            tree,
            expanded: RefCell::new(HashSet::from(["thispc".to_owned()])),
            kids: RefCell::new(HashMap::new()),
            rows: RefCell::new(Vec::new()),
        });
        let me = NavPane { inner };
        let keys = gtk::EventControllerKey::new();
        let hold = me.hold();
        keys.connect_key_pressed(move |_, key, _, _| match hold() {
            Some(m) => m.key(key),
            None => glib::Propagation::Proceed,
        });
        me.inner.tree.add_controller(keys);
        me
    }

    pub fn widget(&self) -> &gtk::ScrolledWindow {
        &self.inner.root
    }

    /// A weak handle for closures on our own widgets.
    fn hold(&self) -> impl Fn() -> Option<NavPane> + Clone + 'static {
        let w = Rc::downgrade(&self.inner);
        move || w.upgrade().map(|inner| NavPane { inner })
    }

    fn top_nodes(&self, x: &Explorer) -> Vec<Option<Node>> {
        let mut out = vec![
            Some(Node {
                id: "home".into(),
                loc: Loc::Home,
                label: s("home").into(),
                icon: String::new(),
                glyph: "home",
                glyph_class: "is-home",
                pinned: false,
                expandable: false,
                drive: None,
            }),
            Some(Node {
                id: "gallery".into(),
                loc: Loc::Gallery,
                label: s("gallery").into(),
                icon: String::new(),
                glyph: "image",
                glyph_class: "is-gallery",
                pinned: false,
                expandable: false,
                drive: None,
            }),
            None,
        ];
        for (path, label, icon) in x.pinned_list() {
            out.push(Some(Node {
                id: format!("pin:{path}"),
                loc: Loc::Folder(path),
                label,
                icon,
                glyph: "folder",
                glyph_class: "",
                pinned: true,
                expandable: true,
                drive: None,
            }));
        }
        out.push(None);
        out.push(Some(Node {
            id: "thispc".into(),
            loc: Loc::ThisPc,
            label: s("thisPC").into(),
            icon: "computer".into(),
            glyph: "pc",
            glyph_class: "",
            pinned: false,
            expandable: true,
            drive: None,
        }));
        if x.place("trash").is_some() {
            out.push(Some(Node {
                id: "trash".into(),
                loc: Loc::Trash,
                label: s("trash").into(),
                icon: if x.state().trash_full { "user-trash-full".into() } else { "user-trash".into() },
                glyph: "trash",
                glyph_class: "",
                pinned: false,
                expandable: false,
                drive: None,
            }));
        }
        out
    }

    fn children(&self, x: &Explorer, node: &Node) -> Option<Result<Vec<Node>, ()>> {
        if node.id == "thispc" {
            let vols = x.state().volumes.clone();
            return Some(Ok(vols
                .into_iter()
                .map(|v| {
                    let mount = st(&v, "mount").to_owned();
                    Node {
                        id: format!("vol:{mount}"),
                        loc: Loc::Folder(mount),
                        label: util::volume_label(&v),
                        icon: util::volume_icon(&v).into(),
                        glyph: "drive",
                        glyph_class: "",
                        pinned: false,
                        expandable: true,
                        drive: Some(v),
                    }
                })
                .collect()));
        }
        let Loc::Folder(path) = &node.loc else { return None };
        match self.inner.kids.borrow().get(path) {
            None => None,
            Some(Kids::Loading) => Some(Err(())),
            Some(Kids::List(list)) => Some(Ok(list
                .iter()
                .map(|d| Node {
                    id: format!("{}>{}", node.id, d.path),
                    loc: Loc::Folder(d.path.clone()),
                    label: d.name.clone(),
                    icon: if d.icon.is_empty() { "folder".into() } else { d.icon.clone() },
                    glyph: "folder",
                    glyph_class: "",
                    pinned: false,
                    expandable: true,
                    drive: None,
                })
                .collect())),
        }
    }

    fn load_kids(&self, path: &str) {
        let Some(x) = self.inner.x.upgrade() else { return };
        self.inner.kids.borrow_mut().insert(path.to_owned(), Kids::Loading);
        self.render();
        let me = self.clone();
        let p = path.to_owned();
        x.list_dirs(path, move |_, r| {
            me.inner.kids.borrow_mut().insert(p, Kids::List(r.unwrap_or_default()));
            me.render();
        });
    }

    fn toggle(&self, node: &Node, open: Option<bool>) {
        if !node.expandable {
            return;
        }
        let open = open.unwrap_or(!self.inner.expanded.borrow().contains(&node.id));
        if open {
            self.inner.expanded.borrow_mut().insert(node.id.clone());
            match &node.loc {
                Loc::Folder(p) => self.load_kids(p),
                _ if node.id == "thispc" => {
                    if let Some(x) = self.inner.x.upgrade() {
                        let me = self.clone();
                        x.load_volumes(move |_| me.render());
                    }
                }
                _ => {}
            }
        } else {
            self.inner.expanded.borrow_mut().remove(&node.id);
        }
        self.render();
    }

    fn row_widget(&self, x: &Explorer, node: &Node, level: i32, has_kids: bool, open: bool) -> gtk::Box {
        let ov = gtk::Overlay::new();
        let row = ActBox::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("xp-node");
        row.set_focusable(true);
        row.set_widget_name(&format!("nav-{}", node.id));
        if node.pinned {
            row.add_css_class("is-pinned");
        }
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.set_margin_start(level * 16);
        content.set_hexpand(true);
        let chev = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        chev.add_css_class("xp-node-chev");
        chev.set_size_request(20, 20);
        chev.set_valign(gtk::Align::Center);
        // the glyph expands to centre itself; the box must not pass that on
        chev.set_hexpand(false);
        if has_kids {
            let g = kit::icons::glyph(if open { "chevron-down" } else { "chevron-right" }, 12);
            g.set_hexpand(true);
            chev.append(&g);
            let cg = gtk::GestureClick::new();
            let hold = self.hold();
            let n = node.clone();
            cg.connect_pressed(move |g, _, _, _| {
                g.set_state(gtk::EventSequenceState::Claimed);
                if let Some(me) = hold() {
                    me.toggle(&n, None);
                }
            });
            chev.add_controller(cg);
        } else {
            chev.add_css_class("is-none");
        }
        content.append(&chev);
        let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon.add_css_class("xp-node-icon");
        icon.set_size_request(16, 16);
        icon.set_valign(gtk::Align::Center);
        if node.icon.is_empty() {
            let g = kit::icons::glyph(node.glyph, 16);
            g.add_css_class("xp-node-glyph");
            if !node.glyph_class.is_empty() {
                g.add_css_class(node.glyph_class);
            }
            icon.append(&g);
        } else {
            icon.append(&kit::icons::file_icon(&node.icon, 16));
        }
        content.append(&icon);
        let label = kit::controls::label(&node.label, "xp-node-label", false);
        label.set_hexpand(true);
        content.append(&label);
        if node.pinned {
            let pin = kit::icons::glyph("pin", 12);
            pin.add_css_class("xp-node-pin");
            pin.set_tooltip_text(Some(s("pinned")));
            content.append(&pin);
        }
        ov.set_child(Some(&content));
        let pill = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        pill.add_css_class("xp-node-pill");
        pill.set_size_request(3, 16);
        pill.set_halign(gtk::Align::Start);
        pill.set_valign(gtk::Align::Center);
        pill.set_can_target(false);
        ov.add_overlay(&pill);
        ov.set_hexpand(true);
        row.append(&ov);
        // a click navigates, the middle button opens a tab
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let w = self.inner.x.clone();
        let n = node.clone();
        let rw = row.downgrade();
        click.connect_released(move |g, _, _, _| {
            let Some(x) = w.upgrade() else { return };
            match g.current_button() {
                gdk::BUTTON_PRIMARY => {
                    if let Some(r) = rw.upgrade() {
                        r.grab_focus();
                    }
                    x.go_to(n.loc.clone());
                }
                gdk::BUTTON_MIDDLE => {
                    x.open_tab(n.loc.clone(), false);
                }
                _ => {}
            }
        });
        row.add_controller(click);
        let w = self.inner.x.clone();
        let n = node.clone();
        row.connect_activated(move |_| {
            if let Some(x) = w.upgrade() {
                x.go_to(n.loc.clone());
            }
        });
        let w = self.inner.x.clone();
        let n = node.clone();
        kit::controls::bind_context_menu(&row, move |_, _| {
            let x = w.upgrade()?;
            Some((Vec::new(), menus::nav_menu(&x, &n.loc, n.drive.clone())))
        });
        let hold = self.hold();
        let n = node.clone();
        crate::dnd::attach_nav_row(
            x,
            row.upcast_ref(),
            &node.loc,
            Rc::new(move || {
                if let Some(me) = hold() {
                    me.toggle(&n, Some(true));
                }
            }),
        );
        row.upcast()
    }

    /// Build the tree again (pinned folders, drives, the Prullenbak icon, expanded folders).
    pub fn render(&self) {
        let Some(x) = self.inner.x.upgrade() else { return };
        let focused = self.inner.rows.borrow().iter().find(|r| r.widget.has_focus()).map(|r| r.node.id.clone());
        let vadj = self.inner.root.vadjustment();
        let scroll = vadj.value();
        kit::controls::clear(&self.inner.tree);
        self.inner.rows.borrow_mut().clear();
        for n in self.top_nodes(&x) {
            match n {
                None => {
                    let sep = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    sep.add_css_class("xp-tree-sep");
                    sep.set_size_request(-1, 1);
                    self.inner.tree.append(&sep);
                }
                Some(n) => self.add_rows(&x, &n, 0),
            }
        }
        self.mark_selected();
        vadj.set_value(scroll);
        if let Some(id) = focused
            && let Some(r) = self.inner.rows.borrow().iter().find(|r| r.node.id == id)
        {
            r.widget.grab_focus();
        }
    }

    fn add_rows(&self, x: &Explorer, node: &Node, level: i32) {
        let open = self.inner.expanded.borrow().contains(&node.id);
        let kids = if open { self.children(x, node) } else { None };
        let has_kids = node.expandable && !matches!(&kids, Some(Ok(list)) if list.is_empty());
        let w = self.row_widget(x, node, level, has_kids, open);
        self.inner.tree.append(&w);
        self.inner.rows.borrow_mut().push(Row { node: node.clone(), widget: w, level });
        match kids {
            Some(Err(())) => {
                let l = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                l.add_css_class("xp-node");
                l.add_css_class("is-loading");
                let t = kit::controls::label(s("loading"), "xp-node-label", false);
                t.set_margin_start((level + 1) * 16 + 20);
                l.append(&t);
                self.inner.tree.append(&l);
            }
            Some(Ok(list)) => {
                for c in list {
                    self.add_rows(x, &c, level + 1);
                }
            }
            None => {}
        }
    }

    /// The accent pill on the row of the active location.
    pub fn mark_selected(&self) {
        let Some(x) = self.inner.x.upgrade() else { return };
        let key = x.loc().key();
        let mut found = false;
        for r in self.inner.rows.borrow().iter() {
            let on = !found && r.node.loc.key() == key;
            if on {
                found = true;
                r.widget.add_css_class("is-selected");
            } else {
                r.widget.remove_css_class("is-selected");
            }
            r.widget.update_state(&[gtk::accessible::State::Selected(Some(on))]);
        }
    }

    /// A folder changed: its children again (None: all).
    pub fn invalidate(&self, path: Option<&str>) {
        match path {
            Some(p) => {
                self.inner.kids.borrow_mut().remove(p);
            }
            None => self.inner.kids.borrow_mut().clear(),
        }
        let reload: Vec<String> = self
            .inner
            .rows
            .borrow()
            .iter()
            .filter(|r| self.inner.expanded.borrow().contains(&r.node.id))
            .filter_map(|r| match &r.node.loc {
                Loc::Folder(p) if path.is_none_or(|q| q == p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        for p in reload {
            self.load_kids(&p);
        }
    }

    fn key(&self, key: gdk::Key) -> glib::Propagation {
        let rows = self.inner.rows.borrow();
        let Some(cur) = rows.iter().position(|r| r.widget.has_focus()) else { return glib::Propagation::Proceed };
        let focus = |i: usize| {
            if let Some(r) = rows.get(i) {
                r.widget.grab_focus();
            }
        };
        match key {
            gdk::Key::Down => focus((cur + 1).min(rows.len() - 1)),
            gdk::Key::Up => focus(cur.saturating_sub(1)),
            gdk::Key::Home => focus(0),
            gdk::Key::End => focus(rows.len() - 1),
            gdk::Key::Right => {
                let n = rows[cur].node.clone();
                if n.expandable && !self.inner.expanded.borrow().contains(&n.id) {
                    drop(rows);
                    self.toggle(&n, Some(true));
                    return glib::Propagation::Stop;
                }
                focus(cur + 1);
            }
            gdk::Key::Left => {
                let n = rows[cur].node.clone();
                if self.inner.expanded.borrow().contains(&n.id) {
                    drop(rows);
                    self.toggle(&n, Some(false));
                    return glib::Propagation::Stop;
                }
                let lvl = rows[cur].level;
                if let Some(i) = (0..cur).rev().find(|i| rows[*i].level < lvl) {
                    focus(i);
                }
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                let loc = rows[cur].node.loc.clone();
                drop(rows);
                if let Some(x) = self.inner.x.upgrade() {
                    x.go_to(loc);
                }
            }
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    }
}
