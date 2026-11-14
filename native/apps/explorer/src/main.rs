//! Win11 GNOME File Explorer in GTK 4 (`win11-gtk-explorer`, GApplication io.github.win11shell.Explorer;
//! ../../APPS.md). The native port of last-webkit:vendor/ui/js/apps/explorer/ on the shared kit (native/kit).
//!
//! One process, many windows: every launch opens a new window (a second `win11-gtk-explorer` is handed to the
//! running process by GApplication).
//!
//!   win11-gtk-explorer                          Start (or the saved tabs, transfer.rs)
//!   win11-gtk-explorer PATH|URI [PATH|URI...]   the first in the window, the others in more tabs; a file
//!                                               opens its folder with the file selected; place words work too
//!                                               (thispc, trash, gallery, documents, trash:///, computer:///)
//!   win11-gtk-explorer --params JSON            the WebKit app's URL params: {path, place, select, tabs,
//!                                               properties, picker, open} (last-webkit host lib.rs run_inner)
//!   win11-gtk-explorer --tab-ticket T           a tab moved out of another window (transfer.rs)
//!   win11-gtk-explorer --picker-stdio           the portal's file picker, its own process (picker.rs)
//!   win11-gtk-explorer --restart-sessions PID   after a restart (shell.restartAll): every window again with its
//!                                               tabs, from the file the restarting image PID left (transfer.rs)
//!
//! Restarts: on `shell.restartAll` the backend ends with a restart note (kit backend.rs) once Explorer's own file
//! operations are done (host lib.rs http_only_shutdown); `restart_plan` keeps the windows' tabs and execs the new
//! build with `--restart-sessions`. A picker stays on a new backend (its request is on stdin).
//!
//! # Modules
//!
//! The core (this builder): `explorer` (one window: state, tabs, navigation, loading, rendering, keyboard,
//! settings, live updates; main.js), `fileview` (the virtualized file list), `tabs` (the strip in the title
//! bar), `toolbar` (address row and command bar), `navpane`, `pages` (Start, Deze pc, Galerij), `menus`
//! (context and command bar menus), `model` (locations and tabs), `util`, `i18n` (every text, nl and en),
//! `actbox` (a box with an activate signal: navigation rows, tabs and page items, so the dev channel's "click"
//! reaches them by name: "nav-<id>", "tab-<id>", "explorer-tile-<folder>", "explorer-drive-<mount>").
//!
//! Widget names for the harness: "explorer-tabs", "explorer-new-tab", "explorer-back" / "-forward" / "-up" /
//! "-refresh", "explorer-address" (+ "-input" while editing), "crumb-<n>", "explorer-search" (+ "-input"),
//! "explorer-commands", "explorer-cmd-<new|cut|copy|paste|rename|share|delete|sort|view|more|empty-trash|restore|
//! details>", "explorer-nav", "explorer-view" (the list) with "explorer-items" (its focusable canvas) and
//! "explorer-columns", "explorer-rename" (the rename box), "explorer-page", "explorer-sec-<id>",
//! "explorer-status" (+ "-details", "-large"), "explorer-details"; menu items "explorer-view-<mode>",
//! "explorer-new-folder", "explorer-new-text", "explorer-open", "explorer-properties", "explorer-restore",
//! "explorer-empty-trash", "explorer-options".
//!
//! # For builder B: ops.rs, dialogs.rs, details.rs, dnd.rs
//!
//! Replace the bodies of the stubs; keep their signatures (the core calls them). Each file's header says what
//! the WebKit app did there.
//!
//! - `ops::open_file(x, &item)`, `rename_item(x, &item, new_name, done)` (inline rename: `done(Ok(()))` closes
//!   the box, `done(Err(msg))` keeps it open with the message), `properties(x, items)` (empty: the folder or
//!   the Prullenbak shown), `new_folder(x)`, `new_text(x)`, `cut(x)`, `copy(x)`, `paste(x, dest)`, `trash(x)`,
//!   `trash_paths(x, paths)`, `delete_permanently(x, items)` (Shift+Delete), `undo(x)`, `compress(x, items)`,
//!   `extract(x, &item)`, `restore(x, items)` (None: selection or all), `empty_trash(x)`,
//!   `open_with_items(x, &item, done(Vec<MenuItem>))` (the "Openen met" submenu), `drop_to(x, paths, dest,
//!   copy)`.
//! - `dialogs::show_properties(x, items)`, `show_drive_properties(x, &volume)`, `show_trash_properties(x)`,
//!   `show_share(x, items)`, `show_options(x)`.
//! - `details::DetailsPane` (`new(WeakExplorer)`, `widget()`, `set_mode(PaneMode)`, `mode()`, `update()`,
//!   `render()`).
//! - `dnd::attach_view(x, &FileView)`, `attach_nav_row(x, &row, &loc, expand)`, `attach_crumb(x, &w, &loc)`,
//!   `attach_tab(x, &w, tab_id)`, `attach_page_item(x, &w, &item, drag, drop)`; the file view's drag and drop
//!   API is at the top of fileview.rs.
//!
//! # For builder C: picker.rs, transfer.rs
//!
//! - `picker::prepare(argv)` (first in main: a picker launch gets its own application id and its stdin
//!   request), `picker::launch_params(p)`, `picker::attach(x, &options)` (by `Explorer::new` before the window
//!   shows, for `--params {"picker": {...}}` or `--picker-stdio`); hooks `Explorer::set_picker(Picker {..})`,
//!   `set_footer(..)`, `is_picker()`. A picker window remembers its size under the key "picker".
//! - `transfer::attach_tab(x, &w, id)`, `attach_strip(x, &strip)`, `claim_ticket(x, ticket)` (at start for
//!   `--tab-ticket`; the window has a placeholder Start tab that `import_tab` replaces), `restored_session()`
//!   (a window opened without arguments), `save_session(x)` (on close), `normalize_tab_state(&v)`.
//!
//! # The window's API (`explorer::Explorer`, a cheap handle; `WeakExplorer` for closures)
//!
//! - State: `ctx()`, `window()`, `anchor()` (parent widget for dialogs, toasts, menus), `state()` (places,
//!   home, user, volumes, clipboard, trash_full), `cfg(key)` / `cfg_bool` / `cfg_str` / `set_cfg(key, v)`
//!   (explorer.*), `set_setting(path, v)`, `privacy_recent()`, `call(method, args, done(&x, result))`.
//! - Tabs: `active_id()`, `tab_ids()`, `tab(id)`, `active_tab()`, `loc()`, `active_items()`, `open_tab(loc,
//!   activate)`, `close_tab(id)`, `activate_tab(id)`, `export_tab(id)`, `session_state()`, `import_tab(state,
//!   before, done)`, `rollback_imported_tab(id, prev)`, `move_tab_before(id, before)`.
//! - Navigation: `navigate(loc, NavOpts)`, `go_to(loc)`, `back`, `forward`, `up`, `refresh`, `reload(id,
//!   delay)`, `reload_active(delay)`, `select_after_reload(paths, rename)`, `open_item(&item, focus)`,
//!   `open_location(&item)`, `open_alles(query)`.
//! - Selection: `selected_items()`, `set_selection(paths, focus)`, `view()` (the FileView of a listing),
//!   `focus_view()`, `cur_dir()`.
//! - Messages: `toast(title, ToastOpts)`, `show_error(msg, title)`, `check_dry(&result)`, `infobars()` (where
//!   job cards go), `push_undo(label, f)` / `pop_undo()` / `has_undo()`, `apply_clipboard(&v)`,
//!   `load_clipboard()`, `load_trash_info()`, `invalidate_dir(path)`, `load_volumes(done)`.
//! - Names: `folder_label(path)`, `loc_label(&loc, full)`, `friendly_dir(dir)`, `Explorer::orig_dir(&item)`,
//!   `item_name(&item)`, `icon_name_for(path)`, `loc_icon(&loc, px)`, `stored_on(path)`, `place_path(id)`.
//!
//! # Environment
//!
//! The kit's: WIN11_APP_BACKEND=port:token (a running backend), WIN11_LANG / WIN11_THEME (screenshots),
//! WIN11_APP_GL=1 (GL renderer), WIN11_DEV_COMMAND / WIN11_DEV_REPORT (harness; commands carry
//! "app": "explorer").
mod actbox;
mod details;
mod dialogs;
mod dnd;
mod explorer;
mod fileview;
mod i18n;
mod menus;
mod model;
mod navpane;
mod ops;
mod pages;
mod picker;
mod tabs;
mod toolbar;
mod transfer;
mod util;

