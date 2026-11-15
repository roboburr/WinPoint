//! What the desktop does (desktop.js "actions" and the menu controller): open, open with, delete with Undo,
//! empty the Recycle Bin, cut/copy/paste, new folder or text document, compress, extract, copy as path,
//! properties, settings pages, Terminal, icon size, sorting and the View toggles; the "dt" action group the
//! context menus use; the backend's files.* and settings.changed events.
use super::call;
use super::grid::{Desk, Item, base_of, first_free, nearest_free, parent_of};
use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

impl Desk {
    /* ------------------------------------------------------------ open */

    pub fn open_items(self: &Rc<Self>, items: Vec<Item>) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            let home = desk.s.borrow().home.clone();
            for it in items {
                let result = match it.special {
                    Some("thispc") => {
                        call(&desk.shell, "shell.openApp", json!({"app": "explorer", "params": {"place": "thispc"}})).await
                    }
                    Some("trash") => {
                        call(&desk.shell, "shell.openApp", json!({"app": "explorer", "params": {"place": "trash"}})).await
                    }
                    Some("home") => {
                        let params = if home.is_empty() { json!({"place": "home"}) } else { json!({"path": home}) };
                        call(&desk.shell, "shell.openApp", json!({"app": "explorer", "params": params})).await
                    }
                    _ if it.is_dir => {
                        call(&desk.shell, "shell.openApp", json!({"app": "explorer", "params": {"path": it.path}})).await
                    }
                    _ => {
                        desk.busy_cursor();
                        call(&desk.shell, "files.open", json!({"path": it.path})).await
                    }
                };
                if let Err(error) = result {
                    let fallback = desk.t("Dit item kon niet worden geopend.", "This item couldn’t be opened.");
                    desk.error_alert(Some(&error), Some(&fallback));
                }
            }
        });
    }

    pub fn open_with(self: &Rc<Self>, it: Item, app: String) {
        self.busy_cursor();
        let desk = self.clone();
        glib::spawn_future_local(async move {
            if let Err(error) = call(&desk.shell, "files.openWith", json!({"path": it.path, "appId": app})).await {
                let fallback = desk.t("Dit item kon niet worden geopend.", "This item couldn’t be opened.");
                desk.error_alert(Some(&error), Some(&fallback));
            }
        });
    }

    pub fn open_settings(self: &Rc<Self>, page: &str) {
        let desk = self.clone();
        let page = page.to_owned();
        glib::spawn_future_local(async move {
            if let Err(error) = call(&desk.shell, "shell.openApp", json!({"app": "settings", "params": {"page": page}})).await {
                desk.error_alert(Some(&error), None);
            }
        });
    }

    pub fn open_terminal(self: &Rc<Self>, dir: Option<String>) {
        self.busy_cursor();
        let desk = self.clone();
        let dir = dir.filter(|d| !d.is_empty()).unwrap_or_else(|| self.s.borrow().dir.clone());
        glib::spawn_future_local(async move {
            if let Err(error) = call(&desk.shell, "files.openTerminal", json!({"dir": dir})).await {
                desk.error_alert(Some(&error), None);
            }
        });
    }

    /* ------------------------------------------------------------ messages */

    pub fn error_alert(self: &Rc<Self>, error: Option<&str>, fallback: Option<&str>) {
        let generic = self.t("Er ging iets mis. Probeer het opnieuw.", "Something went wrong. Try again.");
        let message = error.filter(|e| !e.is_empty()).or(fallback).unwrap_or(&generic).to_owned();
        eprintln!("win11-shell: desktop: {message}");
        super::dialogs::alert(self, &self.t("Dat lukte niet", "That didn’t work"), &message);
    }

    /// --dry-run (test mode): the host changed nothing; say so instead of pretending.
    pub fn dry_run_toast(self: &Rc<Self>, trash: bool) {
        let message = if trash {
            self.t(
                "Win11 Shell draait in de testmodus: er is niets naar de Prullenbak verplaatst.",
                "Win11 Shell is running in test mode: nothing was moved to the Recycle Bin.",
            )
        } else {
            self.t(
                "Win11 Shell draait in de testmodus: deze wijziging is niet echt uitgevoerd.",
                "Win11 Shell is running in test mode: this change wasn’t actually made.",
            )
        };
        super::dialogs::toast(self, "dialog-information-symbolic", &self.t("Testmodus", "Test mode"), &message, None, 5000);
    }

    /* ------------------------------------------------------------ undo */

    pub fn push_undo(self: &Rc<Self>, label: String, run: Rc<dyn Fn(Rc<Desk>)>) -> u64 {
        let mut s = self.s.borrow_mut();
        s.undo_seq += 1;
        let id = s.undo_seq;
        s.undo.push(super::grid::Undo { id, label, run, toast: None });
        if s.undo.len() > 20 {
            s.undo.remove(0);
        }
        id
    }

    pub fn run_undo(self: &Rc<Self>) {
        let Some(u) = self.s.borrow_mut().undo.pop() else { return };
        if let Some(t) = u.toast {
            super::dialogs::close_toast(&t);
        }
        (u.run)(self.clone());
    }

    /* ------------------------------------------------------------ trash */

    pub fn trash_items(self: &Rc<Self>, items: Vec<Item>) {
        let list: Vec<Item> = items.into_iter().filter(|i| i.special.is_none() && !i.path.is_empty()).collect();
        if list.is_empty() {
            return;
        }
        let desk = self.clone();
        glib::spawn_future_local(async move {
            let paths: Vec<String> = list.iter().map(|i| i.path.clone()).collect();
            // keep keyboard focus near where it was
            let order: Vec<String> = desk.visual_order().into_iter().map(|i| i.key).collect();
            let idx = list.iter().filter_map(|i| order.iter().position(|k| *k == i.key)).min().unwrap_or(0);
            // so Undo puts the icons back on their old spot
            let old_pos: Map<String, Value> = {
                let s = desk.s.borrow();
                list.iter()
                    .filter_map(|i| s.placed.get(&i.key).map(|p| (i.key.clone(), json!({"x": p.0, "y": p.1}))))
                    .collect()
            };
            let r = match call(&desk.shell, "files.trash", json!({"paths": paths})).await {
                Ok(r) => r,
                Err(error) => {
                    let fallback = desk.t("Niet alles kon naar de Prullenbak worden verplaatst.", "Not everything could be moved to the Recycle Bin.");
                    desk.error_alert(Some(&error), Some(&fallback));
                    return;
                }
            };
            if r["dryRun"] == true {
                desk.dry_run_toast(true);
                return;
            }
            let failed = r["failed"].as_array().cloned().unwrap_or_default();
            let failed_paths: HashSet<String> = failed
                .iter()
                .filter_map(|f| f["path"].as_str().or(f.as_str()).map(str::to_owned))
                .collect();
            let ok: Vec<Item> = list.iter().filter(|i| !failed_paths.contains(&i.path)).cloned().collect();
            let trashed: Option<Vec<String>> = r["trashed"]
                .as_array()
                .filter(|t| !t.is_empty())
                .map(|t| t.iter().filter_map(|x| x["name"].as_str().map(str::to_owned)).collect());
            let ok_paths: Vec<String> = ok.iter().map(|i| i.path.clone()).collect();
            desk.s.borrow_mut().selected.clear();
            desk.reload().await;
            let rest = desk.visual_order();
            let next = rest.get(idx.min(rest.len().saturating_sub(1))).map(|i| i.key.clone());
            desk.s.borrow_mut().focus = next;
            desk.render();
            if !failed.is_empty() {
                let first = failed.iter().filter_map(|f| f["error"].as_str()).find(|e| !e.is_empty()).unwrap_or("");
                let message = format!(
                    "{} {first}",
                    desk.t("Niet alles kon naar de Prullenbak worden verplaatst.", "Not everything could be moved to the Recycle Bin.")
                );
                desk.error_alert(Some(message.trim()), None);
            }
            if ok.is_empty() {
                return;
            }
            let restore: Rc<dyn Fn(Rc<Desk>)> = Rc::new(move |d: Rc<Desk>| {
                let (trashed, ok_paths, old_pos) = (trashed.clone(), ok_paths.clone(), old_pos.clone());
                glib::spawn_future_local(async move {
                    let names = match trashed {
                        Some(names) => names,
                        None => {
                            // the names inside the Prullenbak: newest entry per original path
                            let listing = call(&d.shell, "files.list", json!({"path": "trash:///"})).await.unwrap_or(Value::Null);
                            let items = listing["items"].as_array().cloned().unwrap_or_default();
                            ok_paths
                                .iter()
                                .map(|p| {
                                    items
                                        .iter()
                                        .filter(|x| x["originalPath"].as_str() == Some(p))
                                        .max_by(|a, b| {
                                            a["deletedAt"].as_f64().unwrap_or(0.0).total_cmp(&b["deletedAt"].as_f64().unwrap_or(0.0))
                                        })
                                        .and_then(|x| x["trashName"].as_str().map(str::to_owned))
                                        .unwrap_or_else(|| base_of(p))
                                })
                                .collect()
                        }
                    };
                    let rr = match call(&d.shell, "files.restoreTrash", json!({"names": names})).await {
                        Ok(rr) => rr,
                        Err(error) => {
                            d.error_alert(Some(&error), None);
                            d.reload().await;
                            return;
                        }
                    };
                    let restored: Vec<String> =
                        rr["restored"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(base_of)).collect()).unwrap_or_default();
                    let ds = d.dset();
                    let positions = ds.get("positions").and_then(Value::as_object).cloned().unwrap_or_default();
                    if ds.get("autoArrange") != Some(&Value::Bool(true)) && !positions.is_empty() {
                        let mut pos = positions;
                        for k in &restored {
                            if let Some(p) = old_pos.get(k) {
                                pos.insert(k.clone(), p.clone());
                            }
                        }
                        d.set_desktop("positions", Value::Object(pos));
                    }
                    d.reload().await;
                    let back: Vec<String> = restored.into_iter().filter(|k| d.has(k)).collect();
                    if let Some(first) = back.first().cloned() {
                        d.set_selection(back, Some(Some(first)));
                    }
                });
            });
            let id = desk.push_undo(desk.t("Verwijderen ongedaan maken", "Undo delete"), restore.clone());
            let message = if ok.len() == 1 {
                desk.t(
                    &format!("“{}” staat nu in de Prullenbak.", ok[0].label),
                    &format!("“{}” is now in the Recycle Bin.", ok[0].label),
                )
            } else {
                desk.t(&format!("{} items staan nu in de Prullenbak.", ok.len()), &format!("{} items are now in the Recycle Bin.", ok.len()))
            };
            let weak = Rc::downgrade(&desk);
            let undo_now: Rc<dyn Fn()> = Rc::new(move || {
                let Some(d) = weak.upgrade() else { return };
                let entry = {
                    let mut s = d.s.borrow_mut();
                    s.undo.iter().position(|u| u.id == id).map(|i| s.undo.remove(i))
                };
                if let Some(u) = entry {
                    (u.run)(d.clone());
                }
            });
            let toast = super::dialogs::toast(
                &desk,
                "user-trash-symbolic",
                &desk.t("Verplaatst naar Prullenbak", "Moved to the Recycle Bin"),
                &message,
                Some((desk.t("Ongedaan maken", "Undo"), undo_now)),
                7000,
            );
            if let Some(u) = desk.s.borrow_mut().undo.iter_mut().find(|u| u.id == id) {
                u.toast = Some(toast.upcast());
            }
        });
    }

    pub fn move_into(self: &Rc<Self>, files: Vec<Item>, dest: String) {
        if dest.is_empty() {
            return;
        }
        let desk = self.clone();
        glib::spawn_future_local(async move {
            let paths: Vec<String> = files.iter().map(|i| i.path.clone()).collect();
            match call(&desk.shell, "files.move", json!({"paths": paths, "dest": dest})).await {
                Ok(r) if r["dryRun"] == true => {
                    desk.dry_run_toast(false);
                    return;
                }
                Ok(r) => {
                    let moved: Vec<String> =
                        r["paths"].as_array().map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_owned)).collect()).unwrap_or_default();
                    let dir = desk.s.borrow().dir.clone();
                    desk.push_undo(
                        desk.t("Verplaatsen ongedaan maken", "Undo move"),
                        Rc::new(move |d: Rc<Desk>| {
                            let (moved, dir) = (moved.clone(), dir.clone());
                            glib::spawn_future_local(async move {
                                if let Err(e) = call(&d.shell, "files.move", json!({"paths": moved, "dest": dir})).await {
                                    d.error_alert(Some(&e), None);
                                }
                                d.reload().await;
                            });
                        }),
                    );
                    if let Some(first) = r["failed"].as_array().and_then(|f| f.first()) {
                        desk.error_alert(first["error"].as_str(), None);
                    }
                }
                Err(error) => desk.error_alert(Some(&error), None),
            }
            desk.reload().await;
        });
    }

    pub fn empty_trash(self: &Rc<Self>) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            desk.load_trash().await;
            let (count, size) = {
                let s = desk.s.borrow();
                (s.trash_count, s.trash_size)
            };
            if count == 0 {
                return;
            }
            let size = super::dialogs::bytes(size, &desk.shell.lang());
            let message = if count == 1 {
                desk.t(
                    &format!("Het item in de Prullenbak ({size}) wordt definitief verwijderd. Dit kun je niet ongedaan maken."),
                    &format!("The item in the Recycle Bin ({size}) will be permanently deleted. This can’t be undone."),
                )
            } else {
                desk.t(
                    &format!("Alle {count} items in de Prullenbak ({size}) worden definitief verwijderd. Dit kun je niet ongedaan maken."),
                    &format!("All {count} items in the Recycle Bin ({size}) will be permanently deleted. This can’t be undone."),
                )
            };
            let weak = Rc::downgrade(&desk);
            super::dialogs::confirm(
                &desk,
                &desk.t("Prullenbak leegmaken?", "Empty Recycle Bin?"),
                &message,
                &desk.t("Definitief verwijderen", "Delete permanently"),
                &desk.t("Annuleren", "Cancel"),
                true,
                move |yes| {
                    let Some(d) = weak.upgrade() else { return };
                    if !yes {
                        return;
                    }
                    glib::spawn_future_local(async move {
                        match call(&d.shell, "files.emptyTrash", json!({})).await {
                            Ok(r) if r["dryRun"] == true => d.dry_run_toast(false),
                            Ok(_) => {
                                super::dialogs::toast(
                                    &d,
                                    "user-trash-symbolic",
                                    &d.t("Prullenbak is leeg", "Recycle Bin is empty"),
                                    &d.t("Alle items zijn definitief verwijderd.", "All items were permanently deleted."),
                                    None,
                                    4000,
                                );
                            }
                            Err(error) => d.error_alert(Some(&error), None),
                        }
                        d.load_trash().await;
                        d.refresh_view();
                    });
                },
            );
        });
    }

    /* ------------------------------------------------------------ clipboard */

    pub fn set_clipboard(self: &Rc<Self>, items: &[Item], op: &str) {
        let paths: Vec<String> = items.iter().filter(|i| i.special.is_none() && !i.path.is_empty()).map(|i| i.path.clone()).collect();
        if paths.is_empty() {
            return;
        }
        let desk = self.clone();
        let op = op.to_owned();
        glib::spawn_future_local(async move {
            match call(&desk.shell, "files.clipboardSet", json!({"paths": paths, "op": op})).await {
                Ok(_) => {
                    desk.set_clip(&json!({"paths": paths, "op": op}));
                    desk.render();
                }
                Err(error) => desk.error_alert(Some(&error), None),
            }
        });
    }

    pub fn copy_path(self: &Rc<Self>, items: &[Item]) {
        let text = items.iter().map(|i| format!("\"{}\"", i.path)).collect::<Vec<_>>().join("\n");
        WidgetExt::display(&self.window).clipboard().set_text(&text);
    }

    /// Pin positions: all current spots, plus `names` at or near the cell under `point` (new or pasted items).
    async fn pin_new_at(self: &Rc<Self>, names: &[String], point: Option<(f64, f64)>) {
        let d = self.dset();
        if d.get("autoArrange") == Some(&Value::Bool(true)) || names.is_empty() {
            return;
        }
        let mut pos = Map::new();
        {
            let s = self.s.borrow();
            let m = s.metrics;
            let mut occ: HashSet<(i32, i32)> = HashSet::new();
            for (k, p) in &s.placed {
                pos.insert(k.clone(), json!({"x": p.0, "y": p.1}));
                occ.insert((p.0.round() as i32, p.1.round() as i32));
            }
            let mut start = point.map(|(x, y)| {
                (
                    (((x - super::grid::ORIGIN) / m.cw).floor() as i32).clamp(0, m.cols - 1),
                    (((y - super::grid::ORIGIN) / m.ch).floor() as i32).clamp(0, m.rows - 1),
                )
            });
            for n in names {
                let (c, r) = match start {
                    Some((c, r)) => nearest_free(c, r, &occ, m.cols, m.rows),
                    None => first_free(&occ, m.cols, m.rows),
                };
                pos.insert(n.clone(), json!({"x": c, "y": r}));
                occ.insert((c, r));
                start = start.or(Some((c, r)));
            }
        }
        self.set_desktop("positions", Value::Object(pos));
    }

    pub fn paste(self: &Rc<Self>, point: Option<(f64, f64)>) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            desk.load_clip().await;
            if desk.s.borrow().clip_paths.is_empty() {
                return;
            }
            let dir = desk.s.borrow().dir.clone();
            let r = match call(&desk.shell, "files.paste", json!({"dest": dir})).await {
                Ok(r) => r,
                Err(error) => {
                    let fallback = desk.t("Plakken is niet gelukt.", "Couldn’t paste.");
                    desk.error_alert(Some(&error), Some(&fallback));
                    return;
                }
            };
            if r["dryRun"] == true {
                desk.dry_run_toast(false);
                return;
            }
            let names: Vec<String> = r["paths"]
                .as_array()
                .map(|a| a.iter().filter_map(|p| p.as_str()).filter(|p| parent_of(p) == dir).map(base_of).collect())
                .unwrap_or_default();
            desk.load_files().await;
            let fresh: Vec<String> = names.iter().filter(|n| !desk.s.borrow().placed.contains_key(*n)).cloned().collect();
            if !fresh.is_empty() {
                desk.pin_new_at(&fresh, point).await;
            }
            desk.load_clip().await;
            desk.reload().await;
            let present: Vec<String> = names.into_iter().filter(|n| desk.has(n)).collect();
            if let Some(first) = present.first().cloned() {
                desk.s.borrow_mut().anchor = Some(first.clone());
                desk.set_selection(present, Some(Some(first)));
            }
        });
    }

    /* ------------------------------------------------------------ new, compress, extract */

    pub fn create_new(self: &Rc<Self>, kind: &str, point: Option<(f64, f64)>) {
        let desk = self.clone();
        let kind = kind.to_owned();
        glib::spawn_future_local(async move {
            let dir = desk.s.borrow().dir.clone();
            let r = if kind == "folder" {
                call(&desk.shell, "files.createFolder", json!({"dir": dir})).await
            } else {
                call(&desk.shell, "files.createFile", json!({"dir": dir, "kind": "text"})).await
            };
            let r = match r {
                Ok(r) => r,
                Err(error) => {
                    desk.error_alert(Some(&error), None);
                    return;
                }
            };
            if r["dryRun"] == true {
                desk.dry_run_toast(false);
                return;
            }
            let Some(path) = r["path"].as_str().map(str::to_owned) else { return };
            let name = base_of(&path);
            desk.load_files().await;
            desk.pin_new_at(std::slice::from_ref(&name), point).await;
            desk.reload().await;
            desk.push_undo(
                desk.t("Nieuw ongedaan maken", "Undo new"),
                Rc::new(move |d: Rc<Desk>| {
                    let path = path.clone();
                    glib::spawn_future_local(async move {
                        if let Err(e) = call(&d.shell, "files.trash", json!({"paths": [path]})).await {
                            d.error_alert(Some(&e), None);
                        }
                        d.reload().await;
                    });
                }),
            );
            desk.select_only(Some(name.clone()));
            desk.start_rename(&name);
        });
    }

    pub fn compress(self: &Rc<Self>, items: Vec<Item>) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            let paths: Vec<String> = items.iter().map(|i| i.path.clone()).collect();
            match call(&desk.shell, "files.compress", json!({"paths": paths})).await {
                Ok(r) if r["dryRun"] == true => desk.dry_run_toast(false),
                Ok(r) => {
                    desk.reload().await;
                    if let Some(n) = r["path"].as_str().map(base_of).filter(|n| desk.has(n)) {
                        desk.select_only(Some(n));
                    }
                }
                Err(error) => desk.error_alert(Some(&error), None),
            }
        });
    }

    pub fn extract(self: &Rc<Self>, it: Item) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            match call(&desk.shell, "files.extract", json!({"path": it.path})).await {
                Ok(r) if r["dryRun"] == true => desk.dry_run_toast(false),
                Ok(r) => {
                    desk.reload().await;
                    if let Some(n) = r["path"].as_str().map(base_of).filter(|n| desk.has(n)) {
                        desk.select_only(Some(n));
                    }
                }
                Err(error) => desk.error_alert(Some(&error), None),
            }
        });
    }

    pub fn properties(self: &Rc<Self>, items: Vec<Item>) {
        let list: Vec<Item> = items.into_iter().filter(|i| !i.path.is_empty()).collect();
        if !list.is_empty() {
            super::dialogs::properties(self, list);
        }
    }

    /* ------------------------------------------------------------ view */

    pub fn set_icon_size(self: &Rc<Self>, size: &str) {
        if !super::grid::SIZE_ORDER.contains(&size) || self.icon_size() == size {
            return;
        }
        self.set_desktop("iconSize", json!(size));
        self.refresh_view();
    }

    pub fn toggle_setting(self: &Rc<Self>, name: &str) {
        let d = self.dset();
        let default = matches!(name, "alignToGrid" | "showIcons");
        let cur = d.get(name).and_then(Value::as_bool).unwrap_or(default);
        if name == "alignToGrid" && !cur {
            // snap everything to the grid right away (like Windows)
            let pos: Map<String, Value> = self
                .s
                .borrow()
                .placed
                .iter()
                .map(|(k, p)| (k.clone(), json!({"x": p.0.round(), "y": p.1.round()})))
                .collect();
            self.set_desktop("positions", Value::Object(pos));
        }
        self.set_desktop(name, json!(!cur));
        self.refresh_view();
    }

    pub fn sort_by(self: &Rc<Self>, key: &str) {
        self.set_desktop("sortBy", json!(key));
        self.set_desktop("positions", json!({}));
        self.refresh_view();
    }

    /// Keep position and selection when an item was renamed (key = file name).
    pub fn renamed_key(self: &Rc<Self>, old: &str, new: &str) {
        if old == new {
            return;
        }
        let mut pos = self.dset().get("positions").and_then(Value::as_object).cloned().unwrap_or_default();
        if let Some(p) = pos.remove(old) {
            pos.insert(new.to_owned(), p);
            self.set_desktop("positions", Value::Object(pos));
        }
        {
            let mut tiles = self.tiles.borrow_mut();
            if let Some(tile) = tiles.remove(old) {
                tile.root.set_widget_name(&format!("desktop-item-{new}"));
                tiles.insert(new.to_owned(), tile);
            }
        }
        let mut s = self.s.borrow_mut();
        if s.selected.remove(old) {
            s.selected.insert(new.to_owned());
        }
        if s.focus.as_deref() == Some(old) {
            s.focus = Some(new.to_owned());
        }
        if s.anchor.as_deref() == Some(old) {
            s.anchor = Some(new.to_owned());
        }
        if let Some(p) = s.placed.remove(old) {
            s.placed.insert(new.to_owned(), p);
        }
    }

    /* ------------------------------------------------------------ events */

    pub fn on_files(self: &Rc<Self>, event: &str, data: &Value) {
        if !self.s.borrow().loaded {
            return;
        }
        match event {
            "files.desktopChanged" => self.reload_soon(),
            "files.dirChanged" => {
                let path = data["path"].as_str().unwrap_or("");
                if !path.is_empty() && path == self.s.borrow().dir {
                    self.reload_soon();
                } else if path.starts_with("trash:") {
                    self.trash_changed();
                }
            }
            "files.trashChanged" => self.trash_changed(),
            "files.clipboardChanged" => {
                if data["paths"].is_array() {
                    self.set_clip(data);
                    self.render();
                }
            }
            _ => {}
        }
    }

    fn trash_changed(self: &Rc<Self>) {
        let desk = self.clone();
        glib::spawn_future_local(async move {
            desk.load_trash().await;
            desk.refresh_view();
        });
    }

    pub fn on_settings(self: &Rc<Self>, data: &Value) {
        let path = data["path"].as_str().map(str::to_owned);
        {
            // our own writes are back in the settings cache (or another surface changed them meanwhile)
            let shell = self.shell.clone();
            self.s.borrow_mut().local.retain(|k, (v, at)| {
                at.elapsed() < Duration::from_secs(3) && shell.setting(&format!("desktop.{k}")) != *v
            });
        }
        let p = path.as_deref().unwrap_or("");
        // only for what the desktop shows (core::touches): Explorer's own explorer.* writes (columns, navWidth, view)
        // and the host's taskbar.tray.known rebuilt every icon before
        let touches = |keys: &[&str]| crate::core::touches(data, keys);
        if touches(&["wallpaper", "theme"]) {
            self.apply_wallpaper(false);
        }
        if touches(&["language"]) {
            self.window.set_title(Some(&self.t("Bureaublad", "Desktop")));
        }
        if !self.s.borrow().loaded {
            return;
        }
        if touches(&["desktop", "explorer.showExtensions", "taskbar.autoHide", "accessibility", "language"]) {
            self.refresh_view();
        }
        if p == "language" {
            // the files service's launcher names (Name[nl]/Name[en]) and display names in the new language
            self.reload_soon();
        }
    }

    pub fn on_geometry(self: &Rc<Self>, data: &Value) {
        // only a real change: set_default_size queues a resize and a desktop frame even for the same size
        let Some((w, h)) = super::size_of(data) else { return };
        let shown = (self.window.width(), self.window.height());
        if self.window.default_size() != (w, h) || (self.window.is_mapped() && shown != (w, h)) {
            self.window.set_default_size(w, h);
        }
    }
}

