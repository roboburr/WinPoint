//! Opslag > Grote bestanden (js/apps/taskmgr/storage/largefiles.js): the big files of a folder
//! (storage.largeFiles) with thumbnails or icons; tick the ones you no longer need and move them to the Prullenbak
//! (files.trash, never deleted outright). Files that belong to an app or the system are flagged, left out of
//! "Alles selecteren" and warned about.
use super::Scope;
use super::explorer::guess_mime;
use super::layout::{self, Bar, Cover};
use crate::app::App;
use crate::hostcall;
use crate::util::{self, fill, plural};
use kit::controls::{Dialog, DialogResult, Dropdown, InfoBar, Severity, Toast, Variant};
use kit::format::bytes;
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "filter" => ("Groter dan", "Larger than"),
        "in" => ("in {where}", "in {where}"),
        "refresh" => ("Vernieuwen", "Refresh"),
        "loading" => ("Grote bestanden zoeken\u{2026}", "Looking for large files\u{2026}"),
        "none" => ("Geen grote bestanden gevonden", "No large files found"),
        "noneBody" => ("Er staan hier geen bestanden die groter zijn dan {min}.", "There are no files larger than {min} here."),
        "name" => ("Naam", "Name"),
        "modified" => ("Gewijzigd", "Modified"),
        "size" => ("Grootte", "Size"),
        "selectAll" => ("Alles selecteren", "Select all"),
        "review" => ("Controleren voor je opruimt", "Review before you clean up"),
        "selected0" => ("Vink de bestanden aan die je niet meer nodig hebt.", "Tick the files you no longer need."),
        "selectedOne" => ("1 bestand geselecteerd", "1 file selected"),
        "selectedOther" => ("{n} bestanden geselecteerd", "{n} files selected"),
        "total" => ("Samen {size}", "{size} in total"),
        "toTrash" => ("Naar Prullenbak verplaatsen", "Move to Recycle Bin"),
        "trashNote" => (
            "Bestanden gaan naar de Prullenbak. Je kunt ze daar terugzetten, zolang je de Prullenbak niet leegmaakt.",
            "Files are moved to the Recycle Bin. You can restore them from there until you empty the Recycle Bin.",
        ),
        "confirmOne" => ("1 bestand naar de Prullenbak verplaatsen?", "Move 1 file to the Recycle Bin?"),
        "confirmOther" => ("{n} bestanden naar de Prullenbak verplaatsen?", "Move {n} files to the Recycle Bin?"),
        "confirmBody" => (
            "Dit maakt {size} vrij zodra je de Prullenbak leegmaakt. Tot die tijd kun je alles terugzetten.",
            "This frees up {size} once you empty the Recycle Bin. Until then, you can restore everything.",
        ),
        "move" => ("Verplaatsen", "Move"),
        "cancel" => ("Annuleren", "Cancel"),
        "movedOne" => ("1 bestand staat in de Prullenbak", "1 file is in the Recycle Bin"),
        "movedOther" => ("{n} bestanden staan in de Prullenbak", "{n} files are in the Recycle Bin"),
        "undoHint" => ("Je kunt ze terugzetten vanuit de Prullenbak", "You can restore them from the Recycle Bin"),
        "openTrash" => ("Prullenbak openen", "Open Recycle Bin"),
        "failedOne" => ("1 bestand kon niet worden verplaatst", "1 file couldn\u{2019}t be moved"),
        "failedOther" => ("{n} bestanden konden niet worden verplaatst", "{n} files couldn\u{2019}t be moved"),
        "failedAll" => ("Verplaatsen is niet gelukt", "Couldn\u{2019}t move the files"),
        "openLocation" => ("Bestandslocatie openen", "Open file location"),
        "deselect" => ("Selectie wissen", "Clear selection"),
        "loadFailed" => ("Kon de lijst niet ophalen", "Couldn\u{2019}t load the list"),
        "appOwned" => ("Hoort bij een app", "Belongs to an app"),
        "appOwnedTip" => (
            "Dit bestand hoort bij een app of bij het systeem. Laat het staan, tenzij je zeker weet dat je het niet meer nodig hebt.",
            "This file belongs to an app or to the system. Leave it unless you\u{2019}re sure you no longer need it.",
        ),
        "appWarnTitle" => ("Sommige bestanden horen bij een app", "Some files belong to an app"),
        "appWarnBody" => (
            "Bijvoorbeeld van Steam of een virtuele machine. Die app werkt daarna misschien niet meer goed.",
            "For example Steam or a virtual machine. That app may not work properly afterwards.",
        ),
        "dryRun" => ("Testmodus: er is niets echt verplaatst.", "Test mode: nothing was actually moved."),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

