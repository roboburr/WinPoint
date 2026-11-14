//! The hand-over to Ubuntu's release upgrader (design §6.4, §3.5, §4.5), GW7.
//!
//! * **`update.release.start {anyway?}`** ([`start`]): the preconditions in Ubuntu's order (`notOffered`;
//!   `updatesPending`: any upgradable package of the snapshot whose candidate has no Phased-Update-Percentage, held
//!   ones too, do-release-upgrade's own rule; `rebootFirst`: `reboot-required.pkgs` names libc6, linux-base or a
//!   linux-image; `busy`: our job, a foreign transaction or a held dpkg/apt lock in `/proc/locks`; `hold`: the
//!   verdict must be ok/warn, or hold with `allowAnyway` and the page's `anyway` after its warning), then
//!   `upgrade.json`, temporary linger, and the terminal: `xdg-terminal-exec --title=<title> -- <sh> -c '<WRAPPER>'
//!   win11-gnome-upgrade <status-file> fail <pkexec> <do-release-upgrade>` (fixed argv, every program from the tool
//!   table), environment `files::actions::user_environment(true)` plus `LANGUAGE`, `WIN11_PRESS_ENTER` and an
//!   activation token, cwd `$HOME`, started through `process_scope::spawn` (Role::App: a transient unit in
//!   `app.slice`, never `app-win11shell.slice`, whose MemoryMax would cover pkexec's root child).
//! * **Linger** ([`linger_on`], [`linger_off`]): `loginctl show-user <uid> -p Linger --value`; only on `no`,
//!   `loginctl enable-linger <uid>` and the stamp `~/.config/win11-gnome/update-linger-by-us`. `disable-linger` only
//!   while that stamp exists (ours) and no upgrader runs; the stamp goes after a successful disable.
//! * **Watching** ([`watch`], owner only; every 5 s while the hand-over is unresolved, and once on every update write
//!   and every `update.status`): the `/proc` scan sets `seenRunning`; the outcome is `model::upgrade_outcome` (reboot
//!   pending → card hidden; never seen + rc ≠ 0 → startFailed; never seen, no status 120 s → startFailed/noWindow;
//!   seen, gone, rc ≠ 0 or no status → ended; rc 0 → back to the offer). Every outcome but the reboot one turns our
//!   linger off; startFailed/ended stay in `upgrade.json` until the card has shown them and the user acts (the next
//!   `update.check` or a new start), the offer outcome removes the files at once.
//! * **The card** ([`overlay`], every host, files only): running/waiting/startFailed/ended > updatesFirst >
//!   rebootFirst (the last two only while the verdict allows a start) > GW6's verdict; `startRc`, `reasons`.
//! * **`update.release.attach`** ([`attach`]): only while Ubuntu's screen session runs (`notRunning`), the trusted
//!   attach helper through `admin::attach_argv` (`noAttachHelper` with the manual command otherwise) in the same
//!   terminal path and wrapper, hold `fail`, its own status file.
use super::{
    Shared, admin, guard, model, model::UpgradeOutcome, paths::Tool, release, restart, store,
};
use crate::{
    files::{ApiError, ApiResult},
    i18n::{Lang, NO_PARAMS},
};
use gio::prelude::AppInfoExt;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs,
    io::Read,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::Stdio,
    time::{Duration, Instant},
};

/// `upgrade.json` in the update state directory (design §4.5).
pub(crate) const UPGRADE: &str = "upgrade.json";
/// The wrapper's status files in `$XDG_RUNTIME_DIR/win11-gnome/` (tmpfs: never older than the boot).
pub(crate) const STATUS: &str = "upgrade-status.json";
pub(crate) const ATTACH_STATUS: &str = "attach-status.json";
/// `~/.config/win11-gnome/update-linger-by-us`: we turned linger on (survives the cache).
pub(crate) const LINGER_STAMP: &str = "update-linger-by-us";
/// `$0` of the wrapper for a start and for an attach.
pub(crate) const UPGRADE_NAME: &str = "win11-gnome-upgrade";
pub(crate) const ATTACH_NAME: &str = "win11-gnome-attach";
/// The watch interval (design §6.4 step 5).
pub(crate) const WATCH_SECONDS: u32 = 5;
/// `main.log` unchanged this long while the upgrader runs → `waiting` (design §3.5).
const LOG_STALE_MS: u64 = 10 * 60 * 1000;
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STATUS: u64 = 4096;
/// The owner's hand-over bookkeeping (start, the watch ticks of the main loop and of HTTP threads, shown, clear)
/// runs one at a time, so `upgrade.json`, linger and the history line are never done twice.
static BOOKKEEPING: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn bookkeeping() -> std::sync::MutexGuard<'static, ()> {
    BOOKKEEPING.lock().unwrap_or_else(|e| e.into_inner())
}

/// The terminal wrapper (design §6.4 step 4), one constant: it runs the command, writes `{rc, endedAt}` to the status
/// file, and holds the window (`read`) when the hold mode is `always` or the command failed, so pkexec's 126/127 and
/// Ubuntu's early refusals stay readable. `$1` status file, `$2` hold mode (`fail` | `always`), the rest the command.
/// `post_upgrade.rs` holds the same text (a test compares them).
pub(crate) const WRAPPER: &str = r#"status=$1; hold=$2; shift 2
"$@"; rc=$?
printf '{"rc":%d,"endedAt":%s}\n' "$rc" "$(date +%s)" > "$status.tmp" && mv -f "$status.tmp" "$status"
if [ "$hold" = always ] || [ "$rc" -ne 0 ]; then printf '\n%s\n' "$WIN11_PRESS_ENTER"; read -r _; fi
exit "$rc""#;

fn product(lang: Lang) -> String {
    crate::i18n::tr(lang, "update.product")
}
fn refusal(code: &str, key: &str, lang: Lang) -> ApiError {
    ApiError::tr(code, lang, key, &json!({"product": product(lang)}))
}
fn seam_error(lang: Lang) -> ApiError {
    ApiError::tr("test_seam", lang, "update.err.testSeam", NO_PARAMS)
}
/// `LANGUAGE` of the terminal (gettext's list): `nl:en` for Dutch, so Ubuntu's upgrader speaks Dutch when its
/// catalogue exists and English otherwise; `en` for English.
pub(crate) fn language_list(lang: Lang) -> String {
    let (code, fallback) = (lang.code(), Lang::En.code());
    if code == fallback {
        code.to_owned()
    } else {
        [code, fallback].join(":")
    }
}

// ------------------------------------------------------------------------------------------------ files

/// `$XDG_RUNTIME_DIR/win11-gnome` (the fixture home's `.run/win11-gnome` in test mode).
pub(crate) fn runtime_dir(shared: &Shared) -> PathBuf {
    let paths = &shared.setup.paths;
    if paths.fixture {
        paths.user_home.join(".run/win11-gnome")
    } else {
        glib::user_runtime_dir().join("win11-gnome")
    }
}
fn stamp(shared: &Shared) -> PathBuf {
    shared.ctx.config_dir.join(LINGER_STAMP)
}
/// The wrapper's `{rc, endedAt}`: a regular file of ours, not a link, at most 4 KiB. The shell writes it with the
/// session's umask, so its mode is not checked; the directory is our 0700 one.
pub(crate) fn read_status(path: &Path) -> Option<i64> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.len() > MAX_STATUS {
        return None;
    }
    let mut text = String::new();
    file.take(MAX_STATUS).read_to_string(&mut text).ok()?;
    serde_json::from_str::<Value>(&text).ok()?["rc"].as_i64()
}
fn mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64,
    )
}
/// The NUL-separated arguments of one `/proc/<pid>/cmdline` (at most 64 KiB).
fn cmdline(dir: &Path) -> Vec<String> {
    let mut bytes = Vec::new();
    if let Ok(file) = fs::File::open(dir.join("cmdline")) {
        let _ = file.take(64 << 10).read_to_end(&mut bytes);
    }
    bytes
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect()
}
/// Ubuntu's screen session (`SCREEN … -S ubuntu-release-upgrade-screen-window`, the text frontend's), the only one
/// the attach helper can resume.
pub(crate) fn screen_running(shared: &Shared) -> bool {
    let Ok(entries) = fs::read_dir(&shared.setup.paths.proc) else {
        return false;
    };
    entries
        .flatten()
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
        })
        .map(|e| cmdline(&e.path()))
        .any(|args| {
            args.first()
                .is_some_and(|a| matches!(a.rsplit('/').next(), Some("screen" | "SCREEN")))
                && args
                    .windows(2)
                    .any(|p| p[0] == "-S" && p[1] == guard::UPGRADE_SCREEN)
        })
}

