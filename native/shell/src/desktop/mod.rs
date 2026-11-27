//! The desktop (D-G63 step 4), ported from `git show 418ea594:vendor/ui/js/desktop/` (desktop.js, wallpaper.js,
//! menus.js): the wallpaper, the icons of ~/Bureaublad with This PC and the Recycle Bin, selection, rubber band, drag
//! to move, inline rename, the Win11 context menus, a toast with Undo and a small Properties dialog.
//!
//! The desktop is its own toplevel, a plain gtk::Window and not an ApplicationWindow: the GtkApplication's ID is
//! the panel's, and the extension tells our surfaces apart by application ID. It carries the Wayland app ID
//! io.github.win11shell.Desktop, so the extension (extension/desktopBridge.js) makes it the desktop (type
//! DESKTOP, below every window, on all workspaces) and hides Ubuntu's DING icons once we report `desktop.ready`
//! after the first frame (again only while it has not taken the window: report_ready). The bus name
//! io.github.win11shell.Desktop carries the action group the extension activates on disable (`desktop-quit`).
//! The desktop covers the primary monitor; each other monitor has a wallpaper window of its own (screens.rs).
mod actions;
mod dialogs;
mod grid;
mod layout;
mod menus;
mod screens;
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
    static READY: Cell<Ready> = const { Cell::new(Ready::Unpainted) };
    /// The one pending desktop.ready (a retry, or one after the bridge came back).
    static RETRY: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

/// Where desktop.ready stands for the current desktop window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ready {
    /// No frame drawn yet: nothing to report.
    Unpainted,
    /// Drawn; the next desktop.ready is due (a failure is logged).
    Pending,
    /// Drawn; desktop.ready failed before (the bridge is not there yet), or the extension had the pid but did not
    /// show the window: retried every 2 s, quietly.
    Retrying,
    /// The extension answered ready: it has our pid, nothing repeats.
    Known,
}

pub fn install(shell: &Rc<Shell>) {
    own_name();
    watch_bridge();
    shell.on("files", |event, data| with(|d| d.on_files(event, data)));
    shell.on("settings.changed", |_, data| with(|d| d.on_settings(data)));
    shell.on("desktop.geometry", |_, data| with(|d| d.on_geometry(data)));
    shell.on("desktop.changed", |_, data| {
        // present false while we are known: the extension lost our pid (an older one dropped it whenever any window
        // of this process closed) or lost the window; one desktop.ready tells it again. Present true ends the quiet
        // retries of an answer that had the pid but not the window (announce).
        let state = READY.with(Cell::get);
        if data["present"] == false && state == Ready::Known {
            READY.with(|r| r.set(Ready::Pending));
            schedule(Duration::ZERO);
        } else if data["present"] == true && state == Ready::Retrying {
            READY.with(|r| r.set(Ready::Known));
            cancel_retry();
        }
    });
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
    READY.with(|r| r.set(Ready::Unpainted));
    let desk = grid::Desk::new(shell);
    DESK.with(|d| *d.borrow_mut() = Some(desk.clone()));
    report_ready(&desk);
    crate::theme::refresh(shell);
    desk.window.present();
    desk.start();
    // the other monitors' desktop windows (screens.rs)
    if let Ok(geo) = shell.gnome.call("desktop.geometry", &json!({})) {
        screens::sync(&desk, &geo);
    }
}

/// The extension's desktop-quit (its disable): the window goes, the bus name stays ours, so the extension's next
/// enable finds no stale desktop to wait for and adopts the window we make when its bridge is back.
fn quit() {
    QUIT.with(|q| q.set(true));
    READY.with(|r| r.set(Ready::Unpainted));
    cancel_retry();
    screens::clear();
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

/// After a desktop-quit, the extension's bridge returning (its enable) brings the desktop back. A bridge that went
/// away without a desktop-quit (GNOME Shell restarted) knows nothing of our window when it returns: desktop.ready
/// again.
fn watch_bridge() {
    let watcher = gio::bus_watch_name(
        gio::BusType::Session,
        BRIDGE_NAME,
        gio::BusNameWatcherFlags::NONE,
        |_, _, _| {
            if QUIT.with(|q| q.get()) {
                glib::timeout_add_local_once(Duration::from_millis(300), || show(&crate::core::shell()));
            } else if READY.with(Cell::get) != Ready::Unpainted {
                READY.with(|r| r.set(Ready::Pending));
                schedule(Duration::from_millis(300));
            }
        },
        |_, _| {
            if READY.with(Cell::get) != Ready::Unpainted {
                READY.with(|r| r.set(Ready::Pending));
            }
            cancel_retry();
        },
    );
    std::mem::forget(watcher);
}

/// desktop.ready {pid} once the first frame is drawn. The extension answers ready when it has our pid; from then on
/// nothing repeats (until 14-11 it went out every 2 s for good, and each one lowered the window, sent
/// desktop.geometry and drew a desktop frame). Only while it has not answered: a retry every 2 s.
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
            if weak.upgrade().is_none_or(|desk| !desk.alive.get()) {
                return;
            }
            READY.with(|r| r.set(Ready::Pending));
            announce();
        });
        *handler.borrow_mut() = Some((clock, id));
    });
}

