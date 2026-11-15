//! One update check of WinPoint (D-G74a): the feed request, what each answer does to `state.json`, and the
//! notification decision. Owner only, on a `winpoint-check` thread of its own, holding `check.lock`.
//!
//! `state.json` (in `$XDG_CACHE_HOME/win11-gnome/winpoint/`, 0700, files 0600, atomic writes):
//! `{schema:1, rev, checkedAt, okAt, nextCheckAt, retryAfter, failures, everOk,
//!   result:'ok'|'noReleases'|'unreachable'|'rateLimited'|'offline'|'error', error:{code,key,params,detail}|null,
//!   latest:{version,tag,name,publishedAt,notesUrl,assets:{tarball,sha256}|null}|null, notified:"V"|null,
//!   blocked:{version,code:'incompatible'|'manual'|'missingParts',params,detail}|null}`.
//! Times are ms since the epoch; texts are stored as keys and params only and rendered per call (winpoint.rs).
//! `notified` is the version that was announced, or that a manual check showed: an automatic check's notification
//! counts only once GNOME took it. `blocked` is a job's verdict for one version (job.rs, install.py `--check`); it
//! goes when another version is offered. A verdict about this PC (`missingParts`, `incompatible`: packages to install,
//! another GNOME or system libraries) also goes with a manual check, "Check for updates", so the card offers the
//! version again once the user fixed it, and the next click runs the release's `--check` afresh (it blocks again if
//! nothing changed); an automatic check keeps it (no notification for that version). `manual` is the release's own
//! (system files to change) and stays.
use super::{
    Shared, feed,
    http::{self, HttpError},
    schedule,
    version::Version,
};
use crate::services::update::store;
use serde_json::{Value, json};
use std::{io, path::Path, sync::atomic::Ordering, time::Duration};

pub(crate) const STATE: &str = "state.json";
pub(crate) const CHECK_LOCK: &str = "check.lock";
pub(crate) const ACCEPT: &str = "application/vnd.github+json";
/// The API list: 20 s without a byte, 60 s in all, 2 MiB.
pub(crate) const FEED_LIMITS: http::Limits = http::Limits {
    max_body: feed::MAX_FEED,
    stall: Duration::from_secs(20),
    deadline: Some(Duration::from_secs(60)),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trigger {
    /// the page's "Check for updates" (`winpoint.check`): no notification, only `notified`
    Manual,
    /// the daily schedule (schedule.rs): skipped offline or metered; may notify
    Auto,
}

/// A failed check as it is stored: a code, a fixed text and its params, and the raw detail for the expander.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Failure {
    pub code: &'static str,
    pub key: &'static str,
    pub params: Value,
    pub detail: String,
}
impl Failure {
    pub(crate) fn http(error: &HttpError) -> Self {
        let params = match error {
            HttpError::Status(code) => json!({"code": code}),
            _ => json!({}),
        };
        Self {
            code: error.code(),
            key: error.key(),
            params,
            detail: error.detail(),
        }
    }
    /// The state folder `dir`, or its file `name`, is not this user's alone (the store's checks: ours, 0700 or
    /// 0600, one link, no symbolic link): `{path}` is the folder, the detail names the file and the error.
    pub(crate) fn cache(dir: &Path, name: Option<&str>, error: &io::Error) -> Self {
        let what = name.map_or_else(|| dir.to_path_buf(), |name| dir.join(name));
        Self {
            code: "cache",
            key: "update.winpoint.err.cache",
            params: json!({"path": dir.display().to_string()}),
            detail: format!("{}: {error}", what.display()),
        }
    }
    fn feed(error: &feed::FeedError) -> Self {
        Self {
            code: "feed",
            key: "update.winpoint.err.feed",
            params: json!({}),
            detail: format!("{error:?}"),
        }
    }
    pub(crate) fn json(&self) -> Value {
        json!({"code":self.code,"key":self.key,"params":self.params,"detail":self.detail})
    }
}

/// What the feed request gave.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fetched {
    /// 200 and a parsed list
    Releases(Vec<feed::Release>),
    /// 404: no release yet, or (after an earlier 200) the repository is gone or private
    NotFound,
    /// 403 or 429 with GitHub's rate-limit headers: no request before `until_ms`
    RateLimited {
        until_ms: u64,
    },
    Failed(Failure),
    /// NetworkManager says there is no connectivity (a manual check; an automatic one never gets here)
    Offline,
}

/// GET the feed and classify the answer.
pub(crate) fn fetch(shared: &Shared, now_ms: u64) -> Fetched {
    let mut body = Vec::new();
    match shared.client.get(
        &shared.origin.feed_url(),
        ACCEPT,
        &FEED_LIMITS,
        &mut body,
        &shared.stop,
    ) {
        Ok(response) => classify(&response, &body, &shared.origin, &shared.arch, now_ms),
        Err(error) => Fetched::Failed(Failure::http(&error)),
    }
}

