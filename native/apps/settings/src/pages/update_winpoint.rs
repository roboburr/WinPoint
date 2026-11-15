//! Instellingen › Update en herstel: WinPoint's own updater (notes/claude/plan.md D-G74a and D-G74), the card at the
//! top of the page (the updater design §7.1). It shows `winpoint.status`, which every host reads from the owner's
//! files (the owner is win11-shell's backend):
//!
//! * the check (phase 1): never checked, checking, up to date with the time of the last check, or why the last check
//!   failed (offline, GitHub's rate limit, the releases gone, a failed request with its raw detail in an expander)
//!   with the last known result under it. "Check for updates" is `winpoint.check`;
//! * a newer release: its version, release date and size, "Release notes", and **Download and install**
//!   (`winpoint.install {version}`; on a metered connection the host answers `metered` and the card asks first, then
//!   sends `metered:true`). A release the updater cannot install keeps phase 1's card: the host's reason (`noDigest`,
//!   no checksum from GitHub; `noAsset`, `dev`, `noPython`, or a verdict of the release's own installer:
//!   `incompatible` with its detail in an expander, `manual`, `missingParts`), and where a download for this PC exists
//!   "Download on GitHub" with the line on how to install it by hand;
//! * the job (D-G74): downloading with its progress (`winpoint.progress` patches the card in place) and Cancel
//!   (`winpoint.cancel {jobId}`), checking the download (verifying, unpacking, preparing) with Cancel, installing (no
//!   Cancel: don't sign out), installed with **Restart WinPoint** (`shell.restartAll {reason:'update'}`) or **Sign
//!   out** (Energie's own question first, then `system.power.action logout`) for a release whose extension changed,
//!   failed with the host's error, what it means for this PC and **Retry** (`winpoint.install` of the job's
//!   version) and the installer's log in an expander, interrupted (Retry finishes it). A reason of the release itself
//!   offers Check, and Download and install of the release offered now, named with its size when it is another one.
//!
//! Every write is forwarded by Settings' host to the owner, which answers within 3 s, else `noOwner`; without the
//! owner the buttons are off and the card says why. The card itself never starts anything but those calls (D-G74:
//! only the owner's job, after a click here, runs the release's installer). While the card waits for a sign-out the
//! page's restart bar is hidden (update.rs, design §6.2): [`WinPointCard::on_relogin`].
//!
//! Host texts (`error.message`, `latest.message`, `job.error.message`, `job.outcomeMessage`) come rendered in the UI
//! language with the status; the card's own are `t(nl, en)`. `winpoint.changed {rev}` reads the status again; while a
//! check runs the card reads it every second (the owner's last change can arrive while it still holds the check's
//! lock). The automatic check's setting, update.winpointAutoCheck, is a toggle in Geavanceerde opties
//! (update_advanced.rs).
use super::Page;
use super::update::when_text;
use super::update_release::{Disc, Tone, err_message, err_value};
use crate::common;
use kit::controls::{self as c, Dialog, DialogResult, Expander, InfoBar, ProgressBar, ProgressRing, Severity, Toast, Variant};
use kit::format::DateFormat;
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

/// settings.css "WinPoint's update card" (the feature-update card's look: update_release.rs `.st-updrel-*`).
pub const CSS: &str = r#"
.st-updwp-details .w11-expander-inner { padding: 4px 16px 12px 58px; }
.st-updwp-detail { color: var(--text-2); font-family: monospace; font-size: var(--fs-caption); }
.st-updwp-bar { margin-top: 8px; }
"#;

/// How long the card says "checking" after its own click while the status does not show the check yet.
const PENDING_MS: u64 = 15_000;
/// How often the card reads the status while a check runs.
const POLL_MS: u64 = 1000;
/// The job states before the installer starts (host services/winpoint/job.rs BEFORE_INSTALL): Cancel still works.
const BEFORE_INSTALL: [&str; 4] = ["downloading", "verifying", "unpacking", "preparing"];
/// Every state of a job the status can show (`state` then comes from `job.state`).
const JOB_STATES: [&str; 8] = ["downloading", "verifying", "unpacking", "preparing", "installing", "installed", "failed", "interrupted"];

// ------------------------------------------------------------------------------------------ the model

/// The disc's colour and motion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Look {
    #[default]
    Plain,
    Ok,
    Busy,
    Warn,
}

#[derive(Debug, PartialEq, Eq)]
struct Line {
    text: String,
    muted: bool,
}

fn plain(text: String) -> Line {
    Line { text, muted: false }
}

fn muted(text: String) -> Line {
    Line { text, muted: true }
}

/// A button of the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    /// "Controleren op updates": winpoint.check
    Check,
    /// "Downloaden en installeren": winpoint.install {version}
    Install,
    /// "Annuleren": winpoint.cancel {jobId}
    Cancel,
    /// "Opnieuw proberen" (Retry, D-G41): winpoint.install of the job's version
    Retry,
    /// "WinPoint opnieuw starten": shell.restartAll {reason:'update'}
    Restart,
    /// "Afmelden": Energie's question, then system.power.action logout
    SignOut,
}

impl Act {
    /// Needs the owner (win11-shell's backend): the winpoint.* writes. A restart and a sign-out do not.
    fn owned(self) -> bool {
        matches!(self, Act::Check | Act::Install | Act::Cancel | Act::Retry)
    }
}

/// What the card shows for one status (pure, so the states are tested on their own).
#[derive(Debug, Default)]
struct Model {
    title: String,
    lines: Vec<Line>,
    /// the release page, for "Release notes" and (with `download`) "Download on GitHub"
    notes: Option<String>,
    download: bool,
    /// the raw detail of a failure (a failed check, the installer's verdict, the job's error and log), for the expander
    detail: Option<String>,
    /// the ring instead of the buttons
    checking: bool,
    /// the buttons in order, each with whether it can be clicked
    buttons: Vec<(Act, bool)>,
    /// a progress bar: Some(None) moving, Some(Some(f)) a fraction
    bar: Option<Option<f64>>,
    look: Look,
    /// the version Download and install or Retry installs
    version: Option<String>,
    /// the job Cancel stops
    job: Option<String>,
    /// the title, the lines and the bar change with every progress event: the card patches them in place
    live: bool,
}

impl Model {
    #[cfg(test)]
    fn has(&self, act: Act) -> bool {
        self.buttons.iter().any(|(a, _)| *a == act)
    }

    /// Everything but what a progress event changes (a live card keeps its widgets while this stays the same).
    fn shape(&self) -> String {
        if !self.live {
            return format!("{self:?}");
        }
        let lines: Vec<bool> = self.lines.iter().map(|l| l.muted).collect();
        format!(
            "live|{lines:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{:?}|{}",
            self.notes,
            self.detail,
            self.buttons,
            self.bar.map(|b| b.is_some()),
            self.look,
            self.version,
            self.job,
            self.download
        )
    }
}

/// A text field that is there and not empty.
fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned)
}

/// A time in ms since the epoch.
fn ms(v: &Value) -> Option<f64> {
    v.as_f64().filter(|v| v.is_finite() && *v > 0.0)
}

/// A byte count (0 when absent).
fn count(v: &Value) -> f64 {
    v.as_f64().filter(|v| v.is_finite() && *v > 0.0).unwrap_or(0.0)
}

