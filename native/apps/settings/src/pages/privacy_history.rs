//! Instellingen › Privacy en beveiliging › Activiteitengeschiedenis, Zoekmachtigingen, Deze pc doorzoeken, as
//! Windows 11 lays them out, with GNOME Settings 50's functions (settings parity, batch F, amendment 45):
//!
//! * Activiteitengeschiedenis: GNOME's File History (org.gnome.desktop.privacy `remember-recent-files`, which the
//!   mirror keeps equal to settings.json `privacy.recentFiles`), File History Duration (`recent-files-max-age`) and
//!   Clear History (apps.clearRecent).
//! * Zoekmachtigingen: GNOME's Search panel, which is the GNOME overview's search: App Search (search-providers
//!   `disable-external` inverted) and the Search Results list (privacy.searchProviders; each provider in `disabled`
//!   or, a DefaultDisabled one, in `enabled`; their order `sort-order`, moved with Omhoog / Omlaag).
//! * Deze pc doorzoeken: Windows' Mijn bestanden zoeken (Klassiek / Uitgebreid) drives WinPoint's own index first
//!   (index.setRoots: Start's file search) and mirrors the same folders into Tracker
//!   (org.freedesktop.Tracker3.Miner.Files `index-recursive-directories` / `index-single-directories`, what Bestanden
//!   and the overview search); the Zoeklocaties are GNOME's Search Locations (standaard-, bladwijzer- en aangepaste
//!   locaties); Mappen uitsluiten van uitgebreid zoeken is the index's own `excluded` list. Without Tracker (its
//!   schema not installed) the page says so and keeps WinPoint's own rows: Klassiek / Uitgebreid then drives the
//!   index alone (Klassiek: the XDG folders, GNOME's default places), and the excluded folders.
use super::app_advanced::{search_list, search_on};
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, RadioGroup, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("activity", activity), ("searchpermissions", searchpermissions), ("searching", searching)];
pub const CSS: &str = "";

const REMEMBER: GKey = gs::key("org.gnome.desktop.privacy", "remember-recent-files");
const MAX_AGE: GKey = gs::key("org.gnome.desktop.privacy", "recent-files-max-age");
const EXTERNAL: GKey = gs::key("org.gnome.desktop.search-providers", "disable-external");
const DISABLED: GKey = gs::key("org.gnome.desktop.search-providers", "disabled");
const ENABLED: GKey = gs::key("org.gnome.desktop.search-providers", "enabled");
const ORDER: GKey = gs::key("org.gnome.desktop.search-providers", "sort-order");
const SEARCH_KEYS: &[GKey] = &[EXTERNAL, DISABLED, ENABLED, ORDER];
const TRACKER: &str = "org.freedesktop.Tracker3.Miner.Files";
const RECURSIVE: GKey = gs::key(TRACKER, "index-recursive-directories");
const SINGLE: GKey = gs::key(TRACKER, "index-single-directories");
/// Alles, WinPoint's index app (its options hold the rest of the index's settings).
const ALLES: &str = "win11shell-alles.desktop";

// ------------------------------------------------------------------------------------------- pure parts

pub(super) fn strings(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect()).unwrap_or_default()
}

/// "Geschiedenis bewaren gedurende": GNOME's presets (1, 7, 30 days, Altijd = -1) and a current value that is no
/// preset as an option of its own.
pub(super) fn age_options(current: Option<i64>) -> Vec<(i64, String)> {
    let days = |n: i64| if n == 1 { t("1 dag", "1 day").to_owned() } else { common::fill(t("{n} dagen", "{n} days"), &[("n", &n.to_string())]) };
    let mut out: Vec<(i64, String)> = [1, 7, 30].iter().map(|n| (*n, days(*n))).collect();
    if let Some(n) = current.filter(|n| *n > 0 && ![1, 7, 30].contains(n)) {
        let at = out.iter().position(|(m, _)| *m > n).unwrap_or(out.len());
        out.insert(at, (n, days(n)));
    }
    out.push((-1, t("Altijd", "Forever").to_owned()));
    out
}

/// The providers in GNOME's order (cc-search-panel.c): those in `sort-order` first, in its order, then the rest as
/// they came (by name).
pub(super) fn ordered(providers: &[Value], order: &[String]) -> Vec<Value> {
    let mut out = providers.to_vec();
    out.sort_by_key(|p| order.iter().position(|o| o == common::s(&p["desktopId"])).unwrap_or(usize::MAX));
    out
}

/// `sort-order` after moving the provider `id` one place up or down: every provider's id in the new order (GNOME
/// writes the whole list).
pub(super) fn moved(ids: &[String], id: &str, up: bool) -> Vec<String> {
    let mut out = ids.to_vec();
    if let Some(i) = out.iter().position(|x| x == id) {
        let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < out.len()) };
        if let Some(j) = j {
            out.swap(i, j);
        }
    }
    out
}

