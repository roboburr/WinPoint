//! Instellingen › Apps › Standaard-apps (6ebf169f:vendor/ui/js/apps/settings/pages/defaultapps.js), laid out as
//! Windows 11 (amendment 22, no category drop-downs at the top): "Een standaardwaarde instellen voor een
//! bestandstype of koppelingstype" (a search over apps.mimeTypes; a type's row opens the apps that can open it,
//! apps.forType, and sets one, apps.setDefaultForType), "Standaardwaarden instellen voor toepassingen" (the apps;
//! an app's view, the data page `defaultapps-app {id}` with the crumb Apps › Standaard-apps › <app>, shows GNOME's
//! Files & Links through app_advanced.rs `handlers_group`; the older deep link `defaultapps {id}` still opens it
//! without the app's crumb), then GNOME's
//! Default Apps as a WinPoint addition (Webbrowser, E-mail, Agenda, Muziek, Video, Foto's, Oproepen and Sms, which
//! GNOME shows only when an app handles them, and WinPoint's Bestandsbeheer, Teksteditor and Terminal:
//! apps.defaults, apps.setDefault), the related "Standaardwaarden kiezen op bestandstype / koppelingstype" views
//! (`{view: "file" | "link"}`) and "Alle standaard-apps opnieuw instellen" (privacy.app.resetHandlers {all}). A
//! choice that a desktop-specific mimeapps.list overrides says so and shows what really opens.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{
    self as c, Dialog, Dropdown, DropdownOption, RadioGroup, SearchBox, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast,
    Variant,
};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("defaultapps", defaultapps), ("defaultapps-app", app_page)];
pub const CSS: &str = "";

struct Kind {
    kind: &'static str,
    icon: &'static str,
    /// GNOME shows the row only when some app handles it (Oproepen, Sms).
    optional: bool,
}

const KINDS: &[Kind] = &[
    Kind { kind: "browser", icon: "globe", optional: false },
    Kind { kind: "mail", icon: "mail", optional: false },
    Kind { kind: "calendar", icon: "calendar", optional: false },
    Kind { kind: "music", icon: "music", optional: false },
    Kind { kind: "video", icon: "video", optional: false },
    Kind { kind: "image", icon: "image", optional: false },
    Kind { kind: "calls", icon: "phone", optional: true },
    Kind { kind: "sms", icon: "mail", optional: true },
    Kind { kind: "files", icon: "folder", optional: false },
    Kind { kind: "text", icon: "document", optional: false },
    Kind { kind: "terminal", icon: "terminal", optional: false },
];

fn title(kind: &str) -> &'static str {
    match kind {
        "browser" => t("Webbrowser", "Web browser"),
        "mail" => t("E-mail", "Email"),
        "calendar" => t("Agenda", "Calendar"),
        "calls" => t("Oproepen", "Calls"),
        "sms" => t("Sms", "SMS"),
        "files" => t("Bestandsbeheer", "File manager"),
        "music" => t("Muziekspeler", "Music player"),
        "video" => t("Videospeler", "Video player"),
        "image" => t("Fotoviewer", "Photo viewer"),
        "text" => t("Teksteditor", "Text editor"),
        _ => t("Terminal", "Terminal"),
    }
}

/// The kind as the toasts name it ("{app} is nu je {kind}").
fn noun(kind: &str) -> String {
    match kind {
        "calls" => t("app voor oproepen", "calls app").to_owned(),
        "sms" => t("sms-app", "SMS app").to_owned(),
        k => title(k).to_lowercase(),
    }
}

