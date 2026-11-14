//! Layout helpers of Samenvatting, Prestaties and Opslag: what css/taskmgr.css does with CSS grid, flex-wrap,
//! max-width and container queries, as one small GTK widget (`Layout`), plus the bars, dots and swatches the
//! HTML drew with inline `style="width: …; background: …"`.
//!
//! - `Layout::columns(hgap, vgap, f)`: `f(width)` gives the column widths; the children fill the columns row by
//!   row (grid-template-columns, also `repeat(auto-fill, minmax(…))` with `auto_fill`, a clamp with one column
//!   `w.min(max)`, a two-column split that stacks when narrow).
//! - `Layout::wrap(hgap, vgap)`: children at their natural width, wrapping onto the next line (flex-wrap).
//! - `.heights(min, nat)`: a single child with this minimum and natural height (min-height / max-height of a
//!   flex item); `.valign(..)`: how children sit in a taller row (align-items); `.centered()`: the columns are
//!   centred (align-items: center of a column flexbox).
//! - `connect_width(f)`: runs after the width changed (once per change, after the layout pass), for the
//!   container-query changes a grid cannot express (labels that hide, alignment that flips).
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// How children sit in a row that is taller than they are.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VAlign {
    /// Stretched to the row (align-items: stretch, the grid default).
    #[default]
    Fill,
    Start,
    Center,
}

