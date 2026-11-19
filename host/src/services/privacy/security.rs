//! Device security (privacy.security): what GNOME Settings' Device Security page reads from fwupd on the system bus
//! (cc-firmware-security-panel.c, read only): the host security id (`HostSecurityId`, "HSI:1! (v2.0.13)"), the
//! security attributes (`GetHostSecurityAttrs`) and the latest security events (`GetHostSecurityEvents`). The host
//! hands the attributes over as fwupd gives them (appstream id, HSI level, result, flags); Settings names them in
//! WinPoint's language. Also Ubuntu's Security Center (privacy.securityCenter.open).
use crate::services::system::common;
use glib::prelude::*;
use serde_json::{Value, json};

pub(super) const NAME: &str = "org.freedesktop.fwupd";
const PATH: &str = "/";
const IFACE: &str = "org.freedesktop.fwupd";
/// fwupd starts on demand and checks the hardware on its first call.
const TIMEOUT_MS: i32 = 15_000;
/// GNOME's panel asks for the last 10 events.
pub(super) const EVENTS: u32 = 10;

/// The desktop ids of Ubuntu's Security Center (the desktop-security-center snap; launchBridge.js APPS).
pub(super) const SECURITY_CENTER: &[&str] = &[
    "desktop-security-center_desktop-security-center.desktop",
    "security-center_security-center.desktop",
];

/// fwupd's FwupdSecurityAttrFlags the page uses.
pub(super) const FLAG_SUCCESS: u64 = 1;
pub(super) const FLAG_OBSOLETED: u64 = 1 << 1;
pub(super) const FLAG_RUNTIME_ISSUE: u64 = 1 << 10;

/// The level and the runtime mark of a HostSecurityId: "HSI:2! (v2.0.13)" → (Some(2), true); "HSI:INVALID" or an
/// empty id → (None, false) (GNOME's "Checks Unavailable").
pub(super) fn parse_hsi(id: &str) -> (Option<u32>, bool) {
    let Some(rest) = id.trim().strip_prefix("HSI:") else {
        return (None, false);
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let level = digits.parse::<u32>().ok().filter(|l| *l <= 5);
    let head = rest.split(' ').next().unwrap_or("");
    (level, level.is_some() && head.contains('!'))
}

/// One attribute or event as JSON (cc-firmware-security-utils.c fu_security_attr_new_from_variant's keys).
pub(super) fn attr_json(dict: &std::collections::BTreeMap<String, glib::Variant>) -> Value {
    let text = |k: &str| dict.get(k).and_then(|v| v.str().map(str::to_owned));
    let uint = |k: &str| dict.get(k).and_then(|v| v.get::<u32>());
    let long = |k: &str| dict.get(k).and_then(|v| v.get::<u64>());
    let flags = long("Flags").unwrap_or(0);
    json!({
        "appstreamId": text("AppstreamId"),
        "title": text("Title").or_else(|| text("Name")),
        "level": uint("HsiLevel").unwrap_or(0),
        "result": uint("HsiResult").unwrap_or(0),
        "previous": uint("HsiResultFallback"),
        "flags": flags,
        "success": flags & FLAG_SUCCESS != 0,
        "runtimeIssue": flags & FLAG_RUNTIME_ISSUE != 0,
        "obsoleted": flags & FLAG_OBSOLETED != 0,
        "created": long("Created"),
        "plugin": text("Plugin"),
    })
}

fn attrs(v: &glib::Variant) -> Vec<Value> {
    let list = v.child_value(0);
    (0..list.n_children())
        .map(|i| attr_json(&common::dictionary(&list.child_value(i))))
        .filter(|a| a["appstreamId"].is_string())
        .collect()
}

/// Secure Boot's state from the attributes: "on", "off", "problem", or None without the attribute (GNOME's
/// Secure Boot row: Active / Turned Off / Has Problems).
pub(super) fn secure_boot(attrs: &[Value]) -> Option<&'static str> {
    let attr = attrs
        .iter()
        .find(|a| a["appstreamId"] == "org.fwupd.hsi.Uefi.SecureBoot")?;
    // FWUPD_SECURITY_ATTR_RESULT_ENABLED = 1, NOT_ENABLED = 2
    Some(match (attr["success"] == true, attr["result"].as_u64()) {
        (true, _) | (_, Some(1)) => "on",
        (false, Some(2)) => "off",
        _ => "problem",
    })
}

/// `{available, hsi, level, runtimeIssue, secureBoot, attrs, events, daemonVersion, vendor, product}`;
/// `available: false` without fwupd (not installed, or it answered nothing).
pub(super) fn read(bus: &gio::DBusConnection, name: &str) -> Value {
    let props = common::call(
        bus,
        name,
        PATH,
        "org.freedesktop.DBus.Properties",
        "GetAll",
        Some(&(IFACE,).to_variant()),
        TIMEOUT_MS,
    );
    let Ok(props) = props else {
        return json!({ "available": false });
    };
    let props = common::dictionary(&props.child_value(0));
    let prop = |k: &str| props.get(k).and_then(|v| v.str().map(str::to_owned));
    let hsi = prop("HostSecurityId").unwrap_or_default();
    let (level, runtime) = parse_hsi(&hsi);
    let found = common::call(
        bus,
        name,
        PATH,
        IFACE,
        "GetHostSecurityAttrs",
        None,
        TIMEOUT_MS,
    )
    .map(|v| attrs(&v))
    .unwrap_or_default();
    let events = common::call(
        bus,
        name,
        PATH,
        IFACE,
        "GetHostSecurityEvents",
        Some(&(EVENTS,).to_variant()),
        TIMEOUT_MS,
    )
    .map(|v| attrs(&v))
    .unwrap_or_default();
    json!({
        "available": true,
        "hsi": hsi,
        "level": level,
        "runtimeIssue": runtime,
        "secureBoot": secure_boot(&found),
        "attrs": found,
        "events": events,
        "daemonVersion": prop("DaemonVersion"),
        "vendor": prop("HostVendor"),
        "product": prop("HostProduct"),
    })
}
