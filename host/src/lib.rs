#![recursion_limit = "512"]
use gtk::{gdk, gio, glib, prelude::*};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    time::Duration,
};
mod app_params;
mod apps;
mod desktop_entries;
mod events;
mod files;
mod gnome;
mod i18n;
mod images;
mod launch;
mod launch_context;
mod picker;
mod portal;
mod process_scope;
mod services;
mod settings;
mod settings_runtime;
mod transfers;
mod wallpaper;
mod window;
mod window_state;

mod backend;
mod runtime;

fn ui_root() -> Result<PathBuf, String> {
    let candidates = [
        std::env::var_os("WIN11_UI_DIR").map(PathBuf::from),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("../share/win11-gnome/ui"))),
        Some(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../vendor/ui")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.join("index.html").is_file())
        .ok_or("De oorspronkelijke interface ontbreekt".into())
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => "application/octet-stream",
    }
}

fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name, value).expect("constant HTTP header")
}
fn respond(request: tiny_http::Request, status: u16, data: Vec<u8>, mime: &str) {
    let response = tiny_http::Response::from_data(data)
        .with_status_code(status)
        .with_header(header("Content-Type", mime))
        .with_header(header("Cache-Control", "no-store"))
        .with_header(header("X-Content-Type-Options", "nosniff"));
    let _ = request.respond(response);
}

fn respond_image(
    request: tiny_http::Request,
    result: Result<images::ImageResponse, images::ImageError>,
) {
    match result {
        Ok(result) => {
            let length = result
                .headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case("Content-Length"))
                .and_then(|(_, v)| v.parse::<usize>().ok());
            let mut headers: Vec<_> = result
                .headers
                .iter()
                .filter_map(|(key, value)| {
                    tiny_http::Header::from_bytes(key.as_str(), value.as_str()).ok()
                })
                .collect();
            headers.extend([
                header("X-Content-Type-Options", "nosniff"),
                header("Referrer-Policy", "no-referrer"),
                header("X-Frame-Options", "SAMEORIGIN"),
            ]);
            if result.status == 304 {
                // tiny_http otherwise selects chunked transfer for an unknown empty length,
                // then correctly suppresses the 304 body and leaves that chunk stream unterminated.
                let mut writer = request.into_writer();
                let mut head = String::from("HTTP/1.1 304 Not Modified\r\nConnection: close\r\n");
                for h in headers {
                    head.push_str(&h.to_string());
                    head.push_str("\r\n");
                }
                head.push_str("\r\n");
                let _ = writer
                    .write_all(head.as_bytes())
                    .and_then(|_| writer.flush());
                return;
            }
            let body: Box<dyn Read + Send> = match result.body {
                images::ImageBody::Bytes(bytes) => Box::new(std::io::Cursor::new(bytes)),
                images::ImageBody::File(file) => Box::new(file),
            };
            let _ = request.respond(tiny_http::Response::new(
                tiny_http::StatusCode(result.status),
                headers,
                body,
                length,
                None,
            ));
        }
        Err(error) => respond(
            request,
            error.status,
            format!("{}\n", error.message).into_bytes(),
            "text/plain; charset=utf-8",
        ),
    }
}

const NATIVE_BRIDGE: &str = r#"
window.__win11shellHost = {backdrop: 'none', runtime: 'rust'};
"#;
fn dev_report(window: &gtk::ApplicationWindow, event: &str, data: &Value) {
    if let Some(path) = std::env::var_os("WIN11_DEV_REPORT") {
        let state = window
            .window()
            .map(|window| window.state())
            .unwrap_or(gdk::WindowState::empty());
        let line = json!({"event":event,"data":data,"maximized":state.contains(gdk::WindowState::MAXIMIZED),
            "minimized":state.contains(gdk::WindowState::ICONIFIED),"focused":window.is_active(),"windowId":window.widget_name().as_str()});
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
        {
            // Multiple standalone hosts may share a development report. One O_APPEND write
            // keeps their JSON lines from interleaving at individual formatting fragments.
            let _ = file.write_all(format!("{line}\n").as_bytes());
        }
    }
}

pub(crate) struct UiRequest {
    kind: UiRequestKind,
    payload: Value,
    ack: std::sync::mpsc::Sender<Result<Value, String>>,
}
pub(crate) enum UiRequestKind {
    FilePicker,
    Lifecycle,
}

static DESKTOP_ENTRIES: AtomicBool = AtomicBool::new(false);

/// Makes this process the single writer of the own desktop entries' `Name=`/`Comment=` (contracts C6). Only
/// `win11-panel` calls it (bin/panel.rs), before [`run`]; the writer is its `--http-only` backend of win11-shell
/// ([`watch_entries`]), since win11-panel itself execs win11-shell.
pub fn enable_desktop_entries() {
    DESKTOP_ENTRIES.store(true, Ordering::Relaxed);
}

/// Follows `settings.language` in a GUI process: native window titles (C4). The settings runtime emits
/// `settings.changed` for every change, from another host's invalidation datagram or from its own 150 ms poll;
/// this keeps only the language switches.
///
/// The language is read from the settings file when it is applied, never taken from the event: the datagram
/// thread and the poll thread dispatch independently, so two quick switches can arrive out of order, and the
/// file is the only state that cannot be stale. One idle callback at a time; an event during it queues the next.
fn watch_language(runtime: &runtime::Runtime) {
    let ctx = runtime.ctx.clone();
    let initial = ctx.lang();
    // Only the main thread (the idle callback) reads and writes `applied`.
    let applied = Arc::new(AtomicU8::new(initial.as_u8()));
    let queued = Arc::new(AtomicBool::new(false));
    runtime.files.events.on(
        "settings.changed",
        Arc::new(move |_| {
            if queued.swap(true, Ordering::AcqRel) {
                return;
            }
            let (ctx, applied, queued) = (ctx.clone(), applied.clone(), queued.clone());
            // GTK objects live on the main thread; the event arrives on a runtime thread.
            glib::idle_add_once(move || {
                queued.store(false, Ordering::Release);
                let lang = ctx.lang();
                if applied.swap(lang.as_u8(), Ordering::AcqRel) == lang.as_u8() {
                    return;
                }
                window::language_changed(lang);
            });
        }),
    );
}