/// The user's folders Tracker knows by token.
#[derive(Clone, Debug, Default)]
pub(super) struct Dirs {
    pub home: String,
    /// (`&DESKTOP`, path), ... of the XDG folders that are set (and are not the home folder itself)
    pub special: Vec<(&'static str, String)>,
}

impl Dirs {
    fn here() -> Dirs {
        let home = glib::home_dir().to_string_lossy().into_owned();
        let special = [
            ("&DESKTOP", glib::UserDirectory::Desktop),
            ("&DOCUMENTS", glib::UserDirectory::Documents),
            ("&DOWNLOAD", glib::UserDirectory::Downloads),
            ("&MUSIC", glib::UserDirectory::Music),
            ("&PICTURES", glib::UserDirectory::Pictures),
            ("&VIDEOS", glib::UserDirectory::Videos),
        ]
        .into_iter()
        .filter_map(|(token, dir)| {
            let path = glib::user_special_dir(dir)?.to_string_lossy().into_owned();
            (path != home).then_some((token, path))
        })
        .collect();
        Dirs { home, special }
    }
}

/// A Tracker directory entry as a path: `$HOME`, `&DESKTOP` ... (cc-search-locations-dialog.c path_from_tracker_dir).
pub(super) fn tracker_path(value: &str, dirs: &Dirs) -> String {
    if value == "$HOME" {
        return dirs.home.clone();
    }
    if let Some(rest) = value.strip_prefix("$HOME/") {
        return format!("{}/{rest}", dirs.home.trim_end_matches('/'));
    }
    if let Some((_, path)) = dirs.special.iter().find(|(token, _)| *token == value) {
        return path.clone();
    }
    value.to_owned()
}

/// A path as Tracker's token when it has one (path_to_tracker_dir), else the path.
pub(super) fn tracker_token(path: &str, dirs: &Dirs) -> String {
    if path == dirs.home {
        return "$HOME".into();
    }
    dirs.special.iter().find(|(_, p)| p == path).map(|(token, _)| (*token).to_owned()).unwrap_or_else(|| path.to_owned())
}

/// Whether a Tracker list holds `path` (as its token or as the path).
pub(super) fn holds(list: &[String], path: &str, dirs: &Dirs) -> bool {
    list.iter().any(|v| tracker_path(v, dirs) == path)
}

/// A Tracker list with `path` added (as its token) or taken out (every spelling of it).
pub(super) fn with_place(list: &[String], path: &str, on: bool, dirs: &Dirs) -> Vec<String> {
    let mut out: Vec<String> = list.iter().filter(|v| tracker_path(v, dirs) != path).cloned().collect();
    if on {
        out.push(tracker_token(path, dirs));
    }
    out
}

/// Which list a place's switch writes: the Persoonlijke map is indexed without its subfolders (GNOME's Home place,
/// `index-single-directories`), every other place with them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Depth {
    Single,
    Recursive,
}

/// One row of the Zoeklocaties.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Place {
    pub path: String,
    pub label: String,
    pub depth: Depth,
    /// "default", "bookmark" or "custom"
    pub group: &'static str,
}

/// The bookmarks of ~/.config/gtk-3.0/bookmarks (`file:///path label`): local folders only, GNOME's own places left
/// out (cc-search-locations-dialog.c get_bookmarks).
pub(super) fn bookmarks(text: &str, taken: &[String]) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let (uri, label) = line.split_once(' ').unwrap_or((line, ""));
        if !uri.starts_with("file://") {
            continue;
        }
        let Some(path) = gio::File::for_uri(uri).path().map(|p| p.to_string_lossy().into_owned()) else { continue };
        if taken.contains(&path) || out.iter().any(|(p, _)| *p == path) {
            continue;
        }
        let label = if label.trim().is_empty() { path.rsplit('/').next().unwrap_or(&path).to_owned() } else { label.trim().to_owned() };
        out.push((path, label));
    }
    out
}

/// Every place GNOME's Search Locations dialog shows: Persoonlijke map and the XDG folders, the bookmarks, and the
/// folders of either list that are none of these (Aangepaste locaties).
pub(super) fn places(dirs: &Dirs, bookmarks_text: &str, recursive: &[String], single: &[String]) -> Vec<Place> {
    let label = |token: &str| -> String {
        match token {
            "&DESKTOP" => t("Bureaublad", "Desktop"),
            "&DOCUMENTS" => t("Documenten", "Documents"),
            "&DOWNLOAD" => t("Downloads", "Downloads"),
            "&MUSIC" => t("Muziek", "Music"),
            "&PICTURES" => t("Afbeeldingen", "Pictures"), // glossary-ok: Afbeeldingen
            _ => t("Video's", "Videos"),
        }
        .to_owned()
    };
    let mut out = vec![Place { path: dirs.home.clone(), label: t("Persoonlijke map", "Home").to_owned(), depth: Depth::Single, group: "default" }];
    out.extend(dirs.special.iter().map(|(token, path)| Place { path: path.clone(), label: label(token), depth: Depth::Recursive, group: "default" }));
    let mut taken: Vec<String> = out.iter().map(|p| p.path.clone()).collect();
    for (path, label) in bookmarks(bookmarks_text, &taken) {
        taken.push(path.clone());
        out.push(Place { path, label, depth: Depth::Recursive, group: "bookmark" });
    }
    for (list, depth) in [(recursive, Depth::Recursive), (single, Depth::Single)] {
        for v in list {
            let path = tracker_path(v, dirs);
            if path.starts_with('/') && !taken.contains(&path) {
                taken.push(path.clone());
                let label = path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&path).to_owned();
                out.push(Place { path, label, depth, group: "custom" });
            }
        }
    }
    out
}

/// Whether a place is on: in the list its switch writes (the Persoonlijke map also while Uitgebreid indexes it with
/// its subfolders).
pub(super) fn place_on(place: &Place, recursive: &[String], single: &[String], dirs: &Dirs) -> bool {
    match place.depth {
        Depth::Single => holds(single, &place.path, dirs) || holds(recursive, &place.path, dirs),
        Depth::Recursive => holds(recursive, &place.path, dirs),
    }
}

/// Tracker's two lists after a place's switch: on adds it to its own list, off takes it out of both.
pub(super) fn lists_for_place(place: &Place, on: bool, recursive: &[String], single: &[String], dirs: &Dirs) -> (Vec<String>, Vec<String>) {
    match (place.depth, on) {
        (Depth::Single, true) => (recursive.to_vec(), with_place(single, &place.path, true, dirs)),
        (Depth::Recursive, true) => (with_place(recursive, &place.path, true, dirs), single.to_vec()),
        (_, false) => (with_place(recursive, &place.path, false, dirs), with_place(single, &place.path, false, dirs)),
    }
}

/// The folders WinPoint's index gets in Klassiek: the places that are on with their subfolders (the index has no
/// "without subfolders"), that exist.
pub(super) fn classic_roots(places: &[Place], recursive: &[String], dirs: &Dirs, exists: impl Fn(&str) -> bool) -> Vec<String> {
    places
        .iter()
        .filter(|p| p.depth == Depth::Recursive && p.path != dirs.home && holds(recursive, &p.path, dirs) && exists(&p.path))
        .map(|p| p.path.clone())
        .collect()
}

