//! The desktop's context menus (vendor/ui/js/desktop/menus.js), Win11 wording and order: the background menu
//! (Beeld, Sorteren op, Vernieuwen, Plakken, Ongedaan maken, Nieuw, Beeldscherminstellingen, Aan persoonlijke
//! voorkeur aanpassen, Openen in Terminal) and the item menu (the row of Knippen, Kopiëren, Naam wijzigen,
//! Verwijderen on top, then Openen, Openen met, ZIP, Kopiëren als pad, Eigenschappen, Openen in Terminal).
//! GTK popover menus over the "dt" action group (actions.rs).
use super::grid::{Desk, Item};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use serde_json::{Value, json};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

fn entry(label: &str, action: &str, accel: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), Some(action));
    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    item
}

fn radio(label: &str, action: &str, target: &str, accel: Option<&str>) -> gio::MenuItem {
    let item = gio::MenuItem::new(Some(label), None);
    item.set_action_and_target_value(Some(action), Some(&target.to_variant()));
    if let Some(accel) = accel {
        item.set_attribute_value("accel", Some(&accel.to_variant()));
    }
    item
}

/// The empty-desktop menu.
pub fn background(desk: &Rc<Desk>) -> gio::Menu {
    let t = |nl: &str, en: &str| desk.t(nl, en);
    let menu = gio::Menu::new();

    let first = gio::Menu::new();
    let view = gio::Menu::new();
    let sizes = gio::Menu::new();
    for (key, nl, en, accel) in [
        ("large", "Grote pictogrammen", "Large icons", "<Control><Shift>2"),
        ("medium", "Middelgrote pictogrammen", "Medium icons", "<Control><Shift>3"),
        ("small", "Kleine pictogrammen", "Small icons", "<Control><Shift>4"),
    ] {
        sizes.append_item(&radio(&t(nl, en), "dt.icon-size", key, Some(accel)));
    }
    view.append_section(None, &sizes);
    let arrange = gio::Menu::new();
    arrange.append_item(&entry(&t("Pictogrammen automatisch schikken", "Auto arrange icons"), "dt.auto-arrange", None));
    arrange.append_item(&entry(&t("Pictogrammen uitlijnen op raster", "Align icons to grid"), "dt.align-grid", None));
    view.append_section(None, &arrange);
    let show = gio::Menu::new();
    show.append_item(&entry(&t("Bureaubladpictogrammen weergeven", "Show desktop icons"), "dt.show-icons", None));
    view.append_section(None, &show);
    first.append_submenu(Some(&t("Beeld", "View")), &view);

    let sort = gio::Menu::new();
    for (key, nl, en) in [("name", "Naam", "Name"), ("size", "Grootte", "Size"), ("type", "Itemtype", "Item type"), ("modified", "Gewijzigd op", "Date modified")] {
        sort.append_item(&radio(&t(nl, en), "dt.sort-by", key, None));
    }
    first.append_submenu(Some(&t("Sorteren op", "Sort by")), &sort);
    first.append_item(&entry(&t("Vernieuwen", "Refresh"), "dt.refresh", Some("F5")));
    menu.append_section(None, &first);

    let second = gio::Menu::new();
    second.append_item(&entry(&t("Plakken", "Paste"), "dt.paste", Some("<Control>v")));
    let undo = desk.s.borrow().undo.last().map(|u| u.label.clone());
    if let Some(label) = undo {
        second.append_item(&entry(&label, "dt.undo", Some("<Control>z")));
    }
    let new = gio::Menu::new();
    let folder = gio::Menu::new();
    folder.append_item(&entry(&t("Map", "Folder"), "dt.new-folder", Some("<Control><Shift>n")));
    new.append_section(None, &folder);
    let docs = gio::Menu::new();
    docs.append_item(&entry(&t("Tekstdocument", "Text Document"), "dt.new-text", None));
    new.append_section(None, &docs);
    second.append_submenu(Some(&t("Nieuw", "New")), &new);
    menu.append_section(None, &second);

    let third = gio::Menu::new();
    third.append_item(&entry(&t("Beeldscherminstellingen", "Display settings"), "dt.display-settings", None));
    third.append_item(&entry(&t("Aan persoonlijke voorkeur aanpassen", "Personalise"), "dt.personalize", None));
    menu.append_section(None, &third);

    let fourth = gio::Menu::new();
    fourth.append_item(&entry(&t("Openen in Terminal", "Open in Terminal"), "dt.open-terminal", None));
    menu.append_section(None, &fourth);
    menu
}

