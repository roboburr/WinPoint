//! Instellingen › the shared IP, DNS, security and 802.1X dialogs of Wi-Fi, Ethernet and VPN (no page of its own)
//! (batch C, notes/claude/settings-parity.md §2), and the property list both network pages show.
//!
//! Windows 11's "Bewerken" dialogs on GNOME Settings' connection editor: every dialog reads the profile from
//! `net.connection.get` and sends only the keys it manages to `net.connection.update` (the host merges them into
//! NetworkManager's settings, so every other key stays). Amendment 40's libnma spike (notes/claude/settings-c.md):
//! libnma-gtk4's `nma_ws_*` widgets are installed, but they bring their own GNOME layout and libnma's texts, which
//! have no Dutch catalogue here; these forms are WinPoint's own, in Windows' wording, with the keys GNOME's forms
//! write.
use super::{Builder, Page};
use crate::common::{self, ct};
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, Severity, Toast, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[];
pub const CSS: &str = r#"
.st-net-form-section { font-weight: 600; margin-top: 8px; }
.st-net-row-entry { min-width: 110px; }
.st-net-error { color: var(--sys-critical); font-size: var(--fs-caption); }
.st-net-key { font-size: var(--fs-subtitle); }
.st-net-qr { padding: 12px; background-color: #ffffff; border-radius: var(--radius-card); }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The text of a table row in the UI language.
pub(super) fn tr(row: &(&'static str, &'static str, &'static str)) -> &'static str {
    if kit::i18n::en() { row.2 } else { row.1 }
}

fn label_of(table: &[(&'static str, &'static str, &'static str)], key: &str) -> Option<&'static str> {
    table.iter().find(|r| r.0 == key).map(tr)
}

pub(super) fn options(table: &[(&'static str, &'static str, &'static str)]) -> Vec<DropdownOption> {
    table.iter().map(|r| DropdownOption::new(r.0, tr(r))).collect()
}

// ------------------------------------------------------------------------------------------ the tables

/// (method, nl, en): ipv4.method (GNOME's IPv4 page; Windows names the first two)
pub const V4_METHODS: &[(&str, &str, &str)] = &[
    ("auto", "Automatisch (DHCP)", "Automatic (DHCP)"),
    ("manual", "Handmatig", "Manual"),
    ("link-local", "Alleen link-local", "Link-local only"),
    ("shared", "Gedeeld met andere computers", "Shared to other computers"),
    ("disabled", "Uitgeschakeld", "Disabled"),
];

/// (method, nl, en): ipv6.method (GNOME's IPv6 page)
pub const V6_METHODS: &[(&str, &str, &str)] = &[
    ("auto", "Automatisch", "Automatic"),
    ("dhcp", "Automatisch, alleen DHCP", "Automatic, DHCP only"),
    ("manual", "Handmatig", "Manual"),
    ("link-local", "Alleen link-local", "Link-local only"),
    ("shared", "Gedeeld met andere computers", "Shared to other computers"),
    ("disabled", "Uitgeschakeld", "Disabled"),
    ("ignore", "Genegeerd", "Ignored"),
];

/// (kind, nl, en): the security types of GNOME's Wi-Fi security page
pub const SECURITY: &[(&str, &str, &str)] = &[
    ("open", "Open", "Open"),
    ("owe", "Enhanced Open", "Enhanced Open"),
    ("wep", "WEP-sleutel (40/128-bits)", "WEP key (40/128-bit)"),
    ("wep-passphrase", "WEP-wachtwoordzin (128-bits)", "WEP passphrase (128-bit)"),
    ("leap", "LEAP", "LEAP"),
    ("dynamic-wep", "Dynamische WEP (802.1X)", "Dynamic WEP (802.1X)"),
    ("wpa-psk", "WPA/WPA2-Persoonlijk", "WPA/WPA2-Personal"),
    ("sae", "WPA3-Persoonlijk", "WPA3-Personal"),
    ("wpa-eap", "WPA/WPA2-Enterprise", "WPA/WPA2-Enterprise"),
];

/// (eap, nl, en): the 802.1X methods (libnma's EAP list, Windows' names for PEAP and TTLS)
pub const EAP: &[(&str, &str, &str)] = &[
    ("peap", "Beveiligde EAP (PEAP)", "Protected EAP (PEAP)"),
    ("ttls", "Getunneld TLS (TTLS)", "Tunnelled TLS (TTLS)"),
    ("tls", "TLS", "TLS"),
    ("pwd", "PWD", "PWD"),
    ("fast", "FAST", "FAST"),
    ("leap", "LEAP", "LEAP"),
];

/// (key, nl, en): PEAP's inner method (802-1x.phase2-auth)
const PEAP_INNER: &[(&str, &str, &str)] = &[("mschapv2", "MSCHAPv2", "MSCHAPv2"), ("md5", "MD5", "MD5"), ("gtc", "GTC", "GTC")];

/// (key, nl, en): TTLS's inner method, `auth:` = phase2-auth, `eap:` = phase2-autheap
const TTLS_INNER: &[(&str, &str, &str)] = &[
    ("auth:pap", "PAP", "PAP"),
    ("auth:mschap", "MSCHAP", "MSCHAP"),
    ("eap:mschapv2", "MSCHAPv2", "MSCHAPv2"),
    ("auth:mschapv2", "MSCHAPv2 (geen EAP)", "MSCHAPv2 (no EAP)"),
    ("auth:chap", "CHAP", "CHAP"),
    ("eap:md5", "MD5", "MD5"),
    ("eap:gtc", "GTC", "GTC"),
];

/// (key, nl, en): FAST's inner method (802-1x.phase2-autheap)
const FAST_INNER: &[(&str, &str, &str)] = &[("gtc", "GTC", "GTC"), ("mschapv2", "MSCHAPv2", "MSCHAPv2")];

/// (key, nl, en): 802-1x.phase1-peapver
const PEAP_VERSION: &[(&str, &str, &str)] = &[("auto", "Automatisch", "Automatic"), ("0", "Versie 0", "Version 0"), ("1", "Versie 1", "Version 1")];

/// (key, nl, en): 802-1x.phase1-fast-provisioning
const FAST_PROVISIONING: &[(&str, &str, &str)] =
    &[("0", "Uit", "Off"), ("1", "Anoniem", "Anonymous"), ("2", "Geverifieerd", "Authenticated"), ("3", "Beide", "Both")];

/// (flags, nl, en): where a password is kept (NMSettingSecretFlags: 1 the user's keyring, 0 the system, 2 never)
const SECRET_STORE: &[(&str, &str, &str)] = &[
    ("1", "Alleen voor deze gebruiker opslaan", "Save for this user only"),
    ("0", "Voor alle gebruikers opslaan", "Save for all users"),
    ("2", "Elke keer vragen", "Ask every time"),
];

/// (assigned-mac-address, nl, en): Windows' "Willekeurige hardwareadressen" on 802-11-wireless
pub const RANDOM_MAC: &[(&str, &str, &str)] = &[("permanent", "Uit", "Off"), ("stable", "Aan", "On"), ("random", "Dagelijks wijzigen", "Change daily")];

/// (assigned-mac-address, nl, en): GNOME's Cloned Address on 802-3-ethernet
pub const CLONED_MAC: &[(&str, &str, &str)] = &[
    ("default", "Standaard", "Default"),
    ("preserve", "Behouden", "Preserve"),
    ("permanent", "Vast hardwareadres", "Permanent address"),
    ("random", "Willekeurig", "Random"),
    ("stable", "Stabiel", "Stable"),
];

// ------------------------------------------------------------------------------------------ pure helpers

/// The prefix length of an IPv4 netmask ("255.255.255.0" → 24) or of a plain length ("24").
pub fn v4_prefix(text: &str) -> Option<u32> {
    let t = text.trim();
    if let Ok(n) = t.parse::<u32>() {
        return (n <= 32).then_some(n);
    }
    let bits = u32::from(t.parse::<std::net::Ipv4Addr>().ok()?);
    let n = bits.leading_ones();
    (bits.checked_shl(n).unwrap_or(0) == 0).then_some(n)
}

/// The IPv4 netmask of a prefix length (24 → "255.255.255.0").
pub fn v4_mask(prefix: u32) -> String {
    let bits = if prefix == 0 { 0 } else { u32::MAX << (32 - prefix.min(32)) };
    std::net::Ipv4Addr::from(bits).to_string()
}

/// Whether a text is an IPv4 (or, `six`, an IPv6) address.
pub fn ip_ok(text: &str, six: bool) -> bool {
    let t = text.trim();
    if six { t.parse::<std::net::Ipv6Addr>().is_ok() } else { t.parse::<std::net::Ipv4Addr>().is_ok() }
}

/// An IPv6 prefix length (1-128).
pub fn v6_prefix(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok().filter(|n| (1..=128).contains(n))
}

fn fam(six: bool) -> &'static str {
    if six { "ipv6" } else { "ipv4" }
}

/// The static addresses of a family (address-data).
pub fn addresses(settings: &Value, six: bool) -> Vec<(String, u32)> {
    settings[fam(six)]["address-data"]
        .as_array()
        .map(|a| a.iter().map(|x| (s(&x["address"]).to_owned(), x["prefix"].as_u64().unwrap_or(0) as u32)).collect())
        .unwrap_or_default()
}

/// The DNS servers of a family (dns-data, else the deprecated dns: network-order u32 or 16 bytes).
pub fn dns_list(settings: &Value, six: bool) -> Vec<String> {
    let g = &settings[fam(six)];
    if let Some(a) = g["dns-data"].as_array().filter(|a| !a.is_empty()) {
        return a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect();
    }
    let Some(a) = g["dns"].as_array() else { return Vec::new() };
    a.iter()
        .filter_map(|x| {
            if six {
                let bytes: Vec<u8> = x.as_array()?.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect();
                let arr: [u8; 16] = bytes.try_into().ok()?;
                Some(std::net::Ipv6Addr::from(arr).to_string())
            } else {
                Some(std::net::Ipv4Addr::from((x.as_u64()? as u32).to_ne_bytes()).to_string())
            }
        })
        .collect()
}

/// The routes of a family (route-data): (dest, prefix, next hop, metric).
pub fn routes(settings: &Value, six: bool) -> Vec<(String, u32, String, Option<u64>)> {
    settings[fam(six)]["route-data"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|r| (s(&r["dest"]).to_owned(), r["prefix"].as_u64().unwrap_or(0) as u32, s(&r["next-hop"]).to_owned(), r["metric"].as_u64()))
                .collect()
        })
        .unwrap_or_default()
}

fn method(settings: &Value, six: bool) -> &str {
    settings[fam(six)]["method"].as_str().unwrap_or("auto")
}

/// Windows' "IP-toewijzing" value: "Automatisch (DHCP)", "Handmatig", or GNOME's other methods (per family when
/// they differ).
pub fn ip_assignment_text(settings: &Value) -> String {
    let (m4, m6) = (method(settings, false), method(settings, true));
    let auto6 = matches!(m6, "auto" | "dhcp" | "ignore" | "disabled");
    if m4 == "auto" && auto6 {
        return tr(&V4_METHODS[0]).to_owned();
    }
    if m4 == "manual" || m6 == "manual" {
        return tr(&V4_METHODS[1]).to_owned();
    }
    let l4 = label_of(V4_METHODS, m4).unwrap_or(m4);
    if auto6 { l4.to_owned() } else { format!("IPv4: {l4}, IPv6: {}", label_of(V6_METHODS, m6).unwrap_or(m6)) }
}

fn dns_manual(settings: &Value) -> bool {
    [false, true].iter().any(|six| settings[fam(*six)]["ignore-auto-dns"] == true || !dns_list(settings, *six).is_empty())
}

/// Windows' "DNS-servertoewijzing" value.
pub fn dns_text(settings: &Value) -> String {
    tr(&V4_METHODS[if dns_manual(settings) { 1 } else { 0 }]).to_owned()
}

/// The security kind of a Wi-Fi profile (`SECURITY`'s keys).
pub fn security_kind(settings: &Value) -> &'static str {
    let sec = &settings["802-11-wireless-security"];
    match sec["key-mgmt"].as_str() {
        None => "open",
        Some("owe") => "owe",
        Some("none") if sec["wep-key-type"] == 2 => "wep-passphrase",
        Some("none") => "wep",
        Some("ieee8021x") if sec["auth-alg"] == "leap" => "leap",
        Some("ieee8021x") => "dynamic-wep",
        Some("sae") => "sae",
        Some("wpa-eap" | "wpa-eap-suite-b-192") => "wpa-eap",
        Some(_) => "wpa-psk",
    }
}

/// "Beveiligingstype": the kind, with the 802.1X method for an enterprise network.
pub fn security_text(settings: &Value) -> String {
    let kind = security_kind(settings);
    let base = label_of(SECURITY, kind).unwrap_or(kind).to_owned();
    match settings["802-1x"]["eap"][0].as_str().and_then(|e| label_of(EAP, e)) {
        Some(eap) if matches!(kind, "wpa-eap" | "dynamic-wep") => format!("{base} ({eap})"),
        _ => base,
    }
}

/// The 802-11-wireless-security and 802-1x groups of a security kind (as the host's `security_base`); null removes
/// a group the kind does not use.
pub fn security_base(kind: &str) -> Value {
    match kind {
        "owe" => json!({ "802-11-wireless-security": { "key-mgmt": "owe" }, "802-1x": null }),
        "wep" => json!({ "802-11-wireless-security": { "key-mgmt": "none", "wep-key-type": 1 }, "802-1x": null }),
        "wep-passphrase" => json!({ "802-11-wireless-security": { "key-mgmt": "none", "wep-key-type": 2 }, "802-1x": null }),
        "leap" => json!({ "802-11-wireless-security": { "key-mgmt": "ieee8021x", "auth-alg": "leap" }, "802-1x": null }),
        "dynamic-wep" => json!({ "802-11-wireless-security": { "key-mgmt": "ieee8021x", "auth-alg": "open" } }),
        "wpa-psk" => json!({ "802-11-wireless-security": { "key-mgmt": "wpa-psk" }, "802-1x": null }),
        "sae" => json!({ "802-11-wireless-security": { "key-mgmt": "sae" }, "802-1x": null }),
        "wpa-eap" => json!({ "802-11-wireless-security": { "key-mgmt": "wpa-eap" } }),
        _ => json!({ "802-11-wireless-security": null, "802-1x": null }),
    }
}

/// A WPA key NetworkManager takes: 8-63 printable ASCII characters or 64 hexadecimal digits.
pub fn psk_ok(key: &str) -> bool {
    if key.len() == 64 { key.bytes().all(|b| b.is_ascii_hexdigit()) } else { (8..=63).contains(&key.len()) && key.bytes().all(|b| (32..127).contains(&b)) }
}

/// A WEP key: 5 or 13 ASCII characters, or 10 or 26 hexadecimal digits.
pub fn wep_key_ok(key: &str) -> bool {
    match key.len() {
        5 | 13 => key.is_ascii(),
        10 | 26 => key.bytes().all(|b| b.is_ascii_hexdigit()),
        _ => false,
    }
}

/// A MAC address from NM's JSON (six numbers, or text).
pub fn mac_text(v: &Value) -> String {
    match v {
        Value::Array(a) if a.len() == 6 => a.iter().map(|b| format!("{:02X}", b.as_u64().unwrap_or(0))).collect::<Vec<_>>().join(":"),
        Value::String(s) => s.to_uppercase(),
        _ => String::new(),
    }
}

/// The path of a `file://` certificate from NM's JSON bytes.
pub fn cert_path(v: &Value) -> String {
    let bytes: Vec<u8> = v.as_array().map(|a| a.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect()).unwrap_or_default();
    let bytes = bytes.strip_suffix(&[0]).unwrap_or(&bytes);
    std::str::from_utf8(bytes).ok().and_then(|t| t.strip_prefix("file://")).unwrap_or("").to_owned()
}

/// "Netwerkband": the host's band ("2.4", "5", "6") as Windows writes it.
pub fn band_text(band: &str) -> String {
    match band {
        "" => String::new(),
        "2.4" => t("2,4 GHz", "2.4 GHz").to_owned(),
        b => format!("{b} GHz"),
    }
}

/// The text of a Wi-Fi SSID in NM's JSON (bytes).
pub fn ssid_text(v: &Value) -> String {
    match v {
        Value::Array(a) => String::from_utf8_lossy(&a.iter().filter_map(|b| b.as_u64().map(|b| b as u8)).collect::<Vec<_>>()).into_owned(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// "Verbinding met een datalimiet": NMMetered yes (1) or guess-yes (3).
pub fn metered_on(settings: &Value) -> bool {
    matches!(settings["connection"]["metered"].as_i64(), Some(1 | 3))
}

/// "Beschikbaar maken voor andere gebruikers": no `user:` permission.
pub fn shared(settings: &Value) -> bool {
    settings["connection"]["permissions"].as_array().is_none_or(|a| a.is_empty())
}

/// The permissions of a profile only this user may use (GNOME's `user:<name>:`).
pub fn private_permissions() -> Value {
    json!([format!("user:{}:", glib::user_name().to_string_lossy())])
}

/// The Windows property list of an adapter (and its Wi-Fi network): label, value; empty values are left out.
pub fn properties(dev: &Value, settings: Option<&Value>) -> Vec<(String, String)> {
    let wifi = dev["type"] == "wifi";
    let mut rows: Vec<(String, String)> = Vec::new();
    let mut push = |k: &str, v: String| {
        if !v.is_empty() {
            rows.push((k.to_owned(), v));
        }
    };
    let join = |v: &Value| v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    if wifi {
        let ap = &dev["ap"];
        push(t("SSID:", "SSID:"), settings.map(|x| ssid_text(&x["802-11-wireless"]["ssid"])).unwrap_or_else(|| s(&ap["ssid"]).to_owned()));
        if let Some(x) = settings {
            push(t("Beveiligingstype:", "Security type:"), security_text(x));
        }
        push(t("Fabrikant:", "Manufacturer:"), s(&dev["vendor"]).to_owned());
        push(t("Beschrijving:", "Description:"), s(&dev["model"]).to_owned());
        push(t("Versie van stuurprogramma:", "Driver version:"), s(&dev["driverVersion"]).to_owned());
        push(t("Netwerkband:", "Network band:"), band_text(s(&ap["band"])));
        push(t("Netwerkkanaal:", "Network channel:"), ap["channel"].as_u64().filter(|c| *c > 0).map(|c| c.to_string()).unwrap_or_default());
        let rate = dev["bitrate"].as_u64().unwrap_or(0) / 1000;
        push(t("Verbindingssnelheid (ontvangen/verzenden):", "Link speed (Receive/Transmit):"), if rate > 0 { format!("{rate}/{rate} (Mbps)") } else { String::new() });
    } else {
        let speed = dev["speed"].as_u64().unwrap_or(0);
        push(t("Verbindingssnelheid (ontvangen/verzenden):", "Link speed (Receive/Transmit):"), if speed > 0 { format!("{speed}/{speed} (Mbps)") } else { String::new() });
    }
    let v6: Vec<&Value> = dev["ip6"]["addresses"].as_array().map(|a| a.iter().collect()).unwrap_or_default();
    let link_local = |a: &&Value| s(&a["address"]).to_lowercase().starts_with("fe80:");
    push(t("IPv6-adres:", "IPv6 address:"), v6.iter().filter(|a| !link_local(a)).map(|a| s(&a["address"])).collect::<Vec<_>>().join(", "));
    push(
        t("Link-local IPv6-adres:", "Link-local IPv6 address:"),
        v6.iter().filter(|a| link_local(a)).map(|a| format!("{}%{}", s(&a["address"]), s(&dev["iface"]))).collect::<Vec<_>>().join(", "),
    );
    push(t("IPv6-DNS-servers:", "IPv6 DNS servers:"), join(&dev["ip6"]["dns"]));
    push(t("IPv4-adres:", "IPv4 address:"), dev["ip4"]["addresses"].as_array().map(|a| a.iter().map(|x| s(&x["address"])).collect::<Vec<_>>().join(", ")).unwrap_or_default());
    push(t("IPv4-DNS-servers:", "IPv4 DNS servers:"), join(&dev["ip4"]["dns"]));
    push(t("Standaardgateway:", "Default gateway:"), [s(&dev["ip4"]["gateway"]), s(&dev["ip6"]["gateway"])].iter().filter(|g| !g.is_empty()).copied().collect::<Vec<_>>().join(", "));
    if !wifi {
        push(t("Fabrikant:", "Manufacturer:"), s(&dev["vendor"]).to_owned());
        push(t("Beschrijving:", "Description:"), s(&dev["model"]).to_owned());
        push(t("Versie van stuurprogramma:", "Driver version:"), s(&dev["driverVersion"]).to_owned());
    }
    push(t("Fysiek adres (MAC):", "Physical address (MAC):"), s(&dev["hwAddress"]).to_owned());
    rows
}

/// "Laatst gebruikt" of a profile (connection.timestamp, seconds).
pub fn last_used(settings: &Value) -> String {
    match settings["connection"]["timestamp"].as_u64() {
        Some(ts) if ts > 0 => kit::format::date(ts as f64 * 1000.0, kit::format::DateFormat::DateTime),
        _ => String::new(),
    }
}

// ------------------------------------------------------------------------------------------ small widgets

/// The "Kopiëren" button of a property list.
pub fn copy_button(p: &Page, rows: Vec<(String, String)>, name: &str) -> gtk::Button {
    let copy = c::button(t("Kopiëren", "Copy"), Variant::Standard);
    copy.set_widget_name(name);
    let pp = p.clone();
    copy.connect_clicked(move |_| {
        let text = rows.iter().map(|(k, v)| format!("{k}\t{v}")).collect::<Vec<_>>().join("\n");
        let ok = common::copy_text(&text);
        pp.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).severity(if ok { Severity::Success } else { Severity::Error }).timeout(2000));
    });
    copy
}

pub fn entry(text: &str, placeholder: &str, name: &str) -> gtk::Entry {
    let e = c::text_box(placeholder);
    e.set_text(text);
    e.set_hexpand(true);
    e.set_widget_name(name);
    e
}

pub(super) fn password_entry(placeholder: &str, name: &str) -> gtk::Entry {
    let e = entry("", placeholder, name);
    e.set_visibility(false);
    e.set_input_purpose(gtk::InputPurpose::Password);
    e
}

pub(super) fn section(text: &str) -> gtk::Label {
    c::label(text, "st-net-form-section", true)
}

pub fn error_label() -> gtk::Label {
    let l = c::label("", "st-net-error", true);
    l.set_visible(false);
    l
}

pub fn show_error(l: &gtk::Label, m: &str) {
    l.set_label(m);
    l.set_visible(!m.is_empty());
}

/// A form in a scrolling column (a long dialog stays inside the window).
pub fn scroller(form: &gtk::Box) -> gtk::Widget {
    let sw = c::scrolled(form);
    sw.set_propagate_natural_height(true);
    sw.set_max_content_height(520);
    sw.set_min_content_width(440);
    sw.upcast()
}

/// "Tekens weergeven" for the password entries of a form.
pub(super) fn show_characters(entries: &[gtk::Entry]) -> gtk::CheckButton {
    let cb = c::checkbox(t("Tekens weergeven", "Show characters"));
    let list = entries.to_vec();
    cb.connect_toggled(move |b| {
        for e in &list {
            e.set_visibility(b.is_active());
        }
    });
    cb
}

/// Rows of entries with a remove button each, and an add button (addresses, routes).
pub(super) struct Rows {
    pub(super) root: gtk::Box,
    list: gtk::Box,
    cols: Vec<&'static str>,
    rows: RefCell<Vec<(gtk::Box, Vec<gtk::Entry>)>>,
    prefix: String,
}

impl Rows {
    pub(super) fn new(cols: Vec<&'static str>, values: Vec<Vec<String>>, add: &str, prefix: &str) -> Rc<Rows> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 6);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
        root.append(&list);
        let me = Rc::new(Rows { root: root.clone(), list, cols, rows: RefCell::new(Vec::new()), prefix: prefix.to_owned() });
        for v in values {
            me.add_row(&v);
        }
        let btn = c::button_with_glyph("plus", add, Variant::Standard);
        btn.set_halign(gtk::Align::Start);
        btn.set_widget_name(&format!("{prefix}-add"));
        let weak = Rc::downgrade(&me);
        btn.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade() {
                me.add_row(&[]);
            }
        });
        root.append(&btn);
        me
    }

    fn add_row(self: &Rc<Self>, values: &[String]) {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let n = self.rows.borrow().len();
        let entries: Vec<gtk::Entry> = self
            .cols
            .iter()
            .enumerate()
            .map(|(i, ph)| {
                let e = entry(values.get(i).map(String::as_str).unwrap_or(""), ph, &format!("{}-{n}-{i}", self.prefix));
                e.add_css_class("st-net-row-entry");
                row.append(&e);
                e
            })
            .collect();
        let rm = c::icon_button("delete", t("Verwijderen", "Remove"), 32);
        let (weak, r) = (Rc::downgrade(self), row.clone());
        rm.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade() {
                me.list.remove(&r);
                me.rows.borrow_mut().retain(|(b, _)| b != &r);
            }
        });
        row.append(&rm);
        self.list.append(&row);
        self.rows.borrow_mut().push((row, entries));
    }

    /// The filled rows' texts (a row with every entry empty is left out).
    pub(super) fn values(&self) -> Vec<Vec<String>> {
        self.rows
            .borrow()
            .iter()
            .map(|(_, es)| es.iter().map(|e| e.text().trim().to_owned()).collect::<Vec<_>>())
            .filter(|v| v.iter().any(|x| !x.is_empty()))
            .collect()
    }
}

/// Run a host call for a dialog: busy meanwhile, closed on success (then `ok(answer)`), the error under the form.
pub fn dialog_call(d: &Dialog, err: &gtk::Label, method: &str, args: Value, ok: impl FnOnce(Value) + 'static) {
    d.set_busy(true);
    let wait = super::call(method, args);
    let (d, err) = (d.clone(), err.clone());
    glib::spawn_future_local(async move {
        match wait.await {
            Ok(v) => {
                d.close(DialogResult::Primary);
                ok(v);
            }
            Err(e) => {
                d.set_busy(false);
                show_error(&err, &e.message);
            }
        }
    });
}

type Changes = Rc<dyn Fn() -> Result<Value, String>>;

/// A dialog that collects `changes()` (Err: the message under the form), saves them with net.connection.update
/// and then runs `saved()`.
fn edit_dialog(p: &Page, title: &str, form: &gtk::Box, uuid: &str, changes: Changes, saved: Rc<dyn Fn()>) {
    let err = error_label();
    form.append(&err);
    let (uuid, anchor) = (uuid.to_owned(), p.anchor());
    Dialog::new(title)
        .body(&scroller(form))
        .primary(t("Opslaan", "Save"))
        .width(520)
        .on_primary(move |d| {
            match changes() {
                Err(m) => show_error(&err, &m),
                Ok(ch) => {
                    show_error(&err, "");
                    let (saved, anchor) = (saved.clone(), anchor.clone());
                    dialog_call(d, &err, "net.connection.update", json!({ "uuid": uuid, "settings": ch }), move |v| {
                        common::note_dry_run(&anchor, &v);
                        saved();
                    });
                }
            }
            false
        })
        .show(&p.anchor(), |_| {});
}

fn form_box() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 12)
}

