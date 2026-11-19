//! `machine.time.*`: the system clock through systemd-timedated (org.freedesktop.timedate1), as GNOME Settings' Date
//! & Time panel sets it: SetNTP, SetTime (only while NTP is off), SetTimezone, each with `interactive` true (polkit
//! `timedate1.set-*`, yes for a local administrator on Ubuntu).
//!
//! There is no timesync1 (amendment 30): Ubuntu runs chrony, so "Nu synchroniseren" is SetNTP(false) then
//! SetNTP(true), which restarts chrony.service (ntp-units.d); the status is timedate1's NTPSynchronized and the time
//! server comes from chrony's own configuration files, read only (a `server` or `pool` line in chrony.conf, its
//! `confdir` and `sourcedir` files), else from timesyncd.conf, else there is none and the page hides the row.
use super::{Buses, TIMEDATE, dry, unavailable, zones};
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::sync::Mutex;

/// What this host saw of the synchronisation: NTPSynchronized at the last read, and when it last turned true
/// (Windows' "Laatste geslaagde tijdsynchronisatie").
#[derive(Default)]
pub(crate) struct Seen {
    synced: Option<bool>,
    last_sync: Option<u64>,
}

/// The time servers of chrony's configuration: `server`/`pool`/`peer` lines of chrony.conf, then of the `confdir`
/// (*.conf) and `sourcedir` (*.sources) files it names, in that order, without doubles.
pub(crate) fn chrony_servers(
    conf: &str,
    read_dir: &dyn Fn(&str, &str) -> Vec<String>,
) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let add = |text: &str, out: &mut Vec<String>| {
        for line in text.lines() {
            let mut words = line.split_whitespace();
            if let (Some("server" | "pool" | "peer"), Some(host)) = (words.next(), words.next())
                && !out.iter().any(|h| h == host)
                && host.len() <= 253
            {
                out.push(host.to_owned());
            }
        }
    };
    add(conf, &mut out);
    for line in conf.lines() {
        let mut words = line.split_whitespace();
        let (key, dir) = (words.next(), words.next());
        let ext = match key {
            Some("confdir") => "conf",
            Some("sourcedir") => "sources",
            _ => continue,
        };
        if let Some(dir) = dir {
            for text in read_dir(dir, ext) {
                add(&text, &mut out);
            }
        }
    }
    out
}

/// The `NTP=` (else `FallbackNTP=`) servers of systemd-timesyncd's configuration.
pub(crate) fn timesyncd_servers(conf: &str) -> Vec<String> {
    let find = |key: &str| {
        conf.lines()
            .filter_map(|l| l.trim().strip_prefix(key))
            .filter_map(|v| v.strip_prefix('='))
            .flat_map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>())
            .collect::<Vec<_>>()
    };
    let ntp = find("NTP");
    if ntp.is_empty() {
        find("FallbackNTP")
    } else {
        ntp
    }
}

/// The time server this computer uses, if its configuration names one.
pub(crate) fn server(b: &Buses) -> Option<String> {
    let read_dir = |dir: &str, ext: &str| -> Vec<String> {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(b.file(dir))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == ext))
            .collect();
        files.sort();
        files
            .iter()
            .take(64)
            .filter_map(|p| std::fs::read_to_string(p).ok())
            .collect()
    };
    let chrony = std::fs::read_to_string(b.file("/etc/chrony/chrony.conf"))
        .or_else(|_| std::fs::read_to_string(b.file("/etc/chrony.conf")));
    let list = match chrony {
        Ok(conf) => chrony_servers(&conf, &read_dir),
        Err(_) => std::fs::read_to_string(b.file("/etc/systemd/timesyncd.conf"))
            .map(|c| timesyncd_servers(&c))
            .unwrap_or_default(),
    };
    list.into_iter().next()
}

fn props(b: &Buses) -> Option<Value> {
    let (_, path, iface) = TIMEDATE;
    b.timedate()?.props(path, iface).ok()
}

/// The zone the computer has now: timedate1's Timezone, else /etc/localtime.
pub(crate) fn current_zone(b: &Buses) -> Option<String> {
    props(b)
        .and_then(|p| p["Timezone"].as_str().map(str::to_owned))
        .filter(|z| !z.is_empty())
        .or_else(|| zones::local_zone(b))
}

