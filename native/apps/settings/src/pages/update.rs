//! Instellingen › Update en herstel (vendor/ui/js/apps/settings/pages/update.js): Windows Update for Ubuntu (the
//! update.* service: the hero with its state and one button, the info bars, the feature-update card of
//! update_release.rs, the update rows with their packages and "Wat is er nieuw", the metered / removals confirm,
//! Meer opties), the settings backup (export / import), reset everything, and Over Win11 Shell.
//!
//! Every text is rendered here from the snapshot's codes and params ({phase, rows, job, restart, ...}), so a
//! language switch needs no host round trip; host texts (error.message, warnings[].message) come with the next
//! update.status. Errors are matched on key or code, never on their text. Without an update.* service (an older
//! host: update.status is an unknown method) the hero falls back to system.updates. Structural changes
//! (update.changed) read the snapshot again; update.progress patches it in place.
use super::update_release::{Disc, Host, PRODUCT, PromptRow, ReleaseCard, Tone, bar, err_message, err_value};
use super::{Builder, Content, Later, Page, PageFuture, later};
use crate::common;
use kit::controls::{self as c, Chevron, Dialog, DialogResult, Expander, InfoBar, ProgressBar, ProgressRing, ProgressState, SettingsCard, SettingsExpander, SettingsGroup, Severity, Toast, Variant};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

pub const PAGES: &[(&str, Builder)] = &[("update", update)];

/// settings.css "Update hero" and "Update rows".
pub const CSS: &str = r#"
.st-upd { min-height: 88px; padding: 4px 0; }
.st-upd-title {
  font-family: "Segoe UI Variable Display", "Segoe UI Variable", "Segoe UI", "Selawik", "Inter Display", "Inter", "Noto Sans", "Ubuntu Sans", sans-serif;
  font-size: var(--fs-subtitle);
  font-weight: 600;
}
.st-upd-sub { color: var(--text-2); }
.st-upd-badge { min-width: 22px; min-height: 22px; border-radius: 11px; background-color: var(--sys-success); color: var(--text-inverse); }
.st-upd-badge.st-error { background-color: var(--sys-critical); }
.st-upd-detail { color: var(--text-1); }
.st-upd-bars { margin-top: -6px; }
.st-updrows-head { min-height: 32px; margin: 0 0 4px 1px; }
.st-updrows-title { font-weight: 600; }
.st-updrows-note { margin: 4px 0 0 1px; color: var(--text-2); font-size: var(--fs-caption); }
.st-updrow-state { color: var(--text-2); }
.w11-settings-expander .w11-expander-inner > .st-updrow-details.w11-nested-other { padding: 4px 16px 12px 16px; }
.st-updrow-pkg { padding: 8px 0; border-top: 1px solid var(--stroke-divider); }
.st-updrow-pkg:first-child { border-top-width: 0; }
.st-updrow-pkgsum, .st-updrow-tag { color: var(--text-2); font-size: var(--fs-caption); }
.st-updrow-links { margin: 4px 0 0 -11px; }
.st-updnews-name { font-weight: 600; }
.st-updnews-h { margin-top: 8px; color: var(--text-2); font-size: var(--fs-caption); }
.st-updnews-text { margin-top: 2px; }
.st-updnews-links { margin: 2px 0 0 -11px; }
.st-updnews-none { color: var(--text-2); }
.st-updnews-wait { padding: 24px 0; }
"#;

// ------------------------------------------------------------------------------------------ texts from codes

/// "7 uur geleden (25-10-2026 07:35)".
fn when_text(at: f64) -> String {
    format!(
        "{} ({} {})",
        kit::format::rel_time(at, kit::format::now_ms()),
        kit::format::date(at, DateFormat::Short),
        kit::format::time(at, false, true)
    )
}

fn checked_text(s: &Value) -> String {
    match s["checkedAt"].as_f64().filter(|v| v.is_finite() && *v > 0.0) {
        Some(at) => common::fill(t("Laatst gecontroleerd: {when}", "Last checked: {when}"), &[("when", &when_text(at))]),
        None => String::new(),
    }
}

fn pct_of(v: &Value) -> f64 {
    v.as_f64().filter(|v| v.is_finite()).map(|v| v.round().clamp(0.0, 100.0)).unwrap_or(0.0)
}

