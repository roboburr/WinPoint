//! Batch B's `devices.*` tests: the pure parsers on fixtures, the argument checks, the dry run, and the daemon
//! calls against the fakes of fake_bus.rs on the private session bus (the calls recorded, the interactive flag).
use super::fake_bus::{Fake, Handler, Object, Props};
use super::*;
use crate::services::system::common::LangContext;
use glib::variant::ToVariant;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn lang() -> Lang {
    Lang::En
}

// ------------------------------------------------------------------------------------------------ batteries

#[test]
fn connected_devices_leave_the_computers_own_power_out() {
    let mouse = json!({"Type": 5, "Model": "MX Master 3 ", "Vendor": "Logitech", "Percentage": 55.4, "State": 2,
        "IsPresent": true, "NativePath": "hidpp_battery_0", "IconName": "battery-good-symbolic", "BatteryLevel": 1});
    let item = batteries::item("/org/freedesktop/UPower/devices/mouse_dev_1", &mouse).unwrap();
    assert_eq!(item["id"], "mouse_dev_1");
    assert_eq!(item["kind"], "mouse");
    assert_eq!(item["model"], "MX Master 3");
    assert_eq!(item["percentage"], 55);
    assert_eq!(item["state"], "discharging");
    assert_eq!(item["level"], Value::Null);
    assert_eq!(item["address"], Value::Null);
    // the laptop's own battery and the line power are Power & battery's
    let own = json!({"Type": 2, "PowerSupply": true, "Percentage": 80.0, "IsPresent": true});
    assert!(batteries::item("/x/battery_BAT0", &own).is_none());
    assert!(batteries::item("/x/line_power_AC", &json!({"Type": 1, "Online": true})).is_none());
    // a UPS powers the computer too, but GNOME lists it
    let ups =
        json!({"Type": 3, "PowerSupply": true, "Percentage": 100.0, "State": 4, "IsPresent": true});
    assert_eq!(
        batteries::item("/x/ups_hiddev0", &ups).unwrap()["state"],
        "full"
    );
    // a peripheral battery that is not there
    assert!(
        batteries::item(
            "/x/gone",
            &json!({"Type": 6, "IsPresent": false, "Percentage": 0.0})
        )
        .is_none()
    );
    // a coarse level
    let kb = json!({"Type": 6, "BatteryLevel": 7, "Percentage": 70.0, "IsPresent": true});
    assert_eq!(batteries::item("/x/kbd", &kb).unwrap()["level"], "high");
}

#[test]
fn a_bluetooth_battery_carries_its_address() {
    assert_eq!(
        batteries::address_of("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_0F", "").as_deref(),
        Some("AA:BB:CC:DD:EE:0F")
    );
    assert_eq!(
        batteries::address_of("hid-aa:bb:cc:dd:ee:ff-battery", "").as_deref(),
        Some("AA:BB:CC:DD:EE:FF")
    );
    assert_eq!(
        batteries::address_of("hidpp_battery_3", "11:22:33:44:55:66").as_deref(),
        Some("11:22:33:44:55:66")
    );
    assert_eq!(batteries::address_of("hidpp_battery_3", "4031-ab-cd"), None);
    assert_eq!(batteries::address_of("/org/bluez/hci0/dev_AA_BB", ""), None);
    assert_eq!(batteries::kind_name(17), "headset");
    assert_eq!(batteries::kind_name(99), "unknown");
}

// ---------------------------------------------------------------------------------------------- thunderbolt

fn bolt_device(uid: &str, status: &str, stored: bool) -> Value {
    json!({"Uid": uid, "Name": "Thunderbolt Dock", "Vendor": "Lenovo", "Type": "peripheral", "Status": status,
        "Stored": stored, "Policy": "default", "Key": "missing", "Parent": "host-1", "Domain": "domain0",
        "Generation": 3, "ConnectTime": 1_700_000_000u64, "AuthorizeTime": 0, "StoreTime": 0, "Label": ""})
}

#[test]
fn bolt_devices_read_as_gnomes_device_dialog() {
    let new = thunderbolt::device_json(
        "/d/1",
        &bolt_device("c6030000-0080-7f18", "connected", false),
    );
    assert_eq!(
        new["pending"], true,
        "connected, not authorized, not stored: GNOME's Pending Devices"
    );
    assert_eq!(new["authorized"], false);
    assert_eq!(new["connectTime"], 1_700_000_000u64);
    assert_eq!(new["authorizeTime"], Value::Null, "0 is no time");
    let known = thunderbolt::device_json("/d/2", &bolt_device("x", "authorized-secure", true));
    assert_eq!(
        (known["authorized"].clone(), known["pending"].clone()),
        (json!(true), json!(false))
    );
    let away = thunderbolt::device_json("/d/3", &bolt_device("y", "disconnected", true));
    assert_eq!(
        (away["connected"].clone(), away["pending"].clone()),
        (json!(false), json!(false))
    );
}

#[test]
fn bolt_arguments_are_checked() {
    assert!(thunderbolt::uid_ok("c6030000-0080-7f18-23e2-0c81d4c3d108"));
    for bad in ["", "a b", "../x", "x;y", &"a".repeat(65)] {
        assert!(!thunderbolt::uid_ok(bad), "{bad:?}");
    }
    assert_eq!(
        thunderbolt::mode_arg(&json!("disabled"), lang()).unwrap(),
        "disabled"
    );
    assert_eq!(
        thunderbolt::mode_arg(&json!("off"), lang())
            .unwrap_err()
            .code,
        "bad_args"
    );
    assert_eq!(
        thunderbolt::policy_arg(&Value::Null, lang()).unwrap(),
        "default"
    );
    assert_eq!(
        thunderbolt::policy_arg(&json!("iommu"), lang()).unwrap(),
        "iommu"
    );
    assert!(thunderbolt::policy_arg(&json!(1), lang()).is_err());
}

// ------------------------------------------------------------------------------------------------ bluetooth

#[test]
fn the_adapter_reads_as_the_panel_shows_it() {
    let objects = json!({
        "/org/bluez/hci1": {"org.bluez.Adapter1": {"Address": "aa:bb:cc:00:11:22", "Alias": "", "Name": "second"}},
        "/org/bluez/hci0": {"org.bluez.Adapter1": {"Address": "aa:bb:cc:dd:ee:ff", "Alias": "werkplek",
            "Name": "BlueZ 5.85", "Powered": true, "Discoverable": false, "DiscoverableTimeout": 180}},
        "/org/bluez/hci0/dev_11": {"org.bluez.Device1": {"Address": "11:22:33:44:55:66"}},
    });
    let (path, props) = bluetooth::first_adapter(&objects).unwrap();
    assert_eq!(
        path, "/org/bluez/hci0",
        "the lowest path, as system.bluetooth.get"
    );
    let a = bluetooth::adapter_json(&path, &props);
    assert_eq!(
        (a["alias"].clone(), a["address"].clone()),
        (json!("werkplek"), json!("AA:BB:CC:DD:EE:FF"))
    );
    assert_eq!(
        (a["powered"].clone(), a["discoverable"].clone()),
        (json!(true), json!(false))
    );
    // no alias: the name
    let b = bluetooth::adapter_json(
        "/org/bluez/hci1",
        &objects["/org/bluez/hci1"]["org.bluez.Adapter1"],
    );
    assert_eq!(b["alias"], "second");
}

