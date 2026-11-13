//! Win11 GNOME Calendar (`win11-calendar`): a Windows 11 style calendar app in GTK 4, opened from the taskbar
//! clock's calendar on the day that was clicked.
//!
//! One instance per session (GApplication "io.github.win11shell.Calendar"); a second launch hands its command
//! line or URIs to the running one, which goes to that day and comes to the front.
//!
//!   win11-calendar                          today
//!   win11-calendar --date 2026-11-13        that day (also --date=2026-11-13, or a bare 2026-11-13)
//!   win11-calendar win11-calendar:2026-11-13
//!   win11-calendar --view day|week|month    a view (with or without a date; ?view=day in a URI)
//!   win11-calendar --new [--date ...]       a new event on that day
//!   gio open win11-calendar:2026-11-13      through the desktop file's x-scheme-handler/win11-calendar
//!
//! Environment: WIN11_CALENDAR_GL=1 keeps GTK's GL renderer (the cairo renderer is the default, for RAM).
mod backdrop;
mod config;
mod dates;
mod glyphs;
mod gnome;
mod store;
mod theme;
mod ui;

use gtk::prelude::*;
use gtk::{gio, glib};

pub const APP_ID: &str = "io.github.win11shell.Calendar";
pub const SCHEME: &str = "win11-calendar";

fn main() -> glib::ExitCode {
    if std::env::var_os("GSK_RENDERER").is_none() && std::env::var("WIN11_CALENDAR_GL").as_deref() != Ok("1") {
        // SAFETY: first thing in main, before GTK or any other thread starts.
        unsafe { std::env::set_var("GSK_RENDERER", "cairo") };
    }
    glib::set_prgname(Some(APP_ID));
    let app = gtk::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE | gio::ApplicationFlags::HANDLES_OPEN)
        .build();
    app.connect_startup(|_| {
        config::load();
        glib::set_application_name(config::t("Agenda", "Calendar"));
        theme::install();
    });
    app.connect_command_line(|app, cl| {
        let args: Vec<String> = cl.arguments().iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
        if args.iter().any(|a| a == "--help" || a == "-h") {
            cl.print_literal(
                "win11-calendar [--date YYYY-MM-DD] [--view day|week|month] [--new] [win11-calendar:YYYY-MM-DD]\n",
            );
            return glib::ExitCode::SUCCESS;
        }
        ui::show(app, parse_args(&args));
        glib::ExitCode::SUCCESS
    });
    app.connect_open(|app, files, _hint| {
        let mut req = ui::Request::default();
        for f in files {
            parse_uri(&f.uri(), &mut req);
        }
        ui::show(app, req);
    });
    app.connect_activate(|app| ui::show(app, ui::Request::default()));
    app.run()
}

fn parse_args(args: &[String]) -> ui::Request {
    let mut req = ui::Request::default();
    let mut i = 0;
    while i < args.len() {
        let a = args[i].trim();
        if let Some(v) = a.strip_prefix("--date=") {
            req.date = dates::parse_ymd(v).or(req.date);
        } else if a == "--date" || a == "-d" {
            if let Some(v) = args.get(i + 1) {
                req.date = dates::parse_ymd(v).or(req.date);
                i += 1;
            }
        } else if let Some(v) = a.strip_prefix("--view=") {
            req.view = ui::View::parse(v).or(req.view);
        } else if a == "--view" {
            if let Some(v) = args.get(i + 1) {
                req.view = ui::View::parse(v).or(req.view);
                i += 1;
            }
        } else if a == "--new" {
            req.new_event = true;
        } else if a.contains(':') {
            parse_uri(a, &mut req);
        } else if let Some(d) = dates::parse_ymd(a) {
            req.date = Some(d);
        }
        i += 1;
    }
    req
}

/// win11-calendar:2026-11-13, win11-calendar://2026-11-13, win11-calendar:2026-11-13?view=day,
/// win11-calendar:?date=2026-11-13&view=week (also a file:// or other URI is ignored).
fn parse_uri(uri: &str, req: &mut ui::Request) {
    let Some((scheme, rest)) = uri.trim().split_once(':') else { return };
    if !scheme.eq_ignore_ascii_case(SCHEME) {
        return;
    }
    let rest = rest.trim_start_matches('/');
    let (path, query) = match rest.split_once('?') {
        Some((p, q)) => (p, q),
        None => (rest, ""),
    };
    if let Some(d) = dates::parse_ymd(path.trim_end_matches('/')) {
        req.date = Some(d);
    }
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        match k {
            "date" => req.date = dates::parse_ymd(v).or(req.date),
            "view" => req.view = ui::View::parse(v).or(req.view),
            "new" => req.new_event = v != "0" && v != "false",
            _ => {}
        }
    }
}