fn description(kind: &str) -> &'static str {
    match kind {
        "browser" => t("Webpagina’s en internetkoppelingen", "Web pages and internet links"),
        "mail" => t("E-mailadressen (mailto-koppelingen)", "Email addresses (mailto links)"),
        "calendar" => t("Agenda-uitnodigingen en afspraken (.ics)", "Calendar invitations and appointments (.ics)"),
        "calls" => t("Telefoonnummers (tel-koppelingen)", "Phone numbers (tel links)"),
        "sms" => t("Sms-berichten (sms-koppelingen)", "Text messages (sms links)"),
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

/// The candidates of a kind (apps.defaults' own, by the types the apps open; the current default first when it is
/// not one of them).
fn candidates(defs: &Value, kind: &str) -> Vec<Value> {
    let cur = &defs[kind];
    let mut cands: Vec<Value> = defs["candidates"][kind].as_array().cloned().unwrap_or_default();
    if cur.is_object() && !cands.iter().any(|a| a["id"] == cur["id"]) {
        cands.insert(0, json!({ "id": cur["id"], "name": cur["name"], "icon": cur["icon"] }));
    }
    cands
}

/// Whether the row of a kind is shown: always, or for Oproepen and Sms only while an app handles them.
fn shown(k: &Kind, defs: &Value) -> bool {
    !k.optional || defs[k.kind].is_object() || defs["candidates"][k.kind].as_array().is_some_and(|a| !a.is_empty())
}

fn icon_of(a: &Value) -> String {
    a["icon"].as_str().filter(|s| !s.is_empty()).map(str::to_owned).unwrap_or_else(|| common::value_key(&a["id"]))
}

fn options(cands: &[Value]) -> Vec<DropdownOption> {
    let mut opts: Vec<DropdownOption> =
        cands.iter().map(|a| DropdownOption::new(&common::value_key(&a["id"]), common::s(&a["name"])).app_icon(&icon_of(a))).collect();
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

/// The warning toast of a choice that a desktop-specific mimeapps.list overrides.
fn overridden_toast(p: &Page, app: &str, current: &str, kind: &str, r: &Value) {
    let files: Vec<String> =
        r["overriddenBy"].as_array().map(|a| a.iter().filter_map(|f| f.as_str().filter(|s| !s.is_empty()).map(str::to_owned)).collect()).unwrap_or_default();
    let title = common::fill(
        t("{app} is opgeslagen, maar {current} blijft je {kind}", "{app} was saved, but {current} stays your {kind}"),
        &[("app", app), ("current", current), ("kind", kind)],
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
}

fn kind_card(p: &Page, k: &'static Kind, defs: &Value) -> SettingsCard {
    let cur = &defs[k.kind];
    let cur_id = Rc::new(RefCell::new(common::value_key(&cur["id"])));
    let cands = Rc::new(candidates(defs, k.kind));
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
                let current = eff["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .or_else(|| name_of(&cands, &next))
                    .or_else(|| r["current"].as_str().filter(|s| !s.is_empty()).map(str::to_owned))
                    .unwrap_or_else(|| none().to_owned());
                overridden_toast(&p, &app, &current, &noun(k.kind), &r);
                return;
            }
            *cur_id.borrow_mut() = id.clone();
            if r["dryRun"] != true {
                let title = common::fill(t("{app} is nu je {kind}", "{app} is now your {kind}"), &[("app", &app), ("kind", &noun(k.kind))]);
                common::toast_success(&p.anchor(), &title, 2500);
            }
        });
    });
    let card = SettingsCard::new(k.icon, title(k.kind), description(k.kind));
    card.set_control(&common::sized(dd.widget(), 240));
    card
}

// ------------------------------------------------------------------------------------------- type rows

/// A type's row title: its extensions (".html, .htm"), else its name.
pub(super) fn type_title(item: &Value) -> String {
    let ext: Vec<&str> = item["extensions"].as_array().map(|a| a.iter().filter_map(Value::as_str).take(4).collect()).unwrap_or_default();
    if ext.is_empty() { common::s(&item["name"]).to_owned() } else { ext.join(", ") }
}

/// Whether an apps.mimeTypes item belongs to the view (`link`: x-scheme-handler, `file`: the rest).
pub(super) fn of_view(item: &Value, view: &str) -> bool {
    common::s(&item["mime"]).starts_with("x-scheme-handler/") == (view == "link")
}

/// The button of a type row: the app that opens it now, or "Een standaardwaarde kiezen".
fn default_button(item: &Value) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("w11-button");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let def = &item["default"];
    if def.is_object() {
        content.append(&kit::icons::app_icon(&icon_of(def), 20));
        content.append(&gtk::Label::new(Some(common::s(&def["name"]))));
    } else {
        content.append(&gtk::Label::new(Some(t("Een standaardwaarde kiezen", "Choose a default"))));
    }
    b.set_child(Some(&content));
    b
}

/// One file or link type with its default app; a click opens the apps that can open it.
fn type_row(p: &Page, item: &Value) -> SettingsCard {
    let mime = common::s(&item["mime"]).to_owned();
    let name = common::s(&item["name"]).to_owned();
    let title = type_title(item);
    let card = SettingsCard::new(if mime.starts_with("x-scheme-handler/") { "link" } else { "file" }, &title, if title == name { &mime } else { &name });
    let slot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    let b = default_button(item);
    b.set_widget_name(&format!("defaultapps-type-{}", mime.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>()));
    slot.append(&b);
    let (pp, slot2) = (p.clone(), slot.clone());
    b.connect_clicked(move |_| choose_for_type(&pp, &mime, &title, &slot2));
    card.set_control(&slot);
    card
}

