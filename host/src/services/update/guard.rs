//! The power and restart guard (design §4.9) and the release-upgrader detection every update write runs first
//! (§4.3, §6.4 step 5). Files and `/proc` only: no D-Bus, no program, so it runs in any host (the panel's
//! `system/power.rs` in reader mode, GX1's `shell.restartAll`). D-G74: WinPoint's own update job
//! (`services/winpoint/job.rs`) blocks power actions while its installer runs and `shell.restartAll` while it runs at
//! all or waits for a sign-out; it asks [`busy_for_winpoint`] before it starts.
use super::{
    paths::{Paths, Setup, process_setup},
    store::{self, JOB, Store},
};
use crate::{files::ApiError, services::ServiceContext};
use serde_json::{Value, json};
use std::{
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
};

/// A dead owner's unfinished job blocks power actions only while `job.json` is younger than this (the owner
/// rewrites it on every status change and at least every 60 s, design §4.9).
const DEAD_OWNER_WINDOW_MS: u64 = 10 * 60 * 1000;
/// The screen session Ubuntu's text frontend runs in (`DistUpgradeMain.py:164`).
pub(crate) const UPGRADE_SCREEN: &str = "ubuntu-release-upgrade-screen-window";

fn product(lang: crate::i18n::Lang) -> String {
    crate::i18n::tr(lang, "update.product")
}
fn seam_error(lang: crate::i18n::Lang) -> ApiError {
    ApiError::tr("test_seam", lang, "update.err.testSeam", crate::i18n::NO_PARAMS)
}

/// `reboot`, `shutdown`, `suspend` and `logout` (never `lock`) are refused while our job commits (owner alive,
/// or dead but its unfinished job.json is under 10 minutes old) and while Ubuntu's release upgrader runs.
/// A failed test seam refuses too (fail closed: a misconfigured test never powers the machine off).
pub(crate) fn power_blocked(ctx: &ServiceContext, action: &str) -> Option<ApiError> {
    if !matches!(action, "reboot" | "shutdown" | "suspend" | "logout") {
        return None;
    }
    let lang = ctx.lang();
    let setup = match process_setup() {
        Ok(setup) => setup,
        Err(_) => return Some(seam_error(lang)),
    };
    power_blocked_in(ctx, &setup, lang)
}
pub(crate) fn power_blocked_in(ctx: &ServiceContext, setup: &Setup, lang: crate::i18n::Lang) -> Option<ApiError> {
    if upgrader_running(&setup.paths) {
        return Some(ApiError::tr("update_busy", lang, "update.guard.upgrade", crate::i18n::NO_PARAMS));
    }
    // D-G74: WinPoint's installer replaces its own programs; a power action now would cut it off mid-way
    if crate::services::winpoint::job::installing(&ctx.cache_dir) {
        return Some(ApiError::tr("update_busy", lang, "update.winpoint.err.installing", crate::i18n::NO_PARAMS));
    }
    let store = Store::open(ctx.cache_dir.join("update")).ok()?;
    let job = store.read(JOB)?;
    let boot = store::boot_id(&setup.paths);
    let written = store.modified_ms(JOB).unwrap_or(0);
    job_blocks_power(&job, boot.as_deref(), store::owner_alive(&job["owner"]), written, store::now_ms())
        .then(|| ApiError::tr("update_busy", lang, "update.guard.power", crate::i18n::NO_PARAMS))
}
/// The job part of the power rule, pure: unfinished, this boot, committing, and the owner alive or the file
/// recent.
pub(crate) fn job_blocks_power(job: &Value, boot_id: Option<&str>, owner_alive: bool, written_ms: u64, now_ms: u64) -> bool {
    let unfinished = job.get("finishedAt").is_none_or(Value::is_null);
    let this_boot = boot_id.is_some() && job["bootId"].as_str() == boot_id;
    let recent = now_ms.saturating_sub(written_ms) < DEAD_OWNER_WINDOW_MS;
    unfinished && this_boot && job["committing"] == true && (owner_alive || recent)
}

