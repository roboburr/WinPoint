//! The taskbars of the other monitors (Windows 11's "Mijn taakbalk weergeven op alle beeldschermen", Robin 16-11:
//! "hoofdscherm met hoofdtaakbalk, en tweede scherm met taakbalk met alleen de draaiende app op dat scherm"). The main
//! taskbar (panel/mod.rs) stays on the primary monitor with Start, search, Task View, the pins, the tray, the clock and
//! every flyout; each other monitor gets a 48 px window of its own with only app buttons (panel/taskbar.rs
//! `build_secondary`, panel/taskbar/apps.rs `Role::Secondary`).
//!
//! The extension (extension/panelBridge.js) publishes the monitors (`panel.monitors`: their index, logical size, the
//! primary, whether the panel is up); this module makes one window per other monitor once the panel is adopted, with
//! the Wayland app ID io.github.win11shell.SecondaryTaskbar.Monitor<N>, and drops it when the monitor goes, becomes the
//! primary or "taskbar.allDisplays" is turned off. The extension makes it a dock at the bottom of that monitor with its
//! own strut, hides it when the panel would be hidden (a focused fullscreen app on that monitor) and blurs it. Which
//! windows a taskbar shows is "taskbar.multiDisplayApps" (Windows' MMTaskbarMode, `Mode`).
//!
//! A taskbar window is a plain gtk::Window (not the GtkApplication's, whose ID is the panel's). The extension closes
//! it when it is disabled (the lock screen); it is then destroyed, and made anew when the panel is back.
use super::{BAR, taskbar};
use crate::core::Shell;
use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// The app ID of a taskbar window, followed by its monitor's index (extension/panelBridge.js STRIP_ID).
pub const APP_ID_PREFIX: &str = "io.github.win11shell.SecondaryTaskbar.Monitor";
const BRIDGE_NAME: &str = "io.github.win11shell.GnomeBridge";

/// "Bij gebruik van meerdere beeldschermen mijn taakbalk-apps weergeven op" (taskbar.multiDisplayApps): Windows'
/// MMTaskbarMode 0, 1 and 2. Anything else reads as the default, `Window`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// "Alle taakbalken": every taskbar shows the pins and every window.
    All,
    /// "Hoofdtaakbalk en taakbalk waar het venster is geopend": the main taskbar shows every window, the others
    /// their own monitor's.
    MainAndWindow,
    /// "Taakbalk waar het venster is geopend": each taskbar its own monitor's windows (the main taskbar with the pins).
    Window,
}

impl Mode {
    pub fn parse(value: &Value) -> Mode {
        match value.as_str() {
            Some("all") => Mode::All,
            Some("mainAndWindow") => Mode::MainAndWindow,
            _ => Mode::Window,
        }
    }
}

/// One monitor of a panel.monitors answer: its index, whether it is the primary, its logical size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub index: i64,
    pub primary: bool,
    pub w: i32,
    pub h: i32,
}

/// The extension's panel.monitors.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub primary: i64,
    pub monitors: Vec<Monitor>,
    /// The panel window is adopted (the taskbars are made only after it: a new window carries the panel's app ID
    /// for a moment).
    pub panel: bool,
}

impl Layout {
    pub fn parse(data: &Value) -> Option<Layout> {
        let list = data["monitors"].as_array()?;
        let monitors = list
            .iter()
            .filter_map(|m| {
                let index = m["index"].as_i64()?;
                let w = m["logical"]["w"].as_i64().filter(|w| *w > 0).or(m["w"].as_i64())?;
                let h = m["logical"]["h"].as_i64().filter(|h| *h > 0).or(m["h"].as_i64())?;
                Some(Monitor { index, primary: m["primary"] == true, w: w as i32, h: h as i32 })
            })
            .collect();
        Some(Layout { primary: data["primary"].as_i64().unwrap_or(-1), monitors, panel: data["panel"] == true })
    }
}

/// The monitors that get a taskbar of their own: every one but the primary, while the panel is up, there are two or
/// more and "taskbar.allDisplays" is on.
pub fn wanted(layout: &Layout, all_displays: bool) -> Vec<i64> {
    if !all_displays || !layout.panel || layout.monitors.len() < 2 {
        return Vec::new();
    }
    let mut out: Vec<i64> = layout.monitors.iter().map(|m| m.index).filter(|i| *i != layout.primary).collect();
    out.dedup();
    out
}

/// "taskbar.allDisplays": on unless it is exactly false.
pub fn all_displays(shell: &Shell) -> bool {
    shell.setting("taskbar.allDisplays") != Value::Bool(false)
}

struct Strip {
    monitor: i64,
    window: gtk::Window,
    size: Rc<Cell<(i32, i32)>>,
    root: gtk::Widget,
}

thread_local! {
    static LAYOUT: RefCell<Layout> = RefCell::new(Layout::default());
    static STRIPS: RefCell<Vec<Strip>> = const { RefCell::new(Vec::new()) };
    static QUEUED: Cell<bool> = const { Cell::new(false) };
}

