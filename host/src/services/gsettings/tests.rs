//! B0's tests of `gsettings.*` (notes/claude/settings-parity.md §7): the allowlist audit against the installed
//! schemas, the value mapping, writes on a memory backend (never dconf), lockdown, events, held accelerators.
use super::*;
use crate::events::Events;
use std::sync::Mutex;

type Seen = Arc<Mutex<Vec<(String, Value)>>>;

struct Fixture {
    root: PathBuf,
    events: Arc<Events>,
    service: Arc<GSettings>,
    seen: Seen,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.service.shutdown();
        self.events.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture_with(config: Config, dry_run: bool) -> Fixture {
    let root = std::env::temp_dir().join(format!("gsettings-{}", crate::events::nonce()));
    std::fs::create_dir_all(&root).unwrap();
    let events = Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        root.join("config"),
        root.join("cache"),
        root.clone(),
        "settings".into(),
        dry_run,
    )
    .unwrap();
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let emit: Emit = Arc::new(move |name, data| {
        recorder.lock().unwrap().push((name.to_owned(), data));
    });
    let service = GSettings::with(ctx, config, Some(emit)).unwrap();
    Fixture {
        root,
        events,
        service,
        seen,
    }
}

/// A private memory backend: never dconf.
fn memory(dry_run: bool) -> Fixture {
    fixture_with(
        Config {
            backend: Backend::Memory,
            extension_dirs: Some(Vec::new()),
        },
        dry_run,
    )
}

fn call(f: &Fixture, method: &str, args: Value) -> ApiResult {
    f.service.call(method, &args).expect("a gsettings method")
}

fn err(result: ApiResult) -> (String, String, Value) {
    let e = result.expect_err("a refusal");
    (
        e.code,
        e.key.unwrap_or_default(),
        e.data.unwrap_or(Value::Null),
    )
}

const MOUSE: &str = "org.gnome.desktop.peripherals.mouse";

fn installed(schema: &str) -> Option<gio::SettingsSchema> {
    gio::SettingsSchemaSource::default()?.lookup(schema, true)
}

#[test]
fn allowlist_audit_against_the_installed_schemas() {
    let mut seen: HashMap<(String, String, String), &str> = HashMap::new();
    for (batch, row) in allow::rows() {
        assert!(
            !allow::FORBIDDEN_SCHEMAS
                .iter()
                .any(|p| row.schema.starts_with(p)),
            "{batch}: {} is never allowlisted (ubuntu-dock, DING, tiling-assistant)",
            row.schema
        );
        for key in row.keys {
            assert!(
                !(row.mode.writes() && allow::forbidden(row.schema, key)),
                "{batch}: {} {key} may never be written",
                row.schema
            );
            assert_ne!(
                *key, "disable-while-typing-timeout",
                "amendment 10: mutter 50 ignores it"
            );
        }
        let Some(schema) = installed(row.schema) else {
            assert!(
                row.optional.is_some(),
                "{batch}: {} is not installed and not optional",
                row.schema
            );
            continue;
        };
        match row.path {
            None => assert!(
                schema.path().is_some(),
                "{batch}: {} is relocatable: give a path",
                row.schema
            ),
            Some(pattern) => {
                assert!(
                    schema.path().is_none(),
                    "{batch}: {} has a fixed path",
                    row.schema
                );
                assert!(
                    pattern.starts_with('/') && pattern.ends_with('/'),
                    "{pattern}"
                );
            }
        }
        let keys: Vec<String> = if row.keys == ["*"] {
            schema.list_keys().iter().map(|k| k.to_string()).collect()
        } else {
            row.keys.iter().map(|k| (*k).to_owned()).collect()
        };
        for key in keys {
            assert!(
                schema.has_key(&key),
                "{batch}: {} has no key {key}",
                row.schema
            );
            let kind = value::kind(&schema.key(&key));
            assert!(
                !matches!(kind, Kind::Unsupported(_)),
                "{batch}: {} {key} has a type the API does not map: {kind:?}",
                row.schema
            );
            let slot = (
                row.schema.to_owned(),
                row.path.unwrap_or("").to_owned(),
                key.clone(),
            );
            if let Some(other) = seen.insert(slot, batch) {
                panic!(
                    "{} {key} is in two allow tables ({other}, {batch})",
                    row.schema
                );
            }
        }
    }
    // WinPoint's look keys are Owned, with the setting the page links to
    for (schema, key, setting) in owned::LOOK_KEYS {
        let probe = if *key == "*" { "picture-uri" } else { key };
        let row = allow::find(schema, None, probe)
            .unwrap_or_else(|| panic!("{schema} {key} is not listed"));
        assert_eq!(row.mode, Mode::Owned(setting), "{schema} {key}");
    }
    // the mirror's pairs are Mirrored rows with their first setting (amendment 49); render pairs are A's
    for pair in crate::settings_mirror::all_pairs() {
        let row = allow::find(pair.schema, None, pair.key).unwrap();
        assert_eq!(
            row.mode,
            Mode::Mirrored(pair.settings[0]),
            "{} {}",
            pair.schema,
            pair.key
        );
    }
    // the extension schema and the global-shortcuts table are read natively, never through the mapper (43)
    assert!(
        allow::find(
            "org.gnome.settings-daemon.global-shortcuts",
            None,
            "applications"
        )
        .is_none()
    );
    assert!(allow::find("org.gnome.shell", None, "enabled-extensions").is_none());
    assert_eq!(
        allow::find("org.gnome.mutter", None, "experimental-features")
            .unwrap()
            .mode,
        Mode::Read
    );
}

