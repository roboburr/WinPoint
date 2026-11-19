//! Instellingen › Bluetooth en apparaten › Muis (batch A, notes/claude/settings-parity.md §2, plan.json "mouse"): every
//! mouse function of GNOME Settings 50's Mouse & Touchpad panel, in the order and wording of Windows 11's
//! Bluetooth & devices › Mouse.
//!
//! * Primaire muisknop (Links / Rechts) = `org.gnome.desktop.peripherals.mouse left-handed`; touchpads follow it
//!   (their own `left-handed` is 'mouse', GNOME's "Primary Button" for mice and touchpads).
//! * Snelheid van muisaanwijzer, Windows' 1-20 with 10 in the middle = `speed` -1..1, (n - 10) / 10.
//! * Nauwkeurigheid van aanwijzer verbeteren = `accel-profile`: on writes 'default', off 'flat' (GNOME's Mouse
//!   Acceleration; 'adaptive' reads as on).
//! * WinPoint addition: Plakken met de middelste muisknop = `org.gnome.desktop.interface gtk-enable-primary-paste`.
//! * Schuiven: Inactieve vensters schuiven (GNOME always scrolls the window under the pointer: shown on, fixed),
//!   Schuifrichting = `natural-scroll` (Neerwaartse beweging schuift omhoog = natural).
//! * Aanwijsstick (WinPoint addition while `caps.pointingstick`): its speed and acceleration
//!   (`org.gnome.desktop.peripherals.pointingstick`).
//! * Instellingen testen: GNOME's Test Settings dialog (clicks, the double click at the current double-click time,
//!   scrolling).
//! * Gerelateerde instellingen: Aanvullende muisinstellingen opens Windows' Eigenschappen voor Muis (Knoppen: Primaire
//!   en secundaire knop omwisselen, Dubbelkliksnelheid with its test folder = `double-click` 100-1000 ms, Middelste
//!   klik nabootsen = `middle-click-emulation`; Opties voor aanwijzer: the speed, the precision and Locatie van
//!   aanwijzer weergeven wanneer ik op de Ctrl-toets druk = `locate-pointer`); Muisaanwijzer opens the
//!   accessibility page.
//!
//! The mapped controls (a key whose value is no plain bool), the scroll-direction choices and the test dialog are
//! shared with touchpad.rs and pen.rs (`pub(super)`).
use super::{Builder, Content, Page, PageFuture};
use crate::common::{self, ct};
use crate::gs::{self, GKey, Gs};
use kit::controls::{self as c, Chevron, Dialog, Pivot, SettingsCard, SettingsGroup, Slider, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::Cell;
use std::rc::Rc;

pub const PAGES: &[(&str, Builder)] = &[("mouse", mouse)];
pub const CSS: &str = r#"
.st-input-test-area {
  min-height: 96px;
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
}
.st-input-chip {
  padding: 4px 10px;
  border: 1px solid var(--stroke-card);
  border-radius: 12px;
  color: var(--text-2);
}
.st-input-chip.st-done { color: var(--text-1); border-color: var(--accent-default, currentColor); }
.st-input-folder { padding: 8px; border-radius: var(--radius-card); }
.st-input-dialog-section { margin-top: 8px; }
"#;

const MOUSE: &str = "org.gnome.desktop.peripherals.mouse";
const STICK: &str = "org.gnome.desktop.peripherals.pointingstick";
const IFACE: &str = "org.gnome.desktop.interface";
const LEFT: GKey = gs::key(MOUSE, "left-handed");
const SPEED: GKey = gs::key(MOUSE, "speed");
const ACCEL: GKey = gs::key(MOUSE, "accel-profile");
const NATURAL: GKey = gs::key(MOUSE, "natural-scroll");
const MIDDLE: GKey = gs::key(MOUSE, "middle-click-emulation");
/// GNOME's double-click time (ms); the touchpad page's test dialog reads it too.
pub(super) const DOUBLE: GKey = gs::key(MOUSE, "double-click");
const LOCATE: GKey = gs::key(IFACE, "locate-pointer");
const PASTE: GKey = gs::key(IFACE, "gtk-enable-primary-paste");
const STICK_SPEED: GKey = gs::key(STICK, "speed");
const STICK_ACCEL: GKey = gs::key(STICK, "accel-profile");

fn keys() -> Vec<GKey> {
    vec![LEFT, SPEED, ACCEL, NATURAL, MIDDLE, DOUBLE, LOCATE, PASTE, STICK_SPEED, STICK_ACCEL]
}

// ------------------------------------------------------------------------------------------ value mappings

/// Windows' mouse pointer speed, 1-20 with GNOME's 0 at 10, from GNOME's speed (-1..1).
pub(super) fn speed20_ui(v: &Value) -> f64 {
    (v.as_f64().unwrap_or(0.0) * 10.0 + 10.0).round().clamp(1.0, 20.0)
}

/// GNOME's speed for Windows' 1-20: (n - 10) / 10.
pub(super) fn speed20_gs(n: f64) -> Value {
    json!(((n.round() - 10.0) / 10.0).clamp(-1.0, 1.0))
}

/// Acceleration (Nauwkeurigheid van aanwijzer verbeteren) is on unless the profile is 'flat'.
pub(super) fn accel_on(v: &Value) -> bool {
    v.as_str() != Some("flat")
}

/// GNOME Settings' write for the acceleration switch: 'default' or 'flat'.
pub(super) fn accel_value(on: bool) -> Value {
    json!(if on { "default" } else { "flat" })
}

/// Dubbelkliksnelheid, 1 (slow: 1000 ms) to 10 (fast: 100 ms), from GNOME's double-click time.
pub(super) fn double_ui(v: &Value) -> f64 {
    ((1100.0 - v.as_f64().unwrap_or(400.0)) / 100.0).round().clamp(1.0, 10.0)
}

/// GNOME's double-click time for a Dubbelkliksnelheid step.
pub(super) fn double_gs(n: f64) -> Value {
    json!(1100 - (n.round() as i64).clamp(1, 10) * 100)
}

/// Whether a press at `now_us` after one at `prev_us` is a double click at GNOME's double-click time `ms`.
pub(super) fn is_double(prev_us: Option<i64>, now_us: i64, ms: i64) -> bool {
    prev_us.is_some_and(|prev| now_us >= prev && now_us - prev <= ms.max(1) * 1000)
}

/// Schuifrichting's choices for natural-scroll (Windows' wording; natural = down motion scrolls up).
pub(super) fn scroll_options() -> Vec<(Value, String)> {
    vec![
        (json!(false), t("Neerwaartse beweging schuift omlaag", "Down motion scrolls down").into()),
        (json!(true), t("Neerwaartse beweging schuift omhoog", "Down motion scrolls up").into()),
    ]
}

// ------------------------------------------------------------------------------------------ mapped controls

/// The harness name of a control bound to `k` and its lock state (gs.rs does both for its own controls).
pub(super) fn bind_state(gs: &Gs, keys: &[GKey], w: &impl IsA<gtk::Widget>, k: &GKey) {
    w.as_ref().set_widget_name(&gs::control_name(gs.page().id(), keys, k));
    let writable = gs.writable(k);
    w.as_ref().set_sensitive(writable);
    if !writable && gs.available(k) {
        w.as_ref().set_tooltip_text(Some(gs::locked_text()));
    }
}

/// A switch for a key whose value maps to on and off (`on`), writing `write(on)`: optimistic, reverted with the
/// error toast, following changes.
pub(super) fn mapped_toggle(gs: &Gs, keys: &[GKey], k: &GKey, on: fn(&Value) -> bool, write: fn(bool) -> Value) -> ToggleSwitch {
    let sw = ToggleSwitch::new(on(&gs.value(k)));
    bind_state(gs, keys, sw.switch(), k);
    if !gs.writable(k) {
        sw.set_sensitive(false);
    }
    let (g, k2, sw2) = (gs.clone(), k.clone(), sw.clone());
    sw.connect_toggled(move |v| {
        let (sw3, g3, k3) = (sw2.clone(), g.clone(), k2.clone());
        g.set(&k2, write(v), move || sw3.set_active(on(&g3.value(&k3))));
    });
    let sw2 = sw.clone();
    gs.follow(k, move |v| sw2.set_active(on(v)));
    sw
}

/// A check box for a key whose value maps to checked and unchecked (`on`), writing `write(checked)`.
pub(super) fn mapped_checkbox(gs: &Gs, keys: &[GKey], k: &GKey, label: &str, on: fn(&Value) -> bool, write: fn(bool) -> Value) -> gtk::CheckButton {
    let cb = c::checkbox(label);
    cb.set_active(on(&gs.value(k)));
    bind_state(gs, keys, &cb, k);
    let silent = Rc::new(Cell::new(false));
    let (g, k2, s2) = (gs.clone(), k.clone(), silent.clone());
    cb.connect_toggled(move |b| {
        if s2.get() {
            return;
        }
        let (b2, s3, g3, k3) = (b.clone(), s2.clone(), g.clone(), k2.clone());
        g.set(&k2, write(b.is_active()), move || {
            s3.set(true);
            b2.set_active(on(&g3.value(&k3)));
            s3.set(false);
        });
    });
    let cb2 = cb.clone();
    gs.follow(k, move |v| {
        silent.set(true);
        cb2.set_active(on(v));
        silent.set(false);
    });
    cb
}

/// A row of "Langzaam" slider "Snel" (Windows' classic dialogs).
fn slow_fast(slider: &Slider) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.append(&c::label(t("Langzaam", "Slow"), "w11-text-2", false));
    slider.widget().set_hexpand(true);
    row.append(slider.widget());
    row.append(&c::label(t("Snel", "Fast"), "w11-text-2", false));
    row
}

