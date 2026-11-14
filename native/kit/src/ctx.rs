//! The app's context: backend client, GNOME extension client, settings cache and an event bus. One per process,
//! on the GTK main thread only (`Rc`), made at startup by `kit::run` and reachable as `kit::ctx()`. Other threads
//! reach the main thread through `dispatch` (events) or `kit::spawn` (work with a result).
use crate::app::AppSpec;
use crate::backend::{Backend, CallError};
use crate::gnome::Gnome;
use crate::theme::Look;
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

type Listener = Rc<dyn Fn(&str, &Value)>;
type SettingsListener = Rc<dyn Fn(&Value, Option<&str>)>;

/// A handle to remove a listener again (`Ctx::off`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListenerId(u64);

pub struct Ctx {
    app: gtk::Application,
    spec: AppSpec,
    backend: Backend,
    gnome: Gnome,
    settings: RefCell<Value>,
    listeners: RefCell<Vec<(u64, String, Listener)>>,
    settings_listeners: RefCell<Vec<SettingsListener>>,
    next_id: Cell<u64>,
    /// settings.changed events received: a settings.set reply older than one of them is not applied.
    ev_seq: Cell<u64>,
    /// JSON of the settings the listeners last received.
    notified: RefCell<String>,
    system_dark: Cell<bool>,
    gsettings: RefCell<Option<gio::Settings>>,
    monitor: RefCell<Option<gio::FileMonitor>>,
    reload_pending: Cell<bool>,
}

thread_local! {
    static CTX: RefCell<Option<Rc<Ctx>>> = const { RefCell::new(None) };
}

/// The process's context (main thread, after `kit::run`'s startup). Panics before that.
pub fn ctx() -> Rc<Ctx> {
    try_ctx().expect("kit::ctx() used before kit::run started the app")
}

/// The context when it exists.
pub fn try_ctx() -> Option<Rc<Ctx>> {
    CTX.with(|c| c.borrow().clone())
}

/// From any thread: deliver an event to the listeners on the main thread.
pub fn dispatch(event: String, data: Value) {
    let deliver = move || {
        if let Some(ctx) = try_ctx() {
            ctx.incoming(&event, &data);
        }
    };
    let main = glib::MainContext::default();
    if main.is_owner() {
        // invoke() would run it at once here: at startup (WIN11_APP_BACKEND) "backend.ready" comes before the
        // context exists, so the settings were never fetched. Later on the main thread it keeps the event order.
        main.spawn_local(async move { deliver() });
    } else {
        main.invoke(deliver);
    }
}

fn stream_status(connected: bool) {
    dispatch("bridge.status".into(), json!({ "connected": connected }));
}

/// The settings file the host keeps (XDG_CONFIG_HOME or ~/.config, win11-gnome/settings.json;
/// WIN11_GNOME_CONFIG_DIR overrides the folder).
pub fn settings_path() -> std::path::PathBuf {
    let dir = std::env::var_os("WIN11_GNOME_CONFIG_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(glib::user_config_dir)
            .join("win11-gnome")
    });
    dir.join("settings.json")
}

fn settings_file() -> Option<Value> {
    serde_json::from_slice(&std::fs::read(settings_path()).ok()?).ok().filter(Value::is_object)
}

/// GNOME's own light/dark choice (org.gnome.desktop.interface color-scheme), dark when unknown.
fn gnome_settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup("org.gnome.desktop.interface", true)?;
    if !schema.has_key("color-scheme") {
        return None;
    }
    Some(gio::Settings::new("org.gnome.desktop.interface"))
}

fn prefers_dark(s: &Option<gio::Settings>) -> bool {
    match s {
        Some(s) => s.string("color-scheme").as_str() == "prefer-dark",
        None => true,
    }
}

const LAUNCH_METHODS: &[&str] = &["files.open", "apps.launch", "files.openWith"];

