//! The notification area (tray.js): the StatusNotifierItem icons of apps in the background, from the backend's
//! tray.list and tray.changed, and the "^" chevron that opens the overflow flyout "tray". Which icon goes where:
//! settings taskbar.tray {hiddenMenu, shown, order}; Passive items are not shown. Left click: Activate (an
//! ItemIsMenu item or a `fallback: "menu"` answer opens the menu); middle: SecondaryActivate; wheel: Scroll;
//! right click: the app's dbusmenu (tray.menu) as a popover menu, else ContextMenu (the app draws its own).
use super::util;
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

const MAX_COLS: usize = 5;

struct TrayButton {
    button: gtk::Button,
    image: gtk::Image,
    overlay_img: gtk::Image,
    dot: gtk::Box,
    status: RefCell<String>,
}

struct OpenMenu {
    id: String,
    popover: gtk::PopoverMenu,
    json: RefCell<String>,
    clicking: Cell<u32>,
    closed_waiting: Cell<bool>,
    busy: Cell<bool>,
    again: Cell<bool>,
    x: i32,
    y: i32,
}

pub struct Tray {
    shell: Rc<Shell>,
    strip: gtk::Widget,
    pub area: gtk::Box,
    chevron: gtk::Button,
    list: gtk::Box,
    supported: Cell<bool>,
    items: RefCell<Vec<Value>>,
    changed_seq: Cell<u64>,
    buttons: RefCell<HashMap<String, Rc<TrayButton>>>,
    overflow_count: Cell<usize>,
    menu: RefCell<Option<Rc<OpenMenu>>>,
    menu_seq: Cell<u64>,
    wheel: Cell<f64>,
}

thread_local! {
    static TRAY: RefCell<Option<Rc<Tray>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Rc<Tray>) -> R) -> Option<R> {
    let t = TRAY.with(|x| x.borrow().clone());
    t.map(|t| f(&t))
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned()
}

fn tooltip_of(x: &Value) -> String {
    let title = {
        let t = s(&x["tooltip"], "title");
        let t = if t.trim().is_empty() { s(x, "title") } else { t };
        let t = if t.trim().is_empty() { s(x, "appName") } else { t };
        t.trim().to_owned()
    };
    let text = s(&x["tooltip"], "text").trim().to_owned();
    if !text.is_empty() && text != title {
        if title.is_empty() { text } else { format!("{title}\n{text}") }
    } else if !title.is_empty() {
        title
    } else {
        s(x, "appName")
    }
}

fn has_entries(list: &[Value]) -> bool {
    list.iter().any(|m| m.is_object() && m["type"] != "separator")
}

pub fn build(shell: &Rc<Shell>, strip: &gtk::Widget) -> gtk::Box {
    let area = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    area.add_css_class("w11-ni-area");
    area.set_valign(gtk::Align::Center);
    area.set_visible(false);
    let chevron = gtk::Button::new();
    chevron.add_css_class("w11-tb-btn");
    chevron.add_css_class("w11-ni-chevron");
    chevron.add_css_class("w11-tb-hit");
    chevron.set_widget_name("tb-tray-chevron");
    let chev = crate::icons::glyph("chevron-up", 16);
    chev.add_css_class("w11-ni-chevron-ico");
    chevron.set_child(Some(&chev));
    chevron.set_visible(false);
    chevron.connect_clicked(|b| {
        let b = b.clone();
        with(move |t| t.toggle_overflow(&b));
    });
    let list = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    list.add_css_class("w11-ni-list");
    area.append(&chevron);
    area.append(&list);

    let tray = Rc::new(Tray {
        shell: shell.clone(),
        strip: strip.clone(),
        area: area.clone(),
        chevron,
        list,
        supported: Cell::new(false),
        items: RefCell::new(Vec::new()),
        changed_seq: Cell::new(0),
        buttons: RefCell::new(HashMap::new()),
        overflow_count: Cell::new(0),
        menu: RefCell::new(None),
        menu_seq: Cell::new(0),
        wheel: Cell::new(0.0),
    });
    TRAY.with(|x| *x.borrow_mut() = Some(tray.clone()));
    tray.update_texts();

    shell.on("tray.changed", |_, d| {
        with(|t| {
            t.changed_seq.set(t.changed_seq.get() + 1);
            t.supported.set(true);
            t.set_items(d["items"].as_array().cloned().unwrap_or_default());
        });
    });
    shell.on("tray.status", |_, d| {
        with(|t| {
            let was = t.supported.get();
            let now = d["supported"] == true;
            t.supported.set(now);
            if now && !was {
                load();
            } else if !now {
                t.set_items(Vec::new());
            }
        });
    });
    shell.on("tray.menuChanged", |_, d| {
        let id = s(d, "id");
        with(|t| {
            let open = t.menu.borrow().clone();
            if let Some(m) = open.filter(|m| m.id == id) {
                t.refresh_menu(&m);
            }
        });
    });
    load();
    area
}

