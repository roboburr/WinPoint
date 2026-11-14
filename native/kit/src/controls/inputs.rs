//! Text box, field header, text area, search box with suggestions (AutoSuggestBox), number box, slider with
//! its value, drop-down (ComboBox) (components.js TextBox, SearchBox, Slider, Dropdown).
use crate::i18n::{self, t};
use crate::icons;
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// A text box (`.fl-textbox`, 32 px) with a placeholder.
pub fn text_box(placeholder: &str) -> gtk::Entry {
    let e = gtk::Entry::new();
    e.add_css_class("w11-entry");
    if !placeholder.is_empty() {
        e.set_placeholder_text(Some(placeholder));
    }
    e
}

/// A multi-line text box in a scrolled window (`textarea.fl-textbox__input`), at least `min_height` px.
pub fn text_area(min_height: i32) -> (gtk::ScrolledWindow, gtk::TextView) {
    let tv = gtk::TextView::new();
    tv.add_css_class("w11-textarea");
    tv.set_wrap_mode(gtk::WrapMode::WordChar);
    let sw = gtk::ScrolledWindow::new();
    sw.set_child(Some(&tv));
    sw.set_min_content_height(min_height.max(60));
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    (sw, tv)
}

/// A control with a header above it and an optional description (or error) below (`.fl-field`).
pub fn field(header: &str, control: &impl IsA<gtk::Widget>, description: Option<&str>) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("w11-field");
    if !header.is_empty() {
        b.append(&super::label(header, "w11-field-header", false));
    }
    match description {
        Some(d) => {
            let inner = gtk::Box::new(gtk::Orientation::Vertical, 4);
            inner.append(control.as_ref());
            inner.append(&super::label(d, "w11-field-desc", true));
            b.append(&inner);
        }
        None => b.append(control.as_ref()),
    }
    b
}

/// A number box (WinUI NumberBox with inline spin buttons).
pub fn number_box(min: f64, max: f64, step: f64, value: f64, digits: u32) -> gtk::SpinButton {
    let s = gtk::SpinButton::with_range(min, max, if step > 0.0 { step } else { 1.0 });
    s.add_css_class("w11-numberbox");
    s.set_digits(digits);
    s.set_value(value);
    s.set_numeric(true);
    s
}

// ----------------------------------------------------------------------------------------------- search box

/// One suggestion of a search box.
#[derive(Clone, Debug, Default)]
pub struct Suggestion {
    pub value: String,
    pub label: String,
    pub description: String,
    pub glyph: String,
}

struct SearchInner {
    root: glib::WeakRef<gtk::Box>,
    text: gtk::Text,
    clear: gtk::Button,
    popover: gtk::Popover,
    list: gtk::Box,
    items: RefCell<Vec<(String, gtk::Button)>>,
    active: Cell<Option<usize>>,
    changed: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    submit: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    chosen: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    debounce: Cell<u32>,
    pending: RefCell<Option<glib::SourceId>>,
    silent: Cell<bool>,
    own_placeholder: bool,
}

/// A search box (`.fl-search`, AutoSuggestBox): the text, a clear button while focused with text, the search
/// glyph (submits), and an optional suggestion list below (`set_suggestions`). Enter submits (or picks the
/// highlighted suggestion), Up/Down move through the suggestions, Escape closes them.
#[derive(Clone)]
pub struct SearchBox {
    root: gtk::Box,
    inner: Rc<SearchInner>,
}

