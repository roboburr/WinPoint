//! Instellingen › Tijd en taal (6ebf169f:vendor/ui/js/apps/settings/pages/time.js; settings parity batch E,
//! notes/claude/settings-parity.md §2): the overview with a live clock, Datum en tijd and Taal en regio, in Windows 11's
//! order with GNOME Settings' Date & Time and Region & Language functions behind them.
//!
//! * Datum en tijd (amendments 16, 30, 68): Tijd automatisch instellen (timedate1 SetNTP), Tijdzone automatisch
//!   instellen (GNOME's automatic-timezone, which needs location services), De datum en tijd handmatig instellen
//!   (SetTime, only while NTP is off), Tijdzone (SetTimezone, Windows' "(UTC+01:00) Amsterdam" labels), Automatisch
//!   aanpassen aan zomertijd (always on: tzdata does it), the system-tray clock's switches (GNOME's clock-show-date,
//!   -seconds, -weekday and the calendar's week numbers, mirrored to WinPoint's tray) and, under Aanvullende
//!   instellingen, Nu synchroniseren (SetNTP false then true; no timesync1). The 24-hour clock and the first day of
//!   the week are on Regionale notatie only, linked from here (amendment 16).
//! * Taal en regio (amendments 17, 18, 59): Weergavetaal (AccountsService SetLanguage, applied at the next sign-in,
//!   with Windows' sign-out banner), Taal van WinPoint (settings.language "system", "nl" or "en"), Voorkeurstalen with
//!   Een taal toevoegen (language support through PackageKit, before a language can be the display language), and
//!   the regional format inline with its samples and Notaties wijzigen (formats.rs).
use super::formats::{self, CLOCK_FORMAT, REGION, WEEK_START};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use crate::pickers::{DatePicker, TimePicker, admin_button};
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, MenuItem, SearchBox, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch, Variant};
use kit::format::{self, DateFormat};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("time", time), ("datetime", datetime), ("language", language)];
pub const CSS: &str = r#"
.st-clock { padding: 4px 0 6px 0; }
.st-clock-time {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: 56px; font-weight: 600; letter-spacing: -0.5px;
}
.st-clock-date { font-size: var(--fs-body-large); color: var(--text-1); }
.st-clock-meta { margin-top: 6px; color: var(--text-2); }
.st-language-list row { padding: 6px 10px; border-radius: 4px; }
"#;

const AUTO_TZ: GKey = gs::key("org.gnome.desktop.datetime", "automatic-timezone");
const LOCATION: GKey = gs::key("org.gnome.system.location", "enabled");
const SHOW_DATE: GKey = gs::key("org.gnome.desktop.interface", "clock-show-date");
const SHOW_SECONDS: GKey = gs::key("org.gnome.desktop.interface", "clock-show-seconds");
const SHOW_WEEKDAY: GKey = gs::key("org.gnome.desktop.interface", "clock-show-weekday");
const WEEK_NUMBERS: GKey = gs::key("org.gnome.desktop.calendar", "show-weekdate");

fn h24() -> bool {
    kit::ctx().setting("time.h24") != false
}

/// Build the page again shortly after `machine.changed` of one of `whats` (a burst is one rebuild; the language
/// install's progress events are not changes).
fn refresh_on_what(p: &Page, whats: &'static [&'static str]) {
    let pending = Rc::new(Cell::new(false));
    let pp = p.clone();
    p.on("machine.changed", move |d| {
        let what = d["what"].as_str().unwrap_or("");
        if !whats.contains(&what) || d.get("percent").is_some() || pending.replace(true) {
            return;
        }
        let (pending, p2) = (pending.clone(), pp.clone());
        pp.timeout(300, move || {
            pending.set(false);
            p2.refresh();
        });
    });
}

/// The clock of Tijd en taal and Datum en tijd: the time, the date and the zone, live.
fn clock_hero(p: &Page, zone: Option<&str>, language: Option<&str>) -> gtk::Box {
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
    if let Some(zone) = zone.filter(|z| !z.is_empty()) {
        meta.append(&c::label(&common::fill(t("Tijdzone: {tz}", "Time zone: {tz}"), &[("tz", zone)]), "", false));
    }
    if let Some(language) = language {
        meta.append(&c::label(&common::fill(t("Taal: {l}", "Language: {l}"), &[("l", language)]), "", false));
    }
    hero.append(&meta);
    hero
}