/// The answer to the list request: 200 parses, 404 is no release (or unreachable), a rate
/// limit waits, anything else is an error with its status code.
pub(crate) fn classify(
    response: &http::Response,
    body: &[u8],
    origin: &feed::Origin,
    arch: &str,
    now_ms: u64,
) -> Fetched {
    let seconds = |name: &str| {
        response
            .header(name)
            .and_then(|v| v.trim().parse::<u64>().ok())
    };
    match response.status {
        200 => match feed::parse_releases(body, origin, arch) {
            Ok(all) => Fetched::Releases(all),
            Err(error) => Fetched::Failed(Failure::feed(&error)),
        },
        404 => Fetched::NotFound,
        403 | 429
            if response.header("x-ratelimit-remaining").map(str::trim) == Some("0")
                || response.header("retry-after").is_some() =>
        {
            let reset = seconds("x-ratelimit-reset").map(|s| s.saturating_mul(1000));
            let retry = seconds("retry-after").map(|s| now_ms + s.saturating_mul(1000));
            let until = [reset, retry, Some(now_ms + schedule::HOUR_MS)]
                .into_iter()
                .flatten()
                .max()
                .unwrap_or(now_ms + schedule::HOUR_MS);
            Fetched::RateLimited { until_ms: until }
        }
        code => {
            let mut failure = Failure::http(&HttpError::Status(code));
            failure.detail = format!("HTTP {code} from {}", response.url);
            Fetched::Failed(failure)
        }
    }
}

/// `state.json` after an answer (pure: the time, this host's version and the jitter are given).
pub(crate) fn next_state(
    prev: &Value,
    fetched: &Fetched,
    now_ms: u64,
    current: &Version,
    jitter_ms: u64,
) -> Value {
    let keep = |name: &str| prev.get(name).cloned().unwrap_or(Value::Null);
    let failures = prev["failures"].as_u64().unwrap_or(0);
    let ever_ok = prev["everOk"] == true;
    let mut state = json!({
        "schema": 1, "rev": prev["rev"].as_u64().unwrap_or(0), "checkedAt": keep("checkedAt"), "okAt": keep("okAt"),
        "nextCheckAt": keep("nextCheckAt"), "retryAfter": keep("retryAfter"), "failures": failures,
        "everOk": ever_ok, "result": keep("result"), "error": keep("error"), "latest": keep("latest"),
        "notified": keep("notified"), "blocked": keep("blocked"),
    });
    let fixed = |code: &str, key: &str| json!({"code":code,"key":key,"params":{},"detail":""});
    match fetched {
        Fetched::Releases(all) => {
            state["latest"] = feed::choose(all, current)
                .map(feed::Release::json)
                .unwrap_or(Value::Null);
            state["result"] = json!("ok");
            state["everOk"] = json!(true);
            state["failures"] = json!(0);
            state["error"] = Value::Null;
            state["retryAfter"] = Value::Null;
            state["checkedAt"] = json!(now_ms);
            state["okAt"] = json!(now_ms);
            state["nextCheckAt"] = json!(schedule::after_ok(now_ms, jitter_ms));
        }
        Fetched::NotFound if ever_ok => {
            // the repository was there before: made private or renamed; keep the last known release
            state["result"] = json!("unreachable");
            state["error"] = fixed("unreachable", "update.winpoint.err.unreachable");
            state["retryAfter"] = Value::Null;
            state["checkedAt"] = json!(now_ms);
            state["nextCheckAt"] = json!(now_ms + schedule::DAY_MS);
        }
        Fetched::NotFound => {
            // no release published yet: shown as up to date
            state["result"] = json!("noReleases");
            state["latest"] = Value::Null;
            state["failures"] = json!(0);
            state["error"] = Value::Null;
            state["retryAfter"] = Value::Null;
            state["checkedAt"] = json!(now_ms);
            state["okAt"] = json!(now_ms);
            state["nextCheckAt"] = json!(schedule::after_ok(now_ms, jitter_ms));
        }
        Fetched::RateLimited { until_ms } => {
            state["result"] = json!("rateLimited");
            state["error"] = fixed("rateLimited", "update.winpoint.err.rateLimited");
            state["retryAfter"] = json!(until_ms);
            state["checkedAt"] = json!(now_ms);
            state["nextCheckAt"] = json!(until_ms);
        }
        Fetched::Failed(failure) => {
            state["result"] = json!("error");
            state["error"] = failure.json();
            state["failures"] = json!(failures + 1);
            state["retryAfter"] = Value::Null;
            state["checkedAt"] = json!(now_ms);
            state["nextCheckAt"] = json!(now_ms + schedule::backoff_ms(failures + 1));
        }
        Fetched::Offline => {
            // no request was made: the schedule and checkedAt stay
            state["result"] = json!("offline");
            state["error"] = fixed("offline", "update.winpoint.err.offline");
        }
    }
    state["blocked"] = blocked_for(&state["blocked"], &state["latest"]);
    state
}

