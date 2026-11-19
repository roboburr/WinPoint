//! Instellingen › Toegankelijkheid › Muisaanwijzer en aanraken, Tekstcursor, Vergrootglas, Verteller (batch A,
//! notes/claude/settings-parity.md §2, plan.json "mousepointer", "textcursor", "magnifier", "narrator"): Windows 11's
//! Vision pages with the functions of GNOME Settings 50's Seeing and Zoom pages they hold.
//!
//! * Muisaanwijzer en aanraken, Windows' order: Grootte = `org.gnome.desktop.interface cursor-size` on GNOME
//!   Settings' five sizes (24 standard, 32, 48, 64, 96 px; Windows' slider has 15 steps, GNOME's dialog five, so the
//!   slider has GNOME's; a size set elsewhere shows at the nearest step and is written only when the slider moves);
//!   Muisaanwijzerindicator = `locate-pointer` (GNOME's Locate Pointer, the Ctrl key). Windows' Stijl van
//!   muisaanwijzer (white, black, inverted, custom colour) needs cursor themes GNOME Settings 50 does not offer, and
//!   cursor-theme is in no allow table: left out. Aanraken: Windows' Aanraakindicator has no GNOME function; in its
//!   place GNOME's touchscreen mapping, Aanraakscherm toewijzen aan beeldscherm per touchscreen (amendment 8, the
//!   rows of pen.rs: so a computer with a touchscreen and no pen reaches them too, slice 1's note 1). Gerelateerde
//!   instellingen: Muis.
//! * Tekstcursor: Windows' Indicator voor tekstcursor and Dikte van tekstcursor have no GNOME function (GTK's caret
//!   width is no setting on Wayland): left out. GNOME's Cursor Blinking as Windows' own words of the classic
//!   Keyboard Properties: Knipperende tekstcursor = `cursor-blink`, Knippersnelheid = `cursor-blink-time` (100-2500 ms,
//!   Langzaam left, Snel right, as GNOME's Blink Speed) with Windows' test box (GTK follows the keys at once).
//!   Gerelateerde instellingen: Geavanceerde toetsenbordinstellingen (Tekenherhaling).
//! * Vergrootglas (GNOME's Zoom), Windows' order: Vergrootglas = `org.gnome.desktop.a11y.applications
//!   screen-magnifier-enabled` (an expander with GNOME's three magnifier shortcuts of media-keys, shown as key caps,
//!   set in Sneltoetsen); Zoomniveau with − and + = `org.gnome.desktop.a11y.magnifier mag-factor` (GNOME Settings'
//!   1.00-20.00 in steps of 0.25, shown as 100-2000 %; a factor set elsewhere, by GNOME Shell's own zoom keys, steps
//!   to the next quarter); Kleuren omkeren = `invert-lightness`. Weergave: Volledig scherm / Vastgezet / Lens =
//!   `lens-mode` with `screen-position` (GNOME's Magnifier View and Screen Area: Volledig scherm is full-screen
//!   without the lens, Vastgezet a half of the screen, Lens follows the pointer with the size of that half, since
//!   GNOME Shell's lens over the full screen does nothing), written together in one `gsettings.set`; Schermgebied =
//!   the half (Bovenste, Onderste, Linker, Rechter); Buiten de schermranden doorgaan = `scroll-at-edges`; De
//!   muisaanwijzer houden = `mouse-tracking` (Gecentreerd op het scherm = centered, Binnen de randen van het scherm =
//!   push, Met de inhoud meebewegen = proportional, GNOME's Follow Behavior); Vergrootglas laten volgen:
//!   Muisaanwijzer (`mouse-tracking` none or the chosen way), Toetsenbordfocus (`focus-tracking`), Tekstcursor
//!   (`caret-tracking`): Windows' boxes, GNOME's keys without a GNOME Settings control (ticked writes the schema's
//!   default). WinPoint additions from GNOME (windows.json): Kleuraanpassing (Helderheid and Contrast set the red,
//!   green and blue keys together, -0.8..0.8 as GNOME Settings; Kleur = `color-saturation`) and Dradenkruis
//!   (`show-cross-hairs`; Over muisaanwijzer heen = `cross-hairs-clip` inverted; Dikte 1-100 px; Lengte 0-100 % =
//!   20-4096 px, the schema's range; Kleur; Dekking = `cross-hairs-opacity`, the alpha of GNOME's colour button).
//!   Windows' Zoomtoename (no GNOME key; settings-defaults.json is frozen), Randen afvlakken and the sign-in rows
//!   (GNOME's key persists; the login screen is GDM's) are left out.
//! * Verteller (GNOME's Screen Reader): Verteller = `screen-reader-enabled` (GNOME Shell starts Orca), with
//!   Sneltoets voor Verteller (media-keys `screenreader`, set in Sneltoetsen); Instellingen voor Verteller openen =
//!   GNOME's Configure, sensitive while the screen reader is on as GNOME Settings' row: `input.screenReader.preferences`
//!   asks the running Orca for its own preferences window over D-Bus (amendment 36), so no Python runs under WinPoint.
//!   Windows' voice, verbosity, Narrator key and braille sections are Orca's preferences.
//!
//! Shared with a11y_interaction.rs (`pub(super)`): a switch expander, the shortcut rows, sliders between two words
//! with a value text, the write of keys that belong together.
use super::mouse::{bind_state, mapped_checkbox};
use super::pen::{TOUCHSCREEN, touchscreen_group, touchscreen_path};
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Chevron, Dropdown, DropdownOption, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, Slider, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("mousepointer", mousepointer), ("textcursor", textcursor), ("magnifier", magnifier), ("narrator", narrator)];
pub const CSS: &str = r#"
.st-a11y-scale-end { color: var(--text-2); }
.st-a11y-test-box { min-width: 200px; }
.st-a11y-zoom-value { min-width: 64px; }
.st-a11y-swatch { border-radius: 4px; }
.st-a11y-note.st-expander-note.w11-nested-other { padding-left: 58px; }
"#;

const IFACE: &str = "org.gnome.desktop.interface";
const SIZE: GKey = gs::key(IFACE, "cursor-size");
const LOCATE: GKey = gs::key(IFACE, "locate-pointer");
const BLINK: GKey = gs::key(IFACE, "cursor-blink");
const BLINK_TIME: GKey = gs::key(IFACE, "cursor-blink-time");

