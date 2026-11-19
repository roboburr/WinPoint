//! Instellingen › Bluetooth en apparaten › Printers en scanners and one printer (batch B, notes/claude/settings-parity.md
//! §2): Windows 11's pages holding GNOME Settings' Printers panel (cc-printers-panel.c, pp-new-printer-dialog.c,
//! pp-printer-entry.c, pp-jobs-dialog.c, pp-options-dialog.c, pp-details-dialog.c) through `printers.*`.
//!
//! * **printers**: "Een printer of scanner toevoegen · Apparaat toevoegen" searches the network and the scheduler's
//!   backends (printers.discover) and lists what it found under the card, each with its own "Apparaat toevoegen": a
//!   driverless (IPP Everywhere) printer is added at once (ppd `everywhere`, amendment 41), another one first asks for
//!   its driver ("Stuurprogramma kiezen": the best matches, the list by manufacturer, a PPD file). "De printer die ik
//!   wil gebruiken, staat niet in de lijst · Handmatig toevoegen" is GNOME's address entry ("Een printer toevoegen met
//!   behulp van een IP-adres of hostnaam", with the print server's user and password when it asks). The installed
//!   printers follow, each opening its page; GNOME's "No Print Service" when cupsd does not answer; Windows'
//!   "Scanner openen" starts Documentscanner (GNOME has no scanner panel).
//! * **printer** (a data page, `{name}`): Windows' printer page: the status with GNOME's "restart after an error"
//!   ("Hervatten"), "Als standaard instellen", "Verwijderen" (with Windows' question) and "⋯ Printkoppen reinigen";
//!   "Afdrukwachtrij openen" (GNOME's Jobs dialog: onderbreken, hervatten, annuleren, bovenaan zetten, verifiëren,
//!   alle documenten annuleren), "Testpagina afdrukken", "Printereigenschappen" (Naam, Locatie, Adres,
//!   Stuurprogramma: GNOME's Printer Details) and "Voorkeursinstellingen voor afdrukken" (GNOME's Printing Options:
//!   the PPD's groups and the IPP options, each change written at once); the ink and toner levels and "Meer
//!   informatie".
//! * Live: printers.changed.
use super::{Builder, Page, PageFuture};
use crate::common;
use kit::controls::{
    self as c, Chevron, Dropdown, DropdownOption, MenuItem, RadioGroup, SettingsCard, SettingsExpander,
    SettingsGroup, Severity, Variant,
};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("printers", list_page), ("printer", printer_page)];
pub const CSS: &str = r#"
.st-prn-key { color: var(--text-2); }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

// ------------------------------------------------------------------------------------------- pure helpers

/// A printer-state-reason as GNOME's printer row words it (pp-printer-entry.c), in Windows 11's terms.
pub(super) fn reason_text(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "media-low" => t("Papier bijna op", "Low on paper"),
        "media-empty" | "media-needed" => t("Papier op", "Out of paper"),
        "media-jam" => t("Papierstoring", "Paper jam"),
        "toner-low" => t("Toner bijna op", "Low on toner"),
        "toner-empty" => t("Toner op", "Out of toner"),
        "marker-supply-low" => t("Inkt bijna op", "Low on ink"),
        "marker-supply-empty" => t("Inkt op", "Out of ink"),
        "marker-waste-almost-full" => t("Opvangbak bijna vol", "Waste receptacle almost full"),
        "marker-waste-full" => t("Opvangbak vol", "Waste receptacle full"),
        "door-open" => t("Klep open", "Door open"),
        "cover-open" => t("Deksel open", "Cover open"),
        "offline" | "shutdown" => t("Offline", "Offline"),
        "connecting-to-device" => t("Verbinden met de printer…", "Connecting to the printer…"),
        "opc-near-eol" => t("Drum bijna versleten", "Drum almost worn out"),
        "opc-life-over" => t("Drum versleten", "Drum worn out"),
        "developer-low" => t("Ontwikkelaar bijna op", "Low on developer"),
        "developer-empty" => t("Ontwikkelaar op", "Out of developer"),
        "cups-missing-filter" => t("Stuurprogramma onvolledig", "Driver incomplete"),
        "spool-area-full" => t("Afdrukwachtrij vol", "Print queue full"),
        _ => return None,
    })
}

/// The status line of a printer (Windows' "Inactief", "Bezig met afdrukken", "Onderbroken", or GNOME's reason).
pub(super) fn status_text(p: &Value) -> String {
    let reasons = p["reasons"].as_array().cloned().unwrap_or_default();
    let worst = reasons
        .iter()
        .filter(|r| s(&r["severity"]) == "error")
        .chain(reasons.iter().filter(|r| s(&r["severity"]) == "warning"))
        .find_map(|r| reason_text(s(&r["reason"])));
    if let Some(w) = worst {
        return w.to_owned();
    }
    if p["accepting"] == false {
        return t("Weigert afdruktaken", "Rejecting print jobs").to_owned();
    }
    match s(&p["state"]) {
        "processing" => t("Bezig met afdrukken", "Printing").to_owned(),
        "stopped" => t("Onderbroken", "Paused").to_owned(),
        "idle" => t("Inactief", "Idle").to_owned(),
        _ => t("Onbekend", "Unknown").to_owned(),
    }
}

/// Whether GNOME's "restart printer after an error" applies: the queue is stopped or rejects jobs.
pub(super) fn needs_resume(p: &Value) -> bool {
    s(&p["state"]) == "stopped" || p["accepting"] == false
}

/// A queue name for a new printer from its name or model, as g-c-c's canonicalize_device_name: CUPS's
/// forbidden characters, spaces and non-ASCII become '-', runs of '-' collapse, at most 120 bytes, unique among
/// `taken`.
pub(super) fn queue_name(text: &str, taken: &[String]) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        let bad = ch.is_whitespace()
            || ch.is_control()
            || !ch.is_ascii()
            || matches!(ch, '/' | '\\' | '?' | '\'' | '"' | '#' | ',');
        if !bad {
            out.push(ch);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut base = out.trim_matches('-').to_owned();
    base.truncate(120);
    if base.is_empty() {
        base = "Printer".into();
    }
    let lower: Vec<String> = taken.iter().map(|n| n.to_lowercase()).collect();
    if !lower.contains(&base.to_lowercase()) {
        return base;
    }
    (2..1000).map(|n| format!("{base}-{n}")).find(|c| !lower.contains(&c.to_lowercase())).unwrap_or(base)
}

/// The words of a device's kind: Windows' "Netwerkprinter", "USB-printer", ...
fn device_kind(d: &Value) -> &'static str {
    let uri = s(&d["uri"]);
    if uri.starts_with("usb:") || uri.starts_with("ippusb:") {
        t("USB-printer", "USB printer")
    } else if uri.starts_with("smb:") {
        t("Gedeelde Windows-printer (SMB)", "Shared Windows printer (SMB)")
    } else if d["remoteQueue"] == true {
        t("Gedeelde printer op een afdrukserver", "Shared printer on a print server")
    } else if d["network"] == true || uri.contains("://") {
        t("Netwerkprinter", "Network printer")
    } else {
        t("Printer", "Printer")
    }
}

fn device_title(d: &Value) -> String {
    [s(&d["info"]), s(&d["makeModel"]), s(&d["uri"])].into_iter().find(|x| !x.trim().is_empty()).unwrap_or("").to_owned()
}

/// A job's state as words (GNOME's Jobs dialog).
pub(super) fn job_state_text(j: &Value) -> &'static str {
    if j["authRequired"] == true {
        return t("Verificatie vereist", "Authentication required");
    }
    match s(&j["state"]) {
        "pending" => t("In de wachtrij", "Queued"),
        "held" => t("Onderbroken", "Paused"),
        "processing" => t("Bezig met afdrukken", "Printing"),
        "stopped" => t("Gestopt", "Stopped"),
        "canceled" => t("Geannuleerd", "Cancelled"),
        "aborted" => t("Afgebroken", "Aborted"),
        "completed" => t("Voltooid", "Completed"),
        _ => t("Onbekend", "Unknown"),
    }
}

/// The text of an IPP option value (GNOME's pp-ipp-option-widget.c).
pub(super) fn ipp_choice_text(option: &str, value: &str) -> String {
    match (option, value) {
        ("orientation-requested", "3") => t("Staand", "Portrait").to_owned(),
        ("orientation-requested", "4") => t("Liggend", "Landscape").to_owned(),
        ("orientation-requested", "5") => t("Liggend (omgekeerd)", "Reverse landscape").to_owned(),
        ("orientation-requested", "6") => t("Staand (omgekeerd)", "Reverse portrait").to_owned(),
        ("sides", "one-sided") => t("Enkelzijdig", "One-sided").to_owned(),
        ("sides", "two-sided-long-edge") => t("Dubbelzijdig, lange zijde", "Two-sided, long edge").to_owned(),
        ("sides", "two-sided-short-edge") => t("Dubbelzijdig, korte zijde", "Two-sided, short edge").to_owned(),
        _ => value.to_owned(),
    }
}

