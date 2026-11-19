//! Native settings persistence, preserving the original store's validation and subtree semantics.
//! Root's controller owns event delivery, compatibility listeners and desktop side effects.
use crate::files::ApiError;
use crate::i18n::{Lang, NO_PARAMS};
use serde_json::{Map, Value, json};
use std::{
    ffi::{CStr, CString},
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, ApiError>;

#[derive(Clone, Debug)]
pub(crate) struct Change {
    pub settings: Value,
    pub path: Option<String>,
    pub changed: Vec<String>,
}

#[derive(Clone)]
struct ReadCache {
    raw: Option<Vec<u8>>,
    effective: Value,
}

pub(crate) struct SettingsStore {
    config: PathBuf,
    path: PathBuf,
    read_cache: Mutex<Option<ReadCache>>,
    observed: Mutex<Value>,
}

impl SettingsStore {
    pub(crate) fn new(config: PathBuf) -> Result<Self> {
        let lang = crate::i18n::language_in(&config);
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&config)
            .map_err(|error| save_error(lang, error))?;
        let metadata = fs::symlink_metadata(&config).map_err(|error| save_error(lang, error))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(save_error(
                lang,
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    crate::i18n::tr(lang, "settings.dirNotOwned"),
                ),
            ));
        }
        let store = Self {
            path: config.join("settings.json"),
            config,
            read_cache: Mutex::new(None),
            observed: Mutex::new(Value::Null),
        };
        let _lock = store.lock()?;
        let initial = store.read_current(false)?;
        *store
            .observed
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = initial;
        drop(_lock);
        Ok(store)
    }

    /// The language of this store's settings file (contracts C1): the language of its error messages.
    pub(crate) fn lang(&self) -> Lang {
        crate::i18n::language_in(&self.config)
    }

    /// Fresh disk data without advancing the notification watermark.
    pub(crate) fn snapshot(&self) -> Result<Value> {
        let _lock = self.lock()?;
        self.read_current(false)
    }

    /// Apply a desktop effect to the latest state while other hosts cannot commit
    /// a newer setting. This keeps an older notification from winning a write race.
    pub(crate) fn with_snapshot<T>(&self, apply: impl FnOnce(&Value) -> T) -> Result<T> {
        let _lock = self.lock()?;
        let snapshot = self.read_current(false)?;
        Ok(apply(&snapshot))
    }

    pub(crate) fn reload(&self) -> Result<Option<Change>> {
        let _lock = self.lock()?;
        let snapshot = self.read_current(false)?;
        Ok(self.observe(snapshot, None))
    }

    /// A cross-host notification: re-read the file even when its bytes are the ones this store cached. The file may
    /// hold a partial object (a native write that replaced a subtree, an older writer, a hand edit); like every
    /// load, the defaults are merged under it (`effective`), so a missing key reads as its default everywhere.
    pub(crate) fn reread_disk(&self) -> Result<Option<Change>> {
        let _lock = self.lock()?;
        let snapshot = self.read_current(true)?;
        Ok(self.observe(snapshot, None))
    }

    pub(crate) fn call(
        &self,
        method: &str,
        args: &Value,
    ) -> Option<Result<(Value, Option<Change>)>> {
        if !matches!(
            method,
            "settings.get"
                | "settings.set"
                | "settings.patch"
                | "settings.reset"
                | "settings.export"
                | "settings.import"
                | "settings.backupFile"
        ) {
            return None;
        }
        let lang = self.lang();
        let empty = json!({});
        let args = if args.is_null() { &empty } else { args };
        if !args.is_object() {
            return Some(Err(ApiError::tr(
                "bad_args",
                lang,
                "settings.badArgs",
                NO_PARAMS,
            )));
        }
        Some(match method {
            "settings.get"=>self.snapshot().map(|value|(value,None)),
            "settings.set"=>{
                if !args.as_object().unwrap().contains_key("path") {Err(ApiError::tr("bad_args", lang, "settings.noPath", NO_PARAMS))}
                else {self.set(&args["path"],args.get("value").cloned().unwrap_or(Value::Null))}
            },
            "settings.patch"=>{
                let patch=if truthy(&args["patch"]){args["patch"].clone()}else{json!({})};self.patch(patch)
            },
            "settings.reset"=>self.reset(&args["path"]),
            "settings.export"=>self.snapshot().and_then(|value|pretty(&value, lang).map(|text|(json!({"json":text,"name":crate::i18n::trf(lang, "settings.exportName", &json!({"date": stamp("%Y-%m-%d")}))}),None))),
            "settings.import"=>self.import(&args["json"]),
            "settings.backupFile"=>self.backup().map(|value|(value,None)),
            _=>unreachable!(),
        })
    }

    fn set(&self, path: &Value, value: Value) -> Result<(Value, Option<Change>)> {
        let lang = self.lang();
        let segments = split_path(path, lang)?;
        if segments == ["version"] {
            return Err(ApiError::tr(
                "read_only",
                lang,
                "settings.readOnly",
                NO_PARAMS,
            ));
        }
        let value = check_value(&segments, value, lang)?;
        // D-G24: the language is exactly "nl", "en" or "system" (amendment 18), also not an object under it; a value
        // of the wrong type was refused as bad_value just above. Reading stays C1 (anything else is nl).
        if segments[0] == "language" && (segments.len() > 1 || !valid_language(&value)) {
            return Err(with_data(
                ApiError::tr("invalid_value", lang, "settings.badValue", NO_PARAMS),
                json!({"path":segments.join(".")}),
            ));
        }
        let label = Some(segments.join("."));
        self.apply(label, move |data| {
            set_path(data, &segments, value);
            Ok(())
        })
    }

    fn patch(&self, patch: Value) -> Result<(Value, Option<Change>)> {
        let lang = self.lang();
        if !patch.is_object() {
            return Err(ApiError::tr(
                "bad_value",
                lang,
                "settings.badPatch",
                NO_PARAMS,
            ));
        }
        let mut dropped = Vec::new();
        let mut clean = sanitize(&patch, defaults(), "", &mut dropped);
        if !dropped.is_empty() {
            return Err(with_data(
                ApiError::tr("bad_value", lang, "settings.badValue", NO_PARAMS),
                json!({"paths":dropped}),
            ));
        }
        // D-G24, as in set()
        if clean
            .get("language")
            .is_some_and(|language| !valid_language(language))
        {
            return Err(with_data(
                ApiError::tr("invalid_value", lang, "settings.badValue", NO_PARAMS),
                json!({"paths":["language"]}),
            ));
        }
        clean.as_object_mut().unwrap().shift_remove("version");
        self.apply(None, move |data| {
            *data = deep_merge(data, &clean);
            Ok(())
        })
    }

    fn reset(&self, path: &Value) -> Result<(Value, Option<Change>)> {
        if path.is_null() || path.as_str() == Some("") {
            return self.apply(None, |data| {
                *data = defaults().clone();
                Ok(())
            });
        }
        let segments = split_path(path, self.lang())?;
        let replacement = get_path(defaults(), &segments).cloned();
        let label = Some(segments.join("."));
        self.apply(label, move |data| {
            if let Some(value) = replacement {
                set_path(data, &segments, value);
            } else {
                delete_path(data, &segments);
            }
            Ok(())
        })
    }

    fn import(&self, text: &Value) -> Result<(Value, Option<Change>)> {
        let lang = self.lang();
        let data = if text.is_object() {
            text.clone()
        } else {
            let text = text
                .as_str()
                .filter(|text| text.chars().count() <= 4 * 1024 * 1024)
                .ok_or_else(|| invalid_settings(lang))?;
            serde_json::from_str::<Value>(text).map_err(|_| invalid_settings(lang))?
        };
        let Some(object) = data.as_object() else {
            return Err(invalid_settings(lang));
        };
        if !object
            .keys()
            .any(|key| key != "version" && defaults().get(key).is_some())
        {
            return Err(invalid_settings(lang));
        }
        let mut dropped = Vec::new();
        let mut clean = sanitize(&data, defaults(), "", &mut dropped);
        clean["version"] = defaults()["version"].clone();
        // D-G31: an import drops a language that settings.set would refuse (D-G24) and keeps the current one;
        // D-G49 (WU-FIX): a file without a language keeps it too; everything else of the file is imported as before
        let keep_language = object
            .get("language")
            .is_none_or(|language| !valid_language(language));
        if keep_language {
            clean.as_object_mut().unwrap().shift_remove("language");
        }
        let merged = deep_merge(defaults(), &clean);
        self.apply(None, move |data| {
            let current = data
                .get("language")
                .filter(|language| valid_language(language))
                .cloned();
            *data = merged;
            // the file's own language when it is one; else the current one (C1 reads anything else as nl, the
            // default kept here)
            if keep_language && let Some(current) = current {
                data["language"] = current;
            }
            Ok(())
        })
    }

    fn apply(
        &self,
        label: Option<String>,
        mutate: impl FnOnce(&mut Value) -> Result<()>,
    ) -> Result<(Value, Option<Change>)> {
        let _lock = self.lock()?;
        let old = self.read_current(false)?;
        let mut new = old.clone();
        mutate(&mut new)?;
        // `new` is written as it is; every reader (this one too) sees it with the defaults under it, as a load does
        let mut view = effective(&new);
        let actual = diff_paths(&old, &view);
        let path = if actual.is_empty() {
            view = old;
            None
        } else {
            self.write(&self.path, &new, true)?;
            label
        };
        let mut change = self.observe(view.clone(), path.clone());
        // A peer may have changed the disk since our observation watermark. If this
        // write restores our earlier value, observe() alone sees no change, although
        // the peer's OS effects still need to be replaced by this latest commit.
        if !actual.is_empty() {
            if let Some(change) = change.as_mut() {
                for changed in actual {
                    if !change.changed.contains(&changed) {
                        change.changed.push(changed);
                    }
                }
            } else {
                change = Some(Change {
                    settings: view.clone(),
                    path,
                    changed: actual,
                });
            }
        }
        Ok((view, change))
    }

    fn backup(&self) -> Result<Value> {
        let _lock = self.lock()?;
        let data = self.read_current(false)?;
        let date = stamp("%Y%m%d-%H%M%S");
        let mut target = self.config.join(format!("settings-backup-{date}.json"));
        let mut n = 2;
        while target.exists() {
            target = self.config.join(format!("settings-backup-{date}-{n}.json"));
            n += 1;
        }
        self.write(&target, &data, false)?;
        Ok(
            json!({"path":target,"name":target.file_name().unwrap().to_string_lossy(),"dir":self.config}),
        )
    }

    fn observe(&self, snapshot: Value, path: Option<String>) -> Option<Change> {
        let mut observed = self
            .observed
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let changed = diff_paths(&observed, &snapshot);
        if changed.is_empty() {
            return None;
        }
        *observed = snapshot.clone();
        Some(Change {
            settings: snapshot,
            path,
            changed,
        })
    }

    fn read_current(&self, reread: bool) -> Result<Value> {
        let mut cache = self
            .read_cache
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let raw = match fs::read(&self.path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(_) => {
                self.backup_broken();
                None
            }
        };
        if !reread {
            if let Some(cache) = cache.as_ref() {
                if cache.raw == raw {
                    return Ok(cache.effective.clone());
                }
            }
        }
        let parsed = raw
            .as_ref()
            .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok())
            .filter(Value::is_object);
        let (raw, view) = match (raw, parsed) {
            // every load and reload (start-up, the 150 ms poll, cross-host notifications) merges the defaults under
            // the file, so a partial file (an older writer, {"language":"en"} alone) loses no default. Nothing is
            // written back: the file keeps exactly what its writer saved.
            (Some(raw), Some(value)) => (Some(raw), effective(&value)),
            (Some(_), None) => {
                self.backup_broken();
                (None, defaults().clone())
            }
            (None, _) => (None, defaults().clone()),
        };
        *cache = Some(ReadCache {
            raw,
            effective: view.clone(),
        });
        Ok(view)
    }

    fn backup_broken(&self) {
        if self.path.exists() {
            let _ = fs::rename(
                &self.path,
                self.config
                    .join(format!("settings.broken-{}.json", stamp("%Y%m%d-%H%M%S"))),
            );
        }
    }

    fn write(&self, target: &Path, data: &Value, update_cache: bool) -> Result<()> {
        let lang = self.lang();
        let bytes = (pretty(data, lang)? + "\n").into_bytes();
        let temporary = self.config.join(format!(".settings-{}.tmp", nonce()));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&temporary)
                .map_err(|error| save_error(lang, error))?;
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|error| save_error(lang, error))?;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|error| save_error(lang, error))?;
            fs::rename(&temporary, target).map_err(|error| save_error(lang, error))?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        } else if update_cache {
            *self
                .read_cache
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(ReadCache {
                raw: Some(bytes),
                effective: effective(data),
            });
        }
        result
    }

    fn lock(&self) -> Result<Lock> {
        let lang = self.lang();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.config.join(".settings.lock"))
            .map_err(|error| save_error(lang, error))?;
        let metadata = file.metadata().map_err(|error| save_error(lang, error))?;
        if !metadata.is_file()
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
            || metadata.nlink() != 1
        {
            return Err(save_error(
                lang,
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    crate::i18n::tr(lang, "settings.lockNotPrivate"),
                ),
            ));
        }
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(Lock(file));
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(save_error(lang, error));
            }
        }
    }
}

struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

fn defaults() -> &'static Value {
    static DEFAULTS: OnceLock<Value> = OnceLock::new();
    DEFAULTS.get_or_init(|| {
        serde_json::from_str(include_str!("settings-defaults.json"))
            .expect("bundled reference settings defaults")
    })
}
/// What a settings document means: the file's values with known keys of the wrong type dropped, unknown keys kept,
/// and the defaults merged under it (objects key by key). Used for every load and for the result of every write.
fn effective(value: &Value) -> Value {
    let mut dropped = Vec::new();
    let clean = sanitize(value, defaults(), "", &mut dropped);
    deep_merge(defaults(), &clean)
}
fn save_error(lang: Lang, error: io::Error) -> ApiError {
    with_data(
        ApiError::tr("save_failed", lang, "settings.saveFailed", NO_PARAMS),
        json!(error.to_string()),
    )
}
fn invalid_settings(lang: Lang) -> ApiError {
    ApiError::tr("invalid_settings", lang, "settings.invalidFile", NO_PARAMS)
}
/// `error` with `data` (the path or paths of a bad value, the io error text of a failed save).
fn with_data(mut error: ApiError, data: Value) -> ApiError {
    error.data = Some(data);
    error
}
/// D-G24: `settings.language` may be set to exactly "nl" or "en" (contracts C1 for reading), or "system": the
/// session's own language (settings parity, amendment 18; `i18n::Lang::of` resolves it).
fn valid_language(value: &Value) -> bool {
    matches!(value.as_str(), Some("nl" | "en" | "system"))
}
fn pretty(value: &Value, lang: Lang) -> Result<String> {
    serde_json::to_string_pretty(value)
        .map_err(|_| ApiError::tr("bad_value", lang, "settings.notSavable", NO_PARAMS))
}
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Number(value) => value.as_f64() != Some(0.0),
        Value::String(value) => !value.is_empty(),
        Value::Array(value) => !value.is_empty(),
        Value::Object(value) => !value.is_empty(),
    }
}

