//! WinPoint's own updater `winpoint.*` (notes/claude/plan.md D-G74a, D-G74): the version check against GitHub's
//! releases and, after the user's click, the download and install of a newer release (job.rs).
//!
//! * **Roles.** The **owner** is win11-shell's backend (`win11-panel --http-only` with `WIN11_NATIVE_INSTANCE`, not a
//!   dry run: lib.rs run_http_only calls [`WinPoint::start`]) holding `winpoint/owner.lock`; a loser retries every
//!   30 s. Only the owner contacts GitHub and runs jobs: its daily schedule, the checks and the installs it is asked
//!   for. Every other host is a **reader**: `winpoint.status` reads the files, and `winpoint.check`,
//!   `winpoint.install` and `winpoint.cancel` are forwarded synchronously (`winpoint.request` → `winpoint.reply`,
//!   3 s, else `noOwner`). A dry run answers from the files and never checks or installs; a failed test seam
//!   (seam.rs) is `unsupported` for the life of the process: no network at all.
//! * **State.** `$XDG_CACHE_HOME/win11-gnome/winpoint/` (0700, files 0600): `state.json` (check.rs), `job.json` and
//!   `work/` (job.rs), `owner.lock`, `owner.json`, `check.lock` and `job.lock`. Texts are stored as keys and rendered
//!   per call in the caller's language. A folder or lock file that is not this user's alone is no seam failure: the
//!   card says which (`update.winpoint.err.cache`, the button stays), and every call tries the folder again, so the
//!   service recovers once it is fixed.
//! * **Events.** `winpoint.changed {rev}` from the owner whenever `state.json` or `job.json` changes, also when a
//!   check starts; `winpoint.progress {jobId, state, done, total}` at most once a second while a job downloads.
//! * **D-G74.** WinPoint starts one Python program of its own at run time: the release bundle's scripts/install.py
//!   (the system's python3 with `-E -s -B`, `--check` or the install), only from `winpoint.install`, that is after the
//!   user clicked Download and install or Try again. Never from a check, a timer, a notification, a status read or
//!   a reattach; no automatic installation (tests.rs holds the code to that).
//! * **Privacy.** One GET a day at most, of the public release list, with WinPoint's version in the User-Agent;
//!   `update.winpointAutoCheck` off stops the schedule and the notification (the button still works).
#[path = "winpoint/check.rs"]
pub(crate) mod check;
#[cfg(test)]
#[path = "winpoint/e2e_tests.rs"]
mod e2e_tests;
#[path = "winpoint/feed.rs"]
pub(crate) mod feed;
#[path = "winpoint/http.rs"]
pub(crate) mod http;
#[path = "winpoint/job.rs"]
pub(crate) mod job;
#[path = "winpoint/notify.rs"]
pub(crate) mod notify;
#[path = "winpoint/schedule.rs"]
pub(crate) mod schedule;
#[path = "winpoint/seam.rs"]
pub(crate) mod seam;
#[cfg(test)]
#[path = "winpoint/tests.rs"]
mod tests;
#[path = "winpoint/verify.rs"]
pub(crate) mod verify;
#[path = "winpoint/version.rs"]
pub(crate) mod version;

use super::{
    ServiceContext,
    update::{
        net::Net,
        store::{self, Store},
    },
};
use crate::{
    files::{ApiError, ApiResult},
    i18n::Lang,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};
use version::Version;

/// The methods, their argument keys, and whether they write (writes are forwarded and dry-run'd).
const METHODS: &[(&str, &[&str], bool)] = &[
    ("winpoint.status", &[], false),
    ("winpoint.check", &[], true),
    ("winpoint.install", &["version", "metered"], true),
    ("winpoint.cancel", &["jobId"], true),
];
/// The method table as (name, dangerous), for services/mod.rs `method_tables`.
#[cfg(test)]
pub(crate) fn method_flags() -> Vec<(&'static str, Option<bool>)> {
    METHODS
        .iter()
        .map(|(name, _, write)| (*name, Some(*write)))
        .collect()
}
/// How long a reader waits for the owner's answer to a forwarded call.
const FORWARD_WAIT: Duration = Duration::from_secs(3);
const OWNER_RETRY_S: u32 = 30;
/// scripts/install.py's payload marker: a host started from an installed payload (else a development run).
const PAYLOAD_MARKER: &str = "share/win11-gnome/.win11-gnome-owned";
const PAYLOAD_MAGIC: &str = "win11-gnome-rust-host-v1\n";
/// The version file install.py writes into the payload (D-G74a); missing: older than 0.9.0.
const PAYLOAD_VERSION: &str = "share/win11-gnome/VERSION";
/// The state folder under the host's cache folder (`$XDG_CACHE_HOME/win11-gnome`); uninstall.py removes it.
const STATE_DIR: &str = "winpoint";
/// A `shell.restartAll` this recent keeps an install from starting (the owner itself is about to restart).
const RESTART_WINDOW_MS: u64 = 10 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Owner,
    Reader,
    Dry,
    Unsupported,
}

/// Why there is no usable state folder.
#[derive(Debug)]
pub(crate) enum Blocked {
    /// the test seam was refused: `unsupported` for the life of the process
    Seam,
    /// the folder or a lock file in it is not this user's alone (stored and shown as `cache`; tried again)
    Cache(check::Failure),
}

/// What the check thread, the schedule and the status share.
pub(crate) struct Shared {
    pub ctx: Arc<ServiceContext>,
    pub store: Store,
    pub seam: seam::Seam,
    pub client: http::Client,
    pub origin: feed::Origin,
    pub arch: String,
    /// the host shuts down: a running GET ends within 200 ms (http.rs watchdog)
    pub stop: AtomicBool,
    rev: AtomicU64,
    notifier: notify::Notifier,
    /// a test's NetworkManager answer (None: ask NetworkManager)
    net: Mutex<Option<Net>>,
    /// one read-modify-write of `state.json` at a time (a check and a job's verdict)
    state_mx: Mutex<()>,
    /// this owner's running job (job.rs)
    pub job: Mutex<Option<job::Running>>,
    /// what tests put in place of this host's prefix, session, spawner and version
    pub hooks: Mutex<job::Hooks>,
    /// when this host last heard `shell.restartAll` (ms; 0: never)
    restart_seen: AtomicU64,
}

