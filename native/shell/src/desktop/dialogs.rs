//! The desktop's own small windows: the Win11 content dialog (confirm, alert), the Properties dialog
//! (desktop/properties.js, General tab) and the toast with Undo (desktop/toast.js) above the taskbar.
//! Dialogs are plain windows with an app ID of their own (super::DIALOG_ID): every toplevel of this process would
//! otherwise carry the panel's ID, and the extension would take them for the taskbar.
use super::grid::{Desk, Item};
use gtk::prelude::*;
use gtk::{gdk, glib};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

/// "12,3 MB" (core/format.js bytes, Dutch decimal comma).
pub fn bytes(n: u64, lang: &str) -> String {
    if n < 1000 {
        return format!("{n} {}", if n == 1 { "byte" } else { "bytes" });
    }
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let digits = if v < 1.0 { 2 } else if v >= 100.0 { 0 } else { 1 };
    let mut text = format!("{v:.digits$}");
    if lang != "en" {
        text = text.replace('.', ",");
    }
    format!("{text} {}", UNITS[i])
}

fn date(ms: &Value, lang: &str) -> String {
    let Some(ms) = ms.as_f64().filter(|m| *m > 0.0) else { return "–".into() };
    glib::DateTime::from_unix_local((ms / 1000.0) as i64)
        .ok()
        .and_then(|d| d.format(if lang == "en" { "%-d/%-m/%Y %H:%M" } else { "%-d-%-m-%Y %H:%M" }).ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "–".into())
}

fn themed(desk: &Desk, window: &gtk::Window) {
    window.add_css_class("w11");
    window.add_css_class(if desk.shell.dark() { "w11-dark" } else { "w11-light" });
}