#[test]
fn send_files_arguments_are_checked() {
    let ok = bluetooth::send_args(
        &json!({"address": "aa:bb:cc:dd:ee:ff", "name": " Telefoon\n van Robin "}),
        lang(),
    );
    assert_eq!(
        ok.unwrap(),
        (
            "AA:BB:CC:DD:EE:FF".to_owned(),
            "Telefoon van Robin".to_owned()
        )
    );
    for bad in [
        json!({}),
        json!({"address": "aa:bb"}),
        json!({"address": "--device=x"}),
        json!({"address": 5}),
    ] {
        assert_eq!(
            bluetooth::send_args(&bad, lang())
                .unwrap_err()
                .key
                .as_deref(),
            Some("devices.badDevice")
        );
    }
    assert!(
        bluetooth::send_args(&json!({"address": "AA:BB:CC:DD:EE:FF", "name": 3}), lang()).is_err()
    );
    let long = "é".repeat(200);
    assert!(bluetooth::clean_name(&long).len() <= 248);
}

#[test]
fn send_files_starts_bluetooth_sendto_with_the_device() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::env::temp_dir().join(format!("sendto-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let out = dir.join("argv");
    let tool = dir.join("bluetooth-sendto");
    std::fs::write(
        &tool,
        format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", out.display()),
    )
    .unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o700)).unwrap();
    let r = bluetooth::send_files(Some(tool), "AA:BB:CC:DD:EE:FF", "Telefoon", lang()).unwrap();
    assert_eq!(r["ok"], true);
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !out.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        "--device=AA:BB:CC:DD:EE:FF\n--name=Telefoon\n"
    );
    let missing = bluetooth::send_files(Some(dir.join("absent")), "AA:BB:CC:DD:EE:FF", "", lang())
        .unwrap_err();
    assert_eq!(
        (missing.code.as_str(), missing.key.as_deref()),
        ("unsupported", Some("devices.sendFilesMissing"))
    );
    assert!(bluetooth::send_files(None, "AA:BB:CC:DD:EE:FF", "", lang()).is_err());
    let _ = std::fs::remove_dir_all(dir);
}

// ------------------------------------------------------------------------------------------ the D-Bus fakes

fn handler(f: fn(&str, &glib::Variant, &Props) -> fake_bus::Answer) -> Handler {
    Arc::new(f)
}

const BOLT_MANAGER_XML: &str = r#"<node><interface name="org.freedesktop.bolt1.Manager">
  <method name="ListDevices"><arg type="ao" direction="out"/></method>
  <method name="ListDomains"><arg type="ao" direction="out"/></method>
  <method name="DeviceByUid"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
  <method name="EnrollDevice"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
  <method name="ForgetDevice"><arg type="s" direction="in"/></method>
  <property name="Version" type="u" access="read"/><property name="Probing" type="b" access="read"/>
  <property name="DefaultPolicy" type="s" access="read"/><property name="SecurityLevel" type="s" access="read"/>
  <property name="AuthMode" type="s" access="readwrite"/>
</interface></node>"#;

const BOLT_DEVICE_XML: &str = r#"<node><interface name="org.freedesktop.bolt1.Device">
  <method name="Authorize"><arg type="s" direction="in"/></method>
  <property name="Uid" type="s" access="read"/><property name="Name" type="s" access="read"/>
  <property name="Vendor" type="s" access="read"/><property name="Type" type="s" access="read"/>
  <property name="Status" type="s" access="read"/><property name="Stored" type="b" access="read"/>
  <property name="Policy" type="s" access="read"/><property name="Parent" type="s" access="read"/>
  <property name="ConnectTime" type="t" access="read"/>
</interface></node>"#;

const DOCK: &str = "/org/freedesktop/bolt/devices/c6030000_dock";
const HOST: &str = "/org/freedesktop/bolt/devices/host";

fn bolt_fake() -> Option<Fake> {
    let manager = handler(|member, args, _| {
        let path = |p: &str| glib::variant::ObjectPath::try_from(p).unwrap();
        match member {
            "ListDevices" => Ok(Some((vec![path(HOST), path(DOCK)],).to_variant())),
            "ListDomains" => Ok(Some(
                (Vec::<glib::variant::ObjectPath>::new(),).to_variant(),
            )),
            "DeviceByUid" => match args.child_value(0).str() {
                Some("c6030000-dock") => Ok(Some((path(DOCK),).to_variant())),
                _ => Err((
                    "org.freedesktop.bolt.Error.NotFound".into(),
                    "device not found".into(),
                )),
            },
            "EnrollDevice" if args.child_value(0).str() == Some("deny") => Err((
                "org.freedesktop.PolicyKit1.Error.NotAuthorized".into(),
                "not authorized (fake)".into(),
            )),
            "EnrollDevice" => Ok(Some((path(DOCK),).to_variant())),
            "ForgetDevice" => Ok(None),
            _ => Err((
                "org.freedesktop.DBus.Error.UnknownMethod".into(),
                member.into(),
            )),
        }
    });
    let device = handler(|_, _, _| Ok(None));
    let m = "org.freedesktop.bolt1.Manager";
    let d = "org.freedesktop.bolt1.Device";
    Fake::start(
        vec![
            Object {
                path: "/org/freedesktop/bolt".into(),
                xml: BOLT_MANAGER_XML.into(),
                handler: manager,
            },
            Object {
                path: DOCK.into(),
                xml: BOLT_DEVICE_XML.into(),
                handler: device.clone(),
            },
            Object {
                path: HOST.into(),
                xml: BOLT_DEVICE_XML.into(),
                handler: device,
            },
        ],
        vec![
            ("/org/freedesktop/bolt", m, "Version", 1u32.to_variant()),
            ("/org/freedesktop/bolt", m, "Probing", false.to_variant()),
            (
                "/org/freedesktop/bolt",
                m,
                "DefaultPolicy",
                "auto".to_variant(),
            ),
            (
                "/org/freedesktop/bolt",
                m,
                "SecurityLevel",
                "user".to_variant(),
            ),
            (
                "/org/freedesktop/bolt",
                m,
                "AuthMode",
                "enabled".to_variant(),
            ),
            (DOCK, d, "Uid", "c6030000-dock".to_variant()),
            (DOCK, d, "Name", "Thunderbolt Dock".to_variant()),
            (DOCK, d, "Vendor", "Lenovo".to_variant()),
            (DOCK, d, "Type", "peripheral".to_variant()),
            (DOCK, d, "Status", "connected".to_variant()),
            (DOCK, d, "Stored", false.to_variant()),
            (DOCK, d, "Policy", "default".to_variant()),
            (DOCK, d, "Parent", "host-1".to_variant()),
            (DOCK, d, "ConnectTime", 1_700_000_000u64.to_variant()),
            (HOST, d, "Uid", "host-1".to_variant()),
            (HOST, d, "Type", "host".to_variant()),
            (HOST, d, "Status", "authorized".to_variant()),
        ],
    )
}

fn skipped() {
    eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
}

