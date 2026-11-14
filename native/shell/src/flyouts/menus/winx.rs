//! The Win+X quick link menu (`git show 418ea594:vendor/ui/js/shell/winx.js`): a plain text list without icons, groups
//! split by lines, access keys (underlined when opened from the keyboard), and the "Afsluiten of afmelden" submenu to
//! the right. Keyboard: arrows, Home/End, Enter, the access keys, Right for the submenu (Left or Esc closes it).
use super::menu::{self, Icon, Item};
use super::util;
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use gtk::glib;
use gtk::prelude::*;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Id {
    Apps,
    System,
    Network,
    DiskMgmt,
    Disk,
    Terminal,
    TaskMgr,
    Settings,
    Explorer,
    Alles,
    Search,
    Run,
    Power,
    Desktop,
}

struct Entry {
    id: Id,
    key: char,
    name: &'static str,
}

const fn e(id: Id, key: char, name: &'static str) -> Option<Entry> {
    Some(Entry { id, key, name })
}

/// The list with its groups (None = a line), as winx.js ITEMS.
const ITEMS: &[Option<Entry>] = &[
    e(Id::Apps, 'f', "apps"),
    e(Id::System, 'y', "system"),
    e(Id::Network, 'w', "network"),
    e(Id::DiskMgmt, 'b', "diskmgmt"),
    e(Id::Disk, 'k', "disk"),
    None,
    e(Id::Terminal, 'i', "terminal"),
    None,
    e(Id::TaskMgr, 't', "taskmgr"),
    e(Id::Settings, 'n', "settings"),
    e(Id::Explorer, 'e', "explorer"),
    e(Id::Alles, 'a', "alles"),
    e(Id::Search, 's', "search"),
    e(Id::Run, 'r', "run"),
    None,
    e(Id::Power, 'u', "power"),
    e(Id::Desktop, 'd', "desktop"),
];

fn text(shell: &Shell, id: Id) -> String {
    match id {
        Id::Apps => shell.t("Geïnstalleerde apps", "Installed apps"),
        Id::System => shell.t("Systeem", "System"),
        Id::Network => shell.t("Netwerkverbindingen", "Network connections"),
        Id::DiskMgmt => shell.t("Schijfbeheer", "Disk Management"),
        Id::Disk => shell.t("Schijfruimte", "Disk space"),
        Id::Terminal => shell.t("Terminal", "Terminal"),
        Id::TaskMgr => shell.t("Taakbeheer", "Task Manager"),
        Id::Settings => shell.t("Instellingen", "Settings"),
        Id::Explorer => shell.t("Verkenner", "File Explorer"),
        Id::Alles => shell.t("Alles", "Alles"),
        Id::Search => shell.t("Zoeken", "Search"),
        Id::Run => shell.t("Uitvoeren", "Run"),
        Id::Power => shell.t("Afsluiten of afmelden", "Shut down or sign out"),
        Id::Desktop => shell.t("Bureaublad", "Desktop"),
    }
}

/// The label with its access key underlined (the first occurrence of the key, any case).
fn markup(label: &str, key: char, show: bool) -> String {
    let at = label.char_indices().find(|(_, c)| c.to_lowercase().next() == Some(key));
    match at {
        Some((i, c)) if show => {
            let (head, rest) = label.split_at(i);
            let tail = &rest[c.len_utf8()..];
            format!(
                "{}<u>{}</u>{}",
                glib::markup_escape_text(head),
                glib::markup_escape_text(&c.to_string()),
                glib::markup_escape_text(tail)
            )
        }
        _ => glib::markup_escape_text(label).to_string(),
    }
}

pub struct WinX {
    shell: Rc<Shell>,
    root: gtk::Box,
    menu: gtk::Box,
    sub: gtk::Box,
    rows: RefCell<Vec<(gtk::Button, Id)>>,
    sub_rows: RefCell<Vec<gtk::Button>>,
    sub_open: Cell<bool>,
    show_accel: Cell<bool>,
    /// Hover timers: only the latest one acts.
    hover_seq: Cell<u64>,
}

