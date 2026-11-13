//! The runner supplies a private session bus and private filesystem. Every launch is a fixture callback.
use super::*;
use crate::services::system::common::{call as dbus_call, first, session};
use glib::variant::ToVariant;
use std::time::Instant;
const NAME: &str = "org.freedesktop.FileManager1";
fn name_owner(bus: &gio::DBusConnection) -> Option<String> {
    dbus_call(
        bus,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "GetNameOwner",
        Some(&(NAME,).to_variant()),
        1000,
    )
    .ok()
    .and_then(|v| first(&v).as_str().map(str::to_owned))
}
fn wait(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(4);
    while !condition() && Instant::now() < until {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(condition(), "private protocol condition timed out");
}
#[test]
#[ignore = "Requires work/native-core-test/run_private.py isolated session bus"]
fn native_core_uses_private_filemanager_protocol_and_events() {
    let root = PathBuf::from(
        std::env::var_os("WIN11_NATIVE_CORE_TEST_ROOT").expect("isolated fixture required"),
    );
    assert_eq!(root.parent(), Some(std::path::Path::new("/tmp")));
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("win11-native-core-")
    );
    let expected: Value =
        serde_json::from_slice(&fs::read(root.join("expected.json")).unwrap()).unwrap();
    let bus = session().unwrap();
    let other = gio::DBusConnection::for_address_sync(
        &std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    dbus_call(
        &other,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(NAME, 0u32).to_variant()),
        1000,
    )
    .unwrap();
    let original = name_owner(&bus).unwrap();
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        root.join("config"),
        root.join("cache"),
        root.join("ui"),
        "panel".into(),
        false,
    )
    .unwrap();
    let core = NativeCore::build(ctx, Arc::new(|_| {}), false).unwrap();
    let (tx, rx) = mpsc::channel();
    *core.launcher.lock().unwrap() = Some(Arc::new(move |app, params, token| {
        tx.send(json!({"app":app,"params":params,"token":token}))
            .unwrap();
        Ok(1234)
    }));
    core.filemanager.start();
    thread::sleep(Duration::from_millis(150));
    assert_eq!(
        name_owner(&bus).as_deref(),
        Some(original.as_str()),
        "force-own queues and never replaces another file manager"
    );
    dbus_call(
        &other,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ReleaseName",
        Some(&(NAME,).to_variant()),
        1000,
    )
    .unwrap();
    wait(|| name_owner(&bus).as_deref() == bus.unique_name().as_deref());
    let mut checks = vec![
        "Forced FileManager1 waits behind an existing owner and acquires only after release"
            .to_owned(),
    ];
    for case in expected["filemanager"].as_array().unwrap() {
        let uris = case["uris"]
            .as_array()
            .unwrap()
            .iter()
            .map(string)
            .collect::<Vec<_>>();
        let token = string(&case["token"]);
        dbus_call(
            &other,
            NAME,
            "/org/freedesktop/FileManager1",
            NAME,
            case["method"].as_str().unwrap(),
            Some(&(uris, token).to_variant()),
            2000,
        )
        .unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(4)).unwrap(),
            case["expected"],
            "FileManager1 {case}"
        );
        checks.push(format!(
            "{} path/selection/properties and activation token match Python",
            case["method"]
        ));
    }
    for uris in [
        vec!["bad\nuri".to_owned()],
        vec!["x".into(); 1001],
        vec!["é".repeat(8193)],
    ] {
        let error = dbus_call(
            &other,
            NAME,
            "/org/freedesktop/FileManager1",
            NAME,
            "ShowItems",
            Some(&(uris, "").to_variant()),
            2000,
        )
        .unwrap_err();
        assert!(
            error.contains("org.freedesktop.DBus.Error.InvalidArgs"),
            "{error}"
        );
    }
    checks.push(
        "Malformed and oversized FileManager1 requests return typed InvalidArgs before launching"
            .into(),
    );
    // ALLOW_REPLACEMENT lets an explicitly replacing app take the name; we wait and regain it later.
    let replacer = gio::DBusConnection::for_address_sync(
        &std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap(),
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None,
        gio::Cancellable::NONE,
    )
    .unwrap();
    dbus_call(
        &replacer,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(NAME, 6u32).to_variant()),
        1000,
    )
    .unwrap();
    wait(|| name_owner(&bus) == replacer.unique_name().map(|v| v.to_string()));
    dbus_call(
        &replacer,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "ReleaseName",
        Some(&(NAME,).to_variant()),
        1000,
    )
    .unwrap();
    wait(|| name_owner(&bus).as_deref() == bus.unique_name().as_deref());
    checks.push(
        "An explicit replacement is respected and the native service reacquires from the queue"
            .into(),
    );
    // A second Rust host's Unix event reaches the backlog observer without opening another SSE subscription.
    let peer = crate::events::Events::new(&root.join("events")).unwrap();
    let recorded = Arc::new(Mutex::new(vec![]));
    let list = recorded.clone();
    events.on(
        "explorer.backlog",
        Arc::new(move |v| list.lock().unwrap().push(v.clone())),
    );
    peer.emit(
        "app.navigate",
        json!({"app":"explorer","params":{"path":"/fixture/from-peer"}}),
    );
    wait(|| {
        recorded.lock().unwrap().clear();
        core.backlog.ready(&json!({"id":"peer-page"})).unwrap();
        recorded.lock().unwrap().iter().any(|v| {
            v["navigate"]
                .as_array()
                .unwrap()
                .iter()
                .any(|n| n["params"]["path"] == "/fixture/from-peer")
        })
    });
    assert_eq!(events.subscriber_count(), 0);
    checks.push("Cross-host app.navigate is recovered through explorer.ready without a phantom UI subscriber".into());
    assert_eq!(core.clipboard.reason, "gnome");
    assert_eq!(
        core.call("clipboard.list", &json!({})).unwrap().unwrap(),
        json!({"supported":false,"items":[]})
    );
    assert!(!root.join("UNSAFE").exists());
    checks.push("GNOME clipboard reports unsupported and never starts wl-paste or wl-copy".into());
    core.shutdown();
    wait(|| name_owner(&bus).is_none());
    events.shutdown();
    peer.shutdown();
    checks.push("Shutdown unregisters and releases only its own FileManager1 name".into());
    fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(
            &json!({"passed":checks.len(),"checks":checks,"realAppLaunches":0,"userBusChanges":0}),
        )
        .unwrap(),
    )
    .unwrap();
}
