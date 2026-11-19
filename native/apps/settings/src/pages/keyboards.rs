//! Instellingen › Tijd en taal › Taal en regio › Taalopties and Tijd en taal › Typen › Geavanceerde
//! toetsenbordinstellingen (batch A, notes/claude/settings-parity.md §2, plan.json "language-options",
//! "keyboard-advanced"): GNOME Settings 50's Keyboard panel in the order and wording of Windows 11.
//!
//! **Taalopties** (Windows' Language options › Keyboards; GNOME's Input Sources):
//! * Toetsenborden: "Geïnstalleerde toetsenborden" with "Een toetsenbord toevoegen", then one card per input source
//!   (`org.gnome.desktop.input-sources sources`, written only through `input.sources.set`, which also brings the
//!   system keyboard along as Ubuntu's GNOME Settings does). Its ⋯ menu is GNOME's row menu: Omhoog, Omlaag,
//!   Indeling weergeven (tecla, `input.preview`, while `caps.tecla`), Opties (an IBus engine's setup, started by
//!   GNOME Shell: `input.engineSetup`), Verwijderen. Without sources: "Geen toetsenborden ingesteld: de indeling van
//!   het systeem wordt gebruikt".
//! * "Een toetsenbord toevoegen" is GNOME's Add Input Source: "Zoeken op taal of land", first the layouts of the UI
//!   language and of the languages already set, "Meer weergeven" for all of xkeyboard-config's layouts and the IBus
//!   engines (`input.layouts`, `input.engines`).
//!
//! **Geavanceerde toetsenbordinstellingen** (Windows' Advanced keyboard settings; GNOME's Keyboard panel):
//! * Standaardinvoermethode overschrijven: the first input source (GNOME's order: the chosen one moves to the front).
//! * Schakelen tussen invoermethoden: "Voor elk app-venster een andere invoermethode gebruiken" = `per-window`;
//!   "Sneltoetsen voor invoertaal" opens Sneltoetsen at its Typen section.
//! * WinPoint additions from GNOME: Speciale tekens invoeren (Toets voor alternatieve tekens = `lv3:*`, Samenstellingstoets
//!   = `compose:*`, both through `input.options.set`, every other xkb option kept), Tekenherhaling (Windows' Keyboard
//!   Properties: `repeat`, `delay` 2000-100 ms, `repeat-interval` 110-15 ms, a test box), Windows-toets ("Windows-toets
//!   opent Start" = WinPoint's `start.windowsKey`, amendment 58; GNOME's `overlay-key` as its own row, GNOME session
//!   only).
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, MenuItem, SearchBox, SettingsCard, SettingsGroup, Slider, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("language-options", language_options), ("keyboard-advanced", keyboard_advanced)];
pub const CSS: &str = r#"
.st-kb-choices { border: 1px solid var(--stroke-card); border-radius: var(--radius-card); background-color: var(--bg-card); }
.st-kb-choice { padding: 6px 10px; }
.st-kb-plain { margin-bottom: 6px; }
"#;

const IS: &str = "org.gnome.desktop.input-sources";
const KEYBOARD: &str = "org.gnome.desktop.peripherals.keyboard";
const SOURCES: GKey = gs::key(IS, "sources");
const XKB: GKey = gs::key(IS, "xkb-options");
const PER_WINDOW: GKey = gs::key(IS, "per-window");
const SWITCH: GKey = gs::key("org.gnome.desktop.wm.keybindings", "switch-input-source");
const REPEAT: GKey = gs::key(KEYBOARD, "repeat");
const DELAY: GKey = gs::key(KEYBOARD, "delay");
const INTERVAL: GKey = gs::key(KEYBOARD, "repeat-interval");
const OVERLAY: GKey = gs::key("org.gnome.mutter", "overlay-key");

// ------------------------------------------------------------------------------------------ the catalogue

/// One thing "Een toetsenbord toevoegen" offers.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Choice {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    /// The line under the name: the languages (an IBus engine: "Invoermethode · language").
    pub sub: String,
    pub languages: Vec<String>,
    /// Everything the search looks in: name, id, languages, countries.
    pub search: String,
}

/// What the page knows of the layouts and engines (`input.layouts`, `input.engines`).
#[derive(Clone, Debug, Default)]
pub(super) struct Catalogue {
    pub choices: Vec<Choice>,
    /// An engine's layout (for Indeling weergeven) and whether it has a setup program.
    pub engines: HashMap<String, (Option<String>, bool)>,
}

