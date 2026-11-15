//! The update state files in `~/.cache/win11-gnome/update/` (design §4.5): directory 0700, files 0600, every
//! write a temp file + rename. The job lock, the owner lock and `owner.json`, the boot id and the persisted `rev`.
use super::paths::Paths;
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const SNAPSHOT: &str = "snapshot.json";
pub(crate) const JOB: &str = "job.json";
pub(crate) const JOBS: &str = "jobs.jsonl";
pub(crate) const JOB_LOCK: &str = "lock";
pub(crate) const OWNER_LOCK: &str = "owner.lock";
pub(crate) const OWNER: &str = "owner.json";
/// Largest state file we read (a snapshot with a few thousand packages stays far below it).
const MAX_FILE: u64 = 8 << 20;
/// `jobs.jsonl` keeps 90 days (the history match window).
const JOBS_DAYS: u64 = 90;
/// The real procfs: liveness of an owner (pid + start) is always checked against it, never a fixture.
const REAL_PROC: &str = "/proc";

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// 32 hex characters from /dev/urandom (job ids, nonces).
pub(crate) fn token() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub(crate) fn is_token(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn denied() -> io::Error {
    io::Error::from(io::ErrorKind::PermissionDenied)
}

/// A directory of ours, 0700, not a symlink.
pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || meta.mode() & 0o077 != 0 {
        return Err(denied());
    }
    Ok(())
}

/// An flock that is released when dropped (and by the kernel when the process ends).
#[derive(Debug)]
pub(crate) struct FileLock(File);
impl Drop for FileLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Store {
    dir: PathBuf,
}
impl Store {
    /// `<cache>/update`, created 0700.
    pub(crate) fn open(dir: PathBuf) -> io::Result<Self> {
        private_dir(&dir)?;
        Ok(Self { dir })
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
    /// A state file of ours (regular, 0600-ish, one link, ≤ 8 MiB), or None.
    pub(crate) fn read(&self, name: &str) -> Option<Value> {
        let bytes = read_private(&self.path(name), MAX_FILE).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
    /// Atomic write (temp file + rename), mode 0600.
    pub(crate) fn write(&self, name: &str, value: &Value) -> io::Result<()> {
        atomic(&self.path(name), &serde_json::to_vec(value)?)
    }
    #[allow(dead_code)] // job.json → jobs.jsonl once seen (GW3/GW4), upgrade.json (GW7)
    pub(crate) fn remove(&self, name: &str) {
        let _ = fs::remove_file(self.path(name));
    }
    /// The file's modification time in ms, if it exists.
    pub(crate) fn modified_ms(&self, name: &str) -> Option<u64> {
        let modified = fs::symlink_metadata(self.path(name)).ok()?.modified().ok()?;
        Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
    }
    /// One finished job per line; lines older than 90 days are dropped on each append.
    pub(crate) fn append_job(&self, entry: &Value, now: u64) -> io::Result<()> {
        let old = read_private(&self.path(JOBS), MAX_FILE).unwrap_or_default();
        let horizon = now.saturating_sub(JOBS_DAYS * 86_400_000);
        let mut out = String::new();
        for line in String::from_utf8_lossy(&old).lines() {
            if serde_json::from_str::<Value>(line)
                .is_ok_and(|v| v["finishedAt"].as_u64().is_some_and(|t| t >= horizon))
            {
                out.push_str(line);
                out.push('\n');
            }
        }
        out.push_str(&serde_json::to_string(entry)?);
        out.push('\n');
        atomic(&self.path(JOBS), out.as_bytes())
    }
    #[cfg_attr(not(test), allow(dead_code))] // update.history (GW4)
    pub(crate) fn jobs(&self) -> Vec<Value> {
        let bytes = read_private(&self.path(JOBS), MAX_FILE).unwrap_or_default();
        String::from_utf8_lossy(&bytes)
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }
    /// A non-blocking exclusive flock on `name` (created 0600). None = another process holds it.
    pub(crate) fn try_lock(&self, name: &str) -> io::Result<Option<FileLock>> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(self.path(name))?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } || meta.nlink() != 1 || meta.mode() & 0o077 != 0
        {
            return Err(denied());
        }
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                return Ok(Some(FileLock(file)));
            }
            let error = io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::EINTR) => continue,
                Some(libc::EWOULDBLOCK) => return Ok(None),
                _ => return Err(error),
            }
        }
    }
    /// Is `name` locked by anyone (us included)? Used by readers: a held job lock means a job runs.
    pub(crate) fn locked(&self, name: &str) -> bool {
        self.held(name).unwrap_or(false)
    }
    /// [`locked`](Self::locked), with the error of a lock file that is not ours alone (try_lock's checks) instead of
    /// "free"; a missing file is free and is not created.
    pub(crate) fn held(&self, name: &str) -> io::Result<bool> {
        if !self.path(name).exists() {
            return Ok(false);
        }
        Ok(self.try_lock(name)?.is_none())
    }
    /// The folder (`Store::open`'s).
    pub(crate) fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Read a file of ours with the checks of every state read: no symlink, regular, our uid, no group/other