/// GitHub's `published_at` ("2026-11-20T10:00:00Z") as "20 nov 2026" / "20 Nov 2026".
fn release_date(v: &Value) -> Option<String> {
    let iso = v.as_str()?;
    let at = glib::DateTime::from_iso8601(iso, None).ok()?;
    Some(kit::format::date(at.to_unix() as f64 * 1000.0, DateFormat::Medium))
}

/// The release page (the host checked it is the repository's own; only a web link is opened).
fn notes_url(latest: &Value) -> Option<String> {
    text(&latest["notesUrl"]).filter(|u| u.starts_with("https://") || u.starts_with("http://"))
}

/// Whether the card offers the release by hand: a download for this PC that the updater does not install itself (no
/// checksum from GitHub, system files to change, or no install from this run: a dry run).
fn by_hand(latest: &Value) -> bool {
    text(&latest["asset"]).is_some()
        && latest["canInstall"] != true
        && matches!(latest["reason"].as_str(), None | Some("noDigest" | "manual"))
}

/// "Downloadgrootte: …" of a release with a download for this PC.
fn size_fact(latest: &Value) -> Option<String> {
    let size = ms(&latest["size"]).filter(|_| text(&latest["asset"]).is_some() && latest["reason"] != "noAsset")?;
    Some(common::fill(t("Downloadgrootte: {size}", "Download size: {size}"), &[("size", &kit::format::bytes(size))]))
}

fn cant_check() -> String {
    t("WinPoint kan niet controleren op updates", "WinPoint can’t check for updates").to_owned()
}

fn auto_off_line() -> Line {
    muted(t("Automatisch controleren staat uit", "Automatic checks are off").to_owned())
}

/// "Pak … uit en voer … uit": installing a release by hand (phase 1's line).
fn howto(latest: &Value) -> Line {
    let file = text(&latest["asset"]).unwrap_or_default();
    let version = common::s(&latest["version"]);
    muted(common::fill(
        t(
            "Pak {file} uit en voer in de map winpoint-{version} ./scripts/install.sh uit. Je instellingen blijven bewaard.",
            "Unpack {file} and run ./scripts/install.sh in its winpoint-{version} folder. Your settings are kept.",
        ),
        &[("file", &file), ("version", version)],
    ))
}

/// A detail too long to show whole (more than about ten lines): it scrolls in the expander.
fn long_detail(detail: &str) -> bool {
    detail.lines().count() > 10 || detail.len() > 1200
}

