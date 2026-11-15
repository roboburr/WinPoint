#![recursion_limit = "512"]
use gio::prelude::*;
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
mod gnome_look;
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

mod backend;
mod runtime;

/// WinPoint's version: `[workspace.package] version` of the root Cargo.toml, the one source (D-G74a). Settings
/// shows it (`shell.info`), the updater compares it (services/winpoint.rs).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// `win11-panel --version` prints this line (bin/panel.rs). The same bytes are the marker that tools look for in the
/// file: scripts/package.py and tools/release.py read a binary's version without running it.
pub const VERSION_LINE: &str = concat!("WinPoint ", env!("CARGO_PKG_VERSION"), "\n");

#[cfg(test)]
mod version_tests {
    #[test]
    fn metadata_has_the_workspace_version() {
        let metadata: serde_json::Value =
            serde_json::from_str(include_str!("../../extension/metadata.json")).unwrap();
        assert_eq!(
            metadata["version-name"],
            super::VERSION,
            "tools/release.py bump"
        );
        assert_eq!(
            super::VERSION_LINE,
            format!("WinPoint {}\n", super::VERSION)
        );
        let version = super::services::winpoint::version::Version::parse(super::VERSION);
        assert!(
            version.is_some_and(|v| v.pre.is_empty()),
            "a release version"
        );
    }
}

/// The UI folder: `share/win11-gnome/ui` of the installed payload (or WIN11_UI_DIR), `vendor/ui` in a checkout.
/// It holds only assets now (the built-in wallpapers and the Settings icon, served below `/ui/assets/`), so the
/// wallpapers' SOURCES.json marks it; its parent is the payload root (caption CSS, runtime.rs).
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
        .find(|p| p.join("assets/wallpapers/SOURCES.json").is_file())
        .ok_or("De oorspronkelijke interface ontbreekt".into())
}

/// The type of a file below `/ui/assets/` (runtime.rs): images, fonts and the wallpapers' SOURCES.json.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()).unwrap_or("") {
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

static DESKTOP_ENTRIES: AtomicBool = AtomicBool::new(false);

/// Makes this process the single writer of the own desktop entries' `Name=`/`Comment=` (contracts C6). Only
/// `win11-panel` calls it (bin/panel.rs), before [`run`]; the writer is its `--http-only` backend of win11-shell
/// ([`watch_entries`]), since win11-panel itself execs win11-shell.
pub fn enable_desktop_entries() {
    DESKTOP_ENTRIES.store(true, Ordering::Relaxed);
}

/// C6 under the native shell: `win11-panel --http-only` (win11-shell's backend) writes the own desktop entries'
/// `Name=`/`Comment=` at start and, debounced, after a language switch. [`desktop_entries::schedule`] runs on its
/// own thread (not on this mode's main loop, run_http_only) and reads the language from the settings file when it
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
    native_shell_switch(kind);
    let http_only = std::env::args().any(|arg| arg == "--http-only");
    // A GUI launch of an app kind runs its native program; a missing one is said here, on the caller's stderr,
    // before ensure_native moves this process into a unit of its own (whose stderr is the journal).
    let app = (!http_only).then(|| native_app(kind));
    // The backend of a native GTK 4 app (native/kit backend.rs: `--app <kind> --http-only` with
    // WIN11_NATIVE_INSTANCE) stays the app's child, in the app's own unit. Moved into a unit of its own, this
    // process would end at once and the app would report its backend as gone (an app started outside its launcher).
    let app_backend =
        kind != "panel" && http_only && std::env::var_os("WIN11_NATIVE_INSTANCE").is_some();
    let relocated = if app_backend {
        Ok(false)
    } else {
        process_scope::ensure_native(kind)
    };
    match relocated {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            eprintln!("Win11 GNOME-procesisolatie: {error}");
            std::process::exit(1);
        }
    }
    if let Some(app) = app {
        exec_native_app(&app);
    }
    reap_inherited_children();
    if let Err(err) = run_http_only(kind) {
        eprintln!("Win11 voor GNOME: {err}");
        std::process::exit(1);
    }
}

