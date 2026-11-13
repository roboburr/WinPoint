//! Look: GTK CSS from the UI's tokens (vendor/ui/css/tokens.css, shell.css, flyouts.css), one stylesheet per
//! module in css/, plus a generated part for the user's choices (theme.js `apply`): the accent palette from
//! theme.accent (OKLCH steps like Windows' AccentPalette, theme.js `accentPalette`), the text scale, and
//! "Accentkleur weergeven op Start en taakbalk" (theme.accentOnTaskbar, dark mode only, shell.js
//! data-accent-surfaces).
//!
//! Every toplevel gets the classes `w11`, `w11-dark` or `w11-light`, `w11-transparent` or `w11-opaque`
//! (theme.transparency), and `w11-accent-surfaces` / `w11-high-contrast` / `w11-reduce-motion` when on;
//! windows created later get them as they are added (gtk::Window::toplevels), and `style_window` does it on
//! demand.
use crate::core::Shell;
use gtk::prelude::*;
use gtk::{gdk, gio};
use std::cell::RefCell;
use std::rc::Rc;

const SHEETS: &[&str] = &[
    include_str!("../css/base.css"),
    include_str!("../css/taskbar.css"),
    include_str!("../css/quick.css"),
    include_str!("../css/notifications.css"),
    include_str!("../css/menus.css"),
    include_str!("../css/start.css"),
    include_str!("../css/desktop.css"),
];

/// The user's look as theme.js applies it.
#[derive(Clone, PartialEq, Debug)]
pub struct Look {
    pub dark: bool,
    pub transparency: bool,
    pub accent_surfaces: bool,
    pub high_contrast: bool,
    pub reduce_motion: bool,
    pub text_scale: f64,
    pub palette: Palette,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            dark: true,
            transparency: true,
            accent_surfaces: false,
            high_contrast: false,
            reduce_motion: false,
            text_scale: 1.0,
            palette: accent_palette("#0078d4"),
        }
    }
}

thread_local! {
    static DYNAMIC: RefCell<Option<gtk::CssProvider>> = const { RefCell::new(None) };
    static LOOK: RefCell<Look> = RefCell::new(Look::default());
    static LAST_CSS: RefCell<String> = const { RefCell::new(String::new()) };
    static TOPLEVELS: RefCell<Option<gio::ListModel>> = const { RefCell::new(None) };
}

pub fn install(shell: &Rc<Shell>) {
    let Some(display) = gdk::Display::default() else { return };
    let fixed = gtk::CssProvider::new();
    fixed.connect_parsing_error(|_, section, error| {
        eprintln!("win11-shell: css {}: {}", section.to_str(), error);
    });
    fixed.load_from_string(&SHEETS.join("\n"));
    gtk::style_context_add_provider_for_display(&display, &fixed, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let dynamic = gtk::CssProvider::new();
    dynamic.connect_parsing_error(|_, section, error| {
        eprintln!("win11-shell: theme css {}: {}", section.to_str(), error);
    });
    gtk::style_context_add_provider_for_display(&display, &dynamic, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    DYNAMIC.with(|d| *d.borrow_mut() = Some(dynamic));

    // Windows made later (Start, the desktop, dialogs) get the classes the moment they exist.
    let toplevels = gtk::Window::toplevels();
    toplevels.connect_items_changed(|model, position, _removed, added| {
        for i in position..position + added {
            if let Some(window) = model.item(i).and_downcast::<gtk::Window>() {
                style_window(&window);
            }
        }
    });
    TOPLEVELS.with(|t| *t.borrow_mut() = Some(toplevels));

    refresh(shell);
    shell.on("settings.changed", |_, _| refresh(&crate::core::shell()));
}

/// The user's choices as CSS: the accent palette, the text scale and the accent surfaces as custom
/// properties, and the classes on every toplevel.
pub fn refresh(shell: &Shell) {
    let look = look_of(shell);
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_interface_color_scheme(if look.dark {
            gtk::InterfaceColorScheme::Dark
        } else {
            gtk::InterfaceColorScheme::Light
        });
        settings.set_gtk_enable_animations(!look.reduce_motion);
    }
    let css = dynamic_css(&look);
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
    let look_changed = LOOK.with(|l| {
        let mut l = l.borrow_mut();
        let differs = *l != look;
        *l = look;
        differs
    });
    for w in gtk::Window::list_toplevels() {
        if let Ok(w) = w.downcast::<gtk::Window>() {
            style_window(&w);
        }
    }
    if look_changed {
        crate::icons::invalidate();
    }
}

/// Give a window the theme's classes now (refresh and new toplevels do this by themselves).
pub fn style_window(window: &gtk::Window) {
    let look = LOOK.with(|l| l.borrow().clone());
    window.add_css_class("w11");
    toggle(window, "w11-dark", look.dark);
    toggle(window, "w11-light", !look.dark);
    toggle(window, "w11-transparent", look.transparency);
    toggle(window, "w11-opaque", !look.transparency);
    toggle(window, "w11-accent-surfaces", look.accent_surfaces);
    toggle(window, "w11-high-contrast", look.high_contrast);
    toggle(window, "w11-reduce-motion", look.reduce_motion);
}

/// The look in effect (after `refresh`).
pub fn look() -> Look {
    LOOK.with(|l| l.borrow().clone())
}

/// The accent fill colour in effect (light2 in dark mode, dark1 in light mode), for cairo drawing.
pub fn accent_rgba() -> gdk::RGBA {
    let look = look();
    let hex = if look.dark { look.palette.light2 } else { look.palette.dark1 };
    gdk::RGBA::parse(hex.as_str()).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0))
}

