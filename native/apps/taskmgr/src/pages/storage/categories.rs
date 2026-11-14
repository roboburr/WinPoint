//! Opslag > Categorieën (js/apps/taskmgr/storage/categories.js): what takes space in the home folder
//! (storage.categories with its events storage.categoriesProgress / storage.categoriesReady), as a donut and a
//! coloured legend (Documenten, Afbeeldingen, ... Chat-apps, Ontwikkeltools, Cache). Each row opens that folder
//! in Verkennen.
use super::layout::{self, Bar, Layout, Seg, VAlign};
use crate::app::{App, EventSub};
use crate::hostcall;
use crate::util::{self, fill};
use kit::chart::{Donut, Segment};
use kit::controls::{InfoBar, Severity};
use kit::format::{bytes, number};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Wat neemt ruimte in je persoonlijke map?", "What takes up space in your home folder?"),
        "computing" => ("Berekenen hoeveel ruimte elke categorie gebruikt\u{2026}", "Working out how much space each category uses\u{2026}"),
        "refresh" => ("Opnieuw berekenen", "Calculate again"),
        "total" => ("in totaal", "in total"),
        "failed" => ("Kon de categorie\u{eb}n niet berekenen", "Couldn\u{2019}t calculate the categories"),
        "hint" => ("Klik op een categorie om te zien wat erin zit.", "Click a category to see what\u{2019}s in it."),
        "progress" => ("{files} bestanden bekeken \u{b7} {size}", "{files} files checked \u{b7} {size}"),
        "updating" => ("Bijwerken\u{2026}", "Updating\u{2026}"),
        "updated" => ("Bijgewerkt {when}", "Updated {when}"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

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
        "appdata" => "apps",
        "snaps" | "flatpak" => "app",
        "cache" => "clean",
        "other" => "more",
        _ => "folder",
    }
}

pub struct Categories {
    me: Weak<Categories>,
    on_explore: Box<dyn Fn(String, String)>,
    data: RefCell<Option<Value>>,
    loading: Cell<bool>,
    root: gtk::Box,
    body: gtk::Box,
    status: gtk::Label,
    progress: RefCell<Option<gtk::Label>>,
    _subs: RefCell<Vec<EventSub>>,
}