/// What this host compares with: the higher of its own version and the installed payload's (a newer install that
/// has not restarted this process yet), unless WinPoint's own install of that version did not finish
/// ([`unconfirmed`]: then the version that install started from); in test mode the seam's `running`, unless a test
/// gave this host its own version (job::Hooks).
fn current_of(
    seam: &seam::Seam,
    prefix: Option<&Path>,
    job: Option<&Value>,
    own: Option<&Version>,
) -> Version {
    if let (Some(running), None) = (seam.running(), own) {
        return running.clone();
    }
    let own = own
        .cloned()
        .unwrap_or_else(|| Version::parse(crate::VERSION).expect("the crate version is SemVer"));
    let installed = prefix.and_then(|p| version_file(&p.join(PAYLOAD_VERSION)));
    current_from(own, installed.as_deref().and_then(Version::parse), job)
}
/// The pure rule of [`current_of`]. An unfinished install of V leaves V's payload, and once install.py swapped the
/// binaries too, a host that runs V: V counts for neither, and the record's `from` (the version this host compared
/// with when the job started, job.rs `new_record`) is what WinPoint is until that install ends `installed`, so the
/// card keeps the job with Try again and `winpoint.install` V is still accepted. A record without `from` (written
/// before it existed) falls back to this host's own version below V. A job that changed nothing of what its click
/// found is no such install: V installed by hand after it (scripts/install.sh) is V.
pub(crate) fn current_from(
    own: Version,
    installed: Option<Version>,
    job: Option<&Value>,
) -> Version {
    let top = match installed {
        Some(installed) if installed > own => installed,
        _ => own.clone(),
    };
    if !unconfirmed(job, &top) {
        return top;
    }
    job.and_then(|record| record["from"].as_str())
        .and_then(Version::parse)
        .filter(|from| *from < top)
        .unwrap_or(if own < top { own } else { top })
}
/// The payload says `version`, but job.json holds WinPoint's own install of that version and it may have left that
/// payload unfinished: it is installing, was interrupted, or failed with the outcome `unknown` (install.py swapped the
/// new payload in, then stopped, or could not put everything back), or it is a Try again of such an install (at its
/// click the payload or this host's own version said `version` already, or the record it replaced was such an
/// install, job.rs `new_record` `found`: the payload rolled back to U while the binaries stayed V counts as well) that
/// runs now or ended without installing. That version does not count as installed, so the card still offers it and
/// Try again runs install.py once more, which rolls forward (its repair). An `installed` record, or none (cleared),
/// confirms it, and so does any other job that ended without changing what its click found (`nothingChanged`,
/// `cancelled`, `rolledBack`): the payload says `version` because another install (scripts/install.sh) put it there
/// since.
fn unconfirmed(job: Option<&Value>, version: &Version) -> bool {
    let version = version.to_string();
    job.is_some_and(|record| {
        record["version"].as_str() == Some(version.as_str())
            && match job::effective_state(record) {
                "installed" => false,
                "installing" | "interrupted" => true,
                _ => {
                    record["outcome"] == "unknown"
                        || record["found"].as_str() == Some(version.as_str())
                }
            }
    })
}
fn auto_check_of(ctx: &ServiceContext) -> bool {
    ctx.settings()["update"]["winpointAutoCheck"] != false
}

impl Shared {
    pub(crate) fn current(&self) -> Version {
        let own = self
            .hooks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .own_version
            .clone();
        let record = self.store.read(job::JOB).filter(Value::is_object);
        current_of(
            &self.seam,
            self.prefix().as_deref(),
            record.as_ref(),
            own.as_ref(),
        )
    }
    /// What [`Shared::current`] would be without job.json: the higher of this host's own version and the payload's
    /// (a new job's `found`, job.rs `new_record`).
    pub(crate) fn found(&self) -> Version {
        let own = self
            .hooks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .own_version
            .clone();
        current_of(&self.seam, self.prefix().as_deref(), None, own.as_ref())
    }
    /// The payload prefix this host runs from (a test's own, job::Hooks), or None: a development run.
    pub(crate) fn prefix(&self) -> Option<PathBuf> {
        let hook = self
            .hooks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .prefix
            .clone();
        hook.or_else(install_prefix)
    }
    fn installed_version(&self) -> Option<String> {
        version_file(&self.prefix()?.join(PAYLOAD_VERSION))
    }
    pub(crate) fn auto_check(&self) -> bool {
        auto_check_of(&self.ctx)
    }
    /// `state.json` as an object: missing, unreadable or any other JSON (`[]`, `0`, a string) is `{}`, which the
    /// next write replaces, so a damaged file heals instead of stopping every check.
    pub(crate) fn read_state(&self) -> Value {
        self.store
            .read(check::STATE)
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    }
    /// NetworkManager's connectivity and metered state (one NO_AUTO_START read; none at all: online).
    pub(crate) fn network(&self) -> Net {
        if let Some(net) = *self.net.lock().unwrap_or_else(|e| e.into_inner()) {
            return net;
        }
        match gio::bus_get_sync(gio::BusType::System, gio::Cancellable::NONE) {
            Ok(conn) => super::update::net::read(&conn),
            Err(_) => Net::default(),
        }
    }
    /// The next `rev`: one more than ours, the stored state's, the stored job's and `floor`.
    fn next_rev(&self, floor: u64) -> u64 {
        let stored = |name: &str| {
            self.store
                .read(name)
                .and_then(|s| s["rev"].as_u64())
                .unwrap_or(0)
        };
        let rev = self
            .rev
            .load(Ordering::SeqCst)
            .max(stored(check::STATE))
            .max(stored(job::JOB))
            .max(floor)
            + 1;
        self.rev.store(rev, Ordering::SeqCst);
        rev
    }
    /// Write `state` with the next `rev` and tell every page (`winpoint.changed {rev}`).
    pub(crate) fn write_state(&self, mut state: Value) {
        if !state.is_object() {
            state = json!({});
        }
        let rev = self.next_rev(state["rev"].as_u64().unwrap_or(0));
        state["rev"] = json!(rev);
        state["schema"] = json!(1);
        if self.store.write(check::STATE, &state).is_ok() {
            self.ctx.emit("winpoint.changed", json!({"rev":rev}));
        }
    }
    /// Change `state.json` in place (one writer at a time in this process: the owner's check and job).
    pub(crate) fn update_state(&self, change: impl FnOnce(&mut Value)) {
        let _one = self.state_mx.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = self.read_state();
        change(&mut state);
        self.write_state(state);
    }
    /// Write `job.json` with the next `rev` and tell every page.
    pub(crate) fn write_job(&self, record: &mut Value) {
        let rev = self.next_rev(record["rev"].as_u64().unwrap_or(0));
        record["rev"] = json!(rev);
        if self.store.write(job::JOB, record).is_ok() {
            self.ctx.emit("winpoint.changed", json!({"rev":rev}));
        }
    }
    /// A change without a file of its own (a cleared job): the pages read the status again.
    pub(crate) fn bump(&self) {
        let rev = self.next_rev(0);
        self.ctx.emit("winpoint.changed", json!({"rev":rev}));
    }
    /// The notification for `version` (notify.rs); its action opens Settings › Update & recovery. True once GNOME
    /// showed it (waits up to 10 s; never on the main thread).
    pub(crate) fn notify(&self, version: &Version) -> bool {
        let lang = self.ctx.lang();
        let params = json!({"version": version.to_string()});
        let note = notify::Note {
            title: crate::i18n::trf(lang, "update.winpoint.notifyTitle", &params),
            body: crate::i18n::tr(lang, "update.winpoint.notifyBody"),
            action: crate::i18n::tr(lang, "update.winpoint.notifyAction"),
        };
        self.notifier.show(note, self.open_update()).wait()
    }
    /// "WinPoint V is installed" after a job (job.rs), with what finishes it: a sign-out or a restart of WinPoint.
    /// Its action opens Settings › Update & recovery too; it never starts anything else.
    pub(crate) fn notify_installed(&self, version: &Version, relogin: bool) {
        let lang = self.ctx.lang();
        let params = json!({"version": version.to_string()});
        let body = if relogin {
            crate::i18n::tr(lang, "update.winpoint.installedRelogin")
        } else {
            crate::i18n::tr(lang, "update.winpoint.installedRestart")
        };
        let note = notify::Note {
            title: crate::i18n::trf(lang, "update.winpoint.installedTitle", &params),
            body,
            action: crate::i18n::tr(lang, "update.winpoint.notifyAction"),
        };
        let _ = self.notifier.show(note, self.open_update());
    }
    fn open_update(&self) -> notify::Open {
        let ctx = Arc::downgrade(&self.ctx);
        Arc::new(move |token| {
            let Some(ctx) = ctx.upgrade() else { return };
            let mut args = json!({"app":"settings","params":{"page":"update"}});
            if let Some(token) = token {
                args["activationToken"] = json!(token);
            }
            let _ = ctx.call("shell.openApp", &args);
        })
    }
}