#[test]
fn bolt_is_read_and_written_with_the_password_dialog() {
    let Some(fake) = bolt_fake() else {
        return skipped();
    };
    let bus = session().unwrap();
    let got = thunderbolt::get(&bus, &fake.name);
    assert_eq!(got["available"], true);
    assert_eq!(
        (got["authMode"].clone(), got["securityLevel"].clone()),
        (json!("enabled"), json!("user"))
    );
    let devices = got["devices"].as_array().unwrap();
    assert_eq!(
        devices.len(),
        1,
        "the host is no device of the list: {devices:?}"
    );
    assert_eq!(
        (devices[0]["uid"].clone(), devices[0]["pending"].clone()),
        (json!("c6030000-dock"), json!(true))
    );
    assert!(
        fake.calls.lock().unwrap().iter().all(|c| !c.interactive),
        "reads never ask for a password"
    );

    fake.clear();
    thunderbolt::set_auth_mode(&bus, &fake.name, "disabled", lang()).unwrap();
    let set = fake.calls_of("Set");
    assert_eq!(set.len(), 1);
    assert!(
        set[0].interactive,
        "Direct access asks for the password: {set:?}"
    );
    assert_eq!(
        (set[0].path.as_str(), set[0].iface.as_str()),
        ("/org/freedesktop/bolt", "org.freedesktop.DBus.Properties")
    );
    assert!(
        set[0].args.contains("'AuthMode'") && set[0].args.contains("'disabled'"),
        "{}",
        set[0].args
    );
    assert_eq!(
        fake.prop(
            "/org/freedesktop/bolt",
            "org.freedesktop.bolt1.Manager",
            "AuthMode"
        )
        .unwrap()
        .str(),
        Some("disabled")
    );

    thunderbolt::enroll(&bus, &fake.name, "c6030000-dock", "default", lang()).unwrap();
    let enroll = fake.calls_of("EnrollDevice");
    assert_eq!(enroll[0].args, "('c6030000-dock', 'default', '')");
    assert!(enroll[0].interactive);
    thunderbolt::authorize(&bus, &fake.name, "c6030000-dock", lang()).unwrap();
    let auth = fake.calls_of("Authorize");
    assert_eq!(
        (
            auth[0].path.as_str(),
            auth[0].args.as_str(),
            auth[0].interactive
        ),
        (DOCK, "('',)", true)
    );
    thunderbolt::forget(&bus, &fake.name, "c6030000-dock", lang()).unwrap();
    assert!(fake.calls_of("ForgetDevice")[0].interactive);

    let denied = thunderbolt::enroll(&bus, &fake.name, "deny", "default", lang()).unwrap_err();
    assert_eq!(
        (denied.code.as_str(), denied.key.as_deref()),
        ("denied", Some("core.notAuthorized"))
    );
    let gone = thunderbolt::authorize(&bus, &fake.name, "unknown", lang()).unwrap_err();
    assert_eq!(gone.key.as_deref(), Some("devices.thunderboltNotFound"));
    // no bolt at all
    assert_eq!(
        thunderbolt::get(&bus, "org.example.NoBolt")["available"],
        false
    );
}

const UPOWER_XML: &str = r#"<node><interface name="org.freedesktop.UPower">
  <method name="EnumerateDevices"><arg type="ao" direction="out"/></method>
</interface></node>"#;
const UPOWER_DEVICE_XML: &str = r#"<node><interface name="org.freedesktop.UPower.Device">
  <property name="Type" type="u" access="read"/><property name="Model" type="s" access="read"/>
  <property name="Percentage" type="d" access="read"/><property name="PowerSupply" type="b" access="read"/>
  <property name="IsPresent" type="b" access="read"/><property name="NativePath" type="s" access="read"/>
  <property name="State" type="u" access="read"/>
</interface></node>"#;

