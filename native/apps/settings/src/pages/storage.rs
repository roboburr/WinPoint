//! Instellingen › Systeem › Opslag (vendor/ui/js/apps/settings/pages/storage.js): the drive with the home
//! folder, what fills the personal folder (a coloured stacked bar and a category list, like Windows 11) and
//! every drive.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, UsageBar};
use crate::widgets::{Clamp, Wrap};
use kit::chart::{Segment, StackedBar};
use kit::controls::{self as c, ProgressRing, SettingsCard, SettingsGroup};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("storage", storage)];

/// The category glyphs take their bar's colour (settings.css `.st-cat .fl-sc__icon { color: var(--cat) }`).
pub const CSS: &str = "
.st-cat-documents .w11-sc-icon { color: var(--chart-1); }
.st-cat-pictures .w11-sc-icon { color: var(--chart-2); }
.st-cat-videos .w11-sc-icon { color: var(--chart-4); }
.st-cat-music .w11-sc-icon { color: var(--chart-5); }
.st-cat-downloads .w11-sc-icon { color: var(--chart-6); }
.st-cat-desktop .w11-sc-icon { color: var(--chart-3); }
.st-cat-trash .w11-sc-icon { color: var(--chart-8); }
.st-cat-browser .w11-sc-icon { color: var(--chart-7); }
.st-cat-chat .w11-sc-icon { color: #e74856; }
.st-cat-dev .w11-sc-icon { color: #8e8cd8; }
.st-cat-appdata .w11-sc-icon { color: #00b7c3; }
.st-cat-snaps .w11-sc-icon { color: #ea6d2c; }
.st-cat-flatpak .w11-sc-icon { color: #4f9ce8; }
.st-cat-cache .w11-sc-icon { color: #a0a0a0; }
.st-cat-other .w11-sc-icon { color: #6e7c8c; }
";

fn cat_icon(id: &str) -> &'static str {
    match id {
        "documents" => "document",
        "pictures" => "image",
        "videos" => "video",
        "music" => "music",
        "downloads" => "download",
        "desktop" => "desktop",
        "trash" => "trash",
        "browser" => "globe",
        "chat" => "people",
        "dev" => "code",
        "appdata" => "app",
        "snaps" | "flatpak" => "apps",
        "cache" => "clean",
        _ => "folder",
    }
}

fn cat_color(id: &str) -> &'static str {
    match id {
        "documents" => "chart-1",
        "pictures" => "chart-2",
        "videos" => "chart-4",
        "music" => "chart-5",
        "downloads" => "chart-6",
        "desktop" => "chart-3",
        "trash" => "chart-8",
        "browser" => "chart-7",
        "chat" => "#e74856",
        "dev" => "#8e8cd8",
        "appdata" => "#00b7c3",
        "snaps" => "#ea6d2c",
        "flatpak" => "#4f9ce8",
        "cache" => "#a0a0a0",
        "other" => "#6e7c8c",
        _ => "chart-8",
    }
}

fn cat_name(c: &Value) -> String {
    let id = c["id"].as_str().unwrap_or("");
    let known = match id {
        "documents" => Some(t("Documenten", "Documents")),
        "pictures" => Some(t("Afbeeldingen", "Pictures")),
        "videos" => Some(t("Video’s", "Videos")),
        "music" => Some(t("Muziek", "Music")),
        "downloads" => Some(t("Downloads", "Downloads")),
        "desktop" => Some(t("Bureaublad", "Desktop")),
        "trash" => Some(t("Prullenbak", "Recycle Bin")),
        "browser" => Some(t("Browsers", "Browsers")),
        "chat" => Some(t("Chat-apps", "Chat apps")),
        "dev" => Some(t("Ontwikkeltools", "Developer tools")),
        "appdata" => Some(t("App-gegevens", "App data")),
        "snaps" => Some(t("Snap-apps", "Snap apps")),
        "flatpak" => Some(t("Flatpak-apps", "Flatpak apps")),
        "cache" => Some(t("Tijdelijke bestanden (cache)", "Temporary files (cache)")),
        "other" => Some(t("Overige", "Other")),
        _ => None,
    };
    known.map(str::to_owned).unwrap_or_else(|| c["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(id).to_owned())
}

fn bytes(v: &Value) -> String {
    kit::format::bytes(v.as_f64().unwrap_or(0.0))
}

fn open_path(p: &Page, path: &str) {
    let fut = p.run_with("shell.openApp", json!({ "app": "explorer", "params": { "path": path } }), None, true);
    p.spawn(async move {
        let _ = fut.await;
    });
}

fn volume_card(p: &Page, v: &Value) -> gtk::Widget {
    let pct = common::volume_percent(v);
    let full = pct >= 90.0;
    let mount = common::s(&v["mount"]).to_owned();
    // the same naming as Verkenner › Deze pc: no raw /run/media/<user>/<UUID> folders; other mounts keep their folder
    let first = |keys: &[&str]| keys.iter().find_map(|k| v[*k].as_str().filter(|s| !s.is_empty()).map(str::to_owned)).unwrap_or_default();
    let name = if v["isSystem"] == true {
        let n = first(&["name"]);
        if n.is_empty() { t("Systeemschijf", "System drive").to_owned() } else { n }
    } else {
        first(&["name", "label", "device"])
    };
    let removable_mount = mount.starts_with("/media/") || mount.starts_with("/run/media/") || mount.starts_with("/mnt/");
    let title = if removable_mount || (!mount.is_empty() && name.contains(&mount)) { name.clone() } else { format!("{name} ({mount})") };
    let icon = if v["removable"] == true || v["kind"] == "usb" { "usb" } else { "drive" };
    let card = SettingsCard::clickable(icon, &title, "", c::Chevron::Right);
    card.widget().add_css_class("st-vol-card");
    let desc = gtk::Box::new(gtk::Orientation::Vertical, 6);
    desc.add_css_class("st-vol");
    let bar = UsageBar::new(pct, None, 90.0);
    desc.append(&Clamp::new(bar.widget(), 360));
    let mut text = common::fill(t("{f} vrij van {t}", "{f} free of {t}"), &[("f", &bytes(&v["free"])), ("t", &bytes(&v["total"]))]);
    if full {
        text.push_str(&format!(" · {}", t("Bijna vol", "Almost full")));
    }
    let l = c::label(&text, if full { "st-vol-text st-full" } else { "st-vol-text" }, true);
    desc.append(&l);
    common::card_extra(&card, &desc);
    card.set_control(&common::value_label(&kit::format::percent(pct, 0)));
    let pp = p.clone();
    card.connect_clicked(move || open_path(&pp, &mount));
    card.widget().clone()
}

struct Cats {
    data: RefCell<Option<Value>>,
    ok: bool,
    show_all: Cell<bool>,
    slot: gtk::Box,
}

fn render_cats(p: &Page, st: &Rc<Cats>) {
    let group = SettingsGroup::new(t("Je persoonlijke map", "Your personal folder"), "");
    let data = st.data.borrow().clone();
    let Some(data) = data else {
        if !st.ok {
            group.add(common::notice(t("Kan niet berekenen wat er in je persoonlijke map staat.", "Can’t calculate what’s in your personal folder.")).widget());
        } else {
            let card = SettingsCard::new(
                "",
                t("Bezig met berekenen…", "Calculating…"),
                t(
                    "Win11 Shell bekijkt welke bestanden de meeste ruimte innemen. Dit kan even duren.",
                    "Win11 Shell is looking at which files take up the most space. This can take a moment.",
                ),
            );
            card.set_icon_widget(ProgressRing::new(20, None).widget());
            group.add(card.widget());
        }
        common::set_slot(&st.slot, &[group.widget().clone().upcast()]);
        return;
    };
    let mut cats: Vec<Value> = data["categories"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["size"].as_f64().unwrap_or(0.0) > 0.0).collect();
    cats.sort_by(|a, b| b["size"].as_f64().unwrap_or(0.0).total_cmp(&a["size"].as_f64().unwrap_or(0.0)));
    let total: f64 = cats.iter().map(|c| c["size"].as_f64().unwrap_or(0.0)).sum();
    let max = cats.first().and_then(|c| c["size"].as_f64()).filter(|m| *m > 0.0).unwrap_or(1.0);
    group.set_description(&common::fill(t("Wat er in {p} ruimte inneemt", "What takes up space in {p}"), &[("p", data["home"].as_str().filter(|s| !s.is_empty()).unwrap_or("~"))]));

    let bar_card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    bar_card.add_css_class("st-catbar-card");
    let bar = StackedBar::new(10, false);
    bar.set_format(kit::format::bytes);
    bar.update(
        cats.iter().map(|c| Segment::new(c["size"].as_f64().unwrap_or(0.0), cat_color(c["id"].as_str().unwrap_or("")), &cat_name(c))).collect(),
        Some(total),
    );
    bar_card.append(bar.widget());
    let legend = Wrap::new(16, 6);
    legend.add_css_class("st-catbar-legend");
    for c in cats.iter().take(8) {
        let key = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        key.add_css_class("st-catbar-key");
        let dot = gtk::DrawingArea::new();
        dot.set_content_width(8);
        dot.set_content_height(8);
        dot.set_valign(gtk::Align::Center);
        let color = cat_color(c["id"].as_str().unwrap_or("")).to_owned();
        dot.set_draw_func(move |_, cr, w, h| {
            let (r, g, b, a) = kit::chart::resolve(&color);
            cr.set_source_rgba(r, g, b, a);
            cr.arc(w as f64 / 2.0, h as f64 / 2.0, w.min(h) as f64 / 2.0, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
        });
        kit::theme::track(&dot);
        key.append(&dot);
        key.append(&gtk::Label::new(Some(&cat_name(c))));
        legend.append(&key);
    }
    bar_card.append(&legend);
    group.add(&bar_card);

    let shown = if st.show_all.get() { cats.len() } else { cats.len().min(6) };
    for c in cats.iter().take(shown) {
        let id = c["id"].as_str().unwrap_or("");
        let size = c["size"].as_f64().unwrap_or(0.0);
        let path = c["path"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
        let card = match &path {
            Some(_) => SettingsCard::clickable(cat_icon(id), &cat_name(c), "", c::Chevron::Right),
            None => SettingsCard::new(cat_icon(id), &cat_name(c), ""),
        };
        card.widget().add_css_class("st-cat");
        card.widget().add_css_class(&format!("st-cat-{id}"));
        let ub = UsageBar::new(size / max * 100.0, Some(cat_color(id)), 101.0);
        let wrap = Clamp::new(ub.widget(), 340);
        wrap.add_css_class("st-cat-bar");
        common::card_extra(&card, &wrap);
        card.set_control(&common::value_label(&kit::format::bytes(size)));
        if let Some(path) = path {
            card.widget().set_tooltip_text(Some(t("Openen in Verkenner", "Open in File Explorer")));
            let pp = p.clone();
            card.connect_clicked(move || open_path(&pp, &path));
        }
        group.add(card.widget());
    }
    if cats.len() > 6 {
        let open = st.show_all.get();
        let more = SettingsCard::clickable("", if open { t("Minder categorieën weergeven", "Show fewer categories") } else { t("Meer categorieën weergeven", "Show more categories") }, "", c::Chevron::None);
        more.widget().add_css_class("st-more");
        more.widget().set_widget_name("storage-more");
        let chev = kit::icons::glyph(if open { "chevron-up" } else { "chevron-down" }, 12);
        chev.add_css_class("st-more-chev");
        more.set_control(&chev);
        let (pp, st2) = (p.clone(), st.clone());
        more.connect_clicked(move || {
            st2.show_all.set(!st2.show_all.get());
            render_cats(&pp, &st2);
        });
        group.add(more.widget());
    }
    common::set_slot(&st.slot, &[group.widget().clone().upcast()]);
}

fn storage(p: Page) -> PageFuture {
    Box::pin(async move {
        let (vols, cats) = (p.call("storage.volumes", json!({})), p.call("storage.categories", json!({ "refresh": false })));
        let (vols, cats) = (vols.await, cats.await);
        if !p.alive() {
            return Content::new();
        }
        let vols: Option<Vec<Value>> = vols.ok().and_then(|v| v.as_array().cloned());
        let mut out: Content = Vec::new();
        if vols.is_none() {
            out.push(common::notice(t("Opslaginformatie is op deze computer niet beschikbaar.", "Storage information isn’t available on this computer.")).widget().clone().upcast());
        }
        let list = vols.clone().unwrap_or_default();
        let sys = list.iter().find(|v| v["isHome"] == true).or_else(|| list.iter().find(|v| v["isSystem"] == true)).or_else(|| list.first()).cloned();

        // ---- the drive with the home folder
        if let Some(sys) = &sys {
            let pct = common::volume_percent(sys);
            let sum = gtk::Box::new(gtk::Orientation::Vertical, 12);
            sum.add_css_class("st-storage-sum");
            let head = Wrap::new(16, 4);
            let name = sys["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(t("Systeemschijf", "System drive"));
            let title = c::label(&format!("{name} ({}) – {}", common::s(&sys["mount"]), bytes(&sys["total"])), "st-storage-title", true);
            title.set_hexpand(true);
            head.append(&title);
            let nums = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            nums.add_css_class("st-storage-nums");
            nums.set_valign(gtk::Align::Center);
            nums.append(&gtk::Label::new(Some(&common::fill(t("{u} gebruikt", "{u} used"), &[("u", &bytes(&sys["used"]))]))));
            nums.append(&gtk::Label::new(Some(&common::fill(t("{f} vrij", "{f} free"), &[("f", &bytes(&sys["free"]))]))));
            head.append(&nums);
            sum.append(&head);
            let bar = UsageBar::new(pct, None, 90.0);
            bar.set_thickness(8);
            sum.append(bar.widget());
            out.push(sum.upcast());
        }

        // ---- categories
        let (ok, ready_data) = match &cats {
            Ok(v) => (true, (v["ready"] == true).then(|| v["data"].clone())),
            Err(_) => (false, None),
        };
        let st = Rc::new(Cats { data: RefCell::new(ready_data), ok, show_all: Cell::new(false), slot: common::slot() });
        render_cats(&p, &st);
        if st.data.borrow().is_none() {
            let (pp, st2) = (p.clone(), st.clone());
            p.on("storage.categoriesReady", move |d| {
                if d["data"].is_object() {
                    *st2.data.borrow_mut() = Some(d["data"].clone());
                    render_cats(&pp, &st2);
                }
            });
            // fall back to polling in case the event was missed
            let (pp, st2) = (p.clone(), st.clone());
            p.timeout(4000, move || {
                if st2.data.borrow().is_some() {
                    return;
                }
                let fut = pp.get("storage.categories", json!({ "refresh": false }));
                let p2 = pp.clone();
                pp.spawn(async move {
                    if let Some(r) = fut.await
                        && r["ready"] == true
                        && st2.data.borrow().is_none()
                        && p2.alive()
                    {
                        *st2.data.borrow_mut() = Some(r["data"].clone());
                        render_cats(&p2, &st2);
                    }
                });
            });
        }
        out.push(st.slot.clone().upcast());

        // ---- clean up
        let clean = SettingsCard::new(
            "broom",
            t("Ruimte vrijmaken", "Free up space"),
            t("Bekijk wat veilig kan worden opgeruimd, zoals de Prullenbak en tijdelijke bestanden", "See what can safely be cleaned up, such as the Recycle Bin and temporary files"),
        );
        let btn = c::button(t("Ruimte vrijmaken", "Free up space"), c::Variant::Accent);
        btn.set_widget_name("storage-cleanup");
        let pp = p.clone();
        btn.connect_clicked(move |_| {
            let fut = pp.run_with("shell.openApp", json!({ "app": "taskmgr", "params": { "page": "storage" } }), None, true);
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
        clean.set_control(&btn);
        let g = SettingsGroup::new("", "");
        g.add(clean.widget());
        out.push(g.widget().clone().upcast());

        // ---- drives
        if !list.is_empty() {
            let g = SettingsGroup::new(t("Stations", "Drives"), t("Alle schijven en USB-sticks op deze pc", "All disks and USB sticks on this PC"));
            for v in &list {
                g.add(&volume_card(&p, v));
            }
            out.push(g.widget().clone().upcast());
        }
        out
    })
}
