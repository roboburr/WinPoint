//! The drive artwork and the app icon of util.js (original drawings, 48-unit and 16-unit grids), drawn with
//! cairo the way the SVGs read: linear gradients in the shape's bounding box, rounded rectangles, strokes.
use gtk::cairo::{self, Context, LinearGradient, RadialGradient};
use gtk::prelude::*;
use std::f64::consts::PI;

fn hex(c: &str) -> (f64, f64, f64) {
    let (r, g, b) = kit::theme::rgb(c);
    (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0)
}

fn rgb(cr: &Context, c: &str) {
    let (r, g, b) = hex(c);
    cr.set_source_rgb(r, g, b);
}

fn rgba(cr: &Context, c: &str, a: f64) {
    let (r, g, b) = hex(c);
    cr.set_source_rgba(r, g, b, a);
}

/// An SVG rect with rx (clamped like SVG).
fn rrect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        cr.rectangle(x, y, w, h);
        return;
    }
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn fill_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64, c: &str) {
    rrect(cr, x, y, w, h, r);
    rgb(cr, c);
    let _ = cr.fill();
}

fn circle(cr: &Context, cx: f64, cy: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(cx, cy, r, 0.0, 2.0 * PI);
}

fn fill_circle(cr: &Context, cx: f64, cy: f64, r: f64, c: &str) {
    circle(cr, cx, cy, r);
    rgb(cr, c);
    let _ = cr.fill();
}

/// A two-stop linear gradient over the box (x, y, w, h): top to bottom, or top-left to bottom-right.
fn grad(x: f64, y: f64, w: f64, h: f64, a: &str, b: &str, vertical: bool) -> LinearGradient {
    let g = if vertical { LinearGradient::new(x, y, x, y + h) } else { LinearGradient::new(x, y, x + w, y + h) };
    let (r1, g1, b1) = hex(a);
    let (r2, g2, b2) = hex(b);
    g.add_color_stop_rgb(0.0, r1, g1, b1);
    g.add_color_stop_rgb(1.0, r2, g2, b2);
    g
}

fn grad_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64, a: &str, b: &str, vertical: bool) {
    rrect(cr, x, y, w, h, r);
    let _ = cr.set_source(&grad(x, y, w, h, a, b, vertical));
    let _ = cr.fill();
}

fn line(cr: &Context, pts: &[(f64, f64, f64, f64)], c: &str, width: f64, cap: cairo::LineCap) {
    for (x1, y1, x2, y2) in pts {
        cr.move_to(*x1, *y1);
        cr.line_to(*x2, *y2);
    }
    rgb(cr, c);
    cr.set_line_width(width);
    cr.set_line_cap(cap);
    let _ = cr.stroke();
}

