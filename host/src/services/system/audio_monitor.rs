//! system.audio's change monitor: the Sound service's PulseAudio client while a pulse server answers it
//! (services/sound.rs, amendment 39: its subscription is the one source of changes), else one read-only PipeWire
//! monitor (pw-dump) with the same graph filtering and polling fallback. libpulse loads on any desktop with GTK apps,
//! also where no pulse server runs (PipeWire and WirePlumber without pipewire-pulse): the client's listener is
//! registered whenever libpulse loads. pw-dump starts if the initial connection fails or Pulse disconnects later,
//! and is stopped and reaped when Pulse recovers, leaving one active source of graph notifications.
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
/// pw-dump's read-only monitor of the PipeWire graph.
const PW_DUMP: [&str; 3] = ["pw-dump", "--monitor", "--no-colors"];

/// The Sound service's PulseAudio client as the monitor uses it (a test stands in for libpulse).
pub(super) trait Pulse: Send + Sync {
    /// Listen to the client's changes; None without libpulse.
    fn listen(&self, changed: Arc<dyn Fn() + Send + Sync>) -> Option<u64>;
    /// Wait (at most 1.5 s) for the first connection; whether a pulse server answered.
    fn connected(&self) -> bool;
    /// Whether the client is connected now.
    fn ready(&self) -> bool;
    fn unlisten(&self, id: u64);
}

/// The process's own client (services/sound.rs).
struct SoundClient;

impl Pulse for SoundClient {
    fn listen(&self, changed: Arc<dyn Fn() + Send + Sync>) -> Option<u64> {
        crate::services::sound::audio_listen(changed)
    }
    fn connected(&self) -> bool {
        crate::services::sound::audio_connected()
    }
    fn ready(&self) -> bool {
        crate::services::sound::audio_ready()
    }
    fn unlisten(&self, id: u64) {
        crate::services::sound::audio_unlisten(id)
    }
}

pub(super) struct AudioMonitor {
    stopped: AtomicBool,
    /// pw-dump runs and has sent its first dump
    alive: AtomicBool,
    child: Mutex<Option<Child>>,
    /// the Sound client's listener (whenever libpulse loads)
    pulse: Option<u64>,
    client: Arc<dyn Pulse>,
}
impl AudioMonitor {
    pub fn start(changed: Arc<dyn Fn() + Send + Sync>) -> Option<Arc<Self>> {
        let pw_dump = (available("pw-dump") && available("wpctl"))
            .then(|| PW_DUMP.iter().map(|s| (*s).to_owned()).collect());
        Self::start_with(Arc::new(SoundClient), pw_dump, changed)
    }

