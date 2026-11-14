//! Services (js/apps/taskmgr/pages/services.js): the user's own systemd services (systemctl --user) with a status
//! pill and start / stop / restart, always confirmed with a plain explanation.
use super::Page;
use crate::app::App;
use crate::hostcall;
use crate::store::Subscription;
use crate::table::{Col, Opts, SortVal, TRow, TmTable, cell_row_id};
use crate::util::{self, Icon, TmGlyph, fill};
use kit::controls::{Dialog, DialogResult, MenuItem};
use kit::prelude::*;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::{Rc, Weak};

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Services", "Services"),
        "start" => ("Starten", "Start"),
        "stop" => ("Stoppen", "Stop"),
        "restart" => ("Opnieuw starten", "Restart"),
        "refresh" => ("Vernieuwen", "Refresh"),
        "name" => ("Naam", "Name"),
        "description" => ("Beschrijving", "Description"),
        "status" => ("Status", "Status"),
        "actions" => ("Acties", "Actions"),
        "active" => ("Actief", "Running"),
        "inactive" => ("Gestopt", "Stopped"),
        "failed" => ("Mislukt", "Failed"),
        "activating" => ("Bezig met starten", "Starting"),
        "deactivating" => ("Bezig met stoppen", "Stopping"),
        "unknown" => ("Onbekend", "Unknown"),
        "intro" => ("Achtergrondservices van je eigen account", "Background services of your own account"),
        "introText" => (
            "Deze kleine programma\u{2019}s werken op de achtergrond voor je apps. Systeemservices staan hier niet, zodat je niets belangrijks per ongeluk stopt.",
            "These small programs work in the background for your apps. System services aren\u{2019}t listed, so you can\u{2019}t stop anything important by accident.",
        ),
        "confirmStop" => ("\u{2018}{name}\u{2019} stoppen?", "Stop \u{2018}{name}\u{2019}?"),
        "confirmRestart" => ("\u{2018}{name}\u{2019} opnieuw starten?", "Restart \u{2018}{name}\u{2019}?"),
        "confirmStart" => ("\u{2018}{name}\u{2019} starten?", "Start \u{2018}{name}\u{2019}?"),
        "stopBody" => (
            "Apps die deze service gebruiken, werken misschien niet goed tot je de service weer start.",
            "Apps that use this service may not work properly until you start it again.",
        ),
        "protectedTip" => ("Nodig voor je bureaublad \u{2013} kan niet worden gestopt", "Needed by your desktop \u{2013} can\u{2019}t be stopped"),
        "restartBody" => (
            "De service stopt heel even en start dan opnieuw. Dat helpt vaak als iets is blijven hangen.",
            "The service stops briefly and starts again. This often helps when something is stuck.",
        ),
        "startBody" => (
            "De service wordt gestart en blijft actief tot je je afmeldt of hem stopt.",
            "The service starts and keeps running until you sign out or stop it.",
        ),
        "done_start" => ("{name} is gestart", "{name} has started"),
        "done_stop" => ("{name} is gestopt", "{name} has stopped"),
        "done_restart" => ("{name} is opnieuw gestart", "{name} has restarted"),
        "failedAction" => ("Dat is niet gelukt", "That didn\u{2019}t work"),
        "loading" => ("Services laden\u{2026}", "Loading services\u{2026}"),
        "none" => ("Geen services gevonden.", "No services found."),
        "counts" => ("{a} actief \u{b7} {i} gestopt", "{a} running \u{b7} {i} stopped"),
        "countsFailed" => ("{a} actief \u{b7} {i} gestopt \u{b7} {f} mislukt", "{a} running \u{b7} {i} stopped \u{b7} {f} failed"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

fn short(name: &str) -> String {
    name.strip_suffix(".service").unwrap_or(name).to_owned()
}

fn shown(s: &Value) -> String {
    let d = util::text(&s["displayName"]);
    if d.is_empty() { short(&util::text(&s["name"])) } else { d }
}

fn state_of(s: &Value) -> &'static str {
    match s["state"].as_str().unwrap_or("").to_lowercase().as_str() {
        "active" => "active",
        "failed" => "failed",
        "activating" | "reloading" => "activating",
        "deactivating" => "deactivating",
        "inactive" => "inactive",
        _ => "unknown",
    }
}

const STATE_ORDER: [&str; 6] = ["failed", "activating", "active", "deactivating", "inactive", "unknown"];

struct Services {
    me: Weak<Services>,
    app: Rc<App>,
    root: gtk::Box,
    table: TmTable,
    items: RefCell<Vec<Value>>,
    query: RefCell<String>,
    busy: RefCell<HashSet<String>>,
    subs: RefCell<Vec<Subscription>>,
    counts: gtk::Label,
    start_btn: gtk::Button,
    stop_btn: gtk::Button,
    restart_btn: gtk::Button,
    commands: Vec<gtk::Widget>,
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let page = Rc::new_cyclic(|weak: &Weak<Services>| build(app, weak.clone()));
    page.wire();
    page
}