/// "Een standaard-app selecteren voor {type}": the apps apps.forType gives, then apps.setDefaultForType.
fn choose_for_type(p: &Page, mime: &str, title: &str, slot: &gtk::Box) {
    let fut = p.call("apps.forType", json!({ "mime": mime }));
    let (pp, mime, title, slot) = (p.clone(), mime.to_owned(), title.to_owned(), slot.clone());
    p.spawn(async move {
        let item = match fut.await {
            Ok(v) => v,
            Err(e) => return pp.error(&e, None),
        };
        let apps: Vec<Value> = item["apps"].as_array().cloned().unwrap_or_default();
        let heading = common::fill(t("Een standaard-app selecteren voor {type}", "Select a default app for {type}"), &[("type", &title)]);
        if apps.is_empty() {
            c::alert(&pp.anchor(), &heading, t("Er zijn geen apps die dit type kunnen openen.", "No apps can open this type."), || {});
            return;
        }
        let choices: Vec<(String, String)> = apps.iter().map(|a| (common::value_key(&a["id"]), common::s(&a["name"]).to_owned())).collect();
        let refs: Vec<(&str, &str)> = choices.iter().map(|(v, l)| (v.as_str(), l.as_str())).collect();
        let current = apps.iter().find(|a| a["isDefault"] == true).map(|a| common::value_key(&a["id"]));
        let radios = RadioGroup::new(&refs, current.as_deref(), None, false);
        radios.widget().set_widget_name("defaultapps-type-apps");
        let radios2 = radios.clone();
        let (p2, mime2) = (pp.clone(), mime.clone());
        Dialog::new(&heading).body(radios.widget()).primary(t("Standaard instellen", "Set default")).width(440).show(&pp.anchor(), move |r| {
            if r != c::DialogResult::Primary {
                return;
            }
            let Some(id) = radios2.value() else { return };
            let fut = p2.run("apps.setDefaultForType", json!({ "mime": mime2, "id": id }));
            let (p3, slot, mime3, title) = (p2.clone(), slot.clone(), mime2.clone(), title.clone());
            p2.spawn(async move {
                let Some(r) = fut.await else { return };
                let again = p3.call("apps.forType", json!({ "mime": mime3 })).await;
                if let Ok(now) = again {
                    c::clear(&slot);
                    let b = default_button(&now);
                    let (p4, slot2, mime4) = (p3.clone(), slot.clone(), mime3.clone());
                    b.connect_clicked(move |_| choose_for_type(&p4, &mime4, &title, &slot2));
                    slot.append(&b);
                }
                if r["effective"] == false {
                    let app = choices.iter().find(|(v, _)| *v == id).map(|(_, l)| l.clone()).unwrap_or(id.clone());
                    let current = r["current"].as_str().unwrap_or("").to_owned();
                    overridden_toast(&p3, &app, if current.is_empty() { none() } else { &current }, &mime3, &r);
                }
            });
        });
    });
}

// ------------------------------------------------------------------------------------------------ the views

fn defaultapps(p: Page) -> PageFuture {
    Box::pin(async move {
        if let Some(id) = p.param("id").filter(|s| !s.is_empty()) {
            return app_view(&p, &id, false).await;
        }
        if let Some(view) = p.param("view").filter(|v| v == "file" || v == "link") {
            return types_view(&p, &view);
        }
        main_view(&p).await
    })
}