/// A Win11 content dialog: title, body, a footer of buttons (the first is the accent one). `done` gets the
/// index of the chosen button, None when closed otherwise.
fn dialog(
    desk: &Rc<Desk>,
    title: &str,
    body: &gtk::Widget,
    buttons: &[(String, bool)],
    danger: bool,
    done: impl FnOnce(Option<usize>) + 'static,
) -> gtk::Window {
    let window = gtk::Window::builder().title(title).resizable(false).default_width(448).build();
    // no title bar (a content dialog), but GTK's own frame and shadow
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    crate::wayland::set_app_id(&window, super::DIALOG_ID);
    window.add_css_class("dt-dialog");
    themed(desk, &window);
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.add_css_class("dt-dialog__content");
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("dt-dialog__title");
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    content.append(&heading);
    content.append(body);
    let handle = gtk::WindowHandle::new();
    handle.set_child(Some(&content));
    outer.append(&handle);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.add_css_class("dt-dialog__footer");
    footer.set_homogeneous(true);
    outer.append(&footer);
    window.set_child(Some(&outer));

    let done: Rc<RefCell<Option<Box<dyn FnOnce(Option<usize>)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    for (i, (label, primary)) in buttons.iter().enumerate() {
        let button = gtk::Button::with_label(label);
        button.add_css_class("dt-btn");
        if *primary {
            button.add_css_class(if danger { "dt-btn--danger" } else { "dt-btn--accent" });
            window.set_default_widget(Some(&button));
        }
        button.set_hexpand(true);
        let done = done.clone();
        let win = window.downgrade();
        button.connect_clicked(move |_| {
            let f = done.borrow_mut().take();
            if let Some(f) = f {
                f(Some(i));
            }
            if let Some(w) = win.upgrade() {
                w.destroy();
            }
        });
        footer.append(&button);
    }
    let on_close = done.clone();
    window.connect_close_request(move |_| {
        let f = on_close.borrow_mut().take();
        if let Some(f) = f {
            f(None);
        }
        glib::Propagation::Proceed
    });
    let keys = gtk::EventControllerKey::new();
    let win = window.downgrade();
    keys.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape
            && let Some(w) = win.upgrade()
        {
            w.close();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(keys);
    desk.dialogs.borrow_mut().push(window.downgrade());
    window.present();
    window
}

fn message(text: &str) -> gtk::Widget {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("dt-dialog__message");
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.set_max_width_chars(52);
    label.upcast()
}

pub fn confirm(
    desk: &Rc<Desk>,
    title: &str,
    text: &str,
    yes: &str,
    no: &str,
    danger: bool,
    done: impl FnOnce(bool) + 'static,
) {
    dialog(desk, title, &message(text), &[(yes.to_owned(), true), (no.to_owned(), false)], danger, move |choice| {
        done(choice == Some(0))
    });
}

pub fn alert(desk: &Rc<Desk>, title: &str, text: &str) {
    dialog(desk, title, &message(text), &[("OK".to_owned(), true)], false, |_| {});
}

/// Eigenschappen (General): name, type, location, size, size on disk, contents, dates.
pub fn properties(desk: &Rc<Desk>, items: Vec<Item>) {
    let lang = desk.shell.lang().to_owned();
    let t = |nl: &str, en: &str| desk.t(nl, en);
    let grid = gtk::Grid::new();
    grid.add_css_class("dt-props");
    grid.set_row_spacing(8);
    grid.set_column_spacing(12);
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    head.add_css_class("dt-props__head");
    let icon = gtk::Image::from_icon_name(if items.len() == 1 && !items[0].is_dir { "text-x-generic" } else { "folder" });
    icon.set_pixel_size(32);
    head.append(&icon);
    let name = if items.len() == 1 { items[0].label.clone() } else { t(&format!("{} items", items.len()), &format!("{} items", items.len())) };
    let name_label = gtk::Label::new(Some(&name));
    name_label.set_xalign(0.0);
    name_label.set_wrap(true);
    name_label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    head.append(&name_label);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    body.append(&head);
    body.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let calculating = t("Bezig met berekenen…", "Calculating…");
    let calc = gtk::Label::new(Some(&calculating));
    calc.add_css_class("dt-props__value");
    calc.set_xalign(0.0);
    grid.attach(&calc, 0, 0, 2, 1);
    body.append(&grid);
    let title = if items.len() == 1 {
        t(&format!("Eigenschappen van {name}"), &format!("{name} Properties"))
    } else {
        t("Eigenschappen", "Properties")
    };
    dialog(desk, &title, body.upcast_ref(), &[("OK".to_owned(), true)], false, |_| {});

    let args = if items.len() == 1 {
        json!({"path": items[0].path})
    } else {
        json!({"paths": items.iter().map(|i| i.path.clone()).collect::<Vec<_>>()})
    };
    let shell = desk.shell.clone();
    let weak_grid = grid.downgrade();
    let weak_icon = icon.downgrade();
    let desk = desk.clone();
    glib::spawn_future_local(async move {
        let result = super::call(&shell, "files.properties", args).await;
        let Some(grid) = weak_grid.upgrade() else { return };
        while let Some(child) = grid.first_child() {
            grid.remove(&child);
        }
        let p = match result {
            Ok(p) => p,
            Err(error) => {
                let label = gtk::Label::new(Some(&error));
                label.set_wrap(true);
                label.set_xalign(0.0);
                grid.attach(&label, 0, 0, 2, 1);
                return;
            }
        };
        if let (Some(image), Some(icon)) = (weak_icon.upgrade(), p["icon"].as_str().filter(|i| !i.is_empty())) {
            if icon.starts_with('/') {
                image.set_from_file(Some(icon));
            } else {
                image.set_icon_name(Some(icon));
            }
        }
        let t = |nl: &str, en: &str| desk.t(nl, en);
        let mut rows: Vec<(String, String)> = Vec::new();
        let kind = p["type"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| {
            if p["isDir"] == true { t("Bestandsmap", "File folder") } else { t("Bestand", "File") }
        });
        rows.push((t("Type", "Type"), kind));
        if let Some(location) = p["location"].as_str().filter(|s| !s.is_empty()) {
            rows.push((t("Locatie", "Location"), location.to_owned()));
        }
        let size = p["size"].as_u64().unwrap_or(0);
        let partial = if p["partial"] == true { "+" } else { "" };
        rows.push((t("Grootte", "Size"), format!("{}{partial}", with_bytes(size, &lang))));
        if let Some(on_disk) = p["sizeOnDisk"].as_u64() {
            rows.push((t("Grootte op schijf", "Size on disk"), with_bytes(on_disk, &lang)));
        }
        if let Some(c) = p["contains"].as_object() {
            let (files, folders) = (c.get("files").and_then(Value::as_u64).unwrap_or(0), c.get("folders").and_then(Value::as_u64).unwrap_or(0));
            let en = format!(
                "{files} {}, {folders} {}",
                if files == 1 { "file" } else { "files" },
                if folders == 1 { "folder" } else { "folders" }
            );
            rows.push((t("Bevat", "Contains"), t(&format!("{files} bestanden, {folders} mappen"), &en)));
        }
        if p.get("created").is_some_and(|c| c.as_f64().is_some()) {
            rows.push((t("Gemaakt op", "Created"), date(&p["created"], &lang)));
        }
        if p.get("modified").is_some_and(|c| c.as_f64().is_some()) {
            rows.push((t("Gewijzigd op", "Modified"), date(&p["modified"], &lang)));
        }
        if p.get("accessed").is_some_and(|c| c.as_f64().is_some()) {
            rows.push((t("Geopend op", "Accessed"), date(&p["accessed"], &lang)));
        }
        for (i, (label, value)) in rows.into_iter().enumerate() {
            let l = gtk::Label::new(Some(&label));
            l.add_css_class("dt-props__label");
            l.set_xalign(0.0);
            l.set_valign(gtk::Align::Start);
            l.set_width_chars(16);
            let v = gtk::Label::new(Some(&value));
            v.add_css_class("dt-props__value");
            v.set_xalign(0.0);
            v.set_wrap(true);
            v.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            v.set_selectable(true);
            v.set_hexpand(true);
            v.set_max_width_chars(40);
            grid.attach(&l, 0, i as i32, 1, 1);
            grid.attach(&v, 1, i as i32, 1, 1);
        }
    });
}

/// "12,3 MB (12.912.640 bytes)".
fn with_bytes(n: u64, lang: &str) -> String {
    let short = bytes(n, lang);
    if n < 1000 {
        return short;
    }
    let digits = n.to_string();
    let sep = if lang == "en" { ',' } else { '.' };
    let mut grouped = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(sep);
        }
        grouped.push(c);
    }
    format!("{short} ({grouped} bytes)")
}

