//! The app's own widgets: the proportional flex row of the partition bar (`FlexBar`, flex-grow by share with
//! min-width), the bar segment that hides its lines when narrow (`Segment`, disks.css's container queries), the
//! page clamp (`.dk-page` max-width, and the width for the media queries), plus small builders: a text field
//! with an error line, the usage bar, a radio with a description, the details grid.
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene, gsk};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// Unparent every child, also the ones the widget did not add itself (a menu's popover parented on it).
fn unparent_all(w: &gtk::Widget) {
    while let Some(c) = w.first_child() {
        c.unparent();
    }
}

/// Position the popovers parented on a custom widget (menus); GTK wants this from the parent's size_allocate.
fn present_popovers(w: &gtk::Widget) {
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(p) = child.downcast_ref::<gtk::Popover>()
            && p.is_visible()
        {
            p.present();
        }
        c = child.next_sibling();
    }
}

fn at(x: f64, y: f64) -> Option<gsk::Transform> {
    Some(gsk::Transform::new().translate(&graphene::Point::new(x as f32, y as f32)))
}

// ------------------------------------------------------------------------------------------------ FlexBar

mod flex_imp {
    use super::*;

    #[derive(Default)]
    pub struct FlexBar {
        pub items: RefCell<Vec<(gtk::Widget, f64)>>,
        pub gap: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FlexBar {
        const NAME: &'static str = "DisksFlexBar";
        type Type = super::FlexBar;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("dkbar");
        }
    }

    impl ObjectImpl for FlexBar {
        fn dispose(&self) {
            self.items.borrow_mut().clear();
            unparent_all(self.obj().upcast_ref());
        }
    }