#[test]
fn relocatable_path_holes() {
    use allow::{hole_ok, path_matches};
    for (hole, good, bad) in [
        ("appid", "org-gnome-nautilus", "Org.Gnome"),
        ("custom", "custom12", "custom"),
        ("vidpid", "056a:0357", "56a:357"),
        ("serial", "1a2b3c4d", "1A2B"),
        ("serial", "default-056a:0357", "default-056a"),
        ("padctl", "buttonA", "button1"),
        ("padctl", "ringA-ccw", "ringA-left"),
        ("padctl", "stripB-up", "strip-up"),
        ("button", "buttonC", "buttonCC"),
        ("service", "rygel", "Rygel"),
    ] {
        assert!(hole_ok(hole, good), "{hole} {good}");
        assert!(!hole_ok(hole, bad), "{hole} {bad}");
    }
    let pad = "/org/gnome/desktop/peripherals/tablets/{vidpid}/{padctl}/";
    assert!(path_matches(
        pad,
        "/org/gnome/desktop/peripherals/tablets/056a:0357/buttonA/"
    ));
    assert!(!path_matches(
        pad,
        "/org/gnome/desktop/peripherals/tablets/056a:0357/"
    ));
    assert!(!path_matches(
        pad,
        "/org/gnome/desktop/peripherals/tablets/056a:0357/buttonA"
    ));
    assert!(!path_matches(
        pad,
        "/org/gnome/desktop/peripherals/tablets/056a:0357//buttonA/"
    ));
    assert!(!path_matches(
        pad,
        "/org/gnome/desktop/peripherals/tablets/../buttonA/"
    ));
}

#[test]
fn json_and_gvariant_round_trip_for_every_supported_type() {
    let cases: Vec<(Kind, Value)> = vec![
        (Kind::Bool, json!(true)),
        (Kind::Int, json!(-42)),
        (Kind::Uint, json!(4_000_000_000u64)),
        (Kind::Uint16, json!(3389)),
        (Kind::Double, json!(0.5)),
        (Kind::Str, json!("Yaru")),
        (Kind::StrList, json!(["<Super>l", "<Control><Alt>l"])),
        (Kind::IntList, json!([0, 50, 50, 100])),
        (Kind::DoubleList, json!([0.0, 0.25, 1.0, 1.0])),
        (Kind::PairList, json!([["xkb", "us"], ["ibus", "anthy"]])),
        (Kind::Point, json!([52.37, 4.89])),
        (
            Kind::Enum(vec!["flat".into(), "default".into()]),
            json!("flat"),
        ),
        (Kind::Flags(vec!["a".into(), "b".into()]), json!(["a", "b"])),
    ];
    for (kind, json_value) in &cases {
        let v =
            value::to_variant(kind, json_value).unwrap_or_else(|| panic!("{kind:?} {json_value}"));
        assert_eq!(value::to_json(kind, &v), *json_value, "{kind:?}");
    }
    for (kind, bad) in [
        (Kind::Bool, json!(1)),
        (Kind::Int, json!(1.5)),
        (Kind::Int, json!(4_000_000_000u64)),
        (Kind::Uint, json!(-1)),
        (Kind::Uint16, json!(70000)),
        (Kind::Double, json!("0.5")),
        (Kind::Str, json!(null)),
        (Kind::StrList, json!(["a", 1])),
        (Kind::PairList, json!([["a"]])),
        (Kind::Point, json!([1.0])),
        (Kind::Unsupported("a(ssssud)".into()), json!([])),
    ] {
        assert!(value::to_variant(&kind, &bad).is_none(), "{kind:?} {bad}");
    }
    assert_eq!(value::kind_of("a{sv}"), Kind::Unsupported("a{sv}".into()));
    // enum and flags come from the key's range
    let mouse = installed(MOUSE).unwrap();
    assert!(
        matches!(value::kind(&mouse.key("accel-profile")), Kind::Enum(c) if c.contains(&"flat".to_owned()))
    );
    assert_eq!(value::kind(&mouse.key("speed")), Kind::Double);
    assert_eq!(
        value::range_json(&mouse.key("speed"), &Kind::Double),
        json!({"min": -1.0, "max": 1.0})
    );
}

