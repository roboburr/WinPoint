//! Instellingen › Tijd en taal › Taal en regio › Regionale notatie (Windows 11: Language & region › Regional format ›
//! Change formats; batch E, notes/claude/settings-parity.md §2, amendments 16 and 17): the regional format with
//! GNOME's Formats preview, and Windows' "Notaties wijzigen" rows: Agenda, Eerste dag van de week (GNOME's
//! week-start-day), Korte datum, Korte tijd (GNOME's clock-format, the only place it is set: amendment 16) and Lange
//! tijd.
//!
//! The regional format is GNOME's Formats choice: `org.gnome.system.locale region` ("" follows the display language)
//! and AccountsService's FormatsLocale (`users.setFormats`), both written as GNOME's dialog writes them; it applies at
//! the next sign-in, so the page says so with "Afmelden". The time format goes to clock-format and, as GNOME does, the
//! file choosers' own clock-format keys. Linux has no per-field date patterns: the dates are shown as the region
//! writes them (the host's preview, `machine.locales {preview}`), not chosen.
//!
//! The region drop-down, its preview rows and the sign-out banner are shared with Taal en regio (time.rs).
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, Severity};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("formats", page)];
pub const CSS: &str = "";

pub(super) const REGION: GKey = gs::key("org.gnome.system.locale", "region");
pub(super) const WEEK_START: GKey = gs::key("org.gnome.desktop.calendar", "week-start-day");
pub(super) const CLOCK_FORMAT: GKey = gs::key("org.gnome.desktop.interface", "clock-format");
/// GNOME writes the time format to the file choosers' keys too (cc-datetime-panel.c change_clock_settings).
const FILE_CHOOSERS: [GKey; 2] = [gs::key("org.gtk.Settings.FileChooser", "clock-format"), gs::key("org.gtk.gtk4.Settings.FileChooser", "clock-format")];

// ------------------------------------------------------------------------------------- shared with Taal en regio

/// The locale the formats are written in now: the region, else the account's language, else the session's.
pub(super) fn effective_region(region: &str, me: &Value, locales: &Value) -> String {
    [region, me["language"].as_str().unwrap_or(""), locales["session"]["language"].as_str().unwrap_or("")]
        .into_iter()
        .find(|s| !s.is_empty())
        .unwrap_or("en_US.UTF-8")
        .to_owned()
}

/// The name of a locale in the list (its id when the list has no name for it).
pub(super) fn locale_name(locales: &Value, id: &str) -> String {
    locales["locales"].as_array().into_iter().flatten().find(|l| l["id"] == id).and_then(|l| l["name"].as_str()).unwrap_or(id).to_owned()
}

/// The region drop-down's options: "" (Windows' "Aanbevolen [Nederlands (Nederland)]": the display language's
/// formats) and every generated locale by name; the current value is kept when the list does not have it.
pub(super) fn region_options(locales: &Value, display_language: &str, current: &str) -> Vec<(Value, String)> {
    let recommended = common::fill(t("Aanbevolen [{name}]", "Recommended [{name}]"), &[("name", &locale_name(locales, display_language))]);
    let mut out: Vec<(Value, String)> = vec![(json!(""), recommended)];
    for l in locales["locales"].as_array().into_iter().flatten() {
        if let (Some(id), Some(name)) = (l["id"].as_str(), l["name"].as_str()) {
            out.push((json!(id), name.to_owned()));
        }
    }
    if !current.is_empty() && !out.iter().any(|(v, _)| v == current) {
        out.push((json!(current), current.to_owned()));
    }
    out
}

/// The locale AccountsService's FormatsLocale gets for a region choice: the choice, or for "" ("Aanbevolen") the
/// display language's locale, as GNOME's Formats dialog always passes a real locale (Ubuntu's accounts-daemon hands
/// it to save-to-pam-env, which refuses an empty one and would keep the LC_* lines of an earlier choice).
pub(super) fn formats_locale(choice: &str, display_language: &str) -> String {
    if choice.is_empty() { display_language.to_owned() } else { choice.to_owned() }
}

