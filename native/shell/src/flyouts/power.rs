//! Power confirmation (sign out, shut down, restart, sleep, lock), ported from
//! `git show 418ea594:vendor/ui/js/shell/power.js`: Start hands the action to the panel (D-G58, panel-power), which
//! shows the confirmation over the whole monitor.
//!
//!   confirm(shell, action)   shutdown / reboot / logout: a Win11 ContentDialog on a smoke over the whole monitor
//!                            ("Wil je de pc afsluiten?", what happens to unsaved work, accent button and
//!                            Annuleren) when settings power.confirm is on; Esc, Annuleren or another flyout cancel.
//!                            suspend runs without asking; lock runs at once (no dialog, no screen).
//!   then                     the full-screen "Afsluiten…" screen fades in and system.power.action {action} runs;
//!                            in test mode (dryRun) the screen says nothing was switched off and goes away again.
//!
//! The dialog and the screen are the flyout "power", stretched over the panel window's whole overlay.
use crate::core::Shell;
use crate::flyouts::{Flyout, Placement};
use gtk::prelude::*;
use gtk::{gdk, glib, pango};
use serde_json::{Value, json};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

const TEXTS: &[(&str, &str, &str)] = &[
    ("shutdownTitle", "Wil je de pc afsluiten?", "Do you want to shut down the PC?"),
    ("rebootTitle", "Wil je de pc opnieuw opstarten?", "Do you want to restart the PC?"),
    ("logoutTitle", "Wil je je afmelden?", "Do you want to sign out?"),
    ("suspendTitle", "Wil je de slaapstand gebruiken?", "Do you want to put the PC to sleep?"),
    ("lockTitle", "Wil je de pc vergrendelen?", "Do you want to lock the PC?"),
    (
        "shutdownBody",
        "Alle apps worden gesloten. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
        "All apps will close. Any work you haven’t saved might be lost.",
    ),
    (
        "rebootBody",
        "Alle apps worden gesloten en de pc start daarna vanzelf weer op. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
        "All apps will close and the PC will restart by itself. Any work you haven’t saved might be lost.",
    ),
    (
        "logoutBody",
        "Alle apps worden gesloten en je komt terug in het aanmeldscherm. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
        "All apps will close and you’ll return to the sign-in screen. Any work you haven’t saved might be lost.",
    ),
    ("suspendBody", "", ""),
    ("lockBody", "", ""),
    ("saveTip", "Tip: sla je werk eerst op.", "Tip: save your work first."),
    ("shutdown", "Afsluiten", "Shut down"),
    ("reboot", "Opnieuw opstarten", "Restart"),
    ("logout", "Afmelden", "Sign out"),
    ("suspend", "Slaapstand", "Sleep"),
    ("lock", "Vergrendelen", "Lock"),
    ("cancel", "Annuleren", "Cancel"),
    ("shuttingDown", "Afsluiten…", "Shutting down…"),
    ("rebooting", "Opnieuw opstarten…", "Restarting…"),
    ("loggingOut", "Afmelden…", "Signing out…"),
    ("sleeping", "Slaapstand…", "Going to sleep…"),
    ("locking", "Vergrendelen…", "Locking…"),
    ("dryRun", "Testmodus: er is niets uitgeschakeld.", "Test mode: nothing was switched off."),
    ("failedTitle", "Dat is niet gelukt", "That didn’t work"),
    ("failed", "De pc kon dit nu niet doen. Probeer het opnieuw.", "The PC couldn’t do this right now. Try again."),
];

const ACTIONS: [&str; 5] = ["shutdown", "reboot", "logout", "suspend", "lock"];

fn sh() -> Rc<Shell> {
    crate::core::shell()
}

fn t(key: &str) -> String {
    TEXTS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, nl, en)| sh().t(nl, en))
        .unwrap_or_else(|| key.to_owned())
}

fn progress_key(action: &str) -> &'static str {
    match action {
        "reboot" => "rebooting",
        "logout" => "loggingOut",
        "suspend" => "sleeping",
        "lock" => "locking",
        _ => "shuttingDown",
    }
}

