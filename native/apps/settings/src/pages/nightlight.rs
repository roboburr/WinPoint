//! Instellingen › Systeem › Beeldscherm › Nachtlampje (batch D, notes/claude/settings-parity.md §2, amendment 25):
//! Windows 11's night light page on GNOME's own (gsd-color, org.gnome.settings-daemon.plugins.color): the status
//! row with Nu uitschakelen / Nu inschakelen (gsd's DisabledUntilTomorrow, host display.nightLight.*), Sterkte
//! (night-light-temperature, 4700 K at 0 % to 1700 K at 100 %, GNOME's slider range), and Nachtlampje plannen
//! (night-light-enabled) with Zonsondergang tot zonsopkomst / Uren instellen and the two time pickers. The schedule
//! is never rewritten: outside it the row says when night light turns on. GNOME's "Night Light Unavailable" when
//! mutter or gsd-color say so.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use crate::pickers::{self, TimePicker};
use kit::controls::{self as c, SettingsCard, SettingsGroup, Severity};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("nightlight", nightlight)];
pub const CSS: &str = "";

const SCHEMA: &str = "org.gnome.settings-daemon.plugins.color";
const ENABLED: GKey = gs::key(SCHEMA, "night-light-enabled");
const TEMPERATURE: GKey = gs::key(SCHEMA, "night-light-temperature");
const AUTOMATIC: GKey = gs::key(SCHEMA, "night-light-schedule-automatic");
const FROM: GKey = gs::key(SCHEMA, "night-light-schedule-from");
const TO: GKey = gs::key(SCHEMA, "night-light-schedule-to");
const COORDINATES: GKey = gs::key(SCHEMA, "night-light-last-coordinates");
const LOCATION: GKey = gs::key("org.gnome.system.location", "enabled");
fn keys() -> Vec<GKey> {
    vec![ENABLED, TEMPERATURE, AUTOMATIC, FROM, TO, COORDINATES, LOCATION]
}

/// Windows' Sterkte (0-100) for a colour temperature: 0 % is GNOME's 4700 K, 100 % its warmest 1700 K.
pub(crate) fn strength_of(kelvin: f64) -> f64 {
    ((4700.0 - kelvin) / 30.0).clamp(0.0, 100.0).round()
}

/// The colour temperature (K) of a strength.
pub(crate) fn kelvin_of(strength: f64) -> u32 {
    (4700.0 - 30.0 * strength.clamp(0.0, 100.0)).round() as u32
}

fn strength_ui(v: &Value) -> f64 {
    strength_of(v.as_f64().unwrap_or(2700.0))
}

fn kelvin_value(n: f64) -> Value {
    json!(kelvin_of(n))
}

/// What the status row says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Now {
    /// Nachtlampje plannen is off
    Off,
    /// on now (until the end of the schedule)
    On,
    /// Nu uitschakelen was used: off until tomorrow
    Disabled,
    /// outside the schedule: turns on at these hours (amendment 25's info line)
    At(f64),
    /// sunset to sunrise without a known sunset
    AtSunset,
}

pub(crate) fn now_of(enabled: bool, status: &Value, automatic: bool, from: f64) -> Now {
    if !enabled {
        return Now::Off;
    }
    if status["disabledUntilTomorrow"] == true {
        return Now::Disabled;
    }
    if status["active"] == true {
        return Now::On;
    }
    match if automatic { status["sunset"].as_f64() } else { Some(from) } {
        Some(h) => Now::At(h),
        None => Now::AtSunset,
    }
}

fn h24() -> bool {
    common::setting_bool("time.h24", true)
}

/// "Zonsondergang tot zonsopkomst (20:41; 06:07)" with gsd-color's times when it knows them.
fn sunset_label(status: &Value) -> String {
    match (status["sunset"].as_f64(), status["sunrise"].as_f64()) {
        (Some(set), Some(rise)) => common::fill(
            t("Zonsondergang tot zonsopkomst ({from}; {to})", "Sunset to sunrise ({from}; {to})"),
            &[("from", &pickers::time_text(set, h24())), ("to", &pickers::time_text(rise, h24()))],
        ),
        _ => t("Zonsondergang tot zonsopkomst", "Sunset to sunrise").to_owned(),
    }
}

struct Night {
    p: Page,
    gs: Gs,
    status: RefCell<Value>,
    now_slot: gtk::Box,
    location_slot: gtk::Box,
    manual: RefCell<Vec<gtk::Widget>>,
    radio: RefCell<Option<c::RadioGroup>>,
}