/// The payload prefix this host runs from (`<prefix>/bin/<program>` next to `<prefix>/share/win11-gnome` with
/// install.py's marker), or None for a development run.
fn install_prefix() -> Option<PathBuf> {
    let exe = crate::exec_path(std::env::current_exe().ok()?);
    let prefix = exe.parent()?.parent()?;
    let marker = std::fs::read_to_string(prefix.join(PAYLOAD_MARKER)).ok()?;
    (marker == PAYLOAD_MAGIC).then(|| prefix.to_path_buf())
}
/// The folder this host's program runs from (the `dev` reason names it).
fn exe_dir() -> String {
    std::env::current_exe()
        .ok()
        .map(crate::exec_path)
        .and_then(|exe| exe.parent().map(|p| p.display().to_string()))
        .unwrap_or_default()
}
/// The installed payload's VERSION (install.py), when this host runs from one.
fn installed_version() -> Option<String> {
    version_file(&install_prefix()?.join(PAYLOAD_VERSION))
}
pub(crate) fn version_file(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let line = text.strip_suffix('\n').unwrap_or(&text);
    Version::parse(line).map(|v| v.to_string())
}

fn spec(method: &str) -> Option<(&'static [&'static str], bool)> {
    METHODS
        .iter()
        .find(|(name, _, _)| *name == method)
        .map(|(_, keys, write)| (*keys, *write))
}
/// The argument allowlist: an object with only the method's keys; `winpoint.install` needs `version` (a version
/// of at most 32 characters) and takes `metered` (a bool), `winpoint.cancel` needs `jobId` (a job's token).
pub(crate) fn check_args(method: &str, args: &Value) -> bool {
    let Some((keys, _)) = spec(method) else {
        return false;
    };
    let Some(object) = args.as_object() else {
        return false;
    };
    if !object.keys().all(|k| keys.contains(&k.as_str())) {
        return false;
    }
    match method {
        "winpoint.install" => {
            object
                .get("version")
                .and_then(Value::as_str)
                .is_some_and(|v| v.len() <= 32 && Version::parse(v).is_some())
                && object.get("metered").is_none_or(Value::is_boolean)
        }
        "winpoint.cancel" => object
            .get("jobId")
            .and_then(Value::as_str)
            .is_some_and(store::is_token),
        _ => true,
    }
}
fn bad_args(lang: Lang) -> ApiError {
    ApiError::tr(
        "bad_args",
        lang,
        "update.err.badArgs",
        crate::i18n::NO_PARAMS,
    )
}
fn seam_error(lang: Lang) -> ApiError {
    ApiError::tr(
        "test_seam",
        lang,
        "update.err.testSeam",
        crate::i18n::NO_PARAMS,
    )
}
/// A [`Blocked::Cache`] failure as the answer of a write (its raw detail in `data`).
fn cache_error(lang: Lang, failure: &check::Failure) -> ApiError {
    let mut error = ApiError::tr(failure.code, lang, failure.key, &failure.params);
    error.data = Some(json!({"detail": failure.detail}));
    error
}
/// No owner answered a forwarded call: a check says it cannot check, an install or a Cancel that it cannot update.
fn no_owner_for(method: &str, lang: Lang) -> ApiError {
    let key = if method == "winpoint.check" {
        "update.winpoint.err.noOwner"
    } else {
        "update.winpoint.err.noOwnerUpdate"
    };
    ApiError::tr("noOwner", lang, key, crate::i18n::NO_PARAMS)
}
fn no_owner(lang: Lang) -> ApiError {
    no_owner_for("winpoint.check", lang)
}

/// The owner's error as the reader passes it on (contracts C2: code, message, key, params, data).
fn error_from_json(value: &Value, lang: Lang) -> ApiError {
    let Some(code) = value["code"].as_str() else {
        return no_owner(lang);
    };
    ApiError {
        code: code.to_owned(),
        message: value["message"].as_str().unwrap_or("").to_owned(),
        data: value.get("data").cloned(),
        key: value["key"].as_str().map(str::to_owned),
        params: value.get("params").cloned(),
    }
}

/// A stored error record `{code, key, params, detail}` in `lang` (only `update.` keys are rendered).
fn render(record: &Value, state: &Value, lang: Lang) -> Value {
    let Some(key) = record["key"].as_str().filter(|k| k.starts_with("update.")) else {
        return Value::Null;
    };
    let mut params = record["params"].as_object().cloned().unwrap_or_default();
    if key == "update.winpoint.err.rateLimited" {
        // the local time of day GitHub lets WinPoint ask again
        let time = state["retryAfter"]
            .as_u64()
            .and_then(|ms| glib::DateTime::from_unix_local((ms / 1000) as i64).ok())
            .and_then(|t| t.format("%H:%M").ok())
            .map(|t| t.to_string())
            .unwrap_or_default();
        params.insert("time".into(), json!(time));
    }
    let params = Value::Object(params);
    json!({"code":record["code"],"key":key,"params":params,"message":crate::i18n::trf(lang, key, &params),
           "detail":record["detail"].as_str().unwrap_or("")})
}

