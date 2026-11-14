//! The taskbar's app buttons (taskbar.js computeGroups / render, shell.js app catalogue, pins and window store):
//! pinned apps (taskbar.pinned, the shipped browser pin shows the user's default browser) and running windows,
//! merged by desktop id (aliases: snap names, reverse-DNS tail, icon, name), unpinned running apps in first-seen
//! order. Icons: the catalogue's; for our own apps their desktop entry's (own_entry). Click: launch, focus or
//! minimize the one window, or the window list of a group; middle click: a new instance; right click: the jump
//! list (flyouts::menus). Hover: the window list after 450 ms. Drag: reorder the pins. Too many apps for the
//! room: a "..." button with the rest.
use super::util;
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

const PITCH: i32 = 44; // 40 px button + 4 px gap
const PREVIEW_DELAY: u64 = 450;
const SHIPPED_TASKBAR: [&[&str]; 2] = [
    &["win11shell-explorer.desktop", "firefox_firefox.desktop", "snap-store_snap-store.desktop", "win11shell-settings.desktop"],
    &["org.gnome.Nautilus.desktop", "firefox_firefox.desktop", "win11shell-settings.desktop"],
];
const SHIPPED_BROWSER: &str = "firefox_firefox.desktop";

#[derive(Clone)]
pub struct Group {
    pub key: String,
    pub app_id: Option<String>,
    pub pinned: bool,
    pub windows: Vec<Value>,
    pub name: String,
    pub icon: String,
}

struct AppButton {
    button: gtk::Button,
    image: gtk::Image,
}

#[derive(Default)]
struct Drag {
    key: String,
    start_x: f64,
    active: bool,
}

pub struct Apps {
    shell: Rc<Shell>,
    strip: gtk::Widget,
    pub boxw: gtk::Box,
    group_box: RefCell<Option<gtk::Widget>>,
    tray: RefCell<Option<gtk::Widget>>,
    // catalogue
    list: RefCell<Vec<Value>>,
    by_id: RefCell<HashMap<String, Value>>,
    alias: RefCell<HashMap<String, String>>,
    browser: RefCell<Option<String>>,
    catalog_ready: Cell<bool>,
    load_seq: Cell<u64>,
    load_timer: RefCell<Option<glib::SourceId>>,
    // windows and pins
    windows: RefCell<Vec<Value>>,
    pending_pins: RefCell<Option<(Vec<String>, Instant)>>,
    running_order: RefCell<Vec<String>>,
    groups: RefCell<Vec<Group>>,
    buttons: RefCell<HashMap<String, Rc<AppButton>>>,
    overflow_btn: gtk::Button,
    overflow_keys: RefCell<Vec<String>>,
    max_apps: Cell<usize>,
    layout_queued: Cell<bool>,
    render_timer: RefCell<Option<glib::SourceId>>,
    drag: RefCell<Option<Drag>>,
    shift: Cell<bool>,
    // window list (hover preview)
    preview: RefCell<Option<gtk::Popover>>,
    preview_key: RefCell<Option<String>>,
    preview_sticky: Cell<bool>,
    preview_sig: RefCell<String>,
    preview_closed: RefCell<(String, Option<Instant>)>,
    show_timer: RefCell<Option<glib::SourceId>>,
    hide_timer: RefCell<Option<glib::SourceId>>,
}

thread_local! {
    static APPS: RefCell<Option<Rc<Apps>>> = const { RefCell::new(None) };
}

fn with<R>(f: impl FnOnce(&Rc<Apps>) -> R) -> Option<R> {
    let a = APPS.with(|x| x.borrow().clone());
    a.map(|a| f(&a))
}

