//! Look: GTK CSS from the UI's tokens. css/base.css is the shell's palette and components (native/shell/css),
//! css/calendar.css the app's own sheet, and a generated part for the user's choices: the accent palette from
//! theme.accent (theme.js accentPalette, the colour maths below are native/shell/src/theme.rs's), the text
//! scale, tooltips, and each calendar's colour (`.cal-c-<id>` on chips, blocks and check boxes).
//!
//! The window gets the classes `w11`, `w11-dark` or `w11-light` and `w11-opaque` (the app's Mica is the solid
//! #202020 / #f3f3f3; the GNOME extension adds blur and the shadow around it).
//!
//! The text scale is GNOME's text-scaling-factor, as in the kit and the shell (settings parity, amendment 49):
//! GTK 4.22 applies it to point sizes only and this ramp is px, so the ramp is scaled by exactly GNOME's factor,
//! never by settings.json's textScale on top (config.rs `text_scale` is only the fallback). The key is read from
//! GSettings itself (GTK's Wayland backend takes its settings from the portal only; GTK's gtk-xft-dpi is the next
//! source), and a change of it applies the look again.
use crate::config::Config;
use crate::store::CalendarInfo;
use gtk::gdk;
use gtk::prelude::*;
use std::cell::RefCell;

const SHEETS: &[&str] = &[include_str!("../css/base.css"), include_str!("../css/calendar.css")];

thread_local! {
    static DYNAMIC: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static LAST_CSS: RefCell<String> = const { RefCell::new(String::new()) };
    static LOOK: RefCell<(bool, String)> = RefCell::new((true, "#4cc2ff".into()));
    /// What `apply` had last, for a change of the text scale.
    static LAST_APPLY: RefCell<Option<(Config, Vec<CalendarInfo>)>> = const { RefCell::new(None) };
    /// org.gnome.desktop.interface, where it has text-scaling-factor.
    static INTERFACE: RefCell<Option<gtk::gio::Settings>> = const { RefCell::new(None) };
}

/// GNOME's text-scaling-factor from GTK's dpi (96 x 1024 at 1.0), within the schema's 0.5 ..= 3.0; None for
/// GTK's "no dpi".
fn text_scale_of_dpi(dpi: i32) -> Option<f64> {
    (dpi > 0).then(|| (f64::from(dpi) / (96.0 * 1024.0)).clamp(0.5, 3.0))
}

/// The text scale in effect (module docs).
fn text_scale(cfg: &Config) -> f64 {
    INTERFACE
        .with(|i| i.borrow().as_ref().map(|i| i.double("text-scaling-factor")))
        .filter(|v| v.is_finite() && *v > 0.0)
        .map(|v| v.clamp(0.5, 3.0))
        .or_else(|| gtk::Settings::default().and_then(|s| text_scale_of_dpi(s.gtk_xft_dpi())))
        .unwrap_or(cfg.text_scale)
}

fn apply_again() {
    if let Some((cfg, calendars)) = LAST_APPLY.with(|l| l.borrow().clone()) {
        apply(&cfg, &calendars);
    }
}

