//! Process-local cooperative copy/move controls. No signals, global pause flag or persisted paths.
use super::{ApiError, ApiResult, Context};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Condvar, Mutex},
};

const MAX_ACTIVE: usize = 32;
const MAX_RECENT: usize = 128;

#[derive(Default)]
struct RegistryState {
    closing: bool,
    jobs: BTreeMap<String, Arc<Job>>,
}

#[derive(Default)]
pub(super) struct Registry(Mutex<RegistryState>);

struct State {
    phase: &'static str,
    pause: bool,
    cancel: bool,
    conflict: Option<Conflict>,
    policy: Option<Choice>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Choice {
    Replace,
    Skip,
    KeepBoth,
    Cancel,
    Retry,
}
impl Choice {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "replace" => Some(Self::Replace),
            "skip" => Some(Self::Skip),
            "keepBoth" => Some(Self::KeepBoth),
            "cancel" => Some(Self::Cancel),
            "retry" => Some(Self::Retry),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::Skip => "skip",
            Self::KeepBoth => "keepBoth",
            Self::Cancel => "cancel",
            Self::Retry => "retry",
        }
    }
}
struct Conflict {
    id: String,
    choices: Vec<String>,
    answer: Option<(Choice, bool)>,
}

pub(super) struct Job {
    id: String,
    state: Mutex<State>,
    wake: Condvar,
}

fn terminal(phase: &str) -> bool {
    matches!(phase, "finished" | "cancelled" | "failed")
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:.".contains(&b))
}

fn error(ctx: &Context, code: &str, key: &str) -> ApiError {
    ApiError::tr(code, ctx.lang(), key, crate::i18n::NO_PARAMS)
}

impl Registry {
    pub fn register(&self, id: &str, ctx: &Context) -> Result<Arc<Job>, ApiError> {
        if !valid_id(id) {
            return Err(error(ctx, "bad_args", "files.transferBadControl"));
        }
        let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if registry.closing {
            return Err(error(ctx, "offline", "files.shuttingDown"));
        }
        if registry.jobs.get(id).is_some_and(|job| job.active()) {
            return Err(error(ctx, "busy", "files.transferDuplicate"));
        }
        if registry.jobs.values().filter(|job| job.active()).count() >= MAX_ACTIVE {
            return Err(error(ctx, "busy", "files.transferLimit"));
        }
        // Retain a bounded number of terminal answers so an in-flight last control is harmless.
        while registry.jobs.len() >= MAX_RECENT {
            let old = registry
                .jobs
                .iter()
                .find(|(_, job)| !job.active())
                .map(|(id, _)| id.clone());
            let Some(old) = old else { break };
            registry.jobs.remove(&old);
        }
        let job = Arc::new(Job {
            id: id.into(),
            state: Mutex::new(State {
                phase: "running",
                pause: false,
                cancel: false,
                conflict: None,
                policy: None,
            }),
            wake: Condvar::new(),
        });
        registry.jobs.insert(id.into(), job.clone());
        Ok(job)
    }

