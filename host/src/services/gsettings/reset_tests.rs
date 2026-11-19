//! The service-side reset ([`reset_values`], notes/claude/settings-i.md package host-settings H4) on a memory
//! backend, never dconf: a service resets only its own rows, as [`write_values`] writes them.
use super::*;

const SOUND: &str = "org.gnome.desktop.sound";
const METHOD: &str = "sound.alerts.set";

fn memory_env() -> Env {
    Env::new(
        &Config {
            backend: Backend::Memory,
            extension_dirs: Some(Vec::new()),
        },
        None,
    )
}

fn theme_name(env: &mut Env) -> Value {
    read_item_in(env, SOUND, None, "theme-name", Lang::En).unwrap()
}

#[test]
fn a_service_resets_its_own_rows_and_nothing_else() {
    let lang = Lang::En;
    let mut env = memory_env();
    write_values_in(
        &mut env,
        METHOD,
        SOUND,
        None,
        &[("theme-name", json!("__custom"))],
        lang,
        false,
    )
    .unwrap();
    assert_eq!(theme_name(&mut env)["isDefault"], false);
    // a dry run checks and writes nothing
    let out = reset_values_in(&mut env, METHOD, SOUND, None, &["theme-name"], lang, true).unwrap();
    assert_eq!(out["items"][0]["value"], "__custom");
    assert_eq!(theme_name(&mut env)["value"], "__custom");
    assert_eq!(theme_name(&mut env)["isDefault"], false);
    // the method's own Service row: g_settings_reset
    let out = reset_values_in(&mut env, METHOD, SOUND, None, &["theme-name"], lang, false).unwrap();
    let item = &out["items"][0];
    assert_eq!(item["isDefault"], true, "{item}");
    assert_eq!(item["value"], item["default"], "{item}");
    assert_eq!(theme_name(&mut env)["isDefault"], true);
    // another method's row, and gsettings.reset itself (Caller::Api), are refused
    write_values_in(
        &mut env,
        METHOD,
        SOUND,
        None,
        &[("theme-name", json!("__custom"))],
        lang,
        false,
    )
    .unwrap();
    let err = reset_values_in(
        &mut env,
        "sound.setVolume",
        SOUND,
        None,
        &["theme-name"],
        lang,
        false,
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("gsettings.useMethod"));
    assert_eq!(err.data, Some(json!({ "method": METHOD })));
    let err = reset(
        &mut env,
        SOUND,
        None,
        &["theme-name".to_owned()],
        lang,
        false,
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("gsettings.useMethod"));
    assert_eq!(
        theme_name(&mut env)["value"],
        "__custom",
        "a refusal resets nothing"
    );
    // a Read row and an Owned row: never
    let err = reset_values_in(
        &mut env,
        METHOD,
        "org.gnome.desktop.notifications",
        None,
        &["application-children"],
        lang,
        false,
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("gsettings.readOnly"));
    let err = reset_values_in(
        &mut env,
        METHOD,
        "org.gnome.desktop.interface",
        None,
        &["gtk-theme"],
        lang,
        false,
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("gsettings.ownedByWinPoint"));
    // one refused key refuses the whole call
    let err = reset_values_in(
        &mut env,
        METHOD,
        SOUND,
        None,
        &["theme-name", "no-such-key"],
        lang,
        false,
    )
    .unwrap_err();
    assert_eq!(err.key.as_deref(), Some("gsettings.notAllowed"));
    assert_eq!(theme_name(&mut env)["value"], "__custom");
    // bad arguments
    for (path, keys) in [
        (None, &[][..]),
        (Some("no-slashes"), &["theme-name"][..]),
        (None, &["Bad Key"][..]),
    ] {
        let err = reset_values_in(&mut env, METHOD, SOUND, path, keys, lang, false).unwrap_err();
        assert_eq!(
            err.key.as_deref(),
            Some("gsettings.badArgs"),
            "{path:?} {keys:?}"
        );
    }
}

#[test]
fn resetting_a_held_default_is_refused_before_any_key_changes() {
    let dir = std::env::temp_dir().join(format!("reset-held-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let xml = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../extension/schemas/org.gnome.shell.extensions.win11-gnome.gschema.xml");
    std::fs::copy(
        xml,
        dir.join("org.gnome.shell.extensions.win11-gnome.gschema.xml"),
    )
    .unwrap();
    assert!(
        std::process::Command::new("/usr/bin/glib-compile-schemas")
            .arg(&dir)
            .status()
            .unwrap()
            .success()
    );
    let mut env = Env::new(
        &Config {
            backend: Backend::Memory,
            extension_dirs: Some(vec![dir.clone()]),
        },
        None,
    );
    let method = "input.shortcuts.set";
    for (schema, key, ext_key) in [
        (
            "org.gnome.desktop.wm.keybindings",
            "switch-applications",
            "task-view",
        ),
        (
            "org.gnome.settings-daemon.plugins.media-keys",
            "logout",
            "task-manager",
        ),
    ] {
        if env.schema(schema).is_none() {
            continue;
        }
        write_values_in(
            &mut env,
            method,
            schema,
            None,
            &[(key, json!(["<Super>q"]))],
            Lang::En,
            false,
        )
        .unwrap();
        for dry in [true, false] {
            let error =
                reset_values_in(&mut env, method, schema, None, &[key], Lang::En, dry).unwrap_err();
            assert_eq!(error.key.as_deref(), Some("gsettings.heldByWinPoint"));
            assert_eq!(error.data.as_ref().unwrap()["setting"], ext_key);
            assert_eq!(
                read_item_in(&mut env, schema, None, key, Lang::En).unwrap()["value"],
                json!(["<Super>q"])
            );
        }
        if schema == "org.gnome.desktop.wm.keybindings" {
            write_values_in(
                &mut env,
                method,
                schema,
                None,
                &[("close", json!(["<Super>z"]))],
                Lang::En,
                false,
            )
            .unwrap();
            assert!(
                reset_values_in(
                    &mut env,
                    method,
                    schema,
                    None,
                    &["close", key],
                    Lang::En,
                    false
                )
                .is_err()
            );
            assert_eq!(
                read_item_in(&mut env, schema, None, "close", Lang::En).unwrap()["value"],
                json!(["<Super>z"])
            );
        }
        write_values_in(
            &mut env,
            method,
            owned::EXTENSION_SCHEMA,
            None,
            &[(ext_key, json!([]))],
            Lang::En,
            false,
        )
        .unwrap();
        reset_values_in(&mut env, method, schema, None, &[key], Lang::En, false).unwrap();
        assert_eq!(
            read_item_in(&mut env, schema, None, key, Lang::En).unwrap()["isDefault"],
            true
        );
    }
    drop(env);
    std::fs::remove_dir_all(dir).unwrap();
}
