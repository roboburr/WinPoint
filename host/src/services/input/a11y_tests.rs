//! Batch A's host tests of slices 3 and 4 (notes/claude/settings-parity.md §7): the pages Toegankelijkheid,
//! Tekstgrootte, Visuele effecten, Contrastthema's, Muisaanwijzer en aanraken and Tekstcursor, the Toegankelijkheid
//! tile of Snelle instellingen (native/shell/src/flyouts/quick.rs), and slice 4's Vergrootglas, Verteller, Audio,
//! Toetsenbord and Muis bind GNOME's keys through `gsettings.*`: every key they read or write has its allow row in the
//! mode the control needs, and the writes the pages send pass `gsettings.set`'s checks and read back GNOME's values.
//! Verteller's one method, `input.screenReader.preferences` (amendment 36), runs against an Orca fake on the
//! sandbox's private session bus.
use super::screen_reader;
use crate::i18n::Lang;
use crate::services::gsettings::allow::{self, Mode};
use crate::services::gsettings::{self, Backend, Caller, Config, Env};
use serde_json::{Value, json};

const IFACE: &str = "org.gnome.desktop.interface";
const A11Y_IFACE: &str = "org.gnome.desktop.a11y.interface";

#[test]
fn every_key_the_accessibility_pages_bind_has_its_allow_row() {
    let write = [
        (
            "org.gnome.desktop.a11y",
            "always-show-universal-access-status",
        ),
        (A11Y_IFACE, "show-status-shapes"),
        (IFACE, "overlay-scrolling"),
        (IFACE, "cursor-size"),
        (IFACE, "locate-pointer"),
        (IFACE, "cursor-blink"),
        (IFACE, "cursor-blink-time"),
        // the quick settings tile's functions
        (
            "org.gnome.desktop.a11y.applications",
            "screen-magnifier-enabled",
        ),
        (
            "org.gnome.desktop.a11y.applications",
            "screen-reader-enabled",
        ),
        (
            "org.gnome.desktop.a11y.applications",
            "screen-keyboard-enabled",
        ),
        ("org.gnome.desktop.a11y.keyboard", "stickykeys-enable"),
        ("org.gnome.desktop.a11y.keyboard", "slowkeys-enable"),
        ("org.gnome.desktop.a11y.keyboard", "mousekeys-enable"),
    ];
    for (schema, key) in write {
        assert_eq!(
            allow::find(schema, None, key).map(|r| r.mode),
            Some(Mode::Write),
            "{schema} {key}"
        );
    }
    // the rendering pairs (host/src/settings_mirror/render.rs): Settings writes GNOME's key, the mirror the setting
    for (schema, key, setting) in [
        (A11Y_IFACE, "high-contrast", "accessibility.highContrast"),
        (A11Y_IFACE, "reduced-motion", "accessibility.reduceMotion"),
        (IFACE, "enable-animations", "accessibility.reduceMotion"),
        (IFACE, "text-scaling-factor", "accessibility.textScale"),
    ] {
        let row = allow::find(schema, None, key).unwrap_or_else(|| panic!("{schema} {key}"));
        assert_eq!(row.mode, Mode::Mirrored(setting), "{schema} {key}");
        assert!(row.mode.writes());
    }
    // Contrastthema's shows GNOME's toggle-contrast shortcut, which Sneltoetsen writes
    assert_eq!(
        allow::find(
            "org.gnome.settings-daemon.plugins.media-keys",
            None,
            "toggle-contrast"
        )
        .map(|r| r.mode),
        Some(Mode::Service("input.shortcuts.set"))
    );
    // the touchscreen rows of Muisaanwijzer en aanraken are pen.rs' (amendment 8)
    assert_eq!(
        allow::find(
            "org.gnome.desktop.peripherals.touchscreen",
            Some("/org/gnome/desktop/peripherals/touchscreens/04f3:2072/"),
            "output"
        )
        .map(|r| r.mode),
        Some(Mode::Write)
    );
    // Windows' Stijl van muisaanwijzer is left out: cursor-theme stays in no table (GNOME Settings 50 has no control)
    assert!(allow::find(IFACE, None, "cursor-theme").is_none());
}

const MAG: &str = "org.gnome.desktop.a11y.magnifier";
const KB: &str = "org.gnome.desktop.a11y.keyboard";
const A11Y_MOUSE: &str = "org.gnome.desktop.a11y.mouse";
const WM_PREFS: &str = "org.gnome.desktop.wm.preferences";
const MEDIA: &str = "org.gnome.settings-daemon.plugins.media-keys";

