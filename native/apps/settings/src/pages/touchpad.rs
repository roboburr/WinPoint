//! Instellingen › Bluetooth en apparaten › Touchpad (batch A, notes/claude/settings-parity.md §2, plan.json
//! "touchpad"): every touchpad function of GNOME Settings 50's Mouse & Touchpad panel, in the order and wording of
//! Windows 11's Bluetooth & devices › Touchpad. The page needs `caps.touchpad` (registry.rs).
//!
//! * Touchpad (the expander's switch) = `org.gnome.desktop.peripherals.touchpad send-events`: off writes 'disabled';
//!   Touchpad ingeschakeld laten wanneer er een muis is aangesloten unticked is 'disabled-on-external-mouse' (in
//!   GNOME's schema, not in its panel), ticked 'enabled'.
//! * Cursorsnelheid, Windows' 1-10 with 5 in the middle = `speed` -1..1, (n - 5) / 5.
//! * Opnieuw instellen: `gsettings.reset` of every key of the touchpad's allow row (g_settings_reset, so Ubuntu's
//!   overrides come back as GNOME Settings would see them).
//! * Tikken: Touchpad uitschakelen tijdens het typen = `disable-while-typing` (amendments 10 and 67: no sensitivity
//!   dropdown, mutter 50 ignores `disable-while-typing-timeout`); Met één vinger tikken = `tap-to-click`; Met twee
//!   vingers tikken (libinput's tapping always has it: shown ticked and fixed while tapping is on); Twee keer tikken
//!   en slepen = `tap-and-drag`; the lower right corner = `click-method` 'areas', unticked 'fingers' (GNOME's
//!   Secondary Click: Corner Push / Two Finger Push).
//! * Schuiven en zoomen: GNOME's Scroll Method as Windows' row and the edge alternative,
//!   `two-finger-scrolling-enabled` and `edge-scrolling-enabled` written together in one `gsettings.set`;
//!   Schuifrichting = `natural-scroll`; Knijpen om te zoomen (apps handle it: fixed on).
//! * Bewegingen met drie / vier vingers: GNOME Shell's fixed gestures, said as a fact; the three-finger tap follows
//!   `tap-button-map` (read only here).
//! * Instellingen testen and Gerelateerde instellingen: Muis (its Primaire muisknop is the touchpad's too).
use super::mouse::{self, DOUBLE};
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::Cell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("touchpad", touchpad)];
/// The notes in the expanders line up with their rows (the kit's 58 px nested indent), as slice 4 did for the
/// accessibility pages; settings.css gives a plain `.st-expander-note` 16 px.
pub const CSS: &str = ".st-touchpad-note.st-expander-note.w11-nested-other { padding-left: 58px; }";

const TOUCHPAD: &str = "org.gnome.desktop.peripherals.touchpad";
const SEND: GKey = gs::key(TOUCHPAD, "send-events");
const SPEED: GKey = gs::key(TOUCHPAD, "speed");
const TYPING: GKey = gs::key(TOUCHPAD, "disable-while-typing");
const TAP: GKey = gs::key(TOUCHPAD, "tap-to-click");
const TAP_DRAG: GKey = gs::key(TOUCHPAD, "tap-and-drag");
const TAP_MAP: GKey = gs::key(TOUCHPAD, "tap-button-map");
const CLICK: GKey = gs::key(TOUCHPAD, "click-method");
const TWO: GKey = gs::key(TOUCHPAD, "two-finger-scrolling-enabled");
const EDGE: GKey = gs::key(TOUCHPAD, "edge-scrolling-enabled");
const NATURAL: GKey = gs::key(TOUCHPAD, "natural-scroll");

/// Every key of the touchpad's allow row (host/src/services/gsettings/allow/input.rs): what Opnieuw instellen resets.
const RESET_KEYS: &[&str] = &[
    "send-events",
    "speed",
    "accel-profile",
    "disable-while-typing",
    "tap-to-click",
    "tap-button-map",
    "tap-and-drag",
    "tap-and-drag-lock",
    "click-method",
    "two-finger-scrolling-enabled",
    "edge-scrolling-enabled",
    "natural-scroll",
    "left-handed",
    "middle-click-emulation",
];

fn keys() -> Vec<GKey> {
    // the mouse's double-click time for the test dialog (not bound on this page)
    vec![SEND, SPEED, TYPING, TAP, TAP_DRAG, TAP_MAP, CLICK, TWO, EDGE, NATURAL, DOUBLE]
}

// ------------------------------------------------------------------------------------------ value mappings

