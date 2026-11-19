//! Instellingen › Systeem › Schermtijd (batch D, notes/claude/settings-parity.md §2): GNOME Settings 50's Wellbeing
//! panel in Windows 11's card style (Windows itself has screen time only in Microsoft Family; the page sits beside
//! Focus as the plan says):
//!
//! * Schermtijd: GNOME's statistics row (host wellbeing.history, gnome-shell's own record): the selected day with its
//!   screen time and the average of that weekday, the week with its total and the average week, a bar per day of
//!   the week (a click selects the day; the daily limit as a line), Vorige week / Volgende week; "Schermtijd is niet
//!   beschikbaar" before there is a record, "Vastleggen van schermtijd is uitgeschakeld" with Inschakelen while
//!   history-enabled is off; Schermtijd vastleggen (history-enabled).
//! * Schermlimieten: Limiet voor schermtijd (daily-limit-enabled) with Dagelijkse limiet in hours and minutes
//!   (daily-limit-seconds, at least a minute as in GNOME) and Grijswaarden (grayscale); managed by Ouderlijk toezicht
//!   when malcontent limits this user (GNOME's banner).
//! * Pauzeherinneringen: Herinneringen voor je ogen and Herinneringen om te bewegen (selected-breaks), the Schema voor
//!   bewegingspauzes (the movement break's duration-seconds and interval-seconds, GNOME's four schedules), Geluiden
//!   (play-sound of both breaks).
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsGroup, Severity, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("screentime", screentime)];
pub const CSS: &str = "
.st-screentime-chart { padding: 16px 18px; }
.st-screentime-big { font-size: 20px; font-weight: 600; }
.st-screentime-day { min-width: 34px; padding: 2px 0; }
";

const LIMITS: &str = "org.gnome.desktop.screen-time-limits";
const HISTORY: GKey = gs::key(LIMITS, "history-enabled");
const LIMIT_ON: GKey = gs::key(LIMITS, "daily-limit-enabled");
const LIMIT: GKey = gs::key(LIMITS, "daily-limit-seconds");
const GRAY: GKey = gs::key(LIMITS, "grayscale");
const BREAKS: GKey = gs::key("org.gnome.desktop.break-reminders", "selected-breaks");
const MOVEMENT: &str = "org.gnome.desktop.break-reminders.movement";
const DURATION: GKey = gs::key(MOVEMENT, "duration-seconds");
const INTERVAL: GKey = gs::key(MOVEMENT, "interval-seconds");
const MOVE_SOUND: GKey = gs::key(MOVEMENT, "play-sound");
const EYE_SOUND: GKey = gs::key("org.gnome.desktop.break-reminders.eyesight", "play-sound");
fn keys() -> Vec<GKey> {
    vec![HISTORY, LIMIT_ON, LIMIT, GRAY, BREAKS, DURATION, INTERVAL, MOVE_SOUND, EYE_SOUND]
}

/// GNOME Settings 50's movement break schedules (cc-break-schedule-row: duration, interval in seconds).
const SCHEDULES: &[(u64, u64)] = &[(60, 1200), (120, 1200), (180, 1800), (300, 1800)];

// ------------------------------------------------------------------------------------------ the statistics

/// One day of the host's history.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Day {
    pub date: String,
    pub weekday: i32,
    pub seconds: i64,
}

pub(crate) fn days_of(history: &Value) -> Vec<Day> {
    history["days"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| {
            Some(Day { date: d["date"].as_str()?.to_owned(), weekday: d["weekday"].as_i64()? as i32, seconds: d["seconds"].as_i64().unwrap_or(0) })
        })
        .collect()
}

/// The first day of the week as an ISO weekday (1 = Monday .. 7 = Sunday): time.firstDayOfWeek (0 Sunday .. 6
/// Saturday), or Monday for "Standaard (regio)" (nl-NL and en-GB start on Monday).
pub(crate) fn week_start(setting: i64) -> i32 {
    match setting {
        0 => 7,
        1..=6 => setting as i32,
        _ => 1,
    }
}

/// The seven days of week `back` (0 = this week, 1 = the week before, ...) as indices into `days` (None: before the
/// first record or after today). `days` ends today.
pub(crate) fn week(days: &[Day], start: i32, back: usize) -> [Option<usize>; 7] {
    let mut out = [None; 7];
    let Some(today) = days.last() else { return out };
    let offset = ((today.weekday - start + 7) % 7) as i64;
    let last = days.len() as i64 - 1;
    let first = last - offset - 7 * back as i64;
    for (i, slot) in out.iter_mut().enumerate() {
        let index = first + i as i64;
        if (0..=last).contains(&index) {
            *slot = Some(index as usize);
        }
    }
    out
}