/// `\s+\)` -> `)` (the first), then runs of white space to one space, trimmed: a missing version leaves no stray space.
fn tidy(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    let mut done = false;
    while i < chars.len() {
        if !done && chars[i].is_whitespace() {
            let mut j = i;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && chars[j] == ')' {
                out.push(')');
                i = j + 1;
                done = true;
                continue;
            }
            out.extend(&chars[i..j]);
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    let s: String = out.into_iter().collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// §3.3 row titles by kind.
fn row_title(r: &Value) -> String {
    let template = match r["kind"].as_str().unwrap_or("") {
        "security" => t("Beveiligingsupdates voor Ubuntu {version}", "Security updates for Ubuntu {version}"),
        "gnome" => t("GNOME Shell {version}", "GNOME Shell {version}"),
        "webkit" => t("WebKitGTK {version} (webweergave van {product})", "WebKitGTK {version} ({product}’s web view)"),
        "gtk" => t("GTK 3 {version}", "GTK 3 {version}"),
        "soup" => t("libsoup {version}", "libsoup {version}"),
        "glib" => t("GLib {version}", "GLib {version}"),
        "kernel" => t("Linux-kernel {version}", "Linux kernel {version}"),
        _ => return t("Overige updates", "Other updates").to_owned(),
    };
    tidy(&common::fill(template, &[("version", &common::value_key(&r["version"])), ("product", PRODUCT)]))
}

fn row_caption(r: &Value) -> String {
    let n = r["count"].as_f64().filter(|v| v.is_finite()).map(|v| v as i64).unwrap_or(0);
    let size = r["size"].as_f64().filter(|v| v.is_finite() && *v > 0.0);
    let c = match size {
        Some(size) => common::fill(
            &common::plural(n, t("1 pakket · {size}", "1 package · {size}"), t("{n} pakketten · {size}", "{n} packages · {size}")),
            &[("size", &kit::format::bytes(size))],
        ),
        None => common::plural(n, t("1 pakket", "1 package"), t("{n} pakketten", "{n} packages")),
    };
    if r["security"] == true && r["kind"] != "security" { format!("{} · {c}", t("Beveiligingsupdate", "Security update")) } else { c }
}

fn row_state(r: &Value) -> String {
    let pct = pct_of(&r["pct"]).to_string();
    let template = match r["state"].as_str().unwrap_or("") {
        "downloading" => t("Downloaden - {pct}%", "Downloading - {pct}%"),
        "pendingInstall" => t("Wachten op installatie", "Pending install"),
        "installing" => t("Installeren - {pct}%", "Installing - {pct}%"),
        "installed" => t("Geïnstalleerd", "Installed"),
        "restart" => t("Wachten op opnieuw opstarten", "Pending restart"),
        "failed" => t("Installatiefout", "Install error"),
        _ => t("Wachten op downloaden", "Pending download"),
    };
    common::fill(template, &[("pct", &pct)])
}

fn moving(state: &str) -> bool {
    matches!(state, "downloading" | "installing")
}

fn busy_phase(phase: &str) -> bool {
    matches!(phase, "checking" | "downloading" | "installing" | "waiting" | "foreign" | "releaseRunning")
}

/// restart.rebootPkgs -> friendly names (§3.2).
fn pkg_name(name: &str) -> String {
    if name.starts_with("linux-image-") || name.starts_with("linux-modules-") || name == "linux-base" {
        return t("Linux-kernel", "Linux kernel").into();
    }
    if name == "libc6" {
        return t("systeembibliotheek", "system library").into();
    }
    if name.starts_with("gnome-shell") || name.starts_with("mutter") || name.starts_with("libmutter-") {
        return t("GNOME Shell", "GNOME Shell").into();
    }
    name.into()
}

/// At most 3 names, then "en 2 andere".
fn short_list(names: &[String]) -> String {
    let mut xs: Vec<&str> = Vec::new();
    for n in names {
        if !xs.contains(&n.as_str()) {
            xs.push(n);
        }
    }
    if xs.len() > 3 {
        let more = common::plural((xs.len() - 3) as i64, t("en 1 andere", "and 1 other"), t("en {n} andere", "and {n} others"));
        format!("{} {more}", xs[..3].join(", "))
    } else {
        kit::i18n::list(&xs)
    }
}

fn restart_text(r: &Value) -> String {
    let names: Vec<String> = r["rebootPkgs"].as_array().map(|a| a.iter().filter_map(|n| n.as_str()).map(pkg_name).collect()).unwrap_or_default();
    if names.is_empty() {
        t("Start je pc opnieuw op om de updates af te ronden.", "Restart your PC to finish installing updates.").into()
    } else {
        common::fill(t("Start je pc opnieuw op om de update van {list} af te ronden.", "Restart your PC to finish the update of {list}."), &[("list", &short_list(&names))])
    }
}

/// Shell restart parts (§3.4, restart.shellParts) -> component names.
fn part_name(p: &str) -> String {
    match p {
        "webkit" | "jsc" => "WebKitGTK".into(),
        "gtk" | "gdk" => "GTK".into(),
        "soup" => "libsoup".into(),
        "glib" | "gio" | "gobject" => "GLib".into(),
        "pixbuf" => "GDK Pixbuf".into(),
        "cairo" => "cairo".into(),
        "archive" => "libarchive".into(),
        "self" => PRODUCT.into(),
        other => other.into(),
    }
}

fn key_of(e: &Value) -> &str {
    e["key"].as_str().unwrap_or("")
}

/// The test seam's refusal (key, or code test_seam).
fn is_test_seam(e: &Value) -> bool {
    key_of(e) == "update.err.testSeam" || e["code"] == "test_seam"
}

/// InfoBar severity of a host error next to the hero; default warning. A refusal without a catalogue key (the kit's
/// own errors: offline, bad_response) is matched on its code (update.err.cancelled goes with the code cancelled).
fn bar_severity(e: &Value) -> Severity {
    if key_of(e) == "update.err.cancelled" || (e["key"].is_null() && e["code"] == "cancelled") {
        return Severity::Info;
    }
    if is_test_seam(e) { Severity::Error } else { Severity::Warning }
}

/// Errors after which the hero's Retry checks again instead of installing: no server, offline, the check failed.
fn is_recheck(e: &Value) -> bool {
    matches!(key_of(e), "update.err.network" | "update.err.offline" | "update.err.checkFailed") || e["code"] == "network"
}

// ------------------------------------------------------------------------------------------ hero model (§3.2)

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    CheckFor,
    InstallAll,
    Cancel,
    Attach,
    RestartNow,
    Repair,
    OpenUpdater,
    LegacyCheck,
    Retry,
}

impl Action {
    fn label(self) -> &'static str {
        match self {
            Action::CheckFor | Action::LegacyCheck => t("Controleren op updates", "Check for updates"),
            Action::InstallAll => t("Alles downloaden en installeren", "Download & install all"),
            Action::Cancel => t("Annuleren", "Cancel"),
            Action::Attach => t("Venster weer openen", "Show the upgrade window"),
            Action::RestartNow => t("Nu opnieuw opstarten", "Restart now"),
            Action::Repair => t("Herstellen", "Repair"),
            Action::OpenUpdater => t("Software-updater openen", "Open Software Updater"),
            Action::Retry => t("Opnieuw proberen", "Retry"),
        }
    }
    fn variant(self) -> Variant {
        if self == Action::Cancel { Variant::Standard } else { Variant::Accent }
    }
}

struct HeroModel {
    tone: Tone,
    busy: bool,
    ring: bool,
    glyph: &'static str,
    badge: Option<bool>,
    title: String,
    sub: String,
    action: Option<Action>,
    disabled: bool,
}

impl HeroModel {
    fn new(tone: Tone, title: &str, sub: String) -> HeroModel {
        HeroModel { tone, busy: false, ring: false, glyph: "update", badge: None, title: title.to_owned(), sub, action: None, disabled: false }
    }
    fn act(mut self, a: Action) -> HeroModel {
        self.action = Some(a);
        self
    }
    fn badge(mut self, error: bool) -> HeroModel {
        self.badge = Some(error);
        self
    }
    fn busy(mut self) -> HeroModel {
        self.busy = true;
        self
    }
    fn unknown(sub: String) -> HeroModel {
        HeroModel { glyph: "warning", ..HeroModel::new(Tone::Caution, t("Kan niet controleren op updates", "Can’t check for updates"), sub) }.act(Action::OpenUpdater)
    }
}

fn unknown_desc() -> String {
    t(
        "Gebruik de Software-updater van Ubuntu om te kijken of er updates zijn.",
        "Use Ubuntu’s Software Updater to see whether updates are available.",
    )
    .into()
}

/// Without update.*: system.updates ({available, checkedAt}); None when that failed too.
fn legacy_model(u: Option<&Value>) -> HeroModel {
    let u = u.cloned().unwrap_or(Value::Null);
    let sub = checked_text(&u);
    match u["available"].as_f64() {
        Some(n) if n.is_finite() && n > 0.0 => HeroModel::new(Tone::Accent, t("Updates beschikbaar", "Updates available"), sub).act(Action::OpenUpdater),
        Some(n) if n == 0.0 => HeroModel::new(Tone::Ok, t("Je bent up-to-date", "You’re up to date"), sub).badge(false).act(Action::LegacyCheck),
        _ => HeroModel::unknown(unknown_desc()),
    }
}

fn hero_model(s: Option<&Value>, legacy: Option<&Value>) -> HeroModel {
    let Some(s) = s else { return legacy_model(legacy) };
    let j = &s["job"];
    let checked = checked_text(s);
    let allow_cancel = j["allowCancel"] == true;
    match s["phase"].as_str().unwrap_or("") {
        "idle" => HeroModel::new(Tone::Ok, t("Je bent up-to-date", "You’re up to date"), checked).badge(false).act(Action::CheckFor),
        "checking" => HeroModel { ring: true, ..HeroModel::new(Tone::Accent, t("Controleren op updates…", "Checking for updates…"), String::new()) },
        "available" => HeroModel::new(Tone::Accent, t("Updates beschikbaar", "Updates available"), checked).act(Action::InstallAll),
        "downloading" => {
            // the simulate (and its confirm): "Voorbereiden…"; then "{done} van {total}" while the total is known,
            // else no caption: the rows show "Downloaden - 45%"
            let total = s["totals"]["downloadSize"].as_f64().filter(|v| v.is_finite() && *v > 0.0);
            let left = j["downloadLeft"].as_f64().filter(|v| v.is_finite());
            let sub = if j["simulating"] == true || (!j["confirm"].is_null() && j["confirm"] != false) {
                t("Voorbereiden…", "Getting ready…").to_owned()
            } else if let (Some(total), Some(left)) = (total, left) {
                let done = kit::format::bytes((total - left).max(0.0));
                let tot = kit::format::bytes(total);
                match j["remaining"].as_f64().filter(|v| v.is_finite() && *v > 0.0) {
                    Some(eta) => common::fill(
                        t("{done} van {total} · nog ongeveer {eta}", "{done} of {total} · about {eta} left"),
                        &[("done", &done), ("total", &tot), ("eta", &kit::format::duration_long(eta))],
                    ),
                    None => common::fill(t("{done} van {total}", "{done} of {total}"), &[("done", &done), ("total", &tot)]),
                }
            } else {
                String::new()
            };
            let m = HeroModel::new(Tone::Accent, t("Updates downloaden", "Downloading updates"), sub).busy();
            if allow_cancel { m.act(Action::Cancel) } else { m }
        }
        "installing" => HeroModel::new(Tone::Accent, t("Updates installeren", "Installing updates"), t("Laat je pc aan staan.", "Keep your PC turned on.").into()).busy(),
        "waiting" => {
            if j["status"] == 31 {
                return HeroModel::new(
                    Tone::Accent,
                    t("Wachten op toestemming", "Waiting for permission"),
                    t("Bevestig in het venster dat om je wachtwoord vraagt.", "Confirm in the window that asks for your password.").into(),
                );
            }
            let sub = if j["lockHolder"] == "unattended" {
                t("Ubuntu installeert op dit moment automatische updates.", "Ubuntu is installing automatic updates right now.")
            } else {
                t("Een ander programma installeert software.", "Another program is installing software.")
            };
            let m = HeroModel::new(Tone::Accent, t("Wachten tot een andere installatie klaar is", "Waiting for another installation to finish"), sub.into());
            if allow_cancel { m.act(Action::Cancel) } else { m }
        }
        "foreign" => {
            let title = if j["foreignOwn"] == true {
                common::fill(t("{product} installeert of verwijdert een app", "{product} is installing or removing an app"), &[("product", PRODUCT)])
            } else {
                t("Er wordt software geïnstalleerd door een ander programma", "Another program is installing software").into()
            };
            HeroModel {
                disabled: true,
                ..HeroModel::new(Tone::Accent, &title, t("Je kunt updates installeren zodra dat klaar is.", "You can install updates as soon as that’s finished.").into())
                    .act(Action::InstallAll)
            }
        }
        "releaseRunning" => {
            let v = {
                let a = common::s(&s["release"]["version"]);
                if a.is_empty() { common::s(&s["release"]["released"]["version"]) } else { a }
            };
            let title = if v.is_empty() {
                t("Ubuntu wordt bijgewerkt", "Upgrading Ubuntu").to_owned()
            } else {
                common::fill(t("Ubuntu wordt bijgewerkt naar {version}", "Upgrading Ubuntu to {version}"), &[("version", v)])
            };
            HeroModel::new(Tone::Accent, &title, t("Het venster van Ubuntu doet de upgrade. Sluit het niet.", "Ubuntu’s window does the upgrade. Don’t close it.").into())
                .busy()
                .act(Action::Attach)
        }
        "restartRequired" => HeroModel::new(Tone::Caution, t("Opnieuw opstarten vereist", "Restart required"), restart_text(&s["restart"])).act(Action::RestartNow),
        "error" => HeroModel::new(
            Tone::Accent,
            t("Er zijn problemen opgetreden bij het installeren van sommige updates", "There were problems installing some updates"),
            err_message(&s["error"]).unwrap_or_default(),
        )
        .badge(true)
        .act(Action::Retry),
        "unsupported" => {
            let sub = match err_message(&s["error"]) {
                Some(m) if is_test_seam(&s["error"]) => m,
                _ => common::fill(
                    t(
                        "{product} kan updates hier niet zelf installeren. Gebruik de Software-updater van Ubuntu.",
                        "{product} can’t install updates here itself. Use Ubuntu’s Software Updater.",
                    ),
                    &[("product", PRODUCT)],
                ),
            };
            HeroModel::unknown(sub)
        }
        "offline" => {
            let sub = match s["checkedAt"].as_f64().filter(|v| v.is_finite() && *v > 0.0) {
                Some(at) => common::fill(
                    t(
                        "Maak verbinding met internet om te controleren op updates. Laatst gecontroleerd: {when}",
                        "Connect to the internet to check for updates. Last checked: {when}",
                    ),
                    &[("when", &when_text(at))],
                ),
                None => t("Maak verbinding met internet om te controleren op updates.", "Connect to the internet to check for updates.").into(),
            };
            HeroModel::new(Tone::Accent, t("Je bent offline", "You’re offline"), sub).badge(true).act(Action::Retry)
        }
        "repairRequired" => HeroModel::new(
            Tone::Accent,
            t("Een eerdere installatie is niet afgerond", "An earlier installation didn’t finish"),
            t(
                "Ubuntu moet dit eerst herstellen. Daarna kun je weer updates installeren.",
                "Ubuntu needs to repair this first. Then you can install updates again.",
            )
            .into(),
        )
        .badge(true)
        .act(Action::Repair),
        _ => HeroModel::unknown(unknown_desc()),
    }
}

// ------------------------------------------------------------------------------------------ the rows (§3.3)

/// One update row: a settings expander kept across repaints, so an opened row stays open.
struct RowView {
    exp: SettingsExpander,
    cur: RefCell<Value>,
    state: gtk::Label,
    pbar: ProgressBar,
    retry: gtk::Button,
    details: gtk::Box,
    /// the snapshot rev the package list was filled for (None: never)
    filled: RefCell<Option<Value>>,
}

// ------------------------------------------------------------------------------------------ the section

type Waiters = Vec<Box<dyn FnOnce(())>>;
type PkgWaiters = Vec<Box<dyn FnOnce(Vec<Value>)>>;

struct Upd {
    p: Page,
    weak: Weak<Upd>,
    /// the last update.status snapshot; None = no update.* service here (or it failed)
    s: RefCell<Option<Value>>,
    /// without update.*: the system.updates answer, or None
    legacy: RefCell<Option<Value>>,
    /// update.status is an unknown method on this host
    legacy_mode: Cell<bool>,
    /// (rev, list) of update.status {packages: true}
    pkg_cache: RefCell<Option<(Value, Vec<Value>)>>,
    /// the packages request in flight: its rev and who waits for it (one per rev)
    pkg_req: RefCell<Option<(Value, PkgWaiters)>>,
    /// an immediate refusal of a write (busy, stale, cannot_cancel, ...): an info bar
    call_error: RefCell<Option<Value>>,
    /// the open metered / removals confirm
    confirm_open: RefCell<Option<(String, Dialog)>>,
    /// the key of the confirm answered last: not shown again while the job still has it
    confirm_answered: RefCell<Option<String>>,
    /// buttons whose action is in flight: they stay disabled through every repaint
    pending: RefCell<Vec<gtk::Button>>,
    /// shell.restartAll answered (not a dry run): every Win11 process restarts, so its button stays disabled
    restart_all_sent: Cell<bool>,
    hero: crate::widgets::Wrap,
    glyph: gtk::Box,
    glyph_key: RefCell<String>,
    disc: RefCell<Option<Disc>>,
    title: gtk::Label,
    sub: gtk::Label,
    actions: gtk::Box,
    btn: gtk::Button,
    model: RefCell<Option<HeroModel>>,
    bars: gtk::Box,
    rows_el: gtk::Box,
    rows_list: gtk::Box,
    views: RefCell<HashMap<String, Rc<RowView>>>,
    release: ReleaseCard,
    prompt: PromptRow,
    loading: Cell<bool>,
    again: Cell<bool>,
    waiters: RefCell<Waiters>,
}

impl Upd {
    fn new(p: &Page) -> Rc<Upd> {
        Rc::new_cyclic(|weak: &Weak<Upd>| {
            let hero = crate::widgets::Wrap::new(20, 20);
            hero.add_css_class("st-upd");
            hero.set_widget_name("update-hero");
            let glyph = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            glyph.set_size_request(64, 64);
            glyph.set_valign(gtk::Align::Center);
            glyph.set_hexpand(false);
            let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
            text.set_hexpand(true);
            text.set_valign(gtk::Align::Center);
            text.set_size_request(240, -1);
            let title = c::label("", "st-upd-title", true);
            title.set_widget_name("update-title");
            let sub = c::label("", "st-upd-sub", true);
            sub.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            text.append(&title);
            text.append(&sub);
            let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            actions.set_valign(gtk::Align::Center);
            actions.set_visible(false);
            hero.append(&glyph);
            hero.append(&text);
            hero.append(&actions);
            let btn = c::button(Action::CheckFor.label(), Variant::Accent);
            btn.set_widget_name("update-action");
            let w = weak.clone();
            btn.connect_clicked(move |b| {
                let Some(u) = w.upgrade() else { return };
                let action = u.model.borrow().as_ref().and_then(|m| m.action);
                if let Some(a) = action
                    && b.is_sensitive()
                {
                    u.run_action(a, b);
                }
            });
            let bars = gtk::Box::new(gtk::Orientation::Vertical, 8);
            bars.add_css_class("st-upd-bars");
            bars.set_visible(false);
            let rows_el = gtk::Box::new(gtk::Orientation::Vertical, 4);
            rows_el.add_css_class("st-updrows");
            rows_el.set_visible(false);
            let rows_list = gtk::Box::new(gtk::Orientation::Vertical, 4);
            let host = Host {
                reload: {
                    let w = weak.clone();
                    Rc::new(move || match w.upgrade() {
                        Some(u) => u.load(),
                        None => {
                            let (fill, wait) = later::<()>();
                            fill(());
                            wait
                        }
                    })
                },
                restart_now: {
                    let pp = p.clone();
                    Rc::new(move || restart_now(&pp))
                },
            };
            Upd {
                p: p.clone(),
                weak: weak.clone(),
                s: RefCell::new(None),
                legacy: RefCell::new(None),
                legacy_mode: Cell::new(false),
                pkg_cache: RefCell::new(None),
                pkg_req: RefCell::new(None),
                call_error: RefCell::new(None),
                confirm_open: RefCell::new(None),
                confirm_answered: RefCell::new(None),
                pending: RefCell::new(Vec::new()),
                restart_all_sent: Cell::new(false),
                hero,
                glyph,
                glyph_key: RefCell::new(String::new()),
                disc: RefCell::new(None),
                title,
                sub,
                actions,
                btn,
                model: RefCell::new(None),
                bars,
                rows_el,
                rows_list,
                views: RefCell::new(HashMap::new()),
                release: ReleaseCard::new(p, host.clone()),
                prompt: PromptRow::new(p, host),
                loading: Cell::new(false),
                again: Cell::new(false),
                waiters: RefCell::new(Vec::new()),
            }
        })
    }

    fn rev(&self) -> Value {
        self.s.borrow().as_ref().map(|s| s["rev"].clone()).unwrap_or(Value::Null)
    }

    fn phase(&self) -> String {
        self.s.borrow().as_ref().map(|s| common::value_key(&s["phase"])).unwrap_or_default()
    }

    fn is_pending(&self, b: &gtk::Button) -> bool {
        self.pending.borrow().iter().any(|x| x == b)
    }

    // ---- host calls

    /// Read update.status (and system.updates without update.*) and repaint; a call while one runs reads once
    /// more after it. The answer comes when the reading is done.
    fn load(self: &Rc<Self>) -> Later<()> {
        let (fill, wait) = later::<()>();
        self.waiters.borrow_mut().push(Box::new(fill));
        if self.loading.get() {
            self.again.set(true);
            return wait;
        }
        self.loading.set(true);
        let u = self.clone();
        self.p.spawn(async move {
            loop {
                u.again.set(false);
                // the regular snapshot never carries the package list; packages() asks for it when a row needs it
                let r = u.p.call("update.status", json!({})).await;
                if !u.p.alive() {
                    break;
                }
                let legacy_mode = matches!(&r, Err(e) if e.code == "unknown_method");
                *u.s.borrow_mut() = r.ok().filter(Value::is_object);
                u.legacy_mode.set(legacy_mode);
                *u.legacy.borrow_mut() = None;
                if legacy_mode {
                    let l = u.p.call("system.updates", json!({})).await;
                    if !u.p.alive() {
                        break;
                    }
                    *u.legacy.borrow_mut() = l.ok().filter(Value::is_object);
                }
                u.paint(true);
                if !(u.again.get() && u.p.alive()) {
                    break;
                }
            }
            u.loading.set(false);
            let ws: Waiters = u.waiters.borrow_mut().drain(..).collect();
            for w in ws {
                w(());
            }
        });
        wait
    }

    /// The package list of the current rev: one update.status {packages: true} per rev, shared by every open row.
    fn packages(self: &Rc<Self>, done: impl FnOnce(Vec<Value>) + 'static) {
        let rev = self.rev();
        if let Some((r, list)) = self.pkg_cache.borrow().as_ref()
            && *r == rev
        {
            let list = list.clone();
            done(list);
            return;
        }
        {
            let mut req = self.pkg_req.borrow_mut();
            if let Some((r, waiters)) = req.as_mut()
                && *r == rev
            {
                waiters.push(Box::new(done));
                return;
            }
            *req = Some((rev.clone(), vec![Box::new(done)]));
        }
        let fut = self.p.call("update.status", json!({ "packages": true }));
        let u = self.clone();
        self.p.spawn(async move {
            let r = fut.await;
            let list = match r {
                Ok(v) if v["packages"].is_array() => {
                    let list = v["packages"].as_array().cloned().unwrap_or_default();
                    *u.pkg_cache.borrow_mut() = Some((v["rev"].clone(), list.clone()));
                    list
                }
                _ => Vec::new(),
            };
            let waiters = {
                let mut req = u.pkg_req.borrow_mut();
                match req.as_ref() {
                    Some((r, _)) if *r == rev => req.take().map(|(_, w)| w).unwrap_or_default(),
                    _ => Vec::new(),
                }
            };
            for w in waiters {
                w(list.clone());
            }
        });
    }

    /// Run `fut` with `button` disabled; a second click while it runs does nothing (every repaint keeps it
    /// disabled). None when the button was busy already.
    async fn holding<T>(self: &Rc<Self>, button: Option<&gtk::Button>, fut: impl std::future::Future<Output = T>) -> Option<T> {
        let Some(b) = button else { return Some(fut.await) };
        if self.is_pending(b) {
            return None;
        }
        self.pending.borrow_mut().push(b.clone());
        b.set_sensitive(false);
        let r = fut.await;
        self.pending.borrow_mut().retain(|x| x != b);
        Some(r)
    }

    /// A write method: refusals become the info bar (by key / code), a dry run gets its toast, then the page reads
    /// again (and repaints every button's own state); `on_ok(value)` runs on an ok answer, before that repaint.
    async fn write(self: Rc<Self>, method: &str, args: Value, button: Option<gtk::Button>, on_ok: Option<Box<dyn FnOnce(&Value)>>) -> Option<Value> {
        if button.as_ref().is_some_and(|b| self.is_pending(b)) {
            return None;
        }
        let wait = self.p.call(method, args);
        let r = self.holding(button.as_ref(), wait).await?;
        if !self.p.alive() {
            return None;
        }
        match &r {
            Err(e) => {
                if e.code == "unknown_method" {
                    self.p.error(e, None);
                } else {
                    *self.call_error.borrow_mut() = Some(err_value(e));
                }
            }
            Ok(v) => {
                *self.call_error.borrow_mut() = None;
                common::note_dry_run(&self.p.anchor(), v);
                if let Some(f) = on_ok {
                    f(v);
                }
            }
        }
        self.load().await;
        r.ok()
    }

    fn spawn_write(self: &Rc<Self>, method: &'static str, args: Value, button: Option<gtk::Button>) {
        let u = self.clone();
        self.p.spawn(async move {
            let _ = u.write(method, args, button, None).await;
        });
    }

    /// Hero actions that are no update.* write: the button stays disabled until they end, then follows the state.
    fn hero_hold(self: &Rc<Self>, b: &gtk::Button, fut: impl std::future::Future<Output = ()> + 'static) {
        let (u, b) = (self.clone(), b.clone());
        self.p.spawn(async move {
            let _ = u.holding(Some(&b), fut).await;
            if u.p.alive() {
                u.paint_hero();
            }
        });
    }

    fn run_action(self: &Rc<Self>, a: Action, b: &gtk::Button) {
        let b = Some(b.clone());
        match a {
            Action::CheckFor => {
                // the click also asks Ubuntu for a new version (at most daily: its answer and any refusal stay
                // silent; the result reaches the release card through update.changed)
                let _ = self.p.call("update.release.check", json!({}));
                self.spawn_write("update.check", json!({}), b);
            }
            Action::InstallAll => self.spawn_write("update.install", json!({}), b),
            Action::Cancel => {
                let job_id = self.s.borrow().as_ref().map(|s| s["job"]["jobId"].clone()).unwrap_or(Value::Null);
                self.spawn_write("update.cancel", json!({ "jobId": job_id }), b);
            }
            Action::Attach => self.spawn_write("update.release.attach", json!({}), b),
            Action::Repair => self.spawn_write("update.repair", json!({}), b),
            Action::RestartNow => {
                let fut = restart_now(&self.p);
                self.hero_hold(&self.btn, fut);
            }
            Action::OpenUpdater => {
                let (fill, wait) = later::<()>();
                super::update_advanced::open_updater(&self.p, move || fill(()));
                self.hero_hold(&self.btn, wait);
            }
            Action::LegacyCheck => {
                let fut = self.load();
                self.hero_hold(&self.btn, fut);
            }
            Action::Retry => {
                // a failed install is tried again; a failed check (or nothing left to install) checks again
                let install = self.s.borrow().as_ref().is_some_and(|s| {
                    s["phase"] == "error" && s["rows"].as_array().is_some_and(|r| !r.is_empty()) && !is_recheck(&s["error"])
                });
                self.spawn_write(if install { "update.install" } else { "update.check" }, json!({}), b);
            }
        }
    }

    // ---- painting

    fn paint_hero(&self) {
        let m = hero_model(self.s.borrow().as_ref(), self.legacy.borrow().as_ref());
        for cls in ["st-ok", "st-accent", "st-caution", "st-busy"] {
            self.hero.remove_css_class(cls);
        }
        self.hero.add_css_class(match m.tone {
            Tone::Ok => "st-ok",
            Tone::Accent => "st-accent",
            Tone::Caution => "st-caution",
        });
        if m.busy {
            self.hero.add_css_class("st-busy");
        }
        let key = format!("{}|{}", if m.ring { "ring" } else { m.glyph }, m.badge.map(|e| if e { "error" } else { "ok" }).unwrap_or(""));
        if *self.glyph_key.borrow() != key {
            *self.glyph_key.borrow_mut() = key;
            c::clear(&self.glyph);
            *self.disc.borrow_mut() = None;
            if m.ring {
                let ring = ProgressRing::new(40, None);
                ring.widget().set_hexpand(true);
                self.glyph.append(ring.widget());
            } else {
                let ov = gtk::Overlay::new();
                let disc = Disc::new(64, 32, m.glyph);
                ov.set_child(Some(disc.widget()));
                if let Some(error) = m.badge {
                    let badge = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                    badge.add_css_class("st-upd-badge");
                    if error {
                        badge.add_css_class("st-error");
                    }
                    badge.set_halign(gtk::Align::End);
                    badge.set_valign(gtk::Align::End);
                    badge.set_size_request(22, 22);
                    let g = kit::icons::glyph_stroke(if error { "close" } else { "check" }, 12, 2.2);
                    g.set_hexpand(true);
                    g.set_halign(gtk::Align::Center);
                    g.set_valign(gtk::Align::Center);
                    badge.append(&g);
                    ov.add_overlay(&badge);
                }
                self.glyph.append(&ov);
                *self.disc.borrow_mut() = Some(disc);
            }
        }
        if let Some(d) = self.disc.borrow().as_ref() {
            d.set_tone(m.tone);
            d.set_glyph(m.glyph);
            d.set_busy(m.busy);
        }
        self.title.set_label(&m.title);
        self.sub.set_label(&m.sub);
        self.sub.set_visible(!m.sub.is_empty());
        match m.action {
            // states without a button (checking, installing, ...)
            None => {
                if self.btn.parent().is_some() {
                    self.actions.remove(&self.btn);
                }
                self.actions.set_visible(false);
            }
            Some(a) => {
                if self.btn.parent().is_none() {
                    self.actions.append(&self.btn);
                }
                self.actions.set_visible(true);
                self.btn.set_label(a.label());
                if a.variant() == Variant::Accent {
                    self.btn.add_css_class("w11-accent");
                } else {
                    self.btn.remove_css_class("w11-accent");
                }
                self.btn.set_sensitive(!m.disabled && !self.is_pending(&self.btn));
            }
        }
        *self.model.borrow_mut() = Some(m);
    }

    fn paint_bars(&self) {
        c::clear(&self.bars);
        let s = self.s.borrow().clone();
        if let Some(s) = s {
            let phase = common::s(&s["phase"]).to_owned();
            if phase == "error"
                && let Some(detail) = s["error"]["detail"].as_str().filter(|d| !d.is_empty())
            {
                let exp = Expander::new("", t("Technische details", "Technical details"), "");
                exp.header_button().set_widget_name("update-details");
                let l = c::label(detail, "st-upd-detail w11-mono w11-caption", true);
                l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                l.set_selectable(true);
                let sc = gtk::ScrolledWindow::new();
                sc.set_hscrollbar_policy(gtk::PolicyType::Never);
                sc.set_propagate_natural_height(true);
                sc.set_max_content_height(240);
                sc.set_child(Some(&l));
                exp.set_content(&sc);
                self.bars.append(exp.widget());
            }
            let r = &s["restart"];
            let parts: Vec<String> = r["shellParts"].as_array().map(|a| a.iter().map(common::value_key).collect()).unwrap_or_default();
            // the Win11 restart info bar (§3.4): hidden while a reboot is pending, except when WebKitGTK changed
            if r["shell"] == true && (r["reboot"] != true || parts.iter().any(|p| p == "webkit" || p == "jsc")) {
                let mut names: Vec<String> = Vec::new();
                for p in &parts {
                    let n = part_name(p);
                    if !names.contains(&n) {
                        names.push(n);
                    }
                }
                let list = if names.is_empty() { PRODUCT.to_owned() } else { kit::i18n::list(&names.iter().map(String::as_str).collect::<Vec<_>>()) };
                let b = bar(
                    Severity::Warning,
                    &common::fill(t("{product} moet opnieuw starten", "{product} needs to restart"), &[("product", PRODUCT)]),
                    &common::fill(
                        t(
                            "Onderdelen van het bureaublad zijn bijgewerkt ({list}). Opnieuw starten duurt een paar seconden; vensters van {product} gaan even dicht.",
                            "Parts of the desktop were updated ({list}). Restarting takes a few seconds; {product} windows close briefly.",
                        ),
                        &[("list", &list), ("product", PRODUCT)],
                    ),
                );
                let rb = c::button(&common::fill(t("{product} opnieuw starten", "Restart {product}"), &[("product", PRODUCT)]), Variant::Standard);
                rb.set_widget_name("update-restart-all");
                rb.set_sensitive(!self.restart_all_sent.get());
                let w = self.weak.clone();
                rb.connect_clicked(move |b| {
                    let Some(u) = w.upgrade() else { return };
                    if u.restart_all_sent.get() {
                        return;
                    }
                    let w2 = u.weak.clone();
                    let on_ok: Box<dyn FnOnce(&Value)> = Box::new(move |v: &Value| {
                        if v["dryRun"] != true
                            && let Some(u) = w2.upgrade()
                        {
                            u.restart_all_sent.set(true);
                        }
                    });
                    let (u2, b) = (u.clone(), b.clone());
                    u.p.spawn(async move {
                        let _ = u2.write("shell.restartAll", json!({ "reason": "libraries" }), Some(b), Some(on_ok)).await;
                    });
                });
                b.set_action_widget(&rb);
                self.bars.append(b.widget());
            }
            let rows = s["rows"].as_array().cloned().unwrap_or_default();
            if s["network"]["metered"] == true && rows.iter().any(|x| x["state"] == "pendingDownload" || x["state"] == "downloading") {
                self.bars.append(
                    bar(
                        Severity::Info,
                        "",
                        t(
                            "Je gebruikt een verbinding met een datalimiet. Downloaden kan kosten met zich meebrengen.",
                            "You’re on a metered connection. Downloading may cost you data charges.",
                        ),
                    )
                    .widget(),
                );
            }
            for w in s["warnings"].as_array().into_iter().flatten() {
                if let Some(m) = err_message(w) {
                    self.bars.append(bar(Severity::Warning, "", &m).widget());
                }
            }
            if let Some(m) = err_message(&s["error"])
                && !matches!(phase.as_str(), "error" | "repairRequired" | "unsupported" | "offline")
            {
                self.bars.append(bar(bar_severity(&s["error"]), "", &m).widget());
            }
            let ce = self.call_error.borrow().clone();
            if let Some(ce) = ce
                && let Some(m) = err_message(&ce)
            {
                let b = InfoBar::new(bar_severity(&ce), "", &m);
                b.widget().add_css_class("st-notice");
                b.widget().set_widget_name("update-call-error");
                let w = self.weak.clone();
                b.connect_closed(move || {
                    if let Some(u) = w.upgrade() {
                        *u.call_error.borrow_mut() = None;
                    }
                });
                self.bars.append(b.widget());
            }
        }
        self.bars.set_visible(self.bars.first_child().is_some());
    }

    fn row_view(self: &Rc<Self>, row: &Value) -> Rc<RowView> {
        let id = common::value_key(&row["id"]);
        let exp = SettingsExpander::new("", &row_title(row), &row_caption(row));
        exp.widget().add_css_class("st-updrow");
        exp.header_button().set_widget_name(&format!("update-row-{id}"));
        let state = c::label("", "st-updrow-state", false);
        state.set_xalign(1.0);
        state.set_halign(gtk::Align::End);
        let pbar = ProgressBar::new();
        pbar.widget().set_size_request(160, -1);
        pbar.widget().set_hexpand(false);
        pbar.widget().set_halign(gtk::Align::End);
        pbar.widget().set_visible(false);
        let status = gtk::Box::new(gtk::Orientation::Vertical, 6);
        status.add_css_class("st-updrow-status");
        status.set_size_request(160, -1);
        status.set_valign(gtk::Align::Center);
        status.append(&state);
        status.append(pbar.widget());
        let retry = c::button(t("Opnieuw proberen", "Retry"), Variant::Standard);
        retry.set_widget_name(&format!("update-row-retry-{id}"));
        retry.set_visible(false);
        exp.header().set_control(&status);
        exp.header().add_control(&retry);
        let details = gtk::Box::new(gtk::Orientation::Vertical, 0);
        details.add_css_class("st-updrow-details");
        exp.add(&details);
        let v = Rc::new(RowView { exp, cur: RefCell::new(row.clone()), state, pbar, retry, details, filled: RefCell::new(None) });
        let (w, vw) = (self.weak.clone(), Rc::downgrade(&v));
        v.retry.connect_clicked(move |b| {
            if let (Some(u), Some(v)) = (w.upgrade(), vw.upgrade()) {
                let id = v.cur.borrow()["id"].clone();
                u.spawn_write("update.install", json!({ "rows": [id] }), Some(b.clone()));
            }
        });
        let (w, vw) = (self.weak.clone(), Rc::downgrade(&v));
        v.exp.connect_toggled(move |open| {
            if open && let (Some(u), Some(v)) = (w.upgrade(), vw.upgrade()) {
                u.fill_row(&v);
            }
        });
        self.update_row(&v, row);
        v
    }

    /// The row's packages ("name from → to", the summary, "Wordt later aangeboden") and "Wat is er nieuw".
    fn fill_row(self: &Rc<Self>, v: &Rc<RowView>) {
        let (w, vw) = (self.weak.clone(), Rc::downgrade(v));
        self.packages(move |all| {
            let (Some(u), Some(v)) = (w.upgrade(), vw.upgrade()) else { return };
            if !u.p.alive() {
                return;
            }
            *v.filled.borrow_mut() = Some(u.rev());
            let cur = v.cur.borrow().clone();
            let mine: Vec<Value> = all.into_iter().filter(|p| p["row"] == cur["id"]).collect();
            c::clear(&v.details);
            let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
            for p in &mine {
                let item = gtk::Box::new(gtk::Orientation::Horizontal, 12);
                item.add_css_class("st-updrow-pkg");
                let text = gtk::Box::new(gtk::Orientation::Vertical, 0);
                text.set_hexpand(true);
                text.set_valign(gtk::Align::Center);
                let name = common::value_key(&p["name"]);
                let to = common::value_key(&p["to"]);
                let from = common::value_key(&p["from"]);
                let line = if !from.is_empty() && from != "-" { format!("{name} {from} → {to}") } else { format!("{name} {to}") };
                let n = c::label(&line, "st-updrow-pkgname", true);
                n.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                n.set_selectable(true);
                text.append(&n);
                if let Some(sum) = p["summary"].as_str().filter(|s| !s.is_empty()) {
                    text.append(&c::label(sum, "st-updrow-pkgsum", true));
                }
                item.append(&text);
                if p["phased"] == true || p["held"] == true {
                    let tag = c::label(
                        if p["phased"] == true { t("Wordt later aangeboden", "Offered later") } else { t("Wordt nu niet geïnstalleerd", "Not being installed right now") },
                        "st-updrow-tag",
                        false,
                    );
                    tag.set_ellipsize(gtk::pango::EllipsizeMode::None);
                    tag.set_valign(gtk::Align::Center);
                    item.append(&tag);
                }
                list.append(&item);
            }
            v.details.append(&list);
            let ids: Vec<Value> = mine.iter().filter(|p| p["phased"] != true && p["held"] != true).map(|p| p["id"].clone()).take(20).collect();
            if !ids.is_empty() {
                let link = c::hyperlink(t("Wat is er nieuw", "What’s new"), None);
                link.set_widget_name(&format!("update-whatsnew-{}", common::value_key(&cur["id"])));
                let w = u.weak.clone();
                let (row, ids, pkgs) = (cur.clone(), ids.clone(), mine.clone());
                link.connect_clicked(move |_| {
                    if let Some(u) = w.upgrade() {
                        u.whats_new(&row, ids.clone(), pkgs.clone());
                    }
                });
                let links = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                links.add_css_class("st-updrow-links");
                links.append(&link);
                v.details.append(&links);
            }
        });
    }

    fn update_row(self: &Rc<Self>, v: &Rc<RowView>, r: &Value) {
        *v.cur.borrow_mut() = r.clone();
        v.exp.header().set_title(&row_title(r));
        v.exp.header().set_description(&row_caption(r));
        let title = row_title(r);
        v.pbar.widget().update_property(&[gtk::accessible::Property::Label(&title)]);
        v.state.set_label(&row_state(r));
        let state = common::s(&r["state"]);
        let failed = state == "failed";
        if moving(state) || failed {
            v.pbar.widget().set_visible(true);
            let pct = if failed { r["pct"].as_f64().filter(|p| p.is_finite()).map(|_| pct_of(&r["pct"])).unwrap_or(100.0) } else { pct_of(&r["pct"]) };
            v.pbar.set_fraction(pct / 100.0);
            v.pbar.set_state(if failed { ProgressState::Error } else { ProgressState::Normal });
        } else {
            v.pbar.widget().set_visible(false);
        }
        v.retry.set_visible(failed);
        v.retry.set_sensitive(!(self.s.borrow().is_some() && busy_phase(&self.phase())) && !self.is_pending(&v.retry));
        if failed {
            v.exp.widget().add_css_class("st-failed");
        } else {
            v.exp.widget().remove_css_class("st-failed");
        }
        if v.exp.expanded() && v.filled.borrow().as_ref() != Some(&self.rev()) {
            self.fill_row(v);
        }
    }

    fn paint_rows(self: &Rc<Self>, structural: bool) {
        let s = self.s.borrow().clone();
        let rows: Vec<Value> = match &s {
            Some(s) if s["phase"] != "checking" => s["rows"].as_array().cloned().unwrap_or_default(),
            _ => Vec::new(),
        };
        if !structural {
            for r in &rows {
                let v = self.views.borrow().get(&common::value_key(&r["id"])).cloned();
                if let Some(v) = v {
                    self.update_row(&v, r);
                }
            }
            return;
        }
        let keep: Vec<String> = rows.iter().map(|r| common::value_key(&r["id"])).collect();
        self.views.borrow_mut().retain(|id, _| keep.contains(id));
        c::clear(&self.rows_el);
        c::clear(&self.rows_list);
        let phase = s.as_ref().map(|s| common::value_key(&s["phase"])).unwrap_or_default();
        if !rows.is_empty() && phase == "restartRequired" {
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            head.add_css_class("st-updrows-head");
            let title = c::label(t("Ook beschikbaar", "Also available"), "st-updrows-title", false);
            title.set_hexpand(true);
            title.set_valign(gtk::Align::Center);
            head.append(&title);
            let all = c::button(Action::InstallAll.label(), Variant::Standard);
            all.set_widget_name("update-install-also");
            all.set_valign(gtk::Align::Center);
            let w = self.weak.clone();
            all.connect_clicked(move |b| {
                if let Some(u) = w.upgrade() {
                    u.spawn_write("update.install", json!({}), Some(b.clone()));
                }
            });
            head.append(&all);
            self.rows_el.append(&head);
        }
        if !rows.is_empty() {
            for r in &rows {
                let id = common::value_key(&r["id"]);
                let existing = self.views.borrow().get(&id).cloned();
                let v = match existing {
                    Some(v) => {
                        self.update_row(&v, r);
                        v
                    }
                    None => {
                        let v = self.row_view(r);
                        self.views.borrow_mut().insert(id, v.clone());
                        v
                    }
                };
                self.rows_list.append(v.exp.widget());
            }
            self.rows_el.append(&self.rows_list);
        }
        if let Some(s) = &s
            && s["phase"] != "checking"
        {
            let tot = &s["totals"];
            let n = |v: &Value| v.as_f64().filter(|v| v.is_finite()).map(|v| v as i64).unwrap_or(0);
            if n(&tot["phased"]) > 0 {
                self.rows_el.append(&c::label(
                    &common::plural(n(&tot["phased"]), t("1 update wordt later aangeboden", "1 update will be offered later"), t("{n} updates worden later aangeboden", "{n} updates will be offered later")),
                    "st-updrows-note",
                    true,
                ));
            }
            if n(&tot["held"]) > 0 {
                self.rows_el.append(&c::label(
                    &common::plural(
                        n(&tot["held"]),
                        t("1 update wordt nu niet geïnstalleerd", "1 update isn’t being installed right now"),
                        t("{n} updates worden nu niet geïnstalleerd", "{n} updates aren’t being installed right now"),
                    ),
                    "st-updrows-note",
                    true,
                ));
            }
        }
        self.rows_el.set_visible(self.rows_el.first_child().is_some());
    }

    /// The metered / removals confirm of a paused job (§3.2 dialogs, job.confirm). Once answered, the same confirm
    /// is not shown again while a reload still sees it (update.confirm in flight); a failed answer shows it again.
    fn paint_confirm(self: &Rc<Self>) {
        let s = self.s.borrow().clone();
        let c = s.as_ref().map(|s| s["job"]["confirm"].clone()).filter(Value::is_object);
        let job_id = s.as_ref().map(|s| s["job"]["jobId"].clone()).unwrap_or(Value::Null);
        let key = match (&c, job_id.is_null() || job_id == false) {
            (Some(c), false) => Some(format!("{}:{}", common::value_key(&job_id), common::value_key(&c["kind"]))),
            _ => None,
        };
        let open_key = self.confirm_open.borrow().as_ref().map(|(k, _)| k.clone());
        if let Some(ok) = open_key
            && Some(&ok) != key.as_ref()
        {
            let open = self.confirm_open.borrow_mut().take();
            if let Some((_, d)) = open {
                d.close(DialogResult::Close);
            }
        }
        let answered = self.confirm_answered.borrow().clone();
        if answered.is_some() && answered != key {
            *self.confirm_answered.borrow_mut() = None;
        }
        let Some(key) = key else { return };
        let Some(c) = c else { return };
        if self.confirm_open.borrow().is_some() || self.confirm_answered.borrow().as_deref() == Some(key.as_str()) {
            return;
        }
        let metered = c["kind"] == "metered";
        let body = if metered {
            let size = c["size"].as_f64().or_else(|| s.as_ref().and_then(|s| s["totals"]["downloadSize"].as_f64())).unwrap_or(f64::NAN);
            common::fill(t("Er wordt ongeveer {size} gedownload.", "About {size} will be downloaded."), &[("size", &kit::format::bytes(size))])
        } else {
            let names: Vec<String> = c["removals"].as_array().map(|a| a.iter().map(common::value_key).collect()).unwrap_or_default();
            common::fill(
                t("Voor deze updates moet {list} worden verwijderd. Toch installeren?", "These updates need {list} to be removed. Install anyway?"),
                &[("list", &kit::i18n::list(&names.iter().map(String::as_str).collect::<Vec<_>>()))],
            )
        };
        let d = Dialog::new(if metered { t("Toch downloaden?", "Download anyway?") } else { t("Pakketten verwijderen?", "Remove packages?") })
            .text(&body)
            .icon("warning")
            .primary(if metered { t("Downloaden", "Download") } else { t("Toch installeren", "Install anyway") })
            .close_label(t("Annuleren", "Cancel"));
        *self.confirm_open.borrow_mut() = Some((key.clone(), d.clone()));
        let w = self.weak.clone();
        d.show(&self.p.anchor(), move |r| {
            let Some(u) = w.upgrade() else { return };
            if u.confirm_open.borrow().as_ref().map(|(k, _)| k.as_str()) != Some(key.as_str()) {
                return;
            }
            *u.confirm_open.borrow_mut() = None;
            *u.confirm_answered.borrow_mut() = Some(key.clone());
            if !u.p.alive() {
                return;
            }
            let u2 = u.clone();
            u.p.spawn(async move {
                let v = u2.clone().write("update.confirm", json!({ "jobId": job_id, "accept": r == DialogResult::Primary }), None, None).await;
                if v.is_none() && u2.p.alive() && u2.confirm_answered.borrow().as_deref() == Some(key.as_str()) {
                    *u2.confirm_answered.borrow_mut() = None;
                    u2.paint_confirm();
                }
            });
        });
    }

    fn paint(self: &Rc<Self>, structural: bool) {
        self.paint_hero();
        if structural {
            self.paint_bars();
            let s = self.s.borrow().clone();
            self.release.update(s.as_ref());
            self.prompt.update(s.as_ref());
            self.paint_confirm();
        }
        self.paint_rows(structural);
    }

    /// "Wat is er nieuw" (update.details; the apt backend fetches Ubuntu's changelogs).
    fn whats_new(self: &Rc<Self>, row: &Value, ids: Vec<Value>, pkgs: Vec<Value>) {
        let body = gtk::Box::new(gtk::Orientation::Vertical, 16);
        let wait = gtk::Box::new(gtk::Orientation::Vertical, 0);
        wait.add_css_class("st-updnews-wait");
        wait.append(ProgressRing::new(24, None).widget());
        body.append(&wait);
        let sc = gtk::ScrolledWindow::new();
        sc.set_hscrollbar_policy(gtk::PolicyType::Never);
        sc.set_propagate_natural_height(true);
        sc.set_max_content_height(420);
        sc.set_child(Some(&crate::widgets::Clamp::new(&body, 510)));
        sc.set_widget_name("update-whatsnew");
        Dialog::new(&row_title(row)).body(&sc).close_label(t("Sluiten", "Close")).width(560).show(&self.p.anchor(), |_| {});
        let fut = self.p.call("update.details", json!({ "ids": ids }));
        let p = self.p.clone();
        self.p.spawn(async move {
            let r = fut.await;
            c::clear(&body);
            let none = |text: &str| c::label(text, "st-updnews-none", true);
            let no_details = t("Ubuntu heeft voor deze update geen beschrijving.", "Ubuntu has no description for this update.");
            let v = match r {
                Err(e) => {
                    body.append(&none(if e.message.is_empty() { no_details } else { &e.message }));
                    return;
                }
                Ok(v) => v,
            };
            let links = |urls: &[Value]| -> gtk::Box {
                let b = gtk::Box::new(gtk::Orientation::Vertical, 0);
                b.add_css_class("st-updnews-links");
                for u in urls.iter().filter_map(Value::as_str).filter(|u| u.starts_with("https://")) {
                    let label = u.trim_start_matches("https://");
                    let label = label.strip_prefix("www.").unwrap_or(label);
                    let a = c::hyperlink(label, None);
                    a.set_halign(gtk::Align::Start);
                    let (p, uri) = (p.clone(), u.to_owned());
                    a.connect_clicked(move |_| {
                        let fut = p.run("apps.openUri", json!({ "uri": uri }));
                        p.spawn(async move {
                            let _ = fut.await;
                        });
                    });
                    b.append(&a);
                }
                b
            };
            let mut any = false;
            for d in v["details"].as_array().into_iter().flatten() {
                let text: Vec<&str> = [&d["updateText"], &d["changelog"]].iter().filter_map(|x| x.as_str()).filter(|x| !x.is_empty()).collect();
                let text = text.join("\n\n");
                let cves = d["cves"].as_array().cloned().unwrap_or_default();
                let bugs = d["bugs"].as_array().cloned().unwrap_or_default();
                if text.is_empty() && cves.is_empty() && bugs.is_empty() {
                    continue;
                }
                any = true;
                let sec = gtk::Box::new(gtk::Orientation::Vertical, 0);
                let name = match pkgs.iter().find(|x| x["id"] == d["id"]) {
                    Some(p) => format!("{} {}", common::value_key(&p["name"]), common::value_key(&p["to"])),
                    None => common::value_key(&d["id"]).split(';').take(2).collect::<Vec<_>>().join(" "),
                };
                sec.append(&c::label(&name, "st-updnews-name", true));
                if !text.is_empty() {
                    sec.append(&c::label(t("Wijzigingen", "Changes"), "st-updnews-h", false));
                    let l = c::label(&text, "st-updnews-text w11-mono w11-caption", true);
                    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
                    l.set_selectable(true);
                    sec.append(&l);
                }
                if !cves.is_empty() {
                    sec.append(&c::label(t("Beveiligingsproblemen", "Security issues"), "st-updnews-h", false));
                    sec.append(&links(&cves));
                }
                if !bugs.is_empty() {
                    sec.append(&c::label(t("Opgeloste fouten", "Fixed bugs"), "st-updnews-h", false));
                    sec.append(&links(&bugs));
                }
                body.append(&sec);
            }
            if !any {
                body.append(&none(no_details));
            }
        });
    }

    /// The events (§4.4): structural changes read the snapshot again, progress patches it in place.
    fn listen(self: &Rc<Self>) {
        let w = self.weak.clone();
        self.p.on("update.changed", move |d| {
            let Some(u) = w.upgrade() else { return };
            let stale = u.s.borrow().as_ref().is_none_or(|s| d["rev"] != s["rev"]);
            if stale {
                let _ = u.load();
            }
        });
        // a host without update.*: its own re-checks arrive as system.updates.changed
        let w = self.weak.clone();
        self.p.on("system.updates.changed", move |_| {
            if let Some(u) = w.upgrade()
                && u.legacy_mode.get()
            {
                let _ = u.load();
            }
        });
        let w = self.weak.clone();
        self.p.on("update.progress", move |d| {
            let Some(u) = w.upgrade() else { return };
            let same = {
                let s = u.s.borrow();
                s.as_ref().is_some_and(|s| s["job"].is_object() && d.is_object() && d["jobId"] == s["job"]["jobId"])
            };
            if !same {
                let _ = u.load();
                return;
            }
            if let Some(s) = u.s.borrow_mut().as_mut() {
                for k in ["status", "pct", "speed", "remaining", "downloadLeft", "allowCancel"] {
                    if let Some(v) = d.get(k) {
                        s["job"][k] = v.clone();
                    }
                }
                if let Some(rows) = s["rows"].as_array_mut() {
                    for r in rows {
                        let p = &d["rows"][common::value_key(&r["id"])];
                        if p.is_object() {
                            r["state"] = p["state"].clone();
                            r["pct"] = p["pct"].clone();
                        }
                    }
                }
            }
            u.paint(false);
        });
    }
}

/// Settings' own restart, like Systeem › Aan/uit: the confirm (unless power.confirm is off), then
/// system.power.action reboot, which the host's power guard checks.
fn restart_now(p: &Page) -> Later<()> {
    let (fill, wait) = later::<()>();
    let fill: Rc<RefCell<Option<Box<dyn FnOnce(())>>>> = Rc::new(RefCell::new(Some(Box::new(fill))));
    // the hold ends when the restart was asked for, or when the confirm was cancelled
    let finish = move || {
        let f = fill.borrow_mut().take();
        if let Some(f) = f {
            f(());
        }
    };
    let go = {
        let (pp, finish) = (p.clone(), finish.clone());
        move || {
            let fut = pp.run("system.power.action", json!({ "action": "reboot" }));
            pp.spawn(async move {
                let _ = fut.await;
                finish();
            });
        }
    };
    if common::setting_bool("power.confirm", true) {
        let pp = p.clone();
        Dialog::new(t("Wil je de pc opnieuw opstarten?", "Do you want to restart the PC?"))
            .text(t(
                "Alle apps worden gesloten en de pc start daarna vanzelf weer op. Werk dat je nog niet hebt opgeslagen, kan verloren gaan.",
                "All apps will close and the PC will restart by itself. Any work you haven’t saved might be lost.",
            ))
            .primary(t("Opnieuw opstarten", "Restart"))
            .close_label(t("Annuleren", "Cancel"))
            .show(&p.anchor(), move |r| {
                if r == DialogResult::Primary && pp.alive() {
                    go();
                } else {
                    finish();
                }
            });
    } else {
        go();
    }
    wait
}

// ------------------------------------------------------------------------------------------ backup, reset, about

fn exported_name(v: &Value) -> String {
    if let Some(n) = v["name"].as_str().filter(|s| !s.is_empty()) {
        return n.to_owned();
    }
    // the UTC date, as new Date().toISOString()
    let stamp = glib::DateTime::now_utc().ok().and_then(|d| d.format("%Y-%m-%d").ok()).map(|s| s.to_string()).unwrap_or_default();
    common::fill(t("win11shell-instellingen-{date}.json", "win11shell-settings-{date}.json"), &[("date", &stamp)])
}

/// The Downloads folder (XDG), else ~/Downloads.
fn downloads_dir() -> std::path::PathBuf {
    glib::user_special_dir(glib::UserDirectory::Downloads).unwrap_or_else(|| glib::home_dir().join("Downloads"))
}

/// Write the export into Downloads like a browser download: "name.json", else "name (1).json", ...; the name used.
fn save_download(dir: &std::path::Path, name: &str, text: &str) -> std::io::Result<String> {
    std::fs::create_dir_all(dir)?;
    let (stem, ext) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i..]),
        _ => (name, ""),
    };
    for n in 0..1000 {
        let file = if n == 0 { name.to_owned() } else { format!("{stem} ({n}){ext}") };
        let path = dir.join(&file);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                use std::io::Write;
                f.write_all(text.as_bytes())?;
                return Ok(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, name.to_owned()))
}

