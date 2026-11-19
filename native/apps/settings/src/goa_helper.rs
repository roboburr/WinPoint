//! `win11-gtk-settings --goa-add <provider>` and `--goa-refresh <account id>` (batch E, notes/claude/settings-parity.md
//! §5.6): GNOME Online Accounts' own "add account" and "sign in again" dialogs in a process of their own, so
//! libgoa-backend (and the libadwaita it pulls in) never loads into the Settings window. The host's `goa.add` and
//! `goa.signIn` start this mode.
//!
//! As GNOME Settings' Online Accounts panel (cc-online-accounts-panel.c): libgoa-1.0's `goa_client_new_sync`, then
//! libgoa-backend's `goa_provider_get_for_provider_type` and `goa_provider_add_account` (or, for an account that
//! needs attention, `goa_provider_refresh_account`), both asynchronous, with no parent widget, so GOA shows its
//! dialog as a window of its own. Both libraries are opened with dlopen; GOA's own OAuth flow runs in the browser
//! (goa-oauth2-handler). The process ends when the dialog closes: 0 added or signed in, 1 cancelled or failed, 2
//! wrong arguments or GOA not installed. goa-daemon tells the host about the new account (goa.changed).
use gtk::glib;
use gtk::glib::translate::FromGlibPtrFull;
use libc::{c_char, c_void};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString};

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Task {
    /// Add an account of this GOA provider type.
    Add(String),
    /// Sign in to this account again.
    Refresh(String),
}

/// A GOA provider type or account id: ASCII letters, digits, `_` and `-`, at most 128.
fn valid_word(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The task of a helper argument (`requested`'s value): a provider, or `refresh:<account id>`.
pub fn task(spec: &str) -> Option<Task> {
    match spec.strip_prefix("refresh:") {
        Some(id) => valid_word(id, 128).then(|| Task::Refresh(id.to_owned())),
        None => valid_word(spec, 64).then(|| Task::Add(spec.to_owned())),
    }
}

/// The helper argument of a `--goa-add` / `--goa-refresh` command line, if the line asks for the helper (checked
/// before the GApplication starts): the provider, or `refresh:<id>`.
pub fn requested(args: &[String]) -> Option<String> {
    let at = args.iter().position(|a| a == "--goa-add" || a.starts_with("--goa-add=") || a == "--goa-refresh" || a.starts_with("--goa-refresh="))?;
    let (flag, inline) = match args[at].split_once('=') {
        Some((f, v)) => (f, Some(v.to_owned())),
        None => (args[at].as_str(), None),
    };
    let value = inline.unwrap_or_else(|| args.get(at + 1).cloned().unwrap_or_default());
    Some(if flag == "--goa-refresh" { format!("refresh:{value}") } else { value })
}

// ------------------------------------------------------------------------------------------------- the libraries

type Callback = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

#[allow(non_snake_case)]
struct Goa {
    goa_client_new_sync: unsafe extern "C" fn(*mut c_void, *mut *mut glib::ffi::GError) -> *mut c_void,
    goa_client_lookup_by_id: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    goa_object_peek_account: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    goa_account_get_provider_type: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    goa_provider_get_for_provider_type: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    goa_provider_add_account: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, Option<Callback>, *mut c_void),
    goa_provider_add_account_finish: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut glib::ffi::GError) -> *mut c_void,
    goa_provider_refresh_account: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void, Option<Callback>, *mut c_void),
    goa_provider_refresh_account_finish: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut *mut glib::ffi::GError) -> glib::ffi::gboolean,
    adw_init: Option<unsafe extern "C" fn()>,
}

/// The symbol `name` of `handle`.
unsafe fn sym(handle: *mut c_void, name: &CStr) -> Option<*mut c_void> {
    let p = unsafe { libc::dlsym(handle, name.as_ptr()) };
    (!p.is_null()).then_some(p)
}