/// C6 under the native shell: `win11-panel --http-only` (win11-shell's backend) writes the own desktop entries'
/// `Name=`/`Comment=` at start and, debounced, after a language switch. No GLib main loop runs in this mode;
/// [`desktop_entries::schedule`] runs on its own thread and reads the language from the settings file when it
/// runs, so two quick switches seen out of order still end in the language of the file.
fn watch_entries(runtime: &runtime::Runtime) {
    let ctx = runtime.ctx.clone();
    let applied = Arc::new(AtomicU8::new(ctx.lang().as_u8()));
    {
        let ctx = ctx.clone();
        desktop_entries::schedule(Duration::ZERO, move || ctx.lang());
    }
    runtime.files.events.on(
        "settings.changed",
        Arc::new(move |_| {
            let lang = ctx.lang();
            if applied.swap(lang.as_u8(), Ordering::AcqRel) == lang.as_u8() {
                return;
            }
            let ctx = ctx.clone();
            desktop_entries::schedule(Duration::from_secs(1), move || ctx.lang());
        }),
    );
}

pub fn run(kind: &'static str) {
    // Before any thread exists: the Explorer restart lock a re-exec handed over (shell.restartAll).
    let handoff = take_handoff();
    // The handed-over lock is close-on-exec again: an exec into win11-shell releases it.
    native_shell_switch(kind);
    match process_scope::ensure_native(kind) {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("Win11 GNOME-procesisolatie: {error}");
            std::process::exit(1);
        }
    }
    reap_inherited_children();
    if let Err(err) = run_inner(kind, handoff) {
        eprintln!("Win11 voor GNOME: {err}");
        std::process::exit(1);
    }
}

/// D-G70: the native GTK 4 shell (native/shell, `win11-shell`) is the shell: taskbar, flyouts, Start and the
/// desktop. win11-panel without `--http-only` execs `win11-shell` from its own directory with the same argv and
/// environment, so the pid, GNOME's launch scope and the Panel identity stay; win11-shell then starts
/// `win11-panel --http-only` as its backend (native/shell/src/backend.rs). There is no WebKit fallback: when
/// win11-shell is missing or the exec fails, win11-panel exits and the extension shows its fallback bar.
fn native_shell_switch(kind: &str) {
    if kind != "panel" || std::env::args().any(|arg| arg == "--http-only") {
        return;
    }
    // A binary replaced by rename (scripts/install.py) is named "<path> (deleted)": its directory holds the new build.
    let shell = std::env::current_exe()
        .map(exec_path)
        .unwrap_or_default()
        .with_file_name("win11-shell");
    use std::os::unix::process::CommandExt;
    let error = std::process::Command::new(&shell)
        .args(std::env::args_os().skip(1))
        .exec();
    eprintln!("Win11 voor GNOME: {}: {error}", shell.display());
    std::process::exit(1);
}

