//! Mobile broadband (`net.modem.*`, notes/claude/settings-parity.md §5.4): ModemManager on the system bus, as GNOME
//! Settings' Mobile Network panel (cc-wwan-device.c, cc-wwan-data.c): the modems with their SIMs, state, lock and
//! retries, network type, signal, modes, operator and details; SIM unlock (PIN, PUK), the SIM PIN lock, the network
//! mode, the operator scan and registration, the primary SIM slot. Every ModemManager write goes through
//! `common::call_interactive` (polkit `org.freedesktop.ModemManager1.Device.Control`, §5.8). The data connection
//! itself (on/off, roaming, APN) is NetworkManager's gsm profile, written with `net.connection.*`.
use super::connections as nm;
use super::settings_map as map;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::{call, call_interactive, first, getall, variant_json};
use glib::variant::ToVariant;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

pub(crate) const MM: &str = "org.freedesktop.ModemManager1";
pub(crate) const MM_PATH: &str = "/org/freedesktop/ModemManager1";
const MODEM_PREFIX: &str = "/org/freedesktop/ModemManager1/Modem/";

fn iface(name: &str) -> String {
    format!("{MM}.{name}")
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn u(v: &Value) -> u64 {
    v.as_u64().unwrap_or(0)
}

/// MMModemLock as text.
pub(crate) fn lock_name(n: u64) -> &'static str {
    match n {
        1 => "none",
        2 => "sim-pin",
        3 => "sim-pin2",
        4 => "sim-puk",
        5 => "sim-puk2",
        6 => "ph-sp-pin",
        7 => "ph-sp-puk",
        8 => "ph-net-pin",
        9 => "ph-net-puk",
        10 => "ph-sim-pin",
        11 => "ph-corp-pin",
        12 => "ph-corp-puk",
        13 => "ph-fsim-pin",
        14 => "ph-fsim-puk",
        15 => "ph-netsub-pin",
        16 => "ph-netsub-puk",
        _ => "unknown",
    }
}

/// MMModemState as text.
pub(crate) fn state_name(n: i64) -> &'static str {
    match n {
        -1 => "failed",
        1 => "initializing",
        2 => "locked",
        3 => "disabled",
        4 => "disabling",
        5 => "enabling",
        6 => "enabled",
        7 => "searching",
        8 => "registered",
        9 => "disconnecting",
        10 => "connecting",
        11 => "connected",
        _ => "unknown",
    }
}

/// MMModemStateFailedReason as text.
pub(crate) fn failed_reason(n: u64) -> &'static str {
    match n {
        0 => "",
        2 => "sim-missing",
        3 => "sim-error",
        4 => "unknown-capabilities",
        5 => "esim-without-profiles",
        _ => "unknown",
    }
}

/// MMModem3gppRegistrationState as text.
pub(crate) fn registration_name(n: u64) -> &'static str {
    match n {
        0 => "idle",
        1 | 6 | 9 => "home",
        2 => "searching",
        3 => "denied",
        5 | 7 | 10 => "roaming",
        8 => "emergency-only",
        11 => "attached-rlos",
        _ => "unknown",
    }
}

/// MMModemAccessTechnology bits as names.
const TECH: &[(u64, &str)] = &[
    (1 << 0, "pots"),
    (1 << 1, "gsm"),
    (1 << 2, "gsm-compact"),
    (1 << 3, "gprs"),
    (1 << 4, "edge"),
    (1 << 5, "umts"),
    (1 << 6, "hsdpa"),
    (1 << 7, "hsupa"),
    (1 << 8, "hspa"),
    (1 << 9, "hspa-plus"),
    (1 << 10, "1xrtt"),
    (1 << 11, "evdo0"),
    (1 << 12, "evdoa"),
    (1 << 13, "evdob"),
    (1 << 14, "lte"),
    (1 << 15, "5gnr"),
    (1 << 16, "lte-cat-m"),
    (1 << 17, "lte-nb-iot"),
];

pub(crate) fn tech_names(bits: u64) -> Vec<&'static str> {
    TECH.iter()
        .filter(|(b, _)| bits & b != 0)
        .map(|(_, n)| *n)
        .collect()
}

