//! display.* under GNOME (notes/claude/settings-parity.md §5.5, §7; plan testing "D: GetCurrentState decode and
//! ApplyMonitorsConfig encode"): the decode and encode on fixtures, the refusals, and the methods against the fake
//! DisplayConfig and gsd-color on the private session bus (display/fake.rs), with a recording bridge in place of
//! extension/displayBridge.js.
use super::fake::{Fake, FakeState, STATE_TYPE, TWO_MONITORS};
use super::*;
use crate::i18n::Lang;
use std::cell::RefCell;

fn fixture() -> Value {
    let ty = glib::VariantTy::new(STATE_TYPE).unwrap();
    let v = glib::Variant::parse(Some(ty), TWO_MONITORS).unwrap();
    mutter::decode_state(
        &variant_json(&v),
        &json!({"ApplyMonitorsConfigAllowed": true, "NightLightSupported": true, "PanelOrientationManaged": false}),
    )
}

/// A Display on its own context (`dry_run` as asked), without new()'s watcher threads.
struct TestDisplay {
    root: std::path::PathBuf,
    display: Display,
}

impl TestDisplay {
    fn new(language: &str, dry_run: bool) -> Self {
        let root = std::env::temp_dir().join(format!("win11-display-{}", crate::events::nonce()));
        let config = root.join("config");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            config.join("settings.json"),
            json!({"language": language}).to_string(),
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
        let display = Display {
            ctx,
            stopped: AtomicBool::new(true),
            last: Mutex::new(Value::Null),
            context: Mutex::new(None),
        };
        Self { root, display }
    }
}