fn export(p: &Page, slot: &gtk::Box) {
    let fut = p.call("settings.export", json!({}));
    let (p, slot) = (p.clone(), slot.clone());
    p.clone().spawn(async move {
        let v = match fut.await {
            Ok(v) => v,
            Err(e) => {
                p.error(&e, None);
                return;
            }
        };
        let name = exported_name(&v);
        let text = match &v["json"] {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        };
        let dir = downloads_dir();
        let (fill, wait) = later::<std::io::Result<String>>();
        kit::spawn(move || save_download(&dir, &name, &text), fill);
        let saved = match wait.await {
            Ok(n) => n,
            Err(e) => {
                common::show_error_text(&p.anchor(), &e.to_string(), None);
                return;
            }
        };
        let b = p.call("settings.backupFile", json!({})).await;
        if !p.alive() {
            return;
        }
        let copy = match &b {
            Ok(b) => b["path"].as_str().filter(|s| !s.is_empty()).map(|path| common::fill(t("Er is ook een reservekopie bewaard: {path}", "A backup copy was also kept: {path}"), &[("path", path)])),
            Err(_) => None,
        };
        let bar = bar(
            Severity::Success,
            &common::fill(t("Instellingen opgeslagen als “{name}” in je map Downloads", "Settings saved as “{name}” in your Downloads folder"), &[("name", &saved)]),
            &copy.unwrap_or_default(),
        );
        bar.set_closable(true);
        bar.widget().set_widget_name("update-exported");
        common::set_slot(&slot, &[bar.widget().clone().upcast()]);
    });
}

