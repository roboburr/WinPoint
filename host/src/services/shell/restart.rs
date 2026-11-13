//! `shell.restartAll` (notes/claude/windows-update-gnome.md §5, GX1): restart every Win11 process.
//!
//! * The origin asks `update/guard.rs::restart_all_blocked` (an upgrader, an own update job in any phase, an active
//!   Software job) and then emits `shell.restartAll {nonce, origin, at, reason}` to every host (and to itself).
//! * Every host re-execs itself through its own `request_shutdown(true)` after a per-kind delay, so the screen
//!   stays usable: the apps 300 ms → panel 1200 ms (`win11-panel --http-only`, win11-shell's backend) → the origin
//!   2000 ms. We never signal another process. win11-file-portal exits instead (D-Bus activation brings the new
//!   build on the next request).
//! * Just before its exec a receiver re-checks the update job and its own Software jobs. If one is active it
//!   defers until that job ends: it listens to the job's events (`software.changed`, `update.changed`,
//!   `update.owner`) and to the job owner's exit (a pidfd), never polls, and announces the deferral as
//!   `shell.restartDeferred`. WU-FIX (D-G53): Explorer's wait for its own file operations (lib.rs
//!   `defer_while_busy`, `files.idle`) is announced the same way (`wait:'files'`), and a receiver whose deferred
//!   restart runs after all says so with `shell.restartResumed` right before its exec, so the update owner scans
//!   the restart state again and the Settings InfoBar goes away (GX1 review n5).
//! * One restart per process: while this process takes part in a restart (scheduled, deferred or under way),
//!   a second `shell.restartAll` answers that restart's nonce with `pending:true` and sends no second event.
use super::NativeCore;
use crate::{
    files::{ApiError, ApiResult},
    services::update::{guard, store},
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
    thread,
    time::Duration,
};

/// The event every host acts on, and the one a receiver sends when it waits for a job of its own.
pub(super) const EVENT: &str = "shell.restartAll";
pub(super) const DEFERRED: &str = "shell.restartDeferred";
/// WU-FIX (GX1 review n5): a deferred restart runs after all (the update owner rescans after the exec).
pub(super) const RESUMED: &str = crate::services::update::restart::RESUMED_EVENT;
/// Events that can end what a deferred receiver waits for.
const RESUME: [&str; 3] = ["software.changed", "update.changed", "update.owner"];
/// The Software job states that keep a restart waiting (`software.rs` `active()`).
const SOFTWARE_ACTIVE: [&str; 3] = ["queued", "running", "authenticating"];
const MAX_JOB_FILE: u64 = 1 << 20;
/// Software job files read per check (the guard's own bound).
const MAX_JOB_FILES: usize = 500;

/// What one host does with the event (design §5).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Plan {
    /// win11-file-portal: exit, D-Bus activation starts the new build on the next request
    Exit,
    /// re-exec after this delay
    Exec(Duration),
}

/// The per-kind plan. The origin goes last (2000 ms) whatever its kind; a second Settings window is an app
/// like the others.
pub(crate) fn plan(kind: &str, origin: bool) -> Plan {
    match kind {
        "picker" => Plan::Exit,
        _ if origin => Plan::Exec(Duration::from_millis(2000)),
        "panel" => Plan::Exec(Duration::from_millis(1200)),
        // explorer, taskmgr, alles, disks, defaultapps, installer, a Settings that is not the origin
        _ => Plan::Exec(Duration::from_millis(300)),
    }
}

/// `{reason:'libraries'|'update'}` and nothing else.
pub(crate) fn reason(args: &Value) -> Option<&'static str> {
    let object = args.as_object()?;
    if object.keys().any(|key| key != "reason") {
        return None;
    }
    match object.get("reason")?.as_str()? {
        "libraries" => Some("libraries"),
        "update" => Some("update"),
        _ => None,
    }
}

