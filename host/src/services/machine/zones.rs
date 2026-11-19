//! `machine.time.zones`: the time zones of Datum en tijd's "Tijdzone" drop-down, labelled as Windows labels them
//! ("(UTC+01:00) Amsterdam": the standard offset, then the city), sorted by offset and city. The list is tzdata's
//! zone1970.tab (every zone with a place of its own, as GNOME's city search offers them), then UTC and the zone the
//! computer has now when that is another one.
use super::Buses;
use crate::i18n::Lang;
use serde_json::{Value, json};

/// The zones of zone1970.tab (or the older zone.tab): its third column, in file order.
pub(crate) fn parse_tab(text: &str) -> Vec<String> {
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::trim)
        .filter(|z| valid_name(z))
        .map(str::to_owned)
        .collect()
}

/// A zone name timedate1 could be asked for: `Area/City` characters only, no `..`, at most 64 bytes.
pub(crate) fn valid_name(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 64
        && !zone.starts_with('/')
        && !zone.ends_with('/')
        && !zone.contains("..")
        && !zone.contains("//")
        && zone
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/_+-".contains(&b))
}

/// A zone that exists in tzdata (a file under /usr/share/zoneinfo).
pub(crate) fn exists(b: &Buses, zone: &str) -> bool {
    valid_name(zone) && b.file(&format!("/usr/share/zoneinfo/{zone}")).is_file()
}

/// The city of a zone as Windows shows it: "Europe/Amsterdam" → "Amsterdam", "America/Argentina/Buenos_Aires" →
/// "Buenos Aires (Argentina)".
pub(crate) fn city(zone: &str) -> String {
    let parts: Vec<String> = zone.split('/').map(|p| p.replace('_', " ")).collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [.., area, last] if parts.len() >= 3 => format!("{last} ({area})"),
        [.., last] => last.clone(),
    }
}

/// The zone's standard offset in minutes east of UTC: the smaller of its January and July offsets this year (summer
/// time is the larger one; Dublin's winter time counts as its standard, as Windows has it).
pub(crate) fn standard_offset(zone: &str) -> Option<i32> {
    let tz = glib::TimeZone::from_identifier(Some(zone))?;
    let year = glib::DateTime::now_utc().map(|d| d.year()).unwrap_or(2026);
    let at = |month: i32| {
        let t = glib::DateTime::from_utc(year, month, 15, 12, 0, 0.0)
            .ok()?
            .to_unix();
        let interval = tz.find_interval(glib::TimeType::Universal, t);
        (interval >= 0).then(|| tz.offset(interval) / 60)
    };
    match (at(1), at(7)) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// "(UTC+01:00)", "(UTC-03:30)", "(UTC+00:00)".
pub(crate) fn offset_text(minutes: i32) -> String {
    let sign = if minutes < 0 { '-' } else { '+' };
    let m = minutes.unsigned_abs();
    format!("(UTC{sign}{:02}:{:02})", m / 60, m % 60)
}

/// The label of a zone, as the drop-down shows it.
pub(crate) fn label(zone: &str, lang: Lang) -> String {
    if matches!(
        zone,
        "UTC" | "Etc/UTC" | "Etc/UCT" | "UCT" | "Etc/Universal" | "Universal" | "Etc/Zulu" | "Zulu"
    ) {
        return format!("(UTC) {}", crate::i18n::tr(lang, "machine.zone.utc"));
    }
    match standard_offset(zone) {
        Some(m) => format!("{} {}", offset_text(m), city(zone)),
        None => city(zone),
    }
}

/// The zone the computer has now from /etc/localtime (when timedate1 is not there to say it).
pub(crate) fn local_zone(b: &Buses) -> Option<String> {
    let target = std::fs::read_link(b.file("/etc/localtime")).ok()?;
    let text = target.to_string_lossy().into_owned();
    let (_, zone) = text.split_once("zoneinfo/")?;
    valid_name(zone).then(|| zone.to_owned())
}

/// `machine.time.zones {}` → `{zones: [{id, label, offset}]}`.
pub(crate) fn list(b: &Buses, current: Option<&str>, lang: Lang) -> Value {
    let tab = std::fs::read_to_string(b.file("/usr/share/zoneinfo/zone1970.tab"))
        .or_else(|_| std::fs::read_to_string(b.file("/usr/share/zoneinfo/zone.tab")))
        .unwrap_or_default();
    let mut ids = parse_tab(&tab);
    for extra in ["UTC"].into_iter().chain(current) {
        if !ids.iter().any(|z| z == extra) {
            ids.push(extra.to_owned());
        }
    }
    let mut zones: Vec<(i32, String, String)> = ids
        .into_iter()
        .map(|id| {
            let offset = if id == "UTC" {
                0
            } else {
                standard_offset(&id).unwrap_or(0)
            };
            (offset, label(&id, lang), id)
        })
        .collect();
    zones.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| city(&a.2).to_lowercase().cmp(&city(&b.2).to_lowercase()))
    });
    zones.dedup_by(|a, b| a.2 == b.2);
    json!({
        "zones": zones
            .into_iter()
            .map(|(offset, label, id)| json!({"id": id, "label": label, "offset": offset}))
            .collect::<Vec<_>>(),
    })
}