/// Windows' cursor speed, 1-10 with GNOME's 0 at 5, from GNOME's speed (-1..1).
fn speed10_ui(v: &Value) -> f64 {
    (v.as_f64().unwrap_or(0.0) * 5.0 + 5.0).round().clamp(1.0, 10.0)
}

/// GNOME's speed for Windows' 1-10: (n - 5) / 5.
fn speed10_gs(n: f64) -> Value {
    json!(((n.round() - 5.0) / 5.0).clamp(-1.0, 1.0))
}

/// Whether the touchpad is on (send-events other than 'disabled').
fn touchpad_on(v: &Value) -> bool {
    v.as_str() != Some("disabled")
}

/// Whether "Touchpad ingeschakeld laten wanneer er een muis is aangesloten" is ticked for send-events `v`; while the
/// touchpad is off the last choice stays (`before`).
fn keep_with_mouse(v: &Value, before: bool) -> bool {
    match v.as_str() {
        Some("disabled-on-external-mouse") => false,
        Some("disabled") => before,
        _ => true,
    }
}

/// send-events for the switch and the check box.
fn send_events(on: bool, keep: bool) -> &'static str {
    match (on, keep) {
        (false, _) => "disabled",
        (true, true) => "enabled",
        (true, false) => "disabled-on-external-mouse",
    }
}

/// The scroll method of the two scrolling keys: "two", "edge" or "" (neither).
fn scroll_method(two: &Value, edge: &Value) -> &'static str {
    if *two == true {
        "two"
    } else if *edge == true {
        "edge"
    } else {
        ""
    }
}

/// The values of a scroll method, written together.
fn scroll_values(method: &str) -> Value {
    json!({ "two-finger-scrolling-enabled": method == "two", "edge-scrolling-enabled": method == "edge" })
}

/// The corner check box: 'areas' (Corner Push) is ticked; unticked writes 'fingers' (Two Finger Push).
fn corner_on(v: &Value) -> bool {
    v.as_str() == Some("areas")
}

fn corner_value(on: bool) -> Value {
    json!(if on { "areas" } else { "fingers" })
}

/// What a three-finger tap does for tap-button-map (libinput: 'lmr' swaps the two- and three-finger taps).
fn three_finger_tap(map: &Value) -> &'static str {
    if map.as_str() == Some("lmr") {
        t("Met drie vingers tikken: rechtermuisknop", "Tap with three fingers: right mouse button")
    } else {
        t("Met drie vingers tikken: middelste muisknop", "Tap with three fingers: middle mouse button")
    }
}

/// A nested block of check boxes in an expander.
fn checks(boxes: &[&gtk::CheckButton]) -> gtk::Box {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 8);
    for cb in boxes {
        b.append(*cb);
    }
    b
}

/// A nested line of text in an expander.
fn note(text: &str) -> gtk::Label {
    let l = c::label(text, "w11-text-2", true);
    l.add_css_class("st-expander-note");
    l.add_css_class("st-touchpad-note");
    l.set_xalign(0.0);
    l
}

