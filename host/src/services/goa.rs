//! `goa.*` (batch E): online accounts through GNOME Online Accounts, as GNOME Settings' Online Accounts panel
//! (notes/claude/settings-parity.md §5.6).
//!
//! * `goa.list`: the accounts of goa-daemon's ObjectManager (session bus `org.gnome.OnlineAccounts`,
//!   `/org/gnome/OnlineAccounts`), each with the services it can be used for. A service is offered when its
//!   interface is on the account object (enabled) or its `<Service>Disabled` property is true (switched off): goa-daemon
//!   exports a service's interface only while it is enabled, and only for the features its provider has.
//! * `goa.providers`: the providers GNOME Settings 50 offers (GOA 3.58's built-ins).
//! * `goa.add {provider}`: starts `win11-gtk-settings --goa-add <provider>` (native/apps/settings/src/goa_helper.rs):
//!   GOA's own add dialog (libgoa-backend) in a process of its own, so it never loads into a WinPoint process.
//! * `goa.setService {id, service, on}`: the account's `<Service>Disabled` property (goa-daemon writes accounts.conf).
//! * `goa.signIn {id}`: EnsureCredentials; when that fails, the helper's sign-in dialog (`--goa-refresh <id>`).
//! * `goa.remove {id}`: the account's Remove.
//! * Event `goa.changed {}` after every write and when goa-daemon reports a change (ObjectManager, PropertiesChanged).
use super::ServiceContext;
use super::system::common;
use crate::files::{ApiError, ApiResult};
use crate::i18n::{Lang, NO_PARAMS};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// The methods (name, dangerous), exactly as host/src/api-methods/accounts.json lists them (a test compares).
#[cfg_attr(not(test), allow(dead_code))] // the dispatch tests compare it with the api-methods files
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("goa.add", true),
    ("goa.list", false),
    ("goa.providers", false),
    ("goa.remove", true),
    ("goa.setService", true),
    ("goa.signIn", true),
];

pub(crate) const NAME: &str = "org.gnome.OnlineAccounts";
pub(crate) const ROOT: &str = "/org/gnome/OnlineAccounts";
pub(crate) const ACCOUNT_IFACE: &str = "org.gnome.OnlineAccounts.Account";
const TIMEOUT_MS: i32 = 10_000;

/// GOA's services: (API id, the Account property that switches it off, the interface goa-daemon exports while on).
pub(crate) const SERVICES: &[(&str, &str, &str)] = &[
    ("mail", "MailDisabled", "org.gnome.OnlineAccounts.Mail"),
    (
        "calendar",
        "CalendarDisabled",
        "org.gnome.OnlineAccounts.Calendar",
    ),
    (
        "contacts",
        "ContactsDisabled",
        "org.gnome.OnlineAccounts.Contacts",
    ),
    ("files", "FilesDisabled", "org.gnome.OnlineAccounts.Files"),
    (
        "photos",
        "PhotosDisabled",
        "org.gnome.OnlineAccounts.Photos",
    ),
    (
        "documents",
        "DocumentsDisabled",
        "org.gnome.OnlineAccounts.Documents",
    ),
    (
        "printers",
        "PrintersDisabled",
        "org.gnome.OnlineAccounts.Printers",
    ),
    ("maps", "MapsDisabled", "org.gnome.OnlineAccounts.Maps"),
    ("music", "MusicDisabled", "org.gnome.OnlineAccounts.Music"),
    ("todo", "TodoDisabled", "org.gnome.OnlineAccounts.Todo"),
    ("chat", "ChatDisabled", "org.gnome.OnlineAccounts.Chat"),
    (
        "ticketing",
        "TicketingDisabled",
        "org.gnome.OnlineAccounts.Ticketing",
    ),
    (
        "readLater",
        "ReadLaterDisabled",
        "org.gnome.OnlineAccounts.ReadLater",
    ),
];

