//! Rust-owned local file operations. Unported methods keep the original compatibility contract.
use crate::{
    backend::Backend,
    events::{self, Events},
    i18n::Lang,
};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    ffi::{CStr, CString},
    fs, io,
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) mod actions;
mod archives;
mod clipboard;
mod control;
mod dry;
mod mutate;
mod places;
mod properties;
mod read;
mod search;
mod trash;
mod watch;

// D-G74: WinPoint's updater unpacks its release bundle with the Explorer's extractor (services/winpoint/job.rs)
pub(crate) use archives::{ReleaseLimits, extract_release};

/// The error of an API call (contracts C2): `code` for programs, `message` rendered in the caller's language,
/// and for catalogue texts also `key` + `params`, so the UI can match on them (never on `message`).
#[derive(Debug, Clone)]
pub(super) struct ApiError {
    pub code: String,
    pub message: String,
    pub data: Option<Value>,
    /// Catalogue key of `message` (`stem.name`), set by [`ApiError::tr`] / [`ApiError::trn`].
    pub key: Option<String>,
    /// The named placeholders `message` was filled with (an object of strings or numbers).
    pub params: Option<Value>,
}
pub(super) type ApiResult = Result<Value, ApiError>;
impl ApiError {
    pub fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            data: None,
            key: None,
            params: None,
        }
    }
    /// A catalogue message: `ApiError::tr("not_found", lang, "files.gone", &json!({"name": name}))`
    /// (`crate::i18n::NO_PARAMS` for none). `lang` is a `Lang` or "nl"/"en" (`ctx.lang()`, `ctx.language()`).
    pub fn tr(code: &str, lang: impl crate::i18n::AsLang, key: &str, params: &Value) -> Self {
        Self {
            code: code.into(),
            message: crate::i18n::trf(lang, key, params),
            data: None,
            key: Some(key.into()),
            params: Some(if params.is_object() {
                params.clone()
            } else {
                json!({})
            }),
        }
    }
    /// A plural catalogue message (`key.one` / `key.other` / `key.zero`); `params` gets `n` unless it has one.
    #[allow(dead_code)] // the frozen round-2 API (round2-gate.md)
    pub fn trn(
        code: &str,
        lang: impl crate::i18n::AsLang,
        key: &str,
        n: impl crate::i18n::Count,
        params: &Value,
    ) -> Self {
        let params = crate::i18n::count_params(&n, params);
        Self {
            code: code.into(),
            message: crate::i18n::trn(lang, key, n, &params),
            data: None,
            key: Some(key.into()),
            params: Some(params),
        }
    }
    /// `{"code", "message", "key"?, "params"?, "data"?}`: key and params only for catalogue messages.
    pub fn json(&self) -> Value {
        let mut value = json!({"code":self.code,"message":self.message});
        if let Some(key) = &self.key {
            value["key"] = json!(key);
            value["params"] = self.params.clone().unwrap_or_else(|| json!({}));
        }
        if let Some(data) = &self.data {
            value["data"] = data.clone();
        }
        value
    }
}

pub(super) struct Context {
    pub home: PathBuf,
    pub desktop: PathBuf,
    /// The settings language of this call, "nl" or "en" (C1); [`Context::lang`] is the same as a `Lang`.
    pub language: String,
    /// `explorer.showHidden`, read from `settings_dir` only by the first call that needs it (`files.list` without
    /// its own `showHidden`), so building a context costs one `stat` (round2-gate §2). Holds no text.
    show_hidden: std::sync::OnceLock<bool>,
    settings_dir: Option<PathBuf>,
    events: Option<Arc<Events>>,
    transfers: Arc<control::Registry>,
}
impl Context {
    pub fn new(home: PathBuf, desktop: PathBuf, language: String, show_hidden: bool) -> Self {
        Self {
            home,
            desktop,
            language,
            show_hidden: std::sync::OnceLock::from(show_hidden),
            settings_dir: None,
            events: None,
            transfers: Arc::new(control::Registry::default()),
        }
    }
    /// The language of this call's texts (C1), for `crate::i18n::tr(ctx.lang(), …)` and `ApiError::tr`.
    pub fn lang(&self) -> Lang {
        Lang::parse(&self.language)
    }
    /// Whether hidden items are listed when a call does not say (`explorer.showHidden`).
    pub fn show_hidden(&self) -> bool {
        *self.show_hidden.get_or_init(|| {
            self.settings_dir
                .as_ref()
                .and_then(|dir| fs::read(dir.join("settings.json")).ok())
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|settings| show_hidden_of(&settings))
        })
    }
    pub fn emit(&self, name: &str, data: Value) {
        if let Some(events) = &self.events {
            events.emit(name, data);
        }
    }
    pub fn changed(&self, path: &Path) {
        self.emit("files.dirChanged", json!({"path":path}));
        if path == self.desktop {
            self.emit("files.desktopChanged", json!({}));
        }
    }
    pub fn item(&self, path: &Path) -> ApiResult {
        read::item(path, self)
    }
    pub fn observe_clipboard(&self, revision: u64) {
        if let Some(events) = &self.events {
            events.observe_clipboard(revision);
        }
    }
}