const MB: f64 = 1_048_576.0;
const GB: f64 = 1_073_741_824.0;
const SIZES: [(f64, &str); 5] = [(50.0 * MB, "50 MB"), (100.0 * MB, "100 MB"), (500.0 * MB, "500 MB"), (GB, "1 GB"), (5.0 * GB, "5 GB")];

/// Files that belong to an app or the system rather than to the user (largefiles.js isAppOwned): game data, app
/// folders (any hidden folder such as ~/.local/share, ~/.var/app, ~/.cache), snaps, app-managed folders, system
/// folders and virtual disks. Trashing them can break that app.
pub fn is_app_owned(f: &Value) -> bool {
    if let Some(b) = f["appOwned"].as_bool() {
        return b;
    }
    let path = util::text(&f["path"]);
    let name = util::text(&f["name"]).to_lowercase();
    let parts: Vec<&str> = path.split('/').collect();
    let hidden_dir = parts.len() > 1 && parts[..parts.len() - 1].iter().any(|p| p.len() > 1 && p.starts_with('.'));
    let inside = ["/snap/", "/steamapps/", "/Android/Sdk/", "/node_modules/", "/VirtualBox VMs/"].iter().any(|s| path.contains(s));
    let system = ["/usr/", "/opt/", "/var/", "/etc/", "/lib/", "/lib64/", "/boot/"].iter().any(|s| path.starts_with(s));
    let ext = [".qcow2", ".vdi", ".vmdk", ".vhd", ".vhdx", ".vpk", ".pak", ".asar"].iter().any(|e| name.ends_with(e));
    hidden_dir || inside || system || ext
}

fn mime_of(f: &Value) -> String {
    let m = util::text(&f["mime"]);
    if m.is_empty() { guess_mime(&util::text(&f["name"])).to_owned() } else { m }
}

struct FileRow {
    path: String,
    row: gtk::Box,
    check: gtk::CheckButton,
}

pub struct LargeFiles {
    me: Weak<LargeFiles>,
    app: Rc<App>,
    on_changed: Box<dyn Fn()>,
    scope: RefCell<Option<Scope>>,
    min_size: Cell<f64>,
    files: RefCell<Vec<Value>>,
    checked: RefCell<HashSet<String>>,
    seq: Cell<u64>,
    syncing: Cell<bool>,
    root: gtk::Box,
    where_label: gtk::Label,
    rows: gtk::Box,
    file_rows: RefCell<Vec<FileRow>>,
    all_box: gtk::CheckButton,
    rev_count: gtk::Label,
    rev_total: gtk::Label,
    rev_list: gtk::Box,
    rev_scroll: gtk::ScrolledWindow,
    trash_btn: gtk::Button,
    clear_btn: gtk::Button,
}

