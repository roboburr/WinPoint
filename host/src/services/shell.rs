//! GNOME shell coordination. All application windows run in independent native Rust hosts.
mod backlog;
mod clipboard;
mod filemanager;
mod params;
mod restart;
#[cfg(test)]
mod tests;
use super::{
    ServiceContext,
    system::common::{string, truth},
};
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
const APPS: &[&str] = &[
    "settings",
    "taskmgr",
    "explorer",
    "alles",
    "disks",
    "defaultapps",
    "installer",
];
type Shutdown = dyn Fn(bool) + Send + Sync;
#[cfg(test)]
type Launcher = dyn Fn(&str, &Value, Option<&str>) -> Result<u32, String> + Send + Sync;
pub(crate) struct NativeCore {
    ctx: Arc<ServiceContext>,
    request_shutdown: Arc<Shutdown>,
    stopped: AtomicBool,
    runtime: Mutex<Value>,
    backlog: Arc<backlog::Backlog>,
    filemanager: Arc<filemanager::FileManager>,
    clipboard: clipboard::Clipboard,
    restart: Mutex<restart::State>,
    #[cfg(test)]
    launcher: Mutex<Option<Arc<Launcher>>>,
}
impl NativeCore {
    pub(crate) fn new(
        ctx: Arc<ServiceContext>,
        request_shutdown: Arc<Shutdown>,
    ) -> Result<Arc<Self>, String> {
        Self::build(ctx, request_shutdown, true)
    }
    fn build(
        ctx: Arc<ServiceContext>,
        request_shutdown: Arc<Shutdown>,
        start: bool,
    ) -> Result<Arc<Self>, String> {
        let core = Arc::new_cyclic(|weak| Self {
            backlog: backlog::Backlog::new(ctx.clone()),
            filemanager: filemanager::FileManager::new(ctx.clone(), weak.clone()),
            clipboard: clipboard::Clipboard::new(&ctx),
            runtime: Mutex::new(json!({"runId":crate::events::nonce()})),
            ctx,
            request_shutdown,
            stopped: AtomicBool::new(false),
            restart: Mutex::new(restart::State::default()),
            #[cfg(test)]
            launcher: Mutex::new(None),
        });
        if start {
            core.filemanager.start();
            core.watch_restart_all();
        }
        Ok(core)
    }
    /// The GTK host supplies measured loop lag / renderer state without changing the public info contract.
    pub(crate) fn set_runtime_info(&self, extra: Value) {
        if let (Some(target), Some(extra)) = (
            self.runtime
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_object_mut(),
            extra.as_object(),
        ) {
            target.extend(extra.clone());
        }
    }
    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.filemanager.stop();
    }
    fn info(&self) -> Value {
        let mode = mode(&self.ctx);
        let services = [
            "shell",
            "settings",
            "apps",
            "windows",
            "system",
            "media",
            "clipboard",
            "notifications",
            "storage",
            "filemanager1",
            "disks",
            "files",
            "sysmon",
            "index",
            "tray",
            "display",
        ]
        .into_iter()
        .map(|name| (name.to_owned(), json!({"ok":true})))
        .collect::<serde_json::Map<_, _>>();
        let mut out = json!({"mode":mode,"compositor":"gnome","dryRun":self.ctx.dry_run,
            "version":env!("CARGO_PKG_VERSION"),"layerShell":mode=="layer","layerShellInstalled":false,
            "user":user(self.ctx.lang()),"apps":APPS,"services":services,
            "dev":false,"pid":std::process::id(),"gpu":gpu_info(),"mainLoopLagMs":null,"runId":null,"problem":null});
        if let (Some(out), Some(extra)) = (
            out.as_object_mut(),
            self.runtime
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_object(),
        ) {
            out.extend(extra.clone());
        }
        out
    }
    fn command(&self, args: &Value) -> ApiResult {
        let raw = &args["command"];
        let command = raw.as_str().filter(|s| valid_command(s)).ok_or_else(|| {
            let mut e = ApiError::tr(
                "unknown_command",
                self.ctx.lang(),
                "shell.unknownCommand",
                crate::i18n::NO_PARAMS,
            );
            e.data = Some(json!({"command":raw}));
            e
        })?;
        let cargs = if truth(&args["args"]) {
            args["args"].clone()
        } else {
            json!({})
        };
        if !cargs.is_object() {
            return Err(bad_args(self.ctx.lang()));
        }
        let delivered = self.ctx.subscriber_count();
        self.ctx
            .emit("shell.command", json!({"command":command,"args":cargs}));
        Ok(json!({"delivered":delivered}))
    }
    pub(super) fn open(&self, args: &Value) -> ApiResult {
        self.open_with_token(
            args,
            args["activationToken"]
                .as_str()
                .filter(|t| t.chars().count() < 512),
        )
    }
    fn open_with_token(&self, args: &Value, token: Option<&str>) -> ApiResult {
        let app = own_app(&args["app"]).ok_or_else(|| {
            let mut e = ApiError::tr(
                "unknown_app",
                self.ctx.lang(),
                "shell.unknownApp",
                crate::i18n::NO_PARAMS,
            );
            e.data = Some(json!({"app":args["app"]}));
            e
        })?;
        let mut params = clean_params(&args["params"], self.ctx.lang())?;
        if app == "explorer" {
            if let Some(path) = params["path"].as_str().filter(|s| s.starts_with('~')) {
                params["path"] = json!(expand_user(path));
            }
        }
        if self.ctx.dry_run {
            return Ok(json!({"ok":true,"dryRun":true}));
        }
        if self.stopped.load(Ordering::Relaxed) {
            return Err(ApiError::tr(
                "shutting_down",
                self.ctx.lang(),
                "shell.shuttingDown",
                crate::i18n::NO_PARAMS,
            ));
        }
        let pid = self.launch(app, &params, token).map_err(|e| {
            let mut err = ApiError::tr(
                "spawn_failed",
                self.ctx.lang(),
                "shell.openFailed",
                crate::i18n::NO_PARAMS,
            );
            err.data = Some(json!(e));
            err
        })?;
        // Unlike the reference single-window table, native Explorer allows independent tear-out windows.
        // Do not broadcast app.navigate to every Explorer when opening another one.
        Ok(json!({"started":true,"pid":pid}))
    }
    fn launch(&self, app: &str, params: &Value, token: Option<&str>) -> Result<u32, String> {
        #[cfg(test)]
        if let Some(launch) = self
            .launcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return launch(app, params, token);
        }
        let mut command = crate::launch::command(app, Some(params))?;
        command.env_remove("WIN11_NATIVE_INSTANCE");
        if let Some(token) = token.filter(|t| !t.is_empty()) {
            command
                .env("XDG_ACTIVATION_TOKEN", token)
                .env("DESKTOP_STARTUP_ID", token);
        } else {
            crate::launch::prepare_activation(&mut command, app);
        }
        crate::process_scope::spawn_pid(command, app, None)
    }
    pub(crate) fn call(self: &Arc<Self>, method: &str, args: &Value) -> Option<ApiResult> {
        if method.starts_with("clipboard.") {
            return self.clipboard.call(method, args);
        }
        Some(match method {
            "shell.info" => Ok(self.info()),
            "shell.command" => self.command(args),
            "shell.openApp" => self.open(args),
            "explorer.ready" => self.backlog.ready(args),
            "shell.restartAll" => self.restart_all(args),
            "shell.restart" if self.ctx.dry_run => Ok(json!({"ok":true,"dryRun":true})),
            "shell.restart" | "shell.quit" => {
                let restart = method == "shell.restart";
                let callback = self.request_shutdown.clone();
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(300));
                    callback(restart);
                });
                Ok(json!({}))
            }
            _ => return None,
        })
    }
}
fn mode(ctx: &ServiceContext) -> &'static str {
    if ctx.kind == "panel" { "layer" } else { "app" }
}
fn bad_args(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "shell.badArgs", crate::i18n::NO_PARAMS)
}
fn own_app(v: &Value) -> Option<&'static str> {
    let s = string(v).trim().to_lowercase();
    let s = s.strip_suffix(".desktop").unwrap_or(&s);
    let s = s.strip_prefix("win11shell-").unwrap_or(s);
    APPS.iter().copied().find(|a| *a == s)
}
fn clean_params(v: &Value, lang: Lang) -> ApiResult {
    if v.is_null() {
        return Ok(json!({}));
    }
    if !v.is_object() {
        return Err(bad_args(lang));
    }
    if v.to_string().chars().count() > 64 * 1024 {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "shell.tooMuchData",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(v.clone())
}
fn valid_command(raw: &str) -> bool {
    let s = raw.strip_suffix('\n').unwrap_or(raw);
    let bytes = s.as_bytes();
    (1..=41).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
}
fn expand_user(s: &str) -> String {
    if s == "~" {
        return glib::home_dir().to_string_lossy().into_owned();
    }
    if let Some(tail) = s.strip_prefix("~/") {
        return glib::home_dir().join(tail).to_string_lossy().into_owned();
    }
    let (user, tail) = s[1..].split_once('/').unwrap_or((&s[1..], ""));
    let Ok(user) = std::ffi::CString::new(user) else {
        return s.into();
    };
    let mut buf = vec![0u8; 16384];
    let mut pw = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut out = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwnam_r(
            user.as_ptr(),
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut out,
        )
    };
    if rc == 0 && !out.is_null() {
        let home = unsafe { std::ffi::CStr::from_ptr(pw.pw_dir) }.to_string_lossy();
        return if tail.is_empty() {
            home.into_owned()
        } else {
            PathBuf::from(home.as_ref())
                .join(tail)
                .to_string_lossy()
                .into_owned()
        };
    }
    s.into()
}
fn user(lang: Lang) -> Value {
    let mut buf = vec![0u8; 16384];
    let mut pw = unsafe { std::mem::zeroed::<libc::passwd>() };
    let mut out = std::ptr::null_mut();
    let rc = unsafe {
        libc::getpwuid_r(
            libc::getuid(),
            &mut pw,
            buf.as_mut_ptr().cast(),
            buf.len(),
            &mut out,
        )
    };
    let (name, real) = if rc == 0 && !out.is_null() {
        let name = unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }
            .to_string_lossy()
            .into_owned();
        let real = if pw.pw_gecos.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(pw.pw_gecos) }
                .to_string_lossy()
                .split(',')
                .next()
                .unwrap_or("")
                .trim()
                .to_owned()
        };
        (name, real)
    } else {
        (
            std::env::var("USER").unwrap_or_else(|_| crate::i18n::tr(lang, "shell.userFallback")),
            String::new(),
        )
    };
    json!({"name":name,"realName":if real.is_empty(){&name}else{&real}})
}
/// `shell.info.gpu`: what the WebKit hosts reported about their GPU switches (WIN11SHELL_GPU, `no-gpu`). The
/// native programs leave rendering to GTK 4 (GSK_RENDERER), so the field only keeps the answer's shape.
fn gpu_info() -> Value {
    json!({"mode":"native"})
}