fn cancel(slot: &RefCell<Option<glib::SourceId>>) {
    if let Some(id) = slot.borrow_mut().take() {
        id.remove();
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_owned()
}

pub fn id_of(v: &Value) -> String {
    match v.get("id") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn norm(s: &str) -> String {
    let l = s.to_lowercase();
    l.strip_suffix(".desktop").map(str::to_owned).unwrap_or(l)
}

/// Our own apps (win11shell-settings.desktop, ...): the name after "win11shell-".
pub fn own_app(id: &str) -> Option<&'static str> {
    let base = id.strip_suffix(".desktop").unwrap_or(id);
    ["settings", "taskmgr", "explorer", "alles", "disks"]
        .into_iter()
        .find(|n| base == format!("win11shell-{n}"))
}

/// The desktop entry of one of our own apps in the catalogue (host/src/apps/catalog.rs: id
/// "win11shell-<kind>.desktop", `entry` the entry scripts/install.py writes for it, io.github.win11shell.<Name>
/// .desktop after the app's GApplication id = its windows' app id and StartupWMClass; the one table is host
/// launch.rs OWN_APP_IDS). Several are NoDisplay; their Icon= is what GNOME shows for those windows.
fn own_entry(app: &Value) -> Option<String> {
    (app["own"] == true).then(|| s(app, "entry")).filter(|entry| !entry.is_empty())
}

thread_local! {
    static ENTRY_ICONS: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Icon= of the desktop entry `<id>.desktop` (the first one in the XDG data dirs, the user's own first, as GIO
/// looks it up), NoDisplay entries included: GNOME's window tracker matches those too. None for a missing or
/// Hidden entry or one without an icon. Cached until the catalogue changes (apps.changed).
fn entry_icon(id: &str) -> Option<String> {
    let id = id.trim();
    if id.is_empty() || id.contains('/') || id.starts_with('.') {
        return None;
    }
    let file = if id.ends_with(".desktop") { id.to_owned() } else { format!("{id}.desktop") };
    if let Some(known) = ENTRY_ICONS.with(|c| c.borrow().get(&file).cloned()) {
        return known;
    }
    let icon = std::iter::once(glib::user_data_dir())
        .chain(glib::system_data_dirs())
        .map(|dir| dir.join("applications").join(&file))
        .find(|path| path.is_file())
        .and_then(|path| {
            let entry = glib::KeyFile::new();
            entry.load_from_file(&path, glib::KeyFileFlags::NONE).ok()?;
            if entry.boolean("Desktop Entry", "Hidden").unwrap_or(false) {
                return None;
            }
            let icon = entry.string("Desktop Entry", "Icon").ok()?;
            Some(icon.trim().to_owned()).filter(|i| !i.is_empty())
        });
    ENTRY_ICONS.with(|c| c.borrow_mut().insert(file, icon.clone()));
    icon
}

/// The icon of a window's own desktop entry: the extension's (GNOME's window tracker: the entry named by the
/// window's app id or StartupWMClass), else `<app id>.desktop` read here.
fn window_icon(w: &Value) -> Option<String> {
    Some(s(w, "icon")).filter(|i| !i.is_empty()).or_else(|| entry_icon(&s(w, "appId")))
}

/// "firefox_firefox" -> "firefox" (a snap's doubled name).
fn undouble(s: &str) -> String {
    let b = s.as_bytes();
    if b.len() >= 3 && b.len() % 2 == 1 {
        let mid = b.len() / 2;
        if b[mid] == b'_' && b[..mid] == b[mid + 1..] {
            return s[..mid].to_owned();
        }
    }
    s.to_owned()
}

fn guess_name(shell: &Shell, id: &str) -> String {
    if let Some(own) = own_app(id).filter(|_| id.ends_with(".desktop")) {
        return match own {
            "settings" => shell.t("Instellingen", "Settings"),
            "taskmgr" => shell.t("Taakbeheer", "Task Manager"),
            "explorer" => shell.t("Verkenner", "File Explorer"),
            "alles" => "Alles".into(),
            _ => shell.t("Schijven", "Disks"),
        };
    }
    let base = undouble(id.strip_suffix(".desktop").unwrap_or(id));
    let tail = base.rsplit('.').next().unwrap_or(&base).to_owned();
    let mut c = tail.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => tail,
    }
}

fn guess_icon(id: &str) -> String {
    undouble(id.strip_suffix(".desktop").unwrap_or(id))
}

fn name_from_window(shell: &Shell, w: &Value) -> String {
    let title = s(w, "title");
    let mut tail: Option<&str> = None;
    for sep in [" \u{2014} ", " \u{2013} ", " - "] {
        if let Some(i) = title.rfind(sep) {
            let t = &title[i + sep.len()..];
            if tail.is_none_or(|old| t.len() < old.len()) {
                tail = Some(t);
            }
        }
    }
    if let Some(t) = tail
        && t != title
        && t.chars().count() < 32
        && !t.trim().is_empty()
    {
        return t.trim().to_owned();
    }
    let app_id = s(w, "appId");
    let seed = if !app_id.is_empty() { app_id } else if !title.is_empty() { title } else { "App".into() };
    guess_name(shell, &seed)
}

/// "Recept - Mozilla Firefox" -> "Recept" when the group already says Firefox.
fn clean_title(title: &str, app_name: &str) -> String {
    let name = app_name.trim();
    if name.is_empty() {
        return title.to_owned();
    }
    let pattern = format!("\\s+[\u{2014}\u{2013}-]\\s+(?:mozilla\\s+)?{}$", glib::Regex::escape_string(name));
    let out = glib::Regex::new(&pattern, glib::RegexCompileFlags::CASELESS, glib::RegexMatchFlags::empty())
        .ok()
        .flatten()
        .and_then(|re| re.replace(title, 0, "", glib::RegexMatchFlags::empty()).ok())
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    if out.is_empty() { title.to_owned() } else { out }
}

/// What the window list shows of a group (it is rebuilt only when this changes).
fn preview_sig(g: &Group) -> String {
    let mut out = format!("{}|{}", g.name, g.icon);
    for w in &g.windows {
        out.push_str(&format!("|{}:{}:{}:{}", id_of(w), s(w, "title"), is_focused(w), w["minimized"] == true));
    }
    out
}

fn is_focused(w: &Value) -> bool {
    w["focused"] == true && w["minimized"] != true
}

pub fn build(shell: &Rc<Shell>, strip: &gtk::Widget) -> gtk::Box {
    let boxw = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    boxw.add_css_class("w11-tb-apps");
    boxw.set_valign(gtk::Align::Center);

    let overflow_btn = gtk::Button::new();
    overflow_btn.add_css_class("w11-tb-btn");
    overflow_btn.add_css_class("w11-tb-overflow");
    overflow_btn.add_css_class("w11-tb-hit");
    overflow_btn.set_widget_name("tb-more");
    overflow_btn.set_child(Some(&crate::icons::glyph("more", 20)));
    overflow_btn.set_visible(false);
    overflow_btn.connect_clicked(|b| {
        let b = b.clone();
        with(move |a| a.open_overflow(&b));
    });

    let apps = Rc::new(Apps {
        shell: shell.clone(),
        strip: strip.clone(),
        boxw: boxw.clone(),
        group_box: RefCell::new(None),
        tray: RefCell::new(None),
        list: RefCell::new(Vec::new()),
        by_id: RefCell::new(HashMap::new()),
        alias: RefCell::new(HashMap::new()),
        browser: RefCell::new(None),
        catalog_ready: Cell::new(false),
        load_seq: Cell::new(0),
        load_timer: RefCell::new(None),
        windows: RefCell::new(Vec::new()),
        pending_pins: RefCell::new(None),
        running_order: RefCell::new(Vec::new()),
        groups: RefCell::new(Vec::new()),
        buttons: RefCell::new(HashMap::new()),
        overflow_btn,
        overflow_keys: RefCell::new(Vec::new()),
        max_apps: Cell::new(usize::MAX),
        layout_queued: Cell::new(false),
        render_timer: RefCell::new(None),
        drag: RefCell::new(None),
        shift: Cell::new(false),
        preview: RefCell::new(None),
        preview_key: RefCell::new(None),
        preview_sticky: Cell::new(false),
        preview_sig: RefCell::new(String::new()),
        preview_closed: RefCell::new((String::new(), None)),
        show_timer: RefCell::new(None),
        hide_timer: RefCell::new(None),
    });
    APPS.with(|x| *x.borrow_mut() = Some(apps.clone()));
    update_texts();

    // the catalogue (apps.list + apps.defaults); windows group under their app once it is there (or after 1.5 s)
    load_catalog();
    glib::timeout_add_local_once(Duration::from_millis(1500), || {
        with(|a| {
            if !a.catalog_ready.get() {
                a.catalog_ready.set(true);
                a.render();
            }
        });
    });
    shell.on("apps.changed", |_, _| {
        ENTRY_ICONS.with(|c| c.borrow_mut().clear());
        with(|a| {
            cancel(&a.load_timer);
            *a.load_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(400), || {
                with(|a| *a.load_timer.borrow_mut() = None);
                load_catalog();
            }));
        });
    });

    // the window list: the extension's windows.list, windows.changed and windows.focused
    *apps.windows.borrow_mut() = shell.gnome.windows();
    shell.on("windows.changed", |_, data| {
        with(|a| {
            *a.windows.borrow_mut() = data["windows"].as_array().cloned().unwrap_or_default();
            a.render_soon();
        });
    });
    shell.on("windows.focused", |_, data| {
        let focused = match &data["id"] {
            Value::String(s) => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            _ => None,
        };
        with(|a| {
            for w in a.windows.borrow_mut().iter_mut() {
                let on = focused.as_deref().is_some_and(|f| f == id_of(w));
                w["focused"] = Value::Bool(on);
            }
            a.render_soon();
        });
    });
    boxw
}

