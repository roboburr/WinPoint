//! Instellingen › Tijd en taal (6ebf169f:vendor/ui/js/apps/settings/pages/time.js): the overview with a live clock,
//! Datum en tijd (read-only system time, the taskbar clock's display) and Taal en regio (the display language and
//! how dates, times and numbers look).
use super::network::specs;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch};
use kit::format::{self, DateFormat};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("time", time), ("datetime", datetime), ("language", language)];
pub const CSS: &str = r#"
.st-clock { padding: 4px 0 6px 0; }
.st-clock-time {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: 56px; font-weight: 600; letter-spacing: -0.5px;
}
.st-clock-date { font-size: var(--fs-body-large); color: var(--text-1); }
.st-clock-meta { margin-top: 6px; color: var(--text-2); }
"#;

/// "Europe/Amsterdam" -> "Europe / Amsterdam".
fn tz_name(tz: &str) -> String {
    tz.replace('_', " ").replacen('/', " / ", 1)
}

/// 25 October 2026, 14:35 local time (the sample of time.js).
fn sample_ms() -> f64 {
    glib::DateTime::from_local(2026, 10, 25, 14, 35, 0.0).map(|d| d.to_unix() as f64 * 1000.0).unwrap_or_else(|_| format::now_ms())
}

fn h24() -> bool {
    kit::ctx().setting("time.h24") != false
}

fn time(p: Page) -> PageFuture {
    Box::pin(async move {
        let st = p.get("system.time", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let clock = c::label("", "st-clock-time w11-tabular", false);
        clock.set_widget_name("time-clock");
        let day = c::label("", "st-clock-date", true);
        let tick = {
            let (clock, day) = (clock.clone(), day.clone());
            move || {
                let now = format::now_ms();
                clock.set_label(&format::time(now, common::setting_bool("time.showSeconds", false), h24()));
                day.set_label(&format::date(now, DateFormat::Long));
            }
        };
        tick();
        {
            let tick = tick.clone();
            p.interval(1000, move || {
                tick();
                true
            });
        }
        p.on_settings(move |_, _| tick());
        let hero = gtk::Box::new(gtk::Orientation::Vertical, 2);
        hero.add_css_class("st-clock");
        hero.append(&clock);
        hero.append(&day);
        let meta = crate::widgets::Wrap::new(20, 4);
        meta.add_css_class("st-clock-meta");
        if let Some(tz) = st.as_ref().and_then(|s| s["timezone"].as_str()).filter(|s| !s.is_empty()) {
            meta.append(&c::label(&common::fill(t("Tijdzone: {tz}", "Time zone: {tz}"), &[("tz", &tz_name(tz))]), "", false));
        }
        let lang = if kit::lang() == "en" { "English" } else { "Nederlands" };
        meta.append(&c::label(&common::fill(t("Taal: {l}", "Language: {l}"), &[("l", lang)]), "", false));
        hero.append(&meta);
        let links = SettingsGroup::new("", "");
        links.add(common::link_card(&p, "datetime").widget());
        links.add(common::link_card(&p, "language").widget());
        vec![hero.upcast(), links.widget().clone().upcast()]
    })
}

fn managed() -> &'static str {
    t(
        "Dit regelt Ubuntu. Wijzigen kan in de app Instellingen van Ubuntu: Systeem › Datum en tijd.",
        "Ubuntu manages this. Change it in Ubuntu’s Settings app: System › Date & Time.",
    )
}

/// "Seconden weergeven": one switch for the clock and the taskbar (time.js secondsToggle).
pub fn seconds_toggle(p: &Page) -> ToggleSwitch {
    let read = || common::setting_bool("time.showSeconds", false) || common::setting_bool("taskbar.showSeconds", false);
    let sw = ToggleSwitch::new(read());
    let (pp, sw2) = (p.clone(), sw.clone());
    sw.connect_toggled(move |on| {
        let wait = pp.call("settings.patch", json!({ "patch": { "time": { "showSeconds": on }, "taskbar": { "showSeconds": on } } }));
        let (p2, sw3) = (pp.clone(), sw2.clone());
        pp.spawn(async move {
            if let Err(e) = wait.await {
                sw3.set_active(!on);
                p2.error(&e, None);
            }
        });
    });
    let sw2 = sw.clone();
    p.on_settings(move |_, _| sw2.set_active(read()));
    sw
}

