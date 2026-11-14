//! Instellingen › Apps › Standaard-apps (last-webkit:vendor/ui/js/apps/settings/pages/defaultapps.js): which app opens
//! web pages, mail, folders, music, video, pictures, text and the terminal (apps.defaults, apps.list,
//! apps.setDefault). A choice that a desktop-specific mimeapps.list overrides says so and shows what really opens.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{Dropdown, DropdownOption, SettingsCard, SettingsGroup, Severity, Toast};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("defaultapps", defaultapps)];
pub const CSS: &str = "";

struct Kind {
    kind: &'static str,
    icon: &'static str,
    cats: &'static [&'static str],
    not: &'static [&'static str],
}

const KINDS: &[Kind] = &[
    Kind { kind: "browser", icon: "globe", cats: &["WebBrowser"], not: &[] },
    Kind { kind: "mail", icon: "mail", cats: &["Email"], not: &[] },
    Kind { kind: "files", icon: "folder", cats: &["FileManager"], not: &[] },
    Kind { kind: "music", icon: "music", cats: &["Audio", "Music", "Player"], not: &["Video"] },
    Kind { kind: "video", icon: "video", cats: &["Video", "Player"], not: &[] },
    Kind { kind: "image", icon: "image", cats: &["Viewer", "Photography", "2DGraphics", "RasterGraphics", "Graphics"], not: &[] },
    Kind { kind: "text", icon: "document", cats: &["TextEditor"], not: &[] },
    Kind { kind: "terminal", icon: "terminal", cats: &["TerminalEmulator"], not: &[] },
];

fn title(kind: &str) -> &'static str {
    match kind {
        "browser" => t("Webbrowser", "Web browser"),
        "mail" => t("E-mail", "Email"),
        "files" => t("Bestandsbeheer", "File manager"),
        "music" => t("Muziekspeler", "Music player"),
        "video" => t("Videospeler", "Video player"),
        "image" => t("Fotoviewer", "Photo viewer"),
        "text" => t("Teksteditor", "Text editor"),
        _ => t("Terminal", "Terminal"),
    }
}

fn description(kind: &str) -> &'static str {
    match kind {
        "browser" => t("Webpagina’s en internetkoppelingen", "Web pages and internet links"),
        "mail" => t("E-mailadressen (mailto-koppelingen)", "Email addresses (mailto links)"),
        "files" => t("Mappen openen", "Opening folders"),
        "music" => t("Liedjes en geluidsbestanden", "Songs and audio files"),
        "video" => t("Films en video’s", "Films and videos"),
        "image" => t("Foto’s en afbeeldingen", "Photos and pictures"),
        "text" => t("Tekstbestanden (.txt)", "Text files (.txt)"),
        _ => t("Opdrachtregel", "Command line"),
    }
}

fn none() -> &'static str {
    t("Geen app gekozen", "No app chosen")
}

fn has_cat(app: &Value, cats: &[&str]) -> bool {
    app["categories"].as_array().is_some_and(|a| a.iter().any(|c| c.as_str().is_some_and(|c| cats.contains(&c))))
}

/// The candidates of a kind (defaultapps.js: by category; the current default first when it is not one of them).
fn candidates(k: &Kind, apps: &[Value], cur: &Value) -> Vec<Value> {
    let mut cands: Vec<Value> = apps.iter().filter(|a| has_cat(a, k.cats) && !has_cat(a, k.not)).cloned().collect();
    if k.kind == "image" {
        let n = cands.len();
        cands.retain(|a| has_cat(a, &["Viewer", "Photography"]) || n < 3);
    }
    if cur.is_object() && !cands.iter().any(|a| a["id"] == cur["id"]) {
        cands.insert(0, json!({ "id": cur["id"], "name": cur["name"], "icon": cur["icon"] }));
    }
    cands
}

fn options(cands: &[Value]) -> Vec<DropdownOption> {
    let mut opts: Vec<DropdownOption> = cands
        .iter()
        .map(|a| {
            let id = common::value_key(&a["id"]);
            let icon = a["icon"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| id.clone());
            DropdownOption::new(&id, common::s(&a["name"])).app_icon(&icon)
        })
        .collect();
    if opts.is_empty() {
        let mut o = DropdownOption::new("", none());
        o.disabled = true;
        opts.push(o);
    }
    opts
}

fn name_of(cands: &[Value], id: &str) -> Option<String> {
    cands.iter().find(|a| common::value_key(&a["id"]) == id).and_then(|a| a["name"].as_str()).filter(|s| !s.is_empty()).map(str::to_owned)
}