type ColsFn = Rc<dyn Fn(i32) -> Vec<i32>>;

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Layout {
        pub cols: RefCell<Option<ColsFn>>,
        pub wrap: Cell<bool>,
        pub hgap: Cell<i32>,
        pub vgap: Cell<i32>,
        pub valign: Cell<VAlign>,
        pub center: Cell<bool>,
        pub heights: Cell<Option<(i32, i32)>>,
        pub width: Cell<i32>,
        pub pending: Cell<bool>,
        pub on_width: RefCell<Vec<Rc<dyn Fn(i32)>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Layout {
        const NAME: &'static str = "TaskmgrLayout";
        type Type = super::Layout;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Layout {
        fn dispose(&self) {
            while let Some(child) = self.obj().first_child() {
                child.unparent();
            }
        }
    }

    impl WidgetImpl for Layout {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            let kids = self.obj().visible_children();
            if orientation == gtk::Orientation::Horizontal {
                let mut min = 0;
                let mut nat = 0;
                let mut sum = 0;
                for (i, c) in kids.iter().enumerate() {
                    let (m, n, _, _) = c.measure(orientation, -1);
                    min = min.max(m);
                    nat = nat.max(n);
                    sum += n + if i > 0 { self.hgap.get() } else { 0 };
                }
                if self.wrap.get() {
                    nat = sum.max(min);
                }
                return (min, nat.max(min), -1, -1);
            }
            let width = if for_size >= 0 { for_size } else { self.measure(gtk::Orientation::Horizontal, -1).1 };
            let rows = self.rows(&kids, width);
            let gaps = self.vgap.get() * (rows.len() as i32 - 1).max(0);
            let min: i32 = rows.iter().map(|r| r.min_h).sum::<i32>() + gaps;
            let nat: i32 = rows.iter().map(|r| r.nat_h).sum::<i32>() + gaps;
            match self.heights.get() {
                Some((hmin, hnat)) => {
                    let m = min.max(hmin);
                    (m, hnat.max(m), -1, -1)
                }
                None => (min, nat.max(min), -1, -1),
            }
        }

        fn size_allocate(&self, width: i32, height: i32, _baseline: i32) {
            let kids = self.obj().visible_children();
            let rows = self.rows(&kids, width);
            let fill_one = self.heights.get().is_some() && rows.len() == 1;
            let mut y = 0;
            for row in &rows {
                let row_h = if fill_one { height.max(row.min_h) } else { row.nat_h };
                for cell in &row.cells {
                    let child = &kids[cell.index];
                    let (h, cy) = match self.valign.get() {
                        VAlign::Fill => (row_h, y),
                        VAlign::Start => (cell.nat_h.min(row_h), y),
                        VAlign::Center => {
                            let h = cell.nat_h.min(row_h);
                            (h, y + (row_h - h) / 2)
                        }
                    };
                    child.size_allocate(&gtk::Allocation::new(cell.x, cy, cell.w, h.max(cell.min_h)), -1);
                }
                y += row_h + self.vgap.get();
            }
            if self.width.replace(width) != width && !self.on_width.borrow().is_empty() && !self.pending.replace(true) {
                let weak = self.obj().downgrade();
                glib::idle_add_local_once(move || {
                    let Some(obj) = weak.upgrade() else { return };
                    let imp = obj.imp();
                    imp.pending.set(false);
                    let w = imp.width.get();
                    let fs: Vec<Rc<dyn Fn(i32)>> = imp.on_width.borrow().clone();
                    for f in fs {
                        f(w);
                    }
                });
            }
        }
    }

    pub struct Cell_ {
        pub index: usize,
        pub x: i32,
        pub w: i32,
        pub min_h: i32,
        pub nat_h: i32,
    }

    pub struct Row {
        pub cells: Vec<Cell_>,
        pub min_h: i32,
        pub nat_h: i32,
    }

    impl Layout {
        /// The rows for this width: which child goes where, and how tall each row is.
        pub fn rows(&self, kids: &[gtk::Widget], width: i32) -> Vec<Row> {
            let mut rows: Vec<Row> = Vec::new();
            if kids.is_empty() {
                return rows;
            }
            let hgap = self.hgap.get();
            let mut push = |cells: Vec<Cell_>| {
                let min_h = cells.iter().map(|c| c.min_h).max().unwrap_or(0);
                let nat_h = cells.iter().map(|c| c.nat_h).max().unwrap_or(0);
                rows.push(Row { cells, min_h, nat_h });
            };
            let measure = |c: &gtk::Widget, w: i32| -> (i32, i32, i32) {
                let (wmin, _, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                let w = w.max(wmin).max(1);
                let (hmin, hnat, _, _) = c.measure(gtk::Orientation::Vertical, w);
                (w, hmin, hnat)
            };
            if self.wrap.get() {
                let mut cells = Vec::new();
                let mut x = 0;
                for (i, c) in kids.iter().enumerate() {
                    let (_, nat, _, _) = c.measure(gtk::Orientation::Horizontal, -1);
                    let want = nat.min(width.max(1));
                    if x > 0 && x + want > width {
                        push(std::mem::take(&mut cells));
                        x = 0;
                    }
                    let (w, hmin, hnat) = measure(c, want);
                    cells.push(Cell_ { index: i, x, w, min_h: hmin, nat_h: hnat });
                    x += w + hgap;
                }
                if !cells.is_empty() {
                    push(cells);
                }
                return rows;
            }
            let f = self.cols.borrow().clone();
            let mut widths = f.map(|f| f(width)).unwrap_or_else(|| vec![width]);
            widths.retain(|w| *w > 0);
            if widths.is_empty() {
                widths.push(width.max(1));
            }
            let total: i32 = widths.iter().sum::<i32>() + hgap * (widths.len() as i32 - 1);
            let x0 = if self.center.get() { ((width - total) / 2).max(0) } else { 0 };
            for (r, chunk) in kids.chunks(widths.len()).enumerate() {
                let mut cells = Vec::new();
                let mut x = x0;
                for (j, c) in chunk.iter().enumerate() {
                    let (w, hmin, hnat) = measure(c, widths[j]);
                    cells.push(Cell_ { index: r * widths.len() + j, x, w, min_h: hmin, nat_h: hnat });
                    x += widths[j] + hgap;
                }
                push(cells);
            }
            rows
        }
    }
}

glib::wrapper! {
    /// See the module docs.
    pub struct Layout(ObjectSubclass<imp::Layout>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Layout {
    fn make(hgap: i32, vgap: i32) -> Layout {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let l: Layout = glib::Object::new();
        l.imp().hgap.set(hgap);
        l.imp().vgap.set(vgap);
        l
    }

    /// Columns by width: `f(width)` returns the column widths (the gaps come between them).
    pub fn columns(hgap: i32, vgap: i32, f: impl Fn(i32) -> Vec<i32> + 'static) -> Layout {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let l = Layout::make(hgap, vgap);
        *l.imp().cols.borrow_mut() = Some(Rc::new(f));
        l
    }

    /// One column of at most `max` px, left-aligned (max-width).
    pub fn clamp(max: i32) -> Layout {
        Layout::columns(0, 0, move |w| vec![w.min(max)])
    }

    /// `repeat(auto-fill, minmax(min, 1fr))` with this gap.
    pub fn auto_fill(min: i32, gap: i32) -> Layout {
        Layout::columns(gap, gap, move |w| {
            let n = ((w + gap) / (min + gap)).max(1);
            equal(w, n, gap)
        })
    }

    /// Children at their natural width, wrapping (flex-wrap: wrap).
    pub fn wrap(hgap: i32, vgap: i32) -> Layout {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let l = Layout::make(hgap, vgap);
        l.imp().wrap.set(true);
        l
    }

    /// A single child's minimum and natural height (the flex item's min-height and max-height).
    pub fn heights(self, min: i32, nat: i32) -> Layout {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().heights.set(Some((min, nat)));
        self
    }

    pub fn valign_children(self, v: VAlign) -> Layout {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().valign.set(v);
        self
    }

    /// Centre the columns when they are narrower than the widget.
    pub fn centered(self) -> Layout {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().center.set(true);
        self
    }

    pub fn append(&self, child: &impl IsA<gtk::Widget>) {
        child.as_ref().set_parent(self);
        self.queue_resize();
    }

    /// Remove every child.
    pub fn clear(&self) {
        while let Some(c) = self.first_child() {
            c.unparent();
        }
        self.queue_resize();
    }

    /// Run `f(width)` after the width changed (after the layout pass).
    pub fn connect_width(&self, f: impl Fn(i32) + 'static) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        self.imp().on_width.borrow_mut().push(Rc::new(f));
    }

    fn visible_children(&self) -> Vec<gtk::Widget> {
        let mut out = Vec::new();
        let mut c = self.first_child();
        while let Some(w) = c {
            if w.is_visible() {
                out.push(w.clone());
            }
            c = w.next_sibling();
        }
        out
    }
}

/// `n` equal columns over `w` px with `gap` between them.
pub fn equal(w: i32, n: i32, gap: i32) -> Vec<i32> {
    let n = n.max(1);
    let room = (w - gap * (n - 1)).max(n);
    let base = room / n;
    let extra = room - base * n;
    (0..n).map(|i| base + if i < extra { 1 } else { 0 }).collect()
}

// ------------------------------------------------------------------------------------------- small pieces

/// The display font of css/taskmgr.css (`--font-display`), for cairo / pango text.
pub const DISPLAY_FONT: &str = "Segoe UI Variable Display, Segoe UI Variable, Segoe UI, Selawik, Inter Display, Inter, Noto Sans, Ubuntu Sans, sans-serif";

/// A font size token of the kit ("fs-body" -> 14.0), scaled by the text scale like the CSS.
pub fn fs(token: &str, fallback: f64) -> f64 {
    kit::theme::token_str(token)
        .and_then(|v| v.trim().trim_end_matches("px").trim().parse::<f64>().ok())
        .filter(|v| v.is_finite() && *v > 0.0)
        .unwrap_or(fallback)
}

/// A colour for cairo: a kit token ("chart-cpu", "accent", "text-2"), an app token ("tm-cat-other") or any CSS
/// colour, in the theme in effect.
pub fn color(name: &str) -> gdk::RGBA {
    let resolved = crate::util::chart_color(name);
    let (r, g, b, a) = kit::chart::resolve(&resolved);
    gdk::RGBA::new(r as f32, g as f32, b as f32, a as f32)
}

pub fn set_source(cr: &gtk::cairo::Context, c: &gdk::RGBA) {
    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
}

/// `color-mix(in srgb, a p, b)`: p of `a` (0..1) over `b`.
pub fn mix(a: &gdk::RGBA, p: f64, b: &gdk::RGBA) -> gdk::RGBA {
    let p = p.clamp(0.0, 1.0) as f32;
    gdk::RGBA::new(
        a.red() * p + b.red() * (1.0 - p),
        a.green() * p + b.green() * (1.0 - p),
        a.blue() * p + b.blue() * (1.0 - p),
        a.alpha() * p + b.alpha() * (1.0 - p),
    )
}

/// A rounded rectangle path.
pub fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.arc(x + r, y + r, r, PI, PI + FRAC_PI_2);
    cr.close_path();
}

/// One coloured part of a `Bar`: its share (0..1 of the bar), a colour function (read at draw time, so it
/// follows the theme) and a tooltip.
pub struct Seg {
    pub frac: f64,
    pub color: Rc<dyn Fn() -> gdk::RGBA>,
    pub tip: String,
}

impl Seg {
    pub fn new(frac: f64, color: impl Fn() -> gdk::RGBA + 'static, tip: &str) -> Seg {
        Seg { frac: if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 }, color: Rc::new(color), tip: tip.to_owned() }
    }

    /// A part in a named colour (`color(name)` at draw time).
    pub fn named(frac: f64, name: &str, tip: &str) -> Seg {
        let n = name.to_owned();
        Seg::new(frac, move || color(&n), tip)
    }
}

struct BarInner {
    segs: RefCell<Vec<Seg>>,
    gap: Cell<f64>,
}

/// A pill-shaped bar on the chart track (`.tm-bar`, `.tm-vol__bar`, `.tm-cat__bar`, `.tm-xrow__bar`, ...): the
/// parts from the left, `gap` px apart, each with its tooltip.
#[derive(Clone)]
pub struct Bar {
    area: gtk::DrawingArea,
    inner: Rc<BarInner>,
}

impl Bar {
    pub fn new(height: i32, class: &str) -> Bar {
        let area = gtk::DrawingArea::new();
        area.set_content_height(height);
        area.set_hexpand(true);
        area.set_valign(gtk::Align::Center);
        if !class.is_empty() {
            area.add_css_class(class);
        }
        let inner = Rc::new(BarInner { segs: RefCell::new(Vec::new()), gap: Cell::new(0.0) });
        // the widget owns the state (the handle may be dropped): the closures hold it, it does not hold the widget
        let st = inner.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let inner = &st;
            let (w, h) = (w as f64, h as f64);
            rounded(cr, 0.0, 0.0, w, h, h / 2.0);
            cr.clip();
            set_source(cr, &color("chart-track"));
            cr.paint().ok();
            let mut x = 0.0;
            let gap = inner.gap.get();
            for s in inner.segs.borrow().iter() {
                let sw = s.frac * w;
                if sw <= 0.0 {
                    continue;
                }
                set_source(cr, &(s.color)());
                if inner.segs.borrow().len() == 1 {
                    // a single fill keeps its round end (border-radius: inherit)
                    rounded(cr, 0.0, 0.0, sw, h, h / 2.0);
                } else {
                    cr.rectangle(x, 0.0, sw, h);
                }
                cr.fill().ok();
                x += sw + gap;
            }
        });
        area.set_has_tooltip(true);
        let st = inner.clone();
        area.connect_query_tooltip(move |a, x, _, _, tip| {
            let inner = &st;
            let w = a.width() as f64;
            let mut at = 0.0;
            for s in inner.segs.borrow().iter() {
                let sw = s.frac * w;
                if (x as f64) >= at && (x as f64) < at + sw + inner.gap.get() && !s.tip.is_empty() {
                    tip.set_text(Some(&s.tip));
                    return true;
                }
                at += sw + inner.gap.get();
            }
            false
        });
        kit::theme::track(&area);
        Bar { area, inner }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn set(&self, segs: Vec<Seg>) {
        *self.inner.segs.borrow_mut() = segs;
        self.area.queue_draw();
    }

    /// One fill of `frac` in a named colour.
    pub fn set_fill(&self, frac: f64, name: &str) {
        self.set(vec![Seg::named(frac, name, "")]);
    }

    /// Px between the parts (`.tm-vol__bar { gap: 1px }`).
    pub fn set_gap(&self, gap: f64) {
        self.inner.gap.set(gap);
    }
}

