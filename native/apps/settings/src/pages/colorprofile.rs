//! Instellingen › Systeem › Beeldscherm › Kleurprofiel (batch B, notes/claude/settings-parity.md §2): Windows 11's
//! Colour profile page (24H2) holding GNOME Settings' Color panel (cc-color-panel.c, cc-color-calibrate.c) through
//! `devices.color.*` (colord, colord-session).
//!
//! * Every colour-managed device (displays first, then printers, scanners, cameras) is an expander with GNOME's
//!   title ("Ingebouwd beeldscherm", "{model}") and its default profile or "Niet gekalibreerd". Inside:
//!   "Kleurbeheer gebruiken voor dit apparaat" (Device.SetEnabled), Windows' "Kleurprofiel" dropdown (GNOME's default
//!   radio: MakeProfileDefault), each profile with its date, "Voor alle gebruikers" and colord's warnings and a "⋯"
//!   menu: "Instellen voor alle gebruikers" (the shield: InstallSystemWide, auth_admin_keep) and "Verwijderen";
//!   "Profiel toevoegen" (GNOME's dialog: the compatible profiles colord knows and "Bestand importeren…");
//!   "Kalibreren…" for a display while a colour sensor is connected (GNOME's assistant in one dialog: kwaliteit,
//!   kalibratieapparaat, beeldschermtype, witpunt, helderheid, profielnaam), then GNOME's full-screen calibration
//!   window with the sample colour, the questions of colord-session and the progress (`devices.calibrate`).
//! * No buttons on the expander headers (a button there never gets its click: notes/claude/settings-b.md note 4).
//! * Live: devices.changed {what: color}.
use super::{Builder, Page, PageFuture};
use crate::common;
use kit::controls::{
    self as c, Dropdown, DropdownOption, MenuItem, SettingsCard, SettingsExpander, SettingsGroup, Severity, ToggleSwitch,
    Variant,
};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("colorprofile", page)];
pub const CSS: &str = r#"
.st-cp-key { color: var(--text-2); }
.st-cal-window { background: #000000; }
.st-cal-text { color: #ffffff; font-size: 16px; }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

// ------------------------------------------------------------------------------------------- pure helpers

/// A device kind as words and its glyph.
pub(super) fn kind_text(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "display" => (t("Beeldscherm", "Display"), "display"),
        "printer" => (t("Printer", "Printer"), "print"),
        "scanner" => (t("Scanner", "Scanner"), "print"),
        "camera" => (t("Camera", "Camera"), "camera"),
        "webcam" => (t("Webcam", "Webcam"), "camera"),
        _ => (t("Apparaat", "Device"), "devices"),
    }
}

/// GNOME's device title (cc_color_device_get_title): the laptop's screen and the built-in webcam by name, else the
/// model (the vendor, the id when there is neither).
pub(super) fn device_title(d: &Value) -> String {
    let embedded = d["embedded"] == true;
    match s(&d["kind"]) {
        "display" if embedded => return t("Ingebouwd beeldscherm", "Built-in display").to_owned(),
        "webcam" if embedded => return t("Ingebouwde webcam", "Built-in webcam").to_owned(),
        _ => {}
    }
    let (vendor, model) = (s(&d["vendor"]).trim(), s(&d["model"]).trim());
    let name = match (vendor.is_empty(), model.is_empty()) {
        (false, false) if !model.to_lowercase().starts_with(&vendor.to_lowercase()) => format!("{vendor} {model}"),
        (_, false) => model.to_owned(),
        (false, true) => vendor.to_owned(),
        _ => s(&d["id"]).to_owned(),
    };
    if name.is_empty() { kind_text(s(&d["kind"])).0.to_owned() } else { name }
}

/// A profile's title as GNOME shows it: colord's title (the file name without one), "Standaard: " for a standard
/// space, "Kleurruimte: " for a conversion profile, "Testprofiel: " for an abstract one.
pub(super) fn profile_title(p: &Value) -> String {
    let mut title = s(&p["title"]).trim().to_owned();
    if title.is_empty() {
        title = s(&p["filename"]).rsplit('/').next().unwrap_or("").to_owned();
    }
    if title.is_empty() {
        title = s(&p["id"]).to_owned();
    }
    if !s(&p["standardSpace"]).is_empty() || s(&p["source"]) == "standard" {
        return common::fill(t("Standaard: {name}", "Default: {name}"), &[("name", &title)]);
    }
    match s(&p["kind"]) {
        "colorspace-conversion" => common::fill(t("Kleurruimte: {name}", "Colourspace: {name}"), &[("name", &title)]),
        "abstract" => common::fill(t("Testprofiel: {name}", "Test profile: {name}"), &[("name", &title)]),
        _ => title,
    }
}

/// The profile kind that suits a device kind (cd_device_kind_to_profile_kind).
fn profile_kind(device_kind: &str) -> &'static str {
    match device_kind {
        "display" => "display-device",
        "printer" => "output-device",
        _ => "input-device",
    }
}

