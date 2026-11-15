//! The shared software review and job tracking of 6ebf169f:vendor/ui/js/core/software.js (and css/software.css): its
//! texts, the details list, the review dialog (software.prepare, then software.execute only from the reviewed plan's
//! own button), and the job view that follows software.status. Preparing never authorises execution.
use kit::controls::{self as c, Dialog, DialogResult, InfoBar, ProgressState, Severity, Toast, Variant};
use kit::prelude::*;
use kit::t;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

/// softwareT by key, in the UI language.
fn text(key: &str) -> Option<&'static str> {
    Some(match key {
        "install" => t("Installeren", "Install"),
        "uninstall" => t("Verwijderen", "Uninstall"),
        "removeShortcut" => t("Snelkoppeling verwijderen", "Remove shortcut"),
        "installName" => t("‘{name}’ installeren", "Install “{name}”"),
        "uninstallName" => t("‘{name}’ verwijderen", "Uninstall “{name}”"),
        "removeShortcutName" => t("Snelkoppeling ‘{name}’ verwijderen", "Remove shortcut “{name}”"),
        "review" => t("Installatie controleren", "Review installation"),
        "reviewRemove" => t("Verwijderen controleren", "Review removal"),
        "provider" => t("Pakketbeheerder", "Package manager"),
        "scope" => t("Installatie voor", "Install for"),
        "user" => t("Alleen mijzelf", "Just me"),
        "system" => t("Alle gebruikers", "All users"),
        "version" => t("Versie", "Version"),
        "architecture" => t("Architectuur", "Architecture"),
        "size" => t("Grootte", "Size"),
        "source" => t("Bestand", "File"),
        "action" => t("Actie", "Action"),
        "auth" => t("Er kan om je wachtwoord worden gevraagd.", "You may be asked for your password."),
        "cancel" => t("Annuleren", "Cancel"),
        "close" => t("Sluiten", "Close"),
        "checking" => t("Beschikbaarheid controleren…", "Checking availability…"),
        "unavailable" => t("Deze actie is niet beschikbaar.", "This action isn’t available."),
        "failed" => t("De actie kon niet worden gestart", "The operation couldn’t be started"),
        "removeOnly" => t(
            "Alleen de snelkoppeling wordt verwijderd. Het programma en het oorspronkelijke bestand blijven behouden.",
            "Only the shortcut will be removed. The program and the original file will be kept.",
        ),
        "queued" => t("Wachten op starten…", "Waiting to start…"),
        "running" => t("Bezig…", "Working…"),
        "authenticating" => t("Wachten op toestemming…", "Waiting for authorisation…"),
        "completed" => t("Voltooid", "Completed"),
        "failedJob" => t("Mislukt", "Failed"),
        "cancelled" => t("Geannuleerd", "Cancelled"),
        "interrupted" => t("Uitkomst onbekend", "Outcome unknown"),
        "interruptedBody" => t(
            "Controleer eerst of de app is geïnstalleerd of verwijderd voordat je het opnieuw probeert.",
            "Check whether the app was installed or removed before trying again.",
        ),
        "preview" => t("Voorbeeld voltooid", "Preview completed"),
        "previewBody" => t("Er is niets geïnstalleerd of verwijderd.", "Nothing was installed or removed."),
        "cancelRequested" => t(
            "Annulering aangevraagd. Wachten op de pakketbeheerder…",
            "Cancellation requested. Waiting for the package manager…",
        ),
        "cancelFailed" => t("Annuleren is nu niet mogelijk", "Can’t cancel right now"),
        "statusFailed" => t(
            "De voortgang is tijdelijk niet beschikbaar. Opnieuw verbinden…",
            "Progress is temporarily unavailable. Reconnecting…",
        ),
        "shortcutProvider" => t("Snelkoppeling", "Shortcut"),
        "retryReview" => t(
            "Controleer Activiteit voordat je opnieuw probeert. Controleer de actie opnieuw voor een nieuwe poging.",
            "Check Activity and review the operation once more before you try again.",
        ),
        _ => return None,
    })
}