const APPS: &str = "org.gnome.desktop.a11y.applications";
const MAG: &str = "org.gnome.desktop.a11y.magnifier";
const MEDIA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const ZOOM: GKey = gs::key(APPS, "screen-magnifier-enabled");
const READER: GKey = gs::key(APPS, "screen-reader-enabled");
const FACTOR: GKey = gs::key(MAG, "mag-factor");
const POSITION: GKey = gs::key(MAG, "screen-position");
const LENS: GKey = gs::key(MAG, "lens-mode");
const EDGES: GKey = gs::key(MAG, "scroll-at-edges");
const MOUSE_TRACK: GKey = gs::key(MAG, "mouse-tracking");
const FOCUS_TRACK: GKey = gs::key(MAG, "focus-tracking");
const CARET_TRACK: GKey = gs::key(MAG, "caret-tracking");
const INVERT: GKey = gs::key(MAG, "invert-lightness");
const BRIGHTNESS: [GKey; 3] = [gs::key(MAG, "brightness-red"), gs::key(MAG, "brightness-green"), gs::key(MAG, "brightness-blue")];
const CONTRAST: [GKey; 3] = [gs::key(MAG, "contrast-red"), gs::key(MAG, "contrast-green"), gs::key(MAG, "contrast-blue")];
const SATURATION: GKey = gs::key(MAG, "color-saturation");
const CROSS: GKey = gs::key(MAG, "show-cross-hairs");
const CROSS_CLIP: GKey = gs::key(MAG, "cross-hairs-clip");
const CROSS_THICK: GKey = gs::key(MAG, "cross-hairs-thickness");
const CROSS_LENGTH: GKey = gs::key(MAG, "cross-hairs-length");
const CROSS_COLOR: GKey = gs::key(MAG, "cross-hairs-color");
const CROSS_OPACITY: GKey = gs::key(MAG, "cross-hairs-opacity");
/// GNOME's magnifier shortcuts (media-keys; Sneltoetsen sets them through input.shortcuts.set).
const ZOOM_KEYS: [GKey; 3] = [gs::key(MEDIA, "magnifier"), gs::key(MEDIA, "magnifier-zoom-in"), gs::key(MEDIA, "magnifier-zoom-out")];
const READER_KEY: GKey = gs::key(MEDIA, "screenreader");

/// GNOME Settings' Magnification Factor (its spin row: 1.00-20.00 in steps of 0.25).
const FACTOR_MIN: f64 = 1.0;
const FACTOR_MAX: f64 = 20.0;
const FACTOR_STEP: f64 = 0.25;
/// The halves of `screen-position` (GNOME's Screen Area without Full Screen).
const HALVES: [&str; 4] = ["top-half", "bottom-half", "left-half", "right-half"];

/// GNOME Settings' cursor sizes (cc-ua-seeing-page's Cursor Size dialog), px.
const CURSOR_SIZES: [i64; 5] = [24, 32, 48, 64, 96];

// ------------------------------------------------------------------------------------------ value mappings

/// The slider step (1-5) of a cursor size: GNOME's own size, else the nearest of them.
fn size_ui(v: &Value) -> f64 {
    let px = v.as_i64().unwrap_or(24);
    let (i, _) = CURSOR_SIZES.iter().enumerate().min_by_key(|(_, s)| (**s - px).abs()).unwrap_or((0, &24));
    (i + 1) as f64
}

/// The cursor size of a slider step.
fn size_gs(n: f64) -> Value {
    json!(CURSOR_SIZES[(n.round() as usize).clamp(1, CURSOR_SIZES.len()) - 1])
}

/// The size's label next to the slider ("24 px (standaard)").
fn size_text(px: i64) -> String {
    if px == 24 { format!("{px} px ({})", t("standaard", "default")) } else { format!("{px} px") }
}

/// GNOME's blink time (ms) on Windows' Knippersnelheid slider: Langzaam (2500 ms) left, Snel (100 ms) right.
fn blink_ui(v: &Value) -> f64 {
    (2600.0 - v.as_f64().unwrap_or(1200.0)).clamp(100.0, 2500.0)
}

/// The blink time of a slider position.
fn blink_gs(n: f64) -> Value {
    json!((2600.0 - n.round()).clamp(100.0, 2500.0) as i64)
}

/// The zoom level as Windows shows it ("200%") of GNOME's mag-factor.
fn factor_text(v: f64) -> String {
    format!("{}%", (v * 100.0).round() as i64)
}

/// Zoomniveau's − and +: the next quarter below or above `v` (a factor set elsewhere, GNOME Shell's zoom keys
/// multiply, steps onto GNOME Settings' grid), within 1.00-20.00.
fn factor_step(v: f64, up: bool) -> f64 {
    let q = v / FACTOR_STEP;
    let next = if up { (q + 1e-6).floor() + 1.0 } else { (q - 1e-6).ceil() - 1.0 };
    (next * FACTOR_STEP).clamp(FACTOR_MIN, FACTOR_MAX)
}

/// Weergave of lens-mode and screen-position: "lens", "full" (Volledig scherm) or "docked" (Vastgezet: a half).
fn view_of(lens: &Value, position: &Value) -> &'static str {
    if *lens == true {
        "lens"
    } else if position.as_str().is_some_and(|p| HALVES.contains(&p)) {
        "docked"
    } else {
        "full"
    }
}

/// The half a docked view or a lens uses: the current one, else the top half (Windows' docked view is on top).
fn half_of(position: &Value) -> &'static str {
    position.as_str().and_then(|p| HALVES.iter().find(|h| **h == p)).copied().unwrap_or("top-half")
}

/// The values a Weergave choice writes, together.
fn view_values(view: &str, position: &Value) -> Value {
    match view {
        "lens" => json!({ "lens-mode": true, "screen-position": half_of(position) }),
        "docked" => json!({ "lens-mode": false, "screen-position": half_of(position) }),
        _ => json!({ "lens-mode": false, "screen-position": "full-screen" }),
    }
}

fn view_options() -> Vec<(&'static str, &'static str)> {
    vec![("full", t("Volledig scherm", "Full screen")), ("docked", t("Vastgezet", "Docked")), ("lens", t("Lens", "Lens"))]
}

fn area_options() -> Vec<(Value, String)> {
    vec![
        (json!("top-half"), t("Bovenste helft", "Top half").into()),
        (json!("bottom-half"), t("Onderste helft", "Bottom half").into()),
        (json!("left-half"), t("Linkerhelft", "Left half").into()),
        (json!("right-half"), t("Rechterhelft", "Right half").into()),
    ]
}

/// De muisaanwijzer houden: GNOME's Follow Behavior (mouse-tracking).
fn keep_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("centered", t("Gecentreerd op het scherm", "Centred on the screen")),
        ("push", t("Binnen de randen van het scherm", "Within the edges of the screen")),
        ("proportional", t("Met de inhoud meebewegen", "Moving with the contents")),
    ]
}

/// Whether a tracking key follows (anything but 'none').
fn tracks(v: &Value) -> bool {
    v.as_str().is_some_and(|m| m != "none")
}

/// Toetsenbordfocus ticked writes focus-tracking's default, unticked 'none'.
fn focus_value(on: bool) -> Value {
    json!(if on { "proportional" } else { "none" })
}

/// Tekstcursor ticked writes caret-tracking's default, unticked 'none'.
fn caret_value(on: bool) -> Value {
    json!(if on { "centered" } else { "none" })
}

