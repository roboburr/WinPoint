//! Instellingen › Tijd en taal › Typen › Sneltoetsen (batch A, notes/claude/settings-parity.md §2, plan.json
//! "shortcuts", amendment 13): GNOME Settings 50's View and Customize Shortcuts in Windows 11's style. Windows has
//! no shortcut editor; this is WinPoint's page under Typen, with Windows' cards, expanders and wording.
//!
//! * "Sneltoetsen zoeken" filters every row (name and keys) and opens the sections with a match.
//! * Sections as expanders, in GNOME's order behind WinPoint's own: WinPoint (Taakweergave openen = the extension's
//!   `task-view`, Taakbeheer openen = `task-manager`, both editable; Start openen with the Windows key, a link to
//!   Geavanceerde toetsenbordinstellingen), Toegankelijkheid, Starters, Navigatie, Schermafbeeldingen, Geluid en
//!   media, Systeem, Typen, Vensters (`input.shortcuts`: GNOME Settings' own catalogue).
//! * A row shows its keys as key caps ("Uitgeschakeld" without); a click on them is GNOME's shortcut editor:
//!   `pickers::KeyCapture` (GNOME Shell captures the key, `input.capture`), Esc cancels, Backspace turns the
//!   shortcut off, a combination another shortcut has asks "Sneltoets vervangen?" (GNOME's Replace) and then writes
//!   with `replace`. The ⋯ menu: Sneltoets instellen, Sneltoets opnieuw instellen (when changed), Uitschakelen.
//! * An accelerator WinPoint holds (amendment 13: `<Alt>Tab` for Taakweergave, Ctrl+Alt+Delete for Taakbeheer) is
//!   named on the row it came from: "Alt + Tab: In gebruik door Taakweergave"; taking it back is the conflict with
//!   WinPoint's row.
//! * Aangepaste sneltoetsen: GNOME's custom shortcuts (Naam, Opdracht, Sneltoets), Sneltoets toevoegen, a row's
//!   dialog with Verwijderen.
//! * "Alle sneltoetsen opnieuw instellen" (GNOME's Reset All…, confirmed): GNOME's shortcuts only; WinPoint's and the
//!   custom ones stay.
//!
//! The page follows `input.changed {what: "shortcuts"}` (every write of this host) by building itself again; the
//! open sections and the search survive that (`OPEN`, `QUERY`). A deep link `{section: "typing"}` (Geavanceerde
//! toetsenbordinstellingen › Sneltoetsen voor invoertaal) opens that section.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::pickers::{Capture, KeyCapture};
use kit::controls::{self as c, Chevron, Dialog, DialogResult, MenuItem, SearchBox, SettingsCard, SettingsExpander, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("shortcuts", shortcuts)];
pub const CSS: &str = r#"
.st-shortcut-keys { padding: 2px 6px; min-height: 28px; }
.st-shortcut-off { color: var(--text-3); }
.st-shortcut-search { margin-bottom: 4px; }
"#;

const EXT: &str = "org.gnome.shell.extensions.win11-gnome";
const MEDIA: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const CUSTOM_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

thread_local! {
    /// The sections open on this page (kept over a rebuild).
    static OPEN: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    /// The search text (kept over a rebuild).
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
}

/// The sections: (id, glyph, nl, en).
const SECTIONS: &[(&str, &str, &str, &str)] = &[
    ("winpoint", "start", "WinPoint", "WinPoint"),
    ("accessibility", "accessibility", "Toegankelijkheid", "Accessibility"),
    ("launchers", "apps", "Starters", "Launchers"),
    ("navigation", "desktop", "Navigatie", "Navigation"),
    ("screenshots", "image", "Schermafbeeldingen", "Screenshots"),
    ("media", "music", "Geluid en media", "Sound & media"),
    ("system", "settings", "Systeem", "System"),
    ("typing", "keyboard", "Typen", "Typing"),
    ("windows", "window", "Vensters", "Windows"),
];

