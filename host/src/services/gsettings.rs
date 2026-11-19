//! `gsettings.*`: GNOME's own settings, read and written the way GNOME Settings does (notes/claude/settings-parity.md
//! §6.1). Settings pages bind their controls to these keys through native/apps/settings/src/gs.rs.
//!
//! * `gsettings.get {keys: [{schema, key, path?}]}` → `{items: [Item]}`.
//! * `gsettings.set {schema, path?, values: {key: value}}` or `{writes: [{schema, path?, values}]}` (dangerous):
//!   every key of every write is checked before anything is queued (allowlisted in a writing mode, the right type,
//!   `range_check`, writable, no accelerator the extension holds), then per write `delay`, `set_value`, `apply`, and
//!   after all of them `Settings::sync` and a read-back of every key. Writes go in the given order (the proxy page
//!   puts `mode` last, amendment 51). Under `ctx.dry_run` only the checks run.
//! * `gsettings.reset {schema, path?, keys}` (dangerous). A service resets its own keys with [`reset_values`], as it
//!   writes them with [`write_values`].
//! * Event `gsettings.changed {schema, path, key, item}` for every change of a schema this service has opened
//!   (connect_changed and connect_writable_changed), also a change made elsewhere.
//! * Paths are GSettings' valid paths only ([`path_ok`]: `/` at both ends, no `//`): `g_settings_new_full` gives NULL
//!   for any other, which glib-rs does not check in a release build. A relocatable schema is opened only at a path an
//!   allow row matches; any other path is reported `allowed: false` without a settings object.
//!
//! `gio::Settings` is not `Send`: one worker thread with its own GLib MainContext owns every object of the service
//! (as system.rs does for its watchers), and the HTTP threads hand it jobs and wait for the answer.
pub(crate) mod allow;
pub(crate) mod owned;
#[cfg(test)]
mod reset_tests;
#[cfg(test)]
mod tests;
pub(crate) mod value;

use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use allow::{Allow, Mode};
use gio::prelude::*;
use serde_json::{Value, json};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::Duration;
use value::Kind;

/// The methods (name, dangerous), as host/src/api-methods/foundation.json lists them.
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("gsettings.get", false),
    ("gsettings.reset", true),
    ("gsettings.set", true),
];
const MAX_KEYS: usize = 256;
const MAX_WRITES: usize = 32;
const JOB_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) type Emit = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// The backend of the worker's settings: GNOME's (dconf, or the sandboxes' keyfile), or one of the test backends.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)] // the test backends
pub(crate) enum Backend {
    #[default]
    Default,
    Memory,
    Null,
    /// A keyfile backend whose `defaults-dir` holds a `locks` file (lockdown in tests).
    Keyfile {
        file: PathBuf,
        defaults: PathBuf,
    },
}

/// What a service instance runs on.
#[derive(Clone, Debug, Default)]
pub(crate) struct Config {
    pub backend: Backend,
    /// Where the extension's compiled schema may be; None: owned.rs `extension_schema_dirs`.
    pub extension_dirs: Option<Vec<PathBuf>>,
}

fn keyfile_backend(file: &Path, defaults: &Path) -> Option<gio::SettingsBackend> {
    let (file, defaults) = (file.to_str()?, defaults.to_str()?);
    // registers GKeyfileSettingsBackend, whose type is private
    drop(gio::keyfile_settings_backend_new(file, "/", None));
    let ty = glib::Type::from_name("GKeyfileSettingsBackend")?;
    if !glib::object::ObjectClass::from_type(ty)?.has_property("defaults-dir", None) {
        return None;
    }
    glib::Object::builder_with_type(ty)
        .property("filename", file)
        .property("root-path", "/")
        .property("defaults-dir", defaults)
        .build()
        .downcast::<gio::SettingsBackend>()
        .ok()
}

