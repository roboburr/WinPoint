//! The helpers every page uses (last-webkit:vendor/ui/js/apps/settings/common.js), on the kit's controls: shared texts,
//! error and dry-run toasts, controls bound to a settings path, link cards, notices, "not available" cards,
//! colour names and maths, the wallpaper (path, twin, texture), the hero row and its tiles, the usage bar,
//! the accent colour from the wallpaper, recently opened pages, and small layout helpers.
use crate::pages::Page;
use crate::widgets::Wrap;
use kit::CallError;
use kit::controls::{self as c, Dropdown, DropdownOption, InfoBar, SettingsCard, Severity, Toast, ToggleSwitch};
use gtk::gdk_pixbuf;
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

// ------------------------------------------------------------------------------------------- texts (ct)

/// The shared texts of common.js `ct`.
pub mod ct {
    use kit::t;
    pub fn failed() -> &'static str {
        t("Dat lukte niet", "That didn’t work")
    }
    pub fn unavailable() -> &'static str {
        t("Niet beschikbaar", "Not available")
    }
    pub fn unavailable_here() -> &'static str {
        t("Dit is op deze computer niet beschikbaar.", "This isn’t available on this computer.")
    }
    pub fn loading() -> &'static str {
        t("Even geduld…", "Just a moment…")
    }
    pub fn retry() -> &'static str {
        t("Opnieuw proberen", "Try again")
    }
    pub fn related() -> &'static str {
        t("Gerelateerde instellingen", "Related settings")
    }
    pub fn saved() -> &'static str {
        t("Opgeslagen", "Saved")
    }
    pub fn dry_run() -> &'static str {
        t("Testmodus: er is niets echt gewijzigd.", "Test mode: nothing was actually changed.")
    }
    pub fn copied() -> &'static str {
        t("Gekopieerd naar het klembord", "Copied to the clipboard")
    }
    pub fn copy_failed() -> &'static str {
        t("Kopiëren lukte niet", "Couldn’t copy")
    }
    pub fn on() -> &'static str {
        t("Aan", "On")
    }
    pub fn off() -> &'static str {
        t("Uit", "Off")
    }
}