/// WinPoint's own language, as its drop-down names it.
fn winpoint_language_name() -> &'static str {
    if kit::lang() == "en" { "English (United Kingdom)" } else { "Nederlands" }
}

fn time(p: Page) -> PageFuture {
    Box::pin(async move {
        let st = p.get("machine.time.get", json!({})).await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        let hero = clock_hero(&p, st["zoneLabel"].as_str(), Some(winpoint_language_name()));
        let links = common::child_links(&p, "time");
        refresh_on_what(&p, &["time"]);
        vec![hero.upcast(), links.widget().clone().upcast()]
    })
}

// --------------------------------------------------------------------------------------------- Datum en tijd

/// (hours, minutes) of a TimePicker value (fractional hours), minutes rounded.
pub fn hours_minutes(hours: f64) -> (i32, i32) {
    let total = (hours.rem_euclid(24.0) * 60.0).round() as i32 % (24 * 60);
    (total / 60, total % 60)
}

/// The microseconds since the epoch of a local date and time (what timedate1 SetTime takes), None for no such time.
pub fn local_usec(date: (i32, u32, u32), hours: f64) -> Option<i64> {
    let (h, m) = hours_minutes(hours);
    let dt = glib::DateTime::from_local(date.0, date.1 as i32, date.2 as i32, h, m, 0.0).ok()?;
    Some(dt.to_unix() * 1_000_000)
}

/// "Datum en tijd wijzigen" (Windows' dialog; GNOME's Date & Time dialog): Datum, Tijd, Wijzigen.
fn manual_dialog(p: &Page) {
    let now = glib::DateTime::now_local().ok();
    let date = DatePicker::new(now.as_ref().map(|d| (d.year(), d.month() as u32, d.day_of_month() as u32)).unwrap_or((2026, 1, 1)));
    date.widget().set_widget_name("st-datetime-date");
    let time = TimePicker::new(now.as_ref().map(|d| d.hour() as f64 + d.minute() as f64 / 60.0).unwrap_or(12.0), h24());
    time.widget().set_widget_name("st-datetime-time");
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&c::field(t("Datum", "Date"), date.widget(), None));
    body.append(&c::field(t("Tijd", "Time"), time.widget(), None));
    let pp = p.clone();
    // glossary-ok: Datum en tijd
    Dialog::new(t("Datum en tijd wijzigen", "Change date and time"))
        .body(&body)
        .primary(t("Wijzigen", "Change"))
        .width(400)
        .on_primary(move |d| {
            let Some(usec) = local_usec(date.value(), time.value()) else {
                return false;
            };
            d.set_busy(true);
            let (d2, wait) = (d.clone(), pp.run("machine.time.setTime", json!({ "usec": usec })));
            pp.spawn(async move {
                let ok = wait.await.is_some();
                d2.set_busy(false);
                if ok {
                    d2.close(DialogResult::Primary);
                }
            });
            false
        })
        .show(&p.anchor(), |_| {});
}

/// The lines under "Nu synchroniseren": the last synchronisation this computer saw (or whether it is synchronised)
/// and the time server, when its configuration names one.
pub fn sync_lines(st: &Value) -> Vec<String> {
    let mut lines = Vec::new();
    match st["lastSync"].as_f64() {
        Some(ms) => lines.push(common::fill(
            t("Laatste geslaagde tijdsynchronisatie: {when}", "Last successful time synchronisation: {when}"),
            &[("when", &format!("{} {}", format::date(ms, DateFormat::Short), format::time(ms, true, h24())))],
        )),
        None if st["synced"] == true => lines.push(t("De tijd is gesynchroniseerd met een tijdserver.", "The time is synchronised with a time server.").to_owned()),
        None => lines.push(t("De tijd is nog niet gesynchroniseerd.", "The time hasn’t been synchronised yet.").to_owned()),
    }
    if let Some(server) = st["server"].as_str().filter(|s| !s.is_empty()) {
        lines.push(common::fill(t("Tijdserver: {server}", "Time server: {server}"), &[("server", server)]));
    }
    lines
}

/// Whether the time zone can be chosen: GNOME's rule (cc-datetime-page.c on_permission_changed), the zone button stays
/// sensitive unless the automatic time zone is on and location services allow it (`!auto_timezone ||
/// !location_allowed`), so with location services off a zone can still be picked.
pub fn zone_sensitive(available: bool, auto_tz: bool, located: bool) -> bool {
    available && !(auto_tz && located)
}