fn is_archive(it: &Item) -> bool {
    let p = it.path.to_lowercase();
    !it.is_dir
        && [".zip", ".tar", ".tgz", ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".7z"].iter().any(|e| p.ends_with(e))
}

/// The menu for the selected items; `apps` is apps.forFile for a single file (may be empty).
pub fn item(desk: &Rc<Desk>, items: &[Item], apps: &[Value]) -> gio::Menu {
    let t = |nl: &str, en: &str| desk.t(nl, en);
    let menu = gio::Menu::new();
    let one = (items.len() == 1).then(|| &items[0]);

    // special icons
    if one.is_some_and(|i| i.special == Some("trash")) {
        let open = gio::Menu::new();
        open.append_item(&entry(&t("Openen", "Open"), "dt.open", Some("Return")));
        menu.append_section(None, &open);
        let empty = gio::Menu::new();
        empty.append_item(&entry(&t("Prullenbak leegmaken", "Empty Recycle Bin"), "dt.empty-trash", None));
        menu.append_section(None, &empty);
        return menu;
    }
    if one.is_some_and(|i| i.special == Some("thispc")) {
        let open = gio::Menu::new();
        open.append_item(&entry(&t("Openen", "Open"), "dt.open", Some("Return")));
        menu.append_section(None, &open);
        let props = gio::Menu::new();
        props.append_item(&entry(&t("Eigenschappen", "Properties"), "dt.thispc-properties", Some("<Alt>Return")));
        menu.append_section(None, &props);
        return menu;
    }

    let has_files = items.iter().any(|i| i.special.is_none());
    let home_only = one.is_some_and(|i| i.special == Some("home"));
    if !home_only {
        // the Win11 row of icon buttons on top
        let top = gio::Menu::new();
        for (icon, nl, en, action) in [
            ("edit-cut-symbolic", "Knippen", "Cut", "dt.cut"),
            ("edit-copy-symbolic", "Kopiëren", "Copy", "dt.copy"),
            ("document-edit-symbolic", "Naam wijzigen", "Rename", "dt.rename"),
            ("user-trash-symbolic", "Verwijderen", "Delete", "dt.delete"),
        ] {
            let item = entry(&t(nl, en), action, None);
            item.set_attribute_value("verb-icon", Some(&icon.to_variant()));
            top.append_item(&item);
        }
        let section = gio::MenuItem::new_section(None, &top);
        section.set_attribute_value("display-hint", Some(&"horizontal-buttons".to_variant()));
        menu.append_item(&section);
    }

    let open = gio::Menu::new();
    open.append_item(&entry(&t("Openen", "Open"), "dt.open", Some("Return")));
    if one.is_some_and(|i| i.special.is_none() && !i.is_dir && !i.is_desktop_entry) {
        let sub = gio::Menu::new();
        if apps.is_empty() {
            sub.append_item(&gio::MenuItem::new(Some(&t("Geen andere apps gevonden", "No other apps found")), None));
        } else {
            for app in apps {
                let name = app["name"].as_str().unwrap_or("");
                let id = app["id"].as_str().unwrap_or("");
                if name.is_empty() || id.is_empty() {
                    continue;
                }
                let label = if app["isDefault"] == true { format!("{name} ({})", t("standaard", "default")) } else { name.to_owned() };
                sub.append_item(&radio(&label, "dt.open-with", id, None));
            }
        }
        open.append_submenu(Some(&t("Openen met", "Open with")), &sub);
    }
    menu.append_section(None, &open);

    let more = gio::Menu::new();
    if has_files && !home_only {
        more.append_item(&entry(&t("Comprimeren tot ZIP-bestand", "Compress to ZIP file"), "dt.compress", None));
        if one.is_some_and(is_archive) {
            more.append_item(&entry(&t("Alles uitpakken…", "Extract all…"), "dt.extract", None));
        }
    }
    if has_files || home_only {
        more.append_item(&entry(&t("Kopiëren als pad", "Copy as path"), "dt.copy-path", Some("<Control><Shift>c")));
    }
    more.append_item(&entry(&t("Eigenschappen", "Properties"), "dt.properties", Some("<Alt>Return")));
    menu.append_section(None, &more);

    if one.is_some_and(|i| i.is_dir || i.special == Some("home")) {
        let term = gio::Menu::new();
        term.append_item(&entry(&t("Openen in Terminal", "Open in Terminal"), "dt.open-terminal-item", None));
        menu.append_section(None, &term);
    }
    menu
}

