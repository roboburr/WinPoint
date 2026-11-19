//! The batteries of connected devices (GNOME's Power › Connected Devices, cc-power-panel.c): UPower's devices
//! except the computer's own (line power and the batteries that power it, which Power & battery shows), so mice,
//! keyboards, headsets, game controllers, pens and a UPS. Bluetooth & devices puts them on the device cards: a
//! Bluetooth device by its address (UPower's bluez backend names `/org/bluez/hciN/dev_XX_XX_..`, the HID batteries
//! `hid-xx:xx:..-battery`), any other one as a card of its own.
use crate::services::system::common::{call, first, getall, string, truth};
use serde_json::{Value, json};

const UPOWER_PATH: &str = "/org/freedesktop/UPower";
const UPOWER: &str = "org.freedesktop.UPower";
const DEVICE: &str = "org.freedesktop.UPower.Device";

/// UpDeviceKind (upower's up-types.h) as a name.
pub(super) fn kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "line-power",
        2 => "battery",
        3 => "ups",
        4 => "monitor",
        5 => "mouse",
        6 => "keyboard",
        7 => "pda",
        8 => "phone",
        9 => "media-player",
        10 => "tablet",
        11 => "computer",
        12 => "gaming-input",
        13 => "pen",
        14 => "touchpad",
        15 => "modem",
        16 => "network",
        17 => "headset",
        18 => "speakers",
        19 => "headphones",
        20 => "video",
        21 => "other-audio",
        22 => "remote-control",
        23 => "printer",
        24 => "scanner",
        25 => "camera",
        26 => "wearable",
        27 => "toy",
        28 => "bluetooth-generic",
        _ => "unknown",
    }
}

/// UpDeviceState as a name.
pub(super) fn state_name(state: u64) -> &'static str {
    match state {
        1 => "charging",
        2 => "discharging",
        3 => "empty",
        4 => "full",
        5 => "pending-charge",
        6 => "pending-discharge",
        _ => "unknown",
    }
}

/// UpDeviceLevel (a device that reports a coarse level instead of a percentage), None for "none" and "unknown".
pub(super) fn level_name(level: u64) -> Option<&'static str> {
    Some(match level {
        3 => "low",
        4 => "critical",
        6 => "normal",
        7 => "high",
        8 => "full",
        _ => return None,
    })
}

fn hex_pairs(text: &str, separator: char) -> Option<String> {
    let parts: Vec<&str> = text.split(separator).collect();
    (parts.len() == 6
        && parts
            .iter()
            .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit())))
    .then(|| parts.join(":").to_uppercase())
}

/// The Bluetooth address of a UPower device, from its native path (or its serial, which the HID backend sets to
/// the address).
pub(super) fn address_of(native_path: &str, serial: &str) -> Option<String> {
    if let Some(rest) = native_path
        .rsplit('/')
        .next()
        .and_then(|s| s.strip_prefix("dev_"))
    {
        return hex_pairs(rest, '_');
    }
    if let Some(rest) = native_path.strip_prefix("hid-") {
        let address = rest.strip_suffix("-battery").unwrap_or(rest);
        if let Some(a) = hex_pairs(address, ':') {
            return Some(a);
        }
    }
    hex_pairs(serial.trim(), ':')
}

/// One device of `devices.batteries`, None for the computer's own power (module docs) and for a device that is
/// not there.
pub(super) fn item(path: &str, d: &Value) -> Option<Value> {
    let kind = kind_name(d["Type"].as_u64().unwrap_or(0));
    if kind == "line-power" || (kind == "battery" && truth(&d["PowerSupply"])) {
        return None;
    }
    let percentage = d["Percentage"].as_f64().filter(|p| p.is_finite());
    let level = level_name(d["BatteryLevel"].as_u64().unwrap_or(0));
    if d["IsPresent"] == false && percentage.unwrap_or(0.0) <= 0.0 && level.is_none() {
        return None;
    }
    let native = string(&d["NativePath"]);
    let id = path.rsplit('/').next().unwrap_or(path);
    Some(json!({
        "id": id,
        "path": path,
        "kind": kind,
        "model": string(&d["Model"]).trim(),
        "vendor": string(&d["Vendor"]).trim(),
        "percentage": percentage.map(|p| p.clamp(0.0, 100.0).round() as i64),
        "level": level,
        "state": state_name(d["State"].as_u64().unwrap_or(0)),
        "icon": string(&d["IconName"]),
        "address": address_of(&native, &string(&d["Serial"])),
        "nativePath": native,
    }))
}

/// `devices.batteries` without UPower.
pub(super) fn absent() -> Value {
    json!({"available": false, "devices": []})
}

/// `devices.batteries`: UPower's EnumerateDevices, each device's properties, [`item`] (sorted by kind and name).
pub(super) fn get(bus: &gio::DBusConnection, name: &str) -> Value {
    let Ok(paths) = call(
        bus,
        name,
        UPOWER_PATH,
        UPOWER,
        "EnumerateDevices",
        None,
        2000,
    ) else {
        return absent();
    };
    let mut devices: Vec<Value> = first(&paths)
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| p.as_str())
        .filter_map(|p| item(p, &getall(bus, name, p, DEVICE).ok()?))
        .collect();
    devices.sort_by(|a, b| {
        string(&a["kind"]).cmp(&string(&b["kind"])).then_with(|| {
            string(&a["model"])
                .to_lowercase()
                .cmp(&string(&b["model"]).to_lowercase())
        })
    });
    json!({"available": true, "devices": devices})
}