/// Whether a check keeps a job's verdict (state.json `blocked`): an automatic one keeps every verdict, a manual one
/// only `manual` (the release's own), not what this PC can change (`missingParts`, `incompatible`).
pub(crate) fn verdict_kept(blocked: &Value, trigger: Trigger) -> bool {
    trigger == Trigger::Auto
        || !matches!(
            blocked["code"].as_str(),
            Some("missingParts" | "incompatible")
        )
}

/// A job's verdict stays only while its version is the one offered.
fn blocked_for(blocked: &Value, latest: &Value) -> Value {
    if !blocked.is_null()
        && blocked["version"].is_string()
        && blocked["version"] == latest["version"]
    {
        blocked.clone()
    } else {
        Value::Null
    }
}

/// The stored release when it is newer than `current` (the one the card offers).
pub(crate) fn newer_latest(state: &Value, current: &Version) -> Option<feed::Release> {
    feed::Release::from_json(&state["latest"]).filter(|r| r.version > *current)
}

/// Whether this check sends the notification for `version`: an automatic check with the setting on,
/// for a version not notified before. A check that finds it and sends nothing records `notified`, so a manual
/// check silences it; one that sends records it only once GNOME took the notification ([`record`]).
pub(crate) fn notify_now(state: &Value, version: &Version, trigger: Trigger, auto: bool) -> bool {
    trigger == Trigger::Auto && auto && state["notified"].as_str() != Some(&version.to_string())
}

/// One check, start to end (the `winpoint-check` thread). An automatic one first asks NetworkManager and skips an
/// offline or metered link without a trace; `check.lock` makes a second check at the same time a no-op, and a
/// `check.lock` that is not ours alone is a failed check (`cache`) on the card instead of no check ever.
pub(crate) fn run(shared: &Shared, trigger: Trigger) {
    if trigger == Trigger::Auto {
        let net = shared.network();
        if !net.online || net.metered {
            return;
        }
    }
    let _lock = match check_lock(shared) {
        Ok(Some(lock)) => lock,
        Ok(None) => return,
        Err(error) => {
            let failure = Failure::cache(shared.store.dir(), Some(CHECK_LOCK), &error);
            record(
                shared,
                &shared.read_state(),
                &Fetched::Failed(failure),
                trigger,
            );
            return;
        }
    };
    let prev = shared.read_state();
    // the card shows "checking" from here (readers see the held check.lock)
    shared.write_state(prev.clone());
    let fetched = fetch(shared, store::now_ms());
    if shared.stop.load(Ordering::SeqCst) {
        return;
    }
    record(shared, &prev, &fetched, trigger);
}

/// `check.lock`, `Ok(None)` while another check holds it, or the error of a lock file that is not ours alone. A
/// reader's `winpoint.status` takes the free lock for a moment to see whether it is held (`Store::held`), so a busy
/// lock is tried again for 200 ms before the check counts as running.
pub(crate) fn check_lock(shared: &Shared) -> io::Result<Option<store::FileLock>> {
    for _ in 0..10 {
        match shared.store.try_lock(CHECK_LOCK)? {
            Some(lock) => return Ok(Some(lock)),
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    Ok(None)
}

/// Write the answer's state and send the notification when it is due. The version counts as announced only when
/// the notification was shown (no session bus, no notification server: the next automatic check tries again);
/// a version a job blocked, or one a job is busy with, is not announced.
pub(crate) fn record(shared: &Shared, prev: &Value, fetched: &Fetched, trigger: Trigger) {
    let current = shared.current();
    let jitter = schedule::jitter_ms(schedule::OK_JITTER_MS);
    let mut next = next_state(prev, fetched, store::now_ms(), &current, jitter);
    if !verdict_kept(&next["blocked"], trigger) {
        next["blocked"] = Value::Null;
    }
    let busy = super::job::job_active(&shared.ctx.cache_dir);
    // a release without a download for this PC (yet) is shown on the card but announced only once it has one
    if let Some(release) = newer_latest(&next, &current).filter(|r| r.assets.is_some()) {
        let version = release.version.to_string();
        let blocked = next["blocked"]["version"] == version;
        let due = notify_now(&next, &release.version, trigger, shared.auto_check());
        if !due || blocked || busy || shared.notify(&release.version) {
            next["notified"] = json!(version);
        }
    }
    // a job may have blocked the offered version while this check ran: that verdict is newer than the check
    shared.update_state(|state| {
        if state["blocked"] != prev["blocked"] {
            next["blocked"] = blocked_for(&state["blocked"], &next["latest"]);
        }
        *state = next;
    });
}