pub(crate) struct Files {
    pub(crate) events: Arc<Events>,
    clipboard: clipboard::Clipboard,
    watcher: Option<watch::Watcher>,
    config: PathBuf,
    external: Option<Backend>,
    dry_run: bool,
    jobs: Mutex<(bool, usize)>,
    idle: Condvar,
    /// [`Files::busy`] found a call in flight: the end of the last one emits [`IDLE_EVENT`] (once)
    idle_wanted: AtomicBool,
    transfers: Arc<control::Registry>,
}
/// The local event (this process only, never sent to other windows) that ends a [`Files::busy`] wait: the last
/// `files.*` call in flight has ended (D-G45, GX1 m5: a restarting Explorer waits for its copy).
pub(crate) const IDLE_EVENT: &str = "files.idle";
impl Files {
    pub(crate) fn new(backend: &Backend) -> Result<Arc<Self>, String> {
        let runtime = glib::user_runtime_dir().join("win11-gnome");
        events::private_dir(&runtime).map_err(|error| error.to_string())?;
        let socket_root = socket_root(&runtime);
        let events = Events::new(&socket_root)
            .map_err(|error| format!("Bestandsgebeurtenissen starten: {error}"))?;
        let clipboard =
            clipboard::Clipboard::new(runtime.join("files")).map_err(|error| error.message)?;
        let snapshot = clipboard.clone();
        events.set_clipboard_loader(Arc::new(move || snapshot.event_snapshot()));
        let external = std::env::var_os("WIN11_REFERENCE_INSTANCE").map(|_| backend.clone());
        // An explicitly attached dry-run backend must never become writable through migration.
        let dry_run = crate::backend::dry_run()
            || external.as_ref().is_some_and(|backend| {
                backend_call(backend, "shell.info")
                    .map(|value| value["dryRun"] == true)
                    .unwrap_or(true)
            });
        let config = std::env::var_os("WIN11_GNOME_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| glib::user_config_dir().join("win11-gnome"));
        let watcher = if external.is_none() {
            Some(watch::Watcher::new(
                events.clone(),
                desktop_dir(&glib::home_dir()),
            )?)
        } else {
            None
        };
        Ok(Arc::new(Self {
            events,
            watcher,
            clipboard,
            config,
            external,
            dry_run,
            jobs: Mutex::new((false, 0)),
            idle: Condvar::new(),
            idle_wanted: AtomicBool::new(false),
            transfers: Arc::new(control::Registry::default()),
        }))
    }

    /// Is a `files.*` call of this process in flight (a copy, a move, a paste, an extraction …)? When it is, the
    /// end of the last one emits the local [`IDLE_EVENT`] once (D-G45: the `shell.restartAll` receiver defers
    /// Explorer's exec until then). Checked and armed under the job lock, so that end cannot be missed.
    pub(crate) fn busy(&self) -> bool {
        let jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        let busy = jobs.1 > 0;
        if busy {
            self.idle_wanted.store(true, Ordering::SeqCst);
        }
        busy
    }

    /// Tests elsewhere in the crate (lib.rs's restart receiver): a file service on its own temporary root.
    #[cfg(test)]
    pub(crate) fn for_tests(root: &Path) -> Arc<Self> {
        Arc::new(Self {
            events: Events::new(&root.join("events")).unwrap(),
            clipboard: clipboard::Clipboard::new(root.join("clipboard")).unwrap(),
            watcher: None,
            config: root.join("config"),
            external: None,
            dry_run: false,
            jobs: Mutex::new((false, 0)),
            idle: Condvar::new(),
            idle_wanted: AtomicBool::new(false),
            transfers: Arc::new(control::Registry::default()),
        })
    }
    /// Tests: one `files.*` call in flight until the guard is dropped (a copy that is still running).
    #[cfg(test)]
    pub(crate) fn job_for_tests(&self) -> impl Drop + '_ {
        self.jobs
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1 += 1;
        Job(self)
    }

    pub(crate) fn context(&self) -> Context {
        let home = glib::home_dir();
        let desktop = desktop_dir(&home);
        let mut context = if let Some(settings) = self.external_settings() {
            let language = context_language(&settings);
            Context::new(home, desktop, language, show_hidden_of(&settings))
        } else {
            // C1 and round2-gate §2: one stat of settings.json per call; the file is parsed only after it
            // changed (i18n::language_in), and showHidden only when a call needs it (Context::show_hidden).
            let language = crate::i18n::language_in(&self.config).code().to_owned();
            let mut context = Context::new(home, desktop, language, false);
            context.show_hidden = std::sync::OnceLock::new();
            context.settings_dir = Some(self.config.clone());
            context
        };
        context.events = Some(self.events.clone());
        context.transfers = self.transfers.clone();
        context
    }

