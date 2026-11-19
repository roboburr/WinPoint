//! Mobile hotspot (`net.hotspot.*`, notes/claude/settings-parity.md §5.4): GNOME Settings' Wi-Fi hotspot
//! (net-device-wifi.c `start_hotspot`, cc-wifi-hotspot-dialog.c) behind Windows 11's Mobile hotspot page.
//!
//! As GNOME: a hotspot is a Wi-Fi profile in mode `ap` (or `adhoc` on an adapter without AP mode) with IPv4
//! `shared`; a new one is called "Hotspot", never connects by itself, takes the computer's name and a random key of 8
//! letters and digits, and gets its security from the adapter (RSN: WPA2 with CCMP, WPA: WPA with TKIP, else a WEP
//! key); an existing one is changed in place (Update2 merge) and activated again. Windows' extra is the band (`a`,
//! `bg`, or none: NetworkManager chooses). Reading the key is GetSecrets (interactive), only when the page asks. The
//! random key a new hotspot would get is proposed once per host run (`proposed_key`), so the page shows one key.
use super::connections::{self as nm, NM};
use super::settings_map as map;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::getall;
use serde_json::{Value, json};

/// NMDeviceWifiCapabilities bits GNOME's hotspot reads.
pub(crate) const CAP_WPA: u64 = 0x10;
pub(crate) const CAP_RSN: u64 = 0x20;
pub(crate) const CAP_AP: u64 = 0x40;
const CAP_2GHZ: u64 = 0x200;
const CAP_5GHZ: u64 = 0x400;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// Whether a profile is a hotspot (gnome-control-center `is_hotspot_connection`): Wi-Fi in mode ap or adhoc, IPv4
/// shared.
pub(crate) fn is_hotspot(settings: &Value) -> bool {
    s(&settings["connection"]["type"]) == "802-11-wireless"
        && matches!(s(&settings["802-11-wireless"]["mode"]), "ap" | "adhoc")
        && s(&settings["ipv4"]["method"]) == "shared"
}

/// A band of the page: none (NetworkManager chooses), `a` (5 GHz) or `bg` (2.4 GHz).
pub(crate) fn band_ok(band: &str) -> bool {
    matches!(band, "" | "a" | "bg")
}

/// An SSID: 1 to 32 bytes.
pub(crate) fn ssid_ok(ssid: &str) -> bool {
    !ssid.is_empty() && ssid.len() <= 32
}

/// A key for the key management: WPA (wpa-psk, sae) 8-63 printable ASCII or 64 hex digits; WEP (none) 5 or 13
/// characters or 10 or 26 hex digits (libnm `nm_utils_wpa_psk_valid`, `nm_utils_wep_key_valid`).
pub(crate) fn password_ok(key_mgmt: &str, password: &str) -> bool {
    let printable = |p: &str| p.bytes().all(|b| (0x20..0x7f).contains(&b));
    let hex = |p: &str| p.bytes().all(|b| b.is_ascii_hexdigit());
    match key_mgmt {
        "none" => match password.len() {
            5 | 13 => printable(password),
            10 | 26 => hex(password),
            _ => false,
        },
        _ => match password.len() {
            8..=63 => printable(password),
            64 => hex(password),
            _ => false,
        },
    }
}

/// Random bytes of the kernel.
fn random_bytes(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut buf);
    }
    buf
}

/// GNOME's random key (cc-wifi-hotspot-dialog.c `get_random_wpa_key`: 8 letters and digits; `get_random_wep_key`:
/// 10 hex digits), from `bytes` (rejection sampling keeps the letters equally likely).
pub(crate) fn random_key(wep: bool, bytes: &[u8]) -> String {
    const ALNUM: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    const HEX: &[u8] = b"0123456789abcdef";
    let (set, len) = if wep { (HEX, 10) } else { (ALNUM, 8) };
    let limit = 256 - 256 % set.len();
    bytes
        .iter()
        .filter(|b| (**b as usize) < limit)
        .map(|b| set[*b as usize % set.len()] as char)
        .take(len)
        .collect()
}

fn new_key(wep: bool) -> String {
    loop {
        let key = random_key(wep, &random_bytes(64));
        if key.len() == if wep { 10 } else { 8 } {
            return key;
        }
    }
}

