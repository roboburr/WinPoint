//! NetworkManager on the system bus: profiles (`Settings`, `Settings.Connection`), devices, active connections and
//! the manager's switches (notes/claude/settings-parity.md §5.4). Reads use short timeouts; every write goes through
//! `common::call_interactive` (ALLOW_INTERACTIVE_AUTHORIZATION, §5.8), so GNOME Shell's polkit agent asks for the
//! password where NetworkManager's policy wants one (`settings.modify.system`, `network-control`, ...), as GNOME
//! Settings' libnm calls do.
use super::settings_map::{self as map, Groups};
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::{
    call, call_interactive, first, getall, object_path, variant_json,
};
use glib::Variant;
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(crate) const NM: &str = "org.freedesktop.NetworkManager";
pub(crate) const PATH: &str = "/org/freedesktop/NetworkManager";
pub(crate) const SETTINGS: &str = "/org/freedesktop/NetworkManager/Settings";
const PROPS: &str = "org.freedesktop.DBus.Properties";

fn iface(name: &str) -> String {
    format!("{NM}.{name}")
}

/// Update2/AddConnection2 flag: save to disk (NM_SETTINGS_UPDATE2_FLAG_TO_DISK, NM_SETTINGS_ADD_CONNECTION2_FLAG_TO_DISK).
const TO_DISK: u32 = 0x1;

pub(crate) fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr("unsupported", lang, "net.unavailable", NO_PARAMS)
}

pub(crate) fn not_found(lang: Lang) -> ApiError {
    ApiError::tr("not_found", lang, "net.notFound", NO_PARAMS)
}

pub(crate) fn no_device(lang: Lang) -> ApiError {
    ApiError::tr("not_found", lang, "net.noDevice", NO_PARAMS)
}

/// The system bus with NetworkManager on it.
pub(crate) fn bus(lang: Lang) -> Result<gio::DBusConnection, ApiError> {
    let bus = crate::services::system::common::system().map_err(|_| unavailable(lang))?;
    getall(&bus, NM, PATH, NM).map_err(|_| unavailable(lang))?;
    Ok(bus)
}

/// The profile path of a uuid (Settings.GetConnectionByUuid).
pub(crate) fn path_of(
    bus: &gio::DBusConnection,
    uuid: &str,
    lang: Lang,
) -> Result<String, ApiError> {
    let r = call(
        bus,
        NM,
        SETTINGS,
        &iface("Settings"),
        "GetConnectionByUuid",
        Some(&(uuid,).to_variant()),
        3000,
    )
    .map_err(|_| not_found(lang))?;
    first(&r)
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| not_found(lang))
}

/// GetSettings of a profile.
pub(crate) fn settings_of(bus: &gio::DBusConnection, path: &str) -> Result<Groups, String> {
    let r = call(
        bus,
        NM,
        path,
        &iface("Settings.Connection"),
        "GetSettings",
        None,
        5000,
    )?;
    Ok(map::groups_of(&r.child_value(0)))
}

/// GetSecrets of one setting group (interactive: the policy may ask for the password).
pub(crate) fn secrets_of(
    bus: &gio::DBusConnection,
    path: &str,
    group: &str,
    lang: Lang,
) -> Result<Groups, ApiError> {
    let r = call_interactive(
        bus,
        NM,
        path,
        &iface("Settings.Connection"),
        "GetSecrets",
        Some(&(group,).to_variant()),
        lang,
    )?;
    Ok(map::groups_of(&r.child_value(0)))
}