fn run_inner(kind: &'static str, handoff: Option<std::fs::File>) -> Result<(), String> {
    if std::env::args().any(|arg| arg == "--http-only") {
        return run_http_only(kind);
    }
    let id = match kind {
        "settings" => "io.github.win11shell.Settings",
        "taskmgr" => "io.github.win11shell.TaskManager",
        "alles" => "io.github.win11shell.Search",
        "disks" => "io.github.win11shell.Disks",
        "defaultapps" => "io.github.win11shell.DefaultApps",
        "installer" => "io.github.win11shell.Installer",
        _ => "io.github.win11shell.Explorer",
    };
    glib::set_prgname(Some(id));
    let program_class = std::ffi::CString::new(id).expect("static application ID");
    // GDK chooses the Wayland application ID during initialization.
    unsafe { gdk::ffi::gdk_set_program_class(program_class.as_ptr()) };
    gtk::init().map_err(|err| err.to_string())?;
    let flags = if kind == "alles" {
        gio::ApplicationFlags::HANDLES_COMMAND_LINE
    } else {
        gio::ApplicationFlags::NON_UNIQUE
    };
    let app = gtk::Application::new(Some(id), flags);
    if kind == "alles" {
        // Claim/forward the application identity before opening another backend or WebKit process.
        app.register(gio::Cancellable::NONE)
            .map_err(|err| format!("Start registreren: {err}"))?;
        if app.is_remote() {
            // Forward deep links before constructing any backend or index worker.
            let args: Vec<_> = std::env::args().collect();
            app.run_with_args(&args);
            return Ok(());
        }
    }
    let ui = ui_root()?;
    let (ui_sender, ui_receiver) = std::sync::mpsc::channel();
    let ui_receiver = std::rc::Rc::new(ui_receiver);
    let restart = Arc::new(AtomicBool::new(false));
    let requested = restart.clone();
    let lifecycle = ui_sender.clone();
    // D-G45 (GX1 m5): a restarting Explorer waits for its own file operations (defer_while_busy)
    let explorer_files: Arc<std::sync::OnceLock<Arc<files::Files>>> = Arc::default();
    let pending_restart: Arc<PendingRestart> = Arc::default();
    let (receiver_files, receiver_pending) = (explorer_files.clone(), pending_restart.clone());
    let runtime = runtime::Runtime::new(
        ui.clone(),
        kind,
        ui_sender,
        Arc::new(move |again| {
            if again && restart_all_plan().is_some() {
                // shell.restartAll: decided on the main thread, where the windows are
                let (requested, lifecycle) = (requested.clone(), lifecycle.clone());
                let restart: Box<dyn FnOnce() + Send> = Box::new(move || {
                    glib::MainContext::default()
                        .invoke(move || restart_on_main(kind, &requested, &lifecycle));
                });
                match receiver_files.get().filter(|_| kind == "explorer") {
                    Some(files) => defer_while_busy(files, &receiver_pending, restart),
                    None => restart(),
                }
                return;
            }
            if again {
                requested.store(true, Ordering::Relaxed);
            }
            let (ack, _) = std::sync::mpsc::channel();
            let _ = lifecycle.send(UiRequest {
                kind: UiRequestKind::Lifecycle,
                payload: json!({"restart":again}),
                ack,
            });
        }),
        true,
    )?;
    watch_language(&runtime);
    if kind == "explorer" {
        let _ = explorer_files.set(runtime.files.clone());
        watch_files_idle(&runtime.files, pending_restart);
    }
    let backend = runtime.backend.clone();
    let window_id = runtime.transfers.window_id.clone();
    let saved_window = window_state::WindowState::open(&runtime.ctx.config_dir, kind, id);
    // This Explorer has read its own session (window state): the next restarting Explorer may flush its own.
    drop(handoff);
    let (port, stop) = runtime.server()?;
    let instance = std::env::var_os("WIN11_NATIVE_INSTANCE").map(PathBuf::from);
    if let Some(path) = &instance {
        backend::write_instance(path, port, &backend.token)?;
    }
    let beat = Arc::downgrade(&runtime);
    glib::timeout_add_local(Duration::from_millis(500), move || {
        if let Some(runtime) = beat.upgrade() {
            runtime.beat();
            glib::ControlFlow::Continue
        } else {
            glib::ControlFlow::Break
        }
    });
    let mut url = url::Url::parse(&format!("http://127.0.0.1:{port}/ui/index.html"))
        .map_err(|e| e.to_string())?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("token", &backend.token);
        query.extend_pairs([("surface", "app"), ("app", kind)]);
        let mut args: Vec<_> = std::env::args()
            .skip(1)
            .filter(|arg| !matches!(arg.as_str(), "--dry-run" | "--http-only"))
            .collect();
        if args.first().map(String::as_str) == Some("--app") {
            args.drain(..args.len().min(2));
        }
        if args.first().map(String::as_str) == Some("--tab-ticket") {
            if let Some(ticket) = args.get(1) {
                query.append_pair("tabTicket", ticket);
            }
        } else if args.first().map(String::as_str) == Some("--params") {
            if let Some(raw) = args.get(1)
                && let Ok(params) = serde_json::from_str::<Value>(raw)
                && let Some(params) = params.as_object()
            {
                for (key, value) in params {
                    query.append_pair(
                        key,
                        &value
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| value.to_string()),
                    );
                }
            }
        } else if let Some(path) = args.first() {
            query.append_pair("path", path);
            if args.len() > 1 {
                query.append_pair(
                    "tabs",
                    &serde_json::to_string(&args[1..]).map_err(|error| error.to_string())?,
                );
            }
        }
    }
    let address = url.to_string();
    let navigation = std::rc::Rc::new(std::cell::RefCell::new(None));
    if kind == "alles" {
        let pending = navigation.clone();
        app.connect_command_line(move |app, command| {
            if command.is_remote() {
                let args = command.arguments();
                let params = args.windows(2).find_map(|pair| {
                    (pair[0] == "--params")
                        .then(|| pair[1].to_str())
                        .flatten()
                        .filter(|raw| raw.len() <= 64 * 1024)
                        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                        .filter(Value::is_object)
                });
                if let Some(params) = params {
                    *pending.borrow_mut() = Some(params);
                }
            }
            app.activate();
            0
        });
    }
    let flush_window = saved_window.clone();
    app.connect_activate(move |app| {
        window::build(
            app,
            id,
            kind,
            &address,
            &window_id,
            ui_receiver.clone(),
            navigation.clone(),
            saved_window.clone(),
        );
    });
    // The URL already contains parsed application parameters; don't let GTK parse them as options.
    app.run_with_args(&[id]);
    let restarting = restart.load(Ordering::Relaxed);
    let plan = restarting.then(take_restart_all).flatten();
    // Restarting Explorers flush and read the one window-state slot one at a time (design §5: each restores
    // its own tabs); the lock goes to the new image, which drops it after reading.
    let handoff = (plan.is_some() && kind == "explorer")
        .then(explorer_handoff)
        .flatten();
    if let Some(state) = flush_window {
        state.flush();
    }
    stop.store(true, Ordering::Relaxed);
    runtime.shutdown();
    if let Some(path) = instance {
        backend::remove_instance(&path);
    }
    if restarting {
        let error = reexec(kind, plan.as_ref(), handoff);
        return Err(format!("Herstarten: {error}"));
    }
    Ok(())
}