/// The key a new hotspot would get, the same one for as long as the host runs (one per kind: WPA, WEP): every
/// net.hotspot.get without a stored hotspot shows it, and net.hotspot.start without a password uses it. A fresh key
/// per call changed the Wachtwoord row and the QR code while the user was reading them (each NetworkManager signal
/// makes the page ask again), so the hotspot came up with another key than the one typed into a phone.
pub(crate) fn proposed_key(wep: bool) -> String {
    static PROPOSED: std::sync::Mutex<[Option<String>; 2]> = std::sync::Mutex::new([None, None]);
    let mut keys = PROPOSED.lock().unwrap_or_else(|e| e.into_inner());
    keys[usize::from(wep)]
        .get_or_insert_with(|| new_key(wep))
        .clone()
}

/// The key management of a new hotspot on an adapter with `caps`.
pub(crate) fn key_mgmt_for(caps: u64) -> &'static str {
    if caps & (CAP_RSN | CAP_WPA) != 0 {
        "wpa-psk"
    } else {
        "none"
    }
}

/// The security group of a new hotspot (GNOME: RSN → WPA2 with CCMP, WPA → WPA with TKIP, else a WEP key).
pub(crate) fn security_for(caps: u64, password: &str) -> Value {
    if caps & CAP_RSN != 0 {
        json!({ "key-mgmt": "wpa-psk", "proto": ["rsn"], "pairwise": ["ccmp"], "group": ["ccmp"], "psk": password })
    } else if caps & CAP_WPA != 0 {
        json!({ "key-mgmt": "wpa-psk", "proto": ["wpa"], "pairwise": ["tkip"], "group": ["tkip"], "psk": password })
    } else {
        json!({ "key-mgmt": "none", "wep-key-type": 1, "wep-key0": password })
    }
}

/// GNOME's new hotspot profile: "Hotspot", no autoconnect, mode ap (adhoc without AP mode), IPv4 shared, IPv6
/// ignored; Windows' band when one is chosen.
pub(crate) fn new_profile(
    uuid: &str,
    ssid: &str,
    password: &str,
    band: &str,
    caps: u64,
    iface: &str,
) -> Value {
    let mut wifi = json!({ "ssid": ssid, "mode": if caps & CAP_AP != 0 { "ap" } else { "adhoc" } });
    if !band.is_empty() {
        wifi["band"] = json!(band);
    }
    let mut connection =
        json!({ "id": "Hotspot", "uuid": uuid, "type": "802-11-wireless", "autoconnect": false });
    if !iface.is_empty() {
        connection["interface-name"] = json!(iface);
    }
    json!({
        "connection": connection,
        "802-11-wireless": wifi,
        "802-11-wireless-security": security_for(caps, password),
        "ipv4": { "method": "shared" },
        "ipv6": { "method": "ignore" },
    })
}

/// The changes to an existing hotspot (what the page gave: name, key for its key management, band; "" = any band).
pub(crate) fn profile_changes(
    ssid: Option<&str>,
    password: Option<&str>,
    band: Option<&str>,
    key_mgmt: &str,
) -> Value {
    let mut out = json!({});
    if let Some(ssid) = ssid {
        out["802-11-wireless"]["ssid"] = json!(ssid);
    }
    if let Some(band) = band {
        out["802-11-wireless"]["band"] = if band.is_empty() {
            Value::Null
        } else {
            json!(band)
        };
    }
    if let Some(pw) = password {
        let key = if key_mgmt == "none" {
            "wep-key0"
        } else {
            "psk"
        };
        out["802-11-wireless-security"][key] = json!(pw);
    }
    out
}

/// The hotspot's security as the page names it.
pub(crate) fn security_name(settings: &Value) -> &'static str {
    let sec = &settings["802-11-wireless-security"];
    match s(&sec["key-mgmt"]) {
        "" => "open",
        "none" => "wep",
        "sae" => "wpa3",
        "wpa-psk" => {
            let proto = sec["proto"].as_array();
            if proto.is_some_and(|p| p.len() == 1 && p[0] == "wpa") {
                "wpa"
            } else {
                "wpa2"
            }
        }
        _ => "other",
    }
}

