//! Instellingen › Bluetooth en apparaten › Pen en tablet (batch A, notes/claude/settings-parity.md §2, plan.json
//! "pen", amendments 8 and 9): every function of GNOME Settings 50's Wacom panel, in the layout and wording of
//! Windows 11's Bluetooth & devices › Pen & Windows Ink. The page needs `caps.tablet` (registry.rs).
//!
//! * The tablet: `input.tablets` (udev, /sys, libwacom); with more than one, a drop-down chooses which.
//! * Kies met welke hand je schrijft = the relocatable `org.gnome.desktop.peripherals.tablet` at
//!   `/org/gnome/desktop/peripherals/tablets/<vid>:<pid>/` `left-handed` (GNOME's Button Location; shown unless
//!   libwacom says the tablet cannot be turned round).
//! * Tablet (GNOME's rows): Tabletmodus = `mapping` absolute / relative and Hoogte-breedteverhouding behouden =
//!   `keep-aspect` for a pen tablet; Toewijzen aan beeldscherm = `output` ([vendor, product, serial] from Mutter's
//!   DisplayConfig, ["", "", ""] automatic) unless the tablet is built into the computer; Kalibreren (a pen display or
//!   a built-in screen) = the 4-point full-screen calibrator writing `area`; Knoppen toewijzen (a tablet with a pad)
//!   = `input.tablet.mapButtons`, GNOME Shell's pad OSD.
//! * Penknoppen: the relocatable `org.gnome.desktop.peripherals.tablet.stylus` at
//!   `/org/gnome/desktop/peripherals/stylus/<serial>/` (`default-<vid>:<pid>` for a pen without a serial,
//!   amendment 9): Knop 1 / 2 / 3 = `button-action`, `secondary-button-action`, `tertiary-button-action` with GNOME's
//!   choices, Toetsaanslag verzenden with its key (`*-keybinding`, pickers::KeyCapture); Windows' "De bovenste knop
//!   van mijn pen gebruiken om inkt te wissen" = `eraser-button-mode` 'default' (ticked) / 'button'. The pens are
//!   the serial-less id plus every pen GTK knows (GdkSeat's tools; one that comes in later rebuilds the page).
//! * Pendruk: Gevoel van pendruk / gumdruk = `pressure-curve` / `eraser-pressure-curve`, GNOME's seven preset curves
//!   from Zacht to Stevig.
//! * Instellingen testen: a drawing area with the pen's pressure (it also learns the pen's serial).
//! * Aanraakscherm toewijzen aan beeldscherm (amendment 8) for every touchscreen: the relocatable
//!   `org.gnome.desktop.peripherals.touchscreen` at `touchscreens/<vid>:<pid>/` `output`.
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use crate::pickers::{Capture, KeyCapture};
use kit::controls::{self as c, Dialog, Dropdown, DropdownOption, SettingsCard, SettingsGroup, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::RefCell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("pen", pen)];
pub const CSS: &str = r#"
.st-pen-canvas {
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
}
.st-pen-calibrate { background-color: #000000; color: #ffffff; }
"#;

const TABLET: &str = "org.gnome.desktop.peripherals.tablet";
const STYLUS: &str = "org.gnome.desktop.peripherals.tablet.stylus";
pub(super) const TOUCHSCREEN: &str = "org.gnome.desktop.peripherals.touchscreen";

thread_local! {
    /// The tablet (vid:pid) and the pen (stylus id) chosen on this page; they survive the page's rebuilds.
    static CHOSEN: RefCell<(Option<String>, Option<String>)> = const { RefCell::new((None, None)) };
}

// ------------------------------------------------------------------------------------------ pure parts

/// The settings id of a pen (libmutter-18): its serial in hex, or `default-<vid>:<pid>` without one.
fn stylus_id(serial: u64, vidpid: &str) -> String {
    if serial == 0 { format!("default-{vidpid}") } else { format!("{serial:x}") }
}

fn tablet_path(vidpid: &str) -> String {
    format!("/org/gnome/desktop/peripherals/tablets/{vidpid}/")
}

fn stylus_path(id: &str) -> String {
    format!("/org/gnome/desktop/peripherals/stylus/{id}/")
}

pub(super) fn touchscreen_path(vidpid: &str) -> String {
    format!("/org/gnome/desktop/peripherals/touchscreens/{vidpid}/")
}

/// GNOME Settings' pressure curves (cc-wacom-stylus-page.c), from soft to firm; the middle one is linear.
const CURVES: [[i64; 4]; 7] = [
    [0, 75, 25, 100],
    [0, 50, 50, 100],
    [0, 25, 75, 100],
    [0, 0, 100, 100],
    [25, 0, 100, 75],
    [50, 0, 100, 50],
    [75, 0, 100, 25],
];

/// The slider step (0 soft - 6 firm) of a pressure curve: the nearest preset.
fn curve_ui(v: &Value) -> f64 {
    let Some(points) = v.as_array().filter(|a| a.len() == 4) else {
        return 3.0;
    };
    let points: Vec<i64> = points.iter().map(|p| p.as_i64().unwrap_or(0)).collect();
    let distance = |c: &[i64; 4]| c.iter().zip(&points).map(|(a, b)| (a - b).abs()).sum::<i64>();
    CURVES.iter().enumerate().min_by_key(|(_, c)| distance(c)).map(|(i, _)| i as f64).unwrap_or(3.0)
}

/// The preset curve of a slider step.
fn curve_gs(n: f64) -> Value {
    json!(CURVES[(n.round() as usize).min(CURVES.len() - 1)])
}

/// Mutter's tablet area (padding left, right, top, bottom as fractions) from the four calibration taps, each a
/// position (0..1) on the monitor of the targets at 1/8 and 7/8 (upper left, upper right, lower left, lower right),
/// tapped while the area was [0, 0, 0, 0]. None for taps too close together (a misclick) or a result out of range.
fn calibration_area(taps: &[(f64, f64); 4]) -> Option<[f64; 4]> {
    let x_min = (taps[0].0 + taps[2].0) / 2.0;
    let x_max = (taps[1].0 + taps[3].0) / 2.0;
    let y_min = (taps[0].1 + taps[1].1) / 2.0;
    let y_max = (taps[2].1 + taps[3].1) / 2.0;
    if x_max - x_min < 0.3 || y_max - y_min < 0.3 {
        return None;
    }
    // the targets are 6/8 of the screen apart: stretch to the whole screen
    let (w, h) = ((x_max - x_min) * 8.0 / 6.0, (y_max - y_min) * 8.0 / 6.0);
    let (left, top) = (x_min - w / 8.0, y_min - h / 8.0);
    let area = [left, 1.0 - (left + w), top, 1.0 - (top + h)];
    area.iter().all(|p| (-0.25..=0.4).contains(p)).then(|| area.map(|p| (p * 10000.0).round() / 10000.0))
}

/// The targets of the calibrator, in tap order.
const TARGETS: [(f64, f64); 4] = [(0.125, 0.125), (0.875, 0.125), (0.125, 0.875), (0.875, 0.875)];

/// What kind of tablet this is, in words.
fn kind_text(kind: &str) -> &'static str {
    match kind {
        "display" => t("Penbeeldscherm", "Pen display"),
        "builtin" => t("Ingebouwd scherm met pen", "Built-in screen with pen"),
        _ => t("Tekentablet", "Drawing tablet"),
    }
}

