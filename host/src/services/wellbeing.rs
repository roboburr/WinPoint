//! `wellbeing.*` (batch D, notes/claude/settings-parity.md §5.5): GNOME Settings 50's Screen Time statistics for
//! Systeem › Schermtijd.
//!
//! * `wellbeing.history {}` → `{available, today, active, parental, days: [{date, weekday, seconds}]}`: GNOME Shell's
//!   own record of when the session was active (`$XDG_DATA_HOME/gnome-shell/session-active-history.json`, written by
//!   js/misc/timeLimitsManager.js: an array of `{oldState, newState, wallTimeSecs}`, UserState 0 inactive, 1
//!   active), read as GNOME Settings' cc-screen-time-statistics-row reads it, summed per local day for the last 14
//!   weeks (gnome-shell keeps no more), every day from the first one with a record to today (`weekday` 1 = Monday
//!   ... 7 = Sunday). An active period that has not ended counts up to now. A malformed file is `available: false`
//!   (GNOME: "Screen Time Unavailable"); no file is an empty `days`.
//! * `parental`: Parental Controls (malcontent) limits this user's sessions (AccountsService's
//!   com.endlessm.ParentalControls.SessionLimits LimitType is not 0), so GNOME Settings shows its banner and the
//!   limit rows are managed there.
//!
//! The settings themselves (history-enabled, daily-limit-*, grayscale, the break reminders) are ordinary
//! `gsettings.*` keys (allow/system.rs).
use super::ServiceContext;
use super::system::common::{call, getall, system};
use crate::files::ApiResult;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::sync::Arc;

/// The methods (name, dangerous), exactly as host/src/api-methods/system.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[("wellbeing.history", false)];

/// timeLimitsManager.js HISTORY_THRESHOLD_SECONDS: how long gnome-shell keeps its transitions.
pub(crate) const HISTORY_SECONDS: i64 = 14 * 7 * 24 * 60 * 60;
/// UserState.ACTIVE (UserState.INACTIVE is 0; the file's values may not change, gnome-shell says).
const ACTIVE: i64 = 1;
/// JavaScript's Number.MAX_SAFE_INTEGER (gnome-shell refuses other wall times).
const MAX_SAFE: i64 = (1 << 53) - 1;
const ACCOUNTS: &str = "org.freedesktop.Accounts";
const SESSION_LIMITS: &str = "com.endlessm.ParentalControls.SessionLimits";

/// One state change: (active afterwards, wall time in seconds since the epoch).
pub(crate) type Transition = (bool, i64);

/// gnome-shell's history file as its `_loadTransitions` accepts it: None when malformed (not an array, an entry
/// without the three integers, oldState equal to newState, an unknown state, wall times going back). Entries in the
/// future and those older than 14 weeks are left out, except the last one, which always stays (gnome-shell's
/// `_filterOldTransition`).
pub(crate) fn parse(text: &str, now: i64) -> Option<Vec<Transition>> {
    let history: Value = serde_json::from_str(text).ok()?;
    let entries = history.as_array()?;
    let mut previous = 0i64;
    let mut out = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        let state = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_i64)
                .filter(|s| matches!(s, 0 | 1))
        };
        let (Some(old), Some(new)) = (state("oldState"), state("newState")) else {
            return None;
        };
        let time = entry
            .get("wallTimeSecs")
            .filter(|v| v.is_i64() || v.is_u64())
            .and_then(Value::as_i64)
            .filter(|t| (0..=MAX_SAFE).contains(t))?;
        if old == new || time < previous {
            return None;
        }
        previous = time;
        let last = i + 1 == entries.len();
        if time <= now && (time >= now - HISTORY_SECONDS || last) {
            out.push((new == ACTIVE, time));
        }
    }
    Some(out)
}

/// The active periods [start, end) of the transitions; one still open ends at `now`.
pub(crate) fn periods(transitions: &[Transition], now: i64) -> Vec<(i64, i64)> {
    let mut out = Vec::new();
    let mut since: Option<i64> = None;
    for &(active, time) in transitions {
        match (active, since) {
            (true, None) => since = Some(time),
            (false, Some(start)) => {
                if time > start {
                    out.push((start, time));
                }
                since = None;
            }
            _ => {}
        }
    }
    if let Some(start) = since
        && now > start
    {
        out.push((start, now));
    }
    out
}

