//! Instellingen › Toegankelijkheid › Audio, Toetsenbord, Muis (batch A, notes/claude/settings-parity.md §2, plan.json
//! "a11y-audio", "a11y-keyboard", "a11y-mouse"): Windows 11's Hearing and Interaction pages with the functions of
//! GNOME Settings 50's Hearing, Typing and Pointing & Clicking pages they hold.
//!
//! * Audio: Mijn scherm laten knipperen tijdens audiomeldingen = `org.gnome.desktop.wm.preferences visual-bell` with
//!   `visual-bell-type` (GNOME's Visual Alerts and Flash Area, written together: Uit = visual-bell false, Het actieve
//!   venster = frame-flash, Het hele scherm = fullscreen-flash; Windows' title bar choice has no GNOME value of its
//!   own and is left out) and Testen (GNOME's Test Flash: the window's bell, which Mutter shows as the alert);
//!   WinPoint addition: Overversterking = `org.gnome.desktop.sound allow-volume-above-100-percent` (GNOME's
//!   Overamplification; batch D's row, the same key as on Geluid). Windows' Mono-audio has no GNOME function.
//! * Toetsenbord, Windows' order: Plaktoetsen, Filtertoetsen and Wisseltoetsen as expanders with their switch.
//!   Plaktoetsen = `org.gnome.desktop.a11y.keyboard stickykeys-enable` with Plaktoetsen uitschakelen wanneer twee
//!   toetsen tegelijk worden ingedrukt (`stickykeys-two-key-off`), Een geluid afspelen ... (`stickykeys-modifier-beep`)
//!   and amendment 14's note (shortcutBridge.js does not open Start with the Windows key while Sticky Keys is on).
//!   Filtertoetsen is on while GNOME's Slow Keys or Bounce Keys is (its switch turns Slow Keys on, or both off), with
//!   Korte toetsaanslagen negeren (`slowkeys-enable`, Acceptatievertraging 0-500 ms, the three beeps) and Onbedoelde
//!   toetsaanslagen negeren (`bouncekeys-enable`, 0-900 ms, the reject beep), and Tekenherhaling → Geavanceerde
//!   toetsenbordinstellingen (GNOME's Repeat Keys). Wisseltoetsen = `togglekeys-enable` (GNOME's Sound Keys).
//!   Amendment 24: Windows' Sneltoets voor Plaktoetsen / Filtertoetsen / Wisseltoetsen is one GNOME key (`enable`,
//!   Enable by Keyboard), so the row is in all three expanders with a note that it covers all three, and there is no
//!   separate row. Schermtoetsenbord en Print Screen: Schermtoetsenbord = `org.gnome.desktop.a11y.applications
//!   screen-keyboard-enabled`; De Print Screen-toets gebruiken om schermopname te openen = whether GNOME Shell's
//!   `show-screenshot-ui` holds Print (written through `input.shortcuts.set`, which checks conflicts). Windows'
//!   Toegangstoetsen onderstrepen has no GNOME function (GTK 4 shows them while Alt is held). Meldingsvoorkeuren: Een
//!   geluid afspelen wanneer ik ... via het toetsenbord in- of uitschakel = `feature-state-change-beep` (GNOME's key
//!   without a GNOME Settings control).
//! * Muis: Muistoetsen = `mousekeys-enable` (GNOME's Mouse Keys) with Windows' two sliders on GNOME's keys without a
//!   GNOME Settings control: Snelheid van muistoetsen = `mousekeys-max-speed` (10-1000 px per second) and Versnelling
//!   van muistoetsen = `mousekeys-accel-time` (3000-100 ms to the full speed, Langzaam to Snel). WinPoint additions
//!   in Windows' Control Panel words: Een venster activeren door de muisaanwijzer erop te plaatsen = `focus-mode`
//!   (on writes 'sloppy', off 'click', as GNOME's Activate Windows on Hover), Dubbelkliksnelheid = the mouse's
//!   `double-click` (mouse.rs' 1-10 = 1000-100 ms, GNOME's Double-Click Delay). Klikhulp (GNOME's Click Assist):
//!   Gesimuleerde secundaire klik (`org.gnome.desktop.a11y.mouse secondary-click-enabled`, Acceptatievertraging
//!   0,5-3,0 s) and Klikken door te zweven (`dwell-click-enabled`, Vertraging 0,2-3,0 s, Bewegingsdrempel 0-30 px).
//!   GNOME's Locate Pointer is on Muisaanwijzer en aanraken (a related link), as in Windows.
use super::a11y_vision::{keep, scale_row, switch_expander, value_text, write_together};
use super::mouse::{self, bind_state, mapped_toggle};
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Chevron, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Slider, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("a11y-audio", a11y_audio), ("a11y-keyboard", a11y_keyboard), ("a11y-mouse", a11y_mouse)];
pub const CSS: &str = r#"
.st-a11y-checks { padding-top: 4px; padding-bottom: 4px; }
"#;