fn bad_file() -> &'static str {
    t("Dit bestand bevat geen instellingen van Win11 Shell.", "This file doesn’t contain Win11 Shell settings.")
}

fn import(p: &Page, slot: &gtk::Box) {
    let filter = gtk::FileFilter::new();
    filter.add_pattern("*.json");
    filter.add_mime_type("application/json");
    filter.set_name(Some("JSON"));
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::new();
    dialog.set_title(t("Bestand kiezen", "Choose file"));
    dialog.set_filters(Some(&filters));
    dialog.set_default_filter(Some(&filter));
    let window = p.window().window().clone();
    let (p, slot) = (p.clone(), slot.clone());
    dialog.open(Some(&window), gio::Cancellable::NONE, move |res| {
        let Ok(file) = res else { return };
        let name = file.basename().map(|b| b.to_string_lossy().into_owned()).unwrap_or_default();
        let (p, slot) = (p.clone(), slot.clone());
        file.load_contents_async(gio::Cancellable::NONE, move |res| {
            let text = res.ok().and_then(|(bytes, _)| String::from_utf8(bytes.to_vec()).ok()).filter(|s| serde_json::from_str::<Value>(s).is_ok());
            let Some(text) = text else {
                p.toast(Toast::new(bad_file()).severity(Severity::Error).timeout(5000));
                return;
            };
            if !p.alive() {
                return;
            }
            let pp = p.clone();
            Dialog::new(t("Instellingen importeren?", "Import settings?"))
                .text(&common::fill(
                    t(
                        "Je huidige instellingen worden vervangen door die uit “{name}”. Tip: exporteer eerst je huidige instellingen als je ze wilt bewaren.",
                        "Your current settings will be replaced by those in “{name}”. Tip: export your current settings first if you want to keep them.",
                    ),
                    &[("name", &name)],
                ))
                .primary(t("Importeren", "Import"))
                .show(&p.anchor(), move |r| {
                    if r != DialogResult::Primary {
                        return;
                    }
                    let fut = pp.call("settings.import", json!({ "json": text }));
                    let (p, slot) = (pp.clone(), slot.clone());
                    pp.spawn(async move {
                        match fut.await {
                            Ok(_) => {
                                common::toast_success(&p.anchor(), t("Instellingen geïmporteerd", "Settings imported"), 2500);
                                common::set_slot(&slot, &[]);
                            }
                            Err(e) => p.error(&e, Some(bad_file())),
                        }
                    });
                });
        });
    });
}