/// A Win11 toast at the bottom right above the taskbar: app line with close button, title, message and an
/// optional action button. Closes itself after `timeout_ms` (0: stays).
pub fn toast(
    desk: &Rc<Desk>,
    icon: &str,
    title: &str,
    text: &str,
    action: Option<(String, Rc<dyn Fn()>)>,
    timeout_ms: u32,
) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    card.add_css_class("dt-toast");
    card.set_widget_name("desktop-toast");
    let head = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    head.add_css_class("dt-toast__head");
    let app_icon = gtk::Image::from_icon_name(icon);
    app_icon.set_pixel_size(16);
    app_icon.add_css_class("dt-toast__appicon");
    head.append(&app_icon);
    let app = gtk::Label::new(Some(&desk.t("Bureaublad", "Desktop")));
    app.set_xalign(0.0);
    app.set_hexpand(true);
    app.add_css_class("dt-toast__app");
    head.append(&app);
    let close = gtk::Button::from_icon_name("window-close-symbolic");
    close.add_css_class("dt-toast__close");
    close.set_tooltip_text(Some(&desk.t("Sluiten", "Close")));
    head.append(&close);
    card.append(&head);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 2);
    body.add_css_class("dt-toast__body");
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("dt-toast__title");
    heading.set_xalign(0.0);
    heading.set_wrap(true);
    body.append(&heading);
    let msg = gtk::Label::new(Some(text));
    msg.add_css_class("dt-toast__msg");
    msg.set_xalign(0.0);
    msg.set_wrap(true);
    msg.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    msg.set_max_width_chars(40);
    body.append(&msg);
    card.append(&body);
    if let Some((label, run)) = action {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.add_css_class("dt-toast__actions");
        let button = gtk::Button::with_label(&label);
        button.add_css_class("dt-btn");
        button.set_hexpand(true);
        button.set_widget_name("desktop-toast-action");
        let card_weak = card.downgrade();
        button.connect_clicked(move |_| {
            if let Some(c) = card_weak.upgrade() {
                close_toast(c.upcast_ref());
            }
            run();
        });
        row.append(&button);
        card.append(&row);
    }
    let card_weak = card.downgrade();
    close.connect_clicked(move |_| {
        if let Some(c) = card_weak.upgrade() {
            close_toast(c.upcast_ref());
        }
    });
    desk.toasts.append(&card);
    if timeout_ms > 0 {
        let card_weak = card.downgrade();
        glib::timeout_add_local_once(Duration::from_millis(timeout_ms as u64), move || {
            if let Some(c) = card_weak.upgrade() {
                close_toast(c.upcast_ref());
            }
        });
    }
    card
}

pub fn close_toast(card: &gtk::Widget) {
    if let Some(parent) = card.parent().and_downcast::<gtk::Box>() {
        parent.remove(card);
    }
}
