//! Processen (js/apps/taskmgr/pages/processes.js): Win11's grouping (Apps / Achtergrondprocessen /
//! Systeemprocessen), expandable app rows, heat-map cells, the totals over the columns, live updates, the
//! context menu, restart (and "Win11 Shell opnieuw starten" when our own shell is selected, like "Windows
//! Verkenner opnieuw starten").
use super::Page;
use crate::actions::{self, ProcRow, norm_status, pid_of, restart_target_of, target_of};
use crate::app::{App, HealthSub};
use crate::store::Subscription;
use crate::table::{Col, Opts, SortVal, TGroup, TRow, TmTable};
use crate::util::{self, Icon, TmGlyph, fill};
use kit::controls::{DropDownButton, MenuItem, Variant};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Processen", "Processes"),
        "newTask" => ("Nieuwe taak", "Run new task"),
        "endTask" => ("Taak beëindigen", "End task"),
        "efficiency" => ("Efficiëntiemodus", "Efficiency mode"),
        "view" => ("Weergave", "View"),
        "groupByType" => ("Groeperen op type", "Group by type"),
        "expandAll" => ("Alles uitvouwen", "Expand all"),
        "collapseAll" => ("Alles samenvouwen", "Collapse all"),
        "columns" => ("Kolommen", "Columns"),
        "apps" => ("Apps", "Apps"),
        "background" => ("Achtergrondprocessen", "Background processes"),
        "system" => ("Systeemprocessen", "System processes"),
        "name" => ("Naam", "Name"),
        "status" => ("Status", "Status"),
        "cpu" => ("CPU", "CPU"),
        "memory" => ("Geheugen", "Memory"),
        "disk" => ("Schijf", "Disk"),
        "pid" => ("PID", "PID"),
        "procName" => ("Procesnaam", "Process name"),
        "cmd" => ("Opdrachtregel", "Command line"),
        "user" => ("Gebruiker", "User"),
        "notResponding" => ("Reageert niet", "Not responding"),
        "suspended" => ("Onderbroken", "Suspended"),
        "effTip" => ("Efficiëntiemodus staat aan", "Efficiency mode is on"),
        "noMatch" => ("Geen processen gevonden voor \u{201c}{q}\u{201d}.", "No processes found for \u{201c}{q}\u{201d}."),
        "loading" => ("Processen laden\u{2026}", "Loading processes\u{2026}"),
        "waiting" => ("Wachten op Win11 Shell\u{2026}", "Waiting for Win11 Shell\u{2026}"),
        "systemProc" => ("Systeemproces \u{2013} kan niet worden beëindigd", "System process \u{2013} can\u{2019}t be ended"),
        "selectFirst" => ("Selecteer eerst een app of proces", "Select an app or process first"),
        "cpuTip" => ("Hoeveel rekenkracht van de processor dit gebruikt", "How much processor power this uses"),
        "memTip" => ("Hoeveel werkgeheugen dit gebruikt", "How much memory this uses"),
        "diskTip" => ("Hoeveel dit van en naar de schijf leest en schrijft", "How much this reads from and writes to disk"),
        "newTaskTip" => ("Een app zoeken en starten", "Find and start an app"),
        "restart" => ("Opnieuw starten", "Restart"),
        "restartShell" => ("Win11 Shell opnieuw starten", "Restart Win11 Shell"),
        "restartTip" => (
            "Stoppen en meteen weer starten \u{2013} handig als iets is blijven hangen",
            "Stop and start again right away \u{2013} handy when something got stuck",
        ),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

// ------------------------------------------------------------------------------------------------ row model