/// A round dot of `px` in a colour read at draw time (`.tm-mini__dot`, `.tm-cat__subdot`).
pub fn dot(px: i32, color: impl Fn() -> gdk::RGBA + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(px);
    a.set_content_height(px);
    a.set_valign(gtk::Align::Center);
    a.set_halign(gtk::Align::Center);
    a.set_draw_func(move |_, cr, w, h| {
        set_source(cr, &color());
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0, 0.0, std::f64::consts::TAU);
        cr.fill().ok();
    });
    kit::theme::track(&a);
    a
}

/// A rounded colour block of `w` x `h` with an optional white glyph in it (`.tm-cat__swatch`, `.tm-xrow__swatch`).
pub fn swatch(w: i32, h: i32, radius: f64, glyph: Option<(&str, f64)>, color: impl Fn() -> gdk::RGBA + 'static) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(w);
    a.set_content_height(h);
    a.set_valign(gtk::Align::Center);
    a.set_halign(gtk::Align::Center);
    let glyph = glyph.map(|(g, s)| (g.to_owned(), s));
    a.set_draw_func(move |_, cr, w, h| {
        set_source(cr, &color());
        rounded(cr, 0.0, 0.0, w as f64, h as f64, radius);
        cr.fill().ok();
        if let Some((g, s)) = &glyph {
            let (x, y) = ((w as f64 - s) / 2.0, (h as f64 - s) / 2.0);
            kit::icons::draw_glyph(cr, g, x, y, *s, &gdk::RGBA::WHITE);
        }
    });
    kit::theme::track(&a);
    a
}

