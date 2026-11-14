//! Actions on results (actions.js): open, open the containing folder in Verkenner, copy paths or names,
//! properties, move to the Prullenbak.
use crate::model::Entry;
use crate::text;
use kit::controls::{self, Dialog, DialogResult, Severity, Toast};
use kit::prelude::*;
use kit::{CallError, t};
use std::rc::Rc;

/// The most items moved to the Prullenbak at once (actions.js TRASH_MAX).
pub const TRASH_MAX: usize = 500;

fn fail(parent: &gtk::Widget, title: &str, e: &CallError) {
    if e.code == "cancelled" {
        return;
    }
    Toast::new(title).message(&e.message).severity(Severity::Error).show(parent);
}

fn open_path(parent: &gtk::Widget, path: &str) {
    let p = parent.downgrade();
    kit::ctx().call("files.open", json!({ "path": path }), move |r| {
        if let (Err(e), Some(p)) = (r, p.upgrade()) {
            fail(&p, t("Kon dit niet openen", "Couldn’t open this"), &e);
        }
    });
}

/// Open like a double click: folders in our Verkenner (files.open does that), files with their app.
pub fn open_item(parent: &gtk::Widget, e: &Entry) {
    open_path(parent, &e.path);
}

/// Open the folder that holds the item in Verkenner, with the item selected.
pub fn open_containing(parent: &gtk::Widget, e: &Entry) {
    let p = parent.downgrade();
    kit::ctx().call(
        "shell.openApp",
        json!({ "app": "explorer", "params": { "path": e.folder(), "select": e.path } }),
        move |r| {
            if let (Err(e), Some(p)) = (r, p.upgrade()) {
                fail(&p, t("Kon dit niet openen", "Couldn’t open this"), &e);
            }
        },
    );
}

/// Folders open themselves in Verkenner; files open their folder.
pub fn open_in_explorer(parent: &gtk::Widget, e: &Entry) {
    if e.is_dir {
        open_path(parent, &e.path);
    } else {
        open_containing(parent, e);
    }
}

/// Copy the paths (or names) to the clipboard, one per line, and say so.
pub fn copy_paths(parent: &gtk::Widget, items: &[Rc<Entry>], names: bool, capped: bool) {
    if items.is_empty() {
        return;
    }
    let text: Vec<&str> = items.iter().map(|i| if names { i.name.as_str() } else { i.path.as_str() }).collect();
    let Some(display) = gdk::Display::default() else {
        Toast::new(t("Kopiëren is niet gelukt", "Couldn’t copy")).severity(Severity::Error).show(parent);
        return;
    };
    display.clipboard().set_text(&text.join("\n"));
    let message = if capped {
        text::copy_capped(items.len())
    } else if items.len() == 1 {
        text[0].to_owned()
    } else {
        String::new()
    };
    Toast::new(&text::copied(items.len(), names)).message(&message).severity(Severity::Success).timeout(2600).show(parent);
}

// ------------------------------------------------------------------------------------------- properties

fn prop_row(grid: &gtk::Grid, row: &mut i32, label: &str, value: &gtk::Widget) {
    let l = controls::label(label, "alles-props-dt", true);
    l.set_valign(gtk::Align::Start);
    grid.attach(&l, 0, *row, 1, 1);
    value.set_hexpand(true);
    grid.attach(value, 1, *row, 1, 1);
    *row += 1;
}

fn value_label(text: &str, class: &str) -> gtk::Widget {
    let l = controls::label(if text.is_empty() { "–" } else { text }, &format!("alles-props-dd {class}"), true);
    // wrap within the 460 px dialog instead of widening it
    l.set_max_width_chars(36);
    l.set_selectable(true);
    l.set_can_focus(false);
    l.upcast()
}

fn prop_list() -> gtk::Grid {
    let g = gtk::Grid::new();
    g.add_css_class("alles-props-list");
    g.set_row_spacing(8);
    g.set_column_spacing(12);
    g
}