/// `machine.time.get {}` → `{available, timezone, zoneLabel, ntp, canNtp, synced, localRtc, server, lastSync, now}`.
pub(crate) fn get(b: &Buses, seen: &Mutex<Seen>, lang: Lang) -> Value {
    let p = props(b);
    let now = crate::services::system::common::now_ms();
    let synced = p.as_ref().map(|p| p["NTPSynchronized"] == true);
    let last_sync = {
        let mut s = seen.lock().unwrap_or_else(|e| e.into_inner());
        if synced == Some(true) && s.synced == Some(false) {
            s.last_sync = Some(now);
        }
        if synced.is_some() {
            s.synced = synced;
        }
        s.last_sync
    };
    let zone = p
        .as_ref()
        .and_then(|p| p["Timezone"].as_str().map(str::to_owned))
        .filter(|z| !z.is_empty())
        .or_else(|| zones::local_zone(b))
        .unwrap_or_else(|| "UTC".into());
    let flag = |k: &str| p.as_ref().is_some_and(|p| p[k] == true);
    json!({
        "available": p.is_some(),
        "timezone": zone,
        "zoneLabel": zones::label(&zone, lang),
        "ntp": flag("NTP"),
        "canNtp": flag("CanNTP"),
        "synced": flag("NTPSynchronized"),
        "localRtc": flag("LocalRTC"),
        "server": server(b),
        "lastSync": last_sync,
        "now": now,
    })
}

fn timedate(b: &Buses, lang: Lang) -> Result<super::Peer, ApiError> {
    b.timedate()
        .filter(|_| props(b).is_some())
        .ok_or_else(|| unavailable(lang, "machine.timeUnavailable"))
}

fn call(b: &Buses, method: &str, args: glib::Variant, lang: Lang) -> Result<(), ApiError> {
    let (_, path, iface) = TIMEDATE;
    timedate(b, lang)?
        .interactive(path, iface, method, &args, lang)
        .map(|_| ())
}

/// `machine.time.setNtp {on}`: "Tijd automatisch instellen".
pub(crate) fn set_ntp(b: &Buses, on: bool, lang: Lang, dry_run: bool) -> ApiResult {
    if dry_run {
        return dry();
    }
    call(b, "SetNTP", (on, true).to_variant(), lang)?;
    Ok(json!({"ok": true}))
}

/// The microseconds since the epoch a manual time may be: 2000-01-01 to 2200-01-01 (GNOME's dialog offers years
/// from 1 to 9999, but timedated refuses times before its build epoch anyway).
pub(crate) fn valid_usec(usec: i64) -> bool {
    (946_684_800_000_000..7_258_118_400_000_000).contains(&usec)
}

/// `machine.time.setTime {usec}`: "De datum en tijd handmatig instellen" (only while NTP is off, as GNOME).
pub(crate) fn set_time(b: &Buses, usec: i64, lang: Lang, dry_run: bool) -> ApiResult {
    if !valid_usec(usec) {
        return Err(ApiError::tr("bad_args", lang, "machine.badTime", NO_PARAMS));
    }
    if dry_run {
        return dry();
    }
    if props(b).is_some_and(|p| p["NTP"] == true) {
        return Err(ApiError::tr("denied", lang, "machine.ntpOn", NO_PARAMS));
    }
    call(b, "SetTime", (usec, false, true).to_variant(), lang)?;
    Ok(json!({"ok": true}))
}

/// `machine.time.setTimezone {zone}`: "Tijdzone".
pub(crate) fn set_timezone(b: &Buses, zone: &str, lang: Lang, dry_run: bool) -> ApiResult {
    if !zones::exists(b, zone) {
        return Err(ApiError::tr("bad_args", lang, "machine.badZone", NO_PARAMS));
    }
    if dry_run {
        return dry();
    }
    call(b, "SetTimezone", (zone, true).to_variant(), lang)?;
    Ok(json!({"ok": true, "timezone": zone, "zoneLabel": zones::label(zone, lang)}))
}

/// `machine.time.sync {}`: "Nu synchroniseren": SetNTP(false), then SetNTP(true) (amendment 30), only while NTP is
/// on (it would switch automatic time on otherwise).
pub(crate) fn sync(b: &Buses, seen: &Mutex<Seen>, lang: Lang, dry_run: bool) -> ApiResult {
    if dry_run {
        return dry();
    }
    let p = props(b).ok_or_else(|| unavailable(lang, "machine.timeUnavailable"))?;
    if p["NTP"] != true || p["CanNTP"] == false {
        return Err(ApiError::tr("denied", lang, "machine.ntpOff", NO_PARAMS));
    }
    call(b, "SetNTP", (false, true).to_variant(), lang)?;
    // the restart: chrony comes back and synchronises again, NTPSynchronized turns true once it did
    seen.lock().unwrap_or_else(|e| e.into_inner()).synced = Some(false);
    // NTP must not stay off: one more try when the second call fails
    call(b, "SetNTP", (true, true).to_variant(), lang)
        .or_else(|_| call(b, "SetNTP", (true, true).to_variant(), lang))?;
    Ok(json!({"ok": true}))
}
