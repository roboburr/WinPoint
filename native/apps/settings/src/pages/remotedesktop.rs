//! Instellingen › Systeem › Extern bureaublad (batch C, notes/claude/settings-parity.md §2): Windows 11's Remote
//! Desktop page with GNOME Settings' Remote Desktop behind it (gnome-remote-desktop, `sharing.rdp.*`).
//!
//! * "Extern bureaublad" is GNOME's Desktop Sharing switch, with Windows' question "Extern bureaublad inschakelen?":
//!   `sharing.rdp.set` writes `enable`, makes GNOME's TLS certificate when there is none, stores GNOME's default sign-in
//!   details when the keyring has none, and starts the user unit. Its expander holds Windows' Network Level
//!   Authentication box (always on: gnome-remote-desktop always asks for credentials), the port, and GNOME's
//!   "Externe besturing" (`view-only`, inverted, bound through `crate::gs`).
//! * "Aanmeldingsgegevens" (Windows' "Gebruikers van Extern bureaublad" as GNOME's Login Details): user name and
//!   password with Kopiëren and Weergeven, Bewerken, GNOME's "Nieuw wachtwoord genereren" and "Versleuteling
//!   controleren" (the certificate's SHA-256 fingerprint).
//! * "Pc-naam" and "IP-adres" with Kopiëren (GNOME's How to Connect).
//! * "Extern aanmelden" (GNOME's Remote Login: the system daemon, every call with the administrator's password):
//!   its switch asks for the sign-in details first, its port, sign-in details and fingerprint.
use super::net_editor as ed;
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use crate::pickers;
use kit::controls::{self as c, Dialog, DialogResult, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("remotedesktop", remotedesktop)];
pub const CSS: &str = r#"
.st-rd-fingerprint { font-family: "Cascadia Mono", "Ubuntu Mono", monospace; font-size: 15px; }
"#;

const RDP: &str = "org.gnome.desktop.remote-desktop.rdp";
const ENABLE: GKey = gs::key(RDP, "enable");
const VIEW_ONLY: GKey = gs::key(RDP, "view-only");

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// A fingerprint (`AB:CD:…`, 32 pairs) in four lines of eight pairs, as GNOME's dialog splits it.
pub fn fingerprint_lines(fp: &str) -> String {
    let pairs: Vec<&str> = fp.split(':').filter(|p| !p.is_empty()).collect();
    pairs.chunks(8).map(|c| c.join(":")).collect::<Vec<_>>().join("\n")
}

/// The addresses another device can connect to: IPv4 first, then global IPv6 (no loopback, no link-local).
pub fn addresses(devs: &Value) -> Vec<String> {
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for d in devs["devices"].as_array().into_iter().flatten() {
        for a in d["ip4"]["addresses"].as_array().into_iter().flatten() {
            let ip = s(&a["address"]);
            if !ip.is_empty() && !ip.starts_with("127.") && !ip.starts_with("169.254.") && !v4.iter().any(|x| x == ip) {
                v4.push(ip.to_owned());
            }
        }
        for a in d["ip6"]["addresses"].as_array().into_iter().flatten() {
            let ip = s(&a["address"]).to_lowercase();
            if !ip.is_empty() && ip != "::1" && !ip.starts_with("fe80:") && !v6.contains(&ip) {
                v6.push(ip);
            }
        }
    }
    v4.extend(v6);
    v4
}

/// GNOME's "Generate New Password": 16 letters and digits.
pub fn new_password() -> String {
    const SET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    (0..16).map(|_| SET[glib::random_int_range(0, SET.len() as i32) as usize] as char).collect()
}

/// A user name and password the host takes: both filled in, no line breaks, the name not starting with "-".
pub fn credentials_ok(user: &str, password: &str) -> bool {
    !user.is_empty() && !user.starts_with('-') && !password.is_empty() && !user.chars().chain(password.chars()).any(char::is_control)
}

/// A value with its Kopiëren button (a single value, as Windows' Pc-naam).
fn copy_value(p: &Page, value: &str, name: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&common::value_label(value));
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