fn child_type_label(t: &str) -> String {
    let en = kit::i18n::en();
    let s = match (t, en) {
        ("renderer", false) => "weergave",
        ("renderer", true) => "renderer",
        ("gpu-process" | "gpu", false) => "GPU-proces",
        ("gpu-process" | "gpu", true) => "GPU process",
        ("utility", false) => "hulpproces",
        ("utility", true) => "utility",
        ("zygote", false) => "starter",
        ("zygote", true) => "zygote",
        ("crashpad-handler", false) => "foutrapportage",
        ("crashpad-handler", true) => "crash reporter",
        ("broker", _) => "broker",
        ("tab", false) => "tabblad",
        ("tab", true) => "tab",
        ("rdd", _) => "media",
        ("socket", false) => "netwerk",
        ("socket", true) => "network",
        ("content", false) => "inhoud",
        ("content", true) => "content",
        (other, _) => return other.to_owned(),
    };
    s.to_owned()
}

fn is_stopped(state: &str) -> bool {
    matches!(state, "T" | "t" | "stopped")
}

/// A helpful name for a child process of an app ("Firefox (tabblad)", "Web Content", "tsserver").
fn child_name(p: &Value, parent: &str) -> String {
    let display = util::text(&p["displayName"]);
    if !display.is_empty() && display != parent {
        return display;
    }
    let cmd = util::text(&p["cmd"]);
    if let Some(i) = cmd.find("--type=") {
        let t: String = cmd[i + 7..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
        if !t.is_empty() {
            return format!("{parent} ({})", child_type_label(&t));
        }
    }
    if cmd.contains("-contentproc") {
        let last = cmd.trim_end().rsplit(' ').next().unwrap_or("");
        let kind = if ["tab", "rdd", "socket", "utility", "gpu"].contains(&last) { last } else { "content" };
        return format!("{parent} ({})", child_type_label(kind));
    }
    let name = util::text(&p["name"]);
    if !name.is_empty() && name.to_lowercase() != parent.to_lowercase() {
        return name;
    }
    parent.to_owned()
}

fn proc_row(p: &Value, parent: Option<&Rc<ProcRow>>) -> ProcRow {
    let pid = pid_of(p);
    let name = match parent {
        Some(par) => child_name(p, &par.name),
        None => {
            let d = util::text(&p["displayName"]);
            if d.is_empty() { util::text(&p["name"]) } else { d }
        }
    };
    let status = if is_stopped(p["state"].as_str().unwrap_or("")) {
        "suspended"
    } else if p["efficiency"] == true {
        "efficiency"
    } else if parent.is_some_and(|par| par.status == "notResponding" && par.pids.first().copied() == pid) {
        "notResponding"
    } else {
        "running"
    };
    ProcRow {
        kind: "proc",
        key: format!("p:{}", pid.unwrap_or(0)),
        name,
        display_name: util::text(&p["displayName"]),
        icon: parent.map(|par| par.icon.clone()).unwrap_or_default(),
        pids: pid.into_iter().collect(),
        procs: Rc::new(vec![p.clone()]),
        cpu: util::num(&p["cpu"]),
        memory: util::num(&p["memory"]),
        disk: util::num(&p["diskRead"]) + util::num(&p["diskWrite"]),
        threads: util::num(&p["threads"]),
        status,
        can_kill: p["canKill"] != false,
        efficiency: p["efficiency"] == true,
        cmd: util::text(&p["cmd"]),
        exe: util::text(&p["exe"]),
        user: util::text(&p["user"]),
        proc_name: util::text(&p["name"]),
        pid,
        parent: parent.cloned(),
        ..Default::default()
    }
}

fn group_row(kind: &'static str, g: &Value, by_pid: &HashMap<i64, Value>) -> ProcRow {
    let gpids: Vec<i64> = g["pids"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
    let procs: Vec<Value> = gpids.iter().filter_map(|p| by_pid.get(p).cloned()).collect();
    let prefix = &kind[..1];
    let icon = util::text(&g["icon"]);
    let mut row = ProcRow {
        kind,
        key: format!("{prefix}:{}", util::text(&g["key"])),
        name: util::text(&g["name"]),
        icon,
        pids: if procs.is_empty() { gpids.clone() } else { procs.iter().filter_map(pid_of).collect() },
        cpu: util::num(&g["cpu"]),
        memory: util::num(&g["memory"]),
        disk: util::num(&g["diskRate"]),
        status: norm_status(g["status"].as_str().unwrap_or("")),
        windows: g["windows"].as_array().map(|a| a.iter().map(util::text).collect()).unwrap_or_default(),
        can_kill: if procs.is_empty() { g["canKill"] != false } else { procs.iter().all(|p| p["canKill"] != false) },
        efficiency: if procs.is_empty() { g["efficiency"] == true } else { procs.iter().all(|p| p["efficiency"] == true) },
        cmd: procs.first().map(|p| util::text(&p["cmd"])).unwrap_or_default(),
        exe: procs.first().map(|p| util::text(&p["exe"])).unwrap_or_default(),
        user: procs.first().map(|p| util::text(&p["user"])).filter(|u| !u.is_empty()).unwrap_or_else(|| util::text(&g["user"])),
        proc_name: procs.first().map(|p| util::text(&p["name"])).unwrap_or_default(),
        pid: if procs.len() == 1 { pid_of(&procs[0]) } else { None },
        threads: procs.iter().map(|p| util::num(&p["threads"])).sum(),
        ..Default::default()
    };
    if row.status == "running" && row.efficiency {
        row.status = "efficiency";
    }
    let n = procs.len();
    row.procs = Rc::new(procs);
    if n > 1 {
        // the children's parent is a copy without children (no reference cycle)
        let parent = Rc::new(row.clone());
        row.children = row.procs.iter().map(|p| Rc::new(proc_row(p, Some(&parent)))).collect();
    }
    row
}

/// The groups of a sysmon.processes answer (processes.js buildProcessGroups): (id, label, rows).
pub fn build_groups(data: &Value, grouped: bool) -> Vec<(&'static str, String, Vec<Rc<ProcRow>>)> {
    let all = data["processes"].as_array().cloned().unwrap_or_default();
    let by_pid: HashMap<i64, Value> = all.iter().filter_map(|p| pid_of(p).map(|pid| (pid, p.clone()))).collect();
    let list = |field: &str, kind: &'static str| -> Vec<Rc<ProcRow>> {
        data[field].as_array().into_iter().flatten().map(|g| Rc::new(group_row(kind, g, &by_pid))).collect()
    };
    let apps = list("apps", "app");
    let bg = list("background", "bg");
    let sys: Vec<Rc<ProcRow>> = if data["system"].is_array() {
        list("system", "sys")
    } else {
        // SPEC-only payloads: group other users' processes by groupKey / name
        let mut order: Vec<String> = Vec::new();
        let mut groups: HashMap<String, Value> = HashMap::new();
        for p in all.iter().filter(|p| p["group"] == "system") {
            let k = p["groupKey"].as_str().map(str::to_owned).unwrap_or_else(|| util::text(&p["name"]));
            let g = groups.entry(k.clone()).or_insert_with(|| {
                order.push(k.clone());
                let d = util::text(&p["displayName"]);
                json!({ "key": k, "name": if d.is_empty() { util::text(&p["name"]) } else { d }, "pids": [], "cpu": 0.0, "memory": 0.0, "diskRate": 0.0, "status": "running" })
            });
            if let Some(a) = g["pids"].as_array_mut() {
                a.push(p["pid"].clone());
            }
            g["cpu"] = json!(util::num(&g["cpu"]) + util::num(&p["cpu"]));
            g["memory"] = json!(util::num(&g["memory"]) + util::num(&p["memory"]));
            g["diskRate"] = json!(util::num(&g["diskRate"]) + util::num(&p["diskRead"]) + util::num(&p["diskWrite"]));
        }
        order.iter().filter_map(|k| groups.get(k)).map(|g| Rc::new(group_row("sys", g, &by_pid))).collect()
    };
    if grouped {
        vec![("apps", tr("apps").to_owned(), apps), ("background", tr("background").to_owned(), bg), ("system", tr("system").to_owned(), sys)]
    } else {
        vec![("all", String::new(), apps.into_iter().chain(bg).chain(sys).collect())]
    }
}

/// What the table shows of a row.
fn row_value(r: &ProcRow, mem_total: f64) -> Value {
    let app_icon = if r.kind == "app" || (r.kind == "proc" && !r.icon.is_empty()) { r.icon.clone() } else { String::new() };
    json!({
        "name": r.name,
        "icon": app_icon,
        "glyph": if r.kind == "sys" { "settings" } else { "window" },
        "count": if r.children.is_empty() { String::new() } else { format!(" ({})", r.children.len()) },
        "nameClass": if r.status == "notResponding" { "taskmgr-hang" } else { "" },
        "status": r.status,
        "pid": r.pid,
        "pids": r.pids,
        "procName": r.proc_name,
        "user": r.user,
        "cpu": r.cpu,
        "memory": r.memory,
        "disk": r.disk,
        "cmd": r.cmd,
        "windows": r.windows.join("\n"),
        "canKill": r.can_kill,
        "kind": r.kind,
        "heatCpu": r.cpu / 35.0,
        "heatMem": r.memory / (mem_total.max(1.0) * 0.1),
        "heatDisk": r.disk / (25.0 * 1_048_576.0),
    })
}

fn to_trow(r: &Rc<ProcRow>, mem_total: f64, map: &mut HashMap<String, Rc<ProcRow>>) -> TRow {
    map.insert(r.key.clone(), r.clone());
    let mut t = TRow::new(r.key.clone(), row_value(r, mem_total));
    t.children = r.children.iter().map(|c| to_trow(c, mem_total, map)).collect();
    t
}

// ------------------------------------------------------------------------------------------------------ page

struct Processes {
    me: Weak<Processes>,
    app: Rc<App>,
    root: gtk::Box,
    table: TmTable,
    grouped: Cell<bool>,
    data: RefCell<Option<Rc<Value>>>,
    perf: RefCell<Option<Rc<Value>>>,
    query: RefCell<String>,
    subs: RefCell<Vec<Subscription>>,
    health_sub: RefCell<Option<HealthSub>>,
    rows: RefCell<HashMap<String, Rc<ProcRow>>>,
    end_btn: gtk::Button,
    end_wrap: gtk::Box,
    restart_btn: gtk::Button,
    restart_wrap: gtk::Box,
    eff_btn: gtk::Button,
    eff_wrap: gtk::Box,
    commands: Vec<gtk::Widget>,
}

fn status_cell() -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.add_css_class("taskmgr-status");
    let leaf = util::icon(TmGlyph::Leaf, 16);
    let pause = util::icon(TmGlyph::Pause, 14);
    let l = gtk::Label::new(None);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    b.append(&leaf);
    b.append(&pause);
    b.append(&l);
    b.upcast()
}

