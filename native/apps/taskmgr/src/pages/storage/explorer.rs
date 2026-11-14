//! Opslag > Verkennen (js/apps/taskmgr/storage/explorer.js): storage.scan with live progress and Annuleren
//! (storage.scanProgress / storage.scanDone, storage.scanResult when the tree did not come with the event,
//! storage.scanCancel), the breadcrumb, the sunburst with hover and drill-down, and the side list of the biggest
//! items with its context menu. Folders deeper than the first scan are scanned when you open them and grafted in.
use super::Scope;
use super::layout::{self, Layout};
use super::sunburst::{Node, Sunburst, arc_color, files_text};
use crate::app::{App, EventSub, HealthSub};
use crate::hostcall::{self, State};
use crate::util::{self, fill};
use kit::controls::{Breadcrumb, InfoBar, MenuItem, Severity, Toast, TopAction, Variant};
use kit::format::{bytes, number};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "scanning" => ("Bezig met scannen\u{2026}", "Scanning\u{2026}"),
        "cancel" => ("Annuleren", "Cancel"),
        "stopped" => ("Scannen gestopt", "Scan stopped"),
        "stoppedBody" => ("Je hebt het scannen gestopt. Start opnieuw wanneer je wilt.", "You stopped the scan. Start again whenever you like."),
        "again" => ("Opnieuw scannen", "Scan again"),
        "pick" => ("Map kiezen\u{2026}", "Choose folder\u{2026}"),
        "openExplorer" => ("Openen in Verkenner", "Open in File Explorer"),
        "rescan" => ("Opnieuw scannen", "Scan again"),
        "failed" => ("Scannen is mislukt", "The scan failed"),
        "contents" => ("Grootste items in {name}", "Largest items in {name}"),
        "empty" => ("Deze map is leeg.", "This folder is empty."),
        "otherOne" => ("Overige ({n} kleiner item)", "Other ({n} smaller item)"),
        "otherMany" => ("Overige ({n} kleinere items)", "Other ({n} smaller items)"),
        "otherShort" => ("Overige", "Other"),
        "openLocation" => ("Bestandslocatie openen", "Open file location"),
        "largeHere" => ("Grote bestanden in deze map", "Large files in this folder"),
        "copyPath" => ("Pad kopi\u{eb}ren", "Copy path"),
        "open" => ("Openen", "Open"),
        "loadingFolder" => ("Map wordt gescand\u{2026}", "Scanning folder\u{2026}"),
        "hint" => (
            "Wijs een stuk aan voor details. Klik om een map te openen; klik in het midden om terug te gaan.",
            "Point at a segment for details. Click a folder to open it; click the centre to go back.",
        ),
        "pickTitle" => ("Kies een map om te scannen", "Choose a folder to scan"),
        "noPicker" => ("Kiezen is hier niet beschikbaar", "Choosing isn\u{2019}t available here"),
        "copied" => ("Pad gekopieerd", "Path copied"),
        "up" => ("Omhoog", "Up"),
        "hostDown" => ("Win11 Shell reageert niet. Scan opnieuw zodra het weer werkt.", "Win11 Shell is not responding. Scan again once it\u{2019}s back."),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

// ------------------------------------------------------------------------------------------------- scanner

/// A failed or cancelled scan: `code` "cancelled", "timeout", "failed", "scan_failed" or the host's.
#[derive(Clone, Debug)]
pub struct ScanError {
    pub code: String,
    pub message: String,
}

type Done = Box<dyn FnOnce(Result<Rc<Node>, ScanError>)>;

struct Job {
    path: String,
    job_id: RefCell<Value>,
    on_progress: Rc<dyn Fn(&Value)>,
    done: RefCell<Option<Done>>,
}

impl Job {
    fn settle(&self, r: Result<Rc<Node>, ScanError>) {
        let done = self.done.borrow_mut().take();
        if let Some(f) = done {
            f(r);
        }
    }
}

/// One scan at a time (explorer.js createScanner); buffers scanDone events that arrive before the job id is
/// known; keeps the trees by path (the volume cards colour their bars with them).
pub struct Scanner {
    me: Weak<Scanner>,
    pub trees: RefCell<HashMap<String, Rc<Node>>>,
    early: RefCell<HashMap<String, Value>>,
    active: RefCell<Option<Rc<Job>>>,
    listeners: RefCell<Vec<Rc<dyn Fn(&str)>>>,
    _subs: RefCell<Vec<EventSub>>,
    _health: RefCell<Option<HealthSub>>,
}

fn id_key(v: &Value) -> String {
    util::text(v)
}