/// Every profile path.
pub(crate) fn list(bus: &gio::DBusConnection) -> Result<Vec<String>, String> {
    let r = call(
        bus,
        NM,
        SETTINGS,
        &iface("Settings"),
        "ListConnections",
        None,
        3000,
    )?;
    Ok(first(&r)
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

/// The active connections: (active path, its properties).
pub(crate) fn actives(bus: &gio::DBusConnection) -> Vec<(String, Value)> {
    let Ok(root) = getall(bus, NM, PATH, NM) else {
        return Vec::new();
    };
    root["ActiveConnections"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|p| p.as_str())
                .filter_map(|p| {
                    getall(bus, NM, p, &iface("Connection.Active"))
                        .ok()
                        .map(|v| (p.to_owned(), v))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The aliases a page may use for NM's connection types.
pub(crate) fn type_name(t: &str) -> &str {
    match t {
        "wifi" => "802-11-wireless",
        "ethernet" => "802-3-ethernet",
        other => other,
    }
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The summary of one profile (net.connections): what the lists show without a second call.
pub(crate) fn summary(
    path: &str,
    settings: &Value,
    active: Option<&(String, Value)>,
    ifaces: &BTreeMap<String, String>,
) -> Value {
    let c = &settings["connection"];
    let w = &settings["802-11-wireless"];
    let mut out = json!({
        "uuid": c["uuid"], "id": c["id"], "type": c["type"], "path": path,
        "autoconnect": c["autoconnect"] != false,
        "timestamp": c["timestamp"].as_u64().unwrap_or(0),
        "interface": c["interface-name"].as_str().unwrap_or(""),
        "metered": c["metered"].as_i64().unwrap_or(0),
        "permissions": c["permissions"].as_array().cloned().unwrap_or_default(),
        "shared": c["permissions"].as_array().is_none_or(|a| a.is_empty()),
        "active": active.is_some(),
    });
    if w.is_object() {
        out["ssid"] = json!(map::ssid_text(&w["ssid"]));
        out["hidden"] = json!(w["hidden"] == true);
        out["mode"] = json!(w["mode"].as_str().unwrap_or("infrastructure"));
        out["keyMgmt"] = json!(s(&settings["802-11-wireless-security"]["key-mgmt"]));
        out["eap"] = settings["802-1x"]["eap"].clone();
    }
    if settings["bluetooth"].is_object() {
        out["bluetoothType"] = settings["bluetooth"]["type"].clone();
    }
    if settings["vpn"].is_object() {
        out["vpnType"] = settings["vpn"]["service-type"].clone();
    }
    if let Some((apath, a)) = active {
        out["activePath"] = json!(apath);
        out["state"] = json!(active_state(a["State"].as_u64().unwrap_or(0)));
        let devices: Vec<Value> = a["Devices"]
            .as_array()
            .map(|d| {
                d.iter()
                    .filter_map(|p| p.as_str().and_then(|p| ifaces.get(p)).map(|i| json!(i)))
                    .collect()
            })
            .unwrap_or_default();
        out["device"] = devices.first().cloned().unwrap_or(json!(""));
    }
    out
}

/// NMActiveConnectionState as text.
pub(crate) fn active_state(n: u64) -> &'static str {
    match n {
        1 => "activating",
        2 => "activated",
        3 => "deactivating",
        4 => "deactivated",
        _ => "unknown",
    }
}

/// net.connections {type?}: every profile, the active ones first, then the last used.
pub(crate) fn connections(
    bus: &gio::DBusConnection,
    filter: Option<&str>,
) -> Result<Value, String> {
    let actives = actives(bus);
    let ifaces = device_ifaces(bus);
    let mut out = Vec::new();
    for path in list(bus)? {
        let Ok(groups) = settings_of(bus, &path) else {
            continue;
        };
        let j = map::to_json(&groups);
        let t = s(&j["connection"]["type"]);
        if filter.is_some_and(|f| type_name(f) != t) {
            continue;
        }
        let active = actives.iter().find(|(_, a)| a["Connection"] == json!(path));
        out.push(summary(&path, &j, active, &ifaces));
    }
    out.sort_by(|a, b| {
        b["active"]
            .as_bool()
            .cmp(&a["active"].as_bool())
            .then(b["timestamp"].as_u64().cmp(&a["timestamp"].as_u64()))
            .then(s(&a["id"]).to_lowercase().cmp(&s(&b["id"]).to_lowercase()))
    });
    Ok(json!({ "connections": out }))
}

/// Device path → interface name.
pub(crate) fn device_ifaces(bus: &gio::DBusConnection) -> BTreeMap<String, String> {
    device_paths(bus)
        .into_iter()
        .filter_map(|p| {
            getall(bus, NM, &p, &iface("Device"))
                .ok()
                .map(|d| (p, s(&d["Interface"]).to_owned()))
        })
        .collect()
}

pub(crate) fn device_paths(bus: &gio::DBusConnection) -> Vec<String> {
    call(bus, NM, PATH, NM, "GetDevices", None, 3000)
        .ok()
        .and_then(|r| {
            first(&r).as_array().map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// The device path of an interface name.
pub(crate) fn device_of(bus: &gio::DBusConnection, name: &str) -> Option<String> {
    device_paths(bus)
        .into_iter()
        .find(|p| getall(bus, NM, p, &iface("Device")).is_ok_and(|d| d["Interface"] == name))
}

/// net.connection.get {uuid}: every key NM has for the profile, as JSON.
pub(crate) fn get(bus: &gio::DBusConnection, uuid: &str, lang: Lang) -> Result<Value, ApiError> {
    let path = path_of(bus, uuid, lang)?;
    let groups = settings_of(bus, &path).map_err(|_| not_found(lang))?;
    let props = getall(bus, NM, &path, &iface("Settings.Connection")).unwrap_or(json!({}));
    let actives = actives(bus);
    let active = actives.iter().find(|(_, a)| a["Connection"] == json!(path));
    let j = map::to_json(&groups);
    Ok(json!({
        "uuid": uuid,
        "path": path,
        "settings": j,
        "summary": summary(&path, &j, active, &device_ifaces(bus)),
        "unsaved": props["Unsaved"] == true,
        "filename": props["Filename"].as_str().unwrap_or(""),
    }))
}

/// The base of an update: NM's settings, plus every secret NM keeps for the profile when the change sets a secret
/// (NetworkManager keeps the stored secrets only for an update without any; nm-settings-connection.c).
pub(crate) fn base_for_update(
    bus: &gio::DBusConnection,
    path: &str,
    changes: &Value,
    lang: Lang,
) -> Result<Groups, ApiError> {
    let mut base = settings_of(bus, path).map_err(|_| not_found(lang))?;
    if map::has_secrets(changes) {
        let groups: Vec<String> = base
            .keys()
            .filter(|g| map::SECRET_GROUPS.contains(&g.as_str()))
            .cloned()
            .collect();
        for g in groups {
            match secrets_of(bus, path, &g, lang) {
                Ok(secrets) => map::add_secrets(&mut base, &secrets),
                // no secrets stored for this group (NoSecrets), or the agent has none: nothing to keep
                Err(e) if e.code == "failed" => {}
                Err(e) => return Err(e),
            }
        }
    }
    Ok(base)
}

/// The arguments of Update2 and AddConnection2: `(a{sa{sv}} settings, u flags, a{sv} args)`, the settings as the
/// dictionary itself (a glib::Variant in the tuple would become a `v`, which NetworkManager refuses).
pub(crate) fn settings_args(groups: &Groups) -> Variant {
    (groups.clone(), TO_DISK, BTreeMap::<String, Variant>::new()).to_variant()
}

/// Update2 (to disk) of a merged profile.
pub(crate) fn update2(
    bus: &gio::DBusConnection,
    path: &str,
    groups: &Groups,
    lang: Lang,
) -> Result<(), ApiError> {
    let args = settings_args(groups);
    call_interactive(
        bus,
        NM,
        path,
        &iface("Settings.Connection"),
        "Update2",
        Some(&args),
        lang,
    )
    .map(|_| ())
}

/// AddConnection2 (to disk): the new profile's path.
pub(crate) fn add(
    bus: &gio::DBusConnection,
    groups: &Groups,
    lang: Lang,
) -> Result<String, ApiError> {
    let args = settings_args(groups);
    let r = call_interactive(
        bus,
        NM,
        SETTINGS,
        &iface("Settings"),
        "AddConnection2",
        Some(&args),
        lang,
    )?;
    Ok(variant_json(&r)[0].as_str().unwrap_or("").to_owned())
}

pub(crate) fn delete(bus: &gio::DBusConnection, path: &str, lang: Lang) -> Result<(), ApiError> {
    call_interactive(
        bus,
        NM,
        path,
        &iface("Settings.Connection"),
        "Delete",
        None,
        lang,
    )
    .map(|_| ())
}

/// ActivateConnection(profile, device, "/"): "/" for either lets NetworkManager choose.
pub(crate) fn activate(
    bus: &gio::DBusConnection,
    conn: &str,
    device: &str,
    lang: Lang,
) -> Result<String, ApiError> {
    let bad = |_| ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS);
    let args = (
        object_path(conn).map_err(bad)?,
        object_path(device).map_err(bad)?,
        object_path("/").map_err(bad)?,
    )
        .to_variant();
    let r = call_interactive(bus, NM, PATH, NM, "ActivateConnection", Some(&args), lang)?;
    Ok(first(&r).as_str().unwrap_or("").to_owned())
}

pub(crate) fn deactivate(
    bus: &gio::DBusConnection,
    active: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    let path = object_path(active).map_err(|_| not_found(lang))?;
    call_interactive(
        bus,
        NM,
        PATH,
        NM,
        "DeactivateConnection",
        Some(&(path,).to_variant()),
        lang,
    )
    .map(|_| ())
}

/// Device.Reapply with the profile as it is now (an empty dict: the device's settings-connection), so an IP, DNS or
/// metered change takes effect on the active connection; false when the device cannot apply it without
/// reconnecting.
pub(crate) fn reapply(bus: &gio::DBusConnection, device: &str, lang: Lang) -> bool {
    let args = (Groups::new(), 0u64, 0u32).to_variant();
    call_interactive(
        bus,
        NM,
        device,
        &iface("Device"),
        "Reapply",
        Some(&args),
        lang,
    )
    .is_ok()
}

pub(crate) fn disconnect(
    bus: &gio::DBusConnection,
    device: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    call_interactive(bus, NM, device, &iface("Device"), "Disconnect", None, lang).map(|_| ())
}

/// The arguments of AddAndActivateConnection2: `(a{sa{sv}}, o device, o specific object, a{sv} options)`.
pub(crate) fn activate_args(
    groups: &Groups,
    device: glib::variant::ObjectPath,
    specific: glib::variant::ObjectPath,
    options: BTreeMap<String, Variant>,
) -> Variant {
    (groups.clone(), device, specific, options).to_variant()
}

/// AddAndActivateConnection2 (persist on disk): (profile path, active path).
pub(crate) fn add_and_activate(
    bus: &gio::DBusConnection,
    groups: &Groups,
    device: &str,
    lang: Lang,
) -> Result<(String, String), ApiError> {
    let dev = object_path(device).map_err(|_| no_device(lang))?;
    let specific = object_path("/").map_err(|_| no_device(lang))?;
    let options = BTreeMap::from([("persist".to_owned(), "disk".to_variant())]);
    let args = activate_args(groups, dev, specific, options);
    let r = call_interactive(
        bus,
        NM,
        PATH,
        NM,
        "AddAndActivateConnection2",
        Some(&args),
        lang,
    )?;
    let j = variant_json(&r);
    Ok((s(&j[0]).to_owned(), s(&j[1]).to_owned()))
}

/// Properties.Set of a NetworkManager switch (WirelessEnabled, WwanEnabled, ConnectivityCheckEnabled).
pub(crate) fn set_switch(
    bus: &gio::DBusConnection,
    prop: &str,
    on: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    let args = (NM, prop, on.to_variant()).to_variant();
    call_interactive(bus, NM, PATH, PROPS, "Set", Some(&args), lang).map(|_| ())
}

// ------------------------------------------------------------------------------------------------- devices

/// NMDeviceType as the page's name.
pub(crate) fn device_type(n: u64) -> &'static str {
    match n {
        1 => "ethernet",
        2 => "wifi",
        5 => "bluetooth",
        8 => "modem",
        10 => "bond",
        11 => "vlan",
        13 => "bridge",
        14 => "generic",
        16 => "tun",
        20 => "veth",
        29 => "wireguard",
        30 => "wifi-p2p",
        32 => "loopback",
        _ => "other",
    }
}

/// NMDeviceState as the page's state (`unplugged`: a wired device without a cable).
pub(crate) fn device_state(n: u64, carrier: Option<bool>) -> &'static str {
    match n {
        10 => "unmanaged",
        20 if carrier == Some(false) => "unplugged",
        20 => "unavailable",
        30 => "disconnected",
        40..=90 => "connecting",
        100 => "connected",
        110 => "deactivating",
        120 => "failed",
        _ => "unknown",
    }
}

/// The Wi-Fi band of a frequency in MHz.
pub(crate) fn band(mhz: u64) -> &'static str {
    match mhz {
        2400..=2500 => "2.4",
        4900..=5899 => "5",
        5925..=7125 => "6",
        _ => "",
    }
}

/// The Wi-Fi channel of a frequency in MHz (IEEE 802.11: 2.4 GHz from 2407, channel 14 at 2484; 5 GHz from 5000;
/// 6 GHz from 5950).
pub(crate) fn channel(mhz: u64) -> u64 {
    match mhz {
        2484 => 14,
        2412..=2472 => (mhz - 2407) / 5,
        5000..=5899 => (mhz - 5000) / 5,
        5955..=7115 => (mhz - 5950) / 5,
        _ => 0,
    }
}

/// The security of an access point from its flags (as system.network's, plus the NM 802.1X bits).
pub(crate) fn ap_security(flags: u64, wpa: u64, rsn: u64) -> &'static str {
    let k = wpa | rsn;
    if k & (0x200 | 0x2000) != 0 {
        "wpa-eap"
    } else if k & 0x400 != 0 && k & 0x100 == 0 {
        "sae"
    } else if k & 0x100 != 0 {
        "wpa-psk"
    } else if k & (0x800 | 0x1000) != 0 {
        "owe"
    } else if flags & 1 != 0 {
        "wep"
    } else {
        "open"
    }
}

/// The vendor and model udev knows for an interface (ID_VENDOR_FROM_DATABASE / ID_MODEL_FROM_DATABASE, as libnm).
pub(crate) fn udev_names(
    iface_name: &str,
    sys: &std::path::Path,
    udev: &std::path::Path,
) -> (String, String) {
    if iface_name.is_empty() || iface_name.contains('/') || iface_name.starts_with('.') {
        return (String::new(), String::new());
    }
    let index = std::fs::read_to_string(sys.join(iface_name).join("ifindex")).unwrap_or_default();
    let index = index.trim();
    if index.is_empty() || !index.bytes().all(|b| b.is_ascii_digit()) {
        return (String::new(), String::new());
    }
    let data = std::fs::read_to_string(udev.join(format!("n{index}"))).unwrap_or_default();
    let get = |key: &str| {
        data.lines()
            .find_map(|l| {
                l.strip_prefix("E:")
                    .and_then(|l| l.strip_prefix(key))
                    .and_then(|l| l.strip_prefix('='))
            })
            .unwrap_or("")
            .to_owned()
    };
    (
        get("ID_VENDOR_FROM_DATABASE"),
        get("ID_MODEL_FROM_DATABASE"),
    )
}

fn ip_config(bus: &gio::DBusConnection, path: &str, six: bool) -> Value {
    if path.is_empty() || path == "/" {
        return Value::Null;
    }
    let Ok(c) = getall(
        bus,
        NM,
        path,
        &iface(if six { "IP6Config" } else { "IP4Config" }),
    ) else {
        return Value::Null;
    };
    let addresses: Vec<Value> = c["AddressData"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| json!({ "address": x["address"], "prefix": x["prefix"] }))
                .collect()
        })
        .unwrap_or_default();
    let dns: Vec<Value> = if let Some(data) = c["NameserverData"].as_array() {
        data.iter()
            .filter_map(|x| x["address"].as_str().map(|a| json!(a)))
            .collect()
    } else if six {
        c["Nameservers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|b| ipv6_text(b))
                    .map(|t| json!(t))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        c["Nameservers"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|n| n.as_u64())
                    .map(|n| json!(ipv4_text(n as u32)))
                    .collect()
            })
            .unwrap_or_default()
    };
    json!({ "addresses": addresses, "gateway": c["Gateway"].as_str().unwrap_or(""), "dns": dns, "domains": c["Domains"] })
}

