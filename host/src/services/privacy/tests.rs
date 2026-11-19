//! privacy.* tests: the pure parts on fixtures (PermissionStore values, Flatpak metadata, snapd answers, global
//! shortcut variants, globs and mimeapps.list, autostart files), and the service against fakes: a PermissionStore
//! and a GlobalShortcutsRebind on tests/rust-sandbox's private session bus (calls recorded with their interactive
//! flag), a snapd on a socket of its own (requests recorded with their headers). The tests that touch GIO's app
//! index, GSettings or files under HOME run only in the sandbox (keyfile GSettings, private HOME and buses).
use super::*;
use gio::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, mpsc};

/// Tests that write `~/.config/mimeapps.list` (here and apps/defaults.rs) take turns: GIO rewrites the whole file.
pub(crate) static MIMEAPPS: Mutex<()> = Mutex::new(());

/// tests/rust-sandbox/run-tests.sh's environment, or None (never the live session): keyfile GSettings, a private
/// session bus, and HOME, XDG_DATA_HOME, XDG_CONFIG_HOME and the system-tool directory inside `$SANDBOX`. Keyfile
/// GSettings alone is not enough: a hand run with the real HOME would write the user's own mimeapps.list, desktop
/// files and Trash. Batch F's tests that touch GIO's app index, GSettings or files under HOME all ask this.
pub(crate) fn sandbox() -> Option<String> {
    sandbox_in(|name| std::env::var(name).ok())
}

/// [`sandbox`] for an environment given by `var` (the private session bus's address when it is the sandbox).
fn sandbox_in(var: impl Fn(&str) -> Option<String>) -> Option<String> {
    let root = std::path::PathBuf::from(var("SANDBOX")?);
    let address = var("DBUS_SESSION_BUS_ADDRESS")?;
    let inside = |name: &str, dir: &str| {
        var(name).is_some_and(|value| std::path::Path::new(&value) == root.join(dir))
    };
    (root.is_absolute()
        && var("GSETTINGS_BACKEND").as_deref() == Some("keyfile")
        && inside("HOME", "home")
        && inside("XDG_DATA_HOME", "data")
        && inside("XDG_CONFIG_HOME", "config")
        && inside("WIN11_SYSTEM_TOOL_DIR", "guard-bin")
        && !address.contains("/run/user/"))
    .then_some(address)
}