// ------------------------------------------------------------------------------------------------ preconditions

/// do-release-upgrade's own rule (`do-release-upgrade:175-191`): an upgradable package whose candidate has no
/// Phased-Update-Percentage blocks the upgrade, held ones too. → (blocking count, held names when only held ones
/// block: "Alles" will not install those, so the card names them). `pup` is apt's stored percentage (phasing.rs);
/// do-release-upgrade reads the raw field, which differs only for a literal 100, a malformed field or records that
/// disagree.
pub(crate) fn updates_pending(snapshot: &Value) -> (usize, Vec<String>) {
    let pkgs: Vec<model::Pkg> = snapshot["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(model::Pkg::from_json)
        .collect();
    let blocking: Vec<&model::Pkg> = pkgs.iter().filter(|p| p.pup.is_none()).collect();
    let mut held: Vec<String> = Vec::new();
    if !blocking.is_empty() && blocking.iter().all(|p| p.held) {
        for p in &blocking {
            if !held.contains(&p.name) {
                held.push(p.name.clone());
            }
        }
    }
    (blocking.len(), held)
}
/// `reboot-required.pkgs` names libc6, linux-base or a linux-image (do-release-upgrade's "not rebooted" check).
pub(crate) fn reboot_first(shared: &Shared) -> bool {
    let reboot = restart::reboot(&shared.setup.paths);
    reboot.required
        && reboot
            .pkgs
            .iter()
            .any(|p| p == "libc6" || p == "linux-base" || p.starts_with("linux-image-"))
}
/// Our job (unfinished in this boot, owner alive), a foreign PackageKit transaction (an own Software job is one
/// too), or a held dpkg/apt lock (`/proc/locks` against the lock files' inodes; never a comm scan).
pub(crate) fn busy(shared: &Shared, snapshot: &Value) -> bool {
    let paths = &shared.setup.paths;
    let boot = store::boot_id(paths);
    let job = shared.store.read(store::JOB).is_some_and(|j| {
        j.get("finishedAt").is_none_or(Value::is_null)
            && boot.is_some()
            && j["bootId"].as_str() == boot.as_deref()
            && store::owner_alive(&j["owner"])
    });
    job || snapshot["foreign"] == true
        || guard::software_job_active(&shared.ctx.cache_dir.join("software/jobs"))
        || guard::apt_lock_held(paths)
}
/// A verdict that lets the upgrade start (design §6.2): ok or warn.
fn starts(verdict: &str) -> bool {
    matches!(verdict, "ok" | "warn")
}

/// Unit tests set GW6's verdict (and the offer) per cache directory here: compat.rs is GW6's, so this tree's
/// `release::status` always says `none`.
#[cfg(test)]
pub(crate) static TEST_RELEASE: std::sync::Mutex<Vec<(PathBuf, Value)>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(test)]
fn test_release(shared: &Shared, release: &mut Value) {
    let table = TEST_RELEASE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, extra)) = table.iter().find(|(dir, _)| *dir == shared.ctx.cache_dir) {
        for (key, value) in extra.as_object().into_iter().flatten() {
            release[key] = value.clone();
        }
    }
}
/// The snapshot's release object with GW6's verdict in `state` (design §4.3).
pub(crate) fn release_view(shared: &Shared) -> Value {
    #[allow(unused_mut)]
    let mut release = release::status(shared);
    #[cfg(test)]
    test_release(shared, &mut release);
    release
}

// ------------------------------------------------------------------------------------------------ tools

/// One program of the tool table with a timeout: (exit code, stdout). None: missing, killed, or no exit code.
fn run_tool(shared: &Shared, tool: Tool, args: &[String]) -> Option<(i32, String)> {
    let mut child = shared.setup.tools.command(tool).ok()?;
    child
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = child.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = (&mut stdout).take(64 << 10).read_to_string(&mut text);
        text
    });
    let started = Instant::now();
    let rc = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if started.elapsed() < TOOL_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(20))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let text = reader.join().unwrap_or_default();
    Some((rc?, text))
}
fn uid() -> String {
    unsafe { libc::getuid() }.to_string()
}

/// Turn linger on for the upgrade when it is off (design §6.4 step 3); true when the stamp says it is ours.
pub(crate) fn linger_on(shared: &Shared) -> bool {
    let shown = run_tool(
        shared,
        Tool::Loginctl,
        &[
            "show-user".into(),
            uid(),
            "-p".into(),
            "Linger".into(),
            "--value".into(),
        ],
    );
    match shown {
        Some((0, value)) if value.trim() == "no" => {
            let enabled = run_tool(shared, Tool::Loginctl, &["enable-linger".into(), uid()]);
            if enabled.is_some_and(|(rc, _)| rc == 0) {
                let stamp_text = json!({"uid":uid(),"since":store::now_ms()}).to_string();
                return store::atomic(&stamp(shared), stamp_text.as_bytes()).is_ok();
            }
            false
        }
        // already on: ours only when an earlier start of ours left the stamp
        Some((0, value)) if value.trim() == "yes" => stamp(shared).is_file(),
        _ => false,
    }
}
/// `loginctl disable-linger <uid>`, only while our stamp exists and no upgrader runs; the stamp goes after a
/// successful call. True when it ran and worked.
pub(crate) fn linger_off(shared: &Shared) -> bool {
    if !stamp(shared).is_file() || guard::upgrader_running(&shared.setup.paths) {
        return false;
    }
    let ok = run_tool(shared, Tool::Loginctl, &["disable-linger".into(), uid()])
        .is_some_and(|(rc, _)| rc == 0);
    if ok {
        let _ = fs::remove_file(stamp(shared));
    }
    ok
}

/// The terminal (design §6.4 step 4): `xdg-terminal-exec --title=<title> -- <sh> -c '<WRAPPER>' <name> <status>
/// <hold> <command…>` through `process_scope::spawn` (Role::App, `app.slice`). The environment is the user's
/// (`files::actions::user_environment(true)`, without the service's own test seams) plus `LANGUAGE`,
/// `WIN11_PRESS_ENTER` and an activation token for the default terminal; in test mode the tool table's `PATH` and
/// bus addresses go on top (and no token is asked for), so a stub terminal reaches nothing. cwd `$HOME`.
fn terminal(
    shared: &Shared,
    lang: Lang,
    name: &str,
    status: &Path,
    hold: &str,
    tail: Vec<OsString>,
    tag: &str,
) -> Result<(), ApiError> {
    let setup = &shared.setup;
    let no_terminal = || {
        ApiError::tr(
            "no_terminal",
            lang,
            "update.release.err.noTerminal",
            NO_PARAMS,
        )
    };
    let sh = setup.tools.path(Tool::Sh).map_err(|_| seam_error(lang))?;
    let xte = setup
        .tools
        .path(Tool::XdgTerminalExec)
        .map_err(|_| no_terminal())?;
    if !xte.is_file() {
        return Err(no_terminal());
    }
    let mut command = setup
        .tools
        .command(Tool::XdgTerminalExec)
        .map_err(|_| no_terminal())?;
    let table: Vec<(OsString, OsString)> = command
        .get_envs()
        .filter_map(|(k, v)| Some((k.to_owned(), v?.to_owned())))
        .collect();
    let home = &setup.paths.user_home;
    let mut env: BTreeMap<OsString, OsString> = crate::files::actions::user_environment(true);
    env.retain(|key, _| !key.to_string_lossy().starts_with(super::paths::VAR_PREFIX));
    env.insert("LANGUAGE".into(), language_list(lang).into());
    env.insert(
        "WIN11_PRESS_ENTER".into(),
        crate::i18n::tr(lang, "update.release.pressEnter").into(),
    );
    env.insert("PWD".into(), home.as_os_str().to_owned());
    env.remove(std::ffi::OsStr::new("DESKTOP_STARTUP_ID"));
    env.remove(std::ffi::OsStr::new("XDG_ACTIVATION_TOKEN"));
    if setup.test {
        for (key, value) in table.into_iter().filter(|(k, _)| k != "LANG") {
            env.insert(key, value);
        }
    } else if let Some(token) = crate::files::actions::default_terminal()
        .and_then(|info| info.id())
        .and_then(|id| crate::launch::activation_token(&json!({"desktopId":id.as_str()})))
    {
        env.insert("DESKTOP_STARTUP_ID".into(), token.clone().into());
        env.insert("XDG_ACTIVATION_TOKEN".into(), token.into());
    }
    command.env_clear().envs(&env).current_dir(home);
    command
        .arg(format!(
            "--title={}",
            crate::i18n::tr(lang, "update.release.terminalTitle")
        ))
        .arg("--")
        .arg(sh)
        .arg("-c")
        .arg(WRAPPER)
        .arg(name)
        .arg(status)
        .arg(hold)
        .args(tail);
    let env: Vec<(OsString, OsString)> = env.into_iter().collect();
    crate::process_scope::spawn(command, tag, Some(env)).map_err(|_| no_terminal())
}
/// The status directory (0700, ours) with the old status file removed.
fn fresh_status(shared: &Shared, name: &str) -> Option<PathBuf> {
    let dir = runtime_dir(shared);
    store::private_dir(&dir).ok()?;
    let path = dir.join(name);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(dir.join(format!("{name}.tmp")));
    Some(path)
}

