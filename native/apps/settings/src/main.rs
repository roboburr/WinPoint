//! Win11 GNOME Settings in GTK 4 (`win11-gtk-settings`, see ../../APPS.md): the WebKit app
//! 6ebf169f:vendor/ui/js/apps/settings/ rebuilt natively on win11-kit, with the same pages, backend and look.
//!
//! One process (GApplication "io.github.win11shell.Settings"); a second launch hands its arguments to the
//! running one, which opens that page and comes to the front.
//!
//!   win11-gtk-settings                                  Startpagina
//!   win11-gtk-settings --params '{"page":"display"}'    a page (an id or alias of registry.rs), as
//!                                                       `win11-app --app settings --params ..` passes it
//!   win11-gtk-settings --params '{"page":"bluetooth","add":1}'   page parameters (host/src/app_params.rs)
//!   win11-gtk-settings --page display                   the same, for hand use (also a bare page id)
//!
//! The backend is `win11-app --app settings --http-only` (the kit starts it; WIN11_APP_BACKEND=port:token
//! names a running one). Recently opened pages are kept in $XDG_CONFIG_HOME/win11-gnome/settings-app.json.
//!
//! While an update job of ours runs, closing the window keeps the process (keepalive.rs); a launch then opens a
//! new window. On `shell.restartAll` the new build opens on the update page when this Settings asked for the
//! restart (the origin), else on the page it showed (`restart_plan`); without a window it quits.
mod app;
// The shared toolbox of every page (common.js, colorpicker.js and the previews): some items are there for the
// pages of builders B and C and have no caller yet.
#[allow(dead_code)]
mod colorpicker;
#[allow(dead_code)]
mod common;
mod keepalive;
mod navicons;
mod pages;
#[allow(dead_code)]
mod preview;
mod registry;
#[allow(dead_code)]
mod widgets;

use kit::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

const SPEC: kit::AppSpec = kit::AppSpec::new("settings", "io.github.win11shell.Settings", "Instellingen", "Settings")
    .icon("settings")
    .usage(
        "win11-gtk-settings [--params JSON] [--page ID]\n\
         \n\
         \x20 --params JSON   page parameters as win11-app passes them: {\"page\":\"display\"}, {\"page\":\"search\",\"q\":\"wifi\"}\n\
         \x20 --page ID       open this page (an id or alias: display, sound, bluetooth, wifi, update, ...)\n\
         \n\
         A second launch opens the page in the running window.",
    );

thread_local! {
    static ROUTER: RefCell<Option<Rc<app::Router>>> = const { RefCell::new(None) };
}

/// The page and its parameters of a command line: `--params JSON` (its "page" key), `--page ID` or a bare id.
fn parse_args(args: &[String]) -> (Option<String>, Value) {
    let mut page = None;
    let mut params = json!({});
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--params" || a.starts_with("--params=") {
            let raw = match a.strip_prefix("--params=") {
                Some(v) => Some(v.to_owned()),
                None => {
                    i += 1;
                    args.get(i).cloned()
                }
            };
            if let Some(mut obj) = raw
                .filter(|r| r.len() <= 64 * 1024)
                .and_then(|r| serde_json::from_str::<Value>(&r).ok())
                .filter(Value::is_object)
            {
                if let Some(map) = obj.as_object_mut()
                    && let Some(p) = map.remove("page")
                {
                    page = match p {
                        Value::String(s) => Some(s),
                        Value::Null => None,
                        other => Some(other.to_string()),
                    };
                }
                params = obj;
            }
        } else if a == "--page" {
            i += 1;
            page = args.get(i).cloned();
        } else if let Some(v) = a.strip_prefix("--page=") {
            page = Some(v.to_owned());
        } else if !a.starts_with('-') && page.is_none() {
            page = Some(a.trim_start_matches("ms-settings:").to_owned());
        }
        i += 1;
    }
    (page.filter(|p| !p.trim().is_empty()), params)
}

/// The router whose window is open (after the keep-alive held the process without a window there is none).
fn open_router() -> Option<Rc<app::Router>> {
    let router = ROUTER.with(|r| r.borrow().clone())?;
    let app = kit::ctx().app().clone();
    let window: &gtk::Window = router.window().window().upcast_ref();
    app.windows().iter().any(|w| w == window).then_some(router)
}

/// The backend restarts (kit `on_restart`): `shell.restartAll` reopens the window on the update page when this
/// Settings asked for it (the WebKit app's rule, 6ebf169f host lib.rs restart_args), else on the page it shows;
/// a Settings that only the keep-alive kept has nothing to reopen. A plain shell.restart stays on a new backend.
fn restart_plan(restart: &kit::Restart) -> kit::RestartPlan {
    if !restart.all {
        return kit::RestartPlan::Stay;
    }
    let Some(router) = open_router() else { return kit::RestartPlan::Quit };
    let (page, params) = if restart.origin { ("update", json!({})) } else { router.current().unwrap_or(("home", json!({}))) };
    let mut params = if params.is_object() { params } else { json!({}) };
    params["page"] = json!(page);
    kit::RestartPlan::Exec(vec!["--params".into(), params.to_string()])
}

fn main() -> glib::ExitCode {
    kit::run(SPEC, |ctx, args, first| {
        let (page, params) = parse_args(&args);
        if first {
            kit::theme::install_app_css(&format!("{}\n{}", include_str!("../css/settings.css"), pages::css()));
            common::load_shell_info();
            kit::on_restart(restart_plan);
            keepalive::install(ctx);
        } else if let Some(r) = open_router() {
            // a relaunch navigates the running window (frame.onAppNavigate)
            if let Some(p) = page {
                r.show(&p, params, app::Show::default());
            }
            r.window().present();
            return;
        }
        // the first launch, or a launch while the update keep-alive kept the process without a window
        let start = page.as_deref().and_then(registry::resolve).unwrap_or("home");
        let router = app::build(start, params);
        ROUTER.with(|r| *r.borrow_mut() = Some(router));
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_restart_argv_reopens_its_page() {
        // restart_plan's arguments, as the new image parses them
        let update = vec!["--params".to_string(), json!({"page": "update"}).to_string()];
        assert_eq!(parse_args(&update), (Some("update".into()), json!({})));
        let search = vec!["--params".to_string(), json!({"page": "search", "q": "wifi"}).to_string()];
        assert_eq!(parse_args(&search), (Some("search".into()), json!({"q": "wifi"})));
    }
}
