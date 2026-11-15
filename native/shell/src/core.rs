//! The shared state of the shell: the backend client, the GNOME extension client, the settings cache and an
//! event bus. One `Shell` per process, used from the GTK main thread only (`Rc`); other threads reach it through
//! `dispatch`, which runs on the main context.
//!
//! The extension's events come twice: straight from its D-Bus signal (gnome.rs) and relayed by the backend's event
//! stream (host/src/gnome.rs `relays_event`: windows.*, nativeBackdrop.changed, notifications.*). The direct one
//! counts and the backend's copy is dropped, so every event arrives once (a second windows.changed refreshed Task
//! View with four more windows.list calls, a second notifications.new pulsed the bell again) and a copy that trails
//! the direct one (by up to 80 ms) never puts an older state back.
use crate::backend::Backend;
use crate::gnome::Gnome;
use gtk::glib;
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

type Listener = Rc<dyn Fn(&str, &Value)>;

/// The extension's notifications.closed and notifications.action that came over D-Bus and whose backend copy has
/// not come yet (they name no source, unlike notifications.new); a bound for a backend that relays nothing.
const AWAITED: usize = 64;

pub struct Shell {
    pub app: gtk::Application,
    pub backend: Backend,
    pub gnome: Gnome,
    settings: RefCell<Value>,
    listeners: RefCell<Vec<(String, Listener)>>,
    pub flyouts: RefCell<Option<Rc<crate::flyouts::FlyoutHost>>>,
    // an extension event came over D-Bus: the backend's copies of them are not needed
    direct: Cell<bool>,
    awaited: RefCell<VecDeque<(String, Value)>>,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

/// The process's shell (main thread).
pub fn shell() -> Rc<Shell> {
    SHELL.with(|s| s.borrow().clone()).expect("shell not initialized")
}

/// From any thread: deliver an event to the shell's listeners on the main thread.
pub fn dispatch(event: String, data: Value) {
    glib::MainContext::default().invoke(move || {
        if let Some(shell) = SHELL.with(|s| s.borrow().clone()) {
            shell.emit(&event, &data);
        }
    });
}

/// The extension's signal (gnome.rs, on the main thread).
fn dispatch_gnome(event: String, data: Value) {
    glib::MainContext::default().invoke(move || {
        if let Some(shell) = SHELL.with(|s| s.borrow().clone()) {
            shell.direct.set(true);
            if matches!(event.as_str(), "notifications.closed" | "notifications.action") {
                let mut awaited = shell.awaited.borrow_mut();
                if awaited.len() >= AWAITED {
                    awaited.pop_front();
                }
                awaited.push_back((event.clone(), data.clone()));
            }
            shell.emit(&event, &data);
        }
    });
}

/// The backend's event stream (its reader thread): all but the copies of what the extension's signal brought.
fn dispatch_backend(event: String, data: Value) {
    glib::MainContext::default().invoke(move || {
        if let Some(shell) = SHELL.with(|s| s.borrow().clone())
            && !shell.relayed(&event, &data)
        {
            shell.emit(&event, &data);
        }
    });
}

/// Whether a backend event is a copy of the extension's (`awaited`: the signal's closed and action events whose
/// copy is still due; a match is taken out). What only the backend sends stays: the backdrop's reset when the
/// extension went or came (NameOwnerChanged), the notifications of its own store (no GNOME, a test: another
/// source) and their closed and action events, which no signal matches.
fn relayed_copy(event: &str, data: &Value, awaited: &mut VecDeque<(String, Value)>) -> bool {
    match event {
        "windows.changed" | "windows.focused" => true,
        "nativeBackdrop.changed" => data["reset"] != true,
        "notifications.new" => data["notification"]["source"] == "gnome",
        "notifications.closed" | "notifications.action" => {
            let copy = awaited.iter().position(|(e, d)| e == event && d == data);
            copy.and_then(|i| awaited.remove(i)).is_some()
        }
        _ => false,
    }
}

/// Whether a settings.changed touches one of `keys` (dotted paths; a key covers what lies under it, and a change
/// of a whole object above it counts too). The host sends the leaf paths that changed (`changed`,
/// host/src/settings.rs `diff_paths`, which stops at 200); an event without that list, or with a full one, touches
/// every key (backend.ready's first settings, and `Value::Null` for a first render).
pub fn touches(data: &Value, keys: &[&str]) -> bool {
    let Some(changed) = data["changed"].as_array().filter(|c| c.len() < 200) else { return true };
    let under = |path: &str, key: &str| path == key || path.strip_prefix(key).is_some_and(|rest| rest.starts_with('.'));
    changed.iter().filter_map(Value::as_str).any(|path| keys.iter().any(|key| under(path, key) || under(key, path)))
}

/// The user's settings.json as the host keeps it (XDG_CONFIG_HOME or ~/.config, win11-gnome/settings.json).
fn settings_file() -> Option<Value> {
    let dir = std::env::var_os("WIN11_GNOME_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"));
    serde_json::from_slice(&std::fs::read(dir.join("settings.json")).ok()?).ok()
}

