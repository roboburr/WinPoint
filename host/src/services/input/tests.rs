//! Batch A's host tests of `input.*` (slice 1: tablets) and of the allow rows the Muis, Touchpad and Pen en
//! tablet pages bind (notes/claude/settings-parity.md §7).
use super::tablets::{self, Device};
use super::*;
use crate::services::gsettings::allow::{self, Mode};
use gio::prelude::*;
use std::path::Path;
use std::sync::Mutex;

fn write(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// One input event device: its udev entry and its /sys ids.
fn device(
    root: &Path,
    minor: u32,
    devname: &str,
    props: &[&str],
    ids: (&str, &str, &str),
    name: &str,
) {
    let mut udev = String::from("E:ID_INPUT=1\n");
    for p in props {
        udev.push_str(&format!("E:{p}=1\n"));
    }
    write(root, &format!("run/udev/data/c13:{minor}"), &udev);
    let sys = format!("sys/dev/char/13:{minor}");
    write(
        root,
        &format!("{sys}/uevent"),
        &format!("MAJOR=13\nMINOR={minor}\nDEVNAME={devname}\n"),
    );
    write(
        root,
        &format!("{sys}/device/id/bustype"),
        &format!("{}\n", ids.0),
    );
    write(
        root,
        &format!("{sys}/device/id/vendor"),
        &format!("{}\n", ids.1),
    );
    write(
        root,
        &format!("{sys}/device/id/product"),
        &format!("{}\n", ids.2),
    );
    write(root, &format!("{sys}/device/name"), &format!("{name}\n"));
}

const INTUOS: &str = "# a comment\n[Device]\nName=Wacom Intuos Pro M\nModelName=PTH-660\n\
DeviceMatch=usb|056a|0357;bluetooth|056a|0360;\nClass=Intuos5\nLayout=wacom-intuos-pro-2-m.svg\nIntegratedIn=\n\
Styli=@intuos4;@intuos5;\n\n[Features]\nStylus=true\nReversible=true\nTouch=true\nNumRings=1\n\n\
[Buttons]\nLeft=A;B;C;D;E;F;G;H;I\nRing=I\nRingNumModes=4\n";
const ELAN: &str = "[Device]\nName=ELAN 2072\nDeviceMatch=usb|04f3|2072\nClass=PenDisplay\n\
IntegratedIn=Display;System\nStyli=@generic-no-eraser;\n[Features]\nStylus=true\nTouch=true\n";
const KAMVAS: &str = "[Device]\nName=Huion Kamvas Pro 24\nDeviceMatch=usb|256c|006e|Tablet Monitor Pen|HUION_M184;\
usb|256c|006e|Tablet Monitor Pad|HUION_M184;\nIntegratedIn=Display\n[Features]\nNumStrips=1\nReversible=false\n\
NumRings=0\nStylus=true\n[Buttons]\nLeft=A;B;C;D;E;F;G;H;I;J\nRight=K;L;M;N;O;P;Q;R;S;T\n";
// libwacom 1.x's counts
const OLD: &str = "[Device]\nName=Old Pad\nDeviceMatch=usb|056a|00b9\n[Features]\nRing=true\nRing2=true\nButtons=8\n";

fn fixture() -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("input-tablets-{}", crate::events::nonce()));
    write(
        &root,
        "usr/share/libwacom/wacom-intuos-pro-2-m.tablet",
        INTUOS,
    );
    write(&root, "usr/share/libwacom/elan-2072.tablet", ELAN);
    write(
        &root,
        "usr/share/libwacom/huion-kamvas-pro-24.tablet",
        KAMVAS,
    );
    write(
        &root,
        "usr/share/libwacom/generic.tablet",
        "[Device]\nName=Generic\nDeviceMatch=generic\n",
    );
    write(&root, "usr/share/libwacom/README", "not a tablet file");
    // an override of the same name wins
    write(
        &root,
        "etc/libwacom/elan-2072.tablet",
        &ELAN.replace("Name=ELAN 2072", "Name=ELAN 2072 (lokaal)"),
    );
    // a mouse first: no tablet
    device(
        &root,
        64,
        "input/event0",
        &["ID_INPUT_MOUSE"],
        ("0003", "046d", "c52b"),
        "Logitech USB Receiver",
    );
    // the Intuos: pen, pad (udev marks it a tablet too), the touch part (a touchpad), its legacy mouse node
    device(
        &root,
        70,
        "input/event6",
        &["ID_INPUT_TABLET"],
        ("0003", "056A", "0357"),
        "Wacom Intuos Pro M Pen",
    );
    device(
        &root,
        71,
        "input/event7",
        &["ID_INPUT_TABLET", "ID_INPUT_TABLET_PAD"],
        ("0003", "056a", "0357"),
        "Wacom Intuos Pro M Pad",
    );
    device(
        &root,
        72,
        "input/event8",
        &["ID_INPUT_TOUCHPAD"],
        ("0003", "056a", "0357"),
        "Wacom Intuos Pro M Finger",
    );
    device(
        &root,
        40,
        "input/mouse3",
        &["ID_INPUT_TABLET"],
        ("0003", "056a", "0357"),
        "Wacom Intuos Pro M Pen",
    );
    // a laptop screen with a pen: the pen and the touchscreen share the id
    device(
        &root,
        80,
        "input/event10",
        &["ID_INPUT_TABLET"],
        ("0018", "04f3", "2072"),
        "ELAN2072:00 04F3:2072 Stylus",
    );
    device(
        &root,
        81,
        "input/event11",
        &["ID_INPUT_TOUCHSCREEN"],
        ("0018", "04f3", "2072"),
        "ELAN2072:00 04F3:2072",
    );
    // a Huion pen display, matched by its kernel name
    device(
        &root,
        82,
        "input/event12",
        &["ID_INPUT_TABLET"],
        ("0003", "256c", "006e"),
        "Tablet Monitor Pen",
    );
    device(
        &root,
        83,
        "input/event13",
        &["ID_INPUT_TABLET", "ID_INPUT_TABLET_PAD"],
        ("0003", "256c", "006e"),
        "Tablet Monitor Pad",
    );
    // a tablet libwacom does not know, an ExpressKey Remote (a pad on its own)
    device(
        &root,
        90,
        "input/event20",
        &["ID_INPUT_TABLET"],
        ("0003", "28bd", "0905"),
        "UGTABLET 10 inch PenTablet Pen",
    );
    device(
        &root,
        91,
        "input/event21",
        &["ID_INPUT_TABLET", "ID_INPUT_TABLET_PAD"],
        ("0003", "056a", "0331"),
        "Wacom ExpressKey Remote Pad",
    );
    // a broken entry: no ids in /sys
    write(
        &root,
        "run/udev/data/c13:95",
        "E:ID_INPUT=1\nE:ID_INPUT_TABLET=1\n",
    );
    write(
        &root,
        "sys/dev/char/13:95/uevent",
        "DEVNAME=input/event31\n",
    );
    root
}

