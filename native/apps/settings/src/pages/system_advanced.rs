//! Instellingen › Systeem › Geavanceerd (batch C, notes/claude/settings-parity.md §2): Windows 11 25H2's System ›
//! Advanced (the former "Voor ontwikkelaars") with GNOME Settings' System › Secure Shell, and Windows' Optional
//! features for the OpenSSH server: the "OpenSSH-server" switch (ssh.socket, else ssh.service, through systemd with
//! the administrator's password, `sharing.ssh.set`), GNOME's "SSH-aanmeldingsopdracht" with Kopiëren, and, without
//! openssh-server, "Een optionele functie toevoegen" (PackageKit, `sharing.ssh.install`, with its progress; the server
//! arrives off, as Windows' optional feature, and is turned on with the switch).
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::pickers;
use kit::controls::{self as c, ProgressRing, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;

pub const PAGES: &[(&str, Builder)] = &[("system-advanced", system_advanced)];
pub const CSS: &str = "";

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// The line under the switch: what runs (socket activation, as Ubuntu's openssh-server ships it).
pub fn ssh_status(ssh: &Value) -> &'static str {
    let socket = ssh["socket"]["running"] == true;
    let service = ssh["service"]["running"] == true;
    match (socket, service) {
        (true, true) => t("Aan: er is nu een verbinding", "On: there’s a connection now"),
        (true, false) => t("Aan: start wanneer er een verbinding binnenkomt", "On: starts when a connection comes in"),
        (false, true) => t("Aan", "On"),
        _ if ssh["service"]["active"] == "failed" || ssh["socket"]["active"] == "failed" => t("Kan niet worden gestart", "Couldn’t start"),
        _ => t("Uit", "Off"),
    }
}

/// The install progress text ("Wordt geïnstalleerd… 40%").
pub fn progress_text(percentage: Option<u64>) -> String {
    match percentage.filter(|p| *p <= 100) {
        Some(p) => common::fill(t("Wordt geïnstalleerd… {n}%", "Installing… {n}%"), &[("n", &p.to_string())]),
        None => t("Wordt geïnstalleerd…", "Installing…").to_owned(),
    }
}

/// The toast after the install: as Windows' optional feature the server arrives off (the host turned off what the
/// package started); when that was refused (the password dialog cancelled) the toast says it is on.
pub fn installed_text(answer: &Value) -> &'static str {
    if answer["on"] == true {
        t("OpenSSH-server is geïnstalleerd en ingeschakeld", "OpenSSH Server is installed and turned on")
    } else {
        t("OpenSSH-server is geïnstalleerd", "OpenSSH Server is installed")
    }
}

/// A command with its Kopiëren button.
fn copy_value(p: &Page, value: &str, name: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let l = common::value_label(value);
    l.set_selectable(true);
    row.append(&l);
    let b = c::button(t("Kopiëren", "Copy"), Variant::Standard);
    b.set_widget_name(name);
    let (pp, text) = (p.clone(), value.to_owned());
    b.connect_clicked(move |_| {
        let ok = common::copy_text(&text);
        pp.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).severity(if ok { Severity::Success } else { Severity::Error }).timeout(2000));
    });
    row.append(&b);
    row
}

/// The installed server: the switch and the login command.
fn ssh_card(p: &Page, ssh: &Value, group: &SettingsGroup) {
    let exp = SettingsExpander::new("terminal", t("OpenSSH-server", "OpenSSH Server"), t("Toegang tot deze pc via Secure Shell (SSH)", "Access this PC using Secure Shell (SSH)"));
    exp.widget().set_widget_name("system-advanced-ssh");
    let sw = ToggleSwitch::new(ssh["on"] == true);
    sw.switch().set_widget_name("system-advanced-ssh-switch");
    sw.switch().set_tooltip_text(Some(t("Hiervoor is een beheerderswachtwoord nodig.", "This needs an administrator password.")));
    {
        let (pp, sw2) = (p.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let wait = pp.run("sharing.ssh.set", json!({ "on": on }));
            let (pp2, sw) = (pp.clone(), sw2.clone());
            pp.spawn(async move {
                match wait.await {
                    Some(v) => {
                        common::note_dry_run(&pp2.anchor(), &v);
                        pp2.refresh();
                    }
                    None => sw.set_active(!on),
                }
            });
        });
    }
    exp.header().set_control(sw.widget());
    let state = SettingsCard::new("", t("Status", "Status"), "");
    state.set_control(&common::value_label(ssh_status(ssh)));
    state.widget().set_widget_name("system-advanced-ssh-status");
    exp.add(state.widget());
    let cmd = SettingsCard::new(
        "",
        t("SSH-aanmeldingsopdracht", "SSH login command"),
        t("Voer deze opdracht uit op het andere apparaat om je aan te melden bij deze pc", "Run this command on the other device to sign in to this PC"),
    );
    cmd.set_control(&copy_value(p, s(&ssh["command"]), "system-advanced-ssh-copy"));
    exp.add(cmd.widget());
    exp.set_expanded(true);
    group.add(exp.widget());
}

