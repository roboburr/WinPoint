//! Details (js/apps/taskmgr/pages/details.js): every process in one flat, sortable table (multiple selection).
use super::Page;
use crate::actions::{self, ProcRow, combine, norm_status, pid_of, restart_target_of, target_of};
use crate::app::{App, HealthSub};
use crate::store::Subscription;
use crate::table::{Col, Opts, TRow, TmTable};
use crate::util::{self, Icon, fill};
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Details", "Details"),
        "endTask" => ("Taak beëindigen", "End task"),
        "efficiency" => ("Efficiëntiemodus", "Efficiency mode"),
        "properties" => ("Eigenschappen", "Properties"),
        "name" => ("Naam", "Name"),
        "pid" => ("PID", "PID"),
        "status" => ("Status", "Status"),
        "user" => ("Gebruiker", "User name"),
        "cpu" => ("CPU", "CPU"),
        "memory" => ("Geheugen", "Memory"),
        "threads" => ("Threads", "Threads"),
        "cmd" => ("Opdrachtregel", "Command line"),
        "desc" => ("Omschrijving", "Description"),
        "running" => ("Actief", "Running"),
        "waiting" => ("Wacht", "Waiting"),
        "suspended" => ("Onderbroken", "Suspended"),
        "zombie" => ("Afgesloten", "Exited"),
        "hung" => ("Reageert niet", "Not responding"),
        "loading" => ("Processen laden\u{2026}", "Loading processes\u{2026}"),
        "waitingHost" => ("Wachten op Win11 Shell\u{2026}", "Waiting for Win11 Shell\u{2026}"),
        "noMatch" => ("Geen processen gevonden voor \u{201c}{q}\u{201d}.", "No processes found for \u{201c}{q}\u{201d}."),
        "systemProc" => ("Systeemproces \u{2013} kan niet worden beëindigd", "System process \u{2013} can\u{2019}t be ended"),
        "selectFirst" => ("Selecteer eerst een proces", "Select a process first"),
        "restart" => ("Opnieuw starten", "Restart"),
        "restartShell" => ("Win11 Shell opnieuw starten", "Restart Win11 Shell"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

fn state_label(p: &Value, hung: bool) -> &'static str {
    if hung {
        return tr("hung");
    }
    let s = p["state"].as_str().unwrap_or("S");
    let word = match s {
        "running" => 'R',
        "sleeping" => 'S',
        "idle" => 'I',
        "waiting" => 'D',
        "stopped" => 'T',
        "zombie" => 'Z',
        other => other.chars().next().unwrap_or('S'),
    };
    match word {
        'D' => tr("waiting"),
        'T' | 't' => tr("suspended"),
        'Z' | 'X' => tr("zombie"),
        _ => tr("running"),
    }
}

struct Details {
    me: Weak<Details>,
    app: Rc<App>,
    root: gtk::Box,
    table: TmTable,
    data: RefCell<Option<Rc<Value>>>,
    query: RefCell<String>,
    rows: RefCell<HashMap<String, Rc<ProcRow>>>,
    subs: RefCell<Vec<Subscription>>,
    health_sub: RefCell<Option<HealthSub>>,
    pending_pid: Cell<Option<i64>>,
    end_btn: gtk::Button,
    end_wrap: gtk::Box,
    restart_btn: gtk::Button,
    restart_wrap: gtk::Box,
    eff_btn: gtk::Button,
    eff_wrap: gtk::Box,
    prop_btn: gtk::Button,
    commands: Vec<gtk::Widget>,
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let page = Rc::new_cyclic(|weak: &Weak<Details>| build(app, weak.clone()));
    page.wire();
    page
}