/// GNOME's "Encryption Fingerprint" dialog.
fn fingerprint_dialog(p: &Page, fp: Option<&str>) {
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&c::label(
        t(
            "De versleutelingsvingerafdruk is te zien in apps die verbinding maken met deze pc en moet daar hetzelfde zijn.",
            "The encryption fingerprint can be seen in apps that connect to this PC and should be identical there.",
        ),
        "",
        true,
    ));
    match fp {
        Some(fp) => {
            let l = c::label(&fingerprint_lines(fp), "st-rd-fingerprint", false);
            l.set_selectable(true);
            l.set_widget_name("remotedesktop-fingerprint");
            body.append(&l);
        }
        None => body.append(&c::label(t("Er is nog geen certificaat. Het wordt gemaakt wanneer je Extern bureaublad inschakelt.", "There’s no certificate yet. It’s made when you turn on Remote Desktop."), "", true)),
    }
    Dialog::new(t("Versleutelingsvingerafdruk", "Encryption fingerprint")).body(&body).close_label(t("Sluiten", "Close")).width(460).show(&p.anchor(), |_| {});
}

/// What a sign-in details dialog does with what was typed: (dialog, its error line, user name, password).
type Save = Rc<dyn Fn(&Dialog, &gtk::Label, String, String)>;

/// The sign-in details dialog: user name, password (shown or not, GNOME's Generate New Password); `save` gets them,
/// `done` runs when it closes.
#[allow(clippy::too_many_arguments)]
fn credentials_dialog(p: &Page, title: &str, intro: &str, primary: &str, user: &str, password: &str, save: Save, done: impl FnOnce(DialogResult) + 'static) {
    let form = gtk::Box::new(gtk::Orientation::Vertical, 12);
    if !intro.is_empty() {
        form.append(&c::label(intro, "", true));
    }
    let name = ed::entry(user, "", "remotedesktop-edit-user");
    form.append(&c::field(t("Gebruikersnaam", "User name"), &name, None));
    let pw = ed::password_entry("", "remotedesktop-edit-password");
    pw.set_text(password);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    pw.set_hexpand(true);
    row.append(&pw);
    let regen = c::icon_button("refresh", t("Nieuw wachtwoord genereren", "Generate new password"), 32);
    regen.set_widget_name("remotedesktop-edit-generate");
    {
        let pw = pw.clone();
        regen.connect_clicked(move |_| pw.set_text(&new_password()));
    }
    row.append(&regen);
    form.append(&c::field(t("Wachtwoord", "Password"), &row, None));
    form.append(&ed::show_characters(&[pw.clone()]));
    let err = ed::error_label();
    form.append(&err);
    Dialog::new(title)
        .body(&form)
        .primary(primary)
        .width(460)
        .on_primary(move |d| {
            let (u, w) = (name.text().trim().to_owned(), pw.text().to_string());
            if !credentials_ok(&u, &w) {
                ed::show_error(&err, t("Vul een gebruikersnaam en een wachtwoord in. De gebruikersnaam mag niet met een streepje beginnen.", "Enter a user name and a password. The user name can’t start with a hyphen."));
                return false;
            }
            save(d, &err, u, w);
            false
        })
        .show(&p.anchor(), done);
}