/// The generation Windows shows as the network type ("5G", "4G", "3G", "2G"; "" when not known).
pub(crate) fn generation(bits: u64) -> &'static str {
    if bits & (1 << 15) != 0 {
        "5G"
    } else if bits & ((1 << 14) | (1 << 16) | (1 << 17)) != 0 {
        "4G"
    } else if bits & ((0x1f << 5) | (0x7 << 11)) != 0 {
        "3G"
    } else if bits & ((0xf << 1) | (1 << 10)) != 0 {
        "2G"
    } else {
        ""
    }
}

/// MMModemMode bits (2G, 3G, 4G, 5G) as names, newest first (Windows' order); `any` for MM_MODEM_MODE_ANY.
pub(crate) fn mode_names(bits: u64) -> Vec<&'static str> {
    if bits == 0xffff_ffff {
        return vec!["any"];
    }
    [
        (1 << 4, "5g"),
        (1 << 3, "4g"),
        (1 << 2, "3g"),
        (1 << 1, "2g"),
    ]
    .iter()
    .filter(|(b, _)| bits & b != 0)
    .map(|(_, n)| *n)
    .collect()
}

/// A modem's object path (`/org/freedesktop/ModemManager1/Modem/<n>`), nothing else.
pub(crate) fn modem_ok(path: &str) -> bool {
    path.strip_prefix(MODEM_PREFIX)
        .is_some_and(|n| !n.is_empty() && n.len() <= 6 && n.bytes().all(|b| b.is_ascii_digit()))
}

/// A SIM PIN: 4 to 8 digits.
pub(crate) fn pin_ok(pin: &str) -> bool {
    (4..=8).contains(&pin.len()) && pin.bytes().all(|b| b.is_ascii_digit())
}

/// A PUK: 8 digits.
pub(crate) fn puk_ok(puk: &str) -> bool {
    puk.len() == 8 && puk.bytes().all(|b| b.is_ascii_digit())
}

/// A network mode pair: something allowed, the preferred one nothing or one bit of it (MM_MODEM_MODE_ANY allowed).
pub(crate) fn modes_ok(allowed: u64, preferred: u64) -> bool {
    allowed > 0
        && allowed <= 0xffff_ffff
        && (preferred == 0 || (preferred.is_power_of_two() && allowed & preferred != 0))
}

/// An operator code for Register: "" (automatic) or MCC+MNC, 5 or 6 digits.
pub(crate) fn operator_ok(code: &str) -> bool {
    code.is_empty() || ((5..=6).contains(&code.len()) && code.bytes().all(|b| b.is_ascii_digit()))
}

/// The `UnlockRetries` dictionary (`a{uu}`, lock → retries left) as {lock name: retries}.
fn retries_json(v: &glib::Variant) -> Value {
    let mut out = Map::new();
    if v.type_().as_str() == "a{uu}" {
        for i in 0..v.n_children() {
            let e = v.child_value(i);
            if let (Some(k), Some(n)) =
                (e.child_value(0).get::<u32>(), e.child_value(1).get::<u32>())
            {
                out.insert(lock_name(k as u64).to_owned(), json!(n));
            }
        }
    }
    Value::Object(out)
}

/// GetManagedObjects (`a{oa{sa{sv}}}`) as {path: {interface: {property: value}}}, the retries readable.
pub(crate) fn objects_json(v: &glib::Variant) -> Value {
    let mut out = Map::new();
    let root = if v.type_().as_str().starts_with('(') && v.n_children() > 0 {
        v.child_value(0)
    } else {
        v.clone()
    };
    if !root.type_().as_str().starts_with("a{oa{sa{sv}}}") {
        return Value::Object(out);
    }
    for i in 0..root.n_children() {
        let e = root.child_value(i);
        let Some(path) = e.child_value(0).str().map(str::to_owned) else {
            continue;
        };
        let mut ifaces = Map::new();
        let list = e.child_value(1);
        for j in 0..list.n_children() {
            let ie = list.child_value(j);
            let Some(name) = ie.child_value(0).str().map(str::to_owned) else {
                continue;
            };
            let props = ie.child_value(1);
            let mut m = Map::new();
            for k in 0..props.n_children() {
                let pe = props.child_value(k);
                let key_v = pe.child_value(0);
                let (Some(key), Some(value)) = (key_v.str(), pe.child_value(1).as_variant()) else {
                    continue;
                };
                let j = if key == "UnlockRetries" {
                    retries_json(&value)
                } else {
                    variant_json(&value)
                };
                m.insert(key.to_owned(), j);
            }
            ifaces.insert(name, Value::Object(m));
        }
        out.insert(path, Value::Object(ifaces));
    }
    Value::Object(out)
}

