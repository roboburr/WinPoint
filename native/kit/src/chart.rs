//! Charts after 6ebf169f:vendor/ui/js/core/charts.js, drawn with cairo and redrawn on theme changes: the streaming
//! line / area graph with its grid (Task Manager's performance graphs), the sparkline, the donut and the stacked bar.
//! Colours are token names of tokens.css ("chart-cpu", "chart-memory", "chart-disk", "chart-network",
//! "chart-gpu", "accent", "chart-1".."chart-8") or any CSS colour ("#0078d4").
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

/// charts.js CHART_COLORS.
pub mod colors {
    pub const CPU: &str = "chart-cpu";
    pub const MEMORY: &str = "chart-memory";
    pub const DISK: &str = "chart-disk";
    pub const NETWORK: &str = "chart-network";
    pub const GPU: &str = "chart-gpu";
    pub const ACCENT: &str = "accent";
    /// charts.js PALETTE.
    pub const PALETTE: [&str; 8] = ["chart-1", "chart-2", "chart-3", "chart-4", "chart-5", "chart-6", "chart-7", "chart-8"];
}

/// A token name or CSS colour as (r, g, b, a).
pub fn resolve(color: &str) -> (f64, f64, f64, f64) {
    let c = if color.starts_with('#') || color.starts_with("rgb") {
        crate::theme::rgba(color)
    } else {
        crate::theme::token(color)
    };
    (c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64)
}

/// One series of a line chart.
#[derive(Clone, Debug)]
pub struct Series {
    pub values: Vec<f64>,
    /// None: the chart's colour.
    pub color: Option<String>,
    pub dashed: bool,
    /// None: the chart's fill setting.
    pub fill: Option<bool>,
    pub line_width: Option<f64>,
}

impl Series {
    pub fn new(values: Vec<f64>) -> Series {
        Series { values, color: None, dashed: false, fill: None, line_width: None }
    }
}

/// LineChart options (charts.js defaults).
#[derive(Clone, Debug)]
pub struct LineOptions {
    pub min: f64,
    pub max: f64,
    pub fill: bool,
    pub gridlines: bool,
    pub color: String,
    /// Samples across the width (60 = one minute at 1 s).
    pub points: usize,
    pub border: bool,
    pub fill_alpha: f64,
    /// Grid rows; 0: automatic (one per 26 px, 2..10).
    pub grid_rows: u32,
    pub line_width: f64,
    /// The vertical grid lines scroll with the samples, like Task Manager.
    pub scroll_grid: bool,
}

impl Default for LineOptions {
    fn default() -> LineOptions {
        LineOptions {
            min: 0.0,
            max: 100.0,
            fill: true,
            gridlines: true,
            color: colors::CPU.into(),
            points: 60,
            border: true,
            fill_alpha: 0.12,
            grid_rows: 0,
            line_width: 1.0,
            scroll_grid: true,
        }
    }
}

struct LineInner {
    opts: RefCell<LineOptions>,
    series: RefCell<Vec<Series>>,
    tick: Cell<u64>,
}

/// The streaming line / area graph: `push` one sample (or one per series), newest at the right.
#[derive(Clone)]
pub struct LineChart {
    area: gtk::DrawingArea,
    inner: Rc<LineInner>,
}

