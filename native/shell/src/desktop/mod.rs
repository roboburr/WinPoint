//! The desktop (D-G63 step 4), ported from vendor/ui/js/desktop/ (desktop.js, wallpaper.js, menus.js): the
//! wallpaper, the icons of ~/Bureaublad with This PC and the Recycle Bin, selection, rubber band, drag to move,
//! inline rename, the Win11 context menus, a toast with Undo and a small Properties dialog.
//!
//! The desktop is its own toplevel, a plain gtk::Window and not an ApplicationWindow: the GtkApplication's ID is
//! the panel's, and the extension tells our surfaces apart by application ID. It carries the Wayland app ID
//! io.github.win11shell.Desktop, so the extension (extension/desktopBridge.js) makes it the desktop (type
//! DESKTOP, below every window, on all workspaces) and hides Ubuntu's DING icons once we report `desktop.ready`
//! after the first frame (and every 2 s after, like host/src/gnome.rs DESKTOP_ADAPTER). The bus name
//! io.github.win11shell.Desktop carries the action group the extension activates on disable (`desktop-quit`).
mod actions;
mod dialogs;
mod grid;
mod menus;
mod wallpaper;

use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Object path of the action group the extension calls (`org.gtk.Actions.Activate("desktop-quit")`).
pub const BUS_PATH: &str = "/io/github/win11shell/Desktop";
/// App ID of our dialogs: neither the panel's nor the desktop's, so the extension leaves them ordinary windows.
pub const DIALOG_ID: &str = "io.github.win11shell.DesktopDialog";
const BRIDGE_NAME: &str = "io.github.win11shell.GnomeBridge";

thread_local! {
    static DESK: RefCell<Option<Rc<grid::Desk>>> = const { RefCell::new(None) };
    static QUIT: Cell<bool> = const { Cell::new(false) };
}

pub fn install(shell: &Rc<Shell>) {
    own_name();
    watch_bridge();
    shell.on("files", |event, data| with(|d| d.on_files(event, data)));
    shell.on("settings.changed", |_, data| with(|d| d.on_settings(data)));
    shell.on("desktop.geometry", |_, data| with(|d| d.on_geometry(data)));
    show(shell);
}

fn with(f: impl FnOnce(&Rc<grid::Desk>)) {
    if let Some(desk) = DESK.with(|d| d.borrow().clone()) {
        f(&desk);
    }
}

/// Create the desktop window (once; again after a desktop-quit when the extension comes back).
fn show(shell: &Rc<Shell>) {
    if DESK.with(|d| d.borrow().is_some()) {
        return;
    }
    QUIT.with(|q| q.set(false));
    let desk = grid::Desk::new(shell);
    DESK.with(|d| *d.borrow_mut() = Some(desk.clone()));
    report_ready(&desk);
    crate::theme::refresh(shell);
    desk.window.present();
    desk.start();
}

/// The extension's desktop-quit (its disable): the window goes, the bus name stays ours, so the extension's next
/// enable finds no stale desktop to wait for and adopts the window we make when its bridge is back.
fn quit() {
    QUIT.with(|q| q.set(true));
    if let Some(desk) = DESK.with(|d| d.borrow_mut().take()) {
        desk.destroy();
    }
}

fn own_name() {
    let owner = gio::bus_own_name(
        gio::BusType::Session,
        crate::DESKTOP_ID,
        gio::BusNameOwnerFlags::NONE,
        |connection, _| {
            let group = gio::SimpleActionGroup::new();
            let quit_action = gio::SimpleAction::new("desktop-quit", None);
            quit_action.connect_activate(|_, _| quit());
            group.add_action(&quit_action);
            let show_action = gio::SimpleAction::new("desktop-show", None);
            show_action.connect_activate(|_, _| show(&crate::core::shell()));
            group.add_action(&show_action);
            if let Err(error) = connection.export_action_group(BUS_PATH, &group) {
                eprintln!("win11-shell: desktop: exporting {BUS_PATH}: {error}");
            }
        },
        |_, _| {},
        |_, name| eprintln!("win11-shell: desktop: the bus name {name} belongs to another process"),
    );
    std::mem::forget(owner);
}

/// After a desktop-quit, the extension's bridge returning (its enable) brings the desktop back.
fn watch_bridge() {
    let watcher = gio::bus_watch_name(
        gio::BusType::Session,
        BRIDGE_NAME,
        gio::BusNameWatcherFlags::NONE,
        |_, _, _| {
            if QUIT.with(|q| q.get()) {
                glib::timeout_add_local_once(Duration::from_millis(300), || show(&crate::core::shell()));
            }
        },
        |_, _| {},
    );
    std::mem::forget(watcher);
}