impl Scanner {
    pub fn new(app: &Rc<App>) -> Rc<Scanner> {
        let sc = Rc::new_cyclic(|me: &Weak<Scanner>| Scanner {
            me: me.clone(),
            trees: RefCell::new(HashMap::new()),
            early: RefCell::new(HashMap::new()),
            active: RefCell::new(None),
            listeners: RefCell::new(Vec::new()),
            _subs: RefCell::new(Vec::new()),
            _health: RefCell::new(None),
        });
        let w = Rc::downgrade(&sc);
        let progress = app.on("storage.scanProgress", move |_, d| {
            let Some(sc) = w.upgrade() else { return };
            let job = sc.active.borrow().clone();
            if let Some(job) = job
                && !d["jobId"].is_null()
                && d["jobId"] == *job.job_id.borrow()
            {
                (job.on_progress)(d);
            }
        });
        let w = Rc::downgrade(&sc);
        let done = app.on("storage.scanDone", move |_, d| {
            let Some(sc) = w.upgrade() else { return };
            if !d.is_object() {
                return;
            }
            let job = sc.active.borrow().clone();
            match job {
                Some(job) if !d["jobId"].is_null() && d["jobId"] == *job.job_id.borrow() => sc.finish(&job, d.clone()),
                _ => {
                    let key = id_key(&d["jobId"]);
                    sc.early.borrow_mut().insert(key.clone(), d.clone());
                    let w = sc.me.clone();
                    glib::timeout_add_local_once(Duration::from_secs(30), move || {
                        if let Some(sc) = w.upgrade() {
                            sc.early.borrow_mut().remove(&key);
                        }
                    });
                }
            }
        });
        // the host stopped answering (or restarts): its scan job and the scanDone event will never come
        let w = Rc::downgrade(&sc);
        let health = app.on_health(move |st, _| {
            let Some(sc) = w.upgrade() else { return };
            if st == State::Ok {
                return;
            }
            let job = sc.active.borrow_mut().take();
            if let Some(job) = job {
                job.settle(Err(ScanError { code: "timeout".into(), message: tr("hostDown").into() }));
            }
        });
        sc._subs.borrow_mut().extend([progress, done]);
        *sc._health.borrow_mut() = Some(health);
        sc
    }

    fn is_active(&self, job: &Rc<Job>) -> bool {
        self.active.borrow().as_ref().is_some_and(|a| Rc::ptr_eq(a, job))
    }

    /// Run `f(path)` when a tree arrived.
    pub fn on_tree(&self, f: impl Fn(&str) + 'static) {
        self.listeners.borrow_mut().push(Rc::new(f));
    }

    fn finish(&self, job: &Rc<Job>, d: Value) {
        if self.is_active(job) {
            self.active.borrow_mut().take();
        }
        let error = util::text(&d["error"]);
        let code = util::text(&d["errorCode"]);
        if !error.is_empty() || !code.is_empty() {
            let code = if !code.is_empty() {
                code
            } else if error == "cancelled" {
                "cancelled".into()
            } else {
                "scan_failed".into()
            };
            let message = if code == "cancelled" {
                "cancelled".to_owned()
            } else if !error.is_empty() {
                error
            } else {
                util::text(&d["message"])
            };
            job.settle(Err(ScanError { code, message }));
            return;
        }
        if !d["tree"].is_object() && d["fetched"] != true {
            // the event may come without the (large) tree: fetch it with storage.scanResult {jobId}
            let job_id = if d["jobId"].is_null() { job.job_id.borrow().clone() } else { d["jobId"].clone() };
            *self.active.borrow_mut() = Some(job.clone());
            let w = self.me.clone();
            let job = job.clone();
            hostcall::call("storage.scanResult", json!({ "jobId": job_id }), move |r| {
                let Some(sc) = w.upgrade() else { return };
                if !sc.is_active(&job) {
                    return; // cancelled or replaced meanwhile
                }
                match r {
                    Ok(r) if r["state"] == "running" => {
                        let w = sc.me.clone();
                        glib::timeout_add_local_once(Duration::from_millis(400), move || {
                            if let Some(sc) = w.upgrade()
                                && sc.is_active(&job)
                            {
                                sc.finish(&job, json!({ "jobId": job_id }));
                            }
                        });
                    }
                    Ok(r) if r["state"] == "error" || !r["tree"].is_object() => {
                        let code = util::text(&r["errorCode"]);
                        sc.finish(
                            &job,
                            json!({ "jobId": job_id, "fetched": true, "error": util::text(&r["error"]), "errorCode": if code.is_empty() { "failed".into() } else { code } }),
                        );
                    }
                    Ok(r) => sc.finish(&job, json!({ "jobId": job_id, "fetched": true, "tree": r["tree"] })),
                    Err(e) => {
                        let code = if e.code.is_empty() { "failed".to_owned() } else { e.code.clone() };
                        sc.finish(&job, json!({ "jobId": job_id, "fetched": true, "error": e.message, "errorCode": code }));
                    }
                }
            });
            return;
        }
        if d["tree"].is_object() {
            let tree = Node::from_value(&d["tree"]);
            self.trees.borrow_mut().insert(job.path.clone(), tree.clone());
            let ls: Vec<Rc<dyn Fn(&str)>> = self.listeners.borrow().clone();
            for f in ls {
                f(&job.path);
            }
            job.settle(Ok(tree));
        } else {
            job.settle(Err(ScanError { code: "failed".into(), message: String::new() }));
        }
    }

