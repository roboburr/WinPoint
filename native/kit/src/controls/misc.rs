//! Breadcrumb, badge, avatar, empty state, page header, list helpers and the accent swatch grid
//! (components.js Breadcrumb, Badge, Avatar, EmptyState, PageHeader, ListItem, ColorSwatchGrid).
use crate::i18n::t;
use crate::icons;
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

/// Remove every child of a box.
pub fn clear(b: &impl IsA<gtk::Box>) {
    let b = b.as_ref();
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A vertical scrolled window around `child` (no horizontal scrolling).
pub fn scrolled(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    sw.set_vexpand(true);
    sw.set_hexpand(true);
    sw.set_child(Some(child.as_ref()));
    sw
}

/// An InfoBadge (`.fl-badge`): a number or text; "" is the 6 px dot. Severity: "attention" (accent),
/// "informational", "success", "caution", "critical".
pub fn badge(value: &str, severity: &str) -> gtk::Label {
    let l = gtk::Label::new(if value.is_empty() { None } else { Some(value) });
    l.add_css_class("w11-badge");
    match severity {
        "informational" | "success" | "caution" | "critical" => l.add_css_class(&format!("w11-{severity}")),
        _ => {}
    }
    if value.is_empty() {
        l.add_css_class("w11-dot");
        l.set_size_request(6, 6);
    }
    l.set_valign(gtk::Align::Center);
    l.set_halign(gtk::Align::Center);
    l
}

/// A round avatar (PersonPicture): initials of `name`, or the "user" glyph, `size` px.
pub fn avatar(name: &str, size: i32) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.add_css_class("w11-avatar");
    b.set_size_request(size, size);
    b.set_hexpand(false);
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Center);
    let initials: String = name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect();
    if initials.is_empty() {
        let g = icons::glyph("user", (size / 2).max(12));
        g.set_hexpand(true);
        b.append(&g);
    } else {
        let l = gtk::Label::new(Some(&initials));
        l.set_hexpand(true);
        let attrs = gtk::pango::AttrList::new();
        attrs.insert(gtk::pango::AttrSize::new_size_absolute((size as f64 * 0.4 * gtk::pango::SCALE as f64) as i32));
        l.set_attributes(Some(&attrs));
        b.append(&l);
    }
    b
}

/// An empty state (`.fl-empty`): a 64 px disc with a glyph, a title, a message and an optional action.
pub fn empty_state(glyph: &str, title: &str, message: &str, action: Option<&gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 6);
    b.add_css_class("w11-empty");
    b.set_halign(gtk::Align::Center);
    b.set_valign(gtk::Align::Center);
    let disc = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    disc.add_css_class("w11-empty-icon");
    disc.set_halign(gtk::Align::Center);
    disc.set_hexpand(false);
    let g = icons::glyph_stroke(if glyph.is_empty() { "info" } else { glyph }, 28, 1.2);
    g.set_hexpand(true);
    disc.append(&g);
    b.append(&disc);
    if !title.is_empty() {
        let l = super::label(title, "w11-empty-title w11-body-strong", true);
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        b.append(&l);
    }
    if !message.is_empty() {
        let l = super::label(message, "w11-empty-message", true);
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        l.set_max_width_chars(56);
        b.append(&l);
    }
    if let Some(a) = action {
        a.add_css_class("w11-empty-action");
        a.set_halign(gtk::Align::Center);
        b.append(a);
    }
    b
}

/// A page title (28 px Display semibold) with an optional description (`.fl-page-header`).
pub fn page_header(title: &str, description: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
    b.add_css_class("w11-page-header");
    b.append(&super::label(title, "w11-title", true));
    if !description.is_empty() {
        b.append(&super::label(description, "w11-page-header-desc", true));
    }
    b
}