/// What a receiver waits for before its exec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wait {
    /// our update job (`update/job.json`) is active; its owner's pid
    Update(Option<u32>),
    /// a Software job of this process is queued, running or authenticating; its owner's pid
    Software(u32),
}
impl Wait {
    fn code(self) -> &'static str {
        match self {
            Wait::Update(_) => "update",
            Wait::Software(_) => "software",
        }
    }
    /// the job's owner when it is another process: its exit (a pidfd) ends the wait too
    fn other_owner(self, me: u32) -> Option<u32> {
        match self {
            Wait::Update(Some(owner)) | Wait::Software(owner) if owner != me => Some(owner),
            _ => None,
        }
    }
}
/// The guard's rule for an update job (`restart_all_blocked`): no `finishedAt` and a live owner.
pub(crate) fn update_job_active(job: &Value, alive: impl Fn(&Value) -> bool) -> bool {
    job.get("finishedAt").is_none_or(Value::is_null) && alive(&job["owner"])
}
/// The owner of a Software job that is still active (`software.rs` `active()`, owner alive), if it is one of
/// `owners`.
pub(crate) fn software_job_owner(
    job: &Value,
    owners: &[u32],
    alive: impl Fn(&Value) -> bool,
) -> Option<u32> {
    let owner = job["_owner"]["pid"]
        .as_u64()
        .and_then(|p| u32::try_from(p).ok())?;
    (SOFTWARE_ACTIVE.contains(&job["state"].as_str().unwrap_or(""))
        && owners.contains(&owner)
        && alive(&job["_owner"]))
    .then_some(owner)
}
fn read_job(path: &Path) -> Option<Value> {
    store::read_private(path, MAX_JOB_FILE)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
}
fn pid_of(owner: &Value) -> Option<u32> {
    owner["pid"].as_u64().and_then(|p| u32::try_from(p).ok())
}
/// The re-check just before the exec: the update job files, then the Software jobs of `owners` (this process).
/// Files only.
/// `exited` are job owners whose pidfd reported their exit: they are gone even while they are still zombies.
pub(crate) fn wait_for(cache: &Path, owners: &[u32], exited: &[u32]) -> Option<Wait> {
    let gone = |owner: &Value| pid_of(owner).is_some_and(|pid| exited.contains(&pid));
    if let Some(job) = read_job(&cache.join("update").join(store::JOB))
        && update_job_active(&job, |owner| store::owner_alive(owner) && !gone(owner))
    {
        return Some(Wait::Update(pid_of(&job["owner"])));
    }
    let entries = std::fs::read_dir(cache.join("software").join("jobs")).ok()?;
    entries
        .flatten()
        .take(MAX_JOB_FILES)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| read_job(&path))
        .find_map(|job| {
            software_job_owner(&job, owners, |owner| {
                store::owner_alive(owner) && !gone(owner)
            })
        })
        .map(Wait::Software)
}
/// WU-FIX (D-G53): what a wait code waits for, in the log line.
fn waited_for(wait: &str) -> String {
    match wait {
        "files" => "file operation".into(),
        code => format!("{code} job"),
    }
}
/// At most this many exited owners are remembered (one restart rarely waits for more than one or two).
const MAX_EXITED: usize = 8;

/// Shared state of one host's restart: at most one restart per process.
#[derive(Default)]
pub(super) struct State {
    /// the nonce of the event this process accepted (scheduled, deferred or under way); later events are
    /// ignored until the exec
    scheduled: Option<String>,
    /// the nonce this process sent as the origin; a second call answers it again instead of a second event
    sent: Option<String>,
    /// a restart that waits for a job to end
    deferred: Option<Pending>,
    /// the job owner whose exit is watched (one pidfd thread per owner)
    watching: Option<u32>,
    /// job owners that have exited (their pidfd became readable)
    exited: Vec<u32>,
    /// the nonce and the wait whose deferral was announced (once, not again on every re-check)
    announced: Option<(String, &'static str)>,
}
impl State {
    /// the restart this process already takes part in
    fn under_way(&self) -> Option<&String> {
        self.scheduled.as_ref().or(self.sent.as_ref())
    }
}
#[derive(Clone, Debug)]
struct Pending {
    nonce: String,
    origin: bool,
    plan: Plan,
}

impl NativeCore {
    /// `shell.restartAll` (origin side).
    pub(super) fn restart_all(&self, args: &Value) -> ApiResult {
        let lang = self.ctx.lang();
        let reason = reason(args).ok_or_else(|| super::bad_args(lang))?;
        // A restart this process already takes part in (a second click within the origin's 2 s, or while a job
        // defers it): the same restart again, no second event, so no host restarts twice (review M1).
        if let Some(nonce) = self.restart_under_way() {
            return Ok(json!({"ok":true,"nonce":nonce,"pending":true}));
        }
        // files and /proc only (no D-Bus, no program), so it runs in any host and in dry-run as well
        if let Some(error) = guard::restart_all_blocked(&self.ctx) {
            return Err(error);
        }
        if self.ctx.dry_run {
            return Ok(json!({"ok":true,"dryRun":true}));
        }
        if self.stopped.load(Ordering::Relaxed) {
            return Err(ApiError::tr(
                "shutting_down",
                lang,
                "shell.shuttingDown",
                crate::i18n::NO_PARAMS,
            ));
        }
        let nonce = crate::events::nonce();
        {
            let mut state = self.restart.lock().unwrap_or_else(|e| e.into_inner());
            // two calls at the same moment: the first one's restart
            if let Some(nonce) = state.under_way() {
                return Ok(json!({"ok":true,"nonce":nonce,"pending":true}));
            }
            state.sent = Some(nonce.clone());
        }
        self.ctx.emit(
            EVENT,
            json!({"nonce":nonce,"origin":std::process::id(),"at":store::now_ms(),"reason":reason}),
        );
        Ok(json!({"ok":true,"nonce":nonce}))
    }
    fn restart_under_way(&self) -> Option<String> {
        self.restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .under_way()
            .cloned()
    }

    /// Every host (NativeCore::new): act on the event; resume a deferred restart on its job's events.
    pub(super) fn watch_restart_all(self: &Arc<Self>) {
        if self.ctx.kind == "picker" && !self.ctx.dry_run {
            // win11-file-portal: count its FileChooser calls from the first one on (portal.rs registers its
            // backend right after this), so the event never ends a request that has no window yet
            crate::count_portal_calls();
        }
        let weak = Arc::downgrade(self);
        self.ctx.events.on(
            EVENT,
            Arc::new(move |data| {
                if let Some(core) = weak.upgrade() {
                    core.restart_event(data);
                }
            }),
        );
        for event in RESUME {
            let weak = Arc::downgrade(self);
            self.ctx.events.on(
                event,
                Arc::new(move |_| {
                    if let Some(core) = weak.upgrade() {
                        core.resume_restart();
                    }
                }),
            );
        }
    }

