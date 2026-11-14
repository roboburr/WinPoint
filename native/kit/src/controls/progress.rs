//! Progress bar and progress ring (components.js ProgressBar, ProgressRing), drawn with cairo: the bar is a
//! 1 px track with a 3 px accent bar (paused: caution, error: critical; the CSS `color` of .w11-progress), the
//! indeterminate bar runs the two sliding bars of components.css; the ring is a track and an accent arc, or a
//! spinning arc when indeterminate.
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::f64::consts::PI;
use std::rc::Rc;

/// The bar's state colours (`.fl-progress.is-paused`, `.is-error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    Normal,
    Paused,
    Error,
}

struct BarInner {
    fraction: Cell<f64>,
    indeterminate: Cell<bool>,
    start: Cell<i64>,
    tick: std::cell::RefCell<Option<gtk::TickCallbackId>>,
}

/// A progress bar, 3 px high.
#[derive(Clone)]
pub struct ProgressBar {
    area: gtk::DrawingArea,
    inner: Rc<BarInner>,
}

fn ease(t: f64) -> f64 {
    // cubic-bezier(0.4, 0, 0.6, 1), close enough: smoothstep
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl ProgressBar {
    pub fn new() -> ProgressBar {
        let area = gtk::DrawingArea::new();
        area.add_css_class("w11-progress");
        area.set_content_height(3);
        area.set_hexpand(true);
        area.set_valign(gtk::Align::Center);
        let inner = Rc::new(BarInner {
            fraction: Cell::new(0.0),
            indeterminate: Cell::new(false),
            start: Cell::new(0),
            tick: std::cell::RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        area.set_draw_func(move |a, cr, w, h| {
            let Some(inner) = weak.upgrade() else { return };
            let (w, h) = (w as f64, h as f64);
            let y0 = ((h - 3.0) / 2.0).max(0.0);
            let c = a.color();
            let bar = |cr: &gtk::cairo::Context, x: f64, bw: f64| {
                let x1 = x.max(0.0);
                let x2 = (x + bw).min(w);
                if x2 <= x1 {
                    return;
                }
                rounded(cr, x1, y0, x2 - x1, 3.0, 1.5);
                let _ = cr.fill();
            };
            if inner.indeterminate.get() {
                let now = a.frame_clock().map(|f| f.frame_time()).unwrap_or(0);
                let t = (((now - inner.start.get()) as f64 / 1_000_000.0) % 2.0) / 2.0;
                cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
                // fl-prog-a: 40 % wide, -40 % -> 100 % over the first 55 %
                if t <= 0.55 {
                    let p = ease(t / 0.55);
                    bar(cr, (-0.4 + 1.4 * p) * w, 0.4 * w);
                }
                // fl-prog-b: 60 % wide, -60 % -> 100 % from 45 % on
                if t >= 0.45 {
                    let p = ease((t - 0.45) / 0.55);
                    bar(cr, (-0.6 + 1.6 * p) * w, 0.6 * w);
                }
                return;
            }
            let track = crate::theme::token("stroke-control-strong");
            cr.set_source_rgba(track.red() as f64, track.green() as f64, track.blue() as f64, track.alpha() as f64);
            cr.rectangle(0.0, y0 + 1.0, w, 1.0);
            let _ = cr.fill();
            cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
            bar(cr, 0.0, w * inner.fraction.get().clamp(0.0, 1.0));
        });
        crate::theme::track(&area);
        crate::controls::own(&area, &inner);
        ProgressBar { area, inner }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// 0.0 ..= 1.0 (ends the indeterminate state).
    pub fn set_fraction(&self, f: f64) {
        self.inner.fraction.set(if f.is_finite() { f.clamp(0.0, 1.0) } else { 0.0 });
        self.set_indeterminate(false);
        self.area.queue_draw();
    }

    pub fn fraction(&self) -> f64 {
        self.inner.fraction.get()
    }

    /// The two sliding bars.
    pub fn set_indeterminate(&self, on: bool) {
        if self.inner.indeterminate.replace(on) == on {
            return;
        }
        if on {
            let now = self.area.frame_clock().map(|f| f.frame_time()).unwrap_or(0);
            self.inner.start.set(now);
            let id = self.area.add_tick_callback(|a, _| {
                a.queue_draw();
                glib::ControlFlow::Continue
            });
            *self.inner.tick.borrow_mut() = Some(id);
        } else if let Some(id) = self.inner.tick.borrow_mut().take() {
            id.remove();
        }
        self.area.queue_draw();
    }

    pub fn set_state(&self, s: ProgressState) {
        let a = &self.area;
        a.remove_css_class("w11-paused");
        a.remove_css_class("w11-error");
        match s {
            ProgressState::Paused => a.add_css_class("w11-paused"),
            ProgressState::Error => a.add_css_class("w11-error"),
            ProgressState::Normal => {}
        }
        a.queue_draw();
    }
}

impl Default for ProgressBar {
    fn default() -> ProgressBar {
        ProgressBar::new()
    }
}

struct RingInner {
    fraction: Cell<Option<f64>>,
    start: Cell<i64>,
    tick: std::cell::RefCell<Option<gtk::TickCallbackId>>,
}

/// A progress ring of `size` px (32 by default in the HTML); `None` fraction spins.
#[derive(Clone)]
pub struct ProgressRing {
    area: gtk::DrawingArea,
    inner: Rc<RingInner>,
}

impl ProgressRing {
    pub fn new(size: i32, fraction: Option<f64>) -> ProgressRing {
        let area = gtk::DrawingArea::new();
        area.add_css_class("w11-ring");
        area.set_content_width(size);
        area.set_content_height(size);
        area.set_halign(gtk::Align::Center);
        area.set_valign(gtk::Align::Center);
        let inner = Rc::new(RingInner {
            fraction: Cell::new(None),
            start: Cell::new(0),
            tick: std::cell::RefCell::new(None),
        });
        let weak = Rc::downgrade(&inner);
        area.set_draw_func(move |a, cr, w, h| {
            let Some(inner) = weak.upgrade() else { return };
            let s = w.min(h) as f64;
            let sw = (s / 10.5).round().max(2.0);
            let r = (s - sw) / 2.0;
            let (cx, cy) = (w as f64 / 2.0, h as f64 / 2.0);
            cr.set_line_width(sw);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            let c = a.color();
            match inner.fraction.get() {
                Some(f) => {
                    let track = crate::theme::token("fill-control-alt-3");
                    cr.set_source_rgba(track.red() as f64, track.green() as f64, track.blue() as f64, track.alpha() as f64);
                    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
                    let _ = cr.stroke();
                    if f > 0.0 {
                        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
                        cr.arc(cx, cy, r, -PI / 2.0, -PI / 2.0 + 2.0 * PI * f.min(1.0));
                        let _ = cr.stroke();
                    }
                }
                None => {
                    let now = a.frame_clock().map(|f| f.frame_time()).unwrap_or(0);
                    let secs = (now - inner.start.get()) as f64 / 1_000_000.0;
                    let spin = (secs % 2.0) / 2.0 * 2.0 * PI;
                    // fl-ring-dash: 1 % -> 55 % -> 1 % of the circle over 1.6 s, the offset running ahead
                    let t = (secs % 1.6) / 1.6;
                    let (len, off) = if t < 0.5 {
                        let p = ease(t / 0.5);
                        (0.01 + 0.54 * p, 0.18 * p)
                    } else {
                        let p = ease((t - 0.5) / 0.5);
                        (0.55 - 0.54 * p, 0.18 + 0.81 * p)
                    };
                    let a0 = -PI / 2.0 + spin + off * 2.0 * PI;
                    cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
                    cr.arc(cx, cy, r, a0, a0 + len * 2.0 * PI);
                    let _ = cr.stroke();
                }
            }
        });
        crate::theme::track(&area);
        crate::controls::own(&area, &inner);
        let me = ProgressRing { area, inner };
        me.set_fraction(fraction);
        me
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// Some(0.0 ..= 1.0), or None to spin.
    pub fn set_fraction(&self, f: Option<f64>) {
        let f = f.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
        self.inner.fraction.set(f);
        match f {
            None => {
                if self.inner.tick.borrow().is_none() {
                    let now = self.area.frame_clock().map(|f| f.frame_time()).unwrap_or(0);
                    self.inner.start.set(now);
                    let id = self.area.add_tick_callback(|a, _| {
                        a.queue_draw();
                        glib::ControlFlow::Continue
                    });
                    *self.inner.tick.borrow_mut() = Some(id);
                }
            }
            Some(_) => {
                if let Some(id) = self.inner.tick.borrow_mut().take() {
                    id.remove();
                }
            }
        }
        self.area.queue_draw();
    }
}

fn rounded(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}