/// The schemas, the backend and the opened settings of one set of operations.
pub(crate) struct Env {
    source: Option<gio::SettingsSchemaSource>,
    extension: Option<gio::SettingsSchemaSource>,
    backend: Option<gio::SettingsBackend>,
    open: HashMap<(String, String), gio::Settings>,
    emit: Option<Emit>,
}

impl Env {
    pub(crate) fn new(config: &Config, emit: Option<Emit>) -> Env {
        let dirs = config
            .extension_dirs
            .clone()
            .unwrap_or_else(owned::extension_schema_dirs);
        let backend = match &config.backend {
            Backend::Default => None,
            Backend::Memory => Some(gio::memory_settings_backend_new()),
            Backend::Null => Some(gio::null_settings_backend_new()),
            Backend::Keyfile { file, defaults } => keyfile_backend(file, defaults),
        };
        Env {
            source: gio::SettingsSchemaSource::default(),
            extension: owned::extension_source(&dirs),
            backend,
            open: HashMap::new(),
            emit,
        }
    }

    /// A schema by id: the extension's from its own directory, the rest from the default source.
    pub(crate) fn schema(&self, id: &str) -> Option<gio::SettingsSchema> {
        if id == owned::EXTENSION_SCHEMA
            && let Some(schema) = self.extension.as_ref().and_then(|s| s.lookup(id, false))
        {
            return Some(schema);
        }
        self.source.as_ref()?.lookup(id, true)
    }

    /// The settings object of (schema, path), opened once and watched when the service emits events. None for a
    /// relocatable schema at a path GSettings refuses ([`path_ok`]: `g_settings_new_full` would give NULL).
    pub(crate) fn settings(
        &mut self,
        schema: &gio::SettingsSchema,
        path: &str,
    ) -> Option<gio::Settings> {
        let id = schema.id().to_string();
        let slot = (id.clone(), path.to_owned());
        if let Some(settings) = self.open.get(&slot) {
            return Some(settings.clone());
        }
        let relocatable = schema.path().is_none();
        if relocatable && !path_ok(path) {
            return None;
        }
        let settings =
            gio::Settings::new_full(schema, self.backend.as_ref(), relocatable.then_some(path));
        if let Some(emit) = &self.emit {
            for writable in [false, true] {
                let (emit, id, path) = (emit.clone(), id.clone(), path.to_owned());
                let notify = move |s: &gio::Settings, key: &str| {
                    let row = find_row(&id, &path, s.settings_schema().as_ref(), key);
                    emit(
                        "gsettings.changed",
                        json!({"schema": id, "path": path, "key": key, "item": item(s, &id, &path, key, row)}),
                    );
                };
                if writable {
                    settings.connect_writable_changed(None, notify);
                } else {
                    settings.connect_changed(None, notify);
                }
            }
        }
        self.open.insert(slot, settings.clone());
        Some(settings)
    }

    #[cfg(test)]
    pub(crate) fn opened(&self) -> usize {
        self.open.len()
    }

