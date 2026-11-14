//! Samenvatting / Summary (js/apps/taskmgr/pages/summary.js, css/taskmgr.css "Samenvatting"): the plain-language
//! health hero (sysmon.health) with one-click fixes, five live mini graphs that open Prestaties, the apps with the
//! most impact (they open Processen on the app) and the disks with their free space (they open Opslag), and what
//! was checked and found fine.
use super::Page;
use super::storage::layout::{self, Bar, Layout, VAlign};
use crate::app::App;
use crate::hostcall;
use crate::store::Subscription;
use crate::util::{self, TmGlyph, fill, plural};
use kit::chart::{LineChart, LineOptions, Series};
use kit::controls::Variant;
use kit::format::{bitrate, bytes};
use kit::prelude::*;
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Samenvatting", "Summary"),
        "recheck" => ("Opnieuw controleren", "Check again"),
        "okTitle" => ("Je pc werkt prima", "Your PC is running fine"),
        "okBody" => (
            "Processor, geheugen, schijven en services zijn gecontroleerd. Alles ziet er goed uit.",
            "Processor, memory, disks and services were checked. Everything looks good.",
        ),
        "warnOne" => ("Er vraagt 1 ding je aandacht", "1 thing needs your attention"),
        "warnOther" => ("Er vragen {n} dingen je aandacht", "{n} things need your attention"),
        "warnBody" => (
            "Hieronder staat wat er aan de hand is. Met \u{e9}\u{e9}n klik los je het op.",
            "Here\u{2019}s what\u{2019}s going on. Fix it with one click.",
        ),
        "checked" => ("Gecontroleerd om {time}", "Checked at {time}"),
        "checking" => ("Bezig met controleren\u{2026}", "Checking\u{2026}"),
        "now" => ("Nu in gebruik", "In use now"),
        "topApps" => ("Apps die het meest gebruiken", "Apps using the most"),
        "allProcs" => ("Alle processen", "All processes"),
        "disks" => ("Schijven", "Disks"),
        "allStorage" => ("Opslag bekijken", "View storage"),
        "goodToKnow" => ("Goed om te weten", "Good to know"),
        "allGood" => ("Alles in orde", "All good"),
        "cpu" => ("Processor", "Processor"),
        "memory" => ("Geheugen", "Memory"),
        "disk" => ("Schijf", "Disk"),
        "gpu" => ("Videokaart", "Graphics card"),
        "network" => ("Netwerk", "Network"),
        "ofGB" => ("{used} van {total}", "{used} of {total}"),
        "free" => ("{free} vrij van {total}", "{free} free of {total}"),
        "almostFull" => ("Bijna vol", "Almost full"),
        "impactHigh" => ("Hoog", "High"),
        "impactMid" => ("Gemiddeld", "Medium"),
        "impactLow" => ("Laag", "Low"),
        "sysDisk" => ("Systeemschijf", "System disk"),
        "noGpu" => ("Geen gegevens", "No data"),
        "up" => ("\u{2191} {v}", "\u{2191} {v}"),
        "noApps" => ("Geen apps geopend.", "No apps open."),
        "loading" => ("Laden\u{2026}", "Loading\u{2026}"),
        "openFailed" => ("Kon dit niet openen", "Couldn\u{2019}t open this"),
        "aProcesses" => ("Processen bekijken", "View processes"),
        "aProcess" => ("Proces bekijken", "View process"),
        "aMemory" => ("Geheugengebruik bekijken", "View memory use"),
        "aPerformance" => ("Prestaties bekijken", "View performance"),
        "aServices" => ("Services bekijken", "View services"),
        "aStorage" => ("Opslag bekijken", "View storage"),
        "aFree" => ("Ruimte vrijmaken", "Free up space"),
        "aStartup" => ("Opstart-apps bekijken", "View startup apps"),
        "aDetails" => ("Details bekijken", "View details"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

fn sev_order(s: &str) -> u8 {
    match s {
        "critical" => 0,
        "warning" => 1,
        "info" => 2,
        "ok" => 3,
        _ => 9,
    }
}

/// Methods a health action may ask us to call directly (the host suggests them).
const ACTION_METHODS: [&str; 1] = ["system.openUpdater"];

fn is_own(a: &Value) -> bool {
    let app = util::text(&a["app"]);
    app.is_empty() || app == "taskmgr" || app == "win11shell-taskmgr.desktop"
}

/// summary.js impactOf: CPU and memory share as one figure.
fn impact_of(a: &Value, total: f64) -> f64 {
    util::num(&a["cpu"]) / 8.0 + util::num(&a["memory"]) / (total * 0.06)
}

struct Mini {
    id: &'static str,
    kind: &'static str,
    device: RefCell<String>,
    button: gtk::Button,
    value: gtk::Label,
    sub: gtk::Label,
    chart: LineChart,
}

struct AppRow {
    button: gtk::Button,
    icon: gtk::Box,
    icon_key: RefCell<String>,
    name: gtk::Label,
    meta: gtk::Label,
    impact: gtk::Label,
    bar: Bar,
    pid: RefCell<Value>,
}

struct Summary {
    me: Weak<Summary>,
    app: Rc<App>,
    root: gtk::Box,
    hero: gtk::Box,
    glyph_wrap: gtk::Box,
    hero_title: gtk::Label,
    hero_body: gtk::Label,
    hero_meta: gtk::Label,
    recheck: gtk::Button,
    attention: Layout,
    minis: Vec<Rc<Mini>>,
    apps_list: gtk::Box,
    app_rows: RefCell<Vec<Rc<AppRow>>>,
    vols_list: gtk::Box,
    good: Layout,
    health: RefCell<Option<Rc<Value>>>,
    vols: RefCell<Option<Rc<Value>>>,
    subs: RefCell<Vec<Subscription>>,
    commands: Vec<gtk::Widget>,
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    Rc::new_cyclic(|me: &Weak<Summary>| build(app, me.clone()))
}

fn section(title: &str, link: Option<gtk::Button>, body: &impl IsA<gtk::Widget>) -> gtk::Box {
    let s = layout::vbox(8, "taskmgr-section");
    let head = layout::hbox(12, "taskmgr-section-head");
    let t = layout::section_title(title);
    t.set_hexpand(true);
    t.set_valign(gtk::Align::Center);
    head.append(&t);
    if let Some(l) = link {
        l.set_valign(gtk::Align::Center);
        head.append(&l);
    }
    s.append(&head);
    s.append(body.as_ref());
    s
}

fn build(app: &Rc<App>, me: Weak<Summary>) -> Summary {
    // ---- the hero
    let glyph_wrap = layout::hbox(0, "taskmgr-hero-glyph");
    glyph_wrap.set_size_request(56, 56);
    glyph_wrap.set_valign(gtk::Align::Center);
    glyph_wrap.append(&util::icon(TmGlyph::StatusOk, 56));
    let hero_title = layout::wrap(tr("loading"), "taskmgr-hero-title");
    let hero_body = layout::wrap("", "taskmgr-hero-body");
    hero_body.set_visible(false);
    let text = layout::vbox(0, "taskmgr-hero-text");
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&hero_title);
    text.append(&hero_body);
    let main = layout::hbox(20, "");
    main.append(&glyph_wrap);
    main.append(&text);
    let hero_meta = layout::label("", "taskmgr-hero-meta w11-caption");
    hero_meta.set_halign(gtk::Align::End);
    let recheck = kit::controls::button_with_glyph("refresh", tr("recheck"), Variant::Standard);
    recheck.set_widget_name("taskmgr-recheck");
    recheck.set_halign(gtk::Align::End);
    let side = layout::vbox(8, "taskmgr-hero-side");
    side.set_valign(gtk::Align::Center);
    side.append(&hero_meta);
    side.append(&recheck);
    // .tm-hero wraps below a 760 px page: the side (time, button) goes under the text, aligned left
    let sw = side.downgrade();
    let hero_row = Layout::columns(20, 12, move |w| {
        let side_w = sw.upgrade().map(|s| s.measure(gtk::Orientation::Horizontal, -1).1).unwrap_or(180);
        if w > 654 { vec![w - side_w - 20, side_w] } else { vec![w] }
    })
    .valign_children(VAlign::Center);
    hero_row.append(&main);
    hero_row.append(&side);
    let (mw, rw) = (hero_meta.downgrade(), recheck.downgrade());
    hero_row.connect_width(move |w| {
        let align = if w > 654 { gtk::Align::End } else { gtk::Align::Start };
        if let (Some(m), Some(r)) = (mw.upgrade(), rw.upgrade()) {
            m.set_halign(align);
            r.set_halign(align);
        }
    });
    let hero = layout::vbox(0, "taskmgr-hero");
    hero.append(&hero_row);

    let attention = Layout::auto_fill(380, 8);
    attention.add_css_class("taskmgr-attn");
    attention.set_visible(false);

    // ---- the live tiles
    let defs: [(&'static str, &'static str, &str); 5] =
        [("cpu", "cpu", "cpu"), ("memory", "memory", "memory"), ("disk", "disk", ""), ("gpu", "gpu", "gpu:0"), ("network", "network", "")];
    let mut minis = Vec::new();
    let tiles = Layout::columns(8, 8, |w| {
        // repeat(5) above a 980 px page, 3 up to 620 px, else 2
        let n = if w > 924 {
            5
        } else if w > 564 {
            3
        } else {
            2
        };
        layout::equal(w, n, 8)
    });
    tiles.add_css_class("taskmgr-minis");
    for (id, kind, device) in defs {
        let content = layout::vbox(0, "taskmgr-mini-content");
        let head = layout::hbox(8, "taskmgr-mini-head");
        let k = kind;
        head.append(&layout::dot(8, move || layout::color(&format!("chart-{k}"))));
        let l = layout::label(tr(id), "");
        l.set_hexpand(true);
        head.append(&l);
        let chev = kit::icons::glyph("chevron-right", 12);
        chev.add_css_class("taskmgr-mini-chev");
        head.append(&chev);
        let value = layout::label("\u{2013}", "taskmgr-mini-value");
        let sub = layout::label("", "taskmgr-mini-sub");
        content.append(&head);
        content.append(&value);
        content.append(&sub);
        let chart = LineChart::new(LineOptions {
            max: 100.0,
            color: util::kind_color(kind).into(),
            gridlines: false,
            border: false,
            fill_alpha: 0.14,
            line_width: 1.5,
            points: 60,
            scroll_grid: false,
            ..Default::default()
        });
        chart.widget().set_size_request(-1, 52);
        chart.widget().set_vexpand(false);
        chart.widget().add_css_class("taskmgr-mini-graph");
        let body = layout::vbox(0, "");
        body.append(&content);
        body.append(chart.widget());
        let button = layout::flat_button("taskmgr-mini", &body);
        button.set_overflow(gtk::Overflow::Hidden);
        button.set_widget_name(&format!("taskmgr-mini-{id}"));
        let mini = Rc::new(Mini { id, kind, device: RefCell::new(device.to_owned()), button: button.clone(), value, sub, chart });
        let a = Rc::downgrade(app);
        let m = Rc::downgrade(&mini);
        button.connect_clicked(move |_| {
            if let (Some(a), Some(m)) = (a.upgrade(), m.upgrade()) {
                let d = m.device.borrow().clone();
                a.navigate("performance", json!({ "device": if d.is_empty() { "cpu".to_owned() } else { d } }));
            }
        });
        tiles.append(&button);
        minis.push(mini);
    }

    // ---- apps and volumes
    let apps_list = layout::vbox(0, "taskmgr-list");
    apps_list.append(&layout::empty_line(tr("loading")));
    let vols_list = layout::vbox(0, "taskmgr-list");
    vols_list.append(&layout::empty_line(tr("loading")));
    let good = Layout::auto_fill(280, 8);
    good.add_css_class("taskmgr-good");

    let link_procs = layout::section_link(tr("allProcs"));
    link_procs.set_widget_name("taskmgr-summary-processes");
    let a = Rc::downgrade(app);
    link_procs.connect_clicked(move |_| {
        if let Some(a) = a.upgrade() {
            a.navigate("processes", json!({}));
        }
    });
    let link_storage = layout::section_link(tr("allStorage"));
    link_storage.set_widget_name("taskmgr-summary-storage");
    let a = Rc::downgrade(app);
    link_storage.connect_clicked(move |_| {
        if let Some(a) = a.upgrade() {
            a.navigate("storage", json!({}));
        }
    });
    let apps_card = layout::vbox(0, "taskmgr-card");
    apps_card.append(&apps_list);
    let vols_card = layout::vbox(0, "taskmgr-card");
    vols_card.append(&vols_list);
    // .tm-two: two columns, one below a 900 px page
    let two = Layout::columns(16, 24, |w| if w > 844 { layout::equal(w, 2, 16) } else { vec![w] });
    two.add_css_class("taskmgr-two");
    two.append(&section(tr("topApps"), Some(link_procs), &apps_card));
    two.append(&section(tr("disks"), Some(link_storage), &vols_card));

    let now = section(tr("now"), None, &tiles);
    now.add_css_class("taskmgr-section-spaced");
    let good_section = section(tr("goodToKnow"), None, &good);
    good_section.add_css_class("taskmgr-section-spaced");
    let col = layout::vbox(0, "");
    col.append(&hero);
    col.append(&attention);
    col.append(&now);
    col.append(&two);
    col.append(&good_section);
    let clamp = Layout::clamp(1280);
    clamp.append(&col);
    let root = layout::vbox(0, "taskmgr-scroll taskmgr-summary");
    root.append(&clamp);

    let w = me.clone();
    recheck.connect_clicked(move |_| {
        if let Some(p) = w.upgrade() {
            p.recheck();
        }
    });

    Summary {
        me,
        app: app.clone(),
        root,
        hero,
        glyph_wrap,
        hero_title,
        hero_body,
        hero_meta,
        recheck,
        attention,
        minis,
        apps_list,
        app_rows: RefCell::new(Vec::new()),
        vols_list,
        good,
        health: RefCell::new(None),
        vols: RefCell::new(None),
        subs: RefCell::new(Vec::new()),
        commands: vec![super::new_task_button(app).upcast()],
    }
}

