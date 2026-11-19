//! `caps.get`: what this computer has, so Settings hides what GNOME Settings hides (notes/claude/settings-parity.md
//! §6.2). Every value but `session` is a bool; the registry's `needs` (native/apps/settings/src/registry.rs) names
//! one of [`KEYS`].
//!
//! * Input devices from udev's database: `/run/udev/data/c*` with `E:ID_INPUT_<KIND>=1`; FileMonitors on `/dev/input`
//!   and `/run/udev/data` read them again 300 ms and 1.5 s after the last event (devtmpfs makes the node before udev
//!   writes its entry, and removes it before udev deletes it) and emit `caps.changed {caps}` when they change.
//! * Hardware from /sys (thunderbolt, backlight, battery) and the system bus (ModemManager, iio-sensor-proxy,
//!   logind, fprintd, colord; 500 ms timeouts, cached for 30 s), the privacy screen from Mutter's DisplayConfig.
//! * Installed tools and services from their files.
use super::ServiceContext;
use super::system::common;
use crate::files::ApiResult;
use gio::prelude::*;
use serde_json::{Map, Value, json};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// The methods (name, dangerous), as host/src/api-methods/foundation.json lists them.
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[("caps.get", false)];

/// Every key of `caps.get` (the registry's `needs` must be one of them; tests/i18n/test_native.py compares).
pub(crate) const KEYS: &[&str] = &[
    "session",
    "gnome",
    "mouse",
    "touchpad",
    "pointingstick",
    "tablet",
    "touchscreen",
    "thunderbolt",
    "modem",
    "ambientLight",
    "accelerometer",
    "privacyScreen",
    "backlight",
    "battery",
    "hibernate",
    "fingerprint",
    "colorSensor",
    "geoclue",
    "tracker",
    "usbguard",
    "sshServer",
    "userShare",
    "malcontent",
    "realmd",
    "rygel",
    "tecla",
    "orca",
    "simpleScan",
    "securityCenter",
    "insights",
    "whoopsie",
];

/// The udev input kinds: (caps key, udev property).
const INPUT_KINDS: &[(&str, &str)] = &[
    ("mouse", "ID_INPUT_MOUSE"),
    ("touchpad", "ID_INPUT_TOUCHPAD"),
    ("pointingstick", "ID_INPUT_POINTINGSTICK"),
    ("tablet", "ID_INPUT_TABLET"),
    ("touchscreen", "ID_INPUT_TOUCHSCREEN"),
];

/// Installed files: (caps key, path below the root; any one of them).
const FILES: &[(&str, &[&str])] = &[
    ("sshServer", &["usr/sbin/sshd"]),
    ("userShare", &["usr/libexec/gnome-user-share-webdav"]),
    ("malcontent", &["usr/bin/malcontent-control"]),
    ("realmd", &["usr/sbin/realm", "usr/libexec/realmd"]),
    ("rygel", &["usr/bin/rygel"]),
    ("tecla", &["usr/bin/tecla"]),
    ("orca", &["usr/bin/orca"]),
    ("simpleScan", &["usr/bin/simple-scan"]),
    ("securityCenter", &["snap/bin/desktop-security-center"]),
    ("insights", &["usr/bin/ubuntu-insights"]),
    (
        "whoopsie",
        &["usr/share/dbus-1/system-services/com.ubuntu.WhoopsiePreferences.service"],
    ),
    (
        "geoclue",
        &["usr/share/dbus-1/system-services/org.freedesktop.GeoClue2.service"],
    ),
    (
        "usbguard",
        &["usr/share/dbus-1/system-services/org.usbguard1.service"],
    ),
];

