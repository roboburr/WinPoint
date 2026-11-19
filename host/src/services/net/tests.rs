//! Batch C's `net.*` tests (notes/claude/settings-parity.md §7): the settings map on fixtures (a round trip keeps
//! every key it does not change), the value mappings, the QR text, dry runs that write nothing, and (`#[ignore]`)
//! the calls the methods make to NetworkManager and gsd's Rfkill, against tests/dbus-fakes/fakes/c/networkmanager.py
//! inside tests/dbus-fakes/run.sh --batch c.
use super::connections as nm;
use super::settings_map::{self as map, Groups, Kind, MapError};
use super::*;
use crate::services::system::common::LangContext;
use glib::Variant;
use glib::variant::ToVariant;
use serde_json::json;
use std::collections::BTreeMap;

fn dict(pairs: Vec<(&str, Variant)>) -> BTreeMap<String, Variant> {
    pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect()
}

/// A GetSettings answer as NetworkManager 1.54 sends it, with keys and groups WinPoint has no row for.
fn fixture() -> Variant {
    let mut g: Groups = BTreeMap::new();
    g.insert(
        "connection".into(),
        dict(vec![
            ("id", "Thuis".to_variant()),
            ("uuid", "6a1f0c52-8d3e-4b7a-9c11-2f5e8d4a7b01".to_variant()),
            ("type", "802-11-wireless".to_variant()),
            ("autoconnect", true.to_variant()),
            ("timestamp", 1_763_280_000u64.to_variant()),
            ("zone", "home".to_variant()),
            ("permissions", Vec::<String>::new().to_variant()),
        ]),
    );
    g.insert(
        "802-11-wireless".into(),
        dict(vec![
            ("ssid", b"Thuis".to_vec().to_variant()),
            ("mode", "infrastructure".to_variant()),
            (
                "seen-bssids",
                vec!["AA:BB:CC:00:11:22".to_owned()].to_variant(),
            ),
            ("cloned-mac-address", vec![2u8, 0, 0, 0, 0, 1].to_variant()),
        ]),
    );
    g.insert(
        "ipv4".into(),
        dict(vec![
            ("method", "auto".to_variant()),
            ("addresses", Vec::<Vec<u32>>::new().to_variant()),
            (
                "address-data",
                Vec::<BTreeMap<String, Variant>>::new().to_variant(),
            ),
            ("dns", vec![16_843_009u32].to_variant()),
            ("dns-data", vec!["1.1.1.1".to_owned()].to_variant()),
        ]),
    );
    g.insert("proxy".into(), BTreeMap::new());
    g.insert(
        "x-vendor".into(),
        dict(vec![
            ("pairs", vec![(1i32, 2i32)].to_variant()),
            ("count", 7i32.to_variant()),
        ]),
    );
    g.to_variant()
}

fn ty(g: &Groups, group: &str, key: &str) -> String {
    g[group][key].type_().as_str().to_owned()
}

#[test]
fn a_merge_keeps_every_key_it_does_not_change() {
    let mut g = map::groups_of(&fixture());
    let before = g.clone();
    let changes = json!({
        "connection": { "autoconnect": false, "metered": 1 },
        "802-11-wireless": { "assigned-mac-address": "stable" },
        "ipv4": {
            "method": "manual",
            "address-data": [{ "address": "192.168.1.10", "prefix": 24 }],
            "gateway": "192.168.1.1",
            "dns-data": ["9.9.9.9", "149.112.112.112"],
        },
        "x-vendor": { "count": 8 },
    });
    map::merge(&mut g, &changes).unwrap();
    // the changes, typed as NetworkManager's D-Bus API wants them
    assert_eq!(g["connection"]["autoconnect"], false.to_variant());
    assert_eq!(ty(&g, "connection", "metered"), "i");
    assert_eq!(ty(&g, "ipv4", "address-data"), "aa{sv}");
    assert_eq!(ty(&g, "ipv4", "gateway"), "s");
    assert_eq!(ty(&g, "ipv4", "dns-data"), "as");
    assert_eq!(
        g["802-11-wireless"]["assigned-mac-address"],
        "stable".to_variant()
    );
    // a key no row knows keeps the type NM sent
    assert_eq!(g["x-vendor"]["count"], 8i32.to_variant());
    // untouched keys and groups, also ones WinPoint has no row for, stay as they were
    for (group, key) in [
        ("connection", "timestamp"),
        ("connection", "zone"),
        ("connection", "uuid"),
        ("802-11-wireless", "ssid"),
        ("802-11-wireless", "seen-bssids"),
        ("x-vendor", "pairs"),
    ] {
        assert_eq!(g[group][key], before[group][key], "{group}.{key}");
    }
    assert!(g.contains_key("proxy"));
    // the deprecated twins NM would prefer are gone, so NM takes the change
    assert!(!g["ipv4"].contains_key("addresses"));
    assert!(!g["ipv4"].contains_key("dns"));
    assert!(!g["802-11-wireless"].contains_key("cloned-mac-address"));
    // and the result survives a trip through a{sa{sv}}
    let v = map::to_variant(&g);
    assert_eq!(v.type_().as_str(), "a{sa{sv}}");
    assert_eq!(map::groups_of(&v), g);
}

#[test]
fn the_dbus_arguments_carry_the_settings_dictionary_itself() {
    let g = map::groups_of(&fixture());
    assert_eq!(nm::settings_args(&g).type_().as_str(), "(a{sa{sv}}ua{sv})");
    let path = |p: &str| glib::variant::ObjectPath::try_from(p).unwrap();
    let args = nm::activate_args(
        &g,
        path("/org/freedesktop/NetworkManager/Devices/1"),
        path("/"),
        BTreeMap::new(),
    );
    assert_eq!(args.type_().as_str(), "(a{sa{sv}}ooa{sv})");
}

#[test]
fn null_removes_a_key_or_a_group_but_never_the_identity() {
    let mut g = map::groups_of(&fixture());
    map::merge(
        &mut g,
        &json!({ "connection": { "zone": null }, "proxy": null }),
    )
    .unwrap();
    assert!(!g["connection"].contains_key("zone"));
    assert!(!g.contains_key("proxy"));
    let mut g = map::groups_of(&fixture());
    assert_eq!(
        map::merge(&mut g, &json!({ "connection": { "id": null } })),
        Err(MapError::BadValue {
            group: "connection".into(),
            key: "id".into()
        })
    );
    assert_eq!(
        map::merge(&mut g, &json!({ "connection": null })),
        Err(MapError::BadArgs)
    );
}

#[test]
fn a_bad_value_names_its_group_and_key() {
    for (changes, group, key) in [
        (
            json!({ "connection": { "autoconnect": "yes" } }),
            "connection",
            "autoconnect",
        ),
        (
            json!({ "ipv4": { "address-data": [{ "address": "10.0.0.1" }] } }),
            "ipv4",
            "address-data",
        ),
        (
            json!({ "ipv4": { "address-data": [{ "address": "10.0.0.1", "prefix": 24, "x": 1 }] } }),
            "ipv4",
            "address-data",
        ),
        (
            json!({ "802-11-wireless": { "bssid": "zz:bb:cc:dd:ee:ff" } }),
            "802-11-wireless",
            "bssid",
        ),
        (
            json!({ "802-11-wireless": { "mode": "infra\u{0}structure" } }),
            "802-11-wireless",
            "mode",
        ),
        (
            json!({ "connection": { "metered": 4294967296u64 } }),
            "connection",
            "metered",
        ),
        (
            json!({ "802-1x": { "ca-cert": "relative/ca.pem" } }),
            "802-1x",
            "ca-cert",
        ),
        // a key with no row and no value from NM has no type
        (json!({ "ipv4": { "made-up": 1 } }), "ipv4", "made-up"),
    ] {
        let mut g = map::groups_of(&fixture());
        assert_eq!(
            map::merge(&mut g, &changes),
            Err(MapError::BadValue {
                group: group.into(),
                key: key.into()
            }),
            "{changes}"
        );
    }
    for changes in [
        json!([]),
        json!({ "Bad Group": {} }),
        json!({ "ipv4": { "Bad Key": 1 } }),
        json!({ "ipv4": 3 }),
    ] {
        let mut g = map::groups_of(&fixture());
        assert_eq!(
            map::merge(&mut g, &changes),
            Err(MapError::BadArgs),
            "{changes}"
        );
    }
}

#[test]
fn values_get_networkmanagers_types() {
    let cases = [
        (Kind::Mac, json!("aa:bb:cc:dd:ee:0f"), "ay"),
        (Kind::Mac, json!([1, 2, 3, 4, 5, 6]), "ay"),
        (Kind::Bytes, json!("Café"), "ay"),
        (Kind::Cert, json!("/home/u/ca.pem"), "ay"),
        (
            Kind::RouteData,
            json!([{ "dest": "10.0.0.0", "prefix": 8, "next-hop": "10.0.0.1", "metric": 100 }]),
            "aa{sv}",
        ),
        (
            Kind::StrDict,
            json!({ "remote": "vpn.example.org" }),
            "a{ss}",
        ),
        (Kind::U32s, json!([1, 2]), "au"),
        (Kind::ByteArrays, json!([[1, 2], [3]]), "aay"),
    ];
    for (kind, value, t) in cases {
        let v = map::value_variant(kind, &value).unwrap_or_else(|| panic!("{kind:?} {value}"));
        assert_eq!(v.type_().as_str(), t);
    }
    assert_eq!(map::value_variant(Kind::Mac, &json!("aa:bb:cc")), None);
    assert_eq!(map::value_variant(Kind::Bytes, &json!([256])), None);
    let blob = map::value_variant(Kind::Cert, &json!("/etc/ssl/ca.pem")).unwrap();
    assert_eq!(
        map::cert_path(&blob.get::<Vec<u8>>().unwrap()).as_deref(),
        Some("/etc/ssl/ca.pem")
    );
    assert_eq!(map::cert_path(b"-----BEGIN CERT"), None);
    assert_eq!(
        map::parse_mac("AA-BB-CC-DD-EE-FF"),
        Some(vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff])
    );
    assert_eq!(
        map::format_mac(&[0xaa, 0xb, 0, 1, 2, 0xff]),
        "AA:0B:00:01:02:FF"
    );
    assert_eq!(
        nm::ipv4_text(u32::from_ne_bytes([192, 168, 1, 1])),
        "192.168.1.1"
    );
}

