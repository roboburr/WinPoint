use super::*;
use std::collections::{BTreeMap, BTreeSet};
fn reference() -> Value {
    serde_json::from_str(include_str!("../reference-tests.json")).unwrap()
}
#[test]
fn commands_match_original_validation_payload_and_delivery_count() {
    let (root, core) = fixture(true);
    let _stream = core.ctx.events.subscribe();
    let emitted = Arc::new(Mutex::new(vec![]));
    let events = emitted.clone();
    core.ctx.events.on(
        "shell.command",
        Arc::new(move |v| events.lock().unwrap().push(json!(["shell.command", v]))),
    );
    for case in reference()["commands"].as_array().unwrap() {
        emitted.lock().unwrap().clear();
        assert_eq!(
            result(core.command(&case["args"])),
            case["expected"],
            "command {}",
            case["args"]
        );
        assert_eq!(json!(*emitted.lock().unwrap()), case["events"]);
    }
    finish(root, core);
}
#[test]
fn app_aliases_and_parameter_size_match_python() {
    let cases = reference();
    for case in cases["apps"].as_array().unwrap() {
        assert_eq!(
            json!(own_app(&case["input"])),
            case["expected"],
            "app {case}"
        );
    }
    for case in cases["params"].as_array().unwrap() {
        assert_eq!(
            result(clean_params(&case["input"], Lang::Nl)),
            case["expected"],
            "params length {}",
            case["input"].to_string().len()
        );
    }
}
#[test]
fn gnome_clipboard_apis_match_all_python_unsupported_cases() {
    let (root, core) = fixture(true);
    for case in reference()["clipboard"].as_array().unwrap() {
        assert_eq!(
            result(
                core.call(case["method"].as_str().unwrap(), &case["args"])
                    .unwrap()
            ),
            case["expected"]
        );
    }
    finish(root, core);
}
#[test]
fn filemanager_name_policy_limits_and_activation_tokens_match_python() {
    let cases = reference();
    for case in cases["policy"].as_array().unwrap() {
        assert_eq!(
            json!(filemanager::policy(
                case["mode"].as_str().unwrap(),
                case["compositor"].as_str().unwrap(),
                case["force"].as_str().unwrap(),
                case["full"].as_str().unwrap()
            )),
            case["expected"],
            "policy {case}"
        );
    }
    for case in cases["tokens"].as_array().unwrap() {
        assert_eq!(
            json!(filemanager::clean_token(&case["input"])),
            case["expected"]
        );
    }
    for case in cases["uris"].as_array().unwrap() {
        let result = match filemanager::check_uris(&case["input"]) {
            Ok(v) => json!({"ok":v}),
            Err(v) => json!({"error":v}),
        };
        assert_eq!(result, case["expected"], "URI validation {case}");
    }
}
#[test]
fn filemanager_uri_grouping_selection_and_limits_match_original_explorer_args() {
    let cases = reference();
    let e = &cases["env"];
    let strings = |v: &Value| {
        v.as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect::<BTreeSet<_>>()
    };
    let env = params::Env {
        cwd: string(&e["cwd"]),
        home: string(&e["home"]),
        hostname: string(&e["hostname"]),
        dirs: Some(strings(&e["dirs"])),
        exists: strings(&e["exists"]),
        remote: e["remote"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), string(v)))
            .collect(),
        gio: false,
    };
    for case in cases["explorer"].as_array().unwrap() {
        let uris = case["uris"]
            .as_array()
            .unwrap()
            .iter()
            .map(string)
            .collect::<Vec<_>>();
        assert_eq!(
            params::explorer(
                &uris,
                truth(&case["items"]),
                truth(&case["properties"]),
                &env
            )
            .unwrap_or(Value::Null),
            case["expected"],
            "Explorer uris {:?} items={} properties={}",
            uris.iter().take(5).collect::<Vec<_>>(),
            case["items"],
            case["properties"]
        );
    }
}
#[test]
fn explorer_backlog_matches_original_ack_reconnect_close_limit_and_expiry_trace() {
    let (root, core) = fixture(true);
    let emitted = Arc::new(Mutex::new(vec![]));
    let records = emitted.clone();
    core.ctx.events.on(
        "explorer.backlog",
        Arc::new(move |data| {
            let mut data = data.clone();
            data["boot"] = json!("BOOT");
            records
                .lock()
                .unwrap()
                .push(json!(["explorer.backlog", data]));
        }),
    );
    for case in reference()["backlog"].as_array().unwrap() {
        emitted.lock().unwrap().clear();
        match case["op"].as_str().unwrap() {
            "nav" => core.ctx.emit("app.navigate", case["args"].clone()),
            "close" => core.ctx.emit("shell.appClosed", case["args"].clone()),
            "wait" => core.backlog.age(case["args"].as_u64().unwrap()),
            "ready" => assert_eq!(
                result(core.backlog.ready(&case["args"])),
                case["expected"],
                "backlog {case}"
            ),
            _ => panic!(),
        };
        assert_eq!(
            json!(*emitted.lock().unwrap()),
            case["events"],
            "backlog events {case}"
        );
    }
    assert_eq!(core.ctx.subscriber_count(), 0);
    finish(root, core);
}
fn normalize(v: &Value, ids: &BTreeMap<String, String>) -> Value {
    match v {
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        if k == "time" {
                            json!(0)
                        } else if k == "id" {
                            json!(ids.get(&string(v)).cloned().unwrap_or_else(|| string(v)))
                        } else {
                            normalize(v, ids)
                        },
                    )
                })
                .collect(),
        ),
        Value::Array(a) => json!(a.iter().map(|v| normalize(v, ids)).collect::<Vec<_>>()),
        _ => v.clone(),
    }
}
#[test]
fn pinned_clipboard_history_and_storage_match_python_trace() {
    let (root, core) = fixture(true);
    let cases = reference();
    let path = core.ctx.config_dir.join("clipboard-pinned.json");
    fs::write(&path, cases["historyInitial"].to_string()).unwrap();
    let mut history = clipboard::History::new(path.clone());
    let mut ids = BTreeMap::<String, String>::new();
    for case in cases["history"].as_array().unwrap() {
        let resolve = |name: &Value| {
            ids.iter()
                .find(|(_, value)| **value == string(name))
                .map(|(key, _)| key.clone())
                .unwrap_or_else(|| string(name))
        };
        let actual = match case["op"].as_str().unwrap() {
            "public" => Ok(history.public()),
            "raw" => {
                let (raw, mime) =
                    clipboard::History::raw(history.get(&string(&case["args"])).unwrap()).unwrap();
                Ok(json!([raw, mime]))
            }
            "pin" => history.pin(
                &resolve(&case["args"][0]),
                truth(&case["args"][1]),
                Lang::Nl,
            ),
            "remove" => Ok(json!(history.remove(&resolve(&case["args"])))),
            "clear" => Ok(json!(history.clear())),
            "add" => {
                let item = history.add("text", case["args"].as_str(), &[], "image/png");
                let next = ids.len() + 1;
                ids.entry(string(&item["id"]))
                    .or_insert(format!("NEW{next}"));
                Ok(item)
            }
            _ => panic!(),
        };
        assert_eq!(
            normalize(&result(actual), &ids),
            case["expected"],
            "history {case}"
        );
        assert_eq!(normalize(&history.public(), &ids), case["public"]);
        let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(normalize(&stored, &ids), case["stored"], "persisted {case}");
    }
    finish(root, core);
}
#[test]
fn shell_info_keeps_original_user_apps_and_runtime_metadata_shape() {
    let (root, core) = fixture(true);
    let info = core.info();
    assert_eq!(info["user"], reference()["user"]);
    assert_eq!(
        info["apps"],
        json!([
            "settings",
            "taskmgr",
            "explorer",
            "alles",
            "disks",
            "defaultapps",
            "installer"
        ])
    );
    assert_eq!(info["compositor"], "gnome");
    assert_eq!(info["mode"], "app");
    assert!(info["services"].is_object());
    assert_eq!(info["dryRun"], true);
    assert!(info["runId"].as_str().is_some_and(|s| !s.is_empty()));
    // the WebKit hosts' GPU switches are gone: GTK 4 renders the native programs
    assert_eq!(info["gpu"], json!({"mode":"native"}));
    core.set_runtime_info(json!({"mainLoopLagMs":7,"runId":"new-run"}));
    assert_eq!(core.info()["mainLoopLagMs"], 7);
    assert_eq!(core.info()["runId"], "new-run");
    finish(root, core);
}