// ------------------------------------------------------------------------------------------ IP

/// One family of the IP dialog.
struct FamilyForm {
    six: bool,
    method: Dropdown,
    addrs: Rc<Rows>,
    gateway: gtk::Entry,
    auto_routes: gtk::CheckButton,
    routes: Rc<Rows>,
    never_default: gtk::CheckButton,
}

impl FamilyForm {
    fn new(settings: &Value, six: bool, into: &gtk::Box) -> FamilyForm {
        let f = fam(six);
        into.append(&section(if six { "IPv6" } else { "IPv4" }));
        let table = if six { V6_METHODS } else { V4_METHODS };
        let current = method(settings, six).to_owned();
        let mut opts = options(table);
        if !table.iter().any(|r| r.0 == current) {
            opts.push(DropdownOption::new(&current, &current));
        }
        let dd = Dropdown::new(opts, Some(&current));
        dd.widget().set_widget_name(&format!("net-ip-{f}-method"));
        into.append(&c::field(t("IP-toewijzing", "IP assignment"), dd.widget(), None));
        let manual = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let prefix_col = if six { t("Lengte van subnetvoorvoegsel", "Subnet prefix length") } else { t("Subnetmasker", "Subnet mask") };
        let addr_values = addresses(settings, six).into_iter().map(|(a, p)| vec![a, if six { p.to_string() } else { v4_mask(p) }]).collect();
        let addrs = Rows::new(vec![t("IP-adres", "IP address"), prefix_col], addr_values, t("Adres toevoegen", "Add address"), &format!("net-ip-{f}-address"));
        manual.append(&c::field(t("Adressen", "Addresses"), &addrs.root, None));
        let gateway = entry(s(&settings[f]["gateway"]), "", &format!("net-ip-{f}-gateway"));
        manual.append(&c::field(t("Gateway", "Gateway"), &gateway, None));
        into.append(&manual);
        let show_manual = |m: &str| matches!(m, "manual" | "shared");
        manual.set_visible(show_manual(&current));
        {
            let manual = manual.clone();
            dd.connect_changed(move |m| manual.set_visible(show_manual(m)));
        }
        // GNOME's routes and "only for resources on its network", under Windows' fields
        let adv = kit::controls::Expander::new("", t("Geavanceerd", "Advanced"), t("Routes en het gebruik van deze verbinding", "Routes and how this connection is used"));
        let advanced = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let auto_routes = c::checkbox(t("Routes automatisch ophalen", "Get routes automatically"));
        auto_routes.set_active(settings[f]["ignore-auto-routes"] != true);
        auto_routes.set_widget_name(&format!("net-ip-{f}-auto-routes"));
        advanced.append(&auto_routes);
        let route_values = routes(settings, six)
            .into_iter()
            .map(|(d, p, nh, m)| vec![d, if six { p.to_string() } else { v4_mask(p) }, nh, m.map(|m| m.to_string()).unwrap_or_default()])
            .collect();
        let routes = Rows::new(
            vec![t("Adres", "Address"), prefix_col, t("Gateway", "Gateway"), t("Metrische waarde", "Metric")],
            route_values,
            t("Route toevoegen", "Add route"),
            &format!("net-ip-{f}-route"),
        );
        advanced.append(&c::field(t("Routes", "Routes"), &routes.root, None));
        let never_default = c::checkbox(t("Deze verbinding alleen gebruiken voor bronnen in het eigen netwerk", "Use this connection only for resources on its network"));
        never_default.set_active(settings[f]["never-default"] == true);
        never_default.set_widget_name(&format!("net-ip-{f}-never-default"));
        advanced.append(&never_default);
        adv.set_content(&advanced);
        into.append(adv.widget());
        FamilyForm { six, method: dd, addrs, gateway, auto_routes, routes, never_default }
    }

