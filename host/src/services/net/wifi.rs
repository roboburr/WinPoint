//! Wi-Fi specifics of `net.*`: a hidden network's profile (`net.wifi.hidden`), the security key and its QR text
//! (`net.wifi.secrets`, gnome-control-center's Share Network dialog).
use super::connections::{self as nm, NM};
use super::settings_map as map;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use crate::services::system::common::getall;
use serde_json::{Value, json};

/// The profile of a hidden network: `connection` (id = the SSID, autoconnect as asked), `802-11-wireless` (the
/// SSID's bytes, infrastructure, hidden unless `hidden` is false: a network in the list that needs the 802.1X or
/// WEP fields, GNOME's NMAWifiDialog for an access point), automatic IPv4 and IPv6, and the security kind's groups
/// with the page's `secrets` (only the two security groups) over them. None for an SSID of 0 or more than 32 bytes,
/// an unknown kind, a `hidden` that is no boolean, or other groups in `secrets`.
pub(crate) fn hidden_settings(args: &Value, uuid: &str) -> Option<Value> {
    let ssid = args["ssid"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 32)?;
    let kind = args["security"].as_str().unwrap_or("open");
    let hidden = match &args["hidden"] {
        Value::Null => true,
        Value::Bool(b) => *b,
        _ => return None,
    };
    let mut settings = json!({
        "connection": {
            "id": ssid, "uuid": uuid, "type": "802-11-wireless",
            "autoconnect": args["autoconnect"] != false,
        },
        "802-11-wireless": { "ssid": ssid, "mode": "infrastructure", "hidden": hidden },
        "ipv4": { "method": "auto" },
        "ipv6": { "method": "auto" },
    });
    let mut security = map::security_base(kind)?;
    match &args["secrets"] {
        Value::Null => {}
        Value::Object(o) => {
            if o.keys()
                .any(|g| g != "802-11-wireless-security" && g != "802-1x")
            {
                return None;
            }
            map::overlay(&mut security, &args["secrets"]);
        }
        _ => return None,
    }
    // a new profile has no groups to remove: the nulls of the security kind only clear on an update
    for (g, v) in security.as_object()? {
        if !v.is_null() {
            settings[g] = v.clone();
        }
    }
    Some(settings)
}

/// The Wi-Fi device to use: the named interface, or the first Wi-Fi device.
pub(crate) fn wifi_device(bus: &gio::DBusConnection, name: Option<&str>) -> Option<String> {
    nm::device_paths(bus).into_iter().find(|p| {
        getall(bus, NM, p, &format!("{NM}.Device"))
            .is_ok_and(|d| d["DeviceType"] == 2 && name.is_none_or(|n| d["Interface"] == n))
    })
}

/// net.wifi.secrets {uuid}: the network's name, security and key (GetSecrets of 802-11-wireless-security, which may
/// ask for the password), and the QR text of the Share Network dialog.
pub(crate) fn secrets(bus: &gio::DBusConnection, uuid: &str, lang: Lang) -> ApiResult {
    let path = nm::path_of(bus, uuid, lang)?;
    let settings = map::to_json(&nm::settings_of(bus, &path).map_err(|_| nm::not_found(lang))?);
    let wireless = &settings["802-11-wireless"];
    if !wireless.is_object() {
        return Err(ApiError::tr("bad_args", lang, "net.notWifi", NO_PARAMS));
    }
    let ssid = map::ssid_text(&wireless["ssid"]);
    let hidden = wireless["hidden"] == true;
    let key_mgmt = settings["802-11-wireless-security"]["key-mgmt"]
        .as_str()
        .unwrap_or("")
        .to_owned();
    let mut key = String::new();
    if !key_mgmt.is_empty() && key_mgmt != "owe" {
        let secrets = map::to_json(&nm::secrets_of(
            bus,
            &path,
            "802-11-wireless-security",
            lang,
        )?);
        let sec = &secrets["802-11-wireless-security"];
        key = match key_mgmt.as_str() {
            "wpa-psk" | "sae" => sec["psk"].as_str(),
            "none" => {
                let index = settings["802-11-wireless-security"]["wep-tx-keyidx"]
                    .as_u64()
                    .unwrap_or(0)
                    .min(3);
                sec[format!("wep-key{index}")].as_str()
            }
            "ieee8021x" => sec["leap-password"].as_str(),
            _ => None,
        }
        .unwrap_or("")
        .to_owned();
    }
    let shareable = matches!(key_mgmt.as_str(), "" | "owe" | "wpa-psk" | "sae" | "none");
    let qr =
        shareable.then(|| map::wifi_qr(&ssid, map::qr_security(&key_mgmt), Some(&key), hidden));
    Ok(
        json!({ "uuid": uuid, "ssid": ssid, "keyMgmt": key_mgmt, "key": key, "hidden": hidden, "qr": qr }),
    )
}