/// One SIM as the page shows it.
pub(crate) fn sim_json(path: &str, sim: &Value) -> Value {
    let op = s(&sim["OperatorIdentifier"]);
    json!({
        "path": path,
        "active": sim["Active"] != false,
        "iccid": s(&sim["SimIdentifier"]),
        "imsi": s(&sim["Imsi"]),
        "eid": s(&sim["Eid"]),
        "operatorName": s(&sim["OperatorName"]),
        "operatorCode": op,
        "mcc": op.get(..3).unwrap_or(""),
        "mnc": op.get(3..).unwrap_or(""),
        "esim": u(&sim["SimType"]) == 2,
    })
}

/// One modem as the page shows it, from its interfaces (objects_json) and the SIM objects.
pub(crate) fn modem_json(path: &str, ifaces: &Value, objects: &Value) -> Value {
    let m = &ifaces[iface("Modem")];
    let g = &ifaces[iface("Modem.Modem3gpp")];
    let sim_of = |p: &str| -> Value {
        if p.is_empty() || p == "/" {
            Value::Null
        } else {
            sim_json(p, &objects[p][iface("Sim")])
        }
    };
    let sim_path = s(&m["Sim"]);
    let slots: Vec<Value> = m["SimSlots"]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, p)| json!({ "slot": i + 1, "sim": sim_of(s(p)) }))
                .collect()
        })
        .unwrap_or_default();
    let tech = u(&m["AccessTechnologies"]);
    let modes = |pair: &Value| {
        let (a, p) = (u(&pair[0]), u(&pair[1]));
        json!({ "allowed": a, "preferred": p, "allowedNames": mode_names(a), "preferredNames": if p == 0 { vec![] } else { mode_names(p) } })
    };
    let supported: Vec<Value> = m["SupportedModes"]
        .as_array()
        .map(|a| a.iter().map(modes).collect())
        .unwrap_or_default();
    let imei = if s(&g["Imei"]).is_empty() {
        s(&m["EquipmentIdentifier"])
    } else {
        s(&g["Imei"])
    };
    json!({
        "path": path,
        "manufacturer": s(&m["Manufacturer"]),
        "model": s(&m["Model"]),
        "firmware": s(&m["Revision"]),
        "hardwareRevision": s(&m["HardwareRevision"]),
        "deviceIdentifier": s(&m["DeviceIdentifier"]),
        "imei": imei,
        "ownNumbers": m["OwnNumbers"].as_array().cloned().unwrap_or_default(),
        "state": state_name(m["State"].as_i64().unwrap_or(0)),
        "failedReason": failed_reason(u(&m["StateFailedReason"])),
        "unlockRequired": lock_name(u(&m["UnlockRequired"])),
        "unlockRetries": m["UnlockRetries"].clone(),
        "technologies": tech_names(tech),
        "networkType": generation(tech),
        "signal": u(&m["SignalQuality"][0]),
        "supportedModes": supported,
        "currentModes": modes(&m["CurrentModes"]),
        "sim": sim_of(sim_path),
        "simSlots": slots,
        "primarySimSlot": u(&m["PrimarySimSlot"]),
        "registration": registration_name(u(&g["RegistrationState"])),
        "operatorName": s(&g["OperatorName"]),
        "operatorCode": s(&g["OperatorCode"]),
        "pinEnabled": u(&g["EnabledFacilityLocks"]) & 1 != 0,
        "is3gpp": g.is_object(),
    })
}