/// How many weeks back the history reaches.
pub(crate) fn weeks_back(days: &[Day], start: i32) -> usize {
    let Some(today) = days.last() else { return 0 };
    let offset = ((today.weekday - start + 7) % 7) as usize;
    days.len().saturating_sub(offset + 1).div_ceil(7)
}

/// GNOME's "Average <weekday>": the mean of that weekday over the history, the ongoing today left out unless it is
/// the only one.
pub(crate) fn weekday_average(days: &[Day], weekday: i32) -> Option<i64> {
    let last = days.len().checked_sub(1)?;
    let mut same: Vec<i64> = days.iter().enumerate().filter(|(i, d)| d.weekday == weekday && *i != last).map(|(_, d)| d.seconds).collect();
    if same.is_empty() {
        same = days.iter().filter(|d| d.weekday == weekday).map(|d| d.seconds).collect();
    }
    (!same.is_empty()).then(|| same.iter().sum::<i64>() / same.len() as i64)
}

/// GNOME's "Average Week": the mean weekly total of the weeks with a record.
pub(crate) fn week_average(days: &[Day], start: i32) -> Option<i64> {
    let totals: Vec<i64> = (0..=weeks_back(days, start)).map(|b| week(days, start, b).iter().flatten().map(|&i| days[i].seconds).sum()).collect();
    (!totals.is_empty() && days.iter().any(|d| d.seconds > 0)).then(|| totals.iter().sum::<i64>() / totals.len() as i64)
}

/// "3 u 25 min" / "3 h 25 min" (kit's short duration, whole minutes).
fn span(seconds: i64) -> String {
    let rounded = (seconds.max(0) / 60) * 60;
    if rounded == 0 { kit::format::duration(0.0).replace("0 s", "0 min") } else { kit::format::duration(rounded as f64) }
}

/// "16 november" / "16 November" of a YYYY-MM-DD date.
fn day_label(date: &str) -> String {
    let parts: Vec<i32> = date.split('-').filter_map(|p| p.parse().ok()).collect();
    match parts.as_slice() {
        [_, m, d] => format!("{d} {}", kit::format::month_name(*m, false)),
        _ => date.to_owned(),
    }
}

/// The page's state.
struct Shot {
    p: Page,
    gs: Gs,
    history: RefCell<Value>,
    back: Cell<usize>,
    selected: Cell<Option<usize>>,
    slot: gtk::Box,
}

fn start_of_week() -> i32 {
    week_start(common::setting_f64("time.firstDayOfWeek").map(|n| n as i64).unwrap_or(-1))
}

/// One bar: its share of the scale, the limit line, the grid; selected bars in the accent colour.
fn bar(value: Option<i64>, max: f64, limit: Option<f64>, selected: bool) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(28);
    area.set_content_height(140);
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        let grid = kit::chart::resolve("chart-grid");
        cr.set_source_rgba(grid.0, grid.1, grid.2, grid.3);
        cr.set_line_width(1.0);
        for f in [0.0, 0.5, 1.0] {
            let y = (h - 1.0 - f * (h - 2.0)).round() + 0.5;
            cr.move_to(0.0, y);
            cr.line_to(w, y);
        }
        let _ = cr.stroke();
        if let Some(v) = value.filter(|v| *v > 0) {
            let bh = (v as f64 / max).clamp(0.0, 1.0) * (h - 2.0);
            let (x, bw) = (w * 0.2, w * 0.6);
            let y = h - 1.0 - bh;
            let r = 3.0f64.min(bh / 2.0).min(bw / 2.0);
            let a = kit::chart::resolve("accent");
            cr.set_source_rgba(a.0, a.1, a.2, if selected { a.3 } else { a.3 * 0.55 });
            cr.new_sub_path();
            cr.arc(x + bw - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
            cr.line_to(x + bw, h - 1.0);
            cr.line_to(x, h - 1.0);
            cr.arc(x + r, y + r, r, std::f64::consts::PI, 3.0 * std::f64::consts::FRAC_PI_2);
            cr.close_path();
            let _ = cr.fill();
        }
        if let Some(l) = limit {
            let y = (h - 1.0 - (l / max).clamp(0.0, 1.0) * (h - 2.0)).round() + 0.5;
            let c = kit::chart::resolve("sys-caution");
            cr.set_source_rgba(c.0, c.1, c.2, c.3);
            cr.set_dash(&[4.0, 3.0], 0.0);
            cr.move_to(0.0, y);
            cr.line_to(w, y);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);
        }
    });
    kit::theme::track(&area);
    area
}