/// The rows of a set of sign-in details: user name with Kopiëren, the password hidden until Weergeven.
fn credential_rows(p: &Page, prefix: &str, user: &str, password: Option<&str>) -> Vec<gtk::Widget> {
    let mut out: Vec<gtk::Widget> = Vec::new();
    let u = SettingsCard::new("", t("Gebruikersnaam", "User name"), "");
    u.set_control(&copy_value(p, user, &format!("{prefix}-copy-user")));
    out.push(u.widget().clone());
    let w = SettingsCard::new("", t("Wachtwoord", "Password"), "");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let shown = Rc::new(Cell::new(false));
    let value = common::value_label("••••••••");
    value.set_widget_name(&format!("{prefix}-password"));
    value.set_selectable(true);
    row.append(&value);
    if let Some(pw) = password {
        let show = c::button(t("Weergeven", "Show"), Variant::Standard);
        show.set_widget_name(&format!("{prefix}-show"));
        let (value2, pw2, shown2) = (value.clone(), pw.to_owned(), shown.clone());
        show.connect_clicked(move |b| {
            shown2.set(!shown2.get());
            value2.set_label(if shown2.get() { &pw2 } else { "••••••••" });
            b.set_label(if shown2.get() { t("Verbergen", "Hide") } else { t("Weergeven", "Show") });
        });
        row.append(&show);
        let copy = c::button(t("Kopiëren", "Copy"), Variant::Standard);
        copy.set_widget_name(&format!("{prefix}-copy-password"));
        let (pp, text) = (p.clone(), pw.to_owned());
        copy.connect_clicked(move |_| {
            let ok = common::copy_text(&text);
            pp.toast(Toast::new(if ok { ct::copied() } else { ct::copy_failed() }).severity(if ok { Severity::Success } else { Severity::Error }).timeout(2000));
        });
        row.append(&copy);
    }
    w.set_control(&row);
    out.push(w.widget().clone());
    out
}

/// Run a host write; `done(result)` on success, the switch back (when given) on failure.
fn write(p: &Page, method: &'static str, args: Value, revert: Option<(ToggleSwitch, bool)>, done: impl FnOnce(Value) + 'static) {
    let wait = p.run(method, args);
    p.spawn(async move {
        match wait.await {
            Some(v) => done(v),
            None => {
                if let Some((sw, was)) = revert {
                    sw.set_active(was);
                }
            }
        }
    });
}