/// "{f} vrij van {t}" with the values filled in.
pub fn fill(template: &str, vars: &[(&str, &str)]) -> String {
    let mut s = template.to_owned();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

/// The one / other form for `n`, with {n} filled in (makeT plurals).
pub fn plural(n: i64, one: &str, other: &str) -> String {
    (if n == 1 { one } else { other }).replace("{n}", &n.to_string())
}

// ------------------------------------------------------------------------------------------- feedback

/// common.js showError: an error toast with the host's message ("Dit is op deze computer niet beschikbaar."
/// for an unknown method), 6 s.
pub fn show_error(anchor: &impl IsA<gtk::Widget>, err: &CallError, title: Option<&str>) {
    let message = if err.code == "unknown_method" { ct::unavailable_here().to_owned() } else { err.message.clone() };
    show_error_text(anchor, &message, title);
}

/// An error toast with this message.
pub fn show_error_text(anchor: &impl IsA<gtk::Widget>, message: &str, title: Option<&str>) {
    let _ = Toast::new(title.unwrap_or(ct::failed())).message(message).severity(Severity::Error).timeout(6000).show(anchor);
}

/// common.js noteDryRun: the "Testmodus" toast after a dangerous call that ran with --dry-run.
pub fn note_dry_run(anchor: &impl IsA<gtk::Widget>, result: &Value) {
    if result["dryRun"] == true {
        let _ = Toast::new(ct::dry_run()).severity(Severity::Info).timeout(3500).show(anchor);
    }
}

/// A success toast.
pub fn toast_success(anchor: &impl IsA<gtk::Widget>, title: &str, ms: u32) {
    let _ = Toast::new(title).severity(Severity::Success).timeout(ms).show(anchor);
}

// ------------------------------------------------------------------------------------------- settings

/// A setting as a bool (`default` when absent; JS `!!getSetting(path, default)`).
pub fn setting_bool(path: &str, default: bool) -> bool {
    match kit::ctx().setting(path) {
        Value::Null => default,
        Value::Bool(b) => b,
        Value::Number(n) => n.as_f64().is_some_and(|v| v != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// A setting as text (numbers too).
pub fn setting_str(path: &str) -> Option<String> {
    match kit::ctx().setting(path) {
        Value::String(s) => Some(s),
        Value::Null => None,
        v => Some(v.to_string()),
    }
}

/// A setting as a number.
pub fn setting_f64(path: &str) -> Option<f64> {
    kit::ctx().setting(path).as_f64()
}

/// setSetting with common.js's error handling: optimistic, reverted on error with an error toast;
/// `after(ok)` runs once the host answered.
pub fn set_setting(p: &Page, path: &str, value: Value, after: impl FnOnce(bool) + 'static) {
    let anchor = p.anchor();
    kit::ctx().set_setting_then(path, value, move |r| match r {
        Ok(_) => after(true),
        Err(e) => {
            show_error(&anchor, &e, None);
            after(false);
        }
    });
}

/// A toggle switch bound to a settings path (common.js settingToggle; `invert`: the switch shows !value).
pub fn setting_toggle(p: &Page, path: &str, invert: bool) -> ToggleSwitch {
    let read = {
        let path = path.to_owned();
        move || setting_bool(&path, false) != invert
    };
    let sw = ToggleSwitch::new(read());
    let (pp, path_s, r) = (p.clone(), path.to_owned(), read.clone());
    let sw2 = sw.clone();
    sw.connect_toggled(move |on| {
        let (sw, r) = (sw2.clone(), r.clone());
        set_setting(&pp, &path_s, json!(on != invert), move |ok| {
            if !ok {
                sw.set_active(r());
            }
        });
    });
    let sw2 = sw.clone();
    p.on_settings(move |_, _| sw2.set_active(read()));
    sw
}

/// The text key of a JSON option value (strings as they are, numbers and booleans as JSON).
pub fn value_key(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A drop-down bound to a settings path (common.js settingDropdown); `options` are (value, label) with the
/// value as JSON (a string or a number, written back as such); `fallback` when the setting is absent (else the
/// first option).
pub fn setting_dropdown(p: &Page, path: &str, options: Vec<(Value, String)>, fallback: Option<Value>) -> Dropdown {
    let first = options.first().map(|(v, _)| v.clone()).unwrap_or(Value::Null);
    let read = {
        let path = path.to_owned();
        let fb = fallback.unwrap_or(first);
        move || {
            let v = kit::ctx().setting(&path);
            value_key(if v.is_null() { &fb } else { &v })
        }
    };
    let dd = Dropdown::new(options.iter().map(|(v, l)| DropdownOption::new(&value_key(v), l)).collect(), Some(&read()));
    let values: Rc<HashMap<String, Value>> = Rc::new(options.iter().map(|(v, _)| (value_key(v), v.clone())).collect());
    let (pp, path_s, r, dd2) = (p.clone(), path.to_owned(), read.clone(), dd.clone());
    dd.connect_changed(move |key| {
        let value = values.get(key).cloned().unwrap_or_else(|| json!(key));
        let (dd, r) = (dd2.clone(), r.clone());
        set_setting(&pp, &path_s, value, move |ok| {
            if !ok {
                dd.set_selected(Some(&r()));
            }
        });
    });
    let dd2 = dd.clone();
    p.on_settings(move |_, _| dd2.set_selected(Some(&read())));
    dd
}

/// A settings card with a toggle bound to a settings path (common.js toggleCard).
pub fn toggle_card(p: &Page, icon: &str, title: &str, description: &str, path: &str, invert: bool) -> (SettingsCard, ToggleSwitch) {
    let sw = setting_toggle(p, path, invert);
    let card = SettingsCard::new(icon, title, description);
    card.set_control(sw.widget());
    (card, sw)
}

/// A settings card with a drop-down bound to a settings path (common.js dropdownCard).
pub fn dropdown_card(
    p: &Page,
    icon: &str,
    title: &str,
    description: &str,
    path: &str,
    options: Vec<(Value, String)>,
    fallback: Option<Value>,
) -> (SettingsCard, Dropdown) {
    let dd = setting_dropdown(p, path, options, fallback);
    let card = SettingsCard::new(icon, title, description);
    card.set_control(dd.widget());
    (card, dd)
}

/// The clickable card that opens another page, with the registry's glyph, title and description
/// (common.js linkCard).
pub fn link_card(p: &Page, id: &str) -> SettingsCard {
    link_card_with(p, id, None, None, None)
}

/// `link_card` with an own title, description or glyph.
pub fn link_card_with(p: &Page, id: &str, title: Option<&str>, description: Option<&str>, icon: Option<&str>) -> SettingsCard {
    let info = crate::registry::page(id);
    let card = SettingsCard::clickable(
        icon.or(info.map(|i| i.icon)).unwrap_or(""),
        title.unwrap_or_else(|| info.map(|i| i.title()).unwrap_or(id)),
        description.unwrap_or_else(|| info.map(|i| i.description()).unwrap_or("")),
        c::Chevron::Right,
    );
    if let Some(b) = card.button() {
        b.set_widget_name(&format!("link-{id}"));
    }
    let (pp, id) = (p.clone(), id.to_owned());
    card.connect_clicked(move || pp.go(&id));
    card
}

/// A non-closable info bar explaining why something is not available (common.js notice).
pub fn notice(message: &str) -> InfoBar {
    notice_full(Severity::Info, "", message)
}

/// `notice` with a severity and a title.
pub fn notice_full(severity: Severity, title: &str, message: &str) -> InfoBar {
    let bar = InfoBar::new(severity, title, message);
    bar.set_closable(false);
    bar.widget().add_css_class("st-notice");
    bar
}

/// A disabled-looking card that says "Niet beschikbaar" (common.js unavailableCard).
pub fn unavailable_card(icon: &str, title: &str, description: Option<&str>) -> SettingsCard {
    let card = SettingsCard::new(icon, title, description.unwrap_or(ct::unavailable_here()));
    let l = c::label(ct::unavailable(), "w11-text-3", false);
    l.set_ellipsize(gtk::pango::EllipsizeMode::None);
    card.set_control(&l);
    card.widget().add_css_class("st-unavailable");
    card
}

/// A control at a fixed width (CSS `width`): `px` wide whatever its text needs; longer text is cut.
pub fn sized(w: &impl IsA<gtk::Widget>, px: i32) -> crate::widgets::Clamp {
    w.as_ref().set_size_request(px, -1);
    let c = crate::widgets::Clamp::new(w, px);
    c.set_halign(gtk::Align::End);
    c.set_valign(gtk::Align::Center);
    c
}

/// The grey value text on the right of a card (`.st-value`).
pub fn value_label(text: &str) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.add_css_class("st-value");
    l
}

/// The box with a card's title and description; extra content (a usage bar) can go below them.
pub fn card_text_box(card: &SettingsCard) -> Option<gtk::Box> {
    let root = card.widget();
    let inner = match root.downcast_ref::<gtk::Button>() {
        Some(b) => b.child(),
        None => root.first_child(),
    }?;
    inner.first_child()?.next_sibling().and_downcast::<gtk::Box>()
}

/// Put a widget under a card's title and description (a description that is more than text).
pub fn card_extra(card: &SettingsCard, w: &impl IsA<gtk::Widget>) {
    if let Some(text) = card_text_box(card) {
        text.append(w.as_ref());
    }
}

/// A slot (`.st-slot`): a column a page fills later; hidden while empty.
pub fn slot() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 22);
    b.add_css_class("st-slot");
    b.set_visible(false);
    b
}

/// Replace a slot's content.
pub fn set_slot(slot: &gtk::Box, widgets: &[gtk::Widget]) {
    c::clear(slot);
    for w in widgets {
        slot.append(w);
    }
    slot.set_visible(!widgets.is_empty());
    // a slot's blocks sit in the page's 22 px column (display: contents); a group after a group 28 px more
    let mut prev_group = false;
    for w in widgets {
        let group = w.has_css_class("w11-settings-group");
        w.set_margin_top(if group && prev_group { 28 } else { 0 });
        if w.is_visible() {
            prev_group = group;
        }
    }
}

/// settings.css spacing of a column of blocks (22 px apart): a group with a title 4 px more, an untitled group
/// after a group or slot 6 px more, a slot after a group 6 px more; the first block none.
pub fn space(column: &gtk::Box) {
    let mut prev: Option<gtk::Widget> = None;
    let mut child = column.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        let after_block = prev.as_ref().is_some_and(|p| p.has_css_class("w11-settings-group") || p.has_css_class("st-slot"));
        let top = match &prev {
            None => 0,
            Some(_) if w.has_css_class("w11-settings-group") => {
                if w.first_child().is_some_and(|t| t.is_visible()) {
                    4
                } else if after_block {
                    6
                } else {
                    0
                }
            }
            Some(p) if w.has_css_class("st-slot") && p.has_css_class("w11-settings-group") => 6,
            Some(_) => 0,
        };
        w.set_margin_top(top);
        if w.is_visible() {
            prev = Some(w);
        }
    }
}

/// A section header like Win11's "Gerelateerde instellingen" (common.js sectionTitle).
pub fn section_title(text: &str) -> gtk::Label {
    c::label(text, "st-section-title", true)
}

/// Keyboard keys as little key caps (`.st-kbd`).
pub fn kbd(keys: &[&str]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 2);
    b.add_css_class("st-keys");
    for k in keys {
        let l = gtk::Label::new(Some(k));
        l.add_css_class("st-kbd");
        b.append(&l);
    }
    b
}