/// The computer's name as a hotspot's default name (GNOME: the display hostname), at most 32 bytes.
pub(crate) fn default_ssid(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if out.len() + c.len_utf8() > 32 {
            break;
        }
        out.push(c);
    }
    if out.is_empty() {
        "Hotspot".to_owned()
    } else {
        out
    }
}

fn host_name(bus: &gio::DBusConnection) -> String {
    let pretty = getall(
        bus,
        "org.freedesktop.hostname1",
        "/org/freedesktop/hostname1",
        "org.freedesktop.hostname1",
    )
    .ok()
    .and_then(|p| p["PrettyHostname"].as_str().map(str::to_owned))
    .filter(|p| !p.trim().is_empty());
    default_ssid(&pretty.unwrap_or_else(|| glib::host_name().to_string()))
}

/// A Wi-Fi adapter for the hotspot.
pub(crate) struct Adapter {
    pub path: String,
    pub iface: String,
    pub caps: u64,
    pub active: String,
}

fn adapters(bus: &gio::DBusConnection) -> Vec<Adapter> {
    nm::device_paths(bus)
        .into_iter()
        .filter_map(|p| {
            let d = getall(bus, NM, &p, &format!("{NM}.Device")).ok()?;
            if d["DeviceType"] != 2 || d["Managed"] == false {
                return None;
            }
            let w = getall(bus, NM, &p, &format!("{NM}.Device.Wireless")).unwrap_or(json!({}));
            Some(Adapter {
                iface: s(&d["Interface"]).to_owned(),
                caps: w["WirelessCapabilities"].as_u64().unwrap_or(0),
                active: s(&d["ActiveConnection"]).to_owned(),
                path: p,
            })
        })
        .collect()
}

/// The hotspot profiles: (path, settings as JSON).
fn profiles(bus: &gio::DBusConnection) -> Vec<(String, Value)> {
    nm::list(bus)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let j = map::to_json(&nm::settings_of(bus, &p).ok()?);
            is_hotspot(&j).then_some((p, j))
        })
        .collect()
}

/// The hotspot profile to use: the active one, else the adapter's, else GNOME's "Hotspot", else the first.
fn pick<'a>(
    profiles: &'a [(String, Value)],
    actives: &[(String, Value)],
    iface: &str,
) -> Option<&'a (String, Value)> {
    profiles
        .iter()
        .find(|(p, _)| actives.iter().any(|(_, a)| a["Connection"] == json!(p)))
        .or_else(|| {
            profiles
                .iter()
                .find(|(_, j)| s(&j["connection"]["interface-name"]) == iface && !iface.is_empty())
        })
        .or_else(|| {
            profiles
                .iter()
                .find(|(_, j)| j["connection"]["id"] == "Hotspot")
        })
        .or_else(|| profiles.first())
}

/// The adapter: the named one, else the one the hotspot runs on, else the first with AP mode, else the first.
fn adapter<'a>(
    list: &'a [Adapter],
    iface: Option<&str>,
    hotspot_active: &[String],
) -> Option<&'a Adapter> {
    if let Some(name) = iface {
        return list.iter().find(|a| a.iface == name);
    }
    list.iter()
        .find(|a| hotspot_active.contains(&a.active))
        .or_else(|| list.iter().find(|a| a.caps & CAP_AP != 0))
        .or_else(|| list.first())
}

fn no_wifi(lang: Lang) -> ApiError {
    ApiError::tr("not_found", lang, "net.noWifi", NO_PARAMS)
}

pub(crate) fn bad_password(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "net.badPassword", NO_PARAMS)
}