#[test]
fn the_sandbox_guard_needs_all_of_run_tests_environment() {
    let root = "/srv/g/tests/rust-sandbox";
    let full: HashMap<&str, String> = [
        ("SANDBOX", root.to_owned()),
        ("GSETTINGS_BACKEND", "keyfile".to_owned()),
        ("HOME", format!("{root}/home")),
        ("XDG_DATA_HOME", format!("{root}/data")),
        ("XDG_CONFIG_HOME", format!("{root}/config")),
        ("WIN11_SYSTEM_TOOL_DIR", format!("{root}/guard-bin")),
        (
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/tmp/dbus-private".to_owned(),
        ),
    ]
    .into();
    let with = |change: &[(&'static str, Option<&str>)]| {
        let mut env = full.clone();
        for (name, value) in change {
            match value {
                Some(v) => env.insert(name, (*v).to_owned()),
                None => env.remove(name),
            };
        }
        sandbox_in(move |name| env.get(name).cloned())
    };
    assert_eq!(with(&[]).as_deref(), Some("unix:path=/tmp/dbus-private"));
    // keyfile GSettings by hand, with the real HOME and the live bus: never the sandbox
    assert_eq!(
        sandbox_in(|name| match name {
            "GSETTINGS_BACKEND" => Some("keyfile".to_owned()),
            "HOME" => Some("/home/robin".to_owned()),
            "DBUS_SESSION_BUS_ADDRESS" => Some("unix:path=/tmp/private".to_owned()),
            _ => None,
        }),
        None
    );
    for name in [
        "SANDBOX",
        "GSETTINGS_BACKEND",
        "HOME",
        "XDG_DATA_HOME",
        "XDG_CONFIG_HOME",
        "WIN11_SYSTEM_TOOL_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ] {
        assert_eq!(with(&[(name, None)]), None, "without {name}");
    }
    for (name, value) in [
        ("HOME", "/home/robin"),
        ("XDG_DATA_HOME", "/home/robin/.local/share"),
        ("XDG_CONFIG_HOME", "/home/robin/.config"),
        ("WIN11_SYSTEM_TOOL_DIR", "/usr/bin"),
        ("GSETTINGS_BACKEND", "dconf"),
        ("DBUS_SESSION_BUS_ADDRESS", "unix:path=/run/user/1000/bus"),
        ("SANDBOX", "tests/rust-sandbox"),
    ] {
        assert_eq!(with(&[(name, Some(value))]), None, "{name}={value}");
    }
}

/// A temporary directory that goes with the test (tests/rust-sandbox counts what tests leave in TMPDIR).
struct Temp(std::path::PathBuf);

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn context(dry_run: bool) -> (Arc<ServiceContext>, Arc<Mutex<Vec<Value>>>, Temp) {
    let root = std::env::temp_dir().join(format!("privacy-{}", crate::events::nonce()));
    std::fs::create_dir_all(&root).unwrap();
    let events = crate::events::Events::new(&root.join("events")).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    events.on(
        "privacy.changed",
        Arc::new(move |data: &Value| record.lock().unwrap().push(data.clone())),
    );
    let ctx = ServiceContext::new(
        events,
        root.join("config"),
        root.join("cache"),
        root.clone(),
        "settings".into(),
        dry_run,
    )
    .unwrap();
    (ctx, seen, Temp(root))
}

fn error_of(result: Option<ApiResult>) -> ApiError {
    result.expect("handled").expect_err("an error")
}

// ----------------------------------------------------------------------------------------------- pure parts

#[test]
fn permission_values_follow_gnome_settings() {
    let location = perms::kind("location", "location").unwrap();
    let camera = perms::kind("devices", "camera").unwrap();
    let inhibit = perms::kind("gnome", "shortcuts-inhibitor").unwrap();
    assert!(
        perms::kind("gnome", "shortcuts-inhibit").is_none(),
        "amendment 7: the id is shortcuts-inhibitor"
    );
    assert!(
        perms::kind("notifications", "notification").is_none(),
        "only the entries GNOME Settings shows"
    );
    // true/false become GNOME's own values; location keeps its timestamp
    assert_eq!(
        perms::parse_value(camera, &json!(false), None).unwrap(),
        ["no"]
    );
    assert_eq!(
        perms::parse_value(inhibit, &json!(true), None).unwrap(),
        ["GRANTED"]
    );
    let stamp = vec!["EXACT".to_owned(), "1700000000".to_owned()];
    assert_eq!(
        perms::parse_value(location, &json!(false), Some(&stamp)).unwrap(),
        ["NONE", "1700000000"]
    );
    assert_eq!(
        perms::parse_value(location, &json!(true), None).unwrap(),
        ["EXACT", "0"]
    );
    assert_eq!(
        perms::parse_value(location, &json!(["CITY", "12"]), None).unwrap(),
        ["CITY", "12"]
    );
    for bad in [
        json!("maybe"),
        json!(["yes", "1"]),
        json!([]),
        json!(3),
        json!([true]),
    ] {
        assert!(perms::parse_value(camera, &bad, None).is_none(), "{bad}");
    }
    assert!(perms::parse_value(location, &json!(["EXACT", "soon"]), None).is_none());
    // nothing stored counts as allowed; only the "no" value refuses
    assert!(perms::allowed(camera, None));
    assert!(!perms::allowed(camera, Some(&["no".to_owned()])));
    assert!(perms::allowed(location, Some(&stamp)));
    assert!(!perms::allowed(inhibit, Some(&["DENIED".to_owned()])));
    for (app, ok) in [
        ("org.gnome.Maps", true),
        ("snap.firefox", true),
        ("", false),
        (".x", false),
        ("a/b", false),
        ("a b", false),
    ] {
        assert_eq!(perms::app_id_ok(app), ok, "{app}");
    }
}

#[test]
fn a_lookup_answer_lists_every_app_with_its_value() {
    let mut map = HashMap::new();
    map.insert(
        "org.gnome.Maps".to_owned(),
        vec!["EXACT".to_owned(), "0".to_owned()],
    );
    map.insert(
        "snap.firefox".to_owned(),
        vec!["NONE".to_owned(), "5".to_owned()],
    );
    let parsed = perms::parse_lookup(&map.to_variant());
    assert_eq!(
        parsed[0],
        (
            "org.gnome.Maps".to_owned(),
            vec!["EXACT".to_owned(), "0".to_owned()]
        )
    );
    assert_eq!(parsed.len(), 2);
    assert!(perms::parse_lookup(&"text".to_variant()).is_empty());
    let entries = vec![
        (perms::kind("location", "location").unwrap(), parsed),
        (perms::kind("background", "background").unwrap(), vec![]),
    ];
    let rows = perms::app_rows(&entries, "snap.firefox");
    assert_eq!(rows.len(), 1, "background is a section of its own");
    assert_eq!(rows[0]["key"], "location");
    assert_eq!(
        (rows[0]["set"].clone(), rows[0]["allowed"].clone()),
        (json!(true), json!(false))
    );
    assert_eq!(
        perms::app_rows(&entries, "org.example.Other")[0]["set"],
        false
    );
}

const METADATA: &str = "[Application]\nname=org.example.Editor\nruntime=org.gnome.Platform/x86_64/48\n\n[Context]\n\
shared=network;ipc;\nsockets=x11;wayland;pulseaudio;session-bus;\ndevices=dri;all;\nfilesystems=home:ro;host;xdg-download;\n\n\
[Session Bus Policy]\nca.desrt.dconf=talk\norg.freedesktop.Notifications=talk\n";

#[test]
fn flatpak_metadata_gives_gnomes_static_permissions() {
    let rows = flatpak::static_permissions(METADATA);
    let keys: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                r["key"].as_str().unwrap().to_owned(),
                r["access"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let want = [
        ("sessionBus", "full"),
        ("devices", "full"),
        ("network", "full"),
        ("home", "read"),
        ("fileSystem", "full"),
        ("settings", "full"),
        ("sound", "full"),
    ];
    assert_eq!(
        keys,
        want.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect::<Vec<_>>()
    );
    assert!(flatpak::static_permissions("[Context]\nshared=ipc;\n").is_empty());
    assert!(flatpak::static_permissions("not a keyfile [").is_empty());
    for (id, ok) in [
        ("org.example.Editor", true),
        ("com.x_y.A-b", true),
        ("single", false),
        ("org..x", false),
        ("org.1x", false),
        ("org/x.y", false),
    ] {
        assert_eq!(flatpak::id_ok(id), ok, "{id}");
    }
    let xml = r#"<component><id>org.example.Editor</id><releases><release version="2.4.1" date="2026-10-01"/><release version="2.4.0"/></releases></component>"#;
    assert_eq!(flatpak::appstream_version(xml).as_deref(), Some("2.4.1"));
    assert_eq!(flatpak::appstream_version("<component/>"), None);
}

#[test]
fn a_flatpak_deploy_its_version_and_its_disk_use() {
    let root = std::env::temp_dir().join(format!("privacy-flatpak-{}", crate::events::nonce()));
    let deploy = root.join("system/app/org.example.Editor/current/active");
    std::fs::create_dir_all(deploy.join("files/share/metainfo")).unwrap();
    std::fs::write(deploy.join("metadata"), METADATA).unwrap();
    std::fs::write(
        deploy.join("files/share/metainfo/org.example.Editor.metainfo.xml"),
        r#"<component><releases><release version="3.0"/></releases></component>"#,
    )
    .unwrap();
    let roots = [root.join("user"), root.join("system")];
    assert_eq!(
        flatpak::deploy(&roots, "org.example.Editor"),
        Some(deploy.clone())
    );
    assert_eq!(flatpak::deploy(&roots, "org.example.Missing"), None);
    assert_eq!(
        flatpak::version(&deploy, "org.example.Editor").as_deref(),
        Some("3.0")
    );
    // hard links count once, the skipped directory not at all
    let home = root.join("home");
    let (data, cache) = {
        std::fs::create_dir_all(home.join(".var/app/org.example.Editor/cache/fontconfig")).unwrap();
        flatpak::data_dirs(&home, "org.example.Editor").unwrap()
    };
    let big = vec![7u8; 64 * 1024];
    std::fs::write(data.join("state"), &big).unwrap();
    std::fs::hard_link(data.join("state"), data.join("state-link")).unwrap();
    std::fs::write(cache.join("fontconfig/cache-1"), &big).unwrap();
    let mut walk = flatpak::Walk::new();
    let without_cache = walk.usage(&data, Some(&cache));
    let cache_use = walk.usage(&cache, None);
    assert!(
        without_cache >= 64 * 1024 && without_cache < 2 * 64 * 1024,
        "{without_cache}"
    );
    assert!(cache_use >= 64 * 1024, "{cache_use}");
    // a cache reached through a symlink is never followed or emptied
    let outside = root.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("keep"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, cache.join("link")).unwrap();
    let freed = flatpak::clear(&cache).unwrap();
    assert!(freed >= 64 * 1024);
    assert_eq!(
        std::fs::read_dir(&cache).unwrap().count(),
        0,
        "the cache is empty"
    );
    assert!(cache.is_dir(), "the directory itself stays");
    assert_eq!(std::fs::read(outside.join("keep")).unwrap(), b"keep");
    let linked = root.join("home2");
    std::fs::create_dir_all(linked.join(".var/app")).unwrap();
    std::os::unix::fs::symlink(&outside, linked.join(".var/app/org.example.Editor")).unwrap();
    assert!(
        flatpak::data_dirs(&linked, "org.example.Editor").is_none(),
        "a symlinked ~/.var/app/<id>"
    );
    let _ = std::fs::remove_dir_all(root);
}

fn http(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

const CONNECTIONS: &str = r#"{"type":"sync","status-code":200,"result":{
 "plugs":[
  {"snap":"spotify","plug":"camera","interface":"camera"},
  {"snap":"spotify","plug":"audio-playback","interface":"audio-playback","connections":[{"snap":"snapd","slot":"audio-playback"}]},
  {"snap":"spotify","plug":"desktop-legacy","interface":"desktop-legacy"},
  {"snap":"spotify","plug":"lonely","interface":"no-such-slot"},
  {"snap":"other","plug":"camera","interface":"camera"}],
 "slots":[
  {"snap":"snapd","slot":"camera","interface":"camera"},
  {"snap":"snapd","slot":"audio-playback","interface":"audio-playback"},
  {"snap":"pulseaudio","slot":"audio-playback","interface":"audio-playback"},
  {"snap":"snapd","slot":"desktop-legacy","interface":"desktop-legacy"}]}}"#;

#[test]
fn snapd_answers_and_a_snaps_plugs() {
    let v = snapd::parse_response(&http("200 OK", CONNECTIONS)).unwrap();
    let plugs = snapd::plugs(&v["result"], "spotify");
    let names: Vec<&str> = plugs.iter().map(|p| p["plug"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["audio-playback", "camera"],
        "hidden interfaces and plugs without a slot are left out"
    );
    assert_eq!(plugs[0]["connected"], true);
    assert_eq!(
        plugs[0]["slot"],
        json!({"snap": "snapd", "slot": "audio-playback"})
    );
    assert_eq!(plugs[0]["slots"].as_array().unwrap().len(), 2);
    assert_eq!(
        (plugs[1]["connected"].clone(), plugs[1]["slot"].clone()),
        (json!(false), Value::Null)
    );
    // chunked bodies, and snapd's errors with their kind
    let chunked = b"HTTP/1.1 202 Accepted\r\nTransfer-Encoding: chunked\r\n\r\n10\r\n{\"type\":\"async\",\r\n10\r\n\"change\":\"42\"}  \r\n0\r\n\r\n";
    assert_eq!(snapd::parse_response(chunked).unwrap()["change"], "42");
    let denied = http(
        "403 Forbidden",
        r#"{"type":"error","status-code":403,"result":{"message":"access denied","kind":"login-required"}}"#,
    );
    let e = snapd::parse_response(&denied).unwrap_err();
    assert_eq!(
        (e.status, e.kind.as_str(), e.message.as_str()),
        (403, "login-required", "access denied")
    );
    assert!(snapd::parse_response(b"HTTP/1.1 200 OK\r\n").is_err());
    assert_eq!(
        snapd::connect_body(false, "spotify", "camera", "snapd", "camera"),
        json!({"action":"disconnect","plugs":[{"snap":"spotify","plug":"camera"}],"slots":[{"snap":"snapd","slot":"camera"}]})
    );
    for (name, ok) in [
        ("firefox", true),
        ("snap-store", true),
        ("firefox_beta", true),
        ("Firefox", false),
        ("-x", false),
        ("a/b", false),
    ] {
        assert_eq!(snapd::name_ok(name), ok, "{name}");
    }
    assert!(snapd::port_ok("audio-playback") && !snapd::port_ok("a b") && !snapd::port_ok(""));
    // errors map to the polkit refusals call_interactive gives
    let lang = Lang::En;
    assert_eq!(
        snap_error(e, lang).key.as_deref(),
        Some("core.notAuthorized")
    );
    let cancelled = snapd::SnapError {
        status: 403,
        kind: "auth-cancelled".into(),
        message: String::new(),
    };
    assert_eq!(
        snap_error(cancelled, lang).key.as_deref(),
        Some("core.authCancelled")
    );
    let gone = snapd::SnapError {
        status: 0,
        kind: String::new(),
        message: "No such file or directory (os error 2)".into(),
    };
    assert_eq!(
        snap_error(gone, lang).key.as_deref(),
        Some("privacy.snapUnavailable")
    );
    let other = snapd::SnapError {
        status: 400,
        kind: String::new(),
        message: "no such plug".into(),
    };
    let e = snap_error(other, lang);
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("failed", "The snap interface wasn’t changed: no such plug")
    );
}

fn shortcut(id: &str, description: &str, accels: &[&str]) -> glib::Variant {
    let d = glib::VariantDict::new(None);
    d.insert_value("description", &description.to_variant());
    d.insert_value(
        "shortcuts",
        &accels
            .iter()
            .map(|a| a.to_string())
            .collect::<Vec<_>>()
            .to_variant(),
    );
    glib::Variant::tuple_from_iter([id.to_variant(), d.end()])
}

fn shortcut_list(items: &[glib::Variant]) -> glib::Variant {
    glib::Variant::array_from_iter_with_type(glib::VariantTy::new("(sa{sv})").unwrap(), items)
}

#[test]
fn global_shortcuts_read_and_rebind_as_the_portal_stores_them() {
    let value = shortcut_list(&[
        shortcut("record", "Start recording", &["<Super>r"]),
        shortcut("mute", "Mute", &[]),
    ]);
    let list = shortcuts::parse(&value);
    assert_eq!(list.len(), 2);
    let json = shortcuts::to_json(&list);
    assert_eq!(json[0]["id"], "record");
    assert_eq!(json[0]["description"], "Start recording");
    assert_eq!(json[0]["accels"], json!(["<Super>r"]));
    assert!(shortcuts::parse(&"x".to_variant()).is_empty());
    // a portal entry with a trigger string only
    let d = glib::VariantDict::new(None);
    d.insert_value("preferred_trigger", &"<Control>F9".to_variant());
    let trigger = d.end();
    assert_eq!(shortcuts::accels(&trigger), ["<Control>F9"]);
    let rebound = shortcuts::with_accels(&trigger, &["<Alt>F9".to_owned()]);
    assert_eq!(shortcuts::accels(&rebound), ["<Alt>F9"]);
    // the call: every entry, the changed one with its new accelerators, or none at all for Alles verwijderen
    let args = shortcuts::rebind_args(
        "org.example.Recorder",
        &list,
        &[("record".into(), vec!["<Super><Shift>r".into()])],
        false,
    );
    assert_eq!(args.type_().as_str(), "(sa(sa{sv}))");
    assert_eq!(args.child_value(0).str(), Some("org.example.Recorder"));
    let sent = shortcuts::parse(&args.child_value(1));
    assert_eq!(shortcuts::accels(&sent[0].dict), ["<Super><Shift>r"]);
    assert!(shortcuts::accels(&sent[1].dict).is_empty());
    let cleared = shortcuts::parse(&shortcuts::rebind_args("a.b", &list, &[], true).child_value(1));
    assert!(
        cleared
            .iter()
            .all(|s| shortcuts::accels(&s.dict).is_empty())
    );
    assert_eq!(
        glib::VariantDict::new(Some(&cleared[0].dict))
            .lookup_value("description", None)
            .and_then(|v| v.get::<String>()),
        Some("Start recording".to_owned()),
        "the rest of the entry is kept"
    );
    assert_eq!(
        shortcuts::path("org.example.Recorder").as_deref(),
        Some("/org/gnome/settings-daemon/global-shortcuts/org.example.Recorder/")
    );
    assert!(shortcuts::path("../x").is_none() && shortcuts::path("a//b").is_none());
    assert!(
        shortcuts::accel_ok("<Control><Alt>t")
            && !shortcuts::accel_ok("")
            && !shortcuts::accel_ok("a\nb")
    );
}

#[test]
fn globs_mimeapps_and_types() {
    let globs = "# comment\n50:text/html:*.html\n40:text/html:*.htm\n60:text/html:*.xhtml-not\n50:text/html:*.html\n\
                 10:image/png:*.png\n50:image/png:bad*\n50:text/x-readme:README\n";
    let ext = handlers::extensions(&[globs.to_owned()]);
    assert_eq!(ext["text/html"], [".xhtml-not", ".html", ".htm"]);
    assert_eq!(ext["image/png"], [".png"]);
    assert!(!ext.contains_key("text/x-readme"), "only *.ext globs");
    let mimeapps = "[Default Applications]\ntext/html=firefox.desktop;\nx-scheme-handler/tel=dialer.desktop;\n\
                    [Added Associations]\nimage/png=viewer.desktop;\n[Removed Associations]\ntext/plain=editor.desktop;\n[Other]\nkey=x\n";
    assert_eq!(
        handlers::user_types(mimeapps),
        [
            "image/png",
            "text/html",
            "text/plain",
            "x-scheme-handler/tel"
        ]
    );
    assert!(handlers::user_types("not [ a keyfile").is_empty());
    // a type's name in the settings language, from shared-mime-info's XML (GIO's own follows the process locale)
    let root = std::env::temp_dir().join(format!("privacy-mime-{}", crate::events::nonce()));
    std::fs::create_dir_all(root.join("application")).unwrap();
    std::fs::write(
        root.join("application/x-7z-compressed.xml"),
        r#"<?xml version="1.0"?><mime-type xmlns="http://www.freedesktop.org/standards/shared-mime-info" type="application/x-7z-compressed">
<comment>7-zip archive</comment><comment xml:lang="nl">7-zip-archief</comment><comment xml:lang="en_GB">7-zip archive (GB)</comment></mime-type>"#,
    )
    .unwrap();
    let roots = [root.join("missing"), root.clone()];
    assert_eq!(
        handlers::label("application/x-7z-compressed", Lang::Nl, &roots),
        "7-zip-archief"
    );
    assert_eq!(
        handlers::label("application/x-7z-compressed", Lang::En, &roots),
        "7-zip archive (GB)"
    );
    let _ = std::fs::remove_dir_all(root);
    for (t, ok) in [
        ("text/html", true),
        ("x-scheme-handler/tel", true),
        ("text", false),
        ("a/b/c", false),
        ("text/ html", false),
    ] {
        assert_eq!(handlers::type_ok(t), ok, "{t}");
    }
}

#[test]
fn autostart_files_and_search_providers() {
    let base = "# a comment\n[Desktop Entry]\nName=Editor\nExec=editor\nX-GNOME-Autostart-enabled=false\n\n[Desktop Action new]\nName=New\n";
    let on = app::autostart_text(base, true, false);
    assert!(on.contains("X-GNOME-Autostart-enabled=true") && on.contains("Hidden=false"));
    assert!(on.contains("# a comment") && on.contains("[Desktop Action new]\nName=New"));
    assert_eq!(on.matches("X-GNOME-Autostart-enabled").count(), 1);
    assert!(app::starts(&on) && !app::starts(base));
    let generated = app::autostart_text("[Desktop Entry]\nName=A\nHidden=true\n", true, true);
    assert!(
        generated.contains("X-GNOME-Settings-Autostart-Generated=true") && app::starts(&generated)
    );
    assert_eq!(
        app::set_desktop_key("Name=x\n", "K", "v"),
        "Name=x\n[Desktop Entry]\nK=v\n"
    );
    for (id, ok) in [
        ("org.gnome.Calculator.desktop", true),
        ("firefox_firefox.desktop", true),
        (".desktop", false),
        ("a/b.desktop", false),
        ("x", false),
    ] {
        assert_eq!(app::desktop_id_ok(id), ok, "{id}");
    }
    let root = std::env::temp_dir().join(format!("privacy-search-{}", crate::events::nonce()));
    let dir = root.join("gnome-shell/search-providers");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.ini"), "[Shell Search Provider]\nDesktopId=org.example.Notes.desktop\nBusName=x\nDefaultDisabled=true\n").unwrap();
    std::fs::write(
        dir.join("b.ini"),
        "[Shell Search Provider]\nDesktopId=org.example.Files.desktop\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("c.txt"),
        "[Shell Search Provider]\nDesktopId=org.example.Text.desktop\n",
    )
    .unwrap();
    let dirs = [root.clone()];
    assert_eq!(
        app::search_provider("org.example.Notes.desktop", &dirs).unwrap()["defaultDisabled"],
        true
    );
    assert_eq!(
        app::search_provider("org.example.Files.desktop", &dirs).unwrap()["defaultDisabled"],
        false
    );
    assert!(app::search_provider("org.example.Text.desktop", &dirs).is_none());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn bad_arguments_are_refused_in_the_settings_language() {
    let (ctx, _, _ctx_root) = context(false);
    let service = Privacy::with(ctx.clone(), Targets::default());
    for (method, args) in [
        (
            "privacy.permissions",
            json!({"table": "devices", "id": "speakers"}),
        ),
        (
            "privacy.setPermission",
            json!({"table": "devices", "id": "camera", "app": "a/b", "value": true}),
        ),
        (
            "privacy.deletePermission",
            json!({"table": "location", "id": "location", "app": 5}),
        ),
        ("privacy.app", json!({"id": "../etc/passwd"})),
        ("privacy.app", json!([])),
        (
            "privacy.app.removeHandler",
            json!({"id": 5, "type": "text/plain"}),
        ),
        (
            "privacy.app.setAutostart",
            json!({"id": "a/b.desktop", "on": true}),
        ),
        ("privacy.reports.set", json!({"mode": "always"})),
        ("privacy.reports.set", json!({})),
        (
            "privacy.insights.set",
            json!({"source": "kernel", "consent": true}),
        ),
        (
            "privacy.insights.set",
            json!({"source": "linux", "consent": "yes"}),
        ),
        (
            "privacy.insights.report",
            json!({"source": "ubuntu_release_upgrader"}),
        ),
    ] {
        let e = error_of(service.call(method, &args));
        assert_eq!(
            (e.code.as_str(), e.key.as_deref(), e.message.as_str()),
            ("bad_args", Some("privacy.badArgs"), "Ongeldige invoer."),
            "{method}"
        );
    }
    std::fs::create_dir_all(&ctx.config_dir).unwrap();
    std::fs::write(ctx.config_dir.join("settings.json"), r#"{"language":"en"}"#).unwrap();
    let e = error_of(service.call(
        "privacy.app",
        &json!({"id": "no-such-app-anywhere.desktop"}),
    ));
    assert_eq!(
        (e.code.as_str(), e.message.as_str()),
        ("not_found", "This app wasn’t found.")
    );
    assert!(
        service.call("privacy.nothing", &json!({})).is_none(),
        "an unknown method is not this service's"
    );
}

// --------------------------------------------------------------------------------------- fakes and the service

type Calls = Arc<Mutex<Vec<(String, String, bool)>>>;

/// A D-Bus object on its own connection to the private session bus, answered by `reply` (method, params).
struct Fake {
    name: String,
    calls: Calls,
    stop: Arc<AtomicBool>,
    context: glib::MainContext,
    thread: Option<std::thread::JoinHandle<()>>,
}

type Reply = Box<
    dyn Fn(&str, &glib::Variant) -> Result<Option<glib::Variant>, (String, String)> + Send + Sync,
>;

/// A fake's properties: the value of a property by name.
type Props = Box<dyn Fn(&str) -> glib::Variant + Send + Sync>;

impl Fake {
    fn start(
        address: String,
        xml: &'static str,
        path: &'static str,
        iface: &'static str,
        reply: Reply,
    ) -> Fake {
        Fake::start_with(
            address,
            xml,
            path,
            iface,
            reply,
            Box::new(|_| ().to_variant()),
        )
    }

    /// [`Fake::start`] with properties (Get and GetAll answer `props`).
    fn start_with(
        address: String,
        xml: &'static str,
        path: &'static str,
        iface: &'static str,
        reply: Reply,
        props: Props,
    ) -> Fake {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (thread_stop, record) = (stop.clone(), calls.clone());
        let reply = Arc::new(reply);
        let thread = std::thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let connection = gio::DBusConnection::for_address_sync(
                    &address,
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    gio::Cancellable::NONE,
                )
                .unwrap();
                let node = gio::DBusNodeInfo::for_xml(xml).unwrap();
                let info = node.lookup_interface(iface).unwrap();
                let id = connection
                    .register_object(
                        path,
                        &info,
                        move |_, _, _, _, method, params, invocation| {
                            let interactive = invocation
                                .message()
                                .flags()
                                .contains(gio::DBusMessageFlags::ALLOW_INTERACTIVE_AUTHORIZATION);
                            record.lock().unwrap().push((
                                method.to_owned(),
                                params.print(true).to_string(),
                                interactive,
                            ));
                            match reply(method, &params) {
                                Ok(value) => invocation.return_value(value.as_ref()),
                                Err((name, message)) => {
                                    invocation.return_dbus_error(&name, &message)
                                }
                            }
                        },
                        move |_, _, _, _, name| props(name),
                        |_, _, _, _, _, _| false,
                    )
                    .unwrap();
                ready
                    .send((
                        connection.unique_name().unwrap().to_string(),
                        context.clone(),
                    ))
                    .unwrap();
                while !thread_stop.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                connection.unregister_object(id).unwrap();
            });
        });
        let (name, context) = started.recv().unwrap();
        Fake {
            name,
            calls,
            stop,
            context,
            thread: Some(thread),
        }
    }

    fn calls(&self) -> Vec<(String, String, bool)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Drop for Fake {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

const STORE_XML: &str = r#"<node><interface name="org.freedesktop.impl.portal.PermissionStore">
<method name="Lookup"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sas}" direction="out"/><arg type="v" direction="out"/></method>
<method name="SetPermission"><arg type="s" direction="in"/><arg type="b" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="as" direction="in"/></method>
<method name="DeletePermission"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="s" direction="in"/></method>
<method name="Delete"><arg type="s" direction="in"/><arg type="s" direction="in"/></method>
</interface></node>"#;

