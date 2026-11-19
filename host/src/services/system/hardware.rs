use super::{common::*, number_arg, pct_arg};
use crate::files::{ApiError, ApiResult};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::Mutex,
    time::{Duration, Instant},
};
fn read(p: impl AsRef<std::path::Path>) -> Option<String> {
    fs::read(p)
        .ok()
        .map(|s| String::from_utf8_lossy(&s).trim().into())
}
fn backlight() -> Option<(String, PathBuf, i64)> {
    let mut all = vec![];
    for dir in fs::read_dir("/sys/class/backlight").ok()?.flatten() {
        let p = dir.path();
        let max = read(p.join("max_brightness"))
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        if max <= 0 {
            continue;
        }
        let kind = read(p.join("type")).unwrap_or_else(|| "raw".into());
        let rank = match kind.as_str() {
            "firmware" => 0,
            "platform" => 1,
            "raw" => 2,
            _ => 3,
        };
        all.push((rank, dir.file_name().to_string_lossy().into_owned(), p, max));
    }
    all.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
    all.into_iter()
        .next()
        .map(|(_, name, p, max)| (name, p, max))
}
pub(super) fn session_path() -> String {
    let fallback = "/org/freedesktop/login1/session/auto".to_owned();
    let Ok(bus) = system() else { return fallback };
    let root = "/org/freedesktop/login1";
    let iface = "org.freedesktop.login1.Manager";
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if let Ok(r) = call(
            &bus,
            "org.freedesktop.login1",
            root,
            iface,
            "GetSession",
            Some(&(id,).to_variant()),
            1500,
        ) {
            if let Some(p) = first(&r).as_str() {
                return p.into();
            }
        }
    }
    if let Ok(r) = call(
        &bus,
        "org.freedesktop.login1",
        root,
        iface,
        "GetSessionByPID",
        Some(&(std::process::id(),).to_variant()),
        1500,
    ) {
        if let Some(p) = first(&r).as_str() {
            return p.into();
        }
    }
    if let Ok(r) = call(
        &bus,
        "org.freedesktop.login1",
        root,
        iface,
        "ListSessions",
        None,
        1500,
    ) {
        let sessions = first(&r);
        let empty = Vec::new();
        let mine = sessions
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter(|s| s[1] == unsafe { libc::geteuid() } && !string(&s[3]).is_empty())
            .collect::<Vec<_>>();
        for s in &mine {
            let p = s[4].as_str().unwrap_or("");
            if getall(
                &bus,
                "org.freedesktop.login1",
                p,
                "org.freedesktop.login1.Session",
            )
            .is_ok_and(|p| truth(&p["Active"]))
            {
                return p.into();
            }
        }
        if let Some(first) = mine.first() {
            return string(&first[4]);
        }
    }
    fallback
}
pub(super) fn session_id() -> Option<String> {
    std::env::var("XDG_SESSION_ID")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| {
            let bus = system().ok()?;
            let p = getall(
                &bus,
                "org.freedesktop.login1",
                &session_path(),
                "org.freedesktop.login1.Session",
            )
            .ok()?;
            p["Id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        })
}
/// Mutter's backlight (amendment 31): GNOME 50 owns the brightness in mutter (MetaBacklight, the DisplayConfig
/// `Backlight` property `(uaa{sv})` with `connector`, `active`, `min`, `max` and `value` per monitor, and
/// `SetBacklight(u serial, s connector, i value)`). Setting it there keeps GNOME Shell's brightness slider, idle
/// dimming and gsd-power's automatic brightness on the same level; logind's SetBrightness would bypass mutter, so it
/// stays only for Hyprland and as the fallback without mutter.
#[derive(Debug, PartialEq)]
pub(super) struct MutterBacklight {
    pub serial: u32,
    pub connector: String,
    pub min: i64,
    pub max: i64,
    pub value: i64,
}
/// The first active monitor's entry of a `Backlight` value (its `variant_json` form `[serial, [{...}]]`): the level
/// Settings shows.
pub(super) fn mutter_backlight_of(v: &Value) -> Option<MutterBacklight> {
    mutter_backlights_of(v).into_iter().next()
}
/// Every active monitor's entry of a `Backlight` value, in its order (no active one: the first usable one), as GNOME
/// Shell's brightness slider sets them all together.
pub(super) fn mutter_backlights_of(v: &Value) -> Vec<MutterBacklight> {
    let Some(serial) = integer(&v[0]).and_then(|s| u32::try_from(s).ok()) else {
        return Vec::new();
    };
    let Some(entries) = v[1].as_array() else {
        return Vec::new();
    };
    let usable = |e: &&Value| {
        e["connector"].as_str().is_some_and(|c| !c.is_empty())
            && integer(&e["max"]).unwrap_or(0) > integer(&e["min"]).unwrap_or(0)
    };
    let mut chosen: Vec<&Value> = entries
        .iter()
        .filter(usable)
        .filter(|e| truth(&e["active"]))
        .collect();
    if chosen.is_empty() {
        chosen.extend(entries.iter().find(usable));
    }
    chosen
        .into_iter()
        .map(|e| MutterBacklight {
            serial,
            connector: string(&e["connector"]),
            min: integer(&e["min"]).unwrap_or(0),
            max: integer(&e["max"]).unwrap_or(0),
            value: integer(&e["value"]).unwrap_or(0),
        })
        .collect()
}
/// The raw level of each backlight for one 0-100 % level: (connector, level in that backlight's own range), so
/// every backlight shows the same fraction.
#[cfg_attr(not(test), allow(dead_code))] // the rule mutter_set_all_on follows (mutter_set_on per backlight)
pub(super) fn levels_for(list: &[MutterBacklight], percent: i64) -> Vec<(String, i64)> {
    list.iter()
        .map(|b| (b.connector.clone(), backlight_raw(percent, b.min, b.max)))
        .collect()
}
/// A raw level as 0-100 %.
pub(super) fn backlight_percent(value: i64, min: i64, max: i64) -> i64 {
    if max <= min {
        return 0;
    }
    round((value - min) as f64 * 100. / (max - min) as f64, 0).clamp(0., 100.) as i64
}
/// 0-100 % as a raw level in mutter's range (mutter's own minimum is the lowest it allows).
pub(super) fn backlight_raw(percent: i64, min: i64, max: i64) -> i64 {
    (min + round((max - min) as f64 * percent.clamp(0, 100) as f64 / 100., 0) as i64)
        .clamp(min, max)
}
fn mutter_backlights_on(bus: &gio::DBusConnection, dest: &str) -> Vec<MutterBacklight> {
    call(
        bus,
        dest,
        crate::services::display::mutter::PATH,
        "org.freedesktop.DBus.Properties",
        "Get",
        Some(&(crate::services::display::mutter::IFACE, "Backlight").to_variant()),
        1000,
    )
    .map(|v| mutter_backlights_of(&first(&v)))
    .unwrap_or_default()
}
fn mutter_backlight_on(bus: &gio::DBusConnection, dest: &str) -> Option<MutterBacklight> {
    mutter_backlights_on(bus, dest).into_iter().next()
}
fn mutter_backlight() -> Option<(gio::DBusConnection, MutterBacklight)> {
    let bus = session().ok()?;
    let b = mutter_backlight_on(&bus, crate::services::display::mutter::NAME)?;
    Some((bus, b))
}
fn mutter_backlights() -> Option<(gio::DBusConnection, Vec<MutterBacklight>)> {
    let bus = session().ok()?;
    let list = mutter_backlights_on(&bus, crate::services::display::mutter::NAME);
    (!list.is_empty()).then_some((bus, list))
}
/// SetBacklight of every backlight in `list` to the same `percent` (GNOME Shell's slider scales them together);
/// the first one's answer, whose level Settings shows. A later backlight that refuses keeps its level.
pub(super) fn mutter_set_all_on(
    bus: &gio::DBusConnection,
    dest: &str,
    list: Vec<MutterBacklight>,
    percent: i64,
) -> Result<MutterBacklight, String> {
    let mut first: Option<Result<MutterBacklight, String>> = None;
    for b in list {
        let connector = b.connector.clone();
        let result = mutter_set_on(bus, dest, b, percent);
        match &first {
            None => first = Some(result),
            Some(_) => {
                if let Err(e) = result {
                    eprintln!("SetBacklight {connector}: {e}");
                }
            }
        }
    }
    first.unwrap_or_else(|| Err("no backlight".into()))
}
/// SetBacklight to `percent` (0-100); a stale serial (the level changed meanwhile, or another backlight was just
/// set) is read again once, for the same connector.
pub(super) fn mutter_set_on(
    bus: &gio::DBusConnection,
    dest: &str,
    mut b: MutterBacklight,
    percent: i64,
) -> Result<MutterBacklight, String> {
    for attempt in 0..2 {
        let raw = backlight_raw(percent, b.min, b.max);
        let result = call(
            bus,
            dest,
            crate::services::display::mutter::PATH,
            crate::services::display::mutter::IFACE,
            "SetBacklight",
            Some(&(b.serial, b.connector.as_str(), raw as i32).to_variant()),
            3000,
        );
        match result {
            Ok(_) => {
                b.value = raw;
                return Ok(b);
            }
            Err(e) if attempt == 0 && e.contains("serial") => {
                let connector = std::mem::take(&mut b.connector);
                b = mutter_backlights_on(bus, dest)
                    .into_iter()
                    .find(|n| n.connector == connector)
                    .ok_or(e)?;
            }
            Err(e) => return Err(e),
        }
    }
    Err("SetBacklight".into())
}
pub(super) fn brightness_get() -> Value {
    if let Some((_, b)) = mutter_backlight() {
        return json!({"available":true,"value":backlight_percent(b.value, b.min, b.max),"device":b.connector});
    }
    let none = || json!({"available":false,"value":null});
    let Some((name, p, max)) = backlight() else {
        return none();
    };
    let Some(cur) = read(p.join("brightness")).and_then(|s| s.parse::<i64>().ok()) else {
        return none();
    };
    json!({"available":true,"value":round(cur as f64*100./max as f64,0)as i64,"device":name})
}
/// The level `args` asks for ({value} or {delta}) from the current one.
fn wanted(args: &Value, current: i64, lang: crate::i18n::Lang) -> Result<i64, ApiError> {
    if !args["delta"].is_null() {
        Ok(round(current as f64 + number_arg(&args["delta"], lang)?, 0).clamp(0., 100.) as i64)
    } else if !args["value"].is_null() {
        pct_arg(&args["value"], lang)
    } else {
        Err(ApiError::tr(
            "bad_args",
            lang,
            "system.brightness.missing",
            crate::i18n::NO_PARAMS,
        ))
    }
}
pub(super) fn brightness_set(args: &Value, lang: crate::i18n::Lang) -> ApiResult {
    let failed = || {
        ApiError::tr(
            "failed",
            lang,
            "system.brightness.failed",
            crate::i18n::NO_PARAMS,
        )
    };
    if let Some((bus, list)) = mutter_backlights() {
        let v = wanted(
            args,
            backlight_percent(list[0].value, list[0].min, list[0].max),
            lang,
        )?;
        let b = mutter_set_all_on(&bus, crate::services::display::mutter::NAME, list, v)
            .map_err(|_| failed())?;
        return Ok(json!({"available":true,"value":v,"device":b.connector}));
    }
    let (name, _, max) = backlight().ok_or_else(|| {
        ApiError::tr(
            "unsupported",
            lang,
            "system.brightness.unsupported",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let current = number(&brightness_get()["value"]).unwrap_or(0.) as i64;
    let v = wanted(args, current, lang)?;
    let raw = round(max as f64 * v as f64 / 100., 0).max(1.) as u32;
    let bus = system().map_err(|_| failed())?;
    call(
        &bus,
        "org.freedesktop.login1",
        &session_path(),
        "org.freedesktop.login1.Session",
        "SetBrightness",
        Some(&("backlight", name.as_str(), raw).to_variant()),
        3000,
    )
    .map_err(|_| failed())?;
    Ok(json!({"available":true,"value":v,"device":name}))
}
pub(super) struct Battery {
    health: Mutex<Option<(Instant, Option<i64>, Option<i64>)>>,
}
impl Battery {
    pub fn new() -> Self {
        Self {
            health: Mutex::new(None),
        }
    }
    fn none(ac: bool) -> Value {
        json!({"present":false,"percent":null,"state":"none","timeToEmpty":null,"timeToFull":null,"onAC":ac,"health":null,"cycles":null,"energyRate":null})
    }
    pub fn get(&self) -> Value {
        let Ok(bus) = system() else {
            return Self::none(true);
        };
        let up = "org.freedesktop.UPower";
        let Ok(p) = getall(
            &bus,
            up,
            "/org/freedesktop/UPower/devices/DisplayDevice",
            "org.freedesktop.UPower.Device",
        ) else {
            return Self::none(true);
        };
        let Ok(root) = getall(&bus, up, "/org/freedesktop/UPower", up) else {
            return Self::none(true);
        };
        let ac = !truth(&root["OnBattery"]);
        if !truth(&p["IsPresent"]) || (!p["Type"].is_null() && p["Type"] != 2) {
            return Self::none(ac);
        }
        let (health, cycles) = self.health_cycles(&bus);
        let state = match integer(&p["State"]).unwrap_or(0) {
            1 => "charging",
            2 => "discharging",
            3 => "empty",
            4 => "full",
            5 => "pending-charge",
            6 => "pending-discharge",
            _ => "unknown",
        };
        let seconds = |v: &Value| integer(v).filter(|n| *n != 0);
        json!({"present":true,"percent":round(number(&p["Percentage"]).unwrap_or(0.),0)as i64,"state":state,"timeToEmpty":seconds(&p["TimeToEmpty"]),"timeToFull":seconds(&p["TimeToFull"]),"onAC":ac,"health":health,"cycles":cycles,"energyRate":number(&p["EnergyRate"]).filter(|n|*n!=0.).map(|n|round(n,2))})
    }
    fn health_cycles(&self, bus: &gio::DBusConnection) -> (Option<i64>, Option<i64>) {
        let mut cache = self.health.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, health, cycles)) = *cache {
            if at.elapsed() < Duration::from_secs(300) {
                return (health, cycles);
            }
        }
        let (mut health, mut cycles) = (None, None);
        if let Ok(r) = call(
            bus,
            "org.freedesktop.UPower",
            "/org/freedesktop/UPower",
            "org.freedesktop.UPower",
            "EnumerateDevices",
            None,
            1500,
        ) {
            for path in first(&r).as_array().unwrap_or(&vec![]) {
                if let Some(path) = path.as_str() {
                    if let Ok(p) = getall(
                        bus,
                        "org.freedesktop.UPower",
                        path,
                        "org.freedesktop.UPower.Device",
                    ) {
                        if p["Type"] != 2 || !truth(&p["PowerSupply"]) {
                            continue;
                        }
                        let full = number(&p["EnergyFull"]).unwrap_or(0.);
                        let design = number(&p["EnergyFullDesign"]).unwrap_or(0.);
                        if full > 0. && design > 0. {
                            health = Some(round((full * 100. / design).min(100.), 0) as i64)
                        }
                        cycles = integer(&p["ChargeCycles"]).filter(|n| *n > 0);
                        break;
                    }
                }
            }
        }
        if let Ok(ds) = fs::read_dir("/sys/class/power_supply") {
            let mut ds = ds
                .flatten()
                .filter(|d| d.file_name().to_string_lossy().starts_with("BAT"))
                .collect::<Vec<_>>();
            ds.sort_by_key(|d| d.file_name());
            if let Some(d) = ds.first() {
                let n = |key| read(d.path().join(key)).and_then(|s| s.parse::<i64>().ok());
                if let Some(c) = n("cycle_count").filter(|n| *n > 0) {
                    cycles = Some(c)
                }
                if health.is_none() {
                    let full = n("energy_full")
                        .filter(|n| *n != 0)
                        .or_else(|| n("charge_full"));
                    let design = n("energy_full_design")
                        .filter(|n| *n != 0)
                        .or_else(|| n("charge_full_design"));
                    if let (Some(f), Some(d)) = (full, design) {
                        if f != 0 && d != 0 {
                            health = Some(round((f as f64 * 100. / d as f64).min(100.), 0) as i64)
                        }
                    }
                }
            }
        }
        *cache = Some((Instant::now(), health, cycles));
        (health, cycles)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::display::fake::{Fake, FakeState};
    #[test]
    fn mutters_backlight_entry_and_level_mapping() {
        let v = json!([4, [
            {"connector": "HDMI-1", "active": false, "min": 0, "max": 100, "value": 10},
            {"connector": "eDP-1", "active": true, "min": 1, "max": 19393, "value": 9697}
        ]]);
        let b = mutter_backlight_of(&v).unwrap();
        assert_eq!(
            (b.serial, b.connector.as_str(), b.min, b.max),
            (4, "eDP-1", 1, 19393)
        );
        assert_eq!(backlight_percent(b.value, b.min, b.max), 50);
        assert_eq!(
            backlight_raw(0, 1, 19393),
            1,
            "mutter's minimum, never below"
        );
        assert_eq!(backlight_raw(100, 1, 19393), 19393);
        assert_eq!(backlight_raw(150, 0, 15), 15);
        assert_eq!(backlight_raw(50, 0, 15), 8);
        assert_eq!(backlight_percent(5, 10, 10), 0, "no range");
        assert!(
            mutter_backlight_of(&json!([1, []])).is_none(),
            "no backlight: logind's fallback"
        );
        assert!(
            mutter_backlight_of(&json!([1, [{"connector": "eDP-1", "min": 5, "max": 5}]]))
                .is_none()
        );
        // every active backlight, each with its own range: the same fraction for all (GNOME Shell's slider)
        let two = json!([7, [
            {"connector": "eDP-1", "active": true, "min": 1, "max": 19393, "value": 9697},
            {"connector": "HDMI-1", "active": false, "min": 0, "max": 100, "value": 10},
            {"connector": "DP-2", "active": true, "min": 0, "max": 100, "value": 50}
        ]]);
        let list = mutter_backlights_of(&two);
        assert_eq!(
            list.iter()
                .map(|b| (b.serial, b.connector.as_str()))
                .collect::<Vec<_>>(),
            [(7, "eDP-1"), (7, "DP-2")],
            "the inactive one is left alone"
        );
        assert_eq!(
            mutter_backlight_of(&two).unwrap().connector,
            "eDP-1",
            "the level shown: the first"
        );
        assert_eq!(
            levels_for(&list, 25),
            [("eDP-1".to_owned(), 4849), ("DP-2".to_owned(), 25)]
        );
        assert_eq!(
            levels_for(&list, 100),
            [("eDP-1".to_owned(), 19393), ("DP-2".to_owned(), 100)]
        );
        // no active entry: the first usable one
        let b = mutter_backlight_of(
            &json!([2, [{"connector": "eDP-2", "min": 0, "max": 9, "value": 9}]]),
        )
        .unwrap();
        assert_eq!(b.connector, "eDP-2");
    }
    /// SetBacklight on the fake DisplayConfig (amendment 31), with a stale serial read again once.
    #[test]
    fn the_brightness_goes_through_mutters_set_backlight() {
        let Some(fake) = Fake::start(FakeState::default()) else {
            eprintln!(
                "skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)"
            );
            return;
        };
        let bus = session().unwrap();
        let b = mutter_backlight_on(&bus, &fake.name).unwrap();
        assert_eq!(
            (b.serial, b.connector.as_str(), b.max, b.value),
            (3, "eDP-1", 19393, 9696)
        );
        let b = mutter_set_on(&bus, &fake.name, b, 25).unwrap();
        assert_eq!(b.value, 4848);
        assert_eq!(fake.calls_of("SetBacklight"), [json!([3, "eDP-1", 4848])]);
        // the level changed elsewhere (the fake's serial moved on): read again, then set
        let stale = MutterBacklight {
            serial: 1,
            ..mutter_backlight_on(&bus, &fake.name).unwrap()
        };
        let b = mutter_set_on(&bus, &fake.name, stale, 100).unwrap();
        assert_eq!(b.value, 19393);
        let calls = fake.calls_of("SetBacklight");
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2], json!([4, "eDP-1", 19393]));
        assert_eq!(
            fake.state.lock().unwrap().backlight.as_ref().unwrap().4,
            19393
        );
        // every backlight together (the fake has one): one SetBacklight per backlight
        let list = mutter_backlights_on(&bus, &fake.name);
        assert_eq!(list.len(), 1);
        let b = mutter_set_all_on(&bus, &fake.name, list, 50).unwrap();
        assert_eq!(b.value, backlight_raw(50, 0, 19393));
        assert_eq!(fake.calls_of("SetBacklight").len(), 4);
        assert!(mutter_set_all_on(&bus, &fake.name, Vec::new(), 50).is_err());
        // no backlight in mutter
        fake.state.lock().unwrap().backlight = None;
        assert!(mutter_backlight_on(&bus, &fake.name).is_none());
    }
}
