//! Opstart-apps (js/apps/taskmgr/pages/startup.js): which apps start when you sign in, with a switch per row.
use super::Page;
use super::services::intro_card;
use crate::app::App;
use crate::hostcall;
use crate::store::Subscription;
use crate::table::{Col, Opts, SortVal, TRow, TmTable, cell_row_id};
use crate::util::{self, Icon, fill, plural};
use kit::controls::{MenuItem, ToggleSwitch};
use kit::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Opstart-apps", "Startup apps"),
        "enable" => ("Inschakelen", "Enable"),
        "disable" => ("Uitschakelen", "Disable"),
        "name" => ("Naam", "Name"),
        "source" => ("Bron", "Source"),
        "status" => ("Status", "Status"),
        "impact" => ("Invloed bij opstarten", "Startup impact"),
        "command" => ("Opdracht", "Command"),
        "user" => ("Persoonlijk", "Personal"),
        "system" => ("Systeem", "System"),
        "on" => ("Ingeschakeld", "Enabled"),
        "off" => ("Uitgeschakeld", "Disabled"),
        "notMeasured" => ("Niet gemeten", "Not measured"),
        "impact_none" => ("Geen", "None"),
        "impact_low" => ("Laag", "Low"),
        "impact_medium" => ("Gemiddeld", "Medium"),
        "impact_high" => ("Hoog", "High"),
        "summaryOne" => ("1 van {m} apps start automatisch als je je aanmeldt.", "1 of {m} apps starts automatically when you sign in."),
        "summaryOther" => ("{n} van {m} apps starten automatisch als je je aanmeldt.", "{n} of {m} apps start automatically when you sign in."),
        "tip" => ("Minder opstart-apps betekent dat je pc sneller klaar is na het aanmelden.", "Fewer startup apps means your PC is ready sooner after signing in."),
        "failed" => ("Kon dit niet wijzigen", "Couldn\u{2019}t change this"),
        "loading" => ("Opstart-apps laden\u{2026}", "Loading startup apps\u{2026}"),
        "none" => ("Er zijn geen opstart-apps.", "There are no startup apps."),
        "enabledToast" => ("{name} start voortaan automatisch", "{name} will start automatically"),
        "disabledToast" => ("{name} start niet meer automatisch", "{name} will no longer start automatically"),
        "sourceTipUser" => ("Door jou of een app ingesteld (~/.config/autostart)", "Set by you or an app (~/.config/autostart)"),
        "sourceTipSystem" => ("Ingesteld door Ubuntu; uitzetten geldt alleen voor jou", "Set by Ubuntu; turning it off only affects you"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

fn impact_order(i: &str) -> Option<f64> {
    match i {
        "none" => Some(0.0),
        "low" => Some(1.0),
        "medium" => Some(2.0),
        "high" => Some(3.0),
        _ => None,
    }
}

struct Startup {
    me: Weak<Startup>,
    app: Rc<App>,
    root: gtk::Box,
    table: TmTable,
    items: RefCell<Vec<Value>>,
    query: RefCell<String>,
    pending: RefCell<HashSet<String>>,
    subs: RefCell<Vec<Subscription>>,
    summary: gtk::Label,
    enable_btn: gtk::Button,
    disable_btn: gtk::Button,
    commands: Vec<gtk::Widget>,
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let page = Rc::new_cyclic(|weak: &Weak<Startup>| build(app, weak.clone()));
    page.wire();
    page
}

fn build(app: &Rc<App>, me: Weak<Startup>) -> Startup {
    let w_switch = me.clone();
    let cols = vec![
        Col::name("name", tr("name"), 24).width(260).flex().required().tip(|v| util::text(&v["description"])),
        Col::text("source", tr("source"), |v| tr(if v["source"] == "system" { "system" } else { "user" }).to_owned())
            .width(130)
            .value(|v| SortVal::Text(util::text(&v["source"])))
            .tip(|v| tr(if v["source"] == "system" { "sourceTipSystem" } else { "sourceTipUser" }).to_owned()),
        Col::custom(
            "status",
            tr("status"),
            move || {
                let sw = ToggleSwitch::new(false);
                sw.set_state_texts(tr("on"), tr("off"));
                let root = sw.widget().clone();
                root.add_css_class("taskmgr-startup-toggle");
                // the switch first, then its text (startup.js: row-reverse)
                if let (Some(state), Some(switch)) = (root.first_child(), root.last_child())
                    && state != switch
                {
                    root.reorder_child_after(&state, Some(&switch));
                }
                sw.switch().set_widget_name("taskmgr-startup-switch");
                let w = w_switch.clone();
                let rw = root.downgrade();
                sw.connect_toggled(move |on| {
                    let (Some(p), Some(r)) = (w.upgrade(), rw.upgrade()) else { return };
                    if let Some(id) = cell_row_id(&r) {
                        p.set_enabled(&id, on);
                    }
                });
                root.upcast()
            },
            |w, v| {
                if v["__group"] == true {
                    return;
                }
                let Some(b) = w.downcast_ref::<gtk::Box>() else { return };
                // the ToggleSwitch is owned by its box; reach the gtk::Switch and its text directly
                let mut c = b.first_child();
                while let Some(child) = c {
                    if let Some(sw) = child.downcast_ref::<gtk::Switch>() {
                        let on = v["enabled"] == true;
                        if sw.is_active() != on {
                            // silent: the box's handler only acts on a change the row does not have yet
                            sw.set_active(on);
                        }
                        sw.set_sensitive(v["pending"] != true);
                    } else if let Some(l) = child.downcast_ref::<gtk::Label>() {
                        l.set_label(tr(if v["enabled"] == true { "on" } else { "off" }));
                        l.set_sensitive(v["pending"] != true);
                    }
                    c = child.next_sibling();
                }
            },
        )
        .width(190)
        .value(|v| SortVal::Num(if v["enabled"] == true { 0.0 } else { 1.0 })),
        // Ubuntu does not measure how much an app slows down signing in: hidden by default, a dash with a tooltip
        Col::text("impact", tr("impact"), |v| match impact_order(v["impact"].as_str().unwrap_or("")) {
            Some(_) => tr(&format!("impact_{}", v["impact"].as_str().unwrap_or(""))).to_owned(),
            None => "\u{2014}".to_owned(),
        })
        .width(170)
        .hidden()
        .class("taskmgr-muted")
        .value(|v| SortVal::Num(impact_order(v["impact"].as_str().unwrap_or("")).unwrap_or(-1.0)))
        .tip(|v| if impact_order(v["impact"].as_str().unwrap_or("")).is_some() { String::new() } else { tr("notMeasured").to_owned() }),
        Col::text("command", tr("command"), |v| util::text(&v["command"])).width(360).hidden().class("taskmgr-mono").tip(|v| util::text(&v["command"])),
    ];
    let table = TmTable::new(
        cols,
        Opts { storage_key: "startup", multi: false, tree: false, default_sort: Some(("name", false)), row_class: None, rows_class: "taskmgr-rows-44", group_counts: false },
    );
    table.view().set_widget_name("taskmgr-startup");
    table.set_empty_text(tr("loading"));

    let intro = intro_card("startup", "", tr("tip"), None);
    let summary = util::find_named(intro.upcast_ref(), "taskmgr-intro-title").and_downcast::<gtk::Label>().unwrap_or_else(|| gtk::Label::new(None));
    let pad = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pad.add_css_class("taskmgr-page-pad");
    pad.append(&intro);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("taskmgr-startup");
    root.set_vexpand(true);
    root.append(&pad);
    let tw = table.widget();
    tw.set_vexpand(true);
    root.append(&tw);

    let new_btn = super::new_task_button(app);
    let enable_btn = util::cmd_button(Icon::Glyph("check"), tr("enable"));
    enable_btn.set_widget_name("taskmgr-enable");
    enable_btn.set_sensitive(false);
    let disable_btn = util::cmd_button(Icon::Glyph("close"), tr("disable"));
    disable_btn.set_widget_name("taskmgr-disable");
    disable_btn.set_sensitive(false);
    let commands = vec![new_btn.upcast(), util::cmd_sep(), enable_btn.clone().upcast(), disable_btn.clone().upcast()];
    Startup {
        me,
        app: app.clone(),
        root,
        table,
        items: RefCell::new(Vec::new()),
        query: RefCell::new(String::new()),
        pending: RefCell::new(HashSet::new()),
        subs: RefCell::new(Vec::new()),
        summary,
        enable_btn,
        disable_btn,
        commands,
    }
}

impl Startup {
    fn wire(&self) {
        let w = self.me.clone();
        self.table.connect_selection_changed(move |_| {
            if let Some(p) = w.upgrade() {
                p.update_commands();
            }
        });
        let w = self.me.clone();
        self.table.connect_activate(move |id| {
            let Some(p) = w.upgrade() else { return };
            let on = p.item(id).is_some_and(|i| i["enabled"] == true);
            p.set_enabled(id, !on);
        });
        let w = self.me.clone();
        self.table.connect_context_menu(move |keys| {
            let p = w.upgrade()?;
            let id = keys.first()?.clone();
            let on = p.item(&id)?["enabled"] == true;
            let p2 = p.clone();
            Some((
                Vec::new(),
                vec![MenuItem::new(tr(if on { "disable" } else { "enable" }), move || p2.set_enabled(&id, !on)).glyph(if on { "close" } else { "check" }).name("taskmgr-menu-toggle")],
            ))
        });
        let w = self.me.clone();
        self.enable_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(id) = p.table.selection().first()
            {
                p.set_enabled(id, true);
            }
        });
        let w = self.me.clone();
        self.disable_btn.connect_clicked(move |_| {
            if let Some(p) = w.upgrade()
                && let Some(id) = p.table.selection().first()
            {
                p.set_enabled(id, false);
            }
        });
    }

    fn item(&self, id: &str) -> Option<Value> {
        self.items.borrow().iter().find(|i| util::text(&i["id"]) == id).cloned()
    }

    fn set_flag(&self, id: &str, on: bool) {
        for it in self.items.borrow_mut().iter_mut() {
            if util::text(&it["id"]) == id {
                it["enabled"] = json!(on);
            }
        }
    }

    fn set_enabled(&self, id: &str, on: bool) {
        let Some(it) = self.item(id) else { return };
        let prev = it["enabled"] == true;
        if self.pending.borrow().contains(id) || prev == on {
            return;
        }
        self.pending.borrow_mut().insert(id.to_owned());
        self.set_flag(id, on);
        self.render();
        let name = util::text(&it["name"]);
        let w = self.me.clone();
        let id = id.to_owned();
        hostcall::call("sysmon.setStartup", json!({ "id": it["id"], "enabled": on }), move |r| {
            let Some(p) = w.upgrade() else { return };
            match r {
                Ok(_) => p.app.toast_success(&fill(tr(if on { "enabledToast" } else { "disabledToast" }), &[("name", &name)]), 2500),
                Err(e) => {
                    p.set_flag(&id, prev);
                    p.app.toast_error(tr("failed"), &e.message);
                }
            }
            p.pending.borrow_mut().remove(&id);
            p.render();
        });
    }

    fn update_commands(&self) {
        let it = self.table.selection().first().and_then(|k| self.item(k));
        self.enable_btn.set_sensitive(it.as_ref().is_some_and(|i| i["enabled"] != true));
        self.disable_btn.set_sensitive(it.as_ref().is_some_and(|i| i["enabled"] == true));
    }

    fn render(&self) {
        let q = self.query.borrow().trim().to_lowercase();
        if q.is_empty() {
            self.table.set_filter(None);
        } else {
            self.table.set_filter(Some(Rc::new(move |v: &Value| {
                util::text(&v["name"]).to_lowercase().contains(&q) || util::text(&v["command"]).to_lowercase().contains(&q)
            })));
        }
        let items = self.items.borrow().clone();
        let pending = self.pending.borrow().clone();
        let rows: Vec<TRow> = items
            .iter()
            .map(|it| {
                let id = util::text(&it["id"]);
                TRow::new(
                    id.clone(),
                    json!({
                        "id": id,
                        "name": util::text(&it["name"]),
                        "icon": util::text(&it["icon"]),
                        "glyph": "startup",
                        "description": util::text(&it["description"]),
                        "source": util::text(&it["source"]),
                        "enabled": it["enabled"] == true,
                        "pending": pending.contains(&id),
                        "impact": it["impact"],
                        "command": util::text(&it["command"]),
                    }),
                )
            })
            .collect();
        self.table.set_rows(rows);
        let n = items.iter().filter(|i| i["enabled"] == true).count() as i64;
        self.summary.set_label(&fill(&plural(n, tr("summaryOne"), tr("summaryOther")), &[("m", &items.len().to_string())]));
        self.table.set_empty_text(if items.is_empty() { tr("none") } else { "" });
        self.update_commands();
    }
}

impl Page for Startup {
    fn id(&self) -> &'static str {
        "startup"
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

    fn mount(&self, _params: &Value) {
        let w = self.me.clone();
        let sub = self.app.store.subscribe("startup", move |d| {
            let Some(p) = w.upgrade() else { return };
            *p.items.borrow_mut() = d.as_array().cloned().unwrap_or_default();
            p.render();
        });
        self.subs.borrow_mut().push(sub);
    }

    fn unmount(&self) {
        self.subs.borrow_mut().clear();
    }

    fn searchable(&self) -> bool {
        true
    }

    fn on_search(&self, query: &str) {
        *self.query.borrow_mut() = query.to_owned();
        self.render();
    }

    fn refresh(&self) -> bool {
        self.app.store.refresh("startup");
        true
    }
}
