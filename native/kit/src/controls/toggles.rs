//! Toggle switch with its On/Off text, check box, radio group (components.js ToggleSwitch, Checkbox,
//! RadioGroup).
use crate::i18n::{self, t};
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

type BoolHandler = Rc<dyn Fn(bool)>;

struct SwitchInner {
    switch: gtk::Switch,
    state: gtk::Label,
    silent: Cell<bool>,
    handlers: RefCell<Vec<BoolHandler>>,
    texts: RefCell<Option<(String, String)>>,
}

/// A 40 x 20 toggle switch with "Aan" / "Uit" before it (WinUI ToggleSwitch; the state text follows the
/// language). Clicking the text toggles too.
#[derive(Clone)]
pub struct ToggleSwitch {
    root: gtk::Box,
    inner: Rc<SwitchInner>,
}

impl ToggleSwitch {
    pub fn new(active: bool) -> ToggleSwitch {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        root.add_css_class("w11-toggle");
        root.set_valign(gtk::Align::Center);
        let state = gtk::Label::new(None);
        state.add_css_class("w11-toggle-state");
        state.set_xalign(1.0);
        let switch = gtk::Switch::new();
        switch.add_css_class("w11-switch");
        switch.set_valign(gtk::Align::Center);
        switch.set_active(active);
        root.append(&state);
        root.append(&switch);
        let inner = Rc::new(SwitchInner {
            switch: switch.clone(),
            state,
            silent: Cell::new(false),
            handlers: RefCell::new(Vec::new()),
            texts: RefCell::new(None),
        });
        super::own(&root, &inner);
        let me = ToggleSwitch { root, inner };
        me.render();
        let weak = Rc::downgrade(&me.inner);
        switch.connect_active_notify(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            render_switch(&inner);
            if inner.silent.get() {
                return;
            }
            let v = inner.switch.is_active();
            let hs: Vec<BoolHandler> = inner.handlers.borrow().clone();
            for h in hs {
                h(v);
            }
        });
        let click = gtk::GestureClick::new();
        let sw = switch.downgrade();
        click.connect_released(move |g, _, _, _| {
            if let Some(sw) = sw.upgrade()
                && sw.is_sensitive()
            {
                sw.set_active(!sw.is_active());
                g.set_state(gtk::EventSequenceState::Claimed);
            }
        });
        me.inner.state.add_controller(click);
        let weak = Rc::downgrade(&me.inner);
        i18n::bind(&me.inner.state, move |_| {
            if let Some(inner) = weak.upgrade() {
                render_switch(&inner);
            }
        });
        me
    }

    fn render(&self) {
        render_switch(&self.inner);
    }

    /// The whole control.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The gtk::Switch itself (give it a widget name for the dev channel).
    pub fn switch(&self) -> &gtk::Switch {
        &self.inner.switch
    }

    pub fn active(&self) -> bool {
        self.inner.switch.is_active()
    }

    /// Set without running the handlers.
    pub fn set_active(&self, on: bool) {
        self.inner.silent.set(true);
        self.inner.switch.set_active(on);
        self.inner.silent.set(false);
        self.render();
    }

    /// Run `f(on)` when the user toggles it.
    pub fn connect_toggled(&self, f: impl Fn(bool) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }

    pub fn set_sensitive(&self, sensitive: bool) {
        self.inner.switch.set_sensitive(sensitive);
        self.inner.state.set_sensitive(sensitive);
        if sensitive {
            self.root.remove_css_class("w11-disabled");
        } else {
            self.root.add_css_class("w11-disabled");
        }
    }

    /// Own state texts instead of Aan/Uit (the app relabels them).
    pub fn set_state_texts(&self, on: &str, off: &str) {
        *self.inner.texts.borrow_mut() = Some((on.to_owned(), off.to_owned()));
        self.render();
    }

    /// Hide the state text (a switch with a label of its own).
    pub fn set_state_visible(&self, visible: bool) {
        self.inner.state.set_visible(visible);
    }
}

fn render_switch(inner: &SwitchInner) {
    let on = inner.switch.is_active();
    let texts = inner.texts.borrow().clone();
    let text = match texts {
        Some((on_text, off_text)) => if on { on_text } else { off_text },
        None => t(if on { "Aan" } else { "Uit" }, if on { "On" } else { "Off" }).to_owned(),
    };
    inner.state.set_label(&text);
}

/// A check box with a label (`.fl-check`); `checkbox_desc` adds a caption below the label.
pub fn checkbox(label: &str) -> gtk::CheckButton {
    let c = gtk::CheckButton::with_label(label);
    c.add_css_class("w11-check");
    c
}