    fn restart_event(self: &Arc<Self>, data: &Value) {
        // a dry-run host never restarts (nothing real happens in dry-run, design §4.11)
        if self.ctx.dry_run || self.stopped.load(Ordering::Relaxed) {
            return;
        }
        let Some(nonce) = data["nonce"]
            .as_str()
            .filter(|n| !n.is_empty() && n.len() <= 128)
        else {
            return;
        };
        let origin = data["origin"].as_u64() == Some(u64::from(std::process::id()));
        let plan = plan(&self.ctx.kind, origin);
        let delay = match plan {
            Plan::Exit => Duration::ZERO,
            Plan::Exec(delay) => delay,
        };
        {
            let mut state = self.restart.lock().unwrap_or_else(|e| e.into_inner());
            if state.scheduled.is_some() {
                return;
            }
            state.scheduled = Some(nonce.into());
        }
        let pending = Pending {
            nonce: nonce.into(),
            origin,
            plan,
        };
        let core = self.clone();
        let _ = thread::Builder::new()
            .name("win11-restart-all".into())
            .spawn(move || {
                thread::sleep(delay);
                core.restart_when_free(pending);
            });
    }

    /// The re-check just before the exec (design §5): defer while a job runs, else restart now.
    fn restart_when_free(self: &Arc<Self>, pending: Pending) {
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        if pending.plan == Plan::Exit {
            crate::quit_when_idle();
            return;
        }
        let me = std::process::id();
        let owners = [me];
        let exited = self
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .exited
            .clone();
        let Some(wait) = wait_for(&self.ctx.cache_dir, &owners, &exited) else {
            // WU-FIX (GX1 review n5): a restart that waited runs now
            self.announce_resumed(&pending.nonce, None);
            self.restart_now(&pending);
            return;
        };
        let watch = wait.other_owner(me);
        let nonce = pending.nonce.clone();
        {
            let mut state = self.restart.lock().unwrap_or_else(|e| e.into_inner());
            state.deferred = Some(pending);
            if let Some(owner) = watch
                && state.watching != Some(owner)
            {
                state.watching = Some(owner);
                self.watch_owner(owner);
            }
        }
        self.announce_wait(&nonce, wait.code());
        // A job that ended between the check and storing `deferred` sent its event before anyone waited.
        if wait_for(&self.ctx.cache_dir, &owners, &exited).is_none() {
            self.resume_restart();
        }
    }

    // WU-FIX (D-G53, GX1 review n5): every wait is announced the same way, once per restart and wait, and so is
    // its end.
    /// A wait of this restart: one log line and `shell.restartDeferred {nonce, pid, kind, wait}`, once.
    fn announce_wait(&self, nonce: &str, wait: &'static str) {
        let first = {
            let mut state = self.restart.lock().unwrap_or_else(|e| e.into_inner());
            let first = state
                .announced
                .as_ref()
                .is_none_or(|(n, w)| n != nonce || *w != wait);
            state.announced = Some((nonce.to_owned(), wait));
            first
        };
        if first {
            eprintln!(
                "Win11 restart: {} waits for its {} before it restarts",
                self.ctx.kind,
                waited_for(wait)
            );
            self.ctx.emit(
                DEFERRED,
                json!({"nonce":nonce,"pid":std::process::id(),"kind":self.ctx.kind,"wait":wait}),
            );
        }
    }
    /// The announced wait (`only` that one, or any) has ended and this process restarts now: one log line and
    /// `shell.restartResumed {nonce, pid, kind, wait}`, so the update owner rescans after the exec.
    fn announce_resumed(&self, nonce: &str, only: Option<&'static str>) {
        let ended = {
            let mut state = self.restart.lock().unwrap_or_else(|e| e.into_inner());
            match state.announced.take() {
                Some((n, w)) if n == nonce && only.is_none_or(|o| o == w) => Some(w),
                other => {
                    state.announced = other;
                    None
                }
            }
        };
        if let Some(wait) = ended {
            eprintln!(
                "Win11 restart: {} restarts now (its {} ended)",
                self.ctx.kind,
                waited_for(wait)
            );
            self.ctx.emit(
                RESUMED,
                json!({"nonce":nonce,"pid":std::process::id(),"kind":self.ctx.kind,"wait":wait}),
            );
        }
    }
    // WU-FIX end

    fn resume_restart(self: &Arc<Self>) {
        let Some(pending) = self
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .deferred
            .take()
        else {
            return;
        };
        let core = self.clone();
        let _ = thread::Builder::new()
            .name("win11-restart-all".into())
            .spawn(move || core.restart_when_free(pending));
    }

