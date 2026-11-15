//! Win11 GNOME native shell (GTK 4, D-G63): the taskbar with its flyouts, Start and the desktop in one process,
//! without WebKit. Content comes from the host's Rust services (`win11-panel --http-only`, see backend.rs); the
//! GNOME extension positions, blurs and focuses the windows (gnome.rs).
//!
//! Environment:
//!   WIN11_SHELL_SURFACES   comma list of surfaces to run (default "panel,start,desktop")
//!   WIN11_SHELL_BACKEND    "port:token" of a running backend instead of starting one (tests)
//!   WIN11_DEV_COMMAND / WIN11_DEV_REPORT   the harness's dev channel (dev.rs)
//!
//! shell.restartAll (Windows Update): the backend ends with a restart note and the shell execs itself again
//! (`restart`, backend.rs), so the taskbar runs the new build and the new GTK libraries.
mod backend;
mod core;
mod desktop;
mod dev;
mod flyouts;
mod gnome;
mod icons;
mod panel;
mod start;
mod theme;
mod wayland;

use gtk::prelude::*;
use gtk::{gio, glib};

pub const PANEL_ID: &str = "io.github.win11shell.Panel";
pub const START_ID: &str = "io.github.win11shell.Start";
pub const DESKTOP_ID: &str = "io.github.win11shell.Desktop";

fn main() -> glib::ExitCode {
    // RAM first (Robin): GTK's cairo renderer draws on the CPU and keeps no GL context with its driver memory;
    // the taskbar, flyouts and Start are light enough for it. WIN11_SHELL_GL=1 or an explicit GSK_RENDERER
    // keeps GTK's default (GL/Vulkan).
    if std::env::var_os("GSK_RENDERER").is_none() && std::env::var("WIN11_SHELL_GL").as_deref() != Ok("1") {
        // SAFETY: first thing in main, before GTK or any other thread starts.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
        // the user had none: the backend gives an app it launches the user's renderer, not our cairo (host
        // files/actions.rs GPU_KEYS); a marker that a WinPoint parent passed on is the user's already
        if std::env::var_os("WIN11SHELL_USER_GPU").is_none() {
            let user =
                std::env::var("GDK_DEBUG").map_or(serde_json::json!({}), |v| serde_json::json!({ "GDK_DEBUG": v }));
            unsafe { std::env::set_var("WIN11SHELL_USER_GPU", user.to_string()) };
        }
    }
    glib::set_prgname(Some(PANEL_ID));
    let app = gtk::Application::builder()
        .application_id(PANEL_ID)
        .flags(gio::ApplicationFlags::empty())
        .build();
    app.connect_startup(|app| {
        let backend = match backend::Backend::connect(core::backend_ready) {
            Ok(backend) => backend,
            Err(error) => {
                eprintln!("win11-shell: backend: {error}");
                app.quit();
                return;
            }
        };
        let shell = core::Shell::new(app, backend);
        shell.on("backend.restart", |_, _| restart());
        theme::install(&shell);
        let surfaces = std::env::var("WIN11_SHELL_SURFACES").unwrap_or_else(|_| "panel,start,desktop".into());
        let wanted = |s: &str| surfaces.split(',').any(|x| x.trim() == s);
        if wanted("panel") {
            panel::install(&shell);
        }
        if wanted("start") {
            start::install(&shell);
        }
        if wanted("desktop") {
            desktop::install(&shell);
        }
        dev::install(&shell);
        // The shell is resident: it lives as long as the session, whatever windows are mapped.
        std::mem::forget(app.hold());
    });
    // Resident: an activate (the extension's open_new_window when it comes back after the lock screen) only shows
    // the taskbar again if the extension closed it.
    app.connect_activate(|_| panel::show_again());
    app.run_with_args::<&str>(&[])
}

/// `shell.restartAll` reached the backend (backend.rs): exec win11-shell again, the installed build with the same
/// arguments, which starts a new backend. The pid, GNOME's launch scope and the Panel identity stay; the windows
/// are gone only while the new image starts. When the exec fails the shell stays, on a new backend, with its
/// Panel name taken back (`after_failed_exec`).
fn restart() {
    let shell = core::shell();
    let Some(note) = shell.backend.take_restart() else {
        shell.backend.answer_restart(true);
        return;
    };
    let error = exec_self(&shell.app, note.token.as_deref());
    eprintln!("win11-shell: restart: {error}; staying on a new backend");
    after_failed_exec(&shell.app);
    shell.backend.answer_restart(true);
}

/// The shell stays after a failed exec: undo what `exec_self` and `Command::exec` changed. The Panel id back on the
/// session bus (exec_self released it for the new image; GApplication's objects are still exported on this
/// connection, so the name is all that is missing): the extension's org.gtk.Actions calls (task view, the power
/// dialog) reach this process again and a later launch of the Panel is not a second taskbar. SIGPIPE ignored again,
/// as Rust's runtime set it at start (a failed `Command::exec` leaves it at SIG_DFL). The same as native/kit
/// app.rs `after_failed_exec` (the shell has no kit dependency).
fn after_failed_exec(app: &gtk::Application) {
    // SAFETY: signal(2) with SIG_IGN; nothing in this process installs a SIGPIPE handler of its own
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let (Some(bus), Some(id)) = (app.dbus_connection(), app.application_id()) else { return };
    // flags 4: DBUS_NAME_FLAG_DO_NOT_QUEUE
    let reply = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(id.as_str(), 4u32).to_variant()),
        Some(glib::VariantTy::new("(u)").expect("static type")),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    );
    match reply.map(|r| r.child_value(0).get::<u32>()) {
        // 1 PRIMARY_OWNER, 4 ALREADY_OWNER
        Ok(Some(1 | 4)) => {}
        Ok(code) => eprintln!("win11-shell: restart: {id} was taken by another instance while the exec failed (RequestName {code:?})"),
        Err(error) => eprintln!("win11-shell: restart: taking {id} back: {error}"),
    }
}

/// The binary a restart execs: this one's path (a binary replaced by rename shows as "<path> (deleted)": that
/// path holds the new build), or the running image when the path is gone (host lib.rs exec_target).
fn exec_target() -> std::path::PathBuf {
    use std::os::unix::ffi::OsStrExt;
    let path = std::env::current_exe().ok().map(|exe| match exe.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
        Some(path) => std::path::PathBuf::from(std::ffi::OsStr::from_bytes(path)),
        None => exe,
    });
    match path {
        Some(path) if path.is_file() => path,
        _ => std::path::PathBuf::from("/proc/self/exe"),
    }
}

fn exec_self(app: &gtk::Application, token: Option<&str>) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let target = exec_target();
    // the new image must become the primary instance of the Panel id again
    if let (Some(bus), Some(id)) = (app.dbus_connection(), app.application_id()) {
        let _ = bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ReleaseName",
            Some(&(id.as_str(),).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        );
        let _ = bus.flush_sync(gio::Cancellable::NONE);
    }
    let mut command = std::process::Command::new(&target);
    command.args(std::env::args_os().skip(1)).env_remove("XDG_ACTIVATION_TOKEN").env_remove("DESKTOP_STARTUP_ID");
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token).env("DESKTOP_STARTUP_ID", token);
    }
    command.exec()
}