    /// The settings of an attached reference instance (WIN11_REFERENCE_INSTANCE, tests only), which owns them.
    fn external_settings(&self) -> Option<Value> {
        self.external
            .as_ref()
            .and_then(|backend| backend_call(backend, "settings.get"))
    }

    /// The language of a call that fails before its [`Context`] exists (C1).
    fn lang(&self) -> Lang {
        match self.external_settings() {
            Some(settings) => Lang::of_settings(&settings),
            None => crate::i18n::language_in(&self.config),
        }
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !matches!(
            method,
            "files.list"
                | "files.stat"
                | "files.desktop"
                | "files.createFolder"
                | "files.createFile"
                | "files.rename"
                | "files.copy"
                | "files.move"
                | "files.transferControl"
                | "files.resolveConflict"
                | "files.clipboardSet"
                | "files.clipboardGet"
                | "files.clipboardClear"
                | "files.paste"
                | "files.trash"
                | "files.trashInfo"
                | "files.emptyTrash"
                | "files.restoreTrash"
                | "files.properties"
                | "files.setAttributes"
                | "files.compress"
                | "files.extract"
                | "files.places"
                | "files.find"
                | "files.watch"
                | "files.unwatch"
                | "files.open"
                | "files.openWith"
                | "files.openTerminal"
                | "files.trustLauncher"
        ) {
            return None;
        }
        if matches!(method, "files.watch" | "files.unwatch") && self.watcher.is_none() {
            return None;
        }
        let _job = {
            let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
            if jobs.0 {
                // render outside the lock: with an attached reference backend, lang() is a settings.get call
                drop(jobs);
                return Some(Err(ApiError::tr(
                    "offline",
                    self.lang(),
                    "files.shuttingDown",
                    crate::i18n::NO_PARAMS,
                )));
            }
            jobs.1 += 1;
            Job(self)
        };
        let empty = json!({});
        let args = if args.is_null() { &empty } else { args };
        if !args.is_object() {
            return Some(Err(ApiError::tr(
                "bad_args",
                self.lang(),
                "files.badArgs",
                crate::i18n::NO_PARAMS,
            )));
        }
        if self.dry_run
            && self.external.is_some()
            && matches!(
                method,
                "files.rename"
                    | "files.copy"
                    | "files.move"
                    | "files.paste"
                    | "files.trash"
                    | "files.emptyTrash"
                    | "files.restoreTrash"
                    | "files.setAttributes"
                    | "files.compress"
                    | "files.extract"
                    | "files.open"
                    | "files.openWith"
                    | "files.openTerminal"
                    | "files.trustLauncher"
            )
        {
            // Keep the reference validation and dry results of an attached compatibility server.
            return None;
        }
        let context = self.context();
        if method == "files.transferControl" {
            return Some(self.transfers.call(args, &context, self.dry_run));
        }
        if method == "files.resolveConflict" {
            return Some(self.transfers.resolve(args, &context, self.dry_run));
        }
        if self.dry_run {
            if let Some(result) = dry::call(method, args, &context) {
                return Some(result);
            }
        }
        if let Some(watcher) = &self.watcher
            && let Some(result) = watcher.call(method, args, &context)
        {
            return Some(result);
        }
        if let Some(result) = search::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = actions::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = places::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = trash::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = properties::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = archives::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = read::call(method, args, &context) {
            return Some(result);
        }
        if let Some(result) = mutate::call(method, args, &context) {
            return Some(result);
        }
        self.clipboard.call(method, args, &context)
    }

    pub(crate) fn native_watch(&self) -> bool {
        self.watcher.is_some()
    }

    pub(crate) fn shutdown(&self) {
        // A paused transfer must wake before we wait for workers to leave; cancellation is cooperative.
        self.transfers.shutdown();
        // Closing a window must not terminate a worker halfway through copying a file.
        let mut jobs = self.jobs.lock().unwrap_or_else(|error| error.into_inner());
        jobs.0 = true;
        while jobs.1 > 0 {
            jobs = self
                .idle
                .wait(jobs)
                .unwrap_or_else(|error| error.into_inner());
        }
        drop(jobs);
        if let Some(watcher) = &self.watcher {
            watcher.shutdown();
        }
        self.events.shutdown();
    }
}

/// `explorer.showHidden` of a settings document.
fn show_hidden_of(settings: &Value) -> bool {
    settings["explorer"]["showHidden"]
        .as_bool()
        .unwrap_or(false)
}

fn desktop_dir(home: &Path) -> PathBuf {
    std::env::var_os("WIN11SHELL_DESKTOP_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            glib::user_special_dir(glib::UserDirectory::Desktop).filter(|path| path != home)
        })
        .unwrap_or_else(|| home.join("Desktop"))
}

