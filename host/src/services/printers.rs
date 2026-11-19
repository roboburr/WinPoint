//! `printers.*` (batch B, notes/claude/settings-parity.md §5.3): GNOME Settings' Printers panel for Windows 11's
//! "Printers en scanners" and its printer page. Reads and the user's own jobs go to the scheduler through libcups
//! (dlopen, printers/cups.rs: the queues, their PPD options, jobs, drivers, the test page and the head cleaning);
//! every admin write goes through cups-pk-helper (printers/helper.rs), which checks polkit itself (§5.8), with GNOME
//! Shell's password dialog (`call_interactive`). Finding printers is printers/discover.rs.
//!
//! A driverless (IPP Everywhere) queue is made as cupsd 2.4 makes one for `lpadmin -m everywhere`: PrinterAdd with the
//! PPD name `everywhere`, which the scheduler answers by asking the printer for its attributes and writing the PPD
//! itself; the new queue starts stopped and rejecting jobs, so `printers.add` then enables it and accepts jobs, as
//! g-c-c's pp-new-printer.c does, and sets the locale's paper (A4) as its default (amendment 41; the experiment is in
//! notes/claude/settings-b.md).
//!
//! Every dangerous method checks its arguments first and under `ctx.dry_run` answers `{ok, dryRun}` without calling
//! anything. A watcher thread (own MainContext, asleep in it while nothing happens) holds a CUPS notification
//! subscription as g-c-c's printers panel does (notify-recipient-uri `dbus://`, a lease of 600 s renewed every 500 s,
//! so the subscription of a backend that was killed ends within 10 minutes) and turns the scheduler's
//! org.cups.cupsd.Notifier signals into `printers.changed {}` (the first signal of a burst arms a one-shot 250 ms
//! source); every write emits it too. `shutdown` cancels the subscription itself, at once.
mod cups;
mod discover;
mod helper;
mod ipp;
mod ppd;

use super::ServiceContext;
use super::system::common::{session, system};
use crate::files::{ApiError, ApiResult};
use crate::i18n::Lang;
use cups::{Conn, Server};
use glib::variant::ToVariant;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

/// g-c-c's cc-printers-panel.c: SUBSCRIPTION_DURATION and RENEW_INTERVAL.
const SUBSCRIPTION_LEASE: i32 = 600;
const RENEW_EVERY: Duration = Duration::from_secs(500);
/// How long the watcher gathers the scheduler's signals before it emits `printers.changed` once.
const EMIT_DELAY: Duration = Duration::from_millis(250);

/// The methods (name, dangerous), exactly as host/src/api-methods/devices.json lists them (a test compares).
pub(crate) const METHODS: &[(&str, bool)] = &[
    ("printers.add", true),
    ("printers.cleanHeads", true),
    ("printers.discover", false),
    ("printers.drivers", false),
    ("printers.get", false),
    ("printers.job.authenticate", true),
    ("printers.job.cancel", true),
    ("printers.job.hold", true),
    ("printers.job.priority", true),
    ("printers.job.release", true),
    ("printers.jobs", false),
    ("printers.jobs.clear", true),
    ("printers.list", false),
    ("printers.remove", true),
    ("printers.rename", true),
    ("printers.setAcceptJobs", true),
    ("printers.setDefault", true),
    ("printers.setEnabled", true),
    ("printers.setLocation", true),
    ("printers.setOption", true),
    ("printers.testPage", true),
];

/// GNOME's test page (cups-filters' PDF banner) and the job that cleans the heads (pp-maintenance-command.c).
const TEST_PAGE: &str = "/usr/share/cups/data/testprint";
const CLEAN_COMMAND: &[u8] = b"#CUPS-COMMAND\nClean all\n";

/// Where the daemons are: the system bus, cups-pk-helper's and Avahi's names and libcups's own scheduler; the tests
/// put a fake on the private session bus and an IPP responder on loopback.
#[derive(Clone)]
pub(crate) struct Endpoints {
    pub system_bus: bool,
    pub helper: String,
    pub avahi: String,
    pub server: Server,
    /// `$HOME/.cups/lpoptions`, the user's own default printer (cupsGetDests reads it first).
    pub lpoptions: Option<PathBuf>,
    /// GNOME's test page.
    pub test_page: PathBuf,
}

impl Endpoints {
    fn real() -> Self {
        Self {
            system_bus: true,
            helper: "org.opensuse.CupsPkHelper.Mechanism".into(),
            avahi: "org.freedesktop.Avahi".into(),
            server: Server::default(),
            lpoptions: std::env::var_os("HOME").map(|h| Path::new(&h).join(".cups/lpoptions")),
            test_page: PathBuf::from(TEST_PAGE),
        }
    }

    fn bus(&self) -> Result<gio::DBusConnection, String> {
        if self.system_bus { system() } else { session() }
    }
}

fn bad(lang: Lang) -> ApiError {
    ApiError::tr("bad_args", lang, "printers.badArgs", crate::i18n::NO_PARAMS)
}

fn not_found(lang: Lang) -> ApiError {
    ApiError::tr(
        "not_found",
        lang,
        "printers.notFound",
        crate::i18n::NO_PARAMS,
    )
}

/// A text argument: at most `max` bytes, no control characters (the names, places and URIs CUPS stores).
fn text_arg(v: &Value, max: usize, lang: Lang) -> Result<String, ApiError> {
    match v {
        Value::Null => Ok(String::new()),
        Value::String(s) if s.len() <= max && !s.chars().any(char::is_control) => Ok(s.clone()),
        _ => Err(bad(lang)),
    }
}

/// A queue name as CUPS takes one (scheduler/ipp.c validate_name, GNOME's "no space, tab, # or /"): 1-127 bytes,
/// printable, none of space, tab, '/', '\\', '?', '\'', '"' or '#'.
pub(crate) fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 128
        && !name.chars().any(|c| {
            c.is_control() || c.is_whitespace() || matches!(c, '/' | '\\' | '?' | '\'' | '"' | '#')
        })
}