/// access, one link, at most `max` bytes.
pub(crate) fn read_private(path: &Path, max: u64) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
        || meta.len() > max
    {
        return Err(denied());
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max {
        return Err(denied());
    }
    Ok(bytes)
}

/// Temp file in the same directory, 0600, fsync, rename, fsync the directory.
pub(crate) fn atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or_else(denied)?;
    let temp = parent.join(format!(
        ".{}-{}-{}.tmp",
        std::process::id(),
        now_ms(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// `/proc/sys/kernel/random/boot_id` (re-rooted in test mode).
pub(crate) fn boot_id(paths: &Paths) -> Option<String> {
    let text = fs::read_to_string(&paths.boot_id).ok()?;
    let id = text.trim();
    (!id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')).then(|| id.to_owned())
}

/// Field 22 (start time in clock ticks) of `/proc/<pid>/stat` of the real procfs.
pub(crate) fn process_start(pid: u32) -> Option<String> {
    process_start_in(Path::new(REAL_PROC), pid)
}
pub(crate) fn process_start_in(proc: &Path, pid: u32) -> Option<String> {
    let stat = fs::read_to_string(proc.join(pid.to_string()).join("stat")).ok()?;
    stat.rsplit_once(") ")?.1.split_whitespace().nth(19).map(str::to_owned)
}
/// `{pid, start}` of this process.
pub(crate) fn current_owner() -> Value {
    let pid = std::process::id();
    json!({"pid":pid,"start":process_start(pid)})
}
/// An owner `{pid, start}` whose process still exists with the same start time.
pub(crate) fn owner_alive(owner: &Value) -> bool {
    owner["pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .and_then(process_start)
        .is_some_and(|start| owner["start"] == start)
}

/// The owner election among Settings processes (design §4.1, pure apart from the flock).
#[derive(Debug)]
pub(crate) enum Election {
    /// we hold `owner.lock` for our whole life (the kernel drops it on exit or crash)
    Won(FileLock),
    /// another Settings process owns it: we are a mirror
    Lost,
}
pub(crate) fn elect(store: &Store) -> io::Result<Election> {
    match store.try_lock(OWNER_LOCK)? {
        Some(lock) => {
            let mut owner = current_owner();
            owner["since"] = json!(now_ms());
            store.write(OWNER, &owner)?;
            Ok(Election::Won(lock))
        }
        None => Ok(Election::Lost),
    }
}
/// The next `rev`: one more than both our last value and the snapshot's (so a new owner continues, design §4.1).
pub(crate) fn next_rev(store: &Store, current: u64) -> u64 {
    let persisted = store.read(SNAPSHOT).and_then(|s| s["rev"].as_u64()).unwrap_or(0);
    current.max(persisted) + 1
}