/// Tracker's lists for a choice of Mijn bestanden zoeken: Uitgebreid indexes the whole home folder with its
/// subfolders (`$HOME` recursive, localsearch's own default); Klassiek takes it out again and indexes the places,
/// GNOME's default places (the Persoonlijke map without subfolders and the XDG folders) when none is on.
pub(super) fn tracker_for_mode(enhanced: bool, recursive: &[String], single: &[String], dirs: &Dirs) -> (Vec<String>, Vec<String>) {
    if enhanced {
        let mut r = with_place(recursive, &dirs.home, true, dirs);
        r.sort_by_key(|v| v != "$HOME");
        return (r, with_place(single, &dirs.home, false, dirs));
    }
    let mut r = with_place(recursive, &dirs.home, false, dirs);
    let mut s = single.to_vec();
    if r.is_empty() {
        r = dirs.special.iter().map(|(token, _)| (*token).to_owned()).collect();
        s = with_place(&s, &dirs.home, true, dirs);
    }
    (r, s)
}

/// Uitgebreid is WinPoint's index over every drive (`alles.roots` "auto", its default); a list of folders is Klassiek.
pub(super) fn enhanced(roots: &Value) -> bool {
    !roots.is_array() || roots.as_array().is_some_and(|r| r.is_empty())
}

// ------------------------------------------------------------------------------------------ Activiteitengeschiedenis

/// "Geschiedenis wissen": ask, then apps.clearRecent (privacy.js clearBtn, GNOME's Clear History…).
fn clear_recent(p: &Page) {
    let pp = p.clone();
    c::confirm(
        &p.anchor(),
        t("Activiteitengeschiedenis wissen?", "Clear activity history?"),
        t(
            "De lijst met onlangs geopende bestanden wordt leeggemaakt, ook in andere apps die deze lijst gebruiken. Je bestanden zelf worden niet verwijderd.",
            "The list of recently opened files will be emptied, including in other apps that use this list. Your files themselves aren’t deleted.",
        ),
        t("Wissen", "Clear"),
        true,
        move |ok| {
            if !ok {
                return;
            }
            let wait = pp.call("apps.clearRecent", json!({}));
            let p2 = pp.clone();
            pp.spawn(async move {
                match wait.await {
                    Ok(v) if v["dryRun"] == true => common::note_dry_run(&p2.anchor(), &v),
                    Ok(_) => common::toast_success(&p2.anchor(), t("De activiteitengeschiedenis is gewist", "Activity history was cleared"), 2500),
                    Err(e) if e.code == "unknown_method" => p2.toast(
                        Toast::new(t("Wissen is in deze versie nog niet beschikbaar", "Clearing isn’t available in this version yet"))
                            .severity(Severity::Info)
                            .timeout(5000),
                    ),
                    Err(e) => p2.error(&e, None),
                }
            });
        },
    );
}

fn activity(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &[REMEMBER, MAX_AGE]).await;
        if !p.alive() {
            return Content::new();
        }
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let g = SettingsGroup::new("", "");
        // Windows' own row (Activity history): "opslaan" is "store" here
        let title = t("Mijn activiteitengeschiedenis op dit apparaat opslaan", "Store my activity history on this device"); // glossary-ok: Opslaan
        let desc = t(
            "Bijhouden welke bestanden je opent, zodat Start, Verkenner en andere apps je recente bestanden kunnen weergeven. Alleen jij ziet deze lijst en hij blijft op dit apparaat.",
            "Keep track of the files you open, so Start, File Explorer and other apps can show your recent files. Only you see this list and it stays on this device.",
        );
        let card = match gs.toggle_card(&REMEMBER, "history", title, desc, false) {
            Some(card) => card,
            // without GNOME's schema: WinPoint's own setting, as before
            None => {
                let (card, sw) = common::toggle_card(&p, "history", title, desc, "privacy.recentFiles", false);
                sw.switch().set_widget_name("st-activity-remember-recent-files");
                card
            }
        };
        g.add(card.widget());
        if gs.available(&MAX_AGE) {
            let options: Vec<(Value, String)> = age_options(gs.value(&MAX_AGE).as_i64()).into_iter().map(|(n, l)| (json!(n), l)).collect();
            let dd = gs.dropdown(&MAX_AGE, options);
            let age = SettingsCard::new("clock", t("Geschiedenis bewaren gedurende", "Keep history for"), t("Hoelang recente bestanden in de lijst blijven", "How long recent files stay in the list"));
            age.set_control(&common::sized(dd.widget(), 180));
            // GNOME: the duration only counts while File History is on
            let sensitive = {
                let (gs, age) = (gs.clone(), age.clone());
                move || age.set_sensitive(gs.value(&REMEMBER) != false && gs.writable(&MAX_AGE))
            };
            sensitive();
            gs.follow(&REMEMBER, move |_| sensitive());
            g.add(age.widget());
        }
        let clear_btn = c::button(t("Geschiedenis wissen", "Clear history"), Variant::Standard);
        clear_btn.set_widget_name("activity-clear");
        {
            let pp = p.clone();
            clear_btn.connect_clicked(move |_| clear_recent(&pp));
        }
        let clear = SettingsCard::new(
            "delete",
            t("Activiteitengeschiedenis wissen", "Clear activity history"),
            t("De lijst met onlangs geopende bestanden leegmaken. De bestanden zelf blijven bestaan.", "Empty the list of recently opened files. The files themselves stay."),
        );
        clear.set_control(&clear_btn);
        // Windows keeps the clear row in the same group as the rows above it (its spacing, no group gap)
        g.add(clear.widget());
        vec![g.widget().clone().upcast()]
    })
}

// ------------------------------------------------------------------------------------------------ Zoekmachtigingen

fn related(p: &Page, ids: &[&str]) -> Option<SettingsGroup> {
    common::links_group(p, t("Gerelateerde instellingen", "Related settings"), ids)
}

