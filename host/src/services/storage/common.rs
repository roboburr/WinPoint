use crate::{files::ApiError, i18n::Lang};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::unix::{
        fs::{MetadataExt, PermissionsExt},
        process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) fn now() -> f64 {
    let mut t = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 / 1e9
}
pub(crate) fn wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

pub(crate) fn read(path: impl AsRef<Path>) -> String {
    fs::read(path)
        .map(|v| String::from_utf8_lossy(&v).trim().to_owned())
        .unwrap_or_default()
}

pub(crate) fn names(path: impl AsRef<Path>) -> Vec<String> {
    fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}
pub(crate) fn int(s: impl AsRef<str>) -> Option<i64> {
    s.as_ref().trim().parse().ok()
}
pub(crate) fn num(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}
pub(crate) fn valint(v: &Value) -> Option<i64> {
    match v {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().filter(|f| f.is_finite()).map(|f| f as i64)),
        Value::Bool(b) => Some(i64::from(*b)),
        Value::String(s) => int(s),
        _ => None,
    }
}
pub(crate) fn truth(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
pub(crate) fn round(v: f64, n: i32) -> f64 {
    if !v.is_finite() {
        return v;
    }
    format!("{:.*}", n.max(0) as usize, v).parse().unwrap_or(v)
}

pub(crate) fn command_exists(name: &str) -> bool {
    which(name, None).is_some()
}
pub(crate) fn which(name: &str, path: Option<&str>) -> Option<PathBuf> {
    if name.contains('/') {
        let p = PathBuf::from(name);
        return executable(&p).then_some(p);
    }
    let paths = path
        .map(str::to_owned)
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_else(|_| "/bin:/usr/bin".into()));
    paths
        .split(':')
        .map(|d| Path::new(if d.is_empty() { "." } else { d }).join(name))
        .find(|p| executable(p))
}
pub(crate) fn executable(p: &Path) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
pub(crate) struct Run {
    pub ok: bool,
    pub out: String,
    pub err: String,
    pub rc: i32,
}
pub(crate) fn run(program: &str, args: &[&str], timeout: f64) -> Run {
    run_env(program, args, timeout, None)
}
pub(crate) fn run_env(
    program: &str,
    args: &[&str],
    timeout: f64,
    env: Option<&BTreeMap<String, String>>,
) -> Run {
    let mut c = Command::new(program);
    if let Some(env) = env {
        c.env_clear().envs(env);
    }
    c.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        c.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut p = match c.spawn() {
        Ok(p) => p,
        Err(e) => {
            return Run {
                ok: false,
                rc: if e.kind() == std::io::ErrorKind::NotFound {
                    127
                } else {
                    126
                },
                out: String::new(),
                err: e.to_string(),
            };
        }
    };
    let stdout = p.stdout.take();
    let stderr = p.stderr.take();
    let o = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stdout {
            let _ = s.take(16 * 1024 * 1024).read_to_end(&mut v);
        }
        v
    });
    let e = std::thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stderr {
            let _ = s.take(4 * 1024 * 1024).read_to_end(&mut v);
        }
        v
    });
    let start = Instant::now();
    let rc = loop {
        match p.try_wait() {
            Ok(Some(s)) => break s.code().unwrap_or(126),
            Ok(None) if start.elapsed() < Duration::from_secs_f64(timeout) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                unsafe {
                    libc::kill(-(p.id() as i32), libc::SIGKILL);
                }
                let _ = p.wait();
                break 124;
            }
        }
    };
    Run {
        ok: rc == 0,
        rc,
        out: String::from_utf8_lossy(&o.join().unwrap_or_default()).into_owned(),
        err: String::from_utf8_lossy(&e.join().unwrap_or_default()).into_owned(),
    }
}
pub(crate) fn data() -> &'static Value {
    static DATA: OnceLock<Value> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("data.json")).expect("originalstorageconstants")
    })
}
pub(crate) fn tr(lang: &str, key: &str, args: &[(&str, String)]) -> String {
    let mut s = data()["_STR"][lang][key]
        .as_str()
        .or_else(|| data()["_STR"]["nl"][key].as_str())
        .unwrap_or(key)
        .to_owned();
    for (k, v) in args {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}
pub(crate) fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(glib::home_dir)
}
pub(crate) fn expand_user(s: &str) -> PathBuf {
    if s == "~" {
        return home();
    }
    if let Some(rel) = s.strip_prefix("~/") {
        return home().join(rel);
    }
    if let Some(rest) = s.strip_prefix('~') {
        let (name, suffix) = rest.split_once('/').unwrap_or((rest, ""));
        if let Ok(name) = std::ffi::CString::new(name) {
            let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
            let mut result = std::ptr::null_mut();
            let mut buf = vec![0u8; 16384];
            let code = unsafe {
                libc::getpwnam_r(
                    name.as_ptr(),
                    pwd.as_mut_ptr(),
                    buf.as_mut_ptr() as *mut _,
                    buf.len(),
                    &mut result,
                )
            };
            if code == 0 && !result.is_null() {
                let entry = unsafe { pwd.assume_init() };
                if !entry.pw_dir.is_null() {
                    let dir = unsafe { std::ffi::CStr::from_ptr(entry.pw_dir) }
                        .to_string_lossy()
                        .into_owned();
                    return PathBuf::from(dir).join(suffix);
                }
            }
        }
    }
    PathBuf::from(s)
}
pub(crate) fn user_dir(kind: &str) -> PathBuf {
    let cfg = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".config"))
        .join("user-dirs.dirs");
    let key = format!("XDG_{kind}_DIR=");
    for line in read(cfg).lines() {
        if let Some(v) = line.trim().strip_prefix(&key) {
            let v = v
                .trim()
                .trim_matches('"')
                .replace("$HOME", &home().to_string_lossy());
            let p = expand_user(&v);
            if p.is_absolute() && p != home() {
                return p;
            }
        }
    }
    home().join(match kind {
        "DOCUMENTS" => "Documents",
        "PICTURES" => "Pictures",
        "VIDEOS" => "Videos",
        "MUSIC" => "Music",
        "DOWNLOAD" => "Downloads",
        _ => "Desktop",
    })
}