const WM: &str = "org.gnome.desktop.wm.preferences";
const KB: &str = "org.gnome.desktop.a11y.keyboard";
const A11Y_MOUSE: &str = "org.gnome.desktop.a11y.mouse";
const BELL: GKey = gs::key(WM, "visual-bell");
const BELL_TYPE: GKey = gs::key(WM, "visual-bell-type");
const LOUD: GKey = gs::key("org.gnome.desktop.sound", "allow-volume-above-100-percent");
const STICKY: GKey = gs::key(KB, "stickykeys-enable");
const TWO_KEY: GKey = gs::key(KB, "stickykeys-two-key-off");
const MOD_BEEP: GKey = gs::key(KB, "stickykeys-modifier-beep");
const SLOW: GKey = gs::key(KB, "slowkeys-enable");
const SLOW_DELAY: GKey = gs::key(KB, "slowkeys-delay");
const SLOW_PRESS: GKey = gs::key(KB, "slowkeys-beep-press");
const SLOW_ACCEPT: GKey = gs::key(KB, "slowkeys-beep-accept");
const SLOW_REJECT: GKey = gs::key(KB, "slowkeys-beep-reject");
const BOUNCE: GKey = gs::key(KB, "bouncekeys-enable");
const BOUNCE_DELAY: GKey = gs::key(KB, "bouncekeys-delay");
const BOUNCE_REJECT: GKey = gs::key(KB, "bouncekeys-beep-reject");
const TOGGLE: GKey = gs::key(KB, "togglekeys-enable");
const ENABLE: GKey = gs::key(KB, "enable");
const STATE_BEEP: GKey = gs::key(KB, "feature-state-change-beep");
const MOUSEKEYS: GKey = gs::key(KB, "mousekeys-enable");
const MOUSEKEYS_SPEED: GKey = gs::key(KB, "mousekeys-max-speed");
const MOUSEKEYS_ACCEL: GKey = gs::key(KB, "mousekeys-accel-time");
const OSK: GKey = gs::key("org.gnome.desktop.a11y.applications", "screen-keyboard-enabled");
const SCREENSHOT: GKey = gs::key("org.gnome.shell.keybindings", "show-screenshot-ui");
const FOCUS_MODE: GKey = gs::key(WM, "focus-mode");
const SECONDARY: GKey = gs::key(A11Y_MOUSE, "secondary-click-enabled");
const SECONDARY_TIME: GKey = gs::key(A11Y_MOUSE, "secondary-click-time");
const DWELL: GKey = gs::key(A11Y_MOUSE, "dwell-click-enabled");
const DWELL_TIME: GKey = gs::key(A11Y_MOUSE, "dwell-time");
const DWELL_THRESHOLD: GKey = gs::key(A11Y_MOUSE, "dwell-threshold");

// ------------------------------------------------------------------------------------------ value mappings

/// Mijn scherm laten knipperen of visual-bell and visual-bell-type: "off", "window" or "screen".
fn flash_of(bell: &Value, kind: &Value) -> &'static str {
    if *bell != true {
        "off"
    } else if kind.as_str() == Some("frame-flash") {
        "window"
    } else {
        "screen"
    }
}

/// The values a flash choice writes, together (Uit keeps the flash area, as GNOME's Visual Alerts switch).
fn flash_values(choice: &str) -> Value {
    match choice {
        "window" => json!({ "visual-bell": true, "visual-bell-type": "frame-flash" }),
        "screen" => json!({ "visual-bell": true, "visual-bell-type": "fullscreen-flash" }),
        _ => json!({ "visual-bell": false }),
    }
}

fn flash_options() -> Vec<(&'static str, &'static str)> {
    vec![
        ("off", t("Uit", "Off")),
        ("window", t("Het actieve venster laten knipperen", "Flash the active window")),
        ("screen", t("Het hele scherm laten knipperen", "Flash the entire screen")),
    ]
}

/// Filtertoetsen is on while Slow Keys or Bounce Keys is.
fn filter_on(slow: &Value, bounce: &Value) -> bool {
    *slow == true || *bounce == true
}

/// What the Filtertoetsen switch writes: on turns Slow Keys on (Windows' Korte toetsaanslagen negeren), off both.
fn filter_values(on: bool) -> Value {
    if on { json!({ "slowkeys-enable": true }) } else { json!({ "slowkeys-enable": false, "bouncekeys-enable": false }) }
}

/// Whether GNOME Shell's interactive screenshot holds the Print key.
fn print_on(v: &Value) -> bool {
    v.as_array().into_iter().flatten().any(|a| a.as_str() == Some("Print"))
}

/// show-screenshot-ui with Print added (first) or taken out; its other keys stay.
fn print_accels(v: &Value, on: bool) -> Vec<String> {
    let others = v.as_array().into_iter().flatten().filter_map(|a| a.as_str()).filter(|a| *a != "Print" && !a.is_empty()).map(str::to_owned);
    if on { std::iter::once("Print".to_owned()).chain(others).collect() } else { others.collect() }
}

/// Activate Windows on Hover: on unless focus-mode is 'click'.
fn hover_on(v: &Value) -> bool {
    v.as_str().is_some_and(|m| m != "click")
}

fn hover_value(on: bool) -> Value {
    json!(if on { "sloppy" } else { "click" })
}

