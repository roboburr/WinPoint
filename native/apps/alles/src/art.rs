//! Alles's own pictures (main.js): the app logo (a warm tile with a magnifier), the welcome illustration, the
//! "Archief (zip)" glyph and the text glyphs of the toggle buttons (Aa, ab, .*).
use gtk::cairo;
use gtk::prelude::*;
use std::f64::consts::PI;

fn rounded(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 1.5 * PI);
    cr.close_path();
}

fn stop(g: &cairo::LinearGradient, at: f64, color: u32) {
    let r = ((color >> 16) & 0xff) as f64 / 255.0;
    let gr = ((color >> 8) & 0xff) as f64 / 255.0;
    let b = (color & 0xff) as f64 / 255.0;
    g.add_color_stop_rgb(at, r, gr, b);
}

/// The 16 px logo of the title bar (allesLogo), `size` px.
pub fn logo(size: i32) -> gtk::DrawingArea {
    let d = gtk::DrawingArea::new();
    d.set_content_width(size);
    d.set_content_height(size);
    d.set_valign(gtk::Align::Center);
    d.add_css_class("alles-logo");
    d.set_draw_func(|_, cr, w, h| {
        let s = w.min(h) as f64 / 16.0;
        cr.scale(s, s);
        let g = cairo::LinearGradient::new(1.0, 1.0, 15.0, 15.0);
        stop(&g, 0.0, 0xffb347);
        stop(&g, 1.0, 0xec5b13);
        rounded(cr, 1.0, 1.0, 14.0, 14.0, 3.5);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(1.6);
        cr.new_sub_path();
        cr.arc(7.1, 7.1, 3.1, 0.0, 2.0 * PI);
        let _ = cr.stroke();
        cr.set_line_width(1.8);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.move_to(9.4, 9.4);
        cr.line_to(12.3, 12.3);
        let _ = cr.stroke();
    });
    d
}