impl Catalogue {
    pub(super) fn from(layouts: &Value, engines: &Value) -> Catalogue {
        let names = |list: &Value| -> HashMap<String, String> {
            list.as_array().into_iter().flatten().filter_map(|l| Some((l["code"].as_str()?.to_owned(), l["name"].as_str()?.to_owned()))).collect()
        };
        let (languages, countries) = (names(&layouts["languages"]), names(&layouts["countries"]));
        let mut choices: Vec<Choice> = Vec::new();
        for l in layouts["layouts"].as_array().into_iter().flatten() {
            let (Some(id), Some(name)) = (l["id"].as_str(), l["name"].as_str()) else {
                continue;
            };
            let langs: Vec<String> = l["languages"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(str::to_owned)).collect();
            let lang_names: Vec<&str> = langs.iter().filter_map(|c| languages.get(c).map(String::as_str)).collect();
            let country_names: Vec<&str> = l["countries"].as_array().into_iter().flatten().filter_map(|c| countries.get(c.as_str()?).map(String::as_str)).collect();
            choices.push(Choice {
                kind: "xkb",
                id: id.to_owned(),
                name: name.to_owned(),
                sub: lang_names.join(", "),
                search: format!("{name} {id} {} {}", lang_names.join(" "), country_names.join(" ")),
                languages: langs,
            });
        }
        let mut map = HashMap::new();
        for e in engines["engines"].as_array().into_iter().flatten() {
            let (Some(id), Some(name)) = (e["id"].as_str(), e["name"].as_str()) else {
                continue;
            };
            let language = e["languageName"].as_str().unwrap_or("");
            let sub = if language.is_empty() { t("Invoermethode", "Input method").to_owned() } else { format!("{} · {language}", t("Invoermethode", "Input method")) };
            map.insert(id.to_owned(), (e["layout"].as_str().map(str::to_owned), e["setup"] == true));
            choices.push(Choice {
                kind: "ibus",
                id: id.to_owned(),
                name: name.to_owned(),
                sub,
                search: format!("{name} {id} {language}"),
                languages: e["language"].as_str().map(|l| vec![l.to_owned()]).unwrap_or_default(),
            });
        }
        Catalogue { choices, engines: map }
    }

    /// The name of an input source (its id when the catalogue does not know it).
    pub(super) fn name(&self, kind: &str, id: &str) -> String {
        self.choices.iter().find(|c| c.kind == kind && c.id == id).map(|c| c.name.clone()).unwrap_or_else(|| id.to_owned())
    }

    /// The layout tecla shows for a source: an xkb source's own id, an IBus engine's layout.
    pub(super) fn layout_of(&self, kind: &str, id: &str) -> Option<String> {
        match kind {
            "xkb" => Some(id.to_owned()),
            _ => self.engines.get(id).and_then(|(layout, _)| layout.clone()),
        }
    }

    pub(super) fn has_setup(&self, kind: &str, id: &str) -> bool {
        kind == "ibus" && self.engines.get(id).is_some_and(|(_, setup)| *setup)
    }
}

/// The input sources of `sources` (`[[type, id], ...]`).
pub(super) fn sources_of(v: &Value) -> Vec<(String, String)> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| Some((p[0].as_str()?.to_owned(), p[1].as_str()?.to_owned())))
        .collect()
}

fn sources_json(list: &[(String, String)]) -> Value {
    json!({ "sources": list.iter().map(|(k, i)| json!([k, i])).collect::<Vec<_>>() })
}

const ENGLISH_CODES: &[&str] = &["eng"];
const DUTCH_CODES: &[&str] = &["nld", "dut"];

/// The ISO 639 codes of the UI language that open the add dialog's first list (xkeyboard-config's three letters).
fn ui_languages(en: bool) -> &'static [&'static str] {
    if en { ENGLISH_CODES } else { DUTCH_CODES }
}

/// The add dialog's list: without a search the suggested choices (the UI language's and those of `languages`) unless
/// `all`; with one every choice that matches; never one already set. `(choices, more)`: whether "Meer weergeven"
/// has anything more to show.
pub(super) fn visible_choices(cat: &Catalogue, query: &str, all: bool, set: &[(String, String)], languages: &[String], en: bool) -> (Vec<Choice>, bool) {
    let free = |c: &&Choice| !set.iter().any(|(k, i)| k == c.kind && *i == c.id);
    let query = query.trim();
    if !query.is_empty() {
        let found: Vec<Choice> = cat.choices.iter().filter(free).filter(|c| super::shortcuts::matches(query, &c.search)).take(300).cloned().collect();
        return (found, false);
    }
    if all {
        return (cat.choices.iter().filter(free).cloned().collect(), false);
    }
    let suggested = |c: &&Choice| c.languages.iter().any(|l| ui_languages(en).contains(&l.as_str()) || languages.contains(l));
    let list: Vec<Choice> = cat.choices.iter().filter(free).filter(suggested).cloned().collect();
    let more = cat.choices.iter().filter(free).count() > list.len();
    (list, more)
}

fn tecla() -> bool {
    common::caps()["tecla"] == true
}

// ------------------------------------------------------------------------------------------ Taalopties

/// Run a host method from a click: its error toast (or the Testmodus toast) when it answers.
pub(super) fn fire(p: &Page, method: &str, args: Value) {
    let wait = p.run(method, args);
    p.spawn(async move {
        let _ = wait.await;
    });
}

/// Write the sources (the page follows the key and builds itself again).
fn set_sources(p: &Page, list: &[(String, String)]) {
    fire(p, "input.sources.set", sources_json(list));
}

