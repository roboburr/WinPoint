//! Settings card, settings expander, generic expander and settings group (components.js SettingsCard,
//! SettingsExpander, Expander, SettingsGroup; the Windows 11 Instellingen rows).
use crate::icons;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The trailing mark of a clickable card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chevron {
    /// ">" (opens a page).
    Right,
    /// The "external" glyph (opens another app or a link).
    External,
    /// None.
    None,
}

struct CardInner {
    root: gtk::Widget,
    icon_slot: gtk::Box,
    title: gtk::Label,
    desc: gtk::Label,
    control: gtk::Box,
}

/// A settings card (`.fl-settings-card`): icon, title, description and a trailing control; a clickable card is
/// a button with a chevron. Classes: w11-settings-card, w11-sc-icon / -title / -desc / -control / -chevron.
#[derive(Clone)]
pub struct SettingsCard {
    inner: Rc<CardInner>,
}

impl SettingsCard {
    /// A card with an icon (glyph name, app icon name, or "" for none), title and description ("" for none).
    pub fn new(icon: &str, title: &str, description: &str) -> SettingsCard {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        SettingsCard::build(b.upcast(), icon, title, description, Chevron::None)
    }

    /// A clickable card (a gtk::Button: `connect_clicked`), with a chevron.
    pub fn clickable(icon: &str, title: &str, description: &str, chevron: Chevron) -> SettingsCard {
        let b = gtk::Button::new();
        SettingsCard::build(b.upcast(), icon, title, description, chevron)
    }

    fn build(root: gtk::Widget, icon: &str, title: &str, description: &str, chevron: Chevron) -> SettingsCard {
        root.add_css_class("w11-settings-card");
        let inner_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        inner_box.set_valign(gtk::Align::Center);
        let icon_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon_slot.add_css_class("w11-sc-icon");
        icon_slot.set_valign(gtk::Align::Center);
        icon_slot.set_halign(gtk::Align::Center);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        let title_l = super::label(title, "w11-sc-title", true);
        let desc_l = super::label(description, "w11-sc-desc", true);
        desc_l.set_visible(!description.is_empty());
        text.append(&title_l);
        text.append(&desc_l);
        let control = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        control.add_css_class("w11-sc-control");
        control.set_valign(gtk::Align::Center);
        control.set_halign(gtk::Align::End);
        control.set_hexpand(false);
        control.set_visible(false);
        inner_box.append(&icon_slot);
        inner_box.append(&text);
        inner_box.append(&control);
        match chevron {
            Chevron::None => {}
            Chevron::Right | Chevron::External => {
                let g = if chevron == Chevron::External { icons::glyph("external", 14) } else { icons::glyph("chevron-right", 12) };
                let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                slot.add_css_class("w11-sc-chevron");
                slot.set_valign(gtk::Align::Center);
                slot.set_hexpand(false);
                g.set_halign(gtk::Align::Center);
                slot.append(&g);
                inner_box.append(&slot);
            }
        }
        if let Some(b) = root.downcast_ref::<gtk::Box>() {
            b.append(&inner_box);
        } else if let Some(b) = root.downcast_ref::<gtk::Button>() {
            inner_box.set_hexpand(true);
            b.set_child(Some(&inner_box));
        }
        let me = SettingsCard { inner: Rc::new(CardInner { root, icon_slot, title: title_l, desc: desc_l, control }) };
        me.set_icon(icon);
        me
    }

    /// The card (a gtk::Box, or a gtk::Button when clickable).
    pub fn widget(&self) -> &gtk::Widget {
        &self.inner.root
    }

    /// The button of a clickable card.
    pub fn button(&self) -> Option<gtk::Button> {
        self.inner.root.downcast_ref::<gtk::Button>().cloned()
    }

    /// Run `f` when a clickable card is clicked (no effect on a plain card).
    pub fn connect_clicked(&self, f: impl Fn() + 'static) {
        if let Some(b) = self.button() {
            b.connect_clicked(move |_| f());
        }
    }

    pub fn set_title(&self, s: &str) {
        self.inner.title.set_label(s);
    }

    pub fn set_description(&self, s: &str) {
        self.inner.desc.set_label(s);
        self.inner.desc.set_visible(!s.is_empty());
    }

