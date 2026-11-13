//! GIO directory monitors confined to an owned GLib thread and context.
//! Only paths, replies and commands cross threads; FileMonitor objects never do.
use super::{ApiError, ApiResult, Context, abs_in, directory, trash};
use crate::events::Events;
use crate::i18n::Lang;
use gio::prelude::*;
use gtk::{gio, glib};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    thread::{self, JoinHandle},
    time::Duration,
};

const MAX: usize = 64;
const PINNED: u64 = 1_000_000;
const DEBOUNCE: Duration = Duration::from_millis(300);

enum Command {
    Watch {
        path: PathBuf,
        pinned: bool,
        trash: bool,
        reply: mpsc::SyncSender<bool>,
    },
    Unwatch(PathBuf, mpsc::SyncSender<()>),
    Notify(PathBuf),
    Emit(PathBuf, u64),
    Shutdown,
    #[cfg(test)]
    Inspect(mpsc::SyncSender<Vec<(PathBuf, u64)>>),
}

struct Monitor {
    monitor: gio::FileMonitor,
    refs: u64,
    used: u64,
    trash: bool,
}
struct Pending {
    source: glib::Source,
    revision: u64,
}
struct State {
    events: Arc<Events>,
    desktop: PathBuf,
    context: glib::MainContext,
    sender: glib::Sender<Command>,
    monitors: HashMap<PathBuf, Monitor>,
    pending: HashMap<PathBuf, Pending>,
    clock: u64,
}
impl State {
    fn watch(&mut self, path: PathBuf, pinned: bool, trash: bool) -> bool {
        self.clock = self.clock.wrapping_add(1);
        if let Some(entry) = self.monitors.get_mut(&path) {
            entry.refs = entry.refs.saturating_add(1);
            entry.used = self.clock;
            entry.trash |= trash;
            return true;
        }
        if self.monitors.len() >= MAX {
            let victim = self
                .monitors
                .iter()
                .filter(|(path, _)| *path != &self.desktop)
                .min_by_key(|(_, entry)| entry.used)
                .map(|(path, _)| path.clone());
            if let Some(path) = victim {
                self.drop_monitor(&path);
            }
        }
        let Ok(monitor) = gio::File::for_path(&path)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return false;
        };
        monitor.set_rate_limit(300);
        let sender = self.sender.clone();
        let changed = path.clone();
        monitor.connect_changed(move |_, _, _, _| {
            let _ = sender.send(Command::Notify(changed.clone()));
        });
        self.monitors.insert(
            path,
            Monitor {
                monitor,
                refs: if pinned { PINNED } else { 1 },
                used: self.clock,
                trash,
            },
        );
        true
    }

    fn drop_monitor(&mut self, path: &Path) {
        if let Some(entry) = self.monitors.remove(path) {
            entry.monitor.cancel();
        }
    }

    fn unwatch(&mut self, path: &Path) {
        if path == self.desktop {
            return;
        }
        if let Some(entry) = self.monitors.get_mut(path) {
            entry.refs = entry.refs.saturating_sub(1);
            if entry.refs == 0 {
                self.drop_monitor(path);
            }
        }
    }

    fn notify(&mut self, path: PathBuf) {
        // The original Debouncer holds a window from the first trigger. Continuous
        // writes must keep refreshing the UI instead of postponing it until quiet.
        if self.pending.contains_key(&path) {
            return;
        }
        if self.pending.len() > 256 {
            for (_, pending) in self.pending.drain() {
                pending.source.destroy();
            }
        }
        self.clock = self.clock.wrapping_add(1);
        let revision = self.clock;
        let sender = self.sender.clone();
        let changed = path.clone();
        let source = glib::timeout_source_new(
            DEBOUNCE,
            Some("win11-files-dir-changed"),
            glib::Priority::DEFAULT,
            move || {
                let _ = sender.send(Command::Emit(changed.clone(), revision));
                glib::ControlFlow::Break
            },
        );
        source.attach(Some(&self.context));
        self.pending.insert(path, Pending { source, revision });
    }

    fn emit(&mut self, path: PathBuf, revision: u64) {
        if !self
            .pending
            .get(&path)
            .is_some_and(|pending| pending.revision == revision)
        {
            return;
        }
        self.pending.remove(&path);
        self.events
            .emit_local("files.dirChanged", json!({"path":path}));
        if path == self.desktop {
            self.events.emit_local("files.desktopChanged", json!({}));
        }
        if self.monitors.get(&path).is_some_and(|entry| entry.trash) {
            trash::invalidate();
            self.events.emit_local("files.trashChanged", json!({}));
            self.events
                .emit_local("files.dirChanged", json!({"path":"trash:///"}));
        }
    }

    fn close(&mut self) {
        for (_, entry) in self.monitors.drain() {
            entry.monitor.cancel();
        }
        for (_, pending) in self.pending.drain() {
            pending.source.destroy();
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.close();
    }
}