/// What the app buttons of every taskbar filter by (panel/taskbar/apps.rs `shows`).
pub struct Ctx {
    pub primary: i64,
    pub mode: Mode,
    /// The monitors that have a taskbar window of their own now.
    pub strips: Vec<i64>,
}

pub fn context() -> Ctx {
    let shell = crate::core::shell();
    Ctx {
        primary: LAYOUT.with(|l| l.borrow().primary),
        mode: Mode::parse(&shell.setting("taskbar.multiDisplayApps")),
        strips: STRIPS.with(|s| s.borrow().iter().map(|s| s.monitor).collect()),
    }
}

pub fn install(shell: &Rc<Shell>) {
    shell.on("panel.monitors", |_, data| {
        if let Some(layout) = Layout::parse(data) {
            LAYOUT.with(|l| *l.borrow_mut() = layout);
            queue_reconcile();
        }
    });
    shell.on("panel.changed", |_, data| {
        let present = data["present"] == true;
        LAYOUT.with(|l| l.borrow_mut().panel = present);
        fetch();
        if present {
            // an extension that adopts the panel again (its enable) knows no taskbar's button rectangles
            super::taskbar::resend_icon_rects();
        }
    });
    shell.on("settings.changed", |_, data| {
        if crate::core::touches(data, &["taskbar.allDisplays"]) {
            queue_reconcile();
        }
        if crate::core::touches(data, &["taskbar.multiDisplayApps"]) {
            super::taskbar::render_all_apps();
        }
        if crate::core::touches(data, &["language"]) {
            retitle();
        }
    });
    shell.on("nativeBackdrop.changed", |_, data| apply_backdrop(data));
    // The extension's bridge (again): its view of the monitors; gone: no taskbars without it.
    let watcher = gtk::gio::bus_watch_name(
        gtk::gio::BusType::Session,
        BRIDGE_NAME,
        gtk::gio::BusNameWatcherFlags::NONE,
        |_, _, _| {
            glib::timeout_add_local_once(Duration::from_millis(200), fetch);
        },
        |_, _| {
            LAYOUT.with(|l| l.borrow_mut().panel = false);
            queue_reconcile();
        },
    );
    std::mem::forget(watcher);
    glib::idle_add_local_once(fetch);
}

/// The panel window was shown again (the extension's open_new_window): its monitors once more.
pub fn show_again() {
    glib::idle_add_local_once(fetch);
}

/// The extension's panel.monitors now.
fn fetch() {
    let shell = crate::core::shell();
    if let Ok(answer) = shell.gnome.call("panel.monitors", &json!({}))
        && let Some(layout) = Layout::parse(&answer)
    {
        LAYOUT.with(|l| *l.borrow_mut() = layout);
    }
    queue_reconcile();
}

fn queue_reconcile() {
    if QUEUED.with(|q| q.replace(true)) {
        return;
    }
    glib::idle_add_local_once(|| {
        QUEUED.with(|q| q.set(false));
        reconcile();
    });
}

/// Make the taskbars that are wanted and missing, size the others to their monitor and drop the rest.
fn reconcile() {
    let shell = crate::core::shell();
    let layout = LAYOUT.with(|l| l.borrow().clone());
    let want = wanted(&layout, all_displays(&shell));
    let gone: Vec<Strip> = STRIPS.with(|s| {
        let (keep, gone): (Vec<Strip>, Vec<Strip>) = s.borrow_mut().drain(..).partition(|st| want.contains(&st.monitor));
        *s.borrow_mut() = keep;
        gone
    });
    let changed = !gone.is_empty();
    for strip in gone {
        drop_strip(strip);
    }
    let mut created = false;
    for monitor in layout.monitors.iter().filter(|m| want.contains(&m.index)) {
        let size = (monitor.w.max(320), BAR);
        let existing = STRIPS.with(|s| s.borrow().iter().find(|st| st.monitor == monitor.index).map(|st| (st.size.clone(), st.window.clone())));
        match existing {
            Some((cell, window)) => {
                if cell.get() != size {
                    cell.set(size);
                    window.set_default_size(size.0, size.1);
                    window.queue_resize();
                }
            }
            None => {
                let strip = create(&shell, monitor.index, size);
                STRIPS.with(|s| s.borrow_mut().push(strip));
                created = true;
            }
        }
    }
    if changed || created {
        // which taskbars exist decides what the main one shows
        super::taskbar::render_all_apps();
    }
}

/// The title of a taskbar window (what Ctrl+Alt+Tab and a screen reader name it), in the current language.
fn title(shell: &Shell) -> String {
    shell.t("Taakbalk", "Taskbar")
}

/// The language changed: every taskbar window takes the title in the new one.
fn retitle() {
    let shell = crate::core::shell();
    let name = title(&shell);
    STRIPS.with(|s| {
        for strip in s.borrow().iter() {
            if strip.window.title().as_deref() != Some(name.as_str()) {
                strip.window.set_title(Some(&name));
            }
        }
    });
}