/// A colour key's slider value, to two decimals (GNOME Settings' sliders step 0.02 and 0.01).
fn round2(n: f64) -> f64 {
    (n * 100.0).round() / 100.0
}

/// The crosshairs' Lengte, 0-100 %, of cross-hairs-length (the schema's 20-4096 px).
fn length_ui(v: &Value) -> f64 {
    ((v.as_f64().unwrap_or(4096.0) - 20.0) * 100.0 / 4076.0).round().clamp(0.0, 100.0)
}

/// cross-hairs-length for a Lengte percentage.
fn length_gs(n: f64) -> Value {
    json!((20.0 + n.round().clamp(0.0, 100.0) * 4076.0 / 100.0).round() as i64)
}

/// A colour of `#rrggbb` (cross-hairs-color) as cairo's 0-1 channels; red for anything else (GNOME's default).
fn rgb_of(hex: &str) -> (f64, f64, f64) {
    let h = kit::theme::normalize_hex(hex).unwrap_or_else(|| "#ff0000".into());
    let ch = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).map(|v| v as f64 / 255.0).unwrap_or(0.0);
    (ch(1), ch(3), ch(5))
}

// ------------------------------------------------------------------------------------------ shared rows

/// Keep the page's `Gs` until the page goes (gs.rs subscribes weakly: slice 2's note 11).
pub(super) fn keep(p: &Page, gs: &Gs) {
    let kept = gs.clone();
    p.on_cleanup(move || drop(kept));
}

/// An expander whose header holds the switch of a boolean key (Windows' Plaktoetsen, Dradenkruis, ...): gs.rs's
/// switch (optimistic, locked, following changes); a click on it toggles only the switch (touchpad.rs). None when
/// the key is not available here.
pub(super) fn switch_expander(gs: &Gs, k: &GKey, icon: &str, title: &str, description: &str) -> Option<(SettingsExpander, ToggleSwitch)> {
    if !gs.available(k) {
        return None;
    }
    let exp = SettingsExpander::new(icon, title, description);
    let sw = gs.toggle(k, false);
    exp.header_button().set_widget_name(&format!("{}-expander", sw.switch().widget_name()));
    exp.header().set_control(sw.widget());
    Some((exp, sw))
}

/// The keys of a shortcut value (its first accelerator) as key caps, or "Uitgeschakeld".
fn shortcut_keys(v: &Value) -> gtk::Widget {
    let en = kit::ctx().lang() == "en";
    let accel = v.as_array().into_iter().flatten().filter_map(|a| a.as_str()).find(|a| !a.is_empty());
    match accel.and_then(|a| super::shortcuts::accel_keys(a, en)) {
        Some(keys) => {
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            common::kbd(&refs).upcast()
        }
        None => c::label(t("Uitgeschakeld", "Disabled"), "w11-text-2", false).upcast(),
    }
}

/// A row of one of GNOME's accessibility shortcuts (media-keys): its keys, following changes; a click opens
/// Sneltoetsen at Toegankelijkheid, where it is set. None when the key is not there.
pub(super) fn shortcut_card(p: &Page, gs: &Gs, k: &GKey, title: &str) -> Option<SettingsCard> {
    if !gs.available(k) {
        return None;
    }
    let card = SettingsCard::clickable("", title, t("Wijzigen in Sneltoetsen", "Change it in Keyboard shortcuts"), Chevron::Right);
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    slot.append(&shortcut_keys(&gs.value(k)));
    card.set_control(&slot);
    if let Some(b) = card.button() {
        b.set_widget_name(&format!("st-{}-{}", p.id(), k.key));
    }
    let slot2 = slot.clone();
    gs.follow(k, move |v| {
        common::set_slot(&slot2, &[shortcut_keys(v)]);
    });
    let pp = p.clone();
    card.connect_clicked(move || pp.go_with("shortcuts", json!({ "section": "accessibility" })));
    Some(card)
}

/// A slider between two words, with a value text of a fixed width after it (a text that grows would move the
/// slider under the pointer: slice 3's harness finding).
pub(super) fn scale_row(slider: &Slider, left: &str, right: &str, value: Option<&gtk::Label>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&c::label(left, "w11-caption st-a11y-scale-end", false));
    slider.widget().set_size_request(200, -1);
    row.append(slider.widget());
    row.append(&c::label(right, "w11-caption st-a11y-scale-end", false));
    if let Some(l) = value {
        row.append(l);
    }
    row
}

/// The value text of a slider bound to `k` (`text` of the slider position), following the slider and the key;
/// `widest` sizes it.
pub(super) fn value_text(gs: &Gs, slider: &Slider, k: &GKey, to_ui: fn(&Value) -> f64, text: fn(f64) -> String, widest: &str) -> gtk::Label {
    let l = common::value_label(&text(to_ui(&gs.value(k))));
    l.set_width_chars(widest.chars().count() as i32);
    l.set_xalign(0.0);
    l.set_widget_name(&format!("{}-value", slider.scale().widget_name()));
    {
        let l = l.clone();
        slider.connect_changed(move |n| l.set_label(&text(n)));
    }
    let l2 = l.clone();
    gs.follow(k, move |v| l2.set_label(&text(to_ui(v))));
    l
}

/// Write a named subset of known keys together through Gs::set_values, including cache updates and rollback.
pub(super) fn write_together(gs: &Gs, keys: &[GKey], values: Value, revert: impl FnOnce() + 'static) {
    let items: Vec<(&GKey, Value)> = keys.iter().filter_map(|k| values.get(k.key).map(|v| (k, v.clone()))).collect();
    gs.set_values(&items, revert);
}