fn searchpermissions(p: Page) -> PageFuture {
    Box::pin(async move {
        let (list, names) = (p.call("privacy.searchProviders", json!({})), p.call("apps.list", json!({})));
        let gs = Gs::load(&p, SEARCH_KEYS).await;
        let (list, names) = (list.await, names.await);
        if !p.alive() {
            return Content::new();
        }
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let mut out: Content = vec![
            common::notice(t(
                "Deze instellingen gelden voor het zoeken in het overzicht van GNOME (Activiteiten). Zoeken in Start gebruikt de eigen index van WinPoint (Deze pc doorzoeken).",
                "These settings apply to searching in the GNOME overview (Activities). Search in Start uses WinPoint’s own index (Searching this PC).",
            ))
            .widget()
            .clone()
            .upcast(),
        ];
        let g = SettingsGroup::new("", "");
        match gs.toggle_card(
            &EXTERNAL,
            "search",
            t("Zoekresultaten van apps opnemen", "Include results from apps"),
            t("Apps kunnen resultaten weergeven wanneer je in het overzicht zoekt", "Apps can show results when you search in the overview"),
            true,
        ) {
            Some(card) => g.add(card.widget()),
            None => g.add(common::unavailable_card("search", t("Zoekresultaten van apps opnemen", "Include results from apps"), None).widget()),
        }
        out.push(g.widget().clone().upcast());
        let mut providers: Vec<Value> = list.ok().and_then(|v| v["providers"].as_array().cloned()).unwrap_or_default();
        // the names as Geïnstalleerde apps shows them (apps.list: WinPoint's language); the host's own otherwise
        let names: Vec<Value> = names.ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
        for prov in providers.iter_mut() {
            if let Some(name) = names.iter().find(|a| a["id"] == prov["desktopId"]).and_then(|a| a["name"].as_str()).filter(|n| !n.is_empty()) {
                prov["name"] = json!(name);
            }
        }
        if !providers.is_empty() && gs.available(&ORDER) {
            out.push(results_group(&p, &gs, providers).widget().clone().upcast());
        }
        if let Some(g) = related(&p, &["searching"]) {
            out.push(g.widget().clone().upcast());
        }
        out
    })
}

/// Whether a row is the first or the last of the list (its Omhoog / Omlaag has nowhere to go).
type Ends = Rc<std::cell::RefCell<HashMap<String, (bool, bool)>>>;

/// "Zoekresultaten": one row per provider (switch, Omhoog, Omlaag), in `sort-order`.
fn results_group(p: &Page, gs: &Gs, providers: Vec<Value>) -> SettingsGroup {
    let g = SettingsGroup::new(t("Zoekresultaten", "Search results"), t("De resultaten worden in de volgorde van deze lijst weergegeven.", "Results are shown in the order of this list."));
    let providers = Rc::new(providers);
    let ends: Ends = Rc::default();
    let rows: Rc<HashMap<String, (gtk::Widget, Rc<dyn Fn()>)>> = Rc::new(
        providers.iter().map(|prov| (common::s(&prov["desktopId"]).to_owned(), provider_row(p, gs, prov, providers.clone(), ends.clone()))).collect(),
    );
    let fill = {
        let (g, rows, providers, gs) = (g.clone(), rows.clone(), providers.clone(), gs.clone());
        move || {
            g.clear();
            let list = ordered(&providers, &strings(&gs.value(&ORDER)));
            let last = list.len().saturating_sub(1);
            for (i, prov) in list.iter().enumerate() {
                let id = common::s(&prov["desktopId"]).to_owned();
                ends.borrow_mut().insert(id.clone(), (i == 0, i == last));
                if let Some((row, refresh)) = rows.get(&id) {
                    g.add(row);
                    refresh();
                }
            }
        }
    };
    fill();
    gs.follow(&ORDER, move |_| fill());
    g
}

/// A widget-name part for a desktop id (`org.gnome.Nautilus.desktop` → `org-gnome-nautilus`).
pub(super) fn id_part(id: &str) -> String {
    id.trim_end_matches(".desktop").chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect()
}

fn provider_row(p: &Page, gs: &Gs, prov: &Value, all: Rc<Vec<Value>>, ends: Ends) -> (gtk::Widget, Rc<dyn Fn()>) {
    let _ = p;
    let id = common::s(&prov["desktopId"]).to_owned();
    let part = id_part(&id);
    let default_disabled = prov["defaultDisabled"] == true;
    let list_key = if default_disabled { ENABLED } else { DISABLED };
    let icon = prov["icon"].as_str().filter(|s| !s.is_empty() && !s.starts_with('/')).unwrap_or("application-x-executable");
    let card = SettingsCard::new(icon, common::s(&prov["name"]), "");
    let sw = ToggleSwitch::new(search_on(default_disabled, &id, &strings(&gs.value(&DISABLED)), &strings(&gs.value(&ENABLED))));
    sw.switch().set_widget_name(&format!("st-searchpermissions-{}-{part}", list_key.key));
    let silent = Rc::new(std::cell::Cell::new(false));
    {
        let (gs2, sw2, id2, key2, silent) = (gs.clone(), sw.clone(), id.clone(), list_key.clone(), silent.clone());
        sw.connect_toggled(move |on| {
            if silent.get() {
                return;
            }
            let list = search_list(on, default_disabled, &id2, &strings(&gs2.value(&key2)));
            let (sw3, silent) = (sw2.clone(), silent.clone());
            gs2.set(&key2, json!(list), move || {
                silent.set(true);
                sw3.set_active(!on);
                silent.set(false);
            });
        });
    }
    let up = c::icon_button("arrow-up", t("Omhoog", "Move up"), 32);
    let down = c::icon_button("arrow-down", t("Omlaag", "Move down"), 32);
    up.set_widget_name(&format!("searchpermissions-up-{part}"));
    down.set_widget_name(&format!("searchpermissions-down-{part}"));
    for (b, is_up) in [(&up, true), (&down, false)] {
        let (gs2, id2, all) = (gs.clone(), id.clone(), all.clone());
        b.connect_clicked(move |_| {
            let ids: Vec<String> = ordered(&all, &strings(&gs2.value(&ORDER))).iter().map(|v| common::s(&v["desktopId"]).to_owned()).collect();
            let next = moved(&ids, &id2, is_up);
            if next != ids {
                gs2.set(&ORDER, json!(next), || {});
            }
        });
    }
    // GNOME greys the list while App Search is off; a locked key keeps its control insensitive; the first row has
    // no Omhoog, the last no Omlaag
    let refresh: Rc<dyn Fn()> = {
        let (gs, sw, up, down, list_key, id) = (gs.clone(), sw.clone(), up.clone(), down.clone(), list_key.clone(), id.clone());
        Rc::new(move || {
            let external_on = gs.value(&EXTERNAL) != true;
            sw.set_sensitive(external_on && gs.writable(&list_key));
            if !gs.writable(&list_key) {
                sw.switch().set_tooltip_text(Some(gs::locked_text()));
            }
            let (first, last) = ends.borrow().get(&id).copied().unwrap_or((false, false));
            up.set_sensitive(external_on && gs.writable(&ORDER) && !first);
            down.set_sensitive(external_on && gs.writable(&ORDER) && !last);
            silent.set(true);
            sw.set_active(search_on(default_disabled, &id, &strings(&gs.value(&DISABLED)), &strings(&gs.value(&ENABLED))));
            silent.set(false);
        })
    };
    refresh();
    for k in [EXTERNAL, DISABLED, ENABLED] {
        let refresh = refresh.clone();
        gs.follow(&k, move |_| refresh());
    }
    card.set_control(sw.widget());
    card.add_control(&up);
    card.add_control(&down);
    card.widget().set_widget_name(&format!("searchpermissions-provider-{part}"));
    (card.widget().clone(), refresh)
}