pub fn install() {
    let Some(display) = gdk::Display::default() else { return };
    let fixed = gtk::CssProvider::new();
    fixed.connect_parsing_error(|_, section, error| {
        eprintln!("win11-calendar: css {}: {}", section.to_str(), error);
    });
    fixed.load_from_string(&SHEETS.join("\n"));
    gtk::style_context_add_provider_for_display(&display, &fixed, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let dynamic = gtk::CssProvider::new();
    dynamic.connect_parsing_error(|_, section, error| {
        eprintln!("win11-calendar: theme css {}: {}", section.to_str(), error);
    });
    gtk::style_context_add_provider_for_display(&display, &dynamic, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    DYNAMIC.with(|d| *d.borrow_mut() = Some(dynamic));
    let interface = gtk::gio::SettingsSchemaSource::default()
        .and_then(|source| source.lookup("org.gnome.desktop.interface", true))
        .filter(|schema| schema.has_key("text-scaling-factor"))
        .map(|_| gtk::gio::Settings::new("org.gnome.desktop.interface"));
    if let Some(i) = &interface {
        i.connect_changed(Some("text-scaling-factor"), |_, _| apply_again());
    }
    INTERFACE.with(|slot| *slot.borrow_mut() = interface);
    if let Some(settings) = gtk::Settings::default() {
        settings.connect_gtk_xft_dpi_notify(|_| apply_again());
    }
}

/// Apply the settings and the calendars' colours: CSS, GTK's colour scheme, the window classes.
pub fn apply(cfg: &Config, calendars: &[CalendarInfo]) {
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_interface_color_scheme(if cfg.dark {
            gtk::InterfaceColorScheme::Dark
        } else {
            gtk::InterfaceColorScheme::Light
        });
    }
    LAST_APPLY.with(|l| *l.borrow_mut() = Some((cfg.clone(), calendars.to_vec())));
    let palette = accent_palette(&cfg.accent);
    let fill = if cfg.dark { palette.light2.clone() } else { palette.dark1.clone() };
    LOOK.with(|l| *l.borrow_mut() = (cfg.dark, fill));
    let css = dynamic_css(cfg, text_scale(cfg), &palette, calendars);
    let changed = LAST_CSS.with(|last| {
        let mut last = last.borrow_mut();
        if *last == css {
            false
        } else {
            *last = css.clone();
            true
        }
    });
    if changed {
        DYNAMIC.with(|d| {
            if let Some(p) = d.borrow().as_ref() {
                p.load_from_string(&css);
            }
        });
    }
    for w in gtk::Window::list_toplevels() {
        if let Ok(w) = w.downcast::<gtk::Window>() {
            style_window(&w);
        }
    }
}

pub fn style_window(window: &gtk::Window) {
    let dark = LOOK.with(|l| l.borrow().0);
    window.add_css_class("w11");
    window.add_css_class("w11-opaque");
    toggle(window, "w11-dark", dark);
    toggle(window, "w11-light", !dark);
}

/// The accent fill in effect (light2 in dark mode, dark1 in light mode), for cairo drawing.
pub fn accent_rgba() -> gdk::RGBA {
    let hex = LOOK.with(|l| l.borrow().1.clone());
    gdk::RGBA::parse(hex.as_str()).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0))
}

