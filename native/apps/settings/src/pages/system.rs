//! Instellingen › Systeem (6ebf169f:vendor/ui/js/apps/settings/pages/system.js): the device hero and the links to
//! the System sub-pages. The hero (`device_hero`) and the rename hint are shared with Startpagina and Info.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::preview::DeviceArt;
use crate::widgets::Wrap;
use kit::controls::{self as c, Dialog, DialogResult, SettingsGroup};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("system", system)];
pub const CSS: &str = "";

/// How to rename the computer (we never change the hostname ourselves; system.js renameHint).
pub fn rename_hint(p: &Page, name: &str) {
    let shown = if name.is_empty() { t("Deze pc", "This PC") } else { name };
    let body = common::fill(
        t(
            "Deze pc heet nu “{name}”. Je wijzigt de naam in Ubuntu: open de app Instellingen van Ubuntu en ga naar Systeem › Info › Apparaatnaam. Ubuntu vraagt daarbij om je wachtwoord.",
            "This PC is called “{name}”. To change the name, open Ubuntu’s Settings app and go to System › About › Device Name. Ubuntu will ask for your password.",
        ),
        &[("name", shown)],
    );
    let name = name.to_owned();
    Dialog::new(t("De naam van deze pc wijzigen", "Rename this PC"))
        .text(&body)
        .primary("OK")
        .secondary(t("Naam kopiëren", "Copy name"))
        .no_close()
        .show(&p.anchor(), move |r| {
            if r == DialogResult::Secondary {
                common::copy_text(&name);
            }
        });
}

/// The network tile's (glyph, title, status) (system.js networkTileStatus); None without data.
pub fn network_tile_status(net: &Value) -> Option<(String, String, Vec<String>)> {
    if net.is_null() {
        return None;
    }
    let p = &net["primary"];
    let kind = p["type"].as_str().unwrap_or("");
    if kind.is_empty() || kind == "none" {
        return Some(("wifi-off".into(), t("Netwerk", "Network").into(), vec![t("Niet verbonden", "Not connected").into()]));
    }
    let icon = if kind == "wifi" { kit::icons::wifi_icon(Some(p["strength"].as_f64().unwrap_or(70.0))) } else { "ethernet" };
    let title = p["name"].as_str().filter(|s| !s.is_empty()).unwrap_or(t("Netwerk", "Network"));
    Some((icon.into(), title.into(), vec![t("Verbonden", "Connected").into()]))
}

/// The update tile's (glyph, title, status lines) (system.js updateTileStatus); None without data.
pub fn update_tile_status(u: &Value) -> Option<(String, String, Vec<String>)> {
    if u.is_null() {
        return None;
    }
    let status = match u["available"].as_i64() {
        None => String::new(),
        Some(n) if n > 0 => common::plural(n, t("1 update beschikbaar", "1 update available"), t("{n} updates beschikbaar", "{n} updates available")),
        Some(_) => t("Up-to-date", "Up to date").into(),
    };
    let checked = u["checkedAt"]
        .as_f64()
        .map(|ms| common::fill(t("Gecontroleerd {when}", "Checked {when}"), &[("when", &kit::format::rel_time(ms, kit::format::now_ms()))]))
        .unwrap_or_default();
    Some(("update".into(), t("Updates", "Updates").into(), vec![status, checked].into_iter().filter(|s| !s.is_empty()).collect()))
}

/// The device hero of Startpagina and Systeem (system.js deviceHero): the monitor with the wallpaper, the
/// computer's name and model, "Naam wijzigen", and the network and update tiles.
pub fn device_hero(p: &Page, info: &Value, net: &Value, upd: &Value, small: bool) -> gtk::Widget {
    let art = DeviceArt::new(small);
    {
        let art = art.clone();
        p.on_settings(move |s, _| art.update(s));
    }
    let tiles = Wrap::new(4, 4);
    tiles.add_css_class("st-hero-tiles");
    let net_tile = network_tile_status(net).map(|(icon, title, status)| {
        let pp = p.clone();
        let tile = common::hero_tile(&icon, &title, &status, Some(Rc::new(move || pp.go("network"))));
        tile.set_widget_name("hero-network");
        tile
    });
    let update_tile = |p: &Page, u: &Value| {
        update_tile_status(u).map(|(icon, title, status)| {
            let pp = p.clone();
            let tile = common::hero_tile(&icon, &title, &status, Some(Rc::new(move || pp.go("update"))));
            tile.set_widget_name("hero-update");
            tile
        })
    };
    if let Some(t) = &net_tile {
        tiles.append(t);
    }
    let upd_tile = update_tile(p, upd);
    if let Some(t) = &upd_tile {
        tiles.append(t);
    }
    let has_aside = net_tile.is_some() || upd_tile.is_some();
    if has_aside {
        // keep the update count current when the host checks again (system.updates.changed)
        let (pp, tiles2) = (p.clone(), tiles.clone());
        p.on("system.updates.changed", move |d| {
            let Some(nt) = update_tile(&pp, d) else { return };
            tiles2.remove_all();
            if let Some(t) = &net_tile {
                tiles2.append(t);
            }
            tiles2.append(&nt);
        });
    }
    let hostname = info["hostname"].as_str().filter(|s| !s.is_empty()).unwrap_or(t("Deze pc", "This PC")).to_owned();
    let model = info["deviceModel"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| common::os_label(common::s(&info["os"]), common::s(&info["osVersion"])));
    let rename = c::hyperlink(t("Naam wijzigen", "Rename"), None);
    rename.set_widget_name("hero-rename");
    let (pp, name) = (p.clone(), info["hostname"].as_str().unwrap_or("").to_owned());
    rename.connect_clicked(move |_| rename_hint(&pp, &name));
    let hero = common::hero(
        Some(art.upcast_ref()),
        &hostname,
        &[model],
        Some(rename.upcast_ref()),
        if has_aside { Some(tiles.upcast_ref()) } else { None },
    );
    hero.upcast()
}

fn system(p: Page) -> PageFuture {
    Box::pin(async move {
        let (info, net, upd) = (p.get("system.info", json!({})), p.get("system.network.get", json!({})), p.get("system.updates", json!({})));
        let (info, net, upd) = (info.await.unwrap_or(Value::Null), net.await.unwrap_or(Value::Null), upd.await.unwrap_or(Value::Null));
        if !p.alive() {
            return Content::new();
        }
        let group = SettingsGroup::new("", "");
        for id in ["display", "sound", "notifications", "power", "storage", "clipboard", "about"] {
            group.add(common::link_card(&p, id).widget());
        }
        vec![device_hero(&p, &info, &net, &upd, false), group.widget().clone().upcast()]
    })
}