    /// Scan `path` (a running scan is cancelled first); `on_progress(event)` while it runs, `done` once.
    pub fn scan(&self, path: &str, refresh: bool, on_progress: impl Fn(&Value) + 'static, done: impl FnOnce(Result<Rc<Node>, ScanError>) + 'static) {
        if self.active.borrow().is_some() {
            self.cancel();
        }
        let job = Rc::new(Job { path: path.to_owned(), job_id: RefCell::new(Value::Null), on_progress: Rc::new(on_progress), done: RefCell::new(Some(Box::new(done))) });
        *self.active.borrow_mut() = Some(job.clone());
        let w = self.me.clone();
        hostcall::call("storage.scan", json!({ "path": path, "maxDepth": 4, "topN": 40, "refresh": refresh }), move |r| {
            let Some(sc) = w.upgrade() else { return };
            match r {
                Ok(r) => {
                    *job.job_id.borrow_mut() = r["jobId"].clone();
                    if r["tree"].is_object() {
                        sc.finish(&job, json!({ "jobId": r["jobId"], "tree": r["tree"] }));
                        return;
                    }
                    let early = sc.early.borrow_mut().remove(&id_key(&r["jobId"]));
                    if let Some(d) = early
                        && !r["jobId"].is_null()
                    {
                        sc.finish(&job, d);
                    }
                }
                Err(e) => {
                    if sc.is_active(&job) {
                        sc.active.borrow_mut().take();
                    }
                    job.settle(Err(ScanError { code: e.code.clone(), message: e.message.clone() }));
                }
            }
        });
    }

    /// Stop the running scan (storage.scanCancel); its `done` gets "cancelled".
    pub fn cancel(&self) {
        let job = self.active.borrow_mut().take();
        let Some(job) = job else { return };
        let id = job.job_id.borrow().clone();
        if !id.is_null() {
            hostcall::call("storage.scanCancel", json!({ "jobId": id }), |_| {});
        }
        job.settle(Err(ScanError { code: "cancelled".into(), message: "cancelled".into() }));
    }
}

// --------------------------------------------------------------------------------------------- explorer