/// What a `shell.restartAll` re-exec needs (design §5). services/shell/restart.rs sets it right before
/// `request_shutdown(true)`; a plain `shell.restart` leaves it empty and re-execs with its own argv.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestartAll {
    /// this process called `shell.restartAll` (Settings reopens on its update page)
    pub(crate) origin: bool,
    /// a fresh activation token for the new window (app kinds), so it maps focused like a launch
    pub(crate) token: Option<String>,
}
static RESTART_ALL: std::sync::Mutex<Option<RestartAll>> = std::sync::Mutex::new(None);
/// What restart.rs runs when the main thread does not restart this process after all ([`restart_on_main`]).
type Dropped = Box<dyn FnOnce() + Send>;
static RESTART_DROPPED: std::sync::Mutex<Option<Dropped>> = std::sync::Mutex::new(None);
pub(crate) fn prepare_restart_all(plan: RestartAll, dropped: Option<Dropped>) {
    *RESTART_DROPPED.lock().unwrap_or_else(|e| e.into_inner()) = dropped;
    *RESTART_ALL.lock().unwrap_or_else(|e| e.into_inner()) = Some(plan);
}
pub(crate) fn restart_all_plan() -> Option<RestartAll> {
    RESTART_ALL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
pub(crate) fn take_restart_all() -> Option<RestartAll> {
    RESTART_DROPPED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    RESTART_ALL.lock().unwrap_or_else(|e| e.into_inner()).take()
}
/// The plan is not carried out (an app host without a window): forget it and tell restart.rs, which logs it,
/// announces it and takes a later restart again. Whether there was a plan.
pub(crate) fn drop_restart_all() -> bool {
    let dropped = RESTART_DROPPED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .take();
    let plan = RESTART_ALL.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(dropped) = dropped {
        dropped();
    }
    plan.is_some()
}
/// Tests that set or take the process-wide plan run one at a time.
#[cfg(test)]
pub(crate) static RESTART_TESTS: std::sync::Mutex<()> = std::sync::Mutex::new(());
// WU-FIX (D-G53, R3-NATIVE review n2): a wait that only this file sees (Explorer's own file operations,
// defer_while_busy) is announced by services/shell/restart.rs like its own waits (`shell.restartDeferred
// {wait:'files'}`, then `shell.restartResumed` when the exec runs after all). restart.rs sets the announcer with the
// plan; `true`: the exec waits, `false`: it runs now.
pub(crate) type RestartWaiting = Arc<dyn Fn(bool) + Send + Sync>;
static RESTART_WAITING: std::sync::Mutex<Option<RestartWaiting>> = std::sync::Mutex::new(None);
pub(crate) fn on_restart_wait(waiting: Option<RestartWaiting>) {
    *RESTART_WAITING.lock().unwrap_or_else(|e| e.into_inner()) = waiting;
}
fn note_restart_wait(waiting: bool) {
    let note = RESTART_WAITING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if let Some(note) = note {
        note(waiting);
    }
}
// WU-FIX end

/// A `shell.restartAll` exec that waits for this process's file operations (D-G45).
type PendingRestart = std::sync::Mutex<Option<Box<dyn FnOnce() + Send>>>;
/// D-G45 (GX1 m5): a restarting Explorer waits while one of its own `files.*` calls is in flight (a copy, a move,
/// a paste, an extraction …), so its windows do not close in the middle of a copy. Its restart then runs on the
/// local `files.idle` event (watch_files_idle), which the end of the last call emits; with nothing in flight it
/// runs at once. Files::busy() arms that event under the job lock, so an end right after the check is not missed.
/// WU-FIX (D-G53): the wait is announced like the other deferrals (restart.rs: one log line and
/// `shell.restartDeferred {wait:'files'}`); `pending` stays locked until then, so its end is announced after it.
fn defer_while_busy(
    files: &files::Files,
    pending: &PendingRestart,
    restart: Box<dyn FnOnce() + Send>,
) {
    let run = {
        let mut slot = pending.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(restart);
        if files.busy() {
            note_restart_wait(true);
            return;
        }
        slot.take()
    };
    if let Some(run) = run {
        run();
    }
}
/// The other half of [`defer_while_busy`]: a deferred restart runs when the last file operation ended (WU-FIX:
/// announced as `shell.restartResumed` first, so the update owner rescans after this exec).
fn watch_files_idle(files: &files::Files, pending: Arc<PendingRestart>) {
    files.events.on(
        files::IDLE_EVENT,
        Arc::new(move |_| {
            let run = pending.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(run) = run {
                note_restart_wait(false);
                run();
            }
        }),
    );
}

/// `shell.restartAll` on the GTK main thread. Explorer closes its windows the way a user does (the page saves its
/// tabs on `win11:close`; window.rs closes the window 200 ms later), so the window state it flushes holds this
/// window's session; a page that does not close within 3 s is quit. Every other host quits at once (as the
/// Lifecycle request does), which also ends an update keep-alive hold. An app host without a window (Settings kept
/// alive by its update job) does not come back: there is no window to reopen, and its next start is new anyway;
/// restart.rs logs and announces that and takes a later restart again (review m2).
fn restart_on_main(
    kind: &'static str,
    requested: &AtomicBool,
    lifecycle: &std::sync::mpsc::Sender<UiRequest>,
) {
    let Some(app) = gio::Application::default() else {
        // no default application to ask: the Lifecycle request as for shell.restart
        requested.store(true, Ordering::Relaxed);
        let (ack, _) = std::sync::mpsc::channel();
        let _ = lifecycle.send(UiRequest {
            kind: UiRequestKind::Lifecycle,
            payload: json!({"restart":true}),
            ack,
        });
        return;
    };
    let windows = app
        .downcast_ref::<gtk::Application>()
        .map(|app| app.windows())
        .unwrap_or_default();
    if windows.is_empty() {
        drop_restart_all();
        return;
    }
    requested.store(true, Ordering::Relaxed);
    if kind != "explorer" {
        app.quit();
        return;
    }
    for window in &windows {
        window.close();
    }
    let weak = app.downgrade();
    glib::timeout_add_local_once(Duration::from_secs(3), move || {
        if let Some(app) = weak.upgrade() {
            app.quit();
        }
    });
}

/// win11-file-portal on `shell.restartAll`: exit when no file dialog is open and no FileChooser call is
/// unanswered (D-Bus activation starts the new build on the next request). With a request under way it stays,
/// and its own idle exit (15 s after the last request) ends it later; a dialog is never cancelled, also not one
/// that is still preparing and has no window yet (review m3).
pub(crate) fn quit_when_idle() {
    glib::MainContext::default().invoke(|| {
        let Some(app) = gio::Application::default() else {
            return;
        };
        let no_window = app
            .downcast_ref::<gtk::Application>()
            .is_none_or(|app| app.windows().is_empty());
        let no_call = PORTAL_CALLS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        if no_window && no_call {
            app.quit();
        } else {
            eprintln!("Win11 restart: a file dialog is under way; the portal ends after it");
        }
    });
}

/// win11-file-portal: the FileChooser calls (OpenFile, SaveFile, SaveFiles) whose reply has not left yet, as
/// (caller, serial). portal.rs keeps its requests to itself; a filter on the shared session connection sees each
/// call when it arrives and its reply when it leaves, so a request counts from its D-Bus call to its answer,
/// also while it is preparing without a window (portal.rs `Phase::Preparing`) and while it resolves.
static PORTAL_CALLS: std::sync::Mutex<Vec<(String, u32)>> = std::sync::Mutex::new(Vec::new());
/// More unanswered calls than this are not tracked (portal.rs answers a fifth one at once with LimitsExceeded).
const MAX_PORTAL_CALLS: usize = 256;
const PORTAL_IFACE: &str = "org.freedesktop.impl.portal.FileChooser";
/// Called once from the portal's NativeCore::new (services/shell/restart.rs), before portal.rs registers its
/// backend on the same connection (`gio::bus_get_sync` returns the process's one session connection).
pub(crate) fn count_portal_calls() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(bus) = gio::bus_get_sync(gio::BusType::Session, gio::Cancellable::NONE) else {
            return;
        };
        // GDBus runs filters on its worker thread, for every message in and out; this one only notes and
        // passes each message on unchanged
        bus.add_filter(|_, message, incoming| {
            let (peer, serial) = if incoming {
                (message.sender(), message.serial())
            } else {
                (message.destination(), message.reply_serial())
            };
            note_portal_message(
                &mut PORTAL_CALLS.lock().unwrap_or_else(|e| e.into_inner()),
                incoming,
                message.message_type(),
                message.interface().as_deref(),
                peer.as_deref(),
                serial,
            );
            Some(message.clone())
        });
    });
}
fn note_portal_message(
    calls: &mut Vec<(String, u32)>,
    incoming: bool,
    kind: gio::DBusMessageType,
    interface: Option<&str>,
    peer: Option<&str>,
    serial: u32,
) {
    let Some(peer) = peer else {
        return;
    };
    match kind {
        gio::DBusMessageType::MethodCall
            if incoming && interface == Some(PORTAL_IFACE) && calls.len() < MAX_PORTAL_CALLS =>
        {
            calls.push((peer.to_owned(), serial));
        }
        gio::DBusMessageType::MethodReturn | gio::DBusMessageType::Error if !incoming => {
            calls.retain(|(caller, call)| !(caller == peer && *call == serial));
        }
        _ => {}
    }
}

