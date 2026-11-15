//! Flyouts of the taskbar: one at a time, inside the panel window's overlay, above the strip. While one is open
//! the panel takes the keyboard (panel.keyboard) and input only where the flyout is, plus the strip (panel/mod.rs
//! `Input`); a flyout that covers the monitor (Task View, the power screen) takes it everywhere. A click on another
//! window takes the focus from the panel and closes the flyout, and that click reaches its window (light dismiss,
//! as in Windows 11); so do Start opening, Alt+Tab and an app taking the focus.
pub mod calendar;
pub mod menus;
pub mod notifications;
pub mod power;
pub mod quick;

use crate::core::Shell;
use gtk::glib;
use gtk::prelude::*;
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// Where a flyout sits above the 48 px strip (12 px from the edges, as in Windows 11).
#[derive(Clone, Copy)]
pub enum Placement {
    BottomRight,
    BottomLeft,
    BottomCenter,
    /// Centred over this x (logical px from the monitor's left edge).
    BottomAt(i32),
}

pub trait Flyout {
    fn name(&self) -> &'static str;
    fn widget(&self) -> gtk::Widget;
    fn placement(&self) -> Placement {
        Placement::BottomRight
    }
    /// Covers the monitor (Task View, the power screen): the panel takes the input everywhere while it is open, and
    /// a click beside it lands on the dismiss layer. Any other flyout takes input only within its own bounds.
    fn covers_monitor(&self) -> bool {
        false
    }
    /// Stays open when the focus leaves the panel (the power screen while its action runs).
    fn holds_focus(&self) -> bool {
        false
    }
    fn opened(&self) {}
    fn closed(&self) {}
}

/// Where the panel window takes input beside the strip (panel/mod.rs turns it into the input region).
pub enum Input {
    /// The whole window: a flyout that covers the monitor is open.
    Monitor,
    /// These rectangles in window coordinates: the open flyout's frame, the toasts.
    Areas(Vec<gtk::cairo::RectangleInt>),
}

pub struct FlyoutHost {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    dismiss: gtk::Box,
    items: RefCell<Vec<(Rc<dyn Flyout>, gtk::Box)>>,
    open: RefCell<Option<&'static str>>,
    listeners: RefCell<Vec<Rc<dyn Fn(bool)>>>,
    /// The panel window had the focus while the open flyout was up (only then does losing it close the flyout).
    focused: Cell<bool>,
    /// Counts the opens, so a focus check that is still waiting belongs to one flyout.
    epoch: Cell<u64>,
    /// One more area that takes input whatever flyout is open: the toasts (notifications.rs).
    extra_input: Cell<Option<gtk::cairo::RectangleInt>>,
}