/// Snelheid van muistoetsen: mousekeys-max-speed in px per second, 10-1000.
fn mousekeys_speed_ui(v: &Value) -> f64 {
    v.as_f64().unwrap_or(10.0).clamp(10.0, 1000.0)
}

fn mousekeys_speed_gs(n: f64) -> Value {
    json!(((n / 10.0).round() * 10.0).clamp(10.0, 1000.0) as i64)
}

/// Versnelling van muistoetsen: Langzaam (3000 ms to the full speed) left, Snel (100 ms) right.
fn mousekeys_accel_ui(v: &Value) -> f64 {
    (3100.0 - v.as_f64().unwrap_or(300.0)).clamp(100.0, 3000.0)
}

fn mousekeys_accel_gs(n: f64) -> Value {
    json!((3100.0 - (n / 100.0).round() * 100.0).clamp(100.0, 3000.0) as i64)
}

/// A key in seconds (GNOME Settings' 0.1 s steps).
fn seconds_ui(v: &Value) -> f64 {
    v.as_f64().unwrap_or(1.2)
}

fn seconds_gs(n: f64) -> Value {
    json!((n * 10.0).round() / 10.0)
}

fn seconds_text(n: f64) -> String {
    format!("{} s", kit::format::number_min((n * 10.0).round() / 10.0, 1, 1))
}

/// A key in whole milliseconds or pixels.
fn whole_ui(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn whole_gs(n: f64) -> Value {
    json!(n.round() as i64)
}

fn ms_text(n: f64) -> String {
    format!("{} ms", n.round() as i64)
}

fn px_text(n: f64) -> String {
    format!("{} px", n.round() as i64)
}

/// The double-click time the Dubbelkliksnelheid step stands for (mouse.rs' steps).
fn double_text(n: f64) -> String {
    format!("{} ms", 1100 - (n.round() as i64).clamp(1, 10) * 100)
}

/// The time to the full speed a Versnelling position stands for.
fn accel_text(n: f64) -> String {
    format!("{} ms", (3100.0 - n).round() as i64)
}

fn speed_text(n: f64) -> String {
    format!("{} px/s", n.round() as i64)
}

// ------------------------------------------------------------------------------------------ shared rows

/// A block of check boxes nested in an expander.
fn checks(boxes: &[gtk::CheckButton]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    b.add_css_class("st-a11y-checks");
    for cb in boxes {
        b.append(cb);
    }
    b
}

/// A nested line of text in an expander, aligned with the nested rows' titles.
fn note(text: &str) -> gtk::Label {
    let l = c::label(text, "w11-text-2", true);
    l.add_css_class("st-expander-note");
    l.add_css_class("st-a11y-note");
    l.set_xalign(0.0);
    l
}

/// A slider's range (min, max, step), its mapping (GNOME's value to the slider and back) and its words.
struct Scale {
    range: (f64, f64, f64),
    to_ui: fn(&Value) -> f64,
    from_ui: fn(f64) -> Value,
    words: (&'static str, &'static str),
    text: fn(f64) -> String,
    widest: &'static str,
}

/// A card with a slider between two words and its value text; None when the key is not there.
fn slider_card(gs: &Gs, k: &GKey, title: &str, scale: Scale) -> Option<(SettingsCard, Slider)> {
    if !gs.available(k) {
        return None;
    }
    let s = gs.slider(k, scale.range.0, scale.range.1, scale.range.2, scale.to_ui, scale.from_ui);
    let value = value_text(gs, &s, k, scale.to_ui, scale.text, scale.widest);
    let card = SettingsCard::new("", title, "");
    card.set_control(&scale_row(&s, scale.words.0, scale.words.1, Some(&value)));
    Some((card, s))
}

/// Amendment 24: Windows' Sneltoets voor Plaktoetsen / Filtertoetsen / Wisseltoetsen is GNOME's one `enable` key;
/// the row (named `st-a11y-keyboard-enable-<which>`) and the note that it covers all three.
fn shortcut_rows(exp: &SettingsExpander, gs: &Gs, which: &str, title: &str, description: &str) {
    if !gs.available(&ENABLE) {
        return;
    }
    let sw = gs.toggle(&ENABLE, false);
    sw.switch().set_widget_name(&format!("st-a11y-keyboard-enable-{which}"));
    let card = SettingsCard::new("", title, description);
    card.set_control(sw.widget());
    exp.add(card.widget());
    exp.add(&note(t(
        "Deze sneltoetsinstelling geldt voor Plaktoetsen, Filtertoetsen en Wisseltoetsen samen.",
        "This shortcut setting applies to Sticky keys, Filter keys and Toggle keys together.",
    )));
}

/// Keep `w` (bound to `own`) sensitive only while `k` is on: a delay or beep of a feature that is off does nothing.
fn while_on(gs: &Gs, k: &GKey, w: &impl IsA<gtk::Widget>, own: &GKey) {
    let sync = {
        let (g, w, own, k) = (gs.clone(), w.as_ref().clone(), own.clone(), k.clone());
        move || w.set_sensitive(g.writable(&own) && g.value(&k) == true)
    };
    sync();
    gs.follow(k, move |_| sync());
}

// ------------------------------------------------------------------------------------------ Audio

fn a11y_audio(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [BELL, BELL_TYPE, LOUD];
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        keep(&p, &gs);
        let mut out: Content = Vec::new();
        let group = SettingsGroup::new(t("Audio", "Audio"), "");
        if gs.available(&BELL) && gs.available(&BELL_TYPE) {
            let options: Vec<DropdownOption> = flash_options().into_iter().map(|(v, l)| DropdownOption::new(v, l)).collect();
            let dd = Dropdown::new(options, Some(flash_of(&gs.value(&BELL), &gs.value(&BELL_TYPE))));
            bind_state(&gs, &keys, dd.widget(), &BELL);
            dd.widget().set_sensitive(gs.writable(&BELL) && gs.writable(&BELL_TYPE));
            let sync: Rc<dyn Fn()> = {
                let (g, dd) = (gs.clone(), dd.clone());
                Rc::new(move || dd.set_selected(Some(flash_of(&g.value(&BELL), &g.value(&BELL_TYPE)))))
            };
            {
                let (g, sync) = (gs.clone(), sync.clone());
                dd.connect_changed(move |choice| {
                    let sync2 = sync.clone();
                    write_together(&g, &[BELL, BELL_TYPE], flash_values(choice), move || sync2());
                });
            }
            for k in [&BELL, &BELL_TYPE] {
                let sync = sync.clone();
                gs.follow(k, move |_| sync());
            }
            // GNOME's Test Flash: the window's bell, which Mutter shows as the visual alert while it is on
            let test = c::button(t("Testen", "Test"), Variant::Standard);
            test.set_widget_name("st-a11y-audio-test");
            test.connect_clicked(|b| {
                if let Some(surface) = b.native().and_then(|n| n.surface()) {
                    surface.beep();
                }
            });
            let card = SettingsCard::new("notification", t("Mijn scherm laten knipperen tijdens audiomeldingen", "Flash my screen during audio notifications"), "");
            card.set_control(&common::sized(dd.widget(), 300));
            card.add_control(&test);
            group.add(card.widget());
        }
        if let Some(card) = gs.toggle_card(
            &LOUD,
            "volume",
            t("Overversterking", "Over-amplification"),
            t("Volume boven 100% toestaan, met verminderde geluidskwaliteit", "Allow volume to exceed 100%, with reduced sound quality"),
            false,
        ) {
            group.add(card.widget());
        }
        if group.items().first_child().is_some() {
            out.push(group.widget().clone().upcast());
        } else {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("volume", t("Audio", "Audio"), None).widget());
            out.push(g.widget().clone().upcast());
        }
        let related = SettingsGroup::new(ct::related(), "");
        related.add(
            common::link_card_with(&p, "sound", None, Some(t("Volume, uitvoer- en invoerapparaten, systeemgeluiden", "Volume, output and input devices, system sounds")), None).widget(),
        );
        out.push(related.widget().clone().upcast());
        out
    })
}