/// A section title inside a dialog.
fn dialog_section(text: &str) -> gtk::Label {
    let l = c::label(text, "w11-body-strong st-input-dialog-section", true);
    l.set_margin_top(8);
    l
}

/// A chip of the test dialog that gets its check mark once the action was seen.
fn chip(name: &str, text: &str) -> (gtk::Box, Rc<dyn Fn()>) {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    b.add_css_class("st-input-chip");
    b.set_widget_name(name);
    let mark = kit::icons::glyph("check", 12);
    mark.set_visible(false);
    b.append(&mark);
    b.append(&gtk::Label::new(Some(text)));
    let weak = b.downgrade();
    let done: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(b) = weak.upgrade() {
            b.add_css_class("st-done");
            mark.set_visible(true);
        }
    });
    (b, done)
}

/// GNOME's Test Settings (Mouse & Touchpad): a click area (primary, secondary and middle click, the double click
/// at the double-click time `gs` holds now) and a list to scroll.
pub(super) fn test_dialog(gs: &Gs) {
    let p = gs.page().clone();
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(&c::label(
        t(
            "Klik, dubbelklik en schuif in de vakken hieronder. Een vinkje laat zien wat werkt.",
            "Click, double-click and scroll in the boxes below. A tick shows what works.",
        ),
        "w11-text-2",
        true,
    ));
    let chips = crate::widgets::Wrap::new(8, 8);
    let (primary, mark_primary) = chip("st-input-test-primary", t("Primaire klik", "Primary click"));
    let (secondary, mark_secondary) = chip("st-input-test-secondary", t("Secundaire klik", "Secondary click"));
    let (middle, mark_middle) = chip("st-input-test-middle", t("Middelste klik", "Middle click"));
    let (double, mark_double) = chip("st-input-test-double", t("Dubbelklik", "Double-click"));
    let (scroll, mark_scroll) = chip("st-input-test-scroll", t("Schuiven", "Scrolling"));
    for w in [&primary, &secondary, &middle, &double, &scroll] {
        chips.append(w);
    }
    body.append(&chips);

    let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
    area.add_css_class("st-input-test-area");
    area.set_widget_name("st-input-test-area");
    let hint = c::label(t("Klik of dubbelklik hier", "Click or double-click here"), "w11-text-2", false);
    hint.set_halign(gtk::Align::Center);
    hint.set_valign(gtk::Align::Center);
    hint.set_vexpand(true);
    area.append(&hint);
    let last = Rc::new(Cell::new(None::<i64>));
    let click = gtk::GestureClick::new();
    click.set_button(0);
    {
        let gs = gs.clone();
        click.connect_pressed(move |g, _, _, _| {
            let now = glib::monotonic_time();
            match g.current_button() {
                1 => {
                    mark_primary();
                    let ms = gs.value(&DOUBLE).as_i64().unwrap_or(400);
                    if is_double(last.get(), now, ms) {
                        mark_double();
                        last.set(None);
                    } else {
                        last.set(Some(now));
                    }
                }
                2 => mark_middle(),
                3 => mark_secondary(),
                _ => {}
            }
        });
    }
    area.add_controller(click);
    body.append(&area);

    let lines = gtk::Box::new(gtk::Orientation::Vertical, 6);
    lines.set_margin_top(10);
    lines.set_margin_bottom(10);
    lines.set_margin_start(14);
    lines.set_margin_end(14);
    for n in 1..=40 {
        lines.append(&c::label(&common::fill(t("Schuif door deze lijst: regel {n}", "Scroll through this list: line {n}"), &[("n", &n.to_string())]), "", false));
    }
    let list = gtk::ScrolledWindow::new();
    list.set_hscrollbar_policy(gtk::PolicyType::Never);
    list.set_min_content_height(140);
    list.set_max_content_height(140);
    list.add_css_class("st-input-test-area");
    list.set_widget_name("st-input-test-list");
    list.set_child(Some(&lines));
    let wheel = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    wheel.set_propagation_phase(gtk::PropagationPhase::Capture);
    wheel.connect_scroll(move |_, _, _| {
        mark_scroll();
        glib::Propagation::Proceed
    });
    list.add_controller(wheel);
    body.append(&list);
    Dialog::new(t("Instellingen testen", "Test settings")).body(&body).close_label(t("Sluiten", "Close")).width(540).show(&p.anchor(), |_| {});
}

