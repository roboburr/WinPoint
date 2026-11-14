//! Instellingen › Update en herstel: Ubuntu release upgrades on the Update page
//! (vendor/ui/js/apps/settings/pages/update-release.js): the feature-update card "Ubuntu 26.10 is beschikbaar"
//! (states checking, ok, warn, hold, ubuntuHold, blocked, unknown, and the hand-over states updatesFirst,
//! rebootFirst, running, waiting, startFailed, ended) with its start dialog, and the "Nieuwe Ubuntu-versies" row of
//! Meer opties (Ubuntu's Prompt=). No page of its own: update.rs builds both and calls `update(snapshot)` after
//! every update.status it reads. Also the helpers the Update pages share: the update glyph in its disc (turning
//! while busy), the wait for a forwarded write's update.reply, and host errors as JSON envelopes.
use super::{Later, Page, later};
use crate::common;
use kit::CallError;
use kit::controls::{self as c, Dialog, DialogResult, Dropdown, DropdownOption, Expander, InfoBar, ProgressRing, SettingsCard, Severity, Variant};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;

pub const PAGES: &[(&str, super::Builder)] = &[];

/// settings.css "GW6: the feature-update card".
pub const CSS: &str = r#"
.st-updrel-card { border: 1px solid var(--stroke-card); border-radius: var(--radius-card); background-color: var(--bg-card); }
.st-updrel-inner { padding: 16px 16px 16px 20px; }
.st-updrel-title { font-weight: 600; }
.st-updrel-line { color: var(--text-2); }
.st-updrel-line.st-ok { color: var(--text-1); }
.st-updrel-line.st-muted { margin-top: 4px; font-size: var(--fs-caption); }
.st-updrel-checking { color: var(--text-2); }
.st-updrel-links { margin: 2px 0 -6px -11px; }
.st-updrel-before .w11-expander-inner { padding: 4px 16px 12px 58px; }
.st-updrel-reason { padding: 2px 0; color: var(--text-1); }
"#;

/// The product name in every text ("Win11 Shell" in both languages).
pub const PRODUCT: &str = "Win11 Shell";

/// Ubuntu's main.log of the release upgrader (design §3.5 ended; the host may name another in release.logPath).
const UPGRADE_LOG: &str = "/var/log/dist-upgrade/main.log";
/// The hand-over states: shown also when meta-release no longer offers the release (mid-upgrade).
const HANDOVER: &[&str] = &["updatesFirst", "rebootFirst", "running", "waiting", "startFailed", "ended"];
/// How long a forwarded card write waits for the owning window's update.reply before the page re-reads anyway.
const CARD_REPLY_MS: u64 = 60 * 1000;
/// How long a forwarded Prompt change waits (the owner waits for Ubuntu's password dialog).
const REPLY_WAIT_MS: u64 = 10 * 60 * 1000;

// ------------------------------------------------------------------------------------------ shared helpers

/// A host error as the JSON envelope the pages match on ({code, message, key, data}).
pub fn err_value(e: &CallError) -> Value {
    json!({ "code": e.code, "message": e.message, "key": e.key, "data": e.data })
}

/// The error's message, when there is one.
pub fn err_message(e: &Value) -> Option<String> {
    e["message"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

/// A mirror Settings window forwards a write to the owning one, which answers with the event update.reply
/// {nonce, ok, error?}: wait for it, at most `ms` (None when it never comes).
pub fn reply(nonce: Value, ms: u64) -> Later<Option<Value>> {
    let (fill, wait) = later::<Option<Value>>();
    let fill: Rc<RefCell<Option<Box<dyn FnOnce(Option<Value>)>>>> = Rc::new(RefCell::new(Some(Box::new(fill))));
    let listener: Rc<Cell<Option<kit::ListenerId>>> = Rc::new(Cell::new(None));
    let timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));
    let done = {
        let (fill, listener, timer) = (fill.clone(), listener.clone(), timer.clone());
        Rc::new(move |d: Option<Value>| {
            let Some(f) = fill.borrow_mut().take() else { return };
            if let Some(id) = listener.take() {
                kit::ctx().off(id);
            }
            if let Some(t) = timer.borrow_mut().take() {
                t.remove();
            }
            f(d);
        })
    };
    let d2 = done.clone();
    let id = kit::ctx().on("update.reply", move |_, d| {
        if d["nonce"] == nonce {
            d2(Some(d.clone()));
        }
    });
    listener.set(Some(id));
    let t2 = timer.clone();
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(ms), move || {
        // this source has run: forget it before `done` would remove it
        t2.borrow_mut().take();
        done(None);
    });
    *timer.borrow_mut() = Some(id);
    wait
}

/// The colours of a disc (settings.css .st-upd__disc: accent, success or caution).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Accent,
    Ok,
    Caution,
}

struct DiscInner {
    glyph: RefCell<String>,
    tone: Cell<Tone>,
    busy: Cell<bool>,
    start: Cell<i64>,
    tick: RefCell<Option<gtk::TickCallbackId>>,
    glyph_px: f64,
}

/// The update glyph in a round disc (`.st-upd__disc`, `.st-updrel__disc`), drawn with cairo so that it can turn
/// (2.4 s a turn) while busy; still under reduced motion.
#[derive(Clone)]
pub struct Disc {
    area: gtk::DrawingArea,
    inner: Rc<DiscInner>,
}