// ------------------------------------------------------------------------------------------------ start

/// `update.release.start {anyway?:bool}` → `{started:true}` (`already:true` while our start is still going);
/// refusals of design §3.7 (module doc).
pub(super) fn start(shared: &Shared, args: &Value, lang: Lang) -> ApiResult {
    let _one = bookkeeping();
    let paths = &shared.setup.paths;
    // a start of ours that is still on its way or running: no second terminal
    if let Some(record) = our_record(shared)
        && record["outcome"].is_null()
        && matches!(
            outcome_of(shared, &record, guard::upgrader_running(paths)),
            UpgradeOutcome::Starting | UpgradeOutcome::Running
        )
    {
        return Ok(json!({"started":true,"already":true}));
    }
    let release = release_view(shared);
    let snapshot = shared.store.read(store::SNAPSHOT).unwrap_or(Value::Null);
    if release["offered"] != true {
        return Err(refusal(
            "not_offered",
            "update.release.err.notOffered",
            lang,
        ));
    }
    let (pending, held) = updates_pending(&snapshot);
    if pending > 0 {
        let mut error = refusal("updates_pending", "update.release.err.updatesPending", lang);
        error.data = Some(json!({"count":pending,"held":held}));
        return Err(error);
    }
    if reboot_first(shared) {
        return Err(refusal(
            "reboot_first",
            "update.release.err.rebootFirst",
            lang,
        ));
    }
    if busy(shared, &snapshot) {
        return Err(refusal("busy", "update.release.err.busy", lang));
    }
    let verdict = release["state"].as_str().unwrap_or("none");
    let anyway = verdict == "hold" && args["anyway"] == true && release["allowAnyway"] == true;
    if verdict == "ubuntuHold" {
        return Err(refusal(
            "not_offered",
            "update.release.err.notOffered",
            lang,
        ));
    }
    if !starts(verdict) && !anyway {
        return Err(refusal("hold", "update.release.err.hold", lang));
    }
    let (pkexec, upgrader) = match (
        shared.setup.tools.path(Tool::Pkexec),
        shared.setup.tools.path(Tool::DoReleaseUpgrade),
    ) {
        (Ok(p), Ok(u)) => (p, u),
        _ => return Err(seam_error(lang)),
    };
    // an earlier outcome (startFailed/ended, or a record of another boot) goes with its status files
    remove(shared);
    let Some(status) = fresh_status(shared, STATUS) else {
        return Err(refusal("failed", "update.err.failed", lang));
    };
    // design §6.4 steps 2-4: upgrade.json, linger, the terminal
    let mut record = json!({"jobId":store::token().unwrap_or_default(),"startedAt":store::now_ms(),"bootId":store::boot_id(paths),
                            "version":release["version"],"codename":release["codename"],"anyway":anyway,
                            "lingerByUs":false,"seenRunning":false,"seenAt":null,"outcome":null,"shownAt":null,"card":"running"});
    shared
        .store
        .write(UPGRADE, &record)
        .map_err(|_| refusal("failed", "update.err.failed", lang))?;
    record["lingerByUs"] = json!(linger_on(shared));
    let _ = shared.store.write(UPGRADE, &record);
    if let Err(error) = terminal(
        shared,
        lang,
        UPGRADE_NAME,
        &status,
        "fail",
        vec![pkexec.into(), upgrader.into()],
        "release-upgrade",
    ) {
        linger_off(shared);
        shared.store.remove(UPGRADE);
        return Err(error);
    }
    announce(shared);
    Ok(json!({"started":true}))
}

// ------------------------------------------------------------------------------------------------ attach

/// `update.release.attach {}` → `{started:true}`: `pkexec <release-upgrade-attach>` in the terminal wrapper (hold
/// `fail`), only while Ubuntu's screen session runs.
pub(super) fn attach(shared: &Shared, lang: Lang) -> ApiResult {
    if !screen_running(shared) {
        return Err(ApiError::tr(
            "not_running",
            lang,
            "update.release.err.notRunning",
            NO_PARAMS,
        ));
    }
    let argv = admin::attach_argv(shared, lang)?;
    let Some(status) = fresh_status(shared, ATTACH_STATUS) else {
        return Err(refusal("failed", "update.err.failed", lang));
    };
    let tail = argv.into_iter().map(PathBuf::into_os_string).collect();
    terminal(
        shared,
        lang,
        ATTACH_NAME,
        &status,
        "fail",
        tail,
        "release-attach",
    )?;
    Ok(json!({"started":true}))
}

// ------------------------------------------------------------------------------------------------ watching

