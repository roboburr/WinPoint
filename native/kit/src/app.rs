//! Process setup (`kit::run`): GSK_RENDERER=cairo unless set or WIN11_APP_GL=1 (RAM; with cairo, manual font
//! rendering) and GDK_DEBUG=portals unless set ([`own_environment`]), the prgname and GApplication id (= the
//! Wayland app id the extension knows the window by), HANDLES_COMMAND_LINE so a second launch reaches the running
//! process, and at startup: the settings, the theme, the backend, the dev channel.
//!
//! Restarts (`shell.restartAll` of Windows Update, `shell.restart`): the backend never restarts itself; it ends with
//! a restart note (backend.rs) and [`backend_restarted`] asks the app's [`on_restart`] hook what to do: exec itself
//! again (the installed build, with a new backend and the activation token of the note), quit, or stay on a new
//! backend.
use crate::ctx::Ctx;
use gtk::prelude::*;
use gtk::{gio, glib};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

/// What the kit needs to know about an app.
#[derive(Clone, Debug)]
pub struct AppSpec {
    /// The app kind of APPS.md ("explorer", "settings", "taskmgr", "alles", "disks", "defaultapps",
    /// "installer"): the backend's `--app <kind>`, the extension's backdrop kind, the dev channel's app.
    pub kind: &'static str,
    /// The GApplication id, e.g. "io.github.win11shell.TaskManager".
    pub id: &'static str,
    /// The app's name (Dutch, English): the application name and the window title by default.
    pub title: (&'static str, &'static str),
    /// The title bar icon: a glyph name (icons.js) or an icon-theme / app icon name.
    pub icon: &'static str,
    /// The text `--help` prints (also without a display); empty: a one-line default.
    pub usage: &'static str,
    /// Start the backend (`win11-app --app <kind> --http-only`); true by default.
    pub backend: bool,
}

impl AppSpec {
    /// A spec with the "app" glyph, a default usage line and a backend.
    pub const fn new(kind: &'static str, id: &'static str, title_nl: &'static str, title_en: &'static str) -> AppSpec {
        AppSpec { kind, id, title: (title_nl, title_en), icon: "app", usage: "", backend: true }
    }

    /// The title bar icon (glyph or icon name).
    pub const fn icon(mut self, icon: &'static str) -> AppSpec {
        self.icon = icon;
        self
    }

    /// The `--help` text.
    pub const fn usage(mut self, usage: &'static str) -> AppSpec {
        self.usage = usage;
        self
    }

    /// Run without a backend (the gallery, tests).
    pub const fn without_backend(mut self) -> AppSpec {
        self.backend = false;
        self
    }

    fn help(&self) -> String {
        if self.usage.is_empty() {
            format!("win11-gtk-{} [arguments]\n", self.kind)
        } else if self.usage.ends_with('\n') {
            self.usage.to_owned()
        } else {
            format!("{}\n", self.usage)
        }
    }
}

thread_local! {
    static STARTED: Cell<bool> = const { Cell::new(false) };
    static RESTART_HOOK: RefCell<Option<Rc<dyn Fn(&Restart) -> RestartPlan>>> = const { RefCell::new(None) };
}

/// What the backend's restart asks of the app ([`on_restart`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Restart {
    /// `shell.restartAll`: every Win11 process restarts and runs the new build (false: a plain `shell.restart` of
    /// this app's backend)
    pub all: bool,
    /// this app's backend called `shell.restartAll` (Settings reopens on its update page)
    pub origin: bool,
}

/// The app's answer to a restart ([`on_restart`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestartPlan {
    /// exec the app again (the installed build) with these arguments (argv[1..]); window sizes are saved first
    Exec(Vec<String>),
    /// keep running on a new backend (a picker, work that must not go away)
    Stay,
    /// quit: nothing to reopen, the next start runs the new build
    Quit,
}

/// How the app restarts when its backend asks. Without a hook [`default_restart_plan`] applies. The hook runs on
/// the main thread; save what the new process needs (synchronously: an exec follows at once) and answer.
pub fn on_restart(f: impl Fn(&Restart) -> RestartPlan + 'static) {
    RESTART_HOOK.with(|h| *h.borrow_mut() = Some(Rc::new(f)));
}

/// Without a hook: a `shell.restartAll` execs the app with the arguments it was started with, or quits when it has
/// no window (an app kept alive by work of its own reopens nothing); a plain `shell.restart` keeps the app on a
/// new backend.
pub fn default_restart_plan(restart: &Restart, windows: bool) -> RestartPlan {
    match (restart.all, windows) {
        (false, _) => RestartPlan::Stay,
        (true, true) => RestartPlan::Exec(std::env::args().skip(1).collect()),
        (true, false) => RestartPlan::Quit,
    }
}