/// One desktop.ready for the drawn window, while the extension does not have it yet.
fn announce() {
    let alive = DESK.with(|d| d.borrow().as_ref().is_some_and(|d| d.alive.get()));
    let state = READY.with(Cell::get);
    if !alive || !matches!(state, Ready::Pending | Ready::Retrying) {
        cancel_retry();
        return;
    }
    match crate::core::shell().gnome.call("desktop.ready", &json!({"pid": std::process::id()})) {
        Ok(answer) if answer["ready"] == true && answer["present"] != false => {
            READY.with(|r| r.set(Ready::Known));
            cancel_retry();
        }
        Ok(answer) if answer["ready"] == true => {
            // the pid is known but the window is not shown (the overview hides it; an older extension, still loaded
            // until the next login, may also drop the pid meanwhile and then sends no new desktop.changed): ask again
            // quietly every 2 s until desktop.changed reports the window present. The current extension answers
            // these from its pid list without touching the window.
            READY.with(|r| r.set(Ready::Retrying));
            schedule(Duration::from_secs(2));
        }
        answer => {
            if state == Ready::Pending {
                eprintln!("win11-shell: desktop.ready: {}", answer.err().unwrap_or_else(|| "not ready".into()));
            }
            READY.with(|r| r.set(Ready::Retrying));
            schedule(Duration::from_secs(2));
        }
    }
}

/// announce() after `delay`, in place of any pending one.
fn schedule(delay: Duration) {
    cancel_retry();
    let id = glib::timeout_add_local_once(delay, || {
        // fired: the source is gone, so cancel_retry must not remove it again
        RETRY.with(|r| r.borrow_mut().take());
        announce();
    });
    RETRY.with(|r| *r.borrow_mut() = Some(id));
}

fn cancel_retry() {
    if let Some(id) = RETRY.with(|r| r.borrow_mut().take()) {
        id.remove();
    }
}

/// One monitor of desktop.geometry's `areas`: its index, whether it is the primary, its logical size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Area {
    pub index: i64,
    pub primary: bool,
    pub w: i32,
    pub h: i32,
}

/// The monitors that get a desktop window of their own besides the desktop (screens.rs): every one but the primary,
/// when there are two or more. A desktop window stays on one monitor (Mutter keeps it in that monitor's work area),
/// so the desktop itself covers the primary monitor and each other monitor has its own wallpaper window.
pub fn others(geo: &Value) -> Vec<Area> {
    let areas: Vec<Area> = geo["areas"]
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let (w, h) = (a["w"].as_i64()? as i32, a["h"].as_i64()? as i32);
                    let index = a["index"].as_i64()?;
                    (w > 0 && h > 0).then_some(Area { index, primary: a["primary"] == true, w, h })
                })
                .collect()
        })
        .unwrap_or_default();
    if areas.len() < 2 || !areas.iter().any(|a| a.primary) {
        return Vec::new();
    }
    areas.into_iter().filter(|a| !a.primary).collect()
}

/// The extension's view of the monitors (desktop.geometry): logical size of the desktop (the primary monitor).
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
    decode_pixbuf(bytes, target).map(|pixbuf| texture_of(&pixbuf))
}

/// `decode`, as the pixbuf (the wallpaper's "span" takes a part of it).
pub fn decode_pixbuf(bytes: &[u8], target: Option<((i32, i32), bool)>) -> Option<gtk::gdk_pixbuf::Pixbuf> {
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
    loader.pixbuf()
}

/// A decoded picture as a texture.
pub fn texture_of(pixbuf: &gtk::gdk_pixbuf::Pixbuf) -> gdk::Texture {
    let format = if pixbuf.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    gdk::MemoryTexture::new(pixbuf.width(), pixbuf.height(), format, &pixbuf.read_pixel_bytes(), pixbuf.rowstride() as usize)
        .upcast()
}

/// A backend call as a future (the answer on the main thread).
pub async fn call(shell: &Shell, method: &str, args: Value) -> Result<Value, String> {
    let backend = shell.backend.clone();
    let method = method.to_owned();
    gio::spawn_blocking(move || backend.call(&method, &args))
        .await
        .unwrap_or_else(|_| Err("backend thread panicked".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_monitor_has_no_other_desktop_window() {
        let one = json!({"available": true, "w": 1440, "h": 900, "logical": {"w": 1440, "h": 900},
            "areas": [{"index": 0, "primary": true, "x": 0, "y": 0, "w": 1440, "h": 900}]});
        assert!(others(&one).is_empty());
        // an older extension without areas
        assert!(others(&json!({"w": 1440, "h": 900})).is_empty());
    }

    #[test]
    fn every_monitor_but_the_primary_gets_one() {
        let two = json!({"available": true, "x": 0, "y": 0, "w": 1440, "h": 900, "monitor": 0, "areas": [
            {"index": 0, "primary": true, "x": 0, "y": 0, "w": 1440, "h": 900},
            {"index": 1, "primary": false, "x": 1440, "y": 0, "w": 1280, "h": 960}]});
        assert_eq!(others(&two), vec![Area { index: 1, primary: false, w: 1280, h: 960 }]);
        // the primary on the right, a third monitor; a monitor without a size is left out
        let three = json!({"areas": [
            {"index": 0, "primary": false, "x": 0, "y": 0, "w": 1280, "h": 1024},
            {"index": 1, "primary": true, "x": 1280, "y": 0, "w": 1440, "h": 900},
            {"index": 2, "primary": false, "x": 2720, "y": 0, "w": 0, "h": 900}]});
        assert_eq!(others(&three), vec![Area { index: 0, primary: false, w: 1280, h: 1024 }]);
        // no primary known yet: none
        assert!(others(&json!({"areas": [{"index": 0, "w": 10, "h": 10}, {"index": 1, "w": 10, "h": 10}]})).is_empty());
    }
}