/// "Regionale notatie": the drop-down bound to `region`, which also sends the account's FormatsLocale (GNOME's
/// Formats dialog writes both); when AccountsService refuses, the error is shown and the region goes back.
pub(super) fn region_dropdown(p: &Page, gs: &Gs, locales: &Value, display_language: &str) -> Dropdown {
    let current = gs.value(&REGION).as_str().unwrap_or("").to_owned();
    let dd = gs.dropdown(&REGION, region_options(locales, display_language, &current));
    let (pp, gs2, display, before) = (p.clone(), gs.clone(), display_language.to_owned(), Rc::new(std::cell::RefCell::new(current)));
    dd.connect_changed(move |v| {
        let locale = formats_locale(v, &display);
        if locale.is_empty() {
            return;
        }
        let wait = pp.run("users.setFormats", json!({ "locale": locale }));
        let (gs3, before2, chosen) = (gs2.clone(), before.clone(), v.to_owned());
        pp.spawn(async move {
            if wait.await.is_some() {
                *before2.borrow_mut() = chosen;
            } else {
                // the session reads the region key: it goes back with AccountsService's copy
                let back = before2.borrow().clone();
                gs3.set(&REGION, json!(back), || {});
            }
        });
    });
    // GNOME's Region panel says at once that the formats apply at the next sign-in, and its preview follows: the
    // page is built again when the key changes (here or elsewhere)
    let pp = p.clone();
    gs.follow(&REGION, move |_| pp.refresh());
    dd
}

/// The rows of GNOME's Formats preview (Datums, Tijden, Datums en tijden, Getallen, Maatsysteem, Papier).
pub(super) fn preview_rows(preview: &Value) -> Vec<(String, String)> {
    if !preview.is_object() {
        return Vec::new();
    }
    let text = |k: &str| preview[k].as_str().unwrap_or("").to_owned();
    let measurement = if preview["measurement"] == "imperial" { t("Imperiaal", "Imperial") } else { t("Metrisch", "Metric") };
    let rows = vec![
        (t("Datums", "Dates").to_owned(), text("date")),
        (t("Tijden", "Times").to_owned(), text("time")),
        (t("Datums en tijden", "Dates and times").to_owned(), text("dateTime")),
        (t("Getallen", "Numbers").to_owned(), text("number")),
        (t("Maatsysteem", "Measurement system").to_owned(), measurement.to_owned()),
        (t("Papier", "Paper").to_owned(), paper_text(preview)),
    ];
    rows.into_iter().filter(|(_, v)| !v.is_empty()).collect()
}

/// "A4", "US Letter", or "216 × 330 mm".
pub(super) fn paper_text(preview: &Value) -> String {
    if let Some(name) = preview["paper"].as_str() {
        return name.to_owned();
    }
    match (preview["paperSize"][0].as_u64(), preview["paperSize"][1].as_u64()) {
        (Some(w), Some(h)) if w > 0 && h > 0 => format!("{w} × {h} mm"),
        _ => String::new(),
    }
}

/// Key/value rows, selectable (the look of Info's specifications).
pub(super) fn spec_rows(rows: &[(String, String)]) -> gtk::Widget {
    let grid = gtk::Grid::new();
    grid.add_css_class("st-specs");
    grid.set_column_spacing(24);
    grid.set_row_spacing(10);
    for (r, (k, v)) in rows.iter().enumerate() {
        let key = c::label(k, "st-spec-key", true);
        key.set_size_request(160, -1);
        key.set_valign(gtk::Align::Start);
        let val = c::label(v, "st-spec-value", true);
        val.set_selectable(true);
        val.set_hexpand(true);
        grid.attach(&key, 0, r as i32, 1, 1);
        grid.attach(&val, 1, r as i32, 1, 1);
    }
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&grid);
    wrap.upcast()
}

/// The sign-out banner of a language or format that applies at the next sign-in (Windows: "Windows moet je
/// afmelden om je nieuwe weergavetaal toe te passen." with "Afmelden"; GNOME's restart banner).
pub(super) fn signout_banner(p: &Page, message: &str) -> gtk::Widget {
    let bar = common::notice_full(Severity::Warning, "", message);
    let pp = p.clone();
    bar.set_action(Some((
        t("Afmelden", "Sign out"),
        Rc::new(move || {
            let wait = pp.run("system.power.action", json!({ "action": "logout" }));
            pp.spawn(async move {
                let _ = wait.await;
            });
        }),
    )));
    bar.widget().set_widget_name("st-signout");
    bar.widget().clone().upcast()
}

/// "Eerste dag van de week": GNOME's week-start-day, its "default" the region's (Windows' days, Monday first).
pub(super) fn week_start_options() -> Vec<(Value, String)> {
    vec![
        (json!("default"), t("Standaard (regio)", "Default (region)").into()),
        (json!("monday"), t("Maandag", "Monday").into()),
        (json!("tuesday"), t("Dinsdag", "Tuesday").into()),
        (json!("wednesday"), t("Woensdag", "Wednesday").into()),
        (json!("thursday"), t("Donderdag", "Thursday").into()),
        (json!("friday"), t("Vrijdag", "Friday").into()),
        (json!("saturday"), t("Zaterdag", "Saturday").into()),
        (json!("sunday"), t("Zondag", "Sunday").into()),
    ]
}