/// A monitor's label: Mutter's display name, else vendor and product.
fn monitor_label(m: &Value) -> String {
    let name = common::s(&m["name"]);
    if !name.is_empty() {
        return name.to_owned();
    }
    let both = format!("{} {}", common::s(&m["vendor"]), common::s(&m["product"]));
    if both.trim().is_empty() { common::s(&m["connector"]).to_owned() } else { both.trim().to_owned() }
}

/// The [vendor, product, serial] of a monitor, the value of the output key.
fn output_of(m: &Value) -> Value {
    json!([common::s(&m["vendor"]), common::s(&m["product"]), common::s(&m["serial"])])
}

/// The choices of an output key: Automatisch, every monitor, and a value that names none of them.
fn output_options(monitors: &[Value], current: &Value) -> Vec<(Value, String)> {
    let mut out = vec![(json!(["", "", ""]), t("Automatisch", "Automatic").to_owned())];
    for m in monitors {
        out.push((output_of(m), monitor_label(m)));
    }
    let known = out.iter().any(|(v, _)| v == current);
    if !known && current.as_array().is_some_and(|a| a.iter().any(|s| common::s(s) != "")) {
        out.push((current.clone(), t("Ander beeldscherm (niet aangesloten)", "Another display (not connected)").to_owned()));
    }
    out
}

/// GNOME's stylus button actions in its order, with Windows' words.
fn action_options() -> Vec<(Value, String)> {
    [
        ("default", t("Linkermuisknop", "Left mouse button")),
        ("middle", t("Middelste muisknop", "Middle mouse button")),
        ("right", t("Rechtermuisknop", "Right mouse button")),
        ("back", t("Terug", "Back")),
        ("forward", t("Vooruit", "Forward")),
        ("switch-monitor", t("Naar ander beeldscherm", "Switch display")),
        ("keybinding", t("Toetsaanslag verzenden", "Send keystroke")),
    ]
    .into_iter()
    .map(|(v, l)| (json!(v), l.to_owned()))
    .collect()
}

/// A key combination as Windows writes it ("Ctrl+Z"); "" for none.
fn accel_label(accel: &str) -> String {
    if accel.is_empty() {
        return String::new();
    }
    match gtk::accelerator_parse(accel) {
        Some((key, mods)) => gtk::accelerator_get_label(key, mods).to_string(),
        None => accel.to_owned(),
    }
}

// ------------------------------------------------------------------------------------------ the pens GTK knows

fn is_pen(tool: &gdk::DeviceTool) -> bool {
    matches!(
        tool.tool_type(),
        gdk::DeviceToolType::Pen | gdk::DeviceToolType::Eraser | gdk::DeviceToolType::Brush | gdk::DeviceToolType::Pencil | gdk::DeviceToolType::Airbrush
    )
}

fn seat() -> Option<gdk::Seat> {
    gdk::Display::default().and_then(|d| d.default_seat())
}