/// A list row (components.js ListItem / `.fl-list-row`): glyph, title, description and a trailing widget, for
/// gtk::ListBox rows or list factories.
pub fn list_item(glyph: &str, title: &str, description: &str, trailing: Option<&gtk::Widget>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    b.add_css_class("w11-list-row");
    if !glyph.is_empty() {
        let g = if icons::has_glyph(glyph) { icons::glyph(glyph, 16) } else { icons::app_icon(glyph, 16) };
        g.set_valign(gtk::Align::Center);
        b.append(&g);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&super::label(title, "", false));
    if !description.is_empty() {
        text.append(&super::label(description, "w11-list-row-desc", false));
    }
    b.append(&text);
    if let Some(t) = trailing {
        t.set_valign(gtk::Align::Center);
        b.append(t);
    }
    b
}

/// A gtk::ListBox styled as a Win11 list (`.w11-list`).
pub fn list_box() -> gtk::ListBox {
    let l = gtk::ListBox::new();
    l.add_css_class("w11-list");
    l.set_selection_mode(gtk::SelectionMode::None);
    l
}

/// The characters a breadcrumb's current page keeps at least (its whole title when shorter), and an earlier item.
const CURRENT_CHARS: usize = 24;
const ITEM_CHARS: usize = 3;

struct CrumbInner {
    title_size: bool,
    handlers: RefCell<Vec<Rc<dyn Fn(&str, usize)>>>,
}

/// A breadcrumb (`.fl-breadcrumb`): items separated by chevrons, the last one the current page; title size
/// (28 px) or body size.
#[derive(Clone)]
pub struct Breadcrumb {
    root: gtk::Box,
    inner: Rc<CrumbInner>,
}