/// The profiles GNOME's "Add Profile" offers a device (gcm_prefs_is_profile_suitable_for_device): its kind and
/// colourspace, no standard space, not on the device already.
pub(super) fn compatible(device: &Value, all: &[Value]) -> Vec<Value> {
    let on: HashSet<&str> = device["profiles"].as_array().into_iter().flatten().map(|p| s(&p["path"])).collect();
    let kind = profile_kind(s(&device["kind"]));
    let space = s(&device["colorspace"]);
    all.iter()
        .filter(|p| s(&p["kind"]) == kind)
        .filter(|p| space.is_empty() || s(&p["colorspace"]).is_empty() || s(&p["colorspace"]) == space)
        .filter(|p| s(&p["standardSpace"]).is_empty())
        .filter(|p| !on.contains(s(&p["path"])))
        .cloned()
        .collect()
}

/// GNOME's calibration choices (cc-color-calibrate.ui): quality with its time, display type, white point.
pub(super) fn qualities() -> Vec<(u32, String)> {
    vec![
        (2, format!("{} ({})", t("Hoog", "High"), t("40 minuten", "40 minutes"))),
        (1, format!("{} ({})", t("Gemiddeld", "Medium"), t("30 minuten", "30 minutes"))),
        (0, format!("{} ({})", t("Laag", "Low"), t("15 minuten", "15 minutes"))),
    ]
}

pub(super) fn display_kinds() -> Vec<(u32, &'static str)> {
    vec![
        (1, t("LCD", "LCD")),
        (8, t("LED", "LED")),
        (2, t("CRT", "CRT")),
        (5, t("Projector", "Projector")),
        (9, t("Plasma", "Plasma")),
        (10, t("LCD (CCFL-achtergrondverlichting)", "LCD (CCFL backlight)")),
        (11, t("LCD (RGB-ledachtergrondverlichting)", "LCD (RGB LED backlight)")),
        (12, t("LCD (witte ledachtergrondverlichting)", "LCD (white LED backlight)")),
        (13, t("Breed kleurbereik LCD (CCFL-achtergrondverlichting)", "Wide gamut LCD (CCFL backlight)")),
        (14, t("Breed kleurbereik LCD (RGB-ledachtergrondverlichting)", "Wide gamut LCD (RGB LED backlight)")),
    ]
}

pub(super) fn whitepoints() -> Vec<(u32, &'static str)> {
    vec![
        (0, t("Eigen witpunt van het beeldscherm", "Native to display")),
        (5000, t("D50 (drukwerk en publicaties)", "D50 (printing and publishing)")),
        (5500, t("D55", "D55")),
        (6500, t("D65 (fotografie en grafisch werk)", "D65 (photography and graphics)")),
        (7500, t("D75", "D75")),
    ]
}

/// colord-session's questions during a calibration (CdSessionInteraction), as GNOME asks them.
pub(super) fn interaction_text(code: i64) -> &'static str {
    match code {
        0 => t("Plaats de kleursensor op het scherm en kies Hervatten.", "Place the colour sensor on the screen and choose Resume."),
        1 => t("Zet de kleursensor in de kalibratiestand en kies Hervatten.", "Set the colour sensor to calibration mode and choose Resume."),
        2 => t("Zet de kleursensor in de meetstand en kies Hervatten.", "Set the colour sensor to measurement mode and choose Resume."),
        3 => t("Sluit het deksel van de laptop niet tijdens het kalibreren.", "Don’t close the laptop lid while calibrating."),
        _ => t("Kies Hervatten om verder te gaan.", "Choose Resume to continue."),
    }
}

// ------------------------------------------------------------------------------------------------- the page

struct ColorPage {
    p: Page,
    state: RefCell<Value>,
    /// The devices whose expander is open (their colord paths).
    open: RefCell<HashSet<String>>,
    busy: Cell<bool>,
    body: gtk::Box,
}