#[test]
fn every_key_of_the_slice_4_pages_has_its_allow_row() {
    let mut write: Vec<(&str, &str)> = vec![
        (
            "org.gnome.desktop.a11y.applications",
            "screen-magnifier-enabled",
        ),
        (
            "org.gnome.desktop.a11y.applications",
            "screen-reader-enabled",
        ),
        (
            "org.gnome.desktop.a11y.applications",
            "screen-keyboard-enabled",
        ),
        (WM_PREFS, "visual-bell"),
        (WM_PREFS, "visual-bell-type"),
        (WM_PREFS, "focus-mode"),
        // batch D's row (one owner for the key Geluid and Audio share, amendment 50)
        ("org.gnome.desktop.sound", "allow-volume-above-100-percent"),
        ("org.gnome.desktop.peripherals.mouse", "double-click"),
    ];
    for key in [
        "mag-factor",
        "screen-position",
        "lens-mode",
        "scroll-at-edges",
        "mouse-tracking",
        "focus-tracking",
        "caret-tracking",
        "invert-lightness",
        "brightness-red",
        "brightness-green",
        "brightness-blue",
        "contrast-red",
        "contrast-green",
        "contrast-blue",
        "color-saturation",
        "show-cross-hairs",
        "cross-hairs-clip",
        "cross-hairs-thickness",
        "cross-hairs-length",
        "cross-hairs-color",
        "cross-hairs-opacity",
    ] {
        write.push((MAG, key));
    }
    for key in [
        "stickykeys-enable",
        "stickykeys-two-key-off",
        "stickykeys-modifier-beep",
        "slowkeys-enable",
        "slowkeys-delay",
        "slowkeys-beep-press",
        "slowkeys-beep-accept",
        "slowkeys-beep-reject",
        "bouncekeys-enable",
        "bouncekeys-delay",
        "bouncekeys-beep-reject",
        "togglekeys-enable",
        "enable",
        "feature-state-change-beep",
        "mousekeys-enable",
        "mousekeys-max-speed",
        "mousekeys-accel-time",
    ] {
        write.push((KB, key));
    }
    for key in [
        "secondary-click-enabled",
        "secondary-click-time",
        "dwell-click-enabled",
        "dwell-time",
        "dwell-threshold",
    ] {
        write.push((A11Y_MOUSE, key));
    }
    for (schema, key) in write {
        assert_eq!(
            allow::find(schema, None, key).map(|r| r.mode),
            Some(Mode::Write),
            "{schema} {key}"
        );
    }
    // shown as key caps and set in Sneltoetsen; Print Screen goes through input.shortcuts.set (its conflicts)
    for (schema, key) in [
        (MEDIA, "magnifier"),
        (MEDIA, "magnifier-zoom-in"),
        (MEDIA, "magnifier-zoom-out"),
        (MEDIA, "screenreader"),
        ("org.gnome.shell.keybindings", "show-screenshot-ui"),
    ] {
        assert_eq!(
            allow::find(schema, None, key).map(|r| r.mode),
            Some(Mode::Service("input.shortcuts.set")),
            "{schema} {key}"
        );
    }
    // GNOME's keys Settings 50 shows no control for stay out of every table (audible-bell remains, windows.json)
    assert!(allow::find(WM_PREFS, None, "audible-bell").is_none());
}

fn memory() -> Env {
    Env::new(
        &Config {
            backend: Backend::Memory,
            extension_dirs: Some(Vec::new()),
        },
        None,
    )
}

fn installed(schema: &str) -> bool {
    gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup(schema, true))
        .is_some()
}

/// `gsettings.set` as the page sends it, on `env`: the values the host read back.
fn set(env: &mut Env, args: Value, dry_run: bool) -> Result<Value, String> {
    let writes = gsettings::parse_set(&args, Lang::En).map_err(|e| e.message)?;
    let done = gsettings::set(env, &writes, Lang::En, dry_run, Caller::Api)
        .map_err(|e| format!("{}: {}", e.code, e.message))?;
    let mut values = serde_json::Map::new();
    for item in done["items"].as_array().into_iter().flatten() {
        values.insert(
            item["key"].as_str().unwrap_or("").to_owned(),
            item["value"].clone(),
        );
    }
    Ok(Value::Object(values))
}

