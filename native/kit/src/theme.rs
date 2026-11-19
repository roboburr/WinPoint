//! Look: the Win11 tokens and controls as GTK CSS (css/tokens.css, base.css, controls.css, frame.css), a
//! generated part for the user's choices (theme.js `apply`: the accent palette from theme.accent with Windows'
//! own blue, the text ramp scaled by the text scale, tooltips, the on/off shapes) and the app's own sheet
//! (`install_app_css`). Every toplevel gets the classes `w11`, `w11-dark` / `w11-light`,
//! `w11-transparent` / `w11-opaque`, `w11-hc` (high contrast) and `w11-reduce-motion`, kept up to date live; a
//! window with the class `w11-unstyled` is left alone.
//!
//! What GNOME sets for the whole session comes in as GNOME's own keys ([`System`], notes/claude/settings-parity.md
//! amendments 46 and 49): the text scale is `org.gnome.desktop.interface text-scaling-factor`. GTK 4.22 applies
//! that factor to text sized in points only, never to px, and the kit's type ramp is px, so the kit scales its ramp
//! by exactly GNOME's factor: once, the same as GNOME's own apps, and never settings.json's textScale on top (the
//! mirror keeps that setting equal to GNOME's key; it is only the fallback without the schema). The kit reads the
//! keys itself (GSettings) because GTK 4.22's Wayland backend takes its settings from the settings portal only (no
//! portal: gtk-xft-dpi -1, gtk-interface-contrast unsupported, as in the test harness); GTK's dpi is the next
//! source. GNOME's high contrast (`org.gnome.desktop.a11y.interface high-contrast`, GTK's `prefers-contrast: more`)
//! turns `w11-hc` on as accessibility.highContrast does, and GNOME's On/Off Shapes (`show-status-shapes`) show the I
//! and O shapes in the toggle switches.
//!
//! The tokens can be read from Rust for cairo drawing: `token("text-2")`, `token("chart-cpu")`, `accent_rgba()`.
use gtk::gdk;
use gtk::gio;
use gtk::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

const TOKENS: &str = include_str!("../css/tokens.css");
const SHEETS: &[&str] = &[
    include_str!("../css/tokens.css"),
    include_str!("../css/base.css"),
    include_str!("../css/controls.css"),
    include_str!("../css/frame.css"),
];

/// The user's look as theme.js applies it.
#[derive(Clone, PartialEq, Debug)]
pub struct Look {
    pub dark: bool,
    pub accent: String,
    pub transparency: bool,
    pub high_contrast: bool,
    pub reduce_motion: bool,
    /// The text scale in effect (module docs): GNOME's text-scaling-factor, else accessibility.textScale / 100;
    /// 0.5 ..= 3.0.
    pub text_scale: f64,
    /// GNOME's On/Off Shapes (org.gnome.desktop.a11y.interface show-status-shapes).
    pub status_shapes: bool,
}

impl Default for Look {
    fn default() -> Look {
        Look {
            dark: true,
            accent: "#0078d4".into(),
            transparency: true,
            high_contrast: false,
            reduce_motion: false,
            text_scale: 1.0,
            status_shapes: false,
        }
    }
}

/// GNOME's settings for the session (module docs): org.gnome.desktop.interface text-scaling-factor and
/// org.gnome.desktop.a11y.interface high-contrast and show-status-shapes, else what GTK carries of them.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct System {
    /// text-scaling-factor (0.5 ..= 3.0); None without the schema and without a dpi from GTK.
    pub text_scale: Option<f64>,
    /// high-contrast.
    pub high_contrast: bool,
    /// show-status-shapes.
    pub status_shapes: bool,
}

/// GNOME's two schemas, opened once (`install`) where they are installed; their changes apply the look again.
struct GnomeKeys {
    interface: Option<gio::Settings>,
    a11y: Option<gio::Settings>,
}