#[test]
fn memory_backend_writes_read_back_and_reset() {
    let f = memory(false);
    let key = |schema: &str, key: &str| json!({"schema": schema, "key": key});
    let items = call(
        &f,
        "gsettings.get",
        json!({"keys": [key(MOUSE, "left-handed"), key(MOUSE, "accel-profile")]}),
    )
    .unwrap();
    let left = &items["items"][0];
    assert_eq!(
        (
            left["value"].clone(),
            left["isDefault"].clone(),
            left["type"].clone(),
            left["mode"].clone()
        ),
        (json!(false), json!(true), json!("b"), json!("write"))
    );
    assert!(
        items["items"][1]["range"]["choices"]
            .as_array()
            .unwrap()
            .contains(&json!("flat"))
    );
    // one schema, several keys: the mouse page's reference case (plan.testing step 4)
    let set = call(
        &f,
        "gsettings.set",
        json!({"schema": MOUSE, "values": {"left-handed": true, "speed": 0.5, "accel-profile": "flat", "natural-scroll": true}}),
    )
    .unwrap();
    assert_eq!(set["items"].as_array().unwrap().len(), 4);
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [key(MOUSE, "left-handed"), key(MOUSE, "speed")]}),
    )
    .unwrap();
    assert_eq!(got["items"][0]["value"], true);
    assert_eq!(got["items"][0]["isDefault"], false);
    assert_eq!(got["items"][1]["value"], 0.5);
    // several schemas, in order (amendment 51)
    let touchpad = "org.gnome.desktop.peripherals.touchpad";
    call(
        &f,
        "gsettings.set",
        json!({"writes": [
            {"schema": touchpad, "values": {"tap-to-click": false}},
            {"schema": MOUSE, "values": {"double-click": 600}},
        ]}),
    )
    .unwrap();
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [key(touchpad, "tap-to-click"), key(MOUSE, "double-click")]}),
    )
    .unwrap();
    assert_eq!(
        (
            got["items"][0]["value"].clone(),
            got["items"][1]["value"].clone()
        ),
        (json!(false), json!(600))
    );
    // reset
    let reset = call(
        &f,
        "gsettings.reset",
        json!({"schema": MOUSE, "keys": ["left-handed", "speed"]}),
    )
    .unwrap();
    assert_eq!(reset["items"][0]["value"], false);
    assert_eq!(reset["items"][1]["isDefault"], true);
    // a relocatable schema at a typed path (pen and tablet, amendment 9)
    let tablet = "/org/gnome/desktop/peripherals/tablets/056a:0357/";
    call(
        &f,
        "gsettings.set",
        json!({"schema": "org.gnome.desktop.peripherals.tablet", "path": tablet, "values": {"left-handed": true}}),
    )
    .unwrap();
    let stylus = "/org/gnome/desktop/peripherals/stylus/default-056a:0357/";
    call(
        &f,
        "gsettings.set",
        json!({"schema": "org.gnome.desktop.peripherals.tablet.stylus", "path": stylus, "values": {"pressure-curve": [0, 25, 75, 100]}}),
    )
    .unwrap();
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": "org.gnome.desktop.peripherals.tablet", "path": tablet, "key": "left-handed"}]}),
    )
    .unwrap();
    assert_eq!(got["items"][0]["value"], true);
    assert_eq!(got["items"][0]["path"], tablet);
}

