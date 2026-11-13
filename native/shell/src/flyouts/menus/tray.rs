//! The tray overflow "Verborgen pictogrammen" (vendor/ui/js/shell/tray.js): the SNI icons that are not shown on
//! the taskbar, in a grid of 40 x 40 cells, at most 5 per row. Same mouse behaviour as on the taskbar: left =
//! Activate (ItemIsMenu or fallback:'menu' opens the menu), middle = SecondaryActivate, right = the menu
//! (the app's dbusmenu through tray.menu in our own menu, else the app's own ContextMenu).
use super::menu::{self, Icon, Item};
use super::util;
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const MAX_COLS: usize = 5;

pub struct Tray {
    shell: Rc<Shell>,
    root: gtk::Box,
    grid: gtk::Grid,
    items: RefCell<Vec<Value>>,
    supported: Cell<bool>,
    /// The chevron's centre when open_tray placed the overflow.
    anchor: Cell<Option<i32>>,
}

/// The tray item whose dbusmenu is open in the jumplist flyout.
struct OpenMenu {
    id: String,
    x: i32,
    y: i32,
    json: String,
    clicked: Rc<Cell<bool>>,
    busy: bool,
    again: bool,
}

thread_local! {
    static TRAY: RefCell<Option<Rc<Tray>>> = const { RefCell::new(None) };
    static MENU: RefCell<Option<OpenMenu>> = const { RefCell::new(None) };
}

struct Handle(Rc<Tray>);

impl Flyout for Handle {
    fn name(&self) -> &'static str {
        "tray"
    }
    fn widget(&self) -> gtk::Widget {
        self.0.root.clone().upcast()
    }
    fn placement(&self) -> Placement {
        Placement::BottomRight
    }
    fn opened(&self) {
        // never close from inside the host's open()
        self.0.render_with(false);
        self.0.place();
        if self.0.overflow().is_empty() {
            // nothing hidden: Win11 has no overflow then (the chevron hides)
            glib::idle_add_local_once(|| {
                let shell = crate::core::shell();
                if shell.flyouts().is_open("tray") {
                    shell.flyouts().close();
                }
            });
        }
    }
}

pub fn register(shell: &Rc<Shell>) {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("w11-tray-flyout");
    root.set_widget_name("menus-tray");
    root.set_tooltip_text(Some(&shell.t("Verborgen pictogrammen", "Hidden icons")));
    let grid = gtk::Grid::new();
    grid.add_css_class("w11-tray-grid");
    grid.set_row_homogeneous(true);
    grid.set_column_homogeneous(true);
    root.append(&grid);
    let tray = Rc::new(Tray {
        shell: shell.clone(),
        root,
        grid,
        items: RefCell::new(Vec::new()),
        supported: Cell::new(false),
        anchor: Cell::new(None),
    });
    shell.flyouts().register(Rc::new(Handle(tray.clone())));
    TRAY.with(|t| *t.borrow_mut() = Some(tray.clone()));

    shell.on("tray", |event, data| {
        let Some(tray) = TRAY.with(|t| t.borrow().clone()) else { return };
        match event {
            "tray.changed" => {
                tray.supported.set(true);
                tray.set_items(data["items"].as_array().cloned().unwrap_or_default());
            }
            "tray.status" => {
                if data["supported"] == true {
                    tray.load();
                } else {
                    tray.supported.set(false);
                    tray.set_items(Vec::new());
                }
            }
            "tray.menuChanged" => {
                let id = data["id"].as_str().unwrap_or("").to_owned();
                if MENU.with(|m| m.borrow().as_ref().is_some_and(|m| m.id == id)) {
                    refresh_menu();
                }
            }
            _ => {}
        }
    });
    shell.on("settings.changed", |_, _| {
        if let Some(tray) = TRAY.with(|t| t.borrow().clone()) {
            tray.render();
        }
    });
    tray.load();
}

/// Open (or close) the overflow centred over the chevron at `x`.
pub fn open(shell: &Rc<Shell>, x: Option<i32>) {
    let host = shell.flyouts();
    if host.is_open("tray") {
        host.close();
        return;
    }
    if let Some(tray) = TRAY.with(|t| t.borrow().clone()) {
        if x.is_some() {
            tray.anchor.set(x);
        }
        tray.render();
        tray.place();
    }
    host.open("tray");
}