/// Known string settings that take one of a few values: taskbar.multiDisplayApps is Windows 11's
/// "When using multiple displays, show my taskbar apps on" (MMTaskbarMode 0, 1, 2). A write of anything else is
/// refused as bad_value; a stored one reads as the default, like a value of the wrong type.
const CHOICES: &[(&str, &[&str])] = &[(
    "taskbar.multiDisplayApps",
    &["all", "mainAndWindow", "window"],
)];
fn choice_ok(path: &str, value: &Value) -> bool {
    CHOICES
        .iter()
        .find(|(known, _)| *known == path)
        .is_none_or(|(_, values)| value.as_str().is_some_and(|value| values.contains(&value)))
}
fn type_ok(default: &Value, value: &Value, path: &str) -> bool {
    match default {
        Value::Null => true,
        Value::Bool(_) => value.is_boolean(),
        Value::Number(_) => value.is_number(),
        Value::String(_) => {
            (value.is_string() && choice_ok(path, value))
                || (path == "alles.roots" && value.is_array())
        }
        Value::Array(_) => {
            value.is_array()
                && (!matches!(path, "taskbar.tray.shown" | "taskbar.tray.order")
                    || value.as_array().unwrap().iter().all(Value::is_string))
        }
        Value::Object(_) => value.is_object(),
    }
}
fn sanitize(value: &Value, default: &Value, prefix: &str, dropped: &mut Vec<String>) -> Value {
    if let (Some(values), Some(defaults)) = (value.as_object(), default.as_object()) {
        let mut output = Map::new();
        for (key, value) in values {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match defaults.get(key) {
                None => {
                    output.insert(key.clone(), value.clone());
                }
                Some(default) if !type_ok(default, value, &path) => dropped.push(path),
                Some(default) => {
                    output.insert(key.clone(), sanitize(value, default, &path, dropped));
                }
            }
        }
        Value::Object(output)
    } else {
        value.clone()
    }
}
fn check_value(segments: &[String], value: Value, lang: Lang) -> Result<Value> {
    let Some(default) = get_path(defaults(), segments) else {
        return Ok(value);
    };
    let path = segments.join(".");
    if !type_ok(default, &value, &path) {
        return Err(with_data(
            ApiError::tr("bad_value", lang, "settings.badValue", NO_PARAMS),
            json!({"path":path}),
        ));
    }
    if default.is_object() {
        let mut dropped = Vec::new();
        let clean = sanitize(&value, default, &path, &mut dropped);
        if !dropped.is_empty() {
            return Err(with_data(
                ApiError::tr("bad_value", lang, "settings.badValue", NO_PARAMS),
                json!({"paths":dropped}),
            ));
        }
        Ok(clean)
    } else {
        Ok(value)
    }
}