#[test]
fn refusals_carry_their_codes_keys_and_data() {
    let f = memory(false);
    let set = |schema: &str, values: Value| {
        call(
            &f,
            "gsettings.set",
            json!({"schema": schema, "values": values}),
        )
    };
    assert_eq!(
        err(set(MOUSE, json!({"left-handed": "yes"}))),
        (
            "bad_args".into(),
            "gsettings.badValue".into(),
            json!({"key": "left-handed"})
        )
    );
    assert_eq!(
        err(set(MOUSE, json!({"accel-profile": "nonsense"}))).1,
        "gsettings.badValue"
    );
    assert_eq!(
        err(set(MOUSE, json!({"speed": 5.0}))).1,
        "gsettings.badValue",
        "out of the key's range"
    );
    assert_eq!(
        err(set(
            "org.gnome.desktop.interface",
            json!({"gtk-theme": "Adwaita"})
        )),
        (
            "denied".into(),
            "gsettings.ownedByWinPoint".into(),
            json!({"setting": "theme.gtkFamily"})
        )
    );
    assert_eq!(
        err(set(
            "org.gnome.desktop.input-sources",
            json!({"sources": [["xkb", "us"]]})
        ))
        .2,
        json!({"method": "input.sources.set"})
    );
    assert_eq!(
        err(set(
            "org.gnome.mutter",
            json!({"experimental-features": []})
        ))
        .1,
        "gsettings.notAllowed"
    );
    // a Read row (experimental-features is also forbidden, so it is refused before its mode)
    assert_eq!(
        err(set(
            "org.gnome.desktop.notifications",
            json!({"application-children": []})
        ))
        .1,
        "gsettings.readOnly"
    );
    assert_eq!(
        err(set("org.gnome.shell", json!({"enabled-extensions": []}))).1,
        "gsettings.notAllowed"
    );
    assert_eq!(
        err(set(
            "org.gnome.desktop.interface",
            json!({"font-name": "Ubuntu 11"})
        ))
        .1,
        "gsettings.notAllowed"
    );
    assert_eq!(
        err(set("org.example.missing", json!({"x": true}))).1,
        "gsettings.notAllowed"
    );
    assert_eq!(
        err(set(
            "org.gnome.desktop.a11y.keyboard",
            json!({"no-such-key": true})
        ))
        .0,
        "unsupported",
        "an allowlisted schema without the key"
    );
    // one bad key refuses the whole call: nothing was queued
    assert!(set(MOUSE, json!({"left-handed": true, "speed": "fast"})).is_err());
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": MOUSE, "key": "left-handed"}]}),
    )
    .unwrap();
    assert_eq!(got["items"][0]["value"], false);
    // paths: a fixed schema at another path (bad arguments), a relocatable one without its path (no row)
    assert_eq!(
        err(call(
            &f,
            "gsettings.set",
            json!({"schema": MOUSE, "path": "/elsewhere/", "values": {"left-handed": true}})
        ))
        .1,
        "gsettings.badArgs",
        "a path that does not fit the schema"
    );
    assert_eq!(
        err(call(&f, "gsettings.set", json!({"schema": "org.gnome.desktop.peripherals.tablet", "values": {"left-handed": true}}))).1,
        "gsettings.notAllowed"
    );
    for bad in [
        json!({}),
        json!({"schema": MOUSE}),
        json!({"schema": MOUSE, "values": {}}),
        json!({"writes": []}),
    ] {
        assert_eq!(err(call(&f, "gsettings.set", bad)).1, "gsettings.badArgs");
    }
    // not allowlisted keys are reported, never read; missing schemas are unavailable
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": "org.gnome.desktop.interface", "key": "font-name"}, {"schema": "org.example.missing", "key": "x"}]}),
    )
    .unwrap();
    assert_eq!(
        (
            got["items"][0]["allowed"].clone(),
            got["items"][0]["value"].clone()
        ),
        (json!(false), Value::Null)
    );
    assert_eq!(got["items"][1]["available"], false);
}

