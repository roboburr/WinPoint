//! The desktop window and its icons (`git show 418ea594:vendor/ui/js/desktop/desktop.js`): a column-major grid from the
//! top left (small, medium, large), saved positions (settings desktop.positions, keyed by file name), labels with a
//! text shadow (two lines, the full name when selected), translucent hover and selection fills, rubber band,
//! Ctrl/Shift multi-select, drag to move (grid snapping), drop on a folder to move into it or on the Recycle Bin
//! to delete, double click or Enter opens, F2 renames inline, Delete trashes with an Undo toast, Ctrl+C/X/V/A/Z,
//! arrows, type-to-select, Ctrl+Shift+2/3/4 and Ctrl+wheel change the icon size.
use super::wallpaper::Wallpaper;
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib, graphene, pango};
use serde_json::{Map, Value, json};
use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub const ORIGIN: f64 = 3.0;
const SPECIAL_ORDER: [&str; 3] = ["thispc", "home", "trash"];
pub const SIZE_ORDER: [&str; 3] = ["small", "medium", "large"];

/// Cell width, cell height and icon size of an icon size (desktop.js SIZES).
pub fn size_dims(size: &str) -> (f64, f64, i32) {
    match size {
        "small" => (76.0, 74.0, 32),
        "large" => (112.0, 148.0, 96),
        _ => (76.0, 100.0, 48),
    }
}

