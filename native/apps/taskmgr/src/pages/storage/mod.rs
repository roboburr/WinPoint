//! Opslag / Storage (js/apps/taskmgr/pages/storage.js, css/taskmgr.css "Opslag"): the volume cards with their big
//! usage bars (coloured by the biggest folders once a volume is scanned), then four views: Verkennen (the
//! sunburst explorer), Categorieën, Grote bestanden and Opruimen. Deep links pick a volume (`volume`: id or mount
//! point), a folder (`path`, `label`) and a view (`tab`). The page scrolls as a whole; Opruimen's footer stays at
//! the bottom of the view (position: sticky).
//!
//! `layout` holds the layout helpers Samenvatting and Prestaties use too.
use super::Page;

pub mod categories;
pub mod cleanup;
pub mod explorer;
pub mod largefiles;
pub mod layout;
pub mod sunburst;

use crate::app::App;
use crate::store::Subscription;
use crate::util::{self, Icon, fill};
use categories::Categories;
use cleanup::Cleanup;
use explorer::{Explorer, Scanner};
use kit::controls::Pivot;
use kit::format::bytes;
use kit::prelude::*;
use largefiles::LargeFiles;
use layout::{Bar, Layout, Seg};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Opslag", "Storage"),
        "refresh" => ("Vernieuwen", "Refresh"),
        "explore" => ("Verkennen", "Explore"),
        "categories" => ("Categorie\u{eb}n", "Categories"),
        "large" => ("Grote bestanden", "Large files"),
        "cleanup" => ("Opruimen", "Clean up"),
        "free" => ("{free} vrij van {total}", "{free} free of {total}"),
        "almostFull" => ("Bijna vol", "Almost full"),
        "full" => ("Vol", "Full"),
        "home" => ("Persoonlijke map", "Home folder"),
        "used" => ("{used} gebruikt", "{used} used"),
        "otherUsed" => ("Overig gebruikt", "Other used"),
        "loading" => ("Schijven laden\u{2026}", "Loading disks\u{2026}"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

const TABS: [&str; 4] = ["explore", "categories", "large", "cleanup"];

/// "ext4" -> "EXT4"; display-ready names ("NTFS", "exFAT") stay as they are.
fn fs_label(fs: &str) -> String {
    if !fs.is_empty() && fs.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) { fs.to_uppercase() } else { fs.to_owned() }
}

/// The folder a view works on: its path and the name to show for it.
#[derive(Clone, Debug)]
pub struct Scope {
    pub path: String,
    pub label: String,
}

struct Storage {
    me: Weak<Storage>,
    app: Rc<App>,
    overlay: gtk::Overlay,
    vol_row: Layout,
    pivot: Pivot,
    view: gtk::Box,
    vols: RefCell<Option<Vec<Value>>>,
    tab: RefCell<String>,
    scope: RefCell<Option<Scope>>,
    pending: RefCell<Option<Value>>,
    scanner: Rc<Scanner>,
    explorer: Rc<Explorer>,
    large: Rc<LargeFiles>,
    cleanup: Rc<Cleanup>,
    categories: Rc<Categories>,
    subs: RefCell<Vec<Subscription>>,
    commands: Vec<gtk::Widget>,
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let page = Rc::new_cyclic(|me: &Weak<Storage>| build(app, me.clone()));
    page.wire();
    page
}

