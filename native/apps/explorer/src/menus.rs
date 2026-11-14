//! The menus (main.js `menus`): Nieuw, Sorteren, Weergeven and "..." of the command bar, the column header's
//! menu, and the context menus of items (with Explorer's icon row on top), the background and the
//! navigation pane.
use crate::explorer::Explorer;
use crate::fileview::VIEW_MODES;
use crate::i18n::s;
use crate::model::Loc;
use crate::util::{self, Item, flag, st};
use crate::{dialogs, ops};
use kit::controls::{MenuItem, TopAction, show_menu_with_top};
use serde_json::{Value, json};

/// Show a menu at (x, y) of `anchor`.
pub fn show(anchor: &gtk::Widget, x: f64, y: f64, top: Vec<TopAction>, items: Vec<MenuItem>) {
    if top.is_empty() && items.is_empty() {
        return;
    }
    show_menu_with_top(anchor, x, y, top, items);
}

fn act(x: &Explorer, f: impl Fn(&Explorer) + 'static) -> impl Fn() + 'static {
    let w = x.weak();
    move || {
        if let Some(x) = w.upgrade() {
            f(&x);
        }
    }
}

/// Nieuw: Map, Tekstdocument.
pub fn new_menu(x: &Explorer) -> Vec<MenuItem> {
    vec![
        MenuItem::new(s("newFolder"), act(x, ops::new_folder)).glyph("folder").accel("Ctrl+Shift+N").name("explorer-new-folder"),
        MenuItem::separator(),
        MenuItem::new(s("newText"), act(x, ops::new_text)).glyph("document").name("explorer-new-text"),
    ]
}

/// The glyph per view mode (the WebKit menu draws its own small grids; these are the nearest glyphs).
fn view_glyph(m: &str) -> &'static str {
    match m {
        "xl" => "image",
        "large" | "medium" => "grid",
        "small" => "apps",
        "list" => "list",
        "details" => "properties",
        "tiles" => "taskview",
        _ => "summary",
    }
}

fn view_items(x: &Explorer) -> Vec<MenuItem> {
    let cur = x.cfg_str("view", "details");
    VIEW_MODES
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let mode = (*m).to_owned();
            MenuItem::new(s(if *m == "details" { "detailsView" } else { m }), act(x, move |x| x.set_view(&mode)))
                .radio(cur == *m)
                .glyph(view_glyph(m))
                .accel(&format!("Ctrl+Shift+{}", i + 1))
                .name(&format!("explorer-view-{m}"))
        })
        .collect()
}

fn group_items(x: &Explorer, kind: &str) -> Vec<MenuItem> {
    let cur = x.cfg_str("groupBy", "none");
    let keys: &[&str] = if kind == "trash" { &["name", "deleted", "size"] } else { &["name", "modified", "type", "size"] };
    let label = |k: &str| match k {
        "name" => s("colName"),
        "modified" => s("colModified"),
        "type" => s("colType"),
        "size" => s("colSize"),
        _ => s("colDeleted"),
    };
    let mut out: Vec<MenuItem> = keys
        .iter()
        .map(|k| {
            let key = (*k).to_owned();
            MenuItem::new(label(k), act(x, move |x| x.set_group(&key))).radio(cur == *k)
        })
        .collect();
    out.push(MenuItem::separator());
    out.push(MenuItem::new(s("none"), act(x, |x| x.set_group("none"))).radio(cur == "none"));
    out
}

/// Sorteren: the columns, Oplopend / Aflopend and Groeperen op.
pub fn sort_menu(x: &Explorer) -> Vec<MenuItem> {
    let kind = x.loc().kind();
    let (key, desc) = x.sort_state(kind);
    let mut out: Vec<MenuItem> = x
        .sort_keys(kind)
        .into_iter()
        .map(|(id, label)| {
            let on = key == id;
            MenuItem::new(&label, act(x, move |x| x.set_cfg("sortBy", json!(id)))).radio(on)
        })
        .collect();
    out.push(MenuItem::separator());
    out.push(MenuItem::new(s("asc"), act(x, |x| x.set_sort_dir(false))).radio(!desc));
    out.push(MenuItem::new(s("desc"), act(x, |x| x.set_sort_dir(true))).radio(desc));
    out.push(MenuItem::separator());
    out.push(MenuItem::submenu(s("groupBy"), group_items(x, kind)).glyph("list"));
    out
}

