//! Instellingen › Privacy en beveiliging (6ebf169f:vendor/ui/js/apps/settings/pages/privacy.js): recent files (and
//! clearing them), and how Win11 Shell keeps you safe.
use super::{Builder, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, SettingsCard, SettingsGroup, Severity, Toast, Variant};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("privacy", page)];
pub const CSS: &str = "";

/// "Wissen": ask, then apps.clearRecent (privacy.js clearBtn).
fn clear_recent(p: &Page) {
    let pp = p.clone();
    c::confirm(
        &p.anchor(),
        t("Recente bestanden wissen?", "Clear recent files?"),
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
                    Ok(_) => common::toast_success(&p2.anchor(), t("De lijst met recente bestanden is gewist", "The recent files list was cleared"), 2500),
                    Err(e) if e.code == "unknown_method" => p2.toast(
                        Toast::new(t("Wissen is in deze versie nog niet beschikbaar", "Clearing isn’t available in this version yet"))
                            .message(t("Zet “Recente bestanden bijhouden” uit om de lijst te verbergen.", "Turn off “Keep track of recent files” to hide the list."))
                            .severity(Severity::Info)
                            .timeout(5000),
                    ),
                    Err(e) => p2.error(&e, None),
                }
            });
        },
    );
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let activity = SettingsGroup::new(t("Recente activiteit", "Recent activity"), "");
        let (recent, sw) = common::toggle_card(
            &p,
            "history",
            t("Recente bestanden bijhouden", "Keep track of recent files"),
            t(
                "Start en Verkenner laten de bestanden zien die je onlangs hebt geopend. Alleen jij ziet deze lijst; hij blijft op deze computer.",
                "Start and File Explorer show the files you opened recently. Only you see this list; it stays on this computer.",
            ),
            "privacy.recentFiles",
            false,
        );
        sw.switch().set_widget_name("privacy-recent");
        activity.add(recent.widget());
        let clear_btn = c::button(t("Wissen", "Clear"), Variant::Standard);
        clear_btn.set_widget_name("privacy-clear");
        {
            let pp = p.clone();
            clear_btn.connect_clicked(move |_| clear_recent(&pp));
        }
        let clear = SettingsCard::new(
            "delete",
            t("Geschiedenis van recente bestanden wissen", "Clear recent files history"),
            t(
                "De lijst met onlangs geopende bestanden leegmaken. De bestanden zelf blijven bestaan.",
                "Empty the list of recently opened files. The files themselves stay.",
            ),
        );
        clear.set_control(&clear_btn);
        activity.add(clear.widget());

        let security = SettingsGroup::new(t("Zo houdt WinPoint je veilig", "How WinPoint keeps you safe"), "");
        for (icon, title, desc) in [
            (
                "trash",
                t("Verwijderen gaat naar de Prullenbak", "Deleted files go to the Recycle Bin"),
                t(
                    "Bestanden worden nooit meteen gewist. Je kunt ze terugzetten tot je de Prullenbak leegmaakt.",
                    "Files are never erased straight away. You can restore them until you empty the Recycle Bin.",
                ),
            ),
            (
                "shield",
                t("Eerst vragen", "Asking first"),
                t(
                    "Voordat er iets belangrijks gebeurt, zoals afsluiten of opruimen, vraagt WinPoint of je het zeker weet.",
                    "Before something important happens, like shutting down or cleaning up, WinPoint asks if you’re sure.",
                ),
            ),
            (
                "key",
                t("Beheerdersrechten", "Administrator rights"),
                t(
                    "Sommige taken hebben je wachtwoord nodig. WinPoint legt altijd eerst uit waarom, daarna vraagt Ubuntu om je wachtwoord.",
                    "Some tasks need your password. WinPoint always explains why first, then Ubuntu asks for your password.",
                ),
            ),
            (
                "lock",
                t("Je gegevens blijven op deze computer", "Your data stays on this computer"),
                t(
                    "WinPoint stuurt geen gegevens over jou of je bestanden naar internet. Alleen de updatecontrole vraagt GitHub één keer per dag naar de nieuwste versie van WinPoint (GitHub ziet dan je IP-adres en de versie van WinPoint). Je zet die uit in Update en herstel › Geavanceerde opties.",
                    "WinPoint doesn’t send any data about you or your files to the internet. Only the update check asks GitHub once a day for WinPoint’s latest version (GitHub then sees your IP address and WinPoint’s version). You can turn it off in Update & recovery › Advanced options.",
                ),
            ),
        ] {
            security.add(SettingsCard::new(icon, title, desc).widget());
        }
        vec![activity.widget().clone().upcast(), security.widget().clone().upcast()]
    })
}
