use super::*;

fn isolated(backend: &gio::SettingsBackend) -> Preferences {
    // An explicit private backend is essential: tests must never touch dconf.
    Preferences::with_source(gio::SettingsSchemaSource::default().as_ref(), Some(backend))
}
#[test]
fn energy_patch_validation_never_coerces_or_accepts_partial_patches() {
    for invalid in [
        json!({}),
        json!([]),
        json!({"idleDelay":true}),
        json!({"idleDelay":-1}),
        json!({"idleDelay":1.5}),
        json!({"idleDelay":4294967296u64}),
        json!({"idleDim":1}),
        json!({"idleDim":false,"idleDelay":0}),
        json!({"sleepAC":{"action":"suspend","timeout":0}}),
        json!({"sleepAC":{"action":"shutdown","timeout":60}}),
        json!({"sleepAC":{"action":"nothing","timeout":-1}}),
        json!({"sleepAC":{"action":"nothing","timeout":60,"extra":1}}),
        json!({"sleepBattery":{"action":"suspend","timeout":"120"}}),
        json!({"powerProfile":""}),
        json!({"shell":"command"}),
    ] {
        assert!(Patch::parse(&invalid, Lang::Nl).is_err(), "{invalid}");
    }
    assert!(matches!(
        Patch::parse(&json!({"idleDelay":0}), Lang::Nl),
        Ok(Patch::IdleDelay(0))
    ));
    assert!(matches!(
        Patch::parse(
            &json!({"sleepAC":{"action":"nothing","timeout":0}}),
            Lang::Nl
        ),
        Ok(Patch::Sleep { timeout: 0, .. })
    ));
}
#[test]
fn energy_memory_backend_preserves_custom_timeouts_and_never() {
    let p = isolated(&gio::memory_settings_backend_new());
    assert!(
        p.session.is_some() && p.power.is_some(),
        "GNOME schemas required for target platform"
    );
    assert_eq!(
        p.set(&json!({"idleDelay":137}), None, Lang::Nl).unwrap()["screen"]["idleDelay"],
        137
    );
    assert_eq!(
        p.set(&json!({"idleDelay":0}), None, Lang::Nl).unwrap()["screen"]["idleDelay"],
        0
    );
    assert_eq!(
        p.set(&json!({"idleDim":false}), None, Lang::Nl).unwrap()["dim"]["enabled"],
        false
    );
    let power = p.power.as_ref().unwrap();
    power
        .set_string("sleep-inactive-ac-type", "hibernate")
        .unwrap();
    power.set_int("sleep-inactive-ac-timeout", 193).unwrap();
    assert_eq!(p.snapshot(None)["sleepAC"]["action"], "hibernate");
    assert_eq!(p.snapshot(None)["sleepAC"]["timeout"], 193);
    let never = p
        .set(
            &json!({"sleepAC":{"action":"nothing","timeout":193}}),
            None,
            Lang::Nl,
        )
        .unwrap();
    assert_eq!(
        never["sleepAC"],
        json!({"available":true,"writable":true,"action":"nothing","timeout":193})
    );
    p.set(
        &json!({"sleepBattery":{"action":"nothing","timeout":0}}),
        None,
        Lang::Nl,
    )
    .unwrap();
    assert_eq!(power.int("sleep-inactive-battery-timeout"), 0);
    // An unknown/unsupported suspend capability cannot enable suspend.
    assert_eq!(
        p.set(
            &json!({"sleepAC":{"action":"suspend","timeout":60}}),
            None,
            Lang::Nl
        )
        .unwrap_err()
        .code,
        "unsupported"
    );
    assert_eq!(power.string("sleep-inactive-ac-type"), "nothing");
    assert_eq!(power.int("sleep-inactive-ac-timeout"), 193);
}
#[test]
fn energy_readonly_backend_and_missing_schemas_are_honest() {
    let p = isolated(&gio::null_settings_backend_new());
    assert_eq!(p.snapshot(None)["screen"]["writable"], false);
    assert_eq!(
        p.set(&json!({"idleDelay":0}), None, Lang::Nl)
            .unwrap_err()
            .code,
        "denied"
    );
    let missing = Preferences::with_source(None, None);
    let snapshot = missing.snapshot(None);
    assert_eq!(snapshot["screen"]["available"], false);
    assert_eq!(snapshot["profiles"]["available"], false);
    assert_eq!(snapshot["canSuspend"], false);
    assert_eq!(
        missing
            .set(&json!({"idleDim":true}), None, Lang::Nl)
            .unwrap_err()
            .code,
        "unsupported"
    );
}
#[test]
fn energy_schema_checks_reject_wrong_types_unknown_keys_and_enum_values() {
    let p = isolated(&gio::memory_settings_backend_new());
    assert_eq!(
        write(
            p.power.as_ref(),
            &[("sleep-inactive-ac-type", "made-up".to_variant())],
            Lang::Nl
        )
        .unwrap_err()
        .code,
        "bad_args"
    );
    assert_eq!(
        write(
            p.session.as_ref(),
            &[("idle-delay", true.to_variant())],
            Lang::Nl
        )
        .unwrap_err()
        .code,
        "bad_args"
    );
    assert_eq!(
        write(
            p.session.as_ref(),
            &[("unknown", 0u32.to_variant())],
            Lang::Nl
        )
        .unwrap_err()
        .code,
        "unsupported"
    );
}
#[test]
fn energy_external_gsettings_changes_trigger_existing_monitor_without_polling() {
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let p = isolated(&gio::memory_settings_backend_new());
            let count = std::rc::Rc::new(std::cell::Cell::new(0));
            let changed = count.clone();
            p.watch(move || changed.set(changed.get() + 1));
            p.session
                .as_ref()
                .unwrap()
                .set_uint("idle-delay", 43)
                .unwrap();
            p.power
                .as_ref()
                .unwrap()
                .set_boolean("idle-dim", false)
                .unwrap();
            while context.pending() {
                context.iteration(false);
            }
            assert_eq!(count.get(), 2);
        })
        .unwrap();
}