/// A dash line key of a graph legend (`.tm-legend__line`, 18 x 2, solid or dashed).
pub fn legend_line(color_name: &str, dashed: bool) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(18);
    a.set_content_height(8);
    a.set_valign(gtk::Align::Center);
    let name = color_name.to_owned();
    a.set_draw_func(move |_, cr, w, h| {
        set_source(cr, &color(&name));
        cr.set_line_width(2.0);
        if dashed {
            cr.set_dash(&[4.0, 2.0], 0.0);
        }
        let y = (h as f64 / 2.0).round();
        cr.move_to(0.0, y);
        cr.line_to(w as f64, y);
        cr.stroke().ok();
    });
    kit::theme::track(&a);
    a
}

/// A label with classes (ellipsized).
pub fn label(text: &str, class: &str) -> gtk::Label {
    kit::controls::label(text, class, false)
}

/// A wrapping label with classes.
pub fn wrap(text: &str, class: &str) -> gtk::Label {
    kit::controls::label(text, class, true)
}

/// A vertical or horizontal box with classes.
pub fn vbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

pub fn hbox(spacing: i32, class: &str) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b
}

/// A clickable card or row (a `<button>` of the HTML): a flat button with the classes and `child`.
pub fn flat_button(class: &str, child: &impl IsA<gtk::Widget>) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("taskmgr-flat");
    for c in class.split_whitespace() {
        b.add_css_class(c);
    }
    b.set_child(Some(child.as_ref()));
    b
}