/// What the status of every host is made of (pure apart from the liveness of a job's processes, so the state
/// rules are tested on their own).
pub(crate) struct View<'a> {
    pub mode: &'static str,
    /// None: not known (the state folder cannot be used)
    pub owner_running: Option<bool>,
    pub checking: bool,
    /// the state folder or a lock file in it cannot be used: a stored error record (check.rs `Failure`)
    pub fault: Option<Value>,
    pub state: &'a Value,
    /// `job.json` (job.rs), if there is one
    pub job: Option<&'a Value>,
    pub current: &'a Version,
    pub installed: Option<String>,
    pub arch: &'a str,
    pub auto: bool,
    pub dry: bool,
    /// the system's python3 is there (the installer's interpreter, job.rs)
    pub python: bool,
    /// the folder this host's program runs from (the `dev` reason names it)
    pub exe_dir: String,
    pub lang: Lang,
}

/// Why the offered release has no install button, and its text: `noAsset` (no download for this PC), `dev` (a
/// development run), `noDigest` (GitHub gave no digest for its tarball: the card links to the release page),
/// `noPython`, or a job's verdict for it (`incompatible`, `manual`, `missingParts`, from install.py `--check`).
fn reason_of(view: &View, release: &feed::Release) -> Option<(&'static str, Value, Value)> {
    let lang = view.lang;
    let Some(assets) = release.assets.as_ref() else {
        let params = json!({"arch": view.arch});
        let message = crate::i18n::trf(lang, "update.winpoint.reason.noAsset", &params);
        return Some(("noAsset", params, json!(message)));
    };
    if view.mode == "dev" {
        let params = json!({"path": view.exe_dir});
        let message = crate::i18n::trf(lang, "update.winpoint.reason.dev", &params);
        return Some(("dev", params, json!(message)));
    }
    if assets.tarball.digest.is_none() {
        let message = crate::i18n::tr(lang, "update.winpoint.reason.noDigest");
        return Some(("noDigest", json!({}), json!(message)));
    }
    if !view.python {
        let message = crate::i18n::tr(lang, "update.winpoint.reason.noPython");
        return Some(("noPython", json!({}), json!(message)));
    }
    let blocked = job::blocked_reason(view.state, &release.version)?;
    let params = blocked["params"].clone();
    let (code, key) = job::reason_key(blocked["code"].as_str().unwrap_or(""), &params);
    Some((
        code,
        params.clone(),
        json!(crate::i18n::trf(lang, key, &params)),
    ))
}

/// The job codes that end with a reason of the release itself: Try again would end the same way.
const FINAL_CODES: [&str; 6] = [
    "incompatible",
    "manual",
    "missingParts",
    "noDigest",
    "noAsset",
    "stale",
];

/// The job part of the status: its state as it shows (job.rs `effective_state`) and its texts, or None when it no
/// longer matters (cleared, cancelled, a failure that a check came after, a version that is no longer newer).
fn job_view(view: &View, latest: &Value) -> Option<Value> {
    let record = view.job?;
    let state = job::effective_state(record);
    let version = record["version"].as_str().and_then(Version::parse)?;
    let newer = version > *view.current;
    let checked_since = view.state["checkedAt"]
        .as_u64()
        .zip(record["finishedAt"].as_u64())
        .is_some_and(|(checked, finished)| checked > finished);
    // the release offered now: an interrupted install gives way once another one is (installing that one goes
    // through install.py's own repair), and stays while none is (nothing else to offer)
    let offered = latest.is_null() || latest["version"] == record["version"];
    let shown = match state {
        "downloading" | "verifying" | "unpacking" | "preparing" | "installing" => true,
        // until a restart or a sign-out, unless another install took its place since (the payload says another
        // version)
        "installed" => view.installed.as_deref() == record["version"].as_str(),
        "interrupted" => newer && offered,
        "failed" => newer && !checked_since,
        _ => false,
    };
    if !shown {
        return None;
    }
    let lang = view.lang;
    let (error, outcome) = match (state, record["state"].as_str()) {
        // the owner died before the installer: the next owner writes this too (job.rs reattach)
        ("failed", Some(before)) if job::BEFORE_INSTALL.contains(&before) => (
            json!({"code":"ownerStopped","key":"update.winpoint.err.ownerStopped","params":{},"detail":""}),
            json!("nothingChanged"),
        ),
        ("interrupted", _) => (record["error"].clone(), json!("unknown")),
        _ => (record["error"].clone(), record["outcome"].clone()),
    };
    let outcome_message = match outcome.as_str() {
        Some("nothingChanged") => {
            json!(crate::i18n::tr(lang, "update.winpoint.err.nothingChanged"))
        }
        Some("rolledBack") => json!(crate::i18n::tr(lang, "update.winpoint.err.rolledBack")),
        Some("unknown") => json!(crate::i18n::tr(lang, "update.winpoint.err.unknown")),
        _ => Value::Null,
    };
    let code = error["code"].as_str().unwrap_or("");
    let can_retry = matches!(state, "failed" | "interrupted")
        && !FINAL_CODES.contains(&code)
        && latest["version"] == record["version"]
        && latest["canInstall"] == true;
    Some(json!({
        "jobId": record["jobId"], "version": record["version"], "state": state, "done": record["done"],
        "total": record["total"], "startedAt": record["startedAt"], "finishedAt": record["finishedAt"],
        "restart": record["restart"], "outcome": outcome, "error": render(&error, view.state, lang),
        "outcomeMessage": outcome_message, "logTail": record["logTail"],
        "canCancel": job::BEFORE_INSTALL.contains(&state), "canRetry": can_retry,
    }))
}