impl Summary {
    // ------------------------------------------------------------------------------------------- health

    /// Params for an action that stays inside Task Manager (never a second Task Manager through shell.openApp).
    fn own_params(&self, it: &Value) -> Value {
        let mut p = if it["action"]["params"].is_object() { it["action"]["params"].clone() } else { json!({}) };
        if util::text(&p["page"]).is_empty() {
            p["page"] = json!(if util::text(&p["pid"]).is_empty() { "summary" } else { "processes" });
        }
        if p["page"] == "storage" && util::text(&p["tab"]).is_empty() && util::text(&p["section"]).is_empty() {
            // "Ruimte vrijmaken" for the system / home disk and for a big Prullenbak lands on Opruimen
            let volume = util::text(&p["volume"]);
            let v = if volume.is_empty() {
                None
            } else {
                self.vols
                    .borrow()
                    .as_ref()
                    .and_then(|vs| vs.as_array().and_then(|a| a.iter().find(|x| util::text(&x["id"]) == volume || util::text(&x["mount"]) == volume).cloned()))
            };
            let trash = util::text(&it["id"]).starts_with("trash");
            if trash || volume.is_empty() || v.as_ref().is_some_and(|v| v["isSystem"] == true || v["isHome"] == true) {
                p["tab"] = json!("cleanup");
            }
        }
        p
    }