use explorer::{Explorer, Params};
use gtk::glib;
use serde_json::Value;

const SPEC: kit::AppSpec = kit::AppSpec::new("explorer", "io.github.win11shell.Explorer", "Verkenner", "File Explorer")
    .icon("folder")
    .usage(
        "win11-gtk-explorer [PATH|URI ...] | --params JSON | --tab-ticket TICKET | --picker-stdio\n\
         \n\
         Opens a File Explorer window (one process, a window per launch).\n\
         PATH|URI     a folder (a file opens its folder with the file selected), file:// URIs, trash:///,\n\
         \x20            computer:///, or a place: home, gallery, thispc, trash, desktop, documents, downloads,\n\
         \x20            pictures, music, videos; more than one opens more tabs\n\
         --params     {\"path\", \"place\", \"select\", \"tabs\", \"properties\", \"picker\", \"open\"}\n\
         --tab-ticket a tab moved out of another Explorer window\n\
         --picker-stdio  the file picker of the desktop portal: the request on stdin, the answer on stdout",
    );

fn truthy(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() == Some(1.0),
        Value::String(s) => matches!(s.to_lowercase().as_str(), "1" | "true" | "yes" | "ja"),
        _ => false,
    }
}

/// The WebKit app's params object (also app.navigate's) as `Params`.
pub(crate) fn params_from_json(v: &Value) -> Params {
    let text = |k: &str| match &v[k] {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    let picker = match &v["picker"] {
        Value::Object(_) => Some(v["picker"].clone()),
        Value::String(s) => serde_json::from_str::<Value>(s).ok().filter(Value::is_object),
        _ => None,
    };
    Params {
        path: text("path"),
        place: text("place"),
        select: v.get("select").filter(|s| !s.is_null()).cloned(),
        tabs: v.get("tabs").filter(|s| !s.is_null()).cloned(),
        properties: truthy(&v["properties"]),
        tab_ticket: text("tabTicket"),
        picker,
        open: text("open"),
        session: None,
    }
}

/// argv[1..] of a launch as `Params`.
fn parse_args(args: &[String]) -> Params {
    let mut it = args.iter().peekable();
    let mut positional: Vec<String> = Vec::new();
    let mut params = Params::default();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--tab-ticket" => params.tab_ticket = it.next().cloned().filter(|t| !t.is_empty()),
            "--params" => {
                if let Some(raw) = it.next()
                    && raw.len() <= 1024 * 1024
                    && let Ok(v) = serde_json::from_str::<Value>(raw)
                    && v.is_object()
                {
                    params = params_from_json(&v);
                }
            }
            // after a restart: main.rs restored_windows took them already
            "--restart-sessions" => {
                it.next();
            }
            // flags the host may pass along that mean nothing here; --picker-stdio is picker.rs's (main)
            "--dry-run" | "--http-only" | "--picker-stdio" | "--" => {}
            other if other.starts_with("--") => eprintln!("win11-gtk-explorer: unknown option {other}"),
            other => positional.push(other.to_owned()),
        }
    }
    if params.path.is_none() && params.place.is_none() && !positional.is_empty() {
        params.path = Some(positional[0].clone());
        if positional.len() > 1 {
            params.tabs = Some(Value::Array(positional[1..].iter().cloned().map(Value::String).collect()));
        }
    }
    params
}