impl Drop for TestDisplay {
    fn drop(&mut self) {
        self.display.ctx.events.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A bridge that records its calls and answers from `answer` (None: the extension is not there).
struct Recorder {
    calls: RefCell<Vec<(String, Value)>>,
    answer: fn(&str) -> Option<Value>,
}

impl Recorder {
    fn new(answer: fn(&str) -> Option<Value>) -> Self {
        Self {
            calls: RefCell::new(Vec::new()),
            answer,
        }
    }
    fn call(&self, method: &str, args: &Value) -> Result<Value, String> {
        self.calls
            .borrow_mut()
            .push((method.to_owned(), args.clone()));
        (self.answer)(method).ok_or_else(|| "no extension".to_owned())
    }
    fn methods(&self) -> Vec<String> {
        self.calls.borrow().iter().map(|(m, _)| m.clone()).collect()
    }
}

fn shell_bridge_answers(method: &str) -> Option<Value> {
    Some(match method {
        "display.expectConfirm" => json!({"id": 4}),
        "display.confirm" => json!({"pending": true}),
        _ => json!({"ok": true}),
    })
}

fn no_bridge(_: &str) -> Option<Value> {
    None
}

/// display.apply's arguments for the fixture: both monitors side by side at their current modes, `method`.
fn two_side_by_side(method: &str) -> Value {
    json!({"serial": 7, "method": method, "logical": [
        {"x": 0, "y": 0, "scale": 1.6666666666666667, "transform": 0, "primary": true,
            "monitors": [{"connector": "eDP-1", "mode": "2560x1600@60.002"}]},
        {"x": 1536, "y": 0, "scale": 2.0, "transform": 0, "primary": false,
            "monitors": [{"connector": "HDMI-1", "mode": "3840x2160@60.000", "props": {"underscanning": true}}]}
    ]})
}

#[test]
fn get_current_state_decodes_monitors_modes_and_logical_monitors() {
    let state = fixture();
    assert_eq!(state["supported"], true);
    assert_eq!(state["serial"], 7);
    assert_eq!(state["allowed"], true);
    assert_eq!(state["nightLightSupported"], true);
    assert_eq!(state["layoutMode"], 1);
    let m = &state["monitors"][0];
    assert_eq!(m["connector"], "eDP-1");
    assert_eq!(m["displayName"], "Ingebouwd beeldscherm");
    assert_eq!(m["builtin"], true);
    assert_eq!(m["current"], "2560x1600@60.002");
    assert_eq!(m["preferred"], "2560x1600@60.002");
    assert_eq!(m["modes"].as_array().unwrap().len(), 4);
    assert_eq!(m["modes"][0]["width"], 2560);
    assert_eq!(m["modes"][0]["refresh"], 60.002);
    assert_eq!(m["modes"][0]["scales"].as_array().unwrap().len(), 5);
    assert_eq!(
        m["modes"][2]["variable"], true,
        "refresh-rate-mode variable"
    );
    assert_eq!(m["modes"][1]["variable"], false);
    assert_eq!(
        m["privacyScreen"],
        json!({"enabled": false, "locked": false})
    );
    assert_eq!(m["minRefreshRate"], 48);
    assert_eq!(m["colorModes"], json!([0, 1]), "HDR (BT.2100) offered");
    assert_eq!(
        m["underscanning"],
        Value::Null,
        "no is-underscanning: not supported"
    );
    assert_eq!(m["logical"], 0);
    assert_eq!(m["primary"], true);
    assert_eq!(m["scale"], 1.6666666666666667);
    let tv = &state["monitors"][1];
    assert_eq!(tv["connector"], "HDMI-1");
    assert_eq!(tv["vendor"], "DEL");
    assert_eq!(tv["product"], "DELL U2720Q");
    assert_eq!(tv["serial"], "ABC123");
    assert_eq!(tv["underscanning"], false, "supported, off");
    assert_eq!(tv["x"], 1536);
    assert_eq!(tv["primary"], false);
    assert_eq!(state["logical"][1]["monitors"], json!(["HDMI-1"]));
    // a monitor in no logical monitor is off ("Alleen weergeven op 1")
    let mut reply = variant_json(
        &glib::Variant::parse(
            Some(glib::VariantTy::new(STATE_TYPE).unwrap()),
            TWO_MONITORS,
        )
        .unwrap(),
    );
    reply[2].as_array_mut().unwrap().pop();
    let off = mutter::decode_state(&reply, &json!({"ApplyMonitorsConfigAllowed": false}));
    assert_eq!(off["monitors"][1]["logical"], Value::Null);
    assert_eq!(off["monitors"][1]["x"], Value::Null);
    assert_eq!(
        off["allowed"], false,
        "Beeldscherminstellingen kunnen nu niet worden gewijzigd"
    );
    assert_eq!(
        off["nightLightSupported"],
        Value::Null,
        "an older mutter has no NightLightSupported"
    );
}

#[test]
fn apply_monitors_config_is_encoded_with_mutters_signature() {
    let plan = mutter::parse_apply(&two_side_by_side("persistent"), Lang::Nl).unwrap();
    assert_eq!(plan.method, mutter::PERSISTENT);
    assert_eq!(plan.logical.len(), 2);
    assert_eq!(plan.logical[1].monitors[0].underscanning, Some(true));
    let v = mutter::encode(&plan, mutter::VERIFY);
    assert_eq!(v.type_().as_str(), "(uua(iiduba(ssa{sv}))a{sv})");
    let j = variant_json(&v);
    assert_eq!(j[0], 7);
    assert_eq!(j[1], 0, "the method asked for, not the plan's");
    assert_eq!(j[2][0][0], 0);
    assert_eq!(j[2][1][0], 1536);
    assert_eq!(j[2][0][2], 1.6666666666666667);
    assert_eq!(j[2][0][4], true);
    assert_eq!(j[2][0][5][0][0], "eDP-1");
    assert_eq!(j[2][0][5][0][1], "2560x1600@60.002");
    assert_eq!(j[2][0][5][0][2], json!({}));
    assert_eq!(j[2][1][5][0][2], json!({"underscanning": true}));
    assert_eq!(j[3], json!({}));
    // a mirror (one logical monitor, both monitors), HDR and the layout mode
    let mirror = json!({"serial": 9, "method": 2, "props": {"layout-mode": 2}, "logical": [
        {"x": 0, "y": 0, "scale": 1.0, "primary": true, "monitors": [
            {"connector": "eDP-1", "mode": "1920x1200@60.000", "props": {"color-mode": 1}},
            {"connector": "HDMI-1", "mode": "1920x1080@60.000"}]}]});
    let plan = mutter::parse_apply(&mirror, Lang::Nl).unwrap();
    assert_eq!(plan.layout_mode, Some(2));
    assert_eq!(plan.logical[0].transform, 0, "transform defaults to normal");
    let j = variant_json(&mutter::encode(&plan, plan.method));
    assert_eq!(j[2][0][5].as_array().unwrap().len(), 2);
    assert_eq!(j[2][0][5][0][2], json!({"color-mode": 1}));
    assert_eq!(j[3], json!({"layout-mode": 2}));
}

#[test]
fn a_wrong_layout_is_refused_before_mutter_sees_it() {
    let mut good = two_side_by_side("persistent");
    good["props"] = json!({});
    good["logical"][0]["monitors"][0]["props"] = json!({});
    assert!(mutter::parse_apply(&good, Lang::Nl).is_ok());
    let mut cases = Vec::new();
    for (path, value) in [
        ("/serial", json!(-1)),
        ("/serial", json!("7")),
        ("/method", json!("forever")),
        ("/method", json!(3)),
        ("/logical", json!([])),
        ("/logical/0/scale", json!(0)),
        ("/logical/0/scale", json!(9)),
        ("/logical/0/transform", json!(8)),
        ("/logical/0/primary", json!("yes")),
        ("/logical/0/x", json!(1.5)),
        ("/logical/0/monitors", json!([])),
        ("/logical/0/monitors/0/connector", json!("eDP 1")),
        ("/logical/0/monitors/0/connector", json!("")),
        ("/logical/0/monitors/0/mode", json!("a'b")),
        ("/logical/0/monitors/0/props", json!({"brightness": 1})),
        (
            "/logical/0/monitors/0/props",
            json!({"underscanning": "on"}),
        ),
        ("/logical/1/monitors/0/connector", json!("eDP-1")),
        ("/props", json!({"layout-mode": 0})),
        ("/props", json!({"global-scale": 2})),
    ] {
        let mut args = good.clone();
        *args.pointer_mut(path).unwrap() = value;
        cases.push((path, args));
    }
    for (path, args) in cases {
        let e = mutter::parse_apply(&args, Lang::En).unwrap_err();
        assert_eq!(e.code, "bad_args", "{path}");
        assert_eq!(e.key.as_deref(), Some("display.badConfig"), "{path}");
        assert_eq!(e.message, "Invalid display layout.");
    }
    assert_eq!(
        mutter::parse_apply(&json!({}), Lang::Nl)
            .unwrap_err()
            .message,
        "Ongeldige beeldschermindeling."
    );
}

#[test]
fn mutters_refusals_say_what_happened_in_the_settings_language() {
    let stale = "GDBus.Error:org.freedesktop.DBus.Error.AccessDenied: The requested configuration is based on stale information";
    let e = mutter::apply_error(stale, Lang::En);
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("failed", Some("display.stale"))
    );
    assert_eq!(
        e.message,
        "Your displays changed in the meantime. Try again."
    );
    let e = mutter::apply_error(
        "GDBus.Error:org.freedesktop.DBus.Error.AccessDenied: Changing configuration is not allowed",
        Lang::Nl,
    );
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("display.notAllowed"))
    );
    assert_eq!(
        e.message,
        "Beeldscherminstellingen kunnen nu niet worden gewijzigd."
    );
    let e = mutter::apply_error(
        "GDBus.Error:org.freedesktop.DBus.Error.InvalidArgs: Logical monitors not adjacent",
        Lang::En,
    );
    assert_eq!(e.key.as_deref(), Some("display.applyFailed"));
    assert_eq!(
        e.message,
        "Couldn’t apply the display settings: Logical monitors not adjacent"
    );
    let e = mutter::apply_error(
        "GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: The name is not activatable",
        Lang::Nl,
    );
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("unsupported", Some("display.unavailable"))
    );
}