fn separator() -> gtk::Widget {
    let s = gtk::Separator::new(gtk::Orientation::Horizontal);
    s.add_css_class("alles-props-sep");
    s.upcast()
}

/// The Eigenschappen dialog (files.properties; what it cannot read comes from the result itself).
pub fn show_properties(parent: &gtk::Widget, e: &Entry) {
    let e = e.clone();
    let p = parent.downgrade();
    kit::ctx().call("files.properties", json!({ "path": e.path }), move |r| {
        let Some(parent) = p.upgrade() else { return };
        let partial = r.is_err();
        let pr = r.ok().filter(Value::is_object).unwrap_or_else(|| json!({}));
        properties_dialog(&parent, &e, &pr, partial);
    });
}

fn properties_dialog(parent: &gtk::Widget, e: &Entry, p: &Value, partial: bool) {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.add_css_class("alles-props");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    head.add_css_class("alles-props-head");
    let icon = kit::icons::file_icon(if e.is_dir { "folder" } else { e.icon_mime() }, 32);
    icon.add_css_class("alles-props-icon");
    head.append(&icon);
    let name = controls::label(&e.name, "alles-props-name", true);
    name.set_max_width_chars(30);
    name.set_selectable(true);
    name.set_can_focus(false);
    name.set_hexpand(true);
    head.append(&name);
    content.append(&head);
    content.append(&separator());

    let size = p["size"].as_f64().unwrap_or(e.size);
    let list = prop_list();
    let mut row = 0;
    let type_text = p["type"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| text::type_name(e));
    prop_row(&list, &mut row, t("Type", "Type"), &value_label(&type_text, ""));
    prop_row(&list, &mut row, t("Locatie", "Location"), &value_label(e.folder(), ""));
    let size_text = if size < 1024.0 {
        kit::format::bytes(size)
    } else {
        format!("{} ({} bytes)", kit::format::bytes(size), text::num(size))
    };
    prop_row(&list, &mut row, t("Grootte", "Size"), &value_label(&size_text, ""));
    if p["contains"].is_object() {
        let c = &p["contains"];
        let v = text::contains_value(c["files"].as_f64().unwrap_or(0.0), c["folders"].as_f64().unwrap_or(0.0));
        prop_row(&list, &mut row, t("Bevat", "Contains"), &value_label(&v, ""));
    }
    content.append(&list);
    content.append(&separator());

    let list = prop_list();
    let mut row = 0;
    if let Some(c) = p["created"].as_f64().filter(|c| *c > 0.0) {
        prop_row(&list, &mut row, t("Gemaakt op", "Created"), &value_label(&kit::format::file_time(c), ""));
    }
    let modified = p["modified"].as_f64().unwrap_or(e.modified);
    let modified_text = if modified > 0.0 { kit::format::file_time(modified) } else { String::new() };
    prop_row(&list, &mut row, t("Gewijzigd op", "Modified"), &value_label(&modified_text, ""));
    if let Some(a) = p["accessed"].as_f64().filter(|a| *a > 0.0) {
        prop_row(&list, &mut row, t("Geopend op", "Accessed"), &value_label(&kit::format::file_time(a), ""));
    }
    if let Some(o) = p["owner"].as_str().filter(|s| !s.is_empty()) {
        prop_row(&list, &mut row, t("Eigenaar", "Owner"), &value_label(o, ""));
    }
    if let Some(perm) = p["permissions"].as_str().filter(|s| !s.is_empty()) {
        prop_row(&list, &mut row, t("Rechten", "Permissions"), &value_label(perm, "w11-mono"));
    }
    content.append(&list);
    content.append(&separator());

    let attrs = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    attrs.add_css_class("alles-props-attrs");
    let al = controls::label(t("Kenmerken", "Attributes"), "alles-props-attrlabel", false);
    al.set_size_request(110, -1);
    attrs.append(&al);
    let ro = controls::checkbox(t("Alleen-lezen", "Read-only"));
    ro.set_active(p["readOnly"] == true);
    ro.set_sensitive(false);
    attrs.append(&ro);
    let hidden = p.get("hidden").and_then(Value::as_bool).unwrap_or(e.hidden);
    let hi = controls::checkbox(t("Verborgen", "Hidden"));
    hi.set_active(hidden);
    hi.set_sensitive(false);
    attrs.append(&hi);
    content.append(&attrs);
    if partial {
        content.append(&controls::label(
            t("Niet alle gegevens konden worden gelezen.", "Not all details could be read."),
            "alles-props-note",
            true,
        ));
    }
    let title = if kit::i18n::en() { format!("{} Properties", e.name) } else { format!("Eigenschappen van {}", e.name) };
    let width = parent_width(parent).map(|w| (w - 48).clamp(320, 460)).unwrap_or(460);
    Dialog::new(&title).body(&content).close_label(t("OK", "OK")).width(width).show(parent, |_| {});
}