#[test]
fn libwacom_files_give_the_model_its_pad_and_where_it_is_built_in() {
    let w = tablets::parse_wacom(INTUOS).unwrap();
    assert_eq!(w.name, "Wacom Intuos Pro M");
    assert_eq!(
        w.matches,
        vec![
            ("usb".into(), "056a:0357".into(), String::new()),
            ("bluetooth".into(), "056a:0360".into(), String::new())
        ]
    );
    assert_eq!((w.buttons, w.rings, w.strips, w.dials), (9, 1, 0, 0));
    assert_eq!(
        (w.display, w.system, w.reversible),
        (false, false, Some(true))
    );
    let e = tablets::parse_wacom(ELAN).unwrap();
    assert!(e.display && e.system && e.reversible.is_none() && e.buttons == 0);
    let k = tablets::parse_wacom(KAMVAS).unwrap();
    assert_eq!((k.buttons, k.strips, k.reversible), (20, 1, Some(false)));
    assert_eq!(k.matches[0].2, "Tablet Monitor Pen");
    let old = tablets::parse_wacom(OLD).unwrap();
    assert_eq!(
        (old.buttons, old.rings),
        (8, 2),
        "libwacom 1.x's Ring, Ring2 and Buttons"
    );
    assert!(tablets::parse_wacom("[Device]\nName=Generic\nDeviceMatch=generic\n").is_none());
    assert!(tablets::parse_wacom("").is_none());
}