/// Draw the drive art of a kind on the 48-unit grid (scaled by the caller).
pub fn draw_drive(cr: &Context, kind: &str, system: bool) {
    let butt = cairo::LineCap::Butt;
    match kind {
        "nvme" => {
            grad_rect(cr, 3.0, 15.0, 42.0, 18.0, 2.5, "#2f8f5b", "#1c6b42", false);
            fill_rect(cr, 40.0, 18.0, 5.0, 12.0, 0.0, "#e4c56a");
            line(cr, &[(41.0, 20.0, 41.0, 28.0), (43.0, 20.0, 43.0, 28.0)], "#b8942f", 0.8, butt);
            fill_rect(cr, 7.0, 18.5, 12.0, 11.0, 1.2, "#1d2228");
            fill_rect(cr, 21.0, 18.5, 12.0, 11.0, 1.2, "#1d2228");
            fill_rect(cr, 8.5, 20.0, 9.0, 3.0, 0.6, "#3a4450");
            fill_rect(cr, 22.5, 20.0, 9.0, 3.0, 0.6, "#3a4450");
            fill_rect(cr, 34.5, 20.0, 4.0, 8.0, 0.8, "#262c33");
            fill_circle(cr, 5.2, 24.0, 1.3, "#0f3d26");
        }
        "ssd" => {
            grad_rect(cr, 6.0, 9.0, 36.0, 30.0, 3.0, "#4b5563", "#2b3139", true);
            fill_rect(cr, 10.0, 13.0, 28.0, 13.0, 1.5, "#e8edf2");
            fill_rect(cr, 13.0, 16.0, 14.0, 2.4, 1.0, "#2563eb");
            fill_rect(cr, 13.0, 20.5, 20.0, 1.6, 0.8, "#9aa6b2");
            fill_rect(cr, 10.0, 33.0, 9.0, 3.0, 1.0, "#1c2127");
            fill_rect(cr, 21.0, 33.0, 5.0, 3.0, 1.0, "#1c2127");
        }
        "usb" => {
            grad_rect(cr, 4.0, 16.0, 30.0, 16.0, 3.5, "#3f4652", "#20252c", false);
            fill_rect(cr, 33.0, 18.5, 11.0, 11.0, 1.2, "#cfd6de");
            fill_rect(cr, 36.0, 21.5, 3.0, 2.0, 0.0, "#7b8590");
            fill_rect(cr, 36.0, 24.5, 3.0, 2.0, 0.0, "#7b8590");
            fill_rect(cr, 8.0, 20.0, 16.0, 8.0, 1.5, "#d1272e");
            fill_circle(cr, 28.0, 24.0, 1.6, "#4ce07a");
        }
        "sd" => {
            // M12 6h18l8 8v26a2.5 2.5 0 0 1-2.5 2.5h-23A2.5 2.5 0 0 1 10 40V8a2 2 0 0 1 2-2Z
            cr.move_to(12.0, 6.0);
            cr.line_to(30.0, 6.0);
            cr.line_to(38.0, 14.0);
            cr.line_to(38.0, 40.0);
            cr.arc(35.5, 40.0, 2.5, 0.0, PI / 2.0);
            cr.line_to(12.5, 42.5);
            cr.arc(12.5, 40.0, 2.5, PI / 2.0, PI);
            cr.line_to(10.0, 8.0);
            cr.arc(12.0, 8.0, 2.0, PI, 1.5 * PI);
            cr.close_path();
            let _ = cr.set_source(&grad(10.0, 6.0, 28.0, 36.5, "#2f6bd8", "#1b47a0", true));
            let _ = cr.fill();
            line(cr, &[(15.0, 6.0, 15.0, 13.0), (19.0, 6.0, 19.0, 13.0), (23.0, 6.0, 23.0, 13.0), (27.0, 6.0, 27.0, 13.0)], "#e4c56a", 2.0, butt);
            rrect(cr, 14.0, 22.0, 20.0, 12.0, 1.5);
            rgba(cr, "#ffffff", 0.9);
            let _ = cr.fill();
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(8.0);
            if let Ok(ext) = cr.text_extents("SD") {
                cr.move_to(24.0 - ext.x_advance() / 2.0, 31.0);
                rgb(cr, "#1b47a0");
                let _ = cr.show_text("SD");
            }
        }
        "optical" => {
            circle(cr, 24.0, 24.0, 19.0);
            let g = RadialGradient::new(5.0 + 0.35 * 38.0, 5.0 + 0.3 * 38.0, 0.0, 5.0 + 0.35 * 38.0, 5.0 + 0.3 * 38.0, 0.9 * 38.0);
            for (o, c) in [(0.0, "#f5f7fa"), (0.5, "#c7d2de"), (0.75, "#e9d8f5"), (1.0, "#b9c7d6")] {
                let (r, gg, b) = hex(c);
                g.add_color_stop_rgb(o, r, gg, b);
            }
            let _ = cr.set_source(&g);
            let _ = cr.fill_preserve();
            rgb(cr, "#9aa9b8");
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            circle(cr, 24.0, 24.0, 6.0);
            rgb(cr, "#ffffff");
            let _ = cr.fill_preserve();
            rgb(cr, "#9aa9b8");
            let _ = cr.stroke();
            fill_circle(cr, 24.0, 24.0, 2.4, "#9aa9b8");
        }
        "loop" => {
            rrect(cr, 5.0, 13.0, 38.0, 22.0, 4.0);
            rgb(cr, "#7d8a99");
            cr.set_line_width(2.0);
            cr.set_dash(&[4.0, 3.0], 0.0);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
            rrect(cr, 11.0, 19.0, 16.0, 10.0, 1.5);
            rgba(cr, "#7d8a99", 0.5);
            let _ = cr.fill();
            fill_circle(cr, 36.0, 29.0, 1.8, "#7d8a99");
        }
        _ => {
            // hdd
            grad_rect(cr, 5.0, 7.0, 38.0, 34.0, 3.5, "#cfd5dc", "#9ea8b3", true);
            circle(cr, 21.0, 22.0, 11.0);
            rgb(cr, "#e9edf1");
            let _ = cr.fill_preserve();
            rgb(cr, "#8c97a3");
            cr.set_line_width(1.0);
            let _ = cr.stroke();
            fill_circle(cr, 21.0, 22.0, 2.5, "#8c97a3");
            line(cr, &[(36.0, 12.0, 27.0, 25.0)], "#5f6b78", 2.2, cairo::LineCap::Round);
            fill_circle(cr, 36.0, 12.0, 2.4, "#5f6b78");
            fill_rect(cr, 9.0, 35.0, 30.0, 3.0, 1.2, "#7f8a96");
            fill_circle(cr, 38.0, 36.5, 1.0, "#4ce07a");
        }
    }
    if system {
        rrect(cr, 30.0, 30.0, 16.0, 16.0, 3.0);
        rgb(cr, "#0067c0");
        let _ = cr.fill_preserve();
        rgb(cr, "#ffffff");
        cr.set_line_width(1.5);
        let _ = cr.stroke();
        for (x, y) in [(34.0, 34.0), (38.6, 34.0), (34.0, 38.6), (38.6, 38.6)] {
            fill_rect(cr, x, y, 3.4, 3.4, 0.0, "#ffffff");
        }
    }
}