fn name_arg(v: &Value, lang: Lang) -> Result<String, ApiError> {
    match v.as_str() {
        Some(n) if name_ok(n) => Ok(n.to_owned()),
        Some(_) => Err(ApiError::tr(
            "bad_args",
            lang,
            "printers.badName",
            crate::i18n::NO_PARAMS,
        )),
        None => Err(bad(lang)),
    }
}

/// A device URI: a scheme and something after it, no spaces or control characters, at most 1024 bytes.
pub(crate) fn uri_ok(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    !scheme.is_empty()
        && scheme.len() <= 32
        && scheme
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase())
        && scheme
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '+' | '.' | '-'))
        && !rest.is_empty()
        && uri.len() <= 1024
        && !uri.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// A PPD name of CUPS-Get-PPDs ("everywhere", "drv:///sample.drv/generic.ppd", "lsb/usr/...").
pub(crate) fn ppd_name_ok(ppd: &str) -> bool {
    !ppd.is_empty()
        && ppd.len() <= 1024
        && !ppd.contains("..")
        && !ppd.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// A PPD file for PrinterAddWithPpdFile: a regular file this user can open that starts as a PPD ("*PPD-Adobe") or
/// is gzipped (GNOME's chooser takes *.ppd and *.ppd.gz).
pub(crate) fn ppd_file_ok(path: &Path) -> bool {
    use std::io::Read;
    if !path.is_file() {
        return false;
    }
    let mut head = [0u8; 10];
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let n = file.read(&mut head).unwrap_or(0);
    head[..n].starts_with(b"*PPD-Adobe") || head[..n].starts_with(&[0x1f, 0x8b])
}

/// An option keyword (a PPD keyword or an IPP attribute name) and its values.
pub(crate) fn option_ok(option: &str, values: &[String]) -> bool {
    !option.is_empty()
        && option.len() <= 64
        && option
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && !option.ends_with("-default")
        && !values.is_empty()
        && values.len() <= 32
        && values
            .iter()
            .all(|v| !v.is_empty() && v.len() <= 255 && !v.chars().any(char::is_control))
}

fn values_arg(v: &Value) -> Vec<String> {
    match v {
        Value::String(s) => vec![s.clone()],
        Value::Number(n) => vec![n.to_string()],
        Value::Bool(b) => vec![if *b { "True" } else { "False" }.into()],
        Value::Array(a) => a
            .iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn id_arg(v: &Value, lang: Lang) -> Result<i32, ApiError> {
    v.as_i64()
        .filter(|n| (1..=i32::MAX as i64).contains(n))
        .map(|n| n as i32)
        .ok_or_else(|| bad(lang))
}

fn on_arg(v: &Value, lang: Lang) -> Result<bool, ApiError> {
    v.as_bool().ok_or_else(|| bad(lang))
}

/// The answer when there is no scheduler to ask (GNOME's "No Print Service").
fn unavailable(reason: &str) -> Value {
    json!({"available": false, "reason": reason, "printers": [], "default": null})
}

fn ipp_error(e: &cups::IppError, lang: Lang) -> ApiError {
    if e.status == -1 {
        return ApiError::tr(
            "unsupported",
            lang,
            "printers.noLibcups",
            crate::i18n::NO_PARAMS,
        );
    }
    if e.unreachable() {
        return ApiError::tr(
            "unavailable",
            lang,
            "printers.noCups",
            crate::i18n::NO_PARAMS,
        );
    }
    match e.status {
        cups::STATUS_NOT_FOUND => not_found(lang),
        cups::STATUS_FORBIDDEN | cups::STATUS_NOT_AUTHENTICATED => {
            ApiError::tr("denied", lang, "core.notAuthorized", crate::i18n::NO_PARAMS)
        }
        _ => helper::cups_refused(&e.message, lang),
    }
}

/// The user's default printer of `~/.cups/lpoptions` ("Default name" or "Default name/instance").
pub(crate) fn lpoptions_default(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.trim_start().strip_prefix("Default ")?;
        let dest = rest.split_whitespace().next()?;
        Some(dest.split('/').next().unwrap_or(dest).to_owned())
    })
}

/// `~/.cups/lpoptions` without the user's own default (g-c-c's set_local_default_printer(NULL), which
/// printer_set_default does before PrinterSetDefault on this computer's scheduler): the Default line becomes a Dest
/// line with its options, no Default line is added. None when the file names no user default (nothing to write).
pub(crate) fn lpoptions_without_default(text: &str) -> Option<String> {
    lpoptions_default(text)?;
    let out: Vec<String> = text
        .lines()
        .map(|line| match line.trim_start().strip_prefix("Default ") {
            Some(rest) => format!("Dest {rest}"),
            None => line.to_owned(),
        })
        .collect();
    Some(out.join("\n") + "\n")
}

/// `~/.cups/lpoptions` with `name` as the user's default (g-c-c's set_local_default_printer(name), which
/// printer_set_default does instead of PrinterSetDefault when the scheduler is another computer's): the old
/// Default line becomes a Dest line, the queue's Dest line (or a new line) its Default.
pub(crate) fn lpoptions_with_default(text: &str, name: &str) -> String {
    let mut out = Vec::new();
    let mut placed = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("Default ") {
            let dest = rest.split_whitespace().next().unwrap_or("");
            if dest == name {
                out.push(line.to_owned());
                placed = true;
            } else {
                out.push(format!("Dest {rest}"));
            }
        } else if let Some(rest) = trimmed.strip_prefix("Dest ") {
            let dest = rest.split_whitespace().next().unwrap_or("");
            if dest == name && !placed {
                out.push(format!("Default {rest}"));
                placed = true;
            } else {
                out.push(line.to_owned());
            }
        } else {
            out.push(line.to_owned());
        }
    }
    if !placed {
        out.push(format!("Default {name}"));
    }
    out.join("\n") + "\n"
}

