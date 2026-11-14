//! Instellingen › Bluetooth en apparaten (last-webkit:vendor/ui/js/apps/settings/pages/bluetooth.js): the power toggle,
//! the device tiles (connect, disconnect, remove, battery), the Win11 "Een apparaat toevoegen" dialog (search, pick,
//! confirm a code / passkey / PIN, then done or failed) and the "Bluetooth reageert niet" banner with "Bluetooth
//! opnieuw starten" (system.bluetooth.reset, one job shared by the page, the dialog and Startpagina).
//!
//! Deep link: {page: "bluetooth", add: 1} opens "Een apparaat toevoegen" (quick settings › Bluetooth ›
//! Apparaat toevoegen). While the dialog lists devices it renews system.bluetooth.discover every 20 s and stops
//! the search when it closes; pairing progress arrives as system.bluetooth.pairing {address, stage, passkey?,
//! pin?, entered?, qid?}; questions are answered with system.bluetooth.pairReply.
//!
//! Shared with home.rs (keep the signatures): `device_glyph`, `device_status`, `is_not_responding`,
//! `toggle_device`, `reset_bluetooth`, `open_add_device`.
use super::background::FillGrid;
use super::{Builder, Content, Page, PageFuture};
use crate::common;
use kit::CallError;
use kit::controls::{self as c, Chevron, Dialog, DialogResult, InfoBar, MenuItem, ProgressBar, ProgressRing, SettingsCard, SettingsGroup, Severity, ToggleSwitch, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

pub const PAGES: &[(&str, Builder)] = &[("bluetooth", page)];
pub const CSS: &str = r#"
.st-dev {
  min-height: 50px;
  padding: 12px 8px 12px 16px;
  border: 1px solid var(--stroke-card);
  border-radius: var(--radius-card);
  background-color: var(--bg-card);
  color: var(--text-1);
  transition: background-color 83ms linear;
}
button.st-dev {
  min-width: 0; margin: 0;
  background-image: none; box-shadow: none; text-shadow: none; -gtk-icon-shadow: none;
  outline: 0 solid transparent; font-weight: 400;
}
.st-dev:hover { background-color: var(--fill-control-2); }
.st-dev:focus-visible { outline: 2px solid var(--focus-outer); outline-offset: -2px; }
.st-dev.st-busy { opacity: 0.7; }
.st-dev.st-busy .st-dev-icon, .st-dev.st-busy .st-dev-name { opacity: 0.6; }
.st-dev-icon { min-width: 32px; color: var(--text-1); }
.st-dev.st-connected .st-dev-icon, .st-dev-add .st-dev-icon { color: var(--accent-fg-3); }
.st-dev-status { color: var(--text-2); font-size: var(--fs-caption); }
.st-bt-stuck-busy { min-height: 32px; padding: 0 6px; }

.st-add-lead { color: var(--text-1); }
button.st-add-kind {
  min-height: 44px; margin: 0; padding: 11px 16px;
  border: 1px solid var(--stroke-control); border-bottom-color: var(--stroke-control-bottom);
  border-radius: var(--radius-control);
  background-color: var(--fill-control); background-image: none; box-shadow: none;
  text-shadow: none; -gtk-icon-shadow: none; outline: 0 solid transparent;
  color: var(--text-1); font-weight: 400;
  transition: background-color 83ms linear;
}
button.st-add-kind:hover { background-color: var(--fill-control-2); }
button.st-add-kind:active { background-color: var(--fill-control-3); color: var(--text-2); border-bottom-color: var(--stroke-control); }
button.st-add-kind:disabled { color: var(--text-disabled); background-color: var(--fill-control-disabled); border-bottom-color: var(--stroke-control); }
button.st-add-kind:focus-visible { outline: 2px solid var(--focus-outer); outline-offset: -2px; }
.st-add-kind-icon { min-width: 28px; }
.st-add-kind-title { font-weight: 600; }
.st-add-kind-desc { color: var(--text-2); font-size: var(--fs-caption); }
button.st-add-kind:disabled .st-add-kind-title, button.st-add-kind:disabled .st-add-kind-desc { color: var(--text-disabled); }
.st-add-kind-na { color: var(--text-disabled); font-size: var(--fs-caption); }
.st-add-list-scroll { margin: 0 -6px; }
.st-add-list { padding: 2px 6px; }
.st-add-dev {
  border: 1px solid var(--stroke-card); border-radius: var(--radius-control);
  background-color: var(--fill-control);
  transition: background-color 83ms linear, opacity 167ms linear;
}
.st-add-dev:not(.st-dim):hover, .st-add-dev.st-selected { background-color: var(--fill-control-2); }
.st-add-dev.st-dim { opacity: 0.45; }
button.st-add-dev-head {
  min-height: 40px; margin: 0; padding: 8px 14px;
  border: 0 solid transparent; border-radius: var(--radius-control);
  background-color: transparent; background-image: none; box-shadow: none;
  text-shadow: none; -gtk-icon-shadow: none; outline: 0 solid transparent;
  color: var(--text-1); font-weight: 400;
}
button.st-add-dev-head:disabled, button.st-add-dev-head:disabled .st-add-dev-name { color: var(--text-1); }
button.st-add-dev-head:disabled .st-add-dev-sub { color: var(--text-2); }
button.st-add-dev-head:focus-visible { outline: 2px solid var(--focus-outer); outline-offset: -2px; }
.st-add-dev-icon { min-width: 28px; color: var(--text-1); }
.st-add-dev.st-selected .st-add-dev-name { font-weight: 600; }
.st-add-dev-sub { color: var(--text-2); font-size: var(--fs-caption); }
.st-add-panel { padding: 0 16px 16px 56px; }
.st-add-status { color: var(--text-2); font-size: var(--fs-caption); }
.st-add-code {
  margin: 2px 0;
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: var(--fs-title); font-weight: 600; color: var(--text-1);
}
.st-add-digit { transition: color 167ms linear; }
.st-add-digit.st-typed { color: var(--accent-fg); }
.st-add-err { color: var(--sys-critical); font-size: var(--fs-caption); }
.st-add-actions button { min-width: 112px; }
.st-add-searching { padding: 8px 2px 0 2px; color: var(--text-2); font-size: var(--fs-caption); }
.st-add-hint { margin-top: 4px; color: var(--text-2); font-size: var(--fs-caption); }
.st-add-result-title {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: var(--fs-body-large); font-weight: 600;
}
.st-add-result-msg { margin: -4px 0 0 36px; color: var(--text-2); }
.st-add-devcard {
  min-height: 38px; padding: 10px 14px;
  border: 1px solid var(--stroke-card); border-radius: var(--radius-control);
  background-color: var(--fill-control);
}
.st-add-devcard.st-connected .st-add-dev-icon { color: var(--accent-fg-3); }
.st-add-result.st-failed .st-add-devcard { opacity: 0.75; }
.st-add-result-note { margin: -6px 0 0 2px; color: var(--text-2); font-size: var(--fs-caption); }
"#;

fn s(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

/// A value as text (a passkey may come as a number), "" for none.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn upper_addr(d: &Value) -> String {
    text_of(&d["address"]).to_uppercase()
}

fn dev_name(d: &Value) -> String {
    let n = s(&d["name"]);
    if n.is_empty() { text_of(&d["address"]) } else { n.to_owned() }
}

// ------------------------------------------------------------------------------------------------ glyphs

/// A glyph for a device from its BlueZ icon name or its name (bluetooth.js deviceGlyph).
pub fn device_glyph(name: &str) -> &'static str {
    let n = name.to_lowercase();
    let has = |s: &str| n.contains(s);
    if has("headphone") || has("headset") || has("earbud") || has("audio-head") {
        "headphones"
    } else if has("mouse") {
        "mouse"
    } else if has("keyboard") {
        "keyboard"
    } else if has("phone") || has("smartphone") {
        "phone"
    } else if has("game") || has("gaming") || has("joystick") {
        "gamepad"
    } else if has("speaker") || has("audio") || has("sound") {
        "sound"
    } else if has("print") {
        "print"
    } else if has("camera") || has("video") {
        "camera"
    } else {
        "devices"
    }
}

/// The host's bt_kind() as a glyph (bluetooth.js KIND_ICON).
fn kind_icon(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "headphones" => "headphones",
        "phone" => "phone",
        "computer" => "pc",
        "keyboard" => "keyboard",
        "mouse" => "mouse",
        "gamepad" => "gamepad",
        "pen" => "edit",
        "display" => "display",
        "camera" => "camera",
        "printer" => "print",
        "network" => "network",
        "media" => "sound",
        "other" => "bluetooth",
        _ => return None,
    })
}

/// The kinds without a glyph of their own, drawn on the glyphs' 20 x 20 grid with their stroke
/// (bluetooth.js KIND_DRAWING).
fn kind_drawing(kind: &'static str, px: i32) -> gtk::Widget {
    let area = gtk::DrawingArea::new();
    area.set_content_width(px);
    area.set_content_height(px);
    area.set_halign(gtk::Align::Center);
    area.set_valign(gtk::Align::Center);
    area.set_can_target(false);
    area.set_draw_func(move |a, cr, w, _| {
        use std::f64::consts::PI;
        let color = a.color();
        cr.scale(w as f64 / 20.0, w as f64 / 20.0);
        cr.set_source_rgba(color.red() as f64, color.green() as f64, color.blue() as f64, color.alpha() as f64);
        cr.set_line_width(kit::icons::STROKE);
        cr.set_line_cap(gtk::cairo::LineCap::Round);
        cr.set_line_join(gtk::cairo::LineJoin::Round);
        let rrect = |x: f64, y: f64, w: f64, h: f64, r: f64| {
            cr.new_sub_path();
            cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
            cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
            cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
            cr.arc(x + r, y + r, r, PI, 1.5 * PI);
            cr.close_path();
        };
        match kind {
            "headset" => {
                cr.move_to(3.5, 12.5);
                cr.line_to(3.5, 10.0);
                cr.arc(10.0, 10.0, 6.5, PI, 2.0 * PI);
                cr.line_to(16.5, 12.5);
                rrect(3.0, 11.0, 3.5, 5.0, 1.5);
                rrect(13.5, 11.0, 3.5, 5.0, 1.5);
                cr.new_sub_path();
                cr.move_to(15.25, 16.0);
                cr.line_to(15.25, 16.5);
                cr.arc(13.25, 16.5, 2.0, 0.0, PI / 2.0);
                cr.line_to(11.0, 18.5);
                let _ = cr.stroke();
            }
            "speaker" => {
                rrect(5.0, 2.5, 10.0, 15.0, 2.0);
                cr.new_sub_path();
                cr.arc(10.0, 11.75, 2.75, 0.0, 2.0 * PI);
                let _ = cr.stroke();
                cr.arc(10.0, 6.0, 0.9, 0.0, 2.0 * PI);
                let _ = cr.fill();
            }
            _ => {
                rrect(5.0, 5.0, 10.0, 10.0, 3.0);
                for (y0, y1) in [(5.0, 2.5), (15.0, 17.5)] {
                    cr.new_sub_path();
                    cr.move_to(7.0, y0);
                    cr.line_to(7.5, y1);
                    cr.line_to(12.5, y1);
                    cr.line_to(13.0, y0);
                }
                cr.new_sub_path();
                cr.move_to(10.0, 7.75);
                cr.line_to(10.0, 10.0);
                cr.line_to(11.5, 11.0);
                let _ = cr.stroke();
            }
        }
    });
    area.upcast()
}

/// The icon of a device: the host's kind (bt_kind), else guessed from its BlueZ icon name or its name
/// (bluetooth.js deviceIcon).
pub fn device_icon(d: &Value, px: i32) -> gtk::Widget {
    let kind = s(&d["kind"]);
    let icon = s(&d["icon"]);
    match kind {
        "headset" => return kind_drawing("headset", px),
        "speaker" => return kind_drawing("speaker", px),
        "watch" => return kind_drawing("watch", px),
        _ => {}
    }
    if let Some(g) = kind_icon(kind)
        && !(kind == "other" && !icon.is_empty() && device_glyph(icon) != "devices")
    {
        return kit::icons::glyph(g, px).upcast();
    }
    kit::icons::glyph(device_glyph(if icon.is_empty() { s(&d["name"]) } else { icon }), px).upcast()
}