    /// The button text: the host's, except "Taakbeheer openen"-style labels, which make no sense in here.
    fn action_label(&self, it: &Value) -> String {
        let a = &it["action"];
        let label = util::text(&a["label"]);
        if !is_own(a) {
            return label;
        }
        let p = self.own_params(it);
        let lower = label.to_lowercase();
        let generic = label.is_empty() || lower.contains("taakbeheer") || lower.contains("task manager");
        if !generic {
            return label;
        }
        match p["page"].as_str().unwrap_or("") {
            "processes" => tr(if !util::text(&p["pid"]).is_empty() {
                "aProcess"
            } else if p["sort"] == "memory" {
                "aMemory"
            } else {
                "aProcesses"
            })
            .to_owned(),
            "performance" => tr("aPerformance").to_owned(),
            "services" => tr("aServices").to_owned(),
            "storage" => tr(if p["tab"] == "cleanup" { "aFree" } else { "aStorage" }).to_owned(),
            "startup" => tr("aStartup").to_owned(),
            "details" => tr("aDetails").to_owned(),
            _ => if label.is_empty() { tr("aProcesses").to_owned() } else { label },
        }
    }

    fn run_action(&self, it: &Value) {
        let a = &it["action"];
        if !a.is_object() {
            return;
        }
        let method = util::text(&a["method"]);
        if ACTION_METHODS.contains(&method.as_str()) {
            let app = self.app.clone();
            hostcall::call(&method, json!({}), move |r| {
                if let Err(e) = r {
                    app.toast_error(tr("openFailed"), &e.message);
                }
            });
            return;
        }
        if is_own(a) {
            let p = self.own_params(it);
            let page = util::text(&p["page"]);
            self.app.navigate(&page, p);
            return;
        }
        let params = if a["params"].is_object() { a["params"].clone() } else { json!({}) };
        let app = self.app.clone();
        hostcall::call("shell.openApp", json!({ "app": a["app"], "params": params }), move |r| {
            if let Err(e) = r {
                app.toast_error(tr("openFailed"), &e.message);
            }
        });
    }

