//! Instellingen › Systeem › Multitasking (batch D, notes/claude/settings-parity.md §2, amendments 11 and 12): Windows 11's
//! Multitasking page on GNOME Settings' Multitasking functions, in Windows' order:
//!
//! * Vensters uitlijnen = mutter's edge-tiling (GNOME's Window Resize; WinPoint's own snap layouts have no settings).
//! * Bureaubladen, Windows' two rows (amendment 12): "Op de taakbalk alle geopende vensters weergeven" =
//!   `taskbar.allDesktops` (WinPoint's taskbar), "Alle geopende vensters weergeven wanneer ik op Alt+Tab druk" =
//!   `taskView.allDesktops` (Alt+Tab is WinPoint's Taakweergave, native/shell/src/flyouts/menus/taskview.rs) with the
//!   same choice in GNOME's app and window switchers (`current-workspace-only`, inverted). The row is bound to GNOME
//!   Settings' App Switching key (app-switcher current-workspace-only): it shows that key and follows its outside
//!   changes, and taskView.allDesktops follows it (at the page's start too), so WinPoint's Alt+Tab and GNOME's
//!   Super+Tab agree; then GNOME's Workspaces (dynamic or a fixed number) and Multi-Monitor (main display only or all).
//! * Last, a WinPoint addition (amendment 11): Actieve hoek (enable-hot-corners), which only GNOME's own and Ubuntu's
//!   sessions use (WinPoint's desktop turns GNOME's hot corner off).
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("multitasking", multitasking)];
pub const CSS: &str = "";

const EDGE: GKey = gs::key("org.gnome.mutter", "edge-tiling");
const DYNAMIC: GKey = gs::key("org.gnome.mutter", "dynamic-workspaces");
const PRIMARY: GKey = gs::key("org.gnome.mutter", "workspaces-only-on-primary");
const NUMBER: GKey = gs::key("org.gnome.desktop.wm.preferences", "num-workspaces");
const HOT: GKey = gs::key("org.gnome.desktop.interface", "enable-hot-corners");
const APPS: GKey = gs::key("org.gnome.shell.app-switcher", "current-workspace-only");
const WINDOWS: GKey = gs::key("org.gnome.shell.window-switcher", "current-workspace-only");
fn keys() -> Vec<GKey> {
    vec![EDGE, DYNAMIC, PRIMARY, NUMBER, HOT, APPS, WINDOWS]
}

/// Windows' choices of both Bureaubladen rows: false = only the desktop in use, true = all desktops.
fn desktop_options() -> Vec<(bool, &'static str)> {
    vec![
        (false, t("Alleen op het bureaublad dat ik gebruik", "Only on the desktop I’m using")),
        (true, t("Op alle bureaubladen", "On all desktops")),
    ]
}

/// What Windows' Alt+Tab row writes into GNOME's switchers (amendment 12): current-workspace-only is "not all
/// desktops", for the keys that can take it (a locked or missing key is left as it is).
pub(crate) fn switcher_writes(all: bool, writable: &[bool; 2]) -> Option<Value> {
    let writes: Vec<Value> = [&APPS, &WINDOWS]
        .iter()
        .zip(writable)
        .filter(|(_, ok)| **ok)
        .map(|(k, _)| json!({ "schema": k.schema, "values": { k.key: !all } }))
        .collect();
    (!writes.is_empty()).then(|| json!({ "writes": writes }))
}

/// The Alt+Tab row: what it shows for GNOME's app-switcher current-workspace-only (`apps`, None when the key is
/// locked or missing) and WinPoint's taskView.allDesktops (`setting`), and the taskView.allDesktops to write so the two
/// agree. GNOME's key wins, as at the mirror's first start (amendment 47): the row is bound to it, and writing GNOME's
/// key from a page that only opened would change the GNOME session. Without the key the row is taskView.allDesktops.
pub(crate) fn alt_tab_choice(apps: Option<bool>, setting: bool) -> (bool, Option<bool>) {
    match apps {
        Some(current_only) => (!current_only, (setting == current_only).then_some(!current_only)),
        None => (setting, None),
    }
}