/// The centre group and the tray, for the room the buttons have (taskbar.js layout()).
pub fn set_layout_widgets(group: &gtk::Widget, tray: &gtk::Widget) {
    with(|a| {
        *a.group_box.borrow_mut() = Some(group.clone());
        *a.tray.borrow_mut() = Some(tray.clone());
    });
}

/// The strip's width changed, or the tray did: check how many buttons fit.
pub fn queue_layout() {
    with(|a| a.queue_layout());
}

/// Settings changed (pins, language).
pub fn settings_changed(lang_changed: bool) {
    with(|a| {
        let done = {
            let pending = a.pending_pins.borrow();
            pending.as_ref().is_some_and(|(ids, at)| {
                let now = a.shell.setting("taskbar.pinned");
                let raw: Vec<String> =
                    now.as_array().map(|l| l.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                &raw == ids || at.elapsed() > Duration::from_secs(3)
            })
        };
        if done {
            *a.pending_pins.borrow_mut() = None;
        }
        if lang_changed {
            update_texts();
            load_catalog();
        }
        a.render();
    });
}

pub fn flyout_opened() {
    with(|a| a.hide_preview());
}

fn update_texts() {
    with(|a| {
        a.overflow_btn.set_tooltip_text(Some(&a.shell.t("Meer apps", "More apps")));
    });
}

fn load_catalog() {
    let Some((shell, seq)) = with(|a| {
        a.load_seq.set(a.load_seq.get() + 1);
        (a.shell.clone(), a.load_seq.get())
    }) else {
        return;
    };
    shell.backend.set_lang(shell.lang());
    let s2 = shell.clone();
    shell.call("apps.defaults", json!({}), move |defaults| {
        s2.call("apps.list", json!({}), move |list| {
            with(|a| {
                if a.load_seq.get() != seq {
                    return;
                }
                if let Ok(d) = &defaults {
                    *a.browser.borrow_mut() = d["browser"]["id"].as_str().map(str::to_owned);
                }
                match list {
                    Ok(Value::Array(l)) => {
                        let l: Vec<Value> = l
                            .into_iter()
                            .filter(|x| x.get("id").and_then(|i| i.as_str()).is_some_and(|i| !i.is_empty()) && x["noDisplay"] != true)
                            .collect();
                        *a.list.borrow_mut() = l;
                    }
                    Ok(_) => *a.list.borrow_mut() = Vec::new(),
                    Err(e) => eprintln!("win11-shell: apps.list: {e}"),
                }
                a.index();
                a.catalog_ready.set(true);
                a.render();
            });
        });
    });
}

impl Apps {
    fn index(&self) {
        let list = self.list.borrow();
        let mut by_id = HashMap::new();
        let mut alias: HashMap<String, String> = HashMap::new();
        let mut put = |k: String, id: &str| {
            if !k.is_empty() && !alias.contains_key(&k) {
                alias.insert(k, id.to_owned());
            }
        };
        for a in list.iter() {
            let id = s(a, "id");
            by_id.insert(id.clone(), a.clone());
            put(norm(&id), &id);
        }
        for a in list.iter() {
            let id = s(a, "id");
            let n = norm(&id);
            if let Some((x, y)) = n.split_once('_')
                && !x.is_empty()
                && !y.is_empty()
            {
                put(x.to_owned(), &id);
                put(y.to_owned(), &id);
            }
            put(n.rsplit('.').next().unwrap_or("").to_owned(), &id);
            for p in ["org.", "com.", "io.", "net.", "dev."] {
                if let Some(rest) = n.strip_prefix(p)
                    && let Some((_, tail)) = rest.split_once('.')
                {
                    put(tail.to_owned(), &id);
                }
            }
            let icon = norm(&s(a, "icon"));
            put(icon.rsplit('/').next().unwrap_or("").to_owned(), &id);
            put(s(a, "name").to_lowercase(), &id);
        }
        drop(list);
        *self.by_id.borrow_mut() = by_id;
        *self.alias.borrow_mut() = alias;
    }

    fn app(&self, id: &str) -> Option<Value> {
        self.by_id.borrow().get(id).cloned()
    }

    /// Best app for a window: exact desktop id, then aliases.
    fn resolve(&self, desktop_id: &str, app_id: &str) -> Option<Value> {
        if !desktop_id.is_empty()
            && let Some(a) = self.app(desktop_id)
        {
            return Some(a);
        }
        let alias = self.alias.borrow();
        for k in [desktop_id, app_id] {
            if k.is_empty() {
                continue;
            }
            let n = norm(k);
            if let Some(id) = alias.get(&n) {
                return self.app(id);
            }
            let tail = n.rsplit('.').next().unwrap_or("");
            if let Some(id) = alias.get(tail) {
                return self.app(id);
            }
        }
        None
    }

    fn default_browser(&self) -> Option<String> {
        self.browser.borrow().clone().filter(|b| self.by_id.borrow().contains_key(b))
    }

    /// taskbar.pinned as shown: while it is the shipped list, the browser pin is the user's default browser.
    pub fn pins(&self) -> Vec<String> {
        if let Some((ids, _)) = self.pending_pins.borrow().as_ref() {
            return ids.clone();
        }
        let v = self.shell.setting("taskbar.pinned");
        let ids: Vec<String> = v.as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        if !SHIPPED_TASKBAR.iter().any(|d| d.len() == ids.len() && d.iter().zip(&ids).all(|(a, b)| a == b)) {
            return ids;
        }
        let Some(b) = self.default_browser().filter(|b| b != SHIPPED_BROWSER) else { return ids };
        if ids.contains(&b) {
            ids.into_iter().filter(|x| x != SHIPPED_BROWSER).collect()
        } else {
            ids.into_iter().map(|x| if x == SHIPPED_BROWSER { b.clone() } else { x }).collect()
        }
    }

    fn set_pins(self: &Rc<Self>, ids: Vec<String>) {
        let mut seen = std::collections::HashSet::new();
        let ids: Vec<String> = ids.into_iter().filter(|x| seen.insert(x.clone())).collect();
        self.shell.set_setting("taskbar.pinned", json!(ids));
        *self.pending_pins.borrow_mut() = Some((ids, Instant::now()));
        self.render();
    }

    fn compute_groups(&self) -> Vec<Group> {
        let no_catalog = self.list.borrow().is_empty();
        let mut out: Vec<Group> = Vec::new();
        for id in self.pins() {
            if out.iter().any(|g| g.key == id) {
                continue;
            }
            let app = self.app(&id);
            let own = id.ends_with(".desktop") && own_app(&id).is_some();
            if app.is_none() && (!no_catalog || !own) {
                continue;
            }
            let name = app.as_ref().map(|a| s(a, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| guess_name(&self.shell, &id));
            // our own apps: the icon of their desktop entry, as their windows have it
            let icon = app
                .as_ref()
                .and_then(own_entry)
                .and_then(|entry| entry_icon(&entry))
                .or_else(|| app.as_ref().map(|a| s(a, "icon")).filter(|n| !n.is_empty()))
                .unwrap_or_else(|| guess_icon(&id));
            out.push(Group { key: id.clone(), app_id: Some(id), pinned: true, windows: Vec::new(), name, icon });
        }
        for w in self.windows.borrow().iter() {
            let app = self.resolve(&s(w, "desktopId"), &s(w, "appId"));
            let key = match &app {
                Some(a) => s(a, "id"),
                None => {
                    let seed = [s(w, "appId"), s(w, "icon"), s(w, "title"), format!("h:{}", id_of(w))]
                        .into_iter()
                        .find(|x| !x.is_empty())
                        .unwrap_or_default();
                    format!("win:{seed}")
                }
            };
            if let Some(g) = out.iter_mut().find(|g| g.key == key) {
                g.windows.push(w.clone());
                continue;
            }
            let name = app.as_ref().map(|a| s(a, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| name_from_window(&self.shell, w));
            let app_icon = app.as_ref().map(|a| s(a, "icon")).filter(|n| !n.is_empty());
            // The window of one of our apps resolves to the catalogue's own app (the extension names it
            // win11shell-<kind>.desktop); the window's own desktop entry (io.github.win11shell.<Name>.desktop,
            // NoDisplay or not, a path icon included) is what GNOME shows for it, and the catalogue's icon only when
            // that entry has none. Other apps: the catalogue's icon, then the window's entry.
            let icon = match app.as_ref().and_then(own_entry) {
                Some(entry) => window_icon(w).or_else(|| entry_icon(&entry)).or(app_icon),
                None => app_icon.or_else(|| window_icon(w)),
            }
            .unwrap_or_else(|| s(w, "appId"));
            out.push(Group {
                key,
                app_id: app.as_ref().map(|a| s(a, "id")),
                pinned: false,
                windows: vec![w.clone()],
                name,
                icon,
            });
        }
        // stable order for unpinned running apps
        let unpinned: Vec<String> = out.iter().filter(|g| !g.pinned).map(|g| g.key.clone()).collect();
        let mut order = self.running_order.borrow_mut();
        order.retain(|k| unpinned.contains(k));
        for k in &unpinned {
            if !order.contains(k) {
                order.push(k.clone());
            }
        }
        let mut ordered: Vec<Group> = out.iter().filter(|g| g.pinned).cloned().collect();
        for k in order.iter() {
            if let Some(g) = out.iter().find(|g| &g.key == k) {
                ordered.push(g.clone());
            }
        }
        ordered
    }

    fn group(&self, key: &str) -> Option<Group> {
        self.groups.borrow().iter().find(|g| g.key == key).cloned()
    }

    fn render_soon(self: &Rc<Self>) {
        if self.render_timer.borrow().is_some() {
            return;
        }
        *self.render_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(16), || {
            with(|a| {
                *a.render_timer.borrow_mut() = None;
                a.render();
            });
        }));
    }

    pub fn render(self: &Rc<Self>) {
        if self.drag.borrow().as_ref().is_some_and(|d| d.active) {
            return; // never reshuffle under the pointer
        }
        if !self.catalog_ready.get() {
            return; // wait for apps.list so windows group under the right app
        }
        let groups = self.compute_groups();
        let max = self.max_apps.get();
        let (visible, overflow): (Vec<Group>, Vec<String>) = if groups.len() > max {
            let n = max.saturating_sub(1);
            (groups[..n].to_vec(), groups[n..].iter().map(|g| g.key.clone()).collect())
        } else {
            (groups.clone(), Vec::new())
        };
        *self.groups.borrow_mut() = groups;
        *self.overflow_keys.borrow_mut() = overflow.clone();

        // remove stale buttons
        let wanted: Vec<&str> = visible.iter().map(|g| g.key.as_str()).collect();
        let stale: Vec<String> = self.buttons.borrow().keys().filter(|k| !wanted.contains(&k.as_str())).cloned().collect();
        for k in stale {
            let b = self.buttons.borrow_mut().remove(&k);
            if let Some(b) = b {
                self.boxw.remove(&b.button);
            }
        }
        let mut prev: Option<gtk::Widget> = None;
        for g in &visible {
            let existing = self.buttons.borrow().get(&g.key).cloned();
            let btn = match existing {
                Some(b) => b,
                None => {
                    let b = self.make_button(&g.key);
                    self.boxw.append(&b.button);
                    util::flash_class(&b.button, "is-entering", 260);
                    self.buttons.borrow_mut().insert(g.key.clone(), b.clone());
                    b
                }
            };
            self.update_button(&btn, g);
            self.boxw.reorder_child_after(&btn.button, prev.as_ref());
            prev = Some(btn.button.clone().upcast());
        }
        if overflow.is_empty() {
            self.overflow_btn.set_visible(false);
        } else {
            if self.overflow_btn.parent().is_none() {
                self.boxw.append(&self.overflow_btn);
            }
            self.overflow_btn.set_visible(true);
            self.boxw.reorder_child_after(&self.overflow_btn, prev.as_ref());
        }
        // the window list follows its group
        let pk = self.preview_key.borrow().clone();
        if let Some(key) = pk {
            let g = self.group(&key).filter(|g| !g.windows.is_empty());
            let has_button = self.buttons.borrow().contains_key(&key);
            if let Some(g) = g.filter(|_| has_button) {
                if preview_sig(&g) != *self.preview_sig.borrow() {
                    self.fill_preview(&key);
                }
            } else {
                self.hide_preview();
            }
        }
        self.queue_layout();
    }

    fn queue_layout(self: &Rc<Self>) {
        if self.layout_queued.replace(true) {
            return;
        }
        glib::idle_add_local_once(|| {
            with(|a| {
                a.layout_queued.set(false);
                a.layout();
            });
        });
    }

    /// How many buttons fit: the same clearance on both sides, so the group never leaves the screen's centre.
    fn layout(self: &Rc<Self>) {
        let width = self.strip.width();
        if width <= 0 {
            return;
        }
        let tray_w = self.tray.borrow().as_ref().map(|t| t.width()).unwrap_or(0);
        let group_w = self.group_box.borrow().as_ref().map(|g| g.width()).unwrap_or(0);
        let right_reserve = tray_w + 16;
        let avail = PITCH.max(width - 2 * right_reserve);
        let fixed = (group_w - self.boxw.width()).max(0);
        let cap = ((avail - fixed + 4) / PITCH).max(1) as usize;
        let n = self.groups.borrow().len();
        let old = self.max_apps.get();
        self.max_apps.set(cap);
        if cap != old && (n > cap || old < n) {
            self.render();
        }
    }

    fn make_button(self: &Rc<Self>, key: &str) -> Rc<AppButton> {
        let button = gtk::Button::new();
        button.add_css_class("w11-tb-btn");
        button.add_css_class("w11-tb-app");
        button.add_css_class("w11-tb-hit");
        button.set_widget_name(&format!("tb-app-{key}"));
        let overlay = gtk::Overlay::new();
        let image = gtk::Image::new();
        image.add_css_class("w11-tb-ico");
        image.set_pixel_size(24);
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        overlay.set_child(Some(&image));
        let ind = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ind.add_css_class("w11-tb-ind");
        ind.set_halign(gtk::Align::Center);
        ind.set_valign(gtk::Align::End);
        ind.set_can_target(false);
        overlay.add_overlay(&ind);
        button.set_child(Some(&overlay));

        let k = key.to_owned();
        button.connect_clicked(move |_| {
            let k = k.clone();
            with(move |a| {
                let shift = a.shift.replace(false);
                a.activate(&k, shift);
            });
        });
        // Shift+click: a new instance (the state of the press)
        let press = gtk::GestureClick::new();
        press.set_button(gdk::BUTTON_PRIMARY);
        press.set_propagation_phase(gtk::PropagationPhase::Capture);
        press.connect_pressed(|g, _, _, _| {
            let shift = g.current_event_state().contains(gdk::ModifierType::SHIFT_MASK);
            with(|a| a.shift.set(shift));
        });
        button.add_controller(press);
        // middle click: a new instance
        let middle = gtk::GestureClick::new();
        middle.set_button(gdk::BUTTON_MIDDLE);
        let k = key.to_owned();
        middle.connect_released(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            with(|a| {
                if let Some(id) = a.group(&k).and_then(|g| g.app_id) {
                    a.launch(&id);
                }
            });
        });
        button.add_controller(middle);
        // right click: the jump list
        let right = gtk::GestureClick::new();
        right.set_button(gdk::BUTTON_SECONDARY);
        let k = key.to_owned();
        right.connect_pressed(move |g, _, _, _| {
            g.set_state(gtk::EventSequenceState::Claimed);
            with(|a| a.open_jump_list(&k));
        });
        button.add_controller(right);
        // hover: the window list
        let motion = gtk::EventControllerMotion::new();
        let k = key.to_owned();
        motion.connect_enter(move |_, _, _| {
            with(|a| a.schedule_preview(&k));
        });
        motion.connect_leave(|_| {
            with(|a| a.schedule_preview_hide());
        });
        button.add_controller(motion);
        // drag: reorder
        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        drag.set_propagation_phase(gtk::PropagationPhase::Capture);
        let k = key.to_owned();
        drag.connect_drag_begin(move |g, x, y| {
            let Some(w) = g.widget() else { return };
            with(|a| {
                let start_x = w
                    .compute_point(&a.boxw, &gtk::graphene::Point::new(x as f32, y as f32))
                    .map(|p| p.x() as f64)
                    .unwrap_or(x);
                *a.drag.borrow_mut() = Some(Drag { key: k.clone(), start_x, active: false });
            });
        });
        drag.connect_drag_update(|g, dx, dy| {
            with(|a| a.drag_update(g, dx, dy));
        });
        drag.connect_drag_end(|_, _, _| {
            with(|a| a.drag_end());
        });
        button.add_controller(drag);

        Rc::new(AppButton { button, image })
    }

    fn update_button(&self, btn: &AppButton, g: &Group) {
        util::set_icon(&self.shell, &btn.image, &g.icon, 24, true);
        let running = !g.windows.is_empty();
        let focused = g.windows.iter().any(is_focused);
        util::set_class(&btn.button, "is-running", running);
        util::set_class(&btn.button, "is-focused", focused);
        util::set_class(&btn.button, "is-multi", g.windows.len() > 1);
        util::set_class(&btn.button, "is-pinned", g.pinned);
        if running {
            let n = g.windows.len();
            let count = if n == 1 {
                self.shell.t("1 venster", "1 window")
            } else {
                self.shell.t(&format!("{n} vensters"), &format!("{n} windows"))
            };
            let label = self.shell.t(&format!("{}, {count} geopend", g.name), &format!("{}, {count} open", g.name));
            btn.button.set_tooltip_text(Some(&label));
            // the window list takes the tooltip's place
            btn.button.set_has_tooltip(false);
        } else {
            btn.button.set_tooltip_text(Some(&g.name));
        }
    }

    /* ----------------------------------------------------------- actions */

    pub fn launch(self: &Rc<Self>, id: &str) {
        if let Some(own) = own_app(id) {
            util::open_app(&self.shell, own);
        } else {
            let id2 = id.to_owned();
            let name = self.app(id).map(|a| s(&a, "name")).unwrap_or_else(|| id.to_owned());
            self.shell.call("apps.launch", json!({"id": id2}), move |r| {
                if let Err(e) = r {
                    eprintln!("win11-shell: {name} kon niet worden gestart: {e}");
                }
            });
        }
        if let Some(b) = self.buttons.borrow().get(id) {
            util::flash_class(&b.image, "is-bounce", 740);
        }
    }

    fn focus(self: &Rc<Self>, w: &Value) {
        let id = id_of(w);
        for x in self.windows.borrow_mut().iter_mut() {
            let me = id_of(x) == id;
            x["focused"] = Value::Bool(me);
            if me {
                x["minimized"] = Value::Bool(false);
            }
        }
        self.shell.gnome.send("windows.focus", json!({"id": id}));
        self.render();
    }

    fn minimize(self: &Rc<Self>, w: &Value) {
        let id = id_of(w);
        for x in self.windows.borrow_mut().iter_mut() {
            if id_of(x) == id {
                x["minimized"] = Value::Bool(true);
                x["focused"] = Value::Bool(false);
            }
        }
        self.shell.gnome.send("windows.minimize", json!({"id": id}));
        self.render();
    }

    fn close_window(&self, w: &Value) {
        self.shell.gnome.send("windows.close", json!({"id": id_of(w)}));
    }

    fn activate(self: &Rc<Self>, key: &str, shift: bool) {
        let Some(g) = self.group(key) else { return };
        if g.windows.is_empty() {
            self.hide_preview();
            if let Some(id) = &g.app_id {
                self.launch(id);
            }
            return;
        }
        if shift && let Some(id) = &g.app_id {
            self.launch(id);
            return;
        }
        if g.windows.len() == 1 {
            self.hide_preview();
            let w = &g.windows[0];
            if is_focused(w) { self.minimize(w) } else { self.focus(w) }
            return;
        }
        // several windows: the window list
        let open_here = self.preview_key.borrow().as_deref() == Some(key);
        if open_here && self.preview_sticky.get() {
            self.hide_preview();
            return;
        }
        let just_closed = {
            let c = self.preview_closed.borrow();
            c.0 == key && c.1.is_some_and(|t| t.elapsed() < Duration::from_millis(350))
        };
        if just_closed {
            return; // the press that closed the list (autohide) was this click
        }
        self.show_preview(key, true);
    }

    fn open_jump_list(self: &Rc<Self>, key: &str) {
        let Some(g) = self.group(key) else { return };
        self.hide_preview();
        let x = self.buttons.borrow().get(key).map(|b| util::window_point(&b.button, b.button.width() as f64 / 2.0, 0.0).0);
        let Some(x) = x else { return };
        crate::flyouts::menus::jump_list(&self.shell, g.app_id.clone(), g.windows.clone(), x);
    }

    fn open_overflow(self: &Rc<Self>, anchor: &gtk::Button) {
        self.hide_preview();
        let keys = self.overflow_keys.borrow().clone();
        let popover = gtk::Popover::new();
        popover.add_css_class("w11-tb-menu");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 2);
        list.add_css_class("w11-tb-overflow-list");
        for k in keys {
            let Some(g) = self.group(&k) else { continue };
            let row = gtk::Button::new();
            row.add_css_class("w11-tb-menu-item");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let image = gtk::Image::new();
            util::set_icon(&self.shell, &image, &g.icon, 16, true);
            line.append(&image);
            let label = if g.windows.is_empty() { g.name.clone() } else { format!("{} ({})", g.name, g.windows.len()) };
            let l = gtk::Label::new(Some(&label));
            l.set_xalign(0.0);
            line.append(&l);
            row.set_child(Some(&line));
            let p = popover.downgrade();
            row.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.popdown();
                }
                let k = k.clone();
                with(move |a| a.activate(&k, false));
            });
            list.append(&row);
        }
        popover.set_child(Some(&list));
        let x = util::center_x(anchor, &self.strip);
        util::show_popover(&self.strip, &popover, x);
    }

    /* ---------------------------------------------------------- dragging */

    fn drag_update(self: &Rc<Self>, gesture: &gtk::GestureDrag, dx: f64, dy: f64) {
        let (key, start_x, active) = match self.drag.borrow().as_ref() {
            Some(d) => (d.key.clone(), d.start_x, d.active),
            None => return,
        };
        if !active {
            if dx.abs() < 6.0 && dy.abs() < 6.0 {
                return;
            }
            if let Some(d) = self.drag.borrow_mut().as_mut() {
                d.active = true;
            }
            gesture.set_state(gtk::EventSequenceState::Claimed);
            self.hide_preview();
            if let Some(b) = self.buttons.borrow().get(&key) {
                b.button.add_css_class("is-dragging");
            }
        }
        let Some(me) = self.buttons.borrow().get(&key).map(|b| b.button.clone()) else { return };
        let x = start_x + dx;
        let others: Vec<gtk::Button> = self.ordered_buttons().into_iter().filter(|b| b != &me).collect();
        let mut index = others.len();
        for (i, b) in others.iter().enumerate() {
            if let Some(r) = b.compute_bounds(&self.boxw)
                && x < (r.x() + r.width() / 2.0) as f64
            {
                index = i;
                break;
            }
        }
        let current = self.ordered_buttons().iter().position(|b| b == &me).unwrap_or(0);
        if index != current {
            let after = if index == 0 { None } else { others.get(index - 1).cloned() };
            self.boxw.reorder_child_after(&me, after.as_ref());
        }
    }

    fn ordered_buttons(&self) -> Vec<gtk::Button> {
        let map = self.buttons.borrow();
        let mut out = Vec::new();
        let mut child = self.boxw.first_child();
        while let Some(c) = child {
            if let Ok(b) = c.clone().downcast::<gtk::Button>()
                && map.values().any(|x| x.button == b)
            {
                out.push(b);
            }
            child = c.next_sibling();
        }
        out
    }

    fn drag_end(self: &Rc<Self>) {
        let Some(d) = self.drag.borrow_mut().take() else { return };
        if !d.active {
            return;
        }
        if let Some(b) = self.buttons.borrow().get(&d.key) {
            b.button.remove_css_class("is-dragging");
        }
        let keys: Vec<String> = {
            let map = self.buttons.borrow();
            self.ordered_buttons()
                .iter()
                .filter_map(|b| map.iter().find(|(_, x)| &x.button == b).map(|(k, _)| k.clone()))
                .collect()
        };
        let pinned_of = |k: &String| self.group(k).is_some_and(|g| g.pinned);
        let pins = self.pins();
        let visible_pinned: Vec<String> = keys.iter().filter(|k| pinned_of(k)).cloned().collect();
        let hidden_pinned: Vec<String> = pins.iter().filter(|k| !visible_pinned.contains(k)).cloned().collect();
        let next: Vec<String> = visible_pinned.into_iter().chain(hidden_pinned).collect();
        {
            let mut order = self.running_order.borrow_mut();
            let unpinned: Vec<String> = keys.iter().filter(|k| !pinned_of(k)).cloned().collect();
            let rest: Vec<String> = order.iter().filter(|k| !keys.contains(k)).cloned().collect();
            *order = unpinned.into_iter().chain(rest).collect();
        }
        if next != pins { self.set_pins(next) } else { self.render() }
    }

    /* ------------------------------------------------------ window list */

    fn schedule_preview(self: &Rc<Self>, key: &str) {
        cancel(&self.hide_timer);
        if self.drag.borrow().as_ref().is_some_and(|d| d.active) {
            return;
        }
        if self.shell.flyouts.borrow().as_ref().is_some_and(|f| f.current().is_some()) {
            return;
        }
        let has_windows = self.group(key).is_some_and(|g| !g.windows.is_empty());
        if !has_windows {
            if self.preview.borrow().is_some() && !self.preview_sticky.get() {
                self.hide_preview();
            }
            return;
        }
        if self.preview_key.borrow().as_deref() == Some(key) {
            return;
        }
        cancel(&self.show_timer);
        let delay = if self.preview.borrow().is_some() { 60 } else { PREVIEW_DELAY };
        let k = key.to_owned();
        *self.show_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            with(|a| {
                *a.show_timer.borrow_mut() = None;
                let sticky = a.preview_sticky.get();
                a.show_preview(&k, sticky);
            });
        }));
    }

    fn schedule_preview_hide(self: &Rc<Self>) {
        cancel(&self.show_timer);
        cancel(&self.hide_timer);
        if self.preview.borrow().is_none() {
            return;
        }
        let ms = if self.preview_sticky.get() { 900 } else { 380 };
        *self.hide_timer.borrow_mut() = Some(glib::timeout_add_local_once(Duration::from_millis(ms), || {
            with(|a| {
                *a.hide_timer.borrow_mut() = None;
                a.hide_preview();
            });
        }));
    }

    fn show_preview(self: &Rc<Self>, key: &str, sticky: bool) {
        let Some(g) = self.group(key) else { return };
        if g.windows.is_empty() {
            return;
        }
        let Some(btn) = self.buttons.borrow().get(key).map(|b| b.button.clone()) else { return };
        cancel(&self.show_timer);
        cancel(&self.hide_timer);
        // a hover list that becomes sticky is opened again with autohide (a click elsewhere closes it)
        let existing = self.preview.borrow().clone();
        let popover = match existing {
            Some(p) if p.is_autohide() == sticky => p,
            other => {
                if let Some(p) = other {
                    p.popdown();
                }
                let p = gtk::Popover::new();
                p.add_css_class("w11-tb-preview");
                p.set_autohide(sticky);
                p.set_has_arrow(false);
                p.set_position(gtk::PositionType::Top);
                p.set_parent(&self.strip);
                let weak = p.downgrade();
                p.connect_closed(move |p| {
                    let me = p.clone();
                    with(|a| {
                        let is_current = a.preview.borrow().as_ref().is_some_and(|x| x == &me);
                        if is_current {
                            let key = a.preview_key.borrow_mut().take().unwrap_or_default();
                            *a.preview_closed.borrow_mut() = (key, Some(Instant::now()));
                            *a.preview.borrow_mut() = None;
                            a.preview_sticky.set(false);
                        }
                    });
                    let weak = weak.clone();
                    glib::idle_add_local_once(move || {
                        if let Some(p) = weak.upgrade() {
                            p.unparent();
                        }
                    });
                });
                *self.preview.borrow_mut() = Some(p.clone());
                p
            }
        };
        *self.preview_key.borrow_mut() = Some(key.to_owned());
        self.preview_sticky.set(sticky);
        self.fill_preview(key);
        if let Some(r) = util::bounds(&btn, &self.strip) {
            popover.set_pointing_to(Some(&gdk::Rectangle::new(r.x(), 0, r.width(), 1)));
        }
        if !popover.is_visible() {
            popover.popup();
        }
    }

    fn fill_preview(self: &Rc<Self>, key: &str) {
        let Some(g) = self.group(key) else { return };
        let Some(popover) = self.preview.borrow().clone() else { return };
        *self.preview_sig.borrow_mut() = preview_sig(&g);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
        content.add_css_class("w11-tb-pv");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        head.add_css_class("w11-tb-pv-head");
        let icon = gtk::Image::new();
        util::set_icon(&self.shell, &icon, &g.icon, 16, true);
        head.append(&icon);
        let name = gtk::Label::new(Some(&g.name));
        name.add_css_class("w11-tb-pv-app");
        name.set_xalign(0.0);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        head.append(&name);
        content.append(&head);
        let close_text = self.shell.t("Sluiten", "Close");
        for w in &g.windows {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            row.add_css_class("w11-tb-pv-item");
            row.set_widget_name(&format!("tb-pv-{}", id_of(w)));
            if is_focused(w) {
                row.add_css_class("is-current");
            }
            if w["minimized"] == true {
                row.add_css_class("is-minimized");
            }
            let raw = s(w, "title");
            let mut title = clean_title(&raw, &g.name);
            if title.is_empty() {
                title = g.name.clone();
            }
            let label = gtk::Label::new(Some(&title));
            label.add_css_class("w11-tb-pv-title");
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            label.set_max_width_chars(32);
            label.set_width_chars(20);
            if !raw.is_empty() {
                label.set_tooltip_text(Some(&raw));
            }
            row.append(&label);
            let close = gtk::Button::new();
            close.add_css_class("w11-tb-pv-close");
            close.set_child(Some(&crate::icons::glyph("close", 12)));
            close.set_tooltip_text(Some(&close_text));
            close.set_focus_on_click(false);
            let win = w.clone();
            close.connect_clicked(move |b| {
                if let Some(row) = b.parent() {
                    row.add_css_class("is-closing");
                }
                with(|a| a.close_window(&win));
            });
            row.append(&close);
            let click = gtk::GestureClick::new();
            click.set_button(0);
            let win = w.clone();
            let close_w = close.clone();
            click.connect_released(move |g, _, x, y| {
                let Some(row) = g.widget() else { return };
                if let Some(hit) = row.pick(x, y, gtk::PickFlags::DEFAULT)
                    && (hit == close_w.clone().upcast::<gtk::Widget>() || hit.is_ancestor(&close_w))
                {
                    return;
                }
                match g.current_button() {
                    gdk::BUTTON_MIDDLE => {
                        with(|a| a.close_window(&win));
                    }
                    gdk::BUTTON_PRIMARY => {
                        with(|a| {
                            a.hide_preview();
                            a.focus(&win);
                        });
                    }
                    _ => {}
                }
            });
            row.add_controller(click);
            content.append(&row);
        }
        let motion = gtk::EventControllerMotion::new();
        motion.connect_enter(|_, _, _| {
            with(|a| cancel(&a.hide_timer));
        });
        motion.connect_leave(|_| {
            with(|a| a.schedule_preview_hide());
        });
        content.add_controller(motion);
        popover.set_child(Some(&content));
    }

    pub fn hide_preview(&self) {
        cancel(&self.show_timer);
        cancel(&self.hide_timer);
        let p = self.preview.borrow_mut().take();
        *self.preview_key.borrow_mut() = None;
        self.preview_sticky.set(false);
        if let Some(p) = p {
            p.popdown();
        }
    }
}