    fn prefix(&self, text: &str) -> Option<u32> {
        if self.six { v6_prefix(text) } else { v4_prefix(text) }
    }

    fn changes(&self) -> Result<Value, String> {
        let m = self.method.selected().unwrap_or_else(|| "auto".into());
        let family = if self.six { "IPv6" } else { "IPv4" };
        let err = |template: &str, v: &str| common::fill(template, &[("family", family), ("value", v)]);
        let bad_address = |v: &str| err(t("{family}: “{value}” is geen geldig IP-adres.", "{family}: “{value}” isn’t a valid IP address."), v);
        let bad_prefix = |v: &str| err(t("{family}: “{value}” is geen geldig subnetmasker of voorvoegsel.", "{family}: “{value}” isn’t a valid subnet mask or prefix."), v);
        let bad_gateway = |v: &str| err(t("{family}: “{value}” is geen geldige gateway.", "{family}: “{value}” isn’t a valid gateway."), v);
        let mut out = json!({ "method": m });
        if matches!(m.as_str(), "manual" | "shared") {
            let mut data = Vec::new();
            for row in self.addrs.values() {
                if !ip_ok(&row[0], self.six) {
                    return Err(bad_address(&row[0]));
                }
                let prefix = self.prefix(&row[1]).ok_or_else(|| bad_prefix(&row[1]))?;
                data.push(json!({ "address": row[0], "prefix": prefix }));
            }
            if m == "manual" && data.is_empty() {
                return Err(common::fill(t("{family}: typ ten minste één IP-adres.", "{family}: type at least one IP address."), &[("family", family)]));
            }
            let gw = self.gateway.text().trim().to_owned();
            if !gw.is_empty() && !ip_ok(&gw, self.six) {
                return Err(bad_gateway(&gw));
            }
            out["address-data"] = json!(data);
            out["gateway"] = if gw.is_empty() { Value::Null } else { json!(gw) };
        } else {
            // Windows: an automatic address has no static ones next to it
            out["address-data"] = json!([]);
            out["gateway"] = Value::Null;
        }
        let mut rdata = Vec::new();
        for row in self.routes.values() {
            if !ip_ok(&row[0], self.six) {
                return Err(bad_address(&row[0]));
            }
            let prefix = self.prefix(&row[1]).ok_or_else(|| bad_prefix(&row[1]))?;
            let mut r = json!({ "dest": row[0], "prefix": prefix });
            if !row[2].is_empty() {
                if !ip_ok(&row[2], self.six) {
                    return Err(bad_gateway(&row[2]));
                }
                r["next-hop"] = json!(row[2]);
            }
            if !row[3].is_empty() {
                let metric = row[3].parse::<u32>().map_err(|_| err(t("{family}: “{value}” is geen geldige metrische waarde.", "{family}: “{value}” isn’t a valid metric."), &row[3]))?;
                r["metric"] = json!(metric);
            }
            rdata.push(r);
        }
        out["route-data"] = json!(rdata);
        out["ignore-auto-routes"] = json!(!self.auto_routes.is_active());
        out["never-default"] = json!(self.never_default.is_active());
        Ok(out)
    }
}