/// The shortcuts' names in Windows' wording (GNOME's descriptions are the fallback): (key, nl, en).
const NAMES: &[(&str, &str, &str)] = &[
    ("task-view", "Taakweergave openen", "Open Task View"),
    ("task-manager", "Taakbeheer openen", "Open Task Manager"),
    ("magnifier", "Vergrootglas in- of uitschakelen", "Turn Magnifier on or off"),
    ("magnifier-zoom-in", "Inzoomen", "Zoom in"),
    ("magnifier-zoom-out", "Uitzoomen", "Zoom out"),
    ("screenreader", "Verteller in- of uitschakelen", "Turn Narrator on or off"),
    ("on-screen-keyboard", "Schermtoetsenbord in- of uitschakelen", "Turn on-screen keyboard on or off"),
    ("increase-text-size", "Tekst groter maken", "Make text bigger"),
    ("decrease-text-size", "Tekst kleiner maken", "Make text smaller"),
    ("toggle-contrast", "Contrastthema in- of uitschakelen", "Turn contrast theme on or off"),
    ("help", "Help-browser starten", "Launch help browser"),
    ("control-center", "Instellingen", "Settings"),
    ("calculator", "Rekenmachine starten", "Launch calculator"),
    ("email", "E-mailprogramma starten", "Launch email app"),
    ("terminal", "Terminal starten", "Launch terminal"),
    ("www", "Webbrowser starten", "Launch web browser"),
    ("home", "Persoonlijke map", "Home folder"),
    ("search", "Zoeken", "Search"),
    ("move-to-workspace-1", "Venster naar bureaublad 1 verplaatsen", "Move window to desktop 1"),
    ("move-to-workspace-2", "Venster naar bureaublad 2 verplaatsen", "Move window to desktop 2"),
    ("move-to-workspace-3", "Venster naar bureaublad 3 verplaatsen", "Move window to desktop 3"),
    ("move-to-workspace-4", "Venster naar bureaublad 4 verplaatsen", "Move window to desktop 4"),
    ("move-to-workspace-last", "Venster naar het laatste bureaublad verplaatsen", "Move window to the last desktop"),
    ("move-to-workspace-left", "Venster één bureaublad naar links verplaatsen", "Move window one desktop to the left"),
    ("move-to-workspace-right", "Venster één bureaublad naar rechts verplaatsen", "Move window one desktop to the right"),
    ("move-to-monitor-left", "Venster één beeldscherm naar links verplaatsen", "Move window one display to the left"),
    ("move-to-monitor-right", "Venster één beeldscherm naar rechts verplaatsen", "Move window one display to the right"),
    ("move-to-monitor-up", "Venster één beeldscherm omhoog verplaatsen", "Move window one display up"),
    ("move-to-monitor-down", "Venster één beeldscherm omlaag verplaatsen", "Move window one display down"),
    ("switch-applications", "Schakelen tussen apps", "Switch apps"),
    ("switch-windows", "Schakelen tussen vensters", "Switch windows"),
    ("switch-group", "Schakelen tussen vensters van een app", "Switch windows of an app"),
    ("switch-panels", "Schakelen tussen systeembesturingselementen", "Switch system controls"),
    ("cycle-windows", "Direct schakelen tussen vensters", "Switch windows directly"),
    ("cycle-group", "Direct schakelen tussen vensters van een app", "Switch windows of an app directly"),
    ("cycle-panels", "Direct schakelen tussen systeembesturingselementen", "Switch system controls directly"),
    ("show-desktop", "Alle vensters verbergen", "Hide all windows"),
    ("switch-to-workspace-1", "Naar bureaublad 1 schakelen", "Switch to desktop 1"),
    ("switch-to-workspace-2", "Naar bureaublad 2 schakelen", "Switch to desktop 2"),
    ("switch-to-workspace-3", "Naar bureaublad 3 schakelen", "Switch to desktop 3"),
    ("switch-to-workspace-4", "Naar bureaublad 4 schakelen", "Switch to desktop 4"),
    ("switch-to-workspace-last", "Naar het laatste bureaublad schakelen", "Switch to the last desktop"),
    ("switch-to-workspace-left", "Naar het bureaublad links schakelen", "Switch to the desktop on the left"),
    ("switch-to-workspace-right", "Naar het bureaublad rechts schakelen", "Switch to the desktop on the right"),
    ("show-screenshot-ui", "Interactief een schermafbeelding maken", "Take a screenshot interactively"),
    ("screenshot", "Een schermafbeelding maken", "Take a screenshot"),
    ("screenshot-window", "Een schermafbeelding van een venster maken", "Take a screenshot of a window"),
    ("show-screen-recording-ui", "Interactief een schermopname maken", "Record the screen interactively"),
    ("volume-mute", "Geluid dempen of dempen opheffen", "Mute or unmute volume"),
    ("volume-down", "Volume omlaag", "Volume down"),
    ("volume-up", "Volume omhoog", "Volume up"),
    ("mic-mute", "Microfoon dempen of dempen opheffen", "Mute or unmute microphone"),
    ("media", "Mediaspeler starten", "Launch media player"),
    ("play", "Afspelen (of afspelen/onderbreken)", "Play (or play/pause)"),
    ("pause", "Afspelen onderbreken", "Pause playback"),
    ("stop", "Afspelen stoppen", "Stop playback"),
    ("previous", "Vorig nummer", "Previous track"),
    ("next", "Volgend nummer", "Next track"),
    ("eject", "Uitwerpen", "Eject"),
    ("logout", "Afmelden", "Sign out"),
    ("reboot", "Opnieuw opstarten", "Restart"),
    ("shutdown", "Afsluiten", "Shut down"),
    ("screensaver", "Scherm vergrendelen", "Lock screen"),
    ("toggle-message-tray", "De meldingenlijst weergeven", "Show the notification list"),
    ("toggle-quick-settings", "Snelle instellingen openen", "Open quick settings"),
    ("focus-active-notification", "De actieve melding selecteren", "Select the active notification"),
    ("toggle-overview", "Het activiteitenoverzicht weergeven", "Show the Activities overview"),
    ("toggle-application-view", "Alle apps weergeven", "Show all apps"),
    ("panel-run-dialog", "Het venster Uitvoeren weergeven", "Show the Run box"),
    ("restore-shortcuts", "De sneltoetsen terugzetten", "Restore the keyboard shortcuts"),
    ("switch-input-source", "Naar de volgende invoermethode schakelen", "Switch to the next input method"),
    ("switch-input-source-backward", "Naar de vorige invoermethode schakelen", "Switch to the previous input method"),
    ("activate-window-menu", "Het venstermenu openen", "Open the window menu"),
    ("toggle-fullscreen", "Volledig scherm in- of uitschakelen", "Turn full screen on or off"),
    ("toggle-maximized", "Venster maximaliseren of herstellen", "Maximise or restore window"),
    ("maximize", "Venster maximaliseren", "Maximise window"),
    ("unmaximize", "Venster herstellen", "Restore window"),
    ("close", "Venster sluiten", "Close window"),
    ("minimize", "Venster minimaliseren", "Minimise window"),
    ("begin-move", "Venster verplaatsen", "Move window"),
    ("begin-resize", "Formaat van venster wijzigen", "Resize window"),
    ("toggle-on-all-workspaces", "Venster op alle bureaubladen of op één weergeven", "Show window on all desktops or on one"),
    ("raise-or-lower", "Venster naar voren halen als het bedekt is, anders naar achteren", "Raise window if covered, otherwise lower it"),
    ("raise", "Venster boven andere vensters plaatsen", "Raise window above other windows"),
    ("lower", "Venster onder andere vensters plaatsen", "Lower window below other windows"),
    ("maximize-vertically", "Venster verticaal maximaliseren", "Maximise window vertically"),
    ("maximize-horizontally", "Venster horizontaal maximaliseren", "Maximise window horizontally"),
    ("toggle-tiled-left", "Venster links uitlijnen", "Snap window to the left"),
    ("toggle-tiled-right", "Venster rechts uitlijnen", "Snap window to the right"),
];