impl Disc {
    pub fn new(size: i32, glyph_px: i32, glyph: &str) -> Disc {
        let area = gtk::DrawingArea::new();
        area.set_content_width(size);
        area.set_content_height(size);
        area.set_halign(gtk::Align::Center);
        area.set_valign(gtk::Align::Center);
        let inner = Rc::new(DiscInner {
            glyph: RefCell::new(glyph.to_owned()),
            tone: Cell::new(Tone::Accent),
            busy: Cell::new(false),
            start: Cell::new(0),
            tick: RefCell::new(None),
            glyph_px: glyph_px as f64,
        });
        let i2 = inner.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let (bg, fg) = match i2.tone.get() {
                Tone::Accent => (kit::theme::token("accent-subtle"), kit::theme::token("accent-fg-3")),
                Tone::Ok => {
                    let c = kit::theme::token("sys-success");
                    (gdk::RGBA::new(c.red(), c.green(), c.blue(), c.alpha() * 0.16), c)
                }
                Tone::Caution => {
                    let c = kit::theme::token("sys-caution");
                    (gdk::RGBA::new(c.red(), c.green(), c.blue(), c.alpha() * 0.16), c)
                }
            };
            let r = w.min(h) / 2.0;
            cr.set_source_rgba(bg.red() as f64, bg.green() as f64, bg.blue() as f64, bg.alpha() as f64);
            cr.arc(w / 2.0, h / 2.0, r, 0.0, 2.0 * PI);
            let _ = cr.fill();
            let angle = if i2.busy.get() {
                let now = a.frame_clock().map(|f| f.frame_time()).unwrap_or(0);
                let secs = (now - i2.start.get()) as f64 / 1_000_000.0;
                (secs % 2.4) / 2.4 * 2.0 * PI
            } else {
                0.0
            };
            let px = i2.glyph_px;
            cr.save().ok();
            cr.translate(w / 2.0, h / 2.0);
            cr.rotate(angle);
            kit::icons::draw_glyph(cr, &i2.glyph.borrow(), -px / 2.0, -px / 2.0, px, &fg);
            cr.restore().ok();
        });
        kit::theme::track(&area);
        Disc { area, inner }
    }

    pub fn widget(&self) -> &gtk::DrawingArea {
        &self.area
    }

    pub fn set_glyph(&self, name: &str) {
        if *self.inner.glyph.borrow() != name {
            *self.inner.glyph.borrow_mut() = name.to_owned();
            self.area.queue_draw();
        }
    }

    pub fn set_tone(&self, tone: Tone) {
        if self.inner.tone.replace(tone) != tone {
            self.area.queue_draw();
        }
    }

    /// Turn the glyph (not with reduced motion).
    pub fn set_busy(&self, busy: bool) {
        let busy = busy && !kit::ctx().reduce_motion();
        if self.inner.busy.replace(busy) == busy {
            return;
        }
        if busy {
            let now = self.area.frame_clock().map(|f| f.frame_time()).unwrap_or(0);
            self.inner.start.set(now);
            let id = self.area.add_tick_callback(|a, _| {
                a.queue_draw();
                glib::ControlFlow::Continue
            });
            *self.inner.tick.borrow_mut() = Some(id);
        } else if let Some(id) = self.inner.tick.borrow_mut().take() {
            id.remove();
        }
        self.area.queue_draw();
    }
}

/// An info bar without a close button, spaced like the page's notices (`.st-notice`).
pub fn bar(severity: Severity, title: &str, message: &str) -> InfoBar {
    let b = InfoBar::new(severity, title, message);
    b.set_closable(false);
    b.widget().add_css_class("st-notice");
    b
}

/// What a page needs from the Update page around it (update-release.js host).
#[derive(Clone)]
pub struct Host {
    /// Read update.status again and repaint the page.
    pub reload: Rc<dyn Fn() -> Later<()>>,
    /// The hero's "Nu opnieuw opstarten" flow (for the rebootFirst card state).
    pub restart_now: Rc<dyn Fn() -> Later<()>>,
}

/// The snapshot's release object, or None.
fn rel_of(s: Option<&Value>) -> Option<Value> {
    s.map(|s| s["release"].clone()).filter(Value::is_object)
}

fn version_of(r: &Value) -> String {
    let v = common::s(&r["version"]);
    if !v.is_empty() { v.to_owned() } else { common::s(&r["released"]["version"]).to_owned() }
}

/// "9 maanden" / "5 jaar" (design §3.5: months under 24, else years).
fn support_text(months: &Value) -> String {
    let Some(m) = months.as_f64().filter(|m| m.is_finite() && *m > 0.0) else { return String::new() };
    if m < 24.0 {
        common::plural(m.round() as i64, t("1 maand", "1 month"), t("{n} maanden", "{n} months"))
    } else {
        common::plural((m / 12.0).round() as i64, t("1 jaar", "1 year"), t("{n} jaar", "{n} years"))
    }
}

/// distro-info's "2027-07-15" -> "juli 2027" / "July 2027".
fn eol_text(eol: &Value) -> String {
    let Some(s) = eol.as_str() else { return String::new() };
    let mut parts = s.split('-').map(|x| x.parse::<i32>().ok());
    let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (parts.next(), parts.next(), parts.next()) else { return String::new() };
    match glib::DateTime::from_local(y, m, d, 12, 0, 0.0) {
        Ok(dt) => kit::format::date(dt.to_unix() as f64 * 1000.0, DateFormat::Month),
        Err(_) => String::new(),
    }
}

