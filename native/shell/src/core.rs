//! The shared state of the shell: the backend client, the GNOME extension client, the settings cache and an
//! event bus. One `Shell` per process, used from the GTK main thread only (`Rc`); other threads reach it through
//! `dispatch`, which runs on the main context.
use crate::backend::Backend;
use crate::gnome::Gnome;
use gtk::glib;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;

type Listener = Rc<dyn Fn(&str, &Value)>;

pub struct Shell {
    pub app: gtk::Application,
    pub backend: Backend,
    pub gnome: Gnome,
    settings: RefCell<Value>,
    listeners: RefCell<Vec<(String, Listener)>>,
    pub flyouts: RefCell<Option<Rc<crate::flyouts::FlyoutHost>>>,
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
        });
        SHELL.with(|s| *s.borrow_mut() = Some(shell.clone()));
        shell.backend.events(dispatch);
        shell.gnome.events(dispatch);
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