/// Windows' Eigenschappen voor Muis (Aanvullende muisinstellingen), with what GNOME has of it: Knoppen and Opties voor
/// aanwijzer. Its controls are the page's keys; they follow every change, also one made on the page.
fn properties_dialog(gs: &Gs, keys: &[GKey]) {
    let p = gs.page().clone();
    let rename = |w: &gtk::Widget, key: &str| w.set_widget_name(&format!("st-mouse-props-{key}"));

    // Knoppen
    let buttons = gtk::Box::new(gtk::Orientation::Vertical, 8);
    buttons.append(&dialog_section(t("Knopconfiguratie", "Button configuration")));
    if gs.available(&LEFT) {
        let swap = gs.checkbox(&LEFT, t("Primaire en secundaire knop omwisselen", "Switch primary and secondary buttons"), false);
        rename(swap.upcast_ref(), "left-handed");
        buttons.append(&swap);
        buttons.append(&c::label(
            t(
                "Schakel dit selectievakje in als je de rechterknop wilt gebruiken voor primaire functies, zoals selecteren en slepen.",
                "Select this check box to make the button on the right the one you use for primary functions such as selecting and dragging.",
            ),
            "w11-caption w11-text-2",
            true,
        ));
    }
    if gs.available(&DOUBLE) {
        buttons.append(&dialog_section(t("Dubbelkliksnelheid", "Double-click speed")));
        buttons.append(&c::label(
            t(
                "Dubbelklik op de map om de instelling te testen. Als de map niet wordt geopend of gesloten, kies je een lagere snelheid.",
                "Double-click the folder to test your setting. If the folder doesn’t open or close, try using a slower setting.",
            ),
            "w11-caption w11-text-2",
            true,
        ));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let slider = gs.slider(&DOUBLE, 1.0, 10.0, 1.0, double_ui, double_gs);
        rename(slider.scale().upcast_ref(), "double-click");
        let speed = slow_fast(&slider);
        speed.set_hexpand(true);
        speed.set_valign(gtk::Align::Center);
        row.append(&speed);
        let folder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        folder.add_css_class("st-input-folder");
        folder.set_widget_name("st-mouse-props-folder");
        let glyph = kit::icons::glyph("folder", 40);
        folder.append(&glyph);
        let last = Rc::new(Cell::new(None::<i64>));
        let click = gtk::GestureClick::new();
        {
            let gs = gs.clone();
            click.connect_pressed(move |_, _, _, _| {
                let now = glib::monotonic_time();
                let ms = gs.value(&DOUBLE).as_i64().unwrap_or(400);
                if is_double(last.get(), now, ms) {
                    let open = kit::icons::glyph_name(&glyph).as_deref() == Some("folder-open");
                    kit::icons::set_glyph(&glyph, if open { "folder" } else { "folder-open" });
                    last.set(None);
                } else {
                    last.set(Some(now));
                }
            });
        }
        folder.add_controller(click);
        row.append(&folder);
        buttons.append(&row);
    }
    if gs.available(&MIDDLE) {
        buttons.append(&dialog_section(t("Middelste knop", "Middle button")));
        let middle = gs.checkbox(&MIDDLE, t("Middelste klik nabootsen (links en rechts tegelijk)", "Emulate middle click (left and right together)"), false);
        rename(middle.upcast_ref(), "middle-click-emulation");
        buttons.append(&middle);
    }

    // Opties voor aanwijzer
    let options = gtk::Box::new(gtk::Orientation::Vertical, 8);
    if gs.available(&SPEED) {
        options.append(&dialog_section(t("Beweging", "Motion")));
        options.append(&c::label(t("Selecteer een snelheid voor de aanwijzer:", "Select a pointer speed:"), "w11-caption w11-text-2", true));
        let slider = gs.slider(&SPEED, 1.0, 20.0, 1.0, speed20_ui, speed20_gs);
        rename(slider.scale().upcast_ref(), "speed");
        options.append(&slow_fast(&slider));
    }
    if gs.available(&ACCEL) {
        let precise = mapped_checkbox(gs, keys, &ACCEL, t("Nauwkeurigheid van aanwijzer verbeteren", "Enhance pointer precision"), accel_on, accel_value);
        rename(precise.upcast_ref(), "accel-profile");
        options.append(&precise);
    }
    if gs.available(&LOCATE) {
        options.append(&dialog_section(t("Zichtbaarheid", "Visibility")));
        let locate = gs.checkbox(&LOCATE, t("Locatie van aanwijzer weergeven wanneer ik op de Ctrl-toets druk", "Show location of pointer when I press the Ctrl key"), false);
        rename(locate.upcast_ref(), "locate-pointer");
        options.append(&locate);
    }

    let stack = gtk::Stack::new();
    stack.set_vhomogeneous(false);
    stack.set_hhomogeneous(true);
    stack.add_named(&buttons, Some("buttons"));
    stack.add_named(&options, Some("options"));
    let tabs = Pivot::new(&[("buttons", t("Knoppen", "Buttons"), ""), ("options", t("Opties voor aanwijzer", "Pointer Options"), "")], Some("buttons"));
    tabs.widget().set_widget_name("st-mouse-props-tabs");
    {
        let stack = stack.clone();
        tabs.connect_selected(move |id| stack.set_visible_child_name(id));
    }
    let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
    body.append(tabs.widget());
    body.append(&stack);
    Dialog::new(t("Eigenschappen voor Muis", "Mouse Properties")).body(&body).close_label(t("Sluiten", "Close")).width(560).show(&p.anchor(), |_| {});
}