    /// A glyph name, an app icon name, or "" for none.
    pub fn set_icon(&self, icon: &str) {
        super::misc::clear(&self.inner.icon_slot);
        if icon.is_empty() {
            self.inner.icon_slot.set_visible(false);
            return;
        }
        self.inner.icon_slot.set_visible(true);
        if icons::has_glyph(icon) {
            self.inner.icon_slot.append(&icons::glyph(icon, 20));
        } else {
            self.inner.icon_slot.append(&icons::app_icon(icon, 20));
        }
    }

    /// Any widget as the icon (20 px).
    pub fn set_icon_widget(&self, w: &impl IsA<gtk::Widget>) {
        super::misc::clear(&self.inner.icon_slot);
        self.inner.icon_slot.append(w.as_ref());
        self.inner.icon_slot.set_visible(true);
    }

    /// Replace the trailing control.
    pub fn set_control(&self, w: &impl IsA<gtk::Widget>) {
        super::misc::clear(&self.inner.control);
        self.inner.control.append(w.as_ref());
        self.inner.control.set_visible(true);
    }

    /// Add one more trailing control (8 px apart).
    pub fn add_control(&self, w: &impl IsA<gtk::Widget>) {
        self.inner.control.append(w.as_ref());
        self.inner.control.set_visible(true);
    }

    /// Remove the trailing control.
    pub fn clear_control(&self) {
        super::misc::clear(&self.inner.control);
        self.inner.control.set_visible(false);
    }

    /// The trailing control box.
    pub fn control_box(&self) -> &gtk::Box {
        &self.inner.control
    }

    pub fn set_sensitive(&self, s: bool) {
        self.inner.root.set_sensitive(s);
    }
}

struct ExpanderInner {
    root: gtk::glib::WeakRef<gtk::Box>,
    header: gtk::Button,
    card: SettingsCard,
    chevron: gtk::Image,
    revealer: gtk::Revealer,
    content: gtk::Box,
    expanded: Cell<bool>,
    handlers: RefCell<Vec<Rc<dyn Fn(bool)>>>,
}

/// A settings expander (`.fl-settings-expander`): a card header with a chevron that opens nested rows below
/// (`add`: SettingsCard rows are indented to 58 px with dividers; other widgets get the same padding).
#[derive(Clone)]
pub struct SettingsExpander {
    root: gtk::Box,
    inner: Rc<ExpanderInner>,
}

impl SettingsExpander {
    pub fn new(icon: &str, title: &str, description: &str) -> SettingsExpander {
        let e = Expander::build(icon, title, description, true);
        SettingsExpander { root: e.root, inner: e.inner }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The header card (set_control for a control on the header, set_title, ...).
    pub fn header(&self) -> &SettingsCard {
        &self.inner.card
    }

    /// The header button (widget name for the dev channel).
    pub fn header_button(&self) -> &gtk::Button {
        &self.inner.header
    }

    /// Add a nested row.
    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        let w = w.as_ref();
        if !w.has_css_class("w11-settings-card") {
            w.add_css_class("w11-nested-other");
        }
        self.inner.content.append(w);
    }

    /// Remove all nested rows.
    pub fn clear(&self) {
        super::misc::clear(&self.inner.content);
    }

    pub fn set_expanded(&self, open: bool) {
        expander_set(&self.inner, open);
    }

    pub fn expanded(&self) -> bool {
        self.inner.expanded.get()
    }

    /// Run `f(expanded)` when the user opens or closes it.
    pub fn connect_toggled(&self, f: impl Fn(bool) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }
}

/// A generic expander (`.fl-expander`): a header card with title, description and a chevron, and content with
/// 16 px padding below it.
#[derive(Clone)]
pub struct Expander {
    root: gtk::Box,
    inner: Rc<ExpanderInner>,
}

impl Expander {
    pub fn new(icon: &str, title: &str, description: &str) -> Expander {
        Expander::build(icon, title, description, false)
    }