    fn render_health(&self) {
        let Some(health) = self.health.borrow().clone() else { return };
        let mut items: Vec<Value> = health.as_array().or_else(|| health["items"].as_array()).cloned().unwrap_or_default();
        items.sort_by_key(|i| sev_order(i["severity"].as_str().unwrap_or("")));
        let sev_of = |i: &Value| i["severity"].as_str().unwrap_or("").to_owned();
        let attn: Vec<&Value> = items.iter().filter(|i| matches!(sev_of(i).as_str(), "warning" | "critical")).collect();
        let infos: Vec<&Value> = items.iter().filter(|i| sev_of(i) == "info").collect();
        let oks: Vec<&Value> = items.iter().filter(|i| sev_of(i) == "ok").collect();
        let sev = if attn.iter().any(|i| sev_of(i) == "critical") {
            "critical"
        } else if !attn.is_empty() {
            "warning"
        } else {
            "ok"
        };
        util::set_class(&self.hero, "taskmgr-hero-warn", sev != "ok");
        kit::controls::clear(&self.glyph_wrap);
        self.glyph_wrap.append(&util::icon(
            match sev {
                "critical" => TmGlyph::StatusCritical,
                "warning" => TmGlyph::StatusWarning,
                _ => TmGlyph::StatusOk,
            },
            56,
        ));
        let n = attn.len() as i64;
        self.hero_title.set_label(&if n > 0 { plural(n, tr("warnOne"), tr("warnOther")) } else { tr("okTitle").to_owned() });
        self.hero_body.set_label(tr(if n > 0 { "warnBody" } else { "okBody" }));
        self.hero_body.set_visible(true);
        self.hero_meta.set_label(&fill(tr("checked"), &[("time", &kit::format::time(kit::format::now_ms(), false, true))]));

        self.attention.clear();
        for it in attn.iter().chain(infos.iter()) {
            let s = sev_of(it);
            let card = layout::hbox(14, &format!("taskmgr-attn-card taskmgr-attn-{s}"));
            let icon = kit::icons::severity_icon(
                match s.as_str() {
                    "critical" => "error",
                    "warning" => "warning",
                    _ => "info",
                },
                20,
            );
            icon.set_valign(gtk::Align::Start);
            icon.set_margin_top(2);
            card.append(&icon);
            let text = layout::vbox(2, "");
            text.set_hexpand(true);
            text.set_valign(gtk::Align::Center);
            text.append(&layout::wrap(&util::text(&it["title"]), "taskmgr-attn-title"));
            let msg = util::text(&it["message"]);
            if !msg.is_empty() {
                text.append(&layout::wrap(&msg, "taskmgr-attn-msg"));
            }
            card.append(&text);
            if it["action"].is_object() {
                let b = kit::controls::button(&self.action_label(it), if s == "info" { Variant::Standard } else { Variant::Accent });
                b.set_valign(gtk::Align::Center);
                b.set_widget_name(&format!("taskmgr-health-{}", util::text(&it["id"])));
                let w = self.me.clone();
                let item = (*it).clone();
                b.connect_clicked(move |_| {
                    if let Some(p) = w.upgrade() {
                        p.run_action(&item);
                    }
                });
                card.append(&b);
            }
            self.attention.append(&card);
        }
        self.attention.set_visible(!attn.is_empty() || !infos.is_empty());

        self.good.clear();
        if oks.is_empty() {
            self.good.append(&layout::empty_line(tr("allGood")));
        }
        for it in oks {
            let item = layout::hbox(12, "taskmgr-good-item");
            let icon = kit::icons::severity_icon("success", 16);
            icon.set_valign(gtk::Align::Start);
            icon.set_margin_top(2);
            item.append(&icon);
            let text = layout::vbox(0, "");
            text.set_hexpand(true);
            text.append(&layout::wrap(&util::text(&it["title"]), "taskmgr-good-title"));
            let msg = util::text(&it["message"]);
            if !msg.is_empty() {
                text.append(&layout::wrap(&msg, "taskmgr-good-msg"));
            }
            item.append(&text);
            self.good.append(&item);
        }
    }