/// Key names on key caps, as Windows writes them: (keysym name in lower case, nl, en).
const KEY_NAMES: &[(&str, &str, &str)] = &[
    ("super", "Windows", "Windows"),
    ("control", "Ctrl", "Ctrl"),
    ("alt", "Alt", "Alt"),
    ("shift", "Shift", "Shift"),
    ("space", "Spatiebalk", "Space"),
    ("return", "Enter", "Enter"),
    ("kp_enter", "Enter", "Enter"),
    ("escape", "Esc", "Esc"),
    ("backspace", "Backspace", "Backspace"),
    ("delete", "Delete", "Delete"),
    ("insert", "Insert", "Insert"),
    ("tab", "Tab", "Tab"),
    ("page_up", "Page Up", "Page Up"),
    ("prior", "Page Up", "Page Up"),
    ("page_down", "Page Down", "Page Down"),
    ("next", "Page Down", "Page Down"),
    ("home", "Home", "Home"),
    ("end", "End", "End"),
    ("left", "Pijl-links", "Left arrow"),
    ("right", "Pijl-rechts", "Right arrow"),
    ("up", "Pijl-omhoog", "Up arrow"),
    ("down", "Pijl-omlaag", "Down arrow"),
    ("print", "Print Screen", "Print Screen"),
    ("above_tab", "Toets boven Tab", "Key above Tab"),
    ("menu", "Menutoets", "Menu key"),
];

/// The name of a shortcut in the UI language; `fallback` (GNOME's English description) for a key the table lacks.
pub(super) fn name_of(key: &str, fallback: &str, en: bool) -> String {
    NAMES.iter().find(|(k, _, _)| *k == key).map(|(_, nl, e)| if en { *e } else { *nl }).unwrap_or(fallback).to_owned()
}

fn modifier_name(m: &str) -> Option<&'static str> {
    match m.to_ascii_lowercase().as_str() {
        "shift" => Some("shift"),
        "control" | "ctrl" | "primary" => Some("control"),
        "alt" | "mod1" => Some("alt"),
        "super" | "mod4" => Some("super"),
        "hyper" => Some("hyper"),
        "meta" => Some("meta"),
        _ => None,
    }
}

/// A key's cap text: Windows' name, or the character it types (upper case), or the keysym's own name.
pub(super) fn key_label(name: &str, en: bool) -> String {
    let lower = name.to_ascii_lowercase();
    if let Some((_, nl, e)) = KEY_NAMES.iter().find(|(k, _, _)| *k == lower) {
        return (if en { *e } else { *nl }).to_owned();
    }
    if let Some(rest) = name.strip_prefix("XF86") {
        return rest.to_owned();
    }
    if let Some(rest) = name.strip_prefix("KP_") {
        return format!("Num {rest}");
    }
    if name.chars().count() == 1 {
        return name.to_uppercase();
    }
    if let Some(ch) = gtk::gdk::Key::from_name(name).and_then(|k| k.to_unicode()).filter(|c| !c.is_control() && !c.is_whitespace()) {
        return ch.to_uppercase().collect();
    }
    name.to_owned()
}

/// The key caps of an accelerator (modifiers first, in Windows' order: Windows, Ctrl, Alt, Shift); None for a text
/// that is no accelerator.
pub(super) fn accel_keys(accel: &str, en: bool) -> Option<Vec<String>> {
    let mut mods: Vec<&'static str> = Vec::new();
    let mut rest = accel;
    while let Some(after) = rest.strip_prefix('<') {
        let end = after.find('>')?;
        let m = modifier_name(&after[..end])?;
        if !mods.contains(&m) {
            mods.push(m);
        }
        rest = &after[end + 1..];
    }
    if rest.is_empty() || rest.contains(['<', '>']) {
        return None;
    }
    let mut keys: Vec<String> = Vec::new();
    for m in ["super", "control", "alt", "shift", "hyper", "meta"] {
        if mods.contains(&m) {
            keys.push(key_label(m, en));
        }
    }
    keys.push(key_label(rest, en));
    Some(keys)
}

/// An accelerator as text ("Windows + Spatiebalk"), the accelerator itself when it does not parse.
pub(super) fn accel_text(accel: &str, en: bool) -> String {
    accel_keys(accel, en).map(|k| k.join(" + ")).unwrap_or_else(|| accel.to_owned())
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).filter(|s| !s.is_empty()).collect()
}

/// Whether a search text matches (every word somewhere in `haystack`, case ignored).
pub(super) fn matches(query: &str, haystack: &str) -> bool {
    let hay = haystack.to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

/// "In gebruik door Taakweergave" for an accelerator WinPoint's `setting` holds.
fn held_text(setting: &str) -> &'static str {
    if setting == "task-manager" {
        t("In gebruik door Taakbeheer", "In use by Task Manager")
    } else {
        t("In gebruik door Taakweergave", "In use by Task View")
    }
}

/// The key caps of the first accelerator (and "+ n" for more), or "Uitgeschakeld".
fn keys_widget(accels: &[String]) -> gtk::Widget {
    let en = kit::i18n::en();
    match accels.first().and_then(|a| accel_keys(a, en)) {
        Some(keys) => {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let refs: Vec<&str> = keys.iter().map(String::as_str).collect();
            row.append(&common::kbd(&refs));
            if accels.len() > 1 {
                row.append(&c::label(&format!("+{}", accels.len() - 1), "w11-text-2", false));
            }
            row.upcast()
        }
        None => c::label(t("Uitgeschakeld", "Disabled"), "w11-text-2 st-shortcut-off", false).upcast(),
    }
}

/// What a row needs to be written again.
#[derive(Clone)]
struct Target {
    schema: String,
    key: String,
    name: String,
}