/// net.hotspot.get {secrets?}: the hotspot as Windows' page shows it. Without a stored hotspot: the computer's name
/// and a fresh random key (what turning it on would use). The stored key only with `secrets` (GetSecrets).
pub(crate) fn get(bus: &gio::DBusConnection, secrets: bool, lang: Lang) -> ApiResult {
    let list = adapters(bus);
    let actives = nm::actives(bus);
    let all = profiles(bus);
    let active_paths: Vec<String> = actives
        .iter()
        .filter(|(_, a)| all.iter().any(|(p, _)| a["Connection"] == json!(p)))
        .map(|(ap, _)| ap.clone())
        .collect();
    let Some(dev) = adapter(&list, None, &active_paths) else {
        return Ok(json!({ "available": false, "active": false, "configured": false }));
    };
    let chosen = pick(&all, &actives, &dev.iface);
    let active =
        chosen.and_then(|(p, _)| actives.iter().find(|(_, a)| a["Connection"] == json!(p)));
    let state = active
        .map(|(_, a)| nm::active_state(a["State"].as_u64().unwrap_or(0)))
        .unwrap_or("");
    let mut out = json!({
        "available": true,
        "iface": dev.iface,
        "apCapable": dev.caps & CAP_AP != 0,
        "bands": { "2.4": dev.caps & CAP_2GHZ != 0, "5": dev.caps & CAP_5GHZ != 0 },
        "active": matches!(state, "activated" | "activating"),
        "state": state,
        "configured": chosen.is_some(),
    });
    match chosen {
        Some((path, j)) => {
            let w = &j["802-11-wireless"];
            let key_mgmt = s(&j["802-11-wireless-security"]["key-mgmt"]).to_owned();
            out["uuid"] = j["connection"]["uuid"].clone();
            out["ssid"] = json!(map::ssid_text(&w["ssid"]));
            out["band"] = json!(s(&w["band"]));
            out["keyMgmt"] = json!(key_mgmt);
            out["security"] = json!(security_name(j));
            out["password"] = Value::Null;
            if secrets && !key_mgmt.is_empty() {
                let sec = map::to_json(&nm::secrets_of(
                    bus,
                    path,
                    "802-11-wireless-security",
                    lang,
                )?);
                let key = if key_mgmt == "none" {
                    "wep-key0"
                } else {
                    "psk"
                };
                out["password"] = sec["802-11-wireless-security"][key].clone();
            }
        }
        None => {
            let key_mgmt = key_mgmt_for(dev.caps);
            out["uuid"] = Value::Null;
            out["ssid"] = json!(host_name(bus));
            out["band"] = json!("");
            out["keyMgmt"] = json!(key_mgmt);
            out["security"] = json!(security_name(
                &json!({ "802-11-wireless-security": security_for(dev.caps, "") })
            ));
            out["password"] = json!(proposed_key(key_mgmt == "none"));
        }
    }
    out["qr"] = match out["password"].as_str() {
        Some(pw) => json!(map::wifi_qr(
            s(&out["ssid"]),
            map::qr_security(s(&out["keyMgmt"])),
            Some(pw),
            false
        )),
        None => Value::Null,
    };
    // Windows' "Mijn internetverbinding delen vanaf": the primary connection when it is not the hotspot
    let root = getall(bus, NM, nm::PATH, NM).unwrap_or(json!({}));
    let primary = s(&root["PrimaryConnection"]).to_owned();
    out["uplink"] = Value::Null;
    out["sameAdapter"] = json!(false);
    if let Some((_, a)) = actives.iter().find(|(p, _)| *p == primary)
        && !active_paths.contains(&primary)
    {
        let ifaces = nm::device_ifaces(bus);
        let device = a["Devices"]
            .as_array()
            .and_then(|d| d.first())
            .and_then(|d| d.as_str())
            .and_then(|d| ifaces.get(d))
            .cloned()
            .unwrap_or_default();
        out["sameAdapter"] = json!(device == dev.iface);
        out["uplink"] = json!({ "id": a["Id"], "type": a["Type"], "iface": device });
    }
    Ok(out)
}

/// The checked arguments of net.hotspot.start.
pub(crate) struct Start {
    pub ssid: Option<String>,
    pub password: Option<String>,
    pub band: Option<String>,
    pub iface: Option<String>,
    pub activate: bool,
}

/// Check net.hotspot.start's arguments (null or absent: keep the stored value, or GNOME's default for a new
/// hotspot). A key must fit WPA or WEP here; the exact check against the profile's key management follows.
pub(crate) fn start_args(args: &Value, lang: Lang) -> Result<Start, ApiError> {
    let bad = || ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS);
    let text = |v: &Value| -> Result<Option<String>, ApiError> {
        match v {
            Value::Null => Ok(None),
            Value::String(x) => Ok(Some(x.clone())),
            _ => Err(bad()),
        }
    };
    let ssid = text(&args["ssid"])?;
    if ssid.as_deref().is_some_and(|x| !ssid_ok(x)) {
        return Err(ApiError::tr("bad_args", lang, "net.badSsid", NO_PARAMS));
    }
    let password = text(&args["password"])?;
    if password
        .as_deref()
        .is_some_and(|p| !password_ok("wpa-psk", p) && !password_ok("none", p))
    {
        return Err(bad_password(lang));
    }
    let band = text(&args["band"])?;
    if band.as_deref().is_some_and(|b| !band_ok(b)) {
        return Err(bad());
    }
    let iface = match &args["iface"] {
        Value::Null => None,
        Value::String(x) if super::iface_ok(x) => Some(x.clone()),
        _ => return Err(bad()),
    };
    let activate = match &args["activate"] {
        Value::Null => true,
        Value::Bool(b) => *b,
        _ => return Err(bad()),
    };
    Ok(Start {
        ssid,
        password,
        band,
        iface,
        activate,
    })
}