/// A provider's name: a brand name that reads the same in every language, or a catalogue key (GOA translates it).
#[derive(Clone, Copy, Debug)]
pub(crate) enum ProviderName {
    Brand(&'static str),
    Key(&'static str),
}

/// The providers GNOME Settings 50 offers to add (plan.json email-accounts), GOA 3.58's types and their names.
pub(crate) const PROVIDERS: &[(&str, ProviderName)] = &[
    ("google", ProviderName::Brand("Google")),
    ("ms_graph", ProviderName::Brand("Microsoft 365")),
    ("exchange", ProviderName::Brand("Microsoft Exchange")),
    ("owncloud", ProviderName::Brand("Nextcloud")),
    ("imap_smtp", ProviderName::Key("goa.provider.imapSmtp")),
    ("webdav", ProviderName::Brand("WebDAV")),
    ("kerberos", ProviderName::Key("goa.provider.kerberos")),
];

/// libgoa-backend, which the helper loads (looked for as a file: loading it here would bring GTK into this process).
const BACKEND: &str = "libgoa-backend-1.0.so.2";

fn backend_installed() -> bool {
    let arch = std::env::consts::ARCH;
    [
        format!("/usr/lib/{arch}-linux-gnu"),
        "/usr/lib64".into(),
        "/usr/lib".into(),
        format!("/lib/{arch}-linux-gnu"),
    ]
    .iter()
    .any(|dir| std::path::Path::new(dir).join(BACKEND).is_file())
}

/// win11-gtk-settings next to this host (scripts/install.py installs them together).
fn helper_binary() -> Option<PathBuf> {
    let bin = crate::exec_path(std::env::current_exe().ok()?).with_file_name("win11-gtk-settings");
    bin.is_file().then_some(bin)
}

/// An account id as goa-daemon makes them (`account_<time>_<n>`).
pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The accounts of a GetManagedObjects answer (as JSON: {path: {interface: {property: value}}}), sorted by name.
pub(crate) fn accounts_of(objects: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = objects
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(path, ifaces)| {
            let a = ifaces.get(ACCOUNT_IFACE)?;
            let text = |k: &str| a[k].as_str().unwrap_or("").to_owned();
            let services: Vec<Value> = SERVICES
                .iter()
                .filter_map(|(id, prop, iface)| {
                    let disabled = a[*prop] == true;
                    (ifaces.get(*iface).is_some() || disabled).then(|| json!({"id": id, "on": !disabled}))
                })
                .collect();
            Some(json!({
                "id": text("Id"),
                "path": path,
                "provider": text("ProviderType"),
                "providerName": text("ProviderName"),
                "providerIcon": text("ProviderIcon"),
                "identity": text("Identity"),
                "presentationIdentity": if text("PresentationIdentity").is_empty() { text("Identity") } else { text("PresentationIdentity") },
                "attentionNeeded": a["AttentionNeeded"] == true,
                "locked": a["IsLocked"] == true,
                "temporary": a["IsTemporary"] == true,
                "services": services,
            }))
        })
        .collect();
    out.sort_by(|a, b| {
        let key = |v: &Value| {
            (
                v["providerName"].as_str().unwrap_or("").to_lowercase(),
                v["presentationIdentity"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase(),
            )
        };
        key(a).cmp(&key(b))
    });
    out
}

/// goa-daemon on a bus: the connection and the name to call (the well-known name, or a test fake's).
#[derive(Clone)]
pub(crate) struct Daemon {
    pub bus: gio::DBusConnection,
    pub name: String,
}

impl Daemon {
    fn session() -> Option<Daemon> {
        Some(Daemon {
            bus: common::session().ok()?,
            name: NAME.into(),
        })
    }

    fn objects(&self) -> Result<Value, String> {
        let v = common::call(
            &self.bus,
            &self.name,
            ROOT,
            "org.freedesktop.DBus.ObjectManager",
            "GetManagedObjects",
            None,
            TIMEOUT_MS,
        )?;
        Ok(common::first(&v))
    }
}

pub(crate) struct Goa {
    ctx: Arc<ServiceContext>,
    daemon: Option<Daemon>,
    /// Tests: what goa.add and goa.signIn start instead of win11-gtk-settings (None: the helper next to this host).
    helper: Option<PathBuf>,
    watcher: Mutex<Option<Watcher>>,
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "core.badArgs", NO_PARAMS)
}

impl Goa {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        Ok(Arc::new(Self {
            ctx,
            daemon: None,
            helper: None,
            watcher: Mutex::new(None),
        }))
    }

