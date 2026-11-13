//! Win11 GNOME native shell (GTK 4, D-G63): the taskbar with its flyouts, Start and the desktop in one process,
//! without WebKit. Content comes from the host's Rust services (`win11-panel --http-only`, see backend.rs); the
//! GNOME extension positions, blurs and focuses the windows as it does for the WebKit surfaces (gnome.rs).
//!
//! Environment:
//!   WIN11_SHELL_SURFACES   comma list of surfaces to run (default "panel,start,desktop")
//!   WIN11_SHELL_BACKEND    "port:token" of a running backend instead of starting one (tests)
//!   WIN11_DEV_COMMAND / WIN11_DEV_REPORT   the harness's dev channel (dev.rs)
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
    app.connect_activate(|_| {});
    app.run_with_args::<&str>(&[])
}