impl LineChart {
    pub fn new(opts: LineOptions) -> LineChart {
        let area = gtk::DrawingArea::new();
        area.add_css_class("w11-chart");
        area.set_hexpand(true);
        area.set_vexpand(true);
        let inner = Rc::new(LineInner { opts: RefCell::new(opts), series: RefCell::new(Vec::new()), tick: Cell::new(0) });
        let weak = Rc::downgrade(&inner);
        area.set_draw_func(move |_, cr, w, h| {
            if let Some(inner) = weak.upgrade() {
                draw_line(&inner, cr, w as f64, h as f64);
            }
        });
        crate::theme::track(&area);
        crate::controls::own(&area, &inner);
        LineChart { area, inner }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// Replace the series.
    pub fn update(&self, series: Vec<Series>) {
        *self.inner.series.borrow_mut() = series;
        self.area.queue_draw();
    }

    /// Append one sample to each series (`values[i]` to series i; missing series are made).
    pub fn push(&self, values: &[f64]) {
        let points = self.inner.opts.borrow().points.max(2);
        {
            let mut s = self.inner.series.borrow_mut();
            while s.len() < values.len() {
                s.push(Series::new(Vec::new()));
            }
            for (i, v) in values.iter().enumerate() {
                s[i].values.push(*v);
                if s[i].values.len() > points * 2 {
                    let cut = s[i].values.len() - points;
                    s[i].values.drain(..cut);
                }
            }
        }
        self.inner.tick.set(self.inner.tick.get() + 1);
        self.area.queue_draw();
    }

    pub fn set_max(&self, max: f64) {
        self.inner.opts.borrow_mut().max = max;
        self.area.queue_draw();
    }

    pub fn set_color(&self, color: &str) {
        self.inner.opts.borrow_mut().color = color.to_owned();
        self.area.queue_draw();
    }

    pub fn set_points(&self, n: usize) {
        self.inner.opts.borrow_mut().points = n.max(2);
        self.area.queue_draw();
    }

    /// Change other options.
    pub fn set_options(&self, opts: LineOptions) {
        *self.inner.opts.borrow_mut() = opts;
        self.area.queue_draw();
    }

    /// The series now.
    pub fn series(&self) -> Vec<Series> {
        self.inner.series.borrow().clone()
    }
}

fn draw_line(inner: &LineInner, cr: &gtk::cairo::Context, w: f64, h: f64) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let o = inner.opts.borrow().clone();
    let base = resolve(&o.color);
    let range = (o.max - o.min).max(1e-9);
    let points = o.points.max(2);
    let step_x = w / (points - 1) as f64;
    if o.gridlines {
        cr.set_source_rgba(base.0, base.1, base.2, base.3 * 0.16);
        cr.set_line_width(1.0);
        let rows = if o.grid_rows == 0 { ((h / 26.0).round() as u32).clamp(2, 10) } else { o.grid_rows.max(1) };
        let cell = h / rows as f64;
        for i in 1..rows {
            let y = (i as f64 * cell).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(w, y);
        }
        let offset = if o.scroll_grid { (inner.tick.get() as f64 * step_x) % cell } else { 0.0 };
        let mut x = w - offset;
        while x > 0.0 {
            let xx = x.round() + 0.5;
            cr.move_to(xx, 0.0);
            cr.line_to(xx, h);
            x -= cell;
        }
        let _ = cr.stroke();
    }
    for s in inner.series.borrow().iter() {
        let vals: Vec<f64> = s.values.iter().rev().take(points).rev().copied().collect();
        if vals.is_empty() {
            continue;
        }
        let c = s.color.as_deref().map(resolve).unwrap_or(base);
        let x0 = w - (vals.len() - 1) as f64 * step_x;
        let y = |v: f64| {
            let v = if v.is_finite() { v } else { 0.0 };
            h - ((v.clamp(o.min, o.max) - o.min) / range) * (h - 1.0) - 0.5
        };
        let trace = |cr: &gtk::cairo::Context| {
            for (i, v) in vals.iter().enumerate() {
                let (px, py) = (x0 + i as f64 * step_x, y(*v));
                if i == 0 {
                    cr.move_to(px, py);
                } else {
                    cr.line_to(px, py);
                }
            }
        };
        if s.fill.unwrap_or(o.fill) && !s.dashed {
            trace(cr);
            cr.line_to(w, h);
            cr.line_to(x0, h);
            cr.close_path();
            cr.set_source_rgba(c.0, c.1, c.2, c.3 * o.fill_alpha);
            let _ = cr.fill();
        }
        trace(cr);
        if s.dashed {
            cr.set_dash(&[4.0, 3.0], 0.0);
        }
        cr.set_source_rgba(c.0, c.1, c.2, c.3);
        cr.set_line_width(s.line_width.unwrap_or(o.line_width));
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        let _ = cr.stroke();
        cr.set_dash(&[], 0.0);
    }
    if o.border {
        cr.set_source_rgba(base.0, base.1, base.2, base.3);
        cr.set_line_width(1.0);
        cr.rectangle(0.5, 0.5, w - 1.0, h - 1.0);
        let _ = cr.stroke();
    }
}