fn icon_btn(icon: Icon, tip: &str, name: &str) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-icon-button");
    b.add_css_class("taskmgr-action");
    if let Some(i) = icon.image(14) {
        b.set_child(Some(&i));
    }
    b.set_size_request(28, 28);
    b.set_valign(gtk::Align::Center);
    b.set_tooltip_text(Some(tip));
    b.set_widget_name(name);
    b
}

fn status_setup() -> gtk::Widget {
    let pill = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    pill.add_css_class("taskmgr-pill");
    pill.add_css_class("taskmgr-state");
    pill.set_halign(gtk::Align::Start);
    pill.set_valign(gtk::Align::Center);
    let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    dot.add_css_class("taskmgr-state-dot");
    dot.set_size_request(6, 6);
    dot.set_valign(gtk::Align::Center);
    pill.append(&dot);
    pill.append(&gtk::Label::new(None));
    pill.upcast()
}

fn status_bind(w: &gtk::Widget, v: &Value) {
    if v["__group"] == true {
        w.set_visible(false);
        return;
    }
    w.set_visible(true);
    let st = v["shownState"].as_str().unwrap_or("unknown");
    for s in STATE_ORDER {
        util::set_class(w, &format!("taskmgr-is-{s}"), s == st);
    }
    if let Some(l) = w.last_child().and_downcast::<gtk::Label>() {
        l.set_label(tr(st));
    }
}

fn build(app: &Rc<App>, me: Weak<Services>) -> Services {
    let w_actions = me.clone();
    let mut name = Col::name("name", tr("name"), 16).width(260).required().tip(|v| {
        let unit = util::text(&v["unit"]);
        if v["critical"] == true { format!("{unit} \u{b7} {}", tr("protectedTip")) } else { unit }
    });
    name.value = Rc::new(|v| SortVal::Text(util::text(&v["name"]).to_lowercase()));
    let cols = vec![
        name,
        Col::text("description", tr("description"), |v| util::text(&v["description"])).width(300).flex(),
        Col::custom("status", tr("status"), status_setup, status_bind)
            .width(150)
            .value(|v| SortVal::Num(STATE_ORDER.iter().position(|s| *s == v["state"].as_str().unwrap_or("unknown")).unwrap_or(5) as f64)),
        Col::custom(
            "actions",
            tr("actions"),
            move || {
                let b = gtk::Box::new(gtk::Orientation::Horizontal, 2);
                b.add_css_class("taskmgr-actions");
                for (icon, action) in [(Icon::Glyph("play"), "start"), (Icon::Tm(TmGlyph::Stop), "stop"), (Icon::Glyph("restart"), "restart")] {
                    let btn = icon_btn(icon, tr(action), &format!("taskmgr-service-{action}"));
                    let w = w_actions.clone();
                    btn.connect_clicked(move |b| {
                        let (Some(p), Some(key)) = (w.upgrade(), cell_row_id(b)) else { return };
                        let item = p.items.borrow().iter().find(|s| util::text(&s["name"]) == key).cloned();
                        if let Some(s) = item {
                            p.act(&s, action);
                        }
                    });
                    b.append(&btn);
                }
                b.upcast()
            },
            |w, v| {
                if v["__group"] == true {
                    w.set_visible(false);
                    return;
                }
                w.set_visible(true);
                let st = v["state"].as_str().unwrap_or("unknown");
                let busy = v["busy"] == true;
                let critical = v["critical"] == true;
                let mut c = w.first_child();
                let mut i = 0;
                while let Some(b) = c {
                    let on = match i {
                        0 => !(busy || st == "active" || st == "activating"),
                        1 => !(busy || st == "inactive" || st == "failed" || critical),
                        _ => !(busy || st != "active" || critical),
                    };
                    b.set_sensitive(on);
                    if i == 1 {
                        b.set_tooltip_text(Some(if critical { tr("protectedTip") } else { tr("stop") }));
                    }
                    i += 1;
                    c = b.next_sibling();
                }
            },
        )
        .width(132)
        .unsortable(),
    ];
    let table = TmTable::new(
        cols,
        Opts { storage_key: "services", multi: false, tree: false, default_sort: Some(("name", false)), row_class: None, rows_class: "taskmgr-rows-40", group_counts: false },
    );
    table.view().set_widget_name("taskmgr-services");
    table.set_empty_text(tr("loading"));

    let counts = util::label("", "taskmgr-intro-meta");
    let intro = intro_card("services", tr("intro"), tr("introText"), Some(&counts));
    let pad = gtk::Box::new(gtk::Orientation::Vertical, 0);
    pad.add_css_class("taskmgr-page-pad");
    pad.append(&intro);
    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("taskmgr-services");
    root.set_vexpand(true);
    root.append(&pad);
    let tw = table.widget();
    tw.set_vexpand(true);
    root.append(&tw);

    let start_btn = util::cmd_button(Icon::Glyph("play"), tr("start"));
    start_btn.set_widget_name("taskmgr-start");
    let stop_btn = util::cmd_button(Icon::Tm(TmGlyph::Stop), tr("stop"));
    stop_btn.set_widget_name("taskmgr-stop");
    let restart_btn = util::cmd_button(Icon::Glyph("restart"), tr("restart"));
    restart_btn.set_widget_name("taskmgr-restart");
    for b in [&start_btn, &stop_btn, &restart_btn] {
        b.set_sensitive(false);
    }
    let refresh_btn = util::cmd_button(Icon::Glyph("refresh"), tr("refresh"));
    refresh_btn.set_widget_name("taskmgr-refresh");
    let a = Rc::downgrade(app);
    refresh_btn.connect_clicked(move |_| {
        if let Some(a) = a.upgrade() {
            a.store.refresh("services");
        }
    });
    let commands = vec![start_btn.clone().upcast(), stop_btn.clone().upcast(), restart_btn.clone().upcast(), util::cmd_sep(), refresh_btn.upcast()];
    Services {
        me,
        app: app.clone(),
        root,
        table,
        items: RefCell::new(Vec::new()),
        query: RefCell::new(String::new()),
        busy: RefCell::new(HashSet::new()),
        subs: RefCell::new(Vec::new()),
        counts,
        start_btn,
        stop_btn,
        restart_btn,
        commands,
    }
}