    fn recheck(&self) {
        self.hero_meta.set_label(tr("checking"));
        self.recheck.set_sensitive(false);
        let store = self.app.store.clone();
        let b = self.recheck.downgrade();
        self.app.store.refresh_then("health", move |_| {
            store.refresh_then("volumes", move |_| {
                if let Some(b) = b.upgrade() {
                    b.set_sensitive(true);
                }
            });
        });
    }

    // ------------------------------------------------------------------------------------- performance

    fn render_perf(&self, p: &Value) {
        if !p["cpu"].is_object() {
            return;
        }
        let h = self.app.store.history();
        let last60 = |s: &VecDeque<f64>| crate::store::tail(s, 60);
        let disks = p["disks"].as_array().cloned().unwrap_or_default();
        let sys_disk = disks.iter().find(|d| d["mounts"].as_array().is_some_and(|m| m.iter().any(|x| x == "/"))).or(disks.first()).cloned();
        let nets = p["network"].as_array().cloned().unwrap_or_default();
        let net = nets.iter().find(|n| n["kind"] == "ethernet" || n["kind"] == "wifi").or(nets.first()).cloned();
        for tl in &self.minis {
            tl.chart.set_color(util::kind_color(tl.kind));
            let mut v = "\u{2013}".to_owned();
            let mut sub = String::new();
            let mut series: Vec<f64> = Vec::new();
            match tl.id {
                "cpu" => {
                    let c = &p["cpu"];
                    v = format!("{}%", util::num(&c["usage"]).round() as i64);
                    let t = c["temperature"].as_f64().map(|t| format!("{} \u{b0}C", t.round() as i64));
                    sub = [Some(util::ghz(util::num(&c["freqMHz"]))), t].into_iter().flatten().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ");
                    series = last60(&h.cpu);
                }
                "memory" => {
                    let m = &p["memory"];
                    let (used, total) = (util::num(&m["used"]), util::num(&m["total"]));
                    v = format!("{}%", if total > 0.0 { (used / total * 100.0).round() as i64 } else { 0 });
                    let b1 = |x: f64| kit::format::bytes_digits(x, Some(1));
                    sub = fill(tr("ofGB"), &[("used", &b1(used)), ("total", &b1(total))]);
                    series = last60(&h.mem);
                }
                "disk" => {
                    if let Some(d) = &sys_disk {
                        v = format!("{}%", util::num(&d["activePct"]).round() as i64);
                        sub = format!("{} \u{b7} {}/s", tr("sysDisk"), bytes(util::num(&d["readBps"]) + util::num(&d["writeBps"])));
                        let name = util::text(&d["name"]);
                        series = h.disks.get(&name).map(|x| last60(&x.active)).unwrap_or_default();
                        *tl.device.borrow_mut() = format!("disk:{name}");
                    }
                }
                "gpu" => {
                    let g = p["gpus"].as_array().and_then(|a| a.first());
                    match g.and_then(|g| g["usage"].as_f64().map(|u| (g, u))) {
                        Some((g, u)) => {
                            v = format!("{}%", u.round() as i64);
                            let name = gpu_short(&util::text(&g["name"]));
                            let t = g["temperature"].as_f64().map(|t| format!("{} \u{b0}C", t.round() as i64)).unwrap_or_default();
                            sub = [name, t].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" \u{b7} ");
                            series = h.gpus.get(&0).map(|x| last60(&x.usage)).unwrap_or_default();
                        }
                        None => sub = tr("noGpu").to_owned(),
                    }
                    tl.button.set_visible(g.is_some());
                }
                "network" => {
                    if let Some(n) = &net {
                        v = bitrate(util::num(&n["rxBps"]));
                        let name = [&n["name"], &n["iface"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default();
                        sub = format!("{} \u{b7} {name}", fill(tr("up"), &[("v", &bitrate(util::num(&n["txBps"])))]));
                        let iface = util::text(&n["iface"]);
                        series = h.nets.get(&iface).map(|x| last60(&x.rx)).unwrap_or_default();
                        *tl.device.borrow_mut() = format!("net:{iface}");
                    }
                }
                _ => {}
            }
            if tl.value.label() != v {
                tl.value.set_label(&v);
            }
            if tl.sub.label() != sub {
                tl.sub.set_label(&sub);
            }
            if tl.kind == "network" {
                tl.chart.set_max(series.iter().copied().fold(1.0, f64::max));
            }
            tl.chart.update(vec![Series::new(series)]);
        }
    }

    // ------------------------------------------------------------------------------------------- apps

    fn render_apps(&self, procs: &Value) {
        let total = procs["totals"]["memoryTotal"].as_f64().filter(|t| *t > 0.0).unwrap_or(1.0);
        let mut apps: Vec<Value> = procs["apps"].as_array().cloned().unwrap_or_default();
        apps.sort_by(|a, b| impact_of(b, total).partial_cmp(&impact_of(a, total)).unwrap_or(std::cmp::Ordering::Equal));
        apps.truncate(5);
        if apps.is_empty() {
            self.app_rows.borrow_mut().clear();
            kit::controls::clear(&self.apps_list);
            self.apps_list.append(&layout::empty_line(tr("noApps")));
            return;
        }
        if self.app_rows.borrow().is_empty() {
            kit::controls::clear(&self.apps_list);
        }
        while self.app_rows.borrow().len() < apps.len() {
            let row = self.app_row();
            row.button.set_widget_name(&format!("taskmgr-summary-app-{}", self.app_rows.borrow().len()));
            self.apps_list.append(&row.button);
            self.app_rows.borrow_mut().push(row);
        }
        while self.app_rows.borrow().len() > apps.len() {
            if let Some(r) = self.app_rows.borrow_mut().pop() {
                self.apps_list.remove(&r.button);
            }
        }
        let max_i = apps.iter().map(|a| impact_of(a, total)).fold(0.5, f64::max);
        let rows = self.app_rows.borrow().clone();
        for (a, row) in apps.iter().zip(rows.iter()) {
            let imp = impact_of(a, total);
            let level = if imp > 1.2 {
                "high"
            } else if imp > 0.45 {
                "mid"
            } else {
                "low"
            };
            let icon = util::text(&a["icon"]);
            if *row.icon_key.borrow() != icon || row.icon.first_child().is_none() {
                kit::controls::clear(&row.icon);
                row.icon.append(&util::app_or_glyph(&icon, "app", 24));
                *row.icon_key.borrow_mut() = icon;
            }
            let name = util::text(&a["name"]);
            if row.name.label() != name {
                row.name.set_label(&name);
            }
            let meta = format!("CPU {} \u{b7} {}", util::cpu_pct(util::num(&a["cpu"])), util::mem_mb(util::num(&a["memory"])));
            if row.meta.label() != meta {
                row.meta.set_label(&meta);
            }
            row.impact.set_label(tr(match level {
                "high" => "impactHigh",
                "mid" => "impactMid",
                _ => "impactLow",
            }));
            util::set_class(&row.impact, "taskmgr-impact-high", level == "high");
            row.bar.set_fill((imp / max_i).clamp(0.04, 1.0), if level == "high" { "tm-warn-bar" } else { "accent" });
            *row.pid.borrow_mut() = a["pids"].as_array().and_then(|p| p.first()).cloned().unwrap_or(Value::Null);
        }
    }

    fn app_row(&self) -> Rc<AppRow> {
        let icon = layout::hbox(0, "");
        icon.set_size_request(24, 24);
        icon.set_valign(gtk::Align::Center);
        let name = layout::label("", "taskmgr-app-row-name");
        let meta = layout::label("", "taskmgr-app-row-meta");
        let text = layout::vbox(2, "");
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&name);
        text.append(&meta);
        let impact = layout::label("", "taskmgr-impact-label");
        impact.set_xalign(1.0);
        let bar = Bar::new(4, "taskmgr-impact-bar");
        let imp = layout::vbox(4, "taskmgr-impact");
        imp.set_size_request(110, -1);
        // the bar expands inside its 110 px column only
        imp.set_hexpand(false);
        imp.set_valign(gtk::Align::Center);
        imp.append(&impact);
        imp.append(bar.widget());
        let body = layout::hbox(14, "");
        body.append(&icon);
        body.append(&text);
        body.append(&imp);
        let button = layout::flat_button("taskmgr-app-row", &body);
        let row = Rc::new(AppRow { button, icon, icon_key: RefCell::new(String::new()), name, meta, impact, bar, pid: RefCell::new(Value::Null) });
        let a = Rc::downgrade(&self.app);
        let r = Rc::downgrade(&row);
        row.button.connect_clicked(move |_| {
            if let (Some(a), Some(r)) = (a.upgrade(), r.upgrade()) {
                a.navigate("processes", json!({ "pid": r.pid.borrow().clone() }));
            }
        });
        row
    }