/// From the backend's reader thread: it answers now.
pub fn backend_ready() {
    dispatch("backend.ready".into(), Value::Null);
}

impl Shell {
    pub fn new(app: &gtk::Application, backend: Backend) -> Rc<Shell> {
        let gnome = Gnome::new();
        // The first paint comes from the settings file (the backend may still be starting); backend.ready
        // replaces it with the backend's answer (D-G67).
        let settings = settings_file().unwrap_or_else(|| json!({}));
        let shell = Rc::new(Shell {
            app: app.clone(),
            backend,
            gnome,
            settings: RefCell::new(settings),
            listeners: RefCell::new(Vec::new()),
            flyouts: RefCell::new(None),
            direct: Cell::new(false),
            awaited: RefCell::new(VecDeque::new()),
        });
        SHELL.with(|s| *s.borrow_mut() = Some(shell.clone()));
        shell.backend.events(dispatch_backend);
        shell.gnome.events(dispatch_gnome);
        shell.on("backend.ready", |_, _| {
            crate::core::shell().call("settings.get", json!({}), |result| {
                if let Ok(settings) = result && settings.is_object() {
                    let shell = crate::core::shell();
                    *shell.settings.borrow_mut() = settings.clone();
                    shell.emit("settings.changed", &json!({"settings": settings, "path": null}));
                }
            });
        });
        shell.on("settings.changed", |_, data| {
            let shell = crate::core::shell();
            if let Some(settings) = data.get("settings").filter(|s| s.is_object()) {
                *shell.settings.borrow_mut() = settings.clone();
            } else if let Ok(settings) = shell.backend.call("settings.get", &json!({})) {
                *shell.settings.borrow_mut() = settings;
            }
        });
        shell
    }

    /// Listen to events whose name equals `prefix` or starts with it followed by '.' (e.g. "tray" gets
    /// "tray.changed"); "" listens to all.
    pub fn on(&self, prefix: &str, f: impl Fn(&str, &Value) + 'static) {
        self.listeners.borrow_mut().push((prefix.to_owned(), Rc::new(f)));
    }

    /// A backend event that is the relayed copy of one the extension's signal brought. Until a signal came (no
    /// session bus of our own) the backend's copies are the only ones.
    fn relayed(&self, event: &str, data: &Value) -> bool {
        self.direct.get() && relayed_copy(event, data, &mut self.awaited.borrow_mut())
    }

    pub fn emit(&self, event: &str, data: &Value) {
        let matching: Vec<Listener> = self
            .listeners
            .borrow()
            .iter()
            .filter(|(p, _)| p.is_empty() || event == p || event.starts_with(&format!("{p}.")))
            .map(|(_, f)| f.clone())
            .collect();
        for f in matching {
            f(event, data);
        }
    }