/// "IP-instellingen bewerken" (Windows' IP assignment · Bewerken; GNOME's IPv4 and IPv6 pages).
pub fn edit_ip(p: &Page, uuid: &str, settings: &Value, saved: Rc<dyn Fn()>) {
    let form = form_box();
    let v4 = FamilyForm::new(settings, false, &form);
    let v6 = FamilyForm::new(settings, true, &form);
    let changes: Changes = Rc::new(move || Ok(json!({ "ipv4": v4.changes()?, "ipv6": v6.changes()? })));
    edit_dialog(p, t("IP-instellingen bewerken", "Edit IP settings"), &form, uuid, changes, saved);
}

// ------------------------------------------------------------------------------------------ DNS

struct DnsFamily {
    six: bool,
    on: gtk::CheckButton,
    preferred: gtk::Entry,
    alternate: gtk::Entry,
    more: gtk::Entry,
}

/// "DNS-serverinstellingen bewerken" (Windows' DNS server assignment · Bewerken; GNOME's Automatic DNS and servers).
pub fn edit_dns(p: &Page, uuid: &str, settings: &Value, saved: Rc<dyn Fn()>) {
    let form = form_box();
    let manual_now = dns_manual(settings);
    let mode = Dropdown::new(options(&V4_METHODS[..2]), Some(if manual_now { "manual" } else { "auto" }));
    mode.widget().set_widget_name("net-dns-mode");
    form.append(&c::field(t("DNS-servertoewijzing", "DNS server assignment"), mode.widget(), None));
    let manual = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let mut fams = Vec::new();
    for six in [false, true] {
        let list = dns_list(settings, six);
        let f = fam(six);
        let on = c::checkbox(if six { "IPv6" } else { "IPv4" });
        on.set_active(!list.is_empty() || settings[f]["ignore-auto-dns"] == true);
        on.set_widget_name(&format!("net-dns-{f}"));
        manual.append(&on);
        let preferred = entry(list.first().map(String::as_str).unwrap_or(""), "", &format!("net-dns-{f}-preferred"));
        let alternate = entry(list.get(1).map(String::as_str).unwrap_or(""), "", &format!("net-dns-{f}-alternate"));
        let more = entry(&list.iter().skip(2).cloned().collect::<Vec<_>>().join(", "), t("Gescheiden door komma's", "Separated by commas"), &format!("net-dns-{f}-more"));
        let fields = gtk::Box::new(gtk::Orientation::Vertical, 8);
        fields.append(&c::field(t("Voorkeurs-DNS", "Preferred DNS"), &preferred, None));
        fields.append(&c::field(t("Alternatieve DNS", "Alternate DNS"), &alternate, None));
        fields.append(&c::field(t("Meer DNS-servers", "More DNS servers"), &more, None));
        fields.set_visible(on.is_active());
        {
            let fields = fields.clone();
            on.connect_toggled(move |b| fields.set_visible(b.is_active()));
        }
        manual.append(&fields);
        fams.push(DnsFamily { six, on, preferred, alternate, more });
    }
    // GNOME's "Automatic" DNS switch: the network's servers next to the typed ones
    let also = c::checkbox(t("Ook de DNS-servers van het netwerk gebruiken", "Also use the network’s DNS servers"));
    also.set_active(manual_now && [false, true].iter().all(|six| settings[fam(*six)]["ignore-auto-dns"] != true));
    also.set_widget_name("net-dns-also-auto");
    manual.append(&also);
    manual.set_visible(manual_now);
    {
        let manual = manual.clone();
        mode.connect_changed(move |m| manual.set_visible(m == "manual"));
    }
    form.append(&manual);
    let changes: Changes = Rc::new(move || {
        let manual = mode.selected().as_deref() == Some("manual");
        let mut out = json!({});
        for f in &fams {
            let name = fam(f.six);
            if !manual || !f.on.is_active() {
                out[name] = json!({ "dns-data": [], "ignore-auto-dns": false });
                continue;
            }
            let mut servers: Vec<String> = Vec::new();
            let more = f.more.text().to_string();
            let typed = [f.preferred.text().to_string(), f.alternate.text().to_string()].into_iter().chain(more.split(',').map(str::to_owned));
            for text in typed {
                let v = text.trim();
                if v.is_empty() {
                    continue;
                }
                if !ip_ok(v, f.six) {
                    return Err(common::fill(t("“{value}” is geen geldig DNS-serveradres.", "“{value}” isn’t a valid DNS server address."), &[("value", v)]));
                }
                servers.push(v.to_owned());
            }
            out[name] = json!({ "dns-data": servers, "ignore-auto-dns": !servers.is_empty() && !also.is_active() });
        }
        Ok(out)
    });
    edit_dialog(p, t("DNS-serverinstellingen bewerken", "Edit DNS settings"), &form, uuid, changes, saved);
}