/// A card with a control nested in an expander.
fn nested_card(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> SettingsCard {
    let card = SettingsCard::new("", title, description);
    card.set_control(control);
    card
}

/// Value texts of the sliders.
fn percent_text(n: f64) -> String {
    format!("{}%", n.round() as i64)
}

fn px_text(n: f64) -> String {
    format!("{} px", n.round() as i64)
}

/// A group of the widgets there are; None when it is empty.
fn filled(group: SettingsGroup) -> Option<gtk::Widget> {
    group.items().first_child().is_some().then(|| group.widget().clone().upcast())
}

/// A slider between two words ("Klein" slider "Groot").
fn between(slider: &Slider, left: &str, right: &str) -> gtk::Box {
    scale_row(slider, left, right, None)
}

// ------------------------------------------------------------------------------------------ the pages

fn mousepointer(p: Page) -> PageFuture {
    Box::pin(async move {
        // the touchscreens and monitors only where there is a touchscreen (caps, as the registry's needs)
        let found = if common::caps_ok(Some("touchscreen")) { p.get("input.tablets", json!({})).await.unwrap_or(Value::Null) } else { Value::Null };
        if !p.alive() {
            return Content::new();
        }
        {
            // a touchscreen plugged in or out; any other caps change leaves the page (and a slider in use) alone
            let (pp, touch) = (p.clone(), common::caps()["touchscreen"].clone());
            p.on("caps.changed", move |data| {
                if data["caps"].is_object() && data["caps"]["touchscreen"] != touch {
                    pp.refresh();
                }
            });
        }
        let touchscreens: Vec<Value> = found["touchscreens"].as_array().cloned().unwrap_or_default();
        let monitors: Vec<Value> = found["monitors"].as_array().cloned().unwrap_or_default();
        let touch_keys: Vec<(GKey, String)> = touchscreens
            .iter()
            .map(|ts| (gs::at(TOUCHSCREEN, &touchscreen_path(common::s(&ts["vidpid"])), "output"), common::s(&ts["name"]).to_owned()))
            .collect();
        let mut keys = vec![SIZE, LOCATE];
        keys.extend(touch_keys.iter().map(|(k, _)| k.clone()));
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        let group = SettingsGroup::new(t("Muisaanwijzer", "Mouse pointer"), "");
        if gs.available(&SIZE) {
            let slider = gs.slider(&SIZE, 1.0, CURSOR_SIZES.len() as f64, 1.0, size_ui, size_gs);
            let value_l = common::value_label(&size_text(size_gs(size_ui(&gs.value(&SIZE))).as_i64().unwrap_or(24)));
            value_l.set_widget_name("st-mousepointer-size-value");
            // a fixed width: a label that grows or shrinks with the size would move the slider under the pointer
            // while it is dragged (the harness's first click landed one step off that way)
            value_l.set_width_chars(CURSOR_SIZES.iter().map(|px| size_text(*px).chars().count()).max().unwrap_or(16) as i32);
            value_l.set_xalign(0.0);
            {
                let value_l = value_l.clone();
                slider.connect_changed(move |n| value_l.set_label(&size_text(size_gs(n).as_i64().unwrap_or(24))));
            }
            {
                let value_l = value_l.clone();
                gs.follow(&SIZE, move |v| value_l.set_label(&size_text(size_gs(size_ui(v)).as_i64().unwrap_or(24))));
            }
            let row = between(&slider, t("Klein", "Small"), t("Groot", "Large"));
            row.append(&value_l);
            let card = SettingsCard::new("mouse", t("Grootte", "Size"), t("De muisaanwijzer groter maken", "Make the mouse pointer bigger"));
            card.set_control(&row);
            group.add(card.widget());
        }
        if let Some(card) = gs.toggle_card(
            &LOCATE,
            "eye",
            t("Muisaanwijzerindicator", "Mouse indicator"),
            t("Locatie van aanwijzer weergeven wanneer ik op de Ctrl-toets druk", "Show location of pointer when I press the Ctrl key"),
            false,
        ) {
            group.add(card.widget());
        }
        if group.items().first_child().is_some() {
            out.push(group.widget().clone().upcast());
        }

        // Aanraken: GNOME's touchscreen mapping in place of Windows' touch indicator (module docs)
        if let Some(touch) = touchscreen_group(&gs, &touch_keys, &monitors) {
            out.push(touch);
        }

        let related = SettingsGroup::new(ct::related(), "");
        related.add(
            common::link_card_with(&p, "mouse", None, Some(t("Snelheid van muisaanwijzer, primaire knop, schuiven", "Mouse pointer speed, primary button, scrolling")), None).widget(),
        );
        out.push(related.widget().clone().upcast());
        out
    })
}

fn textcursor(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [BLINK, BLINK_TIME];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let group = SettingsGroup::new(t("Knipperen", "Blinking"), "");
        if let Some(card) = gs.toggle_card(&BLINK, "text-cursor", t("Knipperende tekstcursor", "Blinking text cursor"), t("De cursor knippert in tekstvakken", "The cursor blinks in text boxes"), false) {
            group.add(card.widget());
        }
        if gs.available(&BLINK_TIME) {
            let slider = gs.slider(&BLINK_TIME, 100.0, 2500.0, 100.0, blink_ui, blink_gs);
            let card = SettingsCard::new("", t("Knippersnelheid van cursor", "Cursor blink rate"), t("Hoe snel de tekstcursor knippert", "How fast the text cursor blinks"));
            card.set_control(&between(&slider, t("Langzaam", "Slow"), t("Snel", "Fast")));
            // no speed while the cursor does not blink, as GNOME Settings
            let sync = {
                let (g, slider) = (gs.clone(), slider.clone());
                move || slider.set_sensitive(g.writable(&BLINK_TIME) && g.value(&BLINK) != false)
            };
            sync();
            gs.follow(&BLINK, move |_| sync());
            group.add(card.widget());
            let test = c::text_box(t("Typ hier om de cursor te zien", "Type here to see the cursor"));
            test.add_css_class("st-a11y-test-box");
            test.set_widget_name("st-textcursor-test");
            let card = SettingsCard::new("", t("Testvak", "Test box"), t("Klik in het vak om de cursor te zien knipperen", "Select the box to see the cursor blink"));
            card.set_control(&test);
            group.add(card.widget());
        }
        if group.items().first_child().is_some() {
            out.push(group.widget().clone().upcast());
        } else {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("text-cursor", t("Tekstcursor", "Text cursor"), None).widget());
            out.push(g.widget().clone().upcast());
        }
        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "keyboard-advanced").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

// ------------------------------------------------------------------------------------------ Vergrootglas

fn magnifier_keys() -> Vec<GKey> {
    let mut keys = vec![ZOOM, FACTOR, POSITION, LENS, EDGES, MOUSE_TRACK, FOCUS_TRACK, CARET_TRACK, INVERT, SATURATION];
    keys.extend(BRIGHTNESS);
    keys.extend(CONTRAST);
    keys.extend([CROSS, CROSS_CLIP, CROSS_THICK, CROSS_LENGTH, CROSS_COLOR, CROSS_OPACITY]);
    keys.extend(ZOOM_KEYS);
    keys
}

/// Zoomniveau: − and + around the level in % (GNOME's mag-factor in quarters).
fn zoom_level_card(gs: &Gs) -> SettingsCard {
    let card = SettingsCard::new("zoom-in", t("Zoomniveau", "Zoom level"), t("Hoe sterk het vergrootglas vergroot", "How far Magnifier zooms in"));
    let minus = c::icon_button("minus", t("Uitzoomen", "Zoom out"), 32);
    minus.set_widget_name("st-magnifier-zoom-out");
    let plus = c::icon_button("plus", t("Inzoomen", "Zoom in"), 32);
    plus.set_widget_name("st-magnifier-zoom-in");
    let value = common::value_label(&factor_text(gs.value(&FACTOR).as_f64().unwrap_or(2.0)));
    value.set_widget_name("st-magnifier-mag-factor");
    value.add_css_class("st-a11y-zoom-value");
    value.set_width_chars(factor_text(FACTOR_MAX).chars().count() as i32);
    let writable = gs.writable(&FACTOR);
    let sync: Rc<dyn Fn(f64)> = {
        let (minus, plus, value) = (minus.clone(), plus.clone(), value.clone());
        Rc::new(move |v: f64| {
            value.set_label(&factor_text(v));
            minus.set_sensitive(writable && v > FACTOR_MIN + 1e-6);
            plus.set_sensitive(writable && v < FACTOR_MAX - 1e-6);
        })
    };
    sync(gs.value(&FACTOR).as_f64().unwrap_or(2.0));
    if !writable {
        value.set_tooltip_text(Some(gs::locked_text()));
    }
    for (b, up) in [(&minus, false), (&plus, true)] {
        let (g, sync) = (gs.clone(), sync.clone());
        b.connect_clicked(move |_| {
            let next = factor_step(g.value(&FACTOR).as_f64().unwrap_or(2.0), up);
            sync(next);
            let (g2, sync2) = (g.clone(), sync.clone());
            g.set(&FACTOR, json!(next), move || sync2(g2.value(&FACTOR).as_f64().unwrap_or(2.0)));
        });
    }
    gs.follow(&FACTOR, move |v| sync(v.as_f64().unwrap_or(2.0)));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&minus);
    row.append(&value);
    row.append(&plus);
    card.set_control(&row);
    card
}