fn build(app: &Rc<App>, me: Weak<Storage>) -> Storage {
    let scanner = Scanner::new(app);
    let w = me.clone();
    let explorer = Explorer::new(app, &scanner, move |path, name| {
        if let Some(p) = w.upgrade() {
            p.set_scope(Scope { path, label: name }, false);
            p.set_tab("large", true);
        }
    });
    let a = Rc::downgrade(app);
    let large = LargeFiles::new(app, move || {
        if let Some(a) = a.upgrade() {
            a.store.refresh("volumes");
        }
    });
    let a = Rc::downgrade(app);
    let cleanup = Cleanup::new(app, move || {
        if let Some(a) = a.upgrade() {
            a.store.refresh("volumes");
            a.store.refresh("health");
        }
    });
    let w = me.clone();
    let categories = Categories::new(app, move |path, name| {
        if let Some(p) = w.upgrade() {
            p.set_scope(Scope { path, label: name }, true);
            p.set_tab("explore", true);
        }
    });

    let saved = util::load("storage.tab").as_str().unwrap_or("explore").to_owned();
    let tab = if TABS.contains(&saved.as_str()) { saved } else { "explore".to_owned() };
    let vol_row = Layout::auto_fill(250, 8);
    vol_row.add_css_class("taskmgr-vols");
    vol_row.set_widget_name("taskmgr-volumes");
    let loading = layout::label(tr("loading"), "taskmgr-vols-loading w11-caption taskmgr-text-2");
    vol_row.append(&loading);
    let items: Vec<(&str, &str, &str)> = TABS.iter().map(|id| (*id, tr(id), "")).collect();
    let pivot = Pivot::new(&items, Some(&tab));
    pivot.widget().add_css_class("taskmgr-storage-tabs");
    let view = layout::vbox(0, "taskmgr-storage-view");
    view.set_size_request(-1, 400);
    let content = layout::vbox(0, "taskmgr-scroll taskmgr-storage");
    content.append(&vol_row);
    content.append(pivot.widget());
    content.append(&view);
    let clamp = Layout::clamp(1400);
    clamp.append(&content);
    let scroll = kit::controls::scrolled(&clamp);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&scroll));
    overlay.set_overflow(gtk::Overflow::Hidden);
    overlay.set_vexpand(true);
    overlay.add_overlay(cleanup.footer());
    cleanup.footer().set_visible(false);
    // position: sticky; bottom: 0 for Opruimen's footer: its place in the list, or the bottom of the view
    let cw = Rc::downgrade(&cleanup);
    overlay.connect_get_child_position(move |ov, child| {
        let c = cw.upgrade()?;
        if child != c.footer().upcast_ref::<gtk::Widget>() {
            return None;
        }
        let b = c.placeholder().compute_bounds(ov)?;
        let w = (b.width() as i32).max(1);
        let (_, fh, _, _) = child.measure(gtk::Orientation::Vertical, w);
        let y = (b.y() as i32).min(ov.height() - fh);
        Some(gdk::Rectangle::new(b.x() as i32, y, w, fh))
    });
    let ow = overlay.downgrade();
    let reflow = move |_: &gtk::Adjustment| {
        if let Some(o) = ow.upgrade() {
            o.queue_allocate();
        }
    };
    let adj = scroll.vadjustment();
    adj.connect_value_changed(reflow.clone());
    adj.connect_changed(reflow);

    let refresh = util::cmd_button(Icon::Glyph("refresh"), tr("refresh"));
    refresh.set_widget_name("taskmgr-refresh");
    let w = me.clone();
    refresh.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.refresh_all();
        }
    });
    Storage {
        me,
        app: app.clone(),
        overlay,
        vol_row,
        pivot,
        view,
        vols: RefCell::new(None),
        tab: RefCell::new(tab),
        scope: RefCell::new(None),
        pending: RefCell::new(None),
        scanner,
        explorer,
        large,
        cleanup,
        categories,
        subs: RefCell::new(Vec::new()),
        commands: vec![refresh.upcast()],
    }
}

impl Storage {
    fn wire(&self) {
        let w = self.me.clone();
        self.pivot.connect_selected(move |id| {
            if let Some(p) = w.upgrade() {
                p.set_tab(id, false);
            }
        });
        let w = self.me.clone();
        self.scanner.on_tree(move |path| {
            let Some(p) = w.upgrade() else { return };
            let is_vol = p.vols.borrow().as_ref().is_some_and(|vs| vs.iter().any(|v| util::text(&v["mount"]) == path));
            if is_vol {
                p.render_vols();
            }
        });
    }

    fn home_path(&self) -> String {
        if let Some(h) = self.categories.home() {
            return h;
        }
        let user = self.app.user();
        if user.is_empty() { "/home".into() } else { format!("/home/{user}") }
    }

    fn home_scope(&self) -> Scope {
        Scope { path: self.home_path(), label: tr("home").into() }
    }

    // ---------------------------------------------------------------------------------------- volumes

    /// The volume a path is on (the longest mount point that holds it).
    fn vol_of(&self, path: &str) -> Option<Value> {
        let vols = self.vols.borrow();
        let mut list: Vec<&Value> = vols.as_ref()?.iter().collect();
        list.sort_by_key(|v| std::cmp::Reverse(util::text(&v["mount"]).len()));
        list.into_iter()
            .find(|v| {
                let m = util::text(&v["mount"]);
                !m.is_empty() && (path == m || path.starts_with(&if m == "/" { "/".to_owned() } else { format!("{m}/") }))
            })
            .cloned()
    }

    fn render_vols(&self) {
        let Some(vols) = self.vols.borrow().clone() else { return };
        self.vol_row.clear();
        let path = self.scope.borrow().as_ref().map(|s| s.path.clone()).unwrap_or_else(|| self.home_path());
        let sel = self.vol_of(&path).map(|v| util::text(&v["mount"]));
        let full = |v: &Value| util::vol_pct(v) >= 90.0;
        // system disk first, then the home disk, then disks that are (almost) full, then the host's order
        let mut list: Vec<Value> = vols.iter().filter(|v| v["hidden"] != true).cloned().collect();
        list.sort_by(|a, b| {
            (b["isSystem"] == true).cmp(&(a["isSystem"] == true)).then((b["isHome"] == true).cmp(&(a["isHome"] == true))).then(full(b).cmp(&full(a)))
        });
        for v in &list {
            let mount = util::text(&v["mount"]);
            let card = self.vol_card(v, sel.as_deref() == Some(mount.as_str()));
            self.vol_row.append(&card);
        }
    }