    /// A backend call off the main thread; `done` runs on the main thread.
    pub fn call(&self, method: &str, args: Value, done: impl FnOnce(Result<Value, String>) + 'static) {
        let backend = self.backend.clone();
        let method = method.to_owned();
        glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(move || backend.call(&method, &args))
                .await
                .unwrap_or_else(|_| Err("backend thread panicked".into()));
            done(result);
        });
    }

    /// A backend call whose answer does not matter.
    pub fn fire(&self, method: &str, args: Value) {
        self.call(method, args, |result| {
            if let Err(error) = result {
                eprintln!("win11-shell: {error}");
            }
        });
    }

    pub fn settings(&self) -> Value {
        self.settings.borrow().clone()
    }

    /// A setting by dotted path ("taskbar.pinned"), Null when absent.
    pub fn setting(&self, path: &str) -> Value {
        let settings = self.settings.borrow();
        let mut v = &*settings;
        for part in path.split('.') {
            match v.get(part) {
                Some(next) => v = next,
                None => return Value::Null,
            }
        }
        v.clone()
    }

    pub fn set_setting(&self, path: &str, value: Value) {
        self.fire("settings.set", json!({"path": path, "value": value}));
    }

    pub fn lang(&self) -> &'static str {
        if self.setting("language").as_str() == Some("en") { "en" } else { "nl" }
    }

    /// Dutch or English text by the shell's language.
    pub fn t(&self, nl: &str, en: &str) -> String {
        if self.lang() == "en" { en.to_owned() } else { nl.to_owned() }
    }

    pub fn dark(&self) -> bool {
        match self.setting("theme.mode").as_str() {
            Some("light") => false,
            Some("dark") => true,
            _ => true,
        }
    }

    pub fn flyouts(&self) -> Rc<crate::flyouts::FlyoutHost> {
        self.flyouts.borrow().clone().expect("panel not installed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touches_follows_the_changed_paths() {
        let known = json!({"path": "taskbar.tray.known", "changed": ["taskbar.tray.known.sni:org.kde.Nord-1"]});
        assert!(!touches(&known, &["start", "theme", "accessibility", "taskbar.pinned"]));
        assert!(!touches(&known, &["taskbar.tray.hiddenMenu", "taskbar.tray.shown", "taskbar.tray.order"]));
        assert!(touches(&known, &["taskbar.tray"]));
        // a key above or below the changed path, not a sibling with the same start
        let pins = json!({"changed": ["start.pinned"]});
        assert!(touches(&pins, &["start"]) && touches(&pins, &["start.pinned"]));
        assert!(!touches(&json!({"changed": ["start.pinnedOld"]}), &["start.pinned"]));
        assert!(touches(&json!({"changed": ["taskbar"]}), &["taskbar.tray.shown"]));
        // no list (backend.ready, a first render) or a cut-off one: everything
        assert!(touches(&json!({"path": null}), &["start"]));
        assert!(touches(&Value::Null, &["start"]));
        let many: Vec<String> = (0..200).map(|i| format!("explorer.columns.c{i}")).collect();
        assert!(touches(&json!({"changed": many}), &["start"]));
        assert!(!touches(&json!({"changed": []}), &["start"]));
    }

    #[test]
    fn backend_copies_of_extension_events_are_dropped_once() {
        let mut awaited = VecDeque::new();
        assert!(relayed_copy("windows.changed", &json!({"windows": []}), &mut awaited));
        assert!(relayed_copy("windows.focused", &json!({"id": 7}), &mut awaited));
        assert!(relayed_copy("nativeBackdrop.changed", &json!({"available": true, "windows": []}), &mut awaited));
        // the backend's own: the extension's name owner changed
        let reset = json!({"available": false, "reset": true, "windows": []});
        assert!(!relayed_copy("nativeBackdrop.changed", &reset, &mut awaited));
        // GNOME's notifications by their source; the backend's own store keeps its events
        let gnome = json!({"notification": {"id": 3, "source": "gnome"}});
        assert!(relayed_copy("notifications.new", &gnome, &mut awaited));
        let own = json!({"notification": {"id": 3, "source": "dbus"}});
        assert!(!relayed_copy("notifications.new", &own, &mut awaited));
        let closed = json!({"id": 3, "reason": 2});
        assert!(!relayed_copy("notifications.closed", &closed, &mut awaited));
        awaited.push_back(("notifications.closed".into(), closed.clone()));
        assert!(relayed_copy("notifications.closed", &closed, &mut awaited));
        assert!(!relayed_copy("notifications.closed", &closed, &mut awaited));
        assert!(!relayed_copy("tray.changed", &json!({"items": []}), &mut awaited));
        assert!(!relayed_copy("settings.changed", &json!({"changed": []}), &mut awaited));
    }
}
