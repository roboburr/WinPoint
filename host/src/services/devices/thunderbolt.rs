//! Thunderbolt and USB4 through bolt (GNOME's Privacy › Thunderbolt, cc-bolt-panel.c and cc-bolt-device-dialog.c):
//! the Manager's AuthMode ("Directe toegang": `enabled` / `disabled`), its devices with their status, and the
//! device dialog's actions: "Autoriseren en verbinden" (EnrollDevice for a new device, Authorize for a stored one)
//! and "Apparaat vergeten" (ForgetDevice). bolt checks polkit for each (`org.freedesktop.bolt.manage`, `.enroll`,
//! `.authorize`), so the writes go through `call_interactive`.
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use crate::services::system::common::{
    call, call_interactive, first, getall, object_path, string, truth,
};
use glib::variant::ToVariant;
use serde_json::{Value, json};

const MANAGER_PATH: &str = "/org/freedesktop/bolt";
const MANAGER: &str = "org.freedesktop.bolt1.Manager";
const DEVICE: &str = "org.freedesktop.bolt1.Device";
const DOMAIN: &str = "org.freedesktop.bolt1.Domain";

/// The values bolt takes (bolt-enums.h): AuthMode, and the policy of EnrollDevice.
const MODES: &[&str] = &["enabled", "disabled"];
const POLICIES: &[&str] = &["default", "manual", "auto", "iommu"];

pub(super) fn unavailable(lang: Lang) -> ApiError {
    ApiError::tr(
        "unavailable",
        lang,
        "devices.thunderboltUnavailable",
        crate::i18n::NO_PARAMS,
    )
}

/// A bolt device uid: the device's unique_id of sysfs (a UUID), so letters, digits and '-', at most 64.
pub(super) fn uid_ok(uid: &str) -> bool {
    !uid.is_empty()
        && uid.len() <= 64
        && uid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub(super) fn uid_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    v.as_str()
        .filter(|u| uid_ok(u))
        .map(str::to_owned)
        .ok_or_else(|| super::bad(lang, "devices.badDevice"))
}

fn one_of(
    v: &Value,
    allowed: &'static [&'static str],
    lang: Lang,
) -> Result<&'static str, ApiError> {
    v.as_str()
        .and_then(|s| allowed.iter().find(|a| **a == s).copied())
        .ok_or_else(|| super::bad(lang, "devices.badArgs"))
}

pub(super) fn mode_arg(v: &Value, lang: Lang) -> Result<&'static str, ApiError> {
    one_of(v, MODES, lang)
}

/// The policy of an enrolment; none given is bolt's "default" (the Manager's DefaultPolicy), as GNOME passes.
pub(super) fn policy_arg(v: &Value, lang: Lang) -> Result<&'static str, ApiError> {
    if v.is_null() {
        Ok("default")
    } else {
        one_of(v, POLICIES, lang)
    }
}

/// One device as Settings shows it: GNOME's device dialog fields (Status, UUID, the times) and what it may do.
pub(super) fn device_json(path: &str, d: &Value) -> Value {
    let status = string(&d["Status"]);
    let authorized = status.starts_with("authorized");
    let connected = !matches!(status.as_str(), "disconnected" | "unknown" | "");
    let stored = truth(&d["Stored"]);
    let time = |key: &str| d[key].as_u64().filter(|t| *t > 0);
    json!({
        "path": path,
        "uid": string(&d["Uid"]),
        "name": string(&d["Name"]),
        "vendor": string(&d["Vendor"]),
        "label": string(&d["Label"]),
        "type": string(&d["Type"]),
        "status": status,
        "authorized": authorized,
        "connected": connected,
        "stored": stored,
        "policy": string(&d["Policy"]),
        "key": string(&d["Key"]),
        "parent": string(&d["Parent"]),
        "domain": string(&d["Domain"]),
        "generation": d["Generation"].as_u64().unwrap_or(0),
        "connectTime": time("ConnectTime"),
        "authorizeTime": time("AuthorizeTime"),
        "storeTime": time("StoreTime"),
        // GNOME's "Pending Devices": connected, not authorized, not stored
        "pending": connected && !authorized && !stored,
    })
}

/// `devices.thunderbolt.get` without bolt.
pub(super) fn absent() -> Value {
    json!({"available": false, "authMode": "", "devices": []})
}

fn paths(bus: &gio::DBusConnection, name: &str, method: &str) -> Vec<String> {
    call(bus, name, MANAGER_PATH, MANAGER, method, None, 3000)
        .map(|v| first(&v))
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .map(string)
        .collect()
}