// ------------------------------------------------------------------------------------------ 802.1X

/// The 802-1x keys the EAP form manages; the others (subject-match, system-ca-certs, ...) stay as they are.
const EAP_KEYS: &[&str] = &[
    "identity",
    "anonymous-identity",
    "domain-suffix-match",
    "ca-cert",
    "client-cert",
    "private-key",
    "private-key-password",
    "private-key-password-flags",
    "password",
    "password-flags",
    "phase1-peapver",
    "phase1-fast-provisioning",
    "pac-file",
    "phase2-auth",
    "phase2-autheap",
];

/// The keys each EAP method uses (of `EAP_KEYS`).
pub fn eap_keys(method: &str) -> &'static [&'static str] {
    match method {
        "tls" => &["identity", "domain-suffix-match", "ca-cert", "client-cert", "private-key", "private-key-password", "private-key-password-flags"],
        "leap" | "pwd" => &["identity", "password", "password-flags"],
        "fast" => &["anonymous-identity", "phase1-fast-provisioning", "pac-file", "phase2-autheap", "identity", "password", "password-flags"],
        "ttls" => &["anonymous-identity", "domain-suffix-match", "ca-cert", "phase2-auth", "phase2-autheap", "identity", "password", "password-flags"],
        _ => &["anonymous-identity", "domain-suffix-match", "ca-cert", "phase1-peapver", "phase2-auth", "identity", "password", "password-flags"],
    }
}

/// A file field ("CA-certificaat": a path and "Bladeren…").
pub(super) fn file_field(p: &Page, label: &str, path: &str, name: &str) -> (gtk::Box, gtk::Entry) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let e = entry(path, t("Geen", "None"), name);
    row.append(&e);
    let browse = c::button(t("Bladeren…", "Browse…"), Variant::Standard);
    let (pp, e2) = (p.clone(), e.clone());
    browse.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::new();
        dialog.set_title(t("Een bestand kiezen", "Choose a file"));
        dialog.set_modal(true);
        let e3 = e2.clone();
        dialog.open(Some(pp.window().window()), gio::Cancellable::NONE, move |r| {
            if let Ok(f) = r
                && let Some(path) = f.path()
            {
                e3.set_text(&path.to_string_lossy());
            }
        });
    });
    row.append(&browse);
    (c::field(label, &row, None), e)
}

fn table_pick(table: &[(&str, &str, &str)], v: &str, fallback: &'static str) -> String {
    if table.iter().any(|r| r.0 == v) { v.to_owned() } else { fallback.to_owned() }
}

/// The 802.1X form (Ethernet's "Verificatie-instellingen", Wi-Fi's WPA-Enterprise and dynamic WEP).
pub struct EapForm {
    root: gtk::Box,
    method: Dropdown,
    identity: gtk::Entry,
    anonymous: gtk::Entry,
    domain: gtk::Entry,
    ca: gtk::Entry,
    client: gtk::Entry,
    key: gtk::Entry,
    key_password: gtk::Entry,
    password: gtk::Entry,
    store: Dropdown,
    peap_version: Dropdown,
    peap_inner: Dropdown,
    ttls_inner: Dropdown,
    fast_inner: Dropdown,
    provisioning: Dropdown,
    pac: gtk::Entry,
    stored: bool,
}