/// "Een standaardwaarde instellen voor een bestandstype of koppelingstype": a search box; up to 20 types match.
fn type_search(p: &Page) -> SettingsGroup {
    let g = SettingsGroup::new(t("Een standaardwaarde instellen voor een bestandstype of koppelingstype", "Set a default for a file type or link type"), "");
    let search = SearchBox::new(Some(t("Voer een bestandstype of koppelingstype in", "Enter a file type or link type")));
    search.widget().set_widget_name("defaultapps-type-search");
    search.set_debounce(250);
    let results = gtk::Box::new(gtk::Orientation::Vertical, 4);
    results.set_widget_name("defaultapps-type-results");
    let seq = Rc::new(std::cell::Cell::new(0u64));
    let (pp, results2) = (p.clone(), results.clone());
    search.connect_changed(move |q| {
        let q = q.trim().to_owned();
        seq.set(seq.get() + 1);
        let mine = seq.get();
        c::clear(&results2);
        if q.is_empty() {
            return;
        }
        let fut = pp.call("apps.mimeTypes", json!({ "query": q, "limit": 20 }));
        let (p2, results3, seq2) = (pp.clone(), results2.clone(), seq.clone());
        pp.spawn(async move {
            let Ok(v) = fut.await else { return };
            if seq2.get() != mine || !p2.alive() {
                return;
            }
            let items = v["items"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                results3.append(&SettingsCard::new("search", t("Geen bestandstypen of koppelingstypen gevonden", "No file types or link types found"), "").widget().clone());
            }
            for item in &items {
                results3.append(type_row(&p2, item).widget());
            }
        });
    });
    g.add(search.widget());
    g.add(&results);
    g
}

/// "Standaardwaarden instellen voor toepassingen": the apps, searchable; each opens its view.
fn app_list(p: &Page, apps: Vec<Value>) -> SettingsGroup {
    let g = SettingsGroup::new(t("Standaardwaarden instellen voor toepassingen", "Set defaults for applications"), "");
    let search = SearchBox::new(Some(t("Apps zoeken", "Search apps")));
    search.widget().set_widget_name("defaultapps-app-search");
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_widget_name("defaultapps-apps");
    let mut apps: Vec<Value> = apps.into_iter().filter(|a| a["noDisplay"] != true && !common::s(&a["id"]).starts_with("win11shell-")).collect();
    apps.sort_by(|a, b| kit::format::compare_natural(common::s(&a["name"]), common::s(&b["name"])));
    let cards: Vec<(String, gtk::Widget)> = apps
        .iter()
        .map(|a| {
            let id = common::value_key(&a["id"]);
            let card = SettingsCard::clickable("", common::s(&a["name"]), "", c::Chevron::Right);
            card.set_icon_widget(&kit::icons::app_icon(&icon_of(a), 32));
            card.widget().add_css_class("st-app-row");
            if let Some(b) = card.button() {
                b.set_widget_name(&format!("defaultapps-app-{id}"));
            }
            let (pp, id2) = (p.clone(), id.clone());
            card.connect_clicked(move || pp.go_with("defaultapps-app", json!({ "id": id2 })));
            (common::s(&a["name"]).to_lowercase(), card.widget().clone())
        })
        .collect();
    for (_, w) in &cards {
        list.append(w);
    }
    let cards = Rc::new(cards);
    search.connect_changed(move |q| {
        let q = q.trim().to_lowercase();
        for (name, w) in cards.iter() {
            w.set_visible(q.is_empty() || name.contains(&q));
        }
    });
    g.add(search.widget());
    g.add(&list);
    g
}