impl ColorPage {
    async fn reload(self: &Rc<Self>) {
        let st = self.p.get("devices.color.get", json!({})).await;
        if let Some(st) = st.filter(|v| v.is_object())
            && self.p.alive()
        {
            *self.state.borrow_mut() = st;
            self.paint();
        }
    }

    fn write(self: &Rc<Self>, method: &'static str, args: Value, error: &'static str) {
        let me = self.clone();
        self.p.spawn(async move {
            let _ = me.p.run_with(method, args, Some(error), false).await;
            if me.p.alive() {
                me.reload().await;
            }
        });
    }

    /// GNOME's Add Profile dialog: the compatible profiles and "Bestand importeren…".
    fn add_profile(self: &Rc<Self>, device: &Value) {
        let all = self.state.borrow()["profiles"].as_array().cloned().unwrap_or_default();
        let offered = compatible(device, &all);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 10);
        body.set_widget_name("colorprofile-add-dialog");
        let options: Vec<(String, String)> = offered.iter().map(|p| (s(&p["path"]).to_owned(), profile_title(p))).collect();
        let refs: Vec<(&str, &str)> = options.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        let radio = c::RadioGroup::new(&refs, refs.first().map(|(v, _)| *v), None, false);
        radio.widget().set_widget_name("colorprofile-add-list");
        if offered.is_empty() {
            body.append(&c::label(t("Er zijn geen andere profielen voor dit apparaat. Importeer een ICC-bestand.", "There are no other profiles for this device. Import an ICC file."), "", true));
        } else {
            body.append(&c::label(t("Kies een profiel voor dit apparaat.", "Choose a profile for this device."), "", true));
            body.append(radio.widget());
        }
        let import = c::button(t("Bestand importeren…", "Import file…"), Variant::Standard);
        import.set_widget_name("colorprofile-import");
        import.set_halign(gtk::Align::Start);
        body.append(&import);
        let dialog = c::Dialog::new(t("Profiel toevoegen", "Add profile"))
            .body(&body)
            .width(500)
            .close_label(t("Annuleren", "Cancel"));
        let dialog = if offered.is_empty() { dialog } else { dialog.primary(t("Toevoegen", "Add")) };
        let device_path = s(&device["path"]).to_owned();
        {
            let (me, dlg, dev) = (self.clone(), dialog.clone(), device_path.clone());
            import.connect_clicked(move |_| {
                dlg.close(c::DialogResult::Close);
                me.import(&dev);
            });
        }
        let me = self.clone();
        dialog.show(&self.p.anchor(), move |r| {
            if r == c::DialogResult::Primary
                && let Some(profile) = radio.value()
            {
                me.write("devices.color.addProfile", json!({"device": device_path, "profile": profile}), t("Het profiel kon niet worden toegevoegd", "Couldn’t add the profile"));
            }
        });
    }

    /// GNOME's Import File…: an ICC file from the file chooser.
    fn import(self: &Rc<Self>, device: &str) {
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(t("ICC-kleurprofielen", "ICC colour profiles")));
        for pattern in ["*.icc", "*.icm", "*.ICC", "*.ICM"] {
            filter.add_pattern(pattern);
        }
        filter.add_mime_type("application/vnd.iccprofile");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::new();
        dialog.set_title(t("Een kleurprofiel importeren", "Import a colour profile"));
        dialog.set_modal(true);
        dialog.set_filters(Some(&filters));
        dialog.set_default_filter(Some(&filter));
        let (me, device) = (self.clone(), device.to_owned());
        dialog.open(Some(self.p.window().window()), gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result
                && let Some(path) = file.path().and_then(|p| p.to_str().map(str::to_owned))
            {
                me.write("devices.color.importProfile", json!({"device": device, "path": path}), t("Het profiel kon niet worden geïmporteerd", "Couldn’t import the profile"));
            }
        });
    }

    /// GNOME's calibration assistant in one dialog, then the full-screen calibration.
    fn calibrate(self: &Rc<Self>, device: &Value) {
        let sensors = self.state.borrow()["sensors"].as_array().cloned().unwrap_or_default();
        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.set_widget_name("colorprofile-calibrate-dialog");
        body.append(&c::label(
            t(
                "Kalibreren maakt een kleurprofiel voor je beeldscherm. Hoe langer het kalibreren duurt, hoe beter het profiel. Je kunt het apparaat tijdens het kalibreren niet gebruiken.",
                "Calibration creates a colour profile for your display. The longer calibration takes, the better the profile. You can’t use the device while it’s being calibrated.",
            ),
            "",
            true,
        ));
        let q: Vec<(String, String)> = qualities().into_iter().map(|(v, l)| (v.to_string(), l)).collect();
        let quality = Dropdown::new(q.iter().map(|(v, l)| DropdownOption::new(v, l)).collect(), Some("1"));
        quality.widget().set_widget_name("colorprofile-cal-quality");
        body.append(&c::field(t("Kwaliteit", "Quality"), quality.widget(), None));
        let sensor = Dropdown::new(
            sensors
                .iter()
                .map(|x| DropdownOption::new(s(&x["path"]), &[s(&x["vendor"]), s(&x["model"])].join(" ").trim().to_owned()))
                .collect(),
            sensors.first().map(|x| s(&x["path"])),
        );
        sensor.widget().set_widget_name("colorprofile-cal-sensor");
        body.append(&c::field(t("Kalibratieapparaat", "Calibration device"), sensor.widget(), None));
        let k: Vec<(String, &str)> = display_kinds().into_iter().map(|(v, l)| (v.to_string(), l)).collect();
        let kind = Dropdown::new(k.iter().map(|(v, l)| DropdownOption::new(v, l)).collect(), Some("12"));
        kind.widget().set_widget_name("colorprofile-cal-kind");
        body.append(&c::field(t("Beeldschermtype", "Display type"), kind.widget(), None));
        let w: Vec<(String, &str)> = whitepoints().into_iter().map(|(v, l)| (v.to_string(), l)).collect();
        let white = Dropdown::new(w.iter().map(|(v, l)| DropdownOption::new(v, l)).collect(), Some("6500"));
        white.widget().set_widget_name("colorprofile-cal-whitepoint");
        body.append(&c::field(
            t("Witpunt van het profiel", "Profile white point"),
            white.widget(),
            Some(t("De meeste beeldschermen worden gekalibreerd op D65.", "Most displays should be calibrated to D65.")),
        ));
        body.append(&c::label(
            t(
                "Stel het beeldscherm in op de helderheid die je meestal gebruikt: daar is het kleurbeheer het nauwkeurigst.",
                "Set the display to the brightness you usually use: colour management is most accurate there.",
            ),
            "st-cp-key",
            true,
        ));
        let title = c::text_box("");
        title.set_text(&device_title(device));
        title.set_widget_name("colorprofile-cal-title");
        body.append(&c::field(t("Profielnaam", "Profile name"), &title, None));
        let device_path = s(&device["path"]).to_owned();
        let me = self.clone();
        c::Dialog::new(t("Beeldscherm kalibreren", "Calibrate display"))
            .body(&body)
            .width(540)
            .primary(t("Kalibreren starten", "Start calibration"))
            .close_label(t("Annuleren", "Cancel"))
            .show(&self.p.anchor(), move |r| {
                if r != c::DialogResult::Primary {
                    return;
                }
                let num = |d: &Dropdown| d.selected().and_then(|v| v.parse::<u32>().ok()).unwrap_or(0);
                let mut args = json!({
                    "device": device_path, "quality": num(&quality), "kind": num(&kind), "whitepoint": num(&white),
                    "title": title.text().trim().to_owned(),
                });
                if let Some(sp) = sensor.selected() {
                    args["sensor"] = json!(sp);
                }
                let me2 = me.clone();
                me.p.spawn(async move {
                    let r = me2.p.run_with("devices.color.calibrate.start", args, Some(t("Kalibreren kon niet worden gestart", "Couldn’t start the calibration")), false).await;
                    if r.is_some_and(|v| v["dryRun"] != true) && me2.p.alive() {
                        me2.calibration_window();
                    }
                });
            });
    }

    /// GNOME's full-screen calibration window: the sample colour colord-session asks for, its questions with
    /// Hervatten, the progress, Annuleren and, at the end, Gereed.
    fn calibration_window(self: &Rc<Self>) {
        let window = gtk::Window::new();
        window.set_title(Some(t("Beeldscherm kalibreren", "Calibrate display")));
        window.set_decorated(false);
        window.add_css_class("st-cal-window");
        window.set_widget_name("colorprofile-calibration");
        let sample = Rc::new(Cell::new((0.5f64, 0.5f64, 0.5f64)));
        let patch = gtk::DrawingArea::new();
        patch.set_content_width(360);
        patch.set_content_height(360);
        patch.set_halign(gtk::Align::Center);
        {
            let sample = sample.clone();
            patch.set_draw_func(move |_, cr, w, h| {
                let (r, g, b) = sample.get();
                cr.set_source_rgb(r, g, b);
                cr.rectangle(0.0, 0.0, w as f64, h as f64);
                let _ = cr.fill();
            });
        }
        let status = c::label(t("Verstoor het kalibratieapparaat niet zolang het kalibreren bezig is.", "Don’t disturb the calibration device while calibration is in progress."), "st-cal-text", true);
        status.set_justify(gtk::Justification::Center);
        let progress = c::ProgressBar::new();
        progress.widget().set_size_request(360, -1);
        progress.widget().set_halign(gtk::Align::Center);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_halign(gtk::Align::Center);
        let cancel = c::button(t("Annuleren", "Cancel"), Variant::Standard);
        cancel.set_widget_name("colorprofile-cal-cancel");
        let resume = c::button(t("Hervatten", "Resume"), Variant::Accent);
        resume.set_widget_name("colorprofile-cal-resume");
        resume.set_visible(false);
        let done = c::button(t("Gereed", "Done"), Variant::Accent);
        done.set_widget_name("colorprofile-cal-done");
        done.set_visible(false);
        buttons.append(&cancel);
        buttons.append(&resume);
        buttons.append(&done);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 18);
        column.set_valign(gtk::Align::Center);
        column.set_halign(gtk::Align::Center);
        column.append(&patch);
        column.append(&status);
        column.append(progress.widget());
        column.append(&buttons);
        window.set_child(Some(&column));
        window.set_transient_for(Some(self.p.window().window()));
        window.fullscreen();
        window.present();
        let finished = Rc::new(Cell::new(false));
        {
            let (pp, w, finished) = (self.p.clone(), window.clone(), finished.clone());
            cancel.connect_clicked(move |_| {
                if !finished.get() {
                    let wait = pp.run_with("devices.color.calibrate.cancel", json!({}), Some(t("Annuleren lukte niet", "Couldn’t cancel")), true);
                    pp.spawn(async move {
                        let _ = wait.await;
                    });
                }
                w.close();
            });
        }
        {
            let (pp, resume2, status2) = (self.p.clone(), resume.clone(), status.clone());
            resume.connect_clicked(move |_| {
                resume2.set_visible(false);
                status2.set_text(t("Verstoor het kalibratieapparaat niet zolang het kalibreren bezig is.", "Don’t disturb the calibration device while calibration is in progress."));
                let wait = pp.run_with("devices.color.calibrate.resume", json!({}), Some(t("Hervatten lukte niet", "Couldn’t resume")), true);
                pp.spawn(async move {
                    let _ = wait.await;
                });
            });
        }
        {
            let (me, w) = (self.clone(), window.clone());
            done.connect_clicked(move |_| {
                w.close();
                let me2 = me.clone();
                me.p.spawn(async move { me2.reload().await });
            });
        }
        let (patch2, status2, resume2, done2, cancel2, progress2) = (patch.clone(), status.clone(), resume.clone(), done.clone(), cancel.clone(), progress.clone());
        self.p.on("devices.calibrate", move |d| {
            match s(&d["state"]) {
                "sample" => {
                    let ch = |k: &str| d["sample"][k].as_f64().unwrap_or(0.0).clamp(0.0, 1.0);
                    sample.set((ch("red"), ch("green"), ch("blue")));
                    patch2.queue_draw();
                }
                "interaction" => {
                    let message = s(&d["message"]);
                    let text = interaction_text(d["code"].as_i64().unwrap_or(-1));
                    status2.set_text(&if message.is_empty() { text.to_owned() } else { format!("{message}\n{text}") });
                    resume2.set_visible(true);
                }
                "progress" => progress2.set_fraction(d["progress"].as_f64().unwrap_or(0.0) / 100.0),
                "finished" => {
                    finished.set(true);
                    status2.set_text(if d["ok"] == true {
                        t("Het kalibreren is voltooid. Het nieuwe profiel is het standaardprofiel van dit beeldscherm.", "Calibration is complete. The new profile is this display’s default profile.")
                    } else {
                        t("Het kalibreren is mislukt of afgebroken.", "Calibration failed or was cancelled.")
                    });
                    resume2.set_visible(false);
                    cancel2.set_visible(false);
                    done2.set_visible(true);
                    progress2.set_fraction(1.0);
                }
                // "gamma": the host put colord-session's ramp into Mutter's calibration session itself
                _ => {}
            }
        });
        // the page goes (another page, the window closes): the calibration window goes with it
        let w = window.clone();
        self.p.on_cleanup(move || w.close());
    }

    fn profile_row(self: &Rc<Self>, device: &Value, p: &Value, i: usize, j: usize) -> gtk::Widget {
        let mut lines: Vec<String> = Vec::new();
        if let Some(created) = p["created"].as_f64() {
            lines.push(kit::format::file_time(created * 1000.0));
        }
        if p["systemWide"] == true {
            lines.push(t("Voor alle gebruikers", "For all users").to_owned());
        }
        if p["warnings"].as_array().is_some_and(|w| !w.is_empty()) {
            lines.push(t("Er zijn problemen met dit profiel gevonden", "Problems were found with this profile").to_owned());
        }
        let is_default = device["default"] == p["path"];
        let card = SettingsCard::new(if is_default { "check" } else { "" }, &profile_title(p), &lines.join(" · "));
        card.widget().set_widget_name(&format!("colorprofile-{i}-profile-{j}"));
        let more = c::icon_button("more", t("Meer opties", "More options"), 32);
        more.set_widget_name(&format!("colorprofile-{i}-profile-{j}-more"));
        let (me, dev, prof) = (self.clone(), s(&device["path"]).to_owned(), p.clone());
        more.connect_clicked(move |b| {
            let mut items = Vec::new();
            if prof["systemWide"] != true {
                let (me1, dev1, path1) = (me.clone(), dev.clone(), s(&prof["path"]).to_owned());
                items.push(
                    MenuItem::new(t("Instellen voor alle gebruikers", "Set for all users"), move || {
                        me1.write("devices.color.installSystemWide", json!({"profile": path1, "device": dev1}), t("Het profiel kon niet voor alle gebruikers worden ingesteld", "Couldn’t set the profile for all users"))
                    })
                    .glyph("shield")
                    .name("colorprofile-system-wide"),
                );
            }
            let (me2, dev2, path2) = (me.clone(), dev.clone(), s(&prof["path"]).to_owned());
            items.push(
                MenuItem::new(t("Verwijderen", "Remove"), move || {
                    me2.write("devices.color.removeProfile", json!({"device": dev2, "profile": path2}), t("Het profiel kon niet worden verwijderd", "Couldn’t remove the profile"))
                })
                .glyph("trash")
                .name("colorprofile-remove"),
            );
            c::show_menu_for(b, items);
        });
        card.set_control(&more);
        card.widget().clone()
    }

    fn device_row(self: &Rc<Self>, d: &Value, i: usize, can_calibrate: bool) -> gtk::Widget {
        let path = s(&d["path"]).to_owned();
        let (kind, glyph) = kind_text(s(&d["kind"]));
        let profiles = d["profiles"].as_array().cloned().unwrap_or_default();
        let default = profiles.iter().find(|p| p["path"] == d["default"]).map(profile_title);
        let desc = format!("{kind} · {}", default.unwrap_or_else(|| t("Niet gekalibreerd", "Not calibrated").to_owned()));
        let exp = SettingsExpander::new(glyph, &device_title(d), &desc);
        exp.header_button().set_widget_name(&format!("colorprofile-device-{i}"));
        // Kleurbeheer gebruiken voor dit apparaat
        let enabled = d["enabled"] != false;
        let toggle = ToggleSwitch::new(enabled);
        toggle.switch().set_widget_name(&format!("colorprofile-{i}-enabled"));
        {
            let (me, path) = (self.clone(), path.clone());
            toggle.connect_toggled(move |on| {
                if me.busy.get() {
                    return;
                }
                me.write("devices.color.setEnabled", json!({"device": path, "on": on}), t("Kleurbeheer kon niet worden gewijzigd", "Couldn’t change colour management"));
            });
        }
        let card = SettingsCard::new("", t("Kleurbeheer gebruiken voor dit apparaat", "Use colour management for this device"), "");
        card.set_control(toggle.widget());
        exp.add(card.widget());
        // Kleurprofiel (Windows' dropdown; GNOME's default radio)
        if !profiles.is_empty() {
            let dd = Dropdown::new(
                profiles.iter().map(|p| DropdownOption::new(s(&p["path"]), &profile_title(p))).collect(),
                d["default"].as_str(),
            );
            dd.widget().set_widget_name(&format!("colorprofile-{i}-default"));
            dd.widget().set_sensitive(enabled);
            {
                let (me, path, current) = (self.clone(), path.clone(), s(&d["default"]).to_owned());
                dd.connect_changed(move |v| {
                    if v != current {
                        me.write("devices.color.makeDefault", json!({"device": path, "profile": v}), t("Het standaardprofiel kon niet worden ingesteld", "Couldn’t set the default profile"));
                    }
                });
            }
            let card = SettingsCard::new("", t("Kleurprofiel", "Colour profile"), t("Het profiel dat dit apparaat gebruikt", "The profile this device uses"));
            card.set_control(dd.widget());
            exp.add(card.widget());
            for (j, p) in profiles.iter().enumerate() {
                exp.add(&self.profile_row(d, p, i, j));
            }
        }
        // Profiel toevoegen
        let add = SettingsCard::new("plus", t("Profiel toevoegen", "Add profile"), "");
        let b = c::button(t("Profiel toevoegen", "Add profile"), Variant::Standard);
        b.set_widget_name(&format!("colorprofile-{i}-add"));
        {
            let (me, dev) = (self.clone(), d.clone());
            b.connect_clicked(move |_| me.add_profile(&dev));
        }
        add.set_control(&b);
        exp.add(add.widget());
        if can_calibrate && s(&d["kind"]) == "display" {
            let cal = SettingsCard::new("color", t("Kalibreren", "Calibrate"), t("Een nieuw profiel maken met een kleursensor", "Create a new profile with a colour sensor"));
            let b = c::button(t("Kalibreren…", "Calibrate…"), Variant::Standard);
            b.set_widget_name(&format!("colorprofile-{i}-calibrate"));
            b.set_sensitive(d["inhibited"] != true);
            let (me, dev) = (self.clone(), d.clone());
            b.connect_clicked(move |_| me.calibrate(&dev));
            cal.set_control(&b);
            exp.add(cal.widget());
        }
        if self.open.borrow().contains(&path) {
            exp.set_expanded(true);
        }
        let me = self.clone();
        exp.connect_toggled(move |open| {
            if open {
                me.open.borrow_mut().insert(path.clone());
            } else {
                me.open.borrow_mut().remove(&path);
            }
        });
        exp.widget().clone().upcast()
    }

    fn paint(self: &Rc<Self>) {
        let st = self.state.borrow().clone();
        self.busy.set(true);
        let mut out: Vec<gtk::Widget> = Vec::new();
        if st["available"] != true {
            let empty = c::empty_state(
                "color",
                t("Kleurbeheer is niet beschikbaar", "Colour management isn’t available"),
                t(
                    "De kleurbeheerservice (colord) draait niet op deze computer, dus kleurprofielen kunnen hier niet worden beheerd.",
                    "The colour management service (colord) isn’t running on this computer, so colour profiles can’t be managed here.",
                ),
                None,
            );
            empty.set_widget_name("colorprofile-unavailable");
            out.push(empty.upcast());
        } else {
            let devices = st["devices"].as_array().cloned().unwrap_or_default();
            let can_calibrate = st["sensors"].as_array().is_some_and(|a| !a.is_empty());
            if devices.is_empty() {
                let n = common::notice_full(
                    Severity::Info,
                    t("Geen apparaten", "No devices"),
                    t("Er zijn geen apparaten gevonden waarvan de kleur kan worden beheerd.", "No devices that can be colour managed were found."),
                );
                n.widget().set_widget_name("colorprofile-none");
                out.push(n.widget().clone().upcast());
            } else {
                let group = SettingsGroup::new(t("Apparaten", "Devices"), "");
                group.widget().set_widget_name("colorprofile-devices");
                for (i, d) in devices.iter().enumerate() {
                    group.add(&self.device_row(d, i, can_calibrate));
                }
                out.push(group.widget().clone().upcast());
            }
            if !can_calibrate {
                let n = common::notice(t(
                    "Sluit een kleursensor aan om een beeldscherm te kalibreren.",
                    "Connect a colour sensor to calibrate a display.",
                ));
                n.widget().set_widget_name("colorprofile-no-sensor");
                out.push(n.widget().clone().upcast());
            }
        }
        common::set_slot(&self.body, &out);
        self.busy.set(false);
    }
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let st = p.get("devices.color.get", json!({})).await;
        if !p.alive() {
            return Vec::new();
        }
        let st = st.filter(|v| v.is_object()).unwrap_or_else(|| json!({ "available": false }));
        let body = common::slot();
        let page = Rc::new(ColorPage {
            p: p.clone(),
            state: RefCell::new(st),
            open: RefCell::new(HashSet::new()),
            busy: Cell::new(false),
            body: body.clone(),
        });
        // the first device opens by itself, as Windows shows the display's profile at once
        if let Some(first) = page.state.borrow()["devices"].as_array().and_then(|a| a.first())
            && let Some(path) = first["path"].as_str()
        {
            page.open.borrow_mut().insert(path.to_owned());
        }
        {
            let pg = page.clone();
            p.on("devices.changed", move |d| {
                if d["what"] == "color" {
                    let me = pg.clone();
                    pg.p.spawn(async move { me.reload().await });
                }
            });
        }
        page.paint();
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card(&p, "display").widget());
        related.add(common::link_card(&p, "printers").widget());
        vec![body.upcast(), related.widget().clone().upcast()]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_titled_as_gnome_titles_them() {
        assert_eq!(device_title(&json!({"kind": "display", "embedded": true, "model": "0x1234"})), t("Ingebouwd beeldscherm", "Built-in display"));
        assert_eq!(device_title(&json!({"kind": "display", "vendor": "Dell", "model": "U2720Q"})), "Dell U2720Q");
        assert_eq!(device_title(&json!({"kind": "printer", "vendor": "HP", "model": "HP LaserJet M404"})), "HP LaserJet M404", "no vendor twice");
        assert_eq!(device_title(&json!({"kind": "scanner", "id": "sane-1"})), "sane-1");
        assert_eq!(device_title(&json!({"kind": "camera"})), t("Camera", "Camera"));
    }

    #[test]
    fn profiles_are_titled_and_offered_as_gnome_does() {
        assert_eq!(profile_title(&json!({"title": "sRGB", "kind": "colorspace-conversion", "standardSpace": "srgb"})),
            common::fill(t("Standaard: {name}", "Default: {name}"), &[("name", "sRGB")]));
        assert_eq!(profile_title(&json!({"title": "", "filename": "/home/u/.local/share/icc/dell.icc", "kind": "display-device"})), "dell.icc");
        assert_eq!(profile_title(&json!({"title": "Lab", "kind": "abstract"})), common::fill(t("Testprofiel: {name}", "Test profile: {name}"), &[("name", "Lab")]));
        let device = json!({"kind": "display", "colorspace": "rgb", "profiles": [{"path": "/p/1"}]});
        let all = vec![
            json!({"path": "/p/1", "kind": "display-device", "colorspace": "rgb"}),
            json!({"path": "/p/2", "kind": "display-device", "colorspace": "rgb"}),
            json!({"path": "/p/3", "kind": "output-device", "colorspace": "cmyk"}),
            json!({"path": "/p/4", "kind": "display-device", "colorspace": "rgb", "standardSpace": "srgb"}),
            json!({"path": "/p/5", "kind": "display-device", "colorspace": "gray"}),
        ];
        let offered = compatible(&device, &all);
        assert_eq!(offered.len(), 1, "same kind and colourspace, no standard space, not on the device yet: {offered:?}");
        assert_eq!(offered[0]["path"], "/p/2");
        let printer = json!({"kind": "printer", "colorspace": "cmyk", "profiles": []});
        assert_eq!(compatible(&printer, &all)[0]["path"], "/p/3");
    }

    #[test]
    fn the_calibration_offers_gnomes_choices() {
        assert_eq!(qualities().iter().map(|q| q.0).collect::<Vec<_>>(), [2, 1, 0]);
        assert_eq!(display_kinds().iter().map(|k| k.0).collect::<Vec<_>>(), [1, 8, 2, 5, 9, 10, 11, 12, 13, 14]);
        assert_eq!(whitepoints().iter().map(|w| w.0).collect::<Vec<_>>(), [0, 5000, 5500, 6500, 7500]);
        assert_ne!(interaction_text(0), interaction_text(9));
    }
}