/// The statistics card (GNOME's CcScreenTimeStatisticsRow).
fn render_chart(s: &Rc<Shot>) {
    let history = s.history.borrow().clone();
    let days = days_of(&history);
    let recording = s.gs.value(&HISTORY) != false;
    let card = gtk::Box::new(gtk::Orientation::Vertical, 14);
    card.add_css_class("w11-settings-card");
    card.add_css_class("st-screentime-chart");
    card.set_widget_name("st-screentime-chart");
    if !recording {
        let bar = common::notice_full(
            Severity::Info,
            t("Vastleggen van schermtijd is uitgeschakeld", "Screen time recording is off"),
            t("Schakel vastleggen in om de geschiedenis van je schermgebruik te bekijken.", "Turn on recording to see your screen usage history."),
        );
        bar.widget().set_widget_name("st-screentime-disabled");
        if s.gs.writable(&HISTORY) {
            let on = c::button(t("Inschakelen", "Turn on"), c::Variant::Standard);
            on.set_widget_name("st-screentime-enable");
            let s2 = s.clone();
            on.connect_clicked(move |_| s2.gs.set(&HISTORY, json!(true), || {}));
            bar.set_action_widget(&on);
        }
        common::set_slot(&s.slot, &[bar.widget().clone().upcast()]);
        return;
    }
    if history["available"] == false || days.iter().all(|d| d.seconds == 0) {
        let bar = common::notice_full(
            Severity::Info,
            t("Schermtijd is niet beschikbaar", "Screen time isn’t available"),
            t("Informatie over schermtijd wordt hier weergegeven nadat je de pc een tijdje hebt gebruikt.", "Screen time information will appear here after a period of usage."),
        );
        bar.widget().set_widget_name("st-screentime-unavailable");
        common::set_slot(&s.slot, &[bar.widget().clone().upcast()]);
        return;
    }
    let start = start_of_week();
    let back = s.back.get().min(weeks_back(&days, start));
    let wk = week(&days, start, back);
    let selected = s.selected.get().filter(|i| wk.contains(&Some(*i))).or_else(|| wk.iter().rev().flatten().next().copied());
    let total: i64 = wk.iter().flatten().map(|&i| days[i].seconds).sum();
    let limit = (s.gs.value(&LIMIT_ON) == true).then(|| s.gs.value(&LIMIT).as_f64()).flatten();
    let max_value = wk.iter().flatten().map(|&i| days[i].seconds as f64).fold(0.0, f64::max).max(limit.unwrap_or(0.0));
    // the scale: whole hours, at least one
    let max = ((max_value / 3600.0).ceil().max(1.0)) * 3600.0;

    // the figures: the selected day and the week
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 24);
    let figures = |title: &str, value: &str, sub: &str| {
        let col = gtk::Box::new(gtk::Orientation::Vertical, 2);
        col.set_hexpand(true);
        col.append(&c::label(title, "w11-text-2", true));
        col.append(&c::label(value, "st-screentime-big", false));
        col.append(&c::label(sub, "w11-text-2", true));
        col
    };
    if let Some(i) = selected {
        let d = &days[i];
        let title = if i + 1 == days.len() { t("Vandaag", "Today").to_owned() } else { format!("{} {}", kit::format::weekday_name(d.weekday, false), day_label(&d.date)) };
        let avg = weekday_average(&days, d.weekday).map(span).unwrap_or_else(|| "–".into());
        let sub = common::fill(t("Gemiddelde {day}: {time}", "Average {day}: {time}"), &[("day", kit::format::weekday_name(d.weekday, false)), ("time", &avg)]);
        head.append(&figures(&title, &span(d.seconds), &sub));
    }
    let week_title = if back == 0 {
        t("Deze week", "This week").to_owned()
    } else {
        let first = wk.iter().flatten().next().map(|&i| day_label(&days[i].date)).unwrap_or_default();
        common::fill(t("Week van {date}", "Week of {date}"), &[("date", &first)])
    };
    let avg_week = week_average(&days, start).map(span).unwrap_or_else(|| "–".into());
    head.append(&figures(&week_title, &span(total), &common::fill(t("Gemiddelde week: {time}", "Average week: {time}"), &[("time", &avg_week)])));
    card.append(&head);

    // the bars, the hour scale on the left
    let chart = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let scale = gtk::Box::new(gtk::Orientation::Vertical, 0);
    scale.set_size_request(-1, 140);
    for (i, f) in [1.0, 0.5, 0.0].iter().enumerate() {
        let l = c::label(&span((max * f) as i64), "w11-text-3", false);
        l.set_xalign(1.0);
        l.set_vexpand(true);
        l.set_valign(match i {
            0 => gtk::Align::Start,
            1 => gtk::Align::Center,
            _ => gtk::Align::End,
        });
        scale.append(&l);
    }
    let scale_col = gtk::Box::new(gtk::Orientation::Vertical, 4);
    scale_col.append(&scale);
    chart.append(&scale_col);
    for (k, slot) in wk.iter().enumerate() {
        let weekday = (start - 1 + k as i32) % 7 + 1;
        let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
        col.append(&bar(slot.map(|i| days[i].seconds), max, limit, *slot == selected && slot.is_some()));
        col.append(&c::label(kit::format::weekday_name(weekday, true), "w11-text-2", false));
        let b = gtk::Button::new();
        b.add_css_class("w11-button");
        b.add_css_class("w11-subtle");
        b.add_css_class("st-screentime-day");
        b.set_child(Some(&col));
        b.set_widget_name(&format!("st-screentime-day-{k}"));
        b.set_hexpand(true);
        if let Some(i) = *slot {
            b.set_tooltip_text(Some(&format!("{} {}: {}", kit::format::weekday_name(weekday, false), day_label(&days[i].date), span(days[i].seconds))));
            let s2 = s.clone();
            b.connect_clicked(move |_| {
                s2.selected.set(Some(i));
                render_chart(&s2);
            });
        } else {
            b.set_sensitive(false);
        }
        chart.append(&b);
    }
    card.append(&chart);

    // Vorige week / Volgende week
    let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    nav.set_halign(gtk::Align::End);
    for (name, glyph, tip, older) in [
        ("st-screentime-previous-week", "chevron-left", t("Vorige week", "Previous week"), true),
        ("st-screentime-next-week", "chevron-right", t("Volgende week", "Next week"), false),
    ] {
        let b = c::icon_button(glyph, tip, 14);
        b.set_widget_name(name);
        b.set_sensitive(if older { back < weeks_back(&days, start) } else { back > 0 });
        let s2 = s.clone();
        b.connect_clicked(move |_| {
            let now = s2.back.get();
            s2.back.set(if older { now + 1 } else { now.saturating_sub(1) });
            s2.selected.set(None);
            render_chart(&s2);
        });
        nav.append(&b);
    }
    card.append(&nav);
    common::set_slot(&s.slot, &[card.upcast()]);
}