/// The welcome / first-run illustration (welcomeArt, 120 x 96): a folder, a document and the magnifier, in
/// the theme's colours.
pub fn welcome() -> gtk::DrawingArea {
    let d = gtk::DrawingArea::new();
    d.set_content_width(120);
    d.set_content_height(96);
    d.set_halign(gtk::Align::Center);
    d.add_css_class("alles-art");
    kit::theme::track(&d);
    d.set_draw_func(|_, cr, _, _| {
        let text = kit::theme::token("text-1");
        let with = |a: f32| {
            let mut c = text;
            c.set_alpha(a);
            c
        };
        let set = |c: &gtk::gdk::RGBA| cr.set_source_rgba(c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
        // .aw-card
        set(&with(0.09));
        rounded(cr, 14.0, 26.0, 58.0, 46.0, 6.0);
        let _ = cr.fill();
        // the folder (aw1)
        let g = cairo::LinearGradient::new(0.0, 28.0, 0.0, 72.0);
        stop(&g, 0.0, 0xffd27a);
        stop(&g, 1.0, 0xf4a62a);
        cr.new_sub_path();
        cr.arc(20.0, 34.0, 6.0, PI, 1.5 * PI);
        cr.line_to(34.0, 28.0);
        cr.line_to(39.0, 33.0);
        cr.line_to(66.0, 33.0);
        cr.arc(66.0, 39.0, 6.0, -PI / 2.0, 0.0);
        cr.line_to(72.0, 66.0);
        cr.arc(66.0, 66.0, 6.0, 0.0, PI / 2.0);
        cr.line_to(20.0, 72.0);
        cr.arc(20.0, 66.0, 6.0, PI / 2.0, PI);
        cr.close_path();
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        // .aw-doc with its lines
        rounded(cr, 44.0, 14.0, 34.0, 44.0, 4.0);
        set(&kit::theme::token("bg-solid-4"));
        let _ = cr.fill_preserve();
        set(&with(0.18));
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        set(&with(0.30));
        cr.set_line_width(2.4);
        cr.set_line_cap(cairo::LineCap::Round);
        for (y, x2) in [(26.0, 71.0), (33.0, 71.0), (40.0, 64.0)] {
            cr.move_to(51.0, y);
            cr.line_to(x2, y);
        }
        let _ = cr.stroke();
        // the magnifier (aw2)
        let g = cairo::LinearGradient::new(63.0, 39.0, 97.0, 73.0);
        stop(&g, 0.0, 0xffb347);
        stop(&g, 1.0, 0xec5b13);
        cr.new_sub_path();
        cr.arc(80.0, 56.0, 17.0, 0.0, 2.0 * PI);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.set_line_width(3.6);
        cr.new_sub_path();
        cr.arc(80.0, 56.0, 9.5, 0.0, 2.0 * PI);
        let _ = cr.stroke();
        cr.set_line_width(4.4);
        cr.move_to(87.0, 63.0);
        cr.line_to(96.0, 72.0);
        let _ = cr.stroke();
        // .aw-spark
        let mut accent = kit::theme::token("accent");
        accent.set_alpha(0.8);
        set(&accent);
        cr.set_line_width(2.0);
        for (x1, y1, x2, y2) in [(101.0, 22.0, 101.0, 30.0), (97.0, 26.0, 105.0, 26.0), (28.0, 12.0, 28.0, 18.0), (25.0, 15.0, 31.0, 15.0)] {
            cr.move_to(x1, y1);
            cr.line_to(x2, y2);
        }
        let _ = cr.stroke();
    });
    d
}

/// A rounded rectangle outline as a filled even-odd path (symbolic icons are recoloured by their fill).
fn outline(x: f64, y: f64, w: f64, h: f64, r: f64, stroke: f64) -> String {
    let rr = |x: f64, y: f64, w: f64, h: f64, r: f64| {
        let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
        format!(
            "M{:.3} {:.3}h{:.3}a{r:.3} {r:.3} 0 0 1 {r:.3} {r:.3}v{:.3}a{r:.3} {r:.3} 0 0 1 -{r:.3} {r:.3}h-{:.3}a{r:.3} {r:.3} 0 0 1 -{r:.3} -{r:.3}v-{:.3}a{r:.3} {r:.3} 0 0 1 {r:.3} -{r:.3}Z",
            x + r,
            y,
            w - 2.0 * r,
            h - 2.0 * r,
            w - 2.0 * r,
            h - 2.0 * r
        )
    };
    let s = stroke / 2.0;
    format!("{}{}", rr(x - s, y - s, w + stroke, h + stroke, r + s), rr(x + s, y + s, w - stroke, h - stroke, (r - s).max(0.1)))
}

/// The zipGlyph of the "Archief (zip)" filter as a symbolic SVG file (written once to the runtime folder), for
/// the kit's drop-down, which takes an icon by path. None when it cannot be written.
pub fn zip_icon_path() -> Option<String> {
    let dir = gtk::glib::user_runtime_dir().join("win11-gnome");
    let path = dir.join("alles-zip-symbolic.svg");
    let st = 1.35;
    let tick = |y1: f64, y2: f64| {
        let r = st / 2.0;
        format!(
            "M{:.3} {:.3}a{r} {r} 0 0 1 {st} 0v{:.3}a{r} {r} 0 0 1 -{st} 0Z",
            10.0 - r,
            y1,
            y2 - y1
        )
    };
    let d = format!(
        "{}{}{}{}{}",
        outline(3.5, 2.75, 13.0, 14.5, 2.0, st),
        tick(2.75, 4.75),
        tick(6.25, 7.75),
        tick(9.25, 10.75),
        outline(8.5, 11.75, 3.0, 3.0, 0.8, st)
    );
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"20\" viewBox=\"0 0 20 20\"><path fill=\"#222222\" fill-rule=\"evenodd\" d=\"{d}\"/></svg>\n"
    );
    if std::fs::read_to_string(&path).ok().as_deref() != Some(svg.as_str()) {
        // The host's own runtime folder: private (0700), or its backends refuse it (host events.rs private_dir).
        std::os::unix::fs::DirBuilderExt::mode(std::fs::DirBuilder::new().recursive(true), 0o700).create(&dir).ok()?;
        let tmp = dir.join(format!("alles-zip-symbolic.svg.{}", std::process::id()));
        std::fs::write(&tmp, &svg).ok()?;
        std::fs::rename(&tmp, &path).ok()?;
    }
    Some(path.to_string_lossy().into_owned())
}

/// The text glyph of a toggle button (.alles-tglyph): "Aa", ".*" (monospace) or "ab" with the bracket under it
/// (.is-word).
pub fn text_glyph(text: &str, kind: &str) -> gtk::Widget {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("alles-tglyph");
    match kind {
        "mono" => {
            l.add_css_class("alles-tglyph-mono");
            l.upcast()
        }
        "word" => {
            l.add_css_class("alles-tglyph-word");
            let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
            b.add_css_class("alles-tglyph-box");
            b.set_valign(gtk::Align::Center);
            b.append(&l);
            let bracket = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            bracket.add_css_class("alles-tglyph-bracket");
            b.append(&bracket);
            b.upcast()
        }
        _ => l.upcast(),
    }
}