/// The intro card of Services and Opstart-apps (`.tm-intro`): a glyph on an accent disc, a title, a text and
/// optionally a meta text on the right.
pub fn intro_card(glyph: &str, title: &str, text: &str, meta: Option<&gtk::Label>) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    card.add_css_class("taskmgr-intro");
    let disc = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    disc.add_css_class("taskmgr-intro-icon");
    disc.set_size_request(36, 36);
    disc.set_valign(gtk::Align::Center);
    disc.set_hexpand(false);
    let g = kit::icons::glyph(glyph, 20);
    g.set_halign(gtk::Align::Center);
    g.set_valign(gtk::Align::Center);
    g.set_hexpand(true);
    disc.append(&g);
    card.append(&disc);
    let texts = gtk::Box::new(gtk::Orientation::Vertical, 0);
    texts.set_hexpand(true);
    texts.set_valign(gtk::Align::Center);
    let t = util::wrap_label(title, "taskmgr-intro-title");
    t.set_widget_name("taskmgr-intro-title");
    texts.append(&t);
    texts.append(&util::wrap_label(text, "taskmgr-intro-text"));
    card.append(&texts);
    if let Some(m) = meta {
        m.set_valign(gtk::Align::Center);
        m.set_ellipsize(gtk::pango::EllipsizeMode::None);
        card.append(m);
    }
    card
}

impl Services {
    fn wire(&self) {
        let w = self.me.clone();
        self.table.connect_selection_changed(move |_| {
            if let Some(p) = w.upgrade() {
                p.update_commands();
            }
        });
        let w = self.me.clone();
        self.table.connect_context_menu(move |keys| {
            let p = w.upgrade()?;
            let s = p.item(keys.first()?)?;
            let st = state_of(&s);
            let critical = s["critical"] == true;
            let mut items = Vec::new();
            let (p1, s1) = (p.clone(), s.clone());
            items.push(MenuItem::new(tr("start"), move || p1.act(&s1, "start")).glyph("play").enabled(st != "active").name("taskmgr-menu-start"));
            let (p2, s2) = (p.clone(), s.clone());
            let mut stop = MenuItem::new(tr("stop"), move || p2.act(&s2, "stop"))
                .glyph("process")
                .enabled(!(st == "inactive" || st == "failed" || critical))
                .name("taskmgr-menu-stop");
            if critical {
                stop = stop.description(tr("protectedTip"));
            }
            items.push(stop);
            let (p3, s3) = (p.clone(), s.clone());
            items.push(MenuItem::new(tr("restart"), move || p3.act(&s3, "restart")).glyph("restart").enabled(st == "active" && !critical).name("taskmgr-menu-restart"));
            util::fix_menu_glyphs(&p.table.view(), vec![("taskmgr-menu-stop", TmGlyph::Stop)]);
            Some((Vec::new(), items))
        });
        for (btn, action) in [(&self.start_btn, "start"), (&self.stop_btn, "stop"), (&self.restart_btn, "restart")] {
            let w = self.me.clone();
            btn.connect_clicked(move |_| {
                let Some(p) = w.upgrade() else { return };
                let s = p.table.selection().first().and_then(|k| p.item(k));
                if let Some(s) = s {
                    p.act(&s, action);
                }
            });
        }
    }