/// Desktop Sharing: the switch with its expander.
fn desktop_sharing(p: &Page, gs: &Gs, rdp: &Value, group: &SettingsGroup) {
    let exp = SettingsExpander::new(
        "remote-desktop",
        t("Extern bureaublad", "Remote Desktop"),
        t(
            "Gebruik een app voor Extern bureaublad (RDP) om verbinding te maken met deze pc en deze vanaf een ander apparaat te gebruiken",
            "Use a Remote Desktop (RDP) app to connect to and use this PC from another device",
        ),
    );
    exp.widget().set_widget_name("remotedesktop-expander");
    let sw = ToggleSwitch::new(rdp["enabled"] == true);
    sw.switch().set_widget_name("st-remotedesktop-enable");
    if !gs.writable(&ENABLE) && gs.available(&ENABLE) {
        sw.set_sensitive(false);
        sw.switch().set_tooltip_text(Some(gs::locked_text()));
    }
    {
        let (pp, sw2) = (p.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            let pp2 = pp.clone();
            let run = move |pp: &Page, sw: &ToggleSwitch| {
                let pp3 = pp.clone();
                write(pp, "sharing.rdp.set", json!({ "on": on }), Some((sw.clone(), !on)), move |v| {
                    common::note_dry_run(&pp3.anchor(), &v);
                    if v["credentials"] == "failed" {
                        pp3.toast(
                            Toast::new(t("Extern bureaublad staat aan, maar de aanmeldingsgegevens konden niet worden opgeslagen.", "Remote Desktop is on, but the sign-in details couldn’t be saved."))
                                .severity(Severity::Warning)
                                .timeout(6000),
                        );
                    }
                    pp3.refresh();
                });
            };
            if on {
                let sw3 = sw2.clone();
                c::confirm(
                    &pp.anchor(),
                    t("Extern bureaublad inschakelen?", "Enable Remote Desktop?"),
                    t(
                        "Andere apparaten kunnen dan met de aanmeldingsgegevens van deze pagina verbinding maken met je huidige bureaublad. De verbinding gebruikt de huidige schermresolutie.",
                        "Other devices can then connect to your current desktop with the sign-in details on this page. The connection uses the current screen resolution.",
                    ),
                    t("Bevestigen", "Confirm"),
                    false,
                    move |ok| {
                        if ok {
                            run(&pp2, &sw3);
                        } else {
                            sw3.set_active(false);
                        }
                    },
                );
            } else {
                run(&pp2, &sw2);
            }
        });
    }
    {
        let sw = sw.clone();
        gs.follow(&ENABLE, move |v| sw.set_active(*v == true));
    }
    exp.header().set_control(sw.widget());
    // Windows' Network Level Authentication: gnome-remote-desktop always asks for the sign-in details
    let nla = c::checkbox(t(
        "Apparaten verplichten verificatie op netwerkniveau te gebruiken om verbinding te maken (aanbevolen)",
        "Require devices to use Network Level Authentication to connect (Recommended)",
    ));
    nla.set_active(true);
    nla.set_sensitive(false);
    nla.set_tooltip_text(Some(t("Extern bureaublad vraagt altijd om een gebruikersnaam en wachtwoord.", "Remote Desktop always asks for a user name and password.")));
    nla.set_widget_name("remotedesktop-nla");
    nla.set_halign(gtk::Align::Start);
    let nla_row = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nla_row.set_margin_top(6);
    nla_row.set_margin_bottom(6);
    nla_row.append(&nla);
    exp.add(&nla_row);
    let port = rdp["port"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| "3389".to_owned());
    let pc = SettingsCard::new("", t("Poort van Extern bureaublad", "Remote Desktop port"), "");
    pc.set_control(&copy_value(p, &port, "remotedesktop-copy-port"));
    exp.add(pc.widget());
    if gs.available(&VIEW_ONLY) {
        let rc = SettingsCard::new("", t("Externe besturing", "Remote control"), t("Toestaan dat externe apparaten de muis en het toetsenbord bedienen", "Allow remote devices to use the mouse and keyboard"));
        let toggle = gs.toggle(&VIEW_ONLY, true);
        // GNOME: Remote Control can only be changed while Desktop Sharing is on
        let on = rdp["enabled"] == true;
        if !on {
            toggle.set_sensitive(false);
        }
        {
            let (toggle2, writable) = (toggle.clone(), gs.writable(&VIEW_ONLY));
            gs.follow(&ENABLE, move |v| toggle2.set_sensitive(*v == true && writable));
        }
        rc.set_control(toggle.widget());
        exp.add(rc.widget());
    }
    exp.set_expanded(rdp["enabled"] == true);
    group.add(exp.widget());
}