/// `winpoint.status`. `state`, in order: `unsupported`; `checkFailed` with the `cache` error while the state folder
/// cannot be used (`ownerRunning` null: not known); a running job (`downloading`, `verifying`, `unpacking`,
/// `preparing`, `installing`); `installed` (until a restart of WinPoint or a sign-out finishes it); `interrupted`;
/// `failed` (until the next check); `checking` (the owner's `check.lock` is held); the last result when it failed:
/// `offline`, `rateLimited`, `unreachable` or `checkFailed`; `available` (a stored release newer than `current`);
/// `upToDate` (a check answered); `unknown` (never checked). `latest` is that newer release or null; it stays under
/// a failed state too, with `reason` (null: `canInstall` in an installed run). `job` is the job of the state, or null.
pub(crate) fn compose(view: &View) -> Value {
    let s = view.state;
    let newer = check::newer_latest(s, view.current);
    let result = s["result"].as_str().unwrap_or("");
    let latest = newer.map(|release| {
        let tarball = release.assets.as_ref().map(|a| &a.tarball);
        let reason = reason_of(view, &release);
        let detail = job::blocked_reason(s, &release.version)
            .filter(|_| reason.as_ref().is_some_and(|(code, _, _)| FINAL_CODES[..3].contains(code)))
            .map_or(Value::Null, |b| b["detail"].clone());
        let can_install = reason.is_none() && view.mode == "installed" && !view.dry;
        let (code, params, message) = reason.map_or((Value::Null, Value::Null, Value::Null), |(c, p, m)| {
            (json!(c), p, m)
        });
        json!({"version":release.version.to_string(),"tag":release.tag,"name":release.name,
               "publishedAt":release.published_at,"notesUrl":release.notes_url,
               "asset":tarball.map(|t| t.name.clone()),"size":tarball.map(|t| t.size),
               "reason":code,"params":params,"message":message,"detail":detail,"canInstall":can_install})
    });
    let latest = latest.unwrap_or(Value::Null);
    let job = if view.mode == "unsupported" || view.fault.is_some() {
        None
    } else {
        job_view(view, &latest)
    };
    let state = if view.mode == "unsupported" {
        "unsupported"
    } else if view.fault.is_some() {
        "checkFailed"
    } else if let Some(state) = job.as_ref().and_then(|j| j["state"].as_str()) {
        match state {
            "downloading" => "downloading",
            "verifying" => "verifying",
            "unpacking" => "unpacking",
            "preparing" => "preparing",
            "installing" => "installing",
            "installed" => "installed",
            "interrupted" => "interrupted",
            _ => "failed",
        }
    } else if view.checking {
        "checking"
    } else {
        match result {
            "offline" => "offline",
            "rateLimited" => "rateLimited",
            "unreachable" => "unreachable",
            "error" => "checkFailed",
            _ if !latest.is_null() => "available",
            "ok" | "noReleases" => "upToDate",
            _ => "unknown",
        }
    };
    let error = if view.mode == "unsupported" {
        let e = seam_error(view.lang);
        json!({"code":e.code,"key":e.key,"params":{},"message":e.message,"detail":""})
    } else if let Some(fault) = &view.fault {
        render(fault, s, view.lang)
    } else if matches!(
        state,
        "offline" | "rateLimited" | "unreachable" | "checkFailed"
    ) {
        render(&s["error"], s, view.lang)
    } else {
        Value::Null
    };
    let ms = |name: &str| s[name].as_u64().map_or(Value::Null, |v| json!(v));
    let rev = s["rev"]
        .as_u64()
        .unwrap_or(0)
        .max(view.job.and_then(|j| j["rev"].as_u64()).unwrap_or(0));
    json!({
        "mode": view.mode, "ownerRunning": view.owner_running, "version": crate::VERSION,
        "installed": view.installed, "current": view.current.to_string(), "arch": view.arch,
        "autoCheck": view.auto, "state": state, "result": if result.is_empty() { Value::Null } else { json!(result) },
        "checkedAt": ms("checkedAt"), "okAt": ms("okAt"), "nextCheckAt": ms("nextCheckAt"),
        "retryAfter": ms("retryAfter"), "latest": latest, "job": job, "error": error,
        "rev": rev, "dryRun": view.dry,
    })
}

pub(crate) struct WinPoint {
    ctx: Arc<ServiceContext>,
    /// None: the seam was refused (`unsupported`, never a state folder)
    seam: Option<seam::Seam>,
    target: notify::Target,
    /// built the first time the state folder opens; until then every use tries again
    shared: Mutex<Option<Arc<Shared>>>,
    dry: bool,
    mode: Mutex<Mode>,
    owner_lock: Mutex<Option<store::FileLock>>,
    started: AtomicBool,
    stopping: AtomicBool,
    /// forwarded calls waiting for the owner's `winpoint.reply`, by nonce
    pending: Mutex<HashMap<String, mpsc::SyncSender<Value>>>,
    weak: Weak<Self>,
}

