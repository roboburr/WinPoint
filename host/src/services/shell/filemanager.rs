use super::super::system::common::session;
use super::*;
use glib::variant::ToVariant;
use std::{
    collections::VecDeque,
    sync::{Condvar, mpsc},
    time::Instant,
};
const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";
const METHODS: &[&str] = &["ShowFolders", "ShowItems", "ShowItemProperties"];
const XML: &str = r#"<node><interface name="org.freedesktop.FileManager1"><method name="ShowFolders"><arg type="as" direction="in"/><arg type="s" direction="in"/></method><method name="ShowItems"><arg type="as" direction="in"/><arg type="s" direction="in"/></method><method name="ShowItemProperties"><arg type="as" direction="in"/><arg type="s" direction="in"/></method></interface></node>"#;
struct Queue {
    items: VecDeque<(String, Vec<String>, String)>,
    rate: VecDeque<(String, VecDeque<Instant>)>,
    stuck: usize,
    calls: u64,
    refused: u64,
    worker: bool,
}
pub(super) struct FileManager {
    ctx: Arc<ServiceContext>,
    core: Weak<NativeCore>,
    stopped: AtomicBool,
    queue: Mutex<Queue>,
    cv: Condvar,
    state: Mutex<(String, String)>,
    #[cfg(test)]
    delay_ms: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    timeout_ms: std::sync::atomic::AtomicU64,
}
impl FileManager {
    pub fn new(ctx: Arc<ServiceContext>, core: Weak<NativeCore>) -> Arc<Self> {
        Arc::new(Self {
            ctx,
            core,
            stopped: AtomicBool::new(false),
            queue: Mutex::new(Queue {
                items: VecDeque::new(),
                rate: VecDeque::new(),
                stuck: 0,
                calls: 0,
                refused: 0,
                worker: false,
            }),
            cv: Condvar::new(),
            state: Mutex::new(("off".into(), String::new())),
            #[cfg(test)]
            delay_ms: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            timeout_ms: std::sync::atomic::AtomicU64::new(3000),
        })
    }
    pub fn start(self: &Arc<Self>) {
        let (own, reason) = policy(
            mode(&self.ctx),
            "gnome",
            &std::env::var("WIN11SHELL_FILEMANAGER1").unwrap_or_default(),
            &full_desktop(),
        );
        self.state.lock().unwrap_or_else(|e| e.into_inner()).1 = reason;
        if !own {
            return;
        }
        let weak = Arc::downgrade(self);
        thread::spawn(move || {
            let context = glib::MainContext::new();
            let _ = context.with_thread_default(|| {
                let Ok(bus) = session() else {
                    if let Some(s) = weak.upgrade() {
                        s.state.lock().unwrap().1 = "no session bus".into();
                    }
                    return;
                };
                let info = gio::DBusNodeInfo::for_xml(XML)
                    .unwrap()
                    .lookup_interface(NAME)
                    .unwrap();
                let target = weak.clone();
                let Ok(reg) = bus.register_object(
                    PATH,
                    &info,
                    move |_, sender, _, _, method, params, inv| {
                        let Some(s) = target.upgrade() else {
                            inv.return_dbus_error(
                                "org.freedesktop.DBus.Error.Failed",
                                "internal error",
                            );
                            return;
                        };
                        if !METHODS.contains(&method) {
                            inv.return_dbus_error(
                                "org.freedesktop.DBus.Error.UnknownMethod",
                                &format!("No method {method}"),
                            );
                            return;
                        }
                        let Some((uris, token)) = params.get::<(Vec<String>, String)>() else {
                            inv.return_dbus_error(
                                "org.freedesktop.DBus.Error.InvalidArgs",
                                "invalid arguments",
                            );
                            return;
                        };
                        let uris = match check_uris(&json!(uris)) {
                            Ok(v) => v,
                            Err(why) => {
                                inv.return_dbus_error(
                                    "org.freedesktop.DBus.Error.InvalidArgs",
                                    &why,
                                );
                                return;
                            }
                        };
                        if let Some(why) = s.submit(sender, method, uris, token) {
                            inv.return_dbus_error(
                                "org.freedesktop.DBus.Error.LimitsExceeded",
                                &why,
                            );
                        } else {
                            inv.return_value(None);
                        }
                    },
                    |_, _, _, _, _| ().to_variant(),
                    |_, _, _, _, _, _| false,
                ) else {
                    return;
                };
                let Some(svc) = weak.upgrade() else {
                    let _ = bus.unregister_object(reg);
                    return;
                };
                svc.state.lock().unwrap().0 = "waiting".into();
                let layer = mode(&svc.ctx) == "layer";
                drop(svc);
                let flags = gio::BusNameOwnerFlags::ALLOW_REPLACEMENT
                    | if layer {
                        gio::BusNameOwnerFlags::empty()
                    } else {
                        gio::BusNameOwnerFlags::DO_NOT_QUEUE
                    };
                let acquired = weak.clone();
                let lost = weak.clone();
                let owner = gio::bus_own_name_on_connection(
                    &bus,
                    NAME,
                    flags,
                    move |_, _| {
                        if let Some(s) = acquired.upgrade() {
                            s.state.lock().unwrap().0 = "owner".into();
                        }
                    },
                    move |_, _| {
                        if let Some(s) = lost.upgrade() {
                            if !s.stopped.load(Ordering::Relaxed) {
                                let mut state = s.state.lock().unwrap();
                                state.0 = if layer { "waiting" } else { "off" }.into();
                                if !layer {
                                    state.1 = "name taken".into();
                                }
                            }
                        }
                    },
                );
                loop {
                    while context.pending() {
                        context.iteration(false);
                    }
                    let Some(svc) = weak.upgrade() else {
                        break;
                    };
                    if svc.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    drop(svc);
                    thread::sleep(Duration::from_millis(10));
                }
                gio::bus_unown_name(owner);
                let _ = bus.unregister_object(reg);
            });
        });
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .items
            .clear();
        self.cv.notify_all();
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0 = "off".into();
    }
    fn submit(
        self: &Arc<Self>,
        sender: &str,
        method: &str,
        uris: Vec<String>,
        token: String,
    ) -> Option<String> {
        let now = Instant::now();
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let mut why = if self.stopped.load(Ordering::Relaxed) {
            Some("service stopped".into())
        } else if queue.items.len() >= 16 {
            Some("too many requests waiting (16)".into())
        } else if queue.stuck >= 4 {
            Some("file locations do not respond".into())
        } else {
            None
        };
        if why.is_none() {
            let pos = queue
                .rate
                .iter()
                .position(|(name, _)| name == sender)
                .unwrap_or_else(|| {
                    if queue.rate.len() >= 256 {
                        queue.rate.retain(|(_, hits)| {
                            hits.back().is_some_and(|at| {
                                now.duration_since(*at) <= Duration::from_secs(10)
                            })
                        });
                        while queue.rate.len() >= 256 {
                            queue.rate.pop_front();
                        }
                    }
                    queue.rate.push_back((sender.into(), VecDeque::new()));
                    queue.rate.len() - 1
                });
            let hits = &mut queue.rate[pos].1;
            while hits
                .front()
                .is_some_and(|at| now.duration_since(*at) > Duration::from_secs(10))
            {
                hits.pop_front();
            }
            if hits.len() >= 20 {
                why = Some("too many requests from this app (20 in 10 s)".into());
            } else {
                hits.push_back(now);
            }
        }
        if why.is_some() {
            queue.refused += 1;
        } else {
            queue.calls += 1;
            queue.items.push_back((method.into(), uris, token));
            self.cv.notify_one();
            if !queue.worker {
                queue.worker = true;
                let svc = self.clone();
                thread::spawn(move || svc.work());
            }
        }
        why
    }
    fn work(self: Arc<Self>) {
        loop {
            let item = {
                let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                while queue.items.is_empty() && !self.stopped.load(Ordering::Relaxed) {
                    queue = self.cv.wait(queue).unwrap_or_else(|e| e.into_inner());
                }
                if self.stopped.load(Ordering::Relaxed) {
                    return;
                }
                queue.items.pop_front().unwrap()
            };
            self.handle(item);
        }
    }
    fn handle(self: &Arc<Self>, (method, uris, token): (String, Vec<String>, String)) {
        struct Wait {
            done: bool,
            late: bool,
        }
        let state = Arc::new(Mutex::new(Wait {
            done: false,
            late: false,
        }));
        let worker_state = state.clone();
        let weak = Arc::downgrade(self);
        let (tx, rx) = mpsc::sync_channel(1);
        #[cfg(test)]
        let delay = self.delay_ms.load(Ordering::Relaxed);
        thread::spawn(move || {
            #[cfg(test)]
            thread::sleep(Duration::from_millis(delay));
            let params = params::explorer(
                &uris,
                method != "ShowFolders",
                method == "ShowItemProperties",
                &params::Env::actual(),
            );
            let mut wait = worker_state.lock().unwrap_or_else(|e| e.into_inner());
            wait.done = true;
            if wait.late {
                if let Some(s) = weak.upgrade() {
                    s.queue.lock().unwrap_or_else(|e| e.into_inner()).stuck -= 1;
                }
            }
            drop(wait);
            let _ = tx.send(params);
        });
        #[cfg(test)]
        let timeout = Duration::from_millis(self.timeout_ms.load(Ordering::Relaxed));
        #[cfg(not(test))]
        let timeout = Duration::from_secs(3);
        let result = match rx.recv_timeout(timeout) {
            Ok(v) => v,
            Err(_) => {
                let mut wait = state.lock().unwrap_or_else(|e| e.into_inner());
                if !wait.done {
                    wait.late = true;
                    self.queue.lock().unwrap_or_else(|e| e.into_inner()).stuck += 1;
                    return;
                }
                drop(wait);
                rx.recv().ok().flatten()
            }
        };
        let Some(params) = result else {
            return;
        };
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        if let Some(core) = self.core.upgrade() {
            let token = clean_token(&json!(token));
            let _ =
                core.open_with_token(&json!({"app":"explorer","params":params}), token.as_deref());
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_rate_stuck_and_stop_limits_are_bounded() {
        let (root, core) = super::super::tests::fixture(true);
        let svc = &core.filemanager;
        svc.queue.lock().unwrap().worker = true;
        for i in 0..16 {
            assert!(
                svc.submit(&format!("sender-{i}"), "ShowItems", vec![], String::new())
                    .is_none()
            );
        }
        assert_eq!(
            svc.submit("extra", "ShowItems", vec![], String::new())
                .as_deref(),
            Some("too many requests waiting (16)")
        );
        svc.queue.lock().unwrap().items.clear();
        svc.queue.lock().unwrap().stuck = 4;
        assert_eq!(
            svc.submit("extra", "ShowItems", vec![], String::new())
                .as_deref(),
            Some("file locations do not respond")
        );
        svc.queue.lock().unwrap().stuck = 0;
        for _ in 0..20 {
            assert!(
                svc.submit("flood", "ShowItems", vec![], String::new())
                    .is_none()
            );
            svc.queue.lock().unwrap().items.clear();
        }
        assert_eq!(
            svc.submit("flood", "ShowItems", vec![], String::new())
                .as_deref(),
            Some("too many requests from this app (20 in 10 s)")
        );
        svc.stop();
        assert_eq!(
            svc.submit("last", "ShowItems", vec![], String::new())
                .as_deref(),
            Some("service stopped")
        );
        assert_eq!(svc.queue.lock().unwrap().refused, 4);
        super::super::tests::finish(root, core);
    }
    #[test]
    fn late_path_checks_are_discarded_and_release_stuck_slot() {
        let (root, core) = super::super::tests::fixture(false);
        *core.launcher.lock().unwrap() = Some(Arc::new(|_, _, _| {
            panic!("late path resolution must never launch")
        }));
        core.filemanager.delay_ms.store(150, Ordering::Relaxed);
        core.filemanager.timeout_ms.store(10, Ordering::Relaxed);
        core.filemanager
            .handle(("ShowFolders".into(), vec!["/tmp".into()], String::new()));
        assert_eq!(core.filemanager.queue.lock().unwrap().stuck, 1);
        let until = Instant::now() + Duration::from_secs(1);
        while core.filemanager.queue.lock().unwrap().stuck != 0 && Instant::now() < until {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(core.filemanager.queue.lock().unwrap().stuck, 0);
        super::super::tests::finish(root, core);
    }
}
pub(super) fn clean_token(token: &Value) -> Option<String> {
    token
        .as_str()
        .filter(|s| {
            !s.is_empty() && s.chars().count() <= 512 && s.bytes().all(|c| (33..=126).contains(&c))
        })
        .map(str::to_owned)
}
pub(super) fn check_uris(uris: &Value) -> Result<Vec<String>, String> {
    let Some(uris) = uris.as_array().filter(|u| u.len() <= 1000) else {
        return Err("expected at most 1000 URIs".into());
    };
    let mut out = vec![];
    for uri in uris {
        let Some(uri) = uri
            .as_str()
            .filter(|s| s.chars().count() <= 8192 && !s.chars().any(|c| c < ' '))
        else {
            return Err("invalid URI".into());
        };
        if !uri.is_empty() {
            out.push(uri.into());
        }
    }
    Ok(out)
}
fn full_desktop() -> String {
    for name in [
        "XDG_CURRENT_DESKTOP",
        "XDG_SESSION_DESKTOP",
        "DESKTOP_SESSION",
    ] {
        for word in std::env::var(name)
            .unwrap_or_default()
            .replace(';', ":")
            .split(':')
        {
            if is_full(word) {
                return word.trim().into();
            }
        }
    }
    String::new()
}
fn is_full(word: &str) -> bool {
    let word = word.trim().to_lowercase();
    const FULL: &[&str] = &[
        "gnome",
        "ubuntu",
        "kde",
        "plasma",
        "plasmawayland",
        "unity",
        "xfce",
        "x-cinnamon",
        "cinnamon",
        "mate",
        "lxqt",
        "lxde",
        "budgie",
        "pantheon",
        "deepin",
        "dde",
        "cosmic",
        "enlightenment",
        "ukui",
    ];
    FULL.contains(&word.as_str()) || FULL.contains(&word.split('-').next().unwrap_or(""))
}
pub(super) fn policy(mode: &str, compositor: &str, force: &str, full: &str) -> (bool, String) {
    let force = force.trim().to_lowercase();
    if ["off", "0", "no", "false"].contains(&force.as_str()) {
        return (false, "WIN11SHELL_FILEMANAGER1=off".into());
    }
    if ["own", "on", "1", "yes", "true"].contains(&force.as_str()) {
        return (true, "WIN11SHELL_FILEMANAGER1=own".into());
    }
    if mode == "layer" {
        if compositor == "hyprland" {
            return (true, "layer mode in Hyprland".into());
        }
        if !full.is_empty() {
            return (false, format!("layer mode inside {full}"));
        }
        return (true, format!("layer mode on {compositor}"));
    }
    if mode == "preview" && compositor == "hyprland" {
        return (true, "preview mode in Hyprland".into());
    }
    (false, format!("{mode} mode on {compositor}"))
}