/// The conflict dialog's title and question for the other shortcut that has the keys.
fn conflict_question(conflict: &Value) -> (String, String) {
    let en = kit::i18n::en();
    let keys = accel_text(conflict["accel"].as_str().unwrap_or(""), en);
    let other = if conflict["id"].is_string() {
        conflict["name"].as_str().unwrap_or("").to_owned()
    } else {
        name_of(conflict["key"].as_str().unwrap_or(""), conflict["name"].as_str().unwrap_or(""), en)
    };
    if conflict["schema"] == EXT {
        let title = held_text(conflict["key"].as_str().unwrap_or("")).to_owned();
        let message = common::fill(
            t(
                "{keys} wordt door WinPoint gebruikt voor {other}. Als je deze vervangt, werkt {other} niet meer met deze sneltoets.",
                "{keys} is used by WinPoint for {other}. If you replace it, {other} no longer works with this shortcut.",
            ),
            &[("keys", &keys), ("other", &other)],
        );
        return (title, message);
    }
    let message = common::fill(
        t(
            "{keys} wordt al gebruikt voor {other}. Als je deze vervangt, werkt {other} niet meer met deze sneltoets.",
            "{keys} is already used for {other}. If you replace it, {other} no longer works with this shortcut.",
        ),
        &[("keys", &keys), ("other", &other)],
    );
    (t("Sneltoets vervangen?", "Replace shortcut?").to_owned(), message)
}

/// Write `args` with `method`; a conflict asks "Vervangen" and writes again with `replace`. `done(true)` after a
/// write that went through (a conflict that is not replaced, or an error: `done(false)`).
fn write_with_conflict(p: &Page, method: &'static str, args: Value, done: Rc<dyn Fn(bool)>) {
    let wait = p.run_with(method, args.clone(), None, false);
    let pp = p.clone();
    p.spawn(async move {
        let Some(v) = wait.await else {
            done(false);
            return;
        };
        if !v["conflict"].is_object() {
            done(true);
            return;
        }
        let (title, message) = conflict_question(&v["conflict"]);
        let p2 = pp.clone();
        c::confirm(&pp.anchor(), &title, &message, t("Vervangen", "Replace"), false, move |yes| {
            if !yes {
                done(false);
                return;
            }
            let mut again = args;
            again["replace"] = json!(true);
            write_with_conflict(&p2, method, again, done);
        });
    });
}

/// GNOME's shortcut editor for one row (module docs).
fn edit(p: &Page, target: &Target) {
    let title = common::fill(t("Sneltoets instellen voor {name}", "Set shortcut for {name}"), &[("name", &target.name)]);
    let (pp, target) = (p.clone(), target.clone());
    KeyCapture::run(p, &title, move |capture| {
        let accels = match capture {
            Capture::Accel(a) => vec![a],
            Capture::Disabled => Vec::new(),
            Capture::Cancelled => return,
        };
        let args = json!({"schema": target.schema, "key": target.key, "accels": accels});
        write_with_conflict(&pp, "input.shortcuts.set", args, Rc::new(|_| {}));
    });
}

/// The ⋯ button of a row.
fn more_button(name: &str, items: impl Fn() -> Vec<MenuItem> + 'static) -> gtk::Button {
    let more = c::icon_button("more", t("Meer opties", "More options"), 32);
    more.set_widget_name(name);
    more.connect_clicked(move |b| {
        c::show_menu_for(b, items());
    });
    more
}

/// The keys button of a row (a flat button with the key caps).
fn keys_button(name: &str, accels: &[String]) -> gtk::Button {
    let keys = gtk::Button::new();
    keys.add_css_class("flat");
    keys.add_css_class("st-shortcut-keys");
    keys.set_child(Some(&keys_widget(accels)));
    keys.set_widget_name(name);
    keys
}

/// One catalogued shortcut as a card; `(card, search text)`.
fn item_card(p: &Page, item: &Value) -> (SettingsCard, String) {
    let en = kit::i18n::en();
    let key = item["key"].as_str().unwrap_or("").to_owned();
    let name = name_of(&key, item["name"].as_str().unwrap_or(""), en);
    let accels = strings(&item["accels"]);
    let held = &item["held"];
    let description = if held.is_object() {
        format!("{}: {}", accel_text(held["accel"].as_str().unwrap_or(""), en), held_text(held["setting"].as_str().unwrap_or("")))
    } else {
        String::new()
    };
    let card = SettingsCard::new("", &name, &description);
    let target = Target { schema: item["schema"].as_str().unwrap_or("").to_owned(), key: key.clone(), name: name.clone() };
    let keys = keys_button(&format!("st-shortcuts-{key}"), &accels);
    let all: Vec<String> = accels.iter().map(|a| accel_text(a, en)).collect();
    let tip = if all.is_empty() { t("Klik om een sneltoets in te stellen", "Click to set a shortcut").to_owned() } else { all.join(", ") };
    keys.set_tooltip_text(Some(&tip));
    {
        let (pp, target) = (p.clone(), target.clone());
        keys.connect_clicked(move |_| edit(&pp, &target));
    }
    let is_default = item["isDefault"] == true;
    let has_keys = !accels.is_empty();
    let more = {
        let (pp, target) = (p.clone(), target.clone());
        more_button(&format!("st-shortcuts-{key}-more"), move || {
            let (p1, t1) = (pp.clone(), target.clone());
            let (p2, t2) = (pp.clone(), target.clone());
            let (p3, t3) = (pp.clone(), target.clone());
            vec![
                MenuItem::new(t("Sneltoets instellen…", "Set shortcut…"), move || edit(&p1, &t1)).glyph("keyboard"),
                MenuItem::new(t("Sneltoets opnieuw instellen", "Reset shortcut"), move || {
                    super::keyboards::fire(&p2, "input.shortcuts.reset", json!({"schema": t2.schema, "key": t2.key}));
                })
                .glyph("undo")
                .enabled(!is_default),
                MenuItem::new(t("Uitschakelen", "Turn off"), move || {
                    let args = json!({"schema": t3.schema, "key": t3.key, "accels": []});
                    write_with_conflict(&p3, "input.shortcuts.set", args, Rc::new(|_| {}));
                })
                .enabled(has_keys),
            ]
        })
    };
    if item["editable"] != true {
        for w in [keys.upcast_ref::<gtk::Widget>(), more.upcast_ref()] {
            w.set_sensitive(false);
            w.set_tooltip_text(Some(crate::gs::locked_text()));
        }
    }
    card.set_control(&keys);
    card.add_control(&more);
    (card, format!("{name} {} {description}", all.join(" ")))
}