/// The width of the window a widget is in.
pub fn parent_width(w: &gtk::Widget) -> Option<i32> {
    w.root().and_downcast::<gtk::Window>().map(|w| w.width()).filter(|w| *w > 0)
}

// ------------------------------------------------------------------------------------------------- trash

/// Confirm, then move to the Prullenbak; `done(true)` when something was moved.
pub fn trash_items(parent: &gtk::Widget, items: Vec<Rc<Entry>>, done: impl FnOnce(bool) + 'static) {
    if items.is_empty() {
        done(false);
        return;
    }
    if items.len() > TRASH_MAX {
        controls::alert(parent, t("Naar Prullenbak", "Move to Recycle Bin"), &text::trash_too_many(TRASH_MAX), move || done(false));
        return;
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.add_css_class("alles-confirm");
    let first = if items.len() == 1 {
        if kit::i18n::en() {
            format!("“{}” will be moved to the Recycle Bin. You can restore it from there later.", items[0].name)
        } else {
            format!("“{}” gaat naar de Prullenbak. Je kunt het daar later terugzetten.", items[0].name)
        }
    } else if kit::i18n::en() {
        format!("The {} selected items will be moved to the Recycle Bin. You can restore them from there later.", text::num(items.len() as f64))
    } else {
        format!("De {} geselecteerde items gaan naar de Prullenbak. Je kunt ze daar later terugzetten.", text::num(items.len() as f64))
    };
    let l = controls::label(&first, "w11-body", true);
    l.set_max_width_chars(56);
    body.append(&l);
    if items.iter().any(|i| i.is_dir) {
        body.append(&controls::label(
            t("Alles wat in deze map staat gaat mee.", "Everything in this folder will be moved too."),
            "alles-confirm-note",
            true,
        ));
    }
    let p = parent.downgrade();
    Dialog::new(&text::trash_title(items.len())).body(&body).primary(t("Verplaatsen", "Move")).danger().show(parent, move |r| {
        if r != DialogResult::Primary {
            done(false);
            return;
        }
        let paths: Vec<&str> = items.iter().map(|i| i.path.as_str()).collect();
        let n = items.len();
        kit::ctx().call("files.trash", json!({ "paths": paths }), move |res| {
            let Some(parent) = p.upgrade() else {
                done(false);
                return;
            };
            match res {
                Ok(r) => {
                    let failed = r["failed"].as_array().map(Vec::len).unwrap_or(0);
                    if failed > 0 {
                        Toast::new(t("Niet alles kon naar de Prullenbak", "Not everything could be moved to the Recycle Bin"))
                            .message(&text::trash_failed_body(failed))
                            .severity(Severity::Warning)
                            .show(&parent);
                    } else {
                        let msg = if r["dryRun"] == true { t("Testmodus: er is niets echt verplaatst.", "Test mode: nothing was actually moved.") } else { "" };
                        Toast::new(&text::trashed(n)).message(msg).severity(Severity::Success).timeout(3500).show(&parent);
                    }
                    done(failed < n);
                }
                Err(e) => {
                    fail(&parent, t("Niet alles kon naar de Prullenbak", "Not everything could be moved to the Recycle Bin"), &e);
                    done(false);
                }
            }
        });
    });
}