/// `devices.thunderbolt.get`: the Manager (AuthMode, Probing, DefaultPolicy), the domains' security level and the
/// devices other than the hosts (GNOME lists peripherals only), connected first.
pub(super) fn get(bus: &gio::DBusConnection, name: &str) -> Value {
    let Ok(manager) = getall(bus, name, MANAGER_PATH, MANAGER) else {
        return absent();
    };
    let security: Vec<String> = paths(bus, name, "ListDomains")
        .iter()
        .filter_map(|p| getall(bus, name, p, DOMAIN).ok())
        .map(|d| string(&d["SecurityLevel"]))
        .filter(|s| !s.is_empty())
        .collect();
    let mut devices: Vec<Value> = paths(bus, name, "ListDevices")
        .iter()
        .filter_map(|p| Some(device_json(p, &getall(bus, name, p, DEVICE).ok()?)))
        .filter(|d| d["type"] != "host")
        .collect();
    devices.sort_by(|a, b| {
        (b["connected"] == true)
            .cmp(&(a["connected"] == true))
            .then_with(|| {
                string(&a["name"])
                    .to_lowercase()
                    .cmp(&string(&b["name"]).to_lowercase())
            })
    });
    json!({
        "available": true,
        "authMode": string(&manager["AuthMode"]),
        "probing": truth(&manager["Probing"]),
        "defaultPolicy": string(&manager["DefaultPolicy"]),
        "securityLevel": security.first().cloned().unwrap_or_else(|| string(&manager["SecurityLevel"])),
        "devices": devices,
    })
}

fn not_found(lang: Lang) -> ApiError {
    ApiError::tr(
        "not_found",
        lang,
        "devices.thunderboltNotFound",
        crate::i18n::NO_PARAMS,
    )
}

/// A refusal of bolt itself: no bolt answers "unavailable", an unknown uid "not found", polkit's answers as
/// `call_interactive` maps them.
fn daemon_error(error: ApiError, lang: Lang) -> ApiError {
    let message = error.message.to_ascii_lowercase();
    if error.key.as_deref() == Some("core.callFailed") {
        if message.contains("serviceunknown") || message.contains("not provided by any") {
            return unavailable(lang);
        }
        if message.contains("not found") || message.contains("unknown device") {
            return not_found(lang);
        }
    }
    error
}

/// `devices.thunderbolt.setAuthMode {mode}`: GNOME's Direct Access switch (Properties.Set of AuthMode).
pub(super) fn set_auth_mode(
    bus: &gio::DBusConnection,
    name: &str,
    mode: &str,
    lang: Lang,
) -> ApiResult {
    call_interactive(
        bus,
        name,
        MANAGER_PATH,
        "org.freedesktop.DBus.Properties",
        "Set",
        Some(&(MANAGER, "AuthMode", mode.to_variant()).to_variant()),
        lang,
    )
    .map_err(|e| daemon_error(e, lang))?;
    Ok(json!({"ok": true, "authMode": mode}))
}

/// `devices.thunderbolt.enroll {uid, policy}`: "Autoriseren en verbinden" of a new device (EnrollDevice stores it
/// with the policy and authorizes it).
pub(super) fn enroll(
    bus: &gio::DBusConnection,
    name: &str,
    uid: &str,
    policy: &str,
    lang: Lang,
) -> ApiResult {
    let reply = call_interactive(
        bus,
        name,
        MANAGER_PATH,
        MANAGER,
        "EnrollDevice",
        Some(&(uid, policy, "").to_variant()),
        lang,
    )
    .map_err(|e| daemon_error(e, lang))?;
    Ok(json!({"ok": true, "path": string(&first(&reply))}))
}

/// `devices.thunderbolt.authorize {uid}`: "Autoriseren en verbinden" of a stored device (Device.Authorize).
pub(super) fn authorize(bus: &gio::DBusConnection, name: &str, uid: &str, lang: Lang) -> ApiResult {
    let path = call(
        bus,
        name,
        MANAGER_PATH,
        MANAGER,
        "DeviceByUid",
        Some(&(uid,).to_variant()),
        3000,
    )
    .map(|v| string(&first(&v)))
    .map_err(|e| {
        if e.contains("ServiceUnknown") {
            unavailable(lang)
        } else {
            not_found(lang)
        }
    })?;
    object_path(&path).map_err(|_| not_found(lang))?;
    call_interactive(
        bus,
        name,
        &path,
        DEVICE,
        "Authorize",
        Some(&("",).to_variant()),
        lang,
    )
    .map_err(|e| daemon_error(e, lang))?;
    Ok(json!({"ok": true}))
}

/// `devices.thunderbolt.forget {uid}`: "Apparaat vergeten" (ForgetDevice: bolt drops the device and its key).
pub(super) fn forget(bus: &gio::DBusConnection, name: &str, uid: &str, lang: Lang) -> ApiResult {
    call_interactive(
        bus,
        name,
        MANAGER_PATH,
        MANAGER,
        "ForgetDevice",
        Some(&(uid,).to_variant()),
        lang,
    )
    .map_err(|e| daemon_error(e, lang))?;
    Ok(json!({"ok": true}))
}
