//! Flyout `quick`: Snelle instellingen (Win+A), ported from vendor/ui/js/shell/quicksettings.js and the QUICK
//! SETTINGS part of vendor/ui/css/flyouts.css. Layout like Windows 11 23H2:
//!   [media card]   a separate card when a player exists (media.list / media.changed)
//!   [tiles 3x2]    Wi-Fi >, Bluetooth >, Vliegtuigmodus, Nachtlampje, Donkere modus, Niet storen
//!   [sliders]      helderheid (only when available), volume (mute button, > output devices)
//!   [footer]       battery % (when present), all settings, power menu
//! Sub-pages (Wi-Fi networks, Bluetooth devices, sound output) slide in with a back arrow. Live updates through
//! system.*.changed, media.changed and settings.changed. Changes are optimistic and revert on an error or in the
//! host's test mode (dryRun), with a short note on the visible page.
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use crate::icons::{app_icon, glyph};
use gtk::prelude::*;
use gtk::{gdk, glib, pango};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/* ==========================================================================
   Texts (quicksettings.js)
   ========================================================================== */

const TEXTS: &[(&str, &str, &str)] = &[
    ("title", "Snelle instellingen", "Quick settings"),
    ("wifi", "Wi-Fi", "Wi-Fi"),
    ("wifiMore", "Wi-Fi-netwerken weergeven", "Show Wi-Fi networks"),
    ("bluetooth", "Bluetooth", "Bluetooth"),
    ("btMore", "Bluetooth-apparaten weergeven", "Show Bluetooth devices"),
    ("airplane", "Vliegtuigmodus", "Flight mode"),
    ("nightLight", "Nachtlampje", "Night light"),
    ("darkMode", "Donkere modus", "Dark mode"),
    ("dnd", "Niet storen", "Do not disturb"),
    ("ethernet", "Ethernet", "Ethernet"),
    ("notConnected", "Niet verbonden", "Not connected"),
    ("network", "Netwerk", "Network"),
    ("brightness", "Helderheid", "Brightness"),
    ("volume", "Volume", "Volume"),
    ("mute", "Geluid uit", "Mute"),
    ("unmute", "Geluid aan", "Unmute"),
    ("output", "Geluidsuitvoer", "Sound output"),
    ("outputMore", "Een geluidsuitvoer kiezen", "Select a sound output"),
    ("allSettings", "Alle instellingen", "All settings"),
    ("battery", "Batterij: {p}", "Battery: {p}"),
    ("batteryCharging", "Batterij: {p} (wordt opgeladen)", "Battery: {p} (charging)"),
    ("batteryFull", "Batterij: {p} (aangesloten)", "Battery: {p} (plugged in)"),
    ("back", "Terug", "Back"),
    ("moreWifi", "Meer Wi-Fi-instellingen", "More Wi-Fi settings"),
    ("moreBt", "Meer Bluetooth-instellingen", "More Bluetooth settings"),
    ("moreSound", "Meer volume-instellingen", "More volume settings"),
    ("connected", "Verbonden", "Connected"),
    ("connectedSecure", "Verbonden, beveiligd", "Connected, secured"),
    ("connectedOpen", "Verbonden, open", "Connected, open"),
    ("secure", "Beveiligd", "Secured"),
    ("open", "Open", "Open"),
    ("saved", "Beveiligd, opgeslagen", "Secured, saved"),
    ("connect", "Verbinding maken", "Connect"),
    ("disconnect", "Verbinding verbreken", "Disconnect"),
    ("connecting", "Verbinding maken…", "Connecting…"),
    ("disconnecting", "Verbinding verbreken…", "Disconnecting…"),
    ("autoConnect", "Automatisch verbinding maken", "Connect automatically"),
    ("properties", "Eigenschappen", "Properties"),
    ("enterKey", "Voer de netwerkbeveiligingssleutel in", "Enter the network security key"),
    ("password", "Wachtwoord", "Password"),
    ("next", "Volgende", "Next"),
    ("cancel", "Annuleren", "Cancel"),
    (
        "openWarning",
        "Anderen kunnen mogelijk zien wat je via dit netwerk verstuurt.",
        "Other people might be able to see info you send over this network.",
    ),
    ("scanning", "Zoeken naar netwerken…", "Looking for networks…"),
    ("noNetworks", "Geen netwerken gevonden", "No networks found"),
    ("wifiOff", "Wi-Fi is uitgeschakeld", "Wi-Fi is turned off"),
    ("wifiOffHint", "Zet Wi-Fi hierboven aan om netwerken te zien.", "Turn on Wi-Fi above to see networks."),
    ("airplaneOn", "Vliegtuigmodus staat aan", "Flight mode is on"),
    (
        "airplaneHint",
        "Zet vliegtuigmodus uit om weer verbinding te maken.",
        "Turn off flight mode to connect again.",
    ),
    ("connectFailed", "Kan geen verbinding maken met dit netwerk.", "Can’t connect to this network."),
    ("btOff", "Bluetooth is uitgeschakeld", "Bluetooth is turned off"),
    (
        "btOffHint",
        "Zet Bluetooth hierboven aan om je apparaten te gebruiken.",
        "Turn on Bluetooth above to use your devices.",
    ),
    ("btNone", "Geen gekoppelde apparaten", "No paired devices"),
    (
        "btNoneHint",
        "Kies hieronder “Apparaat toevoegen” om er een te koppelen.",
        "Choose “Add device” below to pair one.",
    ),
    ("btBattery", "{p} batterij", "{p} battery"),
    ("pairNew", "Apparaat toevoegen", "Add device"),
    ("btStuck", "Bluetooth reageert niet.", "Bluetooth is not responding."),
    ("btReset", "Bluetooth opnieuw starten", "Restart Bluetooth"),
    ("btResetting", "Bluetooth wordt opnieuw gestart…", "Restarting Bluetooth…"),
    ("btResetOk", "Bluetooth werkt weer.", "Bluetooth is working again."),
    ("noSinks", "Geen geluidsapparaten gevonden", "No sound devices found"),
    ("playing", "Speelt af", "Playing"),
    ("paused", "Gepauzeerd", "Paused"),
    ("previous", "Vorige", "Previous"),
    ("next2", "Volgende", "Next"),
    ("play", "Afspelen", "Play"),
    ("pause", "Onderbreken", "Pause"),
    ("otherPlayer", "Andere speler", "Other player"),
    ("unavailable", "Niet beschikbaar", "Not available"),
    ("dryRun", "Testmodus: er is niets veranderd.", "Test mode: nothing was changed."),
    ("failed", "Dat is niet gelukt. Probeer het opnieuw.", "That didn’t work. Try again."),
    ("on", "Aan", "On"),
    ("off", "Uit", "Off"),
    // footer power menu (startmenu.js)
    ("power", "Aan/uit", "Power"),
    ("sleep", "Slaapstand", "Sleep"),
    ("shutdown", "Afsluiten", "Shut down"),
    ("restart", "Opnieuw opstarten", "Restart"),
];

fn sh() -> Rc<Shell> {
    crate::core::shell()
}

fn t(key: &str) -> String {
    TEXTS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, nl, en)| sh().t(nl, en))
        .unwrap_or_else(|| key.to_owned())
}

fn tp(key: &str, p: &str) -> String {
    t(key).replace("{p}", p)
}

/* ==========================================================================
   Small helpers: JSON and widgets
   ========================================================================== */

/// JavaScript truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0 && !x.is_nan()),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn is_false(v: &Value) -> bool {
    *v == Value::Bool(false)
}

/// The host runs with --dry-run (preview / testing): it answered {ok, dryRun} and changed nothing.
fn is_dry_run(r: &Value) -> bool {
    r.is_object() && truthy(&r["dryRun"])
}

/// `{...v, key: val}` (a non-object becomes an object).
fn with(v: &Value, key: &str, val: Value) -> Value {
    let mut o = v.as_object().cloned().unwrap_or_default();
    o.insert(key.to_owned(), val);
    Value::Object(o)
}

fn pct(v: f64) -> String {
    format!("{}%", v.round() as i64)
}

fn err_text(e: &str) -> String {
    let l = e.to_ascii_lowercase();
    if l.contains("unknown_method") || l.contains("unknown method") {
        t("unavailable")
    } else if e.trim().is_empty() {
        t("failed")
    } else {
        e.to_owned()
    }
}

fn wifi_icon(strength: Option<f64>) -> &'static str {
    match strength {
        None => "wifi",
        Some(s) if s >= 67.0 => "wifi-3",
        Some(s) if s >= 34.0 => "wifi-2",
        Some(_) => "wifi-1",
    }
}

fn volume_icon(volume: f64, muted: bool) -> &'static str {
    if muted || volume <= 0.0 {
        "volume-mute"
    } else if volume < 34.0 {
        "volume-1"
    } else if volume < 67.0 {
        "volume-2"
    } else {
        "volume"
    }
}

fn lbl(text: &str, classes: &[&str]) -> gtk::Label {
    let l = gtk::Label::new(Some(text));
    l.set_xalign(0.0);
    for c in classes {
        l.add_css_class(c);
    }
    l
}

/// One line, ellipsized; takes the width it is given instead of asking for its whole text.
fn ell(l: gtk::Label) -> gtk::Label {
    l.set_ellipsize(pango::EllipsizeMode::End);
    l.set_single_line_mode(true);
    l.set_max_width_chars(8);
    l.set_hexpand(true);
    l
}

/// Wrapping text within the flyout's width.
fn wrapped(l: gtk::Label) -> gtk::Label {
    l.set_wrap(true);
    l.set_wrap_mode(pango::WrapMode::WordChar);
    l.set_max_width_chars(40);
    l
}

fn hbox(spacing: i32, classes: &[&str]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, spacing);
    for c in classes {
        b.add_css_class(c);
    }
    b
}

fn vbox(spacing: i32, classes: &[&str]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, spacing);
    for c in classes {
        b.add_css_class(c);
    }
    b
}

fn grow() -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.set_hexpand(true);
    b
}

fn clear_box(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

fn set_a11y(w: &impl IsA<gtk::Accessible>, text: &str) {
    w.update_property(&[gtk::accessible::Property::Label(text)]);
}

/// A button without the GTK theme's look (quick.css styles it by its classes).
fn button(classes: &[&str]) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("q-b");
    for c in classes {
        b.add_css_class(c);
    }
    b
}

fn icon_btn(icon: &str, label: &str, size: i32, icon_size: i32, classes: &[&str]) -> gtk::Button {
    let b = button(classes);
    b.add_css_class("q-icon-btn");
    b.set_child(Some(&glyph(icon, icon_size)));
    b.set_tooltip_text(Some(label));
    set_a11y(&b, label);
    b.set_size_request(size, size);
    b.set_valign(gtk::Align::Center);
    b
}

fn set_btn_icon(b: &gtk::Button, icon: &str, size: i32) {
    b.set_child(Some(&glyph(icon, size)));
}

fn set_btn_label(b: &gtk::Button, text: &str) {
    let l = lbl(text, &[]);
    l.set_xalign(0.5);
    l.set_ellipsize(pango::EllipsizeMode::End);
    b.set_child(Some(&l));
}

/// Win11 Button (standard or accent).
fn text_btn(label: &str, accent: bool) -> gtk::Button {
    let b = button(&["q-btn"]);
    if accent {
        b.add_css_class("q-accent");
    }
    set_btn_label(&b, label);
    b
}