thread_local! {
    static GNOME_KEYS: RefCell<Option<GnomeKeys>> = const { RefCell::new(None) };
}

/// A schema's settings object when the schema and the key are installed.
fn settings_with(schema: &str, key: &str) -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let found = source.lookup(schema, true)?;
    found.has_key(key).then(|| gio::Settings::new(schema))
}

/// GNOME's text-scaling-factor as the kit uses it (finite, within the schema's 0.5 ..= 3.0).
fn factor(v: f64) -> Option<f64> {
    (v.is_finite() && v > 0.0).then(|| v.clamp(0.5, 3.0))
}

impl System {
    /// GNOME's text-scaling-factor from GTK's dpi (GTK's Wayland backend: 96 x 1024 x the factor), within the
    /// schema's 0.5 ..= 3.0; None for GTK's "no dpi".
    pub fn text_scale_of_dpi(dpi: i32) -> Option<f64> {
        (dpi > 0).then(|| (f64::from(dpi) / (96.0 * 1024.0)).clamp(0.5, 3.0))
    }

    /// GNOME's keys now (module docs), else GTK's settings (all off without either).
    pub fn read() -> System {
        let gtk_settings = gtk::Settings::default();
        let (interface, a11y) =
            GNOME_KEYS.with(|g| g.borrow().as_ref().map(|k| (k.interface.clone(), k.a11y.clone())).unwrap_or((None, None)));
        let text_scale = interface
            .as_ref()
            .and_then(|i| factor(i.double("text-scaling-factor")))
            .or_else(|| gtk_settings.as_ref().and_then(|s| System::text_scale_of_dpi(s.gtk_xft_dpi())));
        let (high_contrast, status_shapes) = match &a11y {
            Some(a) => (a.boolean("high-contrast"), a.boolean("show-status-shapes")),
            None => gtk_settings.as_ref().map_or((false, false), |s| {
                (s.gtk_interface_contrast() == gtk::InterfaceContrast::More, s.is_gtk_show_status_shapes())
            }),
        };
        System { text_scale, high_contrast, status_shapes }
    }
}

impl Look {
    /// The look of a settings object (settings.get), `system_dark` for theme.mode other than light/dark.
    pub fn from_settings(s: &serde_json::Value, system_dark: bool) -> Look {
        let dark = match (forced_dark(), s["theme"]["mode"].as_str()) {
            (Some(d), _) => d,
            (None, Some("light")) => false,
            (None, Some("dark")) => true,
            _ => system_dark,
        };
        let ts = s["accessibility"]["textScale"].as_f64().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(100.0);
        Look {
            dark,
            accent: s["theme"]["accent"].as_str().unwrap_or("#0078d4").to_owned(),
            transparency: s["theme"]["transparency"] != false,
            high_contrast: s["accessibility"]["highContrast"] == true,
            reduce_motion: s["accessibility"]["reduceMotion"] == true,
            text_scale: ts.clamp(50.0, 300.0) / 100.0,
            status_shapes: false,
        }
    }

    /// The look of a settings object with GNOME's session settings (module docs): GNOME's text scale replaces
    /// textScale (never both), GNOME's high contrast and status shapes join WinPoint's.
    pub fn from_settings_and_system(s: &serde_json::Value, system_dark: bool, sys: &System) -> Look {
        let mut look = Look::from_settings(s, system_dark);
        if let Some(ts) = sys.text_scale {
            look.text_scale = ts;
        }
        look.high_contrast |= sys.high_contrast;
        look.status_shapes = sys.status_shapes;
        look
    }
}

struct State {
    look: Look,
    palette: Palette,
    dynamic: Option<gtk::CssProvider>,
    app: Option<gtk::CssProvider>,
    last_css: String,
    toplevels: Option<gio::ListModel>,
    listeners: Vec<Rc<dyn Fn()>>,
    tracked: Vec<gtk::glib::WeakRef<gtk::Widget>>,
    tokens_dark: HashMap<String, String>,
    tokens_light: HashMap<String, String>,
    tokens_hc: HashMap<String, String>,
    dyn_tokens: HashMap<String, String>,
}