struct SparkInner {
    values: RefCell<Vec<f64>>,
    color: RefCell<String>,
    max: Cell<Option<f64>>,
    points: Cell<usize>,
}

/// A sparkline (charts.js Sparkline): the last 30 samples with a light fill, scaled to their maximum unless
/// `set_max`.
#[derive(Clone)]
pub struct Sparkline {
    area: gtk::DrawingArea,
    inner: Rc<SparkInner>,
}

impl Sparkline {
    pub fn new(color: &str) -> Sparkline {
        let area = gtk::DrawingArea::new();
        area.add_css_class("w11-chart");
        let inner = Rc::new(SparkInner {
            values: RefCell::new(Vec::new()),
            color: RefCell::new(color.to_owned()),
            max: Cell::new(None),
            points: Cell::new(30),
        });
        let weak = Rc::downgrade(&inner);
        area.set_draw_func(move |_, cr, w, h| {
            let Some(inner) = weak.upgrade() else { return };
            let (w, h) = (w as f64, h as f64);
            let points = inner.points.get().max(2);
            let vals: Vec<f64> = inner.values.borrow().iter().rev().take(points).rev().copied().collect();
            if vals.len() < 2 {
                return;
            }
            let c = resolve(&inner.color.borrow());
            let top = inner.max.get().unwrap_or_else(|| vals.iter().copied().fold(1.0, f64::max));
            let range = top.max(1e-9);
            let step = (w - 2.0) / (points - 1) as f64;
            let x0 = w - 1.0 - (vals.len() - 1) as f64 * step;
            let y = |v: f64| h - 1.0 - (v.clamp(0.0, top) / range) * (h - 2.0);
            let trace = |cr: &gtk::cairo::Context| {
                for (i, v) in vals.iter().enumerate() {
                    if i == 0 {
                        cr.move_to(x0, y(*v));
                    } else {
                        cr.line_to(x0 + i as f64 * step, y(*v));
                    }
                }
            };
            trace(cr);
            cr.line_to(w - 1.0, h);
            cr.line_to(x0, h);
            cr.close_path();
            cr.set_source_rgba(c.0, c.1, c.2, c.3 * 0.14);
            let _ = cr.fill();
            trace(cr);
            cr.set_source_rgba(c.0, c.1, c.2, c.3);
            cr.set_line_width(1.25);
            cr.set_line_join(gtk::cairo::LineJoin::Round);
            let _ = cr.stroke();
        });
        crate::theme::track(&area);
        crate::controls::own(&area, &inner);
        Sparkline { area, inner }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn update(&self, values: Vec<f64>) {
        *self.inner.values.borrow_mut() = values;
        self.area.queue_draw();
    }

    pub fn push(&self, v: f64) {
        let points = self.inner.points.get();
        {
            let mut vals = self.inner.values.borrow_mut();
            vals.push(v);
            if vals.len() > points * 2 {
                let cut = vals.len() - points;
                vals.drain(..cut);
            }
        }
        self.area.queue_draw();
    }

    pub fn set_max(&self, max: Option<f64>) {
        self.inner.max.set(max);
        self.area.queue_draw();
    }

    pub fn set_color(&self, color: &str) {
        *self.inner.color.borrow_mut() = color.to_owned();
        self.area.queue_draw();
    }
}

/// One segment of a donut or stacked bar.
#[derive(Clone, Debug)]
pub struct Segment {
    pub value: f64,
    pub color: String,
    pub label: String,
}

impl Segment {
    pub fn new(value: f64, color: &str, label: &str) -> Segment {
        Segment { value, color: color.to_owned(), label: label.to_owned() }
    }
}

struct DonutInner {
    segments: RefCell<Vec<Segment>>,
    total: Cell<Option<f64>>,
    thickness: f64,
    gap: f64,
}

/// A donut (charts.js Donut): segments around a track with 2 px gaps, a label (17 % of the size, Display
/// semibold) and a sublabel (text-2) inside.
#[derive(Clone)]
pub struct Donut {
    root: gtk::Overlay,
    area: gtk::DrawingArea,
    label: gtk::Label,
    sublabel: gtk::Label,
    inner: Rc<DonutInner>,
}

impl Donut {
    pub fn new(size: i32, thickness: f64) -> Donut {
        let area = gtk::DrawingArea::new();
        area.add_css_class("w11-chart");
        area.set_content_width(size);
        area.set_content_height(size);
        let root = gtk::Overlay::new();
        root.add_css_class("w11-donut");
        root.set_halign(gtk::Align::Center);
        root.set_valign(gtk::Align::Center);
        root.set_child(Some(&area));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_halign(gtk::Align::Center);
        text.set_valign(gtk::Align::Center);
        text.set_can_target(false);
        let label = gtk::Label::new(None);
        label.add_css_class("w11-donut-label");
        let attrs = gtk::pango::AttrList::new();
        attrs.insert(gtk::pango::AttrSize::new_size_absolute(((size as f64 * 0.17).round() * gtk::pango::SCALE as f64) as i32));
        attrs.insert(gtk::pango::AttrInt::new_weight(gtk::pango::Weight::Semibold));
        label.set_attributes(Some(&attrs));
        let sublabel = gtk::Label::new(None);
        sublabel.add_css_class("w11-donut-sub");
        sublabel.add_css_class("w11-text-2");
        let attrs = gtk::pango::AttrList::new();
        attrs.insert(gtk::pango::AttrSize::new_size_absolute(((size as f64 * 0.095).round().max(10.0) * gtk::pango::SCALE as f64) as i32));
        sublabel.set_attributes(Some(&attrs));
        sublabel.set_visible(false);
        text.append(&label);
        text.append(&sublabel);
        root.add_overlay(&text);
        let inner = Rc::new(DonutInner {
            segments: RefCell::new(Vec::new()),
            total: Cell::new(None),
            thickness: if thickness > 0.0 { thickness } else { 12.0 },
            gap: 2.0,
        });
        let weak = Rc::downgrade(&inner);
        area.set_draw_func(move |_, cr, w, h| {
            let Some(inner) = weak.upgrade() else { return };
            let size = w.min(h) as f64;
            let r = (size - inner.thickness) / 2.0;
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            let circ = 2.0 * PI * r;
            cr.set_line_width(inner.thickness);
            let t = resolve("chart-track");
            cr.set_source_rgba(t.0, t.1, t.2, t.3);
            cr.arc(cx, cy, r, 0.0, 2.0 * PI);
            let _ = cr.stroke();
            let segs = inner.segments.borrow();
            let visible: Vec<&Segment> = segs.iter().filter(|s| s.value > 0.0).collect();
            let sum = inner.total.get().unwrap_or_else(|| visible.iter().map(|s| s.value).sum());
            let mut offset = 0.0;
            for s in &visible {
                let len = if sum > 0.0 { s.value / sum * circ } else { 0.0 };
                let g = if visible.len() > 1 { inner.gap.min(len / 2.0) } else { 0.0 };
                let a0 = -PI / 2.0 + offset / r;
                let a1 = a0 + (len - g).max(0.0) / r;
                let c = resolve(&s.color);
                cr.set_source_rgba(c.0, c.1, c.2, c.3);
                cr.arc(cx, cy, r, a0, a1);
                let _ = cr.stroke();
                offset += len;
            }
        });
        crate::theme::track(&area);
        crate::controls::own(&area, &inner);
        Donut { root, area, label, sublabel, inner }
    }

