//! Fluent-style line glyphs drawn with cairo in the widget's CSS `color` (so hover, pressed and disabled
//! colours apply like a symbolic icon), on the 20 x 20 grid of last-webkit:vendor/ui/js/core/icons.js with its 1.35
//! stroke, and the title bar's caption glyphs on appframe.js's 10 x 10 grid with a 1 px stroke.
use gtk::cairo;
use gtk::gdk;
use gtk::prelude::*;
use std::f64::consts::PI;

const STROKE: f64 = 1.35;

/// A glyph by name, `px` logical pixels square.
pub fn glyph(name: &'static str, px: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(px);
    area.set_content_height(px);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_can_target(false);
    area.add_css_class("cal-glyph");
    set(&area, name);
    // The colour comes from CSS states (hover, pressed, backdrop): redraw when they change.
    area.connect_state_flags_changed(|a, _| a.queue_draw());
    area
}

/// Show another glyph in an area made by `glyph`.
pub fn set(area: &gtk::DrawingArea, name: &'static str) {
    area.set_draw_func(move |a, cr, w, h| {
        let c = a.color();
        cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        draw(cr, name, w as f64, h as f64);
    });
    area.queue_draw();
}

/// A filled colour dot (a calendar's colour), `px` square.
pub fn dot(color: gdk::RGBA, px: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(px);
    area.set_content_height(px);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_can_target(false);
    area.add_css_class("cal-dot");
    area.set_draw_func(move |_, cr, w, h| {
        let r = (w.min(h) as f64) / 2.0;
        cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64);
        cr.arc(w as f64 / 2.0, h as f64 / 2.0, r, 0.0, 2.0 * PI);
        let _ = cr.fill();
    });
    area
}