/// A calendar colour as RGBA.
pub fn color_rgba(hex_in: &str) -> gdk::RGBA {
    let (r, g, b) = rgb(hex_in);
    gdk::RGBA::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

fn toggle(window: &gtk::Window, class: &str, on: bool) {
    if on {
        window.add_css_class(class);
    } else {
        window.remove_css_class(class);
    }
}

fn dynamic_css(cfg: &Config, ts: f64, p: &Palette, calendars: &[CalendarInfo]) -> String {
    let dark = cfg.dark;
    let (accent, accent_text) = if dark { (&p.light2, &p.on_dark) } else { (&p.dark1, &p.on_light) };
    let a = rgb(accent);
    let at = rgb(accent_text);
    let mut css = String::new();
    css.push_str("window.w11 {\n");
    for (name, value) in [
        ("accent-base", &p.base),
        ("accent-light-1", &p.light1),
        ("accent-light-2", &p.light2),
        ("accent-light-3", &p.light3),
        ("accent-dark-1", &p.dark1),
        ("accent-dark-2", &p.dark2),
        ("accent-dark-3", &p.dark3),
        ("accent-on-light", &p.on_light),
        ("accent-on-dark", &p.on_dark),
    ] {
        css.push_str(&format!("  --{name}: {value};\n"));
    }
    let fg = |d: &String, l: &String| if dark { d.clone() } else { l.clone() };
    css.push_str(&format!("  --accent: {accent};\n"));
    css.push_str(&format!("  --accent-2: {};\n", rgba(a, 0.9)));
    css.push_str(&format!("  --accent-3: {};\n", rgba(a, 0.8)));
    css.push_str(&format!("  --accent-text: {accent_text};\n"));
    css.push_str(&format!("  --accent-text-2: {};\n", rgba(at, if dark { 0.5 } else { 0.7 })));
    css.push_str(&format!("  --accent-fg: {};\n", fg(&p.light3, &p.dark2)));
    css.push_str(&format!("  --accent-fg-2: {};\n", fg(&p.light3, &p.dark3)));
    css.push_str(&format!("  --accent-fg-3: {};\n", fg(&p.light2, &p.dark1)));
    css.push_str(&format!("  --accent-subtle: {};\n", rgba(a, if dark { 0.14 } else { 0.12 })));
    css.push_str(&format!("  --selection-bg: {accent};\n"));
    css.push_str(&format!("  --selection-text: {};\n", if dark { "#000000" } else { "#ffffff" }));
    css.push_str(&format!("  --selection-subtle: {};\n", rgba(a, if dark { 0.22 } else { 0.18 })));
    for (name, px) in [
        ("fs-caption", 12.0),
        ("fs-body", 14.0),
        ("fs-body-large", 18.0),
        ("fs-subtitle", 20.0),
        ("fs-title", 28.0),
        ("fs-title-large", 40.0),
        ("fs-display", 68.0),
    ] {
        css.push_str(&format!("  --{name}: {}px;\n", round2(px * ts)));
    }
    css.push_str("}\n");

    let (tip_bg, tip_fg, tip_stroke, tip_shadow) = if dark {
        ("#2c2c2c", "#ffffff", "rgba(0, 0, 0, 0.2)", "rgba(0, 0, 0, 0.26)")
    } else {
        ("#f9f9f9", "rgba(0, 0, 0, 0.8956)", "rgba(0, 0, 0, 0.0578)", "rgba(0, 0, 0, 0.14)")
    };
    css.push_str(&format!(
        "tooltip, tooltip.background {{\n  background-color: {tip_bg};\n  background-image: none;\n  color: {tip_fg};\n  \
         border: 1px solid {tip_stroke};\n  border-radius: 4px;\n  padding: 5px 8px 7px;\n  box-shadow: 0 4px 8px {tip_shadow};\n  \
         text-shadow: none;\n  font-family: \"Segoe UI Variable Text\", \"Segoe UI Variable\", \"Segoe UI\", \"Selawik\", \"Ubuntu Sans\", \"Noto Sans\", sans-serif;\n  \
         font-size: {}px;\n}}\ntooltip > box {{ border-spacing: 6px; }}\ntooltip label {{ color: {tip_fg}; }}\n",
        round2(12.0 * ts)
    ));

    // Calendar colours: tinted event blocks and all-day bars with a solid edge, coloured check boxes.
    let (block, block_hover, banner, banner_hover) = if dark { (0.36, 0.46, 0.32, 0.42) } else { (0.22, 0.3, 0.2, 0.28) };
    for c in calendars {
        let cls = c.css();
        let col = rgb(&c.color);
        let solid = hex(col);
        css.push_str(&format!(
            "button.cal-block.{cls} {{ background-color: {}; border-left-color: {solid}; }}\n\
             button.cal-block.{cls}:hover {{ background-color: {}; }}\n\
             button.cal-chip.is-banner.{cls} {{ background-color: {}; box-shadow: inset 3px 0 0 {solid}; }}\n\
             button.cal-chip.is-banner.{cls}:hover {{ background-color: {}; }}\n\
             checkbutton.cal-cal-check.{cls} > check {{ border-color: {solid}; }}\n\
             checkbutton.cal-cal-check.{cls} > check:checked, checkbutton.cal-cal-check.{cls}:hover > check:checked {{ background-color: {solid}; border-color: {solid}; color: {}; }}\n",
            rgba(col, block),
            rgba(col, block_hover),
            rgba(col, banner),
            rgba(col, banner_hover),
            text_on(&solid, "#ffffff"),
        ));
    }
    css
}

// ----------------------------------------------------------------------------- colour maths (theme.js)

/// The Win11-like accent palette of a base colour (theme.js accentPalette): light mode fills use dark1,
/// dark mode fills use light2, like Windows.
#[derive(Clone, PartialEq, Debug)]
pub struct Palette {
    pub base: String,
    pub light1: String,
    pub light2: String,
    pub light3: String,
    pub dark1: String,
    pub dark2: String,
    pub dark3: String,
    pub on_light: String,
    pub on_dark: String,
}

pub fn accent_palette(input: &str) -> Palette {
    let hex_in = normalize_hex(input).unwrap_or_else(|| "#0078d4".into());
    let mut p = if hex_in == "#0078d4" || hex_in == "#0067c0" {
        // Windows' own palette for the default blue (registry AccentPalette).
        Palette {
            base: "#0078d4".into(),
            light1: "#0091f8".into(),
            light2: "#4cc2ff".into(),
            light3: "#99ebff".into(),
            dark1: "#0067c0".into(),
            dark2: "#003e92".into(),
            dark3: "#001a68".into(),
            on_light: String::new(),
            on_dark: String::new(),
        }
    } else {
        let (l, c, h) = hex_to_oklch(&hex_in);
        let mk = |lv: f64, c_mul: f64| oklch_to_hex(lv.clamp(0.12, 0.97), c * c_mul, h);
        // Lightness steps calibrated on Windows' own blue palette (OKLCH), clamped so that dark-mode fills
        // stay light enough and light-mode fills dark enough.
        let up = |k: f64| l + (0.95 - l) * k;
        let l1 = up(0.2).max(0.6);
        let l2 = up(0.55).max(0.72);
        let l3 = up(0.85).max(0.86);
        let d1 = (l * 0.91).min(0.6);
        let d2 = (l * 0.68).min(0.47);
        let d3 = (l * 0.475).min(0.34);
        Palette {
            base: hex_in.clone(),
            light1: mk(l1, 1.02),
            light2: mk(l2, 0.86),
            light3: mk(l3, 0.55),
            dark1: mk(d1, 0.97),
            dark2: mk(d2, 0.9),
            dark3: mk(d3, 0.83),
            on_light: String::new(),
            on_dark: String::new(),
        }
    };
    p.on_light = text_on(&p.dark1, "#ffffff");
    p.on_dark = text_on(&p.light2, "#000000");
    p
}

fn normalize_hex(input: &str) -> Option<String> {
    let mut h = input.trim().to_lowercase();
    if !h.starts_with('#') {
        h.insert(0, '#');
    }
    let digits = &h[1..];
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        3 => Some(format!("#{}", digits.chars().flat_map(|c| [c, c]).collect::<String>())),
        6 => Some(h),
        8 => Some(h[..7].to_owned()),
        _ => None,
    }
}

