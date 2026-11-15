//! When the owner checks by itself (D-G74a): at most once a day, with jitter, never while a check is
//! refused by GitHub's rate limit, and only with `update.winpointAutoCheck` on. The timers that ask [`due`] are in
//! winpoint.rs `start` (one 3 to 10 minutes after the start, then hourly, which also covers a suspend and a late
//! network); the check thread itself skips an offline or metered link (check.rs).
use serde_json::Value;

pub(crate) const HOUR_MS: u64 = 3_600_000;
pub(crate) const DAY_MS: u64 = 24 * HOUR_MS;
/// The first automatic check: this many seconds after the owner starts, plus up to [`FIRST_JITTER_S`].
pub(crate) const FIRST_DELAY_S: u32 = 180;
pub(crate) const FIRST_JITTER_S: u32 = 420;
/// The first automatic check of a test seam's run (seam.rs): soon and fixed, so a harness scenario sees the
/// notification without a wait of minutes.
pub(crate) const TEST_FIRST_DELAY_S: u32 = 5;
pub(crate) const TICK_S: u32 = 3600;
/// The jitter added to a day after a success (spreads the machines over GitHub's hours).
pub(crate) const OK_JITTER_MS: u64 = 2 * HOUR_MS;
/// A manual check this soon after the last one answers `skipped:'recent'`.
pub(crate) const RECENT_MS: u64 = 30_000;

/// Is an automatic check due now? `auto` is the setting, `job_active` a running update job (phase 2).
/// `nextCheckAt` and `retryAfter` must have passed; a `checkedAt` more than an hour in the future, or a
/// `nextCheckAt` further away than any schedule sets, is a clock that jumped back: due.
pub(crate) fn due(now_ms: u64, state: &Value, auto: bool, job_active: bool) -> bool {
    if !auto || job_active {
        return false;
    }
    let field = |name: &str| state[name].as_u64();
    let jumped = field("checkedAt").is_some_and(|at| at > now_ms + HOUR_MS)
        || field("nextCheckAt").is_some_and(|at| at > now_ms + DAY_MS + OK_JITTER_MS + HOUR_MS)
        || field("retryAfter").is_some_and(|at| at > now_ms + DAY_MS + OK_JITTER_MS + HOUR_MS);
    if jumped {
        return true;
    }
    field("nextCheckAt").is_none_or(|at| now_ms >= at)
        && field("retryAfter").is_none_or(|at| now_ms >= at)
}

/// Seconds from the owner's start to its first automatic check: 3 minutes plus `jitter_s` (below
/// [`FIRST_JITTER_S`]), or [`TEST_FIRST_DELAY_S`] under a test seam.
pub(crate) fn first_delay_s(test_seam: bool, jitter_s: u32) -> u32 {
    if test_seam {
        TEST_FIRST_DELAY_S
    } else {
        FIRST_DELAY_S + jitter_s.min(FIRST_JITTER_S - 1)
    }
}

/// The next automatic check after a success: a day plus `jitter_ms` (below [`OK_JITTER_MS`]).
pub(crate) fn after_ok(now_ms: u64, jitter_ms: u64) -> u64 {
    now_ms + DAY_MS + jitter_ms.min(OK_JITTER_MS - 1)
}

/// The wait after the `failures`-th failure in a row: 6 h, 12 h, then 24 h.
pub(crate) fn backoff_ms(failures: u64) -> u64 {
    match failures {
        0 | 1 => 6 * HOUR_MS,
        2 => 12 * HOUR_MS,
        _ => DAY_MS,
    }
}

/// A random jitter in `0..max_ms` (GLib's random source; whole seconds).
pub(crate) fn jitter_ms(max_ms: u64) -> u64 {
    let seconds = i32::try_from(max_ms / 1000).unwrap_or(i32::MAX).max(1);
    glib::random_int_range(0, seconds) as u64 * 1000
}