    pub fn call(&self, args: &Value, ctx: &Context, dry_run: bool) -> ApiResult {
        let id = args["jobId"].as_str().filter(|id| valid_id(id));
        let action = args["action"]
            .as_str()
            .filter(|a| matches!(*a, "pause" | "resume" | "cancel"));
        let (Some(id), Some(action)) = (id, action) else {
            return Err(error(ctx, "bad_args", "files.transferBadControl"));
        };
        // A preview must not create a job, change flags or even require a real running transfer.
        if dry_run {
            return Ok(json!({"ok":true,"dryRun":true,"jobId":id,"action":action}));
        }
        let job = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| error(ctx, "not_found", "files.transferGone"))?;
        job.request(action);
        let mut result = job.json();
        result["ok"] = json!(true);
        Ok(result)
    }

    pub fn shutdown(&self) {
        let jobs = {
            let mut registry = self.0.lock().unwrap_or_else(|e| e.into_inner());
            registry.closing = true;
            registry.jobs.values().cloned().collect::<Vec<_>>()
        };
        for job in jobs {
            job.request("cancel");
        }
    }

    pub fn resolve(&self, args: &Value, ctx: &Context, dry_run: bool) -> ApiResult {
        let id = args["jobId"].as_str().filter(|id| valid_id(id));
        let conflict = args["conflictId"].as_str().filter(|id| valid_id(id));
        let choice = args["choice"].as_str().and_then(Choice::parse);
        if args.get("applyToAll").is_some_and(|v| !v.is_boolean()) {
            return Err(error(ctx, "bad_args", "files.transferBadControl"));
        }
        let (Some(id), Some(conflict), Some(choice)) = (id, conflict, choice) else {
            return Err(error(ctx, "bad_args", "files.transferBadControl"));
        };
        if dry_run {
            return Ok(json!({"ok":true,"dryRun":true,"jobId":id,"conflictId":conflict}));
        }
        let job = self
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .get(id)
            .cloned()
            .ok_or_else(|| error(ctx, "not_found", "files.transferGone"))?;
        let mut state = job.state.lock().unwrap_or_else(|e| e.into_inner());
        let pending = state
            .conflict
            .as_mut()
            .filter(|c| c.id == conflict && c.answer.is_none())
            .ok_or_else(|| error(ctx, "stale", "files.transferConflictGone"))?;
        if !pending.choices.iter().any(|c| c == choice.name()) {
            return Err(error(ctx, "bad_args", "files.transferBadControl"));
        }
        if choice == Choice::Retry && args["applyToAll"] == true {
            return Err(error(ctx, "bad_args", "files.transferBadControl"));
        }
        pending.answer = Some((choice, args["applyToAll"] == true));
        if choice == Choice::Cancel {
            state.cancel = true;
            state.pause = false;
        }
        job.wake.notify_all();
        drop(state);
        let mut out = job.json();
        out["ok"] = json!(true);
        out["conflictId"] = json!(conflict);
        Ok(out)
    }
}

impl Job {
    fn active(&self) -> bool {
        !terminal(self.state.lock().unwrap_or_else(|e| e.into_inner()).phase)
    }

    pub(super) fn request(&self, action: &str) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !terminal(state.phase) {
            match action {
                "cancel" => {
                    state.cancel = true;
                    state.pause = false;
                }
                "pause" if !state.cancel => state.pause = true,
                "resume" => state.pause = false,
                _ => {}
            }
        }
        self.wake.notify_all();
    }

    pub fn json(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let active = !terminal(state.phase);
        let phase = if !active {
            state.phase
        } else if state.cancel {
            "cancelling"
        } else if state.pause && state.phase != "paused" {
            "pausing"
        } else {
            state.phase
        };
        json!({"jobId":self.id,"state":phase,"paused":state.phase == "paused" && !state.cancel,
            "cancelled":state.phase == "cancelled","canPause":active && !state.cancel && !matches!(state.phase,"committing"|"waitingConflict"),
            "canCancel":active && !state.cancel})
    }

    pub fn checkpoint(&self, mut changed: impl FnMut()) -> io::Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if state.cancel {
                return Err(cancelled());
            }
            if !state.pause {
                if state.phase == "paused" {
                    state.phase = "running";
                    drop(state);
                    changed();
                }
                return Ok(());
            }
            if state.phase != "paused" {
                state.phase = "paused";
                drop(state);
                changed();
                state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                continue;
            }
            state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Once admitted, an individual rename/source-removal commit cannot be interrupted halfway.
    /// Requests arriving inside it stay pending and are observed at the next checkpoint.
    pub fn begin_commit(self: &Arc<Self>, mut changed: impl FnMut()) -> io::Result<Commit> {
        loop {
            self.checkpoint(&mut changed)?;
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.pause || state.cancel {
                continue;
            }
            state.phase = "committing";
            drop(state);
            return Ok(Commit(self.clone()));
        }
    }

    pub fn finish(&self, phase: &'static str) {
        debug_assert!(terminal(phase));
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.phase = phase;
        state.pause = false;
        state.conflict = None;
        self.wake.notify_all();
    }

    /// Used at the last pre-delete boundary. A request during verification restarts admission,
    /// so resuming always revalidates both complete trees before any source is removed.
    pub fn pending(&self) -> bool {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.pause || state.cancel
    }

    pub fn conflict(
        &self,
        mut payload: Value,
        choices: &[&str],
        force_question: bool,
        mut emit: impl FnMut(Value),
    ) -> io::Result<Choice> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.cancel {
            return Err(cancelled());
        }
        if !force_question
            && let Some(policy) = state.policy
            && choices.contains(&policy.name())
        {
            return Ok(policy);
        }
        let id = format!("conflict-{}", crate::events::nonce());
        payload["jobId"] = json!(self.id);
        payload["conflictId"] = json!(id);
        payload["choices"] = json!(choices);
        state.conflict = Some(Conflict {
            id,
            choices: choices.iter().map(|c| (*c).into()).collect(),
            answer: None,
        });
        state.phase = "waitingConflict";
        drop(state);
        emit(payload);
        state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if state.cancel {
                state.conflict = None;
                state.phase = "running";
                return Err(cancelled());
            }
            if let Some((choice, all)) = state.conflict.as_mut().and_then(|c| c.answer.take()) {
                if all && choice != Choice::Retry {
                    state.policy = Some(choice);
                }
                state.conflict = None;
                state.phase = "running";
                return Ok(choice);
            }
            state = self.wake.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
}

