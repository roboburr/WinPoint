//! Instellingen › Startpagina (last-webkit:vendor/ui/js/apps/settings/pages/home.js): the device hero, recommended
//! settings, Bluetooth devices, the personalisation quick switch and storage at a glance.
use super::bluetooth::{self, device_glyph, device_status, is_not_responding};
use super::system::device_hero;
use super::themes::theme_grid;
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, UsageBar};
use crate::registry;
use crate::widgets::{AutoGrid, Wrap};
use kit::controls::{self as c, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("home", home)];
pub const CSS: &str = "";

const DEFAULT_RECOMMENDED: &[&str] = &["display", "sound", "taskbar", "background"];

/// A Startpagina card (`.st-hcard`): title (a widget, so a toggle can sit next to it), description, body, foot.
fn home_card(title: &gtk::Widget, desc: &str, body: &[gtk::Widget], foot: &[gtk::Widget], class: &str) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 12);
    card.add_css_class("st-hcard");
    if !class.is_empty() {
        card.add_css_class(class);
    }
    card.set_valign(gtk::Align::Start);
    let head = gtk::Box::new(gtk::Orientation::Vertical, 2);
    head.append(title);
    if !desc.is_empty() {
        head.append(&c::label(desc, "st-hcard-desc", true));
    }
    card.append(&head);
    let b = gtk::Box::new(gtk::Orientation::Vertical, 12);
    b.add_css_class("st-hcard-body");
    for w in body {
        b.append(w);
    }
    card.append(&b);
    if !foot.is_empty() {
        let f = Wrap::new(4, 4);
        f.add_css_class("st-hcard-foot");
        for w in foot {
            f.append(w);
        }
        card.append(&f);
    }
    card
}

fn title_label(text: &str) -> gtk::Widget {
    c::label(text, "st-hcard-title", true).upcast()
}

/// A row of a Startpagina list (`.st-hrow`): glyph, title, description and a trailing widget or chevron;
/// a button when `on_click` is given.
fn row(icon: &gtk::Widget, title: &str, desc: &str, trailing: Option<&gtk::Widget>, on_click: Option<Box<dyn Fn()>>) -> gtk::Widget {
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 14);
    let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    ic.add_css_class("st-hrow-icon");
    ic.set_size_request(24, -1);
    ic.set_valign(gtk::Align::Center);
    ic.set_hexpand(false);
    icon.set_halign(gtk::Align::Center);
    ic.append(icon);
    body.append(&ic);
    let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
    text.set_hexpand(true);
    text.set_valign(gtk::Align::Center);
    text.append(&c::label(title, "st-hrow-title", false));
    if !desc.is_empty() {
        text.append(&c::label(desc, "st-hrow-desc", false));
    }
    body.append(&text);
    match trailing {
        Some(t) => {
            t.set_valign(gtk::Align::Center);
            body.append(t);
        }
        None => {
            let chev = kit::icons::glyph("chevron-right", 12);
            chev.add_css_class("st-hrow-chev");
            chev.set_size_request(20, -1);
            body.append(&chev);
        }
    }
    match on_click {
        Some(f) => {
            let b = gtk::Button::new();
            b.add_css_class("st-hrow");
            b.set_child(Some(&body));
            b.connect_clicked(move |_| f());
            b.upcast()
        }
        None => {
            body.add_css_class("st-hrow");
            body.add_css_class("st-static");
            body.upcast()
        }
    }
}

fn empty_line(text: &str) -> gtk::Widget {
    c::label(text, "st-hcard-empty", true).upcast()
}

fn small_button(label: &str) -> gtk::Button {
    let b = c::button(label, Variant::Standard);
    b.add_css_class("w11-small");
    b.set_valign(gtk::Align::Center);
    b
}

struct Bt {
    state: RefCell<Value>,
    not_ready: Cell<bool>,
    resetting: Cell<bool>,
    body: gtk::Box,
    toggle: Option<ToggleSwitch>,
}

