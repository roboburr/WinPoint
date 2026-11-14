//! The context every page gets (main.js `ctx`): the store, the window, the shell's info (user, version, test
//! mode), the search query, navigation to another page, "Nieuwe taak", and helpers for toasts, dialogs, settings
//! and events whose listeners go away with the page.
use crate::hostcall;
use crate::store::Store;
use kit::controls::{Severity, Toast};
use kit::frame::AppWindow;
use kit::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

type Navigator = Rc<dyn Fn(&str, Value)>;
type SettingsFn = Rc<dyn Fn(&Value, Option<&str>)>;

pub struct App {
    pub store: Store,
    pub window: AppWindow,
    info: RefCell<Value>,
    query: RefCell<String>,
    navigator: RefCell<Option<Navigator>>,
    settings_subs: RefCell<Vec<(u64, SettingsFn)>>,
    next_id: Cell<u64>,
}

/// A settings listener of `App::on_settings`; dropping it removes it.
pub struct SettingsSub {
    app: Weak<App>,
    id: u64,
}

impl Drop for SettingsSub {
    fn drop(&mut self) {
        if let Some(app) = self.app.upgrade()
            && let Ok(mut subs) = app.settings_subs.try_borrow_mut()
        {
            subs.retain(|(i, _)| *i != self.id);
        }
    }
}

/// An event listener of `App::on`; dropping it removes it.
pub struct EventSub(Option<kit::ListenerId>);

impl Drop for EventSub {
    fn drop(&mut self) {
        if let (Some(id), Some(ctx)) = (self.0.take(), kit::try_ctx()) {
            ctx.off(id);
        }
    }
}

/// A health listener of `App::on_health`; dropping it removes it.
pub struct HealthSub(u64);

impl Drop for HealthSub {
    fn drop(&mut self) {
        hostcall::health().off(self.0);
    }
}

impl App {
    pub fn new(store: Store, window: AppWindow) -> Rc<App> {
        let app = Rc::new(App {
            store,
            window,
            info: RefCell::new(json!({})),
            query: RefCell::new(String::new()),
            navigator: RefCell::new(None),
            settings_subs: RefCell::new(Vec::new()),
            next_id: Cell::new(1),
        });
        let weak = Rc::downgrade(&app);
        kit::ctx().on_settings(move |s, path| {
            let Some(app) = weak.upgrade() else { return };
            let subs: Vec<SettingsFn> = app.settings_subs.borrow().iter().map(|(_, f)| f.clone()).collect();
            for f in subs {
                f(s, path);
            }
        });
        app
    }

    /// A widget of the window (parent for dialogs and toasts).
    pub fn parent(&self) -> gtk::Widget {
        self.window.window().clone().upcast()
    }

    // ------------------------------------------------------------------------------------------ shell info

    /// shell.info as fetched at startup ({version, dryRun, user: {name}, ...}).
    pub fn info(&self) -> Value {
        self.info.borrow().clone()
    }

    pub(crate) fn set_info(&self, info: Value) {
        *self.info.borrow_mut() = info;
        kit::ctx().emit("taskmgr.info", &self.info());
    }

    /// The user's login name ("" while unknown).
    pub fn user(&self) -> String {
        let info = self.info.borrow();
        info["user"]["name"].as_str().or_else(|| info["user"].as_str()).unwrap_or("").to_owned()
    }

    /// The host runs with --dry-run (actions are not carried out).
    pub fn dry_run(&self) -> bool {
        self.info.borrow()["dryRun"] == true
    }

    // ------------------------------------------------------------------------------------------- navigation

    /// The search box's text.
    pub fn query(&self) -> String {
        self.query.borrow().clone()
    }

    pub(crate) fn set_query(&self, q: &str) {
        *self.query.borrow_mut() = q.to_owned();
    }

    pub(crate) fn set_navigator(&self, f: impl Fn(&str, Value) + 'static) {
        *self.navigator.borrow_mut() = Some(Rc::new(f));
    }

    /// Go to a page with deep-link params ({pid}, {device}, {volume}, {tab}, ...), after the current event.
    pub fn navigate(&self, id: &str, params: Value) {
        let Some(f) = self.navigator.borrow().clone() else { return };
        let id = id.to_owned();
        glib::idle_add_local_once(move || f(&id, params));
    }

    /// "Nieuwe taak": open the shell's search to find and start an app.
    pub fn new_task(self: &Rc<Self>) {
        let me = self.clone();
        hostcall::call("shell.command", json!({ "command": "toggle-search", "args": {} }), move |r| {
            if let Err(e) = r {
                me.toast_error(kit::t("Zoeken kon niet worden geopend", "Couldn\u{2019}t open search"), &e.to_string());
            }
        });
    }

    // ----------------------------------------------------------------------------------------- feedback

    /// A toast in the window.
    pub fn toast(&self, toast: Toast) -> kit::controls::ToastHandle {
        toast.show(self.window.window())
    }

    /// An error toast with a title and the error text.
    pub fn toast_error(&self, title: &str, message: &str) {
        self.toast(Toast::new(title).message(message).severity(Severity::Error));
    }

    /// A success toast that goes after `ms`.
    pub fn toast_success(&self, title: &str, ms: u32) {
        self.toast(Toast::new(title).severity(Severity::Success).timeout(ms));
    }

    /// An info toast (test mode, ...).
    pub fn toast_info(&self, title: &str, message: &str) {
        self.toast(Toast::new(title).message(message).severity(Severity::Info));
    }

    /// Dialog.alert with the error disc.
    pub fn alert_error(&self, title: &str, message: &str, done: impl FnOnce() + 'static) {
        kit::controls::Dialog::new(title).text(message).icon("error").close_label("OK").show(self.window.window(), move |_| done());
    }

    // -------------------------------------------------------------------------------------- subscriptions

    /// A setting by dotted path (Null when absent).
    pub fn setting(&self, path: &str) -> Value {
        kit::ctx().setting(path)
    }

    /// Change a setting (optimistic); on failure a toast "Kon de instelling niet opslaan" and `revert()`.
    pub fn set_setting(self: &Rc<Self>, path: &str, value: Value, revert: impl FnOnce() + 'static) {
        let me = self.clone();
        kit::ctx().set_setting_then(path, value, move |r| {
            if let Err(e) = r {
                me.toast_error(kit::t("Kon de instelling niet opslaan", "Couldn\u{2019}t save the setting"), &e.to_string());
                revert();
            }
        });
    }

    /// Run `f(settings, path)` after every settings change while the returned handle lives.
    pub fn on_settings(self: &Rc<Self>, f: impl Fn(&Value, Option<&str>) + 'static) -> SettingsSub {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        self.settings_subs.borrow_mut().push((id, Rc::new(f)));
        SettingsSub { app: Rc::downgrade(self), id }
    }

    /// Listen to backend / extension events (kit `Ctx::on`) while the returned handle lives.
    pub fn on(&self, prefix: &str, f: impl Fn(&str, &Value) + 'static) -> EventSub {
        EventSub(Some(kit::ctx().on(prefix, f)))
    }

    /// Run `f(state, info)` on host health changes while the returned handle lives.
    pub fn on_health(&self, f: impl Fn(hostcall::State, &hostcall::Info) + 'static) -> HealthSub {
        HealthSub(hostcall::health().on_change(f))
    }
}