fn reset(p: &Page) {
    let pp = p.clone();
    Dialog::new(t("Alle instellingen terugzetten?", "Reset all settings?"))
        .text(t(
            "Achtergrond, kleuren, taakbalk, Start en alle andere instellingen van Win11 Shell gaan terug naar de standaard. Je bestanden en apps blijven gewoon bewaard. Dit kan niet ongedaan worden gemaakt, tenzij je eerst exporteert.",
            "Background, colours, taskbar, Start and all other Win11 Shell settings go back to the defaults. Your files and apps are kept. This can’t be undone unless you export first.",
        ))
        .icon("warning")
        .primary(t("Terugzetten", "Reset"))
        .danger()
        .show(&p.anchor(), move |r| {
            if r != DialogResult::Primary {
                return;
            }
            let fut = pp.call("settings.reset", json!({}));
            let p = pp.clone();
            pp.spawn(async move {
                match fut.await {
                    Ok(_) => common::toast_success(&p.anchor(), t("Alle instellingen zijn teruggezet", "All settings were reset"), 2500),
                    Err(e) => p.error(&e, None),
                }
            });
        });
}

fn mode_name(m: &str) -> String {
    match m {
        "layer" => t("Bureaublad", "Desktop").into(),
        "preview" => t("Voorbeeldvenster", "Preview window").into(),
        "serve" => t("Browser", "Browser").into(),
        "app" => t("Los venster", "Separate window").into(),
        other => other.into(),
    }
}