/// The user daemon's sign-in details: the rows, Bewerken, Nieuw wachtwoord genereren, Versleuteling controleren.
fn sign_in(p: &Page, rdp: &Value, creds: &Result<Value, kit::CallError>, group: &SettingsGroup) {
    let (user, password) = match creds {
        Ok(v) => (v["username"].as_str().map(str::to_owned), v["password"].as_str().map(str::to_owned)),
        Err(_) => (None, None),
    };
    let desc = match (creds, &user) {
        (Err(e), _) => e.message.clone(),
        (Ok(_), None) => t("Nog niet ingesteld. Bij het inschakelen worden je gebruikersnaam en een nieuw wachtwoord gebruikt.", "Not set yet. Turning it on uses your user name and a new password.").to_owned(),
        (Ok(_), Some(_)) => t("Gebruik deze gegevens om je aan te melden vanaf het andere apparaat", "Use these details to sign in from the other device").to_owned(),
    };
    let exp = SettingsExpander::new("user", t("Aanmeldingsgegevens", "Sign-in details"), &desc);
    exp.widget().set_widget_name("remotedesktop-credentials");
    let edit = c::button(t("Bewerken", "Edit"), Variant::Standard);
    edit.set_widget_name("remotedesktop-credentials-edit");
    {
        let (pp, u, w) = (p.clone(), user.clone().unwrap_or_else(|| glib::user_name().to_string_lossy().into_owned()), password.clone().unwrap_or_else(new_password));
        edit.connect_clicked(move |_| {
            let pp2 = pp.clone();
            credentials_dialog(
                &pp,
                t("Aanmeldingsgegevens bewerken", "Edit sign-in details"),
                "",
                t("Opslaan", "Save"),
                &u,
                &w,
                Rc::new(move |d: &Dialog, err: &gtk::Label, u: String, w: String| {
                    let pp3 = pp2.clone();
                    ed::dialog_call(d, err, "sharing.rdp.credentials.set", json!({ "username": u, "password": w }), move |v| {
                        common::note_dry_run(&pp3.anchor(), &v);
                        pp3.refresh();
                    });
                }),
                |_| {},
            );
        });
    }
    exp.header().set_control(&edit);
    if let Some(u) = &user {
        for w in credential_rows(p, "remotedesktop", u, password.as_deref()) {
            exp.add(&w);
        }
        exp.set_expanded(true);
    }
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::Start);
    buttons.set_margin_top(10);
    buttons.set_margin_bottom(10);
    let regen = c::button(t("Nieuw wachtwoord genereren", "Generate new password"), Variant::Standard);
    regen.set_widget_name("remotedesktop-generate");
    regen.set_sensitive(user.is_some());
    {
        let (pp, u) = (p.clone(), user.clone().unwrap_or_default());
        regen.connect_clicked(move |_| {
            let pp2 = pp.clone();
            write(&pp, "sharing.rdp.credentials.set", json!({ "username": u, "password": new_password() }), None, move |v| {
                common::note_dry_run(&pp2.anchor(), &v);
                pp2.refresh();
            });
        });
    }
    buttons.append(&regen);
    let verify = c::button(t("Versleuteling controleren", "Verify encryption"), Variant::Standard);
    verify.set_widget_name("remotedesktop-verify");
    {
        let (pp, fp) = (p.clone(), rdp["fingerprint"].as_str().map(str::to_owned));
        verify.connect_clicked(move |_| fingerprint_dialog(&pp, fp.as_deref()));
    }
    buttons.append(&verify);
    exp.add(&buttons);
    group.add(exp.widget());
}

