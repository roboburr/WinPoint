//! Instellingen › Privacy en beveiliging › Apparaatbeveiliging and Diagnostische gegevens en feedback, as Windows 11
//! lays them out, with GNOME Settings 50's functions (settings parity, batch F, amendment 38):
//!
//! * Apparaatbeveiliging: GNOME's Device Security (privacy.security, fwupd read only): the security level (HSI) with
//!   every check by level, Beveiligd opstarten and Beveiligingsprocessor (Windows' Device security sections), the
//!   security events, Technisch rapport kopiëren; Ubuntu's Security Center (privacy.securityCenter.open); USB4 and
//!   Thunderbolt (GNOME's Thunderbolt panel is Bluetooth en apparaten › USB here).
//! * Diagnostische gegevens en feedback: Ubuntu's Diagnostics: the two system reports (privacy.insights.*, through
//!   ubuntu-insights), Diagnostische gegevens weergeven (the report, cancellable) and Foutrapporten naar Canonical
//!   verzenden (privacy.reports.*, whoopsie-preferences; a write may show the password dialog).
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsExpander, SettingsGroup, ToggleSwitch, Variant};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("security", security), ("diagnostics", diagnostics)];
pub const CSS: &str = "";

// ------------------------------------------------------------------------------------------- pure parts

/// A security check's name in WinPoint's language (cc-firmware-security-utils.c names the same appstream ids);
/// fwupd's own title for one this table does not know.
pub(super) fn attr_title(id: &str, fallback: &str) -> String {
    let known = match id {
        "org.fwupd.hsi.Uefi.SecureBoot" => t("UEFI Beveiligd opstarten", "UEFI Secure Boot"),
        "org.fwupd.hsi.Uefi.Pk" => t("UEFI-platformsleutel", "UEFI platform key"),
        "org.fwupd.hsi.Uefi.BootserviceVars" => t("UEFI-opstartservicevariabelen", "UEFI boot service variables"),
        "org.fwupd.hsi.Uefi.Db" => t("UEFI-database voor Beveiligd opstarten", "UEFI Secure Boot database"),
        "org.fwupd.hsi.Tpm.Version20" => t("TPM 2.0", "TPM 2.0"),
        "org.fwupd.hsi.Tpm.EmptyPcr" => t("TPM-platformconfiguratie", "TPM platform configuration"),
        "org.fwupd.hsi.Tpm.ReconstructionPcr0" => t("TPM-reconstructie", "TPM reconstruction"),
        "org.fwupd.hsi.Iommu" => t("IOMMU-beveiliging", "IOMMU protection"),
        "org.fwupd.hsi.Acpi.Dmar" => t("DMA-beveiliging voor het opstarten", "Pre-boot DMA protection"),
        "org.fwupd.hsi.Kernel.Lockdown" => t("Vergrendeling van de Linux-kernel", "Linux kernel lockdown"),
        "org.fwupd.hsi.Kernel.Tainted" => t("Controle van de Linux-kernel", "Linux kernel verification"),
        "org.fwupd.hsi.Kernel.Swap" => t("Linux-wisselgeheugen", "Linux swap"),
        "org.fwupd.hsi.EncryptedRam" => t("Versleuteld RAM", "Encrypted RAM"),
        "org.fwupd.hsi.SuspendToRam" => t("Slaapstand (in RAM)", "Sleep (suspend to RAM)"),
        "org.fwupd.hsi.SuspendToIdle" => t("Slaapstand (bij inactiviteit)", "Sleep (suspend to idle)"),
        "org.fwupd.hsi.SupportedCpu" => t("Ondersteunde processor", "Supported processor"),
        "org.fwupd.hsi.Fwupd.Updates" => t("Controle van firmware-updates", "Firmware update verification"),
        "org.fwupd.hsi.Fwupd.Attestation" => t("Attestatie van de firmware", "Firmware attestation"),
        "org.fwupd.hsi.Fwupd.Plugins" => t("Invoegtoepassingen van fwupd", "fwupd plug-ins"),
        "org.fwupd.hsi.Spi.Bioswe" => t("Schrijfbeveiliging van de firmware", "Firmware write protection"),
        "org.fwupd.hsi.Spi.Ble" => t("Vergrendeling van de firmware", "Firmware lock"),
        "org.fwupd.hsi.Spi.SmmBwp" => t("BIOS-regio van de firmware", "Firmware BIOS region"),
        "org.fwupd.hsi.Spi.Descriptor" => t("Firmwaredescriptor", "Firmware descriptor"),
        "org.fwupd.hsi.PlatformDebugEnabled" => t("Foutopsporing van het platform", "Platform debugging"),
        "org.fwupd.hsi.PlatformDebugLocked" => t("Vergrendeling van foutopsporing", "Platform debugging lock"),
        "org.fwupd.hsi.PlatformFused" => t("Productiemodus van de processor", "Processor fused for production"),
        _ => "",
    };
    if !known.is_empty() {
        return known.to_owned();
    }
    if !fallback.is_empty() {
        return fallback.to_owned();
    }
    id.rsplit('.').next().unwrap_or(id).to_owned()
}