fn choice_row(c: &Choice) -> gtk::ListBoxRow {
    let b = gtk::Box::new(gtk::Orientation::Vertical, 2);
    b.add_css_class("st-kb-choice");
    b.append(&c::label(&c.name, "w11-body", false));
    if !c.sub.is_empty() {
        b.append(&c::label(&c.sub, "w11-caption w11-text-2", false));
    }
    let row = gtk::ListBoxRow::new();
    row.set_child(Some(&b));
    let name: String = c.id.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' }).collect();
    row.set_widget_name(&format!("st-language-options-choice-{}-{name}", c.kind));
    row
}

/// "Een toetsenbord toevoegen" (GNOME's Add Input Source).
fn add_dialog(p: &Page, cat: Rc<Catalogue>, set: Vec<(String, String)>) {
    let en = kit::i18n::en();
    let languages: Vec<String> = set.iter().flat_map(|(k, i)| cat.choices.iter().find(|c| c.kind == k && c.id == *i).map(|c| c.languages.clone()).unwrap_or_default()).collect();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let search = SearchBox::new(Some(t("Zoeken op taal of land", "Search by language or country")));
    search.text_widget().set_widget_name("st-language-options-search");
    search.widget().set_hexpand(true);
    body.append(search.widget());
    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_widget_name("st-language-options-choices");
    let scroller = gtk::ScrolledWindow::new();
    scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
    scroller.set_min_content_height(320);
    scroller.set_max_content_height(320);
    scroller.add_css_class("st-kb-choices");
    scroller.set_child(Some(&list));
    body.append(&scroller);
    let shown: Rc<RefCell<Vec<Choice>>> = Rc::new(RefCell::new(Vec::new()));
    let chosen: Rc<RefCell<Option<Choice>>> = Rc::new(RefCell::new(None));
    let all = Rc::new(Cell::new(false));
    let dialog = Dialog::new(t("Een toetsenbord toevoegen", "Add a keyboard")).body(&body).width(520).primary(t("Toevoegen", "Add"));
    let fill: Rc<dyn Fn()> = {
        let (cat, list, shown, chosen, all, search, set, dialog) = (cat.clone(), list.clone(), shown.clone(), chosen.clone(), all.clone(), search.clone(), set.clone(), dialog.clone());
        Rc::new(move || {
            while let Some(row) = list.first_child() {
                list.remove(&row);
            }
            *chosen.borrow_mut() = None;
            dialog.set_primary_enabled(false);
            let (choices, more) = visible_choices(&cat, &search.text(), all.get(), &set, &languages, en);
            for ch in &choices {
                list.append(&choice_row(ch));
            }
            if choices.is_empty() {
                let empty = gtk::ListBoxRow::new();
                empty.set_selectable(false);
                empty.set_activatable(false);
                empty.set_child(Some(&c::label(t("Geen toetsenborden gevonden", "No keyboards found"), "w11-text-2 st-kb-choice", false)));
                list.append(&empty);
            }
            if more {
                let row = gtk::ListBoxRow::new();
                row.set_selectable(false);
                row.set_child(Some(&c::label(t("Meer weergeven", "Show more"), "w11-link st-kb-choice", false)));
                row.set_widget_name("st-language-options-more");
                list.append(&row);
            }
            *shown.borrow_mut() = choices;
        })
    };
    {
        let (shown, chosen, dialog, all, fill) = (shown.clone(), chosen.clone(), dialog.clone(), all.clone(), fill.clone());
        list.connect_row_activated(move |_, row| {
            let i = row.index() as usize;
            let count = shown.borrow().len();
            if i >= count && !all.get() && row.widget_name() == "st-language-options-more" {
                all.set(true);
                fill();
                return;
            }
            if let Some(ch) = shown.borrow().get(i).cloned() {
                *chosen.borrow_mut() = Some(ch);
                dialog.close(DialogResult::Primary);
            }
        });
    }
    {
        let (shown, chosen, dialog) = (shown.clone(), chosen.clone(), dialog.clone());
        list.connect_row_selected(move |_, row| {
            let ch = row.and_then(|r| shown.borrow().get(r.index() as usize).cloned());
            dialog.set_primary_enabled(ch.is_some());
            *chosen.borrow_mut() = ch;
        });
    }
    {
        let fill = fill.clone();
        search.connect_changed(move |_| fill());
    }
    let pp = p.clone();
    dialog.show(&p.anchor(), move |r| {
        if r != DialogResult::Primary {
            return;
        }
        if let Some(ch) = chosen.borrow().clone() {
            let mut list = set;
            list.push((ch.kind.to_owned(), ch.id));
            set_sources(&pp, &list);
        }
    });
    fill();
    search.focus();
}

