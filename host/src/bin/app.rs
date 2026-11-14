//! Generic entry point for the seven own apps: Explorer, Settings, Task Manager, Alles, Disks, Default apps and the
//! App Installer. A GUI launch execs the native GTK 4 app `win11-gtk-<kind>` (native/apps) without this
//! `--app <kind>` prefix; with `--http-only` it is that app's backend (`win11-app --app <kind> --http-only`).

use std::{ffi::OsStr, process::ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(OsStr::new("--gnome-look")) {
        // The autostart entry of every session but WinPoint's (scripts/session.py): GNOME's own look again.
        return ExitCode::from(win11_host::restore_gnome_look() as u8);
    }
    if first.as_deref() == Some(OsStr::new("--help")) {
        println!(
            "Gebruik: win11-app --app explorer|settings|taskmgr|alles|disks|defaultapps|installer [--params JSON]"
        );
        return ExitCode::SUCCESS;
    }
    if first.as_deref() != Some(OsStr::new("--app")) {
        eprintln!(
            "Gebruik: win11-app --app explorer|settings|taskmgr|alles|disks|defaultapps|installer [--params JSON]"
        );
        return ExitCode::from(2);
    }
    let kind: &'static str = match args.next().as_deref().and_then(OsStr::to_str) {
        Some("explorer") => "explorer",
        Some("settings") => "settings",
        Some("taskmgr") => "taskmgr",
        Some("alles") => "alles",
        Some("disks") => "disks",
        Some("defaultapps") => "defaultapps",
        Some("installer") => "installer",
        _ => {
            eprintln!(
                "Kies een bestaande app: explorer, settings, taskmgr, alles, disks, defaultapps of installer."
            );
            return ExitCode::from(2);
        }
    };
    win11_host::run(kind);
    ExitCode::SUCCESS
}