fn status_bind(w: &gtk::Widget, v: &Value) {
    let (Some(leaf), Some(pause), Some(label)) = (w.first_child(), w.first_child().and_then(|c| c.next_sibling()), w.last_child().and_downcast::<gtk::Label>()) else {
        return;
    };
    let st = v["status"].as_str().unwrap_or("running");
    leaf.set_visible(st == "efficiency");
    pause.set_visible(st == "suspended");
    let (text, hang) = match st {
        "notResponding" => (tr("notResponding"), true),
        "suspended" => (tr("suspended"), false),
        _ => ("", false),
    };
    label.set_label(text);
    label.set_visible(!text.is_empty());
    util::set_class(&label, "taskmgr-hang", hang);
    w.set_tooltip_text(if st == "efficiency" { Some(tr("effTip")) } else { None });
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let page = Rc::new_cyclic(|weak: &Weak<Processes>| build(app, weak.clone()));
    page.wire();
    page
}

fn build(app: &Rc<App>, weak: Weak<Processes>) -> Processes {
    let cols = vec![
        Col::name("name", tr("name"), 16).width(280).flex().required().tip(|v| util::text(&v["windows"])),
        Col::custom("status", tr("status"), status_cell, status_bind).width(140).value(|v| {
            SortVal::Num(match v["status"].as_str().unwrap_or("") {
                "notResponding" => 0.0,
                "suspended" => 1.0,
                "efficiency" => 2.0,
                _ => 3.0,
            })
        }),
        Col::text("pid", tr("pid"), |v| util::text(&v["pid"])).width(76).right().hidden().num("pid"),
        Col::text("procName", tr("procName"), |v| util::text(&v["procName"])).width(150).hidden(),
        Col::text("user", tr("user"), |v| util::text(&v["user"])).width(110).hidden(),
        Col::text("cpu", tr("cpu"), |v| util::cpu_pct(util::num(&v["cpu"])))
            .width(86)
            .right()
            .default_desc()
            .header_tip(tr("cpuTip"))
            .num("cpu")
            .heat(|v| util::num(&v["heatCpu"])),
        Col::text("memory", tr("memory"), |v| util::mem_mb(util::num(&v["memory"])))
            .width(104)
            .right()
            .default_desc()
            .header_tip(tr("memTip"))
            .num("memory")
            .heat(|v| util::num(&v["heatMem"])),
        Col::text("disk", tr("disk"), |v| util::disk_mbs(util::num(&v["disk"])))
            .width(96)
            .right()
            .default_desc()
            .header_tip(tr("diskTip"))
            .num("disk")
            .heat(|v| util::num(&v["heatDisk"])),
        Col::text("cmd", tr("cmd"), |v| util::text(&v["cmd"])).width(320).hidden().class("taskmgr-mono"),
    ];
    let table = TmTable::new(
        cols,
        Opts {
            storage_key: "processes",
            multi: false,
            tree: true,
            default_sort: None,
            row_class: Some(Rc::new(|v: &Value| {
                let mut c = Vec::new();
                match v["status"].as_str() {
                    Some("notResponding") => c.push("taskmgr-row-hung"),
                    Some("efficiency") => c.push("taskmgr-row-efficient"),
                    _ => {}
                }
                if v["canKill"] == false {
                    c.push("taskmgr-row-system");
                }
                c
            })),
            rows_class: "taskmgr-rows-30",
            group_counts: true,
        },
    );
    table.view().set_widget_name("taskmgr-processes");

    let new_btn = super::new_task_button(app);
    new_btn.set_tooltip_text(Some(tr("newTaskTip")));
    let end_btn = util::cmd_button(actions::END, tr("endTask"));
    end_btn.set_widget_name("taskmgr-end-task");
    end_btn.set_sensitive(false);
    let end_wrap = util::tip_wrap(&end_btn, tr("selectFirst"));
    let restart_btn = util::cmd_button(Icon::Glyph("restart"), tr("restart"));
    restart_btn.set_widget_name("taskmgr-restart");
    restart_btn.set_sensitive(false);
    let restart_wrap = util::tip_wrap(&restart_btn, tr("selectFirst"));
    let eff_btn = util::cmd_button(actions::LEAF, tr("efficiency"));
    eff_btn.set_widget_name("taskmgr-efficiency");
    eff_btn.set_sensitive(false);
    let eff_wrap = util::tip_wrap(&eff_btn, tr("selectFirst"));
    let w = weak.clone();
    let view_btn = DropDownButton::new(Some("view"), tr("view"), Variant::Subtle, move || match w.upgrade() {
        Some(p) => p.view_menu(),
        None => Vec::new(),
    });
    view_btn.widget().add_css_class("taskmgr-cmd");
    view_btn.widget().set_widget_name("taskmgr-view");
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("taskmgr-processes");
    root.set_vexpand(true);
    let tw = table.widget();
    tw.set_vexpand(true);
    root.append(&tw);
    let commands: Vec<gtk::Widget> = vec![
        new_btn.upcast(),
        end_wrap.clone().upcast(),
        restart_wrap.clone().upcast(),
        eff_wrap.clone().upcast(),
        util::cmd_sep(),
        view_btn.widget().clone().upcast(),
    ];
    Processes {
        me: weak,
        app: app.clone(),
        root,
        table,
        grouped: Cell::new(util::load_bool("proc.grouped", true)),
        data: RefCell::new(None),
        perf: RefCell::new(None),
        query: RefCell::new(String::new()),
        subs: RefCell::new(Vec::new()),
        health_sub: RefCell::new(None),
        rows: RefCell::new(HashMap::new()),
        end_btn,
        end_wrap,
        restart_btn,
        restart_wrap,
        eff_btn,
        eff_wrap,
        commands,
    }
}