/// One input source's card with its ⋯ menu.
fn source_card(p: &Page, cat: &Rc<Catalogue>, set: &[(String, String)], i: usize, writable: bool) -> SettingsCard {
    let (kind, id) = set[i].clone();
    let name = cat.name(&kind, &id);
    let description = if kind == "ibus" { t("Invoermethode", "Input method") } else { t("Toetsenbordindeling", "Keyboard layout") };
    let card = SettingsCard::new(if kind == "ibus" { "language" } else { "keyboard" }, &name, description);
    let more = c::icon_button("more", t("Meer opties", "More options"), 32);
    more.set_widget_name(&format!("st-language-options-{i}-more"));
    let (pp, cat, set) = (p.clone(), cat.clone(), set.to_vec());
    more.connect_clicked(move |b| {
        let n = set.len();
        let moved = |from: usize, to: usize| {
            let mut l = set.clone();
            l.swap(from, to);
            l
        };
        let (p1, up) = (pp.clone(), if i > 0 { moved(i, i - 1) } else { Vec::new() });
        let (p2, down) = (pp.clone(), if i + 1 < n { moved(i, i + 1) } else { Vec::new() });
        let (p3, layout) = (pp.clone(), cat.layout_of(&kind, &id));
        let (p4, engine) = (pp.clone(), id.clone());
        let (p5, rest) = (pp.clone(), set.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, s)| s.clone()).collect::<Vec<_>>());
        let mut items = vec![
            MenuItem::new(t("Omhoog", "Move up"), move || set_sources(&p1, &up)).glyph("arrow-up").enabled(i > 0).name("st-language-options-up"),
            MenuItem::new(t("Omlaag", "Move down"), move || set_sources(&p2, &down)).glyph("arrow-down").enabled(i + 1 < n).name("st-language-options-down"),
        ];
        if tecla() && let Some(layout) = layout {
            items.push(
                MenuItem::new(t("Indeling weergeven", "Show layout"), move || fire(&p3, "input.preview", json!({ "layout": layout })))
                    .glyph("keyboard")
                    .name("st-language-options-preview"),
            );
        }
        if cat.has_setup(&kind, &id) {
            items.push(MenuItem::new(t("Opties", "Options"), move || fire(&p4, "input.engineSetup", json!({ "id": engine }))).glyph("settings").name("st-language-options-setup"));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new(t("Verwijderen", "Remove"), move || set_sources(&p5, &rest)).glyph("delete").name("st-language-options-remove"));
        c::show_menu_for(b, items);
    });
    if !writable {
        more.set_sensitive(false);
        more.set_tooltip_text(Some(gs::locked_text()));
    }
    card.set_control(&more);
    card
}

/// The key of a service row may be written (lockdown) by its host method.
fn method_writable(gs: &Gs, k: &GKey) -> bool {
    gs.available(k) && gs.item(k)["writable"] == true
}

/// Build the page again when GNOME's value changes away from the one shown (an add, a remove, another program).
/// The page keeps `gs` until it goes: Gs::load's subscription holds it only weakly, and no control of Taalopties
/// holds a clone (without this the first harness run saw the page stay as it was after a write).
fn follow_sources(p: &Page, gs: &Gs, shown: Value) {
    let pp = p.clone();
    gs.follow(&SOURCES, move |v| {
        if *v != shown {
            pp.refresh();
        }
    });
    let keep = gs.clone();
    p.on_cleanup(move || drop(keep));
}

async fn catalogue(p: &Page) -> Rc<Catalogue> {
    let (layouts, engines) = (p.get("input.layouts", json!({})), p.get("input.engines", json!({})));
    let (layouts, engines) = (layouts.await.unwrap_or(Value::Null), engines.await.unwrap_or(Value::Null));
    Rc::new(Catalogue::from(&layouts, &engines))
}