/// `printers.jobs.clear`'s answer, as pp-jobs-dialog.c cancels every job on its own: `{ok, canceled, failed}` when
/// one was cancelled (or there was none), the first error when every cancel failed.
pub(crate) fn clear_answer(results: Vec<Result<(), ApiError>>) -> ApiResult {
    let total = results.len();
    let mut first = None;
    let mut canceled = 0;
    for result in results {
        match result {
            Ok(()) => canceled += 1,
            Err(error) => {
                first.get_or_insert(error);
            }
        }
    }
    match first {
        Some(error) if canceled == 0 => Err(error),
        _ => Ok(json!({"ok": true, "canceled": canceled, "failed": total - canceled})),
    }
}

pub(crate) struct Printers {
    ctx: Arc<ServiceContext>,
    ends: Endpoints,
    stopped: Arc<AtomicBool>,
    context: Mutex<Option<glib::MainContext>>,
    /// The CUPS notification subscription's id (the watcher renews it, `shutdown` cancels it).
    subscription: Arc<Mutex<Option<i32>>>,
    /// How often the watcher renews it ([`RENEW_EVERY`]; shorter in the tests).
    renew_every: Duration,
}

impl Printers {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let printers = Self::with(ctx, Endpoints::real());
        printers.watch();
        Ok(printers)
    }

    fn with(ctx: Arc<ServiceContext>, ends: Endpoints) -> Arc<Self> {
        Self::with_renewal(ctx, ends, RENEW_EVERY)
    }

    fn with_renewal(ctx: Arc<ServiceContext>, ends: Endpoints, renew_every: Duration) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            ends,
            stopped: Arc::new(AtomicBool::new(false)),
            context: Mutex::new(None),
            subscription: Arc::default(),
            renew_every,
        })
    }

    fn changed(&self) {
        self.ctx.emit("printers.changed", json!({}));
    }

    /// Check a method's arguments (also in a dry run, so the page hears about a bad one).
    fn check(method: &str, args: &Value, lang: Lang) -> Result<(), ApiError> {
        match method {
            "printers.get"
            | "printers.testPage"
            | "printers.cleanHeads"
            | "printers.setDefault"
            | "printers.remove"
            | "printers.jobs.clear" => name_arg(&args["name"], lang).map(drop),
            "printers.jobs" => {
                if !args["name"].is_null() {
                    name_arg(&args["name"], lang)?;
                }
                match &args["which"] {
                    Value::Null => Ok(()),
                    Value::String(w)
                        if matches!(w.as_str(), "not-completed" | "completed" | "all") =>
                    {
                        Ok(())
                    }
                    _ => Err(bad(lang)),
                }
            }
            "printers.add" => {
                name_arg(&args["name"], lang)?;
                if !uri_ok(args["uri"].as_str().unwrap_or("")) {
                    return Err(bad(lang));
                }
                let ppd = text_arg(&args["ppd"], 1024, lang)?;
                let file = text_arg(&args["ppdFile"], 4096, lang)?;
                if (!ppd.is_empty() && !ppd_name_ok(&ppd)) || (!ppd.is_empty() && !file.is_empty())
                {
                    return Err(bad(lang));
                }
                // the mechanism reads the file as root: only a PPD (or a gzipped one) the user can read
                if !file.is_empty() && !(file.starts_with('/') && ppd_file_ok(Path::new(&file))) {
                    return Err(ApiError::tr(
                        "bad_args",
                        lang,
                        "printers.badFile",
                        crate::i18n::NO_PARAMS,
                    ));
                }
                text_arg(&args["info"], 1024, lang)?;
                text_arg(&args["location"], 1024, lang).map(drop)
            }
            "printers.rename" => {
                name_arg(&args["name"], lang)?;
                name_arg(&args["newName"], lang).map(drop)
            }
            "printers.setLocation" => {
                name_arg(&args["name"], lang)?;
                if !args["location"].is_string() {
                    return Err(bad(lang));
                }
                text_arg(&args["location"], 1024, lang).map(drop)
            }
            "printers.setEnabled" | "printers.setAcceptJobs" => {
                name_arg(&args["name"], lang)?;
                on_arg(&args["on"], lang).map(drop)
            }
            "printers.setOption" => {
                name_arg(&args["name"], lang)?;
                let option = args["option"].as_str().unwrap_or("");
                if option_ok(option, &values_arg(&args["values"])) {
                    Ok(())
                } else {
                    Err(bad(lang))
                }
            }
            "printers.job.cancel" | "printers.job.hold" | "printers.job.release" => {
                id_arg(&args["id"], lang).map(drop)
            }
            "printers.job.priority" => {
                id_arg(&args["id"], lang)?;
                match args["priority"].as_i64() {
                    Some(1..=100) => Ok(()),
                    _ => Err(bad(lang)),
                }
            }
            "printers.job.authenticate" => {
                id_arg(&args["id"], lang)?;
                text_arg(&args["username"], 255, lang)?;
                text_arg(&args["domain"], 255, lang)?;
                if !args["password"].is_string() {
                    return Err(bad(lang));
                }
                text_arg(&args["password"], 1024, lang).map(drop)
            }
            "printers.discover" => {
                match &args["address"] {
                    Value::Null => {}
                    Value::String(a) if discover::target(a).is_some() => {}
                    Value::String(_) => {
                        return Err(ApiError::tr(
                            "bad_args",
                            lang,
                            "printers.badAddress",
                            crate::i18n::NO_PARAMS,
                        ));
                    }
                    _ => return Err(bad(lang)),
                }
                text_arg(&args["username"], 255, lang)?;
                text_arg(&args["password"], 1024, lang).map(drop)
            }
            "printers.drivers" => {
                text_arg(&args["deviceId"], 1024, lang)?;
                text_arg(&args["makeModel"], 1024, lang).map(drop)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let dangerous = METHODS.iter().find(|(name, _)| *name == method)?.1;
        let lang = self.ctx.lang();
        if let Err(error) = Self::check(method, args, lang) {
            return Some(Err(error));
        }
        if dangerous && self.ctx.dry_run {
            return Some(Ok(json!({"ok": true, "dryRun": true})));
        }
        Some(self.run(method, args, lang))
    }

    fn conn(&self, lang: Lang) -> Result<Conn, ApiError> {
        Conn::open(&self.ends.server, None, 10_000).map_err(|e| ipp_error(&e, lang))
    }

    fn run(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let name = args["name"].as_str().unwrap_or("").to_owned();
        match method {
            "printers.list" => Ok(self.list()),
            "printers.get" => self.get(&name, lang),
            "printers.jobs" => self.jobs(args["name"].as_str(), args["which"].as_str(), lang),
            "printers.drivers" => self.drivers(args, lang),
            "printers.discover" => self.discover(args, lang),
            "printers.testPage" => {
                let file = std::fs::File::open(&self.ends.test_page).map_err(|_| {
                    ApiError::tr(
                        "unsupported",
                        lang,
                        "printers.noTestPage",
                        crate::i18n::NO_PARAMS,
                    )
                })?;
                let title = crate::i18n::tr(lang, "printers.testPageTitle");
                let job = self.print(&name, &title, None, &file, lang)?;
                self.changed();
                Ok(json!({"ok": true, "job": job}))
            }
            "printers.cleanHeads" => {
                let conn = self.conn(lang)?;
                let attrs = self.attributes(&conn, &name, &["printer-commands"], lang)?;
                if !ipp::can_clean(&attrs) {
                    return Err(ApiError::tr(
                        "unsupported",
                        lang,
                        "printers.noCleaning",
                        crate::i18n::NO_PARAMS,
                    ));
                }
                drop(conn);
                let file = memfile(CLEAN_COMMAND).map_err(|_| bad(lang))?;
                let title = crate::i18n::tr(lang, "printers.cleanHeadsTitle");
                let job = self.print(
                    &name,
                    &title,
                    Some("application/vnd.cups-command"),
                    &file,
                    lang,
                )?;
                self.changed();
                Ok(json!({"ok": true, "job": job}))
            }
            "printers.job.priority" => {
                let id = id_arg(&args["id"], lang)?;
                let priority = args["priority"].as_i64().unwrap_or(50) as i32;
                let conn = self.conn(lang)?;
                let mut req = conn.request(cups::SET_JOB_ATTRIBUTES);
                req.string(
                    cups::OPERATION,
                    cups::URI,
                    "job-uri",
                    &format!("ipp://localhost/jobs/{id}"),
                );
                req.integer(cups::JOB, cups::INTEGER, "job-priority", priority);
                conn.send(req, "/jobs/").map_err(|e| job_error(&e, lang))?;
                self.changed();
                Ok(json!({"ok": true}))
            }
            "printers.job.authenticate" => self.authenticate(args, lang),
            _ => self.admin(method, args, lang),
        }
    }

    /// The writes through cups-pk-helper.
    fn admin(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let bus = self.ends.bus().map_err(|_| {
            ApiError::tr(
                "unsupported",
                lang,
                "printers.noHelper",
                crate::i18n::NO_PARAMS,
            )
        })?;
        let helper = self.ends.helper.as_str();
        let name = args["name"].as_str().unwrap_or("");
        let value = match method {
            "printers.add" => self.add(&bus, args, lang)?,
            "printers.rename" => {
                let new = args["newName"].as_str().unwrap_or("");
                helper::ok(
                    &bus,
                    helper,
                    "PrinterRename",
                    (name, new).to_variant(),
                    lang,
                )?;
                json!({"ok": true, "name": new})
            }
            "printers.setLocation" => {
                let location = args["location"].as_str().unwrap_or("");
                helper::ok(
                    &bus,
                    helper,
                    "PrinterSetLocation",
                    (name, location).to_variant(),
                    lang,
                )?
            }
            "printers.setDefault" => {
                // g-c-c's printer_set_default (pp-utils.c): on this computer's scheduler the user's own default
                // goes first (it would hide the new one) and the scheduler's default is set; with another
                // computer's scheduler only the user's default changes
                let local = self.ends.server.host().is_none_or(|h| cups::is_local(&h));
                let path = self.ends.lpoptions.as_deref();
                let text = path.and_then(|p| std::fs::read_to_string(p).ok());
                if local {
                    if let (Some(path), Some(new)) =
                        (path, text.as_deref().and_then(lpoptions_without_default))
                    {
                        let _ = write_atomic(path, &new);
                    }
                    helper::ok(
                        &bus,
                        helper,
                        "PrinterSetDefault",
                        (name,).to_variant(),
                        lang,
                    )?;
                } else if let Some(path) = path {
                    let new = lpoptions_with_default(text.as_deref().unwrap_or(""), name);
                    write_lpoptions(path, &new).map_err(|_| bad(lang))?;
                }
                json!({"ok": true})
            }
            "printers.setEnabled" => helper::ok(
                &bus,
                helper,
                "PrinterSetEnabled",
                (name, args["on"] == true).to_variant(),
                lang,
            )?,
            "printers.setAcceptJobs" => helper::ok(
                &bus,
                helper,
                "PrinterSetAcceptJobs",
                (name, args["on"] == true, "").to_variant(),
                lang,
            )?,
            "printers.remove" => {
                helper::ok(&bus, helper, "PrinterDelete", (name,).to_variant(), lang)?
            }
            "printers.setOption" => {
                let option = args["option"].as_str().unwrap_or("");
                let values = values_arg(&args["values"]);
                helper::ok(
                    &bus,
                    helper,
                    "PrinterAddOptionDefault",
                    (name, option, values).to_variant(),
                    lang,
                )?
            }
            "printers.job.cancel" => {
                let id = id_arg(&args["id"], lang)?;
                helper::ok(
                    &bus,
                    helper,
                    "JobCancelPurge",
                    (id, false).to_variant(),
                    lang,
                )
                .map_err(|e| job_refused(e, lang))?
            }
            "printers.job.hold" | "printers.job.release" => {
                let id = id_arg(&args["id"], lang)?;
                let until = if method == "printers.job.hold" {
                    "indefinite"
                } else {
                    "no-hold"
                };
                helper::ok(
                    &bus,
                    helper,
                    "JobSetHoldUntil",
                    (id, until).to_variant(),
                    lang,
                )
                .map_err(|e| job_refused(e, lang))?
            }
            "printers.jobs.clear" => {
                // every job on its own (pp-jobs-dialog.c): another user's job that needs the password does not
                // keep the user's own jobs in the queue
                let jobs = self.jobs(Some(name), Some("not-completed"), lang)?;
                let results = jobs["jobs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|job| job["id"].as_i64())
                    .map(|id| {
                        helper::ok(
                            &bus,
                            helper,
                            "JobCancelPurge",
                            (id as i32, false).to_variant(),
                            lang,
                        )
                        .map(drop)
                        .map_err(|e| job_refused(e, lang))
                    })
                    .collect();
                clear_answer(results)?
            }
            _ => return Err(bad(lang)),
        };
        self.changed();
        Ok(value)
    }

    /// `printers.add`: a new queue (or a new driver for one), as pp-new-printer.c: PrinterAdd (a PPD name, also
    /// `everywhere`) or PrinterAddWithPpdFile; a new queue is then enabled, accepts jobs and gets the locale's paper.
    fn add(&self, bus: &gio::DBusConnection, args: &Value, lang: Lang) -> ApiResult {
        let name = args["name"].as_str().unwrap_or("");
        let uri = args["uri"].as_str().unwrap_or("");
        let ppd = args["ppd"].as_str().unwrap_or("");
        let file = args["ppdFile"].as_str().unwrap_or("");
        let info = args["info"].as_str().unwrap_or("");
        let location = args["location"].as_str().unwrap_or("");
        let existed = self.conn(lang).ok().is_some_and(|conn| {
            self.attributes(&conn, name, &["printer-name"], lang)
                .is_ok()
        });
        let helper = self.ends.helper.as_str();
        if file.is_empty() {
            helper::ok(
                bus,
                helper,
                "PrinterAdd",
                (name, uri, ppd, info, location).to_variant(),
                lang,
            )?;
        } else {
            helper::ok(
                bus,
                helper,
                "PrinterAddWithPpdFile",
                (name, uri, file, info, location).to_variant(),
                lang,
            )?;
        }
        if !existed {
            helper::ok(
                bus,
                helper,
                "PrinterSetEnabled",
                (name, true).to_variant(),
                lang,
            )?;
            helper::ok(
                bus,
                helper,
                "PrinterSetAcceptJobs",
                (name, true, "").to_variant(),
                lang,
            )?;
            // g-c-c's printer_set_default_media_size: "a4" outside the letter countries (nl, en-GB)
            let _ = helper::ok(
                bus,
                helper,
                "PrinterAddOptionDefault",
                (name, "media", vec!["a4"]).to_variant(),
                lang,
            );
        }
        Ok(json!({"ok": true, "name": name, "new": !existed}))
    }

    /// One queue's attributes (Get-Printer-Attributes).
    fn attributes(
        &self,
        conn: &Conn,
        name: &str,
        wanted: &[&str],
        lang: Lang,
    ) -> Result<cups::Attrs, ApiError> {
        let mut req = conn.request(cups::GET_PRINTER_ATTRIBUTES);
        req.string(
            cups::OPERATION,
            cups::URI,
            "printer-uri",
            &cups::printer_uri(name),
        );
        req.strings(
            cups::OPERATION,
            cups::KEYWORD,
            "requested-attributes",
            wanted,
        );
        let answer = conn
            .send(req, &format!("/printers/{}", cups::uri_encode(name)))
            .map_err(|e| ipp_error(&e, lang))?;
        let attrs = answer.group(cups::PRINTER);
        if attrs.is_empty() {
            return Err(not_found(lang));
        }
        Ok(attrs)
    }

    fn default_printer(&self, conn: &Conn, names: &[String]) -> Option<String> {
        if let Some(path) = &self.ends.lpoptions
            && let Ok(text) = std::fs::read_to_string(path)
            && let Some(d) = lpoptions_default(&text)
            && names.contains(&d)
        {
            return Some(d);
        }
        let mut req = conn.request(cups::CUPS_GET_DEFAULT);
        req.strings(
            cups::OPERATION,
            cups::KEYWORD,
            "requested-attributes",
            &["printer-name"],
        );
        let answer = conn.send(req, "/").ok()?;
        Some(cups::text(&answer.group(cups::PRINTER), "printer-name")).filter(|n| !n.is_empty())
    }

    /// `printers.list`: the queues (CUPS-Get-Printers), sorted by their title.
    fn list(&self) -> Value {
        if cups::lib().is_none() {
            return unavailable("libcups");
        }
        let conn = match Conn::open(&self.ends.server, None, 5000) {
            Ok(c) => c,
            Err(_) => return unavailable("cupsd"),
        };
        let mut req = conn.request(cups::CUPS_GET_PRINTERS);
        req.strings(
            cups::OPERATION,
            cups::KEYWORD,
            "requested-attributes",
            ipp::PRINTER_ATTRS,
        );
        let groups = match conn.send(req, "/") {
            Ok(answer) => answer.groups(cups::PRINTER),
            // "No destinations added."
            Err(e) if e.status == cups::STATUS_NOT_FOUND => Vec::new(),
            Err(e) if e.unreachable() => return unavailable("cupsd"),
            Err(e) => {
                return json!({"available": true, "printers": [], "default": null, "error": e.message});
            }
        };
        let groups: Vec<_> = groups.into_iter().filter(ipp::listed).collect();
        let names: Vec<String> = groups
            .iter()
            .map(|a| cups::text(a, "printer-name"))
            .collect();
        let default = self.default_printer(&conn, &names);
        let mut printers: Vec<Value> = groups
            .iter()
            .map(|a| ipp::printer(a, default.as_deref()))
            .collect();
        printers.sort_by_key(|p| p["title"].as_str().unwrap_or("").to_lowercase());
        json!({"available": true, "printers": printers, "default": default})
    }

    /// `printers.get {name}`: the queue, its PPD's options (GNOME's Printing Options) and the IPP options g-c-c
    /// adds (pages per side, orientation, two-sided without a PPD Duplex).
    fn get(&self, name: &str, lang: Lang) -> ApiResult {
        let conn = self.conn(lang)?;
        let attrs = self.attributes(&conn, name, &["all"], lang)?;
        let default = self.default_printer(&conn, &[name.to_owned()]);
        let mut printer = ipp::printer(&attrs, default.as_deref());
        let groups = conn
            .ppd(name)
            .map(|text| ppd::parse(&text, lang.code()))
            .unwrap_or_default();
        let defaults =
            |kw: &str| Some(cups::text(&attrs, &format!("{kw}-default"))).filter(|v| !v.is_empty());
        let options = ppd::to_json(&groups, &defaults);
        let mut extra = Vec::new();
        let ints = cups::numbers(&attrs, "number-up-supported");
        if ints.len() > 1 {
            let current = cups::one(&attrs, "number-up-default").as_i64().unwrap_or(1);
            extra.push(json!({"name": "number-up", "values": ints, "value": current.to_string()}));
        }
        let orient = cups::numbers(&attrs, "orientation-requested-supported");
        if orient.len() > 1 {
            let current = cups::one(&attrs, "orientation-requested-default")
                .as_i64()
                .unwrap_or(3);
            extra.push(
                json!({"name": "orientation-requested", "values": orient, "value": current.to_string()}),
            );
        }
        let sides = cups::texts(&attrs, "sides-supported");
        if sides.len() > 1 && !ppd::has_option(&groups, "Duplex") {
            let current = Some(cups::text(&attrs, "sides-default"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "one-sided".into());
            extra.push(json!({"name": "sides", "values": sides, "value": current}));
        }
        printer["uri"] = json!(cups::text(&attrs, "printer-uri-supported"));
        printer["deviceId"] = json!(cups::text(&attrs, "printer-device-id"));
        Ok(json!({"printer": printer, "ppd": !groups.is_empty(), "options": options, "ipp": extra}))
    }

    /// `printers.jobs {name?, which?}`: Get-Jobs of one queue or all, every user's (GNOME's Jobs dialog).
    fn jobs(&self, name: Option<&str>, which: Option<&str>, lang: Lang) -> ApiResult {
        let conn = self.conn(lang)?;
        let mut req = conn.request(cups::GET_JOBS);
        let uri = name.map_or_else(|| "ipp://localhost/".to_owned(), cups::printer_uri);
        req.string(cups::OPERATION, cups::URI, "printer-uri", &uri);
        req.string(
            cups::OPERATION,
            cups::KEYWORD,
            "which-jobs",
            which.unwrap_or("not-completed"),
        );
        req.boolean(cups::OPERATION, "my-jobs", false);
        req.strings(
            cups::OPERATION,
            cups::KEYWORD,
            "requested-attributes",
            ipp::JOB_ATTRS,
        );
        let groups = match conn.send(req, "/") {
            Ok(answer) => answer.groups(cups::JOB),
            Err(e) if e.status == cups::STATUS_NOT_FOUND && name.is_none() => Vec::new(),
            Err(e) => return Err(ipp_error(&e, lang)),
        };
        let mut jobs: Vec<Value> = groups.iter().map(ipp::job).collect();
        jobs.sort_by_key(|j| (j["done"] == true, j["id"].as_i64().unwrap_or(0)));
        Ok(json!({"jobs": jobs}))
    }

    /// `printers.drivers {deviceId?, makeModel?, driverless?}`: CUPS's driver list (CUPS-Get-PPDs) and the best
    /// ones for the device.
    fn drivers(&self, args: &Value, lang: Lang) -> ApiResult {
        let conn = self.conn(lang)?;
        let mut req = conn.request(cups::CUPS_GET_PPDS);
        req.strings(
            cups::OPERATION,
            cups::KEYWORD,
            "requested-attributes",
            &[
                "ppd-name",
                "ppd-make",
                "ppd-make-and-model",
                "ppd-device-id",
                "ppd-natural-language",
                "ppd-product",
            ],
        );
        let answer = conn.send(req, "/").map_err(|e| ipp_error(&e, lang))?;
        let mut drivers: Vec<Value> = answer
            .groups(cups::PRINTER)
            .iter()
            .map(discover::driver)
            .filter(|d| d["name"].as_str().is_some_and(|n| !n.is_empty()))
            .collect();
        drivers.sort_by_key(|d| {
            (
                d["make"].as_str().unwrap_or("").to_lowercase(),
                d["makeModel"].as_str().unwrap_or("").to_lowercase(),
            )
        });
        let device_id = args["deviceId"].as_str().unwrap_or("");
        let make_model = args["makeModel"].as_str().unwrap_or("");
        let best = discover::best(&drivers, device_id, make_model, args["driverless"] == true);
        let everywhere = drivers.iter().any(|d| d["name"] == "everywhere");
        Ok(json!({"drivers": drivers, "best": best, "everywhere": everywhere}))
    }

    /// `printers.discover {address?, username?, password?}`: a typed address probed, or the network and the
    /// scheduler's backends (Avahi and DevicesGet side by side).
    fn discover(&self, args: &Value, lang: Lang) -> ApiResult {
        let credentials = args["password"]
            .as_str()
            .map(|p| (args["username"].as_str().unwrap_or(""), p));
        if let Some(address) = args["address"].as_str() {
            let target = discover::target(address).ok_or_else(|| {
                ApiError::tr(
                    "bad_args",
                    lang,
                    "printers.badAddress",
                    crate::i18n::NO_PARAMS,
                )
            })?;
            let probe = discover::probe(&target, credentials);
            return Ok(json!({"devices": probe.devices, "needsAuth": probe.needs_auth}));
        }
        let ends = self.ends.clone();
        let dnssd = std::thread::spawn(move || match ends.bus() {
            Ok(bus) => discover::avahi(&bus, &ends.avahi, Duration::from_millis(3000)),
            Err(_) => Vec::new(),
        });
        let mut denied = false;
        let mut helper_missing = false;
        let mut devices = match self.ends.bus() {
            Ok(bus) => match helper::devices(&bus, &self.ends.helper, 10, &["dnssd"], lang) {
                Ok(map) => discover::devices_get(&map),
                Err(e) => {
                    denied = e.code == "denied";
                    helper_missing = e.key.as_deref() == Some("printers.noHelper");
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        let network = dnssd.join().unwrap_or_default();
        for d in network {
            if !devices.iter().any(|e| e["uri"] == d["uri"]) {
                devices.push(d);
            }
        }
        Ok(json!({"devices": devices, "denied": denied, "helper": !helper_missing}))
    }

    /// Print a document (the test page, a maintenance command) as the user, as GNOME prints them.
    fn print(
        &self,
        name: &str,
        title: &str,
        format: Option<&str>,
        file: &std::fs::File,
        lang: Lang,
    ) -> Result<i64, ApiError> {
        use std::os::fd::AsRawFd;
        let conn = self.conn(lang)?;
        let mut req = conn.request(cups::PRINT_JOB);
        req.string(
            cups::OPERATION,
            cups::URI,
            "printer-uri",
            &cups::printer_uri(name),
        );
        req.string(cups::OPERATION, cups::NAME, "job-name", title);
        if let Some(format) = format {
            req.string(cups::OPERATION, cups::MIMETYPE, "document-format", format);
        }
        let answer = conn
            .send_document(
                req,
                &format!("/printers/{}", cups::uri_encode(name)),
                file.as_raw_fd(),
            )
            .map_err(|e| ipp_error(&e, lang))?;
        Ok(cups::one(&answer.group(cups::JOB), "job-id")
            .as_i64()
            .unwrap_or(0))
    }

    /// `printers.job.authenticate {id, username?, domain?, password}`: GNOME's "authenticate" of a job held for
    /// authentication (CUPS-Authenticate-Job with auth-info in the order of the queue's auth-info-required).
    fn authenticate(&self, args: &Value, lang: Lang) -> ApiResult {
        let id = id_arg(&args["id"], lang)?;
        let conn = self.conn(lang)?;
        let mut req = conn.request(cups::GET_JOBS);
        req.string(
            cups::OPERATION,
            cups::URI,
            "printer-uri",
            "ipp://localhost/",
        );
        req.string(
            cups::OPERATION,
            cups::KEYWORD,
            "which-jobs",
            "not-completed",
        );
        req.boolean(cups::OPERATION, "my-jobs", false);
        req.strings(
            cups::OPERATION,
            cups::KEYWORD,
            "requested-attributes",
            &["job-id", "job-printer-uri"],
        );
        let jobs = conn
            .send(req, "/")
            .map_err(|e| ipp_error(&e, lang))?
            .groups(cups::JOB);
        let queue = jobs
            .iter()
            .find(|j| cups::one(j, "job-id").as_i64() == Some(id as i64))
            .map(|j| ipp::queue_of(&cups::text(j, "job-printer-uri")))
            .ok_or_else(|| job_not_found(lang))?;
        let required = cups::texts(
            &self.attributes(&conn, &queue, &["auth-info-required"], lang)?,
            "auth-info-required",
        );
        let values: Vec<String> = required
            .iter()
            .filter(|r| *r != "none" && *r != "negotiate")
            .map(|r| match r.as_str() {
                "username" => args["username"].as_str().unwrap_or("").to_owned(),
                "domain" => args["domain"].as_str().unwrap_or("").to_owned(),
                _ => args["password"].as_str().unwrap_or("").to_owned(),
            })
            .collect();
        let mut req = conn.request(cups::CUPS_AUTHENTICATE_JOB);
        req.string(
            cups::OPERATION,
            cups::URI,
            "job-uri",
            &format!("ipp://localhost/jobs/{id}"),
        );
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        // sent only to the scheduler; the method is secret, so neither the arguments nor this is logged
        req.strings(cups::OPERATION, cups::TEXT, "auth-info", &refs);
        conn.send(req, "/jobs/").map_err(|e| job_error(&e, lang))?;
        self.changed();
        Ok(json!({"ok": true}))
    }

    /// The watcher (module docs): the subscription now and every `renew_every`, a one-shot emit per burst of the
    /// scheduler's signals; otherwise the thread sleeps in `iteration(true)` until `shutdown` wakes it.
    fn watch(self: &Arc<Self>) {
        let context = glib::MainContext::new();
        *self.context.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.clone());
        let weak = Arc::downgrade(self);
        let stopped = self.stopped.clone();
        let ends = self.ends.clone();
        let subscription = self.subscription.clone();
        let every = self.renew_every;
        std::thread::spawn(move || {
            let _ = context.with_thread_default(|| {
                let pending = Arc::new(AtomicBool::new(false));
                let bus = ends.bus().ok();
                let sub = bus.as_ref().map(|bus| {
                    let (pending, weak, context) = (pending.clone(), weak.clone(), context.clone());
                    bus.signal_subscribe(
                        None,
                        Some("org.cups.cupsd.Notifier"),
                        None,
                        None,
                        None,
                        gio::DBusSignalFlags::NONE,
                        move |_, _, _, _, _, _| {
                            if pending.swap(true, Ordering::AcqRel) {
                                return;
                            }
                            let (pending, weak) = (pending.clone(), weak.clone());
                            glib::timeout_source_new(
                                EMIT_DELAY,
                                None,
                                glib::Priority::DEFAULT,
                                move || {
                                    pending.store(false, Ordering::Release);
                                    if let Some(me) = weak.upgrade() {
                                        me.changed();
                                    }
                                    glib::ControlFlow::Break
                                },
                            )
                            .attach(Some(&context));
                        },
                    )
                });
                renew_held(&ends.server, &subscription, &stopped);
                let renewal = {
                    let (server, subscription, stopped) =
                        (ends.server.clone(), subscription.clone(), stopped.clone());
                    glib::timeout_source_new(every, None, glib::Priority::DEFAULT, move || {
                        renew_held(&server, &subscription, &stopped);
                        glib::ControlFlow::Continue
                    })
                };
                renewal.attach(Some(&context));
                while !stopped.load(Ordering::Acquire) && weak.strong_count() > 0 {
                    context.iteration(true);
                }
                renewal.destroy();
                if let (Some(bus), Some(sub)) = (bus, sub) {
                    bus.signal_unsubscribe(sub);
                }
                // the service went without a shutdown: the subscription goes with the thread
                let left = subscription
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some(id) = left {
                    cancel_subscription(&ends.server, id);
                }
            });
        });
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
        if let Some(context) = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            context.wakeup();
        }
        // as g-c-c's dispose: cancelled here, at once (the host may end before the watcher runs again); a renewal
        // under way finishes first and sees `stopped` afterwards
        let held = self
            .subscription
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(id) = held {
            cancel_subscription(&self.ends.server, id);
        }
    }
}

/// One renewal under the subscription's lock: none after `shutdown` (it took the id and cancelled it).
fn renew_held(server: &Server, subscription: &Mutex<Option<i32>>, stopped: &AtomicBool) {
    let mut id = subscription.lock().unwrap_or_else(|e| e.into_inner());
    if stopped.load(Ordering::Acquire) {
        return;
    }
    *id = renew(server, *id);
}

/// The scheduler's events Settings follows (g-c-c's printers panel subscribes to the same).
const EVENTS: &[&str] = &[
    "printer-added",
    "printer-deleted",
    "printer-stopped",
    "printer-state-changed",
    "printer-modified",
    "job-created",
    "job-completed",
    "job-state-changed",
];

/// Renew the notification subscription (or make one): its id, None when the scheduler is not there.
fn renew(server: &Server, id: Option<i32>) -> Option<i32> {
    cups::lib()?;
    let conn = Conn::open(server, None, 3000).ok()?;
    if let Some(id) = id {
        let mut req = conn.request(cups::RENEW_SUBSCRIPTION);
        req.string(
            cups::OPERATION,
            cups::URI,
            "printer-uri",
            "ipp://localhost/",
        );
        req.integer(cups::OPERATION, cups::INTEGER, "notify-subscription-id", id);
        req.integer(
            cups::SUBSCRIPTION,
            cups::INTEGER,
            "notify-lease-duration",
            SUBSCRIPTION_LEASE,
        );
        if conn.send(req, "/").is_ok() {
            return Some(id);
        }
    }
    let mut req = conn.request(cups::CREATE_PRINTER_SUBSCRIPTIONS);
    req.string(
        cups::OPERATION,
        cups::URI,
        "printer-uri",
        "ipp://localhost/",
    );
    req.string(
        cups::SUBSCRIPTION,
        cups::URI,
        "notify-recipient-uri",
        "dbus://",
    );
    req.strings(cups::SUBSCRIPTION, cups::KEYWORD, "notify-events", EVENTS);
    req.integer(
        cups::SUBSCRIPTION,
        cups::INTEGER,
        "notify-lease-duration",
        SUBSCRIPTION_LEASE,
    );
    let answer = conn.send(req, "/").ok()?;
    cups::one(&answer.group(cups::SUBSCRIPTION), "notify-subscription-id")
        .as_i64()
        .map(|n| n as i32)
}

fn cancel_subscription(server: &Server, id: i32) {
    let Ok(conn) = Conn::open(server, None, 2000) else {
        return;
    };
    let mut req = conn.request(cups::CANCEL_SUBSCRIPTION);
    req.string(
        cups::OPERATION,
        cups::URI,
        "printer-uri",
        "ipp://localhost/",
    );
    req.integer(cups::OPERATION, cups::INTEGER, "notify-subscription-id", id);
    let _ = conn.send(req, "/");
}

fn job_not_found(lang: Lang) -> ApiError {
    ApiError::tr(
        "not_found",
        lang,
        "printers.jobNotFound",
        crate::i18n::NO_PARAMS,
    )
}

fn job_error(e: &cups::IppError, lang: Lang) -> ApiError {
    if e.status == cups::STATUS_NOT_FOUND {
        return job_not_found(lang);
    }
    ipp_error(e, lang)
}

fn job_refused(e: ApiError, lang: Lang) -> ApiError {
    if e.key.as_deref() == Some("printers.notFound") {
        return job_not_found(lang);
    }
    e
}

/// An anonymous file with `bytes`, read from its start (the maintenance command for Print-Job).
fn memfile(bytes: &[u8]) -> std::io::Result<std::fs::File> {
    use std::io::{Seek, Write};
    use std::os::fd::FromRawFd;
    let fd = unsafe { libc::memfd_create(c"win11-cups-command".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut file = unsafe { std::fs::File::from_raw_fd(fd) };
    file.write_all(bytes)?;
    file.seek(std::io::SeekFrom::Start(0))?;
    Ok(file)
}

/// `~/.cups/lpoptions` written as cupsSetDests writes it: `~/.cups` made private when it is missing.
fn write_lpoptions(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    if let Some(dir) = path.parent()
        && !dir.is_dir()
    {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    write_atomic(path, text)
}

fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = path.with_extension(format!("win11-{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    let mode = std::fs::metadata(path)
        .map(|m| m.permissions().mode())
        .unwrap_or(0o644);
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode & 0o777))?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests;