/// desktop.ready {pid} once the first frame is drawn, then every 2 s while the window lives (the extension
/// re-registers after its bridge was away, as the WebKit desktop's adapter does).
fn report_ready(desk: &Rc<grid::Desk>) {
    let weak = Rc::downgrade(desk);
    let handler: Rc<RefCell<Option<(gdk::FrameClock, glib::SignalHandlerId)>>> = Rc::new(RefCell::new(None));
    let started = Rc::new(Cell::new(false));
    desk.window.connect_map(move |window| {
        if started.get() {
            return;
        }
        let Some(clock) = window.frame_clock() else { return };
        started.set(true);
        let weak = weak.clone();
        let slot = handler.clone();
        let id = clock.connect_after_paint(move |_| {
            let taken = slot.borrow_mut().take();
            if let Some((clock, id)) = taken {
                clock.disconnect(id);
            }
            let Some(desk) = weak.upgrade() else { return };
            let shell = crate::core::shell();
            let pid = std::process::id();
            if let Err(error) = shell.gnome.call("desktop.ready", &json!({"pid": pid})) {
                eprintln!("win11-shell: desktop.ready: {error}");
            }
            let weak = Rc::downgrade(&desk);
            glib::timeout_add_local(Duration::from_secs(2), move || match weak.upgrade() {
                Some(desk) if desk.alive.get() => {
                    crate::core::shell().gnome.send("desktop.ready", json!({"pid": pid}));
                    glib::ControlFlow::Continue
                }
                _ => glib::ControlFlow::Break,
            });
        });
        *handler.borrow_mut() = Some((clock, id));
    });
}

/// The extension's view of the monitors (desktop.geometry): logical size of the whole desktop.
pub fn geometry(shell: &Shell) -> (i32, i32) {
    let geo = shell.gnome.call("desktop.geometry", &json!({})).unwrap_or(Value::Null);
    size_of(&geo).unwrap_or_else(|| {
        if let Some(display) = gdk::Display::default()
            && let Some(monitor) = display.monitors().item(0).and_downcast::<gdk::Monitor>()
        {
            let g = monitor.geometry();
            return (g.width().max(320), g.height().max(240));
        }
        (1920, 1080)
    })
}

/// The logical size in a desktop.geometry answer or event.
pub fn size_of(geo: &Value) -> Option<(i32, i32)> {
    let w = geo["logical"]["w"].as_i64().filter(|w| *w > 0).or(geo["w"].as_i64()).unwrap_or(0) as i32;
    let h = geo["logical"]["h"].as_i64().filter(|h| *h > 0).or(geo["h"].as_i64()).unwrap_or(0) as i32;
    (w > 0 && h > 0).then_some((w.max(320), h.max(240)))
}

/// encodeURIComponent for query values.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode an image (PNG, JPEG, SVG, ... through gdk-pixbuf) off the main thread. `target`: the size it is shown at
/// and whether it covers (else fits) that box; vector images render at exactly that size, raster images only
/// shrink to it. None keeps the natural size.
pub fn decode(bytes: &[u8], target: Option<((i32, i32), bool)>) -> Option<gdk::Texture> {
    use gtk::gdk_pixbuf::PixbufLoader;
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(1024)]).to_ascii_lowercase();
    let vector = head.contains("<svg");
    let loader = PixbufLoader::new();
    if let Some(((tw, th), cover)) = target {
        loader.connect_size_prepared(move |loader, w, h| {
            if w <= 0 || h <= 0 || tw <= 0 || th <= 0 {
                return;
            }
            let (sx, sy) = (tw as f64 / w as f64, th as f64 / h as f64);
            let mut s = if cover { sx.max(sy) } else { sx.min(sy) };
            if !vector && s > 1.0 {
                s = 1.0;
            }
            if (s - 1.0).abs() > 0.001 {
                loader.set_size(((w as f64 * s).round() as i32).max(1), ((h as f64 * s).round() as i32).max(1));
            }
        });
    }
    let written = loader.write(bytes).is_ok();
    let closed = loader.close().is_ok();
    if !written || !closed {
        return None;
    }
    let pixbuf = loader.pixbuf()?;
    let format = if pixbuf.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    Some(
        gdk::MemoryTexture::new(
            pixbuf.width(),
            pixbuf.height(),
            format,
            &pixbuf.read_pixel_bytes(),
            pixbuf.rowstride() as usize,
        )
        .upcast(),
    )
}

/// A backend call as a future (the answer on the main thread).
pub async fn call(shell: &Shell, method: &str, args: Value) -> Result<Value, String> {
    let backend = shell.backend.clone();
    let method = method.to_owned();
    gio::spawn_blocking(move || backend.call(&method, &args))
        .await
        .unwrap_or_else(|_| Err("backend thread panicked".into()))
}