    /// The extension's current bindings (the held accelerators of amendment 13).
    fn held(&self) -> owned::Held {
        owned::held(self.extension.as_ref(), self.backend.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn backend(&self) -> Option<&gio::SettingsBackend> {
        self.backend.as_ref()
    }
}

/// The allow row of a key at a resolved path: relocatable rows match the path, fixed rows need the schema's path.
fn find_row(
    schema_id: &str,
    path: &str,
    schema: Option<&gio::SettingsSchema>,
    key: &str,
) -> Option<&'static Allow> {
    match schema.and_then(|s| s.path()) {
        Some(own) => (own.as_str() == path)
            .then(|| allow::find(schema_id, None, key))
            .flatten(),
        None => allow::find(schema_id, Some(path), key),
    }
}

/// The API's view of one key.
pub(crate) fn item(
    settings: &gio::Settings,
    schema_id: &str,
    path: &str,
    key: &str,
    row: Option<&Allow>,
) -> Value {
    let mut out = json!({
        "schema": schema_id, "path": path, "key": key, "allowed": row.is_some(), "available": false,
        "writable": false, "mode": row.map(|r| r.mode.name()), "setting": Value::Null, "method": Value::Null,
        "value": Value::Null, "default": Value::Null, "isDefault": Value::Null, "type": Value::Null,
        "range": Value::Null,
    });
    match row.map(|r| r.mode) {
        Some(Mode::Owned(s) | Mode::Mirrored(s)) => out["setting"] = json!(s),
        Some(Mode::Service(m)) => out["method"] = json!(m),
        _ => {}
    }
    let Some(schema) = settings.settings_schema() else {
        return out;
    };
    if !schema.has_key(key) {
        return out;
    }
    out["available"] = json!(true);
    out["writable"] = json!(settings.is_writable(key));
    if row.is_none() {
        return out; // not allowlisted: not read either
    }
    let k = schema.key(key);
    let kind = value::kind(&k);
    out["value"] = value::to_json(&kind, &settings.value(key));
    out["default"] = settings
        .default_value(key)
        .map_or(Value::Null, |d| value::to_json(&kind, &d));
    out["isDefault"] = json!(settings.user_value(key).is_none());
    out["type"] = json!(value::type_name(&kind));
    out["range"] = value::range_json(&k, &kind);
    out
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "gsettings.badArgs", NO_PARAMS)
}

fn with_data(mut error: ApiError, data: Value) -> ApiError {
    error.data = Some(data);
    error
}

fn unavailable(lang: Lang, schema: &str, key: Option<&str>) -> ApiError {
    with_data(
        ApiError::tr("unsupported", lang, "gsettings.unavailable", NO_PARAMS),
        json!({"schema": schema, "key": key}),
    )
}

fn schema_id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
}

fn key_ok(s: &str) -> bool {
    !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// One key a call names.
#[derive(Clone, Debug)]
pub(crate) struct KeyRef {
    pub schema: String,
    pub path: Option<String>,
    pub key: String,
}

/// One write of a `gsettings.set`: keys of one schema at one path.
#[derive(Clone, Debug)]
pub(crate) struct Write {
    pub schema: String,
    pub path: Option<String>,
    pub values: Vec<(String, Value)>,
}

/// A path GSettings takes (gsettings.c `path_is_valid`: `/` at both ends, no `//`), of the characters the allow
/// tables' paths and holes use, at most 512 bytes.
pub(crate) fn path_ok(p: &str) -> bool {
    p.starts_with('/')
        && p.ends_with('/')
        && !p.contains("//")
        && p.len() <= 512
        && p.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b':'))
}

fn opt_path(args: &Value, lang: Lang) -> Result<Option<String>, ApiError> {
    match &args["path"] {
        Value::Null => Ok(None),
        Value::String(p) if path_ok(p) => Ok(Some(p.clone())),
        _ => Err(bad(lang)),
    }
}

fn schema_arg(args: &Value, lang: Lang) -> Result<String, ApiError> {
    args["schema"]
        .as_str()
        .filter(|s| schema_id_ok(s))
        .map(str::to_owned)
        .ok_or_else(|| bad(lang))
}

pub(crate) fn parse_get(args: &Value, lang: Lang) -> Result<Vec<KeyRef>, ApiError> {
    let keys = args["keys"]
        .as_array()
        .filter(|k| k.len() <= MAX_KEYS)
        .ok_or_else(|| bad(lang))?;
    keys.iter()
        .map(|k| {
            Ok(KeyRef {
                schema: schema_arg(k, lang)?,
                path: opt_path(k, lang)?,
                key: k["key"]
                    .as_str()
                    .filter(|s| key_ok(s))
                    .ok_or_else(|| bad(lang))?
                    .to_owned(),
            })
        })
        .collect()
}

fn parse_write(args: &Value, lang: Lang) -> Result<Write, ApiError> {
    let values = args["values"]
        .as_object()
        .filter(|v| !v.is_empty() && v.len() <= 64)
        .ok_or_else(|| bad(lang))?;
    Ok(Write {
        schema: schema_arg(args, lang)?,
        path: opt_path(args, lang)?,
        values: values
            .iter()
            .map(|(k, v)| {
                key_ok(k)
                    .then(|| (k.clone(), v.clone()))
                    .ok_or_else(|| bad(lang))
            })
            .collect::<Result<_, _>>()?,
    })
}