    /// The monitor on `client`, with `pw_dump` (its argv; None: pw-dump or wpctl is missing) when the client does not
    /// connect. start() never waits: the first connection is awaited on the monitor's own thread.
    fn start_with(
        client: Arc<dyn Pulse>,
        pw_dump: Option<Vec<String>>,
        changed: Arc<dyn Fn() + Send + Sync>,
    ) -> Option<Arc<Self>> {
        let pulse = client.listen(changed.clone());
        if pulse.is_none() && pw_dump.is_none() {
            return None;
        }
        let monitor = Arc::new(Self {
            stopped: AtomicBool::new(false),
            alive: AtomicBool::new(false),
            child: Mutex::new(None),
            pulse,
            client,
        });
        let Some(argv) = pw_dump else {
            return Some(monitor);
        };
        let weak = Arc::downgrade(&monitor);
        thread::spawn(move || {
            // Await the first connection once; keep supervising after it succeeds, so a later disconnect can
            // start the fallback. The listener remains registered for Pulse's own reconnection.
            if let Some(m) = weak.upgrade() {
                if m.pulse.is_some() {
                    m.client.connected();
                }
            } else {
                return;
            }
            let mut failures = 0u32;
            loop {
                let Some(monitor) = weak.upgrade() else { break };
                if monitor.stopped.load(Ordering::Relaxed) {
                    break;
                }
                if monitor.pulse_ready() {
                    // No fallback child or graph notifications alongside the connected Pulse client.
                    failures = 0;
                    drop(monitor);
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                let started = Instant::now();
                let parent = unsafe { libc::getpid() };
                let mut command = Command::new(&argv[0]);
                command
                    .args(&argv[1..])
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
                    monitor.retry_delay(Duration::from_secs(5));
                    continue;
                };
                let Some(mut stdout) = child.stdout.take() else {
                    let _ = child.kill();
                    let _ = child.wait();
                    monitor.retry_delay(Duration::from_secs(5));
                    continue;
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
                while !monitor.stopped.load(Ordering::Relaxed) && !monitor.pulse_ready() {
                    match stdout.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(n) => {
                            buffer.extend_from_slice(&bytes[..n]);
                            let (consumed, initial, changes) = feed(&buffer, &mut first);
                            if initial {
                                monitor.alive.store(true, Ordering::Relaxed);
                            }
                            for _ in 0..changes {
                                if !monitor.pulse_ready() {
                                    changed();
                                }
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
                if monitor.pulse_ready() {
                    failures = 0;
                    continue;
                }
                failures = if started.elapsed() > Duration::from_secs(60) {
                    0
                } else {
                    (failures + 1).min(5)
                };
                monitor.retry_delay(Duration::from_secs((1u64 << failures).min(30)));
                drop(monitor);
            }
        });
        Some(monitor)
    }
    fn pulse_ready(&self) -> bool {
        self.pulse.is_some() && self.client.ready()
    }

    /// Backoff is interrupted by shutdown or a recovered Pulse client.
    fn retry_delay(&self, delay: Duration) {
        let until = Instant::now() + delay;
        while !self.stopped.load(Ordering::Relaxed) && !self.pulse_ready() && Instant::now() < until
        {
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Whether changes arrive as they happen (system.rs polls less often then): the connected client, or pw-dump.
    pub fn alive(&self) -> bool {
        !self.stopped.load(Ordering::Relaxed)
            && (self.pulse_ready() || self.alive.load(Ordering::Relaxed))
    }
    pub fn stop(&self) {
        if let Some(id) = self.pulse {
            self.client.unlisten(id);
        }
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
    use std::sync::atomic::AtomicUsize;

    /// libpulse that loads; `connected`: whether a pulse server answers.
    struct FakePulse {
        connected: AtomicBool,
        listening: Mutex<Vec<u64>>,
    }
    impl FakePulse {
        fn new(connected: bool) -> Arc<Self> {
            Arc::new(Self {
                connected: AtomicBool::new(connected),
                listening: Mutex::new(Vec::new()),
            })
        }
    }
    impl Pulse for FakePulse {
        fn listen(&self, _changed: Arc<dyn Fn() + Send + Sync>) -> Option<u64> {
            self.listening.lock().unwrap().push(7);
            Some(7)
        }
        fn connected(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }
        fn ready(&self) -> bool {
            self.connected.load(Ordering::Relaxed)
        }
        fn unlisten(&self, id: u64) {
            self.listening.lock().unwrap().retain(|n| *n != id);
        }
    }

    /// A pw-dump stand-in: the first dump, one node change, then quiet.
    fn pw_dump() -> Vec<String> {
        let script =
            r#"printf '[]\n[{"type":"PipeWire:Interface:Node","info":{}}]\n'; exec sleep 30"#;
        ["sh", "-c", script].map(str::to_owned).to_vec()
    }

    fn wait(what: impl Fn() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            if what() {
                return true;
            }
            thread::sleep(Duration::from_millis(20));
        }
        what()
    }

    #[test]
    fn without_a_pulse_server_pw_dump_watches_the_graph() {
        // review D-2: libpulse loads (any GTK desktop) but PipeWire runs without pipewire-pulse
        let client = FakePulse::new(false);
        let changes = Arc::new(AtomicUsize::new(0));
        let c = changes.clone();
        let monitor = AudioMonitor::start_with(
            client.clone(),
            Some(pw_dump()),
            Arc::new(move || {
                c.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .expect("a monitor");
        assert!(
            wait(|| monitor.alive()),
            "pw-dump's first dump: changes arrive at once"
        );
        assert!(
            wait(|| changes.load(Ordering::Relaxed) == 1),
            "the node change"
        );
        assert_eq!(
            *client.listening.lock().unwrap(),
            [7],
            "the client's listener stays for a later connection"
        );
        monitor.stop();
        assert!(client.listening.lock().unwrap().is_empty());
        assert!(wait(|| !monitor.alive()), "pw-dump stopped");
    }

    #[test]
    fn a_connected_pulse_client_is_the_only_source() {
        let client = FakePulse::new(true);
        let changes = Arc::new(AtomicUsize::new(0));
        let c = changes.clone();
        let monitor = AudioMonitor::start_with(
            client.clone(),
            Some(pw_dump()),
            Arc::new(move || {
                c.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .expect("a monitor");
        assert!(monitor.alive(), "the connected client");
        thread::sleep(Duration::from_millis(400));
        assert!(
            monitor.child.lock().unwrap().is_none(),
            "no pw-dump next to it (amendment 39)"
        );
        assert_eq!(changes.load(Ordering::Relaxed), 0);
        monitor.stop();
        // neither libpulse nor pw-dump: no monitor (system.rs polls)
        struct NoPulse;
        impl Pulse for NoPulse {
            fn listen(&self, _: Arc<dyn Fn() + Send + Sync>) -> Option<u64> {
                None
            }
            fn connected(&self) -> bool {
                false
            }
            fn ready(&self) -> bool {
                false
            }
            fn unlisten(&self, _: u64) {}
        }
        assert!(AudioMonitor::start_with(Arc::new(NoPulse), None, Arc::new(|| {})).is_none());
    }
    #[test]
    fn a_lost_pulse_server_starts_one_fallback_and_recovery_stops_it() {
        let client = FakePulse::new(true);
        let changes = Arc::new(AtomicUsize::new(0));
        let sink = changes.clone();
        let monitor = AudioMonitor::start_with(
            client.clone(),
            Some(pw_dump()),
            Arc::new(move || {
                sink.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
        assert!(monitor.alive());
        assert!(monitor.child.lock().unwrap().is_none());
        for cycle in 1..=2 {
            client.connected.store(false, Ordering::Relaxed);
            assert!(
                wait(|| monitor.alive.load(Ordering::Relaxed)),
                "fallback after disconnect {cycle}"
            );
            assert!(wait(|| changes.load(Ordering::Relaxed) == cycle));
            assert!(monitor.child.lock().unwrap().is_some());
            client.connected.store(true, Ordering::Relaxed);
            assert!(
                wait(|| monitor.child.lock().unwrap().is_none()),
                "fallback reaped after recovery"
            );
            assert!(!monitor.alive.load(Ordering::Relaxed));
            assert!(monitor.alive(), "Pulse is the active source again");
        }
        assert_eq!(
            *client.listening.lock().unwrap(),
            [7],
            "one retained Pulse subscription"
        );
        monitor.stop();
        assert!(!monitor.alive());
        assert!(client.listening.lock().unwrap().is_empty());
    }

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
