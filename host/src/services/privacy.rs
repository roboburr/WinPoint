//! `privacy.*` (batch F, notes/claude/settings-parity.md §5.7): app permissions in xdg-desktop-portal's
//! PermissionStore and one app's details for Instellingen › Apps › Geïnstalleerde apps › Geavanceerde opties
//! (GNOME Settings 50's per-app Apps page): storage and Clear Cache, Run in Background and Autostart, the global
//! shortcuts, a snap's interfaces, the app's file and link types.
//!
//! Writes skip under `ctx.dry_run` (the runtime adds the dryRun envelope) and emit `privacy.changed {what}`.
//! PermissionStore and the global-shortcuts rebind are session-bus calls without polkit; snapd checks polkit itself
//! for its writes (`X-Allow-Interaction`, amendment 42), so a refusal there is `denied core.notAuthorized` or
//! `denied core.authCancelled`, as `call_interactive` reports one.
//!
//! Slice 3 adds Privacy en beveiliging's own pages: the error reports (whoopsie-preferences on the system bus,
//! through `call_interactive`), the system reports (`/usr/bin/ubuntu-insights` through `tools::system_tool`,
//! amendment 38), device security (fwupd, read only) with Ubuntu's Security Center, GNOME's search providers and the
//! sandboxed apps the permission pages name.
use super::ServiceContext;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

mod app;
mod flatpak;
mod handlers;
mod insights;
mod perms;
mod reports;
mod search;
mod security;
mod shortcuts;
mod snapd;
#[cfg(test)]
pub(crate) mod tests;

/// The methods (name, dangerous), exactly as host/src/api-methods/apps.json lists them (a test compares).
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("privacy.app", false),
    ("privacy.app.clearCache", true),
    ("privacy.app.globalShortcuts.set", true),
    ("privacy.app.removeHandler", true),
    ("privacy.app.resetHandlers", true),
    ("privacy.app.setAutostart", true),
    ("privacy.app.snapConnect", true),
    ("privacy.apps.sandboxed", false),
    ("privacy.deletePermission", true),
    ("privacy.insights.get", false),
    ("privacy.insights.report", false),
    ("privacy.insights.set", true),
    ("privacy.permissions", false),
    ("privacy.reports.get", false),
    ("privacy.reports.set", true),
    ("privacy.searchProviders", false),
    ("privacy.security", false),
    ("privacy.securityCenter.open", false),
    ("privacy.setPermission", true),
];

/// Where the service's daemons are (tests: fakes on the private bus, a socket of their own, a stub tool).
pub(super) struct Targets {
    pub store: String,
    pub rebind: String,
    pub snapd: PathBuf,
    /// whoopsie-preferences and fwupd (system bus names)
    pub whoopsie: String,
    pub fwupd: String,
    /// The bus of the system daemons: None is the system bus; tests give the private session bus's address.
    pub system: Option<String>,
    /// ubuntu-insights: None is `tools::system_tool("ubuntu-insights")`.
    pub insights: Option<PathBuf>,
}

impl Default for Targets {
    fn default() -> Targets {
        Targets {
            store: perms::NAME.into(),
            rebind: shortcuts::REBIND_NAME.into(),
            snapd: PathBuf::from(snapd::SOCKET),
            whoopsie: reports::NAME.into(),
            fwupd: security::NAME.into(),
            system: None,
            insights: None,
        }
    }
}

pub(crate) struct Privacy {
    ctx: Arc<ServiceContext>,
    targets: Targets,
    /// A report collection runs (one at a time) and whether the dialog asked to stop it.
    reporting: std::sync::Mutex<bool>,
    cancel_report: std::sync::atomic::AtomicBool,
    /// The PermissionStore watch ([`Privacy::watch_store`]): its stop flag and its thread's main context.
    watch_stop: Arc<std::sync::atomic::AtomicBool>,
    watch_context: std::sync::Mutex<Option<glib::MainContext>>,
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "privacy.badArgs", NO_PARAMS)
}