pub fn ext_of(name: &str) -> String {
    if name.starts_with('.') {
        return String::new();
    }
    match name.rfind('.') {
        Some(i) if i + 1 < name.len() && !name[i + 1..].contains('/') => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

pub fn stem_of(name: &str) -> &str {
    match name.rfind('.') {
        Some(i) if i > 0 => &name[..i],
        _ => name,
    }
}

pub fn base_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

pub fn parent_of(path: &str) -> String {
    match path.rfind('/') {
        Some(i) if i > 0 => path[..i].to_owned(),
        _ => "/".to_owned(),
    }
}

/// Natural, case-insensitive order ("Map 2" before "Map 10"), like Intl.Collator {numeric, sensitivity: base}.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let mut n = String::new();
                while let Some(c) = x.peek().copied().filter(char::is_ascii_digit) {
                    n.push(c);
                    x.next();
                }
                let mut m = String::new();
                while let Some(d) = y.peek().copied().filter(char::is_ascii_digit) {
                    m.push(d);
                    y.next();
                }
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                let order = n.len().cmp(&m.len()).then_with(|| n.cmp(m));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct Item {
    pub key: String,
    pub special: Option<&'static str>,
    pub label: String,
    pub icon: String,
    pub mime: String,
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    pub is_desktop_entry: bool,
    pub size: u64,
    pub modified: f64,
}

#[derive(Clone, Copy, Default)]
pub struct Metrics {
    pub cw: f64,
    pub ch: f64,
    pub icon: i32,
    pub cols: i32,
    pub rows: i32,
    pub tw: f64,
}

pub struct Tile {
    pub root: gtk::Box,
    pub image: gtk::Image,
    pub label: gtk::Label,
    pub sig: String,
}

pub enum Gesture {
    Press { key: String, x: f64, y: f64, toggle: bool, was_selected: bool, added: bool },
    Band { x: f64, y: f64, base: HashSet<String>, moved: bool, rects: Option<Vec<(String, graphene::Rect)>> },
    Drag { x: f64, y: f64, keys: Vec<String>, origin: HashMap<String, (f64, f64)>, target: Option<String> },
}

pub struct Undo {
    pub id: u64,
    pub label: String,
    pub run: Rc<dyn Fn(Rc<Desk>)>,
    pub toast: Option<gtk::Widget>,
}

pub struct Rename {
    pub key: String,
    pub entry: gtk::Entry,
    pub old: String,
    pub busy: bool,
    pub tip: Option<gtk::Popover>,
}

#[derive(Default)]
pub struct State {
    pub dir: String,
    pub home: String,
    pub files: Vec<Value>,
    pub trash_count: u64,
    pub trash_size: u64,
    pub clip_paths: Vec<String>,
    pub clip_op: String,
    pub model: Vec<Item>,
    pub placed: HashMap<String, (f64, f64)>,
    pub selected: HashSet<String>,
    pub anchor: Option<String>,
    pub focus: Option<String>,
    pub undo: Vec<Undo>,
    pub undo_seq: u64,
    pub metrics: Metrics,
    pub renaming: Option<Rename>,
    pub gesture: Option<Gesture>,
    pub loaded: bool,
    pub pending_new: Map<String, Value>,
    /// desktop.* values we wrote, until the settings cache has them (settings.set answers by event).
    pub local: HashMap<String, (Value, Instant)>,
    /// What the open context menu is about.
    pub menu_keys: Vec<String>,
    pub menu_point: Option<(f64, f64)>,
    pub type_buf: String,
    pub type_at: Option<Instant>,
    pub kbd: bool,
}

pub struct Desk {
    pub shell: Rc<Shell>,
    pub window: gtk::Window,
    pub layer: gtk::Fixed,
    pub band: gtk::Box,
    pub wall: Rc<Wallpaper>,
    pub toasts: gtk::Box,
    pub actions: gio::SimpleActionGroup,
    pub tiles: RefCell<HashMap<String, Tile>>,
    pub s: RefCell<State>,
    pub icons: RefCell<HashMap<String, Option<gdk::Texture>>>,
    pub dialogs: RefCell<Vec<glib::WeakRef<gtk::Window>>>,
    pub size: Cell<(i32, i32)>,
    pub alive: Cell<bool>,
    pub menu_open: Cell<bool>,
    reload_timer: RefCell<Option<glib::SourceId>>,
    save_timer: RefCell<Option<glib::SourceId>>,
    busy_timer: RefCell<Option<glib::SourceId>>,
}

impl Desk {
    pub fn new(shell: &Rc<Shell>) -> Rc<Desk> {
        let window = gtk::Window::builder().decorated(false).title(shell.t("Bureaublad", "Desktop")).build();
        window.set_widget_name("desktop-window");
        window.add_css_class("w11-desktop");
        crate::wayland::set_app_id(&window, crate::DESKTOP_ID);
        let (w, h) = super::geometry(shell);
        window.set_default_size(w, h);
        // The whole monitor, not GTK's work-area bounds (wayland::keep_size); on_geometry updates the default size.
        let weak = window.downgrade();
        crate::wayland::keep_size(&window, move || weak.upgrade().map(|w| w.default_size()).unwrap_or((0, 0)));

        // The bottom layer only measures the monitor (and is black under a picture that is still loading).
        let sizer = gtk::DrawingArea::new();
        sizer.set_hexpand(true);
        sizer.set_vexpand(true);
        sizer.set_draw_func(|_, cr, _, _| {
            cr.set_source_rgb(0.0, 0.0, 0.0);
            let _ = cr.paint();
        });
        let wall = Wallpaper::new();
        let layer = gtk::Fixed::new();
        layer.add_css_class("dt");
        layer.set_widget_name("desktop-icons");
        layer.set_focusable(true);
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        let band = gtk::Box::new(gtk::Orientation::Vertical, 0);
        band.add_css_class("dt-band");
        band.set_can_target(false);
        band.set_visible(false);
        layer.put(&band, 0.0, 0.0);
        let toasts = gtk::Box::new(gtk::Orientation::Vertical, 8);
        toasts.add_css_class("dt-toast-stack");
        toasts.set_halign(gtk::Align::End);
        toasts.set_valign(gtk::Align::End);
        toasts.set_margin_end(12);
        toasts.set_margin_bottom(crate::panel::BAR + 12);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&sizer));
        overlay.add_overlay(&wall.widget);
        overlay.add_overlay(&layer);
        overlay.add_overlay(&toasts);
        window.set_child(Some(&overlay));

        let actions = gio::SimpleActionGroup::new();
        window.insert_action_group("dt", Some(&actions));
        let desk = Rc::new(Desk {
            shell: shell.clone(),
            window,
            layer,
            band,
            wall,
            toasts,
            actions,
            tiles: RefCell::new(HashMap::new()),
            s: RefCell::new(State { clip_op: "copy".into(), ..Default::default() }),
            icons: RefCell::new(HashMap::new()),
            dialogs: RefCell::new(Vec::new()),
            size: Cell::new((w, h)),
            alive: Cell::new(true),
            menu_open: Cell::new(false),
            reload_timer: RefCell::new(None),
            save_timer: RefCell::new(None),
            busy_timer: RefCell::new(None),
        });
        let weak = Rc::downgrade(&desk);
        sizer.connect_resize(move |_, w, h| {
            if let Some(d) = weak.upgrade() {
                d.on_resize(w, h);
            }
        });
        desk.install_input();
        super::actions::install(&desk);
        desk
    }

    /// First load: the wallpaper decodes in parallel; icons never wait for a large picture.
    pub fn start(self: &Rc<Self>) {
        self.apply_wallpaper(true);
        let desk = self.clone();
        glib::spawn_future_local(async move {
            desk.load_files().await;
            desk.load_trash().await;
            desk.load_clip().await;
            desk.load_home().await;
            desk.s.borrow_mut().loaded = true;
            desk.refresh_view();
        });
    }

    pub fn destroy(&self) {
        self.alive.set(false);
        for timer in [&self.reload_timer, &self.save_timer, &self.busy_timer] {
            if let Some(id) = timer.borrow_mut().take() {
                id.remove();
            }
        }
        for dialog in self.dialogs.borrow_mut().drain(..) {
            if let Some(w) = dialog.upgrade() {
                w.destroy();
            }
        }
        self.window.destroy();
    }

    pub fn t(&self, nl: &str, en: &str) -> String {
        self.shell.t(nl, en)
    }

    fn scale(&self) -> i32 {
        self.window.scale_factor().max(1)
    }

    pub fn apply_wallpaper(self: &Rc<Self>, instant: bool) {
        self.wall.apply(&self.shell, self.size.get(), self.scale(), instant);
    }

    fn on_resize(self: &Rc<Self>, w: i32, h: i32) {
        if w <= 1 || h <= 1 || self.size.get() == (w, h) {
            return;
        }
        self.size.set((w, h));
        self.apply_wallpaper(true);
        if self.s.borrow().loaded {
            self.layout();
            self.render();
        }
    }

    /* ------------------------------------------------------------- settings */

    /// settings.desktop with its defaults and our own writes that are still on their way.
    pub fn dset(&self) -> Map<String, Value> {
        let mut d = Map::new();
        for (k, v) in [
            ("showIcons", json!(true)),
            ("iconSize", json!("medium")),
            ("autoArrange", json!(false)),
            ("alignToGrid", json!(true)),
            ("sortBy", json!("name")),
            ("positions", json!({})),
            ("showThisPC", json!(true)),
            ("showTrash", json!(true)),
            ("showHome", json!(false)),
        ] {
            d.insert(k.into(), v);
        }
        if let Value::Object(m) = self.shell.setting("desktop") {
            for (k, v) in m {
                d.insert(k, v);
            }
        }
        for (k, (v, _)) in self.s.borrow().local.iter() {
            d.insert(k.clone(), v.clone());
        }
        d
    }

    /// Write desktop.<key> (and keep it locally until the settings event brings it back).
    pub fn set_desktop(&self, key: &str, value: Value) {
        self.s.borrow_mut().local.insert(key.to_owned(), (value.clone(), Instant::now()));
        self.shell.set_setting(&format!("desktop.{key}"), value);
    }

    pub fn icon_size(&self) -> String {
        let size = self.dset().get("iconSize").and_then(Value::as_str).unwrap_or("medium").to_owned();
        if SIZE_ORDER.contains(&size.as_str()) { size } else { "medium".into() }
    }

    /* ------------------------------------------------------------- data load */

    pub async fn load_files(&self) {
        match super::call(&self.shell, "files.desktop", json!({})).await {
            Ok(r) => {
                let mut s = self.s.borrow_mut();
                if let Some(dir) = r["dir"].as_str() {
                    s.dir = dir.to_owned();
                }
                s.files = r["items"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter(|f| f["hidden"] != true && !f["name"].as_str().unwrap_or(".").starts_with('.'))
                            .cloned()
                            .collect()
                    })
                    .unwrap_or_default();
            }
            Err(error) => {
                eprintln!("win11-shell: desktop: files.desktop: {error}");
                self.s.borrow_mut().files.clear();
            }
        }
    }

    pub async fn load_trash(&self) {
        let r = super::call(&self.shell, "files.trashInfo", json!({})).await.unwrap_or(Value::Null);
        let mut s = self.s.borrow_mut();
        s.trash_count = r["count"].as_u64().unwrap_or(0);
        s.trash_size = r["size"].as_u64().unwrap_or(0);
    }

    pub async fn load_clip(&self) {
        let r = super::call(&self.shell, "files.clipboardGet", json!({})).await.unwrap_or(Value::Null);
        self.set_clip(&r);
    }

    pub fn set_clip(&self, r: &Value) {
        let mut s = self.s.borrow_mut();
        s.clip_paths = r["paths"]
            .as_array()
            .map(|p| p.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
            .unwrap_or_default();
        s.clip_op = r["op"].as_str().unwrap_or("copy").to_owned();
    }

    async fn load_home(&self) {
        let places = super::call(&self.shell, "files.places", json!({})).await.unwrap_or(Value::Null);
        let home = places
            .as_array()
            .and_then(|p| p.iter().find(|x| x["id"] == "home"))
            .and_then(|x| x["path"].as_str())
            .unwrap_or("")
            .to_owned();
        let mut s = self.s.borrow_mut();
        s.home = if home.is_empty() && !s.dir.is_empty() { parent_of(&s.dir) } else { home };
    }

    /* ----------------------------------------------------------------- model */

    fn label_for(&self, f: &Value) -> String {
        let name = f["name"].as_str().unwrap_or("");
        let display = f["displayName"].as_str().filter(|d| !d.is_empty()).unwrap_or(name);
        if f["shellShortcut"] == "thispc" && f["untrusted"] != true {
            return self.t("Deze pc", "This PC");
        }
        if f["isDesktopEntry"] == true {
            return f["displayName"].as_str().filter(|d| !d.is_empty()).unwrap_or(stem_of(name)).to_owned();
        }
        let show_ext = self.shell.setting("explorer.showExtensions") != false;
        if !show_ext && f["isDir"] != true { stem_of(display).to_owned() } else { display.to_owned() }
    }

    pub fn build_model(&self) {
        let d = self.dset();
        let flag = |k: &str, default: bool| d.get(k).and_then(Value::as_bool).unwrap_or(default);
        let (files, home, trash_count) = {
            let s = self.s.borrow();
            (s.files.clone(), s.home.clone(), s.trash_count)
        };
        let mut specials = Vec::new();
        if flag("showThisPC", true) {
            specials.push(Item {
                key: "::thispc".into(),
                special: Some("thispc"),
                label: self.t("Deze pc", "This PC"),
                icon: "computer".into(),
                is_dir: true,
                ..Default::default()
            });
        }
        if flag("showHome", false) {
            specials.push(Item {
                key: "::home".into(),
                special: Some("home"),
                label: self.t("Persoonlijke map", "Home folder"),
                icon: "user-home".into(),
                is_dir: true,
                path: home,
                ..Default::default()
            });
        }
        if flag("showTrash", true) {
            specials.push(Item {
                key: "::trash".into(),
                special: Some("trash"),
                label: self.t("Prullenbak", "Recycle Bin"),
                icon: if trash_count > 0 { "user-trash-full" } else { "user-trash" }.into(),
                is_dir: true,
                ..Default::default()
            });
        }
        specials.sort_by_key(|i| SPECIAL_ORDER.iter().position(|s| Some(*s) == i.special).unwrap_or(9));
        // The explicit legacy "Deze PC.desktop" file stays on disk; the built-in item opens the same place.
        let show_pc = flag("showThisPC", true);
        let mut items: Vec<Item> = files
            .iter()
            .filter(|f| !(show_pc && f["shellShortcut"] == "thispc"))
            .map(|f| Item {
                key: f["name"].as_str().unwrap_or("").to_owned(),
                special: None,
                label: self.label_for(f),
                icon: f["icon"].as_str().unwrap_or("").to_owned(),
                mime: f["mime"].as_str().unwrap_or("").to_owned(),
                path: f["path"].as_str().unwrap_or("").to_owned(),
                name: f["name"].as_str().unwrap_or("").to_owned(),
                is_dir: f["isDir"] == true,
                is_desktop_entry: f["isDesktopEntry"] == true,
                size: f["size"].as_u64().unwrap_or(0),
                modified: f["modified"].as_f64().unwrap_or(0.0),
            })
            .filter(|i| !i.key.is_empty())
            .collect();
        let by = d.get("sortBy").and_then(Value::as_str).unwrap_or("name").to_owned();
        items.sort_by(|a, b| {
            b.is_dir.cmp(&a.is_dir).then_with(|| {
                let name = || natural_cmp(&a.label, &b.label);
                match by.as_str() {
                    "size" => a.size.cmp(&b.size).then_with(name),
                    "type" => {
                        let ta = if a.is_dir { String::new() } else { Some(ext_of(&a.name)).filter(|e| !e.is_empty()).unwrap_or(a.mime.clone()) };
                        let tb = if b.is_dir { String::new() } else { Some(ext_of(&b.name)).filter(|e| !e.is_empty()).unwrap_or(b.mime.clone()) };
                        natural_cmp(&ta, &tb).then_with(name)
                    }
                    "modified" => b.modified.partial_cmp(&a.modified).unwrap_or(Ordering::Equal).then_with(name),
                    _ => name(),
                }
            })
        });
        let mut s = self.s.borrow_mut();
        s.model = specials.into_iter().chain(items).collect();
        let keys: HashSet<String> = s.model.iter().map(|i| i.key.clone()).collect();
        s.selected.retain(|k| keys.contains(k));
        if s.focus.as_ref().is_some_and(|k| !keys.contains(k)) {
            s.focus = None;
        }
        if s.anchor.as_ref().is_some_and(|k| !keys.contains(k)) {
            s.anchor = None;
        }
    }

    pub fn item(&self, key: &str) -> Option<Item> {
        self.s.borrow().model.iter().find(|i| i.key == key).cloned()
    }

    pub fn has(&self, key: &str) -> bool {
        self.s.borrow().model.iter().any(|i| i.key == key)
    }

    /* ---------------------------------------------------------------- layout */

    pub fn compute_metrics(&self) -> Metrics {
        let (cw, ch, icon) = size_dims(&self.icon_size());
        let ts = (self.shell.setting("accessibility.textScale").as_f64().unwrap_or(100.0)).clamp(100.0, 225.0) / 100.0;
        let cw = (cw + (ts - 1.0) * 24.0).round();
        let ch = (ch + (ts - 1.0) * 32.0).round();
        let bottom = if self.shell.setting("taskbar.autoHide") == true { 0.0 } else { crate::panel::BAR as f64 };
        let (w, h) = self.size.get();
        let (w, h) = (w as f64, h as f64 - bottom);
        let cols = (((w - ORIGIN * 2.0) / cw).floor() as i32).max(1);
        let rows = (((h - ORIGIN * 2.0) / ch).floor() as i32).max(1);
        Metrics { cw, ch, icon, cols, rows, tw: cw - 2.0 }
    }

    pub fn layout(self: &Rc<Self>) {
        let m = self.compute_metrics();
        let d = self.dset();
        let auto = d.get("autoArrange").and_then(Value::as_bool).unwrap_or(false);
        let align = d.get("alignToGrid").and_then(Value::as_bool).unwrap_or(true);
        let pos = d.get("positions").and_then(Value::as_object).cloned().unwrap_or_default();
        let mut s = self.s.borrow_mut();
        s.metrics = m;
        s.placed.clear();
        let model: Vec<String> = s.model.iter().map(|i| i.key.clone()).collect();
        if auto {
            for (i, key) in model.iter().enumerate() {
                let i = i as i32;
                s.placed.insert(key.clone(), ((i / m.rows) as f64, (i % m.rows) as f64));
            }
            return;
        }
        let mut occ: HashSet<(i32, i32)> = HashSet::new();
        let has_pos = !pos.is_empty();
        let mut rest = Vec::new();
        for key in &model {
            let p = pos.get(key);
            let (Some(x), Some(y)) = (p.and_then(|p| p["x"].as_f64()), p.and_then(|p| p["y"].as_f64())) else {
                rest.push(key.clone());
                continue;
            };
            let (x, y) = if align { (x.round(), y.round()) } else { (x, y) };
            if x < -0.01 || y < -0.01 || x > m.cols as f64 - 0.99 || y > m.rows as f64 - 0.99 {
                rest.push(key.clone());
                continue;
            }
            let cell = (x.round() as i32, y.round() as i32);
            if occ.contains(&cell) {
                rest.push(key.clone());
                continue;
            }
            occ.insert(cell);
            s.placed.insert(key.clone(), (x, y));
        }
        let mut added = false;
        for key in rest {
            let (c, r) = first_free(&occ, m.cols, m.rows);
            occ.insert((c, r));
            s.placed.insert(key.clone(), (c as f64, r as f64));
            // once the user arranged icons, new items keep their spot (like Windows)
            if has_pos && !pos.contains_key(&key) {
                s.pending_new.insert(key, json!({"x": c, "y": r}));
                added = true;
            }
        }
        drop(s);
        if added {
            self.save_positions_soon();
        }
    }

    fn save_positions_soon(self: &Rc<Self>) {
        if let Some(id) = self.save_timer.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(600), move || {
            let Some(d) = weak.upgrade() else { return };
            d.save_timer.borrow_mut().take();
            let add = std::mem::take(&mut d.s.borrow_mut().pending_new);
            if add.is_empty() {
                return;
            }
            let mut pos = d.dset().get("positions").and_then(Value::as_object).cloned().unwrap_or_default();
            for (k, v) in add {
                pos.insert(k, v);
            }
            d.set_desktop("positions", Value::Object(pos));
        });
        *self.save_timer.borrow_mut() = Some(id);
    }

    /// The pixel spot of a grid position.
    pub fn spot(m: &Metrics, (x, y): (f64, f64)) -> (f64, f64) {
        ((ORIGIN + x * m.cw + (m.cw - m.tw) / 2.0).round(), (ORIGIN + y * m.ch).round())
    }

    /* ---------------------------------------------------------------- render */

    pub fn render(self: &Rc<Self>) {
        let d = self.dset();
        let show = d.get("showIcons").and_then(Value::as_bool).unwrap_or(true);
        let size = self.icon_size();
        let (model, placed, selected, focus, kbd, m, cut, dragging) = {
            let s = self.s.borrow();
            let cut: HashSet<String> =
                if s.clip_op == "cut" { s.clip_paths.iter().cloned().collect() } else { HashSet::new() };
            let dragging: HashSet<String> = match &s.gesture {
                Some(Gesture::Drag { keys, .. }) => keys.iter().cloned().collect(),
                _ => HashSet::new(),
            };
            (s.model.clone(), s.placed.clone(), s.selected.clone(), s.focus.clone(), s.kbd, s.metrics, cut, dragging)
        };
        for s in SIZE_ORDER {
            if s == size { self.layer.add_css_class(&format!("dt-{s}")) } else { self.layer.remove_css_class(&format!("dt-{s}")) }
        }
        if kbd { self.layer.add_css_class("is-kbd") } else { self.layer.remove_css_class("is-kbd") }
        let mut created = Vec::new();
        {
            let mut tiles = self.tiles.borrow_mut();
            let mut seen = HashSet::new();
            for it in &model {
                seen.insert(it.key.clone());
                let sig = format!("{}|{}|{}|{}|{}|{}", it.label, it.icon, it.mime, it.path, m.icon, it.modified);
                let fresh = !tiles.contains_key(&it.key);
                if fresh {
                    let tile = make_tile(it, m.icon);
                    let (x, y) = Desk::spot(&m, placed.get(&it.key).copied().unwrap_or((0.0, 0.0)));
                    self.layer.put(&tile.root, x, y);
                    tiles.insert(it.key.clone(), tile);
                }
                let Some(tile) = tiles.get_mut(&it.key) else { continue };
                if tile.sig != sig {
                    tile.sig = sig;
                    tile.label.set_label(&it.label);
                    tile.image.set_pixel_size(m.icon);
                    if let Some(wrap) = tile.image.parent() {
                        wrap.set_size_request(m.icon, m.icon);
                    }
                    created.push((it.clone(), tile.image.clone()));
                }
                tile.root.set_widget_name(&format!("desktop-item-{}", it.key));
                tile.root.set_size_request(m.tw as i32, -1);
                if !dragging.contains(&it.key) {
                    let (x, y) = Desk::spot(&m, placed.get(&it.key).copied().unwrap_or((0.0, 0.0)));
                    self.layer.move_(&tile.root, x, y);
                }
                let sel = selected.contains(&it.key);
                toggle(&tile.root, "is-selected", sel);
                toggle(&tile.root, "is-focused", focus.as_deref() == Some(it.key.as_str()));
                toggle(&tile.root, "is-cut", !it.path.is_empty() && cut.contains(&it.path));
                if sel {
                    tile.label.set_lines(-1);
                    tile.label.set_ellipsize(pango::EllipsizeMode::None);
                } else {
                    tile.label.set_lines(2);
                    tile.label.set_ellipsize(pango::EllipsizeMode::End);
                }
                tile.root.set_visible(show);
            }
            let gone: Vec<String> = tiles.keys().filter(|k| !seen.contains(*k)).cloned().collect();
            for key in gone {
                if let Some(tile) = tiles.remove(&key) {
                    self.layer.remove(&tile.root);
                }
            }
        }
        for (it, image) in created {
            self.load_icon(&it, &image, m.icon);
        }
    }

    pub fn refresh_view(self: &Rc<Self>) {
        self.build_model();
        self.layout();
        self.render();
    }

    pub async fn reload(self: &Rc<Self>) {
        self.load_files().await;
        self.load_trash().await;
        self.refresh_view();
    }

    pub fn reload_flash(self: &Rc<Self>) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            desk.reload().await;
            flash(&desk.layer);
        });
    }

    pub fn reload_soon(self: &Rc<Self>) {
        if let Some(id) = self.reload_timer.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(120), move || {
            let Some(d) = weak.upgrade() else { return };
            d.reload_timer.borrow_mut().take();
            glib::spawn_future_local(async move { d.reload().await });
        });
        *self.reload_timer.borrow_mut() = Some(id);
    }

    /// The icon of an item: a thumbnail for pictures, else the item's icon or its MIME type's, from the
    /// backend (the same icons as the HTML desktop), else the GTK theme's folder or file.
    fn load_icon(self: &Rc<Self>, it: &Item, image: &gtk::Image, size: i32) {
        let px = size * self.scale();
        let mut urls = Vec::new();
        if it.special.is_none() && !it.path.is_empty() && it.mime.starts_with("image/") {
            urls.push(format!("/thumb?path={}&size={px}", super::enc(&it.path)));
        }
        let lower = it.icon.to_lowercase();
        let lower = lower.trim_end_matches(".desktop");
        if ["win11shell-settings", "io.github.win11shell.settings", "preferences-system", "org.gnome.settings", "gnome-control-center", "systemsettings"]
            .contains(&lower)
        {
            urls.push("/ui/assets/icons/settings.svg".into());
        }
        if !it.icon.is_empty() {
            urls.push(format!("/icon?name={}&size={px}", super::enc(&it.icon)));
        }
        let mime = if it.is_dir && it.mime.is_empty() && it.special.is_none() { "inode/directory" } else { it.mime.as_str() };
        if !mime.is_empty() {
            urls.push(format!("/icon?mime={}&size={px}", super::enc(mime)));
        }
        let fallback = if it.is_dir { "folder" } else { "text-x-generic" };
        let cache_key = urls.join("|");
        if let Some(hit) = self.icons.borrow().get(&cache_key) {
            set_icon(image, hit.as_ref(), fallback);
            return;
        }
        let backend = self.shell.backend.clone();
        let weak_image = image.downgrade();
        let weak = Rc::downgrade(self);
        glib::spawn_future_local(async move {
            let texture = gio::spawn_blocking(move || {
                urls.iter().find_map(|url| backend.get(url).ok().and_then(|b| super::decode(&b, Some(((px, px), false)))))
            })
            .await
            .ok()
            .flatten();
            if let Some(d) = weak.upgrade() {
                d.icons.borrow_mut().insert(cache_key, texture.clone());
            }
            if let Some(image) = weak_image.upgrade() {
                set_icon(&image, texture.as_ref(), fallback);
            }
        });
    }

    pub fn busy_cursor(self: &Rc<Self>) {
        self.window.set_cursor_from_name(Some("progress"));
        if let Some(id) = self.busy_timer.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(1400), move || {
            if let Some(d) = weak.upgrade() {
                d.busy_timer.borrow_mut().take();
                d.window.set_cursor(None);
            }
        });
        *self.busy_timer.borrow_mut() = Some(id);
    }

    /* ------------------------------------------------------------- selection */

    pub fn set_selection(self: &Rc<Self>, keys: Vec<String>, focus: Option<Option<String>>) {
        {
            let mut s = self.s.borrow_mut();
            s.selected = keys.into_iter().collect();
            if let Some(f) = focus {
                s.focus = f;
            }
        }
        self.render();
    }

    pub fn select_only(self: &Rc<Self>, key: Option<String>) {
        self.s.borrow_mut().anchor = key.clone();
        self.set_selection(key.clone().into_iter().collect(), Some(key));
    }

    pub fn clear_selection(self: &Rc<Self>) {
        self.set_selection(Vec::new(), None);
    }

    pub fn selected_items(&self) -> Vec<Item> {
        let s = self.s.borrow();
        s.model.iter().filter(|i| s.selected.contains(&i.key)).cloned().collect()
    }

    /// Column-major order of the current layout (Shift ranges, type-to-select).
    pub fn visual_order(&self) -> Vec<Item> {
        let s = self.s.borrow();
        let mut order = s.model.clone();
        let at = |k: &str| s.placed.get(k).copied().unwrap_or((0.0, 0.0));
        order.sort_by(|a, b| {
            let (pa, pb) = (at(&a.key), at(&b.key));
            (pa.0.round() as i64).cmp(&(pb.0.round() as i64)).then(pa.1.partial_cmp(&pb.1).unwrap_or(Ordering::Equal))
        });
        order
    }

    fn range_select(self: &Rc<Self>, to: &str) {
        let order: Vec<String> = self.visual_order().into_iter().map(|i| i.key).collect();
        let anchor = self.s.borrow().anchor.clone().unwrap_or_else(|| to.to_owned());
        match (order.iter().position(|k| *k == anchor), order.iter().position(|k| k == to)) {
            (Some(a), Some(b)) => {
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                self.set_selection(order[lo..=hi].to_vec(), Some(Some(to.to_owned())));
            }
            _ => self.select_only(Some(to.to_owned())),
        }
    }

    /// The item under a point of the icon layer (not counting `exclude`).
    pub fn hit(&self, x: f64, y: f64, exclude: &[String]) -> Option<String> {
        let point = graphene::Point::new(x as f32, y as f32);
        let tiles = self.tiles.borrow();
        tiles
            .iter()
            .filter(|(k, t)| !exclude.contains(k) && t.root.is_visible())
            .find(|(_, t)| t.root.compute_bounds(&self.layer).is_some_and(|b| b.contains_point(&point)))
            .map(|(k, _)| k.clone())
    }

    /// A tile's rectangle in the icon layer.
    pub fn tile_rect(&self, key: &str) -> Option<graphene::Rect> {
        self.tiles.borrow().get(key).and_then(|t| t.root.compute_bounds(&self.layer))
    }

    fn in_rename(&self, x: f64, y: f64) -> bool {
        let s = self.s.borrow();
        let Some(r) = &s.renaming else { return false };
        let inside = |w: &gtk::Widget| {
            w.compute_bounds(&self.layer)
                .is_some_and(|b| b.contains_point(&graphene::Point::new(x as f32, y as f32)))
        };
        inside(r.entry.upcast_ref()) || r.tip.as_ref().is_some_and(|t| t.is_visible() && inside(t.upcast_ref()))
    }

    /* -------------------------------------------------------------- pointer */

    fn install_input(self: &Rc<Self>) {
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |g, x, y| {
            if let Some(d) = weak.upgrade() {
                d.press(x, y, g.current_event_state());
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_update(move |_, dx, dy| {
            if let Some(d) = weak.upgrade() {
                d.motion(dx, dy);
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |g, dx, dy| {
            if let Some(d) = weak.upgrade() {
                d.release(dx, dy, g.current_event_state());
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_cancel(move |_, _| {
            if let Some(d) = weak.upgrade() {
                d.cancel_gesture();
            }
        });
        self.layer.add_controller(drag);

        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |g, n, x, y| {
            let Some(d) = weak.upgrade() else { return };
            if d.in_rename(x, y) {
                return;
            }
            match g.current_button() {
                gdk::BUTTON_PRIMARY if n == 2 => d.double(x, y),
                gdk::BUTTON_SECONDARY => d.context(x, y, g.current_event_state()),
                _ => {}
            }
        });
        self.layer.add_controller(click);

        let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let weak = Rc::downgrade(self);
        let lock = Rc::new(Cell::new(false));
        scroll.connect_scroll(move |c, _, dy| {
            let Some(d) = weak.upgrade() else { return glib::Propagation::Proceed };
            if !c.current_event_state().contains(gdk::ModifierType::CONTROL_MASK) {
                return glib::Propagation::Proceed;
            }
            // Ctrl + mouse wheel changes the icon size (like Windows)
            let size = d.icon_size();
            let i = SIZE_ORDER.iter().position(|s| *s == size).unwrap_or(1) as i32;
            let j = (i + if dy < 0.0 { 1 } else { -1 }).clamp(0, 2);
            if j != i && !lock.get() {
                lock.set(true);
                let lock = lock.clone();
                glib::timeout_add_local_once(Duration::from_millis(250), move || lock.set(false));
                d.set_icon_size(SIZE_ORDER[j as usize]);
            }
            glib::Propagation::Stop
        });
        self.layer.add_controller(scroll);

        // capture: before the window's own focus keys (arrows, Tab); renaming and open menus get their keys
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, code, state| match weak.upgrade() {
            Some(d) => d.key(key, code, state),
            None => glib::Propagation::Proceed,
        });
        self.window.add_controller(keys);

        // the real (HiDPI) scale arrives after mapping: wallpaper and icons again at device pixels
        let weak = Rc::downgrade(self);
        self.window.connect_scale_factor_notify(move |_| {
            let Some(d) = weak.upgrade() else { return };
            d.apply_wallpaper(true);
            for tile in d.tiles.borrow_mut().values_mut() {
                tile.sig.clear();
            }
            if d.s.borrow().loaded {
                d.render();
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_is_active_notify(move |w| {
            if let Some(d) = weak.upgrade() {
                toggle(&d.layer, "is-active", w.is_active());
            }
        });
        // Alt+F4 on the desktop asks to shut down, as in Windows; the desktop itself never closes.
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            if let Some(d) = weak.upgrade()
                && d.alive.get()
            {
                if d.shell.flyouts.borrow().is_some() {
                    crate::flyouts::power::confirm(&d.shell, "shutdown");
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    fn press(self: &Rc<Self>, x: f64, y: f64, state: gdk::ModifierType) {
        if self.in_rename(x, y) {
            self.s.borrow_mut().gesture = None;
            return;
        }
        if self.s.borrow().renaming.is_some() {
            self.commit_rename(true);
        }
        self.s.borrow_mut().kbd = false;
        self.layer.remove_css_class("is-kbd");
        self.layer.grab_focus();
        let additive = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        match self.hit(x, y, &[]) {
            Some(key) => {
                let selected = self.s.borrow().selected.contains(&key);
                if shift {
                    self.range_select(&key);
                } else if !selected && !additive {
                    self.select_only(Some(key.clone()));
                } else {
                    self.s.borrow_mut().focus = Some(key.clone());
                    self.render();
                }
                let was_selected = self.s.borrow().selected.contains(&key);
                let mut added = false;
                if additive && !was_selected {
                    let mut s = self.s.borrow_mut();
                    s.selected.insert(key.clone());
                    s.anchor = Some(key.clone());
                    added = true;
                }
                self.s.borrow_mut().gesture = Some(Gesture::Press { key, x, y, toggle: additive, was_selected, added });
                if added {
                    self.render();
                }
            }
            None => {
                let base = if additive { self.s.borrow().selected.clone() } else { HashSet::new() };
                self.s.borrow_mut().gesture = Some(Gesture::Band { x, y, base, moved: false, rects: None });
                if !additive && !shift {
                    self.clear_selection();
                }
            }
        }
    }

    fn motion(self: &Rc<Self>, dx: f64, dy: f64) {
        enum Next {
            Nothing,
            Band(f64, f64, f64, f64),
            StartDrag,
            Drag(Vec<String>, HashMap<String, (f64, f64)>, f64, f64),
        }
        let next = {
            let s = self.s.borrow();
            match &s.gesture {
                Some(Gesture::Band { x, y, moved, .. }) => {
                    if !moved && dx.hypot(dy) < 3.0 { Next::Nothing } else { Next::Band(*x, *y, x + dx, y + dy) }
                }
                Some(Gesture::Press { .. }) => {
                    if dx.hypot(dy) < 5.0 { Next::Nothing } else { Next::StartDrag }
                }
                Some(Gesture::Drag { keys, origin, x, y, .. }) => Next::Drag(keys.clone(), origin.clone(), x + dx, y + dy),
                None => Next::Nothing,
            }
        };
        match next {
            Next::Nothing => {}
            Next::Band(x0, y0, x1, y1) => self.move_band(x0, y0, x1, y1),
            Next::StartDrag => {
                self.start_drag();
                self.motion(dx, dy);
            }
            Next::Drag(keys, origin, cx, cy) => {
                {
                    let tiles = self.tiles.borrow();
                    for k in &keys {
                        if let (Some(t), Some((ox, oy))) = (tiles.get(k), origin.get(k)) {
                            self.layer.move_(&t.root, ox + dx, oy + dy);
                        }
                    }
                }
                let target = self.drop_target(cx, cy, &keys);
                let previous = match &mut self.s.borrow_mut().gesture {
                    Some(Gesture::Drag { target: t, .. }) => std::mem::replace(t, target.clone()),
                    _ => None,
                };
                if previous != target {
                    let tiles = self.tiles.borrow();
                    if let Some(t) = previous.and_then(|k| tiles.get(&k)) {
                        t.root.remove_css_class("is-drop-target");
                    }
                    if let Some(t) = target.as_ref().and_then(|k| tiles.get(k)) {
                        t.root.add_css_class("is-drop-target");
                    }
                }
            }
        }
    }

    fn move_band(self: &Rc<Self>, x0: f64, y0: f64, x1: f64, y1: f64) {
        let (l, t, w, h) = (x0.min(x1), y0.min(y1), (x1 - x0).abs(), (y1 - y0).abs());
        self.band.set_size_request(w.round() as i32, h.round() as i32);
        self.layer.move_(&self.band, l, t);
        if !self.band.is_visible() {
            // on top of the icons
            self.band.insert_before(&self.layer, None::<&gtk::Widget>);
            self.band.set_visible(true);
        }
        let need_rects = matches!(&self.s.borrow().gesture, Some(Gesture::Band { rects: None, .. }));
        let rects = if need_rects {
            let tiles = self.tiles.borrow();
            Some(
                tiles
                    .iter()
                    .filter(|(_, t)| t.root.is_visible())
                    .filter_map(|(k, t)| t.root.compute_bounds(&self.layer).map(|b| (k.clone(), b)))
                    .collect::<Vec<_>>(),
            )
        } else {
            None
        };
        let show = self.dset().get("showIcons").and_then(Value::as_bool).unwrap_or(true);
        {
            let mut s = self.s.borrow_mut();
            let hit = match &mut s.gesture {
                Some(Gesture::Band { base, moved, rects: r, .. }) => {
                    *moved = true;
                    if let Some(new) = rects {
                        *r = Some(new);
                    }
                    if !show {
                        return;
                    }
                    let mut hit = base.clone();
                    for (key, b) in r.iter().flatten() {
                        let (bl, bt, br, bb) =
                            (b.x() as f64, b.y() as f64, (b.x() + b.width()) as f64, (b.y() + b.height()) as f64);
                        if bl < l + w && br > l && bt < t + h && bb > t {
                            if base.contains(key) {
                                hit.remove(key);
                            } else {
                                hit.insert(key.clone());
                            }
                        }
                    }
                    hit
                }
                _ => return,
            };
            s.selected = hit;
        }
        self.render();
    }

    fn release(self: &Rc<Self>, dx: f64, dy: f64, state: gdk::ModifierType) {
        let Some(g) = self.s.borrow_mut().gesture.take() else { return };
        match g {
            Gesture::Band { .. } => self.band.set_visible(false),
            Gesture::Drag { x, y, keys, origin, target } => self.drop_drag(x, y, dx, dy, keys, origin, target),
            Gesture::Press { key, toggle, was_selected, added, .. } => {
                // a plain click on an already selected item narrows the selection to it
                if toggle {
                    if !added && was_selected {
                        self.s.borrow_mut().selected.remove(&key);
                        self.render();
                    }
                } else if !state.contains(gdk::ModifierType::SHIFT_MASK) && self.s.borrow().selected.len() > 1 {
                    self.select_only(Some(key));
                }
            }
        }
    }

    fn cancel_gesture(self: &Rc<Self>) {
        let Some(g) = self.s.borrow_mut().gesture.take() else { return };
        match g {
            Gesture::Band { .. } => self.band.set_visible(false),
            Gesture::Drag { keys, target, .. } => self.clear_drag(&keys, target.as_deref()),
            Gesture::Press { .. } => {}
        }
    }

    fn double(self: &Rc<Self>, x: f64, y: f64) {
        if let Some(it) = self.hit(x, y, &[]).and_then(|k| self.item(&k)) {
            self.open_items(vec![it]);
        }
    }

    fn context(self: &Rc<Self>, x: f64, y: f64, state: gdk::ModifierType) {
        self.layer.grab_focus();
        match self.hit(x, y, &[]) {
            Some(key) => {
                if !self.s.borrow().selected.contains(&key) {
                    self.select_only(Some(key));
                } else {
                    self.s.borrow_mut().focus = Some(key);
                    self.render();
                }
                super::menus::open_menu_at(self, x, y, true);
            }
            None => {
                if !state.contains(gdk::ModifierType::CONTROL_MASK) {
                    self.clear_selection();
                }
                super::menus::open_menu_at(self, x, y, false);
            }
        }
    }

    /* ------------------------------------------------------------ drag & drop */

    fn start_drag(self: &Rc<Self>) {
        let (key, x, y) = match &self.s.borrow().gesture {
            Some(Gesture::Press { key, x, y, .. }) => (key.clone(), *x, *y),
            _ => return,
        };
        if !self.s.borrow().selected.contains(&key) {
            self.select_only(Some(key.clone()));
        }
        let mut keys = vec![key.clone()];
        keys.extend(self.s.borrow().selected.iter().filter(|k| **k != key).cloned());
        let (m, placed) = {
            let s = self.s.borrow();
            (s.metrics, s.placed.clone())
        };
        let mut origin = HashMap::new();
        {
            let tiles = self.tiles.borrow();
            for k in &keys {
                if let Some(t) = tiles.get(k) {
                    origin.insert(k.clone(), Desk::spot(&m, placed.get(k).copied().unwrap_or((0.0, 0.0))));
                    t.root.add_css_class("dt-ghost");
                    t.root.insert_before(&self.layer, None::<&gtk::Widget>);
                }
            }
        }
        self.layer.add_css_class("is-dragging");
        self.s.borrow_mut().gesture = Some(Gesture::Drag { x, y, keys, origin, target: None });
    }

    fn drop_target(&self, x: f64, y: f64, keys: &[String]) -> Option<String> {
        let key = self.hit(x, y, keys)?;
        let it = self.item(&key)?;
        (matches!(it.special, Some("trash") | Some("home")) || (it.is_dir && it.special.is_none())).then_some(key)
    }

    fn clear_drag(self: &Rc<Self>, keys: &[String], target: Option<&str>) {
        self.layer.remove_css_class("is-dragging");
        {
            let tiles = self.tiles.borrow();
            for k in keys {
                if let Some(t) = tiles.get(k) {
                    t.root.remove_css_class("dt-ghost");
                }
            }
            if let Some(t) = target.and_then(|k| tiles.get(k)) {
                t.root.remove_css_class("is-drop-target");
            }
        }
        self.render();
    }

    #[allow(clippy::too_many_arguments)]
    fn drop_drag(
        self: &Rc<Self>,
        _x: f64,
        _y: f64,
        dx: f64,
        dy: f64,
        keys: Vec<String>,
        _origin: HashMap<String, (f64, f64)>,
        target: Option<String>,
    ) {
        let items: Vec<Item> = keys.iter().filter_map(|k| self.item(k)).collect();
        if let Some(target) = target.and_then(|k| self.item(&k)) {
            self.clear_drag(&keys, Some(&target.key));
            let files: Vec<Item> = items.into_iter().filter(|i| i.special.is_none() && !i.path.is_empty()).collect();
            if files.is_empty() {
                return;
            }
            if target.special == Some("trash") {
                self.trash_items(files);
                return;
            }
            let dest = if target.special == Some("home") { self.s.borrow().home.clone() } else { target.path.clone() };
            self.move_into(files, dest);
            return;
        }
        let d = self.dset();
        if d.get("autoArrange").and_then(Value::as_bool).unwrap_or(false) {
            self.clear_drag(&keys, None);
            return;
        }
        let align = d.get("alignToGrid").and_then(Value::as_bool).unwrap_or(true);
        let mut pos = Map::new();
        {
            let mut s = self.s.borrow_mut();
            let m = s.metrics;
            let (dxc, dyc) = (dx / m.cw, dy / m.ch);
            let mut occ: HashSet<(i32, i32)> = s
                .placed
                .iter()
                .filter(|(k, _)| !keys.contains(k))
                .map(|(_, p)| (p.0.round() as i32, p.1.round() as i32))
                .collect();
            let mut next = Vec::new();
            for k in &keys {
                let Some(o) = s.placed.get(k).copied() else { continue };
                let mut x = (o.0 + dxc).clamp(0.0, (m.cols - 1) as f64);
                let mut y = (o.1 + dyc).clamp(0.0, (m.rows - 1) as f64);
                if align {
                    let (mut c, mut r) = (x.round() as i32, y.round() as i32);
                    if occ.contains(&(c, r)) {
                        (c, r) = nearest_free(c, r, &occ, m.cols, m.rows);
                    }
                    x = c as f64;
                    y = r as f64;
                }
                occ.insert((x.round() as i32, y.round() as i32));
                next.push((k.clone(), (x, y)));
            }
            for (k, p) in next {
                s.placed.insert(k, p);
            }
            for (k, p) in &s.placed {
                pos.insert(k.clone(), json!({"x": p.0, "y": p.1}));
            }
        }
        self.clear_drag(&keys, None);
        self.set_desktop("positions", Value::Object(pos));
    }

    /* -------------------------------------------------------------- keyboard */

    fn key(self: &Rc<Self>, key: gdk::Key, code: u32, state: gdk::ModifierType) -> glib::Propagation {
        use gdk::Key;
        if self.s.borrow().renaming.is_some() || self.menu_open.get() || !self.layer.has_focus() {
            return glib::Propagation::Proceed;
        }
        let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = state.contains(gdk::ModifierType::ALT_MASK);
        let sel = self.selected_items();
        let stop = glib::Propagation::Stop;
        if key == Key::Escape && matches!(self.s.borrow().gesture, Some(Gesture::Drag { .. })) {
            self.cancel_gesture();
            return stop;
        }
        if matches!(key, Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End | Key::KP_Up | Key::KP_Down | Key::KP_Left | Key::KP_Right) {
            self.move_focus(key, ctrl, shift);
            return stop;
        }
        // Ctrl+Shift+2/3/4 by physical key (evdev 11..13 + 8 offset as GDK reports them)
        if ctrl && shift && (11..=13).contains(&code) {
            self.set_icon_size(["large", "medium", "small"][(code - 11) as usize]);
            return stop;
        }
        let lower = key.to_lower();
        if ctrl && shift && lower == Key::n {
            self.create_new("folder", None);
            return stop;
        }
        if ctrl && shift && lower == Key::c {
            let with_path: Vec<Item> = sel.into_iter().filter(|i| !i.path.is_empty()).collect();
            if !with_path.is_empty() {
                self.copy_path(&with_path);
            }
            return stop;
        }
        if ctrl && !shift {
            match lower {
                Key::a => {
                    let keys: Vec<String> = self.s.borrow().model.iter().map(|i| i.key.clone()).collect();
                    let first = keys.first().cloned();
                    let focus = self.s.borrow().focus.clone().or(first.clone());
                    self.s.borrow_mut().anchor = first;
                    self.set_selection(keys, Some(focus));
                    return stop;
                }
                Key::c => {
                    self.set_clipboard(&sel, "copy");
                    return stop;
                }
                Key::x => {
                    self.set_clipboard(&sel, "cut");
                    return stop;
                }
                Key::v => {
                    self.paste(None);
                    return stop;
                }
                Key::z => {
                    self.run_undo();
                    return stop;
                }
                Key::space => {
                    let focus = self.s.borrow().focus.clone();
                    if let Some(f) = focus {
                        {
                            let mut s = self.s.borrow_mut();
                            if !s.selected.remove(&f) {
                                s.selected.insert(f);
                            }
                        }
                        self.render();
                    }
                    return stop;
                }
                _ => {}
            }
        }
        if matches!(key, Key::Return | Key::KP_Enter) {
            if alt {
                let list: Vec<Item> = sel.into_iter().filter(|i| i.special.is_none() || i.special == Some("home")).collect();
                self.properties(list);
            } else if !sel.is_empty() {
                self.open_items(sel);
            }
            return stop;
        }
        match key {
            Key::F2 => {
                let focus = self.s.borrow().focus.clone();
                let target = if sel.len() == 1 { Some(sel[0].key.clone()) } else { focus };
                if let Some(k) = target {
                    self.start_rename(&k);
                }
                return stop;
            }
            Key::F5 => {
                self.reload_flash();
                return stop;
            }
            Key::Delete | Key::KP_Delete => {
                self.trash_items(sel);
                return stop;
            }
            Key::Escape => {
                if !self.s.borrow().selected.is_empty() {
                    self.clear_selection();
                    return stop;
                }
                return glib::Propagation::Proceed;
            }
            _ => {}
        }
        if key == Key::Menu || (shift && key == Key::F10) {
            let focus = self.s.borrow().focus.clone();
            let lead = if sel.is_empty() {
                None
            } else {
                Some(focus.filter(|f| sel.iter().any(|i| i.key == *f)).unwrap_or_else(|| sel[0].key.clone()))
            };
            let rect = lead.as_deref().and_then(|k| self.tile_rect(k));
            match rect {
                Some(r) => super::menus::open_menu_at(
                    self,
                    (r.x() + (r.width() / 2.0).min(32.0)) as f64,
                    (r.y() + (r.height() / 2.0).min(40.0)) as f64,
                    true,
                ),
                None => super::menus::open_menu_at(self, 24.0, 24.0, false),
            }
            return stop;
        }
        // type to select
        if let Some(c) = key.to_unicode().filter(|c| !c.is_control() && *c != ' ')
            && !ctrl
            && !alt
        {
            let now = Instant::now();
            let buf = {
                let mut s = self.s.borrow_mut();
                if s.type_at.is_none_or(|t| now.duration_since(t) > Duration::from_millis(900)) {
                    s.type_buf.clear();
                }
                s.type_at = Some(now);
                s.type_buf.extend(c.to_lowercase());
                s.type_buf.clone()
            };
            let order = self.visual_order();
            let focus = self.s.borrow().focus.clone();
            let start = order.iter().position(|i| Some(&i.key) == focus.as_ref()).unwrap_or(0);
            let first = buf.chars().next().unwrap_or(c);
            let single = buf.chars().count() > 1 && buf.chars().all(|x| x == first);
            let prefix = if single { first.to_string() } else { buf.clone() };
            let step = if buf.chars().count() == 1 || single { 1 } else { 0 };
            let n = order.len();
            for i in 0..n {
                let it = &order[(start + i + step) % n];
                if it.label.to_lowercase().starts_with(&prefix) {
                    self.s.borrow_mut().kbd = true;
                    self.select_only(Some(it.key.clone()));
                    break;
                }
            }
            return stop;
        }
        glib::Propagation::Proceed
    }

    fn move_focus(self: &Rc<Self>, key: gdk::Key, ctrl: bool, shift: bool) {
        use gdk::Key;
        let order = self.visual_order();
        let cur = self.s.borrow().focus.clone().filter(|k| self.has(k));
        let next = match (&cur, key) {
            (None, _) | (_, Key::Home) => order.first().map(|i| i.key.clone()),
            (_, Key::End) => order.last().map(|i| i.key.clone()),
            (Some(cur), _) => {
                let s = self.s.borrow();
                let p = s.placed.get(cur).copied().unwrap_or((0.0, 0.0));
                let v = match key {
                    Key::Up | Key::KP_Up => (0.0, -1.0),
                    Key::Down | Key::KP_Down => (0.0, 1.0),
                    Key::Left | Key::KP_Left => (-1.0, 0.0),
                    _ => (1.0, 0.0),
                };
                let mut best = None;
                let mut best_score = f64::INFINITY;
                for it in &s.model {
                    if it.key == *cur {
                        continue;
                    }
                    let q = s.placed.get(&it.key).copied().unwrap_or((0.0, 0.0));
                    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
                    let along = dx * v.0 + dy * v.1;
                    if along <= 0.01 {
                        continue;
                    }
                    let across = (dx * v.1).abs() + (dy * v.0).abs();
                    let score = along + across * 3.0;
                    if score < best_score {
                        best_score = score;
                        best = Some(it.key.clone());
                    }
                }
                best
            }
        };
        let Some(next) = next else { return };
        self.s.borrow_mut().kbd = true;
        if shift {
            self.range_select(&next);
        } else if ctrl {
            self.s.borrow_mut().focus = Some(next);
            self.render();
        } else {
            self.select_only(Some(next));
        }
    }

    /* ---------------------------------------------------------------- rename */

    pub fn start_rename(self: &Rc<Self>, key: &str) {
        let Some(it) = self.item(key) else { return };
        if it.special.is_some() || it.is_desktop_entry || it.path.is_empty() {
            return;
        }
        self.finish_rename(false);
        let (m, p) = {
            let s = self.s.borrow();
            (s.metrics, s.placed.get(key).copied().unwrap_or((0.0, 0.0)))
        };
        let (x, y) = Desk::spot(&m, p);
        let name = it.name.clone();
        let entry = gtk::Entry::new();
        entry.add_css_class("dt-rename");
        entry.set_widget_name("desktop-rename");
        entry.set_text(&name);
        EditableExt::set_alignment(&entry, 0.5);
        entry.set_width_chars(1);
        entry.set_max_width_chars(1);
        entry.set_size_request(m.tw as i32 + 12, -1);
        self.layer.put(&entry, x - 6.0, y + m.icon as f64 + 10.0);
        if let Some(t) = self.tiles.borrow().get(key) {
            t.root.add_css_class("is-renaming");
        }
        self.s.borrow_mut().renaming =
            Some(Rename { key: key.to_owned(), entry: entry.clone(), old: name.clone(), busy: false, tip: None });

        let weak = Rc::downgrade(self);
        entry.connect_activate(move |_| {
            if let Some(d) = weak.upgrade() {
                d.commit_rename(false);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(d) = weak.upgrade() else { return glib::Propagation::Proceed };
            match key {
                gdk::Key::Escape => {
                    d.finish_rename(true);
                    glib::Propagation::Stop
                }
                gdk::Key::Tab | gdk::Key::ISO_Left_Tab => {
                    d.commit_rename(false);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        entry.add_controller(keys);
        // blur commits (like the HTML textarea), once the focus has really gone elsewhere: the error tip or a
        // dialog taking it for a moment does not count
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(self);
        let mine = entry.downgrade();
        focus.connect_leave(move |controller| {
            let (weak, mine, controller) = (weak.clone(), mine.clone(), controller.downgrade());
            glib::timeout_add_local_once(Duration::from_millis(60), move || {
                let Some(d) = weak.upgrade() else { return };
                if controller.upgrade().is_some_and(|c| c.contains_focus()) {
                    return;
                }
                let current =
                    matches!(&d.s.borrow().renaming, Some(r) if Some(&r.entry) == mine.upgrade().as_ref() && !r.busy);
                if current {
                    d.commit_rename(true);
                }
            });
        });
        entry.add_controller(focus);
        let weak = Rc::downgrade(self);
        entry.connect_changed(move |_| {
            if let Some(d) = weak.upgrade()
                && let Some(tip) = d.s.borrow().renaming.as_ref().and_then(|r| r.tip.clone())
            {
                tip.popdown();
            }
        });
        let dot = if it.is_dir { None } else { name.rfind('.').filter(|i| *i > 0) };
        let end = dot.map(|i| name[..i].chars().count() as i32).unwrap_or(-1);
        let e = entry.clone();
        glib::idle_add_local_once(move || {
            e.grab_focus();
            e.select_region(0, end);
        });
    }

    pub fn finish_rename(self: &Rc<Self>, keep_focus: bool) {
        let Some(r) = self.s.borrow_mut().renaming.take() else { return };
        if let Some(tip) = r.tip {
            tip.popdown();
            tip.unparent();
        }
        if r.entry.parent().is_some() {
            self.layer.remove(&r.entry);
        }
        if let Some(t) = self.tiles.borrow().get(&r.key) {
            t.root.remove_css_class("is-renaming");
        }
        if keep_focus {
            self.layer.grab_focus();
        }
    }

    pub fn rename_tip(self: &Rc<Self>, message: &str) {
        let entry = {
            let s = self.s.borrow();
            let Some(r) = &s.renaming else { return };
            r.entry.clone()
        };
        let existing = self.s.borrow().renaming.as_ref().and_then(|r| r.tip.clone());
        let tip = existing.unwrap_or_else(|| {
            let tip = gtk::Popover::new();
            tip.add_css_class("dt-rename-tip");
            tip.set_autohide(false);
            tip.set_has_arrow(true);
            tip.set_position(gtk::PositionType::Bottom);
            tip.set_parent(&entry);
            tip
        });
        let label = gtk::Label::new(Some(message));
        label.set_wrap(true);
        label.set_max_width_chars(30);
        label.set_xalign(0.0);
        tip.set_child(Some(&label));
        if let Some(r) = self.s.borrow_mut().renaming.as_mut() {
            r.tip = Some(tip.clone());
        }
        tip.popup();
        entry.grab_focus();
    }

    pub fn commit_rename(self: &Rc<Self>, from_blur: bool) {
        let (key, value, old) = {
            let s = self.s.borrow();
            let Some(r) = &s.renaming else { return };
            if r.busy {
                return;
            }
            let value = r.entry.text().replace(['\r', '\n'], " ").trim().to_owned();
            (r.key.clone(), value, r.old.clone())
        };
        let Some(it) = self.item(&key) else {
            self.finish_rename(!from_blur);
            return;
        };
        if value.is_empty() || value == old {
            self.finish_rename(!from_blur);
            return;
        }
        if value.contains('/') {
            if from_blur {
                self.finish_rename(false);
            } else {
                self.rename_tip(&self.t("Een naam mag geen schuine streep (/) bevatten.", "A name can’t contain a slash (/)."));
            }
            return;
        }
        if let Some(r) = self.s.borrow_mut().renaming.as_mut() {
            r.busy = true;
        }
        if !it.is_dir && ext_of(&value) != ext_of(&old) {
            let weak = Rc::downgrade(self);
            super::dialogs::confirm(
                self,
                &self.t("Bestandsextensie wijzigen?", "Change the file name extension?"),
                &self.t(
                    "Als je de extensie wijzigt (het deel na de punt), kan het bestand misschien niet meer worden geopend. Wil je de naam toch wijzigen?",
                    "If you change the extension (the part after the dot), the file might not open any more. Change the name anyway?",
                ),
                &self.t("Ja, wijzigen", "Yes, change it"),
                &self.t("Nee", "No"),
                false,
                move |yes| {
                    let Some(d) = weak.upgrade() else { return };
                    if yes {
                        d.do_rename(it, value, old, from_blur);
                    } else {
                        if let Some(r) = d.s.borrow_mut().renaming.as_mut() {
                            r.busy = false;
                        }
                        d.finish_rename(true);
                    }
                },
            );
            return;
        }
        self.do_rename(it, value, old, from_blur);
    }

    fn do_rename(self: &Rc<Self>, it: Item, value: String, old: String, from_blur: bool) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            match super::call(&desk.shell, "files.rename", json!({"path": it.path, "newName": value})).await {
                Ok(res) => {
                    if res["dryRun"] == true {
                        desk.finish_rename(!from_blur);
                        desk.dry_run_toast(false);
                        return;
                    }
                    let dir = desk.s.borrow().dir.clone();
                    let new_path = res["path"].as_str().map(str::to_owned).unwrap_or_else(|| format!("{dir}/{value}"));
                    let new_key = base_of(&new_path);
                    desk.finish_rename(!from_blur);
                    desk.renamed_key(&old, &new_key);
                    let (np, ok, nk) = (new_path.clone(), old.clone(), new_key.clone());
                    desk.push_undo(
                        desk.t("Naam wijzigen ongedaan maken", "Undo rename"),
                        Rc::new(move |d: Rc<Desk>| {
                            let (np, ok, nk) = (np.clone(), ok.clone(), nk.clone());
                            glib::spawn_future_local(async move {
                                match super::call(&d.shell, "files.rename", json!({"path": np, "newName": ok})).await {
                                    Ok(_) => d.renamed_key(&nk, &ok),
                                    Err(e) => d.error_alert(Some(&e), None),
                                }
                                d.reload().await;
                            });
                        }),
                    );
                    desk.reload().await;
                    if desk.has(&new_key) {
                        desk.s.borrow_mut().anchor = Some(new_key.clone());
                        desk.set_selection(vec![new_key.clone()], Some(Some(new_key)));
                    }
                }
                Err(error) => {
                    let still = desk.s.borrow().renaming.is_some();
                    if let Some(r) = desk.s.borrow_mut().renaming.as_mut() {
                        r.busy = false;
                    }
                    if from_blur || !still {
                        desk.finish_rename(false);
                        let fallback = desk.t("De naam kon niet worden gewijzigd.", "The name couldn’t be changed.");
                        desk.error_alert(Some(&error), Some(&fallback));
                    } else {
                        desk.rename_tip(&error);
                    }
                }
            }
        });
    }
}

pub fn toggle(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on { widget.add_css_class(class) } else { widget.remove_css_class(class) }
}

pub fn first_free(occ: &HashSet<(i32, i32)>, cols: i32, rows: i32) -> (i32, i32) {
    for c in 0..cols + 200 {
        for r in 0..rows {
            if !occ.contains(&(c, r)) {
                return (c, r);
            }
        }
    }
    (0, 0)
}

pub fn nearest_free(c: i32, r: i32, occ: &HashSet<(i32, i32)>, cols: i32, rows: i32) -> (i32, i32) {
    let max_d = cols.max(rows) + 1;
    for d in 0..=max_d {
        let mut ring = Vec::new();
        for dc in -d..=d {
            for dr in -d..=d {
                if dc.abs().max(dr.abs()) != d {
                    continue;
                }
                let (cc, rr) = (c + dc, r + dr);
                if cc < 0 || rr < 0 || cc >= cols || rr >= rows {
                    continue;
                }
                ring.push((cc, rr, (dc * dc) as f64 * 1.2 + (dr * dr) as f64));
            }
        }
        ring.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(Ordering::Equal));
        if let Some((cc, rr, _)) = ring.into_iter().find(|(cc, rr, _)| !occ.contains(&(*cc, *rr))) {
            return (cc, rr);
        }
    }
    first_free(occ, cols, rows)
}

fn set_icon(image: &gtk::Image, texture: Option<&gdk::Texture>, fallback: &str) {
    match texture {
        Some(t) => image.set_paintable(Some(t)),
        None => image.set_icon_name(Some(fallback)),
    }
}

fn make_tile(it: &Item, icon: i32) -> Tile {
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("dt-item");
    root.set_widget_name(&format!("desktop-item-{}", it.key));
    let wrap = gtk::Overlay::new();
    wrap.add_css_class("dt-item__icon");
    wrap.set_halign(gtk::Align::Center);
    wrap.set_size_request(icon, icon);
    let image = gtk::Image::new();
    image.set_pixel_size(icon);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    wrap.set_child(Some(&image));
    if it.is_desktop_entry {
        wrap.add_overlay(&shortcut_badge(icon));
    }
    let label = gtk::Label::new(Some(&it.label));
    label.add_css_class("dt-item__label");
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label.set_justify(gtk::Justification::Center);
    label.set_xalign(0.5);
    label.set_lines(2);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_width_chars(1);
    label.set_max_width_chars(1);
    label.set_hexpand(true);
    root.append(&wrap);
    root.append(&label);
    Tile { root, image, label, sig: String::new() }
}

/// The shortcut arrow of a launcher (.desktop), like desktop.js shortcutBadge.
fn shortcut_badge(icon: i32) -> gtk::DrawingArea {
    let s = (icon / 3).max(12);
    let area = gtk::DrawingArea::new();
    area.set_content_width(s);
    area.set_content_height(s);
    area.set_halign(gtk::Align::Start);
    area.set_valign(gtk::Align::End);
    area.set_can_target(false);
    area.add_css_class("dt-item__shortcut");
    area.set_draw_func(|_, cr, w, _| {
        let k = w as f64 / 12.0;
        cr.scale(k, k);
        let (x, y, ww, hh, r) = (0.5, 0.5, 11.0, 11.0, 2.0);
        cr.new_sub_path();
        cr.arc(x + ww - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
        cr.arc(x + ww - r, y + hh - r, r, 0.0, std::f64::consts::FRAC_PI_2);
        cr.arc(x + r, y + hh - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
        cr.arc(x + r, y + r, r, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
        cr.close_path();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        let _ = cr.fill_preserve();
        cr.set_source_rgb(0.604, 0.643, 0.682);
        cr.set_line_width(0.8);
        let _ = cr.stroke();
        cr.set_source_rgb(0.122, 0.435, 0.816);
        cr.set_line_width(1.3);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        cr.move_to(3.4, 8.8);
        cr.curve_to(3.6, 6.4, 5.0, 5.1, 7.5, 5.1);
        let _ = cr.stroke();
        cr.move_to(6.2, 3.6);
        cr.line_to(8.2, 5.1);
        cr.line_to(6.4, 6.9);
        let _ = cr.stroke();
    });
    area
}

/// Vernieuwen (F5): the icons fade in from 25 % in 250 ms.
fn flash(widget: &gtk::Fixed) {
    widget.set_opacity(0.25);
    let started = Instant::now();
    let weak = widget.downgrade();
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(w) = weak.upgrade() else { return glib::ControlFlow::Break };
        let t = (started.elapsed().as_secs_f64() / 0.25).min(1.0);
        w.set_opacity(0.25 + 0.75 * t);
        if t >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}