pub(crate) fn parse_set(args: &Value, lang: Lang) -> Result<Vec<Write>, ApiError> {
    match args.get("writes") {
        Some(writes) => {
            let writes = writes
                .as_array()
                .filter(|w| !w.is_empty() && w.len() <= MAX_WRITES)
                .ok_or_else(|| bad(lang))?;
            if args.get("schema").is_some() || args.get("values").is_some() {
                return Err(bad(lang));
            }
            writes.iter().map(|w| parse_write(w, lang)).collect()
        }
        None => Ok(vec![parse_write(args, lang)?]),
    }
}

pub(crate) fn parse_reset(
    args: &Value,
    lang: Lang,
) -> Result<(String, Option<String>, Vec<String>), ApiError> {
    let keys = args["keys"]
        .as_array()
        .filter(|k| !k.is_empty() && k.len() <= 64)
        .ok_or_else(|| bad(lang))?
        .iter()
        .map(|k| {
            k.as_str()
                .filter(|s| key_ok(s))
                .map(str::to_owned)
                .ok_or_else(|| bad(lang))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((schema_arg(args, lang)?, opt_path(args, lang)?, keys))
}

/// A schema and the path its settings live at: the schema's own path (a given one must equal it), or the given
/// path of a relocatable schema (a valid one, [`path_ok`]: the Rust API's callers reach here without `opt_path`).
fn resolve(
    env: &Env,
    schema_id: &str,
    path: Option<&str>,
    lang: Lang,
) -> Result<(gio::SettingsSchema, String), ApiError> {
    let schema = env
        .schema(schema_id)
        .ok_or_else(|| unavailable(lang, schema_id, None))?;
    match (schema.path().map(|p| p.to_string()), path) {
        (Some(own), None) => Ok((schema, own)),
        (Some(own), Some(p)) if own == p => Ok((schema, own)),
        (None, Some(p)) if path_ok(p) => Ok((schema, p.to_owned())),
        _ => Err(bad(lang)),
    }
}

/// The allow row of a requested key (before the schema is known: a missing schema still reports notAllowed first).
fn requested_row(schema: &str, path: Option<&str>, key: &str) -> Option<&'static Allow> {
    match path {
        None => allow::find(schema, None, key),
        Some(p) => allow::find(schema, Some(p), key).or_else(|| allow::find(schema, None, key)),
    }
}

/// An item without a settings object: a missing schema, or a relocatable schema at a path no allow row matches.
fn bare_item(
    schema: &str,
    path: Option<&str>,
    key: &str,
    row: Option<&Allow>,
    available: bool,
) -> Value {
    json!({
        "schema": schema, "path": path, "key": key, "allowed": row.is_some(), "available": available,
        "writable": false, "mode": row.map(|r| r.mode.name()), "setting": Value::Null, "method": Value::Null,
        "value": Value::Null, "default": Value::Null, "isDefault": Value::Null, "type": Value::Null,
        "range": Value::Null,
    })
}

pub(crate) fn get(env: &mut Env, keys: &[KeyRef], lang: Lang) -> Result<Value, ApiError> {
    let mut items = Vec::with_capacity(keys.len());
    for k in keys {
        let row = requested_row(&k.schema, k.path.as_deref(), &k.key);
        if env.schema(&k.schema).is_none() {
            items.push(bare_item(&k.schema, k.path.as_deref(), &k.key, row, false));
            continue;
        }
        let (schema, path) = resolve(env, &k.schema, k.path.as_deref(), lang)?;
        let row = find_row(&k.schema, &path, Some(&schema), &k.key);
        if row.is_none() && schema.path().is_none() {
            // never a settings object (nor a watched one in `open`) at a path no row allows
            let available = schema.has_key(&k.key);
            items.push(bare_item(&k.schema, Some(&path), &k.key, None, available));
            continue;
        }
        let settings = env.settings(&schema, &path).ok_or_else(|| bad(lang))?;
        items.push(item(&settings, &k.schema, &path, &k.key, row));
    }
    Ok(json!({ "items": items }))
}

struct Planned {
    settings: gio::Settings,
    schema: String,
    path: String,
    values: Vec<(String, glib::Variant)>,
}

/// Who writes: `gsettings.set` / `gsettings.reset` (rows in mode Write or Mirrored), or a host method of another
/// service through [`write_values`] (also the rows in mode `Service(that method)`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Caller<'a> {
    Api,
    Service(&'a str),
}

fn refuse(row: Option<&Allow>, caller: Caller, lang: Lang) -> Option<ApiError> {
    Some(match row.map(|r| r.mode) {
        None => ApiError::tr("denied", lang, "gsettings.notAllowed", NO_PARAMS),
        Some(Mode::Write | Mode::Mirrored(_)) => return None,
        Some(Mode::Service(method)) if caller == Caller::Service(method) => return None,
        Some(Mode::Read) => ApiError::tr("denied", lang, "gsettings.readOnly", NO_PARAMS),
        Some(Mode::Service(method)) => with_data(
            ApiError::tr("denied", lang, "gsettings.useMethod", NO_PARAMS),
            json!({ "method": method }),
        ),
        Some(Mode::Owned(setting)) => with_data(
            ApiError::tr("denied", lang, "gsettings.ownedByWinPoint", NO_PARAMS),
            json!({ "setting": setting }),
        ),
    })
}

/// Check every key of every write; the planned GVariant writes, or the first refusal. Every key needs its allow row
/// at the resolved path (a relocatable row's typed holes) in a mode `caller` may write: Write or Mirrored, and for
/// [`Caller::Service`] also `Service(that method)`; Read and Owned rows (WinPoint's look, `gnome_look::note`) never.
/// The forbidden schemas and keys and the held accelerators apply to every caller.
fn plan(
    env: &mut Env,
    writes: &[Write],
    lang: Lang,
    caller: Caller,
) -> Result<Vec<Planned>, ApiError> {
    let held = env.held();
    let mut planned = Vec::with_capacity(writes.len());
    for w in writes {
        for (key, _) in &w.values {
            let row = requested_row(&w.schema, w.path.as_deref(), key);
            if allow::forbidden(&w.schema, key) {
                return Err(ApiError::tr(
                    "denied",
                    lang,
                    "gsettings.notAllowed",
                    NO_PARAMS,
                ));
            }
            if let Some(error) = refuse(row, caller, lang) {
                return Err(error);
            }
        }
        let (schema, path) = resolve(env, &w.schema, w.path.as_deref(), lang)?;
        for (key, _) in &w.values {
            if let Some(error) =
                refuse(find_row(&w.schema, &path, Some(&schema), key), caller, lang)
            {
                return Err(error);
            }
        }
        let settings = env.settings(&schema, &path).ok_or_else(|| bad(lang))?;
        let mut values = Vec::with_capacity(w.values.len());
        for (key, json_value) in &w.values {
            if !schema.has_key(key) {
                return Err(unavailable(lang, &w.schema, Some(key)));
            }
            let k = schema.key(key);
            let kind = value::kind(&k);
            let bad_value = || {
                with_data(
                    ApiError::tr("bad_args", lang, "gsettings.badValue", NO_PARAMS),
                    json!({ "key": key }),
                )
            };
            let variant = value::to_variant(&kind, json_value).ok_or_else(bad_value)?;
            if variant.type_() != k.value_type() || !k.range_check(&variant) {
                return Err(bad_value());
            }
            if !settings.is_writable(key) {
                return Err(with_data(
                    ApiError::tr("denied", lang, "gsettings.locked", NO_PARAMS),
                    json!({ "key": key }),
                ));
            }
            if matches!(kind, Kind::StrList)
                && let Some(list) = variant.get::<Vec<String>>()
                && let Some((accel, setting)) = owned::held_accel(&w.schema, key, &list, &held)
            {
                return Err(with_data(
                    ApiError::tr("denied", lang, "gsettings.heldByWinPoint", NO_PARAMS),
                    json!({ "key": key, "accel": accel, "setting": setting }),
                ));
            }
            values.push((key.clone(), variant));
        }
        planned.push(Planned {
            settings,
            schema: w.schema.clone(),
            path,
            values,
        });
    }
    Ok(planned)
}

fn items_of(planned: &[Planned]) -> Value {
    let items: Vec<Value> = planned
        .iter()
        .flat_map(|p| {
            let schema = p.settings.settings_schema();
            p.values.iter().map(move |(key, _)| {
                let row = find_row(&p.schema, &p.path, schema.as_ref(), key);
                item(&p.settings, &p.schema, &p.path, key, row)
            })
        })
        .collect();
    json!({ "items": items })
}

/// `gsettings.set` on `env` (see the module docs).
pub(crate) fn set(
    env: &mut Env,
    writes: &[Write],
    lang: Lang,
    dry_run: bool,
    caller: Caller,
) -> Result<Value, ApiError> {
    let planned = plan(env, writes, lang, caller)?;
    if dry_run {
        return Ok(items_of(&planned));
    }
    for p in &planned {
        p.settings.delay();
        for (key, variant) in &p.values {
            if p.settings.set_value(key, variant).is_err() {
                p.settings.revert();
                return Err(ApiError::tr(
                    "failed",
                    lang,
                    "gsettings.saveFailed",
                    NO_PARAMS,
                ));
            }
        }
        p.settings.apply();
    }
    gio::Settings::sync();
    for p in &planned {
        for (key, variant) in &p.values {
            if p.settings.value(key) != *variant {
                return Err(with_data(
                    ApiError::tr("failed", lang, "gsettings.notTaken", NO_PARAMS),
                    json!({ "key": key }),
                ));
            }
        }
    }
    Ok(items_of(&planned))
}

/// `gsettings.reset` on `env`.
pub(crate) fn reset(
    env: &mut Env,
    schema_id: &str,
    path: Option<&str>,
    keys: &[String],
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    reset_as(env, schema_id, path, keys, lang, dry_run, Caller::Api)
}

/// [`reset`] for `caller`: the checks of [`set`] (allow row in a mode `caller` may write, forbidden schemas and keys,
/// writable), then `g_settings_reset` of every key.
pub(crate) fn reset_as(
    env: &mut Env,
    schema_id: &str,
    path: Option<&str>,
    keys: &[String],
    lang: Lang,
    dry_run: bool,
    caller: Caller,
) -> Result<Value, ApiError> {
    for key in keys {
        if allow::forbidden(schema_id, key) {
            return Err(ApiError::tr(
                "denied",
                lang,
                "gsettings.notAllowed",
                NO_PARAMS,
            ));
        }
        if let Some(error) = refuse(requested_row(schema_id, path, key), caller, lang) {
            return Err(error);
        }
    }
    let (schema, resolved) = resolve(env, schema_id, path, lang)?;
    for key in keys {
        if let Some(error) = refuse(
            find_row(schema_id, &resolved, Some(&schema), key),
            caller,
            lang,
        ) {
            return Err(error);
        }
    }
    let settings = env.settings(&schema, &resolved).ok_or_else(|| bad(lang))?;
    let held = env.held();
    for key in keys {
        if !schema.has_key(key) {
            return Err(unavailable(lang, schema_id, Some(key)));
        }
        if !settings.is_writable(key) {
            return Err(with_data(
                ApiError::tr("denied", lang, "gsettings.locked", NO_PARAMS),
                json!({ "key": key }),
            ));
        }
        if let Some(default) = settings
            .default_value(key)
            .and_then(|v| v.get::<Vec<String>>())
            && let Some((accel, setting)) = owned::held_accel(schema_id, key, &default, &held)
        {
            return Err(with_data(
                ApiError::tr("denied", lang, "gsettings.heldByWinPoint", NO_PARAMS),
                json!({ "key": key, "accel": accel, "setting": setting }),
            ));
        }
    }
    if !dry_run {
        for key in keys {
            settings.reset(key);
        }
        gio::Settings::sync();
    }
    let items: Vec<Value> = keys
        .iter()
        .map(|key| {
            let row = find_row(schema_id, &resolved, Some(&schema), key);
            item(&settings, schema_id, &resolved, key, row)
        })
        .collect();
    Ok(json!({ "items": items }))
}

/// The Rust API for another service's own keys (§6.1): one key as `gsettings.get` reports it, default backend, on
/// the caller's thread.
#[allow(dead_code)] // the batches' services
pub(crate) fn read_item(
    schema: &str,
    path: Option<&str>,
    key: &str,
    lang: Lang,
) -> Result<Value, ApiError> {
    read_item_in(
        &mut Env::new(&Config::default(), None),
        schema,
        path,
        key,
        lang,
    )
}

/// [`read_item`] on `env` (tests: a memory backend).
pub(crate) fn read_item_in(
    env: &mut Env,
    schema: &str,
    path: Option<&str>,
    key: &str,
    lang: Lang,
) -> Result<Value, ApiError> {
    if path.is_some_and(|p| !path_ok(p)) || !schema_id_ok(schema) || !key_ok(key) {
        return Err(bad(lang));
    }
    let keys = [KeyRef {
        schema: schema.into(),
        path: path.map(str::to_owned),
        key: key.into(),
    }];
    let mut out = get(env, &keys, lang)?;
    Ok(out["items"][0].take())
}

/// The Rust API for host method `method`'s own keys (§6.1): `gsettings.set`'s checks and write, default backend, on
/// the caller's thread. Every key needs its allow row at the resolved path (a relocatable row's typed holes, so a
/// path composed from a method's arguments cannot reach another key) in mode Write, Mirrored or
/// `Service(method)`; Read and Owned rows are refused, and so are the forbidden schemas and the held accelerators.
#[allow(dead_code)] // the batches' services
pub(crate) fn write_values(
    method: &str,
    schema: &str,
    path: Option<&str>,
    values: &[(&str, Value)],
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let mut env = Env::new(&Config::default(), None);
    write_values_in(&mut env, method, schema, path, values, lang, dry_run)
}

/// [`write_values`] on `env` (tests: a memory backend).
pub(crate) fn write_values_in(
    env: &mut Env,
    method: &str,
    schema: &str,
    path: Option<&str>,
    values: &[(&str, Value)],
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    if path.is_some_and(|p| !path_ok(p))
        || !schema_id_ok(schema)
        || values.is_empty()
        || values.iter().any(|(k, _)| !key_ok(k))
    {
        return Err(bad(lang));
    }
    let write = Write {
        schema: schema.into(),
        path: path.map(str::to_owned),
        values: values
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect(),
    };
    set(env, &[write], lang, dry_run, Caller::Service(method))
}

/// The Rust API's reset for host method `method`'s own keys (§6.1): `gsettings.reset`'s checks for
/// `Caller::Service(method)` (the rows [`write_values`] may write, never Read or Owned ones), then `g_settings_reset`
/// of every key, default backend, on the caller's thread (GNOME Settings' alert chooser resets `theme-name`).
#[allow(dead_code)] // the batches' services
pub(crate) fn reset_values(
    method: &str,
    schema: &str,
    path: Option<&str>,
    keys: &[&str],
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    let mut env = Env::new(&Config::default(), None);
    reset_values_in(&mut env, method, schema, path, keys, lang, dry_run)
}

/// [`reset_values`] on `env` (tests: a memory backend).
pub(crate) fn reset_values_in(
    env: &mut Env,
    method: &str,
    schema: &str,
    path: Option<&str>,
    keys: &[&str],
    lang: Lang,
    dry_run: bool,
) -> Result<Value, ApiError> {
    if path.is_some_and(|p| !path_ok(p))
        || !schema_id_ok(schema)
        || keys.is_empty()
        || keys.iter().any(|k| !key_ok(k))
    {
        return Err(bad(lang));
    }
    let keys: Vec<String> = keys.iter().map(|k| (*k).to_owned()).collect();
    reset_as(
        env,
        schema,
        path,
        &keys,
        lang,
        dry_run,
        Caller::Service(method),
    )
}

thread_local! {
    static ENV: RefCell<Option<Env>> = const { RefCell::new(None) };
}

/// The worker thread (module docs).
struct Worker {
    context: glib::MainContext,
    stop: Arc<AtomicBool>,
}

impl Worker {
    fn start(config: Config, emit: Option<Emit>) -> Result<Worker, String> {
        let context = glib::MainContext::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (ready, started) = mpsc::channel();
        let (thread_context, thread_stop) = (context.clone(), stop.clone());
        std::thread::Builder::new()
            .name("win11-gsettings".into())
            .spawn(move || {
                let _ = thread_context.with_thread_default(|| {
                    ENV.with(|e| *e.borrow_mut() = Some(Env::new(&config, emit)));
                    let _ = ready.send(());
                    while !thread_stop.load(Ordering::Acquire) {
                        thread_context.iteration(true);
                    }
                    ENV.with(|e| e.borrow_mut().take());
                });
            })
            .map_err(|e| e.to_string())?;
        started
            .recv_timeout(JOB_TIMEOUT)
            .map_err(|_| "gsettings: the worker did not start".to_owned())?;
        Ok(Worker { context, stop })
    }

    fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Env) -> T + Send + 'static,
    ) -> Option<T> {
        if self.stop.load(Ordering::Acquire) {
            return None;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.context.invoke(move || {
            if let Some(out) = ENV.with(|e| e.borrow_mut().as_mut().map(job)) {
                let _ = tx.send(out);
            }
        });
        rx.recv_timeout(JOB_TIMEOUT).ok()
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct GSettings {
    ctx: Arc<ServiceContext>,
    worker: Worker,
}

impl GSettings {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let events = ctx.events.clone();
        let emit: Emit = Arc::new(move |name, data| events.emit(name, data));
        Self::with(ctx, Config::default(), Some(emit))
    }

    pub(crate) fn with(
        ctx: Arc<ServiceContext>,
        config: Config,
        emit: Option<Emit>,
    ) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            ctx,
            worker: Worker::start(config, emit)?,
        }))
    }

    fn stopped(&self, lang: Lang) -> ApiError {
        ApiError::tr("offline", lang, "core.serviceStopped", NO_PARAMS)
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let lang = self.ctx.lang();
        let dry_run = self.ctx.dry_run;
        Some(match method {
            "gsettings.get" => parse_get(args, lang).and_then(|keys| {
                self.worker
                    .run(move |env| get(env, &keys, lang))
                    .unwrap_or_else(|| Err(self.stopped(lang)))
            }),
            "gsettings.set" => parse_set(args, lang).and_then(|writes| {
                self.worker
                    .run(move |env| set(env, &writes, lang, dry_run, Caller::Api))
                    .unwrap_or_else(|| Err(self.stopped(lang)))
            }),
            "gsettings.reset" => parse_reset(args, lang).and_then(|(schema, path, keys)| {
                self.worker
                    .run(move |env| reset(env, &schema, path.as_deref(), &keys, lang, dry_run))
                    .unwrap_or_else(|| Err(self.stopped(lang)))
            }),
            _ => return None,
        })
    }

    /// Run a job on the worker (tests: a second writer on the same backend, the way another program writes).
    #[cfg(test)]
    pub(crate) fn on_worker<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Env) -> T + Send + 'static,
    ) -> Option<T> {
        self.worker.run(job)
    }

    pub(crate) fn shutdown(&self) {
        self.worker.stop();
    }
}