/// The time zone drop-down's options: the host's list, and the current zone when the list lacks it.
pub fn zone_options(zones: &Value, current: &str, current_label: &str) -> Vec<DropdownOption> {
    let mut out: Vec<DropdownOption> = zones["zones"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|z| Some(DropdownOption::new(z["id"].as_str()?, z["label"].as_str()?)))
        .collect();
    if !current.is_empty() && !out.iter().any(|o| o.value == current) {
        out.insert(0, DropdownOption::new(current, if current_label.is_empty() { current } else { current_label }));
    }
    out
}

fn datetime(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [AUTO_TZ, LOCATION, SHOW_DATE, SHOW_SECONDS, SHOW_WEEKDAY, WEEK_NUMBERS];
        let (st, zones) = (p.call("machine.time.get", json!({})), p.call("machine.time.zones", json!({})));
        let gs = Gs::load(&p, &keys).await;
        let (st, zones) = (st.await.unwrap_or(Value::Null), zones.await.unwrap_or(Value::Null));
        // geoclue hides no page: this page's caps (cached, or asked now on a cold start) decide its row
        let caps = common::caps_now(&p).await;
        if !p.alive() {
            return Content::new();
        }
        let available = st["available"] == true;
        let ntp_on = st["ntp"] == true;
        let mut out: Content = vec![clock_hero(&p, st["zoneLabel"].as_str(), None).upcast()];
        if !available {
            out.push(common::notice(t("De systeemklok kan hier niet worden ingesteld.", "The system clock can’t be set here.")).widget().clone().upcast());
        }
        let main = SettingsGroup::new("", "");

        // 1. Tijd automatisch instellen
        let ntp_card = SettingsCard::new("clock", t("Tijd automatisch instellen", "Set time automatically"), "");
        let ntp = ToggleSwitch::new(ntp_on);
        ntp.switch().set_widget_name("st-datetime-ntp");
        ntp.set_sensitive(available && st["canNtp"] == true);
        {
            let (pp, ntp2) = (p.clone(), ntp.clone());
            ntp.connect_toggled(move |on| {
                let (wait, ntp3) = (pp.run("machine.time.setNtp", json!({ "on": on })), ntp2.clone());
                pp.spawn(async move {
                    if wait.await.is_none() {
                        ntp3.set_active(!on);
                    }
                });
            });
        }
        ntp_card.set_control(ntp.widget());
        main.add(ntp_card.widget());

        // 2. Tijdzone automatisch instellen (GNOME shows it where geoclue is; it needs location services)
        let located = |gs: &Gs| !gs.available(&LOCATION) || gs.value(&LOCATION) == true;
        if gs.available(&AUTO_TZ) && caps["geoclue"] != false {
            let card = SettingsCard::new("globe", t("Tijdzone automatisch instellen", "Set time zone automatically"), "");
            let sw = gs.toggle(&AUTO_TZ, false);
            let apply = {
                let (gs, sw, card) = (gs.clone(), sw.clone(), card.clone());
                move || {
                    let ok = located(&gs);
                    sw.set_sensitive(ok && gs.writable(&AUTO_TZ));
                    card.set_description(if ok { "" } else { t("Hiervoor moeten locatievoorzieningen zijn ingeschakeld.", "Location services need to be turned on for this.") });
                }
            };
            apply();
            gs.follow(&LOCATION, move |_| apply());
            card.set_control(sw.widget());
            main.add(card.widget());
        }

        // 3. De datum en tijd handmatig instellen
        // glossary-ok: Datum en tijd
        let manual = SettingsCard::new("calendar", t("De datum en tijd handmatig instellen", "Set the date and time manually"), "");
        let change = c::button(t("Wijzigen", "Change"), Variant::Standard);
        change.set_widget_name("st-datetime-manual");
        change.set_sensitive(available && !ntp_on);
        {
            let pp = p.clone();
            change.connect_clicked(move |_| manual_dialog(&pp));
        }
        manual.set_control(&change);
        main.add(manual.widget());

        // 4. Tijdzone
        let current = st["timezone"].as_str().unwrap_or("").to_owned();
        let zone_card = SettingsCard::new("globe", t("Tijdzone", "Time zone"), "");
        let zone = Dropdown::new(zone_options(&zones, &current, st["zoneLabel"].as_str().unwrap_or("")), Some(&current));
        zone.widget().set_widget_name("st-datetime-timezone");
        let zone_ok = move |gs: &Gs| zone_sensitive(available, gs.value(&AUTO_TZ) == true, located(gs));
        zone.widget().set_sensitive(zone_ok(&gs));
        {
            let (pp, zone2, before) = (p.clone(), zone.clone(), Rc::new(RefCell::new(current.clone())));
            zone.connect_changed(move |v| {
                let (wait, zone3, before2, v2) = (pp.run("machine.time.setTimezone", json!({ "zone": v })), zone2.clone(), before.clone(), v.to_owned());
                pp.spawn(async move {
                    if wait.await.is_some() {
                        *before2.borrow_mut() = v2;
                    } else {
                        zone3.set_selected(Some(before2.borrow().as_str()));
                    }
                });
            });
        }
        // both keys move it: the automatic time zone, and location services turned on or off elsewhere
        for k in [&AUTO_TZ, &LOCATION] {
            let (zone2, gs2) = (zone.clone(), gs.clone());
            gs.follow(k, move |_| zone2.widget().set_sensitive(zone_ok(&gs2)));
        }
        zone_card.set_control(&common::sized(zone.widget(), 340));
        main.add(zone_card.widget());

        // 5. Automatisch aanpassen aan zomertijd: tzdata always does it (shown on, amendment 16)
        let dst = SettingsCard::new("clock", t("Automatisch aanpassen aan zomertijd", "Adjust for daylight saving time automatically"), "");
        let dst_sw = ToggleSwitch::new(true);
        dst_sw.switch().set_widget_name("st-datetime-dst");
        dst_sw.set_sensitive(false);
        dst.set_control(dst_sw.widget());
        main.add(dst.widget());

        // 6. the system tray's clock (GNOME's clock keys, mirrored to WinPoint's tray)
        for (k, icon, nl, en) in [
            (&SHOW_DATE, "calendar", "Tijd en datum weergeven in het systeemvak", "Show time and date in the System tray"),
            (&SHOW_SECONDS, "timer", "Seconden weergeven in de klok in het systeemvak", "Show seconds in System tray clock"),
            (&SHOW_WEEKDAY, "calendar", "Weekdag weergeven in de klok in het systeemvak", "Show the weekday in System tray clock"),
            (&WEEK_NUMBERS, "calendar", "Weeknummers weergeven in de agenda", "Show week numbers in the calendar"),
        ] {
            if let Some(card) = gs.toggle_card(k, icon, t(nl, en), "", false) {
                main.add(card.widget());
            }
        }
        out.push(main.widget().clone().upcast());

        // Aanvullende instellingen: Nu synchroniseren
        let more = SettingsGroup::new(t("Aanvullende instellingen", "Additional settings"), "");
        let sync = SettingsCard::new("refresh", t("Nu synchroniseren", "Sync now"), &sync_lines(&st).join("\n"));
        let sync_btn = c::button(t("Nu synchroniseren", "Sync now"), Variant::Standard);
        sync_btn.set_widget_name("st-datetime-sync");
        sync_btn.set_sensitive(available && ntp_on);
        {
            let pp = p.clone();
            sync_btn.connect_clicked(move |b| {
                b.set_sensitive(false);
                let (wait, b2) = (pp.run("machine.time.sync", json!({})), b.clone());
                pp.spawn(async move {
                    let _ = wait.await;
                    b2.set_sensitive(true);
                });
            });
        }
        sync.set_control(&sync_btn);
        more.add(sync.widget());
        out.push(more.widget().clone().upcast());

        // amendment 16: the 24-hour clock and the first day of the week are Regionale notatie's
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(
            common::link_card_with(&p, "formats", None, Some(t("24-uursnotatie, eerste dag van de week en de notatie van datums", "24-hour clock, first day of week and how dates are written")), None)
                .widget(),
        );
        related.add(common::link_card(&p, "language").widget());
        out.push(related.widget().clone().upcast());
        refresh_on_what(&p, &["time"]);
        out
    })
}