fn popup(desk: &Rc<Desk>, model: &gio::Menu, x: f64, y: f64) {
    let popover = gtk::PopoverMenu::from_model_full(model, gtk::PopoverMenuFlags::NESTED);
    popover.add_css_class("dt-menu");
    popover.set_widget_name("desktop-menu");
    popover.set_has_arrow(false);
    popover.set_position(gtk::PositionType::Bottom);
    popover.set_halign(gtk::Align::Start);
    popover.set_parent(&desk.layer);
    popover.set_pointing_to(Some(&gdk::Rectangle::new(x.round() as i32, y.round() as i32, 1, 1)));
    desk.menu_open.set(true);
    let weak = Rc::downgrade(desk);
    popover.connect_closed(move |p| {
        if let Some(d) = weak.upgrade() {
            d.menu_open.set(false);
            d.layer.grab_focus();
        }
        let p = p.clone();
        // after the chosen action ran (it looks its group up through the parent)
        glib::timeout_add_local_once(Duration::from_millis(50), move || {
            if p.parent().is_some() {
                p.unparent();
            }
        });
    });
    popover.popup();
}

/// Open the background menu, or the item menu for the selection, at a point of the icon layer.
pub fn open_menu_at(desk: &Rc<Desk>, x: f64, y: f64, on_item: bool) {
    desk.finish_rename(false);
    if on_item {
        let items = desk.selected_items();
        if items.is_empty() {
            return;
        }
        {
            let mut s = desk.s.borrow_mut();
            s.menu_keys = items.iter().map(|i| i.key.clone()).collect();
            s.menu_point = Some((x, y));
        }
        let one = (items.len() == 1).then(|| items[0].clone());
        let ask = one.filter(|i| i.special.is_none() && !i.is_dir && !i.is_desktop_entry);
        let Some(file) = ask else {
            super::actions::sync(desk, &items);
            popup(desk, &item(desk, &items, &[]), x, y);
            return;
        };
        // apps.forFile, at most 350 ms
        let shown = Rc::new(Cell::new(false));
        let show = {
            let desk = desk.clone();
            let shown = shown.clone();
            move |apps: Vec<Value>| {
                if shown.replace(true) {
                    return;
                }
                super::actions::sync(&desk, &items);
                popup(&desk, &item(&desk, &items, &apps), x, y);
            }
        };
        let late = show.clone();
        glib::timeout_add_local_once(Duration::from_millis(350), move || late(Vec::new()));
        let shell = desk.shell.clone();
        glib::spawn_future_local(async move {
            let apps = super::call(&shell, "apps.forFile", json!({"path": file.path})).await;
            show(apps.ok().and_then(|a| a.as_array().cloned()).unwrap_or_default());
        });
    } else {
        {
            let mut s = desk.s.borrow_mut();
            s.menu_keys.clear();
            s.menu_point = Some((x, y));
        }
        let shown = Rc::new(Cell::new(false));
        let show = {
            let desk = desk.clone();
            let shown = shown.clone();
            move || {
                if shown.replace(true) {
                    return;
                }
                super::actions::sync(&desk, &[]);
                popup(&desk, &background(&desk), x, y);
            }
        };
        let late = show.clone();
        glib::timeout_add_local_once(Duration::from_millis(250), move || late());
        let desk = desk.clone();
        glib::spawn_future_local(async move {
            desk.load_clip().await;
            show();
        });
    }
}