/// The Touchpad expander: the switch, the mouse check box, Cursorsnelheid and Opnieuw instellen.
fn touchpad_expander(p: &Page, gs: &Gs, keys: &[GKey]) -> Option<SettingsExpander> {
    if !gs.available(&SEND) {
        return None;
    }
    let exp = SettingsExpander::new("touchpad", t("Touchpad", "Touchpad"), "");
    exp.set_expanded(true);
    exp.header_button().set_widget_name("st-touchpad-expander");
    let keep = Rc::new(Cell::new(keep_with_mouse(&gs.value(&SEND), true)));
    let sw = ToggleSwitch::new(touchpad_on(&gs.value(&SEND)));
    mouse::bind_state(gs, keys, sw.switch(), &SEND);
    if !gs.writable(&SEND) {
        sw.set_sensitive(false);
    }
    let cb = c::checkbox(t("Touchpad ingeschakeld laten wanneer er een muis is aangesloten", "Leave touchpad on when a mouse is connected"));
    cb.set_widget_name("st-touchpad-send-events-mouse");
    cb.set_active(keep.get());
    cb.set_sensitive(gs.writable(&SEND) && sw.active());
    let silent = Rc::new(Cell::new(false));
    // one place shows a value of send-events
    let show: Rc<dyn Fn(&Value)> = {
        let (sw, cb, keep, silent, gs) = (sw.clone(), cb.clone(), keep.clone(), silent.clone(), gs.clone());
        Rc::new(move |v: &Value| {
            keep.set(keep_with_mouse(v, keep.get()));
            sw.set_active(touchpad_on(v));
            silent.set(true);
            cb.set_active(keep.get());
            silent.set(false);
            cb.set_sensitive(gs.writable(&SEND) && touchpad_on(v));
        })
    };
    let write: Rc<dyn Fn(&'static str)> = {
        let (gs, show) = (gs.clone(), show.clone());
        Rc::new(move |value: &'static str| {
            show(&json!(value));
            let (gs2, show2) = (gs.clone(), show.clone());
            gs.set(&SEND, json!(value), move || show2(&gs2.value(&SEND)));
        })
    };
    {
        let (keep, write) = (keep.clone(), write.clone());
        sw.connect_toggled(move |on| write(send_events(on, keep.get())));
    }
    {
        let (silent, write) = (silent.clone(), write.clone());
        cb.connect_toggled(move |b| {
            if !silent.get() {
                write(send_events(true, b.is_active()));
            }
        });
    }
    {
        let show = show.clone();
        gs.follow(&SEND, move |v| show(v));
    }
    exp.header().set_control(sw.widget());
    exp.add(&checks(&[&cb]));

    if gs.available(&SPEED) {
        let slider = gs.slider(&SPEED, 1.0, 10.0, 1.0, speed10_ui, speed10_gs);
        slider.widget().set_size_request(240, -1);
        let card = SettingsCard::new("", t("Cursorsnelheid", "Cursor speed"), "");
        card.set_control(slider.widget());
        exp.add(card.widget());
    }

    let reset_card = SettingsCard::new(
        "",
        t(
            // Windows' own row ("Reset touchpad settings and gestures to default"), not Restore
            // glossary-ok: Terugzetten
            "Instellingen en bewegingen van touchpad terugzetten op de standaardwaarden",
            "Reset touchpad settings and gestures to default",
        ),
        "",
    );
    let done = kit::icons::glyph("check", 16);
    done.set_visible(false);
    let reset = c::button(t("Opnieuw instellen", "Reset"), Variant::Standard);
    reset.set_widget_name("st-touchpad-reset");
    {
        let (pp, done) = (p.clone(), done.clone());
        reset.connect_clicked(move |b| {
            b.set_sensitive(false);
            let wait = pp.run("gsettings.reset", json!({ "schema": TOUCHPAD, "keys": RESET_KEYS }));
            let (b, done, p2) = (b.clone(), done.clone(), pp.clone());
            pp.spawn(async move {
                let ok = wait.await.is_some();
                b.set_sensitive(true);
                if ok && p2.alive() {
                    done.set_visible(true);
                    let done = done.clone();
                    p2.timeout(3000, move || done.set_visible(false));
                }
            });
        });
    }
    reset_card.set_control(&done);
    reset_card.add_control(&reset);
    exp.add(reset_card.widget());
    Some(exp)
}

/// Tikken: disable-while-typing, the taps, the corner.
fn taps_expander(gs: &Gs, keys: &[GKey]) -> SettingsExpander {
    let exp = SettingsExpander::new("touchpad", t("Tikken", "Taps"), "");
    exp.header_button().set_widget_name("st-touchpad-taps");
    let mut boxes: Vec<gtk::CheckButton> = Vec::new();
    if gs.available(&TYPING) {
        boxes.push(gs.checkbox(&TYPING, t("Touchpad uitschakelen tijdens het typen", "Turn off touchpad while typing"), false));
    }
    if gs.available(&TAP) {
        boxes.push(gs.checkbox(&TAP, t("Met één vinger tikken om één keer te klikken", "Tap with a single finger to single-click"), false));
        // libinput's tapping always has the two-finger tap: ticked and fixed, following tap-to-click
        let two = c::checkbox(t("Met twee vingers tikken om met de rechtermuisknop te klikken", "Tap with two fingers to right-click"));
        two.set_widget_name("st-touchpad-tap-two-fingers");
        two.set_active(gs.value(&TAP) == true);
        two.set_sensitive(false);
        two.set_tooltip_text(Some(t("Dit hoort bij tikken en staat aan zolang tikken aan staat.", "This is part of tapping and is on while tapping is on.")));
        let two2 = two.clone();
        gs.follow(&TAP, move |v| two2.set_active(*v == true));
        boxes.push(two);
    }
    if gs.available(&TAP_DRAG) {
        let drag = gs.checkbox(&TAP_DRAG, t("Twee keer tikken en slepen om meerdere items te selecteren", "Tap twice and drag to multi-select"), false);
        // dragging by a tap needs tapping
        let tap_on = gs.value(&TAP) != false;
        drag.set_sensitive(gs.writable(&TAP_DRAG) && tap_on);
        let (drag2, gs2) = (drag.clone(), gs.clone());
        gs.follow(&TAP, move |v| drag2.set_sensitive(gs2.writable(&TAP_DRAG) && *v != false));
        boxes.push(drag);
    }
    if gs.available(&CLICK) {
        boxes.push(mouse::mapped_checkbox(
            gs,
            keys,
            &CLICK,
            t(
                "Op de rechterbenedenhoek van het touchpad drukken om met de rechtermuisknop te klikken",
                "Press the lower right corner of the touchpad to right-click",
            ),
            corner_on,
            corner_value,
        ));
    }
    let refs: Vec<&gtk::CheckButton> = boxes.iter().collect();
    exp.add(&checks(&refs));
    exp
}