/// The tray items split as the taskbar shows them: (hiddenMenu, on the taskbar, in the overflow) by the
/// settings taskbar.tray {hiddenMenu, shown, order}; Passive items are not shown at all.
pub fn arrange(shell: &Shell, items: &[Value]) -> (bool, Vec<Value>, Vec<Value>) {
    let prefs = shell.setting("taskbar.tray");
    let list = |v: &Value| -> Vec<String> {
        v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
    };
    let hidden_menu = prefs["hiddenMenu"] != false;
    let shown = list(&prefs["shown"]);
    let order = list(&prefs["order"]);
    let mut sorted: Vec<(usize, usize, Value)> = items
        .iter()
        .filter(|x| x["status"].as_str() != Some("passive") && x["id"].is_string())
        .enumerate()
        .map(|(i, x)| {
            let key = util::str_of(x, "key");
            let rank = order.iter().position(|k| *k == key).unwrap_or(order.len() + i);
            (rank, i, x.clone())
        })
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let sorted: Vec<Value> = sorted.into_iter().map(|(_, _, x)| x).collect();
    if !hidden_menu {
        return (false, sorted, Vec::new());
    }
    let (bar, overflow) = sorted.into_iter().partition(|x| shown.contains(&util::str_of(x, "key")));
    (true, bar, overflow)
}