fn hold_text(r: &Value) -> String {
    let version = version_of(r);
    match r["gnomeMajor"].as_i64().map(|n| n.to_string()).or_else(|| r["gnomeMajor"].as_str().filter(|s| !s.is_empty()).map(str::to_owned)) {
        Some(major) => common::fill(
            t(
                "Wacht op een compatibele Win11-versie. {product} werkt nog niet met GNOME {major}, dat bij Ubuntu {version} hoort.",
                "Wait for a compatible Win11 version. {product} doesn’t work with GNOME {major} yet, which comes with Ubuntu {version}.",
            ),
            &[("product", PRODUCT), ("major", &major), ("version", &version)],
        ),
        None => common::fill(
            t(
                "Wacht op een compatibele Win11-versie. Zodra {product} klaar is voor deze versie, zie je de upgrade hier.",
                "Wait for a compatible Win11 version. As soon as {product} is ready for this version, you’ll see the upgrade here.",
            ),
            &[("product", PRODUCT), ("version", &version)],
        ),
    }
}

/// The release's logo (distributor-logo, then Ubuntu's pixmap), else the update glyph in a disc.
fn logo(busy: bool) -> gtk::Widget {
    let b = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    b.set_size_request(48, 48);
    b.set_valign(gtk::Align::Start);
    b.set_hexpand(false);
    let themed = gdk::Display::default().is_some_and(|d| gtk::IconTheme::for_display(&d).has_icon("distributor-logo"));
    let pixmap = "/usr/share/pixmaps/ubuntu-logo.svg";
    if themed || std::path::Path::new(pixmap).exists() {
        let img = if themed { gtk::Image::from_icon_name("distributor-logo") } else { gtk::Image::from_file(pixmap) };
        img.set_pixel_size(48);
        img.set_hexpand(true);
        b.append(&img);
    } else {
        let disc = Disc::new(48, 24, "update");
        disc.set_busy(busy);
        disc.widget().set_hexpand(true);
        b.append(disc.widget());
    }
    b.upcast()
}

// ------------------------------------------------------------------------------------------ the card

struct Line {
    text: String,
    muted: bool,
    ok: bool,
}

impl Line {
    fn plain(text: String) -> Line {
        Line { text, muted: false, ok: false }
    }
    fn muted(text: String) -> Line {
        Line { text, muted: true, ok: false }
    }
}

#[derive(Default)]
struct CardModel {
    title: String,
    checking: Option<String>,
    lines: Vec<Line>,
    links: Vec<gtk::Widget>,
    before: Vec<String>,
    actions: Vec<gtk::Widget>,
    busy_glyph: bool,
}

struct CardInner {
    p: Page,
    host: Host,
    snap: RefCell<Option<Value>>,
    /// the codename update.release.compat was asked for (once per page, state checking)
    asked: RefCell<Option<String>>,
    /// the startFailed / ended state the user left with Retry / Sluiten (this page only)
    back_from: RefCell<Option<String>>,
    /// "Updates installeren en daarna upgraden": the codename whose start dialog follows
    upgrade_after: RefCell<Option<String>>,
    busy: Cell<bool>,
    failure: RefCell<Option<Value>>,
    dialog_open: Cell<bool>,
    el: gtk::Box,
}

/// The feature-update card (design §3.5), with the warn expander and a refusal's info bar under it.
#[derive(Clone)]
pub struct ReleaseCard(Rc<CardInner>);

/// The state left behind on this page (Retry after startFailed, Sluiten after ended).
fn left_key(r: &Value) -> String {
    let k = |v: &Value| common::value_key(v);
    format!("{}|{}|{}|{}|{}", k(&r["state"]), k(&r["codename"]), k(&r["startRc"]), k(&r["lastCheck"]), k(&r["checkedAt"]))
}

impl ReleaseCard {
    pub fn new(p: &Page, host: Host) -> ReleaseCard {
        let el = gtk::Box::new(gtk::Orientation::Vertical, 4);
        el.add_css_class("st-updrel");
        el.set_widget_name("update-release");
        el.set_visible(false);
        ReleaseCard(Rc::new(CardInner {
            p: p.clone(),
            host,
            snap: RefCell::new(None),
            asked: RefCell::new(None),
            back_from: RefCell::new(None),
            upgrade_after: RefCell::new(None),
            busy: Cell::new(false),
            failure: RefCell::new(None),
            dialog_open: Cell::new(false),
            el,
        }))
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.0.el
    }

    /// After every update.status the page reads.
    pub fn update(&self, s: Option<&Value>) {
        let me = &self.0;
        *me.snap.borrow_mut() = s.cloned();
        self.paint();
        let Some(r) = rel_of(s) else { return };
        let codename = common::s(&r["codename"]).to_owned();
        // an offered, tested release without a verdict: ask for the check once (the host refuses what it must not run)
        if r["state"] == "checking" && r["compat"]["needed"] == true && !codename.is_empty() && me.asked.borrow().as_deref() != Some(codename.as_str()) {
            *me.asked.borrow_mut() = Some(codename.clone());
            self.spawn_write("update.release.compat", json!({}), true);
        }
        // "Updates installeren en daarna upgraden": the start dialog once the updates are in and the offer stands
        let phase_error = s.is_some_and(|s| s["phase"] == "error");
        let after = me.upgrade_after.borrow().clone();
        if let Some(after) = after {
            let state = common::s(&r["state"]);
            if codename == after && (state == "ok" || state == "warn") && !phase_error {
                *me.upgrade_after.borrow_mut() = None;
                let card = self.clone();
                me.p.spawn(async move { card.start_dialog(false).await });
            } else if phase_error {
                *me.upgrade_after.borrow_mut() = None;
            }
        }
    }

    fn weak(&self) -> std::rc::Weak<CardInner> {
        Rc::downgrade(&self.0)
    }

    fn spawn_write(&self, method: &str, args: Value, quiet: bool) {
        let card = self.clone();
        let method = method.to_owned();
        self.0.p.spawn(async move {
            let _ = card.write(&method, args, quiet).await;
        });
    }