// ------------------------------------------------------------------------------------------------ Deze pc doorzoeken

/// What the page needs to write the index and Tracker together.
struct Search {
    p: Page,
    gs: Gs,
    dirs: Dirs,
    bookmarks: String,
    /// Tracker's schema is installed (else only WinPoint's index is written)
    tracker: bool,
}

impl Search {
    fn lists(&self) -> (Vec<String>, Vec<String>) {
        (strings(&self.gs.value(&RECURSIVE)), strings(&self.gs.value(&SINGLE)))
    }

    fn places(&self) -> Vec<Place> {
        let (r, s) = self.lists();
        places(&self.dirs, &self.bookmarks, &r, &s)
    }

    /// The index's folders in Klassiek for these Tracker lists.
    fn classic(&self, recursive: &[String], single: &[String]) -> Vec<String> {
        let all = places(&self.dirs, &self.bookmarks, recursive, single);
        classic_roots(&all, recursive, &self.dirs, |p| std::path::Path::new(p).is_dir())
    }
}

/// Amendment 45: WinPoint's index first (`roots`, None = unchanged), then Tracker's lists; `done(ok)`. When Tracker
/// refuses (a locked key), the index gets back the folders it had.
fn write(search: &Rc<Search>, roots: Option<Value>, recursive: Vec<String>, single: Vec<String>, done: impl FnOnce(bool) + 'static) {
    let me = search.clone();
    search.p.spawn(async move {
        let before = me.p.setting("alles.roots");
        let wrote_index = roots.is_some();
        if let Some(roots) = roots {
            let Some(r) = me.p.run("index.setRoots", json!({ "roots": roots })).await else {
                done(false);
                return;
            };
            if r["overridden"] == true {
                me.p.toast(Toast::new(common::s(&r["message"])).severity(Severity::Info).timeout(5000));
            }
        }
        if !me.tracker {
            done(true);
            return;
        }
        let values = json!({ RECURSIVE.key: recursive, SINGLE.key: single });
        let ok = me.p.run("gsettings.set", json!({ "schema": TRACKER, "values": values })).await.is_some();
        if !ok && wrote_index {
            let _ = me.p.call("index.setRoots", json!({ "roots": if before.is_array() { before } else { json!("auto") } })).await;
        }
        done(ok);
    });
}

fn mode_text(enhanced: bool) -> &'static str {
    if enhanced {
        t(
            "Je hele pc doorzoeken, inclusief je bibliotheken en bureaublad. Start doorzoekt alle schijven en Bestanden en het overzicht van GNOME je hele persoonlijke map.",
            "Search your entire PC, including your libraries and desktop. Start searches every drive, and Files and the GNOME overview your whole home folder.",
        )
    } else {
        t(
            "Alleen je bibliotheken en bureaublad doorzoeken: de zoeklocaties hieronder, in Start en in Bestanden.",
            "Only search your libraries and desktop: the search locations below, in Start and in Files.",
        )
    }
}