/// The pens of the chosen tablet: its serial-less id, then every pen GTK has seen (newest last).
fn styli(tablet: &Value) -> Vec<String> {
    let vidpid = common::s(&tablet["vidpid"]);
    let mut out: Vec<String> = tablet["styli"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect();
    if let Some(seat) = seat() {
        for tool in seat.tools() {
            let id = stylus_id(tool.serial(), vidpid);
            if is_pen(&tool) && !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

fn stylus_label(id: &str) -> String {
    if id.starts_with("default-") {
        t("Pen zonder serienummer", "Pen without a serial number").to_owned()
    } else {
        common::fill(t("Pen met serienummer {serial}", "Pen with serial number {serial}"), &[("serial", id)])
    }
}

// ------------------------------------------------------------------------------------------ dialogs

/// The calibrator: a full-screen window on `monitor` with one target at a time; `done(Some(area))` after four taps,
/// `done(None)` for Esc or a misclick.
fn calibrate_window(monitor: Option<gdk::Monitor>, done: impl FnOnce(Option<[f64; 4]>) + 'static) {
    let window = gtk::Window::new();
    window.set_title(Some(t("Kalibreren", "Calibrate")));
    window.add_css_class("st-pen-calibrate");
    window.set_widget_name("st-pen-calibrator");
    let taps: Rc<RefCell<Vec<(f64, f64)>>> = Rc::new(RefCell::new(Vec::new()));
    let done: Rc<RefCell<Option<Box<dyn FnOnce(Option<[f64; 4]>)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
    let area = gtk::DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&area));
    let hint = c::label(
        t(
            "Tik met de pen op het midden van elk doel zodra het verschijnt. Druk op Esc om te annuleren.",
            "Tap the centre of each target with the pen as it appears. Press Esc to cancel.",
        ),
        "w11-body-large",
        true,
    );
    hint.set_halign(gtk::Align::Center);
    hint.set_valign(gtk::Align::Center);
    hint.set_justify(gtk::Justification::Center);
    overlay.add_overlay(&hint);
    {
        let taps = taps.clone();
        area.set_draw_func(move |_, cr, w, h| {
            cr.set_source_rgb(0.0, 0.0, 0.0);
            cr.paint().ok();
            let Some((tx, ty)) = TARGETS.get(taps.borrow().len()) else { return };
            let (x, y) = (tx * f64::from(w), ty * f64::from(h));
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            cr.arc(x, y, 16.0, 0.0, std::f64::consts::TAU);
            cr.move_to(x - 28.0, y);
            cr.line_to(x + 28.0, y);
            cr.move_to(x, y - 28.0);
            cr.line_to(x, y + 28.0);
            cr.stroke().ok();
        });
    }
    let finish = {
        let (window, done) = (window.clone(), done.clone());
        Rc::new(move |result: Option<[f64; 4]>| {
            if let Some(f) = done.borrow_mut().take() {
                f(result);
            }
            window.close();
        })
    };
    let click = gtk::GestureClick::new();
    {
        let (taps, area2, finish) = (taps.clone(), area.clone(), finish.clone());
        click.connect_pressed(move |_, _, x, y| {
            let (w, h) = (f64::from(area2.width().max(1)), f64::from(area2.height().max(1)));
            taps.borrow_mut().push((x / w, y / h));
            let list = taps.borrow().clone();
            if list.len() == 4 {
                finish(calibration_area(&[list[0], list[1], list[2], list[3]]));
            } else {
                area2.queue_draw();
            }
        });
    }
    area.add_controller(click);
    let keys = gtk::EventControllerKey::new();
    {
        let finish = finish.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                finish(None);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }
    window.add_controller(keys);
    {
        let done = done.clone();
        window.connect_close_request(move |_| {
            if let Some(f) = done.borrow_mut().take() {
                f(None);
            }
            glib::Propagation::Proceed
        });
    }
    window.set_child(Some(&overlay));
    match monitor {
        Some(m) => window.fullscreen_on_monitor(&m),
        None => window.fullscreen(),
    }
    window.present();
}

/// The GdkMonitor of a tablet's output, else the built-in one, else none (the window's own).
fn calibration_monitor(monitors: &[Value], output: &Value) -> Option<gdk::Monitor> {
    let connector = monitors
        .iter()
        .find(|m| output_of(m) == *output)
        .or_else(|| monitors.iter().find(|m| m["builtin"] == true))
        .map(|m| common::s(&m["connector"]).to_owned())?;
    let list = gdk::Display::default()?.monitors();
    (0..list.n_items())
        .filter_map(|i| list.item(i).and_downcast::<gdk::Monitor>())
        .find(|m| m.connector().as_deref() == Some(connector.as_str()))
}

/// Kalibreren: area to [0, 0, 0, 0] (as GNOME Settings does, so the taps are the pen's own positions), the
/// calibrator, then the new area, or the old one back.
fn calibrate(gs: &Gs, area_key: &GKey, output_key: &GKey, monitors: Vec<Value>, button: &gtk::Button) {
    let before = gs.value(area_key);
    let monitor = calibration_monitor(&monitors, &gs.value(output_key));
    let p = gs.page().clone();
    let key = area_key.clone();
    let args = |value: Value| json!({ "schema": key.schema, "path": key.path, "values": { key.key: value } });
    let wait = p.run("gsettings.set", args(json!([0.0, 0.0, 0.0, 0.0])));
    button.set_sensitive(false);
    let button = button.clone();
    let pp = p.clone();
    p.spawn(async move {
        let Some(answer) = wait.await else {
            button.set_sensitive(true);
            return;
        };
        if answer["dryRun"] == true {
            button.set_sensitive(true);
            return;
        }
        let alive = pp.alive();
        // Even if navigation closed the page while reset was pending, restore its original calibration.
        let finish = move |result: Option<[f64; 4]>| {
            let value = result.map_or_else(|| before.clone(), |area| json!(area));
            let wait = pp.run("gsettings.set", json!({ "schema": key.schema, "path": key.path, "values": { key.key: value } }));
            let p2 = pp.clone();
            pp.spawn(async move {
                let saved = wait.await;
                if result.is_some() && saved.is_none() {
                    // Saving the measured area failed: do not leave the temporary zero calibration behind.
                    let _ = p2.run("gsettings.set", json!({ "schema": key.schema, "path": key.path, "values": { key.key: before } })).await;
                }
                button.set_sensitive(true);
                if !p2.alive() { return; }
                if let Some(answer) = saved {
                    if result.is_some() && answer["dryRun"] != true {
                        common::toast_success(&p2.anchor(), t("De pen is gekalibreerd", "The pen has been calibrated"), 2500);
                    } else if result.is_none() {
                        p2.toast(kit::controls::Toast::new(t("Kalibreren is geannuleerd", "Calibration was cancelled")).timeout(3000));
                    }
                }
                p2.refresh();
            });
        };
        if alive {
            calibrate_window(monitor, finish);
        } else {
            finish(None);
        }
    });
}

/// Instellingen testen: a drawing area that follows the pen's pressure; a pen it had not seen rebuilds the page.
fn test_dialog(p: &Page, vidpid: String, known: Vec<String>) {
    type Stroke = Vec<(f64, f64, f64)>;
    let strokes: Rc<RefCell<Vec<Stroke>>> = Rc::new(RefCell::new(Vec::new()));
    let area = gtk::DrawingArea::new();
    area.set_content_width(480);
    area.set_content_height(260);
    area.add_css_class("st-pen-canvas");
    area.set_widget_name("st-pen-test-canvas");
    {
        let strokes = strokes.clone();
        area.set_draw_func(move |a, cr, _, _| {
            let color = kit::chart::resolve("accent");
            cr.set_source_rgba(color.0, color.1, color.2, color.3);
            cr.set_line_cap(gtk::cairo::LineCap::Round);
            for stroke in strokes.borrow().iter() {
                for pair in stroke.windows(2) {
                    let ((x0, y0, p0), (x1, y1, p1)) = (pair[0], pair[1]);
                    cr.set_line_width(1.0 + 7.0 * (p0 + p1) / 2.0);
                    cr.move_to(x0, y0);
                    cr.line_to(x1, y1);
                    cr.stroke().ok();
                }
            }
            let _ = a;
        });
    }
    let seen: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    let stylus = gtk::GestureStylus::new();
    let pressure = |g: &gtk::GestureStylus| g.axis(gdk::AxisUse::Pressure).unwrap_or(0.5).clamp(0.0, 1.0);
    {
        let (strokes, area2, seen) = (strokes.clone(), area.clone(), seen.clone());
        stylus.connect_down(move |g, x, y| {
            strokes.borrow_mut().push(vec![(x, y, pressure(g))]);
            if let Some(tool) = g.device_tool().filter(is_pen) {
                let id = stylus_id(tool.serial(), &vidpid);
                if !known.contains(&id) {
                    *seen.borrow_mut() = Some(id);
                }
            }
            area2.queue_draw();
        });
    }
    {
        let (strokes, area2) = (strokes.clone(), area.clone());
        stylus.connect_motion(move |g, x, y| {
            if let Some(last) = strokes.borrow_mut().last_mut() {
                last.push((x, y, pressure(g)));
            }
            area2.queue_draw();
        });
    }
    area.add_controller(stylus);
    let clear = c::button(t("Wissen", "Clear"), Variant::Standard);
    clear.set_widget_name("st-pen-test-clear");
    {
        let (strokes, area2) = (strokes.clone(), area.clone());
        clear.connect_clicked(move |_| {
            strokes.borrow_mut().clear();
            area2.queue_draw();
        });
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&c::label(
        t("Teken met de pen. Hoe harder je drukt, hoe dikker de lijn.", "Draw with the pen. The harder you press, the thicker the line."),
        "w11-text-2",
        true,
    ));
    body.append(&area);
    clear.set_halign(gtk::Align::Start);
    body.append(&clear);
    let pp = p.clone();
    Dialog::new(t("Instellingen testen", "Test settings")).body(&body).close_label(t("Sluiten", "Close")).width(540).show(&p.anchor(), move |_| {
        if let Some(id) = seen.borrow_mut().take() {
            CHOSEN.with(|c| c.borrow_mut().1 = Some(id));
            pp.refresh();
        }
    });
}

// ------------------------------------------------------------------------------------------ the page

/// A card with a drop-down bound to `k`.
fn dropdown_card(gs: &Gs, k: &GKey, icon: &str, title: &str, description: &str, options: Vec<(Value, String)>, width: i32) -> Option<SettingsCard> {
    if !gs.available(k) {
        return None;
    }
    let dd = gs.dropdown(k, options);
    let card = SettingsCard::new(icon, title, description);
    card.set_control(&common::sized(dd.widget(), width));
    Some(card)
}

/// A pressure slider (Zacht - Stevig) bound to a curve key.
fn pressure_card(gs: &Gs, k: &GKey, title: &str) -> Option<SettingsCard> {
    if !gs.available(k) {
        return None;
    }
    let slider = gs.slider(k, 0.0, 6.0, 1.0, curve_ui, curve_gs);
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&c::label(t("Zacht", "Soft"), "w11-text-2", false));
    slider.widget().set_size_request(200, -1);
    row.append(slider.widget());
    row.append(&c::label(t("Stevig", "Firm"), "w11-text-2", false));
    let card = SettingsCard::new("", title, "");
    card.set_control(&row);
    Some(card)
}

/// One pen button: its action and, for Toetsaanslag verzenden, the key it sends.
fn button_card(gs: &Gs, action: &GKey, binding: &GKey, title: &str, description: &str) -> Option<SettingsCard> {
    if !gs.available(action) {
        return None;
    }
    let card = SettingsCard::new("", title, description);
    let dd = gs.dropdown(action, action_options());
    let assign = c::button(t("Toetsaanslag toewijzen", "Assign keystroke"), Variant::Standard);
    assign.set_widget_name(&format!("st-pen-assign-{}", binding.key));
    let show = {
        let (gs, action, binding, card, assign, description) = (gs.clone(), action.clone(), binding.clone(), card.clone(), assign.clone(), description.to_owned());
        Rc::new(move || {
            let keystroke = gs.value(&action) == "keybinding";
            assign.set_visible(keystroke && gs.available(&binding));
            assign.set_sensitive(gs.writable(&binding));
            let label = accel_label(common::s(&gs.value(&binding)));
            card.set_description(&if keystroke {
                if label.is_empty() {
                    t("Nog geen toetsaanslag toegewezen", "No keystroke assigned yet").to_owned()
                } else {
                    common::fill(t("Verzendt {keys}", "Sends {keys}"), &[("keys", &label)])
                }
            } else {
                description.clone()
            });
        })
    };
    show();
    for k in [action, binding] {
        let show = show.clone();
        gs.follow(k, move |_| show());
    }
    {
        let show = show.clone();
        dd.connect_changed(move |_| show());
    }
    {
        let (gs, binding, title) = (gs.clone(), binding.clone(), title.to_owned());
        assign.connect_clicked(move |_| {
            let (gs2, binding2) = (gs.clone(), binding.clone());
            KeyCapture::run(gs.page(), &title, move |capture| match capture {
                Capture::Accel(accel) => gs2.set(&binding2, json!(accel), || {}),
                Capture::Disabled => gs2.set(&binding2, json!(""), || {}),
                Capture::Cancelled => {}
            });
        });
    }
    card.set_control(&common::sized(dd.widget(), 220));
    card.add_control(&assign);
    Some(card)
}

/// Tabletmodus: 'absolute' (on) or 'relative'.
fn mapping_on(v: &Value) -> bool {
    v.as_str() != Some("relative")
}

fn mapping_value(on: bool) -> Value {
    json!(if on { "absolute" } else { "relative" })
}

/// The eraser check box: 'default' (ticked: the eraser erases) or 'button'.
fn eraser_on(v: &Value) -> bool {
    v.as_str() != Some("button")
}

fn eraser_value(on: bool) -> Value {
    json!(if on { "default" } else { "button" })
}

fn pen(p: Page) -> PageFuture {
    Box::pin(async move {
        let found = p.get("input.tablets", json!({})).await.unwrap_or(Value::Null);
        if !p.alive() {
            return Content::new();
        }
        // a tablet plugged in or out
        {
            let (pp, before) = (p.clone(), common::caps());
            p.on("caps.changed", move |data| {
                if data["caps"].is_object()
                    && (data["caps"]["tablet"] != before["tablet"] || data["caps"]["touchscreen"] != before["touchscreen"])
                {
                    pp.refresh();
                }
            });
        }
        let tablets: Vec<Value> = found["tablets"].as_array().cloned().unwrap_or_default();
        let touchscreens: Vec<Value> = found["touchscreens"].as_array().cloned().unwrap_or_default();
        let monitors: Vec<Value> = found["monitors"].as_array().cloned().unwrap_or_default();
        let chosen = CHOSEN.with(|c| c.borrow().clone());
        let tablet = chosen
            .0
            .as_deref()
            .and_then(|id| tablets.iter().find(|t| common::s(&t["vidpid"]) == id))
            .or_else(|| tablets.iter().find(|t| t["pen"] == true))
            .or(tablets.first())
            .cloned();
        let vidpid = tablet.as_ref().map(|t| common::s(&t["vidpid"]).to_owned()).unwrap_or_default();
        let pens = tablet.as_ref().filter(|t| t["pen"] == true).map(styli).unwrap_or_default();
        let stylus = chosen.1.filter(|s| pens.contains(s)).or_else(|| pens.last().cloned());

        // the keys of the chosen tablet, its pen and every touchscreen
        let mut keys: Vec<GKey> = Vec::new();
        let tk = |key: &'static str| gs::at(TABLET, &tablet_path(&vidpid), key);
        let (left, mapping, output, aspect, area) = (tk("left-handed"), tk("mapping"), tk("output"), tk("keep-aspect"), tk("area"));
        if tablet.is_some() {
            keys.extend([left.clone(), mapping.clone(), output.clone(), aspect.clone(), area.clone()]);
        }
        let sk = |key: &'static str| gs::at(STYLUS, &stylus_path(stylus.as_deref().unwrap_or("")), key);
        let buttons = [
            (sk("button-action"), sk("button-keybinding"), t("Knop 1", "Button 1"), t("De onderste knop op de pen", "The lower button on the pen")),
            (sk("secondary-button-action"), sk("secondary-button-keybinding"), t("Knop 2", "Button 2"), t("De bovenste knop op de pen", "The upper button on the pen")),
            (sk("tertiary-button-action"), sk("tertiary-button-keybinding"), t("Knop 3", "Button 3"), t("Alleen op pennen met drie knoppen", "Only on pens with three buttons")),
        ];
        let (curve, eraser_curve, eraser_mode) = (sk("pressure-curve"), sk("eraser-pressure-curve"), sk("eraser-button-mode"));
        if stylus.is_some() {
            for (a, b, _, _) in &buttons {
                keys.push(a.clone());
                keys.push(b.clone());
            }
            keys.extend([curve.clone(), eraser_curve.clone(), eraser_mode.clone()]);
        }
        let touch_keys: Vec<(GKey, String)> = touchscreens
            .iter()
            .map(|ts| (gs::at(TOUCHSCREEN, &touchscreen_path(common::s(&ts["vidpid"])), "output"), common::s(&ts["name"]).to_owned()))
            .collect();
        keys.extend(touch_keys.iter().map(|(k, _)| k.clone()));
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        let Some(tablet) = tablet else {
            let card = SettingsCard::new("pen", t("Geen tablet gevonden", "No tablet found"), t("Sluit een tekentablet of pen aan om de instellingen ervan te zien.", "Connect a drawing tablet or pen to see its settings."));
            card.widget().set_widget_name("st-pen-none");
            let g = SettingsGroup::new("", "");
            g.add(card.widget());
            out.push(g.widget().clone().upcast());
            out.extend(touchscreen_group(&gs, &touch_keys, &monitors));
            return out;
        };
        let kind = common::s(&tablet["kind"]).to_owned();

        // the tablet, and the choice of one when there are more
        let head = SettingsGroup::new("", "");
        let card = SettingsCard::new("pen", common::s(&tablet["name"]), kind_text(&kind));
        card.widget().set_widget_name("st-pen-tablet");
        if tablets.len() > 1 {
            let options: Vec<DropdownOption> = tablets.iter().map(|t| DropdownOption::new(common::s(&t["vidpid"]), common::s(&t["name"]))).collect();
            let dd = Dropdown::new(options, Some(vidpid.as_str()));
            dd.widget().set_widget_name("st-pen-choose-tablet");
            let pp = p.clone();
            dd.connect_changed(move |id| {
                CHOSEN.with(|c| *c.borrow_mut() = (Some(id.to_owned()), None));
                pp.refresh();
            });
            card.set_control(&common::sized(dd.widget(), 240));
        }
        head.add(card.widget());
        let pen = tablet["pen"] == true;
        if pen
            && tablet["reversible"] != false
            && let Some(c) = dropdown_card(
                &gs,
                &left,
                "",
                t("Kies met welke hand je schrijft", "Choose which hand you write with"),
                "",
                vec![(json!(false), t("Rechterhand", "Right hand").into()), (json!(true), t("Linkerhand", "Left hand").into())],
                160,
            )
        {
            head.add(c.widget());
        }
        out.push(head.widget().clone().upcast());

        // GNOME's tablet rows
        let group = SettingsGroup::new(t("Tablet", "Tablet"), "");
        if pen && kind == "tablet" {
            if gs.available(&mapping) {
                let sw = super::mouse::mapped_toggle(&gs, &keys, &mapping, mapping_on, mapping_value);
                let c = SettingsCard::new(
                    "",
                    t("Tabletmodus", "Tablet mode"),
                    t("De pen wijst een vaste plek op het scherm aan; uit beweegt hij de aanwijzer zoals een muis", "The pen points at a fixed place on the screen; off, it moves the pointer like a mouse"),
                );
                c.set_control(sw.widget());
                group.add(c.widget());
            }
        }
        if pen && kind != "builtin" {
            let options = output_options(&monitors, &gs.value(&output));
            if let Some(c) = dropdown_card(&gs, &output, "display", t("Toewijzen aan beeldscherm", "Map to display"), t("Het beeldscherm waarop de pen werkt", "The display the pen works on"), options, 240) {
                group.add(c.widget());
            }
        }
        if pen && kind == "tablet" {
            if let Some(c) = gs.toggle_card(&aspect, "", t("Hoogte-breedteverhouding behouden", "Keep aspect ratio"), t("Het tekengebied krijgt de verhouding van het beeldscherm", "The drawing area takes on the proportions of the display"), false) {
                group.add(c.widget());
            }
        }
        if pen && kind != "tablet" && gs.available(&area) {
            let c = SettingsCard::new("", t("Kalibreren", "Calibrate"), t("Tik op de doelen om de pen precies af te stellen", "Tap the targets to fine-tune the pen"));
            let b = c::button(t("Kalibreren", "Calibrate"), Variant::Standard);
            b.set_widget_name("st-pen-calibrate");
            b.set_sensitive(gs.writable(&area));
            let (gs2, area2, output2, monitors2) = (gs.clone(), area.clone(), output.clone(), monitors.clone());
            b.connect_clicked(move |b| calibrate(&gs2, &area2, &output2, monitors2.clone(), b));
            c.set_control(&b);
            group.add(c.widget());
        }
        if tablet["pad"] == true {
            let n = tablet["buttons"].as_i64().unwrap_or(0);
            let desc = if n > 0 {
                common::plural(n, t("Wat de knop op de tablet doet", "What the button on the tablet does"), t("Wat de {n} knoppen op de tablet doen", "What the {n} buttons on the tablet do"))
            } else {
                t("Wat de knoppen, ringen en strips op de tablet doen", "What the buttons, rings and strips on the tablet do").to_owned()
            };
            let c = SettingsCard::new("", t("Knoppen toewijzen", "Map buttons"), &desc);
            let b = c::button(t("Knoppen toewijzen", "Map buttons"), Variant::Standard);
            b.set_widget_name("st-pen-map-buttons");
            let (pp, id) = (p.clone(), vidpid.clone());
            b.connect_clicked(move |_| {
                let wait = pp.run_with("input.tablet.mapButtons", json!({ "vidpid": id.clone() }), None, true);
                pp.spawn(async move {
                    let _ = wait.await;
                });
            });
            c.set_control(&b);
            group.add(c.widget());
        }
        if group.items().first_child().is_some() {
            out.push(group.widget().clone().upcast());
        }

        // the pen
        if let Some(stylus) = stylus.as_deref() {
            let buttons_group = SettingsGroup::new(t("Penknoppen", "Pen buttons"), "");
            if pens.len() > 1 {
                let options: Vec<DropdownOption> = pens.iter().map(|id| DropdownOption::new(id, &stylus_label(id))).collect();
                let dd = Dropdown::new(options, Some(stylus));
                dd.widget().set_widget_name("st-pen-choose-pen");
                let pp = p.clone();
                dd.connect_changed(move |id| {
                    CHOSEN.with(|c| c.borrow_mut().1 = Some(id.to_owned()));
                    pp.refresh();
                });
                let c = SettingsCard::new("pen", t("Pen", "Pen"), &stylus_label(stylus));
                c.set_control(&common::sized(dd.widget(), 240));
                buttons_group.add(c.widget());
            }
            for (action, binding, title, description) in &buttons {
                if let Some(c) = button_card(&gs, action, binding, title, description) {
                    buttons_group.add(c.widget());
                }
            }
            if gs.available(&eraser_mode) {
                let sw = super::mouse::mapped_toggle(&gs, &keys, &eraser_mode, eraser_on, eraser_value);
                let c = SettingsCard::new(
                    "",
                    t("De bovenste knop van mijn pen gebruiken om inkt te wissen (indien beschikbaar)", "Use the top button of my pen to erase ink (where available)"),
                    "",
                );
                c.set_control(sw.widget());
                buttons_group.add(c.widget());
            }
            out.push(buttons_group.widget().clone().upcast());

            let pressure = SettingsGroup::new(t("Pendruk", "Pen pressure"), "");
            if let Some(c) = pressure_card(&gs, &curve, t("Gevoel van pendruk", "Tip pressure feel")) {
                pressure.add(c.widget());
            }
            if let Some(c) = pressure_card(&gs, &eraser_curve, t("Gevoel van gumdruk", "Eraser pressure feel")) {
                pressure.add(c.widget());
            }
            out.push(pressure.widget().clone().upcast());
        }
        // a pen that comes in later (GNOME Shell announces each tool once it is used)
        if pen && let Some(seat) = seat() {
            let (pp, id, known) = (p.clone(), vidpid.clone(), pens.clone());
            let handler = seat.connect_tool_added(move |_, tool| {
                if is_pen(tool) && !known.contains(&stylus_id(tool.serial(), &id)) {
                    pp.refresh();
                }
            });
            p.on_cleanup(move || seat.disconnect(handler));
        }

        if pen {
            let test = SettingsGroup::new("", "");
            let c = SettingsCard::new("brush", t("Instellingen testen", "Test settings"), t("Tekenen met de pen en de druk uitproberen", "Draw with the pen and try the pressure"));
            let b = c::button(t("Testen", "Test"), Variant::Standard);
            b.set_widget_name("st-pen-test");
            let (pp, id, known) = (p.clone(), vidpid.clone(), pens.clone());
            b.connect_clicked(move |_| test_dialog(&pp, id.clone(), known.clone()));
            c.set_control(&b);
            test.add(c.widget());
            out.push(test.widget().clone().upcast());
        }

        out.extend(touchscreen_group(&gs, &touch_keys, &monitors));
        let related = SettingsGroup::new(ct::related(), "");
        related.add(common::link_card(&p, "display").widget());
        related.add(common::link_card(&p, "mouse").widget());
        out.push(related.widget().clone().upcast());
        out
    })
}

/// Aanraakscherm toewijzen aan beeldscherm (amendment 8), one row per touchscreen (also on Muisaanwijzer en aanraken,
/// a11y_vision.rs).
pub(super) fn touchscreen_group(gs: &Gs, touch_keys: &[(GKey, String)], monitors: &[Value]) -> Option<gtk::Widget> {
    let group = SettingsGroup::new(t("Aanraakscherm", "Touchscreen"), "");
    for (k, name) in touch_keys {
        let options = output_options(monitors, &gs.value(k));
        let description = if touch_keys.len() > 1 { name.as_str() } else { "" };
        if let Some(c) = dropdown_card(gs, k, "display", t("Aanraakscherm toewijzen aan beeldscherm", "Map touchscreen to display"), description, options, 240) {
            group.add(c.widget());
        }
    }
    group.items().first_child().is_some().then(|| group.widget().clone().upcast())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pens_without_a_serial_get_mutters_default_path() {
        assert_eq!(stylus_id(0, "056a:0357"), "default-056a:0357");
        assert_eq!(stylus_id(0x8a01c2f3, "056a:0357"), "8a01c2f3");
        assert_eq!(stylus_path("default-056a:0357"), "/org/gnome/desktop/peripherals/stylus/default-056a:0357/");
        assert_eq!(tablet_path("056a:0357"), "/org/gnome/desktop/peripherals/tablets/056a:0357/");
        assert_eq!(touchscreen_path("04f3:2072"), "/org/gnome/desktop/peripherals/touchscreens/04f3:2072/");
        assert!(stylus_label("default-056a:0357") != stylus_label("8a01c2f3"));
    }

    #[test]
    fn pressure_feel_takes_gnomes_presets() {
        assert_eq!(curve_ui(&json!([0, 0, 100, 100])), 3.0, "linear is the middle");
        assert_eq!(curve_ui(&json!([0, 75, 25, 100])), 0.0);
        assert_eq!(curve_ui(&json!([75, 0, 100, 25])), 6.0);
        assert_eq!(curve_ui(&json!([20, 0, 100, 80])), 4.0, "the nearest preset");
        assert_eq!(curve_ui(&Value::Null), 3.0);
        assert_eq!(curve_ui(&json!([1, 2])), 3.0);
        for n in 0..7 {
            assert_eq!(curve_ui(&curve_gs(n as f64)), n as f64);
        }
        assert_eq!(curve_gs(9.0), json!([75, 0, 100, 25]), "clamped");
    }

    #[test]
    fn calibration_turns_four_taps_into_mutters_padding() {
        // a pen that is exact: no padding
        assert_eq!(calibration_area(&TARGETS), Some([0.0, 0.0, 0.0, 0.0]));
        // every tap 2 % right and 1 % low: the active area moves the same way
        let shifted = TARGETS.map(|(x, y)| (x + 0.02, y + 0.01));
        assert_eq!(calibration_area(&shifted), Some([0.02, -0.02, 0.01, -0.01]));
        // the taps 10 % closer together than the targets: a larger active area
        let squeezed = TARGETS.map(|(x, y)| (0.5 + (x - 0.5) * 0.9, 0.5 + (y - 0.5) * 0.9));
        let area = calibration_area(&squeezed).unwrap();
        assert!(area.iter().all(|p| (p - 0.05).abs() < 1e-3), "{area:?}");
        // a misclick: two taps on one spot
        assert_eq!(calibration_area(&[(0.5, 0.5), (0.52, 0.5), (0.5, 0.9), (0.52, 0.9)]), None);
        assert_eq!(calibration_area(&[(0.6, 0.6), (0.95, 0.6), (0.6, 0.95), (0.95, 0.95)]), None, "out of range");
    }

    #[test]
    fn output_choices_are_automatic_every_monitor_and_a_lost_one() {
        let monitors = vec![
            json!({"connector": "eDP-1", "vendor": "BOE", "product": "0x0bca", "serial": "0x00000000", "name": "Ingebouwd beeldscherm", "builtin": true}),
            json!({"connector": "DP-2", "vendor": "DEL", "product": "DELL U2720Q", "serial": "ABC123", "name": "", "builtin": false}),
        ];
        let options = output_options(&monitors, &json!(["", "", ""]));
        assert_eq!(options.len(), 3);
        assert_eq!(options[0].0, json!(["", "", ""]));
        assert_eq!(options[1], (json!(["BOE", "0x0bca", "0x00000000"]), "Ingebouwd beeldscherm".to_owned()));
        assert_eq!(options[2].1, "DEL DELL U2720Q", "no display name: vendor and product");
        let lost = output_options(&monitors, &json!(["GSM", "LG", "1"]));
        assert_eq!(lost.len(), 4);
        assert_eq!(lost[3].0, json!(["GSM", "LG", "1"]));
        assert_eq!(output_options(&[], &Value::Null).len(), 1);
    }

    #[test]
    fn pen_button_choices_are_gnomes_actions() {
        let values: Vec<Value> = action_options().into_iter().map(|(v, _)| v).collect();
        assert_eq!(values, ["default", "middle", "right", "back", "forward", "switch-monitor", "keybinding"].map(|s| json!(s)));
        assert!(eraser_on(&json!("default")) && !eraser_on(&json!("button")));
        assert!(mapping_on(&json!("absolute")) && !mapping_on(&json!("relative")));
        assert_eq!((mapping_value(true), mapping_value(false)), (json!("absolute"), json!("relative")));
        assert_eq!((eraser_value(true), eraser_value(false)), (json!("default"), json!("button")));
        assert_eq!(accel_label(""), "");
        assert_ne!(kind_text("display"), kind_text("tablet"));
    }

    #[test]
    fn two_tablets_get_control_names_of_their_own() {
        let a = gs::at(TABLET, &tablet_path("056a:0357"), "left-handed");
        let b = gs::at(TOUCHSCREEN, &touchscreen_path("04f3:2072"), "output");
        let c = gs::at(TABLET, &tablet_path("056a:0357"), "output");
        let keys = [a.clone(), b.clone(), c.clone()];
        assert_eq!(gs::control_name("pen", &keys, &a), "st-pen-left-handed");
        assert_eq!(gs::control_name("pen", &keys, &b), "st-pen-touchscreen-output");
        assert_eq!(gs::control_name("pen", &keys, &c), "st-pen-tablet-output");
    }
}