/// Schuiven en zoomen: the scroll method (two keys in one write), the direction, pinch.
fn scroll_expander(p: &Page, gs: &Gs, keys: &[GKey]) -> SettingsExpander {
    let exp = SettingsExpander::new("sort", t("Schuiven en zoomen", "Scroll & zoom"), "");
    exp.header_button().set_widget_name("st-touchpad-scroll");
    if gs.available(&TWO) && gs.available(&EDGE) {
        let current = scroll_method(&gs.value(&TWO), &gs.value(&EDGE));
        let rg = RadioGroup::new(
            &[
                ("two", t("Met twee vingers slepen om te schuiven", "Drag two fingers to scroll")),
                ("edge", t("Schuiven langs de rand", "Scroll along the edge")),
            ],
            Some(current),
            None,
            false,
        );
        rg.widget().set_widget_name(&gs::control_name(p.id(), keys, &TWO));
        let writable = gs.writable(&TWO) && gs.writable(&EDGE);
        rg.widget().set_sensitive(writable);
        if !writable {
            rg.widget().set_tooltip_text(Some(gs::locked_text()));
        }
        if let Some(b) = rg.button("edge") {
            b.set_widget_name(&gs::control_name(p.id(), keys, &EDGE));
        }
        {
            let (gs2, rg2) = (gs.clone(), rg.clone());
            rg.connect_changed(move |method| {
                let values = scroll_values(method);
                let (gs3, rg3) = (gs2.clone(), rg2.clone());
                gs2.set_values(&[(&TWO, values[TWO.key].clone()), (&EDGE, values[EDGE.key].clone())], move || {
                    rg3.set_value(scroll_method(&gs3.value(&TWO), &gs3.value(&EDGE)));
                });
            });
        }
        for k in [&TWO, &EDGE] {
            let (gs2, rg2) = (gs.clone(), rg.clone());
            gs.follow(k, move |_| rg2.set_value(scroll_method(&gs2.value(&TWO), &gs2.value(&EDGE))));
        }
        // a nested row like the boxes of Tikken (the kit's 58 px indent; the harness's screenshot had it at 16 px)
        exp.add(rg.widget());
    }
    if gs.available(&NATURAL) {
        let dd = gs.dropdown(&NATURAL, mouse::scroll_options());
        let card = SettingsCard::new("", t("Schuifrichting", "Scrolling direction"), "");
        card.set_control(&common::sized(dd.widget(), 340));
        exp.add(card.widget());
    }
    let pinch = c::checkbox(t("Knijpen om te zoomen", "Pinch to zoom"));
    pinch.set_widget_name("st-touchpad-pinch");
    pinch.set_active(true);
    pinch.set_sensitive(false);
    pinch.set_tooltip_text(Some(t("Apps die zoomen ondersteunen, doen dit altijd.", "Apps that support zooming always do this.")));
    exp.add(&checks(&[&pinch]));
    exp
}

/// Bewegingen met drie / vier vingers: GNOME Shell's own gestures.
fn gestures_expander(gs: &Gs, title: &str, name: &str, three: bool) -> SettingsExpander {
    let exp = SettingsExpander::new("touchpad", title, "");
    exp.header_button().set_widget_name(name);
    exp.add(&note(t(
        "GNOME Shell regelt vegen met meerdere vingers zelf; dit kan hier niet worden gewijzigd.",
        "GNOME Shell handles multi-finger swipes itself; this can’t be changed here.",
    )));
    if three {
        let tap = note(three_finger_tap(&gs.value(&TAP_MAP)));
        tap.set_widget_name("st-touchpad-three-finger-tap");
        let tap2 = tap.clone();
        gs.follow(&TAP_MAP, move |v| tap2.set_label(three_finger_tap(v)));
        exp.add(&tap);
    }
    exp
}