/// How a gsm profile signs in, from its ppp refuse flags (GNOME's APN dialog "Auth Type"): `auto` when nothing is
/// refused, `none` when everything is, else the one method left.
pub(crate) fn apn_auth(settings: &Value) -> &'static str {
    let ppp = &settings["ppp"];
    let refused = |k: &str| ppp[k] == true;
    let methods = [
        ("pap", "refuse-pap"),
        ("chap", "refuse-chap"),
        ("mschap", "refuse-mschap"),
        ("mschapv2", "refuse-mschapv2"),
        ("eap", "refuse-eap"),
    ];
    let left: Vec<&str> = methods
        .iter()
        .filter(|(_, k)| !refused(k))
        .map(|(m, _)| *m)
        .collect();
    match left.len() {
        5 => "auto",
        0 => "none",
        1 => left[0],
        _ => "other",
    }
}

/// The ppp group of an APN sign-in method (the inverse of [`apn_auth`]; `auto` clears the group). The page builds
/// its own (native/apps/settings/src/pages/mobile.rs `auth_ppp`); the tests check that the two agree.
#[cfg(test)]
pub(crate) fn apn_auth_ppp(auth: &str) -> Option<Value> {
    let all = ["pap", "chap", "mschap", "mschapv2", "eap"];
    match auth {
        "auto" => Some(Value::Null),
        "none" => Some(json!(
            all.iter()
                .map(|m| (format!("refuse-{m}"), json!(true)))
                .collect::<Map<_, _>>()
        )),
        m if all.contains(&m) => Some(json!(
            all.iter()
                .map(|x| (format!("refuse-{x}"), json!(*x != m)))
                .collect::<Map<_, _>>()
        )),
        _ => None,
    }
}

/// A gsm profile as the APN list shows it.
pub(crate) fn gsm_json(path: &str, j: &Value, active: Option<&(String, Value)>) -> Value {
    let c = &j["connection"];
    let gsm = &j["gsm"];
    json!({
        "uuid": c["uuid"], "id": c["id"], "path": path,
        "autoconnect": c["autoconnect"] != false,
        "apn": s(&gsm["apn"]), "username": s(&gsm["username"]),
        "homeOnly": gsm["home-only"] == true,
        "simId": s(&gsm["sim-id"]), "deviceId": s(&gsm["device-id"]),
        "attach": gsm["initial-eps-bearer-configure"] == true,
        "attachApn": s(&gsm["initial-eps-bearer-apn"]),
        "auth": apn_auth(j),
        "active": active.is_some(),
        "state": active.map(|(_, a)| nm::active_state(u(&a["State"]))).unwrap_or(""),
    })
}