/// The job's error and the installer's log, for the expander.
fn job_detail(job: &Value) -> Option<String> {
    let parts: Vec<String> = [text(&job["error"]["detail"]), text(&job["logTail"])].into_iter().flatten().collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

/// The card for a `winpoint.status` answer; `pending`: this page asked for a check and the status does not show
/// it yet.
fn model(s: &Value, pending: bool) -> Model {
    let current = text(&s["current"]).or_else(|| text(&s["version"])).unwrap_or_default();
    let state = match common::s(&s["state"]) {
        "unsupported" => "unsupported",
        other if JOB_STATES.contains(&other) => other,
        _ if pending => "checking",
        other => other,
    };
    // the host sets latest only while it is newer than `current`; it stays under a failed state
    let latest = Some(&s["latest"]).filter(|l| l.is_object() && text(&l["version"]).is_some());
    let job = Some(&s["job"]).filter(|j| j.is_object()).unwrap_or(&Value::Null);
    let job_version = text(&job["version"]).or_else(|| latest.and_then(|l| text(&l["version"]))).unwrap_or_default();
    let v = |template: &str| common::fill(template, &[("version", &job_version)]);
    let auto_off = s["autoCheck"] == false;
    // a dry run has no owner, and its check answers with the test-mode toast
    let owner = s["ownerRunning"] != false || s["mode"] == "dry";
    let mut m = Model::default();
    match state {
        "unsupported" => {
            m.title = cant_check();
            m.lines.extend(err_message(&s["error"]).map(plain));
            m.look = Look::Warn;
        }
        "checking" => {
            m.title = t("Zoeken naar een nieuwe versie van WinPoint…", "Checking for a new version of WinPoint…").to_owned();
            m.checking = true;
            m.look = Look::Busy;
        }
        "downloading" => {
            let (done, total) = (count(&job["done"]), count(&job["total"]));
            let fraction = if total > 0.0 { (done / total).clamp(0.0, 1.0) } else { 0.0 };
            let pct = ((fraction * 100.0).floor() as i64).to_string();
            m.title = common::fill(
                t("WinPoint {version} downloaden: {pct}%", "Downloading WinPoint {version}: {pct}%"),
                &[("version", &job_version), ("pct", &pct)],
            );
            m.lines.push(plain(common::fill(
                t("{done} van {total}", "{done} of {total}"),
                &[("done", &kit::format::bytes(done)), ("total", &kit::format::bytes(total))],
            )));
            m.bar = Some(Some(fraction));
            m.buttons.push((Act::Cancel, job["canCancel"] != false));
            m.look = Look::Busy;
            m.live = true;
        }
        "verifying" | "unpacking" | "preparing" => {
            m.title = t("De download controleren…", "Checking the download…").to_owned();
            m.lines.push(plain(format!("WinPoint {job_version}")));
            m.bar = Some(None);
            m.buttons.push((Act::Cancel, job["canCancel"] != false));
            m.look = Look::Busy;
        }
        "installing" => {
            m.title = v(t("WinPoint {version} installeren…", "Installing WinPoint {version}…"));
            m.lines.push(plain(t("Meld je niet af en zet je pc niet uit.", "Don’t sign out or turn off your PC.").to_owned()));
            m.bar = Some(None);
            m.look = Look::Busy;
        }
        "installed" => {
            m.title = v(t("WinPoint {version} is geïnstalleerd", "WinPoint {version} is installed"));
            if job["restart"] == "relogin" {
                m.lines.push(plain(t("Meld je af en weer aan om af te ronden.", "Sign out and back in to finish.").to_owned()));
                m.buttons.push((Act::SignOut, true));
            } else {
                m.lines.push(plain(t("Start WinPoint opnieuw om af te ronden.", "Restart WinPoint to finish.").to_owned()));
                m.buttons.push((Act::Restart, true));
            }
            m.look = Look::Ok;
        }
        "failed" | "interrupted" => {
            m.title = if state == "failed" {
                v(t("WinPoint {version} is niet geïnstalleerd", "WinPoint {version} wasn’t installed"))
            } else {
                v(t("De installatie van WinPoint {version} is onderbroken", "Installing WinPoint {version} was interrupted"))
            };
            m.lines.extend(err_message(&job["error"]).map(plain));
            if state == "interrupted" {
                m.lines.push(plain(t("Probeer het opnieuw om de installatie af te ronden.", "Try again to finish the installation.").to_owned()));
            } else if let Some(outcome) = text(&job["outcomeMessage"]) {
                m.lines.push(muted(outcome));
            }
            // the job's release when the status still offers it: its page, and phase 1's way by hand when the
            // updater does not install it ("This release has no checksum from GitHub; download it from the link"
            // needs that link)
            if let Some(l) = latest.filter(|l| text(&l["version"]) == text(&job["version"])) {
                m.notes = notes_url(l);
                if by_hand(l) {
                    m.lines.push(howto(l));
                    m.download = true;
                }
            }
            m.detail = job_detail(job);
            if job["canRetry"] == true {
                m.buttons.push((Act::Retry, true));
                m.version = Some(job_version.clone());
            } else {
                // the reason is the release's own (no longer offered, no checksum, this PC): check again instead,
                // or install the release offered now when that one can be, named here when it is another one (the
                // title names the job's)
                m.buttons.push((Act::Check, true));
                if let Some(l) = latest.filter(|l| l["canInstall"] == true) {
                    let version = common::s(&l["version"]);
                    if text(&l["version"]) != text(&job["version"]) {
                        let mut facts = vec![common::fill(t("WinPoint {version} is beschikbaar", "WinPoint {version} is available"), &[("version", version)])];
                        facts.extend(size_fact(l));
                        m.lines.push(plain(facts.join(" · ")));
                    }
                    m.buttons.push((Act::Install, true));
                    m.version = Some(version.to_owned());
                }
            }
            m.look = Look::Warn;
        }
        "offline" | "rateLimited" | "unreachable" | "checkFailed" => {
            m.title = cant_check();
            m.lines.extend(err_message(&s["error"]).map(plain));
            m.detail = text(&s["error"]["detail"]);
            // the last known result stays below
            if let Some(l) = latest {
                let version = common::s(&l["version"]);
                m.lines.push(muted(common::fill(t("WinPoint {version} is beschikbaar", "WinPoint {version} is available"), &[("version", version)])));
                m.notes = notes_url(l);
                // the download on GitHub only for a release the updater does not install itself (phase 1's way)
                if by_hand(l) {
                    m.lines.push(howto(l));
                    m.download = true;
                }
            } else if let Some(at) = ms(&s["okAt"]) {
                m.lines.push(muted(common::fill(t("Laatste geslaagde controle: {when}", "Last successful check: {when}"), &[("when", &when_text(at))])));
            }
            m.buttons.push((Act::Check, true));
            m.look = Look::Warn;
        }
        "available" if latest.is_some() => {
            let l = latest.unwrap_or(&Value::Null);
            let version = common::s(&l["version"]);
            m.title = common::fill(t("WinPoint {version} is beschikbaar", "WinPoint {version} is available"), &[("version", version)]);
            let mut facts = vec![common::fill(t("Je hebt versie {version}", "You have version {version}"), &[("version", &current)])];
            if let Some(date) = release_date(&l["publishedAt"]) {
                facts.push(common::fill(t("Uitgebracht op {date}", "Released on {date}"), &[("date", &date)]));
            }
            facts.extend(size_fact(l));
            m.lines.push(plain(facts.join(" · ")));
            m.notes = notes_url(l);
            if l["canInstall"] == true {
                // checking again stays possible (a newer release, or one that changed)
                m.buttons.push((Act::Check, true));
                m.buttons.push((Act::Install, true));
                m.version = Some(version.to_owned());
            } else {
                // why the updater does not install it ("This release has no checksum from GitHub; …"), rendered by
                // the host, and phase 1's way by hand where a download for this PC exists
                m.lines.extend(text(&l["message"]).map(plain));
                if by_hand(l) {
                    m.lines.push(howto(l));
                    m.download = true;
                }
                if l["reason"] == "incompatible" {
                    m.detail = text(&l["detail"]);
                }
                m.buttons.push((Act::Check, true));
            }
        }
        "upToDate" => {
            m.title = t("WinPoint is bijgewerkt", "WinPoint is up to date").to_owned();
            let mut facts = vec![common::fill(t("Versie {version}", "Version {version}"), &[("version", &current)])];
            if let Some(at) = ms(&s["checkedAt"]) {
                facts.push(common::fill(t("Laatst gecontroleerd: {when}", "Last checked: {when}"), &[("when", &when_text(at))]));
            }
            m.lines.push(plain(facts.join(" · ")));
            if auto_off {
                m.lines.push(auto_off_line());
            }
            m.buttons.push((Act::Check, true));
            m.look = Look::Ok;
        }
        _ => {
            m.title = format!("WinPoint {current}").trim().to_owned();
            m.lines.push(plain(t("Nog niet gecontroleerd op nieuwe versies", "Not checked for new versions yet").to_owned()));
            if auto_off {
                m.lines.push(auto_off_line());
            }
            m.buttons.push((Act::Check, true));
        }
    }
    if JOB_STATES.contains(&state) {
        m.job = text(&job["jobId"]);
    }
    if !owner && m.buttons.iter().any(|(a, _)| a.owned()) {
        for (act, enabled) in m.buttons.iter_mut() {
            if act.owned() {
                *enabled = false;
            }
        }
        let update = m.buttons.iter().any(|(a, _)| matches!(a, Act::Install | Act::Cancel | Act::Retry));
        m.lines.push(muted(
            if update {
                t("WinPoint kan alleen worden bijgewerkt terwijl de taakbalk draait", "WinPoint can only be updated while its taskbar is running")
            } else {
                t("WinPoint kan alleen op updates controleren terwijl de taakbalk draait", "WinPoint can only check for updates while its taskbar is running")
            }
            .to_owned(),
        ));
    }
    m
}

/// Whether the status waits for a sign-out that finishes WinPoint's update (the restart bar is then hidden).
fn relogin_pending(s: &Value) -> bool {
    s["state"] == "installed" && s["job"]["restart"] == "relogin"
}

// ------------------------------------------------------------------------------------------ the card

/// The widgets a progress event changes.
struct Live {
    title: gtk::Label,
    lines: Vec<gtk::Label>,
    bar: Option<ProgressBar>,
}

struct Inner {
    p: Page,
    /// the last winpoint.status (None: not read yet, or an older host without winpoint.*)
    snap: RefCell<Option<Value>>,
    /// this page's check: the status rev at the click, until a status shows the check
    pending: Cell<Option<u64>>,
    /// a click's call (check, install, cancel, restart) is on its way
    busy: Cell<bool>,
    /// shell.restartAll answered (not a dry run): every WinPoint process restarts, so its button stays off
    restart_sent: Cell<bool>,
    /// the error of this page's last call (an info bar under the card)
    failure: RefCell<Option<Value>>,
    loading: Cell<bool>,
    again: Cell<bool>,
    polling: Cell<bool>,
    /// what was painted last (a repaint of the same model would restart the disc's turn)
    painted: RefCell<String>,
    live: RefCell<Option<Live>>,
    /// the card waits for a sign-out, and who wants to know (update.rs: the restart bar)
    relogin: Cell<bool>,
    on_relogin: RefCell<Option<Box<dyn Fn(bool)>>>,
    el: gtk::Box,
}

/// WinPoint's update card at the top of Update en herstel.
#[derive(Clone)]
pub struct WinPointCard(Rc<Inner>);

impl WinPointCard {
    pub fn new(p: &Page) -> WinPointCard {
        let el = gtk::Box::new(gtk::Orientation::Vertical, 4);
        el.add_css_class("st-updwp");
        el.set_widget_name("update-winpoint");
        el.set_visible(false);
        WinPointCard(Rc::new(Inner {
            p: p.clone(),
            snap: RefCell::new(None),
            pending: Cell::new(None),
            busy: Cell::new(false),
            restart_sent: Cell::new(false),
            failure: RefCell::new(None),
            loading: Cell::new(false),
            again: Cell::new(false),
            polling: Cell::new(false),
            painted: RefCell::new(String::new()),
            live: RefCell::new(None),
            relogin: Cell::new(false),
            on_relogin: RefCell::new(None),
            el,
        }))
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.0.el
    }

    fn weak(&self) -> Weak<Inner> {
        Rc::downgrade(&self.0)
    }

    /// `f(pending)` whenever the card starts or stops waiting for a sign-out that finishes WinPoint's update.
    pub fn on_relogin(&self, f: impl Fn(bool) + 'static) {
        *self.0.on_relogin.borrow_mut() = Some(Box::new(f));
    }

    /// The first winpoint.status, started now; the page awaits it before it shows.
    pub fn load(&self) -> impl std::future::Future<Output = ()> + 'static {
        let fut = self.0.p.get("winpoint.status", json!({}));
        let weak = self.weak();
        async move {
            let v = fut.await;
            if let Some(inner) = weak.upgrade()
                && inner.p.alive()
            {
                WinPointCard(inner).apply(v);
            }
        }
    }

    /// winpoint.changed and the auto-check setting read the status again; winpoint.progress patches the running
    /// download in place. The page holds the card while it is shown (its handlers only hold it weakly).
    pub fn listen(&self) {
        let card = self.clone();
        self.0.p.on_cleanup(move || drop(card));
        let weak = self.weak();
        self.0.p.on("winpoint.changed", move |d| {
            let Some(inner) = weak.upgrade() else { return };
            let stale = inner.snap.borrow().as_ref().is_none_or(|s| d["rev"] != s["rev"]);
            if stale {
                WinPointCard(inner).reload();
            }
        });
        let weak = self.weak();
        self.0.p.on("winpoint.progress", move |d| {
            let Some(inner) = weak.upgrade() else { return };
            let patched = inner.snap.borrow_mut().as_mut().is_some_and(|s| progress(s, d));
            if patched {
                WinPointCard(inner).paint();
            } else {
                WinPointCard(inner).reload();
            }
        });
        let weak = self.weak();
        self.0.p.on_settings(move |_, path| {
            if path.is_none_or(|p| p.starts_with("update"))
                && let Some(inner) = weak.upgrade()
            {
                WinPointCard(inner).reload();
            }
        });
    }

    /// Read winpoint.status and repaint; a call while one runs reads once more after it.
    fn reload(&self) {
        let me = &self.0;
        if me.loading.replace(true) {
            me.again.set(true);
            return;
        }
        let card = self.clone();
        me.p.spawn(async move {
            loop {
                card.0.again.set(false);
                let v = card.0.p.get("winpoint.status", json!({})).await;
                if !card.0.p.alive() {
                    break;
                }
                card.apply(v);
                if !card.0.again.get() {
                    break;
                }
            }
            card.0.loading.set(false);
        });
    }

    fn apply(&self, v: Option<Value>) {
        let me = &self.0;
        // a failed read keeps the last status (none at all: no card, as on an older host)
        if let Some(v) = v.filter(Value::is_object) {
            if let Some(rev) = me.pending.get()
                && (v["rev"].as_u64().unwrap_or(0) > rev || v["state"] == "checking")
            {
                me.pending.set(None);
            }
            *me.snap.borrow_mut() = Some(v);
        }
        self.paint();
        let relogin = me.snap.borrow().as_ref().is_some_and(relogin_pending);
        if me.relogin.replace(relogin) != relogin
            && let Some(f) = me.on_relogin.borrow().as_ref()
        {
            f(relogin);
        }
        // while a check runs: read again in a second
        let checking = me.pending.get().is_some() || me.snap.borrow().as_ref().is_some_and(|s| s["state"] == "checking");
        if checking && !me.polling.replace(true) {
            let weak = self.weak();
            me.p.timeout(POLL_MS, move || {
                if let Some(inner) = weak.upgrade() {
                    inner.polling.set(false);
                    WinPointCard(inner).reload();
                }
            });
        }
    }

    /// A click's call: the buttons stay off until it is answered, an error goes into the info bar, then the status is
    /// read again. `done` gets the answer (and the error) first.
    fn act(&self, method: &'static str, args: Value, done: impl FnOnce(&WinPointCard, &Result<Value, kit::CallError>) + 'static) {
        let me = &self.0;
        if me.busy.replace(true) {
            return;
        }
        *me.failure.borrow_mut() = None;
        self.paint();
        let fut = me.p.call(method, args);
        let card = self.clone();
        me.p.spawn(async move {
            let r = fut.await;
            if !card.0.p.alive() {
                return;
            }
            card.0.busy.set(false);
            done(&card, &r);
            match &r {
                Ok(v) => common::note_dry_run(&card.0.p.anchor(), v),
                Err(e) if e.code == "metered" => {}
                Err(e) => *card.0.failure.borrow_mut() = Some(err_value(e)),
            }
            card.paint();
            card.reload();
        });
    }

    /// "Controleren op updates": winpoint.check (forwarded to the owner by Settings' host).
    fn check(&self) {
        let me = &self.0;
        if me.busy.get() {
            return;
        }
        let rev = me.snap.borrow().as_ref().and_then(|s| s["rev"].as_u64()).unwrap_or(0);
        me.pending.set(Some(rev));
        self.act("winpoint.check", json!({}), |card, r| {
            let me = &card.0;
            match r {
                Ok(v) if v["started"] == true => {
                    // the owner's winpoint.changed follows; stop waiting for it after a while
                    let weak = card.weak();
                    me.p.timeout(PENDING_MS, move || {
                        if let Some(inner) = weak.upgrade()
                            && inner.pending.take().is_some()
                        {
                            WinPointCard(inner).reload();
                        }
                    });
                }
                Ok(v) => {
                    me.pending.set(None);
                    if v["skipped"] == "recent" {
                        let _ = Toast::new(t("WinPoint heeft zojuist al op updates gecontroleerd.", "WinPoint checked for updates a moment ago."))
                            .severity(Severity::Info)
                            .timeout(3500)
                            .show(&me.p.anchor());
                    }
                    // running, offline and rateLimited: the status says it
                }
                Err(_) => me.pending.set(None),
            }
        });
    }

    /// "Downloaden en installeren" and "Opnieuw proberen": winpoint.install {version}; on a metered connection the
    /// host answers `metered` and the card asks first (then `metered: true`).
    fn install(&self, version: String, metered: bool) {
        let args = if metered { json!({ "version": version, "metered": true }) } else { json!({ "version": version }) };
        self.act("winpoint.install", args, move |card, r| {
            if let Err(e) = r
                && e.code == "metered"
            {
                if metered {
                    *card.0.failure.borrow_mut() = Some(err_value(e));
                } else {
                    let size = count(&e.data["size"]);
                    card.confirm_metered(version, size);
                }
            }
        });
    }

    fn confirm_metered(&self, version: String, size: f64) {
        let size = if size > 0.0 { kit::format::bytes(size) } else { "?".to_owned() };
        let body = common::fill(
            t(
                "WinPoint {version} is {size}. Downloaden kan kosten met zich meebrengen.",
                "WinPoint {version} is {size}. Downloading it may cost you data.",
            ),
            &[("version", &version), ("size", &size)],
        );
        let weak = self.weak();
        Dialog::new(t("Downloaden via een verbinding met een datalimiet?", "Download over a metered connection?"))
            .text(&body)
            .primary(t("Toch downloaden", "Download anyway"))
            .close_label(t("Annuleren", "Cancel"))
            .show(&self.0.p.anchor(), move |r| {
                if r == DialogResult::Primary
                    && let Some(inner) = weak.upgrade()
                    && inner.p.alive()
                {
                    WinPointCard(inner).install(version, true);
                }
            });
    }

    fn act_on(&self, act: Act, m: &Model) {
        match act {
            Act::Check => self.check(),
            Act::Install | Act::Retry => {
                if let Some(version) = m.version.clone() {
                    self.install(version, false);
                }
            }
            Act::Cancel => {
                if let Some(id) = m.job.clone() {
                    self.act("winpoint.cancel", json!({ "jobId": id }), |_, _| {});
                }
            }
            Act::Restart => self.act("shell.restartAll", json!({ "reason": "update" }), |card, r| {
                if let Ok(v) = r
                    && v["dryRun"] != true
                {
                    card.0.restart_sent.set(true);
                }
            }),
            Act::SignOut => super::power::sign_out(&self.0.p),
        }
    }

    fn link(&self, label: &str, name: &str, url: &str) -> gtk::Widget {
        let a = c::hyperlink(label, None);
        a.set_widget_name(name);
        let (weak, url) = (self.weak(), url.to_owned());
        a.connect_clicked(move |_| {
            let Some(inner) = weak.upgrade() else { return };
            let fut = inner.p.run("apps.openUri", json!({ "uri": url }));
            inner.p.spawn(async move {
                let _ = fut.await;
            });
        });
        a.upcast()
    }

    fn button(&self, act: Act, enabled: bool, m: &Model) -> gtk::Button {
        let (label, variant, name) = match act {
            Act::Check => (t("Controleren op updates", "Check for updates"), Variant::Standard, "update-winpoint-check"),
            Act::Install => (t("Downloaden en installeren", "Download and install"), Variant::Accent, "update-winpoint-install"),
            Act::Cancel => (t("Annuleren", "Cancel"), Variant::Standard, "update-winpoint-cancel"),
            Act::Retry => (t("Opnieuw proberen", "Retry"), Variant::Accent, "update-winpoint-retry"),
            Act::Restart => (t("WinPoint opnieuw starten", "Restart WinPoint"), Variant::Accent, "update-winpoint-restart"),
            Act::SignOut => (t("Afmelden", "Sign out"), Variant::Accent, "update-winpoint-signout"),
        };
        let b = c::button(label, variant);
        b.set_widget_name(name);
        b.set_sensitive(enabled && !self.0.busy.get() && !(act == Act::Restart && self.0.restart_sent.get()));
        let (weak, version, job) = (self.weak(), m.version.clone(), m.job.clone());
        b.connect_clicked(move |_| {
            if let Some(inner) = weak.upgrade() {
                let m = Model { version: version.clone(), job: job.clone(), ..Model::default() };
                WinPointCard(inner).act_on(act, &m);
            }
        });
        b
    }

    fn paint(&self) {
        let me = &self.0;
        let snap = me.snap.borrow().clone();
        let Some(s) = snap else {
            me.el.set_visible(false);
            return;
        };
        let m = model(&s, me.pending.get().is_some());
        let failure = me.failure.borrow().as_ref().and_then(err_message);
        let key = format!("{}|{}|{}|{failure:?}", m.shape(), me.busy.get(), me.restart_sent.get());
        if *me.painted.borrow() == key {
            // the same card: only a running download's numbers change
            if let Some(live) = me.live.borrow().as_ref() {
                live.title.set_label(&m.title);
                for (line, w) in m.lines.iter().zip(&live.lines) {
                    w.set_label(&line.text);
                }
                if let (Some(Some(f)), Some(bar)) = (m.bar, &live.bar) {
                    bar.set_fraction(f);
                }
            }
            return;
        }
        *me.painted.borrow_mut() = key;
        c::clear(&me.el);
        me.el.set_visible(true);

        let card = gtk::Box::new(gtk::Orientation::Vertical, 0);
        card.add_css_class("st-updrel-card");
        let inner = crate::widgets::Wrap::new(16, 12);
        inner.add_css_class("st-updrel-inner");
        let disc = Disc::new(48, 24, "update");
        disc.set_tone(match m.look {
            Look::Ok => Tone::Ok,
            Look::Warn => Tone::Caution,
            Look::Plain | Look::Busy => Tone::Accent,
        });
        disc.set_busy(m.look == Look::Busy);
        disc.widget().set_valign(gtk::Align::Start);
        inner.append(disc.widget());

        let text = gtk::Box::new(gtk::Orientation::Vertical, 2);
        text.set_hexpand(true);
        text.set_valign(gtk::Align::Center);
        text.set_size_request(260, -1);
        let title = c::label(&m.title, "st-updrel-title", true);
        title.set_widget_name("update-winpoint-title");
        text.append(&title);
        let mut lines = Vec::new();
        for l in &m.lines {
            let w = c::label(&l.text, "st-updrel-line", true);
            w.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            if l.muted {
                w.add_css_class("st-muted");
            }
            text.append(&w);
            lines.push(w);
        }
        let bar = m.bar.map(|fraction| {
            let bar = ProgressBar::new();
            bar.widget().add_css_class("st-updwp-bar");
            bar.widget().set_widget_name("update-winpoint-progress");
            match fraction {
                Some(f) => bar.set_fraction(f),
                None => bar.set_indeterminate(true),
            }
            text.append(bar.widget());
            bar
        });
        if let Some(url) = &m.notes {
            let links = crate::widgets::Wrap::new(12, 0);
            links.add_css_class("st-updrel-links");
            links.append(&self.link(t("Opmerkingen bij de release", "Release notes"), "update-winpoint-notes", url));
            if m.download {
                links.append(&self.link(t("Downloaden op GitHub", "Download on GitHub"), "update-winpoint-download", url));
            }
            text.append(&links);
        }
        inner.append(&text);
        *me.live.borrow_mut() = m.live.then(|| Live { title: title.clone(), lines, bar });

        let ctl = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        ctl.set_halign(gtk::Align::End);
        ctl.set_valign(gtk::Align::Center);
        if m.checking {
            let ring = ProgressRing::new(24, None);
            ring.widget().set_widget_name("update-winpoint-checking");
            ctl.append(ring.widget());
        }
        for (act, enabled) in &m.buttons {
            ctl.append(&self.button(*act, *enabled, &m));
        }
        if ctl.first_child().is_some() {
            inner.append(&ctl);
        }
        card.append(&inner);
        me.el.append(&card);

        if let Some(detail) = &m.detail {
            let exp = Expander::new("info", t("Technische details", "Technical details"), "");
            exp.widget().add_css_class("st-updwp-details");
            exp.header_button().set_widget_name("update-winpoint-details");
            let l = c::label(detail, "st-updwp-detail", true);
            l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
            l.set_selectable(true);
            if long_detail(detail) {
                // the installer's log: a fixed height that scrolls (a wrapped label's natural height is not known
                // to the scrolled window before it has its width)
                let sc = gtk::ScrolledWindow::new();
                sc.set_hscrollbar_policy(gtk::PolicyType::Never);
                sc.set_min_content_height(240);
                sc.set_max_content_height(240);
                sc.set_child(Some(&l));
                exp.set_content(&sc);
            } else {
                exp.set_content(&l);
            }
            me.el.append(exp.widget());
        }
        if let Some(msg) = failure {
            let b = InfoBar::new(Severity::Error, "", &msg);
            b.widget().add_css_class("st-notice");
            b.widget().set_widget_name("update-winpoint-error");
            let weak = self.weak();
            b.connect_closed(move || {
                if let Some(inner) = weak.upgrade() {
                    *inner.failure.borrow_mut() = None;
                    WinPointCard(inner).paint();
                }
            });
            me.el.append(b.widget());
        }
    }
}

