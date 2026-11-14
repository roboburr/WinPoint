//! Layout pieces CSS has and GTK lacks: `Clamp` (a column with a max-width, left aligned, that can slide its
//! child up and down without a relayout, for the page enter animation), `Wrap` (flex-wrap: children left to
//! right, on as many lines as needed, growing hexpand children) and `enter` (the fade-and-rise of a new page).
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{glib, graphene, gsk};
use std::cell::{Cell, RefCell};
use std::time::Duration;

// ------------------------------------------------------------------------------------------------- Clamp

mod clamp_imp {
    use super::*;

    #[derive(Default)]
    pub struct Clamp {
        pub child: RefCell<Option<gtk::Widget>>,
        pub max: Cell<i32>,
        pub offset: Cell<f32>,
        pub width: Cell<i32>,
        pub on_width: RefCell<Option<std::rc::Rc<dyn Fn(i32)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Clamp {
        const NAME: &'static str = "StSettingsClamp";
        type Type = super::Clamp;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Clamp {
        fn dispose(&self) {
            if let Some(c) = self.child.borrow_mut().take() {
                c.unparent();
            }
        }
    }

    impl WidgetImpl for Clamp {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let Some(child) = self.child.borrow().clone().filter(|c| c.is_visible()) else { return (0, 0, -1, -1) };
            let max = self.max.get();
            match orientation {
                gtk::Orientation::Horizontal => {
                    let (min, nat, _, _) = child.measure(orientation, for_size);
                    let nat = if max > 0 { nat.min(max) } else { nat };
                    (min, nat.max(min), -1, -1)
                }
                _ => {
                    let w = if for_size >= 0 && max > 0 { for_size.min(max) } else { for_size };
                    let w = if w >= 0 { w.max(child.measure(gtk::Orientation::Horizontal, -1).0) } else { w };
                    let (min, nat, _, _) = child.measure(orientation, w);
                    (min, nat, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            if self.width.replace(width) != width
                && let Some(f) = self.on_width.borrow().clone()
            {
                // not during the allocation: the callback may change CSS classes
                glib::idle_add_local_once(move || f(width));
            }
            let Some(child) = self.child.borrow().clone().filter(|c| c.is_visible()) else { return };
            let max = self.max.get();
            let min = child.measure(gtk::Orientation::Horizontal, -1).0;
            let w = if max > 0 { width.min(max) } else { width }.max(min);
            let off = self.offset.get();
            let transform = if off.abs() > 0.01 { Some(gsk::Transform::new().translate(&graphene::Point::new(0.0, off))) } else { None };
            child.allocate(w, height, baseline, transform);
        }
    }
}

glib::wrapper! {
    /// A single-child container that is at most `max` px wide (CSS max-width, left aligned).
    pub struct Clamp(ObjectSubclass<clamp_imp::Clamp>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Clamp {
    /// `max` 0: no limit.
    pub fn new(child: &impl IsA<gtk::Widget>, max: i32) -> Clamp {
        let c: Clamp = glib::Object::new();
        c.imp().max.set(max);
        c.set_child(child);
        c
    }

    pub fn set_child(&self, child: &impl IsA<gtk::Widget>) {
        if let Some(old) = self.imp().child.borrow_mut().take() {
            old.unparent();
        }
        child.as_ref().set_parent(self);
        *self.imp().child.borrow_mut() = Some(child.as_ref().clone());
        self.queue_resize();
    }

    pub fn child(&self) -> Option<gtk::Widget> {
        self.imp().child.borrow().clone()
    }

    /// Run `f(width)` (after the allocation) whenever the allocated width changes.
    pub fn connect_width(&self, f: impl Fn(i32) + 'static) {
        *self.imp().on_width.borrow_mut() = Some(std::rc::Rc::new(f));
    }

    /// Draw the child `dy` px lower than its place (no relayout).
    pub fn set_offset(&self, dy: f32) {
        self.imp().offset.set(dy);
        self.queue_allocate();
    }
}

/// The page enter animation (main.js headIn / body): fade in while rising `dy` px, `ms` long, with the
/// cubic-bezier(0, 0, 0, 1) ease. Nothing moves with reduced motion.
pub fn enter(widget: &Clamp, dy: f32, ms: u64) {
    if kit::ctx().reduce_motion() {
        widget.set_opacity(1.0);
        widget.set_offset(0.0);
        return;
    }
    widget.set_opacity(0.0);
    widget.set_offset(dy);
    // the end state also without frames (a hidden window, an offscreen render)
    let w2 = widget.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(ms + 120), move || {
        if let Some(w) = w2.upgrade() {
            w.set_opacity(1.0);
            w.set_offset(0.0);
        }
    });
    let start = Cell::new(None::<i64>);
    widget.add_tick_callback(move |w, clock| {
        let now = clock.frame_time();
        let t0 = start.get().unwrap_or_else(|| {
            start.set(Some(now));
            now
        });
        let t = ((now - t0) as f64 / 1000.0 / ms as f64).clamp(0.0, 1.0);
        // close to cubic-bezier(0, 0, 0, 1): very fast at first, settling softly
        let e = 1.0 - (1.0 - t).powi(5);
        w.set_opacity(e.min(1.0));
        w.set_offset(dy * (1.0 - e) as f32);
        if t >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

// -------------------------------------------------------------------------------------------------- Wrap

mod wrap_imp {
    use super::*;

    #[derive(Default)]
    pub struct Wrap {
        pub children: RefCell<Vec<gtk::Widget>>,
        pub hgap: Cell<i32>,
        pub vgap: Cell<i32>,
        pub justify_end: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Wrap {
        const NAME: &'static str = "StSettingsWrap";
        type Type = super::Wrap;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Wrap {
        fn dispose(&self) {
            for c in self.children.borrow_mut().drain(..) {
                c.unparent();
            }
        }
    }

    /// One placed child: (widget, x, width) per line, with the line's height.
    pub struct Line {
        pub items: Vec<(gtk::Widget, i32, i32)>,
        pub height: i32,
    }

    impl Wrap {
        fn visible(&self) -> Vec<gtk::Widget> {
            self.children.borrow().iter().filter(|c| c.is_visible()).cloned().collect()
        }

        pub fn lines(&self, width: i32) -> Vec<Line> {
            let gap = self.hgap.get();
            let mut lines: Vec<Vec<(gtk::Widget, i32)>> = Vec::new();
            let mut cur: Vec<(gtk::Widget, i32)> = Vec::new();
            let mut x = 0;
            for c in self.visible() {
                let (min, nat, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                // a growing child with a width request breaks lines at that width (CSS flex: 1 1 <basis>)
                let basis = if c.hexpands() && c.width_request() > 0 { c.width_request().max(min) } else { nat };
                let w = basis.min(width.max(min)).max(min);
                if !cur.is_empty() && x + gap + w > width {
                    lines.push(std::mem::take(&mut cur));
                    x = 0;
                }
                x += if cur.is_empty() { w } else { gap + w };
                cur.push((c, w));
            }
            if !cur.is_empty() {
                lines.push(cur);
            }
            lines
                .into_iter()
                .map(|line| {
                    let used: i32 = line.iter().map(|(_, w)| *w).sum::<i32>() + gap * (line.len() as i32 - 1).max(0);
                    let extra = (width - used).max(0);
                    let growing = line.iter().filter(|(c, _)| c.hexpands()).count() as i32;
                    let mut x = if growing == 0 && self.justify_end.get() { extra } else { 0 };
                    let mut items = Vec::new();
                    let mut height = 0;
                    let mut left = extra;
                    let mut n = growing;
                    for (c, w) in line {
                        let mut w = w;
                        if growing > 0 && c.hexpands() {
                            let add = left / n.max(1);
                            w += add;
                            left -= add;
                            n -= 1;
                        }
                        let (_, h, _, _) = c.measure(gtk::Orientation::Vertical, w);
                        height = height.max(h);
                        items.push((c, x, w));
                        x += w + gap;
                    }
                    Line { items, height }
                })
                .collect()
        }
    }

    impl WidgetImpl for Wrap {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let kids = self.visible();
            if kids.is_empty() {
                return (0, 0, -1, -1);
            }
            match orientation {
                gtk::Orientation::Horizontal => {
                    let mut min = 0;
                    let mut nat = 0;
                    for c in &kids {
                        let (m, n, _, _) = c.measure(orientation, -1);
                        min = min.max(m);
                        nat += n;
                    }
                    nat += self.hgap.get() * (kids.len() as i32 - 1);
                    (min, nat.max(min), -1, -1)
                }
                _ => {
                    let width = if for_size >= 0 {
                        for_size
                    } else {
                        self.measure(gtk::Orientation::Horizontal, -1).1
                    };
                    let lines = self.lines(width);
                    let h: i32 = lines.iter().map(|l| l.height).sum::<i32>() + self.vgap.get() * (lines.len() as i32 - 1).max(0);
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let mut y = 0;
            for line in self.lines(width) {
                for (c, x, w) in &line.items {
                    let (_, h, _, _) = c.measure(gtk::Orientation::Vertical, *w);
                    let (cy, ch) = match c.valign() {
                        gtk::Align::Fill => (y, line.height),
                        gtk::Align::Start => (y, h),
                        gtk::Align::End => (y + line.height - h, h),
                        _ => (y + (line.height - h) / 2, h),
                    };
                    c.size_allocate(&gtk::Allocation::new(*x, cy, *w, ch), -1);
                }
                y += line.height + self.vgap.get();
            }
        }
    }
}

glib::wrapper! {
    /// CSS flex-wrap: children left to right on as many lines as the width needs; hexpand children share a
    /// line's leftover width; `justify_end` puts the others at the end (justify-content: flex-end).
    pub struct Wrap(ObjectSubclass<wrap_imp::Wrap>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Wrap {
    pub fn new(hgap: i32, vgap: i32) -> Wrap {
        let w: Wrap = glib::Object::new();
        w.imp().hgap.set(hgap);
        w.imp().vgap.set(vgap);
        w
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.as_ref().set_parent(self);
        self.imp().children.borrow_mut().push(child.as_ref().clone());
        self.queue_resize();
    }

    pub fn remove_all(&self) {
        for c in self.imp().children.borrow_mut().drain(..) {
            c.unparent();
        }
        self.queue_resize();
    }

    pub fn set_justify_end(&self, on: bool) {
        self.imp().justify_end.set(on);
        self.queue_allocate();
    }
}

// ---------------------------------------------------------------------------------------------- AutoGrid

mod grid_imp {
    use super::*;

    #[derive(Default)]
    pub struct AutoGrid {
        pub children: RefCell<Vec<gtk::Widget>>,
        pub min: Cell<i32>,
        pub gap: Cell<i32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for AutoGrid {
        const NAME: &'static str = "StSettingsAutoGrid";
        type Type = super::AutoGrid;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for AutoGrid {
        fn dispose(&self) {
            for c in self.children.borrow_mut().drain(..) {
                c.unparent();
            }
        }
    }

    impl AutoGrid {
        fn visible(&self) -> Vec<gtk::Widget> {
            self.children.borrow().iter().filter(|c| c.is_visible()).cloned().collect()
        }

        /// (columns, column width) for a width: as many `min`-wide columns as fit (repeat(auto-fit, minmax(min, 1fr))).
        pub fn columns(&self, width: i32, n: usize) -> (usize, i32) {
            let (min, gap) = (self.min.get().max(1), self.gap.get());
            let fit = ((width + gap) / (min + gap)).max(1) as usize;
            let cols = fit.min(n.max(1));
            let cw = (width - gap * (cols as i32 - 1)) / cols as i32;
            (cols, cw.max(1))
        }

        /// The rows' heights at a width.
        pub fn rows(&self, width: i32) -> (usize, i32, Vec<i32>) {
            let kids = self.visible();
            let (cols, cw) = self.columns(width, kids.len());
            let heights = kids
                .chunks(cols)
                .map(|row| row.iter().map(|c| c.measure(gtk::Orientation::Vertical, cw).1).max().unwrap_or(0))
                .collect();
            (cols, cw, heights)
        }
    }

    impl WidgetImpl for AutoGrid {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let kids = self.visible();
            if kids.is_empty() {
                return (0, 0, -1, -1);
            }
            match orientation {
                gtk::Orientation::Horizontal => {
                    let min = kids.iter().map(|c| c.measure(orientation, -1).0).max().unwrap_or(0);
                    let nat = (self.min.get().max(min)) * kids.len() as i32 + self.gap.get() * (kids.len() as i32 - 1);
                    (min, nat.max(min), -1, -1)
                }
                _ => {
                    let width = if for_size >= 0 { for_size } else { self.min.get() };
                    let (_, _, rows) = self.rows(width);
                    let h = rows.iter().sum::<i32>() + self.gap.get() * (rows.len() as i32 - 1).max(0);
                    (h, h, -1, -1)
                }
            }
        }

        fn size_allocate(&self, width: i32, _height: i32, _baseline: i32) {
            let (cols, cw, rows) = self.rows(width);
            let gap = self.gap.get();
            let mut y = 0;
            for (r, row) in self.visible().chunks(cols).enumerate() {
                for (i, c) in row.iter().enumerate() {
                    let h = c.measure(gtk::Orientation::Vertical, cw).1;
                    let h = if c.valign() == gtk::Align::Fill { rows[r] } else { h };
                    c.size_allocate(&gtk::Allocation::new(i as i32 * (cw + gap), y, cw, h), -1);
                }
                y += rows[r] + gap;
            }
        }
    }
}

glib::wrapper! {
    /// CSS `grid-template-columns: repeat(auto-fit, minmax(<min>, 1fr)); align-items: start`: as many equal
    /// columns as fit, each row as high as its tallest child, children at their own height.
    pub struct AutoGrid(ObjectSubclass<grid_imp::AutoGrid>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl AutoGrid {
    pub fn new(min: i32, gap: i32) -> AutoGrid {
        let g: AutoGrid = glib::Object::new();
        g.imp().min.set(min);
        g.imp().gap.set(gap);
        g
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.as_ref().set_parent(self);
        self.imp().children.borrow_mut().push(child.as_ref().clone());
        self.queue_resize();
    }
}

/// Run `f` once after `ms` (a glib timeout that ignores its source id).
pub fn after(ms: u64, f: impl FnOnce() + 'static) {
    glib::timeout_add_local_once(Duration::from_millis(ms), f);
}
