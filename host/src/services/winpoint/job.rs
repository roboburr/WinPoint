//! WinPoint's update job (D-G74, design 5.3): download, verify, unpack and install a newer release, owner only, on a
//! `winpoint-job` thread of its own holding `job.lock`. It starts only from `winpoint.install`, that is after the
//! user clicked Download and install or Try again in Settings ([`start`]); never from a check, a timer, a
//! notification or a status read. [`reattach`] only watches an installer that already runs.
//!
//! * **Steps.** `downloading`: GitHub's release list again (the job's own answer: its version, asset URL, size and
//!   digest; never a cached digest), refused as `stale` when the clicked version is no longer the one offered and
//!   as `noDigest` when its tarball has no digest; then the `.sha256` (4 KiB), which must name the exact tarball with
//!   GitHub's hash (`mismatch`), and the tarball (`.part`, 0600, streamed into SHA-256, exactly its size, https-only
//!   redirects). `verifying`: the streamed hash must equal the digest, else the file is deleted at once (`checksum`);
//!   Try again reuses a tarball of an earlier run only when it still hashes to the fresh digest. `unpacking`:
//!   files/archives.rs `extract_release` into a fresh `bundle` (link targets byte for byte). `preparing`: the
//!   bundle belongs to V (its Cargo.toml, metadata.json and the marker in bin/win11-panel; scripts/install.py a
//!   regular file), space on the prefix's file system, and `install.py --check` (D-G74's frozen contract):
//!   `gnomeMismatch` and `abi` block V as `incompatible`, `missingParts` as `missingParts`, root-owned system files
//!   to change as `manual`; `extensionChanged` means a sign-out after the install (`restart:'relogin'`).
//!   `installing` is written before the installer can exist (`installer.pid` null until it is seen, with its script
//!   and `installingAt`), so an owner that dies or a start that fails half way never counts as "nothing changed" while
//!   an installer may run: the next look finds it in /proc by its frozen argv ([`find_installer`]), else its status
//!   file decides, else its log (install.py opens it before anything else: none means it never ran). install.py runs
//!   as its own process (a transient unit through process_scope.rs, else a plain child), polled through its
//!   `--status` file every 500 ms; 15 minutes after its own start (the clock of /proc, so a new owner's watch keeps it)
//!   SIGTERM (it rolls back and writes 3), 3 minutes later SIGKILL. Then `installed`, or `failed` (outcome
//!   `nothingChanged`, `rolledBack` or `unknown`), `cancelled` (Cancel before `installing`; it also stops a running
//!   `--check`), or `interrupted` (the installer ended without a status: Try again rolls forward).
//! * **Trust.** No signing key (Robin, 14-11: "Zonder sleutel, via GitHub"): GitHub's account, its TLS and the API's
//!   `digest` are the anchor (verify.rs); in test mode the digest seam.json pins (seam.rs `trusts`).
//! * **Space.** The cache must hold the tarball (unless a verified one is there already) and then its unpacked bundle
//!   (extract_release measures it before it writes), each with 64 MiB to spare; a full disk on the way is `space` too.
//! * **Files.** `job.json` (this job or the last one) and `work/V/` (the `.sha256`, the tarball, `bundle/`,
//!   `install.status`, `install.log`, `check.log`; only the newest V is kept) in
//!   `$XDG_CACHE_HOME/win11-gnome/winpoint/`.
//! * **Finishing.** `installed` + `restart` clears once a new owner runs V (after
//!   `shell.restartAll {reason:'update'}`), `installed` + `relogin` once the session key (boot id and our GNOME
//!   Shell's start) differs; clearing removes `job.json` and `work/`.
use super::{
    Shared, check,
    check::Failure,
    feed,
    http::{self, HttpError, Sink},
    verify,
    version::Version,
};
use crate::{
    files::{ReleaseLimits, extract_release},
    i18n::Lang,
    services::update::{restart, store},
};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(crate) const JOB: &str = "job.json";
pub(crate) const JOB_LOCK: &str = "job.lock";
const WORK: &str = "work";
/// The one Python program WinPoint starts of its own at run time (D-G74): the release bundle's scripts/install.py.
/// `-I` would drop scripts/ from `sys.path` (install.py imports its neighbours), so `-E -s -B`.
const PYTHON: &str = "/usr/bin/python3";
const PYTHON_FLAGS: [&str; 3] = ["-E", "-s", "-B"];
/// The job states before `installing`: Cancel still works, nothing outside `work/` was touched.
pub(crate) const BEFORE_INSTALL: [&str; 4] = ["downloading", "verifying", "unpacking", "preparing"];
const SMALL_ACCEPT: &str = "application/octet-stream";
const SMALL_LIMITS: http::Limits = http::Limits {
    max_body: feed::MAX_SMALL,
    stall: Duration::from_secs(20),
    deadline: Some(Duration::from_secs(60)),
};
const TARBALL_STALL: Duration = Duration::from_secs(30);
/// Room kept free beyond the tarball (the cache) and beyond the new files (the prefix).
const SPARE: u64 = 64 << 20;
const BUNDLE_LIMITS: ReleaseLimits = ReleaseLimits {
    max_bytes: 1 << 30,
    max_entries: 100_000,
    spare: SPARE,
};
const CHECK_TIMEOUT: Duration = Duration::from_secs(120);
/// SIGTERM once the installer has run this long (it rolls back), SIGKILL [`KILL_AFTER`] after that SIGTERM (a hung
/// rollback): [`Clock`].
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const KILL_AFTER: Duration = Duration::from_secs(3 * 60);
/// How long the job looks for an installer whose start failed half way (the request may have reached systemd), and
/// how long after `installingAt` a new owner still waits for one to appear.
const FIND_WAIT: Duration = Duration::from_secs(10);
const SPAWN_GRACE_MS: u64 = 30_000;
const POLL: Duration = Duration::from_millis(500);
const PROGRESS_EVERY: Duration = Duration::from_secs(1);
const PERSIST_EVERY: Duration = Duration::from_secs(2);
const LOG_TAIL: u64 = 4 << 10;
const MAX_CHECK_OUTPUT: usize = 64 << 10;
/// run_child's answer to a set `stop`.
const CANCELLED: &str = "cancelled";

// ------------------------------------------------------------------------------------------------ processes

/// How the job starts its two programs (tests replace it; production: [`Scope`]).
pub(crate) trait Spawner: Send + Sync {
    /// The installer, as its own process with exactly `env`: its pid.
    fn spawn(
        &self,
        command: Command,
        tag: &str,
        env: Vec<(OsString, OsString)>,
    ) -> Result<u32, String>;
    /// The compatibility check, a direct child with exactly `env`, stderr into `log`, at most `timeout`, killed when
    /// `stop` is set (Cancel, the host's shutdown): its exit code and stdout.
    fn output(
        &self,
        command: Command,
        env: Vec<(OsString, OsString)>,
        log: File,
        timeout: Duration,
        stop: &AtomicBool,
    ) -> Result<(Option<i32>, Vec<u8>), String> {
        run_child(command, env, log, timeout, stop)
    }
}
/// The installer as the MainPID of a transient systemd user unit (`app-win11-winpoint-install-…`), so a SIGTERM
/// reaches it; without a user manager (the GNOME harness) a plain child.
pub(crate) struct Scope;
impl Spawner for Scope {
    fn spawn(
        &self,
        mut command: Command,
        tag: &str,
        env: Vec<(OsString, OsString)>,
    ) -> Result<u32, String> {
        // the environment stays exactly the frozen one: no portal opt-in variable
        command.env_remove("GTK_USE_PORTAL");
        crate::process_scope::spawn_pid(command, tag, Some(env))
    }
}
/// A direct child with exactly `env`, at most `timeout` and until `stop` (then SIGKILL: `--check` writes nothing).
pub(crate) fn run_child(
    mut command: Command,
    env: Vec<(OsString, OsString)>,
    log: File,
    timeout: Duration,
    stop: &AtomicBool,
) -> Result<(Option<i32>, Vec<u8>), String> {
    command
        .env_clear()
        .envs(env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(log));
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no stdout")?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buffer = [0u8; 8192];
        while let Ok(n) = stdout.read(&mut buffer) {
            if n == 0 {
                break;
            }
            if out.len() < MAX_CHECK_OUTPUT {
                out.extend_from_slice(&buffer[..n.min(MAX_CHECK_OUTPUT - out.len())]);
            }
        }
        let _ = tx.send(out);
    });
    let deadline = Instant::now() + timeout;
    let code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code(),
            Ok(None) if stop.load(Ordering::SeqCst) || Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(if stop.load(Ordering::SeqCst) {
                    CANCELLED.into()
                } else {
                    "timeout".into()
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error.to_string()),
        }
    };
    Ok((
        code,
        rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default(),
    ))
}