fn toggle(window: &gtk::Window, class: &str, on: bool) {
    if on {
        window.add_css_class(class);
    } else {
        window.remove_css_class(class);
    }
}

fn look_of(shell: &Shell) -> Look {
    let s = shell.settings();
    let dark = shell.dark();
    let accent = s["theme"]["accent"].as_str().unwrap_or("#0078d4");
    let ts = s["accessibility"]["textScale"].as_f64().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(100.0);
    Look {
        dark,
        transparency: s["theme"]["transparency"] != false,
        accent_surfaces: s["theme"]["accentOnTaskbar"] == true && dark,
        high_contrast: s["accessibility"]["highContrast"] == true,
        reduce_motion: s["accessibility"]["reduceMotion"] == true,
        text_scale: ts.clamp(100.0, 225.0) / 100.0,
        palette: accent_palette(accent),
    }
}

fn dynamic_css(look: &Look) -> String {
    let p = &look.palette;
    let (accent, accent_text) = if look.dark { (&p.light2, &p.on_dark) } else { (&p.dark1, &p.on_light) };
    let a = rgb(accent);
    let at = rgb(accent_text);
    let mut css = String::new();
    // Accent palette (tokens.css :root / [data-theme]) and its light/dark mapping, the text ramp (--ts).
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
    let fg = |dark: &String, light: &String| if look.dark { dark.clone() } else { light.clone() };
    css.push_str(&format!("  --accent: {accent};\n"));
    css.push_str(&format!("  --accent-2: {};\n", rgba(a, 0.9)));
    css.push_str(&format!("  --accent-3: {};\n", rgba(a, 0.8)));
    css.push_str(&format!("  --accent-text: {accent_text};\n"));
    css.push_str(&format!("  --accent-text-2: {};\n", rgba(at, if look.dark { 0.5 } else { 0.7 })));
    css.push_str(&format!("  --accent-fg: {};\n", fg(&p.light3, &p.dark2)));
    css.push_str(&format!("  --accent-fg-2: {};\n", fg(&p.light3, &p.dark3)));
    css.push_str(&format!("  --accent-fg-3: {};\n", fg(&p.light2, &p.dark1)));
    css.push_str(&format!("  --accent-subtle: {};\n", rgba(a, if look.dark { 0.14 } else { 0.12 })));
    css.push_str(&format!("  --selection-bg: {accent};\n"));
    css.push_str(&format!("  --selection-text: {};\n", if look.dark { "#000000" } else { "#ffffff" }));
    css.push_str(&format!("  --selection-subtle: {};\n", rgba(a, if look.dark { 0.22 } else { 0.18 })));
    css.push_str(&format!("  --sys-attention: {accent};\n"));
    let ts = look.text_scale;
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

    // Accent colour on Start and taskbar (shell.css :root[data-accent-surfaces], native-backdrop.css): a deep
    // accent-tinted taskbar (46 % of accent-dark-2) and Start / search / notification centre / calendar
    // (40 %), 70 % and 90 % see-through over the extension's blur when transparency is on.
    if look.accent_surfaces {
        let d2 = rgb(&p.dark2);
        let tb_solid = mix(d2, (0x14, 0x14, 0x14), 0.46);
        let layer_solid = mix(d2, (0x1c, 0x1c, 0x1c), 0.40);
        let layer_glass = mix(d2, (24, 24, 24), 0.40);
        css.push_str("window.w11.w11-accent-surfaces {\n");
        css.push_str(&format!("  --tb-solid: {};\n", hex(tb_solid)));
        let tb_glass = if look.transparency { rgba(tb_solid, 0.7) } else { hex(tb_solid) };
        css.push_str(&format!("  --tb-glass: {tb_glass};\n  --tb-tint: {tb_glass};\n"));
        css.push_str("  --tb-hover: rgba(255, 255, 255, 0.09);\n  --tb-active: rgba(255, 255, 255, 0.08);\n");
        css.push_str(&format!("  --layer-accent-solid: {};\n", hex(layer_solid)));
        if look.transparency {
            css.push_str(&format!("  --layer-accent: {};\n", rgba(layer_glass, 0.9)));
        } else {
            css.push_str(&format!("  --layer-accent: {};\n", hex(layer_solid)));
        }
        css.push_str("}\n");
    }

    // Tooltips are popups of their own, outside window.w11: their colours come from here (tokens.css
    // --bg-tooltip, --stroke-flyout, --shadow-tooltip; components.css .fl-slider__tip / the shell's tooltip).
    let (tip_bg, tip_fg, tip_stroke, tip_shadow) = if look.dark {
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

fn rgb(hex: &str) -> (u8, u8, u8) {
    let n = normalize_hex(hex).and_then(|h| u32::from_str_radix(&h[1..], 16).ok()).unwrap_or(0);
    (((n >> 16) & 255) as u8, ((n >> 8) & 255) as u8, (n & 255) as u8)
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn rgba((r, g, b): (u8, u8, u8), a: f64) -> String {
    format!("rgba({r}, {g}, {b}, {})", round2(a))
}

/// color-mix(in srgb, a p, b) for opaque colours.
fn mix(a: (u8, u8, u8), b: (u8, u8, u8), p: f64) -> (u8, u8, u8) {
    let m = |x: u8, y: u8| (x as f64 * p + y as f64 * (1.0 - p)).round().clamp(0.0, 255.0) as u8;
    (m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
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