/// The kind as words, "" for an unknown kind.
fn kind_label(kind: &str) -> &'static str {
    match kind {
        "headphones" => t("Koptelefoon", "Headphones"),
        "headset" => t("Headset", "Headset"),
        "speaker" => t("Luidspreker", "Speaker"),
        "phone" => t("Telefoon", "Phone"),
        "computer" => t("Computer", "Computer"),
        "keyboard" => t("Toetsenbord", "Keyboard"),
        "mouse" => t("Muis", "Mouse"),
        "gamepad" => t("Controller", "Controller"),
        "pen" => t("Pen", "Pen"),
        "display" => t("Beeldscherm", "Display"),
        "camera" => t("Camera", "Camera"),
        "printer" => t("Printer", "Printer"),
        "watch" => t("Horloge", "Watch"),
        "network" => t("Netwerk", "Network"),
        "media" => t("Audio", "Audio"),
        _ => "",
    }
}

// ----------------------------------------------------------------------------------------------- helpers

/// "Verbonden · Batterij 80%" (bluetooth.js deviceStatus).
pub fn device_status(d: &Value) -> String {
    let state = if d["connected"] == true {
        t("Verbonden", "Connected")
    } else if d["paired"] == true {
        t("Gekoppeld", "Paired")
    } else {
        t("Niet verbonden", "Not connected")
    };
    match d["battery"].as_f64() {
        Some(b) => format!("{state} · {}", t("Batterij {n}%", "Battery {n}%").replace("{n}", &format!("{}", b.round() as i64))),
        None => state.to_owned(),
    }
}

/// The host's NotReady answer ("Bluetooth reageert niet"): its code or catalog key, never its text
/// (bluetooth.js isNotResponding).
pub fn is_not_responding(e: &CallError) -> bool {
    e.code == "not_ready" || e.code == "not_responding" || e.key.as_deref() == Some("system.bt_not_responding")
}

/// `is_not_responding` for a `{ok: false, code, key}` answer.
fn value_not_responding(v: &Value) -> bool {
    v["code"] == "not_ready" || v["code"] == "not_responding" || v["key"] == "system.bt_not_responding"
}

/// A host answer `{ok: false, error, code}` counts as failed too (bluetooth.js attempt).
fn checked(r: Result<Value, CallError>) -> Result<Value, CallError> {
    let v = r?;
    if v["ok"] == false {
        let mut e = CallError::new(v["code"].as_str().unwrap_or("failed"), v["error"].as_str().unwrap_or(""));
        e.key = v["key"].as_str().filter(|k| !k.is_empty()).map(str::to_owned);
        e.data = v.clone();
        return Err(e);
    }
    Ok(if v.is_null() { json!({}) } else { v })
}

async fn attempt(p: &Page, method: &str, args: Value) -> Result<Value, CallError> {
    checked(p.call(method, args).await)
}

/// Connect or disconnect a paired device (bluetooth.js toggleDevice).
pub async fn toggle_device(p: &Page, d: &Value) -> Result<Value, CallError> {
    let method = if d["connected"] == true { "system.bluetooth.disconnect" } else { "system.bluetooth.connect" };
    attempt(p, method, json!({ "address": d["address"] })).await
}

/// Win11 "Apparaat verwijderen": ask first, then forget the device and its pairing (bluetooth.js
/// removeDevice). None when the user said "Nee"; `on_confirmed` runs after "Ja", before the host is asked.
async fn remove_device(p: &Page, d: &Value, on_confirmed: impl FnOnce()) -> Option<Result<Value, CallError>> {
    let (fill, wait) = super::later::<bool>();
    Dialog::new(t("Weet je zeker dat je dit apparaat wilt verwijderen?", "Are you sure you want to remove this device?"))
        .text(&common::fill(
            t(
                "“{name}” wordt vergeten, ook de koppeling. Wil je het later weer gebruiken, zet het dan in de koppelstand en voeg het opnieuw toe.",
                "“{name}” will be forgotten, including its pairing. To use it later, put it in pairing mode and add it again.",
            ),
            &[("name", &dev_name(d))],
        ))
        .primary(t("Ja", "Yes"))
        .close_label(t("Nee", "No"))
        .show(&p.anchor(), move |r| fill(r == DialogResult::Primary));
    if !wait.await {
        return None;
    }
    on_confirmed();
    Some(attempt(p, "system.bluetooth.remove", json!({ "address": d["address"] })).await)
}

// ------------------------------------------------------------------------- "Bluetooth opnieuw starten"

/// The outcome of "Bluetooth opnieuw starten".
#[derive(Clone)]
pub struct ResetOutcome {
    pub ok: bool,
    /// The adapter's new state when the host sent one.
    pub state: Option<Value>,
    /// What to tell the user when it failed.
    pub error: String,
}

/// What the listeners of the reset job hear: Busy first, then the outcome.
#[derive(Clone)]
enum ResetEvent {
    Busy,
    Done(ResetOutcome),
}

type ResetWaiter = Box<dyn FnOnce(ResetOutcome)>;
type ResetListener = Rc<dyn Fn(&ResetEvent)>;

thread_local! {
    static RESET_JOB: RefCell<Option<Vec<ResetWaiter>>> = const { RefCell::new(None) };
    static RESET_SUBS: RefCell<Vec<(u64, ResetListener)>> = const { RefCell::new(Vec::new()) };
    static RESET_NEXT: Cell<u64> = const { Cell::new(1) };
}

fn reset_running() -> bool {
    RESET_JOB.with(|j| j.borrow().is_some())
}

fn reset_notify(ev: &ResetEvent) {
    let subs: Vec<ResetListener> = RESET_SUBS.with(|s| s.borrow().iter().map(|(_, f)| f.clone()).collect());
    for f in subs {
        f(ev);
    }
}

fn reset_subscribe(f: impl Fn(&ResetEvent) + 'static) -> u64 {
    let id = RESET_NEXT.with(|n| {
        let id = n.get();
        n.set(id + 1);
        id
    });
    RESET_SUBS.with(|s| s.borrow_mut().push((id, Rc::new(f))));
    id
}

fn reset_unsubscribe(id: u64) {
    RESET_SUBS.with(|s| s.borrow_mut().retain(|(i, _)| *i != id));
}

/// One reset at a time: a second request waits for the running one.
fn reset_with(done: impl FnOnce(ResetOutcome) + 'static) {
    let start = RESET_JOB.with(|j| {
        let mut j = j.borrow_mut();
        match j.as_mut() {
            Some(waiters) => {
                waiters.push(Box::new(done));
                false
            }
            None => {
                *j = Some(vec![Box::new(done)]);
                true
            }
        }
    });
    if !start {
        return;
    }
    reset_notify(&ResetEvent::Busy);
    kit::ctx().call("system.bluetooth.reset", json!({}), |r| {
        let out = match r {
            Ok(v) => ResetOutcome { ok: true, state: if v["available"].is_null() { None } else { Some(v) }, error: String::new() },
            Err(e) => {
                // "… lukte niet" is the banner's title already: for code "failed" the message says what to do
                // next; other codes (blocked, unavailable, offline) keep the host's own text
                let generic = t(
                    "Probeer het nog een keer. Lukt het dan niet, start de computer opnieuw op.",
                    "Try again. If that doesn’t help, restart the computer.",
                );
                let error = if e.code == "failed" || e.message.is_empty() { generic.to_owned() } else { e.message.clone() };
                ResetOutcome { ok: false, state: None, error }
            }
        };
        let waiters = RESET_JOB.with(|j| j.borrow_mut().take()).unwrap_or_default();
        reset_notify(&ResetEvent::Done(out.clone()));
        for w in waiters {
            w(out.clone());
        }
    });
}

/// system.bluetooth.reset (bluetooth.js resetBluetooth): the shared job; every page and the dialog hear it.
pub async fn reset_bluetooth(_p: &Page) -> ResetOutcome {
    let (fill, wait) = super::later();
    reset_with(fill);
    wait.await
}

/// Start the reset (or join the running one) from a button.
fn start_reset() {
    reset_with(|_| {});
}

/// The "Bluetooth reageert niet" info bar: modes None, "stuck", "busy", "ok", "error" (bluetooth.js stuckBar).
struct StuckBar {
    bar: InfoBar,
    mode: RefCell<String>,
}

impl StuckBar {
    fn new(compact: bool) -> Rc<StuckBar> {
        let bar = InfoBar::new(Severity::Warning, "", "");
        bar.set_closable(false);
        bar.widget().add_css_class("st-notice");
        bar.widget().add_css_class("st-bt-stuck");
        bar.widget().set_widget_name(if compact { "bt-add-stuck" } else { "bt-stuck" });
        let me = Rc::new(StuckBar { bar, mode: RefCell::new("?".into()) });
        me.set_mode(None, "");
        me
    }

    fn set_mode(&self, mode: Option<&str>, error: &str) {
        let key = format!("{}|{error}", mode.unwrap_or(""));
        if *self.mode.borrow() == key {
            return;
        }
        *self.mode.borrow_mut() = key;
        self.bar.set_visible(mode.is_some());
        let Some(m) = mode else { return };
        let b = &self.bar;
        // a button goes on its own line under the text (.st-bt-stuck .fl-infobar__action:has(> .fl-btn)); the
        // busy ring stays at the end of the text
        b.set_action_below(m != "busy");
        match m {
            "busy" => {
                b.set_severity(Severity::Info);
                b.set_title(t("Bluetooth wordt opnieuw gestart…", "Restarting Bluetooth…"));
                b.set_message(t("Dit duurt een paar seconden.", "This takes a few seconds."));
                let wrap = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                wrap.add_css_class("st-bt-stuck-busy");
                let ring = ProgressRing::new(20, None);
                ring.widget().set_valign(gtk::Align::Center);
                ring.widget().set_tooltip_text(Some(t("Bluetooth wordt opnieuw gestart…", "Restarting Bluetooth…")));
                wrap.append(ring.widget());
                b.set_action_widget(&wrap);
            }
            "ok" => {
                b.set_severity(Severity::Success);
                b.set_title(t("Bluetooth werkt weer", "Bluetooth is working again"));
                b.set_message(t("Je kunt je apparaten weer verbinden.", "You can connect your devices again."));
                b.set_action(None);
            }
            "error" => {
                b.set_severity(Severity::Error);
                b.set_title(t("Bluetooth opnieuw starten lukte niet", "Couldn’t restart Bluetooth"));
                b.set_message(if error.is_empty() {
                    t("Probeer het nog een keer. Lukt het dan niet, start de computer opnieuw op.", "Try again. If that doesn’t help, restart the computer.")
                } else {
                    error
                });
                b.set_action(Some((t("Opnieuw proberen", "Try again"), Rc::new(start_reset))));
            }
            _ => {
                b.set_severity(Severity::Warning);
                b.set_title(t("Bluetooth reageert niet", "Bluetooth is not responding"));
                b.set_message(t(
                    "Je apparaten kunnen nu geen verbinding maken. Start Bluetooth opnieuw om dit op te lossen.",
                    "Your devices can’t connect right now. Restart Bluetooth to fix this.",
                ));
                b.set_action(Some((t("Bluetooth opnieuw starten", "Restart Bluetooth"), Rc::new(start_reset))));
            }
        }
    }
}

/// Put `w` in `parent`, taking it out of wherever it is now.
fn adopt(parent: &gtk::Box, w: &impl IsA<gtk::Widget>) {
    let w = w.as_ref();
    if let Some(old) = w.parent() {
        if old == *parent.upcast_ref::<gtk::Widget>() {
            return;
        }
        if let Some(b) = old.downcast_ref::<gtk::Box>() {
            b.remove(w);
        } else {
            w.unparent();
        }
    }
    parent.append(w);
}

// --------------------------------------------------------------------------- "Een apparaat toevoegen"

/// The host stops searching 60 s after the last discover {on: true}.
const SEARCH_RENEW_MS: u64 = 20_000;
/// "Zie je je apparaat niet?" after this long without devices.
const HINT_AFTER_MS: u64 = 12_000;
/// Stages that hold an agent question (answered with pairReply).
const QUESTIONS: &[&str] = &["confirm", "passkey", "pin"];
/// Bridge codes: the host's answer never arrived.
const LOST: &[&str] = &["offline", "bad_response"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    Choose,
    List,
    Done,
    Failed,
}