#[test]
fn the_slice_4_pages_writes_pass_gsettings_set_and_read_back() {
    for schema in [MAG, KB, A11Y_MOUSE, WM_PREFS] {
        if !installed(schema) {
            eprintln!("skipped: {schema} is not installed");
            return;
        }
    }
    let mut env = memory();
    // (the page's gsettings.set arguments, what GNOME then holds); the multi-key ones are written together
    let cases = [
        // Vergrootglas › Weergave: Lens and Vastgezet write lens-mode with screen-position
        (
            json!({"schema": MAG, "values": {"lens-mode": true, "screen-position": "top-half"}}),
            json!({"lens-mode": true, "screen-position": "top-half"}),
        ),
        (
            json!({"schema": MAG, "values": {"lens-mode": false, "screen-position": "full-screen"}}),
            json!({"lens-mode": false, "screen-position": "full-screen"}),
        ),
        // Kleuraanpassing: one value for red, green and blue
        (
            json!({"schema": MAG, "values": {"brightness-red": 0.4, "brightness-green": 0.4, "brightness-blue": 0.4}}),
            json!({"brightness-red": 0.4, "brightness-green": 0.4, "brightness-blue": 0.4}),
        ),
        (
            json!({"schema": MAG, "values": {"contrast-red": -0.8, "contrast-green": -0.8, "contrast-blue": -0.8}}),
            json!({"contrast-red": -0.8, "contrast-green": -0.8, "contrast-blue": -0.8}),
        ),
        (
            json!({"schema": MAG, "values": {"mag-factor": 2.25}}),
            json!({"mag-factor": 2.25}),
        ),
        (
            json!({"schema": MAG, "values": {"mouse-tracking": "push"}}),
            json!({"mouse-tracking": "push"}),
        ),
        (
            json!({"schema": MAG, "values": {"focus-tracking": "none"}}),
            json!({"focus-tracking": "none"}),
        ),
        // Dradenkruis: Lengte 0 % is the schema's 20 px, Kleur a #rrggbb, Dekking a fraction
        (
            json!({"schema": MAG, "values": {"cross-hairs-length": 20}}),
            json!({"cross-hairs-length": 20}),
        ),
        (
            json!({"schema": MAG, "values": {"cross-hairs-color": "#00ff00"}}),
            json!({"cross-hairs-color": "#00ff00"}),
        ),
        (
            json!({"schema": MAG, "values": {"cross-hairs-opacity": 0.5}}),
            json!({"cross-hairs-opacity": 0.5}),
        ),
        // Audio: Het actieve venster laten knipperen
        (
            json!({"schema": WM_PREFS, "values": {"visual-bell": true, "visual-bell-type": "frame-flash"}}),
            json!({"visual-bell": true, "visual-bell-type": "frame-flash"}),
        ),
        // Toetsenbord: Filtertoetsen off turns both off
        (
            json!({"schema": KB, "values": {"slowkeys-enable": false, "bouncekeys-enable": false}}),
            json!({"slowkeys-enable": false, "bouncekeys-enable": false}),
        ),
        (
            json!({"schema": KB, "values": {"slowkeys-delay": 250}}),
            json!({"slowkeys-delay": 250}),
        ),
        // Muis: Muistoetsen's sliders, Klikhulp's seconds and pixels, Activate Windows on Hover
        (
            json!({"schema": KB, "values": {"mousekeys-max-speed": 250, "mousekeys-accel-time": 1000}}),
            json!({"mousekeys-max-speed": 250, "mousekeys-accel-time": 1000}),
        ),
        (
            json!({"schema": A11Y_MOUSE, "values": {"secondary-click-time": 1.5, "dwell-time": 0.8, "dwell-threshold": 15}}),
            json!({"secondary-click-time": 1.5, "dwell-time": 0.8, "dwell-threshold": 15}),
        ),
        (
            json!({"schema": WM_PREFS, "values": {"focus-mode": "sloppy"}}),
            json!({"focus-mode": "sloppy"}),
        ),
    ];
    for (args, want) in cases {
        assert_eq!(set(&mut env, args.clone(), false), Ok(want), "{args}");
    }
    // a dry run validates and writes nothing
    let mut fresh = memory();
    let args =
        json!({"schema": MAG, "values": {"lens-mode": true, "screen-position": "left-half"}});
    set(&mut fresh, args, true).expect("the dry run validates");
    let keys = [
        gsettings::KeyRef {
            schema: MAG.into(),
            path: None,
            key: "lens-mode".into(),
        },
        gsettings::KeyRef {
            schema: MAG.into(),
            path: None,
            key: "screen-position".into(),
        },
    ];
    let items = gsettings::get(&mut fresh, &keys, Lang::En).unwrap();
    assert_eq!(
        items["items"][0]["value"],
        json!(false),
        "lens-mode untouched"
    );
    assert_eq!(items["items"][1]["value"], json!("full-screen"));
    // GNOME's ranges hold: a crosshair below 20 px and a brightness above 1 are refused, and nothing is written
    for bad in [
        json!({"schema": MAG, "values": {"cross-hairs-length": 10}}),
        json!({"schema": MAG, "values": {"brightness-red": 0.2, "brightness-green": 1.5}}),
    ] {
        assert!(set(&mut fresh, bad.clone(), false).is_err(), "{bad}");
    }
    let red = [gsettings::KeyRef {
        schema: MAG.into(),
        path: None,
        key: "brightness-red".into(),
    }];
    assert_eq!(
        gsettings::get(&mut fresh, &red, Lang::En).unwrap()["items"][0]["value"],
        json!(0.0),
        "a refused write of three keys writes none of them"
    );
}