fn str_arg<'a>(args: &'a Value, key: &str, lang: Lang) -> Result<&'a str, ApiError> {
    args[key].as_str().ok_or_else(|| bad(lang))
}

fn store_error(e: perms::StoreError, lang: Lang) -> ApiError {
    match e {
        perms::StoreError::Missing => {
            ApiError::tr("unsupported", lang, "privacy.storeUnavailable", NO_PARAMS)
        }
        perms::StoreError::Failed(text) => {
            let mut error = ApiError::tr("failed", lang, "privacy.permissionFailed", NO_PARAMS);
            error.data = Some(json!({ "error": text.chars().take(300).collect::<String>() }));
            error
        }
    }
}

fn snap_error(e: snapd::SnapError, lang: Lang) -> ApiError {
    if e.kind == "auth-cancelled" {
        return ApiError::tr("denied", lang, "core.authCancelled", NO_PARAMS);
    }
    if e.status == 401 || e.status == 403 || e.kind == "login-required" {
        return ApiError::tr("denied", lang, "core.notAuthorized", NO_PARAMS);
    }
    if e.status == 0 && e.kind.is_empty() && e.message.contains("os error") {
        return ApiError::tr("unsupported", lang, "privacy.snapUnavailable", NO_PARAMS);
    }
    ApiError::tr(
        "failed",
        lang,
        "privacy.snapFailed",
        &json!({ "error": e.message }),
    )
}