// ------------------------------------------------------------------------------------------- the controls

/// selected-breaks with `name` added or taken out, the other entries kept.
pub(crate) fn with_break(current: &Value, name: &str, on: bool) -> Value {
    let mut list: Vec<String> = current.as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_owned)).filter(|v| v != name).collect();
    if on {
        list.push(name.to_owned());
    }
    json!(list)
}

fn has_break(gs: &Gs, name: &str) -> bool {
    gs.value(&BREAKS).as_array().is_some_and(|a| a.iter().any(|v| v == name))
}

/// A switch for one break type in selected-breaks.
fn break_toggle(p: &Page, gs: &Gs, name: &'static str) -> ToggleSwitch {
    let sw = ToggleSwitch::new(has_break(gs, name));
    sw.switch().set_widget_name(&format!("st-{}-selected-breaks-{name}", p.id()));
    if !gs.writable(&BREAKS) {
        sw.set_sensitive(false);
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (gs2, sw2) = (gs.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let (gs3, sw3) = (gs2.clone(), sw2.clone());
            gs2.set(&BREAKS, with_break(&gs2.value(&BREAKS), name, on), move || sw3.set_active(has_break(&gs3, name)));
        });
    }
    {
        let (gs2, sw2) = (gs.clone(), sw.clone());
        gs.follow(&BREAKS, move |_| sw2.set_active(has_break(&gs2, name)));
    }
    sw
}

/// "5 minuten elke 30 minuten".
fn schedule_label(duration: u64, interval: u64) -> String {
    common::fill(
        t("{duration} elke {interval}", "{duration} every {interval}"),
        &[("duration", &kit::format::duration_long(duration as f64)), ("interval", &kit::format::duration_long(interval as f64))],
    )
}