/// Indeterminate progress bar that pulses while it exists.
fn indeterminate(label: &str) -> gtk::ProgressBar {
    let p = gtk::ProgressBar::new();
    p.add_css_class("q-progress");
    p.set_pulse_step(0.06);
    set_a11y(&p, label);
    let weak = p.downgrade();
    glib::timeout_add_local(Duration::from_millis(50), move || match weak.upgrade() {
        Some(p) => {
            if p.is_mapped() {
                p.pulse();
            }
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    p
}

fn empty_msg(title: &str, hint: Option<&str>, icon: Option<&str>) -> gtk::Box {
    let b = vbox(4, &["fo-sub__empty"]);
    b.set_valign(gtk::Align::Center);
    b.set_vexpand(true);
    if let Some(i) = icon {
        let g = glyph(i, 24);
        g.add_css_class("fo-sub__empty-icon");
        g.set_margin_bottom(6);
        b.append(&g);
    }
    let tl = wrapped(lbl(title, &["q-strong"]));
    tl.set_justify(gtk::Justification::Center);
    tl.set_xalign(0.5);
    b.append(&tl);
    if let Some(h) = hint {
        let hl = wrapped(lbl(h, &["q-caption", "q-text-2"]));
        hl.set_justify(gtk::Justification::Center);
        hl.set_xalign(0.5);
        b.append(&hl);
    }
    b
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = if bytes[i] == b'%' && i + 3 <= bytes.len() {
            std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        } else {
            None
        };
        match hex {
            Some(v) => {
                out.push(v);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Open Instellingen (optionally on a page: wifi, bluetooth, sound, power, network) and close the flyout.
fn open_settings(page: Option<&str>) {
    let shell = sh();
    let params = match page {
        Some(p) => json!({"page": p}),
        None => json!({}),
    };
    shell.fire("shell.openApp", json!({"app": "settings", "params": params}));
    close_flyouts(&shell);
}

fn close_flyouts(shell: &Shell) {
    let host = shell.flyouts.borrow().clone();
    if let Some(h) = host {
        h.close();
    }
}

fn flyout_open(name: &str) -> bool {
    let host = sh().flyouts.borrow().clone();
    host.is_some_and(|h| h.is_open(name))
}

/* ==========================================================================
   Look: own light/dark classes and the Win11 accent palette (theme.js accentPalette)
   ========================================================================== */

thread_local! {
    static PALETTE: RefCell<Option<(gtk::CssProvider, String)>> = const { RefCell::new(None) };
    /// (accent fill in dark mode, accent fill in light mode), 0..1 rgb, for drawn glyphs.
    static ACCENT: Cell<([f64; 3], [f64; 3])> = const { Cell::new(([0.298, 0.761, 1.0], [0.0, 0.404, 0.753])) };
}

/// The palette root of quick settings and the power dialog: `q-root` plus `q-dark` / `q-light`.
pub(super) fn style_root(w: &impl IsA<gtk::Widget>) {
    w.add_css_class("q-root");
    apply_mode(w);
}

pub(super) fn apply_mode(w: &impl IsA<gtk::Widget>) {
    if sh().dark() {
        w.add_css_class("q-dark");
        w.remove_css_class("q-light");
    } else {
        w.add_css_class("q-light");
        w.remove_css_class("q-dark");
    }
}

fn accent_rgb(dark: bool) -> [f64; 3] {
    let (d, l) = ACCENT.with(|a| a.get());
    if dark { d } else { l }
}

fn parse_hex(input: &str) -> Option<[f64; 3]> {
    let h = input.trim().trim_start_matches('#').to_ascii_lowercase();
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let h = match h.len() {
        3 => h.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => h,
        8 => h[..6].to_owned(),
        _ => return None,
    };
    let n = u32::from_str_radix(&h, 16).ok()?;
    Some([((n >> 16) & 255) as f64, ((n >> 8) & 255) as f64, (n & 255) as f64])
}

fn to_lin(c: f64) -> f64 {
    let c = c / 255.0;
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn from_lin(c: f64) -> f64 {
    255.0 * if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}

fn to_oklch(rgb: [f64; 3]) -> (f64, f64, f64) {
    let [r, g, b] = rgb.map(to_lin);
    let l = (0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b).cbrt();
    let m = (0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b).cbrt();
    let s = (0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b).cbrt();
    let ll = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
    let a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
    let bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
    (ll, a.hypot(bb), (bb.atan2(a).to_degrees() + 360.0) % 360.0)
}

fn oklch_lin(l: f64, c: f64, h: f64) -> [f64; 3] {
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

/// OKLCH to sRGB (0..255), reducing chroma until the colour fits.
fn oklch_rgb(l: f64, c: f64, h: f64) -> [f64; 3] {
    let l = l.clamp(0.0, 1.0);
    let fits = |v: &[f64; 3]| v.iter().all(|x| *x >= -0.0005 && *x <= 1.0005);
    let mut rgb = oklch_lin(l, c, h);
    if !fits(&rgb) {
        let (mut lo, mut hi) = (0.0, c);
        for _ in 0..22 {
            let mid = (lo + hi) / 2.0;
            if fits(&oklch_lin(l, mid, h)) {
                lo = mid
            } else {
                hi = mid
            }
        }
        rgb = oklch_lin(l, lo, h);
    }
    rgb.map(|v| from_lin(v.clamp(0.0, 1.0)).round().clamp(0.0, 255.0))
}

fn luminance(rgb: [f64; 3]) -> f64 {
    let [r, g, b] = rgb.map(to_lin);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast(a: [f64; 3], b: [f64; 3]) -> f64 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

fn text_on(fill: [f64; 3], prefer: [f64; 3]) -> [f64; 3] {
    let other = if prefer[0] > 128.0 { [0.0; 3] } else { [255.0; 3] };
    if contrast(prefer, fill) >= 3.5 {
        return prefer;
    }
    if contrast(other, fill) > contrast(prefer, fill) { other } else { prefer }
}

struct Pal {
    light2: [f64; 3],
    light3: [f64; 3],
    dark1: [f64; 3],
    dark2: [f64; 3],
    on_light: [f64; 3],
    on_dark: [f64; 3],
}

fn accent_palette(input: &str) -> Pal {
    let base = parse_hex(input).unwrap_or([0.0, 120.0, 212.0]);
    let (light2, light3, dark1, dark2) = if base == [0.0, 120.0, 212.0] || base == [0.0, 103.0, 192.0] {
        // Windows' own palette for the default blue
        ([76.0, 194.0, 255.0], [153.0, 235.0, 255.0], [0.0, 103.0, 192.0], [0.0, 62.0, 146.0])
    } else {
        let (l, c, h) = to_oklch(base);
        let mk = |ll: f64, cm: f64| oklch_rgb(ll.clamp(0.12, 0.97), c * cm, h);
        let up = |k: f64| l + (0.95 - l) * k;
        (
            mk(up(0.55).max(0.72), 0.86),
            mk(up(0.85).max(0.86), 0.55),
            mk((l * 0.91).min(0.6), 0.97),
            mk((l * 0.68).min(0.47), 0.9),
        )
    };
    Pal {
        light2,
        light3,
        dark1,
        dark2,
        on_light: text_on(dark1, [255.0; 3]),
        on_dark: text_on(light2, [0.0; 3]),
    }
}

fn css_rgb(c: [f64; 3]) -> String {
    format!("rgb({}, {}, {})", c[0] as u8, c[1] as u8, c[2] as u8)
}

fn css_rgba(c: [f64; 3], a: f64) -> String {
    format!("rgba({}, {}, {}, {a})", c[0] as u8, c[1] as u8, c[2] as u8)
}

/// The accent part of the palette as a provider of its own (above the module sheets).
pub(super) fn refresh_palette() {
    let accent = sh().setting("theme.accent").as_str().unwrap_or("#0078D4").to_owned();
    let p = accent_palette(&accent);
    ACCENT.with(|a| a.set((p.light2.map(|v| v / 255.0), p.dark1.map(|v| v / 255.0))));
    let css = format!(
        ".q-root {{ --q-accent: {}; --q-accent-2: {}; --q-accent-3: {}; --q-accent-text: {}; --q-accent-text-2: {}; \
         --q-accent-fg: {}; }}\n\
         .q-root.q-light {{ --q-accent: {}; --q-accent-2: {}; --q-accent-3: {}; --q-accent-text: {}; \
         --q-accent-text-2: {}; --q-accent-fg: {}; }}\n",
        css_rgb(p.light2),
        css_rgba(p.light2, 0.9),
        css_rgba(p.light2, 0.8),
        css_rgb(p.on_dark),
        css_rgba(p.on_dark, 0.5),
        css_rgb(p.light3),
        css_rgb(p.dark1),
        css_rgba(p.dark1, 0.9),
        css_rgba(p.dark1, 0.8),
        css_rgb(p.on_light),
        css_rgba(p.on_light, 0.7),
        css_rgb(p.dark2),
    );
    PALETTE.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.as_ref().is_some_and(|(_, old)| *old == css) {
            return;
        }
        let provider = match cell.take() {
            Some((p, _)) => p,
            None => {
                let p = gtk::CssProvider::new();
                if let Some(display) = gdk::Display::default() {
                    gtk::style_context_add_provider_for_display(
                        &display,
                        &p,
                        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
                    );
                }
                p
            }
        };
        provider.load_from_string(&css);
        *cell = Some((provider, css));
    });
}

/* ==========================================================================
   State
   ========================================================================== */

#[derive(Default)]
struct State {
    audio: Value,
    brightness: Value,
    battery: Value,
    network: Value,
    bluetooth: Value,
    players: Vec<Value>,
    player_index: usize,
    picked: bool,
    night_available: bool,
    night_enabled: bool,
    wifi_list: Option<Vec<Value>>,
    scanning: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Bright,
    Vol,
}

#[derive(Default)]
struct Throttle {
    timer: RefCell<Option<glib::SourceId>>,
    pending: RefCell<Option<Value>>,
    last: RefCell<Option<Value>>,
    final_timer: RefCell<Option<glib::SourceId>>,
    touched: Cell<Option<Instant>>,
}

impl Throttle {
    fn recently(&self) -> bool {
        self.touched.get().is_some_and(|t| t.elapsed() < Duration::from_millis(700))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Wifi,
    Bluetooth,
    Sound,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Idle,
    Password,
    Connecting,
}

#[derive(Clone)]
struct RowState {
    stage: Stage,
    error: Option<String>,
    auto: bool,
    draft: String,
}

impl Default for RowState {
    fn default() -> Self {
        RowState { stage: Stage::Idle, error: None, auto: true, draft: String::new() }
    }
}

/// A sub-page (Wi-Fi networks, Bluetooth devices, sound output).
struct Sub {
    kind: Kind,
    name: String,
    el: gtk::Box,
    back: gtk::Button,
    notice: gtk::Box,
    list: gtk::Box,
    toggle: Option<gtk::Switch>,
    progress: Option<gtk::ProgressBar>,
    updating: Cell<bool>,
    expanded: RefCell<Option<String>>,
    rows: RefCell<HashMap<String, RowState>>,
    heads: RefCell<HashMap<String, gtk::Button>>,
    focus_after: RefCell<Option<String>>,
    entry: RefCell<Option<gtk::PasswordEntry>>,
    focus_entry: Cell<bool>,
    busy: RefCell<HashMap<String, &'static str>>,
    errors: RefCell<HashMap<String, String>>,
    not_ready: Cell<bool>,
    resetting: Cell<bool>,
}

/// Win11 quick action tile: 96x48 button (optionally split with a > part) and a caption below.
struct Tile {
    id: &'static str,
    el: gtk::Box,
    main: gtk::Button,
    more: Option<gtk::Button>,
    label: gtk::Label,
    icon: RefCell<String>,
    hidden: Cell<bool>,
}

impl Tile {
    fn new(id: &'static str, icon: &str, text: &str, more_label: Option<String>) -> Rc<Tile> {
        let main = button(&["fo-tile__main"]);
        main.set_child(Some(&glyph(icon, 16)));
        main.set_hexpand(true);
        set_a11y(&main, text);
        let frame = hbox(0, &["fo-tile__box"]);
        frame.set_overflow(gtk::Overflow::Hidden);
        frame.set_size_request(96, 48);
        frame.append(&main);
        let more = more_label.map(|ml| {
            let m = button(&["fo-tile__more"]);
            m.set_child(Some(&glyph("chevron-right", 12)));
            m.set_size_request(34, -1);
            m.set_tooltip_text(Some(&ml));
            set_a11y(&m, &ml);
            frame.add_css_class("is-split");
            frame.append(&m);
            m
        });
        let label = lbl(text, &["fo-tile__label"]);
        label.set_ellipsize(pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
        label.set_max_width_chars(12);
        label.set_xalign(0.5);
        let el = vbox(6, &["fo-tile"]);
        el.set_size_request(96, -1);
        el.append(&frame);
        el.append(&label);
        main.connect_clicked(move |_| {
            if let Some(q) = q() {
                q.tile_clicked(id);
            }
        });
        if let Some(m) = &more {
            m.connect_clicked(move |_| {
                if let Some(q) = q() {
                    q.open_sub(if id == "wifi" { Kind::Wifi } else { Kind::Bluetooth });
                }
            });
        }
        Rc::new(Tile { id, el, main, more, label, icon: RefCell::new(icon.to_owned()), hidden: Cell::new(false) })
    }

    fn is_on(&self) -> bool {
        self.el.has_css_class("is-on")
    }

    fn set_on(&self, on: bool) {
        if on {
            self.el.add_css_class("is-on");
        } else {
            self.el.remove_css_class("is-on");
        }
        self.main.update_state(&[gtk::accessible::State::Pressed(if on {
            gtk::AccessibleTristate::True
        } else {
            gtk::AccessibleTristate::False
        })]);
    }

    fn set_text(&self, text: &str) {
        self.label.set_text(text);
        if text.chars().count() > 14 {
            self.label.set_tooltip_text(Some(text));
        } else {
            self.label.set_tooltip_text(None);
        }
    }

    fn set_glyph(&self, icon: &str) {
        if *self.icon.borrow() != icon {
            self.main.set_child(Some(&glyph(icon, 16)));
            *self.icon.borrow_mut() = icon.to_owned();
        }
    }

    fn set_hidden(&self, hidden: bool) {
        self.hidden.set(hidden);
        self.el.set_visible(!hidden);
    }
}

struct Ui {
    wrap: gtk::Box,
    media: gtk::Box,
    stack: gtk::Stack,
    notice: gtk::Box,
    grid: gtk::Grid,
    tiles: Vec<Rc<Tile>>,
    bright_row: gtk::Box,
    bright: gtk::Scale,
    vol_row: gtk::Box,
    mute: gtk::Button,
    vol: gtk::Scale,
    out: gtk::Button,
    battery: gtk::Button,
    battery_draw: gtk::DrawingArea,
    battery_text: gtk::Label,
    power: gtk::MenuButton,
}

impl Ui {
    fn tile(&self, id: &str) -> Option<Rc<Tile>> {
        self.tiles.iter().find(|t| t.id == id).cloned()
    }
}

struct Quick {
    root: gtk::Box,
    st: RefCell<State>,
    ui: RefCell<Option<Rc<Ui>>>,
    sub: RefCell<Option<Rc<Sub>>>,
    lang: Cell<&'static str>,
    notice_gen: Cell<u64>,
    layout_key: Cell<u32>,
    sub_seq: Cell<u32>,
    bright_th: Throttle,
    vol_th: Throttle,
    battery_glyph: Rc<Cell<(f64, bool)>>,
    art: RefCell<HashMap<String, Option<gdk::Texture>>>,
}

thread_local! {
    static Q: RefCell<Option<Rc<Quick>>> = const { RefCell::new(None) };
}

fn q() -> Option<Rc<Quick>> {
    Q.with(|q| q.borrow().clone())
}

struct QuickFlyout {
    root: gtk::Widget,
}

impl Flyout for QuickFlyout {
    fn name(&self) -> &'static str {
        "quick"
    }
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }
    fn placement(&self) -> Placement {
        Placement::BottomRight
    }
    fn opened(&self) {
        if let Some(q) = q() {
            q.on_open();
        }
    }
    fn closed(&self) {
        if let Some(q) = q() {
            q.on_close();
        }
    }
}

/* ==========================================================================
   Setup
   ========================================================================== */

pub fn register(shell: &Rc<Shell>) {
    refresh_palette();
    let root = vbox(0, &[]);
    style_root(&root);
    let quick = Rc::new(Quick {
        root: root.clone(),
        st: RefCell::new(State::default()),
        ui: RefCell::new(None),
        sub: RefCell::new(None),
        lang: Cell::new(shell.lang()),
        notice_gen: Cell::new(0),
        layout_key: Cell::new(u32::MAX),
        sub_seq: Cell::new(0),
        bright_th: Throttle::default(),
        vol_th: Throttle::default(),
        battery_glyph: Rc::new(Cell::new((0.0, false))),
        art: RefCell::new(HashMap::new()),
    });
    Q.with(|q| *q.borrow_mut() = Some(quick.clone()));
    let ui = quick.build_ui();
    root.append(&ui.wrap);
    *quick.ui.borrow_mut() = Some(ui);

    // Backspace = back (not while typing in the password box)
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|ctrl, key, _, _| {
        if key == gdk::Key::BackSpace
            && let Some(q) = q()
            && q.sub.borrow().is_some()
        {
            let typing = ctrl
                .widget()
                .and_then(|w| w.root())
                .and_then(|r| r.focus())
                .is_some_and(|f| f.is::<gtk::Text>() || f.ancestor(gtk::PasswordEntry::static_type()).is_some());
            if !typing {
                q.close_sub(false);
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    root.add_controller(keys);

    let host = shell.flyouts.borrow().clone();
    if let Some(host) = host {
        host.register(Rc::new(QuickFlyout { root: root.upcast() }));
    }

    shell.on("system.audio.changed", |_, d| {
        let Some(q) = q() else { return };
        q.st.borrow_mut().audio = d.clone();
        q.render_volume();
        if let Some(sub) = q.sub_of(Kind::Sound) {
            q.render_sub(&sub);
        }
    });
    shell.on("system.brightness.changed", |_, d| {
        let Some(q) = q() else { return };
        q.st.borrow_mut().brightness = d.clone();
        q.render_brightness();
    });
    shell.on("system.battery.changed", |_, d| {
        let Some(q) = q() else { return };
        q.st.borrow_mut().battery = d.clone();
        q.render_footer();
    });
    shell.on("system.network.changed", |_, d| {
        let Some(q) = q() else { return };
        q.st.borrow_mut().network = d.clone();
        q.render_tiles();
        if q.sub_of(Kind::Wifi).is_some() {
            q.refresh_wifi_list(false);
        }
    });
    shell.on("system.bluetooth.changed", |_, d| {
        let Some(q) = q() else { return };
        q.st.borrow_mut().bluetooth = d.clone();
        q.render_tiles();
        if let Some(sub) = q.sub_of(Kind::Bluetooth) {
            q.render_bt_event(&sub);
        }
    });
    shell.on("media.changed", |_, d| {
        let Some(q) = q() else { return };
        q.st.borrow_mut().players = players_of(d);
        q.render_media();
    });
    shell.on("settings.changed", |_, _| {
        let Some(q) = q() else { return };
        refresh_palette();
        apply_mode(&q.root);
        let lang = sh().lang();
        if lang != q.lang.get() {
            // language switched in Instellingen: rebuild the texts and fetch the host's texts again
            q.lang.set(lang);
            q.rebuild();
        } else {
            q.render_tiles();
            if let Some(ui) = q.ui() {
                ui.battery_draw.queue_draw();
            }
        }
    });

    quick.render_all();
    quick.refresh_all();
}

fn players_of(v: &Value) -> Vec<Value> {
    if let Some(a) = v.as_array() {
        a.clone()
    } else {
        v["players"].as_array().cloned().unwrap_or_default()
    }
}

impl Quick {
    fn ui(&self) -> Option<Rc<Ui>> {
        self.ui.borrow().clone()
    }

    fn sub_of(&self, kind: Kind) -> Option<Rc<Sub>> {
        self.sub.borrow().clone().filter(|s| s.kind == kind)
    }

    fn is_current(&self, sub: &Rc<Sub>) -> bool {
        self.sub.borrow().as_ref().is_some_and(|s| Rc::ptr_eq(s, sub))
    }

    fn throttle(&self, which: Which) -> &Throttle {
        match which {
            Which::Bright => &self.bright_th,
            Which::Vol => &self.vol_th,
        }
    }

    fn rebuild(self: &Rc<Self>) {
        if self.sub.borrow().is_some() {
            self.close_sub(true);
        }
        let ui = self.build_ui();
        clear_box(&self.root);
        self.root.append(&ui.wrap);
        *self.ui.borrow_mut() = Some(ui);
        self.layout_key.set(u32::MAX);
        self.render_all();
        self.refresh_all();
    }

    /* ---------------------------------------------------------------- UI construction */

    fn build_ui(self: &Rc<Self>) -> Rc<Ui> {
        let wrap = vbox(12, &["fo-qs-wrap"]);
        wrap.set_size_request(360, -1);
        let media = vbox(0, &["fo-media", "q-surface"]);
        media.set_visible(false);
        set_a11y(&media, "Media");
        let panel = vbox(0, &["fo-qs", "q-surface"]);
        panel.set_overflow(gtk::Overflow::Hidden);
        set_a11y(&panel, &t("title"));

        let stack = gtk::Stack::new();
        stack.add_css_class("fo-qs__pages");
        stack.set_vhomogeneous(false);
        stack.set_hhomogeneous(true);
        stack.set_interpolate_size(true);
        stack.set_transition_duration(250);
        let main = vbox(0, &["fo-qs__page", "fo-qs__main"]);
        let notice = vbox(0, &["fo-qs__notice-slot"]);
        notice.set_visible(false);
        main.append(&notice);

        let grid = gtk::Grid::new();
        grid.add_css_class("fo-qs__tiles");
        grid.set_column_spacing(11);
        grid.set_row_spacing(14);
        grid.set_halign(gtk::Align::Center);
        let tiles = vec![
            Tile::new("wifi", "wifi-3", &t("wifi"), Some(t("wifiMore"))),
            Tile::new("bluetooth", "bluetooth", &t("bluetooth"), Some(t("btMore"))),
            Tile::new("airplane", "airplane", &t("airplane"), None),
            Tile::new("nightLight", "night-light", &t("nightLight"), None),
            Tile::new("dark", "moon", &t("darkMode"), None),
            Tile::new("dnd", "notification-off", &t("dnd"), None),
        ];
        main.append(&grid);

        // sliders
        let sliders = vbox(4, &["fo-qs__sliders"]);
        let bright_row = hbox(4, &["fo-qs__slider-row"]);
        let bglyph = gtk::CenterBox::new();
        bglyph.add_css_class("fo-qs__slider-glyph");
        bglyph.set_size_request(40, 40);
        bglyph.set_center_widget(Some(&glyph("brightness", 16)));
        bglyph.set_tooltip_text(Some(&t("brightness")));
        let bright = slider(&t("brightness"));
        let end = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        end.set_size_request(32, -1);
        bright_row.append(&bglyph);
        bright_row.append(&bright);
        bright_row.append(&end);
        bright.connect_change_value(|_, _, v| {
            if let Some(q) = q() {
                q.slider_input(Which::Bright, v);
            }
            glib::Propagation::Proceed
        });

        let vol_row = hbox(4, &["fo-qs__slider-row"]);
        let mute = icon_btn("volume-2", &t("mute"), 40, 16, &["fo-qs__mute"]);
        let vol = slider(&t("volume"));
        let out = icon_btn("chevron-right", &t("outputMore"), 32, 12, &["fo-qs__out"]);
        vol_row.append(&mute);
        vol_row.append(&vol);
        vol_row.append(&out);
        vol.connect_change_value(|_, _, v| {
            if let Some(q) = q() {
                q.slider_input(Which::Vol, v);
            }
            glib::Propagation::Proceed
        });
        mute.connect_clicked(|_| {
            if let Some(q) = q() {
                q.toggle_mute();
            }
        });
        out.connect_clicked(|_| {
            if let Some(q) = q() {
                q.open_sub(Kind::Sound);
            }
        });
        sliders.append(&bright_row);
        sliders.append(&vol_row);
        main.append(&sliders);
        stack.add_named(&main, Some("main"));
        stack.set_visible_child_name("main");

        // footer: battery, all settings, power
        let footer = hbox(0, &["fo-qs__footer"]);
        let battery = button(&["fo-qs__battery"]);
        battery.set_valign(gtk::Align::Center);
        let bb = hbox(8, &[]);
        let battery_draw = gtk::DrawingArea::new();
        battery_draw.set_content_width(16);
        battery_draw.set_content_height(16);
        battery_draw.set_valign(gtk::Align::Center);
        let state = self.battery_glyph.clone();
        battery_draw.set_draw_func(move |area, cr, _, _| {
            let (p, charging) = state.get();
            draw_battery(area, cr, p, charging);
        });
        let battery_text = lbl("", &["q-caption", "q-tabular"]);
        bb.append(&battery_draw);
        bb.append(&battery_text);
        battery.set_child(Some(&bb));
        battery.set_visible(false);
        battery.connect_clicked(|_| open_settings(Some("power")));
        let settings = icon_btn("settings", &t("allSettings"), 40, 16, &["fo-qs__settings"]);
        settings.connect_clicked(|_| open_settings(None));
        let power = power_menu();
        footer.append(&battery);
        footer.append(&grow());
        footer.append(&settings);
        footer.append(&power);

        panel.append(&stack);
        panel.append(&footer);
        wrap.append(&media);
        wrap.append(&panel);

        Rc::new(Ui {
            wrap,
            media,
            stack,
            notice,
            grid,
            tiles,
            bright_row,
            bright,
            vol_row,
            mute,
            vol,
            out,
            battery,
            battery_draw,
            battery_text,
            power,
        })
    }

    /* ---------------------------------------------------------------- data */

    fn refresh_all(self: &Rc<Self>) {
        let shell = sh();
        for (method, slot) in [
            ("system.audio.get", 0u8),
            ("system.brightness.get", 1),
            ("system.battery.get", 2),
            ("system.network.get", 3),
            ("system.bluetooth.get", 4),
        ] {
            shell.call(method, json!({}), move |res| {
                let Some(q) = q() else { return };
                let v = res.unwrap_or(Value::Null);
                {
                    let mut st = q.st.borrow_mut();
                    match slot {
                        0 => st.audio = v,
                        1 => st.brightness = v,
                        2 => st.battery = v,
                        3 => st.network = v,
                        _ => st.bluetooth = v,
                    }
                }
                match slot {
                    0 => {
                        q.render_volume();
                        if let Some(sub) = q.sub_of(Kind::Sound) {
                            q.render_sub(&sub);
                        }
                    }
                    1 => q.render_brightness(),
                    2 => q.render_footer(),
                    _ => q.render_tiles(),
                }
            });
        }
        shell.call("media.list", json!({}), |res| {
            let Some(q) = q() else { return };
            q.st.borrow_mut().players = res.map(|v| players_of(&v)).unwrap_or_default();
            q.render_media();
        });
        self.load_night(|q, available, enabled| {
            {
                let mut st = q.st.borrow_mut();
                st.night_available = available;
                st.night_enabled = enabled;
            }
            q.render_tiles();
        });
    }

    fn load_night(self: &Rc<Self>, done: impl FnOnce(&Rc<Quick>, bool, bool) + 'static) {
        let fallback = truthy(&sh().setting("nightLight.enabled"));
        sh().call("system.nightLight.get", json!({}), move |res| {
            let Some(q) = q() else { return };
            let (available, enabled) = match res {
                // host-services-b: {supported, enabled, temperature, tool}
                Ok(r) => {
                    let ok = if r["supported"].is_null() { r["available"].clone() } else { r["supported"].clone() };
                    (!is_false(&ok), if r["enabled"].is_null() { fallback } else { truthy(&r["enabled"]) })
                }
                // no getter on this host (hyprsunset only exists under Hyprland)
                Err(_) => (false, fallback),
            };
            done(&q, available, enabled);
        });
    }

    /* ---------------------------------------------------------------- rendering */

    fn render_all(self: &Rc<Self>) {
        self.render_media();
        self.render_tiles();
        self.render_brightness();
        self.render_volume();
        self.render_footer();
    }

    fn render_tiles(self: &Rc<Self>) {
        let Some(ui) = self.ui() else { return };
        let (n, b, night) = {
            let st = self.st.borrow();
            (st.network.clone(), st.bluetooth.clone(), (st.night_available, st.night_enabled))
        };
        let shell = sh();
        // Wi-Fi (or a wired "network" tile when the pc has no Wi-Fi)
        if let (Some(wifi), Some(air)) = (ui.tile("wifi"), ui.tile("airplane")) {
            if !n.is_object() {
                wifi.set_hidden(true);
                air.set_hidden(true);
            } else if is_false(&n["wifiAvailable"]) {
                let wired = n["primary"]["type"] == "ethernet";
                wifi.set_hidden(false);
                wifi.set_on(wired);
                wifi.set_glyph(if wired { "ethernet" } else { "network" });
                wifi.set_text(&if wired { t("ethernet") } else { t("notConnected") });
                set_a11y(&wifi.main, &t("network"));
                wifi.el.add_css_class("is-static");
                air.set_hidden(false);
                air.set_on(truthy(&n["airplane"]));
            } else {
                let on = truthy(&n["wifiEnabled"]) && !truthy(&n["airplane"]);
                let connected = on && n["primary"]["type"] == "wifi";
                wifi.el.remove_css_class("is-static");
                wifi.set_hidden(false);
                wifi.set_on(on);
                wifi.set_glyph(if !on {
                    "wifi-off"
                } else if connected {
                    wifi_icon(n["primary"]["strength"].as_f64())
                } else {
                    "wifi"
                });
                let name = s(&n["primary"]["name"]);
                wifi.set_text(&if connected && !name.is_empty() { name.to_owned() } else { t("wifi") });
                set_a11y(&wifi.main, &format!("{}: {}", t("wifi"), if on { t("on") } else { t("off") }));
                air.set_hidden(false);
                air.set_on(truthy(&n["airplane"]));
            }
        }
        // Bluetooth
        if let Some(bt) = ui.tile("bluetooth") {
            if !b.is_object() || is_false(&b["available"]) {
                bt.set_hidden(true);
            } else {
                let powered = truthy(&b["powered"]);
                let conn: Vec<&Value> = b["devices"]
                    .as_array()
                    .map(|d| d.iter().filter(|d| truthy(&d["connected"])).collect())
                    .unwrap_or_default();
                bt.set_hidden(false);
                bt.set_on(powered);
                let name = conn.first().map(|d| s(&d["name"]).to_owned()).unwrap_or_default();
                bt.set_text(&if powered && conn.len() == 1 && !name.is_empty() { name } else { t("bluetooth") });
                set_a11y(&bt.main, &format!("{}: {}", t("bluetooth"), if powered { t("on") } else { t("off") }));
            }
        }
        // Night light
        if let Some(nl) = ui.tile("nightLight") {
            nl.set_hidden(!night.0);
            nl.set_on(night.1);
        }
        // Dark mode and do not disturb (settings)
        if let Some(d) = ui.tile("dark") {
            d.set_on(shell.dark());
        }
        if let Some(d) = ui.tile("dnd") {
            d.set_on(shell.setting("notifications.dnd") == Value::Bool(true));
        }
        self.relayout(&ui);
    }

    /// The grid of visible tiles (3 columns), re-attached only when the visible set changes.
    fn relayout(&self, ui: &Ui) {
        let key = ui.tiles.iter().enumerate().fold(0u32, |k, (i, t)| if t.hidden.get() { k } else { k | (1 << i) });
        if key == self.layout_key.get() {
            return;
        }
        self.layout_key.set(key);
        for t in &ui.tiles {
            if t.el.parent().is_some() {
                ui.grid.remove(&t.el);
            }
        }
        let mut i = 0;
        for t in ui.tiles.iter().filter(|t| !t.hidden.get()) {
            ui.grid.attach(&t.el, i % 3, i / 3, 1, 1);
            i += 1;
        }
    }

    fn render_brightness(self: &Rc<Self>) {
        let Some(ui) = self.ui() else { return };
        let b = self.st.borrow().brightness.clone();
        let avail = truthy(&b["available"]) && b["value"].is_number();
        ui.bright_row.set_visible(avail);
        if avail && !self.bright_th.recently() {
            ui.bright.set_value(b["value"].as_f64().unwrap_or(0.0).round());
        }
    }

    fn render_volume(self: &Rc<Self>) {
        let Some(ui) = self.ui() else { return };
        let a = self.st.borrow().audio.clone();
        let avail = a.is_object() && !is_false(&a["available"]) && a["volume"].is_number();
        ui.vol_row.set_visible(avail);
        if !avail {
            return;
        }
        let touched = self.vol_th.recently();
        let volume = a["volume"].as_f64().unwrap_or(0.0);
        if !touched {
            ui.vol.set_value(volume.round());
        }
        let v = if touched { ui.vol.value() } else { volume };
        let muted = truthy(&a["muted"]);
        self.show_mute(&ui, v, muted);
        let sinks = a["sinks"].as_array().is_some_and(|s| !s.is_empty());
        ui.out.set_visible(sinks);
    }

    fn show_mute(&self, ui: &Ui, v: f64, muted: bool) {
        set_btn_icon(&ui.mute, volume_icon(v, muted), 16);
        let label = if muted { t("unmute") } else { t("mute") };
        ui.mute.set_tooltip_text(Some(&label));
        set_a11y(&ui.mute, &label);
        if muted {
            ui.vol_row.add_css_class("is-muted");
        } else {
            ui.vol_row.remove_css_class("is-muted");
        }
    }

    fn render_footer(self: &Rc<Self>) {
        let Some(ui) = self.ui() else { return };
        let b = self.st.borrow().battery.clone();
        let percent = b["percent"].as_f64();
        let present = truthy(&b["present"]) && percent.is_some();
        ui.battery.set_visible(present);
        let Some(p) = percent.filter(|_| present) else { return };
        let state = s(&b["state"]);
        let on_ac = truthy(&b["onAC"]);
        let charging = state == "charging" || (on_ac && state != "fully-charged" && state != "full");
        let full = on_ac && !charging;
        let label = if charging {
            tp("batteryCharging", &pct(p))
        } else if full {
            tp("batteryFull", &pct(p))
        } else {
            tp("battery", &pct(p))
        };
        self.battery_glyph.set((p, charging || full));
        ui.battery_draw.queue_draw();
        ui.battery_text.set_text(&pct(p));
        ui.battery.set_tooltip_text(Some(&label));
        set_a11y(&ui.battery, &label);
    }

    /* ---------------------------------------------------------------- media */

    fn render_media(self: &Rc<Self>) {
        let Some(ui) = self.ui() else { return };
        let box_ = ui.media.clone();
        clear_box(&box_);
        let status = |p: &Value| s(&p["status"]).to_ascii_lowercase();
        let players: Vec<Value> = self
            .st
            .borrow()
            .players
            .iter()
            .filter(|p| p.is_object() && (truthy(&p["title"]) || status(p) == "playing" || status(p) == "paused"))
            .cloned()
            .collect();
        if players.is_empty() {
            box_.set_visible(false);
            return;
        }
        let index = {
            let mut st = self.st.borrow_mut();
            if st.player_index >= players.len() {
                st.player_index = 0;
            }
            // prefer the playing one when the user has not picked
            if !st.picked
                && let Some(i) = players.iter().position(|p| status(p) == "playing")
            {
                st.player_index = i;
            }
            st.player_index
        };
        let p = players[index].clone();
        let count = players.len();
        let playing = status(&p) == "playing";
        let id = p["id"].clone();

        // Text above, picture below (Robin, 13-11: "tekst boven plaatje onder is duidelijker"): the player, the
        // title and artist over the full width, then the picture over the full width in its own proportions.
        let body = vbox(0, &["fo-media__body"]);
        let top = hbox(8, &["fo-media__top"]);
        let icon = app_icon(&player_icon(&p), 16);
        top.append(&icon);
        top.append(&ell(lbl(s(&p["identity"]), &["fo-media__app", "q-caption"])));
        body.append(&top);
        let text = vbox(2, &["fo-media__text"]);
        text.set_hexpand(true);
        let title = if truthy(&p["title"]) { s(&p["title"]) } else { s(&p["identity"]) };
        let tl = ell(lbl(title, &["fo-media__title", "q-strong"]));
        if title.chars().count() > 32 {
            tl.set_tooltip_text(Some(title));
        }
        text.append(&tl);
        let artist = if truthy(&p["artist"]) {
            s(&p["artist"]).to_owned()
        } else if playing {
            t("playing")
        } else {
            t("paused")
        };
        text.append(&ell(lbl(&artist, &["fo-media__artist", "q-caption", "q-text-2"])));
        body.append(&text);
        // art (local only: never fetched from the network); nothing while there is none
        if let Some(art) = self.art_widget(&p) {
            body.append(&art);
        }
        box_.append(&body);

        // controls (centred) with the pager on the right
        let controls = hbox(4, &["fo-media__controls"]);
        controls.set_halign(gtk::Align::Center);
        let btn = |action: &'static str, icon: &str, label: &str, enabled: bool, cls: &str| {
            let b = icon_btn(icon, label, 40, 16, &[cls]);
            b.set_sensitive(enabled);
            let id = id.clone();
            b.connect_clicked(move |_| {
                sh().call("media.control", json!({"id": id, "action": action}), |res| {
                    if let (Err(e), Some(q)) = (res, q()) {
                        q.flash(&err_text(&e), false);
                    }
                });
            });
            b
        };
        controls.append(&btn("previous", "previous", &t("previous"), !is_false(&p["canPrevious"]), "fo-media__prev"));
        controls.append(&btn(
            "playPause",
            if playing { "pause" } else { "play" },
            &if playing { t("pause") } else { t("play") },
            if playing { !is_false(&p["canPause"]) } else { !is_false(&p["canPlay"]) },
            "fo-media__play",
        ));
        controls.append(&btn("next", "next", &t("next2"), !is_false(&p["canNext"]), "fo-media__next"));
        let row = gtk::Overlay::new();
        row.set_child(Some(&controls));
        if count > 1 {
            let pager = hbox(2, &["fo-media__pager"]);
            pager.set_halign(gtk::Align::End);
            pager.set_valign(gtk::Align::Center);
            let prev = icon_btn("chevron-left", &t("otherPlayer"), 24, 10, &[]);
            prev.connect_clicked(move |_| {
                if let Some(q) = q() {
                    {
                        let mut st = q.st.borrow_mut();
                        st.picked = true;
                        st.player_index = (st.player_index + count - 1) % count;
                    }
                    q.render_media();
                }
            });
            let next = icon_btn("chevron-right", &t("otherPlayer"), 24, 10, &[]);
            next.connect_clicked(move |_| {
                if let Some(q) = q() {
                    {
                        let mut st = q.st.borrow_mut();
                        st.picked = true;
                        st.player_index = (st.player_index + 1) % count;
                    }
                    q.render_media();
                }
            });
            pager.append(&prev);
            pager.append(&lbl(&format!("{}/{}", index + 1, count), &["q-caption", "q-text-2", "q-tabular"]));
            pager.append(&next);
            row.add_overlay(&pager);
        }
        box_.append(&row);
        box_.set_visible(true);
    }

    fn art_widget(self: &Rc<Self>, p: &Value) -> Option<gtk::Widget> {
        let path = if truthy(&p["artPath"]) {
            Some(s(&p["artPath"]).to_owned())
        } else {
            let url = s(&p["artUrl"]);
            if let Some(rest) = url.strip_prefix("file://") {
                let path = rest.find('/').map(|i| &rest[i..]).unwrap_or(rest);
                Some(percent_decode(path))
            } else if url.starts_with('/') {
                Some(url.to_owned())
            } else {
                None
            }
        };
        let path = path.filter(|p| !p.is_empty())?;
        let cached = self.art.borrow().get(&path).cloned();
        match cached {
            Some(Some(texture)) => {
                // Full width, its own proportions between 1:1 and 2.4:1, 110 to 180 px high on the ~300 px card.
                let ratio = (texture.width().max(1) as f64 / texture.height().max(1) as f64).clamp(1.0, 2.4);
                let pic = gtk::Picture::for_paintable(&texture);
                pic.set_content_fit(gtk::ContentFit::Cover);
                pic.set_can_shrink(true);
                pic.set_hexpand(true);
                pic.set_size_request(-1, ((300.0 / ratio).round() as i32).clamp(110, 180));
                pic.set_overflow(gtk::Overflow::Hidden);
                pic.add_css_class("fo-media__art");
                Some(pic.upcast())
            }
            Some(None) => None,
            None => {
                self.art.borrow_mut().insert(path.clone(), None);
                let backend = sh().backend.clone();
                let query = format!("/file?path={}", percent_encode(&path));
                glib::spawn_future_local(async move {
                    let bytes = gtk::gio::spawn_blocking(move || backend.get(&query)).await;
                    let texture = match bytes {
                        Ok(Ok(b)) => crate::icons::texture(&b),
                        _ => None,
                    };
                    if let (Some(q), Some(tex)) = (q(), texture) {
                        {
                            let mut art = q.art.borrow_mut();
                            if art.len() > 24 {
                                art.clear();
                            }
                            art.insert(path, Some(tex));
                        }
                        q.render_media();
                    }
                });
                None
            }
        }
    }

    /* ---------------------------------------------------------------- notes */

    /// Short inline note on the visible page (main page or the open sub-page).
    fn flash(self: &Rc<Self>, message: &str, info: bool) {
        let Some(ui) = self.ui() else { return };
        let gen_ = self.notice_gen.get() + 1;
        self.notice_gen.set(gen_);
        let note = hbox(8, &["fo-qs__notice"]);
        if info {
            note.add_css_class("is-info");
        }
        let icon = glyph(if info { "info" } else { "warning" }, 16);
        icon.add_css_class("q-notice-icon");
        icon.set_valign(gtk::Align::Start);
        note.append(&icon);
        let text = wrapped(lbl(message, &["q-caption"]));
        text.set_hexpand(true);
        note.append(&text);
        let slot = match self.sub.borrow().as_ref() {
            Some(sub) => sub.notice.clone(),
            None => ui.notice.clone(),
        };
        clear_box(&slot);
        slot.append(&note);
        slot.set_visible(true);
        let weak = slot.downgrade();
        glib::timeout_add_local_once(Duration::from_secs(5), move || {
            if let (Some(q), Some(slot)) = (q(), weak.upgrade())
                && q.notice_gen.get() == gen_
            {
                clear_box(&slot);
                slot.set_visible(false);
            }
        });
    }

    /// Run a change optimistically: on an error, or in test mode (nothing really changed and the host will not
    /// report a change), undo the optimistic state and say so instead of pretending.
    fn guarded(
        self: &Rc<Self>,
        method: &str,
        args: Value,
        revert: impl FnOnce() + 'static,
        then: impl FnOnce(Option<Value>) + 'static,
    ) {
        sh().call(method, args, move |res| {
            let Some(q) = q() else { return };
            match res {
                Ok(r) => {
                    if is_dry_run(&r) {
                        revert();
                        q.flash(&t("dryRun"), true);
                    }
                    then(Some(r));
                }
                Err(e) => {
                    revert();
                    q.flash(&err_text(&e), false);
                    then(None);
                }
            }
        });
    }

    /// After undoing an optimistic toggle: the tiles and an open Wi-Fi/Bluetooth sub-page show it again.
    fn rerender(self: &Rc<Self>) {
        self.render_tiles();
        let sub = self.sub.borrow().clone();
        if let Some(sub) = sub
            && sub.kind != Kind::Sound
        {
            self.render_sub(&sub);
        }
    }

    /* ---------------------------------------------------------------- actions */

    fn tile_clicked(self: &Rc<Self>, id: &str) {
        let Some(ui) = self.ui() else { return };
        let Some(tile) = ui.tile(id) else { return };
        if tile.el.has_css_class("is-disabled") {
            return;
        }
        let on = !tile.is_on();
        match id {
            "wifi" => self.toggle_wifi(on),
            "bluetooth" => self.toggle_bluetooth(on),
            "airplane" => self.toggle_airplane(on),
            "nightLight" => self.toggle_night(on),
            "dark" => sh().set_setting("theme.mode", json!(if on { "dark" } else { "light" })),
            "dnd" => {
                sh().set_setting("notifications.dnd", json!(on));
                tile.set_on(on);
            }
            _ => {}
        }
    }

    fn toggle_wifi(self: &Rc<Self>, on: bool) {
        let prev = self.st.borrow().network.clone();
        if is_false(&prev["wifiAvailable"]) {
            open_settings(Some("network"));
            return;
        }
        let mut next = with(&prev, "wifiEnabled", json!(on));
        if !on {
            next = with(&next, "primary", json!({"type": "none", "name": "", "strength": null}));
        }
        self.st.borrow_mut().network = next;
        self.render_tiles();
        let me = self.clone();
        self.guarded(
            "system.network.setWifi",
            json!({"enabled": on}),
            move || {
                me.st.borrow_mut().network = prev;
                me.rerender();
            },
            |_| {},
        );
    }

    fn toggle_bluetooth(self: &Rc<Self>, on: bool) {
        let prev = self.st.borrow().bluetooth.clone();
        self.st.borrow_mut().bluetooth = with(&prev, "powered", json!(on));
        self.render_tiles();
        let me = self.clone();
        self.guarded(
            "system.bluetooth.setPowered",
            json!({"powered": on}),
            move || {
                me.st.borrow_mut().bluetooth = prev;
                me.rerender();
            },
            |_| {},
        );
    }

    fn toggle_airplane(self: &Rc<Self>, on: bool) {
        let prev = self.st.borrow().network.clone();
        self.st.borrow_mut().network = with(&prev, "airplane", json!(on));
        self.render_tiles();
        let me = self.clone();
        self.guarded(
            "system.network.setAirplane",
            json!({"enabled": on}),
            move || {
                me.st.borrow_mut().network = prev;
                me.rerender();
            },
            |_| {},
        );
    }

    fn toggle_night(self: &Rc<Self>, on: bool) {
        let prev = {
            let st = self.st.borrow();
            (st.night_available, st.night_enabled)
        };
        self.st.borrow_mut().night_enabled = on;
        self.render_tiles();
        let temp = sh().setting("nightLight.temperature");
        let temp = if temp.is_number() { temp } else { json!(4500) };
        let me = self.clone();
        self.guarded(
            "system.nightLight.set",
            json!({"enabled": on, "temperature": temp}),
            move || {
                {
                    let mut st = me.st.borrow_mut();
                    st.night_available = prev.0;
                    st.night_enabled = prev.1;
                }
                me.render_tiles();
            },
            move |res| match res {
                Some(r) if is_dry_run(&r) => {}
                Some(_) => sh().set_setting("nightLight.enabled", json!(on)),
                None => {
                    // not supported here after all: hide the tile gracefully
                    if prev.0
                        && let Some(q) = q()
                    {
                        q.load_night(|q, available, enabled| {
                            if !available {
                                {
                                    let mut st = q.st.borrow_mut();
                                    st.night_available = false;
                                    st.night_enabled = enabled;
                                }
                                q.render_tiles();
                            }
                        });
                    }
                }
            },
        );
    }

    fn toggle_mute(self: &Rc<Self>) {
        let a = self.st.borrow().audio.clone();
        if !a.is_object() {
            return;
        }
        let muted = !truthy(&a["muted"]);
        self.st.borrow_mut().audio = with(&a, "muted", json!(muted));
        self.render_volume();
        let me = self.clone();
        self.guarded(
            "system.audio.set",
            json!({"muted": muted}),
            move || {
                me.st.borrow_mut().audio = a;
                me.render_volume();
            },
            |_| {},
        );
    }

    /// A slider moved by the user: optimistic state, throttled calls, and a final call that checks test mode.
    fn slider_input(self: &Rc<Self>, which: Which, v: f64) {
        let Some(ui) = self.ui() else { return };
        let v = v.round().clamp(0.0, 100.0);
        let th = self.throttle(which);
        th.touched.set(Some(Instant::now()));
        let args = match which {
            Which::Bright => {
                let mut st = self.st.borrow_mut();
                st.brightness = with(&st.brightness, "value", json!(v as i64));
                json!({"value": v as i64})
            }
            Which::Vol => {
                // like Windows: moving the slider while muted turns the sound back on
                let was_muted = truthy(&self.st.borrow().audio["muted"]);
                let muted = was_muted && v <= 0.0;
                {
                    let mut st = self.st.borrow_mut();
                    st.audio = with(&with(&st.audio, "volume", json!(v as i64)), "muted", json!(muted));
                }
                self.show_mute(&ui, v, muted);
                let mut args = json!({"volume": v as i64});
                if was_muted && !muted {
                    args["muted"] = json!(false);
                }
                args
            }
        };
        let ms = if which == Which::Vol { 80 } else { 90 };
        *th.last.borrow_mut() = Some(args.clone());
        if th.timer.borrow().is_none() {
            self.send_slider(which, args, false);
            let id = glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                let Some(q) = q() else { return };
                let th = q.throttle(which);
                th.timer.borrow_mut().take();
                let pending = th.pending.borrow_mut().take();
                if let Some(p) = pending {
                    q.send_slider(which, p, false);
                }
            });
            *th.timer.borrow_mut() = Some(id);
        } else {
            *th.pending.borrow_mut() = Some(args);
        }
        let old = th.final_timer.borrow_mut().take();
        if let Some(id) = old {
            id.remove();
        }
        let id = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            let Some(q) = q() else { return };
            let th = q.throttle(which);
            th.final_timer.borrow_mut().take();
            let last = th.last.borrow_mut().take();
            if let Some(a) = last {
                q.send_slider(which, a, true);
            }
        });
        *th.final_timer.borrow_mut() = Some(id);
    }

    fn send_slider(self: &Rc<Self>, which: Which, args: Value, fin: bool) {
        let method = match which {
            Which::Bright => "system.brightness.set",
            Which::Vol => "system.audio.set",
        };
        sh().call(method, args, move |res| {
            let Some(q) = q() else { return };
            match res {
                Ok(r) if fin && is_dry_run(&r) => q.restore_after_dry_run(which),
                Err(e) if fin => q.flash(&err_text(&e), false),
                _ => {}
            }
        });
    }

    /// After a slider was released in test mode: show the real value again.
    fn restore_after_dry_run(self: &Rc<Self>, which: Which) {
        self.flash(&t("dryRun"), true);
        let method = match which {
            Which::Bright => "system.brightness.get",
            Which::Vol => "system.audio.get",
        };
        sh().call(method, json!({}), move |res| {
            let Some(q) = q() else { return };
            q.throttle(which).touched.set(None);
            if let Ok(v) = res
                && !v.is_null()
            {
                match which {
                    Which::Bright => {
                        q.st.borrow_mut().brightness = v;
                        q.render_brightness();
                    }
                    Which::Vol => {
                        q.st.borrow_mut().audio = v;
                        q.render_volume();
                    }
                }
            }
        });
    }

    /* ---------------------------------------------------------------- sub-pages */

    fn build_sub(self: &Rc<Self>, kind: Kind) -> Rc<Sub> {
        let n = self.sub_seq.get() + 1;
        self.sub_seq.set(n);
        let (title, more, page) = match kind {
            Kind::Wifi => (t("wifi"), t("moreWifi"), "wifi"),
            Kind::Bluetooth => (t("bluetooth"), t("moreBt"), "bluetooth"),
            Kind::Sound => (t("output"), t("moreSound"), "sound"),
        };
        let el = vbox(0, &["fo-qs__page", "fo-sub"]);
        let header = hbox(4, &["fo-sub__header"]);
        let back = icon_btn("back", &t("back"), 32, 16, &["fo-sub__back"]);
        back.connect_clicked(|_| {
            if let Some(q) = q() {
                q.close_sub(false);
            }
        });
        let title_l = ell(lbl(&title, &["fo-sub__title", "q-strong"]));
        header.append(&back);
        header.append(&title_l);
        let toggle = if kind == Kind::Sound {
            None
        } else {
            let sw = gtk::Switch::new();
            sw.add_css_class("q-switch");
            sw.set_valign(gtk::Align::Center);
            set_a11y(&sw, &title);
            header.append(&sw);
            Some(sw)
        };
        el.append(&header);
        let notice = vbox(0, &["fo-sub__notice"]);
        notice.set_visible(false);
        el.append(&notice);
        let progress = if kind == Kind::Wifi {
            let p = indeterminate(&t("scanning"));
            p.add_css_class("fo-sub__progress");
            p.set_visible(false);
            el.append(&p);
            Some(p)
        } else {
            None
        };
        let list = vbox(2, &["fo-sub__list"]);
        if kind == Kind::Sound {
            set_a11y(&list, &t("output"));
        }
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_propagate_natural_height(true);
        scroller.set_min_content_height(96);
        scroller.set_max_content_height(sub_list_max());
        scroller.set_child(Some(&list));
        el.append(&scroller);
        let footer = hbox(0, &["fo-sub__footer"]);
        let more_b = button(&["fo-sub__more"]);
        more_b.set_child(Some(&lbl(&more, &[])));
        more_b.connect_clicked(move |_| open_settings(Some(page)));
        footer.append(&more_b);
        el.append(&footer);

        let sub = Rc::new(Sub {
            kind,
            name: format!("sub-{n}"),
            el,
            back,
            notice,
            list,
            toggle,
            progress,
            updating: Cell::new(false),
            expanded: RefCell::new(None),
            rows: RefCell::new(HashMap::new()),
            heads: RefCell::new(HashMap::new()),
            focus_after: RefCell::new(None),
            entry: RefCell::new(None),
            focus_entry: Cell::new(false),
            busy: RefCell::new(HashMap::new()),
            errors: RefCell::new(HashMap::new()),
            not_ready: Cell::new(false),
            resetting: Cell::new(false),
        });
        if let Some(sw) = &sub.toggle {
            let weak = Rc::downgrade(&sub);
            sw.connect_state_set(move |_, on| {
                if let (Some(q), Some(sub)) = (q(), weak.upgrade())
                    && !sub.updating.get()
                {
                    match sub.kind {
                        Kind::Wifi => q.toggle_wifi(on),
                        _ => q.toggle_bluetooth(on),
                    }
                    let weak = Rc::downgrade(&sub);
                    glib::idle_add_local_once(move || {
                        if let (Some(q), Some(sub)) = (self::q(), weak.upgrade()) {
                            q.render_sub(&sub);
                        }
                    });
                }
                glib::Propagation::Proceed
            });
        }
        sub
    }

    fn open_sub(self: &Rc<Self>, kind: Kind) {
        let Some(ui) = self.ui() else { return };
        let sub = self.build_sub(kind);
        let prev = self.sub.replace(Some(sub.clone()));
        ui.stack.add_named(&sub.el, Some(&sub.name));
        self.render_sub(&sub);
        ui.stack.set_visible_child_full(&sub.name, gtk::StackTransitionType::SlideLeft);
        if let Some(prev) = prev {
            remove_page_later(&prev.el);
        }
        let back = sub.back.clone();
        glib::idle_add_local_once(move || {
            back.grab_focus();
        });
        match kind {
            Kind::Wifi => self.refresh_wifi_list(true),
            Kind::Bluetooth => {
                let weak = Rc::downgrade(&sub);
                sh().call("system.bluetooth.get", json!({}), move |res| {
                    let Some(q) = q() else { return };
                    if let Ok(b) = res
                        && !b.is_null()
                    {
                        q.st.borrow_mut().bluetooth = b;
                        if let Some(sub) = weak.upgrade()
                            && q.is_current(&sub)
                        {
                            q.render_bt_event(&sub);
                        }
                        q.render_tiles();
                    }
                });
            }
            Kind::Sound => {}
        }
    }

    fn close_sub(self: &Rc<Self>, instant: bool) {
        let Some(sub) = self.sub.take() else { return };
        let Some(ui) = self.ui() else {
            remove_page(&sub.el);
            return;
        };
        ui.stack.set_visible_child_full(
            "main",
            if instant { gtk::StackTransitionType::None } else { gtk::StackTransitionType::SlideRight },
        );
        if instant {
            remove_page(&sub.el);
            return;
        }
        remove_page_later(&sub.el);
        let target: Option<gtk::Widget> = match sub.kind {
            Kind::Wifi => ui.tile("wifi").and_then(|t| t.more.clone()).map(|b| b.upcast()),
            Kind::Bluetooth => ui.tile("bluetooth").and_then(|t| t.more.clone()).map(|b| b.upcast()),
            Kind::Sound => Some(ui.out.clone().upcast()),
        };
        if let Some(w) = target {
            glib::idle_add_local_once(move || {
                w.grab_focus();
            });
        }
    }

    fn render_sub(self: &Rc<Self>, sub: &Rc<Sub>) {
        match sub.kind {
            Kind::Wifi => self.render_wifi(sub),
            Kind::Bluetooth => self.render_bt(sub),
            Kind::Sound => self.render_sound(sub),
        }
    }

    /// Before a list is rebuilt: remember which row head (or the password box) has the focus.
    fn keep_focus(&self, sub: &Sub) {
        if sub.focus_after.borrow().is_none() {
            let key = sub.heads.borrow().iter().find(|(_, b)| b.has_focus()).map(|(k, _)| k.clone());
            *sub.focus_after.borrow_mut() = key;
        }
        let entry_focused = sub
            .entry
            .borrow()
            .as_ref()
            .is_some_and(|e| e.state_flags().contains(gtk::StateFlags::FOCUS_WITHIN));
        if entry_focused {
            sub.focus_entry.set(true);
        }
        sub.heads.borrow_mut().clear();
        *sub.entry.borrow_mut() = None;
    }

    fn restore_focus(&self, sub: &Sub) {
        let want = sub.focus_after.borrow_mut().take();
        let entry = if sub.focus_entry.replace(false) { sub.entry.borrow().clone() } else { None };
        let head = want.and_then(|k| sub.heads.borrow().get(&k).cloned());
        if entry.is_none() && head.is_none() {
            return;
        }
        glib::idle_add_local_once(move || {
            if let Some(e) = entry {
                e.grab_focus();
                e.set_position(-1);
            } else if let Some(h) = head {
                h.grab_focus();
            }
        });
    }

    /* ---------------------------------------------------------------- Wi-Fi */

    fn refresh_wifi_list(self: &Rc<Self>, rescan: bool) {
        let Some(page) = self.sub_of(Kind::Wifi) else { return };
        if rescan {
            self.st.borrow_mut().scanning = true;
            self.render_sub(&page);
        }
        let weak = Rc::downgrade(&page);
        sh().call("system.network.wifiList", json!({"rescan": rescan}), move |res| {
            let Some(q) = q() else { return };
            match res {
                Ok(list) => q.st.borrow_mut().wifi_list = Some(list.as_array().cloned().unwrap_or_default()),
                Err(e) => {
                    if rescan {
                        q.flash(&err_text(&e), false);
                    }
                }
            }
            q.st.borrow_mut().scanning = false;
            if let Some(page) = weak.upgrade()
                && q.is_current(&page)
            {
                q.render_sub(&page);
            }
        });
    }

    fn render_wifi(self: &Rc<Self>, sub: &Rc<Sub>) {
        let (n, nets, scanning) = {
            let st = self.st.borrow();
            (st.network.clone(), st.wifi_list.clone(), st.scanning)
        };
        let airplane = truthy(&n["airplane"]);
        let enabled = truthy(&n["wifiEnabled"]);
        if let Some(sw) = &sub.toggle {
            sub.updating.set(true);
            sw.set_active(enabled && !airplane);
            sw.set_sensitive(!airplane);
            sub.updating.set(false);
        }
        if let Some(p) = &sub.progress {
            p.set_visible(scanning);
        }
        self.keep_focus(sub);
        clear_box(&sub.list);
        if airplane {
            sub.list.append(&empty_msg(&t("airplaneOn"), Some(&t("airplaneHint")), Some("airplane")));
            return;
        }
        if !enabled {
            sub.list.append(&empty_msg(&t("wifiOff"), Some(&t("wifiOffHint")), Some("wifi-off")));
            return;
        }
        let list = nets.clone().unwrap_or_default();
        if list.is_empty() {
            if scanning || nets.is_none() {
                let l = wrapped(lbl(&t("scanning"), &["fo-sub__scanning", "q-caption", "q-text-2"]));
                l.set_xalign(0.5);
                l.set_justify(gtk::Justification::Center);
                sub.list.append(&l);
            } else {
                sub.list.append(&empty_msg(&t("noNetworks"), None, Some("wifi")));
            }
            return;
        }
        for net in &list {
            let row = self.wifi_row(sub, net);
            sub.list.append(&row);
        }
        self.restore_focus(sub);
    }

    fn wifi_row(self: &Rc<Self>, sub: &Rc<Sub>, n: &Value) -> gtk::Box {
        let ssid = s(&n["ssid"]).to_owned();
        let secure = truthy(&n["secure"]);
        let saved = truthy(&n["saved"]);
        let active = truthy(&n["active"]);
        let rs = sub.rows.borrow_mut().entry(ssid.clone()).or_default().clone();
        let is_open = sub.expanded.borrow().as_deref() == Some(ssid.as_str());

        let head = button(&["fo-net__head"]);
        let hb = hbox(12, &[]);
        hb.append(&wifi_glyph(n["strength"].as_f64(), secure));
        let text = vbox(0, &["fo-net__text"]);
        text.set_hexpand(true);
        text.append(&ell(lbl(&ssid, &["fo-net__name"])));
        let status = if rs.stage == Stage::Connecting {
            t("connecting")
        } else if active {
            if secure { t("connectedSecure") } else { t("connectedOpen") }
        } else if !secure {
            t("open")
        } else if saved {
            t("saved")
        } else {
            t("secure")
        };
        text.append(&ell(lbl(&status, &["fo-net__status", "q-caption", "q-text-2"])));
        hb.append(&text);
        head.set_child(Some(&hb));
        head.update_state(&[gtk::accessible::State::Expanded(Some(is_open))]);
        let row = vbox(0, &["fo-net"]);
        if is_open {
            row.add_css_class("is-expanded");
        }
        if active {
            row.add_css_class("is-active");
        }
        row.append(&head);
        sub.heads.borrow_mut().insert(ssid.clone(), head.clone());
        let weak = Rc::downgrade(sub);
        let key = ssid.clone();
        head.connect_clicked(move |_| {
            let (Some(q), Some(sub)) = (q(), weak.upgrade()) else { return };
            let open = sub.expanded.borrow().as_deref() == Some(key.as_str());
            *sub.expanded.borrow_mut() = if open { None } else { Some(key.clone()) };
            if !open
                && let Some(r) = sub.rows.borrow_mut().get_mut(&key)
                && r.stage != Stage::Connecting
            {
                r.stage = Stage::Idle;
            }
            *sub.focus_after.borrow_mut() = Some(key.clone());
            q.render_sub(&sub);
        });
        if !is_open {
            return row;
        }

        let body = vbox(8, &["fo-net__body"]);
        if active {
            let actions = hbox(8, &["fo-net__actions"]);
            let props = button(&["fo-sub__link"]);
            props.set_child(Some(&ell(lbl(&t("properties"), &[]))));
            props.connect_clicked(|_| open_settings(Some("wifi")));
            actions.append(&props);
            actions.append(&grow());
            let disc = text_btn(&t("disconnect"), false);
            disc.connect_clicked(|b| {
                b.set_sensitive(false);
                set_btn_label(b, &t("disconnecting"));
                if let Some(q) = q() {
                    let me = q.clone();
                    q.guarded("system.network.disconnect", json!({}), || {}, move |_| me.refresh_wifi_list(false));
                }
            });
            actions.append(&disc);
            body.append(&actions);
        } else if rs.stage == Stage::Password {
            body.append(&wrapped(lbl(&t("enterKey"), &["q-caption", "q-text-2", "fo-net__prompt"])));
            let pw = gtk::PasswordEntry::builder().placeholder_text(t("password")).show_peek_icon(true).build();
            pw.add_css_class("q-entry");
            set_a11y(&pw, &t("enterKey"));
            pw.set_text(&rs.draft);
            body.append(&pw);
            if let Some(err) = &rs.error {
                pw.add_css_class("is-error");
                body.append(&wrapped(lbl(err, &["fo-net__error", "q-caption"])));
            }
            let w = Rc::downgrade(sub);
            let k = ssid.clone();
            pw.connect_changed(move |e| {
                if let Some(sub) = w.upgrade()
                    && let Some(r) = sub.rows.borrow_mut().get_mut(&k)
                {
                    r.draft = e.text().to_string();
                }
            });
            let go = {
                let w = Rc::downgrade(sub);
                let pw = pw.downgrade();
                let n = n.clone();
                Rc::new(move || {
                    let (Some(q), Some(sub), Some(pw)) = (q(), w.upgrade(), pw.upgrade()) else { return };
                    let v = pw.text().to_string();
                    if v.is_empty() {
                        pw.grab_focus();
                        return;
                    }
                    q.wifi_connect(&sub, &n, Some(v));
                })
            };
            let g = go.clone();
            pw.connect_activate(move |_| g());
            let actions = hbox(8, &["fo-net__actions", "fo-net__actions--split"]);
            actions.set_homogeneous(true);
            let next = text_btn(&t("next"), true);
            let g = go.clone();
            next.connect_clicked(move |_| g());
            let cancel = text_btn(&t("cancel"), false);
            let w = Rc::downgrade(sub);
            let k = ssid.clone();
            cancel.connect_clicked(move |_| {
                let (Some(q), Some(sub)) = (q(), w.upgrade()) else { return };
                if let Some(r) = sub.rows.borrow_mut().get_mut(&k) {
                    r.stage = Stage::Idle;
                    r.error = None;
                }
                *sub.focus_after.borrow_mut() = Some(k.clone());
                q.render_sub(&sub);
            });
            actions.append(&next);
            actions.append(&cancel);
            body.append(&actions);
            *sub.entry.borrow_mut() = Some(pw);
            sub.focus_entry.set(true);
        } else if rs.stage == Stage::Connecting {
            body.append(&lbl(&t("connecting"), &["q-caption", "q-text-2"]));
            body.append(&indeterminate(&t("connecting")));
        } else {
            if !secure {
                let warn = hbox(6, &["fo-net__warn", "q-caption"]);
                let i = glyph("info", 12);
                i.set_valign(gtk::Align::Start);
                i.set_margin_top(2);
                warn.append(&i);
                let l = wrapped(lbl(&t("openWarning"), &[]));
                l.set_hexpand(true);
                warn.append(&l);
                body.append(&warn);
            }
            if let Some(err) = &rs.error {
                body.append(&wrapped(lbl(err, &["fo-net__error", "q-caption"])));
            }
            let actions = hbox(8, &["fo-net__actions"]);
            let auto = gtk::CheckButton::with_label(&t("autoConnect"));
            auto.add_css_class("q-check");
            auto.set_active(rs.auto);
            let w = Rc::downgrade(sub);
            let k = ssid.clone();
            auto.connect_toggled(move |c| {
                if let Some(sub) = w.upgrade()
                    && let Some(r) = sub.rows.borrow_mut().get_mut(&k)
                {
                    r.auto = c.is_active();
                }
            });
            actions.append(&auto);
            actions.append(&grow());
            let connect = text_btn(&t("connect"), true);
            let w = Rc::downgrade(sub);
            let k = ssid.clone();
            let net = n.clone();
            connect.connect_clicked(move |_| {
                let (Some(q), Some(sub)) = (q(), w.upgrade()) else { return };
                if secure && !saved {
                    if let Some(r) = sub.rows.borrow_mut().get_mut(&k) {
                        r.stage = Stage::Password;
                        r.error = None;
                    }
                    q.render_sub(&sub);
                } else {
                    q.wifi_connect(&sub, &net, None);
                }
            });
            actions.append(&connect);
            body.append(&actions);
        }
        row.append(&body);
        row
    }

    fn wifi_connect(self: &Rc<Self>, sub: &Rc<Sub>, n: &Value, password: Option<String>) {
        let ssid = s(&n["ssid"]).to_owned();
        let secure = truthy(&n["secure"]);
        {
            let mut rows = sub.rows.borrow_mut();
            let r = rows.entry(ssid.clone()).or_default();
            r.stage = Stage::Connecting;
            r.error = None;
        }
        self.render_sub(sub);
        let had_password = password.is_some();
        let mut args = json!({"ssid": ssid});
        if let Some(p) = password {
            args["password"] = json!(p);
        }
        let weak = Rc::downgrade(sub);
        sh().call("system.network.connect", args, move |res| {
            let (Some(q), Some(sub)) = (q(), weak.upgrade()) else { return };
            let r = match res {
                Ok(r) => r,
                Err(e) => json!({"ok": false, "error": e}),
            };
            let set = |stage: Stage, error: Option<String>| {
                let mut rows = sub.rows.borrow_mut();
                let x = rows.entry(ssid.clone()).or_default();
                x.stage = stage;
                x.error = error;
                if stage == Stage::Idle {
                    x.draft.clear();
                }
            };
            if is_dry_run(&r) {
                // test mode: nothing was connected
                set(Stage::Idle, None);
                q.flash(&t("dryRun"), true);
                q.render_sub(&sub);
                return;
            }
            if !r.is_null() && !is_false(&r["ok"]) {
                set(Stage::Idle, None);
                *sub.expanded.borrow_mut() = None;
                q.refresh_wifi_list(false);
                return;
            }
            let msg = r["error"]
                .as_str()
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| t("connectFailed"));
            // only the host's codes say it (never the message text): needs_password / wrong_password, and bad_args
            // to a call with a password when the name itself passes the host's check (1-32 UTF-8 bytes)
            let code = s(&r["code"]);
            let ssid_ok = !ssid.is_empty() && ssid.len() <= 32;
            let needs_key = secure
                && (code == "needs_password"
                    || code == "wrong_password"
                    || (had_password && code == "bad_args" && ssid_ok));
            set(if needs_key { Stage::Password } else { Stage::Idle }, Some(msg));
            q.render_sub(&sub);
        });
    }

    /* ---------------------------------------------------------------- Bluetooth */

    /// A render after the host's new state: its own verdict (`stuck`) replaces ours.
    fn render_bt_event(self: &Rc<Self>, sub: &Rc<Sub>) {
        if !self.st.borrow().bluetooth["stuck"].is_null() && sub.busy.borrow().is_empty() {
            sub.not_ready.set(false);
        }
        self.render_bt(sub);
    }

    fn render_bt(self: &Rc<Self>, sub: &Rc<Sub>) {
        let b = self.st.borrow().bluetooth.clone();
        let stuck = truthy(&b["stuck"]) || sub.not_ready.get();
        let resetting = sub.resetting.get();
        let powered = truthy(&b["powered"]);
        if let Some(sw) = &sub.toggle {
            sub.updating.set(true);
            sw.set_active(powered);
            sw.set_sensitive(!resetting);
            sub.updating.set(false);
        }
        self.keep_focus(sub);
        clear_box(&sub.list);
        if stuck || resetting {
            sub.list.append(&self.bt_stuck_notice(sub, resetting));
        }
        if !powered {
            if !stuck && !resetting {
                sub.list.append(&empty_msg(&t("btOff"), Some(&t("btOffHint")), Some("bluetooth")));
            }
            return;
        }
        let mut devs: Vec<Value> = b["devices"]
            .as_array()
            .map(|d| d.iter().filter(|d| d.is_object() && !is_false(&d["paired"])).cloned().collect())
            .unwrap_or_default();
        if devs.is_empty() {
            sub.list.append(&empty_msg(&t("btNone"), Some(&t("btNoneHint")), Some("bluetooth")));
        }
        devs.sort_by(|a, c| {
            truthy(&c["connected"])
                .cmp(&truthy(&a["connected"]))
                .then_with(|| s(&a["name"]).to_lowercase().cmp(&s(&c["name"]).to_lowercase()))
        });
        let expanded = sub.expanded.borrow().clone();
        for d in &devs {
            let addr = s(&d["address"]).to_owned();
            let is_open = expanded.as_deref() == Some(addr.as_str());
            let state = sub.busy.borrow().get(&addr).copied();
            let connected = truthy(&d["connected"]) && !stuck;
            let status = match state {
                Some("connect") => t("connecting"),
                Some(_) => t("disconnecting"),
                None if connected => match d["battery"].as_f64() {
                    Some(p) => format!("{} · {}", t("connected"), tp("btBattery", &pct(p))),
                    None => t("connected"),
                },
                None => t("notConnected"),
            };
            let row = vbox(0, &["fo-net"]);
            if is_open {
                row.add_css_class("is-expanded");
            }
            if connected {
                row.add_css_class("is-active");
            }
            let head = button(&["fo-net__head"]);
            let hb = hbox(12, &[]);
            let g = gtk::CenterBox::new();
            g.add_css_class("fo-net__glyph");
            g.set_size_request(20, 20);
            g.set_center_widget(Some(&glyph(bt_icon(d), 20)));
            hb.append(&g);
            let text = vbox(0, &["fo-net__text"]);
            text.set_hexpand(true);
            let name = if truthy(&d["name"]) { s(&d["name"]) } else { addr.as_str() };
            text.append(&ell(lbl(name, &["fo-net__name"])));
            text.append(&ell(lbl(&status, &["fo-net__status", "q-caption", "q-text-2"])));
            hb.append(&text);
            head.set_child(Some(&hb));
            head.update_state(&[gtk::accessible::State::Expanded(Some(is_open))]);
            let w = Rc::downgrade(sub);
            let k = addr.clone();
            head.connect_clicked(move |_| {
                let (Some(q), Some(sub)) = (q(), w.upgrade()) else { return };
                let open = sub.expanded.borrow().as_deref() == Some(k.as_str());
                *sub.expanded.borrow_mut() = if open { None } else { Some(k.clone()) };
                *sub.focus_after.borrow_mut() = Some(k.clone());
                q.render_sub(&sub);
            });
            sub.heads.borrow_mut().insert(addr.clone(), head.clone());
            row.append(&head);
            if is_open {
                let body = vbox(8, &["fo-net__body"]);
                if let Some(err) = sub.errors.borrow().get(&addr) {
                    body.append(&wrapped(lbl(err, &["fo-net__error", "q-caption"])));
                }
                if state.is_some() {
                    body.append(&indeterminate(&status));
                } else {
                    let actions = hbox(8, &["fo-net__actions"]);
                    actions.append(&grow());
                    let b = text_btn(&if connected { t("disconnect") } else { t("connect") }, !connected);
                    b.set_sensitive(!(resetting || stuck));
                    let w = Rc::downgrade(sub);
                    let k = addr.clone();
                    b.connect_clicked(move |_| {
                        if let (Some(q), Some(sub)) = (q(), w.upgrade()) {
                            q.bt_act(&sub, k.clone(), if connected { "disconnect" } else { "connect" });
                        }
                    });
                    actions.append(&b);
                    body.append(&actions);
                }
                row.append(&body);
            }
            sub.list.append(&row);
        }
        // "Apparaat toevoegen": Instellingen > Bluetooth en apparaten with "Een apparaat toevoegen" open
        let add = vbox(0, &["fo-net", "fo-bt-add"]);
        let head = button(&["fo-net__head"]);
        let hb = hbox(12, &[]);
        let g = gtk::CenterBox::new();
        g.add_css_class("fo-net__glyph");
        g.add_css_class("fo-bt-add__glyph");
        g.set_size_request(20, 20);
        g.set_center_widget(Some(&glyph("plus", 16)));
        hb.append(&g);
        hb.append(&ell(lbl(&t("pairNew"), &["fo-net__name"])));
        head.set_child(Some(&hb));
        head.connect_clicked(|_| {
            let shell = sh();
            shell.fire("shell.openApp", json!({"app": "settings", "params": {"page": "bluetooth", "add": 1}}));
            close_flyouts(&shell);
        });
        sub.heads.borrow_mut().insert("+".into(), head.clone());
        add.append(&head);
        sub.list.append(&add);
        self.restore_focus(sub);
    }

    fn bt_stuck_notice(self: &Rc<Self>, sub: &Rc<Sub>, resetting: bool) -> gtk::Box {
        let n = hbox(8, &["fo-qs__notice", "fo-bt-stuck"]);
        if resetting {
            n.add_css_class("is-info");
        }
        let icon = glyph(if resetting { "info" } else { "warning" }, 16);
        icon.add_css_class("q-notice-icon");
        icon.set_valign(gtk::Align::Start);
        n.append(&icon);
        let body = vbox(2, &["fo-bt-stuck__body"]);
        body.set_hexpand(true);
        body.append(&wrapped(lbl(&if resetting { t("btResetting") } else { t("btStuck") }, &["q-caption"])));
        if resetting {
            body.append(&indeterminate(&t("btResetting")));
        } else {
            let reset = button(&["fo-sub__link", "fo-bt-stuck__reset"]);
            reset.set_halign(gtk::Align::Start);
            reset.set_child(Some(&lbl(&t("btReset"), &["q-caption"])));
            let w = Rc::downgrade(sub);
            reset.connect_clicked(move |_| {
                if let (Some(q), Some(sub)) = (q(), w.upgrade()) {
                    q.bt_reset(&sub);
                }
            });
            body.append(&reset);
        }
        n.append(&body);
        n
    }

    fn bt_act(self: &Rc<Self>, sub: &Rc<Sub>, addr: String, what: &'static str) {
        sub.busy.borrow_mut().insert(addr.clone(), what);
        sub.errors.borrow_mut().remove(&addr);
        self.render_sub(sub);
        let method = if what == "connect" { "system.bluetooth.connect" } else { "system.bluetooth.disconnect" };
        let weak = Rc::downgrade(sub);
        sh().call(method, json!({"address": addr}), move |res| {
            let (Some(q), Some(sub)) = (q(), weak.upgrade()) else { return };
            // the host answers {ok, error?, code?} (its text in our language) instead of failing for device problems
            match res {
                Ok(r) => {
                    if is_false(&r["ok"]) {
                        if is_bt_not_ready(&r) {
                            sub.not_ready.set(true);
                        }
                        let e = r["error"]
                            .as_str()
                            .filter(|e| !e.is_empty())
                            .map(str::to_owned)
                            .unwrap_or_else(|| t("failed"));
                        sub.errors.borrow_mut().insert(addr.clone(), e);
                    } else if is_dry_run(&r) {
                        q.flash(&t("dryRun"), true);
                    } else {
                        *sub.expanded.borrow_mut() = None;
                    }
                }
                Err(e) => {
                    sub.errors.borrow_mut().insert(addr.clone(), err_text(&e));
                }
            }
            sub.busy.borrow_mut().remove(&addr);
            let weak = Rc::downgrade(&sub);
            sh().call("system.bluetooth.get", json!({}), move |res| {
                let Some(q) = self::q() else { return };
                if let Ok(v) = res
                    && !v.is_null()
                {
                    q.st.borrow_mut().bluetooth = v;
                }
                q.render_tiles();
                if let Some(sub) = weak.upgrade()
                    && q.is_current(&sub)
                {
                    *sub.focus_after.borrow_mut() = Some(addr);
                    q.render_sub(&sub);
                }
            });
        });
    }

    /// "Bluetooth opnieuw starten" right here (rfkill cycle on the host when the adapter is stuck).
    fn bt_reset(self: &Rc<Self>, sub: &Rc<Sub>) {
        if sub.resetting.get() {
            return;
        }
        sub.resetting.set(true);
        self.render_sub(sub);
        let weak = Rc::downgrade(sub);
        sh().call("system.bluetooth.reset", json!({}), move |res| {
            let Some(q) = q() else { return };
            let sub = weak.upgrade();
            match res {
                Ok(r) => {
                    if !r["available"].is_null() {
                        q.st.borrow_mut().bluetooth = r;
                    }
                    if let Some(sub) = &sub {
                        sub.not_ready.set(false);
                        sub.errors.borrow_mut().clear();
                        sub.resetting.set(false);
                        q.render_sub(sub);
                    }
                    q.flash(&t("btResetOk"), true);
                }
                Err(e) => {
                    if let Some(sub) = &sub {
                        sub.resetting.set(false);
                        q.render_sub(sub);
                    }
                    q.flash(&err_text(&e), false);
                }
            }
            q.render_tiles();
        });
    }

    /* ---------------------------------------------------------------- Sound */

    fn render_sound(self: &Rc<Self>, sub: &Rc<Sub>) {
        let sinks: Vec<Value> = self.st.borrow().audio["sinks"].as_array().cloned().unwrap_or_default();
        self.keep_focus(sub);
        clear_box(&sub.list);
        if sinks.is_empty() {
            sub.list.append(&empty_msg(&t("noSinks"), None, Some("sound")));
            return;
        }
        for sink in &sinks {
            let id = sink["id"].clone();
            let key = match &id {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let selected = truthy(&sink["default"]);
            let name = if truthy(&sink["name"]) { s(&sink["name"]).to_owned() } else { key.clone() };
            let item = button(&["fo-sink"]);
            if selected {
                item.add_css_class("is-selected");
            }
            item.update_state(&[gtk::accessible::State::Selected(Some(selected))]);
            let hb = hbox(12, &[]);
            let ind = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            ind.add_css_class("fo-sink__ind");
            ind.set_size_request(3, 16);
            ind.set_valign(gtk::Align::Center);
            hb.append(&ind);
            let low = name.to_lowercase();
            let both = format!("{low}{}", key.to_lowercase());
            let icon = if ["headset", "headphone", "hoofdtelefoon"].iter().any(|w| low.contains(w)) {
                "headphones"
            } else if ["hdmi", "display", "monitor", "lg", "dell", "samsung"].iter().any(|w| both.contains(w)) {
                "display"
            } else {
                "volume-2"
            };
            hb.append(&glyph(icon, 16));
            hb.append(&ell(lbl(&name, &["fo-sink__name"])));
            item.set_child(Some(&hb));
            sub.heads.borrow_mut().insert(key.clone(), item.clone());
            let w = Rc::downgrade(sub);
            item.connect_clicked(move |_| {
                if selected {
                    return;
                }
                let (Some(q), Some(sub)) = (q(), w.upgrade()) else { return };
                let prev = q.st.borrow().audio.clone();
                let next: Vec<Value> = prev["sinks"]
                    .as_array()
                    .map(|a| a.iter().map(|x| with(x, "default", json!(x["id"] == id))).collect())
                    .unwrap_or_default();
                q.st.borrow_mut().audio = with(&prev, "sinks", Value::Array(next));
                *sub.focus_after.borrow_mut() = Some(key.clone());
                q.render_sub(&sub);
                let me = q.clone();
                q.guarded(
                    "system.audio.setSink",
                    json!({"id": id}),
                    move || {
                        me.st.borrow_mut().audio = prev;
                        if let Some(sub) = me.sub_of(Kind::Sound) {
                            me.render_sub(&sub);
                        }
                    },
                    |_| {},
                );
            });
            sub.list.append(&item);
        }
        self.restore_focus(sub);
    }

    /* ---------------------------------------------------------------- lifecycle */

    fn on_open(self: &Rc<Self>) {
        if self.sub.borrow().is_some() {
            self.close_sub(true);
        }
        if let Some(ui) = self.ui() {
            clear_box(&ui.notice);
            ui.notice.set_visible(false);
            ui.wrap.add_css_class("is-opening");
        }
        apply_mode(&self.root);
        self.render_all();
        // refresh in the background (events keep it live while open)
        self.refresh_all();
    }

    fn on_close(self: &Rc<Self>) {
        if let Some(ui) = self.ui() {
            ui.wrap.remove_css_class("is-opening");
            ui.power.popdown();
        }
        let sub = self.sub.borrow().clone();
        if let Some(sub) = sub {
            let weak = Rc::downgrade(&sub);
            glib::timeout_add_local_once(Duration::from_millis(200), move || {
                if let (Some(q), Some(sub)) = (q(), weak.upgrade())
                    && q.is_current(&sub)
                    && !flyout_open("quick")
                {
                    q.close_sub(true);
                }
            });
        }
    }
}

/* ==========================================================================
   Free-standing widgets
   ========================================================================== */

fn slider(label: &str) -> gtk::Scale {
    let s = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    s.add_css_class("q-slider");
    s.set_draw_value(false);
    s.set_round_digits(0);
    s.set_hexpand(true);
    s.set_valign(gtk::Align::Center);
    s.set_value(50.0);
    set_a11y(&s, label);
    s
}

/// The sub-page list's height: min(336 px, the monitor minus the taskbar and 260 px of flyout around it).
fn sub_list_max() -> i32 {
    let geo = sh().gnome.call("panel.geometry", &json!({})).unwrap_or(Value::Null);
    let h = geo["logical"]["h"].as_i64().or(geo["h"].as_i64()).unwrap_or(1080) as i32;
    (h - crate::panel::BAR - 260).clamp(120, 336)
}

fn remove_page(el: &gtk::Box) {
    if let Some(stack) = el.parent().and_then(|p| p.downcast::<gtk::Stack>().ok()) {
        stack.remove(el);
    }
}

fn remove_page_later(el: &gtk::Box) {
    let el = el.clone();
    glib::timeout_add_local_once(Duration::from_millis(320), move || {
        // only when it is not shown again by now
        let shown = el
            .parent()
            .and_then(|p| p.downcast::<gtk::Stack>().ok())
            .and_then(|s| s.visible_child())
            .is_some_and(|c| c == el);
        if !shown {
            remove_page(&el);
        }
    });
}

fn wifi_glyph(strength: Option<f64>, secure: bool) -> gtk::Widget {
    let o = gtk::Overlay::new();
    o.add_css_class("fo-net__glyph");
    o.set_size_request(20, 20);
    o.set_valign(gtk::Align::Center);
    o.set_child(Some(&glyph(wifi_icon(strength), 20)));
    if secure {
        let lock = glyph("lock", 10);
        lock.add_css_class("fo-net__lock");
        lock.set_halign(gtk::Align::End);
        lock.set_valign(gtk::Align::End);
        o.add_overlay(&lock);
    }
    o.upcast()
}

/// Host bt_kind() to a glyph; older hosts only give BlueZ's icon name.
fn bt_icon(d: &Value) -> &'static str {
    match s(&d["kind"]) {
        "headphones" | "headset" => return "headphones",
        "speaker" | "media" => return "sound",
        "phone" => return "phone",
        "computer" => return "pc",
        "keyboard" => return "keyboard",
        "mouse" => return "mouse",
        "gamepad" => return "gamepad",
        "pen" => return "edit",
        "display" => return "display",
        "camera" => return "camera",
        "printer" => return "print",
        "watch" => return "clock",
        "network" => return "network",
        _ => {}
    }
    let n = s(&d["icon"]);
    let has = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if has(&["headset", "headphone", "audio-head"]) {
        "headphones"
    } else if has(&["mouse"]) {
        "mouse"
    } else if has(&["keyboard"]) {
        "keyboard"
    } else if has(&["gaming", "joystick", "gamepad"]) {
        "gamepad"
    } else if has(&["phone"]) {
        "phone"
    } else if has(&["audio", "speaker", "card"]) {
        "sound"
    } else if has(&["computer"]) {
        "pc"
    } else {
        "bluetooth"
    }
}

/// "Bluetooth reageert niet": the host's code (not_ready) or its catalog key, never the text.
fn is_bt_not_ready(r: &Value) -> bool {
    r["code"] == "not_ready" || r["key"] == "system.bt_not_responding"
}

fn player_icon(p: &Value) -> String {
    if truthy(&p["icon"]) {
        return s(&p["icon"]).to_owned();
    }
    if truthy(&p["desktopId"]) {
        return s(&p["desktopId"]).trim_end_matches(".desktop").to_owned();
    }
    let id = if truthy(&p["identity"]) { s(&p["identity"]) } else { "audio-player" };
    id.to_lowercase().split_whitespace().collect::<Vec<_>>().join("-")
}

/// The footer's power button: Slaapstand, Afsluiten, Opnieuw opstarten (as Start's power menu), confirmed by
/// power.rs.
fn power_menu() -> gtk::MenuButton {
    let mb = gtk::MenuButton::new();
    mb.add_css_class("q-power");
    mb.set_child(Some(&glyph("power", 16)));
    mb.set_tooltip_text(Some(&t("power")));
    set_a11y(&mb, &t("power"));
    mb.set_direction(gtk::ArrowType::Up);
    mb.set_has_frame(false);
    mb.set_size_request(40, 40);
    mb.set_valign(gtk::Align::Center);
    let pop = gtk::Popover::new();
    pop.add_css_class("q-menu");
    pop.set_has_arrow(false);
    pop.set_position(gtk::PositionType::Top);
    let list = vbox(2, &[]);
    for (icon, key, action) in
        [("sleep", "sleep", "suspend"), ("power", "shutdown", "shutdown"), ("restart", "restart", "reboot")]
    {
        let b = button(&["q-menu-item"]);
        let row = hbox(12, &[]);
        row.append(&glyph(icon, 16));
        row.append(&lbl(&t(key), &[]));
        b.set_child(Some(&row));
        let popw = pop.downgrade();
        b.connect_clicked(move |_| {
            if let Some(p) = popw.upgrade() {
                p.popdown();
            }
            super::power::confirm(&sh(), action);
        });
        list.append(&b);
    }
    pop.set_child(Some(&list));
    mb.set_popover(Some(&pop));
    mb
}

fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 1.5 * pi);
    cr.close_path();
}

/// Battery glyph with a fill level (and a bolt while charging), drawn like the Win11 tray icon (viewBox 20).
fn draw_battery(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, percent: f64, charging: bool) {
    let c = area.color();
    let (r, g, b, a) = (c.red() as f64, c.green() as f64, c.blue() as f64, c.alpha() as f64);
    let dark = sh().dark();
    let p = percent.clamp(0.0, 100.0);
    let w = ((p / 100.0) * 10.5).round().max(1.0);
    let low = p <= 10.0 && !charging;
    cr.scale(16.0 / 20.0, 16.0 / 20.0);
    cr.set_source_rgba(r, g, b, a);
    cr.set_line_width(1.35);
    rounded_rect(cr, 2.0, 5.5, 14.0, 9.0, 2.0);
    let _ = cr.stroke();
    cr.set_line_cap(gtk::cairo::LineCap::Round);
    cr.move_to(18.0, 8.5);
    cr.line_to(18.0, 11.5);
    let _ = cr.stroke();
    if low {
        if dark {
            cr.set_source_rgb(1.0, 0.6, 0.643)
        } else {
            cr.set_source_rgb(0.769, 0.169, 0.11)
        }
    }
    rounded_rect(cr, 3.75, 7.25, w, 5.5, 0.9);
    let _ = cr.fill();
    if charging {
        let [ar, ag, ab] = accent_rgb(dark);
        cr.move_to(10.4, 6.6);
        cr.line_to(7.9, 10.3);
        cr.line_to(10.3, 10.3);
        cr.line_to(9.3, 13.4);
        cr.line_to(12.2, 9.3);
        cr.line_to(9.8, 9.3);
        cr.close_path();
        cr.set_source_rgb(ar, ag, ab);
        let _ = cr.fill_preserve();
        if dark {
            cr.set_source_rgb(0.173, 0.173, 0.173)
        } else {
            cr.set_source_rgb(0.988, 0.988, 0.988)
        }
        cr.set_line_width(0.8);
        let _ = cr.stroke();
    }
}