/// The outcome of our hand-over as the files and the scan show it (recorded ones first).
pub(crate) fn outcome_of(shared: &Shared, record: &Value, running: bool) -> UpgradeOutcome {
    match record["outcome"]["state"].as_str() {
        Some("startFailed") => {
            return record["outcome"]["rc"]
                .as_i64()
                .map_or(UpgradeOutcome::NoWindow, UpgradeOutcome::StartFailed);
        }
        Some("ended") => return UpgradeOutcome::Ended,
        Some("restart") => return UpgradeOutcome::Restart,
        _ => {}
    }
    let paths = &shared.setup.paths;
    let seen = record["seenRunning"] == true || running;
    let rc = read_status(&runtime_dir(shared).join(STATUS));
    let since = Duration::from_millis(
        store::now_ms().saturating_sub(record["startedAt"].as_u64().unwrap_or(0)),
    );
    model::upgrade_outcome(running, seen, rc, since, paths.reboot_required.exists())
}
/// `waiting` (design §3.5): the upgrader runs and `/var/log/dist-upgrade/main.log` has not changed for 10 minutes
/// (counted from when we first saw the upgrader, so an old log of an earlier upgrade does not count).
pub(crate) fn waiting(shared: &Shared, record: Option<&Value>, now: u64) -> bool {
    let Some(log) = mtime_ms(&shared.setup.paths.dist_upgrade_log.join("main.log")) else {
        return false;
    };
    let since = record.map_or(0, |r| {
        r["seenAt"]
            .as_u64()
            .or(r["startedAt"].as_u64())
            .unwrap_or(0)
    });
    now.saturating_sub(log.max(since)) >= LOG_STALE_MS
}
/// `upgrade.json` of this boot.
fn our_record(shared: &Shared) -> Option<Value> {
    let record = shared.store.read(UPGRADE)?;
    (record["bootId"].as_str() == store::boot_id(&shared.setup.paths).as_deref()).then_some(record)
}
/// The owner's bookkeeping, once per tick (module doc). True while the 5 s timer should keep running.
pub(crate) fn watch(shared: &Shared) -> bool {
    let _one = bookkeeping();
    let Some(mut record) = shared.store.read(UPGRADE) else {
        return false;
    };
    let paths = &shared.setup.paths;
    let now = store::now_ms();
    if record["bootId"].as_str() != store::boot_id(paths).as_deref() {
        // a hand-over of another boot (the post-upgrade check normally removed it already)
        linger_off(shared);
        shared.store.remove(UPGRADE);
        return false;
    }
    if !record["outcome"].is_null() {
        return false;
    }
    let running = guard::upgrader_running(paths);
    let mut dirty = false;
    if running && record["seenRunning"] != true {
        record["seenRunning"] = json!(true);
        record["seenAt"] = json!(now);
        dirty = true;
    }
    let outcome = outcome_of(shared, &record, running);
    let finished = |record: &mut Value, state: &str, rc: Option<i64>, key: Option<&str>| {
        record["outcome"] = json!({"state":state,"rc":rc,"key":key,"at":now});
    };
    // the hand-over's window for the update history (history.rs matches any role, gw4.md decision 6)
    let log_job = |record: &Value| {
        if record["seenRunning"] == true {
            let entry = json!({"jobId":record["jobId"],"role":"release","startedAt":record["startedAt"],"finishedAt":now,
                               "outcome":record["outcome"]});
            let _ = shared.store.append_job(&entry, now);
        }
    };
    let card = match outcome {
        UpgradeOutcome::Starting => "running",
        UpgradeOutcome::Running if waiting(shared, Some(&record), now) => "waiting",
        UpgradeOutcome::Running => "running",
        UpgradeOutcome::Restart => {
            finished(&mut record, "restart", None, None);
            log_job(&record);
            "none"
        }
        UpgradeOutcome::StartFailed(rc) => {
            finished(
                &mut record,
                "startFailed",
                Some(rc),
                Some(model::start_failure_key(rc)),
            );
            linger_off(shared);
            "startFailed"
        }
        UpgradeOutcome::NoWindow => {
            finished(
                &mut record,
                "startFailed",
                None,
                Some("update.release.err.noWindow"),
            );
            linger_off(shared);
            "startFailed"
        }
        UpgradeOutcome::Ended => {
            let rc = read_status(&runtime_dir(shared).join(STATUS));
            finished(&mut record, "ended", rc, None);
            linger_off(shared);
            log_job(&record);
            "ended"
        }
        UpgradeOutcome::Offer => {
            finished(&mut record, "offer", Some(0), None);
            linger_off(shared);
            log_job(&record);
            remove(shared);
            announce(shared);
            return false;
        }
    };
    if record["card"] != card {
        record["card"] = json!(card);
        dirty = true;
    }
    if dirty || !record["outcome"].is_null() {
        let _ = shared.store.write(UPGRADE, &record);
        announce(shared);
    }
    record["outcome"].is_null()
}
/// `upgrade.json` and both status files go.
fn remove(shared: &Shared) {
    shared.store.remove(UPGRADE);
    let dir = runtime_dir(shared);
    for name in [STATUS, ATTACH_STATUS] {
        let _ = fs::remove_file(dir.join(name));
    }
}
/// The owner's `update.status` showed the card's startFailed/ended state: note it (design §6.4 "once the card has
/// shown it").
pub(crate) fn shown(shared: &Shared, view: &Value) {
    let _one = bookkeeping();
    if !matches!(
        view["release"]["state"].as_str(),
        Some("startFailed" | "ended")
    ) {
        return;
    }
    if let Some(mut record) = our_record(shared)
        && record["outcome"].is_object()
        && record["shownAt"].is_null()
    {
        record["shownAt"] = json!(store::now_ms());
        let _ = shared.store.write(UPGRADE, &record);
    }
}
/// The next `update.check` after the card showed startFailed/ended returns to the offer and its checks: the record
/// and the status files go. (A reboot-pending record stays for the post-upgrade check.)
pub(crate) fn clear_shown(shared: &Shared) {
    let _one = bookkeeping();
    if let Some(record) = shared.store.read(UPGRADE)
        && matches!(
            record["outcome"]["state"].as_str(),
            Some("startFailed" | "ended")
        )
        && !record["shownAt"].is_null()
    {
        remove(shared);
    }
}

/// `update.changed` after a hand-over change (the page re-reads `update.status`; release.rs's `announce`).
fn announce(shared: &Shared) {
    let snapshot = shared.store.read(store::SNAPSHOT);
    let job = shared.store.read(store::JOB);
    let view = super::compose(shared, snapshot.as_ref(), job.as_ref(), None, false);
    let rev = store::next_rev(
        &shared.store,
        shared.rev.load(std::sync::atomic::Ordering::SeqCst),
    );
    shared.ctx.emit(
        "update.changed",
        json!({"rev":rev,"phase":view["phase"],"totals":view["totals"],"restart":view["restart"],"job":view["job"],
               "error":view["error"],"release":{"state":view["release"]["state"],"version":view["release"]["version"]}}),
    );
}

// ------------------------------------------------------------------------------------------------ the card

/// What the card state depends on (pure, [`card_state`]).
#[derive(Clone, Debug, Default)]
pub(crate) struct Card {
    /// GW6's verdict (`none`, `checking`, `ok`, `warn`, `hold`, `ubuntuHold`, `blocked`, `unknown`)
    pub verdict: String,
    pub offered: bool,
    /// Ubuntu's upgrader runs (the `/proc` scan; also one we did not start)
    pub running: bool,
    pub waiting: bool,
    /// our hand-over's outcome (None: no `upgrade.json` of this boot)
    pub outcome: Option<UpgradeOutcome>,
    pub updates_first: bool,
    pub reboot_first: bool,
}
/// running/waiting/startFailed/ended > updatesFirst > rebootFirst (only while the verdict allows a start and the
/// release is offered) > the verdict. A pending reboot after our upgrade hides the card (`none`).
pub(crate) fn card_state(c: &Card) -> String {
    if c.running {
        return if c.waiting { "waiting" } else { "running" }.into();
    }
    match c.outcome {
        Some(UpgradeOutcome::Starting | UpgradeOutcome::Running) => return "running".into(),
        Some(UpgradeOutcome::StartFailed(_) | UpgradeOutcome::NoWindow) => {
            return "startFailed".into();
        }
        Some(UpgradeOutcome::Ended) => return "ended".into(),
        Some(UpgradeOutcome::Restart) => return "none".into(),
        Some(UpgradeOutcome::Offer) | None => {}
    }
    if c.offered && starts(&c.verdict) {
        if c.updates_first {
            return "updatesFirst".into();
        }
        if c.reboot_first {
            return "rebootFirst".into();
        }
    }
    c.verdict.clone()
}
/// The hand-over overlay on the snapshot's `release` object (design §4.3, every host, files only): `state`,
/// `verdict` (GW6's state under it), `startRc`, `reasons` (startFailed: the rendered start failure; GW6's stay in
/// `verdictReasons`), `heldNames` (updatesFirst when only held packages block), `attachable` (Ubuntu's screen
/// session runs, so "Venster weer openen" can work).
pub(crate) fn overlay(
    shared: &Shared,
    release: &mut Value,
    verdict: Value,
    snapshot: &Value,
    running: bool,
) {
    #[cfg(test)]
    let verdict = {
        let mut extra = json!({"state": verdict});
        test_release(shared, &mut extra);
        test_release(shared, release);
        extra["state"].clone()
    };
    let lang = shared.ctx.lang();
    let now = store::now_ms();
    let record = our_record(shared);
    let outcome = record.as_ref().map(|r| outcome_of(shared, r, running));
    let (pending, held) = updates_pending(snapshot);
    let card = Card {
        verdict: verdict.as_str().unwrap_or("none").to_owned(),
        offered: release["offered"] == true,
        running,
        waiting: running && waiting(shared, record.as_ref(), now),
        outcome,
        updates_first: pending > 0,
        reboot_first: reboot_first(shared),
    };
    let state = card_state(&card);
    let rc = match outcome {
        Some(UpgradeOutcome::StartFailed(rc)) => Some(rc),
        Some(UpgradeOutcome::Ended) => record.as_ref().and_then(|r| r["outcome"]["rc"].as_i64()),
        _ => None,
    };
    if state == "startFailed" {
        let key = match outcome {
            Some(UpgradeOutcome::StartFailed(rc)) => model::start_failure_key(rc),
            _ => "update.release.err.noWindow",
        };
        let reason = ApiError::tr("start_failed", lang, key, NO_PARAMS);
        release["verdictReasons"] = release["reasons"].clone();
        release["reasons"] = json!([reason.json()]);
    }
    release["verdict"] = verdict;
    release["state"] = json!(state);
    release["startRc"] = json!(rc);
    release["heldNames"] = json!(if state == "updatesFirst" {
        held
    } else {
        Vec::new()
    });
    release["attachable"] =
        json!(matches!(state.as_str(), "running" | "waiting") && screen_running(shared));
}