/// An IPv4 address NM sends as a network-order u32.
pub(crate) fn ipv4_text(n: u32) -> String {
    std::net::Ipv4Addr::from(n.to_ne_bytes()).to_string()
}

fn ipv6_text(v: &Value) -> Option<String> {
    let bytes: Vec<u8> = v
        .as_array()?
        .iter()
        .filter_map(|b| b.as_u64().map(|b| b as u8))
        .collect();
    let arr: [u8; 16] = bytes.try_into().ok()?;
    Some(std::net::Ipv6Addr::from(arr).to_string())
}

fn ap_json(bus: &gio::DBusConnection, path: &str, active: &str) -> Option<Value> {
    if path.is_empty() || path == "/" {
        return None;
    }
    let a = getall(bus, NM, path, &iface("AccessPoint")).ok()?;
    let freq = a["Frequency"].as_u64().unwrap_or(0);
    Some(json!({
        "path": path,
        "ssid": map::ssid_text(&a["Ssid"]),
        "bssid": a["HwAddress"].as_str().unwrap_or(""),
        "frequency": freq, "band": band(freq), "channel": channel(freq),
        "strength": a["Strength"].as_u64().unwrap_or(0),
        "maxBitrate": a["MaxBitrate"].as_u64().unwrap_or(0),
        "bandwidth": a["Bandwidth"].as_u64().unwrap_or(0),
        "security": ap_security(a["Flags"].as_u64().unwrap_or(0), a["WpaFlags"].as_u64().unwrap_or(0), a["RsnFlags"].as_u64().unwrap_or(0)),
        "active": path == active,
    }))
}