fn show_items(x: &Explorer) -> Vec<MenuItem> {
    let toggle = |key: &'static str| act(x, move |x| x.toggle_cfg(key));
    vec![
        MenuItem::new(s("navPane"), toggle("navPane")).checked(x.cfg_bool("navPane", true)),
        MenuItem::new(s("detailsPane"), toggle("detailsPane")).checked(x.cfg_bool("detailsPane", false)).accel("Alt+Shift+P"),
        MenuItem::new(s("previewPane"), toggle("previewPane"))
            .checked(!x.cfg_bool("detailsPane", false) && x.cfg_bool("previewPane", false))
            .accel("Alt+P"),
        MenuItem::separator(),
        MenuItem::new(s("checkboxes"), toggle("checkboxes")).checked(x.cfg_bool("checkboxes", false)),
        MenuItem::new(s("extensions"), toggle("showExtensions")).checked(x.cfg_bool("showExtensions", true)),
        MenuItem::new(s("hidden"), toggle("showHidden")).checked(x.cfg_bool("showHidden", false)),
    ]
}

/// Weergeven: the view modes, Compacte weergave and Weergeven >.
pub fn view_menu(x: &Explorer) -> Vec<MenuItem> {
    let mut out = view_items(x);
    out.push(MenuItem::separator());
    out.push(MenuItem::new(s("compact"), act(x, |x| x.toggle_cfg("compact"))).checked(x.cfg_bool("compact", false)));
    out.push(MenuItem::separator());
    out.push(MenuItem::submenu(s("show"), show_items(x)).glyph("eye"));
    out
}

/// "...": selection, Eigenschappen, Opties.
pub fn more_menu(x: &Explorer) -> Vec<MenuItem> {
    let fileish = x.view().is_some();
    let pages = !x.page_items().is_empty();
    let has_sel = x.active_tab().is_some_and(|t| !t.borrow().selection.is_empty());
    vec![
        MenuItem::new(s("selectAll"), act(x, |x| x.select_all())).glyph("check").accel("Ctrl+A").enabled(fileish || pages),
        MenuItem::new(s("selectNone"), act(x, |x| x.select_none())).glyph("close").enabled(has_sel),
        MenuItem::new(s("invert"), act(x, |x| x.invert_selection())).glyph("refresh").enabled(fileish),
        MenuItem::separator(),
        MenuItem::new(s("properties"), act(x, |x| ops::properties(x, x.selected_items()))).glyph("properties").accel("Alt+Enter"),
        MenuItem::new(s("options"), act(x, dialogs::show_options)).glyph("settings").name("explorer-options"),
    ]
}

/// The column header's menu: which columns show.
pub fn columns_menu(x: &Explorer) -> Vec<MenuItem> {
    let hidden: Vec<String> = x.cfg("hiddenColumns").as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
    let toggle = |id: &'static str| {
        act(x, move |x| {
            let mut h: Vec<String> = x.cfg("hiddenColumns").as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect()).unwrap_or_default();
            if let Some(i) = h.iter().position(|c| c == id) {
                h.remove(i);
            } else {
                h.push(id.to_owned());
            }
            x.set_cfg("hiddenColumns", json!(h));
        })
    };
    let on = |id: &str| !hidden.iter().any(|c| c == id);
    vec![
        MenuItem::new(s("colName"), || {}).checked(true).enabled(false),
        MenuItem::new(s("colModified"), toggle("modified")).checked(on("modified")),
        MenuItem::new(s("colType"), toggle("type")).checked(on("type")),
        MenuItem::new(s("colSize"), toggle("size")).checked(on("size")),
    ]
}