impl WinPoint {
    pub(crate) fn new(ctx: Arc<ServiceContext>) -> Result<Arc<Self>, String> {
        let seam = seam::Seam::process();
        Ok(Self::with(ctx, seam, notify::Target::Session))
    }
    /// The service for an explicit seam and notification target (the tests).
    pub(crate) fn with(
        ctx: Arc<ServiceContext>,
        seam: Result<seam::Seam, String>,
        target: notify::Target,
    ) -> Arc<Self> {
        let dry = ctx.dry_run || crate::backend::dry_run();
        let seam = seam.ok();
        let mode = match &seam {
            None => Mode::Unsupported,
            Some(_) if dry => Mode::Dry,
            Some(_) => Mode::Reader,
        };
        let service = Arc::new_cyclic(|weak| Self {
            ctx,
            seam,
            target: if dry { notify::Target::Off } else { target },
            shared: Mutex::new(None),
            dry,
            mode: Mutex::new(mode),
            owner_lock: Mutex::new(None),
            started: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            pending: Mutex::default(),
            weak: weak.clone(),
        });
        if mode == Mode::Reader {
            service.observe();
        }
        let _ = service.open();
        service
    }
    pub(crate) fn mode(&self) -> Mode {
        *self.mode.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn shared(&self) -> Option<Arc<Shared>> {
        self.open().ok()
    }
    /// The shared state, built when the state folder first opens (`Store::open`: ours, 0700, not a link); a folder
    /// that fails is tried again on the next call.
    fn open(&self) -> Result<Arc<Shared>, Blocked> {
        let mut slot = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(shared) = slot.as_ref() {
            return Ok(shared.clone());
        }
        let seam = self.seam.clone().ok_or(Blocked::Seam)?;
        let dir = self.ctx.cache_dir.join(STATE_DIR);
        let store = Store::open(dir.clone())
            .map_err(|error| Blocked::Cache(check::Failure::cache(&dir, None, &error)))?;
        let shared = Arc::new(Shared {
            ctx: self.ctx.clone(),
            store,
            client: seam.client(),
            origin: seam.origin(),
            seam,
            arch: feed::machine(),
            stop: AtomicBool::new(false),
            rev: AtomicU64::new(0),
            notifier: notify::Notifier::new(self.target.clone()),
            net: Mutex::new(None),
            state_mx: Mutex::new(()),
            job: Mutex::new(None),
            hooks: Mutex::default(),
            restart_seen: AtomicU64::new(0),
        });
        *slot = Some(shared.clone());
        Ok(shared)
    }
    /// [`open`](Self::open), and its two lock files usable (not ours alone: [`Blocked::Cache`], as `held` says).
    fn usable(&self) -> Result<Arc<Shared>, Blocked> {
        let shared = self.open()?;
        for name in [store::OWNER_LOCK, check::CHECK_LOCK] {
            if let Err(error) = shared.store.held(name) {
                return Err(Blocked::Cache(check::Failure::cache(
                    shared.store.dir(),
                    Some(name),
                    &error,
                )));
            }
        }
        Ok(shared)
    }

    // ------------------------------------------------------------------------------------ owner

    /// lib.rs run_http_only, on the main thread of win11-shell's backend: try the owner lock (a loser retries every
    /// 30 s) and start the schedule (schedule.rs): once 3 to 10 minutes from now, then every hour. The timers only
    /// read files and start a check thread.
    pub(crate) fn start(&self) {
        if self.mode() != Mode::Reader || self.started.swap(true, Ordering::SeqCst) {
            return;
        }
        if !self.take_over() {
            let weak = self.weak.clone();
            glib::timeout_add_seconds(OWNER_RETRY_S, move || match weak.upgrade() {
                Some(s) if s.mode() == Mode::Reader && !s.stopping.load(Ordering::SeqCst) => {
                    if s.take_over() {
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                }
                _ => glib::ControlFlow::Break,
            });
        }
        let first = schedule::first_delay_s(
            self.seam.as_ref().is_some_and(|s| s.running().is_some()),
            glib::random_int_range(0, schedule::FIRST_JITTER_S as i32) as u32,
        );
        let weak = self.weak.clone();
        glib::timeout_add_seconds_local(first, move || {
            if let Some(service) = weak.upgrade() {
                service.tick();
            }
            glib::ControlFlow::Break
        });
        let weak = self.weak.clone();
        glib::timeout_add_seconds_local(schedule::TICK_S, move || match weak.upgrade() {
            Some(service) if !service.stopping.load(Ordering::SeqCst) => {
                service.tick();
                glib::ControlFlow::Continue
            }
            _ => glib::ControlFlow::Break,
        });
    }
    /// Become the owner when `owner.lock` is free (held for this process's life).
    pub(crate) fn take_over(&self) -> bool {
        if self.stopping.load(Ordering::SeqCst) || self.mode() != Mode::Reader {
            return false;
        }
        let Ok(shared) = self.open() else {
            return false;
        };
        let Ok(store::Election::Won(lock)) = store::elect(&shared.store) else {
            return false;
        };
        *self.owner_lock.lock().unwrap_or_else(|e| e.into_inner()) = Some(lock);
        *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::Owner;
        // what the last owner left: an installer to watch, a job to end, an install that is complete now
        job::reattach(&shared, true);
        true
    }
    /// A schedule tick: start an automatic check when one is due (never while a job runs). A tick never installs.
    fn tick(&self) {
        if self.mode() != Mode::Owner || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Some(shared) = self.shared() else { return };
        let state = shared.read_state();
        let busy = job::job_active(&self.ctx.cache_dir);
        if schedule::due(store::now_ms(), &state, shared.auto_check(), busy) {
            self.spawn_check(check::Trigger::Auto);
        }
    }
    fn spawn_check(&self, trigger: check::Trigger) -> bool {
        let Some(shared) = self.shared() else {
            return false;
        };
        std::thread::Builder::new()
            .name("winpoint-check".into())
            .spawn(move || {
                // blocking GIO calls only, in a context of this thread's own (never the host's main loop)
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| check::run(&shared, trigger));
            })
            .is_ok()
    }

    // ------------------------------------------------------------------------------------ forwarding

    /// Every reader answers the owner's replies to its forwarded calls; a reader that became the owner answers
    /// the requests of the others.
    fn observe(&self) {
        let weak = self.weak.clone();
        self.ctx.events.on(
            "winpoint.reply",
            Arc::new(move |data| {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let Some(nonce) = data["nonce"].as_str() else {
                    return;
                };
                let waiting = service
                    .pending
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(nonce);
                if let Some(waiting) = waiting {
                    let _ = waiting.try_send(data.clone());
                }
            }),
        );
        let weak = self.weak.clone();
        self.ctx.events.on(
            "winpoint.request",
            Arc::new(move |data| {
                if let Some(service) = weak.upgrade() {
                    service.on_request(data);
                }
            }),
        );
        // a restart of every WinPoint process is on its way: no install starts in this owner before it
        let weak = self.weak.clone();
        self.ctx.events.on(
            "shell.restartAll",
            Arc::new(move |_| {
                if let Some(shared) = weak.upgrade().and_then(|s| s.shared()) {
                    shared.restart_seen.store(store::now_ms(), Ordering::SeqCst);
                }
            }),
        );
    }
    /// A forwarded write: validated with the same allowlist, run here, answered as `winpoint.reply` (off the
    /// event receiver thread).
    fn on_request(&self, data: &Value) {
        if self.mode() != Mode::Owner || self.stopping.load(Ordering::SeqCst) {
            return;
        }
        let Some(nonce) = data["nonce"]
            .as_str()
            .filter(|n| store::is_token(n))
            .map(str::to_owned)
        else {
            return;
        };
        let (method, args, weak) = (
            data["method"].as_str().unwrap_or("").to_owned(),
            data["args"].clone(),
            self.weak.clone(),
        );
        let _ = std::thread::Builder::new()
            .name("winpoint-reply".into())
            .spawn(move || {
                let Some(service) = weak.upgrade() else {
                    return;
                };
                let lang = service.ctx.lang();
                let result = if spec(&method).is_some_and(|(_, write)| write)
                    && check_args(&method, &args)
                {
                    service.write(&method, &args, lang)
                } else {
                    Err(bad_args(lang))
                };
                let reply = match result {
                    Ok(result) => json!({"nonce":nonce,"ok":true,"result":result}),
                    Err(error) => json!({"nonce":nonce,"ok":false,"error":error.json()}),
                };
                service.ctx.emit("winpoint.reply", reply);
            });
    }
    /// A reader's write: the owner's own answer, or `noOwner` after 3 s.
    fn forward(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        let nonce = store::token().map_err(|_| bad_args(lang))?;
        let (tx, rx) = mpsc::sync_channel(1);
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(nonce.clone(), tx);
        self.ctx.emit(
            "winpoint.request",
            json!({"nonce":nonce,"method":method,"args":args}),
        );
        let reply = rx.recv_timeout(FORWARD_WAIT);
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&nonce);
        match reply {
            Ok(reply) if reply["ok"] == true => Ok(reply["result"].clone()),
            Ok(reply) => Err(error_from_json(&reply["error"], lang)),
            Err(_) => Err(no_owner_for(method, lang)),
        }
    }