    /// A card write: refusals become the info bar, a dry run its toast, then the page re-reads.
    async fn write(&self, method: &str, args: Value, quiet: bool) -> Option<Value> {
        let me = &self.0;
        if me.busy.get() && !quiet {
            return None;
        }
        if !quiet {
            me.busy.set(true);
            *me.failure.borrow_mut() = None;
            self.paint();
        }
        let r = me.p.call(method, args).await;
        if !me.p.alive() {
            return None;
        }
        let mut value = None;
        match r {
            Err(e) => {
                if !quiet {
                    if e.code == "unknown_method" {
                        me.p.error(&e, None);
                    } else {
                        *me.failure.borrow_mut() = Some(err_value(&e));
                    }
                }
            }
            Ok(v) if v["forwarded"] == true => {
                let d = reply(v["nonce"].clone(), CARD_REPLY_MS).await;
                if !me.p.alive() {
                    return None;
                }
                match d {
                    Some(d) if d["ok"] == false && d["error"].is_object() && !quiet => *me.failure.borrow_mut() = Some(d["error"].clone()),
                    _ => value = Some(v),
                }
            }
            Ok(v) => {
                if !quiet {
                    common::note_dry_run(&me.p.anchor(), &v);
                }
                value = Some(v);
            }
        }
        if !quiet {
            me.busy.set(false);
        }
        (me.host.reload)().await;
        if me.p.alive() {
            self.paint();
        }
        value
    }

    /// The start dialog (§3.5): battery, metered and English-first info bars; with `anyway` the hold warning first.
    async fn start_dialog(&self, anyway: bool) {
        let me = &self.0;
        let snap = me.snap.borrow().clone();
        let Some(r) = rel_of(snap.as_ref()) else { return };
        if me.dialog_open.get() {
            return;
        }
        me.dialog_open.set(true);
        let version = version_of(&r);
        let battery = me.p.call("system.battery.get", json!({})).await;
        if !me.p.alive() {
            me.dialog_open.set(false);
            return;
        }
        let on_battery = matches!(&battery, Ok(b) if b["present"] == true && b["onAC"] == false);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
        if anyway {
            content.append(bar(Severity::Warning, "", &hold_text(&r)).widget());
        }
        let text = c::label(
            t(
                "Het upgraden doet Ubuntu zelf, in een terminalvenster. Het duurt meestal 30 tot 90 minuten en Ubuntu vraagt om je wachtwoord. Sla je werk op, sluit een laptop aan op de lader en sluit het venster niet tot Ubuntu om een herstart vraagt.",
                "Ubuntu does the upgrade itself, in a terminal window. It usually takes 30 to 90 minutes and Ubuntu asks for your password. Save your work, plug a laptop into its charger, and don’t close the window until Ubuntu asks to restart.",
            ),
            "w11-dialog-text",
            true,
        );
        content.append(&text);
        if on_battery {
            content.append(bar(Severity::Warning, "", t("Je laptop werkt op de accu. Sluit hem aan op de lader voordat je begint.", "Your laptop is running on battery. Plug it in before you start.")).widget());
        }
        if snap.as_ref().is_some_and(|s| s["network"]["metered"] == true) {
            content.append(
                bar(
                    Severity::Warning,
                    "",
                    t(
                        "Je gebruikt een verbinding met een datalimiet. Een upgrade downloadt vaak meer dan 2 GB.",
                        "You’re on a metered connection. An upgrade often downloads more than 2 GB.",
                    ),
                )
                .widget(),
            );
        }
        if kit::lang() == "nl" && r["upgraderNl"] == false {
            content.append(bar(Severity::Info, "", "De eerste regels in het venster zijn in het Engels.").widget());
        }
        let (fill, wait) = later::<DialogResult>();
        let mut d = Dialog::new(&common::fill(t("Upgraden naar Ubuntu {version}?", "Upgrade to Ubuntu {version}?"), &[("version", &version)]))
            .body(&crate::widgets::Clamp::new(&content, 470))
            .primary(if anyway { t("Toch upgraden", "Upgrade anyway") } else { t("Upgrade starten", "Start upgrade") })
            .close_label(t("Annuleren", "Cancel"))
            .width(520);
        if anyway {
            d = d.icon("warning");
        }
        d.show(&me.p.anchor(), fill);
        let answer = wait.await;
        me.dialog_open.set(false);
        // the host's start takes {anyway: true} only after this warning (hold + allowAnyway); ok/warn start with {}
        if answer == DialogResult::Primary && me.p.alive() {
            let _ = self.write("update.release.start", if anyway { json!({ "anyway": true }) } else { json!({}) }, false).await;
        }
    }