fn load() {
    let Some((shell, seq)) = with(|t| (t.shell.clone(), t.changed_seq.get())) else { return };
    shell.call("tray.list", json!({}), move |r| {
        with(|t| match r {
            Ok(v) => {
                let supported = v["supported"] == true;
                t.supported.set(supported);
                // a tray.changed that came in meanwhile is newer than this snapshot
                if !supported || t.changed_seq.get() == seq {
                    t.set_items(if supported { v["items"].as_array().cloned().unwrap_or_default() } else { Vec::new() });
                } else {
                    t.render();
                }
            }
            Err(_) => {
                t.supported.set(false);
                t.set_items(Vec::new());
            }
        });
    });
}

/// Settings (`data`, core::touches) or the language changed: the texts by the language, the icons by where they go
/// (hiddenMenu, shown, order). taskbar.tray.known, which the backend writes for every new icon, is none of these.
pub fn settings_changed(data: &Value, lang_changed: bool) {
    with(|t| {
        if lang_changed {
            t.update_texts();
        }
        if lang_changed
            || crate::core::touches(data, &["taskbar.tray.hiddenMenu", "taskbar.tray.shown", "taskbar.tray.order"])
        {
            t.render();
        }
    });
}

/// The tray overflow flyout opened or closed: the chevron turns over.
pub fn sync_active(open: bool) {
    with(|t| util::set_class(&t.chevron, "is-active", open));
}

impl Tray {
    fn update_texts(&self) {
        let tip = self.shell.t("Verborgen pictogrammen weergeven", "Show hidden icons");
        self.chevron.set_tooltip_text(Some(&tip));
        let items = self.items.borrow().clone();
        for x in &items {
            if let Some(b) = self.buttons.borrow().get(&s(x, "id")) {
                self.update_button(b, x);
            }
        }
    }

    fn set_items(&self, list: Vec<Value>) {
        *self.items.borrow_mut() = list.into_iter().filter(|x| x.get("id").is_some_and(|i| !i.is_null())).collect();
        self.render();
    }

    /// (hiddenMenu, on the taskbar, in the overflow)
    fn arrange(&self) -> (bool, Vec<Value>, Vec<Value>) {
        let p = self.shell.setting("taskbar.tray");
        let strings = |v: &Value| -> Vec<String> {
            v.as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default()
        };
        let hidden_menu = p["hiddenMenu"] != false;
        let shown = strings(&p["shown"]);
        let order = strings(&p["order"]);
        let mut sorted: Vec<(usize, usize, Value)> = self
            .items
            .borrow()
            .iter()
            .filter(|x| x["status"] != "passive")
            .enumerate()
            .map(|(i, x)| {
                let key = s(x, "key");
                let r = order.iter().position(|k| k == &key).unwrap_or(order.len() + i);
                (r, i, x.clone())
            })
            .collect();
        sorted.sort_by_key(|(r, i, _)| (*r, *i));
        let sorted: Vec<Value> = sorted.into_iter().map(|(_, _, x)| x).collect();
        if !hidden_menu {
            return (false, sorted, Vec::new());
        }
        let (bar, overflow): (Vec<Value>, Vec<Value>) = sorted.into_iter().partition(|x| shown.contains(&s(x, "key")));
        (true, bar, overflow)
    }