/// The status row: Nu uitschakelen while it is on, Nu inschakelen while it is off until tomorrow, else when it
/// turns on (amendment 25).
fn render_now(n: &Rc<Night>) {
    let status = n.status.borrow().clone();
    let enabled = n.gs.value(&ENABLED) == true;
    let automatic = n.gs.value(&AUTOMATIC) != false;
    let from = n.gs.value(&FROM).as_f64().unwrap_or(20.0);
    let to = if automatic { status["sunrise"].as_f64() } else { n.gs.value(&TO).as_f64() };
    let state = now_of(enabled, &status, automatic, from);
    let (title, desc, button): (String, String, Option<(&str, bool)>) = match state {
        Now::Off => (
            t("Nachtlampje is uitgeschakeld", "Night light is off").into(),
            t("Schakel hieronder Nachtlampje plannen in om warmere kleuren te gebruiken", "Turn on Schedule night light below to use warmer colours").into(),
            None,
        ),
        Now::On => (
            t("Nachtlampje is ingeschakeld", "Night light is on").into(),
            to.map(|h| common::fill(t("Tot {time}", "Until {time}"), &[("time", &pickers::time_text(h, h24()))])).unwrap_or_default(),
            Some((t("Nu uitschakelen", "Turn off now"), true)),
        ),
        Now::Disabled => (
            t("Uitgeschakeld tot morgen", "Off until tomorrow").into(),
            t("Nachtlampje wordt morgen weer volgens de planning ingeschakeld", "Night light turns on by its schedule again tomorrow").into(),
            Some((t("Nu inschakelen", "Turn on now"), false)),
        ),
        Now::At(h) => (common::fill(t("Wordt om {time} ingeschakeld", "Will turn on at {time}"), &[("time", &pickers::time_text(h, h24()))]), String::new(), None),
        Now::AtSunset => (t("Wordt bij zonsondergang ingeschakeld", "Will turn on at sunset").into(), String::new(), None),
    };
    let card = SettingsCard::new("night-light", &title, &desc);
    card.widget().set_widget_name("st-nightlight-status");
    if let Some((label, off)) = button {
        let b = c::button(label, c::Variant::Standard);
        b.set_widget_name("st-nightlight-now");
        let n2 = n.clone();
        b.connect_clicked(move |_| {
            let wait = n2.p.run_with("display.nightLight.disableUntilTomorrow", json!({"on": off}), Some(t("Nachtlampje kon niet worden gewijzigd", "Couldn’t change night light")), false);
            let n3 = n2.clone();
            n2.p.spawn(async move {
                if let Some(v) = wait.await
                    && v["available"] == true
                    && n3.p.alive()
                {
                    *n3.status.borrow_mut() = v;
                    render_now(&n3);
                }
            });
        });
        card.set_control(&b);
    }
    common::set_slot(&n.now_slot, &[card.widget().clone()]);
}

/// The manual times and the location hint follow the schedule's keys.
fn render_schedule(n: &Rc<Night>) {
    let enabled = n.gs.value(&ENABLED) == true;
    let automatic = n.gs.value(&AUTOMATIC) != false;
    for w in n.manual.borrow().iter() {
        w.set_visible(!automatic);
        w.set_sensitive(enabled);
    }
    if let Some(r) = n.radio.borrow().as_ref() {
        r.widget().set_sensitive(enabled && n.gs.writable(&AUTOMATIC));
    }
    // Windows: "Schakel locatieservices in om het nachtlampje bij zonsondergang te plannen"
    if enabled && automatic && n.gs.available(&LOCATION) && n.gs.value(&LOCATION) == false {
        let bar = common::notice(t("Schakel locatieservices in om het nachtlampje bij zonsondergang te plannen.", "Turn on location services to schedule night light at sunset."));
        let link = c::hyperlink(t("Locatie-instellingen", "Location settings"), None);
        link.set_widget_name("st-nightlight-location");
        link.set_halign(gtk::Align::Start);
        let pp = n.p.clone();
        link.connect_clicked(move |_| pp.go("location"));
        let col = gtk::Box::new(gtk::Orientation::Vertical, 6);
        col.append(bar.widget());
        col.append(&link);
        common::set_slot(&n.location_slot, &[col.upcast()]);
    } else {
        common::set_slot(&n.location_slot, &[]);
    }
}

/// A time picker bound to night-light-schedule-from / -to (fractional hours, GNOME's own values).
fn time_row(n: &Rc<Night>, key: GKey, title: &str, fallback: f64) -> SettingsCard {
    let picker = TimePicker::new(n.gs.value(&key).as_f64().unwrap_or(fallback), h24());
    picker.widget().set_widget_name(&gs::control_name(n.p.id(), &keys(), &key));
    if !n.gs.writable(&key) {
        picker.widget().set_sensitive(false);
        picker.widget().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (gs2, p2, k2) = (n.gs.clone(), picker.clone(), key.clone());
        picker.connect_changed(move |hours| {
            let (gs3, p3, k3) = (gs2.clone(), p2.clone(), k2.clone());
            gs2.set(&k2, json!(hours), move || p3.set_value(gs3.value(&k3).as_f64().unwrap_or(fallback)));
        });
    }
    {
        let p2 = picker.clone();
        n.gs.follow(&key, move |v| p2.set_value(v.as_f64().unwrap_or(fallback)));
    }
    let card = SettingsCard::new("", title, "");
    card.set_control(picker.widget());
    card
}