impl EapForm {
    /// `x` is the profile's 802-1x group (Null for none).
    pub fn new(p: &Page, x: &Value) -> Rc<EapForm> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let current = table_pick(EAP, s(&x["eap"][0]), "peap");
        let method = Dropdown::new(options(EAP), Some(&current));
        method.widget().set_widget_name("net-eap-method");
        root.append(&c::field(t("Verificatiemethode", "Authentication method"), method.widget(), None));
        let field = |label: &str, w: &gtk::Widget| {
            let f = c::field(label, w, None);
            root.append(&f);
            f
        };
        let stored = x.is_object() && x["password-flags"].as_u64() != Some(2);
        let unchanged = t("Ongewijzigd", "Unchanged");
        let anonymous = entry(s(&x["anonymous-identity"]), "", "net-eap-anonymous");
        let f_anon = field(t("Anonieme identiteit", "Anonymous identity"), anonymous.upcast_ref());
        let domain = entry(s(&x["domain-suffix-match"]), "", "net-eap-domain");
        let f_domain = field(t("Domein", "Domain"), domain.upcast_ref());
        let (f_ca, ca) = file_field(p, t("CA-certificaat", "CA certificate"), &cert_path(&x["ca-cert"]), "net-eap-ca");
        root.append(&f_ca);
        let (f_client, client) = file_field(p, t("Gebruikerscertificaat", "User certificate"), &cert_path(&x["client-cert"]), "net-eap-client");
        root.append(&f_client);
        let (f_key, key) = file_field(p, t("Persoonlijke sleutel", "Private key"), &cert_path(&x["private-key"]), "net-eap-key");
        root.append(&f_key);
        let key_password = password_entry(if x.is_object() { unchanged } else { "" }, "net-eap-key-password");
        let f_key_pw = field(t("Wachtwoord van persoonlijke sleutel", "Private key password"), key_password.upcast_ref());
        let peap_version = Dropdown::new(options(PEAP_VERSION), Some(&table_pick(PEAP_VERSION, s(&x["phase1-peapver"]), "auto")));
        let f_peapver = field(t("PEAP-versie", "PEAP version"), peap_version.widget().upcast_ref());
        let provisioning = Dropdown::new(options(FAST_PROVISIONING), Some(&table_pick(FAST_PROVISIONING, s(&x["phase1-fast-provisioning"]), "1")));
        let f_prov = field(t("PAC-inrichting", "PAC provisioning"), provisioning.widget().upcast_ref());
        let pac = entry(s(&x["pac-file"]), t("Geen", "None"), "net-eap-pac");
        let f_pac = field(t("PAC-bestand", "PAC file"), pac.upcast_ref());
        let peap_inner = Dropdown::new(options(PEAP_INNER), Some(&table_pick(PEAP_INNER, s(&x["phase2-auth"]), "mschapv2")));
        let ttls_now = if s(&x["phase2-autheap"]).is_empty() { format!("auth:{}", s(&x["phase2-auth"])) } else { format!("eap:{}", s(&x["phase2-autheap"])) };
        let ttls_inner = Dropdown::new(options(TTLS_INNER), Some(&table_pick(TTLS_INNER, &ttls_now, "auth:pap")));
        let fast_inner = Dropdown::new(options(FAST_INNER), Some(&table_pick(FAST_INNER, s(&x["phase2-autheap"]), "gtc")));
        let f_peap_inner = field(t("Interne verificatie", "Inner authentication"), peap_inner.widget().upcast_ref());
        let f_ttls_inner = field(t("Interne verificatie", "Inner authentication"), ttls_inner.widget().upcast_ref());
        let f_fast_inner = field(t("Interne verificatie", "Inner authentication"), fast_inner.widget().upcast_ref());
        let identity = entry(s(&x["identity"]), "", "net-eap-identity");
        let f_identity = field(t("Gebruikersnaam", "User name"), identity.upcast_ref());
        let password = password_entry(if stored { unchanged } else { "" }, "net-eap-password");
        let f_password = field(t("Wachtwoord", "Password"), password.upcast_ref());
        let store = Dropdown::new(options(SECRET_STORE), Some(&x["password-flags"].as_u64().unwrap_or(1).min(2).to_string()));
        store.widget().set_widget_name("net-eap-store");
        let f_store = field(t("Wachtwoord opslaan", "Save password"), store.widget().upcast_ref());
        root.append(&show_characters(&[password.clone(), key_password.clone()]));
        let parts: Vec<(&'static str, gtk::Box)> = vec![
            ("anonymous-identity", f_anon),
            ("domain-suffix-match", f_domain),
            ("ca-cert", f_ca),
            ("client-cert", f_client),
            ("private-key", f_key),
            ("private-key-password", f_key_pw),
            ("phase1-peapver", f_peapver),
            ("phase1-fast-provisioning", f_prov),
            ("pac-file", f_pac),
            ("identity", f_identity.clone()),
            ("password", f_password),
            ("password-flags", f_store),
        ];
        let inners = vec![("peap", f_peap_inner), ("ttls", f_ttls_inner), ("fast", f_fast_inner)];
        let show = move |m: &str| {
            let keys = eap_keys(m);
            for (k, f) in &parts {
                f.set_visible(keys.contains(k));
            }
            for (k, f) in &inners {
                f.set_visible(*k == m);
            }
            // TLS names its identity "Identiteit", the other methods a user name
            if let Some(l) = f_identity.first_child().and_downcast::<gtk::Label>() {
                l.set_label(if m == "tls" { t("Identiteit", "Identity") } else { t("Gebruikersnaam", "User name") });
            }
        };
        show(&current);
        method.connect_changed(move |m| show(m));
        Rc::new(EapForm {
            root,
            method,
            identity,
            anonymous,
            domain,
            ca,
            client,
            key,
            key_password,
            password,
            store,
            peap_version,
            peap_inner,
            ttls_inner,
            fast_inner,
            provisioning,
            pac,
            stored,
        })
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    /// The 802-1x group: the method's keys, null for the managed keys the method does not use; a password left
    /// empty keeps the stored one.
    pub fn changes(&self) -> Result<Value, String> {
        let m = self.method.selected().unwrap_or_else(|| "peap".into());
        let keys = eap_keys(&m);
        let mut out = json!({ "eap": [m] });
        let o = out.as_object_mut().expect("an object");
        for k in EAP_KEYS {
            if !keys.contains(k) {
                o.insert((*k).to_owned(), Value::Null);
            }
        }
        let text = |e: &gtk::Entry| e.text().trim().to_owned();
        let opt = |v: String| if v.is_empty() { Value::Null } else { json!(v) };
        let file = |e: &gtk::Entry| -> Result<Value, String> {
            let v = text(e);
            if v.is_empty() {
                return Ok(Value::Null);
            }
            if !v.starts_with('/') {
                return Err(common::fill(t("“{value}” is geen bestand. Kies een bestand met Bladeren.", "“{value}” isn’t a file. Choose a file with Browse."), &[("value", &v)]));
            }
            Ok(json!(v))
        };
        let identity = text(&self.identity);
        if identity.is_empty() {
            return Err(if m == "tls" { t("Typ een identiteit.", "Type an identity.") } else { t("Typ een gebruikersnaam.", "Type a user name.") }.to_owned());
        }
        o.insert("identity".into(), json!(identity));
        if keys.contains(&"anonymous-identity") {
            o.insert("anonymous-identity".into(), opt(text(&self.anonymous)));
        }
        if keys.contains(&"domain-suffix-match") {
            o.insert("domain-suffix-match".into(), opt(text(&self.domain)));
        }
        if keys.contains(&"ca-cert") {
            o.insert("ca-cert".into(), file(&self.ca)?);
        }
        if m == "tls" {
            let (client, key) = (file(&self.client)?, file(&self.key)?);
            if client.is_null() || key.is_null() {
                return Err(t("TLS heeft een gebruikerscertificaat en een persoonlijke sleutel nodig.", "TLS needs a user certificate and a private key.").to_owned());
            }
            o.insert("client-cert".into(), client);
            o.insert("private-key".into(), key);
            let kp = self.key_password.text().to_string();
            if kp.is_empty() {
                o.remove("private-key-password");
            } else {
                o.insert("private-key-password".into(), json!(kp));
            }
            o.insert("private-key-password-flags".into(), json!(0));
        }
        if keys.contains(&"password") {
            let flags: u64 = self.store.selected().and_then(|v| v.parse().ok()).unwrap_or(1);
            o.insert("password-flags".into(), json!(flags));
            let pw = self.password.text().to_string();
            if flags == 2 {
                o.insert("password".into(), Value::Null);
            } else if !pw.is_empty() {
                o.insert("password".into(), json!(pw));
            } else if self.stored {
                o.remove("password");
            } else {
                return Err(t("Typ het wachtwoord.", "Type the password.").to_owned());
            }
        }
        match m.as_str() {
            "peap" => {
                let v = match self.peap_version.selected().as_deref() {
                    Some(v @ ("0" | "1")) => json!(v),
                    _ => Value::Null,
                };
                o.insert("phase1-peapver".into(), v);
                o.insert("phase2-auth".into(), json!(self.peap_inner.selected().unwrap_or_else(|| "mschapv2".into())));
            }
            "ttls" => {
                let inner = self.ttls_inner.selected().unwrap_or_else(|| "auth:pap".into());
                let (kind, value) = inner.split_once(':').unwrap_or(("auth", "pap"));
                let (set, clear) = if kind == "eap" { ("phase2-autheap", "phase2-auth") } else { ("phase2-auth", "phase2-autheap") };
                o.insert(set.into(), json!(value));
                o.insert(clear.into(), Value::Null);
            }
            "fast" => {
                o.insert("phase1-fast-provisioning".into(), json!(self.provisioning.selected().unwrap_or_else(|| "1".into())));
                o.insert("phase2-autheap".into(), json!(self.fast_inner.selected().unwrap_or_else(|| "gtc".into())));
                o.insert("pac-file".into(), opt(text(&self.pac)));
            }
            _ => {}
        }
        Ok(out)
    }
}

/// "Verificatie-instellingen bewerken" (Windows' Ethernet authentication · Bewerken; GNOME's 802.1x Security).
pub fn edit_8021x(p: &Page, uuid: &str, settings: &Value, saved: Rc<dyn Fn()>) {
    let form = form_box();
    let on = c::checkbox(t("IEEE 802.1X-verificatie inschakelen", "Enable IEEE 802.1X authentication"));
    on.set_active(settings["802-1x"].is_object());
    on.set_widget_name("net-8021x-on");
    form.append(&on);
    let eap = EapForm::new(p, &settings["802-1x"]);
    eap.widget().set_visible(on.is_active());
    {
        let w = eap.widget().clone();
        on.connect_toggled(move |b| w.set_visible(b.is_active()));
    }
    form.append(eap.widget());
    let changes: Changes = Rc::new(move || if on.is_active() { Ok(json!({ "802-1x": eap.changes()? })) } else { Ok(json!({ "802-1x": null })) });
    edit_dialog(p, t("Verificatie-instellingen bewerken", "Edit authentication settings"), &form, uuid, changes, saved);
}

// ------------------------------------------------------------------------------------------ Wi-Fi security

/// The security form of a Wi-Fi network: the type, and its key, user name and password or 802.1X fields.
pub struct SecurityForm {
    root: gtk::Box,
    kind: Dropdown,
    key: gtk::Entry,
    wep_index: Dropdown,
    wep_auth: Dropdown,
    leap_user: gtk::Entry,
    store: Dropdown,
    eap: Rc<EapForm>,
    stored: bool,
    current: String,
}

impl SecurityForm {
    /// `settings` is the profile (Null for a new network); `kind` the type to start with.
    pub fn new(p: &Page, settings: &Value, kind: &str) -> Rc<SecurityForm> {
        let sec = &settings["802-11-wireless-security"];
        let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let kind_dd = Dropdown::new(options(SECURITY), Some(kind));
        kind_dd.widget().set_widget_name("net-security-type");
        root.append(&c::field(t("Beveiligingstype", "Security type"), kind_dd.widget(), None));
        let stored = settings.is_object() && kind != "open";
        let key = password_entry(if stored { t("Ongewijzigd", "Unchanged") } else { "" }, "net-security-key");
        let f_key = c::field(t("Beveiligingssleutel", "Security key"), &key, None);
        root.append(&f_key);
        let index_now = sec["wep-tx-keyidx"].as_u64().unwrap_or(0).min(3).to_string();
        let wep_index = Dropdown::simple(&[("0", "1"), ("1", "2"), ("2", "3"), ("3", "4")], Some(&index_now));
        let f_index = c::field(t("WEP-index", "WEP index"), wep_index.widget(), None);
        root.append(&f_index);
        let wep_auth = Dropdown::simple(
            &[("open", t("Open systeem", "Open system")), ("shared", t("Gedeelde sleutel", "Shared key"))],
            Some(if sec["auth-alg"] == "shared" { "shared" } else { "open" }),
        );
        let f_auth = c::field(t("Verificatie", "Authentication"), wep_auth.widget(), None);
        root.append(&f_auth);
        let leap_user = entry(s(&sec["leap-username"]), "", "net-security-leap-user");
        let f_user = c::field(t("Gebruikersnaam", "User name"), &leap_user, None);
        root.append(&f_user);
        let flags_key = match kind {
            "leap" => "leap-password-flags",
            "wep" | "wep-passphrase" => "wep-key-flags",
            _ => "psk-flags",
        };
        let store = Dropdown::new(options(SECRET_STORE), Some(&sec[flags_key].as_u64().unwrap_or(1).min(2).to_string()));
        store.widget().set_widget_name("net-security-store");
        let f_store = c::field(t("Wachtwoord opslaan", "Save password"), store.widget(), None);
        root.append(&f_store);
        let show_chars = show_characters(std::slice::from_ref(&key));
        root.append(&show_chars);
        let eap = EapForm::new(p, &settings["802-1x"]);
        root.append(eap.widget());
        let eap_w = eap.widget().clone();
        let show = move |k: &str| {
            let wep = matches!(k, "wep" | "wep-passphrase");
            let with_key = wep || matches!(k, "wpa-psk" | "sae" | "leap");
            for w in [f_key.upcast_ref::<gtk::Widget>(), show_chars.upcast_ref(), f_store.upcast_ref()] {
                w.set_visible(with_key);
            }
            f_index.set_visible(wep);
            f_auth.set_visible(wep);
            f_user.set_visible(k == "leap");
            eap_w.set_visible(matches!(k, "wpa-eap" | "dynamic-wep"));
            if let Some(l) = f_key.first_child().and_downcast::<gtk::Label>() {
                l.set_label(if k == "leap" { t("Wachtwoord", "Password") } else { t("Beveiligingssleutel", "Security key") });
            }
        };
        show(kind);
        kind_dd.connect_changed(move |k| show(k));
        Rc::new(SecurityForm { root, kind: kind_dd, key, wep_index, wep_auth, leap_user, store, eap, stored, current: kind.to_owned() })
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.root
    }