    fn link(&self, label: &str, f: impl Fn(&ReleaseCard) + 'static) -> gtk::Widget {
        let a = c::hyperlink(label, None);
        a.set_sensitive(!self.0.busy.get());
        let weak = self.weak();
        a.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                f(&ReleaseCard(inner));
            }
        });
        a.upcast()
    }

    fn button(&self, label: &str, variant: Variant, name: &str, f: impl Fn(&ReleaseCard) + 'static) -> gtk::Widget {
        let b = c::button(label, variant);
        b.set_sensitive(!self.0.busy.get());
        b.set_widget_name(name);
        let weak = self.weak();
        b.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                f(&ReleaseCard(inner));
            }
        });
        b.upcast()
    }

    fn ubuntu_links(&self, r: &Value) -> (Vec<Line>, Vec<gtk::Widget>) {
        if r["ubuntuPromptHeld"] == true {
            (
                vec![Line::muted(common::fill(
                    t("Ubuntu biedt deze upgrade niet meer zelf aan tot {product} er klaar voor is.", "Ubuntu won’t offer this upgrade itself until {product} is ready for it."),
                    &[("product", PRODUCT)],
                ))],
                vec![self.link(t("Ongedaan maken", "Undo"), |card| card.spawn_write("update.release.ubuntuHold", json!({ "on": false }), false))],
            )
        } else {
            (
                vec![Line::muted(
                    t(
                        "De Software-updater van Ubuntu kan deze upgrade nog wel aanbieden. Wacht daar ook mee.",
                        "Ubuntu’s Software Updater may still offer this upgrade. Wait with it there too.",
                    )
                    .to_owned(),
                )],
                vec![self.link(t("Ook de melding van Ubuntu uitstellen", "Postpone Ubuntu’s own prompt too"), |card| {
                    card.spawn_write("update.release.ubuntuHold", json!({ "on": true }), false)
                })],
            )
        }
    }

    /// The card's content per state (§3.5).
    fn view(&self, r: &Value) -> Option<CardModel> {
        let me = &self.0;
        let version = version_of(r);
        let v = |s: &str| common::fill(s, &[("version", &version)]);
        let phase = me.snap.borrow().as_ref().map(|s| common::value_key(&s["phase"])).unwrap_or_default();
        let mut state = common::s(&r["state"]).to_owned();
        // Retry after startFailed / Sluiten after ended: back to the offer on this page (the host keeps its state)
        let back = me.back_from.borrow().clone();
        if back.as_deref() == Some(left_key(r).as_str()) {
            if state == "ended" {
                return None;
            }
            let verdict = common::s(&r["verdict"]);
            state = if !verdict.is_empty() && verdict != "none" { verdict.to_owned() } else { "ok".into() };
        } else if back.is_some() && state != "startFailed" && state != "ended" {
            *me.back_from.borrow_mut() = None;
        }
        let available = v(t("Ubuntu {version} is beschikbaar", "Ubuntu {version} is available"));
        let on_way = v(t("Ubuntu {version} is onderweg", "Ubuntu {version} is on its way"));
        let mut m = CardModel::default();
        match state.as_str() {
            "checking" => {
                m.title = available;
                m.checking = Some(v(t("Controleren of deze pc klaar is voor Ubuntu {version}…", "Checking whether this PC is ready for Ubuntu {version}…")));
            }
            "ok" | "warn" => {
                m.title = available;
                let support = support_text(&r["supportMonths"]);
                let eol = eol_text(&r["eol"]);
                if !support.is_empty() && !eol.is_empty() {
                    m.lines.push(Line::plain(common::fill(
                        t(
                            "De nieuwste versie van Ubuntu, met nieuwe functies. Krijgt {support} updates, tot {eol}.",
                            "The latest version of Ubuntu, with new features. Gets {support} of updates, until {eol}.",
                        ),
                        &[("support", &support), ("eol", &eol)],
                    )));
                }
                m.lines.push(Line {
                    text: common::fill(t("✓ {product} werkt met Ubuntu {version}.", "✓ {product} works with Ubuntu {version}."), &[("product", PRODUCT), ("version", &version)]),
                    muted: false,
                    ok: true,
                });
                let notes = common::s(&r["notesUrl"]).to_owned();
                if notes.starts_with("https://") || notes.starts_with("http://") {
                    m.links.push(self.link(t("Bekijk wat er in deze update zit", "See what’s in this update"), move |card| {
                        let fut = card.0.p.run("apps.openUri", json!({ "uri": notes }));
                        card.0.p.spawn(async move {
                            let _ = fut.await;
                        });
                    }));
                }
                if state == "warn" {
                    m.before = r["reasons"].as_array().map(|a| a.iter().filter_map(|x| err_message(x)).collect()).unwrap_or_default();
                }
                m.actions.push(self.button(t("Downloaden en installeren", "Download and install"), Variant::Accent, "update-release-install", |card| {
                    let card = card.clone();
                    card.0.p.clone().spawn(async move { card.start_dialog(false).await });
                }));
            }
            "hold" => {
                m.title = on_way;
                let (lines, links) = self.ubuntu_links(r);
                m.lines.push(Line::plain(hold_text(r)));
                m.lines.extend(lines);
                if r["allowAnyway"] == true {
                    m.links.push(self.link(t("Toch upgraden…", "Upgrade anyway…"), |card| {
                        let card = card.clone();
                        card.0.p.clone().spawn(async move { card.start_dialog(true).await });
                    }));
                }
                m.links.extend(links);
            }
            "ubuntuHold" => {
                m.title = on_way;
                m.lines.push(Line::plain(t("Ubuntu heeft de upgrade tijdelijk stopgezet.", "Ubuntu has paused the upgrade for now.").to_owned()));
            }
            "blocked" => {
                m.title = v(t("Deze pc is nog niet klaar voor Ubuntu {version}", "This PC isn’t ready for Ubuntu {version} yet"));
                let (lines, links) = self.ubuntu_links(r);
                for reason in r["reasons"].as_array().into_iter().flatten().filter_map(err_message) {
                    m.lines.push(Line::plain(reason));
                }
                m.lines.extend(lines);
                m.links = links;
            }
            "updatesFirst" => {
                m.title = available;
                m.lines.push(Line::plain(t("Installeer eerst de beschikbare updates.", "Install the available updates first.").to_owned()));
                let held: Vec<String> = r["heldNames"].as_array().map(|a| a.iter().map(common::value_key).collect()).unwrap_or_default();
                if !held.is_empty() {
                    m.lines.push(Line::muted(held.join(", ")));
                }
                let codename = common::s(&r["codename"]).to_owned();
                m.actions.push(self.button(t("Updates installeren en daarna upgraden", "Install updates, then upgrade"), Variant::Accent, "update-release-updates", move |card| {
                    *card.0.upgrade_after.borrow_mut() = Some(codename.clone());
                    let card = card.clone();
                    card.0.p.clone().spawn(async move {
                        if card.write("update.install", json!({}), false).await.is_none() {
                            *card.0.upgrade_after.borrow_mut() = None;
                        }
                    });
                }));
            }
            "rebootFirst" => {
                m.title = available;
                m.lines.push(Line::plain(
                    t(
                        "Start je pc eerst opnieuw op om eerdere updates af te ronden. Daarna kun je upgraden.",
                        "Restart your PC first to finish earlier updates. Then you can upgrade.",
                    )
                    .to_owned(),
                ));
                m.actions.push(self.button(t("Nu opnieuw opstarten", "Restart now"), Variant::Standard, "update-release-restart", |card| {
                    let card = card.clone();
                    card.0.busy.set(true);
                    card.paint();
                    card.0.p.clone().spawn(async move {
                        (card.0.host.restart_now)().await;
                        card.0.busy.set(false);
                        if card.0.p.alive() {
                            card.paint();
                        }
                    });
                }));
            }
            "running" | "waiting" => {
                // the hero says exactly this while it shows releaseRunning (§3.2): the card adds it only next to our own job
                if version.is_empty() || (state == "running" && phase == "releaseRunning") {
                    return None;
                }
                m.title = v(t("Ubuntu wordt bijgewerkt naar {version}", "Upgrading Ubuntu to {version}"));
                m.lines.push(Line::plain(
                    if state == "running" {
                        t("Het venster van Ubuntu doet de upgrade. Sluit het niet.", "Ubuntu’s window does the upgrade. Don’t close it.")
                    } else {
                        t("Ubuntu wacht misschien op een antwoord in het upgradevenster.", "Ubuntu may be waiting for an answer in the upgrade window.")
                    }
                    .to_owned(),
                ));
                m.busy_glyph = true;
                // attachable false while Ubuntu's screen session is not there (yet): no button then; one accent
                // button on the page (the hero has its own while it shows releaseRunning)
                if r["attachable"] != false {
                    let variant = if state == "waiting" && phase != "releaseRunning" { Variant::Accent } else { Variant::Standard };
                    m.actions.push(self.button(t("Venster weer openen", "Show the upgrade window"), variant, "update-release-attach", |card| {
                        card.spawn_write("update.release.attach", json!({}), false)
                    }));
                }
            }
            "startFailed" => {
                m.title = t("De upgrade is niet gestart", "The upgrade didn’t start").to_owned();
                let why = err_message(&r["startError"]).or_else(|| r["reasons"].as_array().into_iter().flatten().find_map(err_message));
                if let Some(why) = why {
                    m.lines.push(Line::plain(why));
                }
                let key = left_key(r);
                m.actions.push(self.button(t("Opnieuw proberen", "Retry"), Variant::Standard, "update-release-retry", move |card| {
                    *card.0.back_from.borrow_mut() = Some(key.clone());
                    *card.0.failure.borrow_mut() = None;
                    card.paint();
                }));
            }
            "ended" => {
                m.title = t("De upgrade is gestopt voordat hij klaar was", "The upgrade stopped before it finished").to_owned();
                m.lines.push(Line::plain(
                    t(
                        "Ubuntu heeft de upgrade niet afgemaakt. In het logboek staat wat er is gebeurd.",
                        "Ubuntu didn’t finish the upgrade. The log shows what happened.",
                    )
                    .to_owned(),
                ));
                let log = r["logPath"].as_str().filter(|s| !s.is_empty()).unwrap_or(UPGRADE_LOG).to_owned();
                m.actions.push(self.button(t("Logboek openen", "Open log"), Variant::Standard, "update-release-log", move |card| {
                    let fut = card.0.p.run("files.open", json!({ "path": log }));
                    card.0.p.spawn(async move {
                        let _ = fut.await;
                    });
                }));
                let key = left_key(r);
                m.actions.push(self.button(t("Sluiten", "Close"), Variant::Standard, "update-release-close", move |card| {
                    *card.0.back_from.borrow_mut() = Some(key.clone());
                    card.paint();
                }));
            }
            "unknown" => {
                m.title = available;
                let reason = r["reasons"].as_array().into_iter().flatten().find_map(err_message).unwrap_or_default();
                m.lines.push(Line::plain(common::fill(t("De controle is niet gelukt. {reason}", "The check didn’t work. {reason}"), &[("reason", &reason)]).trim().to_owned()));
                m.actions.push(self.button(t("Opnieuw proberen", "Retry"), Variant::Standard, "update-release-retry", |card| {
                    card.spawn_write("update.release.compat", json!({ "refresh": true }), false)
                }));
            }
            _ => return None,
        }
        Some(m)
    }

    fn paint(&self) {
        let me = &self.0;
        let snap = me.snap.borrow().clone();
        let r = rel_of(snap.as_ref());
        c::clear(&me.el);
        // the offer's states need an offer; the hand-over states show also when the offer went away meanwhile
        let m = r.as_ref().and_then(|r| {
            let state = common::s(&r["state"]);
            let shown = !state.is_empty() && state != "none" && (r["offered"] != false || HANDOVER.contains(&state));
            if shown { self.view(r) } else { None }
        });
        let Some(m) = m else {
            me.el.set_visible(false);
            return;
        };
        me.el.set_visible(true);
        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("st-updrel-card");
        let inner = crate::widgets::Wrap::new(16, 12);
        inner.add_css_class("st-updrel-inner");
        inner.append(&logo(m.busy_glyph));
        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.set_size_request(260, -1);
        text.append(&c::label(&m.title, "st-updrel-title", true));
        if let Some(checking) = &m.checking {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            row.add_css_class("st-updrel-checking");
            row.append(ProgressRing::new(16, None).widget());
            row.append(&c::label(checking, "", true));
            text.append(&row);
        }
        for l in &m.lines {
            let w = c::label(&l.text, "st-updrel-line", true);
            w.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            if l.muted {
                w.add_css_class("st-muted");
            }
            if l.ok {
                w.add_css_class("st-ok");
            }
            text.append(&w);
        }
        if !m.links.is_empty() {
            let links = crate::widgets::Wrap::new(12, 0);
            links.add_css_class("st-updrel-links");
            for l in &m.links {
                links.append(l);
            }
            text.append(&links);
        }
        inner.append(&text);
        if !m.actions.is_empty() {
            let ctl = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            ctl.set_halign(gtk::Align::End);
            ctl.set_valign(gtk::Align::Center);
            for a in &m.actions {
                ctl.append(a);
            }
            inner.append(&ctl);
        }
        card.append(&inner);
        me.el.append(&card);
        if !m.before.is_empty() {
            let exp = Expander::new("warning", t("Let op voordat je upgradet", "Before you upgrade"), "");
            exp.widget().add_css_class("st-updrel-before");
            exp.header_button().set_widget_name("update-release-before");
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for reason in &m.before {
                let l = c::label(&format!("•  {reason}"), "st-updrel-reason", true);
                l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                list.append(&l);
            }
            exp.set_content(&list);
            me.el.append(exp.widget());
        }
        let failure = me.failure.borrow().clone();
        if let Some(msg) = failure.as_ref().and_then(err_message) {
            let b = InfoBar::new(Severity::Error, "", &msg);
            b.widget().add_css_class("st-notice");
            let weak = self.weak();
            b.connect_closed(move || {
                if let Some(inner) = weak.upgrade() {
                    *inner.failure.borrow_mut() = None;
                    ReleaseCard(inner).paint();
                }
            });
            me.el.append(b.widget());
        }
    }
}