/// A `winpoint.progress {jobId, state, done, total}` for the job the status shows: its numbers into the status (true).
fn progress(s: &mut Value, d: &Value) -> bool {
    let state = common::s(&d["state"]);
    if !s["job"].is_object() || s["job"]["jobId"] != d["jobId"] || !BEFORE_INSTALL.contains(&state) {
        return false;
    }
    s["job"]["done"] = d["done"].clone();
    s["job"]["total"] = d["total"].clone();
    s["job"]["state"] = json!(state);
    s["state"] = json!(state);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: &str) -> Value {
        json!({"mode":"installed","ownerRunning":true,"version":"0.9.0","installed":"0.9.0","current":"0.9.0",
               "arch":"x86_64","autoCheck":true,"state":state,"result":null,"checkedAt":null,"okAt":null,
               "nextCheckAt":null,"retryAfter":null,"latest":null,"job":null,"error":null,"rev":3,"dryRun":false})
    }

    fn latest(asset: bool) -> Value {
        json!({"version":"0.9.1","tag":"v0.9.1","name":"WinPoint 0.9.1","publishedAt":"2026-11-20T10:00:00Z",
               "notesUrl":"https://github.com/roboburr/WinPoint/releases/tag/v0.9.1",
               "asset": if asset { json!("winpoint-0.9.1-linux-x86_64.tar.gz") } else { Value::Null },
               "size": if asset { json!(71234567) } else { Value::Null },
               "reason": if asset { Value::Null } else { json!("noAsset") },
               "message": if asset { Value::Null } else { json!("Er is nog geen download voor deze pc (aarch64)") },
               "detail":null})
    }

    /// The release an installed run can install (host compose: no reason, canInstall).
    fn installable() -> Value {
        let mut l = latest(true);
        l["canInstall"] = json!(true);
        l
    }

    fn job(state: &str) -> Value {
        json!({"jobId":"abc123","version":"0.9.1","state":state,"done":0,"total":71234567,"startedAt":1,
               "finishedAt":null,"restart":null,"outcome":null,"error":null,"outcomeMessage":null,"logTail":null,
               "canCancel":BEFORE_INSTALL.contains(&state),"canRetry":false})
    }

    fn with_job(state: &str) -> Value {
        let mut s = status(state);
        s["latest"] = installable();
        s["job"] = job(state);
        s
    }

    fn texts(m: &Model) -> Vec<&str> {
        m.lines.iter().map(|l| l.text.as_str()).collect()
    }

    fn acts(m: &Model) -> Vec<Act> {
        m.buttons.iter().map(|(a, _)| *a).collect()
    }

    /// Never checked, then up to date (with and without the automatic check), checking and this page's own click.
    #[test]
    fn the_quiet_states() {
        let m = model(&status("unknown"), false);
        assert_eq!(m.title, "WinPoint 0.9.0");
        assert_eq!(texts(&m), ["Nog niet gecontroleerd op nieuwe versies"]);
        assert_eq!(m.buttons, [(Act::Check, true)]);
        let mut s = status("upToDate");
        s["checkedAt"] = json!(kit::format::now_ms() - 60_000.0);
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint is bijgewerkt");
        assert!(texts(&m)[0].starts_with("Versie 0.9.0 · Laatst gecontroleerd: "), "{:?}", texts(&m));
        assert_eq!(m.lines.len(), 1);
        assert_eq!(m.look, Look::Ok);
        s["autoCheck"] = json!(false);
        assert_eq!(model(&s, false).lines[1], muted("Automatisch controleren staat uit".into()));
        let m = model(&status("checking"), false);
        assert!(m.checking && m.buttons.is_empty() && m.look == Look::Busy);
        let m = model(&status("upToDate"), true);
        assert!(m.checking, "the click shows checking before the status does");
        assert!(!model(&status("unsupported"), true).checking, "a refused seam never checks");
        assert!(!model(&with_job("installing"), true).checking, "a running job is not a check");
    }

    /// A release this run can install: its facts, Release notes and Download and install (no download link, no
    /// how-to); the version the button sends.
    #[test]
    fn a_release_to_install() {
        let mut s = status("available");
        s["latest"] = installable();
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 is beschikbaar");
        assert!(texts(&m)[0].starts_with("Je hebt versie 0.9.0 · Uitgebracht op 20 nov 2026 · Downloadgrootte: "), "{:?}", texts(&m));
        assert_eq!(m.lines.len(), 1, "no how-to line: WinPoint installs it itself");
        assert_eq!(m.notes.as_deref(), Some("https://github.com/roboburr/WinPoint/releases/tag/v0.9.1"));
        assert!(!m.download);
        assert_eq!(m.buttons, [(Act::Check, true), (Act::Install, true)]);
        assert_eq!(m.version.as_deref(), Some("0.9.1"));
        s["ownerRunning"] = json!(false);
        let m = model(&s, false);
        assert_eq!(m.buttons, [(Act::Check, false), (Act::Install, false)]);
        assert_eq!(texts(&m).last(), Some(&"WinPoint kan alleen worden bijgewerkt terwijl de taakbalk draait"));
    }

    /// A newer release the updater does not install (phase 1's card): the facts, the how-to and both links on the
    /// release page; without a download for this PC the host's reason instead, and no download link.
    #[test]
    fn a_newer_release_by_hand() {
        let mut s = status("available");
        s["latest"] = latest(true);
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 is beschikbaar");
        assert!(texts(&m)[0].starts_with("Je hebt versie 0.9.0 · Uitgebracht op 20 nov 2026 · Downloadgrootte: "), "{:?}", texts(&m));
        assert_eq!(
            m.lines[1],
            muted("Pak winpoint-0.9.1-linux-x86_64.tar.gz uit en voer in de map winpoint-0.9.1 ./scripts/install.sh uit. Je instellingen blijven bewaard.".into())
        );
        assert_eq!(m.notes.as_deref(), Some("https://github.com/roboburr/WinPoint/releases/tag/v0.9.1"));
        assert!(m.download && m.buttons == [(Act::Check, true)]);
        s["latest"] = latest(false);
        let m = model(&s, false);
        assert_eq!(texts(&m), ["Je hebt versie 0.9.0 · Uitgebracht op 20 nov 2026", "Er is nog geen download voor deze pc (aarch64)"]);
        assert!(m.notes.is_some() && !m.download);
        s["latest"]["notesUrl"] = json!("file:///etc/passwd");
        assert!(model(&s, false).notes.is_none(), "only a web page is opened");
    }

    /// No digest from GitHub: the host's reason, then phase 1's links and how-to, no install button. A verdict of
    /// the release's installer: its reason, incompatible with the detail in the expander, no how-to.
    #[test]
    fn the_reasons() {
        let mut s = status("available");
        s["latest"] = latest(true);
        s["latest"]["reason"] = json!("noDigest");
        s["latest"]["message"] = json!("Deze uitgave heeft geen controlegetal van GitHub; download hem via de link");
        let m = model(&s, false);
        assert_eq!(texts(&m)[1], "Deze uitgave heeft geen controlegetal van GitHub; download hem via de link");
        assert!(texts(&m)[2].starts_with("Pak winpoint-0.9.1-linux-x86_64.tar.gz uit"), "{:?}", texts(&m));
        assert!(m.download && m.notes.is_some());
        assert!(!m.has(Act::Install) && m.has(Act::Check));
        assert!(texts(&m)[0].contains("Downloadgrootte: "), "the size of the download by hand");
        s["latest"]["reason"] = json!("incompatible");
        s["latest"]["message"] = json!("WinPoint 0.9.1 werkt niet met de systeembibliotheken van deze pc");
        s["latest"]["detail"] = json!("win11-shell: version `GLIBC_2.45' not found");
        let m = model(&s, false);
        assert_eq!(texts(&m).len(), 2, "no how-to for a release that does not work here");
        assert!(!m.download && m.notes.is_some());
        assert_eq!(m.detail.as_deref(), Some("win11-shell: version `GLIBC_2.45' not found"));
        s["latest"]["reason"] = json!("manual");
        s["latest"]["detail"] = Value::Null;
        let m = model(&s, false);
        assert!(m.download && m.detail.is_none() && texts(&m)[2].starts_with("Pak "), "manual: by hand, with the how-to");
    }

    /// Downloading: the percentage in the title, done of total, the bar, Cancel with the job's id; a progress event
    /// patches the numbers without changing the card's shape.
    #[test]
    fn downloading_and_its_progress() {
        let mut s = with_job("downloading");
        s["job"]["done"] = json!(35_617_284);
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 downloaden: 50%");
        assert_eq!(texts(&m), ["34 MB van 67,9 MB"]);
        assert!(matches!(m.bar, Some(Some(f)) if (f - 0.5).abs() < 0.01));
        assert_eq!(m.buttons, [(Act::Cancel, true)]);
        assert_eq!(m.job.as_deref(), Some("abc123"));
        assert!(m.live && m.look == Look::Busy);
        let shape = m.shape();
        assert!(progress(&mut s, &json!({"jobId":"abc123","state":"downloading","done":71_234_567,"total":71_234_567})));
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 downloaden: 100%");
        assert_eq!(m.shape(), shape, "the same widgets, new numbers");
        assert!(!progress(&mut s, &json!({"jobId":"other","state":"downloading","done":1,"total":2})), "another job");
        assert!(!progress(&mut s, &json!({"jobId":"abc123","state":"installing","done":1,"total":2})), "no numbers once it installs");
        assert!(progress(&mut s, &json!({"jobId":"abc123","state":"verifying","done":1,"total":2})));
        let m = model(&s, false);
        assert_eq!(m.title, "De download controleren…");
        assert_ne!(m.shape(), shape, "another state is another card");
    }

    /// Checking the download, installing (no Cancel), installed with Restart WinPoint or Sign out.
    #[test]
    fn the_install_states() {
        for state in ["verifying", "unpacking", "preparing"] {
            let m = model(&with_job(state), false);
            assert_eq!(m.title, "De download controleren…", "{state}");
            assert_eq!((m.bar, m.buttons.clone()), (Some(None), vec![(Act::Cancel, true)]), "{state}");
        }
        let m = model(&with_job("installing"), false);
        assert_eq!(m.title, "WinPoint 0.9.1 installeren…");
        assert_eq!(texts(&m), ["Meld je niet af en zet je pc niet uit."]);
        assert!(m.buttons.is_empty() && m.bar == Some(None));
        let mut s = with_job("installed");
        s["job"]["restart"] = json!("restart");
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 is geïnstalleerd");
        assert_eq!(texts(&m), ["Start WinPoint opnieuw om af te ronden."]);
        assert_eq!((m.buttons.clone(), m.look), (vec![(Act::Restart, true)], Look::Ok));
        assert!(!relogin_pending(&s));
        s["job"]["restart"] = json!("relogin");
        let m = model(&s, false);
        assert_eq!(texts(&m), ["Meld je af en weer aan om af te ronden."]);
        assert_eq!(m.buttons, [(Act::SignOut, true)]);
        assert!(relogin_pending(&s), "the restart bar waits for the sign-out");
        s["ownerRunning"] = json!(false);
        assert_eq!(model(&s, false).buttons, [(Act::SignOut, true)], "a sign-out needs no owner");
        let mut s = with_job("downloading");
        s["ownerRunning"] = json!(false);
        let m = model(&s, false);
        assert_eq!(m.buttons, [(Act::Cancel, false)]);
        assert_eq!(texts(&m).last(), Some(&"WinPoint kan alleen worden bijgewerkt terwijl de taakbalk draait"));
    }

    /// Failed: the host's error, the outcome line, Try again with the job's version, the error's detail and the log
    /// in the expander; a reason of the release itself checks again instead. Interrupted: Try again finishes it.
    #[test]
    fn failed_and_interrupted() {
        let mut s = with_job("failed");
        s["job"]["error"] = json!({"code":"checksum","key":"update.winpoint.err.checksum","params":{},
                                   "message":"De download is beschadigd en is verwijderd","detail":"winpoint-0.9.1-linux-x86_64.tar.gz: 71234567 bytes, sha256 00"});
        s["job"]["outcome"] = json!("nothingChanged");
        s["job"]["outcomeMessage"] = json!("Er is niets gewijzigd.");
        s["job"]["logTail"] = json!("install.py: done\n");
        s["job"]["canRetry"] = json!(true);
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 is niet geïnstalleerd");
        assert_eq!(m.lines, [plain("De download is beschadigd en is verwijderd".into()), muted("Er is niets gewijzigd.".into())]);
        assert_eq!(m.buttons, [(Act::Retry, true)]);
        assert_eq!(m.version.as_deref(), Some("0.9.1"));
        assert_eq!(m.detail.as_deref(), Some("winpoint-0.9.1-linux-x86_64.tar.gz: 71234567 bytes, sha256 00\n\ninstall.py: done"));
        assert_eq!(m.look, Look::Warn);
        s["job"]["canRetry"] = json!(false);
        assert_eq!(acts(&model(&s, false)), [Act::Check, Act::Install], "no Try again that would end the same way");
        // no checksum from GitHub at the click: the reason, the release page, Download on GitHub and the how-to line
        // (the text points at that link), and a check, no install
        s["job"]["error"] = json!({"code":"noDigest","key":"update.winpoint.reason.noDigest","params":{},
                                   "message":"Deze uitgave heeft geen controlegetal van GitHub; download hem via de link","detail":""});
        s["latest"] = latest(true);
        s["latest"]["reason"] = json!("noDigest");
        let m = model(&s, false);
        assert_eq!(texts(&m)[0], "Deze uitgave heeft geen controlegetal van GitHub; download hem via de link");
        assert!(texts(&m)[2].starts_with("Pak winpoint-0.9.1-linux-x86_64.tar.gz uit"), "{:?}", texts(&m));
        assert_eq!(m.notes.as_deref(), Some("https://github.com/roboburr/WinPoint/releases/tag/v0.9.1"));
        assert!(m.download);
        assert_eq!(m.buttons, [(Act::Check, true)]);
        // a stale job while another release can be installed: that one is named with its size and offered right
        // here; the release page under the job's title is only ever the job's release's
        s["job"]["error"] = json!({"code":"stale","key":"update.winpoint.err.stale","params":{},
                                   "message":"Deze versie wordt niet meer aangeboden; controleer opnieuw","detail":""});
        s["latest"] = installable();
        s["latest"]["version"] = json!("0.9.2");
        s["latest"]["notesUrl"] = json!("https://github.com/roboburr/WinPoint/releases/tag/v0.9.2");
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint 0.9.1 is niet geïnstalleerd");
        assert_eq!(
            texts(&m),
            ["Deze versie wordt niet meer aangeboden; controleer opnieuw", "Er is niets gewijzigd.", "WinPoint 0.9.2 is beschikbaar · Downloadgrootte: 67,9 MB"]
        );
        assert_eq!(m.notes, None, "not 0.9.2's notes under 0.9.1's title");
        assert!(!m.download, "WinPoint installs it itself");
        assert_eq!(m.buttons, [(Act::Check, true), (Act::Install, true)]);
        assert_eq!(m.version.as_deref(), Some("0.9.2"));
        // one that is offered again as it was (the job's own): no line, its page, and Install
        s["latest"] = installable();
        let m = model(&s, false);
        assert_eq!(texts(&m), ["Deze versie wordt niet meer aangeboden; controleer opnieuw", "Er is niets gewijzigd."]);
        assert_eq!(m.notes.as_deref(), Some("https://github.com/roboburr/WinPoint/releases/tag/v0.9.1"));
        assert_eq!((acts(&m), m.version.as_deref()), (vec![Act::Check, Act::Install], Some("0.9.1")));
        let mut s = with_job("interrupted");
        s["job"]["canRetry"] = json!(true);
        let m = model(&s, false);
        assert_eq!(m.title, "De installatie van WinPoint 0.9.1 is onderbroken");
        assert_eq!(texts(&m), ["Probeer het opnieuw om de installatie af te ronden."]);
        assert_eq!(m.buttons, [(Act::Retry, true)]);
        assert!(m.detail.is_none(), "no empty expander");
        // a short detail is shown whole, the installer's log (up to 4 KiB) scrolls
        assert!(!long_detail("winpoint-0.9.1-linux-x86_64.tar.gz: 71234567 bytes, sha256 00"));
        assert!(long_detail(&"install.py: a line of the log\n".repeat(40)));
    }

    /// A failed check: the host's reason, the raw detail for the expander, and the last known result below it.
    #[test]
    fn a_failed_check() {
        let mut s = status("checkFailed");
        s["error"] = json!({"code":"status","key":"update.winpoint.err.status","params":{"code":500},
                            "message":"GitHub gaf een fout (500)","detail":"HTTP 500 from http://127.0.0.1:47811/"});
        s["okAt"] = json!(kit::format::now_ms() - 3_600_000.0);
        let m = model(&s, false);
        assert_eq!(m.title, "WinPoint kan niet controleren op updates");
        assert_eq!(texts(&m)[0], "GitHub gaf een fout (500)");
        assert!(texts(&m)[1].starts_with("Laatste geslaagde controle: "));
        assert_eq!(m.detail.as_deref(), Some("HTTP 500 from http://127.0.0.1:47811/"));
        assert_eq!((m.buttons.clone(), m.look), (vec![(Act::Check, true)], Look::Warn));
        s["latest"] = latest(true);
        let m = model(&s, false);
        assert_eq!(m.lines[1], muted("WinPoint 0.9.1 is beschikbaar".into()));
        assert!(m.notes.is_some() && m.download && texts(&m)[2].starts_with("Pak "), "{:?}", texts(&m));
        // a release the updater installs itself has no download link here; one without a checksum (or with system
        // files) keeps phase 1's link and how-to
        s["latest"] = installable();
        let m = model(&s, false);
        assert!(m.notes.is_some() && !m.download && m.lines.len() == 2, "{:?}", texts(&m));
        s["latest"] = latest(true);
        s["latest"]["reason"] = json!("noDigest");
        let m = model(&s, false);
        assert!(m.download && texts(&m)[2].starts_with("Pak "), "{:?}", texts(&m));
        s["latest"]["reason"] = json!("noAsset");
        assert!(!model(&s, false).download, "no download for this PC");
        let mut s = status("offline");
        s["error"] = json!({"code":"offline","key":"update.winpoint.err.offline","params":{},"message":"Geen internetverbinding","detail":""});
        let m = model(&s, false);
        assert_eq!(texts(&m), ["Geen internetverbinding"]);
        assert!(m.detail.is_none(), "no empty expander");
    }

    /// Without the owner (win11-shell's backend) the button is off and says why; a dry run keeps it, and so does a status
    /// that cannot tell (`ownerRunning` null).
    #[test]
    fn no_owner() {
        let mut s = status("unknown");
        s["ownerRunning"] = json!(false);
        let m = model(&s, false);
        assert_eq!(m.buttons, [(Act::Check, false)]);
        assert_eq!(m.lines.last().map(|l| l.text.as_str()), Some("WinPoint kan alleen op updates controleren terwijl de taakbalk draait"));
        s["mode"] = json!("dry");
        assert_eq!(model(&s, false).buttons, [(Act::Check, true)]);
        // a state folder the host cannot use: whether the owner runs is not known (null), so the button stays (a
        // click tries the folder again) and the card says what is wrong instead of blaming the taskbar
        let mut s = status("checkFailed");
        s["ownerRunning"] = Value::Null;
        s["error"] = json!({"code":"cache","key":"update.winpoint.err.cache","params":{"path":"/c/winpoint"},
                            "message":"WinPoint kan zijn map /c/winpoint niet gebruiken.","detail":"/c/winpoint: Permission denied (os error 13)"});
        let m = model(&s, false);
        assert_eq!((m.buttons.clone(), texts(&m)), (vec![(Act::Check, true)], vec!["WinPoint kan zijn map /c/winpoint niet gebruiken."]));
        assert_eq!(m.detail.as_deref(), Some("/c/winpoint: Permission denied (os error 13)"));
        let mut s = status("unsupported");
        s["error"] = json!({"message":"De testopstelling klopt niet"});
        let m = model(&s, false);
        assert_eq!((m.buttons.clone(), texts(&m)), (vec![], vec!["De testopstelling klopt niet"]));
    }
}
