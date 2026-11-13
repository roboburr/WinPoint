//! Native app discovery, launching, associations and recent-file history for the original UI.
use crate::files::{self, ApiError, ApiResult};
use gio::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU8, Ordering as AtomicOrdering},
        mpsc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
mod catalog;
mod defaults;
mod pertype;
mod recent;

pub(crate) struct Apps {
    catalog: Arc<catalog::Catalog>,
    files: Arc<files::Files>,
    config: PathBuf,
    jobs: Mutex<(bool, usize)>,
    idle: Condvar,
    monitor: Monitor,
}
impl Apps {
    pub(crate) fn new(files: Arc<files::Files>) -> Result<Option<Arc<Self>>, String> {
        if std::env::var_os("WIN11_REFERENCE_INSTANCE").is_some() {
            return Ok(None);
        }
        let config = std::env::var_os("WIN11_GNOME_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"));
        let catalog = Arc::new(catalog::Catalog::new(config.clone()));
        let monitor = Monitor::new(catalog.clone(), files.clone(), config.clone())?;
        Ok(Some(Arc::new(Self {
            catalog,
            files,
            config,
            jobs: Mutex::new((false, 0)),
            idle: Condvar::new(),
            monitor,
        })))
    }
    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !matches!(
            method,
            "apps.list"
                | "apps.launch"
                | "apps.recent"
                | "apps.clearRecent"
                | "apps.forFile"
                | "apps.defaults"
                | "apps.setDefault"
                | "apps.mimeTypes"
                | "apps.forType"
                | "apps.setDefaultForType"
                | "apps.openUri"
        ) {
            return None;
        }
        // C1: Win11's own settings language, by one cached stat (files::Context reads the whole file).
        let lang = crate::i18n::language_in(&self.config);
        let _job = {
            let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
            if jobs.0 {
                return Some(Err(ApiError::tr("offline", lang, "apps.shuttingDown", crate::i18n::NO_PARAMS)));
            }
            jobs.1 += 1;
            Job(self)
        };
        let empty = json!({});
        let args = if args.is_null() { &empty } else { args };
        if !args.is_object() {
            return Some(Err(ApiError::tr("bad_args", lang, "apps.invalidInput", crate::i18n::NO_PARAMS)));
        }
        // The original registry skips these decorated methods before entering their bodies.
        if crate::backend::dry_run() && matches!(method, "apps.setDefault" | "apps.clearRecent") {
            return Some(Ok(json!({"ok":true,"dryRun":true})));
        }
        if let Some(result) = pertype::call(method, args, &self.catalog, lang.code()) {
            if method == "apps.setDefaultForType"
                && result.as_ref().is_ok_and(|v| v["dryRun"] != true)
            {
                self.files.events.emit_local("apps.changed", json!({}));
            }
            return Some(result);
        }
        if let Some(result) = defaults::call(method, args, &self.catalog, lang.code()) {
            return Some(result);
        }
        if method == "apps.recent" || method == "apps.clearRecent" {
            let settings = fs::read(self.config.join("settings.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .unwrap_or(Value::Null);
            return recent::call(method, args, &settings, &self.files.events);
        }
        Some(match method {
            "apps.list" => self.catalog.list(lang.code()),
            "apps.launch" => (|| {
                let id = args["id"]
                    .as_str()
                    .filter(|id| {
                        !id.is_empty()
                            && !id.contains('\0')
                            && id.chars().count() <= if id.starts_with('/') { 4096 } else { 300 }
                    })
                    .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidApp", crate::i18n::NO_PARAMS))?;
                let uris = clean_uris(&args["uris"], lang)?;
                files::actions::launch_app(id, &uris, &self.files.context())
            })(),
            "apps.openUri" => match args["uri"].as_str() {
                Some(uri) => files::actions::open_uri(uri, &self.files.context()),
                None => Err(ApiError::tr("bad_uri", lang, "apps.linkCantOpen", crate::i18n::NO_PARAMS)),
            },
            _ => unreachable!(),
        })
    }
    pub(crate) fn shutdown(&self) {
        let mut jobs = self.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.0 = true;
        while jobs.1 > 0 {
            jobs = self.idle.wait(jobs).unwrap_or_else(|e| e.into_inner());
        }
        drop(jobs);
        self.monitor.shutdown();
    }
}
struct Job<'a>(&'a Apps);
impl Drop for Job<'_> {
    fn drop(&mut self) {
        let mut jobs = self.0.jobs.lock().unwrap_or_else(|e| e.into_inner());
        jobs.1 -= 1;
        self.0.idle.notify_all();
    }
}
fn clean_uris(value: &Value, lang: crate::i18n::Lang) -> Result<Vec<String>, ApiError> {
    let values = match value {
        Value::Null => Vec::new(),
        Value::String(_) => vec![value],
        Value::Array(values) if values.len() <= 500 => values.iter().collect(),
        _ => return Err(ApiError::tr("bad_args", lang, "apps.invalidInput", crate::i18n::NO_PARAMS)),
    };
    values
        .into_iter()
        .map(|value| {
            let uri = value
                .as_str()
                .filter(|uri| {
                    !uri.is_empty() && uri.chars().count() <= 8192 && !uri.chars().any(|c| c < ' ')
                })
                .ok_or_else(|| ApiError::tr("bad_args", lang, "apps.invalidInput", crate::i18n::NO_PARAMS))?;
            if uri.starts_with('/') {
                let path = files::abs(&json!(uri))?;
                return Ok(path_uri(&path));
            }
            let valid = uri.split_once(':').is_some_and(|(scheme, _)| {
                scheme
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
                    && scheme
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"+.-".contains(&b))
            });
            if !valid {
                return Err(ApiError::tr("bad_args", lang, "apps.invalidInput", crate::i18n::NO_PARAMS));
            }
            Ok(uri.to_owned())
        })
        .collect()
}
fn path_uri(path: &std::path::Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut out = "file://".to_owned();
    for b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"/!$&'()*+,;=:@-._~".contains(b) {
            out.push(*b as char);
        } else {
            use std::fmt::Write;
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}
struct Monitor {
    sender: Mutex<Option<glib::Sender<()>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}
impl Monitor {
    fn new(
        catalog: Arc<catalog::Catalog>,
        files: Arc<files::Files>,
        config: PathBuf,
    ) -> Result<Self, String> {
        let (ready, answer) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("win11-app-catalog".into())
            .spawn(move || {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let mainloop = glib::MainLoop::new(Some(&context), false);
                    #[allow(deprecated)]
                    let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
                    let stopping = mainloop.clone();
                    receiver.attach(Some(&context), move |()| {
                        stopping.quit();
                        glib::ControlFlow::Break
                    });
                    let dirty = Arc::new(Mutex::new(None::<Instant>));
                    let observed = dirty.clone();
                    let app_monitor = gio::AppInfoMonitor::get();
                    let signal = app_monitor.connect_changed(move |_| {
                        observed
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .get_or_insert_with(|| Instant::now() + Duration::from_secs(1));
                    });
                    // The catalogue follows Win11's language (C1), read by one cached stat
                    // (i18n::language_in) instead of a full settings read per check. On a change it is rebuilt
                    // here at once, before apps.changed goes out, so the page's apps.list is already in the new
                    // language (round1/cross.md §7 item 2: Start tiles kept the old language for 0.35-0.56 s).
                    let initial = crate::i18n::language_in(&config);
                    let initial_catalog = catalog.clone();
                    let watched = config.clone();
                    let applied = Arc::new(AtomicU8::new(initial.as_u8()));
                    // Only an app change (due) invalidates: a language change needs no invalidate, because
                    // ensure() rebuilds on a language mismatch, so a catalogue that a page's apps.list already
                    // rebuilt in the new language is not built a second time (review n2).
                    let refresh = Arc::new(move |due: bool| {
                        let next = crate::i18n::language_in(&config);
                        let changed = applied.swap(next.as_u8(), AtomicOrdering::SeqCst) != next.as_u8();
                        if due {
                            catalog.invalidate();
                        }
                        if changed {
                            let _ = catalog.list(next.code());
                        }
                        if changed || due {
                            files.events.emit_local("apps.changed", json!({}));
                        }
                    });
                    let polled = refresh.clone();
                    let source = glib::timeout_source_new(
                        Duration::from_millis(150),
                        Some("win11-apps-refresh"),
                        glib::Priority::DEFAULT,
                        move || {
                            let due = {
                                let mut dirty = dirty.lock().unwrap_or_else(|e| e.into_inner());
                                if dirty.is_some_and(|at| at <= Instant::now()) {
                                    *dirty = None;
                                    true
                                } else {
                                    false
                                }
                            };
                            polled(due);
                            glib::ControlFlow::Continue
                        },
                    );
                    source.attach(Some(&context));
                    // The settings store writes settings.json by rename: react to that at once; the 150 ms
                    // poll above stays as the fallback (no monitor, an in-place edit, a coalesced event).
                    let settings_monitor = gio::File::for_path(&watched)
                        .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
                        .ok();
                    let settings_signal = settings_monitor.as_ref().map(|monitor| {
                        let changed = refresh.clone();
                        monitor.connect_changed(move |_, file, other, _| {
                            let named = |f: Option<&gio::File>| {
                                f.and_then(|f| f.basename())
                                    .is_some_and(|name| name.as_os_str() == "settings.json")
                            };
                            if named(Some(file)) || named(other) {
                                changed(false);
                            }
                        })
                    });
                    let _ = ready.send(sender);
                    let _ = initial_catalog.list(initial.code());
                    mainloop.run();
                    source.destroy();
                    if let (Some(monitor), Some(signal)) = (settings_monitor, settings_signal) {
                        monitor.disconnect(signal);
                        monitor.cancel();
                    }
                    app_monitor.disconnect(signal);
                });
            })
            .map_err(|e| e.to_string())?;
        let sender = answer
            .recv_timeout(Duration::from_secs(10))
            .map_err(|e| format!("Appbewaking starten: {e}"))?;
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            thread: Mutex::new(Some(thread)),
        })
    }
    fn shutdown(&self) {
        if let Some(sender) = self.sender.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = sender.send(());
        }
        if let Some(thread) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.shutdown();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launch_uri_validation_and_literal_filename_encoding() {
        let args = json!([
            "/tmp/map met spaties/$(literal)#?",
            "HTTPS://example.invalid/x"
        ]);
        let uris = clean_uris(&args, crate::i18n::Lang::Nl).unwrap();
        assert_eq!(uris[0], "file:///tmp/map%20met%20spaties/$(literal)%23%3F");
        assert_eq!(uris[1], "HTTPS://example.invalid/x");
        for value in [
            json!(true),
            json!(["relative"]),
            json!(["http://a\nb"]),
            json!([null]),
        ] {
            assert!(clean_uris(&value, crate::i18n::Lang::Nl).is_err());
        }
        assert!(clean_uris(&Value::Null, crate::i18n::Lang::Nl).unwrap().is_empty());
    }

    /// The en render test of the `apps` stem (round2-gate §6), through the real call sites; nl is the 3b898e6 text.
    #[test]
    fn app_errors_render_in_the_settings_language_through_their_call_sites() {
        use crate::i18n::Lang;
        let config = std::env::temp_dir().join(format!("win11-apps-i18n-{}", crate::events::nonce()));
        let catalog = catalog::Catalog::new(config.clone());
        let message = |result: Option<ApiResult>| {
            let error = result.expect("handled").expect_err("an error");
            (error.message.clone(), error.json()["key"].clone())
        };
        let cases: [(&str, Value, &str, &str, &str); 7] = [
            ("apps.forType", json!({"mime":"bad"}), "apps.invalidFileType", "Ongeldig bestandstype.", "Invalid file type."),
            ("apps.mimeTypes", json!({"query":5}), "apps.invalidSearch", "Ongeldige zoekterm.", "Invalid search term."),
            ("apps.mimeTypes", json!({"limit":-1}), "apps.invalidInput", "Ongeldige invoer.", "Invalid input."),
            ("apps.setDefaultForType", json!({"mime":"text/plain","id":""}), "apps.invalidApp", "Ongeldige app.", "Invalid app."),
            ("apps.forFile", json!({"path":"relative"}), "apps.invalidPath", "Ongeldig pad.", "Invalid path."),
            ("apps.forFile", json!({"path":"/nonexistent-win11-r2d/x"}), "apps.fileGone", "Dit bestand bestaat niet (meer).", "This file doesn’t exist any more."),
            ("apps.setDefault", json!({"kind":"text","id":"explorer"}), "apps.cantBeDefault", "Deze app kan hier niet als standaard worden gekozen.", "This app can’t be chosen as the default here."),
        ];
        for (method, args, key, nl, en) in cases {
            for (lang, text) in [("nl", nl), ("en", en)] {
                let result = pertype::call(method, &args, &catalog, lang)
                    .or_else(|| defaults::call(method, &args, &catalog, lang));
                assert_eq!(message(result), (text.to_owned(), json!(key)), "{method} {args} {lang}");
            }
        }
        let error = clean_uris(&json!(true), Lang::En).unwrap_err();
        assert_eq!((error.code.as_str(), error.message.as_str()), ("bad_args", "Invalid input."));
        assert_eq!(clean_uris(&json!(true), Lang::Nl).unwrap_err().message, "Ongeldige invoer.");
        let _ = fs::remove_dir_all(config);
    }
}
