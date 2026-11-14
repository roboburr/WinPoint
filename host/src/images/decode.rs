use gdk_pixbuf::{InterpType, Pixbuf};
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Condvar, Mutex, OnceLock},
    time::{Duration, Instant},
};
struct Gate {
    active: Mutex<usize>,
    ready: Condvar,
}
struct Permit(&'static Gate);
impl Drop for Permit {
    fn drop(&mut self) {
        let mut n = self.0.active.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        self.0.ready.notify_one();
    }
}
pub(super) fn permit() -> impl Drop {
    static G: OnceLock<Gate> = OnceLock::new();
    let g = G.get_or_init(|| Gate {
        active: Mutex::new(0),
        ready: Condvar::new(),
    });
    let mut n = g.active.lock().unwrap_or_else(|e| e.into_inner());
    while *n >= 2 {
        n = g.ready.wait(n).unwrap_or_else(|e| e.into_inner())
    }
    *n += 1;
    Permit(g)
}
fn which(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|p| p.join(name))
        .find(|p| {
            p.metadata().is_ok_and(|m| {
                m.is_file()
                    && std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0
            })
        })
}
pub(super) fn bounded(mut cmd: Command, timeout: Duration, max: usize) -> Option<Vec<u8>> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        stdout.take(max as u64 + 1).read_to_end(&mut data).ok()?;
        (data.len() <= max).then_some(data)
    });
    let begin = Instant::now();
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if begin.elapsed() < timeout => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let data = reader.join().ok().flatten();
    if success { data } else { None }
}
fn bwrap(path: &Path, worker: &Path) -> Vec<String> {
    if std::env::var("WIN11SHELL_NO_BWRAP").ok().as_deref() == Some("1") {
        return vec![];
    }
    let Some(exe) = which("bwrap") else {
        return vec![];
    };
    let mut args = vec![
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
    if path.starts_with("/tmp/") {
        args.extend([
            "--ro-bind".into(),
            path.to_string_lossy().into_owned(),
            path.to_string_lossy().into_owned(),
        ])
    }
    // Development/test builds may keep the native helper in /tmp, hidden by the private tmpfs.
    if worker.starts_with("/tmp/") && worker != path {
        args.extend([
            "--ro-bind".into(),
            worker.to_string_lossy().into_owned(),
            worker.to_string_lossy().into_owned(),
        ]);
    }
    args.extend([
        "--unshare-all".into(),
        "--die-with-parent".into(),
        "--new-session".into(),
        "--".into(),
    ]);
    static OK: OnceLock<bool> = OnceLock::new();
    let ok = *OK.get_or_init(|| {
        let mut c = Command::new(&exe);
        c.args(&args).arg("/bin/true");
        bounded(c, Duration::from_secs(5), 1024).is_some()
    });
    if ok {
        let mut out = vec![exe.to_string_lossy().into_owned()];
        out.extend(args);
        out
    } else {
        vec![]
    }
}
fn worker() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("WIN11SHELL_IMAGE_WORKER") {
        return Some(p.into());
    }
    let current = std::env::current_exe().ok()?;
    let p = current.parent()?.join("win11-image-worker");
    p.is_file().then_some(p)
}
pub(super) fn isolated(path: &Path, size: i32, jpeg: bool) -> Option<(Vec<u8>, &'static str)> {
    isolated_inner(path, size, jpeg, false)
}
pub(super) fn isolated_video(path: &Path, size: i32) -> Option<(Vec<u8>, &'static str)> {
    isolated_inner(path, size, false, true)
}
fn isolated_inner(
    path: &Path,
    size: i32,
    jpeg: bool,
    video: bool,
) -> Option<(Vec<u8>, &'static str)> {
    let _permit = permit();
    let path = path.canonicalize().ok()?;
    let worker = worker()?;
    let prefix = bwrap(&path, &worker);
    let mut cmd = if let Some(exe) = prefix.first() {
        let mut c = Command::new(exe);
        c.args(&prefix[1..]).arg(worker);
        c
    } else {
        Command::new(worker)
    };
    if video {
        cmd.arg("--video").arg(path).arg(size.to_string());
    } else {
        cmd.arg(path)
            .arg(size.to_string())
            .arg(if jpeg { "jpeg" } else { "png" });
    }
    let out = bounded(cmd, Duration::from_secs(20), 128 << 20)?;
    if out.len() < 9 || ![b'J', b'P'].contains(&out[0]) {
        return None;
    }
    Some((
        out[1..].to_vec(),
        if out[0] == b'J' {
            "image/jpeg"
        } else {
            "image/png"
        },
    ))
}
pub(super) fn encode(
    path: &Path,
    size: i32,
    jpeg: bool,
    orientation: bool,
) -> Option<(Vec<u8>, &'static str)> {
    let pb = if size > 0 {
        Pixbuf::from_file_at_scale(path, size, size, true).ok()?
    } else {
        let pb = Pixbuf::from_file(path).ok()?;
        if pb.width() > 4096 || pb.height() > 4096 {
            if orientation {
                let (w, h) = fit(pb.width(), pb.height(), 4096);
                pb.scale_simple(w, h, InterpType::Bilinear)?
            } else {
                Pixbuf::from_file_at_scale(path, 4096, 4096, true).ok()?
            }
        } else {
            pb
        }
    };
    let pb = if orientation {
        pb.apply_embedded_orientation().unwrap_or(pb)
    } else {
        pb
    };
    let (fmt, mime, options) = if jpeg && !pb.has_alpha() {
        ("jpeg", "image/jpeg", vec![("quality", "85")])
    } else {
        ("png", "image/png", vec![])
    };
    Some((pb.save_to_bufferv(fmt, &options).ok()?, mime))
}
fn fit(w: i32, h: i32, side: i32) -> (i32, i32) {
    if w >= h {
        (
            side,
            (h as f64 * side as f64 / w as f64)
                .round_ties_even()
                .max(1.) as i32,
        )
    } else {
        (
            (w as f64 * side as f64 / h as f64)
                .round_ties_even()
                .max(1.) as i32,
            side,
        )
    }
}
pub(super) fn limits() {
    unsafe {
        for (lim, val) in [
            (libc::RLIMIT_AS, 2u64 << 30),
            (libc::RLIMIT_CPU, 15),
            (libc::RLIMIT_FSIZE, 128 << 20),
            (libc::RLIMIT_CORE, 0),
        ] {
            let mut old = std::mem::zeroed::<libc::rlimit>();
            if libc::getrlimit(lim, &mut old) == 0 {
                let new = (val as libc::rlim_t).min(old.rlim_max);
                let _ = libc::setrlimit(
                    lim,
                    &libc::rlimit {
                        rlim_cur: new,
                        rlim_max: if old.rlim_max == libc::RLIM_INFINITY {
                            new
                        } else {
                            old.rlim_max
                        },
                    },
                );
            }
        }
        libc::nice(10);
        libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
    }
}
pub(crate) fn worker_main() -> i32 {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 3 {
        return 2;
    }
    if args[0] == "--video" {
        let Some(size) = args[2].to_str().and_then(|s| s.parse::<i32>().ok()) else {
            return 2;
        };
        limits();
        let Some(bytes) = super::video::encode_worker(Path::new(&args[1]), size) else {
            return 1;
        };
        let mut out = std::io::stdout().lock();
        return if out
            .write_all(b"P")
            .and_then(|_| out.write_all(&bytes))
            .and_then(|_| out.flush())
            .is_ok()
        {
            0
        } else {
            1
        };
    }
    let Some(size) = args[1].to_str().and_then(|s| s.parse::<i64>().ok()) else {
        return 2;
    };
    limits();
    let Some((bytes, mime)) = encode(
        Path::new(&args[0]),
        size.clamp(0, 4096) as i32,
        args[2] == "jpeg",
        true,
    ) else {
        return 1;
    };
    let mut out = std::io::stdout().lock();
    if out
        .write_all(if mime == "image/jpeg" { b"J" } else { b"P" })
        .and_then(|_| out.write_all(&bytes))
        .and_then(|_| out.flush())
        .is_ok()
    {
        0
    } else {
        1
    }
}