/// The dialog of a custom shortcut: a new one (`existing` None: Toevoegen) or one that exists (Opslaan, Verwijderen).
fn custom_dialog(p: &Page, existing: Option<&Value>) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    let name = c::text_box("");
    name.set_widget_name("st-shortcuts-custom-name");
    let command = c::text_box(t("Bijvoorbeeld: gnome-terminal", "For example: gnome-terminal"));
    command.set_widget_name("st-shortcuts-custom-command");
    let binding = Rc::new(RefCell::new(String::new()));
    if let Some(e) = existing {
        name.set_text(e["name"].as_str().unwrap_or(""));
        command.set_text(e["command"].as_str().unwrap_or(""));
        *binding.borrow_mut() = e["binding"].as_str().unwrap_or("").to_owned();
    }
    let keys = gtk::Button::new();
    keys.set_widget_name("st-shortcuts-custom-binding");
    keys.set_halign(gtk::Align::Start);
    let show_keys: Rc<dyn Fn()> = {
        let (keys, binding) = (keys.clone(), binding.clone());
        Rc::new(move || {
            let current = binding.borrow().clone();
            if current.is_empty() {
                keys.set_label(t("Sneltoets instellen…", "Set shortcut…"));
            } else {
                keys.set_child(Some(&keys_widget(&[current])));
            }
        })
    };
    show_keys();
    {
        let (pp, binding, show_keys) = (p.clone(), binding.clone(), show_keys.clone());
        keys.connect_clicked(move |_| {
            let (binding, show_keys) = (binding.clone(), show_keys.clone());
            KeyCapture::run(&pp, t("Sneltoets instellen", "Set shortcut"), move |capture| {
                match capture {
                    Capture::Accel(a) => *binding.borrow_mut() = a,
                    Capture::Disabled => binding.borrow_mut().clear(),
                    Capture::Cancelled => return,
                }
                show_keys();
            });
        });
    }
    body.append(&c::field(t("Naam", "Name"), &name, None));
    body.append(&c::field(t("Opdracht", "Command"), &command, None));
    body.append(&c::field(t("Sneltoets", "Shortcut"), &keys, None));
    let error = c::label("", "w11-text-2", true);
    error.set_visible(false);
    body.append(&error);
    let id = existing.and_then(|e| e["id"].as_str()).map(str::to_owned);
    let title = if id.is_some() { t("Aangepaste sneltoets", "Custom shortcut") } else { t("Aangepaste sneltoets toevoegen", "Add custom shortcut") };
    let mut dialog = Dialog::new(title).body(&body).width(460).primary(if id.is_some() { t("Opslaan", "Save") } else { t("Toevoegen", "Add") });
    if id.is_some() {
        dialog = dialog.secondary(t("Verwijderen", "Remove"));
    }
    let dialog = {
        let (pp, id, name, command, binding, error) = (p.clone(), id.clone(), name.clone(), command.clone(), binding.clone(), error.clone());
        dialog.on_primary(move |d| {
            let (n, cmd) = (name.text().trim().to_owned(), command.text().trim().to_owned());
            if n.is_empty() || cmd.is_empty() {
                error.set_text(if n.is_empty() { t("Geef een naam op.", "Enter a name.") } else { t("Geef een opdracht op.", "Enter a command.") });
                error.set_visible(true);
                return false;
            }
            let mut args = json!({"name": n, "command": cmd, "binding": *binding.borrow()});
            let method = match &id {
                Some(id) => {
                    args["id"] = json!(id);
                    "input.shortcuts.custom.update"
                }
                None => "input.shortcuts.custom.add",
            };
            d.set_busy(true);
            let d2 = d.clone();
            let done: Rc<dyn Fn(bool)> = Rc::new(move |ok| {
                d2.set_busy(false);
                if ok {
                    d2.close(DialogResult::Primary);
                }
            });
            write_with_conflict(&pp, method, args, done);
            false
        })
    };
    let pp = p.clone();
    dialog.show(&p.anchor(), move |r| {
        if r == DialogResult::Secondary
            && let Some(id) = id
        {
            super::keyboards::fire(&pp, "input.shortcuts.custom.remove", json!({ "id": id }));
        }
    });
}

/// A custom shortcut as a card; `(card, search text)`.
fn custom_card(p: &Page, custom: &Value) -> (SettingsCard, String) {
    let en = kit::i18n::en();
    let id = custom["id"].as_str().unwrap_or("").to_owned();
    let name = custom["name"].as_str().unwrap_or("").to_owned();
    let command = custom["command"].as_str().unwrap_or("").to_owned();
    let binding = custom["binding"].as_str().unwrap_or("").to_owned();
    let card = SettingsCard::new("", &name, &command);
    let list: Vec<String> = if binding.is_empty() { Vec::new() } else { vec![binding.clone()] };
    let keys = keys_button(&format!("st-shortcuts-{id}"), &list);
    {
        let (pp, custom) = (p.clone(), custom.clone());
        keys.connect_clicked(move |_| custom_dialog(&pp, Some(&custom)));
    }
    let more = {
        let (pp, custom) = (p.clone(), custom.clone());
        more_button(&format!("st-shortcuts-{id}-more"), move || {
            let (p1, c1) = (pp.clone(), custom.clone());
            let (p2, id2) = (pp.clone(), custom["id"].as_str().unwrap_or("").to_owned());
            vec![
                MenuItem::new(t("Bewerken", "Edit"), move || custom_dialog(&p1, Some(&c1))).glyph("edit"),
                MenuItem::new(t("Verwijderen", "Remove"), move || {
                    super::keyboards::fire(&p2, "input.shortcuts.custom.remove", json!({ "id": id2 }));
                })
                .glyph("delete")
                .danger(),
            ]
        })
    };
    card.set_control(&keys);
    card.add_control(&more);
    (card, format!("{name} {command} {}", accel_text(&binding, en)))
}