impl SearchBox {
    /// `placeholder` None: "Zoeken" / "Search", following the language.
    pub fn new(placeholder: Option<&str>) -> SearchBox {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.add_css_class("w11-searchbox");
        root.set_valign(gtk::Align::Center);
        root.set_hexpand(false);
        let text = gtk::Text::new();
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        if let Some(p) = placeholder {
            text.set_placeholder_text(Some(p));
        }
        let clear = gtk::Button::new();
        clear.add_css_class("w11-searchbox-button");
        clear.set_child(Some(&icons::glyph("close", 12)));
        clear.set_visible(false);
        clear.set_can_focus(false);
        i18n::tooltip(&clear, "Wissen", "Clear");
        let search = gtk::Button::new();
        search.add_css_class("w11-searchbox-button");
        search.set_child(Some(&icons::glyph_stroke("search", 14, 1.5)));
        search.set_can_focus(false);
        i18n::tooltip(&search, "Zoeken", "Search");
        root.append(&text);
        root.append(&clear);
        root.append(&search);
        let popover = gtk::Popover::new();
        popover.add_css_class("w11-popup");
        popover.set_has_arrow(false);
        popover.set_autohide(false);
        popover.set_can_focus(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_halign(gtk::Align::Start);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let sc = gtk::ScrolledWindow::new();
        sc.set_hscrollbar_policy(gtk::PolicyType::Never);
        sc.set_propagate_natural_height(true);
        sc.set_max_content_height(420);
        sc.set_child(Some(&list));
        popover.set_child(Some(&sc));
        popover.set_parent(&root);
        let inner = Rc::new(SearchInner {
            root: root.downgrade(),
            text: text.clone(),
            clear: clear.clone(),
            popover,
            list,
            items: RefCell::new(Vec::new()),
            active: Cell::new(None),
            changed: RefCell::new(Vec::new()),
            submit: RefCell::new(Vec::new()),
            chosen: RefCell::new(Vec::new()),
            debounce: Cell::new(0),
            pending: RefCell::new(None),
            silent: Cell::new(false),
            own_placeholder: placeholder.is_none(),
        });
        if inner.own_placeholder {
            i18n::bind(&text, |t2| t2.set_placeholder_text(Some(t("Zoeken", "Search"))));
        }
        super::own(&root, &inner);
        let me = SearchBox { root, inner };
        let weak = Rc::downgrade(&me.inner);
        text.connect_changed(move |_| {
            if let Some(me) = weak.upgrade().and_then(SearchBox::of) {
                me.on_changed();
            }
        });
        let weak = Rc::downgrade(&me.inner);
        clear.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.text.set_text("");
                inner.text.grab_focus();
            }
        });
        let weak = Rc::downgrade(&me.inner);
        search.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade().and_then(SearchBox::of) {
                me.do_submit();
            }
        });
        let weak = Rc::downgrade(&me.inner);
        text.connect_activate(move |_| {
            if let Some(me) = weak.upgrade().and_then(SearchBox::of) {
                if let Some(i) = me.inner.active.get()
                    && me.inner.popover.is_visible()
                {
                    let v = me.inner.items.borrow().get(i).map(|(v, _)| v.clone());
                    if let Some(v) = v {
                        me.choose(&v);
                        return;
                    }
                }
                me.do_submit();
            }
        });
        let focus = gtk::EventControllerFocus::new();
        let weak = Rc::downgrade(&me.inner);
        focus.connect_enter(move |_| {
            if let Some(inner) = weak.upgrade() {
                inner.clear.set_visible(!inner.text.text().is_empty());
                if let Some(r) = inner.root.upgrade() {
                    r.add_css_class("w11-focused");
                }
            }
        });
        let weak = Rc::downgrade(&me.inner);
        focus.connect_leave(move |_| {
            if let Some(inner) = weak.upgrade() {
                if let Some(r) = inner.root.upgrade() {
                    r.remove_css_class("w11-focused");
                }
                let w = Rc::downgrade(&inner);
                glib::timeout_add_local_once(Duration::from_millis(150), move || {
                    if let Some(inner) = w.upgrade()
                        && !inner.text.has_focus()
                    {
                        inner.clear.set_visible(false);
                        inner.popover.popdown();
                    }
                });
            }
        });
        text.add_controller(focus);
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&me.inner);
        keys.connect_key_pressed(move |_, key, _, _| {
            let Some(me) = weak.upgrade().and_then(SearchBox::of) else { return glib::Propagation::Proceed };
            let n = me.inner.items.borrow().len();
            match key {
                gdk::Key::Down | gdk::Key::Up if n > 0 && me.inner.popover.is_visible() => {
                    let cur = me.inner.active.get();
                    let next = match (key, cur) {
                        (gdk::Key::Down, None) => 0,
                        (gdk::Key::Down, Some(i)) => (i + 1).min(n - 1),
                        (_, None) => n - 1,
                        (_, Some(i)) => i.saturating_sub(1),
                    };
                    me.set_active(Some(next));
                    glib::Propagation::Stop
                }
                gdk::Key::Escape if me.inner.popover.is_visible() => {
                    me.inner.popover.popdown();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        text.add_controller(keys);
        me
    }

    fn of(inner: Rc<SearchInner>) -> Option<SearchBox> {
        let root = inner.root.upgrade()?;
        Some(SearchBox { root, inner })
    }

    fn on_changed(&self) {
        let text = self.inner.text.text().to_string();
        self.inner.clear.set_visible(!text.is_empty() && self.inner.text.has_focus());
        if self.inner.silent.get() {
            return;
        }
        if text.is_empty() {
            self.inner.popover.popdown();
        }
        let ms = self.inner.debounce.get();
        if ms == 0 {
            self.emit_changed();
            return;
        }
        if let Some(id) = self.inner.pending.borrow_mut().take() {
            id.remove();
        }
        let weak = Rc::downgrade(&self.inner);
        let id = glib::timeout_add_local_once(Duration::from_millis(ms as u64), move || {
            if let Some(me) = weak.upgrade().and_then(SearchBox::of) {
                me.inner.pending.borrow_mut().take();
                me.emit_changed();
            }
        });
        *self.inner.pending.borrow_mut() = Some(id);
    }

    fn emit_changed(&self) {
        let text = self.inner.text.text().to_string();
        let hs: Vec<Rc<dyn Fn(&str)>> = self.inner.changed.borrow().clone();
        for h in hs {
            h(&text);
        }
    }

    fn do_submit(&self) {
        self.inner.popover.popdown();
        let text = self.inner.text.text().to_string();
        let hs: Vec<Rc<dyn Fn(&str)>> = self.inner.submit.borrow().clone();
        for h in hs {
            h(&text);
        }
    }

    fn choose(&self, value: &str) {
        self.inner.popover.popdown();
        let hs: Vec<Rc<dyn Fn(&str)>> = self.inner.chosen.borrow().clone();
        for h in hs {
            h(value);
        }
    }

    fn set_active(&self, i: Option<usize>) {
        self.inner.active.set(i);
        for (j, (_, b)) in self.inner.items.borrow().iter().enumerate() {
            if Some(j) == i {
                b.add_css_class("w11-active");
            } else {
                b.remove_css_class("w11-active");
            }
        }
    }

    /// The whole control.
    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The text widget (widget name for the dev channel's "type").
    pub fn text_widget(&self) -> &gtk::Text {
        &self.inner.text
    }

    pub fn text(&self) -> String {
        self.inner.text.text().to_string()
    }

    /// Set the text without running the changed handlers.
    pub fn set_text(&self, text: &str) {
        self.inner.silent.set(true);
        self.inner.text.set_text(text);
        self.inner.silent.set(false);
    }

    pub fn set_placeholder(&self, text: &str) {
        self.inner.text.set_placeholder_text(Some(text));
    }

    /// Run the changed handlers only after `ms` without typing (0: at once, the default).
    pub fn set_debounce(&self, ms: u32) {
        self.inner.debounce.set(ms);
    }

    /// Run `f(text)` when the text changes (debounced if set).
    pub fn connect_changed(&self, f: impl Fn(&str) + 'static) {
        self.inner.changed.borrow_mut().push(Rc::new(f));
    }

    /// Run `f(text)` on Enter or the search glyph.
    pub fn connect_submit(&self, f: impl Fn(&str) + 'static) {
        self.inner.submit.borrow_mut().push(Rc::new(f));
    }

    /// Run `f(value)` when a suggestion is picked.
    pub fn connect_suggestion(&self, f: impl Fn(&str) + 'static) {
        self.inner.chosen.borrow_mut().push(Rc::new(f));
    }

    /// Show these suggestions below the box (empty: hide; `no_results` shows "Geen resultaten" instead).
    pub fn set_suggestions(&self, items: &[Suggestion], no_results: bool) {
        let list = &self.inner.list;
        super::misc::clear(list);
        let mut out = Vec::new();
        for s in items {
            let b = gtk::Button::new();
            b.add_css_class("w11-list-item");
            b.set_can_focus(false);
            let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            body.add_css_class("w11-item-body");
            if !s.glyph.is_empty() {
                body.append(&icons::glyph(&s.glyph, 16));
            }
            let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
            text.append(&super::label(&s.label, "", false));
            if !s.description.is_empty() {
                text.append(&super::label(&s.description, "w11-item-desc", false));
            }
            body.append(&text);
            b.set_child(Some(&body));
            let v = s.value.clone();
            let weak = Rc::downgrade(&self.inner);
            b.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade().and_then(SearchBox::of) {
                    me.choose(&v);
                }
            });
            list.append(&b);
            out.push((s.value.clone(), b));
        }
        if items.is_empty() && no_results {
            let l = i18n::label("Geen resultaten", "No results");
            l.add_css_class("w11-popup-empty");
            l.set_xalign(0.0);
            list.append(&l);
        }
        *self.inner.items.borrow_mut() = out;
        self.inner.active.set(None);
        let show = (!items.is_empty() || no_results) && self.inner.text.has_focus();
        if show {
            let w = self.root.width();
            if w > 0 {
                self.inner.list.set_size_request(w - 2, -1);
            }
            self.inner.popover.popup();
        } else {
            self.inner.popover.popdown();
        }
    }

    /// Give the text the focus.
    pub fn focus(&self) {
        self.inner.text.grab_focus();
    }
}

// --------------------------------------------------------------------------------------------------- slider

struct SliderInner {
    scale: gtk::Scale,
    value: gtk::Label,
    format: RefCell<Rc<dyn Fn(f64) -> String>>,
    silent: Cell<bool>,
    changed: RefCell<Vec<Rc<dyn Fn(f64)>>>,
    committed: RefCell<Vec<Rc<dyn Fn(f64)>>>,
    pressed: Cell<bool>,
    commit_timer: RefCell<Option<glib::SourceId>>,
}

/// A slider (`.fl-slider`: 4 px rail, accent fill, 20 px thumb) with its value as text after it.
/// `connect_changed` runs while dragging, `connect_committed` once the drag ends or a key step settles
/// (components.js onInput / onChange).
#[derive(Clone)]
pub struct Slider {
    root: gtk::Box,
    inner: Rc<SliderInner>,
}

impl Slider {
    pub fn new(min: f64, max: f64, step: f64, value: f64) -> Slider {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        root.add_css_class("w11-slider-row");
        let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, min, max, if step > 0.0 { step } else { 1.0 });
        scale.add_css_class("w11-slider");
        scale.set_draw_value(false);
        scale.set_hexpand(true);
        scale.set_value(value);
        let value_label = gtk::Label::new(None);
        value_label.add_css_class("w11-slider-value");
        value_label.set_xalign(1.0);
        root.append(&scale);
        root.append(&value_label);
        let digits = if step > 0.0 && step < 1.0 { 1 } else { 0 };
        let inner = Rc::new(SliderInner {
            scale: scale.clone(),
            value: value_label,
            format: RefCell::new(Rc::new(move |v| crate::format::number(v, digits))),
            silent: Cell::new(false),
            changed: RefCell::new(Vec::new()),
            committed: RefCell::new(Vec::new()),
            pressed: Cell::new(false),
            commit_timer: RefCell::new(None),
        });
        super::own(&root, &inner);
        let me = Slider { root, inner };
        me.render();
        let weak = Rc::downgrade(&me.inner);
        scale.connect_value_changed(move |s| {
            let Some(inner) = weak.upgrade() else { return };
            slider_render(&inner);
            if inner.silent.get() {
                return;
            }
            let v = s.value();
            let hs: Vec<Rc<dyn Fn(f64)>> = inner.changed.borrow().clone();
            for h in hs {
                h(v);
            }
            if !inner.pressed.get() {
                slider_schedule_commit(&inner);
            }
        });
        let click = gtk::GestureClick::new();
        click.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(&me.inner);
        click.connect_pressed(move |_, _, _, _| {
            if let Some(inner) = weak.upgrade() {
                inner.pressed.set(true);
            }
        });
        let weak = Rc::downgrade(&me.inner);
        let end = move || {
            if let Some(inner) = weak.upgrade() {
                inner.pressed.set(false);
                slider_commit(&inner);
            }
        };
        let e1 = end.clone();
        click.connect_released(move |_, _, _, _| e1());
        click.connect_stopped(move |_| end());
        scale.add_controller(click);
        me
    }

    fn render(&self) {
        slider_render(&self.inner);
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn scale(&self) -> &gtk::Scale {
        &self.inner.scale
    }

    pub fn value(&self) -> f64 {
        self.inner.scale.value()
    }

    /// Set without running the handlers.
    pub fn set_value(&self, v: f64) {
        self.inner.silent.set(true);
        self.inner.scale.set_value(v);
        self.inner.silent.set(false);
        self.render();
    }

    /// How the value text reads ("50", "50%", ...).
    pub fn set_format(&self, f: impl Fn(f64) -> String + 'static) {
        *self.inner.format.borrow_mut() = Rc::new(f);
        self.render();
    }

    /// Show or hide the value text.
    pub fn set_value_visible(&self, visible: bool) {
        self.inner.value.set_visible(visible);
    }

    pub fn connect_changed(&self, f: impl Fn(f64) + 'static) {
        self.inner.changed.borrow_mut().push(Rc::new(f));
    }

    pub fn connect_committed(&self, f: impl Fn(f64) + 'static) {
        self.inner.committed.borrow_mut().push(Rc::new(f));
    }

    pub fn set_sensitive(&self, s: bool) {
        self.inner.scale.set_sensitive(s);
        self.inner.value.set_sensitive(s);
    }
}

fn slider_render(inner: &SliderInner) {
    let v = inner.scale.value();
    let f = inner.format.borrow().clone();
    inner.value.set_label(&f(v));
}

fn slider_schedule_commit(inner: &Rc<SliderInner>) {
    if let Some(id) = inner.commit_timer.borrow_mut().take() {
        id.remove();
    }
    let weak = Rc::downgrade(inner);
    let id = glib::timeout_add_local_once(Duration::from_millis(300), move || {
        if let Some(inner) = weak.upgrade() {
            inner.commit_timer.borrow_mut().take();
            slider_commit(&inner);
        }
    });
    *inner.commit_timer.borrow_mut() = Some(id);
}

fn slider_commit(inner: &SliderInner) {
    if let Some(id) = inner.commit_timer.borrow_mut().take() {
        id.remove();
    }
    let v = inner.scale.value();
    let hs: Vec<Rc<dyn Fn(f64)>> = inner.committed.borrow().clone();
    for h in hs {
        h(v);
    }
}

// ------------------------------------------------------------------------------------------------- dropdown

/// One option of a drop-down: value, label, and optionally a glyph, an app icon and a description.
#[derive(Clone, Debug, Default)]
pub struct DropdownOption {
    pub value: String,
    pub label: String,
    pub glyph: String,
    pub app_icon: String,
    pub description: String,
    pub disabled: bool,
}

impl DropdownOption {
    pub fn new(value: &str, label: &str) -> DropdownOption {
        DropdownOption { value: value.into(), label: label.into(), ..Default::default() }
    }
    pub fn glyph(mut self, g: &str) -> DropdownOption {
        self.glyph = g.into();
        self
    }
    pub fn app_icon(mut self, icon: &str) -> DropdownOption {
        self.app_icon = icon.into();
        self
    }
    pub fn description(mut self, d: &str) -> DropdownOption {
        self.description = d.into();
        self
    }
}

struct DropdownInner {
    button: glib::WeakRef<gtk::Button>,
    icon_slot: gtk::Box,
    text: gtk::Label,
    options: RefCell<Vec<DropdownOption>>,
    selected: RefCell<Option<String>>,
    placeholder: RefCell<String>,
    handlers: RefCell<Vec<Rc<dyn Fn(&str)>>>,
}

/// A drop-down (`.fl-dropdown`, ComboBox): the selected option's text and a chevron; a click opens the options
/// in an Acrylic popup with the accent pill on the selected one.
#[derive(Clone)]
pub struct Dropdown {
    button: gtk::Button,
    inner: Rc<DropdownInner>,
}

impl Dropdown {
    pub fn new(options: Vec<DropdownOption>, selected: Option<&str>) -> Dropdown {
        let button = gtk::Button::new();
        button.add_css_class("w11-button");
        button.add_css_class("w11-dropdown");
        button.set_hexpand(false);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let icon_slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let text = gtk::Label::new(None);
        text.add_css_class("w11-dropdown-text");
        text.set_xalign(0.0);
        text.set_hexpand(true);
        text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let chev = icons::glyph("chevron-down", 12);
        chev.add_css_class("w11-dropdown-chevron");
        chev.set_margin_end(2);
        row.append(&icon_slot);
        row.append(&text);
        row.append(&chev);
        button.set_child(Some(&row));
        let inner = Rc::new(DropdownInner {
            button: button.downgrade(),
            icon_slot,
            text,
            options: RefCell::new(options),
            selected: RefCell::new(selected.map(str::to_owned)),
            placeholder: RefCell::new(String::new()),
            handlers: RefCell::new(Vec::new()),
        });
        super::own(&button, &inner);
        let me = Dropdown { button: button.clone(), inner };
        me.render();
        let weak = Rc::downgrade(&me.inner);
        button.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade().and_then(Dropdown::of) {
                me.open();
            }
        });
        let weak = Rc::downgrade(&me.inner);
        i18n::bind(&me.inner.text, move |_| {
            if let Some(me) = weak.upgrade().and_then(Dropdown::of) {
                me.render();
            }
        });
        me
    }

    /// From (value, label) pairs.
    pub fn simple(options: &[(&str, &str)], selected: Option<&str>) -> Dropdown {
        Dropdown::new(options.iter().map(|(v, l)| DropdownOption::new(v, l)).collect(), selected)
    }

    fn of(inner: Rc<DropdownInner>) -> Option<Dropdown> {
        let button = inner.button.upgrade()?;
        Some(Dropdown { button, inner })
    }

    fn render(&self) {
        let sel = self.inner.selected.borrow().clone();
        let opt = sel.as_ref().and_then(|v| self.inner.options.borrow().iter().find(|o| &o.value == v).cloned());
        super::misc::clear(&self.inner.icon_slot);
        match opt {
            Some(o) => {
                self.inner.text.set_label(&o.label);
                self.button.remove_css_class("w11-placeholder");
                if !o.app_icon.is_empty() {
                    self.inner.icon_slot.append(&icons::app_icon(&o.app_icon, 16));
                } else if !o.glyph.is_empty() {
                    self.inner.icon_slot.append(&icons::glyph(&o.glyph, 16));
                }
            }
            None => {
                let p = self.inner.placeholder.borrow().clone();
                self.inner.text.set_label(if p.is_empty() { t("Kies een optie", "Choose an option") } else { &p });
                self.button.add_css_class("w11-placeholder");
            }
        }
        self.inner.icon_slot.set_visible(self.inner.icon_slot.first_child().is_some());
    }

    fn open(&self) {
        let popover = gtk::Popover::new();
        popover.add_css_class("w11-popup");
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.set_halign(gtk::Align::Start);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let width = self.button.width().max(120);
        list.set_size_request(width - 2, -1);
        let sel = self.inner.selected.borrow().clone();
        let mut focus: Option<gtk::Button> = None;
        for o in self.inner.options.borrow().iter() {
            let b = gtk::Button::new();
            b.add_css_class("w11-list-item");
            let ov = gtk::Overlay::new();
            let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            body.add_css_class("w11-item-body");
            if !o.app_icon.is_empty() {
                body.append(&icons::app_icon(&o.app_icon, 16));
            } else if !o.glyph.is_empty() {
                body.append(&icons::glyph(&o.glyph, 16));
            }
            let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
            text.append(&super::label(&o.label, "", false));
            if !o.description.is_empty() {
                text.append(&super::label(&o.description, "w11-item-desc", false));
            }
            body.append(&text);
            ov.set_child(Some(&body));
            if sel.as_deref() == Some(o.value.as_str()) {
                b.add_css_class("w11-selected");
                let pill = gtk::Box::new(gtk::Orientation::Vertical, 0);
                pill.add_css_class("w11-pill");
                pill.set_halign(gtk::Align::Start);
                pill.set_valign(gtk::Align::Center);
                ov.add_overlay(&pill);
                focus = Some(b.clone());
            }
            b.set_child(Some(&ov));
            b.set_sensitive(!o.disabled);
            let v = o.value.clone();
            let weak = Rc::downgrade(&self.inner);
            let pw = popover.downgrade();
            b.connect_clicked(move |_| {
                if let Some(p) = pw.upgrade() {
                    p.popdown();
                }
                if let Some(me) = weak.upgrade().and_then(Dropdown::of) {
                    let changed = me.inner.selected.borrow().as_deref() != Some(v.as_str());
                    *me.inner.selected.borrow_mut() = Some(v.clone());
                    me.render();
                    if changed {
                        let hs: Vec<Rc<dyn Fn(&str)>> = me.inner.handlers.borrow().clone();
                        for h in hs {
                            h(&v);
                        }
                    }
                }
            });
            if focus.is_none() && o.value == sel.clone().unwrap_or_default() {
                focus = Some(b.clone());
            }
            list.append(&b);
        }
        let sc = gtk::ScrolledWindow::new();
        sc.set_hscrollbar_policy(gtk::PolicyType::Never);
        sc.set_propagate_natural_height(true);
        sc.set_max_content_height(520);
        sc.set_child(Some(&list));
        popover.set_child(Some(&sc));
        popover.set_parent(&self.button);
        popover.connect_closed(|p| {
            let p = p.clone();
            glib::idle_add_local_once(move || p.unparent());
        });
        popover.popup();
        if let Some(f) = focus {
            f.grab_focus();
        }
    }

    pub fn widget(&self) -> &gtk::Button {
        &self.button
    }

    /// Replace the options (the selection stays when its value is still there).
    pub fn set_options(&self, options: Vec<DropdownOption>) {
        *self.inner.options.borrow_mut() = options;
        self.render();
    }

    pub fn selected(&self) -> Option<String> {
        self.inner.selected.borrow().clone()
    }

    /// Select without running the handlers (None: the placeholder).
    pub fn set_selected(&self, value: Option<&str>) {
        *self.inner.selected.borrow_mut() = value.map(str::to_owned);
        self.render();
    }

    /// The text shown while nothing is selected (default "Kies een optie").
    pub fn set_placeholder(&self, text: &str) {
        *self.inner.placeholder.borrow_mut() = text.to_owned();
        self.render();
    }

    /// Run `f(value)` when the user picks another option.
    pub fn connect_changed(&self, f: impl Fn(&str) + 'static) {
        self.inner.handlers.borrow_mut().push(Rc::new(f));
    }
}
