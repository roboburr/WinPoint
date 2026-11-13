//! Dates without a time zone library: civil days (proleptic Gregorian, from native/shell flyouts/calendar.rs),
//! "civil minutes" (minutes since 1970-01-01 00:00 local wall time, the unit events are kept in), Dutch and
//! English names, and the formats the views use.
use gtk::glib;

pub type Ymd = (i32, u32, u32);

/// Minutes per day.
pub const DAY: i64 = 1440;

const NL_DAYS: [&str; 7] = ["zondag", "maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag"];
const EN_DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const NL_DAYS_SHORT: [&str; 7] = ["zo", "ma", "di", "wo", "do", "vr", "za"];
const EN_DAYS_SHORT: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const EN_DAYS_MINI: [&str; 7] = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
const NL_MONTHS: [&str; 12] = [
    "januari", "februari", "maart", "april", "mei", "juni", "juli", "augustus", "september", "oktober", "november",
    "december",
];
const EN_MONTHS: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
const NL_MONTHS_SHORT: [&str; 12] = ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];
const EN_MONTHS_SHORT: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Days since 1970-01-01.
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let (m, d) = (m as i64, d as i64);
    let y = if m <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> Ymd {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    ((if m <= 2 { y + 1 } else { y }) as i32, m as u32, d as u32)
}

pub fn day_number(d: Ymd) -> i64 {
    days_from_civil(d.0, d.1, d.2)
}

/// 0 = Sunday ... 6 = Saturday.
pub fn weekday(d: Ymd) -> u32 {
    (day_number(d) + 4).rem_euclid(7) as u32
}

pub fn add_days(d: Ymd, n: i64) -> Ymd {
    civil_from_days(day_number(d) + n)
}

pub fn add_months(y: i32, m: u32, delta: i32) -> (i32, u32) {
    let idx = y * 12 + (m as i32 - 1) + delta;
    (idx.div_euclid(12), idx.rem_euclid(12) as u32 + 1)
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    let (ny, nm) = add_months(y, m, 1);
    (days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)) as u32
}

/// The first day of the week holding `d`, weeks starting on `first_dow` (0 = Sunday, 1 = Monday).
pub fn week_start(d: Ymd, first_dow: u32) -> Ymd {
    let back = (weekday(d) + 7 - first_dow % 7) % 7;
    add_days(d, -(back as i64))
}

/// Today in local time.
pub fn today() -> Ymd {
    glib::DateTime::now_local()
        .map(|d| (d.year(), d.month().clamp(1, 12) as u32, d.day_of_month().clamp(1, 31) as u32))
        .unwrap_or((1970, 1, 1))
}

/// Now as civil minutes.
pub fn now_minutes() -> i64 {
    glib::DateTime::now_local()
        .map(|d| {
            days_from_civil(d.year(), d.month().clamp(1, 12) as u32, d.day_of_month().clamp(1, 31) as u32) * DAY
                + d.hour() as i64 * 60
                + d.minute() as i64
        })
        .unwrap_or(0)
}

/// Civil minutes of a day and a minute of that day.
pub fn at(d: Ymd, minute: i64) -> i64 {
    day_number(d) * DAY + minute
}

/// A civil minute as its day and the minute of that day.
pub fn split(m: i64) -> (Ymd, i64) {
    (civil_from_days(m.div_euclid(DAY)), m.rem_euclid(DAY))
}

/// Unix seconds as civil minutes in the local time zone.
pub fn unix_to_civil(secs: i64) -> i64 {
    match glib::DateTime::from_unix_local(secs) {
        Ok(d) => {
            days_from_civil(d.year(), d.month().clamp(1, 12) as u32, d.day_of_month().clamp(1, 31) as u32) * DAY
                + d.hour() as i64 * 60
                + d.minute() as i64
        }
        Err(_) => secs.div_euclid(60),
    }
}

/// Civil minutes as Unix seconds in the local time zone.
pub fn civil_to_unix(m: i64) -> i64 {
    let ((y, mo, d), min) = split(m);
    glib::DateTime::from_local(y, mo as i32, d as i32, (min / 60) as i32, (min % 60) as i32, 0.0)
        .map(|t| t.to_unix())
        .unwrap_or(m * 60)
}

/// "2026-11-13" (also "20261113") as a valid date.
pub fn parse_ymd(s: &str) -> Option<Ymd> {
    let s = s.trim();
    let (y, m, d) = if s.len() == 8 && s.chars().all(|c| c.is_ascii_digit()) {
        (s[0..4].parse().ok()?, s[4..6].parse().ok()?, s[6..8].parse().ok()?)
    } else {
        let mut parts = s.splitn(3, '-');
        let y: i32 = parts.next()?.trim().parse().ok()?;
        let m: u32 = parts.next()?.trim().parse().ok()?;
        let d: u32 = parts.next()?.trim().parse().ok()?;
        (y, m, d)
    };
    if !(1..=9999).contains(&y) || !(1..=12).contains(&m) || d < 1 || d > days_in_month(y, m) {
        return None;
    }
    Some((y, m, d))
}