fn touchpad(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = keys();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        if common::caps()["touchpad"] == false {
            out.push(common::notice(t("Er is geen touchpad gevonden. Deze instellingen gelden zodra er een is aangesloten.", "No touchpad was found. These settings apply once one is connected.")).widget().clone().upcast());
        }
        if let Some(exp) = touchpad_expander(&p, &gs, &keys) {
            let g = SettingsGroup::new("", "");
            g.add(exp.widget());
            out.push(g.widget().clone().upcast());
        }
        let gestures = SettingsGroup::new(t("Bewegingen en interactie", "Gestures & interaction"), "");
        gestures.add(taps_expander(&gs, &keys).widget());
        gestures.add(scroll_expander(&p, &gs, &keys).widget());
        gestures.add(gestures_expander(&gs, t("Bewegingen met drie vingers", "Three-finger gestures"), "st-touchpad-three-finger", true).widget());
        gestures.add(gestures_expander(&gs, t("Bewegingen met vier vingers", "Four-finger gestures"), "st-touchpad-four-finger", false).widget());
        out.push(gestures.widget().clone().upcast());

        let test = SettingsGroup::new("", "");
        let test_card = SettingsCard::new("check", t("Instellingen testen", "Test settings"), t("Klikken, dubbelklikken en schuiven uitproberen", "Try clicking, double-clicking and scrolling"));
        let test_button = c::button(t("Testen", "Test"), Variant::Standard);
        test_button.set_widget_name("st-touchpad-test");
        {
            let gs = gs.clone();
            test_button.connect_clicked(move |_| mouse::test_dialog(&gs));
        }
        test_card.set_control(&test_button);
        test.add(test_card.widget());
        out.push(test.widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "mouse").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_cursor_speed_maps_onto_gnomes_speed() {
        assert_eq!(speed10_gs(5.0), json!(0.0));
        assert_eq!(speed10_gs(10.0), json!(1.0));
        assert_eq!(speed10_gs(1.0), json!(-0.8));
        assert_eq!(speed10_ui(&json!(-1.0)), 1.0);
        assert_eq!(speed10_ui(&json!(0.4)), 7.0);
        for n in 1..=10 {
            assert_eq!(speed10_ui(&speed10_gs(n as f64)), n as f64);
        }
    }

    #[test]
    fn the_switch_and_the_mouse_box_share_send_events() {
        assert!(touchpad_on(&json!("enabled")) && touchpad_on(&json!("disabled-on-external-mouse")));
        assert!(!touchpad_on(&json!("disabled")));
        assert_eq!(send_events(false, true), "disabled");
        assert_eq!(send_events(true, true), "enabled");
        assert_eq!(send_events(true, false), "disabled-on-external-mouse");
        assert!(keep_with_mouse(&json!("enabled"), false));
        assert!(!keep_with_mouse(&json!("disabled-on-external-mouse"), true));
        assert!(!keep_with_mouse(&json!("disabled"), false), "off: the last choice stays");
        assert!(keep_with_mouse(&json!("disabled"), true));
    }

    #[test]
    fn the_scroll_method_writes_both_keys_together() {
        assert_eq!(scroll_values("two"), json!({"two-finger-scrolling-enabled": true, "edge-scrolling-enabled": false}));
        assert_eq!(scroll_values("edge"), json!({"two-finger-scrolling-enabled": false, "edge-scrolling-enabled": true}));
        assert_eq!(scroll_method(&json!(true), &json!(false)), "two");
        assert_eq!(scroll_method(&json!(false), &json!(true)), "edge");
        assert_eq!(scroll_method(&json!(true), &json!(true)), "two", "GNOME prefers two-finger scrolling");
        assert_eq!(scroll_method(&json!(false), &json!(false)), "");
    }

    #[test]
    fn the_corner_is_gnomes_corner_push() {
        assert!(corner_on(&json!("areas")));
        assert!(!corner_on(&json!("fingers")) && !corner_on(&json!("default")) && !corner_on(&json!("none")));
        assert_eq!((corner_value(true), corner_value(false)), (json!("areas"), json!("fingers")));
    }

    #[test]
    fn reset_covers_the_whole_allow_row_and_no_timeout() {
        // amendment 67: the sensitivity dropdown is gone, and so is its key
        assert!(!RESET_KEYS.contains(&"disable-while-typing-timeout"));
        for k in keys().iter().filter(|k| k.schema == TOUCHPAD) {
            assert!(RESET_KEYS.contains(&k.key), "{} is reset", k.key);
        }
        assert_ne!(three_finger_tap(&json!("lmr")), three_finger_tap(&json!("default")));
    }
}