/// A MIME type for a file name (explorer.js guessMime), for its icon.
pub fn guess_mime(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "iso" => "application/x-cd-image",
        "mkv" => "video/x-matroska",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "cr3" => "image/x-canon-cr3",
        "zip" => "application/zip",
        "pdf" => "application/pdf",
        "deb" => "application/vnd.debian.binary-package",
        "flac" => "audio/flac",
        "mp3" => "audio/mpeg",
        "qcow2" => "application/x-qemu-disk",
        "txt" => "text/plain",
        "odt" => "application/vnd.oasis.opendocument.text",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "js" => "text/javascript",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

struct Row {
    button: gtk::Button,
    node: Rc<Node>,
}

pub struct Explorer {
    me: Weak<Explorer>,
    app: Rc<App>,
    scanner: Rc<Scanner>,
    on_large: Box<dyn Fn(String, String)>,
    scope: RefCell<Option<Scope>>,
    root_tree: RefCell<Option<Rc<Node>>>,
    trail: RefCell<Vec<Rc<Node>>>,
    seq: Cell<u64>,
    root: Layout,
    crumbs: Breadcrumb,
    up_btn: gtk::Button,
    actions: Vec<gtk::Button>,
    sunburst: Sunburst,
    list_title: gtk::Label,
    list: gtk::Box,
    rows: RefCell<Vec<Row>>,
    main: Layout,
    status: gtk::Box,
}

impl Explorer {
    pub fn new(app: &Rc<App>, scanner: &Rc<Scanner>, on_large: impl Fn(String, String) + 'static) -> Rc<Explorer> {
        let ex = Rc::new_cyclic(|me: &Weak<Explorer>| {
            let crumbs = Breadcrumb::new(false);
            crumbs.widget().set_hexpand(true);
            let up_btn = kit::controls::icon_button("arrow-up", tr("up"), 32);
            up_btn.set_widget_name("taskmgr-explore-up");
            up_btn.set_sensitive(false);
            let pick = layout::subtle("folder-open", tr("pick"));
            pick.set_widget_name("taskmgr-explore-pick");
            let rescan = layout::subtle("refresh", tr("rescan"));
            rescan.set_widget_name("taskmgr-explore-rescan");
            let open = layout::subtle("external", tr("openExplorer"));
            open.set_widget_name("taskmgr-explore-open");
            let acts = layout::hbox(2, "taskmgr-explore-actions");
            for b in [&pick, &rescan, &open] {
                acts.append(b);
            }
            let bar = layout::hbox(8, "taskmgr-explore-bar");
            bar.append(&up_btn);
            bar.append(crumbs.widget());
            bar.append(&acts);

            let sunburst = Sunburst::new();
            sunburst.set_widget_name("taskmgr-sunburst");
            let hint = layout::wrap(tr("hint"), "taskmgr-explore-hint w11-caption");
            hint.set_justify(gtk::Justification::Center);
            hint.set_xalign(0.5);
            hint.set_max_width_chars(60);
            hint.set_halign(gtk::Align::Center);
            let chart = layout::vbox(10, "taskmgr-card taskmgr-explore-chart");
            chart.append(&sunburst);
            chart.append(&hint);
            let list_title = layout::label("", "taskmgr-explore-listtitle");
            let list = layout::vbox(0, "taskmgr-explore-list");
            list.set_widget_name("taskmgr-explore-list");
            let side = layout::vbox(0, "taskmgr-card taskmgr-explore-side");
            side.append(&list_title);
            side.append(&layout::max_scroll(&list, 510));
            // two columns (minmax(340px, 1fr) minmax(320px, 420px)), one below an 860 px page
            let main = Layout::columns(12, 12, |w| {
                if w > 804 {
                    let right = (w - 12 - 340).clamp(320, 420);
                    vec![w - 12 - right, right]
                } else {
                    vec![w]
                }
            })
            .valign_children(layout::VAlign::Start);
            main.append(&chart);
            main.append(&side);
            let status = layout::vbox(0, "taskmgr-explore-status");
            status.set_visible(false);
            let root = Layout::columns(0, 12, |w| vec![w]);
            root.add_css_class("taskmgr-explore");
            root.append(&bar);
            root.append(&status);
            root.append(&main);
            Explorer {
                me: me.clone(),
                app: app.clone(),
                scanner: scanner.clone(),
                on_large: Box::new(on_large),
                scope: RefCell::new(None),
                root_tree: RefCell::new(None),
                trail: RefCell::new(Vec::new()),
                seq: Cell::new(0),
                root,
                crumbs,
                up_btn,
                actions: vec![pick, rescan, open],
                sunburst,
                list_title,
                list,
                rows: RefCell::new(Vec::new()),
                main,
                status,
            }
        });
        ex.wire();
        ex
    }

    fn wire(&self) {
        let w = self.me.clone();
        self.crumbs.connect_selected(move |_, i| {
            if let Some(e) = w.upgrade() {
                e.go_to(i as isize);
            }
        });
        let w = self.me.clone();
        self.up_btn.connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.go_up();
            }
        });
        let w = self.me.clone();
        self.actions[0].connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.pick();
            }
        });
        let w = self.me.clone();
        self.actions[1].connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                e.refresh();
            }
        });
        let w = self.me.clone();
        self.actions[2].connect_clicked(move |_| {
            if let Some(e) = w.upgrade() {
                let p = e.current().map(|n| n.path.clone()).unwrap_or_default();
                e.open_in_explorer(&p);
            }
        });
        let w = self.me.clone();
        self.sunburst.connect_drill(move |n| {
            if let Some(e) = w.upgrade() {
                e.drill(n);
            }
        });
        let w = self.me.clone();
        self.sunburst.connect_up(move || {
            if let Some(e) = w.upgrade() {
                e.go_up();
            }
        });
        let w = self.me.clone();
        self.sunburst.connect_hover(move |n| {
            if let Some(e) = w.upgrade() {
                e.mark_hover(n);
            }
        });
        let w = self.me.clone();
        self.sunburst.connect_pick(move |n| {
            if let Some(e) = w.upgrade() {
                e.select_file(n);
            }
        });
        // the action labels hide below a 980 px page
        let acts: Vec<glib::WeakRef<gtk::Button>> = self.actions.iter().map(|b| b.downgrade()).collect();
        self.root.connect_width(move |w| {
            for b in acts.iter().filter_map(|b| b.upgrade()) {
                if let Some(l) = util::first_label(b.upcast_ref()) {
                    l.set_visible(w > 924);
                }
                let text = util::first_label(b.upcast_ref()).map(|l| l.label().to_string()).unwrap_or_default();
                b.set_tooltip_text(if w > 924 || text.is_empty() { None } else { Some(&text) });
            }
        });
        // Alt+Up: one level up while the explorer is on screen
        let w = self.me.clone();
        self.app.window.add_shortcut("<Alt>Up", move || {
            let Some(e) = w.upgrade() else { return false };
            if !e.root.is_mapped() {
                return false;
            }
            e.go_up();
            true
        });
    }

    pub fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn current(&self) -> Option<Rc<Node>> {
        self.trail.borrow().last().cloned()
    }

    fn go_up(&self) {
        let n = self.trail.borrow().len() as isize;
        self.go_to(n - 2);
    }

    // --------------------------------------------------------------------------------------- progress

    fn show_progress(&self, path: &str) -> Rc<dyn Fn(&Value)> {
        self.main.set_visible(false);
        if self.trail.borrow().is_empty() {
            let label = self.scope.borrow().as_ref().map(|s| s.label.clone()).filter(|l| !l.is_empty()).unwrap_or_else(|| util::pretty_path(path));
            self.crumbs.set_items(&[("0", label.as_str())]);
            self.up_btn.set_sensitive(false);
        }
        let card = layout::vbox(10, "taskmgr-card taskmgr-scan");
        card.append(kit::controls::ProgressRing::new(56, None).widget());
        let title = layout::label(tr("scanning"), "w11-subtitle taskmgr-scan-title");
        title.set_halign(gtk::Align::Center);
        let amount = layout::label(" ", "taskmgr-scan-amount");
        amount.set_halign(gtk::Align::Center);
        let cur = layout::label(&util::pretty_path(path), "taskmgr-scan-path");
        cur.set_ellipsize(gtk::pango::EllipsizeMode::Start);
        cur.set_halign(gtk::Align::Center);
        cur.set_max_width_chars(80);
        let cancel = kit::controls::button(tr("cancel"), Variant::Standard);
        cancel.set_widget_name("taskmgr-scan-cancel");
        cancel.set_halign(gtk::Align::Center);
        let sc = Rc::downgrade(&self.scanner);
        cancel.connect_clicked(move |_| {
            if let Some(sc) = sc.upgrade() {
                sc.cancel();
            }
        });
        card.append(&title);
        card.append(&amount);
        card.append(&cur);
        card.append(&cancel);
        kit::controls::clear(&self.status);
        self.status.append(&card);
        self.status.set_visible(true);
        let (aw, cw) = (amount.downgrade(), cur.downgrade());
        Rc::new(move |d: &Value| {
            if let Some(a) = aw.upgrade() {
                a.set_label(&fill("{size} \u{b7} {files}", &[("size", &bytes(util::num(&d["bytes"]))), ("files", &files_text(util::num(&d["files"])))]));
            }
            let current = util::text(&d["current"]);
            if !current.is_empty()
                && let Some(c) = cw.upgrade()
            {
                c.set_label(&util::pretty_path(&current));
            }
        })
    }

    fn show_message(&self, w: &impl IsA<gtk::Widget>) {
        self.main.set_visible(false);
        kit::controls::clear(&self.status);
        self.status.append(w.as_ref());
        self.status.set_visible(true);
    }

    fn run_scan(&self, path: &str, refresh: bool) {
        let my = self.seq.get() + 1;
        self.seq.set(my);
        let cached = if refresh { None } else { self.scanner.trees.borrow().get(path).cloned() };
        if let Some(t) = cached {
            self.set_tree(&t);
            return;
        }
        let progress = self.show_progress(path);
        let w = self.me.clone();
        let p = path.to_owned();
        self.scanner.scan(
            path,
            refresh,
            move |d| progress(d),
            move |r| {
                let Some(e) = w.upgrade() else { return };
                if e.seq.get() != my {
                    return;
                }
                match r {
                    Ok(tree) => e.set_tree(&tree),
                    Err(err) if err.code == "cancelled" => {
                        let again = kit::controls::button(tr("again"), Variant::Standard);
                        let w = e.me.clone();
                        let p2 = p.clone();
                        again.connect_clicked(move |_| {
                            if let Some(e) = w.upgrade() {
                                e.run_scan(&p2, true);
                            }
                        });
                        let card = layout::vbox(0, "taskmgr-card taskmgr-scan");
                        card.append(&kit::controls::empty_state("info", tr("stopped"), tr("stoppedBody"), Some(again.upcast_ref())));
                        e.show_message(&card);
                    }
                    Err(err) => {
                        let bar = InfoBar::new(Severity::Error, tr("failed"), &err.message);
                        bar.set_closable(false);
                        let w = e.me.clone();
                        let p2 = p.clone();
                        bar.set_action(Some((
                            tr("again"),
                            Rc::new(move || {
                                if let Some(e) = w.upgrade() {
                                    e.run_scan(&p2, true);
                                }
                            }),
                        )));
                        e.show_message(bar.widget());
                    }
                }
            },
        );
    }

    fn set_tree(&self, tree: &Rc<Node>) {
        *self.root_tree.borrow_mut() = Some(tree.clone());
        *self.trail.borrow_mut() = vec![tree.clone()];
        self.status.set_visible(false);
        kit::controls::clear(&self.status);
        self.main.set_visible(true);
        self.render(None);
    }

    // ----------------------------------------------------------------------------------------- render

    fn scope_label(&self) -> String {
        self.scope.borrow().as_ref().map(|s| s.label.clone()).unwrap_or_default()
    }

    fn render(&self, direction: Option<&str>) {
        let Some(node) = self.current() else { return };
        let trail = self.trail.borrow().clone();
        let label = self.scope_label();
        let items: Vec<(String, String)> = trail
            .iter()
            .enumerate()
            .map(|(i, n)| (i.to_string(), if i == 0 && !label.is_empty() { label.clone() } else { n.name.clone() }))
            .collect();
        let refs: Vec<(&str, &str)> = items.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        self.crumbs.set_items(&refs);
        self.up_btn.set_sensitive(trail.len() >= 2);
        self.sunburst.set_root(&node, direction, trail.len() > 1, if trail.len() == 1 { &label } else { "" });
        let name = if trail.len() == 1 && !label.is_empty() { label } else { node.name.clone() };
        self.list_title.set_label(&fill(tr("contents"), &[("name", &name)]));
        self.render_list(&node);
    }

    fn render_list(&self, node: &Rc<Node>) {
        kit::controls::clear(&self.list);
        self.rows.borrow_mut().clear();
        let kids = node.kids();
        if kids.is_empty() {
            self.list.append(&layout::empty_line(tr("empty")));
            return;
        }
        let max = kids.iter().map(|k| k.size).fold(0.0, f64::max).max(1.0);
        for c in kids {
            let pct = if node.size > 0.0 { c.size / node.size * 100.0 } else { 0.0 };
            let arc = self.sunburst.arc_of(&c);
            let color = move || match arc {
                Some((top, depth, other)) => arc_color(top, depth, other),
                None => layout::color("tm-cat-other"),
            };
            let icon: gtk::Widget = if c.other {
                let g = kit::icons::glyph("more", 16);
                g.add_css_class("taskmgr-glyph-dim");
                g.upcast()
            } else if c.is_dir {
                kit::icons::app_icon("folder", 20).upcast()
            } else {
                kit::icons::file_icon(guess_mime(&c.name), 20).upcast()
            };
            let icon_box = layout::hbox(0, "taskmgr-xrow-icon");
            icon_box.set_size_request(20, -1);
            icon_box.set_hexpand(false);
            icon_box.set_valign(gtk::Align::Center);
            icon.set_hexpand(true);
            icon_box.append(&icon);
            let name = if c.other {
                if c.count > 0.0 {
                    fill(tr(if c.count.round() as i64 == 1 { "otherOne" } else { "otherMany" }), &[("n", &number(c.count, 0))])
                } else {
                    tr("otherShort").to_owned()
                }
            } else {
                c.name.clone()
            };
            let top = layout::hbox(12, "");
            let nl = layout::label(&name, "taskmgr-xrow-name");
            nl.set_hexpand(true);
            top.append(&nl);
            top.append(&layout::label(&bytes(c.size), "taskmgr-xrow-size"));
            let bar = layout::Bar::new(4, "taskmgr-xrow-bar");
            bar.set(vec![layout::Seg::new((c.size / max).max(0.015), color, "")]);
            let pl = layout::label(&format!("{}%", number(pct, if pct < 10.0 { 1 } else { 0 })), "taskmgr-xrow-pct");
            pl.set_xalign(1.0);
            pl.set_size_request(42, -1);
            let barline = layout::hbox(10, "");
            barline.append(bar.widget());
            barline.append(&pl);
            let mainb = layout::vbox(5, "");
            mainb.set_hexpand(true);
            mainb.set_valign(gtk::Align::Center);
            mainb.append(&top);
            mainb.append(&barline);
            let body = layout::hbox(10, "");
            body.append(&layout::swatch(4, 28, 2.0, None, color));
            body.append(&icon_box);
            body.append(&mainb);
            if c.is_dir && !c.other {
                let chev = kit::icons::glyph("chevron-right", 12);
                chev.add_css_class("taskmgr-xrow-chev");
                body.append(&chev);
            }
            let class = if c.other {
                "taskmgr-xrow taskmgr-is-other"
            } else if c.is_dir {
                "taskmgr-xrow taskmgr-is-dir"
            } else {
                "taskmgr-xrow"
            };
            let row = layout::flat_button(class, &body);
            row.set_widget_name(&format!("taskmgr-explore-row-{}", self.rows.borrow().len()));
            if c.other {
                row.set_focusable(false);
            } else {
                row.set_tooltip_text(Some(&util::pretty_path(&c.path)));
            }
            self.wire_row(&row, &c);
            self.list.append(&row);
            self.rows.borrow_mut().push(Row { button: row, node: c });
        }
    }

    fn wire_row(&self, row: &gtk::Button, c: &Rc<Node>) {
        let motion = gtk::EventControllerMotion::new();
        let (w, n) = (self.me.clone(), c.clone());
        motion.connect_enter(move |_, _, _| {
            if let Some(e) = w.upgrade() {
                e.sunburst.highlight(Some(&n));
            }
        });
        let w = self.me.clone();
        motion.connect_leave(move |_| {
            if let Some(e) = w.upgrade() {
                e.sunburst.highlight(None);
            }
        });
        row.add_controller(motion);
        let focus = gtk::EventControllerFocus::new();
        let (w, n) = (self.me.clone(), c.clone());
        focus.connect_enter(move |_| {
            if let Some(e) = w.upgrade() {
                e.sunburst.highlight(Some(&n));
            }
        });
        let w = self.me.clone();
        focus.connect_leave(move |_| {
            if let Some(e) = w.upgrade() {
                e.sunburst.highlight(None);
            }
        });
        row.add_controller(focus);
        let (w, n) = (self.me.clone(), c.clone());
        row.connect_clicked(move |_| {
            let Some(e) = w.upgrade() else { return };
            if n.other {
                return;
            }
            if n.is_dir {
                e.drill(&n);
            } else {
                e.select_file(&n);
            }
        });
        let keys = gtk::EventControllerKey::new();
        let w = self.me.clone();
        let rw = row.downgrade();
        keys.connect_key_pressed(move |_, key, _, state| {
            let (Some(e), Some(row)) = (w.upgrade(), rw.upgrade()) else { return glib::Propagation::Proceed };
            let alt = state.contains(gdk::ModifierType::ALT_MASK);
            match key {
                gdk::Key::Down | gdk::Key::KP_Down if !alt => {
                    if let Some(n) = row.next_sibling() {
                        n.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Up | gdk::Key::KP_Up if !alt => {
                    if let Some(p) = row.prev_sibling() {
                        p.grab_focus();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::BackSpace => {
                    e.go_up();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        row.add_controller(keys);
        let (w, n) = (self.me.clone(), c.clone());
        kit::controls::bind_context_menu(row, move |_, _| {
            let e = w.upgrade()?;
            if n.other {
                return None;
            }
            Some((Vec::<TopAction>::new(), e.row_menu(&n)))
        });
    }

    fn row_menu(&self, n: &Rc<Node>) -> Vec<MenuItem> {
        let mut items = Vec::new();
        if n.is_dir {
            let (w, node) = (self.me.clone(), n.clone());
            items.push(
                MenuItem::new(tr("open"), move || {
                    if let Some(e) = w.upgrade() {
                        e.drill(&node);
                    }
                })
                .glyph("open")
                .name("taskmgr-menu-explore-open"),
            );
            let (w, path) = (self.me.clone(), n.path.clone());
            items.push(
                MenuItem::new(tr("openExplorer"), move || {
                    if let Some(e) = w.upgrade() {
                        e.open_in_explorer(&path);
                    }
                })
                .glyph("external")
                .name("taskmgr-menu-explore-explorer"),
            );
            let (w, path, name) = (self.me.clone(), n.path.clone(), n.name.clone());
            items.push(
                MenuItem::new(tr("largeHere"), move || {
                    if let Some(e) = w.upgrade() {
                        (e.on_large)(path.clone(), name.clone());
                    }
                })
                .glyph("filter")
                .name("taskmgr-menu-explore-large"),
            );
        } else {
            let (w, path) = (self.me.clone(), util::dir_name(&n.path));
            items.push(
                MenuItem::new(tr("openLocation"), move || {
                    if let Some(e) = w.upgrade() {
                        e.open_in_explorer(&path);
                    }
                })
                .glyph("folder-open")
                .name("taskmgr-menu-explore-location"),
            );
        }
        items.push(MenuItem::separator());
        let (w, path) = (self.me.clone(), n.path.clone());
        items.push(
            MenuItem::new(tr("copyPath"), move || {
                if let Some(e) = w.upgrade() {
                    layout::copy_text(&e.root, &path);
                    e.app.toast_success(tr("copied"), 2000);
                }
            })
            .glyph("copy")
            .name("taskmgr-menu-explore-copy"),
        );
        items
    }

    fn mark_hover(&self, node: Option<&Rc<Node>>) {
        for r in self.rows.borrow().iter() {
            util::set_class(&r.button, "taskmgr-hot", node.is_some_and(|n| Rc::ptr_eq(n, &r.node)));
        }
    }

    fn select_file(&self, node: &Rc<Node>) {
        for r in self.rows.borrow().iter() {
            let on = Rc::ptr_eq(node, &r.node);
            util::set_class(&r.button, "taskmgr-picked", on);
            if on {
                r.button.grab_focus();
            }
        }
    }

    /// The nodes from (not including) `from` down to `target`.
    fn chain(from: &Rc<Node>, target: &Rc<Node>) -> Option<Vec<Rc<Node>>> {
        for c in from.children.borrow().iter() {
            if Rc::ptr_eq(c, target) {
                return Some(vec![c.clone()]);
            }
            if let Some(mut sub) = Self::chain(c, target) {
                sub.insert(0, c.clone());
                return Some(sub);
            }
        }
        None
    }

    fn drill(&self, node: &Rc<Node>) {
        if !node.is_dir || node.other {
            return;
        }
        let Some(cur) = self.current() else { return };
        let path = Self::chain(&cur, node).unwrap_or_else(|| vec![node.clone()]);
        let has_kids = !node.children.borrow().is_empty();
        if (!has_kids || node.truncated.get()) && (node.files.get().is_some_and(|f| f > 0.0) || node.size > 0.0 || node.truncated.get()) {
            // deeper than the first scan: scan this folder and graft the result
            let my = self.seq.get() + 1;
            self.seq.set(my);
            self.list_title.set_label(tr("loadingFolder"));
            kit::controls::clear(&self.list);
            self.rows.borrow_mut().clear();
            self.list.append(&layout::ring_note(28, ""));
            let graft = {
                let w = self.me.clone();
                let node = node.clone();
                move |r: Result<Rc<Node>, ScanError>| {
                    let Some(e) = w.upgrade() else { return };
                    if e.seq.get() != my {
                        return;
                    }
                    match r {
                        Ok(sub) => {
                            let kids = sub.children.borrow().clone();
                            *node.children.borrow_mut() = kids;
                            node.truncated.set(false);
                            if let Some(f) = sub.files.get() {
                                node.files.set(Some(f));
                            }
                            e.trail.borrow_mut().extend(path);
                            e.render(Some("in"));
                        }
                        Err(err) => {
                            if err.code != "cancelled" {
                                e.app.toast(Toast::new(tr("failed")).message(&err.message).severity(Severity::Error));
                            }
                            e.render(None);
                        }
                    }
                }
            };
            let cached = self.scanner.trees.borrow().get(&node.path).cloned();
            match cached {
                Some(sub) => graft(Ok(sub)),
                None => self.scanner.scan(&node.path, false, |_| {}, graft),
            }
            return;
        }
        self.trail.borrow_mut().extend(path);
        self.render(Some("in"));
    }

    fn go_to(&self, i: isize) {
        let len = self.trail.borrow().len() as isize;
        if i < 0 || i >= len - 1 {
            return;
        }
        self.trail.borrow_mut().truncate(i as usize + 1);
        self.render(Some("out"));
    }

    fn open_in_explorer(&self, path: &str) {
        if path.is_empty() {
            return;
        }
        let app = self.app.clone();
        hostcall::call("shell.openApp", json!({ "app": "explorer", "params": { "path": path } }), move |r| {
            if let Err(e) = r {
                app.toast_error(tr("openExplorer"), &e.message);
            }
        });
    }

    /// "Map kiezen": GTK's folder dialog (the desktop's FileChooser portal: Win11's picker when it is chosen). The
    /// WebKit page's shell.pickFile has no window to show in a --http-only backend ({path: null}).
    fn pick(&self) {
        let w = self.me.clone();
        let dialog = gtk::FileDialog::new();
        dialog.set_title(tr("pickTitle"));
        dialog.set_modal(true);
        dialog.select_folder(Some(self.app.window.window()), gio::Cancellable::NONE, move |result| {
            let Some(e) = w.upgrade() else { return };
            match result {
                Ok(folder) => {
                    let path = folder.path().and_then(|p| p.to_str().map(str::to_owned)).unwrap_or_default();
                    if !path.is_empty() {
                        e.set_scope(&Scope { label: util::pretty_path(&path), path }, false);
                    }
                }
                Err(err) if err.matches(gtk::DialogError::Dismissed) || err.matches(gtk::DialogError::Cancelled) => {}
                Err(err) => {
                    e.app.toast(Toast::new(tr("noPicker")).message(err.message()).severity(Severity::Warning));
                }
            }
        });
    }

    // -------------------------------------------------------------------------------------------- api

    /// The scope (folder) to explore; the same scope keeps its tree unless `refresh`.
    pub fn set_scope(&self, s: &Scope, refresh: bool) {
        let same = self.scope.borrow().as_ref().is_some_and(|c| c.path == s.path);
        *self.scope.borrow_mut() = Some(s.clone());
        if same && self.root_tree.borrow().is_some() && !refresh {
            self.render(None);
            return;
        }
        self.root_tree.borrow_mut().take();
        self.trail.borrow_mut().clear();
        self.run_scan(&s.path, refresh);
    }

    pub fn refresh(&self) {
        let path = self.scope.borrow().as_ref().map(|s| s.path.clone());
        if let Some(p) = path {
            self.run_scan(&p, true);
        }
    }
}