fn searching(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &[RECURSIVE, SINGLE]).await;
        if !p.alive() {
            return Content::new();
        }
        let keep = gs.clone();
        p.on_cleanup(move || drop(keep));
        let tracker = gs.available(&RECURSIVE) && gs.available(&SINGLE);
        let bookmarks = std::fs::read_to_string(glib::user_config_dir().join("gtk-3.0").join("bookmarks")).unwrap_or_default();
        let search = Rc::new(Search { p: p.clone(), gs: gs.clone(), dirs: Dirs::here(), bookmarks, tracker });
        let mut out: Content = Vec::new();
        if !tracker {
            // Bestanden and the GNOME overview have no file search here; Start's index (WinPoint's own) still has
            let missing = common::notice(t("Het zoeken in bestanden (Tracker) is op dit apparaat niet beschikbaar.", "File search (Tracker) isn’t available on this device."));
            missing.widget().set_widget_name("searching-no-tracker");
            out.push(missing.widget().clone().upcast());
        }

        // ---- Mijn bestanden zoeken (Klassiek / Uitgebreid)
        let is_enhanced = enhanced(&p.setting("alles.roots"));
        let exp = SettingsExpander::new(
            "search",
            t("Mijn bestanden zoeken", "Find my files"),
            t("Kies waar Start, Bestanden en het overzicht van GNOME naar je bestanden zoeken", "Choose where Start, Files and the GNOME overview look for your files"),
        );
        let rg = RadioGroup::new(&[("classic", t("Klassiek", "Classic")), ("enhanced", t("Uitgebreid", "Enhanced"))], Some(if is_enhanced { "enhanced" } else { "classic" }), None, false);
        rg.widget().set_widget_name("searching-mode");
        if let Some(b) = rg.button("classic") {
            b.set_widget_name("searching-classic");
        }
        if let Some(b) = rg.button("enhanced") {
            b.set_widget_name("searching-enhanced");
        }
        let desc = c::label(mode_text(is_enhanced), "w11-text-2", true);
        // the choice the page shows (the settings cache follows a write only when its event arrives)
        let mode = Rc::new(std::cell::Cell::new(is_enhanced));
        let writable = !tracker || (gs.writable(&RECURSIVE) && gs.writable(&SINGLE));
        rg.widget().set_sensitive(writable);
        if !writable {
            rg.widget().set_tooltip_text(Some(gs::locked_text()));
        }
        exp.add(rg.widget());
        exp.add(&desc);
        exp.set_expanded(true);
        {
            let (search, rg2, desc, mode) = (search.clone(), rg.clone(), desc.clone(), mode.clone());
            rg.connect_changed(move |v| {
                let want = v == "enhanced";
                if want == mode.get() {
                    return;
                }
                mode.set(want);
                let (r, s) = search.lists();
                let (r, s) = tracker_for_mode(want, &r, &s, &search.dirs);
                let roots = if want { json!("auto") } else { json!(search.classic(&r, &s)) };
                let (rg3, desc, mode) = (rg2.clone(), desc.clone(), mode.clone());
                write(&search, Some(roots), r, s, move |ok| {
                    let now = if ok { want } else { !want };
                    mode.set(now);
                    rg3.set_value(if now { "enhanced" } else { "classic" });
                    desc.set_text(mode_text(now));
                });
            });
        }
        let g = SettingsGroup::new("", "");
        g.add(exp.widget());
        if tracker {
            g.add(locations(&search).widget());
        }
        out.push(g.widget().clone().upcast());
        {
            // an outside change of the index's folders (Alles' options, another host)
            let (rg, desc) = (rg.clone(), desc.clone());
            p.on_settings(move |s, _| {
                let now = enhanced(&s["alles"]["roots"]);
                mode.set(now);
                rg.set_value(if now { "enhanced" } else { "classic" });
                desc.set_text(mode_text(now));
            });
        }

        // ---- Mappen uitsluiten van uitgebreid zoeken (the index's own list)
        out.push(excluded_group(&p).widget().clone().upcast());

        // ---- Gerelateerde instellingen
        let rel = related(&p, &["searchpermissions"]).unwrap_or_else(|| SettingsGroup::new(t("Gerelateerde instellingen", "Related settings"), ""));
        let alles = SettingsCard::clickable("search", t("Geavanceerde indexeringsopties", "Advanced indexing options"), t("Stations, verborgen bestanden en de index opnieuw opbouwen in Alles", "Drives, hidden files and rebuilding the index in Alles"), c::Chevron::External);
        if let Some(b) = alles.button() {
            b.set_widget_name("searching-alles");
        }
        {
            let pp = p.clone();
            alles.connect_clicked(move || {
                let fut = pp.run("apps.launch", json!({ "id": ALLES }));
                pp.spawn(async move {
                    let _ = fut.await;
                });
            });
        }
        rel.add(alles.widget());
        out.push(rel.widget().clone().upcast());
        out
    })
}

/// "Zoeklocaties aanpassen": GNOME's Search Locations, rebuilt whenever Tracker's lists change.
fn locations(search: &Rc<Search>) -> SettingsExpander {
    let exp = SettingsExpander::new(
        "folder",
        t("Zoeklocaties aanpassen", "Customise search locations"),
        t("De mappen die Bestanden en het overzicht van GNOME doorzoeken; in Klassiek ook Start", "The folders Files and the GNOME overview search; in Classic also Start"),
    );
    exp.header_button().set_widget_name("searching-locations");
    let fill = {
        let (exp, search) = (exp.clone(), search.clone());
        Rc::new(move || {
            exp.clear();
            let (r, s) = search.lists();
            let all = search.places();
            for (group, title) in [
                ("default", t("Standaardlocaties", "Default locations")),
                ("bookmark", t("Bladwijzerlocaties", "Bookmarked locations")),
                ("custom", t("Aangepaste locaties", "Custom locations")),
            ] {
                let rows: Vec<&Place> = all.iter().filter(|p| p.group == group).collect();
                if rows.is_empty() && group != "custom" {
                    continue;
                }
                exp.add(&common::section_title(title));
                for place in rows {
                    exp.add(place_row(&search, place, place_on(place, &r, &s, &search.dirs)).widget());
                }
                if group == "custom" {
                    exp.add(add_location_row(&search).widget());
                }
            }
        })
    };
    fill();
    for k in [RECURSIVE, SINGLE] {
        let fill = fill.clone();
        search.gs.follow(&k, move |_| fill());
    }
    exp
}

fn place_row(search: &Rc<Search>, place: &Place, on: bool) -> SettingsCard {
    let desc = if place.depth == Depth::Single {
        common::fill(t("{path} (zonder submappen)", "{path} (without subfolders)"), &[("path", &place.path)])
    } else {
        place.path.clone()
    };
    let card = SettingsCard::new("folder", &place.label, &desc);
    let sw = ToggleSwitch::new(on);
    let part = place.path.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect::<String>();
    sw.switch().set_widget_name(&format!("searching-place{part}"));
    let writable = search.gs.writable(&RECURSIVE) && search.gs.writable(&SINGLE);
    sw.set_sensitive(writable);
    if !writable {
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (search, place, sw2) = (search.clone(), place.clone(), sw.clone());
        sw.connect_toggled(move |on| toggle_place(&search, &place, on, Some(sw2.clone())));
    }
    card.set_control(sw.widget());
    if place.group == "custom" {
        let remove = c::button(t("Verwijderen", "Remove"), Variant::Standard);
        remove.set_widget_name(&format!("searching-remove{part}"));
        remove.set_sensitive(writable);
        let (search, place) = (search.clone(), place.clone());
        remove.connect_clicked(move |_| toggle_place(&search, &place, false, None));
        card.add_control(&remove);
    }
    card
}