    /// The job owner's exit ends its job too (a crash sends no event): one thread blocks on its pidfd.
    fn watch_owner(self: &Arc<Self>, pid: u32) {
        let weak = Arc::downgrade(self);
        let _ = thread::Builder::new()
            .name("win11-restart-wait".into())
            .spawn(move || {
                let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::pid_t, 0) };
                let gone = if fd < 0 {
                    // already gone (ESRCH) resumes; any other failure leaves it to the job's events
                    std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
                } else {
                    let fd = fd as libc::c_int;
                    let mut poll = libc::pollfd {
                        fd,
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    loop {
                        let n = unsafe { libc::poll(&mut poll, 1, -1) };
                        if n >= 0
                            || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR)
                        {
                            break;
                        }
                    }
                    unsafe { libc::close(fd) };
                    true
                };
                if let Some(core) = weak.upgrade() {
                    {
                        let mut state = core.restart.lock().unwrap_or_else(|e| e.into_inner());
                        if state.watching == Some(pid) {
                            state.watching = None;
                        }
                        if gone && !state.exited.contains(&pid) {
                            if state.exited.len() >= MAX_EXITED {
                                state.exited.remove(0);
                            }
                            state.exited.push(pid);
                        }
                    }
                    if gone {
                        core.resume_restart();
                    }
                }
            });
    }

    fn restart_now(self: &Arc<Self>, pending: &Pending) {
        // An app window maps focused again, like a launch from Start (no "is ready" notice); the origin goes
        // last, so it ends in front. Best effort: without a token GNOME maps the window unfocused.
        let kind = self.ctx.kind.as_str();
        let token = super::APPS
            .contains(&kind)
            .then(|| crate::launch::activation_token(&json!({"app":kind})))
            .flatten();
        let weak = Arc::downgrade(self);
        let nonce = pending.nonce.clone();
        // WU-FIX (D-G53): lib.rs decides Explorer's wait for its file operations; it is announced here like the
        // others (wait 'files')
        let (files_core, files_nonce) = (weak.clone(), nonce.clone());
        crate::on_restart_wait(Some(Arc::new(move |waiting: bool| {
            if let Some(core) = files_core.upgrade() {
                if waiting {
                    core.announce_wait(&files_nonce, "files");
                } else {
                    core.announce_resumed(&files_nonce, Some("files"));
                }
            }
        })));
        crate::prepare_restart_all(
            crate::RestartAll {
                origin: pending.origin,
                token,
            },
            Some(Box::new(move || {
                if let Some(core) = weak.upgrade() {
                    core.restart_dropped(&nonce);
                }
            })),
        );
        (self.request_shutdown)(true);
    }

    /// The main thread did not restart this process after all: an app host without a window (Settings kept
    /// alive by its update job's hold, design C2) has no window to reopen and ends on its own, and its next
    /// start runs the new build. Not lost without a trace (review m2): one log line and `shell.restartDeferred
    /// {wait:'exit'}`, and the state is cleared, so a later `shell.restartAll` is honoured again.
    fn restart_dropped(&self, nonce: &str) {
        {
            let mut state = self.restart.lock().unwrap_or_else(|e| e.into_inner());
            state.scheduled = None;
            state.sent = None;
            state.deferred = None;
            state.announced = None;
            state.exited.clear();
        }
        eprintln!(
            "Win11 restart: {} has no window; it keeps running until its work ends and starts the new build next time",
            self.ctx.kind
        );
        self.ctx.emit(
            DEFERRED,
            json!({"nonce":nonce,"pid":std::process::id(),"kind":self.ctx.kind,"wait":"exit"}),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::shell::tests::{finish, fixture};
    use std::{
        fs,
        os::unix::fs::{DirBuilderExt, OpenOptionsExt},
        path::PathBuf,
        sync::{Mutex, mpsc},
    };

    fn ms(n: u64) -> Plan {
        Plan::Exec(Duration::from_millis(n))
    }

    #[test]
    fn delays_follow_the_design_order() {
        for kind in [
            "explorer",
            "taskmgr",
            "alles",
            "disks",
            "defaultapps",
            "installer",
        ] {
            assert_eq!(plan(kind, false), ms(300), "{kind}");
        }
        assert_eq!(
            plan("settings", false),
            ms(300),
            "a Settings that is not the origin"
        );
        assert_eq!(plan("panel", false), ms(1200));
        assert_eq!(
            plan("settings", true),
            ms(2000),
            "the origin goes last"
        );
        assert_eq!(
            plan("taskmgr", true),
            ms(2000),
            "any origin goes last"
        );
        assert_eq!(
            plan("picker", false),
            Plan::Exit,
            "win11-file-portal exits"
        );
        // the order of design §5 holds for every pair
        let order = [
            plan("explorer", false),
            plan("panel", false),
            plan("settings", true),
        ];
        assert!(
            order
                .windows(2)
                .all(|w| matches!((w[0], w[1]), (Plan::Exec(a), Plan::Exec(b)) if a < b))
        );
    }

    #[test]
    fn reason_is_libraries_or_update_and_nothing_else() {
        assert_eq!(reason(&json!({"reason":"libraries"})), Some("libraries"));
        assert_eq!(reason(&json!({"reason":"update"})), Some("update"));
        for bad in [
            json!({}),
            json!({"reason":"x"}),
            json!({"reason":1}),
            json!({"reason":"update","x":1}),
            json!([]),
            Value::Null,
        ] {
            assert_eq!(reason(&bad), None, "{bad}");
        }
    }

    #[test]
    fn job_rules_match_the_guard() {
        let alive = |_: &Value| true;
        let dead = |_: &Value| false;
        for phase in [
            json!({"lastStatus":30}),
            json!({"lastStatus":8}),
            json!({"committing":true}),
            json!({"confirm":{"removals":["x"]}}),
        ] {
            let mut job = phase.clone();
            job["owner"] = json!({"pid":1,"start":"1"});
            assert!(update_job_active(&job, alive), "{phase}");
            assert!(!update_job_active(&job, dead), "dead owner {phase}");
            job["finishedAt"] = json!(5);
            assert!(!update_job_active(&job, alive), "finished {phase}");
            job["finishedAt"] = Value::Null;
            assert!(update_job_active(&job, alive), "finishedAt null {phase}");
        }
        for state in SOFTWARE_ACTIVE {
            let job = json!({"state":state,"_owner":{"pid":42,"start":"9"}});
            assert_eq!(software_job_owner(&job, &[42], alive), Some(42), "{state}");
            assert_eq!(
                software_job_owner(&job, &[43], alive),
                None,
                "another process's job {state}"
            );
            assert_eq!(
                software_job_owner(&job, &[43, 42], alive),
                Some(42),
                "one of the owners {state}"
            );
            assert_eq!(
                software_job_owner(&job, &[42], dead),
                None,
                "dead owner {state}"
            );
        }
        for state in ["completed", "failed", "cancelled", "interrupted", ""] {
            assert_eq!(
                software_job_owner(&json!({"state":state,"_owner":{"pid":42}}), &[42], alive),
                None,
                "{state}"
            );
        }
        assert_eq!(
            software_job_owner(&json!({"state":"running","_owner":{}}), &[42], alive),
            None,
            "no owner pid"
        );
    }

    fn private(dir: &Path) {
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .unwrap();
    }
    fn write_private(path: &Path, value: &Value) {
        let _ = fs::remove_file(path);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .unwrap();
        std::io::Write::write_all(&mut file, value.to_string().as_bytes()).unwrap();
    }
    fn me() -> Value {
        store::current_owner()
    }

    #[test]
    fn the_receiver_recheck_reads_the_update_job_and_only_its_own_software_jobs() {
        let root =
            std::env::temp_dir().join(format!("win11-restart-wait-{}", crate::events::nonce()));
        let (update, jobs) = (root.join("update"), root.join("software/jobs"));
        private(&update);
        private(&jobs);
        let pid = std::process::id();
        assert_eq!(wait_for(&root, &[pid], &[]), None, "nothing runs");
        write_private(
            &update.join("job.json"),
            &json!({"jobId":"a","owner":me(),"lastStatus":8}),
        );
        assert_eq!(wait_for(&root, &[pid], &[]), Some(Wait::Update(Some(pid))));
        assert_eq!(
            wait_for(&root, &[pid], &[pid]),
            None,
            "an owner whose pidfd reported its exit"
        );
        write_private(
            &update.join("job.json"),
            &json!({"jobId":"a","owner":me(),"finishedAt":9}),
        );
        assert_eq!(wait_for(&root, &[pid], &[]), None, "finished");
        write_private(
            &update.join("job.json"),
            &json!({"jobId":"a","owner":{"pid":pid,"start":"0"}}),
        );
        assert_eq!(
            wait_for(&root, &[pid], &[]),
            None,
            "owner pid reused (other start time)"
        );
        // another live process (/bin/sleep, a child of this test): its job is not ours, unless it is one of the
        // owners asked for
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let other = child.id();
        write_private(
            &jobs.join("j1.json"),
            &json!({"state":"running","_owner":{"pid":other,"start":store::process_start(other)}}),
        );
        assert_eq!(
            wait_for(&root, &[pid], &[]),
            None,
            "another process's Software job"
        );
        assert_eq!(
            wait_for(&root, &[pid, other], &[]),
            Some(Wait::Software(other)),
            "one of the owners"
        );
        assert_eq!(Wait::Software(other).other_owner(pid), Some(other));
        assert_eq!(
            wait_for(&root, &[pid, other], &[other]),
            None,
            "its pidfd reported its exit"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(
            wait_for(&root, &[pid, other], &[]),
            None,
            "its owner is gone"
        );
        write_private(
            &jobs.join("j2.json"),
            &json!({"state":"authenticating","_owner":me()}),
        );
        assert_eq!(wait_for(&root, &[pid], &[]), Some(Wait::Software(pid)));
        assert_eq!(Wait::Software(pid).other_owner(pid), None);
        write_private(
            &jobs.join("j2.json"),
            &json!({"state":"completed","_owner":me()}),
        );
        assert_eq!(wait_for(&root, &[pid], &[]), None);
        // a group-readable job file is not ours to trust (store::read_private)
        let path = jobs.join("j3.json");
        write_private(&path, &json!({"state":"running","_owner":me()}));
        fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
        assert_eq!(wait_for(&root, &[pid], &[]), None, "not a private file");
        let _ = fs::remove_dir_all(root);
    }

    fn cache(root: &Path) -> PathBuf {
        root.join("cache")
    }
    fn observe(core: &Arc<NativeCore>) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        core.ctx.events.on(
            EVENT,
            Arc::new(move |data| {
                let _ = tx.lock().unwrap().send(data.clone());
            }),
        );
        rx
    }

    #[test]
    fn restart_all_is_refused_by_the_guard_in_every_job_phase_and_for_software_jobs() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (root, core) = fixture(false);
        let events = observe(&core);
        let lang = core.ctx.lang();
        let update = cache(&root).join("update");
        let jobs = cache(&root).join("software/jobs");
        private(&update);
        private(&jobs);
        let call = |args: Value| core.call("shell.restartAll", &args).unwrap();
        // design §3.7 phases of an own job: waiting, downloading, a confirm pause, installing (committing)
        for phase in [
            json!({"lastStatus":30}),
            json!({"lastStatus":8}),
            json!({"confirm":{"removals":["x"]},"lastStatus":8}),
            json!({"lastStatus":9,"committing":true}),
        ] {
            let mut job = phase.clone();
            job["jobId"] = json!("j");
            job["owner"] = me();
            write_private(&update.join("job.json"), &job);
            let error = call(json!({"reason":"libraries"})).unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.key.as_deref()),
                ("update_busy", Some("update.guard.restart")),
                "{phase}"
            );
            assert_eq!(
                error.message,
                crate::i18n::trf(
                    lang,
                    "update.guard.restart",
                    &json!({"product":crate::i18n::tr(lang, "update.product")})
                )
            );
        }
        fs::remove_file(update.join("job.json")).unwrap();
        write_private(
            &jobs.join("s.json"),
            &json!({"state":"queued","_owner":me()}),
        );
        let error = call(json!({"reason":"update"})).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.key.as_deref()),
            ("software_busy", Some("update.guard.software"))
        );
        assert!(events.try_recv().is_err(), "a refusal emits nothing");
        // the English texts come from the same rows
        assert_eq!(
            crate::i18n::tr(crate::i18n::Lang::En, "update.guard.software"),
            "An app is still being installed or removed. Try again afterwards."
        );
        fs::remove_file(jobs.join("s.json")).unwrap();
        // bad arguments
        assert_eq!(call(json!({})).unwrap_err().code, "bad_args");
        assert_eq!(call(json!({"reason":"now"})).unwrap_err().code, "bad_args");
        // free: the event goes out (the origin receives it too) with the nonce the call returns
        let ok = call(json!({"reason":"libraries"})).unwrap();
        assert_eq!(ok["ok"], true);
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event["nonce"], ok["nonce"]);
        assert_eq!(event["origin"], json!(std::process::id()));
        assert_eq!(event["reason"], "libraries");
        assert!(event["at"].as_u64().is_some_and(|at| at > 0));
        finish(root, core);
    }

    /// A host of a kind without a delay of its own ("desktop", no longer a surface: 300 ms as a receiver, 2000 ms
    /// as the origin) and without an activation token (that is asked only for the app kinds), whose
    /// `request_shutdown` reports to the returned channel.
    fn receiver(name: &str) -> (PathBuf, Arc<NativeCore>, mpsc::Receiver<bool>) {
        let root =
            std::env::temp_dir().join(format!("win11-restart-{name}-{}", crate::events::nonce()));
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = crate::services::ServiceContext::new(
            events,
            root.join("config"),
            root.join("cache"),
            root.join("ui"),
            "desktop".into(),
            false,
        )
        .unwrap();
        let (tx, restarts) = mpsc::channel();
        let tx = Mutex::new(tx);
        let core = NativeCore::build(
            ctx,
            Arc::new(move |again| {
                let _ = tx.lock().unwrap().send(again);
            }),
            false,
        )
        .unwrap();
        core.watch_restart_all();
        (root, core, restarts)
    }

    #[test]
    fn a_second_restart_all_while_one_is_under_way_sends_no_second_event() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (root, core, restarts) = receiver("twice");
        let events = observe(&core);
        let call = |core: &Arc<NativeCore>| {
            core.call("shell.restartAll", &json!({"reason":"libraries"}))
                .unwrap()
                .unwrap()
        };
        let first = call(&core);
        assert_eq!(first["ok"], true);
        assert!(first.get("pending").is_none());
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(event["nonce"], first["nonce"]);
        // a double click: the same restart, no second event (review M1)
        let second = call(&core);
        assert_eq!(
            (&second["ok"], &second["nonce"], &second["pending"]),
            (&json!(true), &first["nonce"], &json!(true))
        );
        // several calls at the same moment while it is under way: still that one restart
        let answers: Vec<Value> = thread::scope(|scope| {
            let handles: Vec<_> = (0..4).map(|_| scope.spawn(|| call(&core))).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(
            answers
                .iter()
                .all(|a| a["nonce"] == first["nonce"] && a["pending"] == true)
        );
        assert!(
            events.recv_timeout(Duration::from_millis(300)).is_err(),
            "one event"
        );
        // the origin restarts once, after its 2000 ms; until the exec a call is still that restart
        assert!(restarts.recv_timeout(Duration::from_millis(1000)).is_err());
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        assert!(restarts.recv_timeout(Duration::from_millis(300)).is_err());
        assert_eq!(call(&core)["pending"], true);
        assert!(crate::take_restart_all().is_some_and(|plan| plan.origin));
        finish(root, core);
    }

    #[test]
    fn calls_at_the_same_moment_send_one_event() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        // no receiver here (NativeCore::build without start): only the origin's own bookkeeping decides
        let (root, core) = fixture(false);
        let events = observe(&core);
        let answers: Vec<Value> = thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        core.call("shell.restartAll", &json!({"reason":"update"}))
                            .unwrap()
                            .unwrap()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let event = events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(
            events.recv_timeout(Duration::from_millis(300)).is_err(),
            "one event"
        );
        assert!(
            answers
                .iter()
                .all(|a| a["ok"] == true && a["nonce"] == event["nonce"])
        );
        assert_eq!(
            answers
                .iter()
                .filter(|a| a.get("pending").is_none())
                .count(),
            1,
            "one call sent it, the others answered pending"
        );
        finish(root, core);
    }

    #[test]
    fn a_host_that_does_not_restart_after_all_says_so_and_takes_a_later_restart() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (root, core, restarts) = receiver("dropped");
        let (deferred_tx, deferred) = mpsc::channel();
        let deferred_tx = Mutex::new(deferred_tx);
        core.ctx.events.on(
            DEFERRED,
            Arc::new(move |data| {
                let _ = deferred_tx.lock().unwrap().send(data.clone());
            }),
        );
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n1","origin":1,"at":1}));
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        // the main thread finds no window (lib.rs restart_on_main, an app host kept alive by a hold)
        assert!(crate::drop_restart_all(), "a plan to drop");
        let note = deferred.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            (note["nonce"].as_str(), note["wait"].as_str()),
            (Some("n1"), Some("exit"))
        );
        assert!(core.restart.lock().unwrap().under_way().is_none());
        assert_eq!(crate::take_restart_all(), None, "the plan is gone");
        // a later restartAll is honoured again (it was ignored for good before, review m2)
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n2","origin":1,"at":2}));
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        assert!(crate::take_restart_all().is_some());
        assert!(!crate::drop_restart_all(), "nothing left to drop");
        finish(root, core);
    }

    #[test]
    fn dry_run_answers_without_an_event_but_still_shows_a_refusal() {
        let (root, core) = fixture(true);
        let events = observe(&core);
        assert_eq!(
            core.call("shell.restartAll", &json!({"reason":"libraries"}))
                .unwrap()
                .unwrap(),
            json!({"ok":true,"dryRun":true})
        );
        assert!(events.try_recv().is_err());
        let jobs = cache(&root).join("software/jobs");
        private(&jobs);
        write_private(
            &jobs.join("s.json"),
            &json!({"state":"running","_owner":me()}),
        );
        assert_eq!(
            core.call("shell.restartAll", &json!({"reason":"libraries"}))
                .unwrap()
                .unwrap_err()
                .code,
            "software_busy"
        );
        finish(root, core);
    }

    #[test]
    fn a_receiver_with_its_own_job_defers_until_the_job_event_and_then_restarts_once() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root =
            std::env::temp_dir().join(format!("win11-restart-defer-{}", crate::events::nonce()));
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        // "desktop": 300 ms, and no activation token (that is only asked for the app kinds)
        let ctx = crate::services::ServiceContext::new(
            events,
            root.join("config"),
            root.join("cache"),
            root.join("ui"),
            "desktop".into(),
            false,
        )
        .unwrap();
        let (tx, restarts) = mpsc::channel();
        let tx = Mutex::new(tx);
        let core = NativeCore::build(
            ctx,
            Arc::new(move |again| {
                let _ = tx.lock().unwrap().send(again);
            }),
            false,
        )
        .unwrap();
        core.watch_restart_all();
        let (deferred_tx, deferred) = mpsc::channel();
        let deferred_tx = Mutex::new(deferred_tx);
        core.ctx.events.on(
            DEFERRED,
            Arc::new(move |data| {
                let _ = deferred_tx.lock().unwrap().send(data.clone());
            }),
        );
        // WU-FIX (GX1 review n5): the restart that waited says so before its exec
        let resumed = notes(&core, RESUMED);
        let jobs = cache(&root).join("software/jobs");
        private(&jobs);
        write_private(
            &jobs.join("own.json"),
            &json!({"state":"running","_owner":me()}),
        );
        // an event from another origin: the re-check finds this process's own job and defers
        core.ctx.events.emit_local(
            EVENT,
            json!({"nonce":"n1","origin":1,"at":1,"reason":"libraries"}),
        );
        let note = deferred.recv_timeout(Duration::from_secs(3)).unwrap();
        assert_eq!(
            (
                note["nonce"].as_str(),
                note["wait"].as_str(),
                note["kind"].as_str()
            ),
            (Some("n1"), Some("software"), Some("desktop"))
        );
        assert_eq!(note["pid"], json!(std::process::id()));
        assert!(
            restarts.recv_timeout(Duration::from_millis(400)).is_err(),
            "no restart while the job runs"
        );
        // another job's event (ours still runs) changes nothing; a second restartAll is ignored
        core.ctx
            .events
            .emit_local("software.changed", json!({"jobId":"other"}));
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n2","origin":1,"at":2}));
        assert!(restarts.recv_timeout(Duration::from_millis(700)).is_err());
        assert!(
            deferred.try_recv().is_err(),
            "the deferral is announced once"
        );
        assert!(
            resumed.try_recv().is_err(),
            "not resumed while the job runs"
        );
        // the job ends: the file alone changes nothing (no poll, review n2) ...
        write_private(
            &jobs.join("own.json"),
            &json!({"state":"completed","_owner":me()}),
        );
        assert!(
            restarts.recv_timeout(Duration::from_millis(1200)).is_err(),
            "no restart without the job's event"
        );
        // ... its event resumes the restart, exactly once
        core.ctx.events.emit_local(
            "software.changed",
            json!({"jobId":"own","state":"completed"}),
        );
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        assert!(
            restarts.recv_timeout(Duration::from_millis(500)).is_err(),
            "one restart"
        );
        let note = resumed.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(
            (
                note["nonce"].as_str(),
                note["wait"].as_str(),
                note["kind"].as_str()
            ),
            (Some("n1"), Some("software"), Some("desktop"))
        );
        assert_eq!(note["pid"], json!(std::process::id()));
        assert!(resumed.try_recv().is_err(), "resumed once");
        assert_eq!(
            crate::take_restart_all(),
            Some(crate::RestartAll {
                origin: false,
                token: None
            })
        );
        finish(root, core);
    }

    // WU-FIX (D-G53, GX1 review n5)
    /// Every `event` this host sends or receives.
    fn notes(core: &Arc<NativeCore>, event: &str) -> mpsc::Receiver<Value> {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        core.ctx.events.on(
            event,
            Arc::new(move |data| {
                let _ = tx.lock().unwrap().send(data.clone());
            }),
        );
        rx
    }

    /// Explorer's wait for its file operations is decided in lib.rs (defer_while_busy) after this restart's plan is
    /// set; it is announced like the other waits (once, `wait:'files'`), and so is its end, right before the exec.
    #[test]
    fn a_wait_that_lib_rs_decides_is_announced_like_the_others_and_so_is_its_end() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (root, core, restarts) = receiver("files");
        let (deferred, resumed) = (notes(&core, DEFERRED), notes(&core, RESUMED));
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n1","origin":1,"at":1}));
        // restart_now ran (nothing in the job files): the plan and the announcer are set
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        assert!(deferred.try_recv().is_err() && resumed.try_recv().is_err());
        // lib.rs: a file operation runs (twice: a second check announces nothing new)
        crate::note_restart_wait(true);
        crate::note_restart_wait(true);
        let note = deferred.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            (
                note["nonce"].as_str(),
                note["wait"].as_str(),
                note["kind"].as_str()
            ),
            (Some("n1"), Some("files"), Some("desktop"))
        );
        assert_eq!(note["pid"], json!(std::process::id()));
        assert!(
            deferred.recv_timeout(Duration::from_millis(300)).is_err(),
            "announced once"
        );
        assert!(resumed.try_recv().is_err());
        // files.idle: the exec runs now
        crate::note_restart_wait(false);
        let note = resumed.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            (note["nonce"].as_str(), note["wait"].as_str()),
            (Some("n1"), Some("files"))
        );
        crate::note_restart_wait(false);
        assert!(
            resumed.recv_timeout(Duration::from_millis(300)).is_err(),
            "its end once"
        );
        // a restart that never waited sends neither event
        assert!(crate::take_restart_all().is_some());
        crate::on_restart_wait(None);
        finish(root, core);
        let (root, core, restarts) = receiver("plain");
        let (deferred, resumed) = (notes(&core, DEFERRED), notes(&core, RESUMED));
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n2","origin":1,"at":1}));
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        assert!(
            deferred.recv_timeout(Duration::from_millis(300)).is_err()
                && resumed.try_recv().is_err()
        );
        let _ = crate::take_restart_all();
        crate::on_restart_wait(None);
        finish(root, core);
    }
    // WU-FIX end

    #[test]
    fn an_update_job_of_another_process_defers_until_that_process_exits() {
        let _serial = crate::RESTART_TESTS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let root =
            std::env::temp_dir().join(format!("win11-restart-owner-{}", crate::events::nonce()));
        let events = crate::events::Events::new(&root.join("events")).unwrap();
        let ctx = crate::services::ServiceContext::new(
            events,
            root.join("config"),
            root.join("cache"),
            root.join("ui"),
            "desktop".into(),
            false,
        )
        .unwrap();
        let (tx, restarts) = mpsc::channel();
        let tx = Mutex::new(tx);
        let core = NativeCore::build(
            ctx,
            Arc::new(move |again| {
                let _ = tx.lock().unwrap().send(again);
            }),
            false,
        )
        .unwrap();
        core.watch_restart_all();
        // the owner: a child of this test that sleeps (/bin/sleep is not a guarded program)
        let mut owner = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = owner.id();
        let update = cache(&root).join("update");
        private(&update);
        write_private(
            &update.join("job.json"),
            &json!({"jobId":"j","owner":{"pid":pid,"start":store::process_start(pid)},"lastStatus":8}),
        );
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n","origin":1,"at":1}));
        assert!(
            restarts.recv_timeout(Duration::from_millis(600)).is_err(),
            "deferred while the owner lives"
        );
        assert_eq!(core.restart.lock().unwrap().watching, Some(pid));
        // the owner dies without any event (a crash): its pidfd resumes the restart (also while it is a zombie)
        owner.kill().unwrap();
        assert_eq!(restarts.recv_timeout(Duration::from_secs(3)), Ok(true));
        owner.wait().unwrap();
        let _ = crate::take_restart_all();
        finish(root, core);
    }

    #[test]
    fn a_dry_host_ignores_the_event() {
        let (root, core) = fixture(true);
        core.watch_restart_all();
        core.ctx
            .events
            .emit_local(EVENT, json!({"nonce":"n","origin":1,"at":1}));
        let state = core.restart.lock().unwrap();
        assert!(
            state.scheduled.is_none() && state.deferred.is_none(),
            "nothing scheduled in a dry host"
        );
        drop(state);
        finish(root, core);
    }
}