/// `shell.restartAll` (GX1) is refused while the release upgrader runs, while any job of ours is active (any
/// phase: the Settings re-exec would end the owner), and while a Software job is queued/running/authenticating.
#[allow(dead_code)] // called by shell.restartAll (GX1)
pub(crate) fn restart_all_blocked(ctx: &ServiceContext) -> Option<ApiError> {
    let lang = ctx.lang();
    let setup = match process_setup() {
        Ok(setup) => setup,
        Err(_) => return Some(seam_error(lang)),
    };
    if upgrader_running(&setup.paths) {
        return Some(ApiError::tr("update_busy", lang, "update.guard.upgrade", crate::i18n::NO_PARAMS));
    }
    if let Ok(store) = Store::open(ctx.cache_dir.join("update"))
        && let Some(job) = store.read(JOB)
        && job.get("finishedAt").is_none_or(Value::is_null)
        && store::owner_alive(&job["owner"])
    {
        return Some(ApiError::tr(
            "update_busy",
            lang,
            "update.guard.restart",
            &json!({"product": product(lang)}),
        ));
    }
    if software_job_active(&ctx.cache_dir.join("software/jobs")) {
        return Some(ApiError::tr("software_busy", lang, "update.guard.software", crate::i18n::NO_PARAMS));
    }
    // D-G74: not while WinPoint updates itself, and not while its update waits for a sign-out (new shell binaries
    // against the old extension are the mix the sign-out avoids)
    if crate::services::winpoint::job::job_active(&ctx.cache_dir) {
        return Some(ApiError::tr("update_busy", lang, "update.winpoint.err.installing", crate::i18n::NO_PARAMS));
    }
    if crate::services::winpoint::job::relogin_pending(&ctx.cache_dir) {
        return Some(ApiError::tr("update_busy", lang, "update.winpoint.err.signOutFirst", crate::i18n::NO_PARAMS));
    }
    None
}

/// D-G74: what keeps WinPoint's own update from starting (`winpoint.install`, the code `busy`): Ubuntu's release
/// upgrader, an update job of ours (any phase, live owner) or an active Software job. A failed `WIN11_UPDATE_*` seam
/// counts as busy (fail closed).
pub(crate) fn busy_for_winpoint(ctx: &ServiceContext) -> bool {
    let Ok(setup) = process_setup() else { return true };
    if upgrader_running(&setup.paths) {
        return true;
    }
    if let Ok(store) = Store::open(ctx.cache_dir.join("update"))
        && let Some(job) = store.read(JOB)
        && job.get("finishedAt").is_none_or(Value::is_null)
        && store::owner_alive(&job["owner"])
    {
        return true;
    }
    software_job_active(&ctx.cache_dir.join("software/jobs"))
}
/// A Software job file (`software.rs` jobs dir) that is active with a live owner (its own `active()` rule).
pub(crate) fn software_job_active(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else { return false };
    entries.flatten().take(500).any(|entry| {
        let path = entry.path();
        path.extension().is_some_and(|e| e == "json")
            && store::read_private(&path, 1 << 20)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .is_some_and(|job| {
                    ["queued", "running", "authenticating"].contains(&job["state"].as_str().unwrap_or(""))
                        && store::owner_alive(&job["_owner"])
                })
    })
}