/// softwareT(key): the text, or the key itself when there is none (i18n.js makeT).
pub fn sw(key: &str) -> String {
    text(key).map(str::to_owned).unwrap_or_else(|| key.to_owned())
}

/// i18n.js fill: `{name}` replaced by its value.
pub fn fill(template: &str, params: &[(&str, &str)]) -> String {
    let mut s = template.to_owned();
    for (key, value) in params {
        s = s.replace(&format!("{{{key}}}"), value);
    }
    s
}

/// JavaScript truthiness of a JSON value.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `String(value)` for what the backend sends ("" for null).
pub fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// softwareJobFinished: completed, failed, cancelled or interrupted.
pub fn finished(job: &Value) -> bool {
    matches!(job["state"].as_str(), Some("completed" | "failed" | "cancelled" | "interrupted"))
}

/// The state text key of a job: a completed dry run is "preview", failed is "failedJob".
pub fn state_key(job: &Value) -> String {
    let state = job["state"].as_str().unwrap_or("");
    if truthy(&job["dryRun"]) && state == "completed" {
        "preview".into()
    } else if state == "failed" {
        "failedJob".into()
    } else {
        value_text(&job["state"])
    }
}

/// softwareActionLabel(action, name): "Installeren" or "‘Name’ installeren".
pub fn action_label(action: &Value, name: Option<&Value>) -> String {
    let action = value_text(action);
    match name.map(value_text).filter(|n| !n.is_empty()) {
        Some(n) => fill(&sw(&format!("{action}Name")), &[("name", &n)]),
        None => sw(&action),
    }
}

/// softwareProvider: the package manager's name.
pub fn provider(p: &Value) -> String {
    match p.as_str() {
        Some("shortcut") => sw("shortcutProvider"),
        Some("appimage") => "AppImage".into(),
        Some("packagekit") => "PackageKit".into(),
        Some("flatpak") => "Flatpak".into(),
        Some("snapd") => "Snap".into(),
        _ if truthy(p) => value_text(p),
        _ => String::new(),
    }
}

/// An info bar without a close button (`closable: false`).
pub fn info_bar(severity: Severity, title: &str, message: &str) -> gtk::Box {
    let bar = InfoBar::new(severity, title, message);
    bar.set_closable(false);
    bar.widget().clone()
}

/// A small note (`.sw-note`).
pub fn note(text: &str) -> gtk::Label {
    c::label(text, "installer-sw-note", true)
}

/// softwareDetails(info, {plan}): action (for a plan), package manager, scope, version, architecture, size and file
/// as a two-column list (`dl.sw-details`, 1fr : 2fr).
pub fn details(info: &Value, plan: bool) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.add_css_class("installer-sw-details");
    grid.set_column_spacing(16);
    grid.set_row_spacing(8);
    grid.set_column_homogeneous(true);
    let mut row = 0;
    let mut add = |key: &str, value: String| {
        if value.is_empty() {
            return;
        }
        let dt = c::label(&sw(key), "installer-sw-dt", true);
        dt.set_valign(gtk::Align::Start);
        dt.set_size_request(100, -1);
        let dd = c::label(&value, "installer-sw-dd", true);
        dd.set_valign(gtk::Align::Start);
        grid.attach(&dt, 0, row, 1, 1);
        grid.attach(&dd, 1, row, 2, 1);
        row += 1;
    };
    if plan {
        add("action", action_label(&info["action"], None));
    }
    add("provider", provider(&info["provider"]));
    add("scope", sw(if info["scope"] == "system" { "system" } else { "user" }));
    let meta = if truthy(&info["info"]) { &info["info"] } else { info };
    add("version", value_text(&meta["version"]));
    add("architecture", value_text(&meta["architecture"]));
    if let Some(size) = meta["size"].as_f64().filter(|s| s.is_finite()) {
        add("size", format!("{} MB", kit::format::number(size / 1e6, 2)));
    }
    let source = if truthy(&info["sourcePath"]) { &info["sourcePath"] } else { &meta["sourcePath"] };
    add("source", value_text(source));
    grid
}