    // ------------------------------------------------------------------------------------ calls

    pub(crate) fn call(&self, method: &str, args: &Value) -> Option<ApiResult> {
        let (_, write) = spec(method)?;
        let lang = self.ctx.lang();
        if !check_args(method, args) {
            return Some(Err(bad_args(lang)));
        }
        if !write {
            return Some(Ok(self.status(lang)));
        }
        Some(match self.mode() {
            Mode::Unsupported => Err(seam_error(lang)),
            Mode::Dry => Ok(match method {
                "winpoint.install" => json!({"jobId":null,"dryRun":true}),
                "winpoint.cancel" => json!({"ok":true,"dryRun":true}),
                _ => json!({"started":false,"dryRun":true}),
            }),
            // a state folder this user cannot use: say so at once (no owner could take it either)
            mode => match self.usable() {
                Err(Blocked::Seam) => Err(seam_error(lang)),
                Err(Blocked::Cache(failure)) => Err(cache_error(lang, &failure)),
                Ok(_) if mode == Mode::Owner => self.write(method, args, lang),
                Ok(_) => self.forward(method, args, lang),
            },
        })
    }
    /// A write in the owner: `winpoint.check`, `winpoint.install`, `winpoint.cancel`.
    fn write(&self, method: &str, args: &Value, lang: Lang) -> ApiResult {
        match method {
            "winpoint.check" => self.check_now(),
            "winpoint.install" => self.install(args, lang),
            "winpoint.cancel" => self.cancel(args, lang),
            _ => Err(bad_args(lang)),
        }
    }

    /// `winpoint.install {version, metered?}` in the owner (D-G74): the user's click on Download and install or Try
    /// again, and the only start of a job. The preconditions in order, each with its own code: an installed run
    /// (`dev {path}`); `version` is the release offered now (`stale`) with a download for this PC (`noAsset`), a
    /// digest from GitHub (`noDigest`) and no verdict against it (`incompatible`, `manual`, `missingParts`);
    /// the system's python3 (`noPython`); online (`offline`) and not metered unless `metered:true` or
    /// `update.metered` (`metered`, data `{size}`); nothing else running (`busy`: a job, Ubuntu's upgrader, an update
    /// or Software job, a `shell.restartAll` under way); room in the cache (`space {size, path}`). Then `{jobId}`;
    /// the job asks GitHub again first (job.rs).
    fn install(&self, args: &Value, lang: Lang) -> ApiResult {
        let shared = match self.open() {
            Ok(shared) => shared,
            Err(Blocked::Seam) => return Err(seam_error(lang)),
            Err(Blocked::Cache(failure)) => return Err(cache_error(lang, &failure)),
        };
        let wanted = args["version"]
            .as_str()
            .and_then(Version::parse)
            .ok_or_else(|| bad_args(lang))?;
        let Some(prefix) = shared.prefix() else {
            return Err(ApiError::tr(
                "dev",
                lang,
                "update.winpoint.reason.dev",
                &json!({"path": exe_dir()}),
            ));
        };
        let state = shared.read_state();
        let current = shared.current();
        let stale = || {
            ApiError::tr(
                "stale",
                lang,
                "update.winpoint.err.stale",
                crate::i18n::NO_PARAMS,
            )
        };
        let release = check::newer_latest(&state, &current)
            .filter(|r| r.version == wanted)
            .ok_or_else(stale)?;
        let Some(assets) = release.assets.as_ref() else {
            return Err(ApiError::tr(
                "noAsset",
                lang,
                "update.winpoint.reason.noAsset",
                &json!({"arch": shared.arch}),
            ));
        };
        if assets.tarball.digest.is_none() {
            return Err(ApiError::tr(
                "noDigest",
                lang,
                "update.winpoint.reason.noDigest",
                crate::i18n::NO_PARAMS,
            ));
        }
        if let Some(blocked) = job::blocked_reason(&state, &wanted) {
            let params = blocked["params"].clone();
            let (code, key) = job::reason_key(blocked["code"].as_str().unwrap_or(""), &params);
            let mut error = ApiError::tr(code, lang, key, &params);
            error.data = Some(json!({"detail": blocked["detail"]}));
            return Err(error);
        }
        if !job::python_present() {
            return Err(ApiError::tr(
                "noPython",
                lang,
                "update.winpoint.reason.noPython",
                crate::i18n::NO_PARAMS,
            ));
        }
        let net = shared.network();
        if net.refuses_check() {
            return Err(ApiError::tr(
                "offline",
                lang,
                "update.winpoint.err.offline",
                crate::i18n::NO_PARAMS,
            ));
        }
        let metered_ok =
            args["metered"] == true || self.ctx.settings()["update"]["metered"] == true;
        if net.metered && !metered_ok {
            let mut error = ApiError::tr(
                "metered",
                lang,
                "update.winpoint.err.metered",
                crate::i18n::NO_PARAMS,
            );
            error.data = Some(json!({"size": assets.tarball.size}));
            return Err(error);
        }
        let busy = || {
            ApiError::tr(
                "busy",
                lang,
                "update.winpoint.err.busy",
                crate::i18n::NO_PARAMS,
            )
        };
        if self.busy(&shared) {
            return Err(busy());
        }
        let lock = match shared.store.try_lock(job::JOB_LOCK) {
            Ok(Some(lock)) => lock,
            Ok(None) => return Err(busy()),
            Err(error) => {
                let failure =
                    check::Failure::cache(shared.store.dir(), Some(job::JOB_LOCK), &error);
                return Err(cache_error(lang, &failure));
            }
        };
        job::cache_space(&shared, &wanted, &assets.tarball, lang)?;
        let id = store::token().map_err(|_| busy())?;
        let hook = shared
            .hooks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .session_key
            .clone();
        let session = hook.unwrap_or_else(job::session_key);
        // a Try again of an install of V that may be unfinished stays one, also when this host still runs U and the
        // payload says U (a rollback that left V's binaries): the record it replaces says so
        let earlier = shared.store.read(job::JOB).filter(Value::is_object);
        let found = if unconfirmed(earlier.as_ref(), &wanted) {
            wanted.clone()
        } else {
            shared.found()
        };
        let record = job::new_record(&id, &release, &current, &found, &prefix, lang, session);
        job::start(&shared, record, lock).map_err(|_| busy())?;
        Ok(json!({"jobId": id}))
    }
    /// Something else keeps an install from starting now (`busy`).
    fn busy(&self, shared: &Shared) -> bool {
        let running = shared
            .job
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some();
        let seen = shared.restart_seen.load(Ordering::SeqCst);
        running
            || job::job_active(&self.ctx.cache_dir)
            || (seen > 0 && store::now_ms().saturating_sub(seen) < RESTART_WINDOW_MS)
            || super::update::guard::busy_for_winpoint(&self.ctx)
    }
    /// `winpoint.cancel {jobId}`: `{ok:true}` while the job downloads, verifies, unpacks or prepares (it ends as
    /// `cancelled`, its `.part` deleted); `cannot_cancel` once the installer runs; `no_job` for any other id.
    fn cancel(&self, args: &Value, lang: Lang) -> ApiResult {
        let shared = match self.open() {
            Ok(shared) => shared,
            Err(Blocked::Seam) => return Err(seam_error(lang)),
            Err(Blocked::Cache(failure)) => return Err(cache_error(lang, &failure)),
        };
        match job::cancel(&shared, args["jobId"].as_str().unwrap_or("")) {
            Ok(()) => Ok(json!({"ok":true})),
            Err("cannot_cancel") => Err(ApiError::tr(
                "cannot_cancel",
                lang,
                "update.winpoint.err.cannotCancel",
                crate::i18n::NO_PARAMS,
            )),
            Err(_) => Err(ApiError::tr(
                "no_job",
                lang,
                "update.winpoint.err.noJob",
                crate::i18n::NO_PARAMS,
            )),
        }
    }
    /// The page's "Check for updates": `{started:true}`, or `{started:false, skipped}` with
    /// `running` (a check runs), `recent` (one ended within 30 s), `rateLimited` (GitHub's wait, with
    /// `retryAfter`) or `offline` (NetworkManager has no connectivity; the card then shows offline). It ignores
    /// the schedule and the setting.
    fn check_now(&self) -> ApiResult {
        let lang = self.ctx.lang();
        let shared = match self.open() {
            Ok(shared) => shared,
            Err(Blocked::Seam) => return Err(seam_error(lang)),
            Err(Blocked::Cache(failure)) => return Err(cache_error(lang, &failure)),
        };
        let shared = &shared;
        match check::check_lock(shared) {
            Ok(Some(free)) => drop(free),
            Ok(None) => return Ok(json!({"started":false,"skipped":"running"})),
            Err(error) => {
                let failure =
                    check::Failure::cache(shared.store.dir(), Some(check::CHECK_LOCK), &error);
                return Err(cache_error(lang, &failure));
            }
        }
        let state = shared.read_state();
        let now = store::now_ms();
        if state["checkedAt"]
            .as_u64()
            .is_some_and(|at| at <= now && now - at < schedule::RECENT_MS)
        {
            return Ok(json!({"started":false,"skipped":"recent"}));
        }
        if let Some(until) = state["retryAfter"].as_u64().filter(|until| *until > now) {
            return Ok(json!({"started":false,"skipped":"rateLimited","retryAfter":until}));
        }
        if shared.network().refuses_check() {
            if let Ok(Some(_lock)) = check::check_lock(shared) {
                check::record(
                    shared,
                    &state,
                    &check::Fetched::Offline,
                    check::Trigger::Manual,
                );
            }
            return Ok(json!({"started":false,"skipped":"offline"}));
        }
        if !self.spawn_check(check::Trigger::Manual) {
            return Err(ApiError::tr(
                "failed",
                lang,
                "update.winpoint.err.connect",
                crate::i18n::NO_PARAMS,
            ));
        }
        Ok(json!({"started":true}))
    }