thread_local! {
    static WINX: RefCell<Option<Rc<WinX>>> = const { RefCell::new(None) };
}

struct Handle(Rc<WinX>);

impl Flyout for Handle {
    fn name(&self) -> &'static str {
        "winx"
    }
    fn widget(&self) -> gtk::Widget {
        self.0.root.clone().upcast()
    }
    fn placement(&self) -> Placement {
        Placement::BottomLeft
    }
    fn opened(&self) {
        let me = self.0.clone();
        me.build();
        me.close_sub();
        glib::idle_add_local_once(move || {
            me.menu.grab_focus();
        });
    }
    fn closed(&self) {
        self.0.hover_seq.set(self.0.hover_seq.get() + 1);
        self.0.close_sub();
        // the next open is from the keyboard unless open_winx says otherwise
        self.0.show_accel.set(true);
    }
}

pub fn register(shell: &Rc<Shell>) {
    let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    root.add_css_class("w11-winx-wrap");
    root.set_valign(gtk::Align::End);
    let menu_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    menu_box.add_css_class("w11-mn");
    menu_box.add_css_class("w11-winx");
    menu_box.set_widget_name("menus-winx");
    menu_box.set_focusable(true);
    menu_box.set_valign(gtk::Align::End);
    let sub = gtk::Box::new(gtk::Orientation::Vertical, 0);
    sub.add_css_class("w11-mn");
    sub.add_css_class("w11-winx-sub");
    sub.set_widget_name("menus-winx-power");
    sub.set_valign(gtk::Align::End);
    sub.set_visible(false);
    root.append(&menu_box);
    root.append(&sub);
    let winx = Rc::new(WinX {
        shell: shell.clone(),
        root: root.clone(),
        menu: menu_box.clone(),
        sub: sub.clone(),
        rows: RefCell::new(Vec::new()),
        sub_rows: RefCell::new(Vec::new()),
        sub_open: Cell::new(false),
        show_accel: Cell::new(true),
        hover_seq: Cell::new(0),
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(&winx);
    keys.connect_key_pressed(move |_, key, _, state| {
        let Some(w) = weak.upgrade() else { return glib::Propagation::Proceed };
        if w.key(key, state) { glib::Propagation::Stop } else { glib::Propagation::Proceed }
    });
    root.add_controller(keys);
    // leaving the menu clears the hover (unless the submenu is open)
    let motion = gtk::EventControllerMotion::new();
    let weak = Rc::downgrade(&winx);
    motion.connect_leave(move |_| {
        if let Some(w) = weak.upgrade() {
            w.hover_seq.set(w.hover_seq.get() + 1);
        }
    });
    menu_box.add_controller(motion);

    winx.build();
    shell.flyouts().register(Rc::new(Handle(winx.clone())));
    WINX.with(|w| *w.borrow_mut() = Some(winx));
}

/// Open Win+X with its left edge at `x` (the Start button's left), from the mouse (no access key underlines)
/// or the keyboard; a second call closes it.
pub fn open(shell: &Rc<Shell>, x: Option<i32>, mouse: bool) {
    let host = shell.flyouts();
    if host.is_open("winx") {
        host.close();
        return;
    }
    if let Some(w) = WINX.with(|w| w.borrow().clone()) {
        w.show_accel.set(!mouse);
    }
    if let Some(x) = x {
        host.set_placement_x("winx", x.max(8));
    }
    host.open("winx");
}

impl WinX {
    fn build(self: &Rc<Self>) {
        util::clear(&self.menu);
        let mut rows = Vec::new();
        for entry in ITEMS {
            let Some(entry) = entry else {
                self.menu.append(&menu::separator());
                continue;
            };
            let label = text(&self.shell, entry.id);
            let mut item = Item::plain(label.clone()).named(format!("winx-{}", entry.name));
            if entry.id == Id::Power {
                item.submenu = Some(Vec::new());
            }
            let button = menu::row(&self.shell, &item, false, false, Some(markup(&label, entry.key, self.show_accel.get())));
            let id = entry.id;
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(w) = weak.upgrade() {
                    w.invoke(id, false);
                }
            });
            let motion = gtk::EventControllerMotion::new();
            let weak = Rc::downgrade(self);
            motion.connect_enter(move |_, _, _| {
                if let Some(w) = weak.upgrade() {
                    w.hover(id);
                }
            });
            button.add_controller(motion);
            self.menu.append(&button);
            rows.push((button, id));
        }
        *self.rows.borrow_mut() = rows;
        if self.show_accel.get() {
            self.menu.add_css_class("show-accel");
        } else {
            self.menu.remove_css_class("show-accel");
        }
    }

    fn power_items(&self) -> Vec<Item> {
        let s = &self.shell;
        let power = |action: &'static str| {
            move || {
                let shell = crate::core::shell();
                crate::flyouts::power::confirm(&shell, action);
            }
        };
        vec![
            Item::new(s.t("Afmelden", "Sign out"), Icon::Glyph("signout"), power("logout")).named("winx-logout"),
            Item::new(s.t("Slaapstand", "Sleep"), Icon::Glyph("sleep"), power("suspend")).named("winx-sleep"),
            Item::new(s.t("Afsluiten", "Shut down"), Icon::Glyph("power"), power("shutdown")).named("winx-shutdown"),
            Item::new(s.t("Opnieuw opstarten", "Restart"), Icon::Glyph("restart"), power("reboot")).named("winx-reboot"),
        ]
    }

    fn power_row(&self) -> Option<gtk::Button> {
        self.rows.borrow().iter().find(|(_, id)| *id == Id::Power).map(|(b, _)| b.clone())
    }

    fn open_sub(self: &Rc<Self>, keyboard: bool) {
        if !self.sub_open.get() {
            util::clear(&self.sub);
            let mut rows = Vec::new();
            for item in self.power_items() {
                let button = menu::row(&self.shell, &item, true, false, None);
                let action = item.action.clone();
                button.connect_clicked(move |_| {
                    crate::core::shell().flyouts().close();
                    if let Some(a) = &action {
                        a();
                    }
                });
                self.sub.append(&button);
                rows.push(button);
            }
            *self.sub_rows.borrow_mut() = rows;
            self.place_sub();
            self.sub.set_visible(true);
            self.sub_open.set(true);
            if let Some(row) = self.power_row() {
                row.add_css_class("has-open-submenu");
            }
        }
        if keyboard {
            let first = self.sub_rows.borrow().first().cloned();
            if let Some(b) = first {
                glib::idle_add_local_once(move || {
                    b.grab_focus();
                });
            }
        }
    }

    /// The submenu next to its row, moved up as far as needed to stay above the taskbar.
    fn place_sub(&self) {
        let (Some(row), Some(_)) = (self.power_row(), self.menu.root()) else { return };
        let Some(bounds) = row.compute_bounds(&self.menu) else { return };
        let menu_h = self.menu.height().max(1) as f32;
        let (_, sub_h, _, _) = self.sub.measure(gtk::Orientation::Vertical, -1);
        let top = bounds.y() - 5.0;
        let bottom_gap = menu_h - (top + sub_h as f32);
        self.sub.set_margin_bottom(bottom_gap.max(0.0) as i32);
    }

    fn close_sub(&self) {
        if !self.sub_open.get() {
            return;
        }
        self.sub_open.set(false);
        self.sub.set_visible(false);
        if let Some(row) = self.power_row() {
            row.remove_css_class("has-open-submenu");
        }
    }

    fn hover(self: &Rc<Self>, id: Id) {
        let seq = self.hover_seq.get() + 1;
        self.hover_seq.set(seq);
        let delay = if id == Id::Power { 200 } else { 300 };
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            let Some(w) = weak.upgrade() else { return };
            if w.hover_seq.get() != seq || !w.shell.flyouts().is_open("winx") {
                return;
            }
            if id == Id::Power { w.open_sub(false) } else { w.close_sub() }
        });
    }

    fn invoke(self: &Rc<Self>, id: Id, keyboard: bool) {
        if id == Id::Power {
            self.open_sub(keyboard);
            return;
        }
        let shell = self.shell.clone();
        shell.flyouts().close();
        run(&shell, id);
    }

    fn key(self: &Rc<Self>, key: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> bool {
        use gtk::gdk::Key;
        let in_sub = self.sub_open.get() && self.sub_rows.borrow().iter().any(|b| b.has_focus());
        if in_sub {
            return match key {
                Key::Up | Key::Down | Key::Home | Key::End => menu::move_focus(&self.sub_rows.borrow(), key),
                Key::Left | Key::Escape => {
                    self.close_sub();
                    if let Some(row) = self.power_row() {
                        row.grab_focus();
                    }
                    true
                }
                _ => false,
            };
        }
        let buttons: Vec<gtk::Button> = self.rows.borrow().iter().map(|(b, _)| b.clone()).collect();
        match key {
            Key::Up | Key::Down | Key::Home | Key::End => {
                self.close_sub();
                menu::move_focus(&buttons, key)
            }
            Key::Right => {
                let focused = self.rows.borrow().iter().find(|(b, _)| b.has_focus()).map(|(_, id)| *id);
                if focused == Some(Id::Power) {
                    self.open_sub(true);
                    true
                } else {
                    false
                }
            }
            Key::Return | Key::KP_Enter | Key::space => {
                let focused = self.rows.borrow().iter().find(|(b, _)| b.has_focus()).map(|(_, id)| *id);
                match focused {
                    Some(id) => {
                        self.invoke(id, true);
                        true
                    }
                    None => false,
                }
            }
            _ => {
                if state.intersects(
                    gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::ALT_MASK | gtk::gdk::ModifierType::SUPER_MASK,
                ) {
                    return false;
                }
                let Some(c) = key.to_unicode().and_then(|c| c.to_lowercase().next()) else { return false };
                let hit = ITEMS.iter().flatten().find(|e| e.key == c).map(|e| e.id);
                match hit {
                    Some(id) => {
                        if let Some((b, _)) = self.rows.borrow().iter().find(|(_, x)| *x == id) {
                            b.grab_focus();
                        }
                        self.invoke(id, true);
                        true
                    }
                    None => false,
                }
            }
        }
    }
}