/// One device as the pages show it.
pub(crate) fn device_json(
    bus: &gio::DBusConnection,
    path: &str,
    with_aps: bool,
    uuids: &BTreeMap<String, String>,
) -> Option<Value> {
    let d = getall(bus, NM, path, &iface("Device")).ok()?;
    let code = d["DeviceType"].as_u64().unwrap_or(0);
    let kind = device_type(code);
    let name = s(&d["Interface"]).to_owned();
    let (vendor, model) = udev_names(
        &name,
        std::path::Path::new("/sys/class/net"),
        std::path::Path::new("/run/udev/data"),
    );
    let mut out = json!({
        "path": path, "iface": name, "type": kind, "typeCode": code,
        "stateCode": d["State"].as_u64().unwrap_or(0),
        "managed": d["Managed"] != false, "autoconnect": d["Autoconnect"] != false,
        "driver": s(&d["Driver"]), "driverVersion": s(&d["DriverVersion"]), "firmware": s(&d["FirmwareVersion"]),
        "hwAddress": s(&d["HwAddress"]), "mtu": d["Mtu"].as_u64().unwrap_or(0),
        "vendor": vendor, "model": model,
        "available": d["AvailableConnections"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().and_then(|p| uuids.get(p)).map(|u| json!(u))).collect::<Vec<_>>()).unwrap_or_default(),
        "ip4": ip_config(bus, s(&d["Ip4Config"]), false),
        "ip6": ip_config(bus, s(&d["Ip6Config"]), true),
        "active": Value::Null,
    });
    let mut carrier = None;
    match kind {
        "ethernet" => {
            if let Ok(w) = getall(bus, NM, path, &iface("Device.Wired")) {
                carrier = w["Carrier"].as_bool();
                out["speed"] = json!(w["Speed"].as_u64().unwrap_or(0));
                out["permHwAddress"] = json!(s(&w["PermHwAddress"]));
                out["carrier"] = json!(carrier);
            }
        }
        "wifi" => {
            if let Ok(w) = getall(bus, NM, path, &iface("Device.Wireless")) {
                let caps = w["WirelessCapabilities"].as_u64().unwrap_or(0);
                let active_ap = s(&w["ActiveAccessPoint"]).to_owned();
                out["bitrate"] = json!(w["Bitrate"].as_u64().unwrap_or(0));
                out["permHwAddress"] = json!(s(&w["PermHwAddress"]));
                out["bands"] = json!({ "2.4": caps & 0x200 != 0, "5": caps & 0x400 != 0 });
                out["ap"] = ap_json(bus, &active_ap, &active_ap).unwrap_or(Value::Null);
                if with_aps {
                    let aps: Vec<Value> = w["AccessPoints"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|p| p.as_str())
                                .filter_map(|p| ap_json(bus, p, &active_ap))
                                .collect()
                        })
                        .unwrap_or_default();
                    out["accessPoints"] = json!(aps);
                }
            }
        }
        _ => {}
    }
    out["state"] = json!(device_state(d["State"].as_u64().unwrap_or(0), carrier));
    let ac = s(&d["ActiveConnection"]);
    if !ac.is_empty()
        && ac != "/"
        && let Ok(a) = getall(bus, NM, ac, &iface("Connection.Active"))
    {
        out["active"] = json!({
            "path": ac, "uuid": a["Uuid"], "id": a["Id"], "type": a["Type"],
            "state": active_state(a["State"].as_u64().unwrap_or(0)),
            "default": a["Default"] == true || a["Default6"] == true,
        });
    }
    Some(out)
}