    fn render(&self) {
        let supported = self.supported.get();
        let (hidden_menu, bar, overflow) = if supported { self.arrange() } else { (true, Vec::new(), Vec::new()) };
        self.area.set_visible(supported);
        let want_chevron = supported && hidden_menu && !overflow.is_empty();
        self.overflow_count.set(overflow.len());
        // reconcile the taskbar's icons
        let wanted: Vec<String> = bar.iter().map(|x| s(x, "id")).collect();
        let stale: Vec<String> = self.buttons.borrow().keys().filter(|k| !wanted.contains(k)).cloned().collect();
        for k in stale {
            let b = self.buttons.borrow_mut().remove(&k);
            if let Some(b) = b {
                self.list.remove(&b.button);
            }
        }
        let mut prev: Option<gtk::Widget> = None;
        for x in &bar {
            let id = s(x, "id");
            let existing = self.buttons.borrow().get(&id).cloned();
            let b = match existing {
                Some(b) => b,
                None => {
                    let b = self.make_button(&id);
                    self.list.append(&b.button);
                    util::flash_class(&b.button, "is-entering", 260);
                    self.buttons.borrow_mut().insert(id.clone(), b.clone());
                    b
                }
            };
            self.update_button(&b, x);
            self.list.reorder_child_after(&b.button, prev.as_ref());
            prev = Some(b.button.clone().upcast());
        }
        self.chevron.set_visible(want_chevron);
        let flyouts = self.shell.flyouts.borrow().clone();
        if let Some(f) = flyouts
            && f.is_open("tray")
            && !want_chevron
        {
            f.close();
        }
        let gone = self.menu.borrow().as_ref().is_some_and(|m| !self.items.borrow().iter().any(|x| s(x, "id") == m.id));
        if gone {
            let m = self.menu.borrow_mut().take();
            if let Some(m) = m {
                m.popover.popdown();
            }
        }
        super::tray_resized();
    }

    fn item(&self, id: &str) -> Option<Value> {
        self.items.borrow().iter().find(|x| s(x, "id") == id).cloned()
    }

    fn make_button(&self, id: &str) -> Rc<TrayButton> {
        let button = gtk::Button::new();
        button.add_css_class("w11-tb-btn");
        button.add_css_class("w11-ni-item");
        button.add_css_class("w11-tb-hit");
        let overlay = gtk::Overlay::new();
        overlay.add_css_class("w11-ni-ico");
        let image = gtk::Image::new();
        image.set_pixel_size(16);
        image.add_css_class("w11-ni-img");
        overlay.set_child(Some(&image));
        let overlay_img = gtk::Image::new();
        overlay_img.set_pixel_size(10);
        overlay_img.set_halign(gtk::Align::End);
        overlay_img.set_valign(gtk::Align::End);
        overlay_img.add_css_class("w11-ni-overlay");
        overlay_img.set_visible(false);
        overlay_img.set_can_target(false);
        overlay.add_overlay(&overlay_img);
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("w11-ni-dot");
        dot.set_halign(gtk::Align::End);
        dot.set_valign(gtk::Align::Start);
        dot.set_visible(false);
        dot.set_can_target(false);
        overlay.add_overlay(&dot);
        button.set_child(Some(&overlay));

        let k = id.to_owned();
        button.connect_clicked(move |b| {
            let b = b.clone();
            let k = k.clone();
            with(move |t| t.on_click(&k, &b));
        });
        let middle = gtk::GestureClick::new();
        middle.set_button(gdk::BUTTON_MIDDLE);
        let k = id.to_owned();
        middle.connect_released(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(w) = g.widget() else { return };
            let (px, py) = util::window_point(&w, x, y);
            with(|t| t.shell.fire("tray.secondaryActivate", json!({"id": k, "x": px, "y": py})));
        });
        button.add_controller(middle);
        let right = gtk::GestureClick::new();
        right.set_button(gdk::BUTTON_SECONDARY);
        let k = id.to_owned();
        right.connect_pressed(move |g, _, x, y| {
            g.set_state(gtk::EventSequenceState::Claimed);
            let Some(w) = g.widget() else { return };
            let (px, py) = util::window_point(&w, x, y);
            let k = k.clone();
            with(move |t| {
                if let Some(f) = t.shell.flyouts.borrow().clone()
                    && f.is_open("tray")
                {
                    f.close();
                }
                if let Some(item) = t.item(&k) {
                    t.open_item_menu(&item, px, py);
                }
            });
        });
        button.add_controller(right);
        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
        let k = id.to_owned();
        scroll.connect_scroll(move |c, dx, dy| {
            let shift = c.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
            let horizontal = shift || dx.abs() > dy.abs();
            let raw = if dx.abs() > dy.abs() { dx } else { dy };
            with(|t| {
                let acc = t.wheel.get() + raw;
                let notches = acc.trunc();
                t.wheel.set(acc - notches);
                if notches != 0.0 {
                    t.shell.fire(
                        "tray.scroll",
                        json!({"id": k, "delta": (-notches * 120.0) as i64,
                            "orientation": if horizontal { "horizontal" } else { "vertical" }}),
                    );
                }
            });
            glib::Propagation::Stop
        });
        button.add_controller(scroll);