thread_local! {
    static REVIEWING: Cell<bool> = const { Cell::new(false) };
}

/// reviewSoftware(args): software.prepare, then the review dialog; `done(job)` with the job software.execute
/// started after the reviewed plan's own button, None when nothing ran (or a review is already open).
pub fn review(parent: &gtk::Widget, args: Value, done: impl FnOnce(Option<Value>) + 'static) {
    if REVIEWING.with(|r| r.replace(true)) {
        done(None);
        return;
    }
    let parent = parent.clone();
    kit::ctx().call("software.prepare", args, move |result| match result {
        Ok(plan) => show_review(&parent, plan, Box::new(done)),
        Err(e) => {
            REVIEWING.with(|r| r.set(false));
            Toast::new(&sw("failed")).message(&e.message).severity(Severity::Error).show(&parent);
            done(None);
        }
    });
}

fn show_review(parent: &gtk::Widget, plan: Value, done: Box<dyn FnOnce(Option<Value>)>) {
    let action = plan["action"].as_str().unwrap_or("").to_owned();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 16);
    content.add_css_class("installer-sw-review");
    content.set_widget_name("installer-review-plan");
    content.append(&c::label(&value_text(&plan["name"]), "installer-sw-name", true));
    if truthy(&plan["description"]) {
        content.append(&c::label(&value_text(&plan["description"]), "installer-sw-description", true));
    }
    content.append(&details(&plan, true));
    if action == "removeShortcut" {
        content.append(&info_bar(Severity::Info, "", &sw("removeOnly")));
    }
    for n in plan["notes"].as_array().into_iter().flatten() {
        content.append(&note(&value_text(n)));
    }
    if truthy(&plan["requiresAuth"]) {
        content.append(&info_bar(Severity::Info, "", &sw("auth")));
    }
    if !truthy(&plan["canExecute"]) {
        let reason = if truthy(&plan["reason"]) { value_text(&plan["reason"]) } else { sw("unavailable") };
        content.append(&info_bar(Severity::Warning, "", &reason));
    }
    let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
    status.add_css_class("installer-sw-status");
    status.set_visible(false);
    content.append(&status);

    let plan_id = plan["planId"].as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    let executable = plan["canExecute"] == true && plan_id.is_some();
    let mut dialog = Dialog::new(&sw(if action == "install" { "review" } else { "reviewRemove" }))
        // The page's dialog is 560 px wide (border-box): a long file name wraps instead of widening it.
        .body(&crate::clamp::Clamp::new(&content, 560 - 2 - 48, false))
        .width(560)
        .close_label(&sw("cancel"))
        .default_button(DialogResult::Close);
    if action != "install" {
        dialog = dialog.danger();
    }
    if executable {
        dialog = dialog.primary(&action_label(&plan["action"], Some(&plan["name"])));
    }
    let job: Rc<RefCell<Option<Value>>> = Rc::default();
    let attempted = Rc::new(Cell::new(false));
    let busy = Rc::new(Cell::new(false));
    {
        let (job, busy) = (job.clone(), busy.clone());
        dialog = dialog.on_primary(move |d| {
            let Some(plan_id) = plan_id.clone().filter(|_| executable && !attempted.get()) else { return false };
            attempted.set(true);
            c::clear(&status);
            status.set_visible(false);
            d.set_busy(true);
            busy.set(true);
            let (d, status, job, busy) = (d.clone(), status.clone(), job.clone(), busy.clone());
            kit::ctx().call("software.execute", json!({ "planId": plan_id }), move |result| {
                busy.set(false);
                match result {
                    Ok(started) => {
                        *job.borrow_mut() = Some(started);
                        d.close(DialogResult::Primary);
                    }
                    Err(e) => {
                        d.set_busy(false);
                        status.append(&info_bar(Severity::Error, &sw("failed"), &e.message));
                        status.append(&note(&sw("retryReview")));
                        status.set_visible(true);
                        // A failed request may have used up the single-use plan: another review is needed to retry.
                        d.set_primary_enabled(false);
                    }
                }
            });
            false
        });
    }
    // A language switch closes the review unless it is busy (software.js onLangChange).
    let open: Rc<RefCell<Option<Dialog>>> = Rc::default();
    {
        let (open, busy, armed) = (open.clone(), busy.clone(), Cell::new(false));
        kit::i18n::bind(&content, move |_| {
            if !armed.replace(true) || busy.get() {
                return;
            }
            let d = open.borrow().clone();
            if let Some(d) = d {
                d.close(DialogResult::Close);
            }
        });
    }
    let closing = open.clone();
    dialog.show(parent, move |_| {
        closing.borrow_mut().take();
        REVIEWING.with(|r| r.set(false));
        let started = job.borrow_mut().take();
        done(started);
    });
    if dialog.is_open() {
        *open.borrow_mut() = Some(dialog);
    }
}

