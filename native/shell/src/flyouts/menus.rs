//! Taskbar menus: the app buttons' jump lists and the taskbar's context menu (both in the reusable "jumplist"
//! flyout), Win+X ("winx"), the tray overflow ("tray") and Task View ("taskview"). Ported from
//! git show 418ea594:vendor/ui/js/shell/{taskbar.js openJumpList / taskbarMenuItems, winx.js, tray.js, taskview.js}.
mod menu;
mod taskview;
mod tray;
mod util;
mod winx;

use crate::core::Shell;
use menu::{Icon, Item};
use serde_json::{Value, json};
use std::rc::Rc;

pub fn register(shell: &Rc<Shell>) {
    util::refresh_apps(shell);
    shell.on("apps.changed", |_, _| util::refresh_apps(&crate::core::shell()));
    menu::register(shell);
    winx::register(shell);
    tray::register(shell);
    taskview::register(shell);
}

/// The jump list of an app button: `app_id` its desktop id (None for a window without an app), `windows` its
/// windows (extension windows.changed objects), `x` the button's centre on the monitor.
pub fn jump_list(shell: &Rc<Shell>, app_id: Option<String>, windows: Vec<Value>, x: i32) {
    let host = shell.flyouts();
    if host.is_open("jumplist") {
        host.close();
    }
    let mut items = Vec::new();
    if let Some(id) = app_id.as_deref().filter(|s| !s.is_empty()) {
        let app = util::app(id);
        let name = app
            .as_ref()
            .map(|a| util::str_of(a, "name"))
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| util::app_name(id));
        let icon = app
            .as_ref()
            .map(|a| util::str_of(a, "icon"))
            .filter(|i| !i.is_empty())
            .or_else(|| windows.iter().map(|w| util::str_of(w, "icon")).find(|i| !i.is_empty()))
            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned());
        let launch_id = id.to_owned();
        items.push(
            Item::new(name, Icon::App(icon), move || util::launch(&crate::core::shell(), &launch_id))
                .named("menus-jump-launch"),
        );
        let pin_id = id.to_owned();
        if util::is_pinned(shell, id) {
            items.push(
                Item::new(shell.t("Losmaken van taakbalk", "Unpin from taskbar"), Icon::Glyph("unpin"), move || {
                    util::unpin(&crate::core::shell(), &pin_id)
                })
                .named("menus-jump-unpin"),
            );
        } else {
            items.push(
                Item::new(shell.t("Vastmaken aan taakbalk", "Pin to taskbar"), Icon::Glyph("pin"), move || {
                    util::pin(&crate::core::shell(), &pin_id)
                })
                .named("menus-jump-pin"),
            );
        }
    }
    let ids: Vec<Value> = windows.iter().map(|w| w["id"].clone()).filter(|id| !id.is_null()).collect();
    if !ids.is_empty() {
        let label = if ids.len() > 1 {
            shell.t("Alle vensters sluiten", "Close all windows")
        } else {
            shell.t("Venster sluiten", "Close window")
        };
        items.push(
            Item::new(label, Icon::Glyph("close"), move || {
                let shell = crate::core::shell();
                for id in &ids {
                    shell.gnome.send("windows.close", json!({"id": id}));
                }
            })
            .named("menus-jump-close"),
        );
    }
    if items.is_empty() {
        return;
    }
    menu::show(shell, items, x, true, None);
}

/// The taskbar's own context menu (right click on empty space): Taakbeheer, Taakbalkinstellingen.
pub fn taskbar_menu(shell: &Rc<Shell>, x: i32) {
    let tm_icon = util::app("win11shell-taskmgr.desktop")
        .map(|a| util::str_of(&a, "icon"))
        .filter(|i| !i.is_empty())
        .unwrap_or_else(|| "win11shell-taskmgr".into());
    let items = vec![
        Item::new(shell.t("Taakbeheer", "Task Manager"), Icon::App(tm_icon), || {
            util::open_app(&crate::core::shell(), "taskmgr", json!({}))
        })
        .named("menus-taskbar-taskmgr"),
        Item::new(shell.t("Taakbalkinstellingen", "Taskbar settings"), Icon::Glyph("settings"), || {
            util::open_settings(&crate::core::shell(), "taskbar")
        })
        .named("menus-taskbar-settings"),
    ];
    menu::show(shell, items, x, false, None);
}

/// Win+X with its left edge at `x` (the Start button's left edge), `mouse` true for a right click on Start (no
/// access key underlines). Toggles.
#[allow(dead_code)]
pub fn open_winx(shell: &Rc<Shell>, x: Option<i32>, mouse: bool) {
    winx::open(shell, x, mouse);
}

/// The tray overflow centred over the chevron at `x`. Toggles.
#[allow(dead_code)]
pub fn open_tray(shell: &Rc<Shell>, x: Option<i32>) {
    tray::open(shell, x);
}

/// The tray icons split as the taskbar shows them: (hiddenMenu, on the taskbar, in the overflow), from tray.list
/// items and the settings taskbar.tray.
#[allow(dead_code)]
pub fn tray_arrange(shell: &Shell, items: &[Value]) -> (bool, Vec<Value>, Vec<Value>) {
    tray::arrange(shell, items)
}

/// A left click on a tray icon at (x, y) on the monitor (tray.activate, or the menu).
#[allow(dead_code)]
pub fn tray_click(shell: &Rc<Shell>, item: &Value, x: i32, y: i32) {
    tray::click(shell, item, x, y, false);
}

/// A tray icon's menu at (x, y): its dbusmenu in the menu flyout, or the app's own (tray.contextMenu).
#[allow(dead_code)]
pub fn tray_menu(shell: &Rc<Shell>, item: &Value, x: i32, y: i32) {
    tray::item_menu(shell, item, x, y);
}

/// A menu of your own in the jumplist flyout: (label, glyph name, action) rows, None for a line; centred over `x`
/// or starting at it (clock and system tray context menus).
#[allow(dead_code)]
pub fn context_menu(shell: &Rc<Shell>, rows: Vec<Option<(String, &'static str, Rc<dyn Fn()>)>>, x: i32, center: bool) {
    let items = rows
        .into_iter()
        .map(|r| match r {
            Some((label, glyph, action)) => {
                let mut item = Item::plain(label);
                item.icon = if glyph.is_empty() { Icon::None } else { Icon::Glyph(glyph) };
                item.action = Some(action);
                item
            }
            None => Item::sep(),
        })
        .collect();
    menu::show(shell, items, x, center, None);
}