thread_local! {
    static FORCED_DARK: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    static ST: RefCell<State> = RefCell::new(State {
        look: Look::default(),
        palette: accent_palette("#0078d4"),
        dynamic: None,
        app: None,
        last_css: String::new(),
        toplevels: None,
        listeners: Vec::new(),
        tracked: Vec::new(),
        tokens_dark: parse_block(TOKENS, "window.w11 {"),
        tokens_light: parse_block(TOKENS, "window.w11.w11-light {"),
        tokens_hc: parse_block(TOKENS, "window.w11.w11-hc {"),
        dyn_tokens: HashMap::new(),
    });
}

/// Load the kit's sheets on the default display (kit::run does this at startup).
pub(crate) fn install() {
    let Some(display) = gdk::Display::default() else { return };
    let fixed = gtk::CssProvider::new();
    fixed.connect_parsing_error(|_, section, error| {
        eprintln!("win11-kit: css {}: {}", section.to_str(), error);
    });
    fixed.load_from_string(&SHEETS.join("\n"));
    gtk::style_context_add_provider_for_display(&display, &fixed, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
    let dynamic = gtk::CssProvider::new();
    dynamic.connect_parsing_error(|_, section, error| {
        eprintln!("win11-kit: theme css {}: {}", section.to_str(), error);
    });
    gtk::style_context_add_provider_for_display(&display, &dynamic, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1);
    let toplevels = gtk::Window::toplevels();
    toplevels.connect_items_changed(|model, position, _removed, added| {
        for i in position..position + added {
            if let Some(window) = model.item(i).and_downcast::<gtk::Window>() {
                style_window(&window);
            }
        }
    });
    ST.with(|s| {
        let mut s = s.borrow_mut();
        s.dynamic = Some(dynamic);
        s.toplevels = Some(toplevels);
    });
    // GNOME's session settings (System): a change of the keys, or of what GTK carries, applies the look again
    let again = || {
        if let Some(ctx) = crate::ctx::try_ctx() {
            ctx.apply_look();
        }
    };
    let keys = GnomeKeys {
        interface: settings_with("org.gnome.desktop.interface", "text-scaling-factor"),
        a11y: settings_with("org.gnome.desktop.a11y.interface", "show-status-shapes"),
    };
    if let Some(i) = &keys.interface {
        i.connect_changed(Some("text-scaling-factor"), move |_, _| again());
    }
    if let Some(a) = &keys.a11y {
        for key in ["high-contrast", "show-status-shapes"] {
            a.connect_changed(Some(key), move |_, _| again());
        }
    }
    GNOME_KEYS.with(|g| *g.borrow_mut() = Some(keys));
    if let Some(settings) = gtk::Settings::default() {
        settings.connect_gtk_xft_dpi_notify(move |_| again());
        settings.connect_gtk_interface_contrast_notify(move |_| again());
        settings.connect_gtk_show_status_shapes_notify(move |_| again());
    }
    let look = look();
    apply(&look, true);
}

/// Force dark (Some(true)) or light (Some(false)) whatever theme.mode says, like theme.js `?theme=` (screenshots,
/// the gallery's --light); None follows the settings again. WIN11_THEME=light|dark does this at startup.
pub fn set_forced_dark(dark: Option<bool>) {
    FORCED_DARK.with(|f| f.set(dark));
    if let Some(ctx) = crate::ctx::try_ctx() {
        ctx.apply_look();
    }
}

/// The forced mode (set_forced_dark or WIN11_THEME).
pub fn forced_dark() -> Option<bool> {
    FORCED_DARK.with(|f| f.get()).or_else(|| match std::env::var("WIN11_THEME").as_deref() {
        Ok("light") => Some(false),
        Ok("dark") => Some(true),
        _ => None,
    })
}

/// Add the app's own sheet (css/<kind>.css, `include_str!`) above the kit's. Calling it again replaces it.
pub fn install_app_css(css: &str) {
    let Some(display) = gdk::Display::default() else { return };
    let provider = ST.with(|s| s.borrow().app.clone()).unwrap_or_else(|| {
        let p = gtk::CssProvider::new();
        p.connect_parsing_error(|_, section, error| {
            eprintln!("win11-kit: app css {}: {}", section.to_str(), error);
        });
        gtk::style_context_add_provider_for_display(&display, &p, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2);
        ST.with(|s| s.borrow_mut().app = Some(p.clone()));
        p
    });
    provider.load_from_string(css);
}

/// Apply a look: the generated CSS, GTK's colour scheme and animations, the window classes; listeners run
/// when it changed (or `force`).
pub(crate) fn apply(look: &Look, force: bool) {
    let changed = ST.with(|s| {
        let mut s = s.borrow_mut();
        let changed = s.look != *look;
        if changed || s.palette.base.is_empty() || s.look.accent != look.accent {
            s.palette = accent_palette(&look.accent);
        }
        s.look = look.clone();
        changed
    });
    if let Some(settings) = gtk::Settings::default() {
        settings.set_gtk_interface_color_scheme(if look.dark {
            gtk::InterfaceColorScheme::Dark
        } else {
            gtk::InterfaceColorScheme::Light
        });
        settings.set_gtk_enable_animations(!look.reduce_motion);
        // GtkSwitch draws its I and O images only while GTK's own setting says so, and GTK takes it from the
        // settings portal alone: the kit sets it from GNOME's key (module docs), as it sets the animations
        if settings.is_gtk_show_status_shapes() != look.status_shapes {
            settings.set_gtk_show_status_shapes(look.status_shapes);
        }
    }
    let (css, dyn_tokens) = ST.with(|s| {
        let s = s.borrow();
        dynamic_css(&s.look, &s.palette)
    });
    let provider = ST.with(|s| {
        let mut s = s.borrow_mut();
        s.dyn_tokens = dyn_tokens;
        if s.last_css == css {
            None
        } else {
            s.last_css = css.clone();
            s.dynamic.clone()
        }
    });
    if let Some(p) = provider {
        p.load_from_string(&css);
    }
    for w in gtk::Window::list_toplevels() {
        if let Ok(w) = w.downcast::<gtk::Window>() {
            style_window(&w);
        }
    }
    if changed || force {
        crate::icons::invalidate();
        let tracked: Vec<gtk::Widget> = ST.with(|s| {
            let mut s = s.borrow_mut();
            s.tracked.retain(|w| w.upgrade().is_some());
            s.tracked.iter().filter_map(|w| w.upgrade()).collect()
        });
        for w in tracked {
            w.queue_draw();
        }
        let listeners: Vec<Rc<dyn Fn()>> = ST.with(|s| s.borrow().listeners.clone());
        for f in listeners {
            f();
        }
    }
}

/// Run `f` after every change of the look (dark/light, accent, contrast, text scale): redraw cairo widgets.
pub fn on_change(f: impl Fn() + 'static) {
    ST.with(|s| s.borrow_mut().listeners.push(Rc::new(f)));
}

/// Redraw this widget after every change of the look (cairo drawing with `token()` colours); held weakly.
pub fn track(widget: &impl IsA<gtk::Widget>) {
    let w = widget.as_ref().downgrade();
    ST.with(|s| {
        let mut s = s.borrow_mut();
        if s.tracked.len() > 256 && s.tracked.len().is_power_of_two() {
            s.tracked.retain(|w| w.upgrade().is_some());
        }
        s.tracked.push(w);
    });
}

/// The look in effect.
pub fn look() -> Look {
    ST.with(|s| s.borrow().look.clone())
}

/// Whether the dark theme is in effect.
pub fn dark() -> bool {
    ST.with(|s| s.borrow().look.dark)
}

/// The accent palette in effect.
pub fn palette() -> Palette {
    ST.with(|s| s.borrow().palette.clone())
}

/// Give a window the theme's classes now (new toplevels and every change do this by themselves).
pub fn style_window(window: &gtk::Window) {
    if window.has_css_class("w11-unstyled") {
        return;
    }
    let look = look();
    window.add_css_class("w11");
    toggle(window, "w11-dark", look.dark);
    toggle(window, "w11-light", !look.dark);
    toggle(window, "w11-transparent", look.transparency);
    toggle(window, "w11-opaque", !look.transparency);
    toggle(window, "w11-hc", look.high_contrast);
    toggle(window, "w11-reduce-motion", look.reduce_motion);
}

fn toggle(window: &gtk::Window, class: &str, on: bool) {
    if on {
        window.add_css_class(class);
    } else {
        window.remove_css_class(class);
    }
}

/// A token of tokens.css (with or without the leading "--") in the theme in effect, as RGBA: "text-1",
/// "accent", "stroke-divider", "chart-cpu", "sys-critical", ... Unknown or not a colour: transparent.
pub fn token(name: &str) -> gdk::RGBA {
    token_str(name).and_then(|v| gdk::RGBA::parse(v.as_str()).ok()).unwrap_or(gdk::RGBA::new(0.0, 0.0, 0.0, 0.0))
}

/// A token's CSS value text in the theme in effect (var() resolved).
pub fn token_str(name: &str) -> Option<String> {
    let mut key = name.trim().trim_start_matches("--").to_owned();
    for _ in 0..8 {
        let v = ST.with(|s| {
            let s = s.borrow();
            let v = s.dyn_tokens.get(&key).cloned();
            let v = v.or_else(|| if s.look.high_contrast { s.tokens_hc.get(&key).cloned() } else { None });
            let v = v.or_else(|| if s.look.dark { None } else { s.tokens_light.get(&key).cloned() });
            v.or_else(|| s.tokens_dark.get(&key).cloned())
        })?;
        match v.trim().strip_prefix("var(--").and_then(|r| r.strip_suffix(')')) {
            Some(next) => key = next.to_owned(),
            None => return Some(v),
        }
    }
    None
}

/// The accent fill colour in effect (light-2 in dark mode, dark-1 in light mode).
pub fn accent_rgba() -> gdk::RGBA {
    let p = palette();
    let hex = if dark() { p.light2 } else { p.dark1 };
    gdk::RGBA::parse(hex.as_str()).unwrap_or(gdk::RGBA::new(0.0, 0.47, 0.83, 1.0))
}

/// A hex or CSS colour as RGBA (transparent when it does not parse).
pub fn rgba(color: &str) -> gdk::RGBA {
    gdk::RGBA::parse(color).unwrap_or(gdk::RGBA::new(0.0, 0.0, 0.0, 0.0))
}

fn parse_block(css: &str, head: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(start) = css.find(head) else { return out };
    let body = &css[start + head.len()..];
    let end = body.find("\n}").unwrap_or(body.len());
    let mut in_comment = false;
    for line in body[..end].lines() {
        let line = line.trim();
        if in_comment {
            if line.contains("*/") {
                in_comment = false;
            }
            continue;
        }
        if line.starts_with("/*") {
            in_comment = !line.contains("*/");
            continue;
        }
        if let Some(rest) = line.strip_prefix("--")
            && let Some((k, v)) = rest.split_once(':')
        {
            out.insert(k.trim().to_owned(), v.trim().trim_end_matches(';').trim().to_owned());
        }
    }
    out
}

/// GNOME's On/Off Shapes in the kit's switches: GtkSwitch's own I (on) and O (off) images, which controls.css
/// hides otherwise; the slider covers the image of the other state.
const STATUS_SHAPES_CSS: &str = "switch.w11-switch > image { color: var(--text-2); opacity: 1; -gtk-icon-size: 10px; }\n\
    switch.w11-switch:checked > image { color: var(--accent-text); }\n\
    switch.w11-switch:disabled > image { color: var(--text-disabled); }\n";

fn dynamic_css(look: &Look, p: &Palette) -> (String, HashMap<String, String>) {
    let dark = look.dark;
    let (accent, accent_text) = if dark { (&p.light2, &p.on_dark) } else { (&p.dark1, &p.on_light) };
    let a = rgb(accent);
    let at = rgb(accent_text);
    let fg = |d: &String, l: &String| if dark { d.clone() } else { l.clone() };
    let mut vars: Vec<(String, String)> = vec![
        ("accent-base".into(), p.base.clone()),
        ("accent-light-1".into(), p.light1.clone()),
        ("accent-light-2".into(), p.light2.clone()),
        ("accent-light-3".into(), p.light3.clone()),
        ("accent-dark-1".into(), p.dark1.clone()),
        ("accent-dark-2".into(), p.dark2.clone()),
        ("accent-dark-3".into(), p.dark3.clone()),
        ("accent-on-light".into(), p.on_light.clone()),
        ("accent-on-dark".into(), p.on_dark.clone()),
        ("accent".into(), accent.clone()),
        ("accent-2".into(), rgba_text(a, 0.9)),
        ("accent-3".into(), rgba_text(a, 0.8)),
        ("accent-text".into(), accent_text.clone()),
        ("accent-text-2".into(), rgba_text(at, if dark { 0.5 } else { 0.7 })),
        ("accent-fg".into(), fg(&p.light3, &p.dark2)),
        ("accent-fg-2".into(), fg(&p.light3, &p.dark3)),
        ("accent-fg-3".into(), fg(&p.light2, &p.dark1)),
        ("accent-subtle".into(), rgba_text(a, if dark { 0.14 } else { 0.12 })),
        ("selection-bg".into(), accent.clone()),
        ("selection-text".into(), if dark { "#000000".into() } else { "#ffffff".into() }),
        ("selection-subtle".into(), rgba_text(a, if dark { 0.22 } else { 0.18 })),
        ("sys-attention".into(), accent.clone()),
        ("ring".into(), if dark { "#ffffff".into() } else { "rgba(0, 0, 0, 0.8956)".into() }),
    ];
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
        vars.push((name.into(), format!("{}px", round2(px * ts))));
    }
    if !look.transparency || look.high_contrast {
        vars.push(("bg-mica-glass".into(), if dark { "#202020".into() } else { "#f3f3f3".into() }));
    }
    let mut css = String::from("window.w11 {\n");
    for (k, v) in &vars {
        css.push_str(&format!("  --{k}: {v};\n"));
    }
    css.push_str("}\n");
    if look.status_shapes {
        css.push_str(STATUS_SHAPES_CSS);
    }
    // Tooltips are popups of their own (no window.w11 around them): their colours come from here (tokens.css
    // --bg-tooltip-solid, --stroke-flyout, --shadow-tooltip; components.css .fl-tooltip).
    let (tip_bg, tip_fg, tip_stroke, tip_shadow) = if dark {
        ("#2c2c2c", "#ffffff", "rgba(0, 0, 0, 0.2)", "rgba(0, 0, 0, 0.26)")
    } else {
        ("#f9f9f9", "rgba(0, 0, 0, 0.8956)", "rgba(0, 0, 0, 0.0578)", "rgba(0, 0, 0, 0.14)")
    };
    css.push_str(&format!(
        "tooltip, tooltip.background {{\n  background-color: {tip_bg};\n  background-image: none;\n  color: {tip_fg};\n  \
         border: 1px solid {tip_stroke};\n  border-radius: 4px;\n  padding: 5px 8px 7px 8px;\n  box-shadow: 0 4px 8px {tip_shadow};\n  \
         text-shadow: none;\n  font-family: \"Segoe UI Variable Text\", \"Segoe UI Variable\", \"Segoe UI\", \"Selawik\", \"Inter\", \"Noto Sans\", \"Ubuntu Sans\", sans-serif;\n  \
         font-size: {}px;\n}}\ntooltip > box {{ border-spacing: 6px; }}\ntooltip label {{ color: {tip_fg}; }}\n",
        round2(12.0 * ts)
    ));
    (css, vars.into_iter().collect())
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

/// theme.js accentPalette.
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
        let up = |k: f64| l + (0.95 - l) * k;
        Palette {
            base: hex_in.clone(),
            light1: mk(up(0.2).max(0.6), 1.02),
            light2: mk(up(0.55).max(0.72), 0.86),
            light3: mk(up(0.85).max(0.86), 0.55),
            dark1: mk((l * 0.91).min(0.6), 0.97),
            dark2: mk((l * 0.68).min(0.47), 0.9),
            dark3: mk((l * 0.475).min(0.34), 0.83),
            on_light: String::new(),
            on_dark: String::new(),
        }
    };
    p.on_light = text_on(&p.dark1, "#ffffff");
    p.on_dark = text_on(&p.light2, "#000000");
    p
}