fn about_group(sh: &Value) -> gtk::Widget {
    let some = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let version = some(&sh["version"]);
    let compositor = some(&sh["compositor"]).map(|c| {
        let mut ch = c.chars();
        match ch.next() {
            Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
            None => String::new(),
        }
    });
    let rows: Vec<(&str, Option<String>)> = vec![
        (t("Versie", "Version"), version.clone()),
        (t("Weergave", "View"), some(&sh["mode"]).map(|m| mode_name(&m))),
        (t("Vensterbeheer", "Window manager"), compositor),
        (t("Testmodus", "Test mode"), (sh["dryRun"] == true).then(|| t("Aan (er wordt niets echt gewijzigd)", "On (nothing is actually changed)").to_owned())),
    ];
    let exp = SettingsExpander::new("info", t("Over Win11 Shell", "About Win11 Shell"), t("Een Windows 11-achtig bureaublad voor Ubuntu", "A Windows 11-style desktop for Ubuntu"));
    exp.header_button().set_widget_name("update-about");
    exp.header().set_control(&common::value_label(&version.as_ref().map(|v| format!("{} {v}", t("Versie", "Version"))).unwrap_or_default()));
    let grid = gtk::Grid::new();
    grid.add_css_class("st-specs");
    grid.set_column_spacing(24);
    grid.set_row_spacing(10);
    let mut r = 0;
    for (k, v) in rows {
        let Some(v) = v else { continue };
        let key = c::label(k, "st-spec-key", true);
        key.set_size_request(130, -1);
        key.set_max_width_chars(28);
        key.set_valign(gtk::Align::Start);
        let val = c::label(&v, "st-spec-value", true);
        val.set_selectable(true);
        val.set_hexpand(true);
        val.set_wrap_mode(gtk::pango::WrapMode::WordChar);
        grid.attach(&key, 0, r, 1, 1);
        grid.attach(&val, 1, r, 1, 1);
        r += 1;
    }
    let wrap = gtk::Box::new(gtk::Orientation::Vertical, 0);
    wrap.append(&grid);
    exp.add(&wrap);
    let g = SettingsGroup::new("", "");
    g.add(exp.widget());
    g.widget().clone().upcast()
}