fn main() -> glib::ExitCode {
    // a picker is its own process with its own application id (picker.rs)
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let spec = match picker::prepare(&argv) {
        Ok(Some(id)) => kit::AppSpec { id, ..SPEC },
        Ok(None) => SPEC,
        Err(e) => {
            eprintln!("win11-gtk-explorer: {e}");
            return glib::ExitCode::FAILURE;
        }
    };
    kit::run(spec, |_ctx, args, first| {
        if first {
            kit::theme::install_app_css(include_str!("../css/explorer.css"));
            kit::on_restart(restart_plan);
        }
        // after a restart: every window of the restarting image again (the first one synchronously)
        let sessions = restart_pid(&args).map(transfer::take_restart_sessions).unwrap_or_default();
        if !sessions.is_empty() {
            for session in sessions {
                Explorer::new(Params { session: Some(session), ..Default::default() });
            }
            return;
        }
        Explorer::new(picker::launch_params(parse_args(&args)));
    })
}

/// The pid of `--restart-sessions PID`.
fn restart_pid(args: &[String]) -> Option<&str> {
    args.windows(2).find(|w| w[0] == "--restart-sessions").map(|w| w[1].as_str())
}

/// The backend restarts (kit `on_restart`): a picker stays on a new backend; otherwise every window's tabs are kept
/// (transfer.rs save_for_restart, the active window last) and the new build opens them again; without a window
/// there is nothing to reopen. A plain shell.restart keeps the windows on a new backend.
fn restart_plan(restart: &kit::Restart) -> kit::RestartPlan {
    let mut windows: Vec<Explorer> = Explorer::all().into_iter().filter(|x| !x.is_picker()).collect();
    if !restart.all || Explorer::all().iter().any(Explorer::is_picker) {
        return kit::RestartPlan::Stay;
    }
    if windows.is_empty() {
        return kit::RestartPlan::Quit;
    }
    if let Some(active) = Explorer::active()
        && let Some(i) = windows.iter().position(|x| x.window().window() == active.window().window())
    {
        let active = windows.remove(i);
        windows.push(active);
    }
    match transfer::save_for_restart(&windows) {
        Some(pid) => kit::RestartPlan::Exec(vec!["--restart-sessions".into(), pid.to_string()]),
        // the saved session of window-state.json comes back
        None => kit::RestartPlan::Exec(Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args() {
        let p = parse_args(&["/a".into(), "/b".into()]);
        assert_eq!(p.path.as_deref(), Some("/a"));
        assert_eq!(p.tabs, Some(serde_json::json!(["/b"])));
        let p = parse_args(&["--params".into(), r#"{"place":"trash","properties":"1"}"#.into()]);
        assert_eq!(p.place.as_deref(), Some("trash"));
        assert!(p.properties);
        let p = parse_args(&["--tab-ticket".into(), "abc".into()]);
        assert_eq!(p.tab_ticket.as_deref(), Some("abc"));
    }

    #[test]
    fn restart_arguments() {
        let args: Vec<String> = vec!["--restart-sessions".into(), "4242".into()];
        assert_eq!(restart_pid(&args), Some("4242"));
        // not a location of its own: a window without the sessions file gets the saved session
        let p = parse_args(&args);
        assert!(!p.explicit() && p.path.is_none() && p.tabs.is_none());
        assert_eq!(restart_pid(&["/a".into()]), None);
        assert!(transfer::take_restart_sessions("not-a-pid").is_empty());
        assert!(transfer::take_restart_sessions("0").is_empty());
    }
}