/// "#abc", "abc", "#aabbcc", "#aabbccdd" -> "#aabbcc".
pub fn normalize_hex(input: &str) -> Option<String> {
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

/// A hex colour's channels (black when it does not parse).
pub fn rgb(hex: &str) -> (u8, u8, u8) {
    let n = normalize_hex(hex).and_then(|h| u32::from_str_radix(&h[1..], 16).ok()).unwrap_or(0);
    (((n >> 16) & 255) as u8, ((n >> 8) & 255) as u8, (n & 255) as u8)
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn rgba_text((r, g, b): (u8, u8, u8), a: f64) -> String {
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

/// WCAG contrast of two hex colours.
pub fn contrast(a: &str, b: &str) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// Text colour (#ffffff or #000000) for a fill; `prefer` wins when it is readable enough (theme.js textOn).
pub fn text_on(fill: &str, prefer: &str) -> String {
    let other = if prefer == "#ffffff" { "#000000" } else { "#ffffff" };
    if contrast(prefer, fill) >= 3.5 {
        return prefer.to_owned();
    }
    if contrast(other, fill) > contrast(prefer, fill) { other.to_owned() } else { prefer.to_owned() }
}

/// The Windows accent colour grid (Instellingen > Persoonlijke instellingen > Kleuren; theme.js ACCENT_PRESETS).
pub const ACCENT_PRESETS: [&str; 48] = [
    "#FFB900", "#FF8C00", "#F7630C", "#CA5010", "#DA3B01", "#EF6950", "#D13438", "#FF4343", "#E74856", "#E81123",
    "#EA005E", "#C30052", "#E3008C", "#BF0077", "#C239B3", "#9A0089", "#0078D4", "#0063B1", "#8E8CD8", "#6B69D6",
    "#8764B8", "#744DA9", "#B146C2", "#881798", "#0099BC", "#2D7D9A", "#00B7C3", "#038387", "#00B294", "#018574",
    "#00CC6A", "#10893E", "#7A7574", "#5D5A58", "#68768A", "#515C6B", "#567C73", "#486860", "#498205", "#107C10",
    "#767676", "#4C4A48", "#69797E", "#4A5459", "#647C64", "#525E54", "#847545", "#7E735F",
];

#[cfg(test)]
mod tests {
    //! The glass of an own app window (D-G60, "alles behalve de taakbalk mag op 90% opacity"): the harness sees only
    //! the class (tests/gnome/scenarios/frosted-glass.py B), so the 90 % itself is checked here.
    use super::*;

    #[test]
    fn the_glass_mica_is_90_percent_over_the_blur_and_opaque_in_high_contrast() {
        let glass = |head: &str| parse_block(TOKENS, head).get("bg-mica-glass").cloned();
        assert_eq!(glass("window.w11 {").as_deref(), Some("rgba(32, 32, 32, 0.9)"));
        assert_eq!(glass("window.w11.w11-light {").as_deref(), Some("rgba(243, 243, 243, 0.9)"));
        assert_eq!(glass("window.w11.w11-hc {").as_deref(), Some("var(--bg-mica-solid)"));
        let rule = "window.w11-app-window.w11-glass .w11-frame { background-color: var(--bg-mica-glass); }";
        assert!(include_str!("../css/frame.css").lines().any(|l| l.trim() == rule), "frame.css: {rule}");
        // transparency off: the solid Mica again (dynamic_css overrides the token)
        let look = Look { transparency: false, ..Look::default() };
        let (_, vars) = dynamic_css(&look, &accent_palette(&look.accent));
        assert_eq!(vars.get("bg-mica-glass").map(String::as_str), Some("#202020"));
        let (_, vars) = dynamic_css(&Look::default(), &accent_palette("#0078d4"));
        assert_eq!(vars.get("bg-mica-glass"), None, "with transparency the tokens' 90 % stays");
    }

    #[test]
    fn the_text_scale_is_gtks_factor_once_and_settings_json_only_without_it() {
        // settings parity, amendment 49: GNOME's text-scaling-factor, else GTK's dpi (96 x 1024 at 1.0)
        assert_eq!(factor(1.25), Some(1.25));
        assert_eq!(factor(8.0), Some(3.0), "within the schema's range");
        assert_eq!(factor(f64::NAN), None);
        assert_eq!(System::text_scale_of_dpi(96 * 1024), Some(1.0));
        assert_eq!(System::text_scale_of_dpi(147_456), Some(1.5));
        assert_eq!(System::text_scale_of_dpi(-1), None);
        assert_eq!(System::text_scale_of_dpi(10), Some(0.5), "within the schema's range");
        let s = serde_json::json!({"accessibility": {"textScale": 125, "highContrast": false}});
        let sys = System { text_scale: Some(1.5), ..System::default() };
        let look = Look::from_settings_and_system(&s, true, &sys);
        assert_eq!(look.text_scale, 1.5, "GNOME's factor, not 1.25 x 1.5");
        let (_, vars) = dynamic_css(&look, &accent_palette(&look.accent));
        assert_eq!(vars.get("fs-body").map(String::as_str), Some("21px"), "14 px x 1.5, once");
        // neither GNOME's key nor a dpi from GTK: settings.json's textScale, as before (now within GNOME's 50-300 %)
        let look = Look::from_settings_and_system(&s, true, &System::default());
        assert_eq!(look.text_scale, 1.25);
        let wide = serde_json::json!({"accessibility": {"textScale": 300}});
        assert_eq!(Look::from_settings(&wide, true).text_scale, 3.0);
    }

    #[test]
    fn gnome_high_contrast_and_status_shapes_reach_the_look() {
        let s = serde_json::json!({"accessibility": {"highContrast": false}});
        let sys = System { high_contrast: true, status_shapes: true, ..System::default() };
        let look = Look::from_settings_and_system(&s, true, &sys);
        assert!(look.high_contrast && look.status_shapes);
        let (css, _) = dynamic_css(&look, &accent_palette(&look.accent));
        assert!(css.contains("switch.w11-switch > image { color: var(--text-2); opacity: 1;"));
        let on = serde_json::json!({"accessibility": {"highContrast": true}});
        assert!(Look::from_settings_and_system(&on, true, &System::default()).high_contrast);
        let (css, _) = dynamic_css(&Look::default(), &accent_palette("#0078d4"));
        assert!(!css.contains("switch.w11-switch > image"), "no shapes unless GNOME asks for them");
    }
}