// --------------------------------------------------------------------------------------------- Taal en regio

/// The "Taal van WinPoint" values (amendment 18): the display language (settings.language "system"), or nl or en.
fn winpoint_options() -> Vec<DropdownOption> {
    vec![
        DropdownOption::new("system", t("Weergavetaal gebruiken (aanbevolen)", "Use the display language (recommended)")),
        // endonyms, as Windows lists display languages (D-G1b: the Dutch entry stays "Nederlands")
        DropdownOption::new("nl", "Nederlands"),
        DropdownOption::new("en", "English (United Kingdom)"),
    ]
}

/// settings.language as the drop-down shows it: "system", "en", else "nl" (C1).
pub fn winpoint_value(setting: &Value) -> &'static str {
    match setting.as_str() {
        Some("system") => "system",
        Some("en") => "en",
        _ => "nl",
    }
}

/// The display languages to choose from: the generated locales whose language support is installed (amendment 59:
/// a language is installed first, then chosen), by their own names, and the account's current one.
pub fn display_options(locales: &Value, current: &str) -> Vec<DropdownOption> {
    let mut out: Vec<DropdownOption> = locales["locales"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| l["supported"] == true)
        .filter_map(|l| Some(DropdownOption::new(l["id"].as_str()?, l["endonym"].as_str().or(l["name"].as_str())?)))
        .collect();
    out.sort_by_key(|o| o.label.to_lowercase());
    if !out.iter().any(|o| o.value == current) {
        let label = locales["locales"].as_array().into_iter().flatten().find(|l| l["id"] == current).and_then(|l| l["endonym"].as_str()).unwrap_or(current).to_owned();
        let label = if current.is_empty() { t("Systeemstandaard", "System default").to_owned() } else { label };
        out.insert(0, DropdownOption::new(current, &label));
    }
    out
}