    fn build(icon: &str, title: &str, description: &str, settings: bool) -> Expander {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("w11-expander");
        if settings {
            root.add_css_class("w11-settings-expander");
        }
        let card = SettingsCard::clickable(icon, title, description, Chevron::None);
        let header = card.button().unwrap_or_default();
        header.add_css_class("w11-expander-header");
        let chevron = icons::glyph("chevron-down", 12);
        let chev_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        chev_box.add_css_class("w11-expander-chevron");
        chev_box.set_valign(gtk::Align::Center);
        chev_box.set_hexpand(false);
        chevron.set_halign(gtk::Align::Center);
        chevron.set_valign(gtk::Align::Center);
        chevron.set_hexpand(true);
        chev_box.append(&chevron);
        if let Some(inner_box) = header.child().and_downcast::<gtk::Box>() {
            inner_box.append(&chev_box);
        }
        let revealer = gtk::Revealer::new();
        revealer.set_transition_type(gtk::RevealerTransitionType::SlideDown);
        revealer.set_transition_duration(167);
        let content_frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_frame.add_css_class("w11-expander-content");
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("w11-expander-inner");
        content_frame.append(&content);
        revealer.set_child(Some(&content_frame));
        root.append(&header);
        root.append(&revealer);
        let inner = Rc::new(ExpanderInner {
            root: root.downgrade(),
            header: header.clone(),
            card,
            chevron,
            revealer,
            content,
            expanded: Cell::new(false),
            handlers: RefCell::new(Vec::new()),
        });
        let weak = Rc::downgrade(&inner);
        header.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                let open = !inner.expanded.get();
                expander_set(&inner, open);
                let hs: Vec<Rc<dyn Fn(bool)>> = inner.handlers.borrow().clone();
                for h in hs {
                    h(open);
                }
            }
        });
        super::own(&root, &inner);
        Expander { root, inner }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn header(&self) -> &SettingsCard {
        &self.inner.card
    }

    pub fn header_button(&self) -> &gtk::Button {
        &self.inner.header
    }

    /// Set the content.
    pub fn set_content(&self, w: &impl IsA<gtk::Widget>) {
        super::misc::clear(&self.inner.content);
        self.inner.content.append(w.as_ref());
    }

    /// Open or close (without running the handlers).
    pub fn set_expanded(&self, open: bool) {
        expander_set(&self.inner, open);
    }

    pub fn expanded(&self) -> bool {
        self.inner.expanded.get()
    }

    pub fn connect_toggled(&self, f: impl Fn(bool) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }
}

fn expander_set(inner: &ExpanderInner, open: bool) {
    inner.expanded.set(open);
    inner.revealer.set_reveal_child(open);
    icons::set_glyph(&inner.chevron, if open { "chevron-up" } else { "chevron-down" });
    if let Some(root) = inner.root.upgrade() {
        if open {
            root.add_css_class("w11-expanded");
        } else {
            root.remove_css_class("w11-expanded");
        }
    }
}

/// A settings group (`.fl-settings-group`): a semibold title, an optional description and cards 4 px apart;
/// groups after one another are 28 px apart.
#[derive(Clone)]
pub struct SettingsGroup {
    root: gtk::Box,
    title: gtk::Label,
    desc: gtk::Label,
    items: gtk::Box,
}

impl SettingsGroup {
    pub fn new(title: &str, description: &str) -> SettingsGroup {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("w11-settings-group");
        let title_l = super::label(title, "w11-settings-group-title", true);
        let desc_l = super::label(description, "w11-settings-group-desc", true);
        let items = gtk::Box::new(gtk::Orientation::Vertical, 4);
        items.add_css_class("w11-settings-group-items");
        root.append(&title_l);
        root.append(&desc_l);
        root.append(&items);
        let me = SettingsGroup { root, title: title_l, desc: desc_l, items };
        me.set_title(title);
        me.set_description(description);
        me
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// Add a card (or any widget).
    pub fn add(&self, w: &impl IsA<gtk::Widget>) {
        self.items.append(w.as_ref());
    }

    pub fn set_title(&self, s: &str) {
        self.title.set_label(s);
        self.title.set_visible(!s.is_empty());
    }

    pub fn set_description(&self, s: &str) {
        self.desc.set_label(s);
        self.desc.set_visible(!s.is_empty());
        if s.is_empty() {
            self.title.remove_css_class("w11-has-desc");
        } else {
            self.title.add_css_class("w11-has-desc");
        }
    }

    /// Remove the cards (title and description stay).
    pub fn clear(&self) {
        super::misc::clear(&self.items);
    }

    /// The box the cards are in.
    pub fn items(&self) -> &gtk::Box {
        &self.items
    }
}