#[test]
fn a_dry_run_validates_and_writes_nothing() {
    let f = memory(true);
    let set = call(
        &f,
        "gsettings.set",
        json!({"schema": MOUSE, "values": {"left-handed": true}}),
    )
    .unwrap();
    assert_eq!(
        set["items"][0]["value"], false,
        "the current value: nothing changed"
    );
    assert!(
        err(call(
            &f,
            "gsettings.set",
            json!({"schema": MOUSE, "values": {"left-handed": 3}})
        ))
        .1 == "gsettings.badValue"
    );
    call(
        &f,
        "gsettings.reset",
        json!({"schema": MOUSE, "keys": ["speed"]}),
    )
    .unwrap();
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": MOUSE, "key": "left-handed"}]}),
    )
    .unwrap();
    assert_eq!(got["items"][0]["isDefault"], true);
}

#[test]
fn a_locked_key_is_denied() {
    // a keyfile backend whose defaults directory locks one key (lockdown), and the null backend (nothing writable)
    let dir = std::env::temp_dir().join(format!("gsettings-locks-{}", crate::events::nonce()));
    std::fs::create_dir_all(dir.join("defaults")).unwrap();
    std::fs::write(
        dir.join("defaults/locks"),
        "/org/gnome/desktop/peripherals/mouse/left-handed\n",
    )
    .unwrap();
    let locked = fixture_with(
        Config {
            backend: Backend::Keyfile {
                file: dir.join("keyfile"),
                defaults: dir.join("defaults"),
            },
            extension_dirs: Some(Vec::new()),
        },
        false,
    );
    let got = call(&locked, "gsettings.get", json!({"keys": [{"schema": MOUSE, "key": "left-handed"}, {"schema": MOUSE, "key": "speed"}]}))
        .unwrap();
    assert_eq!(got["items"][0]["writable"], false, "the locks file");
    assert_eq!(got["items"][1]["writable"], true);
    assert_eq!(
        err(call(
            &locked,
            "gsettings.set",
            json!({"schema": MOUSE, "values": {"left-handed": true}})
        )),
        (
            "denied".into(),
            "gsettings.locked".into(),
            json!({"key": "left-handed"})
        )
    );
    call(
        &locked,
        "gsettings.set",
        json!({"schema": MOUSE, "values": {"speed": 0.25}}),
    )
    .unwrap();
    drop(locked);
    let null = fixture_with(
        Config {
            backend: Backend::Null,
            extension_dirs: Some(Vec::new()),
        },
        false,
    );
    assert_eq!(
        err(call(
            &null,
            "gsettings.set",
            json!({"schema": MOUSE, "values": {"speed": 0.25}})
        ))
        .1,
        "gsettings.locked"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_external_write_emits_gsettings_changed() {
    let f = memory(false);
    call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": MOUSE, "key": "speed"}]}),
    )
    .unwrap();
    // another writer on the same backend, the way another program changes GNOME's settings
    f.service
        .on_worker(|env| {
            let schema = env.schema(MOUSE).unwrap();
            let other = gio::Settings::new_full(&schema, env.backend(), None);
            other.set_double("speed", -0.25).unwrap();
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let found = f.seen.lock().unwrap().iter().any(|(name, data)| {
            name == "gsettings.changed" && data["key"] == "speed" && data["item"]["value"] == -0.25
        });
        if found {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no gsettings.changed: {:?}",
            f.seen.lock().unwrap()
        );
        f.service.on_worker(|_| ()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    let (_, data) = f
        .seen
        .lock()
        .unwrap()
        .iter()
        .find(|(n, _)| n == "gsettings.changed")
        .cloned()
        .unwrap();
    assert_eq!(
        (data["schema"].clone(), data["path"].clone()),
        (json!(MOUSE), json!("/org/gnome/desktop/peripherals/mouse/"))
    );
}

#[test]
fn accelerators_the_extension_holds_are_refused_one_by_one() {
    use owned::{Held, held_accel, is_alt_tab, is_ctrl_alt_delete};
    assert!(
        is_alt_tab("<Alt>Tab")
            && is_alt_tab("<mod1>tab")
            && !is_alt_tab("<Super>Tab")
            && !is_alt_tab("<Shift><Alt>Tab")
    );
    assert!(
        is_ctrl_alt_delete("<Control><Alt>Delete") && is_ctrl_alt_delete("<Primary><Alt>delete")
    );
    assert!(!is_ctrl_alt_delete("<Control>Delete") && !is_ctrl_alt_delete("<Super>l"));
    let nothing = Held::default();
    let alt_tab = vec!["<Alt>Tab".to_owned()];
    assert_eq!(
        held_accel(
            "org.gnome.desktop.wm.keybindings",
            "switch-windows",
            &alt_tab,
            &nothing
        ),
        None
    );
    // the extension's real schema, compiled into a private directory
    let dir = std::env::temp_dir().join(format!("gsettings-ext-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../extension/schemas/org.gnome.shell.extensions.win11-gnome.gschema.xml");
    std::fs::copy(
        &xml,
        dir.join("org.gnome.shell.extensions.win11-gnome.gschema.xml"),
    )
    .unwrap();
    let status = std::process::Command::new("/usr/bin/glib-compile-schemas")
        .arg(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    let f = fixture_with(
        Config {
            backend: Backend::Memory,
            extension_dirs: Some(vec![dir.clone()]),
        },
        false,
    );
    let write = |schema: &'static str, key: &'static str, accels: Value| {
        f.service
            .on_worker(move |env| {
                let w = Write {
                    schema: schema.into(),
                    path: None,
                    values: vec![(key.into(), accels)],
                };
                set(
                    env,
                    &[w],
                    Lang::Nl,
                    false,
                    Caller::Service("input.shortcuts.set"),
                )
            })
            .unwrap()
    };
    let wm = "org.gnome.desktop.wm.keybindings";
    assert_eq!(
        err(write(
            wm,
            "switch-windows",
            json!(["<Alt>Tab", "<Super>Tab"])
        )),
        (
            "denied".into(),
            "gsettings.heldByWinPoint".into(),
            json!({"key": "switch-windows", "accel": "<Alt>Tab", "setting": "task-view"})
        )
    );
    write(wm, "switch-applications", json!(["<Super>Tab"])).unwrap();
    let media = "org.gnome.settings-daemon.plugins.media-keys";
    assert_eq!(
        err(write(media, "logout", json!(["<Control><Alt>Delete"]))).2["setting"],
        "task-manager"
    );
    write(media, "logout", json!(["<Super><Shift>q"])).unwrap();
    // through the API the keybinding schemas belong to input.shortcuts.set
    assert_eq!(
        err(call(
            &f,
            "gsettings.set",
            json!({"schema": wm, "values": {"switch-applications": ["<Super>Tab"]}})
        ))
        .2,
        json!({"method": "input.shortcuts.set"})
    );
    // the extension's own keys are read from its own schema directory
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": owned::EXTENSION_SCHEMA, "key": "task-view"}]}),
    )
    .unwrap();
    assert_eq!(got["items"][0]["value"], json!(["<Alt>Tab"]));
    drop(f);
    let _ = std::fs::remove_dir_all(dir);
}

const TABLET: &str = "org.gnome.desktop.peripherals.tablet";

#[test]
fn a_path_gsettings_refuses_never_reaches_a_settings_object() {
    // g_settings_new_full gives NULL for these (path_is_valid), and glib-rs 0.18 checks that only in debug builds:
    // the release host crashed on the first property access (review of B0, finding 1)
    let f = memory(false);
    for path in [
        "//",
        "/a//b/",
        "/org/gnome/desktop/peripherals/tablets//",
        "/x y/",
        "/x/\u{e9}/",
        "/no-end",
    ] {
        let get = json!({"keys": [{"schema": TABLET, "path": path, "key": "area"}]});
        assert_eq!(
            err(call(&f, "gsettings.get", get)).1,
            "gsettings.badArgs",
            "{path}"
        );
        let set = json!({"schema": TABLET, "path": path, "values": {"left-handed": true}});
        assert_eq!(
            err(call(&f, "gsettings.set", set)).1,
            "gsettings.badArgs",
            "{path}"
        );
        let reset = json!({"schema": TABLET, "path": path, "keys": ["left-handed"]});
        assert_eq!(
            err(call(&f, "gsettings.reset", reset)).1,
            "gsettings.badArgs",
            "{path}"
        );
        let p = path.to_owned();
        let (read, write, opened) = f
            .service
            .on_worker(move |env| {
                let read = read_item_in(env, TABLET, Some(&p), "area", Lang::Nl);
                let write = write_values_in(
                    env,
                    "input.tablet.set",
                    TABLET,
                    Some(&p),
                    &[("left-handed", json!(true))],
                    Lang::Nl,
                    false,
                );
                let schema = env.schema(TABLET).unwrap();
                let opened = env.settings(&schema, &p).is_some();
                (read.map_err(|e| e.key), write.map_err(|e| e.key), opened)
            })
            .unwrap();
        assert_eq!(
            read,
            Err(Some("gsettings.badArgs".into())),
            "read_item {path}"
        );
        assert_eq!(
            write,
            Err(Some("gsettings.badArgs".into())),
            "write_values {path}"
        );
        assert!(!opened, "Env::settings {path}");
    }
    // a valid path no allow row matches: reported, but no settings object is made (nor kept in `open`)
    let opened = |f: &Fixture| f.service.on_worker(|env| env.opened()).unwrap();
    let before = opened(&f);
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": TABLET, "path": "/org/gnome/desktop/peripherals/tablets/nope/", "key": "area"}]}),
    )
    .unwrap();
    let item = &got["items"][0];
    assert_eq!(
        (
            item["allowed"].clone(),
            item["available"].clone(),
            item["value"].clone()
        ),
        (json!(false), json!(true), Value::Null)
    );
    assert_eq!(opened(&f), before);
    // the row's own path opens (and watches) it
    let got = call(
        &f,
        "gsettings.get",
        json!({"keys": [{"schema": TABLET, "path": "/org/gnome/desktop/peripherals/tablets/056a:0357/", "key": "left-handed"}]}),
    )
    .unwrap();
    assert_eq!(got["items"][0]["allowed"], true);
    assert_eq!(got["items"][0]["value"], false);
    assert_eq!(opened(&f), before + 1);
}

#[test]
fn write_values_keeps_the_allow_rows_their_paths_and_the_owned_keys() {
    // the Rust API of the batches' own methods (review of B0, finding 2): the mode of its own Service rows is the
    // method's, everything else is gsettings.set's rule
    let f = memory(false);
    let run = |method: &'static str,
               schema: &'static str,
               path: Option<&'static str>,
               key: &'static str,
               value: Value| {
        f.service
            .on_worker(move |env| {
                write_values_in(env, method, schema, path, &[(key, value)], Lang::Nl, false)
                    .map_err(|e| {
                        (
                            e.code,
                            e.key.unwrap_or_default(),
                            e.data.unwrap_or(Value::Null),
                        )
                    })
            })
            .unwrap()
    };
    let app = "org.gnome.desktop.notifications.application";
    // (a) a path composed from an argument that misses the {appid} hole: GNOME's global banner key is no child
    assert_eq!(
        run(
            "notifications.app.set",
            app,
            Some("/org/gnome/desktop/notifications/"),
            "show-banners",
            json!(false)
        )
        .unwrap_err()
        .1,
        "gsettings.notAllowed"
    );
    assert_eq!(
        run(
            "notifications.app.set",
            app,
            Some("/org/gnome/desktop/notifications/application/Org.Bad/"),
            "show-banners",
            json!(false)
        )
        .unwrap_err()
        .1,
        "gsettings.notAllowed",
        "an appid hole takes [a-z0-9-] only"
    );
    let child = Some("/org/gnome/desktop/notifications/application/org-gnome-calculator/");
    let done = run(
        "notifications.app.set",
        app,
        child,
        "show-banners",
        json!(false),
    )
    .unwrap();
    assert_eq!(done["items"][0]["value"], false);
    // another method may not write that method's rows, nor may gsettings.set
    assert_eq!(
        run(
            "privacy.setPermission",
            app,
            child,
            "show-banners",
            json!(true)
        )
        .unwrap_err(),
        (
            "denied".into(),
            "gsettings.useMethod".into(),
            json!({"method": "notifications.app.set"})
        )
    );
    assert_eq!(
        err(call(
            &f,
            "gsettings.set",
            json!({"schema": app, "path": child, "values": {"show-banners": true}})
        ))
        .1,
        "gsettings.useMethod"
    );
    // (b) WinPoint's look keys go through gnome_look::note, never here
    assert_eq!(
        run(
            "input.shortcuts.set",
            "org.gnome.desktop.interface",
            None,
            "color-scheme",
            json!("prefer-dark")
        )
        .unwrap_err(),
        (
            "denied".into(),
            "gsettings.ownedByWinPoint".into(),
            json!({"setting": "theme.mode"})
        )
    );
    // (c) a key in no table, (d) a Read row
    assert_eq!(
        run(
            "input.shortcuts.set",
            "org.gnome.desktop.interface",
            None,
            "font-name",
            json!("Ubuntu 11")
        )
        .unwrap_err()
        .1,
        "gsettings.notAllowed"
    );
    assert_eq!(
        run(
            "notifications.app.set",
            "org.gnome.desktop.notifications",
            None,
            "application-children",
            json!([])
        )
        .unwrap_err()
        .1,
        "gsettings.readOnly"
    );
    // a Write row is any caller's
    run("input.mouse.set", MOUSE, None, "left-handed", json!(true)).unwrap();
}

/// The contract's committed sources (notes/claude/settings-parity/): the plan's pages and controls, and the GNOME
/// Settings and Windows inventories its controls were made from.
const SOURCES: &[(&str, &str)] = &[
    (
        "plan.json",
        include_str!("../../../../notes/claude/settings-parity/plan.json"),
    ),
    (
        "gnome-a.json",
        include_str!("../../../../notes/claude/settings-parity/gnome-a.json"),
    ),
    (
        "gnome-b.json",
        include_str!("../../../../notes/claude/settings-parity/gnome-b.json"),
    ),
    (
        "windows.json",
        include_str!("../../../../notes/claude/settings-parity/windows.json"),
    ),
];

/// Keys a source names only to say there is no control for them (GNOME Settings has no UI, Windows' row stays):
/// never in a writing mode.
const NO_CONTROL: &[(&str, &str, &str)] = &[
    ("org.gnome.system.proxy", "use-same-proxy", "have no UI"),
    ("org.gnome.system.proxy.http", "enabled", "have no UI"),
    (
        "org.gnome.system.proxy.http",
        "use-authentication",
        "have no UI",
    ),
    (
        "org.gnome.system.proxy.http",
        "authentication-user",
        "have no UI",
    ),
    (
        "org.gnome.desktop.remote-desktop.rdp",
        "screen-share-mode",
        "has no UI",
    ),
    (
        "org.gnome.desktop.input-sources",
        "show-all-sources",
        "has no UI",
    ),
    (
        "org.gnome.desktop.wm.preferences",
        "audible-bell",
        "remains",
    ),
];

fn key_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// Whether a source names `key`: as a whole word, or in the plan's shorthand (`brightness-red/green/blue`).
fn named(key: &str) -> bool {
    let whole = |text: &str| {
        text.match_indices(key).any(|(at, _)| {
            !text[..at].chars().next_back().is_some_and(key_char)
                && !text[at + key.len()..].chars().next().is_some_and(key_char)
        })
    };
    let short = |text: &str| {
        let Some((prefix, last)) = key.rsplit_once('-') else {
            return false;
        };
        let prefix = format!("{prefix}-");
        text.match_indices(&prefix).any(|(at, _)| {
            let tail: String = text[at + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || *c == '/')
                .collect();
            tail.contains('/') && tail.split('/').any(|part| part == last)
        })
    };
    SOURCES.iter().any(|(_, text)| whole(text) || short(text))
}

#[test]
fn every_written_key_has_a_control_in_the_contracts_sources() {
    // review of B0, finding 6: a Write or Mirrored row lets any API caller change a GNOME session key, so it needs a
    // page control (`*` rows are whole schemas a page binds, checked by the audit above)
    let mut missing = Vec::new();
    for (batch, row) in allow::rows() {
        if !matches!(row.mode, Mode::Write | Mode::Mirrored(_)) {
            continue;
        }
        for key in row.keys.iter().filter(|k| **k != "*") {
            if NO_CONTROL
                .iter()
                .any(|(schema, k, _)| *schema == row.schema && k == key)
            {
                missing.push(format!(
                    "allow/{batch}.rs: {} {key}: no control in GNOME Settings",
                    row.schema
                ));
            } else if !named(key) {
                missing.push(format!(
                    "allow/{batch}.rs: {} {key}: in no page or control",
                    row.schema
                ));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "notes/claude/settings-parity/: {missing:#?}"
    );
    for (schema, key, why) in NO_CONTROL {
        assert!(
            SOURCES
                .iter()
                .any(|(_, text)| text.contains(key) && text.contains(why)),
            "{schema} {key}: the source that says \"{why}\""
        );
    }
    assert!(
        named("brightness-green") && named("contrast-blue"),
        "the shorthand"
    );
    assert!(!named("always-show-text-caret") && !named("toolkit-accessibility"));
}