#[test]
fn night_light_status_and_identify_labels() {
    let none = mutter::night_light(None, Some(true));
    assert_eq!(
        none["available"], false,
        "no gsd-color: nothing to turn off until tomorrow"
    );
    assert_eq!(
        none["supported"], true,
        "the keys still work, as in GNOME Settings"
    );
    assert_eq!(
        mutter::night_light(None, Some(false))["supported"],
        false,
        "Night Light Unavailable"
    );
    let p = json!({"NightLightActive": true, "DisabledUntilTomorrow": false, "Temperature": 2700, "Sunrise": 6.5, "Sunset": 20.75});
    let s = mutter::night_light(Some(&p), None);
    assert_eq!(
        s["supported"], true,
        "an older mutter without NightLightSupported"
    );
    assert_eq!(s["active"], true);
    assert_eq!(s["sunset"], 20.75);
    assert_eq!(
        mutter::night_light(Some(&p), Some(false))["supported"],
        false
    );
    let bad = json!({"Sunrise": -1.0, "Sunset": 99.0});
    assert_eq!(
        mutter::night_light(Some(&bad), Some(true))["sunrise"],
        Value::Null
    );
    assert_eq!(mutter::labels(&fixture()), json!({"eDP-1": 1, "HDMI-1": 2}));
}