/// `current_exe()` names a binary replaced by rename (scripts/install.py) as `"<path> (deleted)"`: that path holds
/// the new build (host lib.rs `exec_path`).
pub fn exec_path(exe: PathBuf) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    match exe.as_os_str().as_bytes().strip_suffix(b" (deleted)") {
        Some(path) => PathBuf::from(std::ffi::OsStr::from_bytes(path)),
        None => exe,
    }
}

/// The file a restart execs: this binary's path, or the running image itself (`/proc/self/exe`) when that path
/// no longer names a file (host lib.rs `exec_target`).
pub fn exec_target(current: std::io::Result<PathBuf>) -> PathBuf {
    match current.map(exec_path) {
        Ok(path) if path.is_file() => path,
        _ => PathBuf::from("/proc/self/exe"),
    }
}

/// The backend restarted (ctx.rs on "backend.restart"): the hook's plan, carried out. An exec that fails keeps the
/// app running on a new backend, with its application id taken back (`after_failed_exec`).
pub(crate) fn backend_restarted(ctx: &Ctx) {
    let backend = ctx.backend();
    let Some(note) = backend.take_restart() else {
        backend.answer_restart(true);
        return;
    };
    let restart = Restart { all: note.all, origin: note.origin };
    let hook = RESTART_HOOK.with(|h| h.borrow().clone());
    let windows = !ctx.app().windows().is_empty();
    let plan = match hook {
        Some(hook) => hook(&restart),
        None => default_restart_plan(&restart, windows),
    };
    match plan {
        RestartPlan::Stay => backend.answer_restart(true),
        RestartPlan::Quit => {
            eprintln!("win11-kit: {}: restart: no window to reopen; quitting (the next start runs the new build)", ctx.kind());
            backend.answer_restart(false);
            backend.close();
            ctx.app().quit();
        }
        RestartPlan::Exec(args) => {
            for window in crate::frame::AppWindow::all() {
                window.save_size();
            }
            let error = exec_self(ctx.app(), &args, note.token.as_deref());
            eprintln!("win11-kit: {}: restart: {error}; staying on a new backend", ctx.kind());
            after_failed_exec(ctx.app(), ctx.kind());
            backend.answer_restart(true);
        }
    }
}

/// The app stays after a failed exec: undo what `exec_self` and `Command::exec` changed. The application id back
/// on the session bus (exec_self released it for the new image; GApplication's objects are still exported on this
/// connection, so the name is all that is missing): the next launch is handed to this process again instead of
/// becoming a second primary instance with a backend of its own. SIGPIPE ignored again, as Rust's runtime set it
/// at start (a failed `Command::exec` leaves it at SIG_DFL). native/shell/src/main.rs has the same for the shell.
fn after_failed_exec(app: &gtk::Application, kind: &str) {
    // SAFETY: signal(2) with SIG_IGN; nothing in the kit installs a SIGPIPE handler of its own
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let (Some(bus), Some(id)) = (app.dbus_connection(), app.application_id()) else { return };
    // flags 4: DBUS_NAME_FLAG_DO_NOT_QUEUE
    let reply = bus.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "RequestName",
        Some(&(id.as_str(), 4u32).to_variant()),
        Some(glib::VariantTy::new("(u)").expect("static type")),
        gio::DBusCallFlags::NONE,
        2000,
        gio::Cancellable::NONE,
    );
    match reply.map(|r| r.child_value(0).get::<u32>()) {
        // 1 PRIMARY_OWNER, 4 ALREADY_OWNER
        Ok(Some(1 | 4)) => {}
        Ok(code) => {
            eprintln!("win11-kit: {kind}: restart: {id} was taken by another instance while the exec failed (RequestName {code:?})")
        }
        Err(error) => eprintln!("win11-kit: {kind}: restart: taking {id} back: {error}"),
    }
}

/// exec this app again with `args` and the activation token. The application id is released first, so the new
/// image becomes the primary instance again (a launch in between becomes the primary one and gets our arguments).
fn exec_self(app: &gtk::Application, args: &[String], token: Option<&str>) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    let target = exec_target(std::env::current_exe());
    if let (Some(bus), Some(id)) = (app.dbus_connection(), app.application_id()) {
        let _ = bus.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "ReleaseName",
            Some(&(id.as_str(),).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            gio::Cancellable::NONE,
        );
        let _ = bus.flush_sync(gio::Cancellable::NONE);
    }
    let mut command = std::process::Command::new(&target);
    command.args(args).env_remove("XDG_ACTIVATION_TOKEN").env_remove("DESKTOP_STARTUP_ID");
    if let Some(token) = token {
        command.env("XDG_ACTIVATION_TOKEN", token).env("DESKTOP_STARTUP_ID", token);
    }
    command.exec()
}