/// fwupd's FwupdSecurityAttrResult in WinPoint's language (fu_security_attr_result_to_string).
pub(super) fn result_text(result: u64) -> &'static str {
    match result {
        1 => t("Ingeschakeld", "Enabled"),
        2 => t("Uitgeschakeld", "Disabled"),
        3 => t("Geldig", "Valid"),
        4 => t("Ongeldig", "Invalid"),
        5 => t("Vergrendeld", "Locked"),
        6 => t("Ontgrendeld", "Unlocked"),
        7 => t("Versleuteld", "Encrypted"),
        8 => t("Niet versleuteld", "Not encrypted"),
        9 => t("Gewijzigd", "Tainted"),
        10 => t("Niet gewijzigd", "Not tainted"),
        11 => t("Gevonden", "Found"),
        12 => t("Niet gevonden", "Not found"),
        13 => t("Ondersteund", "Supported"),
        14 => t("Niet ondersteund", "Not supported"),
        _ => t("Onbekend", "Unknown"),
    }
}

/// The security level's title and line (cc-firmware-security-panel.c: Checks Failed, Checks Passed, Protected,
/// Checks Unavailable).
pub(super) fn level_text(level: Option<u64>, runtime_issue: bool) -> (String, String) {
    let (title, line) = match level {
        None => (t("Controles niet beschikbaar", "Checks unavailable"), t("De beveiligingscontroles zijn voor dit apparaat niet beschikbaar.", "Device security checks aren’t available for this device.")),
        Some(0) => (t("Controles mislukt", "Checks failed"), t("Dit apparaat voldoet niet aan de basisvereisten voor hardwarebeveiliging.", "This device doesn’t meet the basic requirements for hardware security.")),
        Some(1) => (
            t("Controles geslaagd", "Checks passed"),
            t(
                "Dit apparaat voldoet aan de basisvereisten voor beveiliging. De hardware beschermt tegen sommige soorten beveiligingsrisico’s.",
                "This device meets the basic security requirements. Its hardware protects against some types of security threats.",
            ),
        ),
        Some(_) => (t("Beschermd", "Protected"), t("Dit apparaat is uitgebreid beschermd tegen beveiligingsrisico’s voor hardware.", "This device has extended protection against hardware security threats.")),
    };
    let mut line = line.to_owned();
    if runtime_issue {
        line.push(' ');
        line.push_str(t("Tijdens het gebruik zijn er problemen gevonden.", "Problems were found while the system is running."));
    }
    (title.to_owned(), line)
}