#[test]
fn a_dry_run_applies_nothing_and_asks_nothing() {
    let Some(fake) = Fake::start(FakeState::default()) else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let t = TestDisplay::new("nl", true);
    let bridge = Recorder::new(shell_bridge_answers);
    let call = |m: &str, a: &Value| bridge.call(m, a);
    let ep = fake.endpoints();
    let r = t
        .display
        .apply(&ep, &two_side_by_side("persistent"), &call)
        .unwrap();
    assert_eq!(
        r,
        json!({"ok": true, "dryRun": true, "method": "persistent", "confirm": "none"})
    );
    // still checked: a dry run refuses what a real run would
    assert!(t.display.apply(&ep, &json!({"serial": 7}), &call).is_err());
    assert_eq!(
        t.display.confirm(&json!({"keep": true}), &call).unwrap()["dryRun"],
        true
    );
    assert_eq!(
        t.display
            .disable_until_tomorrow(&ep, &json!({"on": true}))
            .unwrap()["dryRun"],
        true
    );
    assert!(fake.calls_of("ApplyMonitorsConfig").is_empty());
    assert!(fake.calls_of("Set DisabledUntilTomorrow").is_empty());
    assert!(bridge.methods().is_empty(), "no keep question in a dry run");
    assert!(!fake.state.lock().unwrap().disabled_until_tomorrow);
}

#[test]
fn a_persistent_apply_is_verified_then_settings_asks_the_keep_question() {
    let Some(fake) = Fake::start(FakeState::default()) else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let t = TestDisplay::new("en", false);
    let ep = fake.endpoints();
    let state = t.display.state(&ep);
    assert_eq!(state["serial"], 7);
    assert_eq!(state["monitors"].as_array().unwrap().len(), 2);
    let bridge = Recorder::new(shell_bridge_answers);
    let call = |m: &str, a: &Value| bridge.call(m, a);
    let r = t
        .display
        .apply(&ep, &two_side_by_side("persistent"), &call)
        .unwrap();
    assert_eq!(
        r["confirm"], "settings",
        "displayBridge.js holds back GNOME Shell's own dialog"
    );
    assert_eq!(r["id"], 4);
    assert_eq!(r["timeout"], 15);
    let applied = fake.calls_of("ApplyMonitorsConfig");
    assert_eq!(applied.len(), 2);
    assert_eq!(applied[0][1], 0, "verify first");
    assert_eq!(applied[1][1], 2, "then persistent");
    assert_eq!(applied[1][0], 7, "the state's serial");
    assert_eq!(applied[1][2][1][5][0][2], json!({"underscanning": true}));
    assert_eq!(bridge.methods(), ["display.expectConfirm"]);
    assert_eq!(bridge.calls.borrow()[0].1, json!({"timeout": 15}));
    // the answer goes through the bridge
    let r = t
        .display
        .confirm(&json!({"keep": false, "id": 4}), &call)
        .unwrap();
    assert_eq!(r, json!({"ok": true, "kept": false, "pending": true}));
    assert_eq!(
        bridge.calls.borrow()[1],
        ("display.confirm".into(), json!({"keep": false, "id": 4}))
    );
    let e = t
        .display
        .confirm(&json!({"keep": "yes"}), &call)
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("core.badArgs"));
    // temporary and verify never ask
    let r = t
        .display
        .apply(&ep, &two_side_by_side("temporary"), &call)
        .unwrap();
    assert_eq!(r["confirm"], "none");
    assert_eq!(fake.calls_of("ApplyMonitorsConfig").len(), 3);
    assert_eq!(bridge.methods().len(), 2);
}