fn create(shell: &Rc<Shell>, monitor: i64, size: (i32, i32)) -> Strip {
    let window = gtk::Window::builder().decorated(false).resizable(true).title(title(shell)).build();
    window.add_css_class("w11-strip-window");
    window.set_widget_name(&format!("tb-strip-window-m{monitor}"));
    crate::theme::style_window(&window);
    crate::wayland::set_app_id(&window, &format!("{APP_ID_PREFIX}{monitor}"));
    window.set_default_size(size.0, size.1);
    let cell = Rc::new(Cell::new(size));
    let wanted = cell.clone();
    crate::wayland::keep_size(&window, move || wanted.get());
    let root = taskbar::build_secondary(shell, monitor);
    window.set_child(Some(&root));
    // The extension's disable closes it (the lock screen): it goes, and comes back with the panel.
    window.connect_close_request(move |_| {
        glib::idle_add_local_once(move || remove(monitor));
        glib::Propagation::Stop
    });
    window.present();
    Strip { monitor, window, size: cell, root }
}

fn remove(monitor: i64) {
    let gone: Vec<Strip> = STRIPS.with(|s| {
        let (gone, keep): (Vec<Strip>, Vec<Strip>) = s.borrow_mut().drain(..).partition(|st| st.monitor == monitor);
        *s.borrow_mut() = keep;
        gone
    });
    if gone.is_empty() {
        return;
    }
    for strip in gone {
        drop_strip(strip);
    }
    super::taskbar::render_all_apps();
}

fn drop_strip(strip: Strip) {
    super::taskbar::drop_secondary(strip.monitor);
    strip.window.destroy();
}

/// The extension's blur behind a taskbar (kind "taskbar-secondary" with its monitor): the 70 % tint then, else the
/// solid colour, as panel/mod.rs does for the main one.
fn apply_backdrop(state: &Value) {
    let pid = std::process::id() as u64;
    let blurred = |monitor: i64| {
        state["available"] == true
            && state["windows"].as_array().is_some_and(|ws| {
                ws.iter().any(|w| {
                    w["pid"].as_u64() == Some(pid)
                        && w["kind"] == "taskbar-secondary"
                        && w["monitor"].as_i64() == Some(monitor)
                        && w["blur"] == true
                })
            })
    };
    STRIPS.with(|s| {
        for strip in s.borrow().iter() {
            if blurred(strip.monitor) {
                strip.root.add_css_class("w11-glass");
            } else {
                strip.root.remove_css_class("w11-glass");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout(primary: i64, n: i64, panel: bool) -> Layout {
        Layout {
            primary,
            monitors: (0..n).map(|i| Monitor { index: i, primary: i == primary, w: 1280, h: 1024 }).collect(),
            panel,
        }
    }

    #[test]
    fn every_monitor_but_the_primary_gets_a_taskbar_while_the_panel_is_up() {
        assert_eq!(wanted(&layout(0, 2, true), true), vec![1]);
        assert_eq!(wanted(&layout(1, 3, true), true), vec![0, 2]);
        // one monitor, the panel not adopted yet, "Show my taskbar on all displays" off: none
        assert!(wanted(&layout(0, 1, true), true).is_empty());
        assert!(wanted(&layout(0, 2, false), true).is_empty());
        assert!(wanted(&layout(0, 2, true), false).is_empty());
        assert!(wanted(&Layout::default(), true).is_empty());
    }

    #[test]
    fn modes_follow_windows_and_anything_else_is_the_default() {
        assert_eq!(Mode::parse(&json!("all")), Mode::All);
        assert_eq!(Mode::parse(&json!("mainAndWindow")), Mode::MainAndWindow);
        assert_eq!(Mode::parse(&json!("window")), Mode::Window);
        for other in [json!("main"), json!(null), json!(2), json!("")] {
            assert_eq!(Mode::parse(&other), Mode::Window);
        }
    }

    #[test]
    fn panel_monitors_parse_with_their_logical_size() {
        let data = json!({"available": true, "primary": 0, "panel": true, "monitors": [
            {"index": 0, "primary": true, "x": 0, "y": 0, "w": 2880, "h": 1800, "scale": 2,
             "logical": {"x": 0, "y": 0, "w": 1440, "h": 900}},
            {"index": 1, "primary": false, "x": 2880, "y": 0, "w": 1280, "h": 1024, "scale": 1, "logical": {}},
        ]});
        let layout = Layout::parse(&data).unwrap();
        assert_eq!(layout.primary, 0);
        assert!(layout.panel);
        assert_eq!(layout.monitors[0], Monitor { index: 0, primary: true, w: 1440, h: 900 });
        assert_eq!(layout.monitors[1], Monitor { index: 1, primary: false, w: 1280, h: 1024 });
        assert_eq!(wanted(&layout, true), vec![1]);
        assert!(Layout::parse(&json!({"available": false})).is_none());
    }
}