/// A check box with a label and a description line under it.
pub fn checkbox_desc(label: &str, description: &str) -> gtk::CheckButton {
    let c = gtk::CheckButton::new();
    c.add_css_class("w11-check");
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.append(&super::label(label, "", true));
    b.append(&super::label(description, "w11-caption w11-text-2", true));
    c.set_child(Some(&b));
    c
}

struct RadioInner {
    buttons: RefCell<Vec<(String, gtk::CheckButton)>>,
    silent: Cell<bool>,
    handlers: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    horizontal: bool,
}

/// A radio group (`.fl-radio-group`) of (value, label) options, optionally with a header.
#[derive(Clone)]
pub struct RadioGroup {
    root: gtk::Box,
    inner: Rc<RadioInner>,
}

impl RadioGroup {
    pub fn new(options: &[(&str, &str)], selected: Option<&str>, header: Option<&str>, horizontal: bool) -> RadioGroup {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 4);
        root.add_css_class("w11-radio-group");
        if let Some(h) = header {
            let l = super::label(h, "w11-field-header", false);
            l.set_margin_bottom(4);
            root.append(&l);
        }
        let inner = Rc::new(RadioInner {
            buttons: RefCell::new(Vec::new()),
            silent: Cell::new(false),
            handlers: RefCell::new(Vec::new()),
            horizontal,
        });
        super::own(&root, &inner);
        let me = RadioGroup { root, inner };
        me.set_options(options, selected);
        let weak = Rc::downgrade(&me.inner);
        me.root.connect_notify_local(Some("name"), move |root, _| {
            let Some(inner) = weak.upgrade() else { return };
            let name = root.widget_name();
            if name.is_empty() || name == "GtkBox" { return; }
            for (value, button) in inner.buttons.borrow().iter() {
                button.set_widget_name(&format!("{name}-{value}"));
            }
        });
        me
    }

    /// Replace the options.
    pub fn set_options(&self, options: &[(&str, &str)], selected: Option<&str>) {
        let root = &self.root;
        for (_, b) in self.inner.buttons.borrow_mut().drain(..) {
            if let Some(p) = b.parent() {
                if p == *root.upcast_ref::<gtk::Widget>() {
                    root.remove(&b);
                } else if let Some(pb) = p.downcast_ref::<gtk::Box>() {
                    pb.remove(&b);
                    if pb.first_child().is_none() {
                        root.remove(pb);
                    }
                }
            }
        }
        let list = if self.inner.horizontal {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 24);
            row.add_css_class("w11-radio-group");
            row.add_css_class("w11-horizontal");
            root.append(&row);
            row
        } else {
            root.clone()
        };
        let mut first: Option<gtk::CheckButton> = None;
        let mut buttons = Vec::new();
        for (value, label) in options {
            let b = gtk::CheckButton::with_label(label);
            b.add_css_class("w11-radio");
            if let Some(f) = &first {
                b.set_group(Some(f));
            } else {
                first = Some(b.clone());
            }
            let name = root.widget_name();
            if !name.is_empty() && name != "GtkBox" {
                b.set_widget_name(&format!("{name}-{value}"));
            }
            b.set_active(selected == Some(*value));
            let v = value.to_string();
            let weak = Rc::downgrade(&self.inner);
            b.connect_toggled(move |b| {
                let Some(inner) = weak.upgrade() else { return };
                if !b.is_active() || inner.silent.get() {
                    return;
                }
                let hs: Vec<Rc<dyn Fn(&str)>> = inner.handlers.borrow().clone();
                for h in hs {
                    h(&v);
                }
            });
            list.append(&b);
            buttons.push((value.to_string(), b));
        }
        *self.inner.buttons.borrow_mut() = buttons;
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The selected value.
    pub fn value(&self) -> Option<String> {
        self.inner.buttons.borrow().iter().find(|(_, b)| b.is_active()).map(|(v, _)| v.clone())
    }

    /// Select without running the handlers.
    pub fn set_value(&self, value: &str) {
        self.inner.silent.set(true);
        for (v, b) in self.inner.buttons.borrow().iter() {
            if v == value {
                b.set_active(true);
            }
        }
        self.inner.silent.set(false);
    }

    /// Run `f(value)` when the user picks an option.
    pub fn connect_changed(&self, f: impl Fn(&str) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }

    /// The radio button of a value (for widget names, sensitivity).
    pub fn button(&self, value: &str) -> Option<gtk::CheckButton> {
        self.inner.buttons.borrow().iter().find(|(v, _)| v == value).map(|(_, b)| b.clone())
    }
}