/// The app icon on its 16-unit grid: a drive with its partition bar.
pub fn draw_app(cr: &Context) {
    grad_rect(cr, 0.5, 2.5, 15.0, 11.0, 2.5, "#4b9fea", "#1d5fbf", false);
    fill_rect(cr, 2.5, 4.5, 4.0, 3.2, 0.7, "#ffffff");
    fill_rect(cr, 7.3, 4.5, 4.0, 3.2, 0.7, "#9be7f2");
    fill_rect(cr, 12.1, 4.5, 1.6, 3.2, 0.5, "#c9d6e6");
    cr.move_to(2.6, 10.6);
    cr.line_to(9.0, 10.6);
    rgba(cr, "#ffffff", 0.75);
    cr.set_line_width(1.1);
    cr.set_line_cap(cairo::LineCap::Round);
    let _ = cr.stroke();
    fill_circle(cr, 12.6, 10.6, 0.95, "#5ef0a0");
}

/// The drive art of a kind as a `size` px widget (class dk-art).
pub fn drive_art(kind: &str, size: i32, system: bool) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.add_css_class("dk-art");
    a.set_content_width(size);
    a.set_content_height(size);
    a.set_halign(gtk::Align::Center);
    a.set_valign(gtk::Align::Center);
    a.set_can_target(false);
    let kind = kind.to_owned();
    a.set_draw_func(move |_, cr, w, h| {
        let s = w.min(h) as f64 / 48.0;
        cr.scale(s, s);
        draw_drive(cr, &kind, system);
    });
    a
}

/// The app icon (title bar, 16 px).
pub fn app_glyph(size: i32) -> gtk::DrawingArea {
    let a = gtk::DrawingArea::new();
    a.set_content_width(size);
    a.set_content_height(size);
    a.set_valign(gtk::Align::Center);
    a.set_can_target(false);
    a.set_draw_func(|_, cr, w, h| {
        let s = w.min(h) as f64 / 16.0;
        cr.scale(s, s);
        draw_app(cr);
    });
    a
}