/// The label of a week-start-day value.
pub(super) fn week_start_text(value: &Value) -> String {
    week_start_options().into_iter().find(|(v, _)| v == value).map(|(_, l)| l).unwrap_or_else(|| t("Standaard (regio)", "Default (region)").into())
}

/// The sample times of "Korte tijd" and "Lange tijd" (14:35:12) in the clock format.
pub(super) fn time_sample(h24: bool, seconds: bool) -> &'static str {
    match (h24, seconds) {
        (true, false) => "14:35",
        (true, true) => "14:35:12",
        (false, false) => "2:35 PM",
        (false, true) => "2:35:12 PM",
    }
}

// -------------------------------------------------------------------------------------------------- the page

/// "Korte tijd": clock-format and, where GNOME has them, the file choosers' copies in one `gsettings.set`.
fn clock_dropdown(p: &Page, gs: &Gs, keys: &[GKey]) -> Dropdown {
    let options = vec![
        DropdownOption::new("24h", &format!("{} ({})", time_sample(true, false), t("24-uursnotatie", "24-hour clock"))),
        DropdownOption::new("12h", &format!("{} ({})", time_sample(false, false), t("12-uursnotatie", "12-hour clock"))),
    ];
    let dd = Dropdown::new(options, Some(gs.value(&CLOCK_FORMAT).as_str().unwrap_or("24h")));
    dd.widget().set_widget_name(&gs::control_name(p.id(), keys, &CLOCK_FORMAT));
    if !gs.writable(&CLOCK_FORMAT) {
        dd.widget().set_sensitive(false);
        dd.widget().set_tooltip_text(Some(gs::locked_text()));
    }
    let (pp, gs2, dd2) = (p.clone(), gs.clone(), dd.clone());
    dd.connect_changed(move |v| {
        let mut writes = vec![json!({ "schema": CLOCK_FORMAT.schema, "values": { "clock-format": v } })];
        for k in FILE_CHOOSERS.iter().filter(|k| gs2.writable(k)) {
            writes.push(json!({ "schema": k.schema, "values": { "clock-format": v } }));
        }
        let (wait, gs3, dd3) = (pp.run("gsettings.set", json!({ "writes": writes })), gs2.clone(), dd2.clone());
        pp.spawn(async move {
            if wait.await.is_none() {
                dd3.set_selected(gs3.value(&CLOCK_FORMAT).as_str());
            }
        });
    });
    let dd2 = dd.clone();
    gs.follow(&CLOCK_FORMAT, move |v| dd2.set_selected(v.as_str()));
    dd
}