#[test]
#[ignore = "Run with work/native-energy-test/run_private.py; private D-Bus fixture only"]
fn energy_profiles_use_real_typed_dbus_and_readback() {
    assert_eq!(
        std::env::var("WIN11_PRIVATE_ENERGY_TEST").as_deref(),
        Ok("1")
    );
    let address = std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap();
    assert!(
        address.contains("/tmp/win11-native-energy-"),
        "private bus required"
    );
    let bus = system().unwrap();
    let p = isolated(&gio::memory_settings_backend_new());
    assert_eq!(p.snapshot(Some(&bus))["profiles"]["active"], "balanced");
    assert!(can_suspend(Some(&bus)));
    assert_eq!(
        p.set(&json!({"powerProfile":"power-saver"}), Some(&bus), Lang::Nl)
            .unwrap()["profiles"]["active"],
        "power-saver"
    );
    assert_eq!(
        p.set(&json!({"powerProfile":"injected"}), Some(&bus), Lang::Nl)
            .unwrap_err()
            .code,
        "bad_args"
    );
    assert_eq!(
        p.set(
            &json!({"sleepAC":{"action":"suspend","timeout":173}}),
            Some(&bus),
            Lang::Nl
        )
        .unwrap()["sleepAC"]["timeout"],
        173
    );
    let control = |method: &str| {
        call(
            &bus,
            "org.example.EnergyFixture",
            "/org/example/EnergyFixture",
            "org.example.EnergyFixture",
            method,
            None,
            2000,
        )
        .unwrap()
    };
    control("Inhibit");
    assert_eq!(
        p.snapshot(Some(&bus))["profiles"]["choices"][2]["disabled"],
        true
    );
    assert_eq!(
        p.set(&json!({"powerProfile":"performance"}), Some(&bus), Lang::Nl)
            .unwrap_err()
            .code,
        "unsupported"
    );
    control("Deny");
    assert_eq!(
        p.set(&json!({"powerProfile":"balanced"}), Some(&bus), Lang::Nl)
            .unwrap_err()
            .code,
        "denied"
    );
    control("Stale");
    assert_eq!(
        p.set(&json!({"powerProfile":"balanced"}), Some(&bus), Lang::Nl)
            .unwrap_err()
            .code,
        "failed"
    );
    control("Legacy");
    assert_eq!(
        p.set(&json!({"powerProfile":"balanced"}), Some(&bus), Lang::Nl)
            .unwrap()["profiles"]["active"],
        "balanced"
    );
    control("NoSuspend");
    assert!(!can_suspend(Some(&bus)));
    assert_eq!(
        p.set(
            &json!({"sleepAC":{"action":"suspend","timeout":33}}),
            Some(&bus),
            Lang::Nl
        )
        .unwrap_err()
        .code,
        "unsupported"
    );
    control("ChallengeSuspend");
    assert!(can_suspend(Some(&bus)));
}
