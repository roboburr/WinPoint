//! Explicitly invoked protocol integration test; the runner supplies an isolated bus and commands.
use super::*;
use crate::events::Events;
use glib::variant::ToVariant;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    sync::mpsc,
};
#[test]
#[ignore = "Requires work/native-system-test/run_private.py private D-Bus fixtures"]
fn native_system_uses_private_protocols() {
    let root = PathBuf::from(
        std::env::var_os("WIN11_NATIVE_SYSTEM_TEST_ROOT").expect("private fixture root required"),
    );
    assert_eq!(root.parent(), Some(std::path::Path::new("/tmp")));
    assert!(
        root.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("win11-native-system-")
    );
    let expected: Value =
        serde_json::from_slice(&fs::read(root.join("expected.json")).unwrap()).unwrap();
    let events = Events::new(&root.join("events")).unwrap();
    let ctx = ServiceContext::new(
        events.clone(),
        root.join("config/win11-gnome"),
        root.join("cache/win11-gnome"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vendor/ui"),
        "panel".into(),
        false,
    )
    .unwrap();
    ctx.set_caller(Arc::new(|method, _| {
        if method == "apps.list" {
            Ok(json!([{"id":"org.example.FixtureMusic.desktop","icon":"fixture-mpris-symbolic"}]))
        } else {
            Err(ApiError::new("not_found", "fixture method unavailable"))
        }
    }));
    let svc = System::new(ctx.clone()).unwrap();
    let media = crate::services::media::Media::new(ctx.clone()).unwrap();
    let display = crate::services::display::Display::new(ctx.clone()).unwrap();
    let call = |name: &str, args: Value| {
        svc.call(name, &args)
            .expect("system method exists")
            .unwrap()
    };
    let mut checks = vec![];
    assert_eq!(call("system.audio.get", json!({})), expected["audio"]);
    checks.push("PipeWire volume and device payload matches Python oracle");
    assert_eq!(call("system.network.get", json!({})), expected["network"]);
    checks.push("NetworkManager nested object and active connection state matches Python oracle");
    assert_eq!(
        call("system.network.wifiList", json!({"rescan":false})),
        expected["wifi"]
    );
    checks.push("nmcli escaped SSIDs and signal/security ordering matches Python oracle");
    assert_eq!(call("system.battery.get", json!({})), expected["battery"]);
    checks.push("UPower percentage, health, energy and optional fields match Python oracle");
    assert_eq!(
        display.call("display.list", &json!({})).unwrap().unwrap(),
        expected["display"]
    );
    checks.push("Mutter state parsed as same read-only display contract");
    let players = media.call("media.list", &json!({})).unwrap().unwrap();
    let mut player = players[0].clone();
    player.as_object_mut().unwrap().remove("updated");
    assert_eq!(player, expected["player"]);
    checks.push("MPRIS nested metadata, capabilities and catalogue icon match Python oracle");
    assert_eq!(
        media
            .call(
                "media.control",
                &json!({"id":"org.mpris.MediaPlayer2.fixture","action":"playPause"})
            )
            .unwrap()
            .unwrap(),
        json!({"ok":true})
    );
    assert_eq!(
        media
            .call(
                "media.control",
                &json!({"id":"org.mpris.MediaPlayer2.missing","action":"play"})
            )
            .unwrap()
            .unwrap_err()
            .code,
        "not_found"
    );
    checks.push("MPRIS actual control and closed-player error");
    let audio = call("system.audio.set", json!({"volume":0.5}));
    assert_eq!(audio["available"], true);
    let commands = fs::read_to_string(root.join("commands")).unwrap();
    assert!(commands.contains("set-volume -l 1.0 @DEFAULT_AUDIO_SINK@ 0.50"));
    assert!(commands.contains("set-mute @DEFAULT_AUDIO_SINK@ 0"));
    checks.push(
        "Audio slider coerces percent and automatically unmutes through literal command arguments",
    );
    assert_eq!(
        call("system.network.connect", json!({"ssid":"Fixture Wi-Fi"}))["code"],
        "needs_password"
    );
    assert_eq!(
        call(
            "system.network.connect",
            json!({"ssid":"Fixture Wi-Fi","password":"short"})
        )["code"],
        "wrong_password"
    );
    assert_eq!(
        call(
            "system.network.connect",
            json!({"ssid":"Fixture Wi-Fi","password":"12345678"})
        ),
        json!({"ok":true})
    );
    checks.push(
        "Wi-Fi validation and real typed D-Bus activation without password command arguments",
    );
    assert_eq!(
        call("system.bluetooth.discover", json!({"on":true})),
        json!({"ok":true})
    );
    let bt = call("system.bluetooth.get", json!({}));
    assert_eq!(bt["nearby"][0]["name"], "Test keyboard");
    assert_eq!(bt["nearby"][0]["kind"], "keyboard");
    checks.push("BlueZ discovery filter/session and nearby device shape");
    let stream = events.subscribe();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut name = String::new();
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            if let Some(n) = line.strip_prefix("event: ") {
                name = n.into();
            } else if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(value) = serde_json::from_str::<Value>(data) {
                    let _ = tx.send((name.clone(), value));
                }
            }
        }
    });
    let wait = |stage: &str| -> Value {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            let (name, v) = rx
                .recv_timeout(end.saturating_duration_since(Instant::now()))
                .expect("pairing event in bounded time");
            if name == "system.bluetooth.pairing" && v["stage"] == stage {
                return v;
            }
        }
    };
    let pair_svc = svc.clone();
    let (pair_tx, pair_rx) = mpsc::sync_channel(1);
    let pair = thread::spawn(move || {
        pair_tx
            .send(
                pair_svc
                    .call(
                        "system.bluetooth.pair",
                        &json!({"address":"AA:BB:CC:DD:EE:FF"}),
                    )
                    .unwrap(),
            )
            .unwrap();
    });
    let question = wait("confirm");
    assert_eq!(question["passkey"], "123456");
    let stale = call(
        "system.bluetooth.pairReply",
        json!({"address":"AA:BB:CC:DD:EE:FF","accept":true,"qid":question["qid"].as_u64().unwrap()+1}),
    );
    assert_eq!(stale["code"], "stale");
    assert!(pair_rx.try_recv().is_err());
    checks.push("Temporary BlueZ agent holds confirmation and refuses stale question IDs");
    let bus = system().unwrap();
    let calls: Value = serde_json::from_slice(&fs::read(root.join("calls.json")).unwrap()).unwrap();
    let path = calls
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|r| r["method"] == "org.bluez.AgentManager1.RegisterAgent")
        .unwrap()["args"][0]
        .as_str()
        .unwrap();
    let own = bus.unique_name().unwrap();
    let spoof = common::call(
        &bus,
        &own,
        path,
        "org.bluez.Agent1",
        "RequestPinCode",
        Some(&(object_path("/org/bluez/hci0/dev_AA_BB_CC_DD_EE_FF").unwrap(),).to_variant()),
        1500,
    );
    assert!(spoof.is_err_and(|e| e.contains("Rejected")));
    checks.push("Agent refuses pairing requests from clients other than BlueZ owner");
    assert_eq!(
        call(
            "system.bluetooth.pairReply",
            json!({"address":"AA:BB:CC:DD:EE:FF","accept":true,"qid":question["qid"],"passkey":"123456"})
        ),
        json!({"ok":true})
    );
    let paired = pair_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(paired, json!({"ok":true,"paired":true,"connected":true}));
    pair.join().unwrap();
    checks.push("Pair confirmation → trust → connect complete with original result and events");
    assert_eq!(
        call(
            "system.bluetooth.remove",
            json!({"address":"AA:BB:CC:DD:EE:FF"})
        ),
        json!({"ok":true})
    );
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        let calls: Value =
            serde_json::from_slice(&fs::read(root.join("calls.json")).unwrap()).unwrap();
        if calls
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["method"] == "org.bluez.AgentManager1.UnregisterAgent")
        {
            break;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(20));
    }
    checks.push("Temporary agent unregisters after completion");
    let pair_svc = svc.clone();
    let (pair_tx, pair_rx) = mpsc::sync_channel(1);
    let pair = thread::spawn(move || {
        pair_tx
            .send(
                pair_svc
                    .call(
                        "system.bluetooth.pair",
                        &json!({"address":"AA:BB:CC:DD:EE:FF"}),
                    )
                    .unwrap(),
            )
            .unwrap();
    });
    let _ = wait("confirm");
    assert_eq!(
        call(
            "system.bluetooth.cancelPair",
            json!({"address":"AA:BB:CC:DD:EE:FF"})
        ),
        json!({"ok":true})
    );
    let cancelled = pair_rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap();
    assert_eq!(cancelled["cancelled"], true);
    assert_eq!(cancelled["code"], "cancelled");
    pair.join().unwrap();
    checks.push("Cancelling a held pairing rejects question, cancels own Pair and returns original cancellation shape");
    let dry = ServiceContext::new(
        events.clone(),
        root.join("dry-config"),
        root.join("dry-cache"),
        ctx.ui.clone(),
        "settings".into(),
        true,
    )
    .unwrap();
    let dry_svc = System::new(dry).unwrap();
    let before = fs::read(root.join("commands")).unwrap();
    for name in [
        "system.power.action",
        "system.bluetooth.setPowered",
        "system.network.setWifi",
        "system.openUpdater",
        "system.nightLight.set",
    ] {
        assert_eq!(
            dry_svc
                .call(
                    name,
                    &json!({"action":"poweroff","enabled":false,"powered":false})
                )
                .unwrap()
                .unwrap(),
            json!({"ok":true,"dryRun":true})
        );
    }
    assert_eq!(fs::read(root.join("commands")).unwrap(), before);
    checks.push(
        "Dry-run mutating APIs skip all hardware and command actions before argument execution",
    );
    dry_svc.shutdown();
    media.shutdown();
    display.shutdown();
    svc.shutdown();
    events.shutdown();
    reader.join().unwrap();
    fs::write(
        root.join("results.json"),
        serde_json::to_vec_pretty(&json!({"groups":checks.len(),"passed":true,"checks":checks}))
            .unwrap(),
    )
    .unwrap();
}