impl Breadcrumb {
    /// `title_size` true: the page title breadcrumb of Settings; false: body text.
    pub fn new(title_size: bool) -> Breadcrumb {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("w11-breadcrumb");
        root.add_css_class(if title_size { "w11-breadcrumb-title" } else { "w11-breadcrumb-body" });
        let inner = Rc::new(CrumbInner { title_size, handlers: RefCell::new(Vec::new()) });
        super::own(&root, &inner);
        Breadcrumb { root, inner }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// (id, label) items, the last one current. When they do not fit, the earlier items give way first (each down
    /// to about three characters and "…"), and the current page keeps its whole title up to about 24 characters, as
    /// Windows' BreadcrumbBar keeps the current page.
    pub fn set_items(&self, items: &[(&str, &str)]) {
        clear(&self.root);
        let n = items.len();
        for (i, (id, label)) in items.iter().enumerate() {
            if i > 0 {
                let sep = icons::glyph_stroke("chevron-right", if self.inner.title_size { 16 } else { 12 }, if self.inner.title_size { 1.6 } else { 1.35 });
                sep.add_css_class("w11-breadcrumb-sep");
                self.root.append(&sep);
            }
            if i == n - 1 {
                let l = gtk::Label::new(Some(label));
                l.add_css_class("w11-breadcrumb-current");
                // a title up to 24 characters is never shortened; a longer one keeps 24 and ends in "…" when it must
                if label.chars().count() > CURRENT_CHARS {
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    l.set_width_chars(CURRENT_CHARS as i32);
                }
                l.set_xalign(0.0);
                self.root.append(&l);
            } else {
                // the button keeps its label (the dev channel reads it); its label child may shrink to "Ab…"
                let b = gtk::Button::with_label(label);
                if let Some(l) = b.child().and_downcast::<gtk::Label>() {
                    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
                    l.set_width_chars(label.chars().count().min(ITEM_CHARS) as i32);
                }
                b.add_css_class("w11-breadcrumb-item");
                b.set_widget_name(&format!("crumb-{id}"));
                let id = id.to_string();
                let weak = Rc::downgrade(&self.inner);
                b.connect_clicked(move |_| {
                    if let Some(inner) = weak.upgrade() {
                        let hs: Vec<Rc<dyn Fn(&str, usize)>> = inner.handlers.borrow().clone();
                        for h in hs {
                            h(&id, i);
                        }
                    }
                });
                self.root.append(&b);
            }
        }
    }

    /// Run `f(id, index)` when an earlier item is clicked.
    pub fn connect_selected(&self, f: impl Fn(&str, usize) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }
}

struct SwatchInner {
    buttons: RefCell<Vec<(String, gtk::Button)>>,
    handlers: RefCell<Vec<Rc<dyn Fn(&str)>>>,
}

/// The Windows accent colour grid (components.js ColorSwatchGrid): 40 px swatches, 4 px apart, the selected one
/// ringed with a check.
#[derive(Clone)]
pub struct ColorSwatches {
    root: gtk::Grid,
    inner: Rc<SwatchInner>,
}

impl ColorSwatches {
    /// `colors` None: theme::ACCENT_PRESETS; `columns` 8 in Settings.
    pub fn new(colors: Option<&[&str]>, selected: Option<&str>, columns: i32, size: i32) -> ColorSwatches {
        let root = gtk::Grid::new();
        root.set_row_spacing(4);
        root.set_column_spacing(4);
        let colors: Vec<String> = colors.map(|c| c.iter().map(|s| s.to_string()).collect()).unwrap_or_else(|| crate::theme::ACCENT_PRESETS.iter().map(|s| s.to_string()).collect());
        let inner = Rc::new(SwatchInner { buttons: RefCell::new(Vec::new()), handlers: RefCell::new(Vec::new()) });
        super::own(&root, &inner);
        let provider = gtk::CssProvider::new();
        let mut css = String::new();
        // the colour classes are per grid (the sheet is global): two grids must not paint each other's swatches
        thread_local!(static GRIDS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) });
        let grid_id = GRIDS.with(|g| {
            g.set(g.get() + 1);
            g.get()
        });
        for (i, c) in colors.iter().enumerate() {
            let b = gtk::Button::new();
            b.add_css_class("w11-swatch");
            let class = format!("w11-swatch-{grid_id}-{i}");
            b.add_css_class(&class);
            css.push_str(&format!("button.w11-swatch.{class} {{ background-color: {c}; }}\n"));
            b.set_size_request(size, size);
            b.set_tooltip_text(Some(&format!("{} {}", t("Kleur", "Colour"), c.to_uppercase())));
            let check = icons::glyph_stroke("check", 16, 2.2);
            b.set_child(Some(&check));
            check.set_visible(false);
            let cv = c.clone();
            let weak = Rc::downgrade(&inner);
            b.connect_clicked(move |_| {
                if let Some(inner) = weak.upgrade() {
                    swatch_select(&inner, &cv);
                    let hs: Vec<Rc<dyn Fn(&str)>> = inner.handlers.borrow().clone();
                    for h in hs {
                        h(&cv);
                    }
                }
            });
            let cols = columns.max(1);
            root.attach(&b, i as i32 % cols, i as i32 / cols, 1, 1);
            inner.buttons.borrow_mut().push((c.clone(), b));
        }
        provider.load_from_string(&css);
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 3);
            // the grid's colours go with it
            root.connect_destroy(move |_| gtk::style_context_remove_provider_for_display(&display, &provider));
        }
        let me = ColorSwatches { root, inner };
        if let Some(s) = selected {
            me.set_selected(s);
        }
        me
    }

    pub fn widget(&self) -> &gtk::Grid {
        &self.root
    }

    /// Mark a colour (case-insensitive) without running the handlers.
    pub fn set_selected(&self, color: &str) {
        swatch_select(&self.inner, color);
    }

    /// Run `f(color)` when a swatch is clicked.
    pub fn connect_selected(&self, f: impl Fn(&str) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }
}

fn swatch_select(inner: &SwatchInner, color: &str) {
    {
        let want = crate::theme::normalize_hex(color);
        for (c, b) in inner.buttons.borrow().iter() {
            let on = crate::theme::normalize_hex(c) == want;
            if on {
                b.add_css_class("w11-selected");
            } else {
                b.remove_css_class("w11-selected");
            }
            if let Some(child) = b.child() {
                child.set_visible(on);
            }
        }
    }
}