fn language_options(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = [SOURCES, SWITCH];
        let (gs, cat) = (Gs::load(&p, &keys), catalogue(&p));
        let (gs, cat) = (gs.await, cat.await);
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        if !gs.available(&SOURCES) {
            out.push(common::unavailable_card("keyboard", t("Toetsenborden", "Keyboards"), Some(ct::unavailable_here())).widget().clone().upcast());
            return out;
        }
        let set = sources_of(&gs.value(&SOURCES));
        let writable = method_writable(&gs, &SOURCES);
        let group = SettingsGroup::new(t("Toetsenborden", "Keyboards"), "");
        let head = SettingsCard::new("keyboard", t("Geïnstalleerde toetsenborden", "Installed keyboards"), "");
        let add = c::button(t("Een toetsenbord toevoegen", "Add a keyboard"), Variant::Standard);
        add.set_widget_name("st-language-options-add");
        if !writable {
            add.set_sensitive(false);
            add.set_tooltip_text(Some(gs::locked_text()));
        }
        {
            let (pp, cat, set) = (p.clone(), cat.clone(), set.clone());
            add.connect_clicked(move |_| add_dialog(&pp, cat.clone(), set.clone()));
        }
        head.set_control(&add);
        group.add(head.widget());
        for i in 0..set.len() {
            group.add(source_card(&p, &cat, &set, i, writable).widget());
        }
        out.push(group.widget().clone().upcast());
        if set.is_empty() {
            out.push(common::notice(t("Geen toetsenborden ingesteld: de indeling van het systeem wordt gebruikt.", "No keyboards set: the system layout is used.")).widget().clone().upcast());
        } else if set.len() > 1 {
            let keys = super::shortcuts::accel_text(gs.value(&SWITCH)[0].as_str().unwrap_or(""), kit::i18n::en());
            if !keys.is_empty() {
                let line = common::fill(t("Schakel tussen toetsenborden met {keys}.", "Switch between keyboards with {keys}."), &[("keys", &keys)]);
                out.push(c::label(&line, "w11-text-2", true).upcast());
            }
        }
        follow_sources(&p, &gs, gs.value(&SOURCES));

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "keyboard-advanced").widget());
        related.add(common::link_card(&p, "typing").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

// ------------------------------------------------------------------------------------------ Geavanceerde toetsenbordinstellingen

/// Vertraging voor herhalen, Windows' Lang (1) to Kort (20), from GNOME's delay (ms): 2100 - 100 n.
pub(super) fn delay_ui(v: &Value) -> f64 {
    ((2100.0 - v.as_f64().unwrap_or(500.0)) / 100.0).round().clamp(1.0, 20.0)
}

pub(super) fn delay_gs(n: f64) -> Value {
    json!(2100 - (n.round() as i64).clamp(1, 20) * 100)
}

/// Herhalingssnelheid, Windows' Langzaam (1) to Snel (20), from GNOME's repeat-interval (ms): 115 - 5 n.
pub(super) fn rate_ui(v: &Value) -> f64 {
    ((115.0 - v.as_f64().unwrap_or(30.0)) / 5.0).round().clamp(1.0, 20.0)
}

pub(super) fn rate_gs(n: f64) -> Value {
    json!(115 - (n.round() as i64).clamp(1, 20) * 5)
}

/// The `lv3:` or `compose:` option of xkb-options (GNOME keeps at most one of each).
pub(super) fn option_of(v: &Value, prefix: &str) -> Option<String> {
    v.as_array()?.iter().filter_map(|o| o.as_str()).find(|o| o.starts_with(prefix)).map(str::to_owned)
}

/// Toets voor alternatieve tekens: (option or None for the layout's default, nl, en); `lv3:ralt_alt` is GNOME's off.
const LV3: &[(Option<&str>, &str, &str)] = &[
    (None, "Standaard van de indeling", "Layout default"),
    (Some("lv3:ralt_alt"), "Geen", "None"),
    (Some("lv3:lalt_switch"), "Linker Alt", "Left Alt"),
    (Some("lv3:ralt_switch"), "Rechter Alt", "Right Alt"),
    (Some("lv3:lwin_switch"), "Linker Windows-toets", "Left Windows key"),
    (Some("lv3:rwin_switch"), "Rechter Windows-toets", "Right Windows key"),
    (Some("lv3:menu_switch"), "Menutoets", "Menu key"),
    (Some("lv3:switch"), "Rechter Ctrl", "Right Ctrl"),
];

/// Samenstellingstoets: (option or None for off, nl, en).
const COMPOSE: &[(Option<&str>, &str, &str)] = &[
    (None, "Uit", "Off"),
    (Some("compose:ralt"), "Rechter Alt", "Right Alt"),
    (Some("compose:lwin"), "Linker Windows-toets", "Left Windows key"),
    (Some("compose:rwin"), "Rechter Windows-toets", "Right Windows key"),
    (Some("compose:menu"), "Menutoets", "Menu key"),
    (Some("compose:lctrl"), "Linker Ctrl", "Left Ctrl"),
    (Some("compose:rctrl"), "Rechter Ctrl", "Right Ctrl"),
    (Some("compose:caps"), "Caps Lock", "Caps Lock"),
    (Some("compose:sclk"), "Scroll Lock", "Scroll Lock"),
    (Some("compose:prsc"), "Print Screen", "Print Screen"),
    (Some("compose:ins"), "Insert", "Insert"),
];

const NONE_VALUE: &str = "none";

/// A drop-down for one kind of xkb option, written through `input.options.set {arg: option | null}`.
fn option_dropdown(p: &Page, gs: &Gs, prefix: &'static str, arg: &'static str, table: &'static [(Option<&'static str>, &'static str, &'static str)]) -> Dropdown {
    let en = kit::i18n::en();
    let current = option_of(&gs.value(&XKB), prefix);
    let mut options: Vec<DropdownOption> = table.iter().map(|(v, nl, e)| DropdownOption::new(v.unwrap_or(NONE_VALUE), if en { e } else { nl })).collect();
    // an option GNOME's list does not have (set elsewhere) stays a choice of its own
    if let Some(other) = current.as_deref().filter(|o| !table.iter().any(|(v, _, _)| *v == Some(*o))) {
        options.push(DropdownOption::new(other, other));
    }
    let dd = Dropdown::new(options, Some(current.as_deref().unwrap_or(NONE_VALUE)));
    dd.widget().set_widget_name(&format!("st-keyboard-advanced-{arg}"));
    if !method_writable(gs, &XKB) {
        dd.widget().set_sensitive(false);
        dd.widget().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (pp, gs2, dd2) = (p.clone(), gs.clone(), dd.clone());
        dd.connect_changed(move |choice| {
            let value = if choice == NONE_VALUE { Value::Null } else { json!(choice) };
            let wait = pp.run("input.options.set", json!({ arg: value }));
            let (gs3, dd3) = (gs2.clone(), dd2.clone());
            pp.spawn(async move {
                if wait.await.is_none() {
                    dd3.set_selected(Some(option_of(&gs3.value(&XKB), prefix).as_deref().unwrap_or(NONE_VALUE)));
                }
            });
        });
    }
    let dd2 = dd.clone();
    gs.follow(&XKB, move |v| dd2.set_selected(Some(option_of(v, prefix).as_deref().unwrap_or(NONE_VALUE))));
    dd
}