fn ipp_option_text(option: &str) -> &'static str {
    match option {
        "number-up" => t("Pagina's per vel", "Pages per sheet"),
        "orientation-requested" => t("Afdrukstand", "Orientation"),
        "sides" => t("Dubbelzijdig afdrukken", "Print on both sides"),
        _ => "",
    }
}

fn kv_line(title: &str, value: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let k = c::label(title, "st-prn-key", false);
    k.set_size_request(160, -1);
    k.set_xalign(0.0);
    row.append(&k);
    let v = c::label(value, "", true);
    v.set_selectable(true);
    v.set_hexpand(true);
    v.set_xalign(0.0);
    row.append(&v);
    row
}

/// "#RRGGBB" as cairo's 0..1 channels.
pub(super) fn rgb(hex: &str) -> Option<(f64, f64, f64)> {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some((((v >> 16) & 255) as f64 / 255.0, ((v >> 8) & 255) as f64 / 255.0, (v & 255) as f64 / 255.0))
}

/// A level bar of one ink or toner, in the marker's own colours.
fn ink_bar(level: Option<i64>, colors: &[String]) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_content_width(180);
    area.set_content_height(10);
    area.set_valign(gtk::Align::Center);
    let rgbs: Vec<(f64, f64, f64)> = colors.iter().filter_map(|c| rgb(c)).collect();
    let fraction = level.map(|l| (l as f64 / 100.0).clamp(0.0, 1.0)).unwrap_or(0.0);
    area.set_draw_func(move |_, cr, w, h| {
        let (w, h) = (w as f64, h as f64);
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.25);
        cr.rectangle(0.0, 0.0, w, h);
        let _ = cr.fill();
        let fill = w * fraction;
        if rgbs.is_empty() {
            cr.set_source_rgb(0.35, 0.35, 0.35);
            cr.rectangle(0.0, 0.0, fill, h);
            let _ = cr.fill();
        } else {
            let part = fill / rgbs.len() as f64;
            for (i, (r, g, b)) in rgbs.iter().enumerate() {
                cr.set_source_rgb(*r, *g, *b);
                cr.rectangle(part * i as f64, 0.0, part, h);
                let _ = cr.fill();
            }
        }
        cr.set_source_rgba(0.5, 0.5, 0.5, 0.6);
        cr.set_line_width(1.0);
        cr.rectangle(0.5, 0.5, w - 1.0, h - 1.0);
        let _ = cr.stroke();
    });
    area
}

// -------------------------------------------------------------------------------------------- the driver

/// What the user chose in "Stuurprogramma kiezen": a PPD name (`everywhere` too) or a PPD file.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Driver {
    Ppd(String),
    File(String),
}

impl Driver {
    fn args(&self) -> Value {
        match self {
            Driver::Ppd(n) => json!({ "ppd": n }),
            Driver::File(f) => json!({ "ppdFile": f }),
        }
    }
}

/// The models of one manufacturer for the driver dropdown (IPP Everywhere has its own choice).
fn models_of(all: &[Value], make: &str) -> Vec<DropdownOption> {
    all.iter()
        .filter(|d| s(&d["make"]) == make && s(&d["name"]) != "everywhere")
        .map(|d| DropdownOption::new(s(&d["name"]), s(&d["makeModel"])))
        .collect()
}

/// The manufacturer a device's make-and-model or IEEE 1284 id names, else Generic, else the first.
pub(super) fn guess_make(makes: &[String], make_model: &str, device_id: &str) -> String {
    let (mm, id) = (make_model.to_lowercase(), device_id.to_lowercase());
    makes
        .iter()
        .find(|m| {
            let m = m.to_lowercase();
            !m.is_empty() && (mm.starts_with(&m) || id.contains(&format!("mfg:{m}")))
        })
        .or_else(|| makes.iter().find(|m| m.as_str() == "Generic"))
        .or_else(|| makes.first())
        .cloned()
        .unwrap_or_default()
}