impl Processes {
    fn weak(&self) -> Weak<Processes> {
        self.me.clone()
    }

    fn wire(self: &Rc<Self>) {
        let w = self.weak();
        self.table.connect_selection_changed(move |_| {
            if let Some(p) = w.upgrade() {
                p.update_commands();
            }
        });
        let w = self.weak();
        self.table.connect_delete(move |keys| {
            let Some(p) = w.upgrade() else { return };
            let row = keys.first().and_then(|k| p.rows.borrow().get(k).cloned());
            if let Some(r) = row {
                actions::end_task(&p.app, &r);
            }
        });
        let w = self.weak();
        self.table.connect_context_menu(move |keys| {
            let p = w.upgrade()?;
            let row = keys.first().and_then(|k| p.rows.borrow().get(k).cloned())?;
            util::fix_menu_glyphs(&p.table.view(), actions::menu_glyphs(&row));
            Some(actions::process_menu(&p.app, &row, Some(&p.table)))
        });
        let w = self.weak();
        self.end_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(r) = p.selected()
            {
                actions::end_task(&p.app, &r);
            }
        });
        let w = self.weak();
        self.restart_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(r) = p.selected()
            {
                actions::restart_task(&p.app, &r);
            }
        });
        let w = self.weak();
        self.eff_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(r) = p.selected()
            {
                actions::set_efficiency(&p.app, &r, !target_of(&r).efficiency);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.weak();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) && state.contains(gdk::ModifierType::ALT_MASK)
                && let Some(r) = p.selected()
            {
                actions::show_properties(&p.app, &r);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.root.add_controller(keys);
    }

    fn selected(&self) -> Option<Rc<ProcRow>> {
        let key = self.table.selection().into_iter().next()?;
        self.rows.borrow().get(&key).cloned()
    }

    fn view_menu(self: &Rc<Self>) -> Vec<MenuItem> {
        let w = self.weak();
        let grouped = self.grouped.get();
        let mut items = vec![
            MenuItem::new(tr("groupByType"), move || {
                if let Some(p) = w.upgrade() {
                    p.grouped.set(!grouped);
                    util::save("proc.grouped", json!(!grouped));
                    p.render();
                }
            })
            .checked(grouped)
            .name("taskmgr-group-by-type"),
            MenuItem::separator(),
        ];
        let t = self.table.clone();
        items.push(MenuItem::new(tr("expandAll"), move || t.expand_all(true)).glyph("chevron-down"));
        let t = self.table.clone();
        items.push(MenuItem::new(tr("collapseAll"), move || t.expand_all(false)).glyph("chevron-up"));
        items.push(MenuItem::separator());
        items.push(MenuItem::submenu(tr("columns"), self.table.column_items()).glyph("grid"));
        items
    }

    fn update_commands(&self) {
        let r = self.selected();
        let rt = restart_target_of(r.as_deref());
        util::set_button_label(&self.restart_btn, if rt.kind == "shell" { tr("restartShell") } else { tr("restart") });
        self.restart_btn.set_sensitive(rt.ok);
        let tip = match (&r, rt.ok) {
            (None, _) => tr("selectFirst").to_owned(),
            (Some(_), true) => if rt.kind == "shell" { String::new() } else { tr("restartTip").to_owned() },
            (Some(_), false) => rt.reason.clone(),
        };
        util::set_tip(&self.restart_wrap, &tip);
        let Some(r) = r else {
            self.end_btn.set_sensitive(false);
            self.eff_btn.set_sensitive(false);
            util::set_tip(&self.end_wrap, tr("selectFirst"));
            util::set_tip(&self.eff_wrap, tr("selectFirst"));
            self.eff_btn.remove_css_class("taskmgr-pressed");
            return;
        };
        let tg = target_of(&r);
        self.end_btn.set_sensitive(tg.can_kill);
        util::set_tip(&self.end_wrap, if tg.can_kill { "" } else { tr("systemProc") });
        self.eff_btn.set_sensitive(tg.can_kill);
        util::set_tip(&self.eff_wrap, if tg.can_kill { "" } else { tr("systemProc") });
        util::set_class(&self.eff_btn, "taskmgr-pressed", tg.efficiency);
    }

    fn mem_total(&self) -> f64 {
        let t = self.data.borrow().as_ref().map(|d| util::num(&d["totals"]["memoryTotal"])).unwrap_or(0.0);
        if t > 0.0 {
            return t;
        }
        let p = self.perf.borrow().as_ref().map(|d| util::num(&d["memory"]["total"])).unwrap_or(0.0);
        if p > 0.0 { p } else { 1.0 }
    }

    fn render(&self) {
        let Some(data) = self.data.borrow().clone() else { return };
        let mem_total = self.mem_total();
        let mut map = HashMap::new();
        let groups: Vec<TGroup> = build_groups(&data, self.grouped.get())
            .into_iter()
            .map(|(id, label, rows)| TGroup { id: id.into(), label, rows: rows.iter().map(|r| to_trow(r, mem_total, &mut map)).collect() })
            .collect();
        *self.rows.borrow_mut() = map;
        self.table.set_groups(groups);
        self.update_header();
        self.update_commands();
    }

    fn update_header(&self) {
        let data = self.data.borrow().clone();
        let perf = self.perf.borrow().clone();
        let tot = data.as_ref().map(|d| d["totals"].clone()).unwrap_or(Value::Null);
        let cpu = tot["cpu"].as_f64().map(|c| format!("{}%", c.round() as i64)).unwrap_or_default();
        let mem = match (tot["memoryUsed"].as_f64(), tot["memoryTotal"].as_f64()) {
            (Some(u), Some(t)) if t > 0.0 => format!("{}%", (u / t * 100.0).round() as i64),
            _ => String::new(),
        };
        let disk = if let Some(a) = tot["diskActive"].as_f64() {
            format!("{}%", a.round() as i64)
        } else if let Some(disks) = perf.as_ref().and_then(|p| p["disks"].as_array().cloned()).filter(|d| !d.is_empty()) {
            format!("{}%", disks.iter().map(|d| util::num(&d["activePct"])).fold(0.0, f64::max).round() as i64)
        } else if tot["diskRead"].is_number() {
            util::disk_mbs(util::num(&tot["diskRead"]) + util::num(&tot["diskWrite"]))
        } else {
            String::new()
        };
        self.table.set_header_values(&[("cpu", cpu), ("memory", mem), ("disk", disk)]);
    }

    fn apply_filter(&self) {
        let q = self.query.borrow().trim().to_lowercase();
        let has_data = self.data.borrow().is_some();
        let empty = if has_data {
            fill(tr("noMatch"), &[("q", self.query.borrow().trim())])
        } else if crate::hostcall::health().is_ok() {
            tr("loading").to_owned()
        } else {
            tr("waiting").to_owned()
        };
        self.table.set_empty_text(&empty);
        if q.is_empty() {
            self.table.set_filter(None);
            return;
        }
        let is_num = q.chars().all(|c| c.is_ascii_digit());
        self.table.set_filter(Some(Rc::new(move |v: &Value| {
            if is_num && v["pids"].as_array().is_some_and(|a| a.iter().any(|p| util::text(p).starts_with(&q))) {
                return true;
            }
            util::text(&v["name"]).to_lowercase().contains(&q) || util::text(&v["procName"]).to_lowercase().contains(&q)
        })));
    }

    fn sort_param(&self, col: &str) {
        let id = match col {
            "memory" | "cpu" | "disk" | "name" => col,
            _ => return,
        };
        self.table.sort_by(id, id != "name");
    }

    fn select_pid(&self, pid: i64) {
        let Some(data) = self.data.borrow().clone() else { return };
        for (_, _, rows) in build_groups(&data, self.grouped.get()) {
            for r in rows {
                if r.pids.contains(&pid) {
                    if !r.children.is_empty() && r.pids.len() > 1 {
                        self.table.expand(&r.key, true);
                        self.table.select(&format!("p:{pid}"), true);
                    } else {
                        self.table.select(&r.key, true);
                    }
                    return;
                }
            }
        }
    }
}