pub(crate) fn atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".{}.{}.new",
        path.file_name().unwrap_or_default().to_string_lossy(),
        crate::events::nonce()
    ));
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
pub(crate) fn normalized(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(c),
        }
    }
    out
}
pub(crate) fn canonical(path: &Path) -> PathBuf {
    if let Ok(p) = fs::canonicalize(path) {
        return p;
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| home())
            .join(path)
    };
    if let Some(parent) = absolute.parent() {
        if let Ok(base) = fs::canonicalize(parent) {
            return normalized(&base.join(absolute.file_name().unwrap_or_default()));
        }
    }
    normalized(&absolute)
}
pub(crate) fn check_dir(path: &Value, lang: Lang) -> Result<PathBuf, ApiError> {
    let path = path
        .as_str()
        .filter(|s| !s.is_empty() && !s.contains('\0'))
        .ok_or_else(|| {
            ApiError::tr("bad_args", lang, "storage.noFolder", crate::i18n::NO_PARAMS)
        })?;
    let expanded = expand_user(path);
    let p = canonical(&expanded);
    if !p.is_absolute() {
        return Err(ApiError::tr(
            "bad_args",
            lang,
            "storage.notAbsolute",
            crate::i18n::NO_PARAMS,
        ));
    }
    if p != Path::new("/")
        && !p.to_string_lossy().starts_with("/run/media/")
        && ["/proc", "/sys", "/dev", "/run"]
            .iter()
            .any(|r| p == Path::new(r) || p.starts_with(r))
    {
        return Err(ApiError::tr(
            "unsupported",
            lang,
            "storage.notScannable",
            crate::i18n::NO_PARAMS,
        ));
    }
    if !p.is_dir() {
        return Err(ApiError::tr(
            "not_found",
            lang,
            "storage.noSuchFolder",
            crate::i18n::NO_PARAMS,
        ));
    }
    Ok(p)
}
pub(crate) fn scan_priority() {
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as u32;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        libc::syscall(libc::SYS_ioprio_set, 1, tid, 3 << 13);
    }
}
pub(crate) fn du(path: &Path, deadline: Option<f64>, same_fs: bool) -> (u64, u64) {
    let Ok(root) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if !root.is_dir() {
        return (root.blocks() * 512, 1);
    }
    let dev = root.dev();
    let mut total = root.blocks() * 512;
    let mut files = 0;
    let mut stack = vec![path.to_owned()];
    let mut seen = std::collections::HashSet::new();
    let mut n = 0;
    while let Some(p) = stack.pop() {
        let Ok(entries) = fs::read_dir(p) else {
            continue;
        };
        for e in entries.flatten() {
            let Ok(s) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            if s.is_dir() {
                if same_fs && s.dev() != dev {
                    continue;
                }
                total += s.blocks() * 512;
                stack.push(e.path());
            } else {
                if s.nlink() > 1 && !seen.insert((s.dev(), s.ino())) {
                    continue;
                }
                total += s.blocks() * 512;
                files += 1;
            }
            n += 1;
            if n % 4096 == 0 && deadline.is_some_and(|d| now() > d) {
                return (total, files);
            }
        }
    }
    (total, files)
}
pub(crate) fn statvfs(path: &Path) -> Option<libc::statvfs> {
    use std::os::unix::ffi::OsStrExt;
    let p = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut s = std::mem::MaybeUninit::uninit();
    if unsafe { libc::statvfs(p.as_ptr(), s.as_mut_ptr()) } == 0 {
        Some(unsafe { s.assume_init() })
    } else {
        None
    }
}
pub(crate) fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
pub(crate) fn ps(v: &Value) -> String {
    match v {
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::String(s) => s.clone(),
        _ => v.to_string(),
    }
}

#[cfg(test)]
mod rounding_tests {
    use super::*;
    #[test]
    fn decimal_rounding_matches_reference_binary_float_cases() {
        assert_eq!(round(2.675, 2), 2.67);
        assert_eq!(round(12.345, 2), 12.35);
        assert_eq!(round(2.5, 0), 2.);
        assert_eq!(round(3.5, 0), 4.);
    }
}