    pub fn kind(&self) -> String {
        self.kind.selected().unwrap_or_else(|| "open".into())
    }

    /// The security groups for the chosen type (null removes the ones it does not use). A key left empty on a
    /// stored network of the same type keeps the stored one.
    pub fn changes(&self) -> Result<Value, String> {
        let kind = self.kind();
        let mut out = security_base(&kind);
        let key = self.key.text().to_string();
        let keep = key.is_empty() && self.stored && kind == self.current;
        let flags: u64 = self.store.selected().and_then(|v| v.parse().ok()).unwrap_or(1);
        if matches!(kind.as_str(), "wpa-eap" | "dynamic-wep") {
            out["802-1x"] = self.eap.changes()?;
            return Ok(out);
        }
        let sec = &mut out["802-11-wireless-security"];
        match kind.as_str() {
            "wpa-psk" | "sae" => {
                sec["psk-flags"] = json!(flags);
                if flags == 2 {
                    sec["psk"] = Value::Null;
                } else if !keep {
                    let ok = if kind == "sae" { !key.is_empty() && key.len() <= 128 } else { psk_ok(&key) };
                    if !ok {
                        return Err(t("De beveiligingssleutel moet uit 8 tot 63 tekens bestaan.", "The security key must be 8 to 63 characters.").to_owned());
                    }
                    sec["psk"] = json!(key);
                }
            }
            "wep" | "wep-passphrase" => {
                let index: u64 = self.wep_index.selected().and_then(|v| v.parse().ok()).unwrap_or(0);
                sec["wep-tx-keyidx"] = json!(index);
                sec["auth-alg"] = json!(self.wep_auth.selected().unwrap_or_else(|| "open".into()));
                sec["wep-key-flags"] = json!(flags);
                let slot = format!("wep-key{index}");
                if flags == 2 {
                    sec[slot.as_str()] = Value::Null;
                } else if !keep {
                    let ok = if kind == "wep" { wep_key_ok(&key) } else { !key.is_empty() && key.len() <= 64 };
                    if !ok {
                        return Err(t("Typ een geldige WEP-sleutel.", "Type a valid WEP key.").to_owned());
                    }
                    sec[slot.as_str()] = json!(key);
                }
            }
            "leap" => {
                let user = self.leap_user.text().trim().to_owned();
                if user.is_empty() {
                    return Err(t("Typ een gebruikersnaam.", "Type a user name.").to_owned());
                }
                sec["leap-username"] = json!(user);
                sec["leap-password-flags"] = json!(flags);
                if flags == 2 {
                    sec["leap-password"] = Value::Null;
                } else if !keep {
                    if key.is_empty() {
                        return Err(t("Typ het wachtwoord.", "Type the password.").to_owned());
                    }
                    sec["leap-password"] = json!(key);
                }
            }
            _ => {}
        }
        Ok(out)
    }
}

/// "Beveiligingsinstellingen bewerken" (Wi-Fi properties' security type · Bewerken; GNOME's Security tab).
pub fn edit_security(p: &Page, uuid: &str, settings: &Value, saved: Rc<dyn Fn()>) {
    let form = form_box();
    let sec = SecurityForm::new(p, settings, security_kind(settings));
    form.append(sec.widget());
    let changes: Changes = Rc::new(move || sec.changes());
    edit_dialog(p, t("Beveiligingsinstellingen bewerken", "Edit security settings"), &form, uuid, changes, saved);
}

/// The Wi-Fi security key and its QR code (net.wifi.secrets; GNOME's Share Network).
pub fn show_key(p: &Page, uuid: &str) {
    let wait = p.run("net.wifi.secrets", json!({ "uuid": uuid }));
    let pp = p.clone();
    p.spawn(async move {
        let Some(r) = wait.await else { return };
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        let key = s(&r["key"]);
        let none = t("Dit netwerk heeft geen beveiligingssleutel.", "This network doesn’t have a security key.");
        let kl = c::label(if key.is_empty() { none } else { key }, if key.is_empty() { "" } else { "st-net-key" }, true);
        kl.set_selectable(!key.is_empty());
        kl.set_widget_name("net-wifi-key");
        body.append(&c::field(t("Netwerkbeveiligingssleutel", "Network security key"), &kl, None));
        if let Some(qr) = r["qr"].as_str().and_then(|text| crate::pickers::QrCode::new(text, 200)) {
            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.add_css_class("st-net-qr");
            frame.set_halign(gtk::Align::Center);
            frame.append(&qr);
            body.append(&frame);
            body.append(&c::label(
                t("Scan deze QR-code met een telefoon of tablet om verbinding te maken met dit netwerk.", "Scan this QR code with a phone or tablet to connect to this network."),
                "w11-text-2",
                true,
            ));
        }
        Dialog::new(&common::fill(t("Wi-Fi-beveiligingssleutel van “{ssid}”", "Wi-Fi security key of “{ssid}”"), &[("ssid", s(&r["ssid"]))]))
            .body(&body)
            .width(420)
            .show(&pp.anchor(), |_| {});
    });
}

/// What a new-network dialog collected: (ssid, security kind, security groups, autoconnect, hidden).
pub type NewNetwork = (String, String, Value, bool, bool);

/// The fields of a new network (Bekende netwerken beheren › Netwerk toevoegen; Wi-Fi › Verborgen netwerk): name,
/// security, "Automatisch verbinding maken" and (`with_hidden`) "Verbinding maken, zelfs als dit netwerk niet
/// uitzendt"; `done(dialog, error label, values)` saves.
pub fn new_network(p: &Page, title: &str, primary: &str, with_hidden: bool, done: Rc<dyn Fn(&Dialog, &gtk::Label, NewNetwork)>) {
    new_network_with(p, title, primary, ("", "wpa-psk"), with_hidden, done, |_| {});
}

/// [`new_network`] for a given network: `preset` = (SSID, security kind); a given SSID is fixed (the Wi-Fi list's
/// access point, as GNOME's Wi-Fi dialog for it). `closed(result)` runs when the dialog closes (Primary after a
/// successful `done`).
pub fn new_network_with(
    p: &Page,
    title: &str,
    primary: &str,
    preset: (&str, &str),
    with_hidden: bool,
    done: Rc<dyn Fn(&Dialog, &gtk::Label, NewNetwork)>,
    closed: impl FnOnce(DialogResult) + 'static,
) {
    let (ssid0, kind0) = preset;
    let form = form_box();
    let name = entry(ssid0, "", "net-new-ssid");
    if !ssid0.is_empty() {
        name.set_editable(false);
        name.set_can_focus(false);
    }
    form.append(&c::field(t("Netwerknaam", "Network name"), &name, None));
    let sec = SecurityForm::new(p, &Value::Null, kind0);
    form.append(sec.widget());
    let auto = c::checkbox(t("Automatisch verbinding maken", "Connect automatically"));
    auto.set_active(true);
    auto.set_widget_name("net-new-auto");
    form.append(&auto);
    let hidden = c::checkbox(t("Verbinding maken, zelfs als dit netwerk niet uitzendt", "Connect even if this network is not broadcasting"));
    hidden.set_widget_name("net-new-hidden");
    hidden.set_visible(with_hidden);
    form.append(&hidden);
    let err = error_label();
    form.append(&err);
    Dialog::new(title)
        .body(&scroller(&form))
        .primary(primary)
        .width(520)
        .on_primary(move |d| {
            // Spaces are part of an SSID, including one fixed by the visible network list.
            let ssid = name.text().to_string();
            if ssid.is_empty() || ssid.len() > 32 {
                show_error(&err, t("Typ een netwerknaam van 1 tot 32 tekens.", "Type a network name of 1 to 32 characters."));
                return false;
            }
            match sec.changes() {
                Err(m) => show_error(&err, &m),
                Ok(groups) => {
                    show_error(&err, "");
                    done(d, &err, (ssid, sec.kind(), groups, auto.is_active(), !with_hidden || hidden.is_active()));
                }
            }
            false
        })
        .show(&p.anchor(), closed);
}

/// The settings of a new Wi-Fi profile (net.connection.add) from a new-network dialog.
pub fn new_wifi_settings(n: &NewNetwork) -> Value {
    let (ssid, _, groups, auto, hidden) = n;
    let mut out = json!({
        "connection": { "id": ssid, "type": "802-11-wireless", "autoconnect": auto },
        "802-11-wireless": { "ssid": ssid, "mode": "infrastructure", "hidden": hidden },
        "ipv4": { "method": "auto" },
        "ipv6": { "method": "auto" },
    });
    for (g, v) in groups.as_object().into_iter().flatten() {
        if !v.is_null() {
            out[g.as_str()] = v.clone();
        }
    }
    out
}

