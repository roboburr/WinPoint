//! Win11 GNOME App Installer in GTK 4 (../../APPS.md): 6ebf169f:vendor/ui/js/apps/installer/main.js with
//! js/core/software.js, css/installer.css and css/software.css on the kit. Choose (GTK's file dialog, so the
//! FileChooser portal's picker) or drop one installation file; it is checked (software.inspect) and shown with its
//! details; "Installatie controleren" opens the review (software.prepare) whose own button alone runs it
//! (software.execute); the job's progress (software.status) and the Activity list (software.jobs) follow. `--params
//! {"uninstall": id}` reviews a removal instead (software.uninstallInfo). One process: a relaunch presents the window
//! and takes its new file or removal (the WebKit app's app.navigate).
mod clamp;
mod software;

use clamp::Clamp;
use kit::controls::{self as c, InfoBar, SettingsCard, SettingsGroup, Severity, Variant};
use kit::frame::{AppWindow, WindowOptions};
use kit::prelude::*;
use kit::t;
use serde_json::Map;
use software::JobView;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const USAGE: &str = "win11-gtk-installer [FILE | --params JSON]

  FILE                                 check this installation file (an absolute path)
  --params '{\"path\":\"/x.deb\"}'        the same
  --params '{\"uninstall\":\"app.desktop\"}'   review the removal of this app
  A second launch is handed to the running window.";

const SPEC: kit::AppSpec =
    kit::AppSpec::new("installer", "io.github.win11shell.Installer", "App-installatieprogramma", "App Installer")
        .icon("download")
        .usage(USAGE);

/// The file dialog's filter (main.js patterns).
const PATTERNS: [&str; 6] = ["*.AppImage", "*.appimage", "*.deb", "*.flatpakref", "*.flatpak", "*.snap"];

fn main() -> glib::ExitCode {
    let running: Rc<RefCell<Option<Rc<App>>>> = Rc::default();
    kit::run(SPEC, move |ctx, args, first| {
        let params = params(&args);
        if !first {
            let app = running.borrow().clone();
            if let Some(app) = app {
                app.win.present();
                app.navigate(&params);
            }
            return;
        }
        kit::theme::install_app_css(include_str!("../css/installer.css"));
        let app = App::new(ctx);
        app.win.present();
        app.start(&params);
        *running.borrow_mut() = Some(app);
    })
}

/// The launch arguments as the WebKit app got them in its URL query (`git show 6ebf169f:host/src/lib.rs`,
/// run_inner): `--params JSON` gives its keys (strings as they are, other values as JSON text), `--tab-ticket T`
/// gives tabTicket, otherwise the first argument is `path` and the others `tabs`.
fn params(args: &[String]) -> Map<String, Value> {
    let mut args: Vec<&str> =
        args.iter().map(String::as_str).filter(|a| !matches!(*a, "--dry-run" | "--http-only")).collect();
    if args.first() == Some(&"--app") {
        args.drain(..args.len().min(2));
    }
    let mut out = Map::new();
    match args.first().copied() {
        Some("--tab-ticket") => {
            if let Some(ticket) = args.get(1) {
                out.insert("tabTicket".into(), json!(ticket));
            }
        }
        Some("--params") => {
            if let Some(Value::Object(given)) = args.get(1).and_then(|raw| serde_json::from_str::<Value>(raw).ok()) {
                for (key, value) in given {
                    let text = value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string());
                    out.insert(key, Value::String(text));
                }
            }
        }
        Some(path) => {
            out.insert("path".into(), json!(path));
            if args.len() > 1 {
                out.insert("tabs".into(), json!(serde_json::to_string(&args[1..]).unwrap_or_default()));
            }
        }
        None => {}
    }
    out
}