#[test]
fn batteries_come_from_upower() {
    let enumerate = handler(|_, _, _| {
        let p = |s: &str| glib::variant::ObjectPath::try_from(s).unwrap();
        Ok(Some(
            (vec![
                p("/org/freedesktop/UPower/devices/battery_BAT0"),
                p("/org/freedesktop/UPower/devices/headset_dev_AA"),
            ],)
                .to_variant(),
        ))
    });
    let device = handler(|_, _, _| Ok(None));
    let i = "org.freedesktop.UPower.Device";
    let (bat, head) = (
        "/org/freedesktop/UPower/devices/battery_BAT0",
        "/org/freedesktop/UPower/devices/headset_dev_AA",
    );
    let Some(fake) = Fake::start(
        vec![
            Object {
                path: "/org/freedesktop/UPower".into(),
                xml: UPOWER_XML.into(),
                handler: enumerate,
            },
            Object {
                path: bat.into(),
                xml: UPOWER_DEVICE_XML.into(),
                handler: device.clone(),
            },
            Object {
                path: head.into(),
                xml: UPOWER_DEVICE_XML.into(),
                handler: device,
            },
        ],
        vec![
            (bat, i, "Type", 2u32.to_variant()),
            (bat, i, "PowerSupply", true.to_variant()),
            (bat, i, "Percentage", 80.0f64.to_variant()),
            (head, i, "Type", 17u32.to_variant()),
            (head, i, "Model", "WH-1000XM4".to_variant()),
            (head, i, "Percentage", 40.0f64.to_variant()),
            (head, i, "IsPresent", true.to_variant()),
            (
                head,
                i,
                "NativePath",
                "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF".to_variant(),
            ),
            (head, i, "State", 2u32.to_variant()),
        ],
    ) else {
        return skipped();
    };
    let bus = session().unwrap();
    let got = batteries::get(&bus, &fake.name);
    let devices = got["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1, "{devices:?}");
    assert_eq!(devices[0]["address"], "AA:BB:CC:DD:EE:FF");
    assert_eq!(devices[0]["percentage"], 40);
    assert_eq!(
        batteries::get(&bus, "org.example.NoUPower")["available"],
        false
    );
}

const OBJECT_MANAGER_XML: &str = r#"<node><interface name="org.freedesktop.DBus.ObjectManager">
  <method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>
</interface></node>"#;
const ADAPTER_XML: &str = r#"<node><interface name="org.bluez.Adapter1">
  <property name="Address" type="s" access="read"/><property name="Alias" type="s" access="readwrite"/>
  <property name="Powered" type="b" access="readwrite"/><property name="Discoverable" type="b" access="readwrite"/>
</interface></node>"#;
const HCI0: &str = "/org/bluez/hci0";

/// BlueZ's GetManagedObjects built from the fake's Adapter1 properties.
fn managed_objects(props: &Props) -> glib::Variant {
    let props = props.lock().unwrap();
    let adapter: std::collections::HashMap<String, glib::Variant> = props
        .iter()
        .filter(|((path, iface, _), _)| path == HCI0 && iface == "org.bluez.Adapter1")
        // a Variant value becomes the `v` of a{sv} by itself
        .map(|((_, _, name), v)| (name.clone(), v.clone()))
        .collect();
    let ifaces = std::collections::HashMap::from([("org.bluez.Adapter1".to_owned(), adapter)]);
    let objects = std::collections::HashMap::from([(
        glib::variant::ObjectPath::try_from(HCI0).unwrap(),
        ifaces,
    )]);
    (objects,).to_variant()
}

fn bluez_fake(discoverable: bool) -> Option<Fake> {
    let om = handler(|_, _, props| Ok(Some(managed_objects(props))));
    let a = "org.bluez.Adapter1";
    Fake::start(
        vec![
            Object {
                path: "/".into(),
                xml: OBJECT_MANAGER_XML.into(),
                handler: om,
            },
            Object {
                path: HCI0.into(),
                xml: ADAPTER_XML.into(),
                handler: handler(|_, _, _| Ok(None)),
            },
        ],
        vec![
            (HCI0, a, "Address", "AA:BB:CC:DD:EE:FF".to_variant()),
            (HCI0, a, "Alias", "werkplek".to_variant()),
            (HCI0, a, "Powered", true.to_variant()),
            (HCI0, a, "Discoverable", discoverable.to_variant()),
        ],
    )
}

#[test]
fn discoverable_is_a_lease_that_undoes_only_itself() {
    let Some(fake) = bluez_fake(false) else {
        return skipped();
    };
    let bus = session().unwrap();
    let mut lease = bluetooth::Lease::default();
    let on =
        bluetooth::set_discoverable(&bus, &fake.name, &mut lease, true, bluetooth::LEASE, lang())
            .unwrap();
    assert_eq!(
        (on["alias"].clone(), on["discoverable"].clone()),
        (json!("werkplek"), json!(true))
    );
    assert_eq!(fake.calls_of("Set").len(), 1);
    assert!(lease.active() && !lease.expired());
    // the page's renewal: no second write
    bluetooth::set_discoverable(&bus, &fake.name, &mut lease, true, bluetooth::LEASE, lang())
        .unwrap();
    assert_eq!(fake.calls_of("Set").len(), 1);
    let off = bluetooth::set_discoverable(
        &bus,
        &fake.name,
        &mut lease,
        false,
        bluetooth::LEASE,
        lang(),
    )
    .unwrap();
    assert_eq!(off["discoverable"], false);
    let set = fake.calls_of("Set");
    assert_eq!(set.len(), 2);
    assert!(
        set[1].args.contains("'Discoverable'") && set[1].args.contains("false"),
        "{}",
        set[1].args
    );
    assert!(!lease.active());
    drop(fake);

    // discoverable already (bluetoothctl): the lease leaves it so
    let Some(fake) = bluez_fake(true) else {
        return skipped();
    };
    let mut lease = bluetooth::Lease::default();
    bluetooth::set_discoverable(&bus, &fake.name, &mut lease, true, bluetooth::LEASE, lang())
        .unwrap();
    bluetooth::end_lease(&bus, &fake.name, &mut lease);
    assert!(
        fake.calls_of("Set").is_empty(),
        "{:?}",
        fake.calls_of("Set")
    );
    // no BlueZ: unavailable
    let none = bluetooth::set_discoverable(
        &bus,
        "org.example.NoBluez",
        &mut lease,
        true,
        bluetooth::LEASE,
        lang(),
    )
    .unwrap_err();
    assert_eq!(none.key.as_deref(), Some("devices.noBluetooth"));
}

/// A connection of its own on the private bus, to send signals as a daemon would (its unique name is the sender).
fn emitter() -> Option<gio::DBusConnection> {
    let address = super::fake_bus::private_bus_address()?;
    gio::DBusConnection::for_address_sync(
        &address,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .ok()
}

/// Every `devices.changed` the service emits, in order.
fn changes(ctx: &Arc<ServiceContext>) -> Arc<Mutex<Vec<Value>>> {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = seen.clone();
    ctx.events.on(
        "devices.changed",
        Arc::new(move |d: &Value| sink.lock().unwrap().push(d.clone())),
    );
    seen
}

fn wait_for(what: &str, timeout: Duration, mut done: impl FnMut() -> bool) {
    let end = Instant::now() + timeout;
    while !done() {
        assert!(Instant::now() < end, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn the_watcher_sleeps_and_emits_a_burst_of_signals_once() {
    // review B finding 2: no tick while nothing happens; the first signal of a quiet kind arms one emit
    let (Some(fake), Some(daemon)) = (bluez_fake(false), emitter()) else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    let seen = changes(&lc.ctx);
    let mut ends = Endpoints::session(&fake.name);
    ends.upower = daemon.unique_name().unwrap().to_string();
    let devices = Devices::with(lc.ctx.clone(), ends);
    devices.watch();
    std::thread::sleep(Duration::from_millis(300));
    let signal = |path: &str| {
        daemon
            .emit_signal(
                None,
                path,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                Some(
                    &(
                        "org.freedesktop.UPower.Device",
                        std::collections::HashMap::<String, glib::Variant>::new(),
                        Vec::<String>::new(),
                    )
                        .to_variant(),
                ),
            )
            .unwrap();
    };
    for i in 0..5 {
        signal(&format!("/org/freedesktop/UPower/devices/mouse_{i}"));
    }
    daemon.flush_sync(gio::Cancellable::NONE).unwrap();
    wait_for("the burst's event", Duration::from_secs(5), || {
        !seen.lock().unwrap().is_empty()
    });
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(
        *seen.lock().unwrap(),
        [json!({"what": "batteries"})],
        "five signals, one event"
    );
    // quiet again: nothing more comes by itself; the next signal is the next event
    signal("/org/freedesktop/UPower/devices/mouse_0");
    daemon.flush_sync(gio::Cancellable::NONE).unwrap();
    wait_for("the second event", Duration::from_secs(5), || {
        seen.lock().unwrap().len() == 2
    });
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(seen.lock().unwrap().len(), 2);
    devices.shutdown();
}

#[test]
fn a_lease_without_a_renewal_ends_by_itself() {
    // review B finding 2: a one-shot source at the lease's end instead of a 250 ms tick, pushed on by a renewal
    let Some(fake) = bluez_fake(false) else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    let seen = changes(&lc.ctx);
    let lease = Duration::from_millis(600);
    let devices = Devices::with_lease(lc.ctx.clone(), Endpoints::session(&fake.name), lease);
    devices.watch();
    let on = json!({"on": true});
    devices
        .call("devices.bluetooth.setDiscoverable", &on)
        .unwrap()
        .unwrap();
    assert_eq!(fake.calls_of("Set").len(), 1);
    std::thread::sleep(Duration::from_millis(300));
    let renewed = Instant::now();
    devices
        .call("devices.bluetooth.setDiscoverable", &on)
        .unwrap()
        .unwrap();
    // past the first end (600 ms after the start), before the renewed one
    std::thread::sleep(Duration::from_millis(450));
    assert_eq!(
        fake.calls_of("Set").len(),
        1,
        "the renewal pushed the end on"
    );
    wait_for("the lease's end", Duration::from_secs(5), || {
        fake.calls_of("Set").len() == 2
    });
    assert!(
        renewed.elapsed() >= lease - Duration::from_millis(50),
        "{:?}",
        renewed.elapsed()
    );
    let off = &fake.calls_of("Set")[1];
    assert!(
        off.args.contains("'Discoverable'") && off.args.contains("false"),
        "{}",
        off.args
    );
    wait_for("the bluetooth event", Duration::from_secs(5), || {
        seen.lock()
            .unwrap()
            .iter()
            .filter(|d| d["what"] == "bluetooth")
            .count()
            >= 3
    });
    assert!(!devices.lease.lock().unwrap().lease.active());
    // shutdown with a lease: ended at once
    devices
        .call("devices.bluetooth.setDiscoverable", &on)
        .unwrap()
        .unwrap();
    assert_eq!(fake.calls_of("Set").len(), 3);
    devices.shutdown();
    assert_eq!(fake.calls_of("Set").len(), 4, "shutdown ends the lease");
}

#[test]
fn a_dry_run_writes_nothing_and_bad_arguments_are_refused_first() {
    let Some(fake) = bolt_fake() else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    // LangContext's context is not a dry run: build one that is, on the same config
    let events = crate::events::Events::new(&lc.root.join("events-dry")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        lc.root.join("config"),
        lc.root.join("cache"),
        lc.root.clone(),
        "settings".into(),
        true,
    )
    .unwrap();
    let devices = Devices::with(ctx, Endpoints::session(&fake.name));
    fake.clear();
    for (method, args) in [
        (
            "devices.thunderbolt.setAuthMode",
            json!({"mode": "disabled"}),
        ),
        (
            "devices.thunderbolt.enroll",
            json!({"uid": "c6030000-dock"}),
        ),
        (
            "devices.thunderbolt.authorize",
            json!({"uid": "c6030000-dock"}),
        ),
        (
            "devices.thunderbolt.forget",
            json!({"uid": "c6030000-dock"}),
        ),
        ("devices.bluetooth.setDiscoverable", json!({"on": true})),
    ] {
        let r = devices.call(method, &args).unwrap().unwrap();
        assert_eq!(r["dryRun"], true, "{method}");
    }
    assert!(
        fake.calls.lock().unwrap().is_empty(),
        "a dry run calls no daemon: {:?}",
        fake.calls.lock().unwrap()
    );
    let bad = devices
        .call("devices.thunderbolt.enroll", &json!({"uid": "a b"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(bad.key.as_deref(), Some("devices.badDevice"));
    let bad = devices
        .call("devices.bluetooth.setDiscoverable", &json!({"on": "ja"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(bad.key.as_deref(), Some("devices.badArgs"));
    assert!(
        devices.call("devices.unknown", &json!({})).is_none(),
        "a method of no table is unknown"
    );
    // the reads run in a dry run too
    let got = devices
        .call("devices.thunderbolt.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(got["authMode"], "enabled");
    events.shutdown();
}

#[test]
fn every_error_key_is_a_row_of_devices_tsv() {
    for key in [
        "devices.badArgs",
        "devices.badDevice",
        "devices.noBluetooth",
        "devices.discoverableFailed",
        "devices.sendFilesMissing",
        "devices.sendFilesFailed",
        "devices.thunderboltUnavailable",
        "devices.thunderboltNotFound",
    ] {
        for lang in [Lang::Nl, Lang::En] {
            let text = crate::i18n::tr(lang, key);
            assert!(!text.is_empty() && text != key, "{key} ({lang:?})");
        }
    }
}

// -------------------------------------------------------------------------------------------------- colour

#[test]
fn colord_objects_read_as_the_colour_panel_shows_them() {
    let profile = color::profile_json(
        "/org/freedesktop/ColorManager/profiles/p1",
        &json!({"ProfileId": "icc-0a1b", "Title": "Dell U2720Q (calibrated)", "Kind": "display-device",
            "Colorspace": "rgb", "Filename": "/home/u/.local/share/icc/dell.icc", "Created": 1763280000i64,
            "IsSystemWide": false, "Warnings": ["vcgt-non-monotonic"], "HasVcgt": true,
            "Metadata": {"DATA_source": "calib"}}),
    );
    assert_eq!(profile["title"], "Dell U2720Q (calibrated)");
    assert_eq!(profile["source"], "calib");
    assert_eq!(profile["warnings"], json!(["vcgt-non-monotonic"]));
    assert_eq!(profile["created"], 1763280000);
    let mut profiles = std::collections::BTreeMap::new();
    profiles.insert(
        "/org/freedesktop/ColorManager/profiles/p1".to_owned(),
        profile,
    );
    let device = color::device_json(
        "/org/freedesktop/ColorManager/devices/xrandr_dell",
        &json!({"DeviceId": "xrandr-Dell-U2720Q", "Kind": "display", "Model": "U2720Q", "Vendor": "Dell",
            "Colorspace": "rgb", "Enabled": true, "Embedded": false,
            "Profiles": ["/org/freedesktop/ColorManager/profiles/p1", "/org/freedesktop/ColorManager/profiles/p2"],
            "ProfilingInhibitors": []}),
        &profiles,
    );
    assert_eq!(
        device["default"], "/org/freedesktop/ColorManager/profiles/p1",
        "colord's first is the default"
    );
    assert_eq!(device["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(
        device["profiles"][1]["title"], "",
        "a profile colord did not list keeps its path"
    );
    assert_eq!(device["inhibited"], false);
}

/// A minimal ICC profile: a 132-byte header with its size and the "acsp" signature.
fn icc(tag: u8) -> Vec<u8> {
    let mut bytes = vec![tag; 132];
    bytes[..4].copy_from_slice(&132u32.to_be_bytes());
    bytes[36..40].copy_from_slice(b"acsp");
    bytes
}

#[test]
fn an_imported_profile_must_be_icc_and_keeps_its_name() {
    assert!(color::is_icc(&icc(0)));
    let mut wrong = icc(0);
    wrong[36..40].copy_from_slice(b"xxxx");
    assert!(!color::is_icc(&wrong) && !color::is_icc(b"short"));
    let mut long = icc(0);
    long[..4].copy_from_slice(&4096u32.to_be_bytes());
    assert!(!color::is_icc(&long), "a header bigger than the file");
    let dir = std::env::temp_dir().join(format!("win11-icc-t-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let source = Path::new("/somewhere/Dell U2720Q.icc");
    assert_eq!(
        color::import_target(&dir, source, &icc(1)),
        dir.join("Dell U2720Q.icc")
    );
    std::fs::write(dir.join("Dell U2720Q.icc"), icc(1)).unwrap();
    assert_eq!(
        color::import_target(&dir, source, &icc(1)),
        dir.join("Dell U2720Q.icc"),
        "the same file again"
    );
    assert_eq!(
        color::import_target(&dir, source, &icc(2)),
        dir.join("Dell U2720Q-2.icc"),
        "another file of that name"
    );
    assert_eq!(
        color::import_target(&dir, Path::new("/x/.hidden"), &icc(3)),
        dir.join("profile.icc")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn calibration_takes_gnomes_choices_only() {
    let ok = json!({"device": "/org/freedesktop/ColorManager/devices/d", "quality": 1, "kind": 12,
        "whitepoint": 6500, "title": "Mijn scherm"});
    let (device, sensor, (q, k, w, t)) = Devices::calibrate_args(&ok, lang()).unwrap();
    assert_eq!(
        (device.as_str(), sensor, q, k, w, t.as_str()),
        (
            "/org/freedesktop/ColorManager/devices/d",
            None,
            1,
            12,
            6500,
            "Mijn scherm"
        )
    );
    for (key, bad) in [
        ("quality", json!(3)),
        ("kind", json!(3)),
        ("whitepoint", json!(6000)),
        ("title", json!("")),
        ("title", json!("a\nb")),
        ("device", json!("/org/other")),
        ("sensor", json!("x")),
    ] {
        let mut args = ok.clone();
        args[key] = bad.clone();
        assert!(
            Devices::calibrate_args(&args, lang()).is_err(),
            "{key} = {bad}"
        );
    }
    let sample =
        color::calibrate_event("UpdateSample", &(0.5f64, 0.25f64, 1.0f64).to_variant()).unwrap();
    assert_eq!(
        sample,
        json!({"state": "sample", "sample": {"red": 0.5, "green": 0.25, "blue": 1.0}})
    );
    let ask = color::calibrate_event(
        "InteractionRequired",
        &(1u32, "Plaats de sensor", "/i.png").to_variant(),
    )
    .unwrap();
    assert_eq!(
        (ask["state"].clone(), ask["code"].clone()),
        (json!("interaction"), json!(1))
    );
    let dict = glib::VariantDict::new(None);
    dict.insert_value("ProfileId", &"icc-new".to_variant());
    let done = color::calibrate_event(
        "Finished",
        &glib::Variant::tuple_from_iter([0u32.to_variant(), dict.end()]),
    )
    .unwrap();
    assert_eq!(
        (done["ok"].clone(), done["profile"].clone()),
        (json!(true), json!("icc-new"))
    );
    assert!(color::calibrate_event("Other", &().to_variant()).is_none());
}

#[test]
fn a_gamma_ramp_is_spread_over_the_lut_as_gnome_does() {
    // cc-color-calibrate.c set_output_gamma: point i mixes ramp[floor(m)] and ramp[ceil(m)], m = (n-1)/(size-1)*i
    let linear = [(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)];
    let (r, g, b) = color::gamma_lut(&linear, 5);
    assert_eq!(r, [0, 16383, 32767, 49151, 65535]);
    assert_eq!((g.clone(), b.clone()), (r.clone(), r));
    // more points than the LUT: the ends are kept, the middle is mixed
    let ramp: Vec<(f64, f64, f64)> = (0..=10).map(|i| (i as f64 / 10.0, 0.5, 2.0)).collect();
    let (r, g, b) = color::gamma_lut(&ramp, 3);
    assert_eq!(r, [0, 32767, 65535]);
    assert_eq!(g, [32767; 3]);
    assert_eq!(b, [65535; 3], "clamped to 1.0");
    // a one-point LUT and an empty ramp do not divide by zero
    assert_eq!(color::gamma_lut(&linear, 1).0, [0]);
    assert!(color::gamma_lut(&[], 4).0.is_empty());
    let none: Vec<(f64, f64, f64)> = Vec::new();
    assert!(color::gamma_ramp(&(none,).to_variant()).is_none());
    assert_eq!(
        color::gamma_ramp(&(vec![(0.5f64, 0.25f64, 1.0f64)],).to_variant()),
        Some(vec![(0.5, 0.25, 1.0)])
    );
}

#[test]
fn no_mutter_colour_manager_is_no_calibration() {
    // Hyprland, or a display colord got from somewhere else (no XRANDR_name): GNOME's setup error, nothing started
    let Some(fake) = colord_fake() else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    let mut ends = Endpoints::session(&fake.name);
    ends.mutter = "org.example.NoMutter".into();
    let devices = Devices::with(lc.ctx.clone(), ends);
    fake.clear();
    let err = devices
        .call(
            "devices.color.calibrate.start",
            &json!({"device": CD_DEV, "quality": 0, "kind": 1, "whitepoint": 0, "title": "x"}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("devices.noCalibration"));
    assert!(
        fake.calls_of("Start").is_empty(),
        "colord-session never started"
    );
    assert!(devices.gamma.lock().unwrap().is_none());
}

const CD_MANAGER_XML: &str = r#"<node><interface name="org.freedesktop.ColorManager">
  <method name="GetDevices"><arg type="ao" direction="out"/></method>
  <method name="GetProfiles"><arg type="ao" direction="out"/></method>
  <method name="GetSensors"><arg type="ao" direction="out"/></method>
  <method name="FindProfileByFilename"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
  <method name="CreateProfileWithFd"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="h" direction="in"/><arg type="a{ss}" direction="in"/><arg type="o" direction="out"/></method>
</interface></node>"#;
const CD_DEVICE_XML: &str = r#"<node><interface name="org.freedesktop.ColorManager.Device">
  <method name="SetEnabled"><arg type="b" direction="in"/></method>
  <method name="MakeProfileDefault"><arg type="o" direction="in"/></method>
  <method name="AddProfile"><arg type="s" direction="in"/><arg type="o" direction="in"/></method>
  <method name="RemoveProfile"><arg type="o" direction="in"/></method>
  <property name="DeviceId" type="s" access="read"/><property name="Kind" type="s" access="read"/>
  <property name="Model" type="s" access="read"/><property name="Vendor" type="s" access="read"/>
  <property name="Colorspace" type="s" access="read"/><property name="Enabled" type="b" access="read"/>
  <property name="Embedded" type="b" access="read"/><property name="Profiles" type="ao" access="read"/>
  <property name="Metadata" type="a{ss}" access="read"/>
</interface></node>"#;
const CD_PROFILE_XML: &str = r#"<node><interface name="org.freedesktop.ColorManager.Profile">
  <method name="InstallSystemWide"/>
  <property name="ProfileId" type="s" access="read"/><property name="Title" type="s" access="read"/>
  <property name="Kind" type="s" access="read"/><property name="Colorspace" type="s" access="read"/>
  <property name="IsSystemWide" type="b" access="read"/><property name="Warnings" type="as" access="read"/>
</interface></node>"#;
const CD_SENSOR_XML: &str = r#"<node><interface name="org.freedesktop.ColorManager.Sensor">
  <property name="SensorId" type="s" access="read"/><property name="Kind" type="s" access="read"/>
  <property name="Model" type="s" access="read"/>
</interface></node>"#;
const CD_HELPER_XML: &str = r#"<node><interface name="org.freedesktop.ColorHelper.Display">
  <method name="Start"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/></method>
  <method name="Cancel"/><method name="Resume"/>
</interface></node>"#;

const MUTTER_XML: &str = r#"<node><interface name="org.gnome.Mutter.ColorManager">
  <method name="CalibrateMonitor"><arg type="s" direction="in"/><arg type="o" direction="out"/></method>
</interface></node>"#;
const MUTTER_SESSION_XML: &str = r#"<node><interface name="org.gnome.Mutter.ColorManager.Calibration">
  <method name="SetCrtcGammaLut"><arg type="aq" direction="in"/><arg type="aq" direction="in"/><arg type="aq" direction="in"/></method>
  <method name="Stop"/>
  <property name="GammaLutSize" type="u" access="read"/>
</interface></node>"#;
const MUTTER_SESSION: &str = "/org/gnome/Mutter/ColorManager/Calibration/u1";

const CD_DEV: &str = "/org/freedesktop/ColorManager/devices/xrandr_dell";
const CD_P1: &str = "/org/freedesktop/ColorManager/profiles/p1";
const CD_P2: &str = "/org/freedesktop/ColorManager/profiles/p2";
const CD_SENSOR: &str = "/org/freedesktop/ColorManager/sensors/huey";

fn colord_fake() -> Option<Fake> {
    let op = |p: &str| glib::variant::ObjectPath::try_from(p).unwrap();
    let manager = handler(|member, _, _| {
        let op = |p: &str| glib::variant::ObjectPath::try_from(p).unwrap();
        match member {
            "GetDevices" => Ok(Some((vec![op(CD_DEV)],).to_variant())),
            "GetProfiles" => Ok(Some((vec![op(CD_P1), op(CD_P2)],).to_variant())),
            "GetSensors" => Ok(Some((vec![op(CD_SENSOR)],).to_variant())),
            "FindProfileByFilename" => Err((
                "org.freedesktop.ColorManager.NotFound".into(),
                "not found".into(),
            )),
            "CreateProfileWithFd" => Ok(Some(
                (op("/org/freedesktop/ColorManager/profiles/new"),).to_variant(),
            )),
            _ => Err((
                "org.freedesktop.DBus.Error.UnknownMethod".into(),
                member.into(),
            )),
        }
    });
    let quiet = handler(|_, _, _| Ok(None));
    let (dv, pf, sn) = (
        "org.freedesktop.ColorManager.Device",
        "org.freedesktop.ColorManager.Profile",
        "org.freedesktop.ColorManager.Sensor",
    );
    let object = |path: &str, xml: &str, h: &Handler| Object {
        path: path.into(),
        xml: xml.into(),
        handler: h.clone(),
    };
    Fake::start(
        vec![
            object(color::MANAGER_PATH, CD_MANAGER_XML, &manager),
            object(CD_DEV, CD_DEVICE_XML, &quiet),
            object(CD_P1, CD_PROFILE_XML, &quiet),
            object(CD_P2, CD_PROFILE_XML, &quiet),
            object(CD_SENSOR, CD_SENSOR_XML, &quiet),
            object(color::HELPER_PATH, CD_HELPER_XML, &quiet),
            object(
                color::MUTTER_PATH,
                MUTTER_XML,
                &handler(|_, _, _| {
                    let op = glib::variant::ObjectPath::try_from(MUTTER_SESSION).unwrap();
                    Ok(Some((op,).to_variant()))
                }),
            ),
            object(MUTTER_SESSION, MUTTER_SESSION_XML, &quiet),
        ],
        vec![
            (CD_DEV, dv, "DeviceId", "xrandr-Dell-U2720Q".to_variant()),
            (CD_DEV, dv, "Kind", "display".to_variant()),
            (CD_DEV, dv, "Model", "U2720Q".to_variant()),
            (CD_DEV, dv, "Vendor", "Dell".to_variant()),
            (CD_DEV, dv, "Colorspace", "rgb".to_variant()),
            (CD_DEV, dv, "Enabled", true.to_variant()),
            (CD_DEV, dv, "Embedded", false.to_variant()),
            (CD_DEV, dv, "Profiles", vec![op(CD_P1)].to_variant()),
            (
                CD_DEV,
                dv,
                "Metadata",
                std::collections::HashMap::from([("XRANDR_name".to_owned(), "DP-1".to_owned())])
                    .to_variant(),
            ),
            (
                MUTTER_SESSION,
                color::MUTTER_CALIBRATION,
                "GammaLutSize",
                4u32.to_variant(),
            ),
            (CD_P1, pf, "ProfileId", "icc-1".to_variant()),
            (CD_P1, pf, "Title", "Dell (EDID)".to_variant()),
            (CD_P1, pf, "Kind", "display-device".to_variant()),
            (CD_P1, pf, "Colorspace", "rgb".to_variant()),
            (CD_P1, pf, "IsSystemWide", false.to_variant()),
            (CD_P1, pf, "Warnings", Vec::<String>::new().to_variant()),
            (CD_P2, pf, "ProfileId", "icc-2".to_variant()),
            (CD_P2, pf, "Title", "sRGB".to_variant()),
            (CD_P2, pf, "Kind", "colorspace-conversion".to_variant()),
            (CD_P2, pf, "Colorspace", "rgb".to_variant()),
            (CD_P2, pf, "IsSystemWide", true.to_variant()),
            (CD_P2, pf, "Warnings", Vec::<String>::new().to_variant()),
            (CD_SENSOR, sn, "SensorId", "huey-1".to_variant()),
            (CD_SENSOR, sn, "Kind", "huey".to_variant()),
            (CD_SENSOR, sn, "Model", "Huey".to_variant()),
        ],
    )
}

#[test]
fn colour_writes_go_to_colord_with_the_password_dialog() {
    let Some(fake) = colord_fake() else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    let ends = Endpoints::session(&fake.name);
    let icc_dir = ends.icc_dir.clone();
    let devices = Devices::with(lc.ctx.clone(), ends);
    let call = |m: &str, a: Value| devices.call(m, &a).unwrap();
    let got = call("devices.color.get", json!({})).unwrap();
    assert_eq!(got["available"], true);
    assert_eq!(got["devices"][0]["id"], "xrandr-Dell-U2720Q");
    assert_eq!(got["devices"][0]["default"], CD_P1);
    assert_eq!(got["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(got["sensors"][0]["id"], "huey-1");
    assert!(
        fake.calls.lock().unwrap().iter().all(|c| !c.interactive),
        "reads never ask"
    );

    fake.clear();
    for (method, args, path, member, expect) in [
        (
            "devices.color.setEnabled",
            json!({"device": CD_DEV, "on": false}),
            CD_DEV,
            "SetEnabled",
            "(false,)".to_owned(),
        ),
        (
            "devices.color.makeDefault",
            json!({"device": CD_DEV, "profile": CD_P2}),
            CD_DEV,
            "MakeProfileDefault",
            format!("(objectpath '{CD_P2}',)"),
        ),
        (
            "devices.color.addProfile",
            json!({"device": CD_DEV, "profile": CD_P2}),
            CD_DEV,
            "AddProfile",
            format!("('hard', objectpath '{CD_P2}')"),
        ),
        (
            "devices.color.removeProfile",
            json!({"device": CD_DEV, "profile": CD_P2}),
            CD_DEV,
            "RemoveProfile",
            format!("(objectpath '{CD_P2}',)"),
        ),
    ] {
        call(method, args).unwrap_or_else(|e| panic!("{method}: {e:?}"));
        let last = fake.calls.lock().unwrap().last().cloned().unwrap();
        assert_eq!(
            (
                last.path.as_str(),
                last.member.as_str(),
                last.args.as_str(),
                last.interactive
            ),
            (path, member, expect.as_str(), true),
            "{method}"
        );
    }
    // "Instellen voor alle gebruikers": installed, then the device's default
    fake.clear();
    call(
        "devices.color.installSystemWide",
        json!({"profile": CD_P1, "device": CD_DEV}),
    )
    .unwrap();
    let members: Vec<(String, String)> = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .map(|c| (c.path.clone(), c.member.clone()))
        .collect();
    assert_eq!(
        members,
        [
            (CD_P1.to_owned(), "InstallSystemWide".to_owned()),
            (CD_DEV.to_owned(), "MakeProfileDefault".to_owned())
        ]
    );

    // "Bestand importeren…": copied to the icc folder, created in colord (nobody registered it), added
    fake.clear();
    let source = lc.root.join("Mijn scherm.icc");
    std::fs::write(&source, icc(7)).unwrap();
    let r = call(
        "devices.color.importProfile",
        json!({"device": CD_DEV, "path": source.to_str().unwrap()}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(icc_dir.join("Mijn scherm.icc")).unwrap(),
        icc(7)
    );
    assert_eq!(r["profile"], "/org/freedesktop/ColorManager/profiles/new");
    let create = &fake.calls_of("CreateProfileWithFd")[0];
    let md5 = glib::compute_checksum_for_data(glib::ChecksumType::Md5, &icc(7))
        .unwrap()
        .to_string();
    assert!(
        create.args.contains(&format!("'icc-{md5}'"))
            && create.args.contains("'temp'")
            && create.args.contains("Mijn scherm.icc"),
        "{}",
        create.args
    );
    assert!(
        fake.calls_of("AddProfile")[0]
            .args
            .contains("/profiles/new")
    );
    let not_icc = lc.root.join("note.txt");
    std::fs::write(&not_icc, "no profile").unwrap();
    let err = call(
        "devices.color.importProfile",
        json!({"device": CD_DEV, "path": not_icc.to_str().unwrap()}),
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("devices.notIcc"));

    // calibration: Mutter's session for the display's connector first, then colord-session's Start with the
    // device's and the sensor's ids and GNOME's options
    fake.clear();
    call(
        "devices.color.calibrate.start",
        json!({"device": CD_DEV, "quality": 2, "kind": 12, "whitepoint": 6500,
        "title": "Dell gekalibreerd"}),
    )
    .unwrap();
    let order: Vec<String> = fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| ["CalibrateMonitor", "Start"].contains(&c.member.as_str()))
        .map(|c| format!("{} {}", c.member, c.args))
        .collect();
    assert_eq!(order[0], "CalibrateMonitor ('DP-1',)");
    assert!(order[1].starts_with("Start "), "{order:?}");
    assert_eq!(
        *devices.gamma.lock().unwrap(),
        Some(color::Gamma {
            path: MUTTER_SESSION.into(),
            size: 4
        })
    );
    // colord-session's UpdateGamma: the ramp at the LUT's 4 points into the session (review B finding 10)
    let ramp: Vec<(f64, f64, f64)> = vec![(0.0, 0.0, 0.0), (0.5, 0.25, 1.0), (1.0, 0.5, 1.0)];
    devices.apply_gamma(&(ramp,).to_variant());
    let lut = &fake.calls_of("SetCrtcGammaLut")[0];
    assert_eq!(lut.path, MUTTER_SESSION);
    assert_eq!(
        lut.args,
        "([uint16 0, 21845, 43690, 65535], [uint16 0, 10922, 21845, 32767], [uint16 0, 43690, 65535, 65535])"
    );
    assert!(!lut.interactive);
    // Finished (or a cancel) stops Mutter's session: the display's own curves come back
    devices.end_gamma();
    assert_eq!(fake.calls_of("Stop")[0].path, MUTTER_SESSION);
    assert!(devices.gamma.lock().unwrap().is_none());
    devices.apply_gamma(&(vec![(0.0f64, 0.0f64, 0.0f64)],).to_variant());
    assert_eq!(
        fake.calls_of("SetCrtcGammaLut").len(),
        1,
        "no session, no write"
    );
    call(
        "devices.color.calibrate.start",
        json!({"device": CD_DEV, "quality": 2, "kind": 12, "whitepoint": 6500,
        "title": "Dell gekalibreerd"}),
    )
    .unwrap();
    let start = &fake.calls_of("Start")[0];
    assert_eq!(start.path, "/");
    assert!(
        start.args.starts_with("('xrandr-Dell-U2720Q', 'huey-1', {"),
        "{}",
        start.args
    );
    for part in [
        "'Quality': <uint32 2>",
        "'Whitepoint': <uint32 6500>",
        "'DeviceKind': <uint32 12>",
        "'Title': <'Dell gekalibreerd'>",
    ] {
        assert!(start.args.contains(part), "{part} in {}", start.args);
    }
    call("devices.color.calibrate.resume", json!({})).unwrap();
    call("devices.color.calibrate.cancel", json!({})).unwrap();
    assert_eq!(
        fake.calls_of("Resume").len() + fake.calls_of("Cancel").len(),
        2
    );
    assert_eq!(
        fake.calls_of("Stop").len(),
        2,
        "the cancel stopped the second session"
    );
    let _ = std::fs::remove_dir_all(icc_dir);

    // no colord at all
    let none = Devices::with(lc.ctx.clone(), Endpoints::session("org.example.NoColord"));
    assert_eq!(
        none.call("devices.color.get", &json!({})).unwrap().unwrap()["available"],
        false
    );
}

#[test]
fn a_colour_dry_run_calls_nothing() {
    let Some(fake) = colord_fake() else {
        return skipped();
    };
    let lc = LangContext::new("en", "settings");
    let events = crate::events::Events::new(&lc.root.join("events-dry")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        lc.root.join("config"),
        lc.root.join("cache"),
        lc.root.clone(),
        "settings".into(),
        true,
    )
    .unwrap();
    let devices = Devices::with(ctx, Endpoints::session(&fake.name));
    fake.clear();
    for (method, args) in [
        (
            "devices.color.setEnabled",
            json!({"device": CD_DEV, "on": true}),
        ),
        (
            "devices.color.makeDefault",
            json!({"device": CD_DEV, "profile": CD_P1}),
        ),
        (
            "devices.color.addProfile",
            json!({"device": CD_DEV, "profile": CD_P1}),
        ),
        (
            "devices.color.removeProfile",
            json!({"device": CD_DEV, "profile": CD_P1}),
        ),
        ("devices.color.installSystemWide", json!({"profile": CD_P1})),
        (
            "devices.color.importProfile",
            json!({"device": CD_DEV, "path": "/tmp/x.icc"}),
        ),
        (
            "devices.color.calibrate.start",
            json!({"device": CD_DEV, "quality": 0, "kind": 1, "whitepoint": 0, "title": "x"}),
        ),
        ("devices.color.calibrate.cancel", json!({})),
        ("devices.color.calibrate.resume", json!({})),
    ] {
        let r = devices.call(method, &args).unwrap().unwrap();
        assert_eq!(r["dryRun"], true, "{method}");
    }
    assert!(
        fake.calls.lock().unwrap().is_empty(),
        "{:?}",
        fake.calls.lock().unwrap()
    );
    let bad = devices
        .call(
            "devices.color.makeDefault",
            &json!({"device": CD_DEV, "profile": "/etc"}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(bad.key.as_deref(), Some("devices.badDevice"));
    events.shutdown();
}

#[test]
fn every_colour_error_key_is_a_row_of_devices_tsv() {
    for key in [
        "devices.noColord",
        "devices.colorNotFound",
        "devices.notIcc",
        "devices.importFailed",
        "devices.noSensor",
        "devices.noCalibration",
    ] {
        for lang in [Lang::Nl, Lang::En] {
            let text = crate::i18n::tr(lang, key);
            assert!(!text.is_empty() && text != key, "{key} ({lang:?})");
        }
    }
}