/// What tests put in place of this host's own facts.
#[derive(Default)]
pub(crate) struct Hooks {
    /// the installed prefix (None: the one this host runs from, `install_prefix()`)
    pub prefix: Option<PathBuf>,
    /// the session key (None: the real one)
    pub session_key: Option<Option<String>>,
    /// the spawner (None: [`Scope`])
    pub spawner: Option<Arc<dyn Spawner>>,
    /// this host's own version (None: `crate::VERSION`); set, it also takes the place of the seam's `running`, so a
    /// test sees the version this host compares with follow the payload
    pub own_version: Option<Version>,
    /// the installer's SIGTERM and SIGKILL times (None: [`INSTALL_TIMEOUT`], [`KILL_AFTER`])
    pub install_timeout: Option<Duration>,
    pub kill_after: Option<Duration>,
}

/// The running job of this owner process: Cancel and the shutdown set `stop`; `committed` from the moment the
/// installer starts (no Cancel any more).
pub(crate) struct Running {
    pub id: String,
    pub stop: Arc<AtomicBool>,
    pub committed: bool,
}

// ------------------------------------------------------------------------------------------------ reading

/// `job.json` of the state folder `dir` (`$XDG_CACHE_HOME/win11-gnome/winpoint`), read without creating anything.
fn read_job_in(dir: &Path) -> Option<Value> {
    let bytes = store::read_private(&dir.join(JOB), 1 << 20).ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
}
fn state_of(job: &Value) -> &str {
    job["state"].as_str().unwrap_or("")
}
/// The state a job shows: its own while its owner (or, installing, its installer) lives; an earlier step whose owner
/// died is `failed` (nothing outside `work/` was touched), an install whose owner and installer both died is
/// `interrupted` until the owner reads its status file. An install whose owner died before it saw the installer's
/// pid stays `installing` while that installer is found, or may still appear ([`starting`]).
pub(crate) fn effective_state(job: &Value) -> &'static str {
    let owner = store::owner_alive(&job["owner"]);
    match state_of(job) {
        "installing" if owner || store::owner_alive(&job["installer"]) || starting(job) => {
            "installing"
        }
        "installing" => "interrupted",
        s @ ("downloading" | "verifying" | "unpacking" | "preparing") => {
            if owner {
                BEFORE_INSTALL
                    .into_iter()
                    .find(|b| *b == s)
                    .unwrap_or("failed")
            } else {
                "failed"
            }
        }
        "installed" => "installed",
        "interrupted" => "interrupted",
        "cancelled" => "cancelled",
        "failed" => "failed",
        _ => "",
    }
}
/// `installing` with no installer pid yet: its start was asked for (`installingAt`) less than 30 s ago, or a process
/// with its frozen argv runs.
fn starting(job: &Value) -> bool {
    let installer = &job["installer"];
    if !installer.is_object() || !installer["pid"].is_null() {
        return false;
    }
    let recent = job["installingAt"]
        .as_u64()
        .is_some_and(|at| store::now_ms().saturating_sub(at) < SPAWN_GRACE_MS);
    recent
        || installer["script"]
            .as_str()
            .and_then(find_installer)
            .is_some()
}
/// This user's process that runs `script` as the installer (the system's python3 with `-E -s -B <script> …
/// --status …`, the frozen argv of [`run_install`]), found in /proc: the pid of an installer whose start this owner
/// did not see end (a failed or unfinished `spawn`, or an owner that died meanwhile). Reading /proc starts nothing.
pub(crate) fn find_installer(script: &str) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    let euid = unsafe { libc::geteuid() };
    let mut found = fs::read_dir("/proc").ok()?.flatten().filter_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        if entry.metadata().ok()?.uid() != euid {
            return None;
        }
        let cmdline = fs::read(entry.path().join("cmdline")).ok()?;
        let argv: Vec<&[u8]> = cmdline.strip_suffix(b"\0")?.split(|b| *b == 0).collect();
        let head: Vec<&[u8]> = std::iter::once(PYTHON)
            .chain(PYTHON_FLAGS)
            .chain([script])
            .map(str::as_bytes)
            .collect();
        (argv.len() > head.len()
            && argv[..head.len()] == head[..]
            && argv.contains(&b"--status".as_slice()))
        .then_some(pid)
    });
    found.next()
}
/// How long the process `{pid, start}` has run, from the kernel's own clock (its start in ticks since boot and
/// /proc/uptime): the same for every owner that watches it. Both count a suspend, so a watch reads it once, at its
/// start ([`Clock`]).
fn running_for(process: &Value) -> Option<Duration> {
    let start: f64 = process["start"].as_str()?.parse().ok()?;
    let uptime: f64 = fs::read_to_string("/proc/uptime")
        .ok()?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    (ticks > 0).then(|| Duration::from_secs_f64((uptime - start / ticks as f64).max(0.0)))
}
/// D-G74 guard (`shell.restartAll`, update/guard.rs): a job of WinPoint's updater runs, `downloading` through
/// `installing`, with a live owner or installer. `cache_dir` is `$XDG_CACHE_HOME/win11-gnome`.
pub(crate) fn job_active(cache_dir: &Path) -> bool {
    read_job_in(&cache_dir.join(super::STATE_DIR)).is_some_and(|job| {
        matches!(
            effective_state(&job),
            "downloading" | "verifying" | "unpacking" | "preparing" | "installing"
        )
    })
}
/// D-G74 guard (power actions): WinPoint's installer runs.
pub(crate) fn installing(cache_dir: &Path) -> bool {
    read_job_in(&cache_dir.join(super::STATE_DIR))
        .is_some_and(|job| effective_state(&job) == "installing")
}
/// D-G74 guard (`shell.restartAll`): WinPoint was installed and waits for a sign-out that has not happened yet (the
/// same session key).
pub(crate) fn relogin_pending(cache_dir: &Path) -> bool {
    read_job_in(&cache_dir.join(super::STATE_DIR)).is_some_and(|job| {
        state_of(&job) == "installed"
            && job["restart"] == "relogin"
            && job["sessionKey"] == json!(session_key())
    })
}

/// This login session: the boot id and the start of our uid's GNOME Shell (`-` when none runs). A sign-out and
/// sign-in changes it, a restart of WinPoint does not.
pub(crate) fn session_key() -> Option<String> {
    use crate::services::update::paths;
    let setup = paths::resolve(&paths::Env::default()).ok()?;
    let boot = store::boot_id(&setup.paths)?;
    let shell = restart::gnome_shell_start(&setup.paths, unsafe { libc::geteuid() });
    Some(format!("{boot}:{}", shell.as_deref().unwrap_or("-")))
}