/// The schedule options: GNOME's four, and the current one when it is another.
pub(crate) fn schedule_options(current: Option<(u64, u64)>) -> Vec<(u64, u64)> {
    let mut out = SCHEDULES.to_vec();
    if let Some(c) = current
        && !out.contains(&c)
    {
        out.push(c);
        out.sort_by_key(|(d, i)| (*i, *d));
    }
    out
}

fn schedule_dropdown(p: &Page, gs: &Gs) -> Dropdown {
    let current = |gs: &Gs| Some((gs.value(&DURATION).as_u64()?, gs.value(&INTERVAL).as_u64()?));
    let options = move |gs: &Gs| schedule_options(current(gs)).into_iter().map(|(d, i)| DropdownOption::new(&format!("{d}:{i}"), &schedule_label(d, i))).collect::<Vec<_>>();
    let key = move |gs: &Gs| current(gs).map(|(d, i)| format!("{d}:{i}"));
    let dd = Dropdown::new(options(gs), key(gs).as_deref());
    dd.widget().set_widget_name(&gs::control_name(p.id(), &keys(), &INTERVAL));
    if !(gs.writable(&DURATION) && gs.writable(&INTERVAL)) {
        dd.widget().set_sensitive(false);
        dd.widget().set_tooltip_text(Some(gs::locked_text()));
    }
    let refresh = {
        let (gs2, dd2) = (gs.clone(), dd.clone());
        move || {
            dd2.set_options(options(&gs2));
            dd2.set_selected(key(&gs2).as_deref());
        }
    };
    {
        let (pp, r) = (p.clone(), refresh.clone());
        dd.connect_changed(move |choice| {
            let Some((d, i)) = choice.split_once(':').and_then(|(d, i)| Some((d.parse::<u64>().ok()?, i.parse::<u64>().ok()?))) else { return };
            let wait = pp.run_with("gsettings.set", json!({ "schema": MOVEMENT, "values": { DURATION.key: d, INTERVAL.key: i } }), None, false);
            let r2 = r.clone();
            pp.spawn(async move {
                if wait.await.is_none() {
                    r2();
                }
            });
        });
    }
    for k in [&DURATION, &INTERVAL] {
        let r = refresh.clone();
        gs.follow(k, move |_| r());
    }
    dd
}

/// Geluiden: play-sound of both breaks in one write; on while the movement break's is.
fn sounds_toggle(p: &Page, gs: &Gs) -> ToggleSwitch {
    let sw = ToggleSwitch::new(gs.value(&MOVE_SOUND) != false);
    sw.switch().set_widget_name(&gs::control_name(p.id(), &keys(), &MOVE_SOUND));
    let any_break = |gs: &Gs| has_break(gs, "eyesight") || has_break(gs, "movement");
    let writable = gs.writable(&MOVE_SOUND) && gs.writable(&EYE_SOUND);
    sw.set_sensitive(writable && any_break(gs));
    {
        let (pp, gs2, sw2) = (p.clone(), gs.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let args = json!({ "writes": [
                { "schema": MOVE_SOUND.schema, "values": { MOVE_SOUND.key: on } },
                { "schema": EYE_SOUND.schema, "values": { EYE_SOUND.key: on } },
            ]});
            let wait = pp.run_with("gsettings.set", args, None, false);
            let (gs3, sw3) = (gs2.clone(), sw2.clone());
            pp.spawn(async move {
                if wait.await.is_none() {
                    sw3.set_active(gs3.value(&MOVE_SOUND) != false);
                }
            });
        });
    }
    {
        let sw2 = sw.clone();
        gs.follow(&MOVE_SOUND, move |v| sw2.set_active(*v != false));
    }
    {
        let (gs2, sw2) = (gs.clone(), sw.clone());
        gs.follow(&BREAKS, move |_| sw2.set_sensitive(writable && any_break(&gs2)));
    }
    sw
}

/// Dagelijkse limiet: hours and minutes of daily-limit-seconds (GNOME's duration row, at least one minute).
pub(crate) fn limit_seconds(hours: f64, minutes: f64) -> u64 {
    ((hours.max(0.0) as u64) * 3600 + (minutes.max(0.0) as u64) * 60).max(60)
}