    #[cfg(test)]
    pub(crate) fn with(
        ctx: Arc<ServiceContext>,
        daemon: Daemon,
        helper: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            daemon: Some(daemon),
            helper,
            watcher: Mutex::new(None),
        })
    }

    fn daemon(&self) -> Option<Daemon> {
        self.daemon.clone().or_else(Daemon::session)
    }

    fn unavailable(lang: Lang) -> ApiError {
        ApiError::tr("unsupported", lang, "goa.unavailable", NO_PARAMS)
    }

    fn watch(&self) {
        let mut slot = self.watcher.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_none()
            && let Some(d) = self.daemon()
        {
            *slot = Watcher::start(self.ctx.clone(), d);
        }
    }

    fn changed(&self) {
        self.ctx.emit("goa.changed", json!({}));
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        if !METHODS.iter().any(|(m, _)| *m == method) {
            return None;
        }
        let lang = self.ctx.lang();
        self.watch();
        Some(match method {
            "goa.list" => self.list(),
            "goa.providers" => Ok(self.providers(lang)),
            "goa.add" => self.add(args, lang),
            "goa.setService" => self.set_service(args, lang),
            "goa.signIn" => self.sign_in(args, lang),
            "goa.remove" => self.remove(args, lang),
            _ => return None,
        })
    }

    fn helper(&self) -> Option<PathBuf> {
        match &self.helper {
            Some(p) => Some(p.clone()),
            None => backend_installed().then(helper_binary).flatten(),
        }
    }

    fn list(&self) -> ApiResult {
        let objects = self.daemon().map(|d| d.objects());
        Ok(match objects {
            Some(Ok(objects)) => {
                json!({"available": true, "canAdd": self.helper().is_some(), "accounts": accounts_of(&objects)})
            }
            _ => json!({"available": false, "canAdd": false, "accounts": []}),
        })
    }

    fn providers(&self, lang: Lang) -> Value {
        let list: Vec<Value> = PROVIDERS
            .iter()
            .map(|(id, name)| {
                let name = match name {
                    ProviderName::Brand(brand) => (*brand).to_owned(),
                    ProviderName::Key(key) => crate::i18n::tr(lang, key),
                };
                json!({"type": id, "name": name})
            })
            .collect();
        json!({"providers": list, "canAdd": self.helper().is_some()})
    }

    /// The account with `id` (its object path and Account properties).
    fn account(&self, args: &Value, lang: Lang) -> Result<(Daemon, Value), ApiError> {
        let id = args["id"]
            .as_str()
            .filter(|id| valid_id(id))
            .ok_or_else(|| bad(lang))?;
        let daemon = self.daemon().ok_or_else(|| Self::unavailable(lang))?;
        let objects = daemon.objects().map_err(|_| Self::unavailable(lang))?;
        let account = accounts_of(&objects)
            .into_iter()
            .find(|a| a["id"] == id)
            .ok_or_else(|| ApiError::tr("not_found", lang, "goa.noSuchAccount", NO_PARAMS))?;
        Ok((daemon, account))
    }

    /// Start the helper with these arguments; it runs on its own (the dialog is GOA's), and goa-daemon's signals
    /// bring the new account.
    fn spawn_helper(&self, args: &[&str], lang: Lang) -> ApiResult {
        let bin = self
            .helper()
            .ok_or_else(|| ApiError::tr("unsupported", lang, "goa.noHelper", NO_PARAMS))?;
        let mut child = std::process::Command::new(&bin)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|_| ApiError::tr("failed", lang, "goa.helperFailed", NO_PARAMS))?;
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = child.wait();
            ctx.emit("goa.changed", json!({}));
        });
        Ok(json!({"ok": true, "helper": true}))
    }

    fn add(&self, args: &Value, lang: Lang) -> ApiResult {
        let provider = args["provider"].as_str().ok_or_else(|| bad(lang))?;
        if !PROVIDERS.iter().any(|(id, _)| *id == provider) {
            return Err(ApiError::tr("bad_args", lang, "goa.badProvider", NO_PARAMS));
        }
        if self.ctx.dry_run {
            return Ok(json!({"ok": true, "dryRun": true}));
        }
        self.spawn_helper(&["--goa-add", provider], lang)
    }

    fn writable(account: &Value, lang: Lang) -> Result<(), ApiError> {
        if account["locked"] == true {
            return Err(ApiError::tr("denied", lang, "goa.locked", NO_PARAMS));
        }
        Ok(())
    }

    fn set_service(&self, args: &Value, lang: Lang) -> ApiResult {
        let service = args["service"].as_str().ok_or_else(|| bad(lang))?;
        let on = args["on"].as_bool().ok_or_else(|| bad(lang))?;
        let (_, prop, _) = SERVICES
            .iter()
            .find(|(id, _, _)| *id == service)
            .ok_or_else(|| bad(lang))?;
        let (daemon, account) = self.account(args, lang)?;
        if !account["services"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|s| s["id"] == service)
        {
            return Err(ApiError::tr("bad_args", lang, "goa.badService", NO_PARAMS));
        }
        Self::writable(&account, lang)?;
        if self.ctx.dry_run {
            return Ok(json!({"ok": true, "dryRun": true}));
        }
        let path = account["path"].as_str().unwrap_or("");
        let value = (ACCOUNT_IFACE, *prop, (!on).to_variant()).to_variant();
        common::call_interactive(
            &daemon.bus,
            &daemon.name,
            path,
            "org.freedesktop.DBus.Properties",
            "Set",
            Some(&value),
            lang,
        )?;
        self.changed();
        Ok(json!({"ok": true, "id": account["id"], "service": service, "on": on}))
    }

    fn sign_in(&self, args: &Value, lang: Lang) -> ApiResult {
        let (daemon, account) = self.account(args, lang)?;
        if self.ctx.dry_run {
            return Ok(json!({"ok": true, "dryRun": true}));
        }
        let path = account["path"].as_str().unwrap_or("");
        // valid credentials (a token refreshed by goa-daemon) need no dialog
        if common::call(
            &daemon.bus,
            &daemon.name,
            path,
            ACCOUNT_IFACE,
            "EnsureCredentials",
            None,
            TIMEOUT_MS,
        )
        .is_ok()
        {
            self.changed();
            return Ok(json!({"ok": true, "signedIn": true}));
        }
        let id = account["id"].as_str().unwrap_or("").to_owned();
        self.spawn_helper(&["--goa-refresh", &id], lang)
    }

    fn remove(&self, args: &Value, lang: Lang) -> ApiResult {
        let (daemon, account) = self.account(args, lang)?;
        Self::writable(&account, lang)?;
        if self.ctx.dry_run {
            return Ok(json!({"ok": true, "dryRun": true}));
        }
        let path = account["path"].as_str().unwrap_or("");
        common::call_interactive(
            &daemon.bus,
            &daemon.name,
            path,
            ACCOUNT_IFACE,
            "Remove",
            None,
            lang,
        )?;
        self.changed();
        Ok(json!({"ok": true}))
    }

    pub(crate) fn shutdown(&self) {
        if let Some(w) = self
            .watcher
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
        {
            w.stop();
        }
    }
}