fn tooltip_of(x: &Value) -> String {
    let title = x["tooltip"]["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .or(x["title"].as_str().filter(|s| !s.trim().is_empty()))
        .or(x["appName"].as_str())
        .unwrap_or("")
        .trim()
        .to_owned();
    let text = x["tooltip"]["text"].as_str().unwrap_or("").trim().to_owned();
    if !text.is_empty() && text != title {
        if title.is_empty() { text } else { format!("{title}\n{text}") }
    } else if title.is_empty() {
        util::str_of(x, "appName")
    } else {
        title
    }
}

impl Tray {
    fn load(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.shell.call("tray.list", json!({}), move |r| {
            let Some(tray) = weak.upgrade() else { return };
            match r {
                Ok(v) => {
                    tray.supported.set(v["supported"] == true);
                    let items = if v["supported"] == true { v["items"].as_array().cloned().unwrap_or_default() } else { Vec::new() };
                    tray.set_items(items);
                }
                Err(_) => {
                    tray.supported.set(false);
                    tray.set_items(Vec::new());
                }
            }
        });
    }

    fn set_items(self: &Rc<Self>, items: Vec<Value>) {
        *self.items.borrow_mut() = items.into_iter().filter(|x| x["id"].is_string()).collect();
        self.render();
    }

    fn overflow(&self) -> Vec<Value> {
        if !self.supported.get() {
            return Vec::new();
        }
        arrange(&self.shell, &self.items.borrow()).2
    }

    fn render(&self) {
        self.render_with(true);
    }

    fn render_with(&self, may_close: bool) {
        util::clear(&self.grid);
        let overflow = self.overflow();
        let open = self.shell.flyouts().is_open("tray");
        if overflow.is_empty() {
            if open && may_close {
                self.shell.flyouts().close();
            }
            return;
        }
        let cols = overflow.len().clamp(1, MAX_COLS);
        for (i, x) in overflow.iter().enumerate() {
            let cell = self.cell(x);
            self.grid.attach(&cell, (i % cols) as i32, (i / cols) as i32, 1, 1);
        }
        if open {
            self.place();
        }
    }

    fn cell(&self, x: &Value) -> gtk::Button {
        let id = util::str_of(x, "id");
        let button = gtk::Button::new();
        button.add_css_class("w11-tray-cell");
        button.set_focus_on_click(false);
        button.set_widget_name(&format!("menus-tray-{}", util::str_of(x, "key")));
        button.set_size_request(40, 40);
        let attention = x["status"].as_str() == Some("attention");
        if attention {
            button.add_css_class("is-attention");
        }
        let src = [if attention { util::str_of(x, "attentionIcon") } else { String::new() }, util::str_of(x, "icon"), util::str_of(x, "appIcon")]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let overlay = gtk::Overlay::new();
        let image = if src.is_empty() { crate::icons::glyph("app", 16) } else { util::icon(&self.shell, &src, 16) };
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        overlay.set_child(Some(&image));
        if attention && x["attentionIcon"].as_str().is_none_or(|s| s.is_empty()) {
            let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            dot.add_css_class("w11-tray-dot");
            dot.set_size_request(6, 6);
            dot.set_halign(gtk::Align::End);
            dot.set_valign(gtk::Align::Start);
            overlay.add_overlay(&dot);
        }
        overlay.set_size_request(16, 16);
        overlay.set_halign(gtk::Align::Center);
        overlay.set_valign(gtk::Align::Center);
        button.set_child(Some(&overlay));
        let tip = tooltip_of(x);
        if !tip.is_empty() {
            button.set_tooltip_text(Some(&tip));
        }

        let item = x.clone();
        button.connect_clicked(move |b| {
            let (px, py) = center_of(b);
            click(&crate::core::shell(), &item, px, py, true);
        });
        let middle = gtk::GestureClick::new();
        middle.set_button(2);
        let sid = id.clone();
        middle.connect_released(move |g, _, _, _| {
            if let Some(w) = g.widget() {
                let (px, py) = center_of(&w);
                crate::core::shell().fire("tray.secondaryActivate", json!({"id": sid, "x": px, "y": py}));
            }
        });
        button.add_controller(middle);
        let right = gtk::GestureClick::new();
        right.set_button(3);
        let item = x.clone();
        right.connect_released(move |g, _, _, _| {
            if let Some(w) = g.widget() {
                let (px, py) = center_of(&w);
                item_menu(&crate::core::shell(), &item, px, py);
            }
        });
        button.add_controller(right);
        let keys = gtk::EventControllerKey::new();
        let item = x.clone();
        keys.connect_key_pressed(move |c, key, _, state| {
            let menu_key = key == gtk::gdk::Key::Menu
                || (key == gtk::gdk::Key::F10 && state.contains(gtk::gdk::ModifierType::SHIFT_MASK));
            if menu_key && let Some(w) = c.widget() {
                let (px, py) = center_of(&w);
                item_menu(&crate::core::shell(), &item, px, py);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        button.add_controller(keys);
        button
    }

    /// Centre the overflow over the chevron (it grows with its icons), on screen.
    fn place(&self) {
        let Some(x) = self.anchor.get() else { return };
        let (_, natural, _, _) = self.root.measure(gtk::Orientation::Horizontal, -1);
        let monitor = util::monitor_width(&self.root);
        let left = (x - natural / 2).min(monitor - natural - 12).max(12);
        self.shell.flyouts().set_placement_x("tray", left);
    }
}

/// A widget's centre on the monitor (the panel window covers it from 0, 0).
fn center_of(widget: &impl IsA<gtk::Widget>) -> (i32, i32) {
    let w = widget.as_ref();
    let Some(root) = w.root() else { return (0, 0) };
    match w.compute_bounds(root.upcast_ref::<gtk::Widget>()) {
        Some(b) => ((b.x() + b.width() / 2.0) as i32, (b.y() + b.height() / 2.0) as i32),
        None => (0, 0),
    }
}

/// A left click on a tray icon (taskbar or overflow): ItemIsMenu opens the menu; otherwise the overflow closes
/// first (the app's window must get the focus), then Activate, and a fallback:'menu' answer opens the menu.
pub fn click(shell: &Rc<Shell>, item: &Value, x: i32, y: i32, _from_overflow: bool) {
    if item["itemIsMenu"] == true {
        item_menu(shell, item, x, y);
        return;
    }
    if shell.flyouts().current().is_some() {
        shell.flyouts().close();
    }
    let id = util::str_of(item, "id");
    let again = item.clone();
    shell.call("tray.activate", json!({"id": id, "x": x, "y": y}), move |r| match r {
        Ok(v) if v["fallback"] == "menu" => item_menu(&crate::core::shell(), &again, x, y),
        Ok(_) => {}
        Err(error) => eprintln!("win11-shell: tray.activate: {error}"),
    });
}

/// The menu of a tray icon at (x, y): its dbusmenu in our menu flyout, or the app's own ContextMenu.
pub fn item_menu(shell: &Rc<Shell>, item: &Value, x: i32, y: i32) {
    let id = util::str_of(item, "id");
    if item["hasMenu"] != true {
        shell.fire("tray.contextMenu", json!({"id": id, "x": x, "y": y}));
        return;
    }
    shell.call("tray.menu", json!({"id": id}), move |r| {
        let shell = crate::core::shell();
        let list = r.ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default();
        if !has_entries(&list) {
            shell.fire("tray.contextMenu", json!({"id": id, "x": x, "y": y}));
            return;
        }
        show_menu(&shell, &id, x, y, list);
    });
}

fn has_entries(list: &[Value]) -> bool {
    list.iter().any(|mi| mi.is_object() && mi["type"].as_str() != Some("separator"))
}

fn show_menu(shell: &Rc<Shell>, id: &str, x: i32, y: i32, list: Vec<Value>) {
    let clicked = Rc::new(Cell::new(false));
    let items = map_menu(&list, id, x, y, &clicked);
    let state = OpenMenu {
        id: id.to_owned(),
        x,
        y,
        json: serde_json::to_string(&list).unwrap_or_default(),
        clicked: clicked.clone(),
        busy: false,
        again: false,
    };
    let sid = id.to_owned();
    let on_close: Box<dyn FnOnce()> = Box::new(move || {
        MENU.with(|m| {
            let mut m = m.borrow_mut();
            if m.as_ref().is_some_and(|m| Rc::ptr_eq(&m.clicked, &clicked)) {
                *m = None;
            }
        });
        // a click sends 'clicked' first and 'closed' after it (tray.js menuClosed)
        let clicked = clicked.clone();
        glib::idle_add_local_once(move || {
            if !clicked.get() {
                menu_event(&crate::core::shell(), &sid, json!(0), "closed", |_| {});
            }
        });
    });
    menu::show(shell, items, x, false, Some(on_close));
    MENU.with(|m| *m.borrow_mut() = Some(state));
    menu_event(shell, id, json!(0), "opened", |_| {});
}

fn menu_event(shell: &Shell, id: &str, item_id: Value, event: &str, done: impl FnOnce(Option<Value>) + 'static) {
    shell.call("tray.menuEvent", json!({"id": id, "itemId": item_id, "event": event}), move |r| done(r.ok()));
}

/// MenuItem[] (SPEC 6.15) as menu items.
fn map_menu(list: &[Value], id: &str, x: i32, y: i32, clicked: &Rc<Cell<bool>>) -> Vec<Item> {
    let shell = crate::core::shell();
    let mut out: Vec<Item> = Vec::new();
    for mi in list {
        if !mi.is_object() || mi["visible"] == false {
            continue;
        }
        if mi["type"].as_str() == Some("separator") {
            if out.last().is_some_and(|l| !l.separator) {
                out.push(Item::sep());
            }
            continue;
        }
        let mut entry = Item::plain(mi["label"].as_str().unwrap_or(""));
        entry.disabled = mi["enabled"] == false;
        if let Some(s) = mi["shortcut"].as_str().filter(|s| !s.is_empty()) {
            entry.shortcut = Some(s.to_owned());
        }
        if let Some(icon) = mi["icon"].as_str().filter(|s| !s.is_empty()) {
            entry.icon = Icon::App(icon.to_owned());
        }
        match mi["toggle"].as_str() {
            Some("checkmark") => entry.check = Some(mi["checked"] == true),
            Some("radio") => {
                entry.radio = true;
                entry.check = Some(mi["checked"] == true);
            }
            _ => {}
        }
        let kids = mi["children"].as_array().cloned().unwrap_or_default();
        let sub = if kids.is_empty() { Vec::new() } else { map_menu(&kids, id, x, y, clicked) };
        if !sub.is_empty() {
            entry.submenu = Some(sub);
        } else if mi["submenu"] == true {
            let mut empty = Item::plain(shell.t("(Leeg)", "(Empty)"));
            empty.disabled = true;
            entry.submenu = Some(vec![empty]);
        } else {
            let sid = id.to_owned();
            let item_id = mi["id"].clone();
            let clicked = clicked.clone();
            entry.action = Some(Rc::new(move || {
                clicked.set(true);
                let shell = crate::core::shell();
                let sid2 = sid.clone();
                menu_event(&shell, &sid, item_id.clone(), "clicked", move |r| {
                    let shell = crate::core::shell();
                    // {ok:false}: the app rebuilt its menu meanwhile; show the fresh one at the same place
                    let stale = r.as_ref().is_some_and(|v| v["ok"] == false);
                    menu_event(&shell, &sid2, json!(0), "closed", |_| {});
                    if stale {
                        let item = TRAY.with(|t| {
                            t.borrow().as_ref().and_then(|t| t.items.borrow().iter().find(|i| i["id"].as_str() == Some(&sid2)).cloned())
                        });
                        if let Some(item) = item {
                            item_menu(&shell, &item, x, y);
                        }
                    }
                });
            }));
        }
        out.push(entry);
    }
    while out.last().is_some_and(|l| l.separator) {
        out.pop();
    }
    out
}

/// tray.menuChanged for the open menu: fetch it again (GetLayout only) and show the new layout in place; a change
/// during a fetch makes one more fetch after it.
fn refresh_menu() {
    let id = MENU.with(|m| {
        let mut m = m.borrow_mut();
        let st = m.as_mut()?;
        if st.busy {
            st.again = true;
            return None;
        }
        st.busy = true;
        st.again = false;
        Some(st.id.clone())
    });
    let Some(id) = id else { return };
    let shell = crate::core::shell();
    shell.call("tray.menu", json!({"id": id, "refresh": true}), move |r| {
        let list = r.ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default();
        let json = serde_json::to_string(&list).unwrap_or_default();
        let next = MENU.with(|m| {
            let mut m = m.borrow_mut();
            let st = m.as_mut().filter(|st| st.id == id)?;
            st.busy = false;
            let changed = json != st.json && has_entries(&list);
            if changed {
                st.json = json.clone();
            }
            Some((changed, st.again, st.x, st.y, st.clicked.clone()))
        });
        let Some((changed, again, x, y, clicked)) = next else { return };
        if changed {
            menu::update(map_menu(&list, &id, x, y, &clicked));
        }
        if again {
            refresh_menu();
        }
    });
}