fn param_pid(params: &Value) -> Option<i64> {
    params["pid"].as_i64().or_else(|| params["pid"].as_str().and_then(|s| s.trim().parse().ok()))
}

impl Page for Processes {
    fn id(&self) -> &'static str {
        "processes"
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

    fn fill(&self) -> bool {
        true
    }

    fn mount(&self, params: &Value) {
        let pid = param_pid(params);
        let first_pid = Rc::new(Cell::new(pid));
        let weak = self.weak();
        let fp = first_pid.clone();
        let sub = self.app.store.subscribe("processes", move |d| {
            let Some(p) = weak.upgrade() else { return };
            let first = p.data.borrow().is_none();
            *p.data.borrow_mut() = Some(d.clone());
            p.render();
            if first {
                p.apply_filter();
                if let Some(pid) = fp.take() {
                    p.select_pid(pid);
                }
            }
        });
        self.subs.borrow_mut().push(sub);
        let weak = self.weak();
        let sub = self.app.store.subscribe("performance", move |d| {
            if let Some(p) = weak.upgrade() {
                *p.perf.borrow_mut() = Some(d.clone());
                p.update_header();
            }
        });
        self.subs.borrow_mut().push(sub);
        let weak = self.weak();
        *self.health_sub.borrow_mut() = Some(self.app.on_health(move |_, _| {
            if let Some(p) = weak.upgrade()
                && p.data.borrow().is_none()
            {
                p.apply_filter();
            }
        }));
        if self.data.borrow().is_none() {
            self.apply_filter();
        }
        if self.data.borrow().is_some()
            && let Some(pid) = first_pid.take()
        {
            self.select_pid(pid);
        }
        if let Some(s) = params["sort"].as_str() {
            self.sort_param(s);
        }
    }

    fn navigate(&self, params: &Value) {
        if let Some(pid) = param_pid(params)
            && self.data.borrow().is_some()
        {
            self.select_pid(pid);
        }
        if let Some(s) = params["sort"].as_str() {
            self.sort_param(s);
        }
    }

    fn unmount(&self) {
        self.subs.borrow_mut().clear();
        self.health_sub.borrow_mut().take();
    }

    fn searchable(&self) -> bool {
        true
    }

    fn on_search(&self, query: &str) {
        *self.query.borrow_mut() = query.to_owned();
        self.apply_filter();
    }

    fn refresh(&self) -> bool {
        self.app.store.refresh("processes");
        true
    }
}

/// The rows of a sysmon.processes answer flattened (main.rs debugOpen).
pub fn all_rows(data: &Value) -> Vec<Rc<ProcRow>> {
    build_groups(data, true).into_iter().flat_map(|(_, _, rows)| rows).collect()
}