impl Privacy {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let service = Self::with(ctx, Targets::default());
        service.watch_store();
        Ok(service)
    }

    pub(super) fn with(ctx: Arc<ServiceContext>, targets: Targets) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            targets,
            reporting: std::sync::Mutex::new(false),
            cancel_report: std::sync::atomic::AtomicBool::new(false),
            watch_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            watch_context: std::sync::Mutex::new(None),
        })
    }

    /// GNOME's Apps, Location and Camera panels follow the PermissionStore's `Changed` signal; so does this service,
    /// for as long as it lives: every change of an entry of `perms::KINDS` (a portal dialog answered while a page is
    /// open, another program, Settings in a second window, this host's own writes) becomes
    /// `privacy.changed {what: "permissions", table, id}`, on which Locatie, Camera, Microfoon and Geavanceerde
    /// opties read the entry again. One thread with its own main context; it ends with `shutdown` or the service.
    pub(super) fn watch_store(self: &Arc<Self>) {
        use std::sync::atomic::Ordering;
        let weak = Arc::downgrade(self);
        let (store, stop) = (self.targets.store.clone(), self.watch_stop.clone());
        let (ready, started) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("privacy-store-watch".into())
            .spawn(move || {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let Ok(bus) = crate::services::system::common::session() else {
                        let _ = ready.send(None);
                        return;
                    };
                    let relay = weak.clone();
                    let subscription = perms::watch(&bus, &store, move |kind| {
                        if let Some(service) = relay.upgrade() {
                            service.changed(
                                "permissions",
                                json!({ "table": kind.table, "id": kind.id }),
                            );
                        }
                    });
                    // the stop flag is read at least once a second; shutdown also wakes the context at once
                    let tick = glib::timeout_source_new(
                        Duration::from_secs(1),
                        Some("win11-privacy-store-watch"),
                        glib::Priority::DEFAULT,
                        || glib::ControlFlow::Continue,
                    );
                    tick.attach(Some(&context));
                    let _ = ready.send(Some(context.clone()));
                    while !stop.load(Ordering::Acquire) && weak.strong_count() > 0 {
                        context.iteration(true);
                    }
                    tick.destroy();
                    bus.signal_unsubscribe(subscription);
                });
            });
        // the subscription exists once this returns (a change right after the first call is not missed)
        if spawned.is_ok()
            && let Ok(Some(context)) = started.recv_timeout(Duration::from_secs(5))
        {
            *self.watch_context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context);
        }
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(name, _)| *name == method) {
            return None;
        }
        let lang = self.ctx.lang();
        let empty = json!({});
        let args = if args.is_null() { &empty } else { args };
        if !args.is_object() {
            return Some(Err(bad(lang)));
        }
        Some(match method {
            "privacy.permissions" => self.permissions(args, lang),
            "privacy.setPermission" => self.set_permission(args, lang),
            "privacy.deletePermission" => self.delete_permission(args, lang),
            "privacy.app" => self.app(args, lang),
            "privacy.app.clearCache" => self.clear_cache(args, lang),
            "privacy.app.resetHandlers" => self.reset_handlers(args, lang),
            "privacy.app.removeHandler" => self.remove_handler(args, lang),
            "privacy.app.snapConnect" => self.snap_connect(args, lang),
            "privacy.app.globalShortcuts.set" => self.set_shortcuts(args, lang),
            "privacy.app.setAutostart" => self.set_autostart(args, lang),
            "privacy.apps.sandboxed" => Ok(self.sandboxed(lang)),
            "privacy.reports.get" => Ok(self.reports_get()),
            "privacy.reports.set" => self.reports_set(args, lang),
            "privacy.insights.get" => self.insights_get(lang),
            "privacy.insights.set" => self.insights_set(args, lang),
            "privacy.insights.report" => self.insights_report(args, lang),
            "privacy.security" => Ok(self.security()),
            "privacy.securityCenter.open" => self.open_security_center(lang),
            "privacy.searchProviders" => Ok(search::providers(lang)),
            _ => return None,
        })
    }

    pub(crate) fn shutdown(&self) {
        // a report collection that still runs stops with its dialog's host
        self.cancel_report
            .store(true, std::sync::atomic::Ordering::Release);
        self.watch_stop
            .store(true, std::sync::atomic::Ordering::Release);
        if let Some(context) = self
            .watch_context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            context.wakeup();
        }
    }

    fn changed(&self, what: &str, extra: Value) {
        let mut data = json!({ "what": what });
        if let (Some(out), Some(more)) = (data.as_object_mut(), extra.as_object()) {
            out.extend(more.clone());
        }
        self.ctx.emit("privacy.changed", data);
    }

    fn bus(&self) -> Result<gio::DBusConnection, perms::StoreError> {
        crate::services::system::common::session().map_err(|_| perms::StoreError::Missing)
    }

    // ------------------------------------------------------------------------------------------- PermissionStore

    fn kind_of(&self, args: &Value, lang: Lang) -> Result<&'static perms::Kind, ApiError> {
        perms::kind(str_arg(args, "table", lang)?, str_arg(args, "id", lang)?)
            .ok_or_else(|| bad(lang))
    }

    fn permissions(&self, args: &Value, lang: Lang) -> ApiResult {
        let kind = self.kind_of(args, lang)?;
        let found = self
            .bus()
            .and_then(|bus| perms::lookup(&bus, &self.targets.store, kind.table, kind.id));
        let (available, apps) = match found {
            Ok(apps) => (true, apps),
            Err(perms::StoreError::Missing) => (false, Vec::new()),
            Err(e) => return Err(store_error(e, lang)),
        };
        let apps: Vec<Value> = apps
            .iter()
            .map(|(app, value)| json!({ "app": app, "value": value, "allowed": perms::allowed(kind, Some(value)) }))
            .collect();
        Ok(json!({ "table": kind.table, "id": kind.id, "available": available, "apps": apps }))
    }

    fn set_permission(&self, args: &Value, lang: Lang) -> ApiResult {
        let kind = self.kind_of(args, lang)?;
        let app = str_arg(args, "app", lang)?;
        if !perms::app_id_ok(app) {
            return Err(bad(lang));
        }
        let bus = self.bus().map_err(|e| store_error(e, lang))?;
        let current = perms::lookup(&bus, &self.targets.store, kind.table, kind.id)
            .map_err(|e| store_error(e, lang))?;
        let mine = current
            .iter()
            .find(|(a, _)| a == app)
            .map(|(_, v)| v.as_slice());
        let value = perms::parse_value(kind, &args["value"], mine).ok_or_else(|| bad(lang))?;
        let answer = json!({
            "table": kind.table, "id": kind.id, "app": app, "value": value,
            "allowed": perms::allowed(kind, Some(&value)),
        });
        if self.ctx.dry_run {
            return Ok(answer);
        }
        perms::set(&bus, &self.targets.store, kind.table, kind.id, app, &value)
            .map_err(|e| store_error(e, lang))?;
        self.changed(
            "permissions",
            json!({ "table": kind.table, "id": kind.id, "app": app }),
        );
        Ok(answer)
    }

    fn delete_permission(&self, args: &Value, lang: Lang) -> ApiResult {
        let kind = self.kind_of(args, lang)?;
        let app = match &args["app"] {
            Value::Null => None,
            Value::String(app) if perms::app_id_ok(app) => Some(app.as_str()),
            _ => return Err(bad(lang)),
        };
        let answer = json!({ "table": kind.table, "id": kind.id, "app": app });
        if self.ctx.dry_run {
            return Ok(answer);
        }
        let bus = self.bus().map_err(|e| store_error(e, lang))?;
        perms::delete(&bus, &self.targets.store, kind.table, kind.id, app)
            .map_err(|e| store_error(e, lang))?;
        self.changed(
            "permissions",
            json!({ "table": kind.table, "id": kind.id, "app": app }),
        );
        Ok(answer)
    }

    // ------------------------------------------------------------------------------------------------- the app

    fn app_info(
        &self,
        args: &Value,
        lang: Lang,
    ) -> Result<(gio::DesktopAppInfo, String), ApiError> {
        let id = str_arg(args, "id", lang)?;
        if !app::desktop_id_ok(id) {
            return Err(bad(lang));
        }
        let info = gio::DesktopAppInfo::new(id)
            .ok_or_else(|| ApiError::tr("not_found", lang, "privacy.appNotFound", NO_PARAMS))?;
        Ok((info, id.to_owned()))
    }

    fn app(&self, args: &Value, lang: Lang) -> ApiResult {
        let (info, id) = self.app_info(args, lang)?;
        let sizes = args["sizes"] != false;
        let mut out = app::describe(&info, &id, lang, sizes, &self.targets.snapd);
        let portal = out["portalId"].as_str().map(str::to_owned);
        out["permissions"] = json!([]);
        out["background"] = Value::Null;
        out["permissionsAvailable"] = json!(false);
        if let Some(portal) = &portal
            && let Ok(bus) = self.bus()
        {
            let mut entries = Vec::new();
            let mut available = true;
            for kind in perms::KINDS {
                match perms::lookup(&bus, &self.targets.store, kind.table, kind.id) {
                    Ok(apps) => entries.push((kind, apps)),
                    Err(_) => {
                        available = false;
                        break;
                    }
                }
            }
            if available {
                out["permissions"] = json!(perms::app_rows(&entries, portal));
                let background = entries
                    .iter()
                    .find(|(k, _)| k.key == "background")
                    .and_then(|(_, apps)| apps.iter().find(|(a, _)| a == portal))
                    .map(|(_, v)| v.as_slice());
                let kind = perms::kind("background", "background").expect("a known kind");
                out["background"] = json!({ "set": background.is_some(), "allowed": perms::allowed(kind, background) });
                out["permissionsAvailable"] = json!(true);
            }
        }
        let bare = id.trim_end_matches(".desktop").to_owned();
        let candidates: Vec<String> = portal.into_iter().chain(std::iter::once(bare)).collect();
        out["globalShortcuts"] = shortcuts::registered(&candidates)
            .and_then(|app| {
                shortcuts::read(&app)
                    .map(|list| json!({ "appId": app, "shortcuts": shortcuts::to_json(&list) }))
            })
            .unwrap_or(Value::Null);
        Ok(out)
    }

    fn clear_cache(&self, args: &Value, lang: Lang) -> ApiResult {
        let (info, id) = self.app_info(args, lang)?;
        let app::Kind::Flatpak(fid) = app::Kind::of(&info) else {
            return Err(ApiError::tr(
                "unsupported",
                lang,
                "privacy.noCache",
                NO_PARAMS,
            ));
        };
        let Some((_, cache)) =
            flatpak::data_dirs(&glib::home_dir(), &fid).filter(|(_, c)| c.is_dir())
        else {
            return Err(ApiError::tr(
                "unsupported",
                lang,
                "privacy.noCache",
                NO_PARAMS,
            ));
        };
        if self.ctx.dry_run {
            return Ok(json!({ "id": id, "freed": 0 }));
        }
        let freed = flatpak::clear(&cache)
            .map_err(|_| ApiError::tr("failed", lang, "privacy.cacheFailed", NO_PARAMS))?;
        self.changed("app", json!({ "app": id }));
        Ok(json!({ "id": id, "freed": freed }))
    }

    fn reset_handlers(&self, args: &Value, lang: Lang) -> ApiResult {
        let types = if args["all"] == true && args["id"].is_null() {
            let text = std::fs::read_to_string(glib::user_config_dir().join("mimeapps.list"))
                .unwrap_or_default();
            handlers::user_types(&text)
        } else {
            handlers::supported(&self.app_info(args, lang)?.0)
        };
        if !self.ctx.dry_run {
            // apps/defaults.rs's lock: no apps.setDefault* write in between
            let _lock = crate::apps::ConfigLock::acquire(&glib::user_config_dir())
                .map_err(|_| ApiError::tr("failed", lang, "privacy.handlerFailed", NO_PARAMS))?;
            handlers::reset(&types);
            self.changed("handlers", json!({ "app": args["id"] }));
        }
        Ok(json!({ "types": types }))
    }

    fn remove_handler(&self, args: &Value, lang: Lang) -> ApiResult {
        let (info, id) = self.app_info(args, lang)?;
        let kind = str_arg(args, "type", lang)?;
        let kind = gio::content_type_from_mime_type(kind)
            .map(|c| c.to_string())
            .unwrap_or_else(|| kind.to_owned());
        if !handlers::type_ok(&kind) || !handlers::supported(&info).contains(&kind) {
            return Err(bad(lang));
        }
        if self.ctx.dry_run {
            return Ok(json!({ "id": id, "type": kind }));
        }
        use gio::prelude::AppInfoExt;
        let lock = crate::apps::ConfigLock::acquire(&glib::user_config_dir())
            .map_err(|_| ApiError::tr("failed", lang, "privacy.handlerFailed", NO_PARAMS))?;
        info.remove_supports_type(&kind)
            .map_err(|_| ApiError::tr("failed", lang, "privacy.handlerFailed", NO_PARAMS))?;
        drop(lock);
        self.changed("handlers", json!({ "app": id }));
        Ok(json!({ "id": id, "type": kind }))
    }

    fn snap_connect(&self, args: &Value, lang: Lang) -> ApiResult {
        let (info, id) = self.app_info(args, lang)?;
        let app::Kind::Snap(snap) = app::Kind::of(&info) else {
            return Err(bad(lang));
        };
        let plug = str_arg(args, "plug", lang)?;
        let connect = args["connect"].as_bool().ok_or_else(|| bad(lang))?;
        if !snapd::port_ok(plug) {
            return Err(bad(lang));
        }
        let socket = &self.targets.snapd;
        let wait = Duration::from_secs(15);
        let slot = match &args["slot"] {
            Value::Object(s) => {
                let (slot_snap, slot) = (
                    s.get("snap").and_then(Value::as_str),
                    s.get("slot").and_then(Value::as_str),
                );
                match (slot_snap, slot) {
                    (Some(a), Some(b)) if snapd::name_ok(a) && snapd::port_ok(b) => {
                        (a.to_owned(), b.to_owned())
                    }
                    _ => return Err(bad(lang)),
                }
            }
            Value::Null => {
                // no slot named: the one it is connected to, or its only candidate
                let all = snapd::request(socket, "GET", "/v2/connections?select=all", None, wait)
                    .map_err(|e| snap_error(e, lang))?;
                let plugs = snapd::plugs(&all["result"], &snap);
                let entry = plugs
                    .iter()
                    .find(|p| p["plug"] == plug)
                    .ok_or_else(|| bad(lang))?;
                let only = entry["slots"]
                    .as_array()
                    .filter(|s| s.len() == 1)
                    .map(|s| &s[0]);
                let pick = if connect {
                    only.cloned().unwrap_or(Value::Null)
                } else {
                    entry["slot"].clone()
                };
                match (pick["snap"].as_str(), pick["slot"].as_str()) {
                    (Some(a), Some(b)) => (a.to_owned(), b.to_owned()),
                    _ => return Err(bad(lang)),
                }
            }
            _ => return Err(bad(lang)),
        };
        let answer = json!({ "id": id, "plug": plug, "slot": { "snap": slot.0, "slot": slot.1 }, "connected": connect });
        if self.ctx.dry_run {
            return Ok(answer);
        }
        let body = snapd::connect_body(connect, &snap, plug, &slot.0, &slot.1);
        let reply = snapd::request(
            socket,
            "POST",
            "/v2/interfaces",
            Some(&body),
            Duration::from_secs(600),
        )
        .map_err(|e| snap_error(e, lang))?;
        if let Some(change) = reply["change"].as_str() {
            snapd::wait_change(socket, change, Duration::from_secs(120))
                .map_err(|e| snap_error(e, lang))?;
        }
        self.changed("app", json!({ "app": id }));
        Ok(answer)
    }

    fn set_shortcuts(&self, args: &Value, lang: Lang) -> ApiResult {
        let (info, id) = self.app_info(args, lang)?;
        let clear = args["clear"] == true;
        let mut changes = Vec::new();
        for entry in args["shortcuts"].as_array().into_iter().flatten() {
            let action = entry["id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 255)
                .ok_or_else(|| bad(lang))?;
            let accels: Vec<String> = entry["accels"]
                .as_array()
                .ok_or_else(|| bad(lang))?
                .iter()
                .map(|a| {
                    a.as_str()
                        .filter(|a| shortcuts::accel_ok(a))
                        .map(str::to_owned)
                })
                .collect::<Option<_>>()
                .ok_or_else(|| bad(lang))?;
            changes.push((action.to_owned(), accels));
        }
        if !clear && changes.is_empty() {
            return Err(bad(lang));
        }
        let portal = app::Kind::of(&info).portal_id();
        let bare = id.trim_end_matches(".desktop").to_owned();
        let candidates: Vec<String> = portal.into_iter().chain(std::iter::once(bare)).collect();
        let unavailable = || {
            ApiError::tr(
                "unsupported",
                lang,
                "privacy.shortcutsUnavailable",
                NO_PARAMS,
            )
        };
        let app_id = shortcuts::registered(&candidates).ok_or_else(unavailable)?;
        let current = shortcuts::read(&app_id).ok_or_else(unavailable)?;
        if changes
            .iter()
            .any(|(action, _)| !current.iter().any(|s| s.id == *action))
        {
            return Err(bad(lang));
        }
        let call = shortcuts::rebind_args(&app_id, &current, &changes, clear);
        if self.ctx.dry_run {
            return Ok(json!({ "id": id, "appId": app_id }));
        }
        let bus = self.bus().map_err(|_| unavailable())?;
        shortcuts::rebind(&bus, &self.targets.rebind, &call).map_err(|e| {
            let mut error = ApiError::tr("failed", lang, "privacy.shortcutsFailed", NO_PARAMS);
            error.data = Some(json!({ "error": e.chars().take(300).collect::<String>() }));
            error
        })?;
        self.changed("app", json!({ "app": id }));
        Ok(json!({ "id": id, "appId": app_id }))
    }

    fn set_autostart(&self, args: &Value, lang: Lang) -> ApiResult {
        let (info, id) = self.app_info(args, lang)?;
        let on = args["on"].as_bool().ok_or_else(|| bad(lang))?;
        let user = glib::user_config_dir().join("autostart");
        let failed = || ApiError::tr("failed", lang, "privacy.autostartFailed", NO_PARAMS);
        let (base, generated) = app::autostart_base(&info, &id, &user).ok_or_else(failed)?;
        if app::starts(&base) == on && !generated {
            return Ok(json!({ "id": id, "enabled": on }));
        }
        if !on && generated {
            // nothing starts it: nothing to turn off
            return Ok(json!({ "id": id, "enabled": false }));
        }
        if self.ctx.dry_run {
            return Ok(json!({ "id": id, "enabled": on }));
        }
        app::write_autostart(&user, &id, &app::autostart_text(&base, on, generated))
            .map_err(|_| failed())?;
        self.changed("app", json!({ "app": id }));
        Ok(json!({ "id": id, "enabled": on }))
    }

    // ------------------------------------------------------------- Privacy en beveiliging (slice 3 of batch F)

    /// The bus of the system daemons (whoopsie-preferences, fwupd): the system bus, or the tests' private bus.
    fn system_bus(&self) -> Result<gio::DBusConnection, String> {
        match &self.targets.system {
            None => crate::services::system::common::system(),
            Some(address) => gio::DBusConnection::for_address_sync(
                address,
                gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                    | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                None,
                gio::Cancellable::NONE,
            )
            .map_err(|e| e.to_string()),
        }
    }

    /// Every installed sandboxed app (Flatpak, Snap) with the id the portals know it by, so the permission pages
    /// name the PermissionStore's apps (GNOME: the app id's desktop file).
    fn sandboxed(&self, lang: Lang) -> Value {
        use gio::prelude::{AppInfoExt, Cast};
        let mut apps: Vec<Value> = gio::AppInfo::all()
            .into_iter()
            .filter_map(|info| info.downcast::<gio::DesktopAppInfo>().ok())
            .filter_map(|info| {
                let kind = app::Kind::of(&info);
                let portal = kind.portal_id()?;
                let id = info.id()?.to_string();
                Some(json!({
                    "id": id,
                    "portalId": portal,
                    "kind": if matches!(kind, app::Kind::Snap(_)) { "snap" } else { "flatpak" },
                    "name": crate::services::notifications::app_settings::name_in(&info, lang),
                    "icon": app::icon_string(info.icon()),
                }))
            })
            .take(500)
            .collect();
        apps.sort_by_key(|a| a["name"].as_str().unwrap_or("").to_lowercase());
        json!({ "apps": apps })
    }

    fn reports_get(&self) -> Value {
        match self.system_bus() {
            Ok(bus) => reports::get(&bus, &self.targets.whoopsie),
            Err(_) => json!({ "available": false }),
        }
    }

    fn reports_set(&self, args: &Value, lang: Lang) -> ApiResult {
        let mode = str_arg(args, "mode", lang)?;
        let (report, automatic) = reports::flags(mode).ok_or_else(|| bad(lang))?;
        let unavailable =
            || ApiError::tr("unsupported", lang, "privacy.reportsUnavailable", NO_PARAMS);
        let bus = self.system_bus().map_err(|_| unavailable())?;
        if reports::get(&bus, &self.targets.whoopsie)["available"] != true {
            return Err(unavailable());
        }
        let answer = json!({ "mode": mode, "reportCrashes": report, "automatic": automatic });
        if self.ctx.dry_run {
            return Ok(answer);
        }
        reports::set(&bus, &self.targets.whoopsie, report, automatic, lang)?;
        self.changed("reports", json!({ "mode": mode }));
        Ok(answer)
    }

    /// ubuntu-insights, or the refusal when it is not installed.
    fn insights_tool(&self, lang: Lang) -> Result<PathBuf, ApiError> {
        self.targets
            .insights
            .clone()
            .or_else(|| crate::tools::system_tool("ubuntu-insights"))
            .filter(|p| p.is_file())
            .ok_or_else(|| {
                ApiError::tr(
                    "unsupported",
                    lang,
                    "privacy.insightsUnavailable",
                    NO_PARAMS,
                )
            })
    }

    fn insights_get(&self, lang: Lang) -> ApiResult {
        let Ok(tool) = self.insights_tool(lang) else {
            return Ok(json!({ "available": false, "sources": {} }));
        };
        let mut out = insights::consent(&tool);
        out["available"] = json!(true);
        Ok(out)
    }

    fn insights_set(&self, args: &Value, lang: Lang) -> ApiResult {
        let source = str_arg(args, "source", lang)?;
        let consent = args["consent"].as_bool().ok_or_else(|| bad(lang))?;
        if !insights::source_ok(source) {
            return Err(bad(lang));
        }
        let tool = self.insights_tool(lang)?;
        let answer = json!({ "source": source, "consent": consent });
        if self.ctx.dry_run {
            return Ok(answer);
        }
        insights::set_consent(&tool, source, consent).map_err(|text| {
            let mut error = ApiError::tr("failed", lang, "privacy.consentFailed", NO_PARAMS);
            error.data = Some(json!({ "error": text }));
            error
        })?;
        self.changed("insights", json!({ "source": source }));
        Ok(answer)
    }

    fn insights_report(&self, args: &Value, lang: Lang) -> ApiResult {
        use std::sync::atomic::Ordering;
        if args["cancel"] == true {
            let running = *self.reporting.lock().unwrap_or_else(|e| e.into_inner());
            if running {
                self.cancel_report.store(true, Ordering::Release);
            }
            return Ok(json!({ "cancelled": running }));
        }
        let source = match &args["source"] {
            Value::Null => insights::REPORT_SOURCE,
            Value::String(s) if s == insights::REPORT_SOURCE => insights::REPORT_SOURCE,
            _ => return Err(bad(lang)),
        };
        let tool = self.insights_tool(lang)?;
        {
            let mut running = self.reporting.lock().unwrap_or_else(|e| e.into_inner());
            if *running {
                return Err(ApiError::tr("busy", lang, "privacy.reportBusy", NO_PARAMS));
            }
            *running = true;
            self.cancel_report.store(false, Ordering::Release);
        }
        let result = insights::report(&tool, &self.cancel_report);
        *self.reporting.lock().unwrap_or_else(|e| e.into_inner()) = false;
        match result {
            Ok(report) => Ok(json!({ "source": source, "report": report })),
            Err(insights::ReportError::Cancelled) => Err(ApiError::tr(
                "cancelled",
                lang,
                "privacy.reportCancelled",
                NO_PARAMS,
            )),
            Err(insights::ReportError::Failed(text)) => {
                let mut error = ApiError::tr("failed", lang, "privacy.insightsFailed", NO_PARAMS);
                error.data = Some(json!({ "error": text }));
                Err(error)
            }
        }
    }

    fn security(&self) -> Value {
        let mut out = match self.system_bus() {
            Ok(bus) => security::read(&bus, &self.targets.fwupd),
            Err(_) => json!({ "available": false }),
        };
        out["securityCenter"] = json!(self.security_center().is_some());
        out
    }

    /// The installed Security Center's desktop id.
    fn security_center(&self) -> Option<&'static str> {
        security::SECURITY_CENTER
            .iter()
            .copied()
            .find(|id| gio::DesktopAppInfo::new(id).is_some())
    }

    /// Ubuntu's Security Center as GNOME Shell's child (launchBridge.js `launch.app`); while the bridge cannot start
    /// it (an older extension, no GNOME Shell) the host starts it as GNOME Settings does, through apps.launch.
    fn open_security_center(&self, lang: Lang) -> ApiResult {
        let id = self.security_center().ok_or_else(|| {
            ApiError::tr(
                "not_found",
                lang,
                "privacy.securityCenterMissing",
                NO_PARAMS,
            )
        })?;
        if self.ctx.dry_run {
            return Ok(json!({ "id": id, "dryRun": true }));
        }
        if let Ok(Ok(_)) = crate::gnome::call_api("launch.app", &json!({ "id": id })) {
            return Ok(json!({ "id": id, "via": "shell" }));
        }
        self.ctx.call("apps.launch", &json!({ "id": id }))?;
        Ok(json!({ "id": id, "via": "host" }))
    }
}