    pub fn widget(&self) -> &gtk::Overlay {
        &self.root
    }

    /// New segments, the total (None: their sum), the label and the sublabel.
    pub fn update(&self, segments: Vec<Segment>, total: Option<f64>, label: &str, sublabel: &str) {
        *self.inner.segments.borrow_mut() = segments;
        self.inner.total.set(total);
        self.label.set_label(label);
        self.sublabel.set_label(sublabel);
        self.sublabel.set_visible(!sublabel.is_empty());
        self.area.queue_draw();
    }
}

struct StackInner {
    segments: RefCell<Vec<Segment>>,
    total: Cell<Option<f64>>,
    format: RefCell<Rc<dyn Fn(f64) -> String>>,
}

/// A stacked bar (charts.js StackedBar): segments in one pill-shaped bar of `height` px, with an optional legend
/// (dot, label, value).
#[derive(Clone)]
pub struct StackedBar {
    root: gtk::Box,
    bar: gtk::DrawingArea,
    legend: gtk::FlowBox,
    inner: Rc<StackInner>,
}

impl StackedBar {
    pub fn new(height: i32, legend: bool) -> StackedBar {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.add_css_class("w11-stacked");
        let bar = gtk::DrawingArea::new();
        bar.set_content_height(height.max(2));
        bar.set_hexpand(true);
        let lg = gtk::FlowBox::new();
        lg.set_selection_mode(gtk::SelectionMode::None);
        lg.set_column_spacing(16);
        lg.set_row_spacing(4);
        lg.set_max_children_per_line(8);
        lg.set_visible(legend);
        root.append(&bar);
        root.append(&lg);
        let inner = Rc::new(StackInner {
            segments: RefCell::new(Vec::new()),
            total: Cell::new(None),
            format: RefCell::new(Rc::new(|v| crate::format::number(v, 0))),
        });
        let weak = Rc::downgrade(&inner);
        bar.set_draw_func(move |_, cr, w, h| {
            let Some(inner) = weak.upgrade() else { return };
            let (w, h) = (w as f64, h as f64);
            let r = h / 2.0;
            cr.new_sub_path();
            cr.arc(w - r, r, r, -PI / 2.0, PI / 2.0);
            cr.arc(r, r, r, PI / 2.0, 1.5 * PI);
            cr.close_path();
            cr.clip();
            let t = resolve("chart-track");
            cr.set_source_rgba(t.0, t.1, t.2, t.3);
            cr.paint().ok();
            let segs = inner.segments.borrow();
            let sum = inner.total.get().unwrap_or_else(|| segs.iter().map(|s| s.value.max(0.0)).sum());
            let mut x = 0.0;
            let sep = resolve("bg-solid-base");
            for s in segs.iter().filter(|s| s.value > 0.0) {
                let sw = if sum > 0.0 { (s.value / sum).min(1.0) * w } else { 0.0 };
                let c = resolve(&s.color);
                cr.set_source_rgba(c.0, c.1, c.2, c.3);
                cr.rectangle(x, 0.0, sw, h);
                let _ = cr.fill();
                if x > 0.0 {
                    cr.set_source_rgba(sep.0, sep.1, sep.2, sep.3);
                    cr.rectangle(x - 1.0, 0.0, 1.0, h);
                    let _ = cr.fill();
                }
                x += sw;
            }
        });
        crate::theme::track(&bar);
        crate::controls::own(&bar, &inner);
        StackedBar { root, bar, legend: lg, inner }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// How legend values read.
    pub fn set_format(&self, f: impl Fn(f64) -> String + 'static) {
        *self.inner.format.borrow_mut() = Rc::new(f);
    }

    pub fn update(&self, segments: Vec<Segment>, total: Option<f64>) {
        self.inner.total.set(total);
        while let Some(c) = self.legend.first_child() {
            self.legend.remove(&c);
        }
        let fmt = self.inner.format.borrow().clone();
        for s in &segments {
            let key = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            key.add_css_class("w11-caption");
            let dot = gtk::DrawingArea::new();
            dot.set_content_width(8);
            dot.set_content_height(8);
            dot.set_valign(gtk::Align::Center);
            let color = s.color.clone();
            dot.set_draw_func(move |_, cr, w, h| {
                let c = resolve(&color);
                cr.set_source_rgba(c.0, c.1, c.2, c.3);
                cr.arc(w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0, 0.0, 2.0 * PI);
                let _ = cr.fill();
            });
            crate::theme::track(&dot);
            key.append(&dot);
            let l = gtk::Label::new(Some(&s.label));
            l.add_css_class("w11-text-2");
            key.append(&l);
            let v = gtk::Label::new(Some(&fmt(s.value)));
            v.add_css_class("w11-text-1");
            key.append(&v);
            self.legend.insert(&key, -1);
        }
        *self.inner.segments.borrow_mut() = segments;
        self.bar.queue_draw();
    }
}
