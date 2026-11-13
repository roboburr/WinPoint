//! Shared helpers of the menus module: the app catalogue cache (apps.list, apps.defaults), icons that fall back to
//! the backend's /icon resolver, launching apps and own apps (shell.openApp), and the taskbar pins
//! (taskbar.pinned, with the shipped browser pin shown as the user's default browser, as shell.js createPins).
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::cell::RefCell;

thread_local! {
    static APPS: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    static DEFAULTS: RefCell<Value> = const { RefCell::new(Value::Null) };
}

/// Fetch the app list and the default apps again (at start and on apps.changed).
pub fn refresh_apps(shell: &Shell) {
    shell.call("apps.list", json!({}), |r| {
        if let Ok(v) = r {
            let list = v
                .as_array()
                .cloned()
                .or_else(|| v.get("apps").and_then(|a| a.as_array().cloned()))
                .or_else(|| v.get("items").and_then(|a| a.as_array().cloned()))
                .unwrap_or_default();
            APPS.with(|a| *a.borrow_mut() = list);
        }
    });
    shell.call("apps.defaults", json!({}), |r| {
        if let Ok(v) = r {
            DEFAULTS.with(|d| *d.borrow_mut() = v);
        }
    });
}

fn bare(id: &str) -> &str {
    id.trim_end_matches(".desktop")
}

/// An app of the catalogue by desktop id (with or without ".desktop").
pub fn app(id: &str) -> Option<Value> {
    let want = bare(id);
    APPS.with(|a| a.borrow().iter().find(|x| x["id"].as_str().is_some_and(|i| bare(i) == want)).cloned())
}

pub fn apps() -> Vec<Value> {
    APPS.with(|a| a.borrow().clone())
}

/// The default app of a kind ("browser", "terminal", ...) from apps.defaults.
pub fn default_app(kind: &str) -> Option<String> {
    DEFAULTS.with(|d| d.borrow()[kind]["id"].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
}

pub fn str_of(v: &Value, key: &str) -> String {
    v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_owned()
}

/// Percent-encode a query value.
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// An icon by theme name or absolute path; a name the GTK icon theme does not know is asked from the backend's
/// icon resolver (it knows the app's own icon paths, snaps, flatpaks), which replaces the placeholder.
pub fn icon(shell: &Shell, src: &str, px: i32) -> gtk::Image {
    let image = crate::icons::app_icon(src, px);
    image.add_css_class("w11-mn-appicon");
    if src.is_empty() || src.starts_with('/') {
        return image;
    }
    let name = bare(src);
    let known = gtk::gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon(name));
    if !known {
        let path = format!("/icon?name={}&size={}", enc(name), (px * 2).max(32));
        let backend = shell.backend.clone();
        let weak = image.downgrade();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || backend.get(&path)).await;
            if let (Ok(Ok(bytes)), Some(img)) = (result, weak.upgrade())
                && let Some(texture) = crate::icons::texture(&bytes)
            {
                img.set_paintable(Some(&texture));
            }
        });
    }
    image
}

/// shell.openApp {app, params}: our own HTML apps (settings, taskmgr, explorer, alles, disks).
pub fn open_app(shell: &Shell, app: &str, params: Value) {
    shell.fire("shell.openApp", json!({"app": app, "params": params}));
}

/// Settings at a page.
pub fn open_settings(shell: &Shell, page: &str) {
    open_app(shell, "settings", json!({"page": page}));
}

/// Start an app by desktop id: our own apps through shell.openApp, others through apps.launch (shell.js launch).
pub fn launch(shell: &Shell, id: &str) {
    let b = bare(id);
    if let Some(own) = b.strip_prefix("win11shell-")
        && ["settings", "taskmgr", "explorer", "alles", "disks"].contains(&own)
    {
        open_app(shell, own, json!({}));
        return;
    }
    shell.fire("apps.launch", json!({"id": id}));
}

/// The taskbar's own app name for an id: the catalogue's name, else the id without ".desktop".
pub fn app_name(id: &str) -> String {
    app(id)
        .map(|a| str_of(&a, "name"))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| bare(id).rsplit('.').next().unwrap_or(id).to_owned())
}

/* ---- taskbar pins (shell.js createPins) ---- */

const SHIPPED_TASKBAR: &[&[&str]] = &[
    &[
        "win11shell-explorer.desktop",
        "firefox_firefox.desktop",
        "snap-store_snap-store.desktop",
        "win11shell-settings.desktop",
    ],
    &["org.gnome.Nautilus.desktop", "firefox_firefox.desktop", "win11shell-settings.desktop"],
];
const SHIPPED_BROWSER: &str = "firefox_firefox.desktop";

fn with_default_browser(ids: Vec<String>) -> Vec<String> {
    let shipped = SHIPPED_TASKBAR
        .iter()
        .any(|d| d.len() == ids.len() && d.iter().zip(&ids).all(|(a, b)| *a == b.as_str()));
    if !shipped {
        return ids;
    }
    let Some(browser) = default_app("browser").filter(|b| app(b).is_some()) else { return ids };
    if browser == SHIPPED_BROWSER {
        return ids;
    }
    if ids.contains(&browser) {
        ids.into_iter().filter(|x| x != SHIPPED_BROWSER).collect()
    } else {
        ids.into_iter().map(|x| if x == SHIPPED_BROWSER { browser.clone() } else { x }).collect()
    }
}

/// The taskbar pins as the taskbar shows them.
pub fn taskbar_pins(shell: &Shell) -> Vec<String> {
    let ids = shell
        .setting("taskbar.pinned")
        .as_array()
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    with_default_browser(ids)
}

pub fn is_pinned(shell: &Shell, id: &str) -> bool {
    taskbar_pins(shell).iter().any(|p| bare(p) == bare(id))
}

fn save_pins(shell: &Shell, ids: Vec<String>) {
    let mut seen = Vec::<String>::new();
    for id in ids {
        if !seen.contains(&id) {
            seen.push(id);
        }
    }
    shell.set_setting("taskbar.pinned", json!(seen));
}

pub fn pin(shell: &Shell, id: &str) {
    let mut list: Vec<String> = taskbar_pins(shell).into_iter().filter(|x| bare(x) != bare(id)).collect();
    list.push(id.to_owned());
    save_pins(shell, list);
}

pub fn unpin(shell: &Shell, id: &str) {
    let list = taskbar_pins(shell).into_iter().filter(|x| bare(x) != bare(id)).collect();
    save_pins(shell, list);
}

/// Remove every child of a box.
pub fn clear(container: &impl IsA<gtk::Widget>) {
    let w = container.as_ref();
    while let Some(child) = w.first_child() {
        child.unparent();
    }
}

/// The panel window's width (the monitor), for keeping flyouts on screen.
pub fn monitor_width(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget
        .as_ref()
        .root()
        .and_then(|r| r.downcast::<gtk::Window>().ok())
        .map(|w| w.width())
        .filter(|w| *w > 0)
        .unwrap_or(1920)
}

pub fn monitor_height(widget: &impl IsA<gtk::Widget>) -> i32 {
    widget
        .as_ref()
        .root()
        .and_then(|r| r.downcast::<gtk::Window>().ok())
        .map(|w| w.height())
        .filter(|h| *h > 0)
        .unwrap_or(1080)
}