/// D-G70: the native GTK 4 shell (native/shell, `win11-shell`) is the shell: taskbar, flyouts, Start and the
/// desktop. win11-panel without `--http-only` execs `win11-shell` from its own directory with the same argv and
/// environment, so the pid, GNOME's launch scope and the Panel identity stay; win11-shell then starts
/// `win11-panel --http-only` as its backend (native/shell/src/backend.rs). When win11-shell is missing or the
/// exec fails, win11-panel exits and the extension shows its fallback bar.
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

/// `win11-app --gnome-look`: GNOME's own look again in a session that is not WinPoint's (gnome_look.rs).
pub fn restore_gnome_look() -> i32 {
    gnome_look::restore()
}

/// The native GTK 4 app of a kind (native/apps): `win11-gtk-<kind>`, installed next to the host binaries.
fn native_app_binary(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "explorer" => "win11-gtk-explorer",
        "settings" => "win11-gtk-settings",
        "taskmgr" => "win11-gtk-taskmgr",
        "alles" => "win11-gtk-alles",
        "disks" => "win11-gtk-disks",
        "defaultapps" => "win11-gtk-defaultapps",
        "installer" => "win11-gtk-installer",
        _ => return None,
    })
}

/// The apps are native GTK 4 programs (native/apps), and there is no other app to fall back to: a GUI launch of an
/// app kind needs `win11-gtk-<kind>` next to this binary. Without it the launch names the missing file and the fix
/// on stderr and ends with 127 (the shell's "command not found").
fn native_app(kind: &str) -> PathBuf {
    let app = std::env::current_exe()
        .map(exec_path)
        .unwrap_or_default()
        .with_file_name(native_app_binary(kind).expect("an app kind has a native program"));
    if !app.is_file() {
        eprintln!(
            "{}",
            i18n::trf(
                i18n::current_language(),
                "core.nativeAppMissing",
                &json!({"path": app.display().to_string()})
            )
        );
        std::process::exit(127);
    }
    app
}

/// A GUI launch, already inside its own scope (ensure_native), execs its native app with the same arguments
/// (without the `--app <kind>` of win11-app), so the pid, the launch scope and the slice stay. That app starts
/// `win11-app --app <kind> --http-only` as its backend. An exec that fails ends the launch with 1.
fn exec_native_app(app: &Path) -> ! {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    if args.first().map(|arg| arg.as_os_str()) == Some(std::ffi::OsStr::new("--app")) {
        args.drain(..args.len().min(2));
    }
    use std::os::unix::process::CommandExt;
    let error = std::process::Command::new(app).args(args).exec();
    eprintln!("Win11 voor GNOME: {}: {error}", app.display());
    std::process::exit(1);
}

/// What a `shell.restartAll` restart needs (design §5). services/shell/restart.rs sets it right before
/// `request_shutdown(true)`; a plain `shell.restart` leaves it empty. A native program's backend hands it to its
/// GUI ([`restart_note`]); a backend without one re-execs with it ([`reexec`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RestartAll {
    /// this process called `shell.restartAll` (Settings reopens on its update page)
    pub(crate) origin: bool,
    /// a fresh activation token for the new window (app kinds), so it maps focused like a launch
    pub(crate) token: Option<String>,
}
static RESTART_ALL: std::sync::Mutex<Option<RestartAll>> = std::sync::Mutex::new(None);
pub(crate) fn prepare_restart_all(plan: RestartAll) {
    *RESTART_ALL.lock().unwrap_or_else(|e| e.into_inner()) = Some(plan);
}
pub(crate) fn restart_all_plan() -> Option<RestartAll> {
    RESTART_ALL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}