/// The device being paired.
struct Sel {
    addr: String,
    dev: Value,
    stage: String,
    qid: Value,
    passkey: Option<String>,
    pin: Option<String>,
    entered: Option<i64>,
    token: u64,
    busy: bool,
    cancelling: bool,
    user_cancelled: bool,
    error: Option<String>,
}

/// The device of the done / failed step.
struct Outcome {
    dev: Value,
    connected: bool,
    error: Option<String>,
    id: u64,
}

struct WState {
    /// The last system.bluetooth.get.
    bt: Option<Value>,
    step: Step,
    /// Nearby devices in the order they appeared (no jumping rows).
    found: Vec<(String, Value)>,
    sel: Option<Sel>,
    /// The address picked last: focus goes back to its row.
    last_sel: Option<String>,
    token: u64,
    result: Option<Outcome>,
    results: u64,
    /// The test-mode note above the list.
    note: Option<String>,
    search_err: Option<String>,
    /// A call answered "Bluetooth reageert niet".
    not_ready: bool,
    reset_mode: Option<&'static str>,
    reset_error: String,
    /// Our discover {on: true} runs (or is under way): {on: false} at the end.
    search_wanted: bool,
    list_since: Option<Instant>,
    closed: bool,
    /// The host was told to stop this dialog's search / pairing already.
    released: bool,
}

impl WState {
    fn live(&self) -> bool {
        self.bt.as_ref().is_some_and(|b| b["available"] == true && b["powered"] == true && b["stuck"] != true) && !self.not_ready
    }

    fn bt_usable(&self) -> bool {
        self.bt.as_ref().is_none_or(|b| b["available"] != false)
    }
}

/// A code with digits that turn accent-coloured as they are typed on the device (`.st-add__code`).
#[derive(Clone)]
struct CodeView {
    root: gtk::Box,
    digits: Vec<gtk::Label>,
}

impl CodeView {
    fn new(code: &str, entered: Option<i64>) -> CodeView {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 3);
        root.add_css_class("st-add-code");
        root.add_css_class("w11-tabular");
        let digits: Vec<gtk::Label> = code
            .chars()
            .map(|ch| {
                let l = gtk::Label::new(Some(&ch.to_string()));
                l.add_css_class("st-add-digit");
                root.append(&l);
                l
            })
            .collect();
        let me = CodeView { root, digits };
        me.set_entered(entered);
        me
    }

    fn set_entered(&self, n: Option<i64>) {
        for (i, d) in self.digits.iter().enumerate() {
            if n.is_some_and(|n| (i as i64) < n) {
                d.add_css_class("st-typed");
            } else {
                d.remove_css_class("st-typed");
            }
        }
    }
}

/// One row of the device list: the head (a button that starts pairing) and the panel of the pairing steps.
struct DevRow {
    row: gtk::Box,
    head: gtk::Button,
    icon: gtk::Box,
    name: gtk::Label,
    sub: gtk::Label,
    panel: gtk::Box,
    panel_key: RefCell<Option<String>>,
    kind: RefCell<String>,
    input: RefCell<Option<gtk::Entry>>,
    err: RefCell<Option<gtk::Label>>,
    code: RefCell<Option<CodeView>>,
}

type CloseFn = Box<dyn FnOnce()>;

struct Wizard {
    st: RefCell<WState>,
    dialog: Dialog,
    anchor: gtk::Widget,
    frame: RefCell<Option<gtk::Widget>>,
    lead: gtk::Label,
    body: gtk::Box,
    footer: RefCell<Option<gtk::Box>>,
    footer_btns: RefCell<Vec<gtk::Button>>,
    cancel_btn: gtk::Button,
    note_slot: gtk::Box,
    note_key: RefCell<String>,
    stuck: Rc<StuckBar>,
    list: gtk::Box,
    list_scroll: gtk::ScrolledWindow,
    search_row: gtk::Box,
    hint: gtk::Label,
    rows: RefCell<HashMap<String, Rc<DevRow>>>,
    result_el: RefCell<Option<(u64, String, gtk::Widget)>>,
    again_btn: RefCell<Option<gtk::Button>>,
    on_close: RefCell<Vec<CloseFn>>,
    listeners: RefCell<Vec<kit::ListenerId>>,
    reset_sub: Cell<u64>,
    window_hooks: RefCell<Option<(gtk::Window, gtk::EventControllerKey, glib::SignalHandlerId)>>,
}

thread_local! {
    /// The open dialog (one at a time).
    static WIZARD: RefCell<Option<Rc<Wizard>>> = const { RefCell::new(None) };
}

/// "Apparaat toevoegen" (bluetooth.js openAddDevice / addDeviceHelp): the Win11 "Een apparaat toevoegen"
/// dialog, or the open one.
pub fn open_add_device(p: &Page) {
    open_add_device_then(p, None);
}

/// `open_add_device`; `on_close` runs when the dialog closed.
fn open_add_device_then(p: &Page, on_close: Option<CloseFn>) {
    if let Some(w) = WIZARD.with(|w| w.borrow().clone()) {
        if let Some(f) = on_close {
            w.on_close.borrow_mut().push(f);
        }
        return;
    }
    let w = Wizard::open(&p.anchor());
    if let Some(f) = on_close {
        w.on_close.borrow_mut().push(f);
    }
}

/// Fire and forget (the answer does not matter); `qid` goes along unless it is null.
fn send(method: &str, mut args: Value, qid: Option<&Value>) {
    if let Some(q) = qid.filter(|q| !q.is_null()) {
        args["qid"] = q.clone();
    }
    kit::ctx().fire(method, args);
}