/// Scroll the page so that `w` is at the top (backup.scrollIntoView({block: 'start'})), once it is laid out.
fn scroll_into_view(w: &gtk::Widget) {
    // polled rather than on the frame clock: the page column is laid out once it is in the window
    let weak = w.downgrade();
    let tries = Cell::new(0);
    glib::timeout_add_local(std::time::Duration::from_millis(40), move || {
        let Some(w) = weak.upgrade() else { return glib::ControlFlow::Break };
        tries.set(tries.get() + 1);
        let sw = w.ancestor(gtk::ScrolledWindow::static_type()).and_downcast::<gtk::ScrolledWindow>();
        let ready = w.height() > 0 && sw.as_ref().is_some_and(|sw| sw.vadjustment().upper() > sw.vadjustment().page_size());
        if !ready {
            return if tries.get() < 50 { glib::ControlFlow::Continue } else { glib::ControlFlow::Break };
        }
        if let Some(sw) = sw
            && let Some(content) = sw.child()
        {
            // the viewport's page column (the column's own enter offset is not counted)
            let target = content.first_child().and_then(|clamp| clamp.first_child()).unwrap_or(content);
            if let Some(pt) = w.compute_point(&target, &gtk::graphene::Point::new(0.0, 0.0)) {
                let adj = sw.vadjustment();
                adj.set_value((pt.y() as f64).clamp(adj.lower(), (adj.upper() - adj.page_size()).max(adj.lower())));
            }
        }
        glib::ControlFlow::Break
    });
}