pub(super) struct Watcher {
    sender: Mutex<Option<glib::Sender<Command>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    // Injection avoids process-global XDG/env changes in parallel fixture tests.
    trash_override: Option<Vec<PathBuf>>,
}
impl Watcher {
    pub(super) fn new(events: Arc<Events>, desktop: PathBuf) -> Result<Self, String> {
        Self::start(events, desktop, trash::home_watch_dir(), None)
    }

    fn start(
        events: Arc<Events>,
        desktop: PathBuf,
        home_trash: PathBuf,
        trash_override: Option<Vec<PathBuf>>,
    ) -> Result<Self, String> {
        let (ready, response) = mpsc::sync_channel(1);
        let handle = thread::Builder::new()
            .name("win11-files-watch".into())
            .spawn(move || {
                let context = glib::MainContext::new();
                let result = context.with_thread_default(|| {
                    let mainloop = glib::MainLoop::new(Some(&context), false);
                    #[allow(deprecated)]
                    let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);
                    let mut state = State {
                        events,
                        desktop: desktop.clone(),
                        context: context.clone(),
                        sender: sender.clone(),
                        monitors: HashMap::new(),
                        pending: HashMap::new(),
                        clock: 0,
                    };
                    if desktop.is_dir() {
                        state.watch(desktop, true, false);
                    }
                    if home_trash.is_dir() {
                        state.watch(home_trash, true, true);
                    }
                    let quit = mainloop.clone();
                    let source = receiver.attach(Some(&context), move |command| {
                        match command {
                            Command::Watch {
                                path,
                                pinned,
                                trash,
                                reply,
                            } => {
                                let _ = reply.send(state.watch(path, pinned, trash));
                            }
                            Command::Unwatch(path, reply) => {
                                state.unwatch(&path);
                                let _ = reply.send(());
                            }
                            Command::Notify(path) => state.notify(path),
                            Command::Emit(path, revision) => state.emit(path, revision),
                            Command::Shutdown => {
                                state.close();
                                quit.quit();
                                return glib::ControlFlow::Break;
                            }
                            #[cfg(test)]
                            Command::Inspect(reply) => {
                                let _ = reply.send(
                                    state
                                        .monitors
                                        .iter()
                                        .map(|(p, m)| (p.clone(), m.refs))
                                        .collect(),
                                );
                            }
                        }
                        glib::ControlFlow::Continue
                    });
                    if ready.send(Ok(sender)).is_err() {
                        if let Some(source) = context.find_source_by_id(&source) {
                            source.destroy();
                        }
                        return;
                    }
                    mainloop.run();
                });
                if let Err(error) = result {
                    let _ = ready.send(Err(error.to_string()));
                }
            })
            .map_err(|error| format!("Mapbewaking starten: {error}"))?;
        match response.recv() {
            Ok(Ok(sender)) => Ok(Self {
                sender: Mutex::new(Some(sender)),
                thread: Mutex::new(Some(handle)),
                trash_override,
            }),
            error => {
                let _ = handle.join();
                Err(match error {
                    Ok(Err(error)) => format!("Mapbewaking starten: {error}"),
                    _ => "Mapbewaking kon niet worden gestart.".into(),
                })
            }
        }
    }

    fn send(&self, command: Command, lang: Lang) -> Result<(), ApiError> {
        self.sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .ok_or_else(|| offline(lang))?
            .send(command)
            .map_err(|_| offline(lang))
    }

    fn watch(&self, path: PathBuf, trash: bool, lang: Lang) -> Result<bool, ApiError> {
        let (reply, response) = mpsc::sync_channel(1);
        self.send(
            Command::Watch {
                path,
                pinned: false,
                trash,
                reply,
            },
            lang,
        )?;
        response.recv().map_err(|_| offline(lang))
    }

    fn unwatch(&self, path: PathBuf, lang: Lang) -> Result<(), ApiError> {
        let (reply, response) = mpsc::sync_channel(1);
        self.send(Command::Unwatch(path, reply), lang)?;
        response.recv().map_err(|_| offline(lang))
    }

    fn trash_dirs(&self) -> Vec<PathBuf> {
        self.trash_override
            .clone()
            .unwrap_or_else(trash::watch_dirs)
    }

    pub(super) fn call(&self, method: &str, args: &Value, ctx: &Context) -> Option<ApiResult> {
        if !matches!(method, "files.watch" | "files.unwatch") {
            return None;
        }
        let lang = ctx.lang();
        let result = (|| {
            if args["path"]
                .as_str()
                .is_some_and(|path| path.starts_with("trash:"))
            {
                for path in self.trash_dirs() {
                    if method == "files.watch" {
                        self.watch(path, true, lang)?;
                    } else {
                        self.unwatch(path, lang)?;
                    }
                }
                return Ok(if method == "files.watch" {
                    json!({"ok":true})
                } else {
                    json!({})
                });
            }
            if method == "files.watch" {
                let path = directory(&args["path"], lang)?;
                let is_trash = self.trash_dirs().contains(&path);
                Ok(json!({"ok":self.watch(path, is_trash, lang)?}))
            } else {
                self.unwatch(abs_in(&args["path"], lang)?, lang)?;
                Ok(json!({}))
            }
        })();
        Some(result)
    }

    pub(super) fn shutdown(&self) {
        let sender = self.sender.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(sender) = sender {
            let _ = sender.send(Command::Shutdown);
        }
        if let Some(handle) = self.thread.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
    }
}
impl Drop for Watcher {
    fn drop(&mut self) {
        self.shutdown();
    }
}
fn offline(lang: Lang) -> ApiError {
    ApiError::tr(
        "offline",
        lang,
        "files.shuttingDown",
        crate::i18n::NO_PARAMS,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Read, os::unix::fs::symlink, time::Instant};

    struct Fixture {
        root: PathBuf,
        watcher: Watcher,
        events: Arc<Events>,
        rows: Arc<Mutex<Vec<(String, Value)>>>,
        reader: Option<JoinHandle<()>>,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("win11-watch-{}", crate::events::nonce()));
            let desktop = root.join("Desktop");
            let trash = root.join("Trash/files");
            fs::create_dir_all(&desktop).unwrap();
            fs::create_dir_all(&trash).unwrap();
            fs::create_dir(root.join("Trash/info")).unwrap();
            fs::create_dir(root.join("A")).unwrap();
            let events = Events::new(&root.join("events")).unwrap();
            let mut stream = events.subscribe();
            let rows = Arc::new(Mutex::new(Vec::new()));
            let received = rows.clone();
            let reader = thread::spawn(move || {
                let mut pending = Vec::new();
                let mut buffer = [0; 4096];
                while let Ok(count) = stream.read(&mut buffer) {
                    if count == 0 {
                        break;
                    }
                    pending.extend_from_slice(&buffer[..count]);
                    while let Some(end) = pending.windows(2).position(|bytes| bytes == b"\n\n") {
                        let frame = String::from_utf8_lossy(&pending[..end]).into_owned();
                        pending.drain(..end + 2);
                        let event = frame.lines().find_map(|line| line.strip_prefix("event: "));
                        let data = frame.lines().find_map(|line| line.strip_prefix("data: "));
                        if let (Some(event), Some(data)) = (event, data) {
                            if let Ok(data) = serde_json::from_str(data) {
                                received.lock().unwrap().push((event.to_owned(), data));
                            }
                        }
                    }
                }
            });
            let watcher =
                Watcher::start(events.clone(), desktop, trash.clone(), Some(vec![trash])).unwrap();
            Self {
                root,
                watcher,
                events,
                rows,
                reader: Some(reader),
            }
        }
        fn context(&self) -> Context {
            Context::new(
                self.root.clone(),
                self.root.join("Desktop"),
                "en".into(),
                false,
            )
        }
        fn call(&self, method: &str, path: &Path) -> Value {
            self.watcher
                .call(method, &json!({"path":path}), &self.context())
                .unwrap()
                .unwrap()
        }
        fn snapshots(&self) -> Vec<(PathBuf, u64)> {
            let (reply, response) = mpsc::sync_channel(1);
            self.watcher
                .send(Command::Inspect(reply), Lang::En)
                .unwrap();
            response.recv().unwrap()
        }
        fn count(&self, event: &str, path: Option<&Path>) -> usize {
            self.rows
                .lock()
                .unwrap()
                .iter()
                .filter(|(name, data)| {
                    name == event && path.is_none_or(|path| data["path"] == json!(path))
                })
                .count()
        }
        fn wait(&self, event: &str, path: Option<&Path>, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(5);
            while self.count(event, path) < count {
                assert!(
                    Instant::now() < deadline,
                    "missing {event} {path:?}; got {:?}",
                    self.rows.lock().unwrap()
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.watcher.shutdown();
            self.events.shutdown();
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn external_changes_are_debounced_on_own_glib_context() {
        let f = Fixture::new();
        let path = f.root.join("A");
        assert_eq!(f.call("files.watch", &path), json!({"ok":true}));
        for index in 0..8 {
            fs::write(path.join(format!("external-{index}")), b"fixture").unwrap();
            thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(f.count("files.dirChanged", Some(&path)), 0);
        f.wait("files.dirChanged", Some(&path), 1);
        thread::sleep(Duration::from_millis(400));
        assert_eq!(f.count("files.dirChanged", Some(&path)), 1);
    }

    #[test]
    fn continuous_external_writes_refresh_periodically_before_the_writer_stops() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let f = Fixture::new();
        let path = f.root.join("A");
        f.call("files.watch", &path);
        let stop = Arc::new(AtomicBool::new(false));
        let finished = stop.clone();
        let target = path.join("streaming-write.txt");
        let writer = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(6);
            let mut index = 0;
            while !finished.load(Ordering::Relaxed) && Instant::now() < deadline {
                if fs::write(&target, index.to_string()).is_err() {
                    break;
                }
                index += 1;
                thread::sleep(Duration::from_millis(25));
            }
        });
        let deadline = Instant::now() + Duration::from_secs(4);
        while f.count("files.dirChanged", Some(&path)) < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        let count = f.count("files.dirChanged", Some(&path));
        let still_writing = !writer.is_finished();
        stop.store(true, Ordering::Relaxed);
        writer.join().unwrap();
        assert!(
            count >= 2 && still_writing,
            "expected periodic refresh during continuous writes; count={count}, live={still_writing}"
        );
    }

    #[test]
    fn reference_counts_cancel_after_last_unwatch() {
        let f = Fixture::new();
        let path = f.root.join("A");
        f.call("files.watch", &path);
        f.call("files.watch", &path);
        f.call("files.unwatch", &path);
        assert!(
            f.snapshots()
                .iter()
                .any(|(key, refs)| key == &path && *refs == 1)
        );
        fs::write(path.join("first"), b"fixture").unwrap();
        f.wait("files.dirChanged", Some(&path), 1);
        thread::sleep(Duration::from_millis(350));
        f.call("files.unwatch", &path);
        assert!(!f.snapshots().iter().any(|(key, _)| key == &path));
        let before = f.count("files.dirChanged", Some(&path));
        fs::write(path.join("after-unwatch"), b"fixture").unwrap();
        thread::sleep(Duration::from_millis(650));
        assert_eq!(f.count("files.dirChanged", Some(&path)), before);
    }

    #[test]
    fn desktop_is_automatically_pinned_and_emits_both_events() {
        let f = Fixture::new();
        let desktop = f.root.join("Desktop");
        assert!(
            f.snapshots()
                .iter()
                .any(|(key, refs)| key == &desktop && *refs == PINNED)
        );
        f.call("files.unwatch", &desktop);
        fs::write(desktop.join("external-desktop-note"), b"fixture").unwrap();
        f.wait("files.dirChanged", Some(&desktop), 1);
        f.wait("files.desktopChanged", None, 1);
        assert!(
            f.snapshots()
                .iter()
                .any(|(key, refs)| key == &desktop && *refs == PINNED)
        );
    }

    #[test]
    fn oldest_non_desktop_monitor_is_evicted_at_sixty_four() {
        let f = Fixture::new();
        let a = f.root.join("A");
        f.call("files.watch", &a);
        for index in 0..MAX - 3 {
            let path = f.root.join(format!("folder-{index}"));
            fs::create_dir(&path).unwrap();
            f.call("files.watch", &path);
        }
        assert_eq!(f.snapshots().len(), MAX);
        // Refresh home Trash so A, rather than the startup Trash, is now the LRU victim.
        f.call("files.watch", &f.root.join("Trash/files"));
        let extra = f.root.join("extra");
        fs::create_dir(&extra).unwrap();
        f.call("files.watch", &extra);
        let state = f.snapshots();
        assert_eq!(state.len(), MAX);
        assert!(!state.iter().any(|(path, _)| path == &a));
        assert!(
            state
                .iter()
                .any(|(path, refs)| path == &f.root.join("Desktop") && *refs == PINNED)
        );
    }

    #[test]
    fn trash_watch_uses_files_path_and_emits_uri_and_trash_events() {
        let f = Fixture::new();
        let files = f.root.join("Trash/files");
        assert_eq!(
            f.watcher
                .call("files.watch", &json!({"path":"trash:///"}), &f.context())
                .unwrap()
                .unwrap(),
            json!({"ok":true})
        );
        fs::write(files.join("external-trash-item"), b"fixture").unwrap();
        f.wait("files.dirChanged", Some(&files), 1);
        f.wait("files.trashChanged", None, 1);
        f.wait("files.dirChanged", Some(Path::new("trash:///")), 1);
        assert_eq!(
            f.watcher
                .call("files.unwatch", &json!({"path":"trash:///"}), &f.context())
                .unwrap()
                .unwrap(),
            json!({})
        );
    }

    #[test]
    fn symlink_directory_and_invalid_paths_finish_without_main_gtk() {
        let f = Fixture::new();
        let alias = f.root.join("alias");
        symlink(f.root.join("A"), &alias).unwrap();
        assert_eq!(f.call("files.watch", &alias), json!({"ok":true}));
        fs::write(f.root.join("A/from-other-process"), b"fixture").unwrap();
        f.wait("files.dirChanged", Some(&alias), 1);
        let looped = f.root.join("loop");
        symlink("loop", &looped).unwrap();
        let before = Instant::now();
        assert!(
            f.watcher
                .call("files.watch", &json!({"path":looped}), &f.context())
                .unwrap()
                .is_err()
        );
        assert!(before.elapsed() < Duration::from_secs(1));
        assert!(
            f.watcher
                .call("files.watch", &json!({"path":"relative"}), &f.context())
                .unwrap()
                .is_err()
        );
        assert_eq!(f.call("files.unwatch", &f.root.join("absent")), json!({}));
    }

    #[test]
    fn shutdown_cancels_pending_sources_and_joins_worker() {
        let f = Fixture::new();
        let path = f.root.join("A");
        f.call("files.watch", &path);
        fs::write(path.join("pending"), b"fixture").unwrap();
        let before = Instant::now();
        f.watcher.shutdown();
        assert!(before.elapsed() < Duration::from_secs(2));
        let count = f.count("files.dirChanged", Some(&path));
        fs::write(path.join("after-shutdown"), b"fixture").unwrap();
        thread::sleep(Duration::from_millis(500));
        assert_eq!(f.count("files.dirChanged", Some(&path)), count);
        let error = f
            .watcher
            .call("files.watch", &json!({"path":path}), &f.context())
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, "offline");
        f.watcher.shutdown();
    }
}