/// The first label inside a widget.
fn find_label(w: &gtk::Widget) -> Option<gtk::Label> {
    if let Some(l) = w.downcast_ref::<gtk::Label>() {
        return Some(l.clone());
    }
    let mut child = w.first_child();
    while let Some(c) = child {
        if let Some(found) = find_label(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// A label that follows the language.
fn text(class: &str, wrap: bool, nl: &'static str, en: &'static str) -> gtk::Label {
    let l = c::label(t(nl, en), class, wrap);
    kit::i18n::bind(&l, move |l| l.set_label(t(nl, en)));
    l
}

#[derive(Default)]
struct State {
    /// The checked file or app: ("install" | "uninstall", software.inspect / software.uninstallInfo).
    selection: Option<(&'static str, Value)>,
    loading: bool,
    /// The last check's error text.
    error: String,
    request: u64,
    /// A review is open or starting.
    busy: bool,
    jobs: Vec<Value>,
    jobs_error: String,
    jobs_pending: bool,
    jobs_timer: Option<glib::SourceId>,
    job_view: Option<JobView>,
}

struct App {
    ctx: Rc<kit::Ctx>,
    win: AppWindow,
    page: gtk::Box,
    chooser: gtk::Button,
    detail: gtk::Box,
    progress: gtk::Box,
    activity: gtk::Box,
    state: RefCell<State>,
}

impl App {
    fn new(ctx: &Rc<kit::Ctx>) -> Rc<App> {
        let win = AppWindow::new(WindowOptions { width: 980, height: 720, ..Default::default() });
        let page = gtk::Box::new(gtk::Orientation::Vertical, 24);
        page.add_css_class("installer-page");

        let header = gtk::Box::new(gtk::Orientation::Vertical, 8);
        header.append(&text("installer-title", true, "App-installatieprogramma", "App Installer"));
        header.append(&text(
            "installer-intro",
            true,
            "Installeer een app uit een bestand of bekijk de voortgang van je installaties.",
            "Install an app from a file or view the progress of your installations.",
        ));
        page.append(&header);

        // The drop zone: an icon, "Sleep een installatiebestand hierheen", the formats and "Bestand kiezen".
        let drop = gtk::Box::new(gtk::Orientation::Vertical, 12);
        drop.add_css_class("installer-drop");
        drop.set_widget_name("installer-drop");
        kit::i18n::bind(&drop, |d| {
            d.update_property(&[gtk::accessible::Property::Label(t(
                "Sleep een installatiebestand hierheen",
                "Drag an installation file here",
            ))])
        });
        let icon = kit::icons::app_icon("application-x-executable", 48);
        icon.set_halign(gtk::Align::Center);
        drop.append(&icon);
        let drop_title =
            text("installer-drop-title", true, "Sleep een installatiebestand hierheen", "Drag an installation file here");
        drop_title.set_justify(gtk::Justification::Center);
        drop_title.set_xalign(0.5);
        drop_title.set_halign(gtk::Align::Center);
        drop.append(&drop_title);
        let formats = text(
            "installer-formats",
            true,
            "AppImage, Debian (.deb), Flatpak en Snap",
            "AppImage, Debian (.deb), Flatpak and Snap",
        );
        formats.set_halign(gtk::Align::Center);
        drop.append(&formats);
        let chooser = c::button_with_glyph("folder-open", t("Bestand kiezen", "Choose file"), Variant::Standard);
        chooser.set_widget_name("installer-choose");
        chooser.set_halign(gtk::Align::Center);
        if let Some(l) = find_label(chooser.upcast_ref()) {
            kit::i18n::bind(&l, |l| l.set_label(t("Bestand kiezen", "Choose file")));
        }
        drop.append(&chooser);
        page.append(&drop);

        let detail = gtk::Box::new(gtk::Orientation::Vertical, 16);
        detail.add_css_class("installer-detail");
        detail.set_widget_name("installer-detail");
        detail.set_visible(false);
        page.append(&detail);
        let progress = gtk::Box::new(gtk::Orientation::Vertical, 0);
        progress.add_css_class("installer-progress");
        progress.set_widget_name("installer-progress");
        progress.set_visible(false);
        page.append(&progress);
        let section = gtk::Box::new(gtk::Orientation::Vertical, 0);
        section.append(&text("installer-section-title", false, "Activiteit", "Activity"));
        let activity = gtk::Box::new(gtk::Orientation::Vertical, 0);
        activity.set_widget_name("installer-activity");
        section.append(&activity);
        page.append(&section);

        let outer = Clamp::new(&page, 1000, true);
        let scroller = c::scrolled(&outer);
        win.set_content(&scroller);

        let app = Rc::new(App {
            ctx: ctx.clone(),
            win,
            page,
            chooser,
            detail,
            progress,
            activity,
            state: RefCell::new(State::default()),
        });

        let weak = Rc::downgrade(&app);
        app.chooser.connect_clicked(move |_| {
            if let Some(app) = weak.upgrade() {
                app.browse();
            }
        });
        let weak = Rc::downgrade(&app);
        outer.connect_width(move |width| {
            if let Some(app) = weak.upgrade() {
                if width <= 650 {
                    app.page.add_css_class("installer-narrow");
                } else {
                    app.page.remove_css_class("installer-narrow");
                }
            }
        });

        // A file dropped anywhere on the window (as the WebKit host's native drop); the zone lights up under it.
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        let weak = Rc::downgrade(&app);
        let zone = drop.clone();
        target.connect_drop(move |_, value, _, _| {
            zone.remove_css_class("installer-dragging");
            let Ok(list) = value.get::<gdk::FileList>() else { return false };
            let paths: Vec<String> = list
                .files()
                .iter()
                .take(64)
                .filter_map(|f| f.path())
                .filter(|p| p.is_absolute())
                .filter_map(|p| p.to_str().map(str::to_owned))
                .filter(|p| p.len() <= 8192 && !p.contains('\0'))
                .collect();
            if let Some(app) = weak.upgrade() {
                app.dropped(paths);
            }
            true
        });
        scroller.add_controller(target);
        let motion = gtk::DropControllerMotion::new();
        let zone = drop.clone();
        motion.connect_enter(move |_, _, _| zone.add_css_class("installer-dragging"));
        let zone = drop.clone();
        motion.connect_leave(move |_| zone.remove_css_class("installer-dragging"));
        drop.add_controller(motion);

        let weak = Rc::downgrade(&app);
        ctx.on("software.changed", move |_, _| {
            if let Some(app) = weak.upgrade() {
                app.load_jobs();
            }
        });
        let weak = Rc::downgrade(&app);
        kit::i18n::on_change(move || {
            if let Some(app) = weak.upgrade() {
                app.render_detail();
                app.render_jobs();
            }
        });
        app
    }

    fn start(self: &Rc<Self>, params: &Map<String, Value>) {
        self.render_detail();
        self.render_jobs();
        self.load_jobs();
        self.navigate(params);
    }

    /// The launch's or a relaunch's arguments (main.js frame.onAppNavigate): a removal to review, or a file.
    fn navigate(self: &Rc<Self>, params: &Map<String, Value>) {
        let get = |key: &str| params.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_owned);
        if let Some(id) = get("uninstall") {
            self.removal(&id);
        } else if let Some(path) = get("path") {
            self.inspect(&path);
        }
    }

    fn begin(&self) -> Option<u64> {
        let mut s = self.state.borrow_mut();
        if s.busy {
            return None;
        }
        s.request += 1;
        s.loading = true;
        s.selection = None;
        s.error.clear();
        Some(s.request)
    }

    fn finish(self: &Rc<Self>, request: u64, result: Result<(&'static str, Value), String>) {
        {
            let mut s = self.state.borrow_mut();
            if request != s.request {
                return;
            }
            match result {
                Ok(selection) => s.selection = Some(selection),
                Err(message) => s.error = message,
            }
            s.loading = false;
        }
        self.render_detail();
    }

    fn inspect(self: &Rc<Self>, path: &str) {
        let Some(request) = self.begin() else { return };
        self.render_detail();
        if !path.starts_with('/') {
            self.finish(
                request,
                Err(t(
                    "Kies een bestand met de knop of sleep het vanuit Verkenner.",
                    "Use the button to choose a file, or drag it from File Explorer.",
                )
                .to_owned()),
            );
            return;
        }
        let weak = Rc::downgrade(self);
        self.ctx.call("software.inspect", json!({ "path": path }), move |result| {
            if let Some(app) = weak.upgrade() {
                app.finish(request, result.map(|info| ("install", info)).map_err(|e| e.message));
            }
        });
    }

    fn removal(self: &Rc<Self>, id: &str) {
        let Some(request) = self.begin() else { return };
        self.render_detail();
        let weak = Rc::downgrade(self);
        self.ctx.call("software.uninstallInfo", json!({ "id": id }), move |result| {
            if let Some(app) = weak.upgrade() {
                app.finish(request, result.map(|info| ("uninstall", info)).map_err(|e| e.message));
            }
        });
    }

    fn dropped(self: &Rc<Self>, paths: Vec<String>) {
        if self.state.borrow().busy {
            return;
        }
        if paths.len() != 1 {
            {
                let mut s = self.state.borrow_mut();
                s.request += 1;
                s.loading = false;
                s.selection = None;
                s.error = t("Kies één installatiebestand tegelijk.", "Choose one installation file at a time.").to_owned();
            }
            self.render_detail();
            return;
        }
        self.inspect(&paths[0]);
    }

    /// "Bestand kiezen": GTK's file dialog (the desktop's FileChooser portal, Win11's picker when it is chosen),
    /// filtered to installation files.
    fn browse(self: &Rc<Self>) {
        if self.state.borrow().busy {
            return;
        }
        let filter = gtk::FileFilter::new();
        filter.set_name(Some(t("Installatiebestanden", "Installation files")));
        for p in PATTERNS {
            filter.add_pattern(p);
        }
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);
        let dialog = gtk::FileDialog::builder()
            .title(t("Bestand kiezen", "Choose file"))
            .modal(true)
            .filters(&filters)
            .default_filter(&filter)
            .build();
        let weak = Rc::downgrade(self);
        dialog.open(Some(self.win.window()), gio::Cancellable::NONE, move |result| {
            let Some(app) = weak.upgrade() else { return };
            match result {
                Ok(file) => {
                    if let Some(path) = file.path().and_then(|p| p.to_str().map(str::to_owned)) {
                        app.inspect(&path);
                    }
                }
                Err(e) if e.matches(gtk::DialogError::Dismissed) || e.matches(gtk::DialogError::Cancelled) => {}
                Err(e) => {
                    app.state.borrow_mut().error = e.message().to_owned();
                    app.render_detail();
                }
            }
        });
    }

    /// The checked file (or app to remove): its card, details and notes, and "Installatie controleren".
    fn render_detail(self: &Rc<Self>) {
        let detail = &self.detail;
        c::clear(detail);
        let s = self.state.borrow();
        self.chooser.set_sensitive(!s.busy);
        if s.loading {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("installer-loading");
            row.set_halign(gtk::Align::Center);
            row.append(c::ProgressRing::new(24, None).widget());
            row.append(&c::label(t("Bestand controleren…", "Checking file…"), "", false));
            detail.append(&row);
            detail.set_visible(true);
            return;
        }
        if !s.error.is_empty() {
            detail.append(&software::info_bar(
                Severity::Error,
                t("Het bestand kon niet worden gecontroleerd", "The file couldn’t be checked"),
                &s.error,
            ));
            detail.set_visible(true);
            return;
        }
        let Some((action, info)) = s.selection.as_ref() else {
            detail.set_visible(false);
            return;
        };
        let group = SettingsGroup::new("", "");
        let card = SettingsCard::new("", &software::value_text(&info["name"]), info["description"].as_str().unwrap_or(""));
        let icon = kit::icons::app_icon("application-x-executable", 40);
        // .si-detail .fl-sc__icon: a 40 px box.
        icon.set_size_request(40, 40);
        card.set_icon_widget(&icon);
        card.set_control(&c::label(&software::provider(&info["provider"]), "installer-provider", false));
        group.add(card.widget());
        detail.append(group.widget());
        detail.append(&software::details(info, false));
        for n in info["notes"].as_array().into_iter().flatten() {
            detail.append(&software::note(&software::value_text(n)));
        }
        if *action == "uninstall" && info["action"] == "removeShortcut" {
            detail.append(&software::info_bar(Severity::Info, "", &software::sw("removeOnly")));
        }
        if !software::truthy(&info["supported"]) {
            let reason = if software::truthy(&info["reason"]) {
                software::value_text(&info["reason"])
            } else {
                t("Deze actie is niet beschikbaar.", "This action isn’t available.").to_owned()
            };
            detail.append(&software::info_bar(Severity::Warning, "", &reason));
        } else {
            let label = if *action == "install" {
                t("Installatie controleren", "Review installation")
            } else {
                t("Verwijderen controleren", "Review removal")
            };
            let review = c::button(label, Variant::Accent);
            review.set_widget_name("installer-review");
            review.set_halign(gtk::Align::Start);
            review.set_sensitive(!s.busy);
            let weak = Rc::downgrade(self);
            review.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.start_review();
                }
            });
            detail.append(&review);
        }
        detail.set_visible(true);
    }

    fn start_review(self: &Rc<Self>) {
        let args = {
            let mut s = self.state.borrow_mut();
            if s.busy {
                return;
            }
            let Some((action, info)) = s.selection.as_ref() else { return };
            let mut args = Map::new();
            args.insert("action".into(), json!(action));
            let (key, value) = if *action == "install" { ("path", &info["sourcePath"]) } else { ("id", &info["id"]) };
            if !value.is_null() {
                args.insert(key.into(), value.clone());
            }
            s.busy = true;
            Value::Object(args)
        };
        self.render_detail();
        let weak = Rc::downgrade(self);
        software::review(self.win.window().upcast_ref(), args, move |job| {
            let Some(app) = weak.upgrade() else { return };
            if let Some(job) = job {
                app.show_job(job);
                app.load_jobs();
            }
            app.state.borrow_mut().busy = false;
            app.render_detail();
        });
    }

    /// The progress panel for one job (replacing the one shown).
    fn show_job(self: &Rc<Self>, job: Value) {
        let old = self.state.borrow_mut().job_view.take();
        if let Some(view) = old {
            view.dispose();
        }
        c::clear(&self.progress);
        let weak = Rc::downgrade(self);
        let view = JobView::new(job, move |value| {
            if let Some(app) = weak.upgrade() {
                app.job_changed(value);
            }
        });
        self.progress.append(view.widget());
        self.progress.set_visible(true);
        self.state.borrow_mut().job_view = Some(view);
    }

    fn job_changed(self: &Rc<Self>, value: &Value) {
        {
            let mut s = self.state.borrow_mut();
            match s.jobs.iter().position(|j| j["jobId"] == value["jobId"]) {
                Some(i) => s.jobs[i] = value.clone(),
                None => s.jobs.insert(0, value.clone()),
            }
        }
        self.render_jobs();
    }

    /// Activity: every job, newest first, with "Voortgang bekijken".
    fn render_jobs(self: &Rc<Self>) {
        c::clear(&self.activity);
        let s = self.state.borrow();
        if !s.jobs_error.is_empty() {
            let bar = InfoBar::new(Severity::Warning, "", &s.jobs_error);
            bar.set_closable(false);
            let weak = Rc::downgrade(self);
            bar.set_action(Some((
                t("Opnieuw proberen", "Try again"),
                Rc::new(move || {
                    if let Some(app) = weak.upgrade() {
                        app.load_jobs();
                    }
                }),
            )));
            self.activity.append(bar.widget());
            return;
        }
        if s.jobs.is_empty() {
            self.activity.append(&c::empty_state(
                "download",
                t("Nog geen installaties", "No installations yet"),
                t(
                    "De voortgang van je installaties en verwijderingen verschijnt hier.",
                    "Your installation and removal progress will appear here.",
                ),
                None,
            ));
            return;
        }
        let group = SettingsGroup::new("", "");
        for job in &s.jobs {
            let description = format!(
                "{} · {} · {}",
                software::action_label(&job["action"], None),
                software::provider(&job["provider"]),
                software::sw(&software::state_key(job)),
            );
            let card = SettingsCard::new("download", &software::value_text(&job["name"]), &description);
            let id = software::value_text(&job["jobId"]);
            let show = c::button(t("Voortgang bekijken", "View progress"), Variant::Standard);
            show.set_widget_name(&format!("installer-show-{id}"));
            let weak = Rc::downgrade(self);
            let value = job.clone();
            show.connect_clicked(move |_| {
                if let Some(app) = weak.upgrade() {
                    app.show_job(value.clone());
                }
            });
            card.set_control(&show);
            card.widget().set_widget_name(&format!("installer-job-{id}"));
            group.add(card.widget());
        }
        self.activity.append(group.widget());
    }

    /// software.jobs, again every 500 ms while one is unfinished or the list failed.
    fn load_jobs(self: &Rc<Self>) {
        {
            let mut s = self.state.borrow_mut();
            if s.jobs_pending {
                return;
            }
            s.jobs_pending = true;
            if let Some(id) = s.jobs_timer.take() {
                id.remove();
            }
        }
        let weak = Rc::downgrade(self);
        self.ctx.call("software.jobs", json!({}), move |result| {
            let Some(app) = weak.upgrade() else { return };
            let again = {
                let mut s = app.state.borrow_mut();
                match result {
                    Ok(value) => {
                        s.jobs = match value.as_array() {
                            Some(list) => list.clone(),
                            None => value["jobs"].as_array().cloned().unwrap_or_default(),
                        };
                        s.jobs_error.clear();
                    }
                    Err(e) => {
                        s.jobs_error = if e.message.is_empty() {
                            t("De activiteit kon niet worden geladen.", "Activity couldn’t be loaded.").to_owned()
                        } else {
                            e.message
                        };
                    }
                }
                s.jobs_pending = false;
                !s.jobs_error.is_empty() || s.jobs.iter().any(|j| !software::finished(j))
            };
            app.render_jobs();
            if again {
                let weak = Rc::downgrade(&app);
                let id = glib::timeout_add_local_once(Duration::from_millis(500), move || {
                    if let Some(app) = weak.upgrade() {
                        // This source has run: forget it before loading (which would remove it otherwise).
                        app.state.borrow_mut().jobs_timer.take();
                        app.load_jobs();
                    }
                });
                app.state.borrow_mut().jobs_timer = Some(id);
            }
        });
    }
}