/// Remote Login (the system daemon): its switch asks for the sign-in details, then turns it on.
fn remote_login(p: &Page, sys: &Value, out: &mut Content) {
    let group = SettingsGroup::new(t("Extern aanmelden", "Remote login"), "");
    let exp = SettingsExpander::new(
        "lock",
        t("Extern aanmelden", "Remote login"),
        t(
            "Op afstand aanmelden bij je account wanneer niemand deze pc gebruikt. De schermresolutie kan op afstand worden ingesteld.",
            "Sign in to your account remotely when nobody is using this PC. The screen resolution can be set remotely.",
        ),
    );
    exp.widget().set_widget_name("remotedesktop-login");
    let sw = ToggleSwitch::new(sys["enabled"] == true);
    sw.switch().set_widget_name("remotedesktop-login-switch");
    sw.switch().set_tooltip_text(Some(t("Hiervoor is een beheerderswachtwoord nodig.", "This needs an administrator password.")));
    {
        let (pp, sw2) = (p.clone(), sw.clone());
        sw.connect_toggled(move |on| {
            if !on {
                let pp2 = pp.clone();
                write(&pp, "sharing.rdp.system.set", json!({ "on": false }), Some((sw2.clone(), true)), move |_| pp2.refresh());
                return;
            }
            // the stored details (GetCredentials asks for the password), else GNOME's defaults
            let wait = pp.run("sharing.rdp.credentials.get", json!({ "system": true }));
            let (pp2, sw3) = (pp.clone(), sw2.clone());
            pp.spawn(async move {
                let Some(stored) = wait.await else {
                    sw3.set_active(false);
                    return;
                };
                let user = stored["username"].as_str().map(str::to_owned).unwrap_or_else(|| glib::user_name().to_string_lossy().into_owned());
                let pw = stored["password"].as_str().map(str::to_owned).unwrap_or_else(new_password);
                let (pp4, sw4) = (pp2.clone(), sw3.clone());
                credentials_dialog(
                    &pp2,
                    t("Extern aanmelden inschakelen?", "Turn on remote login?"),
                    t("Met deze gegevens kun je je op afstand aanmelden bij deze pc.", "Use these details to sign in to this PC remotely."),
                    t("Bevestigen", "Confirm"),
                    &user,
                    &pw,
                    Rc::new(move |d: &Dialog, err: &gtk::Label, u: String, w: String| {
                        let (pp5, sw5) = (pp4.clone(), sw4.clone());
                        ed::dialog_call(d, err, "sharing.rdp.system.credentials.set", json!({ "username": u, "password": w }), move |_| {
                            let pp6 = pp5.clone();
                            write(&pp5, "sharing.rdp.system.set", json!({ "on": true }), Some((sw5, false)), move |v| {
                                common::note_dry_run(&pp6.anchor(), &v);
                                pp6.refresh();
                            });
                        });
                    }),
                    // closed without Bevestigen: the switch back
                    move |r| {
                        if r != DialogResult::Primary {
                            sw3.set_active(false);
                        }
                    },
                );
            });
        });
    }
    exp.header().set_control(sw.widget());
    let port = sys["port"].as_u64().map(|n| n.to_string()).unwrap_or_else(|| "3389".to_owned());
    let pc = SettingsCard::new("", t("Poort", "Port"), "");
    pc.set_control(&copy_value(p, &port, "remotedesktop-login-copy-port"));
    exp.add(pc.widget());
    // the sign-in details: read only when asked (the daemon asks for the administrator password)
    let details = common::slot();
    let creds = SettingsCard::new("", t("Aanmeldingsgegevens", "Sign-in details"), t("Hiervoor is een beheerderswachtwoord nodig.", "This needs an administrator password."));
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let show = pickers::admin_button(t("Weergeven", "Show"), Variant::Standard);
    show.set_widget_name("remotedesktop-login-show");
    {
        let (pp, details) = (p.clone(), details.clone());
        show.connect_clicked(move |b| {
            let wait = pp.run("sharing.rdp.credentials.get", json!({ "system": true }));
            let (pp2, details, b) = (pp.clone(), details.clone(), b.clone());
            pp.spawn(async move {
                if let Some(v) = wait.await {
                    let user = s(&v["username"]).to_owned();
                    let rows = if user.is_empty() {
                        vec![SettingsCard::new("", t("Nog niet ingesteld", "Not set yet"), "").widget().clone()]
                    } else {
                        credential_rows(&pp2, "remotedesktop-login", &user, v["password"].as_str())
                    };
                    common::set_slot(&details, &rows);
                    b.set_visible(false);
                }
            });
        });
    }
    row.append(&show);
    let edit = pickers::admin_button(t("Bewerken", "Edit"), Variant::Standard);
    edit.set_widget_name("remotedesktop-login-edit");
    {
        let pp = p.clone();
        edit.connect_clicked(move |_| {
            let pp2 = pp.clone();
            credentials_dialog(
                &pp,
                t("Aanmeldingsgegevens bewerken", "Edit sign-in details"),
                "",
                t("Opslaan", "Save"),
                &glib::user_name().to_string_lossy(),
                &new_password(),
                Rc::new(move |d: &Dialog, err: &gtk::Label, u: String, w: String| {
                    let pp3 = pp2.clone();
                    ed::dialog_call(d, err, "sharing.rdp.system.credentials.set", json!({ "username": u, "password": w }), move |v| {
                        common::note_dry_run(&pp3.anchor(), &v);
                        pp3.refresh();
                    });
                }),
                |_| {},
            );
        });
    }
    row.append(&edit);
    creds.set_control(&row);
    exp.add(creds.widget());
    exp.add(&details);
    let verify = SettingsCard::new("", t("Versleuteling controleren", "Verify encryption"), "");
    let vb = c::button(t("Weergeven", "Show"), Variant::Standard);
    vb.set_widget_name("remotedesktop-login-verify");
    {
        let (pp, fp) = (p.clone(), sys["fingerprint"].as_str().map(str::to_owned));
        vb.connect_clicked(move |_| fingerprint_dialog(&pp, fp.as_deref()));
    }
    verify.set_control(&vb);
    exp.add(verify.widget());
    group.add(exp.widget());
    out.push(group.widget().clone().upcast());
}