    // ---------------------------------------------------------------------------------------- volumes

    fn render_vols(&self) {
        let Some(vols) = self.vols.borrow().clone() else { return };
        kit::controls::clear(&self.vols_list);
        let mut list: Vec<Value> = vols.as_array().into_iter().flatten().filter(|v| v["hidden"] != true).cloned().collect();
        list.sort_by(|a, b| {
            (b["isSystem"] == true)
                .cmp(&(a["isSystem"] == true))
                .then(util::vol_pct(b).partial_cmp(&util::vol_pct(a)).unwrap_or(std::cmp::Ordering::Equal))
        });
        // the friendly name only (like Verkenner); the mount folder in the tooltip, unless two drives share a name
        let mut seen: HashMap<String, usize> = HashMap::new();
        for v in &list {
            *seen.entry(util::text(&v["name"])).or_default() += 1;
        }
        for v in &list {
            let pct = util::vol_pct(v);
            let level = if pct >= 97.0 {
                "critical"
            } else if pct >= 90.0 {
                "warning"
            } else {
                "ok"
            };
            let icon = kit::icons::app_icon(if v["removable"] == true { "drive-removable-media" } else { "drive-harddisk" }, 24);
            icon.set_valign(gtk::Align::Center);
            let name = util::text(&v["name"]);
            let top = layout::hbox(8, "");
            let nl = layout::label(&name, "taskmgr-vol-row-name");
            top.append(&nl);
            if seen.get(&name).copied().unwrap_or(0) > 1 {
                top.append(&layout::label(&format!("({})", util::text(&v["mount"])), "taskmgr-vol-row-mount"));
            }
            if level != "ok" {
                let pill = layout::label(tr("almostFull"), "taskmgr-pill taskmgr-is-warning");
                pill.set_ellipsize(gtk::pango::EllipsizeMode::None);
                pill.set_valign(gtk::Align::Center);
                top.append(&pill);
            }
            let bar = Bar::new(6, "taskmgr-vol-row-bar");
            bar.set_fill(pct.min(100.0) / 100.0, if level == "ok" { "accent" } else { "tm-danger-bar" });
            let meta =
                layout::label(&fill(tr("free"), &[("free", &bytes(util::num(&v["free"]))), ("total", &bytes(util::num(&v["total"])))]), "taskmgr-vol-row-meta");
            let text = layout::vbox(5, "");
            text.set_hexpand(true);
            text.set_valign(gtk::Align::Center);
            text.append(&top);
            text.append(bar.widget());
            text.append(&meta);
            let pl = layout::label(&format!("{}%", pct.round() as i64), "taskmgr-vol-row-pct");
            pl.set_xalign(1.0);
            pl.set_size_request(44, -1);
            pl.set_valign(gtk::Align::Center);
            let body = layout::hbox(14, "");
            body.append(&icon);
            body.append(&text);
            body.append(&pl);
            let row = layout::flat_button(&format!("taskmgr-vol-row taskmgr-is-{level}"), &body);
            row.set_tooltip_text(Some(&util::text(&v["mount"])));
            row.set_widget_name(&format!("taskmgr-summary-volume-{}", util::text(&v["id"])));
            let target = [&v["id"], &v["mount"]].into_iter().map(util::text).find(|s| !s.is_empty()).unwrap_or_default();
            let a = Rc::downgrade(&self.app);
            row.connect_clicked(move |_| {
                if let Some(a) = a.upgrade() {
                    a.navigate("storage", json!({ "volume": target }));
                }
            });
            self.vols_list.append(&row);
        }
    }
}