fn datetime(p: Page) -> PageFuture {
    Box::pin(async move {
        let st = p.get("system.time", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let tz = st.as_ref().and_then(|s| s["timezone"].as_str()).filter(|s| !s.is_empty()).map(tz_name);
        let auto = SettingsCard::new(
            "clock",
            t("Tijd automatisch instellen", "Set time automatically"),
            match &st {
                Some(s) if s["ntp"] == true => t("Aan – de klok wordt via internet gelijkgezet", "On – the clock is synchronised over the internet"),
                Some(_) => t("Uit", "Off"),
                None => managed(),
            },
        );
        if let Some(s) = &st {
            let ntp = ToggleSwitch::new(s["ntp"] == true);
            ntp.set_sensitive(false);
            auto.set_control(ntp.widget());
        }
        let tz_dd = Dropdown::new(vec![DropdownOption::new("tz", tz.as_deref().unwrap_or(t("Onbekend", "Unknown")))], Some("tz"));
        tz_dd.widget().set_sensitive(false);
        let tz_card = SettingsCard::new("globe", t("Tijdzone", "Time zone"), managed());
        tz_card.set_control(&common::sized(tz_dd.widget(), 260));
        let sys = SettingsGroup::new("", "");
        sys.add(auto.widget());
        sys.add(tz_card.widget());

        let sample = sample_ms();
        let display = SettingsGroup::new(t("Weergave", "Display"), "");
        let (h24_card, h24_sw) = common::toggle_card(
            &p,
            "clock",
            t("24-uursnotatie", "24-hour clock"),
            &common::fill(
                t("Bijvoorbeeld {a} in plaats van {b}", "For example {a} instead of {b}"),
                &[("a", &format::time(sample, false, true)), ("b", &format::time(sample, false, false))],
            ),
            "time.h24",
            false,
        );
        h24_sw.switch().set_widget_name("datetime-h24");
        display.add(h24_card.widget());
        let secs = seconds_toggle(&p);
        secs.switch().set_widget_name("datetime-seconds");
        let secs_card = SettingsCard::new("timer", t("Seconden weergeven", "Show seconds"), t("In de klok op de taakbalk", "In the taskbar clock"));
        secs_card.set_control(secs.widget());
        display.add(secs_card.widget());
        let (date_card, date_sw) = common::toggle_card(
            &p,
            "calendar",
            t("Datum weergeven op de taakbalk", "Show date on the taskbar"),
            t("Onder de tijd, rechtsonder", "Below the time, bottom right"),
            "time.showDate",
            false,
        );
        date_sw.switch().set_widget_name("datetime-date");
        display.add(date_card.widget());
        let (first_card, first_dd) = common::dropdown_card(
            &p,
            "calendar",
            t("Eerste dag van de week", "First day of week"),
            t("In de agenda van de taakbalk", "In the taskbar calendar"),
            "time.firstDayOfWeek",
            vec![(json!(1), t("Maandag", "Monday").into()), (json!(0), t("Zondag", "Sunday").into()), (json!(6), t("Zaterdag", "Saturday").into())],
            Some(json!(1)),
        );
        first_dd.widget().set_widget_name("datetime-first-day");
        // the bound drop-down at its CSS width: out of the card first, then back in a fixed-width slot
        first_card.clear_control();
        first_card.set_control(&common::sized(first_dd.widget(), 180));
        display.add(first_card.widget());
        vec![sys.widget().clone().upcast(), display.widget().clone().upcast()]
    })
}

/// The dropdown's value as contracts C1 reads settings.language: exactly "en" is English, anything else Dutch.
fn lang_value() -> &'static str {
    if kit::ctx().setting("language") == "en" { "en" } else { "nl" }
}

fn language(p: Page) -> PageFuture {
    Box::pin(async move {
        // endonyms, as Windows lists display languages (D-G1b: the Dutch entry stays "Nederlands")
        let dd = Dropdown::new(vec![DropdownOption::new("nl", "Nederlands"), DropdownOption::new("en", "English (United Kingdom)")], Some(lang_value()));
        dd.widget().set_widget_name("language-display");
        {
            let (pp, dd2) = (p.clone(), dd.clone());
            dd.connect_changed(move |v| {
                let dd3 = dd2.clone();
                common::set_setting(&pp, "language", json!(v), move |ok| {
                    if !ok {
                        dd3.set_selected(Some(lang_value()));
                    }
                });
            });
        }
        {
            let dd = dd.clone();
            p.on_settings(move |_, _| dd.set_selected(Some(lang_value())));
        }
        let card = SettingsCard::new(
            "language",
            t("Weergavetaal van WinPoint", "WinPoint display language"),
            t(
                "De taal van Instellingen, Start, de taakbalk en de andere onderdelen van WinPoint",
                "The language of Settings, Start, the taskbar and the other parts of WinPoint",
            ),
        );
        card.set_control(&common::sized(dd.widget(), 260));
        let lang = SettingsGroup::new("", "");
        lang.add(card.widget());
        lang.add(common::notice(t("Apps van Ubuntu gebruiken hun eigen taalinstelling.", "Ubuntu apps use their own language setting.")).widget());

        let sample = sample_ms();
        let rows: Vec<(String, String)> = vec![
            (t("Korte datum", "Short date").into(), format::date(sample, DateFormat::Short)),
            (t("Lange datum", "Long date").into(), format::date(sample, DateFormat::Long)),
            (t("Tijd", "Time").into(), format::time(sample, false, h24())),
            (t("Getal", "Number").into(), format::number(1234567.89, 2)),
            (t("Bestandsgrootte", "File size").into(), format::bytes(1288490188.0)),
        ];
        let formats = SettingsExpander::new("globe", t("Notaties", "Formats"), t("Zo worden datum, tijd en getallen weergegeven", "How dates, times and numbers are shown"));
        formats.set_expanded(true);
        formats.add(&specs(&rows, false));
        let fmt = SettingsGroup::new("", "");
        fmt.add(formats.widget());
        vec![lang.widget().clone().upcast(), fmt.widget().clone().upcast()]
    })
}