// ------------------------------------------------------------------------------------------ Toetsenbord

fn keyboard_keys() -> Vec<GKey> {
    vec![
        STICKY,
        TWO_KEY,
        MOD_BEEP,
        SLOW,
        SLOW_DELAY,
        SLOW_PRESS,
        SLOW_ACCEPT,
        SLOW_REJECT,
        BOUNCE,
        BOUNCE_DELAY,
        BOUNCE_REJECT,
        TOGGLE,
        ENABLE,
        STATE_BEEP,
        OSK,
        SCREENSHOT,
    ]
}

fn delay_scale(max: f64, widest: &'static str) -> Scale {
    Scale { range: (0.0, max, 10.0), to_ui: whole_ui, from_ui: whole_gs, words: (t("Kort", "Short"), t("Lang", "Long")), text: ms_text, widest }
}

/// Plaktoetsen.
fn sticky_expander(gs: &Gs) -> Option<SettingsExpander> {
    let (exp, _) = switch_expander(gs, &STICKY, "keyboard", t("Plaktoetsen", "Sticky keys"), t("Eén toets per keer indrukken voor sneltoetsen", "Press one key at a time for keyboard shortcuts"))?;
    shortcut_rows(
        &exp,
        gs,
        "sticky",
        t("Sneltoets voor Plaktoetsen", "Keyboard shortcut for Sticky keys"),
        t("Druk vijf keer op Shift om Plaktoetsen in of uit te schakelen", "Press the Shift key five times to turn Sticky keys on or off"),
    );
    let mut boxes = Vec::new();
    if gs.available(&TWO_KEY) {
        boxes.push(gs.checkbox(
            &TWO_KEY,
            t("Plaktoetsen uitschakelen wanneer twee toetsen tegelijk worden ingedrukt", "Turn off Sticky keys when two keys are pressed at the same time"),
            false,
        ));
    }
    if gs.available(&MOD_BEEP) {
        boxes.push(gs.checkbox(&MOD_BEEP, t("Een geluid afspelen wanneer sneltoetsen worden ingedrukt en losgelaten", "Play a sound when shortcut keys are pressed and released"), false));
    }
    if !boxes.is_empty() {
        exp.add(&checks(&boxes));
    }
    // amendment 14: extension/shortcutBridge.js leaves the Windows key alone while Sticky Keys is on
    exp.add(&note(t("Met Plaktoetsen aan opent de Windows-toets Start niet", "With Sticky Keys on, the Windows key doesn’t open Start")));
    Some(exp)
}