pub fn ymd_string(d: Ymd) -> String {
    format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

/// "2026-11-13T09:30" (or a bare date, midnight) as civil minutes.
pub fn parse_civil(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, Some(t)),
        None => (s, None),
    };
    let d = parse_ymd(date)?;
    let minute = match time {
        Some(t) => parse_hm(t)?,
        None => 0,
    };
    Some(at(d, minute))
}

/// "09:30" (seconds and a zone suffix ignored) as minutes of the day.
pub fn parse_hm(t: &str) -> Option<i64> {
    let mut parts = t.trim().split(':');
    let h: i64 = parts.next()?.trim().parse().ok()?;
    let m: i64 = parts.next().map(|m| m.trim().chars().take(2).collect::<String>()).unwrap_or_default().parse().unwrap_or(0);
    if !(0..=24).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    Some((h * 60 + m).min(DAY))
}

/// Civil minutes as "2026-11-13T09:30".
pub fn civil_string(m: i64) -> String {
    let (d, min) = split(m);
    format!("{}T{:02}:{:02}", ymd_string(d), min / 60, min % 60)
}

// ------------------------------------------------------------------------------------------------ names

pub fn day_name(wd: u32, en: bool) -> &'static str {
    let i = (wd % 7) as usize;
    if en { EN_DAYS[i] } else { NL_DAYS[i] }
}

pub fn day_short(wd: u32, en: bool) -> &'static str {
    let i = (wd % 7) as usize;
    if en { EN_DAYS_SHORT[i] } else { NL_DAYS_SHORT[i] }
}

/// Two letters for the mini calendar ("ma", "Mo").
pub fn day_mini(wd: u32, en: bool) -> &'static str {
    let i = (wd % 7) as usize;
    if en { EN_DAYS_MINI[i] } else { NL_DAYS_SHORT[i] }
}

pub fn month_name(m: u32, en: bool) -> &'static str {
    let i = (m.clamp(1, 12) - 1) as usize;
    if en { EN_MONTHS[i] } else { NL_MONTHS[i] }
}

pub fn month_short(m: u32, en: bool) -> &'static str {
    let i = (m.clamp(1, 12) - 1) as usize;
    if en { EN_MONTHS_SHORT[i] } else { NL_MONTHS_SHORT[i] }
}

/// "november 2026" / "November 2026".
pub fn month_title(y: i32, m: u32, en: bool) -> String {
    format!("{} {}", month_name(m, en), y)
}

/// "vrijdag 13 november 2026" / "Friday, November 13, 2026".
pub fn full_date(d: Ymd, en: bool) -> String {
    if en {
        format!("{}, {} {}, {}", day_name(weekday(d), true), month_name(d.1, true), d.2, d.0)
    } else {
        format!("{} {} {} {}", day_name(weekday(d), false), d.2, month_name(d.1, false), d.0)
    }
}

/// "vr 13 nov. 2026" / "Fri, Nov 13, 2026" (the editor's date buttons).
pub fn short_date(d: Ymd, en: bool) -> String {
    if en {
        format!("{}, {} {}, {}", day_short(weekday(d), true), month_short(d.1, true), d.2, d.0)
    } else {
        format!("{} {} {} {}", day_short(weekday(d), false), d.2, month_short(d.1, false), d.0)
    }
}

/// "1 nov" / "Nov 1" (the first day of a month in the month grid).
pub fn day_month_short(d: Ymd, en: bool) -> String {
    if en { format!("{} {}", month_short(d.1, true), d.2) } else { format!("{} {}", d.2, month_short(d.1, false)) }
}

/// A minute of the day as "09:30" (24 hours) or "9:30 AM".
pub fn fmt_time(minute: i64, h24: bool) -> String {
    let minute = minute.rem_euclid(DAY);
    let (h, m) = (minute / 60, minute % 60);
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        let suffix = if h < 12 { "AM" } else { "PM" };
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        format!("{h12}:{m:02} {suffix}")
    }
}

/// An hour row label: "08:00" or "8 AM".
pub fn fmt_hour(h: i64, h24: bool) -> String {
    if h24 {
        format!("{:02}:00", h)
    } else {
        let suffix = if h < 12 { "AM" } else { "PM" };
        let h12 = if h % 12 == 0 { 12 } else { h % 12 };
        format!("{h12} {suffix}")
    }
}
