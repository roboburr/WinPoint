//! Instellingen › Accounts › Je info (Windows 11: Accounts › Your info; batch E, notes/claude/settings-parity.md §2):
//! the account picture and name, as GNOME Settings' Users panel changes them through AccountsService
//! (`users.setIcon`: a file, one of GNOME's stock pictures, or none; `users.setRealName`).
//!
//! Windows' "Een foto maken · Camera openen" is left out: GNOME Settings 50 on Ubuntu has no camera capture in its
//! avatar chooser either.
use super::accounts::{refresh_on, user_hero};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Dialog, DialogResult, SettingsCard, SettingsExpander, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("yourinfo", page)];
pub const CSS: &str = "";

/// Set the account picture (a path, or "" to remove it), then the page follows users.changed.
fn set_icon(p: &Page, path: &str) {
    let wait = p.run("users.setIcon", json!({ "path": path }));
    p.spawn(async move {
        let _ = wait.await;
    });
}

/// GTK's file dialog for a picture (the desktop's FileChooser portal), as Windows' "Bladeren door bestanden".
fn browse(p: &Page) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(t("Afbeeldingen", "Pictures")));
    for pattern in ["*.jpg", "*.jpeg", "*.png", "*.webp", "*.gif", "*.bmp", "*.svg"] {
        filter.add_pattern(pattern);
    }
    filter.add_mime_type("image/*");
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::new();
    dialog.set_title(t("Een afbeelding kiezen voor je account", "Choose a picture for your account"));
    dialog.set_modal(true);
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));
    if let Some(pictures) = glib::user_special_dir(glib::UserDirectory::Pictures) {
        dialog.set_initial_folder(Some(&gio::File::for_path(pictures)));
    }
    let pp = p.clone();
    dialog.open(Some(p.window().window()), gio::Cancellable::NONE, move |result| match result {
        Ok(file) => {
            if let Some(path) = file.path().and_then(|p| p.to_str().map(str::to_owned)) {
                set_icon(&pp, &path);
            }
        }
        Err(e) if e.matches(gtk::DialogError::Dismissed) || e.matches(gtk::DialogError::Cancelled) => {}
        Err(e) => pp.error(&kit::CallError::new("picker", e.message()), None),
    });
}

/// One stock picture of GNOME's gallery as a round `size` px tile. A gtk::Image with a pixel size: a gtk::Picture
/// asks for its texture's own size (the harness saw common::avatar's 64 px tiles drawn at 128 px).
fn face_tile(path: &str, size: i32) -> gtk::Widget {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.add_css_class("w11-avatar");
    tile.add_css_class("st-avatar");
    tile.set_overflow(gtk::Overflow::Hidden);
    tile.set_size_request(size, size);
    tile.set_halign(gtk::Align::Center);
    tile.set_valign(gtk::Align::Center);
    let image = gtk::Image::new();
    image.set_pixel_size(size);
    image.set_hexpand(true);
    tile.append(&image);
    let path = path.to_owned();
    let weak = image.downgrade();
    kit::spawn(
        move || gdk::Texture::from_filename(&path).ok(),
        move |t: Option<gdk::Texture>| {
            if let (Some(image), Some(t)) = (weak.upgrade(), t) {
                image.set_paintable(Some(&t));
            }
        },
    );
    tile.upcast()
}