/// What Dagelijkse limiet's boxes show for daily-limit-seconds (GNOME's default 8 hours when it has none): hours and
/// minutes. The page's start, an outside change and a refused write all show the key's value through it.
pub(crate) fn limit_parts(value: &Value) -> (f64, f64) {
    let secs = value.as_u64().unwrap_or(28_800);
    ((secs / 3600) as f64, ((secs % 3600) / 60) as f64)
}

fn limit_editor(p: &Page, gs: &Gs) -> gtk::Box {
    let (h0, m0) = limit_parts(&gs.value(&LIMIT));
    let hours = c::number_box(0.0, 23.0, 1.0, h0, 0);
    let minutes = c::number_box(0.0, 59.0, 1.0, m0, 0);
    let name = gs::control_name(p.id(), &keys(), &LIMIT);
    hours.set_widget_name(&format!("{name}-hours"));
    minutes.set_widget_name(&format!("{name}-minutes"));
    // lockdown, as every bound control (gs.rs): insensitive with the lock's tooltip; limits_group's sync() only
    // changes the card around them, so the boxes stay insensitive while the key is not writable
    if !gs.writable(&LIMIT) {
        for sb in [&hours, &minutes] {
            sb.set_sensitive(false);
            if gs.available(&LIMIT) {
                sb.set_tooltip_text(Some(gs::locked_text()));
            }
        }
    }
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_widget_name(&name);
    row.append(&hours);
    row.append(&c::label(t("uur", "hr"), "w11-text-2", false));
    row.append(&minutes);
    row.append(&c::label(t("min", "min"), "w11-text-2", false));
    let silent = Rc::new(Cell::new(false));
    // the key's value in both boxes, without a write
    let show = {
        let (h2, m2, silent) = (hours.clone(), minutes.clone(), silent.clone());
        Rc::new(move |v: &Value| {
            let (h, m) = limit_parts(v);
            silent.set(true);
            h2.set_value(h);
            m2.set_value(m);
            silent.set(false);
        })
    };
    let generation = Rc::new(Cell::new(0u64));
    for sb in [&hours, &minutes] {
        let (pp, gs2, h2, m2, silent, generation, show) = (p.clone(), gs.clone(), hours.clone(), minutes.clone(), silent.clone(), generation.clone(), show.clone());
        sb.connect_value_changed(move |_| {
            if silent.get() {
                return;
            }
            // one write once the spinning stops
            let g = generation.get() + 1;
            generation.set(g);
            let (gs3, h3, m3, generation, show) = (gs2.clone(), h2.clone(), m2.clone(), generation.clone(), show.clone());
            pp.timeout(500, move || {
                if generation.get() == g {
                    // refused (locked, or the host said no): gs.set put the old value back in its cache, the boxes
                    // show it again with the error toast
                    let gs4 = gs3.clone();
                    gs3.set(&LIMIT, json!(limit_seconds(h3.value(), m3.value())), move || show(&gs4.value(&LIMIT)));
                }
            });
        });
    }
    gs.follow(&LIMIT, move |v| show(v));
    row
}

fn limits_group(p: &Page, gs: &Gs, parental: bool) -> SettingsGroup {
    let group = SettingsGroup::new(t("Schermlimieten", "Screen limits"), "");
    if parental {
        let bar = common::notice(t("Sommige instellingen worden beheerd door Ouderlijk toezicht.", "Some settings are managed by Parental Controls."));
        bar.widget().set_widget_name("st-screentime-parental");
        if common::caps()["malcontent"] == true {
            let open = c::button(t("Ouderlijk toezicht openen", "Open Parental Controls"), c::Variant::Standard);
            open.set_widget_name("st-screentime-parental-open");
            let pp = p.clone();
            open.connect_clicked(move |_| {
                let wait = pp.run_with("apps.launch", json!({ "id": "org.freedesktop.MalcontentControl.desktop" }), None, true);
                pp.spawn(async move {
                    let _ = wait.await;
                });
            });
            bar.set_action_widget(&open);
        }
        group.add(bar.widget());
    }
    // the switch on a card of its own, the limit's rows below it (a switch in an expander's header button never gets
    // the pointer's click)
    let limit = SettingsCard::new("timer", t("Limiet voor schermtijd", "Screen time limit"), t("Een dagelijkse limiet voor je schermtijd instellen", "Set a daily limit for your screen time"));
    limit.set_control(gs.toggle(&LIMIT_ON, false).widget());
    let daily = sub_row(SettingsCard::new("", t("Dagelijkse limiet", "Daily limit"), ""));
    daily.set_control(&limit_editor(p, gs));
    let gray = sub_row(SettingsCard::new("", t("Grijswaarden", "Greyscale"), t("Zwart-wit scherm bij het bereiken van de limiet", "Black and white screen when the limit is reached")));
    gray.set_control(gs.toggle(&GRAY, false).widget());
    let inner = [daily.widget().clone(), gray.widget().clone()];
    let sync = {
        let gs2 = gs.clone();
        move || {
            for w in &inner {
                w.set_sensitive(!parental && gs2.value(&LIMIT_ON) == true);
            }
        }
    };
    sync();
    {
        let s = sync.clone();
        gs.follow(&LIMIT_ON, move |_| s());
    }
    if parental {
        limit.widget().set_sensitive(false);
    }
    group.add(limit.widget());
    group.add(daily.widget());
    group.add(gray.widget());
    group
}