pub fn rgb(hex: &str) -> (u8, u8, u8) {
    let n = normalize_hex(hex).and_then(|h| u32::from_str_radix(&h[1..], 16).ok()).unwrap_or(0);
    (((n >> 16) & 255) as u8, ((n >> 8) & 255) as u8, (n & 255) as u8)
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn rgba((r, g, b): (u8, u8, u8), a: f64) -> String {
    format!("rgba({r}, {g}, {b}, {})", round2(a))
}


fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn to_lin(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_lin(c: f64) -> f64 {
    255.0 * if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn hex_to_oklch(hex: &str) -> (f64, f64, f64) {
    let (r, g, b) = rgb(hex);
    let (r, g, b) = (to_lin(r as f64), to_lin(g as f64), to_lin(b as f64));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let big_l = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    (big_l, a.hypot(bb), (bb.atan2(a).to_degrees() + 360.0) % 360.0)
}

fn oklch_to_lin_rgb(l: f64, c: f64, h: f64) -> [f64; 3] {
    let hr = h.to_radians();
    let (a, b) = (c * hr.cos(), c * hr.sin());
    let l_ = (l + 0.3963377774 * a + 0.2158037573 * b).powi(3);
    let m_ = (l - 0.1055613458 * a - 0.0638541728 * b).powi(3);
    let s_ = (l - 0.0894841775 * a - 1.291485548 * b).powi(3);
    [
        4.0767416621 * l_ - 3.3077115913 * m_ + 0.2309699292 * s_,
        -1.2684380046 * l_ + 2.6097574011 * m_ - 0.3413193965 * s_,
        -0.0041960863 * l_ - 0.7034186147 * m_ + 1.707614701 * s_,
    ]
}

/// OKLCH to hex, reducing chroma until the colour fits in sRGB.
fn oklch_to_hex(l: f64, c: f64, h: f64) -> String {
    let l = l.clamp(0.0, 1.0);
    let in_gamut = |v: &[f64; 3]| v.iter().all(|x| *x >= -0.0005 && *x <= 1.0005);
    let mut rgb_lin = oklch_to_lin_rgb(l, c, h);
    if !in_gamut(&rgb_lin) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..22 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(&oklch_to_lin_rgb(l, mid, h)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb_lin = oklch_to_lin_rgb(l, lo, h);
    }
    let ch = |v: f64| from_lin(v.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8;
    hex((ch(rgb_lin[0]), ch(rgb_lin[1]), ch(rgb_lin[2])))
}

fn luminance(hex: &str) -> f64 {
    let (r, g, b) = rgb(hex);
    0.2126 * to_lin(r as f64) + 0.7152 * to_lin(g as f64) + 0.0722 * to_lin(b as f64)
}

fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Text colour (#ffffff or #000000) for a fill; `prefer` wins when it is readable enough.
fn text_on(fill: &str, prefer: &str) -> String {
    let other = if prefer == "#ffffff" { "#000000" } else { "#ffffff" };
    if contrast(prefer, fill) >= 3.5 {
        return prefer.to_owned();
    }
    if contrast(other, fill) > contrast(prefer, fill) { other.to_owned() } else { prefer.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ramp_is_scaled_by_gtks_factor_once() {
        assert_eq!(text_scale_of_dpi(96 * 1024), Some(1.0));
        assert_eq!(text_scale_of_dpi(147_456), Some(1.5));
        assert_eq!(text_scale_of_dpi(-1), None);
        let cfg = Config { text_scale: 1.25, ..Config::default() };
        let css = dynamic_css(&cfg, 1.5, &accent_palette(&cfg.accent), &[]);
        assert!(css.contains("--fs-body: 21px;"), "14 px x GNOME's 1.5, not x 1.25 too");
    }
}
