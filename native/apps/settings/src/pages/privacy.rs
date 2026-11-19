//! Instellingen › Privacy en beveiliging as Windows 11 lays it out (settings parity, batch F): Beveiliging
//! (Apparaatbeveiliging), Machtigingen (Diagnostische gegevens en feedback, Activiteitengeschiedenis,
//! Zoekmachtigingen, Deze pc doorzoeken), App-machtigingen (Locatie, Camera, Microfoon, Meldingen), then how WinPoint
//! keeps you safe (6ebf169f:vendor/ui/js/apps/settings/pages/privacy.js). The recent-files rows moved to
//! Activiteitengeschiedenis (privacy_history.rs), as plan.json says.
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{SettingsCard, SettingsGroup};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("privacy", page)];
pub const CSS: &str = "";

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
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
        // Windows' groups of Privacy & security from the registry; App-machtigingen ends with Meldingen (Windows'
        // Notifications row, which opens Systeem › Meldingen: the per-app switches are there)
        let apps = common::links_group(&p, t("App-machtigingen", "App permissions"), &["location", "camera", "microphone"])
            .unwrap_or_else(|| SettingsGroup::new(t("App-machtigingen", "App permissions"), ""));
        apps.add(common::link_card_with(&p, "notifications", None, Some(t("Welke apps meldingen mogen weergeven", "Which apps may show notifications")), None).widget());
        let groups = [
            common::links_group(&p, t("Beveiliging", "Security"), &["security"]),
            common::links_group(
                &p,
                t("Machtigingen", "Permissions"),
                &["diagnostics", "activity", "searchpermissions", "searching"],
            ),
            Some(apps),
        ];
        let mut out: Content = groups.into_iter().flatten().map(|g| g.widget().clone().upcast()).collect();
        out.push(security.widget().clone().upcast());
        out
    })
}