struct JobInner {
    el: gtk::Box,
    job: RefCell<Value>,
    disposed: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
    pending: Cell<bool>,
    failure: RefCell<String>,
    cancelling: Cell<bool>,
    revision: Cell<u64>,
    listener: Cell<Option<kit::ListenerId>>,
    on_change: Box<dyn Fn(&Value)>,
}

/// createSoftwareJobView: a job's name, action, state, progress bar, messages and Annuleren; it polls
/// software.status every 500 ms (and on software.changed) until the job is finished, since a job may run in
/// another host. `on_change(job)` after every new snapshot.
pub struct JobView {
    inner: Rc<JobInner>,
}

impl JobView {
    pub fn new(job: Value, on_change: impl Fn(&Value) + 'static) -> JobView {
        let el = gtk::Box::new(gtk::Orientation::Vertical, 16);
        el.add_css_class("installer-sw-job");
        el.set_widget_name("installer-job-view");
        let inner = Rc::new(JobInner {
            el,
            job: RefCell::new(job),
            disposed: Cell::new(false),
            timer: RefCell::new(None),
            pending: Cell::new(false),
            failure: RefCell::new(String::new()),
            cancelling: Cell::new(false),
            revision: Cell::new(0),
            listener: Cell::new(None),
            on_change: Box::new(on_change),
        });
        let weak = Rc::downgrade(&inner);
        let id = kit::ctx().on("software.changed", move |_, _| {
            if let Some(me) = weak.upgrade()
                && !me.finished_now()
            {
                me.poll();
            }
        });
        inner.listener.set(Some(id));
        let weak = Rc::downgrade(&inner);
        // Renders now and again after a language switch.
        kit::i18n::bind(&inner.el, move |_| {
            if let Some(me) = weak.upgrade()
                && !me.disposed.get()
            {
                me.render();
            }
        });
        if !inner.finished_now() {
            inner.poll();
        }
        JobView { inner }
    }

    pub fn widget(&self) -> &gtk::Box {
        &self.inner.el
    }

    /// Stop polling and listening.
    pub fn dispose(&self) {
        self.inner.disposed.set(true);
        self.inner.clear_timer();
        if let Some(id) = self.inner.listener.take() {
            kit::ctx().off(id);
        }
    }
}

impl JobInner {
    fn finished_now(&self) -> bool {
        finished(&self.job.borrow())
    }

    fn clear_timer(&self) {
        let timer = self.timer.borrow_mut().take();
        if let Some(id) = timer {
            id.remove();
        }
    }