/// Install a language's support (PackageKit asks for the password); the page follows `machine.changed`.
fn install_language(p: &Page, code: &str) {
    let wait = p.run("machine.languages.install", json!({ "code": code }));
    p.spawn(async move {
        let _ = wait.await;
    });
}

/// "Een taal kiezen om te installeren" (Windows' Een taal toevoegen): the languages whose support is not installed,
/// with a search box.
fn add_language(p: &Page, languages: &[Value]) {
    let mut list: Vec<Value> = languages.iter().filter(|l| l["installed"] != true).cloned().collect();
    list.sort_by_key(|l| l["name"].as_str().unwrap_or("").to_lowercase());
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let search = SearchBox::new(Some(t("Een taalnaam typen", "Type a language name")));
    body.append(search.widget());
    let rows = gtk::ListBox::new();
    rows.add_css_class("w11-list");
    rows.add_css_class("st-language-list");
    rows.set_selection_mode(gtk::SelectionMode::Single);
    for l in &list {
        let row = gtk::ListBoxRow::new();
        row.set_widget_name(&format!("st-language-add-{}", l["code"].as_str().unwrap_or("")));
        let name = l["name"].as_str().unwrap_or("");
        let own = l["endonym"].as_str().unwrap_or("");
        let text = if own.is_empty() || own == name { name.to_owned() } else { format!("{name} ({own})") };
        row.set_child(Some(&c::label(&text, "w11-body", false)));
        rows.append(&row);
    }
    let scroll = c::scrolled(&rows);
    scroll.set_min_content_height(320);
    scroll.set_max_content_height(320);
    body.append(&scroll);
    let names: Rc<Vec<String>> = Rc::new(list.iter().map(|l| format!("{} {}", l["name"].as_str().unwrap_or(""), l["endonym"].as_str().unwrap_or("")).to_lowercase()).collect());
    {
        let (rows2, names2) = (rows.clone(), names.clone());
        let query: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let q2 = query.clone();
        rows.set_filter_func(move |row| names2.get(row.index().max(0) as usize).is_none_or(|n| n.contains(q2.borrow().as_str())));
        search.connect_changed(move |text| {
            *query.borrow_mut() = text.trim().to_lowercase();
            rows2.invalidate_filter();
        });
    }
    let chosen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let codes: Vec<String> = list.iter().map(|l| l["code"].as_str().unwrap_or("").to_owned()).collect();
    let pp = p.clone();
    let c2 = chosen.clone();
    let dialog = Dialog::new(t("Een taal kiezen om te installeren", "Choose a language to install")).body(&body).primary(t("Installeren", "Install")).width(480).on_primary(move |_| {
        if let Some(code) = c2.borrow().clone() {
            install_language(&pp, &code);
            return true;
        }
        false
    });
    {
        let (d, chosen) = (dialog.clone(), chosen.clone());
        rows.connect_row_selected(move |_, row| {
            *chosen.borrow_mut() = row.and_then(|r| codes.get(r.index().max(0) as usize).cloned());
            d.set_primary_enabled(chosen.borrow().is_some());
        });
    }
    dialog.show(&p.anchor(), |_| {});
    dialog.set_primary_enabled(false);
}