fn host() -> Option<Rc<crate::flyouts::FlyoutHost>> {
    sh().flyouts.borrow().clone()
}

struct Power {
    root: gtk::Stack,
    dialog: gtk::Box,
    title: gtk::Label,
    body: gtk::Label,
    tip: gtk::Label,
    primary: gtk::Button,
    cancel: gtk::Button,
    screen: gtk::Box,
    screen_label: gtk::Label,
    note: gtk::Label,
    dots: gtk::DrawingArea,
    pending: RefCell<Option<String>>,
    busy: Cell<bool>,
    on_screen: Cell<bool>,
    ticking: Cell<bool>,
    started: Cell<Option<Instant>>,
    generation: Cell<u64>,
}

thread_local! {
    static POWER: RefCell<Option<Rc<Power>>> = const { RefCell::new(None) };
}

fn power() -> Option<Rc<Power>> {
    POWER.with(|p| p.borrow().clone())
}

struct PowerFlyout {
    root: gtk::Widget,
}

impl Flyout for PowerFlyout {
    fn name(&self) -> &'static str {
        "power"
    }
    fn widget(&self) -> gtk::Widget {
        self.root.clone()
    }
    fn placement(&self) -> Placement {
        Placement::BottomCenter
    }
    fn opened(&self) {
        if let Some(p) = power() {
            p.on_open();
        }
    }
    fn closed(&self) {
        if let Some(p) = power() {
            p.on_close();
        }
    }
}

fn win_button(label: &str, accent: bool) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("q-b");
    b.add_css_class("q-btn");
    if accent {
        b.add_css_class("q-accent");
    }
    let l = gtk::Label::new(Some(label));
    l.set_ellipsize(pango::EllipsizeMode::End);
    b.set_child(Some(&l));
    b.set_hexpand(true);
    b
}

fn set_label(b: &gtk::Button, text: &str) {
    match b.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
        Some(l) => l.set_text(text),
        None => b.set_label(text),
    }
}