async fn main_view(p: &Page) -> Content {
    let (dr, lr) = (p.call("apps.defaults", json!({ "candidates": true })), p.call("apps.list", json!({})));
    let (dr, lr) = (dr.await, lr.await);
    if !p.alive() {
        return Content::new();
    }
    let Ok(defs) = dr else {
        return vec![common::notice(t("Standaard-apps kunnen hier niet worden gelezen.", "Default apps can’t be read here.")).widget().clone().upcast()];
    };
    let apps: Vec<Value> = lr.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut out: Content = vec![type_search(p).widget().clone().upcast(), app_list(p, apps).widget().clone().upcast()];

    // GNOME's Default Apps, as a WinPoint addition below Windows' own lists
    let exp = SettingsExpander::new(
        "open",
        t("Standaard-apps per soort", "Default apps by kind"),
        t("Webbrowser, e-mail, agenda, muziek, video, foto’s en meer", "Web browser, email, calendar, music, video, photos and more"),
    );
    exp.header_button().set_widget_name("defaultapps-kinds");
    for k in KINDS.iter().filter(|k| shown(k, &defs)) {
        exp.add(kind_card(p, k, &defs).widget());
    }
    let kinds = SettingsGroup::new("", "");
    kinds.add(exp.widget());
    out.push(kinds.widget().clone().upcast());

    let related = SettingsGroup::new(common::ct::related(), "");
    for (view, label) in [
        ("file", t("Standaardwaarden kiezen op bestandstype", "Choose defaults by file type")),
        ("link", t("Standaardwaarden kiezen op koppelingstype", "Choose defaults by link type")),
    ] {
        let card = SettingsCard::clickable(if view == "file" { "file" } else { "link" }, label, "", c::Chevron::Right);
        if let Some(b) = card.button() {
            b.set_widget_name(&format!("defaultapps-view-{view}"));
        }
        let pp = p.clone();
        card.connect_clicked(move || pp.go_with("defaultapps", json!({ "view": view })));
        related.add(card.widget());
    }
    let reset = SettingsCard::new(
        "refresh",
        t("Alle standaard-apps opnieuw instellen", "Reset all default apps"),
        t("Je eigen keuzes voor bestandstypen en koppelingstypen verwijderen", "Remove your own choices for file types and link types"),
    );
    let b = c::button(t("Opnieuw instellen", "Reset"), Variant::Standard);
    b.set_widget_name("defaultapps-reset-all");
    let pp = p.clone();
    b.connect_clicked(move |_| {
        let p2 = pp.clone();
        c::confirm(
            &pp.anchor(),
            t("Alle standaard-apps opnieuw instellen?", "Reset all default apps?"),
            t(
                "Je eigen keuzes voor bestandstypen en koppelingstypen worden verwijderd, zodat de standaardwaarden van het systeem weer gelden.",
                "Your own choices for file types and link types will be removed, so the system’s defaults apply again.",
            ),
            t("Opnieuw instellen", "Reset"),
            true,
            move |ok| {
                if !ok {
                    return;
                }
                let fut = p2.run("privacy.app.resetHandlers", json!({ "all": true }));
                let p3 = p2.clone();
                p2.spawn(async move {
                    if let Some(r) = fut.await {
                        if r["dryRun"] != true {
                            common::toast_success(&p3.anchor(), t("De standaard-apps zijn opnieuw ingesteld", "Default apps were reset"), 2500);
                        }
                        p3.refresh();
                    }
                });
            },
        );
    });
    reset.set_control(&b);
    related.add(reset.widget());
    out.push(related.widget().clone().upcast());
    out
}

/// The data page `defaultapps-app {id}` (Windows' "Standaard-apps › <app>"): the app's view, its name as the last
/// crumb; without an id the Standaard-apps page itself.
fn app_page(p: Page) -> PageFuture {
    Box::pin(async move {
        match p.param("id").filter(|s| !s.is_empty()) {
            Some(id) => app_view(&p, &id, true).await,
            None => main_view(&p).await,
        }
    })
}

/// An app's view (Windows' "Standaard-apps › <app>"): GNOME's Files & Links of the app; `crumb` names the header's
/// last crumb after the app.
async fn app_view(p: &Page, id: &str, crumb: bool) -> Content {
    let (r, list) = (p.call("privacy.app", json!({ "id": id, "sizes": false })), p.call("apps.list", json!({})));
    let (r, list) = (r.await, list.await);
    if !p.alive() {
        return Content::new();
    }
    let list: Vec<Value> = list.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let app = match r {
        Ok(v) => Rc::new(super::app_advanced::with_list_name(v, &list)),
        Err(e) => return vec![common::notice(&e.message).widget().clone().upcast()],
    };
    if crumb && let Some(name) = app["name"].as_str().filter(|n| !n.is_empty()) {
        p.set_title(name);
    }
    let icon = app["icon"].as_str().filter(|s| !s.is_empty()).unwrap_or("application-x-executable");
    let art = kit::icons::app_icon(icon, 48);
    let hero = common::hero(Some(art.upcast_ref()), common::s(&app["name"]), &[], None, None);
    let more = SettingsGroup::new("", "");
    let advanced = SettingsCard::clickable("settings", t("Geavanceerde opties", "Advanced options"), "", c::Chevron::Right);
    if let Some(b) = advanced.button() {
        b.set_widget_name("defaultapps-advanced");
    }
    let (pp, id2) = (p.clone(), id.to_owned());
    advanced.connect_clicked(move || pp.go_with("app-advanced", json!({ "id": id2 })));
    more.add(advanced.widget());
    vec![hero.upcast(), super::app_advanced::handlers_group(p, &app).widget().clone().upcast(), more.widget().clone().upcast()]
}

