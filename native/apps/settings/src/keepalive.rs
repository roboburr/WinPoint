//! The update keep-alive of the native Settings (host services/update.rs `hold_application`, design §4.1 C2):
//! while an update job of our own runs, closing the window keeps the process, so the job's owner, this Settings'
//! backend (`win11-app --app settings --http-only`, which dies with the process through PR_SET_PDEATHSIG), finishes
//! it. The backend has no GApplication to hold, so Settings holds its own: it follows `update.changed`,
//! `update.owner` and (until it holds) `update.progress`, and reads `update.status`.
//!
//! Held while our backend is the owner (`mode` "owner") and a job of ours is there (`job`, not `foreign`) that does
//! not wait for a confirm (the WebKit host held no keep-alive during a confirm either, review F7); after the job
//! until its re-check answered (`refreshing`), at most 60 s (pk.rs HOLD_AFTER_JOB); then 1 s more, so the job's
//! notification has left the backend. While held a re-check runs every 5 s as well (an event may be throttled).
//! A failed `update.status` (the backend is gone, and with it the job) lets go.
//!
//! The backend is told whether a window is shown (`update.setWatching`, at every window added or removed and to
//! each new backend): its notification of the job's outcome (host update/pk.rs `notify`: "installed" or "failed"
//! only when nobody watches) cannot go by our event stream, which stays open while the keep-alive holds the
//! process without a window.
use kit::prelude::*;
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// After a job: the hold lasts until its re-check answered, at most this long.
const AFTER_JOB: Duration = Duration::from_secs(60);
/// While held: a re-check this often.
const POLL: Duration = Duration::from_secs(5);
/// The hold ends this long after the job and its re-check.
const GRACE: Duration = Duration::from_secs(1);

/// What `update.status` says about the keep-alive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Want {
    /// a job of ours runs: hold
    Job,
    /// no job, but a check runs (a job's re-check, when we held for the job)
    Check,
    /// nothing of ours runs
    Nothing,
}

/// The keep-alive's reading of an `update.status` answer.
pub fn want(s: &Value) -> Want {
    if s["mode"] != "owner" {
        return Want::Nothing;
    }
    let job = &s["job"];
    if job.is_object() && job["foreign"] != true {
        let confirm = !job["confirm"].is_null() && job["confirm"] != false;
        return if confirm { Want::Nothing } else { Want::Job };
    }
    if s["refreshing"] == true { Want::Check } else { Want::Nothing }
}

#[derive(Default)]
struct State {
    hold: Option<gio::ApplicationHoldGuard>,
    /// the job we held for has ended; its re-check runs since then
    after_job: Option<Instant>,
    scheduled: bool,
    /// what the backend was last told (`update.setWatching`)
    told: Option<bool>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn held() -> bool {
    STATE.with(|s| s.borrow().hold.is_some())
}

/// Start following the update service (once, at the first launch).
pub fn install(ctx: &kit::Ctx) {
    for prefix in ["update.changed", "update.owner", "backend.ready"] {
        ctx.on(prefix, |_, _| schedule(Duration::from_millis(150)));
    }
    // a new backend knows nothing yet
    ctx.on("backend.ready", |_, _| tell_windows(true));
    ctx.app().connect_window_added(|_, _| tell_windows(false));
    ctx.app().connect_window_removed(|_, _| tell_windows(false));
    // a job's progress: only until we hold for it (then update.changed and the poll suffice)
    ctx.on("update.progress", |_, _| {
        if !held() {
            schedule(Duration::from_millis(150));
        }
    });
    schedule(Duration::from_secs(1));
}

/// Tell the backend whether a window is shown, when that changed (or `again`: to a new backend).
fn tell_windows(again: bool) {
    let Some(ctx) = kit::try_ctx() else { return };
    let shown = !ctx.app().windows().is_empty();
    let changed = STATE.with(|s| s.borrow_mut().told.replace(shown) != Some(shown));
    if changed || again {
        ctx.fire("update.setWatching", json!({ "on": shown }));
    }
}

fn schedule(after: Duration) {
    let first = STATE.with(|s| !std::mem::replace(&mut s.borrow_mut().scheduled, true));
    if first {
        glib::timeout_add_local_once(after, check);
    }
}

fn check() {
    STATE.with(|s| s.borrow_mut().scheduled = false);
    let Some(ctx) = kit::try_ctx() else { return };
    ctx.call("update.status", json!({}), |result| {
        apply(match result {
            Ok(s) => want(&s),
            Err(_) => Want::Nothing,
        })
    });
}

fn apply(want: Want) {
    let Some(ctx) = kit::try_ctx() else { return };
    let released = STATE.with(|s| {
        let mut s = s.borrow_mut();
        match want {
            Want::Job => {
                if s.hold.is_none() {
                    s.hold = Some(ctx.app().hold());
                }
                s.after_job = None;
                None
            }
            Want::Check if s.hold.is_some() && s.after_job.get_or_insert_with(Instant::now).elapsed() < AFTER_JOB => None,
            _ => {
                s.after_job = None;
                s.hold.take()
            }
        }
    });
    if let Some(hold) = released {
        glib::timeout_add_local_once(GRACE, move || drop(hold));
    }
    if held() {
        schedule(POLL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keep_alive_reads_the_snapshot_as_the_host_holds() {
        let job = json!({"jobId":"j","status":8,"foreign":false,"confirm":null});
        assert_eq!(want(&json!({"mode":"owner","job":job})), Want::Job);
        assert_eq!(want(&json!({"mode":"mirror","job":job})), Want::Nothing, "another Settings owns the job");
        assert_eq!(want(&json!({"mode":"reader","job":job})), Want::Nothing);
        assert_eq!(want(&json!({"mode":"owner","job":{"jobId":"j","foreign":true}})), Want::Nothing, "not ours");
        assert_eq!(
            want(&json!({"mode":"owner","job":{"jobId":"j","confirm":{"removals":["x"]}}})),
            Want::Nothing,
            "no keep-alive during a confirm (review F7)"
        );
        assert_eq!(want(&json!({"mode":"owner","job":{"jobId":"j","confirm":false}})), Want::Job);
        assert_eq!(want(&json!({"mode":"owner","job":null,"refreshing":true})), Want::Check, "the re-check");
        assert_eq!(want(&json!({"mode":"owner","job":null,"refreshing":false})), Want::Nothing);
        assert_eq!(want(&json!({"mode":"dry","job":job})), Want::Nothing);
        assert_eq!(want(&Value::Null), Want::Nothing);
    }
}