    /// `winpoint.status`: the files only, in every host.
    fn status(&self, lang: Lang) -> Value {
        let empty = json!({});
        let shared = match self.open() {
            Ok(shared) => shared,
            Err(blocked) => {
                let (mode, fault, current) = match (blocked, &self.seam) {
                    (Blocked::Cache(failure), Some(seam)) => (
                        self.mode_name(None),
                        Some(failure.json()),
                        current_of(seam, install_prefix().as_deref(), None, None),
                    ),
                    _ => (
                        "unsupported",
                        None,
                        Version::parse(crate::VERSION).expect("the crate version is SemVer"),
                    ),
                };
                let cache = fault.is_some();
                return compose(&View {
                    mode,
                    owner_running: if cache { None } else { Some(false) },
                    checking: false,
                    fault,
                    state: &empty,
                    job: None,
                    current: &current,
                    installed: if cache { installed_version() } else { None },
                    arch: &feed::machine(),
                    auto: cache && auto_check_of(&self.ctx),
                    dry: self.dry,
                    python: job::python_present(),
                    exe_dir: exe_dir(),
                    lang,
                });
            }
        };
        if self.mode() == Mode::Owner {
            // a job the last owner left unfinished ends here (it never starts a program)
            job::reattach(&shared, false);
        }
        let state = shared.read_state();
        let record = shared.store.read(job::JOB).filter(Value::is_object);
        let owner = shared.store.held(store::OWNER_LOCK);
        let checking = shared.store.held(check::CHECK_LOCK);
        let fault = [(store::OWNER_LOCK, &owner), (check::CHECK_LOCK, &checking)]
            .into_iter()
            .find_map(|(name, held)| {
                let error = held.as_ref().err()?;
                Some(check::Failure::cache(shared.store.dir(), Some(name), error).json())
            });
        let current = shared.current();
        compose(&View {
            mode: self.mode_name(Some(&shared)),
            owner_running: owner.ok(),
            checking: checking.unwrap_or(false),
            fault,
            state: &state,
            job: record.as_ref(),
            current: &current,
            installed: shared.installed_version(),
            arch: &shared.arch,
            auto: shared.auto_check(),
            dry: self.dry,
            python: job::python_present(),
            exe_dir: exe_dir(),
            lang,
        })
    }
    /// `mode` of a status with a seam: `dry`, `installed` (this host runs from install.py's payload) or `dev`.
    fn mode_name(&self, shared: Option<&Shared>) -> &'static str {
        let prefix = match shared {
            Some(shared) => shared.prefix(),
            None => install_prefix(),
        };
        if self.dry {
            "dry"
        } else if prefix.is_some() {
            "installed"
        } else {
            "dev"
        }
    }

    pub(crate) fn shutdown(&self) {
        if self.stopping.swap(true, Ordering::SeqCst) {
            return;
        }
        let shared = self
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(shared) = shared {
            shared.stop.store(true, Ordering::SeqCst);
            // a job before its installer ends within 200 ms (http.rs watchdog); an installer goes on (reattach)
            if let Some(running) = shared
                .job
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                running.stop.store(true, Ordering::SeqCst);
            }
            shared.notifier.shutdown();
        }
        self.owner_lock
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if self.mode() == Mode::Owner {
            *self.mode.lock().unwrap_or_else(|e| e.into_inner()) = Mode::Reader;
        }
    }
}