fn draw(cr: &cairo::Context, name: &str, w: f64, h: f64) {
    if let Some(cap) = name.strip_prefix("cap-") {
        // appframe.js GLYPH: 10 x 10, stroke 1.
        let s = w.min(h) / 10.0;
        cr.save().ok();
        cr.translate((w - 10.0 * s) / 2.0, (h - 10.0 * s) / 2.0);
        cr.scale(s, s);
        cr.set_line_width(1.0);
        cr.set_line_cap(cairo::LineCap::Butt);
        match cap {
            "min" => {
                cr.move_to(0.0, 5.5);
                cr.line_to(10.0, 5.5);
            }
            "max" => round_rect(cr, 0.5, 0.5, 9.0, 9.0, 1.5),
            "restore" => {
                round_rect(cr, 0.5, 2.5, 7.0, 7.0, 1.5);
                cr.move_to(2.6, 0.5);
                cr.line_to(8.0, 0.5);
                cr.curve_to(8.83, 0.5, 9.5, 1.17, 9.5, 2.0);
                cr.line_to(9.5, 7.4);
            }
            _ => {
                cr.move_to(0.5, 0.5);
                cr.line_to(9.5, 9.5);
                cr.move_to(9.5, 0.5);
                cr.line_to(0.5, 9.5);
            }
        }
        let _ = cr.stroke();
        cr.restore().ok();
        return;
    }
    let s = w.min(h) / 20.0;
    cr.save().ok();
    cr.translate((w - 20.0 * s) / 2.0, (h - 20.0 * s) / 2.0);
    cr.scale(s, s);
    cr.set_line_width(STROKE);
    let mut fill = false;
    match name {
        "add" => {
            cr.move_to(10.0, 4.0);
            cr.line_to(10.0, 16.0);
            cr.move_to(4.0, 10.0);
            cr.line_to(16.0, 10.0);
        }
        "chevron-left" => poly(cr, &[(12.5, 4.5), (7.0, 10.0), (12.5, 15.5)]),
        "chevron-right" => poly(cr, &[(7.5, 4.5), (13.0, 10.0), (7.5, 15.5)]),
        "chevron-up" => poly(cr, &[(4.5, 12.5), (10.0, 7.0), (15.5, 12.5)]),
        "chevron-down" => poly(cr, &[(4.5, 7.5), (10.0, 13.0), (15.5, 7.5)]),
        "arrow-up" => {
            cr.move_to(10.0, 16.5);
            cr.line_to(10.0, 3.75);
            poly(cr, &[(4.75, 9.0), (10.0, 3.75), (15.25, 9.0)]);
        }
        "arrow-down" => {
            cr.move_to(10.0, 3.5);
            cr.line_to(10.0, 16.25);
            poly(cr, &[(4.75, 11.0), (10.0, 16.25), (15.25, 11.0)]);
        }
        "arrow-left" => {
            cr.move_to(16.5, 10.0);
            cr.line_to(3.75, 10.0);
            poly(cr, &[(9.0, 4.75), (3.75, 10.0), (9.0, 15.25)]);
        }
        "arrow-right" => {
            cr.move_to(3.5, 10.0);
            cr.line_to(16.25, 10.0);
            poly(cr, &[(11.0, 4.75), (16.25, 10.0), (11.0, 15.25)]);
        }
        "calendar" => {
            round_rect(cr, 3.0, 4.0, 14.0, 13.0, 2.0);
            cr.move_to(3.0, 8.0);
            cr.line_to(17.0, 8.0);
            cr.move_to(7.0, 2.5);
            cr.line_to(7.0, 5.0);
            cr.move_to(13.0, 2.5);
            cr.line_to(13.0, 5.0);
            let _ = cr.stroke();
            for (x, y) in [(7.0, 11.0), (10.0, 11.0), (13.0, 11.0), (7.0, 14.0), (10.0, 14.0)] {
                cr.arc(x, y, 0.9, 0.0, 2.0 * PI);
                let _ = cr.fill();
            }
        }
        "calendar-today" => {
            round_rect(cr, 3.0, 4.0, 14.0, 13.0, 2.0);
            cr.move_to(3.0, 8.0);
            cr.line_to(17.0, 8.0);
            cr.move_to(7.0, 2.5);
            cr.line_to(7.0, 5.0);
            cr.move_to(13.0, 2.5);
            cr.line_to(13.0, 5.0);
            let _ = cr.stroke();
            round_rect(cr, 8.0, 10.5, 4.0, 4.0, 0.8);
            fill = true;
        }
        "menu" => {
            for y in [5.5, 10.0, 14.5] {
                cr.move_to(3.0, y);
                cr.line_to(17.0, y);
            }
        }
        "delete" => {
            cr.move_to(3.5, 5.5);
            cr.line_to(16.5, 5.5);
            cr.move_to(8.0, 5.5);
            cr.line_to(8.0, 4.25);
            cr.curve_to(8.0, 3.56, 8.56, 3.0, 9.25, 3.0);
            cr.line_to(10.75, 3.0);
            cr.curve_to(11.44, 3.0, 12.0, 3.56, 12.0, 4.25);
            cr.line_to(12.0, 5.5);
            cr.move_to(5.0, 5.5);
            cr.line_to(5.85, 15.65);
            cr.curve_to(5.92, 16.55, 6.67, 17.25, 7.58, 17.25);
            cr.line_to(12.42, 17.25);
            cr.curve_to(13.33, 17.25, 14.08, 16.55, 14.15, 15.65);
            cr.line_to(15.0, 5.5);
            cr.move_to(8.5, 8.5);
            cr.line_to(8.75, 14.0);
            cr.move_to(11.5, 8.5);
            cr.line_to(11.25, 14.0);
        }
        "location" => {
            // A map pin: a circle in a drop.
            cr.new_sub_path();
            cr.arc(10.0, 8.5, 2.0, 0.0, 2.0 * PI);
            cr.new_sub_path();
            cr.arc(10.0, 8.5, 5.5, PI * 0.82, PI * 2.18);
            cr.line_to(10.0, 17.5);
            cr.close_path();
        }
        "clock" => {
            cr.new_sub_path();
            cr.arc(10.0, 10.0, 7.0, 0.0, 2.0 * PI);
            cr.move_to(10.0, 6.0);
            cr.line_to(10.0, 10.0);
            cr.line_to(12.75, 11.75);
        }
        "notes" => {
            for (y, x2) in [(5.5, 16.0), (9.0, 16.0), (12.5, 16.0), (16.0, 11.0)] {
                cr.move_to(4.0, y);
                cr.line_to(x2, y);
            }
        }
        "dismiss" => {
            cr.move_to(5.0, 5.0);
            cr.line_to(15.0, 15.0);
            cr.move_to(15.0, 5.0);
            cr.line_to(5.0, 15.0);
        }
        "day" => {
            round_rect(cr, 3.0, 4.0, 14.0, 13.0, 2.0);
            cr.move_to(3.0, 8.0);
            cr.line_to(17.0, 8.0);
            cr.move_to(8.5, 8.0);
            cr.line_to(8.5, 17.0);
            cr.move_to(11.5, 8.0);
            cr.line_to(11.5, 17.0);
        }
        "week" => {
            round_rect(cr, 3.0, 4.0, 14.0, 13.0, 2.0);
            cr.move_to(3.0, 8.0);
            cr.line_to(17.0, 8.0);
            for x in [6.5, 10.0, 13.5] {
                cr.move_to(x, 8.0);
                cr.line_to(x, 17.0);
            }
        }
        "month" => {
            round_rect(cr, 3.0, 4.0, 14.0, 13.0, 2.0);
            cr.move_to(3.0, 8.0);
            cr.line_to(17.0, 8.0);
            cr.move_to(3.0, 12.5);
            cr.line_to(17.0, 12.5);
            for x in [7.67, 12.33] {
                cr.move_to(x, 8.0);
                cr.line_to(x, 17.0);
            }
        }
        _ => {
            cr.new_sub_path();
            cr.arc(10.0, 10.0, 6.0, 0.0, 2.0 * PI);
        }
    }
    if fill {
        let _ = cr.fill();
    } else {
        let _ = cr.stroke();
    }
    cr.restore().ok();
}

fn poly(cr: &cairo::Context, pts: &[(f64, f64)]) {
    if let Some((x, y)) = pts.first() {
        cr.move_to(*x, *y);
    }
    for (x, y) in pts.iter().skip(1) {
        cr.line_to(*x, *y);
    }
}

fn round_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}