fn breaks_group(p: &Page, gs: &Gs) -> SettingsGroup {
    let group = SettingsGroup::new(t("Pauzeherinneringen", "Break reminders"), "");
    let eyes = SettingsCard::new("eye", t("Herinneringen voor je ogen", "Eyesight reminders"), t("Herinneringen om van het scherm weg te kijken", "Reminders to look away from the screen"));
    eyes.set_control(break_toggle(p, gs, "eyesight").widget());
    group.add(eyes.widget());
    let movement = SettingsCard::new("people", t("Herinneringen om te bewegen", "Movement reminders"), t("Herinneringen om even rond te lopen", "Reminders to move around"));
    movement.set_control(break_toggle(p, gs, "movement").widget());
    group.add(movement.widget());
    if gs.available(&DURATION) && gs.available(&INTERVAL) {
        let schedule = sub_row(SettingsCard::new("", t("Schema voor bewegingspauzes", "Movement break schedule"), ""));
        schedule.set_control(&common::sized(schedule_dropdown(p, gs).widget(), 280));
        schedule.widget().set_sensitive(has_break(gs, "movement"));
        let (gs2, w) = (gs.clone(), schedule.widget().clone());
        gs.follow(&BREAKS, move |_| w.set_sensitive(has_break(&gs2, "movement")));
        group.add(schedule.widget());
    }
    if gs.available(&MOVE_SOUND) && gs.available(&EYE_SOUND) {
        let sounds = SettingsCard::new("sound", t("Geluiden", "Sounds"), t("Een geluid afspelen wanneer een pauze eindigt", "Play a sound when a break ends"));
        sounds.set_control(sounds_toggle(p, gs).widget());
        group.add(sounds.widget());
    }
    group
}

async fn reload(s: &Rc<Shot>) {
    if let Some(v) = s.p.get("wellbeing.history", json!({})).await
        && s.p.alive()
    {
        *s.history.borrow_mut() = v;
        render_chart(s);
    }
}

fn screentime(p: Page) -> PageFuture {
    Box::pin(async move {
        let history = p.call("wellbeing.history", json!({}));
        let gs = Gs::load(&p, &keys()).await;
        let history = history.await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        if !gs.available(&HISTORY) {
            return vec![common::notice(t("Schermtijd is op deze computer niet beschikbaar.", "Screen time isn’t available on this computer.")).widget().clone().upcast()];
        }
        let s = Rc::new(Shot { p: p.clone(), gs: gs.clone(), history: RefCell::new(history.clone()), back: Cell::new(0), selected: Cell::new(None), slot: common::slot() });
        let mut out: Content = Vec::new();
        let stats = SettingsGroup::new(t("Schermtijd", "Screen time"), "");
        stats.add(&s.slot);
        if let Some(card) = gs.toggle_card(
            &HISTORY,
            "history",
            t("Schermtijd vastleggen", "Record screen time"),
            t(
                "Als je dit uitschakelt, wordt er geen schermtijd meer opgeslagen. Deze gegevens worden nooit buiten dit apparaat gedeeld.",
                "Turning this off stops screen time data from being saved. This data is never shared outside this device.",
            ),
            false,
        ) {
            stats.add(card.widget());
        }
        out.push(stats.widget().clone().upcast());
        render_chart(&s);
        for k in [&HISTORY, &LIMIT_ON, &LIMIT] {
            let s2 = s.clone();
            gs.follow(k, move |_| render_chart(&s2));
        }
        out.push(limits_group(&p, &gs, history["parental"] == true).widget().clone().upcast());
        out.push(breaks_group(&p, &gs).widget().clone().upcast());
        {
            // gnome-shell writes its record when the session changes state: read it again now and then
            let s2 = s.clone();
            p.interval(60_000, move || {
                let s3 = s2.clone();
                s2.p.spawn(async move { reload(&s3).await });
                true
            });
        }
        out
    })
}