/// The attributes per section, GNOME's order: Beveiligingsniveau 1-4, then the checks while the system runs (level
/// 0); obsoleted ones are left out.
pub(super) fn sections(attrs: &[Value]) -> Vec<(u64, Vec<Value>)> {
    let mut out: Vec<(u64, Vec<Value>)> = Vec::new();
    for level in [1, 2, 3, 4, 0] {
        let items: Vec<Value> = attrs.iter().filter(|a| a["level"].as_u64().unwrap_or(0) == level && a["obsoleted"] != true).cloned().collect();
        if !items.is_empty() {
            out.push((level, items));
        }
    }
    out
}

fn section_name(level: u64) -> String {
    if level == 0 { t("Tijdens het gebruik", "While running").to_owned() } else { common::fill(t("Beveiligingsniveau {n}", "Security level {n}"), &[("n", &level.to_string())]) }
}

fn verdict(a: &Value) -> &'static str {
    if a["success"] == true { t("Geslaagd", "Pass") } else { t("Mislukt", "Fail") }
}

fn event_line(e: &Value) -> String {
    let title = attr_title(common::s(&e["appstreamId"]), common::s(&e["title"]));
    let now = result_text(e["result"].as_u64().unwrap_or(0));
    match e["previous"].as_u64() {
        Some(before) => format!("{title}: {} → {now}", result_text(before)),
        None => format!("{title}: {now}"),
    }
}

/// GNOME's Copy Technical Report, in WinPoint's language.
pub(super) fn technical_report(v: &Value, now_ms: f64) -> String {
    let mut out = Vec::new();
    let title = t("Rapport over apparaatbeveiliging", "Device security report");
    out.push(title.to_owned());
    out.push("=".repeat(title.chars().count()));
    out.push(String::new());
    out.push(t("Rapportgegevens", "Report details").to_owned());
    out.push(format!("  {}: {}", t("Gemaakt op", "Date generated"), kit::format::date(now_ms, DateFormat::DateTime)));
    out.push(format!("  {}: {}", t("fwupd-versie", "fwupd version"), common::s(&v["daemonVersion"])));
    out.push(String::new());
    out.push(t("Systeemgegevens", "System details").to_owned());
    out.push(format!("  {}: {}", t("Fabrikant", "Manufacturer"), common::s(&v["vendor"])));
    out.push(format!("  {}: {}", t("Model", "Model"), common::s(&v["product"])));
    out.push(format!("  {}: {}", t("Beveiligingsniveau", "Security level"), common::s(&v["hsi"])));
    for (level, items) in sections(v["attrs"].as_array().map(Vec::as_slice).unwrap_or(&[])) {
        out.push(String::new());
        out.push(section_name(level));
        for a in items {
            let name = attr_title(common::s(&a["appstreamId"]), common::s(&a["title"]));
            out.push(format!("  {name}: {} ({})", verdict(&a), result_text(a["result"].as_u64().unwrap_or(0))));
        }
    }
    let events = v["events"].as_array().cloned().unwrap_or_default();
    if !events.is_empty() {
        out.push(String::new());
        out.push(t("Beveiligingsgebeurtenissen", "Security events").to_owned());
        for e in &events {
            let when = e["created"].as_f64().map(|s| kit::format::date(s * 1000.0, DateFormat::DateTime)).unwrap_or_default();
            out.push(format!("  {when}  {}", event_line(e)));
        }
    }
    out.push(String::new());
    out.push(t("Meer over de inhoud van dit rapport: https://fwupd.github.io/hsi.html", "For the contents of this report, see https://fwupd.github.io/hsi.html").to_owned());
    out.join("\n") + "\n"
}

// --------------------------------------------------------------------------------------------- Apparaatbeveiliging

fn attr_card(a: &Value) -> SettingsCard {
    let icon = if a["success"] == true { "success" } else { "warning" };
    let name = attr_title(common::s(&a["appstreamId"]), common::s(&a["title"]));
    let card = SettingsCard::new(icon, &name, "");
    card.set_control(&common::value_label(&format!("{} ({})", verdict(a), result_text(a["result"].as_u64().unwrap_or(0)))));
    card
}