fn language(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [REGION, WEEK_START, CLOCK_FORMAT];
        let (me, locales, langs) = (p.call("users.self", json!({})), p.call("machine.locales", json!({})), p.call("machine.languages", json!({})));
        let gs = Gs::load(&p, &keys).await;
        let (me, locales, langs) = (me.await.unwrap_or(Value::Null), locales.await.unwrap_or(Value::Null), langs.await.unwrap_or(Value::Null));
        let region = gs.value(&REGION).as_str().unwrap_or("").to_owned();
        let effective = formats::effective_region(&region, &me, &locales);
        let preview = p.get("machine.locales", json!({ "preview": effective })).await.map(|v| v["preview"].clone()).unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let account = me["language"].as_str().unwrap_or("").to_owned();
        let session_language = locales["session"]["language"].as_str().unwrap_or("");
        let session_formats = locales["session"]["formats"].as_str().unwrap_or("");
        if !account.is_empty() && !session_language.is_empty() && account != session_language {
            // glossary-ok: Afmelden
            out.push(formats::signout_banner(&p, t("WinPoint moet je afmelden om je nieuwe weergavetaal toe te passen.", "WinPoint needs to sign you out to apply your new display language.")));
        } else if !session_formats.is_empty() && effective != session_formats {
            out.push(formats::signout_banner(&p, t("Je nieuwe regionale notatie wordt toegepast wanneer je je opnieuw aanmeldt.", "Your new regional format applies when you sign in again.")));
        }
        let display = if account.is_empty() { session_language.to_owned() } else { account.clone() };

        // Taal: Weergavetaal (the system's, amendment 18) and Taal van WinPoint
        let lang = SettingsGroup::new(t("Taal", "Language"), "");
        let display_card = SettingsCard::new("language", t("Weergavetaal", "Display language"), t("Ubuntu en je apps worden in deze taal weergegeven", "Ubuntu and your apps will appear in this language"));
        let dd = Dropdown::new(display_options(&locales, &account), Some(&account));
        dd.widget().set_widget_name("st-language-display");
        dd.widget().set_sensitive(me["available"] == true);
        {
            let (pp, dd2) = (p.clone(), dd.clone());
            dd.connect_changed(move |v| {
                let (wait, dd3, p2, before) = (pp.run("users.setLanguage", json!({ "language": v })), dd2.clone(), pp.clone(), account.clone());
                pp.spawn(async move {
                    if wait.await.is_some() {
                        p2.refresh();
                    } else {
                        dd3.set_selected(Some(&before));
                    }
                });
            });
        }
        display_card.set_control(&common::sized(dd.widget(), 280));
        lang.add(display_card.widget());
        let wp_card = SettingsCard::new(
            "language",
            t("Taal van WinPoint", "WinPoint language"),
            t("Start, de taakbalk, Instellingen en de andere onderdelen van WinPoint (in het Nederlands of het Engels)", "Start, the taskbar, Settings and the other parts of WinPoint (in Dutch or English)"),
        );
        let wp = Dropdown::new(winpoint_options(), Some(winpoint_value(&p.setting("language"))));
        // the harness's h.set_language(.., via='dropdown') opens this one by name (tests/gnome/lib/scenario.py)
        wp.widget().set_widget_name("language-display");
        {
            let (pp, wp2) = (p.clone(), wp.clone());
            wp.connect_changed(move |v| {
                let wp3 = wp2.clone();
                common::set_setting(&pp, "language", json!(v), move |ok| {
                    if !ok {
                        wp3.set_selected(Some(winpoint_value(&kit::ctx().setting("language"))));
                    }
                });
            });
        }
        {
            let wp = wp.clone();
            p.on_settings(move |s, _| wp.set_selected(Some(winpoint_value(&s["language"]))));
        }
        wp_card.set_control(&common::sized(wp.widget(), 280));
        lang.add(wp_card.widget());
        out.push(lang.widget().clone().upcast());

        // Voorkeurstalen with Een taal toevoegen (amendment 59)
        let list: Vec<Value> = langs["languages"].as_array().cloned().unwrap_or_default();
        let installing = langs["installing"].as_str().map(str::to_owned);
        let preferred = SettingsGroup::new(t("Voorkeurstalen", "Preferred languages"), "");
        let add = SettingsCard::new("plus", t("Een taal toevoegen", "Add a language"), t("De taalondersteuning installeren: vertalingen, woordenboeken en lettertypen", "Install the language support: translations, dictionaries and fonts"));
        let add_btn = admin_button(t("Een taal toevoegen", "Add a language"), Variant::Standard);
        add_btn.set_widget_name("st-language-add");
        add_btn.set_sensitive(langs["canInstall"] == true && installing.is_none());
        {
            let (pp, list2) = (p.clone(), list.clone());
            add_btn.connect_clicked(move |_| add_language(&pp, &list2));
        }
        add.set_control(&add_btn);
        preferred.add(add.widget());
        let progress = SettingsCard::new("download", "", "");
        progress.widget().set_widget_name("st-language-progress");
        progress.widget().set_visible(false);
        let name_of = {
            let list = list.clone();
            move |code: &str| list.iter().find(|l| l["code"] == code).and_then(|l| l["name"].as_str()).unwrap_or(code).to_owned()
        };
        let show_progress = {
            let (progress, name_of) = (progress.clone(), name_of.clone());
            move |code: &str, percent: Option<u64>| {
                progress.set_title(&common::fill(t("{name} installeren", "Installing {name}"), &[("name", &name_of(code))]));
                progress.set_description(&match percent {
                    Some(n) => common::fill(t("{n}% voltooid", "{n}% complete"), &[("n", &n.to_string())]),
                    None => t("Even geduld…", "Just a moment…").to_owned(),
                });
                progress.widget().set_visible(true);
            }
        };
        if let Some(code) = &installing {
            show_progress(code, None);
        }
        preferred.add(progress.widget());
        for l in list.iter().filter(|l| l["installed"] == true) {
            let code = l["code"].as_str().unwrap_or("").to_owned();
            let complete = l["complete"] != false;
            let card = SettingsCard::new(
                "language",
                l["endonym"].as_str().or(l["name"].as_str()).unwrap_or(&code),
                if complete { t("Taalpakket geïnstalleerd", "Language pack installed") } else { t("De taalondersteuning is niet volledig geïnstalleerd", "The language support isn’t completely installed") },
            );
            if let Some(b) = card.button() {
                b.set_widget_name(&format!("st-language-{code}"));
            }
            if !complete {
                let fix = admin_button(t("Installeren", "Install"), Variant::Standard);
                fix.set_widget_name(&format!("st-language-install-{code}"));
                fix.set_sensitive(langs["canInstall"] == true && installing.is_none());
                let (pp, code2) = (p.clone(), code.clone());
                fix.connect_clicked(move |_| install_language(&pp, &code2));
                card.add_control(&fix);
            }
            let more = c::icon_button("more", t("Meer opties", "More options"), 32);
            {
                let pp = p.clone();
                more.connect_clicked(move |b| {
                    let p2 = pp.clone();
                    c::show_menu_for(b, vec![MenuItem::new(t("Taalopties", "Language options"), move || p2.go("language-options")).glyph("keyboard")]);
                });
            }
            card.add_control(&more);
            preferred.add(card.widget());
        }
        {
            let (pp, show) = (p.clone(), show_progress.clone());
            p.on("machine.changed", move |d| {
                if d["what"] != "languages" {
                    return;
                }
                let code = d["code"].as_str().unwrap_or("");
                if d["done"] == true {
                    if d["ok"] == false
                        && let Some(e) = d["error"].as_str()
                    {
                        common::show_error_text(&pp.anchor(), e, Some(t("De taal kan niet worden geïnstalleerd", "The language can’t be installed")));
                    }
                    pp.refresh();
                } else {
                    show(code, d["percent"].as_u64());
                }
            });
        }
        out.push(preferred.widget().clone().upcast());

        // Regio: the regional format inline (amendment 17), Notaties wijzigen opens Regionale notatie
        if gs.available(&REGION) {
            let regio = SettingsGroup::new(t("Regio", "Region"), "");
            let card = SettingsExpander::new("globe", t("Regionale notatie", "Regional format"), t("Apps tonen datums, tijden en getallen in deze notatie", "Apps show dates, times and numbers in this format"));
            card.header().set_control(&common::sized(formats::region_dropdown(&p, &gs, &locales, &display).widget(), 280));
            card.header_button().set_widget_name("st-language-region-samples");
            let h24 = gs.value(&CLOCK_FORMAT) != "12h";
            let mut rows: Vec<(String, String)> = vec![
                (t("Agenda", "Calendar").into(), t("Gregoriaanse kalender", "Gregorian calendar").into()),
                (t("Eerste dag van de week", "First day of week").into(), formats::week_start_text(&gs.value(&WEEK_START))),
            ];
            for (label, value) in [
                (t("Korte datum", "Short date"), preview["date"].as_str().unwrap_or("").to_owned()),
                (t("Korte tijd", "Short time"), formats::time_sample(h24, false).to_owned()),
                (t("Lange tijd", "Long time"), formats::time_sample(h24, true).to_owned()),
                (t("Getallen", "Numbers"), preview["number"].as_str().unwrap_or("").to_owned()),
            ] {
                if !value.is_empty() {
                    rows.push((label.into(), value));
                }
            }
            card.add(&formats::spec_rows(&rows));
            let change = c::button(t("Notaties wijzigen", "Change formats"), Variant::Standard);
            change.set_widget_name("st-language-change-formats");
            change.set_halign(gtk::Align::Start);
            {
                let pp = p.clone();
                change.connect_clicked(move |_| pp.go("formats"));
            }
            card.add(&change);
            regio.add(card.widget());
            out.push(regio.widget().clone().upcast());
        }

        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "language-admin").widget());
        related.add(common::link_card(&p, "typing").widget());
        out.push(related.widget().clone().upcast());
        super::accounts::refresh_on(&p, "users.changed");
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_manual_time_is_the_local_time_in_microseconds() {
        assert_eq!(hours_minutes(14.5), (14, 30));
        assert_eq!(hours_minutes(23.9999), (0, 0), "rounds to midnight");
        assert_eq!(hours_minutes(7.0 + 59.6 / 60.0), (8, 0));
        let usec = local_usec((2026, 11, 16), 14.0 + 35.0 / 60.0).unwrap();
        let back = glib::DateTime::from_unix_local(usec / 1_000_000).unwrap();
        assert_eq!((back.year(), back.month(), back.day_of_month(), back.hour(), back.minute()), (2026, 11, 16, 14, 35));
        assert!(local_usec((2026, 2, 30), 12.0).is_none(), "no 30 February");
    }

    #[test]
    fn the_winpoint_language_takes_system_and_reads_c1_otherwise() {
        assert_eq!(winpoint_value(&json!("system")), "system");
        assert_eq!(winpoint_value(&json!("en")), "en");
        assert_eq!(winpoint_value(&json!("de")), "nl");
        assert_eq!(winpoint_value(&Value::Null), "nl");
        let values: Vec<String> = winpoint_options().into_iter().map(|o| o.value).collect();
        assert_eq!(values, ["system", "nl", "en"]);
    }

    #[test]
    fn only_installed_languages_are_display_languages() {
        let locales = json!({"locales": [
            {"id": "nl_NL.UTF-8", "name": "Dutch (Netherlands)", "endonym": "Nederlands (Nederland)", "supported": false},
            {"id": "en_US.UTF-8", "name": "English (United States)", "endonym": "English (United States)", "supported": true},
            {"id": "en_GB.UTF-8", "name": "English (United Kingdom)", "endonym": "English (United Kingdom)", "supported": true}]});
        let values: Vec<String> = display_options(&locales, "en_GB.UTF-8").into_iter().map(|o| o.value).collect();
        assert_eq!(values, ["en_GB.UTF-8", "en_US.UTF-8"], "by their own names, nl_NL not before its support is installed");
        let with_current = display_options(&locales, "nl_NL.UTF-8");
        assert_eq!((with_current[0].value.as_str(), with_current[0].label.as_str()), ("nl_NL.UTF-8", "Nederlands (Nederland)"), "the current one stays");
    }

    #[test]
    fn the_time_zone_can_be_chosen_unless_location_sets_it() {
        assert!(zone_sensitive(true, false, true));
        assert!(!zone_sensitive(true, true, true), "the automatic time zone with location services sets it");
        assert!(zone_sensitive(true, true, false), "with location services off GNOME lets you pick one");
        assert!(!zone_sensitive(false, false, false), "no timedated: nothing to set");
    }

    #[test]
    fn sync_now_says_what_this_computer_knows() {
        let lines = sync_lines(&json!({"synced": true, "server": "1.ntp.ubuntu.com", "lastSync": null}));
        assert_eq!(lines.len(), 2);
        assert!(lines[1].contains("1.ntp.ubuntu.com"));
        assert_eq!(sync_lines(&json!({"synced": false, "server": null})).len(), 1, "no server: no line for it");
        let options = zone_options(&json!({"zones": [{"id": "UTC", "label": "(UTC) UTC"}]}), "Europe/Amsterdam", "(UTC+01:00) Amsterdam");
        assert_eq!(options.iter().map(|o| o.value.as_str()).collect::<Vec<_>>(), ["Europe/Amsterdam", "UTC"]);
    }
}
