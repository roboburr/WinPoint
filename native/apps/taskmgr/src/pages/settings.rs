//! Instellingen (js/apps/taskmgr/pages/settings.js): settings.taskmgr.* and the efficiency-mode question.
use super::Page;
use crate::app::{App, EventSub, SettingsSub};
use crate::util::{self, TmGlyph, fill};
use kit::controls::{Dropdown, SettingsCard, SettingsGroup, ToggleSwitch};
use kit::prelude::*;
use std::rc::Rc;

fn tr(key: &str) -> &'static str {
    let (nl, en) = match key {
        "title" => ("Instellingen", "Settings"),
        "general" => ("Algemeen", "General"),
        "graphs" => ("Grafieken", "Graphs"),
        "safety" => ("Veiligheid", "Safety"),
        "about" => ("Over Taakbeheer", "About Task Manager"),
        "speed" => ("Realtime-updatesnelheid", "Real-time update speed"),
        "speedDesc" => ("Hoe vaak Taakbeheer de gegevens vernieuwt. Langzamer gebruikt minder stroom.", "How often Task Manager refreshes its data. Slower uses less power."),
        "high" => ("Hoog (elke 0,5 seconde)", "High (every 0.5 seconds)"),
        "normal" => ("Normaal (elke seconde)", "Normal (every second)"),
        "low" => ("Laag (elke 4 seconden)", "Low (every 4 seconds)"),
        "paused" => ("Gepauzeerd", "Paused"),
        "page" => ("Standaardstartpagina", "Default start page"),
        "pageDesc" => ("De pagina die je ziet als je Taakbeheer opent.", "The page you see when you open Task Manager."),
        "onTop" => ("Altijd op voorgrond", "Always on top"),
        "onTopDesc" => ("Taakbeheer blijft boven andere vensters en is op elk bureaublad te zien.", "Keeps Task Manager above other windows and on every desktop."),
        "confirm" => ("Vragen voordat een taak wordt beëindigd", "Ask before ending a task"),
        "confirmDesc" => ("Je krijgt eerst uitleg, zodat je geen werk kwijtraakt.", "You\u{2019}ll get an explanation first, so you don\u{2019}t lose any work."),
        "effConfirm" => ("Vragen voordat efficiëntiemodus aangaat", "Ask before turning on efficiency mode"),
        "effConfirmDesc" => ("Uitleg over wat efficiëntiemodus doet.", "Explains what efficiency mode does."),
        "colors" => ("Kleurgecodeerde grafieken", "Colour-keyed graphs"),
        "colorsDesc" => (
            "Processor blauw, geheugen paars, schijf groen, netwerk oranje en videokaart groenblauw. Uit = alles in je accentkleur.",
            "Processor blue, memory purple, disk green, network orange and graphics teal. Off = everything in your accent colour.",
        ),
        "duration" => ("Grafiekduur", "Graph length"),
        "durationDesc" => ("Hoeveel geschiedenis de grafieken laten zien.", "How much history the graphs show."),
        "s30" => ("30 seconden", "30 seconds"),
        "s60" => ("60 seconden", "60 seconds"),
        "s120" => ("2 minuten", "2 minutes"),
        "s300" => ("5 minuten", "5 minutes"),
        "aboutTitle" => ("Taakbeheer voor Win11 Shell", "Task Manager for Win11 Shell"),
        "aboutDesc" => ("Versie {v}. Alle gegevens komen van je eigen pc; er wordt niets verstuurd.", "Version {v}. All data comes from your own PC; nothing is sent anywhere."),
        "testMode" => ("Testmodus: acties worden niet echt uitgevoerd", "Test mode: actions aren\u{2019}t really carried out"),
        _ => ("", ""),
    };
    kit::t(nl, en)
}

struct Settings {
    root: gtk::Box,
    _settings_sub: SettingsSub,
    _info_sub: EventSub,
}

fn dropdown(options: &[(&str, &str)], selected: &str) -> Dropdown {
    let d = Dropdown::simple(options, Some(selected));
    d.widget().set_size_request(240, -1);
    d
}

fn card(icon: &str, title: &str, desc: &str, control: &impl IsA<gtk::Widget>) -> gtk::Widget {
    let c = SettingsCard::new(icon, tr(title), tr(desc));
    c.set_control(control);
    c.widget().clone()
}

fn about_text(app: &App) -> String {
    let info = app.info();
    let v = info["version"].as_str().filter(|s| !s.is_empty()).unwrap_or("\u{2013}").to_owned();
    let mut s = fill(tr("aboutDesc"), &[("v", &v)]);
    if app.dry_run() {
        s.push(' ');
        s.push_str(tr("testMode"));
        s.push('.');
    }
    s
}

fn speed_of(app: &App) -> String {
    app.setting("taskmgr.updateSpeed").as_str().unwrap_or("normal").to_owned()
}

fn page_of(app: &App) -> String {
    app.setting("taskmgr.defaultPage").as_str().unwrap_or("summary").to_owned()
}

fn seconds_of(app: &App) -> String {
    app.setting("taskmgr.graphSeconds").as_i64().unwrap_or(60).to_string()
}

fn flag(app: &App, path: &str, fallback: bool) -> bool {
    app.setting(path).as_bool().unwrap_or(fallback)
}