fn split_path(path: &Value, lang: Lang) -> Result<Vec<String>> {
    let segments = match path {
        Value::String(path) => path.split('.').map(str::to_owned).collect::<Vec<_>>(),
        Value::Array(path) => path.iter().map(py_string).collect(),
        _ => {
            return Err(ApiError::tr(
                "bad_path",
                lang,
                "settings.badPath",
                NO_PARAMS,
            ));
        }
    };
    if segments.is_empty()
        || segments.len() > 16
        || segments.iter().any(|segment| {
            segment.is_empty()
                || segment.chars().count() > 200
                || segment.chars().any(|character| character < ' ')
        })
    {
        return Err(with_data(
            ApiError::tr("bad_path", lang, "settings.badPath", NO_PARAMS),
            json!({"path":path}),
        ));
    }
    Ok(segments)
}
fn py_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => py_repr(value),
    }
}
fn py_repr(value: &Value) -> String {
    match value {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => {
            let quote = if value.contains('\'') && !value.contains('"') {
                '"'
            } else {
                '\''
            };
            let mut result = String::from(quote);
            for character in value.chars() {
                match character {
                    '\\' => result.push_str("\\\\"),
                    '\n' => result.push_str("\\n"),
                    '\r' => result.push_str("\\r"),
                    '\t' => result.push_str("\\t"),
                    c if c == quote => {
                        result.push('\\');
                        result.push(c);
                    }
                    c if c < ' ' || c == '\u{7f}' => {
                        result.push_str(&format!("\\x{:02x}", c as u32))
                    }
                    c => result.push(c),
                }
            }
            result.push(quote);
            result
        }
        Value::Array(values) => format!(
            "[{}]",
            values.iter().map(py_repr).collect::<Vec<_>>().join(", ")
        ),
        Value::Object(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| format!("{}: {}", py_repr(&json!(key)), py_repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
fn get_path<'a>(data: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut value = data;
    for segment in path {
        value = value.as_object()?.get(segment)?;
    }
    Some(value)
}
fn set_path(data: &mut Value, path: &[String], value: Value) {
    let mut current = data;
    for segment in &path[..path.len() - 1] {
        if !current[segment].is_object() {
            current[segment] = json!({});
        }
        current = &mut current[segment];
    }
    current[&path[path.len() - 1]] = value;
}
fn delete_path(data: &mut Value, path: &[String]) {
    let mut current = data;
    for segment in &path[..path.len() - 1] {
        let Some(next) = current.as_object_mut().and_then(|map| map.get_mut(segment)) else {
            return;
        };
        if !next.is_object() {
            return;
        }
        current = next;
    }
    if let Some(object) = current.as_object_mut() {
        object.shift_remove(&path[path.len() - 1]);
    }
}
fn deep_merge(base: &Value, overrides: &Value) -> Value {
    let mut output = base.as_object().cloned().unwrap_or_default();
    if let Some(values) = overrides.as_object() {
        for (key, value) in values {
            let value = if value.is_object() && output.get(key).is_some_and(Value::is_object) {
                deep_merge(&output[key], value)
            } else {
                value.clone()
            };
            output.insert(key.clone(), value);
        }
    }
    Value::Object(output)
}
fn py_equal(first: &Value, second: &Value) -> bool {
    match (first, second) {
        (Value::Object(first), Value::Object(second)) => {
            first.len() == second.len()
                && first
                    .iter()
                    .all(|(key, value)| second.get(key).is_some_and(|other| py_equal(value, other)))
        }
        (Value::Array(first), Value::Array(second)) => {
            first.len() == second.len()
                && first
                    .iter()
                    .zip(second)
                    .all(|(first, second)| py_equal(first, second))
        }
        (Value::Number(first), Value::Number(second)) => {
            if let (Some(first), Some(second)) = (first.as_i64(), second.as_i64()) {
                return first == second;
            }
            if let (Some(first), Some(second)) = (first.as_u64(), second.as_u64()) {
                return first == second;
            }
            if first.is_f64() && second.is_f64() {
                return first.as_f64() == second.as_f64();
            }
            let (integer, float) = if first.is_f64() {
                (second, first.as_f64().unwrap())
            } else {
                (first, second.as_f64().unwrap())
            };
            if !float.is_finite() || float.fract() != 0.0 {
                return false;
            }
            if let Some(integer) = integer.as_u64() {
                float >= 0.0 && float < 18_446_744_073_709_551_616.0 && integer == float as u64
            } else {
                integer.as_i64().is_some_and(|integer| {
                    float >= -9_223_372_036_854_775_808.0 && float < 0.0 && integer == float as i64
                })
            }
        }
        (Value::Bool(first), Value::Number(second))
        | (Value::Number(second), Value::Bool(first)) => {
            second.as_f64() == Some(if *first { 1.0 } else { 0.0 })
        }
        _ => first == second,
    }
}
fn diff_paths(first: &Value, second: &Value) -> Vec<String> {
    fn walk(first: Option<&Value>, second: Option<&Value>, prefix: &str, out: &mut Vec<String>) {
        if out.len() >= 200 {
            return;
        }
        if let (Some(Value::Object(first)), Some(Value::Object(second))) = (first, second) {
            for key in first
                .keys()
                .chain(second.keys().filter(|key| !first.contains_key(*key)))
            {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                walk(first.get(key), second.get(key), &path, out);
            }
        } else if !match (first, second) {
            (None, None) => true,
            (Some(first), Some(second)) => py_equal(first, second),
            _ => false,
        } {
            out.push(prefix.into());
        }
    }
    let mut result = Vec::new();
    walk(Some(first), Some(second), "", &mut result);
    result
}

fn stamp(format: &str) -> String {
    let format = CString::new(format).unwrap();
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let mut bytes = [0_u8; 64];
    unsafe {
        if libc::localtime_r(&now, tm.as_mut_ptr()).is_null() {
            return "19700101-000000".into();
        }
        libc::strftime(
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            format.as_ptr(),
            tm.as_ptr(),
        );
        CStr::from_ptr(bytes.as_ptr().cast())
            .to_string_lossy()
            .into_owned()
    }
}
fn nonce() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("win11-settings-{}", nonce()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn store(&self) -> SettingsStore {
            SettingsStore::new(self.0.clone()).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn call(store: &SettingsStore, method: &str, args: Value) -> (Value, Option<Change>) {
        store.call(method, &args).unwrap().unwrap()
    }

    #[test]
    fn defaults_loading_sanitizes_known_types_keeps_unknowns_and_does_not_write_valid_input() {
        let fixture = Fixture::new();
        let path = fixture.0.join("settings.json");
        let raw = "{\"language\":\"en\",\"theme\":{\"transparency\":\"bad\",\"custom\":7},\"future\":[1,true]}";
        fs::write(&path, raw).unwrap();
        let store = fixture.store();
        let data = store.snapshot().unwrap();
        assert_eq!(data["language"], "en");
        assert_eq!(data["theme"]["transparency"], true);
        assert_eq!(data["theme"]["custom"], 7);
        assert_eq!(data["future"], json!([1, true]));
        assert_eq!(fs::read_to_string(path).unwrap(), raw);
        assert!(store.reload().unwrap().is_none());
    }
    #[test]
    fn known_types_string_or_list_and_readonly_version_match_original_errors() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let error = store
            .call(
                "settings.set",
                &json!({"path":"theme.transparency","value":1}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "bad_value");
        assert_eq!(error.data, Some(json!({"path":"theme.transparency"})));
        assert_eq!(
            store
                .call("settings.set", &json!({"path":"version","value":2}))
                .unwrap()
                .unwrap_err()
                .code,
            "read_only"
        );
        assert_eq!(
            call(
                &store,
                "settings.set",
                json!({"path":"alles.roots","value":["/tmp"]})
            )
            .0["alles"]["roots"],
            json!(["/tmp"])
        );
        assert_eq!(
            store
                .call(
                    "settings.set",
                    &json!({"path":"taskbar.tray.order","value":[1]})
                )
                .unwrap()
                .unwrap_err()
                .data,
            Some(json!({"path":"taskbar.tray.order"}))
        );
        let error = store
            .call("settings.patch", &json!({"patch":{"theme":{"mode":false}}}))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.data, Some(json!({"paths":["theme.mode"]})));
    }
    #[test]
    fn replacing_a_subtree_writes_it_as_given_but_every_view_keeps_the_defaults() {
        // R2-A extra 1: the file keeps the partial subtree the writer saved; the writer, a peer, a restarted host
        // and a cross-host notification all read it with the defaults merged under it (3b898e6: only a restart did)
        let fixture = Fixture::new();
        let first = fixture.store();
        let second = fixture.store();
        let (value, change) = call(
            &first,
            "settings.set",
            json!({"path":"theme","value":{"mode":"light"}}),
        );
        let mut theme = defaults()["theme"].clone();
        theme["mode"] = json!("light");
        assert_eq!(value["theme"], theme);
        let change = change.unwrap();
        assert_eq!(change.path.as_deref(), Some("theme"));
        assert_eq!(change.changed, vec!["theme.mode"]);
        let file: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("settings.json")).unwrap()).unwrap();
        assert_eq!(file["theme"], json!({"mode":"light"}));
        assert_eq!(first.snapshot().unwrap()["theme"], theme);
        assert_eq!(second.snapshot().unwrap()["theme"], theme);
        assert_eq!(
            second.reread_disk().unwrap().unwrap().changed,
            vec!["theme.mode"]
        );
        let restarted = fixture.store();
        assert_eq!(restarted.snapshot().unwrap()["theme"], theme);
        assert!(restarted.reread_disk().unwrap().is_none());
        assert!(first.reread_disk().unwrap().is_none());
    }
    #[test]
    fn an_unknown_descendant_of_a_known_scalar_is_written_but_every_view_shows_the_default() {
        let fixture = Fixture::new();
        let first = fixture.store();
        let peer = fixture.store();
        call(
            &first,
            "settings.set",
            json!({"path":"theme.mode","value":"light"}),
        );
        // The source checks this exact, unknown path, then set_path replaces its scalar parent. The file holds
        // that shape; a known key of the wrong type reads as its default, as it always did at start-up.
        let (snapshot, change) = call(
            &first,
            "settings.set",
            json!({"path":"theme.mode.custom","value":"future"}),
        );
        assert_eq!(snapshot["theme"]["mode"], "dark");
        assert_eq!(change.unwrap().path.as_deref(), Some("theme.mode.custom"));
        let file: Value =
            serde_json::from_slice(&fs::read(fixture.0.join("settings.json")).unwrap()).unwrap();
        assert_eq!(file["theme"]["mode"], json!({"custom":"future"}));
        assert_eq!(peer.snapshot().unwrap()["theme"], snapshot["theme"]);
        assert!(peer.reread_disk().unwrap().is_none());
        let restarted = fixture.store();
        assert_eq!(restarted.snapshot().unwrap(), snapshot);
        assert!(first.reread_disk().unwrap().is_none());
    }
    #[test]
    fn a_partial_file_keeps_every_default_on_load_and_reload_and_is_not_written_back() {
        let fixture = Fixture::new();
        let path = fixture.0.join("settings.json");
        fs::write(&path, r#"{"language":"en"}"#).unwrap();
        let store = fixture.store();
        let peer = fixture.store();
        let mut expected = defaults().clone();
        expected["language"] = json!("en");
        assert_eq!(store.snapshot().unwrap(), expected);
        // an older writer (or a hand edit) saves a file without most keys: the 150 ms poll (reload) and a
        // cross-host notification (reread_disk) keep the defaults and report only the real changes
        let older = r#"{"language":"nl","theme":{"mode":"light"}}"#;
        fs::write(&path, older).unwrap();
        let change = store.reload().unwrap().unwrap();
        let mut changed = change.changed.clone();
        changed.sort();
        assert_eq!(changed, vec!["language", "theme.mode"]);
        expected["language"] = json!("nl");
        expected["theme"]["mode"] = json!("light");
        assert_eq!(change.settings, expected);
        assert!(store.reload().unwrap().is_none());
        assert!(store.reread_disk().unwrap().is_none());
        let adopted = peer.reread_disk().unwrap().unwrap();
        assert_eq!(adopted.settings, expected);
        let mut changed = adopted.changed.clone();
        changed.sort();
        assert_eq!(changed, vec!["language", "theme.mode"]);
        assert_eq!(fixture.store().snapshot().unwrap(), expected);
        // loading never writes the merged defaults back
        assert_eq!(fs::read_to_string(&path).unwrap(), older);
    }
    #[test]
    fn unknown_keys_survive_and_nested_objects_are_merged_on_every_reload() {
        let fixture = Fixture::new();
        let store = fixture.store();
        assert_eq!(store.snapshot().unwrap(), defaults().clone());
        let raw = r#"{"future":{"a":[1]},"theme":{"mode":"light","custom":7},"taskbar":{"tray":{"shown":["x"]}}}"#;
        fs::write(fixture.0.join("settings.json"), raw).unwrap();
        let data = store.reload().unwrap().unwrap().settings;
        assert_eq!(data["future"], json!({"a":[1]}));
        assert_eq!(data["theme"]["custom"], 7);
        assert_eq!(data["theme"]["mode"], "light");
        assert_eq!(data["theme"]["accent"], "#0067c0");
        assert_eq!(data["taskbar"]["alignment"], "center");
        assert_eq!(data["taskbar"]["tray"]["shown"], json!(["x"]));
        assert_eq!(data["taskbar"]["tray"]["hiddenMenu"], true);
        assert_eq!(data["taskbar"]["tray"]["known"], json!({}));
        assert_eq!(data["version"], 1);
        assert_eq!(store.reread_disk().unwrap().map(|c| c.settings), None);
        assert_eq!(store.snapshot().unwrap(), data);
        assert_eq!(
            fs::read_to_string(fixture.0.join("settings.json")).unwrap(),
            raw
        );
    }
    #[test]
    fn the_object_form_of_language_reads_as_dutch_on_every_load() {
        // contracts C1: the object form that settings.set of language.custom used to write means nl
        let fixture = Fixture::new();
        let path = fixture.0.join("settings.json");
        let raw = r#"{"language":{"custom":"future"},"theme":{"mode":"light"}}"#;
        fs::write(&path, raw).unwrap();
        let store = fixture.store();
        assert_eq!(store.snapshot().unwrap()["language"], "nl");
        assert_eq!(store.snapshot().unwrap()["theme"]["accent"], "#0067c0");
        assert_eq!(crate::i18n::language_in(&fixture.0), Lang::Nl);
        assert_eq!(store.lang(), Lang::Nl);
        let later = r#"{"language":{"custom":"en"}}"#;
        fs::write(&path, later).unwrap();
        let change = store.reload().unwrap().unwrap();
        assert_eq!(change.settings["language"], "nl");
        assert_eq!(change.changed, vec!["theme.mode"]);
        assert!(store.reread_disk().unwrap().is_none());
        assert_eq!(fs::read_to_string(&path).unwrap(), later);
    }
    #[test]
    fn the_language_is_set_to_exactly_nl_or_en_and_anything_else_is_refused() {
        // D-G24 (the mock's ENUMS): code invalid_value with the catalogue's bad-value text; wrong types keep
        // bad_value (the type check comes first, as before)
        let fixture = Fixture::new();
        let store = fixture.store();
        for value in ["EN", "de", "", " en", "nl-NL", "en_GB"] {
            let error = store
                .call("settings.set", &json!({"path":"language","value":value}))
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, "invalid_value", "{value:?}");
            assert_eq!(error.message, "Deze waarde past niet bij deze instelling.");
            assert_eq!(error.key.as_deref(), Some("settings.badValue"));
            assert_eq!(error.data, Some(json!({"path":"language"})));
        }
        let error = store
            .call(
                "settings.set",
                &json!({"path":"language.custom","value":"future"}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.data),
            ("invalid_value", Some(json!({"path":"language.custom"})))
        );
        let error = store
            .call("settings.set", &json!({"path":"language","value":1}))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "bad_value");
        // the object form as a value is a wrong type too (the default is a string), so it stays bad_value; only
        // a dotted path below language (language.custom, above) reaches the D-G24 check (review n1)
        let error = store
            .call(
                "settings.set",
                &json!({"path":"language","value":{"custom":"x"}}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.data),
            ("bad_value", Some(json!({"path":"language"})))
        );
        let error = store
            .call(
                "settings.patch",
                &json!({"patch":{"language":{"custom":"x"}}}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.data),
            ("bad_value", Some(json!({"paths":["language"]})))
        );
        let error = store
            .call(
                "settings.patch",
                &json!({"patch":{"language":"fr","theme":{"mode":"light"}}}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.data),
            ("invalid_value", Some(json!({"paths":["language"]})))
        );
        assert!(!fixture.0.join("settings.json").exists());
        assert_eq!(
            call(
                &store,
                "settings.set",
                json!({"path":"language","value":"en"})
            )
            .0["language"],
            "en"
        );
        assert_eq!(
            call(&store, "settings.patch", json!({"patch":{"language":"nl"}})).0["language"],
            "nl"
        );
    }
    #[test]
    fn settings_errors_render_in_the_language_of_their_settings_file() {
        // the en render test of the settings stem, through the real call sites
        let fixture = Fixture::new();
        let store = fixture.store();
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"en"}),
        );
        let error = store
            .call("settings.set", &json!({"path":"language","value":"de"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.message, "This value isn’t valid for this setting.");
        assert_eq!(
            error.json(),
            json!({"code":"invalid_value","message":"This value isn’t valid for this setting.","key":"settings.badValue","params":{},"data":{"path":"language"}})
        );
        let error = store.call("settings.set", &json!({})).unwrap().unwrap_err();
        assert_eq!(error.message, "Specify which setting to change.");
        let error = store
            .call("settings.set", &json!({"path":"version","value":2}))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.message, "This setting can’t be changed.");
        let error = store
            .call("settings.set", &json!({"path":"theme..mode","value":"x"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            ("bad_path", "Invalid setting.")
        );
        let error = store
            .call("settings.import", &json!({"json":"[]"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(error.message, "This file doesn’t contain valid settings.");
        let exported = call(&store, "settings.export", json!({})).0;
        assert!(
            exported["name"]
                .as_str()
                .unwrap()
                .starts_with("win11shell-settings-")
        );
        assert!(exported["name"].as_str().unwrap().ends_with(".json"));
        let error = save_error(store.lang(), io::Error::other("disk full"));
        assert_eq!(
            error.json(),
            json!({"code":"save_failed","message":"Couldn’t save the settings.","key":"settings.saveFailed","params":{},"data":"disk full"})
        );
        // and in Dutch again after a switch back, with the 3b898e6 bytes
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"nl"}),
        );
        let error = store.call("settings.set", &json!({})).unwrap().unwrap_err();
        assert_eq!(error.message, "Geef aan welke instelling je wilt wijzigen.");
        assert!(
            call(&store, "settings.export", json!({})).0["name"]
                .as_str()
                .unwrap()
                .starts_with("win11shell-instellingen-")
        );
    }
    #[test]
    fn concurrent_independent_stores_do_not_lose_disjoint_writes() {
        let fixture = Fixture::new();
        let mut threads = Vec::new();
        for index in 0..8 {
            let directory = fixture.0.clone();
            threads.push(std::thread::spawn(move || {
                let store = SettingsStore::new(directory).unwrap();
                call(
                    &store,
                    "settings.set",
                    json!({"path":["future",format!("key{index}")],"value":index}),
                );
            }));
        }
        for thread in threads {
            thread.join().unwrap();
        }
        let data = fixture.store().snapshot().unwrap();
        for index in 0..8 {
            assert_eq!(data["future"][format!("key{index}")], index);
        }
    }
    #[test]
    fn snapshot_does_not_suppress_external_notifications_and_noop_does_not_notify() {
        let fixture = Fixture::new();
        let first = fixture.store();
        let second = fixture.store();
        assert!(
            call(
                &first,
                "settings.set",
                json!({"path":"language","value":"nl"})
            )
            .1
            .is_none()
        );
        assert!(!fixture.0.join("settings.json").exists());
        call(
            &first,
            "settings.set",
            json!({"path":"language","value":"en"}),
        );
        assert_eq!(second.snapshot().unwrap()["language"], "en");
        let change = second.reload().unwrap().unwrap();
        assert_eq!(change.changed, vec!["language"]);
        assert!(change.path.is_none());
        assert!(second.reload().unwrap().is_none());
    }
    #[test]
    fn restoring_a_peers_write_notifies_even_when_it_matches_our_watermark() {
        let fixture = Fixture::new();
        let first = fixture.store();
        let peer = fixture.store();
        call(
            &peer,
            "settings.set",
            json!({"path":"theme.mode","value":"light"}),
        );
        let change = call(
            &first,
            "settings.set",
            json!({"path":"theme.mode","value":"dark"}),
        )
        .1
        .unwrap();
        assert_eq!(change.changed, vec!["theme.mode"]);
        assert_eq!(change.settings["theme"]["mode"], "dark");
        assert_eq!(change.path.as_deref(), Some("theme.mode"));
        assert!(
            call(
                &first,
                "settings.set",
                json!({"path":"theme.mode","value":"dark"})
            )
            .1
            .is_none()
        );
    }
    #[test]
    fn persisted_theme_paths_are_preserved_alongside_other_observed_changes() {
        let fixture = Fixture::new();
        let first = fixture.store();
        let peer = fixture.store();
        call(
            &peer,
            "settings.set",
            json!({"path":"theme.mode","value":"light"}),
        );
        let change = call(
            &first,
            "settings.patch",
            json!({"patch":{"language":"en","theme":{"mode":"dark"}}}),
        )
        .1
        .unwrap();
        assert!(change.changed.contains(&"language".into()));
        assert!(change.changed.contains(&"theme.mode".into()));
        assert_eq!(change.settings["theme"]["mode"], "dark");
    }
    #[test]
    fn patch_reset_import_and_export_keep_original_shapes_and_order() {
        let fixture = Fixture::new();
        let store = fixture.store();
        call(
            &store,
            "settings.patch",
            json!({"patch":{"version":99,"theme":{"mode":"light"},"future":{"z":1}}}),
        );
        assert_eq!(store.snapshot().unwrap()["version"], 1);
        assert_eq!(store.snapshot().unwrap()["theme"]["accent"], "#0067c0");
        call(&store, "settings.reset", json!({"path":"future.z"}));
        assert_eq!(store.snapshot().unwrap()["future"], json!({}));
        let imported = call(
            &store,
            "settings.import",
            json!({"json":{"language":"en","version":99,"theme":{"mode":12},"extra":true}}),
        )
        .0;
        assert_eq!(imported["version"], 1);
        assert_eq!(imported["theme"]["mode"], "dark");
        assert_eq!(imported["extra"], true);
        let exported = call(&store, "settings.export", json!({})).0;
        let text = exported["json"].as_str().unwrap();
        assert!(text.starts_with("{\n  \"version\": 1,\n  \"language\": \"en\","));
        assert!(!text.ends_with('\n'));
        // the import above set the language to en, so the suggested name is the English one (R2-A); the Dutch
        // name is checked in settings_errors_render_in_the_language_of_their_settings_file
        assert!(
            exported["name"]
                .as_str()
                .unwrap()
                .starts_with("win11shell-settings-")
        );
        assert_eq!(
            store
                .call("settings.import", &json!({"json":{"onlyUnknown":1}}))
                .unwrap()
                .unwrap_err()
                .code,
            "invalid_settings"
        );
        assert_eq!(
            store
                .call("settings.import", &json!({"json":"[]"}))
                .unwrap()
                .unwrap_err()
                .code,
            "invalid_settings"
        );
    }
    #[test]
    fn an_import_drops_an_invalid_language_and_keeps_the_current_one() {
        // D-G31: the language settings.set would refuse (D-G24) is dropped; the rest of the file is imported
        let fixture = Fixture::new();
        let store = fixture.store();
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"en"}),
        );
        for language in [
            json!("fr"),
            json!("EN"),
            json!(""),
            json!({"custom":"nl"}),
            json!(1),
            json!(null),
            json!(["nl"]),
        ] {
            let imported = call(
                &store,
                "settings.import",
                json!({"json":{"language":language,"theme":{"mode":"light"}}}),
            )
            .0;
            assert_eq!(imported["language"], "en", "{language:?}");
            assert_eq!(imported["theme"]["mode"], "light", "{language:?}");
            assert_eq!(store.snapshot().unwrap()["language"], "en", "{language:?}");
            call(
                &store,
                "settings.set",
                json!({"path":"theme.mode","value":"dark"}),
            );
        }
        assert_eq!(crate::i18n::language_in(&fixture.0), Lang::En);
        // as JSON text, and with Dutch as the current language
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"nl"}),
        );
        let imported = call(
            &store,
            "settings.import",
            json!({"json":"{\"language\":\"de\",\"taskbar\":{\"alignment\":\"left\"}}"}),
        )
        .0;
        assert_eq!(
            (&imported["language"], &imported["taskbar"]["alignment"]),
            (&json!("nl"), &json!("left"))
        );
        // a valid language is imported; D-G49: a file without one keeps the current one (en, then nl)
        let imported = call(&store, "settings.import", json!({"json":{"language":"en"}})).0;
        assert_eq!(imported["language"], "en");
        let imported = call(
            &store,
            "settings.import",
            json!({"json":{"theme":{"mode":"light"}}}),
        )
        .0;
        assert_eq!(
            (&imported["language"], &imported["theme"]["mode"]),
            (&json!("en"), &json!("light"))
        );
        assert_eq!(store.snapshot().unwrap()["language"], "en");
        let imported = call(
            &store,
            "settings.import",
            json!({"json":"{\"version\":1,\"taskbar\":{\"alignment\":\"center\"}}"}),
        )
        .0;
        assert_eq!(imported["language"], "en", "as JSON text");
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"nl"}),
        );
        let imported = call(
            &store,
            "settings.import",
            json!({"json":{"theme":{"mode":"dark"}}}),
        )
        .0;
        assert_eq!(imported["language"], "nl");
        assert_eq!(crate::i18n::language_in(&fixture.0), Lang::Nl);
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"en"}),
        );
        // a hand-edited current value that is no language (C1 reads it as nl) is not kept: the default is
        fs::write(fixture.0.join("settings.json"), r#"{"language":"EN"}"#).unwrap();
        let imported = call(
            &store,
            "settings.import",
            json!({"json":{"language":"xx","theme":{"mode":"light"}}}),
        )
        .0;
        assert_eq!(imported["language"], "nl");
    }
    #[test]
    fn bad_config_is_retained_and_backups_are_private_unique_and_complete() {
        let fixture = Fixture::new();
        fs::write(fixture.0.join("settings.json"), "broken \u{e9}").unwrap();
        let store = fixture.store();
        let broken = fs::read_dir(&fixture.0)
            .unwrap()
            .flatten()
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("settings.broken-")
            })
            .unwrap();
        assert_eq!(fs::read_to_string(broken.path()).unwrap(), "broken \u{e9}");
        assert_eq!(store.snapshot().unwrap(), defaults().clone());
        let first = call(&store, "settings.backupFile", json!({})).0;
        let second = call(&store, "settings.backupFile", json!({})).0;
        assert_ne!(first["path"], second["path"]);
        let path = PathBuf::from(first["path"].as_str().unwrap());
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(path).unwrap()).unwrap(),
            defaults().clone()
        );
    }
    #[test]
    fn failed_save_rolls_back_memory_and_keeps_existing_file() {
        let fixture = Fixture::new();
        let store = fixture.store();
        call(
            &store,
            "settings.set",
            json!({"path":"language","value":"en"}),
        );
        let before = fs::read(fixture.0.join("settings.json")).unwrap();
        fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o500)).unwrap();
        let result = store
            .call("settings.set", &json!({"path":"language","value":"nl"}))
            .unwrap();
        fs::set_permissions(&fixture.0, fs::Permissions::from_mode(0o700)).unwrap();
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(result.unwrap_err().code, "save_failed");
            assert_eq!(fs::read(fixture.0.join("settings.json")).unwrap(), before);
            assert_eq!(store.snapshot().unwrap()["language"], "en");
            assert!(store.reload().unwrap().is_none());
        }
    }
    #[test]
    fn path_validation_and_python_numeric_equality_preserve_noop_behavior() {
        let fixture = Fixture::new();
        let store = fixture.store();
        assert_eq!(
            store
                .call("settings.set", &json!({"path":"theme..mode","value":"x"}))
                .unwrap()
                .unwrap_err()
                .data,
            Some(json!({"path":"theme..mode"}))
        );
        assert_eq!(
            store
                .call("settings.set", &json!({"path":null,"value":1}))
                .unwrap()
                .unwrap_err()
                .code,
            "bad_path"
        );
        assert!(
            call(
                &store,
                "settings.set",
                json!({"path":"accessibility.textScale","value":100.0})
            )
            .1
            .is_none()
        );
        call(
            &store,
            "settings.set",
            json!({"path":["unknown",true,null],"value":7}),
        );
        assert_eq!(store.snapshot().unwrap()["unknown"]["True"]["None"], 7);
        assert!(py_equal(
            &json!(9007199254740992_u64),
            &json!(9007199254740992_f64)
        ));
        assert!(!py_equal(
            &json!(9007199254740993_u64),
            &json!(9007199254740992_f64)
        ));
    }
    #[test]
    fn multi_display_taskbar_settings_have_windows_defaults_and_only_its_three_modes() {
        let fixture = Fixture::new();
        let store = fixture.store();
        let data = store.snapshot().unwrap();
        assert_eq!(data["taskbar"]["allDisplays"], true);
        assert_eq!(data["taskbar"]["multiDisplayApps"], "window");
        for mode in ["all", "mainAndWindow", "window"] {
            call(
                &store,
                "settings.set",
                json!({"path":"taskbar.multiDisplayApps","value":mode}),
            );
            assert_eq!(
                store.snapshot().unwrap()["taskbar"]["multiDisplayApps"],
                mode
            );
        }
        for bad in [json!("main"), json!(""), json!(1), json!(null)] {
            let error = store
                .call(
                    "settings.set",
                    &json!({"path":"taskbar.multiDisplayApps","value":bad}),
                )
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, "bad_value");
            assert_eq!(error.data, Some(json!({"path":"taskbar.multiDisplayApps"})));
        }
        // under a whole taskbar object or in a patch too
        let error = store
            .call(
                "settings.set",
                &json!({"path":"taskbar","value":{"multiDisplayApps":"every"}}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.data,
            Some(json!({"paths":["taskbar.multiDisplayApps"]}))
        );
        let error = store
            .call(
                "settings.patch",
                &json!({"patch":{"taskbar":{"multiDisplayApps":"every"}}}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.data,
            Some(json!({"paths":["taskbar.multiDisplayApps"]}))
        );
        call(
            &store,
            "settings.set",
            json!({"path":"taskbar.allDisplays","value":false}),
        );
        assert_eq!(store.snapshot().unwrap()["taskbar"]["allDisplays"], false);
        assert_eq!(
            store
                .call(
                    "settings.set",
                    &json!({"path":"taskbar.allDisplays","value":"no"}),
                )
                .unwrap()
                .unwrap_err()
                .code,
            "bad_value"
        );
        // a value written by hand reads as the default
        let path = fixture.0.join("settings.json");
        fs::write(&path, "{\"taskbar\":{\"multiDisplayApps\":\"every\"}}").unwrap();
        assert_eq!(
            store.snapshot().unwrap()["taskbar"]["multiDisplayApps"],
            "window"
        );
    }
}