/// GNOME's Select Printer Driver for `device`: Automatisch (IPP Everywhere) for a printer that speaks IPP, the best
/// matches of CUPS's list (printers.drivers), any driver by manufacturer and model, or a PPD file.
async fn choose_driver(p: &Page, device: &Value, action: &str) -> Option<Driver> {
    let driverless = device["driverless"] == true;
    let drivers = p
        .call(
            "printers.drivers",
            json!({"deviceId": s(&device["deviceId"]), "makeModel": s(&device["makeModel"]), "driverless": driverless}),
        )
        .await;
    if !p.alive() {
        return None;
    }
    let drivers = match drivers {
        Ok(v) => v,
        Err(e) => {
            p.error(&e, Some(t("De lijst met stuurprogramma's kon niet worden geladen", "Couldn’t load the list of drivers")));
            return None;
        }
    };
    let all: Vec<Value> = drivers["drivers"].as_array().cloned().unwrap_or_default();
    let recommended: Vec<Value> =
        drivers["best"].as_array().into_iter().flatten().filter(|b| s(&b["name"]) != "everywhere").cloned().collect();
    let uri = s(&device["uri"]);
    let ipp_capable = drivers["everywhere"] == true && (driverless || uri.starts_with("ipp") || uri.starts_with("dnssd"));
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.set_widget_name("printer-driver");
    body.append(&c::label(
        &common::fill(t("Kies het stuurprogramma voor “{name}”.", "Choose the driver for “{name}”."), &[("name", &device_title(device))]),
        "",
        true,
    ));
    let mut modes: Vec<(&str, &str)> = Vec::new();
    if ipp_capable {
        modes.push(("auto", t("Automatisch (IPP Everywhere)", "Automatic (IPP Everywhere)")));
    }
    if !recommended.is_empty() {
        modes.push(("best", t("Aanbevolen stuurprogramma", "Recommended driver")));
    }
    modes.push(("list", t("Uit de lijst kiezen", "Choose from the list")));
    modes.push(("file", t("PPD-bestand installeren", "Install a PPD file")));
    let first = modes[0].0;
    let radio = RadioGroup::new(&modes, Some(first), None, false);
    radio.widget().set_widget_name("printer-driver-mode");
    body.append(radio.widget());

    let best_dd = Dropdown::new(
        recommended.iter().map(|d| DropdownOption::new(s(&d["name"]), s(&d["makeModel"]))).collect(),
        recommended.first().map(|d| s(&d["name"])),
    );
    best_dd.widget().set_widget_name("printer-driver-best");
    let mut makes: Vec<String> = all.iter().map(|d| s(&d["make"]).to_owned()).filter(|m| !m.is_empty()).collect();
    makes.dedup();
    let guess = guess_make(&makes, s(&device["makeModel"]), s(&device["deviceId"]));
    let make_dd = Dropdown::new(makes.iter().map(|m| DropdownOption::new(m, m)).collect(), Some(&guess));
    make_dd.widget().set_widget_name("printer-driver-make");
    let first_models = models_of(&all, &guess);
    let first_model = first_models.first().map(|o| o.value.clone());
    let model_dd = Dropdown::new(first_models, first_model.as_deref());
    model_dd.widget().set_widget_name("printer-driver-model");
    {
        let (model_dd, all) = (model_dd.clone(), all.clone());
        make_dd.connect_changed(move |make| {
            let opts = models_of(&all, make);
            let first = opts.first().map(|o| o.value.clone());
            model_dd.set_options(opts);
            model_dd.set_selected(first.as_deref());
        });
    }
    let file: Rc<RefCell<Option<String>>> = Rc::default();
    let file_label = c::label(t("Geen bestand gekozen", "No file chosen"), "st-prn-key", true);
    let browse = c::button(t("Bladeren…", "Browse…"), Variant::Standard);
    browse.set_widget_name("printer-driver-browse");
    {
        let (pp, file, file_label) = (p.clone(), file.clone(), file_label.clone());
        browse.connect_clicked(move |_| {
            let filter = gtk::FileFilter::new();
            filter.set_name(Some(t("PPD-bestanden", "PPD files")));
            for pattern in ["*.ppd", "*.PPD", "*.ppd.gz"] {
                filter.add_pattern(pattern);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            let dialog = gtk::FileDialog::new();
            dialog.set_title(t("Een PPD-bestand kiezen", "Choose a PPD file"));
            dialog.set_modal(true);
            dialog.set_filters(Some(&filters));
            dialog.set_default_filter(Some(&filter));
            let (file, file_label) = (file.clone(), file_label.clone());
            dialog.open(Some(pp.window().window()), gio::Cancellable::NONE, move |result| {
                if let Ok(f) = result
                    && let Some(path) = f.path().and_then(|p| p.to_str().map(str::to_owned))
                {
                    file_label.set_text(&path);
                    *file.borrow_mut() = Some(path);
                }
            });
        });
    }
    let slot = gtk::Box::new(gtk::Orientation::Vertical, 8);
    body.append(&slot);
    let show = {
        let (slot, best_dd, make_dd, model_dd, browse, file_label) =
            (slot.clone(), best_dd.clone(), make_dd.clone(), model_dd.clone(), browse.clone(), file_label.clone());
        move |mode: &str| {
            c::clear(&slot);
            match mode {
                "auto" => slot.append(&c::label(
                    t(
                        "De printer geeft zelf door wat hij kan; er is geen apart stuurprogramma nodig.",
                        "The printer reports its own features; no separate driver is needed.",
                    ),
                    "st-prn-key",
                    true,
                )),
                "best" => slot.append(&c::field(t("Stuurprogramma", "Driver"), best_dd.widget(), None)),
                "list" => {
                    slot.append(&c::field(t("Fabrikant", "Manufacturer"), make_dd.widget(), None));
                    slot.append(&c::field(t("Model", "Model"), model_dd.widget(), None));
                }
                _ => {
                    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    row.append(&browse);
                    file_label.set_hexpand(true);
                    row.append(&file_label);
                    slot.append(&row);
                }
            }
        }
    };
    show(first);
    radio.connect_changed(show);
    let (fill, wait) = super::later::<Option<Driver>>();
    let fill = Rc::new(RefCell::new(Some(fill)));
    let chosen: Rc<RefCell<Option<Driver>>> = Rc::default();
    let dialog = c::Dialog::new(t("Stuurprogramma kiezen", "Choose a driver"))
        .body(&body)
        .width(520)
        .primary(action)
        .close_label(t("Annuleren", "Cancel"));
    let dialog = {
        let (radio, best_dd, model_dd, file, chosen, pp) =
            (radio.clone(), best_dd.clone(), model_dd.clone(), file.clone(), chosen.clone(), p.clone());
        dialog.on_primary(move |_| {
            let pick = match radio.value().as_deref() {
                Some("auto") => Some(Driver::Ppd("everywhere".into())),
                Some("best") => best_dd.selected().map(Driver::Ppd),
                Some("list") => model_dd.selected().map(Driver::Ppd),
                _ => file.borrow().clone().map(Driver::File),
            };
            match pick {
                Some(d) => {
                    *chosen.borrow_mut() = Some(d);
                    true
                }
                None => {
                    common::show_error_text(&pp.anchor(), t("Kies eerst een stuurprogramma.", "Choose a driver first."), None);
                    false
                }
            }
        })
    };
    dialog.show(&p.anchor(), move |r| {
        if let Some(f) = fill.borrow_mut().take() {
            f(if r == c::DialogResult::Primary { chosen.borrow_mut().take() } else { None });
        }
    });
    wait.await
}

// ------------------------------------------------------------------------------------------ printers page

struct ListPage {
    p: Page,
    state: RefCell<Value>,
    /// What "Apparaat toevoegen" found (None before the first search).
    found: RefCell<Option<Vec<Value>>>,
    searching: Cell<bool>,
    /// The devices being added (their URIs).
    adding: RefCell<Vec<String>>,
    /// GNOME's printer search (shown from five printers on; Windows has none).
    search: c::SearchBox,
    query: RefCell<String>,
    add_slot: gtk::Box,
    body: gtk::Box,
}

impl ListPage {
    async fn reload(self: &Rc<Self>) {
        let st = self.p.get("printers.list", json!({})).await;
        if let Some(st) = st.filter(|v| v.is_object())
            && self.p.alive()
        {
            *self.state.borrow_mut() = st;
            self.paint();
            self.paint_found();
        }
    }

    fn names(&self) -> Vec<String> {
        self.state.borrow()["printers"].as_array().map(|a| a.iter().map(|p| s(&p["name"]).to_owned()).collect()).unwrap_or_default()
    }

    fn installed(&self, uri: &str) -> bool {
        self.state.borrow()["printers"].as_array().is_some_and(|a| a.iter().any(|p| s(&p["deviceUri"]) == uri))
    }

    fn search(self: &Rc<Self>) {
        if self.searching.get() {
            return;
        }
        self.searching.set(true);
        self.paint_found();
        let me = self.clone();
        self.p.spawn(async move {
            let r = me.p.call("printers.discover", json!({})).await;
            me.searching.set(false);
            if !me.p.alive() {
                return;
            }
            match r {
                Ok(v) => {
                    if v["denied"] == true {
                        common::show_error_text(
                            &me.p.anchor(),
                            t(
                                "Alleen printers op het netwerk worden getoond: voor de lokale printers is toestemming nodig.",
                                "Only printers on the network are shown: local printers need permission.",
                            ),
                            None,
                        );
                    }
                    *me.found.borrow_mut() = Some(v["devices"].as_array().cloned().unwrap_or_default());
                }
                Err(e) => {
                    me.p.error(&e, Some(t("Zoeken naar printers lukte niet", "Couldn’t search for printers")));
                    *me.found.borrow_mut() = Some(Vec::new());
                }
            }
            me.paint_found();
        });
    }

    /// Add a found device: a driverless one at once with IPP Everywhere, another after "Stuurprogramma kiezen".
    fn add(self: &Rc<Self>, device: Value) {
        let me = self.clone();
        self.p.spawn(async move {
            let driver = if device["driverless"] == true {
                Some(Driver::Ppd("everywhere".into()))
            } else {
                choose_driver(&me.p, &device, t("Toevoegen", "Add")).await
            };
            let Some(driver) = driver else { return };
            if !me.p.alive() {
                return;
            }
            let uri = s(&device["uri"]).to_owned();
            let title = device_title(&device);
            let name = queue_name(&title, &me.names());
            let mut args = json!({ "name": name, "uri": uri, "info": title, "location": s(&device["location"]) });
            for (k, v) in driver.args().as_object().cloned().unwrap_or_default() {
                args[k] = v;
            }
            me.adding.borrow_mut().push(uri.clone());
            me.paint_found();
            let r = me
                .p
                .run_with("printers.add", args, Some(t("De printer kon niet worden toegevoegd", "Couldn’t add the printer")), false)
                .await;
            me.adding.borrow_mut().retain(|u| u != &uri);
            if !me.p.alive() {
                return;
            }
            if r.is_some_and(|v| v["dryRun"] != true) {
                common::toast_success(&me.p.anchor(), &common::fill(t("“{name}” is toegevoegd", "“{name}” has been added"), &[("name", &title)]), 3000);
            }
            me.reload().await;
        });
    }

    /// GNOME's address entry (Windows' "Een printer toevoegen met behulp van een IP-adres of hostnaam").
    fn manual(self: &Rc<Self>) {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_widget_name("printers-manual-dialog");
        body.append(&c::label(
            t(
                "Typ de hostnaam of het IP-adres van de printer of de afdrukserver, of een volledig adres zoals ipp://, socket:// of smb://.",
                "Type the host name or IP address of the printer or the print server, or a full address such as ipp://, socket:// or smb://.",
            ),
            "",
            true,
        ));
        let entry = c::text_box(t("Bijvoorbeeld 192.168.1.20 of printer.local", "For example 192.168.1.20 or printer.local"));
        entry.set_widget_name("printers-address");
        body.append(&c::field(t("Hostnaam of IP-adres", "Host name or IP address"), &entry, None));
        let auth = gtk::Box::new(gtk::Orientation::Vertical, 8);
        auth.set_visible(false);
        auth.append(&c::label(
            t("Deze afdrukserver vraagt om een gebruikersnaam en wachtwoord.", "This print server asks for a user name and password."),
            "",
            true,
        ));
        let user = c::text_box("");
        user.set_widget_name("printers-user");
        let password = c::text_box("");
        password.set_visibility(false);
        password.set_widget_name("printers-password");
        auth.append(&c::field(t("Gebruikersnaam", "User name"), &user, None));
        auth.append(&c::field(t("Wachtwoord", "Password"), &password, None));
        body.append(&auth);
        let results = gtk::Box::new(gtk::Orientation::Vertical, 4);
        results.set_widget_name("printers-manual-results");
        body.append(&results);
        let dialog = c::Dialog::new(t("Een printer toevoegen met behulp van een IP-adres of hostnaam", "Add a printer using an IP address or hostname"))
            .body(&body)
            .width(560)
            .primary(t("Zoeken", "Search"))
            .close_label(t("Sluiten", "Close"));
        let me = self.clone();
        let dialog = {
            let (entry, user, password, auth, results) = (entry.clone(), user.clone(), password.clone(), auth.clone(), results.clone());
            dialog.on_primary(move |dlg| {
                let address = entry.text().trim().to_owned();
                if address.is_empty() {
                    return false;
                }
                let mut args = json!({ "address": address });
                if auth.is_visible() {
                    args["username"] = json!(user.text().to_string());
                    args["password"] = json!(password.text().to_string());
                }
                dlg.set_busy(true);
                c::clear(&results);
                results.append(&c::label(t("Zoeken…", "Searching…"), "st-prn-key", false));
                let (me, dlg, auth, results) = (me.clone(), dlg.clone(), auth.clone(), results.clone());
                let pp = me.p.clone();
                pp.spawn(async move {
                    let r = me.p.call("printers.discover", args).await;
                    dlg.set_busy(false);
                    if !me.p.alive() {
                        return;
                    }
                    c::clear(&results);
                    match r {
                        Ok(v) => {
                            let devices: Vec<Value> = v["devices"].as_array().cloned().unwrap_or_default();
                            let needs_auth = v["needsAuth"] == true;
                            if needs_auth {
                                auth.set_visible(true);
                            }
                            if devices.is_empty() {
                                results.append(&c::label(
                                    if needs_auth {
                                        t("Geef de gebruikersnaam en het wachtwoord op en zoek opnieuw.", "Enter the user name and password and search again.")
                                    } else {
                                        t("Er is op dit adres geen printer gevonden.", "No printer was found at this address.")
                                    },
                                    "",
                                    true,
                                ));
                            }
                            for (i, d) in devices.iter().enumerate() {
                                let card = SettingsCard::new("print", &device_title(d), &[device_kind(d), s(&d["uri"])].join(" · "));
                                let b = c::button(t("Toevoegen", "Add"), Variant::Accent);
                                b.set_widget_name(&format!("printers-manual-add-{i}"));
                                let (me2, d2, dlg2) = (me.clone(), d.clone(), dlg.clone());
                                b.connect_clicked(move |_| {
                                    dlg2.close(c::DialogResult::Close);
                                    me2.add(d2.clone());
                                });
                                card.set_control(&b);
                                results.append(card.widget());
                            }
                        }
                        Err(e) => results.append(&c::label(&e.message, "", true)),
                    }
                });
                false
            })
        };
        dialog.show(&self.p.anchor(), |_| {});
        entry.grab_focus();
    }

    fn paint_found(self: &Rc<Self>) {
        let mut out: Vec<gtk::Widget> = Vec::new();
        if self.searching.get() {
            let card = SettingsCard::new("search", t("Zoeken naar printers en scanners…", "Searching for printers and scanners…"), "");
            card.set_control(c::ProgressRing::new(20, None).widget());
            card.widget().set_widget_name("printers-searching");
            out.push(card.widget().clone());
        }
        if let Some(found) = self.found.borrow().as_ref() {
            let adding = self.adding.borrow().clone();
            let shown: Vec<&Value> = found.iter().filter(|d| !self.installed(s(&d["uri"]))).collect();
            if shown.is_empty() && !self.searching.get() {
                let card = SettingsCard::new("info", t("Er zijn geen nieuwe printers of scanners gevonden.", "No new printers or scanners were found."), "");
                card.widget().set_widget_name("printers-none-found");
                out.push(card.widget().clone());
            }
            for (i, d) in shown.into_iter().enumerate() {
                let uri = s(&d["uri"]).to_owned();
                let desc = [device_kind(d), s(&d["makeModel"])].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
                let card = SettingsCard::new("print", &device_title(d), &desc);
                card.widget().set_widget_name(&format!("printers-found-{i}"));
                if adding.contains(&uri) {
                    card.set_control(c::ProgressRing::new(20, None).widget());
                } else {
                    let b = c::button(t("Apparaat toevoegen", "Add device"), Variant::Standard);
                    b.set_widget_name(&format!("printers-found-add-{i}"));
                    let (me, dev) = (self.clone(), d.clone());
                    b.connect_clicked(move |_| me.add(dev.clone()));
                    card.set_control(&b);
                }
                out.push(card.widget().clone());
            }
            let manual = SettingsCard::new("", t("De printer die ik wil gebruiken, staat niet in de lijst", "The printer that I want isn’t listed"), "");
            let b = c::button(t("Handmatig toevoegen", "Add manually"), Variant::Standard);
            b.set_widget_name("printers-manual");
            let me = self.clone();
            b.connect_clicked(move |_| me.manual());
            manual.set_control(&b);
            out.push(manual.widget().clone());
        }
        common::set_slot(&self.add_slot, &out);
    }

    fn paint(self: &Rc<Self>) {
        let st = self.state.borrow().clone();
        let mut out: Vec<gtk::Widget> = Vec::new();
        if st["available"] != true {
            let empty = c::empty_state(
                "print",
                t("Afdrukservice niet beschikbaar", "Print service unavailable"),
                if s(&st["reason"]) == "libcups" {
                    t("Afdrukken is op deze computer niet geïnstalleerd (CUPS ontbreekt).", "Printing isn’t installed on this computer (CUPS is missing).")
                } else {
                    t(
                        "De afdrukservice (CUPS) draait niet. Start de computer opnieuw op of vraag je beheerder om CUPS te starten.",
                        "The print service (CUPS) isn’t running. Restart the computer or ask your administrator to start CUPS.",
                    )
                },
                None,
            );
            empty.set_widget_name("printers-unavailable");
            out.push(empty.upcast());
        } else {
            let all = st["printers"].as_array().cloned().unwrap_or_default();
            self.search.widget().set_visible(all.len() >= SEARCH_FROM);
            let query = self.query.borrow().clone();
            let printers: Vec<Value> = all.into_iter().filter(|p| matches_query(p, &query)).collect();
            let group = SettingsGroup::new("", "");
            group.widget().set_widget_name("printers-list");
            if printers.is_empty() && !query.is_empty() {
                let card = SettingsCard::new("search", t("Geen printers gevonden die overeenkomen.", "No matching printers were found."), "");
                card.widget().set_widget_name("printers-no-match");
                group.add(card.widget());
            } else if printers.is_empty() {
                let card = SettingsCard::new("print", t("Er zijn nog geen printers toegevoegd.", "No printers have been added yet."), "");
                card.widget().set_widget_name("printers-empty");
                group.add(card.widget());
            }
            for pr in &printers {
                let mut lines = Vec::new();
                if pr["isDefault"] == true {
                    lines.push(t("Standaard", "Default").to_owned());
                }
                lines.push(status_text(pr));
                let card = SettingsCard::clickable("print", s(&pr["title"]), &lines.join(", "), Chevron::Right);
                card.widget().set_widget_name(&format!("printer-{}", s(&pr["name"])));
                let (pp, name) = (self.p.clone(), s(&pr["name"]).to_owned());
                card.connect_clicked(move || pp.go_with("printer", json!({ "name": name })));
                group.add(card.widget());
            }
            out.push(group.widget().clone().upcast());
        }
        common::set_slot(&self.body, &out);
    }
}

/// From how many printers on GNOME's search shows (Windows' page has none; a long list needs one).
const SEARCH_FROM: usize = 5;

/// GNOME's printer search: the name, model or location holds every word typed.
pub(super) fn matches_query(p: &Value, query: &str) -> bool {
    let hay = [s(&p["title"]), s(&p["name"]), s(&p["makeModel"]), s(&p["location"])].join(" ").to_lowercase();
    query.to_lowercase().split_whitespace().all(|w| hay.contains(w))
}

fn list_page(p: Page) -> PageFuture {
    Box::pin(async move {
        let st = p.get("printers.list", json!({})).await;
        if !p.alive() {
            return Vec::new();
        }
        let st = st.filter(|v| v.is_object()).unwrap_or_else(|| json!({ "available": false }));
        let available = st["available"] == true;
        let body = common::slot();
        // the found devices sit in the card's group: its 4 px between cards, not a slot's 22
        let add_slot = gtk::Box::new(gtk::Orientation::Vertical, 4);
        add_slot.set_visible(false);
        let search = c::SearchBox::new(Some(t("Printers zoeken", "Search printers")));
        search.widget().set_widget_name("printers-search");
        search.widget().set_visible(false);
        let page = Rc::new(ListPage {
            p: p.clone(),
            state: RefCell::new(st),
            found: RefCell::new(None),
            searching: Cell::new(false),
            adding: RefCell::new(Vec::new()),
            search: search.clone(),
            query: RefCell::new(String::new()),
            add_slot: add_slot.clone(),
            body: body.clone(),
        });
        {
            let pg = Rc::downgrade(&page);
            search.connect_changed(move |text| {
                if let Some(pg) = pg.upgrade() {
                    *pg.query.borrow_mut() = text.to_owned();
                    pg.paint();
                }
            });
        }
        let add = SettingsCard::new("plus", t("Een printer of scanner toevoegen", "Add a printer or scanner"), "");
        let b = c::button(t("Apparaat toevoegen", "Add device"), Variant::Standard);
        b.set_widget_name("printers-add");
        b.set_sensitive(available);
        {
            let pg = page.clone();
            b.connect_clicked(move |_| pg.search());
        }
        add.set_control(&b);
        let top = SettingsGroup::new("", "");
        top.add(add.widget());
        top.add(&add_slot);
        {
            let pg = page.clone();
            p.on("printers.changed", move |_| {
                let me = pg.clone();
                pg.p.spawn(async move { me.reload().await });
            });
        }
        page.paint();
        let mut out: Vec<gtk::Widget> = vec![top.widget().clone().upcast(), search.widget().clone().upcast(), body.upcast()];
        if common::caps()["simpleScan"] == true {
            let scanners = SettingsGroup::new(t("Scanners", "Scanners"), "");
            let card = SettingsCard::new("print", t("Scanner openen", "Open scanner"), t("Scannen met Documentscanner", "Scan with Document Scanner"));
            let open = c::button(t("Openen", "Open"), Variant::Standard);
            open.set_widget_name("printers-scan");
            let pp = p.clone();
            open.connect_clicked(move |_| {
                let wait = pp.run("apps.launch", json!({ "id": "simple-scan.desktop" }));
                pp.spawn(async move {
                    let _ = wait.await;
                });
            });
            card.set_control(&open);
            scanners.add(card.widget());
            out.push(scanners.widget().clone().upcast());
        }
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(
            common::link_card_with(&p, "colorprofile", None, Some(t("Kleurprofielen van printers en beeldschermen", "Colour profiles of printers and displays")), None)
                .widget(),
        );
        related.add(common::link_card(&p, "bluetooth-devices").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

// -------------------------------------------------------------------------------------------- printer page

struct PrinterPage {
    p: Page,
    name: String,
    data: RefCell<Value>,
    body: gtk::Box,
}

type Act = Rc<dyn Fn(&'static str, Value)>;

impl PrinterPage {
    fn printer(&self) -> Value {
        self.data.borrow()["printer"].clone()
    }

    /// The printer again (printers.changed, after a write). A queue that is gone (removed or renamed elsewhere:
    /// the host's not_found) shows "Deze printer is niet gevonden", as GNOME's panel drops it; another error keeps
    /// the page as it is (the scheduler may be restarting).
    async fn reload(self: &Rc<Self>) {
        let got = self.p.call("printers.get", json!({ "name": self.name })).await;
        if !self.p.alive() {
            return;
        }
        match got {
            Ok(got) if got.is_object() => {
                *self.data.borrow_mut() = got;
                self.paint();
            }
            r if printer_gone(&r) => common::set_slot(&self.body, &[printer_missing()]),
            _ => {}
        }
    }

    /// Run a write and reload; the toast `done` on success.
    fn act(self: &Rc<Self>, method: &'static str, args: Value, error: &'static str, done: Option<&'static str>) {
        let me = self.clone();
        self.p.spawn(async move {
            let r = me.p.run_with(method, args, Some(error), false).await;
            if !me.p.alive() {
                return;
            }
            if let (Some(v), Some(done)) = (r.as_ref(), done)
                && v["dryRun"] != true
            {
                common::toast_success(&me.p.anchor(), done, 2500);
            }
            me.reload().await;
        });
    }

    /// GNOME's "restart printer after an error": enabled and accepting jobs again.
    fn resume(self: &Rc<Self>) {
        let me = self.clone();
        self.p.spawn(async move {
            let name = me.name.clone();
            let err = t("De printer kon niet worden hervat", "Couldn’t resume the printer");
            if me.p.run_with("printers.setEnabled", json!({"name": name, "on": true}), Some(err), false).await.is_some() {
                let _ = me.p.run_with("printers.setAcceptJobs", json!({"name": name, "on": true}), Some(err), true).await;
            }
            if me.p.alive() {
                me.reload().await;
            }
        });
    }

    fn remove(self: &Rc<Self>) {
        let title = s(&self.printer()["title"]).to_owned();
        let me = self.clone();
        c::Dialog::new(t("Dit apparaat verwijderen?", "Remove this device?"))
            .text(&common::fill(
                t("“{name}” wordt verwijderd. Afdruktaken in de wachtrij worden geannuleerd.", "“{name}” will be removed. Print jobs in the queue will be cancelled."),
                &[("name", &title)],
            ))
            .primary(t("Ja", "Yes"))
            .close_label(t("Nee", "No"))
            .show(&self.p.anchor(), move |r| {
                if r != c::DialogResult::Primary {
                    return;
                }
                let me2 = me.clone();
                me.p.spawn(async move {
                    let r = me2
                        .p
                        .run_with("printers.remove", json!({"name": me2.name}), Some(t("De printer kon niet worden verwijderd", "Couldn’t remove the printer")), false)
                        .await;
                    if r.is_some_and(|v| v["dryRun"] != true) && me2.p.alive() {
                        me2.p.go("printers");
                    }
                });
            });
    }

    /// One job of the queue: its state, owner, time and size; pause or resume, cancel, ⋯ Bovenaan zetten, and
    /// Verifiëren for a job that waits for it.
    fn job_card(self: &Rc<Self>, j: &Value, act: &Act, load: &Rc<dyn Fn()>) -> gtk::Widget {
        let id = j["id"].as_i64().unwrap_or(0);
        let when = j["created"].as_f64().filter(|t| *t > 0.0).map(|secs| kit::format::file_time(secs * 1000.0)).unwrap_or_default();
        let size = j["size"].as_f64().filter(|k| *k > 0.0).map(|k| kit::format::bytes(k * 1024.0)).unwrap_or_default();
        let desc = [job_state_text(j).to_owned(), s(&j["user"]).to_owned(), when, size]
            .into_iter()
            .filter(|x| !x.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let card = SettingsCard::new("document", s(&j["name"]), &desc);
        card.widget().set_widget_name(&format!("printer-job-{id}"));
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if j["authRequired"] == true {
            let b = c::button(t("Verifiëren", "Authenticate"), Variant::Accent);
            b.set_widget_name(&format!("printer-job-{id}-auth"));
            let (me, load) = (self.clone(), load.clone());
            b.connect_clicked(move |_| me.authenticate(id, load.clone()));
            controls.append(&b);
        }
        let held = j["held"] == true;
        let pause = c::icon_button(if held { "play" } else { "pause" }, if held { t("Hervatten", "Resume") } else { t("Onderbreken", "Pause") }, 32);
        pause.set_widget_name(&format!("printer-job-{id}-{}", if held { "release" } else { "hold" }));
        {
            let act = act.clone();
            pause.connect_clicked(move |_| act(if held { "printers.job.release" } else { "printers.job.hold" }, json!({ "id": id })));
        }
        controls.append(&pause);
        let cancel = c::icon_button("close", t("Annuleren", "Cancel"), 32);
        cancel.set_widget_name(&format!("printer-job-{id}-cancel"));
        {
            let act = act.clone();
            cancel.connect_clicked(move |_| act("printers.job.cancel", json!({ "id": id })));
        }
        controls.append(&cancel);
        let more = c::icon_button("more", t("Meer opties", "More options"), 32);
        more.set_widget_name(&format!("printer-job-{id}-more"));
        {
            let act = act.clone();
            more.connect_clicked(move |b| {
                let act = act.clone();
                c::show_menu_for(
                    b,
                    vec![MenuItem::new(t("Bovenaan zetten", "Move to top"), move || act("printers.job.priority", json!({"id": id, "priority": 100})))
                        .glyph("arrow-up")
                        .name("printer-job-top")],
                );
            });
        }
        controls.append(&more);
        card.set_control(&controls);
        card.widget().clone()
    }

    /// GNOME's Jobs dialog as Windows' print queue; it follows the scheduler while it is open.
    fn queue(self: &Rc<Self>) {
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.set_widget_name("printer-queue-list");
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_min_content_height(240);
        scroller.set_max_content_height(420);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&list));
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.append(&scroller);
        let clear = c::button(t("Alle documenten annuleren", "Cancel all documents"), Variant::Standard);
        clear.set_widget_name("printer-queue-clear");
        clear.set_halign(gtk::Align::Start);
        body.append(&clear);
        let title = s(&self.printer()["title"]).to_owned();
        let dialog = c::Dialog::new(&common::fill(t("Afdrukwachtrij van {name}", "Print queue of {name}"), &[("name", &title)]))
            .body(&body)
            .width(640)
            .close_label(t("Sluiten", "Close"));
        let jobs: Rc<RefCell<Vec<Value>>> = Rc::default();
        let painted = Rc::new(Cell::new(false));
        let paint: Rc<RefCell<Option<Rc<dyn Fn()>>>> = Rc::default();
        let load: Rc<dyn Fn()> = {
            let (me, jobs, paint, painted) = (self.clone(), jobs.clone(), paint.clone(), painted.clone());
            Rc::new(move || {
                let (me2, jobs, paint, painted) = (me.clone(), jobs.clone(), paint.clone(), painted.clone());
                me.p.spawn(async move {
                    if let Some(v) = me2.p.get("printers.jobs", json!({ "name": me2.name })).await {
                        let new = v["jobs"].as_array().cloned().unwrap_or_default();
                        // a list that did not change keeps its widgets (a click in progress, the hover)
                        if painted.get() && *jobs.borrow() == new {
                            return;
                        }
                        painted.set(true);
                        *jobs.borrow_mut() = new;
                        let f = paint.borrow().clone();
                        if let Some(f) = f {
                            f();
                        }
                    }
                });
            })
        };
        let act: Act = {
            let (me, load) = (self.clone(), load.clone());
            Rc::new(move |method, args| {
                let (me2, load) = (me.clone(), load.clone());
                me.p.spawn(async move {
                    let _ = me2.p.run_with(method, args, Some(t("Dat lukte niet met deze afdruktaak", "That didn’t work for this print job")), false).await;
                    load();
                });
            })
        };
        *paint.borrow_mut() = Some({
            let (list, jobs, act, me, load) = (list.clone(), jobs.clone(), act.clone(), self.clone(), load.clone());
            Rc::new(move || {
                c::clear(&list);
                let all = jobs.borrow().clone();
                if all.is_empty() {
                    let l = c::label(t("Er staan geen documenten in de wachtrij.", "There are no documents in the queue."), "st-prn-key", true);
                    l.set_widget_name("printer-queue-empty");
                    list.append(&l);
                }
                for j in &all {
                    list.append(&me.job_card(j, &act, &load));
                }
            })
        });
        {
            let (me, load) = (self.clone(), load.clone());
            clear.connect_clicked(move |_| {
                let (me2, load) = (me.clone(), load.clone());
                c::Dialog::new(t("Alle documenten annuleren?", "Cancel all documents?"))
                    .text(t("Alle documenten in de wachtrij van deze printer worden geannuleerd.", "All documents in this printer’s queue will be cancelled."))
                    .primary(t("Ja", "Yes"))
                    .close_label(t("Nee", "No"))
                    .show(&me.p.anchor(), move |r| {
                        if r == c::DialogResult::Primary {
                            let (me3, load) = (me2.clone(), load.clone());
                            me2.p.spawn(async move {
                                let _ = me3.p.run_with("printers.jobs.clear", json!({ "name": me3.name }), Some(t("Annuleren lukte niet", "Couldn’t cancel")), false).await;
                                load();
                            });
                        }
                    });
            });
        }
        load();
        dialog.show(&self.p.anchor(), |_| {});
        let (dlg, load2) = (dialog.clone(), load.clone());
        self.p.interval(2000, move || {
            if !dlg.is_open() {
                return false;
            }
            load2();
            true
        });
    }

    /// GNOME's authentication of a job held for it: user, (domain,) password.
    fn authenticate(self: &Rc<Self>, id: i64, after: Rc<dyn Fn()>) {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let needs: Vec<String> = self.printer()["authInfoRequired"].as_array().into_iter().flatten().map(|v| s(v).to_owned()).collect();
        let user = c::text_box("");
        user.set_widget_name("printer-auth-user");
        let domain = c::text_box("");
        let password = c::text_box("");
        password.set_visibility(false);
        password.set_widget_name("printer-auth-password");
        if needs.is_empty() || needs.iter().any(|v| v == "username") {
            body.append(&c::field(t("Gebruikersnaam", "User name"), &user, None));
        }
        if needs.iter().any(|v| v == "domain") {
            body.append(&c::field(t("Domein", "Domain"), &domain, None));
        }
        body.append(&c::field(t("Wachtwoord", "Password"), &password, None));
        let me = self.clone();
        c::Dialog::new(t("Verificatie vereist", "Authentication required"))
            .body(&body)
            .primary(t("Verifiëren", "Authenticate"))
            .close_label(t("Annuleren", "Cancel"))
            .show(&self.p.anchor(), move |r| {
                if r != c::DialogResult::Primary {
                    return;
                }
                let args = json!({"id": id, "username": user.text().to_string(), "domain": domain.text().to_string(),
                    "password": password.text().to_string()});
                let me2 = me.clone();
                me.p.spawn(async move {
                    let _ = me2.p.run_with("printers.job.authenticate", args, Some(t("Verifiëren lukte niet", "Couldn’t authenticate")), false).await;
                    after();
                });
            });
    }

    /// A new driver for the queue (PrinterAdd / PrinterAddWithPpdFile on the same name and address).
    fn change_driver(self: &Rc<Self>) {
        let me = self.clone();
        self.p.spawn(async move {
            let pr = me.printer();
            let uri = s(&pr["deviceUri"]).to_owned();
            let device = json!({"uri": uri, "info": s(&pr["title"]), "makeModel": s(&pr["makeModel"]),
                "deviceId": s(&pr["deviceId"]), "driverless": false});
            let Some(driver) = choose_driver(&me.p, &device, t("Opslaan", "Save")).await else { return };
            let mut args = json!({"name": me.name, "uri": uri});
            for (k, v) in driver.args().as_object().cloned().unwrap_or_default() {
                args[k] = v;
            }
            let _ = me.p.run_with("printers.add", args, Some(t("Het stuurprogramma kon niet worden gewijzigd", "Couldn’t change the driver")), false).await;
            if me.p.alive() {
                me.reload().await;
            }
        });
    }

    /// GNOME's Printer Details as Windows' "Printereigenschappen": Naam, Locatie, Adres, Stuurprogramma.
    fn properties(self: &Rc<Self>) {
        let pr = self.printer();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_widget_name("printer-properties-dialog");
        let name = c::text_box("");
        name.set_text(&self.name);
        name.set_widget_name("printer-prop-name");
        body.append(&c::field(
            t("Naam", "Name"),
            &name,
            Some(t("Zonder spaties, slashes, hekjes of aanhalingstekens", "Without spaces, slashes, hash signs or quotation marks")),
        ));
        let location = c::text_box("");
        location.set_text(s(&pr["location"]));
        location.set_widget_name("printer-prop-location");
        body.append(&c::field(t("Locatie", "Location"), &location, None));
        body.append(&kv_line(t("Adres", "Address"), s(&pr["deviceUri"])));
        let driver = s(&pr["makeModel"]);
        body.append(&kv_line(t("Stuurprogramma", "Driver"), if driver.is_empty() { t("Onbekend", "Unknown") } else { driver }));
        let change = c::button(t("Ander stuurprogramma kiezen", "Choose another driver"), Variant::Standard);
        change.set_widget_name("printer-prop-driver");
        change.set_halign(gtk::Align::Start);
        body.append(&change);
        let dialog = c::Dialog::new(t("Printereigenschappen", "Printer properties"))
            .body(&body)
            .width(540)
            .primary(t("Opslaan", "Save"))
            .close_label(t("Annuleren", "Cancel"));
        {
            let (me, dlg) = (self.clone(), dialog.clone());
            change.connect_clicked(move |_| {
                dlg.close(c::DialogResult::Close);
                me.change_driver();
            });
        }
        let me = self.clone();
        let old_location = s(&pr["location"]).to_owned();
        dialog.show(&self.p.anchor(), move |r| {
            if r != c::DialogResult::Primary {
                return;
            }
            let (new_name, new_location) = (name.text().trim().to_owned(), location.text().trim().to_owned());
            let me2 = me.clone();
            me.p.spawn(async move {
                let current = me2.name.clone();
                if new_location != old_location
                    && me2
                        .p
                        .run_with("printers.setLocation", json!({"name": current, "location": new_location}), Some(t("De locatie kon niet worden gewijzigd", "Couldn’t change the location")), false)
                        .await
                        .is_none()
                {
                    return;
                }
                if !new_name.is_empty() && new_name != current {
                    let r = me2
                        .p
                        .run_with("printers.rename", json!({"name": current, "newName": new_name}), Some(t("De naam kon niet worden gewijzigd", "Couldn’t change the name")), false)
                        .await;
                    if r.is_some_and(|v| v["dryRun"] != true) {
                        me2.p.go_with("printer", json!({ "name": new_name }));
                        return;
                    }
                }
                if me2.p.alive() {
                    me2.reload().await;
                }
            });
        });
    }

    /// One option of the preferences: a dropdown whose change is written at once (reverted when it fails).
    fn option_row(self: &Rc<Self>, name: &str, text: &str, options: Vec<DropdownOption>, value: &str) -> gtk::Box {
        let dd = Dropdown::new(options, Some(value));
        dd.widget().set_widget_name(&format!("printer-opt-{name}"));
        let (me, option, last, dd2) = (self.clone(), name.to_owned(), Rc::new(RefCell::new(value.to_owned())), dd.clone());
        dd.connect_changed(move |v| {
            if *last.borrow() == v {
                return;
            }
            let (me2, option, v, last, dd3) = (me.clone(), option.clone(), v.to_owned(), last.clone(), dd2.clone());
            me.p.spawn(async move {
                let r = me2
                    .p
                    .run_with("printers.setOption", json!({"name": me2.name, "option": option, "values": [v]}), Some(t("Deze instelling kon niet worden gewijzigd", "Couldn’t change this setting")), false)
                    .await;
                if r.is_some() {
                    *last.borrow_mut() = v;
                } else {
                    let old = last.borrow().clone();
                    dd3.set_selected(Some(&old));
                }
            });
        });
        c::field(text, dd.widget(), None)
    }

    /// GNOME's Printing Options as Windows' "Voorkeursinstellingen voor afdrukken".
    fn preferences(self: &Rc<Self>) {
        let data = self.data.borrow().clone();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
        body.set_widget_name("printer-preferences-dialog");
        // room for the scrollbar beside the drop-downs
        body.set_margin_end(14);
        let ipp = data["ipp"].as_array().cloned().unwrap_or_default();
        if !ipp.is_empty() {
            body.append(&common::section_title(t("Indeling", "Layout")));
            for o in &ipp {
                let name = s(&o["name"]);
                let options: Vec<DropdownOption> = o["values"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| {
                        let val = match v {
                            Value::String(x) => x.clone(),
                            other => other.to_string(),
                        };
                        DropdownOption::new(&val, &ipp_choice_text(name, &val))
                    })
                    .collect();
                body.append(&self.option_row(name, ipp_option_text(name), options, s(&o["value"])));
            }
        }
        let groups = data["options"].as_array().cloned().unwrap_or_default();
        for g in &groups {
            body.append(&common::section_title(s(&g["text"])));
            for o in g["options"].as_array().into_iter().flatten() {
                let options: Vec<DropdownOption> =
                    o["choices"].as_array().into_iter().flatten().map(|ch| DropdownOption::new(s(&ch["value"]), s(&ch["text"]))).collect();
                body.append(&self.option_row(s(&o["name"]), s(&o["text"]), options, s(&o["value"])));
            }
        }
        if ipp.is_empty() && groups.is_empty() {
            body.append(&c::label(t("Deze printer heeft geen instellingen die je hier kunt wijzigen.", "This printer has no settings you can change here."), "", true));
        }
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_hscrollbar_policy(gtk::PolicyType::Never);
        scroller.set_max_content_height(480);
        scroller.set_propagate_natural_height(true);
        scroller.set_child(Some(&body));
        c::Dialog::new(t("Voorkeursinstellingen voor afdrukken", "Printing preferences"))
            .body(&scroller)
            .width(540)
            .close_label(t("Sluiten", "Close"))
            .show(&self.p.anchor(), |_| {});
    }

    fn paint(self: &Rc<Self>) {
        let pr = self.printer();
        let mut out: Vec<gtk::Widget> = Vec::new();
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        if pr["isDefault"] != true {
            let b = c::button(t("Als standaard instellen", "Set as default"), Variant::Standard);
            b.set_widget_name("printer-default");
            let me = self.clone();
            b.connect_clicked(move |_| {
                me.act("printers.setDefault", json!({"name": me.name}), t("De standaardprinter kon niet worden ingesteld", "Couldn’t set the default printer"), None)
            });
            actions.append(&b);
        }
        let remove = c::button(t("Verwijderen", "Remove"), Variant::Standard);
        remove.set_widget_name("printer-remove");
        {
            let me = self.clone();
            remove.connect_clicked(move |_| me.remove());
        }
        actions.append(&remove);
        if pr["canClean"] == true {
            let more = c::icon_button("more", t("Meer opties", "More options"), 32);
            more.set_widget_name("printer-more");
            let me = self.clone();
            more.connect_clicked(move |b| {
                let me2 = me.clone();
                c::show_menu_for(
                    b,
                    vec![MenuItem::new(t("Printkoppen reinigen", "Clean print heads"), move || {
                        me2.act(
                            "printers.cleanHeads",
                            json!({"name": me2.name}),
                            t("Printkoppen reinigen lukte niet", "Couldn’t clean the print heads"),
                            Some(t("De printkoppen worden gereinigd", "The print heads are being cleaned")),
                        )
                    })
                    .glyph("clean")
                    .name("printer-clean")],
                );
            });
            actions.append(&more);
        }
        let mut lines = vec![common::fill(t("Status: {status}", "Status: {status}"), &[("status", &status_text(&pr))])];
        if pr["isDefault"] == true {
            lines.push(t("Dit is je standaardprinter", "This is your default printer").to_owned());
        }
        let art = kit::icons::glyph("print", 64);
        let hero = common::hero(Some(art.upcast_ref()), s(&pr["title"]), &lines, Some(actions.upcast_ref()), None);
        hero.set_widget_name("printer-hero");
        out.push(hero.upcast());
        if needs_resume(&pr) {
            let message = pr["reasons"].as_array().into_iter().flatten().filter_map(|r| reason_text(s(&r["reason"]))).collect::<Vec<_>>().join(", ");
            let n = common::notice_full(
                Severity::Warning,
                if pr["accepting"] == false {
                    t("De printer weigert afdruktaken", "The printer is rejecting print jobs")
                } else {
                    t("De printer is onderbroken", "The printer is paused")
                },
                if message.is_empty() { t("Hervat de printer om weer af te drukken.", "Resume the printer to print again.") } else { &message },
            );
            let me = self.clone();
            n.set_action(Some((t("Hervatten", "Resume"), Rc::new(move || me.resume()))));
            n.widget().set_widget_name("printer-paused");
            out.push(n.widget().clone().upcast());
        }
        let group = SettingsGroup::new(t("Printerinstellingen", "Printer settings"), "");
        let jobs = pr["jobs"].as_i64().unwrap_or(0);
        let queue = SettingsCard::clickable(
            "list",
            t("Afdrukwachtrij openen", "Open print queue"),
            &common::plural(jobs, t("1 document in de wachtrij", "1 document in the queue"), t("{n} documenten in de wachtrij", "{n} documents in the queue")),
            Chevron::External,
        );
        queue.widget().set_widget_name("printer-queue");
        {
            let me = self.clone();
            queue.connect_clicked(move || me.queue());
        }
        group.add(queue.widget());
        let test = SettingsCard::clickable("document", t("Testpagina afdrukken", "Print test page"), "", Chevron::External);
        test.widget().set_widget_name("printer-testpage");
        {
            let me = self.clone();
            test.connect_clicked(move || {
                me.act(
                    "printers.testPage",
                    json!({"name": me.name}),
                    t("De testpagina kon niet worden afgedrukt", "Couldn’t print the test page"),
                    Some(t("De testpagina is naar de printer gestuurd", "The test page has been sent to the printer")),
                )
            });
        }
        group.add(test.widget());
        let props = SettingsCard::clickable(
            "properties",
            t("Printereigenschappen", "Printer properties"),
            t("Naam, locatie, adres en stuurprogramma", "Name, location, address and driver"),
            Chevron::External,
        );
        props.widget().set_widget_name("printer-properties");
        {
            let me = self.clone();
            props.connect_clicked(move || me.properties());
        }
        group.add(props.widget());
        let prefs = SettingsCard::clickable(
            "settings",
            t("Voorkeursinstellingen voor afdrukken", "Printing preferences"),
            t("Papierformaat, dubbelzijdig, kleur en kwaliteit", "Paper size, two-sided, colour and quality"),
            Chevron::External,
        );
        prefs.widget().set_widget_name("printer-preferences");
        {
            let me = self.clone();
            prefs.connect_clicked(move || me.preferences());
        }
        group.add(prefs.widget());
        out.push(group.widget().clone().upcast());
        let markers = pr["markers"].as_array().cloned().unwrap_or_default();
        if !markers.is_empty() {
            let ink = SettingsGroup::new(t("Inkt en toner", "Ink and toner"), "");
            ink.widget().set_widget_name("printer-ink");
            for m in &markers {
                let level = m["level"].as_i64();
                let card = SettingsCard::new("", s(&m["name"]), &level.map(|l| format!("{l}%")).unwrap_or_else(|| t("Niveau onbekend", "Level unknown").to_owned()));
                let colors: Vec<String> = m["colors"].as_array().into_iter().flatten().map(|c| s(c).to_owned()).collect();
                card.set_control(&ink_bar(level, &colors));
                ink.add(card.widget());
            }
            out.push(ink.widget().clone().upcast());
        }
        let more = SettingsExpander::new("info", t("Meer informatie", "More information"), "");
        more.widget().set_widget_name("printer-info");
        let info = gtk::Box::new(gtk::Orientation::Vertical, 8);
        info.set_margin_top(8);
        info.set_margin_bottom(8);
        info.set_margin_start(16);
        info.set_margin_end(16);
        for (k, v) in [
            (t("Model", "Model"), s(&pr["makeModel"]).to_owned()),
            (t("Locatie", "Location"), s(&pr["location"]).to_owned()),
            (t("Adres", "Address"), s(&pr["deviceUri"]).to_owned()),
            (t("Naam van de wachtrij", "Queue name"), self.name.clone()),
            (t("Gedeeld", "Shared"), if pr["shared"] == true { t("Ja", "Yes") } else { t("Nee", "No") }.to_owned()),
        ] {
            if !v.is_empty() {
                info.append(&kv_line(k, &v));
            }
        }
        more.add(&info);
        let group = SettingsGroup::new("", "");
        group.add(more.widget());
        out.push(group.widget().clone().upcast());
        common::set_slot(&self.body, &out);
    }
}

/// Whether `printers.get` says the queue is not there (any more): the host's `not_found` (printers.notFound).
fn printer_gone(r: &Result<Value, kit::CallError>) -> bool {
    matches!(r, Err(e) if e.code == "not_found")
}

/// "Deze printer is niet gevonden": the page of a queue that is not there (any more).
fn printer_missing() -> gtk::Widget {
    let empty = c::empty_state(
        "print",
        t("Deze printer is niet gevonden", "This printer wasn’t found"),
        t("De printer is misschien verwijderd. Ga terug naar Printers en scanners.", "The printer may have been removed. Go back to Printers & scanners."),
        None,
    );
    empty.set_widget_name("printer-missing");
    empty.upcast()
}

fn printer_page(p: Page) -> PageFuture {
    Box::pin(async move {
        let name = p.param("name").unwrap_or_default();
        let got = p.call("printers.get", json!({ "name": name })).await;
        if !p.alive() {
            return Vec::new();
        }
        let got = match got {
            Ok(v) if v.is_object() => v,
            _ => return vec![printer_missing()],
        };
        let title = s(&got["printer"]["title"]).to_owned();
        p.set_title(if title.is_empty() { &name } else { &title });
        let body = common::slot();
        let page = Rc::new(PrinterPage { p: p.clone(), name, data: RefCell::new(got), body: body.clone() });
        {
            let pg = page.clone();
            p.on("printers.changed", move |_| {
                let me = pg.clone();
                pg.p.spawn(async move { me.reload().await });
            });
        }
        page.paint();
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "colorprofile").widget());
        vec![body.upcast(), related.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_queue_is_named_as_gnome_names_it() {
        assert_eq!(queue_name("HP LaserJet M404", &[]), "HP-LaserJet-M404");
        assert_eq!(queue_name("  Epson ET-2850 / Kantoor # 2 ", &[]), "Epson-ET-2850-Kantoor-2");
        assert_eq!(queue_name("HP LaserJet M404", &["hp-laserjet-m404".into()]), "HP-LaserJet-M404-2");
        assert_eq!(queue_name("HP LaserJet M404", &["HP-LaserJet-M404".into(), "HP-LaserJet-M404-2".into()]), "HP-LaserJet-M404-3");
        assert_eq!(queue_name("Drukker 'Zolder'", &[]), "Drukker-Zolder");
        assert_eq!(queue_name("Ünïcode", &[]), "n-code", "non-ASCII becomes '-'");
        assert_eq!(queue_name("///", &[]), "Printer");
        assert!(queue_name(&"x".repeat(300), &[]).len() <= 120);
    }

    #[test]
    fn the_status_names_the_worst_reason_first() {
        let p = json!({"state": "stopped", "accepting": true, "reasons": [
            {"reason": "marker-supply-low", "severity": "warning"}, {"reason": "media-empty", "severity": "error"}]});
        assert_eq!(status_text(&p), t("Papier op", "Out of paper"));
        assert!(needs_resume(&p));
        let idle = json!({"state": "idle", "accepting": true, "reasons": []});
        assert_eq!(status_text(&idle), t("Inactief", "Idle"));
        assert!(!needs_resume(&idle));
        assert!(needs_resume(&json!({"state": "idle", "accepting": false})), "GNOME's restart also accepts jobs again");
        assert_eq!(status_text(&json!({"state": "stopped", "reasons": [{"reason": "paused", "severity": "report"}]})), t("Onderbroken", "Paused"));
        assert_eq!(reason_text("cups-something-new"), None);
    }

    #[test]
    fn options_and_drivers_read_as_gnomes_dialogs() {
        assert_eq!(ipp_choice_text("orientation-requested", "4"), t("Liggend", "Landscape"));
        assert_eq!(ipp_choice_text("sides", "two-sided-long-edge"), t("Dubbelzijdig, lange zijde", "Two-sided, long edge"));
        assert_eq!(ipp_choice_text("number-up", "4"), "4");
        assert_eq!(job_state_text(&json!({"state": "held"})), t("Onderbroken", "Paused"));
        assert_eq!(job_state_text(&json!({"state": "held", "authRequired": true})), t("Verificatie vereist", "Authentication required"));
        assert_eq!(Driver::Ppd("everywhere".into()).args(), json!({"ppd": "everywhere"}));
        assert_eq!(Driver::File("/x.ppd".into()).args(), json!({"ppdFile": "/x.ppd"}));
        let makes: Vec<String> = ["EPSON", "Generic", "HP"].iter().map(|m| m.to_string()).collect();
        assert_eq!(guess_make(&makes, "HP LaserJet Pro M404", ""), "HP");
        assert_eq!(guess_make(&makes, "", "MFG:EPSON;MDL:ET-2850;"), "EPSON");
        assert_eq!(guess_make(&makes, "Brother HL", ""), "Generic");
        assert_eq!(rgb("#00ff80"), Some((0.0, 1.0, 128.0 / 255.0)));
        assert_eq!(rgb("none"), None);
    }

    #[test]
    fn a_queue_that_went_elsewhere_is_not_found() {
        // review B finding 9: printers.changed after a removal or rename elsewhere answers not_found
        let gone: Result<Value, kit::CallError> = Err(kit::CallError::new("not_found", "Deze printer is niet gevonden."));
        assert!(printer_gone(&gone));
        let restarting: Result<Value, kit::CallError> = Err(kit::CallError::new("offline", "De achtergronddienst is niet bereikbaar"));
        assert!(!printer_gone(&restarting), "a transient error keeps the page");
        assert!(!printer_gone(&Ok(json!({"printer": {"name": "HP"}}))));
    }

    #[test]
    fn the_search_finds_every_word_in_name_model_or_location() {
        let p = json!({"title": "HP LaserJet", "name": "HP_LaserJet", "makeModel": "HP LaserJet Pro M404", "location": "Kantoor"});
        assert!(matches_query(&p, ""));
        assert!(matches_query(&p, "laserjet kantoor"));
        assert!(matches_query(&p, "M404"));
        assert!(!matches_query(&p, "epson"));
    }
}