    impl WidgetImpl for FlexBar {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let items = self.items.borrow();
            let visible: Vec<&gtk::Widget> = items.iter().map(|(w, _)| w).filter(|w| w.is_visible()).collect();
            if orientation == gtk::Orientation::Horizontal {
                let gaps = self.gap.get() * (visible.len().saturating_sub(1)) as i32;
                let (mut min, mut nat) = (gaps, gaps);
                for w in &visible {
                    let (m, n, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
                    min += m;
                    nat += n;
                }
                (min, nat, -1, -1)
            } else {
                let (mut min, mut nat) = (0, 0);
                for w in &visible {
                    let (m, n, _, _) = w.measure(gtk::Orientation::Vertical, -1);
                    min = min.max(m);
                    nat = nat.max(n);
                }
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            present_popovers(self.obj().upcast_ref());
            let items: Vec<(gtk::Widget, f64, f64)> = self
                .items
                .borrow()
                .iter()
                .filter(|(w, _)| w.is_visible())
                .map(|(w, weight)| (w.clone(), weight.max(0.0), w.measure(gtk::Orientation::Horizontal, -1).0 as f64))
                .collect();
            let n = items.len();
            if n == 0 {
                return;
            }
            let avail = (width - self.gap.get() * (n as i32 - 1)).max(0) as f64;
            // flex-grow by weight from a zero basis, but never below a child's minimum (flexbox min-width)
            let mut fixed: Vec<Option<f64>> = vec![None; n];
            loop {
                let used: f64 = fixed.iter().flatten().sum();
                let wsum: f64 = items.iter().zip(&fixed).filter(|(_, f)| f.is_none()).map(|((_, w, _), _)| *w).sum();
                let free = (avail - used).max(0.0);
                let mut changed = false;
                for (i, (_, weight, min)) in items.iter().enumerate() {
                    if fixed[i].is_none() {
                        let share = if wsum > 0.0 { free * weight / wsum } else { 0.0 };
                        if share < *min {
                            fixed[i] = Some(*min);
                            changed = true;
                        }
                    }
                }
                if !changed {
                    break;
                }
            }
            let used: f64 = fixed.iter().flatten().sum();
            let wsum: f64 = items.iter().zip(&fixed).filter(|(_, f)| f.is_none()).map(|((_, w, _), _)| *w).sum();
            let free = (avail - used).max(0.0);
            let mut x: f64 = 0.0;
            for (i, (w, weight, min)) in items.iter().enumerate() {
                let cw = fixed[i].unwrap_or(if wsum > 0.0 { free * weight / wsum } else { 0.0 });
                let x0 = x.round();
                let x1 = (x + cw).round();
                let alloc = ((x1 - x0) as i32).max(*min as i32);
                w.allocate(alloc, height, -1, at(x0, 0.0));
                x += cw + self.gap.get() as f64;
            }
        }
    }
}

glib::wrapper! {
    /// Children side by side, `gap` px apart, each as wide as its weight's share (Schijfbeheer's proportional
    /// bar), never narrower than its minimum.
    pub struct FlexBar(ObjectSubclass<flex_imp::FlexBar>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl FlexBar {
    pub fn new(gap: i32) -> FlexBar {
        let b: FlexBar = glib::Object::new();
        b.imp().gap.set(gap);
        b
    }

    pub fn add(&self, child: &impl IsA<gtk::Widget>, weight: f64) {
        child.set_parent(self);
        self.imp().items.borrow_mut().push((child.as_ref().clone(), weight));
        self.queue_resize();
    }
}

// ------------------------------------------------------------------------------------------------ Segment

mod seg_imp {
    use super::*;

    #[derive(Default)]
    pub struct Segment {
        pub parts: RefCell<Vec<gtk::Widget>>,
        pub progress: RefCell<Option<gtk::Widget>>,
        pub fraction: Cell<f64>,
        pub mode: Cell<u8>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Segment {
        const NAME: &'static str = "DisksSegment";
        type Type = super::Segment;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("dkseg");
            klass.set_activate_signal_from_name("activate");
        }
    }

    impl ObjectImpl for Segment {
        fn signals() -> &'static [glib::subclass::Signal] {
            static SIGNALS: std::sync::OnceLock<Vec<glib::subclass::Signal>> = std::sync::OnceLock::new();
            SIGNALS.get_or_init(|| vec![glib::subclass::Signal::builder("activate").action().build()])
        }

        fn dispose(&self) {
            self.parts.borrow_mut().clear();
            self.progress.borrow_mut().take();
            unparent_all(self.obj().upcast_ref());
        }
    }

    impl WidgetImpl for Segment {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::ConstantSize
        }

        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let parts = self.parts.borrow();
            if orientation == gtk::Orientation::Horizontal {
                // min-width: 0 (the bar decides); the natural width fits the longest line
                let nat = parts.iter().map(|w| w.measure(orientation, -1).1).max().unwrap_or(0) + 18;
                (0, nat, -1, -1)
            } else {
                let h: i32 = parts.iter().map(|w| w.measure(orientation, -1).1).sum::<i32>() + parts.len().saturating_sub(1) as i32 + 13;
                (h, h, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            present_popovers(self.obj().upcast_ref());
            // @container (max-width: 64px): no lines, 4 px sides; (max-width: 28px): no body
            let mode = if width <= 28 { 0 } else if width <= 64 { 1 } else { 2 };
            self.mode.set(mode);
            let pad = if mode == 2 { 9 } else { 4 };
            let mut y = 6;
            for w in self.parts.borrow().iter() {
                let (min, _, _, _) = w.measure(gtk::Orientation::Horizontal, -1);
                let cw = min.max(width - 2 * pad);
                let (_, ch, _, _) = w.measure(gtk::Orientation::Vertical, cw);
                w.allocate(cw, ch, -1, at(pad as f64, y as f64));
                y += ch + 1;
            }
            if let Some(p) = self.progress.borrow().as_ref() {
                let pw = (width as f64 * self.fraction.get().clamp(0.0, 1.0)).round() as i32;
                p.allocate(pw.max(0), 3, -1, at(0.0, (height - 3) as f64));
            }
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let obj = self.obj();
            let mode = self.mode.get();
            for (i, w) in self.parts.borrow().iter().enumerate() {
                if (i == 0 && mode >= 1) || mode == 2 {
                    obj.snapshot_child(w, snapshot);
                }
            }
            if let Some(p) = self.progress.borrow().as_ref() {
                obj.snapshot_child(p, snapshot);
            }
        }
    }
}

glib::wrapper! {
    /// One segment of the partition bar: the name row and its lines stacked, cut off when narrow.
    pub struct Segment(ObjectSubclass<seg_imp::Segment>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Segment {
    /// `parts`: the name row first, then the lines; `progress`: a running job's fraction (a 3 px bar).
    pub fn new(parts: Vec<gtk::Widget>, progress: Option<f64>) -> Segment {
        let s: Segment = glib::Object::new();
        for p in &parts {
            p.set_parent(&s);
        }
        *s.imp().parts.borrow_mut() = parts;
        if let Some(f) = progress {
            let bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            bar.add_css_class("dk-seg-progress");
            bar.set_parent(&s);
            s.imp().fraction.set(f);
            *s.imp().progress.borrow_mut() = Some(bar.upcast());
        }
        s.set_overflow(gtk::Overflow::Hidden);
        s
    }

    /// Run `f` for the activate signal (the dev channel's "click").
    pub fn connect_activate(&self, f: impl Fn() + 'static) {
        self.connect_local("activate", false, move |_| {
            f();
            None
        });
    }
}

// ------------------------------------------------------------------------------------------------ Clamp

mod clamp_imp {
    use super::*;

    #[derive(Default)]
    pub struct Clamp {
        pub child: RefCell<Option<gtk::Widget>>,
        pub max: Cell<i32>,
        pub last: Cell<i32>,
        pub on_width: RefCell<Option<Rc<dyn Fn(i32)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "DisksClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("dkclamp");
        }
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            self.child.borrow_mut().take();
            unparent_all(self.obj().upcast_ref());
        }
    }

    impl WidgetImpl for Clamp {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(c) = self.child.borrow().clone() else { return (0, 0, -1, -1) };
            let max = self.max.get();
            if orientation == gtk::Orientation::Horizontal {
                let (min, nat, _, _) = c.measure(orientation, for_size);
                (min, nat.min(max).max(min), -1, -1)
            } else {
                let w = if for_size >= 0 { for_size.min(max) } else { -1 };
                let (min, nat, _, _) = c.measure(orientation, w);
                (min, nat, -1, -1)
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            present_popovers(self.obj().upcast_ref());
            if let Some(c) = self.child.borrow().as_ref() {
                let (min, _, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                let cw = width.min(self.max.get()).max(min);
                let x = ((width - cw) / 2).max(0);
                c.allocate(cw, height, -1, at(x as f64, 0.0));
            }
            if self.last.replace(width) != width {
                let cb = self.on_width.borrow().clone();
                if let Some(cb) = cb {
                    glib::idle_add_local_once(move || cb(width));
                }
            }
        }
    }
}

glib::wrapper! {
    /// A child at most `max` px wide, centred (`.dk-page` max-width), telling the width it got.
    pub struct Clamp(ObjectSubclass<clamp_imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    pub fn new(max: i32) -> Clamp {
        let c: Clamp = glib::Object::new();
        c.imp().max.set(max);
        c.set_hexpand(true);
        c
    }

    pub fn set_child(&self, child: &impl IsA<gtk::Widget>) {
        if let Some(old) = self.imp().child.borrow_mut().take() {
            old.unparent();
        }
        child.set_parent(self);
        *self.imp().child.borrow_mut() = Some(child.as_ref().clone());
        self.queue_resize();
    }

    /// `f(width)` after an allocation with another width (on idle).
    pub fn connect_width(&self, f: impl Fn(i32) + 'static) {
        *self.imp().on_width.borrow_mut() = Some(Rc::new(f));
    }
}

// ------------------------------------------------------------------------------------------------ builders

/// A label with classes (ellipsized unless `wrap`).
pub fn label(text: &str, class: &str, wrap: bool) -> gtk::Label {
    kit::controls::label(text, class, wrap)
}

/// A horizontal or vertical box with classes.
pub fn hbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

pub fn vbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

/// components.js TextBox: header, the entry, a description line that shows an error instead (setError).
#[derive(Clone)]
pub struct Field {
    pub root: gtk::Box,
    pub entry: gtk::Entry,
    desc: gtk::Label,
    description: Rc<RefCell<String>>,
}

impl Field {
    pub fn new(header: &str, value: &str, description: &str) -> Field {
        let root = vbox(8, "w11-field");
        if !header.is_empty() {
            root.append(&label(header, "w11-field-header", true));
        }
        let inner = vbox(4, "");
        let entry = kit::controls::text_box("");
        entry.set_text(value);
        entry.set_hexpand(true);
        inner.append(&entry);
        let desc = label(description, "w11-field-desc", true);
        desc.set_visible(!description.is_empty());
        inner.append(&desc);
        root.append(&inner);
        Field { root, entry, desc, description: Rc::new(RefCell::new(description.to_owned())) }
    }

    pub fn password(header: &str) -> Field {
        let f = Field::new(header, "", "");
        f.entry.set_visibility(false);
        f.entry.set_input_purpose(gtk::InputPurpose::Password);
        f
    }

    pub fn text(&self) -> String {
        self.entry.text().to_string()
    }

    /// An error message under the box (empty: the description again).
    pub fn set_error(&self, msg: &str) {
        let d = self.description.borrow().clone();
        if msg.is_empty() {
            self.root.remove_css_class("w11-error");
            self.entry.remove_css_class("w11-error");
            self.desc.set_label(&d);
            self.desc.set_visible(!d.is_empty());
        } else {
            self.root.add_css_class("w11-error");
            self.entry.add_css_class("w11-error");
            self.desc.set_label(msg);
            self.desc.set_visible(true);
        }
    }
}

/// overview.js UsageBar: a 4 px (large: 8 px) track with the used share in the accent, red from 90 %.
pub fn usage_bar(used: f64, total: f64, large: bool) -> gtk::DrawingArea {
    let pct = if total > 0.0 { (used / total * 100.0).clamp(0.0, 100.0) } else { 0.0 };
    let a = gtk::DrawingArea::new();
    a.add_css_class("dk-usage");
    let h = if large { 8 } else { 4 };
    a.set_content_height(h);
    a.set_hexpand(true);
    a.set_valign(gtk::Align::Center);
    a.set_draw_func(move |_, cr, w, hh| {
        let (w, hh) = (w as f64, hh as f64);
        let r = hh / 2.0;
        let pill = |cr: &gtk::cairo::Context, x: f64, ww: f64| {
            if ww <= 0.0 {
                return;
            }
            let rr = r.min(ww / 2.0);
            cr.new_sub_path();
            cr.arc(x + ww - rr, rr, rr, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
            cr.arc(x + rr, hh - rr, rr, std::f64::consts::FRAC_PI_2, 1.5 * std::f64::consts::PI);
            cr.close_path();
        };
        let set = |cr: &gtk::cairo::Context, c: gtk::gdk::RGBA| cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        set(cr, kit::theme::token("chart-track"));
        pill(cr, 0.0, w);
        let _ = cr.fill();
        // the fill keeps the track's rounded ends (overflow hidden, border-radius inherit)
        cr.save().ok();
        pill(cr, 0.0, w);
        cr.clip();
        set(cr, if pct >= 90.0 { kit::theme::token("sys-critical") } else { kit::theme::token("accent") });
        pill(cr, 0.0, w * pct / 100.0);
        let _ = cr.fill();
        cr.restore().ok();
    });
    kit::theme::track(&a);
    a
}

/// A radio button with a description under its label (components.js RadioGroup option.description).
pub fn radio_desc(label_text: &str, description: &str, group: Option<&gtk::CheckButton>) -> gtk::CheckButton {
    let c = gtk::CheckButton::new();
    c.add_css_class("w11-radio");
    c.add_css_class("dk-radio");
    let b = vbox(0, "");
    b.append(&label(label_text, "", true));
    b.append(&label(description, "w11-caption w11-text-2", true));
    c.set_child(Some(&b));
    if let Some(g) = group {
        c.set_group(Some(g));
    }
    c
}

/// A value of the details grid: text, mono text, or a widget.
pub enum Detail {
    Text(String),
    Mono(String),
    Widget(gtk::Widget),
}

/// util.js detailsGrid: key and value columns (8 px rows, 28 px apart, keys at least 140 px); empty values are
/// skipped.
pub fn details_grid(rows: Vec<Option<(String, Detail)>>) -> gtk::Grid {
    let g = gtk::Grid::new();
    g.add_css_class("dk-details");
    g.set_row_spacing(8);
    g.set_column_spacing(28);
    let mut r = 0;
    for (k, v) in rows.into_iter().flatten() {
        let widget: gtk::Widget = match v {
            Detail::Text(t) | Detail::Mono(t) if t.is_empty() => continue,
            Detail::Text(t) => {
                let l = label(&t, "dk-details-v", true);
                l.set_tooltip_text(Some(&t));
                l.set_halign(gtk::Align::Fill);
                l.upcast()
            }
            Detail::Mono(t) => {
                let l = label(&t, "dk-details-v dk-mono", true);
                l.set_tooltip_text(Some(&t));
                l.set_halign(gtk::Align::Fill);
                l.upcast()
            }
            Detail::Widget(w) => w,
        };
        let kl = label(&k, "dk-details-k", true);
        kl.set_size_request(140, -1);
        kl.set_valign(gtk::Align::Start);
        widget.set_hexpand(true);
        widget.set_valign(gtk::Align::Start);
        g.attach(&kl, 0, r, 1, 1);
        g.attach(&widget, 1, r, 1, 1);
        r += 1;
    }
    g
}

/// The first descendant label with a CSS class (to restyle a kit card's description).
pub fn find_label(w: &gtk::Widget, class: &str) -> Option<gtk::Label> {
    if let Some(l) = w.downcast_ref::<gtk::Label>()
        && l.has_css_class(class)
    {
        return Some(l.clone());
    }
    let mut c = w.first_child();
    while let Some(child) = c {
        if let Some(found) = find_label(&child, class) {
            return Some(found);
        }
        c = child.next_sibling();
    }
    None
}