/// net.hotspot.start: change (or make) the hotspot profile and, unless `activate` is false, turn it on.
pub(crate) fn start(bus: &gio::DBusConnection, a: &Start, uuid: &str, lang: Lang) -> ApiResult {
    let list = adapters(bus);
    let actives = nm::actives(bus);
    let all = profiles(bus);
    let active_paths: Vec<String> = actives
        .iter()
        .filter(|(_, x)| all.iter().any(|(p, _)| x["Connection"] == json!(p)))
        .map(|(ap, _)| ap.clone())
        .collect();
    let dev = adapter(&list, a.iface.as_deref(), &active_paths).ok_or_else(|| {
        if a.iface.is_some() {
            nm::no_device(lang)
        } else {
            no_wifi(lang)
        }
    })?;
    match pick(&all, &actives, &dev.iface) {
        Some((path, j)) => {
            let key_mgmt = s(&j["802-11-wireless-security"]["key-mgmt"]);
            if a.password
                .as_deref()
                .is_some_and(|p| !password_ok(key_mgmt, p))
            {
                return Err(bad_password(lang));
            }
            let changes = profile_changes(
                a.ssid.as_deref(),
                a.password.as_deref(),
                a.band.as_deref(),
                key_mgmt,
            );
            if changes.as_object().is_some_and(|o| !o.is_empty()) {
                let mut base = nm::base_for_update(bus, path, &changes, lang)?;
                map::merge(&mut base, &changes).map_err(|e| super::map_error(e, lang))?;
                nm::update2(bus, path, &base, lang)?;
            }
            if a.activate {
                nm::activate(bus, path, &dev.path, lang)?;
            }
            let j = map::to_json(&nm::settings_of(bus, path).unwrap_or_default());
            Ok(json!({
                "uuid": j["connection"]["uuid"], "ssid": map::ssid_text(&j["802-11-wireless"]["ssid"]),
                "band": s(&j["802-11-wireless"]["band"]), "active": a.activate, "iface": dev.iface,
            }))
        }
        None => {
            let key_mgmt = key_mgmt_for(dev.caps);
            let password = match &a.password {
                Some(p) if !password_ok(key_mgmt, p) => return Err(bad_password(lang)),
                Some(p) => p.clone(),
                None => proposed_key(key_mgmt == "none"),
            };
            let ssid = a.ssid.clone().unwrap_or_else(|| host_name(bus));
            let band = a.band.clone().unwrap_or_default();
            let settings = new_profile(uuid, &ssid, &password, &band, dev.caps, "");
            let groups = map::build(&settings).map_err(|e| super::map_error(e, lang))?;
            if a.activate {
                nm::add_and_activate(bus, &groups, &dev.path, lang)?;
            } else {
                nm::add(bus, &groups, lang)?;
            }
            Ok(
                json!({ "uuid": uuid, "ssid": ssid, "band": band, "active": a.activate, "iface": dev.iface }),
            )
        }
    }
}

/// net.hotspot.stop: deactivate every active hotspot (GNOME's Turn Off Hotspot); nothing to do when none runs.
pub(crate) fn stop(bus: &gio::DBusConnection, lang: Lang) -> ApiResult {
    let all = profiles(bus);
    let mut stopped = 0;
    for (active, a) in nm::actives(bus) {
        if all.iter().any(|(p, _)| a["Connection"] == json!(p)) {
            nm::deactivate(bus, &active, lang)?;
            stopped += 1;
        }
    }
    Ok(json!({ "active": false, "stopped": stopped }))
}