pub fn create(app: &Rc<App>) -> Rc<dyn Page> {
    let speed = dropdown(&[("high", tr("high")), ("normal", tr("normal")), ("low", tr("low")), ("paused", tr("paused"))], &speed_of(app));
    speed.widget().set_widget_name("taskmgr-setting-speed");
    let pages: Vec<(&str, &str)> = ["summary", "processes", "performance", "storage", "startup", "services", "details"].iter().map(|id| (*id, super::title_of(id))).collect();
    let page = dropdown(&pages, &page_of(app));
    page.widget().set_widget_name("taskmgr-setting-page");
    let duration = dropdown(&[("30", tr("s30")), ("60", tr("s60")), ("120", tr("s120")), ("300", tr("s300"))], &seconds_of(app));
    duration.widget().set_widget_name("taskmgr-setting-duration");
    let on_top = ToggleSwitch::new(flag(app, "taskmgr.alwaysOnTop", false));
    on_top.switch().set_widget_name("taskmgr-setting-ontop");
    let confirm = ToggleSwitch::new(flag(app, "taskmgr.confirmEndTask", true));
    confirm.switch().set_widget_name("taskmgr-setting-confirm");
    let colors = ToggleSwitch::new(flag(app, "taskmgr.colorKeyedGraphs", true));
    colors.switch().set_widget_name("taskmgr-setting-colors");
    let eff = ToggleSwitch::new(!util::load_bool("effNoConfirm", false));
    eff.switch().set_widget_name("taskmgr-setting-effconfirm");

    {
        let (a, d) = (app.clone(), speed.clone());
        speed.connect_changed(move |v| {
            let (a2, d2) = (a.clone(), d.clone());
            a.set_setting("taskmgr.updateSpeed", json!(v), move || d2.set_selected(Some(&speed_of(&a2))));
        });
        let (a, d) = (app.clone(), page.clone());
        page.connect_changed(move |v| {
            let (a2, d2) = (a.clone(), d.clone());
            a.set_setting("taskmgr.defaultPage", json!(v), move || d2.set_selected(Some(&page_of(&a2))));
        });
        let (a, d) = (app.clone(), duration.clone());
        duration.connect_changed(move |v| {
            let n: i64 = v.parse().unwrap_or(60);
            let (a2, d2) = (a.clone(), d.clone());
            a.set_setting("taskmgr.graphSeconds", json!(n), move || d2.set_selected(Some(&seconds_of(&a2))));
        });
        for (sw, path) in [(&on_top, "taskmgr.alwaysOnTop"), (&confirm, "taskmgr.confirmEndTask"), (&colors, "taskmgr.colorKeyedGraphs")] {
            let (a, s) = (app.clone(), sw.clone());
            sw.connect_toggled(move |on| {
                let s2 = s.clone();
                a.set_setting(path, json!(on), move || s2.set_active(!on));
            });
        }
        eff.connect_toggled(|on| util::save("effNoConfirm", json!(!on)));
    }

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.add_css_class("taskmgr-settings");
    root.set_widget_name("taskmgr-settings");
    let general = SettingsGroup::new(tr("general"), "");
    general.add(&card("refresh", "speed", "speedDesc", speed.widget()));
    general.add(&card("home", "page", "pageDesc", page.widget()));
    general.add(&card("window", "onTop", "onTopDesc", on_top.widget()));
    let graphs = SettingsGroup::new(tr("graphs"), "");
    graphs.add(&card("color", "colors", "colorsDesc", colors.widget()));
    graphs.add(&card("clock", "duration", "durationDesc", duration.widget()));
    let safety = SettingsGroup::new(tr("safety"), "");
    safety.add(&card("shield", "confirm", "confirmDesc", confirm.widget()));
    let eff_card = SettingsCard::new("", tr("effConfirm"), tr("effConfirmDesc"));
    eff_card.set_icon_widget(&util::icon(TmGlyph::LeafOutline, 20));
    eff_card.set_control(eff.widget());
    safety.add(eff_card.widget());
    let about = SettingsGroup::new(tr("about"), "");
    let about_card = SettingsCard::new("info", tr("aboutTitle"), &about_text(app));
    about.add(about_card.widget());
    for g in [&general, &graphs, &safety, &about] {
        root.append(g.widget());
    }

    let a = Rc::downgrade(app);
    let settings_sub = app.on_settings(move |_, _| {
        let Some(a) = a.upgrade() else { return };
        speed.set_selected(Some(&speed_of(&a)));
        page.set_selected(Some(&page_of(&a)));
        duration.set_selected(Some(&seconds_of(&a)));
        on_top.set_active(flag(&a, "taskmgr.alwaysOnTop", false));
        confirm.set_active(flag(&a, "taskmgr.confirmEndTask", true));
        colors.set_active(flag(&a, "taskmgr.colorKeyedGraphs", true));
    });
    let a = Rc::downgrade(app);
    let info_sub = app.on("taskmgr.info", move |_, _| {
        if let Some(a) = a.upgrade() {
            about_card.set_description(&about_text(&a));
        }
    });
    Rc::new(Settings { root, _settings_sub: settings_sub, _info_sub: info_sub })
}

impl Page for Settings {
    fn id(&self) -> &'static str {
        "settings"
    }

    fn title(&self) -> String {
        tr("title").to_owned()
    }

    fn widget(&self) -> gtk::Widget {
        self.root.clone().upcast()
    }
}
