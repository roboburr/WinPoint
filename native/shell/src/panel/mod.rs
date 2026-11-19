//! The taskbar window: as the WebKit panel host did, it covers the primary monitor, is transparent, takes input
//! only where it has something (the 48 px strip, the open flyout and the toasts; the whole monitor for Task View
//! and the power screen, see `Input`). The extension recognises it by its application ID (PANEL_ID), positions
//! it, reserves the strut and blurs the strip (`nativeBackdrop.configure` kind "panel"). The other monitors get
//! taskbars of their own with only app buttons (strips.rs).
pub mod strips;
pub mod taskbar;

use crate::core::Shell;
use crate::flyouts::FlyoutHost;
use gtk::cairo::{RectangleInt, Region};
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const BAR: i32 = 48;

thread_local! {
    /// The taskbar window, for `show_again`.
    static WINDOW: RefCell<glib::WeakRef<gtk::ApplicationWindow>> = RefCell::new(glib::WeakRef::new());
}

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
    // The extension closes this window when it is disabled (the lock screen does that; a full-monitor transparent
    // client must not outlive it) and asks for the taskbar again when it comes back (open_new_window, an activate
    // of this GApplication). GTK would destroy the window and the taskbar stayed away after every unlock: hide it,
    // and show_again presents it.
    window.connect_close_request(|w| {
        let host = crate::core::shell().flyouts.borrow().clone();
        if let Some(host) = host {
            host.close();
        }
        w.set_visible(false);
        glib::Propagation::Stop
    });
    WINDOW.with(|cell| cell.borrow().set(Some(&window)));
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

    // Input: the strip, the open flyout and the toasts, taken again after every frame (so it follows a flyout
    // that opens, closes or changes size), and at once on the changes that need no layout.
    let input = Rc::new(Input {
        window: window.clone(),
        host: host.clone(),
        rects: RefCell::new(Vec::new()),
        announce: Cell::new(false),
    });
    let inp = input.clone();
    let hook = Rc::new(move |w: &gtk::ApplicationWindow| {
        if let Some(clock) = w.frame_clock() {
            let inp = inp.clone();
            clock.connect_after_paint(move |_| inp.after_paint());
        }
    });
    if window.is_realized() {
        hook(&window);
    }
    window.connect_realize(move |w| hook(w));
    let inp = input.clone();
    host.on_change(move |_| inp.sync());
    let inp = input.clone();
    window.connect_map(move |_| inp.sync());
    window.connect_default_height_notify(move |_| input.sync());

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

    // Frosted strip (the extension's blur, the page's 70 % tint in theme.rs) and the extension's own events. The
    // registration is a blocking call: again only when one of its three settings changed (settings.changed comes
    // for any key, a tray icon's first sighting too), or when the extension started anew (the backend's "reset",
    // e.g. after the lock screen) and no longer knows the strip.
    let options = Rc::new(Cell::new(backdrop_options(shell)));
    configure_backdrop(shell, &strip, options.get());
    let strip_weak = strip.downgrade();
    let opts = options.clone();
    shell.on("nativeBackdrop.changed", move |_, data| {
        if let Some(strip) = strip_weak.upgrade() {
            if data["reset"] == true && data["available"] == true {
                configure_backdrop(&crate::core::shell(), &strip, opts.get());
            } else {
                apply_backdrop(&strip, data);
            }
        }
    });
    let strip_weak = strip.downgrade();
    shell.on("settings.changed", move |_, _| {
        let shell = crate::core::shell();
        if let Some(strip) = strip_weak.upgrade() {
            let now = backdrop_options(&shell);
            if options.replace(now) != now {
                configure_backdrop(&shell, &strip, now);
            }
        }
        // a language switch: the window's title (Ctrl+Alt+Tab, a screen reader) in the new one, as strips::retitle
        // does for the taskbars of the other monitors
        let name = shell.t("Taakbalk", "Taskbar");
        if let Some(w) = WINDOW.with(|w| w.borrow().upgrade())
            && w.title().as_deref() != Some(name.as_str())
        {
            w.set_title(Some(&name));
        }
    });

    install_actions(shell);
    window.present();
    // the taskbars of the other monitors, once the extension has this one
    strips::install(shell);
}