#[test]
fn secrets_are_named_and_kept() {
    assert!(map::has_secrets(
        &json!({ "802-11-wireless-security": { "psk": "geheim123" } })
    ));
    assert!(map::has_secrets(&json!({ "802-1x": { "password": "x" } })));
    assert!(!map::has_secrets(
        &json!({ "802-11-wireless-security": { "psk": null } })
    ));
    assert!(!map::has_secrets(
        &json!({ "connection": { "autoconnect": true } })
    ));
    let mut base = map::groups_of(&fixture());
    let secrets: Groups = BTreeMap::from([(
        "802-11-wireless-security".into(),
        dict(vec![("psk", "oud".to_variant())]),
    )]);
    map::add_secrets(&mut base, &secrets);
    map::merge(
        &mut base,
        &json!({ "802-11-wireless-security": { "psk": "nieuw-wachtwoord" } }),
    )
    .unwrap();
    assert_eq!(
        base["802-11-wireless-security"]["psk"],
        "nieuw-wachtwoord".to_variant()
    );
    let mut again = base.clone();
    map::add_secrets(&mut again, &secrets);
    assert_eq!(
        again["802-11-wireless-security"]["psk"],
        "nieuw-wachtwoord".to_variant(),
        "never over a new value"
    );
}

#[test]
fn the_wifi_qr_text_is_gnome_settings_own() {
    assert_eq!(
        map::wifi_qr("Thuis", "WPA", Some("geheim"), false),
        "WIFI:S:Thuis;T:WPA;P:geheim;;"
    );
    assert_eq!(
        map::wifi_qr(r#"a;b,c:"d\"#, "SAE", Some("p;w"), false),
        r#"WIFI:S:a\;b\,c\:\"d\\;T:SAE;P:p\;w;;"#
    );
    assert_eq!(
        map::wifi_qr("Café", "nopass", None, false),
        "WIFI:S:Café;T:nopass;;"
    );
    assert_eq!(
        map::wifi_qr("Verborgen", "WPA", Some(""), true),
        "WIFI:S:Verborgen;T:nopass;H:true;;"
    );
    assert_eq!(map::qr_security("wpa-psk"), "WPA");
    assert_eq!(map::qr_security("sae"), "SAE");
    assert_eq!(map::qr_security("none"), "WEP");
    assert_eq!(map::qr_security(""), "nopass");
}

#[test]
fn a_hidden_network_gets_a_full_profile() {
    let args = json!({
        "ssid": "Kelder", "security": "wpa-psk", "autoconnect": false,
        "secrets": { "802-11-wireless-security": { "psk": "wachtwoord1", "psk-flags": 1 } },
    });
    let s = wifi::hidden_settings(&args, "0b9e4d21-73aa-4c6f-8e2d-5a1c9f3e6d02").unwrap();
    assert_eq!(s["connection"]["type"], "802-11-wireless");
    assert_eq!(s["connection"]["autoconnect"], false);
    assert_eq!(s["802-11-wireless"]["hidden"], true);
    assert_eq!(s["802-11-wireless-security"]["key-mgmt"], "wpa-psk");
    assert_eq!(s["802-11-wireless-security"]["psk-flags"], 1);
    assert!(s["802-1x"].is_null());
    let g = map::build(&s).unwrap();
    assert_eq!(ty(&g, "802-11-wireless", "ssid"), "ay");
    assert_eq!(ty(&g, "802-11-wireless-security", "psk-flags"), "u");
    let uuid = "0b9e4d21-73aa-4c6f-8e2d-5a1c9f3e6d02";
    assert!(wifi::hidden_settings(&json!({ "ssid": "x".repeat(33) }), uuid).is_none());
    assert!(wifi::hidden_settings(&json!({ "ssid": "a", "security": "wep-128" }), uuid).is_none());
    assert!(
        wifi::hidden_settings(
            &json!({ "ssid": "a", "secrets": { "ipv4": { "method": "disabled" } } }),
            uuid
        )
        .is_none()
    );
    let eap = json!({ "ssid": "Werk", "security": "wpa-eap",
        "secrets": { "802-1x": { "eap": ["peap"], "identity": "jan", "phase2-auth": "mschapv2", "password": "x" } } });
    let g = map::build(&wifi::hidden_settings(&eap, uuid).unwrap()).unwrap();
    assert_eq!(ty(&g, "802-1x", "eap"), "as");
    assert_eq!(
        g["802-11-wireless-security"]["key-mgmt"],
        "wpa-eap".to_variant()
    );
    // a network in the list that needs the 802.1X or WEP fields (the Wi-Fi page's dialog): not hidden
    let mut listed = eap.clone();
    listed["hidden"] = json!(false);
    let s = wifi::hidden_settings(&listed, uuid).unwrap();
    assert_eq!(s["802-11-wireless"]["hidden"], false);
    assert_eq!(s["802-1x"]["identity"], "jan");
    listed["hidden"] = json!("no");
    assert!(
        wifi::hidden_settings(&listed, uuid).is_none(),
        "hidden is a boolean"
    );
}

#[test]
fn every_security_kind_merges_into_a_profile() {
    for kind in [
        "open",
        "owe",
        "wep",
        "wep-passphrase",
        "leap",
        "dynamic-wep",
        "wpa-psk",
        "sae",
        "wpa-eap",
    ] {
        let mut g = map::groups_of(&fixture());
        map::merge(&mut g, &map::security_base(kind).unwrap())
            .unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        if kind == "open" {
            assert!(!g.contains_key("802-11-wireless-security"));
        } else {
            assert!(
                g["802-11-wireless-security"].contains_key("key-mgmt"),
                "{kind}"
            );
        }
    }
    assert!(map::security_base("wpa3-eap").is_none());
}

#[test]
fn bands_channels_states_and_security_of_access_points() {
    assert_eq!((nm::band(2412), nm::channel(2412)), ("2.4", 1));
    assert_eq!(nm::channel(2484), 14);
    assert_eq!((nm::band(5180), nm::channel(5180)), ("5", 36));
    assert_eq!((nm::band(5955), nm::channel(5955)), ("6", 1));
    assert_eq!(nm::band(60_480), "");
    assert_eq!(nm::device_state(20, Some(false)), "unplugged");
    assert_eq!(nm::device_state(20, None), "unavailable");
    assert_eq!(nm::device_state(50, None), "connecting");
    assert_eq!(nm::device_state(100, Some(true)), "connected");
    assert_eq!(nm::ap_security(1, 0, 0x188), "wpa-psk");
    assert_eq!(nm::ap_security(1, 0, 0x400), "sae");
    assert_eq!(
        nm::ap_security(1, 0, 0x500),
        "wpa-psk",
        "WPA2/WPA3 transition"
    );
    assert_eq!(nm::ap_security(1, 0, 0x200), "wpa-eap");
    assert_eq!(nm::ap_security(0, 0, 0x800), "owe");
    assert_eq!(nm::ap_security(1, 0, 0), "wep");
    assert_eq!(nm::ap_security(0, 0, 0), "open");
    assert_eq!(nm::device_type(2), "wifi");
    assert_eq!(nm::type_name("wifi"), "802-11-wireless");
}

#[test]
fn udev_gives_the_vendor_and_model() {
    let root = std::env::temp_dir().join(format!("win11-net-udev-{}", crate::events::nonce()));
    let (sys, udev) = (root.join("sys"), root.join("udev"));
    std::fs::create_dir_all(sys.join("wlp2s0")).unwrap();
    std::fs::create_dir_all(&udev).unwrap();
    std::fs::write(sys.join("wlp2s0/ifindex"), "3\n").unwrap();
    std::fs::write(
        udev.join("n3"),
        "I:123\nE:ID_VENDOR_FROM_DATABASE=Intel Corporation\nE:ID_MODEL_FROM_DATABASE=Wi-Fi 6 AX200\n",
    )
    .unwrap();
    assert_eq!(
        nm::udev_names("wlp2s0", &sys, &udev),
        ("Intel Corporation".into(), "Wi-Fi 6 AX200".into())
    );
    assert_eq!(
        nm::udev_names("../wlp2s0", &sys, &udev),
        (String::new(), String::new())
    );
    assert_eq!(
        nm::udev_names("eth9", &sys, &udev),
        (String::new(), String::new())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_summary_says_what_the_lists_show() {
    let j = map::to_json(&map::groups_of(&fixture()));
    let active = (
        "/org/freedesktop/NetworkManager/ActiveConnection/1".to_owned(),
        json!({ "State": 2, "Devices": ["/d/1"] }),
    );
    let ifaces = BTreeMap::from([("/d/1".to_owned(), "wlan0".to_owned())]);
    let s = nm::summary(
        "/org/freedesktop/NetworkManager/Settings/1",
        &j,
        Some(&active),
        &ifaces,
    );
    assert_eq!(s["ssid"], "Thuis");
    assert_eq!(s["shared"], true);
    assert_eq!(s["active"], true);
    assert_eq!(s["state"], "activated");
    assert_eq!(s["device"], "wlan0");
    assert_eq!(s["timestamp"], 1_763_280_000u64);
    assert_eq!(
        j["802-11-wireless"]["ssid"],
        json!([84, 104, 117, 105, 115]),
        "the SSID as bytes"
    );
}

#[test]
fn signals_and_arguments() {
    assert_eq!(
        signal_kind("/org/freedesktop/NetworkManager/Settings/4"),
        "connections"
    );
    assert_eq!(
        signal_kind("/org/freedesktop/NetworkManager/Devices/2"),
        "devices"
    );
    assert_eq!(
        signal_kind("/org/freedesktop/NetworkManager/AccessPoint/9"),
        "accessPoints"
    );
    assert_eq!(
        signal_kind("/org/freedesktop/NetworkManager/ActiveConnection/1"),
        "active"
    );
    assert_eq!(signal_kind("/org/freedesktop/NetworkManager"), "state");
    assert!(uuid_ok("6a1f0c52-8d3e-4b7a-9c11-2f5e8d4a7b01"));
    assert!(!uuid_ok("6a1f0c52-8d3e-4b7a-9c11-2f5e8d4a7b0"));
    assert!(!uuid_ok("6a1f0c52x8d3e-4b7a-9c11-2f5e8d4a7b01"));
    assert!(iface_ok("wlp2s0") && iface_ok("enx00e04c680001"));
    assert!(!iface_ok("") && !iface_ok("a/b") && !iface_ok("..") && !iface_ok("averyveryverylong"));
}

/// A ServiceContext of its own (a temporary config directory) with `dry_run` as asked.
struct Ctx {
    root: std::path::PathBuf,
    ctx: Arc<ServiceContext>,
}

impl Ctx {
    fn new(dry_run: bool) -> Ctx {
        let root = std::env::temp_dir().join(format!("win11-net-{}", crate::events::nonce()));
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("settings.json"),
            json!({ "language": "en" }).to_string(),
        )
        .unwrap();
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = ServiceContext::new(
            events,
            config,
            root.join("cache"),
            root.clone(),
            "settings".into(),
            dry_run,
        )
        .unwrap();
        Ctx { root, ctx }
    }
}

impl Drop for Ctx {
    fn drop(&mut self) {
        self.ctx.events.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const UUID: &str = "6a1f0c52-8d3e-4b7a-9c11-2f5e8d4a7b01";

/// Every dangerous call with good arguments, as the pages send them.
fn writes() -> Vec<(&'static str, Value)> {
    vec![
        (
            "net.connection.update",
            json!({ "uuid": UUID, "settings": { "connection": { "autoconnect": false } } }),
        ),
        (
            "net.connection.add",
            json!({ "settings": { "connection": { "id": "Kelder", "type": "802-11-wireless" }, "802-11-wireless": { "ssid": "Kelder", "hidden": true } } }),
        ),
        ("net.connection.delete", json!({ "uuid": UUID })),
        (
            "net.connection.activate",
            json!({ "uuid": UUID, "device": "wlan0" }),
        ),
        ("net.connection.deactivate", json!({ "uuid": UUID })),
        (
            "net.device.setEnabled",
            json!({ "iface": "eth0", "on": false }),
        ),
        (
            "net.wifi.hidden",
            json!({ "ssid": "Kelder", "security": "wpa-psk", "secrets": { "802-11-wireless-security": { "psk": "wachtwoord1" } } }),
        ),
        ("net.radios.set", json!({ "wifi": false })),
        ("net.connectivity.set", json!({ "on": false })),
        (
            "net.hotspot.start",
            json!({ "ssid": "WinPoint", "password": "abcd1234", "band": "bg" }),
        ),
        ("net.hotspot.start", json!({ "activate": false })),
        ("net.hotspot.stop", json!({})),
    ]
}

const MODEM: &str = "/org/freedesktop/ModemManager1/Modem/0";

/// Every ModemManager write with good arguments.
fn modem_writes() -> Vec<(&'static str, Value)> {
    vec![
        ("net.modem.unlock", json!({ "modem": MODEM, "pin": "1234" })),
        (
            "net.modem.unlock",
            json!({ "modem": MODEM, "puk": "12345678", "newPin": "4321" }),
        ),
        (
            "net.modem.setPin",
            json!({ "modem": MODEM, "enabled": false, "pin": "1234" }),
        ),
        (
            "net.modem.changePin",
            json!({ "modem": MODEM, "old": "1234", "new": "98765" }),
        ),
        (
            "net.modem.setModes",
            json!({ "modem": MODEM, "allowed": 14, "preferred": 8 }),
        ),
        (
            "net.modem.register",
            json!({ "modem": MODEM, "operator": "20408" }),
        ),
        ("net.modem.register", json!({ "modem": MODEM })),
        (
            "net.modem.setPrimarySlot",
            json!({ "modem": MODEM, "slot": 2 }),
        ),
    ]
}

#[test]
fn a_dry_run_checks_the_arguments_and_writes_nothing() {
    // the unit-test sandbox has no system bus: a real call fails, a dry run never asks the bus
    let dry = Ctx::new(true);
    let net = Net::new(dry.ctx.clone()).unwrap();
    let live = Ctx::new(false);
    let real = Net::new(live.ctx.clone()).unwrap();
    for (method, args) in writes() {
        assert!(METHODS.contains(&(method, true)), "{method} is dangerous");
        let r = net.call(method, &args).unwrap();
        assert!(r.is_ok(), "{method}: {:?}", r.err());
        let e = real.call(method, &args).unwrap().unwrap_err();
        assert_eq!(e.code, "unsupported", "{method} needs NetworkManager");
        assert_eq!(e.key.as_deref(), Some("net.unavailable"));
    }
    // the checks still run
    for (method, args) in [
        (
            "net.connection.update",
            json!({ "uuid": "x", "settings": {} }),
        ),
        (
            "net.connection.update",
            json!({ "uuid": UUID, "settings": { "connection": { "autoconnect": 1 } } }),
        ),
        (
            "net.connection.add",
            json!({ "settings": { "connection": { "id": "a" } } }),
        ),
        (
            "net.device.setEnabled",
            json!({ "iface": "../x", "on": true }),
        ),
        ("net.wifi.hidden", json!({ "ssid": "" })),
        ("net.radios.set", json!({})),
        ("net.connectivity.set", json!({ "on": "yes" })),
    ] {
        let e = net.call(method, &args).unwrap().unwrap_err();
        assert_eq!(e.code, "bad_args", "{method} {args}");
    }
    let e = net
        .call(
            "net.connection.update",
            &json!({ "uuid": UUID, "settings": { "ipv4": { "never-default": "no" } } }),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("net.badValue"));
    assert_eq!(
        e.data,
        Some(json!({ "group": "ipv4", "key": "never-default" }))
    );
    assert!(net.call("net.unknown", &json!({})).is_none());
    // ModemManager's writes: checked, nothing sent in a dry run; for real they need ModemManager
    for (method, args) in modem_writes() {
        assert!(METHODS.contains(&(method, true)), "{method} is dangerous");
        let r = net.call(method, &args).unwrap();
        assert!(r.is_ok(), "{method}: {:?}", r.err());
        let e = real.call(method, &args).unwrap().unwrap_err();
        assert_eq!(e.code, "not_found", "{method} needs ModemManager");
        assert_eq!(e.key.as_deref(), Some("net.noModem"));
    }
    for (method, args, key) in [
        (
            "net.modem.unlock",
            json!({ "modem": MODEM, "pin": "12" }),
            "net.badPin",
        ),
        (
            "net.modem.unlock",
            json!({ "modem": MODEM, "puk": "1234", "newPin": "1234" }),
            "net.badPin",
        ),
        (
            "net.modem.unlock",
            json!({ "modem": "/org/freedesktop/ModemManager1/SIM/0", "pin": "1234" }),
            "net.badArgs",
        ),
        (
            "net.modem.setModes",
            json!({ "modem": MODEM, "allowed": 14, "preferred": 12 }),
            "net.badArgs",
        ),
        (
            "net.modem.register",
            json!({ "modem": MODEM, "operator": "kpn" }),
            "net.badArgs",
        ),
        (
            "net.modem.setPrimarySlot",
            json!({ "modem": MODEM, "slot": 0 }),
            "net.badArgs",
        ),
        (
            "net.hotspot.start",
            json!({ "ssid": "x".repeat(33) }),
            "net.badSsid",
        ),
        (
            "net.hotspot.start",
            json!({ "password": "kort" }),
            "net.badPassword",
        ),
        ("net.hotspot.start", json!({ "band": "6" }), "net.badArgs"),
        ("net.hotspot.start", json!({ "activate": 1 }), "net.badArgs"),
        (
            "net.vpn.import",
            json!({ "path": "relative.ovpn" }),
            "net.badArgs",
        ),
        (
            "net.vpn.import",
            json!({ "path": "/nonexistent/winpoint.ovpn" }),
            "net.badImport",
        ),
        (
            "net.modem.apns",
            json!({ "mcc": "20", "mnc": "08" }),
            "net.badArgs",
        ),
    ] {
        let e = net.call(method, &args).unwrap().unwrap_err();
        assert_eq!(e.code, "bad_args", "{method} {args}");
        assert_eq!(e.key.as_deref(), Some(key), "{method} {args}");
    }
    // the import's dry run reads and checks the file, and starts no nmcli
    let dir = std::env::temp_dir().join(format!("win11-vpn-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let wg = dir.join("wg0.conf");
    std::fs::write(&wg, "[Interface]\nPrivateKey = x\n[Peer]\nPublicKey = y\n").unwrap();
    let r = net
        .call("net.vpn.import", &json!({ "path": wg.to_str().unwrap() }))
        .unwrap()
        .unwrap();
    assert_eq!(r["type"], "wireguard");
    let bad = dir.join("my tunnel.conf");
    std::fs::write(&bad, "[Interface]\nPrivateKey = x\n").unwrap();
    let e = net
        .call("net.vpn.import", &json!({ "path": bad.to_str().unwrap() }))
        .unwrap()
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("net.badImportName"));
    let _ = std::fs::remove_dir_all(&dir);
    net.shutdown();
    real.shutdown();
}

#[test]
fn the_net_texts_are_in_both_languages() {
    use crate::i18n::Lang;
    for key in [
        "net.unavailable",
        "net.badArgs",
        "net.badValue",
        "net.notFound",
        "net.noDevice",
        "net.noWifi",
        "net.notActive",
        "net.notWifi",
        "net.badPassword",
        "net.badSsid",
        "net.badImport",
        "net.badImportName",
        "net.noPlugin",
        "net.noModem",
        "net.noSim",
        "net.badPin",
    ] {
        let (nl, en) = (
            crate::i18n::trf(Lang::Nl, key, NO_PARAMS),
            crate::i18n::trf(Lang::En, key, NO_PARAMS),
        );
        assert!(nl != key && en != key && nl != en, "{key}: {nl} / {en}");
    }
}

// ------------------------------------------------------------------------------------- bus (dbus-fakes)

/// The calls the fake got, in order, as JSON lines (tests/dbus-fakes/run.sh names the file).
fn fake_calls() -> Option<Vec<Value>> {
    let file = std::env::var("WIN11_FAKE_CALLS_NETWORKMANAGER").ok()?;
    Some(
        std::fs::read_to_string(file)
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect(),
    )
}

fn calls_since(n: usize) -> Vec<Value> {
    fake_calls()
        .unwrap_or_default()
        .into_iter()
        .skip(n)
        .collect()
}

fn find<'a>(calls: &'a [Value], method: &str) -> &'a Value {
    calls
        .iter()
        .find(|c| c["method"] == method)
        .unwrap_or_else(|| panic!("no {method} call in {calls:#?}"))
}

/// The fake on the private system bus, or None (then the test says so and passes: it only runs inside run.sh).
fn bus_net() -> Option<(LangContext, Arc<Net>)> {
    if fake_calls().is_none() {
        eprintln!("skipped: run inside tests/dbus-fakes/run.sh --batch c");
        return None;
    }
    let lc = LangContext::new("en", "settings");
    let net = Net::new(lc.ctx.clone()).unwrap();
    Some((lc, net))
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_update_merges_into_networkmanagers_settings_and_allows_the_password_dialog() {
    let Some((_lc, net)) = bus_net() else { return };
    let n = fake_calls().unwrap().len();
    let r = net
        .call(
            "net.connection.update",
            &json!({ "uuid": UUID, "settings": {
                "connection": { "autoconnect": false, "metered": 1 },
                "ipv4": { "method": "manual", "address-data": [{ "address": "192.168.1.30", "prefix": 24 }], "gateway": "192.168.1.1" },
            } }),
        )
        .unwrap()
        .unwrap();
    assert_eq!(r["uuid"], UUID);
    let calls = calls_since(n);
    let update = find(&calls, "Update2");
    assert_eq!(update["interactive"], true);
    let body = update["args"].as_str().unwrap();
    for kept in [
        "'zone': <'home'>",
        "'seen-bssids': <['AA:BB:CC:00:11:22']>",
        "'x-vendor'",
        "'proxy': {}",
    ] {
        assert!(body.contains(kept), "{kept} kept: {body}");
    }
    for new in [
        "'autoconnect': <false>",
        "'metered': <1>",
        "'method': <'manual'>",
        "'address': <'192.168.1.30'>",
    ] {
        assert!(body.contains(new), "{new}: {body}");
    }
    assert!(
        !body.contains("'addresses'"),
        "the legacy twin is dropped: {body}"
    );
    assert_eq!(
        update["signature"], "(a{sa{sv}}ua{sv})",
        "the settings dictionary itself"
    );
    assert!(
        body.ends_with("uint32 1, @a{sv} {})"),
        "saved to disk: {body}"
    );
    assert!(
        calls.iter().all(|c| c["method"] != "GetSecrets"),
        "no secret changed, none asked"
    );
    let reapply = find(&calls, "Reapply");
    assert_eq!(
        (reapply["path"].as_str(), reapply["interactive"].as_bool()),
        (
            Some("/org/freedesktop/NetworkManager/Devices/1"),
            Some(true)
        )
    );
    for read in ["GetConnectionByUuid", "GetSettings"] {
        assert_eq!(
            find(&calls, read)["interactive"],
            false,
            "{read} is a plain read"
        );
    }
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_a_new_key_carries_the_stored_secrets_along() {
    let Some((_lc, net)) = bus_net() else { return };
    let n = fake_calls().unwrap().len();
    let settings =
        json!({ "802-11-wireless-security": { "psk": "nieuwwachtwoord1", "psk-flags": 1 } });
    net.call(
        "net.connection.update",
        &json!({ "uuid": UUID, "settings": settings, "reapply": false }),
    )
    .unwrap()
    .unwrap();
    let calls = calls_since(n);
    let secrets = find(&calls, "GetSecrets");
    assert_eq!(secrets["interactive"], true);
    assert_eq!(secrets["args"], "('802-11-wireless-security',)");
    let at = |m: &str| calls.iter().position(|c| c["method"] == m).unwrap();
    assert!(at("GetSecrets") < at("Update2"));
    let body = find(&calls, "Update2")["args"].as_str().unwrap().to_owned();
    assert!(body.contains("'psk': <'nieuwwachtwoord1'>"), "{body}");
    assert!(body.contains("'psk-flags': <uint32 1>"), "{body}");
    assert!(calls.iter().all(|c| c["method"] != "Reapply"));
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_the_security_key_and_its_qr_code() {
    let Some((_lc, net)) = bus_net() else { return };
    let n = fake_calls().unwrap().len();
    let r = net
        .call("net.wifi.secrets", &json!({ "uuid": UUID }))
        .unwrap()
        .unwrap();
    assert_eq!(r["ssid"], "Lab-5G");
    assert_eq!(r["keyMgmt"], "wpa-psk");
    assert_eq!(r["key"], "geheimwachtwoord");
    assert_eq!(r["qr"], "WIFI:S:Lab-5G;T:WPA;P:geheimwachtwoord;;");
    assert_eq!(find(&calls_since(n), "GetSecrets")["interactive"], true);
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_lists_devices_and_connections() {
    let Some((_lc, net)) = bus_net() else { return };
    let d = net
        .call("net.devices", &json!({ "accessPoints": true }))
        .unwrap()
        .unwrap();
    let devices = d["devices"].as_array().unwrap();
    let wlan = devices.iter().find(|x| x["iface"] == "wlan0").unwrap();
    assert_eq!(
        (wlan["type"].as_str(), wlan["state"].as_str()),
        (Some("wifi"), Some("connected"))
    );
    assert_eq!(wlan["ap"]["ssid"], "Lab-5G");
    assert_eq!(
        (wlan["ap"]["band"].as_str(), wlan["ap"]["channel"].as_u64()),
        (Some("5"), Some(36))
    );
    assert_eq!(wlan["ap"]["security"], "wpa-psk");
    assert_eq!(wlan["active"]["uuid"], UUID);
    assert_eq!(wlan["ip4"]["addresses"][0]["address"], "192.168.1.20");
    assert_eq!(wlan["ip4"]["dns"], json!(["192.168.1.1"]));
    assert_eq!(wlan["bitrate"], 866_000);
    // Lab-5G and eduroam (WPA2-Enterprise: the Wi-Fi page asks for its 802.1X fields)
    let aps = wlan["accessPoints"].as_array().unwrap();
    assert_eq!(aps.len(), 2);
    assert_eq!(
        aps.iter()
            .find(|a| a["ssid"] == "eduroam")
            .map(|a| a["security"].clone()),
        Some(json!("wpa-eap"))
    );
    let eth = devices.iter().find(|x| x["iface"] == "eth0").unwrap();
    assert_eq!(eth["state"], "unplugged");
    assert_eq!(
        eth["available"],
        json!(["0b9e4d21-73aa-4c6f-8e2d-5a1c9f3e6d02"])
    );
    assert_eq!(d["wifiEnabled"], true);
    assert_eq!(d["connectivity"], "full");
    let c = net
        .call("net.connections", &json!({ "type": "wifi" }))
        .unwrap()
        .unwrap();
    let list = c["connections"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (
            list[0]["ssid"].as_str(),
            list[0]["active"].as_bool(),
            list[0]["device"].as_str()
        ),
        (Some("Lab-5G"), Some(true), Some("wlan0"))
    );
    let g = net
        .call("net.connection.get", &json!({ "uuid": UUID }))
        .unwrap()
        .unwrap();
    assert_eq!(g["settings"]["connection"]["zone"], "home");
    assert_eq!(
        g["settings"]["802-11-wireless"]["ssid"],
        json!([76, 97, 98, 45, 53, 71])
    );
    let e = net
        .call(
            "net.connection.get",
            &json!({ "uuid": "00000000-0000-4000-8000-000000000000" }),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(e.code, "not_found");
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_profiles_devices_and_switches_ask_with_the_password_dialog() {
    let Some((_lc, net)) = bus_net() else { return };
    let eth = "0b9e4d21-73aa-4c6f-8e2d-5a1c9f3e6d02";
    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "net.connection.add",
            json!({ "settings": { "connection": { "id": "Profiel 2", "type": "802-3-ethernet", "interface-name": "eth0" }, "802-3-ethernet": {} } }),
            "AddConnection2",
        ),
        (
            "net.connection.activate",
            json!({ "uuid": eth, "device": "eth0" }),
            "ActivateConnection",
        ),
        (
            "net.connection.deactivate",
            json!({ "uuid": UUID }),
            "DeactivateConnection",
        ),
        (
            "net.device.setEnabled",
            json!({ "iface": "eth0", "on": false }),
            "Disconnect",
        ),
        (
            "net.device.setEnabled",
            json!({ "iface": "eth0", "on": true }),
            "ActivateConnection",
        ),
        ("net.radios.set", json!({ "wifi": false }), "Set"),
        ("net.connectivity.set", json!({ "on": false }), "Set"),
        (
            "net.wifi.hidden",
            json!({ "ssid": "Kelder", "security": "sae", "secrets": { "802-11-wireless-security": { "psk": "wachtwoord1" } } }),
            "AddAndActivateConnection2",
        ),
        ("net.connection.delete", json!({ "uuid": eth }), "Delete"),
    ];
    for (method, args, dbus) in cases {
        let n = fake_calls().unwrap().len();
        let r = net.call(method, &args).unwrap();
        assert!(r.is_ok(), "{method}: {:?}", r.err());
        let calls = calls_since(n);
        let call = calls
            .iter()
            .rev()
            .find(|c| c["method"] == dbus)
            .unwrap_or_else(|| panic!("{method}: no {dbus} in {calls:#?}"));
        assert_eq!(call["interactive"], true, "{method} → {dbus}");
        let body = call["args"].as_str().unwrap_or("");
        match (method, dbus) {
            ("net.connection.add", _) => assert!(
                body.contains("'interface-name': <'eth0'>") && body.contains("uint32 1"),
                "{body}"
            ),
            ("net.device.setEnabled", "ActivateConnection") => assert!(
                body.starts_with(
                    "(objectpath '/', objectpath '/org/freedesktop/NetworkManager/Devices/2'"
                ),
                "{body}"
            ),
            ("net.radios.set", _) => assert_eq!(
                body,
                "('org.freedesktop.NetworkManager', 'WirelessEnabled', <false>)"
            ),
            ("net.connectivity.set", _) => assert_eq!(
                body,
                "('org.freedesktop.NetworkManager', 'ConnectivityCheckEnabled', <false>)"
            ),
            ("net.wifi.hidden", _) => {
                assert!(
                    body.contains("'hidden': <true>")
                        && body.contains("'key-mgmt': <'sae'>")
                        && body.contains("'persist': <'disk'>"),
                    "{body}"
                );
                assert!(
                    body.contains("'ssid': <[byte 0x4b, 0x65, 0x6c, 0x64, 0x65, 0x72]>")
                        || body.contains("'ssid': <b'Kelder'>"),
                    "{body}"
                );
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------------- slice 2: hotspot, VPN, modems

use super::{hotspot, modem, providers, vpn};

#[test]
fn a_new_hotspot_is_gnome_settings_own() {
    let caps = hotspot::CAP_AP | hotspot::CAP_RSN | hotspot::CAP_WPA;
    let p = hotspot::new_profile(UUID, "WinPoint", "abcd1234", "", caps, "");
    assert_eq!(p["connection"]["id"], "Hotspot");
    assert_eq!(p["connection"]["autoconnect"], false);
    assert_eq!(p["802-11-wireless"]["mode"], "ap");
    assert!(
        p["802-11-wireless"]["band"].is_null(),
        "any band: NetworkManager chooses"
    );
    assert_eq!(p["ipv4"]["method"], "shared");
    assert_eq!(p["ipv6"]["method"], "ignore");
    let sec = &p["802-11-wireless-security"];
    assert_eq!(
        (
            sec["key-mgmt"].as_str(),
            sec["proto"].clone(),
            sec["pairwise"].clone(),
            sec["psk"].as_str()
        ),
        (
            Some("wpa-psk"),
            json!(["rsn"]),
            json!(["ccmp"]),
            Some("abcd1234")
        )
    );
    assert!(hotspot::is_hotspot(&p));
    let g = map::build(&p).unwrap();
    assert_eq!(ty(&g, "802-11-wireless", "ssid"), "ay");
    assert_eq!(ty(&g, "802-11-wireless-security", "proto"), "as");
    // WPA only: TKIP; no WPA: a WEP key; no AP mode: ad hoc
    let wpa = hotspot::new_profile(
        UUID,
        "x",
        "abcd1234",
        "a",
        hotspot::CAP_AP | hotspot::CAP_WPA,
        "wlan0",
    );
    assert_eq!(wpa["802-11-wireless-security"]["pairwise"], json!(["tkip"]));
    assert_eq!(wpa["802-11-wireless"]["band"], "a");
    assert_eq!(wpa["connection"]["interface-name"], "wlan0");
    let wep = hotspot::new_profile(UUID, "x", "0123456789", "", 0, "");
    assert_eq!(wep["802-11-wireless"]["mode"], "adhoc");
    assert_eq!(
        (
            wep["802-11-wireless-security"]["key-mgmt"].as_str(),
            wep["802-11-wireless-security"]["wep-key0"].as_str()
        ),
        (Some("none"), Some("0123456789"))
    );
    assert_eq!(hotspot::key_mgmt_for(0), "none");
    assert_eq!(hotspot::security_name(&wep), "wep");
    assert_eq!(hotspot::security_name(&wpa), "wpa");
    assert_eq!(hotspot::security_name(&p), "wpa2");
    // an ordinary Wi-Fi profile is no hotspot
    let wifi = map::to_json(&map::groups_of(&fixture()));
    assert!(!hotspot::is_hotspot(&wifi));
}

#[test]
fn hotspot_keys_names_and_changes() {
    for (km, pw, ok) in [
        ("wpa-psk", "abcd1234", true),
        ("wpa-psk", "kort", false),
        ("wpa-psk", &"a".repeat(63), true),
        ("wpa-psk", &"g".repeat(64), false),
        ("wpa-psk", &"ab".repeat(32), true),
        ("wpa-psk", "tab\tin", false),
        ("sae", "abcd1234", true),
        ("none", "abcde", true),
        ("none", "0123456789", true),
        ("none", "012345678g", false),
        ("none", "abcd1234", false),
    ] {
        assert_eq!(hotspot::password_ok(km, pw), ok, "{km} {pw}");
    }
    let key = hotspot::random_key(false, &(0u8..=255).collect::<Vec<_>>());
    assert_eq!(key.len(), 8);
    assert!(key.bytes().all(|b| b.is_ascii_alphanumeric()));
    let wep = hotspot::random_key(true, &(0u8..=255).rev().collect::<Vec<_>>());
    assert_eq!(wep.len(), 10);
    assert!(hotspot::password_ok("none", &wep));
    // bytes past the last whole round of the alphabet are skipped (equally likely letters)
    assert_eq!(hotspot::random_key(false, &[255, 254, 0]), "A");
    // a new hotspot's key is proposed once: every net.hotspot.get shows the same one (the page and its QR code
    // stay as the user reads them), and a start without a password uses it
    let proposed = hotspot::proposed_key(false);
    assert_eq!(proposed.len(), 8);
    assert!(hotspot::password_ok("wpa-psk", &proposed));
    assert_eq!(hotspot::proposed_key(false), proposed, "the same key again");
    let wep = hotspot::proposed_key(true);
    assert!(wep.len() == 10 && hotspot::password_ok("none", &wep));
    assert_eq!(hotspot::proposed_key(true), wep);
    assert_eq!(
        hotspot::default_ssid("  laptop-van-robin  "),
        "laptop-van-robin"
    );
    let long = hotspot::default_ssid(&"é".repeat(20));
    assert_eq!(
        (long.len(), long.chars().count()),
        (32, 16),
        "cut at a character"
    );
    assert_eq!(hotspot::default_ssid(""), "Hotspot");
    let ch = hotspot::profile_changes(Some("Nieuw"), Some("geheim123"), Some(""), "wpa-psk");
    assert_eq!(
        ch,
        json!({ "802-11-wireless": { "ssid": "Nieuw", "band": null }, "802-11-wireless-security": { "psk": "geheim123" } })
    );
    let ch = hotspot::profile_changes(None, Some("abcde"), Some("bg"), "none");
    assert_eq!(
        ch,
        json!({ "802-11-wireless": { "band": "bg" }, "802-11-wireless-security": { "wep-key0": "abcde" } })
    );
    assert_eq!(
        hotspot::profile_changes(None, None, None, "wpa-psk"),
        json!({})
    );
    assert!(map::has_secrets(&hotspot::profile_changes(
        None,
        Some("abcd1234"),
        None,
        "wpa-psk"
    )));
    // a change keeps the rest of the stored hotspot
    let mut g = map::build(&hotspot::new_profile(
        UUID,
        "Oud",
        "abcd1234",
        "a",
        hotspot::CAP_AP | hotspot::CAP_RSN,
        "",
    ))
    .unwrap();
    map::merge(
        &mut g,
        &hotspot::profile_changes(Some("Nieuw"), None, Some(""), "wpa-psk"),
    )
    .unwrap();
    let j = map::to_json(&g);
    assert_eq!(map::ssid_text(&j["802-11-wireless"]["ssid"]), "Nieuw");
    assert!(j["802-11-wireless"]["band"].is_null());
    assert_eq!(j["802-11-wireless"]["mode"], "ap");
    assert_eq!(j["802-11-wireless-security"]["psk"], "abcd1234");
    assert_eq!(j["ipv4"]["method"], "shared");
    // the arguments: null keeps, a bad one names itself
    let lang = crate::i18n::Lang::En;
    let a = hotspot::start_args(&json!({}), lang).unwrap();
    assert!(a.ssid.is_none() && a.password.is_none() && a.band.is_none() && a.activate);
    let a = hotspot::start_args(
        &json!({ "ssid": "X", "password": null, "band": "a", "iface": "wlan0", "activate": false }),
        lang,
    )
    .unwrap();
    assert_eq!(
        (
            a.ssid.as_deref(),
            a.band.as_deref(),
            a.iface.as_deref(),
            a.activate
        ),
        (Some("X"), Some("a"), Some("wlan0"), false)
    );
    assert!(hotspot::start_args(&json!({ "iface": "../x" }), lang).is_err());
    assert!(hotspot::start_args(&json!({ "ssid": 5 }), lang).is_err());
}

#[test]
fn vpn_plugins_come_from_their_name_files() {
    let root = std::env::temp_dir().join(format!("win11-vpn-plugins-{}", crate::events::nonce()));
    let (lib, etc) = (root.join("lib"), root.join("etc"));
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::write(
        lib.join("nm-openvpn-service.name"),
        "[VPN Connection]\nname=openvpn\nservice=org.freedesktop.NetworkManager.openvpn\nprogram=/usr/libexec/nm-openvpn-service\n\n[libnm]\nplugin=libnm-vpn-plugin-openvpn.so\n\n[GNOME]\nproperties=libnm-openvpn-properties\n",
    )
    .unwrap();
    std::fs::write(
        lib.join("nm-pptp-service.name"),
        "[VPN Connection]\nname=pptp\nservice=org.freedesktop.NetworkManager.pptp\n",
    )
    .unwrap();
    std::fs::write(lib.join("broken.name"), "[VPN Connection]\nname=\n").unwrap();
    std::fs::write(lib.join("readme.txt"), "not a plugin").unwrap();
    // the same service again in /etc: the first directory wins
    std::fs::write(
        etc.join("copy.name"),
        "[VPN Connection]\nname=openvpn-copy\nservice=org.freedesktop.NetworkManager.openvpn\n",
    )
    .unwrap();
    let v = vpn::plugins(&[
        lib.as_path(),
        etc.as_path(),
        std::path::Path::new("/nonexistent"),
    ]);
    let list = v["plugins"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{v}");
    assert_eq!(
        (
            list[0]["name"].as_str(),
            list[0]["import"].as_bool(),
            list[0]["editor"].as_bool()
        ),
        (Some("openvpn"), Some(true), Some(true))
    );
    assert_eq!(
        (
            list[1]["name"].as_str(),
            list[1]["import"].as_bool(),
            list[1]["service"].as_str()
        ),
        (
            Some("pptp"),
            Some(false),
            Some("org.freedesktop.NetworkManager.pptp")
        )
    );
    assert_eq!(v["wireguard"], true);
    assert!(vpn::kind_installed("openvpn", &v) && vpn::kind_installed("wireguard", &v));
    assert!(!vpn::kind_installed("vpnc", &v));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn vpn_import_files_and_nmcli_answers() {
    assert_eq!(
        vpn::detect("/x/wg0.conf", "[Interface]\nPrivateKey = abc\n[Peer]\n"),
        Some("wireguard")
    );
    assert_eq!(vpn::detect("/x/office.ovpn", "anything"), Some("openvpn"));
    assert_eq!(
        vpn::detect(
            "/x/client.conf",
            "client\ndev tun\nremote vpn.example.org 1194\n"
        ),
        Some("openvpn")
    );
    assert_eq!(vpn::detect("/x/cisco.pcf", ""), Some("vpnc"));
    assert_eq!(vpn::detect("/x/notes.txt", "hello"), None);
    assert!(vpn::wireguard_name_ok("/home/robin/wg0.conf"));
    assert!(!vpn::wireguard_name_ok("/home/robin/my tunnel.conf"));
    assert!(!vpn::wireguard_name_ok(
        "/home/robin/averyveryverylongname.conf"
    ));
    assert_eq!(
        vpn::added_uuid(
            "Connection 'office' (3c2b1a09-8f7e-4d6c-9b5a-4e3d2c1b0a93) successfully added.\n"
        )
        .as_deref(),
        Some("3c2b1a09-8f7e-4d6c-9b5a-4e3d2c1b0a93")
    );
    assert_eq!(vpn::added_uuid("Connection 'a (b)' (nonsense) added"), None);
    assert_eq!(
        vpn::nmcli_error(
            "Error: failed to import 'x.ovpn': configuration error: unknown option.\n",
            ""
        ),
        "failed to import 'x.ovpn': configuration error: unknown option."
    );
    assert_eq!(vpn::nmcli_error("", "\nsomething\n"), "something");
    let lang = crate::i18n::Lang::En;
    let dir = std::env::temp_dir().join(format!("win11-vpn-import-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let ovpn = dir.join("kantoor.ovpn");
    std::fs::write(&ovpn, "client\nremote vpn.example.org\n").unwrap();
    let (path, kind) = vpn::import_args(&json!({ "path": ovpn.to_str().unwrap() }), lang).unwrap();
    assert_eq!((path.as_str(), kind), (ovpn.to_str().unwrap(), "openvpn"));
    let forced = vpn::import_args(
        &json!({ "path": ovpn.to_str().unwrap(), "type": "vpnc" }),
        lang,
    )
    .unwrap();
    assert_eq!(forced.1, "vpnc");
    assert!(
        vpn::import_args(
            &json!({ "path": ovpn.to_str().unwrap(), "type": "l2tp" }),
            lang
        )
        .is_err()
    );
    let e = vpn::import_args(&json!({ "path": dir.to_str().unwrap() }), lang).unwrap_err();
    assert_eq!(
        e.key.as_deref(),
        Some("net.badImport"),
        "a directory is no file"
    );
    let big = dir.join("big.ovpn");
    std::fs::write(&big, vec![b'#'; 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        vpn::import_args(&json!({ "path": big.to_str().unwrap() }), lang)
            .unwrap_err()
            .key
            .as_deref(),
        Some("net.badImport")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn wireguard_peers_and_vpn_secrets_keep_what_the_page_did_not_send() {
    let peers = json!([
        { "public-key": "AAA=", "endpoint": "wg.example.org:51820", "allowed-ips": ["0.0.0.0/0"], "persistent-keepalive": 25 },
        { "public-key": "BBB=", "allowed-ips": ["10.0.0.0/8"] },
    ]);
    let settings = json!({
        "connection": { "id": "wg0", "type": "wireguard", "interface-name": "wg0", "autoconnect": false },
        "wireguard": { "private-key": "cHJpdmF0ZQ==", "listen-port": 51820, "peer-routes": true, "peers": peers },
        "ipv4": { "method": "manual", "address-data": [{ "address": "10.8.0.2", "prefix": 24 }] },
        "ipv6": { "method": "disabled" },
    });
    let g = map::build(&settings).unwrap();
    assert_eq!(ty(&g, "wireguard", "peers"), "aa{sv}");
    assert_eq!(ty(&g, "wireguard", "listen-port"), "u");
    assert!(map::is_secret("wireguard", "private-key"));
    assert!(!map::has_secrets(
        &json!({ "wireguard": { "peers": peers } })
    ));
    assert!(map::has_secrets(
        &json!({ "wireguard": { "peers": [{ "public-key": "AAA=", "preshared-key": "PSK=" }] } })
    ));
    // a peer without its public key is refused
    let e = map::build(&json!({ "connection": { "id": "w", "type": "wireguard" }, "wireguard": { "peers": [{ "endpoint": "x:1" }] } })).unwrap_err();
    assert_eq!(
        e,
        MapError::BadValue {
            group: "wireguard".into(),
            key: "peers".into()
        }
    );
    // GetSecrets' pre-shared keys join the peers; a later peer list keeps the key of its public key
    let mut base = g.clone();
    let secrets: Groups = BTreeMap::from([(
        "wireguard".to_owned(),
        dict(vec![(
            "peers",
            vec![dict(vec![
                ("public-key", "AAA=".to_variant()),
                ("preshared-key", "PSK=".to_variant()),
            ])]
            .to_variant(),
        )]),
    )]);
    map::add_secrets(&mut base, &secrets);
    let j = map::to_json(&base);
    assert_eq!(j["wireguard"]["peers"][0]["preshared-key"], "PSK=");
    assert!(j["wireguard"]["peers"][1]["preshared-key"].is_null());
    map::merge(
        &mut base,
        &json!({ "wireguard": { "peers": [{ "public-key": "AAA=", "endpoint": "new.example.org:1" }, { "public-key": "CCC=" }] } }),
    )
    .unwrap();
    let j = map::to_json(&base);
    assert_eq!(j["wireguard"]["peers"][0]["endpoint"], "new.example.org:1");
    assert_eq!(
        j["wireguard"]["peers"][0]["preshared-key"], "PSK=",
        "kept by its public key"
    );
    assert_eq!(
        j["wireguard"]["peers"].as_array().unwrap().len(),
        2,
        "BBB= removed, CCC= added"
    );
    assert_eq!(j["wireguard"]["private-key"], "cHJpdmF0ZQ==");
    // vpn.secrets: GetSecrets fills the dictionary, the page's change sets or removes one entry
    let mut vpn: Groups = map::build(&json!({
        "connection": { "id": "Kantoor", "type": "vpn" },
        "vpn": { "service-type": "org.freedesktop.NetworkManager.openvpn", "data": { "remote": "vpn.example.org" }, "secrets": {} },
    }))
    .unwrap();
    let stored: Groups = BTreeMap::from([(
        "vpn".to_owned(),
        dict(vec![(
            "secrets",
            BTreeMap::from([
                ("password".to_owned(), "oud".to_owned()),
                ("cert-pass".to_owned(), "sleutel".to_owned()),
            ])
            .to_variant(),
        )]),
    )]);
    map::add_secrets(&mut vpn, &stored);
    assert_eq!(
        map::to_json(&vpn)["vpn"]["secrets"],
        json!({ "cert-pass": "sleutel", "password": "oud" })
    );
    map::merge(&mut vpn, &json!({ "vpn": { "secrets": { "password": "nieuw", "cert-pass": null, "http-proxy-password": "p" } } })).unwrap();
    assert_eq!(
        map::to_json(&vpn)["vpn"]["secrets"],
        json!({ "http-proxy-password": "p", "password": "nieuw" })
    );
    assert_eq!(
        map::to_json(&vpn)["vpn"]["data"],
        json!({ "remote": "vpn.example.org" }),
        "data is replaced only when sent"
    );
    let e = map::merge(
        &mut vpn,
        &json!({ "vpn": { "secrets": { "password": 1 } } }),
    )
    .unwrap_err();
    assert_eq!(
        e,
        MapError::BadValue {
            group: "vpn".into(),
            key: "secrets".into()
        }
    );
    // gsm and ppp keys are typed (the APN dialog)
    let gsm = map::build(&json!({
        "connection": { "id": "KPN", "type": "gsm" },
        "gsm": { "apn": "portalmmm.nl", "home-only": true, "sim-id": "8931", "initial-eps-bearer-configure": true, "password-flags": 0 },
        "ppp": { "refuse-pap": true, "lcp-echo-interval": 30 },
    }))
    .unwrap();
    assert_eq!(
        (
            ty(&gsm, "gsm", "home-only"),
            ty(&gsm, "gsm", "password-flags"),
            ty(&gsm, "ppp", "lcp-echo-interval")
        ),
        ("b".into(), "u".into(), "u".into())
    );
}

/// A GetManagedObjects answer as ModemManager 1.24 sends it: one locked modem with its SIM.
fn mm_objects() -> glib::Variant {
    let text = "({objectpath '/org/freedesktop/ModemManager1/Modem/0': {\
        'org.freedesktop.ModemManager1.Modem': {\
            'Sim': <objectpath '/org/freedesktop/ModemManager1/SIM/0'>,\
            'SimSlots': <[objectpath '/org/freedesktop/ModemManager1/SIM/0', '/']>,\
            'PrimarySimSlot': <uint32 1>, 'Manufacturer': <'Quectel'>, 'Model': <'EM05-G'>,\
            'Revision': <'EM05GFAR07A07M1G'>, 'EquipmentIdentifier': <'861536030196001'>,\
            'DeviceIdentifier': <'7f1c0b5e2a'>, 'UnlockRequired': <uint32 2>,\
            'UnlockRetries': <{uint32 2: uint32 3, 4: 10}>, 'State': <2>, 'StateFailedReason': <uint32 0>,\
            'AccessTechnologies': <uint32 16416>, 'SignalQuality': <(uint32 72, true)>,\
            'OwnNumbers': <['+31612345678']>,\
            'SupportedModes': <[(uint32 14, uint32 8), (8, 0)]>, 'CurrentModes': <(uint32 14, uint32 8)>},\
        'org.freedesktop.ModemManager1.Modem.Modem3gpp': {\
            'Imei': <'861536030196001'>, 'RegistrationState': <uint32 5>, 'OperatorCode': <'20408'>,\
            'OperatorName': <'KPN'>, 'EnabledFacilityLocks': <uint32 1>}},\
      objectpath '/org/freedesktop/ModemManager1/SIM/0': {\
        'org.freedesktop.ModemManager1.Sim': {\
            'Active': <true>, 'SimIdentifier': <'8931440000000000017'>, 'Imsi': <'204080000000001'>,\
            'OperatorIdentifier': <'20408'>, 'OperatorName': <'KPN'>, 'SimType': <uint32 1>}}},)";
    glib::Variant::parse(Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()), text).unwrap()
}

#[test]
fn a_modem_as_the_mobile_page_shows_it() {
    let v = mm_objects();
    assert_eq!(v.type_().as_str(), "(a{oa{sa{sv}}})");
    let objects = modem::objects_json(&v);
    let path = "/org/freedesktop/ModemManager1/Modem/0";
    let m = modem::modem_json(path, &objects[path], &objects);
    assert_eq!(
        (
            m["manufacturer"].as_str(),
            m["model"].as_str(),
            m["firmware"].as_str()
        ),
        (Some("Quectel"), Some("EM05-G"), Some("EM05GFAR07A07M1G"))
    );
    assert_eq!(
        (m["state"].as_str(), m["unlockRequired"].as_str()),
        (Some("locked"), Some("sim-pin"))
    );
    assert_eq!(m["unlockRetries"], json!({ "sim-pin": 3, "sim-puk": 10 }));
    assert_eq!(
        (m["networkType"].as_str(), m["signal"].as_u64()),
        (Some("4G"), Some(72))
    );
    assert_eq!(m["technologies"], json!(["umts", "lte"]));
    assert_eq!(m["registration"], "roaming");
    assert_eq!(m["pinEnabled"], true);
    assert_eq!(m["imei"], "861536030196001");
    assert_eq!(m["sim"]["iccid"], "8931440000000000017");
    assert_eq!(
        (m["sim"]["mcc"].as_str(), m["sim"]["mnc"].as_str()),
        (Some("204"), Some("08"))
    );
    assert_eq!(m["simSlots"].as_array().map(Vec::len), Some(2));
    assert!(m["simSlots"][1]["sim"].is_null(), "an empty slot");
    assert_eq!(
        m["supportedModes"][0],
        json!({ "allowed": 14, "preferred": 8, "allowedNames": ["4g", "3g", "2g"], "preferredNames": ["4g"] })
    );
    assert_eq!(m["currentModes"]["allowed"], 14);
    // the names
    assert_eq!(
        (
            modem::lock_name(4),
            modem::lock_name(1),
            modem::lock_name(99)
        ),
        ("sim-puk", "none", "unknown")
    );
    assert_eq!(
        (modem::state_name(-1), modem::state_name(11)),
        ("failed", "connected")
    );
    assert_eq!(modem::failed_reason(2), "sim-missing");
    assert_eq!(
        (
            modem::generation(1 << 15),
            modem::generation(1 << 9),
            modem::generation(1 << 4),
            modem::generation(0)
        ),
        ("5G", "3G", "2G", "")
    );
    assert_eq!(modem::mode_names(0xffff_ffff), vec!["any"]);
    assert!(
        modem::modem_ok(path)
            && !modem::modem_ok("/org/freedesktop/ModemManager1/Modem/")
            && !modem::modem_ok("/org/freedesktop/ModemManager1/Modem/0/../1")
    );
    assert!(
        modem::pin_ok("1234")
            && modem::pin_ok("12345678")
            && !modem::pin_ok("123")
            && !modem::pin_ok("12a4")
    );
    assert!(modem::puk_ok("12345678") && !modem::puk_ok("1234567"));
    assert!(
        modem::modes_ok(14, 8)
            && modem::modes_ok(14, 0)
            && !modem::modes_ok(14, 12)
            && !modem::modes_ok(14, 16)
            && !modem::modes_ok(0, 0)
    );
    assert!(
        modem::operator_ok("")
            && modem::operator_ok("20408")
            && modem::operator_ok("310410")
            && !modem::operator_ok("2040")
    );
    let n = modem::network_json(
        &json!({ "status": 2, "operator-long": "KPN", "operator-short": "KPN", "operator-code": "20408", "access-technology": 1 << 15 }),
    );
    assert_eq!(
        n,
        json!({ "code": "20408", "name": "KPN", "short": "KPN", "status": "current", "networkType": "5G" })
    );
}

#[test]
fn apn_sign_in_and_gsm_profiles() {
    for auth in ["auto", "none", "pap", "chap", "mschapv2"] {
        let ppp = modem::apn_auth_ppp(auth).unwrap();
        let settings = json!({ "ppp": ppp });
        assert_eq!(modem::apn_auth(&settings), auth, "{auth}: {ppp}");
    }
    assert!(modem::apn_auth_ppp("kerberos").is_none());
    assert_eq!(
        modem::apn_auth(&json!({ "ppp": { "refuse-pap": true } })),
        "other"
    );
    let j = json!({
        "connection": { "uuid": UUID, "id": "KPN Internet", "type": "gsm", "autoconnect": false },
        "gsm": { "apn": "portalmmm.nl", "home-only": true, "sim-id": "8931" },
    });
    let g = modem::gsm_json("/org/freedesktop/NetworkManager/Settings/5", &j, None);
    assert_eq!(
        (
            g["apn"].as_str(),
            g["autoconnect"].as_bool(),
            g["homeOnly"].as_bool(),
            g["active"].as_bool(),
            g["auth"].as_str()
        ),
        (
            Some("portalmmm.nl"),
            Some(false),
            Some(true),
            Some(false),
            Some("auto")
        )
    );
}

#[test]
fn serviceproviders_give_the_access_points() {
    let xml = r#"<?xml version="1.0" encoding='utf-8'?>
<!DOCTYPE serviceproviders SYSTEM "serviceproviders.2.dtd">
<serviceproviders format="2.0">
<country code="nl">
	<name>Netherlands</name>
	<provider>
		<name>Hi</name>
		<gsm>
			<network-id mcc="204" mnc="08"/>
			<apn value="portalmmm.nl">
				<plan type="postpaid"/>
				<usage type="internet"/>
			</apn>
			<apn value="portalmmm.nl">
				<usage type="mms"/>
				<name>KPN MMS</name>
				<username>KPN</username>
				<password>gprs</password>
				<authentication method="chap"/>
				<dns>10.0.0.1</dns>
			</apn>
		</gsm>
	</provider>
	<provider><name>Other</name><gsm><network-id mcc="204" mnc="16"/><apn value="x"/></gsm></provider>
	<provider><name>CDMA</name><cdma><sid value="1"/></cdma></provider>
</country>
</serviceproviders>"#;
    let v = providers::apns(xml, "204", "08");
    let list = v["providers"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        (list[0]["name"].as_str(), list[0]["country"].as_str()),
        (Some("Hi"), Some("nl"))
    );
    let apns = list[0]["apns"].as_array().unwrap();
    assert_eq!(
        apns[0],
        json!({ "apn": "portalmmm.nl", "name": "", "usage": ["internet"], "username": "", "password": "", "auth": "", "dns": [] })
    );
    assert_eq!(
        (
            apns[1]["name"].as_str(),
            apns[1]["username"].as_str(),
            apns[1]["auth"].as_str(),
            apns[1]["dns"].clone()
        ),
        (
            Some("KPN MMS"),
            Some("KPN"),
            Some("chap"),
            json!(["10.0.0.1"])
        )
    );
    assert_eq!(providers::apns(xml, "204", "99")["providers"], json!([]));
    assert_eq!(
        providers::apns("<not xml", "204", "08")["providers"],
        json!([])
    );
    assert!(
        providers::code_ok("204", true)
            && providers::code_ok("08", false)
            && providers::code_ok("410", false)
    );
    assert!(!providers::code_ok("20", true) && !providers::code_ok("0x", false));
    // the installed database parses too (mobile-broadband-provider-info), when it is there
    if let Ok(text) = std::fs::read_to_string(providers::FILE) {
        let kpn = providers::apns(&text, "204", "08");
        assert!(
            !kpn["providers"].as_array().unwrap().is_empty(),
            "204 08 is in the database"
        );
    }
}

// ------------------------------------------------------------------- slice 2 bus tests (dbus-fakes)

/// The calls the ModemManager fake got.
fn mm_calls() -> Vec<Value> {
    std::env::var("WIN11_FAKE_CALLS_MODEMMANAGER")
        .ok()
        .and_then(|f| std::fs::read_to_string(f).ok())
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn ssid_in(body: &str, ssid: &str) -> bool {
    let bytes: Vec<String> = ssid.bytes().map(|b| format!("0x{b:02x}")).collect();
    body.contains(&format!("'ssid': <b'{ssid}'>"))
        || body.contains(&format!("'ssid': <[byte {}]>", bytes.join(", ")))
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_the_hotspot_is_made_as_gnome_does_then_changed_and_stopped() {
    let Some((_lc, net)) = bus_net() else { return };
    let h = net.call("net.hotspot.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            h["available"].as_bool(),
            h["configured"].as_bool(),
            h["active"].as_bool(),
            h["apCapable"].as_bool()
        ),
        (Some(true), Some(false), Some(false), Some(true)),
        "{h}"
    );
    assert_eq!(h["iface"], "wlan0");
    assert_eq!(
        h["password"].as_str().map(str::len),
        Some(8),
        "a new random key"
    );
    assert_eq!(
        (h["security"].as_str(), h["keyMgmt"].as_str()),
        (Some("wpa2"), Some("wpa-psk"))
    );
    assert_eq!(
        (h["uplink"]["id"].as_str(), h["sameAdapter"].as_bool()),
        (Some("Lab-5G"), Some(true))
    );
    let n = fake_calls().unwrap().len();
    net.call(
        "net.hotspot.start",
        &json!({ "ssid": "WinPoint", "password": "abcd1234", "band": "bg" }),
    )
    .unwrap()
    .unwrap();
    let calls = calls_since(n);
    let add = find(&calls, "AddAndActivateConnection2");
    assert_eq!(add["interactive"], true);
    let body = add["args"].as_str().unwrap();
    for part in [
        "'id': <'Hotspot'>",
        "'autoconnect': <false>",
        "'mode': <'ap'>",
        "'band': <'bg'>",
        "'method': <'shared'>",
        "'method': <'ignore'>",
        "'key-mgmt': <'wpa-psk'>",
        "'proto': <['rsn']>",
        "'pairwise': <['ccmp']>",
        "'psk': <'abcd1234'>",
        "objectpath '/org/freedesktop/NetworkManager/Devices/1'",
        "'persist': <'disk'>",
    ] {
        assert!(body.contains(part), "{part}: {body}");
    }
    assert!(ssid_in(body, "WinPoint"), "{body}");
    let h = net.call("net.hotspot.get", &json!({})).unwrap().unwrap();
    assert_eq!(
        (
            h["configured"].as_bool(),
            h["active"].as_bool(),
            h["ssid"].as_str(),
            h["band"].as_str()
        ),
        (Some(true), Some(true), Some("WinPoint"), Some("bg")),
        "{h}"
    );
    assert!(h["password"].is_null(), "no key without secrets");
    assert!(h["uplink"].is_null() || h["uplink"]["id"] != "Hotspot");
    let n = fake_calls().unwrap().len();
    let h = net
        .call("net.hotspot.get", &json!({ "secrets": true }))
        .unwrap()
        .unwrap();
    assert_eq!(h["password"], "abcd1234");
    assert_eq!(h["qr"], "WIFI:S:WinPoint;T:WPA;P:abcd1234;;");
    assert_eq!(find(&calls_since(n), "GetSecrets")["interactive"], true);
    // a change of the stored hotspot: Update2 (the key kept, so no GetSecrets), then activated again
    let n = fake_calls().unwrap().len();
    net.call(
        "net.hotspot.start",
        &json!({ "ssid": "WinPoint 2", "band": "" }),
    )
    .unwrap()
    .unwrap();
    let calls = calls_since(n);
    let update = find(&calls, "Update2");
    let body = update["args"].as_str().unwrap();
    assert!(
        ssid_in(body, "WinPoint 2") && !body.contains("'band'"),
        "{body}"
    );
    assert!(body.contains("'mode': <'ap'>"), "the rest kept: {body}");
    assert!(calls.iter().all(|c| c["method"] != "GetSecrets"));
    assert!(
        calls
            .iter()
            .all(|c| c["method"] != "AddAndActivateConnection2"),
        "no second profile"
    );
    let at = |m: &str| calls.iter().position(|c| c["method"] == m).unwrap();
    assert!(at("Update2") < at("ActivateConnection"));
    assert_eq!(find(&calls, "ActivateConnection")["interactive"], true);
    // only saving (Windows' Bewerken while it is off) activates nothing
    let n = fake_calls().unwrap().len();
    net.call(
        "net.hotspot.start",
        &json!({ "password": "nieuwesleutel", "activate": false }),
    )
    .unwrap()
    .unwrap();
    let calls = calls_since(n);
    assert!(
        find(&calls, "Update2")["args"]
            .as_str()
            .unwrap()
            .contains("'psk': <'nieuwesleutel'>")
    );
    assert!(calls.iter().all(|c| c["method"] != "ActivateConnection"));
    // off: every active hotspot deactivated
    let n = fake_calls().unwrap().len();
    let r = net.call("net.hotspot.stop", &json!({})).unwrap().unwrap();
    assert!(r["stopped"].as_u64().unwrap() >= 1, "{r}");
    assert_eq!(
        find(&calls_since(n), "DeactivateConnection")["interactive"],
        true
    );
    let h = net.call("net.hotspot.get", &json!({})).unwrap().unwrap();
    assert_eq!(h["active"], false);
    let r = net.call("net.hotspot.stop", &json!({})).unwrap().unwrap();
    assert_eq!(r["stopped"], 0, "nothing to stop");
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_the_modem_is_unlocked_and_set_as_gnomes_mobile_network_panel_does() {
    let Some((_lc, net)) = bus_net() else { return };
    let get = || net.call("net.modem.get", &json!({})).unwrap().unwrap();
    let v = get();
    assert_eq!(v["available"], true);
    let m = &v["modems"][0];
    assert_eq!(
        (m["state"].as_str(), m["unlockRequired"].as_str()),
        (Some("locked"), Some("sim-pin")),
        "{m}"
    );
    assert_eq!(m["unlockRetries"]["sim-pin"], 3);
    assert_eq!(
        (m["iface"].as_str(), m["model"].as_str()),
        (Some("cdc-wdm0"), Some("EM05-G"))
    );
    let conns = m["connections"].as_array().unwrap();
    assert_eq!(conns.len(), 1, "the gsm profile of this SIM: {m}");
    assert_eq!(
        (conns[0]["apn"].as_str(), conns[0]["homeOnly"].as_bool()),
        (Some("portalmmm.nl"), Some(true))
    );
    assert_eq!(v["wwanEnabled"], true);
    let modem = m["path"].as_str().unwrap().to_owned();
    // a wrong PIN costs a try
    let e = net
        .call(
            "net.modem.unlock",
            &json!({ "modem": modem, "pin": "0000" }),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(e.code, "failed");
    assert_eq!(get()["modems"][0]["unlockRetries"]["sim-pin"], 2);
    let n = mm_calls().len();
    net.call(
        "net.modem.unlock",
        &json!({ "modem": modem, "pin": "1234" }),
    )
    .unwrap()
    .unwrap();
    let calls: Vec<Value> = mm_calls().into_iter().skip(n).collect();
    let send = find(&calls, "SendPin");
    assert_eq!(
        (send["args"].as_str(), send["interactive"].as_bool()),
        (Some("('1234',)"), Some(true))
    );
    assert_eq!(send["path"], "/org/freedesktop/ModemManager1/SIM/0");
    let m = get()["modems"][0].clone();
    assert_eq!(
        (
            m["unlockRequired"].as_str(),
            m["registration"].as_str(),
            m["operatorName"].as_str(),
            m["networkType"].as_str()
        ),
        (Some("none"), Some("home"), Some("KPN"), Some("4G"))
    );
    // network mode, operators, SIM lock, slots: each an interactive ModemManager call
    let cases: Vec<(&str, Value, &str, &str)> = vec![
        (
            "net.modem.setModes",
            json!({ "modem": modem, "allowed": 14, "preferred": 4 }),
            "SetCurrentModes",
            "((uint32 14, uint32 4),)",
        ),
        (
            "net.modem.register",
            json!({ "modem": modem, "operator": "20404" }),
            "Register",
            "('20404',)",
        ),
        (
            "net.modem.register",
            json!({ "modem": modem }),
            "Register",
            "('',)",
        ),
        (
            "net.modem.setPin",
            json!({ "modem": modem, "enabled": false, "pin": "1234" }),
            "EnablePin",
            "('1234', false)",
        ),
        (
            "net.modem.changePin",
            json!({ "modem": modem, "old": "1234", "new": "4321" }),
            "ChangePin",
            "('1234', '4321')",
        ),
        (
            "net.modem.setPrimarySlot",
            json!({ "modem": modem, "slot": 2 }),
            "SetPrimarySimSlot",
            "(uint32 2,)",
        ),
    ];
    for (method, args, dbus, text) in cases {
        let n = mm_calls().len();
        let r = net.call(method, &args).unwrap();
        assert!(r.is_ok(), "{method}: {:?}", r.err());
        let calls: Vec<Value> = mm_calls().into_iter().skip(n).collect();
        let c = find(&calls, dbus);
        assert_eq!(
            (c["args"].as_str(), c["interactive"].as_bool()),
            (Some(text), Some(true)),
            "{method}"
        );
    }
    let m = get()["modems"][0].clone();
    assert_eq!(
        (m["pinEnabled"].as_bool(), m["primarySimSlot"].as_u64()),
        (Some(false), Some(2))
    );
    assert_eq!(m["currentModes"]["preferred"], 4);
    // a mode pair the modem does not support, a slot it does not have, a modem that is not there
    for (method, args) in [
        (
            "net.modem.setModes",
            json!({ "modem": modem, "allowed": 6, "preferred": 4 }),
        ),
        (
            "net.modem.setPrimarySlot",
            json!({ "modem": modem, "slot": 3 }),
        ),
    ] {
        assert_eq!(
            net.call(method, &args).unwrap().unwrap_err().code,
            "bad_args",
            "{method}"
        );
    }
    let e = net
        .call(
            "net.modem.register",
            &json!({ "modem": "/org/freedesktop/ModemManager1/Modem/9" }),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("net.noModem"));
    // the scan: the current network first, a forbidden one last
    let n = mm_calls().len();
    let s = net
        .call("net.modem.scan", &json!({ "modem": modem }))
        .unwrap()
        .unwrap();
    let names: Vec<&str> = s["networks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["KPN", "Vodafone NL", "Odido"]);
    assert_eq!(s["networks"][0]["networkType"], "5G");
    let calls: Vec<Value> = mm_calls().into_iter().skip(n).collect();
    assert_eq!(find(&calls, "Scan")["interactive"], true);
    // the read-only calls never allow a password dialog
    assert!(
        mm_calls()
            .iter()
            .filter(|c| c["method"] == "GetManagedObjects" || c["method"] == "GetAll")
            .all(|c| c["interactive"] == false)
    );
}

#[test]
fn the_proxy_pages_writes_pass_the_allow_rows_and_the_password_never_does() {
    use crate::services::gsettings::write_values;
    let lang = crate::i18n::Lang::En;
    // the writes of the proxy page's Opslaan (native/apps/settings/src/pages/proxy.rs manual_writes), mode last
    let writes: Vec<(&str, Vec<(&str, Value)>)> = vec![
        (
            "org.gnome.system.proxy.http",
            vec![("host", json!("proxy.example.org")), ("port", json!(3128))],
        ),
        (
            "org.gnome.system.proxy.https",
            vec![("host", json!("proxy.example.org")), ("port", json!(3128))],
        ),
        ("org.gnome.system.proxy.ftp", vec![("host", json!(""))]),
        (
            "org.gnome.system.proxy.socks",
            vec![("host", json!("socks.example.org")), ("port", json!(1080))],
        ),
        (
            "org.gnome.system.proxy",
            vec![(
                "ignore-hosts",
                json!(["localhost", "127.0.0.0/8", "::1", "*.local"]),
            )],
        ),
        (
            "org.gnome.system.proxy",
            vec![("autoconfig-url", json!("http://wpad/wpad.dat"))],
        ),
        ("org.gnome.system.proxy", vec![("mode", json!("manual"))]),
    ];
    for (schema, values) in &writes {
        let r = write_values("gsettings.set", schema, None, values, lang, true);
        assert!(r.is_ok(), "{schema}: {:?}", r.err());
    }
    let e = write_values(
        "gsettings.set",
        "org.gnome.system.proxy.http",
        None,
        &[("authentication-password", json!("geheim"))],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(
        e.code, "denied",
        "GNOME keeps it in plain text: never written"
    );
    // keys GNOME Settings 50 has no control for (gnome-b.json: "have no UI") are Read rows, never written
    for (schema, key, value) in [
        ("org.gnome.system.proxy", "use-same-proxy", json!(false)),
        ("org.gnome.system.proxy.http", "enabled", json!(true)),
        (
            "org.gnome.system.proxy.http",
            "use-authentication",
            json!(true),
        ),
        (
            "org.gnome.system.proxy.http",
            "authentication-user",
            json!("x"),
        ),
    ] {
        let e =
            write_values("gsettings.set", schema, None, &[(key, value)], lang, true).unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.key.as_deref()),
            ("denied", Some("gsettings.readOnly")),
            "{schema} {key}"
        );
        assert!(
            crate::services::gsettings::read_item(schema, None, key, lang).is_ok(),
            "{schema} {key} can still be read"
        );
    }
    let e = write_values(
        "gsettings.set",
        "org.gnome.system.proxy",
        None,
        &[("mode", json!("sometimes"))],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(e.code, "bad_args");
    let e = write_values(
        "gsettings.set",
        "org.gnome.system.proxy.http",
        None,
        &[("port", json!("8080"))],
        lang,
        true,
    )
    .unwrap_err();
    assert_eq!(e.code, "bad_args", "a port is a number");
}

#[test]
fn attach_bearer_credentials_are_typed_and_preserved_as_secrets() {
    assert!(map::has_secrets(
        &json!({"gsm": {"initial-eps-bearer-password": "private"}})
    ));
    let mut groups = map::groups_of(&fixture());
    map::merge(
        &mut groups,
        &json!({"gsm": {
            "initial-eps-bearer-username": "attach-user",
            "initial-eps-bearer-password": "private",
            "initial-eps-bearer-password-flags": 0,
            "initial-eps-bearer-noauth": false,
            "initial-eps-bearer-refuse-chap": true
        }}),
    )
    .unwrap();
    assert_eq!(
        groups["gsm"]["initial-eps-bearer-password-flags"]
            .type_()
            .as_str(),
        "u"
    );
    assert_eq!(
        groups["gsm"]["initial-eps-bearer-noauth"],
        false.to_variant()
    );
    map::merge(&mut groups, &json!({"gsm": {"apn": "changed"}})).unwrap();
    assert_eq!(
        groups["gsm"]["initial-eps-bearer-password"],
        "private".to_variant()
    );
}

#[test]
#[ignore = "batch C bus test: only tests/dbus-fakes/run.sh --batch c"]
fn bus_vpn_offline_import_saves_typed_settings_with_interactive_authorization() {
    let Some((_lc, net)) = bus_net() else { return };
    let root = std::path::PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap())
        .join("vpn-import-fixture");
    std::fs::create_dir_all(&root).unwrap();
    let path = root.join("wgtest.conf");
    std::fs::write(&path, "[Interface]\nPrivateKey = AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=\nAddress = 10.70.0.2/24\n[Peer]\nPublicKey = ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=\nAllowedIPs = 10.70.0.0/24\nEndpoint = vpn.example.invalid:51820\n").unwrap();
    let n = fake_calls().unwrap().len();
    let result = net
        .call("net.vpn.import", &json!({"path": path}))
        .unwrap()
        .unwrap();
    assert_eq!(result["type"], "wireguard");
    assert!(uuid_ok(result["uuid"].as_str().unwrap()));
    let calls = calls_since(n);
    let add = find(&calls, "AddConnection2");
    assert_eq!(add["interactive"], true);
    assert!(add["args"].as_str().unwrap().contains("'wireguard'"));
    assert!(add["args"].as_str().unwrap().contains("'private-key'"));

    // Host root becomes uid 65534 inside the user namespace. Copy the read-only system importer
    // into our private fixture so stock libnm ownership checks still apply to the actual test uid.
    let module = root.join("libnm-vpn-plugin-openvpn.so");
    std::fs::copy(
        "/usr/lib/x86_64-linux-gnu/NetworkManager/libnm-vpn-plugin-openvpn.so",
        &module,
    )
    .unwrap();
    let name = root.join("nm-openvpn-service.name");
    let descriptor =
        std::fs::read_to_string("/usr/lib/NetworkManager/VPN/nm-openvpn-service.name").unwrap();
    std::fs::write(
        &name,
        descriptor.replace(
            "plugin=libnm-vpn-plugin-openvpn.so",
            &format!("plugin={}", module.display()),
        ),
    )
    .unwrap();
    let ca = root.join("ca.crt");
    std::fs::write(&ca, "private parser fixture, not a real certificate\n").unwrap();
    let ovpn = root.join("private.ovpn");
    std::fs::write(
        &ovpn,
        format!(
            "client\ndev tun\nproto udp\nremote vpn.example.invalid 1194\nauth-user-pass\nca {}\n",
            ca.display()
        ),
    )
    .unwrap();
    let groups =
        super::vpn_import::parse(ovpn.to_str().unwrap(), "openvpn", Some(&name), Lang::En).unwrap();
    assert_eq!(
        groups["vpn"]["service-type"],
        "org.freedesktop.NetworkManager.openvpn".to_variant()
    );
    let n = fake_calls().unwrap().len();
    super::vpn::save_import(&system_bus(Lang::En).unwrap(), &groups, "openvpn", Lang::En).unwrap();
    let calls = calls_since(n);
    let add = find(&calls, "AddConnection2");
    assert_eq!(add["interactive"], true);
    assert!(add["args"].as_str().unwrap().contains("'service-type'"));
    std::fs::remove_dir_all(root).unwrap();
}