#[test]
fn the_installed_libwacom_data_parses() {
    let root = Path::new("/");
    if !root.join("usr/share/libwacom").is_dir() {
        eprintln!("skipped: no /usr/share/libwacom here");
        return;
    }
    let db = tablets::wacom_db(root);
    assert!(db.len() > 100, "{} models", db.len());
    if let Some(w) = tablets::lookup(&db, "056a:0357", "Wacom Intuos Pro M Pen") {
        assert_eq!((w.buttons, w.rings), (9, 1), "{w:?}");
        assert_eq!(w.reversible, Some(true));
    }
    assert!(
        db.iter()
            .all(|w| w.matches.iter().all(|(_, v, _)| tablets::vidpid_ok(v)))
    );
}

#[test]
fn tablets_come_from_udev_sys_and_libwacom() {
    let root = fixture();
    let devices = tablets::devices(&root);
    assert_eq!(
        devices.iter().filter(|d| d.vidpid == "056a:0357").count(),
        2,
        "the pen and the pad, not the touch part nor the legacy mouse node: {devices:?}"
    );
    assert_eq!(
        devices[0],
        Device {
            vidpid: "056a:0357".into(),
            name: "Wacom Intuos Pro M Pen".into(),
            bus: "0003".into(),
            pen: true,
            pad: false,
            touchscreen: false
        },
        "upper-case hex from /sys is written as Mutter's %.4x"
    );
    let v = tablets::scan(&root);
    let list = v["tablets"].as_array().unwrap();
    let ids: Vec<&str> = list.iter().map(|t| t["vidpid"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [
            "056a:0357",
            "04f3:2072",
            "256c:006e",
            "28bd:0905",
            "056a:0331"
        ]
    );
    let intuos = &list[0];
    assert_eq!(
        *intuos,
        json!({"vidpid": "056a:0357", "name": "Wacom Intuos Pro M", "kind": "tablet", "pen": true, "pad": true,
            "buttons": 9, "rings": 1, "strips": 0, "dials": 0, "reversible": true, "styli": ["default-056a:0357"]})
    );
    assert_eq!(list[1]["kind"], "builtin");
    assert_eq!(
        list[1]["name"], "ELAN 2072 (lokaal)",
        "/etc/libwacom overrides /usr/share"
    );
    assert_eq!(list[1]["pad"], false);
    assert_eq!(
        (list[2]["kind"].as_str(), list[2]["buttons"].as_u64()),
        (Some("display"), Some(20))
    );
    assert_eq!(list[2]["reversible"], false);
    let unknown = &list[3];
    assert_eq!(
        (unknown["name"].as_str(), unknown["kind"].as_str()),
        (Some("UGTABLET 10 inch PenTablet"), Some("tablet"))
    );
    assert_eq!(
        unknown["reversible"],
        Value::Null,
        "unknown: the page shows the row"
    );
    let remote = &list[4];
    assert_eq!(
        (remote["pen"].as_bool(), remote["pad"].as_bool()),
        (Some(false), Some(true))
    );
    assert_eq!(remote["styli"], json!([]));
    assert_eq!(
        v["touchscreens"],
        json!([{"vidpid": "04f3:2072", "name": "ELAN2072:00 04F3:2072"}]),
        "amendment 8: the laptop's touchscreen"
    );
    // every id is one the allow tables' {vidpid} and {serial} holes take
    for t in list {
        assert!(tablets::vidpid_ok(t["vidpid"].as_str().unwrap()));
        for s in t["styli"].as_array().unwrap() {
            assert!(allow::hole_ok("serial", s.as_str().unwrap()), "{s}");
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn no_tablet_means_empty_lists_and_no_libwacom_read() {
    let root = std::env::temp_dir().join(format!("input-none-{}", crate::events::nonce()));
    assert_eq!(
        tablets::scan(&root),
        json!({"tablets": [], "touchscreens": []})
    );
    device(
        &root,
        64,
        "input/event0",
        &["ID_INPUT_MOUSE"],
        ("0003", "046d", "c52b"),
        "Logitech USB Receiver",
    );
    // an unreadable libwacom directory would not matter: nothing asks for it without a tablet
    write(&root, "usr/share/libwacom/broken.tablet", "[Device\n");
    assert_eq!(
        tablets::scan(&root),
        json!({"tablets": [], "touchscreens": []})
    );
    let _ = std::fs::remove_dir_all(root);
}

fn state_reply() -> glib::Variant {
    let ty = glib::VariantTy::new(tablets::STATE_TYPE).unwrap();
    let text = "(uint32 7, [(('eDP-1', 'BOE', '0x0bca', '0x00000000'), \
        [('1920x1080@60', 1920, 1080, 60.0, 1.0, [1.0, 2.0], {'is-current': <true>})], \
        {'display-name': <'Ingebouwd beeldscherm'>, 'is-builtin': <true>}), \
        (('DP-2', 'DEL', 'DELL U2720Q', 'ABC123'), [], {'display-name': <'Dell 27\"'>})], \
        [(0, 0, 1.0, uint32 0, true, [('eDP-1', 'BOE', '0x0bca', '0x00000000')], @a{sv} {})], @a{sv} {})";
    glib::Variant::parse(Some(ty), text).unwrap()
}

#[test]
fn display_config_monitors_give_the_output_key_its_edid_triple() {
    let monitors = tablets::parse_monitors(&state_reply());
    assert_eq!(
        monitors,
        vec![
            json!({"connector": "eDP-1", "vendor": "BOE", "product": "0x0bca", "serial": "0x00000000",
                "name": "Ingebouwd beeldscherm", "builtin": true}),
            json!({"connector": "DP-2", "vendor": "DEL", "product": "DELL U2720Q", "serial": "ABC123",
                "name": "Dell 27\"", "builtin": false}),
        ]
    );
    assert!(
        tablets::parse_monitors(&("x",).to_variant()).is_empty(),
        "another type: nothing"
    );
}

/// A private session bus only: tests/rust-sandbox/run-tests.sh's (never the live one under /run/user).
fn private_bus_address() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
        && !address.contains("/run/user/"))
    .then_some(address)
}

#[test]
fn monitors_come_from_display_config_get_current_state() {
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering, mpsc};
    let Some(address) = private_bus_address() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    const XML: &str = r#"<node><interface name="org.gnome.Mutter.DisplayConfig">
        <method name="GetCurrentState"><arg type="u" direction="out"/>
        <arg type="a((ssss)a(siiddada{sv})a{sv})" direction="out"/>
        <arg type="a(iiduba(ssss)a{sv})" direction="out"/><arg type="a{sv}" direction="out"/></method>
        </interface></node>"#;
    let calls: Arc<Mutex<Vec<(String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, started) = mpsc::channel();
    let (seen, thread_stop) = (calls.clone(), stop.clone());
    let server = std::thread::spawn(move || {
        let context = glib::MainContext::new();
        let _ = context.with_thread_default(|| {
            let connection = gio::DBusConnection::for_address_sync(
                &address,
                gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                    | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                None,
                gio::Cancellable::NONE,
            )
            .unwrap();
            let node = gio::DBusNodeInfo::for_xml(XML).unwrap();
            let iface = node
                .lookup_interface("org.gnome.Mutter.DisplayConfig")
                .unwrap();
            let id = connection
                .register_object(
                    "/org/gnome/Mutter/DisplayConfig",
                    &iface,
                    move |_, _, _, _, method, _, invocation| {
                        let interactive = invocation
                            .message()
                            .flags()
                            .contains(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION);
                        seen.lock().unwrap().push((method.to_owned(), interactive));
                        invocation.return_value(Some(&state_reply()));
                    },
                    |_, _, _, _, _| ().to_variant(),
                    |_, _, _, _, _, _| false,
                )
                .unwrap();
            ready
                .send((
                    connection.unique_name().unwrap().to_string(),
                    context.clone(),
                ))
                .unwrap();
            while !thread_stop.load(Ordering::Acquire) {
                context.iteration(true);
            }
            connection.unregister_object(id).unwrap();
        });
    });
    let (name, context) = started.recv().unwrap();
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    let monitors = tablets::monitors_on(&bus, &name);
    assert_eq!(monitors.len(), 2);
    assert_eq!(monitors[1]["product"], "DELL U2720Q");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![("GetCurrentState".to_owned(), false)],
        "one plain call, no password dialog"
    );
    assert!(
        tablets::monitors_on(&bus, ":1.9999").is_empty(),
        "nobody there: no monitors, no error"
    );
    stop.store(true, Ordering::Release);
    context.wakeup();
    server.join().unwrap();
}