/// `current_exe()` names a binary that was replaced by rename (scripts/install.py) as `"<path> (deleted)"`:
/// the path itself now holds the new build (design §5).
pub(crate) fn exec_path(exe: PathBuf) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    match exe.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
        Some(path) => PathBuf::from(std::ffi::OsStr::from_bytes(path)),
        None => exe,
    }
}

/// The file a re-exec runs: `current_exe()` without `" (deleted)"`, or the running image itself
/// (`/proc/self/exe`) when that path no longer names a file.
fn exec_target(current: std::io::Result<PathBuf>) -> std::io::Result<PathBuf> {
    let executable = exec_path(current?);
    if executable.is_file() {
        return Ok(executable);
    }
    eprintln!(
        "Win11 restart: {} is gone; restarting the running build",
        executable.display()
    );
    Ok(PathBuf::from("/proc/self/exe"))
}

/// The argv (without argv[0]) of a `shell.restartAll` re-exec. The flags `--dry-run`/`--http-only` stay.
/// * the origin Settings reopens on its update page: `--app settings … --params {"page":"update"}`;
/// * Explorer drops its navigation arguments (a path, `--params`, a used `--tab-ticket`), so the page restores
///   its tabs from the saved window state (an explicit location would win over the session);
/// * every other host keeps its own argv.
pub(crate) fn restart_args(
    kind: &str,
    args: &[std::ffi::OsString],
    origin: bool,
) -> Vec<std::ffi::OsString> {
    let flags = args
        .iter()
        .filter(|arg| matches!(arg.to_str(), Some("--dry-run" | "--http-only")))
        .cloned();
    match kind {
        "settings" if origin => ["--app", "settings"]
            .into_iter()
            .map(Into::into)
            .chain(flags)
            .chain([
                "--params".into(),
                json!({"page":"update"}).to_string().into(),
            ])
            .collect(),
        "explorer" => flags.collect(),
        _ => args.to_vec(),
    }
}

/// exec this host again: the path behind `current_exe()` (a replaced binary's new build), or, when that path
/// is gone, the running image itself through `/proc/self/exe`.
fn reexec(kind: &str, plan: Option<&RestartAll>, handoff: Option<std::fs::File>) -> std::io::Error {
    use std::os::{fd::AsRawFd, unix::process::CommandExt};
    let executable = match exec_target(std::env::current_exe()) {
        Ok(executable) => executable,
        Err(error) => return error,
    };
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mut command = std::process::Command::new(executable);
    match plan {
        Some(plan) => {
            command.args(restart_args(kind, &args, plan.origin));
            // the first launch's token was used by the old window
            command
                .env_remove("XDG_ACTIVATION_TOKEN")
                .env_remove("DESKTOP_STARTUP_ID");
            if let Some(token) = &plan.token {
                command
                    .env("XDG_ACTIVATION_TOKEN", token)
                    .env("DESKTOP_STARTUP_ID", token);
            }
        }
        None => {
            command.args(args);
        }
    }
    if let Some(file) = &handoff {
        let fd = file.as_raw_fd();
        if unsafe { libc::fcntl(fd, libc::F_SETFD, 0) } == 0 {
            command.env(HANDOFF, fd.to_string());
        }
    }
    command.exec()
}

