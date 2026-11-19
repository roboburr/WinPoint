//! Instellingen › Systeem › Opslag › Opslaginzicht (batch D, notes/claude/settings-parity.md §2, amendment 21): Windows
//! 11's Storage Sense page on GNOME Settings' File History & Trash functions (org.gnome.desktop.privacy, run by
//! gsd-housekeeping every hour):
//!
//! * Opschonen van tijdelijke bestanden: remove-old-temp-files.
//! * Automatisch opschonen van gebruikersinhoud: the Opslaginzicht switch sets both remove-old-trash-files and
//!   remove-old-temp-files and is on while either is (also on Opslag, [`master_switch`]).
//! * Opschoonschema's configureren: "Bestanden in mijn Prullenbak verwijderen als ze er langer in staan dan" with
//!   Windows' presets Nooit, 1, 14, 30, 60 dagen (Nooit = remove-old-trash-files off, a day count = on with
//!   old-files-age), GNOME's "1 uur" (age 0) as an extra, and a current value outside them as its own option; GNOME
//!   uses the same age for temporary files, the row says so.
//! * Opslaginzicht nu uitvoeren: storage.cleanTemp {sense: true} (gsd-housekeeping's purge, now).
//! * GNOME's two buttons below it: Prullenbak leegmaken (files.emptyTrash) and Tijdelijke bestanden verwijderen
//!   (storage.cleanTemp, gsd-housekeeping's RemoveTempFiles), each after a question.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Dropdown, DropdownOption, SettingsCard, SettingsGroup, Severity, Toast, ToggleSwitch};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("storagesense", storagesense)];
pub const CSS: &str = "";

const PRIVACY: &str = "org.gnome.desktop.privacy";
pub(super) const TRASH: GKey = gs::key(PRIVACY, "remove-old-trash-files");
pub(super) const TEMP: GKey = gs::key(PRIVACY, "remove-old-temp-files");
const AGE: GKey = gs::key(PRIVACY, "old-files-age");
fn keys() -> Vec<GKey> {
    vec![TRASH, TEMP, AGE]
}

/// Windows' presets of the Prullenbak row (days); GNOME's "1 uur" (0) is offered too.
const PRESETS: &[u64] = &[0, 1, 14, 30, 60];

fn age_label(days: u64) -> String {
    if days == 0 { t("1 uur", "1 hour").to_owned() } else { common::plural(days as i64, t("1 dag", "1 day"), t("{n} dagen", "{n} days")) }
}

/// The options of the Prullenbak row for the current keys: Nooit, the presets and a current age outside them, and
/// the one selected ("never" or the age).
pub(crate) fn trash_options(trash_on: bool, age: Option<u64>) -> (Vec<(String, String)>, String) {
    let mut days: Vec<u64> = PRESETS.to_vec();
    if let Some(a) = age
        && !days.contains(&a)
    {
        days.push(a);
        days.sort_unstable();
    }
    let mut out = vec![("never".to_owned(), t("Nooit", "Never").to_owned())];
    out.extend(days.into_iter().map(|d| (d.to_string(), age_label(d))));
    let selected = match (trash_on, age) {
        (true, Some(a)) => a.to_string(),
        (true, None) => "30".to_owned(),
        _ => "never".to_owned(),
    };
    (out, selected)
}

/// What a choice of the Prullenbak row writes (one gsettings.set): Nooit only turns the purge off, a day count turns
/// it on with that age.
pub(crate) fn trash_write(choice: &str) -> Option<Value> {
    if choice == "never" {
        return Some(json!({ "schema": PRIVACY, "values": { TRASH.key: false } }));
    }
    let days: u64 = choice.parse().ok()?;
    Some(json!({ "schema": PRIVACY, "values": { TRASH.key: true, AGE.key: days } }))
}

/// Write keys of the privacy schema; `revert` when it was refused.
fn write(p: &Page, args: Value, revert: impl FnOnce() + 'static) {
    let wait = p.run_with("gsettings.set", args, None, false);
    p.spawn(async move {
        if wait.await.is_none() {
            revert();
        }
    });
}

/// The Opslaginzicht switch (amendment 21): both keys at once, on while either is. Named `st-<page>-storage-sense`.
pub(super) fn master_switch(p: &Page, gs: &Gs) -> ToggleSwitch {
    let on = |gs: &Gs| gs.value(&TRASH) == true || gs.value(&TEMP) == true;
    let sw = ToggleSwitch::new(on(gs));
    sw.switch().set_widget_name(&format!("st-{}-storage-sense", p.id()));
    let writable = gs.writable(&TRASH) && gs.writable(&TEMP);
    if !writable {
        sw.set_sensitive(false);
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (pp, gs2, sw2) = (p.clone(), gs.clone(), sw.clone());
        sw.connect_toggled(move |value| {
            if on(&gs2) == value {
                return;
            }
            let (sw3, gs3) = (sw2.clone(), gs2.clone());
            write(&pp, json!({ "schema": PRIVACY, "values": { TRASH.key: value, TEMP.key: value } }), move || sw3.set_active(on(&gs3)));
        });
    }
    for k in [&TRASH, &TEMP] {
        let (gs2, sw2) = (gs.clone(), sw.clone());
        gs.follow(k, move |_| sw2.set_active(on(&gs2)));
    }
    sw
}