pub(crate) fn filename_mime(name: &str) -> String {
    read::filename_mime(name)
}
pub(crate) fn icon_exists(name: &str) -> bool {
    let path = if name.starts_with('/') {
        Some(PathBuf::from(name))
    } else if name.starts_with("file://") {
        url::Url::parse(name)
            .ok()
            .map(|uri| PathBuf::from(percent_decode(uri.path())))
    } else {
        None
    };
    let Some(path) = path else {
        return read::icon_catalog().contains(name);
    };
    if !path.is_file() {
        return false;
    }
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(mut file) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&path)
    else {
        return false;
    };
    if !file.metadata().is_ok_and(|m| m.is_file()) {
        return false;
    }
    let mut head = [0; 4096];
    let Ok(n) = file.read(&mut head) else {
        return false;
    };
    let head = &head[..n];
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    head.starts_with(b"\x89PNG\r\n\x1a\n")
        || head.starts_with(b"\xff\xd8\xff")
        || head.starts_with(b"GIF87a")
        || head.starts_with(b"GIF89a")
        || (head.starts_with(b"RIFF") && head.get(8..12) == Some(b"WEBP"))
        || (head.starts_with(b"BM") && ext == "bmp")
        || (head.starts_with(b"\x00\x00\x01\x00") && matches!(ext.as_str(), "ico" | "cur" | ""))
        || (head.get(4..8) == Some(b"ftyp")
            && head.get(8..12).is_some_and(|brand| {
                [
                    b"avif", b"avis", b"heic", b"heix", b"mif1", b"msf1", b"hevc",
                ]
                .iter()
                .any(|b| brand == b.as_slice())
            }))
        || head.starts_with(b"II*\x00")
        || head.starts_with(b"MM\x00*")
        || head.starts_with(b"\xff\x0a")
        || head.starts_with(b"\x00\x00\x00\x0cJXL ")
        || (head.starts_with(b"\x1f\x8b") && ext == "svgz")
        || head[..head.len().min(64)]
            .windows(9)
            .any(|h| h == b"/* XPM */")
        || {
            let h = head
                .iter()
                .copied()
                .skip_while(|b| b"\xef\xbb\xbf \t\r\n".contains(b))
                .map(|b| b.to_ascii_lowercase())
                .collect::<Vec<_>>();
            h.starts_with(b"<") && h.windows(4).any(|h| h == b"<svg")
        }
        || (ext == "tga" && head.len() > 18)
}
pub(crate) fn first_icon(names: &[&str], fallback: &str) -> String {
    names
        .iter()
        .find(|name| icon_exists(name))
        .copied()
        .unwrap_or(fallback)
        .to_owned()
}

struct Job<'a>(&'a Files);
impl Drop for Job<'_> {
    fn drop(&mut self) {
        let idle = {
            let mut jobs = self
                .0
                .jobs
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            jobs.1 -= 1;
            self.0.idle.notify_all();
            jobs.1 == 0 && self.0.idle_wanted.swap(false, Ordering::SeqCst)
        };
        if idle {
            // outside the job lock: an observer may start the next files.* call
            self.0.events.emit_local(IDLE_EVENT, json!({}));
        }
    }
}

fn backend_call(backend: &Backend, method: &str) -> Option<Value> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(2))
        .build();
    let value = agent
        .post(&format!(
            "http://127.0.0.1:{}/api/call/{method}",
            backend.port
        ))
        .set("X-Shell-Token", &backend.token)
        .send_json(json!({"args":{}}))
        .ok()?
        .into_json::<Value>()
        .ok()?;
    if value["ok"] == true {
        Some(value["result"].clone())
    } else {
        None
    }
}

/// The `Context` language of a reference instance's settings: `"system"` is the session's language
/// ([`Lang::of`]), as everywhere else.
fn context_language(settings: &Value) -> String {
    Lang::of(&settings["language"]).code().to_owned()
}

fn socket_root(runtime: &Path) -> PathBuf {
    let usual = runtime.join("events");
    if usual.as_os_str().len() < 65 {
        return usual;
    }
    // sockaddr_un paths are limited to 107 bytes; isolated XDG roots can be much longer.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    runtime.hash(&mut hasher);
    // A custom TMPDIR can be just as long; the socket fallback must stay short.
    PathBuf::from("/tmp").join(format!(
        "win11-events-{}-{:016x}",
        unsafe { libc::geteuid() },
        hasher.finish()
    ))
}

/// [`abs_in`] in the process's settings language; for callers outside files/ without a language at hand
/// (apps/mod.rs). Inside files/, call `abs_in(value, ctx.lang())`.
pub(super) fn abs(value: &Value) -> Result<PathBuf, ApiError> {
    abs_in(value, crate::i18n::current_language())
}

