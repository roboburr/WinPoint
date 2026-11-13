//! Runs only inside the isolated fixture bus started by work/native-disks-test/run_private.py.
use super::*;
use std::{
    io::{BufRead, BufReader},
    sync::mpsc,
};
const CONTROL: &str = "org.example.Win11DisksFixture";
const CONTROL_PATH: &str = "/org/example/Win11DisksFixture";
fn control(method: &str, value: Option<&str>) -> Value {
    let args = value.map(|s| (s,).to_variant());
    let bus = system().unwrap();
    let reply = bus
        .call_sync(
            Some(CONTROL),
            CONTROL_PATH,
            CONTROL,
            method,
            args.as_ref(),
            None,
            gio::DBusCallFlags::NONE,
            5000,
            gio::Cancellable::NONE,
        )
        .unwrap();
    first(&reply)
}
fn calls() -> Value {
    serde_json::from_str(control("Calls", None).as_str().unwrap()).unwrap()
}
#[test]
#[ignore = "Requires work/native-disks-test/run_private.py isolated UDisks bus"]
fn native_disks_use_private_udisks_protocol() {
    let root = PathBuf::from(
        std::env::var_os("WIN11_NATIVE_DISKS_TEST_ROOT").expect("isolated fixture required"),
    );
    assert_eq!(root.parent(), Some(std::path::Path::new("/tmp")));
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("win11-native-disks-")
    );
    let scenarios: Value =
        serde_json::from_slice(&fs::read(root.join("expected.json")).unwrap()).unwrap();
    let mut checks = vec![];
    for (i, case) in arr(&scenarios).iter().enumerate() {
        control("Reset", case["scenario"].as_str());
        let area = root.join(format!("case-{i}"));
        let ctx = context(&area, "en", truth(&case["dry"]));
        // The original Python ROOT fixture and native UI root must resolve through the same synthetic mount.
        let ctx = ServiceContext::new(
            ctx.events.clone(),
            ctx.config_dir.clone(),
            ctx.cache_dir.clone(),
            root.join("ui"),
            "disks".into(),
            ctx.dry_run,
        )
        .unwrap();
        let opened = Arc::new(Mutex::new(Vec::<Value>::new()));
        let launched = opened.clone();
        ctx.set_caller(Arc::new(move |method, args| {
            assert_eq!(method, "shell.openApp");
            launched.lock().unwrap().push(args.clone());
            Ok(json!({"ok":true}))
        }));
        let mut svc = fixture(ctx);
        Arc::get_mut(&mut svc).unwrap().readonly = truth(&case["readonly"]);
        let name = format!("disks.{}", string(&case["method"]));
        let actual = result(
            svc.call(&name, &case["args"])
                .expect("all original disk methods routed"),
        );
        assert_eq!(
            actual, case["expected"],
            "response scenario {i}: {} {name}",
            case["scenario"]
        );
        assert_eq!(
            calls(),
            case["calls"],
            "typed D-Bus actions scenario {i}: {} {name}",
            case["scenario"]
        );
        assert_eq!(
            json!(*opened.lock().unwrap()),
            case["opened"],
            "Explorer launch scenario {i}"
        );
        if truth(&case["dry"]) {
            assert!(
                arr(&calls())
                    .iter()
                    .all(|c| c["method"] == "SmartGetAttributes")
            );
        }
        svc.shutdown();
        svc.ctx.events.shutdown();
        checks.push(format!(
            "{} {name} dry={} readonly={}",
            case["scenario"], case["dry"], case["readonly"]
        ));
    }
    // A job-backed Format that outlives its D-Bus reply must retain the drive lock until its job disappears.
    control("Reset", Some("still-running"));
    let area = root.join("jobs");
    let mut svc = fixture(context(&area, "en", false));
    Arc::get_mut(&mut svc).unwrap().readonly = false;
    svc.job_timeout.store(75, Ordering::Relaxed);
    let args = json!({"id":format!("{PATH}/block_devices/sdb1"),"expect":{"device":"/dev/sdb1","size":128*MIB},"fsType":"ext4"});
    assert_eq!(
        svc.call("disks.format", &args).unwrap().unwrap_err().code,
        "still_running"
    );
    assert_eq!(
        svc.call("disks.format", &args).unwrap().unwrap_err().code,
        "op_running"
    );
    assert!(
        svc.ops
            .lock()
            .unwrap()
            .contains(&format!("{PATH}/drives/Fixture"))
    );
    control("FinishJob", None);
    let until = Instant::now() + Duration::from_secs(7);
    while !svc.ops.lock().unwrap().is_empty() && Instant::now() < until {
        thread::sleep(Duration::from_millis(25));
    }
    assert!(svc.ops.lock().unwrap().is_empty());
    svc.shutdown();
    svc.ctx.events.shutdown();
    checks.push(
        "Timed-out format remains still_running and retains one-per-drive lock until job completes"
            .into(),
    );
    // Real signal subscription / debounce emits the unchanged disks.changed wire payload.
    control("Reset", Some("normal"));
    let area = root.join("signals");
    let ctx = context(&area, "en", false);
    let events = ctx.events.clone();
    let svc = Disks::new(ctx).unwrap();
    let until = Instant::now() + Duration::from_secs(3);
    while svc.last.lock().unwrap().is_null() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!svc.last.lock().unwrap().is_null());
    let stream = events.subscribe();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { break };
            if line.contains("Changed by fixture") {
                let _ = tx.send(line);
                break;
            }
        }
    });
    control("Emit", None);
    let event = rx
        .recv_timeout(Duration::from_secs(3))
        .expect("UDisks PropertiesChanged delivered as disks.changed");
    assert!(event.contains("Changed by fixture"));
    svc.shutdown();
    events.shutdown();
    checks.push("UDisks signal watch refreshes and emits disks.changed after debounce".into());
    assert!(!root.join("UNSAFE").exists());
    fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":checks.len(),"checks":checks,"realHardwareMutations":0}),
        )
        .unwrap(),
    )
    .unwrap();
}