/// A section's expander, remembering whether it is open.
fn section_expander(id: &str, glyph: &str, title: &str, count: usize) -> SettingsExpander {
    let description = common::plural(count as i64, t("{n} sneltoets", "{n} shortcut"), t("{n} sneltoetsen", "{n} shortcuts"));
    let exp = SettingsExpander::new(glyph, title, &description);
    exp.header_button().set_widget_name(&format!("st-shortcuts-section-{id}"));
    exp.set_expanded(OPEN.with(|o| o.borrow().contains(id)));
    let id = id.to_owned();
    exp.connect_toggled(move |open| {
        OPEN.with(|o| {
            let mut o = o.borrow_mut();
            if open {
                o.insert(id.clone());
            } else {
                o.remove(&id);
            }
        });
    });
    exp
}

/// Scroll the page's column so that `w` is near the top (a deep link to a section).
fn scroll_to(w: &gtk::Widget) {
    let Some(scroller) = w.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>() else {
        return;
    };
    let Some(child) = scroller.child() else {
        return;
    };
    if let Some(point) = w.compute_point(&child, &gtk::graphene::Point::new(0.0, 0.0)) {
        scroller.vadjustment().set_value((point.y() as f64 - 12.0).max(0.0));
    }
}

fn shortcuts(p: Page) -> PageFuture {
    Box::pin(async move {
        let answer = p.call("input.shortcuts", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let data = match answer {
            Ok(v) => v,
            Err(e) => {
                let card = common::unavailable_card("keyboard", t("Sneltoetsen zijn niet beschikbaar", "Keyboard shortcuts aren’t available"), Some(&e.message));
                return vec![card.widget().clone().upcast()];
            }
        };
        let en = kit::i18n::en();
        let wanted = p.take_param("section").and_then(|v| v.as_str().map(str::to_owned));
        if let Some(id) = &wanted {
            OPEN.with(|o| o.borrow_mut().insert(id.clone()));
        }
        let mut out: Content = Vec::new();

        let search = SearchBox::new(Some(t("Sneltoetsen zoeken", "Search shortcuts")));
        search.widget().add_css_class("st-shortcut-search");
        search.text_widget().set_widget_name("st-shortcuts-search");
        search.widget().set_halign(gtk::Align::Start);
        search.widget().set_size_request(320, -1);
        out.push(search.widget().clone().upcast());

        // every row with its search text and section, every section with its expander
        let rows: Rc<RefCell<Vec<(gtk::Widget, String, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let sections: Rc<RefCell<Vec<(String, SettingsExpander)>>> = Rc::new(RefCell::new(Vec::new()));
        let group = SettingsGroup::new("", "");
        for (id, glyph, nl, en_title) in SECTIONS {
            let Some(section) = data["sections"].as_array().into_iter().flatten().find(|s| s["id"] == *id) else {
                continue;
            };
            let items: Vec<&Value> = section["items"].as_array().into_iter().flatten().collect();
            let extra = usize::from(*id == "winpoint");
            let exp = section_expander(id, glyph, if en { en_title } else { nl }, items.len() + extra);
            for item in items {
                let (card, hay) = item_card(&p, item);
                exp.add(card.widget());
                rows.borrow_mut().push((card.widget().clone().upcast(), hay, (*id).to_owned()));
            }
            if *id == "winpoint" {
                // Start: the Windows key itself (start.windowsKey, amendment 58), on Geavanceerde toetsenbordinstellingen
                let start = SettingsCard::clickable(
                    "",
                    t("Start openen", "Open Start"),
                    t("Windows-toets. In- of uitschakelen bij Geavanceerde toetsenbordinstellingen", "Windows key. Turn it on or off in Advanced keyboard settings"),
                    Chevron::Right,
                );
                start.add_control(&common::kbd(&["Windows"]));
                if let Some(b) = start.button() {
                    b.set_widget_name("st-shortcuts-start");
                }
                let pp = p.clone();
                start.connect_clicked(move || pp.go("keyboard-advanced"));
                exp.add(start.widget());
                rows.borrow_mut().push((start.widget().clone().upcast(), format!("{} Windows", t("Start openen", "Open Start")), "winpoint".into()));
            }
            group.add(exp.widget());
            sections.borrow_mut().push(((*id).to_owned(), exp));
        }

        // Aangepaste sneltoetsen
        let custom: Vec<&Value> = data["custom"].as_array().into_iter().flatten().collect();
        let cexp = section_expander("custom", "edit", t("Aangepaste sneltoetsen", "Custom shortcuts"), custom.len());
        for item in &custom {
            let (card, hay) = custom_card(&p, item);
            cexp.add(card.widget());
            rows.borrow_mut().push((card.widget().clone().upcast(), hay, "custom".into()));
        }
        let add_card = SettingsCard::new(
            "",
            t("Een aangepaste sneltoets toevoegen", "Add a custom shortcut"),
            t("Een opdracht starten met een eigen toetsencombinatie", "Run a command with a key combination of your own"),
        );
        let add = c::button(t("Sneltoets toevoegen", "Add shortcut"), Variant::Standard);
        add.set_widget_name("st-shortcuts-custom-add");
        {
            let pp = p.clone();
            add.connect_clicked(move |_| custom_dialog(&pp, None));
        }
        add_card.set_control(&add);
        cexp.add(add_card.widget());
        group.add(cexp.widget());
        sections.borrow_mut().push(("custom".into(), cexp));
        out.push(group.widget().clone().upcast());

        // Alles opnieuw instellen
        let reset_group = SettingsGroup::new("", "");
        let reset_card = SettingsCard::new(
            "undo",
            t("Alle sneltoetsen opnieuw instellen", "Reset all shortcuts"),
            t("De sneltoetsen van GNOME op hun standaardwaarden zetten", "Set GNOME’s shortcuts to their defaults"),
        );
        let reset = c::button(t("Alles opnieuw instellen…", "Reset all…"), Variant::Standard);
        reset.set_widget_name("st-shortcuts-reset-all");
        {
            let pp = p.clone();
            reset.connect_clicked(move |_| {
                let p2 = pp.clone();
                c::confirm(
                    &pp.anchor(),
                    t("Alle sneltoetsen opnieuw instellen?", "Reset all shortcuts?"),
                    t(
                        "De sneltoetsen van GNOME krijgen weer hun standaardwaarden. Aangepaste sneltoetsen en de sneltoetsen van WinPoint blijven zoals ze zijn. Dit kan niet ongedaan worden gemaakt.",
                        "GNOME’s shortcuts get their defaults back. Custom shortcuts and WinPoint’s shortcuts stay as they are. This can’t be undone.",
                    ),
                    t("Alles opnieuw instellen", "Reset all"),
                    true,
                    move |yes| {
                        if yes {
                            super::keyboards::fire(&p2, "input.shortcuts.resetAll", json!({}));
                        }
                    },
                );
            });
        }
        reset_card.set_control(&reset);
        reset_group.add(reset_card.widget());
        out.push(reset_group.widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "keyboard-advanced").widget());
        related.add(
            common::link_card_with(
                &p,
                "a11y-keyboard",
                Some(t("Toegankelijkheid van toetsenbord", "Keyboard accessibility")),
                Some(t("Plaktoetsen, filtertoetsen, schermtoetsenbord", "Sticky keys, filter keys, on-screen keyboard")),
                None,
            )
            .widget(),
        );
        out.push(related.widget().clone().upcast());

        // the search: rows that match, their sections open; empty: everything, the sections as they were
        let filter: Rc<dyn Fn(&str)> = {
            let (rows, sections) = (rows.clone(), sections.clone());
            Rc::new(move |query: &str| {
                let query = query.trim();
                let mut hits: HashSet<String> = HashSet::new();
                for (w, hay, section) in rows.borrow().iter() {
                    let ok = query.is_empty() || matches(query, hay);
                    w.set_visible(ok);
                    if ok {
                        hits.insert(section.clone());
                    }
                }
                for (id, exp) in sections.borrow().iter() {
                    if query.is_empty() {
                        exp.widget().set_visible(true);
                        exp.set_expanded(OPEN.with(|o| o.borrow().contains(id)));
                    } else {
                        exp.widget().set_visible(hits.contains(id));
                        exp.set_expanded(hits.contains(id));
                    }
                }
            })
        };
        {
            let filter = filter.clone();
            search.connect_changed(move |text| {
                QUERY.with(|q| *q.borrow_mut() = text.to_owned());
                filter(text);
            });
        }
        let query = QUERY.with(|q| q.borrow().clone());
        if !query.is_empty() {
            search.set_text(&query);
            filter(&query);
        }
        if let Some(id) = wanted
            && let Some((_, exp)) = sections.borrow().iter().find(|(s, _)| *s == id)
        {
            let w: gtk::Widget = exp.widget().clone().upcast();
            p.timeout(250, move || scroll_to(&w));
        }
        let refresh = soon(&p);
        {
            let refresh = refresh.clone();
            p.on("input.changed", move |d| {
                if d["what"] == "shortcuts" {
                    refresh();
                }
            });
        }
        follow_elsewhere(&p, &data, refresh);
        out
    })
}

/// A rebuild of the page 300 ms from now, once however often it is asked for in that time (this host's own
/// `input.changed` and the `gsettings.changed` of the same write come together).
fn soon(p: &Page) -> Rc<dyn Fn()> {
    let pending = Rc::new(std::cell::Cell::new(false));
    let pp = p.clone();
    Rc::new(move || {
        if !pending.replace(true) {
            let p2 = pp.clone();
            pp.timeout(300, move || p2.refresh());
        }
    })
}

/// The schemas (and paths) of the shortcuts the page shows, one key of each: a `gsettings.get` of them opens them in
/// the host's gsettings service, which then reports their changes (`gsettings.changed`).
fn watched_keys(data: &Value) -> Vec<Value> {
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut keys = Vec::new();
    let mut add = |schema: &str, path: Option<&str>, key: &str| {
        if !schema.is_empty() && !key.is_empty() && seen.insert((schema.to_owned(), path.unwrap_or("").to_owned())) {
            let mut k = json!({ "schema": schema, "key": key });
            if let Some(path) = path {
                k["path"] = json!(path);
            }
            keys.push(k);
        }
    };
    for item in data["sections"].as_array().into_iter().flatten().flat_map(|s| s["items"].as_array().into_iter().flatten()) {
        add(common::s(&item["schema"]), item["path"].as_str(), common::s(&item["key"]));
    }
    add(MEDIA, None, "custom-keybindings");
    for item in data["custom"].as_array().into_iter().flatten() {
        let id = common::s(&item["id"]);
        if !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric()) {
            add(CUSTOM, Some(format!("{CUSTOM_PATH}{id}/").as_str()), "binding");
        }
    }
    keys
}