/// An absolute, lexically normalised path from an API argument (a path, `file://` URI or `~` path); its error
/// text is in `lang`.
pub(super) fn abs_in(value: &Value, lang: Lang) -> Result<PathBuf, ApiError> {
    let raw = value
        .as_str()
        .filter(|path| !path.is_empty() && !path.contains('\0') && path.chars().count() <= 4096)
        .ok_or_else(|| ApiError::tr("bad_args", lang, "files.badPath", crate::i18n::NO_PARAMS))?;
    let decoded = if let Some(path) = raw.strip_prefix("file://") {
        percent_decode(path)
    } else {
        raw.into()
    };
    if decoded.contains('\0') {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files.badPath",
            crate::i18n::NO_PARAMS,
        ));
    }
    let expanded = if let Some(rest) = decoded.strip_prefix('~') {
        let (user, tail) = rest.split_once('/').unwrap_or((rest, ""));
        let home = if user.is_empty() {
            Some(glib::home_dir())
        } else {
            user_home(user)
        };
        if let Some(home) = home {
            if tail.is_empty() {
                home
            } else {
                home.join(tail)
            }
        } else {
            PathBuf::from(&decoded)
        }
    } else {
        PathBuf::from(&decoded)
    };
    if !expanded.is_absolute() {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files.badPath",
            crate::i18n::NO_PARAMS,
        ));
    }
    let mut normalized = PathBuf::from("/");
    for component in expanded.components() {
        match component {
            Component::Normal(name) => normalized.push(name),
            Component::ParentDir => {
                normalized.pop();
            }
            _ => {}
        }
    }
    Ok(normalized)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut result = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%'
            && at + 2 < bytes.len()
            && let (Some(a), Some(b)) = (
                (bytes[at + 1] as char).to_digit(16),
                (bytes[at + 2] as char).to_digit(16),
            )
        {
            result.push((a * 16 + b) as u8);
            at += 3;
        } else {
            result.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8_lossy(&result).into_owned()
}

fn user_home(user: &str) -> Option<PathBuf> {
    let user = CString::new(user).ok()?;
    let mut buffer = vec![0_u8; 16 * 1024];
    let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
    let mut found = std::ptr::null_mut();
    let result = unsafe {
        libc::getpwnam_r(
            user.as_ptr(),
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut found,
        )
    };
    if result != 0 || found.is_null() {
        return None;
    }
    let entry = unsafe { entry.assume_init() };
    Some(PathBuf::from(
        unsafe { CStr::from_ptr(entry.pw_dir) }
            .to_string_lossy()
            .as_ref(),
    ))
}

pub(super) fn paths(value: &Value, lang: Lang) -> Result<Vec<PathBuf>, ApiError> {
    let values = if value.is_string() {
        vec![value]
    } else if let Some(values) = value.as_array() {
        values.iter().collect()
    } else {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files.noItems",
            crate::i18n::NO_PARAMS,
        ));
    };
    if values.is_empty() || values.len() > 10_000 {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "files.noItems",
            crate::i18n::NO_PARAMS,
        ));
    }
    let mut seen = HashSet::new();
    let mut output = Vec::with_capacity(values.len());
    for value in values {
        let path = abs_in(value, lang)?;
        if seen.insert(path.clone()) {
            output.push(path);
        }
    }
    Ok(output)
}

pub(super) fn directory(value: &Value, lang: Lang) -> Result<PathBuf, ApiError> {
    let path = abs_in(value, lang)?;
    if !path.is_dir() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "files.folderGone",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(path)
}