        Rc::new(TrayButton { button, image, overlay_img, dot, status: RefCell::new(String::new()) })
    }

    fn update_button(&self, b: &TrayButton, x: &Value) {
        let key = s(x, "key");
        b.button.set_widget_name(&format!("tb-tray-{}", if key.is_empty() { s(x, "id") } else { key }));
        let tip = tooltip_of(x);
        let att = x["status"] == "attention";
        b.button.set_tooltip_text(Some(&tip));
        util::set_class(&b.button, "is-attention", att);
        let attention_icon = s(x, "attentionIcon");
        let src = [if att { attention_icon.clone() } else { String::new() }, s(x, "icon"), s(x, "appIcon")]
            .into_iter()
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        if src.is_empty() {
            b.image.set_icon_name(Some("application-x-executable"));
        } else {
            util::set_icon(&self.shell, &b.image, &src, 16, false);
        }
        let overlay = s(x, "overlayIcon");
        b.overlay_img.set_visible(!overlay.is_empty());
        if !overlay.is_empty() {
            util::set_icon(&self.shell, &b.overlay_img, &overlay, 10, false);
        }
        b.dot.set_visible(att && attention_icon.is_empty());
        let was = b.status.replace(s(x, "status"));
        if att && !was.is_empty() && was != "attention" {
            util::flash_class(&b.image, "is-pulse", 620);
        }
    }

    fn toggle_overflow(&self, chevron: &gtk::Button) {
        let Some(f) = self.shell.flyouts.borrow().clone() else { return };
        if f.is_open("tray") {
            f.close();
            return;
        }
        super::before_flyout();
        // centred above the chevron: 40 px cells, at most 5 per row, 4 px padding
        let cols = self.overflow_count.get().clamp(1, MAX_COLS) as i32;
        let width = cols * 40 + 10;
        let x = util::window_point(chevron, chevron.width() as f64 / 2.0, 0.0).0;
        f.set_placement_x("tray", x - width / 2);
        f.open("tray");
    }

    fn on_click(&self, id: &str, button: &gtk::Button) {
        let Some(x) = self.item(id) else { return };
        let (px, py) = util::window_point(button, button.width() as f64 / 2.0, button.height() as f64 / 2.0);
        if x["itemIsMenu"] == true {
            if let Some(f) = self.shell.flyouts.borrow().clone()
                && f.is_open("tray")
            {
                f.close();
            }
            self.open_item_menu(&x, px, py);
            return;
        }
        if let Some(f) = self.shell.flyouts.borrow().clone()
            && f.is_open("tray")
        {
            f.close();
        }
        let id2 = id.to_owned();
        self.shell.call("tray.activate", json!({"id": id, "x": px, "y": py}), move |r| match r {
            Ok(v) if v["fallback"] == "menu" => {
                with(|t| {
                    if let Some(item) = t.item(&id2) {
                        t.open_item_menu(&item, px, py);
                    }
                });
            }
            Ok(_) => {}
            Err(e) => eprintln!("win11-shell: tray.activate: {e}"),
        });
    }

    /// The app's dbusmenu in our menu (opened at the pointer, upward above the taskbar), else its own menu.
    fn open_item_menu(&self, x: &Value, px: i32, py: i32) {
        let seq = self.menu_seq.get() + 1;
        self.menu_seq.set(seq);
        let id = s(x, "id");
        if x["hasMenu"] != true {
            self.shell.fire("tray.contextMenu", json!({"id": id, "x": px, "y": py}));
            return;
        }
        if let Some(b) = self.buttons.borrow().get(&id) {
            b.button.add_css_class("is-menu");
        }
        let id2 = id.clone();
        self.shell.call("tray.menu", json!({"id": id}), move |r| {
            with(|t| {
                if let Some(b) = t.buttons.borrow().get(&id2) {
                    b.button.remove_css_class("is-menu");
                }
                if t.menu_seq.get() != seq {
                    return; // the user asked for something else meanwhile
                }
                if t.shell.flyouts.borrow().as_ref().is_some_and(|f| f.current().is_some()) {
                    return;
                }
                let list = r.ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default();
                if !has_entries(&list) {
                    t.shell.fire("tray.contextMenu", json!({"id": id2, "x": px, "y": py}));
                    return;
                }
                t.show_menu(&id2, &list, px, py);
            });
        });
    }

    fn show_menu(&self, id: &str, list: &[Value], px: i32, py: i32) {
        let old = self.menu.borrow_mut().take();
        if let Some(old) = old {
            old.popover.popdown();
        }
        let popover = gtk::PopoverMenu::from_model(None::<&gio::MenuModel>);
        popover.add_css_class("w11-tb-menu");
        let state = Rc::new(OpenMenu {
            id: id.to_owned(),
            popover: popover.clone(),
            json: RefCell::new(serde_json::to_string(list).unwrap_or_default()),
            clicking: Cell::new(0),
            closed_waiting: Cell::new(false),
            busy: Cell::new(false),
            again: Cell::new(false),
            x: px,
            y: py,
        });
        self.apply_model(&state, list);
        if let Some(b) = self.buttons.borrow().get(id) {
            b.button.add_css_class("is-menu");
        }
        let weak = Rc::downgrade(&state);
        popover.connect_closed(move |_| {
            let Some(st) = weak.upgrade() else { return };
            with(|t| {
                if let Some(b) = t.buttons.borrow().get(&st.id) {
                    b.button.remove_css_class("is-menu");
                }
                let mine = t.menu.borrow().as_ref().is_some_and(|m| Rc::ptr_eq(m, &st));
                if mine {
                    *t.menu.borrow_mut() = None;
                }
            });
            // 'clicked' goes first, then 'closed' (the action runs right after the menu closes)
            let st2 = st.clone();
            glib::idle_add_local_once(move || {
                if st2.clicking.get() > 0 {
                    st2.closed_waiting.set(true);
                } else {
                    menu_event(&st2.id, 0, "closed");
                }
            });
        });
        *self.menu.borrow_mut() = Some(state.clone());
        // Win11: the menu opens at the pointer, upward, its bottom just above the taskbar
        let x = self
            .strip
            .root()
            .and_then(|r| r.compute_point(&self.strip, &gtk::graphene::Point::new(px as f32, py as f32)))
            .map(|p| p.x().round() as i32)
            .unwrap_or(px);
        util::show_popover(&self.strip, popover.upcast_ref(), x);
        menu_event(id, 0, "opened");
    }

    /// MenuItem[] (SPEC 6.15) -> a menu model with one action per entry.
    fn apply_model(&self, st: &Rc<OpenMenu>, list: &[Value]) {
        let group = gio::SimpleActionGroup::new();
        let empty = self.shell.t("Leeg", "Empty");
        let model = build_menu(st, list, &group, &empty);
        st.popover.insert_action_group("tray", Some(&group));
        st.popover.set_menu_model(Some(&model));
    }

    /// tray.menuChanged for the open menu: fetch it again (refresh: GetLayout only) and show the new layout.
    fn refresh_menu(&self, st: &Rc<OpenMenu>) {
        if st.busy.get() {
            st.again.set(true);
            return;
        }
        st.busy.set(true);
        st.again.set(false);
        let st = st.clone();
        self.shell.call("tray.menu", json!({"id": st.id, "refresh": true}), move |r| {
            st.busy.set(false);
            with(|t| {
                let still = t.menu.borrow().as_ref().is_some_and(|m| Rc::ptr_eq(m, &st));
                if !still {
                    return;
                }
                let list = r.ok().and_then(|v| v["items"].as_array().cloned()).unwrap_or_default();
                let json = serde_json::to_string(&list).unwrap_or_default();
                if json != *st.json.borrow() && has_entries(&list) {
                    *st.json.borrow_mut() = json;
                    t.apply_model(&st, &list);
                }
                if st.again.get() {
                    t.refresh_menu(&st);
                }
            });
        });
    }
}

