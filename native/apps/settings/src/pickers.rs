//! The pickers of the new Settings pages (notes/claude/settings-parity.md §6.3), in Windows 11's style:
//!
//! * [`TimePicker`]: a button with the time ("20:00", or "8:00 PM" in the 12-hour clock) that opens Windows'
//!   hour/minute flyout; the value is GNOME's fractional hours (night light's schedule).
//! * [`DatePicker`]: a button with the date that opens a calendar flyout (Datum en tijd › handmatig instellen).
//! * [`KeyCapture`]: "Druk op de gewenste toetsencombinatie" for a keyboard shortcut: the host's `input.capture`
//!   first (batch A: extension/inputBridge.js takes the next key through GNOME Shell, so no inhibit prompt, amendment
//!   44), a key controller on the window while that method is not there. Esc cancels, Backspace turns the shortcut off.
//! * [`QrCode`]: a QR code drawn from libqrencode 4 (dlopen; None when the library is missing): Wi-Fi sharing and the
//!   hotspot.
//! * [`admin_button`]: a button with Windows' shield, for an action that asks for the administrator password.
use crate::pages::Page;
use kit::controls::{self as c, Dialog, DialogResult, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

// ------------------------------------------------------------------------------------------------- TimePicker

/// "20:00" / "8:00 PM" for fractional hours.
pub fn time_text(hours: f64, h24: bool) -> String {
    let total = (hours.rem_euclid(24.0) * 60.0).round() as i64 % (24 * 60);
    let (h, m) = (total / 60, total % 60);
    if h24 {
        format!("{h:02}:{m:02}")
    } else {
        let (h12, pm) = (if h % 12 == 0 { 12 } else { h % 12 }, h >= 12);
        format!("{h12}:{m:02} {}", if pm { "PM" } else { "AM" })
    }
}

/// Windows' time picker (night light's Inschakelen / Uitschakelen).
#[derive(Clone)]
pub struct TimePicker {
    button: gtk::MenuButton,
    value: Rc<Cell<f64>>,
    h24: bool,
    handlers: Rc<RefCell<Vec<Rc<dyn Fn(f64)>>>>,
}

impl TimePicker {
    pub fn new(hours: f64, h24: bool) -> TimePicker {
        let button = gtk::MenuButton::new();
        button.add_css_class("w11-button");
        button.set_label(&time_text(hours, h24));
        let picker = TimePicker { button, value: Rc::new(Cell::new(hours)), h24, handlers: Rc::new(RefCell::new(Vec::new())) };
        let popover = gtk::Popover::new();
        let col = gtk::Box::new(gtk::Orientation::Vertical, 12);
        col.set_margin_top(12);
        col.set_margin_bottom(12);
        col.set_margin_start(12);
        col.set_margin_end(12);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let (hmax, hmin) = if h24 { (23.0, 0.0) } else { (12.0, 1.0) };
        let hour = c::number_box(hmin, hmax, 1.0, 0.0, 0);
        hour.set_wrap(true);
        hour.set_widget_name("time-picker-hour");
        let minute = c::number_box(0.0, 59.0, 1.0, 0.0, 0);
        minute.set_wrap(true);
        minute.set_widget_name("time-picker-minute");
        row.append(&hour);
        row.append(&gtk::Label::new(Some(":")));
        row.append(&minute);
        let pm = gtk::DropDown::from_strings(&["AM", "PM"]);
        if !h24 {
            row.append(&pm);
        }
        col.append(&row);
        let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        buttons.set_homogeneous(true);
        let ok = c::button(t("Opslaan", "Save"), Variant::Accent);
        let cancel = c::button(t("Annuleren", "Cancel"), Variant::Standard);
        buttons.append(&ok);
        buttons.append(&cancel);
        col.append(&buttons);
        popover.set_child(Some(&col));
        picker.button.set_popover(Some(&popover));
        {
            let (v, hour, minute, pm) = (picker.value.clone(), hour.clone(), minute.clone(), pm.clone());
            popover.connect_show(move |_| {
                let total = (v.get().rem_euclid(24.0) * 60.0).round() as i64 % (24 * 60);
                let (h, m) = (total / 60, total % 60);
                if h24 {
                    hour.set_value(h as f64);
                } else {
                    hour.set_value(if h % 12 == 0 { 12.0 } else { (h % 12) as f64 });
                    pm.set_selected(u32::from(h >= 12));
                }
                minute.set_value(m as f64);
            });
        }
        {
            let (picker2, popover2) = (picker.clone(), popover.clone());
            ok.connect_clicked(move |_| {
                let h = hour.value() as i64;
                let h = if h24 { h } else { h % 12 + if pm.selected() == 1 { 12 } else { 0 } };
                picker2.set_value(h as f64 + minute.value() / 60.0);
                popover2.popdown();
                let handlers: Vec<_> = picker2.handlers.borrow().clone();
                for f in handlers {
                    f(picker2.value());
                }
            });
        }
        let popover2 = popover.clone();
        cancel.connect_clicked(move |_| popover2.popdown());
        picker
    }

    pub fn widget(&self) -> &gtk::MenuButton {
        &self.button
    }

    pub fn value(&self) -> f64 {
        self.value.get()
    }

    /// Show another value without running the handlers.
    pub fn set_value(&self, hours: f64) {
        self.value.set(hours);
        self.button.set_label(&time_text(hours, self.h24));
    }

    /// Run `f(hours)` when the user saves a time.
    pub fn connect_changed(&self, f: impl Fn(f64) + 'static) {
        self.handlers.borrow_mut().push(Rc::new(f));
    }
}

// ------------------------------------------------------------------------------------------------- DatePicker

/// Windows' date picker: (year, month 1-12, day).
#[derive(Clone)]
pub struct DatePicker {
    button: gtk::MenuButton,
    value: Rc<Cell<(i32, u32, u32)>>,
    handlers: Rc<RefCell<Vec<Rc<dyn Fn((i32, u32, u32))>>>>,
}

fn date_text(d: (i32, u32, u32)) -> String {
    format!("{}-{}-{}", d.2, d.1, d.0)
}

impl DatePicker {
    pub fn new(date: (i32, u32, u32)) -> DatePicker {
        let button = gtk::MenuButton::new();
        button.add_css_class("w11-button");
        button.set_label(&date_text(date));
        let picker = DatePicker { button, value: Rc::new(Cell::new(date)), handlers: Rc::new(RefCell::new(Vec::new())) };
        let popover = gtk::Popover::new();
        let calendar = gtk::Calendar::new();
        calendar.set_widget_name("date-picker-calendar");
        popover.set_child(Some(&calendar));
        picker.button.set_popover(Some(&popover));
        {
            let (v, calendar) = (picker.value.clone(), calendar.clone());
            popover.connect_show(move |_| {
                let (y, m, d) = v.get();
                if let Ok(dt) = glib::DateTime::from_local(y, m as i32, d as i32, 12, 0, 0.0) {
                    calendar.set_date(&dt);
                }
            });
        }
        let (picker2, popover2) = (picker.clone(), popover.clone());
        calendar.connect_day_selected(move |cal| {
            let dt = cal.date();
            picker2.set_value((dt.year(), dt.month() as u32, dt.day_of_month() as u32));
            popover2.popdown();
            let handlers: Vec<_> = picker2.handlers.borrow().clone();
            for f in handlers {
                f(picker2.value());
            }
        });
        picker
    }

    pub fn widget(&self) -> &gtk::MenuButton {
        &self.button
    }

    pub fn value(&self) -> (i32, u32, u32) {
        self.value.get()
    }

    /// Show another date without running the handlers.
    pub fn set_value(&self, date: (i32, u32, u32)) {
        self.value.set(date);
        self.button.set_label(&date_text(date));
    }

    /// Run `f(date)` when the user picks a date.
    pub fn connect_changed(&self, f: impl Fn((i32, u32, u32)) + 'static) {
        self.handlers.borrow_mut().push(Rc::new(f));
    }
}

// ------------------------------------------------------------------------------------------------- KeyCapture

/// What a key capture gave.
#[derive(Clone, Debug, PartialEq)]
pub enum Capture {
    /// A GTK accelerator name ("<Super>t", "<Control><Alt>Delete").
    Accel(String),
    /// Backspace: the shortcut is turned off.
    Disabled,
    /// Esc or the dialog was closed.
    Cancelled,
}

/// The accelerator of a key press, None for a modifier on its own.
pub fn accel_of(keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) -> Option<String> {
    use gtk::gdk::Key;
    const MODIFIERS: &[Key] = &[
        Key::Shift_L, Key::Shift_R, Key::Control_L, Key::Control_R, Key::Alt_L, Key::Alt_R, Key::Super_L, Key::Super_R,
        Key::Meta_L, Key::Meta_R, Key::Hyper_L, Key::Hyper_R, Key::ISO_Level3_Shift, Key::ISO_Level5_Shift,
        Key::Caps_Lock, Key::Num_Lock,
    ];
    if MODIFIERS.contains(&keyval) {
        return None;
    }
    let mods = state & gtk::accelerator_get_default_mod_mask();
    Some(gtk::accelerator_name(keyval.to_lower(), mods).to_string())
}

pub struct KeyCapture;

impl KeyCapture {
    /// "Sneltoets instellen": the next key combination, then `done`.
    pub fn run(p: &Page, title: &str, done: impl FnOnce(Capture) + 'static) {
        let done: Rc<RefCell<Option<Box<dyn FnOnce(Capture)>>>> = Rc::new(RefCell::new(Some(Box::new(done))));
        let finish = {
            let done = done.clone();
            move |c: Capture| {
                if let Some(f) = done.borrow_mut().take() {
                    f(c);
                }
            }
        };
        let finish = Rc::new(finish);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.append(&c::label(
            t("Druk op de gewenste toetsencombinatie.", "Press the key combination you want."),
            "w11-body",
            true,
        ));
        body.append(&c::label(
            t("Druk op Esc om te annuleren of op Backspace om de sneltoets uit te schakelen.", "Press Esc to cancel or Backspace to turn off the shortcut."),
            "w11-text-2",
            true,
        ));
        let dialog = Dialog::new(title).body(&body).close_label(t("Annuleren", "Cancel")).width(400);
        let window = p.window().window().clone();
        let controller = gtk::EventControllerKey::new();
        controller.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let (dialog, finish) = (dialog.clone(), finish.clone());
            controller.connect_key_pressed(move |_, keyval, _, state| {
                if keyval == gtk::gdk::Key::Escape {
                    return glib::Propagation::Proceed; // the dialog closes: Cancelled
                }
                let plain = (state & gtk::accelerator_get_default_mod_mask()).is_empty();
                if keyval == gtk::gdk::Key::BackSpace && plain {
                    finish(Capture::Disabled);
                    dialog.close(DialogResult::Primary);
                    return glib::Propagation::Stop;
                }
                match accel_of(keyval, state) {
                    Some(accel) => {
                        finish(Capture::Accel(accel));
                        dialog.close(DialogResult::Primary);
                        glib::Propagation::Stop
                    }
                    None => glib::Propagation::Stop,
                }
            });
        }
        let added = Rc::new(Cell::new(false));
        // the host's capture (GNOME Shell's modal grab) while it has not answered: a closed dialog releases it
        let host_waits = Rc::new(Cell::new(true));
        let attach = {
            let (window, controller, added) = (window.clone(), controller.clone(), added.clone());
            move || {
                if !added.replace(true) {
                    window.add_controller(controller.clone());
                }
            }
        };
        {
            let (window, controller, added, finish) = (window.clone(), controller.clone(), added.clone(), finish.clone());
            let host_waits = host_waits.clone();
            dialog.show(&p.anchor(), move |_| {
                if added.replace(false) {
                    window.remove_controller(&controller);
                }
                if host_waits.replace(false) {
                    // §5.2: inputBridge.js ends its Main.pushModal (unknown_method before batch A: nothing to end)
                    drop(crate::pages::call("input.capture.cancel", json!({})));
                }
                finish(Capture::Cancelled);
            });
        }
        // the host first (A's input.capture through GNOME Shell), the window's own key controller without it
        let wait = p.call("input.capture", json!({}));
        let (dialog2, finish2) = (dialog.clone(), finish.clone());
        p.spawn(async move {
            let answer = wait.await;
            host_waits.set(false);
            match answer {
                Ok(v) if v["accel"].is_string() => {
                    finish2(Capture::Accel(v["accel"].as_str().unwrap_or("").to_owned()));
                    dialog2.close(DialogResult::Primary);
                }
                Ok(v) if v["disabled"] == true => {
                    finish2(Capture::Disabled);
                    dialog2.close(DialogResult::Primary);
                }
                Ok(_) => dialog2.close(DialogResult::Close),
                Err(_) => attach(),
            }
        });
    }
}