fn security(p: Page) -> PageFuture {
    Box::pin(async move {
        let v = p.call("privacy.security", json!({})).await.unwrap_or_else(|_| json!({ "available": false }));
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();
        let available = v["available"] == true;
        let g = SettingsGroup::new(t("Apparaatbeveiliging", "Device security"), t("Beveiliging die in je apparaat is ingebouwd", "Security that is built into your device"));
        if !available {
            g.add(
                SettingsCard::new(
                    "shield",
                    t("Controles niet beschikbaar", "Checks unavailable"),
                    t("De apparaatbeveiliging kan niet worden gecontroleerd: de firmwareservice (fwupd) antwoordt niet.", "Device security can’t be checked: the firmware service (fwupd) doesn’t answer."),
                )
                .widget(),
            );
        } else {
            let (title, line) = level_text(v["level"].as_u64(), v["runtimeIssue"] == true);
            let exp = SettingsExpander::new("shield", &title, &line);
            exp.header_button().set_widget_name("security-level");
            if let Some(level) = v["level"].as_u64() {
                exp.header().set_control(&common::value_label(&common::fill(t("Niveau {n}", "Level {n}"), &[("n", &level.to_string())])));
            }
            for (level, items) in sections(v["attrs"].as_array().map(Vec::as_slice).unwrap_or(&[])) {
                exp.add(&common::section_title(&section_name(level)));
                for a in &items {
                    exp.add(attr_card(a).widget());
                }
            }
            let copy = c::button(t("Kopiëren", "Copy"), Variant::Standard);
            copy.set_widget_name("security-copy-report");
            {
                let (v, pp) = (v.clone(), p.clone());
                copy.connect_clicked(move |_| {
                    if common::copy_text(&technical_report(&v, kit::format::now_ms())) {
                        common::toast_success(&pp.anchor(), t("Het technische rapport is gekopieerd", "The technical report was copied"), 2500);
                    }
                });
            }
            let report = SettingsCard::new("copy", t("Technisch rapport kopiëren", "Copy technical report"), t("Alle controles en gebeurtenissen als tekst, voor een supportvraag", "Every check and event as text, for a support request"));
            report.set_control(&copy);
            exp.add(report.widget());
            g.add(exp.widget());
            let attrs = v["attrs"].as_array().cloned().unwrap_or_default();
            // Windows' Device security sections: Beveiligd opstarten and Beveiligingsprocessor
            if let Some(state) = v["secureBoot"].as_str() {
                let line = match state {
                    "on" => t(
                        "Beveiligd opstarten is ingeschakeld. Het voorkomt dat schadelijke software wordt geladen wanneer het apparaat opstart.",
                        "Secure boot is on. It prevents malicious software from loading when your device starts up.",
                    ),
                    "off" => t(
                        "Beveiligd opstarten is uitgeschakeld. Je kunt het inschakelen in de firmware-instellingen van het apparaat.",
                        "Secure boot is off. You can turn it on in the device’s firmware settings.",
                    ),
                    _ => t("Er is een probleem gevonden met Beveiligd opstarten.", "A problem was found with secure boot."),
                };
                let card = SettingsCard::new("lock", t("Beveiligd opstarten", "Secure boot"), line);
                card.widget().set_widget_name("security-secure-boot");
                g.add(card.widget());
            }
            if let Some(tpm) = attrs.iter().find(|a| a["appstreamId"] == "org.fwupd.hsi.Tpm.Version20") {
                let line = if tpm["success"] == true {
                    t("Je beveiligingsprocessor (TPM 2.0) is aanwezig en biedt extra versleuteling voor je apparaat.", "Your security processor (TPM 2.0) is present and provides additional encryption for your device.")
                } else {
                    t("Er is geen werkende beveiligingsprocessor (TPM 2.0) gevonden.", "No working security processor (TPM 2.0) was found.")
                };
                g.add(SettingsCard::new("cpu", t("Beveiligingsprocessor", "Security processor"), line).widget());
            }
            let events = v["events"].as_array().cloned().unwrap_or_default();
            if !events.is_empty() {
                let ge = SettingsGroup::new(t("Beveiligingsgebeurtenissen", "Security events"), "");
                for e in &events {
                    let when = e["created"].as_f64().map(|s| kit::format::date(s * 1000.0, DateFormat::DateTime)).unwrap_or_default();
                    ge.add(SettingsCard::new("history", &event_line(e), &when).widget());
                }
                out.push(g.widget().clone().upcast());
                out.push(ge.widget().clone().upcast());
            } else {
                out.push(g.widget().clone().upcast());
            }
        }
        if !available {
            out.push(g.widget().clone().upcast());
        }

        // ---- Ubuntu's Security Center and GNOME's Thunderbolt page
        let more = SettingsGroup::new(t("Meer beveiliging", "More security"), "");
        if v["securityCenter"] == true || common::caps()["securityCenter"] == true {
            // the app's own names (the snap's desktop file: Name[nl], Name[en_GB])
            let open = c::button(t("Beveiligingscentrum openen", "Open Security Centre"), Variant::Standard);
            open.set_widget_name("security-center");
            let pp = p.clone();
            open.connect_clicked(move |_| {
                let fut = pp.run("privacy.securityCenter.open", json!({}));
                pp.spawn(async move {
                    let _ = fut.await;
                });
            });
            let card = SettingsCard::new(
                "shield",
                t("Beveiligingscentrum", "Security Centre"),
                t("Meer beveiligingsinstellingen van Ubuntu, zoals de machtigingen van apps en de firewall", "More of Ubuntu’s security settings, such as app permissions and the firewall"),
            );
            card.set_control(&open);
            more.add(card.widget());
        }
        if crate::registry::page("usb").is_some_and(common::page_visible) {
            more.add(
                common::link_card_with(&p, "usb", Some(t("USB4- en Thunderbolt-apparaten", "USB4 and Thunderbolt devices")), Some(t("Directe toegang van docks en externe apparaten", "Direct access for docks and external devices")), None)
                    .widget(),
            );
        }
        if more.items().first_child().is_some() {
            out.push(more.widget().clone().upcast());
        }
        out
    })
}

