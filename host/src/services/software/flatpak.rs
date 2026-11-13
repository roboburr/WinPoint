//! Flatpak receives fixed argv, an exact app reference and an explicit installation scope.
use super::{Tools, common::*, resolve::valid_ref};
use serde_json::Value;
use std::{
    io::Read,
    os::fd::AsRawFd,
    os::unix::process::ExitStatusExt,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub(super) enum Outcome {
    Done,
    Cancelled,
}
pub(super) fn run(
    tools: &Tools,
    action: &str,
    info: &Value,
    stage: Option<&Path>,
    cancel: &dyn Fn() -> bool,
    update: &dyn Fn(),
) -> Result<Outcome, String> {
    let mut cmd = Command::new(&tools.flatpak);
    if action == "install" {
        cmd.args(["install", "--user", "--noninteractive", "--assumeyes"]);
        match info["format"].as_str() {
            Some("flatpakref") => {
                cmd.arg("--from");
            }
            Some("flatpak") => {
                cmd.arg("--bundle");
            }
            _ => return Err("Invalid Flatpak package type".into()),
        };
        cmd.arg(stage.ok_or("Missing staged package")?);
    } else {
        let reference = info["_ref"]
            .as_str()
            .filter(|s| valid_ref(s))
            .ok_or("Invalid installed Flatpak reference")?;
        let scope = info["scope"]
            .as_str()
            .filter(|s| ["user", "system"].contains(s))
            .ok_or("Invalid Flatpak scope")?;
        cmd.arg("uninstall").arg(format!("--{scope}")).args([
            "--noninteractive",
            "--assumeyes",
            "--no-related",
            "--app",
            reference,
        ]);
    }
    cmd.env("LC_ALL", "C.UTF-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let finished = Arc::new(AtomicBool::new(false));
    let mut readers = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(reader(out, finished.clone()));
    }
    if let Some(out) = child.stderr.take() {
        readers.push(reader(out, finished.clone()));
    }
    let start = Instant::now();
    let mut requested = None;
    let mut killed = false;
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if (cancel() || start.elapsed() > Duration::from_secs(7200)) && requested.is_none() {
            unsafe {
                libc::kill(child.id() as i32, libc::SIGINT);
            };
            requested = Some(Instant::now());
        }
        if requested.is_some_and(|t| t.elapsed() > Duration::from_secs(30)) && !killed {
            let _ = child.kill();
            killed = true;
        }
        update();
        std::thread::sleep(Duration::from_millis(200));
    };
    finished.store(true, Ordering::Release);
    let mut output = Vec::new();
    for r in readers {
        if let Ok(b) = r.join() {
            output.extend(b);
        }
    }
    let detail = clean(&String::from_utf8_lossy(&output), 4000);
    if status.success() {
        return Ok(Outcome::Done);
    }
    if killed || start.elapsed() > Duration::from_secs(7200) {
        return Err("outcome_unknown: Flatpak did not finish normally; check the installed application before retrying".into());
    }
    if requested.is_some()
        && (status.signal() == Some(libc::SIGINT)
            || status.code() == Some(130)
            || detail.to_ascii_lowercase().contains("cancelled"))
    {
        return Ok(Outcome::Cancelled);
    }
    Err(if detail.is_empty() {
        format!("Flatpak exited with {status}")
    } else {
        detail
    })
}

fn reader(
    mut out: impl Read + AsRawFd + Send + 'static,
    finished: Arc<AtomicBool>,
) -> std::thread::JoinHandle<Vec<u8>> {
    let fd = out.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }
    }
    std::thread::spawn(move || {
        let mut tail = Vec::new();
        let mut b = [0u8; 4096];
        let mut drain = 0usize;
        loop {
            if finished.load(Ordering::Acquire) {
                drain += 1;
                if drain > 16 {
                    break;
                }
            }
            match out.read(&mut b) {
                Ok(0) => break,
                Ok(n) => {
                    tail.extend_from_slice(&b[..n]);
                    if tail.len() > 65536 {
                        tail.drain(..tail.len() - 65536);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if finished.load(Ordering::Acquire) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        tail
    })
}