/// The session kind (amendment 56): "gnome", "hyprland" or "other".
pub(crate) fn session() -> &'static str {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return "hyprland";
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if desktop
        .split(':')
        .any(|d| d.eq_ignore_ascii_case("gnome") || d.eq_ignore_ascii_case("winpoint"))
    {
        "gnome"
    } else {
        "other"
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

/// The input kinds udev's database below `root` reports.
pub(crate) fn input_kinds(root: &Path) -> Map<String, Value> {
    let mut out: Map<String, Value> = INPUT_KINDS
        .iter()
        .map(|(k, _)| ((*k).into(), json!(false)))
        .collect();
    let Ok(entries) = std::fs::read_dir(root.join("run/udev/data")) else {
        return out;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        // character devices (c13:64 is /dev/input/event0); "+input:" entries are the parents
        if !name.to_string_lossy().starts_with('c') {
            continue;
        }
        let text = read(&entry.path());
        if !text.lines().any(|l| l == "E:ID_INPUT=1") {
            continue;
        }
        for (key, property) in INPUT_KINDS {
            if text.lines().any(|l| {
                l.strip_prefix("E:")
                    .is_some_and(|p| p == format!("{property}=1"))
            }) {
                out.insert((*key).into(), json!(true));
            }
        }
    }
    out
}

fn non_empty_dir(path: &Path) -> bool {
    std::fs::read_dir(path).is_ok_and(|mut d| d.next().is_some())
}

/// Hardware from /sys and installed files below `root` (the real one is "/").
pub(crate) fn scan_files(root: &Path) -> Map<String, Value> {
    let mut out = input_kinds(root);
    out.insert(
        "thunderbolt".into(),
        json!(non_empty_dir(&root.join("sys/bus/thunderbolt/devices"))),
    );
    out.insert(
        "backlight".into(),
        json!(non_empty_dir(&root.join("sys/class/backlight"))),
    );
    let battery = std::fs::read_dir(root.join("sys/class/power_supply")).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            read(&e.path().join("type")).trim() == "Battery"
                && read(&e.path().join("scope")).trim() != "Device"
        })
    });
    out.insert("battery".into(), json!(battery));
    for (key, paths) in FILES {
        out.insert(
            (*key).into(),
            json!(paths.iter().any(|p| root.join(p).exists())),
        );
    }
    out
}

/// Whether the Tracker 3 miner's schema is installed (the Deze pc doorzoeken page).
fn tracker() -> bool {
    gio::SettingsSchemaSource::default()
        .and_then(|s| s.lookup("org.freedesktop.Tracker3.Miner.Files", true))
        .is_some()
}

fn has_owner(bus: &gio::DBusConnection, name: &str) -> bool {
    common::call(
        bus,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameHasOwner",
        Some(&(name,).to_variant()),
        500,
    )
    .ok()
    .and_then(|v| v.child_value(0).get::<bool>())
    .unwrap_or(false)
}