fn kind_card(p: &Page, k: &'static Kind, defs: &Value, apps: &[Value]) -> SettingsCard {
    let cur = &defs[k.kind];
    let cur_id = Rc::new(RefCell::new(common::value_key(&cur["id"])));
    let cands = Rc::new(candidates(k, apps, cur));
    let dd = Dropdown::new(options(&cands), Some(cur_id.borrow().as_str()));
    dd.set_placeholder(none());
    dd.widget().set_widget_name(&format!("defaultapps-{}", k.kind));
    dd.widget().update_property(&[gtk::accessible::Property::Label(title(k.kind))]);
    let (pp, dd2) = (p.clone(), dd.clone());
    dd.connect_changed(move |id| {
        let id = id.to_owned();
        let fut = pp.run("apps.setDefault", json!({ "kind": k.kind, "id": id }));
        let (p, dd, cur_id, cands) = (pp.clone(), dd2.clone(), cur_id.clone(), cands.clone());
        pp.spawn(async move {
            let Some(r) = fut.await else {
                dd.set_selected(Some(cur_id.borrow().as_str()));
                return;
            };
            let app = name_of(&cands, &id).unwrap_or_else(|| id.clone());
            let kind = title(k.kind).to_lowercase();
            if r["effective"] == false {
                // written to mimeapps.list, but a <desktop>-mimeapps.list wins: say so and show what really opens
                let now = p.call("apps.defaults", json!({})).await;
                let eff = now.ok().map(|v| v[k.kind].clone()).unwrap_or(Value::Null);
                let next = if !eff["id"].is_null() {
                    common::value_key(&eff["id"])
                } else if !r["current"].is_null() {
                    common::value_key(&r["current"])
                } else {
                    cur_id.borrow().clone()
                };
                *cur_id.borrow_mut() = next.clone();
                dd.set_selected(Some(&next));
                let files: Vec<String> = r["overriddenBy"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|f| f.as_str().filter(|s| !s.is_empty()).map(str::to_owned)).collect())
                    .unwrap_or_default();
                let current = eff["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| name_of(&cands, &next))
                    .or_else(|| r["current"].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
                    .unwrap_or_else(|| none().to_owned());
                let title = common::fill(
                    t("{app} is opgeslagen, maar {current} blijft je {kind}", "{app} was saved, but {current} stays your {kind}"),
                    &[("app", &app), ("current", &current), ("kind", &kind)],
                );
                let message = if files.is_empty() {
                    t("Een andere instelling gaat voor.", "Another setting takes precedence.").to_owned()
                } else {
                    common::fill(
                        t(
                            "Een instelling voor deze bureaubladomgeving gaat voor: {files}. Pas de app daar aan of verwijder dat bestand.",
                            "A setting for this desktop environment takes precedence: {files}. Change the app there or remove that file.",
                        ),
                        &[("files", &files.join(", "))],
                    )
                };
                p.toast(Toast::new(&title).message(&message).severity(Severity::Warning).timeout(10000));
                return;
            }
            *cur_id.borrow_mut() = id.clone();
            if r["dryRun"] != true {
                let title = common::fill(t("{app} is nu je {kind}", "{app} is now your {kind}"), &[("app", &app), ("kind", &kind)]);
                common::toast_success(&p.anchor(), &title, 2500);
            }
        });
    });
    let card = SettingsCard::new(k.icon, title(k.kind), description(k.kind));
    card.set_control(&common::sized(dd.widget(), 240));
    card
}

fn defaultapps(p: Page) -> PageFuture {
    Box::pin(async move {
        let (dr, lr) = (p.call("apps.defaults", json!({})), p.call("apps.list", json!({})));
        let (dr, lr) = (dr.await, lr.await);
        if !p.alive() {
            return Content::new();
        }
        let Ok(defs) = dr else {
            return vec![common::notice(t("Standaard-apps kunnen hier niet worden gelezen.", "Default apps can’t be read here.")).widget().clone().upcast()];
        };
        let apps: Vec<Value> = lr.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        let g = SettingsGroup::new(
            "",
            t(
                "Kies welke app een soort bestand of koppeling opent. Je kunt dit altijd weer wijzigen.",
                "Choose which app opens a kind of file or link. You can always change this again.",
            ),
        );
        for k in KINDS {
            g.add(kind_card(&p, k, &defs, &apps).widget());
        }
        vec![g.widget().clone().upcast()]
    })
}