/// Local midnight at the start of the day of `time` in `tz`, and the next one.
fn day_bounds(time: i64, tz: &glib::TimeZone) -> Option<(glib::DateTime, i64, i64)> {
    let local = glib::DateTime::from_unix_utc(time)
        .ok()?
        .to_timezone(tz)
        .ok()?;
    let (y, m, d) = local.ymd();
    let start = glib::DateTime::new(tz, y, m, d, 0, 0, 0.0).ok()?;
    let next = start.add_days(1).ok()?;
    Some((start.clone(), start.to_unix(), next.to_unix()))
}

/// Screen time per local day of `tz`, from the first day with a record (at most 14 weeks back) to today:
/// [(date "YYYY-MM-DD", weekday 1-7, seconds)], every day listed, a period across midnight split over its days.
pub(crate) fn daily(
    transitions: &[Transition],
    now: i64,
    tz: &glib::TimeZone,
) -> Vec<(String, i32, i64)> {
    let Some((_, today_start, _)) = day_bounds(now, tz) else {
        return Vec::new();
    };
    let earliest = now - HISTORY_SECONDS;
    let Some(first) = transitions.iter().map(|t| t.1.max(earliest)).min() else {
        return Vec::new();
    };
    let mut totals: std::collections::BTreeMap<i64, i64> = std::collections::BTreeMap::new();
    for (start, end) in periods(transitions, now) {
        let mut at = start.max(earliest);
        while at < end {
            let Some((_, day, next)) = day_bounds(at, tz) else {
                break;
            };
            let until = end.min(next);
            *totals.entry(day).or_default() += until - at;
            at = until;
        }
    }
    let mut out = Vec::new();
    let Some((mut day, _, _)) = day_bounds(first, tz) else {
        return out;
    };
    // at most 100 days (the 14 weeks and the days of a DST change), never past today
    for _ in 0..100 {
        let start = day.to_unix();
        if start > today_start {
            break;
        }
        let date = day
            .format("%Y-%m-%d")
            .map(|s| s.to_string())
            .unwrap_or_default();
        out.push((
            date,
            day.day_of_week(),
            totals.get(&start).copied().unwrap_or(0),
        ));
        let Ok(next) = day.add_days(1) else { break };
        day = next;
    }
    out
}

/// wellbeing.history from the history file's text (None: no file).
pub(crate) fn history(text: Option<&str>, now: i64, tz: &glib::TimeZone, parental: bool) -> Value {
    let today = glib::DateTime::from_unix_utc(now)
        .ok()
        .and_then(|d| d.to_timezone(tz).ok())
        .and_then(|d| d.format("%Y-%m-%d").ok())
        .map(|s| s.to_string());
    let Some(text) = text else {
        return json!({"available": true, "today": today, "active": false, "parental": parental, "days": []});
    };
    let Some(transitions) = parse(text, now) else {
        return json!({"available": false, "today": today, "active": false, "parental": parental, "days": []});
    };
    let days: Vec<Value> = daily(&transitions, now, tz)
        .into_iter()
        .map(|(date, weekday, seconds)| json!({"date": date, "weekday": weekday, "seconds": seconds}))
        .collect();
    let active = transitions.last().is_some_and(|t| t.0);
    json!({"available": true, "today": today, "active": active, "parental": parental, "days": days})
}

/// Whether malcontent limits this user's sessions: AccountsService's user object has the SessionLimits interface
/// with a LimitType other than 0 (MCT_SESSION_LIMITS_TYPE_NONE), as gnome-shell's parentalControlsManager asks
/// `mct_session_limits_is_enabled`.
pub(crate) fn parental(bus: &gio::DBusConnection, accounts: &str, uid: u32) -> bool {
    let Ok(reply) = call(
        bus,
        accounts,
        "/org/freedesktop/Accounts",
        "org.freedesktop.Accounts",
        "FindUserById",
        Some(&(uid as i64,).to_variant()),
        500,
    ) else {
        return false;
    };
    let Some(path) = reply.child_value(0).str().map(str::to_owned) else {
        return false;
    };
    getall(bus, accounts, &path, SESSION_LIMITS)
        .ok()
        .and_then(|props| props["LimitType"].as_u64())
        .is_some_and(|t| t != 0)
}