fn menu_event(id: &str, item: i64, event: &str) {
    let shell = crate::core::shell();
    let ev = event.to_owned();
    shell.call("tray.menuEvent", json!({"id": id, "itemId": item, "event": event}), move |r| {
        if let Err(e) = r
            && !e.contains("not_found")
        {
            eprintln!("win11-shell: tray.menuEvent {ev}: {e}");
        }
    });
}

fn item_id(v: &Value) -> i64 {
    match &v["id"] {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn build_menu(st: &Rc<OpenMenu>, list: &[Value], group: &gio::SimpleActionGroup, empty: &str) -> gio::Menu {
    let menu = gio::Menu::new();
    let mut section = gio::Menu::new();
    for mi in list.iter().filter(|m| m.is_object()) {
        if mi["type"] == "separator" {
            if section.n_items() > 0 {
                menu.append_section(None, &section);
                section = gio::Menu::new();
            }
            continue;
        }
        let label = mi["label"].as_str().unwrap_or("").to_owned();
        let iid = item_id(mi);
        let kids: Vec<Value> = mi["children"].as_array().cloned().unwrap_or_default();
        if !kids.is_empty() || mi["submenu"] == true {
            let sub = if kids.is_empty() {
                let m = gio::Menu::new();
                let name = format!("e{iid}");
                let a = gio::SimpleAction::new(&name, None);
                a.set_enabled(false);
                group.add_action(&a);
                m.append(Some(empty), Some(&format!("tray.{name}")));
                m
            } else {
                build_menu(st, &kids, group, empty)
            };
            section.append_submenu(Some(&label), &sub);
            continue;
        }
        let name = format!("i{}", if iid < 0 { format!("n{}", -iid) } else { iid.to_string() });
        let action = match mi["toggle"].as_str() {
            Some("checkmark") | Some("radio") => gio::SimpleAction::new_stateful(&name, None, &(mi["checked"] == true).to_variant()),
            _ => gio::SimpleAction::new(&name, None),
        };
        action.set_enabled(mi["enabled"] != false);
        let weak = Rc::downgrade(st);
        action.connect_activate(move |_, _| {
            let Some(st) = weak.upgrade() else { return };
            st.clicking.set(st.clicking.get() + 1);
            let st2 = st.clone();
            let shell = crate::core::shell();
            shell.call("tray.menuEvent", json!({"id": st.id, "itemId": iid, "event": "clicked"}), move |r| {
                st2.clicking.set(st2.clicking.get().saturating_sub(1));
                // {ok:false}: the id is stale (the app rebuilt its menu meanwhile): show the fresh menu instead
                let stale = matches!(&r, Ok(v) if v["ok"] == false);
                if st2.closed_waiting.get() && st2.clicking.get() == 0 {
                    st2.closed_waiting.set(false);
                    menu_event(&st2.id, 0, "closed");
                }
                if stale {
                    with(|t| {
                        if t.menu.borrow().is_none()
                            && let Some(item) = t.item(&st2.id)
                        {
                            t.open_item_menu(&item, st2.x, st2.y);
                        }
                    });
                }
            });
        });
        // a toggle's state is the app's: it shows the new state when it announces it
        action.connect_change_state(|_, _| {});
        group.add_action(&action);
        let item = gio::MenuItem::new(Some(&label), Some(&format!("tray.{name}")));
        section.append_item(&item);
    }
    if section.n_items() > 0 {
        menu.append_section(None, &section);
    }
    menu
}
