//! One read-only PipeWire monitor, with the same graph filtering and polling fallback.
use super::common::available;
use serde_json::Value;
use std::{
    io::Read,
    os::unix::{io::AsRawFd, process::CommandExt},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
pub(super) struct AudioMonitor {
    stopped: AtomicBool,
    alive: AtomicBool,
    child: Mutex<Option<Child>>,
}
impl AudioMonitor {
    pub fn start(changed: Arc<dyn Fn() + Send + Sync>) -> Option<Arc<Self>> {
        if !available("pw-dump") || !available("wpctl") {
            return None;
        }
        let monitor = Arc::new(Self {
            stopped: AtomicBool::new(false),
            alive: AtomicBool::new(false),
            child: Mutex::new(None),
        });
        let weak = Arc::downgrade(&monitor);
        thread::spawn(move || {
            let mut failures = 0;
            loop {
                let Some(monitor) = weak.upgrade() else { break };
                if monitor.stopped.load(Ordering::Relaxed) {
                    break;
                }
                let started = Instant::now();
                let parent = unsafe { libc::getpid() };
                let mut command = Command::new("pw-dump");
                command
                    .args(["--monitor", "--no-colors"])
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null());
                unsafe {
                    command.pre_exec(move || {
                        if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                            return Err(std::io::Error::last_os_error());
                        }
                        if libc::getppid() != parent {
                            return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                        }
                        Ok(())
                    });
                }
                let Ok(mut child) = command.spawn() else {
                    break;
                };
                let Some(mut stdout) = child.stdout.take() else {
                    let _ = child.kill();
                    let _ = child.wait();
                    break;
                };
                let fd = stdout.as_raw_fd();
                unsafe {
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    if flags >= 0 {
                        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                    }
                }
                *monitor.child.lock().unwrap_or_else(|e| e.into_inner()) = Some(child);
                let mut buffer = Vec::new();
                let mut first = true;
                let mut bytes = [0u8; 65536];
                while !monitor.stopped.load(Ordering::Relaxed) {
                    match stdout.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            buffer.extend_from_slice(&bytes[..n]);
                            let (consumed, initial, changes) = feed(&buffer, &mut first);
                            if initial {
                                monitor.alive.store(true, Ordering::Relaxed);
                            }
                            for _ in 0..changes {
                                changed();
                            }
                            if consumed > 0 {
                                buffer.drain(..consumed);
                            }
                            if buffer.len() > 8 * 1024 * 1024 {
                                break;
                            }
                        }
                        Err(e)
                            if matches!(
                                e.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                            ) =>
                        {
                            thread::sleep(Duration::from_millis(20))
                        }
                        Err(_) => break,
                    }
                }
                monitor.alive.store(false, Ordering::Relaxed);
                if let Some(mut child) = monitor
                    .child
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take()
                {
                    if child.try_wait().ok().flatten().is_none() {
                        let _ = child.kill();
                    }
                    let _ = child.wait();
                }
                if monitor.stopped.load(Ordering::Relaxed) {
                    break;
                }
                failures = if started.elapsed() > Duration::from_secs(60) {
                    0
                } else {
                    failures + 1
                };
                if failures > 5 {
                    break;
                }
                let until = Instant::now() + Duration::from_secs((1u64 << failures).min(30));
                while !monitor.stopped.load(Ordering::Relaxed) && Instant::now() < until {
                    thread::sleep(Duration::from_millis(50));
                }
                drop(monitor);
            }
        });
        Some(monitor)
    }
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let mut guard = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(child) = guard.as_mut() {
            if child.try_wait().ok().flatten().is_none() {
                unsafe {
                    libc::kill(child.id() as i32, libc::SIGTERM);
                }
            }
        }
    }
}
fn relevant(chunk: &Value) -> bool {
    chunk.as_array().is_some_and(|a| {
        a.iter().any(|o| {
            o.is_object()
                && (matches!(
                    o["type"].as_str(),
                    Some(
                        "PipeWire:Interface:Node"
                            | "PipeWire:Interface:Device"
                            | "PipeWire:Interface:Metadata"
                    )
                ) || o.get("info").is_some_and(Value::is_null))
        })
    })
}
fn feed(buffer: &[u8], first: &mut bool) -> (usize, bool, usize) {
    let mut offset = 0;
    let mut initial = false;
    let mut changes = 0;
    loop {
        while offset < buffer.len() && b" \t\r\n,".contains(&buffer[offset]) {
            offset += 1;
        }
        if offset == buffer.len() {
            return (offset, initial, changes);
        }
        let mut stream =
            serde_json::Deserializer::from_slice(&buffer[offset..]).into_iter::<Value>();
        match stream.next() {
            Some(Ok(value)) => {
                offset += stream.byte_offset();
                if *first {
                    *first = false;
                    initial = true
                } else if relevant(&value) {
                    changes += 1
                }
            }
            _ => return (offset, initial, changes),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipewire_stream_is_incremental_and_skips_initial_dump() {
        let mut first = true;
        let graph=b" [ {\"type\":\"PipeWire:Interface:Node\",\"info\":{}} ]\n, [{\"type\":\"PipeWire:Interface:Client\",\"info\":{}}] [{\"id\":9,\"info\":null}] [{\"type\":\"PipeWire:Interface:Metadata\"}] [ {\"type\":\"PipeWire:Interface:Node\"";
        let (n, initial, count) = feed(graph, &mut first);
        assert!(initial);
        assert_eq!(count, 2);
        assert!(!first);
        assert!(n < graph.len());
        let mut rest = graph[n..].to_vec();
        rest.extend_from_slice(b"} ]");
        let (n, initial, count) = feed(&rest, &mut first);
        assert_eq!(n, rest.len());
        assert!(!initial);
        assert_eq!(count, 1);
    }
    #[test]
    fn only_expected_object_domains_refresh_audio() {
        assert!(relevant(&serde_json::json!([{"info":null}])));
        assert!(!relevant(
            &serde_json::json!([{"type":"PipeWire:Interface:Client"}])
        ));
        assert!(!relevant(
            &serde_json::json!({"type":"PipeWire:Interface:Node"})
        ));
    }
}
