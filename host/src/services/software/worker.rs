//! Downloaded archive metadata is parsed outside the desktop/HTTP process.
use super::{Tools, common};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
    time::Duration,
};
pub(super) fn command(worker: &Path, source: &Path) -> Command {
    let bwrap = Path::new("/usr/bin/bwrap");
    if !bwrap.is_file() || std::env::var("WIN11SHELL_NO_BWRAP").ok().as_deref() == Some("1") {
        return Command::new(worker);
    }
    let mut prefix = vec![
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
        "--tmpfs".into(),
        "/tmp".into(),
    ];
    for path in [source, worker] {
        if path.starts_with("/tmp/") {
            prefix.extend([
                "--ro-bind".into(),
                path.to_string_lossy().into_owned(),
                path.to_string_lossy().into_owned(),
            ]);
        }
    }
    prefix.extend([
        "--unshare-all".into(),
        "--die-with-parent".into(),
        "--new-session".into(),
        "--".into(),
    ]);
    static OK: OnceLock<bool> = OnceLock::new();
    let ok = *OK.get_or_init(|| {
        let mut p = Command::new(bwrap);
        p.args(&prefix).arg("/bin/true");
        common::bounded(p, Duration::from_secs(4), 1024).is_ok()
    });
    if !ok {
        return Command::new(worker);
    }
    let mut cmd = Command::new(bwrap);
    cmd.args(prefix).arg(worker);
    cmd
}
fn limits() {
    unsafe {
        for (lim, val) in [
            (libc::RLIMIT_AS, 256u64 << 20),
            (libc::RLIMIT_CPU, 10),
            (libc::RLIMIT_FSIZE, 32 << 20),
            (libc::RLIMIT_CORE, 0),
        ] {
            let mut old = std::mem::zeroed::<libc::rlimit>();
            if libc::getrlimit(lim, &mut old) == 0 {
                let val = (val as libc::rlim_t).min(old.rlim_max);
                libc::setrlimit(
                    lim,
                    &libc::rlimit {
                        rlim_cur: val,
                        rlim_max: val,
                    },
                );
            }
        }
        libc::nice(10);
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
    }
}
pub(super) fn main() -> i32 {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 3 || args[0] != "--inspect" {
        return 2;
    }
    let Some(kind) = args[1].to_str().filter(|s| ["deb", "snap"].contains(s)) else {
        return 2;
    };
    let path = PathBuf::from(&args[2]);
    if !path.is_absolute() || !path.is_file() {
        return 2;
    }
    limits();
    let tools = Tools::system();
    let value = match super::inspect::worker_inspect(kind, &path, &tools) {
        Ok(v) => serde_json::json!({"ok":true,"details":v}),
        Err(_) => serde_json::json!({"ok":false}),
    };
    let bytes = serde_json::to_vec(&value).unwrap_or_default();
    if bytes.len() > 1 << 20 {
        return 1;
    }
    if std::io::stdout().lock().write_all(&bytes).is_ok() {
        0
    } else {
        1
    }
}

static METADATA_SLOTS: (std::sync::Mutex<u8>, std::sync::Condvar) =
    (std::sync::Mutex::new(0), std::sync::Condvar::new());
pub(super) struct MetadataSlot;
pub(super) fn metadata_slot() -> MetadataSlot {
    let mut count = METADATA_SLOTS.0.lock().unwrap_or_else(|e| e.into_inner());
    while *count >= 2 {
        count = METADATA_SLOTS
            .1
            .wait(count)
            .unwrap_or_else(|e| e.into_inner());
    }
    *count += 1;
    MetadataSlot
}
impl Drop for MetadataSlot {
    fn drop(&mut self) {
        let mut count = METADATA_SLOTS.0.lock().unwrap_or_else(|e| e.into_inner());
        *count = count.saturating_sub(1);
        METADATA_SLOTS.1.notify_one();
    }
}