/// The GTK switches an app sets for itself. GSK_RENDERER=cairo unless set or WIN11_APP_GL=1 (RAM). GDK_DEBUG=portals
/// unless set: outside a sandbox GTK 4 shows its own file dialog in this process; with the flag, GTK's file dialog
/// asks the desktop's FileChooser portal as a Flatpak app's does (Win11's picker when the portal configuration
/// chooses it; tests/gnome/scenarios/r2d-picker.py). The portal's own picker gets GDK_DEBUG=no-portals (host
/// picker.rs). The user's values go into WIN11SHELL_USER_GPU (unless a WinPoint parent passed its marker on), so the
/// backend gives an app it launches those and not ours (host files/actions.rs GPU_KEYS).
///
/// # Safety
/// Changes the process environment: no other thread may run.
unsafe fn own_environment() {
    let mut own = Vec::new();
    if std::env::var_os("GSK_RENDERER").is_none() && std::env::var("WIN11_APP_GL").as_deref() != Ok("1") {
        own.push(("GSK_RENDERER", "cairo"));
    }
    if std::env::var_os("GDK_DEBUG").is_none() {
        own.push(("GDK_DEBUG", "portals"));
    }
    if own.is_empty() {
        return;
    }
    if std::env::var_os("WIN11SHELL_USER_GPU").is_none() {
        let user: serde_json::Map<String, serde_json::Value> = ["GSK_RENDERER", "GDK_DEBUG"]
            .into_iter()
            .filter_map(|key| Some((key.to_owned(), std::env::var(key).ok()?.into())))
            .collect();
        unsafe { std::env::set_var("WIN11SHELL_USER_GPU", serde_json::Value::Object(user).to_string()) };
    }
    for (key, value) in own {
        unsafe { std::env::set_var(key, value) };
    }
}

/// Run the app: `handler(ctx, args, first)` runs for every launch, the first one and each later one handed over
/// by a second process (`args` are argv[1..] of that launch, `first` is true once). The app decides about
/// windows; create the first one synchronously in the handler (or hold the application while waiting), or the
/// process ends when the handler returns without a window. `--help` / `-h` print `spec.usage` (also without a
/// display) and never reach the handler.
pub fn run(spec: AppSpec, handler: impl Fn(&Rc<Ctx>, Vec<String>, bool) + 'static) -> glib::ExitCode {
    // SAFETY: first thing in main, before GTK or any other thread starts.
    unsafe { own_environment() };
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.iter().any(|a| a == "--help" || a == "-h") {
        print!("{}", spec.help());
        return glib::ExitCode::SUCCESS;
    }
    glib::set_prgname(Some(spec.id));
    let app = gtk::Application::builder()
        .application_id(spec.id)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    let startup_spec = spec.clone();
    app.connect_startup(move |app| {
        // GSK's cairo renderer with GTK 4.22's automatic font rendering inks a glyph's top pixel row outside the
        // extents GTK computed for it: small text loses the dots of its i's (Disks' 12 px captions), and text that
        // moves leaves that row behind (Explorer's column resize, the Alles header; tests/gnome/scenarios/kit-ui.py).
        // Manual font rendering (hinted, pixel-aligned metrics) keeps the ink inside and looks the same.
        if std::env::var("GSK_RENDERER").as_deref() == Ok("cairo")
            && let Some(settings) = gtk::Settings::default()
        {
            settings.set_gtk_font_rendering(gtk::FontRendering::Manual);
        }
        crate::theme::install();
        let ctx = Ctx::create(app, startup_spec.clone());
        glib::set_application_name(crate::i18n::t(startup_spec.title.0, startup_spec.title.1));
        ctx.apply_look();
        crate::dev::install(&ctx);
    });
    app.connect_shutdown(|_| {
        if let Some(ctx) = crate::ctx::try_ctx() {
            ctx.backend().close();
        }
    });
    let handler = Rc::new(handler);
    let h = handler.clone();
    let help = spec.help();
    app.connect_command_line(move |_app, cl| {
        let args: Vec<String> = cl.arguments().iter().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
        if args.iter().any(|a| a == "--help" || a == "-h") {
            cl.print_literal(&help);
            return glib::ExitCode::SUCCESS;
        }
        let Some(ctx) = crate::ctx::try_ctx() else { return glib::ExitCode::FAILURE };
        let first = !STARTED.with(|s| s.replace(true));
        h(&ctx, args, first);
        glib::ExitCode::SUCCESS
    });
    let h = handler.clone();
    app.connect_activate(move |_| {
        let Some(ctx) = crate::ctx::try_ctx() else { return };
        let first = !STARTED.with(|s| s.replace(true));
        h(&ctx, Vec::new(), first);
    });
    app.run()
}