/// Is the job's Python there? (`noPython`; never started by this.)
pub(crate) fn python_present() -> bool {
    fs::metadata(PYTHON).is_ok_and(|m| m.is_file())
}

/// Free bytes for this user on the file system of `path` or its nearest existing ancestor.
pub(crate) fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let mut at = path;
    while !at.exists() {
        at = at.parent()?;
    }
    let c = std::ffi::CString::new(at.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(c.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    let stat = unsafe { stat.assume_init() };
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}
/// A size as the card's texts show it: "71,2 MB" / "71.2 MB", "1,2 GB".
pub(crate) fn size_text(lang: Lang, bytes: u64) -> String {
    let mb = bytes as f64 / 1_000_000.0;
    if mb >= 1000.0 {
        format!("{} GB", crate::i18n::number(lang, mb / 1000.0, 1))
    } else {
        format!("{} MB", crate::i18n::number(lang, mb.max(0.1), 1))
    }
}

// ------------------------------------------------------------------------------------------------ the job

/// How the job ended before `installed`.
enum End {
    Failed {
        error: Failure,
        outcome: &'static str,
    },
    Cancelled,
    /// the host shuts down: nothing is written (the next owner ends the record, [`reattach`])
    Shutdown,
}
fn fail(code: &'static str, key: &'static str, params: Value, detail: String) -> End {
    End::Failed {
        error: Failure {
            code,
            key,
            params,
            detail,
        },
        outcome: "nothingChanged",
    }
}

/// One job, on its thread.
struct Job<'a> {
    shared: &'a Shared,
    record: Value,
    stop: &'a AtomicBool,
    version: Version,
    lang: Lang,
    work: PathBuf,
    prefix: PathBuf,
    last_progress: Instant,
    last_persist: Instant,
}
impl Job<'_> {
    fn id(&self) -> String {
        self.record["jobId"].as_str().unwrap_or("").to_owned()
    }
    fn set(&mut self, state: &str) {
        self.record["state"] = json!(state);
        self.shared.write_job(&mut self.record);
    }
    /// A step's error: a user's Cancel, the host's shutdown, or `end`.
    fn stopped_or(&self, end: End) -> End {
        if self.shared.stop.load(Ordering::SeqCst) {
            End::Shutdown
        } else if self.stop.load(Ordering::SeqCst) {
            End::Cancelled
        } else {
            end
        }
    }
    fn http_error(&self, error: &HttpError) -> End {
        match error {
            HttpError::Cancelled => self.stopped_or(End::Cancelled),
            other => End::Failed {
                error: Failure::http(other),
                outcome: "nothingChanged",
            },
        }
    }
    fn progress(&mut self, done: u64, total: u64, force: bool) {
        if force || self.last_progress.elapsed() >= PROGRESS_EVERY {
            self.last_progress = Instant::now();
            self.shared.ctx.emit(
                "winpoint.progress",
                json!({"jobId":self.id(),"state":self.record["state"],"done":done,"total":total}),
            );
        }
        if force || self.last_persist.elapsed() >= PERSIST_EVERY {
            self.last_persist = Instant::now();
            self.record["done"] = json!(done);
            self.record["total"] = json!(total);
            let _ = self.shared.store.write(JOB, &self.record);
        }
    }
}

/// `winpoint.install` in the owner, once its preconditions held (winpoint.rs): write the record and start the
/// thread, which takes over `lock`. The one way into [`run_job`].
pub(crate) fn start(
    shared: &Arc<Shared>,
    mut record: Value,
    lock: store::FileLock,
) -> Result<String, String> {
    let id = record["jobId"].as_str().unwrap_or("").to_owned();
    let stop = Arc::new(AtomicBool::new(false));
    {
        let mut running = shared.job.lock().unwrap_or_else(|e| e.into_inner());
        if running.is_some() {
            return Err("busy".into());
        }
        *running = Some(Running {
            id: id.clone(),
            stop: stop.clone(),
            committed: false,
        });
    }
    shared.write_job(&mut record);
    let thread = shared.clone();
    let spawned = std::thread::Builder::new()
        .name("winpoint-job".into())
        .spawn(move || {
            let _lock = lock;
            let _running = Clear(&thread);
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| run_job(&thread, record, &stop));
        });
    if let Err(error) = spawned {
        shared.job.lock().unwrap_or_else(|e| e.into_inner()).take();
        return Err(error.to_string());
    }
    Ok(id)
}
/// Clears this process's running job when its thread ends, however it ends.
struct Clear<'a>(&'a Shared);
impl Drop for Clear<'_> {
    fn drop(&mut self) {
        self.0.job.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}

fn run_job(shared: &Shared, record: Value, stop: &AtomicBool) {
    let dir = shared.store.dir().to_path_buf();
    let Some(version) = record["version"].as_str().and_then(Version::parse) else {
        return;
    };
    let prefix = PathBuf::from(record["prefix"].as_str().unwrap_or(""));
    let mut job = Job {
        shared,
        stop,
        lang: Lang::parse(record["lang"].as_str().unwrap_or("nl")),
        work: dir.join(WORK).join(version.to_string()),
        version,
        prefix,
        record,
        last_progress: Instant::now(),
        last_persist: Instant::now(),
    };
    match install_steps(&mut job) {
        Ok(()) => {}
        Err(End::Shutdown) => {}
        Err(End::Cancelled) => {
            let _ = fs::remove_dir_all(job.work.join("bundle"));
            finish(
                shared,
                &mut job.record,
                "cancelled",
                None,
                Some("nothingChanged"),
            );
        }
        Err(End::Failed { error, outcome }) => {
            let _ = fs::remove_dir_all(job.work.join("bundle"));
            // `manual` and `incompatible` stay for this version (a manual check forgets only `missingParts` and
            // `incompatible`, and a GNOME or library upgrade takes long): the verified tarball would only take room
            if matches!(error.code, "manual" | "incompatible") {
                remove_tarballs(&job.work);
            }
            finish(
                shared,
                &mut job.record,
                "failed",
                Some(&error),
                Some(outcome),
            );
        }
    }
}