/// A place's switch (or its Verwijderen): Tracker's lists, and in Klassiek the index's folders first.
fn toggle_place(search: &Rc<Search>, place: &Place, on: bool, sw: Option<ToggleSwitch>) {
    let (r, s) = search.lists();
    let (r, s) = lists_for_place(place, on, &r, &s, &search.dirs);
    let roots = (!enhanced(&search.p.setting("alles.roots"))).then(|| json!(search.classic(&r, &s)));
    write(search, roots, r, s, move |ok| {
        if !ok && let Some(sw) = sw {
            sw.set_active(!on);
        }
    });
}

fn add_location_row(search: &Rc<Search>) -> SettingsCard {
    let card = SettingsCard::new("plus", t("Locatie toevoegen", "Add location"), t("Een map toevoegen die met alle submappen wordt doorzocht", "Add a folder that is searched with all its subfolders"));
    let add = c::button(t("Map kiezen", "Choose folder"), Variant::Standard);
    add.set_widget_name("searching-add");
    add.set_sensitive(search.gs.writable(&RECURSIVE));
    let search = search.clone();
    add.connect_clicked(move |_| {
        let dialog = gtk::FileDialog::new();
        dialog.set_title(t("Een map kiezen om te doorzoeken", "Choose a folder to search"));
        dialog.set_modal(true);
        let (search, window) = (search.clone(), search.p.window().window().clone());
        dialog.select_folder(Some(&window), gio::Cancellable::NONE, move |result| {
            let Ok(folder) = result else { return };
            let Some(path) = folder.path().map(|p| p.to_string_lossy().into_owned()) else { return };
            let place = Place { path, label: String::new(), depth: Depth::Recursive, group: "custom" };
            toggle_place(&search, &place, true, None);
        });
    });
    card.set_control(&add);
    card
}

