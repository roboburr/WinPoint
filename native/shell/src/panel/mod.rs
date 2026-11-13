//! The taskbar window: like the WebKit panel host it covers the primary monitor, is transparent, and takes input
//! only where it has something (the 48 px strip, or the whole monitor while a flyout is open, so a click outside
//! closes it). The extension recognises it by its application ID (PANEL_ID), positions it, reserves the strut
//! and blurs the strip (`nativeBackdrop.configure` kind "panel").
pub mod taskbar;

use crate::core::Shell;
use crate::flyouts::FlyoutHost;
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::rc::Rc;

pub const BAR: i32 = 48;

pub fn install(shell: &Rc<Shell>) {
    let window = gtk::ApplicationWindow::builder()
        .application(&shell.app)
        .decorated(false)
        // Resizable, like the WebKit panel host: a non-resizable toplevel pins its min/max size to the first
        // configure (the work area, 48 px short) and ignores the extension's full-monitor move_resize_frame.
        .resizable(true)
        .title(shell.t("Taakbalk", "Taskbar"))
        .build();
    window.add_css_class("w11-panel-window");
    crate::wayland::set_app_id(&window, crate::PANEL_ID);
    let (w, h) = monitor_size(shell);
    window.set_default_size(w, h);
    let size = Rc::new(std::cell::Cell::new((w, h)));
    let wanted = size.clone();
    crate::wayland::keep_size(&window, move || wanted.get());

    let base = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_vexpand(true);
    spacer.set_can_target(false);
    base.append(&spacer);
    let strip = taskbar::build(shell);
    strip.set_size_request(-1, BAR);
    base.append(&strip);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&base));
    window.set_child(Some(&overlay));

    let host = FlyoutHost::new(shell, &window, &overlay);
    *shell.flyouts.borrow_mut() = Some(host.clone());
    crate::flyouts::register_all(shell);

    // Input: the strip only, the monitor while a flyout is open (FlyoutHost calls back here).
    let win = window.clone();
    host.on_change(move |open| set_input(&win, open));
    let win = window.clone();
    window.connect_map(move |_| set_input(&win, false));
    let win = window.clone();
    window.connect_default_height_notify(move |_| set_input(&win, false));

    // Geometry follows the monitor.
    let weak = window.downgrade();
    shell.on("panel.geometry", move |_, _| {
        if let Some(w) = weak.upgrade() {
            let (width, height) = monitor_size(&crate::core::shell());
            if size.get() != (width, height) {
                size.set((width, height));
                w.set_default_size(width, height);
                w.queue_resize();
            }
        }
    });

    // Frosted strip (the extension's blur, the page's 70 % tint in theme.rs) and the extension's own events.
    configure_backdrop(shell, &strip);
    let strip_weak = strip.downgrade();
    shell.on("nativeBackdrop.changed", move |_, data| {
        if let Some(strip) = strip_weak.upgrade() {
            apply_backdrop(&strip, data);
        }
    });
    let strip_weak = strip.downgrade();
    shell.on("settings.changed", move |_, _| {
        if let Some(strip) = strip_weak.upgrade() {
            configure_backdrop(&crate::core::shell(), &strip);
        }
    });

    install_actions(shell);
    window.present();
}

fn monitor_size(shell: &Shell) -> (i32, i32) {
    let geo = shell.gnome.call("panel.geometry", &json!({})).unwrap_or(Value::Null);
    let w = geo["logical"]["w"].as_i64().or(geo["w"].as_i64()).unwrap_or(1920) as i32;
    let h = geo["logical"]["h"].as_i64().or(geo["h"].as_i64()).unwrap_or(1080) as i32;
    (w.max(320), h.max(240))
}

fn set_input(window: &gtk::ApplicationWindow, flyout_open: bool) {
    let Some(surface) = window.surface() else { return };
    let (w, h) = (window.width().max(1), window.height().max(1));
    let rect = if flyout_open {
        gtk::cairo::RectangleInt::new(0, 0, w, h)
    } else {
        gtk::cairo::RectangleInt::new(0, h - BAR, w, BAR)
    };
    surface.set_input_region(Some(&gtk::cairo::Region::create_rectangle(&rect)));
}

fn configure_backdrop(shell: &Shell, strip: &gtk::Widget) {
    let settings = shell.settings();
    let args = json!({
        "pid": std::process::id(), "kind": "panel",
        "transparency": settings["theme"]["transparency"] != false,
        "highContrast": settings["accessibility"]["highContrast"] == true,
        "autoHide": settings["taskbar"]["autoHide"] == true,
    });
    match shell.gnome.call("nativeBackdrop.configure", &args) {
        Ok(state) => apply_backdrop(strip, &state),
        Err(_) => strip.remove_css_class("w11-glass"),
    }
}

fn apply_backdrop(strip: &gtk::Widget, state: &Value) {
    let pid = std::process::id() as u64;
    let blur = state["available"] == true
        && state["windows"].as_array().is_some_and(|ws| {
            ws.iter().any(|w| w["pid"].as_u64() == Some(pid) && w["kind"] == "taskbar" && w["blur"] == true)
        });
    if blur { strip.add_css_class("w11-glass") } else { strip.remove_css_class("w11-glass") }
}

/// The extension's GActions on the panel (panelBridge.js): panel-taskview, panel-search, panel-power(action).
fn install_actions(shell: &Rc<Shell>) {
    let taskview = gio::SimpleAction::new("panel-taskview", None);
    taskview.connect_activate(|_, _| crate::core::shell().flyouts().open("taskview"));
    shell.app.add_action(&taskview);
    let search = gio::SimpleAction::new("panel-search", None);
    search.connect_activate(|_, _| crate::start::open_search(""));
    shell.app.add_action(&search);
    let power = gio::SimpleAction::new("panel-power", Some(glib::VariantTy::STRING));
    power.connect_activate(|_, parameter| {
        if let Some(action) = parameter.and_then(|p| p.str().map(str::to_owned)) {
            crate::flyouts::power::confirm(&crate::core::shell(), &action);
        }
    });
    shell.app.add_action(&power);
}