// ------------------------------------------------------------------------------------------- colours

/// "Donker blauw" instead of a hex code (common.js colorName).
pub fn color_name(hex: &str) -> String {
    let Some(n) = kit::theme::normalize_hex(hex) else { return String::new() };
    let en = kit::i18n::en();
    let name = |k: &str| -> &'static str {
        match (k, en) {
            ("gray", false) => "Grijs",
            ("gray", true) => "Grey",
            ("red", false) => "Rood",
            ("red", true) => "Red",
            ("orange", false) => "Oranje",
            ("orange", true) => "Orange",
            ("yellow", false) => "Geel",
            ("yellow", true) => "Yellow",
            ("green", false) => "Groen",
            ("green", true) => "Green",
            ("teal", false) => "Groenblauw",
            ("teal", true) => "Teal",
            ("blue", false) => "Blauw",
            ("blue", true) => "Blue",
            ("purple", false) => "Paars",
            ("purple", true) => "Purple",
            ("pink", false) => "Roze",
            ("pink", true) => "Pink",
            ("brown", false) => "Bruin",
            ("brown", true) => "Brown",
            ("black", false) => "Zwart",
            ("black", true) => "Black",
            ("white", false) => "Wit",
            ("white", true) => "White",
            ("dark", false) => "Donker",
            ("dark", true) => "Dark",
            ("light", false) => "Licht",
            _ => "Light",
        }
    };
    let (r, g, b) = kit::theme::rgb(&n);
    let (r, g, b) = (r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let l = (max + min) / 2.0;
    let s = if d > 0.0 { d / (1.0 - (2.0 * l - 1.0).abs()) } else { 0.0 };
    if l < 0.1 {
        return name("black").into();
    }
    if l > 0.94 {
        return name("white").into();
    }
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
    let base = if s < 0.15 {
        "gray"
    } else if !(15.0..345.0).contains(&hue) {
        "red"
    } else if hue < 40.0 {
        if l < 0.42 { "brown" } else { "orange" }
    } else if hue < 65.0 {
        if l < 0.4 { "brown" } else { "yellow" }
    } else if hue < 160.0 {
        "green"
    } else if hue < 195.0 {
        "teal"
    } else if hue < 255.0 {
        "blue"
    } else if hue < 295.0 {
        "purple"
    } else {
        "pink"
    };
    let modifier = if l < 0.3 && base != "brown" {
        Some(name("dark"))
    } else if l > 0.72 {
        Some(name("light"))
    } else {
        None
    };
    match modifier {
        Some(m) => format!("{m} {}", name(base).to_lowercase()),
        None => name(base).into(),
    }
}