/// The capabilities the buses report (each probe bounded by 500 ms; a missing bus or daemon is false).
fn scan_buses() -> Map<String, Value> {
    let mut out = Map::new();
    for key in [
        "modem",
        "ambientLight",
        "accelerometer",
        "hibernate",
        "fingerprint",
        "colorSensor",
        "privacyScreen",
    ] {
        out.insert(key.into(), json!(false));
    }
    if let Ok(bus) = common::system() {
        if has_owner(&bus, "org.freedesktop.ModemManager1") {
            let modems = common::call(
                &bus,
                "org.freedesktop.ModemManager1",
                "/org/freedesktop/ModemManager1",
                "org.freedesktop.DBus.ObjectManager",
                "GetManagedObjects",
                None,
                500,
            );
            out.insert(
                "modem".into(),
                json!(modems.is_ok_and(|v| v.child_value(0).n_children() > 0)),
            );
        }
        if let Ok(props) = common::getall(
            &bus,
            "net.hadess.SensorProxy",
            "/net/hadess/SensorProxy",
            "net.hadess.SensorProxy",
        ) {
            out.insert(
                "ambientLight".into(),
                json!(props["HasAmbientLight"] == true),
            );
            out.insert(
                "accelerometer".into(),
                json!(props["HasAccelerometer"] == true),
            );
        }
        let hibernate = common::call(
            &bus,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
            "CanHibernate",
            None,
            500,
        );
        out.insert(
            "hibernate".into(),
            json!(
                hibernate
                    .is_ok_and(|v| matches!(common::first(&v).as_str(), Some("yes" | "challenge")))
            ),
        );
        let fingerprint = common::call(
            &bus,
            "net.reactivated.Fprint",
            "/net/reactivated/Fprint/Manager",
            "net.reactivated.Fprint.Manager",
            "GetDevices",
            None,
            500,
        );
        out.insert(
            "fingerprint".into(),
            json!(fingerprint.is_ok_and(|v| v.child_value(0).n_children() > 0)),
        );
        let sensors = common::call(
            &bus,
            "org.freedesktop.ColorManager",
            "/org/freedesktop/ColorManager",
            "org.freedesktop.ColorManager",
            "GetSensors",
            None,
            500,
        );
        out.insert(
            "colorSensor".into(),
            json!(sensors.is_ok_and(|v| v.child_value(0).n_children() > 0)),
        );
    }
    if let Ok(bus) = common::session() {
        // Mutter lists "privacy-screen-state" in a monitor's properties when the panel has one
        let state = common::call(
            &bus,
            "org.gnome.Mutter.DisplayConfig",
            "/org/gnome/Mutter/DisplayConfig",
            "org.gnome.Mutter.DisplayConfig",
            "GetCurrentState",
            None,
            500,
        );
        out.insert(
            "privacyScreen".into(),
            json!(state.is_ok_and(|v| v.print(false).contains("privacy-screen-state"))),
        );
    }
    out
}

/// The whole caps object: files below `root`, the buses (`buses`), the session.
pub(crate) fn caps(root: &Path, buses: &Map<String, Value>) -> Value {
    let mut out = Map::new();
    let session = session();
    out.insert("session".into(), json!(session));
    out.insert("gnome".into(), json!(session == "gnome"));
    out.extend(scan_files(root));
    out.extend(buses.clone());
    out.insert("tracker".into(), json!(tracker()));
    // the documented order, every key present
    let ordered: Map<String, Value> = KEYS
        .iter()
        .map(|k| {
            (
                (*k).to_owned(),
                out.get(*k).cloned().unwrap_or(json!(false)),
            )
        })
        .collect();
    Value::Object(ordered)
}

const BUS_CACHE: Duration = Duration::from_secs(30);
/// What the input watcher follows, below the root.
const WATCHED: &[&str] = &["dev/input", "run/udev/data"];
/// The input kinds are read again this long after a change (early) and after the last change (late).
const REREAD: (Duration, Duration) = (Duration::from_millis(300), Duration::from_millis(1500));

type EmitCaps = Arc<dyn Fn(Value) + Send + Sync>;

pub(crate) struct Caps {
    root: PathBuf,
    buses: Mutex<Option<(Instant, Map<String, Value>)>>,
    last: Arc<Mutex<Value>>,
    stopped: Arc<AtomicBool>,
    context: Mutex<Option<glib::MainContext>>,
    /// The watcher's monitors are in place (tests).
    watching: Arc<AtomicBool>,
}