/// Windows' "Een optionele functie toevoegen" for openssh-server.
fn install_card(p: &Page, ssh: &Value, group: &SettingsGroup) {
    let card = SettingsCard::new(
        "terminal",
        t("OpenSSH-server", "OpenSSH Server"),
        t("Niet geïnstalleerd. Hiermee krijg je toegang tot deze pc via Secure Shell (SSH).", "Not installed. This gives access to this PC using Secure Shell (SSH)."),
    );
    card.widget().set_widget_name("system-advanced-ssh-install-card");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let ring = ProgressRing::new(16, None);
    ring.widget().set_valign(gtk::Align::Center);
    let progress = common::value_label("");
    progress.set_widget_name("system-advanced-ssh-progress");
    let busy = ssh["installing"] == true;
    ring.widget().set_visible(busy);
    progress.set_visible(busy);
    if busy {
        progress.set_label(&progress_text(None));
    }
    row.append(ring.widget());
    row.append(&progress);
    let install = pickers::admin_button(t("Installeren", "Install"), Variant::Accent);
    install.set_widget_name("system-advanced-ssh-install");
    install.set_sensitive(!busy);
    {
        let (pp, ring, progress) = (p.clone(), ring.clone(), progress.clone());
        install.connect_clicked(move |b| {
            b.set_sensitive(false);
            ring.widget().set_visible(true);
            progress.set_visible(true);
            progress.set_label(&progress_text(None));
            let wait = pp.run("sharing.ssh.install", json!({}));
            let (pp2, b, ring, progress) = (pp.clone(), b.clone(), ring.clone(), progress.clone());
            pp.spawn(async move {
                match wait.await {
                    Some(v) => {
                        common::note_dry_run(&pp2.anchor(), &v);
                        if v["dryRun"] != true {
                            common::toast_success(&pp2.anchor(), installed_text(&v), 3000);
                        }
                        pp2.refresh();
                    }
                    None => {
                        ring.widget().set_visible(false);
                        progress.set_visible(false);
                        b.set_sensitive(true);
                    }
                }
            });
        });
    }
    {
        let progress = progress.clone();
        p.on("sharing.changed", move |d| {
            if d["what"] == "sshInstall" {
                progress.set_label(&progress_text(d["percentage"].as_u64()));
            }
        });
    }
    row.append(&install);
    card.set_control(&row);
    group.add(card.widget());
}

fn system_advanced(p: Page) -> PageFuture {
    Box::pin(async move {
        let r = p.call("sharing.ssh.get", json!({})).await;
        if !p.alive() {
            return Content::new();
        }
        let ssh = match r {
            Ok(v) if v.is_object() => v,
            Err(e) if e.code == "unknown_method" => return vec![common::notice(ct::unavailable_here()).widget().clone().upcast()],
            Err(e) => return vec![common::notice(&e.message).widget().clone().upcast()],
            _ => return vec![common::notice(ct::unavailable()).widget().clone().upcast()],
        };
        let mut out: Content = Vec::new();
        if ssh["installed"] == true {
            let group = SettingsGroup::new(t("Secure Shell", "Secure Shell"), "");
            ssh_card(&p, &ssh, &group);
            out.push(group.widget().clone().upcast());
        } else {
            let group = SettingsGroup::new(
                t("Een optionele functie toevoegen", "Add an optional feature"),
                t("Optionele functies worden met PackageKit uit de softwarebronnen van je distributie geïnstalleerd.", "Optional features are installed from your distribution’s software sources with PackageKit."),
            );
            install_card(&p, &ssh, &group);
            out.push(group.widget().clone().upcast());
        }
        {
            let pp = p.clone();
            p.on("sharing.changed", move |d| {
                if d["what"] == "ssh" {
                    pp.refresh();
                }
            });
        }
        let related = SettingsGroup::new(ct::related(), "");
        for id in ["remotedesktop", "sharing-advanced"] {
            if crate::registry::page(id).is_some_and(common::page_visible) {
                related.add(common::link_card(&p, id).widget());
            }
        }
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ssh_status_follows_the_units() {
        let on = |socket: bool, service: bool| json!({ "socket": { "running": socket }, "service": { "running": service } });
        assert_eq!(ssh_status(&on(true, false)), t("Aan: start wanneer er een verbinding binnenkomt", "On: starts when a connection comes in"));
        assert_eq!(ssh_status(&on(false, true)), t("Aan", "On"));
        assert_eq!(ssh_status(&on(false, false)), t("Uit", "Off"));
        assert_eq!(ssh_status(&json!({ "service": { "active": "failed" } })), t("Kan niet worden gestart", "Couldn’t start"));
        assert!(progress_text(Some(40)).contains("40%"));
        assert!(!progress_text(Some(140)).contains('%'));
        assert_eq!(installed_text(&json!({ "installed": true, "on": false })), t("OpenSSH-server is geïnstalleerd", "OpenSSH Server is installed"));
        assert_eq!(
            installed_text(&json!({ "installed": true, "on": true })),
            t("OpenSSH-server is geïnstalleerd en ingeschakeld", "OpenSSH Server is installed and turned on")
        );
    }
}