impl Ctx {
    /// Made by kit::run at startup.
    pub(crate) fn create(app: &gtk::Application, spec: AppSpec) -> Rc<Ctx> {
        let gs = gnome_settings();
        let system_dark = prefers_dark(&gs);
        let settings = settings_file().unwrap_or_else(|| json!({}));
        crate::i18n::set_lang(if settings["language"].as_str() == Some("en") { "en" } else { "nl" });
        let backend = if spec.backend {
            match Backend::start(spec.kind, || dispatch("backend.ready".into(), Value::Null)) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("win11-kit: backend: {e}");
                    Backend::none(&e)
                }
            }
        } else {
            Backend::none("no backend")
        };
        backend.set_lang(crate::i18n::lang());
        let ctx = Rc::new(Ctx {
            app: app.clone(),
            spec,
            backend,
            gnome: Gnome::new(),
            notified: RefCell::new(settings.to_string()),
            settings: RefCell::new(settings),
            listeners: RefCell::new(Vec::new()),
            settings_listeners: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
            ev_seq: Cell::new(0),
            system_dark: Cell::new(system_dark),
            gsettings: RefCell::new(None),
            monitor: RefCell::new(None),
            reload_pending: Cell::new(false),
        });
        CTX.with(|c| *c.borrow_mut() = Some(ctx.clone()));
        if let Some(gs) = &gs {
            gs.connect_changed(Some("color-scheme"), |s, _| {
                let Some(ctx) = try_ctx() else { return };
                ctx.system_dark.set(prefers_dark(&Some(s.clone())));
                ctx.apply_look();
                let s = ctx.settings();
                let mode = s["theme"]["mode"].as_str().unwrap_or("");
                if mode != "light" && mode != "dark" {
                    ctx.notify(Some("theme.mode"));
                }
            });
        }
        *ctx.gsettings.borrow_mut() = gs;
        ctx.backend.events(dispatch, stream_status);
        ctx.gnome.events(|event, data| {
            if let Some(ctx) = try_ctx() {
                ctx.incoming(&event, &data);
            }
        });
        ctx.watch_file();
        ctx
    }

    /// The GTK application (for windows: `AppWindow::new` takes it from here).
    pub fn app(&self) -> &gtk::Application {
        &self.app
    }

    /// The app's spec as given to `kit::run`.
    pub fn spec(&self) -> &AppSpec {
        &self.spec
    }

    /// The app's kind ("explorer", "taskmgr", ...).
    pub fn kind(&self) -> &'static str {
        self.spec.kind
    }

    /// The backend client (Send + Sync + Clone): its blocking `call` and `get` for worker threads.
    pub fn backend(&self) -> Backend {
        self.backend.clone()
    }

    /// The GNOME extension client.
    pub fn gnome(&self) -> &Gnome {
        &self.gnome
    }

    /// Whether the backend's event stream is connected now (events "bridge.status" {connected} on changes).
    pub fn connected(&self) -> bool {
        self.backend.connected()
    }

    // ------------------------------------------------------------------------------------------- calls

    /// A backend call off the main thread; `done` runs on the main thread. Calls made while the backend is
    /// still starting wait for it. files.open / apps.launch / files.openWith that the host refuses as an
    /// untrusted launcher ask "Deze snelkoppeling starten?" first and retry once allowed (bridge.js).
    pub fn call(&self, method: &str, args: Value, done: impl FnOnce(Result<Value, CallError>) + 'static) {
        let backend = self.backend.clone();
        let method = method.to_owned();
        glib::spawn_future_local(async move {
            let b = backend.clone();
            let (m, a) = (method.clone(), args.clone());
            let result = gio::spawn_blocking(move || b.call(&m, &a))
                .await
                .unwrap_or_else(|_| Err(CallError::new("internal", "backend thread panicked")));
            match result {
                Err(e)
                    if e.code == "untrusted"
                        && LAUNCH_METHODS.contains(&method.as_str())
                        && e.data["canTrust"] == true
                        && e.data["path"].is_string() =>
                {
                    crate::controls::dialog::confirm_launcher(e, method, args, Box::new(done));
                }
                other => done(other),
            }
        });
    }

    /// `call` as a future (for `glib::spawn_future_local(async move { ... })`).
    pub async fn call_async(&self, method: &str, args: Value) -> Result<Value, CallError> {
        let (tx, rx) = futures_channel_oneshot();
        self.call(method, args, move |r| tx(r));
        rx.await
    }

    /// A backend call whose answer does not matter (errors go to stderr).
    pub fn fire(&self, method: &str, args: Value) {
        let name = method.to_owned();
        self.call(method, args, move |result| {
            if let Err(error) = result {
                eprintln!("win11-kit: {name}: {error}");
            }
        });
    }

    /// GET a backend path ("/icon?name=..&size=..", "/thumb?path=..&size=..") as bytes; `done` on the main
    /// thread.
    pub fn get(&self, path_and_query: &str, done: impl FnOnce(Result<Vec<u8>, CallError>) + 'static) {
        let backend = self.backend.clone();
        let q = path_and_query.to_owned();
        glib::spawn_future_local(async move {
            let result = gio::spawn_blocking(move || backend.get(&q))
                .await
                .unwrap_or_else(|_| Err(CallError::new("internal", "backend thread panicked")));
            done(result);
        });
    }

    /// The blocking call, for worker threads that hold a `Backend` (see `backend()`); on the main thread it
    /// blocks the UI, so prefer `call`.
    pub fn call_blocking(&self, method: &str, args: &Value) -> Result<Value, CallError> {
        self.backend.call(method, args)
    }

    /// An extension call (D-Bus, asynchronous); `done` on the main thread.
    pub fn gnome_call(&self, method: &str, args: Value, done: impl FnOnce(Result<Value, CallError>) + 'static) {
        self.gnome.call(method, &args, done);
    }

    /// An extension call without waiting for the answer.
    pub fn gnome_send(&self, method: &str, args: Value) {
        self.gnome.send(method, &args);
    }

    // ------------------------------------------------------------------------------------------ events

    /// Listen to events whose name equals `prefix` or starts with it followed by '.' ("files" gets
    /// "files.changed"; "" gets everything): the backend's SSE events, the extension's Event signal
    /// (windows.*, nativeBackdrop.changed, ...), and the kit's own "backend.ready" (data null for the first
    /// backend, {respawn: true} for a new one after the last ended: watches and subscriptions are gone) and
    /// "bridge.status".
    pub fn on(&self, prefix: &str, f: impl Fn(&str, &Value) + 'static) -> ListenerId {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.listeners.borrow_mut().push((id, prefix.to_owned(), Rc::new(f)));
        ListenerId(id)
    }

    /// Remove a listener added with `on`.
    pub fn off(&self, id: ListenerId) {
        self.listeners.borrow_mut().retain(|(i, _, _)| *i != id.0);
    }

    /// Deliver an event to the listeners (also for app-internal events).
    pub fn emit(&self, event: &str, data: &Value) {
        let matching: Vec<Listener> = self
            .listeners
            .borrow()
            .iter()
            .filter(|(_, p, _)| p.is_empty() || event == p || (event.len() > p.len() && event.starts_with(p.as_str()) && event.as_bytes()[p.len()] == b'.'))
            .map(|(_, _, f)| f.clone())
            .collect();
        for f in matching {
            f(event, data);
        }
    }

    fn incoming(&self, event: &str, data: &Value) {
        match event {
            // the backend ended with a restart note (backend.rs): exec, quit or a new backend (app.rs); the kit's own
            "backend.restart" => return crate::app::backend_restarted(self),
            // also after a new backend ("respawn": true): its settings, which may have changed meanwhile
            "backend.ready" => self.reload_settings(),
            "settings.changed" => {
                self.ev_seq.set(self.ev_seq.get() + 1);
                match data.get("settings").filter(|s| s.is_object()) {
                    Some(s) => self.replace_settings(s.clone(), data["path"].as_str()),
                    None => self.reload_settings(),
                }
            }
            _ => {}
        }
        self.emit(event, data);
    }

    // ---------------------------------------------------------------------------------------- settings

    /// The settings (settings.get), cached and kept live.
    pub fn settings(&self) -> Value {
        self.settings.borrow().clone()
    }

    /// One setting by dotted path ("taskbar.alignment"); Null when absent.
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

    /// Change a setting through the backend's settings.set, applied at once (theme.js setSetting: optimistic,
    /// reverted when the call fails).
    pub fn set_setting(&self, path: &str, value: Value) {
        self.set_setting_then(path, value, |result| {
            if let Err(e) = result {
                eprintln!("win11-kit: settings.set: {e}");
            }
        });
    }

    /// `set_setting` with the outcome: Ok(settings) or the error (the change is undone then).
    pub fn set_setting_then(&self, path: &str, value: Value, done: impl FnOnce(Result<Value, CallError>) + 'static) {
        let prev = self.settings();
        let mut next = prev.clone();
        set_path(&mut next, path, value.clone());
        self.replace_settings(next, Some(path));
        let seq = self.ev_seq.get();
        let p = path.to_owned();
        self.call("settings.set", json!({ "path": path, "value": value }), move |result| {
            let Some(ctx) = try_ctx() else { return };
            match result {
                Ok(s) => {
                    if s.is_object() && ctx.ev_seq.get() == seq {
                        ctx.replace_settings(s.clone(), Some(&p));
                    }
                    done(Ok(ctx.settings()));
                }
                Err(e) => {
                    ctx.replace_settings(prev, Some(&p));
                    done(Err(e));
                }
            }
        });
    }

    /// Run `f(settings, path)` after every settings change (path None: many keys). Language and theme are
    /// already applied when it runs.
    pub fn on_settings(&self, f: impl Fn(&Value, Option<&str>) + 'static) {
        self.settings_listeners.borrow_mut().push(Rc::new(f));
    }

    /// Fetch the settings again (settings.get).
    pub fn reload_settings(&self) {
        self.call("settings.get", json!({}), |result| {
            if let (Ok(s), Some(ctx)) = (result, try_ctx())
                && s.is_object()
            {
                ctx.replace_settings(s, None);
            }
        });
    }

    fn replace_settings(&self, s: Value, path: Option<&str>) {
        *self.settings.borrow_mut() = s;
        self.apply_look();
        let text = self.settings.borrow().to_string();
        if *self.notified.borrow() != text {
            self.notify(path);
        }
    }

    fn notify(&self, path: Option<&str>) {
        *self.notified.borrow_mut() = self.settings.borrow().to_string();
        let s = self.settings();
        let listeners: Vec<SettingsListener> = self.settings_listeners.borrow().clone();
        for f in listeners {
            f(&s, path);
        }
    }

    pub(crate) fn apply_look(&self) {
        let s = self.settings();
        let lang = if s["language"].as_str() == Some("en") { "en" } else { "nl" };
        crate::i18n::set_lang(lang);
        self.backend.set_lang(crate::i18n::lang());
        let look = Look::from_settings(&s, self.system_dark.get());
        crate::theme::apply(&look, false);
        crate::backdrop::settings_changed(look.transparency, look.high_contrast);
    }

    /// While the backend is not there, settings.json itself is watched (the calendar's way).
    fn watch_file(&self) {
        let file = gio::File::for_path(settings_path());
        let Ok(monitor) = file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) else { return };
        monitor.connect_changed(|_, _, _, event| {
            if matches!(event, gio::FileMonitorEvent::AttributeChanged | gio::FileMonitorEvent::PreUnmount) {
                return;
            }
            let Some(ctx) = try_ctx() else { return };
            if ctx.backend.is_ready() || ctx.reload_pending.replace(true) {
                return;
            }
            glib::timeout_add_local_once(Duration::from_millis(150), || {
                let Some(ctx) = try_ctx() else { return };
                ctx.reload_pending.set(false);
                if let Some(s) = settings_file() {
                    ctx.replace_settings(s, None);
                }
            });
        });
        *self.monitor.borrow_mut() = Some(monitor);
    }

    // ---------------------------------------------------------------------------------------- the look

    /// "nl" or "en" (settings.language).
    pub fn lang(&self) -> &'static str {
        crate::i18n::lang()
    }

    /// The Dutch or English text by the current language.
    pub fn t<'a>(&self, nl: &'a str, en: &'a str) -> &'a str {
        crate::i18n::t(nl, en)
    }

    /// Whether the dark theme is in effect (theme.mode, otherwise GNOME's colour scheme).
    pub fn dark(&self) -> bool {
        crate::theme::dark()
    }

    /// theme.accent, the base colour the user picked ("#0078d4" by default).
    pub fn accent(&self) -> String {
        self.setting("theme.accent").as_str().unwrap_or("#0078d4").to_owned()
    }

    /// accessibility.textScale / 100 (1.0 ..= 2.25).
    pub fn text_scale(&self) -> f64 {
        crate::theme::look().text_scale
    }

    /// accessibility.highContrast.
    pub fn high_contrast(&self) -> bool {
        crate::theme::look().high_contrast
    }

    /// theme.transparency (default on).
    pub fn transparency(&self) -> bool {
        crate::theme::look().transparency
    }

    /// accessibility.reduceMotion.
    pub fn reduce_motion(&self) -> bool {
        crate::theme::look().reduce_motion
    }

    /// time.h24 (24-hour clock; default on in Dutch, off in English).
    pub fn h24(&self) -> bool {
        self.setting("time.h24").as_bool().unwrap_or(self.lang() != "en")
    }
}

