//! Instellingen › Apps › Opstarten (6ebf169f:vendor/ui/js/apps/settings/pages/startup.js): the apps that start when
//! you sign in (sysmon.startup), each with a switch (sysmon.setStartup), sorted by name or status, and the link to
//! Taakbeheer's Opstarten tab.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Chevron, Dropdown, SettingsCard, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("startup", startup)];
pub const CSS: &str = "";

struct State {
    p: Page,
    items: RefCell<Vec<Value>>,
    sort: RefCell<String>,
    list: gtk::Box,
}

fn paint(st: &Rc<State>) {
    c::clear(&st.list);
    let items = st.items.borrow().clone();
    if items.is_empty() {
        st.list.append(&c::empty_state(
            "startup",
            t("Geen opstart-apps", "No startup apps"),
            t("Er starten geen apps automatisch als je je aanmeldt.", "No apps start automatically when you sign in."),
            None,
        ));
        return;
    }
    let by_state = st.sort.borrow().as_str() == "state";
    let mut sorted = items;
    sorted.sort_by(|a, b| {
        let state = if by_state { (b["enabled"] == true).cmp(&(a["enabled"] == true)) } else { std::cmp::Ordering::Equal };
        state.then_with(|| kit::format::compare_natural(common::s(&a["name"]), common::s(&b["name"])))
    });
    for it in &sorted {
        let id = it["id"].clone();
        let name = common::s(&it["name"]).to_owned();
        let sw = ToggleSwitch::new(it["enabled"] == true);
        sw.switch().update_property(&[gtk::accessible::Property::Label(&name)]);
        sw.switch().set_widget_name(&format!("startup-{}", common::value_key(&id)));
        let weak = Rc::downgrade(st);
        let sw2 = sw.clone();
        sw.connect_toggled(move |on| {
            let Some(st) = weak.upgrade() else { return };
            let fut = st.p.run("sysmon.setStartup", json!({ "id": id, "enabled": on }));
            let (sw, id) = (sw2.clone(), id.clone());
            st.p.clone().spawn(async move {
                if fut.await.is_none() {
                    sw.set_active(!on);
                } else if let Some(item) = st.items.borrow_mut().iter_mut().find(|x| x["id"] == id) {
                    item["enabled"] = json!(on);
                }
            });
        });
        let icon = common::s(&it["icon"]);
        let icon = if icon.is_empty() { common::s(&it["id"]) } else { icon };
        let desc = if it["source"] == "system" {
            t("Ingesteld voor iedereen op deze pc", "Set up for everyone on this PC")
        } else {
            t("Alleen voor jou ingesteld", "Set up just for you")
        };
        let card = SettingsCard::new("", &name, desc);
        card.set_icon_widget(&kit::icons::app_icon(icon, 32));
        card.set_control(sw.widget());
        card.widget().add_css_class("st-app-row");
        if let Some(cmd) = it["command"].as_str().filter(|s| !s.is_empty()) {
            card.widget().set_tooltip_text(Some(cmd));
        }
        st.list.append(card.widget());
    }
}

fn startup(p: Page) -> PageFuture {
    Box::pin(async move {
        let r = p.call("sysmon.startup", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let Some(items) = r.ok().and_then(|v| v.as_array().cloned()) else {
            return vec![common::notice(t("De lijst met opstart-apps is niet beschikbaar.", "The list of startup apps isn’t available.")).widget().clone().upcast()];
        };
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.add_css_class("st-app-list");
        let st = Rc::new(State { p: p.clone(), items: RefCell::new(items), sort: RefCell::new("name".into()), list });
        paint(&st);

        let sort = Dropdown::simple(&[("name", t("Naam", "Name")), ("state", t("Status", "Status"))], Some("name"));
        sort.widget().set_widget_name("startup-sort");
        let weak = Rc::downgrade(&st);
        sort.connect_changed(move |v| {
            if let Some(st) = weak.upgrade() {
                *st.sort.borrow_mut() = v.to_owned();
                paint(&st);
            }
        });
        let toolbar = crate::widgets::Wrap::new(20, 12);
        toolbar.add_css_class("st-toolbar");
        toolbar.append(&super::apps::toolbar_field(t("Sorteren op", "Sort by"), sort.widget()));

        let lead = c::label(
            t(
                "Apps kunnen automatisch starten als je je aanmeldt. Zet apps uit die je niet meteen nodig hebt: je computer is dan sneller klaar.",
                "Apps can start automatically when you sign in. Turn off apps you don’t need right away: your computer will be ready sooner.",
            ),
            "st-lead",
            true,
        );
        lead.set_max_width_chars(90);

        let g1 = SettingsGroup::new("", "");
        g1.add(&st.list);
        let tm = SettingsCard::clickable(
            "startup",
            t("Meer details in Taakbeheer", "More details in Task Manager"),
            t("Bekijk opstart-apps en hun opdrachten", "View startup apps and their commands"),
            Chevron::External,
        );
        if let Some(b) = tm.button() {
            b.set_widget_name("startup-taskmgr");
        }
        let pp = p.clone();
        tm.connect_clicked(move || {
            let fut = pp.run_with("shell.openApp", json!({ "app": "taskmgr", "params": { "page": "startup" } }), None, true);
            pp.spawn(async move {
                let _ = fut.await;
            });
        });
        let g2 = SettingsGroup::new("", "");
        g2.add(tm.widget());
        // the page holds the state while it is shown
        p.on_cleanup(move || drop(st));
        vec![lead.upcast(), toolbar.upcast(), g1.widget().clone().upcast(), g2.widget().clone().upcast()]
    })
}