impl Caps {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let caps = Self::at(PathBuf::from("/"));
        caps.watch(ctx);
        Ok(caps)
    }

    fn at(root: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            root,
            buses: Mutex::new(None),
            last: Arc::new(Mutex::new(Value::Null)),
            stopped: Arc::new(AtomicBool::new(false)),
            context: Mutex::new(None),
            watching: Arc::new(AtomicBool::new(false)),
        })
    }

    fn buses(&self) -> Map<String, Value> {
        let mut cache = self.buses.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, value)) = cache.as_ref()
            && at.elapsed() < BUS_CACHE
        {
            return value.clone();
        }
        let value = scan_buses();
        *cache = Some((Instant::now(), value.clone()));
        value
    }

    fn get(&self) -> Value {
        let value = caps(&self.root, &self.buses());
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = value.clone();
        value
    }

    /// The input watcher (module docs) on its own MainContext thread: plugging a mouse or tablet in or out
    /// refreshes the input kinds and emits `caps.changed`.
    fn watch(self: &Arc<Self>, ctx: Arc<ServiceContext>) {
        self.watch_with(Arc::new(move |caps| {
            ctx.emit("caps.changed", json!({ "caps": caps }))
        }));
    }

    fn watch_with(self: &Arc<Self>, emit: EmitCaps) {
        let (stopped, last) = (self.stopped.clone(), self.last.clone());
        let root = self.root.clone();
        let context = glib::MainContext::new();
        *self.context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.clone());
        let ready = self.watching.clone();
        std::thread::spawn(move || {
            let _ = context.with_thread_default(|| {
                let monitors: Vec<gio::FileMonitor> = WATCHED
                    .iter()
                    .filter_map(|dir| {
                        gio::File::for_path(root.join(dir))
                            .monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                            .ok()
                    })
                    .collect();
                let refresh = {
                    let (last, root) = (last.clone(), root.clone());
                    Arc::new(move || {
                        let before = last.lock().unwrap_or_else(|e| e.into_inner()).clone();
                        if !before.is_object() {
                            return; // nobody asked yet
                        }
                        let mut now = before.clone();
                        for (key, value) in input_kinds(&root) {
                            now[key] = value;
                        }
                        if now != before {
                            *last.lock().unwrap_or_else(|e| e.into_inner()) = now.clone();
                            emit(now);
                        }
                    })
                };
                // (early, late): early fires at most every 300 ms, late 1.5 s after the last event
                let pending: Arc<Mutex<(Option<glib::Source>, Option<glib::Source>)>> =
                    Arc::new(Mutex::new((None, None)));
                for monitor in &monitors {
                    let (refresh, pending, context) =
                        (refresh.clone(), pending.clone(), context.clone());
                    monitor.connect_changed(move |_, _, _, _| {
                        let mut slots = pending.lock().unwrap_or_else(|e| e.into_inner());
                        let timer = |early: bool| {
                            let (refresh, pending) = (refresh.clone(), pending.clone());
                            let delay = if early { REREAD.0 } else { REREAD.1 };
                            let source = glib::timeout_source_new(
                                delay,
                                None,
                                glib::Priority::DEFAULT,
                                move || {
                                    let mut slots =
                                        pending.lock().unwrap_or_else(|e| e.into_inner());
                                    *(if early { &mut slots.0 } else { &mut slots.1 }) = None;
                                    drop(slots);
                                    refresh();
                                    glib::ControlFlow::Break
                                },
                            );
                            source.attach(Some(&context));
                            source
                        };
                        if slots.0.is_none() {
                            slots.0 = Some(timer(true));
                        }
                        if let Some(late) = slots.1.take() {
                            late.destroy();
                        }
                        slots.1 = Some(timer(false));
                    });
                }
                ready.store(true, Ordering::Release);
                while !stopped.load(Ordering::Acquire) {
                    context.iteration(true);
                }
                drop(monitors);
            });
        });
    }

    pub(crate) fn call(&self, method: &str, _args: &Value) -> Option<ApiResult> {
        match method {
            "caps.get" => Some(Ok(self.get())),
            _ => None,
        }
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(context) = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            context.wakeup();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, path: &str, text: &str) {
        let p = root.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn caps_reads_fixture_sys_and_udev_trees() {
        let root = std::env::temp_dir().join(format!("caps-{}", crate::events::nonce()));
        let empty = caps(&root, &Map::new());
        assert_eq!(
            empty
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            KEYS,
            "every key, in the documented order"
        );
        assert!(
            KEYS.iter()
                .filter(|k| **k != "session")
                .all(|k| empty[*k].is_boolean())
        );
        assert_eq!(empty["touchpad"], false);
        assert_eq!(empty["battery"], false);
        // a touchpad and a mouse (input character devices); a parent entry and a non-input device count for nothing
        write(
            &root,
            "run/udev/data/c13:64",
            "E:ID_INPUT=1\nE:ID_INPUT_TOUCHPAD=1\nE:ID_PATH=pci\n",
        );
        write(
            &root,
            "run/udev/data/c13:65",
            "E:ID_INPUT=1\nE:ID_INPUT_MOUSE=1\n",
        );
        write(
            &root,
            "run/udev/data/+input:input5",
            "E:ID_INPUT=1\nE:ID_INPUT_TABLET=1\n",
        );
        write(&root, "run/udev/data/c189:1", "E:ID_INPUT_TOUCHSCREEN=1\n");
        write(
            &root,
            "sys/class/backlight/intel_backlight/brightness",
            "100\n",
        );
        write(&root, "sys/class/power_supply/BAT0/type", "Battery\n");
        write(
            &root,
            "sys/class/power_supply/hidpp_battery_0/type",
            "Battery\n",
        );
        write(
            &root,
            "sys/class/power_supply/hidpp_battery_0/scope",
            "Device\n",
        );
        write(&root, "sys/bus/thunderbolt/devices/domain0/uevent", "");
        write(&root, "usr/bin/orca", "");
        let buses = Map::from_iter([("modem".to_owned(), json!(true))]);
        let c = caps(&root, &buses);
        assert_eq!(
            (
                c["touchpad"].clone(),
                c["mouse"].clone(),
                c["tablet"].clone(),
                c["touchscreen"].clone()
            ),
            (json!(true), json!(true), json!(false), json!(false))
        );
        assert_eq!(
            (
                c["backlight"].clone(),
                c["battery"].clone(),
                c["thunderbolt"].clone()
            ),
            (json!(true), json!(true), json!(true))
        );
        assert_eq!(
            (c["orca"].clone(), c["tecla"].clone(), c["modem"].clone()),
            (json!(true), json!(false), json!(true))
        );
        // a peripheral's battery alone is no system battery
        std::fs::remove_dir_all(root.join("sys/class/power_supply/BAT0")).unwrap();
        assert_eq!(caps(&root, &Map::new())["battery"], false);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_tablet_plugged_in_is_seen_once_udev_has_written_its_entry() {
        // review of B0, finding 17: devtmpfs makes /dev/input/eventN before udev writes /run/udev/data/c13:N
        let root = std::env::temp_dir().join(format!("caps-watch-{}", crate::events::nonce()));
        std::fs::create_dir_all(root.join("dev/input")).unwrap();
        std::fs::create_dir_all(root.join("run/udev/data")).unwrap();
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let service = Caps::at(root.clone());
        *service.last.lock().unwrap() = caps(&root, &Map::new());
        let recorder = seen.clone();
        service.watch_with(Arc::new(move |c| recorder.lock().unwrap().push(c)));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !service.watching.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "the watcher did not start");
            std::thread::sleep(Duration::from_millis(10));
        }
        let wait = |what: &str, ok: &dyn Fn(&[Value]) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(6);
            while !ok(&seen.lock().unwrap()) {
                assert!(
                    Instant::now() < deadline,
                    "{what}: {:?}",
                    seen.lock().unwrap()
                );
                std::thread::sleep(Duration::from_millis(20));
            }
        };
        // the node first, the udev entry 100 ms later: the first read finds nothing, a later one the tablet
        write(&root, "dev/input/event7", "");
        std::thread::sleep(Duration::from_millis(100));
        write(
            &root,
            "run/udev/data/c13:71",
            "E:ID_INPUT=1\nE:ID_INPUT_TABLET=1\n",
        );
        wait("tablet plugged in", &|s| {
            s.last().is_some_and(|c| c["tablet"] == true)
        });
        assert_eq!(seen.lock().unwrap().len(), 1, "one caps.changed");
        // unplugged: the node goes before udev deletes its entry
        std::fs::remove_file(root.join("dev/input/event7")).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        std::fs::remove_file(root.join("run/udev/data/c13:71")).unwrap();
        wait("tablet unplugged", &|s| {
            s.last().is_some_and(|c| c["tablet"] == false)
        });
        service.shutdown();
        let _ = std::fs::remove_dir_all(root);
    }
}