impl Wizard {
    fn open(anchor: &gtk::Widget) -> Rc<Wizard> {
        let lead = c::label("", "st-add-lead", true);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 8);
        body.add_css_class("st-add-step");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 16);
        root.add_css_class("st-add");
        root.append(&lead);
        root.append(&body);
        let cancel_btn = c::button(t("Annuleren", "Cancel"), Variant::Standard);
        cancel_btn.set_hexpand(true);
        cancel_btn.set_widget_name("bt-add-cancel");
        let note_slot = gtk::Box::new(gtk::Orientation::Vertical, 0);
        note_slot.set_visible(false);
        let list = gtk::Box::new(gtk::Orientation::Vertical, 4);
        list.add_css_class("st-add-list");
        let list_scroll = gtk::ScrolledWindow::new();
        list_scroll.add_css_class("st-add-list-scroll");
        list_scroll.set_hscrollbar_policy(gtk::PolicyType::Never);
        list_scroll.set_max_content_height(284);
        list_scroll.set_propagate_natural_height(true);
        list_scroll.set_child(Some(&list));
        let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        search_row.add_css_class("st-add-searching");
        let ring = ProgressRing::new(16, None);
        ring.widget().set_valign(gtk::Align::Center);
        search_row.append(ring.widget());
        search_row.append(&c::label(t("Zoeken naar apparaten…", "Looking for devices…"), "", false));
        let hint = c::label(
            t(
                "Zie je je apparaat niet? Zet het in de koppelstand: meestal houd je de aan-knop of de Bluetooth-knop een paar seconden ingedrukt, tot een lampje knippert.",
                "Don’t see your device? Put it in pairing mode: usually by holding its power or Bluetooth button for a few seconds until a light flashes.",
            ),
            "st-add-hint",
            true,
        );
        // 548 px wide (.st-add-dialog): the text wraps in the 500 px between the dialog's paddings
        let content = crate::widgets::Clamp::new(&root, 500);
        let dialog = Dialog::new(t("Een apparaat toevoegen", "Add a device")).body(&content).no_close().width(548);
        let me = Rc::new(Wizard {
            st: RefCell::new(WState {
                bt: None,
                step: Step::Choose,
                found: Vec::new(),
                sel: None,
                last_sel: None,
                token: 0,
                result: None,
                results: 0,
                note: None,
                search_err: None,
                not_ready: false,
                reset_mode: if reset_running() { Some("busy") } else { None },
                reset_error: String::new(),
                search_wanted: false,
                list_since: None,
                closed: false,
                released: false,
            }),
            dialog,
            anchor: anchor.clone(),
            frame: RefCell::new(None),
            lead,
            body,
            footer: RefCell::new(None),
            footer_btns: RefCell::new(Vec::new()),
            cancel_btn: cancel_btn.clone(),
            note_slot,
            note_key: RefCell::new(String::new()),
            stuck: StuckBar::new(true),
            list,
            list_scroll,
            search_row,
            hint,
            rows: RefCell::new(HashMap::new()),
            result_el: RefCell::new(None),
            again_btn: RefCell::new(None),
            on_close: RefCell::new(Vec::new()),
            listeners: RefCell::new(Vec::new()),
            reset_sub: Cell::new(0),
            window_hooks: RefCell::new(None),
        });
        WIZARD.with(|w| *w.borrow_mut() = Some(me.clone()));
        {
            let weak = Rc::downgrade(&me);
            cancel_btn.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.dialog.close(DialogResult::Close);
                }
            });
        }
        {
            let weak = Rc::downgrade(&me);
            me.dialog.show(anchor, move |_| {
                if let Some(me) = weak.upgrade() {
                    me.closed();
                }
            });
        }
        // the dialog's own frame: its body is 468 px high (.st-add-dialog), its footer holds this dialog's buttons
        if let Some(dbody) = content.parent() {
            dbody.set_size_request(-1, 468);
            *me.frame.borrow_mut() = dbody.parent();
            *me.footer.borrow_mut() = dbody.next_sibling().and_downcast::<gtk::Box>();
        }
        if let Some(frame) = me.frame.borrow().as_ref() {
            frame.add_css_class("st-add-dialog");
        }
        me.hook_window();

        // live updates
        let ctx = kit::ctx();
        {
            let weak = Rc::downgrade(&me);
            let id = ctx.on("system.bluetooth.changed", move |_, d| {
                if let Some(me) = weak.upgrade() {
                    me.on_changed(d);
                }
            });
            me.listeners.borrow_mut().push(id);
        }
        {
            let weak = Rc::downgrade(&me);
            let id = ctx.on("system.bluetooth.pairing", move |_, d| {
                if let Some(me) = weak.upgrade() {
                    me.on_pairing(d);
                }
            });
            me.listeners.borrow_mut().push(id);
        }
        {
            let weak = Rc::downgrade(&me);
            me.reset_sub.set(reset_subscribe(move |ev| {
                if let Some(me) = weak.upgrade() {
                    me.on_reset(ev);
                }
            }));
        }

        // start
        let wait = super::call("system.bluetooth.get", json!({}));
        let weak = Rc::downgrade(&me);
        glib::spawn_future_local(async move {
            let r = wait.await;
            let Some(me) = weak.upgrade() else { return };
            match r {
                Ok(v) if !v.is_null() => {
                    if me.take_state(&v) {
                        me.search();
                    }
                }
                _ => me.st.borrow_mut().bt = Some(json!({ "available": false })),
            }
            if !me.st.borrow().closed {
                me.render();
            }
        });
        me.render();
        me
    }

    /// Enter in the PIN box answers the question, Enter on "klaar" closes; elsewhere it does nothing (the
    /// dialog would close on Enter otherwise). A window that closes leaves no search or question behind.
    fn hook_window(self: &Rc<Self>) {
        let Some(window) = self.anchor.root().and_downcast::<gtk::Window>() else { return };
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        let win = window.clone();
        keys.connect_key_pressed(move |_, key, _, _| {
            if !matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
                return glib::Propagation::Proceed;
            }
            let Some(me) = weak.upgrade() else { return glib::Propagation::Proceed };
            if me.st.borrow().closed {
                return glib::Propagation::Proceed;
            }
            let focus = GtkWindowExt::focus(&win);
            let frame = me.frame.borrow().clone();
            let ours = match (&focus, &frame) {
                (Some(f), Some(fr)) => f.is_ancestor(fr) || f == fr,
                (None, _) => true,
                _ => false,
            };
            if !ours {
                return glib::Propagation::Proceed;
            }
            if focus.as_ref().is_some_and(|f| f.is::<gtk::Button>() || f.ancestor(gtk::Button::static_type()).is_some()) {
                return glib::Propagation::Proceed;
            }
            if focus.as_ref().is_some_and(|f| f.ancestor(gtk::Entry::static_type()).is_some() || f.is::<gtk::Entry>()) {
                me.submit();
            } else if me.st.borrow().step == Step::Done {
                me.dialog.close(DialogResult::Close);
            }
            glib::Propagation::Stop
        });
        window.add_controller(keys.clone());
        let weak = Rc::downgrade(self);
        let close_id = window.connect_close_request(move |_| {
            if let Some(me) = weak.upgrade()
                && !me.st.borrow().closed
            {
                me.release_host();
            }
            glib::Propagation::Proceed
        });
        *self.window_hooks.borrow_mut() = Some((window, keys, close_id));
    }

    fn closed(self: &Rc<Self>) {
        self.st.borrow_mut().closed = true;
        if let Some((win, keys, id)) = self.window_hooks.borrow_mut().take() {
            win.remove_controller(&keys);
            win.disconnect(id);
        }
        let ctx = kit::ctx();
        for id in self.listeners.borrow_mut().drain(..) {
            ctx.off(id);
        }
        reset_unsubscribe(self.reset_sub.get());
        self.release_host();
        WIZARD.with(|w| *w.borrow_mut() = None);
        let list: Vec<CloseFn> = self.on_close.borrow_mut().drain(..).collect();
        for f in list {
            f();
        }
    }

    // ---- state

    /// Take the host's state; true when the list should search now (the adapter just became usable).
    fn take_state(&self, s: &Value) -> bool {
        if !s.is_object() || s["available"].is_null() {
            return false;
        }
        let mut st = self.st.borrow_mut();
        let was = st.live();
        st.bt = Some(s.clone());
        if s["stuck"] == false {
            st.not_ready = false;
        }
        let near: Vec<(String, Value)> = s["nearby"].as_array().map(|a| a.iter().map(|d| (upper_addr(d), d.clone())).collect()).unwrap_or_default();
        for (a, d) in &near {
            match st.found.iter_mut().find(|(k, _)| k == a) {
                Some((_, old)) => {
                    if let (Some(o), Some(n)) = (old.as_object_mut(), d.as_object()) {
                        for (k, v) in n {
                            o.insert(k.clone(), v.clone());
                        }
                    } else {
                        *old = d.clone();
                    }
                }
                None => st.found.push((a.clone(), d.clone())),
            }
        }
        let sel = st.sel.as_ref().map(|s| s.addr.clone());
        st.found.retain(|(a, _)| near.iter().any(|(n, _)| n == a) || sel.as_deref() == Some(a.as_str()));
        !was && st.live() && st.step == Step::List && st.sel.is_none()
    }

    // ---- searching

    fn search(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            if st.closed || !st.live() {
                return;
            }
            st.search_wanted = true;
        }
        let wait = super::call("system.bluetooth.discover", json!({ "on": true }));
        let me = self.clone();
        glib::spawn_future_local(async move {
            let r = checked(wait.await);
            let stale = {
                let st = me.st.borrow();
                st.closed || !st.search_wanted
            };
            if stale {
                // the dialog closed (or the search was stopped) while this was under way: its {on: false} may have
                // reached the host first, so stop once more
                if r.is_ok() {
                    send("system.bluetooth.discover", json!({ "on": false }), None);
                }
                return;
            }
            {
                let mut st = me.st.borrow_mut();
                match &r {
                    Ok(_) => st.search_err = None,
                    Err(e) if is_not_responding(e) => {
                        st.not_ready = true;
                        st.search_err = None;
                    }
                    Err(e) => st.search_err = Some(if e.message.is_empty() { t("Zoeken naar apparaten lukte niet.", "Couldn’t search for devices.").to_owned() } else { e.message.clone() }),
                }
            }
            me.render();
        });
    }

    fn stop_search(&self) {
        {
            let mut st = self.st.borrow_mut();
            if !st.search_wanted {
                return;
            }
            st.search_wanted = false;
        }
        send("system.bluetooth.discover", json!({ "on": false }), None);
    }

    /// Everything the host still does for this dialog: an open question, our pairing, the search.
    fn release_host(&self) {
        let sel = {
            let mut st = self.st.borrow_mut();
            if st.released {
                return;
            }
            st.released = true;
            st.sel.as_ref().map(|s| (s.addr.clone(), s.stage.clone(), s.qid.clone()))
        };
        if let Some((addr, stage, qid)) = sel {
            if QUESTIONS.contains(&stage.as_str()) {
                send("system.bluetooth.pairReply", json!({ "address": addr, "accept": false }), Some(&qid));
            }
            send("system.bluetooth.cancelPair", json!({ "address": addr }), None);
        }
        self.stop_search();
    }

    // ---- pairing

    fn start_pair(self: &Rc<Self>, dev: Value) {
        let my = {
            let mut st = self.st.borrow_mut();
            if st.sel.is_some() || st.closed {
                return;
            }
            st.token += 1;
            let my = st.token;
            st.step = Step::List;
            st.note = None;
            st.result = None;
            let addr = upper_addr(&dev);
            let mut d = dev.clone();
            d["address"] = json!(addr);
            st.sel = Some(Sel {
                addr: addr.clone(),
                dev: d,
                stage: "start".into(),
                qid: Value::Null,
                passkey: None,
                pin: None,
                entered: None,
                token: my,
                busy: false,
                cancelling: false,
                user_cancelled: false,
                error: None,
            });
            st.last_sel = Some(addr);
            my
        };
        self.render();
        let address = dev["address"].clone();
        let wait = super::call("system.bluetooth.pair", json!({ "address": address.clone() }));
        let me = self.clone();
        glib::spawn_future_local(async move {
            let r = wait.await;
            if let Err(e) = &r
                && LOST.contains(&e.code.as_str())
                && !me.st.borrow().released
            {
                // the answer got lost (the connection dropped), so the host may still be pairing: stop it, the UI
                // will not wait for it any more
                let q = me.st.borrow().sel.as_ref().filter(|s| s.token == my).map(|s| (s.addr.clone(), s.stage.clone(), s.qid.clone()));
                if let Some((addr, stage, qid)) = q
                    && QUESTIONS.contains(&stage.as_str())
                {
                    send("system.bluetooth.pairReply", json!({ "address": addr, "accept": false }), Some(&qid));
                }
                send("system.bluetooth.cancelPair", json!({ "address": address }), None);
            }
            {
                let st = me.st.borrow();
                if st.closed || st.sel.as_ref().is_none_or(|s| s.token != my) {
                    return;
                }
            }
            let res = match r {
                Ok(v) if v.is_null() => json!({}),
                Ok(v) => v,
                Err(e) => json!({ "ok": false, "error": e.message, "code": e.code, "key": e.key }),
            };
            me.finish_pair(res);
        });
    }

    fn finish_pair(self: &Rc<Self>, res: Value) {
        let stop = {
            let mut st = self.st.borrow_mut();
            let Some(sel) = st.sel.take() else { return };
            if res["dryRun"] == true {
                st.note = Some(t("Testmodus: er is niets gekoppeld.", "Test mode: nothing was paired.").into());
                false
            } else if res["ok"] != false {
                st.results += 1;
                let id = st.results;
                let error = res["error"].as_str().filter(|e| !e.is_empty()).map(str::to_owned);
                st.result = Some(Outcome { dev: sel.dev, connected: res["connected"] != false, error, id });
                st.step = Step::Done;
                true
            } else if sel.user_cancelled && (res["cancelled"] == true || res["code"] == "cancelled") {
                false
            } else {
                if value_not_responding(&res) {
                    st.not_ready = true;
                }
                st.results += 1;
                let id = st.results;
                let error = res["error"].as_str().filter(|e| !e.is_empty()).unwrap_or(t("Koppelen lukte niet. Probeer het opnieuw.", "Pairing failed. Try again.")).to_owned();
                st.result = Some(Outcome { dev: sel.dev, connected: false, error: Some(error), id });
                st.step = Step::Failed;
                false
            }
        };
        if stop {
            // the device is ready: no inquiry next to its fresh connection (the host stops too)
            self.stop_search();
        }
        self.render();
    }

    fn on_pairing(self: &Rc<Self>, d: &Value) {
        let update_code = {
            let mut st = self.st.borrow_mut();
            let Some(sel) = st.sel.as_mut() else { return };
            if upper_addr(d) != sel.addr {
                return;
            }
            let stage = s(&d["stage"]).to_owned();
            if matches!(stage.as_str(), "done" | "failed" | "cancelled") {
                return; // the pair call answers with the outcome
            }
            let pin = text_of(&d["pin"]);
            let code = if !text_of(&d["passkey"]).is_empty() { text_of(&d["passkey"]) } else { pin.clone() };
            let shown = sel.passkey.clone().or_else(|| sel.pin.clone()).unwrap_or_default();
            if stage == "display" && sel.stage == "display" && code == shown {
                sel.entered = d["entered"].as_i64();
                Some((sel.addr.clone(), if pin.is_empty() { sel.entered } else { None }))
            } else {
                sel.stage = stage;
                sel.qid = d["qid"].clone();
                sel.passkey = Some(text_of(&d["passkey"])).filter(|x| !x.is_empty());
                sel.pin = Some(pin).filter(|x| !x.is_empty());
                sel.entered = d["entered"].as_i64();
                sel.error = None;
                sel.busy = false;
                None
            }
        };
        match update_code {
            Some((addr, entered)) => {
                if let Some(r) = self.rows.borrow().get(&addr)
                    && let Some(code) = r.code.borrow().as_ref()
                {
                    code.set_entered(entered);
                }
            }
            None => self.render(),
        }
    }

    /// Answer the question on screen: its qid goes along, so the host never applies it to a newer question.
    fn reply(self: &Rc<Self>, accept: bool, extra: Value) {
        let (addr, qid, token) = {
            let mut st = self.st.borrow_mut();
            let Some(s) = st.sel.as_mut() else { return };
            if s.busy {
                return;
            }
            s.busy = true;
            (s.addr.clone(), s.qid.clone(), s.token)
        };
        self.render();
        let mut args = json!({ "address": addr, "accept": accept });
        if !qid.is_null() {
            args["qid"] = qid.clone();
        }
        if let (Some(a), Some(x)) = (args.as_object_mut(), extra.as_object()) {
            for (k, v) in x {
                a.insert(k.clone(), v.clone());
            }
        }
        let wait = super::call("system.bluetooth.pairReply", args);
        let me = self.clone();
        glib::spawn_future_local(async move {
            let r = wait.await;
            let focus_input = {
                let mut st = me.st.borrow_mut();
                if st.closed {
                    return;
                }
                let Some(s) = st.sel.as_mut().filter(|s| s.token == token && s.qid == qid) else { return };
                s.busy = false;
                let failed = match &r {
                    Err(_) => true,
                    Ok(v) => v["ok"] == false,
                };
                if failed {
                    if let Ok(v) = &r
                        && v["code"] == "stale"
                    {
                        // replaced at the host; its event is on the way
                        false
                    } else {
                        let msg = match &r {
                            Ok(v) => s_owned(&v["error"]),
                            Err(e) => e.message.clone(),
                        };
                        s.error = Some(if msg.is_empty() { t("Koppelen lukte niet. Probeer het opnieuw.", "Pairing failed. Try again.").to_owned() } else { msg });
                        true
                    }
                } else {
                    if accept {
                        s.stage = "pairing".into();
                        s.qid = Value::Null;
                        s.passkey = None;
                        s.pin = None;
                        s.error = None;
                    }
                    false
                }
            };
            me.render();
            if focus_input
                && let Some(r) = me.rows.borrow().get(&addr)
                && let Some(i) = r.input.borrow().as_ref()
            {
                i.grab_focus();
            }
        });
    }

    /// The PIN box's "Verbinden" (and Enter): check it, then answer.
    fn submit(self: &Rc<Self>) {
        let Some((addr, is_pin)) = self.st.borrow().sel.as_ref().filter(|s| s.stage == "passkey" || s.stage == "pin").map(|s| (s.addr.clone(), s.stage == "pin")) else {
            return;
        };
        let Some(row) = self.rows.borrow().get(&addr).cloned() else { return };
        let Some(entry) = row.input.borrow().clone() else { return };
        let raw = entry.text().trim().to_owned();
        // a passkey is always the six digits the device shows (leading zeros too); "205 117" is fine
        let v: String = if is_pin { raw } else { raw.chars().filter(|c| !c.is_whitespace()).collect() };
        let bad = if is_pin {
            v.is_empty() || v.chars().count() > 16 || !v.chars().all(|c| c.is_ascii_alphanumeric())
        } else {
            v.len() != 6 || !v.chars().all(|c| c.is_ascii_digit())
        };
        if bad {
            let msg = if is_pin { t("Typ een pincode van 1 tot 16 letters of cijfers.", "Type a PIN of 1 to 16 letters or digits.") } else { t("Typ de zes cijfers die het apparaat laat zien.", "Type the six digits the device shows.") };
            if let Some(s) = self.st.borrow_mut().sel.as_mut() {
                s.error = Some(msg.into());
            }
            if let Some(e) = row.err.borrow().as_ref() {
                e.set_label(msg);
                e.set_visible(true);
            }
            entry.grab_focus();
            return;
        }
        if let Some(s) = self.st.borrow_mut().sel.as_mut() {
            s.error = None;
        }
        self.reply(true, if is_pin { json!({ "pin": v }) } else { json!({ "passkey": v }) });
    }

    fn cancel_pairing(self: &Rc<Self>) {
        let (addr, stage, qid, token) = {
            let mut st = self.st.borrow_mut();
            let Some(s) = st.sel.as_mut() else { return };
            if s.cancelling {
                return;
            }
            s.cancelling = true;
            s.user_cancelled = true;
            (s.addr.clone(), s.stage.clone(), s.qid.clone(), s.token)
        };
        self.render();
        let me = self.clone();
        glib::spawn_future_local(async move {
            if QUESTIONS.contains(&stage.as_str()) {
                let mut args = json!({ "address": addr, "accept": false });
                if !qid.is_null() {
                    args["qid"] = qid;
                }
                let _ = super::call("system.bluetooth.pairReply", args).await;
            }
            let _ = super::call("system.bluetooth.cancelPair", json!({ "address": addr })).await;
            // the pair call ends with "geannuleerd" and the list returns; never hang here if it does not
            let weak = Rc::downgrade(&me);
            glib::timeout_add_local_once(Duration::from_millis(10_000), move || {
                let Some(me) = weak.upgrade() else { return };
                let drop_it = {
                    let mut st = me.st.borrow_mut();
                    let hit = !st.closed && st.sel.as_ref().is_some_and(|s| s.token == token);
                    if hit {
                        st.sel = None;
                    }
                    hit
                };
                if drop_it {
                    me.render();
                }
            });
        });
    }

    fn retry(self: &Rc<Self>) {
        let (dev, live) = {
            let mut st = self.st.borrow_mut();
            let d = st.result.take().map(|r| r.dev);
            st.step = Step::List;
            let live = st.live();
            let dev = d.and_then(|d| {
                let a = upper_addr(&d);
                st.found.iter().find(|(k, _)| *k == a).map(|(_, v)| v.clone())
            });
            (dev, live)
        };
        if live {
            // renew the search the host kept going after the failure
            self.search();
        }
        match dev {
            Some(d) if live => self.start_pair(d),
            _ => self.render(),
        }
    }

    fn turn_on(self: &Rc<Self>) {
        let wait = super::call("system.bluetooth.setPowered", json!({ "powered": true }));
        let me = self.clone();
        glib::spawn_future_local(async move {
            let r = checked(wait.await);
            if me.st.borrow().closed {
                return;
            }
            match r {
                Err(e) => {
                    if is_not_responding(&e) {
                        me.st.borrow_mut().not_ready = true;
                    } else {
                        common::show_error(&me.anchor, &e, Some(t("Bluetooth aan- of uitzetten lukte niet", "Couldn’t turn Bluetooth on or off")));
                    }
                }
                Ok(v) if !v["available"].is_null() => {
                    me.take_state(&v);
                }
                Ok(_) => {
                    if let Ok(s) = super::call("system.bluetooth.get", json!({})).await {
                        me.take_state(&s);
                    }
                }
            }
            if me.st.borrow().closed {
                return;
            }
            me.render();
            if me.st.borrow().live() {
                me.search();
            }
        });
    }

    // ---- live updates

    fn on_changed(self: &Rc<Self>, d: &Value) {
        let before = self.st.borrow().bt_usable();
        let go = self.take_state(d);
        let (step, not_ready, usable) = {
            let st = self.st.borrow();
            (st.step, st.not_ready, st.bt_usable())
        };
        if go {
            self.search();
        }
        if step == Step::List || (step == Step::Failed && not_ready) || (step == Step::Choose && usable != before) {
            self.render();
        }
    }

    fn on_reset(self: &Rc<Self>, ev: &ResetEvent) {
        if self.st.borrow().closed {
            return;
        }
        let mut search = false;
        match ev {
            ResetEvent::Busy => self.st.borrow_mut().reset_mode = Some("busy"),
            ResetEvent::Done(o) if o.ok => {
                {
                    let mut st = self.st.borrow_mut();
                    st.reset_mode = Some("ok");
                    st.not_ready = false;
                }
                if let Some(s) = &o.state {
                    self.take_state(s);
                }
                let mut st = self.st.borrow_mut();
                if st.step == Step::Failed {
                    st.step = Step::List;
                }
                search = st.step == Step::List;
            }
            ResetEvent::Done(o) => {
                let mut st = self.st.borrow_mut();
                st.reset_mode = Some("error");
                st.reset_error = o.error.clone();
            }
        }
        self.render();
        if search {
            self.search();
        }
    }

    // ---- rendering

    fn render(self: &Rc<Self>) {
        let step = {
            let st = self.st.borrow();
            if st.closed {
                return;
            }
            st.step
        };
        match step {
            Step::Choose => self.render_choose(),
            Step::List => self.render_list(),
            Step::Done | Step::Failed => self.render_result(),
        }
    }

    /// The footer's buttons (one node per button for every step); one button sits in the right half.
    fn set_footer(&self, btns: &[gtk::Button]) {
        if self.footer_btns.borrow().as_slice() == btns {
            return;
        }
        let Some(footer) = self.footer.borrow().clone() else { return };
        c::clear(&footer);
        if btns.len() == 1 {
            footer.append(&gtk::Box::new(gtk::Orientation::Horizontal, 0));
        }
        for b in btns {
            b.set_hexpand(true);
            footer.append(b);
        }
        *self.footer_btns.borrow_mut() = btns.to_vec();
    }

    fn focus_later(&self, w: &impl IsA<gtk::Widget>) {
        let w = w.as_ref().clone();
        glib::idle_add_local_once(move || {
            if w.is_mapped() && w.is_sensitive() {
                w.grab_focus();
            }
        });
    }

    fn render_choose(self: &Rc<Self>) {
        self.lead.set_visible(true);
        self.lead.set_label(t("Kies het type apparaat dat je wilt toevoegen.", "Choose the kind of device you want to add."));
        let bt_ok = self.st.borrow().bt_usable();
        let kinds = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let opt = |icon: &str, title: &str, desc: &str, enabled: bool, name: &str| -> gtk::Button {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
            let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            ic.add_css_class("st-add-kind-icon");
            ic.set_valign(gtk::Align::Center);
            ic.set_hexpand(false);
            let g = kit::icons::glyph(icon, 24);
            g.set_hexpand(true);
            g.set_halign(gtk::Align::Center);
            ic.append(&g);
            row.append(&ic);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 1);
            text.set_hexpand(true);
            text.set_valign(gtk::Align::Center);
            text.append(&c::label(title, "st-add-kind-title", true));
            text.append(&c::label(desc, "st-add-kind-desc", true));
            row.append(&text);
            if !enabled {
                let na = c::label(t("Niet beschikbaar", "Not available"), "st-add-kind-na", false);
                na.set_valign(gtk::Align::Center);
                row.append(&na);
            }
            let b = gtk::Button::new();
            b.add_css_class("st-add-kind");
            b.set_child(Some(&row));
            b.set_sensitive(enabled);
            b.set_widget_name(name);
            b
        };
        let bt = opt(
            "bluetooth",
            t("Bluetooth", "Bluetooth"),
            if bt_ok {
                t("Muizen, toetsenborden, pennen of audio- en andere soorten Bluetooth-apparaten", "Mice, keyboards, pens, or audio and other kinds of Bluetooth devices")
            } else {
                t("Er is geen Bluetooth gevonden op deze computer.", "No Bluetooth was found on this computer.")
            },
            bt_ok,
            "bt-add-kind-bluetooth",
        );
        let weak = Rc::downgrade(self);
        bt.connect_clicked(move |_| {
            if let Some(me) = weak.upgrade() {
                me.go_list();
            }
        });
        kinds.append(&bt);
        kinds.append(&opt(
            "cast",
            t("Draadloos beeldscherm of dock", "Wireless display or dock"),
            t("Draadloze beeldschermen, tv’s of pc’s die Miracast gebruiken, of draadloze docks", "Wireless monitors, TVs, or PCs that use Miracast, or wireless docks"),
            false,
            "bt-add-kind-cast",
        ));
        kinds.append(&opt(
            "plus",
            t("Alle overige", "Everything else"),
            t("Xbox-controllers met draadloze adapter, DLNA en meer", "Xbox controllers with Wireless Adapter, DLNA, and more"),
            false,
            "bt-add-kind-other",
        ));
        c::clear(&self.body);
        self.body.append(&kinds);
        self.set_footer(std::slice::from_ref(&self.cancel_btn));
        if bt_ok {
            self.focus_later(&bt);
        } else {
            self.focus_later(&self.cancel_btn);
        }
    }

    fn go_list(self: &Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.step = Step::List;
            st.list_since = Some(Instant::now());
        }
        self.render();
        self.search();
        // renewed only while the list is shown: not on "klaar" (the search stopped) or "niet gelukt" (the host
        // keeps searching up to 60 s for a retry, which renews it)
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(Duration::from_millis(SEARCH_RENEW_MS), move || {
            let Some(me) = weak.upgrade() else { return glib::ControlFlow::Break };
            let (closed, step) = {
                let st = me.st.borrow();
                (st.closed, st.step)
            };
            if closed {
                return glib::ControlFlow::Break;
            }
            if step == Step::List {
                me.search();
            }
            glib::ControlFlow::Continue
        });
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(HINT_AFTER_MS + 50), move || {
            if let Some(me) = weak.upgrade()
                && me.st.borrow().step == Step::List
            {
                me.render();
            }
        });
        self.focus_later(&self.cancel_btn);
    }

    /// The note above the list (test mode, Bluetooth off, a failed search), rebuilt only when it changes.
    fn info_bar(self: &Rc<Self>, key: &str, bar: Option<(Severity, &str, &str, Option<(&str, Rc<dyn Fn()>)>)>) {
        if *self.note_key.borrow() == key {
            return;
        }
        *self.note_key.borrow_mut() = key.to_owned();
        c::clear(&self.note_slot);
        self.note_slot.set_visible(bar.is_some());
        if let Some((sev, title, msg, action)) = bar {
            let b = InfoBar::new(sev, title, msg);
            b.set_closable(false);
            b.widget().add_css_class("st-notice");
            if let Some(a) = action {
                b.set_action(Some(a));
            }
            self.note_slot.append(b.widget());
        }
    }

    fn render_list(self: &Rc<Self>) {
        self.lead.set_visible(true);
        self.lead.set_label(t(
            "Zorg ervoor dat je apparaat is ingeschakeld en detecteerbaar is. Selecteer hieronder een apparaat om verbinding te maken.",
            "Make sure your device is turned on and discoverable. Select a device below to connect.",
        ));
        if self.list_scroll.parent().as_ref() != Some(self.body.upcast_ref()) {
            c::clear(&self.body);
            adopt(&self.body, &self.stuck.bar.widget().clone());
            self.body.append(&self.note_slot);
            self.body.append(&self.list_scroll);
            self.body.append(&self.search_row);
            self.body.append(&self.hint);
        }
        let (is_stuck, reset_mode, reset_error, off, search_err, note, devs, sel_addr, live, searching, hint) = {
            let st = self.st.borrow();
            let is_stuck = st.bt.as_ref().is_some_and(|b| b["stuck"] == true) || st.not_ready;
            let off = st.bt.as_ref().is_some_and(|b| b["powered"] != true);
            let mut devs: Vec<Value> = st.found.iter().map(|(_, d)| d.clone()).collect();
            if let Some(s) = &st.sel
                && !st.found.iter().any(|(a, _)| *a == s.addr)
            {
                devs.insert(0, s.dev.clone());
            }
            let live = st.live();
            let discovering = st.bt.as_ref().is_some_and(|b| b["discovering"] == true);
            // no spinner next to "Zoeken naar apparaten lukte niet": the host is not searching for us then
            let searching = live && st.search_err.is_none() && (discovering || st.search_wanted) && st.sel.is_none();
            let hint = live && devs.is_empty() && st.list_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(HINT_AFTER_MS));
            (
                is_stuck,
                st.reset_mode,
                st.reset_error.clone(),
                off,
                st.search_err.clone(),
                st.note.clone(),
                devs,
                st.sel.as_ref().map(|s| s.addr.clone()),
                live,
                searching,
                hint,
            )
        };
        self.stuck.set_mode(
            match reset_mode {
                Some("busy") => Some("busy"),
                Some("error") if is_stuck => Some("error"),
                _ if is_stuck => Some("stuck"),
                _ => None,
            },
            &reset_error,
        );
        if is_stuck || reset_mode == Some("busy") {
            self.info_bar("", None);
        } else if off {
            let weak = Rc::downgrade(self);
            let on: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(me) = weak.upgrade() {
                    me.turn_on();
                }
            });
            self.info_bar(
                "off",
                Some((Severity::Warning, t("Bluetooth staat uit", "Bluetooth is off"), t("Zet Bluetooth aan om apparaten te zoeken.", "Turn on Bluetooth to look for devices."), Some((t("Aanzetten", "Turn on"), on)))),
            );
        } else if let Some(e) = &search_err {
            let weak = Rc::downgrade(self);
            let again: Rc<dyn Fn()> = Rc::new(move || {
                if let Some(me) = weak.upgrade() {
                    me.search();
                }
            });
            self.info_bar(&format!("err|{e}"), Some((Severity::Error, "", e, Some((t("Opnieuw proberen", "Try again"), again)))));
        } else if let Some(n) = &note {
            self.info_bar(&format!("note|{n}"), Some((Severity::Info, "", n, None)));
        } else {
            self.info_bar("", None);
        }

        // rows: every nearby device (first-seen order) and the one being paired
        let mut keep: Vec<String> = Vec::new();
        let mut prev: Option<gtk::Widget> = None;
        for d in &devs {
            let a = upper_addr(d);
            let row = self.rows.borrow().get(&a).cloned();
            let row = match row {
                Some(r) => r,
                None => {
                    let r = self.make_row(&a);
                    self.rows.borrow_mut().insert(a.clone(), r.clone());
                    r
                }
            };
            self.update_row(&row, d, sel_addr.as_deref() == Some(a.as_str()), live);
            if row.row.parent().is_none() {
                self.list.append(&row.row);
            }
            self.list.reorder_child_after(&row.row, prev.as_ref());
            prev = Some(row.row.clone().upcast());
            keep.push(a);
        }
        self.rows.borrow_mut().retain(|a, r| {
            let k = keep.contains(a);
            if !k {
                self.list.remove(&r.row);
            }
            k
        });
        self.list_scroll.set_visible(!devs.is_empty());
        self.search_row.set_visible(searching);
        self.hint.set_visible(hint);
        self.set_footer(std::slice::from_ref(&self.cancel_btn));
        self.keep_focus();
    }

    /// The focused control may have gone (a rebuilt panel) or be disabled now (the picked row's head): keep the
    /// focus in the dialog, on the question's control, the progress panel, the row picked last or Annuleren.
    fn keep_focus(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            let Some(me) = weak.upgrade() else { return };
            let (sel, last) = {
                let st = me.st.borrow();
                if st.closed || st.step != Step::List {
                    return;
                }
                (st.sel.as_ref().map(|s| s.addr.clone()), st.last_sel.clone())
            };
            let Some(window) = me.anchor.root().and_downcast::<gtk::Window>() else { return };
            let frame = me.frame.borrow().clone();
            if let (Some(f), Some(fr)) = (GtkWindowExt::focus(&window), frame.as_ref())
                && f.is_ancestor(fr)
                && f.is_sensitive()
                && f.is_mapped()
            {
                return;
            }
            let rows = me.rows.borrow();
            let target: gtk::Widget = match sel.and_then(|a| rows.get(&a).cloned()) {
                Some(r) => {
                    let input = r.input.borrow().clone().map(|i| i.upcast::<gtk::Widget>());
                    input.or_else(|| first_button(&r.panel)).unwrap_or_else(|| r.panel.clone().upcast())
                }
                None => last
                    .and_then(|a| rows.get(&a).filter(|r| r.head.is_sensitive()).map(|r| r.head.clone().upcast()))
                    .or_else(|| rows.values().find(|r| r.head.is_sensitive() && r.head.is_mapped()).map(|r| r.head.clone().upcast()))
                    .unwrap_or_else(|| me.cancel_btn.clone().upcast()),
            };
            target.grab_focus();
        });
    }

    fn make_row(self: &Rc<Self>, addr: &str) -> Rc<DevRow> {
        let icon = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        icon.add_css_class("st-add-dev-icon");
        icon.set_valign(gtk::Align::Center);
        icon.set_hexpand(false);
        let name = c::label("", "st-add-dev-name", false);
        let sub = c::label("", "st-add-dev-sub", false);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&name);
        text.append(&sub);
        let head_box = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        head_box.append(&icon);
        head_box.append(&text);
        let head = gtk::Button::new();
        head.add_css_class("st-add-dev-head");
        head.set_child(Some(&head_box));
        head.set_widget_name(&format!("bt-add-dev-{addr}"));
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 10);
        panel.add_css_class("st-add-panel");
        // holds the focus while a step has no control (progress)
        panel.set_focusable(true);
        panel.set_visible(false);
        let row = gtk::Box::new(gtk::Orientation::Vertical, 0);
        row.add_css_class("st-add-dev");
        row.set_widget_name(&format!("bt-add-row-{addr}"));
        row.append(&head);
        row.append(&panel);
        let weak = Rc::downgrade(self);
        let a = addr.to_owned();
        head.connect_clicked(move |_| {
            let Some(me) = weak.upgrade() else { return };
            let d = me.st.borrow().found.iter().find(|(k, _)| *k == a).map(|(_, d)| d.clone());
            if let Some(d) = d {
                me.start_pair(d);
            }
        });
        Rc::new(DevRow {
            row,
            head,
            icon,
            name,
            sub,
            panel,
            panel_key: RefCell::new(None),
            kind: RefCell::new("?".into()),
            input: RefCell::new(None),
            err: RefCell::new(None),
            code: RefCell::new(None),
        })
    }

    fn update_row(self: &Rc<Self>, r: &Rc<DevRow>, d: &Value, is_sel: bool, live: bool) {
        let kind_key = format!("{}|{}", s(&d["kind"]), s(&d["icon"]));
        if *r.kind.borrow() != kind_key {
            *r.kind.borrow_mut() = kind_key;
            c::clear(&r.icon);
            let i = device_icon(d, 24);
            i.set_hexpand(true);
            r.icon.append(&i);
        }
        r.name.set_label(&dev_name(d));
        let sub = kind_label(s(&d["kind"]));
        r.sub.set_label(sub);
        r.sub.set_visible(!sub.is_empty());
        // a stuck or switched-off adapter cannot pair: the host still lists what the last search found (60 s)
        let (any_sel, key, error) = {
            let st = self.st.borrow();
            let key = st.sel.as_ref().map(|s| {
                format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    s.stage,
                    s.qid,
                    s.passkey.as_deref().unwrap_or(""),
                    s.pin.as_deref().unwrap_or(""),
                    s.cancelling,
                    s.busy && s.stage != "passkey" && s.stage != "pin",
                    s.token
                )
            });
            (st.sel.is_some(), key, st.sel.as_ref().and_then(|s| s.error.clone()))
        };
        let toggle = |class: &str, on: bool| {
            if on {
                r.row.add_css_class(class);
            } else {
                r.row.remove_css_class(class);
            }
        };
        toggle("st-selected", is_sel);
        toggle("st-dim", !is_sel && (any_sel || !live));
        r.head.set_sensitive(!any_sel && live);
        if !is_sel {
            if r.panel_key.borrow().is_some() {
                c::clear(&r.panel);
                *r.panel_key.borrow_mut() = None;
                *r.input.borrow_mut() = None;
                *r.err.borrow_mut() = None;
                *r.code.borrow_mut() = None;
            }
            r.panel.set_visible(false);
            return;
        }
        r.panel.set_visible(true);
        if *r.panel_key.borrow() != key {
            *r.panel_key.borrow_mut() = key;
            c::clear(&r.panel);
            for w in self.panel_for(r) {
                r.panel.append(&w);
            }
        }
        if let Some(e) = r.err.borrow().as_ref() {
            e.set_label(error.as_deref().unwrap_or(""));
            e.set_visible(error.is_some());
        }
    }

    fn panel_for(self: &Rc<Self>, r: &Rc<DevRow>) -> Vec<gtk::Widget> {
        *r.input.borrow_mut() = None;
        *r.err.borrow_mut() = None;
        *r.code.borrow_mut() = None;
        let Some((name, stage, passkey, pin, entered, busy, cancelling, error)) = self.st.borrow().sel.as_ref().map(|s| {
            (dev_name(&s.dev), s.stage.clone(), s.passkey.clone(), s.pin.clone(), s.entered, s.busy, s.cancelling, s.error.clone())
        }) else {
            return Vec::new();
        };
        let progress = |text: &str| -> Vec<gtk::Widget> {
            let bar = ProgressBar::new();
            bar.set_indeterminate(true);
            bar.widget().set_tooltip_text(Some(text));
            vec![c::label(text, "st-add-status", true).upcast(), bar.widget().clone().upcast()]
        };
        if cancelling {
            return progress(t("Annuleren…", "Cancelling…"));
        }
        let actions = |btns: &[&gtk::Button]| -> gtk::Widget {
            let b = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            b.add_css_class("st-add-actions");
            for x in btns {
                b.append(*x);
            }
            b.upcast()
        };
        let cancel = c::button(t("Annuleren", "Cancel"), Variant::Standard);
        cancel.set_widget_name("bt-add-pair-cancel");
        {
            let weak = Rc::downgrade(self);
            cancel.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.cancel_pairing();
                }
            });
        }
        let text = |s: &str| -> gtk::Widget { c::label(s, "st-add-text", true).upcast() };
        let err = c::label(error.as_deref().unwrap_or(""), "st-add-err", true);
        err.set_visible(error.is_some());
        match stage.as_str() {
            "confirm" => {
                // the code shown goes along with the yes: the host checks it against the question it holds
                let ok = c::button(t("Verbinden", "Connect"), Variant::Accent);
                ok.set_widget_name("bt-add-connect");
                ok.set_sensitive(!busy);
                let weak = Rc::downgrade(self);
                let pk = passkey.clone();
                ok.connect_clicked(move |_| {
                    if let Some(me) = weak.upgrade() {
                        me.reply(true, pk.as_ref().map(|p| json!({ "passkey": p })).unwrap_or_else(|| json!({})));
                    }
                });
                self.focus_later(&ok);
                *r.err.borrow_mut() = Some(err.clone());
                match &passkey {
                    Some(code) => vec![
                        text(t("Druk op Verbinden als de pincode op je apparaat overeenkomt.", "Press Connect if the PIN on your device matches this one.")),
                        CodeView::new(code, None).root.upcast(),
                        err.upcast(),
                        actions(&[&ok, &cancel]),
                    ],
                    None => vec![
                        text(&common::fill(
                            t("“{name}” wil verbinding maken met deze pc. Druk op Verbinden om dat toe te staan.", "“{name}” wants to connect to this PC. Press Connect to allow it."),
                            &[("name", &name)],
                        )),
                        err.upcast(),
                        actions(&[&ok, &cancel]),
                    ],
                }
            }
            "passkey" | "pin" => {
                let is_pin = stage == "pin";
                let entry = c::text_box(t("Pincode", "PIN"));
                entry.set_max_length(if is_pin { 16 } else { 7 });
                entry.set_input_purpose(if is_pin { gtk::InputPurpose::FreeForm } else { gtk::InputPurpose::Digits });
                entry.set_halign(gtk::Align::Start);
                entry.set_size_request(296, -1);
                entry.set_widget_name("bt-add-pin");
                let ok = c::button(t("Verbinden", "Connect"), Variant::Accent);
                ok.set_widget_name("bt-add-connect");
                {
                    let weak = Rc::downgrade(self);
                    ok.connect_clicked(move |_| {
                        if let Some(me) = weak.upgrade() {
                            me.submit();
                        }
                    });
                }
                {
                    let (weak, err) = (Rc::downgrade(self), err.clone());
                    entry.connect_changed(move |_| {
                        let Some(me) = weak.upgrade() else { return };
                        let had = me.st.borrow_mut().sel.as_mut().and_then(|s| s.error.take()).is_some();
                        if had {
                            err.set_visible(false);
                        }
                    });
                }
                *r.input.borrow_mut() = Some(entry.clone());
                *r.err.borrow_mut() = Some(err.clone());
                self.focus_later(&entry);
                let msg = if is_pin {
                    t(
                        "Voer de pincode voor “{name}” in. Staat die niet op het apparaat of in de handleiding, probeer dan 0000 of 1234.",
                        "Enter the PIN for “{name}”. If it isn’t on the device or in its manual, try 0000 or 1234.",
                    )
                } else {
                    t("Voer de pincode in die op “{name}” wordt weergegeven.", "Enter the PIN that “{name}” shows.")
                };
                let col = gtk::Box::new(gtk::Orientation::Vertical, 4);
                col.append(&entry);
                col.append(&err);
                vec![text(&common::fill(msg, &[("name", &name)])), col.upcast(), actions(&[&ok, &cancel])]
            }
            "display" => {
                let code = CodeView::new(pin.as_deref().or(passkey.as_deref()).unwrap_or(""), if pin.is_some() { None } else { entered });
                *r.code.borrow_mut() = Some(code.clone());
                let msg = if pin.is_some() {
                    t("Typ deze pincode op “{name}”.", "Type this PIN on “{name}”.")
                } else {
                    t("Typ deze pincode op “{name}” en druk daarna op Enter.", "Type this PIN on “{name}”, then press Enter.")
                };
                self.focus_later(&cancel);
                vec![text(&common::fill(msg, &[("name", &name)])), code.root.upcast(), actions(&[&cancel])]
            }
            "connecting" => progress(t("Gekoppeld. Verbinding maken…", "Paired. Connecting…")),
            _ => progress(t("Verbinding maken…", "Connecting…")),
        }
    }

    /// Done / failed: built once per outcome; a live update (failed and "reageert niet") only refreshes the
    /// reset banner and the retry button, so focus and a click in progress survive.
    fn render_result(self: &Rc<Self>) {
        self.lead.set_visible(false);
        let (ok, dev, connected, error, id, is_stuck, reset_mode, reset_error, live) = {
            let st = self.st.borrow();
            let ok = st.step == Step::Done;
            let (dev, connected, error, id) = match &st.result {
                Some(r) => (r.dev.clone(), r.connected, r.error.clone(), r.id),
                None => (json!({}), false, None, 0),
            };
            let is_stuck = !ok && (st.not_ready || st.bt.as_ref().is_some_and(|b| b["stuck"] == true));
            (ok, dev, connected, error, id, is_stuck, st.reset_mode, st.reset_error.clone(), st.live())
        };
        if is_stuck {
            self.stuck.set_mode(
                Some(match reset_mode {
                    Some("busy") => "busy",
                    Some("error") => "error",
                    _ => "stuck",
                }),
                &reset_error,
            );
        }
        let again_off = is_stuck && reset_mode != Some("ok") && !live;
        let key = format!("{ok}|{connected}|{}|{is_stuck}", error.as_deref().unwrap_or(""));
        let same = self.result_el.borrow().as_ref().is_some_and(|(i, k, w)| *i == id && *k == key && w.parent().as_ref() == Some(self.body.upcast_ref()));
        if same {
            if let Some(b) = self.again_btn.borrow().as_ref() {
                b.set_sensitive(!again_off);
            }
            return;
        }
        let res = gtk::Box::new(gtk::Orientation::Vertical, 14);
        res.add_css_class("st-add-result");
        res.add_css_class(if ok { "st-done" } else { "st-failed" });
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        head.append(&kit::icons::severity_icon(if ok { "success" } else { "error" }, 24));
        let title = c::label(
            if ok { t("Je apparaat is klaar om te gebruiken!", "Your device is ready to go!") } else { t("Verbinden is niet gelukt", "Couldn’t connect") },
            "st-add-result-title",
            true,
        );
        title.set_widget_name("bt-add-result");
        head.append(&title);
        res.append(&head);
        if !ok {
            res.append(&c::label(error.as_deref().unwrap_or(t("Koppelen lukte niet. Probeer het opnieuw.", "Pairing failed. Try again.")), "st-add-result-msg", true));
        }
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 14);
        card.add_css_class("st-add-devcard");
        if ok && connected {
            card.add_css_class("st-connected");
        }
        let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ic.add_css_class("st-add-dev-icon");
        ic.set_valign(gtk::Align::Center);
        ic.set_hexpand(false);
        let i = device_icon(&dev, 24);
        i.set_hexpand(true);
        ic.append(&i);
        card.append(&ic);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_valign(gtk::Align::Center);
        text.set_hexpand(true);
        text.append(&c::label(&dev_name(&dev), "st-add-dev-name", false));
        let status = if ok {
            if connected { t("Verbonden", "Connected") } else { t("Gekoppeld", "Paired") }
        } else {
            kind_label(s(&dev["kind"]))
        };
        if !status.is_empty() {
            text.append(&c::label(status, "st-add-dev-sub", false));
        }
        card.append(&text);
        res.append(&card);
        if ok && let Some(e) = &error {
            res.append(&c::label(e, "st-add-result-note", true));
        }
        if is_stuck {
            adopt(&res, &self.stuck.bar.widget().clone());
        }
        c::clear(&self.body);
        self.body.append(&res);
        *self.result_el.borrow_mut() = Some((id, key, res.upcast()));
        if ok {
            *self.again_btn.borrow_mut() = None;
            let done = c::button(t("Gereed", "Done"), Variant::Accent);
            done.set_widget_name("bt-add-done");
            let weak = Rc::downgrade(self);
            done.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.dialog.close(DialogResult::Close);
                }
            });
            self.set_footer(std::slice::from_ref(&done));
            self.focus_later(&done);
        } else {
            let again = c::button(t("Opnieuw proberen", "Try again"), Variant::Accent);
            again.set_widget_name("bt-add-retry");
            again.set_sensitive(!again_off);
            let weak = Rc::downgrade(self);
            again.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.retry();
                }
            });
            *self.again_btn.borrow_mut() = Some(again.clone());
            self.set_footer(&[again.clone(), self.cancel_btn.clone()]);
            if again_off {
                self.focus_later(&self.cancel_btn);
            } else {
                self.focus_later(&again);
            }
        }
    }
}