/// net.devices {accessPoints?}: the devices (no loopback) and the manager's switches.
pub(crate) fn devices(bus: &gio::DBusConnection, with_aps: bool) -> Result<Value, String> {
    let root = getall(bus, NM, PATH, NM)?;
    let uuids: BTreeMap<String, String> = list(bus)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            settings_of(bus, &p)
                .ok()
                .and_then(|g| {
                    g.get("connection")
                        .and_then(|c| c.get("uuid"))
                        .and_then(|u| u.str().map(str::to_owned))
                })
                .map(|u| (p, u))
        })
        .collect();
    let list: Vec<Value> = device_paths(bus)
        .iter()
        .filter_map(|p| device_json(bus, p, with_aps, &uuids))
        .filter(|d| d["type"] != "loopback")
        .collect();
    let connectivity = match root["Connectivity"].as_u64().unwrap_or(0) {
        1 => "none",
        2 => "portal",
        3 => "limited",
        4 => "full",
        _ => "unknown",
    };
    Ok(json!({
        "devices": list,
        "networking": root["NetworkingEnabled"] != false,
        "wifiEnabled": root["WirelessEnabled"] == true,
        "wifiHardwareEnabled": root["WirelessHardwareEnabled"] != false,
        "wwanEnabled": root["WwanEnabled"] == true,
        "wwanHardwareEnabled": root["WwanHardwareEnabled"] != false,
        "connectivity": connectivity,
        "connectivityCheckEnabled": root["ConnectivityCheckEnabled"] == true,
        "connectivityCheckAvailable": root["ConnectivityCheckAvailable"] == true,
        "version": root["Version"].as_str().unwrap_or(""),
    }))
}

/// RequestScan on every Wi-Fi device (polkit wifi.scan; NetworkManager answers at once and scans in the background).
pub(crate) fn request_scan(bus: &gio::DBusConnection) {
    for p in device_paths(bus) {
        if getall(bus, NM, &p, &iface("Device")).is_ok_and(|d| d["DeviceType"] == 2) {
            let _ = call(
                bus,
                NM,
                &p,
                &iface("Device.Wireless"),
                "RequestScan",
                Some(&(BTreeMap::<String, Variant>::new(),).to_variant()),
                3000,
            );
        }
    }
}