impl Categories {
    pub fn new(app: &Rc<App>, on_explore: impl Fn(String, String) + 'static) -> Rc<Categories> {
        let refresh = layout::subtle("refresh", tr("refresh"));
        refresh.set_widget_name("taskmgr-categories-refresh");
        let status = layout::label("", "taskmgr-cat-status w11-caption");
        status.set_ellipsize(gtk::pango::EllipsizeMode::None);
        status.set_visible(false);
        let left = layout::vbox(0, "");
        left.set_hexpand(true);
        left.append(&layout::section_title(tr("title")));
        left.append(&layout::wrap(tr("hint"), "w11-caption taskmgr-text-2"));
        let right = layout::hbox(12, "");
        right.set_valign(gtk::Align::Start);
        right.append(&status);
        right.append(&refresh);
        let header = layout::hbox(16, "taskmgr-cat-header");
        header.append(&left);
        header.append(&right);
        let body = layout::vbox(0, "");
        let root = layout::vbox(12, "taskmgr-cat");
        root.append(&header);
        root.append(&body);
        let cats = Rc::new_cyclic(|me: &Weak<Categories>| Categories {
            me: me.clone(),
            on_explore: Box::new(on_explore),
            data: RefCell::new(None),
            loading: Cell::new(false),
            root,
            body,
            status,
            progress: RefCell::new(None),
            _subs: RefCell::new(Vec::new()),
        });
        let w = Rc::downgrade(&cats);
        refresh.connect_clicked(move |_| {
            if let Some(c) = w.upgrade() {
                c.load(true);
            }
        });
        let w = Rc::downgrade(&cats);
        let ready = app.on("storage.categoriesReady", move |_, d| {
            let Some(c) = w.upgrade() else { return };
            if d["data"].is_object() {
                *c.data.borrow_mut() = Some(d["data"].clone());
                c.loading.set(false);
                c.set_status("");
                c.render();
            }
        });
        let w = Rc::downgrade(&cats);
        let progress = app.on("storage.categoriesProgress", move |_, d| {
            let Some(c) = w.upgrade() else { return };
            let txt = fill(tr("progress"), &[("files", &number(util::num(&d["files"]), 0)), ("size", &bytes(util::num(&d["bytes"])))]);
            let shown = c.progress.borrow().clone().filter(|l| l.root().is_some());
            match shown {
                Some(l) => l.set_label(&txt),
                None if c.data.borrow().is_some() => c.set_status(&format!("{} {txt}", tr("updating"))),
                None => {}
            }
        });
        cats._subs.borrow_mut().extend([ready, progress]);
        // the category colours of Task Manager's own tokens follow the theme
        let w = Rc::downgrade(&cats);
        kit::theme::on_change(move || {
            if let Some(c) = w.upgrade()
                && c.data.borrow().is_some()
                && c.root.root().is_some()
            {
                c.render();
            }
        });
        cats
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    /// The home folder the host measured (None until the first answer).
    pub fn home(&self) -> Option<String> {
        self.data.borrow().as_ref().map(|d| util::text(&d["home"])).filter(|h| !h.is_empty())
    }

    fn set_status(&self, s: &str) {
        self.status.set_label(s);
        self.status.set_visible(!s.is_empty());
    }

    /// Shown: calculate once, else show what is known.
    pub fn show(&self) {
        let has = self.data.borrow().is_some();
        if !has && !self.loading.get() {
            self.load(false);
        } else if has {
            self.render();
        }
    }

    pub fn load(&self, refresh: bool) {
        self.loading.set(true);
        let progress = layout::label("", "w11-caption taskmgr-text-3");
        progress.set_halign(gtk::Align::Center);
        *self.progress.borrow_mut() = Some(progress.clone());
        if self.data.borrow().is_none() {
            let card = layout::vbox(10, "taskmgr-card taskmgr-scan");
            card.append(kit::controls::ProgressRing::new(40, None).widget());
            let l = layout::wrap(tr("computing"), "taskmgr-text-2");
            l.set_halign(gtk::Align::Center);
            l.set_justify(gtk::Justification::Center);
            card.append(&l);
            card.append(&progress);
            kit::controls::clear(&self.body);
            self.body.append(&card);
        } else {
            self.set_status(tr("updating"));
        }
        let w = self.me.clone();
        hostcall::call("storage.categories", json!({ "refresh": refresh }), move |r| {
            let Some(c) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    if r["data"].is_object() {
                        *c.data.borrow_mut() = Some(r["data"].clone());
                        c.render();
                    }
                    if r["ready"] == true {
                        c.loading.set(false);
                    }
                    let scanned = c.data.borrow().as_ref().and_then(|d| d["scannedAt"].as_f64());
                    if r["running"] == true {
                        c.set_status(tr("updating"));
                    } else if let Some(at) = scanned {
                        c.set_status(&fill(tr("updated"), &[("when", &kit::format::rel_time(at, kit::format::now_ms()))]));
                    } else {
                        c.set_status("");
                    }
                }
                Err(e) => {
                    c.loading.set(false);
                    let bar = InfoBar::new(Severity::Error, tr("failed"), &e.message);
                    bar.set_closable(false);
                    let w = c.me.clone();
                    bar.set_action(Some((
                        tr("refresh"),
                        Rc::new(move || {
                            if let Some(c) = w.upgrade() {
                                c.load(true);
                            }
                        }),
                    )));
                    kit::controls::clear(&c.body);
                    c.body.append(bar.widget());
                }
            }
        });
    }

    fn render(&self) {
        let Some(data) = self.data.borrow().clone() else { return };
        let mut cats: Vec<Value> = data["categories"].as_array().into_iter().flatten().filter(|c| util::num(&c["size"]) > 0.0).cloned().collect();
        cats.sort_by(|a, b| {
            (a["id"] == "other")
                .cmp(&(b["id"] == "other"))
                .then(util::num(&b["size"]).partial_cmp(&util::num(&a["size"])).unwrap_or(std::cmp::Ordering::Equal))
        });
        let sum: f64 = cats.iter().map(|c| util::num(&c["size"])).sum();
        let total = data["total"].as_f64().filter(|t| *t > 0.0).unwrap_or(sum);
        let donut = Donut::new(220, 26.0);
        donut.update(
            cats.iter()
                .map(|c| {
                    let id = util::text(&c["id"]);
                    let size = util::num(&c["size"]);
                    Segment::new(size, &util::category_color(&id), &format!("{}: {}", util::text(&c["name"]), bytes(size)))
                })
                .collect(),
            None,
            &bytes(total),
            tr("total"),
        );
        donut.widget().add_css_class("taskmgr-cat-donut");
        let home = layout::hbox(6, "taskmgr-cat-home");
        home.set_halign(gtk::Align::Center);
        home.append(&kit::icons::glyph("home", 14));
        let h = util::text(&data["home"]);
        home.append(&layout::label(&util::pretty_path(if h.is_empty() { "~" } else { &h }), ""));
        let donut_wrap = layout::vbox(12, "taskmgr-cat-donutwrap");
        donut_wrap.append(donut.widget());
        donut_wrap.append(&home);

        let legend = layout::vbox(2, "taskmgr-cat-legend");
        let max = cats.iter().map(|c| util::num(&c["size"])).fold(1.0, f64::max);
        for c in &cats {
            let id = util::text(&c["id"]);
            let size = util::num(&c["size"]);
            let name = util::text(&c["name"]);
            let path = util::text(&c["path"]);
            let pct = if total > 0.0 { size / total * 100.0 } else { 0.0 };
            let cid = id.clone();
            let color = move || layout::color(&util::category_color(&cid));
            let swatch = layout::swatch(28, 28, 4.0, Some((cat_icon(&id), 14.0)), color.clone());
            let top = layout::hbox(12, "");
            let nl = layout::label(&name, "taskmgr-cat-name");
            nl.set_hexpand(true);
            top.append(&nl);
            top.append(&layout::label(&bytes(size), "taskmgr-cat-size"));
            let bar = Bar::new(4, "taskmgr-cat-bar");
            bar.set(vec![Seg::new((size / max).max(0.01), color.clone(), "")]);
            let pl = layout::label(&format!("{}%", number(pct, if pct < 10.0 { 1 } else { 0 })), "taskmgr-cat-pct");
            pl.set_xalign(1.0);
            pl.set_size_request(42, -1);
            let barline = layout::hbox(10, "");
            barline.append(bar.widget());
            barline.append(&pl);
            let mainb = layout::vbox(5, "");
            mainb.set_hexpand(true);
            mainb.append(&top);
            mainb.append(&barline);
            let chev = kit::icons::glyph("chevron-right", 12);
            chev.add_css_class("taskmgr-glyph-dim");
            let body = layout::hbox(12, "");
            body.append(&swatch);
            body.append(&mainb);
            body.append(&chev);
            let row = layout::flat_button("taskmgr-cat-row", &body);
            row.set_widget_name(&format!("taskmgr-category-{id}"));
            row.set_tooltip_text(Some(&util::pretty_path(&path)));
            let w = self.me.clone();
            let (p2, n2) = (path.clone(), name.clone());
            row.connect_clicked(move |_| {
                if let Some(c) = w.upgrade() {
                    (c.on_explore)(p2.clone(), n2.clone());
                }
            });
            legend.append(&row);
            let mut items: Vec<Value> = c["items"].as_array().into_iter().flatten().filter(|x| util::num(&x["size"]) > 0.0).cloned().collect();
            if items.is_empty() {
                continue;
            }
            items.sort_by(|a, b| util::num(&b["size"]).partial_cmp(&util::num(&a["size"])).unwrap_or(std::cmp::Ordering::Equal));
            let subs = layout::vbox(0, "taskmgr-cat-subs");
            for it in items {
                let (ip, iname) = (util::text(&it["path"]), util::text(&it["name"]));
                let b = layout::hbox(12, "");
                let d = layout::dot(6, color.clone());
                d.set_opacity(0.7);
                b.append(&d);
                let l = layout::label(&iname, "");
                l.set_hexpand(true);
                b.append(&l);
                b.append(&layout::label(&bytes(util::num(&it["size"])), "taskmgr-cat-size"));
                let sub = layout::flat_button("taskmgr-cat-sub", &b);
                sub.set_tooltip_text(Some(&util::pretty_path(&ip)));
                let w = self.me.clone();
                sub.connect_clicked(move |_| {
                    if let Some(c) = w.upgrade() {
                        (c.on_explore)(ip.clone(), iname.clone());
                    }
                });
                subs.append(&sub);
            }
            legend.append(&subs);
        }
        // grid 280 px + the legend; one column below an 820 px page
        let grid = Layout::columns(24, 8, |w| if w > 722 { vec![280, w - 24 - 280] } else { vec![w] }).valign_children(VAlign::Start);
        grid.append(&donut_wrap);
        grid.append(&legend);
        let card = layout::vbox(0, "taskmgr-card taskmgr-cat-card");
        card.append(&grid);
        kit::controls::clear(&self.body);
        self.body.append(&card);
    }
}