/// Weergave: Volledig scherm / Vastgezet / Lens (lens-mode with screen-position, written together); Schermgebied, the
/// half, while the view is not the full screen.
fn view_cards(gs: &Gs, keys: &[GKey]) -> Vec<SettingsCard> {
    if !gs.available(&LENS) || !gs.available(&POSITION) {
        return Vec::new();
    }
    let current = view_of(&gs.value(&LENS), &gs.value(&POSITION));
    let options: Vec<DropdownOption> = view_options().into_iter().map(|(v, l)| DropdownOption::new(v, l)).collect();
    let dd = Dropdown::new(options, Some(current));
    bind_state(gs, keys, dd.widget(), &LENS);
    dd.widget().set_widget_name("st-magnifier-view");
    let writable = gs.writable(&LENS) && gs.writable(&POSITION);
    dd.widget().set_sensitive(writable);
    let area = gs.dropdown(&POSITION, area_options());
    area.set_placeholder(t("Volledig scherm", "Full screen"));
    let sync: Rc<dyn Fn()> = {
        let (g, dd, area) = (gs.clone(), dd.clone(), area.clone());
        Rc::new(move || {
            let view = view_of(&g.value(&LENS), &g.value(&POSITION));
            dd.set_selected(Some(view));
            area.widget().set_sensitive(g.writable(&POSITION) && view != "full");
        })
    };
    sync();
    {
        let (g, sync) = (gs.clone(), sync.clone());
        dd.connect_changed(move |view| {
            let sync2 = sync.clone();
            write_together(&g, &[LENS, POSITION], view_values(view, &g.value(&POSITION)), move || sync2());
        });
    }
    for k in [&LENS, &POSITION] {
        let sync = sync.clone();
        gs.follow(k, move |_| sync());
    }
    let view_card = SettingsCard::new("view", t("Weergave", "View"), t("Waar het vergrote beeld wordt weergegeven", "Where the magnified view appears"));
    view_card.set_control(&common::sized(dd.widget(), 200));
    let area_card = SettingsCard::new(
        "display",
        t("Schermgebied", "Screen area"),
        t("De helft van het scherm voor de vastgezette weergave of de lens", "The half of the screen for the docked view or the lens"),
    );
    area_card.set_control(&common::sized(area.widget(), 200));
    vec![view_card, area_card]
}

/// De muisaanwijzer houden (mouse-tracking's way) and Vergrootglas laten volgen (the three tracking keys).
fn follow_expanders(gs: &Gs, keys: &[GKey]) -> Vec<SettingsExpander> {
    let mut out = Vec::new();
    if !gs.available(&MOUSE_TRACK) {
        return out;
    }
    let now = gs.value(&MOUSE_TRACK);
    let last = Rc::new(RefCell::new(if tracks(&now) { common::s(&now).to_owned() } else { "proportional".to_owned() }));
    let radio = RadioGroup::new(&keep_options(), Some(last.borrow().as_str()), None, false);
    bind_state(gs, keys, radio.widget(), &MOUSE_TRACK);
    radio.widget().set_sensitive(gs.writable(&MOUSE_TRACK) && tracks(&now));
    {
        let (g, last, radio2) = (gs.clone(), last.clone(), radio.clone());
        radio.connect_changed(move |way| {
            *last.borrow_mut() = way.to_owned();
            let (g2, radio3) = (g.clone(), radio2.clone());
            g.set(&MOUSE_TRACK, json!(way), move || radio3.set_value(common::s(&g2.value(&MOUSE_TRACK))));
        });
    }
    let keep_exp = SettingsExpander::new("mouse", t("De muisaanwijzer houden", "Keep the mouse pointer"), "");
    keep_exp.header_button().set_widget_name("st-magnifier-keep-pointer");
    keep_exp.add(radio.widget());
    out.push(keep_exp);

    let pointer = c::checkbox(t("Muisaanwijzer", "Mouse pointer"));
    pointer.set_active(tracks(&now));
    pointer.set_widget_name("st-magnifier-mouse-tracking-follow");
    pointer.set_sensitive(gs.writable(&MOUSE_TRACK));
    let silent = Rc::new(std::cell::Cell::new(false));
    {
        let (g, last, silent) = (gs.clone(), last.clone(), silent.clone());
        pointer.connect_toggled(move |b| {
            if silent.get() {
                return;
            }
            let way = if b.is_active() { last.borrow().clone() } else { "none".to_owned() };
            let (b2, g2, s2) = (b.clone(), g.clone(), silent.clone());
            g.set(&MOUSE_TRACK, json!(way), move || {
                s2.set(true);
                b2.set_active(tracks(&g2.value(&MOUSE_TRACK)));
                s2.set(false);
            });
        });
    }
    {
        let (g, radio, pointer, silent) = (gs.clone(), radio.clone(), pointer.clone(), silent.clone());
        gs.follow(&MOUSE_TRACK, move |v| {
            if tracks(v) {
                *last.borrow_mut() = common::s(v).to_owned();
                radio.set_value(common::s(v));
            }
            radio.widget().set_sensitive(g.writable(&MOUSE_TRACK) && tracks(v));
            silent.set(true);
            pointer.set_active(tracks(v));
            silent.set(false);
        });
    }
    let boxes = gtk::Box::new(gtk::Orientation::Vertical, 8);
    boxes.append(&pointer);
    if gs.available(&FOCUS_TRACK) {
        boxes.append(&mapped_checkbox(gs, keys, &FOCUS_TRACK, t("Toetsenbordfocus", "Keyboard focus"), tracks, focus_value));
    }
    if gs.available(&CARET_TRACK) {
        boxes.append(&mapped_checkbox(gs, keys, &CARET_TRACK, t("Tekstcursor", "Text cursor"), tracks, caret_value));
    }
    let follow_exp = SettingsExpander::new("focus", t("Vergrootglas laten volgen", "Have Magnifier follow"), "");
    follow_exp.header_button().set_widget_name("st-magnifier-follow");
    follow_exp.add(&boxes);
    out.push(follow_exp);
    out
}

