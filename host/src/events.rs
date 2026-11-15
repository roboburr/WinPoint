//! Native events shared by the independent Rust hosts and merged into their original SSE streams.
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    io::{self, Read},
    os::unix::{
        fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixDatagram,
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const LIMIT: usize = 60 * 1024;

pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "De runtime-map moet privé zijn",
        ));
    }
    Ok(())
}

pub(crate) fn nonce() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) struct Events {
    subscribers: Mutex<HashMap<u64, (SyncSender<Vec<u8>>, Arc<AtomicBool>)>>,
    sequence: AtomicU64,
    clipboard_sequence: AtomicU64,
    clipboard_loader: Mutex<Option<Arc<dyn Fn() -> Option<Value> + Send + Sync>>>,
    settings_loader: Mutex<Option<Arc<dyn Fn(&Value) -> Option<Value> + Send + Sync>>>,
    observers: Mutex<HashMap<String, Vec<Arc<dyn Fn(&Value) + Send + Sync>>>>,
    socket: UnixDatagram,
    path: PathBuf,
    root: PathBuf,
    stopped: AtomicBool,
    update_pages: AtomicBool,
}

impl Events {
    pub(crate) fn new(root: &Path) -> io::Result<Arc<Self>> {
        private_dir(root)?;
        static SOCKET_ID: AtomicU64 = AtomicU64::new(0);
        let path = root.join(format!(
            "p{:x}-{:x}.sock",
            std::process::id(),
            SOCKET_ID.fetch_add(1, Ordering::Relaxed)
        ));
        if path.exists() {
            fs::remove_file(&path)?;
        }
        let receiving = UnixDatagram::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        receiving.set_read_timeout(Some(Duration::from_millis(300)))?;
        let socket = UnixDatagram::unbound()?;
        socket.set_nonblocking(true)?;
        let events = Arc::new(Self {
            subscribers: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(1),
            clipboard_sequence: AtomicU64::new(0),
            clipboard_loader: Mutex::new(None),
            settings_loader: Mutex::new(None),
            observers: Mutex::new(HashMap::new()),
            socket,
            path,
            root: root.into(),
            stopped: AtomicBool::new(false),
            update_pages: AtomicBool::new(false),
        });
        let weak = Arc::downgrade(&events);
        std::thread::spawn(move || {
            let mut bytes = vec![0_u8; LIMIT + 1];
            loop {
                let Some(events) = weak.upgrade() else { break };
                if events.stopped.load(Ordering::Relaxed) {
                    break;
                }
                drop(events);
                match receiving.recv(&mut bytes) {
                    Ok(n) if n <= LIMIT => {
                        if let Ok(value) = serde_json::from_slice::<Value>(&bytes[..n])
                            && let Some(name) = value["event"].as_str()
                            && valid_name(name)
                            && let Some(events) = weak.upgrade()
                        {
                            if name == "settings.changed"
                                && value["data"]["_settingsInvalidation"] == true
                            {
                                let loader = events
                                    .settings_loader
                                    .lock()
                                    .ok()
                                    .and_then(|loader| loader.clone());
                                if let Some(data) = loader.and_then(|load| load(&value["data"])) {
                                    events.dispatch(name, &data);
                                }
                            } else if name == "files.clipboardChanged"
                                && value["data"]["_clipboardInvalidation"] == true
                            {
                                let loader = events
                                    .clipboard_loader
                                    .lock()
                                    .ok()
                                    .and_then(|loader| loader.clone());
                                if let Some(data) = loader.and_then(|load| load()) {
                                    events.dispatch(name, &data);
                                }
                            } else {
                                events.dispatch(name, &value["data"]);
                            }
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock
                                | io::ErrorKind::TimedOut
                                | io::ErrorKind::Interrupted
                        ) => {}
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        });
        Ok(events)
    }

    pub(crate) fn emit_local(&self, event: &str, data: Value) {
        if valid_name(event) {
            self.dispatch(event, &data);
        }
    }

    pub(crate) fn emit(&self, event: &str, data: Value) {
        if !valid_name(event) {
            return;
        }
        self.dispatch(event, &data);
        // File selections can be far larger than one Unix datagram. The receiver reads
        // the shared, validated snapshot; it never trusts a path sent in the invalidation.
        let wire_data = if event == "settings.changed" {
            json!({"_settingsInvalidation":true,"path":data["path"]})
        } else if event == "files.clipboardChanged"
            && self
                .clipboard_loader
                .lock()
                .is_ok_and(|loader| loader.is_some())
        {
            json!({"_clipboardInvalidation":true})
        } else {
            data
        };
        let bytes = json!({"event":event,"data":wire_data})
            .to_string()
            .into_bytes();
        if bytes.len() > LIMIT {
            return;
        }
        let Ok(entries) = fs::read_dir(&self.root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path == self.path || path.extension().is_none_or(|extension| extension != "sock") {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.file_type().is_socket()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.mode() & 0o077 != 0
            {
                continue;
            }
            if let Err(error) = self.socket.send_to(&bytes, &path)
                && matches!(
                    error.raw_os_error(),
                    Some(libc::ECONNREFUSED) | Some(libc::ENOENT)
                )
            {
                let _ = fs::remove_file(path);
            }
        }
    }

    /// This host shows the Windows Update page (a `settings` host with the update service): `update.progress`
    /// and `update.reply` reach its event stream (native Settings' update page). Panel, Explorer and the rest only
    /// parse them, and `update.request` / `update.owner` reach observers only, never a page (design §4.4).
    pub(crate) fn show_update_events(&self) {
        self.update_pages.store(true, Ordering::Relaxed);
    }
    fn to_pages(&self, event: &str) -> bool {
        match event {
            "update.request" | "update.owner" => false,
            // D-G74a: a Settings host's forwarded winpoint.check and the owner's answer (services/winpoint.rs)
            "winpoint.request" | "winpoint.reply" => false,
            "update.progress" | "update.reply" => self.update_pages.load(Ordering::Relaxed),
            _ => true,
        }
    }

    fn dispatch(&self, event: &str, data: &Value) {
        if self.to_pages(event)
            && let Ok(mut subscribers) = self.subscribers.lock()
        {
            let payload = data;
            if event == "files.clipboardChanged"
                && let Some(revision) = payload["_revision"].as_u64()
            {
                if revision < self.clipboard_sequence.load(Ordering::Relaxed) {
                    return;
                }
                self.clipboard_sequence
                    .fetch_max(revision, Ordering::Relaxed);
            }
            let frame = format!("event: {event}\ndata: {payload}\n\n").into_bytes();
            subscribers.retain(|_, (sender, closed)| {
                if sender.try_send(frame.clone()).is_ok() {
                    true
                } else {
                    closed.store(true, Ordering::Relaxed);
                    false
                }
            });
        }
        let observers = self
            .observers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(event)
            .cloned()
            .unwrap_or_default();
        // Call outside both locks: an observer may emit another event or subscribe.
        for observer in observers {
            observer(data);
        }
    }

    pub(crate) fn on(&self, event: &str, observer: Arc<dyn Fn(&Value) + Send + Sync>) {
        if valid_name(event) {
            self.observers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(event.to_owned())
                .or_default()
                .push(observer);
        }
    }

    pub(crate) fn observe_clipboard(&self, revision: u64) {
        let _guard = self
            .subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.clipboard_sequence
            .fetch_max(revision, Ordering::Relaxed);
    }
    pub(crate) fn set_clipboard_loader(
        &self,
        loader: Arc<dyn Fn() -> Option<Value> + Send + Sync>,
    ) {
        *self
            .clipboard_loader
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(loader);
    }

    pub(crate) fn set_settings_loader(
        &self,
        loader: Arc<dyn Fn(&Value) -> Option<Value> + Send + Sync>,
    ) {
        *self
            .settings_loader
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(loader);
    }

    pub(crate) fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|(_, closed)| !closed.load(Ordering::Relaxed))
            .count()
    }

    pub(crate) fn subscribe(self: &Arc<Self>) -> EventStream {
        let id = self.sequence.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = mpsc::sync_channel(256);
        let closed = Arc::new(AtomicBool::new(false));
        self.subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(id, (sender.clone(), closed.clone()));
        EventStream {
            events: self.clone(),
            id,
            sender,
            receiver,
            pending: Vec::new(),
            offset: 0,
            closed,
        }
    }

    pub(crate) fn shutdown(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        let _ = fs::remove_file(&self.path);
        if let Ok(mut subscribers) = self.subscribers.lock() {
            for (sender, closed) in subscribers.values() {
                closed.store(true, Ordering::Relaxed);
                let _ = sender.try_send(Vec::new());
            }
            subscribers.clear();
        }
    }
}

impl Drop for Events {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn valid_name(name: &str) -> bool {
    [
        "files.",
        "settings.",
        "apps.",
        "shell.",
        "app.",
        "explorer.",
        "index.",
        "system.",
        "media.",
        "display.",
        "notifications.",
        "nativeBackdrop.",
        "storage.",
        "software.",
        "disks.",
        "tray.",
        "clipboard.",
        "sysmon.",
        "windows.",
        "update.",
        "winpoint.",
    ]
    .iter()
    .any(|prefix| name.starts_with(prefix))
        && name.len() < 128
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'_')
}

pub(crate) struct EventStream {
    events: Arc<Events>,
    id: u64,
    pub(crate) sender: SyncSender<Vec<u8>>,
    receiver: Receiver<Vec<u8>>,
    pending: Vec<u8>,
    offset: usize,
    pub(crate) closed: Arc<AtomicBool>,
}

impl Read for EventStream {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        if target.is_empty() {
            return Ok(0);
        }
        if self.closed.load(Ordering::Relaxed) {
            return Ok(0);
        }
        if self.offset == self.pending.len() {
            let keepalive = std::env::var("WIN11SHELL_SSE_KEEPALIVE")
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|n| n.is_finite() && *n > 0.)
                .unwrap_or(15.);
            self.pending = match self
                .receiver
                .recv_timeout(Duration::from_secs_f64(keepalive.clamp(0.01, 300.)))
            {
                Ok(bytes) if bytes.is_empty() => return Ok(0),
                Ok(bytes) => bytes,
                Err(mpsc::RecvTimeoutError::Timeout) => b": keep-alive\n\n".to_vec(),
                Err(_) => return Ok(0),
            };
            self.offset = 0;
        }
        let n = target.len().min(self.pending.len() - self.offset);
        target[..n].copy_from_slice(&self.pending[self.offset..self.offset + n]);
        self.offset += n;
        Ok(n)
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        if let Ok(mut subscribers) = self.events.subscribers.lock() {
            subscribers.remove(&self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn delayed_clipboard_notifications_cannot_restore_an_older_selection() {
        let root = std::env::temp_dir().join(format!("win11-event-order-{}", nonce()));
        let events = Events::new(&root).unwrap();
        let stream = events.subscribe();
        events.dispatch(
            "files.clipboardChanged",
            &json!({"paths":["/tmp/new"],"op":"copy","_revision":2}),
        );
        let frame = String::from_utf8(
            stream
                .receiver
                .recv_timeout(Duration::from_secs(1))
                .unwrap(),
        )
        .unwrap();
        assert!(frame.contains("/tmp/new"));
        assert!(frame.contains("\"_revision\":2"));
        events.dispatch(
            "files.clipboardChanged",
            &json!({"paths":["/tmp/old"],"op":"copy","_revision":1}),
        );
        assert!(stream.receiver.try_recv().is_err());
        events.observe_clipboard(3);
        events.dispatch(
            "files.clipboardChanged",
            &json!({"paths":["/tmp/new"],"op":"copy","_revision":2}),
        );
        assert!(stream.receiver.try_recv().is_err());
        events.shutdown();
        drop(stream);
        drop(events);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn settings_peers_reload_large_snapshots_and_keep_the_initiating_path() {
        let root = std::env::temp_dir().join(format!("win11-settings-events-{}", nonce()));
        let first = Events::new(&root).unwrap();
        let second = Events::new(&root).unwrap();
        let large = "ä".repeat(100_000);
        let snapshot = json!({"future":large});
        let expected = snapshot.clone();
        second.set_settings_loader(Arc::new(move |notice| {
            Some(json!({"settings":snapshot,"path":notice["path"],"changed":["future"]}))
        }));
        let stream = second.subscribe();
        first.emit(
            "settings.changed",
            json!({"settings":expected,"path":"future","changed":["future"]}),
        );
        let frame = String::from_utf8(
            stream
                .receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap(),
        )
        .unwrap();
        let data: Value = serde_json::from_str(
            frame
                .lines()
                .nth(1)
                .unwrap()
                .strip_prefix("data: ")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(data["settings"], expected);
        assert_eq!(data["path"], "future");
        assert!(stream.receiver.try_recv().is_err());
        first.shutdown();
        second.shutdown();
        drop(stream);
        drop(first);
        drop(second);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn update_progress_and_forwarding_reach_only_settings_pages() {
        let root = std::env::temp_dir().join(format!("win11-update-events-{}", nonce()));
        let owner = Events::new(&root).unwrap();
        let settings = Events::new(&root).unwrap();
        let panel = Events::new(&root).unwrap();
        owner.show_update_events();
        settings.show_update_events();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = seen.clone();
        panel.on(
            "update.request",
            Arc::new(move |data: &Value| observed.lock().unwrap().push(data.clone())),
        );
        let (own_page, settings_page, panel_page) = (owner.subscribe(), settings.subscribe(), panel.subscribe());
        owner.emit("update.progress", json!({"pct":5}));
        owner.emit("update.reply", json!({"nonce":"n","ok":true}));
        owner.emit("update.request", json!({"nonce":"n","method":"update.check","args":{}}));
        owner.emit("update.owner", json!({"pid":1,"state":"gone"}));
        owner.emit("update.changed", json!({"rev":2}));
        let frames = |stream: &EventStream, n: usize| -> Vec<String> {
            (0..n)
                .map(|_| String::from_utf8(stream.receiver.recv_timeout(Duration::from_secs(2)).unwrap()).unwrap())
                .map(|frame| frame.lines().next().unwrap().to_owned())
                .collect()
        };
        for page in [&own_page, &settings_page] {
            assert_eq!(
                frames(page, 3),
                ["event: update.progress", "event: update.reply", "event: update.changed"]
            );
        }
        assert_eq!(frames(&panel_page, 1), ["event: update.changed"]);
        std::thread::sleep(Duration::from_millis(100));
        for page in [&own_page, &settings_page, &panel_page] {
            assert!(page.receiver.try_recv().is_err(), "request/owner never reach a page");
        }
        assert_eq!(seen.lock().unwrap().len(), 1, "observers still get update.request");
        for events in [&owner, &settings, &panel] {
            events.shutdown();
        }
        drop((own_page, settings_page, panel_page));
        drop((owner, settings, panel));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn native_events_reach_another_host_without_rebroadcasting_or_interleaving() {
        let root = std::env::temp_dir().join(format!("win11-events-{}", nonce()));
        let first = Events::new(&root).unwrap();
        let second = Events::new(&root).unwrap();
        let mut stream = second.subscribe();
        first.emit(
            "files.clipboardChanged",
            json!({"paths":["/tmp/a\nb"],"op":"copy"}),
        );
        let frame = stream
            .receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        let frame = String::from_utf8(frame).unwrap();
        let mut lines = frame.lines();
        assert_eq!(lines.next(), Some("event: files.clipboardChanged"));
        let payload = lines.next().unwrap().strip_prefix("data: ").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(payload).unwrap(),
            json!({"op":"copy","paths":["/tmp/a\nb"]})
        );
        stream.pending = b"event: x\ndata: {}\n\n".to_vec();
        let mut chunk = [0_u8; 3];
        assert_eq!(stream.read(&mut chunk).unwrap(), 3);
        assert_eq!(&chunk, b"eve");
        first.shutdown();
        second.shutdown();
        drop(stream);
        drop(first);
        drop(second);
        fs::remove_dir_all(root).unwrap();
    }
}