/// "Standaardwaarden kiezen op bestandstype / koppelingstype": every type of that kind, 100 at a time.
fn types_view(p: &Page, view: &str) -> Content {
    let heading = if view == "file" {
        t("Standaardwaarden kiezen op bestandstype", "Choose defaults by file type")
    } else {
        t("Standaardwaarden kiezen op koppelingstype", "Choose defaults by link type")
    };
    let g = SettingsGroup::new(heading, "");
    let search = SearchBox::new(Some(t("Voer een bestandstype of koppelingstype in", "Enter a file type or link type")));
    search.widget().set_widget_name("defaultapps-view-search");
    search.set_debounce(250);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
    list.set_widget_name("defaultapps-view-list");
    let more = c::button(t("Meer weergeven", "Show more"), Variant::Standard);
    more.set_widget_name("defaultapps-view-more");
    more.set_halign(gtk::Align::Start);
    more.set_visible(false);
    struct Paging {
        query: String,
        offset: u64,
        seq: u64,
    }
    let state = Rc::new(RefCell::new(Paging { query: String::new(), offset: 0, seq: 0 }));
    let view = view.to_owned();
    let load: Rc<dyn Fn(bool)> = {
        let (pp, list, more, state) = (p.clone(), list.clone(), more.clone(), state.clone());
        Rc::new(move |fresh: bool| {
            let (query, offset, mine) = {
                let mut s = state.borrow_mut();
                if fresh {
                    s.offset = 0;
                    s.seq += 1;
                }
                (s.query.clone(), s.offset, s.seq)
            };
            if fresh {
                c::clear(&list);
            }
            // a link view searches among the link types only
            let q = if view == "link" { format!("x-scheme-handler {query}") } else { query };
            let fut = pp.call("apps.mimeTypes", json!({ "query": q.trim(), "offset": offset, "limit": 100 }));
            let (p2, list, more, state, view) = (pp.clone(), list.clone(), more.clone(), state.clone(), view.clone());
            pp.spawn(async move {
                let Ok(v) = fut.await else { return };
                if state.borrow().seq != mine || !p2.alive() {
                    return;
                }
                let items = v["items"].as_array().cloned().unwrap_or_default();
                for item in items.iter().filter(|i| of_view(i, &view)) {
                    list.append(type_row(&p2, item).widget());
                }
                let next = offset + items.len() as u64;
                state.borrow_mut().offset = next;
                more.set_visible(next < v["total"].as_u64().unwrap_or(0));
            });
        })
    };
    {
        let (load, state) = (load.clone(), state.clone());
        search.connect_changed(move |q| {
            state.borrow_mut().query = q.trim().to_owned();
            load(true);
        });
    }
    {
        let load = load.clone();
        more.connect_clicked(move |_| load(false));
    }
    load(true);
    g.add(search.widget());
    g.add(&list);
    g.add(&more);
    vec![g.widget().clone().upcast()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_and_sms_show_only_with_an_app_and_candidates_come_from_the_host() {
        let calls = KINDS.iter().find(|k| k.kind == "calls").unwrap();
        let browser = KINDS.iter().find(|k| k.kind == "browser").unwrap();
        let none = json!({ "candidates": { "calls": [] } });
        assert!(!shown(calls, &none), "GNOME hides Calls without a handler");
        assert!(shown(browser, &none));
        let one = json!({ "candidates": { "calls": [{ "id": "dialer.desktop", "name": "Dialer" }] } });
        assert!(shown(calls, &one));
        let current = json!({ "calls": { "id": "old.desktop", "name": "Old" }, "candidates": { "calls": [] } });
        assert!(shown(calls, &current), "a default set elsewhere is shown");
        let defs = json!({ "browser": { "id": "b.desktop", "name": "B" }, "candidates": { "browser": [{ "id": "a.desktop", "name": "A" }] } });
        let ids: Vec<String> = candidates(&defs, "browser").iter().map(|a| common::value_key(&a["id"])).collect();
        assert_eq!(ids, ["b.desktop", "a.desktop"], "the current default first when it is no candidate");
        assert_eq!(
            KINDS.iter().map(|k| k.kind).collect::<Vec<_>>(),
            ["browser", "mail", "calendar", "music", "video", "image", "calls", "sms", "files", "text", "terminal"],
            "GNOME's order, then WinPoint's own kinds"
        );
    }

    #[test]
    fn type_rows_and_views() {
        let html = json!({ "mime": "text/html", "name": "HTML document", "extensions": [".html", ".htm"] });
        assert_eq!(type_title(&html), ".html, .htm");
        let tel = json!({ "mime": "x-scheme-handler/tel", "name": "tel links", "extensions": [] });
        assert_eq!(type_title(&tel), "tel links");
        assert!(of_view(&tel, "link") && !of_view(&tel, "file"));
        assert!(of_view(&html, "file") && !of_view(&html, "link"));
    }
}