/// A slider that writes the red, green and blue keys of one colour effect together (GNOME Settings' Brightness and
/// Contrast); it shows the red one.
fn rgb_slider(gs: &Gs, keys: &[GKey; 3], name: &str) -> Option<Slider> {
    if !keys.iter().all(|k| gs.available(k)) {
        return None;
    }
    let keys = Rc::new(keys.clone());
    let shown = |g: &Gs, keys: &[GKey; 3]| g.value(&keys[0]).as_f64().unwrap_or(0.0).clamp(-0.8, 0.8);
    let s = Slider::new(-0.8, 0.8, 0.02, shown(gs, &*keys));
    s.set_value_visible(false);
    s.scale().set_widget_name(name);
    let writable = keys.iter().all(|k| gs.writable(k));
    s.set_sensitive(writable);
    if !writable {
        s.scale().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (g, s2, keys) = (gs.clone(), s.clone(), keys.clone());
        s.connect_committed(move |n| {
            let v = round2(n);
            let values: serde_json::Map<String, Value> = keys.iter().map(|k| (k.key.to_owned(), json!(v))).collect();
            let (g2, s3, keys2) = (g.clone(), s2.clone(), keys.clone());
            write_together(&g, &*keys, Value::Object(values), move || s3.set_value(shown(&g2, &*keys2)));
        });
    }
    let s2 = s.clone();
    gs.follow(&keys[0], move |v| s2.set_value(v.as_f64().unwrap_or(0.0).clamp(-0.8, 0.8)));
    Some(s)
}

/// Kleuraanpassing (GNOME's Color Filters without Inverted, which is Windows' Kleuren omkeren above).
fn colour_expander(gs: &Gs) -> Option<SettingsExpander> {
    let exp = SettingsExpander::new(
        "color",
        t("Kleuraanpassing", "Colour adjustment"),
        t("Helderheid, contrast en kleur in het vergrote gebied", "Brightness, contrast and colour in the magnified area"),
    );
    exp.header_button().set_widget_name("st-magnifier-colour");
    let mut any = false;
    if let Some(s) = rgb_slider(gs, &BRIGHTNESS, "st-magnifier-brightness") {
        exp.add(nested_card(t("Helderheid", "Brightness"), "", &scale_row(&s, t("Donker", "Dark"), t("Licht", "Light"), None)).widget());
        any = true;
    }
    if let Some(s) = rgb_slider(gs, &CONTRAST, "st-magnifier-contrast") {
        exp.add(nested_card(t("Contrast", "Contrast"), "", &scale_row(&s, t("Laag", "Low"), t("Hoog", "High"), None)).widget());
        any = true;
    }
    if gs.available(&SATURATION) {
        let s = gs.slider(&SATURATION, 0.0, 1.0, 0.01, |v| v.as_f64().unwrap_or(1.0), |n| json!(round2(n)));
        exp.add(nested_card(t("Kleur", "Colour"), "", &scale_row(&s, t("Grijswaarden", "Greyscale"), t("Volledige kleur", "Full colour"), None)).widget());
        any = true;
    }
    any.then_some(exp)
}

/// The crosshairs' colour: a swatch with the colour's name, opening the colour picker.
fn colour_button(gs: &Gs) -> gtk::Button {
    let hex = Rc::new(RefCell::new(gs.value(&CROSS_COLOR).as_str().and_then(kit::theme::normalize_hex).unwrap_or_else(|| "#ff0000".into())));
    let swatch = gtk::DrawingArea::new();
    swatch.add_css_class("st-a11y-swatch");
    swatch.set_content_width(24);
    swatch.set_content_height(16);
    {
        let hex = hex.clone();
        swatch.set_draw_func(move |_, cr, w, h| {
            let (r, g, b) = rgb_of(&hex.borrow());
            cr.set_source_rgb(r, g, b);
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(0.5, 0.5, 0.5, 0.6);
            cr.set_line_width(1.0);
            let _ = cr.stroke();
        });
    }
    let name = gtk::Label::new(Some(&common::color_name(&hex.borrow())));
    let inner = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    inner.append(&swatch);
    inner.append(&name);
    let b = gtk::Button::new();
    b.set_child(Some(&inner));
    b.set_widget_name("st-magnifier-cross-hairs-color");
    b.set_valign(gtk::Align::Center);
    b.set_sensitive(gs.writable(&CROSS_COLOR));
    if !gs.writable(&CROSS_COLOR) {
        b.set_tooltip_text(Some(gs::locked_text()));
    }
    let show: Rc<dyn Fn(&str)> = {
        let (hex, swatch, name) = (hex.clone(), swatch.clone(), name.clone());
        Rc::new(move |v: &str| {
            if let Some(n) = kit::theme::normalize_hex(v) {
                name.set_label(&common::color_name(&n));
                *hex.borrow_mut() = n;
                swatch.queue_draw();
            }
        })
    };
    {
        let (g, hex, show) = (gs.clone(), hex.clone(), show.clone());
        b.connect_clicked(move |_| {
            let (g2, show2) = (g.clone(), show.clone());
            let current = hex.borrow().clone();
            crate::colorpicker::pick_color(&g.page().anchor(), &current, Some(t("Kleur van dradenkruis", "Crosshair colour")), false, move |chosen| {
                let Some(n) = chosen.as_deref().and_then(kit::theme::normalize_hex) else { return };
                show2(&n);
                let (g3, show3) = (g2.clone(), show2.clone());
                g2.set(&CROSS_COLOR, json!(n), move || show3(common::s(&g3.value(&CROSS_COLOR))));
            });
        });
    }
    gs.follow(&CROSS_COLOR, move |v| show(common::s(v)));
    b
}

