//! Buttons (components.js Button, IconButton, HyperlinkButton; WinUI DropDownButton and SplitButton).
use super::menu::{self, MenuItem};
use crate::icons;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// The button styles of components.css: `.fl-btn` (Standard), `--accent`, `--subtle`, `--danger`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Standard,
    Accent,
    Subtle,
    Danger,
}

impl Variant {
    fn class(self) -> Option<&'static str> {
        match self {
            Variant::Standard => None,
            Variant::Accent => Some("w11-accent"),
            Variant::Subtle => Some("w11-subtle"),
            Variant::Danger => Some("w11-danger"),
        }
    }
}

/// A text button (32 px): `button("Opslaan", Variant::Accent)`. Classes: w11-button [w11-accent|w11-subtle|
/// w11-danger]; add w11-small / w11-large for the other sizes.
pub fn button(label: &str, variant: Variant) -> gtk::Button {
    let b = gtk::Button::with_label(label);
    b.add_css_class("w11-button");
    if let Some(c) = variant.class() {
        b.add_css_class(c);
    }
    b
}

/// A button with a glyph before its text (an empty label gives the icon-only `.fl-btn--icon-only`).
pub fn button_with_glyph(glyph: &str, label: &str, variant: Variant) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-button");
    if let Some(c) = variant.class() {
        b.add_css_class(c);
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_halign(gtk::Align::Center);
    row.append(&icons::glyph(glyph, 16));
    if label.is_empty() {
        b.add_css_class("w11-icon-only");
    } else {
        let l = gtk::Label::new(Some(label));
        l.set_ellipsize(gtk::pango::EllipsizeMode::End);
        row.append(&l);
    }
    b.set_child(Some(&row));
    b
}

/// A subtle square icon button (`.fl-icon-btn`), `size` px (32 by default in the HTML), glyph 16 px, with a
/// tooltip.
pub fn icon_button(glyph: &str, tooltip: &str, size: i32) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-icon-button");
    b.set_child(Some(&icons::glyph(glyph, if size <= 28 { 12 } else { 16 })));
    let s = size.max(16);
    b.set_size_request(s, s);
    b.set_valign(gtk::Align::Center);
    b.set_halign(gtk::Align::Center);
    if !tooltip.is_empty() {
        b.set_tooltip_text(Some(tooltip));
    }
    b
}

/// A hyperlink button (`.fl-link`), with an optional glyph after the text ("external" for links that leave).
pub fn hyperlink(label: &str, glyph: Option<&str>) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-link");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    row.append(&gtk::Label::new(Some(label)));
    if let Some(g) = glyph {
        row.append(&icons::glyph(g, 12));
    }
    b.set_child(Some(&row));
    b
}

/// A drop-down button (WinUI DropDownButton): text (and glyph) with a chevron; a click opens a menu flyout below.
#[derive(Clone)]
pub struct DropDownButton {
    button: gtk::Button,
    label: gtk::Label,
    items: Rc<RefCell<Rc<dyn Fn() -> Vec<MenuItem>>>>,
}

impl DropDownButton {
    /// `items` builds the menu each time it opens.
    pub fn new(glyph: Option<&str>, label: &str, variant: Variant, items: impl Fn() -> Vec<MenuItem> + 'static) -> DropDownButton {
        let button = gtk::Button::new();
        button.add_css_class("w11-button");
        button.add_css_class("w11-dropdown-button");
        if let Some(c) = variant.class() {
            button.add_css_class(c);
        }
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if let Some(g) = glyph {
            row.append(&icons::glyph(g, 16));
        }
        let l = gtk::Label::new(Some(label));
        l.set_visible(!label.is_empty());
        row.append(&l);
        let chev = icons::glyph("chevron-down", 12);
        chev.add_css_class("w11-dropdown-chevron");
        row.append(&chev);
        button.set_child(Some(&row));
        let items: Rc<RefCell<Rc<dyn Fn() -> Vec<MenuItem>>>> = Rc::new(RefCell::new(Rc::new(items)));
        let it = items.clone();
        button.connect_clicked(move |b| {
            let f = it.borrow().clone();
            menu::show_menu_for(b, f());
        });
        DropDownButton { button, label: l, items }
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    pub fn set_label(&self, label: &str) {
        self.label.set_label(label);
        self.label.set_visible(!label.is_empty());
    }

    /// Replace the menu builder.
    pub fn set_items(&self, items: impl Fn() -> Vec<MenuItem> + 'static) {
        *self.items.borrow_mut() = Rc::new(items);
    }
}

/// A split button (WinUI SplitButton): the main part runs the default action, the chevron part opens a menu.
#[derive(Clone)]
pub struct SplitButton {
    root: gtk::Box,
    main: gtk::Button,
    more: gtk::Button,
    items: Rc<RefCell<Rc<dyn Fn() -> Vec<MenuItem>>>>,
}

impl SplitButton {
    pub fn new(glyph: Option<&str>, label: &str, variant: Variant, items: impl Fn() -> Vec<MenuItem> + 'static) -> SplitButton {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("w11-split-button");
        let main = match glyph {
            Some(g) => button_with_glyph(g, label, variant),
            None => button(label, variant),
        };
        main.add_css_class("w11-split-main");
        let more = gtk::Button::new();
        more.add_css_class("w11-button");
        more.add_css_class("w11-split-more");
        if let Some(c) = variant.class() {
            more.add_css_class(c);
            root.add_css_class(c);
        }
        let chev = icons::glyph("chevron-down", 12);
        chev.add_css_class("w11-dropdown-chevron");
        more.set_child(Some(&chev));
        root.append(&main);
        let sep = gtk::Separator::new(gtk::Orientation::Vertical);
        root.append(&sep);
        root.append(&more);
        let items: Rc<RefCell<Rc<dyn Fn() -> Vec<MenuItem>>>> = Rc::new(RefCell::new(Rc::new(items)));
        let it = items.clone();
        let anchor = root.clone();
        more.connect_clicked(move |_| {
            let f = it.borrow().clone();
            menu::show_menu_for(&anchor, f());
        });
        SplitButton { root, main, more, items }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The main part (connect_clicked for the default action).
    pub fn main(&self) -> &gtk::Button {
        &self.main
    }

    /// The chevron part.
    pub fn more(&self) -> &gtk::Button {
        &self.more
    }

    pub fn set_items(&self, items: impl Fn() -> Vec<MenuItem> + 'static) {
        *self.items.borrow_mut() = Rc::new(items);
    }
}