fn service(root: &Path) -> (Input, Arc<crate::events::Events>) {
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        root.join("config"),
        root.join("cache"),
        root.to_path_buf(),
        "settings".into(),
        true,
    )
    .unwrap();
    (
        Input {
            ctx,
            root: root.to_path_buf(),
            gs: gsettings::Config::default(),
        },
        events,
    )
}

#[test]
fn map_buttons_opens_the_pad_osd_of_a_tablet_with_a_pad() {
    let root = fixture();
    let (input, events) = service(&root);
    let calls: Mutex<Vec<(String, Value)>> = Mutex::new(Vec::new());
    let ok = |m: &str, a: &Value| -> Result<ApiResult, String> {
        calls.lock().unwrap().push((m.to_owned(), a.clone()));
        Ok(Ok(json!({"shown": true})))
    };
    let code = |r: ApiResult| r.err().map(|e| (e.code, e.key.unwrap_or_default()));
    for bad in [
        json!({}),
        json!({"vidpid": "056A:0357"}),
        json!({"vidpid": "056a:0357/"}),
        json!({"vidpid": 7}),
    ] {
        assert_eq!(
            code(input.map_buttons(&bad, &ok)),
            Some(("bad_args".into(), "input.badTablet".into())),
            "{bad}"
        );
    }
    assert_eq!(
        code(input.map_buttons(&json!({"vidpid": "1234:5678"}), &ok)),
        Some(("not_found".into(), "input.tabletNotFound".into()))
    );
    assert_eq!(
        code(input.map_buttons(&json!({"vidpid": "04f3:2072"}), &ok)),
        Some(("unsupported".into(), "input.noPad".into())),
        "a laptop pen has no pad"
    );
    assert!(
        calls.lock().unwrap().is_empty(),
        "no refused call reached GNOME Shell"
    );
    assert_eq!(
        input
            .map_buttons(&json!({"vidpid": "056a:0357"}), &ok)
            .unwrap(),
        json!({"shown": true})
    );
    assert_eq!(
        input
            .map_buttons(&json!({"vidpid": "056a:0331"}), &ok)
            .unwrap(),
        json!({"shown": true})
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            ("input.padOsd".to_owned(), json!({"vidpid": "056a:0357"})),
            ("input.padOsd".to_owned(), json!({"vidpid": "056a:0331"}))
        ]
    );
    // no extension on the bus, and the extension's own refusal
    let gone = |_: &str, _: &Value| -> Result<ApiResult, String> {
        Err("GNOME input.padOsd: no such name".into())
    };
    assert_eq!(
        code(input.map_buttons(&json!({"vidpid": "056a:0357"}), &gone)),
        Some(("unsupported".into(), "input.shellUnavailable".into()))
    );
    let refused = |_: &str, _: &Value| -> Result<ApiResult, String> {
        Ok(Err(crate::files::ApiError::new("not_found", "Geen pad")))
    };
    let err = input
        .map_buttons(&json!({"vidpid": "056a:0357"}), &refused)
        .unwrap_err();
    assert_eq!(
        (err.code.as_str(), err.message.as_str()),
        ("not_found", "Geen pad")
    );
    // the refusals read in English too
    let en = crate::files::ApiError::tr(
        "unsupported",
        crate::i18n::Lang::En,
        "input.noPad",
        NO_PARAMS,
    );
    assert!(
        en.message.is_ascii() || en.message.contains('’'),
        "{}",
        en.message
    );
    assert_ne!(
        en.message,
        crate::files::ApiError::tr(
            "unsupported",
            crate::i18n::Lang::Nl,
            "input.noPad",
            NO_PARAMS
        )
        .message
    );
    events.shutdown();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn the_dispatch_answers_the_tablet_methods() {
    let root = fixture();
    let (input, events) = service(&root);
    let v = input.call("input.tablets", &json!({})).unwrap().unwrap();
    assert_eq!(v["tablets"].as_array().map(Vec::len), Some(5));
    assert!(v["monitors"].is_array());
    assert!(
        input.call("input.layouts.nope", &json!({})).is_none(),
        "unknown: the runtime says unknown_method"
    );
    for (name, dangerous) in METHODS {
        assert!(name.starts_with("input."));
        // the tablet methods write nothing; the keyboard and shortcut writes are dangerous (slice 2)
        if name.starts_with("input.tablet") {
            assert!(!dangerous, "{name}");
        }
    }
    events.shutdown();
    let _ = std::fs::remove_dir_all(root);
}