/// The GApplication was activated (the extension's open_new_window when it comes back, or a second start of
/// win11-shell): the taskbar the extension closed is shown again; a shown one stays as it is.
pub fn show_again() {
    if let Some(window) = WINDOW.with(|cell| cell.borrow().upgrade())
        && !window.is_visible()
    {
        window.present();
    }
    strips::show_again();
}

fn monitor_size(shell: &Shell) -> (i32, i32) {
    let geo = shell.gnome.call("panel.geometry", &json!({})).unwrap_or(Value::Null);
    let w = geo["logical"]["w"].as_i64().or(geo["w"].as_i64()).unwrap_or(1920) as i32;
    let h = geo["logical"]["h"].as_i64().or(geo["h"].as_i64()).unwrap_or(1080) as i32;
    (w.max(320), h.max(240))
}

/// The panel window's input region. Only where the window shows something, so a click beside an open flyout
/// reaches the window under it (and closes the flyout through the focus, flyouts/mod.rs); the whole window while
/// Task View or the power screen covers the monitor.
///
/// mutter keeps a new input region without repicking the pointer: a pointer above an area the panel just gave
/// up stayed with the invisible panel surface, and clicks on the window under it did nothing until the pointer
/// crossed the strip (Robin, 14-11). So after the frame that committed a new region the extension is told
/// (panel.inputChanged), and it relayouts the panel's actor, which repicks.
struct Input {
    window: gtk::ApplicationWindow,
    host: Rc<FlyoutHost>,
    /// The rectangles of the region set last.
    rects: RefCell<Vec<RectangleInt>>,
    /// A region was set that the next frame commits: tell the extension after that frame.
    announce: Cell<bool>,
}

impl Input {
    fn sync(&self) {
        let Some(surface) = self.window.surface() else { return };
        let (w, h) = (self.window.width().max(1), self.window.height().max(1));
        let rects = match self.host.input() {
            crate::flyouts::Input::Monitor => vec![RectangleInt::new(0, 0, w, h)],
            crate::flyouts::Input::Areas(areas) => {
                let mut rects = vec![RectangleInt::new(0, h - BAR, w, BAR)];
                rects.extend(areas.into_iter().filter(|r| r.width() > 0 && r.height() > 0));
                rects
            }
        };
        if *self.rects.borrow() == rects {
            return;
        }
        let region = Region::create();
        for r in &rects {
            let _ = region.union_rectangle(r);
        }
        surface.set_input_region(Some(&region));
        *self.rects.borrow_mut() = rects;
        // GTK sends the region with the surface's next commit: draw one, and announce it after.
        self.announce.set(true);
        self.window.queue_draw();
    }

    /// After a frame: a region set before it went out with it, so tell the extension (from an idle: GDK flushes
    /// that commit to the compositor before the next dispatch); then take the region this frame's layout asks for.
    fn after_paint(&self) {
        if self.announce.replace(false) {
            glib::idle_add_local_once(|| crate::core::shell().gnome.send("panel.inputChanged", json!({})));
        }
        self.sync();
    }
}

/// What the strip's registration depends on: transparency, high contrast, auto-hide.
fn backdrop_options(shell: &Shell) -> (bool, bool, bool) {
    let settings = shell.settings();
    (
        settings["theme"]["transparency"] != false,
        settings["accessibility"]["highContrast"] == true,
        settings["taskbar"]["autoHide"] == true,
    )
}

fn configure_backdrop(shell: &Shell, strip: &gtk::Widget, options: (bool, bool, bool)) {
    let (transparency, high_contrast, auto_hide) = options;
    let args = json!({
        "pid": std::process::id(), "kind": "panel",
        "transparency": transparency, "highContrast": high_contrast, "autoHide": auto_hide,
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