/// The NUL-separated arguments of one `/proc/<pid>/cmdline`.
fn cmdline(dir: &Path) -> Vec<String> {
    let mut bytes = Vec::new();
    if let Ok(file) = fs::File::open(dir.join("cmdline")) {
        use std::io::Read;
        let _ = file.take(64 << 10).read_to_end(&mut bytes);
    }
    bytes
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}
fn base(arg: &str) -> &str {
    arg.rsplit('/').next().unwrap_or(arg)
}
/// Is this argument vector Ubuntu's release upgrader (design §6.4 step 5)? Only these (review F12):
/// * a python interpreter whose script is `do-release-upgrade` or the unpacked upgrader
///   `/tmp/ubuntu-release-upgrader-*/<codename>` (what the kernel shows for both, `#!/usr/bin/python3 -s`);
/// * the same program as argv[0] itself;
/// * the root screen daemon `SCREEN … -S ubuntu-release-upgrade-screen-window` (`DistUpgradeMain.py:164-188`).
///
/// An editor, a pager, `grep`, `ls` or any other program that merely names those paths does not count, and
/// neither do our own terminal wrapper and the pkexec prompt before it (do-release-upgrade only as a later
/// argument), so a dismissed password dialog is `startFailed`, not `ended`.
pub(crate) fn is_upgrader(args: &[String]) -> bool {
    let Some(first) = args.first() else { return false };
    let program = base(first);
    let upgrader = |arg: &str| base(arg) == "do-release-upgrade" || arg.starts_with("/tmp/ubuntu-release-upgrader-");
    if program.starts_with("python") {
        return python_script(&args[1..]).is_some_and(upgrader);
    }
    if matches!(program, "screen" | "SCREEN") {
        return args.windows(2).any(|pair| pair[0] == "-S" && pair[1] == UPGRADE_SCREEN);
    }
    upgrader(first)
}
/// The script a python command line runs: its first argument that is not an option (none for `-c`/`-m`/`-`).
fn python_script(args: &[String]) -> Option<&str> {
    let mut at = 0;
    while let Some(arg) = args.get(at) {
        match arg.as_str() {
            "-c" | "-m" | "-" => return None,
            "-W" | "-X" | "-Q" => at += 2,
            option if option.starts_with('-') => at += 1,
            script => return Some(script),
        }
    }
    None
}
/// A `/proc/*/cmdline` scan, any uid (read-only).
pub(crate) fn upgrader_running(paths: &Paths) -> bool {
    let Ok(entries) = fs::read_dir(&paths.proc) else { return false };
    entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())))
        .any(|e| is_upgrader(&cmdline(&e.path())))
}
/// unattended-upgrades runs (comm `unattended-upgr`): only picks the `waitLockAuto` text (design §4.7).
pub(crate) fn unattended_running(paths: &Paths) -> bool {
    let Ok(entries) = fs::read_dir(&paths.proc) else { return false };
    entries.flatten().any(|e| {
        fs::read_to_string(e.path().join("comm")).is_ok_and(|comm| comm.trim() == "unattended-upgr")
    })
}

/// One lock line of `/proc/locks`: (major, minor, inode) of the locked file. Waiter lines (`->`) hold nothing.
pub(crate) fn parse_locks(text: &str) -> Vec<(u32, u32, u64)> {
    text.lines()
        .filter(|line| !line.contains("->"))
        .filter_map(|line| {
            let device = line.split_whitespace().nth(5)?;
            let mut parts = device.split(':');
            let major = u32::from_str_radix(parts.next()?, 16).ok()?;
            let minor = u32::from_str_radix(parts.next()?, 16).ok()?;
            let inode = parts.next()?.parse().ok()?;
            Some((major, minor, inode))
        })
        .collect()
}
/// Is a dpkg/apt lock held (design §6.4 `busy`)? `/proc/locks` (readable without root) against the device and
/// inode of the four lock files. A comm scan is not used for this (every host runs `apt list` until a
/// snapshot exists).
#[allow(dead_code)] // the release start's busy rule (GW7)
pub(crate) fn apt_lock_held(paths: &Paths) -> bool {
    let Ok(text) = fs::read_to_string(&paths.proc_locks) else { return false };
    let held = parse_locks(&text);
    paths.apt_locks().iter().any(|file| {
        fs::metadata(file).is_ok_and(|m| {
            let dev = m.dev();
            held.contains(&(libc::major(dev), libc::minor(dev), m.ino()))
        })
    })
}