/// The keys the pages Muis, Touchpad and Pen en tablet bind (native/apps/settings/src/pages/{mouse,touchpad,pen}.rs),
/// with the path of a relocatable schema.
const BOUND: &[(&str, Option<&str>, &[&str])] = &[
    (
        "org.gnome.desktop.peripherals.mouse",
        None,
        &[
            "left-handed",
            "speed",
            "accel-profile",
            "natural-scroll",
            "middle-click-emulation",
            "double-click",
        ],
    ),
    (
        "org.gnome.desktop.peripherals.pointingstick",
        None,
        &["speed", "accel-profile"],
    ),
    (
        "org.gnome.desktop.interface",
        None,
        &["locate-pointer", "gtk-enable-primary-paste"],
    ),
    (
        "org.gnome.desktop.peripherals.touchpad",
        None,
        &[
            "send-events",
            "speed",
            "disable-while-typing",
            "tap-to-click",
            "tap-and-drag",
            "click-method",
            "two-finger-scrolling-enabled",
            "edge-scrolling-enabled",
            "natural-scroll",
        ],
    ),
    (
        "org.gnome.desktop.peripherals.tablet",
        Some("/org/gnome/desktop/peripherals/tablets/056a:0357/"),
        &["left-handed", "mapping", "output", "keep-aspect", "area"],
    ),
    (
        "org.gnome.desktop.peripherals.tablet.stylus",
        Some("/org/gnome/desktop/peripherals/stylus/default-056a:0357/"),
        &[
            "pressure-curve",
            "eraser-pressure-curve",
            "button-action",
            "secondary-button-action",
            "tertiary-button-action",
            "button-keybinding",
            "secondary-button-keybinding",
            "tertiary-button-keybinding",
            "eraser-button-mode",
        ],
    ),
    (
        "org.gnome.desktop.peripherals.tablet.stylus",
        Some("/org/gnome/desktop/peripherals/stylus/8a01c2f3/"),
        &["pressure-curve", "button-action"],
    ),
    (
        "org.gnome.desktop.peripherals.touchscreen",
        Some("/org/gnome/desktop/peripherals/touchscreens/04f3:2072/"),
        &["output"],
    ),
];