/// A row that belongs to the row above it: its text lines up with that row's title (an empty 20 px icon), as the
/// rows inside a Windows expander do.
fn sub_row(card: SettingsCard) -> SettingsCard {
    let room = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    room.set_size_request(20, 20);
    card.set_icon_widget(&room);
    card
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(weekday_today: i32, n: usize) -> Vec<Day> {
        (0..n)
            .map(|k| {
                let back = (n - 1 - k) as i32;
                Day { date: format!("d{k}"), weekday: (weekday_today - 1 - back).rem_euclid(7) + 1, seconds: (k as i64 + 1) * 600 }
            })
            .collect()
    }

    #[test]
    fn the_week_follows_the_first_day_of_the_week() {
        // today is a Wednesday (3), ten days of history
        let d = days(3, 10);
        // Monday first: this week is Mon, Tue, Wed and four days to come
        let w = week(&d, 1, 0);
        assert_eq!(w, [Some(7), Some(8), Some(9), None, None, None, None]);
        // the week before is complete, the one before that has its last Sunday only
        assert_eq!(week(&d, 1, 1), [Some(0), Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)]);
        assert_eq!(weeks_back(&d, 1), 1);
        // Sunday first (time.firstDayOfWeek 0): Sun .. Sat
        assert_eq!(week_start(0), 7);
        assert_eq!(week_start(-1), 1, "Standaard (regio): Monday for nl-NL and en-GB");
        let w = week(&d, 7, 0);
        assert_eq!(w, [Some(6), Some(7), Some(8), Some(9), None, None, None]);
        assert_eq!(weeks_back(&d, 7), 1);
        assert_eq!(week(&[], 1, 0), [None; 7]);
    }

    #[test]
    fn averages_are_gnomes() {
        let d = days(3, 10);
        // Wednesdays: index 2 (1800 s) and today 9 (6000 s, still going): the mean leaves today out
        assert_eq!(weekday_average(&d, 3), Some(1800));
        // two weeks: (600 + .. + 4200) and (4800 + 5400 + 6000)
        assert_eq!(week_average(&d, 1), Some((16800 + 16200) / 2));
        let today_only = days(3, 1);
        assert_eq!(weekday_average(&today_only, 3), Some(600), "the only one is today");
        assert_eq!(weekday_average(&today_only, 4), None);
    }

    #[test]
    fn the_break_controls_write_gnomes_values() {
        assert_eq!(with_break(&json!(["eyesight"]), "movement", true), json!(["eyesight", "movement"]));
        assert_eq!(with_break(&json!(["eyesight", "movement"]), "eyesight", false), json!(["movement"]));
        assert_eq!(with_break(&json!(["movement"]), "movement", true), json!(["movement"]), "never twice");
        assert_eq!(with_break(&Value::Null, "eyesight", true), json!(["eyesight"]));
        assert_eq!(schedule_options(Some((300, 1800))), SCHEDULES.to_vec());
        assert_eq!(schedule_options(Some((600, 3600))).last(), Some(&(600, 3600)), "another schedule is kept");
        assert_eq!(limit_seconds(8.0, 0.0), 28_800);
        assert_eq!(limit_seconds(0.0, 0.0), 60, "at least a minute, as GNOME's duration row");
        assert_eq!(limit_seconds(1.0, 30.0), 5_400);
        let names = keys();
        assert_eq!(gs::control_name("screentime", &names, &MOVE_SOUND), "st-screentime-movement-play-sound");
        assert_eq!(gs::control_name("screentime", &names, &LIMIT), "st-screentime-daily-limit-seconds");
    }

    #[test]
    fn the_daily_limit_boxes_show_the_keys_value_again_after_a_refused_write() {
        // review D-8: 8 hours in GNOME, the user's 9 refused; the boxes show what the key holds (gs.set's cache)
        assert_eq!(limit_parts(&json!(28_800)), (8.0, 0.0));
        assert_eq!(limit_parts(&json!(5_400)), (1.0, 30.0));
        assert_eq!(limit_parts(&Value::Null), (8.0, 0.0), "GNOME's default without a value");
        for secs in [60u64, 3_600, 5_400, 28_800, 86_340] {
            let (h, m) = limit_parts(&json!(secs));
            assert_eq!(limit_seconds(h, m), secs, "the boxes write back what they show");
        }
    }
}