/// The section link "Alle processen >" (`.fl-link.tm-section__link`).
pub fn section_link(text: &str) -> gtk::Button {
    let b = kit::controls::hyperlink(text, Some("chevron-right"));
    b.add_css_class("taskmgr-section-link");
    b
}

/// A section title (`.tm-section__title`).
pub fn section_title(text: &str) -> gtk::Label {
    let l = wrap(text, "taskmgr-section-title");
    l.set_halign(gtk::Align::Start);
    l
}

/// A subtle command button with a glyph (`Button({variant: 'subtle', icon})`).
pub fn subtle(glyph: &str, text: &str) -> gtk::Button {
    kit::controls::button_with_glyph(glyph, text, kit::controls::Variant::Subtle)
}

/// Relabel a button made with a glyph and a label.
pub fn relabel(b: &gtk::Button, text: &str) {
    if let Some(l) = crate::util::first_label(b.upcast_ref()) {
        l.set_label(text);
    }
}

/// The entrance of a view (dom.js animate: fading in from `dy` px lower over `ms`); none with reduced motion.
pub fn fade_in(w: &impl IsA<gtk::Widget>, dy: i32, ms: u32) {
    let w = w.as_ref().clone();
    if kit::ctx().reduce_motion() {
        return;
    }
    w.set_opacity(0.0);
    let base = w.margin_top();
    w.set_margin_top(base + dy);
    // without frames (a hidden window) the view still ends up in place
    let wk = w.downgrade();
    glib::timeout_add_local_once(Duration::from_millis(ms as u64 + 150), move || {
        if let Some(w) = wk.upgrade() {
            w.set_opacity(1.0);
            w.set_margin_top(base);
        }
    });
    let t0 = glib::monotonic_time();
    let total = ms as f64 * 1000.0;
    w.add_tick_callback(move |w, clock| {
        let p = ((clock.frame_time() - t0) as f64 / total).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - p).powi(4);
        w.set_opacity(e);
        w.set_margin_top(base + (dy as f64 * (1.0 - e)).round() as i32);
        if p >= 1.0 { glib::ControlFlow::Break } else { glib::ControlFlow::Continue }
    });
}