fn s_owned(v: &Value) -> String {
    s(v).to_owned()
}

/// The first button inside a widget.
fn first_button(w: &impl IsA<gtk::Widget>) -> Option<gtk::Widget> {
    let mut child = w.as_ref().first_child();
    while let Some(c) = child {
        if c.is::<gtk::Button>() && c.is_sensitive() {
            return Some(c);
        }
        if let Some(b) = first_button(&c) {
            return Some(b);
        }
        child = c.next_sibling();
    }
    None
}

// ------------------------------------------------------------------------------------------------- page

/// `?add=1`: a value that is not 0, false, no or empty.
fn wants_add(v: &Value) -> bool {
    let text = match v {
        Value::Null => return false,
        Value::String(s) => s.to_lowercase(),
        other => other.to_string(),
    };
    !matches!(text.as_str(), "0" | "false" | "no" | "")
}

struct BtPage {
    p: Page,
    state: RefCell<Value>,
    not_ready: Cell<bool>,
    banner_mode: Cell<Option<&'static str>>,
    banner_error: RefCell<String>,
    ok_gen: Cell<u64>,
    busy: RefCell<HashMap<String, &'static str>>,
    grid: FillGrid,
    name: String,
    banner: Rc<StuckBar>,
    empty_note: InfoBar,
    toggle: ToggleSwitch,
    bt_card: SettingsCard,
    reset_btn: gtk::Button,
}