#[test]
fn without_the_bridge_gnome_shell_asks_and_a_refusal_releases_the_hold() {
    let Some(fake) = Fake::start(FakeState::default()) else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let t = TestDisplay::new("nl", false);
    let ep = fake.endpoints();
    let missing = Recorder::new(no_bridge);
    let call = |m: &str, a: &Value| missing.call(m, a);
    let r = t
        .display
        .apply(&ep, &two_side_by_side("persistent"), &call)
        .unwrap();
    assert_eq!(
        r,
        json!({"ok": true, "method": "persistent", "confirm": "shell"})
    );
    // the persistent call is refused after the hold: the bridge lets GNOME Shell's handler go again
    let bridge = Recorder::new(shell_bridge_answers);
    let call = |m: &str, a: &Value| bridge.call(m, a);
    fake.state.lock().unwrap().apply_error = Some((
        "org.freedesktop.DBus.Error.AccessDenied".into(),
        "The requested configuration is based on stale information".into(),
    ));
    let e = t
        .display
        .apply(&ep, &two_side_by_side("persistent"), &call)
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("display.stale"));
    assert_eq!(
        e.message,
        "De beeldschermen zijn intussen gewijzigd. Probeer het opnieuw."
    );
    // verify failed first: nothing was held
    assert!(bridge.methods().is_empty());
    // only the persistent call fails: the hold is released again
    fake.state.lock().unwrap().apply_error_method = Some(mutter::PERSISTENT);
    assert!(
        t.display
            .apply(&ep, &two_side_by_side("persistent"), &call)
            .is_err()
    );
    assert_eq!(
        bridge.methods(),
        ["display.expectConfirm", "display.release"]
    );
    assert_eq!(bridge.calls.borrow()[1].1, json!({"id": 4}));
    bridge.calls.borrow_mut().clear();
    // identify: the bridge gets display.state's numbers; without it Settings flashes its own tiles
    let r = t.display.identify(&ep, &call).unwrap();
    assert_eq!(r["labels"], json!({"eDP-1": 1, "HDMI-1": 2}));
    assert_eq!(
        bridge.calls.borrow()[0].1,
        json!({"labels": {"eDP-1": 1, "HDMI-1": 2}, "seconds": 3})
    );
    let call = |m: &str, a: &Value| missing.call(m, a);
    let e = t.display.identify(&ep, &call).unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("unsupported", Some("display.identifyUnavailable"))
    );
    let e = t
        .display
        .confirm(&json!({"keep": true}), &call)
        .unwrap_err();
    assert_eq!(e.key.as_deref(), Some("display.confirmFailed"));
}

#[test]
fn night_light_is_turned_off_until_tomorrow_and_back_on() {
    let Some(fake) = Fake::start(FakeState::default()) else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let t = TestDisplay::new("nl", false);
    let ep = fake.endpoints();
    let s = t.display.night_light(&ep);
    assert_eq!(s["available"], true);
    assert_eq!(s["supported"], true);
    assert_eq!(s["active"], true);
    assert_eq!(s["temperature"], 2700);
    assert_eq!(s["sunset"], 20.5);
    let s = t
        .display
        .disable_until_tomorrow(&ep, &json!({"on": true}))
        .unwrap();
    assert_eq!(s["disabledUntilTomorrow"], true, "Nu uitschakelen");
    assert_eq!(fake.calls_of("Set DisabledUntilTomorrow"), [json!(true)]);
    let s = t
        .display
        .disable_until_tomorrow(&ep, &json!({"on": false}))
        .unwrap();
    assert_eq!(s["disabledUntilTomorrow"], false, "Nu inschakelen");
    let e = t
        .display
        .disable_until_tomorrow(&ep, &json!({"on": 1}))
        .unwrap_err();
    assert_eq!(e.code, "bad_args");
    // mutter says the hardware has no gamma (GNOME 50 NightLightSupported)
    fake.state.lock().unwrap().night_light_supported = false;
    assert_eq!(t.display.night_light(&ep)["supported"], false);
    // gsd-color gone
    let gone = mutter::Endpoints {
        mutter: fake.name.clone(),
        color: ":1.99999".into(),
    };
    let e = t
        .display
        .disable_until_tomorrow(&gone, &json!({"on": true}))
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("unsupported", Some("display.nightLightUnavailable"))
    );
    assert_eq!(t.display.night_light(&gone)["available"], false);
}
