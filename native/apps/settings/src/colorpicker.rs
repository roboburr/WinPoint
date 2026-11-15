//! The custom colour dialog "Een aangepaste kleur kiezen" (6ebf169f:vendor/ui/js/apps/settings/colorpicker.js): a
//! hue/saturation spectrum, a brightness slider and a hex field, with a readability check for accent colours.
//! `pick_color(anchor, value, title, accent, done)` gives `done(Some("#rrggbb"))` on Gereed, None otherwise.
use kit::controls::{self as c, Dialog, DialogResult, Slider};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

/// hexToHsv: (hue 0..360, saturation 0..1, value 0..1).
pub fn hex_to_hsv(hex: &str) -> (f64, f64, f64) {
    let n = kit::theme::normalize_hex(hex).unwrap_or_else(|| "#0078d4".into());
    let (r, g, b) = kit::theme::rgb(&n);
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let mut hue = 0.0;
    if d > 0.0 {
        hue = if max == r {
            ((g - b) / d) % 6.0
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        };
        hue = (hue * 60.0 + 360.0) % 360.0;
    }
    (hue, if max > 0.0 { d / max } else { 0.0 }, max)
}

/// hsvToHex.
pub fn hsv_to_hex(h: f64, s: f64, v: f64) -> String {
    let f = |n: f64| {
        let k = (n + h / 60.0) % 6.0;
        v - v * s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    let ch = |x: f64| (x * 255.0).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", ch(f(5.0)), ch(f(3.0)), ch(f(1.0)))
}

struct State {
    hue: Cell<f64>,
    sat: Cell<f64>,
    val: Cell<f64>,
    hex: RefCell<String>,
}

/// Show the dialog over the window of `anchor`; `title` None: "Een aangepaste kleur kiezen"; `accent`: show
/// the light and dark previews and the readability warning.
pub fn pick_color(anchor: &impl IsA<gtk::Widget>, value: &str, title: Option<&str>, accent: bool, done: impl FnOnce(Option<String>) + 'static) {
    let hex0 = kit::theme::normalize_hex(value).unwrap_or_else(|| "#0078d4".into());
    let (h, s, v) = hex_to_hsv(&hex0);
    let st = Rc::new(State { hue: Cell::new(h), sat: Cell::new(s), val: Cell::new(v), hex: RefCell::new(hex0.clone()) });

    // per-dialog colours of the swatch and the two chips
    let provider = gtk::CssProvider::new();
    if let Some(d) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(&d, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 5);
    }

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("st-cp");
    content.set_size_request(360, -1);

    let spectrum = gtk::DrawingArea::new();
    spectrum.add_css_class("st-cp-spectrum");
    spectrum.set_content_height(188);
    spectrum.set_hexpand(true);
    spectrum.set_focusable(true);
    spectrum.set_can_focus(true);
    spectrum.set_widget_name("colorpicker-spectrum");
    spectrum.set_tooltip_text(Some(t("Kleur en verzadiging", "Hue and saturation")));
    {
        let st = st.clone();
        spectrum.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let r = 6.0;
            cr.new_sub_path();
            cr.arc(w - r, r, r, -PI / 2.0, 0.0);
            cr.arc(w - r, h - r, r, 0.0, PI / 2.0);
            cr.arc(r, h - r, r, PI / 2.0, PI);
            cr.arc(r, r, r, PI, 1.5 * PI);
            cr.close_path();
            cr.clip();
            let hues = gtk::cairo::LinearGradient::new(0.0, 0.0, w, 0.0);
            for (i, (rr, gg, bb)) in [(1.0, 0.0, 0.0), (1.0, 1.0, 0.0), (0.0, 1.0, 0.0), (0.0, 1.0, 1.0), (0.0, 0.0, 1.0), (1.0, 0.0, 1.0), (1.0, 0.0, 0.0)]
                .into_iter()
                .enumerate()
            {
                hues.add_color_stop_rgb(i as f64 / 6.0, rr, gg, bb);
            }
            let _ = cr.set_source(&hues);
            let _ = cr.paint();
            let white = gtk::cairo::LinearGradient::new(0.0, 0.0, 0.0, h);
            white.add_color_stop_rgba(0.0, 1.0, 1.0, 1.0, 0.0);
            white.add_color_stop_rgba(1.0, 1.0, 1.0, 1.0, 1.0);
            let _ = cr.set_source(&white);
            let _ = cr.paint();
            // .st-cp__dim: black at 1 - brightness
            cr.set_source_rgba(0.0, 0.0, 0.0, 1.0 - st.val.get());
            let _ = cr.paint();
            // the inset hairline
            let stroke = kit::theme::token("stroke-card");
            cr.set_source_rgba(stroke.red() as f64, stroke.green() as f64, stroke.blue() as f64, stroke.alpha() as f64);
            cr.set_line_width(2.0);
            cr.new_sub_path();
            cr.arc(w - r, r, r, -PI / 2.0, 0.0);
            cr.arc(w - r, h - r, r, 0.0, PI / 2.0);
            cr.arc(r, h - r, r, PI / 2.0, PI);
            cr.arc(r, r, r, PI, 1.5 * PI);
            cr.close_path();
            let _ = cr.stroke();
            cr.reset_clip();
            // the thumb: 20 px, 2 px white border, a dark ring and a soft shadow, filled with the colour
            let x = st.hue.get() / 360.0 * w;
            let y = (1.0 - st.sat.get()) * h;
            let (cr_, cg, cb) = kit::theme::rgb(&st.hex.borrow());
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.35);
            cr.arc(x, y + 2.0, 11.5, 0.0, 2.0 * PI);
            let _ = cr.fill();
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.45);
            cr.arc(x, y, 11.0, 0.0, 2.0 * PI);
            let _ = cr.fill();
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.arc(x, y, 10.0, 0.0, 2.0 * PI);
            let _ = cr.fill();
            cr.set_source_rgb(cr_ as f64 / 255.0, cg as f64 / 255.0, cb as f64 / 255.0);
            cr.arc(x, y, 8.0, 0.0, 2.0 * PI);
            let _ = cr.fill();
        });
    }
    kit::theme::track(&spectrum);
    content.append(&spectrum);

    let bright_label = c::label(t("Helderheid", "Brightness"), "st-cp-label", false);
    content.append(&bright_label);
    let bright = Slider::new(0.0, 100.0, 1.0, (v * 100.0).round());
    bright.set_value_visible(false);
    bright.widget().set_hexpand(true);
    bright.scale().set_tooltip_text(Some(t("Helderheid", "Brightness")));
    content.append(bright.widget());

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    row.add_css_class("st-cp-row");
    let prev = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    prev.set_hexpand(true);
    prev.set_valign(gtk::Align::End);
    let swatch = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    swatch.add_css_class("st-cp-swatch");
    swatch.set_size_request(56, 56);
    swatch.set_valign(gtk::Align::Center);
    prev.append(&swatch);
    let chip_light = c::label(t("Licht", "Light"), "st-cp-chip st-cp-chip-light", false);
    let chip_dark = c::label(t("Donker", "Dark"), "st-cp-chip st-cp-chip-dark", false);
    if accent {
        let chips = gtk::Box::new(gtk::Orientation::Vertical, 4);
        chips.set_valign(gtk::Align::Center);
        chip_light.set_halign(gtk::Align::Start);
        chip_dark.set_halign(gtk::Align::Start);
        chips.append(&chip_light);
        chips.append(&chip_dark);
        prev.append(&chips);
    }
    row.append(&prev);
    let hex_col = gtk::Box::new(gtk::Orientation::Vertical, 8);
    hex_col.add_css_class("w11-field");
    hex_col.append(&c::label(t("Hexadecimaal", "Hexadecimal"), "w11-field-header", false));
    let hex_box = c::text_box("");
    hex_box.set_max_length(7);
    hex_box.set_width_chars(10);
    hex_box.set_size_request(150, -1);
    hex_box.set_text(&hex0.to_uppercase());
    hex_box.set_widget_name("colorpicker-hex");
    hex_col.append(&hex_box);
    let hex_err = c::label(t("Typ een kleur als #0078D4.", "Type a colour like #0078D4."), "w11-field-desc w11-text-critical", true);
    hex_err.set_visible(false);
    hex_col.append(&hex_err);
    row.append(&hex_col);
    content.append(&row);
    let warn = c::label(
        t(
            "Deze kleur is misschien moeilijk leesbaar. Kies een iets lichtere of donkerdere kleur.",
            "This colour may be hard to read. Pick a slightly lighter or darker colour.",
        ),
        "st-cp-warn",
        true,
    );
    warn.set_visible(false);
    if accent {
        content.append(&warn);
    }

    // paint(updateBox): spectrum, swatch, chips, warning; the hex text when it did not come from the box
    let silent = Rc::new(Cell::new(false));
    let paint = {
        let (st, spectrum, provider, hex_box, hex_err, warn, silent) =
            (st.clone(), spectrum.clone(), provider.clone(), hex_box.clone(), hex_err.clone(), warn.clone(), silent.clone());
        Rc::new(move |update_box: bool| {
            let hex = st.hex.borrow().clone();
            spectrum.queue_draw();
            let mut css = format!(".st-cp-swatch {{ background-color: {hex}; }}\n");
            if accent {
                let p = kit::theme::accent_palette(&hex);
                css.push_str(&format!(
                    ".st-cp-chip-light {{ background-color: {}; color: {}; }}\n.st-cp-chip-dark {{ background-color: {}; color: {}; }}\n",
                    p.dark1, p.on_light, p.light2, p.on_dark
                ));
                let worst = kit::theme::contrast(&p.dark1, &p.on_light)
                    .min(kit::theme::contrast(&p.light2, &p.on_dark))
                    .min(kit::theme::contrast(&p.dark1, "#f3f3f3"))
                    .min(kit::theme::contrast(&p.light2, "#202020"));
                warn.set_visible(worst < 2.2);
            }
            provider.load_from_string(&css);
            if update_box {
                silent.set(true);
                hex_box.set_text(&hex.to_uppercase());
                silent.set(false);
                hex_err.set_visible(false);
            }
        })
    };
    let from_hsv = {
        let (st, paint) = (st.clone(), paint.clone());
        Rc::new(move || {
            *st.hex.borrow_mut() = hsv_to_hex(st.hue.get(), st.sat.get(), st.val.get());
            paint(true);
        })
    };

    {
        let (st, f) = (st.clone(), from_hsv.clone());
        bright.connect_changed(move |v| {
            st.val.set(v / 100.0);
            f();
        });
    }
    // parsed live, so Enter (which confirms the dialog) always uses what was typed
    {
        let (st, paint, bright, hex_err, silent) = (st.clone(), paint.clone(), bright.clone(), hex_err.clone(), silent.clone());
        hex_box.connect_changed(move |e| {
            if silent.get() {
                return;
            }
            let v = e.text().trim().to_owned();
            let body = v.trim_start_matches('#');
            let ok = (body.len() == 3 || body.len() == 6) && body.chars().all(|c| c.is_ascii_hexdigit());
            let Some(n) = ok.then(|| kit::theme::normalize_hex(&v)).flatten() else { return };
            hex_err.set_visible(false);
            let (h, s, vv) = hex_to_hsv(&n);
            st.hue.set(h);
            st.sat.set(s);
            st.val.set(vv);
            *st.hex.borrow_mut() = n;
            bright.set_value((vv * 100.0).round());
            paint(false);
        });
    }
    {
        let (hex_err, hb) = (hex_err.clone(), hex_box.clone());
        let focus = gtk::EventControllerFocus::new();
        focus.connect_leave(move |_| {
            if kit::theme::normalize_hex(hb.text().trim()).is_none() {
                hex_err.set_visible(true);
            }
        });
        hex_box.add_controller(focus);
    }
    // pointer: pick hue (x) and saturation (y); a nearly black colour gets 85 % brightness first
    let from_point = {
        let (st, f, bright, spectrum) = (st.clone(), from_hsv.clone(), bright.clone(), spectrum.clone());
        Rc::new(move |x: f64, y: f64| {
            let (w, h) = (spectrum.width().max(1) as f64, spectrum.height().max(1) as f64);
            st.hue.set((x / w).clamp(0.0, 1.0) * 360.0);
            st.sat.set(1.0 - (y / h).clamp(0.0, 1.0));
            if st.val.get() < 0.15 {
                st.val.set(0.85);
                bright.set_value(85.0);
            }
            f();
        })
    };
    let drag = gtk::GestureDrag::new();
    {
        let (fp, sp) = (from_point.clone(), spectrum.clone());
        drag.connect_drag_begin(move |_, x, y| {
            sp.grab_focus();
            fp(x, y);
        });
    }
    {
        let fp = from_point.clone();
        drag.connect_drag_update(move |g, dx, dy| {
            if let Some((x, y)) = g.start_point() {
                fp(x + dx, y + dy);
            }
        });
    }
    spectrum.add_controller(drag);
    let keys = gtk::EventControllerKey::new();
    {
        let (st, f) = (st.clone(), from_hsv.clone());
        keys.connect_key_pressed(move |_, key, _, mods| {
            let step = if mods.contains(gdk::ModifierType::SHIFT_MASK) { 10.0 } else { 2.0 };
            match key {
                gdk::Key::Left => st.hue.set((st.hue.get() - step + 360.0) % 360.0),
                gdk::Key::Right => st.hue.set((st.hue.get() + step) % 360.0),
                gdk::Key::Up => st.sat.set((st.sat.get() + step / 100.0).clamp(0.0, 1.0)),
                gdk::Key::Down => st.sat.set((st.sat.get() - step / 100.0).clamp(0.0, 1.0)),
                _ => return glib::Propagation::Proceed,
            }
            f();
            glib::Propagation::Stop
        });
    }
    spectrum.add_controller(keys);
    paint(true);

    let st2 = st.clone();
    Dialog::new(title.unwrap_or(t("Een aangepaste kleur kiezen", "Pick a custom colour")))
        .body(&content)
        .primary(t("Gereed", "Done"))
        .close_label(t("Annuleren", "Cancel"))
        .width(420)
        .show(anchor, move |r| {
            if let Some(d) = gdk::Display::default() {
                gtk::style_context_remove_provider_for_display(&d, &provider);
            }
            done(if r == DialogResult::Primary { Some(st2.hex.borrow().clone()) } else { None });
        });
}