fn dropdown_of(selected: bool) -> Dropdown {
    let options = desktop_options().into_iter().map(|(v, l)| DropdownOption::new(&v.to_string(), l)).collect();
    Dropdown::new(options, Some(&selected.to_string()))
}

/// "Op de taakbalk alle geopende vensters weergeven": WinPoint's own taskbar.allDesktops.
fn taskbar_row(p: &Page) -> SettingsCard {
    let options = desktop_options().into_iter().map(|(v, l)| (json!(v), l.to_owned())).collect();
    let dd = common::setting_dropdown(p, "taskbar.allDesktops", options, Some(json!(false)));
    dd.widget().set_widget_name("st-multitasking-taskbar-all-desktops");
    let card = SettingsCard::new("", t("Op de taakbalk alle geopende vensters weergeven", "On the taskbar, show all the open windows"), "");
    card.set_control(&common::sized(dd.widget(), 360));
    card
}

/// "Alle geopende vensters weergeven wanneer ik op Alt+Tab druk": taskView.allDesktops and GNOME's switchers.
fn alt_tab_row(p: &Page, gs: &Gs) -> SettingsCard {
    let all = || common::setting_bool("taskView.allDesktops", false);
    let gnome = |gs: &Gs| if gs.writable(&APPS) { gs.value(&APPS).as_bool() } else { None };
    let align = {
        let pp = p.clone();
        move |on: bool| common::set_setting(&pp, "taskView.allDesktops", json!(on), |_| {})
    };
    let (shown, write) = alt_tab_choice(gnome(gs), all());
    let dd = dropdown_of(shown);
    dd.widget().set_widget_name(&gs::control_name(p.id(), &keys(), &APPS));
    if let Some(on) = write {
        align(on);
    }
    {
        let (pp, gs2, dd2) = (p.clone(), gs.clone(), dd.clone());
        dd.connect_changed(move |value| {
            let on = value == "true";
            let dd3 = dd2.clone();
            common::set_setting(&pp, "taskView.allDesktops", json!(on), move |ok| {
                if !ok {
                    dd3.set_selected(Some(&common::setting_bool("taskView.allDesktops", false).to_string()));
                }
            });
            if let Some(args) = switcher_writes(on, &[gs2.writable(&APPS), gs2.writable(&WINDOWS)]) {
                let wait = pp.run_with("gsettings.set", args, None, true);
                pp.spawn(async move {
                    let _ = wait.await;
                });
            }
        });
    }
    {
        let dd2 = dd.clone();
        p.on_settings(move |_, _| dd2.set_selected(Some(&all().to_string())));
    }
    {
        // an outside change of GNOME's key (gsettings, GNOME Settings' App Switching in the Ubuntu session)
        let (gs2, dd2) = (gs.clone(), dd.clone());
        gs.follow(&APPS, move |_| {
            let (shown, write) = alt_tab_choice(gnome(&gs2), all());
            dd2.set_selected(Some(&shown.to_string()));
            if let Some(on) = write {
                align(on);
            }
        });
    }
    let card = SettingsCard::new("", t("Alle geopende vensters weergeven wanneer ik op Alt+Tab druk", "Show all open windows when I press Alt+Tab"), "");
    card.set_control(&common::sized(dd.widget(), 360));
    card
}