/// net.connection.update of a few keys from a page control (optimistic: `revert()` after the error toast).
pub fn save_keys(p: &Page, uuid: &str, changes: Value, revert: impl FnOnce() + 'static) {
    note_own_write(uuid);
    let wait = p.run("net.connection.update", json!({ "uuid": uuid, "settings": changes }));
    let uuid = uuid.to_owned();
    p.spawn(async move {
        if wait.await.is_none() {
            revert();
        } else {
            note_own_write(&uuid);
        }
    });
}

thread_local! {
    /// The profiles a page control wrote last, with when ([`save_keys`]).
    static OWN_WRITES: RefCell<Vec<(String, std::time::Instant)>> = const { RefCell::new(Vec::new()) };
}

/// How long after its own write a page takes NetworkManager's changed profile as its own.
const OWN_WRITE_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

fn note_own_write(uuid: &str) {
    OWN_WRITES.with(|w| {
        let mut w = w.borrow_mut();
        w.retain(|(u, at)| u != uuid && at.elapsed() < OWN_WRITE_WINDOW);
        w.push((uuid.to_owned(), std::time::Instant::now()));
    });
}

/// Whether a page control wrote this profile a moment ago: a page that follows `net.changed` then keeps the profile
/// it reads back instead of building itself again (the settings harness saw the Wi-Fi properties page rebuild after
/// each of its own switches, which swallowed the next click and scrolled the page back to the top).
pub fn own_write_recent(uuid: &str) -> bool {
    OWN_WRITES.with(|w| w.borrow().iter().any(|(u, at)| u == uuid && at.elapsed() < OWN_WRITE_WINDOW))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pages_own_write_is_recognised_for_its_profile_only() {
        assert!(!own_write_recent("a-uuid"));
        note_own_write("a-uuid");
        assert!(own_write_recent("a-uuid"));
        assert!(!own_write_recent("another-uuid"), "another profile changed by another program");
        note_own_write("a-uuid");
        assert_eq!(OWN_WRITES.with(|w| w.borrow().len()), 1, "one entry per profile");
    }

    #[test]
    fn netmasks_and_prefixes() {
        assert_eq!(v4_prefix("255.255.255.0"), Some(24));
        assert_eq!(v4_prefix("255.255.255.255"), Some(32));
        assert_eq!(v4_prefix("0.0.0.0"), Some(0));
        assert_eq!(v4_prefix("255.0.255.0"), None);
        assert_eq!(v4_prefix("16"), Some(16));
        assert_eq!(v4_prefix("33"), None);
        assert_eq!(v4_mask(24), "255.255.255.0");
        assert_eq!(v4_mask(0), "0.0.0.0");
        assert_eq!(v4_mask(32), "255.255.255.255");
        assert_eq!(v4_mask(20), "255.255.240.0");
        assert_eq!(v6_prefix("64"), Some(64));
        assert_eq!(v6_prefix("0"), None);
        assert!(ip_ok("192.168.1.1", false) && !ip_ok("192.168.1", false));
        assert!(ip_ok("fe80::1", true) && !ip_ok("fe80::1", false));
    }

    #[test]
    fn the_settings_read_as_windows_shows_them() {
        kit::i18n::set_forced_lang(Some("en"));
        let auto = json!({ "ipv4": { "method": "auto" }, "ipv6": { "method": "auto" } });
        assert_eq!(ip_assignment_text(&auto), "Automatic (DHCP)");
        assert_eq!(dns_text(&auto), "Automatic (DHCP)");
        let manual = json!({ "ipv4": { "method": "manual", "dns-data": ["9.9.9.9"], "address-data": [{ "address": "10.0.0.2", "prefix": 8 }] }, "ipv6": { "method": "auto" } });
        assert_eq!(ip_assignment_text(&manual), "Manual");
        assert_eq!(dns_text(&manual), "Manual");
        assert_eq!(addresses(&manual, false), vec![("10.0.0.2".to_owned(), 8)]);
        let shared = json!({ "ipv4": { "method": "shared" }, "ipv6": { "method": "link-local" } });
        assert_eq!(ip_assignment_text(&shared), "IPv4: Shared to other computers, IPv6: Link-local only");
        // the deprecated dns of an older NetworkManager
        let old = json!({ "ipv4": { "dns": [u32::from_ne_bytes([1, 1, 1, 1])] }, "ipv6": { "dns": [[0x26, 0x06, 0x47, 0, 0x47, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x11, 0x11]] } });
        assert_eq!(dns_list(&old, false), vec!["1.1.1.1"]);
        assert_eq!(dns_list(&old, true), vec!["2606:4700:4700::1111"]);
        let peap = json!({ "802-11-wireless-security": { "key-mgmt": "wpa-eap" }, "802-1x": { "eap": ["peap"] } });
        assert_eq!(security_text(&peap), "WPA/WPA2-Enterprise (Protected EAP (PEAP))");
        kit::i18n::set_forced_lang(Some("nl"));
        assert_eq!(ip_assignment_text(&auto), "Automatisch (DHCP)");
        assert_eq!(band_text("2.4"), "2,4 GHz");
        assert_eq!(band_text("5"), "5 GHz");
    }

    #[test]
    fn security_kinds_round_trip() {
        for (kind, _, _) in SECURITY {
            let groups = security_base(kind);
            let settings = if groups["802-11-wireless-security"].is_null() { json!({}) } else { groups.clone() };
            assert_eq!(security_kind(&settings), *kind, "{groups}");
        }
        assert!(psk_ok("12345678") && !psk_ok("1234567") && psk_ok(&"a".repeat(64)) && !psk_ok(&"z".repeat(64)));
        assert!(wep_key_ok("abcde") && wep_key_ok("0123456789") && !wep_key_ok("abcdef"));
        assert!(eap_keys("tls").contains(&"client-cert"));
        assert!(!eap_keys("peap").contains(&"client-cert"));
        for (m, _, _) in EAP {
            assert!(eap_keys(m).iter().all(|k| EAP_KEYS.contains(k)), "{m}");
        }
    }

    #[test]
    fn a_new_network_gets_a_full_profile() {
        let groups = json!({ "802-11-wireless-security": { "key-mgmt": "wpa-psk", "psk": "wachtwoord1", "psk-flags": 1 }, "802-1x": null });
        let s = new_wifi_settings(&("Kelder".into(), "wpa-psk".into(), groups, false, true));
        assert_eq!(s["802-11-wireless"]["hidden"], true);
        assert_eq!(s["connection"]["autoconnect"], false);
        assert_eq!(s["802-11-wireless-security"]["psk-flags"], 1);
        assert!(s.get("802-1x").is_none(), "a new profile has no group to remove");
    }

    #[test]
    fn identity_values() {
        assert_eq!(mac_text(&json!([0xaa, 0xbb, 0xcc, 0, 1, 0x0f])), "AA:BB:CC:00:01:0F");
        assert_eq!(mac_text(&json!("aa:bb:cc:dd:ee:ff")), "AA:BB:CC:DD:EE:FF");
        let mut blob: Vec<Value> = b"file:///etc/ssl/ca.pem".iter().map(|b| json!(b)).collect();
        blob.push(json!(0));
        assert_eq!(cert_path(&Value::Array(blob)), "/etc/ssl/ca.pem");
        assert_eq!(ssid_text(&json!([84, 104, 117, 105, 115])), "Thuis");
        assert!(metered_on(&json!({ "connection": { "metered": 1 } })));
        assert!(!metered_on(&json!({ "connection": { "metered": 2 } })));
        assert!(shared(&json!({ "connection": {} })));
        assert!(!shared(&json!({ "connection": { "permissions": ["user:robin:"] } })));
        assert!(private_permissions()[0].as_str().unwrap().starts_with("user:"));
    }

    #[test]
    fn the_property_list_leaves_out_what_is_unknown() {
        kit::i18n::set_forced_lang(Some("nl"));
        let dev = json!({
            "type": "wifi", "iface": "wlan0", "vendor": "Intel Corporation", "model": "", "driverVersion": "6.17",
            "hwAddress": "AA:BB:CC:00:11:22", "bitrate": 866000,
            "ap": { "ssid": "Thuis", "band": "2.4", "channel": 6 },
            "ip4": { "addresses": [{ "address": "192.168.1.20", "prefix": 24 }], "dns": ["192.168.1.1"], "gateway": "192.168.1.1" },
            "ip6": { "addresses": [{ "address": "fe80::1", "prefix": 64 }], "dns": [], "gateway": "" },
        });
        let rows = properties(&dev, None);
        let get = |k: &str| rows.iter().find(|(l, _)| l == k).map(|(_, v)| v.clone());
        assert_eq!(get("Netwerkband:").as_deref(), Some("2,4 GHz"));
        assert_eq!(get("Verbindingssnelheid (ontvangen/verzenden):").as_deref(), Some("866/866 (Mbps)"));
        assert_eq!(get("Link-local IPv6-adres:").as_deref(), Some("fe80::1%wlan0"));
        assert_eq!(get("Standaardgateway:").as_deref(), Some("192.168.1.1"));
        assert_eq!(get("Beschrijving:"), None, "an empty value is left out");
        assert_eq!(get("IPv6-adres:"), None);
    }
}