/// gnome-shell's history file (`global.userdatadir` = `$XDG_DATA_HOME/gnome-shell`).
fn history_file() -> std::path::PathBuf {
    glib::user_data_dir()
        .join("gnome-shell")
        .join("session-active-history.json")
}

pub(crate) struct Wellbeing {
    #[allow(dead_code)]
    // the context of every service; wellbeing.history needs no language or event
    ctx: Arc<ServiceContext>,
}

impl Wellbeing {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self { ctx }))
    }

    pub(crate) fn call(&self, method: &str, _args: &Value) -> Option<ApiResult> {
        if method != "wellbeing.history" {
            return None;
        }
        let now = glib::real_time() / 1_000_000;
        // gnome-shell's file is a few kB (one entry per lock, unlock, sleep and resume over 14 weeks)
        let text = std::fs::File::open(history_file()).ok().and_then(|f| {
            use std::io::Read;
            let mut s = String::new();
            f.take(4 << 20).read_to_string(&mut s).ok().map(|_| s)
        });
        let limited = system()
            .ok()
            .is_some_and(|bus| parental(&bus, ACCOUNTS, unsafe { libc::getuid() }));
        Some(Ok(history(
            text.as_deref(),
            now,
            &glib::TimeZone::local(),
            limited,
        )))
    }

    pub(crate) fn shutdown(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::storage::test_bus::{self, Object, Props, TestBus};

    fn entry(old: i64, new: i64, t: i64) -> Value {
        json!({"oldState": old, "newState": new, "wallTimeSecs": t})
    }

    /// 2026-11-16 12:00 UTC, a Monday.
    const NOW: i64 = 1_794_830_400;
    const H: i64 = 3600;

    #[test]
    fn the_file_is_read_as_gnome_shell_reads_it() {
        let ok = json!([
            entry(0, 1, NOW - 5 * H),
            entry(1, 0, NOW - 3 * H),
            entry(0, 1, NOW - H)
        ])
        .to_string();
        assert_eq!(
            parse(&ok, NOW).unwrap(),
            vec![(true, NOW - 5 * H), (false, NOW - 3 * H), (true, NOW - H)]
        );
        for bad in [
            json!({"oldState": 0}),
            json!([entry(0, 0, NOW)]),
            json!([entry(0, 2, NOW)]),
            json!([entry(0, 1, NOW), entry(1, 0, NOW - 1)]),
            json!([{"oldState": 0, "newState": 1}]),
            json!([{"oldState": 0, "newState": 1, "wallTimeSecs": 1.5}]),
            json!([{"oldState": 0, "newState": 1, "wallTimeSecs": "12"}]),
            json!(["x"]),
        ] {
            assert_eq!(parse(&bad.to_string(), NOW), None, "{bad}");
        }
        assert_eq!(parse("not json", NOW), None);
        // older than 14 weeks: dropped, except the last entry; the future: dropped
        let old = json!([
            entry(0, 1, NOW - HISTORY_SECONDS - 10),
            entry(1, 0, NOW - 100),
            entry(0, 1, NOW + 100)
        ])
        .to_string();
        assert_eq!(parse(&old, NOW).unwrap(), vec![(false, NOW - 100)]);
        let only_old = json!([entry(0, 1, NOW - HISTORY_SECONDS - 10)]).to_string();
        assert_eq!(
            parse(&only_old, NOW).unwrap(),
            vec![(true, NOW - HISTORY_SECONDS - 10)],
            "the last one stays"
        );
    }

    #[test]
    fn screen_time_is_summed_per_local_day_and_split_at_midnight() {
        let utc = glib::TimeZone::utc();
        // Sunday 22:00 to Monday 02:00 (4 h over two days), Monday 09:00 to 10:30, and active since 11:00 (now 12:00)
        let sunday_22 = NOW - 14 * H;
        let t = vec![
            (true, sunday_22),
            (false, sunday_22 + 4 * H),
            (true, NOW - 3 * H),
            (false, NOW - 3 * H / 2),
            (true, NOW - H),
        ];
        let days = daily(&t, NOW, &utc);
        assert_eq!(
            days,
            vec![
                ("2026-11-15".to_owned(), 7, 2 * H),
                ("2026-11-16".to_owned(), 1, 9 * H / 2)
            ]
        );
        let h = history(
            Some(
                &json!([
                    entry(0, 1, NOW - 2 * 86400),
                    entry(1, 0, NOW - 2 * 86400 + 60)
                ])
                .to_string(),
            ),
            NOW,
            &utc,
            false,
        );
        assert_eq!(h["available"], true);
        assert_eq!(h["active"], false);
        assert_eq!(h["today"], "2026-11-16");
        assert_eq!(
            h["days"],
            json!([
                {"date": "2026-11-14", "weekday": 6, "seconds": 60},
                {"date": "2026-11-15", "weekday": 7, "seconds": 0},
                {"date": "2026-11-16", "weekday": 1, "seconds": 0}
            ]),
            "every day up to today"
        );
        assert_eq!(history(None, NOW, &utc, true)["days"], json!([]));
        assert_eq!(history(None, NOW, &utc, true)["parental"], true);
        assert_eq!(history(Some("[1]"), NOW, &utc, false)["available"], false);
    }

    #[test]
    fn a_day_with_a_clock_change_has_its_own_length() {
        let Some(ams) = glib::TimeZone::from_identifier(Some("Europe/Amsterdam")) else {
            eprintln!("skipped: no tzdata for Europe/Amsterdam");
            return;
        };
        // 2026-10-25 is 25 hours long in Amsterdam (CEST to CET at 03:00): active all day
        let start = glib::DateTime::new(&ams, 2026, 10, 25, 0, 0, 0.0)
            .unwrap()
            .to_unix();
        let end = glib::DateTime::new(&ams, 2026, 10, 26, 0, 0, 0.0)
            .unwrap()
            .to_unix();
        let days = daily(&[(true, start), (false, end)], end + 60, &ams);
        assert_eq!(days[0], ("2026-10-25".to_owned(), 7, 25 * H));
        assert_eq!(days[1].0, "2026-10-26");
    }

    /// Every key the Energie en batterij, Opslag, Opslaginzicht, Multitasking and Schermtijd pages read or write has
    /// its allow row (allow/system.rs) in the mode the page uses.
    #[test]
    fn the_keys_of_these_pages_have_their_allow_rows() {
        use crate::services::gsettings;
        let mut env = gsettings::Env::new(
            &gsettings::Config {
                backend: gsettings::Backend::Memory,
                ..Default::default()
            },
            None,
        );
        let power = "org.gnome.settings-daemon.plugins.power";
        let rows: &[(&str, &[&str], &str)] = &[
            (
                "org.gnome.desktop.screen-time-limits",
                &[
                    "history-enabled",
                    "daily-limit-enabled",
                    "daily-limit-seconds",
                    "grayscale",
                ],
                "write",
            ),
            (
                "org.gnome.desktop.break-reminders",
                &["selected-breaks"],
                "write",
            ),
            (
                "org.gnome.desktop.break-reminders.movement",
                &["duration-seconds", "interval-seconds", "play-sound"],
                "write",
            ),
            (
                "org.gnome.desktop.break-reminders.eyesight",
                &["play-sound"],
                "write",
            ),
            (
                "org.gnome.desktop.privacy",
                &[
                    "remove-old-trash-files",
                    "remove-old-temp-files",
                    "old-files-age",
                ],
                "write",
            ),
            (
                power,
                &["power-button-action", "power-saver-profile-on-low-battery"],
                "write",
            ),
            (
                power,
                &[
                    "idle-dim",
                    "sleep-inactive-ac-type",
                    "sleep-inactive-battery-timeout",
                ],
                "service",
            ),
            (
                "org.gnome.desktop.interface",
                &["show-battery-percentage"],
                "mirrored",
            ),
            (
                "org.gnome.desktop.interface",
                &["enable-hot-corners"],
                "write",
            ),
            (
                "org.gnome.mutter",
                &[
                    "edge-tiling",
                    "dynamic-workspaces",
                    "workspaces-only-on-primary",
                ],
                "write",
            ),
            (
                "org.gnome.desktop.wm.preferences",
                &["num-workspaces"],
                "write",
            ),
            (
                "org.gnome.shell.app-switcher",
                &["current-workspace-only"],
                "mirrored",
            ),
            (
                "org.gnome.shell.window-switcher",
                &["current-workspace-only"],
                "write",
            ),
        ];
        for (schema, keys, mode) in rows {
            for key in *keys {
                let item =
                    gsettings::read_item_in(&mut env, schema, None, key, crate::i18n::Lang::En)
                        .unwrap();
                if item["available"] != true {
                    eprintln!("{schema} {key}: not installed here");
                    continue;
                }
                assert_eq!(item["allowed"], true, "{schema} {key}");
                assert_eq!(item["mode"], *mode, "{schema} {key}");
            }
        }
        let percent = gsettings::read_item_in(
            &mut env,
            "org.gnome.desktop.interface",
            None,
            "show-battery-percentage",
            crate::i18n::Lang::En,
        )
        .unwrap();
        assert_eq!(percent["setting"], "taskbar.batteryPercent");
        let apps = gsettings::read_item_in(
            &mut env,
            "org.gnome.shell.app-switcher",
            None,
            "current-workspace-only",
            crate::i18n::Lang::En,
        )
        .unwrap();
        if apps["available"] == true {
            assert_eq!(apps["setting"], "taskView.allDesktops");
            // amendment 49: Multitasking's Alt+Tab row still writes the key itself (gsettings.set, a Mirrored row)
            let wrote = gsettings::set(
                &mut env,
                &gsettings::parse_set(
                    &serde_json::json!({"writes": [
                        {"schema": "org.gnome.shell.app-switcher", "values": {"current-workspace-only": true}},
                        {"schema": "org.gnome.shell.window-switcher", "values": {"current-workspace-only": true}}
                    ]}),
                    crate::i18n::Lang::En,
                )
                .unwrap(),
                crate::i18n::Lang::En,
                true,
                gsettings::Caller::Api,
            );
            assert!(wrote.is_ok(), "{wrote:?}");
        }
    }

    const ACCOUNTS_XML: &str = r#"<interface name="org.freedesktop.Accounts">
<method name="FindUserById"><arg name="id" type="x" direction="in"/><arg name="user" type="o" direction="out"/></method>
</interface>"#;
    // malcontent's com.endlessm.ParentalControls.SessionLimits.xml (/usr/share/dbus-1/interfaces)
    const LIMITS_XML: &str = r#"<interface name="com.endlessm.ParentalControls.SessionLimits">
<property name="LimitType" type="u" access="readwrite"/><property name="DailySchedule" type="(uu)" access="readwrite"/>
<property name="DailyLimit" type="u" access="readwrite"/></interface>"#;
    const USER: &str = "/org/freedesktop/Accounts/User1000";

    #[test]
    fn parental_controls_are_read_from_accountsservice() {
        let props = Props::default();
        props.set(USER, "LimitType", 1u32.to_variant());
        props.set(USER, "DailySchedule", (0u32, 86400u32).to_variant());
        props.set(USER, "DailyLimit", 0u32.to_variant());
        let Some(fake) = TestBus::start(
            vec![
                Object {
                    path: "/org/freedesktop/Accounts".into(),
                    xml: ACCOUNTS_XML.into(),
                },
                Object {
                    path: USER.into(),
                    xml: LIMITS_XML.into(),
                },
            ],
            props,
            Arc::new(|_, _, method, params| match method {
                "FindUserById" if params.child_value(0).get::<i64>() == Some(1000) => Ok(Some(
                    (crate::services::system::common::object_path(USER).unwrap(),).to_variant(),
                )),
                _ => Err((
                    "org.freedesktop.Accounts.Error.Failed".into(),
                    "no such user".into(),
                )),
            }),
        ) else {
            eprintln!(
                "skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)"
            );
            return;
        };
        let bus = test_bus::client();
        assert!(parental(&bus, &fake.name, 1000), "a daily schedule");
        assert!(!parental(&bus, &fake.name, 1001), "another user: not found");
        fake.props.set(USER, "LimitType", 0u32.to_variant());
        assert!(!parental(&bus, &fake.name, 1000), "no limit");
        assert!(!parental(&bus, ":1.999999", 1000), "no AccountsService");
        let calls = fake.calls_of("FindUserById");
        assert_eq!(calls[0].args, json!([1000]));
        assert!(!calls[0].interactive);
    }
}