    fn item(&self, key: &str) -> Option<Value> {
        self.items.borrow().iter().find(|s| util::text(&s["name"]) == key).cloned()
    }

    fn act(self: &Rc<Self>, s: &Value, action: &'static str) {
        let name = shown(s);
        let title = fill(tr(match action {
            "stop" => "confirmStop",
            "restart" => "confirmRestart",
            _ => "confirmStart",
        }), &[("name", &name)]);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
        body.add_css_class("taskmgr-dlg");
        let desc = util::text(&s["description"]);
        if !desc.is_empty() {
            let l = util::wrap_label(&desc, "taskmgr-dlg-hint");
            l.set_max_width_chars(52);
            body.append(&l);
        }
        let l = util::wrap_label(tr(match action {
            "stop" => "stopBody",
            "restart" => "restartBody",
            _ => "startBody",
        }), "w11-dialog-text");
        l.set_max_width_chars(52);
        body.append(&l);
        let mut d = Dialog::new(&title).body(&body).primary(tr(action));
        if action == "stop" {
            d = d.danger();
        }
        let me = self.clone();
        let unit = util::text(&s["name"]);
        d.show(&self.app.parent(), move |r| {
            if r != DialogResult::Primary {
                return;
            }
            me.busy.borrow_mut().insert(unit.clone());
            me.render();
            let m = me.clone();
            hostcall::call("sysmon.serviceAction", json!({ "name": unit, "action": action }), move |r| {
                match r {
                    Ok(_) => m.app.toast_success(&fill(tr(&format!("done_{action}")), &[("name", &name)]), 3000),
                    Err(e) => m.app.toast_error(tr("failedAction"), &e.message),
                }
                m.busy.borrow_mut().remove(&unit);
                let m2 = m.clone();
                m.app.store.refresh_then("services", move |_| m2.render());
            });
        });
    }

    fn update_commands(&self) {
        let s = self.table.selection().first().and_then(|k| self.item(k));
        let st = s.as_ref().map(state_of);
        let critical = s.as_ref().is_some_and(|s| s["critical"] == true);
        self.start_btn.set_sensitive(s.is_some() && !matches!(st, Some("active") | Some("activating")));
        self.stop_btn.set_sensitive(s.is_some() && !matches!(st, Some("inactive") | Some("failed")) && !critical);
        self.restart_btn.set_sensitive(s.is_some() && st == Some("active") && !critical);
    }

    fn render(&self) {
        let q = self.query.borrow().trim().to_lowercase();
        if q.is_empty() {
            self.table.set_filter(None);
        } else {
            self.table.set_filter(Some(Rc::new(move |v: &Value| {
                util::text(&v["unit"]).to_lowercase().contains(&q)
                    || util::text(&v["name"]).to_lowercase().contains(&q)
                    || util::text(&v["description"]).to_lowercase().contains(&q)
            })));
        }
        let items = self.items.borrow().clone();
        let busy = self.busy.borrow().clone();
        let rows: Vec<TRow> = items
            .iter()
            .map(|s| {
                let unit = util::text(&s["name"]);
                let b = busy.contains(&unit);
                TRow::new(
                    unit.clone(),
                    json!({
                        "name": shown(s),
                        "unit": unit,
                        "icon": "",
                        "glyph": "services",
                        "description": util::text(&s["description"]),
                        "state": state_of(s),
                        "shownState": if b { "activating" } else { state_of(s) },
                        "busy": b,
                        "critical": s["critical"] == true,
                    }),
                )
            })
            .collect();
        self.table.set_rows(rows);
        let a = items.iter().filter(|s| state_of(s) == "active").count();
        let f = items.iter().filter(|s| state_of(s) == "failed").count();
        let i = items.len() - a - f;
        self.counts.set_label(&if f > 0 {
            fill(tr("countsFailed"), &[("a", &a.to_string()), ("i", &i.to_string()), ("f", &f.to_string())])
        } else {
            fill(tr("counts"), &[("a", &a.to_string()), ("i", &(items.len() - a).to_string())])
        });
        self.table.set_empty_text(if items.is_empty() { tr("none") } else { "" });
        self.update_commands();
    }
}

impl Page for Services {
    fn id(&self) -> &'static str {
        "services"
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
        let sub = self.app.store.subscribe("services", move |d| {
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
        self.app.store.refresh("services");
        true
    }
}