fn build(app: &Rc<App>, me: Weak<Details>) -> Details {
    let cols = vec![
        Col::name("name", tr("name"), 16).width(200).required().tip(|v| {
            let d = util::text(&v["displayName"]);
            if d != util::text(&v["name"]) { d } else { String::new() }
        }),
        Col::text("pid", tr("pid"), |v| util::text(&v["pid"])).width(80).right().num("pid"),
        Col::text("status", tr("status"), |v| util::text(&v["stateText"])).width(120),
        Col::text("user", tr("user"), |v| util::text(&v["user"])).width(120),
        Col::text("cpu", tr("cpu"), |v| util::cpu_pct(util::num(&v["cpu"]))).width(76).right().default_desc().num("cpu"),
        Col::text("memory", tr("memory"), |v| util::mem_mb(util::num(&v["memory"]))).width(110).right().default_desc().num("memory"),
        Col::text("threads", tr("threads"), |v| kit::format::number(util::num(&v["threads"]), 0)).width(80).right().default_desc().num("threads"),
        Col::text("desc", tr("desc"), |v| util::text(&v["displayName"])).width(200).hidden(),
        Col::text("cmd", tr("cmd"), |v| util::text(&v["cmd"])).width(360).flex().class("taskmgr-mono").tip(|v| util::text(&v["cmd"])),
    ];
    let table = TmTable::new(
        cols,
        Opts {
            storage_key: "details",
            multi: true,
            tree: false,
            default_sort: Some(("name", false)),
            row_class: Some(Rc::new(|v: &Value| {
                let mut c = Vec::new();
                if v["hung"] == true {
                    c.push("taskmgr-row-hung");
                }
                if v["canKill"] == false {
                    c.push("taskmgr-row-system");
                }
                c
            })),
            rows_class: "taskmgr-rows-30",
            group_counts: false,
        },
    );
    table.view().set_widget_name("taskmgr-details");
    let new_btn = super::new_task_button(app);
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
    let prop_btn = util::cmd_button(Icon::Glyph("properties"), tr("properties"));
    prop_btn.set_widget_name("taskmgr-properties");
    prop_btn.set_sensitive(false);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("taskmgr-details");
    root.set_vexpand(true);
    let tw = table.widget();
    tw.set_vexpand(true);
    root.append(&tw);
    let commands = vec![
        new_btn.upcast(),
        util::cmd_sep(),
        end_wrap.clone().upcast(),
        restart_wrap.clone().upcast(),
        eff_wrap.clone().upcast(),
        prop_btn.clone().upcast(),
    ];
    Details {
        me,
        app: app.clone(),
        root,
        table,
        data: RefCell::new(None),
        query: RefCell::new(String::new()),
        rows: RefCell::new(HashMap::new()),
        subs: RefCell::new(Vec::new()),
        health_sub: RefCell::new(None),
        pending_pid: Cell::new(None),
        end_btn,
        end_wrap,
        restart_btn,
        restart_wrap,
        eff_btn,
        eff_wrap,
        prop_btn,
        commands,
    }
}