/// The environment variable that hands the Explorer restart lock to the new image (an fd number). `take_handoff`
/// removes it from the live environment before any thread or child exists; `/proc/<pid>/environ` still shows the
/// exec's original block with it, which is harmless: a process started from that copy gets an fd number that
/// `handoff_file` checks against the lock file's inode (review n12).
const HANDOFF: &str = "WIN11_RESTART_HANDOFF_FD";
fn handoff_path() -> PathBuf {
    glib::user_runtime_dir().join("win11-gnome/explorer-restart.lock")
}
/// Take the Explorer restart lock: a bounded wait (another Explorer releases it as soon as its new image has
/// read the window state; a crash releases it at once), then without it. This runs after the main loop ended,
/// only while another Explorer restarts at the same moment; a try every 50 ms, because a blocking flock cannot be
/// given up after a timeout without leaving a thread that may still take the lock.
fn explorer_handoff() -> Option<std::fs::File> {
    use std::os::{fd::AsRawFd, unix::fs::OpenOptionsExt};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(handoff_path())
        .ok()?;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Some(file);
        }
        if std::time::Instant::now() >= deadline {
            eprintln!(
                "Win11 restart: another Explorer holds the restart lock; going on without it"
            );
            return None;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
/// The lock a re-exec handed over. Called first in [`run`], before any thread exists.
fn take_handoff() -> Option<std::fs::File> {
    let raw = std::env::var_os(HANDOFF)?;
    // SAFETY: run() calls this before it starts any thread.
    unsafe { std::env::remove_var(HANDOFF) };
    handoff_file(&raw, &handoff_path())
}
/// What a handed-over fd number refers to.
#[derive(Debug, PartialEq, Eq)]
enum Handoff {
    /// the lock file at `lock` (same device, inode and owner)
    Ours,
    /// an older lock file whose path now names another file (the fd still shows the path, or the path
    /// `" (deleted)"`): ours, but no longer the lock
    Stale,
    /// anything else (no such fd, not a regular file, another file): not ours
    Foreign,
}
fn handoff_kind(fd: i32, lock: &Path) -> Handoff {
    use std::os::unix::fs::MetadataExt;
    let mut st = unsafe { std::mem::zeroed::<libc::stat>() };
    if unsafe { libc::fstat(fd, &mut st) } != 0 || st.st_mode & libc::S_IFMT != libc::S_IFREG {
        return Handoff::Foreign;
    }
    if std::fs::symlink_metadata(lock).is_ok_and(|path| {
        path.dev() == st.st_dev
            && path.ino() == st.st_ino
            && path.uid() == unsafe { libc::geteuid() }
    }) {
        return Handoff::Ours;
    }
    if std::fs::read_link(format!("/proc/self/fd/{fd}"))
        .is_ok_and(|target| exec_path(target) == lock)
    {
        return Handoff::Stale;
    }
    Handoff::Foreign
}
/// The handed-over lock, only if the fd really is our lock file; it becomes close-on-exec again, so no child
/// inherits it. A stale one is closed (the re-exec cleared its close-on-exec, so it would leak into every child,
/// review n4); an fd that is not ours is left alone.
fn handoff_file(raw: &std::ffi::OsStr, lock: &Path) -> Option<std::fs::File> {
    use std::os::fd::FromRawFd;
    let fd: i32 = raw.to_str()?.parse().ok().filter(|fd| *fd > 2)?;
    match handoff_kind(fd, lock) {
        Handoff::Ours => {
            unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
            Some(unsafe { std::fs::File::from_raw_fd(fd) })
        }
        Handoff::Stale => {
            unsafe { libc::close(fd) };
            None
        }
        Handoff::Foreign => None,
    }
}

/// After a re-exec this image inherits the old image's children: its WebKit processes and workers, which exit
/// once their sockets closed at the exec. Nobody else waits for them, so a thread reaps exactly those pids (a
/// fresh start has none; a pid cannot be reused while it is our unreaped child, and GLib and std only wait for the
/// pids they started).
fn reap_inherited_children() {
    let me = std::process::id();
    // Still one thread here, so every child is listed under it (the kernel hands the children of the old
    // image's other threads to the thread that execs); a scan of /proc if the kernel lacks that file.
    let children: Vec<libc::pid_t> =
        match std::fs::read_to_string(format!("/proc/self/task/{me}/children")) {
            Ok(list) => list
                .split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect(),
            Err(_) => std::fs::read_dir("/proc")
                .map(|entries| {
                    entries
                        .flatten()
                        .filter_map(|entry| entry.file_name().to_str()?.parse::<libc::pid_t>().ok())
                        .filter(|pid| {
                            std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                                stat.rsplit_once(") ").and_then(|(_, rest)| {
                                    rest.split_whitespace().nth(1)?.parse::<u32>().ok()
                                }) == Some(me)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
    fn reap(pid: libc::pid_t) {
        let mut status = 0;
        while unsafe { libc::waitpid(pid, &mut status, 0) } < 0
            && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR)
        {}
    }
    if !children.is_empty() {
        let _ = std::thread::Builder::new()
            .name("win11-reaper".into())
            .spawn(move || children.into_iter().for_each(reap));
    }
}

/// Separate native Everything worker; not a desktop surface.
pub fn run_index_worker() {
    services::index::worker_main();
}
pub fn run_image_worker() {
    std::process::exit(images::worker_main());
}

pub fn run_file_portal() -> Result<(), String> {
    portal::run()
}

pub fn run_software_worker() {
    std::process::exit(services::software::run_worker());
}

/// Native serve mode, with no GTK surface or display connection: the backend of win11-shell (`win11-panel
/// --http-only`, started by native/shell/src/backend.rs with WIN11_NATIVE_INSTANCE), API diagnostics and CI.
fn run_http_only(kind: &'static str) -> Result<(), String> {
    let ui = ui_root()?;
    let (sender, _receiver) = std::sync::mpsc::channel();
    let quit = Arc::new(AtomicBool::new(false));
    let restart = Arc::new(AtomicBool::new(false));
    let exiting = quit.clone();
    let again = restart.clone();
    let runtime = runtime::Runtime::new(
        ui,
        kind,
        sender,
        Arc::new(move |value| {
            again.store(value, Ordering::Relaxed);
            exiting.store(true, Ordering::Relaxed);
        }),
        false,
    )?;
    if kind == "panel" && DESKTOP_ENTRIES.load(Ordering::Relaxed) && !backend::dry_run() {
        watch_entries(&runtime);
    }
    let (port, stop) = runtime.server()?;
    let instance = std::env::var_os("WIN11_NATIVE_INSTANCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            glib::user_runtime_dir().join(format!("win11-gnome/http-{}.json", std::process::id()))
        });
    backend::write_instance(&instance, port, &runtime.backend.token)?;
    while !quit.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(25));
    }
    stop.store(true, Ordering::Relaxed);
    runtime.shutdown();
    backend::remove_instance(&instance);
    if restart.load(Ordering::Relaxed) {
        let plan = take_restart_all();
        return Err(reexec(kind, plan.as_ref(), None).to_string());
    }
    Ok(())
}

#[cfg(test)]
mod restart_tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn a_restarting_explorer_waits_for_its_file_operation_and_restarts_on_files_idle() {
        // D-G45 (GX1 m5): a copy in flight defers the restart; the end of the copy (files.idle) runs it, once
        let _serial = RESTART_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        // WU-FIX (D-G53): the wait and its end are announced (restart.rs's announcer), once each
        let notes: Arc<std::sync::Mutex<Vec<bool>>> = Arc::default();
        let seen = notes.clone();
        on_restart_wait(Some(Arc::new(move |waiting: bool| {
            seen.lock().unwrap().push(waiting)
        })));
        let root = std::env::temp_dir().join(format!("win11-restart-files-{}", events::nonce()));
        std::fs::create_dir(&root).unwrap();
        let files = files::Files::for_tests(&root);
        let pending: Arc<PendingRestart> = Arc::default();
        watch_files_idle(&files, pending.clone());
        let (ran, restarts) = std::sync::mpsc::channel();
        let restart = |label: &'static str| -> Box<dyn FnOnce() + Send> {
            let ran = ran.clone();
            Box::new(move || ran.send(label).unwrap())
        };
        let quiet = |restarts: &std::sync::mpsc::Receiver<&'static str>| {
            restarts.recv_timeout(Duration::from_millis(400)).is_err()
        };
        let copy = files.job_for_tests();
        defer_while_busy(&files, &pending, restart("deferred"));
        assert!(quiet(&restarts), "no restart while the copy runs");
        let list = files.job_for_tests();
        drop(list);
        assert!(
            quiet(&restarts),
            "another call ending is not the end of the copy"
        );
        drop(copy);
        assert_eq!(
            restarts.recv_timeout(Duration::from_secs(2)),
            Ok("deferred")
        );
        assert!(quiet(&restarts), "exactly once");
        assert_eq!(
            *notes.lock().unwrap(),
            [true, false],
            "the wait, then its end"
        );
        // nothing in flight: the restart runs at once, and a later end of a call runs nothing more
        defer_while_busy(&files, &pending, restart("now"));
        assert_eq!(restarts.recv_timeout(Duration::from_secs(1)), Ok("now"));
        drop(files.job_for_tests());
        assert!(quiet(&restarts));
        assert!(pending.lock().unwrap().is_none());
        assert_eq!(
            *notes.lock().unwrap(),
            [true, false],
            "no wait: nothing announced"
        );
        on_restart_wait(None);
        files.events.shutdown();
        drop(files);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    #[test]
    fn a_replaced_binary_execs_its_path() {
        assert_eq!(
            exec_path("/p/bin/win11-panel (deleted)".into()),
            PathBuf::from("/p/bin/win11-panel")
        );
        assert_eq!(
            exec_path("/p/bin/win11-panel".into()),
            PathBuf::from("/p/bin/win11-panel")
        );
        assert_eq!(
            exec_path("/a b/bin/win11-app (deleted)".into()),
            PathBuf::from("/a b/bin/win11-app"),
            "spaces in the prefix"
        );
        assert_eq!(
            exec_path("/p/bin/win11-app (deleted) (deleted)".into()),
            PathBuf::from("/p/bin/win11-app (deleted)"),
            "one suffix"
        );
        assert_eq!(
            exec_path("/p/bin/win11-app(deleted)".into()),
            PathBuf::from("/p/bin/win11-app(deleted)")
        );
        use std::os::unix::ffi::OsStringExt;
        let raw = PathBuf::from(OsString::from_vec(b"/p/\xff/win11-app (deleted)".to_vec()));
        assert_eq!(
            exec_path(raw),
            PathBuf::from(OsString::from_vec(b"/p/\xff/win11-app".to_vec())),
            "not UTF-8"
        );
    }

    #[test]
    fn restart_argv_overrides() {
        let settings = os(&["--app", "settings", "--params", r#"{"page":"bluetooth"}"#]);
        assert_eq!(
            restart_args("settings", &settings, true),
            os(&["--app", "settings", "--params", r#"{"page":"update"}"#])
        );
        assert_eq!(
            restart_args("settings", &settings, false),
            settings,
            "a second Settings keeps its page"
        );
        let dry = os(&["--app", "settings", "--dry-run"]);
        assert_eq!(
            restart_args("settings", &dry, true),
            os(&[
                "--app",
                "settings",
                "--dry-run",
                "--params",
                r#"{"page":"update"}"#
            ])
        );
        // the update page as the host parses it (run_inner: --app pair, then --params)
        let page: Value = serde_json::from_str(
            restart_args("settings", &settings, true)[3]
                .to_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(page, json!({"page":"update"}));
        for explorer in [
            os(&["/home/robin/Documenten"]),
            os(&["--params", r#"{"path":"/x"}"#]),
            os(&["--tab-ticket", "t"]),
            os(&["/a", "/b"]),
            os(&[]),
        ] {
            assert_eq!(
                restart_args("explorer", &explorer, false),
                Vec::<OsString>::new(),
                "{explorer:?}"
            );
            assert_eq!(
                restart_args("explorer", &explorer, true),
                Vec::<OsString>::new()
            );
        }
        assert_eq!(
            restart_args("explorer", &os(&["/a", "--dry-run"]), false),
            os(&["--dry-run"])
        );
        for (kind, args) in [
            ("taskmgr", os(&["--app", "taskmgr"])),
            ("panel", os(&["--http-only"])),
            ("installer", os(&["/x.deb"])),
        ] {
            assert_eq!(restart_args(kind, &args, false), args, "{kind}");
        }
    }

    #[test]
    fn the_restart_plan_is_taken_once() {
        let _serial = RESTART_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_restart_all();
        assert_eq!(restart_all_plan(), None);
        let (tx, dropped) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        prepare_restart_all(
            RestartAll {
                origin: true,
                token: Some("t".into()),
            },
            Some(Box::new(move || {
                let _ = tx.lock().unwrap().send(());
            })),
        );
        assert_eq!(
            restart_all_plan(),
            Some(RestartAll {
                origin: true,
                token: Some("t".into())
            })
        );
        assert_eq!(take_restart_all().map(|p| p.origin), Some(true));
        assert_eq!(take_restart_all(), None);
        assert!(!drop_restart_all(), "taken: nothing to drop");
        assert!(
            dropped.try_recv().is_err(),
            "a taken plan is not reported as dropped"
        );
        // the windowless path: the plan goes and restart.rs hears it once
        let (tx, dropped) = std::sync::mpsc::channel();
        let tx = std::sync::Mutex::new(tx);
        prepare_restart_all(
            RestartAll::default(),
            Some(Box::new(move || {
                let _ = tx.lock().unwrap().send(());
            })),
        );
        assert!(drop_restart_all());
        assert_eq!(dropped.try_recv(), Ok(()));
        assert_eq!(restart_all_plan(), None);
        assert!(!drop_restart_all());
        assert!(dropped.try_recv().is_err());
    }

    fn scratch(name: &str) -> PathBuf {
        use std::os::unix::fs::DirBuilderExt;
        let dir = std::env::temp_dir().join(format!("win11-{name}-{}", events::nonce()));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(&dir)
            .unwrap();
        dir
    }

    #[test]
    fn a_vanished_binary_restarts_the_running_image() {
        let dir = scratch("exec-target");
        let binary = dir.join("win11-app");
        std::fs::write(&binary, b"x").unwrap();
        let deleted = PathBuf::from(format!("{} (deleted)", binary.display()));
        assert_eq!(
            exec_target(Ok(deleted.clone())).unwrap(),
            binary,
            "replaced: its path"
        );
        assert_eq!(exec_target(Ok(binary.clone())).unwrap(), binary);
        std::fs::remove_file(&binary).unwrap();
        assert_eq!(
            exec_target(Ok(deleted)).unwrap(),
            PathBuf::from("/proc/self/exe"),
            "removed and not replaced: the running image"
        );
        std::fs::create_dir(&binary).unwrap();
        assert_eq!(
            exec_target(Ok(binary.clone())).unwrap(),
            PathBuf::from("/proc/self/exe"),
            "not a file"
        );
        assert!(exec_target(Err(std::io::Error::from_raw_os_error(libc::ENOENT))).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_handed_over_fd_must_be_the_lock_file() {
        use std::os::fd::{AsRawFd, IntoRawFd};
        let dir = scratch("handoff");
        let lock = dir.join("explorer-restart.lock");
        let file = std::fs::File::create(&lock).unwrap();
        let fd = file.as_raw_fd();
        assert_eq!(handoff_kind(fd, &lock), Handoff::Ours);
        let other = std::fs::File::create(dir.join("other")).unwrap();
        assert_eq!(
            handoff_kind(other.as_raw_fd(), &lock),
            Handoff::Foreign,
            "another file"
        );
        let pipe = std::fs::File::open("/proc/self/stat").unwrap();
        assert_eq!(
            handoff_kind(pipe.as_raw_fd(), &lock),
            Handoff::Foreign,
            "not our path"
        );
        assert_eq!(handoff_kind(-1, &lock), Handoff::Foreign, "no such fd");
        let dirfd = std::fs::File::open(&dir).unwrap();
        assert_eq!(
            handoff_kind(dirfd.as_raw_fd(), &lock),
            Handoff::Foreign,
            "not a regular file"
        );
        // the lock file was replaced: the old fd is ours, but stale
        std::fs::write(dir.join("new"), b"").unwrap();
        std::fs::rename(dir.join("new"), &lock).unwrap();
        assert_eq!(handoff_kind(fd, &lock), Handoff::Stale);
        assert_eq!(handoff_kind(other.as_raw_fd(), &lock), Handoff::Foreign);
        // the number itself: only a plain fd above stderr
        for raw in ["", "x", "-1", "0", "1", "2", "3x", "99999999999"] {
            assert!(
                handoff_file(std::ffi::OsStr::new(raw), &lock).is_none(),
                "{raw:?}"
            );
        }
        assert!(
            handoff_file(std::ffi::OsStr::new(&other.as_raw_fd().to_string()), &lock).is_none()
        );
        assert_ne!(
            unsafe { libc::fcntl(other.as_raw_fd(), libc::F_GETFD) },
            -1,
            "a foreign fd is left open"
        );
        // ours: the File takes the fd over, close-on-exec again
        let fresh = std::fs::File::create(dir.join("fresh")).unwrap();
        std::fs::rename(dir.join("fresh"), &lock).unwrap();
        let raw = fresh.into_raw_fd();
        unsafe { libc::fcntl(raw, libc::F_SETFD, 0) };
        let taken = handoff_file(std::ffi::OsStr::new(&raw.to_string()), &lock).unwrap();
        assert_eq!(taken.as_raw_fd(), raw);
        assert_eq!(
            unsafe { libc::fcntl(raw, libc::F_GETFD) } & libc::FD_CLOEXEC,
            libc::FD_CLOEXEC
        );
        drop((taken, file, other, pipe, dirfd));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_portal_counts_a_file_chooser_call_until_its_answer() {
        use gio::DBusMessageType::{Error, MethodCall, MethodReturn, Signal};
        let mut calls = Vec::new();
        let iface = Some(PORTAL_IFACE);
        note_portal_message(&mut calls, true, MethodCall, iface, Some(":1.7"), 5);
        note_portal_message(&mut calls, true, MethodCall, iface, Some(":1.8"), 5);
        assert_eq!(calls.len(), 2, "two callers, the same serial");
        // other traffic changes nothing: another interface, a signal, a reply to another call or caller
        note_portal_message(
            &mut calls,
            true,
            MethodCall,
            Some("org.freedesktop.DBus.Properties"),
            Some(":1.7"),
            6,
        );
        note_portal_message(&mut calls, true, Signal, iface, Some(":1.7"), 7);
        note_portal_message(&mut calls, false, MethodReturn, None, Some(":1.7"), 6);
        note_portal_message(&mut calls, false, MethodReturn, None, Some(":1.9"), 5);
        note_portal_message(&mut calls, true, MethodReturn, None, Some(":1.7"), 5);
        note_portal_message(&mut calls, true, MethodCall, iface, None, 9);
        assert_eq!(calls, vec![(":1.7".to_owned(), 5), (":1.8".to_owned(), 5)]);
        // the answers: a result, or an error (Request.Close, a refusal)
        note_portal_message(&mut calls, false, MethodReturn, None, Some(":1.7"), 5);
        assert_eq!(calls, vec![(":1.8".to_owned(), 5)]);
        note_portal_message(&mut calls, false, Error, None, Some(":1.8"), 5);
        assert!(calls.is_empty());
        // bounded
        for serial in 0..(MAX_PORTAL_CALLS as u32 + 10) {
            note_portal_message(&mut calls, true, MethodCall, iface, Some(":1.7"), serial);
        }
        assert_eq!(calls.len(), MAX_PORTAL_CALLS);
    }
}