/// GNOME Settings' rows follow their keys, also a change made elsewhere (another program, gsettings): the page is
/// built again when a key of one of the shown schemas changes (the harness's settings-input.py writes Venster
/// sluiten from outside).
fn follow_elsewhere(p: &Page, data: &Value, refresh: Rc<dyn Fn()>) {
    let keys = watched_keys(data);
    let schemas: HashSet<String> = keys.iter().map(|k| common::s(&k["schema"]).to_owned()).collect();
    p.on("gsettings.changed", move |d| {
        if schemas.contains(common::s(&d["schema"])) {
            refresh();
        }
    });
    let wait = p.get("gsettings.get", json!({ "keys": keys }));
    p.spawn(async move {
        let _ = wait.await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_watches_one_key_of_every_schema_and_path_it_shows() {
        let data = json!({
            "sections": [
                {"id": "windows", "items": [
                    {"schema": "org.gnome.desktop.wm.keybindings", "key": "close"},
                    {"schema": "org.gnome.desktop.wm.keybindings", "key": "minimize"},
                ]},
                {"id": "system", "items": [{"schema": MEDIA, "key": "logout"}, {"schema": "", "key": "x"}]},
            ],
            "custom": [{"id": "custom0"}, {"id": "../x"}],
        });
        let keys = watched_keys(&data);
        assert_eq!(
            keys,
            [
                json!({"schema": "org.gnome.desktop.wm.keybindings", "key": "close"}),
                json!({"schema": MEDIA, "key": "logout"}),
                json!({"schema": CUSTOM, "key": "binding", "path": format!("{CUSTOM_PATH}custom0/")}),
            ],
            "one key per schema and path; the custom list's schema is media-keys itself; no odd custom id"
        );
    }

    /// gnome-control-center 50.3's visible shortcuts (host/src/services/input/shortcuts.rs BUILTIN without the
    /// hidden ones) and WinPoint's two.
    const VISIBLE: &[&str] = &[
        "volume-mute", "volume-down", "volume-up", "mic-mute", "media", "play", "pause", "stop", "previous", "next", "eject",
        "switch-input-source", "switch-input-source-backward", "help", "control-center", "calculator", "email", "terminal",
        "www", "home", "search", "logout", "reboot", "shutdown", "screensaver", "magnifier", "magnifier-zoom-in",
        "magnifier-zoom-out", "screenreader", "on-screen-keyboard", "increase-text-size", "decrease-text-size",
        "toggle-contrast", "show-screenshot-ui", "screenshot", "screenshot-window", "show-screen-recording-ui",
        "toggle-message-tray", "toggle-quick-settings", "focus-active-notification", "toggle-overview",
        "toggle-application-view", "move-to-workspace-1", "move-to-workspace-2", "move-to-workspace-3",
        "move-to-workspace-4", "move-to-workspace-last", "move-to-workspace-left", "move-to-workspace-right",
        "move-to-monitor-left", "move-to-monitor-right", "move-to-monitor-up", "move-to-monitor-down",
        "switch-applications", "switch-windows", "switch-group", "switch-panels", "cycle-windows", "cycle-group",
        "cycle-panels", "show-desktop", "switch-to-workspace-1", "switch-to-workspace-2", "switch-to-workspace-3",
        "switch-to-workspace-4", "switch-to-workspace-last", "switch-to-workspace-left", "switch-to-workspace-right",
        "panel-run-dialog", "restore-shortcuts", "activate-window-menu", "toggle-fullscreen", "toggle-maximized",
        "maximize", "unmaximize", "close", "minimize", "begin-move", "begin-resize", "toggle-on-all-workspaces",
        "raise-or-lower", "raise", "lower", "maximize-vertically", "maximize-horizontally", "toggle-tiled-left",
        "toggle-tiled-right", "task-view", "task-manager",
    ];

    #[test]
    fn every_visible_shortcut_has_a_name_in_both_languages() {
        assert_eq!(VISIBLE.len(), 89);
        for key in VISIBLE {
            let row = NAMES.iter().find(|(k, _, _)| k == key);
            assert!(row.is_some_and(|(_, nl, en)| !nl.is_empty() && !en.is_empty()), "{key}");
        }
        let keys: HashSet<&str> = NAMES.iter().map(|(k, _, _)| *k).collect();
        assert_eq!(keys.len(), NAMES.len(), "one name per key");
        assert_eq!(name_of("close", "Close window", false), "Venster sluiten");
        assert_eq!(name_of("close", "Close window", true), "Close window");
        assert_eq!(name_of("new-in-gnome-51", "GNOME text", false), "GNOME text", "the host's name for a new key");
    }

    #[test]
    fn accelerators_read_as_windows_writes_keys() {
        assert_eq!(accel_keys("<Super>space", false).unwrap(), ["Windows", "Spatiebalk"]);
        assert_eq!(accel_keys("<Super>space", true).unwrap(), ["Windows", "Space"]);
        assert_eq!(accel_keys("<Shift><Control><Alt>r", true).unwrap(), ["Ctrl", "Alt", "Shift", "R"], "Windows' order");
        assert_eq!(accel_keys("<Primary><Alt>Delete", false).unwrap(), ["Ctrl", "Alt", "Delete"]);
        assert_eq!(accel_keys("<Super><Shift>Page_Up", true).unwrap(), ["Windows", "Shift", "Page Up"]);
        assert_eq!(accel_keys("<Super>Prior", true).unwrap(), ["Windows", "Page Up"], "xkb's spelling too");
        assert_eq!(accel_keys("<Super>Left", false).unwrap(), ["Windows", "Pijl-links"]);
        assert_eq!(accel_keys("XF86AudioMute", true).unwrap(), ["AudioMute"]);
        assert_eq!(accel_keys("<Alt>F4", true).unwrap(), ["Alt", "F4"]);
        assert_eq!(accel_keys("<Super>Above_Tab", false).unwrap(), ["Windows", "Toets boven Tab"]);
        assert_eq!(accel_keys("<Mod4>l", true).unwrap(), ["Windows", "L"]);
        assert_eq!(accel_keys("<Nope>x", true), None);
        assert_eq!(accel_keys("<Super>", true), None);
        assert_eq!(accel_text("<Control><Alt>t", false), "Ctrl + Alt + T");
        assert_eq!(accel_text("garbage<", false), "garbage<");
    }

    #[test]
    fn the_search_matches_every_word_in_any_case() {
        assert!(matches("venster sluiten", "Venster sluiten Alt + F4"));
        assert!(matches("f4", "Venster sluiten Alt + F4"));
        assert!(matches("ALT tab", "Schakelen tussen apps Windows + Tab Alt + Tab: In gebruik door Taakweergave"));
        assert!(!matches("venster openen", "Venster sluiten"));
        assert!(matches("", "anything"));
    }

    #[test]
    fn sections_follow_gnomes_order_behind_winpoints() {
        let ids: Vec<&str> = SECTIONS.iter().map(|s| s.0).collect();
        assert_eq!(ids, ["winpoint", "accessibility", "launchers", "navigation", "screenshots", "media", "system", "typing", "windows"]);
    }
}
