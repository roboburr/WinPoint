//! Instellingen › Systeem (6ebf169f:vendor/ui/js/apps/settings/pages/system.js): the device hero and the links to
//! the System sub-pages. The hero (`device_hero`) and the rename dialog (`rename_hint`, batch E: Windows' "De naam
//! van je pc wijzigen" through `machine.setName`) are shared with Startpagina and Info.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::preview::DeviceArt;
use crate::widgets::Wrap;
use kit::controls::{self as c, Dialog, DialogResult};
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("system", system)];
pub const CSS: &str = "";

/// "Naam van deze pc wijzigen" (Windows' "De naam van je pc wijzigen" dialog; GNOME's About › Device Name): the name
/// goes to hostname1 through `machine.setName` (the pretty name as typed, the network name derived from it as GNOME
/// derives it). Linux needs no restart, so Windows' "Nu opnieuw opstarten" step is left out. The name shown is the
/// current one (`name`); the page follows `machine.changed` once the host took the new one.
pub fn rename_hint(p: &Page, name: &str) {
    let entry = c::text_box("");
    entry.set_widget_name("st-system-rename-entry");
    entry.set_max_length(64);
    entry.set_text(name);
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let shown = if name.is_empty() { t("Deze pc", "This PC") } else { name };
    body.append(&c::label(&common::fill(t("Huidige pc-naam: {name}", "Current PC name: {name}"), &[("name", shown)]), "w11-body", true));
    body.append(&c::field(
        t("Een nieuwe naam voor je pc", "A new name for your PC"),
        &entry,
        Some(t("Je kunt een combinatie van letters, koppeltekens en cijfers gebruiken.", "You can use a combination of letters, hyphens and numbers.")),
    ));
    let pp = p.clone();
    let e2 = entry.clone();
    let dialog = Dialog::new(t("De naam van je pc wijzigen", "Rename your PC")).body(&body).primary(t("Volgende", "Next")).width(440).on_primary(move |d| {
        let name = e2.text().trim().to_owned();
        if !rename_ok(&name) {
            return false;
        }
        d.set_busy(true);
        let (d2, wait) = (d.clone(), pp.run("machine.setName", json!({ "name": name })));
        pp.spawn(async move {
            let ok = wait.await.is_some();
            d2.set_busy(false);
            if ok {
                d2.close(DialogResult::Primary);
            }
        });
        false
    });
    {
        let (d, current) = (dialog.clone(), name.trim().to_owned());
        entry.connect_changed(move |e| {
            let text = e.text().trim().to_owned();
            d.set_primary_enabled(rename_ok(&text) && text != current);
        });
    }
    dialog.show(&p.anchor(), |_| {});
    dialog.set_primary_enabled(false);
}

/// A name the host takes (host machine/hostname.rs `clean`): 1-64 characters, no control characters, at least one
/// letter or digit (the network name is made of those).
pub fn rename_ok(name: &str) -> bool {
    let n = name.trim();
    !n.is_empty() && n.chars().count() <= 64 && !n.chars().any(char::is_control) && n.chars().any(char::is_alphanumeric)
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
    let hostname = pc_name(info).unwrap_or_else(|| t("Deze pc", "This PC").to_owned());
    let model = info["deviceModel"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| common::os_label(common::s(&info["os"]), common::s(&info["osVersion"])));
    let rename = c::hyperlink(t("Naam wijzigen", "Rename"), None);
    rename.set_widget_name("hero-rename");
    let (pp, name) = (p.clone(), pc_name(info).unwrap_or_default());
    rename.connect_clicked(move |_| rename_hint(&pp, &name));
    // a new name (machine.changed {what: name}): the hero is built again with it
    let pp = p.clone();
    p.on("machine.changed", move |d| {
        if d["what"] == "name" {
            pp.refresh();
        }
    });
    let hero = common::hero(
        Some(art.upcast_ref()),
        &hostname,
        &[model],
        Some(rename.upcast_ref()),
        if has_aside { Some(tiles.upcast_ref()) } else { None },
    );
    hero.upcast()
}

/// The computer's name as GNOME shows it: the pretty name (`system.info` prettyHostname, which is the host name when
/// there is none), else the host name.
pub fn pc_name(info: &Value) -> Option<String> {
    ["prettyHostname", "hostname"].iter().find_map(|k| info[*k].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned))
}

fn system(p: Page) -> PageFuture {
    Box::pin(async move {
        let (info, net, upd) = (p.get("system.info", json!({})), p.get("system.network.get", json!({})), p.get("system.updates", json!({})));
        let (info, net, upd) = (info.await.unwrap_or(Value::Null), net.await.unwrap_or(Value::Null), upd.await.unwrap_or(Value::Null));
        if !p.alive() {
            return Content::new();
        }
        // the System pages from the registry (settings parity, B0): Windows' order, caps-filtered
        let group = common::child_links(&p, "system");
        vec![device_hero(&p, &info, &net, &upd, false), group.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pc_name_is_what_the_host_takes() {
        assert!(rename_ok("Robins Laptop") && rename_ok("pc-1") && rename_ok("Café"));
        assert!(!rename_ok("") && !rename_ok("   ") && !rename_ok("---") && !rename_ok("a\tb"));
        assert!(rename_ok(&"a".repeat(64)) && !rename_ok(&"a".repeat(65)));
        assert_eq!(pc_name(&json!({"prettyHostname": "Werkplek", "hostname": "werkplek"})).as_deref(), Some("Werkplek"));
        assert_eq!(pc_name(&json!({"prettyHostname": " ", "hostname": "werkplek"})).as_deref(), Some("werkplek"));
        assert_eq!(pc_name(&json!({})), None);
    }
}