impl Details {
    fn wire(&self) {
        let w = self.me.clone();
        self.table.connect_selection_changed(move |_| {
            if let Some(p) = w.upgrade() {
                p.update_commands();
            }
        });
        let w = self.me.clone();
        self.table.connect_activate(move |key| {
            let Some(p) = w.upgrade() else { return };
            let row = p.rows.borrow().get(key).cloned();
            if let Some(r) = row {
                actions::show_properties(&p.app, &r);
            }
        });
        let w = self.me.clone();
        self.table.connect_delete(move |_| {
            let Some(p) = w.upgrade() else { return };
            if let Some(tg) = p.combined()
                && target_of(&tg).can_kill
            {
                actions::end_task(&p.app, &tg);
            }
        });
        let w = self.me.clone();
        self.table.connect_context_menu(move |_| {
            let p = w.upgrade()?;
            let row = p.combined()?;
            util::fix_menu_glyphs(&p.table.view(), actions::menu_glyphs(&row));
            Some(actions::process_menu(&p.app, &row, None))
        });
        let w = self.me.clone();
        self.end_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(tg) = p.combined()
            {
                actions::end_task(&p.app, &tg);
            }
        });
        let w = self.me.clone();
        self.restart_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(tg) = p.combined()
            {
                actions::restart_task(&p.app, &tg);
            }
        });
        let w = self.me.clone();
        self.eff_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(tg) = p.combined()
            {
                actions::set_efficiency(&p.app, &tg, !target_of(&tg).efficiency);
            }
        });
        let w = self.me.clone();
        self.prop_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(r) = p.selected().into_iter().next()
            {
                actions::show_properties(&p.app, &r);
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let w = self.me.clone();
        keys.connect_key_pressed(move |_, key, _, state| {
            let Some(p) = w.upgrade() else { return glib::Propagation::Proceed };
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && state.contains(gdk::ModifierType::ALT_MASK)
                && let Some(r) = p.selected().into_iter().next()
            {
                actions::show_properties(&p.app, &r);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.root.add_controller(keys);
    }

    fn selected(&self) -> Vec<Rc<ProcRow>> {
        let rows = self.rows.borrow();
        self.table.selection().iter().filter_map(|k| rows.get(k).cloned()).collect()
    }

    /// The selection as one action target (details.js combine).
    fn combined(&self) -> Option<Rc<ProcRow>> {
        let sel = self.selected();
        if sel.is_empty() { None } else { Some(combine(&sel)) }
    }

    fn update_commands(&self) {
        let rows = self.selected();
        let combined = if rows.is_empty() { None } else { Some(combine(&rows)) };
        let rt = restart_target_of(combined.as_deref());
        let rt_reason = if rows.is_empty() { tr("selectFirst").to_owned() } else { rt.reason.clone() };
        util::set_button_label(&self.restart_btn, if rt.kind == "shell" { tr("restartShell") } else { tr("restart") });
        self.restart_btn.set_sensitive(rt.ok);
        util::set_tip(&self.restart_wrap, if rt.ok { "" } else { rt_reason.as_str() });
        let Some(c) = combined else {
            self.end_btn.set_sensitive(false);
            self.eff_btn.set_sensitive(false);
            self.prop_btn.set_sensitive(false);
            util::set_tip(&self.end_wrap, tr("selectFirst"));
            util::set_tip(&self.eff_wrap, tr("selectFirst"));
            self.eff_btn.remove_css_class("taskmgr-pressed");
            return;
        };
        let tg = target_of(&c);
        self.end_btn.set_sensitive(tg.can_kill);
        util::set_tip(&self.end_wrap, if tg.can_kill { "" } else { tr("systemProc") });
        self.eff_btn.set_sensitive(tg.can_kill);
        util::set_tip(&self.eff_wrap, if tg.can_kill { "" } else { tr("systemProc") });
        util::set_class(&self.eff_btn, "taskmgr-pressed", tg.efficiency);
        self.prop_btn.set_sensitive(rows.len() == 1);
    }

    fn render(&self) {
        let Some(data) = self.data.borrow().clone() else { return };
        let all = data["processes"].as_array().cloned().unwrap_or_default();
        let by_pid: HashMap<i64, Value> = all.iter().filter_map(|p| pid_of(p).map(|pid| (pid, p.clone()))).collect();
        let mut icon_of: HashMap<i64, String> = HashMap::new();
        let mut app_of: HashMap<i64, Rc<ProcRow>> = HashMap::new();
        let mut hung: HashSet<i64> = HashSet::new();
        for a in data["apps"].as_array().into_iter().flatten() {
            // "Opnieuw starten" on one process of an app restarts the app
            let pids: Vec<i64> = a["pids"].as_array().map(|x| x.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
            let procs: Vec<Value> = pids.iter().filter_map(|p| by_pid.get(p).cloned()).collect();
            let app_row = Rc::new(ProcRow {
                kind: "app",
                name: util::text(&a["name"]),
                icon: util::text(&a["icon"]),
                pids: pids.clone(),
                procs: Rc::new(procs),
                can_kill: a["canKill"] != false,
                status: "running",
                ..Default::default()
            });
            for pid in &pids {
                icon_of.insert(*pid, util::text(&a["icon"]));
                app_of.insert(*pid, app_row.clone());
            }
            if norm_status(a["status"].as_str().unwrap_or("")) == "notResponding"
                && let Some(first) = pids.first()
            {
                hung.insert(*first);
            }
        }
        for field in ["background", "system"] {
            for b in data[field].as_array().into_iter().flatten() {
                let icon = util::text(&b["icon"]);
                if icon.is_empty() {
                    continue;
                }
                for pid in b["pids"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
                    icon_of.insert(pid, icon.clone());
                }
            }
        }
        let mut map = HashMap::new();
        let mut trows = Vec::with_capacity(all.len());
        for p in &all {
            let Some(pid) = pid_of(p) else { continue };
            let is_hung = hung.contains(&pid);
            let row = Rc::new(ProcRow {
                kind: "proc",
                key: format!("d:{pid}"),
                name: util::text(&p["name"]),
                display_name: util::text(&p["displayName"]),
                icon: icon_of.get(&pid).cloned().unwrap_or_default(),
                group: util::text(&p["group"]),
                pid: Some(pid),
                pids: vec![pid],
                procs: Rc::new(vec![p.clone()]),
                cpu: util::num(&p["cpu"]),
                memory: util::num(&p["memory"]),
                threads: util::num(&p["threads"]),
                user: util::text(&p["user"]),
                cmd: util::text(&p["cmd"]),
                exe: util::text(&p["exe"]),
                can_kill: p["canKill"] != false,
                efficiency: p["efficiency"] == true,
                app_row: app_of.get(&pid).cloned(),
                state_text: state_label(p, is_hung).to_owned(),
                status: if is_hung { "notResponding" } else if p["efficiency"] == true { "efficiency" } else { "running" },
                ..Default::default()
            });
            let v = json!({
                "name": row.name,
                "displayName": row.display_name,
                "icon": row.icon,
                "glyph": if row.group == "system" { "settings" } else { "window" },
                "pid": pid,
                "stateText": row.state_text,
                "user": row.user,
                "cpu": row.cpu,
                "memory": row.memory,
                "threads": row.threads,
                "cmd": row.cmd,
                "hung": is_hung,
                "canKill": row.can_kill,
            });
            map.insert(row.key.clone(), row.clone());
            trows.push(TRow::new(row.key.clone(), v));
        }
        *self.rows.borrow_mut() = map;
        self.table.set_rows(trows);
        self.update_commands();
    }

    fn apply_filter(&self) {
        let raw = self.query.borrow().trim().to_owned();
        let q = raw.to_lowercase();
        let empty = if self.data.borrow().is_some() {
            fill(tr("noMatch"), &[("q", &raw)])
        } else if crate::hostcall::health().is_ok() {
            tr("loading").to_owned()
        } else {
            tr("waitingHost").to_owned()
        };
        self.table.set_empty_text(&empty);
        if q.is_empty() {
            self.table.set_filter(None);
            return;
        }
        let is_num = q.chars().all(|c| c.is_ascii_digit());
        self.table.set_filter(Some(Rc::new(move |v: &Value| {
            (is_num && util::text(&v["pid"]).starts_with(&q))
                || util::text(&v["name"]).to_lowercase().contains(&q)
                || util::text(&v["displayName"]).to_lowercase().contains(&q)
                || util::text(&v["cmd"]).to_lowercase().contains(&q)
        })));
    }
}

fn param_pid(params: &Value) -> Option<i64> {
    params["pid"].as_i64().or_else(|| params["pid"].as_str().and_then(|s| s.trim().parse().ok()))
}

impl Page for Details {
    fn id(&self) -> &'static str {
        "details"
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
        self.pending_pid.set(param_pid(params));
        let w = self.me.clone();
        let sub = self.app.store.subscribe("processes", move |d| {
            let Some(p) = w.upgrade() else { return };
            *p.data.borrow_mut() = Some(d.clone());
            p.render();
            p.apply_filter();
            if let Some(pid) = p.pending_pid.get()
                && p.table.select(&format!("d:{pid}"), true)
            {
                p.pending_pid.set(None);
            }
        });
        self.subs.borrow_mut().push(sub);
        let w = self.me.clone();
        *self.health_sub.borrow_mut() = Some(self.app.on_health(move |_, _| {
            if let Some(p) = w.upgrade()
                && p.data.borrow().is_none()
            {
                p.apply_filter();
            }
        }));
        if self.data.borrow().is_none() {
            self.apply_filter();
        }
    }

    fn navigate(&self, params: &Value) {
        if let Some(pid) = param_pid(params) {
            self.table.select(&format!("d:{pid}"), true);
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