// ------------------------------------------------------------------------------------------------ tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::update::{
        Mode, Update,
        admin::tests::shared as fixture_shared,
        tests::{Fixture, script, write},
    };
    use std::{os::unix::fs::PermissionsExt, time::SystemTime};

    const BOOT: &str = "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";

    /// Recording stubs: the terminal writes its argv (NUL-separated), part of its environment and its cwd;
    /// loginctl answers `show-user` from a state file, keeps it, and logs every call to the fixture's stub log.
    fn recording(f: &Fixture, linger: &str) {
        let argv = f.base.join("terminal.argv");
        let env = f.base.join("terminal.env");
        script(
            &f.tools.join("xdg-terminal-exec"),
            &format!(
                "for a in \"$@\"; do printf '%s\\0' \"$a\"; done > '{argv}.tmp'\n\
                 {{ echo \"LANGUAGE=$LANGUAGE\"; echo \"WIN11_PRESS_ENTER=$WIN11_PRESS_ENTER\"; echo \"PATH=$PATH\"; \
                 echo \"SYSTEM=$DBUS_SYSTEM_BUS_ADDRESS\"; echo \"SESSION=$DBUS_SESSION_BUS_ADDRESS\"; echo \"CWD=$(pwd -P)\"; \
                 echo \"SEAM=${{WIN11_UPDATE_TOOL_DIR:-}}\"; }} > '{env}'\nmv '{argv}.tmp' '{argv}'\n",
                argv = argv.display(),
                env = env.display()
            ),
        );
        script(&f.tools.join("mv"), "exec /usr/bin/mv \"$@\"\n");
        fs::write(f.base.join("linger"), format!("{linger}\n")).unwrap();
        script(
            &f.tools.join("loginctl"),
            &format!(
                "echo \"loginctl $*\" >> '{log}'\ncase \"$1\" in\n  show-user) read -r v < '{state}'; echo \"$v\" ;;\n  \
                 enable-linger) echo yes > '{state}' ;;\n  disable-linger) echo no > '{state}' ;;\nesac\n",
                log = f.log.display(),
                state = f.base.join("linger").display()
            ),
        );
    }
    fn terminal_argv(f: &Fixture) -> Option<Vec<String>> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let path = f.base.join("terminal.argv");
        while !path.exists() {
            if Instant::now() > deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let raw = fs::read(path).unwrap();
        Some(
            raw.split(|b| *b == 0)
                .filter(|a| !a.is_empty())
                .map(|a| String::from_utf8_lossy(a).into_owned())
                .collect(),
        )
    }
    fn terminal_env(f: &Fixture) -> BTreeMap<String, String> {
        fs::read_to_string(f.base.join("terminal.env"))
            .unwrap()
            .lines()
            .filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
            .collect()
    }
    /// GW6's verdict and the offer for this fixture (`TEST_RELEASE`).
    fn verdict(shared: &Shared, extra: Value) {
        let mut table = TEST_RELEASE.lock().unwrap();
        table.retain(|(dir, _)| *dir != shared.ctx.cache_dir);
        let mut merged =
            json!({"offered":true,"version":"26.10","codename":"stonking","allowAnyway":false});
        for (k, v) in extra.as_object().unwrap() {
            merged[k] = v.clone();
        }
        table.push((shared.ctx.cache_dir.clone(), merged));
    }
    fn pkg(name: &str, pup: Option<u32>, held: bool) -> Value {
        json!({"id":format!("{name};2.0-1;amd64;ubuntu-resolute-updates"),"name":name,"info":5,"summary":"s","pup":pup,
               "held":held,"phased":false})
    }
    fn set_age(path: &Path, ms_ago: u64) {
        let at = SystemTime::now() - Duration::from_millis(ms_ago);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(at)
            .unwrap();
    }
    fn status_file(shared: &Shared) -> PathBuf {
        runtime_dir(shared).join(STATUS)
    }
    fn fixture_file(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/update/fixtures/upgrade")
            .join(name)
    }

    /// The exact argv and environment of a start (the wrapper and the tool paths, never -f/-d/-p/--allow-third-party
    /// or sudo), through `process_scope::spawn` (without a user manager on this test bus that is its plain spawn;
    /// the bwrap layer test records the app.slice unit), linger turned on with our stamp.
    #[test]
    fn a_start_runs_the_fixed_argv_with_the_language_and_the_press_enter_line() {
        for (lang, language, enter, title) in [
            (
                "nl",
                "nl:en",
                "Druk op Enter om dit venster te sluiten.",
                "Ubuntu-upgrade",
            ),
            (
                "en",
                "en",
                "Press Enter to close this window.",
                "Ubuntu upgrade",
            ),
        ] {
            let f = Fixture::new(&format!("gw7-start-{lang}"));
            recording(&f, "no");
            let shared = fixture_shared(&f, lang);
            verdict(&shared, json!({"state":"ok"}));
            assert_eq!(
                start(&shared, &json!({}), shared.ctx.lang()).unwrap(),
                json!({"started":true})
            );
            let argv = terminal_argv(&f).expect("the terminal stub ran");
            let tool = |name: &str| f.tools.join(name).display().to_string();
            let status = runtime_dir(&shared).join(STATUS).display().to_string();
            let want: Vec<String> = vec![
                format!("--title={title}"),
                "--".into(),
                tool("sh"),
                "-c".into(),
                WRAPPER.into(),
                UPGRADE_NAME.into(),
                status,
                "fail".into(),
                tool("pkexec"),
                tool("do-release-upgrade"),
            ];
            assert_eq!(argv, want, "{lang}");
            for word in ["-f", "-d", "-p", "--allow-third-party", "sudo"] {
                assert!(!argv.iter().any(|a| a == word), "{lang}: {word}");
            }
            let env = terminal_env(&f);
            assert_eq!(env["LANGUAGE"], language);
            assert_eq!(env["WIN11_PRESS_ENTER"], enter);
            assert_eq!(
                env["PATH"],
                f.tools.display().to_string(),
                "test mode: the stub directory only"
            );
            let dead = f.tools.join(super::super::paths::NO_SESSION_BUS);
            assert_eq!(env["SESSION"], format!("unix:path={}", dead.display()));
            let home = shared.setup.paths.user_home.canonicalize().unwrap();
            assert_eq!(env["CWD"], home.display().to_string());
            assert_eq!(
                env["SEAM"], "",
                "the service's own seams stay out of the terminal"
            );
            // linger: off → enabled, the stamp is ours, upgrade.json records it
            let me = uid();
            assert_eq!(
                f.stub_log(),
                format!("loginctl show-user {me} -p Linger --value\nloginctl enable-linger {me}\n")
            );
            assert!(shared.ctx.config_dir.join(LINGER_STAMP).is_file());
            let record = shared.store.read(UPGRADE).unwrap();
            assert_eq!(record["lingerByUs"], true);
            assert_eq!(
                (record["seenRunning"].clone(), record["codename"].clone()),
                (json!(false), json!("stonking"))
            );
            assert_eq!(
                fs::metadata(runtime_dir(&shared))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            // a second click while it is on its way: no second terminal
            fs::remove_file(f.base.join("terminal.argv")).unwrap();
            assert_eq!(
                start(&shared, &json!({}), shared.ctx.lang()).unwrap(),
                json!({"started":true,"already":true})
            );
            std::thread::sleep(Duration::from_millis(200));
            assert!(!f.base.join("terminal.argv").exists());
        }
    }

    /// The preconditions in Ubuntu's order, each with its §3.7 text; a refusal runs no program.
    #[test]
    fn the_preconditions_refuse_in_ubuntus_order() {
        let f = Fixture::new("gw7-pre");
        recording(&f, "no");
        let shared = fixture_shared(&f, "en");
        let refused = |args: Value| start(&shared, &args, Lang::En).unwrap_err();
        verdict(&shared, json!({"offered":false,"state":"ok"}));
        assert_eq!(
            refused(json!({})).message,
            "Ubuntu isn’t offering a new version right now."
        );
        verdict(&shared, json!({"state":"ok"}));
        // updatesPending: a non-phased package (also a held one); a phased one (pup) does not count
        let snapshot = |pkgs: Vec<Value>| {
            shared
                .store
                .write(store::SNAPSHOT, &json!({"packages":pkgs}))
                .unwrap()
        };
        snapshot(vec![
            pkg("libfoo1", None, false),
            pkg("libbar1", Some(10), false),
        ]);
        let e = refused(json!({}));
        assert_eq!(
            (e.code.as_str(), e.message.as_str()),
            ("updates_pending", "Install the available updates first.")
        );
        assert_eq!(e.data, Some(json!({"count":1,"held":[]})));
        snapshot(vec![
            pkg("libheld1", None, true),
            pkg("libbar1", Some(10), false),
        ]);
        assert_eq!(
            refused(json!({})).data,
            Some(json!({"count":1,"held":["libheld1"]})),
            "only held ones block: named"
        );
        snapshot(vec![pkg("libbar1", Some(10), false)]);
        // rebootFirst: libc6, linux-base or a linux-image in reboot-required.pkgs (gnome-shell alone is not; the
        // hold verdict after it then refuses)
        write(
            &f.root.join("run/reboot-required"),
            "*** System restart required ***\n",
        );
        write(&f.root.join("run/reboot-required.pkgs"), "gnome-shell\n");
        verdict(&shared, json!({"state":"hold"}));
        assert_eq!(
            refused(json!({})).key.as_deref(),
            Some("update.release.err.hold")
        );
        verdict(&shared, json!({"state":"ok"}));
        for name in ["libc6", "linux-base", "linux-image-7.0.0-45-generic"] {
            write(
                &f.root.join("run/reboot-required.pkgs"),
                &format!("gnome-shell\n{name}\n"),
            );
            assert_eq!(
                refused(json!({})).message,
                "Restart your PC first.",
                "{name}"
            );
        }
        fs::remove_file(f.root.join("run/reboot-required")).unwrap();
        // busy: our unfinished job of this boot with a live owner, a foreign transaction, a held dpkg lock
        shared
            .store
            .write(
                store::JOB,
                &json!({"bootId":BOOT,"owner":store::current_owner()}),
            )
            .unwrap();
        assert_eq!(
            refused(json!({})).message,
            "Software is still being installed. Try again afterwards."
        );
        shared
            .store
            .write(
                store::JOB,
                &json!({"bootId":BOOT,"owner":store::current_owner(),"finishedAt":1}),
            )
            .unwrap();
        shared
            .store
            .write(store::SNAPSHOT, &json!({"packages":[],"foreign":true}))
            .unwrap();
        assert_eq!(refused(json!({})).code, "busy");
        shared
            .store
            .write(store::SNAPSHOT, &json!({"packages":[]}))
            .unwrap();
        let lock = f.root.join("var/lib/dpkg/lock-frontend");
        write(&lock, "");
        let meta = fs::metadata(&lock).unwrap();
        let device = format!(
            "{:02x}:{:02x}:{}",
            libc::major(meta.dev()),
            libc::minor(meta.dev()),
            meta.ino()
        );
        write(
            &f.root.join("proc/locks"),
            &format!("1: POSIX  ADVISORY  WRITE 4242 {device} 0 EOF\n"),
        );
        assert_eq!(refused(json!({})).code, "busy", "a held dpkg lock");
        write(
            &f.root.join("proc/locks"),
            "1: POSIX  ADVISORY  WRITE 4242 00:00:1 0 EOF\n",
        );
        // hold/blocked: never; hold with allowAnyway only after the page's warning (anyway:true)
        for (state, allow, args, key) in [
            ("hold", false, json!({}), "update.release.err.hold"),
            (
                "hold",
                false,
                json!({"anyway":true}),
                "update.release.err.hold",
            ),
            ("hold", true, json!({}), "update.release.err.hold"),
            (
                "blocked",
                true,
                json!({"anyway":true}),
                "update.release.err.hold",
            ),
            ("none", false, json!({}), "update.release.err.hold"),
            ("unknown", false, json!({}), "update.release.err.hold"),
            (
                "ubuntuHold",
                false,
                json!({}),
                "update.release.err.notOffered",
            ),
        ] {
            verdict(&shared, json!({"state":state,"allowAnyway":allow}));
            assert_eq!(
                refused(args.clone()).key.as_deref(),
                Some(key),
                "{state} {allow} {args}"
            );
        }
        verdict(&shared, json!({"state":"hold","allowAnyway":false}));
        assert_eq!(
            refused(json!({})).message,
            "Win11 Shell isn’t ready for this Ubuntu version yet."
        );
        assert_eq!(f.stub_log(), "", "a refusal runs no program");
        assert!(shared.store.read(UPGRADE).is_none());
        // hold + allowAnyway + the page's anyway: it starts
        verdict(&shared, json!({"state":"hold","allowAnyway":true}));
        assert_eq!(
            start(&shared, &json!({"anyway":true}), Lang::En).unwrap(),
            json!({"started":true})
        );
        assert_eq!(shared.store.read(UPGRADE).unwrap()["anyway"], true);
        assert!(terminal_argv(&f).is_some());
    }

    /// Linger: enable only when off; an already-on linger is not ours (no stamp, never disabled); disable only while
    /// the stamp exists and no upgrader runs; the stamp lives in the config directory (it survives a restart).
    #[test]
    fn linger_is_enabled_only_when_off_and_disabled_only_by_us() {
        let f = Fixture::new("gw7-linger");
        recording(&f, "yes");
        let shared = fixture_shared(&f, "nl");
        let me = uid();
        assert!(!linger_on(&shared), "already on: not ours");
        assert!(!shared.ctx.config_dir.join(LINGER_STAMP).exists());
        assert!(!linger_off(&shared), "no stamp: never disabled");
        fs::write(f.base.join("linger"), "no\n").unwrap();
        assert!(linger_on(&shared));
        assert_eq!(
            fs::read_to_string(f.base.join("linger")).unwrap().trim(),
            "yes"
        );
        assert!(
            linger_on(&shared),
            "a later start finds linger on with our stamp"
        );
        let config = shared.ctx.config_dir.clone();
        assert!(config.join(LINGER_STAMP).is_file());
        // a new service (a restarted Settings) on the same config directory: the stamp is still there and ours
        let again = Shared {
            ctx: shared.ctx.clone(),
            setup: shared.setup.clone(),
            store: shared.store.clone(),
            stats: Default::default(),
            rev: Default::default(),
        };
        write(
            &f.root.join("proc/900/cmdline"),
            "python3\0do-release-upgrade\0",
        );
        assert!(!linger_off(&again), "never while an upgrader runs");
        fs::remove_dir_all(f.root.join("proc/900")).unwrap();
        assert!(linger_off(&again));
        assert!(!config.join(LINGER_STAMP).exists());
        assert_eq!(
            fs::read_to_string(f.base.join("linger")).unwrap().trim(),
            "no"
        );
        let show = format!("loginctl show-user {me} -p Linger --value\n");
        assert_eq!(
            f.stub_log(),
            format!(
                "{show}{show}loginctl enable-linger {me}\n{show}loginctl disable-linger {me}\n"
            )
        );
        // a failing enable leaves no stamp
        script(
            &f.tools.join("loginctl"),
            "[ \"$1\" = show-user ] && echo no\n[ \"$1\" = show-user ]\n",
        );
        assert!(!linger_on(&shared));
        assert!(!config.join(LINGER_STAMP).exists());
    }

    /// The wrapper itself (the constant string, run by /bin/sh through the tool table's `sh` stub in a temp dir): it
    /// writes `{rc, endedAt}`, holds on rc ≠ 0 (and on `always`) until Enter, and exits with the command's rc.
    #[test]
    fn the_wrapper_writes_the_status_and_holds_on_failure() {
        let f = Fixture::new("gw7-wrapper");
        script(&f.tools.join("sh"), "exec /bin/sh \"$@\"\n");
        for name in ["date", "mv"] {
            script(
                &f.tools.join(name),
                &format!("exec /usr/bin/{name} \"$@\"\n"),
            );
        }
        script(
            &f.tools.join("fake-upgrader"),
            "echo \"upgrader $*\"\nexit \"$1\"\n",
        );
        let shared = fixture_shared(&f, "en");
        let run = |hold: &str, rc: i32| {
            let status = f.base.join(format!("status-{hold}-{rc}.json"));
            let mut child = shared.setup.tools.command(Tool::Sh).unwrap();
            child
                .args(["-c", WRAPPER, UPGRADE_NAME])
                .arg(&status)
                .arg(hold)
                .arg(f.tools.join("fake-upgrader"))
                .arg(rc.to_string())
                .env("WIN11_PRESS_ENTER", "Press Enter to close this window.")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped());
            let mut child = child.spawn().unwrap();
            std::thread::sleep(Duration::from_millis(400));
            let held = child.try_wait().unwrap().is_none();
            if held {
                use std::io::Write;
                child.stdin.as_mut().unwrap().write_all(b"\n").unwrap();
            }
            let out = child.wait_with_output().unwrap();
            (
                held,
                out.status.code(),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                read_status(&status),
                status,
            )
        };
        let (held, code, out, rc, status) = run("fail", 0);
        assert_eq!((held, code, rc), (false, Some(0), Some(0)), "rc 0: no hold");
        assert_eq!(out, "upgrader 0\n");
        let text = fs::read_to_string(&status).unwrap();
        assert!(
            text.starts_with("{\"rc\":0,\"endedAt\":") && text.ends_with("}\n"),
            "{text}"
        );
        assert!(!f.base.join("status-fail-0.json.tmp").exists());
        for rc_in in [1, 126, 127] {
            let (held, code, out, rc, _) = run("fail", rc_in);
            assert_eq!(
                (held, code, rc),
                (true, Some(rc_in), Some(rc_in as i64)),
                "rc {rc_in}: held"
            );
            assert_eq!(
                out,
                format!("upgrader {rc_in}\n\nPress Enter to close this window.\n")
            );
        }
        let (held, code, _, rc, _) = run("always", 0);
        assert_eq!((held, code, rc), (true, Some(0), Some(0)), "always: held");
    }

    /// The outcomes of design §6.4 step 5 from fixture files (status rc × seen × reboot × time): the card states,
    /// linger off in every outcome but the reboot one, the cleanup once the card has shown it.
    #[test]
    fn the_watch_turns_status_files_into_startfailed_ended_or_the_offer() {
        let fixture = |rc: Option<&str>, seen: bool, age_ms: u64, reboot: bool| {
            let f = Fixture::new("gw7-watch");
            recording(&f, "yes");
            let shared = fixture_shared(&f, "en");
            verdict(&shared, json!({"state":"ok"}));
            let record = json!({"jobId":"0123456789abcdef0123456789abcdef","startedAt":store::now_ms() - age_ms,"bootId":BOOT,
                                "seenRunning":seen,"outcome":null,"card":"running"});
            shared.store.write(UPGRADE, &record).unwrap();
            fs::write(shared.ctx.config_dir.join(LINGER_STAMP), "{}").unwrap();
            store::private_dir(&runtime_dir(&shared)).unwrap();
            if let Some(name) = rc {
                fs::copy(fixture_file(name), status_file(&shared)).unwrap();
            }
            if reboot {
                write(&f.root.join("run/reboot-required"), "");
            }
            (f, shared)
        };
        let state = |shared: &Shared| {
            super::super::compose(shared, None, None, None, false)["release"].clone()
        };
        // never seen, rc 126/127/1 → startFailed with the rc reason; linger off; kept until shown and a check
        for (file, rc, text) in [
            (
                "status-126.json",
                126,
                "You didn’t enter the password, so nothing was changed.",
            ),
            (
                "status-127.json",
                127,
                "The password was wrong. Nothing was changed.",
            ),
            (
                "status-1.json",
                1,
                "Ubuntu didn’t start the upgrade. The reason is in the terminal window.",
            ),
        ] {
            let (f, shared) = fixture(Some(file), false, 5_000, true);
            assert!(!watch(&shared), "{file}: resolved");
            let r = state(&shared);
            assert_eq!(
                (
                    r["state"].clone(),
                    r["startRc"].clone(),
                    r["verdict"].clone()
                ),
                (json!("startFailed"), json!(rc), json!("ok")),
                "{file}"
            );
            assert_eq!(r["reasons"][0]["message"], text, "{file}");
            assert!(
                f.stub_log().contains("disable-linger"),
                "{file}: linger off"
            );
            clear_shown(&shared);
            assert!(
                shared.store.read(UPGRADE).is_some(),
                "{file}: not shown yet, kept"
            );
            shown(&shared, &json!({"release":r}));
            clear_shown(&shared);
            assert!(
                shared.store.read(UPGRADE).is_none() && !status_file(&shared).exists(),
                "{file}: cleaned"
            );
        }
        // never seen, no status file: still starting before 120 s, noWindow after
        let (_f, shared) = fixture(None, false, 5_000, false);
        assert!(watch(&shared), "starting: the timer goes on");
        assert_eq!(state(&shared)["state"], "running");
        let (_f, shared) = fixture(None, false, 121_000, false);
        assert!(!watch(&shared));
        let r = state(&shared);
        assert_eq!(
            (r["state"].clone(), r["startRc"].clone()),
            (json!("startFailed"), Value::Null)
        );
        assert_eq!(r["reasons"][0]["key"], "update.release.err.noWindow");
        // seen, gone, rc ≠ 0 or no status file → ended (and a jobs.jsonl line for the history)
        for file in [Some("status-1.json"), None] {
            let (f, shared) = fixture(file, true, 600_000, false);
            assert!(!watch(&shared));
            assert_eq!(state(&shared)["state"], "ended", "{file:?}");
            assert!(f.stub_log().contains("disable-linger"));
            assert_eq!(shared.store.jobs().last().unwrap()["role"], "release");
        }
        // rc 0, gone, no reboot → back to the offer at once (files removed, linger off)
        let (f, shared) = fixture(Some("status-0.json"), true, 600_000, false);
        assert!(!watch(&shared));
        assert!(shared.store.read(UPGRADE).is_none() && !status_file(&shared).exists());
        assert!(f.stub_log().contains("disable-linger"));
        assert_eq!(state(&shared)["state"], "ok");
        // rc 0 or seen, with a pending reboot → card hidden, linger stays for the reboot, the record stays
        for (file, seen) in [
            (Some("status-0.json"), false),
            (None, true),
            (Some("status-1.json"), true),
        ] {
            let (f, shared) = fixture(file, seen, 600_000, true);
            assert!(!watch(&shared));
            assert_eq!(state(&shared)["state"], "none", "{file:?} {seen}");
            assert!(
                !f.stub_log().contains("disable-linger"),
                "{file:?}: linger kept for the reboot"
            );
            assert!(shared.store.read(UPGRADE).is_some());
        }
        // a dismissed password dialog while today's reboot is already pending is still startFailed (gw2.md 11)
        let (_f, shared) = fixture(Some("status-126.json"), false, 5_000, true);
        watch(&shared);
        assert_eq!(state(&shared)["state"], "startFailed");
        // a record of another boot is dropped
        let (_f, shared) = fixture(None, false, 5_000, false);
        let mut record = shared.store.read(UPGRADE).unwrap();
        record["bootId"] = json!("another-boot");
        shared.store.write(UPGRADE, &record).unwrap();
        assert!(!watch(&shared));
        assert!(shared.store.read(UPGRADE).is_none());
    }

    /// running and waiting from fixture procfs and `main.log` mtimes; the screen session makes the card attachable.
    #[test]
    fn running_and_waiting_come_from_the_scan_and_the_log_age() {
        let f = Fixture::new("gw7-running");
        recording(&f, "yes");
        let shared = fixture_shared(&f, "nl");
        verdict(&shared, json!({"state":"ok"}));
        let state = |shared: &Shared| {
            super::super::compose(shared, None, None, None, false)["release"].clone()
        };
        write(
            &f.root.join("proc/777/cmdline"),
            "python3\0-s\0/tmp/ubuntu-release-upgrader-x1/stonking\0--mode=server\0",
        );
        let log = f.root.join("var/log/dist-upgrade/main.log");
        write(&log, "x\n");
        // an upgrader we did not start: running; a stale log (11 min): waiting
        set_age(&log, 60_000);
        assert_eq!(
            (
                state(&shared)["state"].clone(),
                state(&shared)["attachable"].clone()
            ),
            (json!("running"), json!(false))
        );
        set_age(&log, 11 * 60_000);
        assert_eq!(state(&shared)["state"], "waiting");
        write(
            &f.root.join("proc/778/cmdline"),
            &format!("SCREEN\0-e\0\\0\\0\0-S\0{}\0", guard::UPGRADE_SCREEN),
        );
        assert_eq!(state(&shared)["attachable"], true);
        // ours, just seen: the old log of an earlier upgrade does not make it waiting
        let record = json!({"startedAt":store::now_ms() - 20_000,"bootId":BOOT,"seenRunning":false,"outcome":null,"card":"running"});
        shared.store.write(UPGRADE, &record).unwrap();
        assert!(watch(&shared), "running: the timer goes on");
        let mut record = shared.store.read(UPGRADE).unwrap();
        assert_eq!(record["seenRunning"], true);
        assert_eq!(state(&shared)["state"], "running");
        record["seenAt"] = json!(store::now_ms() - 11 * 60_000);
        shared.store.write(UPGRADE, &record).unwrap();
        assert!(watch(&shared));
        assert_eq!(shared.store.read(UPGRADE).unwrap()["card"], "waiting");
        assert_eq!(state(&shared)["state"], "waiting");
        assert_eq!(f.stub_log(), "", "running: no program");
    }

    /// The card's precedence (pure) and do-release-upgrade's updatesFirst rule on snapshot packages.
    #[test]
    fn the_card_state_follows_the_precedence() {
        let c = |verdict: &str| Card {
            verdict: verdict.into(),
            offered: true,
            ..Card::default()
        };
        assert_eq!(card_state(&c("ok")), "ok");
        assert_eq!(
            card_state(&Card {
                updates_first: true,
                reboot_first: true,
                ..c("ok")
            }),
            "updatesFirst"
        );
        assert_eq!(
            card_state(&Card {
                reboot_first: true,
                ..c("warn")
            }),
            "rebootFirst"
        );
        for verdict in [
            "hold",
            "blocked",
            "unknown",
            "checking",
            "none",
            "ubuntuHold",
        ] {
            assert_eq!(
                card_state(&Card {
                    updates_first: true,
                    reboot_first: true,
                    ..c(verdict)
                }),
                verdict,
                "only when a start is allowed"
            );
        }
        assert_eq!(
            card_state(&Card {
                updates_first: true,
                offered: false,
                ..c("ok")
            }),
            "ok",
            "not offered"
        );
        assert_eq!(
            card_state(&Card {
                running: true,
                updates_first: true,
                ..c("hold")
            }),
            "running"
        );
        assert_eq!(
            card_state(&Card {
                running: true,
                waiting: true,
                ..c("ok")
            }),
            "waiting"
        );
        for (outcome, want) in [
            (UpgradeOutcome::Starting, "running"),
            (UpgradeOutcome::StartFailed(126), "startFailed"),
            (UpgradeOutcome::NoWindow, "startFailed"),
            (UpgradeOutcome::Ended, "ended"),
            (UpgradeOutcome::Restart, "none"),
            (UpgradeOutcome::Offer, "updatesFirst"),
        ] {
            assert_eq!(
                card_state(&Card {
                    outcome: Some(outcome),
                    updates_first: true,
                    ..c("ok")
                }),
                want,
                "{outcome:?}"
            );
        }
        let pkgs = json!({"packages":[pkg("a1", None, true), pkg("b1", Some(30), true), pkg("a1", None, true)]});
        assert_eq!(updates_pending(&pkgs), (2, vec!["a1".to_string()]));
        assert_eq!(
            updates_pending(&json!({"packages":[pkg("a1", None, true), pkg("c1", None, false)]})),
            (2, Vec::new())
        );
        assert_eq!(updates_pending(&json!({})), (0, Vec::new()));
    }

    /// attach: only while Ubuntu's screen session runs, through the trusted helper and the wrapper (hold fail), its
    /// own status file; without the helper the manual command.
    #[test]
    fn attach_needs_the_screen_session_and_the_helper() {
        let f = Fixture::new("gw7-attach");
        recording(&f, "yes");
        let shared = fixture_shared(&f, "en");
        write(
            &f.root.join("proc/777/cmdline"),
            "python3\0do-release-upgrade\0",
        );
        assert_eq!(
            attach(&shared, Lang::En).unwrap_err().message,
            "The Ubuntu upgrade isn’t running any more."
        );
        write(
            &f.root.join("proc/778/cmdline"),
            &format!("SCREEN\0-S\0{}\0", guard::UPGRADE_SCREEN),
        );
        assert_eq!(attach(&shared, Lang::En).unwrap(), json!({"started":true}));
        let tool = |name: &str| f.tools.join(name).display().to_string();
        let want: Vec<String> = vec![
            "--title=Ubuntu upgrade".into(),
            "--".into(),
            tool("sh"),
            "-c".into(),
            WRAPPER.into(),
            ATTACH_NAME.into(),
            runtime_dir(&shared)
                .join(ATTACH_STATUS)
                .display()
                .to_string(),
            "fail".into(),
            tool("pkexec"),
            tool("release-upgrade-attach"),
        ];
        assert_eq!(terminal_argv(&f).unwrap(), want);
        fs::remove_file(f.tools.join("release-upgrade-attach")).unwrap();
        let error = attach(&shared, Lang::En).unwrap_err();
        assert_eq!(
            error.message,
            "Open a terminal and type: sudo screen -d -r ubuntu-release-upgrade-screen-window"
        );
        assert_eq!(f.stub_log(), "", "no loginctl, nothing else");
    }

    /// Through the service: the argument allowlist, the dry-run envelope, and a missing terminal is noTerminal with
    /// upgrade.json and linger undone.
    #[test]
    fn the_service_routes_start_and_undoes_a_failed_terminal() {
        let f = Fixture::new("gw7-service");
        recording(&f, "no");
        let owner = Update::with_env(f.ctx("settings", false, "en"), &f.env_no_bus());
        assert_eq!(owner.mode(), Mode::Owner);
        let shared = owner.shared.clone().unwrap();
        verdict(&shared, json!({"state":"ok"}));
        fs::remove_file(f.tools.join("xdg-terminal-exec")).unwrap();
        let error = owner
            .call("update.release.start", &json!({}))
            .unwrap()
            .unwrap_err();
        assert_eq!(
            error.message,
            "No terminal was found. Install Ptyxis (Terminal) and try again."
        );
        assert!(shared.store.read(UPGRADE).is_none());
        assert!(
            !shared.ctx.config_dir.join(LINGER_STAMP).exists(),
            "linger undone"
        );
        assert!(f.stub_log().contains("disable-linger"));
        assert_eq!(
            owner
                .call("update.release.start", &json!({"anyway":"yes"}))
                .unwrap()
                .unwrap_err()
                .code,
            "bad_args"
        );
        assert_eq!(
            owner
                .call("update.release.start", &json!({"x":true}))
                .unwrap()
                .unwrap_err()
                .code,
            "bad_args"
        );
        owner.shutdown();
        let dry = Update::with_env(f.ctx("settings", true, "en"), &f.env_no_bus());
        assert_eq!(
            dry.call("update.release.start", &json!({"anyway":true}))
                .unwrap()
                .unwrap(),
            json!({"dryRun":true})
        );
        dry.shutdown();
    }

    /// post_upgrade.rs (win11-launch, no lib crate) carries the same wrapper text.
    #[test]
    fn post_upgrade_uses_the_same_wrapper() {
        assert!(include_str!("../../post_upgrade.rs").contains(WRAPPER));
    }

    /// Only inside tests/update/test_post_upgrade.py's layer (tests/update/run.sh exec), whose private session bus has
    /// a fake user manager that records StartTransientUnit and starts nothing: the start takes process_scope's
    /// user-manager path (Role::App). The driver checks the recorded unit (`app.slice`, the argv, the environment).
    #[test]
    #[ignore = "GW7 layer test: only tests/update/test_post_upgrade.py inside tests/update/run.sh's bwrap layer"]
    fn layer_start_goes_through_the_user_manager() {
        let netns = fs::read_link("/proc/self/ns/net").unwrap();
        let host = std::env::var_os("WIN11_TEST_HOST_NETNS").unwrap_or_default();
        assert!(
            !host.is_empty() && host != netns.as_os_str(),
            "not inside the layer (--unshare-net)"
        );
        assert!(
            !Path::new("/run/dbus/system_bus_socket").exists(),
            "the real system bus is visible"
        );
        let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
        assert!(
            bus.starts_with("unix:path=/tmp/w11-update/dbus-"),
            "not the layer's own session bus: {bus}"
        );
        let f = Fixture::new("gw7-layer");
        recording(&f, "no");
        let shared = fixture_shared(&f, "nl");
        verdict(&shared, json!({"state":"ok"}));
        assert_eq!(
            start(&shared, &json!({}), Lang::Nl).unwrap(),
            json!({"started":true})
        );
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            !f.base.join("terminal.argv").exists(),
            "the user manager took the unit: no plain spawn"
        );
        println!("GW7-LAYER base={}", f.base.display());
    }
}