/// The context menu of the selected items; `done(top, items)` (Openen met is filled in first, at most
/// 450 ms, ops.rs).
pub fn item_menu(x: &Explorer, items: Vec<Item>, done: impl FnOnce(Vec<TopAction>, Vec<MenuItem>) + 'static) {
    if items.is_empty() {
        return;
    }
    let loc = x.loc();
    let kind = loc.kind();
    let one = if items.len() == 1 { Some(items[0].clone()) } else { None };
    if loc == Loc::Trash {
        let (a, b) = (items.clone(), items.clone());
        done(
            Vec::new(),
            vec![
                MenuItem::new(s("restore"), act(x, move |x| ops::restore(x, Some(a.clone())))).glyph("undo").name("explorer-restore"),
                MenuItem::separator(),
                MenuItem::new(s("properties"), act(x, move |x| ops::properties(x, b.clone()))).glyph("properties").accel("Alt+Enter"),
            ],
        );
        return;
    }
    let fileish = loc.is_fileish();
    let is_vol = one.as_ref().is_some_and(|o| o.get("volume").is_some_and(Value::is_object));
    let all = items.clone();
    let top = vec![
        TopAction::new("cut", s("cutMenu"), act(x, ops::cut)).enabled(fileish),
        TopAction::new("copy", s("copyMenu"), act(x, ops::copy)).enabled(!is_vol),
        TopAction::new("rename", s("renameMenu"), act(x, |x| x.rename_selected())).enabled(fileish && one.is_some()),
        TopAction::new("share", s("share"), act(x, move |x| dialogs::show_share(x, all.clone()))).enabled(!is_vol),
        TopAction::new("delete", s("deleteMenu"), act(x, ops::trash)).enabled((fileish || kind == "gallery") && !is_vol),
    ];
    let top = if top.iter().all(|a| !a.enabled) { Vec::new() } else { top };
    let mut list: Vec<MenuItem> = vec![MenuItem::new(s("open"), act(x, |x| x.open_selection())).glyph("open").accel("Enter").name("explorer-open")];
    if items.iter().all(|i| flag(i, "isDir")) {
        let its = items.clone();
        list.push(MenuItem::new(s("openNewTab"), act(x, move |x| x.open_new_tabs(&its))).glyph("window"));
    }
    let open_with_at = match &one {
        Some(o) if !flag(o, "isDir") => {
            list.push(MenuItem::submenu(s("openWith"), vec![MenuItem::new(s("loading"), || {}).enabled(false)]).glyph("apps"));
            Some(list.len() - 1)
        }
        _ => None,
    };
    if let Some(o) = &one
        && ((kind == "home" && !flag(o, "isDir")) || kind == "search")
    {
        let it = o.clone();
        list.push(MenuItem::new(s("openLocation"), act(x, move |x| x.open_location(&it))).glyph("folder-open"));
    }
    list.push(MenuItem::separator());
    if let Some(o) = &one {
        let p = st(o, "path").to_owned();
        if flag(o, "isDir") && !is_vol {
            let pinned = x.is_pinned(&p);
            list.push(
                MenuItem::new(s(if pinned { "unpin" } else { "pin" }), act(x, move |x| if pinned { x.unpin(&p) } else { x.pin(&p) }))
                    .glyph(if pinned { "unpin" } else { "pin" }),
            );
        } else if !flag(o, "isDir") {
            let fav = x.is_fav(&p);
            list.push(MenuItem::new(s(if fav { "unfav" } else { "fav" }), act(x, move |x| if fav { x.unfav(&p) } else { x.fav(&p) })).glyph("star"));
        }
    }
    if fileish && !is_vol {
        let its = items.clone();
        list.push(MenuItem::new(s("compress"), act(x, move |x| ops::compress(x, its.clone()))).glyph("clipboard"));
    }
    if let Some(o) = &one
        && !flag(o, "isDir")
        && util::is_archive(st(o, "name"))
    {
        let it = o.clone();
        list.push(MenuItem::new(s("extract"), act(x, move |x| ops::extract(x, &it))).glyph("folder-open"));
    }
    let vals: Vec<Value> = items.iter().map(|i| (**i).clone()).collect();
    list.push(MenuItem::new(s("copyPath"), act(x, move |x| x.copy_path(&vals))).glyph("link").accel("Ctrl+Shift+C"));
    list.push(MenuItem::separator());
    let its = items.clone();
    list.push(MenuItem::new(s("properties"), act(x, move |x| ops::properties(x, its.clone()))).glyph("properties").accel("Alt+Enter").name("explorer-properties"));
    if let Some(o) = &one
        && flag(o, "isDir")
    {
        let dir = o.get("volume").map(|v| st(v, "mount").to_owned()).filter(|m| !m.is_empty()).unwrap_or_else(|| st(o, "path").to_owned());
        list.push(MenuItem::separator());
        list.push(MenuItem::new(s("terminal"), act(x, move |x| x.terminal(Some(dir.clone())))).glyph("terminal"));
    }
    match (open_with_at, one) {
        (Some(at), Some(item)) => {
            ops::open_with_items(x, &item, move |apps| {
                let mut list = list;
                if let Some(slot) = list.get_mut(at) {
                    *slot = MenuItem::submenu(s("openWith"), apps).glyph("apps");
                }
                done(top, list);
            });
        }
        _ => done(top, list),
    }
}