impl BtPage {
    fn failed(self: &Rc<Self>, e: &CallError, title: Option<&str>) {
        if is_not_responding(e) {
            self.not_ready.set(true);
            self.paint();
            return;
        }
        common::show_error(&self.p.anchor(), e, title);
    }

    async fn reload(self: &Rc<Self>) {
        let r = self.p.get("system.bluetooth.get", json!({})).await;
        if let Some(r) = r
            && self.p.alive()
        {
            if r["stuck"] == false {
                self.not_ready.set(false);
            }
            *self.state.borrow_mut() = r;
            self.paint();
        }
    }

    fn act(self: &Rc<Self>, d: Value) {
        let addr = text_of(&d["address"]);
        let what = if d["connected"] == true { "disconnect" } else { "connect" };
        self.busy.borrow_mut().insert(addr.clone(), what);
        self.paint();
        let me = self.clone();
        self.p.spawn(async move {
            let r = attempt(&me.p, &format!("system.bluetooth.{what}"), json!({ "address": d["address"] })).await;
            me.busy.borrow_mut().remove(&addr);
            if !me.p.alive() {
                return;
            }
            if let Err(e) = r {
                me.failed(&e, None);
            }
            me.reload().await;
            if me.p.alive() {
                me.paint();
            }
        });
    }