/// libgoa-1.0 and libgoa-backend-1.0 (GOA 3.50's asynchronous provider API, with the parent widget); None when
/// either is missing or lacks a symbol.
fn load() -> Option<Goa> {
    unsafe {
        let goa = libc::dlopen(c"libgoa-1.0.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        let backend = libc::dlopen(c"libgoa-backend-1.0.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        if goa.is_null() || backend.is_null() {
            return None;
        }
        // libgoa-backend links libadwaita: the same library, already loaded
        let adw = libc::dlopen(c"libadwaita-1.so.0".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL);
        macro_rules! get {
            ($h:expr, $name:literal) => {
                std::mem::transmute::<*mut c_void, _>(sym($h, $name)?)
            };
        }
        Some(Goa {
            goa_client_new_sync: get!(goa, c"goa_client_new_sync"),
            goa_client_lookup_by_id: get!(goa, c"goa_client_lookup_by_id"),
            goa_object_peek_account: get!(goa, c"goa_object_peek_account"),
            goa_account_get_provider_type: get!(goa, c"goa_account_get_provider_type"),
            goa_provider_get_for_provider_type: get!(backend, c"goa_provider_get_for_provider_type"),
            goa_provider_add_account: get!(backend, c"goa_provider_add_account"),
            goa_provider_add_account_finish: get!(backend, c"goa_provider_add_account_finish"),
            goa_provider_refresh_account: get!(backend, c"goa_provider_refresh_account"),
            goa_provider_refresh_account_finish: get!(backend, c"goa_provider_refresh_account_finish"),
            adw_init: if adw.is_null() { None } else { sym(adw, c"adw_init").map(|p| std::mem::transmute::<*mut c_void, unsafe extern "C" fn()>(p)) },
        })
    }
}

// --------------------------------------------------------------------------------------------------- the dialog

thread_local! {
    static STATE: RefCell<Option<(Goa, glib::MainLoop)>> = const { RefCell::new(None) };
    static EXIT: Cell<i32> = const { Cell::new(1) };
}

/// The GError of a finish call: 1 (cancelled quietly, else the message on stderr).
unsafe fn failed(error: *mut glib::ffi::GError, what: &str) -> i32 {
    if !error.is_null() {
        let e: glib::Error = unsafe { glib::Error::from_glib_full(error) };
        if !e.matches(gtk::gio::IOErrorEnum::Cancelled) {
            eprintln!("win11-gtk-settings: {what}: {}", e.message());
        }
    }
    1
}

unsafe extern "C" fn added(source: *mut c_void, result: *mut c_void, _: *mut c_void) {
    STATE.with(|s| {
        let s = s.borrow();
        let Some((goa, main)) = s.as_ref() else { return };
        let mut error = std::ptr::null_mut();
        let object = unsafe { (goa.goa_provider_add_account_finish)(source, result, &mut error) };
        let code = if object.is_null() {
            unsafe { failed(error, "add account") }
        } else {
            unsafe { glib::gobject_ffi::g_object_unref(object.cast()) };
            0
        };
        EXIT.with(|e| e.set(code));
        main.quit();
    });
}

unsafe extern "C" fn refreshed(source: *mut c_void, result: *mut c_void, _: *mut c_void) {
    STATE.with(|s| {
        let s = s.borrow();
        let Some((goa, main)) = s.as_ref() else { return };
        let mut error = std::ptr::null_mut();
        let ok = unsafe { (goa.goa_provider_refresh_account_finish)(source, result, &mut error) };
        let code = if ok != 0 { 0 } else { unsafe { failed(error, "sign in") } };
        EXIT.with(|e| e.set(code));
        main.quit();
    });
}

/// Settings' application id (main.rs SPEC), which the account dialog's window takes.
const APP_ID: &str = "io.github.win11shell.Settings";

/// Run the helper for a `requested` value (a provider type, or `refresh:<account id>`); the process's exit code.
pub fn run(spec: &str) -> i32 {
    let Some(task) = task(spec) else {
        eprintln!("win11-gtk-settings --goa-add: a provider type is required (google, ms_graph, exchange, owncloud, imap_smtp, webdav, kerberos)");
        return 2;
    };
    let Some(goa) = load() else {
        eprintln!("win11-gtk-settings: GNOME Online Accounts (libgoa-backend-1.0.so.2) is not installed");
        return 2;
    };
    // GOA's dialog is Settings' (GNOME Settings shows it in its own process): with Settings' Wayland app id the shell
    // matches io.github.win11shell.Settings.desktop, its icon and its name, not "GTK Application" (the GNOME
    // harness's taskbar and "is ready" banner)
    glib::set_prgname(Some(APP_ID));
    if gtk::init().is_err() {
        eprintln!("win11-gtk-settings: no display for the account dialog");
        return 2;
    }
    if let Some(init) = goa.adw_init {
        unsafe { init() };
    }
    let mut error = std::ptr::null_mut();
    let client = unsafe { (goa.goa_client_new_sync)(std::ptr::null_mut(), &mut error) };
    if client.is_null() {
        unsafe { failed(error, "GNOME Online Accounts") };
        return 2;
    }
    let mut object = std::ptr::null_mut();
    let provider_type = match &task {
        Task::Add(provider) => provider.clone(),
        Task::Refresh(id) => {
            let id = CString::new(id.as_str()).unwrap_or_default();
            object = unsafe { (goa.goa_client_lookup_by_id)(client, id.as_ptr()) };
            let account = if object.is_null() { std::ptr::null_mut() } else { unsafe { (goa.goa_object_peek_account)(object) } };
            let kind = if account.is_null() { std::ptr::null() } else { unsafe { (goa.goa_account_get_provider_type)(account) } };
            if kind.is_null() {
                eprintln!("win11-gtk-settings: no such online account");
                unsafe { glib::gobject_ffi::g_object_unref(client.cast()) };
                return 2;
            }
            unsafe { CStr::from_ptr(kind) }.to_string_lossy().into_owned()
        }
    };
    let c_type = CString::new(provider_type).unwrap_or_default();
    let provider = unsafe { (goa.goa_provider_get_for_provider_type)(c_type.as_ptr()) };
    if provider.is_null() {
        eprintln!("win11-gtk-settings: GNOME Online Accounts has no such provider");
        unsafe { glib::gobject_ffi::g_object_unref(client.cast()) };
        return 2;
    }
    let main = glib::MainLoop::new(None, false);
    let refresh = matches!(task, Task::Refresh(_));
    STATE.with(|s| *s.borrow_mut() = Some((goa, main.clone())));
    STATE.with(|s| {
        let s = s.borrow();
        let (goa, _) = s.as_ref().expect("set above");
        unsafe {
            if refresh {
                (goa.goa_provider_refresh_account)(provider, client, object, std::ptr::null_mut(), std::ptr::null_mut(), Some(refreshed), std::ptr::null_mut());
            } else {
                (goa.goa_provider_add_account)(provider, client, std::ptr::null_mut(), std::ptr::null_mut(), Some(added), std::ptr::null_mut());
            }
        }
    });
    main.run();
    unsafe {
        if !object.is_null() {
            glib::gobject_ffi::g_object_unref(object.cast());
        }
        glib::gobject_ffi::g_object_unref(provider.cast());
        glib::gobject_ffi::g_object_unref(client.cast());
    }
    EXIT.with(|e| e.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_helper_reads_its_task_from_the_command_line() {
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(requested(&args(&["--goa-refresh", "account_17_0"])), Some("refresh:account_17_0".into()));
        assert_eq!(requested(&args(&["--goa-refresh=account_17_0"])), Some("refresh:account_17_0".into()));
        assert_eq!(task("google"), Some(Task::Add("google".into())));
        assert_eq!(task("refresh:account_17_0"), Some(Task::Refresh("account_17_0".into())));
        for bad in ["", "goo gle", "../x", "refresh:", "refresh:a/b", &"x".repeat(65)] {
            assert_eq!(task(bad), None, "{bad:?}");
        }
        assert_eq!(run("refresh:"), 2, "no account id");
    }

    #[test]
    fn the_goa_libraries_have_every_symbol_when_installed() {
        // GOA 3.50's API (the parent widget, asynchronous add and refresh): a GOA without it is "not installed"
        if std::path::Path::new("/usr/lib/x86_64-linux-gnu/libgoa-backend-1.0.so.2").exists() {
            assert!(load().is_some());
        }
    }
}