/// What the Extern aanmelden card shows of the system daemon: (there, on, port, fingerprint).
pub fn login_shape(sys: Option<&Value>) -> (bool, bool, Value, Value) {
    match sys {
        Some(v) => (v["available"] == true, v["enabled"] == true, v["port"].clone(), v["fingerprint"].clone()),
        None => (false, false, Value::Null, Value::Null),
    }
}

fn remotedesktop(p: Page) -> PageFuture {
    Box::pin(async move {
        let (rr, cr, dr, sr) = (
            p.call("sharing.rdp.get", json!({})),
            p.call("sharing.rdp.credentials.get", json!({})),
            p.get("net.devices", json!({})),
            p.get("sharing.rdp.system.get", json!({})),
        );
        let gs = Gs::load(&p, &[ENABLE, VIEW_ONLY]).await;
        let (rdp, creds, devs, sys) = (rr.await, cr.await, dr.await, sr.await);
        if !p.alive() {
            return Content::new();
        }
        let rdp = match rdp {
            Ok(v) if v.is_object() => v,
            Err(e) if e.code == "unknown_method" => return vec![common::notice(ct::unavailable_here()).widget().clone().upcast()],
            Err(e) => return vec![common::notice(&e.message).widget().clone().upcast()],
            _ => return vec![common::notice(ct::unavailable()).widget().clone().upcast()],
        };
        if rdp["available"] != true {
            let bar = common::notice_full(
                Severity::Warning,
                t("Extern bureaublad is niet beschikbaar", "Remote Desktop isn’t available"),
                t("gnome-remote-desktop is niet geïnstalleerd. Installeer het pakket gnome-remote-desktop om deze pc op afstand te gebruiken.", "gnome-remote-desktop isn’t installed. Install the gnome-remote-desktop package to use this PC remotely."),
            );
            bar.widget().set_widget_name("remotedesktop-unavailable");
            return vec![bar.widget().clone().upcast()];
        }
        let mut out: Content = Vec::new();
        if rdp["enabled"] == true && rdp["running"] != true {
            let bar = common::notice_full(Severity::Warning, "", t("Extern bureaublad staat aan, maar wordt niet uitgevoerd. Zet het uit en weer aan om het opnieuw te starten.", "Remote Desktop is on but isn’t running. Turn it off and on again to restart it."));
            bar.widget().set_widget_name("remotedesktop-not-running");
            out.push(bar.widget().clone().upcast());
        }
        if rdp["tls"] == false && rdp["certificate"] != true {
            let bar = common::notice_full(Severity::Warning, "", t("GnuTLS ontbreekt: het certificaat voor Extern bureaublad kan niet worden gemaakt.", "GnuTLS is missing: the Remote Desktop certificate can’t be created."));
            out.push(bar.widget().clone().upcast());
        }
        let group = SettingsGroup::new("", "");
        desktop_sharing(&p, &gs, &rdp, &group);
        sign_in(&p, &rdp, &creds, &group);
        out.push(group.widget().clone().upcast());

        // GNOME's How to Connect: Windows' Pc-naam, and the addresses
        let connect = SettingsGroup::new(
            t("Verbinding maken", "How to connect"),
            t(
                "Gebruik een app voor Extern bureaublad met het RDP-protocol. Soms is ook meer informatie over de netwerklocatie van dit apparaat nodig.",
                "Use a Remote Desktop app with the RDP protocol. Additional information about this device’s network location may also be required.",
            ),
        );
        let host = s(&rdp["hostname"]).to_owned();
        let name = SettingsCard::new("pc", t("Pc-naam", "PC name"), t("Gebruik deze pc-naam om vanaf je andere apparaat verbinding te maken met deze pc", "Use this PC name to connect to this PC from your other device"));
        name.set_control(&copy_value(&p, &host, "remotedesktop-copy-name"));
        connect.add(name.widget());
        let ips = devs.as_ref().map(addresses).unwrap_or_default();
        if !ips.is_empty() {
            let ip = SettingsCard::new("network", t("IP-adres", "IP address"), t("Of maak verbinding met dit adres", "Or connect to this address"));
            ip.set_control(&copy_value(&p, &ips.join(", "), "remotedesktop-copy-ip"));
            connect.add(ip.widget());
        }
        out.push(connect.widget().clone().upcast());

        let sys_seen = login_shape(sys.as_ref());
        if let Some(sys) = sys.filter(|v| v["available"] == true) {
            remote_login(&p, &sys, &mut out);
        }
        {
            // Desktop Sharing (the user daemon: rdp) and Remote Login (the system daemon: system, also when GNOME
            // Settings or `grdctl --system` changed it): the page again when what it shows changed
            let pp = p.clone();
            let last = RefCell::new((rdp["enabled"].clone(), rdp["running"].clone(), rdp["port"].clone()));
            p.on("sharing.changed", move |d| {
                let pp2 = pp.clone();
                match s(&d["what"]) {
                    "rdp" => {
                        let wait = pp.get("sharing.rdp.get", json!({}));
                        let seen = last.borrow().clone();
                        pp.spawn(async move {
                            if let Some(v) = wait.await
                                && (v["enabled"].clone(), v["running"].clone(), v["port"].clone()) != seen
                            {
                                pp2.refresh();
                            }
                        });
                    }
                    "system" => {
                        let (wait, seen) = (pp.get("sharing.rdp.system.get", json!({})), sys_seen.clone());
                        pp.spawn(async move {
                            if let Some(v) = wait.await
                                && login_shape(Some(&v)) != seen
                            {
                                pp2.refresh();
                            }
                        });
                    }
                    _ => {}
                }
            });
        }
        let related = SettingsGroup::new(ct::related(), "");
        for id in ["power", "sharing-advanced", "about"] {
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
    fn fingerprints_addresses_and_credentials() {
        let fp = (0..32).map(|i| format!("{:02X}", i * 7 % 256)).collect::<Vec<_>>().join(":");
        let lines: Vec<&str> = fingerprint_lines(&fp).leak().lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines.iter().all(|l| l.split(':').count() == 8));
        let devs = json!({ "devices": [
            { "ip4": { "addresses": [{ "address": "192.168.1.20" }, { "address": "127.0.0.1" }] },
              "ip6": { "addresses": [{ "address": "fe80::1" }, { "address": "2001:DB8::20" }] } },
            { "ip4": { "addresses": [{ "address": "169.254.3.4" }, { "address": "192.168.1.20" }] } },
        ]});
        assert_eq!(addresses(&devs), ["192.168.1.20", "2001:db8::20"]);
        let on = json!({ "available": true, "enabled": true, "port": 3389, "fingerprint": "AB:CD" });
        let mut off = on.clone();
        off["enabled"] = json!(false);
        assert_ne!(login_shape(Some(&on)), login_shape(Some(&off)), "turned on or off elsewhere");
        let mut other = on.clone();
        other["fingerprint"] = json!("EF:01");
        assert_ne!(login_shape(Some(&on)), login_shape(Some(&other)), "another certificate imported");
        let mut hostname = on.clone();
        hostname["hostname"] = json!("pc");
        assert_eq!(login_shape(Some(&on)), login_shape(Some(&hostname)));
        assert_ne!(login_shape(None), login_shape(Some(&on)), "the daemon appeared");
        let pw = new_password();
        assert_eq!(pw.len(), 16);
        assert!(credentials_ok("robin", &pw));
        assert!(!credentials_ok("-r", "x") && !credentials_ok("robin", "") && !credentials_ok("a", "b\nc"));
    }
}