    fn vol_card(&self, v: &Value, selected: bool) -> gtk::Button {
        let pct = util::vol_pct(v);
        let level = if pct >= 97.0 {
            "critical"
        } else if pct >= 90.0 {
            "warning"
        } else {
            "ok"
        };
        let mount = util::text(&v["mount"]);
        let name = [&v["name"], &v["label"], &v["mount"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default();
        let total = util::num(&v["total"]);
        let used = util::num(&v["used"]);
        let bar = Bar::new(10, "taskmgr-vol-bar");
        bar.set_gap(1.0);
        let tree = self.scanner.trees.borrow().get(&mount).cloned();
        match tree.filter(|t| !t.children.borrow().is_empty() && level == "ok" && total > 0.0) {
            Some(tree) => {
                let mut segs = Vec::new();
                let mut shown = 0.0;
                for (i, c) in tree.children.borrow().iter().filter(|c| !c.other && c.size > 0.0).take(5).enumerate() {
                    shown += c.size;
                    segs.push(Seg::new(c.size / total, move || sunburst::arc_color(i, 1, false), &format!("{}: {}", c.name, bytes(c.size))));
                }
                let rest = (used - shown).max(0.0);
                segs.push(Seg::named(rest / total, "tm-cat-other", &format!("{}: {}", tr("otherUsed"), bytes(rest))));
                bar.set(segs);
            }
            None => {
                let color = if level == "ok" { "accent" } else { "tm-danger-bar" };
                bar.set(vec![Seg::named(pct.min(100.0) / 100.0, color, &fill(tr("used"), &[("used", &bytes(used))]))]);
            }
        }
        let icon = kit::icons::app_icon(if v["removable"] == true { "drive-removable-media" } else { "drive-harddisk" }, 32);
        icon.set_valign(gtk::Align::Center);
        let names = layout::vbox(0, "");
        names.set_hexpand(true);
        names.set_valign(gtk::Align::Center);
        names.append(&layout::label(&name, "taskmgr-vol-name"));
        // automounted drives (/run/media/<user>/<label or UUID>) are known by their name already: the folder only
        // in the tooltip, so no raw UUIDs on screen
        let auto = ["/run/media/", "/media/", "/mnt/"].iter().any(|p| mount.starts_with(p));
        let parts: Vec<String> = [if auto { String::new() } else { mount.clone() }, fs_label(&util::text(&v["fsType"]))].into_iter().filter(|s| !s.is_empty()).collect();
        let ml = layout::label(&parts.join(" \u{b7} "), "taskmgr-vol-mount");
        ml.set_tooltip_text(Some(&mount));
        names.append(&ml);
        let pl = layout::label(&format!("{}%", pct.round() as i64), "taskmgr-vol-pct");
        pl.set_ellipsize(gtk::pango::EllipsizeMode::None);
        pl.set_valign(gtk::Align::Center);
        let top = layout::hbox(12, "");
        top.append(&icon);
        top.append(&names);
        top.append(&pl);
        let foot = layout::hbox(8, "taskmgr-vol-foot");
        let fl = layout::label(&fill(tr("free"), &[("free", &bytes(util::num(&v["free"]))), ("total", &bytes(total))]), "");
        fl.set_hexpand(true);
        foot.append(&fl);
        if level != "ok" {
            let pill = layout::label(
                tr(if level == "critical" { "full" } else { "almostFull" }),
                if level == "critical" { "taskmgr-pill taskmgr-is-critical" } else { "taskmgr-pill taskmgr-is-warning" },
            );
            pill.set_ellipsize(gtk::pango::EllipsizeMode::None);
            foot.append(&pill);
        }
        let body = layout::vbox(10, "");
        body.append(&top);
        body.append(bar.widget());
        body.append(&foot);
        let mut class = format!("taskmgr-vol taskmgr-is-{level}");
        if selected {
            class.push_str(" taskmgr-selected");
        }
        let card = layout::flat_button(&class, &body);
        card.set_widget_name(&format!("taskmgr-volume-{}", util::text(&v["id"])));
        let w = self.me.clone();
        card.connect_clicked(move |_| {
            let Some(p) = w.upgrade() else { return };
            p.set_scope(Scope { path: mount.clone(), label: name.clone() }, true);
            let tab = p.tab.borrow().clone();
            if tab != "explore" && tab != "large" {
                p.set_tab("explore", true);
            }
        });
        card
    }

    // ------------------------------------------------------------------------------------------ views

    fn set_scope(&self, s: Scope, scan: bool) {
        *self.scope.borrow_mut() = Some(s.clone());
        self.render_vols();
        let tab = self.tab.borrow().clone();
        if tab == "explore" && scan {
            self.explorer.set_scope(&s, false);
        } else if tab == "large" {
            self.large.set_scope(&s);
        }
    }

    fn set_tab(&self, id: &str, select: bool) {
        if !TABS.contains(&id) {
            return;
        }
        *self.tab.borrow_mut() = id.to_owned();
        util::save("storage.tab", json!(id));
        if select {
            self.pivot.select(id);
        }
        self.show_tab(true);
    }

    fn show_tab(&self, anim: bool) {
        kit::controls::clear(&self.view);
        if self.scope.borrow().is_none() {
            *self.scope.borrow_mut() = Some(self.home_scope());
        }
        let scope = self.scope.borrow().clone().unwrap_or_else(|| self.home_scope());
        let tab = self.tab.borrow().clone();
        let node = match tab.as_str() {
            "explore" => {
                self.explorer.set_scope(&scope, false);
                self.explorer.widget()
            }
            "categories" => {
                self.categories.show();
                self.categories.widget()
            }
            "large" => {
                self.large.set_scope(&scope);
                self.large.widget()
            }
            _ => {
                self.cleanup.show();
                self.cleanup.widget()
            }
        };
        self.view.append(&node);
        if anim {
            layout::fade_in(&node, 12, 250);
        }
        self.cleanup.footer().set_visible(tab == "cleanup");
        self.overlay.queue_allocate();
        self.render_vols();
    }

    fn apply_params(&self, p: &Value) {
        let volume = util::text(&p["volume"]);
        let path = util::text(&p["path"]);
        if !volume.is_empty() || !path.is_empty() {
            // health actions pass the volume id; deep links may pass a mount point
            let v = if volume.is_empty() {
                None
            } else {
                self.vols.borrow().as_ref().and_then(|vs| vs.iter().find(|x| util::text(&x["id"]) == volume || util::text(&x["mount"]) == volume).cloned())
            };
            let target = if !path.is_empty() {
                Some(path)
            } else if let Some(v) = &v {
                Some(util::text(&v["mount"]))
            } else if volume.starts_with('/') {
                Some(volume.clone())
            } else {
                None
            };
            if let Some(t) = target.filter(|t| !t.is_empty()) {
                let label = match &v {
                    Some(v) => [&v["name"], &v["mount"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default(),
                    None => {
                        let l = util::text(&p["label"]);
                        if l.is_empty() { util::pretty_path(&t) } else { l }
                    }
                };
                *self.scope.borrow_mut() = Some(Scope { path: t, label });
            }
        }
        let tab = util::text(&p["tab"]);
        if TABS.contains(&tab.as_str()) && *self.tab.borrow() != tab {
            *self.tab.borrow_mut() = tab.clone();
            util::save("storage.tab", json!(tab));
            self.pivot.select(&tab);
        }
        self.show_tab(false);
    }

    fn refresh_all(&self) {
        self.app.store.refresh("volumes");
        let tab = self.tab.borrow().clone();
        match tab.as_str() {
            "explore" => self.explorer.refresh(),
            "categories" => self.categories.load(true),
            "large" => self.large.load(),
            _ => self.cleanup.refresh(),
        }
    }
}

impl Page for Storage {
    fn id(&self) -> &'static str {
        "storage"
    }

    fn title(&self) -> String {
        tr("title").to_owned()
    }

    fn widget(&self) -> gtk::Widget {
        self.overlay.clone().upcast()
    }

    fn commands(&self) -> Vec<gtk::Widget> {
        self.commands.clone()
    }

    fn fill(&self) -> bool {
        true
    }

    fn mount(&self, params: &Value) {
        // with a volume, wait for the volume list so the card and the label are right
        let with_volume = !util::text(&params["volume"]).is_empty();
        if with_volume {
            *self.pending.borrow_mut() = Some(params.clone());
        }
        let w = self.me.clone();
        let sub = self.app.store.subscribe("volumes", move |d| {
            let Some(p) = w.upgrade() else { return };
            *p.vols.borrow_mut() = Some(d.as_array().cloned().unwrap_or_default());
            let pending = p.pending.borrow_mut().take();
            match pending {
                Some(params) => p.apply_params(&params),
                None => p.render_vols(),
            }
        });
        self.subs.borrow_mut().push(sub);
        if !with_volume {
            self.apply_params(params);
        }
    }

    fn navigate(&self, params: &Value) {
        self.apply_params(params);
    }

    fn unmount(&self) {
        self.subs.borrow_mut().clear();
        self.pending.borrow_mut().take();
    }

    fn refresh(&self) -> bool {
        self.refresh_all();
        true
    }
}