fn paint_bt(p: &Page, bt: &Rc<Bt>) {
    let body = &bt.body;
    c::clear(body);
    let st = bt.state.borrow().clone();
    if st["available"] != true {
        body.append(&empty_line(t("Geen Bluetooth gevonden op deze computer", "No Bluetooth found on this computer")));
        return;
    }
    if let Some(sw) = &bt.toggle {
        sw.set_active(st["powered"] == true);
    }
    if st["powered"] != true {
        body.append(&empty_line(t("Bluetooth staat uit", "Bluetooth is off")));
        return;
    }
    let stuck = st["stuck"] == true || bt.not_ready.get();
    if stuck || bt.resetting.get() {
        // the host's "Bluetooth reageert niet": restart it right here (the same reset as the Bluetooth page)
        let reset = small_button(t("Opnieuw starten", "Restart"));
        reset.set_sensitive(!bt.resetting.get());
        reset.set_widget_name("home-bt-reset");
        let (pp, b2) = (p.clone(), bt.clone());
        reset.connect_clicked(move |_| reset_bt(&pp, &b2));
        let icon = kit::icons::severity_icon("warning", 20);
        body.append(&row(
            icon.upcast_ref(),
            t("Bluetooth reageert niet", "Bluetooth is not responding"),
            t("Je apparaten kunnen nu geen verbinding maken.", "Your devices can’t connect right now."),
            Some(reset.upcast_ref()),
            None,
        ));
    }
    let mut devs: Vec<Value> = st["devices"].as_array().cloned().unwrap_or_default();
    devs.sort_by_key(|d| if d["connected"] == true { 0 } else { 1 });
    devs.truncate(3);
    if devs.is_empty() {
        if !stuck {
            body.append(&empty_line(t("Nog geen gekoppelde apparaten", "No paired devices yet")));
        }
        return;
    }
    for d in devs {
        let label = if d["connected"] == true && !stuck { t("Verbreken", "Disconnect") } else { t("Verbinden", "Connect") };
        let btn = small_button(label);
        btn.set_sensitive(!stuck && !bt.resetting.get());
        let (pp, b2, dev) = (p.clone(), bt.clone(), d.clone());
        btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let (pp, b2, dev) = (pp.clone(), b2.clone(), dev.clone());
            let p2 = pp.clone();
            p2.spawn(async move {
                // the host answers {ok:false, error, code} when it does not work: say so (a stuck adapter: the row above)
                let r = bluetooth::toggle_device(&pp, &dev).await;
                if !pp.alive() {
                    return;
                }
                if let Err(e) = r {
                    if is_not_responding(&e) {
                        b2.not_ready.set(true);
                    } else {
                        pp.error(&e, None);
                    }
                }
                reload_bt(&pp, &b2).await;
                if pp.alive() {
                    paint_bt(&pp, &b2);
                }
            });
        });
        let name = d["name"].as_str().unwrap_or("");
        let icon_name = d["icon"].as_str().filter(|s| !s.is_empty()).unwrap_or(name);
        let icon = kit::icons::glyph(device_glyph(icon_name), 20);
        body.append(&row(icon.upcast_ref(), name, &device_status(&d), Some(btn.upcast_ref()), None));
    }
}

async fn reload_bt(p: &Page, bt: &Rc<Bt>) {
    let r = p.get("system.bluetooth.get", json!({})).await;
    if let Some(r) = r
        && p.alive()
    {
        if r["stuck"] == false {
            bt.not_ready.set(false);
        }
        *bt.state.borrow_mut() = r;
        paint_bt(p, bt);
    }
}

fn reset_bt(p: &Page, bt: &Rc<Bt>) {
    if bt.resetting.get() {
        return;
    }
    bt.resetting.set(true);
    paint_bt(p, bt);
    let (pp, b2) = (p.clone(), bt.clone());
    p.spawn(async move {
        let r = bluetooth::reset_bluetooth(&pp).await;
        b2.resetting.set(false);
        if !pp.alive() {
            return;
        }
        if r.ok {
            b2.not_ready.set(false);
            if let Some(s) = &r.state {
                *b2.state.borrow_mut() = s.clone();
            }
        } else {
            common::show_error_text(&pp.anchor(), &r.error, Some(t("Bluetooth opnieuw starten lukte niet", "Couldn’t restart Bluetooth")));
        }
        paint_bt(&pp, &b2);
        if r.ok && r.state.is_none() {
            reload_bt(&pp, &b2).await;
        }
    });
}

