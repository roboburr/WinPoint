//! Generic entry point for the existing Settings, Task Manager, Alles and Disks UIs.
//! The shared library removes this `--app <name>` prefix before reading UI parameters.

use std::{ffi::OsStr, process::ExitCode};

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_deref() == Some(OsStr::new("--help")) {
        println!("Gebruik: win11-app --app settings|taskmgr|alles|disks [--params JSON]");
        return ExitCode::SUCCESS;
    }
    if first.as_deref() != Some(OsStr::new("--app")) {
        eprintln!("Gebruik: win11-app --app settings|taskmgr|alles|disks [--params JSON]");
        return ExitCode::from(2);
    }
    let kind: &'static str = match args.next().as_deref().and_then(OsStr::to_str) {
        Some("settings") => "settings",
        Some("taskmgr") => "taskmgr",
        Some("alles") => "alles",
        Some("disks") => "disks",
        Some("defaultapps") => "defaultapps",
        Some("installer") => "installer",
        _ => {
            eprintln!("Kies een bestaande app: settings, taskmgr, alles of disks.");
            return ExitCode::from(2);
        }
    };
    win11_host::run(kind);
    ExitCode::SUCCESS
}