/// Set a dotted path in a JSON object, making objects on the way.
pub fn set_path(root: &mut Value, path: &str, value: Value) {
    if !root.is_object() {
        *root = json!({});
    }
    let mut cur = root;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            if let Some(obj) = cur.as_object_mut() {
                obj.insert((*part).to_owned(), value);
            }
            return;
        }
        let obj = match cur.as_object_mut() {
            Some(o) => o,
            None => return,
        };
        let next = obj.entry((*part).to_owned()).or_insert_with(|| json!({}));
        if !next.is_object() {
            *next = json!({});
        }
        cur = next;
    }
}

/// A one-shot channel for `call_async` without extra crates.
fn futures_channel_oneshot() -> (impl FnOnce(Result<Value, CallError>), impl std::future::Future<Output = Result<Value, CallError>>) {
    let slot: Rc<RefCell<(Option<Result<Value, CallError>>, Option<std::task::Waker>)>> = Rc::new(RefCell::new((None, None)));
    let tx_slot = slot.clone();
    let tx = move |r: Result<Value, CallError>| {
        let waker = {
            let mut s = tx_slot.borrow_mut();
            s.0 = Some(r);
            s.1.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    };
    let rx = std::future::poll_fn(move |cx| {
        let mut s = slot.borrow_mut();
        match s.0.take() {
            Some(r) => std::task::Poll::Ready(r),
            None => {
                s.1 = Some(cx.waker().clone());
                std::task::Poll::Pending
            }
        }
    });
    (tx, rx)
}

/// Run `work` on a worker thread and `done(result)` on the main thread.
pub fn spawn<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static, done: impl FnOnce(T) + 'static) {
    glib::spawn_future_local(async move {
        if let Ok(v) = gio::spawn_blocking(work).await {
            done(v);
        }
    });
}