/* ---------------------------------------------------------------- the "dt" action group */

fn add(
    desk: &Rc<Desk>,
    name: &str,
    param: Option<&glib::VariantTy>,
    state: Option<glib::Variant>,
    f: impl Fn(&Rc<Desk>, Option<&glib::Variant>) + 'static,
) {
    let action = match state {
        Some(state) => gio::SimpleAction::new_stateful(name, param, &state),
        None => gio::SimpleAction::new(name, param),
    };
    let weak = Rc::downgrade(desk);
    action.connect_activate(move |_, p| {
        if let Some(d) = weak.upgrade() {
            f(&d, p);
        }
    });
    desk.actions.add_action(&action);
}

fn menu_items(d: &Rc<Desk>) -> Vec<Item> {
    let keys = d.s.borrow().menu_keys.clone();
    keys.iter().filter_map(|k| d.item(k)).collect()
}

fn files_of(d: &Rc<Desk>) -> Vec<Item> {
    menu_items(d).into_iter().filter(|i| i.special.is_none()).collect()
}

pub fn install(desk: &Rc<Desk>) {
    let string = Some(glib::VariantTy::STRING);
    add(desk, "icon-size", string, Some("medium".to_variant()), |d, p| {
        if let Some(size) = p.and_then(|p| p.str()) {
            d.set_icon_size(size);
        }
    });
    add(desk, "sort-by", string, Some("name".to_variant()), |d, p| {
        if let Some(key) = p.and_then(|p| p.str()) {
            d.sort_by(key);
        }
    });
    add(desk, "auto-arrange", None, Some(false.to_variant()), |d, _| d.toggle_setting("autoArrange"));
    add(desk, "align-grid", None, Some(true.to_variant()), |d, _| d.toggle_setting("alignToGrid"));
    add(desk, "show-icons", None, Some(true.to_variant()), |d, _| d.toggle_setting("showIcons"));
    add(desk, "refresh", None, None, |d, _| d.reload_flash());
    add(desk, "paste", None, None, |d, _| {
        let point = d.s.borrow().menu_point;
        d.paste(point);
    });
    add(desk, "undo", None, None, |d, _| d.run_undo());
    add(desk, "new-folder", None, None, |d, _| {
        let point = d.s.borrow().menu_point;
        d.create_new("folder", point);
    });
    add(desk, "new-text", None, None, |d, _| {
        let point = d.s.borrow().menu_point;
        d.create_new("text", point);
    });
    add(desk, "display-settings", None, None, |d, _| d.open_settings("display"));
    add(desk, "personalize", None, None, |d, _| d.open_settings("personalization"));
    add(desk, "open-terminal", None, None, |d, _| d.open_terminal(None));
    add(desk, "open-terminal-item", None, None, |d, _| {
        let path = menu_items(d).first().map(|i| i.path.clone());
        d.open_terminal(path);
    });
    add(desk, "open", None, None, |d, _| {
        let items = menu_items(d);
        d.open_items(items);
    });
    add(desk, "open-with", string, None, |d, p| {
        if let (Some(app), Some(it)) = (p.and_then(|p| p.str()), menu_items(d).first().cloned()) {
            d.open_with(it, app.to_owned());
        }
    });
    add(desk, "cut", None, None, |d, _| d.set_clipboard(&files_of(d), "cut"));
    add(desk, "copy", None, None, |d, _| d.set_clipboard(&files_of(d), "copy"));
    add(desk, "rename", None, None, |d, _| {
        if let Some(it) = menu_items(d).first().cloned() {
            let key = it.key.clone();
            // after the menu has closed and given the keyboard back
            let d = d.clone();
            glib::idle_add_local_once(move || d.start_rename(&key));
        }
    });
    add(desk, "delete", None, None, |d, _| d.trash_items(files_of(d)));
    add(desk, "compress", None, None, |d, _| d.compress(files_of(d)));
    add(desk, "extract", None, None, |d, _| {
        if let Some(it) = menu_items(d).first().cloned() {
            d.extract(it);
        }
    });
    add(desk, "copy-path", None, None, |d, _| {
        let items: Vec<Item> = menu_items(d).into_iter().filter(|i| !i.path.is_empty()).collect();
        d.copy_path(&items);
    });
    add(desk, "properties", None, None, |d, _| {
        let items: Vec<Item> = menu_items(d).into_iter().filter(|i| i.special.is_none() || i.special == Some("home")).collect();
        d.properties(items);
    });
    add(desk, "thispc-properties", None, None, |d, _| d.open_settings("about"));
    add(desk, "empty-trash", None, None, |d, _| d.empty_trash());
}