#[test]
fn every_key_the_pointer_pages_bind_is_writable_through_gsettings_set() {
    for (schema, path, keys) in BOUND {
        for key in *keys {
            let row = allow::find(schema, *path, key)
                .unwrap_or_else(|| panic!("{schema} {key} at {path:?}: no allow row"));
            assert!(
                matches!(row.mode, Mode::Write),
                "{schema} {key}: {:?}",
                row.mode.name()
            );
        }
    }
    // the touchpad's Reset (gsettings.reset) resets every key of its row, all of them writable
    let touchpad = allow::find("org.gnome.desktop.peripherals.touchpad", None, "speed").unwrap();
    assert!(touchpad.keys.len() >= 9 && touchpad.keys.iter().all(|k| *k != "*"));
    // amendments 10 and 67: no sensitivity dropdown, so the timeout mutter 50 ignores stays out of every table
    assert!(
        allow::find(
            "org.gnome.desktop.peripherals.touchpad",
            None,
            "disable-while-typing-timeout"
        )
        .is_none()
    );
    // a path outside the holes is no row: another schema's path, upper-case ids, a pad control at a stylus path
    for (schema, path) in [
        (
            "org.gnome.desktop.peripherals.tablet",
            "/org/gnome/desktop/peripherals/tablets/056A:0357/",
        ),
        (
            "org.gnome.desktop.peripherals.tablet",
            "/org/gnome/desktop/peripherals/stylus/default-056a:0357/",
        ),
        (
            "org.gnome.desktop.peripherals.tablet.stylus",
            "/org/gnome/desktop/peripherals/stylus/default-056a/",
        ),
        (
            "org.gnome.desktop.peripherals.tablet.pad-button",
            "/org/gnome/desktop/peripherals/stylus/buttonA/",
        ),
    ] {
        assert!(
            allow::find(schema, Some(path), "left-handed").is_none()
                && allow::find(schema, Some(path), "action").is_none(),
            "{path}"
        );
    }
    assert!(
        allow::find(
            "org.gnome.desktop.peripherals.tablet.pad-button",
            Some("/org/gnome/desktop/peripherals/tablets/056a:0357/ringA-ccw/"),
            "action"
        )
        .is_some()
    );
}