// ------------------------------------------------------------------------------------------ the Prompt row

struct PromptInner {
    p: Page,
    host: Host,
    rel: RefCell<Option<Value>>,
    /// Ubuntu's release upgrader runs (phase releaseRunning): the host refuses every write
    running: Cell<bool>,
    /// the value of the setPrompt in flight
    wanted: RefCell<Option<String>>,
    failure: RefCell<Option<Value>>,
    dropdown: Dropdown,
    card: SettingsCard,
    bars: gtk::Box,
    el: gtk::Box,
}

/// The "Nieuwe Ubuntu-versies" row (§3.6), the first item of Meer opties: Ubuntu's own Prompt= as a drop-down,
/// what the choice means, and the info bars of the root helper and of a refusal.
#[derive(Clone)]
pub struct PromptRow(Rc<PromptInner>);

impl PromptRow {
    pub fn new(p: &Page, host: Host) -> PromptRow {
        let dropdown = Dropdown::new(Vec::new(), None);
        dropdown.widget().set_widget_name("update-prompt");
        dropdown.widget().update_property(&[gtk::accessible::Property::Label(t("Nieuwe Ubuntu-versies", "New Ubuntu versions"))]);
        let card = SettingsCard::new("sparkle", t("Nieuwe Ubuntu-versies", "New Ubuntu versions"), "");
        card.set_control(&common::sized(dropdown.widget(), 200));
        let bars = gtk::Box::new(gtk::Orientation::Vertical, 4);
        bars.set_visible(false);
        let el = gtk::Box::new(gtk::Orientation::Vertical, 4);
        el.add_css_class("st-updprompt");
        el.append(card.widget());
        el.append(&bars);
        el.set_visible(false);
        let row = PromptRow(Rc::new(PromptInner {
            p: p.clone(),
            host,
            rel: RefCell::new(None),
            running: Cell::new(false),
            wanted: RefCell::new(None),
            failure: RefCell::new(None),
            dropdown,
            card,
            bars,
            el,
        }));
        let weak = Rc::downgrade(&row.0);
        row.0.dropdown.connect_changed(move |v| {
            if let Some(inner) = weak.upgrade() {
                let row = PromptRow(inner);
                let v = v.to_owned();
                row.0.p.clone().spawn(async move { row.change(v).await });
            }
        });
        row
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.0.el
    }