/// Dradenkruis (GNOME's Crosshairs), with its switch on the header.
fn crosshair_expander(gs: &Gs) -> Option<SettingsExpander> {
    let (exp, _) = switch_expander(
        gs,
        &CROSS,
        "focus",
        t("Dradenkruis", "Crosshairs"),
        t("De plaats van de muisaanwijzer in het vergrote gebied met lijnen aangeven", "Mark the pointer’s location in the magnified area with lines"),
    )?;
    if gs.available(&CROSS_CLIP) {
        let overlap = gs.checkbox(&CROSS_CLIP, t("Over muisaanwijzer heen", "Over the mouse pointer"), true);
        let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
        b.append(&overlap);
        exp.add(&b);
    }
    if gs.available(&CROSS_THICK) {
        let s = gs.slider(&CROSS_THICK, 1.0, 100.0, 1.0, |v| v.as_f64().unwrap_or(8.0), |n| json!(n.round() as i64));
        let value = value_text(gs, &s, &CROSS_THICK, |v| v.as_f64().unwrap_or(8.0), px_text, "100 px");
        exp.add(nested_card(t("Dikte", "Thickness"), "", &scale_row(&s, t("Dun", "Thin"), t("Dik", "Thick"), Some(&value))).widget());
    }
    if gs.available(&CROSS_LENGTH) {
        let s = gs.slider(&CROSS_LENGTH, 0.0, 100.0, 1.0, length_ui, length_gs);
        let value = value_text(gs, &s, &CROSS_LENGTH, length_ui, percent_text, "100%");
        exp.add(nested_card(t("Lengte", "Length"), "", &scale_row(&s, t("Kort", "Short"), t("Lang", "Long"), Some(&value))).widget());
    }
    if gs.available(&CROSS_COLOR) {
        exp.add(nested_card(t("Kleur", "Colour"), "", &colour_button(gs)).widget());
    }
    if gs.available(&CROSS_OPACITY) {
        let s = gs.slider(&CROSS_OPACITY, 0.0, 100.0, 1.0, |v| (v.as_f64().unwrap_or(0.66) * 100.0).round(), |n| json!(n.round() / 100.0));
        let value = value_text(gs, &s, &CROSS_OPACITY, |v| (v.as_f64().unwrap_or(0.66) * 100.0).round(), percent_text, "100%");
        exp.add(nested_card(t("Dekking", "Opacity"), "", &scale_row(&s, t("Doorzichtig", "Clear"), t("Ondoorzichtig", "Solid"), Some(&value))).widget());
    }
    Some(exp)
}