/// A card with a value on the right.
fn value_card(icon: &str, title: &str, value: &str, name: &str) -> SettingsCard {
    let card = SettingsCard::new(icon, title, "");
    let label = common::value_label(value);
    label.set_widget_name(name);
    card.set_control(&label);
    card
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [REGION, WEEK_START, CLOCK_FORMAT, FILE_CHOOSERS[0].clone(), FILE_CHOOSERS[1].clone()];
        let (locales, me) = (p.call("machine.locales", json!({})), p.call("users.self", json!({})));
        let gs = Gs::load(&p, &keys).await;
        let (locales, me) = (locales.await.unwrap_or(Value::Null), me.await.unwrap_or(Value::Null));
        let region = gs.value(&REGION).as_str().unwrap_or("").to_owned();
        let effective = effective_region(&region, &me, &locales);
        let preview = p.get("machine.locales", json!({ "preview": effective })).await.map(|v| v["preview"].clone()).unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let session = locales["session"]["formats"].as_str().unwrap_or("");
        if !session.is_empty() && session != effective {
            out.push(signout_banner(&p, t("Je nieuwe regionale notatie wordt toegepast wanneer je je opnieuw aanmeldt.", "Your new regional format applies when you sign in again.")));
        }
        let display = me["language"].as_str().filter(|s| !s.is_empty()).or(locales["session"]["language"].as_str()).unwrap_or("").to_owned();

        // the regional format and GNOME's preview
        if gs.available(&REGION) {
            let top = SettingsGroup::new("", "");
            let card = SettingsExpander::new("globe", t("Regionale notatie", "Regional format"), t("Apps tonen datums, tijden en getallen in deze notatie", "Apps show dates, times and numbers in this format"));
            card.header().set_control(&common::sized(region_dropdown(&p, &gs, &locales, &display).widget(), 280));
            card.header_button().set_widget_name("st-formats-preview");
            let rows = preview_rows(&preview);
            if rows.is_empty() {
                card.add(&c::label(
                    t("Er is geen voorbeeld van deze notatie: de landinstelling is niet op deze computer gegenereerd.", "There’s no preview of this format: the locale isn’t generated on this computer."),
                    "w11-body",
                    true,
                ));
            } else {
                card.add(&spec_rows(&rows));
            }
            card.set_expanded(true);
            top.add(card.widget());
            out.push(top.widget().clone().upcast());
        }

        // Windows' "Notaties wijzigen"
        let formats = SettingsGroup::new(t("Notaties wijzigen", "Change formats"), "");
        formats.add(value_card("calendar", t("Agenda", "Calendar"), t("Gregoriaanse kalender", "Gregorian calendar"), "st-formats-calendar").widget());
        if let Some(card) = gs.dropdown_card(&WEEK_START, "calendar", t("Eerste dag van de week", "First day of week"), "", week_start_options()) {
            formats.add(card.widget());
        }
        if let Some(date) = preview["date"].as_str().filter(|s| !s.is_empty()) {
            formats.add(value_card("calendar", t("Korte datum", "Short date"), date, "st-formats-short-date").widget());
        }
        if gs.available(&CLOCK_FORMAT) {
            let card = SettingsCard::new("clock", t("Korte tijd", "Short time"), "");
            card.set_control(&common::sized(clock_dropdown(&p, &gs, &keys).widget(), 220));
            formats.add(card.widget());
            let long = SettingsCard::new("clock", t("Lange tijd", "Long time"), "");
            let label = common::value_label(time_sample(gs.value(&CLOCK_FORMAT) != "12h", true));
            label.set_widget_name("st-formats-long-time");
            long.set_control(&label);
            let l2 = label.clone();
            gs.follow(&CLOCK_FORMAT, move |v| l2.set_label(time_sample(*v != "12h", true)));
            formats.add(long.widget());
        }
        out.push(formats.widget().clone().upcast());

        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "datetime").widget());
        related.add(common::link_card(&p, "language").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_day_takes_every_value_of_gnomes_enum() {
        let values: Vec<String> = week_start_options().iter().map(|(v, _)| v.as_str().unwrap().to_owned()).collect();
        assert_eq!(values, ["default", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"]);
        assert_eq!(week_start_text(&json!("nonsense")), week_start_text(&json!("default")));
    }

    #[test]
    fn the_region_offers_the_recommended_format_first_and_keeps_the_current_one() {
        let locales = json!({"locales": [{"id": "en_GB.UTF-8", "name": "English (United Kingdom)"}, {"id": "nl_NL.UTF-8", "name": "Dutch (Netherlands)"}],
                             "session": {"language": "en_GB.UTF-8"}});
        let opts = region_options(&locales, "en_GB.UTF-8", "de_DE.UTF-8");
        assert_eq!(opts[0].0, json!(""));
        assert!(opts[0].1.contains("English (United Kingdom)"), "{:?}", opts[0]);
        assert_eq!(opts.iter().map(|(v, _)| v.as_str().unwrap()).collect::<Vec<_>>(), ["", "en_GB.UTF-8", "nl_NL.UTF-8", "de_DE.UTF-8"]);
        assert_eq!(effective_region("", &json!({"language": ""}), &locales), "en_GB.UTF-8");
        assert_eq!(effective_region("", &json!({"language": "nl_NL.UTF-8"}), &locales), "nl_NL.UTF-8");
        assert_eq!(effective_region("de_DE.UTF-8", &json!({}), &locales), "de_DE.UTF-8");
    }

    #[test]
    fn the_recommended_format_sends_the_display_languages_locale() {
        assert_eq!(formats_locale("", "nl_NL.UTF-8"), "nl_NL.UTF-8", "never an empty FormatsLocale");
        assert_eq!(formats_locale("de_DE.UTF-8", "nl_NL.UTF-8"), "de_DE.UTF-8");
    }

    #[test]
    fn the_preview_reads_as_gnome_shows_it() {
        let rows = preview_rows(&json!({"date": "16-11-26", "time": "14:35:00", "dateTime": "ma 16 nov 2026", "number": "123.456.789,00",
                                        "measurement": "metric", "paper": "A4"}));
        assert_eq!(rows.len(), 6);
        assert_eq!(rows[3].1, "123.456.789,00");
        assert_eq!(paper_text(&json!({"paper": null, "paperSize": [216, 330]})), "216 × 330 mm");
        assert_eq!(paper_text(&json!({})), "");
        assert!(preview_rows(&Value::Null).is_empty(), "no preview, no rows");
        assert_eq!(time_sample(false, true), "2:35:12 PM");
    }
}