    /// After every update.status the page reads.
    pub fn update(&self, s: Option<&Value>) {
        let rel = s.map(|s| s["release"].clone()).filter(Value::is_object);
        let running = s.is_some_and(|s| s["phase"] == "releaseRunning") || rel.as_ref().is_some_and(|r| r["state"] == "running");
        *self.0.rel.borrow_mut() = rel;
        self.0.running.set(running);
        self.paint();
    }

    /// The drop-down can change the file now: no .d file decides and Ubuntu's upgrader does not run.
    fn changeable(&self) -> bool {
        let over = self.0.rel.borrow().as_ref().is_some_and(|r| common::value_key(&r["override"]).trim() != "" && r["override"] != false);
        !over && !self.0.running.get()
    }

    fn description(r: &Value) -> String {
        let mut lines: Vec<String> = Vec::new();
        let prompt = common::s(&r["prompt"]);
        if prompt == "lts" {
            lines.push(t("Om de twee jaar een upgrade; elke versie krijgt 5 jaar updates.", "An upgrade every two years; each version gets 5 years of updates.").into());
        }
        if prompt == "normal" {
            lines.push(t("Elk half jaar een upgrade; een tussenversie krijgt 9 maanden updates.", "An upgrade every six months; an interim version gets 9 months of updates.").into());
            lines.push(common::fill(
                t(
                    "Elke tussenversie heeft een nieuwe GNOME, en {product} moet daar eerst klaar voor zijn.",
                    "Every interim version has a new GNOME, and {product} has to be ready for it first.",
                ),
                &[("product", PRODUCT)],
            ));
        }
        if prompt == "lts" && r["currentLts"] == false {
            lines.push(
                t(
                    "Je gebruikt een tussenversie, dus Ubuntu biedt toch elke nieuwe versie aan tot de volgende LTS.",
                    "You’re on an interim version, so Ubuntu still offers every new version until the next LTS.",
                )
                .into(),
            );
        }
        let released = common::s(&r["released"]["version"]);
        let offered = !matches!(r["offered"], Value::Null | Value::Bool(false));
        if !offered && !released.is_empty() && prompt != "never" {
            lines.push(common::fill(
                t("Ubuntu {version} is uitgebracht; Ubuntu biedt de upgrade nog niet aan.", "Ubuntu {version} is out; Ubuntu doesn’t offer the upgrade yet."),
                &[("version", released)],
            ));
        }
        let over = common::value_key(&r["override"]);
        if !over.is_empty() && r["override"] != false {
            lines.push(common::fill(t("Deze instelling wordt bepaald door {file}.", "This setting is managed by {file}."), &[("file", &over)]));
        }
        lines.join("\n")
    }