pub(super) struct Commit(Arc<Job>);
impl Drop for Commit {
    fn drop(&mut self) {
        self.0.state.lock().unwrap_or_else(|e| e.into_inner()).phase = "running";
        self.0.wake.notify_all();
    }
}

pub(super) fn cancelled() -> io::Error {
    io::Error::from_raw_os_error(libc::ECANCELED)
}
pub(super) fn is_cancelled(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ECANCELED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::PathBuf, sync::mpsc, time::Duration};

    fn ctx() -> Context {
        Context::new(
            PathBuf::from("/tmp"),
            PathBuf::from("/tmp/Desktop"),
            "en".into(),
            false,
        )
    }

    #[test]
    fn control_registry_is_bounded_unique_per_instance_and_dry_run_is_inert() {
        let ctx = ctx();
        let registry = Registry::default();
        assert!(
            registry
                .call(&json!({"jobId":"absent","action":"pause"}), &ctx, true)
                .unwrap()["dryRun"]
                == true
        );
        assert!(registry.0.lock().unwrap().jobs.is_empty());
        let first = registry.register("one", &ctx).unwrap();
        assert!(registry.register("one", &ctx).is_err());
        assert!(Registry::default().register("one", &ctx).is_ok());
        for i in 1..MAX_ACTIVE {
            registry.register(&format!("other-{i}"), &ctx).unwrap();
        }
        assert!(registry.register("overflow", &ctx).is_err());
        registry
            .call(&json!({"jobId":"one","action":"pause"}), &ctx, true)
            .unwrap();
        assert_eq!(first.json()["state"], "running");
        first.finish("finished");
        assert!(registry.register("one", &ctx).is_ok());
        for job in registry.0.lock().unwrap().jobs.values() {
            job.finish("finished");
        }
        for i in 0..MAX_RECENT * 2 {
            let job = registry.register(&format!("recent-{i}"), &ctx).unwrap();
            job.finish("finished");
        }
        assert!(registry.0.lock().unwrap().jobs.len() <= MAX_RECENT);
    }

    #[test]
    fn paused_checkpoint_wakes_on_resume_cancel_and_shutdown() {
        let ctx = ctx();
        for action in ["resume", "cancel", "shutdown"] {
            let registry = Arc::new(Registry::default());
            let job = registry.register(action, &ctx).unwrap();
            job.request("pause");
            let (tx, rx) = mpsc::channel();
            let worker = job.clone();
            let handle = std::thread::spawn(move || {
                worker.checkpoint(|| {
                    tx.send(()).unwrap();
                })
            });
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            assert_eq!(job.json()["state"], "paused");
            if action == "shutdown" {
                registry.shutdown();
            } else {
                job.request(action);
            }
            let result = handle.join().unwrap();
            assert_eq!(result.is_ok(), action == "resume");
            if action != "resume" {
                assert!(is_cancelled(&result.unwrap_err()));
            }
        }
    }

    #[test]
    fn cancellation_waits_until_an_individual_move_commit_has_finished() {
        let ctx = ctx();
        let job = Registry::default().register("commit", &ctx).unwrap();
        let commit = job.begin_commit(|| {}).unwrap();
        job.request("cancel");
        assert_eq!(job.json()["state"], "cancelling");
        drop(commit);
        assert!(is_cancelled(&job.checkpoint(|| {}).unwrap_err()));
        job.finish("cancelled");
        job.request("resume");
        assert_eq!(job.json()["state"], "cancelled");
        assert_eq!(job.json()["canCancel"], false);
    }

    #[test]
    fn conflict_answers_bind_exact_question_allowlist_and_dry_run_never_answers() {
        let registry = Arc::new(Registry::default());
        let context = ctx();
        let job = registry.register("question", &context).unwrap();
        let worker = job.clone();
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            worker.conflict(
                json!({"kind":"collision"}),
                &["replace", "skip", "keepBoth", "cancel"],
                false,
                |payload| tx.send(payload).unwrap(),
            )
        });
        let question = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(job.json()["state"], "waitingConflict");
        assert_eq!(job.json()["canPause"], false);
        assert_eq!(
            registry
                .resolve(
                    &json!({"jobId":"question","conflictId":"wrong","choice":"replace"}),
                    &context,
                    false
                )
                .unwrap_err()
                .code,
            "stale"
        );
        assert!(registry.resolve(&json!({"jobId":"question","conflictId":question["conflictId"],"choice":"retry"}),&context,false).is_err());
        assert_eq!(registry.resolve(&json!({"jobId":"question","conflictId":question["conflictId"],"choice":"replace"}),&context,true).unwrap()["dryRun"],true);
        assert_eq!(job.json()["state"], "waitingConflict");
        registry.resolve(&json!({"jobId":"question","conflictId":question["conflictId"],"choice":"keepBoth","applyToAll":true}),&context,false).unwrap();
        assert_eq!(handle.join().unwrap().unwrap(), Choice::KeepBoth);
        assert!(registry.resolve(&json!({"jobId":"question","conflictId":question["conflictId"],"choice":"replace"}),&context,false).is_err());
        assert_eq!(
            job.conflict(json!({}), &["keepBoth", "skip"], false, |_| panic!(
                "apply-to-all should answer"
            ))
            .unwrap(),
            Choice::KeepBoth
        );
        let resolver = registry.clone();
        let context = ctx();
        assert_eq!(job.conflict(json!({}),&["replace","skip"],true,move|payload|{
            resolver.resolve(&json!({"jobId":"question","conflictId":payload["conflictId"],"choice":"skip"}),&context,false).unwrap();
        }).unwrap(),Choice::Skip);
    }

    #[test]
    fn waiting_conflict_wakes_on_job_cancel_and_shutdown_without_an_answer() {
        for shutdown in [false, true] {
            let registry = Arc::new(Registry::default());
            let context = ctx();
            let job = registry.register("waiting", &context).unwrap();
            let worker = job.clone();
            let (tx, rx) = mpsc::channel();
            let handle = std::thread::spawn(move || {
                worker.conflict(json!({}), &["skip", "cancel"], false, |payload| {
                    tx.send(payload).unwrap()
                })
            });
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
            if shutdown {
                registry.shutdown();
            } else {
                registry
                    .call(
                        &json!({"jobId":"waiting","action":"cancel"}),
                        &context,
                        false,
                    )
                    .unwrap();
            }
            assert!(is_cancelled(&handle.join().unwrap().unwrap_err()));
        }
    }
}