/// The Prullenbak row's drop-down, following both keys.
fn trash_dropdown(p: &Page, gs: &Gs) -> Dropdown {
    let current = |gs: &Gs| trash_options(gs.value(&TRASH) == true, gs.value(&AGE).as_u64());
    let (options, selected) = current(gs);
    let dd = Dropdown::new(options.iter().map(|(v, l)| DropdownOption::new(v, l)).collect(), Some(&selected));
    dd.widget().set_widget_name(&gs::control_name(p.id(), &keys(), &AGE));
    if !(gs.writable(&TRASH) && gs.writable(&AGE)) {
        dd.widget().set_sensitive(false);
        dd.widget().set_tooltip_text(Some(gs::locked_text()));
    }
    let refresh = {
        let (gs2, dd2) = (gs.clone(), dd.clone());
        move || {
            let (options, selected) = current(&gs2);
            dd2.set_options(options.iter().map(|(v, l)| DropdownOption::new(v, l)).collect());
            dd2.set_selected(Some(&selected));
        }
    };
    {
        let (pp, refresh) = (p.clone(), refresh.clone());
        dd.connect_changed(move |choice| {
            if let Some(args) = trash_write(choice) {
                write(&pp, args, refresh.clone());
            }
        });
    }
    for k in [&TRASH, &AGE] {
        let r = refresh.clone();
        gs.follow(k, move |_| r());
    }
    dd
}

