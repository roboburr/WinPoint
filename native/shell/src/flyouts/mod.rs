//! Flyouts of the taskbar: one at a time, inside the panel window's overlay, above the strip. While one is open
//! the panel takes the whole monitor's input (a click outside closes it) and the keyboard (panel.keyboard).
pub mod calendar;
pub mod menus;
pub mod notifications;
pub mod power;
pub mod quick;

use crate::core::Shell;
use gtk::prelude::*;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

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
    fn opened(&self) {}
    fn closed(&self) {}
}

pub struct FlyoutHost {
    window: gtk::ApplicationWindow,
    overlay: gtk::Overlay,
    dismiss: gtk::Box,
    items: RefCell<Vec<(Rc<dyn Flyout>, gtk::Box)>>,
    open: RefCell<Option<&'static str>>,
    listeners: RefCell<Vec<Rc<dyn Fn(bool)>>>,
}

impl FlyoutHost {
    pub fn new(_shell: &Rc<Shell>, window: &gtk::ApplicationWindow, overlay: &gtk::Overlay) -> Rc<FlyoutHost> {
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
        self.close_quietly();
        let items = self.items.borrow();
        let Some((flyout, frame)) = items.iter().find(|(f, _)| f.name() == name) else {
            eprintln!("win11-shell: no flyout {name}");
            return;
        };
        self.dismiss.set_visible(true);
        frame.set_visible(true);
        *self.open.borrow_mut() = Some(name);
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