/// Settings parity, amendment 18 (batch E): `settings.language` takes "system", the session's language, besides
/// "nl" and "en". A module of its own, so the multi-monitor branch's tests at the end of `mod tests` merge cleanly.
#[cfg(test)]
mod language_tests {
    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn store() -> (Dir, SettingsStore) {
        let path = std::env::temp_dir().join(format!("win11-language-{}", nonce()));
        fs::create_dir(&path).unwrap();
        let store = SettingsStore::new(path.clone()).unwrap();
        (Dir(path), store)
    }

    #[test]
    fn system_is_a_language_setting_and_reads_as_the_sessions_language() {
        let (dir, store) = store();
        for value in ["system", "en", "nl"] {
            store
                .call("settings.set", &json!({"path": "language", "value": value}))
                .unwrap()
                .unwrap();
            assert_eq!(store.snapshot().unwrap()["language"], value);
        }
        store
            .call("settings.patch", &json!({"patch": {"language": "system"}}))
            .unwrap()
            .unwrap();
        assert_eq!(store.snapshot().unwrap()["language"], "system");
        // read as the session's language (C1 extended): whatever this process's environment says
        assert_eq!(crate::i18n::language_in(&dir.0), Lang::system());
        for refused in ["System", "SYSTEM", "system ", "auto", "default"] {
            let error = store
                .call(
                    "settings.set",
                    &json!({"path": "language", "value": refused}),
                )
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, "invalid_value", "{refused:?}");
        }
        assert!(valid_language(&json!("system")) && !valid_language(&json!({"system": true})));
    }

    #[test]
    fn an_imported_file_keeps_its_system_language() {
        let (_dir, store) = store();
        let (data, _) = store
            .call(
                "settings.import",
                &json!({"json": {"language": "system", "theme": {"mode": "light"}}}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(data["language"], "system");
        assert_eq!(store.snapshot().unwrap()["language"], "system");
    }
}