type Store = Arc<Mutex<BTreeMap<(String, String), HashMap<String, Vec<String>>>>>;

/// A PermissionStore that keeps its entries in memory.
fn permission_store(address: String) -> (Fake, Store) {
    let store: Store = Arc::new(Mutex::new(BTreeMap::new()));
    let data = store.clone();
    let not_found = || {
        Err((
            "org.freedesktop.portal.Error.NotFound".to_owned(),
            "No entry".to_owned(),
        ))
    };
    let reply: Reply = Box::new(move |method, params| {
        let mut data = data.lock().unwrap();
        match method {
            "Lookup" => {
                let (table, id) = params.get::<(String, String)>().unwrap();
                match data.get(&(table, id)) {
                    Some(apps) => Ok(Some(glib::Variant::tuple_from_iter([
                        apps.to_variant(),
                        glib::Variant::from_variant(&"".to_variant()),
                    ]))),
                    None => not_found(),
                }
            }
            "SetPermission" => {
                let (table, _create, id, app, values) = params
                    .get::<(String, bool, String, String, Vec<String>)>()
                    .unwrap();
                data.entry((table, id)).or_default().insert(app, values);
                Ok(None)
            }
            "DeletePermission" => {
                let (table, id, app) = params.get::<(String, String, String)>().unwrap();
                match data.get_mut(&(table, id)) {
                    Some(apps) => {
                        apps.remove(&app);
                        Ok(None)
                    }
                    None => not_found(),
                }
            }
            _ => {
                let (table, id) = params.get::<(String, String)>().unwrap();
                data.remove(&(table, id));
                Ok(None)
            }
        }
    });
    let path = "/org/freedesktop/impl/portal/PermissionStore";
    (
        Fake::start(
            address,
            STORE_XML,
            path,
            "org.freedesktop.impl.portal.PermissionStore",
            reply,
        ),
        store,
    )
}