    fn render(self: &Rc<Self>) {
        let el = &self.el;
        c::clear(el);
        let job = self.job.borrow().clone();
        let terminal = finished(&job);
        let name = value_text(&job["name"]);
        el.append(&c::label(&name, "installer-sw-name", true));
        el.append(&c::label(
            &format!("{} · {}", action_label(&job["action"], None), provider(&job["provider"])),
            "installer-sw-job-action",
            true,
        ));
        el.append(&c::label(&sw(&state_key(&job)), "installer-sw-job-state", true));
        let bar = c::ProgressBar::new();
        let progress = job["progress"].as_f64().filter(|p| p.is_finite());
        bar.set_fraction(progress.unwrap_or(0.0) / 100.0);
        bar.set_indeterminate(!terminal && progress.is_none());
        if job["state"] == "failed" {
            bar.set_state(ProgressState::Error);
        }
        bar.widget().update_property(&[gtk::accessible::Property::Label(&name)]);
        el.append(bar.widget());
        if truthy(&job["message"]) {
            el.append(&c::label(&value_text(&job["message"]), "installer-sw-job-message", true));
        }
        if truthy(&job["dryRun"]) && job["state"] == "completed" {
            el.append(&info_bar(Severity::Info, "", &sw("previewBody")));
        }
        if job["state"] == "interrupted" {
            el.append(&info_bar(Severity::Warning, "", &sw("interruptedBody")));
        }
        if truthy(&job["error"]["message"]) {
            el.append(&info_bar(Severity::Error, "", &value_text(&job["error"]["message"])));
        }
        if truthy(&job["cancelRequested"]) && !terminal {
            el.append(&note(&sw("cancelRequested")));
        }
        let failure = self.failure.borrow().clone();
        if !failure.is_empty() {
            el.append(&info_bar(Severity::Warning, "", &failure));
        }
        if !terminal {
            let cancel = c::button(&sw("cancel"), Variant::Standard);
            cancel.set_widget_name("installer-cancel");
            cancel.set_halign(gtk::Align::Start);
            cancel.set_sensitive(
                !self.cancelling.get() && !truthy(&job["cancelRequested"]) && truthy(&job["cancellable"]),
            );
            let weak = Rc::downgrade(self);
            cancel.connect_clicked(move |_| {
                if let Some(me) = weak.upgrade() {
                    me.cancel();
                }
            });
            el.append(&cancel);
        }
    }

    fn cancel(self: &Rc<Self>) {
        let job = self.job.borrow().clone();
        if self.cancelling.get() || truthy(&job["cancelRequested"]) || !truthy(&job["cancellable"]) {
            return;
        }
        self.cancelling.set(true);
        self.revision.set(self.revision.get() + 1);
        self.render();
        let weak = Rc::downgrade(self);
        let job_id = job["jobId"].clone();
        kit::ctx().call("software.cancel", json!({ "jobId": job_id }), move |result| {
            let Some(me) = weak.upgrade() else { return };
            match result {
                Ok(value) => {
                    if value["jobId"] == job_id {
                        *me.job.borrow_mut() = value;
                    }
                    me.failure.borrow_mut().clear();
                }
                Err(e) => {
                    *me.failure.borrow_mut() = if e.message.is_empty() { sw("cancelFailed") } else { e.message };
                }
            }
            me.cancelling.set(false);
            me.revision.set(me.revision.get() + 1);
            if !me.disposed.get() {
                me.render();
                me.poll();
            }
        });
    }

    fn poll(self: &Rc<Self>) {
        if self.disposed.get() || self.pending.get() {
            return;
        }
        self.pending.set(true);
        self.clear_timer();
        let revision = self.revision.get();
        let job_id = self.job.borrow()["jobId"].clone();
        let weak = Rc::downgrade(self);
        kit::ctx().call("software.status", json!({ "jobId": job_id }), move |result| {
            let Some(me) = weak.upgrade() else { return };
            if !me.disposed.get() && revision == me.revision.get() {
                match result {
                    Ok(value) => {
                        *me.job.borrow_mut() = value.clone();
                        me.failure.borrow_mut().clear();
                        me.render();
                        (me.on_change)(&value);
                    }
                    Err(_) => {
                        *me.failure.borrow_mut() = sw("statusFailed");
                        me.render();
                    }
                }
            }
            me.pending.set(false);
            me.schedule();
        });
    }

    fn schedule(self: &Rc<Self>) {
        self.clear_timer();
        if self.disposed.get() || self.finished_now() {
            return;
        }
        let weak = Rc::downgrade(self);
        let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
            if let Some(me) = weak.upgrade() {
                // This source has run: forget it before polling (which would remove it otherwise).
                me.timer.borrow_mut().take();
                me.poll();
            }
        });
        *self.timer.borrow_mut() = Some(id);
    }
}