// ------------------------------------------------------------------------------------------------------ QrCode

#[repr(C)]
struct QRcode {
    version: libc::c_int,
    width: libc::c_int,
    data: *mut u8,
}

type EncodeString = unsafe extern "C" fn(*const libc::c_char, libc::c_int, libc::c_int, libc::c_int, libc::c_int) -> *mut QRcode;
type Free = unsafe extern "C" fn(*mut QRcode);

struct QrLib {
    encode: EncodeString,
    free: Free,
}

fn qr_lib() -> Option<&'static QrLib> {
    static LIB: std::sync::OnceLock<Option<QrLib>> = std::sync::OnceLock::new();
    LIB.get_or_init(|| unsafe {
        let handle = libc::dlopen(c"libqrencode.so.4".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if handle.is_null() {
            return None;
        }
        let encode = libc::dlsym(handle, c"QRcode_encodeString".as_ptr());
        let free = libc::dlsym(handle, c"QRcode_free".as_ptr());
        if encode.is_null() || free.is_null() {
            return None;
        }
        // libqrencode 4's public ABI (qrencode.h): QRcode_encodeString(string, version, level, hint, casesensitive)
        Some(QrLib {
            encode: std::mem::transmute::<*mut libc::c_void, EncodeString>(encode),
            free: std::mem::transmute::<*mut libc::c_void, Free>(free),
        })
    })
    .as_ref()
}