    fn paint_bars(&self) {
        let me = &self.0;
        c::clear(&me.bars);
        // without the root helper Ubuntu's dialog names /bin/sh: say what it is before the choice and while it waits
        let no_helper = me.rel.borrow().as_ref().is_some_and(|r| r["helper"].is_object() && r["helper"]["prompt"] == false);
        if no_helper && (me.wanted.borrow().is_some() || self.changeable()) {
            me.bars.append(
                bar(
                    Severity::Info,
                    "",
                    &common::fill(
                        t(
                            "Ubuntu vraagt om je wachtwoord om ‘/bin/sh’ als beheerder uit te voeren. Dat is {product}, dat alleen de regel Prompt= aanpast.",
                            "Ubuntu asks for your password to run ‘/bin/sh’ as administrator. That’s {product}, changing only the Prompt= line.",
                        ),
                        &[("product", PRODUCT)],
                    ),
                )
                .widget(),
            );
        }
        let failure = me.failure.borrow().clone();
        if let Some(msg) = failure.as_ref().and_then(err_message) {
            let b = InfoBar::new(Severity::Error, "", &msg);
            b.widget().add_css_class("st-notice");
            let weak = Rc::downgrade(&self.0);
            b.connect_closed(move || {
                if let Some(inner) = weak.upgrade() {
                    *inner.failure.borrow_mut() = None;
                    PromptRow(inner).paint_bars();
                }
            });
            me.bars.append(b.widget());
        }
        me.bars.set_visible(me.bars.first_child().is_some());
    }

    fn paint(&self) {
        let me = &self.0;
        let rel = me.rel.borrow().clone();
        let Some(r) = rel.filter(|r| r["prompt"].is_string()) else {
            me.el.set_visible(false);
            return;
        };
        me.el.set_visible(true);
        let prompt = common::s(&r["prompt"]).to_owned();
        let shown = me.wanted.borrow().clone().unwrap_or_else(|| prompt.clone());
        let mut options = vec![
            DropdownOption::new("lts", t("Alleen LTS-versies", "LTS versions only")),
            DropdownOption::new("normal", t("Elke nieuwe versie", "Every new version")),
        ];
        // "Nooit melden" exists only while Ubuntu's file says never: Windows has no such choice
        if prompt == "never" || shown == "never" {
            options.push(DropdownOption::new("never", t("Nooit melden", "Never notify")));
        }
        me.dropdown.set_options(options);
        me.dropdown.set_selected(Some(&shown));
        me.dropdown.widget().set_sensitive(me.wanted.borrow().is_none() && self.changeable());
        me.card.set_description(&Self::description(&r));
        self.paint_bars();
    }

    async fn change(&self, value: String) {
        let me = &self.0;
        let current = me.rel.borrow().as_ref().map(|r| common::value_key(&r["prompt"]));
        if me.wanted.borrow().is_some() || current.is_none() || current.as_deref() == Some(value.as_str()) {
            return;
        }
        *me.wanted.borrow_mut() = Some(value.clone());
        *me.failure.borrow_mut() = None;
        self.paint();
        let r = me.p.call("update.release.setPrompt", json!({ "prompt": value })).await;
        if !me.p.alive() {
            return;
        }
        match r {
            Err(e) => {
                if e.code == "unknown_method" {
                    me.p.error(&e, None);
                } else {
                    *me.failure.borrow_mut() = Some(err_value(&e));
                }
            }
            Ok(v) if v["forwarded"] == true => {
                let d = reply(v["nonce"].clone(), REPLY_WAIT_MS).await;
                if !me.p.alive() {
                    return;
                }
                if let Some(d) = d
                    && d["ok"] == false
                    && d["error"].is_object()
                {
                    *me.failure.borrow_mut() = Some(d["error"].clone());
                }
            }
            Ok(v) => common::note_dry_run(&me.p.anchor(), &v),
        }
        *me.wanted.borrow_mut() = None;
        // the file's value again (also after a refusal or a dry run): the page reads update.status again
        (me.host.reload)().await;
        if me.p.alive() {
            self.paint();
        }
    }
}