/// The context menu of empty space.
pub fn background_menu(x: &Explorer) -> (Vec<TopAction>, Vec<MenuItem>) {
    let loc = x.loc();
    let kind = loc.kind();
    let refresh = MenuItem::new(s("refreshMenu"), act(x, |x| x.refresh())).glyph("refresh").accel("F5");
    if loc == Loc::Trash {
        let has = !x.active_items().is_empty();
        return (
            Vec::new(),
            vec![
                MenuItem::submenu(s("ctxView"), view_items(x)).glyph("view"),
                MenuItem::submenu(s("sortBy"), sort_menu(x)).glyph("sort"),
                refresh,
                MenuItem::separator(),
                MenuItem::new(s("emptyTrash"), act(x, ops::empty_trash)).glyph("trash").enabled(has).name("explorer-empty-trash"),
                MenuItem::new(s("restoreAll"), act(x, |x| ops::restore(x, Some(x.active_items())))).glyph("undo").enabled(has),
                MenuItem::separator(),
                MenuItem::new(s("properties"), act(x, dialogs::show_trash_properties)).glyph("properties"),
            ],
        );
    }
    if !loc.is_fileish() {
        return (Vec::new(), vec![refresh]);
    }
    let mut sort = sort_menu(x);
    sort.truncate(sort.len().saturating_sub(2));
    let folder = loc.is_folder();
    let can_paste = folder && !x.state().clipboard_paths.is_empty();
    (
        Vec::new(),
        vec![
            MenuItem::submenu(s("ctxView"), view_items(x)).glyph("view"),
            MenuItem::submenu(s("sortBy"), sort).glyph("sort"),
            MenuItem::submenu(s("groupBy"), group_items(x, kind)).glyph("list"),
            refresh,
            MenuItem::separator(),
            MenuItem::new(s("pasteMenu"), act(x, |x| ops::paste(x, None))).glyph("paste").accel("Ctrl+V").enabled(can_paste),
            MenuItem::new(s("undo"), act(x, ops::undo)).glyph("undo").accel("Ctrl+Z").enabled(x.has_undo()),
            MenuItem::separator(),
            MenuItem::submenu(s("newMenu"), new_menu(x)).glyph("plus").enabled(folder),
            MenuItem::separator(),
            MenuItem::new(s("properties"), act(x, |x| ops::properties(x, Vec::new()))).glyph("properties"),
            MenuItem::new(s("terminal"), act(x, |x| x.terminal(None))).glyph("terminal").enabled(folder),
        ],
    )
}

/// The context menu of a navigation pane row (`drive`: a drive under Deze pc).
pub fn nav_menu(x: &Explorer, loc: &Loc, drive: Option<Value>) -> Vec<MenuItem> {
    let l = loc.clone();
    let new_tab = MenuItem::new(s("openNewTab"), act(x, move |x| {
        x.open_tab(l.clone(), false);
    }))
    .glyph("window");
    match loc {
        Loc::Trash => vec![
            new_tab,
            MenuItem::separator(),
            MenuItem::new(s("emptyTrash"), act(x, ops::empty_trash)).glyph("trash").enabled(x.state().trash_full),
            MenuItem::new(s("properties"), act(x, dialogs::show_trash_properties)).glyph("properties"),
        ],
        Loc::Folder(p) => {
            let mut out = vec![new_tab, MenuItem::separator()];
            if drive.is_none() {
                let pinned = x.is_pinned(p);
                let pp = p.clone();
                out.push(
                    MenuItem::new(s(if pinned { "unpin" } else { "pin" }), act(x, move |x| if pinned { x.unpin(&pp) } else { x.pin(&pp) }))
                        .glyph(if pinned { "unpin" } else { "pin" }),
                );
            }
            let pp = p.clone();
            out.push(MenuItem::new(s("copyPath"), act(x, move |x| x.copy_path(&[json!({ "path": pp })]))).glyph("link"));
            let pp = p.clone();
            out.push(MenuItem::new(s("terminal"), act(x, move |x| x.terminal(Some(pp.clone())))).glyph("terminal"));
            out.push(MenuItem::separator());
            let pp = p.clone();
            out.push(
                MenuItem::new(s("properties"), act(x, move |x| match &drive {
                    Some(v) => dialogs::show_drive_properties(x, v),
                    None => {
                        let item = json!({ "path": pp, "name": util::basename(&pp), "isDir": true, "icon": x.icon_name_for(&pp) });
                        dialogs::show_properties(x, vec![std::rc::Rc::new(item)]);
                    }
                }))
                .glyph("properties"),
            );
            out
        }
        _ => vec![new_tab],
    }
}