/// The thread that follows goa-daemon (InterfacesAdded/Removed, PropertiesChanged): one debounced `goa.changed`.
struct Watcher {
    context: glib::MainContext,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    fn start(ctx: Arc<ServiceContext>, daemon: Daemon) -> Option<Watcher> {
        let context = glib::MainContext::new();
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_context, thread_stop) = (context.clone(), stop.clone());
        let (ready, subscribed) = std::sync::mpsc::channel::<()>();
        std::thread::Builder::new()
            .name("win11-goa".into())
            .spawn(move || {
                let _ = thread_context.with_thread_default(|| {
                    let pending = Arc::new(AtomicBool::new(false));
                    let id = daemon.bus.signal_subscribe(
                        Some(&daemon.name),
                        None,
                        None,
                        None,
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, _, _| {
                            let ctx = ctx.clone();
                            super::users::debounce(
                                &pending,
                                std::time::Duration::from_millis(250),
                                move || ctx.emit("goa.changed", json!({})),
                            );
                        },
                    );
                    let _ = ready.send(());
                    while !thread_stop.load(Ordering::Acquire) {
                        thread_context.iteration(true);
                    }
                    daemon.bus.signal_unsubscribe(id);
                });
            })
            .ok()?;
        let _ = subscribed.recv_timeout(std::time::Duration::from_secs(2));
        Some(Watcher { context, stop })
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.context.wakeup();
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::users::tests::{FakeBus, Methods, Props, context, private_address};

