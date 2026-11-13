use crate::files::ApiError;
use serde_json::{Value, json};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
pub(super) const MAX_SOURCE: u64 = 4 << 30;
pub(super) fn t<'a>(lang: &str, nl: &'a str, en: &'a str) -> &'a str {
    if lang == "en" { en } else { nl }
}
pub(super) fn err(lang: &str, code: &str, nl: &str, en: &str) -> ApiError {
    ApiError::new(code, t(lang, nl, en))
}
pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub(super) fn id() -> io::Result<String> {
    let mut b = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut b)?;
    Ok(b.iter().map(|b| format!("{b:02x}")).collect())
}
pub(super) fn token(v: &Value) -> Option<&str> {
    v.as_str()
        .filter(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(super) fn private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Software state must be private",
        ));
    }
    Ok(())
}
pub(super) fn user_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(path)?;
    let m = fs::symlink_metadata(path)?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid user application directory",
        ));
    }
    Ok(())
}
pub(super) struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
pub(super) fn lock(root: &Path) -> io::Result<Lock> {
    private_dir(root)?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join(".lock"))?;
    let m = f.metadata()?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.nlink() != 1
        || m.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid software lock",
        ));
    }
    loop {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(Lock(f));
        }
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}
pub(super) fn atomic(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| io::Error::other("No parent"))?;
    let temp = parent.join(format!(".{}.tmp", id()?));
    let r = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        f.write_all(bytes)?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        fs::rename(&temp, path)?;
        if let Ok(d) = File::open(parent) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if r.is_err() {
        let _ = fs::remove_file(temp);
    }
    r
}
pub(super) fn write_json(path: &Path, value: &Value) -> io::Result<()> {
    atomic(path, &serde_json::to_vec(value)?, 0o600)
}
pub(super) fn read_json(path: &Path, max: u64) -> io::Result<Value> {
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)?;
    let m = f.metadata()?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.mode() & 0o077 != 0
        || m.nlink() != 1
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Invalid software state",
        ));
    }
    let mut b = Vec::new();
    f.take(max + 1).read_to_end(&mut b)?;
    if b.len() as u64 > max {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Software state too large",
        ));
    }
    serde_json::from_slice(&b).map_err(io::Error::other)
}
pub(super) fn source(raw: &Value, lang: &str) -> Result<(PathBuf, File, Value), ApiError> {
    let raw = raw
        .as_str()
        .filter(|s| {
            !s.is_empty() && s.len() <= 16384 && !s.contains('\0') && Path::new(s).is_absolute()
        })
        .ok_or_else(|| {
            err(
                lang,
                "bad_args",
                "Kies een geldig installatiebestand.",
                "Choose a valid installation file.",
            )
        })?;
    let path = Path::new(raw).canonicalize().map_err(|_| {
        err(
            lang,
            "not_found",
            "Dit installatiebestand bestaat niet meer.",
            "This installation file no longer exists.",
        )
    })?;
    let f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|_| {
            err(
                lang,
                "no_access",
                "Dit installatiebestand kan niet worden gelezen.",
                "This installation file cannot be read.",
            )
        })?;
    let m = f.metadata().map_err(|_| {
        err(
            lang,
            "no_access",
            "Dit bestand kan niet worden gelezen.",
            "This file cannot be read.",
        )
    })?;
    if !m.is_file() || m.len() == 0 || m.len() > MAX_SOURCE {
        return Err(err(
            lang,
            "unsupported",
            "Kies een normaal installatiebestand van maximaal 4 GiB.",
            "Choose a regular installation file no larger than 4 GiB.",
        ));
    }
    let identity = json!({"path":path,"dev":m.dev(),"ino":m.ino(),"size":m.len(),"mtime":m.mtime(),"mtimeNsec":m.mtime_nsec()});
    Ok((path, f, identity))
}
pub(super) fn hash(mut file: File, cancel: Option<&AtomicBool>) -> io::Result<String> {
    let mut sum = glib::Checksum::new(glib::ChecksumType::Sha256)
        .ok_or_else(|| io::Error::other("SHA256 unavailable"))?;
    let mut block = [0u8; 128 * 1024];
    loop {
        if cancel.is_some_and(|c| c.load(Ordering::Acquire)) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }
        let n = file.read(&mut block)?;
        if n == 0 {
            break;
        }
        sum.update(&block[..n]);
    }
    sum.string()
        .map(|s| s.to_string())
        .ok_or_else(|| io::Error::other("SHA256 failed"))
}
pub(super) fn bounded(mut cmd: Command, timeout: Duration, max: usize) -> io::Result<Vec<u8>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("No child output"))?;
    let reader = std::thread::spawn(move || {
        let mut b = Vec::new();
        out.take(max as u64 + 1).read_to_end(&mut b)?;
        if b.len() > max {
            Err(io::Error::other("Metadata output too large"))
        } else {
            Ok(b)
        }
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait()? {
            Some(s) => break Some(s),
            None if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let b = reader
        .join()
        .map_err(|_| io::Error::other("Metadata reader failed"))??;
    if status.is_some_and(|s| s.success()) {
        Ok(b)
    } else {
        Err(io::Error::other("Metadata reader failed or timed out"))
    }
}
pub(super) fn clean(s: &str, max: usize) -> String {
    s.chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}
pub(super) fn package(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 200
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"+.-:".contains(&b))
}
pub(super) fn flatpak_id(s: &str) -> bool {
    s.len() <= 255
        && s.split('.').count() >= 3
        && s.split('.').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        })
}
pub(super) fn snap_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.as_bytes()[0].is_ascii_lowercase()
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_".contains(&b))
}
pub(super) fn protected(s: &str) -> bool {
    let s = s.split(':').next().unwrap_or(s);
    matches!(
        s,
        "apt"
            | "dpkg"
            | "systemd"
            | "systemd-sysv"
            | "dbus"
            | "dbus-daemon"
            | "libc6"
            | "packagekit"
            | "polkitd"
            | "policykit-1"
            | "gnome-shell"
            | "mutter"
            | "gdm3"
            | "ubuntu-desktop"
            | "ubuntu-desktop-minimal"
            | "ubuntu-session"
            | "gnome-session"
            | "linux-image-generic"
            | "linux-generic"
            | "snapd"
    ) || s.starts_with("linux-image-")
        || s.starts_with("libmutter-")
}

pub(super) fn atomic_new(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let parent = path.parent().ok_or_else(|| io::Error::other("No parent"))?;
    let temp = parent.join(format!(".{}.tmp", id()?));
    let r = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&temp)?;
        f.write_all(bytes)?;
        f.set_permissions(fs::Permissions::from_mode(mode))?;
        f.sync_all()?;
        let source = CString::new(temp.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let target = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
        if unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                target.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if r.is_err() {
        let _ = fs::remove_file(temp);
    }
    r
}
pub(super) fn try_lock(root: &Path) -> io::Result<Lock> {
    private_dir(root)?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(root.join(".lock"))?;
    let m = f.metadata()?;
    if !m.is_file()
        || m.uid() != unsafe { libc::geteuid() }
        || m.nlink() != 1
        || m.mode() & 0o077 != 0
    {
        return Err(io::Error::other("Invalid software lock"));
    }
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Lock(f))
}