/// net.modem.get: every modem with its SIMs, NetworkManager's device and the gsm profiles that fit its SIM.
pub(crate) fn get(bus: &gio::DBusConnection) -> Value {
    let Ok(r) = call(
        bus,
        MM,
        MM_PATH,
        "org.freedesktop.DBus.ObjectManager",
        "GetManagedObjects",
        None,
        3000,
    ) else {
        return json!({ "available": false, "modems": [] });
    };
    let objects = objects_json(&r);
    // NetworkManager's side, when it runs: the modem devices (Udi = the modem's path) and the gsm profiles
    let nm_root = getall(bus, nm::NM, nm::PATH, nm::NM).ok();
    let mut devices: BTreeMap<String, Value> = BTreeMap::new();
    let mut gsm: Vec<(String, Value)> = Vec::new();
    let actives = if nm_root.is_some() {
        nm::actives(bus)
    } else {
        Vec::new()
    };
    if nm_root.is_some() {
        for p in nm::device_paths(bus) {
            if let Ok(d) = getall(bus, nm::NM, &p, &format!("{}.Device", nm::NM))
                && d["DeviceType"] == 8
            {
                devices.insert(s(&d["Udi"]).to_owned(), json!({ "path": p, "iface": d["Interface"], "state": nm::device_state(u(&d["State"]), None), "available": d["AvailableConnections"] }));
            }
        }
        for p in nm::list(bus).unwrap_or_default() {
            if let Ok(g) = nm::settings_of(bus, &p) {
                let j = map::to_json(&g);
                if j["connection"]["type"] == "gsm" {
                    gsm.push((p, j));
                }
            }
        }
    }
    let mut modems: Vec<Value> = Vec::new();
    if let Some(all) = objects.as_object() {
        for (path, ifaces) in all {
            if !modem_ok(path) || !ifaces[iface("Modem")].is_object() {
                continue;
            }
            let mut m = modem_json(path, ifaces, &objects);
            let iccid = s(&m["sim"]["iccid"]).to_owned();
            let dev_id = s(&m["deviceIdentifier"]).to_owned();
            let dev = devices.get(path.as_str()).cloned().unwrap_or(Value::Null);
            let available: Vec<&str> = dev["available"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let profiles: Vec<Value> = gsm
                .iter()
                .filter(|(p, j)| {
                    let sim_id = s(&j["gsm"]["sim-id"]);
                    let device_id = s(&j["gsm"]["device-id"]);
                    available.contains(&p.as_str())
                        || ((sim_id.is_empty() || sim_id == iccid)
                            && (device_id.is_empty() || device_id == dev_id))
                })
                .map(|(p, j)| {
                    let active = actives.iter().find(|(_, a)| a["Connection"] == json!(p));
                    gsm_json(p, j, active)
                })
                .collect();
            m["iface"] = dev["iface"].clone();
            m["nmState"] = dev["state"].clone();
            m["connections"] = json!(profiles);
            modems.push(m);
        }
    }
    let root = nm_root.unwrap_or(json!({}));
    json!({
        "available": true,
        "modems": modems,
        "wwanEnabled": root["WwanEnabled"] == true,
        "wwanHardwareEnabled": root["WwanHardwareEnabled"] != false,
    })
}

pub(crate) fn not_found(lang: Lang) -> ApiError {
    ApiError::tr("not_found", lang, "net.noModem", NO_PARAMS)
}

/// The SIM object of a modem (its `Sim` property).
fn sim_path(bus: &gio::DBusConnection, modem: &str, lang: Lang) -> Result<String, ApiError> {
    let m = getall(bus, MM, modem, &iface("Modem")).map_err(|_| not_found(lang))?;
    let sim = s(&m["Sim"]);
    if sim.is_empty() || sim == "/" {
        return Err(ApiError::tr("not_found", lang, "net.noSim", NO_PARAMS));
    }
    Ok(sim.to_owned())
}

/// Sim.SendPin(pin), or Sim.SendPuk(puk, new pin) (GNOME's SIM Unlock).
pub(crate) fn unlock(
    bus: &gio::DBusConnection,
    modem: &str,
    pin: &str,
    puk: Option<&str>,
    lang: Lang,
) -> Result<(), ApiError> {
    let sim = sim_path(bus, modem, lang)?;
    let (method, args) = match puk {
        Some(puk) => ("SendPuk", (puk, pin).to_variant()),
        None => ("SendPin", (pin,).to_variant()),
    };
    call_interactive(bus, MM, &sim, &iface("Sim"), method, Some(&args), lang).map(|_| ())
}

/// Sim.EnablePin(pin, enabled) (GNOME's SIM Lock switch).
pub(crate) fn enable_pin(
    bus: &gio::DBusConnection,
    modem: &str,
    pin: &str,
    enabled: bool,
    lang: Lang,
) -> Result<(), ApiError> {
    let sim = sim_path(bus, modem, lang)?;
    let args = (pin, enabled).to_variant();
    call_interactive(bus, MM, &sim, &iface("Sim"), "EnablePin", Some(&args), lang).map(|_| ())
}

/// Sim.ChangePin(old, new).
pub(crate) fn change_pin(
    bus: &gio::DBusConnection,
    modem: &str,
    old: &str,
    new: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    let sim = sim_path(bus, modem, lang)?;
    let args = (old, new).to_variant();
    call_interactive(bus, MM, &sim, &iface("Sim"), "ChangePin", Some(&args), lang).map(|_| ())
}

/// The modem's interfaces, or net.noModem.
fn modem_props(bus: &gio::DBusConnection, modem: &str, lang: Lang) -> Result<Value, ApiError> {
    getall(bus, MM, modem, &iface("Modem")).map_err(|_| not_found(lang))
}

/// Modem.SetCurrentModes((allowed, preferred)), one of the modem's SupportedModes.
pub(crate) fn set_modes(
    bus: &gio::DBusConnection,
    modem: &str,
    allowed: u32,
    preferred: u32,
    lang: Lang,
) -> Result<(), ApiError> {
    let m = modem_props(bus, modem, lang)?;
    let supported = m["SupportedModes"].as_array().is_some_and(|a| {
        a.iter()
            .any(|p| u(&p[0]) == allowed as u64 && u(&p[1]) == preferred as u64)
    });
    if !supported {
        return Err(ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS));
    }
    let args = ((allowed, preferred),).to_variant();
    call_interactive(
        bus,
        MM,
        modem,
        &iface("Modem"),
        "SetCurrentModes",
        Some(&args),
        lang,
    )
    .map(|_| ())
}