    fn remove(self: &Rc<Self>, d: Value) {
        let addr = text_of(&d["address"]);
        if self.busy.borrow().contains_key(&addr) {
            return;
        }
        let me = self.clone();
        self.p.spawn(async move {
            // "Verwijderen…" only once the user said "Ja" (like Windows), not behind the question
            let (m2, a2) = (me.clone(), addr.clone());
            let r = remove_device(&me.p, &d, move || {
                m2.busy.borrow_mut().insert(a2, "remove");
                if m2.p.alive() {
                    m2.paint();
                }
            })
            .await;
            let Some(r) = r else { return };
            me.busy.borrow_mut().remove(&addr);
            if !me.p.alive() {
                return;
            }
            if let Err(e) = r {
                me.failed(&e, None);
            }
            me.reload().await;
            if me.p.alive() {
                me.paint();
            }
        });
    }

    fn device_tile(self: &Rc<Self>, d: &Value) -> gtk::Widget {
        let addr = text_of(&d["address"]);
        let state = self.busy.borrow().get(&addr).copied();
        let status_text = match state {
            Some("connect") => t("Verbinden…", "Connecting…"),
            Some("disconnect") => t("Verbinding verbreken…", "Disconnecting…"),
            Some(_) => t("Verwijderen…", "Removing…"),
            None if d["connected"] == true => t("Verbonden", "Connected"),
            None if d["paired"] == true => t("Gekoppeld", "Paired"),
            None => t("Niet verbonden", "Not connected"),
        };
        let tile = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        tile.add_css_class("st-dev");
        if d["connected"] == true {
            tile.add_css_class("st-connected");
        }
        if state.is_some() {
            tile.add_css_class("st-busy");
        }
        tile.set_focusable(true);
        tile.set_widget_name(&format!("bt-dev-{addr}"));
        tile.update_property(&[gtk::accessible::Property::Label(&format!("{}: {}", dev_name(d), device_status(d)))]);
        let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ic.add_css_class("st-dev-icon");
        ic.set_valign(gtk::Align::Center);
        ic.set_hexpand(false);
        let i = device_icon(d, 28);
        i.set_hexpand(true);
        ic.append(&i);
        tile.append(&ic);
        let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.append(&c::label(&dev_name(d), "st-dev-name", false));
        let status = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        status.add_css_class("st-dev-status");
        status.append(&c::label(status_text, "", false));
        if state.is_none()
            && let Some(b) = d["battery"].as_f64()
        {
            let n = b.round() as i64;
            let bat = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            bat.update_property(&[gtk::accessible::Property::Label(&t("Batterij {n}%", "Battery {n}%").replace("{n}", &n.to_string()))]);
            bat.append(&kit::icons::glyph("battery", 16));
            bat.append(&c::label(&format!("{n}%"), "", false));
            status.append(&bat);
        }
        text.append(&status);
        tile.append(&text);
        let more = c::icon_button("more", t("Meer opties", "More options"), 32);
        more.add_css_class("st-dev-more");
        more.set_valign(gtk::Align::Start);
        more.set_sensitive(state.is_none());
        more.set_widget_name(&format!("bt-dev-more-{addr}"));
        {
            let (me, dev) = (self.clone(), d.clone());
            more.connect_clicked(move |b| {
                let connected = dev["connected"] == true;
                let (m1, d1, m2, d2) = (me.clone(), dev.clone(), me.clone(), dev.clone());
                c::show_menu_for(
                    b,
                    vec![
                        MenuItem::new(if connected { t("Verbinding verbreken", "Disconnect") } else { t("Verbinden", "Connect") }, move || m1.act(d1.clone()))
                            .glyph(if connected { "close" } else { "link" })
                            .name("bt-dev-toggle"),
                        MenuItem::separator(),
                        MenuItem::new(t("Apparaat verwijderen", "Remove device"), move || m2.remove(d2.clone())).glyph("trash").name("bt-dev-remove"),
                    ],
                );
            });
        }
        tile.append(&more);
        {
            let (me, dev, addr) = (self.clone(), d.clone(), addr.clone());
            let click = gtk::GestureClick::new();
            click.connect_pressed(move |_, n, _, _| {
                if n == 2 && !me.busy.borrow().contains_key(&addr) {
                    me.act(dev.clone());
                }
            });
            tile.add_controller(click);
        }
        {
            let (me, dev, addr, tile2) = (self.clone(), d.clone(), addr.clone(), tile.clone());
            let keys = gtk::EventControllerKey::new();
            keys.connect_key_pressed(move |_, key, _, _| {
                let focus = tile2.root().and_then(|r| r.focus());
                if focus.as_ref() != Some(tile2.upcast_ref()) || !matches!(key, gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space) {
                    return glib::Propagation::Proceed;
                }
                if !me.busy.borrow().contains_key(&addr) {
                    me.act(dev.clone());
                }
                glib::Propagation::Stop
            });
            tile.add_controller(keys);
        }
        tile.upcast()
    }