/// The card of a slider bound to a speed key.
fn speed_card(gs: &Gs, k: &GKey, icon: &str, title: &str) -> Option<SettingsCard> {
    if !gs.available(k) {
        return None;
    }
    let slider = gs.slider(k, 1.0, 20.0, 1.0, speed20_ui, speed20_gs);
    slider.widget().set_size_request(240, -1);
    let card = SettingsCard::new(icon, title, "");
    card.set_control(slider.widget());
    Some(card)
}

fn mouse(p: Page) -> PageFuture {
    Box::pin(async move {
        let keys = keys();
        let gs = Gs::load(&p, &keys).await;
        if !p.alive() {
            return Content::new();
        }
        let mut out: Content = Vec::new();

        let main = SettingsGroup::new("", "");
        if gs.available(&LEFT) {
            let dd = gs.dropdown(&LEFT, vec![(json!(false), t("Links", "Left").into()), (json!(true), t("Rechts", "Right").into())]);
            let card = SettingsCard::new("mouse", t("Primaire muisknop", "Primary mouse button"), "");
            card.set_control(&common::sized(dd.widget(), 140));
            main.add(card.widget());
        }
        if let Some(card) = speed_card(&gs, &SPEED, "timer", t("Snelheid van muisaanwijzer", "Mouse pointer speed")) {
            main.add(card.widget());
        }
        if gs.available(&ACCEL) {
            let sw = mapped_toggle(&gs, &keys, &ACCEL, accel_on, accel_value);
            let card = SettingsCard::new("focus", t("Nauwkeurigheid van aanwijzer verbeteren", "Enhance pointer precision"), "");
            card.set_control(sw.widget());
            main.add(card.widget());
        }
        if let Some(card) = gs.toggle_card(
            &PASTE,
            "paste",
            t("Plakken met de middelste muisknop", "Middle-click paste"),
            t("Geselecteerde tekst plakken door op de middelste muisknop te klikken", "Paste selected text by clicking the middle mouse button"),
            false,
        ) {
            main.add(card.widget());
        }
        out.push(main.widget().clone().upcast());

        let scroll = SettingsGroup::new(t("Schuiven", "Scrolling"), "");
        let inactive = ToggleSwitch::new(true);
        inactive.set_sensitive(false);
        inactive.switch().set_widget_name("st-mouse-scroll-inactive");
        let inactive_card = SettingsCard::new(
            "window",
            t("Inactieve vensters schuiven wanneer ik de muisaanwijzer erop plaats", "Scroll inactive windows when hovering over them"),
            t("GNOME schuift altijd in het venster onder de muisaanwijzer", "GNOME always scrolls the window under the pointer"),
        );
        inactive_card.set_control(inactive.widget());
        scroll.add(inactive_card.widget());
        if gs.available(&NATURAL) {
            let dd = gs.dropdown(&NATURAL, scroll_options());
            let card = SettingsCard::new("sort", t("Schuifrichting", "Scrolling direction"), "");
            card.set_control(&common::sized(dd.widget(), 340));
            scroll.add(card.widget());
        }
        out.push(scroll.widget().clone().upcast());

        // the pointing stick comes and goes with caps (a ThinkPad's TrackPoint; a docked keyboard)
        let stick = common::caps()["pointingstick"] == true;
        if stick {
            let g = SettingsGroup::new(t("Aanwijsstick", "Pointing stick"), "");
            if let Some(card) = speed_card(&gs, &STICK_SPEED, "timer", t("Snelheid van aanwijsstick", "Pointing stick speed")) {
                g.add(card.widget());
            }
            if gs.available(&STICK_ACCEL) {
                let sw = mapped_toggle(&gs, &keys, &STICK_ACCEL, accel_on, accel_value);
                let card = SettingsCard::new("focus", t("Versnelling van aanwijsstick", "Pointing stick acceleration"), "");
                card.set_control(sw.widget());
                g.add(card.widget());
            }
            out.push(g.widget().clone().upcast());
        }
        {
            let pp = p.clone();
            p.on("caps.changed", move |d| {
                if (d["caps"]["pointingstick"] == true) != stick {
                    pp.refresh();
                }
            });
        }

        let test = SettingsGroup::new("", "");
        let test_card = SettingsCard::new("check", t("Instellingen testen", "Test settings"), t("Klikken, dubbelklikken en schuiven uitproberen", "Try clicking, double-clicking and scrolling"));
        let test_button = c::button(t("Testen", "Test"), Variant::Standard);
        test_button.set_widget_name("st-mouse-test");
        {
            let gs = gs.clone();
            test_button.connect_clicked(move |_| test_dialog(&gs));
        }
        test_card.set_control(&test_button);
        test.add(test_card.widget());
        out.push(test.widget().clone().upcast());

        let related = SettingsGroup::new(ct::related(), "");
        let more = SettingsCard::clickable(
            "mouse",
            t("Aanvullende muisinstellingen", "Additional mouse settings"),
            t("Knoppen, dubbelkliksnelheid en zichtbaarheid van aanwijzer", "Buttons, double-click speed and pointer visibility"),
            Chevron::External,
        );
        if let Some(b) = more.button() {
            b.set_widget_name("st-mouse-more");
        }
        {
            let (gs, keys) = (gs.clone(), keys.clone());
            more.connect_clicked(move || properties_dialog(&gs, &keys));
        }
        related.add(more.widget());
        related.add(
            common::link_card_with(&p, "mousepointer", Some(t("Muisaanwijzer", "Mouse pointer")), Some(t("Grootte en kleur van aanwijzer", "Pointer size and colour")), None)
                .widget(),
        );
        out.push(related.widget().clone().upcast());
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_pointer_speed_maps_onto_gnomes_speed() {
        // plan.json: speed = (n - 10) / 10; the harness' reference: slider position 15 is 0.5
        assert_eq!(speed20_gs(15.0), json!(0.5));
        assert_eq!(speed20_gs(10.0), json!(0.0));
        assert_eq!(speed20_gs(20.0), json!(1.0));
        assert_eq!(speed20_gs(1.0), json!(-0.9));
        assert_eq!(speed20_ui(&json!(0.5)), 15.0);
        assert_eq!(speed20_ui(&json!(-1.0)), 1.0, "GNOME's slowest is the slider's first step");
        assert_eq!(speed20_ui(&json!(0.0)), 10.0);
        assert_eq!(speed20_ui(&Value::Null), 10.0);
        for n in 1..=20 {
            assert_eq!(speed20_ui(&speed20_gs(n as f64)), n as f64, "{n} survives the round trip");
        }
    }

    #[test]
    fn precision_is_gnomes_acceleration_profile() {
        assert!(accel_on(&json!("default")) && accel_on(&json!("adaptive")));
        assert!(!accel_on(&json!("flat")));
        assert_eq!((accel_value(true), accel_value(false)), (json!("default"), json!("flat")));
    }

    #[test]
    fn double_click_speed_runs_from_slow_to_fast() {
        assert_eq!(double_gs(1.0), json!(1000));
        assert_eq!(double_gs(10.0), json!(100));
        assert_eq!(double_ui(&json!(400)), 7.0, "GNOME's default");
        assert_eq!(double_ui(&json!(5000)), 1.0);
        for n in 1..=10 {
            assert_eq!(double_ui(&double_gs(n as f64)), n as f64);
        }
        assert!(is_double(Some(1_000_000), 1_350_000, 400));
        assert!(!is_double(Some(1_000_000), 1_450_000, 400));
        assert!(!is_double(None, 1_000_000, 400), "a first press");
        assert!(!is_double(Some(2_000_000), 1_000_000, 400), "a clock that went back");
    }

    #[test]
    fn scroll_direction_follows_windows_wording() {
        let options = scroll_options();
        assert_eq!(options.len(), 2);
        assert_eq!(options[1].0, json!(true), "natural scrolling: down motion scrolls up");
    }

    #[test]
    fn every_bound_key_gets_a_control_name_of_its_own() {
        let keys = keys();
        let names: std::collections::HashSet<String> = keys.iter().map(|k| gs::control_name("mouse", &keys, k)).collect();
        assert_eq!(names.len(), keys.len());
        assert!(names.contains("st-mouse-left-handed"));
        assert!(names.contains("st-mouse-mouse-speed") && names.contains("st-mouse-pointingstick-speed"));
        // the harness (tests/gnome/scenarios/settings-input.py) clicks these by name
        assert!(names.contains("st-mouse-mouse-accel-profile") && names.contains("st-mouse-natural-scroll"));
        assert!(names.contains("st-mouse-gtk-enable-primary-paste"));
    }
}