/// Bureaubladen: Windows' two rows, then GNOME's workspaces as WinPoint additions.
fn desktops(p: &Page, gs: &Gs) -> SettingsExpander {
    let exp = SettingsExpander::new(
        "taskview",
        t("Bureaubladen", "Desktops"),
        t("Geopende vensters op de taakbalk en bij Alt+Tab, het aantal bureaubladen", "Open windows on the taskbar and with Alt+Tab, the number of desktops"),
    );
    exp.header_button().set_widget_name("st-multitasking-desktops");
    exp.add(taskbar_row(p).widget());
    exp.add(alt_tab_row(p, gs).widget());
    if gs.available(&DYNAMIC) {
        let radio = gs.radio(
            &DYNAMIC,
            vec![
                (json!(true), t("Lege bureaubladen automatisch verwijderen", "Remove empty desktops automatically").to_owned()),
                (json!(false), t("Vast aantal bureaubladen", "Fixed number of desktops").to_owned()),
            ],
        );
        exp.add(radio.widget());
        if gs.available(&NUMBER) {
            let card = SettingsCard::new("", t("Aantal bureaubladen", "Number of desktops"), "");
            card.set_control(&gs.number(&NUMBER, 1.0, 36.0, 1.0));
            card.widget().set_visible(gs.value(&DYNAMIC) == false);
            let w = card.widget().clone();
            gs.follow(&DYNAMIC, move |v| w.set_visible(*v == false));
            exp.add(card.widget());
        }
    }
    if gs.available(&PRIMARY) {
        let radio = gs.radio(
            &PRIMARY,
            vec![
                (json!(true), t("Bureaubladen alleen op het hoofdbeeldscherm", "Desktops on the main display only").to_owned()),
                (json!(false), t("Bureaubladen op alle beeldschermen", "Desktops on all displays").to_owned()),
            ],
        );
        exp.add(radio.widget());
    }
    exp
}

fn multitasking(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &keys()).await;
        if !p.alive() {
            return Content::new();
        }
        let group = SettingsGroup::new("", "");
        if let Some(card) = gs.toggle_card(
            &EDGE,
            "snap",
            t("Vensters uitlijnen", "Snap windows"),
            t(
                "Sleep vensters tegen de boven-, linker- of rechterrand van het scherm om hun formaat te wijzigen",
                "Drag windows against the top, left or right edge of the screen to resize them",
            ),
            false,
        ) {
            group.add(card.widget());
        }
        group.add(desktops(&p, &gs).widget());
        if let Some(card) = gs.toggle_card(
            &HOT,
            "pc",
            t("Actieve hoek", "Hot corner"),
            t(
                "Raak de linkerbovenhoek aan om het activiteitenoverzicht te openen. Geldt in de GNOME- en Ubuntu-sessie.",
                "Touch the top-left corner to open the Activities overview. Applies in the GNOME and Ubuntu sessions.",
            ),
            false,
        ) {
            group.add(card.widget());
        }
        vec![group.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alt_tab_writes_the_inverse_into_both_gnome_switchers() {
        let all = switcher_writes(true, &[true, true]).unwrap();
        assert_eq!(
            all,
            json!({"writes": [
                {"schema": "org.gnome.shell.app-switcher", "values": {"current-workspace-only": false}},
                {"schema": "org.gnome.shell.window-switcher", "values": {"current-workspace-only": false}},
            ]})
        );
        let one = switcher_writes(false, &[false, true]).unwrap();
        assert_eq!(one["writes"].as_array().unwrap().len(), 1, "a locked key is left out");
        assert_eq!(one["writes"][0]["values"]["current-workspace-only"], true);
        assert_eq!(switcher_writes(true, &[false, false]), None);
        assert_eq!(gs::control_name("multitasking", &keys(), &APPS), "st-multitasking-app-switcher-current-workspace-only");
        assert_eq!(gs::control_name("multitasking", &keys(), &NUMBER), "st-multitasking-num-workspaces");
    }

    #[test]
    fn the_alt_tab_row_follows_gnomes_app_switching_and_taskview_follows_it() {
        // review D-7, the first start: GNOME's default (all workspaces) against WinPoint's (only this desktop)
        assert_eq!(alt_tab_choice(Some(false), false), (true, Some(true)), "GNOME's key wins, taskView.allDesktops follows");
        assert_eq!(alt_tab_choice(Some(false), true), (true, None), "they agree: nothing to write");
        // an outside gsettings set ... current-workspace-only true
        assert_eq!(alt_tab_choice(Some(true), true), (false, Some(false)));
        assert_eq!(alt_tab_choice(Some(true), false), (false, None));
        // a locked or missing key: the row is WinPoint's own setting
        assert_eq!(alt_tab_choice(None, true), (true, None));
        assert_eq!(alt_tab_choice(None, false), (false, None));
    }
}