/// A button that asks first, then runs `method` and shows `done` (GNOME's Empty Trash… and Delete Temporary Files…).
fn confirmed_button(p: &Page, name: &str, label: &str, question: (&'static str, &'static str, &'static str), method: &'static str, args: Value, done: &'static str) -> gtk::Button {
    let b = c::button(label, c::Variant::Standard);
    b.set_widget_name(name);
    let pp = p.clone();
    b.connect_clicked(move |_| {
        let (p2, args) = (pp.clone(), args.clone());
        c::confirm(&pp.anchor(), question.0, question.1, question.2, true, move |ok| {
            if !ok {
                return;
            }
            let wait = p2.run(method, args);
            let p3 = p2.clone();
            p2.spawn(async move {
                if let Some(v) = wait.await
                    && v["dryRun"] != true
                    && p3.alive()
                {
                    p3.toast(Toast::new(done).severity(Severity::Success).timeout(3000));
                }
            });
        });
    });
    b
}

fn run_now(p: &Page, gs: &Gs) -> gtk::Button {
    let b = c::button(t("Opslaginzicht nu uitvoeren", "Run Storage Sense now"), c::Variant::Standard);
    b.set_widget_name("st-storagesense-run");
    b.set_halign(gtk::Align::Start);
    let any = |gs: &Gs| gs.value(&TRASH) == true || gs.value(&TEMP) == true;
    b.set_sensitive(any(gs));
    for k in [&TRASH, &TEMP] {
        let (gs2, b2) = (gs.clone(), b.clone());
        gs.follow(k, move |_| b2.set_sensitive(any(&gs2)));
    }
    let pp = p.clone();
    b.connect_clicked(move |b| {
        b.set_sensitive(false);
        let wait = pp.run("storage.cleanTemp", json!({ "sense": true }));
        let (p2, b2) = (pp.clone(), b.clone());
        pp.spawn(async move {
            let answer = wait.await;
            if !p2.alive() {
                return;
            }
            b2.set_sensitive(true);
            if let Some(v) = answer
                && v["dryRun"] != true
            {
                let freed = v["freed"].as_f64().unwrap_or(0.0);
                let message = if freed > 0.0 {
                    common::fill(t("{size} vrijgemaakt", "{size} freed up"), &[("size", &kit::format::bytes(freed))])
                } else {
                    t("Er hoefde niets te worden opgeruimd.", "There was nothing to clean up.").to_owned()
                };
                p2.toast(Toast::new(t("Opslaginzicht is uitgevoerd", "Storage Sense has run")).message(&message).severity(Severity::Success).timeout(4000));
            }
        });
    });
    b
}

fn storagesense(p: Page) -> PageFuture {
    Box::pin(async move {
        let gs = Gs::load(&p, &keys()).await;
        if !p.alive() {
            return Content::new();
        }
        if !gs.available(&TRASH) || !gs.available(&TEMP) {
            return vec![common::notice(t("Opslaginzicht is op deze computer niet beschikbaar.", "Storage Sense isn’t available on this computer.")).widget().clone().upcast()];
        }
        let mut out: Content = Vec::new();
        // Opschonen van tijdelijke bestanden
        let temp = SettingsGroup::new("", "");
        if let Some(card) = gs.toggle_card(
            &TEMP,
            "clean",
            t("Tijdelijke bestanden opschonen", "Cleanup of temporary files"),
            t("Zorg dat je pc soepel blijft werken door tijdelijke app-bestanden automatisch op te ruimen", "Keep your PC running smoothly by automatically cleaning up temporary app files"),
            false,
        ) {
            temp.add(card.widget());
        }
        out.push(temp.widget().clone().upcast());
        // Automatisch opschonen van gebruikersinhoud
        let user = SettingsGroup::new(t("Automatisch opschonen van gebruikersinhoud", "Automatic User content cleanup"), "");
        let sense = SettingsCard::new(
            "storage",
            t("Opslaginzicht", "Storage Sense"),
            t("Automatisch ruimte vrijmaken door tijdelijke bestanden en oude items in de Prullenbak te verwijderen", "Automatically free up space by deleting temporary files and old items in the Recycle Bin"),
        );
        sense.set_control(master_switch(&p, &gs).widget());
        user.add(sense.widget());
        out.push(user.widget().clone().upcast());
        // Opschoonschema's configureren
        let plan = SettingsGroup::new(t("Opschoonschema’s configureren", "Configure cleanup schedules"), "");
        if gs.available(&AGE) {
            let row = SettingsCard::new(
                "trash",
                t("Bestanden in mijn Prullenbak verwijderen als ze er langer in staan dan:", "Delete files in my Recycle Bin if they have been there for over:"),
                t("Dezelfde periode geldt voor tijdelijke bestanden.", "The same period applies to temporary files."),
            );
            row.set_control(&common::sized(trash_dropdown(&p, &gs).widget(), 160));
            plan.add(row.widget());
        }
        out.push(plan.widget().clone().upcast());
        out.push(run_now(&p, &gs).upcast());
        // GNOME's own buttons
        let now = SettingsGroup::new(t("Prullenbak en tijdelijke bestanden", "Recycle Bin and temporary files"), "");
        let empty = SettingsCard::new("trash", t("Prullenbak nu leegmaken", "Empty Recycle Bin now"), t("Alle items in de Prullenbak definitief verwijderen", "Permanently delete all items in the Recycle Bin"));
        empty.set_control(&confirmed_button(
            &p,
            "st-storagesense-empty-trash",
            t("Prullenbak leegmaken", "Empty Recycle Bin"),
            (t("Prullenbak leegmaken?", "Empty Recycle Bin?"), t("Alle items in de Prullenbak worden definitief verwijderd. Dit kun je niet ongedaan maken.", "All items in the Recycle Bin will be permanently deleted. This can’t be undone."), t("Leegmaken", "Empty")),
            "files.emptyTrash",
            json!({}),
            t("De Prullenbak is leeggemaakt", "The Recycle Bin has been emptied"),
        ));
        now.add(empty.widget());
        let delete = SettingsCard::new("clean", t("Tijdelijke bestanden nu verwijderen", "Delete temporary files now"), t("Al je tijdelijke bestanden definitief verwijderen", "Permanently delete all your temporary files"));
        delete.set_control(&confirmed_button(
            &p,
            "st-storagesense-delete-temp",
            t("Verwijderen", "Delete"),
            (
                t("Alle tijdelijke bestanden verwijderen?", "Delete all temporary files?"),
                t("Al je tijdelijke bestanden worden definitief verwijderd. Dit kun je niet ongedaan maken.", "All your temporary files will be permanently deleted. This can’t be undone."),
                t("Verwijderen", "Delete"),
            ),
            "storage.cleanTemp",
            json!({}),
            t("De tijdelijke bestanden zijn verwijderd", "The temporary files have been deleted"),
        ));
        now.add(delete.widget());
        out.push(now.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_prullenbak_row_has_windows_presets_and_keeps_another_age() {
        let values = |o: &[(String, String)]| o.iter().map(|(v, _)| v.clone()).collect::<Vec<_>>();
        let (options, selected) = trash_options(true, Some(30));
        assert_eq!(values(&options), ["never", "0", "1", "14", "30", "60"]);
        assert_eq!(selected, "30");
        let (options, selected) = trash_options(true, Some(7));
        assert_eq!(values(&options), ["never", "0", "1", "7", "14", "30", "60"], "a non-preset age is an option of its own");
        assert_eq!(selected, "7");
        assert_eq!(trash_options(false, Some(30)).1, "never", "off is Nooit, whatever the age");
        assert_eq!(trash_options(true, Some(0)).1, "0", "GNOME's 1 uur");
    }

    #[test]
    fn a_choice_writes_the_switch_and_the_age_together() {
        assert_eq!(trash_write("never"), Some(json!({"schema": PRIVACY, "values": {"remove-old-trash-files": false}})));
        assert_eq!(trash_write("14"), Some(json!({"schema": PRIVACY, "values": {"remove-old-trash-files": true, "old-files-age": 14}})));
        assert_eq!(trash_write("x"), None);
        assert_eq!(gs::control_name("storagesense", &keys(), &AGE), "st-storagesense-old-files-age");
    }
}