/// Filtertoetsen: GNOME's Slow Keys and Bounce Keys under one switch.
fn filter_expander(p: &Page, gs: &Gs) -> Option<SettingsExpander> {
    if !gs.available(&SLOW) || !gs.available(&BOUNCE) {
        return None;
    }
    let exp = SettingsExpander::new(
        "keyboard",
        t("Filtertoetsen", "Filter keys"),
        t("Korte of herhaalde toetsaanslagen negeren en de herhalingsfrequentie van het toetsenbord wijzigen", "Ignore brief or repeated keystrokes and change keyboard repeat rates"),
    );
    exp.header_button().set_widget_name("st-a11y-keyboard-filterkeys-expander");
    let sw = ToggleSwitch::new(filter_on(&gs.value(&SLOW), &gs.value(&BOUNCE)));
    sw.switch().set_widget_name("st-a11y-keyboard-filterkeys");
    if !(gs.writable(&SLOW) && gs.writable(&BOUNCE)) {
        sw.set_sensitive(false);
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    let sync: Rc<dyn Fn()> = {
        let (g, sw) = (gs.clone(), sw.clone());
        Rc::new(move || sw.set_active(filter_on(&g.value(&SLOW), &g.value(&BOUNCE))))
    };
    {
        let (g, sync) = (gs.clone(), sync.clone());
        sw.connect_toggled(move |on| {
            let sync2 = sync.clone();
            write_together(&g, &[SLOW, BOUNCE], filter_values(on), move || sync2());
        });
    }
    for k in [&SLOW, &BOUNCE] {
        let sync = sync.clone();
        gs.follow(k, move |_| sync());
    }
    exp.header().set_control(sw.widget());

    shortcut_rows(
        &exp,
        gs,
        "filter",
        t("Sneltoets voor Filtertoetsen", "Keyboard shortcut for Filter keys"),
        t("Houd Shift acht seconden ingedrukt om Filtertoetsen in of uit te schakelen", "Hold the Shift key for eight seconds to turn Filter keys on or off"),
    );

    // Korte toetsaanslagen negeren: GNOME's Slow Keys
    exp.add(&checks(&[gs.checkbox(&SLOW, t("Korte toetsaanslagen negeren (Trage toetsen)", "Ignore quick keystrokes (Slow keys)"), false)]));
    if let Some((card, _)) = slider_card(gs, &SLOW_DELAY, t("Acceptatievertraging", "Acceptance delay"), delay_scale(500.0, "500 ms")) {
        while_on(gs, &SLOW, card.widget(), &SLOW_DELAY);
        exp.add(card.widget());
    }
    let mut beeps = Vec::new();
    for (k, nl, en) in [
        (&SLOW_PRESS, "Een pieptoon laten horen wanneer een toets wordt ingedrukt", "Beep when a key is pressed"),
        (&SLOW_ACCEPT, "Een pieptoon laten horen wanneer een toets wordt geaccepteerd", "Beep when a key is accepted"),
        (&SLOW_REJECT, "Een pieptoon laten horen wanneer een toets wordt geweigerd", "Beep when a key is rejected"),
    ] {
        if gs.available(k) {
            let cb = gs.checkbox(k, t(nl, en), false);
            while_on(gs, &SLOW, &cb, k);
            beeps.push(cb);
        }
    }
    if !beeps.is_empty() {
        exp.add(&checks(&beeps));
    }

    // Onbedoelde toetsaanslagen negeren: GNOME's Bounce Keys
    exp.add(&checks(&[gs.checkbox(&BOUNCE, t("Onbedoelde toetsaanslagen negeren (Stuitertoetsen)", "Ignore unintended keystrokes (Bounce keys)"), false)]));
    if let Some((card, _)) = slider_card(gs, &BOUNCE_DELAY, t("Acceptatievertraging", "Acceptance delay"), delay_scale(900.0, "900 ms")) {
        while_on(gs, &BOUNCE, card.widget(), &BOUNCE_DELAY);
        exp.add(card.widget());
    }
    if gs.available(&BOUNCE_REJECT) {
        let cb = gs.checkbox(&BOUNCE_REJECT, t("Een pieptoon laten horen wanneer een toets wordt geweigerd", "Beep when a key is rejected"), false);
        while_on(gs, &BOUNCE, &cb, &BOUNCE_REJECT);
        exp.add(&checks(&[cb]));
    }

    // GNOME's Repeat Keys are Geavanceerde toetsenbordinstellingen' Tekenherhaling
    let repeat = SettingsCard::clickable("", t("Tekenherhaling", "Character repeat"), t("Vertraging en snelheid van herhalen", "Repeat delay and rate"), Chevron::Right);
    if let Some(b) = repeat.button() {
        b.set_widget_name("st-a11y-keyboard-repeat");
    }
    let pp = p.clone();
    repeat.connect_clicked(move || pp.go("keyboard-advanced"));
    exp.add(repeat.widget());
    Some(exp)
}

/// Wisseltoetsen (GNOME's Sound Keys).
fn toggle_expander(gs: &Gs) -> Option<SettingsExpander> {
    let (exp, _) = switch_expander(
        gs,
        &TOGGLE,
        "keyboard",
        t("Wisseltoetsen", "Toggle keys"),
        t("Een geluid afspelen wanneer je op Caps Lock, Num Lock of Scroll Lock drukt", "Play a sound whenever you press Caps lock, Num lock or Scroll lock"),
    )?;
    shortcut_rows(
        &exp,
        gs,
        "toggle",
        t("Sneltoets voor Wisseltoetsen", "Keyboard shortcut for Toggle keys"),
        t("Dezelfde instelling als bij Plaktoetsen en Filtertoetsen", "The same setting as for Sticky keys and Filter keys"),
    );
    Some(exp)
}

/// De Print Screen-toets gebruiken om schermopname te openen: Print in show-screenshot-ui, through
/// input.shortcuts.set (its row is the shortcut service's; a conflict is reported and nothing is written).
fn print_card(p: &Page, gs: &Gs) -> Option<SettingsCard> {
    if !gs.available(&SCREENSHOT) {
        return None;
    }
    let sw = ToggleSwitch::new(print_on(&gs.value(&SCREENSHOT)));
    sw.switch().set_widget_name("st-a11y-keyboard-show-screenshot-ui");
    if gs.item(&SCREENSHOT)["writable"] != true {
        sw.set_sensitive(false);
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (pp, g, sw2) = (p.clone(), gs.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let accels = print_accels(&g.value(&SCREENSHOT), on);
            let wait = pp.run_with("input.shortcuts.set", json!({ "schema": SCREENSHOT.schema, "key": SCREENSHOT.key, "accels": accels }), None, false);
            let (p2, g2, sw3) = (pp.clone(), g.clone(), sw2.clone());
            pp.spawn(async move {
                match wait.await {
                    Some(answer) if answer["conflict"].is_object() => {
                        let en = kit::ctx().lang() == "en";
                        let other = &answer["conflict"];
                        let name = super::shortcuts::name_of(common::s(&other["key"]), common::s(&other["name"]), en);
                        common::show_error_text(
                            &p2.anchor(),
                            &common::fill(t("Print Screen wordt al gebruikt door {name}.", "Print screen is already used by {name}."), &[("name", &name)]),
                            Some(t("Sneltoets in gebruik", "Shortcut in use")),
                        );
                        sw3.set_active(print_on(&g2.value(&SCREENSHOT)));
                    }
                    Some(_) => {}
                    None => sw3.set_active(print_on(&g2.value(&SCREENSHOT))),
                }
            });
        });
    }
    let sw2 = sw.clone();
    gs.follow(&SCREENSHOT, move |v| sw2.set_active(print_on(v)));
    let card = SettingsCard::new("image", t("De Print Screen-toets gebruiken om schermopname te openen", "Use the Print screen key to open screen capture"), "");
    card.set_control(sw.widget());
    Some(card)
}