fn magnifier(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = magnifier_keys();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        keep(&p, &gs);
        let mut out: Content = Vec::new();

        let top = SettingsGroup::new("", "");
        if let Some((exp, _)) = switch_expander(&gs, &ZOOM, "magnifier", t("Vergrootglas", "Magnifier"), t("Delen van het scherm vergroten", "Zoom in on parts of the screen")) {
            let shortcuts = [
                (&ZOOM_KEYS[0], t("Vergrootglas in- of uitschakelen", "Turn Magnifier on or off")),
                (&ZOOM_KEYS[1], t("Inzoomen", "Zoom in")),
                (&ZOOM_KEYS[2], t("Uitzoomen", "Zoom out")),
            ];
            for (k, title) in shortcuts {
                if let Some(card) = shortcut_card(&p, &gs, k, title) {
                    exp.add(card.widget());
                }
            }
            top.add(exp.widget());
        }
        if gs.available(&FACTOR) {
            top.add(zoom_level_card(&gs).widget());
        }
        if let Some(card) = gs.toggle_card(&INVERT, "color", t("Kleuren omkeren", "Invert colours"), t("De kleuren in het vergrote gebied omkeren", "Invert the colours in the magnified area"), false) {
            top.add(card.widget());
        }
        out.extend(filled(top));

        let view = SettingsGroup::new(t("Weergave", "View"), "");
        for card in view_cards(&gs, &keys) {
            view.add(card.widget());
        }
        if let Some(card) = gs.toggle_card(
            &EDGES,
            "desktop",
            t("Buiten de schermranden doorgaan", "Extend outside screen edges"),
            t("Het vergrote beeld kan voorbij de rand van het scherm schuiven", "The magnified view can move past the edge of the screen"),
            false,
        ) {
            view.add(card.widget());
        }
        for exp in follow_expanders(&gs, &keys) {
            view.add(exp.widget());
        }
        out.extend(filled(view));

        let effects = SettingsGroup::new(t("Kleur en dradenkruis", "Colour and crosshairs"), "");
        if let Some(exp) = colour_expander(&gs) {
            effects.add(exp.widget());
        }
        if let Some(exp) = crosshair_expander(&gs) {
            effects.add(exp.widget());
        }
        out.extend(filled(effects));

        if out.is_empty() {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("magnifier", t("Vergrootglas", "Magnifier"), None).widget());
            out.push(g.widget().clone().upcast());
        }
        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "textsize").widget());
        related.add(common::link_card(&p, "mousepointer").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

// ------------------------------------------------------------------------------------------ Verteller

/// Instellingen voor Verteller openen: Orca's own preferences, while the screen reader runs (GNOME's Configure).
fn orca_card(p: &Page, gs: &Gs) -> SettingsCard {
    let installed = common::caps()["orca"] == true;
    let card = SettingsCard::new(
        "settings",
        t("Instellingen voor Verteller openen", "Open Narrator settings"),
        if installed {
            t("Stem, snelheid, uitgebreidheid, braille en sneltoetsen in de voorkeuren van Orca", "Voice, speed, verbosity, braille and keyboard commands in Orca’s preferences")
        } else {
            t("Orca, de schermlezer van GNOME, is niet geïnstalleerd", "Orca, GNOME’s screen reader, isn’t installed")
        },
    );
    let open = c::button_with_glyph("external", t("Openen", "Open"), Variant::Standard);
    open.set_widget_name("st-narrator-preferences");
    let sync: Rc<dyn Fn(&Value)> = {
        let open = open.clone();
        Rc::new(move |on: &Value| {
            let on = *on == true;
            open.set_sensitive(installed && on);
            open.set_tooltip_text((installed && !on).then_some(t("Schakel Verteller in om de instellingen te openen", "Turn on Narrator to open its settings")));
        })
    };
    sync(&gs.value(&READER));
    {
        let sync = sync.clone();
        gs.follow(&READER, move |v| sync(v));
    }
    {
        let (pp, g) = (p.clone(), gs.clone());
        open.connect_clicked(move |b| {
            b.set_sensitive(false);
            let wait = pp.run_with("input.screenReader.preferences", json!({}), Some(t("Instellingen voor Verteller openen", "Open Narrator settings")), true);
            let (g2, sync2) = (g.clone(), sync.clone());
            pp.spawn(async move {
                let _ = wait.await;
                sync2(&g2.value(&READER));
            });
        });
    }
    card.set_control(&open);
    card
}

fn narrator(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [READER, READER_KEY];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        keep(&p, &gs);
        let mut out: Content = Vec::new();
        let Some((exp, _)) = switch_expander(&gs, &READER, "narrator", t("Verteller", "Narrator"), t("Een schermlezer die beschrijft wat er op het scherm staat", "A screen reader that describes what’s on your screen")) else {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("narrator", t("Verteller", "Narrator"), None).widget());
            out.push(g.widget().clone().upcast());
            return out;
        };
        exp.set_expanded(true);
        if let Some(card) = shortcut_card(&p, &gs, &READER_KEY, t("Sneltoets voor Verteller", "Keyboard shortcut for Narrator")) {
            exp.add(card.widget());
        }
        let note = c::label(
            t("Verteller is Orca, de schermlezer van GNOME. GNOME Shell start Orca zodra Verteller wordt ingeschakeld.", "Narrator is Orca, GNOME’s screen reader. GNOME Shell starts Orca as soon as Narrator is turned on."),
            "w11-text-2",
            true,
        );
        note.add_css_class("st-expander-note");
        note.add_css_class("st-a11y-note");
        note.set_xalign(0.0);
        exp.add(&note);
        let top = SettingsGroup::new("", "");
        top.add(exp.widget());
        out.push(top.widget().clone().upcast());

        let settings = SettingsGroup::new(t("Stem, uitgebreidheid en braille", "Voice, verbosity and braille"), "");
        settings.add(orca_card(&p, &gs).widget());
        out.push(settings.widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "magnifier").widget());
        related.add(common::link_card(&p, "a11y-keyboard").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pointer_size_has_gnomes_five_sizes() {
        let steps: Vec<Value> = (1..=5).map(|n| size_gs(n as f64)).collect();
        assert_eq!(steps, [json!(24), json!(32), json!(48), json!(64), json!(96)]);
        for (px, step) in [(24, 1.0), (32, 2.0), (48, 3.0), (64, 4.0), (96, 5.0), (40, 2.0), (128, 5.0), (16, 1.0)] {
            assert_eq!(size_ui(&json!(px)), step, "{px} px");
        }
        assert_eq!(size_gs(9.0), json!(96), "clamped to the last size");
        assert!(size_text(24).starts_with("24 px ("));
        assert_eq!(gs::control_name("mousepointer", &[SIZE, LOCATE], &SIZE), "st-mousepointer-cursor-size");
        let ts = gs::at(TOUCHSCREEN, &touchscreen_path("04f3:2072"), "output");
        assert_eq!(gs::control_name("mousepointer", &[SIZE, LOCATE, ts.clone()], &ts), "st-mousepointer-output");
    }

    #[test]
    fn the_blink_rate_is_gnomes_blink_time_slow_to_fast() {
        assert_eq!(blink_ui(&json!(1200)), 1400.0, "GNOME's default in the middle");
        assert_eq!(blink_ui(&json!(2500)), 100.0, "Langzaam on the left");
        assert_eq!(blink_ui(&json!(100)), 2500.0, "Snel on the right");
        for ms in [100, 1200, 2500, 500] {
            assert_eq!(blink_gs(blink_ui(&json!(ms))), json!(ms));
        }
        assert_eq!(blink_gs(0.0), json!(2500));
        assert_eq!(gs::control_name("textcursor", &[BLINK, BLINK_TIME], &BLINK_TIME), "st-textcursor-cursor-blink-time");
    }

    #[test]
    fn the_zoom_level_steps_in_gnomes_quarters() {
        assert_eq!(factor_text(2.0), "200%");
        assert_eq!(factor_text(1.25), "125%");
        assert_eq!(factor_step(2.0, true), 2.25);
        assert_eq!(factor_step(2.0, false), 1.75);
        assert_eq!(factor_step(1.1, true), 1.25, "a factor of GNOME Shell's zoom keys steps onto the grid");
        assert_eq!(factor_step(1.1, false), 1.0);
        assert_eq!(factor_step(1.0, false), 1.0, "100 % is the lowest");
        assert_eq!(factor_step(20.0, true), 20.0, "2000 % the highest");
        assert_eq!(factor_step(25.0, false), 20.0, "a factor above GNOME Settings' range comes back into it");
    }

    #[test]
    fn the_view_is_lens_mode_with_the_screen_position() {
        assert_eq!(view_of(&json!(false), &json!("full-screen")), "full");
        assert_eq!(view_of(&json!(false), &json!("none")), "full");
        assert_eq!(view_of(&json!(false), &json!("left-half")), "docked");
        assert_eq!(view_of(&json!(true), &json!("full-screen")), "lens");
        assert_eq!(view_values("full", &json!("left-half")), json!({"lens-mode": false, "screen-position": "full-screen"}));
        assert_eq!(view_values("docked", &json!("left-half")), json!({"lens-mode": false, "screen-position": "left-half"}));
        assert_eq!(view_values("docked", &json!("full-screen")), json!({"lens-mode": false, "screen-position": "top-half"}), "Windows' docked view is on top");
        assert_eq!(view_values("lens", &json!("full-screen")), json!({"lens-mode": true, "screen-position": "top-half"}), "a lens over the full screen does nothing in GNOME Shell");
        assert_eq!(view_values("lens", &json!("bottom-half")), json!({"lens-mode": true, "screen-position": "bottom-half"}));
        for (view, _) in view_options() {
            let values = view_values(view, &json!("right-half"));
            assert_eq!(view_of(&values["lens-mode"], &values["screen-position"]), view, "{view} round trip");
        }
        let areas: Vec<Value> = area_options().into_iter().map(|(v, _)| v).collect();
        assert_eq!(areas, HALVES.iter().map(|h| json!(h)).collect::<Vec<_>>());
    }

    #[test]
    fn tracking_is_gnomes_follow_behavior() {
        assert_eq!(keep_options().iter().map(|(v, _)| *v).collect::<Vec<_>>(), ["centered", "push", "proportional"]);
        assert!(tracks(&json!("proportional")));
        assert!(!tracks(&json!("none")));
        assert_eq!(focus_value(true), json!("proportional"), "focus-tracking's default");
        assert_eq!(caret_value(true), json!("centered"), "caret-tracking's default");
        assert_eq!(focus_value(false), json!("none"));
    }

    #[test]
    fn the_crosshairs_map_onto_gnomes_keys() {
        assert_eq!(length_ui(&json!(4096)), 100.0);
        assert_eq!(length_ui(&json!(20)), 0.0);
        assert_eq!(length_gs(100.0), json!(4096));
        assert_eq!(length_gs(0.0), json!(20), "the schema's minimum");
        assert_eq!(length_gs(50.0), json!(2058));
        for pct in [0.0, 13.0, 50.0, 100.0] {
            assert_eq!(length_ui(&length_gs(pct)), pct, "{pct} %");
        }
        assert_eq!(rgb_of("#ff0000"), (1.0, 0.0, 0.0));
        assert_eq!(rgb_of("#00ff00"), (0.0, 1.0, 0.0));
        assert_eq!(rgb_of("nonsense"), (1.0, 0.0, 0.0), "GNOME's red");
        assert_eq!(round2(0.123456), 0.12);
        assert_eq!(round2(-0.799), -0.8);
    }

    #[test]
    fn every_magnifier_key_has_a_control_name_of_its_own() {
        let keys = magnifier_keys();
        let names: std::collections::HashSet<String> = keys.iter().map(|k| gs::control_name("magnifier", &keys, k)).collect();
        assert_eq!(names.len(), keys.len());
        assert_eq!(gs::control_name("magnifier", &keys, &ZOOM), "st-magnifier-screen-magnifier-enabled");
        assert_eq!(gs::control_name("magnifier", &keys, &INVERT), "st-magnifier-invert-lightness");
        assert_eq!(gs::control_name("narrator", &[READER, READER_KEY], &READER), "st-narrator-screen-reader-enabled");
    }
}