/// Every step of 5.3 up to the installer's start and its end (Robin's click only: [`start`] → [`run_job`]).
fn install_steps(job: &mut Job) -> Result<(), End> {
    let assets = fresh_release(job)?;
    let digest = assets
        .tarball
        .digest
        .as_deref()
        .and_then(verify::parse_digest)
        .ok_or_else(no_digest)?;
    if !job.shared.seam.trusts(&digest) {
        // test mode: not the tarball seam.json pins, whoever answered on the loopback port (seam.rs)
        return Err(fail(
            "seam",
            "update.err.testSeam",
            json!({}),
            format!("{} is not the seam's tarball", verify::to_hex(&digest)),
        ));
    }
    job.record["total"] = json!(assets.tarball.size);
    job.record["tag"] = json!(format!("v{}", job.version));
    job.set("downloading");
    prune_work(job);
    fs::create_dir_all(&job.work).map_err(|e| io_end("io", &job.work, &e))?;
    let _ = fs::set_permissions(&job.work, fs::Permissions::from_mode(0o700));
    // the .sha256 first: it must name the exact tarball with GitHub's own hash, or nothing more is fetched
    let mut sha = Vec::new();
    let answer = job
        .shared
        .client
        .get(
            &assets.sha256.url,
            SMALL_ACCEPT,
            &SMALL_LIMITS,
            &mut sha,
            job.stop,
        )
        .map_err(|e| job.http_error(&e))?;
    if answer.status != 200 {
        return Err(job.http_error(&HttpError::Status(answer.status)));
    }
    let named = verify::parse_sha256_file(&sha, &assets.tarball.name).map_err(|e| {
        fail(
            "mismatch",
            "update.winpoint.err.mismatch",
            json!({}),
            format!("{}: {e:?}", assets.sha256.name),
        )
    })?;
    if named != digest {
        return Err(fail(
            "mismatch",
            "update.winpoint.err.mismatch",
            json!({}),
            format!(
                "{} says {}, GitHub says {}",
                assets.sha256.name,
                verify::to_hex(&named),
                verify::to_hex(&digest)
            ),
        ));
    }
    let _ = store::atomic(&job.work.join(&assets.sha256.name), &sha);
    let tarball = job.work.join(&assets.tarball.name);
    if !reuse(job, &tarball, &digest, assets.tarball.size)? {
        download(job, &assets.tarball, &tarball, &digest)?;
    }
    // unpacking: a bundle is never trusted again, every run unpacks the verified tarball fresh
    job.set("unpacking");
    let bundle = job.work.join("bundle");
    let partial = job.work.join("bundle.partial");
    let _ = fs::remove_dir_all(&bundle);
    let _ = fs::remove_dir_all(&partial);
    let top = format!("winpoint-{}", job.version);
    let unpacked = extract_release(&tarball, &partial, &top, BUNDLE_LIMITS, job.stop, job.lang)
        .map_err(|error| match error.code.as_str() {
            "cancelled" => job.stopped_or(End::Cancelled),
            // too little room for the unpacked bundle, measured before it was written (or the disk filled anyway)
            "no_space" => {
                let needed = error
                    .data
                    .as_ref()
                    .and_then(|d| d["size"].as_u64())
                    .unwrap_or(SPARE);
                space(job.lang, needed, &job.work)
            }
            _ => fail(
                "archive",
                "update.winpoint.err.archive",
                json!({}),
                error.message,
            ),
        })?;
    fs::rename(&unpacked, &bundle).map_err(|e| io_end("archive", &bundle, &e))?;
    let _ = fs::remove_dir_all(&partial);
    // preparing: the bundle is V's, it fits, and its own installer accepts this PC
    job.set("preparing");
    sane_bundle(&bundle, &job.version)?;
    let needed = installed_size(&bundle) + SPARE;
    let share = job.prefix.join("share");
    if free_space(&share).is_some_and(|free| free < needed) {
        return Err(space(job.lang, needed, &share));
    }
    if job.stop.load(Ordering::SeqCst) {
        return Err(job.stopped_or(End::Cancelled));
    }
    let verdict = run_check(job, &bundle)?;
    job.record["restart"] = json!(if verdict.extension_changed {
        "relogin"
    } else {
        "restart"
    });
    // installing: from here on no Cancel
    {
        let mut running = job.shared.job.lock().unwrap_or_else(|e| e.into_inner());
        if job.stop.load(Ordering::SeqCst) {
            drop(running);
            return Err(job.stopped_or(End::Cancelled));
        }
        if let Some(running) = running.as_mut() {
            running.committed = true;
        }
    }
    // job.json says `installing` before the installer can exist: from here on a crash of this owner, or a start that
    // fails half way, never counts as "nothing changed" while an installer may run (reattach looks for it)
    let script = bundle.join("scripts/install.py").display().to_string();
    job.record["installer"] = json!({"pid":null,"start":null,"script":script});
    job.record["installingAt"] = json!(store::now_ms());
    job.set("installing");
    let started = run_install(job, &bundle);
    // a start that failed may still have reached systemd: look for the installer before anything is concluded
    let pid = match &started {
        Ok(pid) => Some(*pid),
        Err(_) => look_for_installer(job.shared, &script, Instant::now() + FIND_WAIT),
    };
    let Some(pid) = pid else {
        if job.shared.stop.load(Ordering::SeqCst) {
            // the host shuts down while it looks: the next owner looks again (reattach)
            return Err(End::Shutdown);
        }
        unstarted(
            job.shared,
            &mut job.record,
            started.err().unwrap_or_default(),
        );
        return Ok(());
    };
    job.record["installer"] = json!({"pid":pid,"start":store::process_start(pid),"script":script});
    job.shared.write_job(&mut job.record);
    match wait_installer(job.shared, &job.record, &job.work) {
        Waited::Shutdown => Err(End::Shutdown),
        waited => {
            let code = match waited {
                Waited::Status(code) => Some(code),
                _ => None,
            };
            conclude(job.shared, &mut job.record, code);
            Ok(())
        }
    }
}
/// Look for the installer of `script` until `until` (every 200 ms, at least once; not past the host's shutdown).
fn look_for_installer(shared: &Shared, script: &str, until: Instant) -> Option<u32> {
    loop {
        if let Some(pid) = find_installer(script) {
            return Some(pid);
        }
        if Instant::now() >= until || shared.stop.load(Ordering::SeqCst) {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
/// No installer runs for this `installing` record: its start failed, or it ended before anyone saw its pid. Its
/// status file decides; else its log (install.py opens it before it looks at anything): there, it ran and stopped
/// (`interrupted`, Try again rolls forward); none, it never ran (`failed`, nothing changed).
fn unstarted(shared: &Shared, record: &mut Value, detail: String) {
    let work = work_of(shared, record);
    if let Some(code) = read_status(&work) {
        conclude(shared, record, Some(code));
    } else if fs::symlink_metadata(work.join("install.log")).is_ok() {
        conclude(shared, record, None);
    } else {
        let _ = fs::remove_dir_all(work.join("bundle"));
        let error = Failure {
            code: "installer",
            key: "update.winpoint.err.installer",
            params: json!({"code": "-"}),
            detail,
        };
        finish(
            shared,
            record,
            "failed",
            Some(&error),
            Some("nothingChanged"),
        );
    }
}
fn work_of(shared: &Shared, record: &Value) -> PathBuf {
    shared
        .store
        .dir()
        .join(WORK)
        .join(record["version"].as_str().unwrap_or(""))
}
/// The tarballs in a version's work folder (the verified download).
fn remove_tarballs(work: &Path) {
    for entry in fs::read_dir(work).into_iter().flatten().flatten() {
        if entry.path().extension().is_some_and(|x| x == "gz") {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// GitHub's release list again, at the job's start: this answer's assets for the clicked version (stale when it is no
/// longer the one offered). A successful answer is also stored as a check (no notification).
fn fresh_release(job: &mut Job) -> Result<feed::Assets, End> {
    let shared = job.shared;
    let mut body = Vec::new();
    let response = shared
        .client
        .get(
            &shared.origin.feed_url(),
            check::ACCEPT,
            &check::FEED_LIMITS,
            &mut body,
            job.stop,
        )
        .map_err(|e| job.http_error(&e))?;
    let now = store::now_ms();
    let fetched = check::classify(&response, &body, &shared.origin, &shared.arch, now);
    // the answer is a check like any other (no notification): the card shows what GitHub said now
    check::record(
        shared,
        &shared.read_state(),
        &fetched,
        check::Trigger::Manual,
    );
    let all = match &fetched {
        check::Fetched::Releases(all) => all.clone(),
        check::Fetched::Failed(failure) => {
            return Err(End::Failed {
                error: failure.clone(),
                outcome: "nothingChanged",
            });
        }
        check::Fetched::RateLimited { .. } => {
            return Err(fail(
                "rateLimited",
                "update.winpoint.err.rateLimited",
                json!({}),
                format!("HTTP {}", response.status),
            ));
        }
        _ => {
            return Err(fail(
                "unreachable",
                "update.winpoint.err.unreachable",
                json!({}),
                format!("HTTP {}", response.status),
            ));
        }
    };
    let current = shared.current();
    let chosen = feed::choose(&all, &current)
        .filter(|r| r.version == job.version)
        .ok_or_else(|| {
            fail(
                "stale",
                "update.winpoint.err.stale",
                json!({}),
                String::new(),
            )
        })?;
    let assets = chosen.assets.clone().ok_or_else(|| {
        fail(
            "noAsset",
            "update.winpoint.reason.noAsset",
            json!({"arch": shared.arch}),
            String::new(),
        )
    })?;
    if blocked_reason(&shared.read_state(), &job.version).is_some() {
        return Err(fail(
            "stale",
            "update.winpoint.err.stale",
            json!({}),
            "blocked".into(),
        ));
    }
    Ok(assets)
}
fn no_digest() -> End {
    fail(
        "noDigest",
        "update.winpoint.reason.noDigest",
        json!({}),
        String::new(),
    )
}
fn io_end(code: &'static str, path: &Path, error: &io::Error) -> End {
    let key = if code == "archive" {
        "update.winpoint.err.archive"
    } else {
        "update.winpoint.err.io"
    };
    fail(code, key, json!({}), format!("{}: {error}", path.display()))
}
fn space(lang: Lang, needed: u64, path: &Path) -> End {
    fail(
        "space",
        "update.winpoint.err.space",
        json!({"size": size_text(lang, needed), "path": path.display().to_string()}),
        format!("{needed} bytes"),
    )
}

/// Remove every other version's folder under `work/` (only the newest V is kept).
fn prune_work(job: &Job) {
    let Ok(entries) = fs::read_dir(job.shared.store.dir().join(WORK)) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path() != job.work {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Try again: a tarball of an earlier run is used again only when it still hashes to the fresh digest.
fn reuse(job: &mut Job, tarball: &Path, digest: &verify::Hash, size: u64) -> Result<bool, End> {
    if !fs::symlink_metadata(tarball).is_ok_and(|m| m.is_file()) {
        return Ok(false);
    }
    job.set("verifying");
    match verify::hash_file(tarball, job.stop) {
        Ok((hash, length)) if hash == *digest && length == size => Ok(true),
        Ok(_) => {
            let _ = fs::remove_file(tarball);
            Ok(false)
        }
        Err(_) if job.stop.load(Ordering::SeqCst) => Err(job.stopped_or(End::Cancelled)),
        Err(_) => {
            let _ = fs::remove_file(tarball);
            Ok(false)
        }
    }
}

/// The tarball's bytes: into the `.part` file and into SHA-256, with the job's progress.
struct Download<'a, 'j> {
    file: File,
    hasher: verify::Hasher,
    job: &'a mut Job<'j>,
    total: u64,
    /// a write found the disk full (the job then says `space`, not that GitHub cannot be reached)
    full: bool,
}
fn disk_full(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ENOSPC) || error.kind() == io::ErrorKind::StorageFull
}
impl Sink for Download<'_, '_> {
    fn data(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Err(error) = self.file.write_all(bytes) {
            self.full = disk_full(&error);
            return Err(error);
        }
        self.hasher.update(bytes);
        Ok(())
    }
    fn progress(&mut self, done: u64, _total: Option<u64>) {
        let total = self.total;
        self.job.progress(done, total, false);
    }
}
fn download(
    job: &mut Job,
    asset: &feed::Asset,
    tarball: &Path,
    digest: &verify::Hash,
) -> Result<(), End> {
    job.set("downloading");
    let part = tarball.with_file_name(format!("{}.part", asset.name));
    let _ = fs::remove_file(&part);
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&part)
        .map_err(|e| io_end("io", &part, &e))?;
    let limits = http::Limits {
        max_body: asset.size,
        stall: TARBALL_STALL,
        deadline: None,
    };
    let shared: &Shared = job.shared;
    let (client, url, stop) = (&shared.client, asset.url.clone(), job.stop);
    let mut sink = Download {
        file,
        hasher: verify::Hasher::new(),
        job,
        total: asset.size,
        full: false,
    };
    sink.job.progress(0, asset.size, true);
    let result = client.get(&url, SMALL_ACCEPT, &limits, &mut sink, stop);
    let Download {
        file,
        hasher,
        job,
        full,
        ..
    } = sink;
    let synced = file.sync_all();
    drop(file);
    let full = full || synced.as_ref().is_err_and(disk_full);
    let end = match result {
        _ if full => Some(space(job.lang, asset.size.saturating_add(SPARE), &job.work)),
        Ok(response) if response.status == 200 => None,
        Ok(response) => Some(job.http_error(&HttpError::Status(response.status))),
        Err(HttpError::TooLarge | HttpError::Protocol("truncated")) => {
            Some(damaged(asset, "length"))
        }
        Err(error) => Some(job.http_error(&error)),
    };
    if let Some(end) = end {
        let _ = fs::remove_file(&part);
        return Err(end);
    }
    job.set("verifying");
    let length = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    let hash = hasher.finish();
    if synced.is_err() || length != asset.size || hash != *digest {
        let _ = fs::remove_file(&part);
        return Err(damaged(
            asset,
            &format!("{length} bytes, sha256 {}", verify::to_hex(&hash)),
        ));
    }
    job.progress(length, asset.size, true);
    fs::rename(&part, tarball).map_err(|e| {
        let _ = fs::remove_file(&part);
        io_end("io", tarball, &e)
    })
}
/// The download is not the release GitHub described: it is deleted at once.
fn damaged(asset: &feed::Asset, detail: &str) -> End {
    fail(
        "checksum",
        "update.winpoint.err.checksum",
        json!({}),
        format!("{}: {detail}", asset.name),
    )
}

/// The unpacked bundle is V's (nothing in it runs for this): its Cargo.toml, its extension's version-name, the
/// version marker in bin/win11-panel, and scripts/install.py a regular file.
fn sane_bundle(bundle: &Path, version: &Version) -> Result<(), End> {
    let v = version.to_string();
    let bad = |what: &str| {
        fail(
            "bundle",
            "update.winpoint.err.bundle",
            json!({"version": v}),
            what.to_owned(),
        )
    };
    let regular = |path: &Path| fs::symlink_metadata(path).is_ok_and(|m| m.is_file());
    let cargo = fs::read_to_string(bundle.join("Cargo.toml")).unwrap_or_default();
    if !workspace_version(&cargo).is_some_and(|found| found == v) {
        return Err(bad("Cargo.toml [workspace.package] version"));
    }
    let metadata: Value = fs::read(bundle.join("extension/metadata.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or(Value::Null);
    if metadata["version-name"] != v {
        return Err(bad("extension/metadata.json version-name"));
    }
    let panel = bundle.join("bin/win11-panel");
    let marker = format!("WinPoint {v}\n");
    let marked = regular(&panel)
        && fs::read(&panel).is_ok_and(|bytes| {
            bytes
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
        });
    if !marked {
        return Err(bad("bin/win11-panel marker"));
    }
    if !regular(&bundle.join("scripts/install.py")) {
        return Err(bad("scripts/install.py"));
    }
    Ok(())
}
/// `version = "X"` of the `[workspace.package]` table of a Cargo.toml (a line scan).
pub(crate) fn workspace_version(text: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[workspace.package]";
            continue;
        }
        if inside
            && let Some(rest) = line.strip_prefix("version")
            && let Some(value) = rest.trim_start().strip_prefix('=')
        {
            let value = value.trim();
            return value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_owned);
        }
    }
    None
}
/// What the installer copies into the prefix (the old payload and programs stay until it commits).
fn installed_size(bundle: &Path) -> u64 {
    [
        "vendor/icons",
        "vendor/icon-source",
        "vendor/ui/assets",
        "vendor/config",
        "bin",
        "extension",
    ]
    .iter()
    .map(|part| du(&bundle.join(part)))
    .sum()
}
fn du(path: &Path) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    fs::read_dir(path)
        .map(|entries| entries.flatten().map(|e| du(&e.path())).sum())
        .unwrap_or(0)
}

/// The installer's environment (D-G74's frozen contract): exactly these variables.
fn installer_env(bundle: &Path, prefix: &Path) -> Vec<(OsString, OsString)> {
    let home = glib::home_dir();
    let user: OsString = glib::user_name();
    let mut env: Vec<(OsString, OsString)> = vec![
        ("HOME".into(), home.clone().into()),
        ("USER".into(), user.clone()),
        ("LOGNAME".into(), user),
        ("PATH".into(), "/usr/bin:/bin".into()),
        ("LANG".into(), "C.UTF-8".into()),
        ("XDG_DATA_HOME".into(), glib::user_data_dir().into()),
        ("XDG_CONFIG_HOME".into(), glib::user_config_dir().into()),
        ("XDG_CACHE_HOME".into(), glib::user_cache_dir().into()),
        ("XDG_RUNTIME_DIR".into(), glib::user_runtime_dir().into()),
        ("WIN11_BUILD_DIR".into(), bundle.join("bin").into()),
        ("WIN11_NO_SESSION".into(), "1".into()),
    ];
    if prefix != home.join(".local") {
        env.push(("WIN11_INSTALL_PREFIX".into(), prefix.into()));
    }
    env
}
fn spawner(shared: &Shared) -> Arc<dyn Spawner> {
    shared
        .hooks
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .spawner
        .clone()
        .unwrap_or_else(|| Arc::new(Scope))
}

/// What `install.py --check` said yes to.
struct Verdict {
    extension_changed: bool,
}
/// `install.py <bundle> --lang L --no-session --check` (nothing written; one JSON line). A refusal for this PC blocks
/// V (state.json `blocked`).
fn run_check(job: &mut Job, bundle: &Path) -> Result<Verdict, End> {
    let mut command = Command::new(PYTHON);
    command
        .args(PYTHON_FLAGS)
        .arg(bundle.join("scripts/install.py"))
        .arg(bundle)
        .args(["--lang", job.lang.code(), "--no-session", "--check"])
        .current_dir(&job.work);
    let log_path = job.work.join("check.log");
    let log = File::create(&log_path).map_err(|e| io_end("io", &log_path, &e))?;
    let env = installer_env(bundle, &job.prefix);
    let stopped =
        |job: &Job| job.stop.load(Ordering::SeqCst) || job.shared.stop.load(Ordering::SeqCst);
    let answer = spawner(job.shared).output(command, env, log, CHECK_TIMEOUT, job.stop);
    // a Cancel (or the shutdown) while it ran: it was killed, and a verdict that came anyway blocks nothing
    if stopped(job) {
        return Err(job.stopped_or(End::Cancelled));
    }
    let (code, stdout) = answer.map_err(|error| {
        fail(
            "installer",
            "update.winpoint.err.installer",
            json!({"code": "--check"}),
            error,
        )
    })?;
    let text = String::from_utf8_lossy(&stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("");
    let answer: Value = serde_json::from_str(line).unwrap_or(Value::Null);
    let detail = || {
        let tail = tail_of(&log_path, 1024);
        format!("--check exit {code:?}: {line} {tail}")
            .trim()
            .to_owned()
    };
    match (&answer["ok"], code) {
        (Value::Bool(true), Some(0)) => {
            if answer["version"] != job.version.to_string() {
                return Err(fail(
                    "bundle",
                    "update.winpoint.err.bundle",
                    json!({"version": job.version.to_string()}),
                    detail(),
                ));
            }
            let system: Vec<String> = answer["systemFiles"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            if !system.is_empty() {
                let reason = block(job, "manual", json!({"systemFiles": system}), detail());
                return Err(reason);
            }
            Ok(Verdict {
                extension_changed: answer["extensionChanged"] == true,
            })
        }
        (Value::Bool(false), Some(1)) => {
            let raw = answer["code"].as_str().unwrap_or("");
            let refusal = raw.strip_prefix("install.").unwrap_or(raw);
            let params = answer["params"].clone();
            let message = answer["message"].as_str().unwrap_or("").to_owned();
            Err(match refusal {
                "gnomeMismatch" => block(
                    job,
                    "incompatible",
                    json!({"kind":"gnome","needs":text_param(&params["needs"]),
                           "running":text_param(&params["running"])}),
                    message,
                ),
                "abi" => block(
                    job,
                    "incompatible",
                    json!({"kind":"abi","binary":text_param(&params["binary"])}),
                    format!("{message}\n{}", text_param(&params["detail"]))
                        .trim()
                        .to_owned(),
                ),
                "missingParts" => {
                    // install.py names the packages it knows; none known (an empty string): the parts themselves
                    let packages = Some(text_param(&params["packages"]))
                        .filter(|p| !p.trim().is_empty())
                        .unwrap_or_else(|| text_param(&params["missing"]));
                    block(job, "missingParts", json!({"packages": packages}), message)
                }
                "busy" => fail("busy", "update.winpoint.err.busy", json!({}), message),
                _ => fail(
                    "refused",
                    "update.winpoint.err.refused",
                    json!({}),
                    format!("{refusal}: {message}"),
                ),
            })
        }
        _ => Err(fail(
            "installer",
            "update.winpoint.err.installer",
            json!({"code": code.map_or("?".to_owned(), |c| c.to_string())}),
            detail(),
        )),
    }
}
/// A parameter as text: a string, the items of a list joined by spaces, or a number.
fn text_param(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .or_else(|| Some(v.to_string()))
            })
            .collect::<Vec<_>>()
            .join(" "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}
/// Record V as blocked on this PC (`incompatible`, `manual`, `missingParts`) and end the job with that reason.
fn block(job: &mut Job, reason: &'static str, params: Value, detail: String) -> End {
    let version = job.version.to_string();
    let mut params = params;
    params["version"] = json!(version);
    let record = json!({"version":version,"code":reason,"params":params,"detail":detail});
    job.shared.update_state(|state| state["blocked"] = record);
    let (_, key) = reason_key(reason, &params);
    fail(reason, key, params, detail)
}
/// The text of a block reason (`incompatible` has two: GNOME's version or the system libraries).
pub(crate) fn reason_key(reason: &str, params: &Value) -> (&'static str, &'static str) {
    match (reason, params["kind"].as_str()) {
        ("incompatible", Some("gnome")) => ("incompatible", "update.winpoint.reason.gnome"),
        ("incompatible", _) => ("incompatible", "update.winpoint.reason.abi"),
        ("manual", _) => ("manual", "update.winpoint.reason.manual"),
        _ => ("missingParts", "update.winpoint.reason.missingParts"),
    }
}
/// state.json `blocked` when it is for `version`.
pub(crate) fn blocked_reason(state: &Value, version: &Version) -> Option<Value> {
    let blocked = &state["blocked"];
    (blocked["version"].as_str() == Some(&version.to_string())
        && matches!(
            blocked["code"].as_str(),
            Some("incompatible" | "manual" | "missingParts")
        ))
    .then(|| blocked.clone())
}

/// `install.py <bundle> --lang L --no-session --status <work>/install.status --log <work>/install.log`, its own
/// process: its pid, or why its start failed (it may have started all the same: [`install_steps`] looks).
fn run_install(job: &mut Job, bundle: &Path) -> Result<u32, String> {
    let status = job.work.join("install.status");
    let log = job.work.join("install.log");
    let _ = fs::remove_file(&status);
    let _ = fs::remove_file(&log);
    let mut command = Command::new(PYTHON);
    command
        .args(PYTHON_FLAGS)
        .arg(bundle.join("scripts/install.py"))
        .arg(bundle)
        .args(["--lang", job.lang.code(), "--no-session", "--status"])
        .arg(&status)
        .arg("--log")
        .arg(&log)
        .current_dir(&job.work);
    let env = installer_env(bundle, &job.prefix);
    spawner(job.shared).spawn(command, "winpoint-install", env)
}

/// What the wait for the installer saw.
enum Waited {
    /// its `--status` file
    Status(i32),
    /// it ended without one (SIGKILL, a crash)
    Gone,
    /// the host shuts down while it runs (it goes on; the next owner reattaches)
    Shutdown,
}
fn read_status(work: &Path) -> Option<i32> {
    let text = fs::read_to_string(work.join("install.status")).ok()?;
    text.strip_suffix('\n')?.trim().parse().ok()
}
/// The installer's clock in one watch: its age when the watch started (the kernel's, [`running_for`], so a new
/// owner's watch keeps it) plus the watch's own monotonic time, which leaves a suspend out (a lid closed during an
/// install is no hung installer). SIGTERM once that age reaches `timeout`, however far past it the installer is
/// when first seen; SIGKILL only `kill_after` after the SIGTERM this watch sent itself, so install.py always gets
/// the signal that rolls it back first.
pub(crate) struct Clock {
    /// the installer's age at the watch's start
    base: Duration,
    timeout: Duration,
    kill_after: Duration,
    /// when (in the watch's time) this watch sent SIGTERM
    termed: Option<Duration>,
}
impl Clock {
    pub(crate) fn new(base: Duration, timeout: Duration, kill_after: Duration) -> Self {
        Self {
            base,
            timeout,
            kill_after,
            termed: None,
        }
    }
    /// The signal due `watched` into the watch (its monotonic time since it started), if any.
    pub(crate) fn due(&mut self, watched: Duration) -> Option<i32> {
        match self.termed {
            None if self.base + watched >= self.timeout => {
                self.termed = Some(watched);
                Some(libc::SIGTERM)
            }
            Some(at) if watched.saturating_sub(at) >= self.kill_after => Some(libc::SIGKILL),
            _ => None,
        }
    }
}
/// Poll the status file every 500 ms. SIGTERM once the installer has run 15 minutes (install.py rolls back and writes
/// 3); 3 minutes after that SIGTERM SIGKILL, for a rollback that hangs (it then ends `interrupted`, and Try again
/// rolls forward): [`Clock`]. The guards block the sign-out and shutdown that would otherwise stop the installer, so
/// nothing else ends it.
fn wait_installer(shared: &Shared, record: &Value, work: &Path) -> Waited {
    let installer = &record["installer"];
    let (timeout, kill_after) = {
        let hooks = shared.hooks.lock().unwrap_or_else(|e| e.into_inner());
        (
            hooks.install_timeout.unwrap_or(INSTALL_TIMEOUT),
            hooks.kill_after.unwrap_or(KILL_AFTER),
        )
    };
    let watched = Instant::now();
    let mut clock = Clock::new(
        running_for(installer).unwrap_or_default(),
        timeout,
        kill_after,
    );
    let pid = installer["pid"]
        .as_u64()
        .and_then(|p| i32::try_from(p).ok())
        .filter(|p| *p > 0);
    loop {
        if let Some(code) = read_status(work) {
            return Waited::Status(code);
        }
        if !store::owner_alive(installer) {
            // it may have written the file just before it ended
            std::thread::sleep(Duration::from_millis(100));
            return read_status(work).map_or(Waited::Gone, Waited::Status);
        }
        if shared.stop.load(Ordering::SeqCst) {
            return Waited::Shutdown;
        }
        let due = clock.due(watched.elapsed());
        // a signal only for the very process of the record (its pid and start time, checked just before)
        if let Some(number) = due
            && let Some(pid) = pid
            && store::owner_alive(installer)
        {
            unsafe {
                libc::kill(pid, number);
            }
        }
        if due == Some(libc::SIGKILL) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while store::owner_alive(installer) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            return read_status(work).map_or(Waited::Gone, Waited::Status);
        }
        std::thread::sleep(POLL);
    }
}
/// The end of an install: the status code (None: the installer is gone without one).
fn conclude(shared: &Shared, record: &mut Value, code: Option<i32>) {
    let version = record["version"].as_str().unwrap_or("").to_owned();
    let work = work_of(shared, record);
    record["logTail"] = json!(tail_of(&work.join("install.log"), LOG_TAIL));
    let installer = |code: i32, outcome: &'static str| {
        (
            Failure {
                code: "installer",
                key: "update.winpoint.err.installer",
                params: json!({"code": code.to_string()}),
                detail: String::new(),
            },
            outcome,
        )
    };
    let ended = match code {
        Some(0) => {
            let prefix = PathBuf::from(record["prefix"].as_str().unwrap_or(""));
            let found = super::version_file(&prefix.join(super::PAYLOAD_VERSION));
            if found.as_deref() == Some(version.as_str()) {
                None
            } else {
                Some((
                    Failure {
                        code: "versionMismatch",
                        key: "update.winpoint.err.versionMismatch",
                        params: json!({"found": found.unwrap_or_else(|| "-".into()), "version": version}),
                        detail: String::new(),
                    },
                    "unknown",
                ))
            }
        }
        // 1: refused before any change; 2: a bad argument (nothing ran)
        Some(code @ (1 | 2)) => Some(installer(code, "nothingChanged")),
        Some(3) => Some(installer(3, "rolledBack")),
        Some(other) => Some(installer(other, "unknown")),
        None => {
            finish(shared, record, "interrupted", None, Some("unknown"));
            let _ = fs::remove_dir_all(work.join("bundle"));
            return;
        }
    };
    let _ = fs::remove_dir_all(work.join("bundle"));
    match ended {
        None => {
            // the installed release: the tarball is no use any more (the log stays until the job is cleared)
            remove_tarballs(&work);
            finish(shared, record, "installed", None, None);
            if let Some(version) = Version::parse(&version) {
                shared.notify_installed(&version, record["restart"] == "relogin");
            }
        }
        Some((error, outcome)) => finish(shared, record, "failed", Some(&error), Some(outcome)),
    }
}
/// The last `max` bytes of a file as text (a cut character dropped).
fn tail_of(path: &Path, max: u64) -> String {
    let Ok(mut file) = File::open(path) else {
        return String::new();
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut bytes = Vec::new();
    use std::io::Seek;
    let _ = file.seek(io::SeekFrom::Start(length.saturating_sub(max)));
    let _ = file.take(max).read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes);
    text.trim_start_matches('\u{fffd}').to_owned()
}
fn finish(
    shared: &Shared,
    record: &mut Value,
    state: &str,
    error: Option<&Failure>,
    outcome: Option<&str>,
) {
    record["state"] = json!(state);
    record["finishedAt"] = json!(store::now_ms());
    record["error"] = error.map_or(Value::Null, Failure::json);
    record["outcome"] = json!(outcome);
    shared.write_job(record);
}

// ------------------------------------------------------------------------------------------------ the owner's start

/// When the owner takes over (`at_start`) or reads its status: end what the last owner left. `installing` with a
/// live installer gets a watcher thread (the wait of [`wait_installer`]); with no pid yet (the last owner died while
/// it started one) the watcher first looks for it ([`find_installer`], up to 30 s after `installingAt`), else
/// [`unstarted`] decides; with a dead one its status file decides (none: `interrupted`); an earlier step whose owner
/// died is `failed` (nothing outside `work/` changed), and what that step was writing (`bundle.partial`, a `.part`)
/// goes. At the owner's start an `installed` job is cleared once it is done: `restart` when this owner runs V,
/// `relogin` when the session key differs, either once another install replaced V (the payload's VERSION is not
/// V). It never starts a program.
pub(crate) fn reattach(shared: &Arc<Shared>, at_start: bool) {
    if shared
        .job
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_some()
    {
        return;
    }
    let Some(mut record) = shared.store.read(JOB).filter(Value::is_object) else {
        return;
    };
    match state_of(&record) {
        "installing" => {
            let Ok(Some(lock)) = shared.store.try_lock(JOB_LOCK) else {
                return;
            };
            let work = work_of(shared, &record);
            let pending = record["installer"].is_object() && record["installer"]["pid"].is_null();
            if pending || store::owner_alive(&record["installer"]) {
                let id = record["jobId"].as_str().unwrap_or("").to_owned();
                {
                    let mut running = shared.job.lock().unwrap_or_else(|e| e.into_inner());
                    if running.is_some() {
                        return;
                    }
                    *running = Some(Running {
                        id,
                        stop: Arc::new(AtomicBool::new(false)),
                        committed: true,
                    });
                }
                record["owner"] = store::current_owner();
                shared.write_job(&mut record);
                let thread = shared.clone();
                let spawned = std::thread::Builder::new()
                    .name("winpoint-watch".into())
                    .spawn(move || {
                        let _lock = lock;
                        let _running = Clear(&thread);
                        let context = glib::MainContext::new();
                        let _ = context.with_thread_default(|| watch(&thread, &mut record, &work));
                    });
                if spawned.is_err() {
                    shared.job.lock().unwrap_or_else(|e| e.into_inner()).take();
                }
            } else {
                let code = read_status(&work);
                conclude(shared, &mut record, code);
            }
        }
        s if BEFORE_INSTALL.contains(&s) && !store::owner_alive(&record["owner"]) => {
            let Ok(Some(_lock)) = shared.store.try_lock(JOB_LOCK) else {
                return;
            };
            let work = work_of(shared, &record);
            // what the dead owner's step was writing: the unpacked tree and a download's .part
            let _ = fs::remove_dir_all(work.join("bundle"));
            let _ = fs::remove_dir_all(work.join("bundle.partial"));
            for entry in fs::read_dir(&work).into_iter().flatten().flatten() {
                if entry.path().extension().is_some_and(|x| x == "part") {
                    let _ = fs::remove_file(entry.path());
                }
            }
            let error = Failure {
                code: "ownerStopped",
                key: "update.winpoint.err.ownerStopped",
                params: json!({}),
                detail: format!("{s}, owner {}", record["owner"]),
            };
            finish(
                shared,
                &mut record,
                "failed",
                Some(&error),
                Some("nothingChanged"),
            );
        }
        "installed" if at_start => {
            let hooks = shared.hooks.lock().unwrap_or_else(|e| e.into_inner());
            let own = hooks
                .own_version
                .clone()
                .or_else(|| Version::parse(crate::VERSION));
            let key = hooks.session_key.clone().unwrap_or_else(session_key);
            drop(hooks);
            let version = record["version"].as_str().and_then(Version::parse);
            // another install (scripts/install.sh, another version) took its place: nothing left to finish
            let replaced = shared
                .installed_version()
                .is_some_and(|found| record["version"].as_str() != Some(found.as_str()));
            let done = replaced
                || match record["restart"].as_str() {
                    Some("relogin") => record["sessionKey"] != json!(key),
                    _ => own.is_some() && own == version,
                };
            if done {
                clear(shared);
            }
        }
        _ => {}
    }
}
/// A new owner's watch of an install the last owner left: the installer by its pid, or first found by its argv
/// (it may still be starting), then [`wait_installer`] and [`conclude`].
fn watch(shared: &Shared, record: &mut Value, work: &Path) {
    if record["installer"]["pid"].is_null() {
        let script = record["installer"]["script"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let asked = record["installingAt"].as_u64().unwrap_or(0);
        let left = SPAWN_GRACE_MS.saturating_sub(store::now_ms().saturating_sub(asked));
        let found = if script.is_empty() {
            None
        } else {
            look_for_installer(
                shared,
                &script,
                Instant::now() + Duration::from_millis(left),
            )
        };
        let Some(pid) = found else {
            if !shared.stop.load(Ordering::SeqCst) {
                unstarted(
                    shared,
                    record,
                    "WinPoint stopped while the installer started".into(),
                );
            }
            return;
        };
        record["installer"] = json!({"pid":pid,"start":store::process_start(pid),"script":script});
        shared.write_job(record);
    }
    match wait_installer(shared, record, work) {
        Waited::Shutdown => {}
        Waited::Status(code) => conclude(shared, record, Some(code)),
        Waited::Gone => conclude(shared, record, None),
    }
}
/// The update is complete: `job.json` and `work/` go.
pub(crate) fn clear(shared: &Shared) {
    let _ = fs::remove_dir_all(shared.store.dir().join(WORK));
    shared.store.remove(JOB);
    shared.bump();
}

/// Cancel (`winpoint.cancel {jobId}`): only this owner's running job before the installer starts.
pub(crate) fn cancel(shared: &Shared, id: &str) -> Result<(), &'static str> {
    let running = shared.job.lock().unwrap_or_else(|e| e.into_inner());
    match running.as_ref() {
        Some(job) if job.id == id && !job.committed => {
            job.stop.store(true, Ordering::SeqCst);
            Ok(())
        }
        Some(job) if job.id == id => Err("cannot_cancel"),
        _ => Err("no_job"),
    }
}

/// A new job's record (state `downloading`; job.json schema 1, design 5.3). `from` is the version this host compares
/// with at the click, which stays WinPoint's version while the install does not end `installed`, and `found` the
/// higher of this host's own version and the payload's VERSION then (Shared::found), or V when the record this one
/// replaces is an install of V that may be unfinished: it says whether this job is a Try again of such an install
/// (winpoint.rs `current_from`, `unconfirmed`).
pub(crate) fn new_record(
    id: &str,
    release: &feed::Release,
    from: &Version,
    found: &Version,
    prefix: &Path,
    lang: Lang,
    session_key: Option<String>,
) -> Value {
    let size = release.assets.as_ref().map_or(0, |a| a.tarball.size);
    json!({"schema":1,"jobId":id,"version":release.version.to_string(),"from":from.to_string(),"found":found.to_string(),
           "tag":release.tag,"state":"downloading","done":0,"total":size,"startedAt":store::now_ms(),"finishedAt":null,
           "owner":store::current_owner(),"installer":null,"restart":null,"sessionKey":session_key,
           "prefix":prefix.display().to_string(),"lang":lang.code(),"error":null,"outcome":null,"logTail":null})
}

/// The cache and its free space for the tarball (design 5.3 precondition 6): its size and 64 MiB, or only the 64 MiB
/// when a complete tarball of an earlier run is in `work/V` (Try again checks it against the fresh digest and
/// downloads nothing). The unpacked bundle is measured before it is written (extract_release).
pub(crate) fn cache_space(
    shared: &Shared,
    version: &Version,
    tarball: &feed::Asset,
    lang: Lang,
) -> Result<(), crate::files::ApiError> {
    let dir = shared.store.dir();
    let earlier = dir.join(WORK).join(version.to_string()).join(&tarball.name);
    let reused =
        fs::symlink_metadata(&earlier).is_ok_and(|m| m.is_file() && m.len() == tarball.size);
    let needed = if reused { 0 } else { tarball.size }.saturating_add(SPARE);
    if free_space(dir).is_some_and(|free| free < needed) {
        let mut error = crate::files::ApiError::tr(
            "space",
            lang,
            "update.winpoint.err.space",
            &json!({"size": size_text(lang, needed), "path": dir.display().to_string()}),
        );
        error.data = Some(json!({"size": needed, "path": dir}));
        return Err(error);
    }
    Ok(())
}