/// Modem.SetPrimarySimSlot(slot), a slot the modem has.
pub(crate) fn set_primary_slot(
    bus: &gio::DBusConnection,
    modem: &str,
    slot: u32,
    lang: Lang,
) -> Result<(), ApiError> {
    let m = modem_props(bus, modem, lang)?;
    let slots = m["SimSlots"].as_array().map(Vec::len).unwrap_or(0);
    if slot == 0 || slot as usize > slots {
        return Err(ApiError::tr("bad_args", lang, "net.badArgs", NO_PARAMS));
    }
    call_interactive(
        bus,
        MM,
        modem,
        &iface("Modem"),
        "SetPrimarySimSlot",
        Some(&(slot,).to_variant()),
        lang,
    )
    .map(|_| ())
}

/// MMModem3gppNetworkAvailability as text.
pub(crate) fn availability_name(n: u64) -> &'static str {
    match n {
        1 => "available",
        2 => "current",
        3 => "forbidden",
        _ => "unknown",
    }
}

/// One network of Modem3gpp.Scan as the page lists it.
pub(crate) fn network_json(n: &Value) -> Value {
    let tech = u(&n["access-technology"]);
    json!({
        "code": s(&n["operator-code"]),
        "name": if s(&n["operator-long"]).is_empty() { s(&n["operator-short"]) } else { s(&n["operator-long"]) },
        "short": s(&n["operator-short"]),
        "status": availability_name(u(&n["status"])),
        "networkType": generation(tech),
    })
}

/// Modem3gpp.Scan (it may take a minute; interactive): the networks in range, the current one first.
pub(crate) fn scan(bus: &gio::DBusConnection, modem: &str, lang: Lang) -> ApiResult {
    modem_props(bus, modem, lang)?;
    let r = call_interactive(
        bus,
        MM,
        modem,
        &iface("Modem.Modem3gpp"),
        "Scan",
        None,
        lang,
    )?;
    let mut networks: Vec<Value> = first(&r)
        .as_array()
        .map(|a| a.iter().map(network_json).collect())
        .unwrap_or_default();
    networks.sort_by_key(|n| match s(&n["status"]) {
        "current" => 0,
        "available" => 1,
        "unknown" => 2,
        _ => 3,
    });
    Ok(json!({ "networks": networks }))
}

/// Modem3gpp.Register(operator) ("" = automatic).
pub(crate) fn register(
    bus: &gio::DBusConnection,
    modem: &str,
    operator: &str,
    lang: Lang,
) -> Result<(), ApiError> {
    modem_props(bus, modem, lang)?;
    call_interactive(
        bus,
        MM,
        modem,
        &iface("Modem.Modem3gpp"),
        "Register",
        Some(&(operator,).to_variant()),
        lang,
    )
    .map(|_| ())
}