/// The modules of a QR code for `text` (row by row, true = dark), None without libqrencode.
pub fn qr_modules(text: &str) -> Option<(usize, Vec<bool>)> {
    let lib = qr_lib()?;
    let text = std::ffi::CString::new(text).ok()?;
    // version 0 (smallest), error correction M (1), 8-bit mode (2), case-sensitive
    let code = unsafe { (lib.encode)(text.as_ptr(), 0, 1, 2, 1) };
    if code.is_null() {
        return None;
    }
    let width = unsafe { (*code).width }.max(0) as usize;
    let data = unsafe { std::slice::from_raw_parts((*code).data, width * width) };
    let modules = data.iter().map(|b| b & 1 == 1).collect();
    unsafe { (lib.free)(code) };
    Some((width, modules))
}

/// A QR code as a widget (black on white with a quiet zone, `px` square); None without libqrencode.
pub struct QrCode;

impl QrCode {
    pub fn new(text: &str, px: i32) -> Option<gtk::DrawingArea> {
        let (width, modules) = qr_modules(text)?;
        let area = gtk::DrawingArea::new();
        area.set_content_width(px);
        area.set_content_height(px);
        area.set_widget_name("qr-code");
        area.set_draw_func(move |_, cr, w, h| {
            let size = f64::from(w.min(h));
            let cells = width as f64 + 8.0; // a quiet zone of four modules
            let cell = size / cells;
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.paint().ok();
            cr.set_source_rgb(0.0, 0.0, 0.0);
            for (i, dark) in modules.iter().enumerate() {
                if *dark {
                    let (x, y) = ((i % width) as f64 + 4.0, (i / width) as f64 + 4.0);
                    cr.rectangle(x * cell, y * cell, cell + 0.2, cell + 0.2);
                }
            }
            cr.fill().ok();
        });
        Some(area)
    }
}

// -------------------------------------------------------------------------------------------------- AdminButton

/// A button with Windows' shield: the action asks for the administrator password (GNOME's polkit dialog).
pub fn admin_button(label: &str, variant: Variant) -> gtk::Button {
    let b = c::button_with_glyph("shield", label, variant);
    b.set_tooltip_text(Some(t("Hiervoor is een beheerderswachtwoord nodig.", "This needs an administrator password.")));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_read_as_windows_shows_them() {
        assert_eq!(time_text(20.0, true), "20:00");
        assert_eq!(time_text(6.5, true), "06:30");
        assert_eq!(time_text(20.0, false), "8:00 PM");
        assert_eq!(time_text(0.25, false), "12:15 AM");
        assert_eq!(time_text(23.999, true), "00:00");
    }

    #[test]
    fn a_qr_code_has_its_modules_when_libqrencode_is_there() {
        if let Some((width, modules)) = qr_modules("WIFI:S:Thuis;T:WPA;P:geheim;;") {
            assert!(width >= 21 && (width - 17) % 4 == 0, "a QR version's width: {width}");
            assert_eq!(modules.len(), width * width);
            // the finder pattern's top-left corner is dark
            assert!(modules[0] && modules[6] && modules[6 * width]);
        }
    }
}