// --------------------------------------------------------------------------- Diagnostische gegevens en feedback

/// A switch whose write goes to privacy.insights.set: optimistic, reverted with the error toast.
fn consent_switch(p: &Page, source: &'static str, on: bool) -> ToggleSwitch {
    let sw = ToggleSwitch::new(on);
    sw.switch().set_widget_name(&format!("st-diagnostics-insights-{}", source.replace('_', "-")));
    let (pp, sw2) = (p.clone(), sw.clone());
    sw.connect_toggled(move |on| {
        let fut = pp.run("privacy.insights.set", json!({ "source": source, "consent": on }));
        let sw3 = sw2.clone();
        pp.spawn(async move {
            if fut.await.is_none() {
                sw3.set_active(!on);
            }
        });
    });
    sw
}

/// "Diagnostische gegevens weergeven": the report ubuntu-insights would send, made now; closing stops it (GNOME's
/// Cancel Report Generation).
fn show_report(p: &Page) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let status = c::label(t("Het rapport wordt gemaakt…", "Creating the report…"), "w11-text-2", true);
    status.set_widget_name("diagnostics-report-status");
    body.append(&status);
    let done = Rc::new(std::cell::Cell::new(false));
    let dialog = Dialog::new(t("Diagnostische gegevens", "Diagnostic data")).body(&body).close_label(t("Sluiten", "Close")).width(680);
    {
        let (pp, done) = (p.clone(), done.clone());
        dialog.show(&p.anchor(), move |_| {
            if !done.get() {
                let fut = pp.call("privacy.insights.report", json!({ "cancel": true }));
                pp.spawn(async move {
                    let _ = fut.await;
                });
            }
        });
    }
    let fut = p.call("privacy.insights.report", json!({}));
    p.spawn(async move {
        let r = fut.await;
        done.set(true);
        match r {
            Ok(v) => {
                let view = gtk::TextView::new();
                view.set_editable(false);
                view.set_monospace(true);
                view.set_wrap_mode(gtk::WrapMode::WordChar);
                view.buffer().set_text(common::s(&v["report"]));
                view.set_widget_name("diagnostics-report");
                let scroll = c::scrolled(&view);
                scroll.set_min_content_height(360);
                status.set_text(t(
                    "Dit is het systeemrapport dat Ubuntu zou verzenden. Het bevat geen persoonlijke gegevens en wordt alleen verzonden als je dat hebt ingeschakeld.",
                    "This is the system report Ubuntu would send. It contains no personal data and is only sent if you turned that on.",
                ));
                body.append(&scroll);
            }
            Err(e) if e.code == "cancelled" => {}
            Err(e) => status.set_text(&e.message),
        }
    });
}