    fn paint(self: &Rc<Self>) {
        let st = self.state.borrow().clone();
        let stuck = st["stuck"] == true || self.not_ready.get();
        if self.banner_mode.get() == Some("ok") && stuck {
            self.banner_mode.set(None);
        }
        let mode = match self.banner_mode.get() {
            Some(m) => Some(m),
            None if stuck => Some("stuck"),
            None => None,
        };
        self.banner.set_mode(mode, &self.banner_error.borrow());
        let busy_reset = self.banner_mode.get() == Some("busy");
        let powered = st["powered"] == true;
        self.toggle.set_active(powered);
        self.toggle.set_sensitive(!busy_reset);
        self.reset_btn.set_sensitive(!busy_reset);
        self.bt_card.set_description(&if stuck {
            t("Bluetooth reageert niet", "Bluetooth is not responding").to_owned()
        } else if powered {
            common::fill(t("Detecteerbaar als “{name}”", "Discoverable as “{name}”"), &[("name", &self.name)])
        } else {
            t("Bluetooth staat uit", "Bluetooth is off").to_owned()
        });
        self.grid.remove_all();
        let add_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        let ic = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        ic.add_css_class("st-dev-icon");
        ic.set_valign(gtk::Align::Center);
        ic.set_hexpand(false);
        let g = kit::icons::glyph("plus", 24);
        g.set_hexpand(true);
        ic.append(&g);
        add_row.append(&ic);
        let l = c::label(t("Apparaat toevoegen", "Add device"), "st-dev-name", false);
        l.set_valign(gtk::Align::Center);
        add_row.append(&l);
        let add = gtk::Button::new();
        add.add_css_class("st-dev");
        add.add_css_class("st-dev-add");
        add.set_child(Some(&add_row));
        add.set_widget_name("bt-add");
        {
            let me = self.clone();
            add.connect_clicked(move |_| {
                let me2 = me.clone();
                open_add_device_then(
                    &me.p,
                    Some(Box::new(move || {
                        let m3 = me2.clone();
                        me2.p.spawn(async move { m3.reload().await });
                    })),
                );
            });
        }
        self.grid.append(&add);
        let mut devs: Vec<Value> = if powered { st["devices"].as_array().cloned().unwrap_or_default() } else { Vec::new() };
        devs.sort_by(|a, b| (b["connected"] == true).cmp(&(a["connected"] == true)).then_with(|| kit::format::compare_natural(&dev_name(a), &dev_name(b))));
        for d in &devs {
            let mut d = d.clone();
            if stuck {
                d["connected"] = json!(false);
            }
            self.grid.append(&self.device_tile(&d));
        }
        self.empty_note.set_visible(powered && devs.is_empty());
    }
}

fn page(p: Page) -> PageFuture {
    Box::pin(async move {
        let (br, info) = (p.call("system.bluetooth.get", json!({})), p.get("system.info", json!({})));
        let (br, info) = (br.await, info.await);
        if !p.alive() {
            return Content::new();
        }
        let bt = br.ok().filter(|v| v.is_object());
        // deep link ?add=1: once (a refresh of this page must not open the dialog again)
        if p.take_param("add").is_some_and(|v| wants_add(&v)) {
            let pp = p.clone();
            p.timeout(280, move || {
                let p2 = pp.clone();
                open_add_device_then(&pp, Some(Box::new(move || p2.refresh())));
            });
        }
        let Some(bt) = bt.filter(|b| b["available"] == true) else {
            let empty = c::empty_state(
                "bluetooth",
                t("Er is geen Bluetooth gevonden op deze computer.", "No Bluetooth was found on this computer."),
                t("Heeft je computer geen Bluetooth, dan kun je een kleine Bluetooth-USB-stick gebruiken.", "If your computer has no Bluetooth, you can use a small Bluetooth USB adapter."),
                None,
            );
            let card = SettingsCard::clickable(
                "plus",
                t("Apparaat toevoegen", "Add device"),
                t("Muizen, toetsenborden, pennen of audio- en andere soorten Bluetooth-apparaten", "Mice, keyboards, pens, or audio and other kinds of Bluetooth devices"),
                Chevron::Right,
            );
            if let Some(b) = card.button() {
                b.set_widget_name("bt-add");
            }
            let pp = p.clone();
            card.connect_clicked(move || open_add_device(&pp));
            let group = SettingsGroup::new("", "");
            group.add(card.widget());
            return vec![empty.upcast(), group.widget().clone().upcast()];
        };
        let name = info.as_ref().and_then(|i| i["hostname"].as_str()).filter(|s| !s.is_empty()).unwrap_or(t("deze pc", "this PC")).to_owned();

        let toggle = ToggleSwitch::new(bt["powered"] == true);
        toggle.switch().set_widget_name("bt-toggle");
        let bt_card = SettingsCard::new("bluetooth", t("Bluetooth", "Bluetooth"), "");
        bt_card.set_control(toggle.widget());
        let reset_btn = c::button(t("Opnieuw starten", "Restart"), Variant::Standard);
        reset_btn.set_widget_name("bt-reset");
        reset_btn.connect_clicked(|_| start_reset());
        let reset_card = SettingsCard::new(
            "restart",
            t("Bluetooth opnieuw starten", "Restart Bluetooth"),
            t("Helpt als Bluetooth niet meer reageert of apparaten geen verbinding meer maken", "Helps when Bluetooth stops responding or devices no longer connect"),
        );
        reset_card.set_control(&reset_btn);
        let grid = FillGrid::new(264, 8, 8, 0);
        grid.add_css_class("st-dev-grid");
        grid.set_widget_name("bt-devices");
        let empty_note = common::notice_full(
            Severity::Info,
            t("Nog geen apparaten", "No devices yet"),
            t(
                "Kies “Apparaat toevoegen” om een koptelefoon, muis, toetsenbord of ander Bluetooth-apparaat te koppelen.",
                "Choose “Add device” to pair headphones, a mouse, a keyboard or another Bluetooth device.",
            ),
        );
        let page = Rc::new(BtPage {
            p: p.clone(),
            state: RefCell::new(bt),
            not_ready: Cell::new(false),
            banner_mode: Cell::new(if reset_running() { Some("busy") } else { None }),
            banner_error: RefCell::new(String::new()),
            ok_gen: Cell::new(0),
            busy: RefCell::new(HashMap::new()),
            grid: grid.clone(),
            name,
            banner: StuckBar::new(false),
            empty_note: empty_note.clone(),
            toggle: toggle.clone(),
            bt_card: bt_card.clone(),
            reset_btn,
        });
        {
            let pg = Rc::downgrade(&page);
            toggle.connect_toggled(move |on| {
                let Some(pg) = pg.upgrade() else { return };
                pg.toggle.set_sensitive(false);
                let me = pg.clone();
                pg.p.spawn(async move {
                    let r = attempt(&me.p, "system.bluetooth.setPowered", json!({ "powered": on })).await;
                    if !me.p.alive() {
                        return;
                    }
                    me.toggle.set_sensitive(true);
                    match r {
                        Err(e) => {
                            me.toggle.set_active(!on);
                            me.failed(&e, Some(t("Bluetooth aan- of uitzetten lukte niet", "Couldn’t turn Bluetooth on or off")));
                            return;
                        }
                        Ok(v) if !v["available"].is_null() => *me.state.borrow_mut() = v,
                        Ok(_) => {
                            let mut st = me.state.borrow_mut();
                            st["powered"] = json!(on);
                            st["powerState"] = json!(if on { "on" } else { "off" });
                        }
                    }
                    me.paint();
                });
            });
        }
        {
            let weak: Weak<BtPage> = Rc::downgrade(&page);
            let id = reset_subscribe(move |ev| {
                let Some(me) = weak.upgrade() else { return };
                if !me.p.alive() {
                    return;
                }
                me.ok_gen.set(me.ok_gen.get() + 1);
                match ev {
                    ResetEvent::Busy => me.banner_mode.set(Some("busy")),
                    ResetEvent::Done(o) if o.ok => {
                        me.not_ready.set(false);
                        if let Some(s) = &o.state {
                            *me.state.borrow_mut() = s.clone();
                        }
                        me.banner_mode.set(Some("ok"));
                        let (stamp, m2) = (me.ok_gen.get(), me.clone());
                        me.p.timeout(6000, move || {
                            if m2.ok_gen.get() == stamp && m2.banner_mode.get() == Some("ok") {
                                m2.banner_mode.set(None);
                                m2.paint();
                            }
                        });
                        if o.state.is_none() {
                            let m3 = me.clone();
                            me.p.spawn(async move { m3.reload().await });
                        }
                    }
                    ResetEvent::Done(o) => {
                        me.banner_mode.set(Some("error"));
                        *me.banner_error.borrow_mut() = o.error.clone();
                    }
                }
                me.paint();
            });
            p.on_cleanup(move || reset_unsubscribe(id));
        }
        {
            let pg = page.clone();
            p.on("system.bluetooth.changed", move |d| {
                if !d.is_object() || d["available"].is_null() {
                    return;
                }
                *pg.state.borrow_mut() = d.clone();
                if !d["stuck"].is_null() {
                    // the host's own verdict (it counts NotReady answers too)
                    pg.not_ready.set(false);
                }
                if pg.banner_mode.get() == Some("error") && d["stuck"] != true {
                    pg.banner_mode.set(None);
                }
                pg.paint();
            });
        }
        page.paint();

        let main = SettingsGroup::new("", "");
        main.add(bt_card.widget());
        let related = SettingsGroup::new(common::ct::related(), "");
        related.add(common::link_card_with(&p, "sound", None, Some(t("Kies je koptelefoon of speaker als geluidsapparaat", "Choose your headphones or speaker as the sound device")), None).widget());
        related.add(reset_card.widget());
        vec![
            page.banner.bar.widget().clone().upcast(),
            grid.upcast(),
            empty_note.widget().clone().upcast(),
            main.widget().clone().upcast(),
            related.widget().clone().upcast(),
        ]
    })
}