    #[test]
    fn accounts_offer_the_services_their_provider_has() {
        // GetManagedObjects as JSON: a Google account with mail on and calendar switched off, an IMAP account
        let objects = json!({
            "/org/gnome/OnlineAccounts/Accounts/account_1_0": {
                ACCOUNT_IFACE: {"Id": "account_1_0", "ProviderType": "google", "ProviderName": "Google",
                    "Identity": "anna@example.org", "PresentationIdentity": "anna@example.org", "AttentionNeeded": true,
                    "MailDisabled": false, "CalendarDisabled": true, "ContactsDisabled": false, "FilesDisabled": false},
                "org.gnome.OnlineAccounts.Mail": {"EmailAddress": "anna@example.org"},
                "org.gnome.OnlineAccounts.Contacts": {},
            },
            "/org/gnome/OnlineAccounts/Accounts/account_2_0": {
                ACCOUNT_IFACE: {"Id": "account_2_0", "ProviderType": "imap_smtp", "ProviderName": "IMAP and SMTP",
                    "Identity": "bob", "PresentationIdentity": "", "MailDisabled": false, "IsLocked": true},
                "org.gnome.OnlineAccounts.Mail": {},
            },
            "/org/gnome/OnlineAccounts/Manager": {"org.gnome.OnlineAccounts.Manager": {}},
        });
        let list = accounts_of(&objects);
        assert_eq!(list.len(), 2, "the Manager object is no account");
        assert_eq!(list[0]["id"], "account_1_0", "sorted by provider name");
        assert_eq!(list[0]["attentionNeeded"], true);
        assert_eq!(
            list[0]["services"],
            json!([{"id": "mail", "on": true}, {"id": "calendar", "on": false}, {"id": "contacts", "on": true}]),
            "files: no interface and not switched off, so the provider has no such feature"
        );
        assert_eq!(
            list[1]["presentationIdentity"], "bob",
            "the identity when there is no presentation identity"
        );
        assert_eq!(list[1]["locked"], true);
        assert_eq!(list[1]["services"], json!([{"id": "mail", "on": true}]));
    }

    #[test]
    fn ids_and_providers_are_checked() {
        assert!(valid_id("account_1700000000_0"));
        for bad in ["", "../x", "a b", "account/1", &"x".repeat(129)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
        let types: Vec<&str> = PROVIDERS.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            types,
            [
                "google",
                "ms_graph",
                "exchange",
                "owncloud",
                "imap_smtp",
                "webdav",
                "kerberos"
            ]
        );
        // the service properties are the ones of GOA's Account interface (goa.xml), each with its interface
        for (id, prop, iface) in SERVICES {
            assert!(
                prop.ends_with("Disabled") && iface.starts_with("org.gnome.OnlineAccounts."),
                "{id}"
            );
        }
    }