fn update(p: Page) -> PageFuture {
    Box::pin(async move {
        let upd = Upd::new(&p);
        let sh = p.get("shell.info", json!({}));
        upd.listen();
        upd.load().await;
        let sh = sh.await.unwrap_or_else(common::shell_info);
        if !p.alive() {
            return Content::new();
        }
        {
            let u = upd.clone();
            p.on_cleanup(move || {
                let open = u.confirm_open.borrow_mut().take();
                if let Some((_, d)) = open {
                    d.close(DialogResult::Close);
                }
                // the page holds the section while it is shown
                drop(u);
            });
        }

        // ---- Meer opties
        let more = SettingsGroup::new(t("Meer opties", "More options"), "");
        more.add(upd.prompt.widget());
        let hist = SettingsCard::clickable("history", t("Updategeschiedenis", "Update history"), "", Chevron::Right);
        if let Some(b) = hist.button() {
            b.set_widget_name("update-more-history");
        }
        let pp = p.clone();
        hist.connect_clicked(move || pp.go("update-history"));
        more.add(hist.widget());
        let adv = SettingsCard::clickable(
            "settings",
            t("Geavanceerde opties", "Advanced options"),
            t("Melding bij opnieuw opstarten, datalimiet, Software-updater van Ubuntu", "Restart notifications, metered connections, Ubuntu Software Updater"),
            Chevron::Right,
        );
        if let Some(b) = adv.button() {
            b.set_widget_name("update-more-advanced");
        }
        let pp = p.clone();
        adv.connect_clicked(move || pp.go("update-advanced"));
        more.add(adv.widget());

        // ---- backup and reset
        let result_slot = common::slot();
        let export_btn = c::button(t("Exporteren", "Export"), Variant::Standard);
        export_btn.set_widget_name("update-export");
        let (pp, slot) = (p.clone(), result_slot.clone());
        export_btn.connect_clicked(move |_| export(&pp, &slot));
        let import_btn = c::button(t("Bestand kiezen", "Choose file"), Variant::Standard);
        import_btn.set_widget_name("update-import");
        let (pp, slot) = (p.clone(), result_slot.clone());
        import_btn.connect_clicked(move |_| import(&pp, &slot));
        let reset_btn = c::button(t("Terugzetten", "Reset"), Variant::Standard);
        reset_btn.set_widget_name("update-reset");
        let pp = p.clone();
        reset_btn.connect_clicked(move |_| reset(&pp));
        let backup = SettingsGroup::new(
            t("Back-up van instellingen", "Settings backup"),
            t("Bewaar je Win11 Shell-instellingen in een bestand, of zet ze terug", "Save your Win11 Shell settings in a file, or restore them"),
        );
        backup.widget().set_widget_name("update-backup");
        let ex = SettingsCard::new(
            "upload",
            t("Instellingen exporteren", "Export settings"),
            t("Sla al je instellingen op in een bestand (bijvoorbeeld op een USB-stick)", "Save all your settings in a file (for example on a USB stick)"),
        );
        ex.set_control(&export_btn);
        backup.add(ex.widget());
        backup.add(&result_slot);
        let im = SettingsCard::new(
            "download",
            t("Instellingen importeren", "Import settings"),
            t("Zet instellingen terug uit een eerder geëxporteerd bestand", "Restore settings from a file you exported earlier"),
        );
        im.set_control(&import_btn);
        backup.add(im.widget());
        let rs = SettingsCard::new(
            "restart",
            t("Alles terugzetten", "Reset everything"),
            t(
                "Alle instellingen van Win11 Shell terugzetten naar de standaardwaarden. Je bestanden blijven bewaard.",
                "Reset all Win11 Shell settings to their defaults. Your files are kept.",
            ),
        );
        rs.set_control(&reset_btn);
        backup.add(rs.widget());
        // Updategeschiedenis' Herstel row opens this page at the backup and reset group: a one-shot parameter,
        // so a rebuild with the same parameters (a language switch, a refresh) keeps the scroll position
        if p.take_param("section").is_some_and(|s| s == "recovery") {
            scroll_into_view(backup.widget().upcast_ref());
        }

        let out: Content = vec![
            upd.hero.clone().upcast(),
            upd.bars.clone().upcast(),
            upd.release.widget().clone().upcast(),
            upd.rows_el.clone().upcast(),
            more.widget().clone().upcast(),
            backup.widget().clone().upcast(),
            about_group(&sh),
        ];
        out
    })
}
