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
pub(super) fn brightness_get() -> Value {
    let none = || json!({"available":false,"value":null});
    let Some((name, p, max)) = backlight() else {
        return none();
    };
    let Some(cur) = read(p.join("brightness")).and_then(|s| s.parse::<i64>().ok()) else {
        return none();
    };
    json!({"available":true,"value":round(cur as f64*100./max as f64,0)as i64,"device":name})
}
pub(super) fn brightness_set(args: &Value, lang: crate::i18n::Lang) -> ApiResult {
    let (name, _, max) = backlight().ok_or_else(|| {
        ApiError::tr(
            "unsupported",
            lang,
            "system.brightness.unsupported",
            crate::i18n::NO_PARAMS,
        )
    })?;
    let v = if !args["delta"].is_null() {
        round(
            number(&brightness_get()["value"]).unwrap_or(0.) + number_arg(&args["delta"], lang)?,
            0,
        )
        .clamp(0., 100.) as i64
    } else if !args["value"].is_null() {
        pct_arg(&args["value"], lang)?
    } else {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "system.brightness.missing",
            crate::i18n::NO_PARAMS,
        ));
    };
    let raw = round(max as f64 * v as f64 / 100., 0).max(1.) as u32;
    let bus = system().map_err(|_| {
        ApiError::tr(
            "failed",
            lang,
            "system.brightness.failed",
            crate::i18n::NO_PARAMS,
        )
    })?;
    call(
        &bus,
        "org.freedesktop.login1",
        &session_path(),
        "org.freedesktop.login1.Session",
        "SetBrightness",
        Some(&("backlight", name.as_str(), raw).to_variant()),
        3000,
    )
    .map_err(|_| {
        ApiError::tr(
            "failed",
            lang,
            "system.brightness.failed",
            crate::i18n::NO_PARAMS,
        )
    })?;
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