    const XML: &str = r#"<node>
        <interface name="org.freedesktop.DBus.ObjectManager">
          <method name="GetManagedObjects"><arg type="a{oa{sa{sv}}}" direction="out"/></method>
        </interface>
        <interface name="org.gnome.OnlineAccounts.Account">
          <method name="Remove"/><method name="EnsureCredentials"><arg type="i" direction="out"/></method>
          <property name="MailDisabled" type="b" access="readwrite"/>
          <property name="CalendarDisabled" type="b" access="readwrite"/>
        </interface></node>"#;
    const ACCOUNT: &str = "/org/gnome/OnlineAccounts/Accounts/account_1_0";

    fn fake_goa() -> Option<FakeBus> {
        let methods: Methods = Arc::new(|_, _, method, _| {
            match method {
            "GetManagedObjects" => Ok(Some(
                glib::Variant::parse(
                    Some(glib::VariantTy::new("(a{oa{sa{sv}}})").unwrap()),
                    "({objectpath '/org/gnome/OnlineAccounts/Accounts/account_1_0': {'org.gnome.OnlineAccounts.Account': {'Id': <'account_1_0'>, 'ProviderType': <'google'>, 'ProviderName': <'Google'>, 'PresentationIdentity': <'anna@example.org'>, 'MailDisabled': <false>, 'CalendarDisabled': <true>}, 'org.gnome.OnlineAccounts.Mail': @a{sv} {}}},)",
                )
                .unwrap(),
            )),
            "EnsureCredentials" => Ok(Some((3600i32,).to_variant())),
            _ => Ok(None),
        }
        });
        let props: Props = Arc::new(|_, _, _| Some(false.to_variant()));
        FakeBus::start(
            XML,
            vec![
                (ROOT.to_owned(), "org.freedesktop.DBus.ObjectManager"),
                (ACCOUNT.to_owned(), ACCOUNT_IFACE),
            ],
            methods,
            props,
        )
    }

