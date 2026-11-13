use super::*;
use std::fs;
mod private_bus_tests;
fn reference() -> Value {
    serde_json::from_str(include_str!("reference-tests.json")).unwrap()
}
fn options(v: &Value) -> model::ParseOptions {
    model::ParseOptions {
        language: string(&v["language"]),
        shell_paths: arr(&v["shell_paths"]).iter().map(string).collect(),
        swap_files: arr(&v["swap_files"]).iter().map(string).collect(),
        boot_entries: arr(&v["boot_entries"]).to_vec(),
        slaves: v["slaves"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.parse().unwrap(), arr(v).iter().map(string).collect()))
            .collect(),
        usage: v["usage"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), (int(&v[0]), int(&v[1]), int(&v[2]))))
            .collect(),
        available_tools: arr(&v["available_tools"]).iter().map(string).collect(),
    }
}
fn result(value: ApiResult) -> Value {
    match value {
        Ok(v) => json!({"ok":v}),
        Err(e) => json!({"error":e.json()}),
    }
}
#[test]
fn managed_objects_match_all_python_partition_and_protection_cases() {
    for case in arr(&reference()["models"]) {
        let actual = model::parse(&case["objects"], &options(&case["options"]));
        assert_eq!(actual, case["expected"], "model {}", case["name"]);
    }
}
#[test]
fn labels_match_python_unicode_limits_and_errors() {
    for case in arr(&reference()["labels"]) {
        assert_eq!(
            result(
                labels::check_label(
                    &string(&case["language"]),
                    &string(&case["fs"]),
                    &case["label"]
                )
                .map(Value::String)
            ),
            case["expected"],
            "label {case}"
        );
    }
}
#[test]
fn free_space_matches_python_alignment_and_logical_gaps() {
    for case in arr(&reference()["gaps"]) {
        let parts = arr(&case["parts"])
            .iter()
            .map(|p| (int(&p[0]), int(&p[1])))
            .collect::<Vec<_>>();
        let container = case["container"]
            .as_array()
            .map(|v| (int(&v[0]), int(&v[1])));
        assert_eq!(
            json!(model::free_regions(
                int(&case["size"]),
                case["table"].as_str(),
                &parts,
                container
            )),
            case["expected"],
            "gap {case}"
        );
    }
}
#[test]
fn firmware_load_options_match_python_gpt_and_mbr() {
    for case in arr(&reference()["efi"]) {
        let raw = arr(&case["raw"])
            .iter()
            .map(|v| int(v) as u8)
            .collect::<Vec<_>>();
        assert_eq!(
            efi::parse_load_option(&raw).unwrap_or(Value::Null),
            case["expected"],
            "EFI {case}"
        );
    }
}
#[test]
fn smart_summary_and_details_match_python() {
    for case in arr(&reference()["smart"]) {
        let actual = match case["kind"].as_str().unwrap() {
            "summary" => model::smart_summary(&case["input"]),
            "nvme" => model::nvme_health(&case["input"]),
            "ata" => model::ata_attributes(&case["input"]),
            _ => panic!(),
        };
        assert_eq!(actual, case["expected"], "SMART {case}");
    }
}
#[test]
fn dbus_errors_match_python_in_both_languages() {
    for case in arr(&reference()["errors"]) {
        assert_eq!(
            map_error(
                &string(&case["language"]),
                &string(&case["input"]),
                "Fixture"
            )
            .json(),
            case["expected"],
            "error {case}"
        );
    }
}
fn context(root: &std::path::Path, lang: &str, dry: bool) -> Arc<ServiceContext> {
    let config = root.join(lang);
    fs::create_dir_all(&config).unwrap();
    fs::write(
        config.join("settings.json"),
        json!({"language":lang}).to_string(),
    )
    .unwrap();
    ServiceContext::new(
        crate::events::Events::new(&root.join(format!("events-{lang}"))).unwrap(),
        config,
        root.join("cache"),
        root.join("ui"),
        "disks".into(),
        dry,
    )
    .unwrap()
}
fn fixture(ctx: Arc<ServiceContext>) -> Arc<Disks> {
    Arc::new(Disks {
        ctx,
        readonly: true,
        stopped: AtomicBool::new(false),
        snap: Mutex::new(None),
        last: Mutex::new(Value::Null),
        pending: Mutex::new(None),
        boot: Mutex::new(None),
        stats: StatCache::new(),
        ops: Arc::new(Mutex::new(BTreeSet::new())),
        job_timeout: std::sync::atomic::AtomicI32::new(i32::MAX),
    })
}
#[test]
fn action_guards_match_python_without_bus_or_disk_writes() {
    let root = std::env::temp_dir().join(format!("win11-disks-guards-{}", crate::events::nonce()));
    let services = BTreeMap::from([
        ("nl", fixture(context(&root, "nl", true))),
        ("en", fixture(context(&root, "en", true))),
    ]);
    let cases = reference();
    let models = arr(&cases["models"])
        .iter()
        .map(|c| {
            (
                string(&c["name"]),
                model::parse(&c["objects"], &options(&c["options"])),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for case in arr(&cases["guards"]) {
        let name = string(&case["model"]);
        let svc = &services[if name.ends_with("-en") { "en" } else { "nl" }];
        let op = case["op"].as_str().unwrap();
        let args = &case["args"];
        let snap = &models[&name];
        let actual = if [
            "mount",
            "unmount",
            "open",
            "setLabel",
            "format",
            "deletePartition",
            "unlock",
            "lock",
        ]
        .contains(&op)
        {
            svc.volume_checks(snap, op, args)
                .map(|(_, v)| json!({"id":v["id"],"device":v["device"]}))
        } else {
            svc.disk_checks(snap, op, args)
                .map(|d| json!({"disk":d["id"],"device":d["device"]}))
        };
        assert_eq!(
            result(actual),
            case["expected"],
            "guard {} {} {}",
            name,
            op,
            args
        );
    }
    for svc in services.values() {
        svc.ctx.events.shutdown();
    }
    let _ = fs::remove_dir_all(root);
}
#[test]
fn drive_operation_lock_releases_and_serializes_shared_namespaces() {
    let root = std::env::temp_dir().join(format!("win11-disks-ops-{}", crate::events::nonce()));
    let svc = fixture(context(&root, "en", true));
    let first = svc
        .operation(vec!["drive-a".into(), "drive-b".into()])
        .unwrap();
    assert_eq!(
        svc.operation(vec!["drive-b".into()]).err().unwrap().code,
        "op_running"
    );
    let other = svc.operation(vec!["drive-c".into()]).unwrap();
    drop(first);
    assert!(svc.operation(vec!["drive-a".into()]).is_ok());
    drop(other);
    assert!(svc.ops.lock().unwrap().is_empty());
    svc.ctx.events.shutdown();
    let _ = fs::remove_dir_all(root);
}

// R2-B: disks.list's 1 s snapshot is kept per language (round2-gate §2; GE1a's ge1a-disks-race showed
// cachedKeepsOldLanguage: true), and the en render test of the disks stem through its call site.
#[test]
fn the_snapshot_is_used_only_in_the_language_it_was_built_in() {
    let root = std::env::temp_dir().join(format!("win11-disks-lang-{}", crate::events::nonce()));
    let svc = fixture(context(&root, "nl", true));
    let marker = json!({"supported":true,"marker":"nl snapshot"});
    *svc.snap.lock().unwrap() = Some((Instant::now(), Lang::Nl, marker.clone()));
    assert_eq!(
        svc.snapshot(false).unwrap(),
        marker,
        "same language, younger than 1 s: the cache"
    );
    // an atomic rename to English, as the settings store writes it
    let settings = svc.ctx.config_dir.join("settings.json");
    let temp = svc.ctx.config_dir.join("settings.json.new");
    fs::write(&temp, json!({"language":"en"}).to_string()).unwrap();
    fs::rename(&temp, &settings).unwrap();
    assert_eq!(svc.ctx.lang(), Lang::En);
    let after = svc.snapshot(false).unwrap();
    assert_ne!(
        after, marker,
        "a switch within the second must not serve the Dutch snapshot"
    );
    let stored = svc
        .snap
        .lock()
        .unwrap()
        .as_ref()
        .map(|(_, lang, snap)| (*lang, snap.clone()));
    assert_eq!(
        stored,
        Some((Lang::En, after.clone())),
        "the new snapshot is kept as English"
    );
    assert_eq!(
        svc.snapshot(false).unwrap(),
        after,
        "and is the cache in English"
    );
    svc.ctx.events.shutdown();
    let _ = fs::remove_dir_all(root);
}
#[test]
fn the_unavailable_body_does_not_repeat_its_heading_in_english_and_keeps_dutch() {
    let missing = "GDBus.Error:org.freedesktop.DBus.Error.ServiceUnknown: missing";
    for (lang, body) in [
        (
            Lang::Nl,
            "Schijfbeheer werkt niet: UDisks is op deze computer niet beschikbaar.",
        ),
        (Lang::En, "UDisks isn’t available on this computer."),
    ] {
        let error = map_error(lang.code(), missing, "");
        assert_eq!(error.code, "unavailable");
        let snap = unsupported(lang, error.clone());
        assert_eq!(snap["supported"], false);
        assert_eq!(snap["error"], body, "{lang:?}");
        // Dutch: the body is the API message of 3b898e6, byte for byte
        if lang == Lang::Nl {
            assert_eq!(snap["error"], json!(error.message));
        }
    }
    // the English API message stays as the reference tests pin it; only the body under the heading changed
    assert_eq!(
        map_error("en", missing, "").message,
        "Disk management doesn't work: UDisks isn't available on this computer."
    );
    // other errors keep their own message in the body
    let timeout = map_error(
        "en",
        "GDBus.Error:org.freedesktop.DBus.Error.NoReply: timeout",
        "",
    );
    assert_eq!(
        unsupported(Lang::En, timeout.clone())["error"],
        json!(timeout.message)
    );
}
// Review m2: disks.list through Disks::call in English, and a language switch within the snapshot's second (the
// unit form of ge1a-disks-race). run-tests.sh points DBUS_SYSTEM_BUS_ADDRESS at a socket that does not exist, so
// the reply is the unsupported one built from the bus error; outside that sandbox a live UDisks could answer, so
// the test stops there.
#[test]
fn disks_list_follows_a_language_switch_through_the_service() {
    let no_bus = std::env::var("DBUS_SYSTEM_BUS_ADDRESS")
        .ok()
        .and_then(|a| a.strip_prefix("unix:path=").map(PathBuf::from))
        .is_some_and(|p| !p.exists());
    if !no_bus {
        eprintln!(
            "skipped: a system bus may be reachable (run it through tests/rust-sandbox/run-tests.sh)"
        );
        return;
    }
    let bus_error = system().unwrap_err();
    let expect = |lang: Lang| unsupported(lang, map_error(lang.code(), &bus_error, ""));
    let root = std::env::temp_dir().join(format!("win11-disks-call-{}", crate::events::nonce()));
    let svc = fixture(context(&root, "nl", false));
    let list = || {
        svc.call("disks.list", &json!({}))
            .expect("disks.list is routed")
            .expect("disks.list answers with supported: false")
    };
    let nl = list();
    assert_eq!(nl, expect(Lang::Nl));
    assert_eq!(nl["supported"], false);
    // an atomic rename to English right after the Dutch snapshot (well within its second)
    let temp = svc.ctx.config_dir.join("settings.json.new");
    fs::write(&temp, json!({"language":"en"}).to_string()).unwrap();
    fs::rename(&temp, svc.ctx.config_dir.join("settings.json")).unwrap();
    let en = list();
    assert_eq!(en, expect(Lang::En));
    assert_ne!(en["error"], nl["error"]);
    assert_ne!(en["filesystems"], nl["filesystems"]);
    // no socket: glib's "Could not connect" maps to "failed", whose English text leads the body
    if map_error("en", &bus_error, "").code == "failed" {
        let prefix = tr("en", "failed", &[]);
        let prefix = prefix.split("{detail}").next().unwrap();
        assert!(
            !prefix.is_empty() && en["error"].as_str().unwrap().starts_with(prefix),
            "{en}"
        );
    }
    svc.ctx.events.shutdown();
    let _ = fs::remove_dir_all(root);
}