/// "Naam wijzigen": the full name (AccountsService SetRealName).
fn rename(p: &Page, current: &str) {
    let entry = c::text_box("");
    entry.set_text(current);
    entry.set_widget_name("st-yourinfo-name-entry");
    entry.set_max_length(255);
    let body = c::field(t("Volledige naam", "Full name"), &entry, Some(t("Deze naam staat op het aanmeldingsscherm en in Start.", "This name appears on the sign-in screen and in Start.")));
    let pp = p.clone();
    let e2 = entry.clone();
    let dialog = Dialog::new(t("Je naam wijzigen", "Change your name")).body(&body).primary(t("Opslaan", "Save")).width(420).on_primary(move |d| {
        let name = e2.text().trim().to_owned();
        if name.is_empty() {
            return false;
        }
        d.set_busy(true);
        let (d2, wait) = (d.clone(), pp.run("users.setRealName", json!({ "name": name })));
        pp.spawn(async move {
            let ok = wait.await.is_some();
            d2.set_busy(false);
            if ok {
                d2.close(DialogResult::Primary);
            }
        });
        false
    });
    {
        let d = dialog.clone();
        entry.connect_changed(move |e| d.set_primary_enabled(!e.text().trim().is_empty()));
    }
    dialog.show(&p.anchor(), |_| {});
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let (me, faces) = (p.call("users.self", json!({})), p.call("users.faces", json!({})));
        let (me, faces) = (me.await, faces.await);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let u = me.unwrap_or(Value::Null);
        out.push(user_hero(&u, 160).upcast());
        let ok = u["available"] == true;
        if !ok {
            out.push(common::notice(t("Gebruikersaccounts zijn op deze computer niet beschikbaar.", "User accounts aren’t available on this computer.")).widget().clone().upcast());
        }
        let has_picture = u["iconFile"].as_str().is_some_and(|s| !s.is_empty());

        // Windows: Je foto aanpassen (Een bestand kiezen · Bladeren door bestanden), GNOME's gallery and Remove
        let photo = SettingsGroup::new(t("Je foto aanpassen", "Adjust your photo"), "");
        let file_card = SettingsCard::new("image", t("Een bestand kiezen", "Choose a file"), "");
        let browse_btn = c::button(t("Bladeren door bestanden", "Browse files"), Variant::Standard);
        browse_btn.set_widget_name("st-yourinfo-browse");
        {
            let pp = p.clone();
            browse_btn.connect_clicked(move |_| browse(&pp));
        }
        file_card.set_control(&browse_btn);
        file_card.set_sensitive(ok);
        photo.add(file_card.widget());
        let list = faces.ok().map(|f| f["faces"].as_array().cloned().unwrap_or_default()).unwrap_or_default();
        if !list.is_empty() {
            let gallery = SettingsExpander::new("user", t("Een standaardafbeelding kiezen", "Choose a default picture"), t("De afbeeldingen die bij Ubuntu horen", "The pictures that come with Ubuntu"));
            gallery.header_button().set_widget_name("st-yourinfo-gallery");
            let wrap = crate::widgets::Wrap::new(8, 8);
            wrap.set_margin_top(12);
            wrap.set_margin_bottom(12);
            let current = u["iconFile"].as_str().unwrap_or("").to_owned();
            for face in &list {
                let path = face["path"].as_str().unwrap_or("").to_owned();
                let name = face["name"].as_str().unwrap_or("").to_owned();
                let b = gtk::Button::new();
                b.add_css_class("w11-button");
                b.add_css_class("st-face");
                b.set_has_frame(false);
                b.set_widget_name(&format!("st-yourinfo-face-{name}"));
                b.set_tooltip_text(Some(&name));
                b.set_child(Some(&face_tile(&path, 64)));
                // AccountsService copies a stock picture under the user's name: compare by file name only as a hint
                if !current.is_empty() && std::path::Path::new(&current).file_stem().is_some_and(|s| s.to_string_lossy() == name) {
                    b.add_css_class("st-current");
                }
                let pp = p.clone();
                b.connect_clicked(move |_| set_icon(&pp, &path));
                wrap.append(&b);
            }
            gallery.add(&wrap);
            gallery.widget().set_sensitive(ok);
            photo.add(gallery.widget());
        }
        if has_picture {
            let remove = SettingsCard::new("delete", t("Foto verwijderen", "Remove picture"), t("Je initialen gebruiken in plaats van een foto", "Use your initials instead of a picture"));
            let remove_btn = c::button(t("Verwijderen", "Remove"), Variant::Standard);
            remove_btn.set_widget_name("st-yourinfo-remove");
            {
                let pp = p.clone();
                remove_btn.connect_clicked(move |_| set_icon(&pp, ""));
            }
            remove.set_control(&remove_btn);
            remove.set_sensitive(ok);
            photo.add(remove.widget());
        }
        out.push(photo.widget().clone().upcast());

        // WinPoint addition (Windows keeps it in the Control Panel): GNOME's Name row
        let account = SettingsGroup::new(t("Accountinstellingen", "Account settings"), "");
        let name_card = SettingsCard::new("rename", t("Naam wijzigen", "Change name"), &super::accounts::display_name(&u));
        let name_btn = c::button(t("Wijzigen", "Change"), Variant::Standard);
        name_btn.set_widget_name("st-yourinfo-rename");
        {
            let (pp, current) = (p.clone(), u["realName"].as_str().unwrap_or("").to_owned());
            name_btn.connect_clicked(move |_| rename(&pp, &current));
        }
        name_card.set_control(&name_btn);
        name_card.set_sensitive(ok);
        account.add(name_card.widget());
        out.push(account.widget().clone().upcast());

        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "signin").widget());
        related.add(common::link_card(&p, "otherusers").widget());
        out.push(related.widget().clone().upcast());
        refresh_on(&p, "users.changed");
        out
    })
}