fn nightlight(p: Page) -> PageFuture {
    Box::pin(async move {
        let status = p.call("display.nightLight.status", json!({}));
        let gs = Gs::load(&p, &keys()).await;
        let status = status.await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if status["supported"] == false || !gs.available(&ENABLED) {
            let bar = common::notice_full(Severity::Info, t("Nachtlampje is niet beschikbaar", "Night light isn’t available"), t("Dit beeldscherm ondersteunt geen kleuraanpassing.", "This display doesn’t support colour adjustment."));
            bar.widget().set_widget_name("st-nightlight-unavailable");
            return vec![bar.widget().clone().upcast()];
        }
        let n = Rc::new(Night {
            p: p.clone(),
            gs: gs.clone(),
            status: RefCell::new(status.clone()),
            now_slot: common::slot(),
            location_slot: common::slot(),
            manual: RefCell::new(Vec::new()),
            radio: RefCell::new(None),
        });
        let mut out: Content = Vec::new();
        let top = SettingsGroup::new("", "");
        top.add(&n.now_slot);
        // Sterkte
        let strength = gs.slider(&TEMPERATURE, 0.0, 100.0, 1.0, strength_ui, kelvin_value);
        strength.widget().set_size_request(220, -1);
        let card = SettingsCard::new("", t("Sterkte", "Strength"), t("Hoe warmer, hoe minder blauw licht", "The warmer, the less blue light"));
        card.set_control(strength.widget());
        top.add(card.widget());
        // Nachtlampje plannen: the switch on a card of its own (a switch inside an expander's header button never gets
        // the pointer's click, kit cards.rs), the schedule's rows below it
        let plan = SettingsCard::new("calendar", t("Nachtlampje plannen", "Schedule night light"), "");
        plan.set_control(gs.toggle(&ENABLED, false).widget());
        plan.widget().set_widget_name("st-nightlight-schedule");
        top.add(plan.widget());
        let radio = gs.radio(&AUTOMATIC, vec![(json!(true), sunset_label(&status)), (json!(false), t("Uren instellen", "Set hours").to_owned())]);
        let radio_card = SettingsCard::new("", "", "");
        radio_card.set_icon_widget(radio.widget());
        top.add(radio_card.widget());
        *n.radio.borrow_mut() = Some(radio);
        let on = time_row(&n, FROM, t("Inschakelen", "Turn on"), 20.0);
        let off = time_row(&n, TO, t("Uitschakelen", "Turn off"), 6.0);
        top.add(on.widget());
        top.add(off.widget());
        *n.manual.borrow_mut() = vec![on.widget().clone(), off.widget().clone()];
        top.add(&n.location_slot);
        out.push(top.widget().clone().upcast());
        render_now(&n);
        render_schedule(&n);
        for key in [&ENABLED, &AUTOMATIC, &FROM, &TO, &LOCATION] {
            let n2 = n.clone();
            gs.follow(key, move |_| {
                render_now(&n2);
                render_schedule(&n2);
            });
        }
        {
            let n2 = n.clone();
            p.on("display.nightLight.changed", move |d| {
                if d.is_object() {
                    *n2.status.borrow_mut() = d.clone();
                    render_now(&n2);
                }
            });
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strength_maps_onto_gnomes_colour_temperature() {
        assert_eq!(kelvin_of(0.0), 4700);
        assert_eq!(kelvin_of(100.0), 1700);
        assert_eq!(kelvin_of(50.0), 3200);
        assert_eq!(kelvin_of(150.0), 1700, "clamped");
        assert_eq!(strength_of(2700.0), 67.0, "GNOME's default");
        assert_eq!(strength_of(6500.0), 0.0);
        assert_eq!(strength_of(1000.0), 100.0);
        for s in [0.0, 13.0, 67.0, 100.0] {
            assert_eq!(strength_of(kelvin_of(s) as f64), s, "round trip at {s}");
        }
    }

    #[test]
    fn the_status_row_follows_amendment_25() {
        let on = json!({"active": true, "disabledUntilTomorrow": false, "sunset": 20.5});
        assert_eq!(now_of(false, &on, true, 20.0), Now::Off);
        assert_eq!(now_of(true, &on, true, 20.0), Now::On);
        let off = json!({"active": false, "disabledUntilTomorrow": true});
        assert_eq!(now_of(true, &off, false, 20.0), Now::Disabled, "Nu inschakelen only now");
        let outside = json!({"active": false, "disabledUntilTomorrow": false, "sunset": 20.5});
        assert_eq!(now_of(true, &outside, true, 21.0), Now::At(20.5), "sunset to sunrise");
        assert_eq!(now_of(true, &outside, false, 21.0), Now::At(21.0), "set hours");
        assert_eq!(now_of(true, &json!({}), true, 21.0), Now::AtSunset);
    }
}