impl LargeFiles {
    pub fn new(app: &Rc<App>, on_changed: impl Fn() + 'static) -> Rc<LargeFiles> {
        let options: Vec<(String, &str)> = SIZES.iter().map(|(v, l)| ((*v as u64).to_string(), *l)).collect();
        let opt_refs: Vec<(&str, &str)> = options.iter().map(|(v, l)| (v.as_str(), *l)).collect();
        let size_dd = Dropdown::simple(&opt_refs, Some(&((100.0 * MB) as u64).to_string()));
        size_dd.widget().set_size_request(120, -1);
        size_dd.widget().set_widget_name("taskmgr-large-min");
        let where_label = layout::label("", "taskmgr-lf-where");
        let refresh = layout::subtle("refresh", tr("refresh"));
        refresh.set_widget_name("taskmgr-large-refresh");
        let bar = layout::hbox(10, "taskmgr-lf-bar");
        let fl = layout::label(tr("filter"), "taskmgr-text-2");
        fl.set_ellipsize(gtk::pango::EllipsizeMode::None);
        bar.append(&fl);
        bar.append(size_dd.widget());
        bar.append(&where_label);
        let grow = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        grow.set_hexpand(true);
        bar.append(&grow);
        bar.append(&refresh);

        let all_box = kit::controls::checkbox("");
        all_box.set_tooltip_text(Some(tr("selectAll")));
        all_box.set_widget_name("taskmgr-large-all");
        all_box.set_size_request(28, -1);
        let head = layout::hbox(12, "taskmgr-lf-head");
        head.append(&all_box);
        let hn = layout::label(tr("name"), "");
        hn.set_hexpand(true);
        hn.set_margin_start(60);
        head.append(&hn);
        for t in [tr("modified"), tr("size")] {
            let l = layout::label(t, "");
            l.set_xalign(1.0);
            l.set_size_request(120, -1);
            head.append(&l);
        }
        let rows = layout::vbox(0, "taskmgr-lf-rows");
        rows.set_widget_name("taskmgr-large-files");
        let list_card = layout::vbox(0, "taskmgr-card taskmgr-lf-list");
        list_card.append(&head);
        list_card.append(&layout::max_scroll(&rows, 560));

        let rev_count = layout::wrap("", "taskmgr-lf-revcount");
        let rev_total = layout::label("", "taskmgr-lf-revtotal");
        let rev_list = layout::vbox(0, "taskmgr-lf-revlist");
        let rev_scroll = layout::max_scroll(&rev_list, 200);
        let trash_btn = kit::controls::button_with_glyph("trash", tr("toTrash"), Variant::Accent);
        trash_btn.set_widget_name("taskmgr-large-trash");
        trash_btn.set_sensitive(false);
        let clear_btn = kit::controls::button(tr("deselect"), Variant::Subtle);
        clear_btn.add_css_class("w11-small");
        clear_btn.set_halign(gtk::Align::Center);
        clear_btn.set_visible(false);
        let rev_head = layout::hbox(10, "taskmgr-lf-revhead");
        rev_head.append(&kit::icons::glyph("trash", 20));
        rev_head.append(&layout::wrap(tr("review"), "taskmgr-lf-revtitle"));
        let review = layout::vbox(10, "taskmgr-card taskmgr-lf-review");
        review.append(&rev_head);
        review.append(&rev_count);
        review.append(&rev_total);
        review.append(&rev_scroll);
        review.append(&trash_btn);
        review.append(&clear_btn);
        review.append(&layout::wrap(tr("trashNote"), "taskmgr-lf-note w11-caption"));
        // the list and a 300 px review panel; one column below a 900 px page
        let body = layout::Layout::columns(12, 12, |w| if w > 844 { vec![w - 12 - 300, 300] } else { vec![w] }).valign_children(layout::VAlign::Start);
        body.append(&list_card);
        body.append(&review);
        let root = layout::vbox(12, "taskmgr-lf");
        root.append(&bar);
        root.append(&body);

        let lf = Rc::new_cyclic(|me: &Weak<LargeFiles>| LargeFiles {
            me: me.clone(),
            app: app.clone(),
            on_changed: Box::new(on_changed),
            scope: RefCell::new(None),
            min_size: Cell::new(100.0 * MB),
            files: RefCell::new(Vec::new()),
            checked: RefCell::new(HashSet::new()),
            seq: Cell::new(0),
            syncing: Cell::new(false),
            root,
            where_label,
            rows,
            file_rows: RefCell::new(Vec::new()),
            all_box,
            rev_count,
            rev_total,
            rev_list,
            rev_scroll,
            trash_btn,
            clear_btn,
        });
        let w = Rc::downgrade(&lf);
        size_dd.connect_changed(move |v| {
            if let Some(lf) = w.upgrade() {
                lf.min_size.set(v.parse::<f64>().unwrap_or(100.0 * MB));
                lf.load();
            }
        });
        let w = Rc::downgrade(&lf);
        refresh.connect_clicked(move |_| {
            if let Some(lf) = w.upgrade() {
                lf.load();
            }
        });
        // "Alles selecteren" never ticks files that belong to an app (they can still be ticked one by one)
        let w = Rc::downgrade(&lf);
        lf.all_box.connect_toggled(move |b| {
            let Some(lf) = w.upgrade() else { return };
            if lf.syncing.get() {
                return;
            }
            if b.is_active() {
                let paths: Vec<String> = lf.files.borrow().iter().filter(|f| !is_app_owned(f)).map(|f| util::text(&f["path"])).collect();
                lf.checked.borrow_mut().extend(paths);
            } else {
                lf.checked.borrow_mut().clear();
            }
            lf.render_checks();
        });
        let w = Rc::downgrade(&lf);
        lf.trash_btn.connect_clicked(move |_| {
            if let Some(lf) = w.upgrade() {
                lf.move_to_trash();
            }
        });
        let w = Rc::downgrade(&lf);
        lf.clear_btn.connect_clicked(move |_| {
            if let Some(lf) = w.upgrade() {
                lf.checked.borrow_mut().clear();
                lf.render_checks();
            }
        });
        lf
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn min_label(&self) -> String {
        let m = self.min_size.get();
        SIZES.iter().find(|(v, _)| (*v - m).abs() < 1.0).map(|(_, l)| (*l).to_owned()).unwrap_or_else(|| bytes(m))
    }

    pub fn load(&self) {
        let Some(scope) = self.scope.borrow().clone() else { return };
        let my = self.seq.get() + 1;
        self.seq.set(my);
        let where_ = if scope.label.is_empty() { util::pretty_path(&scope.path) } else { scope.label.clone() };
        self.where_label.set_label(&fill(tr("in"), &[("where", &where_)]));
        kit::controls::clear(&self.rows);
        self.file_rows.borrow_mut().clear();
        self.rows.append(&layout::ring_note(28, tr("loading")));
        let w = self.me.clone();
        hostcall::call("storage.largeFiles", json!({ "path": scope.path, "limit": 100, "minSize": self.min_size.get() }), move |r| {
            let Some(lf) = w.upgrade() else { return };
            if lf.seq.get() != my {
                return;
            }
            match r {
                Ok(list) => {
                    let files: Vec<Value> = list.as_array().cloned().unwrap_or_default();
                    let valid: HashSet<String> = files.iter().map(|f| util::text(&f["path"])).collect();
                    lf.checked.borrow_mut().retain(|p| valid.contains(p));
                    *lf.files.borrow_mut() = files;
                    lf.render_rows();
                }
                Err(e) => {
                    kit::controls::clear(&lf.rows);
                    lf.file_rows.borrow_mut().clear();
                    lf.rows.append(&layout::empty_line(&format!("{}: {}", tr("loadFailed"), e.message)));
                }
            }
        });
    }

    fn thumb_for(&self, f: &Value) -> gtk::Widget {
        let mime = mime_of(f);
        let media = mime.starts_with("image/") || mime.starts_with("video/");
        let slot = layout::hbox(0, "taskmgr-lf-thumb");
        slot.set_size_request(48, 36);
        slot.set_hexpand(false);
        slot.set_valign(gtk::Align::Center);
        let icon = kit::icons::file_icon(&mime, 32);
        icon.set_hexpand(true);
        icon.set_halign(gtk::Align::Center);
        if !media {
            slot.append(&icon);
            return slot.upcast();
        }
        slot.add_css_class("taskmgr-is-media");
        let cover = Cover::new(48, 36, 4.0);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&cover));
        if mime.starts_with("video/") {
            let play = layout::hbox(0, "taskmgr-lf-play");
            play.set_size_request(16, 16);
            play.set_halign(gtk::Align::End);
            play.set_valign(gtk::Align::End);
            play.set_margin_end(3);
            play.set_margin_bottom(3);
            let g = kit::icons::glyph("play", 12);
            g.set_hexpand(true);
            play.append(&g);
            overlay.add_overlay(&play);
        }
        slot.append(&overlay);
        let (sw, cw) = (slot.downgrade(), cover.downgrade());
        kit::icons::thumbnail(&util::text(&f["path"]), 48, move |tex| {
            let (Some(slot), Some(cover)) = (sw.upgrade(), cw.upgrade()) else { return };
            match tex {
                Some(t) => cover.set_texture(Some(&t)),
                None => {
                    // no thumbnail: the file's icon instead
                    kit::controls::clear(&slot);
                    slot.remove_css_class("taskmgr-is-media");
                    slot.append(&icon);
                }
            }
        });
        slot.upcast()
    }

    fn render_rows(&self) {
        kit::controls::clear(&self.rows);
        self.file_rows.borrow_mut().clear();
        let files = self.files.borrow().clone();
        if files.is_empty() {
            let e = kit::controls::empty_state("success", tr("none"), &fill(tr("noneBody"), &[("min", &self.min_label())]), None);
            self.rows.append(&e);
            self.render_checks();
            return;
        }
        let max_size = files.iter().map(|f| util::num(&f["size"])).fold(1.0, f64::max);
        for f in &files {
            let path = util::text(&f["path"]);
            let name = util::text(&f["name"]);
            let size = util::num(&f["size"]);
            let app = is_app_owned(f);
            let check = kit::controls::checkbox("");
            check.set_active(self.checked.borrow().contains(&path));
            check.set_tooltip_text(Some(&name));
            check.set_size_request(28, -1);
            check.set_valign(gtk::Align::Center);
            let w = self.me.clone();
            let p = path.clone();
            check.connect_toggled(move |c| {
                let Some(lf) = w.upgrade() else { return };
                if lf.syncing.get() {
                    return;
                }
                if c.is_active() {
                    lf.checked.borrow_mut().insert(p.clone());
                } else {
                    lf.checked.borrow_mut().remove(&p);
                }
                lf.render_checks();
            });
            let text = layout::vbox(0, "");
            text.set_hexpand(true);
            text.set_valign(gtk::Align::Center);
            text.append(&layout::label(&name, "taskmgr-lf-name"));
            let dir = layout::hbox(0, "taskmgr-lf-dir");
            if app {
                let badge = layout::hbox(4, "taskmgr-lf-app");
                badge.append(&kit::icons::glyph("warning", 12));
                let bl = layout::label(tr("appOwned"), "");
                bl.set_ellipsize(gtk::pango::EllipsizeMode::None);
                badge.append(&bl);
                badge.set_tooltip_text(Some(tr("appOwnedTip")));
                dir.append(&badge);
                dir.append(&layout::label("\u{b7}", "taskmgr-lf-sep"));
            }
            dir.append(&layout::label(&util::pretty_path(&util::dir_name(&path)), ""));
            text.append(&dir);
            let loc = kit::controls::icon_button("folder-open", tr("openLocation"), 28);
            loc.add_css_class("taskmgr-lf-loc");
            let w = self.me.clone();
            let p = path.clone();
            loc.connect_clicked(move |_| {
                if let Some(lf) = w.upgrade() {
                    lf.open_location(&p);
                }
            });
            let modified = f["modified"].as_f64().filter(|m| *m > 0.0);
            let date = layout::label(&modified.map(|m| kit::format::rel_time(m, kit::format::now_ms())).unwrap_or_else(|| "\u{2013}".into()), "taskmgr-lf-date");
            date.set_xalign(1.0);
            date.set_size_request(120, -1);
            if let Some(m) = modified {
                date.set_tooltip_text(Some(&kit::format::date(m, kit::format::DateFormat::DateTime)));
            }
            let size_box = layout::vbox(4, "taskmgr-lf-size");
            size_box.set_size_request(120, -1);
            size_box.set_hexpand(false);
            size_box.set_valign(gtk::Align::Center);
            let sl = layout::label(&bytes(size), "");
            sl.set_xalign(1.0);
            size_box.append(&sl);
            let sb = Bar::new(3, "taskmgr-lf-sizebar");
            sb.set_fill((size / max_size).max(0.03), "accent");
            size_box.append(sb.widget());
            let row = layout::hbox(12, if app { "taskmgr-lf-row taskmgr-is-app" } else { "taskmgr-lf-row" });
            row.append(&check);
            row.append(&self.thumb_for(f));
            row.append(&text);
            row.append(&loc);
            row.append(&date);
            row.append(&size_box);
            // a click on the row ticks it (not on the check box or the location button)
            let click = gtk::GestureClick::new();
            let w = self.me.clone();
            let (p, rw) = (path.clone(), row.downgrade());
            click.connect_released(move |_, _, x, y| {
                let (Some(lf), Some(row)) = (w.upgrade(), rw.upgrade()) else { return };
                if let Some(t) = row.pick(x, y, gtk::PickFlags::DEFAULT)
                    && (t.ancestor(gtk::CheckButton::static_type()).is_some() || t.ancestor(gtk::Button::static_type()).is_some())
                {
                    return;
                }
                let on = !lf.checked.borrow().contains(&p);
                if on {
                    lf.checked.borrow_mut().insert(p.clone());
                } else {
                    lf.checked.borrow_mut().remove(&p);
                }
                lf.render_checks();
            });
            row.add_controller(click);
            self.rows.append(&row);
            self.file_rows.borrow_mut().push(FileRow { path, row, check });
        }
        self.render_checks();
    }

    fn render_checks(&self) {
        self.syncing.set(true);
        let checked = self.checked.borrow().clone();
        for r in self.file_rows.borrow().iter() {
            let on = checked.contains(&r.path);
            if r.check.is_active() != on {
                r.check.set_active(on);
            }
            util::set_class(&r.row, "taskmgr-checked", on);
        }
        let files = self.files.borrow().clone();
        let sel: Vec<&Value> = files.iter().filter(|f| checked.contains(&util::text(&f["path"]))).collect();
        let total: f64 = sel.iter().map(|f| util::num(&f["size"])).sum();
        let selectable = files.iter().filter(|f| !is_app_owned(f)).count();
        let all = if selectable > 0 { selectable } else { files.len() };
        let partial = !sel.is_empty() && sel.len() < all;
        self.all_box.set_inconsistent(partial);
        self.all_box.set_active(!sel.is_empty() && sel.len() >= all);
        let n = sel.len() as i64;
        self.rev_count.set_label(&if n > 0 { plural(n, tr("selectedOne"), tr("selectedOther")) } else { tr("selected0").to_owned() });
        self.rev_total.set_label(&fill(tr("total"), &[("size", &bytes(total))]));
        self.rev_total.set_visible(n > 0);
        kit::controls::clear(&self.rev_list);
        for f in sel.iter().take(50) {
            let li = layout::hbox(8, "taskmgr-lf-revitem");
            let nl = layout::label(&util::text(&f["name"]), "");
            nl.set_hexpand(true);
            li.append(&nl);
            li.append(&layout::label(&bytes(util::num(&f["size"])), "taskmgr-text-2 taskmgr-tabular"));
            self.rev_list.append(&li);
        }
        self.rev_scroll.set_visible(n > 0);
        self.trash_btn.set_sensitive(n > 0);
        self.clear_btn.set_visible(n > 0);
        self.syncing.set(false);
    }

    fn move_to_trash(&self) {
        let checked = self.checked.borrow().clone();
        let sel: Vec<Value> = self.files.borrow().iter().filter(|f| checked.contains(&util::text(&f["path"]))).cloned().collect();
        if sel.is_empty() {
            return;
        }
        let total: f64 = sel.iter().map(|f| util::num(&f["size"])).sum();
        let app_sel = sel.iter().any(is_app_owned);
        let content = layout::vbox(16, "taskmgr-dlg");
        if app_sel {
            let bar = InfoBar::new(Severity::Warning, tr("appWarnTitle"), tr("appWarnBody"));
            bar.set_closable(false);
            content.append(bar.widget());
        }
        content.append(&layout::wrap(&fill(tr("confirmBody"), &[("size", &bytes(total))]), ""));
        let list = layout::vbox(2, "taskmgr-dlg-list");
        for f in &sel {
            let li = layout::hbox(10, if is_app_owned(f) { "taskmgr-dlg-item taskmgr-is-app" } else { "taskmgr-dlg-item" });
            li.append(&kit::icons::file_icon(&mime_of(f), 16));
            let nl = layout::label(&util::text(&f["name"]), "");
            nl.set_hexpand(true);
            li.append(&nl);
            if is_app_owned(f) {
                let g = kit::icons::glyph("warning", 12);
                g.add_css_class("taskmgr-lf-app");
                g.set_tooltip_text(Some(tr("appOwned")));
                li.append(&g);
            }
            li.append(&layout::label(&bytes(util::num(&f["size"])), "taskmgr-text-2 taskmgr-tabular"));
            list.append(&li);
        }
        let scroll = layout::max_scroll(&list, 208);
        scroll.add_css_class("taskmgr-dlg-listwrap");
        content.append(&scroll);
        let tot = layout::hbox(8, "taskmgr-dlg-total");
        let n = sel.len() as i64;
        let tl = layout::label(&plural(n, tr("selectedOne"), tr("selectedOther")), "");
        tl.set_hexpand(true);
        tot.append(&tl);
        tot.append(&layout::label(&bytes(total), ""));
        content.append(&tot);
        let mut d = Dialog::new(&plural(n, tr("confirmOne"), tr("confirmOther"))).body(&content).primary(tr("move")).close_label(tr("cancel")).width(480);
        if app_sel {
            d = d.danger();
        }
        let w = self.me.clone();
        d.show(&self.app.parent(), move |r| {
            if r != DialogResult::Primary {
                return;
            }
            let Some(lf) = w.upgrade() else { return };
            let paths: Vec<String> = sel.iter().map(|f| util::text(&f["path"])).collect();
            let w = lf.me.clone();
            hostcall::call("files.trash", json!({ "paths": paths }), move |r| {
                let Some(lf) = w.upgrade() else { return };
                let res = match r {
                    Ok(res) => res,
                    Err(e) => {
                        lf.app.toast_error(tr("failedAll"), &e.message);
                        return;
                    }
                };
                // test mode (--dry-run): the host only pretended, so the files stay in the list
                if res["dryRun"] == true {
                    lf.app.toast(Toast::new(tr("dryRun")).severity(Severity::Info).timeout(4000));
                    return;
                }
                let failed: HashSet<String> =
                    res["failed"].as_array().into_iter().flatten().map(|x| if x.is_string() { util::text(x) } else { util::text(&x["path"]) }).collect();
                let moved: Vec<String> = paths.iter().filter(|p| !failed.contains(*p)).cloned().collect();
                if !moved.is_empty() {
                    lf.files.borrow_mut().retain(|f| !moved.contains(&util::text(&f["path"])));
                    lf.checked.borrow_mut().retain(|p| !moved.contains(p));
                    lf.render_rows();
                    let n = moved.len() as i64;
                    lf.app.toast(
                        Toast::new(&plural(n, tr("movedOne"), tr("movedOther")))
                            .message(tr("undoHint"))
                            .severity(Severity::Success)
                            .timeout(8000)
                            .action(tr("openTrash"), || {
                                hostcall::call("shell.openApp", json!({ "app": "explorer", "params": { "path": "trash:///" } }), |_| {});
                            }),
                    );
                    (lf.on_changed)();
                }
                if !failed.is_empty() {
                    lf.app.toast(Toast::new(&plural(failed.len() as i64, tr("failedOne"), tr("failedOther"))).severity(Severity::Warning));
                }
            });
        });
    }

    fn open_location(&self, path: &str) {
        let app = self.app.clone();
        hostcall::call("shell.openApp", json!({ "app": "explorer", "params": { "path": util::dir_name(path) } }), move |r| {
            if let Err(e) = r {
                app.toast_error(tr("openLocation"), &e.message);
            }
        });
    }

    /// The folder to list; another folder clears the selection.
    pub fn set_scope(&self, s: &Scope) {
        let same = self.scope.borrow().as_ref().is_some_and(|c| c.path == s.path);
        *self.scope.borrow_mut() = Some(s.clone());
        if !same {
            self.checked.borrow_mut().clear();
            self.files.borrow_mut().clear();
        }
        self.load();
    }
}