fn run(shell: &Rc<Shell>, id: Id) {
    match id {
        Id::Apps => util::open_settings(shell, "installed"),
        Id::System => util::open_settings(shell, "about"),
        Id::Network => util::open_settings(shell, "network"),
        Id::DiskMgmt => util::open_app(shell, "disks", json!({})),
        Id::Disk => util::open_app(shell, "taskmgr", json!({"page": "storage"})),
        Id::Terminal => open_terminal(shell),
        Id::TaskMgr => util::open_app(shell, "taskmgr", json!({})),
        Id::Settings => util::open_app(shell, "settings", json!({})),
        Id::Explorer => util::open_app(shell, "explorer", json!({})),
        Id::Alles => util::open_app(shell, "alles", json!({})),
        Id::Search | Id::Run => crate::start::open_search(""),
        Id::Power => {}
        Id::Desktop => {
            shell.flyouts().close();
            shell.gnome.send("windows.showDesktop", json!({}));
        }
    }
}

/// The default terminal (apps.defaults), else the first app in the TerminalEmulator category.
fn open_terminal(shell: &Rc<Shell>) {
    shell.call("apps.defaults", json!({}), |r| {
        let shell = crate::core::shell();
        let id = r
            .ok()
            .and_then(|v| v["terminal"]["id"].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
            .or_else(|| {
                util::apps().iter().find_map(|a| {
                    let terminal = a["categories"]
                        .as_array()
                        .is_some_and(|c| c.iter().any(|x| x.as_str() == Some("TerminalEmulator")));
                    if terminal { a["id"].as_str().map(str::to_owned) } else { None }
                })
            });
        match id {
            Some(id) => util::launch(&shell, &id),
            None => eprintln!("win11-shell: {}", shell.t("Er is geen terminal-app gevonden", "No terminal app was found")),
        }
    });
}