/// Bring the menu actions' states and sensitivity up to date before a menu opens.
pub fn sync(desk: &Rc<Desk>, items: &[Item]) {
    let d = desk.dset();
    let lookup = |name: &str| desk.actions.lookup_action(name).and_downcast::<gio::SimpleAction>();
    let flag = |k: &str, default: bool| d.get(k).and_then(Value::as_bool).unwrap_or(default);
    if let Some(a) = lookup("icon-size") {
        a.set_state(&desk.icon_size().to_variant());
    }
    if let Some(a) = lookup("sort-by") {
        a.set_state(&d.get("sortBy").and_then(Value::as_str).unwrap_or("name").to_variant());
    }
    for (name, key, default) in [("auto-arrange", "autoArrange", false), ("align-grid", "alignToGrid", true), ("show-icons", "showIcons", true)] {
        if let Some(a) = lookup(name) {
            a.set_state(&flag(key, default).to_variant());
        }
    }
    let (clip, undo, trash) = {
        let s = desk.s.borrow();
        (!s.clip_paths.is_empty(), !s.undo.is_empty(), s.trash_count > 0)
    };
    let files = items.iter().any(|i| i.special.is_none());
    let one = (items.len() == 1).then(|| &items[0]);
    let renamable = one.is_some_and(|i| i.special.is_none() && !i.is_desktop_entry);
    for (name, on) in [
        ("paste", clip),
        ("undo", undo),
        ("empty-trash", trash),
        ("cut", files),
        ("copy", files),
        ("delete", files),
        ("rename", renamable),
    ] {
        if let Some(a) = lookup(name) {
            a.set_enabled(on);
        }
    }
}