pub(super) fn validate_name(value: &Value, lang: Lang) -> Result<String, ApiError> {
    let name = value
        .as_str()
        .ok_or_else(|| ApiError::tr("bad_name", lang, "files.noName", crate::i18n::NO_PARAMS))?
        .trim();
    if name.is_empty() {
        return Err(ApiError::tr(
            "bad_name",
            lang,
            "files.noName",
            crate::i18n::NO_PARAMS,
        ));
    }
    if name.contains('/') || name.contains('\0') {
        return Err(ApiError::tr(
            "bad_name",
            lang,
            "files.slash",
            crate::i18n::NO_PARAMS,
        ));
    }
    if matches!(name, "." | "..") {
        return Err(ApiError::tr(
            "bad_name",
            lang,
            "files.reservedName",
            crate::i18n::NO_PARAMS,
        ));
    }
    if name.chars().any(|character| character < ' ') {
        return Err(ApiError::tr(
            "bad_name",
            lang,
            "files.badChars",
            crate::i18n::NO_PARAMS,
        ));
    }
    if name.len() > 255 {
        return Err(ApiError::tr(
            "name_too_long",
            lang,
            "files.nameTooLong",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(name.into())
}

/// The error of a failed file operation on `path`, in `lang`. `{item}` is the quoted name (files.quotedName) or,
/// for a nameless path, files.thisItem: one row per piece keeps the 3b898e6 Dutch byte for byte.
pub(super) fn io_error(error: io::Error, path: &Path, lang: Lang) -> ApiError {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    let item = if name.is_empty() {
        crate::i18n::tr(lang, "files.thisItem")
    } else {
        crate::i18n::trf(
            lang,
            "files.quotedName",
            &serde_json::json!({"name": name.to_string()}),
        )
    };
    let mut result = match error.raw_os_error() {
        Some(libc::EACCES) | Some(libc::EPERM) => ApiError::tr(
            "no_access",
            lang,
            "files.noAccessTo",
            &serde_json::json!({"item": item.to_string()}),
        ),
        Some(libc::ENOENT) => ApiError::tr(
            "not_found",
            lang,
            "files.notFoundItem",
            &serde_json::json!({"item": item.to_string()}),
        ),
        Some(libc::ENOSPC) => {
            ApiError::tr("no_space", lang, "files.noSpace", crate::i18n::NO_PARAMS)
        }
        Some(libc::EROFS) => {
            ApiError::tr("read_only", lang, "files.readOnly", crate::i18n::NO_PARAMS)
        }
        Some(libc::EEXIST) => ApiError::tr("exists", lang, "files.exists", crate::i18n::NO_PARAMS),
        Some(libc::ENAMETOOLONG) => ApiError::tr(
            "name_too_long",
            lang,
            "files.nameTooLong",
            crate::i18n::NO_PARAMS,
        ),
        Some(libc::EBUSY) => ApiError::tr(
            "busy",
            lang,
            "files.inUse",
            &serde_json::json!({"item": item.to_string()}),
        ),
        Some(libc::ENOTDIR) => {
            ApiError::tr("not_dir", lang, "files.notDir", crate::i18n::NO_PARAMS)
        }
        Some(libc::EINVAL) => ApiError::tr(
            "bad_name",
            lang,
            "files.nameNotOnDrive",
            crate::i18n::NO_PARAMS,
        ),
        _ => ApiError::tr(
            "failed",
            lang,
            "files.failedFor",
            &serde_json::json!({"item": item.to_string()}),
        ),
    };
    // "{item} …" starts a sentence: files.thisItem is lower case mid-sentence ("this item"); nl keeps 3b898e6's
    // bytes ("dit item bestaat niet (meer).").
    if lang == Lang::En {
        result.message = upper_first(&result.message);
    }
    if result.code == "failed" {
        result.data = Some(json!(error.to_string()));
    }
    result
}

/// `text` with its first character in upper case (English sentence starts).
fn upper_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Validated trash roots shared by the storage accounting service.
pub(crate) fn storage_trash_dirs() -> Vec<PathBuf> {
    trash::watch_dirs()
        .into_iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_reference_instances_language_system_is_the_sessions_language() {
        assert_eq!(context_language(&json!({"language": "en"})), "en");
        assert_eq!(context_language(&json!({"language": "nl"})), "nl");
        assert_eq!(
            context_language(&json!({})),
            "nl",
            "C1: anything else is Dutch"
        );
        // "system" was Dutch here before: it is the session's messages language, as Lang::system reads it
        assert_eq!(
            context_language(&json!({"language": "system"})),
            crate::i18n::system_code(|k| std::env::var(k).ok())
        );
        let session = |lang: &'static str| {
            crate::i18n::system_code(move |k| (k == "LANG").then(|| lang.to_owned()))
        };
        assert_eq!(session("en_GB.UTF-8"), "en");
        assert_eq!(session("nl_NL.UTF-8"), "nl");
    }
    #[test]
    fn absolute_icons_require_image_content_and_do_not_block_on_special_files() {
        let root =
            std::env::temp_dir().join(format!("win11-icon-content-{}", crate::events::nonce()));
        fs::create_dir(&root).unwrap();
        let text = root.join("not-image.png");
        fs::write(&text, "ordinary text").unwrap();
        assert!(!icon_exists(text.to_str().unwrap()));
        let png = root.join("image.bin");
        fs::write(&png, b"\x89PNG\r\n\x1a\n").unwrap();
        assert!(icon_exists(png.to_str().unwrap()));
        assert!(icon_exists(
            url::Url::from_file_path(&png).unwrap().as_str()
        ));
        let fifo = root.join("pipe");
        let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        unsafe {
            libc::mkfifo(c.as_ptr(), 0o600);
        }
        assert!(!icon_exists(fifo.to_str().unwrap()));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn long_runtime_uses_a_stable_short_socket_directory() {
        let runtime =
            PathBuf::from("/a/very/long/private/runtime/path/".to_owned() + &"é".repeat(100));
        let path = socket_root(&runtime);
        assert!(path.as_os_str().len() < 65);
        assert_eq!(socket_root(&runtime), path);
        assert_ne!(socket_root(&runtime.join("other")), path);
        assert_eq!(
            socket_root(Path::new("/tmp/short")),
            PathBuf::from("/tmp/short/events")
        );
    }
    #[test]
    fn busy_arms_one_local_files_idle_event_for_the_end_of_the_last_job() {
        // D-G45 (GX1 m5): Files::busy() and the local files.idle event the restart receiver waits for
        let root = std::env::temp_dir().join(format!("win11-files-idle-{}", events::nonce()));
        fs::create_dir(&root).unwrap();
        let service = Files::for_tests(&root);
        // a second host on the same event directory: a local event never reaches it
        let other = Events::new(&root.join("events")).unwrap();
        let count = |events: &Arc<Events>| {
            let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = seen.clone();
            events.on(
                IDLE_EVENT,
                Arc::new(move |_| {
                    counter.fetch_add(1, Ordering::SeqCst);
                }),
            );
            seen
        };
        let (here, there) = (count(&service.events), count(&other));
        let seen = || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            (here.load(Ordering::SeqCst), there.load(Ordering::SeqCst))
        };
        assert!(!service.busy(), "no call in flight");
        // without a busy() check the end of a call sends nothing (no event for every files.* call)
        drop(service.job_for_tests());
        assert_eq!(seen(), (0, 0));
        let copy = service.job_for_tests();
        let list = service.job_for_tests();
        assert!(service.busy());
        drop(list);
        assert_eq!(seen(), (0, 0), "one call is still in flight");
        assert!(service.busy());
        drop(copy);
        assert_eq!(seen(), (1, 0), "once, and only in this process");
        assert!(!service.busy());
        drop(service.job_for_tests());
        assert_eq!(seen(), (1, 0), "not armed again");
        other.shutdown();
        service.events.shutdown();
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn closing_a_host_waits_for_its_running_file_job_and_rejects_new_writes() {
        let root = std::env::temp_dir().join(format!("win11-file-shutdown-{}", events::nonce()));
        fs::create_dir(&root).unwrap();
        let service = Arc::new(Files {
            events: Events::new(&root.join("events")).unwrap(),
            clipboard: clipboard::Clipboard::new(root.join("clipboard")).unwrap(),
            watcher: None,
            config: root.clone(),
            external: None,
            dry_run: false,
            jobs: Mutex::new((false, 1)),
            idle: Condvar::new(),
            idle_wanted: AtomicBool::new(false),
            transfers: Arc::new(control::Registry::default()),
        });
        let job = Job(&service);
        let closing = service.clone();
        let (finished, done) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            closing.shutdown();
            finished.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !service.jobs.lock().unwrap().0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(done.try_recv().is_err());
        let result = service
            .call(
                "files.createFolder",
                &json!({"dir":root,"name":"forbidden"}),
            )
            .unwrap();
        assert_eq!(result.unwrap_err().code, "offline");
        assert!(!root.join("forbidden").exists());
        drop(job);
        done.recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        worker.join().unwrap();
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
    // round2-gate §6: the files stem rendered through the real call site (Files::call), in the language of the
    // service's own settings file, read per call (C1; extra 1 of R2-C: one stat instead of a full read).
    #[test]
    fn file_texts_follow_the_settings_language_per_call_through_files_call() {
        let root = std::env::temp_dir().join(format!("win11-files-i18n-{}", events::nonce()));
        fs::create_dir(&root).unwrap();
        let service = Files {
            events: Events::new(&root.join("events")).unwrap(),
            clipboard: clipboard::Clipboard::new(root.join("clipboard")).unwrap(),
            watcher: None,
            config: root.join("config"),
            external: None,
            dry_run: false,
            jobs: Mutex::new((false, 0)),
            idle: Condvar::new(),
            idle_wanted: AtomicBool::new(false),
            transfers: Arc::new(control::Registry::default()),
        };
        fs::create_dir(root.join("config")).unwrap();
        let settings = |value: Value| {
            // rename, as the settings store writes: a new inode, so the cached language is re-read
            let temp = root.join("config/settings.json.tmp");
            fs::write(&temp, value.to_string()).unwrap();
            fs::rename(&temp, root.join("config/settings.json")).unwrap();
        };
        let error = |method: &str, args: Value| service.call(method, &args).unwrap().unwrap_err();
        let missing = root.join("missing.txt");
        let file = root.join("file.txt");
        fs::write(&file, b"x").unwrap();

        settings(json!({"language":"en","explorer":{"showHidden":true}}));
        let context = service.context();
        assert_eq!(
            (context.language.as_str(), context.lang()),
            ("en", Lang::En)
        );
        assert!(
            context.show_hidden(),
            "explorer.showHidden is read on first use"
        );
        let gone = error("files.rename", json!({"path":missing,"newName":"b.txt"}));
        assert_eq!(
            (gone.code.as_str(), gone.message.as_str()),
            ("not_found", "This item no longer exists.")
        );
        let json = gone.json();
        assert_eq!(json["key"], "files.itemGone");
        assert_eq!(json["params"], json!({}));
        assert_eq!(
            error("files.rename", json!({"path":file,"newName":" "})).message,
            "Enter a name."
        );
        assert_eq!(
            error("files.list", json!({"path":file})).message,
            "This isn’t a folder."
        );
        assert_eq!(
            error("files.createFolder", json!({"dir":missing})).message,
            "This folder no longer exists."
        );
        assert_eq!(
            error("files.copy", json!({"paths":[],"dest":root})).message,
            "Select one or more items first."
        );
        // io_error(): the quoted name piece and its sentence, with key + params (C2)
        let gone = error("files.properties", json!({"path":missing}));
        assert_eq!(gone.message, "“missing.txt” no longer exists.");
        assert_eq!(gone.key.as_deref(), Some("files.notFoundItem"));
        assert_eq!(gone.params, Some(json!({"item":"“missing.txt”"})));
        // a nameless item: the lower-case piece starts an English sentence with a capital
        let nameless = io_error(
            io::Error::from_raw_os_error(libc::EBUSY),
            Path::new(""),
            Lang::En,
        );
        assert_eq!(nameless.message, "This item is in use.");
        assert_eq!(
            io_error(
                io::Error::from_raw_os_error(libc::EBUSY),
                Path::new(""),
                Lang::Nl
            )
            .message,
            "dit item is in gebruik."
        );
        let created = service
            .call("files.createFolder", &json!({"dir":root}))
            .unwrap()
            .unwrap();
        assert_eq!(created["path"], json!(root.join("New folder")));
        let places = service.call("files.places", &json!({})).unwrap().unwrap();
        let names: Vec<&str> = places
            .as_array()
            .unwrap()
            .iter()
            .map(|place| place["name"].as_str().unwrap())
            .collect();
        assert!(
            names.contains(&"Recycle Bin") && names.contains(&"System drive"),
            "{names:?}"
        );
        // Files::call's own early errors, in Files::lang(): bad_args for no object, offline while closing
        let invalid = error("files.rename", json!([]));
        assert_eq!(
            (invalid.code.as_str(), invalid.message.as_str()),
            ("bad_args", "Invalid input.")
        );
        assert_eq!(invalid.key.as_deref(), Some("files.badArgs"));
        service.jobs.lock().unwrap().0 = true;
        let offline = error("files.list", json!({"path":root}));
        service.jobs.lock().unwrap().0 = false;
        assert_eq!(
            (offline.code.as_str(), offline.message.as_str()),
            ("offline", "The file service is shutting down.")
        );
        assert_eq!(offline.key.as_deref(), Some("files.shuttingDown"));
        // multi_properties: the piece "All of type " + the type name (extra 3), or "Multiple types"
        let second = root.join("second.txt");
        fs::write(&second, b"y").unwrap();
        let properties = |paths: Value| {
            service
                .call("files.properties", &json!({ "paths": paths }))
                .unwrap()
                .unwrap()
        };
        assert_eq!(
            properties(json!([file, second]))["type"],
            "All of type Text Document"
        );
        assert_eq!(
            properties(json!([file, root.join("New folder")]))["type"],
            "Multiple types"
        );

        // the next call follows a switch to Dutch: 3b898e6's bytes
        settings(json!({"language":"nl"}));
        assert!(!service.context().show_hidden());
        let gone = error("files.rename", json!({"path":missing,"newName":"b.txt"}));
        assert_eq!(gone.message, "Dit item bestaat niet (meer).");
        assert_eq!(
            error("files.properties", json!({"path":missing})).message,
            "‘missing.txt’ bestaat niet (meer)."
        );
        let created = service
            .call("files.createFolder", &json!({"dir":root}))
            .unwrap()
            .unwrap();
        assert_eq!(created["path"], json!(root.join("Nieuwe map")));
        assert_eq!(
            error("files.rename", json!([])).message,
            "Ongeldige invoer."
        );
        assert_eq!(
            properties(json!([file, second]))["type"],
            "Alle van het type Tekstdocument"
        );
        assert_eq!(
            properties(json!([file, root.join("New folder")]))["type"],
            "Meerdere typen"
        );
        // anything but "en" is Dutch (C1), also a missing file
        settings(json!({"language":"EN"}));
        assert_eq!(service.context().language, "nl");
        fs::remove_file(root.join("config/settings.json")).unwrap();
        assert_eq!(
            error("files.rename", json!({"path":file,"newName":""})).message,
            "Geef een naam op."
        );
        // D-G1a: files.emptyTrash (trash.rs) has a singular now; other counts keep 3b898e6's "item(s)"
        let failed = |lang, n: u64| {
            crate::i18n::trn(lang, "files.emptyTrashFailed", n, crate::i18n::NO_PARAMS)
        };
        assert_eq!(failed(Lang::Nl, 1), "1 item kon niet worden verwijderd.");
        assert_eq!(
            failed(Lang::Nl, 2),
            "2 item(s) konden niet worden verwijderd."
        );
        assert_eq!(failed(Lang::En, 2), "2 items couldn’t be deleted.");
        drop(service);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn lexical_paths_uri_special_characters_and_linux_names_keep_their_meaning() {
        assert_eq!(
            abs(&json!("file:///tmp/a%3Fb%23c%2Bd/../x%20y")).unwrap(),
            PathBuf::from("/tmp/x y")
        );
        assert_eq!(
            abs(&json!("file:///tmp/a?b#c")).unwrap(),
            PathBuf::from("/tmp/a?b#c")
        );
        assert!(abs(&json!("file:///tmp/a%00b")).is_err());
        assert_eq!(abs(&json!("//a/../b/./c")).unwrap(), PathBuf::from("/b/c"));
        assert_eq!(
            validate_name(&json!(" a:?*.txt "), Lang::Nl).unwrap(),
            "a:?*.txt"
        );
        assert!(validate_name(&json!("../a"), Lang::Nl).is_err());
        assert_eq!(
            validate_name(&json!("é".repeat(128)), Lang::Nl)
                .unwrap_err()
                .code,
            "name_too_long"
        );
    }
}