/// "NVIDIA GeForce RTX 3060" -> "RTX 3060" (summary.js: NVIDIA, AMD, Intel with GeForce / Radeon).
fn gpu_short(name: &str) -> String {
    let mut rest = name;
    if let Some(r) = ["nvidia", "amd", "intel"].iter().find_map(|p| layout::strip_word(rest, p)) {
        rest = ["geforce", "radeon"].iter().find_map(|q| layout::strip_word(r, q)).unwrap_or(r);
    }
    rest.to_owned()
}

impl Page for Summary {
    fn id(&self) -> &'static str {
        "summary"
    }

    fn title(&self) -> String {
        tr("title").to_owned()
    }

    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }

    fn commands(&self) -> Vec<gtk::Widget> {
        self.commands.clone()
    }

    fn mount(&self, _params: &Value) {
        let w = self.me.clone();
        let health = self.app.store.subscribe("health", move |d| {
            if let Some(p) = w.upgrade() {
                *p.health.borrow_mut() = Some(d.clone());
                p.render_health();
            }
        });
        let w = self.me.clone();
        let perf = self.app.store.subscribe("performance", move |d| {
            if let Some(p) = w.upgrade() {
                p.render_perf(d);
            }
        });
        let w = self.me.clone();
        let procs = self.app.store.subscribe("processes", move |d| {
            if let Some(p) = w.upgrade() {
                p.render_apps(d);
            }
        });
        let w = self.me.clone();
        let vols = self.app.store.subscribe("volumes", move |d| {
            if let Some(p) = w.upgrade() {
                *p.vols.borrow_mut() = Some(d.clone());
                p.render_vols();
            }
        });
        self.subs.borrow_mut().extend([health, perf, procs, vols]);
    }

    fn unmount(&self) {
        self.subs.borrow_mut().clear();
    }

    fn refresh(&self) -> bool {
        for ch in ["health", "performance", "processes", "volumes"] {
            self.app.store.refresh(ch);
        }
        true
    }
}