fn a11y_keyboard(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = keyboard_keys();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        keep(&p, &gs);
        let mut out: Content = Vec::new();

        let keys_group = SettingsGroup::new(t("Plaktoetsen, Filtertoetsen en Wisseltoetsen", "Sticky, Filter and Toggle keys"), "");
        for exp in [sticky_expander(&gs), filter_expander(&p, &gs), toggle_expander(&gs)].into_iter().flatten() {
            keys_group.add(exp.widget());
        }
        if keys_group.items().first_child().is_some() {
            out.push(keys_group.widget().clone().upcast());
        }

        let more = SettingsGroup::new(t("Schermtoetsenbord en Print Screen", "On-screen keyboard and Print screen"), "");
        if let Some(card) = gs.toggle_card(
            &OSK,
            "keyboard",
            t("Schermtoetsenbord", "On-screen keyboard"),
            t("Het schermtoetsenbord gebruiken om in invoervelden te typen", "Use the on-screen keyboard to type in input fields"),
            false,
        ) {
            more.add(card.widget());
        }
        if let Some(card) = print_card(&p, &gs) {
            more.add(card.widget());
        }
        if more.items().first_child().is_some() {
            out.push(more.widget().clone().upcast());
        }

        let notify = SettingsGroup::new(t("Meldingsvoorkeuren", "Notification preferences"), "");
        if let Some(card) = gs.toggle_card(
            &STATE_BEEP,
            "bell",
            t(
                "Een geluid afspelen wanneer ik Plaktoetsen, Filtertoetsen of Wisseltoetsen via het toetsenbord in- of uitschakel",
                "Play a sound when I turn Sticky, Filter or Toggle keys on or off from the keyboard",
            ),
            "",
            false,
        ) {
            notify.add(card.widget());
        }
        if notify.items().first_child().is_some() {
            out.push(notify.widget().clone().upcast());
        }

        if out.is_empty() {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("keyboard", t("Toetsenbord", "Keyboard"), None).widget());
            out.push(g.widget().clone().upcast());
        }
        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "typing").widget());
        related.add(common::link_card(&p, "shortcuts").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

// ------------------------------------------------------------------------------------------ Muis

fn mouse_keys() -> Vec<GKey> {
    vec![MOUSEKEYS, MOUSEKEYS_SPEED, MOUSEKEYS_ACCEL, FOCUS_MODE, mouse::DOUBLE, SECONDARY, SECONDARY_TIME, DWELL, DWELL_TIME, DWELL_THRESHOLD]
}

fn seconds_scale(min: f64) -> Scale {
    Scale { range: (min, 3.0, 0.1), to_ui: seconds_ui, from_ui: seconds_gs, words: (t("Kort", "Short"), t("Lang", "Long")), text: seconds_text, widest: "3,0 s" }
}

fn a11y_mouse(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = mouse_keys();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        keep(&p, &gs);
        let mut out: Content = Vec::new();
        let slow_fast = (t("Langzaam", "Slow"), t("Snel", "Fast"));

        let main = SettingsGroup::new("", "");
        if let Some((exp, _)) = switch_expander(
            &gs,
            &MOUSEKEYS,
            "mouse",
            t("Muistoetsen", "Mouse keys"),
            t("Het numerieke toetsenblok gebruiken om de muisaanwijzer te verplaatsen", "Use the numeric keypad to move the mouse pointer"),
        ) {
            let speed = Scale { range: (10.0, 1000.0, 10.0), to_ui: mousekeys_speed_ui, from_ui: mousekeys_speed_gs, words: slow_fast, text: speed_text, widest: "1000 px/s" };
            if let Some((card, _)) = slider_card(&gs, &MOUSEKEYS_SPEED, t("Snelheid van muistoetsen", "Mouse keys speed"), speed) {
                exp.add(card.widget());
            }
            let accel = Scale { range: (100.0, 3000.0, 100.0), to_ui: mousekeys_accel_ui, from_ui: mousekeys_accel_gs, words: slow_fast, text: accel_text, widest: "3000 ms" };
            if let Some((card, _)) = slider_card(&gs, &MOUSEKEYS_ACCEL, t("Versnelling van muistoetsen", "Mouse keys acceleration"), accel) {
                exp.add(card.widget());
            }
            main.add(exp.widget());
        }
        if gs.available(&FOCUS_MODE) {
            let sw = mapped_toggle(&gs, &keys, &FOCUS_MODE, hover_on, hover_value);
            let card = SettingsCard::new(
                "window",
                t("Een venster activeren door de muisaanwijzer erop te plaatsen", "Activate a window by hovering over it with the mouse"),
                t("Het venster onder de muisaanwijzer wordt actief zonder te klikken", "The window under the pointer becomes active without a click"),
            );
            card.set_control(sw.widget());
            main.add(card.widget());
        }
        let double = Scale { range: (1.0, 10.0, 1.0), to_ui: mouse::double_ui, from_ui: mouse::double_gs, words: slow_fast, text: double_text, widest: "1000 ms" };
        if let Some((card, _)) = slider_card(&gs, &mouse::DOUBLE, t("Dubbelkliksnelheid", "Double-click speed"), double) {
            card.set_icon("mouse");
            card.set_description(t("De tijd tussen de twee klikken van een dubbelklik", "The time between the two clicks of a double-click"));
            main.add(card.widget());
        }
        if main.items().first_child().is_some() {
            out.push(main.widget().clone().upcast());
        }

        // GNOME's Click Assist (WinPoint additions: Windows has dwell only in Eye control)
        let assist = SettingsGroup::new(t("Klikhulp", "Click assist"), "");
        if let Some((exp, _)) = switch_expander(
            &gs,
            &SECONDARY,
            "mouse",
            t("Gesimuleerde secundaire klik", "Simulated secondary click"),
            t("Een secundaire klik geven door de primaire knop ingedrukt te houden", "Secondary click by holding down the primary button"),
        ) {
            if let Some((card, _)) = slider_card(&gs, &SECONDARY_TIME, t("Acceptatievertraging", "Acceptance delay"), seconds_scale(0.5)) {
                exp.add(card.widget());
            }
            assist.add(exp.widget());
        }
        if let Some((exp, _)) = switch_expander(
            &gs,
            &DWELL,
            "mouse",
            t("Klikken door te zweven", "Hover click"),
            t("Een klik geven wanneer de muisaanwijzer stilstaat", "Click when the pointer stops moving"),
        ) {
            if let Some((card, _)) = slider_card(&gs, &DWELL_TIME, t("Vertraging", "Delay"), seconds_scale(0.2)) {
                exp.add(card.widget());
            }
            let threshold = Scale { range: (0.0, 30.0, 1.0), to_ui: whole_ui, from_ui: whole_gs, words: (t("Klein", "Small"), t("Groot", "Large")), text: px_text, widest: "30 px" };
            if let Some((card, _)) = slider_card(&gs, &DWELL_THRESHOLD, t("Bewegingsdrempel", "Motion threshold"), threshold) {
                exp.add(card.widget());
            }
            assist.add(exp.widget());
        }
        if assist.items().first_child().is_some() {
            out.push(assist.widget().clone().upcast());
        }

        if out.is_empty() {
            let g = SettingsGroup::new("", "");
            g.add(common::unavailable_card("mouse", t("Muis", "Mouse"), None).widget());
            out.push(g.widget().clone().upcast());
        }
        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "mousepointer").widget());
        related.add(
            common::link_card_with(
                &p,
                "mouse",
                Some(t("Muisinstellingen", "Mouse settings")),
                Some(t("Primaire knop, snelheid van muisaanwijzer, schuiven", "Primary button, mouse pointer speed, scrolling")),
                None,
            )
            .widget(),
        );
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_screen_flash_is_gnomes_visual_bell_with_its_type() {
        assert_eq!(flash_of(&json!(false), &json!("frame-flash")), "off");
        assert_eq!(flash_of(&json!(true), &json!("frame-flash")), "window");
        assert_eq!(flash_of(&json!(true), &json!("fullscreen-flash")), "screen");
        assert_eq!(flash_values("off"), json!({"visual-bell": false}), "Uit keeps the flash area");
        for (choice, _) in flash_options() {
            let v = flash_values(choice);
            let kind = if v["visual-bell-type"].is_null() { json!("fullscreen-flash") } else { v["visual-bell-type"].clone() };
            assert_eq!(flash_of(&v["visual-bell"], &kind), choice, "{choice} round trip");
        }
        assert_eq!(gs::control_name("a11y-audio", &[BELL, BELL_TYPE, LOUD], &LOUD), "st-a11y-audio-allow-volume-above-100-percent");
        assert_eq!(gs::control_name("a11y-audio", &[BELL, BELL_TYPE, LOUD], &BELL), "st-a11y-audio-visual-bell");
    }

    #[test]
    fn filter_keys_are_slow_keys_or_bounce_keys() {
        assert!(!filter_on(&json!(false), &json!(false)));
        assert!(filter_on(&json!(true), &json!(false)));
        assert!(filter_on(&json!(false), &json!(true)));
        assert_eq!(filter_values(true), json!({"slowkeys-enable": true}));
        assert_eq!(filter_values(false), json!({"slowkeys-enable": false, "bouncekeys-enable": false}));
    }

    #[test]
    fn print_screen_is_print_in_show_screenshot_ui() {
        assert!(print_on(&json!(["Print"])));
        assert!(!print_on(&json!([])));
        assert!(!print_on(&json!(["<Shift>Print"])));
        assert_eq!(print_accels(&json!([]), true), ["Print"]);
        assert_eq!(print_accels(&json!(["Print", "<Super>s"]), false), ["<Super>s"], "its other keys stay");
        assert_eq!(print_accels(&json!(["<Super>s"]), true), ["Print", "<Super>s"]);
        assert_eq!(print_accels(&json!(["Print"]), true), ["Print"], "no second Print");
    }

    #[test]
    fn the_mouse_rows_map_onto_gnomes_keys() {
        assert!(hover_on(&json!("sloppy")));
        assert!(hover_on(&json!("mouse")), "GNOME's third mode reads as on");
        assert!(!hover_on(&json!("click")));
        assert_eq!(hover_value(true), json!("sloppy"));
        assert_eq!(hover_value(false), json!("click"));
        assert_eq!(mousekeys_speed_ui(&json!(10)), 10.0, "GNOME's default at the slow end");
        assert_eq!(mousekeys_speed_gs(254.0), json!(250));
        assert_eq!(mousekeys_accel_ui(&json!(300)), 2800.0);
        assert_eq!(mousekeys_accel_gs(2800.0), json!(300));
        assert_eq!(mousekeys_accel_gs(100.0), json!(3000), "Langzaam: 3 s to the full speed");
        for ms in [100, 300, 1500, 3000] {
            assert_eq!(mousekeys_accel_gs(mousekeys_accel_ui(&json!(ms))), json!(ms));
        }
        assert_eq!(accel_text(2800.0), "300 ms");
        assert_eq!(double_text(7.0), "400 ms", "GNOME's default double-click time");
        assert_eq!(seconds_gs(1.2345), json!(1.2));
        assert_eq!(whole_gs(9.6), json!(10));
        let keys = mouse_keys();
        assert_eq!(gs::control_name("a11y-mouse", &keys, &mouse::DOUBLE), "st-a11y-mouse-double-click");
        let names: std::collections::HashSet<String> = keys.iter().map(|k| gs::control_name("a11y-mouse", &keys, k)).collect();
        assert_eq!(names.len(), keys.len());
        let keys = keyboard_keys();
        let names: std::collections::HashSet<String> = keys.iter().map(|k| gs::control_name("a11y-keyboard", &keys, k)).collect();
        assert_eq!(names.len(), keys.len());
    }
}