#[test]
fn permissions_go_to_the_permission_store_without_a_password() {
    let Some(address) = sandbox() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let (fake, store) = permission_store(address);
    store.lock().unwrap().insert(
        ("location".into(), "location".into()),
        HashMap::from([(
            "org.gnome.Maps".to_owned(),
            vec!["EXACT".to_owned(), "1700000000".to_owned()],
        )]),
    );
    let targets = || Targets {
        store: fake.name.clone(),
        ..Targets::default()
    };
    // a dry run validates and answers, and writes nothing
    let (dry, dry_events, _dry_root) = context(true);
    let service = Privacy::with(dry, targets());
    let r = service
        .call(
            "privacy.setPermission",
            &json!({"table":"location","id":"location","app":"org.gnome.Maps","value":false}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(r["value"], json!(["NONE", "1700000000"]));
    assert!(
        fake.calls().iter().all(|(m, _, _)| m == "Lookup"),
        "{:?}",
        fake.calls()
    );
    assert!(dry_events.lock().unwrap().is_empty());
    assert!(
        service
            .call(
                "privacy.deletePermission",
                &json!({"table":"location","id":"location","app":"org.gnome.Maps"})
            )
            .unwrap()
            .is_ok()
    );
    assert_eq!(store.lock().unwrap().len(), 1);
    // the real thing: the value, the event, never ALLOW_INTERACTIVE_AUTHORIZATION (the store has no polkit)
    let (ctx, events, _ctx_root) = context(false);
    let service = Privacy::with(ctx, targets());
    service
        .call(
            "privacy.setPermission",
            &json!({"table":"location","id":"location","app":"org.gnome.Maps","value":false}),
        )
        .unwrap()
        .unwrap();
    service
        .call("privacy.setPermission", &json!({"table":"gnome","id":"shortcuts-inhibitor","app":"org.example.Game","value":true}))
        .unwrap()
        .unwrap();
    let saved = store.lock().unwrap().clone();
    assert_eq!(
        saved[&("location".to_owned(), "location".to_owned())]["org.gnome.Maps"],
        ["NONE", "1700000000"]
    );
    assert_eq!(
        saved[&("gnome".to_owned(), "shortcuts-inhibitor".to_owned())]["org.example.Game"],
        ["GRANTED"]
    );
    let calls = fake.calls();
    let set: Vec<&(String, String, bool)> =
        calls.iter().filter(|c| c.0 == "SetPermission").collect();
    assert_eq!(set.len(), 2);
    assert_eq!(
        set[0].1,
        "('location', true, 'location', 'org.gnome.Maps', ['NONE', '1700000000'])"
    );
    assert!(calls.iter().all(|c| !c.2), "no interactive authorization");
    assert_eq!(
        events.lock().unwrap()[0],
        json!({"what":"permissions","table":"location","id":"location","app":"org.gnome.Maps"})
    );
    // privacy.permissions lists the entry; a missing entry is empty, not an error
    let list = service
        .call(
            "privacy.permissions",
            &json!({"table":"location","id":"location"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(list["available"], true);
    assert_eq!(
        list["apps"],
        json!([{"app":"org.gnome.Maps","value":["NONE","1700000000"],"allowed":false}])
    );
    let empty = service
        .call(
            "privacy.permissions",
            &json!({"table":"devices","id":"camera"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        (empty["available"].clone(), empty["apps"].clone()),
        (json!(true), json!([]))
    );
    service
        .call(
            "privacy.deletePermission",
            &json!({"table":"gnome","id":"shortcuts-inhibitor","app":"org.example.Game"}),
        )
        .unwrap()
        .unwrap();
    assert!(
        store.lock().unwrap()[&("gnome".to_owned(), "shortcuts-inhibitor".to_owned())].is_empty()
    );
    // no PermissionStore on the bus: privacy.permissions says so, a write is unsupported
    let (ctx, _, _ctx_root) = context(false);
    let none = Privacy::with(
        ctx,
        Targets {
            store: "org.example.NoSuchStore".into(),
            ..Targets::default()
        },
    );
    let list = none
        .call(
            "privacy.permissions",
            &json!({"table":"devices","id":"camera"}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(list["available"], false);
    let e = error_of(none.call(
        "privacy.setPermission",
        &json!({"table":"devices","id":"camera","app":"a.b","value":true}),
    ));
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("unsupported", Some("privacy.storeUnavailable"))
    );
}

/// A desktop file in the private XDG_DATA_HOME, once GIO's app index has it.
fn install_app(id: &str, body: &str) -> Option<std::path::PathBuf> {
    let dir = glib::user_data_dir().join("applications");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join(id);
    std::fs::write(&path, body).ok()?;
    for _ in 0..150 {
        if gio::DesktopAppInfo::new(id).is_some() {
            return Some(path);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    None
}

const REBIND_XML: &str = r#"<node><interface name="org.gnome.GlobalShortcutsRebind">
<method name="RebindShortcuts"><arg type="s" direction="in"/><arg type="a(sa{sv})" direction="in"/></method>
</interface></node>"#;

#[test]
fn global_shortcuts_are_read_natively_and_rebound_through_the_portal() {
    let Some(address) = sandbox() else {
        eprintln!("skipped: no private session bus");
        return;
    };
    let source = gio::SettingsSchemaSource::default();
    if source
        .and_then(|s| {
            s.lookup(
                "org.gnome.settings-daemon.global-shortcuts.application",
                true,
            )
        })
        .is_none()
    {
        eprintln!("skipped: GNOME 50's global-shortcuts schema is not installed");
        return;
    }
    let id = "org.example.Recorder.desktop";
    let Some(file) = install_app(
        id,
        "[Desktop Entry]\nType=Application\nName=Recorder\nExec=true\nX-Flatpak=org.example.Recorder\n",
    ) else {
        panic!("GIO did not see the test app");
    };
    let list = gio::Settings::new("org.gnome.settings-daemon.global-shortcuts");
    list.set_strv("applications", ["org.example.Recorder"])
        .unwrap();
    let app = gio::Settings::with_path(
        "org.gnome.settings-daemon.global-shortcuts.application",
        "/org/gnome/settings-daemon/global-shortcuts/org.example.Recorder/",
    );
    app.set_value(
        "shortcuts",
        &shortcut_list(&[shortcut("record", "Start recording", &["<Super>r"])]),
    )
    .unwrap();
    gio::Settings::sync();
    let reply: Reply = Box::new(|_, _| Ok(None));
    let fake = Fake::start(
        address,
        REBIND_XML,
        "/org/gnome/globalshortcuts",
        "org.gnome.GlobalShortcutsRebind",
        reply,
    );
    let (ctx, events, _ctx_root) = context(false);
    let service = Privacy::with(
        ctx,
        Targets {
            rebind: fake.name.clone(),
            store: "org.example.NoSuchStore".into(),
            ..Targets::default()
        },
    );
    let info = service
        .call("privacy.app", &json!({"id": id, "sizes": false}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (info["kind"].clone(), info["portalId"].clone()),
        (json!("flatpak"), json!("org.example.Recorder"))
    );
    assert_eq!(info["globalShortcuts"]["appId"], "org.example.Recorder");
    assert_eq!(
        info["globalShortcuts"]["shortcuts"][0]["accels"],
        json!(["<Super>r"])
    );
    assert_eq!(
        info["permissionsAvailable"], false,
        "no store on the bus: no permission rows"
    );
    // a dry run sends nothing; the real call sends every entry; an unknown action is refused
    let (dry, _, _dry_root) = context(true);
    let dry = Privacy::with(
        dry,
        Targets {
            rebind: fake.name.clone(),
            ..Targets::default()
        },
    );
    dry.call(
        "privacy.app.globalShortcuts.set",
        &json!({"id": id, "clear": true}),
    )
    .unwrap()
    .unwrap();
    assert!(fake.calls().is_empty());
    service
        .call(
            "privacy.app.globalShortcuts.set",
            &json!({"id": id, "shortcuts": [{"id": "record", "accels": ["<Super><Shift>r"]}]}),
        )
        .unwrap()
        .unwrap();
    let calls = fake.calls();
    assert_eq!(calls.len(), 1);
    assert!(
        calls[0]
            .1
            .starts_with("('org.example.Recorder', [('record', {"),
        "{}",
        calls[0].1
    );
    assert!(
        calls[0].1.contains("<['<Super><Shift>r']>") && !calls[0].2,
        "{:?}",
        calls[0]
    );
    assert_eq!(events.lock().unwrap().last().unwrap()["what"], "app");
    let e = error_of(service.call(
        "privacy.app.globalShortcuts.set",
        &json!({"id": id, "shortcuts": [{"id": "nope", "accels": []}]}),
    ));
    assert_eq!(e.code, "bad_args");
    list.set_strv("applications", glib::StrV::new()).unwrap();
    let e = error_of(service.call(
        "privacy.app.globalShortcuts.set",
        &json!({"id": id, "clear": true}),
    ));
    assert_eq!(e.key.as_deref(), Some("privacy.shortcutsUnavailable"));
    let _ = std::fs::remove_file(file);
}

/// A snapd on a socket of its own: answers from `answer(request line)`, every request head recorded.
fn fake_snapd(socket: &std::path::Path, answer: fn(&str) -> String) -> Arc<Mutex<Vec<String>>> {
    let listener = UnixListener::bind(socket).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
                if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = v.trim().parse().unwrap_or(0);
                }
                head.push_str(&line);
            }
            let mut body = vec![0u8; length];
            let _ = reader.read_exact(&mut body);
            head.push_str(&String::from_utf8_lossy(&body));
            let first = head.lines().next().unwrap_or("").to_owned();
            record.lock().unwrap().push(head);
            let mut stream = stream;
            let _ = stream.write_all(answer(&first).as_bytes());
        }
    });
    seen
}

fn snapd_answers(request: &str) -> String {
    let (status, body) = if request.starts_with("GET /v2/connections") {
        ("200 OK", CONNECTIONS.to_owned())
    } else if request.starts_with("GET /v2/snaps/spotify") {
        ("200 OK", r#"{"type":"sync","result":{"name":"spotify","version":"1.2.3","installed-size":4096}}"#.to_owned())
    } else if request.starts_with("POST /v2/interfaces") {
        (
            "202 Accepted",
            r#"{"type":"async","status-code":202,"change":"7"}"#.to_owned(),
        )
    } else if request.starts_with("GET /v2/changes/7") {
        (
            "200 OK",
            r#"{"type":"sync","result":{"id":"7","ready":true,"status":"Done"}}"#.to_owned(),
        )
    } else {
        (
            "404 Not Found",
            r#"{"type":"error","status-code":404,"result":{"message":"not found"}}"#.to_owned(),
        )
    };
    String::from_utf8(http(status, &body)).unwrap()
}

#[test]
fn snap_interfaces_connect_through_snapd_with_interaction_allowed() {
    if sandbox().is_none() {
        eprintln!("skipped: not in tests/rust-sandbox");
        return;
    }
    let id = "spotify_spotify.desktop";
    let Some(file) = install_app(
        id,
        "[Desktop Entry]\nType=Application\nName=Spotify\nExec=true\nX-SnapInstanceName=spotify\n",
    ) else {
        panic!("GIO did not see the test app");
    };
    let dir = std::env::temp_dir().join(format!("privacy-snapd-{}", crate::events::nonce()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("snapd.socket");
    let seen = fake_snapd(&socket, snapd_answers);
    let (ctx, events, _ctx_root) = context(false);
    let service = Privacy::with(
        ctx,
        Targets {
            snapd: socket.clone(),
            store: "org.example.NoSuchStore".into(),
            ..Targets::default()
        },
    );
    let info = service
        .call("privacy.app", &json!({"id": id}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (info["kind"].clone(), info["version"].clone()),
        (json!("snap"), json!("1.2.3"))
    );
    assert_eq!(info["required"]["kind"], "snap");
    assert_eq!(info["required"]["plugs"].as_array().unwrap().len(), 2);
    assert_eq!(info["storage"]["app"], 4096);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|h| !h.contains("X-Allow-Interaction")),
        "reads need no password"
    );
    // camera has one candidate slot: connect picks it
    service
        .call(
            "privacy.app.snapConnect",
            &json!({"id": id, "plug": "camera", "connect": true}),
        )
        .unwrap()
        .unwrap();
    let heads = seen.lock().unwrap().clone();
    let post = heads
        .iter()
        .find(|h| h.starts_with("POST /v2/interfaces"))
        .expect("a POST");
    assert!(post.contains("X-Allow-Interaction: true"), "{post}");
    assert!(post.contains(r#""action":"connect""#), "{post}");
    assert!(
        post.contains(r#""slots":[{"snap":"snapd","slot":"camera"}]"#),
        "{post}"
    );
    assert!(
        heads.iter().any(|h| h.starts_with("GET /v2/changes/7")),
        "waits for the change"
    );
    assert_eq!(events.lock().unwrap()[0]["what"], "app");
    // audio-playback has two: connect needs a slot named; disconnect takes the connected one
    let e = error_of(service.call(
        "privacy.app.snapConnect",
        &json!({"id": id, "plug": "audio-playback", "connect": true}),
    ));
    assert_eq!(e.code, "bad_args");
    let r = service
        .call(
            "privacy.app.snapConnect",
            &json!({"id": id, "plug": "audio-playback", "connect": false}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(r["slot"], json!({"snap":"snapd","slot":"audio-playback"}));
    // a dry run posts nothing
    let posts = |s: &Arc<Mutex<Vec<String>>>| {
        s.lock()
            .unwrap()
            .iter()
            .filter(|h| h.starts_with("POST"))
            .count()
    };
    let before = posts(&seen);
    let (dry, _, _dry_root) = context(true);
    let dry = Privacy::with(
        dry,
        Targets {
            snapd: socket.clone(),
            ..Targets::default()
        },
    );
    dry.call("privacy.app.snapConnect", &json!({"id": id, "plug": "camera", "slot": {"snap":"snapd","slot":"camera"}, "connect": true}))
        .unwrap()
        .unwrap();
    assert_eq!(posts(&seen), before);
    // snapd gone: unsupported
    let (ctx, _, _ctx_root) = context(false);
    let gone = Privacy::with(
        ctx,
        Targets {
            snapd: dir.join("missing.socket"),
            ..Targets::default()
        },
    );
    let e = error_of(gone.call(
        "privacy.app.snapConnect",
        &json!({"id": id, "plug": "camera", "connect": false}),
    ));
    assert_eq!(e.key.as_deref(), Some("privacy.snapUnavailable"));
    let _ = std::fs::remove_file(file);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn handlers_autostart_and_cache_change_only_the_users_files() {
    if sandbox().is_none() {
        eprintln!("skipped: not in tests/rust-sandbox");
        return;
    }
    let _mimeapps = MIMEAPPS.lock().unwrap_or_else(|e| e.into_inner());
    let id = "org.example.Viewer.desktop";
    let Some(file) = install_app(
        id,
        "[Desktop Entry]\nType=Application\nName=Viewer\nExec=true %f\nMimeType=image/x-win11-test;x-scheme-handler/win11test;\nX-Flatpak=org.example.Viewer\n",
    ) else {
        panic!("GIO did not see the test app");
    };
    // GIO takes an app's types from update-desktop-database's mimeinfo.cache, which this private directory has
    // not: Added Associations give the types to GIO at once, as an installed app's cache would
    let viewer = gio::DesktopAppInfo::new(id).unwrap();
    for t in ["image/x-win11-test", "x-scheme-handler/win11test"] {
        viewer.add_supports_type(t).unwrap();
    }
    let (ctx, _, _ctx_root) = context(false);
    let service = Privacy::with(
        ctx,
        Targets {
            store: "org.example.NoSuchStore".into(),
            ..Targets::default()
        },
    );
    let info = service
        .call("privacy.app", &json!({"id": id, "sizes": false}))
        .unwrap()
        .unwrap();
    let types: Vec<&str> = info["handlers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["type"].as_str().unwrap())
        .collect();
    assert_eq!(
        types,
        ["image/x-win11-test", "x-scheme-handler/win11test"],
        "file types first, then links"
    );
    assert_eq!(info["handlers"][1]["kind"], "link");
    // remove one type: GIO records it under [Removed Associations]; reset takes the user's choice away again
    let mimeapps = glib::user_config_dir().join("mimeapps.list");
    service
        .call(
            "privacy.app.removeHandler",
            &json!({"id": id, "type": "image/x-win11-test"}),
        )
        .unwrap()
        .unwrap();
    let text = std::fs::read_to_string(&mimeapps).unwrap();
    assert!(
        text.contains("[Removed Associations]")
            && text.contains("image/x-win11-test=org.example.Viewer.desktop"),
        "{text}"
    );
    let e = error_of(service.call(
        "privacy.app.removeHandler",
        &json!({"id": id, "type": "text/plain"}),
    ));
    assert_eq!(e.code, "bad_args", "only a type the app handles");
    let r = service
        .call("privacy.app.resetHandlers", &json!({"id": id}))
        .unwrap()
        .unwrap();
    assert_eq!(
        r["types"],
        json!(["image/x-win11-test", "x-scheme-handler/win11test"])
    );
    assert!(
        !std::fs::read_to_string(&mimeapps)
            .unwrap_or_default()
            .contains("image/x-win11-test")
    );
    // Autostart: a copy of the desktop file, then the flag off; a dry run writes nothing
    let autostart = glib::user_config_dir().join("autostart").join(id);
    let _ = std::fs::remove_file(&autostart);
    let (dry, _, _dry_root) = context(true);
    let dry = Privacy::with(dry, Targets::default());
    dry.call("privacy.app.setAutostart", &json!({"id": id, "on": true}))
        .unwrap()
        .unwrap();
    assert!(!autostart.exists());
    service
        .call("privacy.app.setAutostart", &json!({"id": id, "on": true}))
        .unwrap()
        .unwrap();
    let text = std::fs::read_to_string(&autostart).unwrap();
    assert!(
        text.contains("Name=Viewer") && text.contains("X-GNOME-Autostart-enabled=true"),
        "{text}"
    );
    assert_eq!(
        service
            .call("privacy.app", &json!({"id": id, "sizes": false}))
            .unwrap()
            .unwrap()["autostart"]["enabled"],
        true
    );
    service
        .call("privacy.app.setAutostart", &json!({"id": id, "on": false}))
        .unwrap()
        .unwrap();
    assert!(
        std::fs::read_to_string(&autostart)
            .unwrap()
            .contains("X-GNOME-Autostart-enabled=false")
    );
    // Clear Cache: the Flatpak's ~/.var/app/<id>/cache, emptied; a dry run keeps it
    let cache = glib::home_dir().join(".var/app/org.example.Viewer/cache");
    std::fs::create_dir_all(cache.join("thumbs")).unwrap();
    std::fs::write(cache.join("thumbs/a"), vec![1u8; 8192]).unwrap();
    assert_eq!(
        service
            .call("privacy.app", &json!({"id": id, "sizes": false}))
            .unwrap()
            .unwrap()["canClearCache"],
        true
    );
    dry.call("privacy.app.clearCache", &json!({"id": id}))
        .unwrap()
        .unwrap();
    assert!(cache.join("thumbs/a").exists());
    let r = service
        .call("privacy.app.clearCache", &json!({"id": id}))
        .unwrap()
        .unwrap();
    assert!(r["freed"].as_u64().unwrap() >= 8192);
    assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 0);
    let _ = std::fs::remove_file(autostart);
    let _ = std::fs::remove_file(file);
}

// ------------------------------------------------------------------------- slice 3: Privacy en beveiliging

fn attr(id: &str, level: u32, result: u32, flags: u64) -> HashMap<String, glib::Variant> {
    HashMap::from([
        ("AppstreamId".to_owned(), id.to_variant()),
        ("HsiLevel".to_owned(), level.to_variant()),
        ("HsiResult".to_owned(), result.to_variant()),
        ("Flags".to_owned(), flags.to_variant()),
        ("Name".to_owned(), format!("{id} name").to_variant()),
        ("Created".to_owned(), 1_700_000_000u64.to_variant()),
    ])
}

#[test]
fn reports_insights_security_and_providers_parse_as_gnome_reads_them() {
    // whoopsie-preferences: the drop-down's three choices and back
    assert_eq!(reports::mode(false, true), "never");
    assert_eq!(reports::mode(true, true), "auto");
    assert_eq!(reports::mode(true, false), "manual");
    for mode in ["never", "auto", "manual"] {
        let (report, automatic) = reports::flags(mode).unwrap();
        assert_eq!(reports::mode(report, automatic), mode);
    }
    assert!(reports::flags("always").is_none());
    // ubuntu-insights consent: `<source>: true|false` on stdout, a missing file is unknown
    assert_eq!(
        insights::parse_consent("linux: true\nubuntu_release_upgrader: false\n"),
        vec![
            ("linux", Some(true)),
            ("ubuntu_release_upgrader", Some(false))
        ]
    );
    assert_eq!(
        insights::parse_consent("2026/11/16 15:23:01 WARN flag\nlinux: true\n"),
        vec![("linux", Some(true)), ("ubuntu_release_upgrader", None)]
    );
    assert_eq!(insights::parse_consent("linux: maybe")[0], ("linux", None));
    assert!(insights::source_ok("ubuntu_release_upgrader") && !insights::source_ok("kernel"));
    assert!(insights::pretty(r#"{"a":1}"#).unwrap().contains("\"a\": 1"));
    assert!(insights::pretty("not json").is_none() && insights::pretty("[1]").is_none());
    // fwupd's HostSecurityId
    assert_eq!(security::parse_hsi("HSI:1! (v2.0.13)"), (Some(1), true));
    assert_eq!(security::parse_hsi("HSI:3 (v2.0.13)"), (Some(3), false));
    assert_eq!(security::parse_hsi("HSI:0"), (Some(0), false));
    assert_eq!(
        security::parse_hsi("HSI:INVALID:missing-data"),
        (None, false)
    );
    assert_eq!(security::parse_hsi(""), (None, false));
    // the attributes' keys, and Secure Boot's state as GNOME's row shows it
    let json_of = |a: HashMap<String, glib::Variant>| {
        security::attr_json(&a.into_iter().collect::<BTreeMap<_, _>>())
    };
    let boot = json_of(attr(
        "org.fwupd.hsi.Uefi.SecureBoot",
        1,
        1,
        security::FLAG_SUCCESS,
    ));
    assert_eq!(
        (
            &boot["level"],
            &boot["result"],
            &boot["success"],
            &boot["title"]
        ),
        (
            &json!(1),
            &json!(1),
            &json!(true),
            &json!("org.fwupd.hsi.Uefi.SecureBoot name")
        )
    );
    assert_eq!(security::secure_boot(&[boot]), Some("on"));
    let off = json_of(attr("org.fwupd.hsi.Uefi.SecureBoot", 1, 2, 0));
    assert_eq!(security::secure_boot(&[off]), Some("off"));
    let odd = json_of(attr("org.fwupd.hsi.Uefi.SecureBoot", 1, 4, 0));
    assert_eq!(security::secure_boot(&[odd]), Some("problem"));
    let tpm = json_of(attr(
        "org.fwupd.hsi.Tpm.Version20",
        1,
        11,
        1 | security::FLAG_RUNTIME_ISSUE,
    ));
    assert_eq!(tpm["runtimeIssue"], true);
    assert_eq!(
        security::secure_boot(&[tpm]),
        None,
        "no Secure Boot attribute: no row"
    );
    // search providers: per dir by file name, the first dir naming an app wins, only .ini with a DesktopId
    let root = std::env::temp_dir().join(format!("privacy-search-{}", crate::events::nonce()));
    let _keep = Temp(root.clone());
    let (one, two) = (root.join("one"), root.join("two"));
    for (dir, name, body) in [
        (
            &one,
            "b.ini",
            "[Shell Search Provider]\nDesktopId=org.example.B.desktop\nDefaultDisabled=true\n",
        ),
        (
            &one,
            "a.ini",
            "[Shell Search Provider]\nDesktopId=org.example.A.desktop\n",
        ),
        (
            &one,
            "c.txt",
            "[Shell Search Provider]\nDesktopId=org.example.C.desktop\n",
        ),
        (&one, "d.ini", "[Other]\nDesktopId=org.example.D.desktop\n"),
        (&one, "e.ini", "[Shell Search Provider]\nDesktopId=../bad\n"),
        (
            &two,
            "a.ini",
            "[Shell Search Provider]\nDesktopId=org.example.A.desktop\nDefaultDisabled=true\n",
        ),
        (
            &two,
            "z.ini",
            "[Shell Search Provider]\nDesktopId=org.example.Z.desktop\n",
        ),
    ] {
        let dir = dir.join("gnome-shell").join("search-providers");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), body).unwrap();
    }
    assert_eq!(
        search::files(&[one, two]),
        vec![
            ("org.example.A.desktop".to_owned(), false),
            ("org.example.B.desktop".to_owned(), true),
            ("org.example.Z.desktop".to_owned(), false),
        ]
    );
}

/// A stand-in for /usr/bin/ubuntu-insights that logs its arguments (one call per line) and answers as 0.9.3 does.
fn insights_stub(dir: &std::path::Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let (tool, log) = (dir.join("ubuntu-insights"), dir.join("calls.log"));
    let script = format!(
        r#"#!/bin/sh
echo "$*" >> '{log}'
case "$1" in
consent)
  if [ "$3" = "--state" ]; then
    if [ -e '{dir}/refuse' ]; then echo "ERROR cannot write" >&2; exit 1; fi
    echo "$2: $4"; exit 0
  fi
  if [ "$2" = "linux" ]; then echo "linux: true"; exit 0; fi
  echo "ERROR Failed to get consent state for source source=$2 error=\"consent file not found\"" >&2; exit 1 ;;
collect)
  if [ -e '{dir}/slow' ]; then exec sleep 30; fi
  printf '%s' '{{"insightsVersion":"0.9.3","systemInfo":{{"hardware":{{"cpu":{{"cpus":16}}}}}}}}'; exit 0 ;;
esac
exit 2
"#,
        log = log.display(),
        dir = dir.display()
    );
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    (tool, log)
}

#[test]
fn system_reports_go_through_ubuntu_insights() {
    let (ctx, events, root) = context(false);
    let (tool, log) = insights_stub(&root.0);
    let calls = || std::fs::read_to_string(&log).unwrap_or_default();
    let service = Privacy::with(
        ctx,
        Targets {
            insights: Some(tool.clone()),
            ..Targets::default()
        },
    );
    // consent per source; the upgrader has no consent file yet (shown off)
    let got = service
        .call("privacy.insights.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        got,
        json!({"available": true, "sources": {"linux": true, "ubuntu_release_upgrader": null}})
    );
    assert_eq!(calls(), "consent linux\nconsent ubuntu_release_upgrader\n");
    // a dry run validates and writes nothing
    let (dry, dry_events, _dry_root) = context(true);
    let dry = Privacy::with(
        dry,
        Targets {
            insights: Some(tool.clone()),
            ..Targets::default()
        },
    );
    let answer = dry
        .call(
            "privacy.insights.set",
            &json!({"source": "ubuntu_release_upgrader", "consent": true}),
        )
        .unwrap()
        .unwrap();
    assert_eq!(answer["consent"], true);
    assert!(!calls().contains("--state"), "a dry run runs no write");
    assert!(dry_events.lock().unwrap().is_empty());
    // the write: the tool's own consent command, then the event
    service
        .call(
            "privacy.insights.set",
            &json!({"source": "ubuntu_release_upgrader", "consent": true}),
        )
        .unwrap()
        .unwrap();
    assert!(calls().ends_with("consent ubuntu_release_upgrader --state true\n"));
    assert_eq!(
        events.lock().unwrap().last().cloned(),
        Some(json!({"what": "insights", "source": "ubuntu_release_upgrader"}))
    );
    std::fs::write(root.0.join("refuse"), "").unwrap();
    let e = error_of(service.call(
        "privacy.insights.set",
        &json!({"source": "linux", "consent": false}),
    ));
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("failed", Some("privacy.consentFailed"))
    );
    // the report: collect --dry-run, indented
    let report = service
        .call("privacy.insights.report", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(report["source"], "linux");
    assert!(report["report"].as_str().unwrap().contains("\"cpus\": 16"));
    assert!(calls().contains("collect --dry-run\n"));
    // Annuleren: a running report stops, a second one meanwhile is busy
    std::fs::write(root.0.join("slow"), "").unwrap();
    let running = service.clone();
    let worker = std::thread::spawn(move || {
        running
            .call("privacy.insights.report", &json!({"source": "linux"}))
            .unwrap()
    });
    let started = std::time::Instant::now();
    while !*service.reporting.lock().unwrap() {
        assert!(started.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    }
    let e = error_of(service.call("privacy.insights.report", &json!({})));
    assert_eq!(e.key.as_deref(), Some("privacy.reportBusy"));
    let stop = service
        .call("privacy.insights.report", &json!({"cancel": true}))
        .unwrap()
        .unwrap();
    assert_eq!(stop, json!({"cancelled": true}));
    let e = worker.join().unwrap().expect_err("cancelled");
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("cancelled", Some("privacy.reportCancelled"))
    );
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "it stopped at once"
    );
    assert_eq!(
        service
            .call("privacy.insights.report", &json!({"cancel": true}))
            .unwrap()
            .unwrap(),
        json!({"cancelled": false}),
        "nothing runs any more"
    );
    // not installed: get says so, the writes are refused
    let (gone_ctx, _, _gone_root) = context(false);
    let gone = Privacy::with(
        gone_ctx,
        Targets {
            insights: Some(root.0.join("missing")),
            ..Targets::default()
        },
    );
    assert_eq!(
        gone.call("privacy.insights.get", &json!({}))
            .unwrap()
            .unwrap()["available"],
        false
    );
    let e = error_of(gone.call(
        "privacy.insights.set",
        &json!({"source": "linux", "consent": true}),
    ));
    assert_eq!(e.key.as_deref(), Some("privacy.insightsUnavailable"));
}

const WHOOPSIE_XML: &str = r#"<node><interface name="com.ubuntu.WhoopsiePreferences">
<property name="ReportCrashes" type="b" access="read"/>
<property name="ReportMetrics" type="b" access="read"/>
<property name="AutomaticallyReportCrashes" type="b" access="read"/>
<method name="SetReportCrashes"><arg type="b" direction="in"/></method>
<method name="SetAutomaticallyReportCrashes"><arg type="b" direction="in"/></method>
<method name="SetReportMetrics"><arg type="b" direction="in"/></method>
<method name="GetIdentifier"><arg type="s" direction="out"/></method>
</interface></node>"#;

#[test]
fn error_reports_go_to_whoopsie_preferences_with_interaction_allowed() {
    let Some(address) = sandbox() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let state = Arc::new(Mutex::new((true, false)));
    let deny = Arc::new(AtomicBool::new(false));
    let (data, refuse) = (state.clone(), deny.clone());
    let reply: Reply = Box::new(move |method, params| {
        if refuse.load(Ordering::Acquire) {
            return Err((
                "org.freedesktop.DBus.Error.AccessDenied".into(),
                "Not authorized".into(),
            ));
        }
        let on = params.child_value(0).get::<bool>().unwrap_or(false);
        let mut s = data.lock().unwrap();
        match method {
            "SetReportCrashes" => s.0 = on,
            "SetAutomaticallyReportCrashes" => s.1 = on,
            _ => {}
        }
        Ok(None)
    });
    let read = state.clone();
    let props: Props = Box::new(move |name| {
        let s = read.lock().unwrap();
        match name {
            "ReportCrashes" => s.0.to_variant(),
            "AutomaticallyReportCrashes" => s.1.to_variant(),
            _ => false.to_variant(),
        }
    });
    let fake = Fake::start_with(
        address.clone(),
        WHOOPSIE_XML,
        "/com/ubuntu/WhoopsiePreferences",
        "com.ubuntu.WhoopsiePreferences",
        reply,
        props,
    );
    let targets = |name: &str| Targets {
        whoopsie: name.to_owned(),
        system: Some(address.clone()),
        ..Targets::default()
    };
    let (ctx, events, _root) = context(false);
    let service = Privacy::with(ctx, targets(&fake.name));
    let got = service
        .call("privacy.reports.get", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        got,
        json!({"available": true, "mode": "manual", "reportCrashes": true, "automatic": false})
    );
    // a dry run reads and writes nothing
    let (dry, dry_events, _dry_root) = context(true);
    let dry = Privacy::with(dry, targets(&fake.name));
    dry.call("privacy.reports.set", &json!({"mode": "auto"}))
        .unwrap()
        .unwrap();
    assert!(fake.calls().iter().all(|(m, _, _)| !m.starts_with("Set")));
    assert!(dry_events.lock().unwrap().is_empty());
    // Automatisch: both flags, in GNOME's order, each allowing the password dialog
    service
        .call("privacy.reports.set", &json!({"mode": "auto"}))
        .unwrap()
        .unwrap();
    let sets: Vec<(String, String, bool)> = fake
        .calls()
        .into_iter()
        .filter(|(m, _, _)| m.starts_with("Set"))
        .collect();
    assert_eq!(
        sets,
        vec![
            ("SetReportCrashes".to_owned(), "(true,)".to_owned(), true),
            (
                "SetAutomaticallyReportCrashes".to_owned(),
                "(true,)".to_owned(),
                true
            ),
        ]
    );
    assert_eq!(*state.lock().unwrap(), (true, true));
    assert_eq!(
        events.lock().unwrap().last().cloned(),
        Some(json!({"what": "reports", "mode": "auto"}))
    );
    service
        .call("privacy.reports.set", &json!({"mode": "never"}))
        .unwrap()
        .unwrap();
    assert_eq!(*state.lock().unwrap(), (false, false));
    assert_eq!(
        service
            .call("privacy.reports.get", &json!({}))
            .unwrap()
            .unwrap()["mode"],
        "never"
    );
    // polkit's refusal
    deny.store(true, Ordering::Release);
    let e = error_of(service.call("privacy.reports.set", &json!({"mode": "manual"})));
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("denied", Some("core.notAuthorized"))
    );
    // no daemon: not available, nothing to write
    let (none_ctx, _, _none_root) = context(false);
    let none = Privacy::with(none_ctx, targets("com.ubuntu.NoWhoopsieHere"));
    assert_eq!(
        none.call("privacy.reports.get", &json!({}))
            .unwrap()
            .unwrap(),
        json!({"available": false})
    );
    let e = error_of(none.call("privacy.reports.set", &json!({"mode": "auto"})));
    assert_eq!(
        (e.code.as_str(), e.key.as_deref()),
        ("unsupported", Some("privacy.reportsUnavailable"))
    );
}

const FWUPD_XML: &str = r#"<node><interface name="org.freedesktop.fwupd">
<property name="DaemonVersion" type="s" access="read"/>
<property name="HostVendor" type="s" access="read"/>
<property name="HostProduct" type="s" access="read"/>
<property name="HostSecurityId" type="s" access="read"/>
<method name="GetHostSecurityAttrs"><arg type="aa{sv}" direction="out"/></method>
<method name="GetHostSecurityEvents"><arg type="u" direction="in"/><arg type="aa{sv}" direction="out"/></method>
</interface></node>"#;

#[test]
fn device_security_reads_fwupd_without_writing() {
    let Some(address) = sandbox() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let reply: Reply = Box::new(move |method, _| {
        let list: Vec<HashMap<String, glib::Variant>> = match method {
            "GetHostSecurityAttrs" => vec![
                attr(
                    "org.fwupd.hsi.Uefi.SecureBoot",
                    1,
                    1,
                    security::FLAG_SUCCESS,
                ),
                attr("org.fwupd.hsi.Tpm.Version20", 1, 11, security::FLAG_SUCCESS),
                attr("org.fwupd.hsi.Iommu", 2, 12, 0),
            ],
            _ => {
                let mut event = attr("org.fwupd.hsi.Uefi.SecureBoot", 1, 2, 0);
                event.insert("HsiResultFallback".into(), 1u32.to_variant());
                vec![event]
            }
        };
        Ok(Some(glib::Variant::tuple_from_iter([list.to_variant()])))
    });
    let props: Props = Box::new(|name| {
        match name {
            "HostSecurityId" => "HSI:1! (v2.0.13)",
            "DaemonVersion" => "2.0.13",
            "HostVendor" => "WinPoint Labs",
            _ => "Test PC",
        }
        .to_variant()
    });
    let fake = Fake::start_with(
        address.clone(),
        FWUPD_XML,
        "/",
        "org.freedesktop.fwupd",
        reply,
        props,
    );
    let (ctx, events, _root) = context(false);
    let service = Privacy::with(
        ctx,
        Targets {
            fwupd: fake.name.clone(),
            system: Some(address.clone()),
            ..Targets::default()
        },
    );
    let got = service
        .call("privacy.security", &json!({}))
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            &got["available"],
            &got["level"],
            &got["runtimeIssue"],
            &got["secureBoot"]
        ),
        (&json!(true), &json!(1), &json!(true), &json!("on"))
    );
    assert_eq!(
        (&got["daemonVersion"], &got["vendor"], &got["product"]),
        (&json!("2.0.13"), &json!("WinPoint Labs"), &json!("Test PC"))
    );
    assert_eq!(got["attrs"].as_array().unwrap().len(), 3);
    assert_eq!(got["attrs"][2]["level"], 2);
    assert_eq!(
        (&got["events"][0]["result"], &got["events"][0]["previous"]),
        (&json!(2), &json!(1))
    );
    let calls = fake.calls();
    assert!(
        calls
            .iter()
            .any(|(m, a, _)| m == "GetHostSecurityEvents" && a == "(uint32 10,)"),
        "GNOME's last 10 events: {calls:?}"
    );
    assert!(
        calls.iter().all(|(_, _, interactive)| !interactive),
        "reads need no password"
    );
    assert!(events.lock().unwrap().is_empty());
    // no fwupd: not available (GNOME: Checks Unavailable)
    let (none_ctx, _, _none_root) = context(false);
    let none = Privacy::with(
        none_ctx,
        Targets {
            fwupd: "org.freedesktop.NoFwupdHere".into(),
            system: Some(address),
            ..Targets::default()
        },
    );
    assert_eq!(
        none.call("privacy.security", &json!({})).unwrap().unwrap()["available"],
        false
    );
}

/// GNOME Settings 50 has no control for GeoClue's accuracy cap (`max-accuracy-level`: "'exact', no UI"), so the
/// allow table reads it and never writes it, while Locatieservices (`enabled`) stays writable (§6.1).
#[test]
fn location_accuracy_is_read_only_as_gnome_settings_has_no_control() {
    if sandbox().is_none() {
        eprintln!("skipped: not in tests/rust-sandbox");
        return;
    }
    use crate::services::gsettings;
    const LOCATION: &str = "org.gnome.system.location";
    for method in ["gsettings.set", "privacy.setPermission"] {
        // a dry run: the checks only, nothing written even if a check let it through
        let e = gsettings::write_values(
            method,
            LOCATION,
            None,
            &[("max-accuracy-level", json!("country"))],
            Lang::En,
            true,
        )
        .unwrap_err();
        assert_eq!(
            (e.code.as_str(), e.key.as_deref()),
            ("denied", Some("gsettings.readOnly")),
            "{method}"
        );
    }
    let item = gsettings::read_item(LOCATION, None, "max-accuracy-level", Lang::En).unwrap();
    assert_eq!(
        (&item["allowed"], &item["mode"]),
        (&json!(true), &json!("read"))
    );
    assert_eq!(item["value"], "exact", "GNOME's default, untouched");
    let ok = gsettings::write_values(
        "gsettings.set",
        LOCATION,
        None,
        &[("enabled", json!(true))],
        Lang::En,
        true,
    )
    .unwrap();
    assert_eq!(ok["items"][0]["key"], "enabled");
}

/// The en column is en-GB, as Windows' en-GB Settings and the page's "Security Centre" card say (the snap's own
/// Name[en_GB]); App Center keeps its spelling, Ubuntu's product name in every language.
#[test]
fn the_security_centre_is_named_in_en_gb() {
    assert_eq!(
        crate::i18n::tr(Lang::En, "privacy.securityCenterMissing"),
        "Security Centre isn’t installed."
    );
    assert_eq!(
        crate::i18n::tr(Lang::Nl, "privacy.securityCenterMissing"),
        "Het Beveiligingscentrum is niet geïnstalleerd."
    );
    for stem in ["privacy", "background", "notifications", "apps"] {
        let tsv = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("i18n/{stem}.tsv")),
        )
        .unwrap();
        for line in tsv.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
            let en = line.split('\t').nth(2).unwrap_or("");
            assert!(
                !en.replace("App Center", "").contains("Center"),
                "US spelling in {stem}.tsv: {line}"
            );
        }
    }
}

/// GNOME's Apps, Location and Camera panels follow the PermissionStore's `Changed` signal; the service relays it as
/// `privacy.changed {what: "permissions", table, id}` for the entries GNOME shows, only from the store itself, and
/// no longer once it is shut down.
#[test]
fn a_change_in_the_permission_store_from_elsewhere_reaches_the_pages() {
    // the signal's parameters: (s table, s id, b deleted, v data, a{sas} permissions)
    let params = |table: &str, id: &str| {
        (
            table,
            id,
            false,
            "".to_variant(),
            HashMap::<String, Vec<String>>::from([("org.gnome.Maps".into(), vec!["yes".into()])]),
        )
            .to_variant()
    };
    assert_eq!(
        perms::changed_kind(&params("devices", "camera")).map(|k| k.key),
        Some("camera")
    );
    assert_eq!(
        perms::changed_kind(&params("location", "location")).map(|k| k.key),
        Some("location")
    );
    assert!(perms::changed_kind(&params("notifications", "notifications")).is_none());
    assert!(perms::changed_kind(&().to_variant()).is_none());
    let Some(address) = sandbox() else {
        eprintln!("skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)");
        return;
    };
    let connect = || {
        gio::DBusConnection::for_address_sync(
            &address,
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            gio::Cancellable::NONE,
        )
        .unwrap()
    };
    // the "store" is a connection of its own on the private bus (the service follows its name)
    let (store, other) = (connect(), connect());
    let emit = |from: &gio::DBusConnection, table: &str, id: &str| {
        from.emit_signal(
            None,
            "/org/freedesktop/impl/portal/PermissionStore",
            "org.freedesktop.impl.portal.PermissionStore",
            "Changed",
            Some(&params(table, id)),
        )
        .unwrap();
        from.flush_sync(gio::Cancellable::NONE).unwrap();
    };
    let (ctx, events, _root) = context(false);
    let service = Privacy::with(
        ctx,
        Targets {
            store: store.unique_name().unwrap().to_string(),
            ..Targets::default()
        },
    );
    service.watch_store();
    let seen = |n: usize| {
        (0..50).any(|_| {
            if events.lock().unwrap().len() >= n {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
            false
        })
    };
    // a table GNOME Settings does not show, then the camera: only the camera is relayed (in order, so the first
    // one was already passed over when the second arrives)
    emit(&store, "notifications", "notifications");
    emit(&store, "devices", "camera");
    assert!(seen(1), "the camera's change is relayed");
    assert_eq!(
        events.lock().unwrap().clone(),
        [json!({"what": "permissions", "table": "devices", "id": "camera"})]
    );
    // the same signal from another connection is not the store's
    emit(&other, "location", "location");
    emit(&store, "location", "location");
    assert!(seen(2));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        events.lock().unwrap().clone(),
        [
            json!({"what": "permissions", "table": "devices", "id": "camera"}),
            json!({"what": "permissions", "table": "location", "id": "location"}),
        ]
    );
    // after shutdown the watch has ended
    service.shutdown();
    std::thread::sleep(Duration::from_millis(500));
    emit(&store, "devices", "microphone");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(events.lock().unwrap().len(), 2, "nothing after shutdown");
}
