use super::*;
mod oracle_tests;
mod private_bus_tests;
use std::{fs, sync::mpsc};
pub(super) fn fixture(dry: bool) -> (PathBuf, Arc<NativeCore>) {
    let root = std::env::temp_dir().join(format!("win11-core-test-{}", crate::events::nonce()));
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events,
        root.join("config"),
        root.join("cache"),
        root.join("ui"),
        "explorer".into(),
        dry,
    )
    .unwrap();
    let core = NativeCore::build(ctx, Arc::new(|_| {}), false).unwrap();
    (root, core)
}
fn result(r: ApiResult) -> Value {
    match r {
        Ok(v) => json!({"ok":v}),
        Err(e) => {
            // reference-tests.json predates C2's key/params (round 2 adds them): a catalogue error must rebuild its
            // message from key + params, and the oracle compares the fields it recorded (code, message, data).
            let mut error = e.json();
            if let Some(key) = e.key.as_deref() {
                let params = e.params.clone().unwrap_or_else(|| json!({}));
                assert!(
                    crate::i18n::Lang::ALL
                        .iter()
                        .any(|lang| crate::i18n::trf(*lang, key, &params) == e.message),
                    "{key} does not render {:?}",
                    e.message
                );
                let object = error.as_object_mut().unwrap();
                object.shift_remove("key");
                object.shift_remove("params");
            }
            json!({"error":error})
        }
    }
}
pub(super) fn finish(root: PathBuf, core: Arc<NativeCore>) {
    core.shutdown();
    core.ctx.events.shutdown();
    let _ = fs::remove_dir_all(root);
}
#[test]
fn native_open_validates_and_dry_run_never_invokes_launcher() {
    let (root, core) = fixture(true);
    *core.launcher.lock().unwrap() = Some(Arc::new(|_, _, _| panic!("dry-run must not launch")));
    assert_eq!(
        core.call(
            "shell.openApp",
            &json!({"app":"explorer","params":{"path":"~/Documents"}})
        )
        .unwrap()
        .unwrap(),
        json!({"ok":true,"dryRun":true})
    );
    assert_eq!(
        core.call("shell.openApp", &json!({"app":"unknown"}))
            .unwrap()
            .unwrap_err()
            .code,
        "unknown_app"
    );
    assert_eq!(
        core.call("shell.openApp", &json!({"app":"settings","params":[]}))
            .unwrap()
            .unwrap_err()
            .code,
        "bad_args"
    );
    assert_eq!(
        core.ctx.subscriber_count(),
        0,
        "backlog observers are not UI subscriptions"
    );
    finish(root, core);
}
#[test]
fn each_open_targets_an_independent_rust_host_and_keeps_literal_parameters() {
    let (root, core) = fixture(false);
    let launched = Arc::new(Mutex::new(vec![]));
    let list = launched.clone();
    *core.launcher.lock().unwrap() = Some(Arc::new(move |app, params, token| {
        list.lock()
            .unwrap()
            .push(json!({"app":app,"params":params,"token":token}));
        Ok(1234)
    }));
    let args = json!({"app":" Win11Shell-Explorer.desktop ","params":{"path":"/tmp/a $(touch no) `nothing`"},"activationToken":"fresh-token"});
    for _ in 0..2 {
        assert_eq!(
            core.call("shell.openApp", &args).unwrap().unwrap(),
            json!({"started":true,"pid":1234})
        );
    }
    assert_eq!(launched.lock().unwrap().len(), 2);
    assert_eq!(
        launched.lock().unwrap()[0],
        json!({"app":"explorer","params":args["params"],"token":"fresh-token"})
    );
    finish(root, core);
}
#[test]
fn gnome_clipboard_retains_pinned_file_and_reports_reference_unsupported_shape() {
    let (root, core) = fixture(true);
    let path = core.ctx.config_dir.join("clipboard-pinned.json");
    let original = r#"[{"id":"pinned","kind":"text","text":"preserve me","pinned":true}]"#;
    fs::write(&path, original).unwrap();
    for method in [
        "clipboard.paste",
        "clipboard.remove",
        "clipboard.clear",
        "clipboard.pin",
    ] {
        assert_eq!(
            core.call(method, &json!({"id":"pinned"})).unwrap().unwrap(),
            json!({"supported":false})
        );
    }
    assert_eq!(
        core.call("clipboard.list", &json!({})).unwrap().unwrap(),
        json!({"supported":false,"items":[]})
    );
    assert_eq!(fs::read_to_string(path).unwrap(), original);
    finish(root, core);
}
#[test]
fn shell_and_clipboard_errors_render_in_the_language_of_the_settings_file() {
    // R2-A: the shell and clipboard stems in English through their real call sites
    let (root, core) = fixture(true);
    fs::create_dir_all(&core.ctx.config_dir).unwrap();
    fs::write(
        core.ctx.config_dir.join("settings.json"),
        r#"{"language":"en"}"#,
    )
    .unwrap();
    let e = core
        .call("shell.command", &json!({"command":5}))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        e.json(),
        json!({"code":"unknown_command","message":"Unknown command.","key":"shell.unknownCommand","params":{},"data":{"command":5}})
    );
    let e = core
        .call("shell.openApp", &json!({"app":"nope"}))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("unknown_app", "This app doesn’t exist.")
    );
    let e = core
        .call("shell.openApp", &json!({"app":"settings","params":[]}))
        .unwrap()
        .unwrap_err();
    assert_eq!(
        (e.message.as_str(), e.key.as_deref()),
        ("Invalid input.", Some("shell.badArgs"))
    );
    let big = "x".repeat(64 * 1024);
    let e = core
        .call(
            "shell.openApp",
            &json!({"app":"settings","params":{"text":big}}),
        )
        .unwrap()
        .unwrap_err();
    assert_eq!(e.message, "Too much data.");
    let e = core
        .call("explorer.ready", &json!({}))
        .unwrap()
        .unwrap_err();
    assert_eq!(e.message, "Invalid input.");
    let mut history = clipboard::History::new(core.ctx.config_dir.join("clipboard-test.json"));
    let e = history.pin("gone", true, core.ctx.lang()).unwrap_err();
    assert_eq!(
        e.json(),
        json!({"code":"not_found","message":"This item is no longer on the clipboard.","key":"clipboard.notFound","params":{}})
    );
    // and Dutch again after a switch (the 3b898e6 bytes)
    fs::write(
        core.ctx.config_dir.join("settings.json"),
        r#"{"language":"nl"}"#,
    )
    .unwrap();
    let e = core
        .call("shell.command", &json!({"command":5}))
        .unwrap()
        .unwrap_err();
    assert_eq!(e.message, "Onbekende opdracht.");
    assert_eq!(
        history
            .pin("gone", true, core.ctx.lang())
            .unwrap_err()
            .message,
        "Dit item staat niet meer in het klembord."
    );
    finish(root, core);
}