fn diagnostics(p: Page) -> PageFuture {
    Box::pin(async move {
        let (insights, reports) = (p.call("privacy.insights.get", json!({})), p.call("privacy.reports.get", json!({})));
        let (insights, reports) = (insights.await, reports.await);
        if !p.alive() {
            return Content::new();
        }
        let insights = insights.unwrap_or_else(|_| json!({ "available": false }));
        let reports = reports.unwrap_or_else(|_| json!({ "available": false }));
        let g = SettingsGroup::new("", "");

        // ---- Diagnostische gegevens: Ubuntu's System Reporting
        let exp = SettingsExpander::new(
            "chart",
            t("Diagnostische gegevens", "Diagnostic data"),
            t("Help Canonical Ubuntu te verbeteren met anonieme systeemrapporten", "Help Canonical improve Ubuntu with anonymous system reports"),
        );
        exp.header_button().set_widget_name("diagnostics-data");
        if insights["available"] == true {
            for (source, title, desc) in [
                (
                    "linux",
                    t("Elke maand een systeemrapport verzenden", "Send a system report every month"),
                    t("Anonieme gegevens over de hardware en software van dit apparaat", "Anonymous details about this device’s hardware and software"),
                ),
                (
                    "ubuntu_release_upgrader",
                    t("Een systeemrapport verzenden bij een upgrade naar een nieuwe Ubuntu-versie", "Send a system report when upgrading to a new Ubuntu version"),
                    t("Eenmalig, wanneer je Ubuntu naar een nieuwe versie bijwerkt", "Once, when you upgrade Ubuntu to a new version"),
                ),
            ] {
                let card = SettingsCard::new("", title, desc);
                card.set_control(consent_switch(&p, source, insights["sources"][source] == true).widget());
                exp.add(card.widget());
            }
        } else {
            exp.add(&SettingsCard::new("info", t("Systeemrapporten zijn op dit apparaat niet beschikbaar.", "System reports aren’t available on this device."), "").widget().clone());
        }
        exp.set_expanded(true);
        g.add(exp.widget());
        if insights["available"] == true {
            let view = c::button(t("Rapport weergeven", "View report"), Variant::Standard);
            view.set_widget_name("diagnostics-view");
            let pp = p.clone();
            view.connect_clicked(move |_| show_report(&pp));
            let card = SettingsCard::new("eye", t("Diagnostische gegevens weergeven", "View diagnostic data"), t("Het systeemrapport bekijken dat dit apparaat zou verzenden", "See the system report this device would send"));
            card.set_control(&view);
            g.add(card.widget());
        }

        // ---- Foutrapporten naar Canonical verzenden (WinPoint addition: GNOME's Ubuntu Error Reports)
        if reports["available"] == true {
            let options = vec![
                DropdownOption::new("never", t("Nooit", "Never")),
                DropdownOption::new("auto", t("Automatisch", "Automatically")),
                DropdownOption::new("manual", t("Handmatig", "Manually")),
            ];
            let dd = Dropdown::new(options, reports["mode"].as_str());
            dd.widget().set_widget_name("st-diagnostics-reports");
            let current = Rc::new(std::cell::RefCell::new(common::s(&reports["mode"]).to_owned()));
            {
                let (pp, dd2) = (p.clone(), dd.clone());
                dd.connect_changed(move |mode| {
                    let mode = mode.to_owned();
                    let fut = pp.run("privacy.reports.set", json!({ "mode": mode }));
                    let (dd3, current) = (dd2.clone(), current.clone());
                    pp.spawn(async move {
                        match fut.await {
                            Some(_) => *current.borrow_mut() = mode,
                            None => dd3.set_selected(Some(current.borrow().as_str())),
                        }
                    });
                });
            }
            let card = SettingsCard::new(
                "warning",
                t("Foutrapporten naar Canonical verzenden", "Send error reports to Canonical"),
                t(
                    "Foutrapporten bevatten technische details over problemen, zonder je persoonlijke gegevens. Wijzigen vraagt mogelijk om je wachtwoord.",
                    "Error reports contain technical details about problems, without your personal data. Changing this may ask for your password.",
                ),
            );
            card.set_control(&common::sized(dd.widget(), 180));
            g.add(card.widget());
        }
        vec![g.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_are_named_and_grouped_as_gnome_shows_them() {
        kit::i18n::set_forced_lang(Some("nl"));
        assert_eq!(attr_title("org.fwupd.hsi.Uefi.SecureBoot", "x"), "UEFI Beveiligd opstarten");
        assert_eq!(attr_title("org.fwupd.hsi.New.Thing", "New thing"), "New thing", "fwupd's own title for an unknown check");
        assert_eq!(attr_title("org.fwupd.hsi.New.Thing", ""), "Thing");
        assert_eq!(result_text(2), "Uitgeschakeld");
        assert_eq!(level_text(Some(1), false).0, "Controles geslaagd");
        assert!(level_text(Some(1), true).1.ends_with("Tijdens het gebruik zijn er problemen gevonden."));
        assert_eq!(level_text(None, false).0, "Controles niet beschikbaar");
        assert_eq!(level_text(Some(3), false).0, "Beschermd");
        kit::i18n::set_forced_lang(Some("en"));
        assert_eq!(level_text(Some(0), false).0, "Checks failed");
        assert_eq!(result_text(12), "Not found");
        let attrs = vec![
            json!({"appstreamId": "a", "level": 2, "success": true}),
            json!({"appstreamId": "b", "level": 0, "success": false}),
            json!({"appstreamId": "c", "level": 1, "success": true}),
            json!({"appstreamId": "d", "level": 1, "obsoleted": true}),
        ];
        let grouped = sections(&attrs);
        let order: Vec<(u64, Vec<&str>)> = grouped.iter().map(|(l, items)| (*l, items.iter().map(|a| common::s(&a["appstreamId"])).collect())).collect();
        assert_eq!(order, vec![(1, vec!["c"]), (2, vec!["a"]), (0, vec!["b"])], "levels first, the running checks last, obsoleted left out");
        let v = json!({
            "hsi": "HSI:1! (v2.0.13)", "daemonVersion": "2.0.13", "vendor": "Vendor", "product": "Model X",
            "attrs": [{"appstreamId": "org.fwupd.hsi.Uefi.SecureBoot", "level": 1, "result": 1, "success": true}],
            "events": [{"appstreamId": "org.fwupd.hsi.Uefi.SecureBoot", "result": 2, "previous": 1, "created": 1_700_000_000}],
        });
        let report = technical_report(&v, 1_700_000_000_000.0);
        assert!(report.starts_with("Device security report\n======================\n"));
        assert!(report.contains("  Security level: HSI:1! (v2.0.13)\n"));
        assert!(report.contains("Security level 1\n  UEFI Secure Boot: Pass (Enabled)\n"));
        assert!(report.contains("UEFI Secure Boot: Enabled → Disabled"));
        kit::i18n::set_forced_lang(None);
    }
}