impl FlyoutHost {
    pub fn new(shell: &Rc<Shell>, window: &gtk::ApplicationWindow, overlay: &gtk::Overlay) -> Rc<FlyoutHost> {
        let dismiss = gtk::Box::new(gtk::Orientation::Vertical, 0);
        dismiss.set_hexpand(true);
        dismiss.set_vexpand(true);
        dismiss.set_visible(false);
        dismiss.add_css_class("w11-dismiss");
        overlay.add_overlay(&dismiss);
        let host = Rc::new(FlyoutHost {
            window: window.clone(),
            overlay: overlay.clone(),
            dismiss: dismiss.clone(),
            items: RefCell::new(Vec::new()),
            open: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            focused: Cell::new(false),
            epoch: Cell::new(0),
            extra_input: Cell::new(None),
        });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&host);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(h) = weak.upgrade() {
                h.close();
            }
        });
        dismiss.add_controller(click);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&host);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape
                && let Some(h) = weak.upgrade()
                && h.current().is_some()
            {
                h.close();
                return gtk::glib::Propagation::Stop;
            }
            gtk::glib::Propagation::Proceed
        });
        window.add_controller(keys);

        // Light dismiss: the focus leaves the panel (a click on another window, Start, Alt+Tab, an app taking it),
        // the flyout closes. GTK's view first; the extension's windows.focused as well, for a panel that never got
        // the focus. Popovers of a flyout (separate xdg_popups) keep the panel active.
        let weak = Rc::downgrade(&host);
        window.connect_is_active_notify(move |win| {
            let Some(h) = weak.upgrade() else { return };
            if h.current().is_none() {
                return;
            }
            if win.is_active() {
                h.focused.set(true);
            } else if h.focused.get() {
                h.close_after_focus_loss();
            }
        });
        let weak = Rc::downgrade(&host);
        shell.on("windows.focused", move |_, data| {
            if let Some(h) = weak.upgrade()
                && !data["id"].is_null()
                && h.current().is_some()
            {
                h.close_after_focus_loss();
            }
        });
        host
    }

    pub fn register(&self, flyout: Rc<dyn Flyout>) {
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.add_css_class("w11-flyout");
        frame.add_css_class(&format!("w11-flyout-{}", flyout.name()));
        frame.append(&flyout.widget());
        frame.set_visible(false);
        frame.set_valign(gtk::Align::End);
        frame.set_margin_bottom(crate::panel::BAR + 12);
        match flyout.placement() {
            Placement::BottomRight => {
                frame.set_halign(gtk::Align::End);
                frame.set_margin_end(12);
            }
            Placement::BottomLeft => {
                frame.set_halign(gtk::Align::Start);
                frame.set_margin_start(12);
            }
            Placement::BottomCenter => frame.set_halign(gtk::Align::Center),
            Placement::BottomAt(x) => {
                frame.set_halign(gtk::Align::Start);
                frame.set_margin_start(x.max(12));
            }
        }
        self.overlay.add_overlay(&frame);
        self.items.borrow_mut().push((flyout, frame));
    }

    pub fn on_change(&self, f: impl Fn(bool) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn current(&self) -> Option<&'static str> {
        *self.open.borrow()
    }

    pub fn is_open(&self, name: &str) -> bool {
        self.current() == Some(name)
    }

    pub fn toggle(&self, name: &'static str) {
        if self.is_open(name) { self.close() } else { self.open(name) }
    }

    pub fn open(&self, name: &'static str) {
        if self.is_open(name) {
            return;
        }
        // The name first: an unknown one changes nothing (it used to close the open flyout without telling anyone,
        // which left its input region and the panel's keyboard behind: an invisible wall over the monitor).
        if !self.items.borrow().iter().any(|(f, _)| f.name() == name) {
            eprintln!("win11-shell: no flyout {name}");
            return;
        }
        self.close_quietly();
        let items = self.items.borrow();
        let Some((flyout, frame)) = items.iter().find(|(f, _)| f.name() == name) else { return };
        self.dismiss.set_visible(true);
        frame.set_visible(true);
        *self.open.borrow_mut() = Some(name);
        self.epoch.set(self.epoch.get() + 1);
        self.focused.set(self.window.is_active());
        flyout.opened();
        drop(items);
        crate::core::shell().gnome.send("panel.keyboard", json!({"exclusive": true}));
        self.notify(true);
        self.window.present();
    }

    /// Re-anchor an open or opening flyout (e.g. a jump list over its button).
    pub fn set_placement_x(&self, name: &str, x: i32) {
        for (f, frame) in self.items.borrow().iter() {
            if f.name() == name {
                frame.set_halign(gtk::Align::Start);
                frame.set_margin_start(x.max(12));
            }
        }
    }

    pub fn close(&self) {
        if self.close_quietly() {
            crate::core::shell().gnome.send("panel.keyboard", json!({"exclusive": false}));
            self.notify(false);
        }
    }

    /// The focus left the panel: close the open flyout after a short check (as Start does), unless the panel has it
    /// back by then or the flyout holds on to it.
    fn close_after_focus_loss(self: &Rc<Self>) {
        let epoch = self.epoch.get();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            let Some(h) = weak.upgrade() else { return };
            let Some(name) = h.current() else { return };
            if h.epoch.get() != epoch || h.window.is_active() {
                return;
            }
            if h.items.borrow().iter().any(|(f, _)| f.name() == name && f.holds_focus()) {
                return;
            }
            // GTK's focus can lag the compositor during a quick remap: ask the extension first.
            let shell = crate::core::shell();
            let focused = shell.gnome.call("shell.info", &json!({})).ok().is_some_and(|i| i["panelFocused"] == true);
            if !focused {
                h.close();
            }
        });
    }

    /// What of the panel window takes input beside the strip: the open flyout (all of it for one over the
    /// monitor) and the toasts.
    pub fn input(&self) -> Input {
        let mut areas = Vec::new();
        if let Some(name) = self.current() {
            for (f, frame) in self.items.borrow().iter() {
                if f.name() != name {
                    continue;
                }
                if f.covers_monitor() {
                    return Input::Monitor;
                }
                if let Some(b) = frame.compute_bounds(&self.window) {
                    areas.push(gtk::cairo::RectangleInt::new(
                        b.x().floor() as i32,
                        b.y().floor() as i32,
                        (b.x() + b.width()).ceil() as i32 - b.x().floor() as i32,
                        (b.y() + b.height()).ceil() as i32 - b.y().floor() as i32,
                    ));
                }
            }
        }
        areas.extend(self.extra_input.get());
        Input::Areas(areas)
    }

    /// The toasts' area (None: no toast); the panel takes it into its input region after the next frame.
    pub fn set_extra_input(&self, area: Option<gtk::cairo::RectangleInt>) {
        if self.extra_input.replace(area) != area {
            self.window.queue_draw();
        }
    }

    fn close_quietly(&self) -> bool {
        let Some(name) = self.open.borrow_mut().take() else { return false };
        for (f, frame) in self.items.borrow().iter() {
            if f.name() == name {
                frame.set_visible(false);
                f.closed();
            }
        }
        self.dismiss.set_visible(false);
        true
    }

    fn notify(&self, open: bool) {
        let listeners: Vec<_> = self.listeners.borrow().clone();
        for f in listeners {
            f(open);
        }
    }
}

/// Every flyout module registers its flyouts here.
pub fn register_all(shell: &Rc<Shell>) {
    quick::register(shell);
    notifications::register(shell);
    calendar::register(shell);
    menus::register(shell);
    power::register(shell);
}