fn to_lin(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_lin(c: f64) -> f64 {
    255.0 * if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

/// theme.js hexToOklch: (L, C, H).
pub fn hex_to_oklch(hex: &str) -> (f64, f64, f64) {
    let (r, g, b) = kit::theme::rgb(&kit::theme::normalize_hex(hex).unwrap_or_else(|| "#000000".into()));
    let (r, g, b) = (to_lin(r as f64), to_lin(g as f64), to_lin(b as f64));
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ll = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    (ll, a.hypot(bb), (bb.atan2(a).to_degrees() + 360.0) % 360.0)
}

fn oklch_to_lin(l: f64, c: f64, h: f64) -> [f64; 3] {
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

/// theme.js oklchToHex: chroma reduced until the colour fits in sRGB.
pub fn oklch_to_hex(l: f64, c: f64, h: f64) -> String {
    let l = l.clamp(0.0, 1.0);
    let in_gamut = |v: &[f64; 3]| v.iter().all(|x| (-0.0005..=1.0005).contains(x));
    let mut rgb = oklch_to_lin(l, c, h);
    if !in_gamut(&rgb) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..22 {
            let mid = (lo + hi) / 2.0;
            if in_gamut(&oklch_to_lin(l, mid, h)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        rgb = oklch_to_lin(l, lo, h);
    }
    let ch = |v: f64| from_lin(v.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8;
    format!("#{:02x}{:02x}{:02x}", ch(rgb[0]), ch(rgb[1]), ch(rgb[2]))
}

/// common.js accentFor: the accent's fill and text colour in a mode (dark: light-2 / onDark).
pub fn accent_for(hex: &str, dark: bool) -> (String, String) {
    let p = kit::theme::accent_palette(hex);
    if dark { (p.light2, p.on_dark) } else { (p.dark1, p.on_light) }
}

// ------------------------------------------------------------------------------------------- text

/// Copy text to the clipboard (common.js copyText).
pub fn copy_text(text: &str) -> bool {
    match gdk::Display::default() {
        Some(d) => {
            d.clipboard().set_text(text);
            true
        }
        None => false,
    }
}

/// "Ubuntu 26.04.1 LTS" and "26.04.1 LTS (Resolute Raccoon)" -> "Ubuntu 26.04.1 LTS (Resolute Raccoon)"
/// (common.js osLabel).
pub fn os_label(os: &str, ver: &str) -> String {
    if os.is_empty() {
        return ver.to_owned();
    }
    if ver.is_empty() {
        return os.to_owned();
    }
    let v = ver.split(' ').next().unwrap_or("");
    if os.contains(v) {
        let rest = ver[v.len()..].trim();
        let rest = rest.strip_prefix("LTS").map(str::trim_start).unwrap_or(rest);
        return if !rest.is_empty() && !os.contains(rest) { format!("{os} {rest}") } else { os.to_owned() };
    }
    format!("{os} {ver}")
}

/// Percent-encode for a URL query value.
pub fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A value of a JSON object as text ("" when absent).
pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

// ------------------------------------------------------------------------------------------- wallpaper

/// "builtin:bloom-dark" -> "builtin:lotus-dark" (a renamed built-in wallpaper, bridge.js builtinWallpaper).
pub fn builtin_alias(path: &str) -> String {
    match path.strip_prefix("builtin:") {
        Some("bloom-dark") => "builtin:lotus-dark".into(),
        Some("bloom-light") => "builtin:lotus-light".into(),
        _ => path.to_owned(),
    }
}

/// common.js twinFor: the lotus- / flow- twin of a built-in wallpaper for a mode (followTheme).
pub fn twin_for(path: &str, dark: bool, follow: bool) -> String {
    let path = builtin_alias(path);
    if !follow {
        return path;
    }
    if let Some(id) = path.strip_prefix("builtin:") {
        for base in ["lotus", "flow"] {
            if id == format!("{base}-light") || id == format!("{base}-dark") {
                return format!("builtin:{base}-{}", if dark { "dark" } else { "light" });
            }
        }
    }
    path
}

/// theme.js resolveWallpaperPath: the wallpaper path of these settings in a mode ("" for a solid colour).
pub fn resolve_wallpaper_path(s: &Value, dark: bool) -> String {
    let w = &s["wallpaper"];
    if w["type"] == "color" {
        return String::new();
    }
    let path = w["path"].as_str().filter(|p| !p.is_empty()).unwrap_or("builtin:lotus-dark");
    twin_for(path, dark, w["followTheme"] != false)
}

/// Where the UI's own files live (built-in wallpapers), as the shell's wallpaper.rs.
fn ui_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(dir) = std::env::var_os("WIN11_UI_DIR") {
        roots.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        roots.push(dir.join("../share/win11-gnome/ui"));
    }
    roots.push(glib::user_data_dir().join("win11-gnome/ui"));
    roots.push(PathBuf::from("/usr/share/win11-gnome/ui"));
    roots
}

/// The bytes of a wallpaper (worker thread): a built-in SVG from the UI files or the backend's /ui/, a file
/// through the backend's /thumb (`px` wide) or /file.
fn wallpaper_bytes(backend: &kit::Backend, path: &str, px: i32) -> Option<Vec<u8>> {
    let path = builtin_alias(path);
    if let Some(id) = path.strip_prefix("builtin:") {
        if id.is_empty() || id.contains('/') || id.contains("..") {
            return None;
        }
        for root in ui_roots() {
            if let Ok(bytes) = std::fs::read(root.join("assets/wallpapers").join(format!("{id}.svg"))) {
                return Some(bytes);
            }
        }
        return backend.get(&format!("/ui/assets/wallpapers/{}.svg", enc(id))).ok();
    }
    let file = if path.starts_with("file://") {
        gio::File::for_uri(&path).path()?.to_string_lossy().into_owned()
    } else if path.starts_with('/') {
        path.clone()
    } else {
        return None;
    };
    backend
        .get(&format!("/thumb?path={}&size={px}", enc(&file)))
        .ok()
        .filter(|b| !b.is_empty())
        .or_else(|| backend.get(&format!("/file?path={}", enc(&file))).ok())
}

/// Decode image bytes (SVG or bitmap) at about `px` wide, cover-scaled to `size` when given.
fn decode(bytes: &[u8], px: i32, stretch: Option<(i32, i32)>) -> Option<gdk_pixbuf::Pixbuf> {
    let loader = gdk_pixbuf::PixbufLoader::new();
    loader.connect_size_prepared(move |loader, w, h| {
        if w <= 0 || h <= 0 {
            return;
        }
        if let Some((sw, sh)) = stretch {
            loader.set_size(sw, sh);
            return;
        }
        if px > 0 && w > px {
            let s = px as f64 / w as f64;
            loader.set_size(px, ((h as f64 * s).round() as i32).max(1));
        }
    });
    let ok = loader.write(bytes).is_ok();
    let closed = loader.close().is_ok();
    if !ok || !closed {
        return None;
    }
    loader.pixbuf()
}

fn texture_of(pixbuf: &gdk_pixbuf::Pixbuf) -> gdk::Texture {
    let format = if pixbuf.has_alpha() { gdk::MemoryFormat::R8g8b8a8 } else { gdk::MemoryFormat::R8g8b8 };
    gdk::MemoryTexture::new(pixbuf.width(), pixbuf.height(), format, &pixbuf.read_pixel_bytes(), pixbuf.rowstride() as usize).upcast()
}

type TextureWaiters = Vec<Box<dyn FnOnce(Option<gdk::Texture>)>>;

thread_local! {
    static TEXTURES: RefCell<HashMap<String, Option<gdk::Texture>>> = RefCell::new(HashMap::new());
    static WAITING: RefCell<HashMap<String, TextureWaiters>> = RefCell::new(HashMap::new());
}

/// A wallpaper as a texture about `px` wide (common.js wallpaperUrl: built-ins as they are, files as the
/// backend's thumbnail); decoded off the main thread once and cached.
pub fn wallpaper_texture(path: &str, px: i32, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    let key = format!("{px}|{path}");
    if let Some(hit) = TEXTURES.with(|t| t.borrow().get(&key).cloned()) {
        done(hit);
        return;
    }
    let first = WAITING.with(|w| {
        let mut w = w.borrow_mut();
        let list = w.entry(key.clone()).or_default();
        list.push(Box::new(done));
        list.len() == 1
    });
    if !first {
        return;
    }
    let backend = kit::ctx().backend();
    let path = path.to_owned();
    kit::spawn(
        move || {
            let bytes = wallpaper_bytes(&backend, &path, px)?;
            let pb = decode(&bytes, px, None)?;
            Some(texture_of(&pb))
        },
        move |t: Option<gdk::Texture>| {
            TEXTURES.with(|c| {
                let mut c = c.borrow_mut();
                if c.len() > 48 {
                    c.clear();
                }
                c.insert(key.clone(), t.clone());
            });
            let list = WAITING.with(|w| w.borrow_mut().remove(&key)).unwrap_or_default();
            for f in list {
                f(t.clone());
            }
        },
    );
}

/// common.js accentFromImage: the dominant tone of a wallpaper (64 x 40 samples; saturated, mid-bright pixels
/// count most) as a Windows-like accent hex, or None.
pub fn accent_from_image(path: &str, done: impl FnOnce(Option<String>) + 'static) {
    let backend = kit::ctx().backend();
    let path = path.to_owned();
    kit::spawn(
        move || {
            let bytes = wallpaper_bytes(&backend, &path, 256)?;
            let pb = decode(&bytes, 0, Some((64, 40)))?;
            dominant(&pb)
        },
        done,
    );
}

fn dominant(pb: &gdk_pixbuf::Pixbuf) -> Option<String> {
    let bytes = pb.read_pixel_bytes();
    let data: &[u8] = &bytes;
    let (w, h, stride, n) = (pb.width() as usize, pb.height() as usize, pb.rowstride() as usize, pb.n_channels() as usize);
    let mut buckets = [(0f64, 0f64, 0f64, 0f64); 18];
    for y in 0..h {
        for x in 0..w {
            let i = y * stride + x * n;
            if i + 2 >= data.len() {
                continue;
            }
            let (r, g, b) = (data[i] as f64, data[i + 1] as f64, data[i + 2] as f64);
            let max = r.max(g).max(b);
            let min = r.min(g).min(b);
            let v = max / 255.0;
            let s = if max > 0.0 { (max - min) / max } else { 0.0 };
            if s < 0.18 || v < 0.15 {
                continue;
            }
            let d = max - min;
            let mut hue: f64 = 0.0;
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
            let wgt = s * s * (1.0 - (v - 0.65).abs());
            let k = &mut buckets[((hue / 20.0).floor() as usize) % 18];
            k.0 += wgt;
            k.1 += r * wgt;
            k.2 += g * wgt;
            k.3 += b * wgt;
        }
    }
    let best = buckets.iter().copied().fold((0.0, 0.0, 0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a });
    if best.0 <= 0.0 {
        return None;
    }
    let ch = |v: f64| (v / best.0).round().clamp(0.0, 255.0) as u8;
    let hex = format!("#{:02x}{:02x}{:02x}", ch(best.1), ch(best.2), ch(best.3));
    let (l, c, hh) = hex_to_oklch(&hex);
    kit::theme::normalize_hex(&oklch_to_hex(l.clamp(0.48, 0.62), c.clamp(0.1, 0.2), hh))
}

// ------------------------------------------------------------------------------------------- avatar

/// components.js Avatar: the picture at `image` (a file path) in a circle, else the initials (first and last
/// word) or the user glyph, `size` px.
pub fn avatar(name: &str, image: Option<&str>, size: i32) -> gtk::Widget {
    let initials: Vec<char> = name.split_whitespace().filter_map(|w| w.chars().next()).filter(|c| c.is_alphanumeric()).collect();
    let text: String = match initials.len() {
        0 => String::new(),
        1 => initials[0].to_uppercase().collect(),
        n => format!("{}{}", initials[0], initials[n - 1]).to_uppercase(),
    };
    let base = kit::controls::avatar(if text.is_empty() { "" } else { &text }, size);
    // kit::controls::avatar takes the first letters of the first two words; hand it the initials as one word
    if !text.is_empty()
        && let Some(l) = base.first_child().and_downcast::<gtk::Label>()
    {
        l.set_label(&text);
    }
    base.add_css_class("st-avatar");
    base.set_overflow(gtk::Overflow::Hidden);
    if let Some(path) = image.filter(|p| !p.is_empty()) {
        let path = path.to_owned();
        let weak = base.downgrade();
        let px = size * 2;
        kit::spawn(
            move || {
                let bytes = std::fs::read(&path).ok()?;
                let pb = decode(&bytes, px, None)?;
                // cover-crop to a square
                let (w, h) = (pb.width(), pb.height());
                let side = w.min(h);
                let sq = pb.new_subpixbuf((w - side) / 2, (h - side) / 2, side, side);
                Some(texture_of(&sq))
            },
            move |t: Option<gdk::Texture>| {
                let (Some(b), Some(t)) = (weak.upgrade(), t) else { return };
                c::clear(&b);
                let pic = gtk::Picture::for_paintable(&t);
                pic.set_content_fit(gtk::ContentFit::Cover);
                pic.set_size_request(size, size);
                pic.set_can_shrink(true);
                b.append(&pic);
            },
        );
    }
    base.upcast()
}

// ------------------------------------------------------------------------------------------- hero

/// The hero row (common.js Hero, `.st-hero`): art left, title and lines, actions below them, an aside on the
/// right; it wraps on narrow windows.
pub fn hero(art: Option<&gtk::Widget>, title: &str, lines: &[String], actions: Option<&gtk::Widget>, aside: Option<&gtk::Widget>) -> Wrap {
    let row = Wrap::new(24, 16);
    row.add_css_class("st-hero");
    if let Some(a) = art {
        a.set_valign(gtk::Align::Center);
        row.append(a);
    }
    let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text.add_css_class("st-hero-text");
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.set_size_request(200, -1);
    if !title.is_empty() {
        let t = c::label(title, "st-hero-title", true);
        t.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        text.append(&t);
    }
    for l in lines.iter().filter(|l| !l.is_empty()) {
        text.append(&c::label(l, "st-hero-line", true));
    }
    if let Some(a) = actions {
        let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        b.add_css_class("st-hero-actions");
        b.set_halign(gtk::Align::Start);
        b.append(a);
        text.append(&b);
    }
    row.append(&text);
    if let Some(a) = aside {
        a.set_valign(gtk::Align::Center);
        a.set_halign(gtk::Align::End);
        row.append(a);
    }
    row
}

/// A small status tile (common.js HeroTile, `.st-tile`): glyph, title and status lines; a button when
/// `on_click` is given.
pub fn hero_tile(icon: &str, title: &str, status: &[String], on_click: Option<Rc<dyn Fn()>>) -> gtk::Widget {
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    ic.add_css_class("st-tile-icon");
    ic.set_size_request(32, 32);
    ic.set_valign(gtk::Align::Center);
    ic.set_hexpand(false);
    let g = kit::icons::glyph(icon, 24);
    g.set_hexpand(true);
    g.set_halign(gtk::Align::Center);
    g.set_valign(gtk::Align::Center);
    ic.append(&g);
    body.append(&ic);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    text.append(&c::label(title, "st-tile-title", false));
    for s in status.iter().filter(|s| !s.is_empty()) {
        text.append(&c::label(s, "st-tile-status", false));
    }
    body.append(&text);
    let w: gtk::Widget = match on_click {
        Some(f) => {
            let b = gtk::Button::new();
            b.add_css_class("st-tile");
            b.add_css_class("st-clickable");
            b.set_child(Some(&body));
            b.connect_clicked(move |_| f());
            b.upcast()
        }
        None => {
            body.add_css_class("st-tile");
            body.upcast()
        }
    };
    w.set_size_request(200, 56);
    w
}

/// How full a volume is, 0..100: the host's df-style `percent` (used / (used + free)), as Taakbeheer and
/// Verkenner show it (common.js volumePercent).
pub fn volume_percent(v: &Value) -> f64 {
    let p = match v["percent"].as_f64().filter(|p| p.is_finite()) {
        Some(p) => p,
        None => {
            let used = v["used"].as_f64().unwrap_or(0.0);
            let free = v["free"].as_f64().unwrap_or(0.0);
            if used + free > 0.0 { used / (used + free) * 100.0 } else { 0.0 }
        }
    };
    p.clamp(0.0, 100.0)
}

// ------------------------------------------------------------------------------------------- usage bar

struct BarState {
    pct: Cell<f64>,
    color: RefCell<Option<String>>,
    warn_at: f64,
}

/// A thin usage bar (common.js UsageBar, `.st-usage`): 0..100, red from `warn_at`, or a fixed colour (a token
/// name such as "sys-success" or "chart-1", or a hex colour).
#[derive(Clone)]
pub struct UsageBar {
    area: gtk::DrawingArea,
    state: Rc<BarState>,
}

impl UsageBar {
    pub fn new(pct: f64, color: Option<&str>, warn_at: f64) -> UsageBar {
        let area = gtk::DrawingArea::new();
        area.add_css_class("st-usage");
        area.set_content_height(4);
        area.set_hexpand(true);
        area.set_valign(gtk::Align::Center);
        let state = Rc::new(BarState { pct: Cell::new(0.0), color: RefCell::new(None), warn_at });
        // the widget owns the state (it holds no widget, so there is no cycle): the handle may be dropped
        let st = state.clone();
        area.set_draw_func(move |_, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let r = h / 2.0;
            let rounded = |x: f64, width: f64| {
                cr.new_sub_path();
                let rr = r.min(width / 2.0);
                cr.arc(x + width - rr, rr, rr, -std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2);
                cr.arc(x + rr, rr, rr, std::f64::consts::FRAC_PI_2, 1.5 * std::f64::consts::PI);
                cr.close_path();
            };
            let t = kit::chart::resolve("chart-track");
            cr.set_source_rgba(t.0, t.1, t.2, t.3);
            rounded(0.0, w);
            let _ = cr.fill();
            let p = st.pct.get();
            if p <= 0.0 {
                return;
            }
            let color = st.color.borrow().clone().unwrap_or_else(|| if p >= st.warn_at { "sys-critical".into() } else { "accent".into() });
            let c = kit::chart::resolve(&color);
            cr.set_source_rgba(c.0, c.1, c.2, c.3);
            cr.save().ok();
            rounded(0.0, w);
            cr.clip();
            rounded(0.0, (w * p / 100.0).max(h.min(w)));
            let _ = cr.fill();
            cr.restore().ok();
        });
        kit::theme::track(&area);
        let bar = UsageBar { area, state };
        bar.set(pct, color);
        bar
    }

    /// The bar.
    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    /// Another value (and colour).
    pub fn set(&self, pct: f64, color: Option<&str>) {
        let p = if pct.is_finite() { pct.clamp(0.0, 100.0) } else { 0.0 };
        self.state.pct.set(p);
        *self.state.color.borrow_mut() = color.map(str::to_owned);
        self.area.queue_draw();
    }

    /// The thickness in px (4 by default; 6 and 8 for the larger bars).
    pub fn set_thickness(&self, px: i32) {
        self.area.set_content_height(px);
    }
}

// ------------------------------------------------------------------------------------------- recents, info

fn state_path() -> PathBuf {
    let settings = kit::ctx::settings_path();
    settings.with_file_name("settings-app.json")
}

thread_local! {
    static RECENT: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
    static SHELL_INFO: RefCell<Value> = const { RefCell::new(Value::Null) };
}

/// Recently opened sub-pages, newest first (common.js recentPages; feeds "Aanbevolen instellingen").
pub fn recent_pages() -> Vec<&'static str> {
    let list = RECENT.with(|r| {
        r.borrow_mut()
            .get_or_insert_with(|| {
                std::fs::read(state_path())
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v["recent"].as_array().cloned())
                    .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default()
            })
            .clone()
    });
    list.iter().filter_map(|id| crate::registry::page(id).map(|p| p.id)).collect()
}

/// Remember that a sub-page was opened (common.js recordVisit); kept in settings-app.json next to the
/// settings, six at most.
pub fn record_visit(id: &str) {
    let Some(p) = crate::registry::page(id) else { return };
    if p.parent.is_none() {
        return;
    }
    let mut list: Vec<String> = recent_pages().into_iter().filter(|x| *x != id).map(str::to_owned).collect();
    list.insert(0, id.to_owned());
    list.truncate(6);
    if RECENT.with(|r| r.borrow().as_ref() == Some(&list)) {
        return;
    }
    RECENT.with(|r| *r.borrow_mut() = Some(list.clone()));
    let path = state_path();
    kit::spawn(
        move || {
            let mut state = std::fs::read(&path)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            state["recent"] = json!(list);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, state.to_string()).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        },
        |_| {},
    );
}

/// shell.info as theme.js getShellInfo keeps it (version, mode, compositor, dryRun, user); Null until loaded.
pub fn shell_info() -> Value {
    SHELL_INFO.with(|s| s.borrow().clone())
}

/// Load shell.info once (main.rs at start).
pub fn load_shell_info() {
    kit::ctx().call("shell.info", json!({}), |r| {
        if let Ok(v) = r {
            SHELL_INFO.with(|s| *s.borrow_mut() = v);
        }
    });
}