    #[test]
    fn goa_calls_go_to_the_account_and_a_dry_run_changes_nothing() {
        let Some(fake) = fake_goa() else {
            eprintln!(
                "skipped: no private session bus (run through tests/rust-sandbox/run-tests.sh)"
            );
            return;
        };
        let bus = common::session().unwrap();
        let (_lang, ctx) = context(false);
        let goa = Goa::with(
            ctx,
            Daemon {
                bus: bus.clone(),
                name: fake.name.clone(),
            },
            None,
        );
        let list = goa.call("goa.list", &json!({})).unwrap().unwrap();
        assert_eq!(list["available"], true);
        assert_eq!(
            list["accounts"][0]["services"],
            json!([{"id": "mail", "on": true}, {"id": "calendar", "on": false}])
        );
        let set = goa
            .call(
                "goa.setService",
                &json!({"id": "account_1_0", "service": "calendar", "on": true}),
            )
            .unwrap()
            .unwrap();
        assert_eq!(set["on"], true);
        let refused = goa
            .call(
                "goa.setService",
                &json!({"id": "account_1_0", "service": "files", "on": true}),
            )
            .unwrap()
            .unwrap_err();
        assert_eq!(
            refused.key.as_deref(),
            Some("goa.badService"),
            "a feature the provider has not"
        );
        let gone = goa
            .call("goa.remove", &json!({"id": "account_9_9"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(gone.key.as_deref(), Some("goa.noSuchAccount"));
        goa.call("goa.remove", &json!({"id": "account_1_0"}))
            .unwrap()
            .unwrap();
        assert_eq!(
            goa.call("goa.signIn", &json!({"id": "account_1_0"}))
                .unwrap()
                .unwrap()["signedIn"],
            true
        );
        let provider = goa
            .call("goa.add", &json!({"provider": "facebook"}))
            .unwrap()
            .unwrap_err();
        assert_eq!(provider.key.as_deref(), Some("goa.badProvider"));
        assert_eq!(
            fake.writes(&["GetManagedObjects"]),
            vec![
                ("Set".into(), "CalendarDisabled = false".into(), false),
                ("Remove".into(), "()".into(), true),
                ("EnsureCredentials".into(), "()".into(), false),
            ]
        );
        goa.shutdown();
        // under --dry-run the same calls only read
        let (_lang, dry) = context(true);
        let goa = Goa::with(
            dry,
            Daemon {
                bus,
                name: fake.name.clone(),
            },
            None,
        );
        fake.clear();
        for (method, args) in [
            (
                "goa.setService",
                json!({"id": "account_1_0", "service": "mail", "on": false}),
            ),
            ("goa.remove", json!({"id": "account_1_0"})),
            ("goa.signIn", json!({"id": "account_1_0"})),
            ("goa.add", json!({"provider": "google"})),
        ] {
            assert_eq!(
                goa.call(method, &args).unwrap().unwrap()["dryRun"],
                true,
                "{method}"
            );
        }
        assert!(
            fake.writes(&["GetManagedObjects"]).is_empty(),
            "{:?}",
            fake.calls()
        );
        let providers = goa.call("goa.providers", &json!({})).unwrap().unwrap();
        assert_eq!(
            providers["providers"][4],
            json!({"type": "imap_smtp", "name": "IMAP and SMTP"})
        );
        goa.shutdown();
    }

    #[test]
    fn without_goa_daemon_the_list_says_so() {
        let (_lang, ctx) = context(false);
        let goa = Goa::new(ctx).unwrap();
        // the sandbox's private session bus has no goa-daemon to activate
        if private_address().is_some() {
            let list = goa.call("goa.list", &json!({})).unwrap().unwrap();
            assert_eq!(
                (
                    list["available"].as_bool(),
                    list["accounts"].as_array().map(Vec::len)
                ),
                (Some(false), Some(0))
            );
            let err = goa
                .call("goa.remove", &json!({"id": "account_1_0"}))
                .unwrap()
                .unwrap_err();
            assert_eq!(err.key.as_deref(), Some("goa.unavailable"));
        }
        assert!(goa.call("goa.nonsense", &json!({})).is_none());
        assert_eq!(
            goa.call("goa.remove", &json!({"id": "../x"}))
                .unwrap()
                .unwrap_err()
                .code,
            "bad_args"
        );
        goa.shutdown();
    }

    #[test]
    #[ignore = "tests/dbus-fakes/run.sh --batch e: goa-daemon's fake on a private session bus"]
    fn fakes_goa_on_the_session_bus() {
        let (_lang, ctx) = context(false);
        let goa = Goa::new(ctx).unwrap();
        let list = goa.call("goa.list", &json!({})).unwrap().unwrap();
        assert_eq!(
            list["accounts"][0]["services"],
            json!([{"id": "mail", "on": true}, {"id": "calendar", "on": false}])
        );
        goa.call(
            "goa.setService",
            &json!({"id": "account_1_0", "service": "calendar", "on": true}),
        )
        .unwrap()
        .unwrap();
        let list = goa.call("goa.list", &json!({})).unwrap().unwrap();
        assert_eq!(
            list["accounts"][0]["services"][1],
            json!({"id": "calendar", "on": true}),
            "goa-daemon's own state"
        );
        goa.call("goa.remove", &json!({"id": "account_1_0"}))
            .unwrap()
            .unwrap();
        let writes: Vec<(String, String, bool)> =
            crate::services::users::tests::fake_calls("WIN11_FAKE_CALLS_GOA")
                .into_iter()
                .filter(|c| c.0 != "GetManagedObjects")
                .collect();
        assert_eq!(
            writes,
            vec![
                (
                    "Set".to_owned(),
                    "CalendarDisabled = false".to_owned(),
                    false
                ),
                ("Remove".into(), "()".into(), true)
            ]
        );
        goa.shutdown();
    }
}