pub fn register(shell: &Rc<Shell>) {
    let root = gtk::Stack::new();
    root.set_hexpand(true);
    root.set_vexpand(true);
    root.set_transition_type(gtk::StackTransitionType::None);
    super::quick::style_root(&root);
    root.add_css_class("fo-power-root");

    // Confirmation: Win11 ContentDialog on smoke
    let smoke = gtk::Box::new(gtk::Orientation::Vertical, 0);
    smoke.add_css_class("fo-smoke");
    smoke.set_hexpand(true);
    smoke.set_vexpand(true);
    let dialog = gtk::Box::new(gtk::Orientation::Vertical, 0);
    dialog.add_css_class("q-dialog");
    dialog.add_css_class("fo-power");
    dialog.set_size_request(448, -1);
    dialog.set_halign(gtk::Align::Center);
    dialog.set_valign(gtk::Align::Center);
    dialog.set_hexpand(true);
    dialog.set_vexpand(true);
    dialog.set_overflow(gtk::Overflow::Hidden);
    dialog.set_accessible_role(gtk::AccessibleRole::AlertDialog);
    let body_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    body_box.add_css_class("q-dialog__body");
    let title = gtk::Label::new(None);
    title.add_css_class("q-dialog__title");
    title.set_xalign(0.0);
    title.set_wrap(true);
    title.set_wrap_mode(pango::WrapMode::WordChar);
    title.set_max_width_chars(36);
    let body = gtk::Label::new(None);
    body.add_css_class("q-dialog__content");
    body.set_xalign(0.0);
    body.set_wrap(true);
    body.set_wrap_mode(pango::WrapMode::WordChar);
    body.set_max_width_chars(50);
    let tip = gtk::Label::new(None);
    tip.add_css_class("q-dialog__content");
    tip.add_css_class("q-text-2");
    tip.add_css_class("q-dialog__tip");
    tip.set_xalign(0.0);
    tip.set_wrap(true);
    tip.set_max_width_chars(50);
    body_box.append(&title);
    body_box.append(&body);
    body_box.append(&tip);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    footer.add_css_class("q-dialog__footer");
    footer.set_homogeneous(true);
    let primary = win_button(&t("shutdown"), true);
    let cancel = win_button(&t("cancel"), false);
    footer.append(&primary);
    footer.append(&cancel);
    dialog.append(&body_box);
    dialog.append(&footer);
    dialog.update_relation(&[gtk::accessible::Relation::LabelledBy(&[title.upcast_ref()])]);
    smoke.append(&dialog);
    primary.connect_clicked(|_| {
        if let Some(p) = power() {
            p.finish(true);
        }
    });
    cancel.connect_clicked(|_| {
        if let Some(p) = power() {
            p.finish(false);
        }
    });
    // Tab stays within the dialog (two buttons)
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(|_, key, _, _| {
        if (key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab || key == gdk::Key::KP_Tab)
            && let Some(p) = power()
        {
            if p.primary.has_focus() {
                p.cancel.grab_focus();
            } else {
                p.primary.grab_focus();
            }
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    dialog.add_controller(keys);
    // clicking the smoke does not dismiss a ContentDialog: a gentle nudge instead
    let click = gtk::GestureClick::new();
    click.connect_pressed(|gesture, _, x, y| {
        let Some(p) = power() else { return };
        let Some(smoke) = gesture.widget() else { return };
        let inside = smoke
            .pick(x, y, gtk::PickFlags::DEFAULT)
            .is_some_and(|w| w == p.dialog.clone().upcast::<gtk::Widget>() || w.is_ancestor(&p.dialog));
        if inside {
            return;
        }
        gesture.set_state(gtk::EventSequenceState::Claimed);
        p.dialog.remove_css_class("is-entering");
        p.dialog.remove_css_class("is-nudge");
        let d = p.dialog.clone();
        glib::idle_add_local_once(move || d.add_css_class("is-nudge"));
        let d = p.dialog.clone();
        glib::timeout_add_local_once(Duration::from_millis(300), move || d.remove_css_class("is-nudge"));
        p.primary.grab_focus();
    });
    smoke.add_controller(click);

    // Shutdown screen: dots in a circle, "Afsluiten…", a note
    let screen = gtk::Box::new(gtk::Orientation::Vertical, 0);
    screen.add_css_class("fo-shutdown");
    screen.set_hexpand(true);
    screen.set_vexpand(true);
    screen.set_focusable(true);
    screen.set_cursor_from_name(Some("none"));
    screen.set_accessible_role(gtk::AccessibleRole::Alert);
    let center = gtk::Box::new(gtk::Orientation::Vertical, 22);
    center.add_css_class("fo-shutdown__center");
    center.set_halign(gtk::Align::Center);
    center.set_valign(gtk::Align::Center);
    center.set_vexpand(true);
    center.set_margin_bottom(80);
    let dots = gtk::DrawingArea::new();
    dots.add_css_class("fo-dots");
    dots.set_content_width(44);
    dots.set_content_height(44);
    dots.set_halign(gtk::Align::Center);
    dots.set_draw_func(|area, cr, _, _| {
        let Some(p) = power() else { return };
        let elapsed = p.started.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        draw_dots(area, cr, elapsed);
    });
    let screen_label = gtk::Label::new(None);
    screen_label.add_css_class("fo-shutdown__label");
    let note = gtk::Label::new(None);
    note.add_css_class("fo-shutdown__note");
    note.set_wrap(true);
    note.set_justify(gtk::Justification::Center);
    note.set_max_width_chars(48);
    note.set_visible(false);
    center.append(&dots);
    center.append(&screen_label);
    center.append(&note);
    screen.append(&center);
    // nothing to cancel any more: Esc does not take the screen away
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(|_, key, _, _| {
        if key == gdk::Key::Escape && power().is_some_and(|p| p.on_screen.get()) {
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    root.add_controller(keys);

    root.add_named(&smoke, Some("dialog"));
    root.add_named(&screen, Some("screen"));
    root.set_visible_child_name("dialog");

    let p = Rc::new(Power {
        root: root.clone(),
        dialog,
        title,
        body,
        tip,
        primary,
        cancel,
        screen,
        screen_label,
        note,
        dots,
        pending: RefCell::new(None),
        busy: Cell::new(false),
        on_screen: Cell::new(false),
        ticking: Cell::new(false),
        started: Cell::new(None),
        generation: Cell::new(0),
    });
    POWER.with(|cell| *cell.borrow_mut() = Some(p.clone()));

    let host = shell.flyouts.borrow().clone();
    if let Some(host) = host {
        host.register(Rc::new(PowerFlyout { root: root.clone().upcast() }));
        // over the whole monitor (fo-host--fullscreen-all), not above the taskbar
        if let Some(frame) = root.parent() {
            frame.set_halign(gtk::Align::Fill);
            frame.set_valign(gtk::Align::Fill);
            frame.set_hexpand(true);
            frame.set_vexpand(true);
            frame.set_margin_bottom(0);
            frame.set_margin_top(0);
            frame.set_margin_start(0);
            frame.set_margin_end(0);
        }
    }
    shell.on("settings.changed", |_, _| {
        if let Some(p) = power() {
            super::quick::apply_mode(&p.root);
        }
    });
}

/// Ask to confirm `action` ("logout" | "shutdown" | "reboot" | "suspend" | "lock"), then run it through the
/// backend (`system.power.action`). Lock runs without asking.
pub fn confirm(shell: &Rc<Shell>, action: &str) {
    let Some(action) = ACTIONS.iter().copied().find(|a| *a == action) else { return };
    let Some(p) = power().filter(|_| host().is_some()) else {
        // no panel on this process: nothing to show the question on
        shell.fire("system.power.action", json!({"action": action}));
        return;
    };
    if p.busy.get() {
        return;
    }
    if action == "lock" {
        if let Some(h) = host() {
            h.close();
        }
        shell.call("system.power.action", json!({"action": action}), |res| {
            if let Err(e) = res {
                eprintln!("win11-shell: {}: {e}", t("failedTitle"));
            }
        });
        return;
    }
    let needs_confirm = matches!(action, "shutdown" | "reboot" | "logout");
    if !needs_confirm || shell.setting("power.confirm") == Value::Bool(false) {
        p.run(action);
        return;
    }
    *p.pending.borrow_mut() = Some(action.to_owned());
    p.fill(action);
    p.show_dialog_page();
    let Some(h) = host() else { return };
    if h.is_open("power") {
        p.focus_primary();
    } else {
        h.open("power");
    }
}

impl Power {
    fn fill(&self, action: &str) {
        self.title.set_text(&t(&format!("{action}Title")));
        let body = t(&format!("{action}Body"));
        self.body.set_text(&body);
        self.body.set_visible(!body.is_empty());
        self.tip.set_text(&t("saveTip"));
        set_label(&self.primary, &t(action));
        set_label(&self.cancel, &t("cancel"));
        super::quick::apply_mode(&self.root);
    }

    fn show_dialog_page(&self) {
        self.on_screen.set(false);
        self.root.set_visible_child_name("dialog");
        self.screen.set_opacity(1.0);
    }

    fn focus_primary(&self) {
        let b = self.primary.clone();
        glib::idle_add_local_once(move || {
            b.grab_focus();
        });
    }

    fn on_open(&self) {
        if self.on_screen.get() {
            let s = self.screen.clone();
            glib::idle_add_local_once(move || {
                s.grab_focus();
            });
            return;
        }
        self.dialog.remove_css_class("is-nudge");
        self.dialog.add_css_class("is-entering");
        let d = self.dialog.clone();
        glib::timeout_add_local_once(Duration::from_millis(300), move || d.remove_css_class("is-entering"));
        self.focus_primary();
    }

    fn on_close(&self) {
        // Esc, focus loss or another flyout opening = "Annuleren"
        self.pending.borrow_mut().take();
        self.dialog.remove_css_class("is-entering");
        self.dialog.remove_css_class("is-nudge");
        if self.on_screen.get() {
            self.on_screen.set(false);
            self.root.set_visible_child_name("dialog");
        }
    }

    fn finish(self: &Rc<Self>, ok: bool) {
        let action = self.pending.borrow_mut().take();
        match action {
            Some(a) if ok => self.run(&a),
            _ => {
                if let Some(h) = host()
                    && h.is_open("power")
                {
                    h.close();
                }
            }
        }
    }

    /// The shutdown screen and the actual call.
    fn run(self: &Rc<Self>, action: &str) {
        if self.busy.get() || !ACTIONS.contains(&action) {
            return;
        }
        self.busy.set(true);
        self.pending.borrow_mut().take();
        let generation = self.generation.get() + 1;
        self.generation.set(generation);
        self.screen_label.set_text(&t(progress_key(action)));
        self.note.set_visible(false);
        self.note.set_text("");
        self.screen.remove_css_class("is-error");
        self.on_screen.set(true);
        self.started.set(Some(Instant::now()));
        self.root.set_visible_child_name("screen");
        self.screen.set_opacity(0.0);
        self.start_dots();
        if let Some(h) = host()
            && !h.is_open("power")
        {
            h.open("power");
        } else {
            let s = self.screen.clone();
            glib::idle_add_local_once(move || {
                s.grab_focus();
            });
        }
        fade(self.screen.upcast_ref(), 0.0, 1.0, 600, || {});
        let action = action.to_owned();
        let delay = if action == "suspend" { 500 } else { 900 };
        glib::timeout_add_local_once(Duration::from_millis(delay), move || {
            sh().call("system.power.action", json!({"action": action}), move |res| {
                let Some(p) = power() else { return };
                if p.generation.get() != generation {
                    return;
                }
                match res {
                    Err(e) => {
                        let msg = if e.trim().is_empty() { t("failed") } else { e };
                        p.note.set_text(&msg);
                        p.note.set_visible(true);
                        p.screen.add_css_class("is-error");
                        eprintln!("win11-shell: {}: {msg}", t("failedTitle"));
                        later(2600, move || {
                            if let Some(p) = power() {
                                p.remove_screen(generation);
                            }
                        });
                    }
                    Ok(r) if r.is_object() && r["dryRun"] == Value::Bool(true) => {
                        later(1800, move || {
                            let Some(p) = power() else { return };
                            if p.generation.get() != generation {
                                return;
                            }
                            p.note.set_text(&t("dryRun"));
                            p.note.set_visible(true);
                            later(1800, move || {
                                if let Some(p) = power() {
                                    p.remove_screen(generation);
                                }
                            });
                        });
                    }
                    Ok(_) => {
                        // the session is going away; after sleep/resume (or if nothing happened) clear the screen
                        let wait = if action == "suspend" { 1500 } else { 20000 };
                        later(wait, move || {
                            if let Some(p) = power() {
                                p.remove_screen(generation);
                            }
                        });
                    }
                }
            });
        });
    }

    fn remove_screen(self: &Rc<Self>, generation: u64) {
        if self.generation.get() != generation {
            return;
        }
        fade(self.screen.upcast_ref(), 1.0, 0.0, 400, move || {
            let Some(p) = power() else { return };
            if p.generation.get() != generation {
                return;
            }
            p.busy.set(false);
            let shown = p.on_screen.get();
            p.on_screen.set(false);
            p.root.set_visible_child_name("dialog");
            p.screen.set_opacity(1.0);
            if shown
                && let Some(h) = host()
                && h.is_open("power")
            {
                h.close();
            }
        });
    }

    fn start_dots(&self) {
        if self.ticking.replace(true) {
            return;
        }
        self.dots.add_tick_callback(|w, _| {
            let Some(p) = power() else { return glib::ControlFlow::Break };
            if !p.on_screen.get() {
                p.ticking.set(false);
                return glib::ControlFlow::Break;
            }
            w.queue_draw();
            glib::ControlFlow::Continue
        });
    }
}

fn later(ms: u64, f: impl FnOnce() + 'static) {
    glib::timeout_add_local_once(Duration::from_millis(ms), f);
}

/// Opacity from `from` to `to` in `ms` (linear), then `done`; runs on a timer, so it finishes when unmapped too.
fn fade(w: &gtk::Widget, from: f64, to: f64, ms: u64, done: impl FnOnce() + 'static) {
    let w = w.clone();
    let start = Instant::now();
    w.set_opacity(from);
    let mut done = Some(done);
    glib::timeout_add_local(Duration::from_millis(16), move || {
        let k = (start.elapsed().as_secs_f64() * 1000.0 / ms as f64).min(1.0);
        w.set_opacity(from + (to - from) * k);
        if k >= 1.0 {
            if let Some(f) = done.take() {
                f();
            }
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });
}

/* ---- Windows-style orbiting dots (flyouts.css fo-orbit: 5 dots, 5.5 s, 0.24 s apart) ---- */

fn bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    // solve x(t) = x by bisection, return y(t)
    let bx = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * x1 + 3.0 * (1.0 - t) * t * t * x2 + t * t * t;
    let by = |t: f64| 3.0 * (1.0 - t) * (1.0 - t) * t * y1 + 3.0 * (1.0 - t) * t * t * y2 + t * t * t;
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..30 {
        let mid = (lo + hi) / 2.0;
        if bx(mid) < x { lo = mid } else { hi = mid }
    }
    by((lo + hi) / 2.0)
}

#[derive(Clone, Copy)]
enum Ease {
    Out,
    Linear,
    InOut,
}

fn ease(e: Ease, x: f64) -> f64 {
    match e {
        Ease::Linear => x,
        Ease::Out => bezier(0.0, 0.0, 0.58, 1.0, x),
        Ease::InOut => bezier(0.42, 0.0, 0.58, 1.0, x),
    }
}

/// (angle in degrees, opacity) of dot `i` at `t` seconds, or None before its delay.
fn dot_state(t: f64, i: usize) -> Option<(f64, f64)> {
    let local = t - i as f64 * 0.24;
    if local < 0.0 {
        return None;
    }
    let f = (local % 5.5) / 5.5;
    const FRAMES: [(f64, f64, Ease); 8] = [
        (0.0, 225.0, Ease::Out),
        (0.07, 345.0, Ease::Linear),
        (0.30, 455.0, Ease::InOut),
        (0.39, 690.0, Ease::Linear),
        (0.70, 815.0, Ease::Out),
        (0.75, 945.0, Ease::Out),
        (0.76, 945.0, Ease::Linear),
        (1.0, 945.0, Ease::Linear),
    ];
    let mut angle = 945.0;
    for k in 0..FRAMES.len() - 1 {
        let (a, a_deg, e) = FRAMES[k];
        let (b, b_deg, _) = FRAMES[k + 1];
        if f >= a && f < b {
            let p = (f - a) / (b - a);
            angle = a_deg + (b_deg - a_deg) * ease(e, p);
            break;
        }
    }
    let opacity = if f < 0.70 {
        1.0
    } else if f < 0.76 {
        1.0 - ease(Ease::Out, (f - 0.70) / 0.06)
    } else {
        0.0
    };
    Some((angle, opacity))
}

fn draw_dots(area: &gtk::DrawingArea, cr: &gtk::cairo::Context, t: f64) {
    let c = area.color();
    let (r, g, b) = (c.red() as f64, c.green() as f64, c.blue() as f64);
    let (cx, cy) = (22.0, 22.0);
    let radius = 22.0 - 2.5;
    for i in 0..5 {
        let Some((deg, opacity)) = dot_state(t, i) else { continue };
        if opacity <= 0.0 {
            continue;
        }
        let a = deg.to_radians();
        let (x, y) = (cx + radius * a.sin(), cy - radius * a.cos());
        cr.set_source_rgba(r, g, b, opacity);
        cr.new_sub_path();
        cr.arc(x, y, 2.5, 0.0, std::f64::consts::TAU);
        let _ = cr.fill();
    }
}