/// "Mappen uitsluiten van uitgebreid zoeken": the index's `excluded` folders (index.setRoots with the roots it has).
fn excluded_group(p: &Page) -> SettingsGroup {
    let g = SettingsGroup::new(
        t("Mappen uitsluiten van uitgebreid zoeken", "Exclude folders from enhanced search"),
        t("Start doorzoekt deze mappen en hun submappen niet.", "Start doesn’t search these folders and their subfolders."),
    );
    let write = {
        let p = p.clone();
        Rc::new(move |list: Vec<String>| {
            let roots = p.setting("alles.roots");
            let roots = if roots.is_array() { roots } else { json!("auto") };
            let fut = p.run("index.setRoots", json!({ "roots": roots, "excluded": list }));
            p.spawn(async move {
                let _ = fut.await;
            });
        })
    };
    let fill = {
        let (g, p, write) = (g.clone(), p.clone(), write.clone());
        Rc::new(move || {
            g.clear();
            let list = strings(&p.setting("alles")["excluded"]);
            let add = c::button(t("Bladeren", "Browse"), Variant::Standard);
            add.set_widget_name("searching-exclude-add");
            {
                let (p, write, list) = (p.clone(), write.clone(), list.clone());
                add.connect_clicked(move |_| {
                    let dialog = gtk::FileDialog::new();
                    dialog.set_title(t("Een map kiezen om uit te sluiten", "Choose a folder to exclude"));
                    dialog.set_modal(true);
                    let (write, list) = (write.clone(), list.clone());
                    dialog.select_folder(Some(p.window().window()), gio::Cancellable::NONE, move |result| {
                        let Some(path) = result.ok().and_then(|f| f.path()).map(|p| p.to_string_lossy().into_owned()) else { return };
                        if !list.contains(&path) {
                            let mut next = list.clone();
                            next.push(path);
                            write(next);
                        }
                    });
                });
            }
            let head = SettingsCard::new("folder", t("Een uitgesloten map toevoegen", "Add an excluded folder"), "");
            head.set_control(&add);
            g.add(head.widget());
            for (i, path) in list.iter().enumerate() {
                let row = SettingsCard::new("folder", path.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(path), path);
                let remove = c::button(t("Verwijderen", "Remove"), Variant::Standard);
                remove.set_widget_name(&format!("searching-exclude-remove-{i}"));
                let (write, list, path) = (write.clone(), list.clone(), path.clone());
                remove.connect_clicked(move |_| write(list.iter().filter(|x| **x != path).cloned().collect()));
                row.set_control(&remove);
                g.add(row.widget());
            }
        })
    };
    fill();
    p.on_settings(move |_, path| {
        if path.is_none_or(|path| path.starts_with("alles")) {
            fill();
        }
    });
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirs() -> Dirs {
        Dirs {
            home: "/home/u".into(),
            special: vec![("&DESKTOP", "/home/u/Bureaublad".into()), ("&DOCUMENTS", "/home/u/Documenten".into()), ("&MUSIC", "/home/u/Muziek".into())],
        }
    }

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn the_history_duration_has_gnomes_presets_and_the_current_value() {
        kit::i18n::set_forced_lang(Some("nl"));
        let values = |c: Option<i64>| age_options(c).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(values(Some(7)), [1, 7, 30, -1]);
        assert_eq!(values(Some(14)), [1, 7, 14, 30, -1], "a value of its own gets an option in its place");
        assert_eq!(values(Some(90)), [1, 7, 30, 90, -1]);
        assert_eq!(age_options(Some(1))[0].1, "1 dag");
        assert_eq!(age_options(None)[3].1, "Altijd");
        kit::i18n::set_forced_lang(Some("en"));
        assert_eq!(age_options(Some(14))[2].1, "14 days");
        kit::i18n::set_forced_lang(None);
    }

    #[test]
    fn search_results_follow_sort_order_and_move_one_place() {
        let providers: Vec<Value> = ["a.desktop", "b.desktop", "c.desktop", "d.desktop"].iter().map(|id| json!({ "desktopId": id })).collect();
        let ids = |v: Vec<Value>| v.iter().map(|p| common::s(&p["desktopId"]).to_owned()).collect::<Vec<_>>();
        assert_eq!(ids(ordered(&providers, &list(&["c.desktop", "a.desktop"]))), ["c.desktop", "a.desktop", "b.desktop", "d.desktop"]);
        assert_eq!(ids(ordered(&providers, &[])), ["a.desktop", "b.desktop", "c.desktop", "d.desktop"]);
        let order = list(&["c.desktop", "a.desktop", "b.desktop"]);
        assert_eq!(moved(&order, "a.desktop", true), ["a.desktop", "c.desktop", "b.desktop"]);
        assert_eq!(moved(&order, "a.desktop", false), ["c.desktop", "b.desktop", "a.desktop"]);
        assert_eq!(moved(&order, "c.desktop", true), order, "the first stays first");
        assert_eq!(moved(&order, "b.desktop", false), order, "the last stays last");
        assert_eq!(id_part("org.gnome.Nautilus.desktop"), "org-gnome-nautilus");
    }

    #[test]
    fn tracker_tokens_and_places_follow_gnomes_search_locations() {
        let d = dirs();
        assert_eq!(tracker_path("$HOME", &d), "/home/u");
        assert_eq!(tracker_path("$HOME/Projecten", &d), "/home/u/Projecten");
        assert_eq!(tracker_path("&DOCUMENTS", &d), "/home/u/Documenten");
        assert_eq!(tracker_path("&VIDEOS", &d), "&VIDEOS", "an unset folder stays its token");
        assert_eq!(tracker_token("/home/u/Muziek", &d), "&MUSIC");
        assert_eq!(tracker_token("/home/u", &d), "$HOME");
        assert_eq!(tracker_token("/srv/data", &d), "/srv/data");
        assert!(holds(&list(&["/home/u/Documenten"]), "/home/u/Documenten", &d));
        assert_eq!(with_place(&list(&["/home/u/Documenten", "&MUSIC"]), "/home/u/Documenten", true, &d), ["&MUSIC", "&DOCUMENTS"], "written as its token");
        assert_eq!(with_place(&list(&["/home/u/Muziek", "&MUSIC", "/srv"]), "/home/u/Muziek", false, &d), ["/srv"], "every spelling goes");
        let marks = "file:///home/u/Projecten Werk\nfile:///home/u/Documenten\nsftp://server/x Server\nfile:///home/u/Spellen\n";
        let all = places(&d, marks, &list(&["&DESKTOP", "/srv/data", "/home/u/Projecten"]), &list(&["$HOME", "/opt/single"]));
        let rows: Vec<(&str, &str, Depth, &str)> = all.iter().map(|p| (p.path.as_str(), p.label.as_str(), p.depth, p.group)).collect();
        kit::i18n::set_forced_lang(Some("nl"));
        assert_eq!(
            rows[..4],
            [
                ("/home/u", rows[0].1, Depth::Single, "default"),
                ("/home/u/Bureaublad", rows[1].1, Depth::Recursive, "default"),
                ("/home/u/Documenten", rows[2].1, Depth::Recursive, "default"),
                ("/home/u/Muziek", rows[3].1, Depth::Recursive, "default"),
            ]
        );
        kit::i18n::set_forced_lang(None);
        assert_eq!(rows[4], ("/home/u/Projecten", "Werk", Depth::Recursive, "bookmark"));
        assert_eq!(rows[5], ("/home/u/Spellen", "Spellen", Depth::Recursive, "bookmark"), "no label: the folder's name; sftp is left out");
        assert_eq!(rows[6], ("/srv/data", "data", Depth::Recursive, "custom"));
        assert_eq!(rows[7], ("/opt/single", "single", Depth::Single, "custom"));
        assert_eq!(rows.len(), 8, "a bookmark already in a list is no custom place");
    }

    #[test]
    fn classic_and_enhanced_write_the_index_and_tracker_together() {
        let d = dirs();
        let all = places(&d, "", &list(&["&DESKTOP", "&MUSIC"]), &list(&["$HOME"]));
        // Klassiek: the recursive places that are on, never the home folder (the index knows no "without subfolders")
        assert_eq!(classic_roots(&all, &list(&["&DESKTOP", "&MUSIC"]), &d, |_| true), ["/home/u/Bureaublad", "/home/u/Muziek"]);
        assert_eq!(classic_roots(&all, &list(&["&DESKTOP", "&MUSIC"]), &d, |p| p.ends_with("Muziek")), ["/home/u/Muziek"], "only folders that exist");
        // Uitgebreid: $HOME with its subfolders first, out of the single list
        assert_eq!(tracker_for_mode(true, &list(&["&DESKTOP"]), &list(&["$HOME"]), &d), (list(&["$HOME", "&DESKTOP"]), list(&[])));
        // back to Klassiek: $HOME goes, the places stay
        assert_eq!(tracker_for_mode(false, &list(&["$HOME", "&DESKTOP"]), &list(&[]), &d), (list(&["&DESKTOP"]), list(&[])));
        // Klassiek from localsearch's default ($HOME only): GNOME's default places
        assert_eq!(tracker_for_mode(false, &list(&["$HOME"]), &list(&[]), &d), (list(&["&DESKTOP", "&DOCUMENTS", "&MUSIC"]), list(&["$HOME"])));
        // a place's switch: on into its own list, off out of both
        let home = &all[0];
        assert!(place_on(home, &list(&["$HOME"]), &list(&[]), &d), "Uitgebreid indexes the home folder too");
        assert_eq!(lists_for_place(home, false, &list(&["$HOME", "&DESKTOP"]), &list(&["$HOME"]), &d), (list(&["&DESKTOP"]), list(&[])));
        assert_eq!(lists_for_place(home, true, &list(&[]), &list(&[]), &d), (list(&[]), list(&["$HOME"])));
        assert_eq!(lists_for_place(&all[2], true, &list(&[]), &list(&[]), &d), (list(&["&DOCUMENTS"]), list(&[])));
        assert!(enhanced(&json!("auto")) && enhanced(&Value::Null) && enhanced(&json!([])));
        assert!(!enhanced(&json!(["/home/u/Muziek"])));
    }
}