fn home(p: Page) -> PageFuture {
    Box::pin(async move {
        let info = p.get("system.info", json!({}));
        let net = p.get("system.network.get", json!({}));
        let upd = p.get("system.updates", json!({}));
        let bt = p.get("system.bluetooth.get", json!({}));
        let vols = p.get("storage.volumes", json!({}));
        let (info, net, upd, bt, vols) = (
            info.await.unwrap_or(Value::Null),
            net.await.unwrap_or(Value::Null),
            upd.await.unwrap_or(Value::Null),
            bt.await.unwrap_or(Value::Null),
            vols.await.unwrap_or(Value::Null),
        );
        if !p.alive() {
            return Content::new();
        }

        // ---- recommended
        let mut rec: Vec<&'static str> = Vec::new();
        for id in common::recent_pages().into_iter().chain(DEFAULT_RECOMMENDED.iter().copied()) {
            if !rec.contains(&id) {
                rec.push(id);
            }
        }
        rec.truncate(4);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("st-hlist");
        for id in rec {
            let Some(page) = registry::page(id) else { continue };
            let desc = page.parent.and_then(registry::page).map(|pp| pp.title()).unwrap_or(page.description());
            let pp = p.clone();
            let icon = kit::icons::glyph(page.icon, 20);
            let r = row(icon.upcast_ref(), page.title(), desc, None, Some(Box::new(move || pp.go(id))));
            r.set_widget_name(&format!("home-rec-{id}"));
            list.append(&r);
        }
        let rec_card = home_card(
            &title_label(t("Aanbevolen instellingen", "Recommended settings")),
            t("Recente en veelgebruikte instellingen", "Recent and commonly used settings"),
            &[list.upcast()],
            &[],
            "",
        );

        // ---- bluetooth
        let bt_body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bt_body.add_css_class("st-hlist");
        let available = bt["available"] == true;
        let toggle = available.then(|| ToggleSwitch::new(bt["powered"] == true));
        let state = Rc::new(Bt { state: RefCell::new(bt.clone()), not_ready: Cell::new(false), resetting: Cell::new(false), body: bt_body.clone(), toggle: toggle.clone() });
        if let Some(sw) = &toggle {
            sw.switch().set_widget_name("home-bt-toggle");
            let (pp, st, sw2) = (p.clone(), state.clone(), sw.clone());
            sw.connect_toggled(move |on| {
                let (pp, st, sw2) = (pp.clone(), st.clone(), sw2.clone());
                let p2 = pp.clone();
                p2.spawn(async move {
                    let ok = pp.run_with("system.bluetooth.setPowered", json!({ "powered": on }), None, true).await;
                    if ok.is_none() {
                        sw2.set_active(!on);
                    } else if pp.alive() {
                        st.state.borrow_mut()["powered"] = json!(on);
                        paint_bt(&pp, &st);
                    }
                });
            });
        }
        paint_bt(&p, &state);
        {
            let (pp, st) = (p.clone(), state.clone());
            p.on("system.bluetooth.changed", move |d| {
                if d.is_null() {
                    return;
                }
                if d["stuck"] == false {
                    st.not_ready.set(false);
                }
                *st.state.borrow_mut() = d.clone();
                paint_bt(&pp, &st);
            });
        }
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        title_row.add_css_class("st-hcard-title-row");
        let tl = c::label(t("Bluetooth-apparaten", "Bluetooth devices"), "st-hcard-title", true);
        tl.set_hexpand(true);
        title_row.append(&tl);
        if let Some(sw) = &toggle {
            title_row.append(sw.widget());
        }
        let all = c::hyperlink(t("Alle apparaten weergeven", "View all devices"), None);
        let pp = p.clone();
        all.connect_clicked(move |_| pp.go("bluetooth"));
        let mut foot: Vec<gtk::Widget> = vec![all.upcast()];
        if available {
            let add = c::hyperlink(t("Apparaat toevoegen", "Add device"), None);
            let pp = p.clone();
            add.connect_clicked(move |_| bluetooth::open_add_device(&pp));
            foot.push(add.upcast());
        }
        let bt_card = home_card(
            title_row.upcast_ref(),
            t("Apparaten beheren, toevoegen en verbinden", "Manage, add and connect devices"),
            &[bt_body.upcast()],
            &foot,
            "",
        );

        // ---- personalisation
        let mode = common::setting_dropdown(
            &p,
            "theme.mode",
            vec![(json!("light"), t("Licht", "Light").into()), (json!("dark"), t("Donker", "Dark").into())],
            None,
        );
        mode.widget().set_widget_name("home-color-mode");
        let mode_ctl = common::sized(mode.widget(), 140);
        let mode_icon = kit::icons::glyph("theme", 20);
        let mode_row = row(mode_icon.upcast_ref(), t("Kleurmodus", "Colour mode"), "", Some(mode_ctl.upcast_ref()), None);
        mode_row.add_css_class("st-pers-row");
        let browse = c::hyperlink(t("Meer achtergronden, kleuren en thema’s", "Browse more backgrounds, colours and themes"), None);
        let pp = p.clone();
        browse.connect_clicked(move |_| pp.go("personalization"));
        let pers_card = home_card(
            &title_label(t("Persoonlijke instellingen", "Personalise your device")),
            t("Kies een thema, of pas je achtergrond en kleuren aan", "Pick a theme, or change your background and colours"),
            &[theme_grid(&p, true), mode_row],
            &[browse.upcast()],
            "st-hcard-pers",
        );

        let mut cards: Vec<gtk::Box> = vec![rec_card, bt_card, pers_card];

        // ---- storage
        let sys = vols.as_array().and_then(|a| {
            a.iter().find(|v| v["isHome"] == true).or_else(|| a.iter().find(|v| v["isSystem"] == true)).or_else(|| a.first()).cloned()
        });
        if let Some(sys) = sys {
            let pct = common::volume_percent(&sys);
            let store = gtk::Box::new(gtk::Orientation::Vertical, 8);
            store.add_css_class("st-hstore");
            let r = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let ic = kit::icons::glyph("drive", 20);
            ic.set_size_request(24, -1);
            r.append(&ic);
            let name = c::label(&format!("{} ({})", common::s(&sys["name"]), common::s(&sys["mount"])), "st-hstore-name", false);
            name.set_hexpand(true);
            r.append(&name);
            r.append(&c::label(&format!("{}%", pct.round() as i64), "st-hstore-pct w11-tabular", false));
            store.append(&r);
            let bar = UsageBar::new(pct, None, 90.0);
            bar.set_thickness(6);
            store.append(bar.widget());
            let free = common::fill(
                t("{f} vrij van {t}", "{f} free of {t}"),
                &[
                    ("f", &kit::format::bytes(sys["free"].as_f64().unwrap_or(0.0))),
                    ("t", &kit::format::bytes(sys["total"].as_f64().unwrap_or(0.0))),
                ],
            );
            store.append(&c::label(&free, "st-hstore-free", false));
            let details = c::hyperlink(t("Details", "Details"), None);
            let pp = p.clone();
            details.connect_clicked(move |_| pp.go("storage"));
            let cleanup = c::hyperlink(t("Ruimte vrijmaken", "Free up space"), None);
            let pp = p.clone();
            cleanup.connect_clicked(move |_| {
                let fut = pp.run_with("shell.openApp", json!({ "app": "taskmgr", "params": { "page": "storage" } }), None, true);
                pp.spawn(async move {
                    let _ = fut.await;
                });
            });
            cards.push(home_card(
                &title_label(t("Opslag", "Storage")),
                t("Ruimte op je systeemschijf", "Space on your system drive"),
                &[store.upcast()],
                &[details.upcast(), cleanup.upcast()],
                "",
            ));
        }

        let grid = AutoGrid::new(340, 16);
        grid.add_css_class("st-home-grid");
        for card in cards {
            grid.append(&card);
        }

        vec![device_hero(&p, &info, &net, &upd, false), grid.upcast()]
    })
}