/// A scrolled list area that grows with its content up to `max` px, then scrolls (max-height + overflow-y).
pub fn max_scroll(child: &impl IsA<gtk::Widget>, max: i32) -> gtk::ScrolledWindow {
    let sw = gtk::ScrolledWindow::new();
    sw.set_hscrollbar_policy(gtk::PolicyType::Never);
    sw.set_vscrollbar_policy(gtk::PolicyType::Automatic);
    sw.set_propagate_natural_height(true);
    sw.set_max_content_height(max);
    sw.set_child(Some(child.as_ref()));
    sw
}

/// A small progress ring with an optional caption under it (`.tm-list__empty` with a ring).
pub fn ring_note(size: i32, text: &str) -> gtk::Box {
    let b = vbox(8, "taskmgr-list-empty");
    b.set_halign(gtk::Align::Center);
    b.append(kit::controls::ProgressRing::new(size, None).widget());
    if !text.is_empty() {
        let l = wrap(text, "w11-caption");
        l.set_justify(gtk::Justification::Center);
        l.set_xalign(0.5);
        b.append(&l);
    }
    b
}

/// The calm empty line of a list (`.tm-list__empty`).
pub fn empty_line(text: &str) -> gtk::Label {
    let l = wrap(text, "taskmgr-list-empty");
    l.set_justify(gtk::Justification::Center);
    l.set_xalign(0.5);
    l.set_hexpand(true);
    l
}

/// `s` without a leading word `word` (any case) and the white space after it; None when it does not start so.
pub fn strip_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let head = s.get(..word.len())?;
    let rest = s.get(word.len()..)?;
    (head.eq_ignore_ascii_case(word) && rest.starts_with(char::is_whitespace)).then(|| rest.trim_start())
}

/// Copy text to the clipboard.
pub fn copy_text(w: &impl IsA<gtk::Widget>, text: &str) {
    w.as_ref().clipboard().set_text(text);
}

// ------------------------------------------------------------------------------------------ cover thumbnail

mod cover_imp {
    use super::*;
    use gtk::subclass::prelude::*;

    #[derive(Default)]
    pub struct Cover {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub w: Cell<i32>,
        pub h: Cell<i32>,
        pub radius: Cell<f32>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Cover {
        const NAME: &'static str = "TaskmgrCover";
        type Type = super::Cover;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for Cover {}

    impl WidgetImpl for Cover {
        fn measure(&self, orientation: gtk::Orientation, _for_size: i32) -> (i32, i32, i32, i32) {
            let s = if orientation == gtk::Orientation::Horizontal { self.w.get() } else { self.h.get() };
            (s, s, -1, -1)
        }

        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let Some(tex) = self.texture.borrow().clone() else { return };
            let obj = self.obj();
            let (w, h) = (obj.width() as f32, obj.height() as f32);
            let (tw, th) = (tex.width().max(1) as f32, tex.height().max(1) as f32);
            // object-fit: cover
            let scale = (w / tw).max(h / th);
            let (dw, dh) = (tw * scale, th * scale);
            let bounds = gtk::graphene::Rect::new(0.0, 0.0, w, h);
            snapshot.push_rounded_clip(&gtk::gsk::RoundedRect::from_rect(bounds, self.radius.get()));
            snapshot.append_texture(&tex, &gtk::graphene::Rect::new((w - dw) / 2.0, (h - dh) / 2.0, dw, dh));
            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// A fixed-size picture that fills its box (object-fit: cover) with rounded corners (`.tm-lf__thumb img`).
    pub struct Cover(ObjectSubclass<cover_imp::Cover>) @extends gtk::Widget, @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Cover {
    pub fn new(w: i32, h: i32, radius: f32) -> Cover {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        let c: Cover = glib::Object::new();
        c.imp().w.set(w);
        c.imp().h.set(h);
        c.imp().radius.set(radius);
        c
    }

    pub fn set_texture(&self, t: Option<&gdk::Texture>) {
        use gtk::subclass::prelude::ObjectSubclassIsExt;
        *self.imp().texture.borrow_mut() = t.cloned();
        self.queue_draw();
    }
}