/// A private session bus only: tests/rust-sandbox/run-tests.sh's (never the live one under /run/user).
fn private_bus_address() -> Option<String> {
    let address = std::env::var("DBUS_SESSION_BUS_ADDRESS").ok()?;
    (std::env::var("GSETTINGS_BACKEND").as_deref() == Ok("keyfile")
        && !address.contains("/run/user/"))
    .then_some(address)
}

fn code(r: crate::files::ApiResult) -> Option<(String, String)> {
    r.err().map(|e| (e.code, e.key.unwrap_or_default()))
}

#[test]
fn narrator_settings_ask_the_running_orca_for_its_preferences() {
    use gio::prelude::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    let Some(address) = private_bus_address() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let bus = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE).unwrap();
    // no Orca yet: the screen reader is off
    assert_eq!(
        code(screen_reader::show_preferences(Lang::En)),
        Some(("unsupported".into(), "input.orcaNotRunning".into()))
    );
    assert_eq!(
        code(screen_reader::show_preferences_on(
            &bus,
            ":1.9999",
            Lang::Nl
        )),
        Some(("unsupported".into(), "input.orcaNotRunning".into())),
        "nobody there"
    );

    const XML: &str = r#"<node><interface name="org.gnome.Orca.Service">
        <method name="ShowPreferences"><arg type="b" direction="out"/></method>
        </interface></node>"#;
    // (method, interactive flag, auto-start allowed)
    let calls: Arc<Mutex<Vec<(String, bool, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let answer = Arc::new(AtomicBool::new(true));
    let stop = Arc::new(AtomicBool::new(false));
    let (ready, started) = mpsc::channel();
    let (seen, thread_stop, thread_answer) = (calls.clone(), stop.clone(), answer.clone());
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
            let iface = node.lookup_interface(screen_reader::INTERFACE).unwrap();
            let id = connection
                .register_object(
                    screen_reader::OBJECT_PATH,
                    &iface,
                    move |_, _, _, _, method, _, invocation| {
                        let flags = invocation.message().flags();
                        seen.lock().unwrap().push((
                            method.to_owned(),
                            flags.contains(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION),
                            !flags.contains(gio::DBusMessageFlags::NO_AUTO_START),
                        ));
                        let ok = thread_answer.load(Ordering::Acquire);
                        invocation.return_value(Some(&(ok,).to_variant()));
                    },
                    |_, _, _, _, _| ().to_variant(),
                    |_, _, _, _, _, _| false,
                )
                .unwrap();
            // Orca's well-known name, as orca/dbus_service.py registers it
            let owned = connection
                .call_sync(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "RequestName",
                    Some(&(screen_reader::BUS_NAME, 4u32).to_variant()),
                    glib::VariantTy::new("(u)").ok(),
                    gio::DBusCallFlags::NONE,
                    2000,
                    gio::Cancellable::NONE,
                )
                .ok()
                .and_then(|v| v.get::<(u32,)>())
                == Some((1,));
            ready
                .send((
                    connection.unique_name().unwrap().to_string(),
                    owned,
                    context.clone(),
                ))
                .unwrap();
            while !thread_stop.load(Ordering::Acquire) {
                context.iteration(true);
            }
            connection.unregister_object(id).unwrap();
        });
    });
    let (name, owned, context) = started.recv().unwrap();
    assert_eq!(
        screen_reader::show_preferences_on(&bus, &name, Lang::En).unwrap(),
        json!({"shown": true})
    );
    if owned {
        assert_eq!(
            screen_reader::show_preferences(Lang::En).unwrap(),
            json!({"shown": true}),
            "through Orca's well-known name"
        );
    }
    answer.store(false, Ordering::Release);
    assert_eq!(
        code(screen_reader::show_preferences_on(&bus, &name, Lang::En)),
        Some(("failed".into(), "input.orcaFailed".into())),
        "Orca without a script to show"
    );
    let seen = calls.lock().unwrap().clone();
    assert_eq!(seen.len(), if owned { 3 } else { 2 });
    assert!(
        seen.iter()
            .all(|(m, interactive, auto)| m == "ShowPreferences" && !interactive && !auto),
        "plain calls: no password dialog, no D-Bus activation: {seen:?}"
    );
    stop.store(true, Ordering::Release);
    context.wakeup();
    server.join().unwrap();
}