pub(crate) fn take_restart_all() -> Option<RestartAll> {
    RESTART_ALL.lock().unwrap_or_else(|e| e.into_inner()).take()
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

/// win11-file-portal on `shell.restartAll`: exit when no FileChooser call is unanswered (D-Bus activation starts
/// the new build on the next request). With a request under way it stays, and its own idle exit (15 s after the
/// last request) ends it later; a dialog is never cancelled, also not one that is still preparing and has no
/// picker yet (review m3): a call counts until its answer, and its picker lives no longer than the call.
pub(crate) fn quit_when_idle() {
    glib::MainContext::default().invoke(|| {
        let Some(app) = gio::Application::default() else {
            return;
        };
        let no_call = PORTAL_CALLS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        if no_call {
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

/// exec this host again with its own argv: the path behind `current_exe()` (a replaced binary's new build), or,
/// when that path is gone, the running image itself through `/proc/self/exe`. Only a `--http-only` host without a
/// native GUI does this (run_http_only: API diagnostics and CI); a native program restarts itself, backend and all.
/// A `shell.restartAll` plan gives the new image its activation token (the first launch's was used).
fn reexec(plan: Option<&RestartAll>) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let executable = match exec_target(std::env::current_exe()) {
        Ok(executable) => executable,
        Err(error) => return error,
    };
    let mut command = std::process::Command::new(executable);
    command.args(std::env::args_os().skip(1));
    if let Some(plan) = plan {
        command
            .env_remove("XDG_ACTIVATION_TOKEN")
            .env_remove("DESKTOP_STARTUP_ID");
        if let Some(token) = &plan.token {
            command
                .env("XDG_ACTIVATION_TOKEN", token)
                .env("DESKTOP_STARTUP_ID", token);
        }
    }
    command.exec()
}

/// After a re-exec this image inherits the old image's children: its workers, which exit once their sockets closed
/// at the exec. Nobody else waits for them, so a thread reaps exactly those pids (a fresh start has none; a pid
/// cannot be reused while it is our unreaped child, and GLib and std only wait for the pids they started).
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

/// The exit status of a native program's backend that restarts (`shell.restartAll`, `shell.restart`; EX_TEMPFAIL).
/// It wrote `<instance>.restart` ([`restart_note`]) first. Its GUI (native/kit/src/backend.rs for the win11-gtk-*
/// apps, native/shell/src/backend.rs for win11-shell) waits on its child: on this status it saves its own state and
/// execs itself, new build and new backend in one go (restartAll), or starts a new backend (a plain
/// `shell.restart`). Any other exit of the backend makes the GUI start a new one.
pub(crate) const RESTART_EXIT: i32 = 75;

/// `<instance>.restart`, next to the instance file the GUI named in WIN11_NATIVE_INSTANCE.
pub(crate) fn restart_note_path(instance: &Path) -> PathBuf {
    let mut name = instance.as_os_str().to_owned();
    name.push(".restart");
    PathBuf::from(name)
}

/// What the GUI of a native backend needs for its restart: `all` (a `shell.restartAll`; false: a plain
/// `shell.restart`), `origin` (this backend called `shell.restartAll`: Settings reopens on its update page) and the
/// activation `token` for the new window (XDG_ACTIVATION_TOKEN / DESKTOP_STARTUP_ID), `pid` of the backend.
pub(crate) fn restart_note(plan: Option<&RestartAll>) -> Value {
    json!({
        "version": 1,
        "pid": std::process::id(),
        "all": plan.is_some(),
        "origin": plan.is_some_and(|plan| plan.origin),
        "token": plan.and_then(|plan| plan.token.clone()),
    })
}

/// The note, written as backend.rs writes the instance file: a private temp file in the same folder, renamed.
pub(crate) fn write_restart_note(instance: &Path, note: &Value) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = restart_note_path(instance);
    let dir = path.parent().ok_or(std::io::ErrorKind::NotFound)?;
    let tmp = dir.join(format!(
        ".restart-{}-{}",
        std::process::id(),
        events::nonce()
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&tmp)?;
        file.write_all(note.to_string().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// What a `--http-only` host does on `request_shutdown(again)`: `stop` at once, except a `shell.restartAll` restart
/// of the native Explorer's backend, which first waits for Explorer's own file operations (D-G45,
/// [`defer_while_busy`]; announced as `shell.restartDeferred {wait:'files'}` and `shell.restartResumed`).
fn http_only_shutdown(
    kind: &str,
    again: bool,
    files: Option<&files::Files>,
    pending: &PendingRestart,
    stop: Box<dyn FnOnce() + Send>,
) {
    match files.filter(|_| again && kind == "explorer" && restart_all_plan().is_some()) {
        Some(files) => defer_while_busy(files, pending, stop),
        None => stop(),
    }
}

/// Native serve mode, with no GTK surface or display connection: the backend of win11-shell (`win11-panel
/// --http-only`, started by native/shell/src/backend.rs) and of the native apps (`win11-app --app <kind>
/// --http-only`, native/kit/src/backend.rs), both with WIN11_NATIVE_INSTANCE; API diagnostics and CI.
///
/// The main thread runs a GLib main loop on the default context: the services' timeouts (update.rs: the mirror's
/// owner retry, the release hand-over watch) and `MainContext::default().invoke` run there, and a 500 ms timer
/// beats the runtime, so `shell.info` reports `mainLoopLagMs`.
///
/// Restarts: a native program's backend never re-execs itself (a new port and token would leave its GUI offline,
/// and the GUI itself would keep the old build). It writes `<instance>.restart` and exits with [`RESTART_EXIT`];
/// the GUI does the rest. Without WIN11_NATIVE_INSTANCE (diagnostics) the host re-execs itself ([`reexec`]).
fn run_http_only(kind: &'static str) -> Result<(), String> {
    let ui = ui_root()?;
    let main_loop = glib::MainLoop::new(None, false);
    let restart = Arc::new(AtomicBool::new(false));
    let explorer_files: Arc<std::sync::OnceLock<Arc<files::Files>>> = Arc::default();
    let pending_restart: Arc<PendingRestart> = Arc::default();
    let shutdown = {
        let (main_loop, restart) = (main_loop.clone(), restart.clone());
        let (files, pending) = (explorer_files.clone(), pending_restart.clone());
        Arc::new(move |again: bool| {
            let (main_loop, restart) = (main_loop.clone(), restart.clone());
            let stop: Box<dyn FnOnce() + Send> = Box::new(move || {
                restart.store(again, Ordering::Relaxed);
                // a source on the loop's context, never run in this thread: also a quit that comes before
                // run() starts ends the loop (invoke() would run it here at once while nobody owns the context)
                glib::idle_add_once(move || main_loop.quit());
            });
            http_only_shutdown(
                kind,
                again,
                files.get().map(|files| &**files),
                &pending,
                stop,
            );
        })
    };
    let runtime = runtime::Runtime::new(ui, kind, shutdown)?;
    if kind == "explorer" {
        let _ = explorer_files.set(runtime.files.clone());
        watch_files_idle(&runtime.files, pending_restart);
    }
    if kind == "panel" && DESKTOP_ENTRIES.load(Ordering::Relaxed) && !backend::dry_run() {
        watch_entries(&runtime);
    }
    let native = std::env::var_os("WIN11_NATIVE_INSTANCE").map(PathBuf::from);
    // D-G74a: win11-shell's backend is the one owner of WinPoint's update check (services/winpoint.rs): its
    // timers run on this main loop. Diagnostic and CI panel hosts (no instance file, or a dry run) never ask GitHub.
    if kind == "panel" && native.is_some() && !backend::dry_run() {
        runtime.winpoint_start();
    }
    let (port, stop) = runtime.server()?;
    let instance = native.clone().unwrap_or_else(|| {
        glib::user_runtime_dir().join(format!("win11-gnome/http-{}.json", std::process::id()))
    });
    backend::write_instance(&instance, port, &runtime.backend.token)?;
    let beat = Arc::downgrade(&runtime);
    glib::timeout_add_local(Duration::from_millis(500), move || match beat.upgrade() {
        Some(runtime) => {
            runtime.beat();
            glib::ControlFlow::Continue
        }
        None => glib::ControlFlow::Break,
    });
    main_loop.run();
    stop.store(true, Ordering::Relaxed);
    runtime.shutdown();
    backend::remove_instance(&instance);
    if restart.load(Ordering::Relaxed) {
        let plan = take_restart_all();
        if let Some(instance) = native {
            if let Err(error) = write_restart_note(&instance, &restart_note(plan.as_ref())) {
                // without the note the GUI takes this for a crash and starts a new backend
                eprintln!(
                    "Win11 restart: {}: {error}",
                    restart_note_path(&instance).display()
                );
            }
            std::process::exit(RESTART_EXIT);
        }
        return Err(reexec(plan.as_ref()).to_string());
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
    fn a_native_backend_leaves_a_restart_note_for_its_gui() {
        let dir = scratch("restart-note");
        let instance = dir.join("native-settings-42.json");
        assert_eq!(
            restart_note_path(&instance),
            dir.join("native-settings-42.json.restart")
        );
        let plan = RestartAll {
            origin: true,
            token: Some("tok".into()),
        };
        let note = restart_note(Some(&plan));
        assert_eq!(
            (&note["all"], &note["origin"], &note["token"], &note["pid"]),
            (
                &json!(true),
                &json!(true),
                &json!("tok"),
                &json!(std::process::id())
            )
        );
        // a plain shell.restart: a new backend only
        let plain = restart_note(None);
        assert_eq!(
            (&plain["all"], &plain["origin"], &plain["token"]),
            (&json!(false), &json!(false), &Value::Null)
        );
        write_restart_note(&instance, &plain).unwrap();
        write_restart_note(&instance, &note).unwrap();
        let path = restart_note_path(&instance);
        let read: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(read, note, "the last note replaces the first");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(
            names,
            [OsString::from("native-settings-42.json.restart")],
            "no temp file left"
        );
        assert_ne!(RESTART_EXIT, 0);
        assert_ne!(RESTART_EXIT, 1, "not the status of a failed start");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_native_explorer_backend_waits_for_its_file_operations_before_it_restarts() {
        // D-G45 in run_http_only: the backend of win11-gtk-explorer does its file operations itself
        let _serial = RESTART_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_restart_all();
        let root = std::env::temp_dir().join(format!("win11-http-files-{}", events::nonce()));
        std::fs::create_dir(&root).unwrap();
        let files = files::Files::for_tests(&root);
        let pending: Arc<PendingRestart> = Arc::default();
        watch_files_idle(&files, pending.clone());
        let (tx, stops) = std::sync::mpsc::channel();
        let stop = |label: &'static str| -> Box<dyn FnOnce() + Send> {
            let tx = tx.clone();
            Box::new(move || tx.send(label).unwrap())
        };
        let quiet = |stops: &std::sync::mpsc::Receiver<&'static str>| {
            stops.recv_timeout(Duration::from_millis(400)).is_err()
        };
        let copy = files.job_for_tests();
        // a plain shell.restart / shell.quit, and any other kind: at once, also with a copy in flight
        http_only_shutdown("explorer", true, Some(&files), &pending, stop("plain"));
        assert_eq!(stops.try_recv(), Ok("plain"));
        http_only_shutdown("explorer", false, Some(&files), &pending, stop("quit"));
        assert_eq!(stops.try_recv(), Ok("quit"));
        prepare_restart_all(RestartAll::default());
        http_only_shutdown("taskmgr", true, Some(&files), &pending, stop("taskmgr"));
        assert_eq!(stops.try_recv(), Ok("taskmgr"));
        // shell.restartAll in Explorer's backend: after the copy
        http_only_shutdown("explorer", true, Some(&files), &pending, stop("explorer"));
        assert!(quiet(&stops), "no restart while the copy runs");
        drop(copy);
        assert_eq!(stops.recv_timeout(Duration::from_secs(2)), Ok("explorer"));
        assert!(quiet(&stops), "exactly once");
        // nothing in flight: at once
        http_only_shutdown("explorer", true, Some(&files), &pending, stop("idle"));
        assert_eq!(stops.recv_timeout(Duration::from_secs(1)), Ok("idle"));
        let _ = take_restart_all();
        files.events.shutdown();
        drop(files);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_restart_plan_is_taken_once() {
        let _serial = RESTART_TESTS.lock().unwrap_or_else(|e| e.into_inner());
        let _ = take_restart_all();
        assert_eq!(restart_all_plan(), None);
        prepare_restart_all(RestartAll {
            origin: true,
            token: Some("t".into()),
        });
        assert_eq!(
            restart_all_plan(),
            Some(RestartAll {
                origin: true,
                token: Some("t".into())
            }),
            "looking does not take it"
        );
        assert_eq!(take_restart_all().map(|p| p.origin), Some(true));
        assert_eq!(take_restart_all(), None);
        assert_eq!(restart_all_plan(), None);
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