/// A slider between two words (Windows' classic dialogs: Lang · Kort, Langzaam · Snel).
fn between(slider: &Slider, left: &str, right: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.append(&c::label(left, "w11-text-2", false));
    slider.widget().set_hexpand(true);
    row.append(slider.widget());
    row.append(&c::label(right, "w11-text-2", false));
    row.set_size_request(320, -1);
    row
}

/// The overlay key opens something unless it is empty.
fn overlay_on(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.is_empty())
}

fn overlay_value(on: bool) -> Value {
    json!(if on { "Super" } else { "" })
}

fn keyboard_advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = vec![SOURCES, XKB, PER_WINDOW, SWITCH, REPEAT, DELAY, INTERVAL, OVERLAY];
        let (gs, cat) = (Gs::load(&p, &keys), catalogue(&p));
        let (gs, cat) = (gs.await, cat.await);
        if !p.alive() {
            return Content::new();
        }
        let en = kit::i18n::en();
        let mut out: Content = Vec::new();

        // Standaardinvoermethode overschrijven (Windows' plain heading and drop-down)
        let top = gtk::Box::new(gtk::Orientation::Vertical, 8);
        top.add_css_class("st-kb-plain");
        top.append(&common::section_title(t("Standaardinvoermethode overschrijven", "Override for default input method")));
        top.append(&c::label(
            t(
                "Als je een andere invoermethode wilt gebruiken dan de eerste in je lijst met toetsenborden, kies je die hier",
                "If you want to use an input method that’s different from the first one in your list of keyboards, choose it here",
            ),
            "w11-text-2",
            true,
        ));
        let set = sources_of(&gs.value(&SOURCES));
        let options: Vec<DropdownOption> = if set.is_empty() {
            vec![DropdownOption::new("0", t("De indeling van het systeem", "The system layout"))]
        } else {
            set.iter().enumerate().map(|(i, (k, id))| DropdownOption::new(&i.to_string(), &cat.name(k, id))).collect()
        };
        let default = Dropdown::new(options, Some("0"));
        default.widget().set_widget_name("st-keyboard-advanced-default");
        default.widget().set_halign(gtk::Align::Start);
        default.widget().set_size_request(360, -1);
        if set.len() < 2 || !method_writable(&gs, &SOURCES) {
            default.widget().set_sensitive(false);
        }
        {
            let (pp, set) = (p.clone(), set.clone());
            default.connect_changed(move |choice| {
                let Ok(i) = choice.parse::<usize>() else { return };
                if i == 0 || i >= set.len() {
                    return;
                }
                let mut list = set.clone();
                let first = list.remove(i);
                list.insert(0, first);
                set_sources(&pp, &list);
            });
        }
        top.append(default.widget());
        follow_sources(&p, &gs, gs.value(&SOURCES));

        // Schakelen tussen invoermethoden
        top.append(&common::section_title(t("Schakelen tussen invoermethoden", "Switching input methods")));
        if gs.available(&PER_WINDOW) {
            top.append(&gs.checkbox(&PER_WINDOW, t("Voor elk app-venster een andere invoermethode gebruiken", "Let me use a different input method for each app window"), false));
        }
        let hotkeys = c::hyperlink(t("Sneltoetsen voor invoertaal", "Input language hot keys"), None);
        hotkeys.set_widget_name("st-keyboard-advanced-hotkeys");
        hotkeys.set_halign(gtk::Align::Start);
        {
            let pp = p.clone();
            hotkeys.connect_clicked(move |_| pp.go_with("shortcuts", json!({ "section": "typing" })));
        }
        top.append(&hotkeys);
        let switch_keys = gs.value(&SWITCH)[0].as_str().map(|a| super::shortcuts::accel_text(a, en)).unwrap_or_default();
        if !switch_keys.is_empty() {
            top.append(&c::label(&common::fill(t("Nu: {keys}", "Now: {keys}"), &[("keys", &switch_keys)]), "w11-caption w11-text-2", true));
        }
        out.push(top.upcast());

        // WinPoint additions from GNOME: Speciale tekens invoeren
        if gs.available(&XKB) {
            let special = SettingsGroup::new(t("Speciale tekens invoeren", "Special character entry"), "");
            let lv3_dd = option_dropdown(&p, &gs, "lv3:", "lv3", LV3);
            let compose_dd = option_dropdown(&p, &gs, "compose:", "compose", COMPOSE);
            let lv3 = SettingsCard::new(
                "keyboard",
                t("Toets voor alternatieve tekens", "Alternate characters key"),
                t("Typ de extra tekens die soms als derde teken op een toets staan", "Type the extra characters sometimes printed as a third one on a key"),
            );
            lv3.set_control(&common::sized(lv3_dd.widget(), 240));
            special.add(lv3.widget());
            let compose = SettingsCard::new(
                "edit",
                t("Samenstellingstoets", "Compose key"),
                t("Druk op de samenstellingstoets en daarna op een reeks tekens, bijvoorbeeld o en c voor ©", "Press the compose key and then a sequence of characters, for example o and c for ©"),
            );
            compose.set_control(&common::sized(compose_dd.widget(), 240));
            special.add(compose.widget());
            out.push(special.widget().clone().upcast());
        }

        // Tekenherhaling (Windows' Keyboard Properties)
        let repeat = SettingsGroup::new(t("Tekenherhaling", "Character repeat"), "");
        let sliders: Rc<RefCell<Vec<gtk::Widget>>> = Rc::new(RefCell::new(Vec::new()));
        let repeat_on = gs.value(&REPEAT) != false;
        if let Some(card) = gs.toggle_card(&REPEAT, "keyboard", t("Toetsen herhalen", "Repeat keys"), t("Een toets die ingedrukt blijft, typt zijn teken steeds opnieuw", "A key that’s held down types its character again and again"), false) {
            repeat.add(card.widget());
        }
        if gs.available(&DELAY) {
            let s = gs.slider(&DELAY, 1.0, 20.0, 1.0, delay_ui, delay_gs);
            let card = SettingsCard::new("timer", t("Vertraging voor herhalen", "Repeat delay"), "");
            card.set_control(&between(&s, t("Lang", "Long"), t("Kort", "Short")));
            sliders.borrow_mut().push(card.widget().clone().upcast());
            repeat.add(card.widget());
        }
        if gs.available(&INTERVAL) {
            let s = gs.slider(&INTERVAL, 1.0, 20.0, 1.0, rate_ui, rate_gs);
            let card = SettingsCard::new("timer", t("Herhalingssnelheid", "Repeat rate"), "");
            card.set_control(&between(&s, t("Langzaam", "Slow"), t("Snel", "Fast")));
            sliders.borrow_mut().push(card.widget().clone().upcast());
            repeat.add(card.widget());
        }
        let test = SettingsCard::new("", t("Klik hier en houd een toets ingedrukt om de herhalingssnelheid te testen", "Click here and hold down a key to test repeat rate"), "");
        let entry = c::text_box("");
        entry.set_widget_name("st-keyboard-advanced-test");
        test.set_control(&common::sized(&entry, 240));
        sliders.borrow_mut().push(test.widget().clone().upcast());
        repeat.add(test.widget());
        for w in sliders.borrow().iter() {
            w.set_sensitive(repeat_on);
        }
        {
            let sliders = sliders.clone();
            gs.follow(&REPEAT, move |v| {
                for w in sliders.borrow().iter() {
                    w.set_sensitive(*v != false);
                }
            });
        }
        out.push(repeat.widget().clone().upcast());

        // De Windows-toets: WinPoint's own setting (amendment 58) and GNOME's overlay key
        let win = SettingsGroup::new(t("Windows-toets", "Windows key"), "");
        let (start, toggle) = common::toggle_card(
            &p,
            "start",
            t("Windows-toets opent Start", "Windows key opens Start"),
            t("Met Plaktoetsen aan opent de Windows-toets Start niet", "With Sticky Keys on, the Windows key doesn’t open Start"),
            "start.windowsKey",
            false,
        );
        toggle.switch().set_widget_name("st-keyboard-advanced-windows-key");
        win.add(start.widget());
        if gs.available(&OVERLAY) {
            let sw = super::mouse::mapped_toggle(&gs, &keys, &OVERLAY, overlay_on, overlay_value);
            let card = SettingsCard::new(
                "taskview",
                t("Windows-toets opent het activiteitenoverzicht", "Windows key opens the Activities overview"),
                t("Alleen GNOME-sessie. Als dit uit staat, opent de Windows-toets ook in WinPoint niets", "GNOME session only. When this is off, the Windows key opens nothing in WinPoint either"),
            );
            card.set_control(sw.widget());
            win.add(card.widget());
        }
        out.push(win.widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "language-options").widget());
        related.add(common::link_card(&p, "shortcuts").widget());
        related.add(
            common::link_card_with(&p, "a11y-keyboard", Some(t("Toegankelijkheid van toetsenbord", "Keyboard accessibility")), Some(t("Plaktoetsen, filtertoetsen, schermtoetsenbord", "Sticky keys, filter keys, on-screen keyboard")), None)
                .widget(),
        );
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Catalogue {
        let layouts = json!({
            "layouts": [
                {"id": "us", "name": "English (US)", "short": "en", "languages": ["eng"], "countries": ["US"]},
                {"id": "us+intl", "name": "English (US, intl., with dead keys)", "short": "en", "languages": ["eng"], "countries": ["US"]},
                {"id": "nl", "name": "Dutch", "short": "nl", "languages": ["nld"], "countries": ["NL"]},
                {"id": "de", "name": "German", "short": "de", "languages": ["deu"], "countries": ["DE"]},
            ],
            "languages": [{"code": "eng", "name": "Engels"}, {"code": "nld", "name": "Nederlands"}, {"code": "deu", "name": "Duits"}],
            "countries": [{"code": "DE", "name": "Duitsland"}, {"code": "NL", "name": "Nederland"}, {"code": "US", "name": "Verenigde Staten"}],
        });
        let engines = json!({"engines": [{"id": "anthy", "name": "Anthy", "language": "ja", "languageName": "Japans", "layout": "jp", "setup": true}]});
        Catalogue::from(&layouts, &engines)
    }

    #[test]
    fn the_catalogue_names_sources_and_knows_their_layouts() {
        let cat = fixture();
        assert_eq!(cat.choices.len(), 5);
        assert_eq!(cat.name("xkb", "nl"), "Dutch");
        assert_eq!(cat.name("xkb", "fr+bepo"), "fr+bepo", "unknown: the id");
        assert_eq!(cat.layout_of("xkb", "us+intl").as_deref(), Some("us+intl"));
        assert_eq!(cat.layout_of("ibus", "anthy").as_deref(), Some("jp"));
        assert!(cat.has_setup("ibus", "anthy") && !cat.has_setup("xkb", "us"));
        let anthy = cat.choices.iter().find(|c| c.id == "anthy").unwrap();
        assert!(anthy.sub.ends_with("Japans"), "{}", anthy.sub);
        assert!(cat.choices.iter().find(|c| c.id == "de").unwrap().search.contains("Duitsland"), "countries are searched too");
    }

    #[test]
    fn the_add_dialog_suggests_the_ui_language_and_searches_everything() {
        let cat = fixture();
        let set = vec![("xkb".to_owned(), "us".to_owned())];
        let (nl, more) = visible_choices(&cat, "", false, &set, &[], false);
        let ids: Vec<&str> = nl.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["nl"], "Dutch first in nl; us is set already");
        assert!(more);
        let (with_set, _) = visible_choices(&cat, "", false, &set, &["eng".into()], false);
        assert!(with_set.iter().any(|c| c.id == "us+intl"), "the languages already set are suggested too");
        let (en, _) = visible_choices(&cat, "", false, &[], &[], true);
        assert!(en.iter().all(|c| c.languages.contains(&"eng".to_owned())));
        let (all, more) = visible_choices(&cat, "", true, &set, &[], false);
        assert_eq!((all.len(), more), (4, false));
        let (found, _) = visible_choices(&cat, "duitsland", false, &set, &[], false);
        assert_eq!(found.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["de"]);
        let (found, _) = visible_choices(&cat, "japans", false, &set, &[], false);
        assert_eq!(found[0].kind, "ibus");
    }

    #[test]
    fn repeat_sliders_run_as_windows_draws_them() {
        assert_eq!(delay_gs(1.0), json!(2000), "Lang");
        assert_eq!(delay_gs(20.0), json!(100), "Kort");
        assert_eq!(delay_ui(&json!(500)), 16.0, "GNOME's default");
        assert_eq!(rate_gs(1.0), json!(110), "Langzaam");
        assert_eq!(rate_gs(20.0), json!(15), "Snel");
        assert_eq!(rate_ui(&json!(30)), 17.0, "GNOME's default");
        assert_eq!(rate_ui(&json!(5)), 20.0, "faster than the slider: its end");
        for n in 1..=20 {
            assert_eq!(delay_ui(&delay_gs(n as f64)), n as f64);
            assert_eq!(rate_ui(&rate_gs(n as f64)), n as f64);
        }
    }

    #[test]
    fn xkb_options_give_each_kind_its_choice() {
        let v = json!(["caps:ctrl_modifier", "lv3:ralt_switch", "compose:menu"]);
        assert_eq!(option_of(&v, "lv3:").as_deref(), Some("lv3:ralt_switch"));
        assert_eq!(option_of(&v, "compose:").as_deref(), Some("compose:menu"));
        assert_eq!(option_of(&json!([]), "lv3:"), None);
        // the choices are the host's (host/src/services/input/sources.rs LV3, COMPOSE)
        assert_eq!(LV3.iter().filter(|(v, _, _)| v.is_some()).count(), 7);
        assert_eq!(COMPOSE.iter().filter(|(v, _, _)| v.is_some()).count(), 10);
        assert!(overlay_on(&json!("Super")) && overlay_on(&json!("Super_R")) && !overlay_on(&json!("")));
        assert_eq!((overlay_value(true), overlay_value(false)), (json!("Super"), json!("")));
    }

    #[test]
    fn sources_read_and_write_as_gnome_stores_them() {
        let v = json!([["xkb", "us+intl"], ["ibus", "anthy"], ["broken"]]);
        let list = sources_of(&v);
        assert_eq!(list, [("xkb".to_owned(), "us+intl".to_owned()), ("ibus".to_owned(), "anthy".to_owned())]);
        assert_eq!(sources_json(&list), json!({"sources": [["xkb", "us+intl"], ["ibus", "anthy"]]}));
    }
}
